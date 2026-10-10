//! The one single-line text-input widget the editor's message/prompt row uses: Save As now, search
//! and go-to-line. A trimmed-down `Buffer` -- insert, Backspace, Delete-forward, the
//! four moves -- with no line at all to split or join, since a prompt's answer is always one line.
//!
//! Pure `no_std` + `alloc`, so it is tested on the host (`cargo test`).

use alloc::string::{String, ToString};

/// A label (drawn before the input, never edited) and one line of editable text with a cursor.
pub struct Prompt {
    label: String,
    input: String,
    /// A byte offset into `input`, always on a character boundary.
    cursor: usize,
}

impl Prompt {
    /// A new prompt with `input` pre-filled (e.g. the current filename for Save As) and the cursor
    /// at its end, the way a freshly recalled history line starts in the shell.
    pub fn new(label: &str, input: &str) -> Self {
        Self {
            label: label.to_string(),
            input: input.to_string(),
            cursor: input.len(),
        }
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    fn prev_boundary(&self) -> Option<usize> {
        self.input[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
    }

    fn next_boundary(&self) -> Option<usize> {
        let c = self.input[self.cursor..].chars().next()?;
        Some(self.cursor + c.len_utf8())
    }

    pub fn insert_char(&mut self, c: char) {
        debug_assert_ne!(c, '\n', "a prompt is always one line");
        self.input.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub fn backspace(&mut self) -> bool {
        match self.prev_boundary() {
            Some(start) => {
                self.input.remove(start);
                self.cursor = start;
                true
            }
            None => false,
        }
    }

    pub fn delete_forward(&mut self) -> bool {
        match self.next_boundary() {
            Some(_) => {
                self.input.remove(self.cursor);
                true
            }
            None => false,
        }
    }

    pub fn move_left(&mut self) -> bool {
        match self.prev_boundary() {
            Some(start) => {
                self.cursor = start;
                true
            }
            None => false,
        }
    }

    pub fn move_right(&mut self) -> bool {
        match self.next_boundary() {
            Some(end) => {
                self.cursor = end;
                true
            }
            None => false,
        }
    }

    pub fn move_home(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor = 0;
        true
    }

    pub fn move_end(&mut self) -> bool {
        if self.cursor == self.input.len() {
            return false;
        }
        self.cursor = self.input.len();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_prompt_is_prefilled_with_the_cursor_at_its_end() {
        let p = Prompt::new("File Name to Write: ", "notes.txt");
        assert_eq!(p.label(), "File Name to Write: ");
        assert_eq!(p.input(), "notes.txt");
        assert_eq!(p.cursor(), "notes.txt".len());
    }

    #[test]
    fn an_empty_prefill_is_an_empty_prompt() {
        let p = Prompt::new("Search: ", "");
        assert_eq!(p.input(), "");
        assert_eq!(p.cursor(), 0);
    }

    #[test]
    fn insert_advances_the_cursor_by_the_characters_width_in_bytes() {
        let mut p = Prompt::new("", "");
        p.insert_char('a');
        p.insert_char('日');
        assert_eq!(p.input(), "a日");
        assert_eq!(p.cursor(), 1 + '日'.len_utf8());
    }

    #[test]
    fn backspace_and_delete_forward_edit_around_the_cursor() {
        let mut p = Prompt::new("", "abc");
        p.move_home();
        p.move_right();
        assert!(p.backspace());
        assert_eq!(p.input(), "bc");
        assert_eq!(p.cursor(), 0);
        assert!(p.delete_forward());
        assert_eq!(p.input(), "c");
        assert_eq!(p.cursor(), 0);
        assert!(!p.backspace()); // already at the start
    }

    #[test]
    fn delete_forward_at_the_end_does_nothing() {
        let mut p = Prompt::new("", "abc");
        assert!(!p.delete_forward());
    }

    #[test]
    fn home_and_end_move_to_the_ends_of_the_input() {
        let mut p = Prompt::new("", "abc");
        assert!(p.move_home());
        assert_eq!(p.cursor(), 0);
        assert!(!p.move_home());
        assert!(p.move_end());
        assert_eq!(p.cursor(), 3);
        assert!(!p.move_end());
    }

    #[test]
    fn left_and_right_stop_at_the_boundaries() {
        let mut p = Prompt::new("", "ab");
        p.move_home();
        assert!(!p.move_left());
        assert!(p.move_right());
        assert!(p.move_right());
        assert!(!p.move_right());
    }
}
