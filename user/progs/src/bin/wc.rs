//! Prints the line, word, and byte counts of each named file, plus a total if more than one is
//! given -- an ordinary `wc`'s default output, no `-l`/`-w`/`-c` flag to print just one of them
//! yet. No operand reads stdin, printing counts with no trailing name (GNU's own convention --
//! there's no path to show); an explicit `-` operand reads stdin too, but *does* print `-` as its
//! name, the same as any other operand.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
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
    let plain = match progs::cli::plain("wc", USAGE, FLAGS, args) {
        Ok(plain) => plain,
        Err(status) => return status,
    };
    let implicit_stdin = plain.count == 0;

    let mut ok = true;
    let (mut total_lines, mut total_words, mut total_bytes) = (0, 0, 0);
    let mut handle = |path: &str| {
        let Some(data) = progs::read_operand(path) else {
            userlib::write(2, b"wc: ");
            userlib::write(2, path.as_bytes());
            userlib::write(2, b": cannot open\n");
            ok = false;
            return;
        };
        let (lines, words, bytes) = counts(&data);
        total_lines += lines;
        total_words += words;
        total_bytes += bytes;
        let line = if implicit_stdin {
            format!("{lines} {words} {bytes}\n")
        } else {
            format!("{lines} {words} {bytes} {path}\n")
        };
        userlib::write(1, line.as_bytes());
    };

    if implicit_stdin {
        handle("-");
    }
    for path in progs::cli::operands(args) {
        handle(path);
    }

    if plain.count > 1 {
        let line = format!("{total_lines} {total_words} {total_bytes} total\n");
        userlib::write(1, line.as_bytes());
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
