//! Turns lexed `Word`s into real argument strings, against live variable state and the current
//! `$?` -- real POSIX field-splitting: an unquoted expansion's value splits on runs of blanks,
//! and an all-blank/empty result vanishes entirely (no field at all, not an empty one); a quoted
//! expansion always stays glued into exactly one field, even if its value is empty. Literal text
//! never splits here -- that already happened once, at lex time (`lexer`'s own doc comment) --
//! only an expansion's *resolved value*, unknown until now, can still need it.
//!
//! Pure logic: the actual variable/`$?` lookup is a parameter (`var`/`status` below), supplied by
//! the real call site (`shell::mod`, bare-metal only, backed by `frame_stack::var`/
//! `shell::last_status`) rather than reached into directly -- host-tested the same way `fs::
//! sector`/`exec::argplan` already are.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use super::lexer::{Part, Word};

/// Expands every word in `words` into real argument strings, in order -- a word that vanishes
/// entirely (an unquoted expansion whose value is empty/all-blank, and nothing else in that word)
/// contributes no argument at all, so the output can have fewer entries than `words` itself.
pub fn expand_words(
    words: &[Word],
    var: &impl Fn(&str) -> Option<String>,
    status: i32,
) -> Vec<String> {
    words
        .iter()
        .flat_map(|word| expand_word(word, var, status))
        .collect()
}

/// Expands `word` (an assignment's *value* half, from `Word::assignment`) into one string, with
/// no field-splitting at all, quoted or not -- matches the lab's own `expand_assignment`: there's
/// nowhere for a split piece to go (an assignment is one value, not an argv position), so
/// `A=$MULTI` assigns the whole value as one string even though the same `$MULTI` unquoted in an
/// ordinary argument position would split into several.
pub fn expand_assignment_value(
    word: &Word,
    var: &impl Fn(&str) -> Option<String>,
    status: i32,
) -> String {
    let mut s = String::new();
    for part in &word.parts {
        match part {
            Part::Lit(text) => s.push_str(text),
            Part::Var { name, .. } => s.push_str(&var(name).unwrap_or_default()),
            Part::Status { .. } => s.push_str(&status.to_string()),
        }
    }
    s
}

/// Expands one word into zero or more fields. `parts` empty (only ever an explicitly empty quoted
/// word, `""`/`''` -- `lexer::Word`'s own doc comment) is always exactly one empty field, never
/// zero: that distinction -- "a word that happens to be empty" vs. "nothing here at all" -- is
/// exactly what quoting decides, and an empty `Word` only ever arises from the former.
fn expand_word(word: &Word, var: &impl Fn(&str) -> Option<String>, status: i32) -> Vec<String> {
    if word.parts.is_empty() {
        return vec![String::new()];
    }
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut have_current = false;
    for part in &word.parts {
        match part {
            Part::Lit(text) => {
                current.push_str(text);
                have_current = true;
            }
            Part::Var { name, quoted } => {
                let value = var(name).unwrap_or_default();
                glue(
                    &mut fields,
                    &mut current,
                    &mut have_current,
                    &value,
                    *quoted,
                );
            }
            Part::Status { quoted } => {
                let value = status.to_string();
                glue(
                    &mut fields,
                    &mut current,
                    &mut have_current,
                    &value,
                    *quoted,
                );
            }
        }
    }
    if have_current {
        fields.push(current);
    }
    fields
}

/// Glues one expansion's `value` onto `current` (the field being built so far), splitting it
/// first if `quoted` is `false`. A quoted expansion -- even an empty one -- always starts or
/// extends a real field (`have_current = true` unconditionally): that's the one case this
/// function exists to get right, since an *unquoted* empty value does the opposite (contributes
/// nothing at all if nothing else in the word has started a field yet).
fn glue(
    fields: &mut Vec<String>,
    current: &mut String,
    have_current: &mut bool,
    value: &str,
    quoted: bool,
) {
    if quoted {
        current.push_str(value);
        *have_current = true;
        return;
    }
    let mut pieces = value.split_whitespace();
    let Some(first) = pieces.next() else {
        return; // empty/all-blank unquoted value -- vanishes, touching nothing
    };
    current.push_str(first);
    *have_current = true;
    for piece in pieces {
        fields.push(core::mem::take(current));
        current.push_str(piece);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::lexer::Word;

    fn lit(text: &str) -> Part {
        Part::Lit(text.into())
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
    fn word(parts: Vec<Part>) -> Word {
        Word {
            parts,
            assign: None,
        }
    }

    fn lookup(name: &str) -> Option<String> {
        match name {
            "FOO" => Some("bar".into()),
            "MULTI" => Some("a b c".into()),
            "EMPTY" => Some(String::new()),
            _ => None,
        }
    }

    fn expand_one(w: Word, status: i32) -> Vec<String> {
        expand_word(&w, &lookup, status)
    }

    #[test]
    fn a_word_with_no_expansion_is_one_field() {
        assert_eq!(expand_one(word(vec![lit("hello")]), 0), ["hello"]);
    }

    #[test]
    fn an_empty_quoted_word_is_one_empty_field() {
        assert_eq!(expand_one(word(vec![]), 0), [""]);
    }

    #[test]
    fn unquoted_unset_variable_vanishes_entirely() {
        let fields: Vec<String> = Vec::new();
        assert_eq!(
            expand_one(word(vec![var_part("MISSING", false)]), 0),
            fields
        );
    }

    #[test]
    fn quoted_unset_variable_is_one_empty_field() {
        assert_eq!(expand_one(word(vec![var_part("MISSING", true)]), 0), [""]);
    }

    #[test]
    fn unquoted_empty_variable_vanishes_same_as_unset() {
        let fields: Vec<String> = Vec::new();
        assert_eq!(expand_one(word(vec![var_part("EMPTY", false)]), 0), fields);
    }

    #[test]
    fn quoted_empty_variable_is_one_empty_field() {
        assert_eq!(expand_one(word(vec![var_part("EMPTY", true)]), 0), [""]);
    }

    #[test]
    fn a_set_variable_expands_to_its_value() {
        assert_eq!(expand_one(word(vec![var_part("FOO", false)]), 0), ["bar"]);
        assert_eq!(expand_one(word(vec![var_part("FOO", true)]), 0), ["bar"]);
    }

    #[test]
    fn unquoted_multi_word_value_splits_into_several_fields() {
        assert_eq!(
            expand_one(word(vec![var_part("MULTI", false)]), 0),
            ["a", "b", "c"]
        );
    }

    #[test]
    fn quoted_multi_word_value_stays_one_field() {
        assert_eq!(
            expand_one(word(vec![var_part("MULTI", true)]), 0),
            ["a b c"]
        );
    }

    #[test]
    fn literal_text_glues_to_the_first_and_last_split_piece() {
        // `x$MULTIy` -- MULTI = "a b c" -- splits into "xa", "b", "cy".
        assert_eq!(
            expand_one(word(vec![lit("x"), var_part("MULTI", false), lit("y")]), 0),
            ["xa", "b", "cy"]
        );
    }

    #[test]
    fn surrounding_literal_text_survives_an_unquoted_empty_expansion() {
        // `a$MISSINGb` -- MISSING unset -- is just "ab", not ["a", "b"] or ["ab", ""].
        assert_eq!(
            expand_one(
                word(vec![lit("a"), var_part("MISSING", false), lit("b")]),
                0
            ),
            ["ab"]
        );
    }

    #[test]
    fn status_expands_as_a_plain_integer() {
        assert_eq!(expand_one(word(vec![status_part(false)]), 0), ["0"]);
        assert_eq!(expand_one(word(vec![status_part(false)]), 127), ["127"]);
        assert_eq!(expand_one(word(vec![status_part(true)]), 1), ["1"]);
    }

    #[test]
    fn expand_words_flattens_and_can_produce_fewer_fields_than_words() {
        let words = vec![
            word(vec![lit("echo")]),
            word(vec![var_part("MISSING", false)]),
            word(vec![lit("a"), var_part("FOO", false)]),
        ];
        assert_eq!(expand_words(&words, &lookup, 0), ["echo", "abar"]);
    }

    #[test]
    fn assignment_value_does_not_split_a_multi_word_variable() {
        assert_eq!(
            expand_assignment_value(&word(vec![var_part("MULTI", false)]), &lookup, 0),
            "a b c"
        );
    }

    #[test]
    fn assignment_value_of_an_unset_variable_is_empty_not_vanished() {
        assert_eq!(
            expand_assignment_value(&word(vec![var_part("MISSING", false)]), &lookup, 0),
            ""
        );
    }

    #[test]
    fn assignment_value_can_glue_literal_text_and_an_expansion() {
        assert_eq!(
            expand_assignment_value(&word(vec![lit("x="), var_part("FOO", false)]), &lookup, 0),
            "x=bar"
        );
    }
}
