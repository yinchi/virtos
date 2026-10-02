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

fn fold(b: u8, ignore_case: bool) -> u8 {
    if ignore_case {
        b.to_ascii_lowercase()
    } else {
        b
    }
}

/// Whether `needle` occurs in `haystack`, comparing ASCII letters without regard to case if `ignore_case`. An empty needle
/// occurs everywhere.
pub fn contains(haystack: &[u8], needle: &[u8], ignore_case: bool) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack.windows(needle.len()).any(|w| {
        w.iter()
            .zip(needle)
            .all(|(&a, &b)| fold(a, ignore_case) == fold(b, ignore_case))
    })
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
}
