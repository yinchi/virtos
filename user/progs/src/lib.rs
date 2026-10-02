//! Helpers shared by more than one of this crate's `src/bin/*.rs` programs. `cat` doesn't use
//! this -- it deliberately streams a file through a fixed buffer instead of reading it whole, to
//! actually demonstrate that pattern -- but `head`/`tail`/`wc`/`hexdump` all need a file's entire
//! content before they can do their job, so the "open, read to a `Vec`, close" sequence lives here
//! once instead of four times.
#![no_std]

extern crate alloc;

pub mod cli;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
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

/// Joins `parent` and `child` with a `/`, trimming one trailing `/` off `parent` first so it
/// never ends up doubled. Used by every directory-walking recursion (`chmod -R`, `rm -r`,
/// `cp -r`) to build a child's full path from a `getdents` record's own (relative) name.
pub fn join(parent: &str, child: &str) -> String {
    format!("{}/{}", parent.trim_end_matches('/'), child)
}

/// Writes all of `bytes` to `fd`, retrying after a short write; gives up silently on an error
/// (there is nowhere left to report a failed write to stdout/stderr).
pub fn write_all(fd: usize, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        let n = userlib::write(fd, bytes);
        if n <= 0 {
            return;
        }
        bytes = &bytes[n as usize..];
    }
}

/// A buffered stdout, for the filters that produce many small pieces (a line at a time, a number
/// at a time): every `put`/`write!` appends to a buffer that is sent in one `write` per
/// `Out::CAPACITY` bytes instead of one per piece. Flushed when it is dropped, so an early
/// `return` loses nothing; `flush` sends it sooner (before reading stdin interactively, say).
pub struct Out {
    buf: Vec<u8>,
}

impl Out {
    pub const CAPACITY: usize = 4096;

    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(Self::CAPACITY),
        }
    }

    pub fn put(&mut self, bytes: &[u8]) {
        if self.buf.len() + bytes.len() > Self::CAPACITY {
            self.flush();
        }
        if bytes.len() > Self::CAPACITY {
            write_all(1, bytes);
        } else {
            self.buf.extend_from_slice(bytes);
        }
    }

    pub fn flush(&mut self) {
        write_all(1, &self.buf);
        self.buf.clear();
    }
}

impl Default for Out {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Write for Out {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.put(s.as_bytes());
        Ok(())
    }
}

impl Drop for Out {
    fn drop(&mut self) {
        self.flush();
    }
}
