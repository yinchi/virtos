//! The editor's in-memory text: a `Vec` of lines (never containing `\n`), a cursor `(line, col)`
//! (`col` a byte offset into `lines[line]`, always on a char boundary -- the same convention
//! `keyboard/line.rs`'s `LineBuffer` uses for its one line), and a modified flag.
//!
//! What crosses a line boundary and what doesn't: Left/Right and Home/End move the cursor across
//! lines without changing any text; only Backspace at column 0 and Delete at end-of-line actually
//! join two lines into one. Everything above a single line -- soft-wrap, scrolling, the mark and
//! cut buffer -- lives in `layout.rs`, `scroll.rs` and `region.rs`, which build on this module's
//! public cursor and line access rather than reaching into its fields.
//!
//! Pure `no_std` + `alloc`, so it is tested on the host (`cargo test`).

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

/// The text, a cursor into it, and whether it has unsaved changes.
pub struct Buffer {
    lines: Vec<String>,
    cursor_line: usize,
    cursor_col: usize,
    modified: bool,
}

impl Buffer {
    /// One empty line, cursor at its start -- a brand new file.
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor_line: 0,
            cursor_col: 0,
            modified: false,
        }
    }

    /// Splits `text` into lines on `\n`. The inverse of [`Buffer::to_text`]: encoding then decoding
    /// (or the reverse) reproduces the same lines exactly, including an empty file (one empty
    /// line) and a file whose last line is itself empty (a trailing blank line, distinct from the
    /// newline that ends every line). `text` must already be LF-only (the caller's job -- see
    /// `abi::ioctl::Cell`'s neighbour, the editor's own file-reading step, which strips `\r` and
    /// replaces invalid UTF-8 before this ever sees it).
    pub fn from_text(text: &str) -> Self {
        let lines = if text.is_empty() {
            vec![String::new()]
        } else {
            let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
            if text.ends_with('\n') {
                lines.pop(); // the split's trailing empty element is the terminator, not a line
            }
            lines
        };
        Self {
            lines,
            cursor_line: 0,
            cursor_col: 0,
            modified: false,
        }
    }

    /// The lines joined with `\n`, with one final `\n` always appended (POSIX's "a line is
    /// newline-terminated", and what makes this the exact inverse of [`Buffer::from_text`]).
    pub fn to_text(&self) -> String {
        let mut text = self.lines.join("\n");
        text.push('\n');
        text
    }

    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// Clears the modified flag -- called once a save actually succeeds.
    pub fn mark_saved(&mut self) {
        self.modified = false;
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn line(&self, index: usize) -> &str {
        &self.lines[index]
    }

    pub fn current_line(&self) -> &str {
        &self.lines[self.cursor_line]
    }

    /// The cursor's `(line, col)`; `col` is a byte offset into `line(line)`.
    pub fn cursor(&self) -> (usize, usize) {
        (self.cursor_line, self.cursor_col)
    }

    /// Places the cursor at `(line, col)`, clamping both into range: `line` to the last line, then
    /// `col` to that line's length and back to the nearest character boundary at or before it. For
    /// a caller (`layout.rs`'s vertical movement, `region.rs`'s paste) that computes a position
    /// from something other than this buffer's own editing methods, so it never has to duplicate
    /// this clamping itself.
    pub fn set_cursor(&mut self, line: usize, col: usize) {
        self.cursor_line = line.min(self.lines.len() - 1);
        let text = &self.lines[self.cursor_line];
        let mut col = col.min(text.len());
        while col > 0 && !text.is_char_boundary(col) {
            col -= 1;
        }
        self.cursor_col = col;
    }

    fn prev_boundary(&self) -> Option<usize> {
        self.lines[self.cursor_line][..self.cursor_col]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
    }

    fn next_boundary(&self) -> Option<usize> {
        let text = &self.lines[self.cursor_line];
        let c = text[self.cursor_col..].chars().next()?;
        Some(self.cursor_col + c.len_utf8())
    }

    /// Inserts `c` at the cursor and advances past it. `c` must not be `\n` -- a line never
    /// contains one; Enter is [`Buffer::split_line`], a distinct operation, not a character.
    pub fn insert_char(&mut self, c: char) {
        debug_assert_ne!(c, '\n', "a line boundary is split_line, not a character");
        self.lines[self.cursor_line].insert(self.cursor_col, c);
        self.cursor_col += c.len_utf8();
        self.modified = true;
    }

    /// Removes the character before the cursor. At column 0 of a line after the first, this joins
    /// the current line onto the end of the previous one instead (the cursor lands where they met).
    /// Returns whether anything changed (`false` only at the very start of the buffer).
    pub fn backspace(&mut self) -> bool {
        if let Some(start) = self.prev_boundary() {
            self.lines[self.cursor_line].remove(start);
            self.cursor_col = start;
            self.modified = true;
            true
        } else if self.cursor_line > 0 {
            let current = self.lines.remove(self.cursor_line);
            self.cursor_line -= 1;
            self.cursor_col = self.lines[self.cursor_line].len();
            self.lines[self.cursor_line].push_str(&current);
            self.modified = true;
            true
        } else {
            false
        }
    }

    /// Removes the character at the cursor (the cursor doesn't move). At the end of a line before
    /// the last, this joins the next line onto the end of the current one instead. Returns whether
    /// anything changed (`false` only at the very end of the buffer).
    pub fn delete_forward(&mut self) -> bool {
        if self.next_boundary().is_some() {
            self.lines[self.cursor_line].remove(self.cursor_col);
            self.modified = true;
            true
        } else if self.cursor_line + 1 < self.lines.len() {
            let next = self.lines.remove(self.cursor_line + 1);
            self.lines[self.cursor_line].push_str(&next);
            self.modified = true;
            true
        } else {
            false
        }
    }

    /// Enter: splits the current line at the cursor into two. The new line gets whatever followed
    /// the cursor; with `auto_indent`, it is also prefixed with the *original* line's leading
    /// spaces and tabs -- unless that line was nothing but whitespace, in which case nothing is
    /// copied (nano's rule: otherwise indentation would grow without bound from repeated Enters on
    /// a blank line). The cursor lands right after any copied indent.
    pub fn split_line(&mut self, auto_indent: bool) {
        let indent = if auto_indent {
            leading_whitespace(&self.lines[self.cursor_line])
        } else {
            String::new()
        };
        let tail = self.lines[self.cursor_line][self.cursor_col..].to_string();
        self.lines[self.cursor_line].truncate(self.cursor_col);
        let mut new_line = indent;
        let indent_len = new_line.len();
        new_line.push_str(&tail);
        self.lines.insert(self.cursor_line + 1, new_line);
        self.cursor_line += 1;
        self.cursor_col = indent_len;
        self.modified = true;
    }

    /// Removes line `index` entirely and returns its text. The buffer always keeps at least one
    /// line, so removing the only one leaves a fresh empty line rather than none. The cursor, if it
    /// was at or after the removed line, moves to the start of whatever now occupies that position
    /// (clamped to the last line). Used by `region.rs` for both a plain line-cut (`^K`) and, one
    /// line at a time, a multi-line region cut.
    pub fn remove_line(&mut self, index: usize) -> String {
        let removed = if self.lines.len() == 1 {
            core::mem::take(&mut self.lines[0])
        } else {
            self.lines.remove(index)
        };
        self.modified = true;
        match self.cursor_line.cmp(&index) {
            // Before the removed line: unaffected.
            core::cmp::Ordering::Less => {}
            // Was on the removed line: whatever now occupies that position, start of it.
            core::cmp::Ordering::Equal => self.cursor_col = 0,
            // After it: shifts up by one, but keeps its own column.
            core::cmp::Ordering::Greater => self.cursor_line -= 1,
        }
        // Only reachable by the `Equal` arm removing what was also the last line.
        self.cursor_line = self.cursor_line.min(self.lines.len() - 1);
        removed
    }

    /// Inserts a new line holding `text` at `index` (`region.rs`'s paste, and undoing a line-cut).
    pub fn insert_line(&mut self, index: usize, text: String) {
        self.lines.insert(index, text);
        self.modified = true;
    }

    /// Replaces line `index`'s whole text -- `region.rs`'s way to apply a region edit (a same-line
    /// deletion, or a multi-line one's surviving ends merged into one) in a single step, rather
    /// than one character at a time.
    pub fn set_line(&mut self, index: usize, text: String) {
        self.lines[index] = text;
        self.modified = true;
    }

    pub fn move_left(&mut self) -> bool {
        if let Some(start) = self.prev_boundary() {
            self.cursor_col = start;
            true
        } else if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.lines[self.cursor_line].len();
            true
        } else {
            false
        }
    }

    pub fn move_right(&mut self) -> bool {
        if let Some(end) = self.next_boundary() {
            self.cursor_col = end;
            true
        } else if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            self.cursor_col = 0;
            true
        } else {
            false
        }
    }

    /// Start of the *logical* line (not the current screen row -- see `layout.rs`).
    pub fn move_home(&mut self) -> bool {
        if self.cursor_col == 0 {
            return false;
        }
        self.cursor_col = 0;
        true
    }

    /// End of the *logical* line.
    pub fn move_end(&mut self) -> bool {
        let end = self.lines[self.cursor_line].len();
        if self.cursor_col == end {
            return false;
        }
        self.cursor_col = end;
        true
    }

    /// Moves back to the start of the previous word (readline's/`keyboard/line.rs`'s
    /// backward-word: skip anything that isn't part of a word, then the word itself; a word is a
    /// run of letters and digits), crossing into the previous line -- one line at a time, so a run
    /// of blank or punctuation-only lines is still one stop per line, not skipped over -- once
    /// there is nothing left to skip on the current one.
    pub fn move_word_left(&mut self) -> bool {
        if self.cursor_col == 0 {
            if self.cursor_line == 0 {
                return false;
            }
            // Crossing a line boundary is its own stop, landing exactly at the previous line's
            // end -- the same one-boundary-per-call shape `move_left` already has -- rather than
            // also skipping straight into that line's own last word in the same call.
            self.cursor_line -= 1;
            self.cursor_col = self.lines[self.cursor_line].len();
            return true;
        }
        let text = &self.lines[self.cursor_line];
        let after_blanks = back_while(text, self.cursor_col, |c| !is_word(c));
        let after_word = back_while(text, after_blanks, is_word);
        let moved = after_word != self.cursor_col;
        self.cursor_col = after_word;
        moved
    }

    /// Moves forward to the end of the next word, the mirror of [`Buffer::move_word_left`].
    pub fn move_word_right(&mut self) -> bool {
        let len = self.lines[self.cursor_line].len();
        if self.cursor_col == len {
            if self.cursor_line + 1 >= self.lines.len() {
                return false;
            }
            self.cursor_line += 1;
            self.cursor_col = 0;
            return true;
        }
        let text = &self.lines[self.cursor_line];
        let after_blanks = forward_while(text, self.cursor_col, |c| !is_word(c));
        let after_word = forward_while(text, after_blanks, is_word);
        let moved = after_word != self.cursor_col;
        self.cursor_col = after_word;
        moved
    }

    pub fn move_to_first_line(&mut self) -> bool {
        if self.cursor_line == 0 && self.cursor_col == 0 {
            return false;
        }
        self.cursor_line = 0;
        self.cursor_col = 0;
        true
    }

    pub fn move_to_last_line(&mut self) -> bool {
        let last = self.lines.len() - 1;
        let end = self.lines[last].len();
        if self.cursor_line == last && self.cursor_col == end {
            return false;
        }
        self.cursor_line = last;
        self.cursor_col = end;
        true
    }
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}

/// A character of a "word" for word movement: a letter or a digit -- the same rule
/// `keyboard/line.rs`'s own word movement uses.
fn is_word(c: char) -> bool {
    c.is_alphanumeric()
}

/// The byte offset reached by moving back from `from` over the characters of `text` that `pred`
/// accepts.
fn back_while(text: &str, mut from: usize, pred: impl Fn(char) -> bool) -> usize {
    while let Some((i, c)) = text[..from].char_indices().next_back() {
        if !pred(c) {
            break;
        }
        from = i;
    }
    from
}

/// The byte offset reached by moving forward from `from` over the characters of `text` that `pred`
/// accepts.
fn forward_while(text: &str, mut from: usize, pred: impl Fn(char) -> bool) -> usize {
    while let Some(c) = text[from..].chars().next() {
        if !pred(c) {
            break;
        }
        from += c.len_utf8();
    }
    from
}

/// `line`'s leading run of spaces and tabs -- empty if `line` is nothing but whitespace (or is
/// empty), which is [`Buffer::split_line`]'s cue not to copy it.
fn leading_whitespace(line: &str) -> String {
    if line.trim_start_matches([' ', '\t']).is_empty() {
        return String::new();
    }
    line.chars()
        .take_while(|&c| c == ' ' || c == '\t')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_buffer_is_one_empty_unmodified_line() {
        let b = Buffer::new();
        assert_eq!(b.line_count(), 1);
        assert_eq!(b.line(0), "");
        assert_eq!(b.cursor(), (0, 0));
        assert!(!b.is_modified());
    }

    #[test]
    fn from_text_and_to_text_are_exact_inverses() {
        // What `to_text` should reproduce for each `text`: itself, if it already ends in `\n`
        // (including the empty-file case, which becomes the one-blank-line file "\n"); otherwise
        // itself with one `\n` appended.
        let cases = [
            ("", "\n"),
            ("\n", "\n"),
            ("a", "a\n"),
            ("a\n", "a\n"),
            ("a\nb", "a\nb\n"),
            ("a\nb\n", "a\nb\n"),
            ("a\n\n", "a\n\n"),
            ("\na", "\na\n"),
            ("a\n\nb\n", "a\n\nb\n"),
        ];
        for (text, want) in cases {
            assert_eq!(Buffer::from_text(text).to_text(), want, "{text:?}");
        }
    }

    #[test]
    fn from_text_splits_into_the_expected_lines() {
        fn lines(b: &Buffer) -> Vec<&str> {
            (0..b.line_count()).map(|i| b.line(i)).collect()
        }
        assert_eq!(lines(&Buffer::from_text("")), [""]);
        assert_eq!(lines(&Buffer::from_text("\n")), [""]);
        assert_eq!(lines(&Buffer::from_text("a")), ["a"]);
        assert_eq!(lines(&Buffer::from_text("a\n")), ["a"]);
        assert_eq!(lines(&Buffer::from_text("a\nb")), ["a", "b"]);
        assert_eq!(lines(&Buffer::from_text("a\nb\n")), ["a", "b"]);
        assert_eq!(lines(&Buffer::from_text("a\n\n")), ["a", ""]);
        assert_eq!(lines(&Buffer::from_text("\na")), ["", "a"]);
    }

    #[test]
    fn insert_advances_the_cursor_by_the_characters_width_in_bytes() {
        let mut b = Buffer::new();
        b.insert_char('a');
        b.insert_char('日');
        assert_eq!(b.line(0), "a日");
        assert_eq!(b.cursor(), (0, 1 + '日'.len_utf8()));
    }

    #[test]
    fn backspace_within_a_line_removes_the_character_before_the_cursor() {
        let mut b = Buffer::from_text("abc");
        b.set_cursor(0, 2);
        assert!(b.backspace());
        assert_eq!(b.line(0), "ac");
        assert_eq!(b.cursor(), (0, 1));
    }

    #[test]
    fn backspace_at_column_zero_joins_with_the_previous_line() {
        let mut b = Buffer::from_text("ab\ncd");
        b.set_cursor(1, 0);
        assert!(b.backspace());
        assert_eq!(b.line_count(), 1);
        assert_eq!(b.line(0), "abcd");
        assert_eq!(b.cursor(), (0, 2)); // where the join happened
    }

    #[test]
    fn backspace_at_the_very_start_of_the_buffer_does_nothing() {
        let mut b = Buffer::new();
        assert!(!b.backspace());
        assert_eq!(b.cursor(), (0, 0));
    }

    #[test]
    fn delete_forward_within_a_line_removes_the_character_at_the_cursor() {
        let mut b = Buffer::from_text("abc");
        b.set_cursor(0, 1);
        assert!(b.delete_forward());
        assert_eq!(b.line(0), "ac");
        assert_eq!(b.cursor(), (0, 1)); // the cursor doesn't move
    }

    #[test]
    fn delete_forward_at_end_of_line_joins_with_the_next_line() {
        let mut b = Buffer::from_text("ab\ncd");
        b.set_cursor(0, 2);
        assert!(b.delete_forward());
        assert_eq!(b.line_count(), 1);
        assert_eq!(b.line(0), "abcd");
        assert_eq!(b.cursor(), (0, 2)); // stays put
    }

    #[test]
    fn delete_forward_at_the_very_end_of_the_buffer_does_nothing() {
        let mut b = Buffer::from_text("ab");
        b.set_cursor(0, 2);
        assert!(!b.delete_forward());
    }

    #[test]
    fn split_line_without_auto_indent_moves_what_follows_the_cursor_to_a_new_line() {
        let mut b = Buffer::from_text("abcdef");
        b.set_cursor(0, 3);
        b.split_line(false);
        assert_eq!(b.line_count(), 2);
        assert_eq!(b.line(0), "abc");
        assert_eq!(b.line(1), "def");
        assert_eq!(b.cursor(), (1, 0));
    }

    #[test]
    fn split_line_with_auto_indent_copies_the_lines_leading_whitespace() {
        let mut b = Buffer::from_text("  if x:");
        b.set_cursor(0, 7); // end of the line
        b.split_line(true);
        assert_eq!(b.line(0), "  if x:");
        assert_eq!(b.line(1), "  "); // the copied indent, nothing followed the cursor
        assert_eq!(b.cursor(), (1, 2)); // right after the indent

        // The copied indent is the *original* line's, not "up to the cursor": splitting mid-line
        // still copies the whole leading run.
        let mut b = Buffer::from_text("    return x");
        b.set_cursor(0, 10); // between "return" and " x"
        b.split_line(true);
        assert_eq!(b.line(1), "     x"); // 4 spaces of indent + " x" (the tail, unchanged)
    }

    #[test]
    fn split_line_with_auto_indent_leaves_a_whitespace_only_lines_new_line_empty() {
        let mut b = Buffer::from_text("    ");
        b.set_cursor(0, 4);
        b.split_line(true);
        assert_eq!(b.line(0), "    ");
        assert_eq!(b.line(1), ""); // not copied: nothing would ever stop it growing
        assert_eq!(b.cursor(), (1, 0));

        let mut b = Buffer::new(); // a genuinely empty line
        b.split_line(true);
        assert_eq!(b.line(1), "");
    }

    #[test]
    fn remove_line_deletes_it_and_moves_the_cursor_if_it_was_there() {
        let mut b = Buffer::from_text("a\nb\nc");
        b.set_cursor(1, 1);
        assert_eq!(b.remove_line(1), "b");
        assert_eq!(b.line_count(), 2);
        assert_eq!((b.line(0), b.line(1)), ("a", "c"));
        assert_eq!(b.cursor(), (1, 0)); // was on the removed line; lands at the start of what's now there

        // Removing a line *before* the cursor shifts it, but doesn't reset its column.
        let mut b = Buffer::from_text("a\nb\nc");
        b.set_cursor(2, 1);
        b.remove_line(0);
        assert_eq!(b.cursor(), (1, 1));
    }

    #[test]
    fn removing_the_only_line_leaves_one_empty_line() {
        let mut b = Buffer::from_text("only");
        assert_eq!(b.remove_line(0), "only");
        assert_eq!(b.line_count(), 1);
        assert_eq!(b.line(0), "");
        assert_eq!(b.cursor(), (0, 0));
    }

    #[test]
    fn set_line_replaces_a_lines_whole_text() {
        let mut b = Buffer::from_text("a\nb\nc");
        b.set_line(1, "replaced".into());
        assert_eq!((b.line(0), b.line(1), b.line(2)), ("a", "replaced", "c"));
        assert!(b.is_modified());
    }

    #[test]
    fn insert_line_undoes_remove_line() {
        let mut b = Buffer::from_text("a\nc");
        b.insert_line(1, "b".into());
        assert_eq!((b.line(0), b.line(1), b.line(2)), ("a", "b", "c"));
    }

    #[test]
    fn move_left_right_cross_line_boundaries_without_joining_text() {
        let mut b = Buffer::from_text("ab\ncd");
        b.set_cursor(1, 0);
        assert!(b.move_left());
        assert_eq!(b.cursor(), (0, 2)); // end of the previous line
        assert_eq!(b.line_count(), 2); // nothing joined

        assert!(b.move_right());
        assert_eq!(b.cursor(), (1, 0)); // start of the next line
    }

    #[test]
    fn move_left_at_the_very_start_and_right_at_the_very_end_do_nothing() {
        let mut b = Buffer::from_text("ab");
        assert!(!b.move_left());
        b.set_cursor(0, 2);
        assert!(!b.move_right());
    }

    #[test]
    fn home_and_end_are_the_whole_logical_line() {
        let mut b = Buffer::from_text("abc");
        b.set_cursor(0, 1);
        assert!(b.move_end());
        assert_eq!(b.cursor(), (0, 3));
        assert!(!b.move_end()); // already there
        assert!(b.move_home());
        assert_eq!(b.cursor(), (0, 0));
        assert!(!b.move_home());
    }

    #[test]
    fn first_and_last_line_jump_across_the_whole_buffer() {
        let mut b = Buffer::from_text("a\nbb\nccc");
        b.set_cursor(1, 1);
        assert!(b.move_to_last_line());
        assert_eq!(b.cursor(), (2, 3));
        assert!(!b.move_to_last_line());
        assert!(b.move_to_first_line());
        assert_eq!(b.cursor(), (0, 0));
        assert!(!b.move_to_first_line());
    }

    #[test]
    fn word_moves_walk_over_words_of_letters_and_digits() {
        let mut b = Buffer::from_text("a/b-c123");
        b.set_cursor(0, 8); // the end; from_text always starts the cursor at (0, 0)
        for want in [4, 2, 0] {
            assert!(b.move_word_left());
            assert_eq!(b.cursor(), (0, want));
        }
        assert!(!b.move_word_left());
        for want in [1, 3, 8] {
            assert!(b.move_word_right());
            assert_eq!(b.cursor(), (0, want));
        }
        assert!(!b.move_word_right());
    }

    #[test]
    fn words_may_be_wide_characters() {
        let mut b = Buffer::from_text("日本語 abc");
        b.set_cursor(0, "日本語 abc".len()); // the end
        assert!(b.move_word_left());
        assert_eq!(b.cursor(), (0, "日本語 ".len()));
        assert!(b.move_word_left());
        assert_eq!(b.cursor(), (0, 0));
        assert!(b.move_word_right());
        assert_eq!(b.cursor(), (0, "日本語".len()));
    }

    #[test]
    fn word_moves_cross_line_boundaries_one_line_at_a_time() {
        let mut b = Buffer::from_text("one two\nthree");
        b.set_cursor(1, 5); // middle of "three"
        assert!(b.move_word_left());
        assert_eq!(b.cursor(), (1, 0)); // "three"'s own start, same line
        assert!(b.move_word_left());
        assert_eq!(b.cursor(), (0, 7)); // crossed up to the end of "one two"
        assert!(b.move_word_left());
        assert_eq!(b.cursor(), (0, 4)); // "two"
        assert!(b.move_word_left());
        assert_eq!(b.cursor(), (0, 0)); // "one"
        assert!(!b.move_word_left());
    }

    #[test]
    fn word_move_right_crosses_line_boundaries_one_line_at_a_time() {
        let mut b = Buffer::from_text("one\ntwo three");
        assert!(b.move_word_right());
        assert_eq!(b.cursor(), (0, 3)); // end of "one"
        assert!(b.move_word_right());
        assert_eq!(b.cursor(), (1, 0)); // crossed down to the start of the next line
        assert!(b.move_word_right());
        assert_eq!(b.cursor(), (1, 3)); // "two"
        assert!(b.move_word_right());
        assert_eq!(b.cursor(), (1, 9)); // "three"
        assert!(!b.move_word_right());
    }

    #[test]
    fn a_blank_line_is_one_stop_when_crossed() {
        let mut b = Buffer::from_text("a\n\nb");
        b.set_cursor(2, 1); // end of "b"
        assert!(b.move_word_left());
        assert_eq!(b.cursor(), (2, 0)); // "b" itself
        assert!(b.move_word_left());
        assert_eq!(b.cursor(), (1, 0)); // the blank line, one stop
        assert!(b.move_word_left());
        assert_eq!(b.cursor(), (0, 1)); // crossed to the end of "a"
        assert!(b.move_word_left());
        assert_eq!(b.cursor(), (0, 0)); // "a" itself
        assert!(!b.move_word_left());
    }

    #[test]
    fn set_cursor_clamps_line_and_snaps_column_to_a_boundary() {
        let mut b = Buffer::from_text("a日b\nx");
        b.set_cursor(99, 99); // past the last line, past its end
        assert_eq!(b.cursor(), (1, 1));
        b.set_cursor(0, 2); // inside "日"'s byte range: not a boundary
        assert_eq!(b.cursor(), (0, 1)); // snapped back to before it
    }

    #[test]
    fn every_mutation_sets_modified_and_mark_saved_clears_it() {
        let mut b = Buffer::new();
        b.insert_char('x');
        assert!(b.is_modified());
        b.mark_saved();
        assert!(!b.is_modified());
    }
}
