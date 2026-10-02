//! The mounted volumes: where in the tree each open FAT volume answers for paths.
//!
//! `TABLE` (`mounttable.rs`, pure) says which device answers for a path; `volume` holds the open
//! `FatVolume` for each device that is mounted. The root is mounted once at boot
//! (`install_root`); every other mount comes from `mount` -- the `mount` syscall, and the shell's
//! start-up doing the same for the lines of `/etc/fstab`. A volume stays open for as long as it is
//! mounted.
//!
//! SAFETY of the static (every access): single core; every syscall runs with IRQs masked and the
//! shell runs outside interrupts, so nothing else touches it while a call is in progress.

use abi::blk::BLK_MOUNT_MAX;
use abi::errno::{EBUSY, EINVAL, ENAMETOOLONG, ENODEV, ENOENT, ENOTUNIQ};

use super::mounttable::{FindError, MountError, MountTable, Source, is_within};
use super::{devices, path, volume};

static mut TABLE: Option<MountTable> = None;

/// The mount table. Before `install_root` it is empty (everything resolves to device 0).
#[allow(clippy::deref_addrof)]
pub fn table() -> &'static MountTable {
    // SAFETY: see the module doc.
    unsafe { (*(&raw mut TABLE)).get_or_insert_with(MountTable::new) }
}

#[allow(clippy::deref_addrof)]
fn table_mut() -> &'static mut MountTable {
    // SAFETY: see the module doc.
    unsafe { (*(&raw mut TABLE)).get_or_insert_with(MountTable::new) }
}

/// Which device answers for the absolute, normalized `path`, and what remains of it for that
/// volume (`""` for the volume's own root -- what a mount point resolves to -- else a path
/// starting with `/`).
pub fn locate(path: &str) -> (usize, &str) {
    table().resolve(path)
}

/// Opens device `dev` as the boot-time root and mounts it at `/`. `false` if it is not a
/// filesystem `hadris_fat` can open.
pub fn install_root(dev: usize) -> bool {
    if !volume::open(dev) {
        return false;
    }
    table_mut()
        .add("/", dev)
        .expect("the root is the first mount");
    true
}

/// Whether the absolute, normalized `target` names a directory that exists: `Ok(())`, or
/// `ENOENT`/`ENOTDIR`. A mount point (a volume's own root) is one.
fn check_directory(target: &str) -> Result<(), isize> {
    let (dev, rest) = locate(target);
    let vol = volume::get(dev).ok_or(ENOENT)?;
    path::resolve_dir(vol, rest).map(|_| ())
}

/// Mounts the volume `source` names (`LABEL=name` or `UUID=XXXX-XXXX`) on the directory `target`,
/// an absolute, normalized path. `0`, or: `EINVAL` (not a source, or not a filesystem this kernel
/// can open), `ENODEV` (no device has that label or ID), `ENOTUNIQ` (more than one does: a label or
/// an ID is not guaranteed unique, so it is refused rather than guessed), `EBUSY` (that volume is mounted already,
/// `target` is a mount point, or another mount is inside it), `ENAMETOOLONG`, and what
/// `check_directory` says of the target (`ENOENT`, `ENOTDIR`).
pub fn mount(source: &str, target: &str) -> isize {
    let Some(source) = Source::parse(source) else {
        return EINVAL;
    };
    if target.len() > BLK_MOUNT_MAX {
        return ENAMETOOLONG;
    }
    let dev = match source.find(&devices::identities()) {
        Ok(dev) => dev,
        Err(FindError::NoMatch) => return ENODEV,
        Err(FindError::Ambiguous(_)) => return ENOTUNIQ,
    };
    if table().point_of(dev).is_some() || table().covers_a_mount(target) {
        return EBUSY;
    }
    if let Err(e) = check_directory(target) {
        return e;
    }
    // The volume is in place before the table can send a path to it.
    if !volume::open(dev) {
        return EINVAL;
    }
    match table_mut().add(target, dev) {
        Ok(()) => 0,
        Err(_) => {
            // Cannot happen (both were checked above); undo rather than leave a volume nothing
            // refers to.
            volume::close(dev);
            EBUSY
        }
    }
}

/// Unmounts the volume mounted exactly at `target`. `0`, or `EINVAL` (nothing is mounted there)
/// and `EBUSY` (the root, a mount with another inside it, a volume with the working directory in
/// it, or one `in_use` says a file is open on). `in_use` is asked with the volume's device index:
/// which files are open is the fd table's knowledge (`syscall::fd`), not this layer's.
pub fn umount(target: &str, in_use: impl Fn(usize) -> bool) -> isize {
    if !table().is_mount_point(target) {
        return EINVAL;
    }
    let dev = table().resolve(target).0;
    if target == "/" || is_within(target, &super::cwd::get()) || in_use(dev) {
        return EBUSY;
    }
    match table_mut().remove(target) {
        Ok(dev) => {
            // No file is open on it (checked above), so nothing borrows the volume. Nothing is
            // cached in memory (`BlkIo` writes through), so dropping the volume loses no data.
            volume::close(dev);
            0
        }
        Err(MountError::NotMounted) => EINVAL,
        Err(_) => EBUSY,
    }
}
