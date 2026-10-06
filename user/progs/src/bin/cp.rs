//! `cp [-p] [-r] src... dst`: copies each `src`'s content into `dst`, or (when there's more than
//! one `src`, or `dst` is an existing directory) into `dst` under each source's own basename.
//! Overwrites an existing plain-file destination (`open(O_WRONLY)`'s own truncate-or-create
//! behavior handles that case for free, no `stat` needed); `stat` is only needed to tell a
//! directory destination apart from a plain one. `-r`/`-R`/`--recursive` copies a directory
//! source: `dst` is created fresh, or merged into if it already names a directory. `-p` keeps each
//! copy's modify time and a file's executable and read-only bits. Every copy's size is checked
//! once it is closed. The copying itself is `progs::copy::Copier`, shared with `mv`'s copy across
//! volumes. `-n` leaves a destination file that exists alone, silently; `-v` says each copy.
//! Stdin/stdout (`-`) is a later addition if ever needed.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use getargs::Arg;
use progs::cli;
use progs::copy::Copier;
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "cp [-p] [-r] [-n] [-v] src... dst";
const FLAGS: &[(&str, &str)] = &[
    ("-r, -R, --recursive", "copy directories recursively"),
    (
        "-n, --no-clobber",
        "do not overwrite an existing file (silently)",
    ),
    ("-v, --verbose", "say what is copied"),
    (
        "-p, --preserve",
        "keep modify times and the executable and read-only bits",
    ),
];

/// `dst`, or -- if `dst_is_dir` -- `dst` joined with `src`'s own basename. Shared by the
/// top-level dispatch (`copy_operand`) and `copy_one`, so both agree on where one source lands.
fn target_for(src: &str, dst: &str, dst_is_dir: bool) -> String {
    if dst_is_dir {
        progs::join(dst, progs::basename(src))
    } else {
        String::from(dst)
    }
}

/// Copies one operand: `src` into `target` (`dst`, or `dst/basename(src)` if `dst_is_dir`) -- a
/// directory with `recursive`, a plain file otherwise. `false` (already reported) if `src` is a
/// directory and `recursive` is unset, or if copying it into itself was attempted.
fn copy_operand(copier: &Copier, src: &str, dst: &str, dst_is_dir: bool, recursive: bool) -> bool {
    let target = target_for(src, dst, dst_is_dir);
    match userlib::stat(src) {
        Ok(info) if info.is_dir => {
            if !recursive {
                cli::report(
                    "cp",
                    &format!("-r not specified; omitting directory '{src}'"),
                );
                return false;
            }
            let plain = src.trim_end_matches('/');
            if target == plain
                || target
                    .strip_prefix(plain)
                    .is_some_and(|rest| rest.starts_with('/'))
            {
                cli::report(
                    "cp",
                    &format!("cannot copy a directory, '{src}', into itself, '{target}'"),
                );
                return false;
            }
            copier.tree(plain, &target).is_ok()
        }
        _ => {
            if src == target {
                cli::report("cp", &format!("'{src}' and '{target}' are the same file"));
                return false;
            }
            copier.file(src, &target).is_ok()
        }
    }
}

fn run(args: Args) -> ExitCode {
    let mut recursive = false;
    let mut preserve = false;
    let mut no_clobber = false;
    let mut verbose = false;
    let mut count = 0usize;
    let mut first = None;
    let mut last = None;
    let mut opts = cli::opts(args);
    loop {
        let arg = match cli::next("cp", &mut opts) {
            Ok(Some(arg)) => arg,
            Ok(None) => break,
            Err(status) => return status,
        };
        match arg {
            Arg::Long("help") => return progs::help(USAGE, FLAGS),
            Arg::Short('r') | Arg::Short('R') | Arg::Long("recursive") => recursive = true,
            Arg::Short('p') | Arg::Long("preserve") => preserve = true,
            Arg::Short('n') | Arg::Long("no-clobber") => no_clobber = true,
            Arg::Short('v') | Arg::Long("verbose") => verbose = true,
            Arg::Positional(operand) => {
                count += 1;
                first.get_or_insert(operand);
                last = Some(operand);
            }
            other => return cli::invalid("cp", other),
        }
    }

    let Some(dst) = last.filter(|_| count >= 2) else {
        if let Some(src) = first {
            cli::report(
                "cp",
                &format!("missing destination file operand after '{src}'"),
            );
            cli::try_help("cp");
        } else {
            cli::report("cp", "missing operand");
            cli::try_help("cp");
        }
        return ExitCode(1);
    };

    let dst_is_dir = matches!(userlib::stat(dst), Ok(info) if info.is_dir);
    let n_src = count - 1;
    if n_src > 1 && !dst_is_dir {
        cli::report("cp", &format!("target '{dst}' is not a directory"));
        return ExitCode(1);
    }

    let copier = Copier {
        prog: "cp",
        preserve,
        no_clobber,
        verbose,
    };
    let mut ok = true;
    for (i, src) in cli::operands(args).enumerate() {
        if i == count - 1 {
            break; // this operand is dst itself
        }
        if !copy_operand(&copier, src, dst, dst_is_dir, recursive) {
            ok = false;
        }
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
