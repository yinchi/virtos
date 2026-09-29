//! Enters EL0 and returns once the program running there exits or faults -- the hand-rolled
//! setjmp/longjmp in `context.s` (`enter_el0`/`resume_kernel`) is what makes this look like an
//! ordinary function call from Rust's point of view: `enter_el0` upholds the ordinary AAPCS64
//! calling convention exactly (callee-saved registers and SP survive across it), so calling it is
//! indistinguishable from calling any other function that happens to take a very long time --
//! during which a whole EL0 program runs -- before returning.

use arm_sysregs::el1::accessors::{write_elr_el1, write_spsr_el1};
use arm_sysregs::el1::registers::{ElrEl1, SpsrEl1};

use super::elf;
use super::elfparse::ElfError;
use crate::arch::irq;
use crate::arch::mmu::{self, USER_STACK_BOTTOM, USER_STACK_TOP};

unsafe extern "C" {
    /// Checkpoints this project's callee-saved register set and `eret`s into EL0. `SPSR_EL1`/
    /// `ELR_EL1`/`SP_EL0` must already be set (`enter`, below) -- upholds the ordinary AAPCS64
    /// calling convention exactly, so calling it from Rust needs no special handling.
    fn enter_el0();

    /// Restores the register set `enter_el0` saved and jumps back into it directly -- never
    /// returns itself. Called from `syscall::sync_el0_handler` for `exit` and for a caught fault
    /// alike.
    ///
    /// # Safety
    /// Must only be called from within the `sync_el0_64` handler, after `enter` has actually
    /// started a program (so `context.s`'s `KERNEL_CTX` holds a real checkpoint, not its zeroed
    /// initial state).
    pub fn resume_kernel() -> !;
}

/// Enters EL0 at `entry`, with `sp` as the initial `SP_EL0`, and returns once that program exits
/// or faults -- `enter_el0`/`resume_kernel` (`context.s`) are what make an ordinary Rust function
/// call correctly "pause" for however long the program runs, however it ends.
///
/// Interrupts are masked from the first system-register write below until `enter_el0`'s `eret`
/// (see `irq::mask`), then explicitly unmasked again once it returns: every path back out of a
/// program -- `exit`, a caught fault -- is a `sync_el0_64` exception, which unconditionally
/// re-masks IRQs on entry, and `resume_kernel`'s longjmp-style return (`context.s`) bypasses the
/// normal `kernel_exit`/`eret` path that would otherwise restore the mask. Without this, the
/// kernel would carry on with IRQs masked forever after the first program exits -- see `irq::unmask`.
///
/// # Safety
/// `entry` must be mapped EL0-executable and `sp` must be a valid EL0 stack pointer -- callers so
/// far map both inside `mmu::USER_BASE..USER_BASE + USER_SIZE` before calling this.
pub unsafe fn enter(entry: usize, sp: usize) {
    irq::mask();

    // M[3:0] = 0b0000 (EL0t), every DAIF bit unmasked (bits 9:6 = 0) -- the program runs
    // preemptible, exactly like any other code, from the moment it starts.
    // SAFETY: SPSR_EL1 only takes effect on the `eret` inside `enter_el0`, below, by which point
    // ELR_EL1/SP_EL0 are set too.
    unsafe { write_spsr_el1(SpsrEl1::empty()) };
    // SAFETY: `entry` is mapped EL0-executable, per this function's own safety contract.
    unsafe { write_elr_el1(ElrEl1::empty().with_addr(entry as u64)) };
    // SAFETY: `sp` is a valid EL0 stack pointer, per this function's own safety contract. No
    // `arm_sysregs` accessor covers SP_EL0.
    unsafe { core::arch::asm!("msr sp_el0, {sp}", sp = in(reg) sp as u64) };

    // SAFETY: SPSR_EL1/ELR_EL1/SP_EL0 are all set, per `enter_el0`'s own contract.
    unsafe { enter_el0() };

    // Back from the program (its `exit` or a fault, both taken as exceptions, so IRQs are masked
    // again regardless of what the program itself last had them set to) -- unmask unconditionally
    // so the kernel resumes normal operation, exactly as if this had been an ordinary call.
    irq::unmask();
}

/// Loads `elf_bytes`, gives it a fresh stack, and runs it at EL0 to completion, then returns
/// normally -- `elf::load` maps the program's own segments; this additionally maps its stack (no
/// argv/envp on it yet -- nothing produces real arguments until a shell exists to supply them) and
/// calls `enter`.
///
/// # Errors
/// Returns `Err` without entering EL0 at all if the ELF is invalid or can't be mapped (see
/// `elf::load`), or if mapping the stack itself fails.
pub fn run_program(elf_bytes: &[u8]) -> Result<(), ElfError> {
    let entry = elf::load(elf_bytes)?;

    // The stack, mapped and zeroed the same way a segment is while being filled -- see
    // `elf::data_attributes`. Not locked down afterward the way a segment is: a stack is always
    // read/write, never executable, for as long as the program runs.
    mmu::set_user_pages(
        USER_STACK_BOTTOM,
        USER_STACK_TOP,
        Some(elf::data_attributes()),
    )
    .map_err(|_| ElfError::MapFailed)?;
    {
        // The write below touches a page marked USER, which PAN would otherwise forbid --
        // `elf::load` took its own guard for the same reason, but that one already dropped when
        // it returned above.
        let _user = mmu::user_access();
        // SAFETY: just mapped writable, and nothing else uses this range while a program loads.
        unsafe {
            core::ptr::write_bytes(
                USER_STACK_BOTTOM as *mut u8,
                0,
                USER_STACK_TOP - USER_STACK_BOTTOM,
            );
        }
    }

    // SAFETY: `entry` came from `elf::load`, which only ever returns an address inside a segment
    // it just mapped EL0-executable; `USER_STACK_TOP` is the top of the stack just mapped above.
    unsafe { enter(entry, USER_STACK_TOP) };
    Ok(())
}
