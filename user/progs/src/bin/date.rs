//! `date [-u] [-d @SECONDS] [-I[FMT] | -R | +FORMAT]`: prints the time from the real-time clock
//! (or the time `-d @SECONDS` names) in the local time zone -- named by `$TZ`, UTC if it is unset
//! or not a zone the database knows (`progs::local_zone`) -- or in UTC with `-u`.
//!
//! The calendar arithmetic, the time-zone rules (offsets, daylight saving, the abbreviations `EST`
//! and `EDT`) and the `strftime` conversions are `chrono`'s and `chrono-tz`'s -- the whole IANA
//! database is in this binary. `chrono` formats through `alloc`, which is the user heap.
#![no_std]
#![no_main]

extern crate alloc;

use core::fmt::{Display, Write};

use alloc::string::String;
use chrono::{DateTime, TimeZone, Utc};
use getargs::Arg;
use progs::{Out, cli, local_zone};
use userlib::{Args, Env, ExitCode, time};

userlib::entry_with_env!(run);

const USAGE: &str = "date [-u] [-d @SECONDS] [-I[FMT] | -R | +FORMAT]";
const FLAGS: &[(&str, &str)] = &[
    ("-u", "print UTC instead of the local time ($TZ)"),
    (
        "-d @N",
        "show the time N seconds after 1970-01-01 00:00:00 UTC instead of now",
    ),
    (
        "-I[FMT]",
        "ISO 8601: FMT is date (the default), hours, minutes or seconds",
    ),
    ("-R", "RFC 5322 format"),
    (
        "+FORMAT",
        "strftime-style format (%Y %m %d %H %M %S %s %Z %z %a %A %b %B %e %j %F %T ... %%)",
    ),
];

/// GNU's default layout in the C locale.
const DEFAULT_FORMAT: &str = "%a %b %e %H:%M:%S %Z %Y";
const RFC_EMAIL_FORMAT: &str = "%a, %d %b %Y %H:%M:%S %z";

/// What to print, and at what time.
struct Request {
    format: &'static str,
    /// `-d @N`; `None` is now.
    at: Option<i64>,
    /// `-u`: UTC, not the local zone.
    utc: bool,
}

/// Reads the arguments. `Err` is the status to return (`--help`, or a problem already reported).
fn parse(args: Args) -> Result<Request, ExitCode> {
    let mut format: Option<&'static str> = None;
    let mut at = None;
    let mut utc = false;
    let mut opts = cli::opts(args);

    while let Some(arg) = cli::next("date", &mut opts)? {
        let chosen = match arg {
            Arg::Long("help") => return Err(progs::help(USAGE, FLAGS)),
            Arg::Short('u') | Arg::Long("utc" | "universal") => {
                utc = true;
                continue;
            }
            Arg::Short('d') | Arg::Long("date") => {
                let text = cli::value("date", &mut opts)?;
                at = Some(parse_at(text)?);
                continue;
            }
            Arg::Short('R') | Arg::Long("rfc-email") => RFC_EMAIL_FORMAT,
            Arg::Short('I') | Arg::Long("iso-8601") => iso_format(opts.value_opt())?,
            Arg::Positional(text) => match text.strip_prefix('+') {
                Some(spec) => spec,
                None => return Err(invalid_date(text)),
            },
            other => return Err(cli::invalid("date", other)),
        };
        if format.replace(chosen).is_some() {
            cli::report("date", "multiple output formats specified");
            return Err(ExitCode(1));
        }
    }
    Ok(Request {
        format: format.unwrap_or(DEFAULT_FORMAT),
        at,
        utc,
    })
}

fn invalid_date(text: &str) -> ExitCode {
    cli::report("date", &alloc::format!("invalid date '{text}'"));
    ExitCode(1)
}

/// `-d`'s operand: `@N`, seconds since the epoch. (GNU also reads free-form dates and relative
/// times; there is no such parser here.)
fn parse_at(text: &str) -> Result<i64, ExitCode> {
    text.strip_prefix('@')
        .and_then(|digits| digits.parse::<i64>().ok())
        .ok_or_else(|| invalid_date(text))
}

/// The layout for `-I[FMT]`. The offset is the real one (`-04:00`), not always `+00:00`.
fn iso_format(precision: Option<&str>) -> Result<&'static str, ExitCode> {
    match precision {
        None | Some("date") => Ok("%Y-%m-%d"),
        Some("hours") => Ok("%Y-%m-%dT%H%:z"),
        Some("minutes") => Ok("%Y-%m-%dT%H:%M%:z"),
        Some("seconds") => Ok("%Y-%m-%dT%H:%M:%S%:z"),
        Some(other) => {
            cli::report(
                "date",
                &alloc::format!("invalid argument '{other}' for '--iso-8601'"),
            );
            cli::try_help("date");
            Err(ExitCode(1))
        }
    }
}

/// Prints `instant` in `format` and a newline. A conversion `chrono` does not know makes its
/// `Display` fail; nothing has been written by then.
fn print<Z: TimeZone>(instant: &DateTime<Z>, format: &str) -> ExitCode
where
    Z::Offset: Display,
{
    let mut text = String::new();
    if write!(text, "{}", instant.format(format)).is_err() {
        cli::report("date", &alloc::format!("invalid format '+{format}'"));
        return ExitCode(1);
    }
    text.push('\n');
    Out::new().put(text.as_bytes());
    ExitCode(0)
}

fn run(args: Args, env: Env) -> ExitCode {
    let request = match parse(args) {
        Ok(request) => request,
        Err(status) => return status,
    };
    let unix = request.at.unwrap_or_else(|| i64::from(time()));
    let Some(instant) = DateTime::<Utc>::from_timestamp(unix, 0) else {
        cli::report("date", "time out of range");
        return ExitCode(1);
    };
    if request.utc {
        print(&instant, request.format)
    } else {
        print(&instant.with_timezone(&local_zone(env)), request.format)
    }
}
