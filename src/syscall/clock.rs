//! The `clock_gettime` syscall: reads the current time off `arch::rtc`'s PL031.

use crate::arch::rtc::rtc;

/// The current Unix timestamp (seconds since 1970-01-01 00:00:00 UTC) -- a direct read of the
/// PL031's own counter, UTC always (there's no `$TZ` to convert with yet, and this project's one
/// timezone conversion so far, `arch::rtc::DEFAULT_TIMEZONE`, is a boot-time display choice, not
/// something a program should inherit implicitly). Returns `u32`, not this module's usual `isize`
/// -- there's no error this can produce (no path, no user pointer, just a register read, and the
/// RTC is initialized unconditionally in `kernel_main`, well before the shell can ever run), so
/// there's nothing for a negative `isize` to signal; `sync_el0_handler`'s `as u64` on the result
/// zero-extends this correctly regardless. `userlib::time`'s own doc comment says more on why a
/// signed return would have been the wrong call here specifically.
pub fn time() -> u32 {
    rtc().get_unix_timestamp()
}
