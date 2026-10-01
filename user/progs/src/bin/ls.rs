//! `ls [dir...]` -- lists the names in each named directory, one per line, in the order `getdents`
//! returns them (not sorted). No `dir` operand lists the working directory (an empty path, which
//! `open` resolves against it, same as every other relative path).
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use userlib::{Args, DIRENT_SIZE, DirEnt, ExitCode, O_RDONLY};

userlib::entry_with_args!(run);

const USAGE: &str = "ls [dir...]";
const FLAGS: &[(&str, &str)] = &[];

/// Buffer size for one `getdents` call -- fetches several records per syscall rather than one, the
/// way the mature reference's own `ls.rs` does (its `BATCH = 8`), but sized to one memory page (4
/// KiB), a conventional buffer size, rather than an arbitrary record count with no stated reason:
/// 15 records per call at `DIRENT_SIZE`'s current 261 bytes.
const GETDENTS_BUF_SIZE: usize = 4096;

/// Lists one directory's entries to stdout. `false` if `dir` can't be opened, or opens but isn't
/// actually a directory (already reported to stderr either way) -- the first case uses the same
/// "cannot open" wording every other program here already uses, not GNU's own distinct "cannot
/// access"/"cannot open directory" split, which needs error codes this project doesn't have yet.
fn list_one(dir: &str) -> bool {
    let fd = userlib::open(dir, O_RDONLY);
    if fd < 0 {
        userlib::write(2, b"ls: ");
        userlib::write(2, dir.as_bytes());
        userlib::write(2, b": cannot open\n");
        return false;
    }
    let fd = fd as usize;

    let mut buf = [0u8; GETDENTS_BUF_SIZE];
    let mut ok = true;
    loop {
        let n = userlib::getdents(fd, &mut buf);
        // Unlike a plain `read`, a negative `getdents` doesn't just mean "no more data" -- it
        // means `dir` opened fine but wasn't a directory at all (an ordinary file, say), so it's
        // kept apart from the `n == 0` "listing exhausted" case below, the same way the mature
        // reference's own `ls.rs` keeps the two apart.
        if n < 0 {
            userlib::write(2, b"ls: ");
            userlib::write(2, dir.as_bytes());
            userlib::write(2, b": not a directory\n");
            ok = false;
            break;
        }
        if n == 0 {
            break;
        }
        for raw in buf[..n as usize].chunks_exact(DIRENT_SIZE) {
            let Some(ent) = DirEnt::parse(raw) else {
                continue;
            };
            userlib::write(1, ent.name.as_bytes());
            userlib::write(1, b"\n");
        }
    }
    userlib::close(fd);
    ok
}

fn run(args: Args) -> ExitCode {
    let mut dirs: Vec<&str> = Vec::new();
    for arg in args.skip(1) {
        if arg == "--help" {
            return progs::help(USAGE, FLAGS);
        }
        dirs.push(arg);
    }
    if dirs.is_empty() {
        dirs.push("");
    }
    let mut ok = true;
    for dir in dirs {
        if !list_one(dir) {
            ok = false;
        }
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
