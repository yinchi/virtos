//! `cmp [-s] FILE1 FILE2`: compares two files byte by byte. Exit status 0 if they are the same, 1
//! if they differ (after naming the first byte that does, and its line), 2 if it could not
//! compare them. `-` is standard input. Meant for checking that a `cp` or `mv` kept a file whole,
//! so it reads both files in one pass and holds neither.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use getargs::Arg;
use progs::{Out, cli};
use userlib::{Args, ExitCode, O_RDONLY};

userlib::entry_with_args!(run);

const USAGE: &str = "cmp [-s] FILE1 FILE2";
const FLAGS: &[(&str, &str)] = &[(
    "-s",
    "print nothing; the exit status says whether they differ",
)];

const CHUNK: usize = 512;

/// Fills `buf` from `fd` as far as it goes (a `read` may return less, notably from the keyboard
/// or a pipe): the count, which is short only at the end of the file.
fn fill(fd: usize, buf: &mut [u8]) -> usize {
    let mut have = 0;
    while have < buf.len() {
        let n = userlib::read(fd, &mut buf[have..]);
        if n <= 0 {
            break;
        }
        have += n as usize;
    }
    have
}

/// Opens `name` (`-` is stdin, fd 0); reports and returns `None` if it cannot be opened.
fn open(name: &str) -> Option<usize> {
    if name == "-" {
        return Some(0);
    }
    let fd = userlib::open(name, O_RDONLY);
    if fd < 0 {
        cli::report_errno("cmp", name, fd);
        return None;
    }
    Some(fd as usize)
}

fn run(args: Args) -> ExitCode {
    let mut silent = false;
    let mut names: [&str; 2] = [""; 2];
    let mut count = 0usize;
    let mut opts = cli::opts(args);
    loop {
        match cli::next("cmp", &mut opts) {
            Ok(None) => break,
            Ok(Some(Arg::Long("help"))) => return progs::help(USAGE, FLAGS),
            Ok(Some(Arg::Short('s') | Arg::Long("quiet" | "silent"))) => silent = true,
            Ok(Some(Arg::Positional(name))) => {
                if count == 2 {
                    cli::extra_operand("cmp", name);
                    return ExitCode(2);
                }
                names[count] = name;
                count += 1;
            }
            Ok(Some(other)) => {
                cli::invalid("cmp", other);
                return ExitCode(2);
            }
            Err(_) => return ExitCode(2),
        }
    }
    if count < 2 {
        if count == 0 {
            cli::missing_operand("cmp");
        } else {
            cli::report("cmp", &format!("missing operand after '{}'", names[0]));
            cli::try_help("cmp");
        }
        return ExitCode(2);
    }

    let Some(first) = open(names[0]) else {
        return ExitCode(2);
    };
    let Some(second) = open(names[1]) else {
        return ExitCode(2);
    };

    let mut out = Out::new();
    let mut a = [0u8; CHUNK];
    let mut b = [0u8; CHUNK];
    let mut offset = 0usize; // bytes already compared
    let mut line = 1usize; // the line the next byte is on, by the first file's newlines
    loop {
        let na = fill(first, &mut a);
        let nb = fill(second, &mut b);
        let common = na.min(nb);
        for i in 0..common {
            if a[i] != b[i] {
                if !silent {
                    out.put(
                        format!(
                            "{} {} differ: byte {}, line {line}\n",
                            names[0],
                            names[1],
                            offset + i + 1
                        )
                        .as_bytes(),
                    );
                }
                return ExitCode(1);
            }
            if a[i] == b'\n' {
                line += 1;
            }
        }
        offset += common;
        if na != nb {
            // One file ended first: it is a prefix of the other.
            if !silent {
                let shorter = if na < nb { names[0] } else { names[1] };
                cli::report("cmp", &format!("EOF on {shorter} after byte {offset}"));
            }
            return ExitCode(1);
        }
        if na < CHUNK {
            return ExitCode(0); // both ended together
        }
    }
}
