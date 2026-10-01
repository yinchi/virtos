//! The text of the line being typed, and what each key does to it -- pure logic, no console or
//! keyboard device, so it builds and runs under `cargo test --target <host triple>`.
//!
//! `Mode` lives here, not with the code that draws the line, to keep this module host-testable:
//! `Prompt` is the shell's own prompt, `Canonical` is a running program's `read(0)` -- a real
//! tty's cooked mode. Neither is "raw mode" (a full-screen program reading keys directly, which
//! bypasses all of this). Today the two differ only in Ctrl+D: end-of-file is a property of
//! *reading a line for a program*, and a shell that never exits has nothing for it to mean. The
//! split is groundwork for what only `Prompt` will get (cursor movement, history), so those
//! arrive as new arms in `feed`, not a restructuring.
//!
//! The cursor is always at the end of the line -- there are no movement keys yet -- which is why
//! Backspace, Ctrl+U and Ctrl+W ("erase before the cursor") need no cursor state at all.

use alloc::string::String;
use core::mem;

use super::{KEY_D, KEY_H, KEY_U, KEY_W, Token};

/// Which kind of reader is waiting for the line -- see the module doc comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Prompt,
    Canonical,
}

/// What a key did to the line.
#[derive(Debug, PartialEq, Eq)]
pub enum LineEvent {
    /// The text changed; it needs redrawing.
    Changed,
    /// Enter: the finished line (without its newline), which is now empty again.
    Finished(String),
    /// Ctrl+D on a non-empty line in `Canonical` mode: what was typed so far, to be delivered
    /// *without* a newline (POSIX's rule); the line is now empty, so a further Ctrl+D is EOF.
    Partial(String),
    /// Ctrl+D on an empty line in `Canonical` mode.
    EndOfFile,
}

#[derive(Default)]
pub struct LineBuffer {
    text: String,
}

impl LineBuffer {
    pub const fn new() -> Self {
        Self {
            text: String::new(),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn clear(&mut self) {
        self.text.clear();
    }

    /// Takes the line out, leaving it empty.
    pub fn take(&mut self) -> String {
        mem::take(&mut self.text)
    }

    /// Applies one key. `None` means it did nothing that shows: a key with no binding here, or an
    /// edit that had nothing to act on (Backspace on an empty line).
    pub fn feed(&mut self, token: Token, mode: Mode) -> Option<LineEvent> {
        if token.ctrl && !token.alt {
            return match token.code {
                KEY_H => self.backspace(),
                KEY_U => self.kill_line(),
                KEY_W => self.kill_word(),
                KEY_D if mode == Mode::Canonical => Some(if self.text.is_empty() {
                    LineEvent::EndOfFile
                } else {
                    LineEvent::Partial(self.take())
                }),
                _ => None,
            };
        }
        match token.char()? {
            '\n' => Some(LineEvent::Finished(self.take())),
            '\x08' => self.backspace(),
            ch if ch == ' ' || ch.is_ascii_graphic() => {
                self.text.push(ch);
                Some(LineEvent::Changed)
            }
            _ => None,
        }
    }

    fn backspace(&mut self) -> Option<LineEvent> {
        self.text.pop().map(|_| LineEvent::Changed)
    }

    /// Ctrl+U: discards the whole line.
    fn kill_line(&mut self) -> Option<LineEvent> {
        if self.text.is_empty() {
            return None;
        }
        self.text.clear();
        Some(LineEvent::Changed)
    }

    /// Ctrl+W: erases the blank-delimited word before the end of the line, and any blanks between
    /// it and the end (the tty's WERASE / readline's unix-word-rubout).
    fn kill_word(&mut self) -> Option<LineEvent> {
        if self.text.is_empty() {
            return None;
        }
        let kept = self
            .text
            .trim_end_matches(' ')
            .trim_end_matches(|c| c != ' ');
        self.text.truncate(kept.len());
        Some(LineEvent::Changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: u16 = 30;
    const KEY_ENTER: u16 = 28;
    const KEY_BACKSPACE: u16 = 14;
    const KEY_SPACE: u16 = 57;
    const KEY_F1: u16 = 59;

    fn key(code: u16) -> Token {
        Token {
            code,
            shift: false,
            ctrl: false,
            alt: false,
            caps: false,
        }
    }

    fn ctrl(code: u16) -> Token {
        Token {
            ctrl: true,
            ..key(code)
        }
    }

    fn type_str(line: &mut LineBuffer, mode: Mode, s: &str) {
        for ch in s.chars() {
            let code = match ch {
                'a' => KEY_A,
                ' ' => KEY_SPACE,
                _ => panic!("test helper only types 'a' and ' '"),
            };
            line.feed(key(code), mode);
        }
    }

    #[test]
    fn typing_appends_and_enter_finishes_and_empties() {
        let mut line = LineBuffer::new();
        assert_eq!(
            line.feed(key(KEY_A), Mode::Prompt),
            Some(LineEvent::Changed)
        );
        assert_eq!(line.as_str(), "a");
        assert_eq!(
            line.feed(key(KEY_ENTER), Mode::Prompt),
            Some(LineEvent::Finished("a".into()))
        );
        assert_eq!(line.as_str(), "");
    }

    #[test]
    fn enter_on_an_empty_line_finishes_an_empty_line() {
        let mut line = LineBuffer::new();
        assert_eq!(
            line.feed(key(KEY_ENTER), Mode::Canonical),
            Some(LineEvent::Finished(String::new()))
        );
    }

    #[test]
    fn backspace_erases_the_last_character_and_ignores_an_empty_line() {
        let mut line = LineBuffer::new();
        assert_eq!(line.feed(key(KEY_BACKSPACE), Mode::Prompt), None);
        type_str(&mut line, Mode::Prompt, "aa");
        assert_eq!(
            line.feed(key(KEY_BACKSPACE), Mode::Prompt),
            Some(LineEvent::Changed)
        );
        assert_eq!(line.as_str(), "a");
        // Ctrl+H is the same key.
        assert_eq!(
            line.feed(ctrl(KEY_H), Mode::Prompt),
            Some(LineEvent::Changed)
        );
        assert_eq!(line.as_str(), "");
    }

    #[test]
    fn ctrl_u_discards_the_whole_line() {
        let mut line = LineBuffer::new();
        assert_eq!(line.feed(ctrl(KEY_U), Mode::Canonical), None);
        type_str(&mut line, Mode::Canonical, "aa a");
        assert_eq!(
            line.feed(ctrl(KEY_U), Mode::Canonical),
            Some(LineEvent::Changed)
        );
        assert_eq!(line.as_str(), "");
    }

    #[test]
    fn ctrl_w_erases_a_word_and_the_blanks_after_it() {
        let mut line = LineBuffer::new();
        type_str(&mut line, Mode::Prompt, "aa aaa  ");
        assert_eq!(
            line.feed(ctrl(KEY_W), Mode::Prompt),
            Some(LineEvent::Changed)
        );
        assert_eq!(line.as_str(), "aa ");
        line.feed(ctrl(KEY_W), Mode::Prompt);
        assert_eq!(line.as_str(), "");
        assert_eq!(line.feed(ctrl(KEY_W), Mode::Prompt), None);
    }

    #[test]
    fn ctrl_w_on_a_line_of_only_blanks_erases_it_all() {
        let mut line = LineBuffer::new();
        type_str(&mut line, Mode::Prompt, "   ");
        line.feed(ctrl(KEY_W), Mode::Prompt);
        assert_eq!(line.as_str(), "");
    }

    #[test]
    fn ctrl_d_on_an_empty_canonical_line_is_end_of_file() {
        let mut line = LineBuffer::new();
        assert_eq!(
            line.feed(ctrl(KEY_D), Mode::Canonical),
            Some(LineEvent::EndOfFile)
        );
    }

    #[test]
    fn ctrl_d_on_a_non_empty_canonical_line_delivers_it_then_eof_follows() {
        let mut line = LineBuffer::new();
        type_str(&mut line, Mode::Canonical, "aa");
        assert_eq!(
            line.feed(ctrl(KEY_D), Mode::Canonical),
            Some(LineEvent::Partial("aa".into()))
        );
        assert_eq!(line.as_str(), "");
        assert_eq!(
            line.feed(ctrl(KEY_D), Mode::Canonical),
            Some(LineEvent::EndOfFile)
        );
    }

    #[test]
    fn ctrl_d_at_the_prompt_does_nothing() {
        let mut line = LineBuffer::new();
        assert_eq!(line.feed(ctrl(KEY_D), Mode::Prompt), None);
        type_str(&mut line, Mode::Prompt, "aa");
        assert_eq!(line.feed(ctrl(KEY_D), Mode::Prompt), None);
        assert_eq!(line.as_str(), "aa");
    }

    #[test]
    fn unbound_keys_and_unbound_chords_are_ignored() {
        let mut line = LineBuffer::new();
        assert_eq!(line.feed(key(KEY_F1), Mode::Prompt), None);
        assert_eq!(line.feed(ctrl(KEY_A), Mode::Prompt), None);
        assert_eq!(line.feed(key(15), Mode::Prompt), None); // Tab
        assert_eq!(line.as_str(), "");
    }
}
