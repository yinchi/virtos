//! Filling a list of entries into columns to fit a line width, as `ls` and `column` do on a
//! terminal. Pure, so it is tested on the host.
//!
//! The rule is GNU `ls`'s: try the most columns that could possibly fit and take the first count
//! whose per-column widths (each as wide as its widest entry), plus [`SEPARATOR`] between columns,
//! fit the width; one column always "fits" however wide an entry is. Entries are laid out
//! **down** the columns first (`ls`'s default, `column`'s) or **across** the rows first (`-x`).

use alloc::string::String;
use alloc::vec::Vec;

use crate::width::text_width;

/// The gap between two columns.
pub const SEPARATOR: usize = 2;

/// Which way the entries run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    /// Down the first column, then the next: entry `i` is in column `i / rows`.
    Down,
    /// Along the first row, then the next: entry `i` is in column `i % columns`.
    Across,
}

/// A chosen layout: how many rows and columns, and how wide each column is.
#[derive(Debug, PartialEq, Eq)]
pub struct Grid {
    pub rows: usize,
    pub widths: Vec<usize>,
    fill: Fill,
    entries: usize,
}

impl Grid {
    /// The index of the entry shown at (`row`, `col`), or `None` for an empty cell at the end.
    pub fn entry(&self, row: usize, col: usize) -> Option<usize> {
        let index = match self.fill {
            Fill::Down => col * self.rows + row,
            Fill::Across => row * self.widths.len() + col,
        };
        (index < self.entries).then_some(index)
    }
}

/// The widths of each column if `entries` were laid out in `columns` columns.
fn grid_for(entry_widths: &[usize], columns: usize, fill: Fill) -> Grid {
    let entries = entry_widths.len();
    let rows = entries.div_ceil(columns).max(1);
    // With `Down` and a count that does not divide evenly, the last columns can be empty; the
    // columns actually used are what matter for the width.
    let used = match fill {
        Fill::Down => entries.div_ceil(rows).max(1),
        Fill::Across => columns.min(entries).max(1),
    };
    let mut grid = Grid {
        rows,
        widths: alloc::vec![0; used],
        fill,
        entries,
    };
    for (i, &w) in entry_widths.iter().enumerate() {
        let col = match fill {
            Fill::Down => i / rows,
            Fill::Across => i % used,
        };
        grid.widths[col] = grid.widths[col].max(w);
    }
    grid
}

/// How wide a grid is, separators included (the last column is not padded).
fn total(grid: &Grid) -> usize {
    grid.widths.iter().sum::<usize>() + SEPARATOR * (grid.widths.len() - 1)
}

/// The layout with the most columns that fits `line_width`, given each entry's display width.
/// Always at least one column. An empty list is a grid with no rows to print.
pub fn plan(entry_widths: &[usize], line_width: usize, fill: Fill) -> Grid {
    if entry_widths.is_empty() {
        return Grid {
            rows: 0,
            widths: alloc::vec![0],
            fill,
            entries: 0,
        };
    }
    for columns in (2..=entry_widths.len()).rev() {
        let grid = grid_for(entry_widths, columns, fill);
        if total(&grid) <= line_width {
            return grid;
        }
    }
    grid_for(entry_widths, 1, fill)
}

/// `entries` laid out by `grid` (as `plan` returned for their `widths`): one `\n`-ended line per
/// row, entries padded to their column's width, no trailing blanks.
pub fn render(entries: &[&str], entry_widths: &[usize], grid: &Grid) -> String {
    let mut out = String::new();
    for row in 0..grid.rows {
        let mut line = String::new();
        let mut pad = 0;
        for (col, &width) in grid.widths.iter().enumerate() {
            let Some(index) = grid.entry(row, col) else {
                continue;
            };
            // Blanks owed by the previous cell and the gap before this one are written only once
            // something follows them, so a row never ends in spaces.
            for _ in 0..pad {
                line.push(' ');
            }
            line.push_str(entries[index]);
            pad = width - entry_widths[index] + SEPARATOR;
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// A line's fields, as `column -t` reads them: with no `separators`, runs of blanks (spaces and
/// tabs) separate and are not themselves fields, so leading and trailing blanks make none; with
/// `separators` (the characters of `-s`), *each* occurrence of one ends a field, so two in a row
/// leave an empty field between them.
pub fn split_fields<'a>(line: &'a str, separators: Option<&str>) -> Vec<&'a str> {
    match separators {
        None => line.split([' ', '\t']).filter(|f| !f.is_empty()).collect(),
        Some(seps) => line.split(|c| seps.contains(c)).collect(),
    }
}

/// `rows` of fields aligned into a table: each column as wide as its widest cell (by display
/// width), cells joined by `separator`, the last cell of a row not padded -- so no row ends in
/// blanks. One `\n`-ended line per row.
pub fn table(rows: &[Vec<&str>], separator: &str) -> String {
    let mut widths: Vec<usize> = Vec::new();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            let width = text_width(cell);
            match widths.get_mut(i) {
                Some(w) => *w = (*w).max(width),
                None => widths.push(width),
            }
        }
    }
    let mut out = String::new();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            out.push_str(cell);
            if i + 1 < row.len() {
                for _ in text_width(cell)..widths[i] {
                    out.push(' ');
                }
                out.push_str(separator);
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lay(names: &[&str], width: usize, fill: Fill) -> String {
        let widths: Vec<usize> = names.iter().map(|n| n.chars().count()).collect();
        render(names, &widths, &plan(&widths, width, fill))
    }

    #[test]
    fn entries_run_down_the_columns_by_default() {
        let text = lay(&["a", "b", "c", "d", "e"], 80, Fill::Down);
        // Everything fits one row only if the width allows: 5 entries, 1 wide, 4 gaps of 2 = 13.
        assert_eq!(text, "a  b  c  d  e\n");
        let narrow = lay(&["a", "b", "c", "d", "e"], 8, Fill::Down);
        // Three columns fit (3 + 2*2 = 7): rows of two, filled down.
        assert_eq!(narrow, "a  c  e\nb  d\n");
    }

    #[test]
    fn across_runs_along_the_rows() {
        let text = lay(&["a", "b", "c", "d", "e"], 8, Fill::Across);
        assert_eq!(text, "a  b  c\nd  e\n");
    }

    #[test]
    fn each_column_is_as_wide_as_its_widest_entry() {
        let text = lay(&["x", "long name", "y", "z"], 80, Fill::Down);
        assert_eq!(text, "x  long name  y  z\n");
        // Four columns would need 12 + 6 = 18; three means two rows, which is two used columns:
        // {x, long name} and {y, z}.
        let two_rows = lay(&["x", "long name", "y", "z"], 16, Fill::Down);
        assert_eq!(two_rows, "x          y\nlong name  z\n");
    }

    #[test]
    fn nothing_fits_but_one_column() {
        let text = lay(&["aaaa", "bbbb"], 5, Fill::Down);
        assert_eq!(text, "aaaa\nbbbb\n");
        // An entry wider than the line is still listed, alone.
        assert_eq!(lay(&["abcdefghij"], 4, Fill::Down), "abcdefghij\n");
    }

    #[test]
    fn rows_never_end_in_blanks() {
        let text = lay(&["aa", "b", "cc", "d"], 80, Fill::Across);
        assert!(text.lines().all(|l| l == l.trim_end()));
    }

    #[test]
    fn a_ragged_last_column_is_left_short() {
        // 7 entries in 3 columns of 3 rows: the last column has one entry.
        let names = ["1", "2", "3", "4", "5", "6", "7"];
        let widths = alloc::vec![1; 7];
        let grid = plan(&widths, 7, Fill::Down);
        assert_eq!((grid.rows, grid.widths.len()), (3, 3));
        assert_eq!(render(&names, &widths, &grid), "1  4  7\n2  5\n3  6\n");
    }

    #[test]
    fn display_width_not_char_count_decides() {
        // Two wide entries (width 4 each, as `日本`): they do not fit a 9-column line side by side
        // with their separator (4 + 2 + 4 = 10), so they stack.
        let names = ["日本", "日本"];
        let widths = [4, 4];
        assert_eq!(
            render(&names, &widths, &plan(&widths, 9, Fill::Down)),
            "日本\n日本\n"
        );
        assert_eq!(
            render(&names, &widths, &plan(&widths, 10, Fill::Down)),
            "日本  日本\n"
        );
    }

    #[test]
    fn an_empty_list_prints_nothing() {
        assert_eq!(render(&[], &[], &plan(&[], 80, Fill::Down)), "");
    }

    #[test]
    fn blanks_separate_fields_unless_separators_are_given() {
        assert_eq!(split_fields("  a  b\tc ", None), ["a", "b", "c"]);
        assert_eq!(split_fields("a:b::c", Some(":")), ["a", "b", "", "c"]);
        assert_eq!(split_fields("a b:c", Some(":")), ["a b", "c"]);
        assert!(split_fields("   ", None).is_empty());
    }

    #[test]
    fn a_table_pads_every_column_but_the_last() {
        let rows = vec![
            vec!["name", "size"],
            vec!["a", "1"],
            vec!["long", "22", "x"],
        ];
        assert_eq!(table(&rows, "  "), "name  size\na     1\nlong  22    x\n");
        assert_eq!(table(&rows, "|"), "name|size\na   |1\nlong|22  |x\n");
    }

    #[test]
    fn a_table_aligns_by_display_width() {
        let rows = vec![vec!["日本", "x"], vec!["ab", "y"]];
        assert_eq!(table(&rows, " "), "日本 x\nab   y\n");
    }
}
