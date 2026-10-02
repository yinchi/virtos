//! `cp [-r] src... dst` -- copies each `src`'s content into `dst`, or (when there's more than one
//! `src`, or `dst` is an existing directory) into `dst` under each source's own basename.
//! Overwrites an existing plain-file destination (`open(O_WRONLY)`'s own truncate-or-create
//! behavior handles that case for free, no `stat` needed); `stat` is only needed to tell a
//! directory destination apart from a plain one. `-r`/`-R`/`--recursive` copies a directory
//! source: `dst` is created fresh, or merged into if it already names a directory. Preserving
//! timestamps/attributes and stdin/stdout (`-`) are later additions if ever needed. The first
//! real consumer of `open(O_WRONLY)`, proving that round trip the same way `cat` proved the read
//! side.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use getargs::Arg;
use userlib::{ATTR_DIRECTORY, Args, DIRENT_SIZE, DirEnt, ExitCode, O_RDONLY, O_WRONLY};

userlib::entry_with_args!(run);

const USAGE: &str = "cp [-r] src... dst";
const FLAGS: &[(&str, &str)] = &[("-r, -R, --recursive", "copy directories recursively")];
const BUF_SIZE: usize = 512;

/// Same buffer size as `ls.rs`'s/`chmod.rs`'s own batched-`getdents` idiom -- one memory page.
const GETDENTS_BUF_SIZE: usize = 4096;

/// `dst`, or -- if `dst_is_dir` -- `dst` joined with `src`'s own basename. Shared by the
/// top-level dispatch (`copy_operand`) and `copy_one`, so both agree on where one source lands.
fn target_for(src: &str, dst: &str, dst_is_dir: bool) -> String {
    if dst_is_dir {
        progs::join(dst, progs::basename(src))
    } else {
        String::from(dst)
    }
}

/// Copies the plain file `src` to the already-resolved path `target`. `false` on any failure
/// (already reported to stderr).
fn copy_one(src: &str, target: &str) -> bool {
    if src == target {
        userlib::write(2, b"cp: '");
        userlib::write(2, src.as_bytes());
        userlib::write(2, b"' and '");
        userlib::write(2, target.as_bytes());
        userlib::write(2, b"' are the same file\n");
        return false;
    }

    let input = userlib::open(src, O_RDONLY);
    if input < 0 {
        userlib::write(2, b"cp: ");
        userlib::write(2, src.as_bytes());
        userlib::write(2, b": cannot open\n");
        return false;
    }
    let input = input as usize;

    // Read the first chunk before opening (and so truncating) `target`: a source that can't be
    // read at all -- a directory, say -- must not destroy an existing destination first.
    let mut buf = [0u8; BUF_SIZE];
    let first = userlib::read(input, &mut buf);
    if first < 0 {
        userlib::write(2, b"cp: ");
        userlib::write(2, src.as_bytes());
        userlib::write(2, b": cannot read\n");
        userlib::close(input);
        return false;
    }

    let output = userlib::open(target, O_WRONLY);
    if output < 0 {
        userlib::write(2, b"cp: ");
        userlib::write(2, target.as_bytes());
        userlib::write(2, b": cannot open\n");
        userlib::close(input);
        return false;
    }
    let output = output as usize;

    let mut ok = true;
    let mut n = first;
    while n > 0 {
        if userlib::write(output, &buf[..n as usize]) < 0 {
            userlib::write(2, b"cp: ");
            userlib::write(2, target.as_bytes());
            userlib::write(2, b": write error\n");
            ok = false;
            break;
        }
        n = userlib::read(input, &mut buf);
        if n < 0 {
            userlib::write(2, b"cp: ");
            userlib::write(2, src.as_bytes());
            userlib::write(2, b": cannot read\n");
            ok = false;
            break;
        }
    }
    userlib::close(input);
    // Closing `output` is what commits its final size to disk -- a failure here is a failed copy.
    if userlib::close(output) < 0 {
        ok = false;
    }
    ok
}

/// Copies the directory `src` to `dst` and everything under it: `dst` is created if it doesn't
/// exist, or merged into if it's already a directory -- a hard error if it names anything else.
/// Then batches `src`'s own listing via `getdents` (matching `chmod_recursive`'s stack-buffer
/// idiom, not the mature reference's own `Vec`+sort -- no ordering dependency here), dispatching
/// each child to `copy_tree` (a subdirectory) or `copy_one` (a file) by its own `DirEnt.attrs`,
/// no extra `stat` per child. `false` (after reporting each failure) on any failure, having still
/// attempted every entry.
fn copy_tree(src: &str, dst: &str) -> bool {
    match userlib::stat(dst) {
        Ok(info) if info.is_dir => {}
        Ok(_) => {
            userlib::write(2, b"cp: cannot overwrite non-directory '");
            userlib::write(2, dst.as_bytes());
            userlib::write(2, b"' with directory '");
            userlib::write(2, src.as_bytes());
            userlib::write(2, b"'\n");
            return false;
        }
        Err(_) => {
            if userlib::mkdir(dst) < 0 {
                userlib::write(2, b"cp: ");
                userlib::write(2, dst.as_bytes());
                userlib::write(2, b": cannot create directory\n");
                return false;
            }
        }
    }

    let fd = userlib::open(src, O_RDONLY);
    if fd < 0 {
        userlib::write(2, b"cp: ");
        userlib::write(2, src.as_bytes());
        userlib::write(2, b": cannot open\n");
        return false;
    }
    let fd = fd as usize;
    let mut buf = [0u8; GETDENTS_BUF_SIZE];
    let mut ok = true;
    loop {
        let n = userlib::getdents(fd, &mut buf);
        if n < 0 {
            userlib::write(2, b"cp: ");
            userlib::write(2, src.as_bytes());
            userlib::write(2, b": not a directory\n");
            ok = false;
            break;
        }
        if n == 0 {
            break;
        }
        for raw in buf[..n as usize].as_chunks::<DIRENT_SIZE>().0 {
            let Some(ent) = DirEnt::parse(raw) else {
                continue;
            };
            let from = progs::join(src, ent.name);
            let to = progs::join(dst, ent.name);
            let copied = if ent.attrs & ATTR_DIRECTORY != 0 {
                copy_tree(&from, &to)
            } else {
                copy_one(&from, &to)
            };
            if !copied {
                ok = false;
            }
        }
    }
    userlib::close(fd);
    ok
}

/// Copies one operand: `src` into `target` (`dst`, or `dst/basename(src)` if `dst_is_dir`) -- a
/// directory with `recursive`, a plain file otherwise. `false` (already reported) if `src` is a
/// directory and `recursive` is unset, or if copying it into itself was attempted.
fn copy_operand(src: &str, dst: &str, dst_is_dir: bool, recursive: bool) -> bool {
    let target = target_for(src, dst, dst_is_dir);
    match userlib::stat(src) {
        Ok(info) if info.is_dir => {
            if !recursive {
                userlib::write(2, b"cp: -r not specified; omitting directory '");
                userlib::write(2, src.as_bytes());
                userlib::write(2, b"'\n");
                return false;
            }
            let plain = src.trim_end_matches('/');
            if target == plain
                || target
                    .strip_prefix(plain)
                    .is_some_and(|rest| rest.starts_with('/'))
            {
                userlib::write(2, b"cp: cannot copy a directory, '");
                userlib::write(2, src.as_bytes());
                userlib::write(2, b"', into itself, '");
                userlib::write(2, target.as_bytes());
                userlib::write(2, b"'\n");
                return false;
            }
            copy_tree(plain, &target)
        }
        _ => copy_one(src, &target),
    }
}

fn run(args: Args) -> ExitCode {
    let mut recursive = false;
    let mut count = 0usize;
    let mut first = None;
    let mut last = None;
    let mut opts = progs::cli::opts(args);
    loop {
        let arg = match progs::cli::next("cp", &mut opts) {
            Ok(Some(arg)) => arg,
            Ok(None) => break,
            Err(status) => return status,
        };
        match arg {
            Arg::Long("help") => return progs::help(USAGE, FLAGS),
            Arg::Short('r') | Arg::Short('R') | Arg::Long("recursive") => recursive = true,
            Arg::Positional(operand) => {
                count += 1;
                first.get_or_insert(operand);
                last = Some(operand);
            }
            other => return progs::cli::invalid("cp", other),
        }
    }

    let Some(dst) = last.filter(|_| count >= 2) else {
        if let Some(src) = first {
            userlib::write(2, b"cp: missing destination file operand after '");
            userlib::write(2, src.as_bytes());
            userlib::write(2, b"'\n");
        } else {
            userlib::write(2, b"cp: missing operand\n");
        }
        return ExitCode(1);
    };

    let dst_is_dir = matches!(userlib::stat(dst), Ok(info) if info.is_dir);
    let n_src = count - 1;
    if n_src > 1 && !dst_is_dir {
        userlib::write(2, b"cp: target '");
        userlib::write(2, dst.as_bytes());
        userlib::write(2, b"' is not a directory\n");
        return ExitCode(1);
    }

    let mut ok = true;
    for (i, src) in progs::cli::operands(args).enumerate() {
        if i == count - 1 {
            break; // this operand is dst itself
        }
        if !copy_operand(src, dst, dst_is_dir, recursive) {
            ok = false;
        }
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
