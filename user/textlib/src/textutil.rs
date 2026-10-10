//! Line and substring helpers for the filters. Pure `no_std` + `alloc`, so they are tested on the host.

use alloc::vec::Vec;

/// The lines of `data`: split at `\n`, the terminator not included. A last line with no `\n` counts, but the empty
/// remainder after a final `\n` does not, so `"a\nb\n"` and `"a\nb"` are both two lines and `""` is none.
pub fn split_lines(data: &[u8]) -> Vec<&[u8]> {
    let mut lines: Vec<&[u8]> = data.split(|&b| b == b'\n').collect();
    if lines.last().is_some_and(|last| last.is_empty()) {
        lines.pop();
    }
    lines
}

/// The pieces of `line` between occurrences of `delimiter` (any non-empty byte string): `"a,b,,c"`
/// at `,` is `a`, `b`, ``, `c`; a line with no delimiter is one piece, an empty line is one empty
/// piece, and a trailing delimiter leaves a final empty piece.
pub fn split_fields<'a>(line: &'a [u8], delimiter: &[u8]) -> Vec<&'a [u8]> {
    let mut fields = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i + delimiter.len() <= line.len() {
        if &line[i..i + delimiter.len()] == delimiter {
            fields.push(&line[start..i]);
            i += delimiter.len();
            start = i;
        } else {
            i += 1;
        }
    }
    fields.push(&line[start..]);
    fields
}

/// `b`, ASCII-lowercased if `ignore_case`.
fn fold(b: u8, ignore_case: bool) -> u8 {
    if ignore_case {
        b.to_ascii_lowercase()
    } else {
        b
    }
}

/// The first position at or after `from` where `needle` occurs in `haystack`, comparing ASCII letters without regard to case
/// if `ignore_case`. An empty needle occurs at `from` (if that is within the haystack, or at its end).
pub fn find(haystack: &[u8], needle: &[u8], from: usize, ignore_case: bool) -> Option<usize> {
    if from > haystack.len() {
        return None;
    }
    if needle.is_empty() {
        return Some(from);
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| {
            w.iter()
                .zip(needle)
                .all(|(&a, &b)| fold(a, ignore_case) == fold(b, ignore_case))
        })
        .map(|p| p + from)
}

/// Whether `needle` occurs in `haystack`, comparing ASCII letters without regard to case if `ignore_case`. An empty needle
/// occurs everywhere.
pub fn contains(haystack: &[u8], needle: &[u8], ignore_case: bool) -> bool {
    find(haystack, needle, 0, ignore_case).is_some()
}

/// Which occurrences of a pattern in a line count as a match (`fgrep`'s `-w` and `-x`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MatchMode {
    /// Anywhere in the line.
    Any,
    /// `-w`: bounded on both sides by a non-word byte (or the line's end); a word byte is `[A-Za-z0-9_]`.
    Word,
    /// `-x`: the whole line.
    Line,
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The first match of `pattern` in `line` at or after `from`, as `(start, end)`, under `mode`. `Word` retries later
/// occurrences when an earlier one is not bounded (`foobar foo` for `foo` is the second), and an empty pattern matches
/// only an empty line, or (in `Word`) where no word byte touches it -- as GNU's `grep -Fw ''` does.
pub fn match_in_line(
    line: &[u8],
    pattern: &[u8],
    from: usize,
    ignore_case: bool,
    mode: MatchMode,
) -> Option<(usize, usize)> {
    match mode {
        MatchMode::Line => {
            let whole = from == 0
                && line.len() == pattern.len()
                && find(line, pattern, 0, ignore_case) == Some(0);
            whole.then_some((0, line.len()))
        }
        MatchMode::Any => find(line, pattern, from, ignore_case).map(|s| (s, s + pattern.len())),
        MatchMode::Word => {
            let mut at = from;
            while let Some(start) = find(line, pattern, at, ignore_case) {
                let end = start + pattern.len();
                let left_ok = start == 0 || !is_word(line[start - 1]);
                let right_ok = end == line.len() || !is_word(line[end]);
                if left_ok && right_ok {
                    return Some((start, end));
                }
                at = start + 1;
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines() {
        assert_eq!(split_lines(b"a\nb\n"), [&b"a"[..], b"b"]);
        assert_eq!(split_lines(b"a\nb"), [&b"a"[..], b"b"]);
        assert!(split_lines(b"").is_empty());
        assert_eq!(split_lines(b"\n"), [&b""[..]]);
        assert_eq!(split_lines(b"\n\n"), [&b""[..], b""]);
        assert_eq!(split_lines(b"a\n\nb\n"), [&b"a"[..], b"", b"b"]);
        assert_eq!(split_lines(b"one"), [&b"one"[..]]);
    }

    #[test]
    fn fields_are_the_pieces_between_delimiters() {
        assert_eq!(split_fields(b"a,b,c", b","), [&b"a"[..], b"b", b"c"]);
        assert_eq!(split_fields(b"a,,c", b","), [&b"a"[..], b"", b"c"]);
        assert_eq!(split_fields(b"a,b,", b","), [&b"a"[..], b"b", b""]); // a trailing delimiter
        assert_eq!(split_fields(b",a", b","), [&b""[..], b"a"]);
        assert_eq!(split_fields(b"lonely", b","), [&b"lonely"[..]]); // none: the whole line
        assert_eq!(split_fields(b"", b","), [&b""[..]]);
        assert_eq!(split_fields(b",", b","), [&b""[..], b""]);
    }

    #[test]
    fn a_delimiter_may_be_several_bytes() {
        assert_eq!(
            split_fields("a\u{65e5}b".as_bytes(), "\u{65e5}".as_bytes()),
            [&b"a"[..], b"b"]
        );
        assert_eq!(split_fields(b"a::b:c", b"::"), [&b"a"[..], b"b:c"]);
        assert_eq!(split_fields(b"a:::b", b"::"), [&b"a"[..], b":b"]); // left to right, no overlap
    }

    #[test]
    fn substrings() {
        assert!(contains(b"hello world", b"lo wo", false));
        assert!(!contains(b"hello", b"world", false));
        assert!(contains(b"hello", b"hello", false));
        assert!(!contains(b"hell", b"hello", false));
        assert!(contains(b"", b"", false));
        assert!(contains(b"abc", b"", false));
        assert!(!contains(b"", b"a", false));
    }

    #[test]
    fn case_folding_is_ascii() {
        assert!(contains(b"Hello", b"hELLO", true));
        assert!(!contains(b"Hello", b"hELLO", false));
        assert!(contains("É".as_bytes(), "É".as_bytes(), true));
        assert!(!contains("É".as_bytes(), "é".as_bytes(), true)); // not folded
    }

    #[test]
    fn find_from_a_position() {
        assert_eq!(find(b"abcabc", b"bc", 0, false), Some(1));
        assert_eq!(find(b"abcabc", b"bc", 2, false), Some(4));
        assert_eq!(find(b"abcabc", b"bc", 5, false), None);
        assert_eq!(find(b"abc", b"", 3, false), Some(3));
        assert_eq!(find(b"abc", b"", 4, false), None);
        assert_eq!(find(b"ABC", b"b", 0, true), Some(1));
    }

    #[test]
    fn whole_line_matches() {
        let line =
            |l: &str, p: &str| match_in_line(l.as_bytes(), p.as_bytes(), 0, false, MatchMode::Line);
        assert_eq!(line("abc", "abc"), Some((0, 3)));
        assert_eq!(line("abcd", "abc"), None);
        assert_eq!(line("", ""), Some((0, 0)));
    }

    #[test]
    fn word_matches_retry_later_occurrences() {
        let word =
            |l: &str, p: &str| match_in_line(l.as_bytes(), p.as_bytes(), 0, false, MatchMode::Word);
        assert_eq!(word("foo bar", "foo"), Some((0, 3)));
        assert_eq!(word("foobar foo", "foo"), Some((7, 10)));
        assert_eq!(word("foobar", "foo"), None);
        assert_eq!(word("a_foo foo.", "foo"), Some((6, 9)));
        assert_eq!(word("(foo)", "foo"), Some((1, 4)));
        assert_eq!(word("xfoo", "foo"), None);
    }
}
