//! Loading and running EL0 programs. No process table, no scheduler: at most one program is ever
//! resident (see `plans/USERSPACE_PLAN.md` for why that's the real, permanent scope here, not a
//! simplified subset of some later stage).

// Pure logic, no hardware dependency: builds and runs under both targets, like `fs::sector`.
pub mod elfparse;

// Hardware-only: touches the page table / system registers, or exists only to serve code that
// does -- see `fs/mod.rs` for the same split applied to the filesystem.
#[cfg(target_os = "none")]
pub mod elf;
#[cfg(target_os = "none")]
pub mod process;
