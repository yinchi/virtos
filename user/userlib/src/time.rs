//! The `clock_gettime` wrapper: reading the current time.

use abi::SYS_CLOCK_GETTIME;

/// The current Unix timestamp (seconds since 1970-01-01 00:00:00 UTC, always -- see the kernel's
/// `syscall::clock::time` for why there's no timezone to ask for). Returns `u32`: the PL031
/// register behind it is itself unsigned, so times before the Unix epoch are not represented.
pub fn time() -> u32 {
    syscall!(SYS_CLOCK_GETTIME) as u32
}

/// What `utimens` is to do with one timestamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeSet {
    /// The current time (`UTIME_NOW`).
    Now,
    /// Leave it as it is (`UTIME_OMIT`).
    Omit,
    /// This many seconds since 1970-01-01 00:00:00 UTC.
    At(i64),
}

impl TimeSet {
    /// The `(tv_sec, tv_nsec)` pair the syscall's `timespec` holds for this request.
    fn timespec(self) -> (i64, i64) {
        match self {
            TimeSet::Now => (0, abi::UTIME_NOW),
            TimeSet::Omit => (0, abi::UTIME_OMIT),
            TimeSet::At(seconds) => (seconds, 0),
        }
    }
}

/// Sets the access and modify times of the file or directory at `path`. FAT keeps no access time
/// beyond a date, so the kernel accepts `atime` and ignores it; the modify time is kept to 2
/// seconds and must lie in 1980-2107. Returns `0`, or a negative value on error (a missing path,
/// the root, a time FAT cannot hold).
pub fn utimens(path: &str, atime: TimeSet, mtime: TimeSet) -> isize {
    let mut raw = [0u8; abi::UTIMES_SIZE];
    for (slot, request) in [atime, mtime].into_iter().enumerate() {
        let (sec, nsec) = request.timespec();
        raw[slot * 16..slot * 16 + 8].copy_from_slice(&sec.to_le_bytes());
        raw[slot * 16 + 8..slot * 16 + 16].copy_from_slice(&nsec.to_le_bytes());
    }
    syscall!(
        abi::SYS_UTIMENSAT,
        path.as_ptr() as usize,
        path.len(),
        raw.as_ptr() as usize
    )
}
