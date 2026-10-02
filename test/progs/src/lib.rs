//! What the test programs share: the raw `svc`, which deliberately does not go through
//! `userlib`'s wrappers (they pass sensible arguments; these programs exist to pass bad ones), and
//! a way to print one `name result` line per call.
#![no_std]

extern crate alloc;

use alloc::format;
use core::arch::asm;

/// A system call made with exactly these registers: `x8` = `nr`, `x0`-`x3` = the arguments.
pub fn raw(nr: usize, a0: usize, a1: usize, a2: usize, a3: usize) -> isize {
    let ret: isize;
    // SAFETY: only issues the trap; what the kernel does with the arguments is what is under test.
    unsafe {
        asm!(
            "svc #0",
            in("x8") nr,
            inout("x0") a0 => ret,
            in("x1") a1,
            in("x2") a2,
            in("x3") a3,
        );
    }
    ret
}

/// Prints `name result` and a newline to stdout.
pub fn report(name: &str, result: isize) {
    let line = format!("{name} {result}\n");
    raw(abi::SYS_WRITE, 1, line.as_ptr() as usize, line.len(), 0);
}
