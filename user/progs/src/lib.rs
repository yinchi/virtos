//! Helpers shared by more than one of this crate's `src/bin/*.rs` programs. `cat` doesn't use
//! this -- it deliberately streams a file through a fixed buffer instead of reading it whole, to
//! actually demonstrate that pattern -- but `head`/`tail`/`wc`/`hexdump` all need a file's entire
//! content before they can do their job, so the "open, read to a `Vec`, close" sequence lives here
//! once instead of four times.
#![no_std]

extern crate alloc;

pub mod cli;

use alloc::vec::Vec;
use userlib::O_RDONLY;

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
