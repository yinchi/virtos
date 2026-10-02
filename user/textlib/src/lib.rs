//! The pure logic behind the text-filter programs in `user/progs`, kept in its own crate so it is
//! unit-tested on the host (`just test-host`). `no_std` + `alloc`, nothing EL0-specific.
#![no_std]

// `macro_use` (tests only) so the unit tests can write `vec!`/`format!` unqualified, as they would under `std`.
#[cfg_attr(test, macro_use)]
extern crate alloc;

pub mod countspec;
pub mod cutlist;
pub mod glob;
pub mod headtail;
pub mod human;
pub mod sortkey;
pub mod textutil;
pub mod trset;
