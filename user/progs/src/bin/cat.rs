//! `cat [-n] [-E] [-T] [-s] [file...]`: prints each named file's content to stdout, in order. The
//! first real consumer of `open`/`close`, proving that round trip the same way `hello` proved
//! `exit` and `echo` proved `argv`. No operand, or a lone `-` as one, reads fd 0 instead (GNU's
//! own convention: `cat a - b` reads `a`, then stdin, then `b`) -- the interactive keyboard
//! (typed lines, Ctrl+D to end), a redirected file or a pipe.
//!
//! `-n` numbers every output line, `-E` puts a `$` at the end of each, `-T` shows a tab as `^I`,
//! `-s` squeezes runs of blank lines into one. Numbers and the blank-line memory carry across
//! files, as in GNU's. With none of the flags it is the plain byte copy it always was, streamed
//! through a fixed buffer, so binary files are safe to `cat`.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use getargs::Arg;
use progs::{cli, write_all};
use textlib::catshow::{Flags, State};
use userlib::{Args, ExitCode, O_RDONLY};

userlib::entry_with_args!(run);

/// Read in fixed chunks off the stack -- no reason to reach for the heap just to copy bytes
/// through unmodified.
const BUF_SIZE: usize = 512;
const USAGE: &str = "cat [-n] [-E] [-T] [-s] [file...]";
const FLAGS: &[(&str, &str)] = &[
    ("-n", "number all output lines"),
    ("-E", "display $ at the end of each line"),
    ("-T", "display tabs as ^I"),
    ("-s", "squeeze runs of blank lines into one"),
];

/// Streams `path`'s content to stdout -- fd 0 directly if `path` is `-`, otherwise `open`ed and
/// `close`d here. `false` (after reporting it) if `path` names a real file that fails to open.
fn cat_one(path: &str, flags: Flags, state: &mut State) -> bool {
    let fd = if path == "-" {
        0
    } else {
        let fd = userlib::open(path, O_RDONLY);
        if fd < 0 {
            cli::report_errno("cat", path, fd);
            return false;
        }
        fd as usize
    };
    let mut buf = [0u8; BUF_SIZE];
    let mut shown = Vec::new();
    let mut ok = true;
    loop {
        let n = userlib::read(fd, &mut buf);
        if n < 0 {
            cli::report_errno("cat", path, n); // a directory, say
            ok = false;
            break;
        }
        if n == 0 {
            break;
        }
        let chunk = &buf[..n as usize];
        if flags.any() {
            shown.clear();
            state.show(flags, chunk, &mut shown);
            write_all(1, &shown);
        } else {
            write_all(1, chunk);
        }
    }
    if path != "-" {
        userlib::close(fd);
    }
    ok
}

fn run(args: Args) -> ExitCode {
    cli::status(concatenate(args))
}

fn concatenate(args: Args) -> Result<ExitCode, ExitCode> {
    let mut flags = Flags::default();
    let mut any_file = false;
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("cat", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(progs::help(USAGE, FLAGS)),
            Arg::Short('n') | Arg::Long("number") => flags.number = true,
            Arg::Short('E') | Arg::Long("show-ends") => flags.dollar = true,
            Arg::Short('T') | Arg::Long("show-tabs") => flags.tabs = true,
            Arg::Short('s') | Arg::Long("squeeze-blank") => flags.squeeze = true,
            Arg::Positional(_) => any_file = true,
            other => return Err(cli::invalid("cat", other)),
        }
    }

    let mut state = State::default();
    let mut ok = true;
    if !any_file {
        cat_one("-", flags, &mut state);
    }
    for path in cli::operands(args) {
        if !cat_one(path, flags, &mut state) {
            ok = false;
        }
    }
    Ok(ExitCode(if ok { 0 } else { 1 }))
}
