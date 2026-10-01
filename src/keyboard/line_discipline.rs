//! What turns a stream of `Token`s into an edited line on the console: owns the line being typed
//! (`LineBuffer`), which console row it starts on, how it is echoed and redrawn, and what
//! finishing it looks like. The shell's prompt and a running program's `read(0)` both use it, so
//! a line is typed, edited, drawn and finished identically whichever of them is waiting for it
//! (rather than each keeping its own copy of the logic).
//!
//! Rules that keep this module useful to both callers:
//! - It knows nothing about where tokens come from -- not the keyboard device, `wfi`, or
//!   interrupt state. Its callers pop them from `queue`.
//! - It is never called from interrupt context: the IRQ handler only enqueues tokens. It runs only
//!   in the shell's loop or inside a syscall, one at a time, which is why it needs no lock.
//! - It takes the `Console` as a parameter and never keeps one: a caller fetches
//!   `console::get()` fresh for each call, so no `&mut Console` is ever held across code (a
//!   program's `write`) that fetches its own.
//! - There is one instance, for the one console; what varies between uses is the prefix and the
//!   `Mode` passed to `begin`.
//!
//! A line longer than a row wraps onto the rows below it (`console::input_layout` computes how
//! many), and may not outgrow the screen: past `rows - 1` rows, further characters are ignored.
//! Every edit clears the line's rows and rewrites them -- simple, and correct across a wrap and
//! after the screen scrolls, at the cost of redrawing more than a one-character change needs.
//!
//! The caller flushes the display (`gpu::flush`) after any outcome but `Ignored`.

use alloc::string::String;
use core::fmt::Write;

use super::Token;
pub use super::line::Mode;
use super::line::{LineBuffer, LineEvent};
use crate::console::input_layout::{fits_on_screen, rows_needed};
use crate::console::{Console, DEFAULT_BG as BG, DEFAULT_FG as FG};

/// What handling one token did.
pub enum LineOutcome {
    /// Nothing that shows: a modifier, an unbound key, an edit with nothing to act on.
    Ignored,
    /// The line changed and was redrawn.
    Edited,
    /// Enter: here is the line's text. The console has moved to the next row and the UART has the
    /// line.
    Finished(String),
    /// Ctrl+D on a non-empty line in `Canonical` mode: what was typed so far, to be delivered
    /// *without* a newline. The text stays on screen, but the console's cursor is left after it,
    /// since the reader may not draw anything for a while.
    Partial(String),
    /// Ctrl+D on an empty line in `Canonical` mode.
    EndOfFile,
}

pub struct LineDiscipline {
    buf: LineBuffer,
    prefix: String,
    mode: Mode,
    /// The console row the line (prefix first) starts on, and how many rows it last drew over --
    /// what `redraw` clears before rewriting.
    row: usize,
    drawn_rows: usize,
}

/// SAFETY (every access, via `get`): single core, and the discipline runs only from the shell's
/// loop or inside a syscall, never nested and never from the IRQ handler -- see the module doc
/// comment.
static mut LINE_DISCIPLINE: LineDiscipline = LineDiscipline::new();

/// The one line discipline.
#[allow(clippy::deref_addrof)]
pub fn get() -> &'static mut LineDiscipline {
    // SAFETY: see LINE_DISCIPLINE.
    unsafe { &mut *(&raw mut LINE_DISCIPLINE) }
}

impl LineDiscipline {
    const fn new() -> Self {
        Self {
            buf: LineBuffer::new(),
            prefix: String::new(),
            mode: Mode::Prompt,
            row: 0,
            drawn_rows: 1,
        }
    }

    /// Starts a fresh line: `prefix` (drawn, but not part of the text), then an empty line, in
    /// `mode`. Starts on a new row if the cursor is mid-row, so nothing a program printed without
    /// a trailing newline gets overwritten.
    pub fn begin(&mut self, console: &mut Console, prefix: &str, mode: Mode) {
        self.buf.clear();
        self.prefix.clear();
        self.prefix.push_str(prefix);
        self.mode = mode;
        if console.cursor().1 != 0 {
            console.putc('\n', FG, BG);
        }
        self.row = console.cursor().0;
        self.drawn_rows = 1;
        self.redraw(console);
    }

    /// Clears the rows the line occupied and rewrites prefix and text from its first row. The
    /// first row is recomputed afterward from where the cursor actually ended up, which absorbs a
    /// scroll the rewrite itself caused.
    fn redraw(&mut self, console: &mut Console) {
        for row in self.row..(self.row + self.drawn_rows).min(console.rows) {
            console.clear_row(row, BG);
        }
        console.move_cursor(self.row, 0);
        console.puts(&self.prefix, FG, BG);
        console.puts(self.buf.as_str(), FG, BG);
        let needed = rows_needed(self.prefix.len(), self.buf.as_str().len(), console.cols);
        self.row = (console.cursor().0 + 1).saturating_sub(needed);
        self.drawn_rows = needed;
    }

    /// Applies one key press.
    pub fn handle(&mut self, token: Token, console: &mut Console) -> LineOutcome {
        // A character that would push the line past the screen is dropped before it ever reaches
        // the buffer, which has no way to take one back.
        if token
            .char()
            .is_some_and(|c| c == ' ' || c.is_ascii_graphic())
            && !fits_on_screen(
                self.prefix.len(),
                self.buf.as_str().len() + 1,
                console.cols,
                console.rows,
            )
        {
            return LineOutcome::Ignored;
        }
        match self.buf.feed(token, self.mode) {
            None => LineOutcome::Ignored,
            Some(LineEvent::Changed) => {
                self.redraw(console);
                LineOutcome::Edited
            }
            Some(LineEvent::Finished(text)) => {
                console.putc('\n', FG, BG);
                let mut uart = crate::bare_metal::get_uart();
                let _ = writeln!(uart, "{text}");
                LineOutcome::Finished(text)
            }
            Some(LineEvent::Partial(text)) => {
                let mut uart = crate::bare_metal::get_uart();
                let _ = write!(uart, "{text}");
                LineOutcome::Partial(text)
            }
            Some(LineEvent::EndOfFile) => LineOutcome::EndOfFile,
        }
    }
}
