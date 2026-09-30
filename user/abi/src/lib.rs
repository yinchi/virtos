//! Constants the kernel and every `user/*` binary must independently agree on, because nothing at
//! runtime tells either side the other's value -- there's no negotiation step, so both sides
//! simply hold the same literal.
#![no_std]

/// Syscall numbers. Linux's real aarch64 numbers -- borrowed for familiarity, no other Linux-ABI-
/// compatibility claim beyond that (the calling convention and argument order are this project's
/// own, not verified against Linux's).
pub const SYS_READ: usize = 63;
pub const SYS_WRITE: usize = 64;
pub const SYS_EXIT: usize = 93;
pub const SYS_REBOOT: usize = 142;

/// Not a real Linux syscall -- Linux clears a terminal through an `ioctl` a userspace tool
/// interprets itself (writing ANSI escapes), and this project has no generic `ioctl` mechanism yet
/// (`plans/USERSPACE_PLAN.md`'s divergence #5). A dedicated syscall is the smaller thing to build
/// for the one thing that needs it today; revisit if/when a real `ioctl` exists for other reasons
/// and this turns out to be one more request code on top of it, not a syscall of its own.
pub const SYS_CLEAR: usize = 1000;

/// `reboot`'s `cmd` values -- Linux's real magic numbers, borrowed for familiarity (no two-magic-
/// number safety dance like Linux's own `reboot(2)`; this project has no multi-user safety concern
/// that convention exists for).
pub const LINUX_REBOOT_CMD_RESTART: u32 = 0x0123_4567;
pub const LINUX_REBOOT_CMD_POWER_OFF: u32 = 0x4321_FEDC;

/// The fixed userspace heap's lowest address, and its size. No `brk` syscall exists to discover
/// this at runtime (`plans/USERSPACE_PLAN.md`'s divergence #6: a fixed heap needs no syscall
/// support, only a `brk`-style *growable* one would) -- the kernel (which maps this range before
/// entering a program) and `userlib` (which hands it straight to a global allocator) each need
/// their own copy of the same two numbers, so this is where that copy lives. Sized generously for
/// an ordinary utility program; see `arch::mmu`'s own layout for where this sits relative to the
/// image, guard and stack (a compile-time assertion there keeps the two in sync).
pub const USER_HEAP_BOTTOM: usize = 0x45DF_0000;
pub const USER_HEAP_SIZE: usize = 0x0010_0000; // 1 MiB
