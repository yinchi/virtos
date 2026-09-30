//! A kernel-side line-reading REPL: type a name, press Enter, run it. Living at EL1
//! (`crate::shell`, the same place the lab's own real shell lives) specifically so it needs
//! neither `userlib` nor argv/envp to exist first. `lexer`/`syntax` give it a real grammar --
//! quoting, escaping, pipes, redirection; `run` (below) now executes a single command's
//! redirections for real (`syscall::fd::bind_stdio`, for a program; `apply_redirects_standalone`
//! here, for `cd` or a segment with no command at all) -- only a *pipeline* (more than one
//! `|`-separated stage) still parses correctly but is reported as not yet supported rather than
//! run, one build-order step at a time (`plans/USERSPACE_PLAN.md`'s step 10). `cd` is its one
//! builtin (`fs::cwd`'s own doc comment explains why it has to be): everything else is looked up
//! under `usr/bin/` and run as a real EL0 program.
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

/// Builds `syscall::fd::bind_stdio`'s override array from `redirs` -- each `Redirection` names
/// the fd it binds (`In` always fd 0; `Out`/`Dup` carry their own, defaulted to 1 by `syntax`'s
/// own parsing), so this is a plain sort into array slots, in order (a later redirection of the
/// same fd overwrites an earlier one in the same slot -- see `bind_stdio`'s own doc comment for
/// what that means for `Dup`).
#[cfg(target_os = "none")]
fn build_overrides(redirs: &[syntax::Redirection]) -> [Option<StdioRedirect>; 3] {
    let mut overrides: [Option<StdioRedirect>; 3] = [None, None, None];
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
/// reported the same way a failed launch already is. More than one pipeline segment is reported
/// as not yet supported rather than partially executed -- see `plans/USERSPACE_PLAN.md`'s step
/// 10. For a single segment: redirections are applied first, uniformly, before whatever follows
/// -- `cd`, an external program, or nothing at all, for an empty-argv segment of only
/// redirections -- the same ordering a real shell uses, and the reason `cd foo > bar` creates
/// `bar` relative to where the shell started rather than `foo` (`apply_redirects_standalone`'s
/// own doc comment). `cd` stays a builtin; anything else looks up `usr/bin/<argv[0]>` on
/// `system.img` (blk0) and runs it at EL0 to completion, with an empty environment (nothing
/// produces real environment variables yet). Any other failure -- not mounted, not found, bad
/// ELF, argv too large -- is reported and swallowed; one bad command shouldn't take the REPL down.
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
        report(uart, "pipes are not supported yet");
        return;
    }
    // `pipeline` always has at least one segment -- `syntax::parse` never returns an empty one
    // (a pipeline with nothing in it is `SyntaxError::EmptyCommand`, handled above).
    let segment = &pipeline[0];
    let argv = &segment.argv;
    // `cd` never reaches `run_program`, so it never reaches `bind_stdio` either -- same for an
    // empty-argv segment, which has no program to launch at all. Both apply their redirections
    // here instead, directly; anything else still gets its redirections (if any) via
    // `build_overrides`/`bind_stdio`, below, once it actually launches.
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

    let Some(vol) = fs::volume::get(0) else {
        report(uart, "system.img not mounted");
        return;
    };
    let Ok(entry) = fs::path::resolve(vol, &format!("usr/bin/{name}")) else {
        report(uart, &format!("{name}: not found"));
        return;
    };

    let Ok(mut reader) = vol.read_file(&entry) else {
        report(uart, &format!("{name}: failed to open"));
        return;
    };
    let Ok(program) = reader.read_to_vec() else {
        report(uart, &format!("{name}: failed to read"));
        return;
    };

    let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let stdio = build_overrides(&segment.redirs);
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
