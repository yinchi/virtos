//! The text of the line being typed, and where the cursor sits within it (a byte offset, always on
//! a `char` boundary) -- pure logic, no console or keyboard device, so it builds and runs under
//! `cargo test --target <host triple>`. This is deliberately the *only* place Backspace/Delete/
//! cursor movement ever get handled: absorbed here, before Enter is ever reached, never forwarded
//! as raw control bytes to anything else. The line discipline (`line_discipline`) owns one and is
//! the only caller.
//!
//! `Mode` lives here, not with the code that draws the line, to keep this module host-testable:
//! `Prompt` is the shell's own prompt (a readline-style editor), `Canonical` is a running
//! program's `read(0)` -- a real tty's cooked mode, with no cursor movement and nothing past the
//! POSIX line-editing characters. Neither is "raw mode" (a full-screen program reading keys
//! directly, which bypasses all of this).
//!
//! `feed` matches on `Token::effective_code`, not `code`: the keypad's Enter is Enter, and with
//! NumLock off its keys are the navigation keys they are labelled as.

use alloc::string::String;
use core::mem;

use super::Token;
use abi::keys::{
    KEY_A, KEY_B, KEY_BACKSPACE, KEY_D, KEY_DELETE, KEY_E, KEY_END, KEY_ENTER, KEY_F, KEY_H,
    KEY_HOME, KEY_K, KEY_LEFT, KEY_RIGHT, KEY_U, KEY_W,
};

/// Which kind of reader is waiting for the line -- see the module doc comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The shell's prompt: Ctrl+D deletes under the cursor (the shell never exits on end-of-file).
    Prompt,
    /// A program's `read(0)`: Ctrl+D is end-of-file, which the line discipline handles before a
    /// token reaches here.
    Canonical,
}

/// What feeding one `Token` into the buffer did -- as `Option<LineEvent>` to include the
/// possibility of no state change (an unbound key, or a no-op like Left at column 0).
#[derive(Debug, PartialEq, Eq)]
pub enum LineEvent {
    /// The text changed (an insert, Backspace, Delete, or a kill): redraw the line.
    Changed,
    /// Only the cursor moved: redraw just the two affected cells.
    CursorMoved,
    /// Enter: the finished line (without its newline); the buffer is now empty again.
    Finished(String),
}

/// A line-editing buffer with an insertion point.
#[derive(Default)]
pub struct LineBuffer {
    text: String,
    cursor: usize,
}

/// A character of a "word" for word movement: a letter or a digit (readline's rule).
fn is_word(c: char) -> bool {
    c.is_alphanumeric()
}

impl LineBuffer {
    pub const fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
        }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The cursor's byte offset into `as_str`.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    /// Replaces the content wholesale (history recall); the cursor goes to the end, as on a
    /// freshly typed line.
    pub fn set(&mut self, text: &str) {
        self.text.clear();
        self.text.push_str(text);
        self.cursor = self.text.len();
    }

    /// Takes the line out, leaving it empty -- shared by Enter and by Ctrl+D's partial delivery.
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        mem::take(&mut self.text)
    }

    fn prev_boundary(&self) -> Option<usize> {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
    }

    fn next_boundary(&self) -> Option<usize> {
        let c = self.text[self.cursor..].chars().next()?;
        Some(self.cursor + c.len_utf8())
    }

    fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    fn backspace(&mut self) -> bool {
        match self.prev_boundary() {
            Some(start) => {
                self.text.remove(start);
                self.cursor = start;
                true
            }
            None => false,
        }
    }

    /// Removes the character at the cursor; the cursor doesn't move.
    fn delete_forward(&mut self) -> bool {
        if self.cursor >= self.text.len() {
            return false;
        }
        self.text.remove(self.cursor);
        true
    }

    fn move_to(&mut self, offset: usize) -> bool {
        if offset == self.cursor {
            return false;
        }
        self.cursor = offset;
        true
    }

    fn move_left(&mut self) -> bool {
        self.prev_boundary()
            .is_some_and(|start| self.move_to(start))
    }

    fn move_right(&mut self) -> bool {
        self.next_boundary().is_some_and(|end| self.move_to(end))
    }

    fn move_home(&mut self) -> bool {
        self.move_to(0)
    }

    fn move_end(&mut self) -> bool {
        self.move_to(self.text.len())
    }

    /// Removes `text[..cursor]`. Prompt's "kill to start" and `Canonical`'s POSIX KILL ("discard
    /// the whole line") are the same operation, since the cursor is always at the end in
    /// `Canonical`.
    fn kill_to_start(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.text.replace_range(..self.cursor, "");
        self.cursor = 0;
        true
    }

    /// Removes `text[cursor..]`; the cursor doesn't move. Prompt-only -- POSIX canonical mode has
    /// no kill-to-end character.
    fn kill_to_end(&mut self) -> bool {
        if self.cursor == self.text.len() {
            return false;
        }
        self.text.truncate(self.cursor);
        true
    }

    /// The byte offset reached by moving back from `from` over the characters `pred` accepts.
    fn back_while(&self, mut from: usize, pred: impl Fn(char) -> bool) -> usize {
        while let Some((i, c)) = self.text[..from].char_indices().next_back() {
            if !pred(c) {
                break;
            }
            from = i;
        }
        from
    }

    /// The byte offset reached by moving forward from `from` over the characters `pred` accepts.
    fn forward_while(&self, mut from: usize, pred: impl Fn(char) -> bool) -> usize {
        while let Some(c) = self.text[from..].chars().next() {
            if !pred(c) {
                break;
            }
            from += c.len_utf8();
        }
        from
    }

    /// Moves to the start of the word before the cursor (readline's backward-word: skip anything
    /// that is not part of a word, then the word itself).
    fn move_word_left(&mut self) -> bool {
        let start = self.back_while(self.back_while(self.cursor, |c| !is_word(c)), is_word);
        self.move_to(start)
    }

    /// Moves to the end of the next word (readline's forward-word).
    fn move_word_right(&mut self) -> bool {
        let end = self.forward_while(self.forward_while(self.cursor, |c| !is_word(c)), is_word);
        self.move_to(end)
    }

    /// Ctrl+W: removes the blank-delimited word before the cursor and the blanks between it and
    /// the cursor (the tty's WERASE and readline's unix-word-rubout: unlike `move_word_left`, a
    /// word here is anything that is not whitespace, so `ls /tmp/x` loses `/tmp/x` whole).
    fn kill_word_back(&mut self) -> bool {
        let start = self.back_while(self.back_while(self.cursor, char::is_whitespace), |c| {
            !c.is_whitespace()
        });
        if start == self.cursor {
            return false;
        }
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
        true
    }

    /// Feeds one token into the buffer, returning what happened, if anything. `mode` gates the
    /// keys that only mean something in `Mode::Prompt` (movement, Delete, Ctrl+A/E/K/B/F/H/D, the
    /// word keys): each returns `None` when `mode` is wrong, rather than falling through to the
    /// character-insertion arm -- they are simply absent from `Canonical`'s smaller key
    /// vocabulary, not "recognized then blocked". Ctrl+U and Ctrl+W have no such guard: both are
    /// correct in either mode. Up/Down (and Ctrl+P/N) aren't matched here at all: they replace
    /// the buffer via history rather than editing it, so the line discipline handles them. So is
    /// Ctrl+D in `Canonical` (end-of-file isn't an edit).
    pub fn feed(&mut self, token: Token, mode: Mode) -> Option<LineEvent> {
        let prompt = mode == Mode::Prompt;
        let (ctrl, alt) = (token.ctrl, token.alt);
        let moved = |ok: bool| ok.then_some(LineEvent::CursorMoved);
        let changed = |ok: bool| ok.then_some(LineEvent::Changed);
        match token.effective_code() {
            KEY_ENTER => Some(LineEvent::Finished(self.take())),
            KEY_BACKSPACE => changed(self.backspace()),
            KEY_DELETE if prompt => changed(self.delete_forward()),
            // The readline bindings, Prompt only, before the plain arrows because those ignore
            // Ctrl: Ctrl+Left/Right and Alt+B/F move by word.
            KEY_LEFT if ctrl && prompt => moved(self.move_word_left()),
            KEY_RIGHT if ctrl && prompt => moved(self.move_word_right()),
            KEY_B if alt && prompt => moved(self.move_word_left()),
            KEY_F if alt && prompt => moved(self.move_word_right()),
            KEY_B if ctrl && prompt => moved(self.move_left()),
            KEY_F if ctrl && prompt => moved(self.move_right()),
            KEY_H if ctrl && prompt => changed(self.backspace()),
            // Ctrl+D deletes under the cursor at the prompt (bash's, with text on the line; on an
            // empty one bash exits, and this shell never does).
            KEY_D if ctrl && prompt => changed(self.delete_forward()),
            KEY_W if ctrl => changed(self.kill_word_back()),
            KEY_LEFT if prompt => moved(self.move_left()),
            KEY_RIGHT if prompt => moved(self.move_right()),
            KEY_HOME if prompt => moved(self.move_home()),
            KEY_END if prompt => moved(self.move_end()),
            KEY_A if ctrl && prompt => moved(self.move_home()),
            KEY_E if ctrl && prompt => moved(self.move_end()),
            KEY_U if ctrl => changed(self.kill_to_start()),
            KEY_K if ctrl && prompt => changed(self.kill_to_end()),
            _ => {
                // Ctrl/Alt held: a shortcut, not text -- Ctrl+C must not type a literal 'c'.
                if ctrl || alt {
                    return None;
                }
                let c = token.char()?;
                // Control characters (Tab is the only one `char()` can produce) are excluded: a
                // tab-stop jump reads back identical to several literal spaces once it's in the
                // buffer, with no way to tell them apart later. Nothing defines an action for Tab
                // yet; a no-op is simplest until it becomes e.g. completion.
                if c.is_control() {
                    return None;
                }
                self.insert(c);
                Some(LineEvent::Changed)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use abi::keys::{
        KEY_KP1, KEY_KP4, KEY_KP6, KEY_KP7, KEY_KPDOT, KEY_KPENTER, KEY_KPPLUS, KEY_SPACE,
    };

    // Only the keys the tests type that `abi::keys` doesn't name.
    const KEY_X: u16 = 45;
    const KEY_F1: u16 = 59;

    fn key(code: u16) -> Token {
        Token {
            code,
            shift: false,
            ctrl: false,
            alt: false,
            caps: false,
            num: true,
            repeat: false,
        }
    }

    fn ctrl(code: u16) -> Token {
        Token {
            ctrl: true,
            ..key(code)
        }
    }

    fn alt(code: u16) -> Token {
        Token {
            alt: true,
            ..key(code)
        }
    }

    /// A buffer holding `text` with the cursor at its end, typed through `feed` itself.
    fn typed(mode: Mode, text: &str) -> LineBuffer {
        let mut line = LineBuffer::new();
        for ch in text.chars() {
            let code = match ch {
                'a' => KEY_A,
                'b' => KEY_B,
                'x' => KEY_X,
                ' ' => KEY_SPACE,
                _ => panic!("test helper only types 'a', 'b', 'x' and ' '"),
            };
            assert_eq!(line.feed(key(code), mode), Some(LineEvent::Changed));
        }
        line
    }

    /// Where the cursor is, as a count of characters from the start (the tests are all ASCII).
    fn at(line: &LineBuffer) -> usize {
        line.cursor()
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
        assert_eq!((line.as_str(), at(&line)), ("", 0));
    }

    #[test]
    fn keypad_enter_is_enter_and_keypad_digits_type_with_numlock() {
        let mut line = LineBuffer::new();
        assert_eq!(
            line.feed(key(KEY_KP7), Mode::Prompt),
            Some(LineEvent::Changed)
        );
        assert_eq!(
            line.feed(key(KEY_KPPLUS), Mode::Prompt),
            Some(LineEvent::Changed)
        );
        assert_eq!(line.as_str(), "7+");
        assert_eq!(
            line.feed(key(KEY_KPENTER), Mode::Prompt),
            Some(LineEvent::Finished("7+".into()))
        );
    }

    #[test]
    fn keypad_navigates_with_numlock_off() {
        let off = |code| Token {
            num: false,
            ..key(code)
        };
        let mut line = typed(Mode::Prompt, "aab");
        assert_eq!(
            line.feed(off(KEY_KP4), Mode::Prompt),
            Some(LineEvent::CursorMoved)
        );
        assert_eq!(at(&line), 2);
        assert_eq!(
            line.feed(off(KEY_KP7), Mode::Prompt),
            Some(LineEvent::CursorMoved)
        );
        assert_eq!(at(&line), 0);
        assert_eq!(
            line.feed(off(KEY_KP6), Mode::Prompt),
            Some(LineEvent::CursorMoved)
        );
        assert_eq!(
            line.feed(off(KEY_KP1), Mode::Prompt),
            Some(LineEvent::CursorMoved)
        );
        assert_eq!(at(&line), 3);
        // The keypad's dot is Delete.
        line.feed(key(KEY_LEFT), Mode::Prompt);
        assert_eq!(
            line.feed(off(KEY_KPDOT), Mode::Prompt),
            Some(LineEvent::Changed)
        );
        assert_eq!(line.as_str(), "aa");
    }

    #[test]
    fn backspace_erases_before_the_cursor_and_ignores_the_start() {
        let mut line = LineBuffer::new();
        assert_eq!(line.feed(key(KEY_BACKSPACE), Mode::Prompt), None);
        let mut line = typed(Mode::Prompt, "ab");
        line.feed(key(KEY_LEFT), Mode::Prompt);
        assert_eq!(
            line.feed(key(KEY_BACKSPACE), Mode::Prompt),
            Some(LineEvent::Changed)
        );
        assert_eq!((line.as_str(), at(&line)), ("b", 0));
        assert_eq!(line.feed(key(KEY_BACKSPACE), Mode::Prompt), None);
    }

    #[test]
    fn typing_inserts_at_the_cursor() {
        let mut line = typed(Mode::Prompt, "ab");
        line.feed(key(KEY_LEFT), Mode::Prompt);
        line.feed(key(KEY_X), Mode::Prompt);
        assert_eq!((line.as_str(), at(&line)), ("axb", 2));
    }

    #[test]
    fn left_right_home_end_move_and_stop_at_the_ends() {
        let mut line = typed(Mode::Prompt, "ab");
        assert_eq!(line.feed(key(KEY_RIGHT), Mode::Prompt), None);
        assert_eq!(
            line.feed(key(KEY_LEFT), Mode::Prompt),
            Some(LineEvent::CursorMoved)
        );
        assert_eq!(at(&line), 1);
        assert_eq!(
            line.feed(key(KEY_HOME), Mode::Prompt),
            Some(LineEvent::CursorMoved)
        );
        assert_eq!(line.feed(key(KEY_LEFT), Mode::Prompt), None);
        assert_eq!(line.feed(key(KEY_HOME), Mode::Prompt), None);
        assert_eq!(
            line.feed(key(KEY_END), Mode::Prompt),
            Some(LineEvent::CursorMoved)
        );
        assert_eq!(at(&line), 2);
        assert_eq!(line.feed(key(KEY_END), Mode::Prompt), None);
    }

    #[test]
    fn ctrl_chords_match_the_arrows() {
        let mut line = typed(Mode::Prompt, "ab");
        assert_eq!(
            line.feed(ctrl(KEY_B), Mode::Prompt),
            Some(LineEvent::CursorMoved)
        );
        assert_eq!(at(&line), 1);
        assert_eq!(
            line.feed(ctrl(KEY_A), Mode::Prompt),
            Some(LineEvent::CursorMoved)
        );
        assert_eq!(at(&line), 0);
        assert_eq!(
            line.feed(ctrl(KEY_F), Mode::Prompt),
            Some(LineEvent::CursorMoved)
        );
        assert_eq!(
            line.feed(ctrl(KEY_E), Mode::Prompt),
            Some(LineEvent::CursorMoved)
        );
        assert_eq!(at(&line), 2);
    }

    #[test]
    fn delete_and_ctrl_d_remove_the_character_under_the_cursor() {
        let mut line = typed(Mode::Prompt, "abx");
        line.feed(key(KEY_HOME), Mode::Prompt);
        assert_eq!(
            line.feed(key(KEY_DELETE), Mode::Prompt),
            Some(LineEvent::Changed)
        );
        assert_eq!((line.as_str(), at(&line)), ("bx", 0));
        assert_eq!(
            line.feed(ctrl(KEY_D), Mode::Prompt),
            Some(LineEvent::Changed)
        );
        assert_eq!(line.as_str(), "x");
        line.feed(key(KEY_END), Mode::Prompt);
        // Nothing at the end of the line to delete.
        assert_eq!(line.feed(key(KEY_DELETE), Mode::Prompt), None);
        assert_eq!(line.feed(ctrl(KEY_D), Mode::Prompt), None);
    }

    #[test]
    fn ctrl_h_is_backspace_at_the_prompt() {
        let mut line = typed(Mode::Prompt, "ab");
        assert_eq!(
            line.feed(ctrl(KEY_H), Mode::Prompt),
            Some(LineEvent::Changed)
        );
        assert_eq!(line.as_str(), "a");
    }

    #[test]
    fn ctrl_u_kills_to_the_start_and_ctrl_k_to_the_end() {
        let mut line = typed(Mode::Prompt, "aabb");
        line.feed(key(KEY_LEFT), Mode::Prompt);
        line.feed(key(KEY_LEFT), Mode::Prompt);
        assert_eq!(
            line.feed(ctrl(KEY_K), Mode::Prompt),
            Some(LineEvent::Changed)
        );
        assert_eq!((line.as_str(), at(&line)), ("aa", 2));
        assert_eq!(line.feed(ctrl(KEY_K), Mode::Prompt), None);
        line.feed(key(KEY_LEFT), Mode::Prompt);
        assert_eq!(
            line.feed(ctrl(KEY_U), Mode::Prompt),
            Some(LineEvent::Changed)
        );
        assert_eq!((line.as_str(), at(&line)), ("a", 0));
        assert_eq!(line.feed(ctrl(KEY_U), Mode::Prompt), None);
    }

    #[test]
    fn word_movement_skips_non_word_characters_first() {
        let mut line = typed(Mode::Prompt, "aa bb  xx");
        assert_eq!(
            line.feed(ctrl(KEY_LEFT), Mode::Prompt),
            Some(LineEvent::CursorMoved)
        );
        assert_eq!(at(&line), 7);
        assert_eq!(
            line.feed(alt(KEY_B), Mode::Prompt),
            Some(LineEvent::CursorMoved)
        );
        assert_eq!(at(&line), 3);
        line.feed(alt(KEY_B), Mode::Prompt);
        assert_eq!(at(&line), 0);
        assert_eq!(line.feed(alt(KEY_B), Mode::Prompt), None);
        assert_eq!(
            line.feed(ctrl(KEY_RIGHT), Mode::Prompt),
            Some(LineEvent::CursorMoved)
        );
        assert_eq!(at(&line), 2);
        line.feed(alt(KEY_F), Mode::Prompt);
        assert_eq!(at(&line), 5);
    }

    #[test]
    fn ctrl_w_erases_a_blank_delimited_word_and_the_blanks_after_it() {
        let mut line = typed(Mode::Prompt, "aa aaa  ");
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
    fn ctrl_w_works_mid_line_and_leaves_the_rest() {
        let mut line = typed(Mode::Prompt, "aa bb");
        line.feed(key(KEY_LEFT), Mode::Prompt);
        assert_eq!(
            line.feed(ctrl(KEY_W), Mode::Prompt),
            Some(LineEvent::Changed)
        );
        assert_eq!((line.as_str(), at(&line)), ("aa b", 3));
    }

    #[test]
    fn canonical_mode_has_only_the_posix_editing_keys() {
        let mut line = typed(Mode::Canonical, "aa bb");
        // Everything prompt-only is absent, not merely blocked.
        for token in [
            key(KEY_LEFT),
            key(KEY_RIGHT),
            key(KEY_HOME),
            key(KEY_END),
            key(KEY_DELETE),
            ctrl(KEY_A),
            ctrl(KEY_E),
            ctrl(KEY_B),
            ctrl(KEY_F),
            ctrl(KEY_K),
            ctrl(KEY_H),
            ctrl(KEY_D),
            ctrl(KEY_LEFT),
            alt(KEY_B),
        ] {
            assert_eq!(line.feed(token, Mode::Canonical), None);
        }
        assert_eq!((line.as_str(), at(&line)), ("aa bb", 5));
        // ... and the POSIX ones work.
        assert_eq!(
            line.feed(key(KEY_BACKSPACE), Mode::Canonical),
            Some(LineEvent::Changed)
        );
        assert_eq!(
            line.feed(ctrl(KEY_W), Mode::Canonical),
            Some(LineEvent::Changed)
        );
        assert_eq!(line.as_str(), "aa ");
        assert_eq!(
            line.feed(ctrl(KEY_U), Mode::Canonical),
            Some(LineEvent::Changed)
        );
        assert_eq!(line.as_str(), "");
    }

    #[test]
    fn set_replaces_the_text_and_puts_the_cursor_at_the_end() {
        let mut line = typed(Mode::Prompt, "ab");
        line.feed(key(KEY_HOME), Mode::Prompt);
        line.set("xxxx");
        assert_eq!((line.as_str(), at(&line)), ("xxxx", 4));
    }

    #[test]
    fn take_empties_the_line_and_resets_the_cursor() {
        let mut line = typed(Mode::Prompt, "ab");
        assert_eq!(line.take(), "ab");
        assert_eq!((line.as_str(), at(&line), line.is_empty()), ("", 0, true));
    }

    #[test]
    fn unbound_keys_and_unbound_chords_are_ignored() {
        let mut line = LineBuffer::new();
        assert_eq!(line.feed(key(KEY_F1), Mode::Prompt), None);
        assert_eq!(line.feed(ctrl(KEY_X), Mode::Prompt), None);
        assert_eq!(line.feed(alt(KEY_X), Mode::Prompt), None);
        assert_eq!(line.feed(key(15), Mode::Prompt), None); // Tab
        assert_eq!(line.as_str(), "");
    }
}
