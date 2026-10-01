//! A kernel-side line-reading REPL: type a name, press Enter, run it. Living at EL1
//! (`crate::shell`, the same place the lab's own real shell lives) specifically so it needs
//! neither `userlib` nor argv/envp to exist first. `lexer`/`syntax` give it a real grammar --
//! quoting, escaping, `$VAR`/`${VAR}`/`$?` expansion markers, pipes, redirection -- and `run`/
//! `run_pipeline`/`run_segment` (below) execute all of it for real: `run_segment` is the one
//! place a segment -- a leading run of `NAME=...` assignment words, then a builtin or an
//! external program, or nothing at all -- actually runs, for both a plain single command (`run`,
//! `pipe_stdio: None`) and one stage of a multi-stage pipeline via temp files under `/tmp`
//! (`run_pipeline`, `pipe_stdio: Some(...)`, each stage wrapped in `frame_stack::with_subshell`
//! so nothing it does leaks to the next stage or the parent shell). `cd` (`fs::cwd`'s own doc
//! comment explains why it has to be a builtin), `source`/`.` (runs a script's lines unscoped, so
//! its own `cd`/`export` sticks), `sh` (same as `source`, but scoped --
//! `frame_stack::with_child`), and `export`/`unset` are its only builtins (`is_builtin`);
//! everything else is looked up under `usr/bin/` and run as a real EL0 program, with every
//! currently-exported variable (`frame_stack`), plus any one-shot assignment prefix
//! (`frame_stack::with_overlay`), as its `envp`.
//!
//! `lexer`/`syntax`/`frame_stack`/`expand` are pure logic, no hardware dependency: built and
//! host-tested under both targets, like `fs::sector`/`exec::elfparse`. Everything below them
//! touches the console, keyboard, or filesystem, so it's bare-metal-only, like `exec::process`.

mod expand;
mod frame_stack;
mod lexer;
mod syntax;

#[cfg(target_os = "none")]
use crate::console::{self, DEFAULT_BG as BG, DEFAULT_FG as FG};
#[cfg(target_os = "none")]
use crate::drivers::virtio::{gpu, input};
#[cfg(target_os = "none")]
use crate::exec;
#[cfg(target_os = "none")]
use crate::fs;
#[cfg(target_os = "none")]
use crate::keyboard;
#[cfg(target_os = "none")]
use crate::keyboard::line_discipline::{self, LineOutcome, Mode};
#[cfg(target_os = "none")]
use crate::syscall::fd::{self, StdioRedirect};
#[cfg(target_os = "none")]
use alloc::format;
#[cfg(target_os = "none")]
use alloc::string::{String, ToString};
#[cfg(target_os = "none")]
use alloc::vec::Vec;
#[cfg(target_os = "none")]
use arm_pl011_uart::Uart;
#[cfg(target_os = "none")]
use core::fmt::Write;
#[cfg(target_os = "none")]
use hadris_fat::FatVolumeReadExt;

/// Prints `msg` to both UART and the console -- so a typo'd command is visible on screen, not
/// only in a UART log the user may not have open (matches `r10_repl`'s own `report`). Fetches the
/// console fresh via `console::get()` rather than taking one as a parameter, and only for the
/// instant it's needed: `run` (this function's only caller) calls this again right after
/// `run_program`, whose EL0 program may itself have called `console::get()` via `syscall::fd`'s
/// `write` -- holding a `&mut Console` across that call would alias with it.
#[cfg(target_os = "none")]
fn report(uart: &mut Uart, msg: &str) {
    writeln!(uart, "{msg}").ok();
    if let Some(console) = console::get() {
        console.puts(msg, FG, BG);
        console.putc('\n', FG, BG);
    }
}

/// The builtin `name` is, if it is one -- checked before ever trying to resolve
/// `usr/bin/<name>`, and before a pipeline stage is allowed to even attempt connecting to the
/// pipe wiring: none of these launches a real process with a real fd table, so none has anywhere
/// to plug a pipe binding into (`run_pipeline`'s own doc comment).
#[cfg(target_os = "none")]
fn is_builtin(name: &str) -> bool {
    matches!(name, "cd" | "source" | "." | "sh" | "export" | "unset")
}

/// `$?`'s current value -- deliberately *not* part of `frame_stack` (matching the lab's own
/// separation): it isn't a variable a script can `export`/`unset`, just a running record of the
/// last line/pipeline's own exit status.
///
/// SAFETY (every access, via `last_status`/`set_last_status`): single core, shell runs strictly
/// sequentially -- same reasoning `PIPE_COUNTER`/`frame_stack`'s own statics already rest on.
#[cfg(target_os = "none")]
static mut LAST_STATUS: i32 = 0;

/// The exit status `$?` currently expands to.
#[cfg(target_os = "none")]
pub fn last_status() -> i32 {
    #[allow(clippy::deref_addrof)]
    unsafe {
        *(&raw const LAST_STATUS)
    }
}

/// Records `code` as `$?`'s new value -- every path through `run` calls this exactly once per
/// line/pipeline, except a blank or comment-only line (`SyntaxError::EmptyCommand`), which leaves
/// it untouched (the lab's own documented rule).
#[cfg(target_os = "none")]
fn set_last_status(code: i32) {
    #[allow(clippy::deref_addrof)]
    unsafe {
        *(&raw mut LAST_STATUS) = code;
    }
}

/// Makes `argv[1]` (or `/`, with no argument -- there's no `$HOME` to fall back to yet) the
/// working directory, if it names a real directory. Unlike every other command, this can't be an
/// ordinary `usr/bin/` program: an EL0 process changing its own working directory would never be
/// visible to the shell that launched it (real Unix's `cd` is a shell builtin for exactly this
/// reason), and there's only one working directory to begin with here -- the shell's own, since
/// at most one program is ever resident (`fs::cwd`'s own doc comment).
#[cfg(target_os = "none")]
fn cd(uart: &mut Uart, argv: &[String]) -> i32 {
    let target = argv.get(1).map_or("/", String::as_str);
    let Some(vol) = fs::volume::get(0) else {
        report(uart, "system.img not mounted");
        return 1;
    };
    let absolute = fs::cwd::absolute(target);
    if fs::path::resolve_dir(vol, &absolute).is_err() {
        report(uart, &format!("cd: {target}: not a directory"));
        return 1;
    }
    fs::cwd::set(absolute);
    0
}

/// How many scripts may be nested (a script's own line sourcing another, and so on) before
/// `source` refuses to go further -- protects the kernel stack from a script that (directly or
/// indirectly) sources itself. Matches the reference lab's own chosen value; no reason here to
/// pick a different one.
const MAX_SCRIPT_DEPTH: usize = 16;

/// `source FILE` / `. FILE` (`.` is POSIX's own name for this; `source` is bash's alias) --
/// resolves `FILE` against the working directory (no `/usr/bin` search, and no exec bit needed,
/// unlike launching a program: POSIX's own rule for this, and virtos has no notion of an exec
/// bit on a plain file to check anyway) and runs each of its lines as if typed at the prompt.
/// Unscoped, matching the lab's own `source`/`.` (as opposed to `sh FILE`, below, which does push
/// a scope): a sourced script's own `cd`/`export` sticks after it returns, the same as if its
/// lines had been typed directly at the prompt -- this function pushes nothing onto
/// `frame_stack` itself, so there's nothing to pop.
///
/// Also `sh FILE`'s own implementation (below) -- `name` is `"sh"`/`"source"`/`"."` purely for
/// this function's own error-message prefix, and `sh` differs only in wrapping the whole call in
/// `frame_stack::with_child` (virtos has no `$PATH` for `source`/`.` to search ahead of the
/// working directory the way the lab's own version does, so that's the *only* real difference
/// here -- see `frame_stack`'s own doc comment).
///
/// `depth` is the nesting level this is running at (0 at the prompt, incrementing by one per
/// level); refused outright at `MAX_SCRIPT_DEPTH` before this even tries to open `FILE`, and
/// threaded through as `depth + 1` to every line it runs, the same way `run_pipeline` would need
/// to if a builtin were ever allowed to run as one of its stages (it isn't yet -- `is_builtin`'s
/// own doc comment).
///
/// Returns the exit status to report for `$?`: the lab's own rule for a script is "its last
/// line's own status," not a flat success -- tracked here by just taking whatever `run` (this
/// function's own recursive caller for each line) returns, starting from `0` for an empty file or
/// one of nothing but blank/comment lines (which never update it at all -- `run`'s own doc
/// comment on `EmptyCommand`).
#[cfg(target_os = "none")]
fn source(uart: &mut Uart, name: &str, argv: &[String], depth: usize) -> i32 {
    let path = match argv {
        [_, path] => path.as_str(),
        [_] => {
            report(uart, &format!("{name}: usage: {name} FILE"));
            return 1;
        }
        _ => {
            report(uart, &format!("{name}: too many arguments"));
            return 1;
        }
    };
    if depth >= MAX_SCRIPT_DEPTH {
        report(uart, &format!("{name}: {path}: too many levels of scripts"));
        return 1;
    }
    let Some(vol) = fs::volume::get(0) else {
        report(uart, "system.img not mounted");
        return 1;
    };
    let absolute = fs::cwd::absolute(path);
    let Ok(entry) = fs::path::resolve(vol, &absolute) else {
        report(uart, &format!("{name}: {path}: not found"));
        return 1;
    };
    if entry.is_directory() {
        report(uart, &format!("{name}: {path}: is a directory"));
        return 1;
    }
    let Ok(mut reader) = vol.read_file(&entry) else {
        report(uart, &format!("{name}: {path}: failed to open"));
        return 1;
    };
    let Ok(bytes) = reader.read_to_vec() else {
        report(uart, &format!("{name}: {path}: failed to read"));
        return 1;
    };
    let Ok(content) = core::str::from_utf8(&bytes) else {
        report(uart, &format!("{name}: {path}: not valid UTF-8"));
        return 1;
    };
    let mut status = 0;
    for line in content.lines() {
        status = run(uart, line, depth + 1);
    }
    status
}

/// `sh FILE` -- runs `FILE` exactly like `source`/`.` (see that function's own doc comment for
/// why they share one implementation), except scoped: wrapped in `frame_stack::with_child`, so
/// `FILE`'s own `cd`/`export` -- unlike `source`'s -- don't leak back to whoever ran `sh`. Real,
/// immediate caller of `with_child`, which otherwise has none yet.
#[cfg(target_os = "none")]
fn sh(uart: &mut Uart, argv: &[String], depth: usize) -> i32 {
    frame_stack::with_child(|| source(uart, "sh", argv, depth))
}

/// `export NAME[=VALUE]...`: marks each named variable exported -- handed to every program this
/// shell starts from now on (`run`/`run_pipeline`'s own envp-building) -- assigning it first if
/// `=VALUE` is given. `export NAME` for a variable that isn't set yet does nothing (there is
/// nothing to mark), matching the lab's own `export_var` exactly. Every operand is attempted even
/// if an earlier one was an invalid identifier; the status is `1` if any operand was invalid, `0`
/// otherwise.
#[cfg(target_os = "none")]
fn export(uart: &mut Uart, argv: &[String]) -> i32 {
    if argv.len() < 2 {
        report(uart, "export: usage: export NAME[=VALUE]...");
        return 1;
    }
    let mut status = 0;
    for arg in &argv[1..] {
        let (name, value) = match arg.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (arg.as_str(), None),
        };
        if frame_stack::export_var(name, value).is_err() {
            report(uart, &format!("export: '{arg}': not a valid identifier"));
            status = 1;
        }
    }
    status
}

/// `unset NAME...`: removes each named variable, set or not -- removing one that was never set
/// is not an error (POSIX's own rule). No operands is a no-op, matching a real shell's own
/// permissive `unset` with nothing to unset. Always succeeds.
#[cfg(target_os = "none")]
fn unset(argv: &[String]) -> i32 {
    for name in &argv[1..] {
        frame_stack::unset_var(name);
    }
    0
}

/// Applies `redirs` with no program to run afterward -- `cd`, and a segment with no command at
/// all (only redirections, e.g. `> f` -- POSIX still creates the file), both land here rather
/// than through `syscall::fd::bind_stdio`/`run_program`: there is no process, and no fd table to
/// bind into. Each redirect's real side effect still happens -- `In` still errors if its target
/// is missing even though nothing reads it; `Out`/`Append` still creates or truncates its target
/// even though nothing writes to it -- just opened and, for `Out`/`Append`, immediately finished,
/// since there is nothing here to hand the open file to. `Dup` has nothing to alias without a
/// real fd table, so it is a genuine no-op.
///
/// Applied by `run` (this function's only caller) *before* whatever follows -- `cd`'s own
/// `fs::cwd::set`, if any -- touches anything, so a relative path in a redirect resolves against
/// the working directory as it was *before* that: `cd foo > bar` creates `bar` relative to where
/// the shell started, not `foo`, matching a real shell. This falls out of the calling order
/// alone; nothing here needs to know which directory is "the right one" itself.
#[cfg(target_os = "none")]
fn apply_redirects_standalone(redirs: &[syntax::Redirection]) -> Result<(), String> {
    let Some(vol) = fs::volume::get(0) else {
        return Err("system.img not mounted".into());
    };
    for redir in redirs {
        match redir {
            syntax::Redirection::In(path) => {
                let absolute = fs::cwd::absolute(path);
                if fd::open_for_read(vol, &absolute).is_err() {
                    return Err(format!("{path}: not found"));
                }
            }
            syntax::Redirection::Out { path, append, .. } => {
                let absolute = fs::cwd::absolute(path);
                let Ok(writer) = fd::open_for_write(vol, &absolute, *append) else {
                    return Err(format!("{path}: cannot create"));
                };
                if writer.finish().is_err() {
                    return Err(format!("{path}: cannot create"));
                }
            }
            syntax::Redirection::Dup { .. } => {} // nothing to alias without a real fd table
        }
    }
    Ok(())
}

/// Layers `redirs` onto `overrides` (`syscall::fd::bind_stdio`'s own parameter shape) -- each
/// `Redirection` names the fd it binds (`In` always fd 0; `Out`/`Dup` carry their own, defaulted
/// to 1 by `syntax`'s own parsing), so this is a plain sort into array slots, *overwriting*
/// whatever `overrides` already held in that slot. For a plain single command, `overrides`
/// starts as `[None, None, None]`, so this is just "build the array from `redirs`" -- but
/// `run_pipeline` (below) starts it from that stage's own pipe binding instead, which is exactly
/// what makes a stage's own redirect take precedence over the pipe for the same fd (the same
/// layering a real shell uses, and the reason `echo foo > bar | wc` sends `foo` to `bar`, not
/// into the pipe, leaving `wc` to see nothing and report all zeros).
#[cfg(target_os = "none")]
fn apply_overrides(
    mut overrides: [Option<StdioRedirect>; 3],
    redirs: &[syntax::Redirection],
) -> [Option<StdioRedirect>; 3] {
    for redir in redirs {
        match redir {
            syntax::Redirection::In(path) => {
                overrides[0] = Some(StdioRedirect::In(path.clone()));
            }
            syntax::Redirection::Out { fd, path, append } => {
                overrides[*fd as usize] = Some(StdioRedirect::Out {
                    path: path.clone(),
                    append: *append,
                });
            }
            syntax::Redirection::Dup { fd, target } => {
                overrides[*fd as usize] = Some(StdioRedirect::Dup(*target));
            }
        }
    }
    overrides
}

/// Every exported variable as a `"NAME=VALUE"` string, in storage order -- built fresh for every
/// launch (not cached) since `export`/`unset` can change between one command and the next.
#[cfg(target_os = "none")]
fn exported_env() -> Vec<String> {
    frame_stack::exported()
        .map(|(name, value)| format!("{name}={value}"))
        .collect()
}

/// Expands `words` into real argument strings (`expand::expand_words`), against `frame_stack`'s
/// current variables and `$?`'s current value -- the one real call site for both, injected as
/// plain parameters so `expand` itself stays pure/host-testable without reaching into either
/// directly.
#[cfg(target_os = "none")]
fn var_lookup(name: &str) -> Option<String> {
    frame_stack::var(name).map(|value| value.to_string())
}

#[cfg(target_os = "none")]
fn expand_argv(words: &[lexer::Word]) -> Vec<String> {
    expand::expand_words(words, &var_lookup, last_status())
}

/// Splits a leading run of `NAME=...`-shaped words (`lexer::Word::assignment`) off the front of
/// `words`, stopping at the first word that isn't one -- a `NAME=...`-shaped word *after* that
/// point is just an ordinary argument (`echo FOO=bar` never touches `FOO`), matching the lab's
/// own position-based rule exactly (`lexer::Word`'s own doc comment on `assign`). Returns each
/// assignment's name and its still-unexpanded value word, plus whatever's left -- the real
/// command and its arguments, if any.
#[cfg(target_os = "none")]
fn split_assignments(words: &[lexer::Word]) -> (Vec<(&str, lexer::Word)>, &[lexer::Word]) {
    let mut assignments = Vec::new();
    let mut i = 0;
    while i < words.len() {
        match words[i].assignment() {
            Some(pair) => {
                assignments.push(pair);
                i += 1;
            }
            None => break,
        }
    }
    (assignments, &words[i..])
}

/// Runs the builtin `name` is already known to be (`is_builtin`'s own check already happened) --
/// just the dispatch table `run`'s single-segment path and `run_segment` (via `dispatch_segment`)
/// both used to repeat inline before this existed.
#[cfg(target_os = "none")]
fn dispatch_builtin(uart: &mut Uart, name: &str, argv: &[String], depth: usize) -> i32 {
    match name {
        "cd" => cd(uart, argv),
        "source" | "." => source(uart, name, argv, depth),
        "sh" => sh(uart, argv, depth),
        "export" => export(uart, argv),
        "unset" => unset(argv),
        _ => unreachable!("`is_builtin` already said {name} is one"),
    }
}

/// Runs `name`'s command (`argv[0]`, already expanded) -- a builtin or an external program --
/// once any leading assignment prefix has already been stripped by the caller (`run_segment`).
/// `pipe_stdio` is only ever actually used by an external program: no builtin here has real
/// fd-based I/O to connect a pipe to, pipeline stage or not, so a builtin's own redirections (if
/// any) are always applied via `apply_redirects_standalone` -- same as a standalone builtin
/// already used before pipeline stages could be one at all -- and `pipe_stdio` is simply unused
/// for that stage. Returns `(status, aborts)`: `aborts` is `true` only for a failure at the
/// *launch* level (not found, a bad redirect, a bad ELF) -- never a builtin's own `1`, nor a
/// successfully launched program's own nonzero exit -- `run_pipeline`'s own reason for keeping
/// the two apart: a pipeline keeps running after an ordinary command failure, but not after one
/// that couldn't start at all.
#[cfg(target_os = "none")]
fn dispatch_segment(
    uart: &mut Uart,
    name: &str,
    argv: &[String],
    depth: usize,
    redirs: &[syntax::Redirection],
    pipe_stdio: Option<[Option<StdioRedirect>; 3]>,
) -> (i32, bool) {
    if is_builtin(name) {
        if let Err(msg) = apply_redirects_standalone(redirs) {
            report(uart, &msg);
            return (1, true);
        }
        return (dispatch_builtin(uart, name, argv, depth), false);
    }
    let program = match read_program(name) {
        Ok(program) => program,
        Err(msg) => {
            report(uart, &msg);
            return (127, true);
        }
    };
    let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let env = exported_env();
    let env_refs: Vec<&str> = env.iter().map(String::as_str).collect();
    let stdio = apply_overrides(pipe_stdio.unwrap_or([None, None, None]), redirs);
    match exec::process::run_program(&program, &argv_refs, &env_refs, stdio) {
        Ok(code) => (code, false),
        Err(e) => {
            report(uart, &format!("{name}: {e:?}"));
            (126, true)
        }
    }
}

/// Runs one segment: a leading run of `NAME=...` assignment words (`split_assignments`), applied
/// either permanently (`frame_stack::set_var`, no command follows) or as a one-shot overlay
/// (`frame_stack::with_overlay`, visible only to the command's own environment, restored right
/// after -- bash's own rule: a prefix assignment is visible regardless of whether the name is
/// otherwise exported), then whatever command follows, dispatched by `dispatch_segment`.
///
/// `pipe_stdio` is `None` for a plain single command (`run`'s own call site) or `Some(default
/// pipe bindings)` for one pipeline stage (`run_pipeline`'s). A pipeline stage always runs
/// wrapped in `frame_stack::with_subshell` -- around the *whole* segment, assignment-only or not
/// -- so a bare `FOO=bar` stage, a builtin stage's own `cd`/`export`/`unset`, or anything else it
/// does can never leak to the next stage or the parent shell, matching real bash's fork-per-stage
/// isolation even though virtos's stages still run sequentially, not concurrently (confirmed
/// correctly isolating for the external-program case already; this is what extends the same
/// guarantee to a builtin or bare-assignment stage now that both are allowed to be one).
///
/// Returns `(status, aborts)` -- see `dispatch_segment`'s own doc comment for what `aborts` means
/// and why it exists; a bare assignment's own failed redirect counts as an aborting failure too,
/// for the same reason a launch failure does (the segment never really "ran" at all).
#[cfg(target_os = "none")]
fn run_segment(
    uart: &mut Uart,
    segment: &syntax::Segment,
    depth: usize,
    pipe_stdio: Option<[Option<StdioRedirect>; 3]>,
) -> (i32, bool) {
    let is_pipeline_stage = pipe_stdio.is_some();
    let body = || -> (i32, bool) {
        let (assignments, command_words) = split_assignments(&segment.words);
        let overlay: Vec<(String, String)> = assignments
            .into_iter()
            .map(|(name, value_word)| {
                let value =
                    expand::expand_assignment_value(&value_word, &var_lookup, last_status());
                (name.to_string(), value)
            })
            .collect();

        if command_words.is_empty() {
            for (name, value) in &overlay {
                frame_stack::set_var(name, value).ok();
            }
            return match apply_redirects_standalone(&segment.redirs) {
                Ok(()) => (0, false),
                Err(msg) => {
                    report(uart, &msg);
                    (1, true)
                }
            };
        }

        let argv = expand_argv(command_words);
        let name = argv[0].as_str();
        // `with_overlay`'s own restore unconditionally puts the *whole* variable list back,
        // overlay or not -- exactly right for undoing the overlay itself, but it would just as
        // happily undo a builtin's own *permanent* mutation (`export`/`unset`/a `source`d
        // script's own `export`) if nothing actually needed overlaying in the first place. Only
        // wrap when there's a real prefix to apply and later undo; skip it entirely otherwise, so
        // `export FOO=bar` (no prefix) really does stick.
        if overlay.is_empty() {
            dispatch_segment(uart, name, &argv, depth, &segment.redirs, pipe_stdio)
        } else {
            frame_stack::with_overlay(&overlay, || {
                dispatch_segment(uart, name, &argv, depth, &segment.redirs, pipe_stdio)
            })
        }
    };

    if is_pipeline_stage {
        frame_stack::with_subshell(body)
    } else {
        body()
    }
}

/// Reads `usr/bin/<name>` off `system.img` (blk0) into a freshly allocated `Vec<u8>`. Fetches
/// `fs::volume::get(0)` itself, used and dropped entirely within this function -- deliberately
/// never returned or held by a caller across a `run_program` call, which itself fetches the same
/// volume again internally (via `bind_stdio`): `fs::volume::get` hands back a fresh `&'static
/// mut` on every call, sound only as long as two such references are never *simultaneously*
/// live, which holding one across a nested call that fetches its own would risk.
#[cfg(target_os = "none")]
fn read_program(name: &str) -> Result<Vec<u8>, String> {
    let Some(vol) = fs::volume::get(0) else {
        return Err("system.img not mounted".into());
    };
    let Ok(entry) = fs::path::resolve(vol, &format!("usr/bin/{name}")) else {
        return Err(format!("{name}: not found"));
    };
    let Ok(mut reader) = vol.read_file(&entry) else {
        return Err(format!("{name}: failed to open"));
    };
    reader
        .read_to_vec()
        .map_err(|_| format!("{name}: failed to read"))
}

/// Prefix for pipeline temp file names -- a leading dot, distinct from anything a user is likely
/// to type by hand. Not itself the safety mechanism against colliding with a real file
/// (`next_pipe_path` is); just a first line of defense that makes an accidental collision
/// unlikely in the first place.
const PIPE_PREFIX: &str = "/tmp/.pipe";

/// How many names `next_pipe_path` will skip past an existing file before giving up -- guards
/// against a pathological `/tmp`, not a realistic case.
const MAX_PIPE_NAME_ATTEMPTS: usize = 1000;

/// Monotonic counter for pipeline temp file names: one for the shell's whole lifetime,
/// deliberately not reset per pipeline -- a stray leftover from an earlier pipeline (one that
/// panicked before its own cleanup ran, say) should never collide with a live one, which
/// resetting to 0 every time risks.
///
/// SAFETY (every access): single core, and the shell runs strictly sequentially -- nothing else
/// ever touches this.
#[cfg(target_os = "none")]
static mut PIPE_COUNTER: usize = 0;

/// Picks the next unused pipeline temp file path (`/tmp/.pipeN` for the lowest `N`, from
/// `PIPE_COUNTER`, that doesn't already resolve to something) and creates it there, empty, right
/// away -- not just reserving the name. A stage whose own redirect overrides this pipe slot's
/// default binding (`echo foo > bar | wc`) means nothing in `run_pipeline` ever opens this file
/// for writing at all, and the *next* stage reading it must still see an empty file, not a
/// missing one: a real pipe's read end sees EOF, not ENOENT, when nothing was ever written to
/// the other end, and `wc`'s own all-zeros report for an empty input depends on actually being
/// able to open it. `Err` if nothing works out within `MAX_PIPE_NAME_ATTEMPTS` tries, `/tmp`
/// itself isn't there (`ensure_tmp_exists` should have already made sure of that, but this
/// doesn't assume it), or the file itself can't be created.
#[cfg(target_os = "none")]
fn next_pipe_path() -> Result<String, String> {
    #[allow(clippy::deref_addrof)]
    let counter = unsafe { &mut *(&raw mut PIPE_COUNTER) };
    let Some(vol) = fs::volume::get(0) else {
        return Err("system.img not mounted".into());
    };
    for _ in 0..MAX_PIPE_NAME_ATTEMPTS {
        let path = format!("{PIPE_PREFIX}{counter}");
        *counter += 1;
        if fs::path::resolve(vol, &path).is_err() {
            let Ok(writer) = fd::open_for_write(vol, &path, false) else {
                return Err(format!("{path}: cannot create"));
            };
            return match writer.finish() {
                Ok(()) => Ok(path),
                Err(_) => Err(format!("{path}: cannot create")),
            };
        }
    }
    Err("cannot create a unique temp file".into())
}

/// Removes every temp file in `paths`, ignoring errors -- there is nothing more useful to do
/// about a failed cleanup, and the pipeline is already finishing or aborting either way.
#[cfg(target_os = "none")]
fn cleanup_temps(paths: &[String]) {
    let Some(vol) = fs::volume::get(0) else {
        return;
    };
    for path in paths {
        if let Ok(entry) = fs::path::resolve(vol, path) {
            let _ = vol.delete(&entry);
        }
    }
}

/// Runs `pipeline`'s (more than one segment) stages in order: each stage's stdout feeds the next
/// stage's stdin through a temp file under `/tmp`, run to completion before the next one starts
/// -- no concurrency (this project's own "single resident program" scope already rules it out;
/// `yes | head` can't work here either). A stage's own redirections, if any, are layered *on top*
/// of its pipe binding (`apply_overrides`, inside `dispatch_segment`) -- the pipe connects a
/// stage to its neighbor by default, but a stage's own `<`/`>`/`>>`/`>&` for the same fd wins,
/// same as a real shell: `echo foo > bar | wc` sends `foo` to `bar`, not into the pipe, so `wc`
/// sees an empty pipe and reports all zeros.
///
/// Every stage runs through `run_segment` -- a builtin or a bare assignment is as valid a stage
/// as an external program now (`run_segment`'s own doc comment on why that's safe: it always
/// runs wrapped in `with_subshell`, whichever kind of stage it is). A launch-level failure (not
/// found, bad ELF, a bad redirect) aborts the rest, since the next stage would have no valid
/// input otherwise (`run_segment`'s `aborts`); a stage that ran but exited nonzero does not -- a
/// pipeline doesn't stop just because one stage found nothing to do. Every temp file this
/// pipeline created is removed on every exit path, via the one `cleanup_temps` call at the end.
///
/// Returns the exit status to report for `$?`: the *last stage that actually ran*'s own status
/// (bash's own default, no `pipefail` concept here either) -- including a stage that aborted the
/// rest (its own status is then also the pipeline's own final one, there being no later stage to
/// supersede it). `1` for a pipeline that never got any stage running at all (no temp file to
/// use).
#[cfg(target_os = "none")]
fn run_pipeline(uart: &mut Uart, pipeline: &[syntax::Segment], depth: usize) -> i32 {
    let n = pipeline.len();
    let mut temps: Vec<String> = Vec::new();
    for _ in 0..n - 1 {
        match next_pipe_path() {
            Ok(path) => temps.push(path),
            Err(msg) => {
                report(uart, &msg);
                cleanup_temps(&temps);
                return 1;
            }
        }
    }

    let mut status = 1;
    for (i, segment) in pipeline.iter().enumerate() {
        let mut stdio: [Option<StdioRedirect>; 3] = [None, None, None];
        if i > 0 {
            stdio[0] = Some(StdioRedirect::In(temps[i - 1].clone()));
        }
        if i + 1 < n {
            stdio[1] = Some(StdioRedirect::Out {
                path: temps[i].clone(),
                append: false,
            });
        }
        let (stage_status, aborts) = run_segment(uart, segment, depth, Some(stdio));
        status = stage_status;
        if aborts {
            break;
        }
    }

    cleanup_temps(&temps);
    status
}

/// Ensures `/tmp` exists on `system.img`, so a pipeline always has somewhere to put its temp
/// files -- called once, from `run_forever`, right alongside its other startup checks. Treats
/// `/tmp` already existing as a directory as success, the same as `mkdir -p`'s own single-prefix
/// step (`user/progs/src/bin/mkdir.rs`) -- this is the same idea, not the same code: this runs at
/// EL1 and calls `fs::path`/`vol.create_dir` directly rather than through a syscall, the same
/// reason `cd`'s builtin calls `fs::path::resolve_dir` directly instead of going through `open`.
#[cfg(target_os = "none")]
fn ensure_tmp_exists(uart: &mut Uart) {
    let Some(vol) = fs::volume::get(0) else {
        report(uart, "system.img not mounted, /tmp unavailable");
        return;
    };
    if fs::path::resolve_dir(vol, "/tmp").is_ok() {
        return; // already exists as a directory
    }
    let root = vol.root_dir();
    if vol.create_dir(&root, "tmp").is_err() {
        report(uart, "failed to create /tmp");
    }
}

/// Describes a `syntax::SyntaxError` for `report` -- never called with `EmptyCommand`, which
/// `run` (this function's only caller) already treats as "nothing to do" before reaching here.
#[cfg(target_os = "none")]
fn describe_syntax_error(err: syntax::SyntaxError) -> String {
    match err {
        syntax::SyntaxError::Lex(lexer::LexError::Unterminated(what)) => {
            format!("unterminated {what}")
        }
        syntax::SyntaxError::Lex(lexer::LexError::Unsupported(op)) => {
            format!("'{op}' is not supported yet")
        }
        syntax::SyntaxError::Lex(lexer::LexError::BadSubstitution) => "bad substitution".into(),
        syntax::SyntaxError::MissingTarget(op) => {
            format!("no file name after '{op}'")
        }
        syntax::SyntaxError::BadDupTarget => "a '>&' target must be 1 or 2".into(),
        syntax::SyntaxError::EmptyCommand => {
            unreachable!("run() handles this before calling describe_syntax_error")
        }
    }
}

/// Lexes and parses `line` (`syntax::parse`), then either runs it or reports why it can't be,
/// yet. A blank or comment-only line (`SyntaxError::EmptyCommand`) is silently ignored, matching
/// a real shell's own response to pressing Enter on nothing; every other parse failure is
/// reported the same way a failed launch already is. A pipeline of more than one segment runs
/// via `run_pipeline`; a single segment runs through `run_segment` directly here (`pipe_stdio:
/// None` -- its own doc comment covers the rest: assignment words, builtins, redirections,
/// launching a program). Any failure -- not mounted, not found, bad ELF, argv too large -- is
/// reported and swallowed; one bad command shouldn't take the REPL down.
///
/// `depth` is the script-nesting level this line is running at -- `0` from `run_forever`
/// (nothing has sourced anything yet), `source`'s own `depth + 1` for every line a sourced
/// script feeds back through here (`source`'s own doc comment, including the recursion guard
/// that depth is for).
///
/// Returns the exit status to report for `$?`, which this function also records itself
/// (`set_last_status`) before every return -- except for a blank or comment-only line
/// (`SyntaxError::EmptyCommand`), which leaves `$?` untouched and just hands back whatever it
/// already was, matching the lab's own documented rule. Status codes otherwise follow its table:
/// `2` for a syntax error, `127` for a program that couldn't be found at all, `126` for one that
/// was found but couldn't actually be launched (bad ELF, a bad redirect, ...), `0`/`1` for a
/// builtin succeeding/failing, and a launched program's own real exit code otherwise.
#[cfg(target_os = "none")]
fn run(uart: &mut Uart, line: &str, depth: usize) -> i32 {
    let pipeline = match syntax::parse(line) {
        Ok(pipeline) => pipeline,
        Err(syntax::SyntaxError::EmptyCommand) => return last_status(),
        Err(e) => {
            report(uart, &describe_syntax_error(e));
            set_last_status(2);
            return 2;
        }
    };

    let status = if pipeline.len() > 1 {
        run_pipeline(uart, &pipeline, depth)
    } else {
        // `pipeline` always has at least one segment -- `syntax::parse` never returns an empty
        // one (a pipeline with nothing in it is `SyntaxError::EmptyCommand`, handled above).
        run_segment(uart, &pipeline[0], depth, None).0
    };
    set_last_status(status);
    status
}

/// Reads one line at a time from the keyboard queue, echoing to the console, and runs whatever
/// name was typed against `usr/bin/` on `system.img`. Never returns -- replaces both the
/// boot-time hardcoded `hello`/`scratch` run and the dumb 15-second typing test that came before
/// it; see `plans/PORTING.md`.
///
/// Requires a console, GPU and keyboard to all be present -- without a display there's nothing
/// to run an interactive shell on, so this idles forever instead (`main.rs`'s own prior
/// behavior for the "no display" case).
#[cfg(target_os = "none")]
pub fn run_forever(uart: &mut Uart) -> ! {
    if console::get().is_none() || gpu::get().is_none() || input::get().is_none() {
        writeln!(uart, "No virtio-gpu or virtio-keyboard device found").ok();
        loop {
            unsafe { core::arch::asm!("wfe") }
        }
    }
    ensure_tmp_exists(uart);

    loop {
        // Scoped so `console`/`gpu_dev` are dropped before `run` below: it calls `run_program`,
        // whose EL0 program may itself reach `console::get()` (via `syscall::fd`'s `write`),
        // which would otherwise alias a `&mut Console` still held here across that call.
        let line = {
            let console = console::get().expect("checked above");
            let gpu_dev = gpu::get().expect("checked above");
            let discipline = line_discipline::get();
            discipline.begin(
                console,
                &format!("virtos:{}> ", fs::cwd::get()),
                Mode::Prompt,
            );
            gpu_dev.flush();
            loop {
                let Some(token) = keyboard::queue::pop() else {
                    keyboard::queue::wait_for_token();
                    continue;
                };
                match discipline.handle(token, console) {
                    LineOutcome::Ignored => {}
                    LineOutcome::Finished(line) => {
                        gpu_dev.flush();
                        break line;
                    }
                    // `Prompt` mode never produces `Partial`/`EndOfFile` (Ctrl+D is a no-op here:
                    // the shell never exits on EOF).
                    _ => gpu_dev.flush(),
                }
            }
        };

        let trimmed = line.trim();
        if !trimmed.is_empty() {
            run(uart, trimmed, 0);
        }
    }
}
