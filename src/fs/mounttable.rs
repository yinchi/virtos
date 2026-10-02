//! The mount table: which volume answers for which part of the file tree, and how a mount's source is written.
//!
//! Paths reach the filesystem already absolute and normalized (`fs::path::abspath`): `/`, or `/` and components,
//! with no `.`, `..`, empty component or trailing `/`. A `MountTable` is a list of `(mount point, device)`, the root
//! (`/`) first; `resolve` picks the mount whose point is the longest match of a path on a component boundary and hands
//! back the rest of the path for that volume to look up -- so `/root/a` on a volume mounted at `/root` is `/a` there, and
//! `/rootbeer` is not under it. There is no special case for `..`: it is gone before a path gets here, and going up
//! from a mount's root is going up in the path.
//!
//! A `Source` is what follows `mount` or begins an `fstab` line: `LABEL=name` or `UUID=XXXX-XXXX`, naming a device by
//! what its FAT boot sector says (`bootsector`).
//!
//! Pure `no_std` + `alloc`, so it is tested on the host.

use alloc::string::String;
use alloc::vec::Vec;

use super::bootsector::BootInfo;

/// Why the table refused a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountError {
    /// The point already has something mounted on it, or the device is mounted somewhere already.
    Busy,
    /// `remove` was asked about a path that is not a mount point.
    NotMounted,
    /// `remove` was asked about the root, or about a mount with another mounted inside it.
    InUse,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    pub point: String,
    pub dev: usize,
}

/// The mounts, in the order they were made (the root first).
#[derive(Debug, Default)]
pub struct MountTable {
    mounts: Vec<Mount>,
}

/// Whether `path` is `point` or lies under it: a match on a whole component, so `/root` covers `/root` and `/root/a`
/// but not `/rootbeer`. The root (`/`) covers everything.
pub fn is_within(point: &str, path: &str) -> bool {
    point == "/"
        || path == point
        || (path.starts_with(point) && path[point.len()..].starts_with('/'))
}

impl MountTable {
    pub fn new() -> Self {
        Self { mounts: Vec::new() }
    }

    /// Mounts device `dev` at `point` (an absolute, normalized path). `Busy` if something is mounted at `point`, if
    /// `dev` is mounted already (a volume is mounted once), or if a mount lies inside `point`: covering it would leave a
    /// mount reachable through a directory that no longer shows it.
    pub fn add(&mut self, point: &str, dev: usize) -> Result<(), MountError> {
        if self
            .mounts
            .iter()
            .any(|m| m.dev == dev || is_within(point, &m.point))
        {
            return Err(MountError::Busy);
        }
        self.mounts.push(Mount {
            point: String::from(point),
            dev,
        });
        Ok(())
    }

    /// Unmounts what is mounted exactly at `point`, returning its device. `NotMounted` if nothing is; `InUse` for the
    /// root and for a mount with another mount inside it (which has to go first).
    pub fn remove(&mut self, point: &str) -> Result<usize, MountError> {
        let Some(index) = self.mounts.iter().position(|m| m.point == point) else {
            return Err(MountError::NotMounted);
        };
        if point == "/"
            || self
                .mounts
                .iter()
                .any(|m| m.point != point && is_within(point, &m.point))
        {
            return Err(MountError::InUse);
        }
        Ok(self.mounts.remove(index).dev)
    }

    /// The device that answers for `path`, and what remains of the path for that volume: `""` for the volume's own
    /// root, else a path starting with `/`. The table needs a root mounted; before that, everything is device 0.
    pub fn resolve<'a>(&self, path: &'a str) -> (usize, &'a str) {
        let mut best: Option<&Mount> = None;
        for mount in &self.mounts {
            if is_within(&mount.point, path)
                && best.is_none_or(|b| mount.point.len() > b.point.len())
            {
                best = Some(mount);
            }
        }
        match best {
            Some(mount) if mount.point == "/" => (mount.dev, if path == "/" { "" } else { path }),
            Some(mount) => (mount.dev, &path[mount.point.len()..]),
            None => (0, path),
        }
    }

    /// Whether a mount point is `path` or lies inside it (so a mount at `path` would be refused).
    pub fn covers_a_mount(&self, path: &str) -> bool {
        self.mounts.iter().any(|m| is_within(path, &m.point))
    }

    /// Whether `path` is exactly a mount point (the root included).
    pub fn is_mount_point(&self, path: &str) -> bool {
        self.mounts.iter().any(|m| m.point == path)
    }

    /// Where device `dev` is mounted, if it is.
    pub fn point_of(&self, dev: usize) -> Option<&str> {
        self.mounts
            .iter()
            .find(|m| m.dev == dev)
            .map(|m| m.point.as_str())
    }
}

/// How a mount names the volume it wants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// `LABEL=name`: the volume label, compared without regard to ASCII case (FAT labels are upper case).
    Label(String),
    /// `UUID=XXXX-XXXX`: the FAT volume ID.
    Uuid(u32),
}

impl Source {
    /// Reads a source. `None` if it is neither spelling, or the name or ID is empty or malformed. The keywords are upper
    /// case only, as in `fstab`; a UUID is two groups of four hex digits (either case) around a dash.
    pub fn parse(text: &str) -> Option<Self> {
        if let Some(name) = text.strip_prefix("LABEL=") {
            return (!name.is_empty()).then(|| Source::Label(String::from(name)));
        }
        let id = text.strip_prefix("UUID=")?;
        let (high, low) = id.split_once('-')?;
        if high.len() != 4
            || low.len() != 4
            || !high
                .chars()
                .chain(low.chars())
                .all(|c| c.is_ascii_hexdigit())
        {
            return None;
        }
        Some(Source::Uuid(
            u32::from_str_radix(high, 16).ok()? << 16 | u32::from_str_radix(low, 16).ok()?,
        ))
    }

    /// Whether a device whose boot sector says `info` is the one this names.
    pub fn matches(&self, info: &BootInfo) -> bool {
        match self {
            Source::Label(name) => info.label_is(name),
            Source::Uuid(id) => info.volume_id.0 == *id,
        }
    }

    /// The first device (by index) `self` matches among `devices` (`None` for a device that is not FAT).
    pub fn find(&self, devices: &[Option<BootInfo>]) -> Option<usize> {
        devices
            .iter()
            .position(|d| d.as_ref().is_some_and(|info| self.matches(info)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::bootsector;

    fn table() -> MountTable {
        let mut t = MountTable::new();
        t.add("/", 0).unwrap();
        t
    }

    #[test]
    fn everything_is_on_the_root_until_something_is_mounted() {
        let t = table();
        assert_eq!(t.resolve("/"), (0, ""));
        assert_eq!(t.resolve("/bin/cat"), (0, "/bin/cat"));
    }

    #[test]
    fn a_mount_answers_for_its_point_and_what_is_under_it() {
        let mut t = table();
        t.add("/root", 1).unwrap();
        assert_eq!(t.resolve("/root"), (1, ""));
        assert_eq!(t.resolve("/root/a"), (1, "/a"));
        assert_eq!(t.resolve("/root/a/b"), (1, "/a/b"));
        assert_eq!(t.resolve("/bin"), (0, "/bin"));
    }

    #[test]
    fn a_point_covers_whole_components_only() {
        let mut t = table();
        t.add("/root", 1).unwrap();
        assert_eq!(t.resolve("/rootbeer"), (0, "/rootbeer"));
        assert_eq!(t.resolve("/roo"), (0, "/roo"));
        assert!(!is_within("/root", "/rootbeer"));
        assert!(is_within("/root", "/root/x"));
        assert!(is_within("/root", "/root"));
        assert!(is_within("/", "/anything"));
    }

    #[test]
    fn the_longest_match_wins() {
        let mut t = table();
        t.add("/a", 1).unwrap(); // (an outer mount cannot be added after an inner one: `a_mount_cannot_cover_another_mount`)
        t.add("/a/b", 2).unwrap();
        assert_eq!(t.resolve("/a/b/c"), (2, "/c"));
        assert_eq!(t.resolve("/a/b"), (2, ""));
        assert_eq!(t.resolve("/a/x"), (1, "/x"));
        assert_eq!(t.resolve("/a"), (1, ""));
    }

    #[test]
    fn a_mount_point_and_a_device_are_used_once() {
        let mut t = table();
        t.add("/mnt", 1).unwrap();
        assert_eq!(t.add("/mnt", 2), Err(MountError::Busy)); // something is there
        assert_eq!(t.add("/other", 1), Err(MountError::Busy)); // that volume is mounted
        assert_eq!(t.add("/", 3), Err(MountError::Busy));
    }

    #[test]
    fn a_mount_cannot_cover_another_mount() {
        let mut t = table();
        t.add("/mnt/sub", 1).unwrap();
        assert_eq!(t.add("/mnt", 2), Err(MountError::Busy));
        assert_eq!(t.add("/mnt/sub/deeper", 2), Ok(())); // inside one is fine
        assert_eq!(t.add("/mnt/other", 3), Ok(())); // and so is a sibling
    }

    #[test]
    fn unmounting_returns_the_device_and_uncovers_the_directory() {
        let mut t = table();
        t.add("/mnt", 1).unwrap();
        assert_eq!(t.remove("/mnt"), Ok(1));
        assert_eq!(t.resolve("/mnt/x"), (0, "/mnt/x"));
        assert_eq!(t.point_of(1), None);
        t.add("/mnt", 1).unwrap(); // and it can be mounted again
    }

    #[test]
    fn what_cannot_be_unmounted() {
        let mut t = table();
        t.add("/mnt", 1).unwrap();
        t.add("/mnt/sub", 2).unwrap();
        assert_eq!(t.remove("/mnt"), Err(MountError::InUse)); // another mount inside it
        assert_eq!(t.remove("/"), Err(MountError::InUse));
        assert_eq!(t.remove("/mnt/sub/x"), Err(MountError::NotMounted));
        assert_eq!(t.remove("/nowhere"), Err(MountError::NotMounted));
        assert_eq!(t.remove("/mnt/sub"), Ok(2));
        assert_eq!(t.remove("/mnt"), Ok(1)); // inside-out works
    }

    #[test]
    fn asking_about_mounts() {
        let mut t = table();
        t.add("/root", 1).unwrap();
        assert!(t.is_mount_point("/"));
        assert!(t.is_mount_point("/root"));
        assert!(!t.is_mount_point("/root/a"));
        assert!(!t.is_mount_point("/bin"));
        assert_eq!(t.point_of(0), Some("/"));
        assert_eq!(t.point_of(1), Some("/root"));
        assert_eq!(t.point_of(2), None);
    }

    #[test]
    fn sources_are_read_like_fstab_writes_them() {
        assert_eq!(
            Source::parse("LABEL=HOME"),
            Some(Source::Label(String::from("HOME")))
        );
        assert_eq!(
            Source::parse("LABEL=my disk"),
            Some(Source::Label(String::from("my disk")))
        );
        assert_eq!(
            Source::parse("UUID=5E6F-7A8B"),
            Some(Source::Uuid(0x5e6f_7a8b))
        );
        assert_eq!(
            Source::parse("UUID=5e6f-7a8b"),
            Some(Source::Uuid(0x5e6f_7a8b))
        );
        assert_eq!(Source::parse("UUID=0000-0000"), Some(Source::Uuid(0)));
    }

    #[test]
    fn what_is_not_a_source() {
        for bad in [
            "",
            "HOME",
            "LABEL=",
            "label=HOME",
            "UUID=",
            "UUID=5E6F7A8B",
            "UUID=5E6-7A8B",
            "UUID=5E6F-7A8",
            "UUID=5E6F-7A8G",
            "UUID=+E6F-7A8B",
            "vdb",
            "/dev/vdb",
            "LABEL HOME",
        ] {
            assert_eq!(Source::parse(bad), None, "{bad:?}");
        }
    }

    fn fat(label: &[u8; 11], id: u32) -> Option<BootInfo> {
        let mut s = [0u8; 512];
        s[0] = 0xeb;
        s[11..13].copy_from_slice(&512u16.to_le_bytes());
        s[13] = 4;
        s[14..16].copy_from_slice(&1u16.to_le_bytes());
        s[16] = 2;
        s[17..19].copy_from_slice(&512u16.to_le_bytes());
        s[22..24].copy_from_slice(&256u16.to_le_bytes());
        s[38] = 0x29;
        s[39..43].copy_from_slice(&id.to_le_bytes());
        s[43..54].copy_from_slice(label);
        s[510] = 0x55;
        s[511] = 0xaa;
        bootsector::parse(&s)
    }

    #[test]
    fn a_source_finds_its_device_by_what_is_on_it() {
        let devices = [
            fat(b"HOME       ", 1),
            None,
            fat(b"SYSTEM     ", 2),
            fat(b"HOME       ", 3),
        ];
        assert_eq!(
            Source::Label(String::from("SYSTEM")).find(&devices),
            Some(2)
        );
        assert_eq!(Source::Label(String::from("home")).find(&devices), Some(0)); // the first of two, ignoring case
        assert_eq!(Source::Uuid(3).find(&devices), Some(3)); // the ID tells the two apart
        assert_eq!(Source::Label(String::from("NOPE")).find(&devices), None);
        assert_eq!(Source::Uuid(9).find(&devices), None);
        assert_eq!(Source::Label(String::from("HOME")).find(&[]), None);
    }
}
