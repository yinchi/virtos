//! Everything a program does with a file descriptor, plus the console and the path-only (no fd)
//! `mkdir`/`unlink`/`rename`/`getcwd`/`stat`/`chmod`: `open`, `close`, `read`, `write`,
//! `getdents`, `mkdir`, `unlink`, `rename`, `getcwd`, `stat`, `chmod`, `clear`. Each is a thin
//! wrapper over one syscall (see `syscall.rs`), returning what it returns: a count or `0` on
//! success, a negated errno (`abi::errno`; `errmsg` words one) on failure. No stdout buffering yet
//! (every call is its own `write` syscall); revisit if that ever shows up as an actual cost, not
//! preemptively.

use abi::{
    SYS_CHMOD, SYS_CLEAR, SYS_CLOSE, SYS_GETCWD, SYS_GETDENTS, SYS_MKDIRAT, SYS_NEWFSTATAT,
    SYS_OPEN, SYS_READ, SYS_RENAMEAT, SYS_UNLINKAT, SYS_WRITE,
};

/// `open`'s `flags` -- re-exported so a program writes `userlib::O_RDONLY`/`userlib::O_WRONLY`/
/// `userlib::O_APPEND`.
pub use abi::{O_APPEND, O_RDONLY, O_WRONLY};

/// FAT attribute bits, as reported by `stat`/`getdents` and settable (`ATTR_READ_ONLY`/
/// `ATTR_EXEC` only) via `chmod` -- re-exported so a program writes `userlib::ATTR_EXEC`, etc.
pub use abi::{ATTR_DIRECTORY, ATTR_EXEC, ATTR_READ_ONLY};

/// `getdents`'s record layout -- re-exported so a program writes `userlib::DIRENT_SIZE`/
/// `userlib::NAME_MAX`.
pub use abi::{DIRENT_SIZE, NAME_MAX};

/// `getcwd`'s buffer-sizing convention -- re-exported so a program writes `userlib::PATH_MAX`.
pub use abi::PATH_MAX;

/// Opens the file or directory at `path` (`flags` is `O_RDONLY` or `O_WRONLY`). Returns the new
/// file descriptor, or a negated errno on error (see the kernel's `syscall::fd::open` for what
/// can fail and why).
pub fn open(path: &str, flags: usize) -> isize {
    syscall!(SYS_OPEN, path.as_ptr() as usize, path.len(), flags)
}

/// Closes file descriptor `fd`. Returns `0`, or a negated errno if `fd` wasn't open.
pub fn close(fd: usize) -> isize {
    syscall!(SYS_CLOSE, fd)
}

/// Writes `buf` to file descriptor `fd`. Returns the number of bytes written, or a negated errno
/// on error (see the kernel's `syscall::fd` for what can fail and why).
pub fn write(fd: usize, buf: &[u8]) -> isize {
    syscall!(SYS_WRITE, fd, buf.as_ptr() as usize, buf.len())
}

/// Reads into `buf` from file descriptor `fd`. Returns the number of bytes read (possibly `0`,
/// non-blocking -- see the kernel's `syscall::fd::read`), or a negated errno on error.
pub fn read(fd: usize, buf: &mut [u8]) -> isize {
    syscall!(SYS_READ, fd, buf.as_mut_ptr() as usize, buf.len())
}

/// Fills `buf` with as many whole `DIRENT_SIZE` records as fit from an fd opened on a directory,
/// picking up where the last call left off. Returns the number of bytes filled -- `0` once the
/// listing is exhausted -- or a negated errno on error. `buf` must hold at least one `DIRENT_SIZE`
/// record, or the call fails outright, not an empty listing.
pub fn getdents(fd: usize, buf: &mut [u8]) -> isize {
    syscall!(SYS_GETDENTS, fd, buf.as_mut_ptr() as usize, buf.len())
}

/// One decoded `getdents` record.
pub struct DirEnt<'a> {
    pub size: u32,
    pub attrs: u8,
    pub name: &'a str,
}

impl<'a> DirEnt<'a> {
    /// Decodes the record in `raw`. `None` if the name isn't valid UTF-8.
    pub fn parse(raw: &'a [u8; DIRENT_SIZE]) -> Option<Self> {
        let name_len = raw[5] as usize;
        Some(Self {
            size: u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]),
            attrs: raw[4],
            name: core::str::from_utf8(&raw[6..6 + name_len]).ok()?,
        })
    }
}

/// Creates an empty directory at `path`. Returns `0`, or a negated errno on error (a missing or
/// non-directory parent, or an existing entry already at that name).
pub fn mkdir(path: &str) -> isize {
    syscall!(SYS_MKDIRAT, path.as_ptr() as usize, path.len())
}

/// Removes the file, or empty directory, at `path` -- one call for both (see the kernel's
/// `syscall::fd::unlink`). Returns `0`, or a negated errno on error (a missing path, or a
/// non-empty directory).
pub fn unlink(path: &str) -> isize {
    syscall!(SYS_UNLINKAT, path.as_ptr() as usize, path.len())
}

/// Renames or moves the entry at `old` to `new`. Returns `0`, or a negated errno on error (either
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

/// Reads the working directory into `buf`. Returns its length, or a negated errno if `buf` is
/// too small.
pub fn getcwd(buf: &mut [u8]) -> isize {
    syscall!(SYS_GETCWD, buf.as_mut_ptr() as usize, buf.len())
}

/// Every field this project's FAT layer reports for a path, via `stat`: its size, attribute
/// byte, and the three timestamps FAT actually stores (packed date/time, FAT's own encoding --
/// see `abi::STAT_SIZE`'s own doc comment; unpacking them is left to whichever program displays
/// them). `is_dir` is derived from `attrs` (`attrs & ATTR_DIRECTORY != 0`), kept as its own field
/// since most callers only ever want this one bit, not the full byte.
pub struct Stat {
    pub size: u32,
    pub attrs: u8,
    pub is_dir: bool,
    pub created_date: u16,
    pub created_time: u16,
    pub created_time_tenth: u8,
    pub modified_date: u16,
    pub modified_time: u16,
    pub accessed_date: u16,
}

/// Reads `path`'s size, attributes and timestamps. `Err` carries the negated errno the syscall
/// returned.
pub fn stat(path: &str) -> Result<Stat, isize> {
    let mut buf = [0u8; abi::STAT_SIZE];
    let r = syscall!(
        SYS_NEWFSTATAT,
        path.as_ptr() as usize,
        path.len(),
        buf.as_mut_ptr() as usize
    );
    if r < 0 {
        return Err(r);
    }
    let attrs = buf[4];
    Ok(Stat {
        size: u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
        attrs,
        is_dir: attrs & abi::ATTR_DIRECTORY != 0,
        created_date: u16::from_le_bytes([buf[5], buf[6]]),
        created_time: u16::from_le_bytes([buf[7], buf[8]]),
        created_time_tenth: buf[9],
        modified_date: u16::from_le_bytes([buf[10], buf[11]]),
        modified_time: u16::from_le_bytes([buf[12], buf[13]]),
        accessed_date: u16::from_le_bytes([buf[14], buf[15]]),
    })
}

/// Sets `set`'s bits and clears `clear`'s bits of `path`'s attribute byte -- only
/// `ATTR_READ_ONLY`/`ATTR_EXEC` may be named (the kernel rejects anything else). Returns `0`, or
/// a negated errno on error.
pub fn chmod(path: &str, set: u8, clear: u8) -> isize {
    syscall!(
        SYS_CHMOD,
        path.as_ptr() as usize,
        path.len(),
        set as usize,
        clear as usize
    )
}

/// Clears the console (and, via an ANSI escape sequence, whatever real terminal is watching the
/// UART). No fd argument -- there's only ever one console, unlike a real `ioctl(1, CONSOLE_CLEAR)`
/// this project has no generic mechanism for yet (`plans/USERSPACE_PLAN.md`'s divergence #5).
pub fn clear() -> isize {
    syscall!(SYS_CLEAR)
}

/// Mounts the volume `source` names (`LABEL=name` or `UUID=XXXX-XXXX`) on the existing directory
/// `target`. Returns `0`, or a negated errno -- see `abi::SYS_MOUNT` for which.
pub fn mount(source: &str, target: &str) -> isize {
    syscall!(
        abi::SYS_MOUNT,
        source.as_ptr() as usize,
        source.len(),
        target.as_ptr() as usize,
        target.len()
    )
}

/// Unmounts the volume mounted at `target`. Returns `0`, or a negated errno -- see
/// `abi::SYS_UMOUNT2`.
pub fn umount(target: &str) -> isize {
    syscall!(abi::SYS_UMOUNT2, target.as_ptr() as usize, target.len())
}
