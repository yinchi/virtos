//! `poweroff [--reboot]` -- powers off the machine, or with `--reboot` restarts it (the same as
//! the separate `reboot` program), through the kernel's `reboot` syscall.
#![no_std]
#![no_main]

use getargs::Arg;
use progs::cli;
use userlib::{Args, ExitCode, LINUX_REBOOT_CMD_POWER_OFF, LINUX_REBOOT_CMD_RESTART};

userlib::entry_with_args!(run);

const USAGE: &str = "poweroff [--reboot]";
const FLAGS: &[(&str, &str)] = &[("--reboot", "restart instead of powering off")];

fn run(args: Args) -> ExitCode {
    cli::status(act(args))
}

fn act(args: Args) -> Result<ExitCode, ExitCode> {
    let mut command = (LINUX_REBOOT_CMD_POWER_OFF, "cannot power off");
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("poweroff", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(progs::help(USAGE, FLAGS)),
            Arg::Long("reboot") => command = (LINUX_REBOOT_CMD_RESTART, "cannot restart"),
            other => return Err(cli::invalid("poweroff", other)),
        }
    }
    // Only reached if the kernel refused a command this program always passes correctly.
    let err = userlib::reboot(command.0);
    cli::report_errno("poweroff", command.1, err);
    Ok(ExitCode(1))
}
