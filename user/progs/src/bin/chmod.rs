//! `chmod +x|-x|+w|-w [-R] file...` -- only those four symbolic modes exist (a single user, and
//! FAT has just an executable bit and a read-only bit), so anything else, an octal `755`
//! included, is an invalid mode. `-R` recurses into a directory operand, applying the same mode
//! to every descendant, files and subdirectories alike.
#![no_std]
#![no_main]

extern crate alloc;

use userlib::{ATTR_DIRECTORY, Args, DIRENT_SIZE, DirEnt, ExitCode, O_RDONLY};

userlib::entry_with_args!(run);

const USAGE: &str = "chmod +x|-x|+w|-w [-R] file...";
const FLAGS: &[(&str, &str)] = &[("-R", "recurse into directory operands")];

/// Same buffer size as `ls.rs`'s own batched-`getdents` idiom -- one memory page.
const GETDENTS_BUF_SIZE: usize = 4096;

fn report(path: &str) {
    userlib::write(2, b"chmod: ");
    userlib::write(2, path.as_bytes());
    userlib::write(2, b": cannot change permissions\n");
}

/// The attribute bits a mode changes -- `(set, clear)` -- for one of the four symbolic modes.
/// FAT has no write bit, only a read-only bit, so `+w` clears it and `-w` sets it.
fn mode_bits(mode: &str) -> Option<(u8, u8)> {
    match mode {
        "+x" => Some((userlib::ATTR_EXEC, 0)),
        "-x" => Some((0, userlib::ATTR_EXEC)),
        "+w" => Some((0, userlib::ATTR_READ_ONLY)),
        "-w" => Some((userlib::ATTR_READ_ONLY, 0)),
        _ => None,
    }
}

/// Applies `set`/`clear` to every descendant of the directory `path`, depth-first, files and
/// subdirectories alike -- `path` itself is not touched (the caller already did that). Matches
/// `ls.rs`'s own stack-buffer batched-`getdents` idiom: recurses into a child only when its
/// `DirEnt` record itself carries `ATTR_DIRECTORY`, never a separate `stat` call per child.
/// `false` (after reporting it) on any failure, having still attempted every entry.
fn chmod_recursive(path: &str, set: u8, clear: u8) -> bool {
    let fd = userlib::open(path, O_RDONLY);
    if fd < 0 {
        report(path);
        return false;
    }
    let fd = fd as usize;
    let mut buf = [0u8; GETDENTS_BUF_SIZE];
    let mut ok = true;
    loop {
        let n = userlib::getdents(fd, &mut buf);
        if n < 0 {
            report(path);
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
            let child = progs::join(path, ent.name);
            if ent.attrs & ATTR_DIRECTORY != 0 && !chmod_recursive(&child, set, clear) {
                ok = false;
            }
            if userlib::chmod(&child, set, clear) < 0 {
                report(&child);
                ok = false;
            }
        }
    }
    userlib::close(fd);
    ok
}

/// What one command-line argument is.
enum Item {
    Help,
    Recursive,
    Mode(&'static str),
    File(&'static str),
    /// Looks like an option this program does not have.
    Bad(&'static str),
}

/// The arguments in order, classified. `chmod` cannot use `getargs` for this: its modes `-x` and
/// `-w` look like options, so an argument is an option only if it is `-R`, `--help` or `--`, and
/// a dash-word that is not one of the four modes is unrecognized. The first non-option argument
/// is the mode; everything after is a file operand.
struct Items {
    args: core::iter::Skip<Args>,
    ended: bool,
    mode_seen: bool,
}

impl Items {
    fn new(args: Args) -> Self {
        Items {
            args: args.skip(1),
            ended: false,
            mode_seen: false,
        }
    }
}

impl Iterator for Items {
    type Item = Item;

    fn next(&mut self) -> Option<Item> {
        loop {
            let arg = self.args.next()?;
            if !self.ended {
                if arg == "--" {
                    self.ended = true;
                    continue;
                }
                if arg == "--help" {
                    return Some(Item::Help);
                }
                if arg == "-R" {
                    return Some(Item::Recursive);
                }
                if arg.len() > 1 && arg.starts_with('-') && mode_bits(arg).is_none() {
                    return Some(Item::Bad(arg));
                }
            }
            if !self.mode_seen {
                self.mode_seen = true;
                return Some(Item::Mode(arg));
            }
            return Some(Item::File(arg));
        }
    }
}

fn run(args: Args) -> ExitCode {
    let mut mode: Option<&str> = None;
    let mut bits = (0u8, 0u8);
    let mut recursive = false;
    let mut files = 0usize;

    // First pass: the mode and the flags, so `-R` applies to every file wherever it's written.
    for item in Items::new(args) {
        match item {
            Item::Help => return progs::help(USAGE, FLAGS),
            Item::Recursive => recursive = true,
            Item::Bad(arg) => {
                userlib::write(2, b"chmod: invalid option -- '");
                userlib::write(2, arg.as_bytes());
                userlib::write(2, b"'\n");
                return ExitCode(1);
            }
            Item::Mode(text) => {
                let Some(found) = mode_bits(text) else {
                    userlib::write(2, b"chmod: invalid mode: '");
                    userlib::write(2, text.as_bytes());
                    userlib::write(2, b"'\n");
                    return ExitCode(1);
                };
                mode = Some(text);
                bits = found;
            }
            Item::File(_) => files += 1,
        }
    }
    if mode.is_none() {
        userlib::write(2, b"chmod: missing operand\n");
        return ExitCode(1);
    }
    if files == 0 {
        userlib::write(2, b"chmod: missing operand after '");
        userlib::write(2, mode.unwrap_or("").as_bytes());
        userlib::write(2, b"'\n");
        return ExitCode(1);
    }
    let (set, clear) = bits;

    let mut status = 0;
    for item in Items::new(args) {
        let Item::File(path) = item else { continue };

        let mut ok = true;
        if userlib::chmod(path, set, clear) < 0 {
            report(path);
            ok = false;
        }

        if recursive && ok {
            match userlib::stat(path) {
                Ok(info) if info.attrs & ATTR_DIRECTORY != 0 => {
                    if !chmod_recursive(path, set, clear) {
                        ok = false;
                    }
                }
                Ok(_) => {} // a plain file: -R beyond the chmod already done is a no-op
                Err(_) => {
                    report(path);
                    ok = false;
                }
            }
        }

        if !ok {
            status = 1;
        }
    }

    ExitCode(status)
}
