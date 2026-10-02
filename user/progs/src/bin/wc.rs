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

/// Lines, words, characters, bytes and longest line of one input.
#[derive(Default)]
struct Counts {
    lines: usize,
    words: usize,
    chars: usize,
    bytes: usize,
    max_line: usize,
}

impl Counts {
    /// Lines are counted by `\n` bytes -- not `head`/`tail`'s notion of "lines", which also
    /// counts a final unterminated one; a real `wc` doesn't. Words are maximal runs of
    /// non-whitespace bytes. A character is a byte that is not a UTF-8 continuation byte
    /// (`10xxxxxx`).
    fn of(data: &[u8]) -> Self {
        let mut c = Counts {
            bytes: data.len(),
            ..Counts::default()
        };
        c.chars = data.iter().filter(|&&b| b & 0xC0 != 0x80).count();
        let mut in_word = false;
        let mut cur_line = 0;
        for &b in data {
            if b == b'\n' {
                c.lines += 1;
                c.max_line = c.max_line.max(cur_line);
                cur_line = 0;
            } else {
                cur_line += 1;
            }
            if b.is_ascii_whitespace() || b == 0x0b {
                in_word = false;
            } else if !in_word {
                in_word = true;
                c.words += 1;
            }
        }
        c.max_line = c.max_line.max(cur_line);
        c
    }

    fn add(&mut self, other: &Counts) {
        self.lines += other.lines;
        self.words += other.words;
        self.chars += other.chars;
        self.bytes += other.bytes;
        self.max_line = self.max_line.max(other.max_line);
    }
}

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
