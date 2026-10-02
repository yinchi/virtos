//! The initial environment: the parser for `/etc/environment`, the file Linux's `pam_env` reads.
//! Plain `NAME=VALUE` lines -- not shell syntax:
//!
//! - a blank line, or one whose first non-blank character is `#`, is ignored;
//! - otherwise the line is `NAME=VALUE`: the name is what precedes the first `=` (surrounding
//!   blanks trimmed) and must be a valid variable name; the value is **everything after that `=`,
//!   literally** (no `$` expansion, no escapes, blanks kept) except a trailing carriage return --
//!   and except that **one matching pair of single or double quotes around the whole value is
//!   removed** (`PS1='\w> '`), which is how a value keeps leading or trailing blanks that an editor
//!   or a hook would strip (Linux's `pam_env` and systemd's environment files accept quotes too).
//!   Quotes anywhere else, or unmatched, stay as written;
//! - a line that is not of that shape is skipped and reported, not fatal;
//! - a name that appears twice takes its last value, in the place it first appeared.
//!
//! Pure `no_std` + `alloc`, so it is tested on the host. Reading the file and applying it to the
//! shell's frame is `shell::load_environment`.

use alloc::string::String;
use alloc::vec::Vec;

use super::frame_stack::is_valid_name;

/// A line that was skipped, and why.
#[derive(Debug, PartialEq, Eq)]
pub struct Problem {
    /// 1-based.
    pub line: usize,
    pub why: &'static str,
}

/// What `parse` found: the variables in order, and the lines it had to skip.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub vars: Vec<(String, String)>,
    pub problems: Vec<Problem>,
}

/// `value` without one matching pair of single or double quotes around all of it, if it has one.
fn unquote(value: &str) -> &str {
    for quote in ['\'', '"'] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|v| v.strip_suffix(quote))
        {
            return inner;
        }
    }
    value
}

/// Parses the text of an environment file.
pub fn parse(text: &str) -> Parsed {
    let mut parsed = Parsed::default();
    for (index, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let problem = |why| Problem {
            line: index + 1,
            why,
        };
        let Some((name, value)) = line.split_once('=') else {
            parsed
                .problems
                .push(problem("expected NAME=VALUE (no '=')"));
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            parsed
                .problems
                .push(problem("expected NAME=VALUE (empty name)"));
            continue;
        }
        if !is_valid_name(name) {
            parsed.problems.push(problem("not a valid variable name"));
            continue;
        }
        let value = unquote(value);
        match parsed.vars.iter_mut().find(|(n, _)| n == name) {
            Some((_, v)) => *v = String::from(value),
            None => parsed.vars.push((String::from(name), String::from(value))),
        }
    }
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(parsed: &Parsed) -> Vec<(&str, &str)> {
        parsed
            .vars
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_str()))
            .collect()
    }

    #[test]
    fn plain_lines_become_variables_in_order() {
        let p = parse("HOME=/root\nTZ=America/Toronto\n");
        assert_eq!(vars(&p), [("HOME", "/root"), ("TZ", "America/Toronto")]);
        assert!(p.problems.is_empty());
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let p = parse("# the system environment\n\n   \n  # indented comment\nA=1\n");
        assert_eq!(vars(&p), [("A", "1")]);
        assert!(p.problems.is_empty());
    }

    #[test]
    fn the_value_is_literal_after_the_first_equals() {
        let p = parse("A=b=c\nB=\nC=  spaced  \nD=$HOME\nE=a\\nb\n");
        assert_eq!(
            vars(&p),
            [
                ("A", "b=c"),
                ("B", ""),
                ("C", "  spaced  "),
                ("D", "$HOME"),
                ("E", "a\\nb")
            ]
        );
    }

    #[test]
    fn one_matching_pair_of_quotes_around_the_value_is_removed() {
        let p = parse("A='x'\nB=\"y\"\nC='\\w> '\nD=\"  padded  \"\nE=''\nF=\"it's\"\n");
        assert_eq!(
            vars(&p),
            [
                ("A", "x"),
                ("B", "y"),
                ("C", "\\w> "),
                ("D", "  padded  "),
                ("E", ""),
                ("F", "it's")
            ]
        );
    }

    #[test]
    fn other_quotes_stay_as_written() {
        let p = parse("A='x\nB=x'\nC='x\"\nD=a'b'\nE='a'b'\nF='\nG=\"'x'\"\n");
        assert_eq!(
            vars(&p),
            [
                ("A", "'x"),
                ("B", "x'"),
                ("C", "'x\""),
                ("D", "a'b'"),
                ("E", "a'b"), // a pair around everything, whatever is inside
                ("F", "'"),
                ("G", "'x'")
            ]
        );
    }

    #[test]
    fn blanks_around_the_name_are_trimmed_but_not_inside_it() {
        let p = parse("  A  =1\nB C=2\n");
        assert_eq!(vars(&p), [("A", "1")]);
        assert_eq!(
            p.problems,
            [Problem {
                line: 2,
                why: "not a valid variable name"
            }]
        );
    }

    #[test]
    fn crlf_line_endings_and_a_missing_final_newline_are_fine() {
        let p = parse("A=1\r\nB=2");
        assert_eq!(vars(&p), [("A", "1"), ("B", "2")]);
    }

    #[test]
    fn a_bad_line_is_skipped_and_reported_with_its_number() {
        let p = parse("A=1\nnot a variable\n=orphan\n1x=2\nB=2\n");
        assert_eq!(vars(&p), [("A", "1"), ("B", "2")]);
        assert_eq!(
            p.problems,
            [
                Problem {
                    line: 2,
                    why: "expected NAME=VALUE (no '=')"
                },
                Problem {
                    line: 3,
                    why: "expected NAME=VALUE (empty name)"
                },
                Problem {
                    line: 4,
                    why: "not a valid variable name"
                },
            ]
        );
    }

    #[test]
    fn a_repeated_name_takes_its_last_value_in_its_first_place() {
        let p = parse("A=1\nB=2\nA=3\n");
        assert_eq!(vars(&p), [("A", "3"), ("B", "2")]);
    }

    #[test]
    fn an_empty_file_is_an_empty_environment() {
        assert_eq!(parse(""), Parsed::default());
        assert_eq!(parse("\n\n"), Parsed::default());
    }
}
