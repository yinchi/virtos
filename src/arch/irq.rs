//! Masking interrupts for code that shares state with the IRQ handler, and the one-time switch
//! that turns them on in the first place.

/// Unmasks IRQs at this PE (`daifclr #2` clears the I bit). QEMU's direct-kernel boot leaves DAIF
/// in its reset state -- everything masked -- and nothing before this call ever needed otherwise,
/// since every device so far (`blk`, `gpu`, the timer's `now()`) was polled rather than
/// interrupt-driven. Call once, after every interrupt source that might already be pending has
/// been configured (`arch::init::init_devices`, which enables the keyboard's SPI via `discover`).
pub fn enable() {
    // SAFETY: just a DAIF write; the exception vector table (`vectors.s`) is already installed by
    // the time `init_devices` (and therefore this) runs.
    unsafe { core::arch::asm!("msr daifclr, #2") };
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
