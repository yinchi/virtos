//! `mkdir [-p] DIR...` -- creates each named directory. `-p`/`--parents` creates missing parent
//! components too, treating an already-existing directory -- at any level, including the final
//! one -- as success rather than an error (GNU's own `-p` semantics); an existing *file* at any
//! component is still an error.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use getargs::Arg;
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "mkdir [-p] DIR...";
const FLAGS: &[(&str, &str)] = &[("-p, --parents", "create missing parent directories too")];

fn report(path: &str, errno: isize) {
    progs::cli::report_errno("mkdir", &format!("cannot create directory '{path}'"), errno);
}

/// Creates `dir`. With `parents`, walks successively longer `/`-joined prefixes of `dir` instead
/// of `dir` itself directly -- `stat`ing each first so an already-existing directory at any level
/// is skipped rather than re-created (or reported as an error, the way a plain `mkdir` on an
/// existing name always is).
fn mkdir_one(dir: &str, parents: bool) -> bool {
    if !parents {
        let r = userlib::mkdir(dir);
        if r < 0 {
            report(dir, r);
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
                report(&prefix, abi::errno::EEXIST);
                return false;
            }
            Err(_) => {
                let r = userlib::mkdir(&prefix);
                if r < 0 {
                    report(&prefix, r);
                    return false;
                }
            }
        }
    }
    true
}

fn run(args: Args) -> ExitCode {
    let mut parents = false;
    let mut any = false;
    let mut opts = progs::cli::opts(args);
    loop {
        let arg = match progs::cli::next("mkdir", &mut opts) {
            Ok(Some(arg)) => arg,
            Ok(None) => break,
            Err(status) => return status,
        };
        match arg {
            Arg::Long("help") => return progs::help(USAGE, FLAGS),
            Arg::Short('p') | Arg::Long("parents") => parents = true,
            Arg::Positional(_) => any = true,
            other => return progs::cli::invalid("mkdir", other),
        }
    }
    if !any {
        userlib::write(2, b"mkdir: missing operand\n");
        return ExitCode(1);
    }
    let mut ok = true;
    for dir in progs::cli::operands(args) {
        if !mkdir_one(dir, parents) {
            ok = false;
        }
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
