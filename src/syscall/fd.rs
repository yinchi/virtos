//! File descriptor dispatch for `open`/`close`/`read`/`write`. One fd table, not one per program:
//! at most one program is ever resident (see `arch/mmu.rs`'s reasoning), so `reset_for_launch`
//! simply refills it with the three standard entries before each launch, and `end_launch` empties
//! it again once that program exits or faults. `0`/`1`/`2` are always `Keyboard`/`Console`/
//! `Console` right after a reset; everything above them is handed out by `open` and returned by
//! `close`. No redirection yet (`>`/`<`, `2>&1`), so unlike the mature reference's reference-
//! counted `FileRef` (several fds sharing one open file), each fd here exclusively owns its own.

use core::fmt::Write;

use hadris_fat::FatVolumeReadExt;
use hadris_fat::sync::read::FileReader;

use crate::arch::mmu::{self, USER_BASE, USER_STACK_TOP};
use crate::console;
use crate::drivers::virtio::gpu;
use crate::fs;
use crate::fs::blkio::BlkIo;
use crate::keyboard::queue;
use abi::O_RDONLY;

/// This project's own error convention so far -- a negative value, not yet the real negated-errno
/// convention `plans/USERSPACE_PLAN.md` settles on for the full syscall set (no `errno` constants
/// exist yet to return one of).
const EBADF: isize = -1;

/// How many fds a program may have open at once, the three standard ones included -- `open` fails
/// once every slot above them already holds a file. Arbitrary and modest; easy to raise later, no
/// compatibility promise attached to the exact number.
const MAX_FDS: usize = 3 + 8;

enum FileDescriptor {
    Keyboard,
    Console,
    /// An open file, read-only -- no write support yet (`abi::O_RDONLY`'s own doc comment).
    File(FileReader<'static, BlkIo>),
}

/// SAFETY (every access, via `table`): single core, and every syscall runs with IRQs masked
/// (`vectors.s`'s trap entry), so nothing else can touch the table while a call is in progress.
static mut FD_TABLE: [Option<FileDescriptor>; MAX_FDS] = [const { None }; MAX_FDS];

#[allow(clippy::deref_addrof)]
fn table() -> &'static mut [Option<FileDescriptor>; MAX_FDS] {
    // SAFETY: see FD_TABLE.
    unsafe { &mut *(&raw mut FD_TABLE) }
}

/// Prepares the fd table for a new program: drops whatever's still in it (nothing should be --
/// `end_launch` already emptied it when the last program ended) and refills the standard three.
/// Called from `exec::process::run_program`, right before entering EL0.
pub fn reset_for_launch() {
    let table = table();
    *table = [const { None }; MAX_FDS];
    table[0] = Some(FileDescriptor::Keyboard);
    table[1] = Some(FileDescriptor::Console);
    table[2] = Some(FileDescriptor::Console);
}

/// Drops every fd the program that just ended still had open. Cleanup, not a save -- nothing an
/// open file buffers in memory is written out (moot today, read-only; matters once `open` supports
/// writing). Called from `exec::process::enter`, right after the program returns control (`exit`,
/// or a caught fault), on every path.
pub fn end_launch() {
    *table() = [const { None }; MAX_FDS];
}

impl FileDescriptor {
    /// Writes to the console -- both the GPU text console (if `console::set` ran, i.e. a GPU
    /// device exists) and the UART, the same dual visibility every diagnostic print in this
    /// project already has. Writing to `Keyboard` or a `File` is rejected outright, not silently
    /// accepted as a no-op: a mixed-up fd number should be visible as an error, not hidden behind
    /// output that simply never appears.
    fn write(&mut self, bytes: &[u8]) -> isize {
        match self {
            Self::Console => {
                let text = core::str::from_utf8(bytes).unwrap_or("<invalid utf8>");
                if let Some(con) = console::get() {
                    con.puts(text, console::DEFAULT_FG, console::DEFAULT_BG);
                    if let Some(gpu_dev) = gpu::get() {
                        gpu_dev.flush();
                    }
                }
                let mut uart = crate::bare_metal::get_uart();
                let _ = uart.write_str(text);
                bytes.len() as isize
            }
            Self::Keyboard | Self::File(_) => EBADF,
        }
    }

    /// Reads from a file descriptor: the keyboard queue (non-blocking: no process model exists to
    /// block *on*, so an empty queue returns `0` immediately rather than waiting for a key that
    /// might never come), or an open file's next bytes (`0` at end of file). Reading from
    /// `Console` is rejected, same reasoning as writing to `Keyboard`.
    fn read(&mut self, buf: &mut [u8]) -> isize {
        match self {
            Self::Keyboard => {
                let mut n = 0;
                while n < buf.len() {
                    let Some(ch) = queue::pop() else { break };
                    // ASCII-only (see `keyboard`'s own doc comment) -- one byte per character.
                    buf[n] = ch as u8;
                    n += 1;
                }
                n as isize
            }
            Self::Console => EBADF,
            Self::File(reader) => reader.read(buf).map_or(EBADF, |n| n as isize),
        }
    }
}

/// Bounds-checks `ptr`/`len` against the user window before trusting them: the MMU's `USER`
/// attribute bit gates *EL0's* access, not EL1's, so a buggy or malicious pointer aimed at kernel
/// memory is otherwise still readable/writable by the kernel and must be rejected in software.
fn validate(ptr: usize, len: usize) -> bool {
    let Some(end) = ptr.checked_add(len) else {
        return false;
    };
    ptr >= USER_BASE && end <= USER_STACK_TOP
}

pub fn write(fd: usize, ptr: usize, len: usize) -> isize {
    if !validate(ptr, len) {
        return EBADF;
    }
    // The read below touches a page marked USER, which PAN would otherwise forbid.
    let _user = mmu::user_access();
    // SAFETY: validated above to lie entirely within the user window; the guard above lets the
    // kernel read it despite PAN.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
    match table().get_mut(fd).and_then(Option::as_mut) {
        Some(fd) => fd.write(bytes),
        None => EBADF,
    }
}

pub fn read(fd: usize, ptr: usize, len: usize) -> isize {
    if !validate(ptr, len) {
        return EBADF;
    }
    // The write below touches a page marked USER, which PAN would otherwise forbid.
    let _user = mmu::user_access();
    // SAFETY: as in `write`.
    let buf = unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, len) };
    match table().get_mut(fd).and_then(Option::as_mut) {
        Some(fd) => fd.read(buf),
        None => EBADF,
    }
}

/// Opens the file at the user-space path `ptr`/`len` on `system.img` (blk0 -- no path/mount
/// resolution across volumes yet, see `fs::path`'s own doc comment) for reading, returning the
/// lowest free fd number (never one of the standard three) or `-1`. Only `O_RDONLY` is accepted --
/// writing a file is a later step. Also `-1` if the path is missing, names a directory (opening
/// one for `getdents` is a later step too), isn't valid UTF-8, or every fd slot is already in use.
pub fn open(ptr: usize, len: usize, flags: usize) -> isize {
    if flags != O_RDONLY {
        return EBADF;
    }
    if !validate(ptr, len) {
        return EBADF;
    }
    // The read below touches a page marked USER, which PAN would otherwise forbid; kept alive for
    // the rest of this function, since `path` (below) is a zero-copy view straight into it.
    let _user = mmu::user_access();
    // SAFETY: validated above to lie entirely within the user window.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
    let Ok(path) = core::str::from_utf8(bytes) else {
        return EBADF;
    };

    let Some(vol) = fs::volume::get(0) else {
        return EBADF;
    };
    let Ok(entry) = fs::path::resolve(vol, path) else {
        return EBADF;
    };
    if entry.is_directory() {
        return EBADF;
    }
    let Ok(reader) = vol.read_file(&entry) else {
        return EBADF;
    };

    let table = table();
    let Some(fd) = table
        .iter()
        .skip(3)
        .position(Option::is_none)
        .map(|i| i + 3)
    else {
        return EBADF;
    };
    table[fd] = Some(FileDescriptor::File(reader));
    fd as isize
}

/// Closes `fd`: the slot is emptied, dropping whatever it held. `-1` for an already-closed or
/// never-opened fd; closing a standard fd (0/1/2) is allowed, same as a real `close` -- it's just
/// gone for the rest of this program's run.
pub fn close(fd: usize) -> isize {
    match table().get_mut(fd).and_then(Option::take) {
        Some(_) => 0,
        None => EBADF,
    }
}

/// Clears the console -- both the graphical one (if a GPU device exists) and, via an ANSI escape
/// sequence (cursor home, erase display), whatever real terminal is watching the UART, the serial
/// counterpart of clearing the console. No fd argument: there's only ever one console, so there's
/// nothing to look up (unlike `read`/`write`, which are addressing one of several fds).
pub fn clear() -> isize {
    if let Some(con) = console::get() {
        con.clear(console::DEFAULT_BG);
        if let Some(gpu_dev) = gpu::get() {
            gpu_dev.flush();
        }
    }
    let mut uart = crate::bare_metal::get_uart();
    let _ = uart.write_str("\x1b[H\x1b[2J");
    0
}
