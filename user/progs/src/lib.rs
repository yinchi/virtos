//! Helpers shared by more than one of this crate's `src/bin/*.rs` programs. `cat` doesn't use
//! this -- it deliberately streams a file through a fixed buffer instead of reading it whole, to
//! actually demonstrate that pattern -- but `head`/`tail`/`wc`/`hexdump` all need a file's entire
//! content before they can do their job, so the "open, read to a `Vec`, close" sequence lives here
//! once instead of four times.
#![no_std]

extern crate alloc;

pub mod cli;

use alloc::format;
use alloc::vec::Vec;
use userlib::{ExitCode, O_RDONLY};

/// Reads every remaining byte off `fd` into a freshly allocated `Vec<u8>` -- just the read loop,
/// no open/close, so it works equally for an already-open fd (stdin's `0`, or a `-` operand) and
/// for `read_file`'s own "open a path, read it whole" case below.
pub fn read_fd(fd: usize) -> Vec<u8> {
    let mut data = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = userlib::read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
    }
    data
}

/// Reads the file at `path` into a freshly allocated `Vec<u8>`. `None` if it can't be opened.
/// Simplicity over streaming efficiency: fine for this project's small test files, revisit if a
/// program ever needs to handle one too large to comfortably fit the fixed heap at once.
pub fn read_file(path: &str) -> Option<Vec<u8>> {
    let fd = userlib::open(path, O_RDONLY);
    if fd < 0 {
        return None;
    }
    let fd = fd as usize;
    let data = read_fd(fd);
    userlib::close(fd);
    Some(data)
}

/// `read_file(path)`, except a lone `-` means stdin (`read_fd(0)`, GNU's own convention) --
/// shared by every program (`head`/`tail`/`wc`/`hexdump`) that reads a list of path operands
/// whole rather than streaming them the way `cat` does. `None` only for a real path that fails
/// to open; `-` always succeeds (an empty read is a real, if unlikely, possibility, not a
/// failure).
pub fn read_operand(path: &str) -> Option<Vec<u8>> {
    if path == "-" {
        Some(read_fd(0))
    } else {
        read_file(path)
    }
}

/// Prints a `--help` synopsis to stdout: `usage: <usage>`, then one `  <flag>  <description>` line
/// per entry in `flags`, and exits `0` -- every program checks for `--help` before anything else in
/// its own argument handling and returns this directly. `-h` is deliberately not an alias: like GNU
/// coreutils' own `--help`, this is long-form only, since `-h` already means something else in some
/// programs this project will eventually have (`ls -h`, `cp -h`).
pub fn help(usage: &str, flags: &[(&str, &str)]) -> ExitCode {
    userlib::write(1, format!("usage: {usage}\n").as_bytes());
    for (flag, desc) in flags {
        userlib::write(1, format!("  {flag}  {desc}\n").as_bytes());
    }
    ExitCode(0)
}

/// The text after the last `/` in `path` (or all of it if there is none), after trimming one
/// trailing `/` first so a directory operand's own name is returned rather than an empty string.
/// Used by `cp`/`mv`'s directory-destination case (`dst/basename(src)`).
pub fn basename(path: &str) -> &str {
    let path = path.strip_suffix('/').unwrap_or(path);
    match path.rfind('/') {
        Some(i) => &path[i + 1..],
        None => path,
    }
}
