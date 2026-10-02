//! `rm [-r] PATH...` -- removes each named file, or directory. Without `-r`/`-R`/`--recursive`, a
//! non-empty directory is a reported error, not silently skipped or recursed into; with it, the
//! directory and everything under it is removed. `-f`/`--force` ignores an operand that does not
//! exist (no report, no failing status) and a missing operand list; the `.`/`..`/`/` guard still
//! applies. No `-i`/prompting.
#![no_std]
#![no_main]

extern crate alloc;

use getargs::Arg;
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "rm [-r] [-f] PATH...";
const FLAGS: &[(&str, &str)] = &[
    (
        "-r, -R, --recursive",
        "remove directories and their contents",
    ),
    ("-f, --force", "ignore nonexistent operands"),
];

fn report(path: &str) {
    userlib::write(2, b"rm: ");
    userlib::write(2, path.as_bytes());
    userlib::write(2, b": cannot remove\n");
}

/// Refuses an operand outright, `-r` included -- not a prompting-style confirmation, a hard
/// guard nothing bypasses (matches GNU). `fs::cwd::absolute` normalizes `.`/`..` lexically, so
/// `rm -r .`/`rm -r ..`/`rm -r /` all resolve to a real, existing, catastrophic target even
/// though `.`/`..` are never real listable directory entries -- this checks the operand
/// *string* itself, before any `stat`/recursion happens. `true` if `path` was refused (already
/// reported).
fn refuse(path: &str, recursive: bool) -> bool {
    if matches!(progs::basename(path), "." | "..") {
        userlib::write(
            2,
            b"rm: refusing to remove '.' or '..' directory: skipping '",
        );
        userlib::write(2, path.as_bytes());
        userlib::write(2, b"'\n");
        true
    } else if path == "/" {
        if recursive {
            userlib::write(2, b"rm: it is dangerous to operate recursively on '/'\n");
        } else {
            userlib::write(2, b"rm: cannot remove '/': Is a directory\n");
        }
        true
    } else {
        false
    }
}

fn run(args: Args) -> ExitCode {
    let mut recursive = false;
    let mut force = false;
    let mut any = false;
    let mut opts = progs::cli::opts(args);
    loop {
        let arg = match progs::cli::next("rm", &mut opts) {
            Ok(Some(arg)) => arg,
            Ok(None) => break,
            Err(status) => return status,
        };
        match arg {
            Arg::Long("help") => return progs::help(USAGE, FLAGS),
            Arg::Short('r') | Arg::Short('R') | Arg::Long("recursive") => recursive = true,
            Arg::Short('f') | Arg::Long("force") => force = true,
            Arg::Positional(_) => any = true,
            other => return progs::cli::invalid("rm", other),
        }
    }
    if !any && force {
        return ExitCode(0);
    }
    if !any {
        userlib::write(2, b"rm: missing operand\n");
        return ExitCode(1);
    }

    let mut ok = true;
    for path in progs::cli::operands(args) {
        if refuse(path, recursive) {
            ok = false;
            continue;
        }
        let result = match userlib::stat(path) {
            // `-f`: an operand that cannot be examined is treated as nonexistent (this project's
            // `stat` has no separate "no such file" error to tell the cases apart).
            Err(_) if force => true,
            Err(_) => {
                report(path);
                false
            }
            Ok(info) if info.is_dir => {
                if recursive {
                    progs::copy::remove_tree("rm", path).is_ok()
                } else {
                    userlib::write(2, b"rm: cannot remove '");
                    userlib::write(2, path.as_bytes());
                    userlib::write(2, b"': Is a directory\n");
                    false
                }
            }
            Ok(_) => {
                let removed = userlib::unlink(path) >= 0;
                if !removed {
                    report(path);
                }
                removed
            }
        };
        if !result {
            ok = false;
        }
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
