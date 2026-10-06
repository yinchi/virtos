//! `mv [-f | -i | -n] [-u] [-b] [-v] [-T | -t DIR] src... dst`: renames or moves each `src` to `dst`, or (when `dst` is an existing
//! directory) into `dst` under its own basename. Replaces an existing plain-file destination,
//! matching real `mv`(1)/`rename(2)` -- the kernel's `rename` does the replacing, after checking
//! everything else, so a move that fails never costs the destination it would have replaced --
//! and refuses whenever either side of a replacement would be a directory. Telling "replace the
//! file" and "move into the directory" apart, and multiple sources, is this program's own layer,
//! on top of `stat`. The flags decide what happens when a *file* is already there: `-i` asks
//! (stdin), `-n` refuses (`not replacing`, status 1), `-u` skips one that is not older than the
//! source, `-b` renames it to `NAME~` first; `-t DIR` / `-T` name the destination directory
//! explicitly or rule one out; `-v` says each move. A directory in the way is refused whatever the
//! flags.
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

use abi::errno::{EINVAL, ENOTDIR, EXDEV};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use getargs::Arg;
use progs::cli;
use progs::copy::{Copier, partial_path, remove_tree};
use textlib::stamp::fat_to_unix;
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "mv [-f | -i | -n] [-u] [-b] [-v] [-T | -t DIR] src... dst";
const FLAGS: &[(&str, &str)] = &[
    (
        "-f",
        "replace without asking (the default; the last of -f, -i and -n wins)",
    ),
    ("-i", "ask before replacing an existing file (y to go on)"),
    ("-n", "do not replace an existing file (an error, as GNU's)"),
    ("-u", "replace only a file older than the source"),
    ("-b", "keep a replaced file as NAME~"),
    ("-t DIR", "move every src into the directory DIR"),
    (
        "-T",
        "treat dst as a file, never as a directory to move into",
    ),
    ("-v", "say each move"),
];

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
        no_clobber: false,
        verbose: false,
    };
    let copied = if info.is_dir {
        copier.tree(src, &partial)
    } else {
        copier.file(src, &partial)
    };
    if copied.is_err() {
        // The source is untouched; take the half-made copy away again.
        let _ = remove_tree("mv", &partial, false);
        return false;
    }
    let renamed = userlib::rename(&partial, target);
    if renamed < 0 {
        let _ = remove_tree("mv", &partial, false);
        cli::report_errno("mv", &format!("cannot move '{src}' to '{target}'"), renamed);
        return false;
    }
    // The destination is complete: what is left is taking the source away.
    if remove_tree("mv", src, false).is_err() {
        cli::report(
            "mv",
            &format!("'{target}' is complete, but '{src}' could not be removed entirely"),
        );
        return false;
    }
    true
}

/// What to do when the target is a file that exists already.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Overwrite {
    Force,
    Interactive,
    NoClobber,
}

struct Options {
    overwrite: Overwrite,
    update: bool,
    backup: bool,
    verbose: bool,
}

/// Asks whether to replace `target`, on stderr, and reads the answer from stdin: yes if it starts
/// with `y` or `Y`.
fn confirm(target: &str) -> bool {
    userlib::write(2, format!("mv: overwrite '{target}'? ").as_bytes());
    let mut line = [0u8; 64];
    let n = userlib::read(0, &mut line);
    n > 0
        && line[..n as usize]
            .iter()
            .find(|b| !b.is_ascii_whitespace())
            .is_some_and(|b| b.eq_ignore_ascii_case(&b'y'))
}

/// Whether `src` was modified after `target`. FAT counts in 2-second steps: the same stamp is not
/// newer. Where either time cannot be read, the move goes ahead.
fn newer(src: &str, target: &str) -> bool {
    let (Ok(s), Ok(t)) = (userlib::stat(src), userlib::stat(target)) else {
        return true;
    };
    match (
        fat_to_unix(s.modified_date, s.modified_time),
        fat_to_unix(t.modified_date, t.modified_time),
    ) {
        (Some(s), Some(t)) => s > t,
        _ => true,
    }
}

/// Moves `src` to `dst`, or -- if `dst_is_dir` -- into `dst` under `src`'s own basename. `false`
/// on any failure (already reported to stderr).
fn move_one(src: &str, dst: &str, dst_is_dir: bool, o: &Options) -> bool {
    let target = if dst_is_dir {
        format!("{}/{}", dst.trim_end_matches('/'), progs::basename(src))
    } else {
        String::from(dst)
    };
    if src == target {
        cli::report("mv", &format!("'{src}' and '{target}' are the same file"));
        return false;
    }
    // A directory in the way is refused here; a plain file is the kernel's to replace -- once the
    // flags have had their say about it.
    let mut backup: Option<String> = None;
    match userlib::stat(&target) {
        Ok(info) if info.is_dir => {
            cli::report(
                "mv",
                &format!("cannot overwrite directory '{target}' with non-directory"),
            );
            return false;
        }
        Ok(_) => {
            if o.update && !newer(src, &target) {
                return true; // nothing to do is not a failure, and not worth a word
            }
            match o.overwrite {
                // As GNU's `mv -n`: say so, and count it as a failure.
                Overwrite::NoClobber => {
                    cli::report("mv", &format!("not replacing '{target}'"));
                    return false;
                }
                Overwrite::Interactive if !confirm(&target) => return false,
                _ => {}
            }
            if o.backup {
                let kept = format!("{target}~");
                let r = userlib::rename(&target, &kept);
                if r < 0 {
                    cli::report_errno("mv", &format!("cannot backup '{target}'"), r);
                    return false;
                }
                backup = Some(kept);
            }
        }
        Err(_) => {}
    }

    match userlib::rename(src, &target) {
        r if r >= 0 => {}
        EXDEV => {
            if !move_across(src, &target) {
                return false;
            }
        }
        // The kernel's answer to a directory moved into itself is `EINVAL`, which it also gives
        // for a name FAT cannot hold: only say "subdirectory" when the paths look like it.
        EINVAL
            if target
                .strip_prefix(src)
                .is_some_and(|rest| rest.starts_with('/')) =>
        {
            cli::report(
                "mv",
                &format!("cannot move '{src}' to a subdirectory of itself, '{target}'"),
            );
            return false;
        }
        e => {
            cli::report_errno("mv", &format!("cannot move '{src}' to '{target}'"), e);
            return false;
        }
    }
    if o.verbose {
        cli::say(&match backup {
            Some(kept) => format!("renamed '{src}' -> '{target}' (backup: '{kept}')"),
            None => format!("renamed '{src}' -> '{target}'"),
        });
    }
    true
}

fn run(args: Args) -> ExitCode {
    cli::status(mv(args))
}

fn mv(args: Args) -> Result<ExitCode, ExitCode> {
    let mut o = Options {
        overwrite: Overwrite::Force,
        update: false,
        backup: false,
        verbose: false,
    };
    let mut target_dir: Option<&str> = None;
    let mut no_target = false;
    let mut operands: Vec<&str> = Vec::new();
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("mv", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(progs::help(USAGE, FLAGS)),
            Arg::Short('f') | Arg::Long("force") => o.overwrite = Overwrite::Force,
            Arg::Short('i') | Arg::Long("interactive") => o.overwrite = Overwrite::Interactive,
            Arg::Short('n') | Arg::Long("no-clobber") => o.overwrite = Overwrite::NoClobber,
            Arg::Short('u') | Arg::Long("update") => o.update = true,
            Arg::Short('b') | Arg::Long("backup") => o.backup = true,
            Arg::Short('v') | Arg::Long("verbose") => o.verbose = true,
            Arg::Short('T') | Arg::Long("no-target-directory") => no_target = true,
            Arg::Short('t') | Arg::Long("target-directory") => {
                target_dir = Some(cli::value("mv", &mut opts)?);
            }
            Arg::Positional(operand) => operands.push(operand),
            other => return Err(cli::invalid("mv", other)),
        }
    }
    if target_dir.is_some() && no_target {
        cli::report(
            "mv",
            "cannot combine --target-directory (-t) and --no-target-directory (-T)",
        );
        return Err(ExitCode(1));
    }

    // Where they go, and which operands are the things to move.
    let (dst, sources): (&str, &[&str]) = match target_dir {
        Some(dir) => {
            if operands.is_empty() {
                cli::report("mv", "missing file operand");
                cli::try_help("mv");
                return Err(ExitCode(1));
            }
            match userlib::stat(dir) {
                Ok(info) if info.is_dir => {}
                Ok(_) => {
                    cli::report_errno("mv", &format!("target directory '{dir}'"), ENOTDIR);
                    return Err(ExitCode(1));
                }
                Err(e) => {
                    cli::report_errno("mv", &format!("target directory '{dir}'"), e);
                    return Err(ExitCode(1));
                }
            }
            (dir, &operands[..])
        }
        None => {
            let Some((&dst, sources)) = operands
                .split_last()
                .filter(|(_, sources)| !sources.is_empty())
            else {
                match operands.first() {
                    Some(src) => cli::report(
                        "mv",
                        &format!("missing destination file operand after '{src}'"),
                    ),
                    None => cli::report("mv", "missing file operand"),
                }
                cli::try_help("mv");
                return Err(ExitCode(1));
            };
            if no_target && sources.len() > 1 {
                return Err(cli::extra_operand("mv", sources[1]));
            }
            (dst, sources)
        }
    };

    let dst_is_dir = target_dir.is_some()
        || (!no_target && matches!(userlib::stat(dst), Ok(info) if info.is_dir));
    if sources.len() > 1 && !dst_is_dir {
        cli::report("mv", &format!("target '{dst}' is not a directory"));
        return Err(ExitCode(1));
    }
    if !no_target && dst.ends_with('/') && !dst_is_dir {
        cli::report_errno(
            "mv",
            &format!("cannot move '{}' to '{dst}'", sources[0]),
            ENOTDIR,
        );
        return Err(ExitCode(1));
    }

    let mut ok = true;
    for src in sources {
        if !move_one(src, dst, dst_is_dir, &o) {
            ok = false;
        }
    }
    Ok(if ok { ExitCode(0) } else { ExitCode(1) })
}
