//! Everything a program does with a file descriptor, plus the console: `open`, `close`, `read`,
//! `write`, `clear`. Each is a thin wrapper over one syscall (see `syscall.rs`) -- no stdout
//! buffering yet (every call is its own `write` syscall); revisit if that ever shows up as an
//! actual cost, not preemptively.

use abi::{SYS_CLEAR, SYS_CLOSE, SYS_OPEN, SYS_READ, SYS_WRITE};

/// `open`'s `flags` -- re-exported so a program writes `userlib::O_RDONLY`.
pub use abi::O_RDONLY;

/// Opens the file at `path` (read-only -- `flags` must be `O_RDONLY`, the only mode this project
/// supports so far). Returns the new file descriptor, or a negative value on error (see the
/// kernel's `syscall::fd::open` for what can fail and why).
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

/// Clears the console (and, via an ANSI escape sequence, whatever real terminal is watching the
/// UART). No fd argument -- there's only ever one console, unlike a real `ioctl(1, CONSOLE_CLEAR)`
/// this project has no generic mechanism for yet (`plans/USERSPACE_PLAN.md`'s divergence #5).
pub fn clear() -> isize {
    syscall!(SYS_CLEAR)
}
