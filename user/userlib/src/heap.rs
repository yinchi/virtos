//! The user heap: an unconditional `#[global_allocator]` over the program break (`brk`), so any
//! program built against this crate can use `alloc` (`Vec`, `String`, `Box`, ...) with no setup.
//!
//! The heap starts empty and asks the kernel for memory *when an allocation does not fit*, not per
//! allocation: the first request maps a chunk (at least 64 KiB), and each later one grows by at
//! least the heap's current size (capped at 1 MiB per step, so a huge heap does not double past
//! what is needed), so a program that allocates a lot makes a handful of `brk` calls, not
//! thousands. The bookkeeping -- free lists, splitting, coalescing -- is `linked_list_allocator`'s
//! `Heap`, the crate the kernel's own heap uses; this module only decides when to call `brk`. A
//! request the kernel refuses (the heap would run into the stack's guard) is an allocation failure,
//! which `alloc` turns into a panic and the panic handler into `exit(101)`.
//!
//! Memory is never given back: `dealloc` returns blocks to the free list, and the break only moves
//! up.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ptr::{NonNull, null_mut};

use abi::SYS_BRK;
use linked_list_allocator::Heap;

/// Moves the program break to `addr` and returns the resulting break; `brk(0)` asks. A request the
/// kernel cannot grant returns the old break, not an error -- see `abi::SYS_BRK`.
pub fn brk(addr: usize) -> usize {
    syscall!(SYS_BRK, addr) as usize
}

/// The kernel maps whole pages, so ask in whole pages.
const PAGE: usize = 4096;
/// The smallest amount a growth asks for.
const MIN_GROWTH: usize = 64 * 1024;
/// The most a growth asks for *because the heap is big* (a bigger single request is still met).
const MAX_STEP: usize = 1024 * 1024;

struct State {
    heap: Heap,
    /// Whether `heap` has been given its first memory yet.
    started: bool,
    /// How much memory the heap manages: from where it started to the break as it left it.
    size: usize,
}

/// The allocator. A program is one thread with no signal handlers, so nothing re-enters it: no
/// lock.
struct UserHeap(UnsafeCell<State>);

// SAFETY: there is one thread of execution in EL0 (no threads, no asynchronous signals), so the
// state is never accessed concurrently.
unsafe impl Sync for UserHeap {}

#[global_allocator]
static ALLOCATOR: UserHeap = UserHeap(UnsafeCell::new(State {
    heap: Heap::empty(),
    started: false,
    size: 0,
}));

fn round_up(n: usize) -> usize {
    n.div_ceil(PAGE) * PAGE
}

/// Asks the kernel for at least `need` more bytes and hands them to the heap. `false` if it
/// refuses.
fn grow(state: &mut State, need: usize) -> bool {
    let current = brk(0);
    // Room for the allocator's own bookkeeping around the request.
    let need = round_up(need.saturating_add(4 * core::mem::size_of::<usize>() + PAGE));
    let step = need.max(MIN_GROWTH).max(state.size.min(MAX_STEP));
    // Ask for the generous amount, then for just what is needed if the generous one is refused.
    let mut got = 0;
    for ask in [step, need] {
        let Some(target) = current.checked_add(ask) else {
            continue;
        };
        if brk(target) == target {
            got = ask;
            break;
        }
    }
    if got == 0 {
        return false;
    }
    // SAFETY: `current..current + got` is memory `brk` just granted, contiguous with the heap's
    // end, and nothing else uses it.
    unsafe {
        if state.started {
            state.heap.extend(got);
        } else {
            state.heap.init(current as *mut u8, got);
            state.started = true;
        }
    }
    state.size += got;
    true
}

unsafe impl GlobalAlloc for UserHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: see `UserHeap`.
        let state = unsafe { &mut *self.0.get() };
        loop {
            if state.started
                && let Ok(block) = state.heap.allocate_first_fit(layout)
            {
                return block.as_ptr();
            }
            if !grow(state, layout.size().saturating_add(layout.align())) {
                return null_mut();
            }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: see `UserHeap`; `ptr` came from `alloc` with this layout.
        unsafe {
            let state = &mut *self.0.get();
            if let Some(ptr) = NonNull::new(ptr) {
                state.heap.deallocate(ptr, layout);
            }
        }
    }
}
