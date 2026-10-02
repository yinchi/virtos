//! `ls [-1] [-a] [-d] [-F] [-h] [-l] [-R] [-r] [-S] [-t] [FILE...]`: lists the names in each named
//! directory, one per line (`-1` is accepted and changes nothing), **sorted by name** bytewise
//! (the C locale's order: `Mid` before `alpha`). No operand lists the working directory. A file
//! operand is listed as itself (files first, then each directory in name order); `-d` lists a
//! directory operand itself instead of its contents. `-R` recurses (`dir:` headers). `-r`
//! reverses the order, `-S` sorts largest first, `-t` newest modify time first (a `stat` per
//! entry), ties by name. `-F`/`--classify` appends `/` to a directory's name and `*` to an
//! executable's. `-a`/`--all` shows names starting with `.` too (hidden otherwise);
//! `.`/`..` themselves are never shown -- the kernel never returns them. `-l` shows, ahead of each
//! name, the `d`/`w`/`x` attribute triple and the size, no owner and no link count (FAT has
//! neither), and with `-h` the size reads `1.5K`, rounded up like GNU's.
#![no_std]
#![no_main]

extern crate alloc;

use core::fmt::Write;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use getargs::Arg;
use progs::{Out, ReadDirError, cli, join, read_dir};
use textlib::human::human_size;
use userlib::{ATTR_DIRECTORY, ATTR_EXEC, ATTR_READ_ONLY, Args, ExitCode, Stat, stat};

userlib::entry_with_args!(run);

const USAGE: &str = "ls [-1] [-a] [-d] [-F] [-h] [-l] [-R] [-r] [-S] [-t] [FILE...]";
const FLAGS: &[(&str, &str)] = &[
    (
        "-1",
        "one entry per line (always the case; accepted for compatibility)",
    ),
    ("-a, --all", "show names starting with '.' too"),
    (
        "-d, --directory",
        "list directories themselves, not their contents",
    ),
    (
        "-F, --classify",
        "append / to directories and * to executable files",
    ),
    ("-h, --human-readable", "with -l, sizes like 1.5K"),
    ("-l", "long format: d/w/x flags, size, name"),
    ("-R, --recursive", "list subdirectories recursively"),
    ("-r, --reverse", "reverse the order"),
    ("-S", "sort by size, largest first"),
    ("-t", "sort by modify time, newest first"),
];

#[derive(Default)]
struct Options {
    all: bool,
    dir_only: bool,
    classify: bool,
    human: bool,
    long: bool,
    recursive: bool,
    reverse: bool,
    by_size: bool,
    by_time: bool,
}

/// One line of a listing.
struct Item {
    name: String,
    size: u32,
    attrs: u8,
    /// The packed FAT modify date and time, which sort in time order; only filled in for `-t`.
    time: u32,
}

impl Item {
    fn is_dir(&self) -> bool {
        self.attrs & ATTR_DIRECTORY != 0
    }
}

fn time_key(info: &Stat) -> u32 {
    (u32::from(info.modified_date) << 16) | u32::from(info.modified_time)
}

/// Sorts `items` as the options say: by name (bytewise), or by size or time, largest or newest
/// first with the name breaking ties; `-r` reverses whichever it is.
fn sort(items: &mut [Item], o: &Options) {
    items.sort_by(|a, b| {
        let by_name = a.name.as_bytes().cmp(b.name.as_bytes());
        if o.by_size {
            b.size.cmp(&a.size).then(by_name)
        } else if o.by_time {
            b.time.cmp(&a.time).then(by_name)
        } else {
            by_name
        }
    });
    if o.reverse {
        items.reverse();
    }
}

fn print(out: &mut Out, item: &Item, o: &Options) {
    if o.long {
        let type_c = if item.is_dir() { 'd' } else { '-' };
        let w_c = if item.attrs & ATTR_READ_ONLY != 0 {
            '-'
        } else {
            'w'
        };
        let x_c = if item.attrs & ATTR_EXEC != 0 {
            'x'
        } else {
            '-'
        };
        if o.human {
            let _ = write!(
                out,
                "{type_c}{w_c}{x_c} {:>6} ",
                human_size(u64::from(item.size))
            );
        } else {
            let _ = write!(out, "{type_c}{w_c}{x_c} {:>10} ", item.size);
        }
    }
    let suffix = match (o.classify, item.attrs) {
        (true, a) if a & ATTR_DIRECTORY != 0 => "/",
        (true, a) if a & ATTR_EXEC != 0 => "*",
        _ => "",
    };
    let _ = writeln!(out, "{}{suffix}", item.name);
}

/// Lists the contents of the directory `path`, and with `-R` each subdirectory after it. `false`
/// (already reported) if anything could not be listed; what could be listed still is.
fn list_dir(out: &mut Out, path: &str, o: &Options) -> bool {
    let entries = match read_dir(path) {
        Ok(entries) => entries,
        Err(ReadDirError::Open) => {
            cli::report("ls", &format!("{path}: cannot open"));
            return false;
        }
        Err(ReadDirError::Read) => {
            cli::report("ls", &format!("{path}: not a directory"));
            return false;
        }
    };
    let mut items: Vec<Item> = entries
        .into_iter()
        .filter(|ent| o.all || !ent.name.starts_with('.'))
        .map(|ent| Item {
            name: ent.name,
            size: ent.size,
            attrs: ent.attrs,
            time: 0,
        })
        .collect();
    if o.by_time && !o.by_size {
        for item in &mut items {
            if let Ok(info) = stat(&join(path, &item.name)) {
                item.time = time_key(&info);
            }
        }
    }
    sort(&mut items, o);
    for item in &items {
        print(out, item, o);
    }

    let mut ok = true;
    if o.recursive {
        for item in items.iter().filter(|item| item.is_dir()) {
            let child = join(path, &item.name);
            let _ = writeln!(out);
            let _ = writeln!(out, "{child}:");
            ok &= list_dir(out, &child, o);
        }
    }
    ok
}

fn run(args: Args) -> ExitCode {
    cli::status(list(args))
}

fn list(args: Args) -> Result<ExitCode, ExitCode> {
    let mut o = Options::default();
    let mut n_operands = 0usize;
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("ls", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(progs::help(USAGE, FLAGS)),
            Arg::Short('a') | Arg::Long("all") => o.all = true,
            Arg::Short('d') | Arg::Long("directory") => o.dir_only = true,
            Arg::Short('F') | Arg::Long("classify") => o.classify = true,
            Arg::Short('h') | Arg::Long("human-readable") => o.human = true,
            Arg::Short('l') => o.long = true,
            Arg::Short('R') | Arg::Long("recursive") => o.recursive = true,
            Arg::Short('r') | Arg::Long("reverse") => o.reverse = true,
            Arg::Short('S') => o.by_size = true,
            Arg::Short('t') => o.by_time = true,
            // One entry per line is all this `ls` ever does, so `-1` is accepted and changes
            // nothing.
            Arg::Short('1') => {}
            Arg::Positional(_) => n_operands += 1,
            other => return Err(cli::invalid("ls", other)),
        }
    }

    let mut operands: Vec<&'static str> = cli::operands(args).collect();
    if operands.is_empty() {
        operands.push(".");
    }

    // Each operand is a file (listed as itself, all of them first) or a directory (its contents,
    // after them).
    let mut out = Out::new();
    let mut status = 0;
    let mut files: Vec<Item> = Vec::new();
    let mut dirs: Vec<&str> = Vec::new();
    for &operand in &operands {
        match stat(operand) {
            Ok(info) if info.is_dir && !o.dir_only => dirs.push(operand),
            Ok(info) => files.push(Item {
                name: String::from(operand),
                size: info.size,
                attrs: info.attrs,
                time: time_key(&info),
            }),
            Err(_) => {
                cli::report("ls", &format!("{operand}: cannot open"));
                status = 1;
            }
        }
    }

    sort(&mut files, &o);
    for item in &files {
        print(&mut out, item, &o);
    }

    // A header names each directory when there is more than one thing to list, or when recursing.
    let headers = n_operands > 1 || o.recursive;
    // Directories come in name order (or as `-r` says), as GNU does for operands.
    dirs.sort();
    if o.reverse {
        dirs.reverse();
    }
    for (i, dir) in dirs.iter().enumerate() {
        if i > 0 || !files.is_empty() {
            let _ = writeln!(out);
        }
        if headers {
            let _ = writeln!(out, "{dir}:");
        }
        if !list_dir(&mut out, dir, &o) {
            status = 1;
        }
    }
    Ok(ExitCode(status))
}
