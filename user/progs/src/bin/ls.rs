//! `ls [-a] [-l] [-F] [dir...]` -- lists the names in each named directory, one per line, in the
//! order `getdents` returns them (not sorted). No `dir` operand lists the working directory (an
//! empty path, which `open` resolves against it, same as every other relative path).
//! `-F`/`--classify` appends `/` to a directory's name and `*` to an executable file's -- the two
//! FAT attribute bits this project tracks (no `@`/`=`/`|` for symlinks/sockets/pipes, since none
//! of those exist here), read straight off each `getdents` record now that it carries them.
//! `-a`/`--all` shows names starting with `.` too (hidden otherwise, GNU's own convention) --
//! `.`/`..` themselves are never shown, `-a` or not: the kernel never returns them (`getdents`'s
//! own `list_dir` strips them unconditionally), so there's nothing for `-a` to resurrect, only
//! dot-named files to reveal. `-l` shows, ahead of each name, the same `d`/`w`/`x` attribute
//! triple and size `chmod`/`stat` already expose, no owner and no time (FAT has neither).
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use getargs::Arg;
use userlib::{
    ATTR_DIRECTORY, ATTR_EXEC, ATTR_READ_ONLY, Args, DIRENT_SIZE, DirEnt, ExitCode, O_RDONLY,
};

userlib::entry_with_args!(run);

const USAGE: &str = "ls [-a] [-l] [-F] [dir...]";
const FLAGS: &[(&str, &str)] = &[
    ("-a, --all", "show names starting with '.' too"),
    (
        "-l",
        "show the attribute triple and size ahead of each name",
    ),
    (
        "-F, --classify",
        "append / to directories and * to executable files",
    ),
];

/// Buffer size for one `getdents` call -- fetches several records per syscall rather than one, the
/// way the mature reference's own `ls.rs` does (its `BATCH = 8`), but sized to one memory page (4
/// KiB), a conventional buffer size, rather than an arbitrary record count with no stated reason:
/// 15 records per call at `DIRENT_SIZE`'s current 261 bytes.
const GETDENTS_BUF_SIZE: usize = 4096;

/// Lists one directory's entries to stdout. `false` if `dir` can't be opened, or opens but isn't
/// actually a directory (already reported to stderr either way) -- the first case uses the same
/// "cannot open" wording every other program here already uses, not GNU's own distinct "cannot
/// access"/"cannot open directory" split, which needs error codes this project doesn't have yet.
fn list_one(dir: &str, all: bool, long: bool, classify: bool) -> bool {
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
        for raw in buf[..n as usize].as_chunks::<DIRENT_SIZE>().0 {
            let Some(ent) = DirEnt::parse(raw) else {
                continue;
            };
            if !all && ent.name.starts_with('.') {
                continue;
            }
            if long {
                let type_c = if ent.attrs & ATTR_DIRECTORY != 0 {
                    b'd'
                } else {
                    b'-'
                };
                let w_c = if ent.attrs & ATTR_READ_ONLY != 0 {
                    b'-'
                } else {
                    b'w'
                };
                let x_c = if ent.attrs & ATTR_EXEC != 0 {
                    b'x'
                } else {
                    b'-'
                };
                userlib::write(1, &[type_c, w_c, x_c, b' ']);
                userlib::write(1, format!("{:>10} ", ent.size).as_bytes());
            }
            userlib::write(1, ent.name.as_bytes());
            if classify {
                let suffix = if ent.attrs & ATTR_DIRECTORY != 0 {
                    "/"
                } else if ent.attrs & ATTR_EXEC != 0 {
                    "*"
                } else {
                    ""
                };
                userlib::write(1, suffix.as_bytes());
            }
            userlib::write(1, b"\n");
        }
    }
    userlib::close(fd);
    ok
}

fn run(args: Args) -> ExitCode {
    let mut all = false;
    let mut long = false;
    let mut classify = false;
    let mut any = false;
    let mut opts = progs::cli::opts(args);
    loop {
        let arg = match progs::cli::next("ls", &mut opts) {
            Ok(Some(arg)) => arg,
            Ok(None) => break,
            Err(status) => return status,
        };
        match arg {
            Arg::Long("help") => return progs::help(USAGE, FLAGS),
            Arg::Short('a') | Arg::Long("all") => all = true,
            Arg::Short('l') => long = true,
            Arg::Short('F') | Arg::Long("classify") => classify = true,
            Arg::Positional(_) => any = true,
            other => return progs::cli::invalid("ls", other),
        }
    }

    let mut ok = true;
    if any {
        for dir in progs::cli::operands(args) {
            if !list_one(dir, all, long, classify) {
                ok = false;
            }
        }
    } else if !list_one("", all, long, classify) {
        ok = false;
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
