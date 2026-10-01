//! Where an input line (a prompt prefix plus the text typed after it) lands on the screen --
//! pure arithmetic, so the line discipline can tell how many rows to clear before redrawing it
//! and when a line has grown as long as the screen allows. ASCII only, like the console itself:
//! one character is one cell.
//!
//! Follows the console's own xterm wrap-pending rule (`Console::putc`): a line that exactly fills
//! its last row stays on that row until the next character arrives, so it does not claim another.

/// How many rows a line of `prefix_len + text_len` cells occupies on a `cols`-wide console -- at
/// least one, even when empty (the cursor still sits on a row).
pub fn rows_needed(prefix_len: usize, text_len: usize, cols: usize) -> usize {
    (prefix_len + text_len).div_ceil(cols).max(1)
}

/// Whether the line fits on a `rows`-tall console: it may use at most `rows - 1` rows, leaving
/// the last for the cursor to move onto once the line is finished.
pub fn fits_on_screen(prefix_len: usize, text_len: usize, cols: usize, rows: usize) -> bool {
    rows_needed(prefix_len, text_len, cols) < rows
}

/// The `(row, col)`, relative to the line's first row, of the cell the visible cursor occupies
/// when it sits at byte offset `cursor` of a `text_len`-byte text (ASCII, so a byte is a cell).
///
/// Follows the same deferred-wrap rule as drawing: a cursor at the very end of text that exactly
/// fills a row stays on that row's last column rather than jumping to a row nothing has been drawn
/// on yet (which would be past `rows_needed`'s own count -- and off the screen entirely if that is
/// the screen's last row). For any `cursor` short of the end, the character at that offset really
/// is drawn one row down, so it resolves there.
pub fn cursor_position(
    prefix_len: usize,
    text_len: usize,
    cursor: usize,
    cols: usize,
) -> (usize, usize) {
    let cells = prefix_len + cursor;
    if cells > 0 && cells.is_multiple_of(cols) {
        if cursor < text_len {
            (cells / cols, 0)
        } else {
            (cells / cols - 1, cols - 1)
        }
    } else {
        (cells / cols, cells % cols)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_line_still_occupies_a_row() {
        assert_eq!(rows_needed(0, 0, 80), 1);
    }

    #[test]
    fn a_short_line_is_one_row() {
        assert_eq!(rows_needed(10, 5, 80), 1);
    }

    #[test]
    fn a_line_exactly_filling_a_row_does_not_claim_another() {
        assert_eq!(rows_needed(10, 70, 80), 1);
    }

    #[test]
    fn one_cell_past_a_full_row_wraps() {
        assert_eq!(rows_needed(10, 71, 80), 2);
        assert_eq!(rows_needed(0, 161, 80), 3);
    }

    #[test]
    fn the_prefix_counts_toward_the_row() {
        assert_eq!(rows_needed(79, 2, 80), 2);
    }

    #[test]
    fn a_line_may_use_all_but_the_last_row() {
        // 30 rows: 29 are allowed, 30 are not.
        assert!(fits_on_screen(0, 29 * 80, 80, 30));
        assert!(!fits_on_screen(0, 29 * 80 + 1, 80, 30));
    }

    #[test]
    fn the_cursor_at_the_start_is_right_after_the_prefix() {
        assert_eq!(cursor_position(2, 5, 0, 10), (0, 2));
        assert_eq!(cursor_position(0, 5, 0, 10), (0, 0));
    }

    #[test]
    fn the_cursor_mid_line_depends_only_on_what_precedes_it() {
        assert_eq!(cursor_position(2, 5, 3, 10), (0, 5));
        assert_eq!(cursor_position(2, 3, 3, 10), (0, 5));
    }

    #[test]
    fn the_cursor_at_the_very_end_of_a_full_row_stays_on_it() {
        // Regression: this once resolved to "the next row", past `rows_needed`'s own count -- an
        // out-of-range row for `put_char_at` when the line ended on the screen's last row.
        assert_eq!(cursor_position(0, 10, 10, 10), (0, 9));
        assert_eq!(rows_needed(0, 10, 10) - 1, 0);
        assert_eq!(cursor_position(2, 18, 18, 10), (1, 9));
    }

    #[test]
    fn the_cursor_just_before_the_end_of_a_full_row_wraps_to_the_next_row() {
        // "> " (2) + 8 characters fill row 0 exactly, so byte 8 of a longer line starts row 1.
        assert_eq!(cursor_position(2, 20, 8, 10), (1, 0));
        assert_eq!(cursor_position(2, 20, 9, 10), (1, 1));
        assert_eq!(cursor_position(2, 20, 20, 10), (2, 2));
    }
}
