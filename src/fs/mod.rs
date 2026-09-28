//! Filesystem support: mounting and reading/writing the FAT volumes on `blk`'s block devices.

pub mod sector;

/// The `blk`-backed byte-stream adapter `hadris_fat::FatVolume::open` needs -- hardware-only
/// (wraps `drivers::virtio::blk::Blk`), unlike `sector`'s pure arithmetic.
#[cfg(target_os = "none")]
pub mod blkio;
