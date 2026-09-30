//! Prints each named file's content to stdout, in order. The first real consumer of `open`/
//! `close`, proving that round trip the same way `hello` proved `exit` and `echo` proved `argv`.
//! No flags, no stdin fallback when no file is named (an ordinary `cat` reads stdin then) -- just
//! enough to exercise the new syscalls; real behavior gets filled in if something needs it.
#![no_std]
#![no_main]

use userlib::{Args, ExitCode, O_RDONLY};

userlib::entry_with_args!(run);

/// Read in fixed chunks off the stack -- no reason to reach for the heap just to copy bytes
/// through unmodified.
const BUF_SIZE: usize = 512;

fn run(args: Args) -> ExitCode {
    let mut any = false;
    let mut ok = true;
    for path in args.skip(1) {
        any = true;
        let fd = userlib::open(path, O_RDONLY);
        if fd < 0 {
            userlib::write(2, b"cat: ");
            userlib::write(2, path.as_bytes());
            userlib::write(2, b": cannot open\n");
            ok = false;
            continue;
        }
        let fd = fd as usize;
        let mut buf = [0u8; BUF_SIZE];
        loop {
            let n = userlib::read(fd, &mut buf);
            if n <= 0 {
                break;
            }
            userlib::write(1, &buf[..n as usize]);
        }
        userlib::close(fd);
    }
    if !any {
        userlib::write(2, b"cat: missing file operand\n");
        return ExitCode(1);
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
