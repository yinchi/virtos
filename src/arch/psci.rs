//! Powers off or restarts the machine via PSCI (Power State Coordination Interface) -- the firmware
//! service QEMU's `virt` machine implements above EL1, reached with `hvc` (hypervisor call), the
//! conduit `virt` uses by default.

/// PSCI function IDs for `SYSTEM_OFF`/`SYSTEM_RESET` (the 32-bit SMC calling convention QEMU's
/// `virt` machine implements).
const PSCI_SYSTEM_OFF: u64 = 0x8400_0008;
const PSCI_SYSTEM_RESET: u64 = 0x8400_0009;

/// Powers off the machine. Documented by PSCI never to return; the trailing loop is defensive
/// only, in case something intercepts the call without actually powering off the board.
pub fn system_off() -> ! {
    psci_call(PSCI_SYSTEM_OFF);
    loop {
        unsafe { core::arch::asm!("wfe") }
    }
}

/// Restarts the machine. Same defensive fallback as `system_off`.
pub fn system_reset() -> ! {
    psci_call(PSCI_SYSTEM_RESET);
    loop {
        unsafe { core::arch::asm!("wfe") }
    }
}

/// Issues `hvc #0` with `function` in `x0`, per PSCI's SMCCC-based calling convention: up to four
/// arguments in `x0`-`x3`, all clobbered by the call. Neither caller above passes further
/// arguments, so `x1`-`x3` are left undefined on entry.
fn psci_call(function: u64) {
    // SAFETY: HVC #0 traps to the firmware/hypervisor layer QEMU's `virt` machine runs above EL1,
    // which implements PSCI. Not marked `noreturn`: `SYSTEM_OFF`/`SYSTEM_RESET` are documented
    // never to return, but that's a property of the call, not something the compiler can assume --
    // both callers above still fall through to their own defensive loop in case it somehow does.
    unsafe {
        core::arch::asm!(
            "hvc #0",
            in("x0") function,
            out("x1") _,
            out("x2") _,
            out("x3") _,
            options(nomem, nostack),
        );
    }
}
