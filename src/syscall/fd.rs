//! File descriptor dispatch for `open`/`close`/`read`/`write`/`getdents`, plus the path-only
//! (no fd involved) `mkdir`/`unlink`/`rename`/`getcwd`/`stat`/`chmod`. Every path a program supplies is
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
use crate::drivers::virtio::gpu;
use crate::fs;
use crate::fs::blkio::BlkIo;
use crate::fs::fattime::{self, TimeChoice};
use crate::fs::rtc_time::RTC_TIME;
use crate::keyboard::stdin;
use abi::{
    ATTR_DIRECTORY, ATTR_EXEC, ATTR_READ_ONLY, DIRENT_SIZE, NAME_MAX, O_APPEND, O_RDONLY, O_WRONLY,
    STAT_SIZE, UTIMES_SIZE,
};

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
    /// An open file, read-only.
    File(FileReader<'static, BlkIo>),
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
    FileWrite(Rc<RefCell<FileWriter<'static, BlkIo>>>),
    /// An open directory: `entries` captured whole when it was opened (unlike a file, FAT gives no
    /// way to resume a directory walk at an arbitrary byte offset) -- each entry's name, size, and
    /// attribute byte, so `getdents` can hand a child's type back to a caller (`chmod -R`'s own
    /// recursion, eventually `ls -l`/`-F`) without it needing a separate `stat` call per entry.
    /// `next` is how many `getdents` has already returned.
    Dir {
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
    for slot in table().iter_mut() {
        if let Some(FileDescriptor::FileWrite(writer)) = slot.take()
            && let Ok(writer) = Rc::try_unwrap(writer)
        {
            let _ = writer.into_inner().finish();
        }
    }
}

impl FileDescriptor {
    /// Writes to the console or a write-only file. Writing to `Keyboard`, a read-only `File`, or a
    /// `Dir` is rejected outright, not silently accepted as a no-op: a mixed-up fd number should
    /// be visible as an error, not hidden behind output that simply never appears.
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
                // Also the UART, the same dual visibility every diagnostic print in this project
                // already has.
                let mut uart = crate::bare_metal::get_uart();
                let _ = uart.write_str(text);
                bytes.len() as isize
            }
            Self::FileWrite(writer) => writer
                .borrow_mut()
                .write(bytes)
                .map_or(EBADF, |n| n as isize),
            Self::Keyboard | Self::File(_) | Self::Dir { .. } => EBADF,
        }
    }

    /// Reads from a file descriptor: the keyboard (blocks for a whole typed line -- see
    /// `keyboard::stdin`; `0` only for Ctrl+D on an empty line), or an open file's next bytes (`0`
    /// at end of file). Reading from `Console`, a write-only `FileWrite`, or a `Dir` is rejected,
    /// same reasoning as writing to `Keyboard`.
    fn read(&mut self, buf: &mut [u8]) -> isize {
        match self {
            Self::Keyboard => stdin::read(buf),
            Self::Console | Self::FileWrite(_) | Self::Dir { .. } => EBADF,
            Self::File(reader) => reader.read(buf).map_or(EBADF, |n| n as isize),
        }
    }

    /// Fills `buf` with as many whole `DIRENT_SIZE` records as fit, picking up where the last call
    /// left off; `0` once every entry has been returned. Rejected (`EBADF`) for anything but a
    /// directory, and if `buf` can't even hold one record -- otherwise the `0` that means "buffer
    /// too small" couldn't be told apart from "listing exhausted".
    fn getdents(&mut self, buf: &mut [u8]) -> isize {
        let Self::Dir { entries, next } = self else {
            return EBADF;
        };
        if buf.len() < DIRENT_SIZE {
            return EBADF;
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

/// Bounds-checks `ptr`/`len` against the user window before trusting them: the MMU's `USER`
/// attribute bit gates *EL0's* access, not EL1's, so a buggy or malicious pointer aimed at kernel
/// memory is otherwise still readable/writable by the kernel and must be rejected in software.
/// `len == 0` is always valid regardless of `ptr`: a zero-length access touches no memory, and
/// Rust's own empty `&str`/`&[u8]` values carry a dangling sentinel pointer by convention (e.g.
/// `""`'s), not a real address inside the user window at all -- an empty path (`ls`'s own "use the
/// working directory" default) would otherwise fail this check before ever reaching `fs::cwd`.
fn validate(ptr: usize, len: usize) -> bool {
    if len == 0 {
        // Still reject a literal null: `core::slice::from_raw_parts` requires a non-null pointer
        // even for a zero-length slice, unlike the dangling-but-non-null one Rust's own empty
        // `&str`/`&[u8]` values carry.
        return ptr != 0;
    }
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

/// `getdents` on `fd`, into the user-space buffer `ptr`/`len` -- see `FileDescriptor::getdents`.
pub fn getdents(fd: usize, ptr: usize, len: usize) -> isize {
    if !validate(ptr, len) {
        return EBADF;
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

/// Opens `path` (already absolute) for reading -- an ordinary file only, never a directory (a
/// redirected stdin can't be one either). Shared by `open`'s `O_RDONLY` path, `bind_stdio`'s
/// `In`, and `shell`'s own standalone redirect application (`cd`/an empty-argv segment).
pub(crate) fn open_for_read(
    vol: &'static FatVolume<BlkIo>,
    path: &str,
) -> Result<FileReader<'static, BlkIo>, ()> {
    let entry = fs::path::resolve(vol, path).map_err(|_| ())?;
    vol.read_file(&entry).map_err(|_| ())
}

/// Opens `path` (already absolute) for writing: creates it if missing, or truncates it to empty
/// unless `append` -- in which case an existing file is instead opened positioned at its current
/// end (`FileWriter::new_append`, not wrapped by `FatVolumeWriteExt`'s own convenience method,
/// which is always create-or-truncate; a missing file with `append` set is still created fresh,
/// since there is nothing yet to append to). A directory at `path` is refused either way. Shared
/// by `open`'s `O_WRONLY` path (never `append`, since nothing exposes that flag yet),
/// `bind_stdio`'s `Out`, and `shell`'s own standalone redirect application (`cd`/an empty-argv
/// segment).
pub(crate) fn open_for_write(
    vol: &'static FatVolume<BlkIo>,
    path: &str,
    append: bool,
) -> Result<FileWriter<'static, BlkIo>, ()> {
    let (parent, leaf) = fs::path::split(vol, path).map_err(|_| ())?;
    let entry = match fs::path::find_entry(&parent, leaf) {
        Some(entry) if entry.is_directory() => return Err(()),
        Some(entry) => entry,
        None => vol.create_file(&parent, leaf).map_err(|_| ())?,
    };
    if append {
        FileWriter::new_append(vol, &entry).map_err(|_| ())
    } else {
        vol.write_file(&entry).map_err(|_| ())
    }
}

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

/// Binds `overrides[i]`, when present, onto fd `i` for `i` in `0..3` -- called by
/// `exec::process::run_program` right after `reset_for_launch()` and before `enter()`, so a
/// redirected program's very first `read`/`write` already sees the right fd. Processed in array
/// order (0, then 1, then 2) regardless of which redirection the shell line actually wrote first
/// -- building `overrides` from a `Segment`'s redirections already collapses that ordering (a
/// later redirection of the same fd simply overwrites an earlier one in the same array slot), so
/// a `Dup` at index 2 sees whatever index 1 was *already* set to by this same call, not
/// necessarily what fd 1 held before this line ran. This is a known, intentionally accepted
/// divergence from a real shell's snapshot-at-the-time semantics for the less common
/// `cmd 2>&1 > f` ordering (a real shell leaves fd 2 on the original target; this always ends up
/// cloning fd 1 onto `f` instead, the same as `cmd > f 2>&1`) -- not fixed here, since it needs
/// tracking *when* each redirection was written, which nothing before this point keeps.
///
/// `Dup`'s target must already be `Console`, `Keyboard`, or `FileWrite` by the time this runs --
/// the only things `In`/`Out`/a fresh launch's own reset ever put at fd 1/2 -- so this itself
/// never opens anything for it, only clones whichever of those `overrides[target]`'s own
/// processing (if any) already left in place, or the slot's `reset_for_launch` default otherwise.
///
/// `Err` (already reported by the caller) if any `In`/`Out` fails to resolve or open, or a `Dup`'s
/// target doesn't hold one of the three expected kinds; nothing already bound earlier in the same
/// call is undone, since the caller never launches the program at all on `Err`.
pub fn bind_stdio(overrides: [Option<StdioRedirect>; 3]) -> Result<(), ()> {
    let vol = fs::volume::get(0).ok_or(())?;
    for (fd, redirect) in overrides.into_iter().enumerate() {
        let Some(redirect) = redirect else { continue };
        let descriptor = match redirect {
            StdioRedirect::In(path) => {
                FileDescriptor::File(open_for_read(vol, &fs::cwd::absolute(&path))?)
            }
            StdioRedirect::Out { path, append } => FileDescriptor::FileWrite(Rc::new(
                RefCell::new(open_for_write(vol, &fs::cwd::absolute(&path), append)?),
            )),
            StdioRedirect::Dup(target) => match table().get(target as usize) {
                Some(Some(FileDescriptor::Console)) => FileDescriptor::Console,
                Some(Some(FileDescriptor::Keyboard)) => FileDescriptor::Keyboard,
                Some(Some(FileDescriptor::FileWrite(writer))) => {
                    FileDescriptor::FileWrite(writer.clone())
                }
                _ => return Err(()),
            },
        };
        table()[fd] = Some(descriptor);
    }
    Ok(())
}

/// Opens the file or directory at the user-space path `ptr`/`len` (resolved against the working
/// directory first, `fs::cwd::absolute`) on `system.img` (blk0 -- no path/mount resolution across
/// volumes yet, see `fs::path`'s own doc comment), returning the lowest free fd number (never one
/// of the standard three) or `-1`. `O_RDONLY` also opens a
/// directory (the root included, for `""`/`"/"`), for `getdents` rather than `read`/`write`.
/// `O_WRONLY` creates the file if it's missing, or truncates it to empty if it already exists as
/// a plain file (a directory is refused) -- or, combined with `O_APPEND`, opens it positioned at
/// its current end instead (still creating it fresh if it didn't exist); no read-only-bit check
/// yet (`chmod`'s own `ATTR_READ_ONLY` isn't consulted here). Also `-1` for any other `flags`
/// value, a missing path (`O_RDONLY` only -- `O_WRONLY` creates it instead), a path that isn't
/// valid UTF-8, or every fd slot already in use.
pub fn open(ptr: usize, len: usize, flags: usize) -> isize {
    let append = flags & O_APPEND != 0;
    let base = flags & !O_APPEND;
    if base != O_RDONLY && base != O_WRONLY {
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
    let path = fs::cwd::absolute(path);

    let Some(vol) = fs::volume::get(0) else {
        return EBADF;
    };

    let descriptor = if base == O_WRONLY {
        let Ok(writer) = open_for_write(vol, &path, append) else {
            return EBADF;
        };
        FileDescriptor::FileWrite(Rc::new(RefCell::new(writer)))
    } else if let Ok(dir) = fs::path::resolve_dir(vol, &path) {
        // Tried first: a path through directories only (the empty/root path included) opens for
        // `getdents`. If it fails, the same path is tried as an ordinary file below --
        // `resolve_dir` and `resolve` walk identically up to the last component, so this only
        // succeeds here for a path `resolve` would otherwise have resolved to a directory entry
        // it doesn't know what to do with.
        FileDescriptor::Dir {
            entries: list_dir(&dir),
            next: 0,
        }
    } else {
        let Ok(entry) = fs::path::resolve(vol, &path) else {
            return EBADF;
        };
        let Ok(reader) = vol.read_file(&entry) else {
            return EBADF;
        };
        FileDescriptor::File(reader)
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
    table[fd] = Some(descriptor);
    fd as isize
}

/// Closes `fd`: the slot is emptied, dropping whatever it held. For a `FileWrite`, this also
/// commits its final size to disk (`FileWriter::finish`) -- but only once every fd sharing it
/// (`bind_stdio`'s `Dup`) has been closed, this one included; while another still holds it, this
/// just lets go of *this* fd's own clone, leaving the other fully usable (`FileDescriptor::
/// FileWrite`'s own doc comment). `-1` for an already-closed or never-opened fd, or a `finish`
/// that failed; closing a standard fd (0/1/2) is allowed, same as a real `close` -- it's just
/// gone for the rest of this program's run.
pub fn close(fd: usize) -> isize {
    match table().get_mut(fd).and_then(Option::take) {
        Some(FileDescriptor::FileWrite(writer)) => match Rc::try_unwrap(writer) {
            Ok(writer) => match writer.into_inner().finish() {
                Ok(()) => 0,
                Err(_) => EBADF,
            },
            Err(_) => 0, // another fd still shares it; that one's own close finishes it
        },
        Some(_) => 0,
        None => EBADF,
    }
}

/// Creates an empty directory at the user-space path `ptr`/`len` (resolved against the working
/// directory first). `-1` if the path's parent doesn't exist or isn't a directory, if something
/// already exists at that name, or if `path` has no components at all (the root always exists).
pub fn mkdir(ptr: usize, len: usize) -> isize {
    if !validate(ptr, len) {
        return EBADF;
    }
    let _user = mmu::user_access();
    // SAFETY: validated above to lie entirely within the user window.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
    let Ok(path) = core::str::from_utf8(bytes) else {
        return EBADF;
    };
    let path = fs::cwd::absolute(path);
    let Some(vol) = fs::volume::get(0) else {
        return EBADF;
    };
    let Ok((parent, leaf)) = fs::path::split(vol, &path) else {
        return EBADF;
    };
    match vol.create_dir(&parent, leaf) {
        Ok(_) => 0,
        Err(_) => EBADF,
    }
}

/// Removes the file, or empty directory, at the user-space path `ptr`/`len` (resolved against the
/// working directory first) -- one call for both (`hadris_fat::FatVolume::delete`'s own behavior;
/// no `AT_REMOVEDIR` flag to distinguish them, unlike Linux's real `unlinkat`), refusing only a
/// *non-empty* directory. `-1` for a missing path or a non-empty directory.
pub fn unlink(ptr: usize, len: usize) -> isize {
    if !validate(ptr, len) {
        return EBADF;
    }
    let _user = mmu::user_access();
    // SAFETY: validated above to lie entirely within the user window.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
    let Ok(path) = core::str::from_utf8(bytes) else {
        return EBADF;
    };
    let path = fs::cwd::absolute(path);
    let Some(vol) = fs::volume::get(0) else {
        return EBADF;
    };
    let Ok(entry) = fs::path::resolve(vol, &path) else {
        return EBADF;
    };
    match vol.delete(&entry) {
        Ok(()) => 0,
        Err(_) => EBADF,
    }
}

/// Renames or moves the entry at the user-space path `old_ptr`/`old_len` to `new_ptr`/`new_len`
/// (each resolved against the working directory first), within the same volume -- a thin wrap of
/// `hadris_fat::FatVolume::rename`, which refuses if `new` already names something (whatever type
/// it is) and refuses moving a directory into its own descendant. `-1` for any of those, or if
/// either path fails to resolve.
pub fn rename(old_ptr: usize, old_len: usize, new_ptr: usize, new_len: usize) -> isize {
    if !validate(old_ptr, old_len) || !validate(new_ptr, new_len) {
        return EBADF;
    }
    let _user = mmu::user_access();
    // SAFETY: both ranges validated above to lie entirely within the user window.
    let old_bytes = unsafe { core::slice::from_raw_parts(old_ptr as *const u8, old_len) };
    let new_bytes = unsafe { core::slice::from_raw_parts(new_ptr as *const u8, new_len) };
    let (Ok(old_path), Ok(new_path)) = (
        core::str::from_utf8(old_bytes),
        core::str::from_utf8(new_bytes),
    ) else {
        return EBADF;
    };
    let old_path = fs::cwd::absolute(old_path);
    let new_path = fs::cwd::absolute(new_path);
    let Some(vol) = fs::volume::get(0) else {
        return EBADF;
    };
    let Ok(old_entry) = fs::path::resolve(vol, &old_path) else {
        return EBADF;
    };
    let Ok((new_parent, new_leaf)) = fs::path::split(vol, &new_path) else {
        return EBADF;
    };
    match vol.rename(&old_entry, &new_parent, new_leaf) {
        Ok(moved) => {
            // `hadris_fat` stamps a renamed entry with the clock; a move is not a modification
            // (POSIX leaves the time alone), so put the old one back. Failing to is not failing
            // the rename, which has already happened.
            let _ = vol.set_times(&moved, Some(old_entry.modified()), None, None);
            0
        }
        Err(_) => EBADF,
    }
}

/// Copies the working directory's absolute path into the user buffer `ptr`/`len` (no terminating
/// NUL). Returns its length, or `-1` if `buf` is too small.
pub fn getcwd(ptr: usize, len: usize) -> isize {
    if !validate(ptr, len) {
        return EBADF;
    }
    let _user = mmu::user_access();
    let cwd = fs::cwd::get();
    if cwd.len() > len {
        return EBADF;
    }
    // SAFETY: validated above to lie entirely within the user window, and `cwd.len() <= len`.
    let buf = unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, cwd.len()) };
    buf.copy_from_slice(cwd.as_bytes());
    cwd.len() as isize
}

/// Writes every field a FAT directory entry actually stores for the user-space path `ptr`/`len`
/// (resolved against the working directory first) into the `STAT_SIZE`-byte buffer at `out_ptr`
/// -- see `abi::STAT_SIZE`'s own doc comment for the exact layout. `-1` if the path doesn't
/// resolve, or if `out_ptr`/`STAT_SIZE` isn't valid user memory.
///
/// Unlike `open`, this doesn't need a "try as a directory first" branch: `fs::path::resolve`
/// (unlike `resolve_dir`) already returns a real `FileEntry` for *any* named directory, not just
/// a file -- it only refuses the bare root, which has no entry of its own to return at all (no
/// parent directory lists it). That's the one case needing synthetic values; every other
/// directory or file reads every field straight off the real entry.
pub fn stat(ptr: usize, len: usize, out_ptr: usize) -> isize {
    if !validate(ptr, len) || !validate(out_ptr, STAT_SIZE) {
        return EBADF;
    }
    let _user = mmu::user_access();
    // SAFETY: validated above to lie entirely within the user window.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
    let Ok(path) = core::str::from_utf8(bytes) else {
        return EBADF;
    };
    let path = fs::cwd::absolute(path);
    let Some(vol) = fs::volume::get(0) else {
        return EBADF;
    };

    let (size, attrs, created, modified, accessed_date) = if path.trim_matches('/').is_empty() {
        // The true root has no `FileEntry` of its own -- nothing lists it -- so there's nothing
        // real to read; report it as an empty directory at the FAT epoch.
        (
            0u32,
            ATTR_DIRECTORY,
            FatDateTime::EPOCH,
            FatDateTime::EPOCH,
            FatDateTime::EPOCH.date,
        )
    } else {
        let Ok(entry) = fs::path::resolve(vol, &path) else {
            return EBADF;
        };
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
    0
}

/// The only attribute bits a program may change via `chmod` -- `ATTR_DIRECTORY`/`VOLUME_ID`
/// describe the kind of entry on disk, not a permission, and `set_attributes` itself already
/// refuses flipping `DIRECTORY` (though not `VOLUME_ID`) -- checked here too, on both `set` and
/// `clear`, so the rejection is uniform and immediate rather than depending on which bit the
/// crate happens to guard.
const CHMOD_BITS: u8 = ATTR_READ_ONLY | ATTR_EXEC;

/// Sets `set`'s bits and clears `clear`'s bits of the user-space path `ptr`/`len`'s attribute
/// byte (resolved against the working directory first). `-1` if `set`/`clear` name anything
/// outside `CHMOD_BITS`, the path doesn't resolve (this also refuses the bare root, which
/// `fs::path::resolve` already has no entry to return for -- no separate check needed), or the
/// underlying `set_attributes` call fails.
pub fn chmod(ptr: usize, len: usize, set: usize, clear: usize) -> isize {
    if !validate(ptr, len) {
        return EBADF;
    }
    if set > u8::MAX as usize || clear > u8::MAX as usize {
        return EBADF;
    }
    let (set, clear) = (set as u8, clear as u8);
    if (set | clear) & !CHMOD_BITS != 0 {
        return EBADF;
    }
    let _user = mmu::user_access();
    // SAFETY: validated above to lie entirely within the user window.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
    let Ok(path) = core::str::from_utf8(bytes) else {
        return EBADF;
    };
    let path = fs::cwd::absolute(path);
    let Some(vol) = fs::volume::get(0) else {
        return EBADF;
    };
    let Ok(entry) = fs::path::resolve(vol, &path) else {
        return EBADF;
    };
    let attrs = (entry.attributes().bits() | set) & !clear;
    match vol.set_attributes(&entry, DirEntryAttrFlags::from_bits_retain(attrs)) {
        Ok(()) => 0,
        Err(_) => EBADF,
    }
}

/// Sets the modify time of the user-space path `ptr`/`len` (resolved against the working
/// directory first) from the second of the two `timespec`s at `times_ptr` -- the first, the access
/// time, is checked and ignored: FAT stores an access *date* only, nothing here reads it, and a
/// timestamp that coarse is not worth a behavior. A null `times_ptr` is `UTIME_NOW` for both.
/// FAT keeps the modify time to 2 seconds and holds 1980-2107: a time outside that, a `tv_nsec`
/// that is neither a real fraction of a second nor one of the two special values, a path that
/// doesn't resolve, and the bare root (which has no directory entry to hold a time) are all `-1`.
/// The creation time is never touched. The file need not be writable.
pub fn utimensat(ptr: usize, len: usize, times_ptr: usize) -> isize {
    if !validate(ptr, len) {
        return EBADF;
    }
    let _user = mmu::user_access();
    // SAFETY: validated above to lie entirely within the user window.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
    let Ok(path) = core::str::from_utf8(bytes) else {
        return EBADF;
    };
    let path = fs::cwd::absolute(path);
    let mtime = if times_ptr == 0 {
        TimeChoice::Now
    } else {
        if !validate(times_ptr, UTIMES_SIZE) {
            return EBADF;
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
            _ => return EBADF,
        }
    };
    let modified = match mtime {
        TimeChoice::Omit => None,
        TimeChoice::Now => Some(RTC_TIME.now()),
        TimeChoice::At(unix) => match fattime::checked_fields(unix) {
            Some(f) => Some(FatDateTime::new(
                f.year, f.month, f.day, f.hour, f.minute, f.second,
            )),
            None => return EBADF,
        },
    };
    let Some(vol) = fs::volume::get(0) else {
        return EBADF;
    };
    let Ok(entry) = fs::path::resolve(vol, &path) else {
        return EBADF;
    };
    match vol.set_times(&entry, modified, None, None) {
        Ok(()) => 0,
        Err(_) => EBADF,
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
