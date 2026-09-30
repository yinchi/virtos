//! Clears the console. Takes no arguments.
#![no_std]
#![no_main]

userlib::entry!(run);

fn run() {
    userlib::clear();
}
