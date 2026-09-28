//! Pure sector-alignment arithmetic for `blkio`'s byte-stream adapter: turning a byte offset and a
//! requested length into a device sector index, an offset within that sector, and how many bytes a
//! single sector-bounced transfer can actually move.
//!
//! Pure, no hardware dependency, so it's tested on the host (`just test-host`).

/// Bytes per device sector -- fixed by the virtio-blk spec (and everything else in this project).
pub const SECTOR_SIZE: usize = 512;

/// One sector-bounced transfer: which sector to read/write, where in that sector's `SECTOR_SIZE`
/// bytes it starts, and how many bytes this single transfer covers.
pub struct Transfer {
    pub sector: usize,
    pub offset: usize,
    pub len: usize,
}

/// Plans one sector-bounced transfer of up to `want` bytes starting at byte offset `position`:
/// never more than fits in the rest of the sector `position` falls in, and never more than `want`.
/// `len` is always at least 1 when `want` is at least 1, since `offset` is always below
/// `SECTOR_SIZE`. The caller loops (via `embedded_io::Read`/`Write`'s own `read_exact`/`write_all`
/// retry logic) for a transfer longer than one sector; `want == 0` is the caller's responsibility
/// to short-circuit before calling this (embedded-io's own contract for an empty buffer).
pub fn plan(position: u64, want: usize) -> Transfer {
    let sector = (position / SECTOR_SIZE as u64) as usize;
    let offset = (position % SECTOR_SIZE as u64) as usize;
    let len = want.min(SECTOR_SIZE - offset);
    Transfer {
        sector,
        offset,
        len,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_plan(position: u64, want: usize, expect: (usize, usize, usize)) {
        let t = plan(position, want);
        assert_eq!(
            (t.sector, t.offset, t.len),
            expect,
            "plan({position}, {want})"
        );
    }

    #[test]
    fn a_request_within_one_sector_is_not_split() {
        assert_plan(0, 100, (0, 0, 100));
        assert_plan(10, 50, (0, 10, 50));
    }

    #[test]
    fn a_request_longer_than_the_rest_of_the_sector_is_capped_to_it() {
        assert_plan(0, 1000, (0, 0, SECTOR_SIZE));
        assert_plan(500, 50, (0, 500, SECTOR_SIZE - 500));
    }

    #[test]
    fn position_selects_the_right_sector_and_intra_sector_offset() {
        assert_plan(SECTOR_SIZE as u64, 100, (1, 0, 100));
        assert_plan(SECTOR_SIZE as u64 + 1, 1, (1, 1, 1));
        assert_plan(2 * SECTOR_SIZE as u64, 2000, (2, 0, SECTOR_SIZE));
    }

    #[test]
    fn len_is_never_zero_for_a_nonzero_want() {
        // The one edge that would make `len == 0` a real bug: sitting right at the last byte of
        // a sector must still report at least 1 byte available in *this* sector.
        assert_plan(SECTOR_SIZE as u64 - 1, 10, (0, SECTOR_SIZE - 1, 1));
    }
}
