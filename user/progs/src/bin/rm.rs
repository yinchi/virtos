//! `rm PATH...` -- removes each named file, or empty directory. No `-r` (recursive) or `-f`
//! (ignore missing operands) yet -- later additions if ever needed; a non-empty directory is a
//! reported error, not silently skipped or recursed into.
#![no_std]
#![no_main]

use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "rm PATH...";
const FLAGS: &[(&str, &str)] = &[];

fn run(args: Args) -> ExitCode {
    let plain = match progs::cli::plain("rm", USAGE, FLAGS, args) {
        Ok(plain) => plain,
        Err(status) => return status,
    };
    if plain.count == 0 {
        userlib::write(2, b"rm: missing operand\n");
        return ExitCode(1);
    }
    let mut ok = true;
    for path in progs::cli::operands(args) {
        if userlib::unlink(path) < 0 {
            userlib::write(2, b"rm: ");
            userlib::write(2, path.as_bytes());
            userlib::write(2, b": cannot remove\n");
            ok = false;
        }
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
