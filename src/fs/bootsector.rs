//! What a FAT boot sector says about the volume: its label and its volume ID. How the kernel tells the block devices
//! apart -- by what is on them, never by which virtio-mmio slot QEMU put them in (`fs/devices.rs` reads sector 0 of
//! each and calls `parse`).
//!
//! Both sit in the BIOS parameter block's "extended" part, at different offsets for FAT12/16 (label at 43, ID at 39)
//! and FAT32 (71 and 67); the two are told apart the way the Microsoft specification says, by `FATSz16` and
//! `RootEntCnt` both being zero for FAT32. Anything that does not look like a FAT boot sector -- a blank disk, random
//! bytes, exFAT, NTFS, a partition table -- is `None`. The ID is what Linux shows as the `UUID` of a vfat volume
//! (`XXXX-XXXX`, see `VolumeId`); the label is up to 11 bytes, trailing spaces trimmed. `mkfs.fat` writes `NO NAME` when
//! no label is given; that counts as none, as it does for `blkid`.
//!
//! Pure, so it is tested on the host.

use core::fmt;

/// The label field's width in the boot sector.
pub const LABEL_LEN: usize = 11;

/// What `mkfs.fat` writes as the label of a volume that has none.
const NO_LABEL: &[u8] = b"NO NAME";

/// The label of the volume that is the root (`/`).
pub const ROOT_LABEL: &str = "SYSTEM";

/// The volume ID, shown as `XXXX-XXXX` (high half first), upper-case hex -- Linux's `UUID` for a vfat volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeId(pub u32);

impl fmt::Display for VolumeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04X}-{:04X}", self.0 >> 16, self.0 & 0xffff)
    }
}

/// A FAT volume's identity, as its boot sector gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootInfo {
    label: [u8; LABEL_LEN],
    label_len: usize,
    pub volume_id: VolumeId,
}

impl BootInfo {
    /// The label's bytes, without the padding: empty for a volume with none.
    pub fn label(&self) -> &[u8] {
        &self.label[..self.label_len]
    }

    /// Whether the label is `name`. FAT labels are upper case by convention (`mkfs.fat -n` and `mlabel` both
    /// upper-case what they are given), so the comparison ignores ASCII case; an empty `name` matches nothing.
    pub fn label_is(&self, name: &str) -> bool {
        !name.is_empty() && self.label().eq_ignore_ascii_case(name.as_bytes())
    }
}

/// Why no device could be chosen as the root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootError {
    /// No device holds a FAT volume labelled `SYSTEM`.
    NoSystem,
    /// This many devices (more than one) do: a label is not a unique name, and guessing which one
    /// is the system could boot from -- and write to -- the wrong disk.
    Ambiguous(usize),
}

impl fmt::Display for RootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RootError::NoSystem => write!(f, "no FAT volume is labelled {ROOT_LABEL}"),
            RootError::Ambiguous(n) => write!(f, "{n} FAT volumes are labelled {ROOT_LABEL}"),
        }
    }
}

/// Which device is the root of the file tree, given each device's identity (`None` for one that is
/// not FAT), in device order: the one volume labelled `SYSTEM` (without regard to case). Exactly
/// one: none is `NoSystem`, and two or more are `Ambiguous` -- there is no fallback to "the first
/// FAT volume", which could be the HOME disk.
pub fn pick_root(devices: &[Option<BootInfo>]) -> Result<usize, RootError> {
    let mut systems = devices
        .iter()
        .enumerate()
        .filter(|(_, d)| d.is_some_and(|d| d.label_is(ROOT_LABEL)))
        .map(|(index, _)| index);
    match (systems.next(), systems.next()) {
        (Some(index), None) => Ok(index),
        (None, _) => Err(RootError::NoSystem),
        (Some(_), Some(_)) => Err(RootError::Ambiguous(2 + systems.count())),
    }
}

fn u16_at(sector: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([sector[at], sector[at + 1]])
}

/// Reads the identity out of sector 0 of a device, or `None` if it is not a FAT boot sector. `sector` is the
/// whole 512 bytes (a shorter slice is `None`).
pub fn parse(sector: &[u8]) -> Option<BootInfo> {
    if sector.len() < 512 || sector[510] != 0x55 || sector[511] != 0xaa {
        return None;
    }
    let bytes_per_sector = u16_at(sector, 11);
    let sectors_per_cluster = sector[13];
    let reserved_sectors = u16_at(sector, 14);
    let fats = sector[16];
    if !matches!(bytes_per_sector, 512 | 1024 | 2048 | 4096)
        || !sectors_per_cluster.is_power_of_two()
        || reserved_sectors == 0
        || fats == 0
    {
        return None;
    }
    let root_entries = u16_at(sector, 17);
    let fat_size_16 = u16_at(sector, 22);
    // FAT32 has no fixed root directory and keeps its FAT size in a wider field; the extended BPB moves with it.
    let (signature_at, id_at, label_at) = if fat_size_16 == 0 && root_entries == 0 {
        (66, 67, 71)
    } else {
        (38, 39, 43)
    };
    // 0x29: ID, label and file-system type follow. 0x28: only the ID (an older layout); no label then.
    let has_label = match sector[signature_at] {
        0x29 => true,
        0x28 => false,
        _ => return None,
    };
    let volume_id = VolumeId(u32::from_le_bytes(
        sector[id_at..id_at + 4].try_into().unwrap(),
    ));

    let mut label = [0u8; LABEL_LEN];
    let mut label_len = 0;
    if has_label {
        label.copy_from_slice(&sector[label_at..label_at + LABEL_LEN]);
        label_len = label
            .iter()
            .rposition(|&b| b != b' ' && b != 0)
            .map_or(0, |last| last + 1);
        if &label[..label_len] == NO_LABEL {
            label_len = 0;
        }
    }
    Some(BootInfo {
        label,
        label_len,
        volume_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    /// A FAT12/16 boot sector with the given label and ID.
    fn fat16(label: &[u8; 11], id: u32) -> [u8; 512] {
        let mut s = [0u8; 512];
        s[0] = 0xeb;
        s[11..13].copy_from_slice(&512u16.to_le_bytes());
        s[13] = 4; // sectors per cluster
        s[14..16].copy_from_slice(&1u16.to_le_bytes());
        s[16] = 2; // FATs
        s[17..19].copy_from_slice(&512u16.to_le_bytes()); // root entries
        s[22..24].copy_from_slice(&256u16.to_le_bytes()); // FATSz16
        s[38] = 0x29;
        s[39..43].copy_from_slice(&id.to_le_bytes());
        s[43..54].copy_from_slice(label);
        s[510] = 0x55;
        s[511] = 0xaa;
        s
    }

    /// A FAT32 boot sector: no fixed root, everything after it shifted 28 bytes.
    fn fat32(label: &[u8; 11], id: u32) -> [u8; 512] {
        let mut s = [0u8; 512];
        s[0] = 0xeb;
        s[11..13].copy_from_slice(&512u16.to_le_bytes());
        s[13] = 8;
        s[14..16].copy_from_slice(&32u16.to_le_bytes());
        s[16] = 2;
        s[66] = 0x29;
        s[67..71].copy_from_slice(&id.to_le_bytes());
        s[71..82].copy_from_slice(label);
        s[510] = 0x55;
        s[511] = 0xaa;
        s
    }

    #[test]
    fn fat16_label_and_id() {
        let info = parse(&fat16(b"HOME       ", 0x5e6f_7a8b)).unwrap();
        assert_eq!(info.label(), b"HOME");
        assert_eq!(info.volume_id, VolumeId(0x5e6f_7a8b));
        assert_eq!(format!("{}", info.volume_id), "5E6F-7A8B");
    }

    #[test]
    fn fat32_label_and_id() {
        let info = parse(&fat32(b"SYSTEM     ", 0x1a2b_3c4d)).unwrap();
        assert_eq!(info.label(), b"SYSTEM");
        assert_eq!(format!("{}", info.volume_id), "1A2B-3C4D");
    }

    #[test]
    fn the_two_layouts_are_told_apart_by_the_root_entries() {
        // The FAT32 sector read as FAT16 would find its label in the wrong place; a wrong guess shows as no label.
        let mut s = fat32(b"SYSTEM     ", 1);
        s[17..19].copy_from_slice(&512u16.to_le_bytes()); // now it claims a fixed root: FAT16 layout
        assert_eq!(parse(&s), None); // its signature byte is not where FAT16 keeps it
    }

    #[test]
    fn a_full_width_label_keeps_all_eleven() {
        assert_eq!(
            parse(&fat16(b"ABCDEFGHIJK", 0)).unwrap().label(),
            b"ABCDEFGHIJK"
        );
    }

    #[test]
    fn spaces_inside_a_label_stay() {
        assert_eq!(
            parse(&fat16(b"MY DISK    ", 0)).unwrap().label(),
            b"MY DISK"
        );
    }

    #[test]
    fn no_name_means_no_label() {
        let info = parse(&fat16(b"NO NAME    ", 7)).unwrap();
        assert_eq!(info.label(), b"");
        assert!(!info.label_is("NO NAME"));
    }

    #[test]
    fn nul_padding_is_trimmed_too() {
        let mut label = [0u8; 11];
        label[..4].copy_from_slice(b"HOME");
        assert_eq!(parse(&fat16(&label, 0)).unwrap().label(), b"HOME");
    }

    #[test]
    fn label_matching_ignores_case_and_never_matches_empty() {
        let info = parse(&fat16(b"HOME       ", 0)).unwrap();
        assert!(info.label_is("HOME"));
        assert!(info.label_is("home"));
        assert!(!info.label_is("HOM"));
        assert!(!info.label_is("HOME2"));
        assert!(!info.label_is(""));
        let none = parse(&fat16(b"           ", 0)).unwrap();
        assert!(!none.label_is(""));
    }

    #[test]
    fn the_short_signature_byte_gives_an_id_and_no_label() {
        let mut s = fat16(b"HOME       ", 0x0000_0001);
        s[38] = 0x28;
        let info = parse(&s).unwrap();
        assert_eq!(info.label(), b"");
        assert_eq!(info.volume_id, VolumeId(1));
    }

    #[test]
    fn the_root_is_the_system_volume_wherever_it_is() {
        let home = parse(&fat16(b"HOME       ", 1));
        let system = parse(&fat16(b"SYSTEM     ", 2));
        assert_eq!(pick_root(&[home, system]), Ok(1));
        assert_eq!(pick_root(&[system, home]), Ok(0));
        assert_eq!(pick_root(&[None, system]), Ok(1));
    }

    #[test]
    fn without_a_system_volume_there_is_no_root() {
        let old = parse(&fat16(b"OLDDISK    ", 0));
        let home = parse(&fat16(b"HOME       ", 1));
        // Never "the first FAT volume": that could be the HOME disk.
        assert_eq!(pick_root(&[old]), Err(RootError::NoSystem));
        assert_eq!(pick_root(&[None, home, old]), Err(RootError::NoSystem));
        assert_eq!(pick_root(&[None, None]), Err(RootError::NoSystem));
        assert_eq!(pick_root(&[]), Err(RootError::NoSystem));
        assert_eq!(
            pick_root(&[parse(&fat16(b"NO NAME    ", 3))]),
            Err(RootError::NoSystem)
        );
    }

    #[test]
    fn two_system_volumes_are_ambiguous() {
        let a = parse(&fat16(b"SYSTEM     ", 1));
        let b = parse(&fat16(b"system     ", 2)); // the label's case does not matter
        let home = parse(&fat16(b"HOME       ", 3));
        assert_eq!(pick_root(&[a, b]), Err(RootError::Ambiguous(2)));
        assert_eq!(pick_root(&[a, home, b, a]), Err(RootError::Ambiguous(3)));
    }

    #[test]
    fn the_error_says_which_problem_it_is() {
        assert_eq!(
            alloc::format!("{}", RootError::NoSystem),
            "no FAT volume is labelled SYSTEM"
        );
        assert_eq!(
            alloc::format!("{}", RootError::Ambiguous(2)),
            "2 FAT volumes are labelled SYSTEM"
        );
    }

    #[test]
    fn the_id_is_zero_padded() {
        assert_eq!(format!("{}", VolumeId(0)), "0000-0000");
        assert_eq!(format!("{}", VolumeId(0x0000_00ab)), "0000-00AB");
    }

    #[test]
    fn things_that_are_not_fat() {
        assert_eq!(parse(&[0u8; 512]), None); // blank
        assert_eq!(parse(&[0xa5u8; 512]), None); // noise
        assert_eq!(parse(&fat16(b"HOME       ", 0)[..511]), None); // short
        let mut no_signature = fat16(b"HOME       ", 0);
        no_signature[511] = 0;
        assert_eq!(parse(&no_signature), None);
        let mut odd_sector_size = fat16(b"HOME       ", 0);
        odd_sector_size[11..13].copy_from_slice(&500u16.to_le_bytes());
        assert_eq!(parse(&odd_sector_size), None);
        let mut no_fats = fat16(b"HOME       ", 0);
        no_fats[16] = 0; // NTFS looks like this
        assert_eq!(parse(&no_fats), None);
        let mut cluster_not_power_of_two = fat16(b"HOME       ", 0);
        cluster_not_power_of_two[13] = 3;
        assert_eq!(parse(&cluster_not_power_of_two), None);
    }
}
