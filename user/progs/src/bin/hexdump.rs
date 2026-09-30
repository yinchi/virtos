//! A canonical hex+ASCII dump of each named file, 16 bytes per row: an 8-digit offset, each byte
//! in hex (an extra space after the 8th, for readability), then the same 16 bytes as ASCII
//! (`.` for anything not printable), and a closing line with the final offset (matching real
//! `hexdump -C`'s own convention -- printed even for an empty file, whose only output is that one
//! line, and even when the file's length is an exact multiple of 16, where it's not implied by any
//! partial row).
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use userlib::{Args, ExitCode};

userlib::entry_with_args!(run);

const ROW: usize = 16;
/// Width of the hex column for a full row: 16 `"xx "` triples, plus one extra space at the
/// midpoint -- a short final row is padded out to this so the `|ASCII|` column still lines up.
const HEX_COLUMN_WIDTH: usize = ROW * 3 + 1;
const USAGE: &str = "hexdump [file...]";
const FLAGS: &[(&str, &str)] = &[];

fn dump(data: &[u8]) {
    for (row_index, chunk) in data.chunks(ROW).enumerate() {
        let offset = row_index * ROW;
        let mut hex = String::new();
        let mut ascii = String::new();
        for (i, &b) in chunk.iter().enumerate() {
            hex.push_str(&format!("{b:02x} "));
            if i == 7 {
                hex.push(' ');
            }
            ascii.push(if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '.'
            });
        }
        let line = format!("{offset:08x}  {hex:<HEX_COLUMN_WIDTH$}|{ascii}|\n");
        userlib::write(1, line.as_bytes());
    }
    userlib::write(1, format!("{:08x}\n", data.len()).as_bytes());
}

fn run(args: Args) -> ExitCode {
    let mut any = false;
    let mut ok = true;
    for path in args.skip(1) {
        if path == "--help" {
            return progs::help(USAGE, FLAGS);
        }
        any = true;
        let Some(data) = progs::read_file(path) else {
            userlib::write(2, b"hexdump: ");
            userlib::write(2, path.as_bytes());
            userlib::write(2, b": cannot open\n");
            ok = false;
            continue;
        };
        dump(&data);
    }
    if !any {
        userlib::write(2, b"hexdump: missing file operand\n");
        return ExitCode(1);
    }
    if ok { ExitCode(0) } else { ExitCode(1) }
}
