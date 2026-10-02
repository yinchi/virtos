//! The block devices the kernel found, and what is on each: every device's first sector is read
//! once at boot and parsed by `bootsector` into a label and a volume ID, so a device is named by
//! its contents (never by which virtio-mmio slot it sits in -- QEMU's order is not the command
//! line's). One of them is the root of the file tree, chosen by `bootsector::pick_root`; the rest
//! are found and described but not opened as filesystems until they are mounted (`mounts`).
//! `blkinfo` reports a device to a program.

use alloc::vec::Vec;
use core::fmt::Write;

use abi::blk::{BLK_FAT, BLK_LABEL_MAX, BLK_MOUNT_MAX, BLK_ROOT, BlkInfo};

use super::bootsector::{self, BootInfo};
use crate::drivers::virtio::blk;

/// A block device: its size, and its boot-sector identity if it holds a FAT volume.
struct Device {
    capacity: u64,
    boot: Option<BootInfo>,
}

struct Devices {
    list: Vec<Device>,
    root: usize,
}

/// Every device found, and the root among them; set once by `probe` at boot.
static mut DEVICES: Option<Devices> = None;

/// The device table, or `None` before `probe` has run.
fn table() -> Option<&'static Devices> {
    // SAFETY: written once by `probe` at boot, before anything that reads it can run, and never
    // after.
    #[allow(clippy::deref_addrof)]
    unsafe {
        (*(&raw const DEVICES)).as_ref()
    }
}

/// Reads sector 0 of every block device, works out the root and stores the table. Returns the
/// root's index, or `None` if no device holds a FAT volume. The table is written to `log` (the
/// serial log).
pub fn probe(log: &mut impl Write) -> Option<usize> {
    let count = blk::count();
    let mut list = Vec::with_capacity(count);
    for dev in 0..count {
        let device = blk::get(dev);
        let capacity = device.capacity_bytes();
        let mut sector = [0u8; 512];
        // A device too small to have a boot sector, or one whose read fails, is simply "not FAT".
        let boot = if capacity >= 512 && device.read_blocks(0, &mut sector).is_ok() {
            bootsector::parse(&sector)
        } else {
            None
        };
        list.push(Device { capacity, boot });
    }
    let identities: Vec<Option<BootInfo>> = list.iter().map(|d| d.boot).collect();
    let root = bootsector::pick_root(&identities);

    let _ = writeln!(log, "Block devices: {count}");
    for (index, device) in list.iter().enumerate() {
        let size = size_text(device.capacity);
        match device.boot {
            Some(info) => {
                let label = core::str::from_utf8(info.label()).unwrap_or("?");
                let note = match root {
                    Some((r, true)) if r == index => "  (root)",
                    Some((r, false)) if r == index => {
                        "  (root: no SYSTEM volume, using the first FAT volume)"
                    }
                    _ => "",
                };
                let _ = writeln!(
                    log,
                    "  {index}: {size:>7}  {label:<width$}  {}{note}",
                    info.volume_id,
                    width = BLK_LABEL_MAX
                );
            }
            None => {
                let _ = writeln!(log, "  {index}: {size:>7}  not a FAT volume -- ignored");
            }
        }
    }

    let root_index = root.map(|(index, _)| index);
    // SAFETY: sole write to DEVICES, at boot (see `table`).
    unsafe {
        DEVICES = Some(Devices {
            list,
            root: root_index.unwrap_or(0),
        });
    }
    root_index
}

/// A size in the binary units a disk image is made in: `64 MiB`, `512 KiB`, or plain bytes for the
/// odd size.
fn size_text(bytes: u64) -> alloc::string::String {
    use alloc::format;
    if bytes >= 1 << 20 && bytes.is_multiple_of(1 << 20) {
        format!("{} MiB", bytes >> 20)
    } else if bytes >= 1 << 10 && bytes.is_multiple_of(1 << 10) {
        format!("{} KiB", bytes >> 10)
    } else {
        format!("{bytes} B")
    }
}

/// What `SYS_BLKINFO` reports for device `index`: `None` past the last device.
pub fn blkinfo(index: usize) -> Option<BlkInfo> {
    let devices = table()?;
    let device = devices.list.get(index)?;
    let mut info = BlkInfo {
        capacity: device.capacity,
        flags: 0,
        volume_id: 0,
        label_len: 0,
        label: [0; BLK_LABEL_MAX],
        mount_len: 0,
        mount: [0; BLK_MOUNT_MAX],
    };
    if let Some(boot) = device.boot {
        info.flags |= BLK_FAT;
        info.volume_id = boot.volume_id.0;
        info.label_len = u8::try_from(boot.label().len()).unwrap_or(0);
        info.label[..boot.label().len()].copy_from_slice(boot.label());
    }
    if index == devices.root {
        info.flags |= BLK_ROOT;
    }
    if let Some(point) = super::mounts::table().point_of(index) {
        let point = point.as_bytes();
        let n = point.len().min(BLK_MOUNT_MAX); // `mount` refuses a longer one
        info.mount[..n].copy_from_slice(&point[..n]);
        info.mount_len = u16::try_from(n).unwrap_or(0);
    }
    Some(info)
}

/// What each device holds, in device order: its boot-sector identity, `None` for one that is not
/// a FAT volume. What a mount source is matched against.
pub fn identities() -> Vec<Option<BootInfo>> {
    table().map_or_else(Vec::new, |t| t.list.iter().map(|d| d.boot).collect())
}
