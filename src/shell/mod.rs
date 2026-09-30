//! A minimal, kernel-side line-reading REPL: type a name, press Enter, run it. A stopgap for
//! `plans/USERSPACE_PLAN.md`'s step 6 -- no arguments, no pipelines, no builtins -- living at EL1
//! (`crate::shell`, the same place the lab's own, later, real shell lives) specifically so it
//! needs neither `userlib` nor argv/envp to exist first. Gets replaced outright by step 9's
//! PEG-grammar shell, not grown into one in place.

use crate::console::{self, DEFAULT_BG as BG, DEFAULT_FG as FG};
use crate::drivers::virtio::{gpu, input};
use crate::exec;
use crate::fs;
use crate::keyboard;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use arm_pl011_uart::Uart;
use core::fmt::Write;
use hadris_fat::FatVolumeReadExt;

/// Prints `msg` to both UART and the console -- so a typo'd command is visible on screen, not
/// only in a UART log the user may not have open (matches `r10_repl`'s own `report`). Fetches the
/// console fresh via `console::get()` rather than taking one as a parameter, and only for the
/// instant it's needed: `run` (this function's only caller) calls this again right after
/// `run_program`, whose EL0 program may itself have called `console::get()` via `syscall::fd`'s
/// `write` -- holding a `&mut Console` across that call would alias with it.
fn report(uart: &mut Uart, msg: &str) {
    writeln!(uart, "{msg}").ok();
    if let Some(console) = console::get() {
        console.puts(msg, FG, BG);
        console.putc('\n', FG, BG);
    }
}

/// Looks up `usr/bin/<argv[0]>` on `system.img` (blk0) and runs it at EL0 to completion, passing
/// `line` split on whitespace as its `argv` (no quoting/escaping -- that's the real shell's job,
/// step 9) and an empty environment (nothing produces real environment variables yet). Any
/// failure -- not mounted, not found, bad ELF, argv too large -- is reported and swallowed; one bad
/// command shouldn't take the REPL down with it.
///
/// # Panics
/// If `line` is empty or all whitespace -- callers must check `!line.trim().is_empty()` first.
fn run(uart: &mut Uart, line: &str) {
    let argv: Vec<&str> = line.split_whitespace().collect();
    let name = argv[0];

    let Some(vol) = fs::volume::get(0) else {
        report(uart, "system.img not mounted");
        return;
    };
    let Ok(entry) = fs::path::resolve(vol, &format!("usr/bin/{name}")) else {
        report(uart, &format!("{name}: not found"));
        return;
    };

    let Ok(mut reader) = vol.read_file(&entry) else {
        report(uart, &format!("{name}: failed to open"));
        return;
    };
    let Ok(program) = reader.read_to_vec() else {
        report(uart, &format!("{name}: failed to read"));
        return;
    };

    if let Err(e) = exec::process::run_program(&program, &argv, &[]) {
        report(uart, &format!("{name}: {e:?}"));
    }
}

/// Reads one line at a time from the keyboard queue, echoing to the console, and runs whatever
/// name was typed against `usr/bin/` on `system.img`. Never returns -- replaces both the
/// boot-time hardcoded `hello`/`scratch` run and the dumb 15-second typing test that came before
/// it; see `plans/PORTING.md`.
///
/// Requires a console, GPU and keyboard to all be present -- without a display there's nothing
/// to run an interactive shell on, so this idles forever instead (`main.rs`'s own prior
/// behavior for the "no display" case).
pub fn run_forever(uart: &mut Uart) -> ! {
    if console::get().is_none() || gpu::get().is_none() || input::get().is_none() {
        writeln!(uart, "No virtio-gpu or virtio-keyboard device found").ok();
        loop {
            unsafe { core::arch::asm!("wfe") }
        }
    }

    let mut line = String::new();
    loop {
        // Scoped so `console`/`gpu_dev` are dropped before `run` below: it calls `run_program`,
        // whose EL0 program may itself reach `console::get()` (via `syscall::fd`'s `write`),
        // which would otherwise alias a `&mut Console` still held here across that call.
        {
            let console = console::get().expect("checked above");
            let gpu_dev = gpu::get().expect("checked above");
            console.puts("virtos> ", FG, BG);
            gpu_dev.flush();
            loop {
                let Some(ch) = keyboard::queue::pop() else {
                    continue;
                };
                match ch {
                    '\n' => break,
                    '\x08' => {
                        if line.pop().is_some() {
                            console.putc(ch, FG, BG);
                            gpu_dev.flush();
                        }
                    }
                    _ => {
                        line.push(ch);
                        console.putc(ch, FG, BG);
                        gpu_dev.flush();
                    }
                }
            }
            console.putc('\n', FG, BG);
            gpu_dev.flush();
        }

        let trimmed = line.trim();
        if !trimmed.is_empty() {
            run(uart, trimmed);
        }
        line.clear();
    }
}
