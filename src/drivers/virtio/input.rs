//! The VirtIO input device (`virtio-keyboard-device`) `super::discover` finds among the
//! `virtio,mmio` slots the device tree lists.
//!
//! Polled (`pop_pending_event`), not interrupt-driven, for now -- unlike blk/gpu's
//! request-then-wait shape, a key genuinely arrives at a moment nobody requested, so this is a
//! real simplification, not a free one: it only holds up as long as nothing else needs the CPU
//! while waiting for a key. Revisit once something exists that wants to do other things
//! concurrently (a shell, a scheduler).

pub use virtio_drivers::device::input::InputEvent;
use virtio_drivers::device::input::VirtIOInput;
use virtio_drivers::transport::mmio::MmioTransport;

use super::hal::VirtioHalImpl;

pub struct Keyboard {
    inner: VirtIOInput<VirtioHalImpl, MmioTransport<'static>>,
}

/// The keyboard, if one was found. At most one exists on this platform.
static mut KEYBOARD: Option<Keyboard> = None;

impl Keyboard {
    /// The next pending input event, if any -- non-blocking.
    pub fn poll(&mut self) -> Option<InputEvent> {
        self.inner.pop_pending_event()
    }
}

/// Claims the slot `super::discover` classified as the keyboard. A device whose driver fails to
/// initialize is treated as absent.
pub(super) fn set(transport: MmioTransport<'static>) {
    let Ok(inner) = VirtIOInput::new(transport) else {
        return;
    };
    // SAFETY: sole writer, called once before `get` is ever reached.
    unsafe { KEYBOARD = Some(Keyboard { inner }) };
}

/// The keyboard, if `discover` found one.
#[allow(clippy::deref_addrof)]
pub fn get() -> Option<&'static mut Keyboard> {
    // SAFETY: read-only after `discover` has run.
    unsafe { (&mut *(&raw mut KEYBOARD)).as_mut() }
}
