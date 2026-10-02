//! Parses a lexed shell line into a `Pipeline`: one or more `|`-separated `Segment`s, each with
//! its own argv and redirections. Parses the *token stream* `lexer::lex` produces, not the raw
//! line -- `parse` (below) is the one public entry point, lexing internally, so callers never
//! call `lexer::lex` themselves.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::lexer::{self, LexError, RedirOp, Token, Word};

/// One redirection a `Segment` applies before its program starts. Its own target is plain text,
/// not a `Word` -- `$VAR` expansion inside a redirect target isn't implemented yet (`echo x >
/// $FILE` stays unimplemented, `$FILE` taken as a literal name): `Word::raw()` reconstructs
/// whatever text the lexer matched, rendering any expansion marker back as literal `$name`/`$?`
/// rather than silently doing the wrong thing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redirection {
    In(String),
    Out { fd: u8, path: String, append: bool },
    Dup { fd: u8, target: u8 },
}

/// One `|`-separated command: its own words and redirections, in the order they were written.
/// `words`, not `argv` -- each one may still need `$VAR`/`${VAR}`/`$?` expansion (`shell::expand`)
/// before it's a real argument string.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Segment {
    pub words: Vec<Word>,
    pub redirs: Vec<Redirection>,
}

pub type Pipeline = Vec<Segment>;

/// Why `parse` refused a line outright.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyntaxError {
    Lex(LexError),
    /// A redirection operator with nothing after it, named as typed (`"<"`, `">"`, `">>"`,
    /// `">&"`).
    MissingTarget(&'static str),
    /// `>&`'s target wasn't `1` or `2`.
    BadDupTarget,
    /// A line with nothing on it at all -- blank or comment-only: not worth reporting.
    EmptyCommand,
    /// A pipeline stage with no words and no redirections, in a line that has other things on it:
    /// a stray `|` with nothing on one side of it (`| a`, `a |`, `a | | b`).
    MissingCommand,
}

/// Lexes then parses `line` into a `Pipeline`.
pub fn parse(line: &str) -> Result<Pipeline, SyntaxError> {
    let tokens = lexer::lex(line).map_err(SyntaxError::Lex)?;
    if tokens.is_empty() {
        return Err(SyntaxError::EmptyCommand);
    }
    grammar::pipeline(&SliceByRef(&tokens)).map_err(decode_error)
}

/// Exposes a `&[T]` to `peg` as `&T` elements (themselves `Copy`, satisfying `peg`'s built-in
/// `[T]` slice support, which requires the *element* type to be `Copy`) rather than requiring
/// `T` itself to be `Copy` -- `peg`'s own documented pattern for parsing a slice of a non-`Copy`
/// element type (its test suite, `tests/pass/tokens_struct.rs`), reproduced here since the crate
/// doesn't export it itself.
struct SliceByRef<'a, T>(&'a [T]);

impl<'a, T> peg::Parse for SliceByRef<'a, T> {
    type PositionRepr = usize;
    fn start(&self) -> usize {
        0
    }
    fn is_eof(&self, pos: usize) -> bool {
        pos >= self.0.len()
    }
    fn position_repr(&self, pos: usize) -> usize {
        pos
    }
}

impl<'a, T: 'a> peg::ParseElem<'a> for SliceByRef<'a, T> {
    type Element = &'a T;
    fn parse_elem(&'a self, pos: usize) -> peg::RuleResult<&'a T> {
        match self.0[pos..].first() {
            Some(c) => peg::RuleResult::Matched(pos + 1, c),
            None => peg::RuleResult::Failed,
        }
    }
}

/// Translates a `peg::ParseError` back into the specific `SyntaxError` its own `{? Err("...") }`
/// action reported -- see `lexer::decode_error`'s own doc comment for why matching the sentinel
/// string back to its variant works.
fn decode_error(e: peg::error::ParseError<usize>) -> SyntaxError {
    for token in e.expected.tokens() {
        match token {
            "missing-target-in" => return SyntaxError::MissingTarget(RedirOp::In.text()),
            "missing-target-out" => return SyntaxError::MissingTarget(RedirOp::Out.text()),
            "missing-target-append" => return SyntaxError::MissingTarget(RedirOp::Append.text()),
            "missing-target-dup" => return SyntaxError::MissingTarget(RedirOp::DupOut.text()),
            "bad-dup-target" => return SyntaxError::BadDupTarget,
            "empty-command" => return SyntaxError::MissingCommand,
            _ => {}
        }
    }
    // A structurally malformed token stream (e.g. two pipes in a row, each side otherwise empty)
    // can leave tokens unconsumed without tripping one of the named failures above -- the
    // resulting "expected end of input" is best described the same way an empty stage already is,
    // not a case to panic on for an input the lexer itself already accepted.
    SyntaxError::MissingCommand
}

/// One thing a `segment` rule saw: either an ordinary argument, or a redirection. Not part of
/// this module's public API -- `Segment` itself splits these apart before returning.
enum Item {
    Arg(Word),
    Redir(Redirection),
}

peg::parser! {
    grammar grammar<'a>() for SliceByRef<'a, Token> {
        /// A plain argument word, kept structured -- `$VAR`/`${VAR}`/`$?` expansion (`shell::
        /// expand`) still needs to happen against it later.
        rule word() -> Word
            = [Token::Word(w)] { w.clone() }

        /// A redirect's own target, or `>&`'s dup-target digit -- plain text, via `Word::raw()`
        /// (its own doc comment explains why: expansion isn't implemented for these two call
        /// sites yet).
        rule raw_word() -> String
            = [Token::Word(w)] { w.raw() }

        rule redir() -> Redirection
            = [Token::Redir { fd, op: RedirOp::In }] path:raw_word() { Redirection::In(path) }
            / [Token::Redir { op: RedirOp::In, .. }] {? Err("missing-target-in") }
            / [Token::Redir { fd, op: RedirOp::Out }] path:raw_word() {
                Redirection::Out { fd: fd.unwrap_or(1), path, append: false }
              }
            / [Token::Redir { op: RedirOp::Out, .. }] {? Err("missing-target-out") }
            / [Token::Redir { fd, op: RedirOp::Append }] path:raw_word() {
                Redirection::Out { fd: fd.unwrap_or(1), path, append: true }
              }
            / [Token::Redir { op: RedirOp::Append, .. }] {? Err("missing-target-append") }
            / [Token::Redir { fd, op: RedirOp::DupOut }] path:raw_word() {?
                match path.as_str() {
                    "1" => Ok(Redirection::Dup { fd: fd.unwrap_or(1), target: 1 }),
                    "2" => Ok(Redirection::Dup { fd: fd.unwrap_or(1), target: 2 }),
                    _ => Err("bad-dup-target"),
                }
              }
            / [Token::Redir { op: RedirOp::DupOut, .. }] {? Err("missing-target-dup") }

        rule item() -> Item
            = w:word() { Item::Arg(w) }
            / r:redir() { Item::Redir(r) }

        rule segment() -> Segment
            = items:item()* {?
                let mut words = Vec::new();
                let mut redirs = Vec::new();
                for it in items {
                    match it {
                        Item::Arg(a) => words.push(a),
                        Item::Redir(r) => redirs.push(r),
                    }
                }
                if words.is_empty() && redirs.is_empty() {
                    Err("empty-command")
                } else {
                    Ok(Segment { words, redirs })
                }
              }

        pub rule pipeline() -> Pipeline
            = s0:segment() rest:([Token::Pipe] s:segment() { s })* {
                let mut v = Vec::with_capacity(1 + rest.len());
                v.push(s0);
                v.extend(rest);
                v
              }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `.raw()` is good enough for these tests -- none of them involve an expansion, so it's the
    // same text `argv` used to compare directly before `Segment.words` existed.
    fn argv(pipeline: &Pipeline, i: usize) -> Vec<String> {
        pipeline[i].words.iter().map(Word::raw).collect()
    }

    #[test]
    fn a_single_command_with_no_redirections() {
        let p = parse("echo hello world").unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(argv(&p, 0), ["echo", "hello", "world"]);
        assert!(p[0].redirs.is_empty());
    }

    #[test]
    fn a_two_stage_pipeline() {
        let p = parse("ls | wc").unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(argv(&p, 0), ["ls"]);
        assert_eq!(argv(&p, 1), ["wc"]);
    }

    #[test]
    fn a_three_stage_pipeline() {
        let p = parse("ls | cat | wc").unwrap();
        assert_eq!(p.len(), 3);
        assert_eq!(argv(&p, 2), ["wc"]);
    }

    #[test]
    fn a_quoted_operator_is_an_argument_not_syntax() {
        let p = parse(r#"echo "|""#).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(argv(&p, 0), ["echo", "|"]);
    }

    #[test]
    fn a_stage_of_only_redirections_parses() {
        // POSIX allows `> f` (it creates the file); running it is another matter.
        let p = parse("> f").unwrap();
        assert!(p[0].words.is_empty());
        assert_eq!(
            p[0].redirs,
            [Redirection::Out {
                fd: 1,
                path: "f".into(),
                append: false
            }]
        );
    }

    #[test]
    fn each_redirection_form() {
        let p = parse("cmd < in >> out 2>&1").unwrap();
        assert_eq!(argv(&p, 0), ["cmd"]);
        assert_eq!(
            p[0].redirs,
            [
                Redirection::In("in".into()),
                Redirection::Out {
                    fd: 1,
                    path: "out".into(),
                    append: true
                },
                Redirection::Dup { fd: 2, target: 1 },
            ]
        );
    }

    #[test]
    fn plain_out_redirection_defaults_to_fd_1() {
        let p = parse("cmd > out").unwrap();
        assert_eq!(
            p[0].redirs,
            [Redirection::Out {
                fd: 1,
                path: "out".into(),
                append: false
            }]
        );
    }

    #[test]
    fn redirections_can_be_interspersed_with_arguments() {
        let p = parse("cmd a > out b").unwrap();
        assert_eq!(argv(&p, 0), ["cmd", "a", "b"]);
        assert_eq!(
            p[0].redirs,
            [Redirection::Out {
                fd: 1,
                path: "out".into(),
                append: false
            }]
        );
    }

    #[test]
    fn a_redirection_with_nothing_after_it_is_a_missing_target() {
        for (text, want) in [
            ("cmd <", "<"),
            ("cmd >", ">"),
            ("cmd >>", ">>"),
            ("cmd >&", ">&"),
        ] {
            assert_eq!(
                parse(text),
                Err(SyntaxError::MissingTarget(want)),
                "parsing {text:?}"
            );
        }
    }

    #[test]
    fn a_dup_target_that_is_not_1_or_2_is_refused() {
        assert_eq!(parse("cmd >&3"), Err(SyntaxError::BadDupTarget));
        assert_eq!(parse("cmd >&x"), Err(SyntaxError::BadDupTarget));
    }

    #[test]
    fn an_empty_or_comment_only_line_is_an_empty_command() {
        assert_eq!(parse(""), Err(SyntaxError::EmptyCommand));
        assert_eq!(parse("  # just a comment"), Err(SyntaxError::EmptyCommand));
    }

    #[test]
    fn a_pipe_with_nothing_on_a_side_is_an_error_not_a_blank_line() {
        for line in [
            "| a",
            "a |",
            "a | | b",
            "|",
            "a | # nothing",
            "> f |",
            "| > f",
        ] {
            let result = parse(line);
            // `> f |` and `| > f` have a redirection-only stage on one side and nothing on the
            // other; either way the empty stage is what is wrong.
            assert_eq!(result, Err(SyntaxError::MissingCommand), "{line:?}");
        }
    }

    #[test]
    fn a_redirection_only_stage_is_a_command() {
        assert!(parse("a | > f").is_ok());
        assert!(parse("> f").is_ok());
    }

    #[test]
    fn a_lex_error_propagates_as_a_syntax_error() {
        assert_eq!(
            parse("a;b"),
            Err(SyntaxError::Lex(LexError::Unsupported(";")))
        );
    }
}
