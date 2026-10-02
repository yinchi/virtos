//! `echo -e`'s backslash escapes: `\\ \a \b \c \e \f \n \r \t \v`, `\0NNN` (up to three octal
//! digits after the 0) and `\xHH` (up to two hex digits). An unknown escape, a `\x` with no digits
//! and a backslash at the very end are left as written. Pure `no_std` + `alloc`, so it is tested on
//! the host.

use alloc::vec::Vec;

/// The value of up to `max` digits of `radix` at the start of `bytes` (truncated to a byte, as
/// `echo -e '\0777'` is), and how many digits were used; `None` if there is not even one.
fn number(bytes: &[u8], radix: u32, max: usize) -> Option<(u8, usize)> {
    let mut value = 0u32;
    let mut used = 0;
    while used < max && used < bytes.len() {
        match (bytes[used] as char).to_digit(radix) {
            Some(d) => value = value * radix + d,
            None => break,
        }
        used += 1;
    }
    (used > 0).then_some((value as u8, used))
}

/// Appends `arg` to `out` with its escapes interpreted. `true` if `\c` was met: nothing more is
/// to be printed, not this argument's remainder, not later arguments, not the newline.
pub fn interpret(arg: &str, out: &mut Vec<u8>) -> bool {
    let bytes = arg.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' || i + 1 == bytes.len() {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        i += 1;
        let c = bytes[i];
        i += 1;
        match c {
            b'\\' => out.push(b'\\'),
            b'a' => out.push(7),
            b'b' => out.push(8),
            b'c' => return true,
            b'e' => out.push(27),
            b'f' => out.push(12),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'v' => out.push(11),
            b'0' => {
                // `\0NNN`: up to three more octal digits.
                let (value, used) = number(&bytes[i..], 8, 3).unwrap_or((0, 0));
                out.push(value);
                i += used;
            }
            b'x' => match number(&bytes[i..], 16, 2) {
                Some((value, used)) => {
                    out.push(value);
                    i += used;
                }
                None => out.extend_from_slice(b"\\x"), // no digits: as written
            },
            other => {
                out.push(b'\\');
                out.push(other);
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(arg: &str) -> (Vec<u8>, bool) {
        let mut out = Vec::new();
        let stopped = interpret(arg, &mut out);
        (out, stopped)
    }

    fn text(arg: &str) -> Vec<u8> {
        run(arg).0
    }

    #[test]
    fn the_single_character_escapes() {
        assert_eq!(text("\\\\"), b"\\");
        assert_eq!(text("\\a"), [7]);
        assert_eq!(text("\\b"), [8]);
        assert_eq!(text("\\e"), [27]);
        assert_eq!(text("\\f"), [12]);
        assert_eq!(text("\\n"), b"\n");
        assert_eq!(text("\\r"), b"\r");
        assert_eq!(text("\\t"), b"\t");
        assert_eq!(text("\\v"), [11]);
    }

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(text("hello world"), b"hello world");
        assert_eq!(text(""), b"");
        assert_eq!(text("h\u{e9}llo"), "h\u{e9}llo".as_bytes());
    }

    #[test]
    fn backslash_c_stops_everything() {
        assert_eq!(run("ab\\cde"), (b"ab".to_vec(), true));
        assert_eq!(run("\\c"), (Vec::new(), true));
        assert_eq!(run("no stop"), (b"no stop".to_vec(), false));
    }

    #[test]
    fn octal_takes_up_to_three_digits_after_the_zero() {
        assert_eq!(text("\\0101"), b"A");
        assert_eq!(text("\\0101x"), b"Ax"); // a fourth digit is text
        assert_eq!(text("\\01012"), b"A2");
        assert_eq!(text("\\07"), [7]);
        assert_eq!(text("\\0"), [0]); // no digits: a NUL
        assert_eq!(text("\\08"), [0, b'8']); // 8 is not octal
        assert_eq!(text("\\0777"), [0xff]); // 0o777 is cut to a byte
    }

    #[test]
    fn hex_takes_up_to_two_digits() {
        assert_eq!(text("\\x41"), b"A");
        assert_eq!(text("\\x4a"), b"J");
        assert_eq!(text("\\x4A"), b"J");
        assert_eq!(text("\\x414"), b"A4"); // a third digit is text
        assert_eq!(text("\\x9"), [9]);
        assert_eq!(text("\\xff"), [0xff]);
    }

    #[test]
    fn what_is_not_an_escape_stays_as_written() {
        assert_eq!(text("\\x"), b"\\x");
        assert_eq!(text("\\xg"), b"\\xg");
        assert_eq!(text("\\q"), b"\\q");
        assert_eq!(text("\\"), b"\\"); // a backslash at the very end
        assert_eq!(text("a\\"), b"a\\");
        assert_eq!(text("\\1"), b"\\1"); // octal needs the leading 0
    }

    #[test]
    fn escapes_combine() {
        assert_eq!(text("a\\tb\\nc\\\\d"), b"a\tb\nc\\d");
        assert_eq!(text("\\0101\\x42\\n"), b"AB\n");
    }
}
