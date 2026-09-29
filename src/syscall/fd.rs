//! File descriptor dispatch for the `read`/`write` syscalls. At most one program is ever
//! resident and no redirection exists yet, so the mapping from a small fd number to what it
//! refers to is fixed rather than a real per-process table -- `0`/`1`/`2` always mean
//! `Keyboard`/`Console`/`Console`. A real table is divergence #2 in `plans/USERSPACE_PLAN.md`,
//! deferred until a launcher needs to rebind an entry (`open`/`close`, `>`/`<` redirection).

use core::fmt::Write;

use crate::arch::mmu::{self, USER_BASE, USER_STACK_TOP};
use crate::console;
use crate::drivers::virtio::gpu;
use crate::keyboard::queue;

/// This project's own error convention so far -- a negative value, not yet the real negated-errno
/// convention `plans/USERSPACE_PLAN.md` settles on for the full syscall set (no `errno` constants
/// exist yet to return one of).
const EBADF: isize = -1;

#[derive(Clone, Copy)]
enum FileDescriptor {
    Keyboard,
    Console,
}

impl FileDescriptor {
    fn for_fd(fd: usize) -> Option<Self> {
        match fd {
            0 => Some(Self::Keyboard),
            1 | 2 => Some(Self::Console),
            _ => None,
        }
    }

    /// Writes to the console -- both the GPU text console (if `console::set` ran, i.e. a GPU
    /// device exists) and the UART, the same dual visibility every diagnostic print in this
    /// project already has. Writing to `Keyboard` is rejected outright, not silently accepted as
    /// a no-op: a mixed-up fd number should be visible as an error, not hidden behind output that
    /// simply never appears.
    fn write(self, bytes: &[u8]) -> isize {
        match self {
            Self::Console => {
                let text = core::str::from_utf8(bytes).unwrap_or("<invalid utf8>");
                if let Some(con) = console::get() {
                    con.puts(text, console::DEFAULT_FG, console::DEFAULT_BG);
                    if let Some(gpu_dev) = gpu::get() {
                        gpu_dev.flush();
                    }
                }
                let mut uart = crate::bare_metal::get_uart();
                let _ = uart.write_str(text);
                bytes.len() as isize
            }
            Self::Keyboard => EBADF,
        }
    }

    /// Reads whatever the keyboard queue already has, up to `buf.len()` -- non-blocking: no
    /// process model exists to block *on*, so an empty queue returns `0` immediately rather than
    /// waiting for a key that might never come. A real blocking `read(0)` needs a caller that can
    /// give up the CPU while it waits (at least an idle `wfe` loop with IRQs on); nothing in this
    /// project needs that yet. Reading from `Console` is rejected for the same reason writing to
    /// `Keyboard` is.
    fn read(self, buf: &mut [u8]) -> isize {
        match self {
            Self::Keyboard => {
                let mut n = 0;
                while n < buf.len() {
                    let Some(ch) = queue::pop() else { break };
                    // ASCII-only (see `keyboard`'s own doc comment) -- one byte per character.
                    buf[n] = ch as u8;
                    n += 1;
                }
                n as isize
            }
            Self::Console => EBADF,
        }
    }
}

/// Bounds-checks `ptr`/`len` against the user window before trusting them: the MMU's `USER`
/// attribute bit gates *EL0's* access, not EL1's, so a buggy or malicious pointer aimed at kernel
/// memory is otherwise still readable/writable by the kernel and must be rejected in software.
fn validate(ptr: usize, len: usize) -> bool {
    let Some(end) = ptr.checked_add(len) else {
        return false;
    };
    ptr >= USER_BASE && end <= USER_STACK_TOP
}

pub fn write(fd: usize, ptr: usize, len: usize) -> isize {
    let Some(fd) = FileDescriptor::for_fd(fd) else {
        return EBADF;
    };
    if !validate(ptr, len) {
        return EBADF;
    }
    // The read below touches a page marked USER, which PAN would otherwise forbid.
    let _user = mmu::user_access();
    // SAFETY: validated above to lie entirely within the user window; the guard above lets the
    // kernel read it despite PAN.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
    fd.write(bytes)
}

pub fn read(fd: usize, ptr: usize, len: usize) -> isize {
    let Some(fd) = FileDescriptor::for_fd(fd) else {
        return EBADF;
    };
    if !validate(ptr, len) {
        return EBADF;
    }
    // The write below touches a page marked USER, which PAN would otherwise forbid.
    let _user = mmu::user_access();
    // SAFETY: as in `write`.
    let buf = unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, len) };
    fd.read(buf)
}
