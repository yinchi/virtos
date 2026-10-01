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
}
