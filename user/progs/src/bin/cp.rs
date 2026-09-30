//! `cp src dst` -- copies `src`'s content into `dst`, creating it if missing or overwriting it if
//! present. Exactly one source and one destination, no flags -- GNU's fuller `cp [-r] SRC... DST`
//! is a later addition if ever needed, not this program's job yet. The first real consumer of
//! `open(O_WRONLY)`, proving that round trip the same way `cat` proved the read side.
#![no_std]
#![no_main]

use userlib::{Args, ExitCode, O_RDONLY, O_WRONLY};

userlib::entry_with_args!(run);

const USAGE: &str = "cp src dst";
const FLAGS: &[(&str, &str)] = &[];
const BUF_SIZE: usize = 512;

fn run(args: Args) -> ExitCode {
    let mut argv = args.skip(1);
    let (src, dst, extra) = (argv.next(), argv.next(), argv.next());
    if src == Some("--help") {
        return progs::help(USAGE, FLAGS);
    }
    let (Some(src), Some(dst)) = (src, dst) else {
        userlib::write(2, b"cp: usage: cp src dst\n");
        return ExitCode(1);
    };
    if extra.is_some() {
        userlib::write(2, b"cp: usage: cp src dst\n");
        return ExitCode(1);
    }
    if src == dst {
        userlib::write(2, b"cp: src and dst must differ\n");
        return ExitCode(1);
    }

    let input = userlib::open(src, O_RDONLY);
    if input < 0 {
        userlib::write(2, b"cp: ");
        userlib::write(2, src.as_bytes());
        userlib::write(2, b": cannot open\n");
        return ExitCode(1);
    }
    let input = input as usize;

    // Read the first chunk before opening (and so truncating) `dst`: a source that can't be read
    // at all -- a directory, say -- must not destroy an existing destination first.
    let mut buf = [0u8; BUF_SIZE];
    let first = userlib::read(input, &mut buf);
    if first < 0 {
        userlib::write(2, b"cp: ");
        userlib::write(2, src.as_bytes());
        userlib::write(2, b": cannot read\n");
        userlib::close(input);
        return ExitCode(1);
    }

    let output = userlib::open(dst, O_WRONLY);
    if output < 0 {
        userlib::write(2, b"cp: ");
        userlib::write(2, dst.as_bytes());
        userlib::write(2, b": cannot open\n");
        userlib::close(input);
        return ExitCode(1);
    }
    let output = output as usize;

    let mut ok = true;
    let mut n = first;
    while n > 0 {
        if userlib::write(output, &buf[..n as usize]) < 0 {
            userlib::write(2, b"cp: ");
            userlib::write(2, dst.as_bytes());
            userlib::write(2, b": write error\n");
            ok = false;
            break;
        }
        n = userlib::read(input, &mut buf);
        if n < 0 {
            userlib::write(2, b"cp: ");
            userlib::write(2, src.as_bytes());
            userlib::write(2, b": cannot read\n");
            ok = false;
            break;
        }
    }
    userlib::close(input);
    // Closing `output` is what commits its final size to disk -- a failure here is a failed copy.
    if userlib::close(output) < 0 {
        ok = false;
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
