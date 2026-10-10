//! The mark and the cut buffer -- Tier 2's plain line cut/paste (`^K`/`^U` with no mark: whole
//! lines, one `String` per line accumulated) and Tier 2b's region cut/paste (a mark set: one
//! `String` with embedded `\n`s, character-granular at both ends). The two stay genuinely distinct
//! operations, not one written in terms of the other, because nano's own behaviour is: a linewise
//! paste inserts new lines *above* the current one; a characterwise paste splices into it at the
//! exact cursor position. Which one a keypress means, and how the cut buffer's own accumulation
//! (`^K` appending to the last one, mixing linewise and characterwise never being asked to work) is
//! decided by whoever dispatches keys -- this module only does the buffer edits.
//!
//! Pure `no_std` + `alloc`, so it is tested on the host (`cargo test`).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::buffer::Buffer;

/// A `(line, byte column)` position -- the mark, or either end of a region, in the same terms
/// [`Buffer::cursor`] uses.
pub type Position = (usize, usize);

/// `mark` and `cursor`, as `(start, end)` -- whichever comes first in the text. Positions compare
/// the way [`Buffer::cursor`]'s tuples naturally do: by line, then by column within it.
pub fn ordered(mark: Position, cursor: Position) -> (Position, Position) {
    if mark <= cursor {
        (mark, cursor)
    } else {
        (cursor, mark)
    }
}

/// The text of the region from `start` to `end` (`start <= end`; see [`ordered`]), joined with `\n`
/// where it spans lines -- with no leading or trailing newline of its own, matching what
/// `region_text` combined with [`cut_region`]/[`paste_region`] would reproduce.
pub fn region_text(buffer: &Buffer, start: Position, end: Position) -> String {
    if start.0 == end.0 {
        return buffer.line(start.0)[start.1..end.1].to_string();
    }
    let mut text = buffer.line(start.0)[start.1..].to_string();
    for line in start.0 + 1..end.0 {
        text.push('\n');
        text.push_str(buffer.line(line));
    }
    text.push('\n');
    text.push_str(&buffer.line(end.0)[..end.1]);
    text
}

/// Removes the region from `start` to `end`, joining what's left of `start`'s line to what's left
/// of `end`'s, and leaves the cursor at `start`. Returns the region's text (as [`region_text`]
/// would have, just before it was removed).
pub fn cut_region(buffer: &mut Buffer, start: Position, end: Position) -> String {
    let text = region_text(buffer, start, end);
    if start.0 == end.0 {
        let mut line = buffer.line(start.0).to_string();
        line.replace_range(start.1..end.1, "");
        buffer.set_line(start.0, line);
    } else {
        let mut merged = buffer.line(start.0)[..start.1].to_string();
        merged.push_str(&buffer.line(end.0)[end.1..]);
        buffer.set_line(start.0, merged);
        for _ in start.0..end.0 {
            buffer.remove_line(start.0 + 1);
        }
    }
    buffer.set_cursor(start.0, start.1);
    text
}

/// Splices `text` into the current line at the cursor -- an embedded `\n` starts a new line there,
/// the same as typing it would character by character, just in one step. The cursor lands right
/// after the inserted text (before whatever followed the original cursor, now on the last inserted
/// line). Pasting nothing does nothing.
pub fn paste_region(buffer: &mut Buffer, text: &str) {
    if text.is_empty() {
        return;
    }
    let (line, col) = buffer.cursor();
    let before = buffer.line(line)[..col].to_string();
    let after = buffer.line(line)[col..].to_string();
    let mut parts = text.split('\n');
    let first = parts.next().expect("split always yields at least one part");
    let rest: Vec<&str> = parts.collect();

    if rest.is_empty() {
        let mut merged = before;
        merged.push_str(text);
        let cursor_col = merged.len();
        merged.push_str(&after);
        buffer.set_line(line, merged);
        buffer.set_cursor(line, cursor_col);
        return;
    }

    let mut first_line = before;
    first_line.push_str(first);
    buffer.set_line(line, first_line);

    let last_index = rest.len() - 1;
    for (i, part) in rest.into_iter().enumerate() {
        if i < last_index {
            buffer.insert_line(line + 1 + i, part.to_string());
        } else {
            let mut last_line = part.to_string();
            let cursor_col = last_line.len();
            last_line.push_str(&after);
            buffer.insert_line(line + 1 + i, last_line);
            buffer.set_cursor(line + 1 + i, cursor_col);
        }
    }
}

/// Removes the current line entirely and returns its text (no trailing `\n`) -- `^K` with no mark
/// set. Consecutive presses are the caller's job to notice and accumulate (append each cut line, in
/// order, joined by `\n`) into one growing cut buffer, the way nano's do; this function only ever
/// removes one line at a time.
pub fn cut_line(buffer: &mut Buffer) -> String {
    let (line, _) = buffer.cursor();
    buffer.remove_line(line)
}

/// Inserts `text` (one line, or several joined by `\n` from consecutive cuts) as whole new lines
/// *above* the cursor's current line, which -- with everything below it -- shifts down to make
/// room. The cursor lands at the start of the first inserted line, so a `cut_line` immediately
/// followed by `paste_line` reproduces exactly what was there before.
pub fn paste_line(buffer: &mut Buffer, text: &str) {
    let (line, _) = buffer.cursor();
    for (i, part) in text.split('\n').enumerate() {
        buffer.insert_line(line + i, part.to_string());
    }
    buffer.set_cursor(line, 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_puts_the_earlier_position_first() {
        assert_eq!(ordered((0, 5), (2, 1)), ((0, 5), (2, 1)));
        assert_eq!(ordered((2, 1), (0, 5)), ((0, 5), (2, 1)));
        assert_eq!(ordered((1, 2), (1, 5)), ((1, 2), (1, 5)));
        assert_eq!(ordered((1, 5), (1, 2)), ((1, 2), (1, 5)));
        assert_eq!(ordered((1, 2), (1, 2)), ((1, 2), (1, 2))); // an empty region
    }

    #[test]
    fn region_text_within_one_line() {
        let b = Buffer::from_text("hello world");
        assert_eq!(region_text(&b, (0, 6), (0, 11)), "world");
    }

    #[test]
    fn region_text_across_lines_joins_with_newlines_and_no_others() {
        let b = Buffer::from_text("one\ntwo\nthree\nfour");
        assert_eq!(region_text(&b, (0, 1), (3, 2)), "ne\ntwo\nthree\nfo");
        // Adjacent lines: no line survives whole in the middle, just the join.
        assert_eq!(region_text(&b, (0, 2), (1, 1)), "e\nt");
    }

    #[test]
    fn cut_region_within_one_line_joins_what_is_left() {
        let mut b = Buffer::from_text("hello world");
        assert_eq!(cut_region(&mut b, (0, 5), (0, 11)), " world");
        assert_eq!(b.line(0), "hello");
        assert_eq!(b.cursor(), (0, 5));
    }

    #[test]
    fn cut_region_across_lines_merges_the_two_ends_and_removes_what_is_between() {
        let mut b = Buffer::from_text("one\ntwo\nthree\nfour");
        assert_eq!(cut_region(&mut b, (0, 1), (3, 2)), "ne\ntwo\nthree\nfo");
        assert_eq!(b.line_count(), 1);
        assert_eq!(b.line(0), "our"); // "o" (before col 1 of "one") + "ur" (after col 2 of "four")
        assert_eq!(b.cursor(), (0, 1));
    }

    #[test]
    fn cut_region_then_paste_region_is_the_identity() {
        for (text, start, end) in [
            ("hello world", (0, 5), (0, 11)),
            ("one\ntwo\nthree\nfour", (0, 1), (3, 2)),
            ("one\ntwo\nthree\nfour", (1, 0), (2, 5)),
        ] {
            let mut b = Buffer::from_text(text);
            let cut = cut_region(&mut b, start, end);
            paste_region(&mut b, &cut);
            assert_eq!(b.to_text(), Buffer::from_text(text).to_text(), "{text:?}");
            assert_eq!(b.cursor(), end, "{text:?}"); // pasting the same text back reaches the same end
        }
    }

    #[test]
    fn paste_region_within_one_line_lands_the_cursor_after_the_pasted_text() {
        let mut b = Buffer::from_text("ac");
        b.set_cursor(0, 1);
        paste_region(&mut b, "XYZ");
        assert_eq!(b.line(0), "aXYZc");
        assert_eq!(b.cursor(), (0, 4));
    }

    #[test]
    fn paste_region_with_embedded_newlines_splits_the_current_line() {
        let mut b = Buffer::from_text("ac");
        b.set_cursor(0, 1);
        paste_region(&mut b, "1\n2\n3");
        assert_eq!(b.line_count(), 3);
        assert_eq!((b.line(0), b.line(1), b.line(2)), ("a1", "2", "3c"));
        assert_eq!(b.cursor(), (2, 1)); // right after "3", before "c"
    }

    #[test]
    fn pasting_empty_text_does_nothing() {
        let mut b = Buffer::from_text("abc");
        b.set_cursor(0, 1);
        paste_region(&mut b, "");
        assert_eq!(b.to_text(), "abc\n");
        assert_eq!(b.cursor(), (0, 1));
    }

    #[test]
    fn cut_line_removes_the_whole_current_line() {
        let mut b = Buffer::from_text("a\nb\nc");
        b.set_cursor(1, 1); // mid-line: the whole line is cut regardless of the column
        assert_eq!(cut_line(&mut b), "b");
        assert_eq!(b.to_text(), "a\nc\n");
        assert_eq!(b.cursor(), (1, 0));
    }

    #[test]
    fn cut_line_then_paste_line_is_the_identity() {
        let mut b = Buffer::from_text("a\nb\nc");
        b.set_cursor(1, 1);
        let cut = cut_line(&mut b);
        paste_line(&mut b, &cut);
        assert_eq!(b.to_text(), "a\nb\nc\n");
        assert_eq!(b.cursor(), (1, 0));
    }

    #[test]
    fn paste_line_inserts_several_accumulated_lines_above_the_current_one() {
        let mut b = Buffer::from_text("x");
        paste_line(&mut b, "one\ntwo\nthree");
        assert_eq!(b.line_count(), 4);
        assert_eq!(
            (b.line(0), b.line(1), b.line(2), b.line(3)),
            ("one", "two", "three", "x")
        );
        assert_eq!(b.cursor(), (0, 0));
    }

    #[test]
    fn paste_line_never_splices_into_the_current_lines_text() {
        // Unlike paste_region, the column the cursor was at doesn't matter -- a linewise paste
        // never touches what's already on the current line.
        let mut b = Buffer::from_text("hello");
        b.set_cursor(0, 3);
        paste_line(&mut b, "new");
        assert_eq!((b.line(0), b.line(1)), ("new", "hello"));
    }
}
