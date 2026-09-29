//! The keyboard's IRQ-driven queue: where a key press waits between the interrupt that learns of
//! it and whoever reads it via `pop`.
//!
//! Uses `alloc::collections::VecDeque` rather than a hand-rolled fixed-capacity ring buffer --
//! the kernel already has a global allocator, so there's no reason to reimplement bounded-array
//! indexing just to avoid using it. `drain` still enforces a fixed capacity itself (`VecDeque`
//! alone would just keep growing): a full queue drops the *newest* character rather than growing
//! further or evicting something already waiting, matching the reference lab's queue policy (see
//! `plans/USERSPACE_PLAN.md`'s "Keyboard input flake" section) -- dropping is unavoidable without
//! a scheduler, though the real loss point this doesn't fully close is upstream, in virtio-input's
//! own fixed-size event queue.
//!
//! `drain` runs from `gic::irq_handler`; `pop` runs from ordinary code with interrupts on, so it
//! masks them for the instant it takes -- the two never touch `QUEUE`/`KEYS` at the same time.

use alloc::collections::VecDeque;

use super::KeyState;
use crate::arch::irq::without_irqs;
use crate::drivers::virtio::input;

/// How many characters can wait. Matches the reference lab's release-build queue size.
const CAPACITY: usize = 256;

/// SAFETY (both statics): single core; `drain` (the sole writer, called only from the IRQ
/// handler) and `pop` (the sole other accessor, which masks IRQs around its access) never touch
/// these at the same time.
static mut QUEUE: VecDeque<char> = VecDeque::new();
static mut KEYS: KeyState = KeyState::new();

/// Drains every event the keyboard has queued, translates it, and pushes the result. Called from
/// `gic::irq_handler` once it's confirmed the acknowledged interrupt was the keyboard's SPI.
pub fn drain() {
    let Some(keyboard) = input::get() else {
        return;
    };
    // Clears the device's interrupt line so it doesn't keep re-asserting.
    keyboard.ack_interrupt();
    #[allow(clippy::deref_addrof)]
    let (queue, keys) = unsafe { (&mut *(&raw mut QUEUE), &mut *(&raw mut KEYS)) };
    while let Some(event) = keyboard.poll() {
        let Some(ch) = keys.apply(event.code, event.value) else {
            continue;
        };
        if queue.len() < CAPACITY {
            queue.push_back(ch);
        }
    }
}

/// Takes the oldest waiting character, if there is one.
#[allow(clippy::deref_addrof)]
pub fn pop() -> Option<char> {
    without_irqs(|| unsafe { (&mut *(&raw mut QUEUE)).pop_front() })
}
