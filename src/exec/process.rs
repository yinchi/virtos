//! Enters EL0 and returns once the program running there exits or faults -- the hand-rolled
//! setjmp/longjmp in `context.s` (`enter_el0`/`resume_kernel`) is what makes this look like an
//! ordinary function call from Rust's point of view: `enter_el0` upholds the ordinary AAPCS64
//! calling convention exactly (callee-saved registers and SP survive across it), so calling it is
//! indistinguishable from calling any other function that happens to take a very long time --
//! during which a whole EL0 program runs -- before returning.

use alloc::vec::Vec;

use arm_sysregs::el1::accessors::{write_elr_el1, write_spsr_el1};
use arm_sysregs::el1::registers::{ElrEl1, SpsrEl1};

use super::argplan;
use super::elf;
use super::elfparse::ElfError;
use crate::arch::irq;
use crate::arch::mmu::{self, USER_HEAP_BOTTOM, USER_HEAP_TOP, USER_STACK_BOTTOM, USER_STACK_TOP};

/// The most stack a program's initial `argv` and `envp` together (strings plus pointer arrays)
/// may take -- far more than a typed line and a sensible environment can ever produce, and an
/// eighth of the stack (`mmu::USER_STACK_SIZE`), so an absurd list is refused up front instead of
/// leaving the program almost no stack of its own.
pub const ARG_MAX: usize = 128 * 1024;

/// Why a program couldn't be started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchError {
    /// Not a loadable executable -- see `ElfError` for exactly why.
    Elf(ElfError),
    /// `args` and `env` together don't fit in `ARG_MAX`.
    ArgsTooBig,
    /// A `stdio` redirection couldn't be bound -- see `syscall::fd::bind_stdio`.
    BadRedirect,
}

impl From<ElfError> for LaunchError {
    fn from(e: ElfError) -> Self {
        LaunchError::Elf(e)
    }
}

unsafe extern "C" {
    /// Checkpoints this project's callee-saved register set and `eret`s into EL0. `SPSR_EL1`/
    /// `ELR_EL1`/`SP_EL0` must already be set (`enter`, below) -- upholds the ordinary AAPCS64
    /// calling convention exactly, so calling it from Rust needs no special handling.
    fn enter_el0();

    /// Restores the register set `enter_el0` saved and jumps back into it directly -- never
    /// returns itself. Called from `syscall::sync_el0_handler` for `exit` and for a caught fault
    /// alike. `code` becomes `enter`'s own return value: `context.s`'s register restore never
    /// touches `x0`, so whatever AAPCS64 places there for this call survives untouched all the way
    /// back out through `enter_el0`'s own `ret` and the `bl` that reaches it in `enter`'s inline
    /// asm below.
    ///
    /// # Safety
    /// Must only be called from within the `sync_el0_64` handler, after `enter` has actually
    /// started a program (so `context.s`'s `KERNEL_CTX` holds a real checkpoint, not its zeroed
    /// initial state).
    pub fn resume_kernel(code: i32) -> !;
}

/// Enters EL0 at `entry`, with `argv` as the initial `SP_EL0` (the top of what `push_strings` just
/// wrote there -- `argc`/`argv`/`envp` sit at or below it), and returns once that program exits or
/// faults -- `enter_el0`/`resume_kernel` (`context.s`) are what make an ordinary Rust function call
/// correctly "pause" for however long the program runs, however it ends. The returned `i32` is
/// whatever `resume_kernel` was called with: the program's own `exit` code, or a fixed `139` for a
/// caught fault (`syscall::sync_el0_handler`'s own doc comment on both call sites).
///
/// Interrupts are masked from the first system-register write below until `enter_el0`'s `eret`
/// (see `irq::mask`), then explicitly unmasked again once it returns: every path back out of a
/// program -- `exit`, a caught fault -- is a `sync_el0_64` exception, which unconditionally
/// re-masks IRQs on entry, and `resume_kernel`'s longjmp-style return (`context.s`) bypasses the
/// normal `kernel_exit`/`eret` path that would otherwise restore the mask. Without this, the
/// kernel would carry on with IRQs masked forever after the first program exits -- see `irq::unmask`.
///
/// # Safety
/// `entry` must be mapped EL0-executable and `argv` must be a valid EL0 stack pointer, with
/// `argc`/`argv`/`envp` already written at/below it (`push_strings`) -- callers so far map and
/// write both inside `mmu::USER_BASE..USER_BASE + USER_SIZE` before calling this.
pub unsafe fn enter(entry: usize, argc: usize, argv: usize, envp: usize) -> i32 {
    irq::mask();

    // M[3:0] = 0b0000 (EL0t), every DAIF bit unmasked (bits 9:6 = 0) -- the program runs
    // preemptible, exactly like any other code, from the moment it starts.
    // SAFETY: SPSR_EL1 only takes effect on the `eret` inside `enter_el0`, below, by which point
    // ELR_EL1/SP_EL0 are set too.
    unsafe { write_spsr_el1(SpsrEl1::empty()) };
    // SAFETY: `entry` is mapped EL0-executable, per this function's own safety contract.
    unsafe { write_elr_el1(ElrEl1::empty().with_addr(entry as u64)) };
    // SAFETY: `argv` is a valid EL0 stack pointer, per this function's own safety contract. No
    // `arm_sysregs` accessor covers SP_EL0.
    unsafe { core::arch::asm!("msr sp_el0, {sp}", sp = in(reg) argv as u64) };

    // SAFETY: SPSR_EL1/ELR_EL1/SP_EL0 are all set, per `enter_el0`'s own contract. Inline asm
    // (rather than a plain `enter_el0()` call) specifically so x0-x2 are loaded in the same
    // instruction sequence that calls it, with nothing of Rust's own codegen between the two free
    // to reuse those (ordinarily caller-saved) registers first -- AAPCS64 passes the first three
    // integer arguments in x0-x2, which is exactly the argc/argv/envp convention a program's own
    // `_start` expects to find them in. `inout("x0")`, not a separate `in`/`lateout` pair for the
    // same register (not even expressible -- a register names at most one operand): `x0` seeds
    // `argc` going in, exactly as before, and `context.s`'s register restore never touches `x0` on
    // the way back (`resume_kernel`'s own doc comment), so whatever `resume_kernel` was called
    // with is still sitting there once this `bl` "returns", read out as `exit_code`.
    let exit_code: usize;
    unsafe {
        core::arch::asm!(
            "bl {enter}",
            inout("x0") argc => exit_code,
            in("x1") argv,
            in("x2") envp,
            enter = sym enter_el0,
            clobber_abi("C"),
        );
    }
    let exit_code = exit_code as i32;

    // Back from the program: drop whatever fds it still had open (its `write`/`read`'s own table,
    // untouched by the longjmp above) before anything else can look at fd numbers again.
    crate::syscall::fd::end_launch();

    // Back from the program (its `exit` or a fault, both taken as exceptions, so IRQs are masked
    // again regardless of what the program itself last had them set to) -- unmask unconditionally
    // so the kernel resumes normal operation, exactly as if this had been an ordinary call.
    irq::unmask();

    exit_code
}

/// Writes the argument and environment strings onto the stack below `sp` as NUL-terminated
/// strings, plus a `NULL`-terminated array of pointers for each (`argplan::plan` computes the
/// layout). Returns `(argv, envp)`, the arrays' addresses; `argv` is also the new stack pointer.
/// `None` if the plan says everything wouldn't fit above `floor`.
///
/// # Safety
/// `sp`/`floor` must lie inside the user window `elf::load` just mapped, and nothing may be
/// running in it.
unsafe fn push_strings(
    sp: usize,
    floor: usize,
    args: &[&str],
    env: &[&str],
) -> Option<(usize, usize)> {
    let plan = argplan::plan(sp, floor, &lens(args), &lens(env))?;

    for (item, &addr) in args
        .iter()
        .chain(env)
        .zip(plan.args.iter().chain(&plan.env))
    {
        // SAFETY: `plan` keeps every string and both arrays within `floor..sp`, per this
        // function's own safety contract.
        unsafe {
            core::ptr::copy_nonoverlapping(item.as_ptr(), addr as *mut u8, item.len());
            *((addr + item.len()) as *mut u8) = 0;
        }
    }

    // SAFETY: each array's `len + 1` slots were carved out of the window by `plan`; the last is
    // the terminating `NULL`.
    unsafe {
        for (array, addrs) in [(plan.argv, &plan.args), (plan.envp, &plan.env)] {
            let array = array as *mut usize;
            for (i, &addr) in addrs.iter().enumerate() {
                *array.add(i) = addr;
            }
            *array.add(addrs.len()) = 0;
        }
    }
    Some((plan.argv, plan.envp))
}

fn lens(items: &[&str]) -> Vec<usize> {
    items.iter().map(|s| s.len()).collect()
}

/// Maps `start..end` read/write (never executable -- `elf::data_attributes`) and zeroes it. Used
/// for both the stack and the fixed heap: neither gets a segment's "final, more restrictive" pass
/// the way `elf::load`'s own mappings do, since both stay read/write for as long as the program
/// runs.
fn map_and_zero(start: usize, end: usize) -> Result<(), ElfError> {
    mmu::set_user_pages(start, end, Some(elf::data_attributes()))
        .map_err(|_| ElfError::MapFailed)?;
    // The write below touches a page marked USER, which PAN would otherwise forbid -- `elf::load`
    // took its own guard for the same reason, but that one already dropped when it returned to
    // this function's caller.
    let _user = mmu::user_access();
    // SAFETY: just mapped writable, and nothing else uses this range while a program loads.
    unsafe { core::ptr::write_bytes(start as *mut u8, 0, end - start) };
    Ok(())
}

/// Loads `elf_bytes`, gives it a fresh stack and heap with `args` as its `argv` and `env`
/// (`NAME=VALUE` strings) as its `envp`, binds `stdio`'s redirections (if any) onto fd 0/1/2, and
/// runs it at EL0 to completion, then returns normally -- `elf::load` maps the program's own
/// segments; this additionally maps its stack and heap, writes `args`/`env` onto the stack
/// (`push_strings`), and calls `enter`. `stdio[i]` is `None` for "leave fd `i` at whatever
/// `reset_for_launch` set it to" (`Keyboard`/`Console`/`Console`) -- `shell`'s own launch code is
/// the only caller that ever passes anything else.
///
/// # Errors
/// Returns `Err` without entering EL0 at all if the ELF is invalid (`LaunchError::Elf`), if
/// `args`/`env` together don't fit in `ARG_MAX` (`LaunchError::ArgsTooBig`), if mapping the stack
/// or heap fails (`LaunchError::Elf(ElfError::MapFailed)`), or if a `stdio` redirection couldn't
/// be bound (`LaunchError::BadRedirect`). `Ok` carries the program's own real exit status --
/// `enter`'s own doc comment on where that value actually comes from.
pub fn run_program(
    elf_bytes: &[u8],
    args: &[&str],
    env: &[&str],
    stdio: [Option<crate::syscall::fd::StdioRedirect>; 3],
) -> Result<i32, LaunchError> {
    let floor = USER_STACK_TOP - ARG_MAX;

    // Dry run first: a list that can't fit must not cost a load.
    argplan::plan(USER_STACK_TOP, floor, &lens(args), &lens(env)).ok_or(LaunchError::ArgsTooBig)?;

    let entry = elf::load(elf_bytes)?;

    // The stack and the fixed heap, mapped and zeroed the same way a segment is while being
    // filled -- see `elf::data_attributes`. Neither is locked down afterward the way a segment is:
    // both are always read/write, never executable, for as long as the program runs. The heap has
    // nothing in it yet (`userlib`'s allocator starts empty and only ever grows into it), but
    // zeroing it anyway costs nothing and means a program can never read a previous program's
    // leftover heap bytes if it somehow reads before writing.
    map_and_zero(USER_STACK_BOTTOM, USER_STACK_TOP)?;
    map_and_zero(USER_HEAP_BOTTOM, USER_HEAP_TOP)?;

    let (argv, envp) = {
        // The writes inside `push_strings` touch pages marked USER, which PAN would otherwise
        // forbid.
        let _user = mmu::user_access();
        // SAFETY: `USER_STACK_TOP`/`floor` lie inside the stack window just mapped and zeroed
        // above; nothing else touches it until `enter` below runs the program.
        unsafe { push_strings(USER_STACK_TOP, floor, args, env) }.ok_or(LaunchError::ArgsTooBig)?
    };

    // The standard three fds first, nothing left over from whatever ran before (already dropped
    // by `enter`'s own `end_launch` call once that program returned); `bind_stdio` right after
    // then overrides whichever of those `stdio` actually names, before anything below can fail --
    // once this returns `Ok`, the program is considered launched, so nothing after this point may
    // fail either.
    crate::syscall::fd::reset_for_launch();
    crate::syscall::fd::bind_stdio(stdio).map_err(|()| LaunchError::BadRedirect)?;

    // SAFETY: `entry` came from `elf::load`, which only ever returns an address inside a segment
    // it just mapped EL0-executable; `argv`/`envp` came from `push_strings`, which wrote them (and
    // everything they point to) inside the same stack window.
    Ok(unsafe { enter(entry, args.len(), argv, envp) })
}
