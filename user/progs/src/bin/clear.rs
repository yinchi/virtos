//! Clears the console. Takes no arguments besides `--help`.
#![no_std]
#![no_main]

use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "clear";
const FLAGS: &[(&str, &str)] = &[];

fn run(mut args: Args) -> ExitCode {
    if args.nth(1) == Some("--help") {
        return progs::help(USAGE, FLAGS);
    }
    userlib::clear();
    ExitCode(0)
}
