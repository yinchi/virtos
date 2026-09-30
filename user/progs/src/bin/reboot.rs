//! Restarts the machine. Takes no arguments.
#![no_std]
#![no_main]

userlib::entry!(run);

fn run() {
    userlib::reboot(userlib::LINUX_REBOOT_CMD_RESTART);
}
