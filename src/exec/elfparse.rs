//! The pure half of the ELF loader: no side-effects or kernel involvement.
//!
//! Validates a simple, statically-linked AArch64 executable and says which segments to load
//! where, without touching memory or the page table (that's `exec::elf`). Kept free of hardware
//! dependencies so it builds and runs under `cargo test --target <host triple>` (`just test-host`)
//! exactly as it does on target.
//!
//! Returns `Result`, never panics on a malformed file: nothing produces an untrusted ELF yet (no
//! shell, no `chmod +x` on an arbitrary file), but there's no reason to build the
//! `assert!`-your-way-through version first when a mature reference implementation already
//! settled on refusing cleanly instead.

use alloc::vec::Vec;

/// Size, in bytes, of `e_ident`.
const EI_NIDENT: usize = 16;

/// `e_ident[EI_CLASS]` value meaning "64-bit objects" (as opposed to `1` for 32-bit).
const ELFCLASS64: u8 = 2;

/// `e_ident[EI_DATA]` value meaning little-endian (as opposed to `2`, big-endian).
const ELFDATA2LSB: u8 = 1;

/// `e_type` value meaning a statically-linked executable.
const ET_EXEC: u16 = 2;

/// `e_machine` value identifying the target instruction set as AArch64.
const EM_AARCH64: u16 = 183;

/// Program header `p_type` value meaning "load this segment into memory at runtime".
const PT_LOAD: u32 = 1;

/// The page size the loader maps in. Permissions are per page, so segments may not share one.
pub const PAGE_SIZE: usize = 4096;

/// `p_flags` bit meaning the segment should be mapped executable.
pub const PF_X: u32 = 1;

/// `p_flags` bit meaning the segment should be mapped writable.
pub const PF_W: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64Header {
    /// ELF identification segment
    e_ident: [u8; EI_NIDENT],
    /// Object file type, must be ET_EXEC.
    e_type: u16,
    /// Target instruction set architecture, must be EM_AARCH64.
    e_machine: u16,
    /// Object file version -- unused.
    e_version: u32,
    /// Entry point virtual address.
    e_entry: u64,
    /// Program header table file offset.
    e_phoff: u64,
    /// Section header table file offset -- unused: only `PT_LOAD` program headers are ever walked
    /// (no symbol resolution or debugging happens here).
    e_shoff: u64,
    /// Processor-specific flags -- unused; always `0` for AArch64's standard ABI.
    e_flags: u32,
    /// Size of this header, in bytes -- unused; this struct's own `size_of` already gives it.
    e_ehsize: u16,
    /// Size of one program header table entry, in bytes.
    e_phentsize: u16,
    /// Number of entries in the program header table.
    e_phnum: u16,
    /// Section header table fields -- unused, see `e_shoff`.
    e_shentsize: u16,
    e_shnum: u16,
    e_shstrndx: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64ProgramHeader {
    /// Segment type -- only `PT_LOAD` segments are loaded, everything else is skipped.
    p_type: u32,
    /// Segment permission flags (`PF_X`/`PF_W`; `PF_R` is assumed set on every segment).
    p_flags: u32,
    /// Offset of the segment's data within the ELF file.
    p_offset: u64,
    /// Virtual address at which the segment should be mapped.
    p_vaddr: u64,
    /// Physical address -- unused; identity mapping means there's no physical/virtual distinction.
    p_paddr: u64,
    /// Size of the segment's data within the file.
    p_filesz: u64,
    /// Size the segment occupies in memory once loaded (>= `p_filesz`; the remainder is
    /// zero-filled `.bss`-like data the file doesn't store).
    p_memsz: u64,
    /// Required alignment -- unused; the loader doesn't depend on any particular alignment.
    p_align: u64,
}

/// Why a file was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfError {
    /// Shorter than an ELF header.
    TooSmall,
    /// Doesn't start with `\x7fELF`.
    NotElf,
    /// Not 64-bit.
    Not64Bit,
    /// Not little-endian.
    NotLittleEndian,
    /// Not `ET_EXEC` (a statically-linked executable).
    NotExecutable,
    /// Not AArch64.
    WrongMachine,
    /// The program header table doesn't fit in the file, or its entries are too small.
    BadProgramHeaders,
    /// No non-empty `PT_LOAD` segment at all.
    NoLoadSegment,
    /// A `PT_LOAD` segment reaches outside the user window (or wraps around the address space).
    SegmentOutsideWindow,
    /// A `PT_LOAD` segment's file bytes lie (partly) beyond the end of the file.
    SegmentTruncated,
    /// A `PT_LOAD` segment's `p_memsz` is smaller than its `p_filesz`.
    BadSegmentSize,
    /// Two `PT_LOAD` segments occupy (part of) the same page, so they can't each get their own
    /// permissions. This project's own linker scripts page-align every segment.
    SegmentsShareAPage,
    /// The entry point isn't inside any loaded segment.
    BadEntry,
    /// The page table refused a mapping (see `exec::elf`).
    MapFailed,
}

/// One `PT_LOAD` segment, already checked: its file bytes are `offset..offset+filesz` of the file
/// and it occupies `vaddr..vaddr+memsz` of the user window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub vaddr: usize,
    pub offset: usize,
    pub filesz: usize,
    pub memsz: usize,
    pub flags: u32,
}

/// A validated executable: where to start, and what to load.
#[derive(Debug, PartialEq, Eq)]
pub struct Parsed {
    pub entry: usize,
    pub segments: Vec<Segment>,
}

/// Reads a `T` out of `data` at byte offset `off`, unaligned (ELF fields aren't guaranteed to land
/// on a Rust-friendly alignment boundary), or `None` if it doesn't fit.
///
/// `T` must be one of this module's plain-integer `#[repr(C)]` structs, for which every bit
/// pattern is valid.
fn read_at<T: Copy>(data: &[u8], off: usize) -> Option<T> {
    let end = off.checked_add(core::mem::size_of::<T>())?;
    if end > data.len() {
        return None;
    }
    // SAFETY: bounds checked just above; `T` is a plain-integer struct (see above).
    Some(unsafe { (data.as_ptr().add(off) as *const T).read_unaligned() })
}

/// Validates `elf` as a simple, statically-linked AArch64 executable whose loadable segments fit
/// in `window_start..window_end`, and returns what to load. Touches nothing.
pub fn parse(elf: &[u8], window_start: usize, window_end: usize) -> Result<Parsed, ElfError> {
    let header: Elf64Header = read_at(elf, 0).ok_or(ElfError::TooSmall)?;

    if &header.e_ident[0..4] != b"\x7fELF" {
        return Err(ElfError::NotElf);
    }
    if header.e_ident[4] != ELFCLASS64 {
        return Err(ElfError::Not64Bit);
    }
    if header.e_ident[5] != ELFDATA2LSB {
        return Err(ElfError::NotLittleEndian);
    }
    if header.e_type != ET_EXEC {
        return Err(ElfError::NotExecutable);
    }
    if header.e_machine != EM_AARCH64 {
        return Err(ElfError::WrongMachine);
    }

    // The whole program header table must be inside the file, with entries at least as big as
    // the fields we read (a larger `e_phentsize` is legal and just means extra fields).
    let phentsize = header.e_phentsize as usize;
    if phentsize < core::mem::size_of::<Elf64ProgramHeader>() {
        return Err(ElfError::BadProgramHeaders);
    }
    let phoff = usize::try_from(header.e_phoff).map_err(|_| ElfError::BadProgramHeaders)?;
    let phtable_end = (header.e_phnum as usize)
        .checked_mul(phentsize)
        .and_then(|len| phoff.checked_add(len))
        .ok_or(ElfError::BadProgramHeaders)?;
    if phtable_end > elf.len() {
        return Err(ElfError::BadProgramHeaders);
    }

    let mut segments = Vec::new();
    for i in 0..header.e_phnum as usize {
        let phdr: Elf64ProgramHeader =
            read_at(elf, phoff + i * phentsize).ok_or(ElfError::BadProgramHeaders)?;
        if phdr.p_type != PT_LOAD {
            continue;
        }

        let to_usize = |v: u64, err| usize::try_from(v).map_err(|_| err);
        let vaddr = to_usize(phdr.p_vaddr, ElfError::SegmentOutsideWindow)?;
        let memsz = to_usize(phdr.p_memsz, ElfError::SegmentOutsideWindow)?;
        let filesz = to_usize(phdr.p_filesz, ElfError::SegmentTruncated)?;
        let offset = to_usize(phdr.p_offset, ElfError::SegmentTruncated)?;

        if memsz < filesz {
            return Err(ElfError::BadSegmentSize);
        }
        if memsz == 0 {
            continue; // nothing to load or map
        }
        let end = vaddr
            .checked_add(memsz)
            .ok_or(ElfError::SegmentOutsideWindow)?;
        if vaddr < window_start || end > window_end {
            return Err(ElfError::SegmentOutsideWindow);
        }
        if offset
            .checked_add(filesz)
            .is_none_or(|file_end| file_end > elf.len())
        {
            return Err(ElfError::SegmentTruncated);
        }
        segments.push(Segment {
            vaddr,
            offset,
            filesz,
            memsz,
            flags: phdr.p_flags,
        });
    }

    if segments.is_empty() {
        return Err(ElfError::NoLoadSegment);
    }
    // Permissions are per page (read-only, no-execute), so each segment needs pages of its own.
    let pages = |s: &Segment| {
        (
            s.vaddr & !(PAGE_SIZE - 1),
            (s.vaddr + s.memsz + PAGE_SIZE - 1) & !(PAGE_SIZE - 1),
        )
    };
    for (i, a) in segments.iter().enumerate() {
        for b in &segments[i + 1..] {
            let ((a_start, a_end), (b_start, b_end)) = (pages(a), pages(b));
            if a_start < b_end && b_start < a_end {
                return Err(ElfError::SegmentsShareAPage);
            }
        }
    }
    let entry = usize::try_from(header.e_entry).map_err(|_| ElfError::BadEntry)?;
    if !segments
        .iter()
        .any(|s| entry >= s.vaddr && entry < s.vaddr + s.memsz)
    {
        return Err(ElfError::BadEntry);
    }

    Ok(Parsed { entry, segments })
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: usize = 0x4400_0000;
    const END: usize = START + 0x20_0000;

    /// One program header, as the test builder takes it: (type, flags, offset, vaddr, filesz, memsz).
    type Ph = (u32, u32, u64, u64, u64, u64);

    /// A well-formed one-segment executable: 64-byte header, one 56-byte program header, then 16
    /// bytes of "code", loaded at `START` with the entry at its start.
    fn good() -> Vec<u8> {
        build(
            START as u64,
            64,
            &[(PT_LOAD, PF_X, 120, START as u64, 16, 16)],
            136,
        )
    }

    fn build(entry: u64, phoff: u64, phdrs: &[Ph], len: usize) -> Vec<u8> {
        let mut b = alloc::vec![0u8; len.max(64 + phdrs.len() * 56)];
        b[0..4].copy_from_slice(b"\x7fELF");
        b[4] = ELFCLASS64;
        b[5] = ELFDATA2LSB;
        b[16..18].copy_from_slice(&ET_EXEC.to_le_bytes());
        b[18..20].copy_from_slice(&EM_AARCH64.to_le_bytes());
        b[24..32].copy_from_slice(&entry.to_le_bytes());
        b[32..40].copy_from_slice(&phoff.to_le_bytes());
        b[54..56].copy_from_slice(&56u16.to_le_bytes()); // e_phentsize
        b[56..58].copy_from_slice(&(phdrs.len() as u16).to_le_bytes()); // e_phnum
        for (i, &(ty, flags, off, vaddr, filesz, memsz)) in phdrs.iter().enumerate() {
            let at = phoff as usize + i * 56;
            if at + 56 > b.len() {
                continue;
            }
            b[at..at + 4].copy_from_slice(&ty.to_le_bytes());
            b[at + 4..at + 8].copy_from_slice(&flags.to_le_bytes());
            b[at + 8..at + 16].copy_from_slice(&off.to_le_bytes());
            b[at + 16..at + 24].copy_from_slice(&vaddr.to_le_bytes());
            b[at + 32..at + 40].copy_from_slice(&filesz.to_le_bytes());
            b[at + 40..at + 48].copy_from_slice(&memsz.to_le_bytes());
        }
        b
    }

    fn parse_good(b: &[u8]) -> Result<Parsed, ElfError> {
        parse(b, START, END)
    }

    #[test]
    fn accepts_a_well_formed_executable() {
        let p = parse_good(&good()).unwrap();
        assert_eq!(p.entry, START);
        assert_eq!(
            p.segments,
            [Segment {
                vaddr: START,
                offset: 120,
                filesz: 16,
                memsz: 16,
                flags: PF_X
            }]
        );
    }

    #[test]
    fn accepts_bss_and_several_segments_and_skips_other_types() {
        let b = build(
            START as u64,
            64,
            &[
                (PT_LOAD, PF_X, 0, START as u64, 200, 200),
                (6 /* PT_PHDR */, 0, 0, 0, 0, 0),
                (PT_LOAD, PF_W, 200, (START + 0x1000) as u64, 8, 0x4000), // .bss tail
                (PT_LOAD, 0, 0, (START + 0x9000) as u64, 0, 0),           // empty: skipped
            ],
            300,
        );
        let p = parse_good(&b).unwrap();
        assert_eq!(p.segments.len(), 2);
        assert_eq!(p.segments[1].memsz, 0x4000);
    }

    #[test]
    fn refuses_segments_that_share_a_page() {
        // Code at the start of a page and data in the same page: one page can't be both.
        let b = build(
            START as u64,
            64,
            &[
                (PT_LOAD, PF_X, 0, START as u64, 200, 200),
                (PT_LOAD, PF_W, 200, (START + 200) as u64, 8, 64),
            ],
            300,
        );
        assert_eq!(parse_good(&b), Err(ElfError::SegmentsShareAPage));
        // The same two segments a page apart are fine, however small the first one is.
        let b = build(
            START as u64,
            64,
            &[
                (PT_LOAD, PF_X, 0, START as u64, 200, 200),
                (PT_LOAD, PF_W, 200, (START + PAGE_SIZE) as u64, 8, 64),
            ],
            300,
        );
        assert!(parse_good(&b).is_ok());
        // A segment ending exactly on a page boundary doesn't touch the next page.
        let b = build(
            START as u64,
            64,
            &[
                (PT_LOAD, PF_X, 0, START as u64, 200, PAGE_SIZE as u64),
                (PT_LOAD, PF_W, 200, (START + PAGE_SIZE) as u64, 8, 64),
            ],
            300,
        );
        assert!(parse_good(&b).is_ok());
    }

    #[test]
    fn refuses_files_that_are_not_elf() {
        assert_eq!(parse_good(b""), Err(ElfError::TooSmall));
        assert_eq!(parse_good(&good()[..63]), Err(ElfError::TooSmall));
        assert_eq!(parse_good(&[b'a'; 200]), Err(ElfError::NotElf));
        assert_eq!(parse_good(b"#!/bin/sh\necho hi\n"), Err(ElfError::TooSmall));
        let mut b = good();
        b[0] = 0;
        assert_eq!(parse_good(&b), Err(ElfError::NotElf));
    }

    #[test]
    fn refuses_the_wrong_kind_of_elf() {
        let mut b = good();
        b[4] = 1;
        assert_eq!(parse_good(&b), Err(ElfError::Not64Bit));
        let mut b = good();
        b[5] = 2;
        assert_eq!(parse_good(&b), Err(ElfError::NotLittleEndian));
        let mut b = good();
        b[16] = 3; // ET_DYN
        assert_eq!(parse_good(&b), Err(ElfError::NotExecutable));
        let mut b = good();
        b[18] = 62; // x86-64
        assert_eq!(parse_good(&b), Err(ElfError::WrongMachine));
    }

    #[test]
    fn refuses_a_truncated_file() {
        let b = good();
        // Cut anywhere: inside the header, inside the program header, inside the segment's bytes.
        for cut in 0..b.len() {
            assert!(parse_good(&b[..cut]).is_err(), "cut at {cut} was accepted");
        }
    }

    #[test]
    fn refuses_a_bad_program_header_table() {
        // e_phoff past the end of the file / near u64::MAX (must not wrap).
        for phoff in [136u64, 1 << 40, u64::MAX, u64::MAX - 20] {
            let mut b = good();
            b[32..40].copy_from_slice(&phoff.to_le_bytes());
            assert_eq!(
                parse_good(&b),
                Err(ElfError::BadProgramHeaders),
                "phoff {phoff:#x}"
            );
        }
        // Entries smaller than the fields we read.
        let mut b = good();
        b[54..56].copy_from_slice(&40u16.to_le_bytes());
        assert_eq!(parse_good(&b), Err(ElfError::BadProgramHeaders));
        // More entries than the file holds.
        let mut b = good();
        b[56..58].copy_from_slice(&500u16.to_le_bytes());
        assert_eq!(parse_good(&b), Err(ElfError::BadProgramHeaders));
    }

    #[test]
    fn refuses_segments_outside_the_window() {
        let cases: [(u64, u64, u64); 6] = [
            (0x5000_0000, 16, 16),              // above the window
            (0x4000_0000, 16, 16),              // below it (the kernel's own memory!)
            (0, 16, 16),                        // null
            (START as u64 + 0x1f_fff8, 16, 16), // straddles the end
            (START as u64, 16, 0x30_0000),      // bigger than the whole window
            (u64::MAX - 8, 16, 16),             // vaddr + memsz wraps
        ];
        for (vaddr, filesz, memsz) in cases {
            let b = build(
                START as u64,
                64,
                &[(PT_LOAD, PF_X, 120, vaddr, filesz, memsz)],
                136,
            );
            assert_eq!(
                parse_good(&b),
                Err(ElfError::SegmentOutsideWindow),
                "vaddr {vaddr:#x} memsz {memsz:#x}"
            );
        }
    }

    #[test]
    fn refuses_segment_file_ranges_beyond_the_file() {
        for (off, filesz) in [
            (120u64, 17u64),
            (137, 1),
            (u64::MAX, 1),
            (100, u64::MAX),
            (1 << 40, 16),
        ] {
            let b = build(
                START as u64,
                64,
                &[(PT_LOAD, PF_X, off, START as u64, filesz, filesz.max(16))],
                136,
            );
            let r = parse_good(&b);
            assert!(
                matches!(
                    r,
                    Err(ElfError::SegmentTruncated) | Err(ElfError::SegmentOutsideWindow)
                ),
                "off {off:#x} filesz {filesz:#x}: {r:?}"
            );
        }
    }

    #[test]
    fn refuses_memsz_smaller_than_filesz() {
        let b = build(
            START as u64,
            64,
            &[(PT_LOAD, PF_X, 120, START as u64, 16, 8)],
            136,
        );
        assert_eq!(parse_good(&b), Err(ElfError::BadSegmentSize));
    }

    #[test]
    fn refuses_no_load_segments_and_a_stray_entry_point() {
        let b = build(START as u64, 64, &[(6, 0, 0, 0, 0, 0)], 200);
        assert_eq!(parse_good(&b), Err(ElfError::NoLoadSegment));
        let b = build(START as u64, 64, &[], 200);
        assert_eq!(parse_good(&b), Err(ElfError::NoLoadSegment));
        for entry in [
            0u64,
            START as u64 - 4,
            START as u64 + 16,
            0x5000_0000,
            u64::MAX,
        ] {
            let b = build(
                entry,
                64,
                &[(PT_LOAD, PF_X, 120, START as u64, 16, 16)],
                136,
            );
            assert_eq!(parse_good(&b), Err(ElfError::BadEntry), "entry {entry:#x}");
        }
    }
}
