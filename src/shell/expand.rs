//! Turns lexed `Word`s into real argument strings, against live shell state -- real POSIX
//! field-splitting: an unquoted expansion's value splits on runs of blanks, and an all-blank/empty
//! result vanishes entirely (no field at all, not an empty one); a quoted expansion always stays
//! glued into exactly one field, even if its value is empty. Literal text never splits here --
//! that already happened once, at lex time (`lexer`'s own doc comment) -- only an expansion's
//! *resolved value*, unknown until now, can still need it.
//!
//! What a word can hold, and what each becomes: literal text; `$NAME`/`${NAME}`/`$?` and the
//! positional parameters (`$1`, `$#`, `$@`, `$*`); `${NAME:-w}` and its relatives and `${#NAME}`;
//! a command substitution; and a leading `~`. A command substitution is run by the environment
//! (`Env::subst`: it takes the inner command's source and returns what it wrote to stdout, or a
//! message if it could not run) and its result then has *all* trailing newlines removed, as the
//! shell's does, before being spliced in like any other value.
//!
//! Each resulting field also carries a glob *pattern* -- its text with every quoted character
//! escaped (`textlib::glob::escape`) -- so filename expansion can tell what the user quoted from
//! what they did not: unquoted literal text and the unquoted result of an expansion are pattern
//! text (`ls $PATTERN` globs; `ls "$PATTERN"` does not), quoted text never is. `expand_fields`
//! returns them; `expand_words` is the same without the patterns, for callers that never glob.
//!
//! Pure logic: everything that touches shell state -- variables, `$?`, the positional
//! parameters, running a substitution, assigning for `${NAME:=w}` -- is behind the [`Env`] trait,
//! implemented by the real call site (`shell::mod`, bare-metal only) rather than reached into
//! directly, so this is host-tested the same way `fs::sector`/`exec::argplan` already are.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use super::lexer::{ParamOp, Part, Word};
use textlib::glob;

/// The shell state an expansion reads and (for `${NAME:=w}`) writes.
pub trait Env {
    /// The value of the variable `name`, or of a positional parameter (`"0"`..`"9"`, `"10"`...) or
    /// the count (`"#"`); `None` if it is unset.
    fn var(&self, name: &str) -> Option<String>;
    /// `$?`.
    fn status(&self) -> i32;
    /// The positional parameters `$1`.., what `$@` and `$*` stand for.
    fn args(&self) -> Vec<String>;
    /// Runs the command substitution whose source is `text`; what it wrote to stdout, or the
    /// message to report if it could not run.
    fn subst(&mut self, text: &str) -> Result<String, String>;
    /// Assigns `value` to the variable `name` (`${NAME:=w}`).
    fn set(&mut self, name: &str, value: &str);
}

/// One expanded word: its text, and the glob pattern the same word is. They are the same string
/// unless something in it was quoted, in which case the pattern has a backslash before each
/// quoted `\`, `*`, `?` and `[`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Field {
    pub text: String,
    pub pattern: String,
}

impl Field {
    /// Whether the pattern has an unquoted wildcard, i.e. whether filename expansion applies.
    pub fn is_pattern(&self) -> bool {
        glob::is_magic(&self.pattern)
    }

    fn push_unquoted(&mut self, text: &str) {
        self.text.push_str(text);
        self.pattern.push_str(text);
    }

    fn push_quoted(&mut self, text: &str) {
        self.text.push_str(text);
        self.pattern.push_str(&glob::escape(text));
    }
}

/// Expands every word in `words` into fields, in order -- a word that vanishes entirely (an
/// unquoted expansion whose value is empty/all-blank, and nothing else in that word) contributes
/// no field at all, so the output can have fewer entries than `words` itself; `"$@"` can
/// contribute several.
///
/// `Err` is the message from a substitution that could not run (or a `${NAME:?w}` that fired);
/// nothing after it is expanded.
pub fn expand_fields(words: &[Word], env: &mut impl Env) -> Result<Vec<Field>, String> {
    let mut out = Vec::new();
    for word in words {
        out.extend(expand_word(word, env)?);
    }
    Ok(out)
}

/// `expand_fields`, keeping only each field's text.
#[cfg(test)]
pub fn expand_words(words: &[Word], env: &mut impl Env) -> Result<Vec<String>, String> {
    Ok(expand_fields(words, env)?
        .into_iter()
        .map(|f| f.text)
        .collect())
}

/// A substitution's value as spliced in: its output with every trailing newline removed.
fn subst_value(text: &str, env: &mut impl Env) -> Result<String, String> {
    let mut value = env.subst(text)?;
    while value.ends_with('\n') {
        value.pop();
    }
    Ok(value)
}

/// The text of `parts` as one string, with no splitting at all: what an assignment's value is,
/// and what the word of a `${NAME:-word}` is until the whole expansion is spliced in. `$@` and
/// `$*` join with a blank.
fn parts_to_string(parts: &[Part], env: &mut impl Env) -> Result<String, String> {
    let mut s = String::new();
    for part in parts {
        match part {
            Part::Lit(text) | Part::Quoted(text) => s.push_str(text),
            Part::Var { name, .. } => s.push_str(&variable(name, env).join(" ")),
            Part::Status { .. } => s.push_str(&env.status().to_string()),
            Part::Subst { text, .. } => s.push_str(&subst_value(text, env)?),
            Part::Home => s.push_str(&home(env)),
            Part::Param { name, op, word, .. } => s.push_str(&param_value(name, *op, word, env)?),
            Part::Length { name } => s.push_str(&length(name, env)),
        }
    }
    Ok(s)
}

/// Expands `word` (an assignment's *value* half, from `Word::assignment`) into one string, with
/// no field-splitting at all, quoted or not -- matches the lab's own `expand_assignment`: there's
/// nowhere for a split piece to go (an assignment is one value, not an argv position), so
/// `A=$MULTI` assigns the whole value as one string even though the same `$MULTI` unquoted in an
/// ordinary argument position would split into several.
pub fn expand_assignment_value(word: &Word, env: &mut impl Env) -> Result<String, String> {
    parts_to_string(&word.parts, env)
}

/// Expands `word` as a redirection's target: variables and substitutions expanded, **no**
/// splitting beyond what quoting already decided -- it must come out as exactly one field, else
/// `ambiguous redirect` (an unquoted expansion that is empty, or that holds blanks).
pub fn expand_target(word: &Word, env: &mut impl Env) -> Result<String, String> {
    let mut fields = expand_word(word, env)?;
    if fields.len() == 1 {
        Ok(fields.remove(0).text)
    } else {
        Err(alloc::format!("{}: ambiguous redirect", word.raw()))
    }
}

/// The values a variable name stands for: normally one (or none, when unset); `@` and `*` are the
/// positional parameters, one value each.
fn variable(name: &str, env: &impl Env) -> Vec<String> {
    match name {
        "@" | "*" => env.args(),
        _ => env.var(name).into_iter().collect(),
    }
}

/// `$HOME`, or a literal `~` when it is unset or empty -- the home directory a leading tilde
/// stands for.
fn home(env: &impl Env) -> String {
    env.var("HOME")
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "~".to_string())
}

/// `${#NAME}`: the number of characters in the value (the count of positional parameters for `@`
/// and `*`), `0` when unset.
fn length(name: &str, env: &impl Env) -> String {
    match name {
        "@" | "*" => env.args().len().to_string(),
        _ => env.var(name).map_or(0, |v| v.chars().count()).to_string(),
    }
}

/// What a `${NAME op word}` comes to before `word` is looked at.
enum Choice {
    /// The variable's own value.
    Value(String),
    /// The operator wants `word` (expanded in place for `-` and `+`, assigned for `=`, reported
    /// for `?`).
    Word,
    /// Nothing at all (`${U:+x}` with `U` unset).
    Nothing,
}

fn param_choice(name: &str, op: ParamOp, env: &impl Env) -> Choice {
    let value = variable(name, env).join(" ");
    let set = match name {
        "@" | "*" => !env.args().is_empty(),
        _ => env.var(name).is_some(),
    };
    let colon = match op {
        ParamOp::Default { colon }
        | ParamOp::Alternate { colon }
        | ParamOp::Assign { colon }
        | ParamOp::Error { colon } => colon,
    };
    // "Null" is unset, or -- with the colon -- empty as well.
    let null = !set || (colon && value.is_empty());
    match op {
        ParamOp::Alternate { .. } if null => Choice::Nothing,
        ParamOp::Alternate { .. } => Choice::Word,
        _ if null => Choice::Word,
        _ => Choice::Value(value),
    }
}

/// The message of a `${NAME:?word}` that fired.
fn error_message(name: &str, word: &[Part], env: &mut impl Env) -> Result<String, String> {
    let message = parts_to_string(word, env)?;
    Ok(if message.is_empty() {
        alloc::format!("{name}: parameter null or not set")
    } else {
        alloc::format!("{name}: {message}")
    })
}

/// The value of `${NAME op word}` as a plain string (for an assignment value, or inside another
/// expansion's word), with `word` expanded only when the operator calls for it.
fn param_value(
    name: &str,
    op: ParamOp,
    word: &[Part],
    env: &mut impl Env,
) -> Result<String, String> {
    match param_choice(name, op, env) {
        Choice::Value(value) => Ok(value),
        Choice::Nothing => Ok(String::new()),
        Choice::Word => match op {
            ParamOp::Error { .. } => Err(error_message(name, word, env)?),
            ParamOp::Assign { .. } => {
                let assigned = parts_to_string(word, env)?;
                env.set(name, &assigned);
                Ok(assigned)
            }
            _ => parts_to_string(word, env),
        },
    }
}

/// Expands one word into zero or more fields. `parts` empty (only ever an explicitly empty quoted
/// word, `""`/`''` -- `lexer::Word`'s own doc comment) is always exactly one empty field, never
/// zero: that distinction -- "a word that happens to be empty" vs. "nothing here at all" -- is
/// exactly what quoting decides, and an empty `Word` only ever arises from the former.
fn expand_word(word: &Word, env: &mut impl Env) -> Result<Vec<Field>, String> {
    if word.parts.is_empty() {
        return Ok(vec![Field::default()]);
    }
    let mut building = Building::default();
    for part in &word.parts {
        push_part(part, env, &mut building)?;
    }
    if building.have_current {
        building.fields.push(building.current);
    }
    Ok(building.fields)
}

/// A word being expanded: the fields finished so far, the one in progress, and whether that one
/// exists yet (an unquoted empty expansion at the start of a word has not started one).
#[derive(Default)]
struct Building {
    fields: Vec<Field>,
    current: Field,
    have_current: bool,
}

/// Adds the expansion of one part of a word to `b`. A `${NAME-word}` that selects `word` expands
/// its parts in place, so quoting inside the word (`${U:-'a  b'}`) is kept as it is written.
fn push_part(part: &Part, env: &mut impl Env, b: &mut Building) -> Result<(), String> {
    match part {
        // Literal text never holds a blank at the top level of a word (the lexer ended the word
        // there) -- but the word of an unquoted `${NAME-word}` is lexed as one piece, blanks
        // and all, and is split when it is used, like the value of any unquoted expansion.
        Part::Lit(text) if text.contains(char::is_whitespace) => glue(b, text, false),
        Part::Lit(text) => {
            b.current.push_unquoted(text);
            b.have_current = true;
        }
        Part::Quoted(text) => {
            b.current.push_quoted(text);
            b.have_current = true;
        }
        // A tilde's result is a name, not a pattern, and is not split.
        Part::Home => {
            b.current.push_quoted(&home(env));
            b.have_current = true;
        }
        Part::Var { name, quoted } if name == "@" || name == "*" => {
            let args = env.args();
            splice_args(b, &args, *quoted, name == "@");
        }
        Part::Var { name, quoted } => {
            let value = env.var(name).unwrap_or_default();
            glue(b, &value, *quoted);
        }
        Part::Status { quoted } => {
            let value = env.status().to_string();
            glue(b, &value, *quoted);
        }
        Part::Subst { text, quoted } => {
            let value = subst_value(text, env)?;
            glue(b, &value, *quoted);
        }
        Part::Param {
            name,
            op,
            word,
            quoted,
        } => match param_choice(name, *op, env) {
            Choice::Value(value) => glue(b, &value, *quoted),
            Choice::Nothing => {}
            Choice::Word => match op {
                ParamOp::Error { .. } => return Err(error_message(name, word, env)?),
                ParamOp::Assign { .. } => {
                    let assigned = parts_to_string(word, env)?;
                    env.set(name, &assigned);
                    glue(b, &assigned, *quoted);
                }
                _ => {
                    for inner in word {
                        push_part(inner, env, b)?;
                    }
                }
            },
        },
        Part::Length { name } => {
            let value = length(name, env);
            glue(b, &value, false);
        }
    }
    Ok(())
}

/// Splices the positional parameters in: `"$@"` as one field per parameter (none at all when
/// there are none, even if quoted -- the one quoted expansion that can vanish), `"$*"` as one
/// field joined with blanks, and either unquoted as the parameters joined and then split on
/// blanks like any unquoted value.
fn splice_args(b: &mut Building, args: &[String], quoted: bool, separate: bool) {
    if quoted && separate {
        let Some((first, rest)) = args.split_first() else {
            return;
        };
        b.current.push_quoted(first);
        b.have_current = true;
        for arg in rest {
            b.fields.push(core::mem::take(&mut b.current));
            b.current.push_quoted(arg);
        }
    } else {
        glue(b, &args.join(" "), quoted);
    }
}

/// Glues one expansion's `value` onto the field being built, splitting it first if `quoted` is
/// `false`. A quoted expansion -- even an empty one -- always starts or extends a real field
/// (`have_current = true` unconditionally): that's the one case this function exists to get
/// right, since an *unquoted* empty value does the opposite (contributes nothing at all if
/// nothing else in the word has started a field yet). A quoted value is escaped in the pattern;
/// an unquoted one is pattern text, wildcards and all.
fn glue(b: &mut Building, value: &str, quoted: bool) {
    if quoted {
        b.current.push_quoted(value);
        b.have_current = true;
        return;
    }
    let mut pieces = value.split_whitespace();
    let Some(first) = pieces.next() else {
        return; // empty/all-blank unquoted value -- vanishes, touching nothing
    };
    b.current.push_unquoted(first);
    b.have_current = true;
    for piece in pieces {
        b.fields.push(core::mem::take(&mut b.current));
        b.current.push_unquoted(piece);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::lexer::Word;

    fn lit(text: &str) -> Part {
        Part::Lit(text.into())
    }
    fn quoted_lit(text: &str) -> Part {
        Part::Quoted(text.into())
    }
    fn var_part(name: &str, quoted: bool) -> Part {
        Part::Var {
            name: name.into(),
            quoted,
        }
    }
    fn status_part(quoted: bool) -> Part {
        Part::Status { quoted }
    }
    fn subst_part(text: &str, quoted: bool) -> Part {
        Part::Subst {
            text: text.into(),
            quoted,
        }
    }
    fn param(name: &str, op: ParamOp, word: Vec<Part>, quoted: bool) -> Part {
        Part::Param {
            name: name.into(),
            op,
            word,
            quoted,
        }
    }
    fn word(parts: Vec<Part>) -> Word {
        Word {
            parts,
            assign: None,
        }
    }

    /// A stand-in shell: a few variables, `$?`, the positional parameters, and a runner for
    /// substitutions that knows a handful of commands.
    struct TestEnv {
        vars: Vec<(String, String)>,
        status: i32,
        args: Vec<String>,
        ran: Vec<String>,
    }

    impl TestEnv {
        fn new() -> Self {
            let v = |n: &str, val: &str| (n.to_string(), val.to_string());
            Self {
                vars: vec![
                    v("FOO", "bar"),
                    v("MULTI", "a b c"),
                    v("EMPTY", ""),
                    v("HOME", "/root"),
                ],
                status: 0,
                args: Vec::new(),
                ran: Vec::new(),
            }
        }
        fn with_args(mut self, args: &[&str]) -> Self {
            self.args = args.iter().map(|a| a.to_string()).collect();
            self
        }
        fn with_status(mut self, status: i32) -> Self {
            self.status = status;
            self
        }
    }

    impl Env for TestEnv {
        fn var(&self, name: &str) -> Option<String> {
            if name == "#" {
                return Some(self.args.len().to_string());
            }
            if let Ok(n) = name.parse::<usize>() {
                return match n {
                    0 => Some("script".into()),
                    n => self.args.get(n - 1).cloned(),
                };
            }
            self.vars
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        }
        fn status(&self) -> i32 {
            self.status
        }
        fn args(&self) -> Vec<String> {
            self.args.clone()
        }
        fn subst(&mut self, text: &str) -> Result<String, String> {
            self.ran.push(text.into());
            match text {
                "echo hi" => Ok("hi\n".into()),
                "words" => Ok("a b\nc\n\n\n".into()),
                "empty" => Ok(String::new()),
                "inner-newlines" => Ok("x\n\ny\n".into()),
                "blank" => Ok("  \n".into()),
                "star" => Ok("*.txt\n".into()),
                _ => Err("substitution failed".into()),
            }
        }
        fn set(&mut self, name: &str, value: &str) {
            self.vars.retain(|(k, _)| k != name);
            self.vars.push((name.into(), value.into()));
        }
    }

    fn texts(w: Word, env: &mut TestEnv) -> Vec<String> {
        expand_word(&w, env)
            .unwrap()
            .into_iter()
            .map(|f| f.text)
            .collect()
    }

    fn expand_one(w: Word, status: i32) -> Vec<String> {
        texts(w, &mut TestEnv::new().with_status(status))
    }

    fn patterns(w: Word) -> Vec<(String, String)> {
        expand_word(&w, &mut TestEnv::new())
            .unwrap()
            .into_iter()
            .map(|f| (f.text, f.pattern))
            .collect()
    }

    fn none() -> Vec<String> {
        Vec::new()
    }

    // ---- variables and splitting ----

    #[test]
    fn a_word_with_no_expansion_is_one_field() {
        assert_eq!(expand_one(word(vec![lit("hello")]), 0), ["hello"]);
    }

    #[test]
    fn an_empty_quoted_word_is_one_empty_field() {
        assert_eq!(expand_one(word(vec![]), 0), [""]);
    }

    #[test]
    fn unquoted_unset_or_empty_variable_vanishes_entirely() {
        assert_eq!(
            expand_one(word(vec![var_part("MISSING", false)]), 0),
            none()
        );
        assert_eq!(expand_one(word(vec![var_part("EMPTY", false)]), 0), none());
    }

    #[test]
    fn quoted_unset_or_empty_variable_is_one_empty_field() {
        assert_eq!(expand_one(word(vec![var_part("MISSING", true)]), 0), [""]);
        assert_eq!(expand_one(word(vec![var_part("EMPTY", true)]), 0), [""]);
    }

    #[test]
    fn a_set_variable_expands_to_its_value() {
        assert_eq!(expand_one(word(vec![var_part("FOO", false)]), 0), ["bar"]);
        assert_eq!(expand_one(word(vec![var_part("FOO", true)]), 0), ["bar"]);
    }

    #[test]
    fn unquoted_multi_word_value_splits_and_quoted_stays_one_field() {
        assert_eq!(
            expand_one(word(vec![var_part("MULTI", false)]), 0),
            ["a", "b", "c"]
        );
        assert_eq!(
            expand_one(word(vec![var_part("MULTI", true)]), 0),
            ["a b c"]
        );
    }

    #[test]
    fn literal_text_glues_to_the_first_and_last_split_piece() {
        assert_eq!(
            expand_one(word(vec![lit("<"), var_part("MULTI", false), lit(">")]), 0),
            ["<a", "b", "c>"]
        );
    }

    #[test]
    fn surrounding_literal_text_survives_an_unquoted_empty_expansion() {
        assert_eq!(
            expand_one(word(vec![lit("a"), var_part("EMPTY", false), lit("b")]), 0),
            ["ab"]
        );
    }

    #[test]
    fn status_expands_as_a_plain_integer() {
        assert_eq!(expand_one(word(vec![status_part(false)]), 42), ["42"]);
        assert_eq!(expand_one(word(vec![status_part(true)]), 0), ["0"]);
    }

    #[test]
    fn expand_words_flattens_and_can_produce_fewer_fields_than_words() {
        let words = [
            word(vec![lit("echo")]),
            word(vec![var_part("MISSING", false)]),
            word(vec![lit("a"), var_part("FOO", false)]),
        ];
        assert_eq!(
            expand_words(&words, &mut TestEnv::new()).unwrap(),
            ["echo", "abar"]
        );
    }

    // ---- assignment values and redirect targets ----

    #[test]
    fn an_assignment_value_is_one_string_never_split_or_vanished() {
        let mut env = TestEnv::new();
        assert_eq!(
            expand_assignment_value(&word(vec![var_part("MULTI", false)]), &mut env).unwrap(),
            "a b c"
        );
        assert_eq!(
            expand_assignment_value(&word(vec![var_part("MISSING", false)]), &mut env).unwrap(),
            ""
        );
        assert_eq!(
            expand_assignment_value(
                &word(vec![
                    lit("x="),
                    var_part("FOO", false),
                    subst_part("words", false)
                ]),
                &mut env
            )
            .unwrap(),
            "x=bara b\nc"
        );
    }

    #[test]
    fn a_redirect_target_must_be_exactly_one_field() {
        let mut env = TestEnv::new();
        assert_eq!(
            expand_target(
                &word(vec![lit("f"), subst_part("echo hi", false)]),
                &mut env
            ),
            Ok("fhi".into())
        );
        assert_eq!(
            expand_target(&word(vec![var_part("MULTI", false)]), &mut env),
            Err("$MULTI: ambiguous redirect".into())
        );
        assert!(expand_target(&word(vec![var_part("EMPTY", false)]), &mut env).is_err());
        assert_eq!(
            expand_target(&word(vec![var_part("EMPTY", true)]), &mut env),
            Ok(String::new())
        );
    }

    // ---- command substitution ----

    #[test]
    fn a_substitutions_trailing_newlines_are_all_removed() {
        assert_eq!(
            expand_one(word(vec![subst_part("echo hi", true)]), 0),
            ["hi"]
        );
        assert_eq!(
            expand_one(word(vec![subst_part("inner-newlines", true)]), 0),
            ["x\n\ny"]
        );
    }

    #[test]
    fn a_quoted_substitution_is_one_field_and_an_unquoted_one_splits_or_vanishes() {
        assert_eq!(
            expand_one(word(vec![subst_part("words", true)]), 0),
            ["a b\nc"]
        );
        assert_eq!(expand_one(word(vec![subst_part("empty", true)]), 0), [""]);
        assert_eq!(
            expand_one(word(vec![subst_part("words", false)]), 0),
            ["a", "b", "c"]
        );
        assert_eq!(
            expand_one(word(vec![subst_part("blank", false)]), 0),
            none()
        );
    }

    #[test]
    fn a_failing_substitution_stops_the_expansion() {
        let words = [
            word(vec![lit("a")]),
            word(vec![subst_part("boom", false)]),
            word(vec![subst_part("echo hi", false)]),
        ];
        let mut env = TestEnv::new();
        assert_eq!(
            expand_fields(&words, &mut env),
            Err("substitution failed".into())
        );
        assert_eq!(env.ran, ["boom"]); // the one after it never ran
    }

    #[test]
    fn substitutions_run_in_word_order() {
        let words = [
            word(vec![subst_part("echo hi", false)]),
            word(vec![subst_part("words", false)]),
        ];
        let mut env = TestEnv::new();
        expand_fields(&words, &mut env).unwrap();
        assert_eq!(env.ran, ["echo hi", "words"]);
    }

    // ---- quoting and filename patterns ----

    #[test]
    fn unquoted_text_is_pattern_text_and_quoted_text_is_escaped() {
        assert_eq!(
            patterns(word(vec![lit("*.c")])),
            [("*.c".into(), "*.c".into())]
        );
        assert_eq!(
            patterns(word(vec![quoted_lit("*.c")])),
            [("*.c".into(), "\\*.c".into())]
        );
        assert_eq!(
            patterns(word(vec![quoted_lit("a b"), lit("*")])),
            [("a b*".into(), "a b*".into())]
        );
    }

    #[test]
    fn an_unquoted_expansion_with_a_wildcard_is_a_pattern_and_a_quoted_one_is_not() {
        let mut env = TestEnv::new();
        env.set("STAR", "*");
        let bare = expand_word(&word(vec![var_part("STAR", false)]), &mut env).unwrap();
        assert!(bare[0].is_pattern());
        let quoted = expand_word(&word(vec![var_part("STAR", true)]), &mut env).unwrap();
        assert!(!quoted[0].is_pattern());
        assert_eq!(quoted[0].text, "*");
        // The same for a substitution's result.
        let bare = expand_word(&word(vec![subst_part("star", false)]), &mut env).unwrap();
        assert!(bare[0].is_pattern());
        let quoted = expand_word(&word(vec![subst_part("star", true)]), &mut env).unwrap();
        assert!(!quoted[0].is_pattern());
    }

    #[test]
    fn splitting_makes_each_piece_its_own_pattern() {
        let mut env = TestEnv::new();
        env.set("TWO", "a* b?");
        let fields = expand_word(&word(vec![var_part("TWO", false)]), &mut env).unwrap();
        let pats: Vec<&str> = fields.iter().map(|f| f.pattern.as_str()).collect();
        assert_eq!(pats, ["a*", "b?"]);
    }

    // ---- tilde ----

    #[test]
    fn a_tilde_is_home_and_is_not_a_pattern() {
        assert_eq!(
            patterns(word(vec![Part::Home, lit("/*.c")])),
            [("/root/*.c".into(), "/root/*.c".into())]
        );
        let mut env = TestEnv::new();
        env.set("HOME", "/we ird*");
        let fields = expand_word(&word(vec![Part::Home]), &mut env).unwrap();
        assert_eq!(fields[0].text, "/we ird*"); // one field, not split
        assert!(!fields[0].is_pattern());
    }

    #[test]
    fn a_tilde_with_no_home_stays_a_tilde() {
        let mut env = TestEnv::new();
        env.vars.retain(|(k, _)| k != "HOME");
        assert_eq!(texts(word(vec![Part::Home, lit("/x")]), &mut env), ["~/x"]);
        env.set("HOME", "");
        assert_eq!(texts(word(vec![Part::Home]), &mut env), ["~"]);
    }

    // ---- ${NAME op word} and ${#NAME} ----

    fn colon(op: fn(bool) -> ParamOp) -> ParamOp {
        op(true)
    }
    fn plain(op: fn(bool) -> ParamOp) -> ParamOp {
        op(false)
    }
    fn default(colon: bool) -> ParamOp {
        ParamOp::Default { colon }
    }
    fn alternate(colon: bool) -> ParamOp {
        ParamOp::Alternate { colon }
    }
    fn assign(colon: bool) -> ParamOp {
        ParamOp::Assign { colon }
    }
    fn error(colon: bool) -> ParamOp {
        ParamOp::Error { colon }
    }

    #[test]
    fn a_default_is_used_when_the_variable_is_unset_and_with_a_colon_when_empty() {
        let w = |name: &str, op| word(vec![param(name, op, vec![lit("dflt")], false)]);
        assert_eq!(expand_one(w("FOO", colon(default)), 0), ["bar"]);
        assert_eq!(expand_one(w("MISSING", colon(default)), 0), ["dflt"]);
        assert_eq!(expand_one(w("MISSING", plain(default)), 0), ["dflt"]);
        assert_eq!(expand_one(w("EMPTY", colon(default)), 0), ["dflt"]);
        // Without the colon an empty value is a value.
        assert_eq!(expand_one(w("EMPTY", plain(default)), 0), none());
    }

    #[test]
    fn an_alternate_is_used_only_when_there_is_a_value() {
        let w = |name: &str, op| word(vec![param(name, op, vec![lit("alt")], false)]);
        assert_eq!(expand_one(w("FOO", colon(alternate)), 0), ["alt"]);
        assert_eq!(expand_one(w("MISSING", colon(alternate)), 0), none());
        assert_eq!(expand_one(w("EMPTY", colon(alternate)), 0), none());
        assert_eq!(expand_one(w("EMPTY", plain(alternate)), 0), ["alt"]);
    }

    #[test]
    fn assign_stores_what_it_uses() {
        let mut env = TestEnv::new();
        let w = word(vec![param("NEW", colon(assign), vec![lit("v")], false)]);
        assert_eq!(texts(w.clone(), &mut env), ["v"]);
        assert_eq!(env.var("NEW"), Some("v".into()));
        // Already set: nothing assigned, the value comes back.
        env.set("NEW", "kept");
        assert_eq!(texts(w, &mut env), ["kept"]);
    }

    #[test]
    fn error_names_the_variable_and_says_why() {
        let mut env = TestEnv::new();
        let with_message = word(vec![param(
            "MISSING",
            colon(error),
            vec![lit("needed")],
            false,
        )]);
        assert_eq!(
            expand_fields(&[with_message], &mut env),
            Err("MISSING: needed".into())
        );
        let bare = word(vec![param("MISSING", colon(error), vec![], false)]);
        assert_eq!(
            expand_fields(&[bare], &mut env),
            Err("MISSING: parameter null or not set".into())
        );
        let fine = word(vec![param("FOO", colon(error), vec![lit("x")], false)]);
        assert_eq!(texts(fine, &mut env), ["bar"]);
    }

    #[test]
    fn the_word_is_expanded_only_when_used() {
        let mut env = TestEnv::new();
        let used = word(vec![param(
            "FOO",
            colon(default),
            vec![subst_part("echo hi", false)],
            false,
        )]);
        assert_eq!(texts(used, &mut env), ["bar"]);
        assert!(env.ran.is_empty());
        let unused = word(vec![param(
            "MISSING",
            colon(default),
            vec![subst_part("echo hi", false)],
            false,
        )]);
        assert_eq!(texts(unused, &mut env), ["hi"]);
        assert_eq!(env.ran, ["echo hi"]);
    }

    #[test]
    fn the_word_can_hold_variables_and_nest() {
        let mut env = TestEnv::new();
        let inner = param("FOO", colon(default), vec![lit("x")], true);
        let outer = word(vec![param(
            "MISSING",
            colon(default),
            vec![lit("<"), inner, quoted_lit(">")],
            true,
        )]);
        assert_eq!(texts(outer, &mut env), ["<bar>"]);
    }

    #[test]
    fn an_unquoted_default_with_blanks_splits_and_a_quoted_one_does_not() {
        let mut env = TestEnv::new();
        let w = |quoted: bool| {
            // As the lexer produces them: the word's text is quoted when the expansion is.
            let text = if quoted {
                quoted_lit("a b")
            } else {
                lit("a b")
            };
            word(vec![param("MISSING", colon(default), vec![text], quoted)])
        };
        assert_eq!(texts(w(false), &mut env), ["a", "b"]);
        assert_eq!(texts(w(true), &mut env), ["a b"]);
        // Quotes inside an unquoted word are kept as they are written.
        let inner = word(vec![param(
            "MISSING",
            colon(default),
            vec![quoted_lit("a  b")],
            false,
        )]);
        assert_eq!(texts(inner, &mut env), ["a  b"]);
    }

    #[test]
    fn length_counts_characters() {
        let mut env = TestEnv::new();
        env.set("JP", "日本語");
        assert_eq!(
            texts(word(vec![Part::Length { name: "FOO".into() }]), &mut env),
            ["3"]
        );
        assert_eq!(
            texts(word(vec![Part::Length { name: "JP".into() }]), &mut env),
            ["3"]
        );
        assert_eq!(
            texts(
                word(vec![Part::Length {
                    name: "MISSING".into()
                }]),
                &mut env
            ),
            ["0"]
        );
    }

    // ---- positional parameters ----

    #[test]
    fn positional_parameters_are_variables() {
        let mut env = TestEnv::new().with_args(&["one", "two"]);
        assert_eq!(texts(word(vec![var_part("1", false)]), &mut env), ["one"]);
        assert_eq!(texts(word(vec![var_part("2", true)]), &mut env), ["two"]);
        assert_eq!(texts(word(vec![var_part("3", false)]), &mut env), none());
        assert_eq!(texts(word(vec![var_part("#", false)]), &mut env), ["2"]);
        assert_eq!(
            texts(word(vec![var_part("0", false)]), &mut env),
            ["script"]
        );
    }

    #[test]
    fn quoted_at_is_one_field_per_parameter() {
        let mut env = TestEnv::new().with_args(&["a b", "", "c"]);
        assert_eq!(
            texts(word(vec![var_part("@", true)]), &mut env),
            ["a b", "", "c"]
        );
        // Glued to the text around it: the first joins the prefix, the last the suffix.
        assert_eq!(
            texts(
                word(vec![quoted_lit("<"), var_part("@", true), quoted_lit(">")]),
                &mut env
            ),
            ["<a b", "", "c>"]
        );
    }

    #[test]
    fn quoted_at_with_no_parameters_vanishes_but_its_neighbours_stay() {
        let mut env = TestEnv::new();
        assert_eq!(texts(word(vec![var_part("@", true)]), &mut env), none());
        assert_eq!(
            texts(word(vec![quoted_lit("x"), var_part("@", true)]), &mut env),
            ["x"]
        );
    }

    #[test]
    fn quoted_star_is_one_field_and_unquoted_at_and_star_split() {
        let mut env = TestEnv::new().with_args(&["a b", "c"]);
        assert_eq!(texts(word(vec![var_part("*", true)]), &mut env), ["a b c"]);
        assert_eq!(
            texts(word(vec![var_part("@", false)]), &mut env),
            ["a", "b", "c"]
        );
        assert_eq!(
            texts(word(vec![var_part("*", false)]), &mut env),
            ["a", "b", "c"]
        );
    }

    #[test]
    fn a_default_can_name_a_positional_parameter() {
        let mut env = TestEnv::new().with_args(&["x"]);
        let w = word(vec![param("2", colon(default), vec![lit("none")], false)]);
        assert_eq!(texts(w, &mut env), ["none"]);
        let w = word(vec![param("1", colon(default), vec![lit("none")], false)]);
        assert_eq!(texts(w, &mut env), ["x"]);
        let w = word(vec![Part::Length { name: "@".into() }]);
        assert_eq!(texts(w, &mut env), ["1"]);
    }
}
