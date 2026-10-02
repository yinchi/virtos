//! Unix time to the calendar fields a FAT directory entry stores, and `utimensat`'s timestamp
//! requests. Pure arithmetic (no clock, no time zones, no `chrono`): tested on the host.
//! `rtc_time.rs` turns the fields into `hadris_fat`'s `FatDateTime` and supplies them from the
//! real-time clock.
//!
//! Timestamps are stored in **UTC**. FAT has no time-zone field and Windows reads the fields as
//! local time, but the kernel never interprets them: it writes what the clock says (UTC) and
//! `stat` hands them back raw, so read and write agree, as they do on Linux with `mount -o
//! tz=UTC`. Converting to a user's zone is a display matter for the program that prints them
//! (`$TZ`).

/// The calendar fields of one FAT timestamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fields {
    /// 1980 to 2107, the range FAT can store.
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    /// 0 to 59. FAT stores 2-second steps, so an odd second is rounded down on disk.
    pub second: u8,
}

/// 1980-01-01 00:00:00, the FAT epoch and the earliest time it can hold.
pub const FAT_EPOCH: Fields = Fields {
    year: 1980,
    month: 1,
    day: 1,
    hour: 0,
    minute: 0,
    second: 0,
};

/// The last time FAT can hold (its date runs to 2107-12-31; its time to 23:59:58).
const FAT_LAST: Fields = Fields {
    year: 2107,
    month: 12,
    day: 31,
    hour: 23,
    minute: 59,
    second: 58,
};

/// 1980-01-01 00:00:00 UTC, and 2107-12-31 23:59:59 UTC, as seconds since 1970.
const FAT_FIRST_UNIX: i64 = 315_532_800;
const FAT_LAST_UNIX: i64 = 4_354_819_199;

/// The calendar date `days` days after 1970-01-01 (proleptic Gregorian): Howard Hinnant's
/// `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // day of era, 0..=146096
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // year of era, 0..=399
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year, March-based
    let mp = (5 * doy + 2) / 153; // month, March-based
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// The fields for `unix` seconds since 1970. A time before 1980 or after 2107 is clamped whole to
/// the nearest end of what FAT can hold (a 1970 clock, an unset RTC, becomes the FAT epoch, not
/// "1980 with 1970's month and day").
pub fn fields(unix: i64) -> Fields {
    if unix < FAT_FIRST_UNIX {
        return FAT_EPOCH;
    }
    if unix > FAT_LAST_UNIX {
        return FAT_LAST;
    }
    let (year, month, day) = civil_from_days(unix.div_euclid(86_400));
    let secs = unix.rem_euclid(86_400);
    Fields {
        year: year as u16,
        month,
        day,
        hour: (secs / 3600) as u8,
        minute: (secs % 3600 / 60) as u8,
        second: (secs % 60) as u8,
    }
}

/// The fields for `unix` if FAT can hold that time at all (1980-01-01 to 2107-12-31), else `None`
/// -- unlike `fields`, which clamps, because a program that asks for 1970 should be told no, not
/// given 1980.
pub fn checked_fields(unix: i64) -> Option<Fields> {
    (FAT_FIRST_UNIX..=FAT_LAST_UNIX)
        .contains(&unix)
        .then(|| fields(unix))
}

/// What `utimensat` is asking for one timestamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeChoice {
    /// The current time (`UTIME_NOW`).
    Now,
    /// Leave it (`UTIME_OMIT`).
    Omit,
    /// This many seconds since 1970 (UTC).
    At(i64),
}

/// Reads one `timespec` of `utimensat`'s array: `UTIME_NOW`/`UTIME_OMIT` in `tv_nsec` are the two
/// special requests, and otherwise `tv_nsec` must be a real fraction of a second (it is then
/// dropped: FAT has no finer time than 2 seconds). `None` for anything else.
pub fn choose(sec: i64, nsec: i64) -> Option<TimeChoice> {
    match nsec {
        abi::UTIME_NOW => Some(TimeChoice::Now),
        abi::UTIME_OMIT => Some(TimeChoice::Omit),
        0..=999_999_999 => Some(TimeChoice::At(sec)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Fields {
        Fields {
            year,
            month,
            day,
            hour,
            minute,
            second,
        }
    }

    #[test]
    fn the_two_special_nanosecond_values_and_real_fractions() {
        assert_eq!(choose(5, abi::UTIME_NOW), Some(TimeChoice::Now));
        assert_eq!(choose(5, abi::UTIME_OMIT), Some(TimeChoice::Omit));
        assert_eq!(
            choose(1_000_000_000, 0),
            Some(TimeChoice::At(1_000_000_000))
        );
        assert_eq!(choose(7, 999_999_999), Some(TimeChoice::At(7)));
        assert_eq!(choose(7, 1_000_000_000), None);
        assert_eq!(choose(7, -1), None);
        assert_eq!(choose(7, abi::UTIME_OMIT - 1), None);
    }

    #[test]
    fn only_times_fat_can_hold_are_accepted() {
        assert_eq!(checked_fields(315_532_800), Some(FAT_EPOCH)); // 1980-01-01
        assert_eq!(checked_fields(315_532_799), None); // a second before
        assert_eq!(checked_fields(0), None);
        assert_eq!(checked_fields(-1), None);
        assert!(checked_fields(4_354_819_199).is_some()); // 2107-12-31 23:59:59
        assert_eq!(checked_fields(4_354_819_200), None); // 2108-01-01
        assert_eq!(checked_fields(i64::MAX), None);
        assert_eq!(checked_fields(i64::MIN), None);
    }

    #[test]
    fn a_time_in_range_is_broken_down_as_utc() {
        assert_eq!(fields(1_000_000_000), at(2001, 9, 9, 1, 46, 40));
        assert_eq!(fields(1_709_164_800), at(2024, 2, 29, 0, 0, 0)); // a leap day
        assert_eq!(fields(4_107_542_400), at(2100, 3, 1, 0, 0, 0)); // 2100 is not a leap year
        assert_eq!(fields(951_782_400), at(2000, 2, 29, 0, 0, 0)); // 2000 is
        assert_eq!(fields(951_868_800), at(2000, 3, 1, 0, 0, 0));
    }

    #[test]
    fn the_first_and_last_moments_fat_can_hold() {
        assert_eq!(fields(315_532_800), FAT_EPOCH); // 1980-01-01 00:00:00 exactly
        assert_eq!(fields(315_532_799), FAT_EPOCH); // one second earlier clamps up to it
        assert_eq!(fields(4_354_819_199), at(2107, 12, 31, 23, 59, 59)); // FAT drops the odd second
        assert_eq!(fields(4_354_819_200), FAT_LAST); // 2108-01-01 clamps down
    }

    #[test]
    fn an_unset_clock_is_the_fat_epoch_whole() {
        assert_eq!(fields(0), FAT_EPOCH);
        assert_eq!(fields(15_000_000), FAT_EPOCH); // 1970-06-25: the whole date clamps, not just the year
        assert_eq!(fields(-1), FAT_EPOCH);
        assert_eq!(fields(i64::MIN), FAT_EPOCH);
    }

    #[test]
    fn a_time_too_large_to_represent_is_the_last_fat_moment() {
        assert_eq!(fields(i64::MAX), FAT_LAST);
    }

    #[test]
    fn every_day_in_the_range_maps_to_a_real_date() {
        // Walk the whole FAT range a day at a time: dates advance by exactly one day each step.
        let mut prev = fields(FAT_FIRST_UNIX);
        let mut unix = FAT_FIRST_UNIX + 86_400;
        while unix <= FAT_LAST_UNIX {
            let f = fields(unix);
            let days_in_month = match prev.month {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                4 | 6 | 9 | 11 => 30,
                _ if prev.year % 4 == 0 && (prev.year % 100 != 0 || prev.year % 400 == 0) => 29,
                _ => 28,
            };
            let expected = if prev.day < days_in_month {
                (prev.year, prev.month, prev.day + 1)
            } else if prev.month < 12 {
                (prev.year, prev.month + 1, 1)
            } else {
                (prev.year + 1, 1, 1)
            };
            assert_eq!((f.year, f.month, f.day), expected, "{unix}");
            prev = f;
            unix += 86_400;
        }
    }
}
