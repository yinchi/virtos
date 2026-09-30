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

/// Reads the file at `path` into a freshly allocated `Vec<u8>`. `None` if it can't be opened.
/// Simplicity over streaming efficiency: fine for this project's small test files, revisit if a
/// program ever needs to handle one too large to comfortably fit the fixed heap at once.
pub fn read_file(path: &str) -> Option<Vec<u8>> {
    let fd = userlib::open(path, O_RDONLY);
    if fd < 0 {
        return None;
    }
    let fd = fd as usize;
    let mut data = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = userlib::read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
    }
    userlib::close(fd);
    Some(data)
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
