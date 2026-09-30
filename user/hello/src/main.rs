//! The first real EL0 program: writes a greeting and exits. Nothing more -- see
//! `plans/USERSPACE_PLAN.md`'s user program inventory (`hello`, `crash`: "nothing" needed beyond
//! `read`/`write`/`exit`). Kernel-side features get their own throwaway test in `user/scratch`
//! instead of being bolted onto this one -- `hello`'s behavior should stay exactly this stable.
#![no_std]
#![no_main]

userlib::entry!(run);

fn run() {
    userlib::write(1, b"hello from EL0!\n");
}
