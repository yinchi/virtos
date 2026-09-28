//! VirtIO devices, found via the `virtio,mmio` slots the device tree lists, sharing one HAL (the
//! DMA pool in `hal`).

pub mod blk;
pub mod gpu;
pub mod hal;

use core::ptr::NonNull;

use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
use virtio_drivers::transport::{DeviceType, Transport};

/// Classifies every `virtio,mmio` slot and hands each one straight to its driver -- one pass,
/// each slot probed exactly once. Probing a slot again after a driver has already claimed it
/// would construct a second `MmioTransport` aliasing the same live device: harmless-looking
/// (`MmioTransport::new` only reads header fields), but caused exactly the kind of hang this
/// avoids -- discovering everything up front, before any device's first real use, means every
/// later device's discovery would otherwise re-probe every earlier one's slot too. A slot that's
/// empty, not a valid VirtIO device, of no type anything here drives, or whose driver fails to
/// initialize is skipped silently.
pub fn discover(mmio_slots: impl Iterator<Item = (usize, usize)>) {
    for (base, size) in mmio_slots {
        let Some(header) = NonNull::new(base as *mut VirtIOHeader) else {
            continue;
        };
        // SAFETY: `base`/`size` came from a `virtio,mmio` node's own `reg` property, and the
        // caller mapped it (`mmu::map_device`) before this ever runs.
        let Ok(transport) = (unsafe { MmioTransport::new(header, size) }) else {
            continue; // empty slot, or not VirtIO
        };
        match transport.device_type() {
            DeviceType::Block => blk::add(transport),
            DeviceType::GPU => gpu::set(transport),
            _ => {}
        }
    }
}
