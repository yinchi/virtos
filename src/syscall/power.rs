//! The `reboot` syscall: powers off or restarts the machine via `arch::psci`.

use abi::errno::EINVAL;
use abi::{LINUX_REBOOT_CMD_POWER_OFF, LINUX_REBOOT_CMD_RESTART};

use crate::arch::psci;

/// Powers off (`LINUX_REBOOT_CMD_POWER_OFF`) or restarts (`LINUX_REBOOT_CMD_RESTART`) the machine;
/// never returns on success. Any other `cmd` is `EINVAL`. `cmd` is the full register: a value that doesn't fit in `u32` is also unrecognized, not
/// silently truncated to a command it happens to end in.
pub fn reboot(cmd: usize) -> isize {
    match u32::try_from(cmd) {
        Ok(LINUX_REBOOT_CMD_POWER_OFF) => psci::system_off(),
        Ok(LINUX_REBOOT_CMD_RESTART) => psci::system_reset(),
        _ => EINVAL,
    }
}
