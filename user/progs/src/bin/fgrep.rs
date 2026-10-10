//! `fgrep [-i] [-v] [-w] [-x] [-n] [-c] [-l] [-q] [-o] [-H|-h] [-e PATTERN]... [-f FILE]...
//! [PATTERN] [file...]`: prints the lines that contain any of the PATTERNs as **fixed strings** (no
//! regular expressions: `.` and `*` are ordinary characters). Standard input for no file, or `-`.
//! With more than one file each line is prefixed with its file's name (`-H` always, `-h` never).
//! `-w` wants the match bounded by non-word bytes, `-x` wants it to be the whole line, `-o` prints
//! only the matched parts, one per line, and `-f FILE` reads patterns from a file, one per line.
//! Status 0 if any line was selected, 1 if none, 2 if a file could not be read. This is GNU's
//! `fgrep`, i.e. `grep -F`.
#![no_std]
#![no_main]

extern crate alloc;

use core::fmt::Write;

use alloc::vec::Vec;
use getargs::Arg;
use progs::{Out, cli};
use textlib::textutil::{MatchMode, match_in_line, split_lines};
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "fgrep [-i] [-v] [-w] [-x] [-n] [-c] [-l] [-q] [-o] [-H|-h] [-e PATTERN]... [-f FILE]... [PATTERN] [file...]";
const FLAGS: &[(&str, &str)] = &[
    (
        "-e PATTERN",
        "a pattern to look for (may be repeated; without -e the first operand is the pattern)",
    ),
    (
        "-f FILE",
        "read patterns from FILE, one per line (may be repeated, and combined with -e)",
    ),
    ("-i", "ignore case (ASCII letters)"),
    ("-v", "select the lines that do not match"),
    (
        "-w",
        "match only whole words (bounded by non-word characters)",
    ),
    ("-x", "match only whole lines"),
    ("-o", "print only the matched parts, one per line"),
    ("-H", "always prefix each line with its file's name"),
    ("-h", "never prefix a line with its file's name"),
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
    let (mut only_matching, mut mode) = (false, MatchMode::Any);
    let mut with_names: Option<bool> = None; // `-H`/`-h`; the last wins
    let mut pattern_files: Vec<&'static str> = Vec::new();
    let mut patterns: Vec<&'static str> = Vec::new();
    let mut operands: Vec<&'static str> = Vec::new();
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("fgrep", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(progs::help(USAGE, FLAGS)),
            Arg::Short('e') | Arg::Long("regexp") => {
                patterns.push(cli::value("fgrep", &mut opts)?);
            }
            Arg::Short('f') | Arg::Long("file") => {
                pattern_files.push(cli::value("fgrep", &mut opts)?);
            }
            Arg::Short('i') | Arg::Long("ignore-case") => ignore_case = true,
            // `-x` outranks `-w`, as in GNU (a whole line is already bounded).
            Arg::Short('w') | Arg::Long("word-regexp") => {
                if mode != MatchMode::Line {
                    mode = MatchMode::Word;
                }
            }
            Arg::Short('x') | Arg::Long("line-regexp") => mode = MatchMode::Line,
            Arg::Short('o') | Arg::Long("only-matching") => only_matching = true,
            Arg::Short('H') | Arg::Long("with-filename") => with_names = Some(true),
            Arg::Short('h') | Arg::Long("no-filename") => with_names = Some(false),
            Arg::Short('v') | Arg::Long("invert-match") => invert = true,
            Arg::Short('n') | Arg::Long("line-number") => numbers = true,
            Arg::Short('c') | Arg::Long("count") => count = true,
            Arg::Short('l') | Arg::Long("files-with-matches") => list = true,
            Arg::Short('q') | Arg::Long("quiet" | "silent") => quiet = true,
            Arg::Positional(operand) => operands.push(operand),
            other => return Err(cli::invalid("fgrep", other)),
        }
    }
    // `-f FILE`'s lines are patterns too; a file with none contributes none (and so matches
    // nothing, or with `-v` everything).
    let mut file_data: Vec<Vec<u8>> = Vec::new();
    let mut have_pattern_option = !patterns.is_empty();
    for name in pattern_files {
        have_pattern_option = true;
        match progs::read_operand(name) {
            Ok(data) => file_data.push(data),
            Err(e) => {
                cli::report_errno("fgrep", name, e);
                return Err(ExitCode(2));
            }
        }
    }
    // Without -e or -f the first operand is the pattern.
    if !have_pattern_option {
        if operands.is_empty() {
            cli::missing_operand("fgrep");
            return Err(ExitCode(2));
        }
        patterns.push(operands.remove(0));
    }
    let patterns: Vec<&[u8]> = patterns
        .iter()
        .map(|p| p.as_bytes())
        .chain(file_data.iter().flat_map(|data| split_lines(data)))
        .collect();
    if operands.is_empty() {
        operands.push("-");
    }
    let show_names = with_names.unwrap_or(operands.len() > 1);

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
                .any(|p| match_in_line(line, p, 0, ignore_case, mode).is_some());
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
            // `-o` has nothing to show for a line selected by *not* matching.
            if only_matching && invert {
                continue;
            }
            let prefix = |out: &mut Out| {
                if show_names {
                    let _ = write!(out, "{label}:");
                }
                if numbers {
                    let _ = write!(out, "{}:", i + 1);
                }
            };
            if only_matching {
                // Each match, left to right, of whichever pattern starts earliest (the longer
                // one on a tie); the next search starts where it ended. An empty match shows
                // nothing and is stepped over.
                let mut at = 0;
                while at <= line.len() {
                    let best = patterns
                        .iter()
                        .filter_map(|p| match_in_line(line, p, at, ignore_case, mode))
                        .min_by_key(|&(start, end)| (start, core::cmp::Reverse(end)));
                    let Some((start, end)) = best else { break };
                    if end > start {
                        prefix(&mut out);
                        out.put(&line[start..end]);
                        out.put(b"\n");
                        at = end;
                    } else {
                        at = start + 1;
                    }
                }
            } else {
                prefix(&mut out);
                out.put(line);
                out.put(b"\n");
            }
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
