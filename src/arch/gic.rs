use arm_gic::{IntId, gicv2::GicV2};
use spin::{Mutex, Once};

/// Global instance of the GIC (Generic Interrupt Controller) accessor
static GIC: Once<Mutex<GicV2>> = Once::new();

/// arbitrary priority value, all interrupts will have the same priority
const INTERRUPT_PRIORITY: u8 = 0xa0;

/// Initialize the GIC with the specified distributor and CPU interface addresses.
///
/// Callers must map both windows first (see `mmu::map_device`, and their sizes in the DTB's
/// `arm,cortex-a15-gic` node), since this touches both immediately.
pub fn init_gic(gicd: usize, gicc: usize) {
    let mut gic = unsafe { arm_gic::gicv2::GicV2::new(gicd as *mut _, gicc as *mut _) };
    gic.setup();
    gic.set_priority_mask(0xff);
    GIC.call_once(|| Mutex::new(gic));

    // TODO: Enable the GIC distributor and CPU interface, and individual interrupts as needed.
}

/// Returns a reference to the global GIC accessor.
pub fn gic() -> &'static Mutex<GicV2<'static>> {
    GIC.wait()
}

/// Enable the specified SPI (Shared Peripheral Interrupt)
pub fn enable_spi(spi: u32) {
    let mut gic = GIC.wait().lock();
    let id = IntId::spi(spi);
    gic.set_interrupt_priority(id, INTERRUPT_PRIORITY); // arbitrary priority value
    gic.enable_interrupt(id, true)
        .expect("Failed to enable SPI in");
}
