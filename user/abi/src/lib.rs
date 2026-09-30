//! Constants the kernel and every `user/*` binary must independently agree on, because nothing at
//! runtime tells either side the other's value -- there's no negotiation step, so both sides
//! simply hold the same literal.
#![no_std]

/// Syscall numbers. Linux's real aarch64 numbers -- borrowed for familiarity, no other Linux-ABI-
/// compatibility claim beyond that (the calling convention and argument order are this project's
/// own, not verified against Linux's).
pub const SYS_GETCWD: usize = 17;
pub const SYS_MKDIRAT: usize = 34; // Linux's `mkdirat`, but plain-path: no dirfd, no mode
pub const SYS_UNLINKAT: usize = 35; // Linux's `unlinkat`, but plain-path and no `AT_REMOVEDIR`
pub const SYS_RENAMEAT: usize = 38; // Linux's `renameat`, but plain-path on both sides
pub const SYS_NEWFSTATAT: usize = 79; // Linux's `newfstatat`, but plain-path: no dirfd, no flags
pub const SYS_OPEN: usize = 56; // Linux's `openat`, but plain-path: no dirfd, no relative-path base
pub const SYS_CLOSE: usize = 57;
pub const SYS_GETDENTS: usize = 61; // Linux's `getdents64`
pub const SYS_READ: usize = 63;
pub const SYS_WRITE: usize = 64;
pub const SYS_EXIT: usize = 93;
pub const SYS_CLOCK_GETTIME: usize = 113; // Linux's `clock_gettime`, but no clockid argument
// (there's only one clock) and no `timespec` out-param (the RTC has second resolution, nothing to
// put in `tv_nsec`) -- the Unix timestamp itself comes back directly in `x0`.
pub const SYS_REBOOT: usize = 142;

/// Longest absolute path this project deals with (`getcwd`'s buffer, `pwd`'s/`stat`'s own stack
/// buffers) -- an arbitrary but generous bound, not a number `hadris-fat`/FAT itself imposes.
pub const PATH_MAX: usize = 4096;

/// Size of the fixed payload `stat` writes: `size: u32` (little-endian), then `is_dir: u8`. No
/// attributes or timestamps yet -- nothing reads them for anything currently built (no `chmod` to
/// report, no `-l`/`-t` flag anywhere that would display a timestamp); grow this the same way
/// `getdents`'s own record would, once something actually needs one of those fields.
pub const STAT_SIZE: usize = 4 + 1;

// `open`'s `flags` -- Linux's real values. `O_RDONLY` also opens a directory, for `getdents`.
// `O_WRONLY` creates the file if it doesn't exist, and always starts it empty (no append mode
// yet -- nothing needs one until `>>`/`tee -a` do).
pub const O_RDONLY: usize = 0;
pub const O_WRONLY: usize = 1;

/// Longest name one directory entry can carry -- FAT's own long-filename limit, not a number this
/// project picked itself.
pub const NAME_MAX: usize = 255;

/// Size of one record `getdents` fills in: a `name_len: u8`, then `NAME_MAX` bytes of name,
/// NUL-padded. No size or attribute fields yet -- nothing reads a directory listing for anything
/// but names so far (`ls` takes no flags yet); grow this the same way `-n` was added to `head`/
/// `tail`, once a flag that actually needs one (`ls -l`'s size, `-F`'s directory bit) is being
/// built, not ahead of that.
pub const DIRENT_SIZE: usize = 1 + NAME_MAX;

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
