//! Resolving a `/`-separated path against one volume's root directory. Which volume a path is on
//! is the mount table's answer (`volume_for`: `fs::mounts::locate` picks the device, and hands
//! back what remains of the path for that volume); everything here then deals in the part of the
//! path *within* that volume. `cwd`'s `absolute` handles the working-directory half; every path
//! given to `volume_for` is already absolute and normalized.

use hadris_fat::sync::{DirectoryEntry, FatDir, FatVolume, FileEntry};

use super::blkio::BlkIo;
use super::{mounts, volume};

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

/// Resolves `path` to the `FatDir` it names, one component at a time from `vol`'s root -- the root
/// itself if `path` has no components at all (`""` or `"/"`), unlike `resolve` above, which has no
/// entry to return for that case. `Err(())` if any component is missing, or any component
/// (including the last) isn't a directory -- so a path naming an ordinary file always fails here,
/// not just a path through one.
pub fn resolve_dir<'a>(vol: &'a FatVolume<BlkIo>, path: &str) -> Result<FatDir<'a, BlkIo>, ()> {
    let mut dir = vol.root_dir();
    for name in path.split('/').filter(|c| !c.is_empty()) {
        let entry = find_entry(&dir, name).ok_or(())?;
        if !entry.is_directory() {
            return Err(());
        }
        dir = dir.open_entry(&entry).map_err(|_| ())?;
    }
    Ok(dir)
}

/// Splits `path` into the `FatDir` holding its final component and that component's own name --
/// what `mkdir`, a fresh `open(O_WRONLY)`, and the destination side of `rename` all need (a place
/// to create an entry, and the name to give it), where `resolve`/`resolve_dir` only ever hand back
/// something that already exists. Built on `resolve_dir` directly (via `rsplit_once`, not a second
/// component walker) rather than duplicating its loop. `Err(())` if `path` has no components at
/// all (the root itself has no parent to split off), or any component up to the parent isn't a
/// directory.
pub fn split<'a>(
    vol: &'a FatVolume<BlkIo>,
    path: &'a str,
) -> Result<(FatDir<'a, BlkIo>, &'a str), ()> {
    let trimmed = path.trim_matches('/');
    let (parent, leaf) = trimmed.rsplit_once('/').unwrap_or(("", trimmed));
    if leaf.is_empty() {
        return Err(());
    }
    Ok((resolve_dir(vol, parent)?, leaf))
}

/// The volume that answers for the absolute, normalized `path`: its device index, the open
/// volume, and the rest of the path within it (`""` for the volume's own root, which is what a
/// mount point resolves to; otherwise starting with `/`). `None` only if the mount table sent the
/// path to a device with no open volume, which it never does for a mounted one.
pub fn volume_for(path: &str) -> Option<(usize, &'static mut FatVolume<BlkIo>, &str)> {
    let (dev, rest) = mounts::locate(path);
    volume::get(dev).map(|vol| (dev, vol, rest))
}
