//! A program's own life: its entry stub (`start.s`) and the `entry!`/`entry_with_args!` macros,
//! decoding `argc`/`argv` (`Args`), and ending -- `exit`, exit statuses (`Termination`,
//! `ExitCode`), and the panic handler, which ends the program the same way.

use core::arch::asm;

use abi::{SYS_EXIT, SYS_REBOOT};

/// The `cmd` values `reboot` takes, re-exported so a program writes
/// `userlib::LINUX_REBOOT_CMD_POWER_OFF`.
pub use abi::{LINUX_REBOOT_CMD_POWER_OFF, LINUX_REBOOT_CMD_RESTART};

core::arch::global_asm!(include_str!("start.s"));

/// A user-program panic must not double-panic the kernel -- it terminates the program the same way
/// any other abnormal ending does, via `exit`. `101` matches a conventional exit code for a
/// panicking process (familiarity only, not a functional dependency on anything upstream).
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    exit(101)
}

/// Ends the calling program and returns control to the kernel. `no_mangle` so `start.s` can call
/// it directly by symbol name, in its fallback path if `main` somehow returns without calling this
/// itself (which shouldn't happen -- `main` is `-> !` -- but `start.s` falls back to `exit(255)`
/// rather than falling into whatever memory follows if it somehow does).
#[unsafe(no_mangle)]
pub extern "C" fn exit(code: i32) -> ! {
    syscall!(SYS_EXIT, code as usize);
    // `exit` never returns from the kernel's side either; loop defensively in case it somehow
    // does, rather than falling into whatever follows.
    loop {
        unsafe { asm!("wfe") }
    }
}

/// Powers off (`LINUX_REBOOT_CMD_POWER_OFF`) or restarts (`LINUX_REBOOT_CMD_RESTART`) the machine.
/// Never returns on success; returns a negated errno if `cmd` isn't recognized.
pub fn reboot(cmd: u32) -> isize {
    syscall!(SYS_REBOOT, cmd as usize)
}

/// Mirrors `std::process::Termination`, simplified to what a syscall's scalar return value can
/// actually carry: a program either finished with nothing to report (`()`, exit code 0) or a
/// specific exit code (`ExitCode`).
pub trait Termination {
    /// Converts the implementing type into an integer exit code.
    fn report(self) -> i32;
}

impl Termination for () {
    fn report(self) -> i32 {
        0
    }
}

/// A specific process exit code, in the same 0-255 range a real Unix exit status uses.
pub struct ExitCode(pub u8);

impl Termination for ExitCode {
    fn report(self) -> i32 {
        self.0 as i32
    }
}

/// Turns whatever a program's own entry function returned into the actual `exit` syscall. Called
/// by the `entry!`/`entry_with_args!`-generated `main`, not directly by program code.
pub fn terminate<T: Termination>(result: T) -> ! {
    exit(result.report())
}

/// Generates the `#[no_mangle] extern "C" fn main() -> !` that `start.s`'s `bl main` branches to,
/// wired to initialize the heap, call `$run`, and pass its result through `terminate`. `$run` may
/// return `()` (exit code 0) or `ExitCode` (a specific code) -- anything else implementing
/// `Termination`.
///
/// One invocation of this per binary crate -- `userlib::entry!(run);` -- replaces what would
/// otherwise be a hand-written `_start`/panic-handler/exit boilerplate duplicated across every EL0
/// program. For a program that wants its command-line arguments, use `entry_with_args!` instead.
#[macro_export]
macro_rules! entry {
    ($run:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn main() -> ! {
            $crate::init_heap();
            $crate::terminate($run())
        }
    };
}

/// One argument in an `Args` (`argv[0]` is the program's own name, same C convention) -- `argc`/
/// `argv` themselves, as `_start` receives them in `x0`/`x1`, aren't Rust-safe to hand a program
/// directly: this decodes them into an ordinary, `Copy`, single-pass iterator once, here, rather
/// than every consumer re-implementing C-string scanning and UTF-8 validation itself.
#[derive(Clone, Copy)]
pub struct Args {
    argv: *const *const u8,
    remaining: usize,
}

impl Iterator for Args {
    type Item = &'static str;

    fn next(&mut self) -> Option<&'static str> {
        if self.remaining == 0 {
            return None;
        }
        // SAFETY: the only way to construct an `Args` is through `args`, so its contract already
        // holds for every entry this reads.
        unsafe {
            let ptr = *self.argv;
            let mut len = 0;
            while *ptr.add(len) != 0 {
                len += 1;
            }
            let bytes = core::slice::from_raw_parts(ptr, len);
            self.argv = self.argv.add(1);
            self.remaining -= 1;
            // The kernel only ever writes argument strings from its own UTF-8 `str` (typed at a
            // real keyboard) -- invalid UTF-8 here would mean that contract was already broken,
            // not something to paper over by silently ending the iteration early.
            Some(core::str::from_utf8(bytes).expect("argv entry was not valid UTF-8"))
        }
    }
}

/// Decodes the raw `argc`/`argv` `_start` received into an `Args` iterator.
///
/// # Safety
/// `argv` must point to an array of exactly `argc` valid pointers, each to a NUL-terminated,
/// UTF-8 byte sequence, all still mapped for as long as the returned `Args` is used -- true for
/// whatever `_start` forwards directly from the kernel's own `eret`, never something a program
/// should construct itself.
pub unsafe fn args(argc: usize, argv: *const *const u8) -> Args {
    Args {
        argv,
        remaining: argc,
    }
}

/// Like `entry!`, but for a program that wants its command-line arguments: generates the
/// `#[no_mangle] extern "C" fn main(argc: usize, argv: *const *const u8) -> !` that `start.s`'s
/// `bl main` branches to (forwarding `x0`/`x1` unchanged), initializes the heap, decodes them via
/// `args`, and calls `$run` with the resulting `Args`.
#[macro_export]
macro_rules! entry_with_args {
    ($run:path) => {
        #[unsafe(no_mangle)]
        // Only `_start` (start.s) calls this, with the registers the kernel set up; it is not an
        // API that could be handed an arbitrary pointer.
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        pub extern "C" fn main(argc: usize, argv: *const *const u8) -> ! {
            $crate::init_heap();
            // SAFETY: `_start` (start.s) forwards these straight from the kernel's `eret`,
            // upholding `args`'s contract by construction.
            $crate::terminate($run(unsafe { $crate::args(argc, argv) }))
        }
    };
}

/// One `"NAME=VALUE"` entry in an `Env` (envp decoding), still needing its own `split_once('=')`
/// -- left to each caller, the same way `Args` hands back a raw `argv` entry rather than a
/// pre-parsed `(flag, value)` pair.
#[derive(Clone, Copy)]
pub struct Env {
    envp: *const *const u8,
}

impl Iterator for Env {
    type Item = &'static str;

    fn next(&mut self) -> Option<&'static str> {
        // SAFETY: the only way to construct an `Env` is through `env`, so its contract already
        // holds for every entry this reads, and a NULL entry (checked below) is how `envp`'s own
        // array says "no more" -- there's no separate count the way `argc` gives `Args`.
        unsafe {
            let entry = *self.envp;
            if entry.is_null() {
                return None;
            }
            let mut len = 0;
            while *entry.add(len) != 0 {
                len += 1;
            }
            let bytes = core::slice::from_raw_parts(entry, len);
            self.envp = self.envp.add(1);
            // Same contract `Args::next` rests on: the kernel only ever writes these from its
            // own UTF-8 `str`s.
            Some(core::str::from_utf8(bytes).expect("envp entry was not valid UTF-8"))
        }
    }
}

/// Decodes the raw `envp` `_start` received (via `entry_with_env!`) into an `Env` iterator.
///
/// # Safety
/// `envp` must be a NULL-terminated array of valid pointers, each to a NUL-terminated, UTF-8 byte
/// sequence, all still mapped for as long as the returned `Env` is used -- true for whatever
/// `_start` forwards directly from the kernel's own `eret`, never something a program should
/// construct itself.
pub unsafe fn env(envp: *const *const u8) -> Env {
    Env { envp }
}

/// Like `entry_with_args!`, but for a program that also wants its environment: generates
/// `#[no_mangle] extern "C" fn main(argc: usize, argv: *const *const u8, envp: *const *const u8)
/// -> !` (forwarding `x0`-`x2` unchanged -- `_start`'s bare `bl main` never touches `x2`, so
/// whatever the kernel's `eret` placed there is simply still there), and calls `$run` with both
/// an `Args` and an `Env`.
#[macro_export]
macro_rules! entry_with_env {
    ($run:path) => {
        #[unsafe(no_mangle)]
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        pub extern "C" fn main(argc: usize, argv: *const *const u8, envp: *const *const u8) -> ! {
            $crate::init_heap();
            // SAFETY: as in `entry_with_args!`, both `args` and `env`'s contracts hold by
            // construction for whatever `_start` forwards straight from the kernel's `eret`.
            $crate::terminate($run(unsafe { $crate::args(argc, argv) }, unsafe {
                $crate::env(envp)
            }))
        }
    };
}
