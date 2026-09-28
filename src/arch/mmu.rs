//! Builds the page table and turns the MMU on: one identity-mapped table under `TTBR0_EL1`,
//! covering only the kernel's own image (split by `link.ld`'s boundary symbols into RX/RO/RW+XN
//! regions, the stack set apart from `.bss` by an unmapped guard) -- with `TTBR1_EL1` left
//! unused. `enable` maps nothing else: it doesn't take a single device address, and knows
//! nothing about the GIC, UART, RTC or anything else. Every device's MMIO window is mapped by
//! `map_device` instead, called by that device's own bring-up code right before it's first
//! touched (see `init::init_devices`) -- so this module never needs updating just because a new
//! device was added. This project runs at most one CPU with a single address space, so there's
//! only one ASID and one page table, ever.

use aarch64_paging::{
    descriptor::El1Attributes,
    idmap::IdMap,
    paging::{El1And0, MemoryRegion},
};
use arm_sysregs::el0::{accessors::write_pan, registers::Pan};
use arm_sysregs::el1::{
    accessors::{
        read_id_aa64mmfr0_el1, read_id_aa64mmfr1_el1, read_sctlr_el1, write_mair_el1,
        write_sctlr_el1, write_tcr_el1,
    },
    registers::{MairEl1, SctlrEl1, TcrEl1},
};
use spin::Mutex;

/////////////////////////////////////////////////////////////////////////////////////////////
// MAIR_EL1 attribute indexes: there are 8 1-byte slots, of which we use 0 and 1.

/// Attribute index for device memory in MAIR_EL1.
const ATTR_DEVICE_INDEX: u32 = 0;
/// Attribute index for normal memory in MAIR_EL1.
const ATTR_NORMAL_INDEX: u32 = 1;

/////////////////////////////////////////////////////////////////////////////////////////////

// Kernel image boundary symbols, defined in link.ld.
unsafe extern "C" {
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __data_start: u8;
    static __data_end: u8;
    static __stack_bottom: u8;
    static __kernel_end: u8;
}

/// Root level of the page table. A level-1 root holds up to 512 1 GiB entries, i.e. 512^3
/// 4 KiB pages.
const ROOT_LEVEL: usize = 1;

/// Address Space Identifier (ASID) for the root page table.
/// Only ever one address space -- no per-process ASIDs needed.
const ASID: usize = 0;

/// The page table `enable` builds and activates, and every later `map_device` call extends.
/// Locked rather than a bare `static mut`: `map_device` only ever needs the table for one
/// `map_range` call at a time and never holds it across anything else, so a lock costs nothing
/// here and comes with no deadlock risk -- unlike `gic.rs`'s `GicV2`, nothing touches this from
/// an interrupt handler.
static IDMAP: Mutex<Option<IdMap<El1And0>>> = Mutex::new(None);

/// What `enable` turned on beyond translation itself -- reported at boot.
pub struct Hardening {
    pub pan: bool,
}

/// Extracts the address of a linker-defined boundary symbol.
///
/// # Safety
/// `s` must be one of this module's `extern "C"` boundary symbols.
unsafe fn sym_addr(s: &u8) -> usize {
    s as *const u8 as usize
}

/// Turns the MMU on: maps only the kernel's own image, then activates translation. Called once,
/// very early in `kernel_main` -- before any device is touched, DTB included, since the DTB
/// itself lives outside the kernel image and would fault to read afterwards. Every device maps
/// itself in with `map_device` when it's actually brought up.
pub fn enable() -> Hardening {
    // Device-nGnRE: non-Gathering, non-Reordering, no Early write acknowledgement -- the
    // standard "MMIO register access" encoding.
    const MAIR_DEVICE_NGNRE: u8 = 0b0000_0100;
    // Normal memory, Inner/Outer Write-Back, Read/Write-Allocate, Non-transient.
    const MAIR_NORMAL: u8 = 0xff;

    // SAFETY: the only write to MAIR_EL1 in the whole program, and it happens before the MMU
    // (which consults it on every translation) is switched on below.
    unsafe {
        write_mair_el1(
            MairEl1::empty()
                .with_attr(ATTR_DEVICE_INDEX, MAIR_DEVICE_NGNRE)
                .with_attr(ATTR_NORMAL_INDEX, MAIR_NORMAL),
        );
    }

    let mut idmap = IdMap::with_asid(ASID, ROOT_LEVEL, El1And0);

    let kernel_base = El1Attributes::ATTRIBUTE_INDEX_1
        | El1Attributes::INNER_SHAREABLE
        | El1Attributes::VALID
        | El1Attributes::ACCESSED;
    let kernel_rx = kernel_base | El1Attributes::READ_ONLY | El1Attributes::UXN;
    let kernel_ro = kernel_rx | El1Attributes::PXN;
    let kernel_rw = kernel_base | El1Attributes::UXN | El1Attributes::PXN;

    // The kernel's own image, split by link.ld's boundary symbols. Nothing else -- not even a
    // device known at this point (there is none) -- is ever mapped here.
    unsafe {
        idmap
            .map_range(
                &MemoryRegion::new(sym_addr(&__text_start), sym_addr(&__text_end)),
                kernel_rx,
            )
            .unwrap();
        idmap
            .map_range(
                &MemoryRegion::new(sym_addr(&__rodata_start), sym_addr(&__rodata_end)),
                kernel_ro,
            )
            .unwrap();
        idmap
            .map_range(
                &MemoryRegion::new(sym_addr(&__data_start), sym_addr(&__data_end)),
                kernel_rw,
            )
            .unwrap();
        // The stack, with the guard below it (up to `__stack_bottom`) deliberately left
        // unmapped: an overflow is a translation fault, not silent corruption of `.bss`.
        idmap
            .map_range(
                &MemoryRegion::new(sym_addr(&__stack_bottom), sym_addr(&__kernel_end)),
                kernel_rw,
            )
            .unwrap();
    }

    // TCR_EL1: the translation regime the table above was built for. Nothing else in the
    // project sets it up, and the page table does nothing until the MMU is on, so it all has to
    // be said here.
    //
    // - T0SZ = 25: a 39-bit address space, 512 GiB, which a level-1 root of 512 1 GiB entries
    //   covers exactly (`ROOT_LEVEL`).
    // - TG0 = 4 KiB granule; IRGN0/ORGN0 = write-back, read/write-allocate caching for the table
    //   walks themselves; SH0 = inner shareable.
    // - EPD1 = 1: no TTBR1_EL1 walks, since high memory addresses are never used.
    // - IPS: the physical address size the CPU supports (ID_AA64MMFR0_EL1.PARange).
    const T0SZ: u8 = 25;
    const IRGN0_WB_WA: u8 = 0b01;
    const ORGN0_WB_WA: u8 = 0b01;
    const SH0_INNER: u8 = 0b11;
    const TG0_4K: u8 = 0b00;
    let parange = read_id_aa64mmfr0_el1().parange();
    // SAFETY: the only write to TCR_EL1, describing exactly the table just built, before the
    // MMU is switched on below.
    unsafe {
        write_tcr_el1(
            TcrEl1::empty()
                .with_t0sz(T0SZ)
                .with_irgn0(IRGN0_WB_WA)
                .with_orgn0(ORGN0_WB_WA)
                .with_sh0(SH0_INNER)
                .with_tg0(TG0_4K)
                .with_ips(parange)
                | TcrEl1::EPD1,
        );
    }

    // SAFETY: every mapping above covers exactly what this kernel accesses so far -- its own
    // image. Devices and the rest of RAM are deliberately left unmapped here instead of mapped
    // and unused; `activate` points TTBR0_EL1 at the table.
    unsafe {
        idmap.activate();
    }

    // Turn translation on (SCTLR_EL1.M), together with the data and instruction caches (C, I).
    // Before this the CPU ignored the page table entirely: every address was its own physical
    // address. Stale TLB entries can't exist yet, but the barrier sequence is the one the
    // architecture requires around the write.
    // SAFETY: the table maps the code executing this instruction and everything it touches next.
    unsafe {
        core::arch::asm!("dsb ish", "isb");
    }
    // Beyond turning translation on, the checks that catch our own mistakes early: WXN (nothing
    // is ever both writable and executable -- no mapping here is), and stack-pointer alignment
    // checks (the AArch64 ABI keeps SP 16-byte aligned; every stack this project sets up does).
    let mut sctlr = read_sctlr_el1()
        | SctlrEl1::M
        | SctlrEl1::C
        | SctlrEl1::I
        | SctlrEl1::WXN
        | SctlrEl1::SA
        | SctlrEl1::SA0;

    // PAN, if the CPU has it (ID_AA64MMFR1_EL1.PAN): with it set, the kernel faults on any
    // access to a page EL0 may access, so a stray kernel dereference of a user pointer is a
    // fault instead of a silent success. SPAN = 0 makes every exception entry set it again.
    let pan = read_id_aa64mmfr1_el1().is_feat_pan_present();
    if pan {
        sctlr &= !SctlrEl1::SPAN;
    }
    // SAFETY: as above.
    unsafe {
        write_sctlr_el1(sctlr);
        core::arch::asm!("isb");
    }
    if pan {
        // SAFETY: only reached when the CPU has FEAT_PAN, checked above.
        unsafe { write_pan(Pan::PAN) };
    }

    // Keeping IDMAP alive from here on is not optional: `IdMap`'s `Drop` impl panics if an
    // active mapping is ever dropped, since that would free memory the CPU is still using as
    // its page table.
    *IDMAP.lock() = Some(idmap);
    Hardening { pan }
}

/// Maps `base..base+size` as device (MMIO) memory -- non-cacheable, never executable at any
/// privilege level. Called by a device's own bring-up code right before its first register
/// access (see `init::init_devices`): `enable` maps nothing but the kernel's own image, so every
/// device is responsible for mapping its own window in, once, before it's touched.
///
/// # Panics
/// If called before `enable`, or if `base..base+size` overlaps something already mapped.
pub fn map_device(base: usize, size: usize) {
    let device = El1Attributes::ATTRIBUTE_INDEX_0
        | El1Attributes::VALID
        | El1Attributes::ACCESSED
        | El1Attributes::UXN
        | El1Attributes::PXN;
    IDMAP
        .lock()
        .as_mut()
        .expect("mmu::map_device called before mmu::enable")
        .map_range(&MemoryRegion::new(base, base + size), device)
        .expect("mmu::map_device: overlapping or invalid range");
}
