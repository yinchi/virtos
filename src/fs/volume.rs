//! Opening `blk`'s block devices as FAT volumes through `blkio::BlkIo`.
//!
//! Volume-level only: no path/namespace concept at all, just "this device's filesystem is now
//! readable/writable" -- the same split the reference lab draws between `fs::mounts::open_volume`
//! (this layer) and `fs::mounts::mount` (path-level: resolving a device by label, and recording
//! where in a directory tree it answers for). That path-level layer -- `mount`/`umount` syscalls,
//! `/etc/fstab` -- is deferred (`plans/USERSPACE_PLAN.md`); this module is named `volume`, not
//! `mount`, specifically so the function that eventually implements *that* doesn't collide with
//! this one.

use hadris_fat::sync::{FatVolume, FatVolumeBuilder};

use super::blkio::BlkIo;
use super::rtc_time::RTC_TIME;
use crate::drivers::virtio::blk::{self, MAX_BLK};

/// One open volume per `blk` device, indexed the same way `blk::get` is.
static mut VOLUMES: [Option<FatVolume<BlkIo>>; MAX_BLK] = [const { None }; MAX_BLK];

/// Opens the FAT volume on `blk` device `dev`, stamping new and changed entries from the
/// real-time clock (UTC, not the FAT epoch). `false` if it holds no filesystem `hadris_fat` can
/// open.
pub fn open(dev: usize) -> bool {
    let io = BlkIo::new(blk::get(dev));
    let Ok(vol) = FatVolumeBuilder::new(io).time_provider(&RTC_TIME).open() else {
        return false;
    };
    // SAFETY: sole writer, and `get` is only ever called for a `dev` some earlier `open` call
    // already populated.
    #[allow(clippy::deref_addrof)]
    unsafe {
        (&mut *(&raw mut VOLUMES))[dev] = Some(vol)
    };
    true
}

/// Closes the volume on `dev` (`umount`): nothing refers to it any more, the caller having
/// checked that no file is open on it.
pub fn close(dev: usize) {
    // SAFETY: see `open`.
    #[allow(clippy::deref_addrof)]
    unsafe {
        (&mut *(&raw mut VOLUMES))[dev] = None
    };
}

/// The FAT volume open on `blk` device `dev`, if there is one (the mount table only ever sends a
/// path to a device that is mounted, so `None` is a bug or a stale index).
#[allow(clippy::deref_addrof)]
pub fn get(dev: usize) -> Option<&'static mut FatVolume<BlkIo>> {
    // SAFETY: see `open`.
    unsafe { (&mut *(&raw mut VOLUMES))[dev].as_mut() }
}
