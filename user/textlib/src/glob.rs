//! Shell-style pattern matching, for `find -name` and for the shell's own filename expansion: `*` (any run of
//! characters, including none), `?` (any one character), `[abc]`, `[a-z]` and `[!abc]`/`[^abc]` (a set of characters,
//! a range, the complement) with POSIX classes inside (`[[:digit:]]`, `[![:space:]]`), and `\` (the next character,
//! literally). A `[` with no closing `]` is an ordinary character. The whole text must match. Characters are Unicode
//! scalar values, not bytes.
//!
//! [`matches`] is the plain matcher (`find -name '*'` matches dot-names too, as GNU's does). The shell additionally
//! treats a name that starts with `.` as matching only a pattern that writes the dot itself: [`matches_name`].
//! Pure `no_std` + `alloc`, so it is tested on the host.

use alloc::string::String;
use alloc::vec::Vec;

/// Whether `pattern` has any unescaped `*`, `?` or `[` -- that is, whether matching it against names
/// could be anything but equality with its own [`unescape`]d text.
pub fn is_magic(pattern: &str) -> bool {
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '*' | '?' | '[' => return true,
            _ => {}
        }
    }
    false
}

/// `pattern` with each escaping backslash removed: the literal text it stands for when it has no
/// magic. A trailing lone backslash is kept.
pub fn unescape(pattern: &str) -> String {
    let mut out = String::new();
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some(next) => out.push(next),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// `text` as a pattern that matches exactly itself: every `\`, `*`, `?` and `[` escaped.
pub fn escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if matches!(c, '\\' | '*' | '?' | '[') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// [`matches`] with the shell's rule for dot-names: a `name` starting with `.` matches only if
/// `pattern` itself starts with a literal (or escaped) `.`, so `*` and `?` and `[…]` skip it.
pub fn matches_name(pattern: &str, name: &str) -> bool {
    if name.starts_with('.') {
        let explicit = pattern.starts_with('.') || pattern.starts_with("\\.");
        if !explicit {
            return false;
        }
    }
    matches(pattern, name)
}

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
        // `[:name:]` inside a set: a whole class of characters.
        if ch == '['
            && p.get(i + 1) == Some(&':')
            && let Some((name_end, member)) = posix_class(p, i, c)
        {
            found |= member;
            i = name_end;
            first = false;
            continue;
        }
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

/// A `[:name:]` class starting at `p[at]` (the `[`): the index after its closing `:]`, and whether
/// `c` is in the class. `None` when it is not one (no closing `:]`, or an unknown name), in which
/// case the `[` is an ordinary set member.
fn posix_class(p: &[char], at: usize, c: char) -> Option<(usize, bool)> {
    let start = at + 2;
    let mut end = start;
    while end + 1 < p.len() && !(p[end] == ':' && p[end + 1] == ']') {
        end += 1;
    }
    if end + 1 >= p.len() {
        return None;
    }
    let name: String = p[start..end].iter().collect();
    let member = match name.as_str() {
        "alpha" => c.is_alphabetic(),
        "digit" => c.is_ascii_digit(),
        "alnum" => c.is_alphanumeric(),
        "upper" => c.is_uppercase(),
        "lower" => c.is_lowercase(),
        "space" => c.is_whitespace(),
        "blank" => c == ' ' || c == '\t',
        "punct" => c.is_ascii_punctuation(),
        "print" => !c.is_control(),
        "graph" => !c.is_control() && !c.is_whitespace(),
        "cntrl" => c.is_control(),
        "xdigit" => c.is_ascii_hexdigit(),
        _ => return None,
    };
    Some((end + 2, member))
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

    #[test]
    fn posix_classes_in_a_set() {
        assert!(matches("[[:digit:]]", "7"));
        assert!(!matches("[[:digit:]]", "x"));
        assert!(matches("[[:alpha:]_]", "_"));
        assert!(matches("[![:space:]]", "a"));
        assert!(!matches("[![:space:]]", " "));
        assert!(matches("f[[:upper:][:digit:]]", "fQ"));
        assert!(matches("f[[:upper:][:digit:]]", "f3"));
        // An unknown class name is not a class: `[[:nope:]` is a set of its characters, and the
        // last `]` is then an ordinary character after it.
        assert!(matches("[[:nope:]]", ":]"));
        assert!(!matches("[[:nope:]]", ":"));
    }

    #[test]
    fn magic_and_escapes() {
        assert!(is_magic("a*"));
        assert!(is_magic("[a]"));
        assert!(is_magic("?"));
        assert!(!is_magic("abc"));
        assert!(!is_magic("a\\*b"));
        assert_eq!(unescape("a\\*b\\\\"), "a*b\\");
        assert_eq!(escape("a*b?[c\\"), "a\\*b\\?\\[c\\\\");
        assert!(matches(&escape("a*b"), "a*b"));
        assert!(!matches(&escape("a*b"), "axb"));
    }

    #[test]
    fn a_dot_name_needs_an_explicit_dot() {
        assert!(!matches_name("*", ".profile"));
        assert!(!matches_name("?profile", ".profile"));
        assert!(!matches_name("[.]profile", ".profile"));
        assert!(matches_name(".*", ".profile"));
        assert!(matches_name(".profile", ".profile"));
        assert!(matches_name("\\.profile", ".profile"));
        assert!(matches_name("*", "profile"));
        assert!(matches_name("a.*", "a.b")); // only the *start* of a name is special
    }
}
