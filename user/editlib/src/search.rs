//! The two prompts beyond cut/copy/paste: `^W`'s forward search and `^_`/`Alt+G`'s go-to-line.
//! Neither needs a mark or the cut buffer, so they get their own small module rather than crowding
//! into `region.rs`, which is about those two things specifically (its own doc comment says so).
//!
//! Pure `no_std` + `alloc`, so it is tested on the host (`cargo test`).

use crate::buffer::Buffer;

/// Searches for `term` starting at `from` (inclusive: a fresh search may match right where the
/// cursor already sits, but a repeated one won't re-find the same occurrence, because the caller
/// leaves the cursor at a match's *end*, not its start -- see the editor's `do_search`), wrapping
/// around to the start of the buffer once. Case-sensitive substring match only, matching the
/// editor's chosen scope (no case-insensitive or backward search, no search-and-replace). `term`
/// empty never matches, since every position would otherwise "match" it.
///
/// Two passes rather than one modular loop over every line, so the wraparound's own boundary (stop
/// *before* `from` again, not at or past it) reads directly rather than through an index computed
/// mod the line count: pass one covers `from` to the end of the buffer, pass two covers the start of
/// the buffer up to (not including) `from`.
pub fn find(buffer: &Buffer, from: (usize, usize), term: &str) -> Option<(usize, usize)> {
    if term.is_empty() {
        return None;
    }
    let (from_line, from_col) = from;
    let total = buffer.line_count();

    for line in from_line..total {
        let text = buffer.line(line);
        let start = if line == from_line { from_col } else { 0 };
        if start <= text.len()
            && let Some(byte) = text[start..].find(term)
        {
            return Some((line, start + byte));
        }
    }
    for line in 0..=from_line {
        let text = buffer.line(line);
        let end = if line == from_line {
            from_col
        } else {
            text.len()
        };
        if let Some(byte) = text[..end.min(text.len())].find(term) {
            return Some((line, byte));
        }
    }
    None
}

/// The byte offset of character `col` (0-based) into `text` -- clamped to `text`'s own length past
/// its last character, the same way a too-large line or column clamps in [`parse_goto`].
fn char_to_byte(text: &str, col: usize) -> usize {
    text.char_indices()
        .nth(col)
        .map(|(i, _)| i)
        .unwrap_or(text.len())
}

/// Parses a go-to prompt's answer, `LINE` or `LINE,COL` (both 1-based, matching the status bar and
/// `^C`'s message everywhere else this editor shows a position), into a buffer position (a byte
/// offset on the named line). `None` only when `input` doesn't even start with a number -- `0` is
/// one such case, since lines and columns are 1-based and `0` names neither. An in-range-but-past-
/// the-end line or column *clamps* instead (the last line, or that line's own end), the same
/// forgiving rule [`Buffer::set_cursor`] already uses for a position computed some other way.
pub fn parse_goto(input: &str, buffer: &Buffer) -> Option<(usize, usize)> {
    let mut parts = input.splitn(2, ',');
    let line: usize = parts.next()?.trim().parse().ok()?;
    if line == 0 {
        return None;
    }
    let line = (line - 1).min(buffer.line_count() - 1);
    let text = buffer.line(line);

    let byte = match parts.next() {
        Some(col_str) => {
            let col: usize = col_str.trim().parse().ok()?;
            if col == 0 {
                return None;
            }
            char_to_byte(text, col - 1)
        }
        None => 0,
    };
    Some((line, byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer_of(lines: &[&str]) -> Buffer {
        Buffer::from_text(&lines.join("\n"))
    }

    #[test]
    fn finds_the_first_match_at_or_after_the_start() {
        let b = buffer_of(&["one two", "two three"]);
        assert_eq!(find(&b, (0, 0), "two"), Some((0, 4)));
        assert_eq!(find(&b, (0, 4), "two"), Some((0, 4))); // inclusive of the start itself
        assert_eq!(find(&b, (0, 5), "two"), Some((1, 0))); // past it: skips ahead to the next one
    }

    #[test]
    fn crosses_lines_forward() {
        let b = buffer_of(&["aaa", "bbb", "ccc"]);
        assert_eq!(find(&b, (0, 0), "bbb"), Some((1, 0)));
        assert_eq!(find(&b, (1, 1), "ccc"), Some((2, 0)));
    }

    #[test]
    fn wraps_around_once_and_stops_before_where_it_started() {
        let b = buffer_of(&["target here", "nothing"]);
        // Starting just past the only match: wraps around and finds it again, from the top.
        assert_eq!(find(&b, (0, 7), "target"), Some((0, 0)));
    }

    #[test]
    fn a_term_with_no_occurrence_anywhere_is_not_found() {
        let b = buffer_of(&["abc", "def"]);
        assert_eq!(find(&b, (0, 0), "xyz"), None);
    }

    #[test]
    fn an_empty_term_never_matches() {
        let b = buffer_of(&["abc"]);
        assert_eq!(find(&b, (0, 0), ""), None);
    }

    #[test]
    fn a_wide_character_counts_as_one_match_unit_like_any_other() {
        let b = buffer_of(&["a日b"]);
        assert_eq!(find(&b, (0, 0), "日"), Some((0, 1)));
    }

    #[test]
    fn goto_with_only_a_line_lands_at_its_start() {
        let b = buffer_of(&["one", "two", "three"]);
        assert_eq!(parse_goto("2", &b), Some((1, 0)));
    }

    #[test]
    fn goto_with_a_column_lands_on_the_named_character() {
        let b = buffer_of(&["one", "two three"]);
        assert_eq!(parse_goto("2,5", &b), Some((1, 4))); // "two three"[4] == 't' of "three"
    }

    #[test]
    fn goto_clamps_a_line_past_the_end_to_the_last_one() {
        let b = buffer_of(&["one", "two"]);
        assert_eq!(parse_goto("99", &b), Some((1, 0)));
    }

    #[test]
    fn goto_clamps_a_column_past_the_lines_end_to_its_own_end() {
        let b = buffer_of(&["abc"]);
        assert_eq!(parse_goto("1,99", &b), Some((0, 3)));
    }

    #[test]
    fn goto_rejects_zero_and_non_numeric_input() {
        let b = buffer_of(&["abc"]);
        assert_eq!(parse_goto("0", &b), None);
        assert_eq!(parse_goto("x", &b), None);
        assert_eq!(parse_goto("", &b), None);
        assert_eq!(parse_goto("1,0", &b), None);
        assert_eq!(parse_goto("1,x", &b), None);
    }

    #[test]
    fn goto_ignores_surrounding_whitespace() {
        let b = buffer_of(&["one", "two three"]);
        assert_eq!(parse_goto(" 2 , 5 ", &b), Some((1, 4)));
    }
}
