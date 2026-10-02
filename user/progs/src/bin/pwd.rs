//! `pwd` -- prints the working directory. POSIX's `-L`/`-P` (logical vs. physical path) are
//! meaningless here -- there are no symbolic links to tell them apart -- so, unlike GNU's `pwd`,
//! they're just not recognized at all rather than accepted and ignored.
#![no_std]
#![no_main]

use userlib::{Args, ExitCode, PATH_MAX};

userlib::entry_with_args!(run);

const USAGE: &str = "pwd";
const FLAGS: &[(&str, &str)] = &[];

fn run(mut args: Args) -> ExitCode {
    if args.nth(1) == Some("--help") {
        return progs::help(USAGE, FLAGS);
    }
    let mut buf = [0u8; PATH_MAX];
    let len = userlib::getcwd(&mut buf);
    if len < 0 {
        progs::cli::report_errno("pwd", "cannot get the working directory", len);
        return ExitCode(1);
    }
    userlib::write(1, &buf[..len as usize]);
    userlib::write(1, b"\n");
    ExitCode(0)
}
