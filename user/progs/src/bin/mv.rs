//! `mv src... dst` -- renames or moves each `src` to `dst`, or (when `dst` is an existing
//! directory) into `dst` under its own basename. Replaces an existing plain-file destination,
//! matching real `mv`(1)/`rename(2)`; refuses whenever either side of a replacement would be a
//! directory. `hadris-fat`'s own `rename` refuses outright if `dst` already names anything at
//! all, so telling "replace the file" and "move into the directory" apart, and multiple sources,
//! is entirely this program's own layer, on top of `stat` -- deferred from `cp`/`mv`/`rm`/`mkdir`'s
//! first pass until `stat` existed.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "mv src... dst";
const FLAGS: &[(&str, &str)] = &[];

/// Moves `src` to `dst`, or -- if `dst_is_dir` -- into `dst` under `src`'s own basename. `false`
/// on any failure (already reported to stderr).
fn move_one(src: &str, dst: &str, dst_is_dir: bool) -> bool {
    let target = if dst_is_dir {
        format!("{}/{}", dst.trim_end_matches('/'), progs::basename(src))
    } else {
        String::from(dst)
    };
    if src == target {
        userlib::write(2, b"mv: '");
        userlib::write(2, src.as_bytes());
        userlib::write(2, b"' and '");
        userlib::write(2, target.as_bytes());
        userlib::write(2, b"' are the same file\n");
        return false;
    }

    match userlib::stat(&target) {
        // Doesn't exist (or some other `stat` failure) -- rename below reports anything that's
        // actually wrong.
        Err(_) => {}
        Ok(info) if info.is_dir => {
            userlib::write(2, b"mv: cannot overwrite directory '");
            userlib::write(2, target.as_bytes());
            userlib::write(2, b"' with non-directory\n");
            return false;
        }
        // An existing plain file: unlink it first, since `rename` itself refuses an existing name
        // outright, whatever type it is.
        Ok(_) => {
            userlib::unlink(&target);
        }
    }

    if userlib::rename(src, &target) < 0 {
        userlib::write(2, b"mv: cannot move '");
        userlib::write(2, src.as_bytes());
        userlib::write(2, b"' to '");
        userlib::write(2, target.as_bytes());
        userlib::write(2, b"'\n");
        return false;
    }
    true
}

fn run(args: Args) -> ExitCode {
    let plain = match progs::cli::plain("mv", USAGE, FLAGS, args) {
        Ok(plain) => plain,
        Err(status) => return status,
    };
    let count = plain.count;
    let Some(dst) = plain.last.filter(|_| count >= 2) else {
        if let Some(src) = plain.first {
            userlib::write(2, b"mv: missing destination file operand after '");
            userlib::write(2, src.as_bytes());
            userlib::write(2, b"'\n");
        } else {
            userlib::write(2, b"mv: missing operand\n");
        }
        return ExitCode(1);
    };

    let dst_is_dir = matches!(userlib::stat(dst), Ok(info) if info.is_dir);
    let n_src = count - 1;
    if n_src > 1 && !dst_is_dir {
        userlib::write(2, b"mv: target '");
        userlib::write(2, dst.as_bytes());
        userlib::write(2, b"' is not a directory\n");
        return ExitCode(1);
    }

    let mut ok = true;
    for (i, src) in progs::cli::operands(args).enumerate() {
        if i == count - 1 {
            break; // this operand is dst itself
        }
        if !move_one(src, dst, dst_is_dir) {
            ok = false;
        }
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
