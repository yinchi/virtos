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

use alloc::format;
use alloc::vec::Vec;
use getargs::Arg;
use progs::{cli, write_all};
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

#[derive(Default, Clone, Copy)]
struct Flags {
    number: bool,
    dollar: bool,
    tabs: bool,
    squeeze: bool,
}

impl Flags {
    fn any(self) -> bool {
        self.number || self.dollar || self.tabs || self.squeeze
    }
}

/// Where the output has got to, across chunks and across files.
#[derive(Default)]
struct State {
    /// The number of the last line numbered.
    line_no: usize,
    /// Whether the line being written has started (its number, if any, is out).
    in_line: bool,
    /// Whether the last line written was blank, for `-s`.
    prev_blank: bool,
}

impl State {
    fn number(&mut self, out: &mut Vec<u8>) {
        self.line_no += 1;
        out.extend_from_slice(format!("{:>6}\t", self.line_no).as_bytes());
    }

    /// Appends `chunk`, as the flags say it is shown, to `out`.
    fn show(&mut self, flags: Flags, chunk: &[u8], out: &mut Vec<u8>) {
        for &b in chunk {
            if b == b'\n' {
                if !self.in_line {
                    // A blank line: dropped, if it repeats one and `-s` says so.
                    if flags.squeeze && self.prev_blank {
                        continue;
                    }
                    if flags.number {
                        self.number(out);
                    }
                    self.prev_blank = true;
                } else {
                    self.prev_blank = false;
                }
                if flags.dollar {
                    out.push(b'$');
                }
                out.push(b'\n');
                self.in_line = false;
            } else {
                if !self.in_line {
                    if flags.number {
                        self.number(out);
                    }
                    self.in_line = true;
                }
                if b == b'\t' && flags.tabs {
                    out.extend_from_slice(b"^I");
                } else {
                    out.push(b);
                }
            }
        }
    }
}

/// Streams `path`'s content to stdout -- fd 0 directly if `path` is `-`, otherwise `open`ed and
/// `close`d here. `false` (after reporting it) if `path` names a real file that fails to open.
fn cat_one(path: &str, flags: Flags, state: &mut State) -> bool {
    let fd = if path == "-" {
        0
    } else {
        let fd = userlib::open(path, O_RDONLY);
        if fd < 0 {
            cli::report("cat", &format!("{path}: cannot open"));
            return false;
        }
        fd as usize
    };
    let mut buf = [0u8; BUF_SIZE];
    let mut shown = Vec::new();
    loop {
        let n = userlib::read(fd, &mut buf);
        if n <= 0 {
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
    true
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
