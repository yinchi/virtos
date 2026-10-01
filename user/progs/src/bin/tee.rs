//! `tee [-a] [file...]` -- copies stdin to stdout, also writing the same bytes to every named
//! file. POSIX allows zero file operands (just an odd way to copy stdin to stdout), so that
//! isn't a usage error here either. `-a` appends to each file instead of truncating it.
//!
//! One deliberate simplification vs. the mature reference: no per-destination "stop writing,
//! report only once" latch on a write failure mid-stream -- a real write failure on a FAT-backed
//! file isn't exercised by anything in this project yet, and re-reporting on every later chunk
//! if it ever happened is cosmetic, not a correctness gap. A recorded cut, not a silent omission.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use getargs::Arg;
use userlib::{Args, ExitCode, O_APPEND, O_WRONLY};

userlib::entry_with_args!(run);

const USAGE: &str = "tee [-a] [file...]";
const FLAGS: &[(&str, &str)] = &[("-a", "append to each file instead of truncating it")];

/// Same chunk size `progs::read_fd` already uses for its own buffer.
const CHUNK: usize = 512;

fn run(args: Args) -> ExitCode {
    let mut append = false;
    let mut opts = progs::cli::opts(args);
    loop {
        let arg = match progs::cli::next("tee", &mut opts) {
            Ok(Some(arg)) => arg,
            Ok(None) => break,
            Err(status) => return status,
        };
        match arg {
            Arg::Long("help") => return progs::help(USAGE, FLAGS),
            Arg::Short('a') | Arg::Long("append") => append = true,
            Arg::Positional(_) => {}
            other => return progs::cli::invalid("tee", other),
        }
    }

    let flags = O_WRONLY | if append { O_APPEND } else { 0 };
    let mut dests: Vec<(&str, usize)> = Vec::new();
    let mut status = 0;
    for path in progs::cli::operands(args) {
        let fd = userlib::open(path, flags);
        if fd < 0 {
            userlib::write(2, b"tee: ");
            userlib::write(2, path.as_bytes());
            userlib::write(2, b": cannot open\n");
            status = 1;
            continue;
        }
        dests.push((path, fd as usize));
    }

    let mut buf = [0u8; CHUNK];
    loop {
        let n = userlib::read(0, &mut buf);
        if n < 0 {
            userlib::write(2, b"tee: stdin: read error\n");
            status = 1;
            break;
        }
        if n == 0 {
            break;
        }
        let chunk = &buf[..n as usize];
        if userlib::write(1, chunk) < 0 {
            userlib::write(2, b"tee: stdout: write error\n");
            status = 1;
        }
        for (path, fd) in &dests {
            if userlib::write(*fd, chunk) < 0 {
                userlib::write(2, b"tee: ");
                userlib::write(2, path.as_bytes());
                userlib::write(2, b": write error\n");
                status = 1;
            }
        }
    }

    for (path, fd) in &dests {
        if userlib::close(*fd) < 0 {
            userlib::write(2, b"tee: ");
            userlib::write(2, path.as_bytes());
            userlib::write(2, b": close error\n");
            status = 1;
        }
    }

    ExitCode(status)
}
