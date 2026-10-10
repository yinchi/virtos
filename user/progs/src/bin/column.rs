//! `column [-t] [-x] [-s SEPARATORS] [-o STRING] [-c WIDTH] [FILE...]`: lays text out in columns.
//! Without `-t`, each input line is one entry and the entries are filled into columns to the
//! width (`-c`, else the console's width on a terminal, else 80), down the columns first or, with
//! `-x`, across the rows first -- the same arrangement `ls` gives on a terminal. With `-t`, each
//! line is split into fields (blanks by default, or each character of `-s`) and the fields are
//! aligned into a table, the cells joined by `-o` (two spaces by default). Empty lines are
//! skipped. No operand, or `-`, reads standard input.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use getargs::Arg;
use progs::{Out, cli};
use textlib::columns::{self, Fill};
use textlib::width::text_width;
use userlib::{Args, ExitCode, winsize};

userlib::entry_with_args!(run);

const USAGE: &str = "column [-t] [-x] [-s SEPARATORS] [-o STRING] [-c WIDTH] [FILE...]";
const FLAGS: &[(&str, &str)] = &[
    ("-t", "make a table: split lines into fields and align them"),
    (
        "-s SEPARATORS",
        "with -t, the characters that separate fields (default: blanks)",
    ),
    (
        "-o STRING",
        "with -t, put STRING between columns (default: two spaces)",
    ),
    (
        "-c WIDTH",
        "fill to this many columns (default: the terminal's, else 80)",
    ),
    ("-x", "fill rows before columns"),
];

fn run(args: Args) -> ExitCode {
    cli::status(column(args))
}

/// A usage problem: reported with the `Try` trailer; the status to return.
fn usage_error(msg: &str) -> ExitCode {
    cli::report("column", msg);
    cli::try_help("column");
    ExitCode(1)
}

fn column(args: Args) -> Result<ExitCode, ExitCode> {
    let mut table = false;
    let mut fill = Fill::Down;
    let mut separators: Option<&'static str> = None;
    let mut output_separator = "  ";
    let mut width: Option<usize> = None;
    let mut names: Vec<&'static str> = Vec::new();
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("column", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(progs::help(USAGE, FLAGS)),
            Arg::Short('t') | Arg::Long("table") => table = true,
            Arg::Short('x') | Arg::Long("fillrows") => fill = Fill::Across,
            Arg::Short('s') | Arg::Long("separator") => {
                separators = Some(cli::value("column", &mut opts)?);
            }
            Arg::Short('o') | Arg::Long("output-separator") => {
                output_separator = cli::value("column", &mut opts)?;
            }
            Arg::Short('c') | Arg::Long("output-width") => {
                let text = cli::value("column", &mut opts)?;
                match text.parse::<usize>() {
                    Ok(n) if n > 0 => width = Some(n),
                    _ => {
                        return Err(usage_error(&alloc::format!(
                            "invalid columns argument '{text}'"
                        )));
                    }
                }
            }
            Arg::Positional(name) => names.push(name),
            other => return Err(cli::invalid("column", other)),
        }
    }
    if names.is_empty() {
        names.push("-");
    }

    // All the input, as lines: every operand in turn, empty lines dropped.
    let mut status = 0;
    let mut texts: Vec<String> = Vec::new();
    for name in names {
        match progs::read_operand(name) {
            Ok(data) => texts.push(String::from_utf8_lossy(&data).into_owned()),
            Err(e) => {
                cli::report_errno("column", name, e);
                status = 1;
            }
        }
    }
    let lines: Vec<&str> = texts
        .iter()
        .flat_map(|text| text.split('\n'))
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .filter(|line| !line.trim().is_empty())
        .collect();

    let mut out = Out::new();
    if table {
        let rows: Vec<Vec<&str>> = lines
            .iter()
            .map(|line| columns::split_fields(line, separators))
            .collect();
        out.put(columns::table(&rows, output_separator).as_bytes());
    } else {
        let width = width
            .or_else(|| winsize(1).ok().map(|size| usize::from(size.cols)))
            .unwrap_or(80);
        let widths: Vec<usize> = lines.iter().map(|line| text_width(line)).collect();
        let grid = columns::plan(&widths, width, fill);
        out.put(columns::render(&lines, &widths, &grid).as_bytes());
    }
    Ok(ExitCode(status))
}
