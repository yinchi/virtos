//! The fixed userspace heap: an unconditional `#[global_allocator]` backed by the address/size
//! `abi` and the kernel both agree on (`plans/USERSPACE_PLAN.md`'s divergence #6) -- no `brk`
//! syscall exists to grow it, so `init_heap` (called once, by the `entry!`/`entry_with_args!`-
//! generated `main`, before a program's own code runs) just tells `linked_list_allocator` about
//! the whole fixed range up front.

use linked_list_allocator::LockedHeap;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// Initializes the global allocator over the fixed heap range. Not meant to be called directly --
/// `entry!`/`entry_with_args!` (`process.rs`) call it exactly once, before `$run`; `pub` only
/// because a macro expanding in another crate needs to reach it, the same way `exit`/`args` do.
pub fn init_heap() {
    // SAFETY: the kernel maps `abi::USER_HEAP_BOTTOM..+abi::USER_HEAP_SIZE` read/write before
    // entering this program (`exec::process::run_program`), and nothing else in this single-
    // threaded program touches it before this runs, exactly once, first.
    unsafe {
        ALLOCATOR
            .lock()
            .init(abi::USER_HEAP_BOTTOM as *mut u8, abi::USER_HEAP_SIZE);
    }
}
