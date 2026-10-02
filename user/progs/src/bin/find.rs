//! `find [PATH...] [-name GLOB] [-type f|d] [-mindepth N] [-maxdepth N]`: prints each path at or
//! under each PATH (default `.`) that passes every test given, one per line. Paths are printed as
//! GNU prints them: the start path as written, then `dir/name` for what is under it. Unlike GNU's,
//! the entries of a directory are visited in name order.
//!
//! The tests: `-name GLOB` (the last component of the path matches the pattern: `*`, `?`,
//! `[a-z]`), `-type f` or `-type d` (a file or a directory), `-mindepth N` and `-maxdepth N` (how
//! far below the start path: 0 is the start path itself). There is no `-exec`: a program cannot
//! start another.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use progs::{Out, ReadDirError, basename, cli, join, read_dir};
use textlib::glob::matches;
use userlib::{ATTR_DIRECTORY, Args, ExitCode, stat};

userlib::entry_with_args!(run);

const USAGE: &str = "find [PATH...] [-name GLOB] [-type f|d] [-mindepth N] [-maxdepth N]";
const FLAGS: &[(&str, &str)] = &[
    ("-name GLOB", "the last component matches GLOB (* ? [a-z])"),
    ("-type f|d", "a file, or a directory"),
    ("-mindepth N", "at least N levels below the start path"),
    ("-maxdepth N", "at most N levels below the start path"),
];

struct Tests {
    name: Option<&'static str>,
    dirs: Option<bool>,
    min_depth: usize,
    max_depth: usize,
}

fn show(out: &mut Out, path: &str, is_dir: bool, depth: usize, t: &Tests) {
    if depth < t.min_depth {
        return;
    }
    if t.dirs.is_some_and(|want_dir| want_dir != is_dir) {
        return;
    }
    if t.name
        .is_some_and(|pattern| !matches(pattern, basename(path)))
    {
        return;
    }
    out.put(path.as_bytes());
    out.put(b"\n");
}

/// Visits `path` (already known to be a directory or not) and, if it is one and there is depth
/// left, everything under it. `false` if anything under it could not be read (reported).
fn walk(out: &mut Out, path: &str, is_dir: bool, depth: usize, t: &Tests) -> bool {
    show(out, path, is_dir, depth, t);
    if !is_dir || depth >= t.max_depth {
        return true;
    }
    let mut entries = match read_dir(path) {
        Ok(entries) => entries,
        Err(ReadDirError::Open | ReadDirError::Read) => {
            cli::report("find", &format!("{path}: cannot read directory"));
            return false;
        }
    };
    entries.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    let mut ok = true;
    for ent in entries {
        let child = join(path, &ent.name);
        ok &= walk(out, &child, ent.attrs & ATTR_DIRECTORY != 0, depth + 1, t);
    }
    ok
}

fn number(what: &str, text: &str) -> Result<usize, ExitCode> {
    text.parse().map_err(|_| {
        cli::report("find", &format!("invalid argument '{text}' to '{what}'"));
        ExitCode(1)
    })
}

fn run(args: Args) -> ExitCode {
    let mut args = args.skip(1).peekable();
    if args.peek() == Some(&"--help") {
        return progs::help(USAGE, FLAGS);
    }
    // Start paths first, then the tests: the first argument that begins with a dash ends the
    // paths.
    let mut paths: Vec<&'static str> = Vec::new();
    while let Some(&arg) = args.peek() {
        if arg.starts_with('-') && arg.len() > 1 {
            break;
        }
        paths.push(arg);
        args.next();
    }
    if paths.is_empty() {
        paths.push(".");
    }

    let mut t = Tests {
        name: None,
        dirs: None,
        min_depth: 0,
        max_depth: usize::MAX,
    };
    while let Some(predicate) = args.next() {
        let mut value = |what: &str| -> Result<&'static str, ExitCode> {
            args.next().ok_or_else(|| {
                cli::report("find", &format!("missing argument to '{what}'"));
                ExitCode(1)
            })
        };
        let result = match predicate {
            "-name" => value("-name").map(|p| t.name = Some(p)),
            "-type" => value("-type").and_then(|kind| match kind {
                "f" | "d" => {
                    t.dirs = Some(kind == "d");
                    Ok(())
                }
                other => {
                    cli::report("find", &format!("Unknown argument to -type: {other}"));
                    Err(ExitCode(1))
                }
            }),
            "-mindepth" => value("-mindepth")
                .and_then(|n| number("-mindepth", n))
                .map(|n| t.min_depth = n),
            "-maxdepth" => value("-maxdepth")
                .and_then(|n| number("-maxdepth", n))
                .map(|n| t.max_depth = n),
            "-print" => Ok(()),
            other => {
                cli::report("find", &format!("unknown predicate '{other}'"));
                Err(ExitCode(1))
            }
        };
        if let Err(status) = result {
            return status;
        }
    }

    let mut out = Out::new();
    let mut status = 0;
    for path in paths {
        let start = String::from(path);
        match stat(&start) {
            Ok(info) => {
                if !walk(&mut out, &start, info.is_dir, 0, &t) {
                    status = 1;
                }
            }
            Err(_) => {
                cli::report("find", &format!("'{path}': cannot open"));
                status = 1;
            }
        }
    }
    ExitCode(status)
}
