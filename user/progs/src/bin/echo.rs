//! Prints its own arguments (not its own name -- `argv[0]` is skipped, as usual), separated by a
//! single space each, followed by a trailing newline. The first real program to consume `argv` for
//! something genuinely useful, not just to prove the mechanism works (`scratch.rs`'s job) -- also
//! the first to use the heap `userlib` provides, building the line as an ordinary `String` rather
//! than writing each argument as its own syscall.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use userlib::Args;

userlib::entry_with_args!(run);

fn run(args: Args) {
    let mut line = String::new();
    for (i, arg) in args.skip(1).enumerate() {
        if i > 0 {
            line.push(' ');
        }
        line.push_str(arg);
    }
    line.push('\n');
    userlib::write(1, line.as_bytes());
}
