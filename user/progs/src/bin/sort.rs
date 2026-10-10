//! `sort [-r] [-n] [-f] [-b] [-s] [-u] [file...]`: sorts the lines of all the files together (standard input for
//! none, or `-`) and prints them: bytewise by default (the C locale), by leading number with `-n`,
//! reversed with `-r`, and with `-u` each set of lines that compare equal printed once. `-f` folds
//! ASCII case and `-b` ignores each line's leading blanks when comparing. Lines that compare equal
//! by key keep a bytewise order of the whole line (GNU's "last resort"), except under `-u`, where
//! equal keys are duplicates, and `-s`, which leaves them in input order. There are no `-k`/`-t`:
//! the key is always the whole line. All the input is held in the user heap.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use getargs::Arg;
use progs::{Out, cli};
use textlib::sortkey::{Opts, compare};
use textlib::textutil::split_lines;
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "sort [-r] [-n] [-f] [-b] [-s] [-u] [file...]";
const FLAGS: &[(&str, &str)] = &[
    ("-n", "compare by the number at the start of the line"),
    ("-f", "fold lower case to upper case (ASCII) when comparing"),
    ("-b", "ignore leading blanks when comparing"),
    ("-s", "stable: do not fall back to comparing whole lines"),
    ("-r", "reverse the order"),
    ("-u", "print only the first of lines that compare equal"),
];

fn run(args: Args) -> ExitCode {
    cli::status(sort(args))
}

fn sort(args: Args) -> Result<ExitCode, ExitCode> {
    let (mut reverse, mut unique, mut stable) = (false, false, false);
    let mut key = Opts::default();
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("sort", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(progs::help(USAGE, FLAGS)),
            Arg::Short('r') | Arg::Long("reverse") => reverse = true,
            Arg::Short('n') | Arg::Long("numeric-sort") => key.numeric = true,
            Arg::Short('f') | Arg::Long("ignore-case") => key.fold = true,
            Arg::Short('b') | Arg::Long("ignore-leading-blanks") => key.skip_blanks = true,
            Arg::Short('s') | Arg::Long("stable") => stable = true,
            Arg::Short('u') | Arg::Long("unique") => unique = true,
            Arg::Positional(_) => {}
            other => return Err(cli::invalid("sort", other)),
        }
    }

    let mut names: Vec<&str> = cli::operands(args).collect();
    if names.is_empty() {
        names.push("-");
    }
    let mut buffers: Vec<Vec<u8>> = Vec::new();
    for name in names {
        match progs::read_operand(name) {
            Ok(data) => buffers.push(data),
            Err(e) => {
                cli::report_errno("sort", &alloc::format!("cannot read: {name}"), e);
                return Ok(ExitCode(2));
            }
        }
    }

    let mut lines: Vec<&[u8]> = buffers.iter().flat_map(|b| split_lines(b)).collect();
    // With `-u` equal keys are duplicates, so there is no last resort to order them by (and the
    // sort is stable: the first of a set stays first); `-s` asks for the same stability.
    let last_resort = !(unique || stable);
    lines.sort_by(|a, b| {
        let order = compare(a, b, key, last_resort);
        if reverse { order.reverse() } else { order }
    });
    if unique {
        lines.dedup_by(|later, earlier| compare(earlier, later, key, false).is_eq());
    }
    let mut out = Out::new();
    for line in lines {
        out.put(line);
        out.put(b"\n");
    }
    Ok(ExitCode(0))
}
