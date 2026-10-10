//! `ioctl` request codes: what a program can ask of the thing an fd is open on, out of band -- separate
//! from the bytes it reads and writes, so file contents can never control the display. The codes are
//! this project's own (Linux's are terminal-specific and would only mislead), with one deliberate
//! exception: `TIOCGWINSZ` keeps Linux's number and `struct winsize`, so a program learns "am I on a
//! terminal, and how big" the way it would there. The error for an fd that does not understand a
//! request is `ENOTTY`, as on Linux.

/// Clears the console and puts its cursor at the top left. Only the console (stdout/stderr, when they
/// are the console) understands it; `arg` is unused.
pub const CONSOLE_CLEAR: usize = 1;

/// Fills the [`WinSize`] that `arg` points to with the console's size in character cells (Linux's
/// `TIOCGWINSZ`, request number and struct). Answered for stdin (the keyboard) and stdout/stderr (the
/// display) while they are the console; `ENOTTY` when redirected, `EFAULT` for a bad `arg`.
pub const TIOCGWINSZ: usize = 0x5413;

/// Blocks until a key is pressed and fills the [`crate::keys::KeyEvent`] `arg` points to with it --
/// popped straight from the keyboard's token queue, one token per call, bypassing `read(0)`'s line
/// discipline entirely. Answered for stdin (the keyboard) only; `ENOTTY` on stdout/stderr or a
/// redirected stdin, `EFAULT` for a bad `arg`. This and `read(0)` are two independent ways to drain
/// the one queue -- whichever is called pops the next token -- so a program uses one or the other,
/// never both, and there is nothing to switch back on exit or on a fault (see `keyboard/stdin.rs`).
pub const CONSOLE_READ_KEY: usize = 2;

/// Draws one full frame of character cells on the console and places the cursor -- from EL0, since a
/// program can only `write()` bytes and the console itself lives in the kernel. `arg` points to a
/// [`ConsoleDraw`] header ([`CONSOLE_DRAW_HEADER_SIZE`] bytes) immediately followed by `rows * cols`
/// [`Cell`]s, row-major, `rows`/`cols` being the header's own fields. Refuses the whole frame -- draws
/// nothing -- if the header's `rows`/`cols` don't match the console's actual size (`EINVAL`: a program
/// that never called `TIOCGWINSZ`, or whose window changed since) or any cell is malformed (`EINVAL`;
/// see [`Cell`]); a bad pointer or a size that doesn't fit user memory is `EFAULT`. Answered for
/// stdout/stderr (the console) only, never stdin; `ENOTTY` otherwise.
pub const CONSOLE_DRAW: usize = 3;

/// Bytes in a [`WinSize`] as the kernel writes it.
pub const WINSIZE_SIZE: usize = 8;

/// Linux's `struct winsize`: four native-endian (little-endian here) `u16`s. The pixel fields are left 0:
/// a program needs the size in character cells, and the cell size is the console's own business.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WinSize {
    pub rows: u16,
    pub cols: u16,
    pub xpixel: u16,
    pub ypixel: u16,
}

impl WinSize {
    /// A size in character cells, pixel fields 0.
    pub const fn cells(rows: u16, cols: u16) -> Self {
        Self {
            rows,
            cols,
            xpixel: 0,
            ypixel: 0,
        }
    }

    /// The bytes the kernel writes to the user's buffer: `ws_row`, `ws_col`, `ws_xpixel`, `ws_ypixel`.
    pub fn encode(&self) -> [u8; WINSIZE_SIZE] {
        let mut out = [0u8; WINSIZE_SIZE];
        out[0..2].copy_from_slice(&self.rows.to_le_bytes());
        out[2..4].copy_from_slice(&self.cols.to_le_bytes());
        out[4..6].copy_from_slice(&self.xpixel.to_le_bytes());
        out[6..8].copy_from_slice(&self.ypixel.to_le_bytes());
        out
    }

    /// The inverse of [`WinSize::encode`].
    pub fn decode(bytes: [u8; WINSIZE_SIZE]) -> Self {
        Self {
            rows: u16::from_le_bytes([bytes[0], bytes[1]]),
            cols: u16::from_le_bytes([bytes[2], bytes[3]]),
            xpixel: u16::from_le_bytes([bytes[4], bytes[5]]),
            ypixel: u16::from_le_bytes([bytes[6], bytes[7]]),
        }
    }
}

/// The fixed part of a [`CONSOLE_DRAW`] call, immediately before its cells: the frame's declared size
/// (checked against the console's own -- see [`CONSOLE_DRAW`]) and where to place the cursor. A cursor
/// position past the console's range is clamped by the kernel, not refused: a program that means its
/// cursor just past the last column or row (the usual off-the-end sentinel after the last character)
/// is common, not a bug.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConsoleDraw {
    pub rows: u16,
    pub cols: u16,
    pub cursor_row: u16,
    pub cursor_col: u16,
}

/// Bytes in a [`ConsoleDraw`] header, before its cells.
pub const CONSOLE_DRAW_HEADER_SIZE: usize = 8;

impl ConsoleDraw {
    pub fn encode(&self) -> [u8; CONSOLE_DRAW_HEADER_SIZE] {
        let mut out = [0u8; CONSOLE_DRAW_HEADER_SIZE];
        out[0..2].copy_from_slice(&self.rows.to_le_bytes());
        out[2..4].copy_from_slice(&self.cols.to_le_bytes());
        out[4..6].copy_from_slice(&self.cursor_row.to_le_bytes());
        out[6..8].copy_from_slice(&self.cursor_col.to_le_bytes());
        out
    }

    pub fn decode(bytes: [u8; CONSOLE_DRAW_HEADER_SIZE]) -> Self {
        Self {
            rows: u16::from_le_bytes([bytes[0], bytes[1]]),
            cols: u16::from_le_bytes([bytes[2], bytes[3]]),
            cursor_row: u16::from_le_bytes([bytes[4], bytes[5]]),
            cursor_col: u16::from_le_bytes([bytes[6], bytes[7]]),
        }
    }
}

/// One character cell of a [`CONSOLE_DRAW`] frame: a Unicode scalar value and an attribute byte. Must
/// be a *non-control* scalar value (space and above; not `\t`/`\n`/`\u{8}` -- the editor expands tabs
/// itself into ordinary cells before this call, and a cell is one screen position, not a line-oriented
/// write); a cell that fails to decode as one, or that is a wide (East Asian fullwidth) glyph with no
/// room left in its row, refuses the whole frame with `EINVAL`. A wide glyph's *second* column still
/// needs an entry (the array is exactly `rows * cols` cells, one per screen column) but its content is
/// never read: the kernel already knows, from the glyph at the column before it, that this one is the
/// second half, the same way typing a wide character advances the console's cursor by two columns
/// without a caller-visible "continuation" concept. Reserved `attr` bits are ignored, not refused, so
/// a future flag doesn't break a program built against this one. 8 bytes, little-endian, 3 bytes of
/// padding after `attr` -- deliberately not packed, so it stays a plain `#[repr(C)]`-shaped value on
/// both sides of the syscall.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub ch: u32,
    pub attr: u8,
}

/// Swap the cell's foreground and background (as the line discipline's cursor block already does) --
/// the status bar, the mark's region, the help footer's key names.
pub const ATTR_INVERSE: u8 = 1;
/// Draw with a dimmer foreground -- the line-number gutter, a past-end-of-file filler.
pub const ATTR_DIM: u8 = 2;

/// Bytes in one encoded [`Cell`].
pub const CELL_SIZE: usize = 8;

impl Cell {
    /// A cell holding `ch` with no attribute.
    pub const fn plain(ch: char) -> Self {
        Self {
            ch: ch as u32,
            attr: 0,
        }
    }

    pub fn encode(&self) -> [u8; CELL_SIZE] {
        let mut out = [0u8; CELL_SIZE];
        out[0..4].copy_from_slice(&self.ch.to_le_bytes());
        out[4] = self.attr;
        out
    }

    pub fn decode(bytes: [u8; CELL_SIZE]) -> Self {
        Self {
            ch: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            attr: bytes[4],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_codes_are_pinned() {
        assert_eq!(CONSOLE_CLEAR, 1);
        assert_eq!(TIOCGWINSZ, 0x5413); // Linux's, on every architecture
        assert_eq!(CONSOLE_READ_KEY, 2);
        assert_eq!(CONSOLE_DRAW, 3);
    }

    #[test]
    fn console_draw_header_round_trips() {
        let header = ConsoleDraw {
            rows: 30,
            cols: 80,
            cursor_row: 12,
            cursor_col: 5,
        };
        assert_eq!(header.encode().len(), CONSOLE_DRAW_HEADER_SIZE);
        assert_eq!(ConsoleDraw::decode(header.encode()), header);
    }

    #[test]
    fn attr_flags_are_distinct_single_bits() {
        assert_eq!((ATTR_INVERSE, ATTR_DIM), (1, 2));
    }

    #[test]
    fn a_cell_is_eight_bytes_little_endian() {
        let cell = Cell {
            ch: 0x0001_F600,
            attr: ATTR_INVERSE | ATTR_DIM,
        };
        assert_eq!(cell.encode(), [0x00, 0xF6, 0x01, 0x00, 3, 0, 0, 0]);
        assert_eq!(cell.encode().len(), CELL_SIZE);
        assert_eq!(Cell::decode(cell.encode()), cell);
    }

    #[test]
    fn plain_has_no_attribute() {
        let cell = Cell::plain('X');
        assert_eq!(cell.ch, 'X' as u32);
        assert_eq!(cell.attr, 0);
    }

    #[test]
    fn winsize_is_linuxs_layout() {
        // rows, cols, xpixel, ypixel: four little-endian u16s, in that order.
        let bytes = WinSize {
            rows: 30,
            cols: 80,
            xpixel: 0x0102,
            ypixel: 0x0304,
        }
        .encode();
        assert_eq!(bytes, [30, 0, 80, 0, 0x02, 0x01, 0x04, 0x03]);
        assert_eq!(bytes.len(), WINSIZE_SIZE);
    }

    #[test]
    fn winsize_round_trips() {
        let size = WinSize::cells(30, 80);
        assert_eq!(WinSize::decode(size.encode()), size);
        assert_eq!((size.xpixel, size.ypixel), (0, 0));
    }
}
