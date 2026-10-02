//! `mv src... dst`: renames or moves each `src` to `dst`, or (when `dst` is an existing
//! directory) into `dst` under its own basename. Replaces an existing plain-file destination,
//! matching real `mv`(1)/`rename(2)` -- the kernel's `rename` does the replacing, after checking
//! everything else, so a move that fails never costs the destination it would have replaced --
//! and refuses whenever either side of a replacement would be a directory. Telling "replace the
//! file" and "move into the directory" apart, and multiple sources, is this program's own layer,
//! on top of `stat`.
//!
//! **Between two volumes** the kernel's `rename` says `EXDEV`, and `mv` copies and removes
//! instead -- a file or a whole directory tree, in POSIX's order (not GNU's remove-as-you-go), so
//! nothing of the source is touched until all of it has been copied and every file's size
//! checked:
//!
//! 1. the copy is made under a hidden name in the destination's own directory, `.mv-partial` (or
//!    `.mv-partial.1`, `.2`, ... for the first that is free), keeping modify times and the
//!    executable and read-only bits (`progs::copy`);
//! 2. it is renamed into place -- on one volume now, so the kernel's `rename` replaces an
//!    existing file and the old one is never truncated first;
//! 3. only then is the source removed.
//!
//! A failure in step 1 removes the hidden copy and leaves the source whole; a failure in step 3
//! leaves the destination complete and says so. A crash leaves a dot-file to `rm -r`.
#![no_std]
#![no_main]

extern crate alloc;

use abi::errno::EXDEV;
use alloc::format;
use alloc::string::String;
use progs::cli;
use progs::copy::{Copier, partial_path, remove_tree};
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "mv src... dst";
const FLAGS: &[(&str, &str)] = &[];

/// The move between two volumes, described above. Reports its own failures; `false` if it failed.
fn move_across(src: &str, target: &str) -> bool {
    let info = match userlib::stat(src) {
        Ok(info) => info,
        Err(e) => {
            cli::report_errno("mv", &format!("cannot stat '{src}'"), e);
            return false;
        }
    };
    let Some(partial) = partial_path(target, ".mv-partial") else {
        cli::report(
            "mv",
            &format!("cannot move '{src}' to '{target}': no free temporary name"),
        );
        return false;
    };
    let copier = Copier {
        prog: "mv",
        preserve: true,
    };
    let copied = if info.is_dir {
        copier.tree(src, &partial)
    } else {
        copier.file(src, &partial)
    };
    if copied.is_err() {
        // The source is untouched; take the half-made copy away again.
        let _ = remove_tree("mv", &partial);
        return false;
    }
    let renamed = userlib::rename(&partial, target);
    if renamed < 0 {
        let _ = remove_tree("mv", &partial);
        cli::report_errno("mv", &format!("cannot move '{src}' to '{target}'"), renamed);
        return false;
    }
    // The destination is complete: what is left is taking the source away.
    if remove_tree("mv", src).is_err() {
        cli::report(
            "mv",
            &format!("'{target}' is complete, but '{src}' could not be removed entirely"),
        );
        return false;
    }
    true
}

/// Moves `src` to `dst`, or -- if `dst_is_dir` -- into `dst` under `src`'s own basename. `false`
/// on any failure (already reported to stderr).
fn move_one(src: &str, dst: &str, dst_is_dir: bool) -> bool {
    let target = if dst_is_dir {
        format!("{}/{}", dst.trim_end_matches('/'), progs::basename(src))
    } else {
        String::from(dst)
    };
    if src == target {
        cli::report("mv", &format!("'{src}' and '{target}' are the same file"));
        return false;
    }
    // A directory in the way is refused here; a plain file is the kernel's to replace.
    if matches!(userlib::stat(&target), Ok(info) if info.is_dir) {
        cli::report(
            "mv",
            &format!("cannot overwrite directory '{target}' with non-directory"),
        );
        return false;
    }

    match userlib::rename(src, &target) {
        r if r >= 0 => true,
        EXDEV => move_across(src, &target),
        e => {
            cli::report_errno("mv", &format!("cannot move '{src}' to '{target}'"), e);
            false
        }
    }
}

fn run(args: Args) -> ExitCode {
    let plain = match cli::plain("mv", USAGE, FLAGS, args) {
        Ok(plain) => plain,
        Err(status) => return status,
    };
    let count = plain.count;
    let Some(dst) = plain.last.filter(|_| count >= 2) else {
        if let Some(src) = plain.first {
            cli::report(
                "mv",
                &format!("missing destination file operand after '{src}'"),
            );
        } else {
            cli::report("mv", "missing operand");
        }
        cli::try_help("mv");
        return ExitCode(1);
    };

    let dst_is_dir = matches!(userlib::stat(dst), Ok(info) if info.is_dir);
    let n_src = count - 1;
    if n_src > 1 && !dst_is_dir {
        cli::report("mv", &format!("target '{dst}' is not a directory"));
        return ExitCode(1);
    }

    let mut ok = true;
    for (i, src) in cli::operands(args).enumerate() {
        if i == count - 1 {
            break; // this operand is dst itself
        }
        if !move_one(src, dst, dst_is_dir) {
            ok = false;
        }
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
