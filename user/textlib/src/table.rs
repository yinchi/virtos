//! Left-aligned text columns, as `lsblk` prints its devices: every column as wide as its widest cell (the header
//! included), separated by two spaces, the last column not padded, so a row never ends in spaces. Pure, so it is tested
//! on the host.

use alloc::string::String;
use alloc::vec::Vec;

/// The header and rows laid out, one `\n`-ended line each. Every row has as many cells as the header.
pub fn render(header: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = header.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let mut out = String::new();
    let mut line = |cells: &mut dyn Iterator<Item = &str>| {
        let mut text = String::new();
        for (i, cell) in cells.enumerate() {
            if i > 0 {
                text.push_str("  ");
            }
            text.push_str(cell);
            for _ in cell.chars().count()..widths[i] {
                text.push(' ');
            }
        }
        out.push_str(text.trim_end());
        out.push('\n');
    };
    line(&mut header.iter().copied());
    for row in rows {
        line(&mut row.iter().map(String::as_str));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cells: &[&str]) -> Vec<String> {
        cells.iter().map(|c| String::from(*c)).collect()
    }

    #[test]
    fn columns_are_as_wide_as_their_widest_cell() {
        let text = render(
            &["SIZE", "LABEL"],
            &[row(&["64M", "SYSTEM"]), row(&["1M", "HOME"])],
        );
        assert_eq!(text, "SIZE  LABEL\n64M   SYSTEM\n1M    HOME\n");
    }

    #[test]
    fn a_wide_cell_widens_its_column() {
        let text = render(&["A", "B"], &[row(&["long cell", "x"])]);
        assert_eq!(text, "A          B\nlong cell  x\n");
    }

    #[test]
    fn empty_cells_and_the_last_column_leave_no_trailing_spaces() {
        let text = render(
            &["SIZE", "LABEL", "MOUNT"],
            &[row(&["2M", "", ""]), row(&["64M", "SYSTEM", "/"])],
        );
        assert_eq!(text, "SIZE  LABEL   MOUNT\n2M\n64M   SYSTEM  /\n");
    }

    #[test]
    fn no_rows_is_just_the_header() {
        assert_eq!(render(&["SIZE"], &[]), "SIZE\n");
    }
}
