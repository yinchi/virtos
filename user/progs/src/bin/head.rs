//! Prints the first N lines (`-n`/`--lines`, default 10) of each named file, in order.
#![no_std]
#![no_main]

use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const DEFAULT_LINES: usize = 10;

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
    let (n, paths) = match progs::cli::line_count("head", args, DEFAULT_LINES) {
        Ok(v) => v,
        Err(status) => return status,
    };
    if paths.is_empty() {
        userlib::write(2, b"head: missing file operand\n");
        return ExitCode(1);
    }
    let mut ok = true;
    for path in paths {
        let Some(data) = progs::read_file(path) else {
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
