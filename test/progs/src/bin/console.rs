//! Exercises the console `ioctl`s (`TIOCGWINSZ`, `CONSOLE_READ_KEY`, `CONSOLE_DRAW`,
//! `CONSOLE_CLEAR`), one case per first argument, printing the result for `test/cases/console.py`
//! to compare. Raw `svc`s throughout (`testprogs::raw`), so a wrapper that checked its arguments
//! could not hide a kernel that did not.
//!
//! - `winsize FD`: `rows cols`, or the negated errno.
//! - `read-key FD`: blocks for one key, prints `code mods ch`, or the negated errno.
//! - `read-key-ptr FD ADDR`, `winsize-ptr FD ADDR`, `draw-ptr ADDR`: a chosen user pointer.
//! - `ioctl FD REQUEST`: the raw result of an arbitrary request (argument 0).
//! - `draw`: one full frame (rows 0-2 all `X`: plain, inverse, dim; the rest cycle the alphabet).
//! - `draw-hold`: the same frame, then blocks on a key before printing, so a test can look at it.
//! - `draw-timing [N]`: N full frames back to back (default 30); prints the whole seconds taken (the RTC's resolution).
//! - `draw-bad-size`, `draw-bad-char`, `draw-wide-edge`, `draw-cursor-oob`: one violation each.
#![no_std]
#![no_main]

extern crate alloc;

use abi::SYS_IOCTL;
use abi::ioctl::{
    ATTR_DIM, ATTR_INVERSE, CELL_SIZE, CONSOLE_DRAW, CONSOLE_DRAW_HEADER_SIZE, CONSOLE_READ_KEY,
    Cell, ConsoleDraw, TIOCGWINSZ,
};
use alloc::format;
use alloc::vec::Vec;
use testprogs::raw;
use userlib::{Args, ExitCode, write};

userlib::entry_with_args!(run);

const ROWS: usize = 30;
const COLS: usize = 80;

fn say(text: &str) {
    write(1, text.as_bytes());
}

fn number(arg: Option<&str>) -> usize {
    let arg = arg.unwrap_or("0");
    match arg.strip_prefix("0x") {
        Some(hex) => usize::from_str_radix(hex, 16).unwrap_or(0),
        None => arg.parse().unwrap_or(0),
    }
}

/// A header and `ROWS * COLS` cells, ready for `CONSOLE_DRAW`.
fn frame(header: ConsoleDraw, cell_at: impl Fn(usize, usize) -> Cell) -> Vec<u8> {
    let mut buf = Vec::with_capacity(CONSOLE_DRAW_HEADER_SIZE + ROWS * COLS * CELL_SIZE);
    buf.extend_from_slice(&header.encode());
    for row in 0..ROWS {
        for col in 0..COLS {
            buf.extend_from_slice(&cell_at(row, col).encode());
        }
    }
    buf
}

fn header() -> ConsoleDraw {
    ConsoleDraw {
        rows: ROWS as u16,
        cols: COLS as u16,
        cursor_row: 5,
        cursor_col: 7,
    }
}

/// Rows 0, 1, 2 are all `X`, differing only in attribute; the rest cycle through the alphabet.
fn good_cell(row: usize, col: usize) -> Cell {
    match row {
        0 => Cell::plain('X'),
        1 => Cell {
            ch: 'X' as u32,
            attr: ATTR_INVERSE,
        },
        2 => Cell {
            ch: 'X' as u32,
            attr: ATTR_DIM,
        },
        _ => Cell::plain((b'a' + ((row + col) % 26) as u8) as char),
    }
}

fn draw(buf: &[u8]) -> isize {
    raw(SYS_IOCTL, 1, CONSOLE_DRAW, buf.as_ptr() as usize, 0)
}

fn report(result: isize) -> ExitCode {
    say(&format!("{result}\n"));
    ExitCode(0)
}

fn run(mut args: Args) -> ExitCode {
    let _program_name = args.next();
    let mode = args.next().unwrap_or("");
    match mode {
        "winsize" | "winsize-ptr" => {
            let fd = number(args.next());
            if mode == "winsize-ptr" {
                return report(raw(SYS_IOCTL, fd, TIOCGWINSZ, number(args.next()), 0));
            }
            let mut buf = [0u8; 8];
            let ret = raw(SYS_IOCTL, fd, TIOCGWINSZ, buf.as_mut_ptr() as usize, 0);
            if ret < 0 {
                return report(ret);
            }
            let rows = u16::from_le_bytes([buf[0], buf[1]]);
            let cols = u16::from_le_bytes([buf[2], buf[3]]);
            let (xpx, ypx) = (
                u16::from_le_bytes([buf[4], buf[5]]),
                u16::from_le_bytes([buf[6], buf[7]]),
            );
            say(&format!("{rows} {cols} {xpx} {ypx}\n"));
            ExitCode(0)
        }
        "read-key" | "read-key-ptr" => {
            let fd = number(args.next());
            if mode == "read-key-ptr" {
                return report(raw(SYS_IOCTL, fd, CONSOLE_READ_KEY, number(args.next()), 0));
            }
            let mut buf = [0u8; 8];
            let ret = raw(
                SYS_IOCTL,
                fd,
                CONSOLE_READ_KEY,
                buf.as_mut_ptr() as usize,
                0,
            );
            if ret < 0 {
                return report(ret);
            }
            let code = u16::from_le_bytes([buf[0], buf[1]]);
            let ch = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
            say(&format!("{code} {} {ch}\n", buf[2]));
            ExitCode(0)
        }
        "ioctl" => {
            let fd = number(args.next());
            let request = number(args.next());
            report(raw(SYS_IOCTL, fd, request, 0, 0))
        }
        "draw" => report(draw(&frame(header(), good_cell))),
        "draw-hold" => {
            let ret = draw(&frame(header(), good_cell));
            let mut key = [0u8; 8];
            raw(SYS_IOCTL, 0, CONSOLE_READ_KEY, key.as_mut_ptr() as usize, 0);
            report(ret)
        }
        "draw-timing" => {
            // The RTC has second resolution (the counter registers are not open to EL0 here), so
            // time enough frames for that to be a usable measurement.
            let n = args.next().map_or(30, |a| number(Some(a)));
            let buf = frame(header(), good_cell);
            let start = userlib::time();
            for _ in 0..n {
                draw(&buf);
            }
            let secs = userlib::time() - start;
            say(&format!("{n} frames in {secs} s\n"));
            ExitCode(0)
        }
        "draw-bad-size" => {
            let mut h = header();
            h.rows -= 1;
            report(draw(&frame(h, good_cell)))
        }
        "draw-bad-char" => {
            // A surrogate, then (separately) a control character: neither is a drawable scalar.
            let surrogate = draw(&frame(header(), |r, c| {
                if (r, c) == (4, 4) {
                    Cell {
                        ch: 0xD800,
                        attr: 0,
                    }
                } else {
                    good_cell(r, c)
                }
            }));
            let control = draw(&frame(header(), |r, c| {
                if (r, c) == (4, 4) {
                    Cell { ch: 0x09, attr: 0 }
                } else {
                    good_cell(r, c)
                }
            }));
            say(&format!("{surrogate} {control}\n"));
            ExitCode(0)
        }
        "draw-wide-edge" => {
            // A wide glyph in the last column has no room for its second half.
            report(draw(&frame(header(), |r, c| {
                if (r, c) == (4, COLS - 1) {
                    Cell::plain('日')
                } else {
                    good_cell(r, c)
                }
            })))
        }
        "draw-cursor-oob" => {
            let mut h = header();
            h.cursor_row = ROWS as u16 + 5;
            h.cursor_col = COLS as u16 + 5;
            report(draw(&frame(h, good_cell)))
        }
        "draw-ptr" => report(raw(SYS_IOCTL, 1, CONSOLE_DRAW, number(args.next()), 0)),
        "draw-stdin" => report(raw(SYS_IOCTL, 0, CONSOLE_DRAW, 0, 0)),
        _ => ExitCode(2),
    }
}
