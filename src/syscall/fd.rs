//! File descriptor dispatch for `open`/`close`/`read`/`write`/`getdents`, plus the path-only
//! (no fd involved) `mkdir`/`unlink`/`rename`/`getcwd`/`stat`/`chmod`. Every failure is a negated
//! errno (`abi::errno`): each call is written as an inner function returning `Result<isize,
//! isize>`, so `?` carries the errno out, and `ret` flattens it into the one value a syscall
//! returns. Every path a program supplies is
//! resolved against the working directory first (`fs::cwd::absolute`) before it ever reaches
//! `fs::path`. One fd table, not one per program: at most one
//! program is ever resident (see `arch/mmu.rs`'s reasoning), so `reset_for_launch` simply refills
//! it with the three standard entries before each launch, and `end_launch` empties it again once
//! that program exits or faults. `0`/`1`/`2` are always `Keyboard`/`Console`/`Console` right
//! after a reset, unless `bind_stdio` (called right after `reset_for_launch`, from
//! `exec::process::run_program`) overrides one or more with the shell's own redirections --
//! everything above them is handed out by `open` and returned by `close`, same as before.

use core::cell::RefCell;
use core::fmt::Write;

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use hadris_fat::raw::DirEntryAttrFlags;
use hadris_fat::sync::read::FileReader;
use hadris_fat::sync::write::FileWriter;
use hadris_fat::sync::{DirectoryEntry, FatDir, FatVolume};
use hadris_fat::time::{FatDateTime, TimeProvider};
use hadris_fat::{FatVolumeReadExt, FatVolumeWriteExt};

use crate::arch::mmu::{self, USER_BASE, USER_STACK_TOP};
use crate::console;
use crate::console::utf8::Utf8Decoder;
use crate::drivers::virtio::gpu;
use crate::exec::elf;
use crate::fs;
use crate::fs::blkio::BlkIo;
use crate::fs::errno::from_fat;
use crate::fs::fattime::{self, TimeChoice};
use crate::fs::rtc_time::RTC_TIME;
use crate::keyboard::stdin;
use abi::blk::BLKINFO_SIZE;
use abi::errno::{
    EBADF, EBUSY, EEXIST, EFAULT, EINVAL, EIO, EISDIR, EMFILE, ENODEV, ENOTDIR, ENOTTY, ERANGE,
    EXDEV, errmsg,
};
use abi::ioctl::{
    ATTR_DIM, ATTR_INVERSE, CELL_SIZE, CONSOLE_CLEAR, CONSOLE_DRAW, CONSOLE_DRAW_HEADER_SIZE,
    CONSOLE_READ_KEY, Cell, ConsoleDraw, TIOCGWINSZ, WINSIZE_SIZE, WinSize,
};
use abi::keys::KEYEVENT_SIZE;
use abi::{
    ATTR_DIRECTORY, ATTR_EXEC, ATTR_READ_ONLY, DIRENT_SIZE, NAME_MAX, O_APPEND, O_RDONLY, O_WRONLY,
    STAT_SIZE, UTIMES_SIZE,
};

/// How many fds a program may have open at once, the three standard ones included -- `open` fails
/// once every slot above them already holds a file. Arbitrary and modest; easy to raise later, no
/// compatibility promise attached to the exact number.
const MAX_FDS: usize = 3 + 8;

#[derive(Clone)]
enum FileDescriptor {
    Keyboard,
    Console,
    /// An open file, read-only, and the device it is on (`umount` refuses while a file is open
    /// on the volume). Shared like `FileWrite` (see below), for the same reason: a script's
    /// redirected stdin is one open file that every command in the script reads from in turn,
    /// each continuing where the last stopped.
    File(usize, Rc<RefCell<FileReader<'static, BlkIo>>>),
    /// An open file, write-only (`abi::O_WRONLY`) -- created fresh or truncated to empty at open
    /// time (`open`'s own doc comment), or opened positioned at its current end for `>>`
    /// (`open_for_write`). `Rc<RefCell<_>>`, not owned outright, so `bind_stdio`'s `Dup` (`>&`)
    /// can give a second fd the *same* writer (`hadris_fat`'s `FileWriter` allows only one per
    /// directory entry, so two independently-opened writers against the same path would be
    /// rejected outright) rather than a plain index onto the first -- that would leave the second
    /// fd broken the moment the first is closed, which a real `dup`'d fd never is: each of the
    /// two clones owns the writer independently, so either can be closed first, in any order,
    /// with the other still fully working. `close` finishes it -- via `Rc::try_unwrap`, only once
    /// every clone (both closed, or `end_launch` reclaiming any left open at program exit) has let
    /// go, not on the first -- committing its final size to disk (`FileWriter::finish`'s own
    /// contract); a program that never calls `close` on it, or that faults first, still gets this
    /// from `end_launch`, so a redirected stdout/stderr is never left with the wrong size for the
    /// next reader (a pipeline stage, or a later `cat`) to find.
    FileWrite(usize, Rc<RefCell<FileWriter<'static, BlkIo>>>),
    /// An open directory: `entries` captured whole when it was opened (unlike a file, FAT gives no
    /// way to resume a directory walk at an arbitrary byte offset) -- each entry's name, size, and
    /// attribute byte, so `getdents` can hand a child's type back to a caller (`chmod -R`'s own
    /// recursion, eventually `ls -l`/`-F`) without it needing a separate `stat` call per entry.
    /// `next` is how many `getdents` has already returned.
    Dir {
        dev: usize,
        entries: Vec<(String, u32, u8)>,
        next: usize,
    },
}

/// Snapshots `dir`'s entries -- name, size, attribute byte -- for `getdents`, skipping `.`/`..`
/// (real subdirectory entries, unlike the root, which has neither) and any volume-label entry --
/// none of the three is something a program should ever see listed.
fn list_dir(dir: &FatDir<'static, BlkIo>) -> Vec<(String, u32, u8)> {
    let mut entries = Vec::new();
    for entry in dir.entries() {
        let DirectoryEntry::Entry(entry) = entry.expect("directory entry read failed");
        let name = entry.name();
        if name.as_ref() == "."
            || name.as_ref() == ".."
            || entry.attributes().is_volume_label_entry()
        {
            continue;
        }
        entries.push((
            String::from(name.as_ref()),
            entry.len() as u32,
            entry.attributes().bits(),
        ));
    }
    entries
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
    stdin::reset();
    if let Some(con) = console::get() {
        con.forget_saved();
    }
    table[0] = Some(FileDescriptor::Keyboard);
    table[1] = Some(FileDescriptor::Console);
    table[2] = Some(FileDescriptor::Console);
}

/// Drops every fd the program that just ended still had open. Called from
/// `exec::process::enter`, right after the program returns control (`exit`, or a caught fault),
/// on every path. Finishes any `FileWrite` found along the way rather than bare-dropping it --
/// the program's own `close` normally does this (`FileWrite`'s own doc comment), but a program
/// that never calls it, or that faults first, would otherwise leave its redirected stdout/stderr
/// committed with the wrong size for whatever reads it next (a pipeline's next stage, or a later
/// `cat`) to find. Two fds can hold clones of the same `FileWrite` (`bind_stdio`'s `Dup`), so
/// `Rc::try_unwrap` only finishes it once the *last* clone is reclaimed -- if another is still
/// in an earlier or later slot of the same table, this leaves it be, and that slot's own turn
/// through this loop finishes it instead once it, in turn, is the last one left.
pub fn end_launch() {
    // A character the program left half written is one replacement character, not a stray start
    // for the next program's first byte.
    let mut tail = String::new();
    decoder().finish(|c| tail.push(c));
    if !tail.is_empty() {
        console_text(&tail);
    }
    // Give back the screen a full-screen program (one that drew a frame) replaced.
    if let Some(con) = console::get()
        && con.restore_saved()
        && let Some(gpu_dev) = gpu::get()
    {
        gpu_dev.flush();
    }
    for slot in table().iter_mut() {
        if let Some(FileDescriptor::FileWrite(_, writer)) = slot.take()
            && let Ok(writer) = Rc::try_unwrap(writer)
        {
            let _ = writer.into_inner().finish();
        }
    }
}

/// Decodes what a program writes to the console as UTF-8, a byte at a time: a character cut in two
/// by the end of one `write` (`cat` writes 4096-byte chunks) is finished by the next, and a byte
/// that is not valid UTF-8 comes out as U+FFFD instead of ending the stream.
///
/// SAFETY (every access, via `decoder`): single core, and syscalls run with IRQs masked.
static mut DECODER: Utf8Decoder = Utf8Decoder::new();

#[allow(clippy::deref_addrof)]
fn decoder() -> &'static mut Utf8Decoder {
    // SAFETY: see DECODER.
    unsafe { &mut *(&raw mut DECODER) }
}

/// Draws `text` on the console and sends it to the UART, the same dual visibility every
/// diagnostic print in this project already has.
fn console_text(text: &str) {
    if let Some(con) = console::get() {
        con.puts(text, console::DEFAULT_FG, console::DEFAULT_BG);
        if let Some(gpu_dev) = gpu::get() {
            gpu_dev.flush();
        }
    }
    let mut uart = crate::bare_metal::get_uart();
    let _ = uart.write_str(text);
}

/// A program's write to the console: `bytes` through the decoder, then drawn.
fn console_write(bytes: &[u8]) {
    let mut text = String::with_capacity(bytes.len());
    let decoder = decoder();
    for &byte in bytes {
        decoder.push(byte, |c| text.push(c));
    }
    console_text(&text);
}

impl FileDescriptor {
    /// Writes to the console or a write-only file. Writing to `Keyboard`, a read-only `File`, or a
    /// `Dir` is rejected outright, not silently accepted as a no-op: a mixed-up fd number should
    /// be visible as an error, not hidden behind output that simply never appears.
    fn write(&mut self, bytes: &[u8]) -> isize {
        match self {
            Self::Console => {
                console_write(bytes);
                bytes.len() as isize
            }
            Self::FileWrite(_, writer) => writer
                .borrow_mut()
                .write(bytes)
                .map_or_else(|e| from_fat(&e), |n| n as isize),
            Self::Keyboard | Self::File(..) => EBADF,
            Self::Dir { .. } => EISDIR,
        }
    }

    /// Reads from a file descriptor: the keyboard (blocks for a whole typed line -- see
    /// `keyboard::stdin`; `0` only for Ctrl+D on an empty line), or an open file's next bytes (`0`
    /// at end of file). Reading from `Console`, a write-only `FileWrite`, or a `Dir` is rejected,
    /// same reasoning as writing to `Keyboard`.
    fn read(&mut self, buf: &mut [u8]) -> isize {
        match self {
            Self::Keyboard => stdin::read(buf),
            Self::Console | Self::FileWrite(..) => EBADF,
            Self::Dir { .. } => EISDIR,
            Self::File(_, reader) => reader
                .borrow_mut()
                .read(buf)
                .map_or_else(|e| from_fat(&e), |n| n as isize),
        }
    }

    /// Fills `buf` with as many whole `DIRENT_SIZE` records as fit, picking up where the last call
    /// left off; `0` once every entry has been returned. Rejected (`ENOTDIR`) for anything but a
    /// directory, and (`EINVAL`) if `buf` can't even hold one record -- otherwise the `0` that
    /// means "buffer too small" couldn't be told apart from "listing exhausted".
    fn getdents(&mut self, buf: &mut [u8]) -> isize {
        let Self::Dir { entries, next, .. } = self else {
            return ENOTDIR;
        };
        if buf.len() < DIRENT_SIZE {
            return EINVAL;
        }
        let mut off = 0;
        while *next < entries.len() && off + DIRENT_SIZE <= buf.len() {
            let (name, size, attrs) = &entries[*next];
            let name = name.as_bytes();
            let n = name.len().min(NAME_MAX);
            let out = &mut buf[off..off + DIRENT_SIZE];
            out[0..4].copy_from_slice(&size.to_le_bytes());
            out[4] = *attrs;
            out[5] = n as u8;
            out[6..6 + n].copy_from_slice(&name[..n]);
            out[6 + n..].fill(0);
            off += DIRENT_SIZE;
            *next += 1;
        }
        off as isize
    }
}

/// Checks `ptr`/`len` before the kernel touches them: the whole range must lie in the user window
/// *and* in memory the running program actually has mapped (writable too, if the kernel is about to
/// write there). The MMU's `USER` attribute gates *EL0's* access, not EL1's, so a pointer aimed at
/// kernel memory would otherwise be readable by the kernel; and with the MMU on, the kernel faults
/// too on an unmapped page (the gap after the image, the stack's guard, heap above the break) or a
/// write to a read-only one (the program's own code) -- a data abort in the kernel, not a refusal.
/// `len == 0` is always valid regardless of `ptr`: a zero-length access touches no memory, and
/// Rust's own empty `&str`/`&[u8]` values carry a dangling sentinel pointer by convention (e.g.
/// `""`'s), not a real address inside the user window at all -- an empty path (`ls`'s own "use the
/// working directory" default) would otherwise fail this check before ever reaching `fs::cwd`.
fn validate(ptr: usize, len: usize, write: bool) -> bool {
    if len == 0 {
        // Still reject a literal null: `core::slice::from_raw_parts` requires a non-null pointer
        // even for a zero-length slice, unlike the dangling-but-non-null one Rust's own empty
        // `&str`/`&[u8]` values carry.
        return ptr != 0;
    }
    let Some(end) = ptr.checked_add(len) else {
        return false;
    };
    ptr >= USER_BASE && end <= USER_STACK_TOP && elf::user_memory().allows(ptr, len, write)
}

pub fn write(fd: usize, ptr: usize, len: usize) -> isize {
    if !validate(ptr, len, false) {
        return EFAULT;
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
    if !validate(ptr, len, true) {
        return EFAULT;
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

/// `getdents` on `fd`, into the user-space buffer `ptr`/`len` -- see `FileDescriptor::getdents`.
pub fn getdents(fd: usize, ptr: usize, len: usize) -> isize {
    if !validate(ptr, len, true) {
        return EFAULT;
    }
    // The write below touches a page marked USER, which PAN would otherwise forbid.
    let _user = mmu::user_access();
    // SAFETY: as in `write`.
    let buf = unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, len) };
    match table().get_mut(fd).and_then(Option::as_mut) {
        Some(fd) => fd.getdents(buf),
        None => EBADF,
    }
}

/// Flattens an inner call's `Result` into the one value a syscall returns: its result, or the
/// negated errno.
fn ret(result: Result<isize, isize>) -> isize {
    result.unwrap_or_else(|errno| errno)
}

/// Opens `path` (already absolute) for reading -- an ordinary file only, never a directory (a
/// redirected stdin can't be one either: `EISDIR`). Returns the file and the device it is on.
/// Shared by `open`'s `O_RDONLY` path, `bind_stdio`'s `In`, and `shell`'s own standalone redirect
/// application (`cd`/an empty-argv segment).
pub(crate) fn open_for_read(path: &str) -> Result<(usize, FileReader<'static, BlkIo>), isize> {
    let (dev, vol, rest) = fs::path::volume_for(path).ok_or(EIO)?;
    let vol: &'static FatVolume<BlkIo> = vol;
    let entry = match fs::path::resolve(vol, rest) {
        Err(EINVAL) => return Err(EISDIR), // the root
        other => other?,
    };
    Ok((dev, vol.read_file(&entry).map_err(|e| from_fat(&e))?))
}

/// Opens `path` (already absolute) for writing: creates it if missing, or truncates it to empty
/// unless `append` -- in which case an existing file is instead opened positioned at its current
/// end (`FileWriter::new_append`, not wrapped by `FatVolumeWriteExt`'s own convenience method,
/// which is always create-or-truncate; a missing file with `append` set is still created fresh,
/// since there is nothing yet to append to). A directory at `path` is refused either way
/// (`EISDIR`). Returns the writer and the device it is on. Shared by `open`'s `O_WRONLY` path,
/// `bind_stdio`'s `Out`, and `shell`'s own standalone redirect application (`cd`/an empty-argv
/// segment).
pub(crate) fn open_for_write(
    path: &str,
    append: bool,
) -> Result<(usize, FileWriter<'static, BlkIo>), isize> {
    let (dev, vol, rest) = fs::path::volume_for(path).ok_or(EIO)?;
    let vol: &'static FatVolume<BlkIo> = vol;
    let (parent, leaf) = match fs::path::split(vol, rest) {
        Err(EINVAL) => return Err(EISDIR), // the root
        other => other?,
    };
    let entry = match fs::path::find_entry(&parent, leaf) {
        Some(entry) if entry.is_directory() => return Err(EISDIR),
        Some(entry) => entry,
        None => vol.create_file(&parent, leaf).map_err(|e| from_fat(&e))?,
    };
    let writer = if append {
        FileWriter::new_append(vol, &entry)
    } else {
        vol.write_file(&entry)
    };
    Ok((dev, writer.map_err(|e| from_fat(&e))?))
}

/// A command's redirections in the order they were written, each with the fd it binds (an `In`
/// is fd 0; `Out` and `Dup` carry their own). The order matters: `> f 2>&1` and `2>&1 > f` differ.
pub type Stdio = Vec<(usize, StdioRedirect)>;

/// One redirection to apply to a program's stdio before it starts -- built by `shell`'s own
/// launch code from `shell::syntax::Redirection`, with the fd it binds carried separately (the
/// array position in `bind_stdio`'s own parameter), not stored here.
pub enum StdioRedirect {
    In(String),
    Out {
        path: String,
        append: bool,
    },
    /// Make this fd a second owner of whatever `target` (1 or 2) already names (`>&`) -- see
    /// `bind_stdio`'s own handling of this case, and `FileDescriptor::FileWrite`'s doc comment
    /// for why that's a clone, not an index onto `target`.
    Dup(u8),
}

/// The streams standard fds 0-2 start from at every launch, below the command's own redirects:
/// what the enclosing script or builtin was itself redirected to (`with_inherited_stdio`), `None`
/// for a stream that is still the console's (or keyboard's) default. Only ever holds `Keyboard`,
/// `Console`, `File` and `FileWrite`.
///
/// SAFETY (every access, via `base`): single core, and the shell runs strictly sequentially --
/// the same reasoning `FD_TABLE` rests on.
static mut BASE_STDIO: [Option<FileDescriptor>; 3] = [const { None }; 3];

#[allow(clippy::deref_addrof)]
fn base() -> &'static mut [Option<FileDescriptor>; 3] {
    // SAFETY: see BASE_STDIO.
    unsafe { &mut *(&raw mut BASE_STDIO) }
}

/// Opens `overrides` -- the redirects of one command, built by `shell` from
/// `shell::syntax::Redirection`s, plus a pipeline stage's own pipe binding underneath them --
/// on top of `inherited`, giving the three streams the command ends up with (`None`: untouched,
/// still the default). Left to right, each already in effect for the ones after it: a `Dup`
/// (`>&`) copies whatever its target stream is *so far*, `inherited` included, so `2>&1` after
/// `> f` shares `f`, and before it shares what stdout was.
///
/// `Dup`'s target (1 or 2) holds a `Console`, `Keyboard`, `File` or `FileWrite` by then -- an
/// unset one is the console's own default -- so it clones that rather than opening anything: the
/// clone shares the same open file (`FileDescriptor::FileWrite`'s doc comment says why it has to).
///
/// `Err` is what to report (`path: No such file or directory`) if any `In`/`Out` fails to resolve
/// or open; nothing already opened earlier in the same call is undone beyond being dropped
/// uncommitted, since the command never runs.
fn resolve_streams(
    inherited: &[Option<FileDescriptor>; 3],
    overrides: Stdio,
) -> Result<[Option<FileDescriptor>; 3], String> {
    let mut streams = inherited.clone();
    for (fd, redirect) in overrides {
        let stream = match redirect {
            StdioRedirect::In(path) => {
                let (dev, reader) = open_for_read(&fs::cwd::absolute(&path))
                    .map_err(|e| format!("{path}: {}", errmsg(e)))?;
                FileDescriptor::File(dev, Rc::new(RefCell::new(reader)))
            }
            StdioRedirect::Out { path, append } => {
                let (dev, writer) = open_for_write(&fs::cwd::absolute(&path), append)
                    .map_err(|e| format!("{path}: {}", errmsg(e)))?;
                FileDescriptor::FileWrite(dev, Rc::new(RefCell::new(writer)))
            }
            StdioRedirect::Dup(target) => match streams.get(target as usize) {
                Some(Some(stream)) => stream.clone(),
                Some(None) => FileDescriptor::Console,
                None => return Err(format!(">&{target}: bad file descriptor")),
            },
        };
        streams[fd] = Some(stream);
    }
    Ok(streams)
}

/// Binds the base streams, then `overrides` on top of them, onto fds 0-2 -- called by
/// `exec::process::run_program` right after `reset_for_launch()` and before `enter()`, so a
/// program starts with its redirected stdio already in place. `Err` (already reported by the
/// caller) if a redirect can't be opened.
pub fn bind_stdio(overrides: Stdio) -> Result<(), ()> {
    let streams = resolve_streams(base(), overrides).map_err(|_| ())?;
    for (fd, stream) in streams.into_iter().enumerate() {
        if let Some(stream) = stream {
            table()[fd] = Some(stream);
        }
    }
    Ok(())
}

/// Writes `bytes` to the standard output the code running right now was given: what the enclosing
/// script or builtin was itself redirected to (`with_inherited_stdio`), the console when it was
/// not. For the shell's own builtins, which are not programs with an fd 1 of their own, to print
/// where `cd - > f`, `cd - | cat` and `$(cd -)` expect. A failed write (a full disk) is dropped,
/// as a builtin has nowhere else to report it.
pub fn write_stdout(bytes: &[u8]) {
    match base()[1].as_mut() {
        Some(stream) => {
            let _ = stream.write(bytes);
        }
        None => console_write(bytes),
    }
}

/// Runs `f` -- a script, or a builtin that is not a program of its own -- with `overrides`
/// applied on top of the streams it already has, as that code's own stdio: every program `f`
/// launches (and every redirect inside it, layered on top) starts from them, so `./script > out`
/// collects what every command in the script prints, and `source file | cat` pipes it. Nests:
/// whatever `f` is itself redirected to is what its own inner scripts inherit.
///
/// The files the redirects open are shared by every command that inherits them (one open file,
/// one offset), and are committed -- a writer's size written to disk -- when the outermost scope
/// holding them ends, here, on every path out of `f`. `Err` is what to report if a redirect can't be
/// opened, in which case `f` never runs.
pub fn with_inherited_stdio<R>(overrides: Stdio, f: impl FnOnce() -> R) -> Result<R, String> {
    let streams = resolve_streams(base(), overrides)?;
    let outer = core::mem::replace(base(), streams.clone());
    let result = f();
    *base() = outer;
    // Dropping `streams` last: a writer shared with the outer scope (inherited, not opened here)
    // still has a reference there and stays open; one opened here is down to ours (or, with
    // `2>&1`, to its two slots, the second of which finishes it).
    for stream in streams.into_iter().flatten() {
        if let FileDescriptor::FileWrite(_, writer) = stream
            && let Ok(writer) = Rc::try_unwrap(writer)
        {
            let _ = writer.into_inner().finish();
        }
    }
    Ok(result)
}

/// Opens the file or directory at the user-space path `ptr`/`len` (resolved against the working
/// directory first, `fs::cwd::absolute`), returning the lowest free fd number (never one of the
/// standard three) or a negated errno. `O_RDONLY` also opens a directory (the root included, for
/// `""`/`"/"`), for `getdents` rather than `read`/`write`. `O_WRONLY` creates the file if it's
/// missing, or truncates it to empty if it already exists as a plain file (`EISDIR` for a
/// directory) -- or, combined with `O_APPEND`, opens it positioned at its current end instead
/// (still creating it fresh if it didn't exist); no read-only-bit check yet (`chmod`'s own
/// `ATTR_READ_ONLY` isn't consulted here). `EINVAL` for any other `flags` value or a path that
/// isn't valid UTF-8, `ENOENT`/`ENOTDIR` for a path that doesn't resolve (`O_RDONLY` only --
/// `O_WRONLY` creates it instead), `EMFILE` when every fd slot is already in use, and whatever
/// else the volume says (`ENOSPC`, `EIO`, ...).
pub fn open(ptr: usize, len: usize, flags: usize) -> isize {
    ret(open_inner(ptr, len, flags))
}

fn open_inner(ptr: usize, len: usize, flags: usize) -> Result<isize, isize> {
    let append = flags & O_APPEND != 0;
    let base = flags & !O_APPEND;
    if base != O_RDONLY && base != O_WRONLY {
        return Err(EINVAL);
    }
    // The read below touches a page marked USER, which PAN would otherwise forbid; kept alive for
    // the rest of this function, since `path` (below) is a zero-copy view straight into it.
    let _user = mmu::user_access();
    let path = fs::cwd::absolute(user_str(ptr, len)?);
    let (dev, vol, rest) = fs::path::volume_for(&path).ok_or(EIO)?;

    let descriptor = if base == O_WRONLY {
        let (dev, writer) = open_for_write(&path, append)?;
        FileDescriptor::FileWrite(dev, Rc::new(RefCell::new(writer)))
    } else if let Ok(dir) = fs::path::resolve_dir(vol, rest) {
        // Tried first: a path through directories only (the empty/root path included) opens for
        // `getdents`. If it fails, the same path is tried as an ordinary file below --
        // `resolve_dir` and `resolve` walk identically up to the last component, so this only
        // succeeds here for a path `resolve` would otherwise have resolved to a directory entry
        // it doesn't know what to do with.
        FileDescriptor::Dir {
            dev,
            entries: list_dir(&dir),
            next: 0,
        }
    } else {
        let entry = fs::path::resolve(vol, rest)?;
        let reader = vol.read_file(&entry).map_err(|e| from_fat(&e))?;
        FileDescriptor::File(dev, Rc::new(RefCell::new(reader)))
    };

    let table = table();
    let fd = table
        .iter()
        .skip(3)
        .position(Option::is_none)
        .map(|i| i + 3)
        .ok_or(EMFILE)?;
    table[fd] = Some(descriptor);
    Ok(fd as isize)
}

/// Closes `fd`: the slot is emptied, dropping whatever it held. For a `FileWrite`, this also
/// commits its final size to disk (`FileWriter::finish`) -- but only once every fd sharing it
/// (`bind_stdio`'s `Dup`) has been closed, this one included; while another still holds it, this
/// just lets go of *this* fd's own clone, leaving the other fully usable (`FileDescriptor::
/// FileWrite`'s own doc comment). `EBADF` for an already-closed or never-opened fd, or whatever
/// the volume says if the `finish` failed (`ENOSPC`, `EIO`); closing a standard fd (0/1/2) is
/// allowed, same as a real `close` -- it's just gone for the rest of this program's run.
pub fn close(fd: usize) -> isize {
    match table().get_mut(fd).and_then(Option::take) {
        Some(FileDescriptor::FileWrite(_, writer)) => match Rc::try_unwrap(writer) {
            Ok(writer) => match writer.into_inner().finish() {
                Ok(()) => 0,
                Err(e) => from_fat(&e),
            },
            Err(_) => 0, // another fd still shares it; that one's own close finishes it
        },
        Some(_) => 0,
        None => EBADF,
    }
}

/// Creates an empty directory at the user-space path `ptr`/`len` (resolved against the working
/// directory first). `ENOENT`/`ENOTDIR` if the path's parent doesn't exist or isn't a directory,
/// `EEXIST` if something already exists at that name (the root always does), `ENOSPC` when the
/// volume is full.
pub fn mkdir(ptr: usize, len: usize) -> isize {
    ret(mkdir_inner(ptr, len))
}

fn mkdir_inner(ptr: usize, len: usize) -> Result<isize, isize> {
    let _user = mmu::user_access();
    let path = fs::cwd::absolute(user_str(ptr, len)?);
    let (_, vol, rest) = fs::path::volume_for(&path).ok_or(EIO)?;
    let (parent, leaf) = match fs::path::split(vol, rest) {
        Err(EINVAL) => return Err(EEXIST), // the root, or a mount point
        other => other?,
    };
    if fs::path::find_entry(&parent, leaf).is_some() {
        return Err(EEXIST);
    }
    vol.create_dir(&parent, leaf).map_err(|e| from_fat(&e))?;
    Ok(0)
}

/// Removes the file, or empty directory, at the user-space path `ptr`/`len` (resolved against the
/// working directory first) -- one call for both (`hadris_fat::FatVolume::delete`'s own behavior;
/// no `AT_REMOVEDIR` flag to distinguish them, unlike Linux's real `unlinkat`). `ENOENT` for a
/// missing path, `ENOTEMPTY` for a non-empty directory, `EBUSY` for the root, a mount point or a
/// directory with one inside it.
pub fn unlink(ptr: usize, len: usize) -> isize {
    ret(unlink_inner(ptr, len))
}

fn unlink_inner(ptr: usize, len: usize) -> Result<isize, isize> {
    let _user = mmu::user_access();
    let path = fs::cwd::absolute(user_str(ptr, len)?);
    if fs::mounts::table().covers_a_mount(&path) {
        return Err(EBUSY);
    }
    let (_, vol, rest) = fs::path::volume_for(&path).ok_or(EIO)?;
    let entry = match fs::path::resolve(vol, rest) {
        Err(EINVAL) => return Err(EBUSY), // the root
        other => other?,
    };
    vol.delete(&entry).map_err(|e| from_fat(&e))?;
    Ok(0)
}

/// Renames or moves the entry at the user-space path `old_ptr`/`old_len` to `new_ptr`/`new_len`
/// (each resolved against the working directory first), within one volume -- a thin wrap of
/// `hadris_fat::FatVolume::rename` (which refuses moving a directory into its own descendant:
/// `EINVAL`), plus replacing an existing plain-file destination, as POSIX `rename` does. `EXDEV` if
/// the two paths are on different volumes (a program copies and removes instead), `EBUSY` if either
/// side is a mount point (or the source has one inside it), `ENOENT`/`ENOTDIR` for a path that
/// doesn't resolve. A directory on either side of a replacement, or the very same file spelled
/// another way, is `EEXIST` (`EISDIR` for a file replaced by a directory is not told apart: the
/// name is taken either way).
pub fn rename(old_ptr: usize, old_len: usize, new_ptr: usize, new_len: usize) -> isize {
    ret(rename_inner(old_ptr, old_len, new_ptr, new_len))
}

fn rename_inner(
    old_ptr: usize,
    old_len: usize,
    new_ptr: usize,
    new_len: usize,
) -> Result<isize, isize> {
    let _user = mmu::user_access();
    let old_path = fs::cwd::absolute(user_str(old_ptr, old_len)?);
    let new_path = fs::cwd::absolute(user_str(new_ptr, new_len)?);
    // A mount point cannot be moved or replaced, nor can a directory with one inside it: the
    // volume's own root has no directory entry to rename. Checked before the volumes, so that
    // `mv` does not take `EXDEV` for "copy it" and empty a mounted volume.
    if fs::mounts::table().covers_a_mount(&old_path)
        || fs::mounts::table().is_mount_point(&new_path)
    {
        return Err(EBUSY);
    }
    let (old_dev, vol, old_rest) = fs::path::volume_for(&old_path).ok_or(EIO)?;
    let (new_dev, _, new_rest) = fs::path::volume_for(&new_path).ok_or(EIO)?;
    if old_dev != new_dev {
        return Err(EXDEV); // a rename cannot move an entry between volumes: `mv` copies and removes
    }
    let old_entry = fs::path::resolve(vol, old_rest)?;
    let (new_parent, new_leaf) = fs::path::split(vol, new_rest)?;
    // An existing plain file at the destination is replaced (POSIX `rename`), but only once
    // everything above has checked out -- `hadris_fat`'s own `rename` refuses an existing name,
    // and a caller that removed the target first would lose it whenever the rename then failed.
    // A directory on either side, or the very same file spelled another way (`A` for `a`), is
    // refused: replacing one could orphan a whole tree.
    if let Some(existing) = fs::path::find_entry(&new_parent, new_leaf) {
        if existing.is_directory()
            || old_entry.is_directory()
            || old_path.eq_ignore_ascii_case(&new_path)
        {
            return Err(EEXIST);
        }
        vol.delete(&existing).map_err(|e| from_fat(&e))?;
    }
    let moved = vol
        .rename(&old_entry, &new_parent, new_leaf)
        .map_err(|e| from_fat(&e))?;
    // `hadris_fat` stamps a renamed entry with the clock; a move is not a modification
    // (POSIX leaves the time alone), so put the old one back. Failing to is not failing
    // the rename, which has already happened.
    let _ = vol.set_times(&moved, Some(old_entry.modified()), None, None);
    Ok(0)
}

/// Copies the working directory's absolute path into the user buffer `ptr`/`len` (no terminating
/// NUL). Returns its length, or `ERANGE` if `buf` is too small.
pub fn getcwd(ptr: usize, len: usize) -> isize {
    ret(getcwd_inner(ptr, len))
}

fn getcwd_inner(ptr: usize, len: usize) -> Result<isize, isize> {
    if !validate(ptr, len, true) {
        return Err(EFAULT);
    }
    let _user = mmu::user_access();
    let cwd = fs::cwd::get();
    if cwd.len() > len {
        return Err(ERANGE);
    }
    // SAFETY: validated above to lie entirely within the user window, and `cwd.len() <= len`.
    let buf = unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, cwd.len()) };
    buf.copy_from_slice(cwd.as_bytes());
    Ok(cwd.len() as isize)
}

/// Writes every field a FAT directory entry actually stores for the user-space path `ptr`/`len`
/// (resolved against the working directory first) into the `STAT_SIZE`-byte buffer at `out_ptr`
/// -- see `abi::STAT_SIZE`'s own doc comment for the exact layout. `ENOENT`/`ENOTDIR` if the path
/// doesn't resolve, `EFAULT` if `out_ptr`/`STAT_SIZE` isn't valid user memory.
///
/// Unlike `open`, this doesn't need a "try as a directory first" branch: `fs::path::resolve`
/// (unlike `resolve_dir`) already returns a real `FileEntry` for *any* named directory, not just
/// a file -- it only refuses the bare root, which has no entry of its own to return at all (no
/// parent directory lists it). That's the one case needing synthetic values; every other
/// directory or file reads every field straight off the real entry.
pub fn stat(ptr: usize, len: usize, out_ptr: usize) -> isize {
    ret(stat_inner(ptr, len, out_ptr))
}

fn stat_inner(ptr: usize, len: usize, out_ptr: usize) -> Result<isize, isize> {
    if !validate(out_ptr, STAT_SIZE, true) {
        return Err(EFAULT);
    }
    let _user = mmu::user_access();
    let path = fs::cwd::absolute(user_str(ptr, len)?);
    let (_, vol, rest) = fs::path::volume_for(&path).ok_or(EIO)?;

    let (size, attrs, created, modified, accessed_date) = if rest.trim_matches('/').is_empty() {
        // A volume's own root (`/`, or a mount point) has no `FileEntry` of its own -- nothing
        // lists it -- so there's nothing real to read; report it as an empty directory at the
        // FAT epoch.
        (
            0u32,
            ATTR_DIRECTORY,
            FatDateTime::EPOCH,
            FatDateTime::EPOCH,
            FatDateTime::EPOCH.date,
        )
    } else {
        let entry = fs::path::resolve(vol, rest)?;
        (
            entry.len() as u32,
            entry.attributes().bits(),
            entry.created(),
            entry.modified(),
            entry.accessed_date(),
        )
    };

    // SAFETY: validated above to lie entirely within the user window.
    let out = unsafe { core::slice::from_raw_parts_mut(out_ptr as *mut u8, STAT_SIZE) };
    out[0..4].copy_from_slice(&size.to_le_bytes());
    out[4] = attrs;
    out[5..7].copy_from_slice(&created.date.to_le_bytes());
    out[7..9].copy_from_slice(&created.time.to_le_bytes());
    out[9] = created.time_tenth;
    out[10..12].copy_from_slice(&modified.date.to_le_bytes());
    out[12..14].copy_from_slice(&modified.time.to_le_bytes());
    out[14..16].copy_from_slice(&accessed_date.to_le_bytes());
    Ok(0)
}

/// The only attribute bits a program may change via `chmod` -- `ATTR_DIRECTORY`/`VOLUME_ID`
/// describe the kind of entry on disk, not a permission, and `set_attributes` itself already
/// refuses flipping `DIRECTORY` (though not `VOLUME_ID`) -- checked here too, on both `set` and
/// `clear`, so the rejection is uniform and immediate rather than depending on which bit the
/// crate happens to guard.
const CHMOD_BITS: u8 = ATTR_READ_ONLY | ATTR_EXEC;

/// Sets `set`'s bits and clears `clear`'s bits of the user-space path `ptr`/`len`'s attribute
/// byte (resolved against the working directory first). `EINVAL` if `set`/`clear` name anything
/// outside `CHMOD_BITS` or the path is the bare root (which has no entry to change), `ENOENT`/
/// `ENOTDIR` if the path doesn't resolve, and whatever the underlying `set_attributes` says.
pub fn chmod(ptr: usize, len: usize, set: usize, clear: usize) -> isize {
    ret(chmod_inner(ptr, len, set, clear))
}

fn chmod_inner(ptr: usize, len: usize, set: usize, clear: usize) -> Result<isize, isize> {
    if set > u8::MAX as usize || clear > u8::MAX as usize {
        return Err(EINVAL);
    }
    let (set, clear) = (set as u8, clear as u8);
    if (set | clear) & !CHMOD_BITS != 0 {
        return Err(EINVAL);
    }
    let _user = mmu::user_access();
    let path = fs::cwd::absolute(user_str(ptr, len)?);
    let (_, vol, rest) = fs::path::volume_for(&path).ok_or(EIO)?;
    let entry = fs::path::resolve(vol, rest)?;
    let attrs = (entry.attributes().bits() | set) & !clear;
    vol.set_attributes(&entry, DirEntryAttrFlags::from_bits_retain(attrs))
        .map_err(|e| from_fat(&e))?;
    Ok(0)
}

/// Sets the modify time of the user-space path `ptr`/`len` (resolved against the working
/// directory first) from the second of the two `timespec`s at `times_ptr` -- the first, the access
/// time, is checked and ignored: FAT stores an access *date* only, nothing here reads it, and a
/// timestamp that coarse is not worth a behavior. A null `times_ptr` is `UTIME_NOW` for both.
/// FAT keeps the modify time to 2 seconds and holds 1980-2107: a time outside that and a `tv_nsec`
/// that is neither a real fraction of a second nor one of the two special values are `EINVAL`, as
/// is the bare root (which has no directory entry to hold a time); a path that doesn't resolve is
/// `ENOENT`/`ENOTDIR`. The creation time is never touched. The file need not be writable.
pub fn utimensat(ptr: usize, len: usize, times_ptr: usize) -> isize {
    ret(utimensat_inner(ptr, len, times_ptr))
}

fn utimensat_inner(ptr: usize, len: usize, times_ptr: usize) -> Result<isize, isize> {
    let _user = mmu::user_access();
    let path = fs::cwd::absolute(user_str(ptr, len)?);
    let mtime = if times_ptr == 0 {
        TimeChoice::Now
    } else {
        if !validate(times_ptr, UTIMES_SIZE, false) {
            return Err(EFAULT);
        }
        // SAFETY: validated above to lie entirely within the user window.
        let raw = unsafe { core::slice::from_raw_parts(times_ptr as *const u8, UTIMES_SIZE) };
        let read = |at: usize| {
            let sec = i64::from_le_bytes(raw[at..at + 8].try_into().unwrap_or([0; 8]));
            let nsec = i64::from_le_bytes(raw[at + 8..at + 16].try_into().unwrap_or([0; 8]));
            fattime::choose(sec, nsec)
        };
        match (read(0), read(16)) {
            (Some(_), Some(mtime)) => mtime,
            _ => return Err(EINVAL),
        }
    };
    let modified = match mtime {
        TimeChoice::Omit => None,
        TimeChoice::Now => Some(RTC_TIME.now()),
        TimeChoice::At(unix) => {
            let f = fattime::checked_fields(unix).ok_or(EINVAL)?;
            Some(FatDateTime::new(
                f.year, f.month, f.day, f.hour, f.minute, f.second,
            ))
        }
    };
    let (_, vol, rest) = fs::path::volume_for(&path).ok_or(EIO)?;
    let entry = fs::path::resolve(vol, rest)?;
    vol.set_times(&entry, modified, None, None)
        .map_err(|e| from_fat(&e))?;
    Ok(0)
}

/// Whether any open fd refers to a file or directory on device `dev`: what `umount` refuses on.
fn open_on(dev: usize) -> bool {
    table().iter().flatten().any(|fd| match fd {
        FileDescriptor::File(d, _)
        | FileDescriptor::FileWrite(d, _)
        | FileDescriptor::Dir { dev: d, .. } => *d == dev,
        FileDescriptor::Keyboard | FileDescriptor::Console => false,
    })
}

/// Reads the user-space string `ptr`/`len`. `EFAULT` for a range outside the user window,
/// `EINVAL` for one that isn't UTF-8. The caller holds `mmu::user_access()` for as long as the
/// returned view is used.
fn user_str<'a>(ptr: usize, len: usize) -> Result<&'a str, isize> {
    if !validate(ptr, len, false) {
        return Err(EFAULT);
    }
    // SAFETY: validated above to lie entirely within the user window.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
    core::str::from_utf8(bytes).map_err(|_| abi::errno::EINVAL)
}

/// `mount(source, target)`: see `abi::SYS_MOUNT` and `fs::mounts::mount`. The target is resolved
/// against the working directory first; the source (`LABEL=...`/`UUID=...`) is not a path.
pub fn mount(src_ptr: usize, src_len: usize, target_ptr: usize, target_len: usize) -> isize {
    let _user = mmu::user_access();
    let (source, target) = match (user_str(src_ptr, src_len), user_str(target_ptr, target_len)) {
        (Ok(source), Ok(target)) => (source, target),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    fs::mounts::mount(source, &fs::cwd::absolute(target))
}

/// `umount(target)`: see `abi::SYS_UMOUNT2` and `fs::mounts::umount`.
pub fn umount(ptr: usize, len: usize) -> isize {
    let _user = mmu::user_access();
    match user_str(ptr, len) {
        Ok(target) => fs::mounts::umount(&fs::cwd::absolute(target), open_on),
        Err(e) => e,
    }
}

/// `blkinfo(index, out)`: see `abi::SYS_BLKINFO`. Writes `BLKINFO_SIZE` bytes to `out`, or
/// `ENODEV` for an index past the last device, `EFAULT` for a bad `out`.
pub fn blkinfo(index: usize, out_ptr: usize) -> isize {
    if !validate(out_ptr, BLKINFO_SIZE, true) {
        return EFAULT;
    }
    let Some(info) = fs::devices::blkinfo(index) else {
        return ENODEV;
    };
    let _user = mmu::user_access();
    // SAFETY: validated above to lie entirely within the user window.
    let out = unsafe { core::slice::from_raw_parts_mut(out_ptr as *mut u8, BLKINFO_SIZE) };
    out.copy_from_slice(&info.encode());
    0
}

/// Out-of-band control of what `fd` is open on (see `abi::ioctl` for the requests). Only the
/// terminal understands any. `CONSOLE_CLEAR` clears the console (stdout/stderr) and, via an ANSI
/// escape sequence, whatever real terminal is watching the UART. `TIOCGWINSZ` writes the console's
/// size in cells to `arg`, answered for the keyboard (stdin) as well as the console, so a program
/// learns from any of its three standard fds whether it is on a terminal and how big. `CONSOLE_
/// READ_KEY` blocks for one key press and writes it to `arg`; stdin only -- the console fd is for
/// drawing, not reading. `CONSOLE_DRAW` draws a whole frame from `arg`; the console only, the
/// opposite way round. Any other request, or any other kind of fd, is `ENOTTY`; an fd that is not
/// open is `EBADF`.
pub fn ioctl(fd: usize, request: usize, arg: usize) -> isize {
    let entry = table().get(fd).and_then(Option::as_ref);
    match (entry, request) {
        (None, _) => EBADF,
        (Some(FileDescriptor::Console | FileDescriptor::Keyboard), TIOCGWINSZ) => window_size(arg),
        (Some(FileDescriptor::Keyboard), CONSOLE_READ_KEY) => read_key(arg),
        (Some(FileDescriptor::Console), CONSOLE_DRAW) => draw_frame(arg),
        (Some(FileDescriptor::Console), CONSOLE_CLEAR) => clear(),
        _ => ENOTTY,
    }
}

/// `CONSOLE_READ_KEY`: blocks until a key is pressed and writes it (`abi::keys::KeyEvent`) to the
/// user buffer `ptr`; `EFAULT` if it is not writable user memory. The pointer is validated
/// *before* blocking, so a bad one is refused at once rather than after making the caller wait for
/// a keypress that was never going to be delivered anywhere.
fn read_key(ptr: usize) -> isize {
    if !validate(ptr, KEYEVENT_SIZE, true) {
        return EFAULT;
    }
    let event = stdin::read_key();
    let _user = mmu::user_access(); // writes a user buffer: clear PAN while it does
    // SAFETY: validated above to lie entirely within writable user memory.
    let out = unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, KEYEVENT_SIZE) };
    out.copy_from_slice(&event.encode());
    0
}

/// `TIOCGWINSZ`: writes the console's rows and columns (`abi::ioctl::WinSize`, pixel fields 0) to
/// the user buffer `ptr`; `EFAULT` if it is not writable user memory.
fn window_size(ptr: usize) -> isize {
    if !validate(ptr, WINSIZE_SIZE, true) {
        return EFAULT;
    }
    let Some(con) = console::get() else {
        return ENOTTY;
    };
    let size = WinSize::cells(con.rows as u16, con.cols as u16);
    let _user = mmu::user_access();
    // SAFETY: validated above to lie entirely within writable user memory.
    let out = unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, WINSIZE_SIZE) };
    out.copy_from_slice(&size.encode());
    0
}

/// What `draw_frame` needs of one `Cell`: the character, its attribute and how many columns it
/// takes (2 for a wide glyph, 1 otherwise) -- or `None` for a cell that is not a valid,
/// non-control Unicode scalar value, which refuses the whole frame.
fn decode_cell(bytes: [u8; CELL_SIZE]) -> Option<(char, u8, usize)> {
    let cell = Cell::decode(bytes);
    let c = char::from_u32(cell.ch).filter(|c| !c.is_control())?;
    let width = if console::font::glyph_for(c).is_fullwidth() {
        2
    } else {
        1
    };
    Some((c, cell.attr, width))
}

/// The `(fg, bg)` pair for one cell's `attr`. Reserved bits beyond `ATTR_INVERSE`/`ATTR_DIM` are
/// masked off, not refused, so a future flag does not break a program built against this one.
fn cell_colors(attr: u8) -> (u32, u32) {
    let (fg, bg) = (console::DEFAULT_FG, console::DEFAULT_BG);
    match (attr & ATTR_INVERSE != 0, attr & ATTR_DIM != 0) {
        (false, false) => (fg, bg),
        (false, true) => (console::DIM_FG, bg),
        (true, false) => (bg, fg),
        (true, true) => (bg, console::DIM_FG),
    }
}

/// `CONSOLE_DRAW`: draws a whole frame from the header-then-cells buffer at `ptr` (see
/// `abi::ioctl::CONSOLE_DRAW`). Walks the frame **twice**: once to check every cell decodes to a
/// character with room for its width in its row (`EINVAL` on the first violation -- a malformed
/// frame is refused whole, never drawn in part and then rejected), and again to draw. A wide
/// glyph's second column is walked (its bytes still exist in the buffer) but never separately
/// decoded or drawn.
fn draw_frame(ptr: usize) -> isize {
    if !validate(ptr, CONSOLE_DRAW_HEADER_SIZE, false) {
        return EFAULT;
    }
    let Some(con) = console::get() else {
        return ENOTTY;
    };
    let _user = mmu::user_access(); // reads a user buffer: clear PAN while it does
    // SAFETY: validated above to lie entirely within mapped user memory.
    let header_bytes =
        unsafe { core::slice::from_raw_parts(ptr as *const u8, CONSOLE_DRAW_HEADER_SIZE) };
    let header = ConsoleDraw::decode(header_bytes.try_into().unwrap());

    let (rows, cols) = (con.rows, con.cols);
    if usize::from(header.rows) != rows || usize::from(header.cols) != cols {
        return EINVAL;
    }
    let Some(cell_bytes_len) = (rows * cols).checked_mul(CELL_SIZE) else {
        return EINVAL;
    };
    let Some(cells_ptr) = ptr.checked_add(CONSOLE_DRAW_HEADER_SIZE) else {
        return EFAULT;
    };
    if !validate(cells_ptr, cell_bytes_len, false) {
        return EFAULT;
    }
    // SAFETY: validated above to lie entirely within mapped user memory.
    let cell_bytes = unsafe { core::slice::from_raw_parts(cells_ptr as *const u8, cell_bytes_len) };
    let cell_at = |row: usize, col: usize| -> [u8; CELL_SIZE] {
        let start = (row * cols + col) * CELL_SIZE;
        cell_bytes[start..start + CELL_SIZE].try_into().unwrap()
    };

    // Pass 1: every cell that will be drawn must decode, and fit in what is left of its row.
    for row in 0..rows {
        let mut col = 0;
        while col < cols {
            let Some((_, _, width)) = decode_cell(cell_at(row, col)) else {
                return EINVAL;
            };
            if col + width > cols {
                return EINVAL;
            }
            col += width;
        }
    }

    // Pass 2: draw, now that nothing in the frame can panic `put_char_at`. A program's first
    // frame saves what was on screen, for `end_launch` to give back (the alternate screen).
    con.snapshot_once();
    for row in 0..rows {
        let mut col = 0;
        while col < cols {
            let (c, attr, width) = decode_cell(cell_at(row, col)).unwrap();
            let (fg, bg) = cell_colors(attr);
            con.put_char_at(row, col, c, fg, bg);
            col += width;
        }
    }
    con.move_cursor(
        usize::from(header.cursor_row).min(rows - 1),
        usize::from(header.cursor_col).min(cols - 1),
    );
    if let Some(gpu_dev) = gpu::get() {
        gpu_dev.flush();
    }
    0
}

/// `CONSOLE_CLEAR`: clears the console -- both the graphical one (if a GPU device exists) and, via an ANSI escape
/// sequence (cursor home, erase display), whatever real terminal is watching the UART, the serial
/// counterpart of clearing the console. No fd argument: there's only ever one console, so there's
/// nothing to look up (unlike `read`/`write`, which are addressing one of several fds).
fn clear() -> isize {
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
