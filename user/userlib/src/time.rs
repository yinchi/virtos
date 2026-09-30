//! The `clock_gettime` wrapper: reading the current time.

use abi::SYS_CLOCK_GETTIME;

/// The current Unix timestamp (seconds since 1970-01-01 00:00:00 UTC, always -- see the kernel's
/// `syscall::clock::time` for why there's no timezone to ask for). Returns `u32`: the PL031
/// register behind it is itself unsigned, so times before the Unix epoch are not represented.
pub fn time() -> u32 {
    syscall!(SYS_CLOCK_GETTIME) as u32
}
