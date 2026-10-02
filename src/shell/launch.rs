//! Finding the file a command word names, checking that it may run, and reading it: everything
//! `dispatch_segment` needs before it can start a program or a script. The failures come back as
//! the message to report and the exit status a POSIX shell gives them (`126` found but could not
//! be run, `127` not found).
//!
//! A command word containing `/` is a path, relative to the working directory unless it starts
//! with `/`. A bare name is looked up in the directories of `$PATH` (`path_search.rs`; `/usr/bin`
//! if it is unset), in order, as typed. The first regular file found wins: a directory of that
//! name is skipped, as is a candidate that does not exist.
//!
//! A file found has to have the executable bit (`ATTR_EXEC`, `chmod +x`) -- `source FILE` and
//! `sh FILE` do not need it, as in POSIX -- and is then ELF, or something `program_kind::classify`
//! says is a script, or refused as binary data.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use abi::ATTR_EXEC;
use abi::errno::{EACCES, EISDIR, ENOENT, ENOEXEC, errmsg};
use hadris_fat::FatVolumeReadExt;
use hadris_fat::sync::{FatVolume, FileEntry};

use super::program_kind::{Kind, classify};
use super::{frame_stack, path_search};
use crate::fs;
use crate::fs::blkio::BlkIo;

/// The largest executable the shell will read into memory: anything bigger is refused as not
/// executable rather than risking an allocation failure (which would panic the kernel).
const MAX_PROGRAM_SIZE: usize = crate::bare_metal::HEAP_SIZE / 2;

/// Exit statuses for a command that did not run, as in POSIX shells: `126` found but could not be
/// run (a directory, no exec bit, too big, not a valid program), `127` not found.
pub const STATUS_CANNOT_EXECUTE: i32 = 126;
pub const STATUS_NOT_FOUND: i32 = 127;

/// What a command word turned out to be.
pub enum Program {
    /// An ELF file's whole content, for `exec::process::run_program`.
    Elf(Vec<u8>),
    /// A script's text, for the shell to run.
    Script(String),
}

/// A command that could not be run: what to report, and `$?`.
pub struct Failure {
    pub message: String,
    pub status: i32,
}

fn failure(message: String, status: i32) -> Failure {
    Failure { message, status }
}

/// The entry at the absolute path `path` and the volume it is on, if there is one (a volume's own
/// root has no entry, so is "not there").
fn lookup(path: &str) -> Option<(&'static mut FatVolume<BlkIo>, FileEntry)> {
    let (_, vol, rest) = fs::path::volume_for(path)?;
    let entry = fs::path::resolve(vol, rest).ok()?;
    Some((vol, entry))
}

/// The absolute paths a bare `name` would be run from, in order: `$PATH`'s directories (a
/// relative entry is relative to the working directory).
fn candidates(name: &str) -> Vec<String> {
    let path = frame_stack::var("PATH");
    path_search::candidates(path, name)
        .iter()
        .map(|candidate| fs::cwd::absolute(candidate))
        .collect()
}

/// The first regular file `$PATH` names `name` as: what `source` and `.` read, ahead of the
/// working directory (a name with a `/` in it is a path, never searched for). The file need only
/// be readable.
pub fn search_script(name: &str) -> Option<String> {
    if name.contains('/') {
        return None;
    }
    candidates(name)
        .into_iter()
        .find(|path| lookup(path).is_some_and(|(_, entry)| !entry.is_directory()))
}

/// Finds the file `name` names, as the module doc says.
fn find_program(name: &str) -> Result<(&'static mut FatVolume<BlkIo>, FileEntry), Failure> {
    if name.contains('/') {
        return lookup(&fs::cwd::absolute(name))
            .ok_or_else(|| failure(format!("{name}: {}", errmsg(ENOENT)), STATUS_NOT_FOUND));
    }
    for path in candidates(name) {
        if let Some((vol, entry)) = lookup(&path)
            && !entry.is_directory()
        {
            return Ok((vol, entry));
        }
    }
    Err(failure(
        format!("{name}: command not found"),
        STATUS_NOT_FOUND,
    ))
}

/// Finds, checks and reads the command `name` names. See the module doc for the order of
/// checks; every refusal is bash's wording.
pub fn load(name: &str) -> Result<Program, Failure> {
    let (vol, entry) = find_program(name)?;
    let refuse = |why: &str| failure(format!("{name}: {why}"), STATUS_CANNOT_EXECUTE);
    if entry.is_directory() {
        return Err(refuse(errmsg(EISDIR)));
    }
    if entry.attributes().bits() & ATTR_EXEC == 0 {
        return Err(refuse(errmsg(EACCES)));
    }
    if entry.len() as usize > MAX_PROGRAM_SIZE {
        return Err(refuse(&format!("cannot execute: {}", errmsg(ENOEXEC))));
    }
    let Ok(mut reader) = vol.read_file(&entry) else {
        return Err(refuse("failed to open"));
    };
    let Ok(bytes) = reader.read_to_vec() else {
        return Err(refuse("failed to read"));
    };
    match classify(&bytes) {
        Kind::Elf => Ok(Program::Elf(bytes)),
        Kind::Script => String::from_utf8(bytes)
            .map(Program::Script)
            .map_err(|_| refuse("not valid UTF-8")),
        Kind::Binary => Err(refuse(&format!(
            "cannot execute binary file: {}",
            errmsg(ENOEXEC)
        ))),
        Kind::NotUtf8 => Err(refuse("not valid UTF-8")),
    }
}
