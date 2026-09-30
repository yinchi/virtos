//! Prints the first N lines (`-n`/`--lines`, default 10) of each named file, in order. No
//! operand, or a lone `-` as one, reads stdin (`progs::read_operand`'s own doc comment).
#![no_std]
#![no_main]

use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const DEFAULT_LINES: usize = 10;
const USAGE: &str = "head [-n N] [file...]";
const FLAGS: &[(&str, &str)] = &[("-n N, --lines N", "print the first N lines (default 10)")];

/// The first `n` lines of `data` -- up through and including the `n`th `\n`, or the whole slice if
/// it has fewer than `n` newlines at all (matches a real `head`: a short file just prints whole).
/// Empty if `n` is `0`.
fn head_lines(data: &[u8], n: usize) -> &[u8] {
    if n == 0 {
        return &data[..0];
    }
    let mut seen = 0;
    for (i, &b) in data.iter().enumerate() {
        if b == b'\n' {
            seen += 1;
            if seen == n {
                return &data[..=i];
            }
        }
    }
    data
}

fn run(args: Args) -> ExitCode {
    let (n, mut paths) = match progs::cli::line_count("head", USAGE, FLAGS, args, DEFAULT_LINES) {
        Ok(v) => v,
        Err(status) => return status,
    };
    if paths.is_empty() {
        paths.push("-");
    }
    let mut ok = true;
    for path in paths {
        let Some(data) = progs::read_operand(path) else {
            userlib::write(2, b"head: ");
            userlib::write(2, path.as_bytes());
            userlib::write(2, b": cannot open\n");
            ok = false;
            continue;
        };
        userlib::write(1, head_lines(&data, n));
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
