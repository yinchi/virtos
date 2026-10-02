//! What `wc` counts in one input: lines, words, characters, bytes and the longest line. Pure, so it
//! is tested on the host.

/// Lines, words, characters, bytes and longest line of one input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub lines: usize,
    pub words: usize,
    pub chars: usize,
    pub bytes: usize,
    pub max_line: usize,
}

impl Counts {
    /// Lines are counted by `\n` bytes -- not `head`/`tail`'s notion of "lines", which also counts
    /// a final unterminated one; a real `wc` doesn't. Words are maximal runs of non-whitespace
    /// bytes (blank, tab, newline, CR, VT, FF). A character is a byte that is not a UTF-8
    /// continuation byte (`10xxxxxx`), and the longest line is measured in characters too (not
    /// display columns: a wide glyph counts once). A final line with no newline still counts for
    /// the longest.
    pub fn of(data: &[u8]) -> Self {
        let mut c = Counts {
            bytes: data.len(),
            ..Counts::default()
        };
        let mut in_word = false;
        let mut current = 0;
        for &b in data {
            let continuation = b & 0xC0 == 0x80;
            if !continuation {
                c.chars += 1;
            }
            if b == b'\n' {
                c.lines += 1;
                c.max_line = c.max_line.max(current);
                current = 0;
            } else if !continuation {
                current += 1;
            }
            if b.is_ascii_whitespace() || b == 0x0b {
                in_word = false;
            } else if !in_word {
                in_word = true;
                c.words += 1;
            }
        }
        c.max_line = c.max_line.max(current);
        c
    }

    /// Adds `other` into a running total: the counts sum, the longest line is the longer.
    pub fn add(&mut self, other: &Counts) {
        self.lines += other.lines;
        self.words += other.words;
        self.chars += other.chars;
        self.bytes += other.bytes;
        self.max_line = self.max_line.max(other.max_line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(lines: usize, words: usize, chars: usize, bytes: usize, max_line: usize) -> Counts {
        Counts {
            lines,
            words,
            chars,
            bytes,
            max_line,
        }
    }

    #[test]
    fn empty_input_is_all_zero() {
        assert_eq!(Counts::of(b""), c(0, 0, 0, 0, 0));
    }

    #[test]
    fn lines_are_newlines_not_lines() {
        assert_eq!(Counts::of(b"a\nb\n").lines, 2);
        assert_eq!(Counts::of(b"a\nb").lines, 1); // an unterminated last line is not counted
        assert_eq!(Counts::of(b"\n").lines, 1);
    }

    #[test]
    fn words_are_runs_of_non_whitespace() {
        assert_eq!(Counts::of(b"one two  three\n").words, 3);
        assert_eq!(Counts::of(b"  lead and trail  ").words, 3);
        assert_eq!(Counts::of(b"a\tb\nc\rd\x0be\x0cf").words, 6); // every whitespace byte splits
        assert_eq!(Counts::of(b"   \n\n").words, 0);
        assert_eq!(Counts::of(b"a-b_c.d").words, 1);
    }

    #[test]
    fn bytes_and_characters_differ_for_utf8() {
        assert_eq!(Counts::of("h\u{e9}llo".as_bytes()), c(0, 1, 5, 6, 5));
        assert_eq!(
            Counts::of("\u{65e5}\u{672c}\u{8a9e}".as_bytes()),
            c(0, 1, 3, 9, 3)
        );
    }

    #[test]
    fn the_longest_line_is_in_characters() {
        assert_eq!(Counts::of(b"ab\nabcd\nabc\n").max_line, 4);
        assert_eq!(Counts::of(b"ab\nabcd\nabc").max_line, 4); // the last, unterminated, also counts
        assert_eq!(Counts::of(b"ab\nabcdef").max_line, 6);
        assert_eq!(Counts::of("\u{65e5}\u{672c}\nab\n".as_bytes()).max_line, 2); // not 6 bytes
    }

    #[test]
    fn a_total_sums_and_takes_the_longer_line() {
        let mut total = Counts::of(b"a b\n");
        total.add(&Counts::of(b"longer line here\n"));
        assert_eq!(total, c(2, 5, 21, 21, 16));
    }
}
