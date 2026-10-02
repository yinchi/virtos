//! Times as `touch` reads them, and as FAT stores them: `-d STRING`, `-t [[CC]YY]MMDDhhmm[.ss]`, and the packed date and
//! time of a directory entry. Pure (calendar arithmetic on `chrono`, no clock and no zone database), so it is tested on
//! the host. A wall-clock time comes back as a `NaiveDateTime`, for the caller to place in the local zone.

use chrono::{NaiveDate, NaiveDateTime};

/// What `-d` names: an instant, or a wall-clock time in whatever zone the caller says (`$TZ`).
#[derive(Debug, PartialEq, Eq)]
pub enum When {
    /// `@SECONDS`, or a time written with a trailing `Z` (UTC): seconds since 1970-01-01 00:00:00 UTC.
    Epoch(i64),
    /// `YYYY-MM-DD[ T]HH:MM[:SS]`, or a bare date (midnight): a wall-clock time.
    Local(NaiveDateTime),
}

fn digits(text: &str, len: usize) -> Option<u32> {
    (text.len() == len && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

fn make(
    year: u32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> Option<NaiveDateTime> {
    NaiveDate::from_ymd_opt(i32::try_from(year).ok()?, month, day)?
        .and_hms_opt(hour, minute, second)
}

/// `touch -d`'s operand: `@SECONDS` (negative allowed), `YYYY-MM-DD` (midnight), or `YYYY-MM-DD HH:MM[:SS]` with `T`
/// allowed in place of the space; a trailing `Z` makes the time UTC instead of local. `None` for anything else (GNU reads
/// free-form dates and relative times; this does not).
pub fn parse_date(text: &str) -> Option<When> {
    if let Some(seconds) = text.strip_prefix('@') {
        let plain = seconds.strip_prefix('+').unwrap_or(seconds);
        if plain.strip_prefix('-').unwrap_or(plain).is_empty() {
            return None;
        }
        return plain.parse().ok().map(When::Epoch);
    }
    let (text, utc) = match text.strip_suffix('Z') {
        Some(rest) => (rest, true),
        None => (text, false),
    };
    let (date, clock) = match text.split_once([' ', 'T']) {
        Some((date, clock)) => (date, Some(clock)),
        None => (text, None),
    };
    let mut d = date.split('-');
    let (year, month, day) = (
        digits(d.next()?, 4)?,
        digits(d.next()?, 2)?,
        digits(d.next()?, 2)?,
    );
    if d.next().is_some() {
        return None;
    }
    let (hour, minute, second) = match clock {
        None => (0, 0, 0),
        Some(clock) => {
            let mut c = clock.split(':');
            let (hour, minute) = (digits(c.next()?, 2)?, digits(c.next()?, 2)?);
            let second = match c.next() {
                Some(second) => digits(second, 2)?,
                None => 0,
            };
            if c.next().is_some() {
                return None;
            }
            (hour, minute, second)
        }
    };
    let wall = make(year, month, day, hour, minute, second)?;
    Some(if utc {
        When::Epoch(wall.and_utc().timestamp())
    } else {
        When::Local(wall)
    })
}

/// `touch -t`'s operand, POSIX's `[[CC]YY]MMDDhhmm[.ss]`: eight digits (this year -- `year`, which the caller says), ten
/// (a two-digit year: 69-99 are 19YY, 00-68 are 20YY) or twelve (a full year), then an optional `.ss`. A wall-clock time.
pub fn parse_stamp(text: &str, year: i32) -> Option<NaiveDateTime> {
    let (main, second) = match text.split_once('.') {
        Some((main, second)) => (main, digits(second, 2)?),
        None => (text, 0),
    };
    let (year, rest) = match main.len() {
        8 => (u32::try_from(year).ok()?, main),
        10 => {
            let yy = digits(&main[..2], 2)?;
            (if yy >= 69 { 1900 + yy } else { 2000 + yy }, &main[2..])
        }
        12 => (digits(&main[..4], 4)?, &main[4..]),
        _ => return None,
    };
    make(
        year,
        digits(&rest[0..2], 2)?,
        digits(&rest[2..4], 2)?,
        digits(&rest[4..6], 2)?,
        digits(&rest[6..8], 2)?,
        second,
    )
}

/// A FAT directory entry's packed date and time (date: bits 0-4 day, 5-8 month, 9-15 year since 1980; time: bits 0-4
/// seconds/2, 5-10 minutes, 11-15 hours) as seconds since 1970, the stamp being UTC (the kernel stores UTC).
/// `None` for fields that are not a real date, which FAT will store all the same.
pub fn fat_to_unix(date: u16, time: u16) -> Option<i64> {
    let (year, month, day) = (
        1980 + u32::from(date >> 9),
        u32::from((date >> 5) & 0x0f),
        u32::from(date & 0x1f),
    );
    let (hour, minute, second) = (
        u32::from(time >> 11),
        u32::from((time >> 5) & 0x3f),
        u32::from(time & 0x1f) * 2,
    );
    make(year, month, day, hour, minute, second).map(|t| t.and_utc().timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall(y: u32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> When {
        When::Local(make(y, mo, d, h, mi, s).unwrap())
    }

    #[test]
    fn at_seconds() {
        assert_eq!(parse_date("@1000000000"), Some(When::Epoch(1_000_000_000)));
        assert_eq!(parse_date("@-1"), Some(When::Epoch(-1)));
        assert_eq!(parse_date("@+5"), Some(When::Epoch(5)));
        assert_eq!(parse_date("@"), None);
        assert_eq!(parse_date("@-"), None);
        assert_eq!(parse_date("@12x"), None);
    }

    #[test]
    fn a_date_and_a_clock() {
        assert_eq!(parse_date("2024-05-01"), Some(wall(2024, 5, 1, 0, 0, 0)));
        assert_eq!(
            parse_date("2024-05-01 13:45"),
            Some(wall(2024, 5, 1, 13, 45, 0))
        );
        assert_eq!(
            parse_date("2024-05-01 13:45:30"),
            Some(wall(2024, 5, 1, 13, 45, 30))
        );
        assert_eq!(
            parse_date("2024-05-01T13:45:30"),
            Some(wall(2024, 5, 1, 13, 45, 30))
        );
    }

    #[test]
    fn a_trailing_z_is_utc() {
        assert_eq!(
            parse_date("2001-09-09T01:46:40Z"),
            Some(When::Epoch(1_000_000_000))
        );
        assert_eq!(parse_date("1970-01-01Z"), Some(When::Epoch(0)));
    }

    #[test]
    fn what_is_not_a_date() {
        for bad in [
            "",
            "yesterday",
            "2024-5-1",
            "2024-05-01 1:45",
            "2024-05-32",
            "2024-13-01",
            "2024-02-30",
            "2024-05-01 24:00",
            "2024-05-01 13:60",
            "2024-05-01 13:45:61",
            "2024-05-01 13",
            "2024-05-01 13:45:30:10",
            "2024-05-01-02",
            "24-05-01",
            "2024/05/01",
            "2024-05-01  13:45",
        ] {
            assert_eq!(parse_date(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_leap_day_only_in_a_leap_year() {
        assert!(parse_date("2024-02-29").is_some());
        assert_eq!(parse_date("2023-02-29"), None);
    }

    #[test]
    fn posix_stamps() {
        let at = |t| parse_stamp(t, 2026).map(|n| n.and_utc().timestamp());
        let w = |y, mo, d, h, mi, s| make(y, mo, d, h, mi, s).unwrap().and_utc().timestamp();
        assert_eq!(at("202405011345"), Some(w(2024, 5, 1, 13, 45, 0)));
        assert_eq!(at("202405011345.30"), Some(w(2024, 5, 1, 13, 45, 30)));
        assert_eq!(at("2405011345"), Some(w(2024, 5, 1, 13, 45, 0)));
        assert_eq!(at("7005011345"), Some(w(1970, 5, 1, 13, 45, 0))); // 69-99 are 19YY
        assert_eq!(at("6905011345"), Some(w(1969, 5, 1, 13, 45, 0)));
        assert_eq!(at("6805011345"), Some(w(2068, 5, 1, 13, 45, 0))); // 00-68 are 20YY
        assert_eq!(at("05011345"), Some(w(2026, 5, 1, 13, 45, 0))); // no year: this one
    }

    #[test]
    fn what_is_not_a_stamp() {
        for bad in [
            "",
            "1345",
            "0501134",
            "05011345.",
            "05011345.5",
            "05011345.60",
            "1305011345.",
            "2024050113455",
            "abcdefgh",
            "13011345",
            "05321345",
            "05012545",
            "05011360",
            "0501-1345",
        ] {
            assert_eq!(parse_stamp(bad, 2026), None, "{bad:?}");
        }
    }

    #[test]
    fn fat_fields_as_unix_time() {
        // 2001-09-09 01:46:40 UTC: date (21<<9 | 9<<5 | 9), time (1<<11 | 46<<5 | 20)
        assert_eq!(
            fat_to_unix((21 << 9) | (9 << 5) | 9, (1 << 11) | (46 << 5) | 20),
            Some(1_000_000_000)
        );
        assert_eq!(fat_to_unix((1 << 5) | 1, 0), Some(315_532_800)); // the FAT epoch
        assert_eq!(fat_to_unix((21 << 9) | (9 << 5) | 9, 0), Some(999_993_600)); // a date alone: midnight
        assert_eq!(fat_to_unix(0, 0), None); // month 0, day 0
        assert_eq!(fat_to_unix((1 << 5) | 1, 25 << 11), None); // hour 25
    }
}
