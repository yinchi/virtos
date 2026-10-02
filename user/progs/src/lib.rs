//! Helpers shared by more than one of this crate's `src/bin/*.rs` programs. `cat` doesn't use
//! this -- it deliberately streams a file through a fixed buffer instead of reading it whole, to
//! actually demonstrate that pattern -- but `head`/`tail`/`wc`/`hexdump` all need a file's entire
//! content before they can do their job, so the "open, read to a `Vec`, close" sequence lives here
//! once instead of four times.
#![no_std]

extern crate alloc;

pub mod cli;
pub mod copy;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use userlib::{DIRENT_SIZE, DirEnt, ExitCode, O_RDONLY};

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

/// Writes all of `bytes` to `fd`, retrying after a short write. `false` if a write fails.
pub fn write_all_fd(fd: usize, mut bytes: &[u8]) -> bool {
    while !bytes.is_empty() {
        let n = userlib::write(fd, bytes);
        if n <= 0 {
            return false;
        }
        bytes = &bytes[n as usize..];
    }
    true
}

/// Writes all of `bytes` to `fd`, retrying after a short write; gives up silently on an error
/// (there is nowhere left to report a failed write to stdout/stderr).
pub fn write_all(fd: usize, bytes: &[u8]) {
    write_all_fd(fd, bytes);
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

/// One entry of a directory listing: what `getdents` reports.
pub struct Entry {
    pub name: String,
    pub size: u32,
    pub attrs: u8,
}

/// Why a directory could not be listed: the `open` failed, or a `getdents` did (an fd that is not
/// a directory, for one). Kept apart because `ls` words them differently.
pub enum ReadDirError {
    Open,
    Read,
}

/// The whole listing of the directory `path`, in on-disk order, with the fd already closed -- so a
/// caller that recurses holds no descriptor while it does, and depth is not limited by the number
/// of files the kernel lets a program keep open.
pub fn read_dir(path: &str) -> Result<Vec<Entry>, ReadDirError> {
    let fd = userlib::open(path, O_RDONLY);
    if fd < 0 {
        return Err(ReadDirError::Open);
    }
    let fd = fd as usize;
    let mut entries = Vec::new();
    let mut buf = [0u8; DIRENT_SIZE * 15]; // one memory page's worth of records per call
    let result = loop {
        let n = userlib::getdents(fd, &mut buf);
        if n < 0 {
            break Err(ReadDirError::Read);
        }
        if n == 0 {
            break Ok(());
        }
        for raw in buf[..n as usize].as_chunks::<DIRENT_SIZE>().0 {
            if let Some(ent) = DirEnt::parse(raw) {
                entries.push(Entry {
                    name: String::from(ent.name),
                    size: ent.size,
                    attrs: ent.attrs,
                });
            }
        }
    };
    userlib::close(fd);
    result.map(|()| entries)
}

/// The local time zone for the programs that show or read a wall-clock time (`date`, `stat`,
/// `touch`): the IANA name in `$TZ` (`America/Toronto`, `Europe/London`, `UTC`, ...; a leading `:`
/// as POSIX allows is ignored), and UTC when `TZ` is unset, empty, or not a name in the database
/// -- glibc's behaviour too. POSIX rule strings (`EST5EDT,M3.2.0,M11.1.0`) are not understood, so
/// they mean UTC.
///
/// `env` is the program's own environment: it must be started with `userlib::entry_with_env!`,
/// or there is none to look in and this is UTC.
pub fn local_zone(env: userlib::Env) -> chrono_tz::Tz {
    env.filter_map(|entry| entry.strip_prefix("TZ="))
        .next()
        .map(|name| name.strip_prefix(':').unwrap_or(name))
        .and_then(|name| name.parse::<chrono_tz::Tz>().ok())
        .unwrap_or(chrono_tz::Tz::UTC)
}
