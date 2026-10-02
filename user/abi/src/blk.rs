//! What `SYS_BLKINFO` writes for a block device: a fixed 288-byte record, little-endian, at the user pointer.
//!
//! | offset | size | field |
//! |---|---|---|
//! | 0 | 8 | `capacity`: the device's size in bytes |
//! | 8 | 4 | `flags`: `BLK_FAT`, `BLK_ROOT` |
//! | 12 | 4 | `volume_id`: the FAT volume ID (0 when not `BLK_FAT`) |
//! | 16 | 1 | `label_len`: 0-11 |
//! | 17 | 11 | `label`: the first `label_len` bytes are the volume label (no padding) |
//! | 28 | 2 | `mount_len`: 0 if the device is not mounted, else the length of its mount point |
//! | 30 | 2 | reserved, zero |
//! | 32 | 256 | `mount`: the first `mount_len` bytes are where the volume is mounted (an absolute path) |
//!
//! `BlkInfo` is the decoded form; `encode` and `decode` are the two directions of that layout, so the kernel and
//! `userlib` cannot disagree about it.

/// The longest mount point the kernel accepts (and so the most `blkinfo` can report).
pub const BLK_MOUNT_MAX: usize = 256;

/// Size of the record `SYS_BLKINFO` writes.
pub const BLKINFO_SIZE: usize = 32 + BLK_MOUNT_MAX;

/// The device's first sector is a FAT boot sector: `volume_id` and `label` mean something.
pub const BLK_FAT: u32 = 1;
/// The device is the root of the file tree (`/`).
pub const BLK_ROOT: u32 = 2;

/// The label field's width.
pub const BLK_LABEL_MAX: usize = 11;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlkInfo {
    pub capacity: u64,
    pub flags: u32,
    pub volume_id: u32,
    pub label_len: u8,
    pub label: [u8; BLK_LABEL_MAX],
    pub mount_len: u16,
    pub mount: [u8; BLK_MOUNT_MAX],
}

impl BlkInfo {
    /// The label, without padding.
    pub fn label(&self) -> &[u8] {
        &self.label[..usize::from(self.label_len).min(BLK_LABEL_MAX)]
    }

    /// Where the device is mounted, without padding: empty if it is not.
    pub fn mount(&self) -> &[u8] {
        &self.mount[..usize::from(self.mount_len).min(BLK_MOUNT_MAX)]
    }

    pub fn encode(&self) -> [u8; BLKINFO_SIZE] {
        let mut out = [0u8; BLKINFO_SIZE];
        out[0..8].copy_from_slice(&self.capacity.to_le_bytes());
        out[8..12].copy_from_slice(&self.flags.to_le_bytes());
        out[12..16].copy_from_slice(&self.volume_id.to_le_bytes());
        out[16] = self.label_len;
        out[17..28].copy_from_slice(&self.label);
        out[28..30].copy_from_slice(&self.mount_len.to_le_bytes());
        out[32..].copy_from_slice(&self.mount);
        out
    }

    pub fn decode(bytes: &[u8; BLKINFO_SIZE]) -> Self {
        let mut label = [0u8; BLK_LABEL_MAX];
        label.copy_from_slice(&bytes[17..28]);
        let mut mount = [0u8; BLK_MOUNT_MAX];
        mount.copy_from_slice(&bytes[32..]);
        Self {
            capacity: u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
            flags: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            volume_id: u32::from_le_bytes(bytes[12..16].try_into().unwrap()),
            label_len: bytes[16],
            label,
            mount_len: u16::from_le_bytes([bytes[28], bytes[29]]),
            mount,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> BlkInfo {
        let mut label = [0u8; BLK_LABEL_MAX];
        label[..4].copy_from_slice(b"HOME");
        let mut mount = [0u8; BLK_MOUNT_MAX];
        mount[..5].copy_from_slice(b"/root");
        BlkInfo {
            capacity: 1 << 20,
            flags: BLK_FAT,
            volume_id: 0x5e6f_7a8b,
            label_len: 4,
            label,
            mount_len: 5,
            mount,
        }
    }

    #[test]
    fn round_trips() {
        assert_eq!(BlkInfo::decode(&sample().encode()), sample());
    }

    #[test]
    fn the_layout_is_the_documented_one() {
        let bytes = sample().encode();
        assert_eq!(&bytes[0..8], &(1u64 << 20).to_le_bytes());
        assert_eq!(&bytes[8..12], &[1, 0, 0, 0]);
        assert_eq!(&bytes[12..16], &[0x8b, 0x7a, 0x6f, 0x5e]);
        assert_eq!(bytes[16], 4);
        assert_eq!(&bytes[17..21], b"HOME");
        assert_eq!(&bytes[28..32], &[5, 0, 0, 0]);
        assert_eq!(&bytes[32..37], b"/root");
        assert_eq!(bytes.len(), 288);
    }

    #[test]
    fn label_is_cut_at_its_length_even_if_the_length_is_wild() {
        assert_eq!(sample().label(), b"HOME");
        let mut wild = sample();
        wild.label_len = 200;
        assert_eq!(wild.label().len(), BLK_LABEL_MAX);
        wild.mount_len = 60000;
        assert_eq!(wild.mount().len(), BLK_MOUNT_MAX);
        assert_eq!(sample().mount(), b"/root");
    }
}
