//! What `head` and `tail` print of a whole input: the first or last N lines or bytes, or (with a
//! sign on the count) all but the last N, or from the Nth on. Works on a slice, so a caller that
//! has the whole input in memory gets the answer as a sub-slice with no copying. Pure `no_std`, so
//! it is tested on the host.

use crate::countspec::Sign;

/// Whether a count means lines or bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Lines,
    Bytes,
}

/// The number of lines in `data`: one per `\n`, plus one more if the last byte isn't one (an
/// unterminated final line still counts).
pub fn count_lines(data: &[u8]) -> usize {
    let newlines = data.iter().filter(|&&b| b == b'\n').count();
    newlines + usize::from(data.last().is_some_and(|&b| b != b'\n'))
}

/// The offset just past the first `n` lines of `data`: the whole length if it has no more than `n`.
fn lines_end(data: &[u8], n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    data.iter()
        .enumerate()
        .filter(|&(_, &b)| b == b'\n')
        .nth(n - 1)
        .map_or(data.len(), |(i, _)| i + 1)
}

/// What `head` prints: the first `count` lines or bytes; with `Sign::Minus`, everything but the
/// last `count`.
pub fn head(data: &[u8], mode: Mode, sign: Sign, count: usize) -> &[u8] {
    let end = match (mode, sign) {
        (Mode::Bytes, Sign::Minus) => data.len().saturating_sub(count),
        (Mode::Bytes, _) => data.len().min(count),
        (Mode::Lines, Sign::Minus) => lines_end(data, count_lines(data).saturating_sub(count)),
        (Mode::Lines, _) => lines_end(data, count),
    };
    &data[..end]
}

/// What `tail` prints: the last `count` lines or bytes; with `Sign::Plus`, everything from the
/// `count`th on (`+0` and `+1` both mean everything).
pub fn tail(data: &[u8], mode: Mode, sign: Sign, count: usize) -> &[u8] {
    let start = match (mode, sign) {
        (Mode::Bytes, Sign::Plus) => data.len().min(count.saturating_sub(1)),
        (Mode::Bytes, _) => data.len().saturating_sub(count),
        (Mode::Lines, Sign::Plus) => lines_end(data, count.saturating_sub(1)),
        (Mode::Lines, _) => lines_end(data, count_lines(data).saturating_sub(count)),
    };
    &data[start..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use Mode::{Bytes, Lines};
    use Sign::{Minus, None as Unsigned, Plus};

    const TEXT: &[u8] = b"one\ntwo\nthree\nfour\n";

    #[test]
    fn counting_lines() {
        assert_eq!(count_lines(b""), 0);
        assert_eq!(count_lines(b"a\nb\n"), 2);
        assert_eq!(count_lines(b"a\nb"), 2); // an unterminated last line counts
        assert_eq!(count_lines(b"\n"), 1);
    }

    #[test]
    fn head_lines() {
        assert_eq!(head(TEXT, Lines, Unsigned, 2), b"one\ntwo\n");
        assert_eq!(head(TEXT, Lines, Unsigned, 0), b"");
        assert_eq!(head(TEXT, Lines, Unsigned, 99), TEXT); // a short input prints whole
        assert_eq!(head(b"a\nb", Lines, Unsigned, 2), b"a\nb"); // unterminated last line
        assert_eq!(head(b"", Lines, Unsigned, 3), b"");
    }

    #[test]
    fn head_all_but_the_last_lines() {
        assert_eq!(head(TEXT, Lines, Minus, 1), b"one\ntwo\nthree\n");
        assert_eq!(head(TEXT, Lines, Minus, 4), b"");
        assert_eq!(head(TEXT, Lines, Minus, 9), b"");
        assert_eq!(head(TEXT, Lines, Minus, 0), TEXT);
        assert_eq!(head(b"a\nb", Lines, Minus, 1), b"a\n");
    }

    #[test]
    fn head_bytes() {
        assert_eq!(head(b"abcdef", Bytes, Unsigned, 3), b"abc");
        assert_eq!(head(b"abcdef", Bytes, Unsigned, 99), b"abcdef");
        assert_eq!(head(b"abcdef", Bytes, Unsigned, 0), b"");
        assert_eq!(head(b"abcdef", Bytes, Minus, 2), b"abcd");
        assert_eq!(head(b"abcdef", Bytes, Minus, 99), b"");
    }

    #[test]
    fn tail_lines() {
        assert_eq!(tail(TEXT, Lines, Unsigned, 2), b"three\nfour\n");
        assert_eq!(tail(TEXT, Lines, Unsigned, 0), b"");
        assert_eq!(tail(TEXT, Lines, Unsigned, 99), TEXT);
        assert_eq!(tail(b"a\nb", Lines, Unsigned, 1), b"b"); // unterminated last line
        assert_eq!(tail(b"", Lines, Unsigned, 3), b"");
    }

    #[test]
    fn tail_from_the_nth_line() {
        assert_eq!(tail(TEXT, Lines, Plus, 3), b"three\nfour\n");
        assert_eq!(tail(TEXT, Lines, Plus, 1), TEXT);
        assert_eq!(tail(TEXT, Lines, Plus, 0), TEXT);
        assert_eq!(tail(TEXT, Lines, Plus, 5), b"");
        assert_eq!(tail(TEXT, Lines, Plus, 99), b"");
    }

    #[test]
    fn tail_bytes() {
        assert_eq!(tail(b"abcdef", Bytes, Unsigned, 2), b"ef");
        assert_eq!(tail(b"abcdef", Bytes, Unsigned, 99), b"abcdef");
        assert_eq!(tail(b"abcdef", Bytes, Unsigned, 0), b"");
        assert_eq!(tail(b"abcdef", Bytes, Plus, 3), b"cdef");
        assert_eq!(tail(b"abcdef", Bytes, Plus, 1), b"abcdef");
        assert_eq!(tail(b"abcdef", Bytes, Plus, 99), b"");
    }
}
