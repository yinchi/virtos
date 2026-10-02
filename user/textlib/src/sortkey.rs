//! How `sort` compares two lines. By default lines compare **bytewise** (the C locale's order: `Z` before `a`, and a shorter
//! line before a longer one it is a prefix of). With `-n` a line's leading number is its key: optional blanks, an optional `-`,
//! digits and an optional `.` and fraction; a line with no number counts as 0. Numbers compare exactly (no floating point, so
//! any number of digits), and equal keys fall back to comparing the whole lines bytewise -- GNU's "last resort" -- unless the
//! caller asks for the key alone (`sort -u` treats lines with equal keys as duplicates). Pure `no_std`, so it is tested on
//! the host.

use core::cmp::Ordering;

/// A parsed number: its sign, its integer digits without leading zeros and its fraction digits without trailing ones.
struct Number<'a> {
    negative: bool,
    int: &'a [u8],
    frac: &'a [u8],
}

fn parse(line: &[u8]) -> Number<'_> {
    let mut i = 0;
    while i < line.len() && (line[i] == b' ' || line[i] == b'\t') {
        i += 1;
    }
    let negative = line.get(i) == Some(&b'-');
    if negative {
        i += 1;
    }
    let start = i;
    while i < line.len() && line[i].is_ascii_digit() {
        i += 1;
    }
    let mut int = &line[start..i];
    while let [b'0', rest @ ..] = int {
        int = rest;
    }
    let mut frac: &[u8] = &[];
    if line.get(i) == Some(&b'.') {
        let from = i + 1;
        let mut j = from;
        while j < line.len() && line[j].is_ascii_digit() {
            j += 1;
        }
        frac = &line[from..j];
        while let [rest @ .., b'0'] = frac {
            frac = rest;
        }
    }
    // Zero has no sign, so `-0` equals `0`.
    Number {
        negative: negative && !(int.is_empty() && frac.is_empty()),
        int,
        frac,
    }
}

fn magnitude(a: &Number, b: &Number) -> Ordering {
    a.int
        .len()
        .cmp(&b.int.len())
        .then_with(|| a.int.cmp(b.int))
        .then_with(|| a.frac.cmp(b.frac))
}

/// Compares two lines as `sort` does: numerically if `numeric`, else bytewise; with `last_resort`, lines whose keys are equal
/// then compare bytewise as whole lines.
pub fn compare(a: &[u8], b: &[u8], numeric: bool, last_resort: bool) -> Ordering {
    let key = if numeric {
        let (x, y) = (parse(a), parse(b));
        match (x.negative, y.negative) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => magnitude(&x, &y),
            (true, true) => magnitude(&y, &x),
        }
    } else {
        a.cmp(b)
    };
    if last_resort {
        key.then_with(|| a.cmp(b))
    } else {
        key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(a: &str, b: &str) -> Ordering {
        compare(a.as_bytes(), b.as_bytes(), true, false)
    }

    #[test]
    fn bytewise_by_default() {
        let cmp = |a: &str, b: &str| compare(a.as_bytes(), b.as_bytes(), false, true);
        assert_eq!(cmp("a", "b"), Ordering::Less);
        assert_eq!(cmp("Z", "a"), Ordering::Less); // the C locale
        assert_eq!(cmp("ab", "abc"), Ordering::Less); // a prefix is smaller
        assert_eq!(cmp("", "a"), Ordering::Less);
        assert_eq!(cmp("10", "9"), Ordering::Less); // as text
        assert_eq!(cmp("a", "a"), Ordering::Equal);
    }

    #[test]
    fn numbers_compare_as_numbers() {
        assert_eq!(num("9", "10"), Ordering::Less);
        assert_eq!(num("10", "9"), Ordering::Greater);
        assert_eq!(num("2", "2"), Ordering::Equal);
        assert_eq!(num("007", "7"), Ordering::Equal);
        assert_eq!(num("100", "99"), Ordering::Greater);
    }

    #[test]
    fn negatives_and_zero() {
        assert_eq!(num("-1", "1"), Ordering::Less);
        assert_eq!(num("-10", "-9"), Ordering::Less);
        assert_eq!(num("-2", "-1"), Ordering::Less);
        assert_eq!(num("-0", "0"), Ordering::Equal);
        assert_eq!(num("-1", "0"), Ordering::Less);
        assert_eq!(num("0", "-0.0"), Ordering::Equal);
    }

    #[test]
    fn fractions() {
        assert_eq!(num("1.5", "1.25"), Ordering::Greater);
        assert_eq!(num("1.10", "1.1"), Ordering::Equal);
        assert_eq!(num("0.5", "0.05"), Ordering::Greater);
        assert_eq!(num("-1.5", "-1.25"), Ordering::Less);
        assert_eq!(num(".5", "0.5"), Ordering::Equal);
        assert_eq!(num("2.", "2"), Ordering::Equal);
    }

    #[test]
    fn leading_blanks_are_skipped_and_the_rest_is_ignored() {
        assert_eq!(num("  5", "5"), Ordering::Equal);
        assert_eq!(num("5 apples", "5 pears"), Ordering::Equal);
        assert_eq!(num("\t-3x", "-3"), Ordering::Equal);
    }

    #[test]
    fn a_line_with_no_number_is_zero() {
        assert_eq!(num("abc", "0"), Ordering::Equal);
        assert_eq!(num("abc", "1"), Ordering::Less);
        assert_eq!(num("abc", "-1"), Ordering::Greater);
        assert_eq!(num("", "0"), Ordering::Equal);
        assert_eq!(num("-", "0"), Ordering::Equal);
    }

    #[test]
    fn any_number_of_digits() {
        let big = "99999999999999999999999999999999999999";
        let bigger = "100000000000000000000000000000000000000";
        assert_eq!(num(big, bigger), Ordering::Less);
    }

    #[test]
    fn equal_keys_fall_back_to_the_whole_line() {
        let with = |a: &str, b: &str| compare(a.as_bytes(), b.as_bytes(), true, true);
        assert_eq!(with("1 b", "1 a"), Ordering::Greater);
        assert_eq!(with("1", "01"), Ordering::Greater); // "1" > "01" as text
        assert_eq!(with("a", "b"), Ordering::Less); // both zero
        assert_eq!(num("1 b", "1 a"), Ordering::Equal); // without the last resort, equal
    }
}
