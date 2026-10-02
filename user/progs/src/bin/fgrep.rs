//! `fgrep [-i] [-v] [-n] [-c] [-l] [-q] [-e PATTERN]... [PATTERN] [file...]`: prints the lines
//! that contain any of the PATTERNs as **fixed strings** (no regular expressions: `.` and `*` are
//! ordinary characters). Standard input for no file, or `-`. With more than one file each line is
//! prefixed with its file's name. Status 0 if any line was selected, 1 if none, 2 if a file could
//! not be read. This is GNU's `fgrep`, i.e. `grep -F`.
#![no_std]
#![no_main]

extern crate alloc;

use core::fmt::Write;

use alloc::vec::Vec;
use getargs::Arg;
use progs::{Out, cli};
use textlib::textutil::{contains, split_lines};
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "fgrep [-i] [-v] [-n] [-c] [-l] [-q] [-e PATTERN]... [PATTERN] [file...]";
const FLAGS: &[(&str, &str)] = &[
    (
        "-e PATTERN",
        "a pattern to look for (may be repeated; without -e the first operand is the pattern)",
    ),
    ("-i", "ignore case (ASCII letters)"),
    ("-v", "select the lines that do not match"),
    ("-n", "prefix each line with its line number"),
    ("-c", "print only a count of the selected lines per file"),
    ("-l", "print only the names of files with a selected line"),
    (
        "-q",
        "print nothing; the status says whether any line was selected",
    ),
];

fn run(args: Args) -> ExitCode {
    cli::status(fgrep(args))
}

fn fgrep(args: Args) -> Result<ExitCode, ExitCode> {
    let (mut ignore_case, mut invert, mut numbers, mut count, mut list, mut quiet) =
        (false, false, false, false, false, false);
    let mut patterns: Vec<&'static str> = Vec::new();
    let mut operands: Vec<&'static str> = Vec::new();
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("fgrep", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(progs::help(USAGE, FLAGS)),
            Arg::Short('e') | Arg::Long("regexp") => {
                patterns.push(cli::value("fgrep", &mut opts)?);
            }
            Arg::Short('i') | Arg::Long("ignore-case") => ignore_case = true,
            Arg::Short('v') | Arg::Long("invert-match") => invert = true,
            Arg::Short('n') | Arg::Long("line-number") => numbers = true,
            Arg::Short('c') | Arg::Long("count") => count = true,
            Arg::Short('l') | Arg::Long("files-with-matches") => list = true,
            Arg::Short('q') | Arg::Long("quiet" | "silent") => quiet = true,
            Arg::Positional(operand) => operands.push(operand),
            other => return Err(cli::invalid("fgrep", other)),
        }
    }
    // Without -e the first operand is the pattern.
    if patterns.is_empty() {
        if operands.is_empty() {
            cli::missing_operand("fgrep");
            return Err(ExitCode(2));
        }
        patterns.push(operands.remove(0));
    }
    if operands.is_empty() {
        operands.push("-");
    }
    let show_names = operands.len() > 1;

    let mut out = Out::new();
    let (mut any_selected, mut any_error) = (false, false);
    for name in operands {
        let label = if name == "-" {
            "(standard input)"
        } else {
            name
        };
        let data = match progs::read_operand(name) {
            Ok(data) => data,
            Err(e) => {
                cli::report_errno("fgrep", label, e);
                any_error = true;
                continue;
            }
        };

        let mut selected = 0usize;
        for (i, line) in split_lines(&data).into_iter().enumerate() {
            let matched = patterns
                .iter()
                .any(|p| contains(line, p.as_bytes(), ignore_case));
            if matched == invert {
                continue;
            }
            selected += 1;
            any_selected = true;
            if quiet {
                return Ok(ExitCode(0)); // the answer is known
            }
            if count || list {
                if list {
                    break; // one is enough to name the file
                }
                continue;
            }
            if show_names {
                let _ = write!(out, "{label}:");
            }
            if numbers {
                let _ = write!(out, "{}:", i + 1);
            }
            out.put(line);
            out.put(b"\n");
        }
        if list {
            if selected > 0 {
                let _ = writeln!(out, "{label}");
            }
        } else if count && !quiet {
            if show_names {
                let _ = write!(out, "{label}:");
            }
            let _ = writeln!(out, "{selected}");
        }
    }
    Ok(ExitCode(if any_error {
        2
    } else if any_selected {
        0
    } else {
        1
    }))
}
