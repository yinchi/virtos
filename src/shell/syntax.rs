//! Parses a lexed shell line into a `List`: `;`-separated `AndOr`s, each a `Command` (a pipeline,
//! optionally negated by `!`) followed by any number of `&&`/`||` connectors and further commands.
//! A `Pipeline` is one or more `|`-separated `Segment`s, each with its own argv and redirections.
//! Parses the *token stream* `lexer::lex` produces, not the raw line -- `parse` (below) is the
//! one public entry point, lexing internally, so callers never call `lexer::lex` themselves.
//!
//! Precedence, tightest first: `|`, then `&&`/`||` (equal, left-associative), then `;`.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::lexer::{self, LexError, Part, RedirOp, Token, Word};

/// One redirection a `Segment` applies before its program starts. A file target is a `Word`,
/// expanded when the command runs (variables and command substitutions, but never split: it must
/// come out as exactly one word, `shell::expand::expand_target`); `>&`'s target is plain text,
/// checked here to be `1` or `2`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redirection {
    In(Word),
    Out { fd: u8, path: Word, append: bool },
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

/// A pipeline and whether a leading `!` negates its status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub negated: bool,
    pub pipeline: Pipeline,
}

/// Which status lets the command after a connector run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connector {
    /// `&&`: the previous status was 0.
    And,
    /// `||`: the previous status was not 0.
    Or,
}

/// `first (connector command)*`, evaluated left to right: each connector looks at the status of
/// everything to its left, and a command it skips leaves that status as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndOr {
    pub first: Command,
    pub rest: Vec<(Connector, Command)>,
}

/// What one line is: `AndOr`s run one after another, whatever each one's status.
pub type List = Vec<AndOr>;

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
    /// a stray `|`, `&&`, `||` or `;` with nothing on one side of it (`| a`, `a | | b`, `; a`,
    /// `a ; ; b`, `&& a`).
    MissingCommand,
    /// The line ends right after `|`, `&&` or `||`: the command it needs is not there (yet -- a
    /// continuation prompt would ask for it).
    Incomplete(&'static str),
}

/// Lexes then parses `line` into a `List`.
pub fn parse(line: &str) -> Result<List, SyntaxError> {
    let tokens = lexer::lex(line).map_err(SyntaxError::Lex)?;
    if tokens.is_empty() {
        return Err(SyntaxError::EmptyCommand);
    }
    let list = grammar::list(&SliceByRef(&tokens)).map_err(decode_error)?;
    validate_substitutions(&list)?;
    Ok(list)
}

/// Parses the inside of every command substitution in `list`, only to find a syntax error of the
/// whole line now rather than when the substitution runs (after the commands before it did): a
/// line is parsed before any of it runs, as a shell's is. The results are thrown away -- the
/// executor parses a substitution's text again when it expands the word -- and an empty one
/// (`$()`) is fine, just an empty string.
fn validate_substitutions(list: &List) -> Result<(), SyntaxError> {
    let words = list
        .iter()
        .flat_map(|and_or| {
            core::iter::once(&and_or.first).chain(and_or.rest.iter().map(|(_, c)| c))
        })
        .flat_map(|command| command.pipeline.iter())
        .flat_map(|segment| {
            let targets = segment.redirs.iter().filter_map(|r| match r {
                Redirection::In(word) | Redirection::Out { path: word, .. } => Some(word),
                Redirection::Dup { .. } => None,
            });
            segment.words.iter().chain(targets)
        });
    for word in words {
        for part in &word.parts {
            if let Part::Subst { text, .. } = part {
                match parse(text) {
                    Ok(_) | Err(SyntaxError::EmptyCommand) => {}
                    Err(e) => return Err(e),
                }
            }
        }
    }
    Ok(())
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
    // The named failures the parser tried at the furthest position, most specific first: a line
    // that ends in `&&` also "expected a command", and the more useful message is the first.
    let expected: alloc::vec::Vec<&str> = e.expected.tokens().collect();
    let has = |name: &str| expected.contains(&name);
    for (name, error) in [
        (
            "missing-target-in",
            SyntaxError::MissingTarget(RedirOp::In.text()),
        ),
        (
            "missing-target-out",
            SyntaxError::MissingTarget(RedirOp::Out.text()),
        ),
        (
            "missing-target-append",
            SyntaxError::MissingTarget(RedirOp::Append.text()),
        ),
        (
            "missing-target-dup",
            SyntaxError::MissingTarget(RedirOp::DupOut.text()),
        ),
        ("bad-dup-target", SyntaxError::BadDupTarget),
        ("incomplete-pipe", SyntaxError::Incomplete("|")),
        ("incomplete-and", SyntaxError::Incomplete("&&")),
        ("incomplete-or", SyntaxError::Incomplete("||")),
        ("empty-command", SyntaxError::MissingCommand),
    ] {
        if has(name) {
            return error;
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
            = [Token::Redir { fd, op: RedirOp::In }] path:word() { Redirection::In(path) }
            / [Token::Redir { op: RedirOp::In, .. }] {? Err("missing-target-in") }
            / [Token::Redir { fd, op: RedirOp::Out }] path:word() {
                Redirection::Out { fd: fd.unwrap_or(1), path, append: false }
              }
            / [Token::Redir { op: RedirOp::Out, .. }] {? Err("missing-target-out") }
            / [Token::Redir { fd, op: RedirOp::Append }] path:word() {
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

        /// A `!` that is not negating anything (anywhere but the start of a pipeline) is just the
        /// word `!`.
        rule bang_word() -> Word
            = [Token::Bang] { Word { parts: vec![Part::Lit("!".into())], assign: None } }

        rule item() -> Item
            = w:word() { Item::Arg(w) }
            / w:bang_word() { Item::Arg(w) }
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

        // A trailing `|` is "incomplete", not "an empty stage": it is what a continuation prompt
        // would ask the next line for. (A `|` followed by anything else that is not a command
        // is still the empty-stage failure from `segment`.)
        rule trailing_pipe() = [Token::Pipe] ![_] {? Err("incomplete-pipe") }

        rule pipeline() -> Pipeline
            = s0:segment() rest:([Token::Pipe] s:segment() { s })* trailing_pipe()? {
                let mut v = Vec::with_capacity(1 + rest.len());
                v.push(s0);
                v.extend(rest);
                v
              }

        rule command() -> Command
            = bangs:[Token::Bang]* p:pipeline() {
                Command { negated: bangs.len() % 2 == 1, pipeline: p }
              }

        rule connector() -> Connector
            = [Token::AndIf] { Connector::And }
            / [Token::OrIf] { Connector::Or }

        rule trailing_connector() = [Token::AndIf] ![_] {? Err("incomplete-and") }
                                  / [Token::OrIf] ![_] {? Err("incomplete-or") }

        rule and_or() -> AndOr
            = first:command() rest:(c:connector() x:command() { (c, x) })* trailing_connector()? {
                AndOr { first, rest }
              }

        pub rule list() -> List
            = first:and_or() rest:([Token::Semi] a:and_or() { a })* [Token::Semi]? {
                let mut v = Vec::with_capacity(1 + rest.len());
                v.push(first);
                v.extend(rest);
                v
              }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str) -> Word {
        Word {
            parts: vec![Part::Lit(text.into())],
            assign: None,
        }
    }

    /// The pipeline of a line that is exactly one non-negated command -- what every test below
    /// that predates lists means by `parse`.
    fn parse(line: &str) -> Result<Pipeline, SyntaxError> {
        let mut list = super::parse(line)?;
        assert_eq!(list.len(), 1, "{line:?} is not a single and-or");
        let and_or = list.remove(0);
        assert!(and_or.rest.is_empty(), "{line:?} has connectors");
        assert!(!and_or.first.negated, "{line:?} is negated");
        Ok(and_or.first.pipeline)
    }

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
                path: w("f"),
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
                Redirection::In(w("in")),
                Redirection::Out {
                    fd: 1,
                    path: w("out"),
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
                path: w("out"),
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
                path: w("out"),
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
        for line in ["| a", "a | | b", "|", "| > f"] {
            assert_eq!(parse(line), Err(SyntaxError::MissingCommand), "{line:?}");
        }
        // A pipe at the very end is the line asking for more, not a stray one.
        for line in ["a |", "a | # nothing", "> f |"] {
            assert_eq!(parse(line), Err(SyntaxError::Incomplete("|")), "{line:?}");
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
            parse("a;;b"),
            Err(SyntaxError::Lex(LexError::Unsupported(";;")))
        );
    }

    // ---- lists: `;`, `&&`, `||`, `!` ----

    fn names(c: &Command) -> Vec<Vec<String>> {
        c.pipeline
            .iter()
            .map(|s| s.words.iter().map(Word::raw).collect())
            .collect()
    }

    fn one(line: &str) -> AndOr {
        let mut list = super::parse(line).unwrap();
        assert_eq!(list.len(), 1, "{line:?}");
        list.remove(0)
    }

    #[test]
    fn semicolons_make_a_list() {
        let list = super::parse("a; b ;c").unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(names(&list[1].first), [["b"]]);
        assert!(list.iter().all(|a| a.rest.is_empty()));
    }

    #[test]
    fn a_trailing_semicolon_is_allowed() {
        assert_eq!(super::parse("a;").unwrap().len(), 1);
        assert_eq!(super::parse("a; b;").unwrap().len(), 2);
    }

    #[test]
    fn connectors_are_left_associative_and_bind_tighter_than_semicolons() {
        let list = super::parse("a && b || c ; d").unwrap();
        assert_eq!(list.len(), 2);
        let first = &list[0];
        assert_eq!(names(&first.first), [["a"]]);
        assert_eq!(first.rest.len(), 2);
        assert_eq!(first.rest[0].0, Connector::And);
        assert_eq!(first.rest[1].0, Connector::Or);
        assert_eq!(names(&first.rest[1].1), [["c"]]);
    }

    #[test]
    fn a_pipe_binds_tighter_than_a_connector() {
        let a = one("a | b && c | d");
        assert_eq!(names(&a.first), [["a"], ["b"]]);
        assert_eq!(names(&a.rest[0].1), [["c"], ["d"]]);
    }

    #[test]
    fn bang_negates_the_pipeline_after_it() {
        assert!(one("! a | b").first.negated);
        assert_eq!(names(&one("! a | b").first), [["a"], ["b"]]);
        let a = one("a && ! b");
        assert!(!a.first.negated && a.rest[0].1.negated);
        assert!(!one("! ! a").first.negated); // twice is not at all
    }

    #[test]
    fn a_bang_that_is_not_first_is_an_argument() {
        let p = parse("echo !").unwrap();
        assert_eq!(argv(&p, 0), ["echo", "!"]);
        let p = parse("a | ! b").unwrap();
        assert_eq!(argv(&p, 1), ["!", "b"]);
    }

    #[test]
    fn a_connector_or_semicolon_with_nothing_beside_it_is_an_error() {
        for line in [
            "; a",
            "a ; ; b",
            "&& a",
            "|| a",
            "a && && b",
            "a && ; b",
            "!",
            "a && !",
        ] {
            let result = super::parse(line);
            assert!(
                matches!(
                    result,
                    Err(SyntaxError::MissingCommand | SyntaxError::Incomplete(_))
                ),
                "{line:?}: {result:?}"
            );
        }
        assert_eq!(super::parse("; a"), Err(SyntaxError::MissingCommand));
        assert_eq!(super::parse("a ; ; b"), Err(SyntaxError::MissingCommand));
        assert_eq!(super::parse("&& a"), Err(SyntaxError::MissingCommand));
    }

    #[test]
    fn a_line_ending_in_a_connector_is_incomplete() {
        assert_eq!(super::parse("a &&"), Err(SyntaxError::Incomplete("&&")));
        assert_eq!(super::parse("a ||"), Err(SyntaxError::Incomplete("||")));
        assert_eq!(super::parse("a && b |"), Err(SyntaxError::Incomplete("|")));
    }

    #[test]
    fn redirections_belong_to_their_own_command() {
        let a = one("a > x && b < y");
        assert_eq!(a.first.pipeline[0].redirs.len(), 1);
        assert_eq!(a.rest[0].1.pipeline[0].redirs, [Redirection::In(w("y"))]);
    }

    // ---- command substitution ----

    #[test]
    fn a_substitution_is_part_of_its_word() {
        let p = parse("echo a$(b c)d").unwrap();
        assert_eq!(p[0].words.len(), 2);
        assert_eq!(
            p[0].words[1].parts,
            [
                Part::Lit("a".into()),
                Part::Subst {
                    text: "b c".into(),
                    quoted: false
                },
                Part::Lit("d".into()),
            ]
        );
    }

    #[test]
    fn a_syntax_error_inside_a_substitution_is_an_error_of_the_line() {
        // Found before anything on the line runs, as a shell parses the whole line first.
        assert_eq!(
            super::parse("echo a ; echo $(echo b &&)"),
            Err(SyntaxError::Incomplete("&&"))
        );
        assert_eq!(
            super::parse("echo $(; a)"),
            Err(SyntaxError::MissingCommand)
        );
        assert_eq!(
            super::parse("echo $(echo >)"),
            Err(SyntaxError::MissingTarget(">"))
        );
        assert_eq!(
            super::parse("echo `a |`"),
            Err(SyntaxError::Incomplete("|"))
        );
    }

    #[test]
    fn an_empty_substitution_is_fine_and_nests_are_checked() {
        assert!(super::parse("echo $() `` \"$()\"").is_ok());
        assert!(super::parse("echo $(echo $(echo hi))").is_ok());
        assert_eq!(
            super::parse("echo $(echo $(a &&))"),
            Err(SyntaxError::Incomplete("&&"))
        );
    }

    #[test]
    fn a_redirection_target_may_be_a_substitution() {
        let p = parse("cat < $(echo f) > $T").unwrap();
        assert!(
            matches!(&p[0].redirs[0], Redirection::In(w) if matches!(w.parts[0], Part::Subst { .. }))
        );
        assert!(
            matches!(&p[0].redirs[1], Redirection::Out { path, .. } if matches!(path.parts[0], Part::Var { .. }))
        );
    }
}
