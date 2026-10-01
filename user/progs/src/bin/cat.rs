//! Prints each named file's content to stdout, in order. The first real consumer of `open`/
//! `close`, proving that round trip the same way `hello` proved `exit` and `echo` proved `argv`.
//! No operand, or a lone `-` as one, reads fd 0 instead (GNU's own convention: `cat a - b` reads
//! `a`, then stdin, then `b`) -- works for a redirected or piped stdin (a real, blocking
//! `FileReader`/pipe-stage fd); typed bare at the interactive prompt it still reads as empty,
//! since the `Keyboard` fd is non-blocking with nothing to block on yet (`plans/PROGS.md`'s
//! conventions section).
#![no_std]
#![no_main]

use userlib::{Args, ExitCode, O_RDONLY};

userlib::entry_with_args!(run);

/// Read in fixed chunks off the stack -- no reason to reach for the heap just to copy bytes
/// through unmodified.
const BUF_SIZE: usize = 512;
const USAGE: &str = "cat [file...]";
const FLAGS: &[(&str, &str)] = &[];

/// Streams `path`'s content to stdout -- fd 0 directly if `path` is `-`, otherwise `open`ed and
/// `close`d here. `false` (after reporting it) if `path` names a real file that fails to open.
fn cat_one(path: &str) -> bool {
    let fd = if path == "-" {
        0
    } else {
        let fd = userlib::open(path, O_RDONLY);
        if fd < 0 {
            userlib::write(2, b"cat: ");
            userlib::write(2, path.as_bytes());
            userlib::write(2, b": cannot open\n");
            return false;
        }
        fd as usize
    };
    let mut buf = [0u8; BUF_SIZE];
    loop {
        let n = userlib::read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        userlib::write(1, &buf[..n as usize]);
    }
    if path != "-" {
        userlib::close(fd);
    }
    true
}

fn run(args: Args) -> ExitCode {
    let plain = match progs::cli::plain("cat", USAGE, FLAGS, args) {
        Ok(plain) => plain,
        Err(status) => return status,
    };
    let mut ok = true;
    if plain.count == 0 {
        cat_one("-");
    }
    for path in progs::cli::operands(args) {
        if !cat_one(path) {
            ok = false;
        }
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
