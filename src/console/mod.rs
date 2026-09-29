//! A software text console: cursor-addressable character cells rendered as glyphs into a raw
//! pixel framebuffer.
//!
//! Deliberately decoupled from `drivers::virtio::gpu`'s `FramebufferInfo` -- that type lives in a
//! hardware-only module (gated out of the host test build, see `Cargo.toml`), so this module
//! defines its own `Framebuffer` with the same shape instead of depending on it. `arch::init`
//! constructs one from the other's fields when wiring the two together.
//!
//! `put_pixel` is a plain volatile write through a raw pointer -- no hardware-specific crate
//! involved -- so, backed by a `Vec<u8>` instead of a real framebuffer, this whole module runs
//! under `cargo test --target <host triple>` (`just test-host`) exactly as it does on target.

pub mod font;

use font::{GLYPH_HEIGHT, GLYPH_WIDTH, glyph_for};

/// A raw BGRX8888 pixel surface: 8 bits per channel, blue-green-red-padding = 32 bits/pixel.
/// Since the machine is little-endian, a color value 0xAARRGGBB is stored in memory as
/// BB GG RR AA.
pub struct Framebuffer {
    /// Pointer to the start of the pixel data. Must stay valid, and exclusively ours, for as
    /// long as the `Framebuffer` exists.
    pub ptr: *mut u8,
    pub width: usize,
    pub height: usize,
    /// Bytes per row; may exceed `width * 4` if the source pads rows.
    pub stride: usize,
}

impl Framebuffer {
    fn pixel_offset(&self, x: usize, y: usize) -> usize {
        y * self.stride + x * 4
    }

    fn put_pixel(&self, x: usize, y: usize, bgrx: u32) {
        debug_assert!(x < self.width && y < self.height);
        // SAFETY: `x < width` and `y < height` (checked above), so `pixel_offset` lands within
        // the `height * stride` bytes `ptr` promises.
        unsafe {
            (self.ptr.add(self.pixel_offset(x, y)) as *mut u32).write_volatile(bgrx);
        }
    }
}

/// Column spacing for `\t`, matching the traditional terminal default.
const TAB_WIDTH: usize = 8;

/// A classic green-phosphor terminal look, for callers with no reason to pick their own -- the
/// one color choice this module makes, so it exists in exactly one place instead of being
/// duplicated at every call site (including the host tests, which just need two distinct values
/// and may as well use the real ones).
pub const DEFAULT_FG: u32 = 0xFF77_FF77;
pub const DEFAULT_BG: u32 = 0x0000_0000;

/// A software text console: cursor-addressable character cells rendered as glyphs into a raw
/// pixel framebuffer.
pub struct Console {
    fb: Framebuffer,
    pub cols: usize,
    pub rows: usize,
    cursor_row: usize,
    cursor_col: usize,
    /// Set when a glyph was just drawn in the last column: the cursor stays put (at that last
    /// column, not yet moved to the next row) until the *next* character actually arrives --
    /// xterm's rule, resolved by `putc`'s default branch and `newline`'s callers. Without it, a
    /// line that exactly fills the width followed by `\n` would advance a row twice (once for
    /// the fill, once for the newline), silently dropping a row's content.
    wrap_pending: bool,
}

impl Console {
    pub fn new(fb: Framebuffer) -> Self {
        let cols = fb.width / GLYPH_WIDTH;
        let rows = fb.height / GLYPH_HEIGHT;
        Self {
            fb,
            cols,
            rows,
            cursor_row: 0,
            cursor_col: 0,
            wrap_pending: false,
        }
    }

    /// Places `ch` at (`row`, `col`) with the given foreground/background colors, without moving
    /// the cursor.
    fn put_char(&self, row: usize, col: usize, ch: char, fg: u32, bg: u32) {
        assert!(row < self.rows && col < self.cols);
        let glyph = glyph_for(ch);
        let x0 = col * GLYPH_WIDTH;
        let y0 = row * GLYPH_HEIGHT;
        for dy in 0..GLYPH_HEIGHT {
            for dx in 0..GLYPH_WIDTH {
                let set = glyph.get_pixel(dx, dy);
                self.fb
                    .put_pixel(x0 + dx, y0 + dy, if set { fg } else { bg });
            }
        }
    }

    /// Clears one row of character cells to `bg`.
    fn clear_row(&self, row: usize, bg: u32) {
        for dy in 0..GLYPH_HEIGHT {
            for x in 0..self.fb.width {
                self.fb.put_pixel(x, row * GLYPH_HEIGHT + dy, bg);
            }
        }
    }

    /// Clears the whole screen to `bg` and homes the cursor.
    pub fn clear(&mut self, bg: u32) {
        for row in 0..self.rows {
            self.clear_row(row, bg);
        }
        self.cursor_row = 0;
        self.cursor_col = 0;
    }

    /// Scrolls the whole framebuffer up by one row of cells, discarding the top row, and clears
    /// the newly-exposed bottom row to `bg`.
    fn scroll(&mut self, bg: u32) {
        let row_bytes = self.fb.stride * GLYPH_HEIGHT;
        let total_bytes = self.fb.stride * self.fb.height;
        // SAFETY: `row_bytes..total_bytes` and `0..total_bytes - row_bytes` both lie within the
        // `height * stride` bytes `fb.ptr` promises; the two ranges may overlap, hence `copy`
        // (memmove) rather than `copy_nonoverlapping`.
        unsafe {
            core::ptr::copy(
                self.fb.ptr.add(row_bytes),
                self.fb.ptr,
                total_bytes - row_bytes,
            );
        }
        self.clear_row(self.rows - 1, bg);
    }

    /// Advances the cursor to the start of the next row, scrolling if that was the last row.
    fn newline(&mut self, bg: u32) {
        self.cursor_col = 0;
        if self.cursor_row + 1 < self.rows {
            self.cursor_row += 1;
        } else {
            self.scroll(bg);
        }
    }

    /// Writes `ch` at the cursor and advances it: wraps to the next row at the end of a line
    /// (scrolling if needed), and interprets `\n`, `\r`, `\t` and backspace (`\x08`) instead of
    /// drawing them.
    pub fn putc(&mut self, ch: char, fg: u32, bg: u32) {
        match ch {
            '\n' => {
                self.wrap_pending = false;
                return self.newline(bg);
            }
            '\r' => {
                self.wrap_pending = false;
                self.cursor_col = 0;
                return;
            }
            '\x08' => {
                self.wrap_pending = false;
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                    self.put_char(self.cursor_row, self.cursor_col, ' ', fg, bg);
                }
                return;
            }
            '\t' => {
                let spaces = TAB_WIDTH - (self.cursor_col % TAB_WIDTH);
                for _ in 0..spaces {
                    self.putc(' ', fg, bg);
                }
                return;
            }
            _ => {}
        }
        if self.wrap_pending {
            self.wrap_pending = false;
            self.newline(bg);
        }
        self.put_char(self.cursor_row, self.cursor_col, ch, fg, bg);
        if self.cursor_col + 1 < self.cols {
            self.cursor_col += 1;
        } else {
            self.wrap_pending = true;
        }
    }

    /// Writes every character of `s` in turn (see `putc`).
    pub fn puts(&mut self, s: &str, fg: u32, bg: u32) {
        for ch in s.chars() {
            self.putc(ch, fg, bg);
        }
    }
}

/// The console `kernel_main` sets up once a GPU device is found -- lets `syscall::fd`'s `write`
/// reach it too, the same way `blk`/`gpu`/`input` are each reachable via their own module-level
/// `get()`. Hardware-only (a real `Console` needs a real framebuffer), unlike the rest of this
/// module.
#[cfg(target_os = "none")]
static mut CONSOLE: Option<Console> = None;

/// Set once, early in `kernel_main`, right after a GPU device is found.
#[cfg(target_os = "none")]
pub fn set(console: Console) {
    unsafe { CONSOLE = Some(console) };
}

/// The console, if `set` has been called (i.e. a GPU device was found).
#[cfg(target_os = "none")]
#[allow(clippy::deref_addrof)]
pub fn get() -> Option<&'static mut Console> {
    // SAFETY: single core; `set` runs once before anything else touches `CONSOLE`.
    unsafe { (&mut *(&raw mut CONSOLE)).as_mut() }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FG: u32 = DEFAULT_FG;
    const BG: u32 = DEFAULT_BG;

    /// A `Console` over a heap-backed pixel buffer, plus the buffer itself (kept alive alongside
    /// the `Console` that borrows it via raw pointer) -- `cols`x`rows` cells exactly, so wrap and
    /// scroll behavior is easy to provoke.
    fn test_console(cols: usize, rows: usize) -> (Console, Vec<u8>) {
        let width = cols * GLYPH_WIDTH;
        let height = rows * GLYPH_HEIGHT;
        let mut pixels = vec![0u8; width * height * 4];
        let fb = Framebuffer {
            ptr: pixels.as_mut_ptr(),
            width,
            height,
            stride: width * 4,
        };
        (Console::new(fb), pixels)
    }

    /// Whether the cell at (`row`, `col`) is non-blank: any pixel in it isn't `BG`. Doesn't check
    /// *which* glyph was drawn, only that something was -- `font::tests` covers glyph identity.
    fn cell_is_drawn(pixels: &[u8], stride: usize, row: usize, col: usize) -> bool {
        let x0 = col * GLYPH_WIDTH;
        let y0 = row * GLYPH_HEIGHT;
        (0..GLYPH_HEIGHT).any(|dy| {
            (0..GLYPH_WIDTH).any(|dx| {
                let off = (y0 + dy) * stride + (x0 + dx) * 4;
                u32::from_ne_bytes(pixels[off..off + 4].try_into().unwrap()) != BG
            })
        })
    }

    #[test]
    fn putc_draws_and_advances_the_cursor() {
        let (mut console, pixels) = test_console(4, 2);
        console.clear(BG);
        console.putc('a', FG, BG);
        assert!(cell_is_drawn(&pixels, console.cols * GLYPH_WIDTH * 4, 0, 0));
        assert!(!cell_is_drawn(
            &pixels,
            console.cols * GLYPH_WIDTH * 4,
            0,
            1
        ));
        console.putc('b', FG, BG);
        assert!(cell_is_drawn(&pixels, console.cols * GLYPH_WIDTH * 4, 0, 1));
    }

    #[test]
    fn a_full_row_wraps_to_the_next_without_a_blank_cell() {
        let (mut console, pixels) = test_console(2, 2);
        console.clear(BG);
        console.puts("ab", FG, BG); // fills row 0 exactly
        let stride = console.cols * GLYPH_WIDTH * 4;
        assert!(cell_is_drawn(&pixels, stride, 0, 0));
        assert!(cell_is_drawn(&pixels, stride, 0, 1));
        assert!(
            !cell_is_drawn(&pixels, stride, 1, 0),
            "wrap shouldn't skip to row 1 early"
        );
        console.putc('c', FG, BG);
        assert!(cell_is_drawn(&pixels, stride, 1, 0));
    }

    #[test]
    fn a_full_row_followed_by_newline_advances_only_one_row() {
        // Exactly the case xterm's wrap-pending rule exists for: without it, filling the last
        // column would eagerly wrap to row 1, and the following explicit '\n' would advance a
        // second time -- scrolling 'a' off a 2-row screen it should still be on.
        let (mut console, pixels) = test_console(1, 2);
        console.clear(BG);
        console.puts("a\n", FG, BG);
        let stride = console.cols * GLYPH_WIDTH * 4;
        assert!(
            cell_is_drawn(&pixels, stride, 0, 0),
            "'a' must not have scrolled off"
        );
        assert!(!cell_is_drawn(&pixels, stride, 1, 0));
    }

    #[test]
    fn newline_past_the_last_row_scrolls_instead_of_going_out_of_bounds() {
        let (mut console, pixels) = test_console(1, 2);
        console.clear(BG);
        console.puts("a\nb\nc", FG, BG); // 'a' on row 0, 'b' on row 1, then scroll, 'c' on row 1
        let stride = console.cols * GLYPH_WIDTH * 4;
        // Row 0 ('a') scrolled off; row 1 now holds 'b' shifted up... no -- scroll drops the top
        // row, so row 0 now holds what was row 1 ('b'), and 'c' lands on the newly-cleared row 1.
        assert!(
            cell_is_drawn(&pixels, stride, 0, 0),
            "row 1's 'b' should have scrolled up"
        );
        assert!(
            cell_is_drawn(&pixels, stride, 1, 0),
            "'c' should be on the new bottom row"
        );
    }

    #[test]
    fn backspace_erases_the_previous_cell_and_moves_the_cursor_back() {
        let (mut console, pixels) = test_console(4, 1);
        console.clear(BG);
        console.puts("ab", FG, BG);
        console.putc('\x08', FG, BG);
        let stride = console.cols * GLYPH_WIDTH * 4;
        assert!(
            !cell_is_drawn(&pixels, stride, 0, 1),
            "backspace should erase 'b'"
        );
        console.putc('c', FG, BG);
        assert!(
            cell_is_drawn(&pixels, stride, 0, 1),
            "cursor should be back at column 1"
        );
    }

    #[test]
    fn carriage_return_moves_to_column_zero_without_a_new_row() {
        let (mut console, pixels) = test_console(4, 2);
        console.clear(BG);
        console.puts("ab\rc", FG, BG); // overwrites column 0 with 'c', stays on row 0
        let stride = console.cols * GLYPH_WIDTH * 4;
        assert!(cell_is_drawn(&pixels, stride, 0, 0));
        assert!(
            !cell_is_drawn(&pixels, stride, 1, 0),
            "\\r must not advance to the next row"
        );
    }
}
