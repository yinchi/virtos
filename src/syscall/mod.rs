//! `SVC`-based syscalls and the fault path, both reached from `sync_el0_64` (`vectors.s`) via
//! `sync_el0_handler`.
//!
//! `exit`, `read`, `write`, `open`, `close`, `getdents`, `mkdir`, `unlink`, `rename`, `getcwd`,
//! `stat`, `chmod`, `utimensat`, `mount`, `umount`, `blkinfo`, `clock_gettime`, `clear` and
//! `reboot`. A failure is a negated errno in `x0` (`abi::errno`); a number that is no syscall is
//! `ENOSYS`.

mod clock;
pub mod fd;
mod power;

use arm_sysregs::el1::accessors::{read_esr_el1, read_far_el1};
use core::fmt::Write;

use crate::exec::process;
use abi::errno::ENOSYS;
use abi::{
    SYS_BLKINFO, SYS_CHMOD, SYS_CLEAR, SYS_CLOCK_GETTIME, SYS_CLOSE, SYS_EXIT, SYS_GETCWD,
    SYS_GETDENTS, SYS_MKDIRAT, SYS_MOUNT, SYS_NEWFSTATAT, SYS_OPEN, SYS_READ, SYS_REBOOT,
    SYS_RENAMEAT, SYS_UMOUNT2, SYS_UNLINKAT, SYS_UTIMENSAT, SYS_WRITE,
};

// The ESR_EL1 EC field values this handler decodes -- see `sync_el0_handler` for what each one
// means here.
const EC_SVC64: u8 = 0x15;
const EC_IABT_LOWER: u8 = 0x20;
const EC_DABT_LOWER: u8 = 0x24;

/// The exit status a caught fault reports as its "`exit` code" -- `128 + SIGSEGV`, matching the
/// reference lab's own convention (a real Unix exit status for a signal-killed process).
const FAULT_EXIT_CODE: i32 = 139;

/// Mirrors `kernel_entry`'s stack layout exactly (`vectors.s`): `x0`-`x29`, then the `ELR_EL1`/
/// `SPSR_EL1` pair, then `x30` (padded to keep the frame 16-byte aligned). `sync_el0_64` passes
/// the frame's base address in `x0` before calling `sync_el0_handler`, so this struct is how Rust
/// reads a syscall's number/arguments back out of it, and would write a return value back in
/// before `kernel_exit` restores everything -- moot for `exit` below, which never reaches
/// `kernel_exit` at all.
#[repr(C)]
struct TrapFrame {
    x: [u64; 30],
    elr_el1: u64,
    spsr_el1: u64,
    lr: u64,
    _pad: u64,
}

/// Called from `sync_el0_64` with the trap frame's address in `x0`. Decodes `ESR_EL1`'s `EC`
/// field to tell a deliberate syscall apart from a fault.
#[unsafe(no_mangle)]
extern "C" fn sync_el0_handler(regs: *mut TrapFrame) {
    let esr = read_esr_el1();
    match esr.ec() {
        EC_SVC64 => {
            // SAFETY: `regs` points at `kernel_entry`'s just-saved frame, still live on the
            // exception stack -- sole access to it here.
            let regs = unsafe { &mut *regs };
            let nr = regs.x[8] as usize; // syscall number
            let (a0, a1, a2, a3) = (
                regs.x[0] as usize,
                regs.x[1] as usize,
                regs.x[2] as usize,
                regs.x[3] as usize,
            );
            match nr {
                SYS_READ => regs.x[0] = fd::read(a0, a1, a2) as u64,
                SYS_WRITE => regs.x[0] = fd::write(a0, a1, a2) as u64,
                SYS_OPEN => regs.x[0] = fd::open(a0, a1, a2) as u64,
                SYS_CLOSE => regs.x[0] = fd::close(a0) as u64,
                SYS_GETDENTS => regs.x[0] = fd::getdents(a0, a1, a2) as u64,
                SYS_MKDIRAT => regs.x[0] = fd::mkdir(a0, a1) as u64,
                SYS_UNLINKAT => regs.x[0] = fd::unlink(a0, a1) as u64,
                SYS_RENAMEAT => regs.x[0] = fd::rename(a0, a1, a2, a3) as u64,
                SYS_GETCWD => regs.x[0] = fd::getcwd(a0, a1) as u64,
                SYS_NEWFSTATAT => regs.x[0] = fd::stat(a0, a1, a2) as u64,
                SYS_CHMOD => regs.x[0] = fd::chmod(a0, a1, a2, a3) as u64,
                SYS_UTIMENSAT => regs.x[0] = fd::utimensat(a0, a1, a2) as u64,
                SYS_MOUNT => regs.x[0] = fd::mount(a0, a1, a2, a3) as u64,
                SYS_UMOUNT2 => regs.x[0] = fd::umount(a0, a1) as u64,
                SYS_BLKINFO => regs.x[0] = fd::blkinfo(a0, a1) as u64,
                SYS_CLOCK_GETTIME => regs.x[0] = clock::time() as u64,
                SYS_CLEAR => regs.x[0] = fd::clear() as u64,
                SYS_REBOOT => regs.x[0] = power::reboot(a0) as u64,
                SYS_EXIT => {
                    // Never returns to kernel_exit's normal eret-back-to-EL0 path --
                    // resume_kernel (context.s) restores the register set enter_el0
                    // checkpointed and jumps straight back into `exec::process::enter`'s call
                    // site instead, with `a0` (the program's own real exit code) becoming
                    // `enter`'s return value -- see `resume_kernel`'s own doc comment for how.
                    // SAFETY: only reachable once `exec::process::enter` has actually started a
                    // program (context.s's KERNEL_CTX holds a real checkpoint, not its zeroed
                    // initial state).
                    unsafe { process::resume_kernel(a0 as i32) }
                }
                _ => regs.x[0] = ENOSYS as u64, // no such syscall
            }
        }

        // Instruction and data aborts from EL0, respectively.
        EC_IABT_LOWER | EC_DABT_LOWER => {
            let far = read_far_el1();
            let mut uart = crate::bare_metal::get_uart();
            let _ = writeln!(
                uart,
                "Segmentation fault (address {far:#x}, ESR_EL1 {esr:?})"
            );
            // SAFETY: as in the `SYS_EXIT` arm above. `FAULT_EXIT_CODE`, not the program's own
            // choice -- there's no real exit code to read from a program that never called
            // `exit` at all.
            unsafe { process::resume_kernel(FAULT_EXIT_CODE) }
        }

        // Anything else (FP exceptions, alignment faults, ...) -- deliberately narrow decoding,
        // not a general fault-handling framework. `8` is `sync_el0_64`'s own index into
        // `unexpected_exception`'s vector-name table.
        _ => unsafe { unexpected_exception(8) },
    }
}

unsafe extern "C" {
    fn unexpected_exception(vector: usize) -> !;
}
