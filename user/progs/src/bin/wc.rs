//! `wc [-l] [-w] [-m] [-c] [-L] [file...]`: prints the counts of each named file, plus a total if
//! more than one is given. With no flag, lines, words and bytes (POSIX's order); with any, only
//! the ones asked for, still in GNU's fixed order: lines, words, characters (`-m`: UTF-8
//! characters, so a multi-byte one is one), bytes, longest line (`-L`). No operand reads stdin,
//! printing counts with no trailing name (GNU's own convention -- there's no path to show); an
//! explicit `-` operand reads stdin too, but *does* print `-` as its name, the same as any other
//! operand.
#![no_std]
#![no_main]

extern crate alloc;

use core::fmt::Write;

use alloc::string::String;
use getargs::Arg;
use progs::cli;
use textlib::wccount::Counts;
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "wc [-l] [-w] [-m] [-c] [-L] [file...]";
const FLAGS: &[(&str, &str)] = &[
    ("-l", "count lines"),
    ("-w", "count words"),
    ("-m", "count characters"),
    ("-c", "count bytes"),
    ("-L", "report the longest line's length"),
];

/// One output line: the wanted counts in order, then the name if there is one.
fn format_counts(c: &Counts, wanted: [bool; 5], name: Option<&str>) -> String {
    let mut line = String::new();
    let mut first = true;
    for (wanted, count) in wanted
        .into_iter()
        .zip([c.lines, c.words, c.chars, c.bytes, c.max_line])
    {
        if wanted {
            let _ = write!(line, "{}{count}", if first { "" } else { " " });
            first = false;
        }
    }
    if let Some(name) = name {
        let _ = write!(line, " {name}");
    }
    line.push('\n');
    line
}

fn run(args: Args) -> ExitCode {
    cli::status(count(args))
}

fn count(args: Args) -> Result<ExitCode, ExitCode> {
    let (mut lines, mut words, mut chars, mut bytes, mut max_line) =
        (false, false, false, false, false);
    let mut n_files = 0usize;
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("wc", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(progs::help(USAGE, FLAGS)),
            Arg::Short('l') | Arg::Long("lines") => lines = true,
            Arg::Short('w') | Arg::Long("words") => words = true,
            Arg::Short('m') | Arg::Long("chars") => chars = true,
            Arg::Short('c') | Arg::Long("bytes") => bytes = true,
            Arg::Short('L') | Arg::Long("max-line-length") => max_line = true,
            Arg::Positional(_) => n_files += 1,
            other => return Err(cli::invalid("wc", other)),
        }
    }
    // Nothing requested: lines, words and bytes. `-L` alone is not "nothing": it opts out of the
    // default the same as any other flag.
    if !(lines || words || chars || bytes || max_line) {
        (lines, words, bytes) = (true, true, true);
    }
    let wanted = [lines, words, chars, bytes, max_line];

    let implicit_stdin = n_files == 0;
    let mut out = progs::Out::new();
    let mut status = 0;
    let mut total = Counts::default();
    let mut handle = |path: &str| {
        let Some(data) = progs::read_operand(path) else {
            cli::report("wc", &alloc::format!("{path}: cannot open"));
            status = 1;
            return;
        };
        let counts = Counts::of(&data);
        total.add(&counts);
        out.put(format_counts(&counts, wanted, (!implicit_stdin).then_some(path)).as_bytes());
    };
    if implicit_stdin {
        handle("-");
    }
    for path in cli::operands(args) {
        handle(path);
    }
    if n_files > 1 {
        out.put(format_counts(&total, wanted, Some("total")).as_bytes());
    }
    Ok(ExitCode(status))
}
