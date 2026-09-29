//! The free-running system counter (`CNTPCT_EL0`), for bounded polling loops.
//!
//! The `CNTP_*` compare-timer (arm/ack an actual interrupt at some future tick) this was originally
//! ported from `r03_timer` alongside was never wired up to anything real -- nothing in this project
//! ever calls `arm`, so that PPI can never fire, and every device so far is either polled (`now`)
//! or driven by its own interrupt (`keyboard`). Removed rather than kept as unused "for later"
//! infrastructure: unlike `mmu::user_access` (added in step 3, consumed in step 4 right after),
//! nothing on the roadmap needs a periodic tick -- this project has no scheduler, ever (see
//! `plans/USERSPACE_PLAN.md`), which is the one thing a compare-timer interrupt is usually for.

use arm_sysregs::el0::accessors::{read_cntfrq_el0, read_cntpct_el0};

/// The system counter's frequency in Hz, fixed by the platform at boot.
pub fn freq() -> u64 {
    read_cntfrq_el0().clockfreq() as u64
}

/// The system counter's current value, in the same units `freq()` counts per second. Useful for a
/// plain "has this many seconds passed yet" wait with no interrupt involved.
pub fn now() -> u64 {
    read_cntpct_el0().physicalcount()
}
