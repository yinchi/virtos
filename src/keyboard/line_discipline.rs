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
//! What a key does depends on the `Mode` (`line`): `Prompt` (the shell's prompt) edits at a movable
//! cursor and recalls history (`history`); `Canonical` (a program's `read(0)`) is a real tty's
//! cooked mode -- Backspace, Ctrl+U/W, Ctrl+D, no cursor movement, no history, nothing typed there
//! is ever recorded. The cursor is drawn as an inverted cell, and un-inverted when a line ends.
//!
//! A line longer than a row wraps onto the rows below it (`console::input_layout` computes how
//! many), and may not outgrow the screen: past `rows - 1` rows, further characters are ignored.
//! Every edit clears the line's rows and rewrites them -- simple, and correct across a wrap and
//! after the screen scrolls, at the cost of redrawing more than a one-character change needs.
//!
//! Neither mode is "raw mode": that is a full-screen program reading keys directly, which
//! bypasses this module entirely (and doesn't exist yet).
//!
//! The caller flushes the display (`gpu::flush`) after any outcome but `Ignored`.

use alloc::string::String;
use core::fmt::Write;

use super::Token;
use super::history::History;
pub use super::line::Mode;
use super::line::{LineBuffer, LineEvent};
use crate::console::input_layout::{cursor_position, fits_on_screen, rows_needed};
use crate::console::{Console, DEFAULT_BG as BG, DEFAULT_FG as FG};
use abi::keys::{KEY_D, KEY_DOWN, KEY_N, KEY_P, KEY_UP};

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
    /// since the reader may not draw anything for a while (so the display needs a flush).
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
    /// The prompt's command history -- untouched in `Canonical` mode, since a program's `read(0)`
    /// never records or recalls anything.
    history: History,
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
            history: History::new(),
        }
    }

    /// Starts a fresh line: `prefix` (drawn, but not part of the text), then an empty line, in
    /// `mode`. Starts on a new row if the cursor is mid-row, so nothing a program printed without
    /// a trailing newline gets overwritten. Also ends any in-progress history browsing, so a stale
    /// recall position never leaks into this line.
    pub fn begin(&mut self, console: &mut Console, prefix: &str, mode: Mode) {
        self.buf.clear();
        self.prefix.clear();
        self.prefix.push_str(prefix);
        self.mode = mode;
        self.history.reset_recall();
        if console.cursor().1 != 0 {
            console.putc('\n', FG, BG);
        }
        self.row = console.cursor().0;
        self.drawn_rows = 1;
        self.redraw(console);
    }

    /// Clears the rows the line occupied and rewrites prefix and text from its first row, then
    /// places the visible cursor. The first row is recomputed afterward from where the console's
    /// own cursor ended up, which absorbs a scroll the rewrite itself caused (the line moved up).
    fn redraw(&mut self, console: &mut Console) {
        let needed = rows_needed(&self.prefix, self.buf.as_str(), console.cols);
        for row in self.row..(self.row + self.drawn_rows.max(needed)).min(console.rows) {
            console.clear_row(row, BG);
        }
        console.move_cursor(self.row, 0);
        console.puts(&self.prefix, FG, BG);
        console.puts(self.buf.as_str(), FG, BG);
        self.row = (console.cursor().0 + 1).saturating_sub(needed);
        self.drawn_rows = needed;
        self.draw_cursor(console);
    }

    /// Draws the cell at byte offset `cursor` of `text`, inverted or in ordinary colors: whatever
    /// character is actually there, or a space past the end of the text. Takes `text` explicitly
    /// rather than reading the buffer, since un-inverting a line's cursor cell when it ends needs
    /// the text as it was *before* the buffer was emptied.
    fn draw_cell_at(&self, console: &mut Console, text: &str, cursor: usize, inverted: bool) {
        let (rel_row, col) = cursor_position(&self.prefix, text, cursor, console.cols);
        let ch = text[cursor..].chars().next().unwrap_or(' ');
        let (fg, bg) = if inverted { (BG, FG) } else { (FG, BG) };
        console.put_char_at(self.row + rel_row, col, ch, fg, bg);
    }

    fn draw_cursor(&self, console: &mut Console) {
        self.draw_cell_at(console, self.buf.as_str(), self.buf.cursor(), true);
    }

    /// A pure cursor move: un-inverts the old cursor cell, then draws the new one -- two cell
    /// writes however far apart they are. `old_cursor` is an offset into the *current* text, valid
    /// because `LineEvent::CursorMoved` only fires when the text itself didn't change.
    fn redraw_cursor(&self, console: &mut Console, old_cursor: usize) {
        self.draw_cell_at(console, self.buf.as_str(), old_cursor, false);
        self.draw_cursor(console);
    }

    /// Applies one key press.
    pub fn handle(&mut self, token: Token, console: &mut Console) -> LineOutcome {
        if self.mode == Mode::Canonical && token.ctrl && token.code == KEY_D {
            if token.repeat {
                // Holding Ctrl+D must not end this program's input and then the next one's.
                return LineOutcome::Ignored;
            }
            // Un-invert wherever the cursor was drawn: nothing else redraws this row before the
            // reader moves on (its next call, if any, starts a fresh line via `begin`).
            let cursor = self.buf.cursor();
            return if self.buf.is_empty() {
                self.draw_cell_at(console, "", cursor, false);
                LineOutcome::EndOfFile
            } else {
                let text = self.buf.take();
                self.draw_cell_at(console, &text, cursor, false);
                let mut uart = crate::bare_metal::get_uart();
                let _ = write!(uart, "{text}");
                LineOutcome::Partial(text)
            };
        }

        // History: Up/Down, and readline's Ctrl+P/Ctrl+N, which are the same two moves.
        let (plain, chord) = (!token.ctrl && !token.alt, token.ctrl && !token.alt);
        let code = token.effective_code();
        let prev = (code == KEY_UP && plain) || (code == KEY_P && chord);
        let next = (code == KEY_DOWN && plain) || (code == KEY_N && chord);
        if self.mode == Mode::Prompt && (prev || next) {
            let recalled = if prev {
                self.history.recall_prev(self.buf.as_str())
            } else {
                self.history.recall_next()
            };
            let Some(text) = recalled else {
                return LineOutcome::Ignored;
            };
            let text = String::from(text);
            // An entry recorded under a shorter prompt (the directory changed since) may no
            // longer fit; leave the line as it is rather than draw past the screen.
            if !fits_on_screen(&self.prefix, &text, console.cols, console.rows) {
                return LineOutcome::Ignored;
            }
            self.buf.set(&text);
            self.redraw(console);
            return LineOutcome::Edited;
        }

        // A character that would push the line past the screen is dropped before it ever reaches
        // the buffer, which has no way to take one back.
        if let Some(c) = token.char().filter(|&c| c == ' ' || c.is_ascii_graphic()) {
            let mut longer = String::from(self.buf.as_str());
            longer.push(c);
            if !fits_on_screen(&self.prefix, &longer, console.cols, console.rows) {
                return LineOutcome::Ignored;
            }
        }
        let old_cursor = self.buf.cursor();
        match self.buf.feed(token, self.mode) {
            None => LineOutcome::Ignored,
            Some(LineEvent::CursorMoved) => {
                self.redraw_cursor(console, old_cursor);
                LineOutcome::Edited
            }
            Some(LineEvent::Changed) => {
                self.redraw(console);
                LineOutcome::Edited
            }
            Some(LineEvent::Finished(text)) => {
                // Un-invert wherever the cursor was (Enter can be pressed with it anywhere):
                // a finished line is never redrawn, so nothing else would.
                self.draw_cell_at(console, &text, old_cursor, false);
                // Only the prompt has a history: what a program reads is that program's input,
                // not a command to recall.
                if self.mode == Mode::Prompt {
                    self.history.record(&text);
                    self.history.reset_recall();
                }
                console.putc('\n', FG, BG);
                let mut uart = crate::bare_metal::get_uart();
                let _ = writeln!(uart, "{text}");
                LineOutcome::Finished(text)
            }
        }
    }
}
