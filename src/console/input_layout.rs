//! Where a typed line lands on the screen: how many rows a prefix plus text needs, and where a given
//! cursor byte offset lands. The line discipline (`keyboard/line_discipline.rs`) uses it both to size
//! and place the input area, and to find the screen cell for the visible cursor (`cursor_position`).
//!
//! It replays exactly what `Console::write_char` does, by driving the same `Cursor` (`cells.rs`) over
//! an endless screen -- a glyph that ends in the last column leaves the wrap pending, a wide glyph that
//! does not fit moves to the next row whole -- so the layout and the drawing cannot disagree. Text is
//! taken to be printable: the line buffer never holds control characters (`line.rs`), and the
//! prompt is plain text.
//!
//! Pure `no_std`, with no dependency on the rest of the kernel, so it is tested on the host
//! (`just test-host`).

use super::cells::Cursor;
use super::font::cell_width;

/// Where the cursor ends up after drawing `prefix` and then `text` from the top-left cell of a screen
/// `cols` cells wide and endlessly tall.
fn end_cursor(prefix: &str, text: &str, cols: usize) -> Cursor {
    let mut cursor = Cursor::new();
    for c in prefix.chars().chain(text.chars()) {
        let width = cell_width(c);
        if width == 0 {
            continue; // draws nothing and takes no cell
        }
        cursor.start_glyph(width, cols, usize::MAX);
        cursor.end_glyph(width, cols);
    }
    cursor
}

/// How many rows `prefix` and `text` occupy: at least one (the prefix's row, even when both are empty).
/// A row filled exactly does not start the next -- that happens only when another glyph arrives.
pub fn rows_needed(prefix: &str, text: &str, cols: usize) -> usize {
    end_cursor(prefix, text, cols).row + 1
}

/// Whether the line may be as long as it is on a screen of `rows` rows: it may use at most `rows - 1`
/// of them, so it can never scroll off the top or push its own first row out of view.
pub fn fits_on_screen(prefix: &str, text: &str, cols: usize, rows: usize) -> bool {
    rows_needed(prefix, text, cols) < rows
}

/// Where the cursor sits on screen, as (row, column) relative to the line's first row, given a byte
/// offset `cursor` into `text` (always on a char boundary -- `line.rs`'s own invariant). Computed by
/// replaying only `prefix` plus the text *before* the cursor, so the character actually at the cursor
/// (if any) never affects where it's drawn -- the cell right after whatever's been laid out so far.
/// This is the same computation for `cursor == text.len()` (the end of the line) as for any other
/// offset, so it covers Home/End and a wrapped line with no special-casing at any call site .
///
/// A pending wrap resolves to the start of the next row *unless* `cursor` is at the very end of
/// `text` -- there is no real character actually drawn there to justify jumping ahead, and doing so
/// anyway can point past the last row the text actually occupies (`rows_needed`'s own count), which
/// is not always a valid screen row. Matches xterm's own deferred-wrap rule (`cells.rs`'s `Cursor`):
/// a glyph that exactly fills the last column leaves the *visible* cursor sitting on it, not already
/// wrapped to a row nothing has been drawn on yet -- the wrap happens only once another glyph
/// actually arrives. For any `cursor` short of the end, the character that would be at that offset
/// really is drawn one row down (the full line was laid out in one pass), so resolving there matches
/// what is actually on screen.
pub fn cursor_position(prefix: &str, text: &str, cursor: usize, cols: usize) -> (usize, usize) {
    let before = &text[..cursor];
    let c = end_cursor(prefix, before, cols);
    if c.wrap_pending && cursor < text.len() {
        (c.row + 1, 0)
    } else {
        (c.row, c.col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLS: usize = 10;

    /// Where a visible cursor at the end of `text` sits -- `cursor_position` at the end of the
    /// line, the shape every existing test below was written against before `cursor_position` grew
    /// a `cursor` parameter. Not "where the next glyph would go": at a full row, those differ (see
    /// `cursor_position`'s own doc comment).
    fn insertion_point(prefix: &str, text: &str, cols: usize) -> (usize, usize) {
        cursor_position(prefix, text, text.len(), cols)
    }

    #[test]
    fn an_empty_line_is_one_row_with_the_cursor_after_the_prefix() {
        assert_eq!(rows_needed("", "", COLS), 1);
        assert_eq!(rows_needed("> ", "", COLS), 1);
        assert_eq!(insertion_point("> ", "", COLS), (0, 2));
        assert_eq!(insertion_point("", "", COLS), (0, 0));
    }

    #[test]
    fn a_line_grows_a_row_only_when_a_glyph_no_longer_fits() {
        // "> " is two cells, so eight characters fill the row exactly.
        assert_eq!(rows_needed("> ", "1234567", COLS), 1);
        assert_eq!(rows_needed("> ", "12345678", COLS), 1);
        assert_eq!(rows_needed("> ", "123456789", COLS), 2);
        assert_eq!(rows_needed("> ", "12345678901234567890", COLS), 3);
    }

    #[test]
    fn the_insertion_point_after_a_full_row_is_the_start_of_the_next() {
        assert_eq!(insertion_point("> ", "1234567", COLS), (0, 9));
        // Exactly 10 cells (a full row): the cursor stays on the last column, deferred-wrap style
        // (`cursor_position`'s own doc comment) -- there's no real next character to jump to yet.
        assert_eq!(insertion_point("> ", "12345678", COLS), (0, 9));
        assert_eq!(insertion_point("> ", "123456789", COLS), (1, 1));
        assert_eq!(insertion_point("> ", "1234567890123456", COLS), (1, 8));
        // 20 cells: two full rows, same deferred-wrap reasoning -- stays on the second row's last column.
        assert_eq!(insertion_point("> ", "123456789012345678", COLS), (1, 9));
    }

    #[test]
    fn a_wide_glyph_that_does_not_fit_moves_to_the_next_row_whole() {
        // Two prefix cells plus seven letters leave one cell: the wide glyph starts the next row.
        assert_eq!(rows_needed("> ", "1234567日", COLS), 2);
        assert_eq!(insertion_point("> ", "1234567日", COLS), (1, 2));
        // With room for it, it stays.
        assert_eq!(rows_needed("> ", "123456日", COLS), 1);
        // Filled the row exactly: deferred-wrap, stays on the last column (same as the plain-text case above).
        assert_eq!(insertion_point("> ", "123456日", COLS), (0, 9));
        assert_eq!(insertion_point("> ", "12345日", COLS), (0, 9));
    }

    #[test]
    fn a_line_of_wide_glyphs_takes_two_cells_each() {
        // Five per row on ten columns.
        assert_eq!(rows_needed("", "日日日日日", COLS), 1);
        assert_eq!(rows_needed("", "日日日日日日", COLS), 2);
        assert_eq!(insertion_point("", "日日日日日日", COLS), (1, 2));
        // An odd number of columns leaves one cell blank at the end of each row.
        assert_eq!(rows_needed("", "日日日", 5), 2);
    }

    #[test]
    fn zero_width_characters_take_no_cell() {
        assert_eq!(insertion_point("> ", "a\u{200D}b", COLS), (0, 4));
        assert_eq!(rows_needed("", "\u{200D}\u{FE0F}", COLS), 1);
    }

    #[test]
    fn the_line_may_use_all_but_one_row_of_the_screen() {
        let rows = 4; // three rows of ten cells = 30 cells, prefix included
        let ok = "x".repeat(28);
        let full = "x".repeat(30);
        assert!(fits_on_screen("> ", &ok, COLS, rows)); // 30 cells: three rows, the last one exactly full
        assert!(!fits_on_screen("> ", &full, COLS, rows)); // 32 cells: a fourth row
        assert!(fits_on_screen("", "", COLS, 2));
        assert!(!fits_on_screen("", "", COLS, 1));
    }

    #[test]
    fn cursor_position_at_the_start_is_right_after_the_prefix() {
        assert_eq!(cursor_position("> ", "hello", 0, COLS), (0, 2));
        assert_eq!(cursor_position("", "hello", 0, COLS), (0, 0));
    }

    #[test]
    fn cursor_position_mid_line_ignores_what_comes_after_it() {
        // Same offset regardless of what follows the cursor -- only the text *before* it counts.
        assert_eq!(cursor_position("> ", "hello", 3, COLS), (0, 5));
        assert_eq!(cursor_position("> ", "hel", 3, COLS), (0, 5));
    }

    #[test]
    fn cursor_position_at_the_very_end_of_a_full_row_stays_on_it() {
        // Regression: a cursor at the end of text that exactly fills the last row of the *screen*
        // (not just of the line) once resolved to "the next row" past `rows_needed`'s own count --
        // an out-of-range row `console::put_char_at` then panicked on. Deferred-wrap keeps it in
        // bounds: with nothing actually typed past this point, the cursor stays on the row the text
        // occupies, at its last column, exactly like `Cursor::end_glyph`'s own `wrap_pending` state
        // (`cells.rs`) -- never past `rows_needed(prefix, text, cols) - 1`.
        let text = "1234567890"; // exactly one full row of COLS=10
        assert_eq!(cursor_position("", text, text.len(), COLS), (0, COLS - 1));
        assert_eq!(rows_needed("", text, COLS) - 1, 0); // the row `cursor_position` landed on
    }

    #[test]
    fn cursor_position_on_a_wrapped_line() {
        // "> " (2) + 8 chars fills row 0 exactly, so a cursor at byte 8 starts row 1.
        assert_eq!(
            cursor_position("> ", "12345678901234567890", 8, COLS),
            (1, 0)
        );
        assert_eq!(
            cursor_position("> ", "12345678901234567890", 9, COLS),
            (1, 1)
        );
        // The end of the line, on a line that wraps twice.
        assert_eq!(
            cursor_position("> ", "12345678901234567890", 20, COLS),
            (2, 2)
        );
    }

    #[test]
    fn cursor_position_right_after_a_wide_character() {
        // "日" occupies two cells; a cursor placed right after it (byte offset 3, its UTF-8 width)
        // lands in the third cell, not inside the glyph it just passed.
        assert_eq!(cursor_position("", "日", 3, COLS), (0, 2));
        assert_eq!(cursor_position("> ", "日日", 3, COLS), (0, 4));
    }
}
