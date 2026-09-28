//! Filesystem support: opening and reading/writing the FAT volumes on `blk`'s block devices.

pub mod sector;

/// The `blk`-backed byte-stream adapter `hadris_fat::FatVolume::open` needs -- hardware-only
/// (wraps `drivers::virtio::blk::Blk`), unlike `sector`'s pure arithmetic.
#[cfg(target_os = "none")]
pub mod blkio;

/// Opening `blk`'s block devices as FAT volumes through `blkio::BlkIo` -- volume-level only, no
/// path/namespace concept; see `volume`'s own doc comment for the layering.
#[cfg(target_os = "none")]
pub mod volume;
