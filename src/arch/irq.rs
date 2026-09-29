//! Masking interrupts for code that shares state with the IRQ handler, and unmasking them at all
//! in the first place.

/// Unmasks IRQs at this PE (`daifclr #2` clears the I bit). Two callers: `kernel_main`, once, and
/// `exec::process::enter`, after a program returns: any AArch64 exception entry unconditionally
/// re-masks IRQs, and without this, the kernel would carry on with IRQs masked forever after the
/// first program exits.
pub fn unmask() {
    // SAFETY: just a DAIF write; the exception vector table (`vectors.s`) is already installed by
    // the time either caller runs.
    unsafe { core::arch::asm!("msr daifclr, #2") };
}

/// Masks IRQs at this PE (`daifset #2` sets the I bit) -- the other half of `unmask`, for
/// `exec::process::enter` to bracket the handoff into EL0 with, from the first write to
/// `SPSR_EL1`/`ELR_EL1`/`SP_EL0` through the `eret` that consumes them. Matches the reference
/// lab's own `run()`, which masks for exactly the same span.
pub fn mask() {
    // SAFETY: just a DAIF write.
    unsafe { core::arch::asm!("msr daifset, #2") };
}

/// Runs `f` with IRQs masked, then restores the previous mask -- safe to call whether IRQs were
/// already on (the common case) or already off (e.g. from inside another IRQ handler).
pub fn without_irqs<R>(f: impl FnOnce() -> R) -> R {
    let daif: u64;
    // SAFETY: reads DAIF, then masks IRQs (`daifset #2` sets the I bit).
    unsafe { core::arch::asm!("mrs {d}, daif", "msr daifset, #2", d = out(reg) daif) };
    let result = f();
    const I: u64 = 1 << 7;
    if daif & I == 0 {
        // SAFETY: IRQs were unmasked when we came in; `daifclr #2` clears the I bit again.
        unsafe { core::arch::asm!("msr daifclr, #2") };
    }
    result
}
