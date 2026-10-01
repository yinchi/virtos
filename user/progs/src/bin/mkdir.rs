//! `mkdir [-p] DIR...` -- creates each named directory. `-p`/`--parents` creates missing parent
//! components too, treating an already-existing directory -- at any level, including the final
//! one -- as success rather than an error (GNU's own `-p` semantics); an existing *file* at any
//! component is still an error.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "mkdir [-p] DIR...";
const FLAGS: &[(&str, &str)] = &[("-p, --parents", "create missing parent directories too")];

fn report(path: &str) {
    userlib::write(2, b"mkdir: ");
    userlib::write(2, path.as_bytes());
    userlib::write(2, b": cannot create directory\n");
}

/// Creates `dir`. With `parents`, walks successively longer `/`-joined prefixes of `dir` instead
/// of `dir` itself directly -- `stat`ing each first so an already-existing directory at any level
/// is skipped rather than re-created (or reported as an error, the way a plain `mkdir` on an
/// existing name always is).
fn mkdir_one(dir: &str, parents: bool) -> bool {
    if !parents {
        if userlib::mkdir(dir) < 0 {
            report(dir);
            return false;
        }
        return true;
    }
    let mut prefix = String::new();
    if dir.starts_with('/') {
        prefix.push('/');
    }
    for component in dir.split('/').filter(|c| !c.is_empty()) {
        if !prefix.is_empty() && !prefix.ends_with('/') {
            prefix.push('/');
        }
        prefix.push_str(component);
        match userlib::stat(&prefix) {
            Ok(s) if s.is_dir => continue,
            Ok(_) => {
                report(&prefix);
                return false;
            }
            Err(_) => {
                if userlib::mkdir(&prefix) < 0 {
                    report(&prefix);
                    return false;
                }
            }
        }
    }
    true
}

fn run(args: Args) -> ExitCode {
    let mut parents = false;
    let mut dirs: Vec<&str> = Vec::new();
    for arg in args.skip(1) {
        match arg {
            "--help" => return progs::help(USAGE, FLAGS),
            "-p" | "--parents" => parents = true,
            dir => dirs.push(dir),
        }
    }
    if dirs.is_empty() {
        userlib::write(2, b"mkdir: missing operand\n");
        return ExitCode(1);
    }
    let mut ok = true;
    for dir in dirs {
        if !mkdir_one(dir, parents) {
            ok = false;
        }
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
