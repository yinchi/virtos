//! The mechanism behind every syscall wrapper in this crate: the raw `svc` and the `syscall!`
//! macro that pads a call out to its full argument width. Nothing here is called by a program
//! directly -- `io`/`process` wrap it -- so both are crate-private.
//!
//! `x8` = syscall number, `x0`-`x5` = up to six arguments, `x0` = return value -- the calling
//! convention `plans/USERSPACE_PLAN.md`'s architecture section settles on. The kernel's
//! `kernel_entry`/`kernel_exit` trampoline saves and restores every GPR across the trap, so no
//! clobber list beyond `x0`'s own input/output role is needed here.

use core::arch::asm;

/// Issues `svc #0` with `nr` in `x8` and the full `x0`-`x5` argument width this project's calling
/// convention allows. Callers go through the `syscall!` macro below rather than this directly, so
/// they only ever write the arguments a given syscall actually uses.
#[inline(always)]
pub(crate) unsafe fn syscall(
    nr: usize,
    a0: usize,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
    a5: usize,
) -> isize {
    let ret: isize;
    // SAFETY: the caller upholds whatever `nr`'s own syscall expects of `a0`-`a5`; this function
    // only issues the trap.
    unsafe {
        asm!(
            "svc #0",
            in("x8") nr,
            inout("x0") a0 => ret,
            in("x1") a1,
            in("x2") a2,
            in("x3") a3,
            in("x4") a4,
            in("x5") a5,
        );
    }
    ret
}

/// Pads a call out to `syscall`'s full 6-argument form with trailing zeros, so callers only write
/// the arguments a given syscall actually uses -- `syscall!(SYS_EXIT, code)` instead of spelling
/// out `syscall(SYS_EXIT, code, 0, 0, 0, 0, 0)`. Also encapsulates the `unsafe` block: every use in
/// this crate is a plain scalar or a pointer already derived from a safe slice, with no further
/// safety obligation beyond what `syscall` itself documents.
///
/// A plain `macro_rules!`, so it is only visible to modules declared after this one (`lib.rs`
/// declares this module first, with `#[macro_use]`).
macro_rules! syscall {
    ($nr:expr) => {
        unsafe { $crate::syscall::syscall($nr, 0, 0, 0, 0, 0, 0) }
    };
    ($nr:expr, $a0:expr) => {
        unsafe { $crate::syscall::syscall($nr, $a0, 0, 0, 0, 0, 0) }
    };
    ($nr:expr, $a0:expr, $a1:expr) => {
        unsafe { $crate::syscall::syscall($nr, $a0, $a1, 0, 0, 0, 0) }
    };
    ($nr:expr, $a0:expr, $a1:expr, $a2:expr) => {
        unsafe { $crate::syscall::syscall($nr, $a0, $a1, $a2, 0, 0, 0) }
    };
    ($nr:expr, $a0:expr, $a1:expr, $a2:expr, $a3:expr) => {
        unsafe { $crate::syscall::syscall($nr, $a0, $a1, $a2, $a3, 0, 0) }
    };
    ($nr:expr, $a0:expr, $a1:expr, $a2:expr, $a3:expr, $a4:expr) => {
        unsafe { $crate::syscall::syscall($nr, $a0, $a1, $a2, $a3, $a4, 0) }
    };
    ($nr:expr, $a0:expr, $a1:expr, $a2:expr, $a3:expr, $a4:expr, $a5:expr) => {
        unsafe { $crate::syscall::syscall($nr, $a0, $a1, $a2, $a3, $a4, $a5) }
    };
}
