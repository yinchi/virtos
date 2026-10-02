//! `tr`'s character sets and the translation they describe. A set is written as literal characters, ranges (`a-z`), escapes
//! (`\n`, `\t`, `\r`, `\\`, `\0`..`\377` octal, and a backslash before anything else is that character), and the character
//! classes `[:alpha:]`, `[:alnum:]`, `[:blank:]`, `[:digit:]`, `[:lower:]`, `[:punct:]`, `[:space:]`, `[:upper:]`. Sets are
//! **bytes**, and only ASCII characters can be named in them; every other byte of the input passes through untouched, which
//! keeps UTF-8 text intact. Pure `no_std` + `alloc`, so it is tested on the host.

use alloc::vec::Vec;

/// Why a set or a combination of sets was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrError {
    /// A character outside ASCII in a set.
    NonAscii,
    /// `z-a`.
    ReverseRange,
    /// `[:nosuch:]`.
    BadClass,
    /// A translation to an empty second set.
    EmptySecond,
}

fn class(name: &str) -> Option<fn(u8) -> bool> {
    Some(match name {
        "alpha" => |b| b.is_ascii_alphabetic(),
        "alnum" => |b| b.is_ascii_alphanumeric(),
        "blank" => |b| b == b' ' || b == b'\t',
        "digit" => |b| b.is_ascii_digit(),
        "lower" => |b| b.is_ascii_lowercase(),
        "punct" => |b| b.is_ascii_punctuation(),
        "space" => |b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c),
        "upper" => |b| b.is_ascii_uppercase(),
        _ => return None,
    })
}

/// One element of a set, before ranges are formed: a byte, and whether it was written literally (a `-` written plainly is a
/// range operator; an escaped one is not).
fn element(bytes: &[u8], i: &mut usize) -> Result<(u8, bool), TrError> {
    let b = bytes[*i];
    if b >= 0x80 {
        return Err(TrError::NonAscii);
    }
    *i += 1;
    if b != b'\\' || *i >= bytes.len() {
        return Ok((b, true));
    }
    let e = bytes[*i];
    if e >= 0x80 {
        return Err(TrError::NonAscii);
    }
    *i += 1;
    Ok((
        match e {
            b'n' => b'\n',
            b't' => b'\t',
            b'r' => b'\r',
            b'0'..=b'7' => {
                let mut value = u32::from(e - b'0');
                let mut digits = 1;
                while digits < 3 && *i < bytes.len() && (b'0'..=b'7').contains(&bytes[*i]) {
                    value = value * 8 + u32::from(bytes[*i] - b'0');
                    *i += 1;
                    digits += 1;
                }
                value as u8
            }
            other => other,
        },
        false,
    ))
}

/// The bytes `spec` names, in order.
pub fn expand(spec: &str) -> Result<Vec<u8>, TrError> {
    let bytes = spec.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        // `[:name:]`
        if bytes[i] == b'['
            && bytes.get(i + 1) == Some(&b':')
            && let Some(close) = spec[i + 2..].find(":]")
        {
            let name = &spec[i + 2..i + 2 + close];
            let test = class(name).ok_or(TrError::BadClass)?;
            out.extend((0..=127u8).filter(|&b| test(b)));
            i += 2 + close + 2;
            continue;
        }
        let (from, _) = element(bytes, &mut i)?;
        // A range `a-z`: a plain `-` with something after it.
        if bytes.get(i) == Some(&b'-') && i + 1 < bytes.len() {
            i += 1;
            let (to, _) = element(bytes, &mut i)?;
            if to < from {
                return Err(TrError::ReverseRange);
            }
            out.extend(from..=to);
        } else {
            out.push(from);
        }
    }
    Ok(out)
}

/// A compiled `tr`: what to delete, what to map to what, and what to squeeze.
pub struct Tr {
    delete: [bool; 256],
    map: [u8; 256],
    squeeze: [bool; 256],
}

impl Tr {
    /// `tr [-d] [-s] SET1 [SET2]`: with `delete`, SET1 is removed (and SET2, if given, is squeezed afterwards); otherwise SET1
    /// is translated to SET2 -- a SET2 shorter than SET1 is padded with its last byte -- and with `squeeze` runs of a byte
    /// from SET2 (or from SET1, if there is no SET2) become one.
    pub fn new(
        set1: &[u8],
        set2: Option<&[u8]>,
        delete: bool,
        squeeze: bool,
    ) -> Result<Tr, TrError> {
        let mut tr = Tr {
            delete: [false; 256],
            map: core::array::from_fn(|i| i as u8),
            squeeze: [false; 256],
        };
        if delete {
            for &b in set1 {
                tr.delete[usize::from(b)] = true;
            }
            if squeeze && let Some(set2) = set2 {
                for &b in set2 {
                    tr.squeeze[usize::from(b)] = true;
                }
            }
            return Ok(tr);
        }
        match set2 {
            Some(set2) => {
                let Some(&last) = set2.last() else {
                    return Err(TrError::EmptySecond);
                };
                for (i, &b) in set1.iter().enumerate() {
                    tr.map[usize::from(b)] = set2.get(i).copied().unwrap_or(last);
                }
                if squeeze {
                    for &b in set2 {
                        tr.squeeze[usize::from(b)] = true;
                    }
                }
            }
            None => {
                if squeeze {
                    for &b in set1 {
                        tr.squeeze[usize::from(b)] = true;
                    }
                }
            }
        }
        Ok(tr)
    }

    /// `input` after deleting, mapping and squeezing.
    pub fn apply(&self, input: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(input.len());
        for &b in input {
            if self.delete[usize::from(b)] {
                continue;
            }
            let mapped = self.map[usize::from(b)];
            if self.squeeze[usize::from(mapped)] && out.last() == Some(&mapped) {
                continue;
            }
            out.push(mapped);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

    fn e(spec: &str) -> String {
        String::from_utf8(expand(spec).unwrap()).unwrap()
    }

    fn run(set1: &str, set2: Option<&str>, delete: bool, squeeze: bool, input: &str) -> String {
        let (a, b) = (expand(set1).unwrap(), set2.map(|s| expand(s).unwrap()));
        String::from_utf8(
            Tr::new(&a, b.as_deref(), delete, squeeze)
                .unwrap()
                .apply(input.as_bytes()),
        )
        .unwrap()
    }

    #[test]
    fn literals_and_ranges() {
        assert_eq!(e("abc"), "abc");
        assert_eq!(e("a-e"), "abcde");
        assert_eq!(e("a-cx-z"), "abcxyz");
        assert_eq!(e("0-9"), "0123456789");
        assert_eq!(e("a-"), "a-"); // a trailing dash is a dash
        assert_eq!(e("-a"), "-a"); // and so is a leading one
    }

    #[test]
    fn escapes() {
        assert_eq!(e("\\n"), "\n");
        assert_eq!(e("a\\tb"), "a\tb");
        assert_eq!(e("\\\\"), "\\");
        assert_eq!(e("\\101"), "A"); // octal
        assert_eq!(e("\\0"), "\0");
        assert_eq!(e("\\-"), "-");
        assert_eq!(e("a\\-z"), "a-z"); // an escaped dash is not a range
        assert_eq!(e("\\x"), "x");
        assert_eq!(e("ab\\"), "ab\\"); // a trailing backslash is itself
    }

    #[test]
    fn classes() {
        assert_eq!(e("[:digit:]"), "0123456789");
        assert_eq!(e("[:upper:]"), "ABCDEFGHIJKLMNOPQRSTUVWXYZ");
        assert_eq!(e("[:lower:]x"), "abcdefghijklmnopqrstuvwxyzx");
        assert_eq!(e("[:blank:]"), "\t ");
        assert_eq!(e("[:space:]").len(), 6);
        assert_eq!(e("[:alpha:]").len(), 52);
        assert_eq!(e("[:alnum:]").len(), 62);
        assert_eq!(expand("[:nosuch:]"), Err(TrError::BadClass));
        assert_eq!(e("[:"), "[:"); // not a class
    }

    #[test]
    fn refusals() {
        assert_eq!(expand("z-a"), Err(TrError::ReverseRange));
        assert_eq!(expand("é"), Err(TrError::NonAscii));
        assert_eq!(expand("a-é"), Err(TrError::NonAscii));
        assert_eq!(
            Tr::new(b"a", Some(b""), false, false).err(),
            Some(TrError::EmptySecond)
        );
    }

    #[test]
    fn translate() {
        assert_eq!(
            run("abc", Some("xyz"), false, false, "aabbcc d"),
            "xxyyzz d"
        );
        assert_eq!(
            run("a-z", Some("A-Z"), false, false, "Hello, world"),
            "HELLO, WORLD"
        );
        assert_eq!(
            run("[:upper:]", Some("[:lower:]"), false, false, "Hello"),
            "hello"
        );
    }

    #[test]
    fn a_short_second_set_is_padded_with_its_last_byte() {
        assert_eq!(run("abcd", Some("xy"), false, false, "abcd"), "xyyy");
        assert_eq!(run("a-z", Some("*"), false, false, "ab cd"), "** **");
    }

    #[test]
    fn delete() {
        assert_eq!(run("aeiou", None, true, false, "education"), "dctn");
        assert_eq!(run("[:digit:]", None, true, false, "a1b22c"), "abc");
        assert_eq!(run("\\n", None, true, false, "a\nb\n"), "ab");
    }

    #[test]
    fn squeeze() {
        assert_eq!(run(" ", None, false, true, "a   b  c"), "a b c");
        assert_eq!(run("a-c", Some("x"), false, true, "aabbcc"), "x"); // translated, then squeezed as the second set says
        assert_eq!(run("a", Some("b"), false, true, "aaXaa"), "bXb");
        assert_eq!(run("a-z", None, false, true, "aabbXXcc"), "abXXc"); // only the set's own bytes
    }

    #[test]
    fn delete_then_squeeze() {
        assert_eq!(run("0-9", Some(" "), true, true, "a12  34  b"), "a b");
    }

    #[test]
    fn bytes_outside_ascii_pass_through() {
        assert_eq!(run("a", Some("b"), false, false, "aé日a"), "bé日b");
    }
}
