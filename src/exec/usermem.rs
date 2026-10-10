//! Which parts of the user window are mapped, and whether they are writable -- the truth a syscall must
//! check before it touches a user pointer.
//!
//! With the MMU on, the kernel faults too if it reads or writes a page that is unmapped (the guard below
//! the stack, the gap after the program image) or writes a read-only one (the program's own code). A
//! user pointer into such a page must therefore be *refused* (`EFAULT`) before the kernel dereferences
//! it, not discovered by a data abort in the kernel. The loader (`elf.rs`) and `brk` record what they mapped here.
//!
//! Pure `no_std` + `alloc`, with no dependency on the rest of the kernel, so it is tested on the host
//! (`hosttests/`).

use alloc::vec::Vec;

/// One contiguous run of mapped user memory: `start..end` (page-aligned), and whether it is writable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub start: usize,
    pub end: usize,
    pub writable: bool,
}

/// The mapped regions of the running program, kept sorted and merged (adjacent regions with the same
/// permission become one, so a buffer spanning two neighbouring segments is still accepted).
#[derive(Debug, Default)]
pub struct UserMemory {
    regions: Vec<Region>,
}

impl UserMemory {
    pub const fn new() -> Self {
        Self {
            regions: Vec::new(),
        }
    }

    /// Forgets everything (the window is being unmapped).
    pub fn clear(&mut self) {
        self.regions.clear();
    }

    /// Records `start..end` as mapped. Regions may be added in any order.
    pub fn add(&mut self, start: usize, end: usize, writable: bool) {
        if start >= end {
            return;
        }
        self.regions.push(Region {
            start,
            end,
            writable,
        });
        self.regions.sort_by_key(|r| r.start);
        let mut merged: Vec<Region> = Vec::with_capacity(self.regions.len());
        for r in self.regions.drain(..) {
            match merged.last_mut() {
                Some(last) if last.end == r.start && last.writable == r.writable => {
                    last.end = r.end
                }
                _ => merged.push(r),
            }
        }
        self.regions = merged;
    }

    /// Forgets `start..end`: it is unmapped again. A region that straddles it is cut in two, or trimmed. (The heap
    /// gives memory back when the program break moves down.)
    pub fn remove(&mut self, start: usize, end: usize) {
        if start >= end {
            return;
        }
        let mut kept: Vec<Region> = Vec::with_capacity(self.regions.len() + 1);
        for r in self.regions.drain(..) {
            if r.end <= start || r.start >= end {
                kept.push(r); // untouched
                continue;
            }
            if r.start < start {
                kept.push(Region {
                    start: r.start,
                    end: start,
                    writable: r.writable,
                });
            }
            if r.end > end {
                kept.push(Region {
                    start: end,
                    end: r.end,
                    writable: r.writable,
                });
            }
        }
        self.regions = kept;
    }

    /// Whether every byte of `ptr..ptr+len` is mapped -- and writable, if `write`. An empty range is
    /// always fine (nothing is touched), wherever it points.
    pub fn allows(&self, ptr: usize, len: usize, write: bool) -> bool {
        if len == 0 {
            return true;
        }
        let Some(end) = ptr.checked_add(len) else {
            return false;
        };
        self.regions
            .iter()
            .any(|r| r.start <= ptr && end <= r.end && (r.writable || !write))
    }

    #[cfg(test)]
    pub fn regions(&self) -> &[Region] {
        &self.regions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: usize = 0x4400_0000;

    fn sample() -> UserMemory {
        let mut m = UserMemory::new();
        m.add(BASE, BASE + 0x2000, false); // code + rodata, read-only
        m.add(BASE + 0x2000, BASE + 0x4000, true); // data + bss
        m.add(BASE + 0x1F_0000, BASE + 0x20_0000, true); // stack
        m
    }

    #[test]
    fn reads_are_allowed_anywhere_mapped_and_writes_only_where_writable() {
        let m = sample();
        assert!(m.allows(BASE, 16, false));
        assert!(!m.allows(BASE, 16, true)); // code is read-only
        assert!(m.allows(BASE + 0x2000, 16, true));
        assert!(m.allows(BASE + 0x20_0000 - 8, 8, true)); // top of the stack
    }

    #[test]
    fn unmapped_pages_are_refused_including_the_gap_and_the_guard() {
        let m = sample();
        assert!(!m.allows(BASE + 0x4000, 1, false)); // just past the image
        assert!(!m.allows(BASE + 0x10_0000, 1, false)); // the gap
        assert!(!m.allows(BASE + 0x1E_FFFF, 1, false)); // just below the stack
        assert!(!m.allows(BASE - 1, 1, false)); // just below the window
        assert!(!m.allows(BASE + 0x20_0000, 1, false)); // just past it
    }

    #[test]
    fn a_range_may_not_leave_its_region() {
        let m = sample();
        assert!(!m.allows(BASE + 0x3FF0, 0x20, true)); // runs off the end of the data
        assert!(!m.allows(BASE + 0x1F_FFF0, 0x20, true)); // runs off the top of the stack
        assert!(!m.allows(BASE, usize::MAX, false)); // wraps
        assert!(!m.allows(usize::MAX, 2, false));
    }

    #[test]
    fn neighbouring_regions_with_the_same_permission_merge_but_different_ones_do_not() {
        let m = sample();
        // 0x2000 is the join of read-only code and writable data: a range across it is refused either way.
        assert!(!m.allows(BASE + 0x1FF0, 0x20, false));
        let mut m = UserMemory::new();
        m.add(BASE, BASE + 0x1000, true);
        m.add(BASE + 0x1000, BASE + 0x2000, true);
        assert_eq!(m.regions().len(), 1);
        assert!(m.allows(BASE + 0x0FF0, 0x20, true));
    }

    #[test]
    fn regions_may_be_added_in_any_order() {
        let mut m = UserMemory::new();
        m.add(BASE + 0x2000, BASE + 0x3000, true);
        m.add(BASE, BASE + 0x2000, true);
        assert_eq!(
            m.regions(),
            &[Region {
                start: BASE,
                end: BASE + 0x3000,
                writable: true
            }]
        );
    }

    #[test]
    fn removing_a_range_trims_splits_or_drops_regions() {
        let mut m = UserMemory::new();
        m.add(BASE, BASE + 0x4000, true);
        m.remove(BASE + 0x3000, BASE + 0x4000); // the top page goes: trimmed
        assert_eq!(
            m.regions(),
            &[Region {
                start: BASE,
                end: BASE + 0x3000,
                writable: true
            }]
        );
        assert!(!m.allows(BASE + 0x3000, 1, false));
        m.remove(BASE + 0x1000, BASE + 0x2000); // the middle: split in two
        assert_eq!(
            m.regions(),
            &[
                Region {
                    start: BASE,
                    end: BASE + 0x1000,
                    writable: true
                },
                Region {
                    start: BASE + 0x2000,
                    end: BASE + 0x3000,
                    writable: true
                },
            ]
        );
        m.remove(BASE, BASE + 0x1_0000); // everything
        assert!(m.regions().is_empty());
    }

    #[test]
    fn removing_leaves_neighbours_and_empty_ranges_alone() {
        let mut m = sample();
        m.remove(BASE + 0x8000, BASE + 0x9000); // maps nothing: no change
        m.remove(BASE + 0x3000, BASE + 0x3000); // empty
        assert_eq!(m.regions().len(), 3);
        m.remove(BASE + 0x2000, BASE + 0x4000); // exactly the data region
        assert!(m.allows(BASE, 16, false) && !m.allows(BASE + 0x2000, 1, false));
        assert!(m.allows(BASE + 0x20_0000 - 8, 8, true)); // the stack is unharmed
    }

    #[test]
    fn an_empty_range_is_always_fine_and_clear_forgets_everything() {
        let mut m = sample();
        assert!(m.allows(0, 0, true));
        assert!(m.allows(BASE + 0x10_0000, 0, true));
        m.clear();
        assert!(!m.allows(BASE, 1, false));
        m.add(BASE, BASE, true); // empty: ignored
        assert!(m.regions().is_empty());
    }
}
