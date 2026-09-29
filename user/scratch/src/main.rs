//! Not a real program -- a throwaway EL0 test harness, rewritten freely to exercise whatever
//! kernel-side feature is being tested next, so `user/hello` and its future siblings never have to
//! be bent out of shape to prove something new works. Its behavior carries no stability contract;
//! check git history (or `git blame`) for what it last tested if that matters.
//!
//! Currently exercises: a non-blocking `read` (proving the syscall doesn't fault even though
//! nothing produces input yet), then a `write`, then `exit(0)`.
#![no_std]
#![no_main]

// Linux's real aarch64 syscall numbers -- see the kernel's own `syscall` module for why these are
// borrowed (familiarity only, no Linux-ABI-compatibility claim beyond that).
const SYS_READ: usize = 63;
const SYS_WRITE: usize = 64;
const SYS_EXIT: usize = 93;

const MESSAGE: &[u8] = b"scratch: read+write+exit all worked\n";

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    // A stack buffer, not `.bss` -- this binary's one segment is read+exec only (see `link.ld`),
    // so the stack `exec::process::run_program` already mapped read/write is the only writable
    // memory available. `syscall::fd::read`'s `Keyboard` case is non-blocking and the queue is
    // empty (nothing has typed anything), so this returns 0 immediately without touching `buf`.
    let mut buf = [0u8; 8];
    // SAFETY: fd 0 (keyboard) and a pointer/length inside this program's own stack -- exactly
    // what `syscall::fd::read`'s bounds check and PAN guard expect.
    unsafe {
        core::arch::asm!(
            "mov x8, {nr}",
            "svc #0",
            nr = const SYS_READ,
            inout("x0") 0usize => _,
            in("x1") buf.as_mut_ptr(),
            in("x2") buf.len(),
            clobber_abi("C"),
        );
    }

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
