//! What a `hadris_fat` error means as a syscall's negated errno -- the one place that decides it,
//! so every path through the file system reports the same failure the same way.

use abi::errno::{EBUSY, EEXIST, EINVAL, EIO, EISDIR, ENOENT, ENOSPC, ENOTDIR, ENOTEMPTY};
use hadris_fat::Error;

/// `error` as a negated errno. Anything about the volume itself (bad clusters, a corrupt table, a
/// failed device read or write) is `EIO`.
pub fn from_fat(error: &Error) -> isize {
    match error {
        Error::NotAFile => EISDIR,
        Error::NotADirectory => ENOTDIR,
        Error::EntryNotFound | Error::StaleEntry => ENOENT,
        Error::AlreadyExists => EEXIST,
        Error::DirectoryNotEmpty => ENOTEMPTY,
        Error::NoFreeSpace | Error::DirectoryFull => ENOSPC,
        Error::InvalidPath
        | Error::InvalidShortFilename
        | Error::InvalidFilename
        | Error::InvalidAttributeChange { .. } => EINVAL,
        Error::WriterConflict => EBUSY,
        _ => EIO,
    }
}
