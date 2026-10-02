//! `rmdir DIR...`: removes empty directories, the safe counterpart of `rm -r`. Every operand is
//! tried; the status is 1 if any failed. An operand that is not a directory is refused up front
//! (`unlink` alone would remove a plain file), and a directory with anything in it fails in the
//! kernel's `unlink` (the filesystem refuses to delete a non-empty one).
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use progs::cli;
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "rmdir DIR...";
const FLAGS: &[(&str, &str)] = &[];

fn run(args: Args) -> ExitCode {
    let plain = match cli::plain("rmdir", USAGE, FLAGS, args) {
        Ok(plain) => plain,
        Err(status) => return status,
    };
    if plain.count == 0 {
        return cli::missing_operand("rmdir");
    }
    let mut status = 0;
    for dir in cli::operands(args) {
        let why = match userlib::stat(dir) {
            Err(_) => Some("cannot open"),
            Ok(info) if !info.is_dir => Some("not a directory"),
            Ok(_) if userlib::unlink(dir) < 0 => Some("cannot remove (not empty?)"),
            Ok(_) => None,
        };
        if let Some(why) = why {
            cli::report("rmdir", &format!("{dir}: {why}"));
            status = 1;
        }
    }
    ExitCode(status)
}
