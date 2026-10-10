//! The view's scrolling: which screen row of which line sits at the top, and which (line, row)
//! pairs fill the text area from there. Scrolls by whole logical lines -- the top is always the
//! first row of some line, moved just far enough to keep the cursor's own screen row visible, and
//! the bottom line may be cut off, drawn as far as it fits. The one exception is a line taller than
//! the text area itself: while the cursor is in it, the top is `(that line, some row within it)`
//! and scrolling moves row by row through it instead; leaving the line snaps the top back to a
//! whole-line start on the next call, since a normal line is never taller than the screen it's
//! measured against.
//!
//! [`scroll_to_cursor`] assumes the old top and the cursor are already close together, as they are
//! after ordinary movement -- each call it does is proportional to how far the top has to move, not
//! to the file's size. A jump that lands far from the old top (go to line, search) should call
//! [`center_on`] instead, which computes a fresh top directly from the target line, in at most
//! about half a screenful of lines' worth of work.
//!
//! Pure `no_std` + `alloc`, so it is tested on the host (`cargo test`).

use alloc::vec::Vec;

use crate::buffer::Buffer;
use crate::layout::{Row, cell_to_position, position_to_cell, row_count, wrap_line};

/// The view's top: line `line`, its own screen row `sub_row` (always 0 unless that line is taller
/// than the text area and the cursor is presently inside it).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Top {
    pub line: usize,
    pub sub_row: usize,
}

/// Recomputes `top` so `buffer`'s cursor is visible in a text area `height` rows tall, moving it
/// the least amount that does so (or not at all, if the cursor is visible already).
pub fn scroll_to_cursor(
    buffer: &Buffer,
    top: Top,
    height: usize,
    width: usize,
    tab_size: usize,
) -> Top {
    let height = height.max(1);
    let (cursor_line, cursor_byte) = buffer.cursor();
    let cursor_text = buffer.line(cursor_line);
    let (cursor_row, _) = position_to_cell(cursor_text, cursor_byte, width, tab_size);
    let cursor_line_rows = row_count(cursor_text, width, tab_size);

    if cursor_line_rows > height {
        // The cursor's own line doesn't fit the screen: scroll within it, by row.
        let sub_row = if top.line == cursor_line && cursor_row < top.sub_row {
            cursor_row // scrolled up within the line: reveal exactly up to the new row
        } else if top.line == cursor_line && cursor_row < top.sub_row + height {
            top.sub_row // already visible: no movement
        } else {
            // Scrolled down past the bottom, or just entered this line: show the last `height`
            // rows up to and including the cursor's own (also correct if cursor_row is small --
            // `saturating_sub` then leaves `sub_row` at 0, i.e. the line's own top).
            (cursor_row + 1).saturating_sub(height)
        };
        return Top {
            line: cursor_line,
            sub_row,
        };
    }

    // The normal case: the top is a whole line's start (never mid-line once the cursor's own line
    // fits the screen).
    if cursor_line < top.line {
        return Top {
            line: cursor_line,
            sub_row: 0,
        };
    }
    let mut rows_needed = cursor_row + 1; // from the top line's start through the cursor's own row
    for line in top.line..cursor_line {
        rows_needed += row_count(buffer.line(line), width, tab_size);
    }
    let mut top_line = top.line;
    while rows_needed > height && top_line < cursor_line {
        rows_needed -= row_count(buffer.line(top_line), width, tab_size);
        top_line += 1;
    }
    Top {
        line: top_line,
        sub_row: 0,
    }
}

/// A fresh top for a jump to `line` (go to line, search): walks back from it accumulating rows,
/// stopping at line 0 or once roughly half the screen's height is covered, so `line` lands more or
/// less in the middle rather than right at the top edge. Always a whole-line start, even if `line`
/// itself turns out to be taller than the screen -- `scroll_to_cursor`'s next call, once the cursor
/// actually lands somewhere in it, takes over the sub-row detail from there.
pub fn center_on(
    buffer: &Buffer,
    line: usize,
    height: usize,
    width: usize,
    tab_size: usize,
) -> Top {
    let target_rows_above = height / 2;
    let mut top_line = line;
    let mut rows_above = 0;
    while top_line > 0 {
        let candidate = top_line - 1;
        let candidate_rows = row_count(buffer.line(candidate), width, tab_size);
        if rows_above + candidate_rows > target_rows_above {
            break;
        }
        rows_above += candidate_rows;
        top_line = candidate;
    }
    Top {
        line: top_line,
        sub_row: 0,
    }
}

/// The `(line, Row)` pairs that fill the text area from `top`, one per screen row, at most `height`
/// of them (fewer once the buffer runs out of lines).
pub fn visible_rows(
    buffer: &Buffer,
    top: Top,
    height: usize,
    width: usize,
    tab_size: usize,
) -> Vec<(usize, Row)> {
    let mut out = Vec::with_capacity(height);
    let mut skip = top.sub_row;
    for line in top.line..buffer.line_count() {
        if out.len() >= height {
            break;
        }
        for row in wrap_line(buffer.line(line), width, tab_size)
            .into_iter()
            .skip(skip)
        {
            if out.len() >= height {
                break;
            }
            out.push((line, row));
        }
        skip = 0;
    }
    out
}

/// Moves `buffer`'s cursor one screen row up (`up`) or down, crossing into the previous or next
/// logical line where the current one's own rows run out, and landing at (or as close as possible
/// to) `preferred_col` -- the *display* column Up/Down keep returning to across a run of vertical
/// moves, even through lines too short to reach it, until something else moves the cursor
/// horizontally and the caller resets it to `None` (this function only ever reads and updates it;
/// resetting on a horizontal move is the caller's job, since only it knows which key that was).
/// Returns whether anything moved (`false` at the very first or very last row of the buffer).
pub fn move_vertical(
    buffer: &mut Buffer,
    up: bool,
    preferred_col: &mut Option<usize>,
    width: usize,
    tab_size: usize,
) -> bool {
    let (line, byte) = buffer.cursor();
    let (row, col) = position_to_cell(buffer.line(line), byte, width, tab_size);
    let want = preferred_col.unwrap_or(col);

    let (target_line, target_row) = if up {
        if row > 0 {
            (line, row - 1)
        } else if line > 0 {
            (
                line - 1,
                row_count(buffer.line(line - 1), width, tab_size) - 1,
            )
        } else {
            return false; // already the very first row of the buffer
        }
    } else if row + 1 < row_count(buffer.line(line), width, tab_size) {
        (line, row + 1)
    } else if line + 1 < buffer.line_count() {
        (line + 1, 0)
    } else {
        return false; // already the very last row of the buffer
    };

    let new_byte = cell_to_position(buffer.line(target_line), target_row, want, width, tab_size);
    buffer.set_cursor(target_line, new_byte);
    *preferred_col = Some(want);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer_of(lines: &[&str]) -> Buffer {
        Buffer::from_text(&lines.join("\n"))
    }

    #[test]
    fn a_cursor_already_visible_does_not_move_the_top() {
        let mut b = buffer_of(&["a", "b", "c", "d"]);
        b.set_cursor(2, 0);
        let top = Top {
            line: 0,
            sub_row: 0,
        };
        assert_eq!(scroll_to_cursor(&b, top, 3, 80, 4), top);
    }

    #[test]
    fn scrolling_down_advances_by_whole_lines() {
        let mut b = buffer_of(&["a", "b", "c", "d", "e"]);
        b.set_cursor(4, 0); // off the bottom of a 3-row screen starting at line 0
        assert_eq!(
            scroll_to_cursor(
                &b,
                Top {
                    line: 0,
                    sub_row: 0
                },
                3,
                80,
                4
            ),
            Top {
                line: 2,
                sub_row: 0
            } // just enough that lines 2,3,4 (the cursor's) are shown
        );
    }

    #[test]
    fn scrolling_up_past_the_top_reanchors_there() {
        let mut b = buffer_of(&["a", "b", "c", "d", "e"]);
        b.set_cursor(1, 0);
        assert_eq!(
            scroll_to_cursor(
                &b,
                Top {
                    line: 3,
                    sub_row: 0
                },
                3,
                80,
                4
            ),
            Top {
                line: 1,
                sub_row: 0
            }
        );
    }

    #[test]
    fn the_bottom_line_may_be_only_partly_shown() {
        // "abcde" wraps to 2 rows at width 4 ("abcd", then "e"). Moving down to it needs only line 1
        // to move into view too (lines 1, 2 and "abcde"'s first row already make 3), so the top ends
        // up at line 1, and "abcde"'s second row is left off the bottom of the screen.
        let mut b = buffer_of(&["a", "b", "c", "abcde", "fghij"]);
        b.set_cursor(3, 0);
        let top = scroll_to_cursor(
            &b,
            Top {
                line: 0,
                sub_row: 0,
            },
            3,
            4,
            4,
        );
        assert_eq!(
            top,
            Top {
                line: 1,
                sub_row: 0
            }
        );
        let rows = visible_rows(&b, top, 3, 4, 4);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows.iter().map(|(l, _)| *l).collect::<Vec<_>>(), [1, 2, 3]); // "b", "c", "abcde"'s first row
    }

    #[test]
    fn a_line_taller_than_the_screen_scrolls_by_row_while_the_cursor_is_in_it() {
        // One line, width 1 (so each character is its own row): "0123456789", 10 rows, screen 4 tall.
        let mut b = buffer_of(&["0123456789"]);
        b.set_cursor(0, 0);
        let top = scroll_to_cursor(&b, Top::default(), 4, 1, 4);
        assert_eq!(
            top,
            Top {
                line: 0,
                sub_row: 0
            }
        );

        // Move the cursor to row 7 (character '7'): scrolled down, bottom-anchored.
        b.set_cursor(0, 7);
        let top = scroll_to_cursor(&b, top, 4, 1, 4);
        assert_eq!(
            top,
            Top {
                line: 0,
                sub_row: 4
            }
        ); // rows 4..8 shown, cursor's row 7 the last

        // Move back up to row 2: above the current window (4..8), scrolled up to reveal it exactly.
        b.set_cursor(0, 2);
        let top = scroll_to_cursor(&b, top, 4, 1, 4);
        assert_eq!(
            top,
            Top {
                line: 0,
                sub_row: 2
            }
        );

        // Already visible (row 3, within 2..6): no movement.
        b.set_cursor(0, 3);
        assert_eq!(scroll_to_cursor(&b, top, 4, 1, 4), top);
    }

    #[test]
    fn leaving_a_tall_line_goes_back_to_whole_line_scrolling() {
        // Ten one-character lines, then a normal-width one: at width 80 none of them is tall, but
        // `top` still carries a stale non-zero sub_row from when the cursor was somewhere that was.
        // Once the cursor is on the last line, the result must have sub_row 0 (never a leftover),
        // with the top moved back just far enough by whole lines.
        let mut b = buffer_of(&["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "next"]);
        let stale_top = Top {
            line: 0,
            sub_row: 6,
        };
        b.set_cursor(10, 0);
        assert_eq!(
            scroll_to_cursor(&b, stale_top, 3, 80, 4),
            Top {
                line: 8,
                sub_row: 0
            }
        );
    }

    #[test]
    fn center_on_puts_the_target_roughly_in_the_middle() {
        let b = buffer_of(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]);
        // height 4: half is 2, so it walks back up to 2 rows (2 lines, one row each).
        assert_eq!(
            center_on(&b, 5, 4, 80, 4),
            Top {
                line: 3,
                sub_row: 0
            }
        );
    }

    #[test]
    fn center_on_stops_at_the_start_of_the_buffer() {
        let b = buffer_of(&["a", "b", "c"]);
        assert_eq!(
            center_on(&b, 1, 20, 80, 4),
            Top {
                line: 0,
                sub_row: 0
            }
        );
    }

    #[test]
    fn visible_rows_stops_at_the_end_of_the_buffer() {
        let b = buffer_of(&["a", "b"]);
        let rows = visible_rows(&b, Top::default(), 5, 80, 4);
        assert_eq!(rows.len(), 2); // not 5: nothing past the last line
    }

    #[test]
    fn visible_rows_from_a_sub_row_skips_that_lines_earlier_rows() {
        // At width 1 every character of "0123456789" is its own row; skipping 8 leaves rows 8 and 9
        // ('8' and '9') of that line -- exactly what a 2-row budget can hold, so line "next" is
        // never reached.
        let b = buffer_of(&["0123456789", "next"]);
        let rows = visible_rows(
            &b,
            Top {
                line: 0,
                sub_row: 8,
            },
            2,
            1,
            4,
        );
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].0, rows[0].1.start), (0, 8));
        assert_eq!((rows[1].0, rows[1].1.start), (0, 9));
    }

    #[test]
    fn move_vertical_within_one_line_seeks_the_same_display_column() {
        let mut b = buffer_of(&["1234567890", "12345"]);
        b.set_cursor(0, 4); // width 80, tab 4: column 4
        let mut preferred = None;
        assert!(move_vertical(&mut b, false, &mut preferred, 80, 4)); // Down, into the shorter line
        assert_eq!(b.cursor(), (1, 4));
        assert_eq!(preferred, Some(4));
        assert!(move_vertical(&mut b, true, &mut preferred, 80, 4)); // Up, back
        assert_eq!(b.cursor(), (0, 4));
    }

    #[test]
    fn move_vertical_keeps_the_preferred_column_through_a_shorter_line() {
        // Moving down through a line too short to reach the preferred column, then down again into
        // one long enough, lands back at the original column -- not wherever the short line's own
        // end happened to be.
        let mut b = buffer_of(&["1234567890", "12", "1234567890"]);
        b.set_cursor(0, 7);
        let mut preferred = None;
        assert!(move_vertical(&mut b, false, &mut preferred, 80, 4));
        assert_eq!(b.cursor(), (1, 2)); // "12" has no column 7: lands at its own end
        assert_eq!(preferred, Some(7)); // the *want*, not where it actually landed
        assert!(move_vertical(&mut b, false, &mut preferred, 80, 4));
        assert_eq!(b.cursor(), (2, 7)); // column 7 exists again
    }

    #[test]
    fn move_vertical_crosses_a_wrapped_lines_own_rows_before_the_next_line() {
        // "12345678901234567890" at width 5 wraps to four rows of the one line; moving down from its
        // second row goes to its third row, still the same line, before ever reaching "next".
        let mut b = buffer_of(&["12345678901234567890", "next"]);
        b.set_cursor(0, 7); // row 1 (chars 5-9), column 2
        let mut preferred = None;
        assert!(move_vertical(&mut b, false, &mut preferred, 5, 4));
        assert_eq!(b.cursor(), (0, 12)); // row 2, same column
    }

    #[test]
    fn move_vertical_does_nothing_at_the_very_first_or_last_row() {
        let mut b = buffer_of(&["a", "b"]);
        let mut preferred = None;
        assert!(!move_vertical(&mut b, true, &mut preferred, 80, 4));
        b.set_cursor(1, 1);
        assert!(!move_vertical(&mut b, false, &mut preferred, 80, 4));
    }
}
