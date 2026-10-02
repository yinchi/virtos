//! Shell-style pattern matching for `find -name`: `*` (any run of characters, including none), `?` (any one character),
//! `[abc]`, `[a-z]` and `[!abc]`/`[^abc]` (a set of characters, a range, the complement), and `\` (the next character,
//! literally). A `[` with no closing `]` is an ordinary character. The whole text must match; there is no special case for
//! a leading `.` (`find -name '*'` matches dot-names too, as GNU's does). Characters are Unicode scalar values, not bytes.
//! Pure `no_std` + `alloc`, so it is tested on the host.

use alloc::vec::Vec;

/// Whether `text` matches the whole of `pattern`.
pub fn matches(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    // Where to resume after the last `*`: the pattern just past it, and the text position it will try next.
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some((pi + 1, ti));
            pi += 1;
            continue;
        }
        if pi < p.len()
            && let Some(next) = step(&p, pi, t[ti])
        {
            pi = next;
            ti += 1;
            continue;
        }
        match star {
            Some((after, at)) => {
                star = Some((after, at + 1));
                pi = after;
                ti = at + 1;
            }
            None => return false,
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// If the pattern element at `pi` (not a `*`) matches `c`, the index of the next element.
fn step(p: &[char], pi: usize, c: char) -> Option<usize> {
    match p[pi] {
        '?' => Some(pi + 1),
        '[' => match class(p, pi, c) {
            Some((true, next)) => Some(next),
            Some((false, _)) => None,
            None => (c == '[').then_some(pi + 1), // no closing `]`: a plain `[`
        },
        '\\' if pi + 1 < p.len() => (p[pi + 1] == c).then_some(pi + 2),
        literal => (literal == c).then_some(pi + 1),
    }
}

/// The bracket expression starting at `p[open]`: whether `c` is in it, and the index after its `]`; `None` if it never closes.
fn class(p: &[char], open: usize, c: char) -> Option<(bool, usize)> {
    let mut i = open + 1;
    let negate = matches!(p.get(i), Some('!' | '^'));
    if negate {
        i += 1;
    }
    let mut found = false;
    let mut first = true;
    loop {
        let ch = *p.get(i)?;
        if ch == ']' && !first {
            return Some((found != negate, i + 1));
        }
        first = false;
        let lo = if ch == '\\' && i + 1 < p.len() {
            i += 1;
            p[i]
        } else {
            ch
        };
        i += 1;
        if i + 1 < p.len() && p[i] == '-' && p[i + 1] != ']' {
            let hi = p[i + 1];
            found |= (lo..=hi).contains(&c);
            i += 2;
        } else {
            found |= lo == c;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_must_match_whole() {
        assert!(matches("abc", "abc"));
        assert!(!matches("abc", "abcd"));
        assert!(!matches("abc", "ab"));
        assert!(matches("", ""));
        assert!(!matches("", "a"));
    }

    #[test]
    fn star_matches_any_run() {
        assert!(matches("*", ""));
        assert!(matches("*", "anything"));
        assert!(matches("*.txt", "a.txt"));
        assert!(matches("*.txt", ".txt"));
        assert!(!matches("*.txt", "a.txtx"));
        assert!(matches("a*c", "abbbc"));
        assert!(matches("a*c", "ac"));
        assert!(!matches("a*c", "ab"));
        assert!(matches("*a*b*", "xxaxxbxx"));
        assert!(matches("**", "x"));
        assert!(matches("a*b*c", "aXbXbXc"));
    }

    #[test]
    fn star_backtracks() {
        assert!(matches("*ab", "aab"));
        assert!(matches("*ab", "abab"));
        assert!(matches("a*a*a", "aaa"));
        assert!(!matches("a*a*a", "aa"));
        assert!(matches("*.tar.gz", "x.tar.gz"));
        assert!(!matches("*.tar.gz", "x.tar.g"));
    }

    #[test]
    fn question_mark_is_one_character() {
        assert!(matches("?", "a"));
        assert!(!matches("?", ""));
        assert!(!matches("?", "ab"));
        assert!(matches("a?c", "abc"));
        assert!(matches("?", "日")); // a character, not a byte
    }

    #[test]
    fn a_set_or_range() {
        assert!(matches("[abc]", "b"));
        assert!(!matches("[abc]", "d"));
        assert!(matches("[a-c]x", "bx"));
        assert!(!matches("[a-c]x", "dx"));
        assert!(matches("[a-cx-z]", "y"));
        assert!(matches("file[0-9][0-9]", "file42"));
        assert!(!matches("file[0-9][0-9]", "file4"));
    }

    #[test]
    fn a_complement() {
        assert!(matches("[!a]", "b"));
        assert!(!matches("[!a]", "a"));
        assert!(matches("[^a-c]", "d"));
        assert!(!matches("[^a-c]", "b"));
    }

    #[test]
    fn a_bracket_may_hold_a_bracket_or_a_dash() {
        assert!(matches("[]]", "]"));
        assert!(matches("[]a]", "a"));
        assert!(matches("[a-]", "-"));
        assert!(matches("[a-]", "a"));
        assert!(matches("[!]]", "x"));
    }

    #[test]
    fn an_unclosed_bracket_is_a_plain_one() {
        assert!(matches("[abc", "[abc"));
        assert!(matches("a[", "a["));
        assert!(!matches("[abc", "a"));
    }

    #[test]
    fn a_backslash_makes_the_next_character_literal() {
        assert!(matches("\\*", "*"));
        assert!(!matches("\\*", "a"));
        assert!(matches("a\\?b", "a?b"));
        assert!(matches("\\[a]", "[a]"));
        assert!(matches("[\\]]", "]"));
    }

    #[test]
    fn nothing_special_for_a_leading_dot() {
        assert!(matches("*", ".hidden"));
        assert!(matches("?hidden", ".hidden"));
    }
}
