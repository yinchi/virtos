//! Faults on purpose, the way its first argument says: `null` writes to address 0, `jump` runs
//! code at address 0, `kernel` reads kernel memory, `below` reads the byte just under the program's own memory.
//! The kernel is expected to report a segmentation fault and give the shell status 139, and to
//! run the next command as if nothing had happened.
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

fn run(mut args: Args) -> ExitCode {
    match args.nth(1) {
        Some("null") => unsafe { write_volatile(core::ptr::null_mut::<u8>(), 1) },
        Some("jump") => unsafe { core::arch::asm!("br {0}", in(reg) 0usize, options(noreturn)) },
        Some("kernel") => {
            let _ = unsafe { read_volatile(0x4000_0000 as *const u8) };
        }
        Some("below") => {
            let _ = unsafe { read_volatile(0x4400_0000usize.wrapping_sub(8) as *const u8) };
        }
        _ => return ExitCode(2),
    }
    // Not reached when the access faults.
    userlib::write(1, b"survived\n");
    ExitCode(0)
}
