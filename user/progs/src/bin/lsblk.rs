//! `lsblk [-b]`: lists the block devices the kernel found (`blkinfo`, asked for device 0, 1, ...
//! until `ENODEV`): the size, and for a FAT volume its label and its volume ID (what `blkid` calls
//! the UUID). `MOUNTPOINT` is where the volume is mounted (`/` for the root), empty for a device
//! that is not. A device that holds no FAT volume has an empty label and UUID.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use abi::blk::BLK_FAT;
use abi::errno::{ENODEV, errmsg};
use progs::{Out, cli};
use textlib::human::human_size;
use textlib::table::render;
use userlib::{Args, ExitCode, blkinfo};

userlib::entry_with_args!(run);

const USAGE: &str = "lsblk [-b]";
const FLAGS: &[(&str, &str)] = &[("-b", "print sizes in bytes instead of `ls -h` units")];

/// `1M` rather than `1.0M`: a size that is exact needs no decimal.
fn size_text(bytes: u64) -> String {
    let text = human_size(bytes);
    match text.find(".0") {
        Some(at) if text[at + 2..].chars().all(|c| c.is_ascii_alphabetic()) => {
            let mut text = text;
            text.replace_range(at..at + 2, "");
            text
        }
        _ => text,
    }
}

fn run(args: Args) -> ExitCode {
    let mut bytes = false;
    let mut opts = cli::opts(args);
    loop {
        match cli::next("lsblk", &mut opts) {
            Ok(None) => break,
            Ok(Some(getargs::Arg::Long("help"))) => return progs::help(USAGE, FLAGS),
            Ok(Some(getargs::Arg::Short('b') | getargs::Arg::Long("bytes"))) => bytes = true,
            Ok(Some(other)) => return cli::invalid("lsblk", other),
            Err(status) => return status,
        }
    }

    let mut rows: Vec<Vec<String>> = Vec::new();
    for index in 0.. {
        let info = match blkinfo(index) {
            Ok(info) => info,
            Err(ENODEV) => break,
            Err(e) => {
                cli::report(
                    "lsblk",
                    &format!("cannot read the block devices: {}", errmsg(e)),
                );
                return ExitCode(1);
            }
        };
        let fat = info.flags & BLK_FAT != 0;
        let size = if bytes {
            format!("{}", info.capacity)
        } else {
            size_text(info.capacity)
        };
        let label = if fat {
            String::from_utf8_lossy(info.label()).into_owned()
        } else {
            String::new()
        };
        let uuid = if fat {
            let id = info.volume_id;
            format!("{:04X}-{:04X}", id >> 16, id & 0xffff)
        } else {
            String::new()
        };
        let mount = String::from_utf8_lossy(info.mount()).into_owned();
        rows.push(alloc::vec![size, label, uuid, mount]);
    }

    Out::new().put(render(&["SIZE", "LABEL", "UUID", "MOUNTPOINT"], &rows).as_bytes());
    ExitCode(0)
}
