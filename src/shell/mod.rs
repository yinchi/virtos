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
//! `lexer`/`syntax`/`frame_stack`/`expand`/`prompt` are pure logic, no hardware dependency: built and
//! host-tested under both targets, like `fs::sector`/`exec::elfparse`. Everything below them
//! touches the console, keyboard, or filesystem, so it's bare-metal-only, like `exec::process`.

mod environment;
mod expand;
mod frame_stack;
mod fstab;
mod lexer;
mod path_search;
mod program_kind;
mod prompt;
mod syntax;

#[cfg(target_os = "none")]
mod launch;

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
use crate::syscall::fd::{self, Stdio, StdioRedirect};
#[cfg(target_os = "none")]
use abi::errno::{EINVAL, EISDIR, ENODEV, errmsg};
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

/// The builtin `name` is, if it is one -- checked before ever trying to resolve it as a command.
/// A builtin runs in the shell itself rather than as a program of its own, but still under its
/// own redirects and pipe bindings (`fd::with_inherited_stdio`): whatever a `source`d script runs
/// inherits them.
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

/// Makes `argv[1]` (or `$HOME`, with no argument; `/` if `HOME` is unset or empty) the working
/// directory, if it names a real directory. Unlike every other command, this can't be an
/// ordinary `usr/bin/` program: an EL0 process changing its own working directory would never be
/// visible to the shell that launched it (real Unix's `cd` is a shell builtin for exactly this
/// reason), and there's only one working directory to begin with here -- the shell's own, since
/// at most one program is ever resident (`fs::cwd`'s own doc comment).
///
/// `cd -` goes back to `$OLDPWD` and says where it went, as bash does; every successful `cd`
/// leaves the directory it came from in `OLDPWD` (an ordinary exported variable in the current
/// frame, so a child scope's own `cd` doesn't change its parent's `$OLDPWD`, just as it doesn't
/// change its working directory). The lab has neither.
#[cfg(target_os = "none")]
fn cd(uart: &mut Uart, argv: &[String]) -> i32 {
    let home = frame_stack::var("HOME").filter(|home| !home.is_empty());
    let back = argv.get(1).is_some_and(|a| a == "-");
    let target = if back {
        match frame_stack::var("OLDPWD").filter(|old| !old.is_empty()) {
            Some(old) => old,
            None => {
                report(uart, "cd: OLDPWD not set");
                return 1;
            }
        }
    } else {
        argv.get(1).map_or(home.unwrap_or("/"), String::as_str)
    };
    let absolute = fs::cwd::absolute(target);
    let Some((_, vol, rest)) = fs::path::volume_for(&absolute) else {
        report(uart, "no volume mounted there");
        return 1;
    };
    if let Err(e) = fs::path::resolve_dir(vol, rest) {
        report(uart, &format!("cd: {target}: {}", errmsg(e)));
        return 1;
    }
    let old = fs::cwd::get();
    fs::cwd::set(absolute.clone());
    // `OLDPWD` is a valid name, so this cannot fail.
    let _ = frame_stack::export_var("OLDPWD", Some(&old));
    if back {
        // Standard output, as in any shell: `cd - > f` and `$(cd -)` see it.
        fd::write_stdout(format!("{absolute}\n").as_bytes());
    }
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
    // `source` and `.` find a bare name through `$PATH` before the working directory (the file
    // need only be readable, not executable); `sh FILE` and any name with a `/` in it go straight
    // to the working directory.
    let searched = if name == "sh" {
        None
    } else {
        launch::search_script(path)
    };
    let absolute = searched.unwrap_or_else(|| fs::cwd::absolute(path));
    let Some((_, vol, rest)) = fs::path::volume_for(&absolute) else {
        report(uart, "no volume mounted there");
        return 1;
    };
    let entry = match fs::path::resolve(vol, rest) {
        Ok(entry) => entry,
        Err(EINVAL) => {
            report(uart, &format!("{name}: {path}: {}", errmsg(EISDIR))); // the root
            return 1;
        }
        Err(e) => {
            report(uart, &format!("{name}: {path}: {}", errmsg(e)));
            return 1;
        }
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
    run_script_content(uart, content, depth)
}

/// Runs each line of `content` as if typed at the prompt, one nesting level (`depth`) below
/// the caller's own, and returns the last line's status -- `0` for an empty file or one of nothing
/// but blank/comment lines (which leave `$?` alone). Unscoped: whatever the script sets, `cd`s or
/// exports stays set. Shared by `source`/`.`/`sh` and the start-up `.profile`.
#[cfg(target_os = "none")]
fn run_script_content(uart: &mut Uart, content: &str, depth: usize) -> i32 {
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

/// Turns a segment's redirections, in the order they were written, into the stdio bindings
/// (`fd`, what it becomes) the command is launched under: each `Redirection` names the fd it binds
/// (`In` always fd 0; `Out`/`Dup` carry their own, defaulted to 1 by `syntax`'s own parsing), and
/// a file target is expanded here -- variables and command substitutions, never split, and it
/// must come out as exactly one word (`expand::expand_target`). `Err` is the message to report
/// (a substitution that could not run, an `ambiguous redirect`); the command then does not run.
///
/// Appended after whatever a pipeline stage's own pipe binding already put there, so a stage's
/// own redirect for the same fd comes later and wins (the reason `echo foo > bar | wc` sends `foo`
/// to `bar`, not into the pipe, leaving `wc` to see nothing). Applied left to right
/// (`syscall::fd::resolve_streams`), so `> f 2>&1` and `2>&1 > f` differ, as in any shell.
#[cfg(target_os = "none")]
fn resolve_redirs(
    uart: &mut Uart,
    depth: usize,
    redirs: &[syntax::Redirection],
) -> Result<Stdio, String> {
    let mut stdio: Stdio = Vec::new();
    for redir in redirs {
        let mut subst = |text: &str| substitute(uart, text, depth);
        match redir {
            syntax::Redirection::In(word) => {
                let path = expand::expand_target(word, &var_lookup, last_status(), &mut subst)?;
                stdio.push((0, StdioRedirect::In(path)));
            }
            syntax::Redirection::Out { fd, path, append } => {
                let path = expand::expand_target(path, &var_lookup, last_status(), &mut subst)?;
                stdio.push((
                    usize::from(*fd),
                    StdioRedirect::Out {
                        path,
                        append: *append,
                    },
                ));
            }
            syntax::Redirection::Dup { fd, target } => {
                stdio.push((usize::from(*fd), StdioRedirect::Dup(*target)));
            }
        }
    }
    Ok(stdio)
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

/// The most a command substitution's output may be: past it the substitution fails rather than
/// risking the kernel heap (an allocation failure there is a panic).
#[cfg(target_os = "none")]
const SUBST_OUTPUT_MAX: usize = 1 << 20;

/// The status of the last command substitution that finished while the current command's words
/// were being expanded, `None` if none has: a command with no command word (`X=$(false)`) takes
/// its status from it. Reset at the start of every segment.
///
/// SAFETY (every access): single core, and the shell runs strictly sequentially.
#[cfg(target_os = "none")]
static mut LAST_SUBST_STATUS: Option<i32> = None;

#[cfg(target_os = "none")]
fn set_last_subst_status(status: Option<i32>) {
    // SAFETY: see LAST_SUBST_STATUS.
    unsafe { LAST_SUBST_STATUS = status };
}

#[cfg(target_os = "none")]
fn last_subst_status() -> Option<i32> {
    // SAFETY: see LAST_SUBST_STATUS.
    unsafe { LAST_SUBST_STATUS }
}

/// Runs the command substitution whose source is `text` and returns what it wrote to stdout (its
/// trailing newlines are `expand`'s business). A subshell: it runs in a copy of the current frame
/// (`frame_stack::with_subshell`), so its `cd`, `export` and assignments do not leak, with stdout
/// bound to a temp file in `/tmp` -- the pipe mechanism -- that is read back afterwards and
/// removed on every path out. stdin and stderr are the command's own. `$?` is as it was before the
/// substitution when this returns, and the substitution's own status is left in
/// `LAST_SUBST_STATUS`.
///
/// `Err` is what to report: output past `SUBST_OUTPUT_MAX`, nesting past `MAX_SCRIPT_DEPTH` (it
/// shares `source`'s counter, `depth`), no `/tmp` to put the capture in.
#[cfg(target_os = "none")]
fn substitute(uart: &mut Uart, text: &str, depth: usize) -> Result<String, String> {
    if depth >= MAX_SCRIPT_DEPTH {
        return Err("command substitution nested too deeply".into());
    }
    let list = match syntax::parse(text) {
        Ok(list) => list,
        Err(syntax::SyntaxError::EmptyCommand) => {
            set_last_subst_status(Some(0));
            return Ok(String::new());
        }
        // The line's own parse already checked every substitution, so this is only reachable if
        // that and this ever disagree.
        Err(e) => return Err(describe_syntax_error(e)),
    };
    let path = next_pipe_path()?;
    let outer_status = last_status();
    let stdio: Stdio = alloc::vec![(
        1,
        StdioRedirect::Out {
            path: path.clone(),
            append: false,
        }
    )];
    let ran = fd::with_inherited_stdio(stdio, || {
        frame_stack::with_subshell(|| {
            let mut status = 0;
            for and_or in &list {
                status = run_and_or(uart, and_or, depth + 1);
            }
            status
        })
    });
    // The commands inside set `$?` as they went; the outer command has not finished yet.
    set_last_status(outer_status);
    let output = match ran {
        Ok(status) => capture_text(&path).inspect(|_| set_last_subst_status(Some(status))),
        Err(msg) => Err(msg),
    };
    cleanup_temps(&[path]);
    output
}

/// The text of the capture file at `path`, at most `SUBST_OUTPUT_MAX` bytes: invalid UTF-8
/// becomes U+FFFD and NUL bytes are dropped.
#[cfg(target_os = "none")]
fn capture_text(path: &str) -> Result<String, String> {
    let Some((_, vol, rest)) = fs::path::volume_for(path) else {
        return Err("command substitution: no volume mounted".into());
    };
    let Ok(entry) = fs::path::resolve(vol, rest) else {
        return Err("command substitution: its output file is gone".into());
    };
    if entry.len() as usize > SUBST_OUTPUT_MAX {
        return Err("command substitution: output too large".into());
    }
    let Ok(mut reader) = vol.read_file(&entry) else {
        return Err("command substitution: cannot read its output".into());
    };
    let Ok(bytes) = reader.read_to_vec() else {
        return Err("command substitution: cannot read its output".into());
    };
    Ok(String::from_utf8_lossy(&bytes).replace('\0', ""))
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
/// `pipe_stdio` is a pipeline stage's own pipe binding, applied underneath the segment's
/// redirects. All of it is the command's stdio: a program starts with it already bound onto fds
/// 0-2, a script or a builtin runs with it as the stdio every command inside inherits
/// (`fd::with_inherited_stdio`). Returns the status for `$?`: the command's own, `127` for a
/// command not found, `126` for one found but not runnable, `1` for a redirect that could not be
/// opened (reported, and the command does not run).
#[cfg(target_os = "none")]
fn dispatch_segment(
    uart: &mut Uart,
    name: &str,
    argv: &[String],
    depth: usize,
    redirs: Stdio,
    pipe_stdio: Option<Stdio>,
) -> i32 {
    let mut stdio = pipe_stdio.unwrap_or_default();
    stdio.extend(redirs);
    if is_builtin(name) {
        // A builtin runs in the shell itself, but still under its own redirects: `cd` or `export`
        // with `> f` creates `f`, and `source script > out` collects what the script prints.
        return match fd::with_inherited_stdio(stdio, || dispatch_builtin(uart, name, argv, depth)) {
            Ok(code) => code,
            Err(msg) => {
                report(uart, &msg);
                1
            }
        };
    }
    let program = match launch::load(name) {
        Ok(program) => program,
        Err(failure) => {
            report(uart, &failure.message);
            return failure.status;
        }
    };
    let program = match program {
        launch::Program::Elf(bytes) => bytes,
        launch::Program::Script(text) => {
            return run_script_file(uart, name, argv, &text, depth, stdio);
        }
    };
    let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let env = exported_env();
    let env_refs: Vec<&str> = env.iter().map(String::as_str).collect();
    // The redirects are opened first, as the stdio the program inherits, so a failure to open one
    // says which file and why (`run_program` then binds the inherited streams, with nothing of
    // its own on top).
    let ran = fd::with_inherited_stdio(stdio, || {
        exec::process::run_program(&program, &argv_refs, &env_refs, Vec::new())
    });
    match ran {
        Ok(Ok(code)) => code,
        Ok(Err(e)) => {
            report(uart, &format!("{name}: {e:?}"));
            launch::STATUS_CANNOT_EXECUTE
        }
        Err(msg) => {
            report(uart, &msg);
            1
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
/// Returns the segment's status -- see `dispatch_segment`'s own doc comment.
#[cfg(target_os = "none")]
fn run_segment(
    uart: &mut Uart,
    segment: &syntax::Segment,
    depth: usize,
    pipe_stdio: Option<Stdio>,
) -> i32 {
    let is_pipeline_stage = pipe_stdio.is_some();
    let body = || -> i32 {
        set_last_subst_status(None);
        let (assignments, command_words) = split_assignments(&segment.words);
        // Left to right, each value already seeing the ones before it (`A=1 B=$A` makes `B` 1),
        // whether they end up as the shell's own variables or as the command's one-shot overlay.
        let mut overlay: Vec<(String, String)> = Vec::new();
        for (name, value_word) in assignments {
            let seen = |n: &str| {
                overlay
                    .iter()
                    .rev()
                    .find(|(k, _)| k == n)
                    .map(|(_, v)| v.clone())
                    .or_else(|| var_lookup(n))
            };
            let value = {
                let mut subst = |text: &str| substitute(uart, text, depth);
                expand::expand_assignment_value(&value_word, &seen, last_status(), &mut subst)
            };
            match value {
                Ok(value) => overlay.push((name.to_string(), value)),
                Err(msg) => {
                    report(uart, &msg);
                    return 1;
                }
            }
        }

        if command_words.is_empty() {
            for (name, value) in &overlay {
                frame_stack::set_var(name, value).ok();
            }
            // Only the redirects: `> f` alone creates (or truncates) `f`.
            let stdio = match resolve_redirs(uart, depth, &segment.redirs) {
                Ok(stdio) => stdio,
                Err(msg) => {
                    report(uart, &msg);
                    return 1;
                }
            };
            // A command that is only assignments has the status of its last substitution.
            return match fd::with_inherited_stdio(stdio, || ()) {
                Ok(()) => last_subst_status().unwrap_or(0),
                Err(msg) => {
                    report(uart, &msg);
                    1
                }
            };
        }

        let expanded = {
            let mut subst = |text: &str| substitute(uart, text, depth);
            expand::expand_words(command_words, &var_lookup, last_status(), &mut subst)
        };
        let argv = match expanded {
            Ok(argv) => argv,
            Err(msg) => {
                report(uart, &msg);
                return 1;
            }
        };
        let redirs = match resolve_redirs(uart, depth, &segment.redirs) {
            Ok(redirs) => redirs,
            Err(msg) => {
                report(uart, &msg);
                return 1;
            }
        };
        // Every word may have expanded away (`$EMPTY`, `$(true)`): no command word, so what is
        // left is the assignments and redirections, which have already been dealt with the same
        // way the all-assignments case above does.
        let Some(name) = argv.first().map(String::as_str) else {
            // As in a command that was only assignments: they stick.
            for (name, value) in &overlay {
                frame_stack::set_var(name, value).ok();
            }
            return match fd::with_inherited_stdio(redirs, || ()) {
                Ok(()) => last_subst_status().unwrap_or(0),
                Err(msg) => {
                    report(uart, &msg);
                    1
                }
            };
        };
        // `with_overlay`'s own restore unconditionally puts the *whole* variable list back,
        // overlay or not -- exactly right for undoing the overlay itself, but it would just as
        // happily undo a builtin's own *permanent* mutation (`export`/`unset`/a `source`d
        // script's own `export`) if nothing actually needed overlaying in the first place. Only
        // wrap when there's a real prefix to apply and later undo; skip it entirely otherwise, so
        // `export FOO=bar` (no prefix) really does stick.
        if overlay.is_empty() {
            dispatch_segment(uart, name, &argv, depth, redirs, pipe_stdio)
        } else {
            frame_stack::with_overlay(&overlay, || {
                dispatch_segment(uart, name, &argv, depth, redirs, pipe_stdio)
            })
        }
    };

    if is_pipeline_stage {
        frame_stack::with_subshell(body)
    } else {
        body()
    }
}

/// Runs a script file found as a command (`./script`, or a name on `$PATH`; `launch::load` has
/// already checked the exec bit and that it is text): in a child scope, as a real child shell
/// process would -- so its own `cd`, `export` and assignments don't leak back, unlike `source`'s --
/// and under the command's own `stdio` (its redirects and, as a pipeline stage, its pipe
/// binding), which every command inside it inherits (`fd::with_inherited_stdio`). Returns
/// `dispatch_segment`'s status: the script's own once it has run, `126` if it was refused.
///
/// A script takes no arguments: there are no positional parameters yet to hand them to.
#[cfg(target_os = "none")]
fn run_script_file(
    uart: &mut Uart,
    name: &str,
    argv: &[String],
    text: &str,
    depth: usize,
    stdio: Stdio,
) -> i32 {
    let refuse = |uart: &mut Uart, why: &str| {
        report(uart, &format!("{name}: {why}"));
        launch::STATUS_CANNOT_EXECUTE
    };
    if argv.len() > 1 {
        return refuse(uart, "too many arguments");
    }
    if depth >= MAX_SCRIPT_DEPTH {
        return refuse(uart, "too many levels of scripts");
    }
    let ran = fd::with_inherited_stdio(stdio, || {
        frame_stack::with_child(|| run_script_content(uart, text, depth))
    });
    match ran {
        Ok(status) => status,
        Err(msg) => {
            report(uart, &msg);
            1
        }
    }
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
    for _ in 0..MAX_PIPE_NAME_ATTEMPTS {
        let path = format!("{PIPE_PREFIX}{counter}");
        *counter += 1;
        let Some((_, vol, rest)) = fs::path::volume_for(&path) else {
            return Err("no volume mounted".into());
        };
        if fs::path::resolve(vol, rest).is_err() {
            let (_, writer) =
                fd::open_for_write(&path, false).map_err(|e| format!("{path}: {}", errmsg(e)))?;
            return match writer.finish() {
                Ok(()) => Ok(path),
                Err(e) => Err(format!("{path}: {}", errmsg(fs::errno::from_fat(&e)))),
            };
        }
    }
    Err("cannot create a unique temp file".into())
}

/// Removes every temp file in `paths`, ignoring errors -- there is nothing more useful to do
/// about a failed cleanup, and the pipeline is already finishing or aborting either way.
#[cfg(target_os = "none")]
fn cleanup_temps(paths: &[String]) {
    for path in paths {
        let Some((_, vol, rest)) = fs::path::volume_for(path) else {
            continue;
        };
        if let Ok(entry) = fs::path::resolve(vol, rest) {
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
/// found, bad ELF, a bad redirect) does not stop the pipeline either, as in bash: the stage just
/// produced nothing, and the next one reads an empty pipe. Every temp file this pipeline created
/// is removed on every exit path, via the one `cleanup_temps` call at the end.
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
        let mut stdio: Stdio = Vec::new();
        if i > 0 {
            stdio.push((0, StdioRedirect::In(temps[i - 1].clone())));
        }
        if i + 1 < n {
            stdio.push((
                1,
                StdioRedirect::Out {
                    path: temps[i].clone(),
                    append: false,
                },
            ));
        }
        status = run_segment(uart, segment, depth, Some(stdio));
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
    let Some((_, vol, rest)) = fs::path::volume_for("/tmp") else {
        report(uart, "no root volume mounted, /tmp unavailable");
        return;
    };
    if fs::path::resolve_dir(vol, rest).is_ok() {
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
        syntax::SyntaxError::MissingCommand => "no command next to an operator".into(),
        syntax::SyntaxError::Incomplete(op) => format!("no command after '{op}'"),
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
    let list = match syntax::parse(line) {
        Ok(list) => list,
        Err(syntax::SyntaxError::EmptyCommand) => return last_status(),
        Err(e) => {
            report(uart, &describe_syntax_error(e));
            set_last_status(2);
            return 2;
        }
    };

    // `;`-separated entries run one after another whatever each one's status; the line's status
    // is the last one that ran.
    let mut status = 0;
    for and_or in &list {
        status = run_and_or(uart, and_or, depth);
    }
    status
}

/// Runs one `command [&& | || command]...` chain left to right. Each connector looks at the
/// status of everything to its left (`&&` runs the next command on 0, `||` on non-zero); a command
/// it skips leaves that status as it was, so `false && x || y` runs `y`, and `true || x && y`
/// runs `y` too. `$?` is updated after every command that runs, so each one sees its
/// predecessor's. Returns the chain's status.
#[cfg(target_os = "none")]
fn run_and_or(uart: &mut Uart, and_or: &syntax::AndOr, depth: usize) -> i32 {
    let mut status = run_command(uart, &and_or.first, depth);
    set_last_status(status);
    for (connector, command) in &and_or.rest {
        let runs = match connector {
            syntax::Connector::And => status == 0,
            syntax::Connector::Or => status != 0,
        };
        if runs {
            status = run_command(uart, command, depth);
            set_last_status(status);
        }
    }
    status
}

/// Runs one pipeline, negating its status if a `!` led it (`0` becomes `1`, anything else `0`).
/// A pipeline of more than one segment runs via `run_pipeline`; a single segment runs through
/// `run_segment` directly (`pipe_stdio: None` -- its own doc comment covers assignment words,
/// builtins, redirections and launching a program).
#[cfg(target_os = "none")]
fn run_command(uart: &mut Uart, command: &syntax::Command, depth: usize) -> i32 {
    let pipeline = &command.pipeline;
    let status = if pipeline.len() > 1 {
        run_pipeline(uart, pipeline, depth)
    } else {
        // `syntax::parse` never returns an empty pipeline (a stage with nothing in it is a
        // syntax error), so there is always a first segment.
        run_segment(uart, &pipeline[0], depth, None)
    };
    if command.negated {
        i32::from(status == 0)
    } else {
        status
    }
}

/// The environment file: plain `NAME=VALUE` lines (see `environment.rs`), read once at boot.
#[cfg(target_os = "none")]
const ENVIRONMENT_FILE: &str = "/etc/environment";

/// More than this is not an environment file or a profile: it is ignored rather than parsed or
/// run.
#[cfg(target_os = "none")]
const START_UP_FILE_MAX: usize = 64 * 1024;

/// What `read_text` found at a path.
#[cfg(target_os = "none")]
enum TextFile {
    /// Nothing there (or a component of the path is not a directory).
    Missing,
    Text(String),
    /// Something is there but it can't be used: the reason, for the serial note.
    Unusable(&'static str),
}

/// Reads the file at the absolute path `path` as UTF-8 text of at most `START_UP_FILE_MAX` bytes.
#[cfg(target_os = "none")]
fn read_text(path: &str) -> TextFile {
    let Some((_, vol, rest)) = fs::path::volume_for(path) else {
        return TextFile::Unusable("no volume mounted there");
    };
    let Ok(entry) = fs::path::resolve(vol, rest) else {
        return TextFile::Missing;
    };
    if entry.is_directory() {
        return TextFile::Unusable("is a directory");
    }
    if entry.len() as usize > START_UP_FILE_MAX {
        return TextFile::Unusable("not a text file of at most 64 KiB");
    }
    let Ok(mut reader) = vol.read_file(&entry) else {
        return TextFile::Unusable("failed to open");
    };
    let Ok(bytes) = reader.read_to_vec() else {
        return TextFile::Unusable("failed to read");
    };
    match String::from_utf8(bytes) {
        Ok(text) => TextFile::Text(text),
        Err(_) => TextFile::Unusable("not a text file of at most 64 KiB"),
    }
}

/// The file systems to mount at start-up: `<source> <mount point> <type> <options>` lines (see
/// `fstab.rs`).
#[cfg(target_os = "none")]
const FSTAB_FILE: &str = "/etc/fstab";

/// Mounts what `/etc/fstab` lists, in file order -- the shell's part of what Linux splits into
/// `root=` (the kernel mounts the root) and `mount -a` (init mounts the rest) -- and reports on
/// the UART (the serial log). Nothing here is fatal: a missing or unusable file mounts nothing, a
/// bad line is skipped (`fstab::parse`), and a line that cannot be mounted (no volume matches, the
/// mount point is not a directory or is in use, ...) is skipped with the reason. `noauto` lines
/// are left alone and a `nofail` line says nothing if no volume matches. A line for `/` is only
/// checked: the root was chosen at boot, by label (`fs::devices`), before this file could be read,
/// so it must name that volume or it is noted and ignored.
#[cfg(target_os = "none")]
fn mount_fstab(uart: &mut Uart) {
    let text = match read_text(FSTAB_FILE) {
        TextFile::Text(text) => text,
        TextFile::Missing => {
            writeln!(uart, "Fstab: {FSTAB_FILE}: not found -- nothing to mount.").ok();
            return;
        }
        TextFile::Unusable(why) => {
            writeln!(uart, "Fstab: {FSTAB_FILE}: {why} -- nothing to mount.").ok();
            return;
        }
    };
    let parsed = fstab::parse(&text);
    for problem in &parsed.problems {
        writeln!(
            uart,
            "Fstab: {FSTAB_FILE}: line {}: {} -- ignored.",
            problem.line, problem.why
        )
        .ok();
    }
    let mut mounted = 0;
    for entry in parsed.entries.iter().filter(|entry| !entry.noauto) {
        let point = fs::cwd::absolute(&entry.point);
        let what = format!("{} on {point}", entry.source_text);
        let mut note = |result: &str| {
            writeln!(
                uart,
                "Fstab: {FSTAB_FILE}: line {}: {what}: {result}.",
                entry.line
            )
            .ok();
        };
        if point == "/" {
            let root = fs::mounts::table().resolve("/").0;
            if entry.source.find(&fs::devices::identities()) != Ok(root) {
                note(
                    "the root is chosen at boot (the volume labelled SYSTEM), not from this file -- ignored",
                );
            }
            continue;
        }
        match fs::mounts::mount(&entry.source_text, &point) {
            0 => mounted += 1,
            abi::errno::ENODEV if entry.nofail => {}
            abi::errno::ENODEV => note("no volume has that label or ID -- skipped"),
            abi::errno::EINVAL => note("not a FAT filesystem -- skipped"),
            abi::errno::ENOTUNIQ => {
                note(
                    "more than one volume has that label or ID -- skipped (UUID= or unmount the twin)",
                );
            }
            e => note(&format!("{} -- skipped", abi::errno::errmsg(e))),
        }
    }
    writeln!(uart, "Fstab: {mounted} mount(s) from {FSTAB_FILE}.").ok();
}

/// Reads `/etc/environment` into the shell's variables, every one exported, and reports what it
/// did on the UART (the serial log, not the console): a missing or unusable file is an empty
/// environment, and a bad line is skipped, never fatal.
#[cfg(target_os = "none")]
fn load_environment(uart: &mut Uart) {
    let text = match read_text(ENVIRONMENT_FILE) {
        TextFile::Text(text) => text,
        TextFile::Missing => {
            writeln!(
                uart,
                "Environment: {ENVIRONMENT_FILE}: not found -- starting empty."
            )
            .ok();
            return;
        }
        TextFile::Unusable(why) => {
            writeln!(
                uart,
                "Environment: {ENVIRONMENT_FILE}: {why} -- starting empty."
            )
            .ok();
            return;
        }
    };
    let parsed = environment::parse(&text);
    for problem in &parsed.problems {
        writeln!(
            uart,
            "Environment: {ENVIRONMENT_FILE}: line {}: {} -- ignored.",
            problem.line, problem.why
        )
        .ok();
    }
    for (name, value) in &parsed.vars {
        let _ = frame_stack::export_var(name, Some(value)); // names were validated by `parse`
    }
    writeln!(
        uart,
        "Environment: {} variable(s) from {ENVIRONMENT_FILE}.",
        parsed.vars.len()
    )
    .ok();
}

/// `$HOME` as a path, if it is set and not empty.
#[cfg(target_os = "none")]
fn home_dir() -> Option<String> {
    frame_stack::var("HOME")
        .filter(|home| !home.is_empty())
        .map(String::from)
}

/// Starts the shell in `$HOME`, as a login program would: `chdir` to it. There is no login or
/// user system, so this is the init shell's own doing, once, at boot, after the environment is
/// loaded. No `HOME` (or an empty one) leaves the shell in `/`; one that names no directory is
/// reported on the UART and does the same.
#[cfg(target_os = "none")]
fn enter_home(uart: &mut Uart) {
    let Some(home) = home_dir() else { return };
    let absolute = fs::cwd::absolute(&home);
    let entered = fs::path::volume_for(&absolute)
        .ok_or(ENODEV)
        .and_then(|(_, vol, rest)| fs::path::resolve_dir(vol, rest).map(|_| ()));
    match entered {
        Ok(()) => fs::cwd::set(absolute),
        Err(e) => {
            writeln!(
                uart,
                "Environment: cannot enter HOME={home}: {} -- staying in /.",
                errmsg(e)
            )
            .ok();
        }
    }
}

/// Runs `$HOME/.profile`, the user's start-up script, if there is one: its lines run in the shell
/// itself, as `source` would (so its assignments, `export`s, `cd`s and `PS1` changes are the
/// shell's own), and what it prints appears before the first prompt. No `HOME` or no such file is
/// silent -- a profile is optional, unlike the environment file. One that is a directory, is not
/// UTF-8 text, or is over `START_UP_FILE_MAX` is a note on the UART and is skipped. A failing
/// line reports and the script goes on, as any script does. `$?` is left as the profile's last
/// command set it, as in bash and dash.
#[cfg(target_os = "none")]
fn run_profile(uart: &mut Uart) {
    let Some(home) = home_dir() else { return };
    let path = fs::cwd::absolute(&format!("{}/.profile", home.trim_end_matches('/')));
    match read_text(&path) {
        TextFile::Missing => {}
        TextFile::Unusable(why) => {
            writeln!(uart, "Profile: {path}: {why} -- skipped.").ok();
        }
        TextFile::Text(content) => {
            run_script_content(uart, &content, 0);
        }
    }
}

/// What the init shell does for itself before it reads its first line, in this order: seed `PS1`,
/// mount the file systems in `/etc/fstab` (before anything else, so that `/etc/environment` and
/// `$HOME` may live on one), load the environment (which may replace `PS1`), enter `$HOME`, run its `.profile`. The notes go to
/// the UART (the serial log); what the profile prints goes to the console. All of it comes before
/// the first prompt is drawn.
#[cfg(target_os = "none")]
fn start_up(uart: &mut Uart) {
    // The prompt this shell has always shown, now just a variable like any other: `PS1=` or
    // `unset PS1` falls back to `prompt::DEFAULT_PROMPT`. Seeded first so `/etc/environment` and
    // `.profile` can replace it.
    let _ = frame_stack::set_var("PS1", prompt::DEFAULT_SHELL_PS1);
    mount_fstab(uart);
    load_environment(uart);
    enter_home(uart);
    run_profile(uart);
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
    start_up(uart);

    loop {
        // Scoped so `console`/`gpu_dev` are dropped before `run` below: it calls `run_program`,
        // whose EL0 program may itself reach `console::get()` (via `syscall::fd`'s `write`),
        // which would otherwise alias a `&mut Console` still held here across that call.
        let line = {
            let console = console::get().expect("checked above");
            let gpu_dev = gpu::get().expect("checked above");
            let discipline = line_discipline::get();
            let prompt = prompt::render(frame_stack::var("PS1"), &fs::cwd::get());
            discipline.begin(console, &prompt, Mode::Prompt);
            gpu_dev.flush();
            // The serial transcript gets the prompt too (the typed line follows it, as in a
            // terminal's own log): a log that ends with the prompt means the shell is ready for
            // the next line, which is what the QEMU test harness waits for.
            write!(uart, "{prompt}").ok();
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
