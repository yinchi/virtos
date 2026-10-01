//! A kernel-side line-reading REPL: type a name, press Enter, run it. Living at EL1
//! (`crate::shell`, the same place the lab's own real shell lives) specifically so it needs
//! neither `userlib` nor argv/envp to exist first. `lexer`/`syntax` give it a real grammar --
//! quoting, escaping, pipes, redirection -- and `run`/`run_pipeline` (below) execute all of it
//! for real: a single command's own redirections (`syscall::fd::bind_stdio`, for a program;
//! `apply_redirects_standalone` here, for `cd` or a segment with no command at all), and a
//! multi-stage pipeline via temp files under `/tmp` (`run_pipeline`), a stage's own redirections
//! layered on top of its pipe binding the same way a real shell does (`apply_overrides`'s own
//! doc comment). `cd` is its one builtin (`fs::cwd`'s own doc comment explains why it has to
//! be): everything else is looked up under `usr/bin/` and run as a real EL0 program.
//!
//! `lexer`/`syntax` are pure logic, no hardware dependency: built and host-tested under both
//! targets, like `fs::sector`/`exec::elfparse`. Everything below them touches the console,
//! keyboard, or filesystem, so it's bare-metal-only, like `exec::process`.

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
use crate::syscall::fd::{self, StdioRedirect};
#[cfg(target_os = "none")]
use alloc::format;
#[cfg(target_os = "none")]
use alloc::string::String;
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

/// Makes `argv[1]` (or `/`, with no argument -- there's no `$HOME` to fall back to yet) the
/// working directory, if it names a real directory. Unlike every other command, this can't be an
/// ordinary `usr/bin/` program: an EL0 process changing its own working directory would never be
/// visible to the shell that launched it (real Unix's `cd` is a shell builtin for exactly this
/// reason), and there's only one working directory to begin with here -- the shell's own, since
/// at most one program is ever resident (`fs::cwd`'s own doc comment).
#[cfg(target_os = "none")]
fn cd(uart: &mut Uart, argv: &[String]) {
    let target = argv.get(1).map_or("/", String::as_str);
    let Some(vol) = fs::volume::get(0) else {
        report(uart, "system.img not mounted");
        return;
    };
    let absolute = fs::cwd::absolute(target);
    if fs::path::resolve_dir(vol, &absolute).is_err() {
        report(uart, &format!("cd: {target}: not a directory"));
        return;
    }
    fs::cwd::set(absolute);
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
/// of its pipe binding (`apply_overrides`) -- the pipe connects a stage to its neighbor by
/// default, but a stage's own `<`/`>`/`>>`/`>&` for the same fd wins, same as a real shell:
/// `echo foo > bar | wc` sends `foo` to `bar`, not into the pipe, so `wc` sees an empty pipe and
/// reports all zeros.
///
/// Only a stage that resolves to a real external program runs here: `cd` or an empty-argv stage
/// (only redirections, no command) has no fd table to connect to the pipe in the first place
/// (`apply_redirects_standalone`'s whole reason for existing doesn't extend to "and also wire
/// this into the pipe"), so either aborts the whole pipeline with a clear message instead of
/// guessing at an undefined interaction. A launch-level failure (not found, bad ELF, ...) aborts
/// the rest too, since the next stage would have no valid input otherwise; a launched program's
/// own nonzero exit does not -- a pipeline doesn't stop just because one stage found nothing to
/// do. Every temp file this pipeline created is removed on every exit path, via the one
/// `cleanup_temps` call at the end.
#[cfg(target_os = "none")]
fn run_pipeline(uart: &mut Uart, pipeline: &[syntax::Segment]) {
    let n = pipeline.len();
    let mut temps: Vec<String> = Vec::new();
    for _ in 0..n - 1 {
        match next_pipe_path() {
            Ok(path) => temps.push(path),
            Err(msg) => {
                report(uart, &msg);
                cleanup_temps(&temps);
                return;
            }
        }
    }

    for (i, segment) in pipeline.iter().enumerate() {
        let argv = &segment.argv;
        if argv.is_empty() || argv[0] == "cd" {
            report(
                uart,
                "cd, or a stage with no command, is not supported in a pipeline",
            );
            break;
        }

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
        let stdio = apply_overrides(stdio, &segment.redirs);

        let name = argv[0].as_str();
        let program = match read_program(name) {
            Ok(program) => program,
            Err(msg) => {
                report(uart, &msg);
                break;
            }
        };
        let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        if let Err(e) = exec::process::run_program(&program, &argv_refs, &[], stdio) {
            report(uart, &format!("{name}: {e:?}"));
            break;
        }
    }

    cleanup_temps(&temps);
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
/// via `run_pipeline`; a single segment runs directly here, with its redirections applied first,
/// uniformly, before whatever follows -- `cd`, an external program, or nothing at all, for an
/// empty-argv segment of only redirections -- the same ordering a real shell uses, and the
/// reason `cd foo > bar` creates `bar` relative to where the shell started rather than `foo`
/// (`apply_redirects_standalone`'s own doc comment). `cd` stays a builtin; anything else looks up
/// `usr/bin/<argv[0]>` on `system.img` (blk0) and runs it at EL0 to completion, with an empty
/// environment (nothing produces real environment variables yet). Any other failure -- not
/// mounted, not found, bad ELF, argv too large -- is reported and swallowed; one bad command
/// shouldn't take the REPL down.
#[cfg(target_os = "none")]
fn run(uart: &mut Uart, line: &str) {
    let pipeline = match syntax::parse(line) {
        Ok(pipeline) => pipeline,
        Err(syntax::SyntaxError::EmptyCommand) => return,
        Err(e) => {
            report(uart, &describe_syntax_error(e));
            return;
        }
    };

    if pipeline.len() > 1 {
        run_pipeline(uart, &pipeline);
        return;
    }
    // `pipeline` always has at least one segment -- `syntax::parse` never returns an empty one
    // (a pipeline with nothing in it is `SyntaxError::EmptyCommand`, handled above).
    let segment = &pipeline[0];
    let argv = &segment.argv;
    // `cd` never reaches `run_program`, so it never reaches `bind_stdio` either -- same for an
    // empty-argv segment, which has no program to launch at all. Both apply their redirections
    // here instead, directly; anything else still gets its redirections (if any) via
    // `apply_overrides`/`bind_stdio`, below, once it actually launches.
    let launches_program = !argv.is_empty() && argv[0] != "cd";

    if !segment.redirs.is_empty()
        && !launches_program
        && let Err(msg) = apply_redirects_standalone(&segment.redirs)
    {
        report(uart, &msg);
        return;
    }

    if argv.is_empty() {
        return; // a bare `> f` -- POSIX creates the file, runs nothing
    }
    let name = argv[0].as_str();

    if name == "cd" {
        cd(uart, argv);
        return;
    }

    let program = match read_program(name) {
        Ok(program) => program,
        Err(msg) => {
            report(uart, &msg);
            return;
        }
    };

    let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let stdio = apply_overrides([None, None, None], &segment.redirs);
    if let Err(e) = exec::process::run_program(&program, &argv_refs, &[], stdio) {
        report(uart, &format!("{name}: {e:?}"));
    }
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

    let mut line = String::new();
    loop {
        // Scoped so `console`/`gpu_dev` are dropped before `run` below: it calls `run_program`,
        // whose EL0 program may itself reach `console::get()` (via `syscall::fd`'s `write`),
        // which would otherwise alias a `&mut Console` still held here across that call.
        {
            let console = console::get().expect("checked above");
            let gpu_dev = gpu::get().expect("checked above");
            console.puts(&format!("virtos:{}> ", fs::cwd::get()), FG, BG);
            gpu_dev.flush();
            loop {
                let Some(ch) = keyboard::queue::pop() else {
                    continue;
                };
                match ch {
                    '\n' => break,
                    '\x08' => {
                        if line.pop().is_some() {
                            console.putc(ch, FG, BG);
                            gpu_dev.flush();
                        }
                    }
                    _ => {
                        line.push(ch);
                        console.putc(ch, FG, BG);
                        gpu_dev.flush();
                    }
                }
            }
            console.putc('\n', FG, BG);
            gpu_dev.flush();
        }

        let trimmed = line.trim();
        if !trimmed.is_empty() {
            run(uart, trimmed);
        }
        line.clear();
    }
}
