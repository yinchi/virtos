//! Maps a parsed ELF's segments into the fixed user window and copies their content in --
//! `exec::elfparse` does the pure validation; this touches the page table and copies bytes.
//!
//! Segments are locked down to their *final* permissions only after every byte is written: every
//! page starts out kernel-writable regardless of the segment's own permissions (a read-only
//! `.rodata` segment still needs the kernel to write its initial content), then gets re-mapped to
//! `p_flags`' real permissions once copying is done. Mapping with the final (possibly read-only)
//! attributes up front and writing through them anyway would be the wrong order -- the page
//! wouldn't be kernel-writable at all yet.

use aarch64_paging::descriptor::El1Attributes;

use super::elfparse::{self, ElfError, PAGE_SIZE, PF_W, PF_X, Segment};
use crate::arch::mmu::{self, USER_BASE, USER_IMAGE_END};

/// Rounds `vaddr..vaddr+memsz` out to whole pages, since permissions are per page (`elfparse`
/// already refused any two segments that would need to share one).
fn pages(seg: &Segment) -> (usize, usize) {
    let start = seg.vaddr & !(PAGE_SIZE - 1);
    let end = (seg.vaddr + seg.memsz + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    (start, end)
}

/// Attributes for a segment's pages while the kernel is still writing to them: always writable
/// and never executable, regardless of the segment's own final permissions -- `final_attributes`
/// (below) locks those down once copying is done. Also what the stack is mapped with (`process`)
/// -- it never gets a "final", more restrictive pass the way a segment does.
pub(super) fn data_attributes() -> El1Attributes {
    El1Attributes::ATTRIBUTE_INDEX_1
        | El1Attributes::INNER_SHAREABLE
        | El1Attributes::VALID
        | El1Attributes::ACCESSED
        | El1Attributes::USER
        | El1Attributes::PXN
        | El1Attributes::UXN
}

/// A segment's real, final permissions, straight from the ELF's own `p_flags`. Every segment this
/// loader ever sees has `PF_R` set (nothing in this project's toolchain emits an unreadable
/// segment), so there is no separate read-permission bit to derive here.
fn final_attributes(flags: u32) -> El1Attributes {
    let mut attrs = El1Attributes::ATTRIBUTE_INDEX_1
        | El1Attributes::INNER_SHAREABLE
        | El1Attributes::VALID
        | El1Attributes::ACCESSED
        | El1Attributes::USER
        | El1Attributes::PXN; // kernel must never execute user code
    if flags & PF_W == 0 {
        attrs |= El1Attributes::READ_ONLY;
    }
    if flags & PF_X == 0 {
        attrs |= El1Attributes::UXN;
    }
    attrs
}

/// Makes instructions just written to `start..end` visible to instruction fetch: the CPU's data
/// and instruction caches are separate, and the kernel wrote the code through the former.
fn sync_instruction_cache(start: usize, end: usize) {
    // CTR_EL0.DminLine (bits 19:16): log2 of the smallest data cache line, in 4-byte words.
    let ctr: usize;
    // SAFETY: reads an ID register.
    unsafe { core::arch::asm!("mrs {c}, ctr_el0", c = out(reg) ctr) };
    let line = 4usize << ((ctr >> 16) & 0xf);
    let mut addr = start & !(line - 1);
    while addr < end {
        // SAFETY: cleans one cache line of memory this function's caller just wrote.
        unsafe { core::arch::asm!("dc cvau, {a}", a = in(reg) addr) };
        addr += line;
    }
    // SAFETY: barrier and instruction-cache invalidate, no memory effects of their own.
    unsafe {
        core::arch::asm!("dsb ish", "ic ialluis", "dsb ish", "isb");
    }
}

/// Parses `elf_bytes` and maps its segments into the user window's image area
/// (`USER_BASE..USER_IMAGE_END`; the stack is separate, see `process::enter`). Returns the ELF's
/// entry point, ready to become `ELR_EL1` for the `eret` into EL0.
///
/// Reused as-is for every program load, not just the first: it begins by unmapping whatever the
/// previous program had, so nothing carries over.
///
/// # Errors
/// Returns `Err` if the ELF is invalid or a mapping fails. Nothing is copied or mapped if the file
/// itself is bad; a `MapFailed` partway through is fine too, since the next load starts by
/// unmapping the whole image area again regardless of what state it was left in.
pub fn load(elf_bytes: &[u8]) -> Result<usize, ElfError> {
    let parsed = elfparse::parse(elf_bytes, USER_BASE, USER_IMAGE_END)?;

    // The kernel writes user pages below (zeroing, copying), which PAN would otherwise forbid.
    let _user = mmu::user_access();

    // 1. Nothing of the previous program stays mapped.
    mmu::set_user_pages(USER_BASE, USER_IMAGE_END, None).map_err(|_| ElfError::MapFailed)?;

    // 2. Each segment's pages, writable while they are filled: zeroed whole (a page is never left
    //    holding the previous program's bytes), then the file's bytes copied in.
    for seg in &parsed.segments {
        let (start, end) = pages(seg);
        mmu::set_user_pages(start, end, Some(data_attributes()))
            .map_err(|_| ElfError::MapFailed)?;
        let src = &elf_bytes[seg.offset..seg.offset + seg.filesz];
        // SAFETY: `parse` checked the segment lies inside the image window and the source range
        // inside `elf_bytes`; the pages were just mapped writable and nothing else uses the
        // window while a program is being loaded.
        unsafe {
            core::ptr::write_bytes(start as *mut u8, 0, end - start);
            core::ptr::copy_nonoverlapping(src.as_ptr(), seg.vaddr as *mut u8, seg.filesz);
        }
    }

    // 3. Lock each segment down to its own permissions, and make code visible to instruction fetch.
    for seg in &parsed.segments {
        let (start, end) = pages(seg);
        mmu::set_user_pages(start, end, Some(final_attributes(seg.flags)))
            .map_err(|_| ElfError::MapFailed)?;
        if seg.flags & PF_X != 0 {
            sync_instruction_cache(seg.vaddr, seg.vaddr + seg.memsz);
        }
    }

    // A broad TLB invalidate (every EL1&0 entry, not just this window's pages) rather than a
    // precise per-page one: this runs once per program load, not on any hot path, so simplicity --
    // guaranteed correct regardless of exactly which pages changed -- outweighs the cost of
    // over-invalidating a few entries that didn't need it.
    unsafe {
        core::arch::asm!("dsb ishst", "tlbi vmalle1is", "dsb ish", "isb");
    }

    Ok(parsed.entry)
}
