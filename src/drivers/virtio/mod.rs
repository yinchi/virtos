//! VirtIO devices, found via the `virtio,mmio` slots the device tree lists, sharing one HAL (the
//! DMA pool in `hal`).

pub mod blk;
pub mod hal;

use core::ptr::NonNull;

use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
use virtio_drivers::transport::{DeviceType, Transport};

/// Every discovered `virtio,mmio` slot holding a device of the requested `device_type`, as a
/// transport, in the order the slots were given. A slot that's empty or not a valid VirtIO
/// device is skipped silently.
fn mmio_transports(
    mmio_slots: impl Iterator<Item = (usize, usize)>,
    device_type: DeviceType,
) -> impl Iterator<Item = MmioTransport<'static>> {
    mmio_slots.filter_map(move |(base, size)| {
        let header = NonNull::new(base as *mut VirtIOHeader)?;
        // SAFETY: `base`/`size` came from a `virtio,mmio` node's own `reg` property, and the
        // caller mapped it (`mmu::map_device`) before this ever runs.
        let transport = unsafe { MmioTransport::new(header, size) }.ok()?; // empty slot, or not VirtIO
        (transport.device_type() == device_type).then_some(transport)
    })
}
