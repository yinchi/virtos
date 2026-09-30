//! Everything a program does with a file descriptor, plus the console and the path-only (no fd)
//! `mkdir`/`unlink`/`rename`: `open`, `close`, `read`, `write`, `getdents`, `mkdir`, `unlink`,
//! `rename`, `clear`. Each is a thin wrapper over one syscall (see `syscall.rs`) -- no stdout
//! buffering yet (every call is its own `write` syscall); revisit if that ever shows up as an
//! actual cost, not preemptively.

use abi::{
    SYS_CLEAR, SYS_CLOSE, SYS_GETDENTS, SYS_MKDIRAT, SYS_OPEN, SYS_READ, SYS_RENAMEAT,
    SYS_UNLINKAT, SYS_WRITE,
};

/// `open`'s `flags` -- re-exported so a program writes `userlib::O_RDONLY`/`userlib::O_WRONLY`.
pub use abi::{O_RDONLY, O_WRONLY};

/// `getdents`'s record layout -- re-exported so a program writes `userlib::DIRENT_SIZE`/
/// `userlib::NAME_MAX`.
pub use abi::{DIRENT_SIZE, NAME_MAX};

/// Opens the file or directory at `path` (`flags` is `O_RDONLY` or `O_WRONLY`). Returns the new
/// file descriptor, or a negative value on error (see the kernel's `syscall::fd::open` for what
/// can fail and why).
pub fn open(path: &str, flags: usize) -> isize {
    syscall!(SYS_OPEN, path.as_ptr() as usize, path.len(), flags)
}

/// Closes file descriptor `fd`. Returns `0`, or a negative value if `fd` wasn't open.
pub fn close(fd: usize) -> isize {
    syscall!(SYS_CLOSE, fd)
}

/// Writes `buf` to file descriptor `fd`. Returns the number of bytes written, or a negative value
/// on error (see the kernel's `syscall::fd` for what can fail and why).
pub fn write(fd: usize, buf: &[u8]) -> isize {
    syscall!(SYS_WRITE, fd, buf.as_ptr() as usize, buf.len())
}

/// Reads into `buf` from file descriptor `fd`. Returns the number of bytes read (possibly `0`,
/// non-blocking -- see the kernel's `syscall::fd::read`), or a negative value on error.
pub fn read(fd: usize, buf: &mut [u8]) -> isize {
    syscall!(SYS_READ, fd, buf.as_mut_ptr() as usize, buf.len())
}

/// Fills `buf` with as many whole `DIRENT_SIZE` records as fit from an fd opened on a directory,
/// picking up where the last call left off. Returns the number of bytes filled -- `0` once the
/// listing is exhausted -- or a negative value on error. `buf` must hold at least one `DIRENT_SIZE`
/// record, or the call fails outright, not an empty listing.
pub fn getdents(fd: usize, buf: &mut [u8]) -> isize {
    syscall!(SYS_GETDENTS, fd, buf.as_mut_ptr() as usize, buf.len())
}

/// One decoded `getdents` record.
pub struct DirEnt<'a> {
    pub name: &'a str,
}

impl<'a> DirEnt<'a> {
    /// Decodes the record in `raw`, which must be exactly `DIRENT_SIZE` bytes. `None` if the name
    /// isn't valid UTF-8.
    pub fn parse(raw: &'a [u8]) -> Option<Self> {
        let name_len = raw[0] as usize;
        Some(Self {
            name: core::str::from_utf8(&raw[1..1 + name_len]).ok()?,
        })
    }
}

/// Creates an empty directory at `path`. Returns `0`, or a negative value on error (a missing or
/// non-directory parent, or an existing entry already at that name).
pub fn mkdir(path: &str) -> isize {
    syscall!(SYS_MKDIRAT, path.as_ptr() as usize, path.len())
}

/// Removes the file, or empty directory, at `path` -- one call for both (see the kernel's
/// `syscall::fd::unlink`). Returns `0`, or a negative value on error (a missing path, or a
/// non-empty directory).
pub fn unlink(path: &str) -> isize {
    syscall!(SYS_UNLINKAT, path.as_ptr() as usize, path.len())
}

/// Renames or moves the entry at `old` to `new`. Returns `0`, or a negative value on error (either
/// path fails to resolve, or `new` already names something).
pub fn rename(old: &str, new: &str) -> isize {
    syscall!(
        SYS_RENAMEAT,
        old.as_ptr() as usize,
        old.len(),
        new.as_ptr() as usize,
        new.len()
    )
}

/// Clears the console (and, via an ANSI escape sequence, whatever real terminal is watching the
/// UART). No fd argument -- there's only ever one console, unlike a real `ioctl(1, CONSOLE_CLEAR)`
/// this project has no generic mechanism for yet (`plans/USERSPACE_PLAN.md`'s divergence #5).
pub fn clear() -> isize {
    syscall!(SYS_CLEAR)
}
