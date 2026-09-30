//! Powers off the machine. Takes no arguments besides `--help`.
#![no_std]
#![no_main]

use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "poweroff";
const FLAGS: &[(&str, &str)] = &[];

fn run(mut args: Args) -> ExitCode {
    if args.nth(1) == Some("--help") {
        return progs::help(USAGE, FLAGS);
    }
    userlib::reboot(userlib::LINUX_REBOOT_CMD_POWER_OFF);
    ExitCode(0)
}
