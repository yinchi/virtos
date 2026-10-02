//! `tr [-d] [-s] SET1 [SET2]`: reads standard input and writes it to standard output with
//! characters translated (SET1 to SET2, position by position; a shorter SET2 is padded with its
//! last character), deleted (`-d`: those in SET1) or squeezed (`-s`: a run of one character from
//! the last set given becomes one). A set is literal characters, ranges (`a-z`), escapes (`\n`,
//! `\t`, `\\`, octal `\101`) and classes (`[:digit:]`, `[:upper:]`, ...). Sets are bytes and ASCII
//! only; every other byte of the input passes through, which keeps UTF-8 text intact.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use getargs::Arg;
use progs::cli;
use textlib::trset::{Tr, TrError, expand};
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "tr [-d] [-s] SET1 [SET2]";
const FLAGS: &[(&str, &str)] = &[
    ("-d", "delete the characters in SET1"),
    (
        "-s",
        "replace each run of a repeated character (from the last set) with one",
    ),
];

fn run(args: Args) -> ExitCode {
    cli::status(tr(args))
}

fn set_error(e: TrError, spec: &str) -> ExitCode {
    match e {
        TrError::NonAscii => cli::report("tr", "non-ASCII characters in a set are not supported"),
        TrError::ReverseRange => {
            cli::report(
                "tr",
                &format!("range endpoints in '{spec}' are in reverse order"),
            );
        }
        TrError::BadClass => cli::report("tr", &format!("invalid character class in '{spec}'")),
        TrError::EmptySecond => {
            cli::report("tr", "when not truncating set1, string2 must be non-empty");
        }
    }
    ExitCode(1)
}

fn tr(args: Args) -> Result<ExitCode, ExitCode> {
    let (mut delete, mut squeeze) = (false, false);
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("tr", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(progs::help(USAGE, FLAGS)),
            Arg::Short('d') | Arg::Long("delete") => delete = true,
            Arg::Short('s') | Arg::Long("squeeze-repeats") => squeeze = true,
            Arg::Positional(_) => {}
            other => return Err(cli::invalid("tr", other)),
        }
    }

    let mut sets = cli::operands(args);
    let Some(first) = sets.next() else {
        return Err(cli::missing_operand("tr"));
    };
    let second = sets.next();
    if let Some(extra) = sets.next() {
        return Err(cli::extra_operand("tr", extra));
    }
    // Translating needs both sets; deleting alone takes one; deleting and squeezing, or squeezing
    // alone, take one or two.
    match (delete, squeeze, second) {
        (false, false, None) => {
            cli::report("tr", &format!("missing operand after '{first}'"));
            cli::try_help("tr");
            return Err(ExitCode(1));
        }
        (true, false, Some(extra)) => return Err(cli::extra_operand("tr", extra)),
        _ => {}
    }

    let set1 = expand(first).map_err(|e| set_error(e, first))?;
    let set2 = match second {
        Some(spec) => Some(expand(spec).map_err(|e| set_error(e, spec))?),
        None => None,
    };
    let tr = Tr::new(&set1, set2.as_deref(), delete, squeeze).map_err(|e| set_error(e, first))?;

    let input = progs::read_fd(0);
    progs::write_all(1, &tr.apply(&input));
    Ok(ExitCode(0))
}
