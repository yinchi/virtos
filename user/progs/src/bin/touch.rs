//! `touch [-c] [-d STRING | -t STAMP | -r FILE] FILE...`: creates a file that does not exist, and
//! sets the modify time of each operand -- to now, or to the time `-d`, `-t` or `-r` names --
//! without changing what is in it. A directory can be touched too.
//!
//! Times written as wall-clock (`-d 2024-05-01 12:00`, `-t 202405011200`) are in the local zone,
//! `$TZ`, as `date` and `stat` show them; the kernel stores UTC. FAT keeps the modify time to 2
//! seconds, and its access time (a date only, which nothing reads) is not touched.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;

use chrono::{DateTime, Datelike, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use getargs::Arg;
use progs::{cli, local_zone};
use textlib::stamp::{When, fat_to_unix, parse_date, parse_stamp};
use userlib::{Args, Env, ExitCode, O_APPEND, O_WRONLY, TimeSet, close, open, stat, utimens};

userlib::entry_with_env!(run);

const USAGE: &str = "touch [-c] [-d STRING | -t STAMP | -r FILE] FILE...";
const FLAGS: &[(&str, &str)] = &[
    ("-c", "do not create a file that does not exist"),
    (
        "-d STRING",
        "use this time instead of now: @SECONDS, YYYY-MM-DD, or YYYY-MM-DD HH:MM[:SS] (T for the space, Z for UTC)",
    ),
    ("-t STAMP", "use [[CC]YY]MMDDhhmm[.ss] instead of now"),
    ("-r FILE", "use FILE's times instead of now"),
];

/// Where the time comes from.
enum Source {
    /// `-d`/`-t`: one instant.
    At(i64),
    /// `-r`: the reference file's modify time.
    Reference(TimeSet),
}

fn invalid_date(text: &str) -> ExitCode {
    cli::report("touch", &format!("invalid date format '{text}'"));
    ExitCode(1)
}

/// A wall-clock time in `zone` as seconds since 1970. `None` for one that does not exist there (a
/// daylight-saving gap); one that happens twice takes the earlier.
fn in_zone(zone: Tz, wall: &NaiveDateTime) -> Option<i64> {
    zone.from_local_datetime(wall)
        .earliest()
        .map(|t| t.timestamp())
}

fn run(args: Args, env: Env) -> ExitCode {
    cli::status(touch(args, env))
}

fn touch(args: Args, env: Env) -> Result<ExitCode, ExitCode> {
    let mut no_create = false;
    let mut source: Option<Source> = None;
    let mut files: Vec<&str> = Vec::new();
    let zone = local_zone(env);
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("touch", &mut opts)? {
        let mut choose = |new: Source| -> Result<(), ExitCode> {
            if source.is_some() {
                cli::report("touch", "cannot specify times from more than one source");
                cli::try_help("touch");
                return Err(ExitCode(1));
            }
            source = Some(new);
            Ok(())
        };
        match arg {
            Arg::Long("help") => return Ok(progs::help(USAGE, FLAGS)),
            Arg::Short('c') | Arg::Long("no-create") => no_create = true,
            Arg::Short('d') | Arg::Long("date") => {
                let text = cli::value("touch", &mut opts)?;
                let seconds = match parse_date(text) {
                    Some(When::Epoch(seconds)) => Some(seconds),
                    Some(When::Local(wall)) => in_zone(zone, &wall),
                    None => None,
                };
                choose(Source::At(seconds.ok_or_else(|| invalid_date(text))?))?;
            }
            Arg::Short('t') => {
                let text = cli::value("touch", &mut opts)?;
                let this_year = DateTime::<Utc>::from_timestamp(i64::from(userlib::time()), 0)
                    .map_or(1970, |now| now.year());
                let seconds = parse_stamp(text, this_year).and_then(|wall| in_zone(zone, &wall));
                choose(Source::At(seconds.ok_or_else(|| invalid_date(text))?))?;
            }
            Arg::Short('r') | Arg::Long("reference") => {
                let reference = cli::value("touch", &mut opts)?;
                match stat(reference) {
                    Ok(info) => choose(Source::Reference(
                        fat_to_unix(info.modified_date, info.modified_time)
                            .map_or(TimeSet::Omit, TimeSet::At),
                    ))?,
                    Err(_) => {
                        cli::report(
                            "touch",
                            &format!("failed to get attributes of '{reference}'"),
                        );
                        return Err(ExitCode(1));
                    }
                }
            }
            Arg::Positional(path) => files.push(path),
            other => return Err(cli::invalid("touch", other)),
        }
    }
    if files.is_empty() {
        cli::report("touch", "missing file operand");
        cli::try_help("touch");
        return Err(ExitCode(1));
    }
    let mtime = match source {
        None => TimeSet::Now,
        Some(Source::At(seconds)) => TimeSet::At(seconds),
        Some(Source::Reference(mtime)) => mtime,
    };

    let mut status = 0;
    for path in files {
        // This project's `stat` has no separate "no such file" error, so any failure to examine
        // the path counts as it not existing: `-c` leaves it alone, and otherwise the create
        // below reports whatever really is wrong.
        if stat(path).is_err() {
            if no_create {
                continue;
            }
            // Opening for write creates a missing file; the append open leaves nothing to
            // truncate.
            let fd = open(path, O_WRONLY | O_APPEND);
            if fd < 0 {
                cli::report("touch", &format!("cannot touch '{path}'"));
                status = 1;
                continue;
            }
            close(fd as usize);
        }
        if utimens(path, TimeSet::Omit, mtime) < 0 {
            cli::report("touch", &format!("setting times of '{path}' failed"));
            status = 1;
        }
    }
    Ok(ExitCode(status))
}
