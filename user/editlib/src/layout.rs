//! Soft-wrap: how one logical line becomes one or more screen rows, and the map between a byte
//! offset into the line and a `(row, column)` on screen, in both directions.
//!
//! A line breaks at the last column that fits (nano's and vim's default, not at blanks); a
//! character that doesn't fit what's left of a row -- a wide (two-column) glyph, or a tab that
//! would cross the edge -- moves whole to the next row, the same rule for both, rather than
//! splitting either. A tab's width depends on the column it starts at (`tab_size - col %
//! tab_size`), so it is re-measured after a break, at column 0 of the new row.
//!
//! A row boundary belongs to the row that *starts* there: the position right after the last
//! character of a full row is the same position as the start of the next one, and
//! [`position_to_cell`] always reports the latter -- **except** at the very end of the line's own
//! text, which has no next row to defer to. There, if the last row is exactly full, [`wrap_line`]
//! appends one further, empty row, so that position still has a `(row, column)` of its own (column
//! 0 of it) instead of colliding with the last real row's own end column (one past its last valid
//! column). An empty line is one empty row, the same way.
//!
//! Pure `no_std` + `alloc`, so it is tested on the host (`cargo test`).

use alloc::vec::Vec;

use crate::width::cell_width;

/// One screen row of a wrapped line: the byte range of `text` drawn on it (`text[start..end]`,
/// `start`/`end` both on character boundaries). Never wider than the line's `width`, in display
/// columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub start: usize,
    pub end: usize,
}

/// `c`'s width starting at display column `col` -- every character's own [`cell_width`], except a
/// tab, which reaches the next multiple of `tab_size` (so it is always between 1 and `tab_size`
/// columns wide, depending only on where it starts). `pub` so a renderer building the actual
/// [`abi::ioctl::Cell`]s of a row (walking it the same way, but emitting characters instead of just
/// measuring them) uses this exact rule rather than a second copy of it.
pub fn width_at(c: char, col: usize, tab_size: usize) -> usize {
    if c == '\t' {
        tab_size - col % tab_size
    } else {
        cell_width(c)
    }
}

/// Splits `text` into the screen rows it draws as, for a row `width` columns wide (clamped to at
/// least 1: a real console is never narrower, but a degenerate `width` should still terminate
/// rather than loop). Never empty: at least one row always exists, covering the whole line (see
/// this module's doc comment for the trailing empty row's rule).
pub fn wrap_line(text: &str, width: usize, tab_size: usize) -> Vec<Row> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut row_start = 0;
    let mut col = 0;
    for (byte, c) in text.char_indices() {
        let w = width_at(c, col, tab_size).min(width);
        if col + w > width {
            rows.push(Row {
                start: row_start,
                end: byte,
            });
            row_start = byte;
            col = width_at(c, 0, tab_size).min(width); // re-measured at the new row's column 0
        } else {
            col += w;
        }
    }
    rows.push(Row {
        start: row_start,
        end: text.len(),
    });
    if col == width {
        // The last row is exactly full: give the position right after the text its own row rather
        // than colliding with this row's own one-past-the-last-column.
        rows.push(Row {
            start: text.len(),
            end: text.len(),
        });
    }
    rows
}

/// How many screen rows `text` draws as -- `wrap_line(...).len()`, for `scroll.rs`'s "is this line
/// taller than the screen" check without building the whole `Vec` where only the count is needed.
pub fn row_count(text: &str, width: usize, tab_size: usize) -> usize {
    // A tab's width depends only on the column within its own row (never on anything before the
    // current row), so counting rows needs the same walk as `wrap_line`, just without collecting.
    let width = width.max(1);
    let mut rows = 1;
    let mut col = 0;
    for c in text.chars() {
        let w = width_at(c, col, tab_size).min(width);
        if col + w > width {
            rows += 1;
            col = width_at(c, 0, tab_size).min(width);
        } else {
            col += w;
        }
    }
    if col == width {
        rows += 1;
    }
    rows
}

/// How many columns a Tab typed at byte offset `byte_offset` of `text` would take: the distance from
/// where the cursor is drawn to the next multiple of `tab_size` (always `1..=tab_size`). What a
/// "tabs to spaces" editor inserts, so the result lines up exactly as a real tab there would.
pub fn tab_fill(text: &str, byte_offset: usize, width: usize, tab_size: usize) -> usize {
    let (_, col) = position_to_cell(text, byte_offset, width, tab_size);
    width_at('\t', col, tab_size)
}

/// Where byte offset `byte_offset` of `text` (on a character boundary; `text.len()` itself, one
/// past the end, is valid too) falls on screen: `(row, column)`, `row` an index into
/// [`wrap_line`]'s result.
pub fn position_to_cell(
    text: &str,
    byte_offset: usize,
    width: usize,
    tab_size: usize,
) -> (usize, usize) {
    let rows = wrap_line(text, width, tab_size);
    let last = rows.len() - 1;
    for (i, row) in rows.iter().enumerate() {
        if row.start <= byte_offset && (byte_offset < row.end || i == last) {
            let mut col = 0;
            for c in text[row.start..byte_offset].chars() {
                col += width_at(c, col, tab_size);
            }
            return (i, col);
        }
    }
    unreachable!("wrap_line's rows cover every offset from 0 to text.len() inclusive")
}

/// The inverse-ish of [`position_to_cell`]: the byte offset on screen row `row` at or immediately
/// before display column `col` -- landing before a character that would straddle or start past
/// `col` (a wide glyph whose second column is asked for lands before the whole glyph, same as a
/// real terminal), or at the row's own end if `col` reaches past everything drawn on it. `row` past
/// the last one clamps to the last row -- `layout`'s callers (`scroll.rs`'s Up/Down) ask for a
/// fixed row count and let this decide what is actually there, rather than checking first.
pub fn cell_to_position(
    text: &str,
    row: usize,
    col: usize,
    width: usize,
    tab_size: usize,
) -> usize {
    let rows = wrap_line(text, width, tab_size);
    let row = &rows[row.min(rows.len() - 1)];
    let mut acc = 0;
    for (i, c) in text[row.start..row.end].char_indices() {
        let w = width_at(c, acc, tab_size);
        if acc + w > col {
            return row.start + i;
        }
        acc += w;
    }
    row.end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges(text: &str, width: usize, tab_size: usize) -> Vec<(usize, usize)> {
        wrap_line(text, width, tab_size)
            .into_iter()
            .map(|r| (r.start, r.end))
            .collect()
    }

    #[test]
    fn a_line_shorter_than_the_width_is_one_row() {
        assert_eq!(ranges("hello", 10, 8), [(0, 5)]);
    }

    #[test]
    fn an_empty_line_is_one_empty_row() {
        assert_eq!(ranges("", 10, 8), [(0, 0)]);
    }

    #[test]
    fn a_line_exactly_the_width_gets_a_trailing_empty_row() {
        assert_eq!(ranges("12345", 5, 8), [(0, 5), (5, 5)]);
    }

    #[test]
    fn a_longer_line_breaks_at_the_edge() {
        // "1234567890", width 4: "1234", "5678", "90".
        assert_eq!(ranges("1234567890", 4, 8), [(0, 4), (4, 8), (8, 10)]);
    }

    #[test]
    fn a_line_that_wraps_and_also_exactly_fills_its_last_row() {
        // "12345678", width 4: "1234", "5678", then the trailing empty row (the last real row is
        // exactly full too).
        assert_eq!(ranges("12345678", 4, 8), [(0, 4), (4, 8), (8, 8)]);
    }

    #[test]
    fn a_wide_glyph_that_would_straddle_the_edge_moves_whole_to_the_next_row() {
        // width 3: "ab" (2 cols) + "日" (2 cols) would be 4 -- "日" moves down instead of splitting.
        assert_eq!(ranges("ab日", 3, 8), [(0, 2), (2, 2 + '日'.len_utf8())]);
        // At the very start of a row, it fits a wide-enough row -- and, at width 2, fills it
        // exactly, so (like any exactly-full row) there is a trailing empty one too.
        let end = '日'.len_utf8();
        assert_eq!(ranges("日", 2, 8), [(0, end), (end, end)]);
    }

    #[test]
    fn a_wide_glyph_is_never_split_even_when_it_is_the_whole_row() {
        // width 1 (narrower than any wide glyph can ever fit): still whole, one glyph per row,
        // never half a glyph on one row and half on the next. Clamped to the row's own width, each
        // glyph exactly fills it, so the last row gets its trailing empty one too.
        assert_eq!(ranges("日本", 1, 8), [(0, 3), (3, 6), (6, 6)]);
    }

    #[test]
    fn a_tab_that_would_cross_the_edge_moves_whole_to_the_next_row() {
        // tab_size 4, width 5: 'a' (col 0->1), the first tab from col 1 reaches col 4 (fits, 1..4 <=
        // 5), 'b' (col 4->5, fits exactly), the second tab from col 5 would need to reach col 8 (3
        // more, straddling width 5) -- it moves to the next row instead, reaching col 4 there (a
        // full stop, starting fresh at column 0), then 'c' fills the row exactly (col 4->5), so
        // there is a trailing empty row too.
        assert_eq!(ranges("a\tb\tc", 5, 4), [(0, 3), (3, 5), (5, 5)]);
    }

    #[test]
    fn a_tab_that_fits_exactly_does_not_move() {
        // tab_size 4, width 4: one tab from col 0 reaches col 4 exactly.
        assert_eq!(ranges("\t", 4, 4), [(0, 1), (1, 1)]); // exactly full: trailing empty row too
    }

    #[test]
    fn row_count_agrees_with_wrap_lines_own_length() {
        for (text, width, tab) in [
            ("hello", 10, 8),
            ("", 10, 8),
            ("12345678", 4, 8),
            ("ab日", 3, 8),
            ("a\tb\tc", 5, 4),
        ] {
            assert_eq!(
                row_count(text, width, tab),
                wrap_line(text, width, tab).len(),
                "{text:?}"
            );
        }
    }

    #[test]
    fn position_to_cell_walks_a_single_row() {
        assert_eq!(position_to_cell("hello", 0, 10, 8), (0, 0));
        assert_eq!(position_to_cell("hello", 3, 10, 8), (0, 3));
        assert_eq!(position_to_cell("hello", 5, 10, 8), (0, 5)); // one past the last character
    }

    #[test]
    fn position_to_cell_puts_a_row_boundary_at_the_start_of_the_next_row() {
        // "1234567890" wraps to "1234"/"5678"/"90" at width 4: offset 4 (right after "1234") is
        // (1, 0), not (0, 4) -- the boundary belongs to the row that starts there.
        assert_eq!(position_to_cell("1234567890", 4, 4, 8), (1, 0));
        assert_eq!(position_to_cell("1234567890", 8, 4, 8), (2, 0));
        assert_eq!(position_to_cell("1234567890", 10, 4, 8), (2, 2)); // the true end: no next row
    }

    #[test]
    fn position_to_cell_finds_the_trailing_row_of_an_exactly_full_line() {
        assert_eq!(position_to_cell("12345", 5, 5, 8), (1, 0)); // not (0, 5)
    }

    #[test]
    fn position_to_cell_accounts_for_a_wide_glyphs_two_columns() {
        assert_eq!(position_to_cell("a日b", "a日".len(), 10, 8), (0, 3)); // 1 (a) + 2 (日)
    }

    #[test]
    fn position_to_cell_and_cell_to_position_round_trip_every_offset() {
        for (text, width, tab) in [
            ("hello world", 5, 8),
            ("a\tb\tcd", 6, 4),
            ("ab日本cd", 4, 8),
            ("", 5, 8),
            ("12345", 5, 8), // exactly full
        ] {
            for byte_offset in text.char_indices().map(|(i, _)| i).chain([text.len()]) {
                let (row, col) = position_to_cell(text, byte_offset, width, tab);
                assert_eq!(
                    cell_to_position(text, row, col, width, tab),
                    byte_offset,
                    "{text:?} at {byte_offset}"
                );
            }
        }
    }

    #[test]
    fn cell_to_position_lands_before_a_glyph_that_would_straddle_the_target_column() {
        // "a日b": columns are a=0, 日=1..3, b=3. Asking for column 2 (inside 日) lands before it,
        // at column 1's position -- the same as asking for column 1.
        assert_eq!(
            cell_to_position("a日b", 0, 2, 10, 8),
            cell_to_position("a日b", 0, 1, 10, 8)
        );
    }

    #[test]
    fn cell_to_position_past_the_rows_own_width_lands_at_its_end() {
        assert_eq!(cell_to_position("hi", 0, 99, 10, 8), 2);
    }

    #[test]
    fn cell_to_position_clamps_a_row_past_the_last_one() {
        assert_eq!(cell_to_position("hello", 99, 0, 10, 8), 0);
    }

    #[test]
    fn a_typed_tab_fills_to_the_next_stop() {
        assert_eq!(tab_fill("", 0, 80, 4), 4);
        assert_eq!(tab_fill("ab", 2, 80, 4), 2);
        assert_eq!(tab_fill("abcd", 4, 80, 4), 4); // exactly on a stop: a whole one
        assert_eq!(tab_fill("日", 3, 80, 4), 2); // a wide glyph is two columns
        assert_eq!(tab_fill("a\tb", 2, 80, 4), 4); // after a tab the cursor is on column 4: a whole stop
    }
}
