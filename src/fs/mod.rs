//! Filesystem support: opening and reading/writing the FAT volumes on `blk`'s block devices.

pub mod sector;

/// Unix time to FAT's calendar fields, and `utimensat`'s timestamp requests -- pure arithmetic.
pub mod fattime;

/// The `blk`-backed byte-stream adapter `hadris_fat::FatVolume::open` needs -- hardware-only
/// (wraps `drivers::virtio::blk::Blk`), unlike `sector`'s pure arithmetic.
#[cfg(target_os = "none")]
pub mod blkio;

/// Opening `blk`'s block devices as FAT volumes through `blkio::BlkIo` -- volume-level only, no
/// path/namespace concept; see `volume`'s own doc comment for the layering.
#[cfg(target_os = "none")]
pub mod volume;

/// Resolving a `/`-separated path against one volume's root directory -- no mount table. `cwd`'s
/// `absolute` handles the working-directory half; everything here still deals in already-absolute
/// paths.
#[cfg(target_os = "none")]
pub mod path;

/// The kernel's one working directory, and resolving a path relative to it -- see its own doc
/// comment for why this is `fs`, not `shell`.
#[cfg(target_os = "none")]
pub mod cwd;

/// The real-time clock as the FAT volume's `TimeProvider`: new and modified entries get the
/// current UTC time instead of the FAT epoch.
#[cfg(target_os = "none")]
pub mod rtc_time;
