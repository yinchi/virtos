//! Resolving a `/`-separated path against a volume's root directory. No mount table, no working
//! directory: every path is walked from `vol.root_dir()`, on whichever single volume the caller
//! already picked (`syscall::fd::open` and `shell::run` both currently hardcode volume 0 --
//! `plans/USERSPACE_PLAN.md`'s note on why real multi-volume path resolution isn't needed yet).

use hadris_fat::sync::{DirectoryEntry, FatDir, FatVolume, FileEntry};

use super::blkio::BlkIo;

/// The entry named `name` directly inside `dir`, if there is one. Case-insensitive, matching
/// FAT's own short-name semantics.
pub fn find_entry(dir: &FatDir<'_, BlkIo>, name: &str) -> Option<FileEntry> {
    for entry in dir.entries() {
        let DirectoryEntry::Entry(entry) = entry.expect("directory entry read failed");
        if entry.name().eq_ignore_ascii_case(name) {
            return Some(entry);
        }
    }
    None
}

/// Resolves `path` (its `/`-separated components; leading, trailing and doubled slashes are all
/// tolerated -- empty components are just skipped) against `vol`'s root directory, one component
/// at a time. `Err(())` if any component is missing, a non-final component isn't a directory, or
/// `path` has no components at all (`""` or `"/"` -- the root itself has no entry to return).
pub fn resolve(vol: &FatVolume<BlkIo>, path: &str) -> Result<FileEntry, ()> {
    let mut dir = vol.root_dir();
    let mut comps = path.split('/').filter(|c| !c.is_empty()).peekable();
    loop {
        let name = comps.next().ok_or(())?;
        let entry = find_entry(&dir, name).ok_or(())?;
        if comps.peek().is_none() {
            return Ok(entry);
        }
        if !entry.is_directory() {
            return Err(());
        }
        dir = dir.open_entry(&entry).map_err(|_| ())?;
    }
}
