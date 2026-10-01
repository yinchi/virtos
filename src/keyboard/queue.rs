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
//! `drain` runs from `gic::irq_handler` and, while a program is blocked in `read(0)` (IRQs are
//! masked inside every syscall), from `keyboard::stdin` directly; `pop` runs from ordinary code
//! with interrupts on, so it masks them for the instant it takes -- none of them ever touches
//! `QUEUE`/`KEYS` at the same time as another.

use alloc::collections::VecDeque;

use super::{KeyState, Token, admits};
use crate::arch::irq::without_irqs;
use crate::drivers::virtio::input;
use core::fmt::Write;

/// How many characters can wait. Matches the reference lab's release-build queue size.
const CAPACITY: usize = 256;

/// SAFETY (both statics): single core; `drain` (the sole writer, called only from the IRQ
/// handler) and `pop` (the sole other accessor, which masks IRQs around its access) never touch
/// these at the same time.
static mut QUEUE: VecDeque<Token> = VecDeque::new();
static mut KEYS: KeyState = KeyState::new();
/// Whether the overflow note for the current burst has been printed -- one note per burst, not one
/// per dropped key. Cleared once the queue has been emptied (`pop`).
static mut OVERFLOW_NOTED: bool = false;

/// Drains every event the keyboard has queued, translates it, and pushes the result. Called from
/// `gic::irq_handler` once it's confirmed the acknowledged interrupt was the keyboard's SPI.
pub fn drain() {
    let Some(keyboard) = input::get() else {
        return;
    };
    // Clears the device's interrupt line so it doesn't keep re-asserting.
    keyboard.ack_interrupt();
    #[allow(clippy::deref_addrof)]
    let (queue, keys, noted) = unsafe {
        (
            &mut *(&raw mut QUEUE),
            &mut *(&raw mut KEYS),
            &mut *(&raw mut OVERFLOW_NOTED),
        )
    };
    while let Some(event) = keyboard.poll() {
        let Some(token) = keys.apply(event.code, event.value) else {
            continue;
        };
        // A key's auto-repeat is dropped while anything is still queued (see `admits`), so a slow
        // reader never has a backlog of repeats to work off after the key is let go.
        if !admits(&token, queue.len()) {
            continue;
        }
        if queue.len() < CAPACITY {
            queue.push_back(token);
        } else if !*noted {
            *noted = true;
            let _ = writeln!(
                crate::bare_metal::get_uart(),
                "\n[keyboard: input queue full, further keys dropped]"
            );
        }
    }
}

/// Takes the oldest waiting key press, if there is one.
#[allow(clippy::deref_addrof)]
pub fn pop() -> Option<Token> {
    without_irqs(|| {
        // SAFETY: see QUEUE.
        let (queue, noted) = unsafe { (&mut *(&raw mut QUEUE), &mut *(&raw mut OVERFLOW_NOTED)) };
        let token = queue.pop_front();
        if queue.is_empty() {
            *noted = false; // the next burst gets its own note
        }
        token
    })
}

/// Sleeps until an interrupt arrives, unless a key is already waiting. The check and the `wfi`
/// happen with IRQs masked: unmasked, a keyboard IRQ landing between "queue is empty" and `wfi`
/// would be handled and gone, and the sleep that followed would wait for the *next* key. (A
/// masked-but-pending interrupt still wakes `wfi`, so it is delivered the moment IRQs are
/// unmasked again.)
#[allow(clippy::deref_addrof)]
pub fn wait_for_token() {
    without_irqs(|| {
        if unsafe { (*(&raw const QUEUE)).is_empty() } {
            // SAFETY: plain `wfi`, no memory or register effects.
            unsafe { core::arch::asm!("wfi") };
        }
    });
}
