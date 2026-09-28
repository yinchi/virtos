use arm_gic::{IntId, InterruptGroup, gicv2::GicV2};
use core::sync::atomic::{AtomicUsize, Ordering};

/// arbitrary priority value, all interrupts will have the same priority
const INTERRUPT_PRIORITY: u8 = 0xa0;

/// Base address of the GIC Distributor, set once by `init_gic`.
static GICD_BASE: AtomicUsize = AtomicUsize::new(0);
/// Base address of the GIC CPU Interface, set once by `init_gic`.
static GICC_BASE: AtomicUsize = AtomicUsize::new(0);

/// Returns a fresh `GicV2` wrapping the same MMIO windows `init_gic` was given. See this module's
/// doc comment for why a fresh instance, not a shared one, is constructed every time.
pub fn gic() -> GicV2<'static> {
    let gicd = GICD_BASE.load(Ordering::Relaxed);
    let gicc = GICC_BASE.load(Ordering::Relaxed);
    assert!(gicd != 0 && gicc != 0, "gic() called before init_gic");
    // SAFETY: gicd/gicc were mapped (mmu::map_device) and validated by init_gic before being
    // stored; nothing else ever writes GICD_BASE/GICC_BASE.
    unsafe { GicV2::new(gicd as *mut _, gicc as *mut _) }
}

/// Initialize the GIC with the specified distributor and CPU interface addresses.
///
/// Callers must map both windows first (see `mmu::map_device`, and their sizes in the DTB's
/// `arm,cortex-a15-gic` node), since this touches both immediately.
pub fn init_gic(gicd: usize, gicc: usize) {
    GICD_BASE.store(gicd, Ordering::Relaxed);
    GICC_BASE.store(gicc, Ordering::Relaxed);

    let mut gic = gic();
    gic.setup();
    gic.set_priority_mask(0xff);

    // TODO: enable individual SPIs (`enable_spi`) and unmask IRQs (PSTATE.I) once a real device
    // needs one -- there is nothing to interrupt yet.
}

/// Enable the specified SPI (Shared Peripheral Interrupt)
pub fn enable_spi(spi: u32) {
    let mut gic = gic();
    let id = IntId::spi(spi);
    gic.set_interrupt_priority(id, INTERRUPT_PRIORITY); // arbitrary priority value
    gic.enable_interrupt(id, true)
        .expect("Failed to enable SPI in");
}

/// Called from `vectors.s` on every IRQ taken at EL1 (`irq_el1h`).
#[unsafe(no_mangle)]
extern "C" fn irq_handler() {
    let mut gic = gic();
    // Group0: matches this QEMU config (no GIC Security Extensions), which uses the plain
    // IAR/EOIR registers rather than the separate Group1 AIAR/AEOIR ones.
    if let Some(intid) = gic.get_and_acknowledge_interrupt(InterruptGroup::Group0) {
        gic.end_interrupt(intid, InterruptGroup::Group0);
    }
}
