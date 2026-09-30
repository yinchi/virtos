//! `stat FILE...` -- prints each named path's size and type (regular file or directory). No
//! attributes or timestamps yet (`userlib::Stat`'s own doc comment) -- nothing built so far
//! (`chmod`, `-l`/`-t`) would have anything to show for them.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "stat FILE...";
const FLAGS: &[(&str, &str)] = &[];

fn print_one(path: &str, info: &userlib::Stat) {
    let kind = if info.is_dir {
        "directory"
    } else {
        "regular file"
    };
    userlib::write(1, format!("  File: {path}\n").as_bytes());
    userlib::write(1, format!("  Size: {}\n", info.size).as_bytes());
    userlib::write(1, format!("  Type: {kind}\n").as_bytes());
}

fn run(args: Args) -> ExitCode {
    let mut any = false;
    let mut ok = true;
    for path in args.skip(1) {
        if path == "--help" {
            return progs::help(USAGE, FLAGS);
        }
        any = true;
        match userlib::stat(path) {
            Ok(info) => print_one(path, &info),
            Err(_) => {
                userlib::write(2, b"stat: ");
                userlib::write(2, path.as_bytes());
                userlib::write(2, b": cannot stat\n");
                ok = false;
            }
        }
    }
    if !any {
        userlib::write(2, b"stat: missing operand\n");
        return ExitCode(1);
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
