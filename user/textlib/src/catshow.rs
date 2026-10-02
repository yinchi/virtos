//! `cat`'s display flags -- `-n` number lines, `-E` end each with `$`, `-T` show tabs as `^I`, `-s`
//! squeeze runs of blank lines -- as a state machine that can be fed an input in pieces: numbers
//! and the blank-line memory carry across chunks, and across files. Pure `no_std` + `alloc`, so it
//! is tested on the host.

use alloc::format;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    pub number: bool,
    pub dollar: bool,
    pub tabs: bool,
    pub squeeze: bool,
}

impl Flags {
    /// Whether any flag is set; with none, `cat` is a plain byte copy that never touches the
    /// bytes (so a binary file is safe to `cat`).
    pub fn any(self) -> bool {
        self.number || self.dollar || self.tabs || self.squeeze
    }
}

/// Where the output has got to, across chunks and across files.
#[derive(Debug, Default)]
pub struct State {
    /// The number of the last line numbered.
    line_no: usize,
    /// Whether the line being written has started (its number, if any, is out).
    in_line: bool,
    /// Whether the last line written was blank, for `-s`.
    prev_blank: bool,
}

impl State {
    fn number(&mut self, out: &mut Vec<u8>) {
        self.line_no += 1;
        out.extend_from_slice(format!("{:>6}\t", self.line_no).as_bytes());
    }

    /// Appends `chunk`, as `flags` say it is shown, to `out`.
    pub fn show(&mut self, flags: Flags, chunk: &[u8], out: &mut Vec<u8>) {
        for &b in chunk {
            if b == b'\n' {
                if !self.in_line {
                    // A blank line: dropped, if it repeats one and `-s` says so.
                    if flags.squeeze && self.prev_blank {
                        continue;
                    }
                    if flags.number {
                        self.number(out);
                    }
                    self.prev_blank = true;
                } else {
                    self.prev_blank = false;
                }
                if flags.dollar {
                    out.push(b'$');
                }
                out.push(b'\n');
                self.in_line = false;
            } else {
                if !self.in_line {
                    if flags.number {
                        self.number(out);
                    }
                    self.in_line = true;
                }
                if b == b'\t' && flags.tabs {
                    out.extend_from_slice(b"^I");
                } else {
                    out.push(b);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

    const N: Flags = Flags {
        number: true,
        dollar: false,
        tabs: false,
        squeeze: false,
    };
    const E: Flags = Flags {
        number: false,
        dollar: true,
        tabs: false,
        squeeze: false,
    };
    const T: Flags = Flags {
        number: false,
        dollar: false,
        tabs: true,
        squeeze: false,
    };
    const S: Flags = Flags {
        number: false,
        dollar: false,
        tabs: false,
        squeeze: true,
    };
    const NS: Flags = Flags {
        number: true,
        dollar: false,
        tabs: false,
        squeeze: true,
    };

    fn show(flags: Flags, chunks: &[&[u8]]) -> String {
        let mut state = State::default();
        let mut out = Vec::new();
        for chunk in chunks {
            state.show(flags, chunk, &mut out);
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn no_flags_means_no_work_to_do() {
        assert!(!Flags::default().any());
        assert!(N.any() && E.any() && T.any() && S.any());
    }

    #[test]
    fn numbering_right_aligns_in_six_columns_and_a_tab() {
        assert_eq!(show(N, &[b"a\nb\n"]), "     1\ta\n     2\tb\n");
        assert_eq!(show(N, &[b"x"]), "     1\tx"); // an unterminated last line is numbered too
    }

    #[test]
    fn blank_lines_are_numbered_unless_squeezed() {
        assert_eq!(
            show(N, &[b"a\n\n\nb\n"]),
            "     1\ta\n     2\t\n     3\t\n     4\tb\n"
        );
        assert_eq!(
            show(NS, &[b"a\n\n\nb\n"]),
            "     1\ta\n     2\t\n     3\tb\n"
        );
    }

    #[test]
    fn dollar_marks_every_line_end_but_not_an_unterminated_one() {
        assert_eq!(show(E, &[b"a\n\nb"]), "a$\n$\nb");
    }

    #[test]
    fn tabs_become_caret_i() {
        assert_eq!(show(T, &[b"a\tb\t\n"]), "a^Ib^I\n");
    }

    #[test]
    fn squeeze_keeps_one_blank_line_of_each_run() {
        assert_eq!(show(S, &[b"a\n\n\n\nb\n\nc\n"]), "a\n\nb\n\nc\n");
        assert_eq!(show(S, &[b"\n\n\na\n"]), "\na\n"); // a leading run too
    }

    #[test]
    fn the_state_carries_across_chunk_boundaries() {
        let whole = show(NS, &[b"one\n\n\ntwo\nthree\n"]);
        // Every way of cutting the same input in two gives the same output -- inside a line,
        // between the newlines of a blank run, and right after a newline.
        let input = b"one\n\n\ntwo\nthree\n";
        for cut in 0..=input.len() {
            assert_eq!(
                show(NS, &[&input[..cut], &input[cut..]]),
                whole,
                "cut at {cut}"
            );
        }
        let one_byte_at_a_time: Vec<&[u8]> = input.chunks(1).collect();
        assert_eq!(show(NS, &one_byte_at_a_time), whole);
    }

    #[test]
    fn numbers_and_blank_memory_carry_across_files() {
        // The second "file" starts where the first stopped: its first line is number 3, and a
        // blank line right after the first file's blank is still squeezed.
        assert_eq!(
            show(NS, &[b"a\n\n", b"\nb\n"]),
            "     1\ta\n     2\t\n     3\tb\n"
        );
        // An unterminated last line runs on into the next file's first line, as in GNU cat.
        assert_eq!(show(N, &[b"yy", b"a\nb"]), "     1\tyya\n     2\tb");
    }

    #[test]
    fn flags_combine() {
        let all = Flags {
            number: true,
            dollar: true,
            tabs: true,
            squeeze: true,
        };
        assert_eq!(
            show(all, &[b"a\tb\n\n\nc\n"]),
            "     1\ta^Ib$\n     2\t$\n     3\tc$\n"
        );
    }
}
