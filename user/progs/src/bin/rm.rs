//! `rm [-r] PATH...` -- removes each named file, or directory. Without `-r`/`-R`/`--recursive`, a
//! non-empty directory is a reported error, not silently skipped or recursed into; with it, the
//! directory and everything under it is removed. No `-f` (ignore missing operands) or
//! `-i`/prompting yet -- later additions if ever needed.
#![no_std]
#![no_main]

extern crate alloc;

use getargs::Arg;
use userlib::{ATTR_DIRECTORY, Args, DIRENT_SIZE, DirEnt, ExitCode, O_RDONLY};

userlib::entry_with_args!(run);

const USAGE: &str = "rm [-r] PATH...";
const FLAGS: &[(&str, &str)] = &[(
    "-r, -R, --recursive",
    "remove directories and their contents",
)];

/// Same buffer size as `ls.rs`'s/`chmod.rs`'s own batched-`getdents` idiom -- one memory page.
const GETDENTS_BUF_SIZE: usize = 4096;

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

/// Removes the directory at `path` and everything under it: batches `path`'s own listing via
/// `getdents` (matching `chmod_recursive`'s stack-buffer idiom, not the mature reference's own
/// `Vec`+sort -- no `-v` here to make ordering visible, and `ls.rs` is already unsorted by
/// design), recursing into a child first when it's a directory, then removing it -- and finally
/// `path` itself, now empty. `false` (after reporting each failure) on any failure, having still
/// attempted every entry.
fn remove_recursive(path: &str) -> bool {
    let fd = userlib::open(path, O_RDONLY);
    if fd < 0 {
        report(path);
        return false;
    }
    let fd = fd as usize;
    let mut buf = [0u8; GETDENTS_BUF_SIZE];
    let mut ok = true;
    loop {
        let n = userlib::getdents(fd, &mut buf);
        if n < 0 {
            report(path);
            ok = false;
            break;
        }
        if n == 0 {
            break;
        }
        for raw in buf[..n as usize].chunks_exact(DIRENT_SIZE) {
            let Some(ent) = DirEnt::parse(raw) else {
                continue;
            };
            let child = progs::join(path, ent.name);
            let removed = if ent.attrs & ATTR_DIRECTORY != 0 {
                remove_recursive(&child)
            } else {
                userlib::unlink(&child) >= 0
            };
            if !removed {
                report(&child);
                ok = false;
            }
        }
    }
    userlib::close(fd);
    if ok && userlib::unlink(path) < 0 {
        report(path);
        ok = false;
    }
    ok
}

fn run(args: Args) -> ExitCode {
    let mut recursive = false;
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
            Arg::Positional(_) => any = true,
            other => return progs::cli::invalid("rm", other),
        }
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
            Err(_) => {
                report(path);
                false
            }
            Ok(info) if info.is_dir => {
                if recursive {
                    remove_recursive(path)
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
