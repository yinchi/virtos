//! Prints its own arguments (not its own name -- `argv[0]` is skipped, as usual), separated by a
//! single space each, followed by a trailing newline. The first real program to consume `argv`
//! for something genuinely useful, not just to prove the mechanism works (`user/scratch`'s job).
//! No `userlib` yet (`plans/USERSPACE_PLAN.md`'s step 8) -- just enough inline asm for one syscall,
//! same as `user/hello`.
#![no_std]
#![no_main]

// Linux's real aarch64 syscall numbers -- see the kernel's own `syscall` module for why these are
// borrowed (familiarity only, no Linux-ABI-compatibility claim beyond that).
const SYS_WRITE: usize = 64;
const SYS_EXIT: usize = 93;

/// The length of the NUL-terminated string at `ptr`.
///
/// # Safety
/// `ptr` must point at a byte sequence that contains a `0` within mapped, readable memory.
unsafe fn strlen(ptr: *const u8) -> usize {
    let mut len = 0;
    // SAFETY: the caller guarantees a `0` exists somewhere in `ptr`'s mapped memory, so this
    // eventually stops; every byte visited before that is real, mapped memory by the same
    // guarantee.
    unsafe {
        while *ptr.add(len) != 0 {
            len += 1;
        }
    }
    len
}

/// Writes `len` bytes at `ptr` to `fd`.
///
/// # Safety
/// `ptr`/`len` must describe memory this program owns -- exactly what `syscall::fd::write`'s
/// bounds check and PAN guard expect.
unsafe fn write(fd: usize, ptr: *const u8, len: usize) {
    // SAFETY: the caller upholds `ptr`/`len`'s contract; `fd` is just an integer.
    unsafe {
        core::arch::asm!(
            "mov x8, {nr}",
            "svc #0",
            nr = const SYS_WRITE,
            inout("x0") fd => _,
            in("x1") ptr,
            in("x2") len,
            clobber_abi("C"),
        );
    }
}

/// # Safety
/// Only ever reached by the kernel's own `eret` into this program's entry point (`exec::process`),
/// which sets `x0`/`x1` to a real `argc` and an `argv` pointing to `argc` valid, NUL-terminated
/// strings before jumping here -- never called as an ordinary Rust function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start(argc: usize, argv: *const *const u8) -> ! {
    // Skip `argv[0]` (this program's own name) -- ordinary `echo` semantics.
    for i in 1..argc {
        if i > 1 {
            // SAFETY: a single `' '` byte, valid for the length of this function's own stack frame.
            unsafe { write(1, b" ".as_ptr(), 1) };
        }
        // SAFETY: `exec::process::run_program`'s caller writes `argc` pointers at `argv`, each one
        // pointing to a NUL-terminated string elsewhere on this program's own stack.
        let arg = unsafe { *argv.add(i) };
        // SAFETY: as documented on `strlen`, given the same guarantee as above.
        let len = unsafe { strlen(arg) };
        // SAFETY: `arg`/`len` describe one of the argument strings just read above.
        unsafe { write(1, arg, len) };
    }
    // SAFETY: a single `'\n'` byte, valid for the length of this function's own stack frame.
    unsafe { write(1, b"\n".as_ptr(), 1) };

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
    // Nothing here can actually panic (no allocation, no indexing beyond `argv`'s own contract, no
    // unwrapping), so this is unreachable in practice -- just needs to exist, and not call back
    // into anything that could itself panic.
    loop {
        unsafe { core::arch::asm!("wfe") }
    }
}
