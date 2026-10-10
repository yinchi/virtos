//! The parser for `.editrc` (in `$HOME`), read once when `edit` starts. Same shape as
//! `shell/environment.rs`'s `/etc/environment` parser, in format only -- not shared code, since the
//! two live in different crates (the kernel and this one) and there is nothing to share beyond the
//! shape both happen to use:
//!
//! - a blank line, or one whose first non-blank character is `#`, is ignored;
//! - otherwise the line is `NAME=VALUE`: the name is what precedes the first `=` (surrounding
//!   blanks trimmed); the value is everything after that `=`, trimmed, except a trailing carriage
//!   return;
//! - unlike `/etc/environment`, only four names exist, each with a specific value shape --
//!   `TABSIZE` (`1..=16`), `LINENOS`, `AUTOINDENT` and `TABSTOSPACES` (`0` or `1`) -- so a name this parser doesn't
//!   know, or a value that doesn't fit the name it's for, is a reported problem, and that setting
//!   keeps its default;
//! - a line that is not of that shape (no `=`, an empty name) is likewise skipped and reported;
//! - a name that appears twice takes its last valid value; an invalid repeat doesn't undo an
//!   earlier valid one.
//!
//! Pure `no_std` + `alloc`, so it is tested on the host (`cargo test`). Turning a [`Problem`] into
//! the message-line note `edit` shows ("bad TABSIZE in .editrc, using 4") is the caller's job,
//! since only it knows which default applies -- this module reports what went wrong, not how to
//! word it on screen.

use alloc::vec::Vec;

/// The settings `.editrc` can override, already validated -- what `edit` actually reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// Columns per tab stop, `1..=16`.
    pub tab_size: u8,
    /// Whether the line-number gutter starts on.
    pub line_numbers: bool,
    /// Whether auto-indent starts on.
    pub auto_indent: bool,
    /// Whether the Tab key types spaces up to the next tab stop instead of a tab character.
    pub tabs_to_spaces: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tab_size: 4,
            line_numbers: false,
            auto_indent: false,
            tabs_to_spaces: false,
        }
    }
}

/// A line that didn't change `config`, and why.
#[derive(Debug, PartialEq, Eq)]
pub struct Problem {
    /// 1-based.
    pub line: usize,
    pub why: &'static str,
}

/// What `parse` found: the settings (defaults, where nothing valid overrode them) and the lines it
/// had to skip.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub config: Config,
    pub problems: Vec<Problem>,
}

/// A value that must be `0` or `1`, as `true`/`false`.
fn bit(value: &str) -> Option<bool> {
    match value {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

/// Parses the text of a `.editrc`.
pub fn parse(text: &str) -> Parsed {
    let mut parsed = Parsed::default();
    for (index, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut problem = |why| {
            parsed.problems.push(Problem {
                line: index + 1,
                why,
            })
        };
        let Some((name, value)) = line.split_once('=') else {
            problem("expected NAME=VALUE (no '=')");
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        match name {
            "TABSIZE" => match value.parse::<u8>() {
                Ok(n) if (1..=16).contains(&n) => parsed.config.tab_size = n,
                _ => problem("TABSIZE must be 1-16"),
            },
            "LINENOS" => match bit(value) {
                Some(b) => parsed.config.line_numbers = b,
                None => problem("LINENOS must be 0 or 1"),
            },
            "AUTOINDENT" => match bit(value) {
                Some(b) => parsed.config.auto_indent = b,
                None => problem("AUTOINDENT must be 0 or 1"),
            },
            "TABSTOSPACES" => match bit(value) {
                Some(b) => parsed.config.tabs_to_spaces = b,
                None => problem("TABSTOSPACES must be 0 or 1"),
            },
            "" => problem("expected NAME=VALUE (empty name)"),
            _ => problem("unknown option"),
        }
    }
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_is_every_default() {
        assert_eq!(parse(""), Parsed::default());
        assert_eq!(parse("\n\n"), Parsed::default());
        assert_eq!(
            Config::default(),
            Config {
                tab_size: 4,
                line_numbers: false,
                auto_indent: false,
                tabs_to_spaces: false
            }
        );
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let p = parse("# a comment\n\n   \n  # indented\nTABSIZE=8\n");
        assert_eq!(p.config.tab_size, 8);
        assert!(p.problems.is_empty());
    }

    #[test]
    fn every_setting_can_be_overridden() {
        let p = parse("TABSIZE=2\nLINENOS=1\nAUTOINDENT=1\nTABSTOSPACES=1\n");
        assert_eq!(
            p.config,
            Config {
                tab_size: 2,
                line_numbers: true,
                auto_indent: true,
                tabs_to_spaces: true
            }
        );
        assert!(p.problems.is_empty());
    }

    #[test]
    fn tabsize_accepts_its_whole_range_and_nothing_outside_it() {
        assert_eq!(parse("TABSIZE=1\n").config.tab_size, 1);
        assert_eq!(parse("TABSIZE=16\n").config.tab_size, 16);
        // (" 4" and "4 " are not in this list: the value is trimmed before parsing, so they are 4.)
        for bad in ["0", "17", "-1", "4.0", "", "x"] {
            let p = parse(&alloc::format!("TABSIZE={bad}\n"));
            assert_eq!(
                p.config.tab_size, 4,
                "{bad:?} should not change the default"
            );
            assert_eq!(
                p.problems,
                [Problem {
                    line: 1,
                    why: "TABSIZE must be 1-16"
                }]
            );
        }
    }

    #[test]
    fn linenos_and_autoindent_accept_only_0_or_1() {
        for bad in ["2", "true", "yes", "", "01", "-1"] {
            let p = parse(&alloc::format!("LINENOS={bad}\n"));
            assert!(!p.config.line_numbers);
            assert_eq!(
                p.problems,
                [Problem {
                    line: 1,
                    why: "LINENOS must be 0 or 1"
                }]
            );

            let p = parse(&alloc::format!("AUTOINDENT={bad}\n"));
            assert!(!p.config.auto_indent);
            assert_eq!(
                p.problems,
                [Problem {
                    line: 1,
                    why: "AUTOINDENT must be 0 or 1"
                }]
            );
        }
    }

    #[test]
    fn blanks_around_the_name_and_value_are_trimmed() {
        let p = parse("  TABSIZE  =  8  \n");
        assert_eq!(p.config.tab_size, 8);
        assert!(p.problems.is_empty());
    }

    #[test]
    fn an_unknown_name_is_reported_and_changes_nothing() {
        let p = parse("SOFTWRAP=1\n");
        assert_eq!(p.config, Config::default());
        assert_eq!(
            p.problems,
            [Problem {
                line: 1,
                why: "unknown option"
            }]
        );
    }

    #[test]
    fn a_bad_line_is_skipped_and_reported_with_its_number() {
        let p = parse("TABSIZE=8\nnot an option\n=orphan\nLINENOS=1\n");
        assert_eq!(
            p.config,
            Config {
                tab_size: 8,
                line_numbers: true,
                ..Config::default()
            }
        );
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
            ]
        );
    }

    #[test]
    fn a_repeated_name_takes_its_last_valid_value() {
        let p = parse("TABSIZE=8\nTABSIZE=2\n");
        assert_eq!(p.config.tab_size, 2);

        // An invalid repeat is reported but doesn't undo the earlier valid one.
        let p = parse("TABSIZE=8\nTABSIZE=99\n");
        assert_eq!(p.config.tab_size, 8);
        assert_eq!(
            p.problems,
            [Problem {
                line: 2,
                why: "TABSIZE must be 1-16"
            }]
        );
    }

    #[test]
    fn crlf_line_endings_and_a_missing_final_newline_are_fine() {
        let p = parse("TABSIZE=8\r\nLINENOS=1");
        assert_eq!(
            p.config,
            Config {
                tab_size: 8,
                line_numbers: true,
                ..Config::default()
            }
        );
        assert!(p.problems.is_empty());
    }

    #[test]
    fn tabstospaces_is_a_bit_like_the_other_toggles() {
        assert!(parse("TABSTOSPACES=1\n").config.tabs_to_spaces);
        let p = parse("TABSTOSPACES=yes\n");
        assert!(!p.config.tabs_to_spaces);
        assert_eq!(p.problems[0].why, "TABSTOSPACES must be 0 or 1");
    }
}
