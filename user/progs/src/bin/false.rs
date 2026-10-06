//! `false` -- exits 1. Every argument besides `--help` is ignored, as the real one's.
#![no_std]
#![no_main]

use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "false";
const FLAGS: &[(&str, &str)] = &[];

fn run(mut args: Args) -> ExitCode {
    if args.nth(1) == Some("--help") {
        return progs::help(USAGE, FLAGS);
    }
    ExitCode(1)
}
