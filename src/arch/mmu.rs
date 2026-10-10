//! Builds the page table and turns the MMU on: one identity-mapped table under `TTBR0_EL1`,
//! covering only the kernel's own image (split by `link.ld`'s boundary symbols into RX/RO/RW+XN
//! regions, the stack set apart from `.bss` by an unmapped guard) -- with `TTBR1_EL1` left
//! unused. `enable` maps nothing else: it doesn't take a single device address, and knows
//! nothing about the GIC, UART, RTC or anything else. Every device's MMIO window is mapped by
//! `map_device` instead, called by that device's own bring-up code right before it's first
//! touched (see `init::init_devices`) -- so this module never needs updating just because a new
//! device was added. This project runs at most one CPU with a single address space, so there's
//! only one ASID and one page table, ever.

use core::sync::atomic::{AtomicBool, Ordering};

use aarch64_paging::{
    MapError,
    descriptor::{El1Attributes, PhysicalAddress},
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

// Fixed EL0-accessible user window -- deliberately far from the kernel's own image (64 MiB into
// RAM, matching the reference lab's own choice for this exact QEMU `virt` machine): an off-by-one
// in this module's own boundary math lands in genuinely unmapped memory and faults immediately,
// rather than landing ambiguously on an adjacent mapped region with the wrong permissions. Left
// unmapped by `enable` itself -- it's the ELF loader (`exec::elf`) that owns every mapping inside
// it, one program load at a time, using each segment's own `p_flags`.
//
// Sized to fit comfortably under QEMU's default 128 MiB RAM (confirmed via `info mtree`) for this
// exact `virt` machine, same reasoning as the reference lab's own choice: the kernel image and
// heap sit below `USER_BASE`, the window ends at `0x4600_0000`, and the top 32 MiB stays free.
pub const USER_BASE: usize = 0x4400_0000;
pub const USER_SIZE: usize = 0x0200_0000; // 32 MiB

// The window's layout, from the bottom: the program image (its segments, wherever it links them,
// page-aligned), the heap (grown on demand by `brk`, starting at the page-aligned end of the
// image), an unmapped guard, then the stack up to the top of the window. Only the pages a
// program's segments/heap/stack actually need are ever mapped (`exec::elf`/`exec::process`) --
// the rest of the gap between the heap and the guard stays unmapped, so a stack overflow or a wild pointer into either faults instead of silently
// running into the program's own data.
/// The stack's size: generous, per the project's own rule of thumb for MiB-scale stacks/buffers.
pub const USER_STACK_SIZE: usize = 0x0010_0000; // 1 MiB
/// The unmapped guard below the stack -- larger than any single frame a sane program has, so one
/// large frame can't step over it.
pub const USER_GUARD_SIZE: usize = 0x0001_0000; // 64 KiB
/// One past the highest stack address: the initial stack pointer's starting point.
pub const USER_STACK_TOP: usize = USER_BASE + USER_SIZE;
/// The lowest stack address; the guard lies just below it.
pub const USER_STACK_BOTTOM: usize = USER_STACK_TOP - USER_STACK_SIZE;
/// The highest address (exclusive) a program's own segments may occupy.
pub const USER_IMAGE_END: usize = USER_STACK_BOTTOM - USER_GUARD_SIZE;

/// The page table `enable` builds and activates, and every later `map_device` call extends.
/// Locked rather than a bare `static mut`: `map_device` only ever needs the table for one
/// `map_range` call at a time and never holds it across anything else, so a lock costs nothing
/// here and comes with no deadlock risk -- unlike `gic.rs`'s `GicV2`, nothing touches this from
/// an interrupt handler.
static IDMAP: Mutex<Option<IdMap<El1And0>>> = Mutex::new(None);

/// Whether PAN is on (the CPU has FEAT_PAN and `enable` turned it on). `user_access` only touches
/// the `PAN` register if so, since writing it is undefined on a CPU without the feature.
static PAN_ENABLED: AtomicBool = AtomicBool::new(false);

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
        PAN_ENABLED.store(true, Ordering::Relaxed);
    }

    // Keeping IDMAP alive from here on is not optional: `IdMap`'s `Drop` impl panics if an
    // active mapping is ever dropped, since that would free memory the CPU is still using as
    // its page table.
    *IDMAP.lock() = Some(idmap);
    Hardening { pan }
}

/// Maps or unmaps `start..end` within the user window -- `Some(attrs)` maps every page in range
/// with `attrs`, `None` unmaps it. Used by the ELF loader (`exec::elf`), which needs finer control
/// than `map_device`'s "always unmapped -> mapped, exactly once" contract: it re-maps a segment
/// with fresh attributes as it locks down final permissions, and clears out whatever a previous
/// program left mapped before loading a new one -- both are mapped-to-mapped or
/// mapped-to-unmapped transitions `map_range` itself refuses (see `enable`/`map_device`'s own doc
/// comments on that break-before-make restriction). `IdMap::modify_range` handles both directions,
/// and a still-unmapped range too -- the case this always is on a range's very first use.
///
/// # Panics
/// If called before `enable`.
pub fn set_user_pages(
    start: usize,
    end: usize,
    attrs: Option<El1Attributes>,
) -> Result<(), MapError> {
    let region = MemoryRegion::new(start, end);
    IDMAP
        .lock()
        .as_mut()
        .expect("mmu::set_user_pages called before mmu::enable")
        .modify_range(&region, &|chunk, descriptor| {
            let flags = match attrs {
                // A level-3 entry is a page descriptor, which must be marked as such.
                Some(a) if descriptor.level() == 3 => a | El1Attributes::TABLE_OR_PAGE,
                Some(a) => a,
                None => El1Attributes::empty(),
            };
            descriptor.set(PhysicalAddress(chunk.start().0), flags)
        })
}

/// Lets the kernel touch user memory for as long as the returned guard lives, by clearing PAN.
/// Held only around the few places that mean to -- the ELF loader, argv/envp setup, and any
/// syscall that takes a user pointer (none of these exist yet; this is the guard they'll use, per
/// `plans/USERSPACE_PLAN.md`'s step 3/4 split). Does nothing if the CPU has no PAN.
#[must_use = "PAN is only cleared while the guard is alive"]
pub fn user_access() -> UserAccess {
    if PAN_ENABLED.load(Ordering::Relaxed) {
        // SAFETY: only reached when the CPU has FEAT_PAN (`PAN_ENABLED` is only ever set then).
        unsafe { write_pan(Pan::empty()) };
    }
    UserAccess
}

/// Dropping this restores PAN to on, if `user_access` turned it off. Holding it across an
/// exception (a nested syscall, or a fault) doesn't leave PAN off longer than intended either:
/// `enable`'s `SPAN = 0` makes every exception entry set PAN again regardless of this guard.
pub struct UserAccess;

impl Drop for UserAccess {
    fn drop(&mut self) {
        if PAN_ENABLED.load(Ordering::Relaxed) {
            // SAFETY: as in `user_access`.
            unsafe { write_pan(Pan::PAN) };
        }
    }
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
