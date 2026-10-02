//! `sort [-r] [-n] [-u] [file...]`: sorts the lines of all the files together (standard input for
//! none, or `-`) and prints them: bytewise by default (the C locale), by leading number with `-n`,
//! reversed with `-r`, and with `-u` each set of lines that compare equal printed once. Lines that
//! compare equal by key keep a bytewise order of the whole line (GNU's "last resort"), except under
//! `-u`, where equal keys are duplicates. All the input is held in the user heap.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use getargs::Arg;
use progs::{Out, cli};
use textlib::sortkey::compare;
use textlib::textutil::split_lines;
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "sort [-r] [-n] [-u] [file...]";
const FLAGS: &[(&str, &str)] = &[
    ("-n", "compare by the number at the start of the line"),
    ("-r", "reverse the order"),
    ("-u", "print only the first of lines that compare equal"),
];

fn run(args: Args) -> ExitCode {
    cli::status(sort(args))
}

fn sort(args: Args) -> Result<ExitCode, ExitCode> {
    let (mut reverse, mut numeric, mut unique) = (false, false, false);
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("sort", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(progs::help(USAGE, FLAGS)),
            Arg::Short('r') | Arg::Long("reverse") => reverse = true,
            Arg::Short('n') | Arg::Long("numeric-sort") => numeric = true,
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
    // sort is stable: the first of a set stays first).
    let last_resort = !unique;
    lines.sort_by(|a, b| {
        let order = compare(a, b, numeric, last_resort);
        if reverse { order.reverse() } else { order }
    });
    if unique {
        lines.dedup_by(|later, earlier| compare(earlier, later, numeric, false).is_eq());
    }
    let mut out = Out::new();
    for line in lines {
        out.put(line);
        out.put(b"\n");
    }
    Ok(ExitCode(0))
}
