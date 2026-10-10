//! `read(0)` from the keyboard: hands a running program one finished line of typed input at a
//! time, like a real tty's cooked mode.
//!
//! The program is blocked inside a syscall, and IRQs are masked for the whole of every syscall
//! (`syscall::sync_el0_handler`), so no keyboard interrupt can reach `queue` while it waits.
//! `read_line` therefore drains the device into the queue itself (`queue::drain`, the same
//! producer the interrupt handler uses), pops what is there, and feeds it to the line discipline
//! -- the one the shell's prompt uses -- so Backspace and friends are absorbed here exactly as
//! they are there. A program only ever sees a finished line's printable bytes plus a trailing
//! newline. Keys typed before the program asked for input wait in the queue and are read like any
//! other, in order.
//!
//! Ctrl+D on an empty line is end-of-file (`read` returns `0`), so a program reading until EOF
//! (`cat` with no operand) has a way to stop. On a non-empty line it delivers what's typed so far
//! without a trailing newline instead (POSIX canonical mode's actual rule); a further Ctrl+D on
//! the now-empty line is then EOF.

use alloc::vec::Vec;

use super::line_discipline::{self, LineOutcome, Mode};
use super::queue;
use crate::console;
use crate::drivers::virtio::gpu;

/// The rest of the last line handed out, when the program's buffer was smaller than the line.
///
/// SAFETY (both statics, every access): single core, syscalls run with IRQs masked -- nothing
/// else touches them.
static mut PENDING: Vec<u8> = Vec::new();
/// How far into `PENDING` the program has read.
static mut PENDING_POS: usize = 0;

/// Forgets any leftover line -- called at the start of each launch, so one program's unread
/// input can't leak into the next.
#[allow(clippy::deref_addrof)]
pub fn reset() {
    // SAFETY: see PENDING.
    unsafe {
        (*(&raw mut PENDING)).clear();
        PENDING_POS = 0;
    }
}

/// Fills `buf` from the current line, blocking for a new one if it's used up. Returns the number
/// of bytes copied, or `0` at end-of-file (Ctrl+D on an empty line) -- or if `buf` is empty, or
/// no console exists to type on.
#[allow(clippy::deref_addrof)]
pub fn read(buf: &mut [u8]) -> isize {
    if buf.is_empty() {
        return 0;
    }
    // SAFETY: see PENDING.
    let (pending, pos) = unsafe { (&mut *(&raw mut PENDING), &mut *(&raw mut PENDING_POS)) };

    if *pos >= pending.len() {
        let Some(line) = read_line() else {
            return 0;
        };
        *pending = line;
        *pos = 0;
    }

    let n = buf.len().min(pending.len() - *pos);
    buf[..n].copy_from_slice(&pending[*pos..*pos + n]);
    *pos += n;
    n as isize
}

/// Blocks until a key is pressed and returns it: one token popped straight from the queue,
/// bypassing the line discipline entirely -- no Enter-wait, no echo, and `read(0)`'s state
/// (`PENDING`) untouched. This and `read` are two independent readers of the one token queue;
/// whichever is called pops the next token, so a program that mixed them would simply split the
/// stream (unsupported, not guarded). Nothing here is stateful, so there is nothing to reset on a
/// launch, an exit or a fault, unlike a raw-mode toggle on `read(0)`.
pub fn read_key() -> abi::keys::KeyEvent {
    loop {
        // IRQs are masked inside a syscall, so the device's events wait until fetched here --
        // same reasoning as `read_line`'s loop.
        queue::drain();
        if let Some(token) = queue::pop() {
            return token.to_key_event();
        }
        // SAFETY: plain `wfi`, no memory or register effects.
        unsafe { core::arch::asm!("wfi") };
    }
}

/// Blocks until Enter finishes a line, or Ctrl+D delivers what's typed so far -- either way,
/// what's returned already has whatever trailing newline the caller should see (one for Enter,
/// none for Ctrl+D's partial delivery). `None` on Ctrl+D with nothing typed, or with nothing to
/// type on.
fn read_line() -> Option<Vec<u8>> {
    let (Some(console), Some(gpu)) = (console::get(), gpu::get()) else {
        return None;
    };
    let discipline = line_discipline::get();
    // Draws the (still empty) line immediately, so a program waiting on input is visibly doing so.
    discipline.begin(console, "", Mode::Canonical);
    gpu.flush();

    loop {
        // IRQs are masked, so the device's events wait there until we fetch them.
        queue::drain();
        while let Some(token) = queue::pop() {
            // Fetched fresh for each token, never held across the loop: see `line_discipline`'s
            // doc comment.
            let console = console::get()?;
            match discipline.handle(token, console) {
                LineOutcome::Ignored => {}
                LineOutcome::Edited => gpu.flush(),
                LineOutcome::Finished(text) => {
                    gpu.flush();
                    let mut bytes = text.into_bytes();
                    bytes.push(b'\n');
                    return Some(bytes);
                }
                LineOutcome::Partial(text) => {
                    gpu.flush();
                    return Some(text.into_bytes());
                }
                LineOutcome::EndOfFile => return None,
            }
        }
        // Nothing typed yet: sleep until an interrupt line goes up. A masked interrupt still
        // wakes `wfi`; the loop then fetches it.
        // SAFETY: plain `wfi`, no memory or register effects.
        unsafe { core::arch::asm!("wfi") };
    }
}
