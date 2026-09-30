//! Prints the last N lines (`-n`/`--lines`, default 10) of each named file, in order.
#![no_std]
#![no_main]

use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const DEFAULT_LINES: usize = 10;

/// The number of lines in `data`: one per `\n`, plus one more if the last byte isn't one (an
/// unterminated final line still counts).
fn count_lines(data: &[u8]) -> usize {
    let newlines = data.iter().filter(|&&b| b == b'\n').count();
    let ends_open = data.last().is_some_and(|&b| b != b'\n');
    newlines + ends_open as usize
}

/// The last `n` lines of `data`: skip `data`'s total line count minus `n` lines (`saturating_sub`
/// -- `n` at or past the total skips nothing, `n = 0` skips everything, both for free, no special
/// case needed for either), then return what's left.
fn tail_lines(data: &[u8], n: usize) -> &[u8] {
    let mut skip = count_lines(data).saturating_sub(n);
    let mut start = 0;
    while skip > 0 {
        match data[start..].iter().position(|&b| b == b'\n') {
            Some(i) => {
                start += i + 1;
                skip -= 1;
            }
            // No more newlines left to skip past: the rest of `data` is the unterminated final
            // line `count_lines` counted, and `skip` reaching here means even that one is skipped.
            None => return &data[data.len()..],
        }
    }
    &data[start..]
}

fn run(args: Args) -> ExitCode {
    let (n, paths) = match progs::cli::line_count("tail", args, DEFAULT_LINES) {
        Ok(v) => v,
        Err(status) => return status,
    };
    if paths.is_empty() {
        userlib::write(2, b"tail: missing file operand\n");
        return ExitCode(1);
    }
    let mut ok = true;
    for path in paths {
        let Some(data) = progs::read_file(path) else {
            userlib::write(2, b"tail: ");
            userlib::write(2, path.as_bytes());
            userlib::write(2, b": cannot open\n");
            ok = false;
            continue;
        };
        userlib::write(1, tail_lines(&data, n));
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
