use fdt::Fdt;

use crate::arch::mmu;
use crate::arch::rtc::RTC;

/// Base address of the UART device: hard-coded, unlike every other device's, since it's used to
/// print the very first boot message, before the DTB is even read.
pub const UART_BASE: usize = 0x0900_0000;
/// Fallback size of the UART's memory-mapped I/O region, used only if it can't be extracted from
/// the DTB.
const UART_SIZE_FALLBACK: usize = 0x1000;

/// Max size of the DTB QEMU generates for the `virt` machine; observed to be exactly this size.
const DTB_BUF_SIZE: usize = 0x0010_0000; // 1MiB
static mut DTB_BUFFER: [u8; DTB_BUF_SIZE] = [0; DTB_BUF_SIZE];

/// Initialize the various devices and store their base addresses.
///
/// Turns the MMU on first (`mmu::enable`, which maps only the kernel's own image), then maps
/// and brings up each device in turn -- UART first, since `writer` keeps being used for the rest
/// of this function and beyond, then the GIC and RTC once the DTB has located them.
pub fn init_devices(
    dtb_ptr: usize, // Pointer to the device tree blob (DTB)
    mut writer: impl core::fmt::Write,
) -> mmu::Hardening {
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

    // From here on, every access must go through a mapping: `enable` turns translation on,
    // mapping only the kernel's own image. `writer` is a live pointer into UART MMIO created
    // before this call and still used below (and by the caller, after this function returns),
    // so it's mapped immediately -- nothing fallible runs between `enable` and this call, so a
    // panic anywhere below (or in the caller) can still reach the UART to report it.
    let hardening = mmu::enable();

    // UART's size, from the same DTB node whose address is already hard-coded above -- falling
    // back to `UART_SIZE_FALLBACK` keeps the boot message printable even if that node is somehow
    // missing, the same reason `UART_BASE` isn't derived from the DTB at all.
    let uart_size = fdt
        .find_compatible(&["arm,pl011"])
        .and_then(|node| node.reg())
        .and_then(|mut regions| regions.next())
        .and_then(|region| region.size)
        .unwrap_or(UART_SIZE_FALLBACK);
    mmu::map_device(UART_BASE, uart_size);

    let mut gic_regions = fdt
        .find_compatible(&["arm,cortex-a15-gic"])
        .and_then(|node| node.reg())
        .expect("Cannot find Generic Interrupt Controller (GIC)!");
    let gicd = gic_regions
        .next()
        .expect("Cannot find GIC distributor address");
    let gicc = gic_regions
        .next()
        .expect("Cannot find GIC CPU interface address");

    mmu::map_device(
        gicd.starting_address as usize,
        gicd.size
            .expect("GIC distributor has no size in its DTB `reg` entry"),
    );
    mmu::map_device(
        gicc.starting_address as usize,
        gicc.size
            .expect("GIC CPU interface has no size in its DTB `reg` entry"),
    );
    super::gic::init_gic(
        gicd.starting_address as usize,
        gicc.starting_address as usize,
    );

    let rtc = fdt
        .find_compatible(&["arm,pl031"])
        .and_then(|node| node.reg())
        .and_then(|mut regions| regions.next())
        .expect("Cannot find Real-Time Clock (RTC) address");

    mmu::map_device(
        rtc.starting_address as usize,
        rtc.size.expect("RTC has no size in its DTB `reg` entry"),
    );
    RTC.call_once(|| unsafe { arm_pl031::Rtc::new(rtc.starting_address as _) });

    hardening
}
