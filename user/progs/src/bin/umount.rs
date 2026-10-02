//! `umount TARGET`: unmounts the volume mounted at TARGET (`umount` syscall: the root, a mount
//! with another inside it, and a volume with open files or the working directory in it are
//! refused as busy).
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;

use abi::errno::{EINVAL, errmsg};
use progs::cli;
use userlib::{Args, ExitCode, umount};

userlib::entry_with_args!(run);

const USAGE: &str = "umount TARGET";
const FLAGS: &[(&str, &str)] = &[];

fn run(args: Args) -> ExitCode {
    let plain = match cli::plain("umount", USAGE, FLAGS, args) {
        Ok(plain) => plain,
        Err(status) => return status,
    };
    match plain.count {
        0 => cli::missing_operand("umount"),
        1 => {
            let target = plain.first.unwrap_or("");
            let result = umount(target);
            if result >= 0 {
                return ExitCode(0);
            }
            let why = if result == EINVAL {
                "not a mount point"
            } else {
                errmsg(result)
            };
            cli::report("umount", &format!("{target}: {why}"));
            ExitCode(1)
        }
        _ => cli::extra_operand("umount", plain.last.unwrap_or("")),
    }
}
