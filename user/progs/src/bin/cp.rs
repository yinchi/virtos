//! `cp src... dst` -- copies each `src`'s content into `dst`, or (when there's more than one
//! `src`, or `dst` is an existing directory) into `dst` under each source's own basename.
//! Overwrites an existing plain-file destination (`open(O_WRONLY)`'s own truncate-or-create
//! behavior handles that case for free, no `stat` needed); `stat` is only needed to tell a
//! directory destination apart from a plain one. GNU's `-r` and preserving timestamps/attributes
//! are later additions if ever needed. The first real consumer of `open(O_WRONLY)`, proving that
//! round trip the same way `cat` proved the read side.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use userlib::{Args, ExitCode, O_RDONLY, O_WRONLY};

userlib::entry_with_args!(run);

const USAGE: &str = "cp src... dst";
const FLAGS: &[(&str, &str)] = &[];
const BUF_SIZE: usize = 512;

/// Copies `src` to `dst`, or -- if `dst_is_dir` -- into `dst` under `src`'s own basename. `false`
/// on any failure (already reported to stderr).
fn copy_one(src: &str, dst: &str, dst_is_dir: bool) -> bool {
    let target = if dst_is_dir {
        format!("{}/{}", dst.trim_end_matches('/'), progs::basename(src))
    } else {
        String::from(dst)
    };
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

    let output = userlib::open(&target, O_WRONLY);
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

fn run(args: Args) -> ExitCode {
    let plain = match progs::cli::plain("cp", USAGE, FLAGS, args) {
        Ok(plain) => plain,
        Err(status) => return status,
    };
    let count = plain.count;
    let Some(dst) = plain.last.filter(|_| count >= 2) else {
        if let Some(src) = plain.first {
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
        if !copy_one(src, dst, dst_is_dir) {
            ok = false;
        }
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
