//! `echo [-neE] args...`: prints its arguments (not its own name -- `argv[0]` is skipped, as
//! usual), separated by a single space each, followed by a newline. `-n` suppresses the newline;
//! `-e` interprets backslash escapes in the arguments (`\\ \a \b \c \e \f \n \r \t \v \0NNN
//! \xHH`; `\c` stops all output, the newline included; an unknown escape is left as written); `-E`
//! turns that off (the default). Options are the leading arguments made of a dash and only `n`,
//! `e`, `E` (`-ne`, `-en`); the first argument that is not one, and everything after it, is text
//! -- and `--help` is only recognized as the very first argument, matching GNU's own `echo`.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "echo [-neE] [args...]";
const FLAGS: &[(&str, &str)] = &[
    ("-n", "suppress the trailing newline"),
    ("-e", "interpret backslash escapes"),
    ("-E", "do not interpret backslash escapes (the default)"),
];

/// Whether `arg` is an option group: a dash, then only `n`, `e` and `E`.
fn is_options(arg: &str) -> bool {
    arg.len() > 1
        && arg.starts_with('-')
        && arg[1..].bytes().all(|b| matches!(b, b'n' | b'e' | b'E'))
}

/// The value of up to `max` digits of `radix` at the start of `bytes`, and how many were used.
fn number(bytes: &[u8], radix: u32, max: usize) -> Option<(u8, usize)> {
    let mut value = 0u32;
    let mut used = 0;
    while used < max && used < bytes.len() {
        match (bytes[used] as char).to_digit(radix) {
            Some(d) => value = value * radix + d,
            None => break,
        }
        used += 1;
    }
    (used > 0).then_some((value as u8, used))
}

/// Appends `arg` to `out` with its escapes interpreted. `true` if `\c` was met: nothing more is
/// to be printed.
fn escaped(arg: &str, out: &mut Vec<u8>) -> bool {
    let bytes = arg.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' || i + 1 == bytes.len() {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        i += 1;
        let c = bytes[i];
        i += 1;
        match c {
            b'\\' => out.push(b'\\'),
            b'a' => out.push(7),
            b'b' => out.push(8),
            b'c' => return true,
            b'e' => out.push(27),
            b'f' => out.push(12),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'v' => out.push(11),
            b'0' => {
                // `\0NNN`: up to three more octal digits.
                let (value, used) = number(&bytes[i..], 8, 3).unwrap_or((0, 0));
                out.push(value);
                i += used;
            }
            b'x' => match number(&bytes[i..], 16, 2) {
                Some((value, used)) => {
                    out.push(value);
                    i += used;
                }
                None => out.extend_from_slice(b"\\x"), // no digits: as written
            },
            other => {
                out.push(b'\\');
                out.push(other);
            }
        }
    }
    false
}

fn run(args: Args) -> ExitCode {
    let mut args = args.skip(1).peekable();
    if args.peek() == Some(&"--help") {
        return progs::help(USAGE, FLAGS);
    }
    let (mut newline, mut interpret) = (true, false);
    while let Some(&arg) = args.peek() {
        if !is_options(arg) {
            break;
        }
        for b in arg[1..].bytes() {
            match b {
                b'n' => newline = false,
                b'e' => interpret = true,
                _ => interpret = false, // `E`
            }
        }
        args.next();
    }

    let mut out: Vec<u8> = Vec::new();
    let mut stopped = false;
    for (i, arg) in args.enumerate() {
        if i > 0 {
            out.push(b' ');
        }
        if interpret {
            if escaped(arg, &mut out) {
                stopped = true;
                break;
            }
        } else {
            out.extend_from_slice(arg.as_bytes());
        }
    }
    if newline && !stopped {
        out.push(b'\n');
    }
    progs::write_all(1, &out);
    ExitCode(0)
}
