//! The parser for `/etc/fstab`, the file Linux's `mount -a` reads (and, at boot, the init system): which volumes to mount
//! where. The init shell reads it at start-up and mounts each line in file order (`shell::mount_fstab`).
//!
//! One entry per line, whitespace-separated: `<source> <mount point> <type> <options> [<dump> [<pass>]]`.
//!
//! - a blank line, or one whose first non-blank character is `#`, is ignored;
//! - the **source** is `LABEL=name` or `UUID=XXXX-XXXX` (`mounttable::Source`), naming a volume by what is on it;
//! - the **mount point** is an absolute path (no blanks in it: there is no `\040` escape);
//! - the **type** is `vfat` or `fat`;
//! - the **options** are comma-separated, and there are only two choices to make: `auto` (mount it at start-up) or `noauto`
//!   (leave the line for `mount` by hand), and `fail` (a source no device matches is reported) or `nofail` (it is not).
//!   `defaults` is `auto,fail` and may be written for form's sake; the others are read left to right, the last of a pair
//!   winning. Anything else is refused rather than ignored -- `ro` most of all (read-only mounts are not enforced, and mounting
//!   a volume read-write that the file says is read-only would be a surprise), as are the options that only mean something
//!   to Linux's FAT driver or to a system with owners, modes and access times (`noatime`, `rw`, `umask=`, ...);
//! - `dump` and `pass`, which only matter to tools this system does not have, are accepted and ignored; more fields than
//!   that are a problem;
//! - a line that is not of that shape is skipped and reported, never fatal.
//!
//! The parser only reads the file; whether a source names a device, whether the mount point exists, and what a line for `/`
//! means (the root is chosen before this file can be read, so such a line is only checked) are for the caller.
//!
//! Pure `no_std` + `alloc`, so it is tested on the host.

use alloc::string::String;
use alloc::vec::Vec;

use crate::fs::mounttable::Source;

/// A line that was skipped, and why.
#[derive(Debug, PartialEq, Eq)]
pub struct Problem {
    /// 1-based.
    pub line: usize,
    pub why: &'static str,
}

/// One usable line.
#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    /// 1-based, for the caller's notes.
    pub line: usize,
    pub source: Source,
    /// The source as written (`LABEL=HOME`), for messages and for handing to `mount`.
    pub source_text: String,
    /// The mount point as written (absolute, not yet normalized).
    pub point: String,
    /// `noauto`: the line is there for `mount SOURCE TARGET` by hand, not to be mounted at start-up (default `auto`).
    pub noauto: bool,
    /// `nofail`: a source that no device matches is not worth a note (default `fail`: it is reported).
    pub nofail: bool,
}

/// What `parse` found: the usable entries in file order, and the lines it had to skip.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub entries: Vec<Entry>,
    pub problems: Vec<Problem>,
}

/// Reads the options field: `(noauto, nofail)`, or why it is refused.
fn options(field: &str) -> Result<(bool, bool), &'static str> {
    let (mut noauto, mut nofail) = (false, false);
    for option in field.split(',') {
        match option {
            "defaults" => {}
            "auto" => noauto = false,
            "noauto" => noauto = true,
            "fail" => nofail = false,
            "nofail" => nofail = true,
            "ro" => return Err("read-only mounts are not supported"),
            "" => return Err("empty option"),
            _ => return Err("unsupported option"),
        }
    }
    Ok((noauto, nofail))
}

/// Parses the text of an fstab.
pub fn parse(text: &str) -> Parsed {
    let mut parsed = Parsed::default();
    for (index, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let number = index + 1;
        let mut problem = |why| parsed.problems.push(Problem { line: number, why });
        let fields: Vec<&str> = line.split_ascii_whitespace().collect();
        if fields.len() < 4 {
            problem("expected <source> <mount point> <type> <options>");
            continue;
        }
        if fields.len() > 6 {
            problem("too many fields");
            continue;
        }
        let Some(source) = Source::parse(fields[0]) else {
            problem("source must be LABEL=name or UUID=XXXX-XXXX");
            continue;
        };
        if !fields[1].starts_with('/') {
            problem("mount point must be an absolute path");
            continue;
        }
        if !matches!(fields[2], "vfat" | "fat") {
            problem("unsupported filesystem type (only vfat)");
            continue;
        }
        let (noauto, nofail) = match options(fields[3]) {
            Ok(flags) => flags,
            Err(why) => {
                problem(why);
                continue;
            }
        };
        parsed.entries.push(Entry {
            line: number,
            source,
            source_text: String::from(fields[0]),
            point: String::from(fields[1]),
            noauto,
            nofail,
        });
    }
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(p: &Parsed) -> Vec<(&str, &str)> {
        p.entries
            .iter()
            .map(|e| (e.source_text.as_str(), e.point.as_str()))
            .collect()
    }

    #[test]
    fn a_plain_line() {
        let p = parse("LABEL=HOME /root vfat defaults\n");
        assert_eq!(points(&p), [("LABEL=HOME", "/root")]);
        assert_eq!(p.entries[0].source, Source::Label(String::from("HOME")));
        assert_eq!(p.entries[0].line, 1);
        assert!(!p.entries[0].noauto && !p.entries[0].nofail);
        assert!(p.problems.is_empty());
    }

    #[test]
    fn uuid_sources_and_fat_as_a_type() {
        let p = parse("UUID=5E6F-7A8B /mnt fat defaults\n");
        assert_eq!(p.entries[0].source, Source::Uuid(0x5e6f_7a8b));
    }

    #[test]
    fn fields_may_be_separated_by_any_blanks() {
        let p = parse("  LABEL=HOME \t /root\t\tvfat   defaults  \n");
        assert_eq!(points(&p), [("LABEL=HOME", "/root")]);
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let p = parse(
            "# <source> <mount point> <type> <options>\n\n   \n  # indented\nLABEL=HOME /root vfat defaults\n",
        );
        assert_eq!(points(&p), [("LABEL=HOME", "/root")]);
        assert_eq!(p.entries[0].line, 5);
        assert!(p.problems.is_empty());
    }

    #[test]
    fn dump_and_pass_are_ignored() {
        assert_eq!(
            parse("LABEL=HOME /root vfat defaults 0 2\n").entries.len(),
            1
        );
        assert_eq!(parse("LABEL=HOME /root vfat defaults 0\n").entries.len(), 1);
        assert_eq!(
            parse("LABEL=HOME /root vfat defaults 0 2 3\n").problems,
            [Problem {
                line: 1,
                why: "too many fields"
            }]
        );
    }

    #[test]
    fn the_options_that_mean_something() {
        let p = parse(
            "LABEL=A /a vfat noauto\nLABEL=B /b vfat defaults,nofail\nLABEL=C /c vfat defaults\nLABEL=D /d vfat auto,fail\n",
        );
        assert_eq!(
            p.entries
                .iter()
                .map(|e| (e.noauto, e.nofail))
                .collect::<Vec<_>>(),
            [(true, false), (false, true), (false, false), (false, false)]
        );
    }

    #[test]
    fn defaults_is_auto_and_fail_and_the_last_of_a_pair_wins() {
        let flags = |options: &str| {
            let p = parse(&alloc::format!("LABEL=A /a vfat {options}\n"));
            (p.entries[0].noauto, p.entries[0].nofail)
        };
        assert_eq!(flags("defaults"), (false, false));
        assert_eq!(flags("noauto,defaults"), (true, false)); // `defaults` does not undo a choice
        assert_eq!(flags("nofail,defaults"), (false, true));
        assert_eq!(flags("noauto,auto"), (false, false));
        assert_eq!(flags("auto,noauto"), (true, false));
        assert_eq!(flags("nofail,fail"), (false, false));
        assert_eq!(flags("noauto,nofail"), (true, true));
    }

    #[test]
    fn read_only_and_unknown_options_refuse_the_line() {
        let p = parse(
            "LABEL=A /a vfat ro\nLABEL=B /b vfat defaults,ro\nLABEL=C /c vfat defaults,sync\nLABEL=D /d vfat defaults,\nLABEL=E /e vfat ,defaults\nLABEL=F /f vfat rw\nLABEL=G /g vfat noatime\n",
        );
        assert!(p.entries.is_empty());
        assert_eq!(
            p.problems,
            [
                Problem {
                    line: 1,
                    why: "read-only mounts are not supported"
                },
                Problem {
                    line: 2,
                    why: "read-only mounts are not supported"
                },
                Problem {
                    line: 3,
                    why: "unsupported option"
                },
                Problem {
                    line: 4,
                    why: "empty option"
                },
                Problem {
                    line: 5,
                    why: "empty option"
                },
                Problem {
                    line: 6,
                    why: "unsupported option"
                },
                Problem {
                    line: 7,
                    why: "unsupported option"
                },
            ]
        );
    }

    #[test]
    fn a_line_that_is_not_an_entry_is_skipped_and_numbered() {
        let p = parse(
            "LABEL=A /a vfat defaults\nLABEL=B /b\n/dev/vdb /c vfat defaults\nLABEL=D d vfat defaults\nLABEL=E /e ext4 defaults\nLABEL=F /f vfat defaults\n",
        );
        assert_eq!(points(&p), [("LABEL=A", "/a"), ("LABEL=F", "/f")]);
        assert_eq!(
            p.problems,
            [
                Problem {
                    line: 2,
                    why: "expected <source> <mount point> <type> <options>"
                },
                Problem {
                    line: 3,
                    why: "source must be LABEL=name or UUID=XXXX-XXXX"
                },
                Problem {
                    line: 4,
                    why: "mount point must be an absolute path"
                },
                Problem {
                    line: 5,
                    why: "unsupported filesystem type (only vfat)"
                },
            ]
        );
    }

    #[test]
    fn bad_sources() {
        for bad in [
            "label=A",
            "LABEL=",
            "UUID=5E6F7A8B",
            "UUID=5E6F-7A8",
            "vdb",
            "LABEL",
        ] {
            let p = parse(&alloc::format!("{bad} /a vfat defaults\n"));
            assert_eq!(p.problems.len(), 1, "{bad:?}");
            assert!(p.entries.is_empty(), "{bad:?}");
        }
    }

    #[test]
    fn crlf_endings_and_a_missing_final_newline_are_fine() {
        let p = parse("LABEL=A /a vfat defaults\r\nLABEL=B /b vfat defaults");
        assert_eq!(points(&p), [("LABEL=A", "/a"), ("LABEL=B", "/b")]);
    }

    #[test]
    fn lines_keep_file_order_and_repeats() {
        let p =
            parse("LABEL=A /x vfat defaults\nLABEL=B /x vfat defaults\nLABEL=A /y vfat defaults\n");
        assert_eq!(
            points(&p),
            [("LABEL=A", "/x"), ("LABEL=B", "/x"), ("LABEL=A", "/y")]
        );
    }

    #[test]
    fn an_empty_file_is_nothing_to_mount() {
        assert_eq!(parse(""), Parsed::default());
        assert_eq!(parse("\n# only a comment\n"), Parsed::default());
    }
}
