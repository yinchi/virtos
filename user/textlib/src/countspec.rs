//! The count operand of `head` and `tail` (`-n` and `-c`): a number, with an optional sign that changes what it means. `head -n
//! 5` is the first five lines and `head -n -5` all but the last five; `tail -n 5` is the last five and `tail -n +5` everything from
//! the fifth on. Pure, so it is tested on the host.

/// The sign written before the number, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sign {
    None,
    Plus,
    Minus,
}

/// `text` as a sign and a count: digits only after an optional `+` or `-`, no overflow. `None` for anything else, including a
/// sign with no digits.
pub fn parse(text: &str) -> Option<(Sign, usize)> {
    let (sign, digits) = match text.as_bytes().first()? {
        b'+' => (Sign::Plus, &text[1..]),
        b'-' => (Sign::Minus, &text[1..]),
        _ => (Sign::None, text),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().map(|n| (sign, n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_and_signed_numbers() {
        assert_eq!(parse("5"), Some((Sign::None, 5)));
        assert_eq!(parse("0"), Some((Sign::None, 0)));
        assert_eq!(parse("+5"), Some((Sign::Plus, 5)));
        assert_eq!(parse("-5"), Some((Sign::Minus, 5)));
        assert_eq!(parse("007"), Some((Sign::None, 7)));
    }

    #[test]
    fn refusals() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("+"), None);
        assert_eq!(parse("-"), None);
        assert_eq!(parse("x"), None);
        assert_eq!(parse("5x"), None);
        assert_eq!(parse("--5"), None);
        assert_eq!(parse("+-5"), None);
        assert_eq!(parse("5 "), None);
        assert_eq!(parse("99999999999999999999999"), None); // too big to count
    }
}
