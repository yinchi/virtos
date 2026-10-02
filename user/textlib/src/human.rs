//! `ls -h`'s sizes: `1.5K`, `234`, `12M` -- powers of 1024, rounded **up** (GNU's rule), one decimal place under 10 of a unit
//! and none from 10 up. Pure, so it is tested on the host.

use alloc::format;
use alloc::string::String;

/// `bytes` as `ls -h` writes it.
pub fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes}");
    }
    let mut unit = 1024u64;
    for suffix in ["K", "M", "G", "T"] {
        // Tenths of this unit, rounded up.
        let tenths = (bytes * 10).div_ceil(unit);
        if tenths < 100 {
            return format!("{}.{}{suffix}", tenths / 10, tenths % 10);
        }
        let whole = bytes.div_ceil(unit);
        if whole < 1024 || suffix == "T" {
            return format!("{whole}{suffix}");
        }
        unit *= 1024;
    }
    unreachable!("the loop returns at its last suffix")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_a_kilobyte_is_the_number() {
        assert_eq!(human_size(0), "0");
        assert_eq!(human_size(1), "1");
        assert_eq!(human_size(1023), "1023");
    }

    #[test]
    fn one_decimal_below_ten_units_rounded_up() {
        assert_eq!(human_size(1024), "1.0K");
        assert_eq!(human_size(1025), "1.1K");
        assert_eq!(human_size(1536), "1.5K");
        assert_eq!(human_size(9 * 1024), "9.0K");
        assert_eq!(human_size(9 * 1024 + 1), "9.1K");
        assert_eq!(human_size(10 * 1024 - 1), "10K"); // 9.999 rounds up to 10.0, printed whole
    }

    #[test]
    fn whole_units_from_ten_up_rounded_up() {
        assert_eq!(human_size(10 * 1024), "10K");
        assert_eq!(human_size(10 * 1024 + 1), "11K");
        assert_eq!(human_size(1000 * 1024), "1000K");
    }

    #[test]
    fn larger_units() {
        assert_eq!(human_size(1024 * 1024), "1.0M");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0M");
        assert_eq!(human_size(12 * 1024 * 1024), "12M");
        assert_eq!(human_size(1024 * 1024 * 1024), "1.0G");
        assert_eq!(human_size(u32::MAX as u64), "4.0G");
    }

    #[test]
    fn just_under_the_next_unit_moves_up() {
        assert_eq!(human_size(1024 * 1024 - 1), "1.0M"); // 1023.999K is not "1024K"
    }
}
