//! Prints the line, word, and byte counts of each named file, plus a total if more than one is
//! given -- an ordinary `wc`'s default output, no `-l`/`-w`/`-c` flag to print just one of them yet.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "wc [file...]";
const FLAGS: &[(&str, &str)] = &[];

/// `(lines, words, bytes)`. Lines are counted by `\n` bytes -- not `head`/`tail`'s notion of
/// "lines", which also counts a final unterminated one; a real `wc` doesn't. Words are maximal
/// runs of non-whitespace bytes.
fn counts(data: &[u8]) -> (usize, usize, usize) {
    let mut lines = 0;
    let mut words = 0;
    let mut in_word = false;
    for &b in data {
        if b == b'\n' {
            lines += 1;
        }
        if b.is_ascii_whitespace() {
            in_word = false;
        } else if !in_word {
            in_word = true;
            words += 1;
        }
    }
    (lines, words, data.len())
}

fn run(args: Args) -> ExitCode {
    let mut paths: Vec<&str> = Vec::new();
    for arg in args.skip(1) {
        if arg == "--help" {
            return progs::help(USAGE, FLAGS);
        }
        paths.push(arg);
    }
    if paths.is_empty() {
        userlib::write(2, b"wc: missing file operand\n");
        return ExitCode(1);
    }

    let mut ok = true;
    let (mut total_lines, mut total_words, mut total_bytes) = (0, 0, 0);
    for &path in &paths {
        let Some(data) = progs::read_file(path) else {
            userlib::write(2, b"wc: ");
            userlib::write(2, path.as_bytes());
            userlib::write(2, b": cannot open\n");
            ok = false;
            continue;
        };
        let (lines, words, bytes) = counts(&data);
        total_lines += lines;
        total_words += words;
        total_bytes += bytes;
        userlib::write(1, format!("{lines} {words} {bytes} {path}\n").as_bytes());
    }
    if paths.len() > 1 {
        let line = format!("{total_lines} {total_words} {total_bytes} total\n");
        userlib::write(1, line.as_bytes());
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
