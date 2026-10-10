//! The pure logic behind `edit`: the line buffer, the soft-wrap layout, the
//! view's scrolling, the mark and cut buffer, and the `.editrc` parser. `no_std` + `alloc`, no
//! dependency on `userlib` or any syscall, so every module here is host-tested
//! (`cargo test`) exactly like the kernel's own pure modules.

#![no_std]

extern crate alloc;

pub mod buffer;
pub mod editrc;
pub mod layout;
pub mod prompt;
pub mod region;
pub mod render;
pub mod scroll;
pub mod search;
pub mod width;
