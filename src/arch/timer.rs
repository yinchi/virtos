//! EL1 physical timer (`CNTP_*`).

use core::sync::atomic::{AtomicU64, Ordering};

use arm_sysregs::el0::accessors::{read_cntfrq_el0, write_cntp_ctl_el0, write_cntp_tval_el0};
use arm_sysregs::el0::registers::{CntpCtlEl0, CntpTvalEl0};

/// PPI number for the non-secure EL1 physical timer interrupt (confirmed via the DTB's `timer`
/// node: PPI 14 -> GIC interrupt ID 16+14 = 30).
pub const PPI: u32 = 14;

/// How many times the timer has fired, incremented by `ack`.
static TICKS: AtomicU64 = AtomicU64::new(0);

/// The system counter's frequency in Hz, fixed by the platform at boot.
pub fn freq() -> u64 {
    read_cntfrq_el0().clockfreq() as u64
}

/// Arms (or re-arms) the timer to fire `ticks` counter cycles from now.
pub fn arm(ticks: u64) {
    write_cntp_tval_el0(CntpTvalEl0::empty().with_timervalue(ticks as i32));
    write_cntp_ctl_el0(CntpCtlEl0::ENABLE);
}

/// How many times the timer has fired so far.
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Called from `gic::irq_handler` when the timer's PPI fires: re-arms it for another `ticks`
/// cycles and counts the tick.
pub fn ack(ticks: u64) {
    arm(ticks);
    TICKS.fetch_add(1, Ordering::Relaxed);
}
