//! `crash` -- says so, then reads an address no program can: the demonstration of the fault path
//! (the kernel reports `Segmentation fault`, the shell sees status 139, and the next command runs
//! as usual). Takes no arguments besides `--help`.
#![no_std]
#![no_main]

use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "crash";
const FLAGS: &[(&str, &str)] = &[];

fn run(mut args: Args) -> ExitCode {
    if args.nth(1) == Some("--help") {
        return progs::help(USAGE, FLAGS);
    }
    userlib::write(1, b"about to crash\n");
    // A kernel-space-style address, so invalid for a more basic reason than "happens not to be
    // mapped". A real load the compiler cannot reason away.
    let bad = 0xffff_8000_0000_0000usize as *const u8;
    // SAFETY: deliberately not: the fault is the point.
    unsafe { core::ptr::read_volatile(bad) };
    // Not reached when the fault does what it should.
    ExitCode(0)
}
