//! What each character cell of the console holds, as far as *width* goes: a normal one-cell glyph, or
//! one half of a two-cell (wide) glyph. The console draws pixels and forgets what it drew, so without
//! this it could not tell that the character before the cursor was wide -- which Backspace needs, and
//! which overwriting half of a wide glyph needs (the other half must not be left behind).
//!
//! It also holds the cursor and its wrap rules (`Cursor`), which follow xterm's: a glyph that ends in the
//! last column leaves the cursor there with a *wrap pending*, and the wrap happens only when the next
//! glyph arrives -- so a full row followed by a newline does not produce a blank row.
//!
//! Pure `no_std` + `alloc`, with no dependency on the rest of the kernel, so it is tested on the host
//! (`just test-host`).

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cell {
    /// A one-cell glyph, or nothing drawn yet.
    Narrow,
    /// The left cell of a wide glyph.
    WideLeft,
    /// The right cell of a wide glyph.
    WideRight,
}

/// `Clone`, so a saved copy of the screen can keep the grid alongside its pixels: the pixels alone do
/// not say which cells are half a wide glyph.
#[derive(Clone)]
pub struct CellGrid {
    cols: usize,
    rows: usize,
    cells: Vec<Cell>,
}

impl CellGrid {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols,
            rows,
            cells: vec![Cell::Narrow; cols * rows],
        }
    }

    pub fn get(&self, row: usize, col: usize) -> Cell {
        self.cells[row * self.cols + col]
    }

    /// Whether a glyph `width` cells wide starting at `col` stays within a row of `cols` cells.
    pub fn fits(col: usize, width: usize, cols: usize) -> bool {
        col + width <= cols
    }

    /// Records a glyph of `width` (1 or 2) drawn at (`row`, `col`). Returns the columns of this row that
    /// were the other half of a wide glyph the new one only partly covers: the caller blanks those
    /// cells, so no half of a glyph is left on screen.
    pub fn place(&mut self, row: usize, col: usize, width: usize) -> [Option<usize>; 2] {
        let mut blank = [None, None];
        for c in col..col + width {
            match self.get(row, c) {
                // Its left half lies outside the new glyph (only possible for the first cell).
                Cell::WideRight if c == col => blank[0] = Some(c - 1),
                // Its right half lies outside the new glyph (only possible for the last cell).
                Cell::WideLeft if c + 1 >= col + width => blank[1] = Some(c + 1),
                _ => {}
            }
        }
        for c in blank.into_iter().flatten() {
            self.set(row, c, Cell::Narrow);
        }
        if width == 2 {
            self.set(row, col, Cell::WideLeft);
            self.set(row, col + 1, Cell::WideRight);
        } else {
            self.set(row, col, Cell::Narrow);
        }
        blank
    }

    /// The column Backspace moves the cursor at (`row`, `col`) to: one *character* back -- two cells
    /// if the character before the cursor is wide -- and never off the start of the row.
    pub fn back(&self, row: usize, col: usize) -> usize {
        match col {
            0 => 0,
            _ if self.get(row, col - 1) == Cell::WideRight => col.saturating_sub(2),
            _ => col - 1,
        }
    }

    pub fn clear_row(&mut self, row: usize) {
        self.cells[row * self.cols..(row + 1) * self.cols].fill(Cell::Narrow);
    }

    /// Moves every row up one, as the pixels do; the last row becomes empty.
    pub fn scroll_up(&mut self) {
        self.cells.copy_within(self.cols.., 0);
        self.clear_row(self.rows - 1);
    }

    fn set(&mut self, row: usize, col: usize, cell: Cell) {
        self.cells[row * self.cols + col] = cell;
    }
}

/// The cursor: a cell, plus whether a wrap is pending (see the module comment). While it is, `col` is
/// the last column, where the last glyph ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
    pub wrap_pending: bool,
}

impl Cursor {
    pub const fn new() -> Self {
        Self {
            row: 0,
            col: 0,
            wrap_pending: false,
        }
    }

    /// Moves to the start of the next row (a newline, or the wrap of a full row). Returns `true` if the
    /// cursor was already on the last row of `rows`, so the caller must scroll: the cursor then stays
    /// on that row.
    pub fn line_feed(&mut self, rows: usize) -> bool {
        self.col = 0;
        self.wrap_pending = false;
        if self.row >= rows - 1 {
            true
        } else {
            self.row += 1;
            false
        }
    }

    /// Readies the cursor for a glyph `width` cells wide: wraps first if a wrap is pending or the glyph
    /// would not fit in what is left of the row. Returns `true` if that wrap needs a scroll.
    pub fn start_glyph(&mut self, width: usize, cols: usize, rows: usize) -> bool {
        if self.wrap_pending || !CellGrid::fits(self.col, width, cols) {
            self.line_feed(rows)
        } else {
            false
        }
    }

    /// Advances past the glyph of `width` cells just drawn at `col`. One that ends in the last column
    /// leaves the cursor on it with the wrap pending.
    pub fn end_glyph(&mut self, width: usize, cols: usize) {
        if self.col + width >= cols {
            self.col = cols - 1;
            self.wrap_pending = true;
        } else {
            self.col += width;
        }
    }

    /// Carriage return: back to the start of the row.
    pub fn carriage_return(&mut self) {
        self.col = 0;
        self.wrap_pending = false;
    }

    /// Tab: on to the next multiple of `tab_width`, but never past the last column.
    pub fn tab(&mut self, tab_width: usize, cols: usize) {
        self.col = ((self.col / tab_width + 1) * tab_width).min(cols - 1);
        self.wrap_pending = false;
    }

    /// Backspace: one character back (see `CellGrid::back`), from the last column too.
    pub fn backspace(&mut self, grid: &CellGrid) {
        self.col = grid.back(self.row, self.col);
        self.wrap_pending = false;
    }

    /// Explicit positioning.
    pub fn move_to(&mut self, row: usize, col: usize) {
        self.row = row;
        self.col = col;
        self.wrap_pending = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Cell::*;

    const COLS: usize = 5;
    const ROWS: usize = 3;

    /// Types `text` (one char per cell, `W` = a wide glyph, `\n` a newline) the way `Console` does;
    /// returns the cursor and how many times it had to scroll.
    fn type_text(text: &str) -> (Cursor, usize) {
        let mut cur = Cursor::new();
        let mut scrolls = 0;
        for c in text.chars() {
            match c {
                '\n' => scrolls += cur.line_feed(ROWS) as usize,
                _ => {
                    let width = if c == 'W' { 2 } else { 1 };
                    scrolls += cur.start_glyph(width, COLS, ROWS) as usize;
                    cur.end_glyph(width, COLS);
                }
            }
        }
        (cur, scrolls)
    }

    #[test]
    fn filling_a_row_leaves_the_cursor_on_the_last_column_with_the_wrap_pending() {
        let (cur, scrolls) = type_text("abcde");
        assert_eq!(
            cur,
            Cursor {
                row: 0,
                col: 4,
                wrap_pending: true
            }
        );
        assert_eq!(scrolls, 0);
    }

    #[test]
    fn the_next_glyph_wraps_first() {
        let (cur, _) = type_text("abcdef");
        assert_eq!(
            cur,
            Cursor {
                row: 1,
                col: 1,
                wrap_pending: false
            }
        );
    }

    #[test]
    fn a_newline_after_a_full_row_leaves_no_blank_row() {
        let (cur, _) = type_text("abcde\nf");
        assert_eq!(
            cur,
            Cursor {
                row: 1,
                col: 1,
                wrap_pending: false
            }
        );
    }

    #[test]
    fn a_full_last_row_does_not_scroll_until_something_follows() {
        let (cur, scrolls) = type_text("abcdeabcdeabcde"); // three full rows
        assert_eq!((cur.row, cur.wrap_pending, scrolls), (2, true, 0));
        let (cur, scrolls) = type_text("abcdeabcdeabcdef");
        assert_eq!((cur.row, cur.col, scrolls), (2, 1, 1));
        let (_, scrolls) = type_text("abcdeabcdeabcde\n");
        assert_eq!(scrolls, 1); // the newline scrolls, once
    }

    #[test]
    fn a_wide_glyph_ending_in_the_last_column_sets_the_pending_wrap() {
        let (cur, _) = type_text("abcW");
        assert_eq!(
            cur,
            Cursor {
                row: 0,
                col: 4,
                wrap_pending: true
            }
        );
    }

    #[test]
    fn a_wide_glyph_that_does_not_fit_wraps_whole() {
        let (cur, _) = type_text("abcdW");
        assert_eq!(
            cur,
            Cursor {
                row: 1,
                col: 2,
                wrap_pending: false
            }
        ); // W in cells 0-1 of row 1
        let (cur, _) = type_text("abcdeW"); // the same after a full row (wrap pending)
        assert_eq!(
            cur,
            Cursor {
                row: 1,
                col: 2,
                wrap_pending: false
            }
        );
    }

    #[test]
    fn cursor_movement_clears_the_pending_wrap_without_wrapping() {
        for movement in [
            |c: &mut Cursor, _: &CellGrid| c.carriage_return(),
            |c: &mut Cursor, g: &CellGrid| c.backspace(g),
            |c: &mut Cursor, _: &CellGrid| c.tab(8, COLS),
            |c: &mut Cursor, _: &CellGrid| c.move_to(0, 2),
        ] {
            let (mut cur, _) = type_text("abcde");
            movement(&mut cur, &CellGrid::new(COLS, ROWS));
            assert!(!cur.wrap_pending);
            assert_eq!(cur.row, 0);
        }
    }

    #[test]
    fn backspace_from_the_last_column_moves_left_one_cell() {
        let (mut cur, _) = type_text("abcde");
        cur.backspace(&CellGrid::new(COLS, ROWS));
        assert_eq!(cur.col, 3);
    }

    #[test]
    fn a_tab_never_passes_the_last_column() {
        let mut cur = Cursor::new();
        cur.col = 2;
        cur.tab(8, COLS);
        assert_eq!(cur.col, 4);
    }

    #[test]
    fn a_wide_glyph_is_two_cells() {
        let mut g = CellGrid::new(10, 3);
        assert_eq!(g.place(0, 2, 2), [None, None]);
        assert_eq!(
            [g.get(0, 1), g.get(0, 2), g.get(0, 3), g.get(0, 4)],
            [Narrow, WideLeft, WideRight, Narrow]
        );
    }

    #[test]
    fn a_glyph_that_would_pass_the_last_column_does_not_fit() {
        assert!(CellGrid::fits(8, 2, 10));
        assert!(!CellGrid::fits(9, 2, 10));
        assert!(CellGrid::fits(9, 1, 10));
    }

    #[test]
    fn overwriting_one_half_blanks_the_other() {
        let mut g = CellGrid::new(10, 1);
        g.place(0, 2, 2);
        // A narrow glyph on the right half orphans the left half...
        assert_eq!(g.place(0, 3, 1), [Some(2), None]);
        assert_eq!([g.get(0, 2), g.get(0, 3)], [Narrow, Narrow]);
        // ...and on the left half, the right half.
        g.place(0, 2, 2);
        assert_eq!(g.place(0, 2, 1), [None, Some(3)]);
        assert_eq!([g.get(0, 2), g.get(0, 3)], [Narrow, Narrow]);
    }

    #[test]
    fn a_wide_glyph_straddling_two_wide_glyphs_blanks_both_outer_halves() {
        let mut g = CellGrid::new(10, 1);
        g.place(0, 0, 2); // cells 0,1
        g.place(0, 2, 2); // cells 2,3
        assert_eq!(g.place(0, 1, 2), [Some(0), Some(3)]); // now 1,2; 0 and 3 are orphans
        assert_eq!(
            [g.get(0, 0), g.get(0, 1), g.get(0, 2), g.get(0, 3)],
            [Narrow, WideLeft, WideRight, Narrow]
        );
    }

    #[test]
    fn exactly_overwriting_a_wide_glyph_blanks_nothing() {
        let mut g = CellGrid::new(10, 1);
        g.place(0, 4, 2);
        assert_eq!(g.place(0, 4, 2), [None, None]);
    }

    #[test]
    fn backspace_moves_back_one_character() {
        let mut g = CellGrid::new(10, 1);
        g.place(0, 0, 1); // a
        g.place(0, 1, 2); // wide, cells 1-2
        g.place(0, 3, 1); // b; cursor at 4
        assert_eq!(g.back(0, 4), 3); // over b
        assert_eq!(g.back(0, 3), 1); // over the wide glyph: two cells
        assert_eq!(g.back(0, 1), 0); // over a
        assert_eq!(g.back(0, 0), 0); // stays at the start of the row
    }

    #[test]
    fn scrolling_moves_the_grid_with_the_rows() {
        let mut g = CellGrid::new(4, 3);
        g.place(1, 0, 2);
        g.place(2, 2, 2);
        g.scroll_up();
        assert_eq!([g.get(0, 0), g.get(0, 1)], [WideLeft, WideRight]);
        assert_eq!([g.get(1, 2), g.get(1, 3)], [WideLeft, WideRight]);
        assert_eq!(g.get(2, 2), Narrow);
    }

    #[test]
    fn clearing_forgets_wide_glyphs() {
        let mut g = CellGrid::new(4, 2);
        g.place(0, 0, 2);
        g.place(1, 0, 2);
        g.clear_row(0);
        assert_eq!(g.get(0, 0), Narrow);
        assert_eq!(g.get(1, 0), WideLeft);
        g.clear_row(1);
        assert_eq!(g.get(1, 0), Narrow);
    }
}
