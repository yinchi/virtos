//! `seq [-s SEP] [-w] [FIRST [INCREMENT]] LAST`: prints the numbers from FIRST to LAST, stepping by
//! INCREMENT (default 1, and FIRST defaults to 1), separated by SEP (default a newline) and ended
//! by a newline.
//!
//! Integers only (64-bit): the shell has no loops, so this is a source of input for `sort`, `head`
//! and friends, not a counter for scripts. A number may be negative, so the arguments are read by
//! hand: `getargs` would take `-5` for an option.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use core::fmt::Write;

use progs::Out;
use progs::cli::{extra_operand, missing_operand, report, try_help};
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "seq [-s SEP] [-w] [FIRST [INCREMENT]] LAST";
const FLAGS: &[(&str, &str)] = &[
    (
        "-s SEP",
        "separate the numbers with SEP instead of a newline",
    ),
    (
        "-w",
        "pad with leading zeros to the width of the widest of FIRST and LAST",
    ),
];

/// A number operand: an optional sign, then digits. `None` for anything else (a fraction, an
/// exponent, text).
fn number(text: &str) -> Option<i64> {
    let digits = text.strip_prefix('+').unwrap_or(text);
    if digits.strip_prefix('-').unwrap_or(digits).is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// Whether `arg` reads as an option rather than an operand: a dash, then something that is not a
/// digit (`-5` is a number).
fn is_option(arg: &str) -> bool {
    arg.len() > 1 && arg.starts_with('-') && !arg[1..].starts_with(|c: char| c.is_ascii_digit())
}

/// How many characters `n` takes, minus sign included.
fn width_of(n: i64) -> usize {
    let mut digits = 1;
    let mut rest = n.unsigned_abs() / 10;
    while rest > 0 {
        digits += 1;
        rest /= 10;
    }
    digits + usize::from(n < 0)
}

fn fail(msg: &str) -> ExitCode {
    report("seq", msg);
    try_help("seq");
    ExitCode(1)
}

fn run(args: Args) -> ExitCode {
    let mut separator = "\n";
    let mut equal_width = false;
    let mut numbers: [&str; 3] = [""; 3];
    let mut count = 0usize;
    let mut args = args.skip(1);
    let mut options_done = false;

    while let Some(arg) = args.next() {
        if !options_done && arg == "--" {
            options_done = true;
        } else if !options_done && arg == "--help" {
            return progs::help(USAGE, FLAGS);
        } else if !options_done && (arg == "-w" || arg == "--equal-width") {
            equal_width = true;
        } else if !options_done && (arg == "-s" || arg == "--separator") {
            let Some(value) = args.next() else {
                return fail("option requires an argument -- 's'");
            };
            separator = value;
        } else if !options_done && let Some(value) = arg.strip_prefix("-s") {
            separator = value;
        } else if !options_done && is_option(arg) {
            return if arg.starts_with("--") {
                fail(&format!("unrecognized option '{arg}'"))
            } else {
                let c = arg[1..].chars().next().unwrap_or('-');
                fail(&format!("invalid option -- '{c}'"))
            };
        } else if count < 3 {
            numbers[count] = arg;
            count += 1;
        } else {
            return extra_operand("seq", arg);
        }
    }
    if count == 0 {
        return missing_operand("seq");
    }

    let mut values = [1i64; 3]; // FIRST, INCREMENT, LAST
    let slots: &[usize] = match count {
        1 => &[2],
        2 => &[0, 2],
        _ => &[0, 1, 2],
    };
    for (slot, text) in slots.iter().zip(&numbers[..count]) {
        match number(text) {
            Some(n) => values[*slot] = n,
            None => return fail(&format!("invalid integer argument: '{text}'")),
        }
    }
    let [first, step, last] = values;
    if step == 0 {
        return fail(&format!("invalid Zero increment value: '{}'", numbers[1]));
    }

    // `-w`: zeros up to the width of the widest end, counting a minus sign.
    let width = if equal_width {
        width_of(first).max(width_of(last))
    } else {
        0
    };
    let mut out = Out::new();
    let mut next = Some(first);
    let mut printed = false;
    while let Some(n) = next {
        if (step > 0 && n > last) || (step < 0 && n < last) {
            break;
        }
        if printed {
            let _ = out.write_str(separator);
        }
        printed = true;
        if n < 0 {
            let _ = write!(out, "-{:0>1$}", n.unsigned_abs(), width.saturating_sub(1));
        } else {
            let _ = write!(out, "{n:0>width$}");
        }
        next = n.checked_add(step); // stops, rather than wrapping, at the end of the range of i64
    }
    if printed {
        out.put(b"\n");
    }
    ExitCode(0)
}
