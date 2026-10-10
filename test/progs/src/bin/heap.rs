//! Exercises `brk` and the user heap on top of it. With no argument it prints one `name result`
//! line per check (`test/cases/heap.py` compares them); `dirty` grows the heap, fills its first
//! page with 0xAA and prints its address; `peek ADDR` reads one byte there (a segmentation fault
//! if nothing is mapped); `clean` grows the heap and reports whether its first page is zero.
#![no_std]
#![no_main]

extern crate alloc;

use abi::SYS_BRK;
use alloc::vec::Vec;
use core::ptr::{read_volatile, write_volatile};
use testprogs::{raw, report};
use userlib::{Args, ExitCode, write};

userlib::entry_with_args!(run);

const PAGE: usize = 4096;
/// One past the highest address the heap may reach (`arch::mmu::USER_IMAGE_END`).
const CEILING: usize = 0x45EF_0000;

fn brk(addr: usize) -> usize {
    raw(SYS_BRK, addr, 0, 0, 0) as usize
}

fn page_is_zero(addr: usize) -> bool {
    (0..PAGE).all(|i| unsafe { read_volatile((addr + i) as *const u8) } == 0)
}

fn run(mut args: Args) -> ExitCode {
    match args.nth(1) {
        Some("dirty") => {
            let start = brk(0);
            brk(start + PAGE);
            for i in 0..PAGE {
                unsafe { write_volatile((start + i) as *mut u8, 0xAA) };
            }
            let line = alloc::format!("{start}\n");
            write(1, line.as_bytes());
            return ExitCode(0);
        }
        Some("peek") => {
            let addr: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);
            let _ = unsafe { read_volatile(addr as *const u8) };
            write(1, b"survived\n");
            return ExitCode(0);
        }
        Some("clean") => {
            // The heap starts where the image ends, at the same address `dirty` filled.
            let start = brk(0);
            brk(start + PAGE);
            let line = alloc::format!("{}\n", page_is_zero(start));
            write(1, line.as_bytes());
            return ExitCode(0);
        }
        _ => {}
    }

    // The allocator owns whatever it has grown the break to (this program formats strings), so
    // make it take its first chunk now and work above it. Later small allocations are served from
    // that chunk and leave the break alone.
    drop(core::hint::black_box(Vec::<u8>::with_capacity(1)));
    let start = brk(0);
    report(
        "break-is-page-aligned-or-at-least-above-zero",
        (start != 0) as isize,
    );
    report("brk-zero-is-stable", (brk(0) == start) as isize);

    // Growing: by a page and one byte maps two pages; the break is byte-exact.
    let target = start + PAGE + 1;
    report(
        "grow-by-a-page-and-a-byte-returns-the-request",
        (brk(target) == target) as isize,
    );
    report("the-first-new-page-is-zero", page_is_zero(start) as isize);
    report(
        "the-second-new-page-is-zero",
        page_is_zero(start + PAGE) as isize,
    );
    unsafe { write_volatile((start + PAGE) as *mut u8, 7) };
    report(
        "the-last-mapped-byte-is-writable",
        unsafe { read_volatile((start + PAGE) as *const u8) } as isize,
    );

    // Shrinking back to mid-page zeroes the rest of that page; growing again reads zero.
    unsafe { write_volatile((start + 100) as *mut u8, 9) };
    unsafe { write_volatile((start + 200) as *mut u8, 9) };
    report(
        "shrink-returns-the-request",
        (brk(start + 150) == start + 150) as isize,
    );
    report(
        "regrow-returns-the-request",
        (brk(target) == target) as isize,
    );
    report(
        "regrown-memory-above-the-shrink-is-zero",
        (unsafe { read_volatile((start + 200) as *const u8) } == 0) as isize,
    );
    report(
        "regrown-second-page-is-zero",
        page_is_zero(start + PAGE) as isize,
    );
    report(
        "memory-below-the-shrink-survived",
        unsafe { read_volatile((start + 100) as *const u8) } as isize,
    );

    // Refusals leave the break where it was.
    let now = brk(0);
    report(
        "below-the-heap-is-refused",
        (brk(0x4400_0000) == now) as isize,
    );
    report(
        "past-the-ceiling-is-refused",
        (brk(CEILING + 1) == now) as isize,
    );
    report(
        "exactly-the-ceiling-is-allowed",
        (brk(CEILING) == CEILING) as isize,
    );
    report("back-down-after-the-ceiling", (brk(now) == now) as isize);

    // A pointer into the unmapped part of the window (just above the break) is EFAULT, not a
    // kernel fault.
    let hole = brk(0) + PAGE;
    report(
        "syscall-pointer-into-an-unmapped-hole",
        raw(abi::SYS_WRITE, 1, hole, 4, 0),
    );

    // The kernel may not write into the program's own (read-only) code either.
    report(
        "syscall-write-into-read-only-code",
        raw(abi::SYS_READ, 0, run as *const () as usize, 4, 0),
    );

    // The allocator on top: several MiB, touched end to end, then reused after a free.
    let mut big: Vec<u8> = Vec::new();
    big.resize(8 * 1024 * 1024, 0x5A);
    report(
        "an-8-mib-vec-is-intact",
        big.iter().all(|&b| b == 0x5A) as isize,
    );
    drop(big);
    let mut again: Vec<u8> = Vec::new();
    again.resize(8 * 1024 * 1024, 1);
    report(
        "the-freed-space-is-reusable",
        (again.len() == 8 * 1024 * 1024) as isize,
    );
    ExitCode(0)
}
