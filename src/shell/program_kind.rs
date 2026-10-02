//! What a file that the shell has been asked to run actually is: an ELF program, a script, or
//! something that is neither. Pure, so it is tested on the host; `launch.rs` reads the file and
//! acts on the answer.
//!
//! Like a real shell's `execve` fallback: a file that starts with the ELF magic is a program
//! whatever else is in it. Anything else is what `execve` would refuse with `ENOEXEC`, and bash
//! then decides by looking at the file's first few bytes: a **NUL byte** in them means binary data
//! (`cannot execute binary file`), and anything else is taken for a script and run by the shell
//! itself. There are no `#!` interpreter lines: the shell is the only interpreter there is, so a
//! script is always run by it, and a first line starting with `#!` is just a comment.

/// The four bytes every ELF file starts with.
const ELF_MAGIC: &[u8] = b"\x7fELF";

/// How many leading bytes decide whether a non-ELF file is binary or looks like a script -- bash's
/// own heuristic (it probes the first 80; any NUL in the prefix means binary).
pub const SCRIPT_PROBE_LEN: usize = 128;

/// What a file turned out to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Starts with the ELF magic: `exec::process::run_program` loads it (and validates it).
    Elf,
    /// Text the shell can run as a script.
    Script,
    /// Not ELF, and a NUL in the first `SCRIPT_PROBE_LEN` bytes: binary data, not a script.
    Binary,
    /// No NUL in the probe, but not valid UTF-8 as a whole -- the shell reads scripts as UTF-8.
    NotUtf8,
}

/// Classifies the whole content of a file.
pub fn classify(bytes: &[u8]) -> Kind {
    if bytes.starts_with(ELF_MAGIC) {
        return Kind::Elf;
    }
    if bytes[..bytes.len().min(SCRIPT_PROBE_LEN)].contains(&0) {
        return Kind::Binary;
    }
    if core::str::from_utf8(bytes).is_err() {
        return Kind::NotUtf8;
    }
    Kind::Script
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_elf_magic_decides_whatever_follows() {
        assert_eq!(classify(b"\x7fELF"), Kind::Elf);
        assert_eq!(classify(b"\x7fELF\x02\x01\x01\0\0\0"), Kind::Elf);
        assert_eq!(classify(b"\x7fELF not really"), Kind::Elf); // `run_program` validates it
        assert_eq!(classify(b"\x7fEL"), Kind::Script); // too short to be the magic
    }

    #[test]
    fn plain_text_is_a_script() {
        assert_eq!(classify(b"echo hi\n"), Kind::Script);
        assert_eq!(classify(b"#!/bin/sh\necho hi\n"), Kind::Script); // `#!` is just a comment
        assert_eq!(classify(b""), Kind::Script); // an empty file runs as an empty script
        assert_eq!(classify("echo héllo 日本\n".as_bytes()), Kind::Script);
    }

    #[test]
    fn a_nul_in_the_probe_means_binary() {
        assert_eq!(classify(b"echo\0hi"), Kind::Binary);
        assert_eq!(classify(b"\0"), Kind::Binary);
        let mut late = alloc::vec![b'a'; SCRIPT_PROBE_LEN - 1];
        late.push(0);
        assert_eq!(classify(&late), Kind::Binary); // the last probed byte
    }

    #[test]
    fn a_nul_past_the_probe_is_not_looked_for() {
        // Bash only looks at the start; a NUL later makes the text no less a script as far as
        // this check goes (it is still valid UTF-8).
        let mut text = alloc::vec![b'a'; SCRIPT_PROBE_LEN];
        text.push(0);
        assert_eq!(classify(&text), Kind::Script);
    }

    #[test]
    fn text_that_is_not_utf8_is_refused_separately() {
        assert_eq!(classify(b"echo \xff\xfe\n"), Kind::NotUtf8);
        assert_eq!(classify(b"\xc3\x28"), Kind::NotUtf8); // an invalid two-byte sequence
    }
}
