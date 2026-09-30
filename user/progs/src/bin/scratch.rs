//! Not a real program -- a throwaway EL0 test harness, rewritten freely to exercise whatever
//! kernel-side feature is being tested next, so `hello.rs`/`echo.rs` and their siblings never have
//! to be bent out of shape to prove something new works. Its behavior carries no stability
//! contract; check git history (or `git blame`) for what it last tested if that matters.
//!
//! Currently exercises: a non-blocking `read` (proving the syscall doesn't fault even though
//! nothing produces input yet), then a `write`, then `exit(0)`.
#![no_std]
#![no_main]

userlib::entry!(run);

fn run() {
    let mut buf = [0u8; 8];
    // Non-blocking (`syscall::fd::read`'s `Keyboard` case) and the queue is empty (nothing has
    // typed anything), so this returns 0 immediately without touching `buf`.
    userlib::read(0, &mut buf);
    userlib::write(1, b"scratch: read+write+exit all worked\n");
}
