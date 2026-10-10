//! Turning one screen row's text (a [`Row`] from `layout::wrap_line`) into the actual
//! `abi::ioctl::Cell`s a `CONSOLE_DRAW` frame needs: a tab becomes that many blank cells up to its
//! stop, a wide glyph's second column is filled with a blank cell (never read by the kernel, but
//! still needs an entry there -- see `abi::ioctl::Cell`'s own doc comment), and a zero-width mark
//! draws nothing. The row is always padded out to exactly `cols` cells, whatever it actually drew,
//! so a screen built row by row this way is always exactly `rows * cols` cells long, matching
//! `CONSOLE_DRAW`'s own requirement.
//!
//! Uses [`crate::layout::width_at`], the very function `wrap_line` measured the row with, so a row
//! it produced can never overflow here: this is deliberately not a second, independent width
//! computation that could drift from it.
//!
//! Pure `no_std` + `alloc`, so it is tested on the host (`cargo test`).

use alloc::vec::Vec;

use abi::ioctl::Cell;

use crate::layout::{Row, width_at};

/// Renders one screen row -- `text[row.start..row.end]`, already known to fit within `cols` columns
/// -- as exactly `cols` cells, appended to `out`.
pub fn render_row(out: &mut Vec<Cell>, text: &str, row: Row, cols: usize, tab_size: usize) {
    let mut col = 0;
    for c in text[row.start..row.end].chars() {
        let width = width_at(c, col, tab_size).min(cols.saturating_sub(col));
        if width == 0 {
            continue; // a zero-width mark: nothing to draw for it
        }
        out.push(if c == '\t' {
            Cell::plain(' ')
        } else {
            Cell::plain(c)
        });
        for _ in 1..width {
            out.push(Cell::plain(' ')); // the rest of a tab's stop, or a wide glyph's second column
        }
        col += width;
    }
    for _ in col..cols {
        out.push(Cell::plain(' '));
    }
}

/// `text[row.start..byte]`'s display width -- the column `byte` falls at within `row`'s own local
/// coordinates (column 0 at `row.start`), for any `byte` in `row.start..=row.end`. Used by
/// [`region_columns`] instead of `layout::position_to_cell`, which at exactly `row.end` reports the
/// *next* row's column 0 instead (its own documented rule for where a row boundary belongs) -- not
/// what a highlight's own right edge, drawn on *this* row, needs.
fn column_within_row(text: &str, row: Row, byte: usize, tab_size: usize) -> usize {
    let mut col = 0;
    for c in text[row.start..byte].chars() {
        col += width_at(c, col, tab_size);
    }
    col
}

/// The screen-column range of `row` (line `line`'s wrapped row `row`) that falls inside the marked
/// region from `start` to `end` (ordered -- `region::ordered`'s job, not this function's) -- `None`
/// if none of `row` is inside it. Meant to `ATTR_INVERSE`-mark cells `render_row` already built, the
/// same after-the-fact way the cursor's own cell is marked, rather than a selection parameter woven
/// into `render_row` itself.
pub fn region_columns(
    text: &str,
    row: Row,
    line: usize,
    start: (usize, usize),
    end: (usize, usize),
    tab_size: usize,
) -> Option<(usize, usize)> {
    if line < start.0 || line > end.0 {
        return None;
    }
    let row_start = if line == start.0 {
        start.1.max(row.start)
    } else {
        row.start
    };
    let row_end = if line == end.0 {
        end.1.min(row.end)
    } else {
        row.end
    };
    if row_start >= row_end {
        return None;
    }
    Some((
        column_within_row(text, row, row_start, tab_size),
        column_within_row(text, row, row_end, tab_size),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::wrap_line;

    fn row_cells(text: &str, cols: usize, tab_size: usize) -> Vec<Vec<char>> {
        wrap_line(text, cols, tab_size)
            .into_iter()
            .map(|row| {
                let mut out = Vec::new();
                render_row(&mut out, text, row, cols, tab_size);
                assert_eq!(
                    out.len(),
                    cols,
                    "row for {text:?} must be exactly {cols} cells"
                );
                out.into_iter()
                    .map(|c| char::from_u32(c.ch).unwrap())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_short_line_is_padded_with_blanks() {
        assert_eq!(row_cells("hi", 5, 8), [['h', 'i', ' ', ' ', ' ']]);
    }

    #[test]
    fn every_row_of_a_wrapped_line_is_exactly_cols_cells() {
        for (text, cols, tab) in [
            ("", 5, 8),
            ("hello world", 4, 8),
            ("12345", 5, 8), // exactly full, plus the trailing empty row
            ("a\tb\tc", 5, 4),
            ("ab日", 3, 8),
        ] {
            row_cells(text, cols, tab); // the assertion inside row_cells does the real checking
        }
    }

    #[test]
    fn a_tab_fills_blanks_up_to_its_stop() {
        // tab_size 4: 'a' then a tab reaching column 4, i.e. 3 blank cells.
        assert_eq!(row_cells("a\tb", 6, 4), [['a', ' ', ' ', ' ', 'b', ' ']]);
    }

    #[test]
    fn a_wide_glyph_is_followed_by_one_blank_filler_cell() {
        // "a日b" exactly fills 4 columns, so (as usual) there is a trailing empty row too.
        assert_eq!(
            row_cells("a日b", 4, 8),
            [['a', '日', ' ', 'b'], [' ', ' ', ' ', ' ']]
        );
    }

    #[test]
    fn a_zero_width_mark_draws_nothing() {
        assert_eq!(row_cells("a\u{200B}b", 4, 8), [['a', 'b', ' ', ' ']]);
    }

    #[test]
    fn an_empty_row_is_all_blanks() {
        assert_eq!(row_cells("", 3, 8), [[' ', ' ', ' ']]);
    }

    fn row_of(text: &str) -> Row {
        Row {
            start: 0,
            end: text.len(),
        }
    }

    #[test]
    fn region_columns_within_one_line_and_row() {
        let row = row_of("hello world");
        assert_eq!(
            region_columns("hello world", row, 0, (0, 6), (0, 11), 8),
            Some((6, 11))
        );
    }

    #[test]
    fn region_columns_is_none_off_the_regions_lines() {
        let row = row_of("abc");
        assert_eq!(region_columns("abc", row, 0, (1, 0), (2, 3), 8), None);
        assert_eq!(region_columns("abc", row, 3, (1, 0), (2, 3), 8), None);
    }

    #[test]
    fn region_columns_covers_a_whole_line_strictly_between_the_ends() {
        // Line 1 is entirely inside the region from line 0 to line 2: the whole row highlights.
        let row = row_of("middle");
        assert_eq!(
            region_columns("middle", row, 1, (0, 2), (2, 3), 8),
            Some((0, 6))
        );
    }

    #[test]
    fn region_columns_on_the_start_line_begins_at_the_marks_own_column() {
        let row = row_of("hello world");
        assert_eq!(
            region_columns("hello world", row, 0, (0, 6), (1, 3), 8),
            Some((6, 11)) // to the end of this line -- the region continues onto the next
        );
    }

    #[test]
    fn region_columns_on_the_end_line_stops_at_the_cursors_own_column() {
        let row = row_of("hello world");
        assert_eq!(
            region_columns("hello world", row, 1, (0, 6), (1, 5), 8),
            Some((0, 5)) // from this line's own start -- the region began on an earlier one
        );
    }

    #[test]
    fn region_columns_is_none_for_an_empty_region_on_one_line() {
        let row = row_of("abc");
        assert_eq!(region_columns("abc", row, 0, (0, 1), (0, 1), 8), None);
    }

    #[test]
    fn region_columns_only_covers_a_wrapped_lines_own_row() {
        // "0123456789" at width 4 wraps to rows [0..4), [4..8), [8..10). A region from column 5 to
        // column 9 touches only the second and third rows, not the first.
        let text = "0123456789";
        let rows = wrap_line(text, 4, 8);
        assert_eq!(region_columns(text, rows[0], 0, (0, 5), (0, 9), 8), None);
        assert_eq!(
            region_columns(text, rows[1], 0, (0, 5), (0, 9), 8),
            Some((1, 4)) // columns 5..8, local to this row: 5-4=1 to 8-4=4
        );
        assert_eq!(
            region_columns(text, rows[2], 0, (0, 5), (0, 9), 8),
            Some((0, 1)) // column 8..9, local to this row: 8-8=0 to 9-8=1
        );
    }

    #[test]
    fn region_columns_respects_tab_size() {
        // "a\tb": a tab at column 1 reaches column 4 with tab_size 4, so "b" starts at column 4.
        let row = row_of("a\tb");
        assert_eq!(
            region_columns("a\tb", row, 0, (0, 2), (0, 3), 4),
            Some((4, 5))
        );
    }
}
