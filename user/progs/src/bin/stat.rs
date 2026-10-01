//! `stat FILE...` -- prints each named path's size, type, FAT attributes (read-only, executable),
//! and its `Create`/`Modify` timestamps. No display-format flag (`-c`/`--format`) yet -- nothing
//! needs one until something wants a machine-readable subset of this rather than the fixed
//! layout below.
//!
//! The accessed date is not shown: nothing in this project's own `hadris_fat` layer updates it
//! on a read (Linux's `noatime`), and it's set only at creation/write time, to that same
//! operation's own date -- so it would always just repeat `Modify`'s date, not say anything new.
//! `userlib::Stat::accessed_date` still carries it, for a later `relatime`-style update or an
//! `ls -lu`.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use userlib::{ATTR_DIRECTORY, ATTR_EXEC, ATTR_READ_ONLY, Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "stat FILE...";
const FLAGS: &[(&str, &str)] = &[];

/// One FAT packed date (bits 0-4 day, 5-8 month, 9-15 year-since-1980) as `(year, month, day)`.
fn split_date(date: u16) -> (u16, u16, u16) {
    (1980 + (date >> 9), (date >> 5) & 0x0f, date & 0x1f)
}

/// One FAT packed time (bits 0-4 seconds/2, 5-10 minutes, 11-15 hours) as `(hour, minute, second)`.
fn split_time(time: u16) -> (u16, u16, u16) {
    (time >> 11, (time >> 5) & 0x3f, (time & 0x1f) * 2)
}

fn print_one(path: &str, info: &userlib::Stat) {
    let kind = if info.attrs & ATTR_DIRECTORY != 0 {
        "directory"
    } else {
        "regular file"
    };
    userlib::write(1, format!("  File: {path}\n").as_bytes());
    userlib::write(
        1,
        format!("  Size: {:<12} Type: {kind}\n", info.size).as_bytes(),
    );
    userlib::write(
        1,
        format!(
            " Attrs: read-only={}  exec={}\n",
            if info.attrs & ATTR_READ_ONLY != 0 {
                "yes"
            } else {
                "no"
            },
            if info.attrs & ATTR_EXEC != 0 {
                "yes"
            } else {
                "no"
            }
        )
        .as_bytes(),
    );
    let (y, mo, d) = split_date(info.modified_date);
    let (h, mi, s) = split_time(info.modified_time);
    userlib::write(
        1,
        format!("Modify: {y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}\n").as_bytes(),
    );
    let (y, mo, d) = split_date(info.created_date);
    let (h, mi, s) = split_time(info.created_time);
    userlib::write(
        1,
        format!("Create: {y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}\n").as_bytes(),
    );
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
