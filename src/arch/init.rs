use fdt::Fdt;

use crate::arch::rtc::RTC;

/// Base address of the UART device.
pub const UART_BASE: usize = 0x0900_0000;

/// Max size of the DTB QEMU generates for the `virt` machine; observed to be exactly this size.
const DTB_BUF_SIZE: usize = 0x0010_0000; // 1MiB
static mut DTB_BUFFER: [u8; DTB_BUF_SIZE] = [0; DTB_BUF_SIZE];

/// Initialize the various devices and store their base addresses.
pub fn init_devices(
    dtb_ptr: usize, // Pointer to the device tree blob (DTB)
    mut writer: impl core::fmt::Write,
) {
    writeln!(
        writer,
        "Initializing base addresses with DTB pointer: {:#x}",
        dtb_ptr
    )
    .unwrap_or_default();

    let src = dtb_ptr as *const u8;

    // `Fdt::from_ptr` unconditionally rejects a null pointer, but our DTB genuinely lives at
    // address 0x0 (QEMU places it in flash there since our kernel occupies RAM start). Read the
    // header's `totalsize` field via a raw pointer read -- not a reference -- so this works even
    // though `src` is null, then copy the DTB into an owned buffer and hand `Fdt::new` a normal
    // slice, which has no such null check.
    let totalsize = u32::from_be(unsafe { (src.add(4) as *const u32).read_unaligned() }) as usize;
    assert!(
        totalsize <= DTB_BUF_SIZE,
        "device tree blob ({totalsize} bytes) exceeds DTB_BUF_SIZE ({DTB_BUF_SIZE} bytes)"
    );

    // SAFETY: `&raw mut` forms a pointer without creating a reference, so this alone needs no
    // unsafe. The copy and slice below do need it: `dst` is non-null (a real static's address),
    // so both are sound as long as nothing else touches DTB_BUFFER concurrently -- true here,
    // since this whole function only ever runs once.
    let dst = &raw mut DTB_BUFFER as *mut u8;
    let data = unsafe {
        core::ptr::copy_nonoverlapping(src, dst, totalsize);
        core::slice::from_raw_parts(dst, totalsize)
    };

    let fdt = Fdt::new(data).unwrap_or_else(|_| panic!("Failed to parse FDT"));

    let mut gic_regions = fdt
        .find_compatible(&["arm,cortex-a15-gic"])
        .and_then(|node| node.reg())
        .expect("Cannot find Generic Interrupt Controller (GIC)!");
    let gicd = gic_regions
        .next()
        .expect("Cannot find GIC distributor address")
        .starting_address as usize;
    let gicc = gic_regions
        .next()
        .expect("Cannot find GIC controller address")
        .starting_address as usize;

    super::gic::init_gic(gicd, gicc);

    let rtc = fdt
        .find_compatible(&["arm,pl031"])
        .and_then(|node| node.reg())
        .and_then(|mut regions| regions.next())
        .expect("Cannot find Real-Time Clock (RTC) address")
        .starting_address as usize;

    RTC.call_once(|| unsafe { arm_pl031::Rtc::new(rtc as _) });
}
