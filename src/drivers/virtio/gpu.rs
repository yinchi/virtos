//! The VirtIO GPU device `super::discover` finds among the `virtio,mmio` slots the device tree
//! lists, exposing its pixel buffer.
//!
//! Not interrupt-based: `VirtIOGpu` only exposes blocking, internally-polling calls (`flush`
//! included), so there's no interrupt to wait for -- unlike `blk.rs`, there was never a choice to
//! make here.

use virtio_drivers::device::gpu::VirtIOGpu;
use virtio_drivers::transport::mmio::MmioTransport;

use super::hal::VirtioHalImpl;

/// The fixed resolution negotiated with the device.
pub const WIDTH: u32 = 640;
pub const HEIGHT: u32 = 480;

/// The device's pixel buffer: a BGRX8888 surface, `HEIGHT` rows of `stride` bytes.
pub struct FramebufferInfo {
    pub ptr: *mut u8,
    pub width: usize,
    pub height: usize,
    pub stride: usize,
}

pub struct Gpu {
    inner: VirtIOGpu<VirtioHalImpl, MmioTransport<'static>>,
}

/// The GPU, if one was found. At most one exists on this platform.
static mut GPU: Option<Gpu> = None;

impl Gpu {
    /// Negotiates this module's fixed resolution and returns the device's DMA-backed pixel
    /// buffer -- kept alive for as long as `self` is, since the backing allocation lives inside
    /// `self.inner`.
    pub fn framebuffer(&mut self) -> FramebufferInfo {
        let buf = self
            .inner
            .change_resolution(WIDTH, HEIGHT)
            .expect("failed to set up the virtio-gpu framebuffer");
        FramebufferInfo {
            ptr: buf.as_mut_ptr(),
            width: WIDTH as usize,
            height: HEIGHT as usize,
            stride: WIDTH as usize * 4,
        }
    }

    /// Pushes whatever's currently in the framebuffer to the actual display.
    pub fn flush(&mut self) {
        self.inner
            .flush()
            .expect("failed to flush the virtio-gpu framebuffer");
    }
}

/// Claims the slot `super::discover` classified as the GPU. A device whose driver fails to
/// initialize is treated as absent.
pub(super) fn set(transport: MmioTransport<'static>) {
    let Ok(inner) = VirtIOGpu::new(transport) else {
        return;
    };
    // SAFETY: sole writer, called once before `get` is ever reached.
    unsafe { GPU = Some(Gpu { inner }) };
}

/// The GPU, if `find` found one.
#[allow(clippy::deref_addrof)]
pub fn get() -> Option<&'static mut Gpu> {
    // SAFETY: read-only after `find` has run.
    unsafe { (&mut *(&raw mut GPU)).as_mut() }
}
