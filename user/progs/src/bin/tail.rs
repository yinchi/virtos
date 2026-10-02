//! `tail [-q] [-v] [-n N | -c N] [file...]`: prints the last N lines (`-n`/`--lines`, default
//! 10) or bytes (`-c`/`--bytes`) of each named file, in order. A plus count (`-n +5`) is
//! everything from the fifth on. With several files each is preceded by a `==> name <==` header
//! (`-q` for none, `-v` for one even with a single file). No operand, or a lone `-` as one, reads
//! stdin (`progs::read_operand`'s own doc comment). The whole input is read first, then sliced
//! (`textlib::headtail`).
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use progs::{Out, cli};
use textlib::headtail;
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const DEFAULT_LINES: usize = 10;
const USAGE: &str = "tail [-q] [-v] [-n N | -c N] [file...]";
const FLAGS: &[(&str, &str)] = &[
    (
        "-n N, --lines N",
        "print the last N lines (default 10); -n +N prints from line N on",
    ),
    (
        "-c N, --bytes N",
        "print the last N bytes; -c +N prints from byte N on",
    ),
    ("-q", "never print headers giving file names"),
    ("-v", "always print headers giving file names"),
];

fn run(args: Args) -> ExitCode {
    let mut o = match cli::count_args("tail", USAGE, FLAGS, args, DEFAULT_LINES) {
        Ok(o) => o,
        Err(status) => return status,
    };
    let show_names = o.verbose || (o.files.len() > 1 && !o.quiet);
    if o.files.is_empty() {
        o.files = Vec::from(["-"]);
    }
    let mut out = Out::new();
    let mut ok = true;
    let mut printed = false;
    for path in o.files {
        let data = match progs::read_operand(path) {
            Ok(data) => data,
            Err(e) => {
                cli::report_errno("tail", path, e);
                ok = false;
                continue;
            }
        };
        if show_names {
            let name = if path == "-" { "standard input" } else { path };
            out.put(format!("{}==> {name} <==\n", if printed { "\n" } else { "" }).as_bytes());
        }
        printed = true;
        out.put(headtail::tail(&data, o.mode, o.sign, o.count));
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
