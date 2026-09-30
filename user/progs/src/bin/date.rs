//! `date` -- prints the current time, UTC always (the only zone there is without `$TZ`, which
//! needs environment variables this project doesn't have yet -- `plans/USERSPACE_PLAN.md`'s step
//! 11). No flags yet: GNU's `-u`/`-d`/`-I`/`-R`/`+FORMAT` are later additions if ever needed, not
//! this pass's job -- just enough to prove the new `clock_gettime` syscall end to end.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use chrono::{DateTime, Utc};
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "date";
const FLAGS: &[(&str, &str)] = &[];

/// GNU's classic default layout in the C locale, minus the real `%Z` (there's only ever one zone
/// here, so it's spelled out literally rather than asking `chrono` to look one up).
const DEFAULT_FORMAT: &str = "%a %b %e %H:%M:%S UTC %Y";

fn run(mut args: Args) -> ExitCode {
    if args.nth(1) == Some("--help") {
        return progs::help(USAGE, FLAGS);
    }
    // `userlib::time()` can't fail (its own doc comment says why), so the only thing left to
    // check is `from_timestamp`'s own `Option` -- moot for any real `u32` value today, but it's
    // the general contract `chrono` gives back, not one this program should just assume holds.
    let Some(instant) = DateTime::<Utc>::from_timestamp(i64::from(userlib::time()), 0) else {
        userlib::write(2, b"date: time out of range\n");
        return ExitCode(1);
    };
    userlib::write(
        1,
        format!("{}\n", instant.format(DEFAULT_FORMAT)).as_bytes(),
    );
    ExitCode(0)
}
