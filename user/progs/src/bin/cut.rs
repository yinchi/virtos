//! `cut (-c LIST | -f LIST) [-d C] [-s] [file...]`: prints selected parts of each line: with `-c`
//! the characters at the positions in LIST (counted in UTF-8 characters, from 1), with `-f` the
//! fields (split at the delimiter `C`, a tab by default) whose numbers are in LIST. A LIST is `N`,
//! `N-M`, `N-`, `-M`, separated by commas. Selected fields are joined with the delimiter, in their
//! original order; a line with no delimiter is printed whole, unless `-s`.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use getargs::Arg;
use progs::{Out, cli};
use textlib::cutlist::{self, ListError, Range};
use textlib::textutil::{split_fields, split_lines};
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "cut (-c LIST | -f LIST) [-d C] [-s] [file...]";
const FLAGS: &[(&str, &str)] = &[
    ("-c LIST", "select these character positions"),
    ("-f LIST", "select these fields"),
    ("-d C", "the field delimiter, one character (default: tab)"),
    ("-s", "with -f, skip lines that have no delimiter"),
];

fn run(args: Args) -> ExitCode {
    cli::status(cut(args))
}

/// A usage problem: reported with the `Try` trailer; the status to return.
fn usage_error(msg: &str) -> ExitCode {
    cli::report("cut", msg);
    cli::try_help("cut");
    ExitCode(1)
}

fn list_error(e: ListError, what: &str, text: &str) -> ExitCode {
    match e {
        ListError::Invalid => usage_error(&format!("invalid {what} value '{text}'")),
        ListError::Zero => usage_error(&format!("{what}s are numbered from 1")),
        ListError::Decreasing => usage_error("invalid decreasing range"),
    }
}

fn cut(args: Args) -> Result<ExitCode, ExitCode> {
    let (mut fields, mut chars): (Option<&'static str>, Option<&'static str>) = (None, None);
    let mut delimiter = "\t";
    let mut only_delimited = false;
    // Operands are gathered here, not by `cli::operands`: that second walk would take an option's
    // value for one.
    let mut names: Vec<&'static str> = Vec::new();
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("cut", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(progs::help(USAGE, FLAGS)),
            Arg::Short('f') | Arg::Long("fields") => fields = Some(cli::value("cut", &mut opts)?),
            Arg::Short('c') | Arg::Long("characters") => {
                chars = Some(cli::value("cut", &mut opts)?);
            }
            Arg::Short('d') | Arg::Long("delimiter") => {
                delimiter = cli::value("cut", &mut opts)?;
            }
            Arg::Short('s') | Arg::Long("only-delimited") => only_delimited = true,
            Arg::Positional(name) => names.push(name),
            other => return Err(cli::invalid("cut", other)),
        }
    }
    if delimiter.chars().count() != 1 {
        return Err(usage_error("the delimiter must be a single character"));
    }
    let (ranges, by_field): (Vec<Range>, bool) = match (fields, chars) {
        (None, None) => {
            return Err(usage_error(
                "you must specify a list of characters or fields",
            ));
        }
        (Some(_), Some(_)) => return Err(usage_error("only one type of list may be specified")),
        (Some(list), None) => (
            cutlist::parse(list).map_err(|e| list_error(e, "field", list))?,
            true,
        ),
        (None, Some(list)) => (
            cutlist::parse(list).map_err(|e| list_error(e, "character position", list))?,
            false,
        ),
    };
    if only_delimited && !by_field {
        return Err(usage_error(
            "suppressing non-delimited lines makes sense only when operating on fields",
        ));
    }

    if names.is_empty() {
        names.push("-");
    }
    let mut out = Out::new();
    let mut status = 0;
    for name in names {
        let data = match progs::read_operand(name) {
            Ok(data) => data,
            Err(e) => {
                cli::report_errno("cut", name, e);
                status = 1;
                continue;
            }
        };
        for line in split_lines(&data) {
            if by_field {
                cut_fields(
                    &mut out,
                    line,
                    delimiter.as_bytes(),
                    &ranges,
                    only_delimited,
                );
            } else {
                cut_chars(&mut out, line, &ranges);
            }
        }
    }
    Ok(ExitCode(status))
}

fn cut_fields(
    out: &mut Out,
    line: &[u8],
    delimiter: &[u8],
    ranges: &[Range],
    only_delimited: bool,
) {
    let fields = split_fields(line, delimiter);
    if fields.len() == 1 {
        // No delimiter in the line: whole, unless it is to be skipped.
        if !only_delimited {
            out.put(line);
            out.put(b"\n");
        }
        return;
    }
    let mut first = true;
    for (i, field) in fields.iter().enumerate() {
        if cutlist::contains(ranges, i + 1) {
            if !first {
                out.put(delimiter);
            }
            out.put(field);
            first = false;
        }
    }
    out.put(b"\n");
}

/// The characters of `line` at the positions in `ranges`. A line that is not valid UTF-8 is cut
/// by bytes instead.
fn cut_chars(out: &mut Out, line: &[u8], ranges: &[Range]) {
    match core::str::from_utf8(line) {
        Ok(text) => {
            for (i, (at, c)) in text.char_indices().enumerate() {
                if cutlist::contains(ranges, i + 1) {
                    out.put(&line[at..at + c.len_utf8()]);
                }
            }
        }
        Err(_) => {
            for (i, byte) in line.iter().enumerate() {
                if cutlist::contains(ranges, i + 1) {
                    out.put(core::slice::from_ref(byte));
                }
            }
        }
    }
    out.put(b"\n");
}
