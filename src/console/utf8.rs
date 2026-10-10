//! A streaming UTF-8 decoder: bytes go in one at a time, `char`s come out, and a character split
//! across two `write` syscalls (`cat` sends 4096-byte chunks, so a multibyte character can straddle
//! a chunk boundary) is completed when its remaining bytes arrive.
//!
//! Invalid input never stops the stream: each maximal invalid sequence becomes one U+FFFD and the
//! decoder resynchronizes on the byte that broke it (the WHATWG Encoding Standard's "UTF-8 decoder"
//! algorithm, so overlong forms, surrogates and values above U+10FFFF are all invalid).
//!
//! Pure `no_std`, no dependency on the rest of the kernel, so it is tested on the host
//! (`just test-host`).

const REPLACEMENT: char = '\u{FFFD}';

/// Decoder state between bytes: how much of the current multibyte character has arrived.
pub struct Utf8Decoder {
    /// The bits gathered so far of the character being decoded.
    code_point: u32,
    /// Continuation bytes still expected; `0` between characters.
    needed: u8,
    /// Continuation bytes received so far for this character.
    seen: u8,
    /// The range the *next* continuation byte must fall in. Narrower than `0x80..=0xBF` only for
    /// the byte right after a lead byte that could otherwise start an overlong form, a surrogate or
    /// a value above U+10FFFF.
    lower: u8,
    upper: u8,
}

impl Utf8Decoder {
    pub const fn new() -> Self {
        Self {
            code_point: 0,
            needed: 0,
            seen: 0,
            lower: 0x80,
            upper: 0xBF,
        }
    }

    /// Whether a multibyte character is partly received.
    pub fn is_pending(&self) -> bool {
        self.needed != 0
    }

    /// Feeds one byte; calls `emit` for each character it completes -- none while a multibyte
    /// character is incomplete, one normally, two when an invalid sequence is cut short by a byte
    /// that is itself a valid start (U+FFFD, then that byte's own character or start).
    pub fn push(&mut self, byte: u8, mut emit: impl FnMut(char)) {
        self.push_inner(byte, &mut emit);
    }

    /// Feeds one byte to the decoder. If a complete character is formed, it is passed to `emit`.
    /// If an invalid sequence is encountered, U+FFFD is emitted and the decoder resynchronizes.
    fn push_inner(&mut self, byte: u8, emit: &mut impl FnMut(char)) {
        if self.needed == 0 {
            match byte {
                0x00..=0x7F => emit(byte as char),
                0xC2..=0xDF => self.start(1, byte & 0x1F),
                0xE0..=0xEF => {
                    if byte == 0xE0 {
                        self.lower = 0xA0; // not an overlong three-byte form
                    }
                    if byte == 0xED {
                        self.upper = 0x9F; // not a surrogate
                    }
                    self.start(2, byte & 0x0F);
                }
                0xF0..=0xF4 => {
                    if byte == 0xF0 {
                        self.lower = 0x90; // not an overlong four-byte form
                    }
                    if byte == 0xF4 {
                        self.upper = 0x8F; // not above U+10FFFF
                    }
                    self.start(3, byte & 0x07);
                }
                // A stray continuation byte, 0xC0/0xC1 (always overlong), or 0xF5..=0xFF.
                _ => emit(REPLACEMENT),
            }
            return;
        }

        if !(self.lower..=self.upper).contains(&byte) {
            // The sequence is cut short: one U+FFFD for what arrived, then this byte is read again
            // as the start of whatever comes next.
            *self = Self::new();
            emit(REPLACEMENT);
            self.push_inner(byte, emit);
            return;
        }

        self.lower = 0x80;
        self.upper = 0xBF;
        self.code_point = (self.code_point << 6) | (byte & 0x3F) as u32;
        self.seen += 1;
        if self.seen == self.needed {
            let c = char::from_u32(self.code_point).unwrap_or(REPLACEMENT);
            *self = Self::new();
            emit(c);
        }
    }

    /// Starts a new multi-byte sequence with the given number of needed continuation bytes and
    /// the initial bits.
    fn start(&mut self, needed: u8, bits: u8) {
        self.needed = needed;
        self.seen = 0;
        self.code_point = bits as u32;
    }

    /// The stream is over: a character still incomplete becomes one U+FFFD. Leaves the decoder
    /// ready for a fresh stream.
    pub fn finish(&mut self, mut emit: impl FnMut(char)) {
        if self.is_pending() {
            emit(REPLACEMENT);
        }
        *self = Self::new();
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;
    use super::*;
    use alloc::string::String;

    fn decode_chunks(chunks: &[&[u8]]) -> String {
        let mut d = Utf8Decoder::new();
        let mut out = String::new();
        for chunk in chunks {
            for &b in *chunk {
                d.push(b, |c| out.push(c));
            }
        }
        d.finish(|c| out.push(c));
        out
    }

    fn decode(bytes: &[u8]) -> String {
        decode_chunks(&[bytes])
    }

    #[test]
    fn ascii_passes_through() {
        assert_eq!(decode(b"hello\n\x00\x7f"), "hello\n\0\x7f");
    }

    #[test]
    fn valid_sequences_of_every_length() {
        // 2, 3 and 4 bytes, plus the extremes of each length.
        for s in [
            "é",
            "€",
            "𐍈",
            "\u{80}",
            "\u{7FF}",
            "\u{800}",
            "\u{FFFF}",
            "\u{10000}",
            "\u{10FFFF}",
            "日本語",
        ] {
            assert_eq!(decode(s.as_bytes()), s);
        }
    }

    #[test]
    fn split_at_every_possible_boundary() {
        let text = "aé€𐍈z日";
        let bytes = text.as_bytes();
        for cut in 0..=bytes.len() {
            assert_eq!(
                decode_chunks(&[&bytes[..cut], &bytes[cut..]]),
                text,
                "cut at {cut}"
            );
        }
        // And one byte at a time.
        let ones: alloc::vec::Vec<&[u8]> = bytes.chunks(1).collect();
        assert_eq!(decode_chunks(&ones), text);
    }

    #[test]
    fn stray_continuation_and_invalid_bytes_are_one_replacement_each() {
        assert_eq!(decode(&[0x80]), "\u{FFFD}");
        assert_eq!(decode(&[0x82, b'a']), "\u{FFFD}a");
        assert_eq!(
            decode(&[0xC0, 0xC1, 0xF5, 0xFF]),
            "\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}"
        );
    }

    #[test]
    fn overlong_surrogate_and_out_of_range_are_invalid() {
        assert_eq!(decode(&[0xC0, 0x80]), "\u{FFFD}\u{FFFD}"); // overlong NUL
        assert_eq!(decode(&[0xE0, 0x80, 0x80]), "\u{FFFD}\u{FFFD}\u{FFFD}"); // overlong three-byte
        assert_eq!(decode(&[0xED, 0xA0, 0x80]), "\u{FFFD}\u{FFFD}\u{FFFD}"); // U+D800
        assert_eq!(
            decode(&[0xF4, 0x90, 0x80, 0x80]),
            "\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}"
        ); // U+110000
    }

    #[test]
    fn truncated_sequence_is_cut_short_by_the_next_valid_start() {
        // A three-byte lead with one continuation, then ASCII: one U+FFFD, and the ASCII survives.
        assert_eq!(decode(&[0xE2, 0x82, b'x']), "\u{FFFD}x");
        // Cut short by another lead byte, which then decodes normally.
        assert_eq!(decode(&[0xE2, 0xC3, 0xA9]), "\u{FFFD}é");
    }

    #[test]
    fn incomplete_at_the_end_is_one_replacement() {
        assert_eq!(decode(&[b'a', 0xE2, 0x82]), "a\u{FFFD}");
        let mut d = Utf8Decoder::new();
        d.push(0xC3, |_| panic!("nothing complete yet"));
        assert!(d.is_pending());
        d.finish(|_| {});
        assert!(!d.is_pending());
    }

    #[test]
    fn every_byte_value_yields_something_and_never_panics() {
        let all: alloc::vec::Vec<u8> = (0..=255).collect();
        let s = decode(&all);
        assert!(s.starts_with("\0\u{1}"));
        assert!(s.chars().count() > 128);
    }
}
