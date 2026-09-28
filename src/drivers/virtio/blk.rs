//! The VirtIO block devices `super::discover` finds among the `virtio,mmio` slots the device
//! tree lists.
//!
//! Uses `VirtIOBlk`'s plain, synchronous `read_blocks`/`write_blocks` -- which busy-polls the
//! virtqueue directly, no interrupt involved -- rather than its non-blocking submit/complete
//! pair (`read_blocks_nb`/`complete_read_blocks`). Both block the caller identically; the
//! interrupt-driven version only pays off once something else could usefully run while waiting
//! (a scheduler, which doesn't exist yet), so there's nothing to gain yet from the extra SPI
//! wiring and IRQ dispatch it would need.

use virtio_drivers::Error;
use virtio_drivers::device::blk::VirtIOBlk;
use virtio_drivers::transport::mmio::MmioTransport;

use super::hal::VirtioHalImpl;

/// The most block devices this kernel tracks.
pub const MAX_BLK: usize = 4;

/// Every discovered block device, in slot order. Only the first `BLK_COUNT` entries are valid.
static mut BLK: [Option<Blk>; MAX_BLK] = [const { None }; MAX_BLK];
/// How many of `BLK`'s entries `add` populated.
static mut BLK_COUNT: usize = 0;

/// Wrapper around a VirtIO block device.
pub struct Blk {
    inner: VirtIOBlk<VirtioHalImpl, MmioTransport<'static>>,
}

impl Blk {
    /// Reads one or more 512-byte sectors into `buf`.
    pub fn read_blocks(&mut self, block_id: usize, buf: &mut [u8]) -> Result<(), Error> {
        self.inner.read_blocks(block_id, buf)
    }

    /// Writes `buf` to one or more 512-byte sectors.
    pub fn write_blocks(&mut self, block_id: usize, buf: &[u8]) -> Result<(), Error> {
        self.inner.write_blocks(block_id, buf)
    }

    /// Device capacity in bytes (`VirtIOBlk::capacity` is in 512-byte sectors).
    pub fn capacity_bytes(&self) -> u64 {
        self.inner.capacity() * 512
    }
}

/// Claims a slot `super::discover` classified as a block device, up to `MAX_BLK`. Populates the
/// static `BLK` table that `get`/`count` read back; a device whose driver fails to initialize,
/// or arrives past `MAX_BLK`, is dropped like an empty slot.
#[allow(clippy::deref_addrof)]
pub(super) fn add(transport: MmioTransport<'static>) {
    // SAFETY: sole writer, called once per slot before `get`/`count` are ever reached.
    let (blk, count) = unsafe { (&mut *(&raw mut BLK), &mut *(&raw mut BLK_COUNT)) };
    if *count == MAX_BLK {
        return;
    }
    let Ok(inner) = VirtIOBlk::<VirtioHalImpl, _>::new(transport) else {
        return;
    };
    blk[*count] = Some(Blk { inner });
    *count += 1;
}

/// How many block devices `find_all` found.
#[allow(clippy::deref_addrof)]
pub fn count() -> usize {
    // SAFETY: read-only after `find_all` has run.
    unsafe { *(&raw const BLK_COUNT) }
}

/// Block device `dev` (an index below `count()`).
#[allow(clippy::deref_addrof)]
pub fn get(dev: usize) -> &'static mut Blk {
    // SAFETY: see BLK.
    unsafe { (&mut *(&raw mut BLK))[dev].as_mut().unwrap() }
}
