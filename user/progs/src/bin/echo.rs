//! Prints its own arguments (not its own name -- `argv[0]` is skipped, as usual), separated by a
//! single space each, followed by a trailing newline. Builds the line as an ordinary `String`
//! rather than writing each argument as its own syscall.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "echo [args...]";
const FLAGS: &[(&str, &str)] = &[];

fn run(args: Args) -> ExitCode {
    let mut line = String::new();
    for (i, arg) in args.skip(1).enumerate() {
        // `--help` is only recognized as the very first argument -- once real output has
        // started, later text is just data to echo, the same as a real `echo`.
        if i == 0 && arg == "--help" {
            return progs::help(USAGE, FLAGS);
        }
        if i > 0 {
            line.push(' ');
        }
        line.push_str(arg);
    }
    line.push('\n');
    userlib::write(1, line.as_bytes());
    ExitCode(0)
}
