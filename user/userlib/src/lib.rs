//! Shared `no_std` runtime for every EL0 binary built against it: `_start`, syscall wrappers,
//! `argv` decoding, `exit`, and a global allocator over a fixed heap -- the boilerplate
//! `user/hello`, `user/echo` and `user/scratch` each used to hand-roll independently before this
//! crate existed. Re-exported flat, so a program writes `userlib::write`, `userlib::exit`,
//! `userlib::entry!`, ... .
//!
//! - `syscall`: the raw `svc` and the `syscall!` macro -- the mechanism, crate-private.
//! - `io`: `read`/`write`, each a thin wrapper over one syscall.
//! - `heap`: an unconditional global allocator over the fixed heap
//!   (`plans/USERSPACE_PLAN.md`'s divergence #6) -- `extern crate alloc` (`Vec`/`String`) just
//!   works in any program built against this crate, from its very first version.
//! - `process`: a program's own life -- entry stub and macros, `argc`/`argv`/`envp`, `exit`, the
//!   panic handler.
//! - `time`: `clock_gettime`, wrapped as `time()`.

#![no_std]

extern crate alloc;

// Declared first, with `#[macro_use]`: `syscall!` is a plain `macro_rules!`, visible only to the
// modules that follow it.
#[macro_use]
mod syscall;
mod blk;
mod heap;
mod io;
mod process;
mod time;

pub use blk::blkinfo;
pub use heap::init_heap;
pub use io::*;
pub use process::*;
pub use time::*;

// The syscall numbers live in the shared `abi` crate (the kernel uses the same ones); re-exported
// so `userlib::SYS_WRITE`, ... work for a program that wants to make a raw syscall itself.
pub use abi::{SYS_EXIT, SYS_READ, SYS_WRITE};
