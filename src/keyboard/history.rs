//! Command history for the shell's prompt (`Mode::Prompt` only -- a program's `read(0)` never
//! touches this). Append-only: `record` is the only method that ever writes `lines`, and it only
//! ever pushes -- `recall_prev`/`recall_next` only move a cursor over what's already there and
//! read it back, never rewrite it. So editing a recalled entry and pressing Enter never rewrites
//! history in place: the edited text is appended as a new entry (by whichever caller records it),
//! and the original recalled entry is untouched at its original position, matching real bash.
//!
//! Deliberately simple about editing a recalled line and then pressing Up/Down again: the edit is
//! discarded and recall continues from the stored (unedited) entry, not resumed from the edit --
//! real bash's readline is more elaborate here (an in-progress edit to a recalled entry survives
//! navigating away and back within one browsing session, only reverting once that specific entry
//! is actually submitted), but that's more machinery than this shell's history needs.
//!
//! Pure, with no dependency on the rest of the kernel, so it is tested on the host.

use alloc::string::String;
use alloc::vec::Vec;

/// How many lines are kept before the oldest is evicted -- generous (a shell session realistically
/// never approaches this before a reboot).
const CAPACITY: usize = 64;

/// A shell's command history: what's been run, plus where an Up/Down browsing session currently
/// is.
pub struct History {
    /// Oldest first; capped at `CAPACITY`, evicting the oldest entry on overflow.
    lines: Vec<String>,
    /// Index into `lines` currently shown, or `None` when showing the live/pending line (no
    /// recall in progress, or recall has walked past the newest entry back to it).
    cursor: Option<usize>,
    /// The in-progress line, saved by the first `recall_prev` of a browsing session and restored
    /// once `recall_next` walks back past the newest entry.
    pending: String,
}

impl History {
    pub const fn new() -> Self {
        Self {
            lines: Vec::new(),
            cursor: None,
            pending: String::new(),
        }
    }

    /// Records a finished line. Skips an empty line, and skips a line that exactly duplicates the
    /// *last* entry (only consecutive duplicates collapse -- recalling and re-running an older,
    /// non-last entry unedited still appends a new copy). Evicts the oldest entry once already at
    /// `CAPACITY`.
    pub fn record(&mut self, line: &str) {
        if line.is_empty() || self.lines.last().map(String::as_str) == Some(line) {
            return;
        }
        if self.lines.len() >= CAPACITY {
            self.lines.remove(0);
        }
        self.lines.push(String::from(line));
    }

    /// Moves one entry further back in history and returns it, saving `current` as the pending
    /// line on the first call of a browsing session. `None` (a no-op) at the oldest entry, or if
    /// there's no history at all.
    pub fn recall_prev(&mut self, current: &str) -> Option<&str> {
        let next_index = match self.cursor {
            None => {
                if self.lines.is_empty() {
                    return None;
                }
                self.pending.clear();
                self.pending.push_str(current);
                self.lines.len() - 1
            }
            Some(0) => return None,
            Some(i) => i - 1,
        };
        self.cursor = Some(next_index);
        Some(&self.lines[next_index])
    }

    /// Moves one entry forward in history and returns it; past the newest entry, returns to the
    /// saved pending line and ends the browsing session. `None` (a no-op) if no recall is in
    /// progress.
    pub fn recall_next(&mut self) -> Option<&str> {
        let i = self.cursor?;
        if i + 1 >= self.lines.len() {
            self.cursor = None;
            return Some(&self.pending);
        }
        self.cursor = Some(i + 1);
        Some(&self.lines[i + 1])
    }

    /// Ends any in-progress browsing session -- called on a fresh prompt and after a line
    /// finishes, so a stale recall position never leaks into the next line.
    pub fn reset_recall(&mut self) {
        self.cursor = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(lines: &[&str]) -> History {
        let mut h = History::new();
        for l in lines {
            h.record(l);
        }
        h
    }

    #[test]
    fn recall_with_no_history_does_nothing() {
        let mut h = History::new();
        assert_eq!(h.recall_prev("typing"), None);
        assert_eq!(h.recall_next(), None);
    }

    #[test]
    fn up_walks_back_and_stops_at_the_oldest() {
        let mut h = with(&["a", "b", "c"]);
        assert_eq!(h.recall_prev(""), Some("c"));
        assert_eq!(h.recall_prev(""), Some("b"));
        assert_eq!(h.recall_prev(""), Some("a"));
        assert_eq!(h.recall_prev(""), None);
    }

    #[test]
    fn down_walks_forward_then_restores_the_in_progress_line() {
        let mut h = with(&["a", "b"]);
        assert_eq!(h.recall_prev("half-typed"), Some("b"));
        assert_eq!(h.recall_prev("ignored"), Some("a"));
        assert_eq!(h.recall_next(), Some("b"));
        assert_eq!(h.recall_next(), Some("half-typed"));
        // The session is over: Down again is a no-op.
        assert_eq!(h.recall_next(), None);
    }

    #[test]
    fn only_consecutive_duplicates_and_empty_lines_are_skipped() {
        let h = with(&["a", "a", "", "b", "a"]);
        assert_eq!(h.lines, ["a", "b", "a"]);
    }

    #[test]
    fn the_oldest_entry_is_evicted_past_capacity() {
        let mut h = History::new();
        for i in 0..CAPACITY + 3 {
            h.record(&alloc::format!("cmd{i}"));
        }
        assert_eq!(h.lines.len(), CAPACITY);
        assert_eq!(h.lines[0], "cmd3");
    }

    #[test]
    fn reset_recall_forgets_the_browsing_position() {
        let mut h = with(&["a", "b"]);
        h.recall_prev("");
        h.reset_recall();
        assert_eq!(h.recall_next(), None);
        assert_eq!(h.recall_prev(""), Some("b"));
    }
}
