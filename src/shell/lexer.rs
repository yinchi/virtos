//! Splits a shell command line into tokens: words (honoring quoting, escaping, `$NAME`/`${NAME}`/
//! `$?` expansion markers, and `NAME=...`-shaped assignment words), `|`, and redirection operators.
//! `plans/USERSPACE_PLAN.md`'s step 10 note ("hand-roll the tokenizer; don't reach for `shlex`") is
//! the actual lesson here: `shlex::split`-style tokenizing throws away whether a word was quoted or
//! escaped, so `echo "|"` would misparse as a pipe and `a>b` would stay one word. Quoting/escaping
//! is decided while scanning here instead (via a `peg` grammar, same as `syntax.rs`), so a quoted
//! or escaped character is never mistaken for an operator -- not by post-hoc checking a flag, but
//! because the grammar itself only ever recognizes `|`/`<`/`>`/etc. as operators outside of any
//! quote or escape in the first place.
//!
//! A word doesn't resolve to a plain `String` here -- it's a [`Word`], a sequence of [`Part`]s
//! (literal text, or a `$NAME`/`${NAME}`/`$?` reference, each knowing whether it came from inside
//! double quotes) plus `assign`, the byte offset of `=` if this word is shaped like `NAME=...`
//! (matches the reference lab's own `Word` exactly). Actually substituting a variable's value, and
//! deciding whether a `NAME=...`-shaped word in *this* position really counts as an assignment
//! (only a leading run of them does), are both jobs for later, higher layers (`shell::expand`,
//! `shell::mod`'s own assignment-prefix handling) -- this module only recognizes the shape.
//!
//! The rules:
//! - A word is what lies between blanks. Single quotes are fully literal -- not just escaping, but
//!   expansion too: `$x` inside single quotes is never a variable reference. Inside double quotes
//!   everything is literal except that a backslash escapes `"`, `\`, `$` and a backtick (so `"\$"`
//!   is a literal `$`, not the start of an expansion), and `$NAME`/`${NAME}`/`$?` are recognized.
//!   An unquoted backslash makes the next character literal (so `\$` outside quotes is a literal
//!   `$` too). Quoting or escaping any part of a word no longer marks the whole word in one flag
//!   the way it used to -- that's now implicit per-expansion (`Part::Var`/`Status`'s own `quoted`).
//! - Outside single quotes and escapes, `$NAME` (letters/digits/underscore, not starting with a
//!   digit) and `${NAME}` are variable references; `$?`/`${?}` is the last line/pipeline's own
//!   exit status. A `$` followed by anything else -- a digit, another `$`, end of input -- stays a
//!   literal `$`: this project has no positional parameters or process ids to name (matches the
//!   reference lab's own documented choice). `${` with no matching `NAME}` is
//!   `LexError::BadSubstitution`.
//! - A word whose text starts with a valid identifier followed by `=` (the identifier must be
//!   literal -- an expansion can never supply the name half) is marked `assign: Some(offset)`.
//! - `#` starts a comment only at the start of a word: `echo a#b` prints `a#b`, `echo a #b`
//!   prints `a`.
//! - `|`, `<`, `>`, `>>` and `>&` end a word and are operators. An unquoted, unescaped word that
//!   is just the digit `1` or `2` (or `0` before `<`), *immediately* followed by an operator, is
//!   that operator's file descriptor number (`2>err`, `2>>err`, `2>&1`); anywhere else digits
//!   are ordinary text (`a2>x` is the word `a2` and `>x`; `echo 2 >x` echoes `2`; a quoted
//!   `"2">x` is a word; `3>x` is the word `3` and `>x`; `2<x`/`0>x` don't count either, since 2
//!   isn't an input descriptor and 0 isn't an output one).
//! - Backtick, `*`, `?`, `~`, `{` and `}` (outside `${`) are still ordinary characters: command
//!   substitution and globbing aren't implemented.
//! - `;`, `&`, `(`, `)` and here-documents (`<<`, `<&`) are not supported, and are refused
//!   rather than taken for text.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use super::frame_stack::is_valid_name;

/// A redirection operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirOp {
    /// `<`
    In,
    /// `>`
    Out,
    /// `>>`
    Append,
    /// `>&`: duplicate an output descriptor onto another (`2>&1`).
    DupOut,
}

impl RedirOp {
    /// The operator as typed, for messages.
    pub fn text(self) -> &'static str {
        match self {
            RedirOp::In => "<",
            RedirOp::Out => ">",
            RedirOp::Append => ">>",
            RedirOp::DupOut => ">&",
        }
    }
}

/// One piece of a [`Word`]: literal text, or an expansion -- a `$NAME`/`${NAME}` variable
/// reference, or `$?` (the last line/pipeline's own exit status). `Lit` carries no `quoted` flag
/// of its own: blank-separated word-splitting already happened here, at lex time (that's the
/// whole reason a `Word` is a sequence of parts rather than one string); only an *expansion's own
/// result*, resolved later against live state, can still need splitting, which is exactly what
/// each `Var`/`Status`'s own `quoted` decides (`shell::expand`'s own doc comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Lit(String),
    Var { name: String, quoted: bool },
    Status { quoted: bool },
}

/// A word: a run of adjacent [`Part`]s, in lexical order, plus `assign` -- the byte offset of `=`
/// within `parts[0]` if this word's own text is shaped like `NAME=...` (`detect_assign`), `None`
/// otherwise. Marked unconditionally, regardless of where the word sits in its segment: deciding
/// whether that *counts* as a real assignment (only a leading run of such words does) is a later,
/// higher layer's job, not this one's -- matches the reference lab's own `Word` exactly, including
/// the division of labor.
///
/// `parts` is empty only for an explicitly empty quoted word (`""`/`''`) -- `unquoted()`'s own
/// grammar rule can never match zero characters, so that's the only way this arises, and it always
/// means "one empty field," never "nothing at all" (`shell::expand`'s own doc comment on the
/// distinction).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Word {
    pub parts: Vec<Part>,
    /// Read by `assignment()` below -- `shell::split_assignments`'s own call, for a segment's
    /// leading `NAME=...` words.
    pub assign: Option<usize>,
}

impl Word {
    /// Reconstructs this word as plain text, rendering a `Var`/`Status` part back as `$name`/`$?`
    /// literally rather than expanding it. Used only by the two call sites that don't implement
    /// expansion at all yet -- a redirect's own target, and `>&`'s dup-target check
    /// (`syntax::grammar`) -- so `echo x > $FILE` stays unimplemented (`$FILE` taken as a literal
    /// five-character name) rather than silently doing the wrong thing.
    pub fn raw(&self) -> String {
        let mut s = String::new();
        for part in &self.parts {
            match part {
                Part::Lit(t) => s.push_str(t),
                Part::Var { name, .. } => {
                    s.push('$');
                    s.push_str(name);
                }
                Part::Status { .. } => s.push_str("$?"),
            }
        }
        s
    }

    /// If `assign` says this word is shaped like `NAME=VALUE`, splits it into the name and a new
    /// `Word` for everything after the `=` (itself never `assign`-marked -- a value is never
    /// re-examined for its own leading `NAME=`). `None` if `assign` is `None`.
    pub fn assignment(&self) -> Option<(&str, Word)> {
        let i = self.assign?;
        let Some(Part::Lit(first)) = self.parts.first() else {
            unreachable!("detect_assign only ever sets `assign` when parts[0] is a Lit")
        };
        let name = &first[..i];
        let mut value_parts = Vec::with_capacity(self.parts.len());
        let rest = &first[i + 1..];
        if !rest.is_empty() {
            value_parts.push(Part::Lit(rest.to_string()));
        }
        value_parts.extend(self.parts[1..].iter().cloned());
        Some((
            name,
            Word {
                parts: value_parts,
                assign: None,
            },
        ))
    }
}

/// `parts[0]`'s own text, if it's a `Lit` shaped like `NAME=...` with `NAME` a valid identifier --
/// the byte offset of the `=`. A word starting with an expansion (`$FOO=bar`) is never an
/// assignment: the name half must be literal, matching the reference lab's own rule (and simple
/// common sense -- there's nothing to look up yet to decide what the name even is).
fn detect_assign(parts: &[Part]) -> Option<usize> {
    let Some(Part::Lit(text)) = parts.first() else {
        return None;
    };
    let eq = text.find('=')?;
    is_valid_name(&text[..eq]).then_some(eq)
}

/// One token `lex` produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Word(Word),
    Pipe,
    /// `;`: ends one pipeline-list entry and begins the next.
    Semi,
    /// `&&`: run the next pipeline only if the last one succeeded.
    AndIf,
    /// `||`: run the next pipeline only if the last one failed.
    OrIf,
    /// An unquoted `!` standing alone: negates the pipeline it starts. Anywhere but the start of a
    /// pipeline the parser takes it back as the ordinary one-character word `!`.
    Bang,
    /// A redirection operator and the descriptor number typed right before it, if any.
    Redir {
        fd: Option<u8>,
        op: RedirOp,
    },
}

/// Why `lex` refused a line outright.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LexError {
    /// An unclosed quote or a trailing, unescaped backslash -- `"quote"` or `"escape"`.
    Unterminated(&'static str),
    /// A real shell operator this project doesn't implement: `;;`, `&`, `(`, `)`, `<<`, `<&`.
    Unsupported(&'static str),
    /// `${` with no matching `NAME}` (or `${?}`'s own `?`) -- bash's own "bad substitution".
    BadSubstitution,
}

/// Scans already-matched text -- from inside unquoted or double-quoted context; single-quoted and
/// escaped text never reach this at all, so a `$` there is never special in the first place -- for
/// `$NAME`, `${NAME}`/`${?}` and `$?`, returning the resulting parts in order. A `$` followed by
/// anything else (a digit, another `$`, end of input) stays a literal `$`: this project has no
/// positional parameters or `$$` to name (matches the reference lab's own documented choice).
/// `${` with no matching `NAME}` is `LexError::BadSubstitution`.
fn scan_expansions(text: &str, quoted: bool) -> Result<Vec<Part>, LexError> {
    let mut parts = Vec::new();
    let mut lit = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            lit.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('?') => {
                chars.next();
                if !lit.is_empty() {
                    parts.push(Part::Lit(core::mem::take(&mut lit)));
                }
                parts.push(Part::Status { quoted });
            }
            Some('{') => {
                chars.next();
                let mut name = String::new();
                let mut closed = false;
                for c2 in chars.by_ref() {
                    if c2 == '}' {
                        closed = true;
                        break;
                    }
                    name.push(c2);
                }
                if !closed {
                    return Err(LexError::BadSubstitution);
                }
                if !lit.is_empty() {
                    parts.push(Part::Lit(core::mem::take(&mut lit)));
                }
                if name == "?" {
                    parts.push(Part::Status { quoted });
                } else if is_valid_name(&name) {
                    parts.push(Part::Var { name, quoted });
                } else {
                    return Err(LexError::BadSubstitution);
                }
            }
            Some(c2) if c2.is_ascii_alphabetic() || c2 == '_' => {
                let mut name = String::new();
                while let Some(&c3) = chars.peek() {
                    if c3.is_ascii_alphanumeric() || c3 == '_' {
                        name.push(c3);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if !lit.is_empty() {
                    parts.push(Part::Lit(core::mem::take(&mut lit)));
                }
                parts.push(Part::Var { name, quoted });
            }
            _ => lit.push('$'), // not followed by anything expansion-eligible -- literal `$`
        }
    }
    if !lit.is_empty() {
        parts.push(Part::Lit(lit));
    }
    Ok(parts)
}

/// Merges adjacent `Part::Lit` entries (which arise constantly -- e.g. a word with no expansion in
/// it at all is just one `Lit` after this, not one per grammar sub-rule that happened to
/// contribute to it) -- cosmetic only, `expand`'s own output is identical either way, but a
/// `Word` with the fewest parts that still means the same thing is easier to read in a test
/// failure or a debug print, and is what `detect_assign` expects (`parts[0]` must be one whole
/// `Lit` up to its own `=`, not split across several).
fn merge_lits(parts: Vec<Part>) -> Vec<Part> {
    let mut merged: Vec<Part> = Vec::with_capacity(parts.len());
    for part in parts {
        if let (Some(Part::Lit(prev)), Part::Lit(next)) = (merged.last_mut(), &part) {
            prev.push_str(next);
        } else {
            merged.push(part);
        }
    }
    merged
}

/// One escaped-or-raw character from inside a double-quoted string -- kept distinct until the
/// whole quoted span is assembled (`assemble_dquoted`) specifically so an *escaped* `$` (from
/// `"\$"`) never ends up concatenated with adjacent raw text before `scan_expansions` runs: were
/// that allowed, `"\$FOO"` would wrongly re-combine into the literal text `$FOO` and then be
/// *rescanned* as if it were an unescaped reference to `$FOO`, exactly the bug this type exists to
/// avoid. A raw run (`Raw`) is scanned for expansions; an escaped char (`Esc`) becomes a literal
/// `Part` on its own, isolated from whatever raw text sits next to it on either side.
enum DChar {
    Raw(char),
    Esc(char),
}

/// Walks `chars` (one double-quoted string's content, in order), scanning each maximal run of
/// `Raw` characters for `$`-expansions (`scan_expansions`, always `quoted: true` here) while
/// `Esc` characters become their own isolated literal part -- see `DChar`'s own doc comment for
/// why the isolation matters.
fn assemble_dquoted(chars: Vec<DChar>) -> Result<Vec<Part>, LexError> {
    let mut parts = Vec::new();
    let mut raw = String::new();
    for c in chars {
        match c {
            DChar::Raw(c) => raw.push(c),
            DChar::Esc(c) => {
                if !raw.is_empty() {
                    parts.extend(scan_expansions(&core::mem::take(&mut raw), true)?);
                }
                parts.push(Part::Lit(c.to_string()));
            }
        }
    }
    if !raw.is_empty() {
        parts.extend(scan_expansions(&raw, true)?);
    }
    Ok(parts)
}

peg::parser! {
    grammar grammar() for str {
        rule blank()
            = [' ' | '\t' | '\x0B' | '\x0C' | '\r' | '\n']

        // Characters excluded from an unquoted word: either an operator (its own rule
        // elsewhere) or refused outright (";", "&", "(", ")" have no rule anywhere else here).
        rule unquoted_start()
            = !['"' | '\'' | '\\' | '|' | '<' | '>' | '#' | ';' | '&' | '(' | ')'
                | ' ' | '\t' | '\x0B' | '\x0C' | '\r' | '\n'] [_]
        // "#" may continue a word already started but is excluded from unquoted_start, so it
        // can never start one -- at a fresh position only `comment` matches "#".
        rule unquoted_tail_char() = unquoted_start() / "#"
        rule unquoted() -> Vec<Part>
            = s:$(unquoted_start() unquoted_tail_char()*) {?
                scan_expansions(s, false).map_err(decode_as_token)
              }

        rule squote_char() = !"'" [_]
        rule squoted() -> Vec<Part>
            = "'" s:$(squote_char()*) "'" { vec![Part::Lit(s.to_string())] }
            / "'" squote_char()* {? Err("unterminated-quote") }
        // No escaping or expansion inside single quotes.

        rule dquote_escape() -> DChar = "\\" c:['"' | '\\' | '$' | '`'] { DChar::Esc(c) }
        rule dquote_char() -> DChar = dquote_escape() / (!"\"" c:[_] { DChar::Raw(c) })
        // A backslash not immediately before one of the four specials falls through to the
        // catch-all and is kept, as a raw character, by the *next* dquote_char -- pushing a lone
        // '\\' and letting the following character (or end of input) be handled normally on the
        // next iteration, rather than needing a dedicated "backslash but not before a special"
        // rule.
        rule dquoted() -> Vec<Part>
            = "\"" s:dquote_char()* "\"" {? assemble_dquoted(s).map_err(decode_as_token) }
            / "\"" dquote_char()* {? Err("unterminated-quote") }

        rule escape_piece() -> Vec<Part>
            = "\\" c:[_] { vec![Part::Lit(c.to_string())] }
            / "\\" {? Err("unterminated-escape") }
        // Outside any quote, a backslash escapes exactly the next character, whatever it is --
        // including `$`, which this isolates from `tail`'s own trailing `unquoted_tail_char`
        // text the same way `assemble_dquoted` isolates an escaped `$` inside double quotes:
        // `piece` (below) and the raw text after it are two separate grammar productions, never
        // concatenated into one string before either is scanned for expansions.

        rule piece() -> Vec<Part> = dquoted() / squoted() / escape_piece()
        rule tail() -> Vec<Part>
            = p:piece() s:$(unquoted_tail_char()*) {?
                let mut parts = p;
                if !s.is_empty() {
                    parts.extend(scan_expansions(s, false).map_err(decode_as_token)?);
                }
                Ok(parts)
              }

        // A word is unquoted text with zero or more (piece, then more text) pairs after it, or
        // -- if it starts with a piece instead -- one or more of those same pairs.
        rule word_token() -> Word
            = u:unquoted() ts:tail()* {
                let mut parts = u;
                for t in ts { parts.extend(t); }
                let parts = merge_lits(parts);
                let assign = detect_assign(&parts);
                Word { parts, assign }
              }
            / ts:tail()+ {
                let mut parts = Vec::new();
                for t in ts { parts.extend(t); }
                let parts = merge_lits(parts);
                let assign = detect_assign(&parts);
                Word { parts, assign }
              }

        rule comment() = "#" [_]*

        // A bare "0"/"1"/"2" pairs with an operator only when immediately followed by the one
        // it belongs to; a longer digit run, or a digit not immediately adjacent to an
        // operator, is left for `word_token` -- so every one of these is tried *before*
        // word_token, or word_token would claim a lone eligible digit as its own word before
        // the operator rule ever saw it.
        rule digit0() -> char = ['0']
        rule digit12() -> char = ['1' | '2']

        // "<<" and "<&" are refused outright, regardless of what (if anything) precedes them --
        // listed before in_redir, or in_redir would consume just the fd digit and the first
        // "<", leaving a stray second character to be mis-parsed as its own token.
        rule op_heredoc() -> Token = digit0()? "<<" {? Err("op-heredoc") }
        rule op_dupin() -> Token = digit0()? "<&" {? Err("op-dupin") }
        // The lookahead matters, not just the ordering above: `{? Err(...) }` failing only makes
        // *that* alternative backtrack, it doesn't abort the parse -- without this, "<<" would
        // fail op_heredoc and then silently succeed here as a bare "<", one character short of
        // the truth, with the stray second "<" or "&" left to be mis-parsed as its own token
        // right afterward.
        rule in_redir() -> Token
            = d:digit0()? "<" !['<' | '&'] { Token::Redir { fd: d.map(|c| c as u8 - b'0'), op: RedirOp::In } }

        // append_redir/dup_redir listed before out_redir for the same reason: ">" is a strict
        // prefix of both, so trying it first would steal their match.
        rule append_redir() -> Token
            = d:digit12()? ">>" { Token::Redir { fd: d.map(|c| c as u8 - b'0'), op: RedirOp::Append } }
        rule dup_redir() -> Token
            = d:digit12()? ">&" { Token::Redir { fd: d.map(|c| c as u8 - b'0'), op: RedirOp::DupOut } }
        rule out_redir() -> Token
            = d:digit12()? ">" { Token::Redir { fd: d.map(|c| c as u8 - b'0'), op: RedirOp::Out } }

        // `;;` ends a `case` branch, which this shell has no use for yet: refused by name rather
        // than lexed as two `;`, so its error says what was typed.
        rule op_dsemi() -> Token = ";;" {? Err("op-dsemi") }
        // The lookahead matters for the reason `in_redir`'s does: `op_dsemi`'s refusal only makes
        // that alternative backtrack, so without it `;;` would lex as two `;`.
        rule semi() -> Token = ";" !";" { Token::Semi }
        // "&&" is listed before the lone "&" below, which is still refused (job control).
        rule and_if() -> Token = "&&" { Token::AndIf }
        rule op_amp() -> Token = "&" {? Err("op-amp") }
        rule op_lparen() -> Token = "(" {? Err("op-lparen") }
        rule op_rparen() -> Token = ")" {? Err("op-rparen") }

        // "||" before "|": the latter is a strict prefix of it.
        rule or_if() -> Token = "||" { Token::OrIf }
        rule pipe() -> Token = "|" { Token::Pipe }

        // A `!` is the negation operator only as a whole word of its own: followed by a blank,
        // an operator or the end of the line. `!x` and `a!` are ordinary words.
        rule bang() -> Token
            = "!" &(blank() / ['|' | '<' | '>' | '&' | ';' | '(' | ')'] / ![_]) { Token::Bang }

        rule token() -> Token
            = op_heredoc()
            / op_dupin()
            / in_redir()
            / append_redir()
            / dup_redir()
            / out_redir()
            / op_dsemi()
            / semi()
            / and_if()
            / op_amp()
            / op_lparen()
            / op_rparen()
            / or_if()
            / pipe()
            / bang()
            / w:word_token() { Token::Word(w) }

        // Blanks and comments are just two more no-op alternatives in the same repeated choice
        // as a real token -- not a separate skip-whitespace wrapper around `token()`, which is
        // what would make a `#` outside of a word ambiguous with one already claimed as part of
        // a word by `unquoted_tail_char`. Filtering `None`s out afterward keeps `token()` itself
        // from needing to know anything about either.
        pub rule tokens() -> Vec<Token>
            = ts:(t:token() {Some(t)} / blank() {None} / comment() {None})* {
                ts.into_iter().flatten().collect()
              }
    }
}

/// Translates a `LexError` into the one sentinel string `BadSubstitution` always uses
/// (`decode_error`'s own doc comment explains the convention) -- `scan_expansions`/
/// `assemble_dquoted` can only ever fail with `BadSubstitution` (the `Result` type is shared with
/// `LexError` generally, but an unterminated quote/escape is caught by the grammar itself, not by
/// either of these two functions).
fn decode_as_token(_: LexError) -> &'static str {
    "bad-substitution"
}

/// Splits `line` into tokens, in order. An empty or comment-only line (`#` at the start of a
/// word runs to the end of the line) produces an empty list, not an error.
pub fn lex(line: &str) -> Result<Vec<Token>, LexError> {
    grammar::tokens(line).map_err(decode_error)
}

/// Translates a `peg::ParseError` back into the specific `LexError` its own `{? Err("...") }`
/// action reported -- `peg`'s own error tracking keeps the ones tried at the furthest position
/// reached, regardless of which alternative the parser backtracked through afterward, so decoding
/// is just matching the string back to its variant -- never reached for any other reason, since
/// every character this grammar doesn't otherwise recognize as an operator, quote, or expansion
/// falls into an ordinary literal.
fn decode_error(e: peg::error::ParseError<peg::str::LineCol>) -> LexError {
    for token in e.expected.tokens() {
        match token {
            "unterminated-quote" => return LexError::Unterminated("quote"),
            "unterminated-escape" => return LexError::Unterminated("escape"),
            "bad-substitution" => return LexError::BadSubstitution,
            "op-heredoc" => return LexError::Unsupported("<<"),
            "op-dupin" => return LexError::Unsupported("<&"),
            "op-dsemi" => return LexError::Unsupported(";;"),
            "op-amp" => return LexError::Unsupported("&"),
            "op-lparen" => return LexError::Unsupported("("),
            "op-rparen" => return LexError::Unsupported(")"),
            _ => {}
        }
    }
    unreachable!("lexer grammar always tags its own failures with one of the strings above")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(parts: Vec<Part>) -> Token {
        let assign = detect_assign(&parts);
        Token::Word(Word { parts, assign })
    }
    fn w(text: &str) -> Token {
        word(vec![Part::Lit(text.into())])
    }
    fn var(name: &str, quoted: bool) -> Part {
        Part::Var {
            name: name.into(),
            quoted,
        }
    }
    fn status(quoted: bool) -> Part {
        Part::Status { quoted }
    }
    fn r(fd: Option<u8>, op: RedirOp) -> Token {
        Token::Redir { fd, op }
    }
    fn lexed(line: &str) -> Vec<Token> {
        lex(line).unwrap()
    }

    #[test]
    fn words_are_split_on_blanks() {
        assert_eq!(lexed("echo a b"), [w("echo"), w("a"), w("b")]);
        assert_eq!(lexed("  echo \t a   "), [w("echo"), w("a")]);
        assert_eq!(lexed(""), []);
        assert_eq!(lexed("   "), []);
    }

    #[test]
    fn quotes_group_and_mark_the_word() {
        assert_eq!(lexed(r#"echo "a b""#), [w("echo"), w("a b")]);
        assert_eq!(lexed("echo 'a b'"), [w("echo"), w("a b")]);
        assert_eq!(lexed(r#"echo a"b c"d"#), [w("echo"), w("ab cd")]);
        assert_eq!(lexed(r#"echo """#), [w("echo"), word(vec![])]); // an empty word
    }

    #[test]
    fn quoted_operators_are_text() {
        assert_eq!(lexed(r#"echo "|""#), [w("echo"), w("|")]);
        assert_eq!(lexed("echo '|'"), [w("echo"), w("|")]);
        assert_eq!(
            lexed(r#"echo ">" '<' ">>""#),
            [w("echo"), w(">"), w("<"), w(">>")]
        );
        assert_eq!(
            lexed(r#"echo ";" "&" "(""#),
            [w("echo"), w(";"), w("&"), w("(")]
        );
    }

    #[test]
    fn single_quotes_are_fully_literal() {
        assert_eq!(lexed(r"echo 'a\b'"), [w("echo"), w(r"a\b")]);
        assert_eq!(lexed(r#"echo 'a"b'"#), [w("echo"), w(r#"a"b"#)]);
        assert_eq!(lexed("echo '$x `y`'"), [w("echo"), w("$x `y`")]);
    }

    #[test]
    fn double_quotes_escape_only_four_characters() {
        assert_eq!(lexed(r#"echo "a\"b""#), [w("echo"), w(r#"a"b"#)]);
        assert_eq!(lexed(r#"echo "a\\b""#), [w("echo"), w(r"a\b")]);
        assert_eq!(lexed(r#"echo "\$""#), [w("echo"), w("$")]);
        assert_eq!(lexed(r#"echo "\`""#), [w("echo"), w("`")]);
        assert_eq!(lexed(r#"echo "a\nb""#), [w("echo"), w(r"a\nb")]); // any other backslash is itself
        assert_eq!(lexed(r#"echo "\ ""#), [w("echo"), w(r"\ ")]);
    }

    #[test]
    fn an_escaped_dollar_is_never_rescanned_for_expansion() {
        // The bug `DChar`/`assemble_dquoted` exist to avoid: if the escaped `$` were naively
        // concatenated with the raw text after it before scanning, this would wrongly become a
        // reference to `$FOO`.
        assert_eq!(lexed(r#"echo "\$FOO""#), [w("echo"), w("$FOO")]);
        assert_eq!(lexed(r"echo \$FOO"), [w("echo"), w("$FOO")]);
    }

    #[test]
    fn an_unquoted_backslash_escapes_the_next_character() {
        assert_eq!(lexed(r"echo a\ b"), [w("echo"), w("a b")]);
        assert_eq!(lexed(r"echo \|"), [w("echo"), w("|")]);
        assert_eq!(lexed(r"echo \>x"), [w("echo"), w(">x")]);
        assert_eq!(lexed(r"echo \\"), [w("echo"), w(r"\")]);
        assert_eq!(lexed(r"echo \#a"), [w("echo"), w("#a")]);
    }

    #[test]
    fn unterminated_quotes_and_escapes_are_errors() {
        assert_eq!(lex(r#"echo "abc"#), Err(LexError::Unterminated("quote")));
        assert_eq!(lex("echo 'abc"), Err(LexError::Unterminated("quote")));
        assert_eq!(lex(r"echo abc\"), Err(LexError::Unterminated("escape")));
        assert_eq!(lex(r#"echo "abc\"#), Err(LexError::Unterminated("quote")));
    }

    #[test]
    fn a_comment_starts_only_at_the_start_of_a_word() {
        assert_eq!(lexed("# nothing here"), []);
        assert_eq!(lexed("echo a #b c"), [w("echo"), w("a")]);
        assert_eq!(lexed("echo a#b"), [w("echo"), w("a#b")]);
        assert_eq!(lexed("echo #"), [w("echo")]);
        assert_eq!(lexed(r##"echo "#""##), [w("echo"), w("#")]);
        assert_eq!(
            lexed("echo a | # trailing"),
            [w("echo"), w("a"), Token::Pipe]
        );
    }

    #[test]
    fn pipes_and_redirections_end_words() {
        assert_eq!(lexed("a|b"), [w("a"), Token::Pipe, w("b")]);
        assert_eq!(lexed("a>b"), [w("a"), r(None, RedirOp::Out), w("b")]);
        assert_eq!(lexed("a>>b"), [w("a"), r(None, RedirOp::Append), w("b")]);
        assert_eq!(lexed("a<b"), [w("a"), r(None, RedirOp::In), w("b")]);
        assert_eq!(
            lexed("a | b > c < d"),
            [
                w("a"),
                Token::Pipe,
                w("b"),
                r(None, RedirOp::Out),
                w("c"),
                r(None, RedirOp::In),
                w("d")
            ]
        );
        assert_eq!(lexed("a >&2"), [w("a"), r(None, RedirOp::DupOut), w("2")]);
    }

    #[test]
    fn a_descriptor_number_right_before_an_operator_is_taken() {
        assert_eq!(
            lexed("cmd 2>x"),
            [w("cmd"), r(Some(2), RedirOp::Out), w("x")]
        );
        assert_eq!(
            lexed("cmd 1>x"),
            [w("cmd"), r(Some(1), RedirOp::Out), w("x")]
        );
        assert_eq!(
            lexed("cmd 2>>x"),
            [w("cmd"), r(Some(2), RedirOp::Append), w("x")]
        );
        assert_eq!(
            lexed("cmd 2>&1"),
            [w("cmd"), r(Some(2), RedirOp::DupOut), w("1")]
        );
        assert_eq!(
            lexed("cmd 0<x"),
            [w("cmd"), r(Some(0), RedirOp::In), w("x")]
        );
    }

    #[test]
    fn digits_elsewhere_are_ordinary_text() {
        assert_eq!(lexed("a2>x"), [w("a2"), r(None, RedirOp::Out), w("x")]);
        assert_eq!(
            lexed("echo 2 >x"),
            [w("echo"), w("2"), r(None, RedirOp::Out), w("x")]
        );
        assert_eq!(
            lexed(r#"echo "2">x"#),
            [w("echo"), w("2"), r(None, RedirOp::Out), w("x")]
        );
        assert_eq!(
            lexed(r"echo \2>x"),
            [w("echo"), w("2"), r(None, RedirOp::Out), w("x")]
        );
        assert_eq!(
            lexed("cmd 3>x"),
            [w("cmd"), w("3"), r(None, RedirOp::Out), w("x")]
        );
        assert_eq!(
            lexed("cmd 12>x"),
            [w("cmd"), w("12"), r(None, RedirOp::Out), w("x")]
        );
        assert_eq!(
            lexed("cmd 2<x"),
            [w("cmd"), w("2"), r(None, RedirOp::In), w("x")]
        ); // 2 is not an input descriptor
        assert_eq!(
            lexed("cmd 0>x"),
            [w("cmd"), w("0"), r(None, RedirOp::Out), w("x")]
        ); // nor 0 an output one
    }

    #[test]
    fn non_dollar_expansion_characters_are_still_ordinary() {
        assert_eq!(
            lexed("echo `y` * ? ~ {a,b}"),
            [w("echo"), w("`y`"), w("*"), w("?"), w("~"), w("{a,b}")]
        );
    }

    #[test]
    fn unquoted_var_is_recognized() {
        assert_eq!(lexed("echo $x"), [w("echo"), word(vec![var("x", false)])]);
        assert_eq!(
            lexed("echo $x!"),
            [
                w("echo"),
                word(vec![var("x", false), Part::Lit("!".into())])
            ]
        );
        assert_eq!(
            lexed("echo a$xb"),
            [
                w("echo"),
                word(vec![Part::Lit("a".into()), var("xb", false)])
            ]
        );
    }

    #[test]
    fn braced_var_stops_at_the_closing_brace() {
        assert_eq!(
            lexed("echo ${x}y"),
            [
                w("echo"),
                word(vec![var("x", false), Part::Lit("y".into())])
            ]
        );
    }

    #[test]
    fn dollar_question_is_status() {
        assert_eq!(lexed("echo $?"), [w("echo"), word(vec![status(false)])]);
        assert_eq!(lexed("echo ${?}"), [w("echo"), word(vec![status(false)])]);
    }

    #[test]
    fn quoted_var_is_marked_quoted() {
        assert_eq!(
            lexed(r#"echo "$x""#),
            [w("echo"), word(vec![var("x", true)])]
        );
        assert_eq!(
            lexed(r#"echo "$x" $x"#),
            [
                w("echo"),
                word(vec![var("x", true)]),
                word(vec![var("x", false)])
            ]
        );
    }

    #[test]
    fn dollar_not_followed_by_a_name_is_literal() {
        assert_eq!(lexed("echo $"), [w("echo"), w("$")]);
        assert_eq!(lexed("echo $1"), [w("echo"), w("$1")]);
        assert_eq!(lexed("echo $$"), [w("echo"), w("$$")]);
        assert_eq!(lexed(r#"echo "$""#), [w("echo"), w("$")]);
    }

    #[test]
    fn single_quotes_block_expansion_but_double_quotes_do_not() {
        assert_eq!(lexed("echo '$x'"), [w("echo"), w("$x")]);
        assert_eq!(
            lexed(r#"echo "$x""#),
            [w("echo"), word(vec![var("x", true)])]
        );
    }

    #[test]
    fn bad_substitution_is_an_error() {
        assert_eq!(lex("echo ${"), Err(LexError::BadSubstitution));
        assert_eq!(lex("echo ${x"), Err(LexError::BadSubstitution));
        assert_eq!(lex("echo ${1abc}"), Err(LexError::BadSubstitution));
        assert_eq!(lex(r#"echo "${""#), Err(LexError::BadSubstitution));
    }

    #[test]
    fn assignment_shaped_words_are_marked() {
        let Token::Word(word) = &lexed("FOO=bar")[0] else {
            panic!("expected a word")
        };
        assert_eq!(word.assign, Some(3));
        assert_eq!(
            word.assignment(),
            Some((
                "FOO",
                Word {
                    parts: vec![Part::Lit("bar".into())],
                    assign: None
                }
            ))
        );
    }

    #[test]
    fn assignment_value_can_contain_an_expansion() {
        let Token::Word(word) = &lexed("FOO=$BAR")[0] else {
            panic!("expected a word")
        };
        assert_eq!(
            word.assignment(),
            Some((
                "FOO",
                Word {
                    parts: vec![var("BAR", false)],
                    assign: None
                }
            ))
        );
    }

    #[test]
    fn a_plain_word_is_not_marked_as_an_assignment() {
        let Token::Word(word) = &lexed("echo")[0] else {
            panic!("expected a word")
        };
        assert_eq!(word.assign, None);
    }

    #[test]
    fn an_invalid_identifier_before_equals_is_not_an_assignment() {
        let Token::Word(word) = &lexed("1FOO=bar")[0] else {
            panic!("expected a word")
        };
        assert_eq!(word.assign, None);
    }

    #[test]
    fn a_word_starting_with_an_expansion_is_never_an_assignment() {
        // `$FOO=bar` -- the name half must be literal text, not an expansion.
        let Token::Word(word) = &lexed("$FOO=bar")[0] else {
            panic!("expected a word")
        };
        assert_eq!(word.assign, None);
    }

    #[test]
    fn unsupported_syntax_is_refused() {
        assert_eq!(lex("a ;; b"), Err(LexError::Unsupported(";;")));
        assert_eq!(lex("a &"), Err(LexError::Unsupported("&")));
        assert_eq!(lex("a &>f"), Err(LexError::Unsupported("&")));
        assert_eq!(lex("(a)"), Err(LexError::Unsupported("(")));
        assert_eq!(lex("a <<EOF"), Err(LexError::Unsupported("<<")));
        assert_eq!(lex("a <&0"), Err(LexError::Unsupported("<&")));
    }

    #[test]
    fn list_operators_are_tokens() {
        assert_eq!(
            lexed("a; b && c || d"),
            [
                w("a"),
                Token::Semi,
                w("b"),
                Token::AndIf,
                w("c"),
                Token::OrIf,
                w("d")
            ]
        );
        // No blanks needed around them, and `||` is not two pipes.
        assert_eq!(lexed("a&&b"), [w("a"), Token::AndIf, w("b")]);
        assert_eq!(lexed("a||b"), [w("a"), Token::OrIf, w("b")]);
        assert_eq!(lexed("a|b"), [w("a"), Token::Pipe, w("b")]);
    }

    #[test]
    fn a_quoted_list_operator_is_a_word() {
        assert_eq!(lexed(r#"echo ";" "&&""#).len(), 3);
        assert!(matches!(lexed(r#"echo ";""#)[1], Token::Word(_)));
        assert!(matches!(lexed(r"echo \;")[1], Token::Word(_)));
    }

    #[test]
    fn a_bang_is_an_operator_only_as_a_whole_word() {
        assert_eq!(lexed("! a"), [Token::Bang, w("a")]);
        assert_eq!(lexed("a && !b"), [w("a"), Token::AndIf, w("!b")]);
        assert_eq!(lexed("!"), [Token::Bang]);
        assert_eq!(lexed("a!"), [w("a!")]);
        assert_eq!(lexed(r"\! a"), [w("!"), w("a")]);
        assert_eq!(lexed("'!' a"), [w("!"), w("a")]);
    }
}
