//! `mkdir DIR...` -- creates each named directory. No flags yet (GNU's `-p`, creating missing
//! parents too, is a later addition if ever needed).
#![no_std]
#![no_main]

use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "mkdir DIR...";
const FLAGS: &[(&str, &str)] = &[];

fn run(args: Args) -> ExitCode {
    let mut any = false;
    let mut ok = true;
    for dir in args.skip(1) {
        if dir == "--help" {
            return progs::help(USAGE, FLAGS);
        }
        any = true;
        if userlib::mkdir(dir) < 0 {
            userlib::write(2, b"mkdir: ");
            userlib::write(2, dir.as_bytes());
            userlib::write(2, b": cannot create directory\n");
            ok = false;
        }
    }
    if !any {
        userlib::write(2, b"mkdir: missing operand\n");
        return ExitCode(1);
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
