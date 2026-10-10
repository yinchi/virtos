//! `rmdir [-p] [-v] [--ignore-fail-on-non-empty] DIR...`: removes empty directories, the safe
//! counterpart of `rm -r`. Every operand is tried; the status is 1 if any failed. An operand that
//! is not a directory is refused up front (`unlink` alone would remove a plain file), and a
//! directory with anything in it fails in the kernel's `unlink` (the filesystem refuses to delete
//! a non-empty one).
//!
//! `-p` removes each operand's parents too (`a/b/c` is `c`, then `a/b`, then `a`), stopping at
//! the first that cannot be removed; `-v` says each directory it removes;
//! `--ignore-fail-on-non-empty` stays quiet, with status 0, about a directory that was not empty.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use getargs::Arg;
use progs::cli;
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "rmdir [-p] [-v] [--ignore-fail-on-non-empty] DIR...";
const FLAGS: &[(&str, &str)] = &[
    ("-p", "also remove each operand's parent directories"),
    ("-v", "say each directory as it is removed"),
    (
        "--ignore-fail-on-non-empty",
        "do not report a directory that is not empty",
    ),
];

fn run(args: Args) -> ExitCode {
    cli::status(rmdir(args))
}

/// Removes the directory `dir` (which must be one), `Err(errno)` if it cannot be.
fn remove(dir: &str) -> Result<(), isize> {
    match userlib::stat(dir) {
        Err(e) => Err(e),
        Ok(info) if !info.is_dir => Err(abi::errno::ENOTDIR),
        Ok(_) => match userlib::unlink(dir) {
            r if r < 0 => Err(r),
            _ => Ok(()),
        },
    }
}

fn rmdir(args: Args) -> Result<ExitCode, ExitCode> {
    let (mut parents, mut verbose, mut ignore_non_empty) = (false, false, false);
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("rmdir", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(progs::help(USAGE, FLAGS)),
            Arg::Short('p') | Arg::Long("parents") => parents = true,
            Arg::Short('v') | Arg::Long("verbose") => verbose = true,
            Arg::Long("ignore-fail-on-non-empty") => ignore_non_empty = true,
            Arg::Positional(_) => {}
            other => return Err(cli::invalid("rmdir", other)),
        }
    }

    let mut any = false;
    let mut status = 0;
    for operand in cli::operands(args) {
        any = true;
        // The operand itself and, with `-p`, each ancestor in turn: `a/b/c`, `a/b`, `a`.
        let mut dir = operand.trim_end_matches('/');
        if dir.is_empty() {
            dir = operand; // `/` (or `//`): left as written, for the kernel to refuse
        }
        loop {
            match remove(dir) {
                Ok(()) => {
                    if verbose {
                        cli::say(&format!("rmdir: removing directory, '{dir}'"));
                    }
                }
                Err(e) => {
                    // GNU keeps quiet about a non-empty directory under the option -- and
                    // then does not count it as a failure either.
                    if !(ignore_non_empty && e == abi::errno::ENOTEMPTY) {
                        cli::report_errno("rmdir", &format!("failed to remove '{dir}'"), e);
                        status = 1;
                    }
                    break;
                }
            }
            if !parents {
                break;
            }
            // The parent is everything before the last `/`, skipping any run of them; nothing
            // left (a relative path with no more components, or the root) ends the walk.
            let Some(slash) = dir.rfind('/') else { break };
            let parent = dir[..slash].trim_end_matches('/');
            if parent.is_empty() {
                break;
            }
            dir = parent;
        }
    }
    if !any {
        return Err(cli::missing_operand("rmdir"));
    }
    Ok(ExitCode(status))
}
