//! The first real EL0 program: does nothing but `exit(0)`, proving `exec::process::run_program`'s
//! whole round trip (ELF loaded, mapped, entered, and genuinely returned from) before anything
//! that reads/writes is attempted. No `userlib` yet -- just enough inline asm for one syscall.
#![no_std]
#![no_main]

/// Linux's real `exit` syscall number for aarch64 -- see the kernel's own `syscall.rs` for why
/// these numbers are borrowed (familiarity only, no Linux-ABI-compatibility claim beyond that).
const SYS_EXIT: usize = 93;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    // SAFETY: `x8` = syscall number, `svc #0` traps to the kernel's `sync_el0_64` -- the only
    // syscall this program ever makes, and the last thing it ever does.
    unsafe {
        core::arch::asm!(
            "mov x8, {nr}",
            "svc #0",
            nr = const SYS_EXIT,
            options(noreturn),
        );
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // Nothing here can actually panic (no allocation, no indexing, no unwrapping), so this is
    // unreachable in practice -- just needs to exist, and not call back into anything that could
    // itself panic.
    loop {
        unsafe { core::arch::asm!("wfe") }
    }
}
