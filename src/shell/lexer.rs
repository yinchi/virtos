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
    /// Unquoted literal text: shell metacharacters in it (`*`, `?`, `[`) are pattern characters
    /// for filename expansion.
    Lit(String),
    /// Literal text that was quoted or escaped (`'…'`, `"…"` text, `\x`): never a pattern.
    Quoted(String),
    Var {
        name: String,
        quoted: bool,
    },
    Status {
        quoted: bool,
    },
    /// `$(text)` or a backtick pair: `text` is the inner command's *source* (backticks already
    /// un-escaped), run when the word is expanded. The lexer only needed to find where it ends --
    /// which takes lexing it -- and the syntax layer validates it by parsing, so a bad one is an
    /// error of the whole line before anything runs (`shell::syntax::parse`). `quoted` decides, as
    /// for `Var`, whether the result is split into fields.
    Subst {
        text: String,
        quoted: bool,
    },
    /// An unquoted `~` (or `~/…`) at the start of a word, or of an assignment's value: the home
    /// directory, `$HOME`. The result is never split or globbed.
    Home,
    /// `${NAME-word}` and its relatives: `word` is expanded only when `op` calls for it.
    Param {
        name: String,
        op: ParamOp,
        word: Vec<Part>,
        quoted: bool,
    },
    /// `${#NAME}`: the length of `NAME`'s value, in characters.
    Length {
        name: String,
    },
}

/// The operator of a `${NAME<op>word}` expansion. `colon` is whether it was written with a
/// leading `:`, which makes an *empty* value count as unset too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamOp {
    /// `-`/`:-`: the value, or `word` if there is none.
    Default { colon: bool },
    /// `+`/`:+`: `word` if there is a value, nothing otherwise.
    Alternate { colon: bool },
    /// `=`/`:=`: the value, or `word` -- which is also assigned to `NAME`.
    Assign { colon: bool },
    /// `?`/`:?`: the value, or an error naming `NAME` with `word` as the message.
    Error { colon: bool },
}

impl ParamOp {
    fn text(self) -> &'static str {
        match self {
            ParamOp::Default { colon: false } => "-",
            ParamOp::Default { colon: true } => ":-",
            ParamOp::Alternate { colon: false } => "+",
            ParamOp::Alternate { colon: true } => ":+",
            ParamOp::Assign { colon: false } => "=",
            ParamOp::Assign { colon: true } => ":=",
            ParamOp::Error { colon: false } => "?",
            ParamOp::Error { colon: true } => ":?",
        }
    }
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
                Part::Lit(t) | Part::Quoted(t) => s.push_str(t),
                Part::Var { name, .. } => {
                    s.push('$');
                    s.push_str(name);
                }
                Part::Status { .. } => s.push_str("$?"),
                Part::Subst { text, .. } => {
                    s.push_str("$(");
                    s.push_str(text);
                    s.push(')');
                }
                Part::Home => s.push('~'),
                Part::Param { name, op, word, .. } => {
                    s.push_str("${");
                    s.push_str(name);
                    s.push_str(op.text());
                    let inner = Word {
                        parts: word.clone(),
                        assign: None,
                    };
                    s.push_str(&inner.raw());
                    s.push('}');
                }
                Part::Length { name } => {
                    s.push_str("${#");
                    s.push_str(name);
                    s.push('}');
                }
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

/// A literal run as a part: `Quoted` when it came from inside quotes (never a pattern).
fn lit_part(text: String, quoted: bool) -> Part {
    if quoted {
        Part::Quoted(text)
    } else {
        Part::Lit(text)
    }
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
                    parts.push(lit_part(core::mem::take(&mut lit), quoted));
                }
                parts.push(Part::Status { quoted });
            }
            // Positional parameters: one digit (`$10` is `${1}0`, as in every shell), and the
            // count and the whole list.
            Some(c2 @ ('0'..='9' | '#' | '@' | '*')) => {
                chars.next();
                if !lit.is_empty() {
                    parts.push(lit_part(core::mem::take(&mut lit), quoted));
                }
                parts.push(Part::Var {
                    name: c2.to_string(),
                    quoted,
                });
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
                    parts.push(lit_part(core::mem::take(&mut lit), quoted));
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
                    parts.push(lit_part(core::mem::take(&mut lit), quoted));
                }
                parts.push(Part::Var { name, quoted });
            }
            _ => lit.push('$'), // not followed by anything expansion-eligible -- literal `$`
        }
    }
    if !lit.is_empty() {
        parts.push(lit_part(lit, quoted));
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
        match (merged.last_mut(), &part) {
            (Some(Part::Lit(prev)), Part::Lit(next))
            | (Some(Part::Quoted(prev)), Part::Quoted(next)) => prev.push_str(next),
            _ => merged.push(part),
        }
    }
    merged
}

/// Replaces a word's leading unquoted `~` -- only as the whole word or before a `/` -- with
/// [`Part::Home`], and likewise the `~` at the start of an assignment's value (`X=~/bin`). There is
/// one user here, so `~name` is not a home directory: it stays text.
fn apply_tilde(mut parts: Vec<Part>) -> Vec<Part> {
    let alone = parts.len() == 1;
    let Some(Part::Lit(first)) = parts.first() else {
        return parts;
    };
    // Where a tilde prefix could start in the first literal: its start, or just after `NAME=`.
    let start = match first.find('=') {
        Some(eq) if is_valid_name(&first[..eq]) => eq + 1,
        _ => 0,
    };
    let rest = &first[start..];
    // A bare `~` counts only if nothing is glued after it (`~$X` is ordinary text); `~/…` always.
    let is_tilde = (rest == "~" && alone) || rest.starts_with("~/");
    if !is_tilde {
        return parts;
    }
    let head = first[..start].to_string();
    let tail = rest[1..].to_string();
    let mut out = Vec::with_capacity(parts.len() + 2);
    if !head.is_empty() {
        out.push(Part::Lit(head));
    }
    out.push(Part::Home);
    if !tail.is_empty() {
        out.push(Part::Lit(tail));
    }
    out.extend(parts.drain(1..));
    out
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
    /// A whole command substitution (`$(…)` or backticks) inside the quotes.
    Sub(Part),
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
                parts.push(Part::Quoted(c.to_string()));
            }
            DChar::Sub(part) => {
                if !raw.is_empty() {
                    parts.extend(scan_expansions(&core::mem::take(&mut raw), true)?);
                }
                parts.push(part);
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
            = !"$(" !"${" !"`" !['"' | '\'' | '\\' | '|' | '<' | '>' | '#' | ';' | '&' | '(' | ')'
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
            = "'" s:$(squote_char()*) "'" { vec![Part::Quoted(s.to_string())] }
            / "'" squote_char()* {? Err("unterminated-quote") }
        // No escaping or expansion inside single quotes.

        rule dquote_escape() -> DChar = "\\" c:['"' | '\\' | '$' | '`'] { DChar::Esc(c) }
        rule dquote_sub() -> DChar = p:substitution(true) { DChar::Sub(p) }
        rule dquote_char() -> DChar
            = dquote_escape() / dquote_sub() / (!"\"" !"$((" c:[_] { DChar::Raw(c) })
        // A backslash not immediately before one of the four specials falls through to the
        // catch-all and is kept, as a raw character, by the *next* dquote_char -- pushing a lone
        // '\\' and letting the following character (or end of input) be handled normally on the
        // next iteration, rather than needing a dedicated "backslash but not before a special"
        // rule.
        rule dquoted() -> Vec<Part>
            = "\"" s:dquote_char()* "\"" {? assemble_dquoted(s).map_err(decode_as_token) }
            / "\"" dquote_char()* {? Err("unterminated-quote") }

        rule escape_piece() -> Vec<Part>
            = "\\" c:[_] { vec![Part::Quoted(c.to_string())] }
            / "\\" {? Err("unterminated-escape") }
        // Outside any quote, a backslash escapes exactly the next character, whatever it is --
        // including `$`, which this isolates from `tail`'s own trailing `unquoted_tail_char`
        // text the same way `assemble_dquoted` isolates an escaped `$` inside double quotes:
        // `piece` (below) and the raw text after it are two separate grammar productions, never
        // concatenated into one string before either is scanned for expansions.

        // Command substitution. The inside of `$( … )` is lexed with the ordinary token rules
        // until the first unquoted ")" -- which is how a ")" in inner quotes or a nested `$(…)`
        // is skipped, and why ")" is a token (an error) everywhere else. "#" starts a comment
        // that runs to the end of the line, so a ")" after it does not close anything.
        rule subst_items() = (token() {()} / blank() / comment())*
        rule paren_subst(quoted: bool) -> Part
            = "$(" !"(" s:$(subst_items()) ")" { Part::Subst { text: s.to_string(), quoted } }
            / "$(" !"(" subst_items() ![_] {? Err("unterminated-subst") }
        // `$((` is arithmetic, not a substitution of a subshell: refused by name.
        rule arith() -> Part = "$((" {? Err("op-arith") }
        // Backticks: a backslash makes the next character part of the body; the body is then
        // un-escaped (`\$`, `\``, `\\`, and `\"` inside double quotes) and must lex.
        rule bt_char() = "\\" [_] / !"`" [_]
        rule backtick(quoted: bool) -> Part
            = "`" s:$(bt_char()*) "`" {? backtick_part(s, quoted) }
            / "`" bt_char()* {? Err("unterminated-subst") }
        // `${NAME}` and the forms with an operator. The word after the operator is lexed with
        // the same pieces as anywhere else (quotes, escapes, substitutions, nested `${…}`),
        // up to the first unquoted `}`; plain `$NAME` runs inside it are scanned for expansions.
        rule param_name() -> String
            = s:$(['A'..='Z' | 'a'..='z' | '_'] ['A'..='Z' | 'a'..='z' | '0'..='9' | '_']*
                  / ['0'..='9']+
                  / ['?' | '#' | '@' | '*']) { s.to_string() }
        rule param_op() -> ParamOp
            = ":-" { ParamOp::Default { colon: true } }
            / ":=" { ParamOp::Assign { colon: true } }
            / ":+" { ParamOp::Alternate { colon: true } }
            / ":?" { ParamOp::Error { colon: true } }
            / "-" { ParamOp::Default { colon: false } }
            / "=" { ParamOp::Assign { colon: false } }
            / "+" { ParamOp::Alternate { colon: false } }
            / "?" { ParamOp::Error { colon: false } }
        rule param_raw(quoted: bool) -> Vec<Part>
            = s:$((!"}" !"${" !"$(" !"`" !"'" !"\"" !"\\" [_])+) {?
                scan_expansions(s, quoted).map_err(decode_as_token)
              }
        rule param_item(quoted: bool) -> Vec<Part>
            = dquoted() / squoted() / escape_piece()
            / p:substitution(quoted) { vec![p] }
            / param_raw(quoted)
        rule param_word(quoted: bool) -> Vec<Part>
            = items:param_item(quoted)* { merge_lits(items.into_iter().flatten().collect()) }
        rule param_exp(quoted: bool) -> Part
            = "${" "#" n:param_name() "}" { Part::Length { name: n } }
            / "${" n:param_name() op:param_op() w:param_word(quoted) "}" {
                Part::Param { name: n, op, word: w, quoted }
              }
            / "${" n:param_name() "}" {
                if n == "?" { Part::Status { quoted } } else { Part::Var { name: n, quoted } }
              }
            // Whatever else follows `${`: consume it up to the `}` (or the end) before refusing,
            // so this failure is the furthest one and its sentinel is what gets reported.
            / "${" (!"}" [_])* "}"? {? Err("bad-substitution") }

        rule substitution(quoted: bool) -> Part
            = arith() / paren_subst(quoted) / backtick(quoted) / param_exp(quoted)

        rule piece() -> Vec<Part>
            = dquoted() / squoted() / escape_piece() / p:substitution(false) { vec![p] }
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
                let parts = apply_tilde(merge_lits(parts));
                let assign = detect_assign(&parts);
                Word { parts, assign }
              }
            / ts:tail()+ {
                let mut parts = Vec::new();
                for t in ts { parts.extend(t); }
                let parts = apply_tilde(merge_lits(parts));
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

/// The text of a backtick body, un-escaped as bash does: `\$`, `` \` `` and `\\` lose their
/// backslash, and so does `\"` inside double quotes; every other backslash is kept.
fn unescape_backticks(body: &str, quoted: bool) -> String {
    let mut out = String::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek().copied() {
                Some(n @ ('$' | '`' | '\\')) => {
                    out.push(n);
                    chars.next();
                    continue;
                }
                Some('"') if quoted => {
                    out.push('"');
                    chars.next();
                    continue;
                }
                _ => {}
            }
        }
        out.push(c);
    }
    out
}

/// What the grammar's own sentinel for `error` is, so a failure found while lexing a backtick
/// body surfaces as that same error of the whole line.
fn error_sentinel(error: LexError) -> &'static str {
    match error {
        LexError::Unterminated("quote") => "unterminated-quote",
        LexError::Unterminated("escape") => "unterminated-escape",
        LexError::Unterminated(_) => "unterminated-subst",
        LexError::BadSubstitution => "bad-substitution",
        LexError::Unsupported("<<") => "op-heredoc",
        LexError::Unsupported("<&") => "op-dupin",
        LexError::Unsupported(";;") => "op-dsemi",
        LexError::Unsupported("(") => "op-lparen",
        LexError::Unsupported(")") => "op-rparen",
        LexError::Unsupported("$((") => "op-arith",
        LexError::Unsupported(_) => "op-amp",
    }
}

/// A backtick pair's body as a [`Part::Subst`], having checked it lexes.
fn backtick_part(body: &str, quoted: bool) -> Result<Part, &'static str> {
    let text = unescape_backticks(body, quoted);
    lex(&text).map_err(error_sentinel)?;
    Ok(Part::Subst { text, quoted })
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
    // Several sentinels can be expected at the furthest position (a failed alternative, then the
    // fallback that also fails); this order says which the user is told about -- a refused
    // operator before the "unterminated" the failure also looks like.
    let expected: alloc::vec::Vec<&str> = e.expected.tokens().collect();
    for (name, error) in [
        ("op-arith", LexError::Unsupported("$((")),
        ("op-heredoc", LexError::Unsupported("<<")),
        ("op-dupin", LexError::Unsupported("<&")),
        ("op-dsemi", LexError::Unsupported(";;")),
        ("op-amp", LexError::Unsupported("&")),
        ("op-lparen", LexError::Unsupported("(")),
        ("op-rparen", LexError::Unsupported(")")),
        ("bad-substitution", LexError::BadSubstitution),
        (
            "unterminated-subst",
            LexError::Unterminated("command substitution"),
        ),
        ("unterminated-quote", LexError::Unterminated("quote")),
        ("unterminated-escape", LexError::Unterminated("escape")),
    ] {
        if expected.contains(&name) {
            return error;
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
    /// `lex`, with quoted text folded into the literal text around it: most tests here are
    /// about *where words end* and what expansions they hold, not about whether a piece was
    /// quoted (the `quoted_text_is_kept_apart` tests below check that directly).
    fn lexed(line: &str) -> Vec<Token> {
        lex(line).unwrap().into_iter().map(flatten).collect()
    }

    fn flatten(token: Token) -> Token {
        let Token::Word(word) = token else {
            return token;
        };
        let parts = word
            .parts
            .into_iter()
            .map(|p| match p {
                Part::Quoted(t) => Part::Lit(t),
                other => other,
            })
            .collect();
        let parts = merge_lits(parts);
        let assign = word.assign;
        Token::Word(Word { parts, assign })
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
        // (The backtick and the tilde used to be on this list; each is an expansion now.)
        assert_eq!(
            lexed("echo * ? {a,b}"),
            [w("echo"), w("*"), w("?"), w("{a,b}")]
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

    fn subst(text: &str, quoted: bool) -> Part {
        Part::Subst {
            text: text.into(),
            quoted,
        }
    }

    #[test]
    fn a_command_substitution_is_one_part() {
        assert_eq!(lexed("$(echo hi)"), [word(vec![subst("echo hi", false)])]);
        assert_eq!(
            lexed("a$(b)c"),
            [word(vec![
                Part::Lit("a".into()),
                subst("b", false),
                Part::Lit("c".into())
            ])]
        );
        assert_eq!(
            lexed("\"x$(b)y\""),
            [word(vec![
                Part::Lit("x".into()),
                subst("b", true),
                Part::Lit("y".into())
            ])]
        );
    }

    #[test]
    fn the_inside_is_its_own_quoting_context() {
        // The inner quotes do not end the outer ones, and a ")" inside them is not the end.
        assert_eq!(
            lexed(r#""$(echo "a b")""#),
            [word(vec![subst(r#"echo "a b""#, true)])]
        );
        assert_eq!(
            lexed("$(echo ')' \\) z)"),
            [word(vec![subst("echo ')' \\) z", false)])]
        );
        assert_eq!(
            lexed("$(echo $(echo hi))"),
            [word(vec![subst("echo $(echo hi)", false)])]
        );
        // Operators that end a word elsewhere are ordinary inside.
        assert_eq!(
            lexed("$(a | b ; c && d > e)"),
            [word(vec![subst("a | b ; c && d > e", false)])]
        );
    }

    #[test]
    fn a_comment_inside_swallows_the_closing_paren() {
        assert_eq!(
            lex("$(echo a # c)"),
            Err(LexError::Unterminated("command substitution"))
        );
    }

    #[test]
    fn unterminated_and_arithmetic_substitutions_are_refused() {
        assert_eq!(
            lex("echo $(a"),
            Err(LexError::Unterminated("command substitution"))
        );
        assert_eq!(
            lex("echo `a"),
            Err(LexError::Unterminated("command substitution"))
        );
        assert_eq!(lex("echo $((1+2))"), Err(LexError::Unsupported("$((")));
        assert_eq!(lex("echo \"$((1))\""), Err(LexError::Unsupported("$((")));
        // A ")" outside any substitution is still refused.
        assert_eq!(lex("echo a)"), Err(LexError::Unsupported(")")));
    }

    #[test]
    fn a_lex_error_inside_a_substitution_is_the_lines() {
        assert_eq!(lex("echo $(echo 'x)"), Err(LexError::Unterminated("quote")));
        assert_eq!(lex("echo $(a & b)"), Err(LexError::Unsupported("&")));
        assert_eq!(lex("echo $(echo ${)"), Err(LexError::BadSubstitution));
    }

    #[test]
    fn quoting_and_escaping_keep_a_dollar_paren_literal() {
        assert_eq!(lexed("'$(a)'"), [w("$(a)")]);
        // An escaped `$` leaves a bare "(" behind, which is refused as it is anywhere.
        assert_eq!(lex(r"\$(a)"), Err(LexError::Unsupported("(")));
        assert_eq!(lexed(r#""\$(a)""#), [w("$(a)")]);
    }

    #[test]
    fn backticks_are_a_substitution_with_their_own_escaping() {
        assert_eq!(lexed("`echo hi`"), [word(vec![subst("echo hi", false)])]);
        assert_eq!(
            lexed(r#""`echo hi`""#),
            [word(vec![subst("echo hi", true)])]
        );
        // `\`` inside is a nested pair, and `\$` and `\\` lose their backslash.
        assert_eq!(
            lexed(r"`echo \`date\``"),
            [word(vec![subst("echo `date`", false)])]
        );
        assert_eq!(
            lexed(r"`echo \$HOME \\ \x`"),
            [word(vec![subst(r"echo $HOME \ \x", false)])]
        );
        // `\"` loses its backslash only inside double quotes.
        assert_eq!(
            lexed(r#""`echo \"a\"`""#),
            [word(vec![subst(r#"echo "a""#, true)])]
        );
    }

    // ---- quoted text is kept apart from unquoted (it is never a filename pattern) ----

    fn raw_word(line: &str) -> Vec<Part> {
        let Token::Word(word) = lex(line).unwrap().remove(0) else {
            panic!("expected a word")
        };
        word.parts
    }

    #[test]
    fn quoted_text_is_kept_apart_quoted_text_tests() {
        assert_eq!(raw_word("*.c"), [Part::Lit("*.c".into())]);
        assert_eq!(raw_word("'*.c'"), [Part::Quoted("*.c".into())]);
        assert_eq!(raw_word("\"*.c\""), [Part::Quoted("*.c".into())]);
        assert_eq!(
            raw_word("\\*.c"),
            [Part::Quoted("*".into()), Part::Lit(".c".into())]
        );
        // A word is a mix, in order; adjacent pieces of the same kind merge.
        assert_eq!(
            raw_word("a'b'\"c\"*"),
            [
                Part::Lit("a".into()),
                Part::Quoted("bc".into()),
                Part::Lit("*".into())
            ]
        );
    }

    #[test]
    fn quoted_text_inside_double_quotes_around_an_expansion() {
        assert_eq!(
            raw_word("\"a$X*\""),
            [
                Part::Quoted("a".into()),
                Part::Var {
                    name: "X".into(),
                    quoted: true
                },
                Part::Quoted("*".into()),
            ]
        );
    }

    #[test]
    fn a_quoted_name_is_not_an_assignment() {
        let Token::Word(word) = lex("'X'=1").unwrap().remove(0) else {
            panic!("expected a word")
        };
        assert_eq!(word.assign, None);
        let Token::Word(word) = lex("X='a b'").unwrap().remove(0) else {
            panic!("expected a word")
        };
        assert_eq!(word.assign, Some(1));
        assert_eq!(
            word.assignment().unwrap().1.parts,
            [Part::Quoted("a b".into())]
        );
    }

    // ---- tilde, `${…}` forms, positional parameters ----

    #[test]
    fn a_leading_tilde_is_the_home_directory() {
        assert_eq!(raw_word("~"), [Part::Home]);
        assert_eq!(raw_word("~/bin"), [Part::Home, Part::Lit("/bin".into())]);
        // Only at the start of a word, and only as the whole prefix.
        assert_eq!(raw_word("a~"), [Part::Lit("a~".into())]);
        assert_eq!(raw_word("~x"), [Part::Lit("~x".into())]);
        assert_eq!(raw_word("~root/x"), [Part::Lit("~root/x".into())]);
        assert_eq!(raw_word("'~'"), [Part::Quoted("~".into())]);
        assert_eq!(raw_word("\\~"), [Part::Quoted("~".into())]);
        assert_eq!(
            raw_word("~$X"),
            [
                Part::Lit("~".into()),
                Part::Var {
                    name: "X".into(),
                    quoted: false
                }
            ]
        );
    }

    #[test]
    fn a_tilde_at_the_start_of_an_assignments_value_is_home_too() {
        let Token::Word(word) = lex("PATH=~/bin").unwrap().remove(0) else {
            panic!("expected a word")
        };
        assert_eq!(word.assign, Some(4));
        assert_eq!(
            word.assignment().unwrap().1.parts,
            [Part::Home, Part::Lit("/bin".into())]
        );
        // Not after a colon, as bash's would: one user, one home, no `PATH`-style lists here.
        assert_eq!(raw_word("A=b:~"), [Part::Lit("A=b:~".into())]);
    }

    #[test]
    fn positional_parameters_and_their_specials() {
        for (text, name) in [
            ("$1", "1"),
            ("$9", "9"),
            ("$0", "0"),
            ("$#", "#"),
            ("$@", "@"),
            ("$*", "*"),
        ] {
            assert_eq!(
                raw_word(text),
                [Part::Var {
                    name: name.into(),
                    quoted: false
                }],
                "{text}"
            );
        }
        // One digit: `$10` is `${1}` then `0`; the braces take a longer number.
        assert_eq!(
            raw_word("$10"),
            [
                Part::Var {
                    name: "1".into(),
                    quoted: false
                },
                Part::Lit("0".into())
            ]
        );
        assert_eq!(
            raw_word("${10}"),
            [Part::Var {
                name: "10".into(),
                quoted: false
            }]
        );
        assert_eq!(
            raw_word("\"$@\""),
            [Part::Var {
                name: "@".into(),
                quoted: true
            }]
        );
        // `$$` and a lone `$` are still just text.
        assert_eq!(raw_word("$$"), [Part::Lit("$$".into())]);
    }

    #[test]
    fn a_parameter_expansion_with_an_operator() {
        use ParamOp::*;
        for (text, op) in [
            ("${X:-w}", Default { colon: true }),
            ("${X-w}", Default { colon: false }),
            ("${X:=w}", Assign { colon: true }),
            ("${X=w}", Assign { colon: false }),
            ("${X:+w}", Alternate { colon: true }),
            ("${X+w}", Alternate { colon: false }),
            ("${X:?w}", Error { colon: true }),
            ("${X?w}", Error { colon: false }),
        ] {
            assert_eq!(
                raw_word(text),
                [Part::Param {
                    name: "X".into(),
                    op,
                    word: vec![Part::Lit("w".into())],
                    quoted: false
                }],
                "{text}"
            );
        }
    }

    #[test]
    fn the_word_of_an_expansion_may_hold_blanks_quotes_and_more_expansions() {
        assert_eq!(
            raw_word("${X:-a b}"),
            [Part::Param {
                name: "X".into(),
                op: ParamOp::Default { colon: true },
                word: vec![Part::Lit("a b".into())],
                quoted: false
            }]
        );
        assert_eq!(
            raw_word("\"${X:-'q' $Y}\""),
            [Part::Param {
                name: "X".into(),
                op: ParamOp::Default { colon: true },
                word: vec![
                    Part::Quoted("q ".into()),
                    Part::Var {
                        name: "Y".into(),
                        quoted: true
                    }
                ],
                quoted: true
            }]
        );
        // Nested, and a `}` inside inner quotes does not close it.
        assert_eq!(
            raw_word("${A:-${B:-c}}"),
            [Part::Param {
                name: "A".into(),
                op: ParamOp::Default { colon: true },
                word: vec![Part::Param {
                    name: "B".into(),
                    op: ParamOp::Default { colon: true },
                    word: vec![Part::Lit("c".into())],
                    quoted: false
                }],
                quoted: false
            }]
        );
        assert_eq!(
            raw_word("${A:-'}'}"),
            [Part::Param {
                name: "A".into(),
                op: ParamOp::Default { colon: true },
                word: vec![Part::Quoted("}".into())],
                quoted: false
            }]
        );
    }

    #[test]
    fn an_empty_word_and_a_length() {
        assert_eq!(
            raw_word("${X:-}"),
            [Part::Param {
                name: "X".into(),
                op: ParamOp::Default { colon: true },
                word: vec![],
                quoted: false
            }]
        );
        assert_eq!(raw_word("${#X}"), [Part::Length { name: "X".into() }]);
        // `${#}` is `$#`, the count.
        assert_eq!(
            raw_word("${#}"),
            [Part::Var {
                name: "#".into(),
                quoted: false
            }]
        );
    }

    #[test]
    fn a_malformed_braced_expansion_is_a_bad_substitution() {
        for line in [
            "echo ${",
            "echo ${}",
            "echo ${a b}",
            "echo ${1x}",
            "echo ${X:-a",
            "echo ${-}",
        ] {
            assert_eq!(lex(line), Err(LexError::BadSubstitution), "{line:?}");
        }
    }
}
