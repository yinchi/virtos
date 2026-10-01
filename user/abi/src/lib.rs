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
pub const SYS_CHMOD: usize = 53; // Linux's `fchmodat`, but plain-path: no dirfd, no flags

/// Longest absolute path this project deals with (`getcwd`'s buffer, `pwd`'s/`stat`'s own stack
/// buffers) -- an arbitrary but generous bound, not a number `hadris-fat`/FAT itself imposes.
pub const PATH_MAX: usize = 4096;

/// Size of the fixed payload `stat` writes: `size: u32` (LE), `attrs: u8`, `created_date: u16`
/// (LE), `created_time: u16` (LE), `created_time_tenth: u8`, `modified_date: u16` (LE),
/// `modified_time: u16` (LE), `accessed_date: u16` (LE) -- every field a FAT directory entry
/// actually stores. Dates/times are FAT's own packed encoding, not calendar values -- unpacking
/// them is left to whichever program displays them (`date`'s own `chrono` conversion is unrelated;
/// FAT's packed format predates it and has nothing to do with Unix time).
pub const STAT_SIZE: usize = 16;

// `open`'s `flags` -- Linux's real values. `O_RDONLY` also opens a directory, for `getdents`.
// `O_WRONLY` creates the file if it doesn't exist, and always starts it empty unless combined with
// `O_APPEND` (`O_WRONLY | O_APPEND`), which instead starts writing at the file's current end --
// creating it fresh too, if it didn't already exist, since there's nothing yet to append to.
pub const O_RDONLY: usize = 0;
pub const O_WRONLY: usize = 1;
pub const O_APPEND: usize = 0o2000; // Linux's real value, kept clear of O_RDONLY/O_WRONLY's own bit

/// Longest name one directory entry can carry -- FAT's own long-filename limit, not a number this
/// project picked itself.
pub const NAME_MAX: usize = 255;

/// Size of one record `getdents` fills in: `size: u32` (LE), `attrs: u8`, `name_len: u8`, then
/// `NAME_MAX` bytes of name, NUL-padded.
pub const DIRENT_SIZE: usize = 4 + 1 + 1 + NAME_MAX;

// --- FAT attribute bits ---------------------------------------------------------------------
// The bits `stat`/`chmod`/`getdents` all deal in, directly off (or onto) a FAT directory entry's
// own attribute byte -- `chmod` may only set/clear `ATTR_READ_ONLY`/`ATTR_EXEC`, the other two
// name the kind of entry on disk rather than a permission.

/// FAT attribute: read-only.
pub const ATTR_READ_ONLY: u8 = 0x01;
/// FAT attribute: directory.
pub const ATTR_DIRECTORY: u8 = 0x10;
/// FAT attribute: executable. Not standard in FAT -- reuses a reserved bit (no existing meaning),
/// this project's own convention (already set at build time by `tools/mkimage` on everything
/// under `usr/bin/`).
pub const ATTR_EXEC: u8 = 0x40;

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards the literal values nothing else cross-checks at compile time -- kernel and
    /// `userlib` each hold their own copy of what these mean (`fd.rs`'s `stat`/`chmod`/`getdents`,
    /// `io.rs`'s matching decode), so a typo here would silently desync the two.
    #[test]
    fn values_match_what_fd_rs_and_io_rs_assume() {
        assert_eq!(SYS_CHMOD, 53);
        assert_eq!(O_APPEND, 0o2000);
        assert_eq!(STAT_SIZE, 16);
        assert_eq!(DIRENT_SIZE, 4 + 1 + 1 + NAME_MAX);
        assert_eq!(
            [ATTR_READ_ONLY, ATTR_DIRECTORY, ATTR_EXEC],
            [0x01, 0x10, 0x40]
        );
    }
}
