//! The VirtIO input device (`virtio-keyboard-device`) `super::discover` finds among the
//! `virtio,mmio` slots the device tree lists.
//!
//! IRQ-driven: `gic::irq_handler` calls `keyboard::queue::drain` each time this device's SPI
//! fires, which drains every pending event via `poll()` in one go -- unlike blk/gpu's
//! request-then-wait shape, a key genuinely arrives at a moment nobody requested, so polling this
//! device from `kernel_main` the way blk/gpu are polled would mean losing keys while busy doing
//! anything else.

use core::sync::atomic::{AtomicU32, Ordering};

use arm_gic::IntId;
pub use virtio_drivers::device::input::InputEvent;
use virtio_drivers::device::input::VirtIOInput;
use virtio_drivers::transport::mmio::MmioTransport;

use super::hal::VirtioHalImpl;
use crate::arch::gic::enable_interrupt;

pub struct Keyboard {
    inner: VirtIOInput<VirtioHalImpl, MmioTransport<'static>>,
}

/// The keyboard, if one was found. At most one exists on this platform.
static mut KEYBOARD: Option<Keyboard> = None;
/// This device's SPI number, if it has one -- 0 (never a real SPI, QEMU's `virt` machine starts
/// numbering these well above it) means "not set".
static SPI: AtomicU32 = AtomicU32::new(0);

impl Keyboard {
    /// The next pending input event, if any -- non-blocking.
    pub fn poll(&mut self) -> Option<InputEvent> {
        self.inner.pop_pending_event()
    }

    /// Clears the virtio-mmio transport's interrupt status so the level-triggered line
    /// deasserts. Called from `queue::drain` once it's about to (or has just finished) draining
    /// this device's events.
    pub fn ack_interrupt(&mut self) {
        self.inner.ack_interrupt();
    }
}

/// Claims the slot `super::discover` classified as the keyboard and, if the DTB gave it an SPI,
/// enables that interrupt at the GIC. A device whose driver fails to initialize is treated as
/// absent.
pub(super) fn set(transport: MmioTransport<'static>, irq: Option<u32>) {
    let Ok(inner) = VirtIOInput::new(transport) else {
        return;
    };
    // SAFETY: sole writer, called once before `get` is ever reached.
    unsafe { KEYBOARD = Some(Keyboard { inner }) };
    if let Some(irq) = irq {
        SPI.store(irq, Ordering::Relaxed);
        enable_interrupt(IntId::spi(irq));
    }
}

/// The keyboard, if `discover` found one.
#[allow(clippy::deref_addrof)]
pub fn get() -> Option<&'static mut Keyboard> {
    // SAFETY: read-only after `discover` has run.
    unsafe { (&mut *(&raw mut KEYBOARD)).as_mut() }
}

/// This device's SPI number, if `discover` found both a keyboard and an SPI for it.
pub fn spi() -> Option<u32> {
    match SPI.load(Ordering::Relaxed) {
        0 => None,
        spi => Some(spi),
    }
}
