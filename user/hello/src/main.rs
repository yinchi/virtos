//! The first real EL0 program: writes a greeting and `exit(0)`s. Nothing more -- see
//! `plans/USERSPACE_PLAN.md`'s user program inventory (`hello`, `crash`: "nothing" needed beyond
//! `read`/`write`/`exit`). Kernel-side features get their own throwaway test in `user/scratch`
//! instead of being bolted onto this one -- `hello`'s behavior should stay exactly this stable.
//! No `userlib` yet -- just enough inline asm for one syscall.
#![no_std]
#![no_main]

/// Linux's real `write` syscall number for aarch64 -- see the kernel's own `syscall` module for
/// why these are borrowed (familiarity only, no Linux-ABI-compatibility claim beyond that).
const SYS_WRITE: usize = 64;
const SYS_EXIT: usize = 93;

const MESSAGE: &[u8] = b"hello from EL0!\n";

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    // SAFETY: fd 1 (console) and a pointer/length this binary's own `.text`/`.rodata` segment
    // owns -- exactly what `syscall::fd::write`'s bounds check and PAN guard expect.
    unsafe {
        core::arch::asm!(
            "mov x8, {nr}",
            "svc #0",
            nr = const SYS_WRITE,
            inout("x0") 1usize => _,
            in("x1") MESSAGE.as_ptr(),
            in("x2") MESSAGE.len(),
            clobber_abi("C"),
        );
    }
    // SAFETY: `x8` = syscall number, `svc #0` traps to the kernel's `sync_el0_64` -- the last
    // thing this program ever does.
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
