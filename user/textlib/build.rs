//! Generates the table of code points the console draws two columns wide, from the very `unifont`
//! data the kernel's console renders with -- so a program that lays text out in columns agrees
//! with the display by construction, without linking the ~2 MB of glyph bitmaps into every
//! program that only needs to know a width. Written to `$OUT_DIR/wide.rs` and included by
//! `src/width.rs`.

use std::fmt::Write as _;
use std::{env, fs, path::Path};

/// Code points that take no column and draw nothing -- must match `src/width.rs`'s own
/// `is_zero_width` (those are excluded from the table, so the two cannot disagree on them).
fn zero_width(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{2060}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}')
}

fn main() {
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    for cp in 0..=0x10_FFFFu32 {
        let Some(c) = char::from_u32(cp) else {
            continue;
        };
        // A control character has no glyph here (the console draws U+FFFD for one, which is
        // narrow), and a code point the font does not cover is narrow too.
        if zero_width(c) || c.is_control() {
            continue;
        }
        if unifont::get_glyph(c).is_some_and(|glyph| glyph.is_fullwidth()) {
            match ranges.last_mut() {
                Some(range) if range.1 + 1 == cp => range.1 = cp,
                _ => ranges.push((cp, cp)),
            }
        }
    }

    let mut out = String::from(
        "/// Inclusive `(first, last)` runs of code points drawn two columns wide (generated).\n\
         pub(crate) const WIDE: &[(u32, u32)] = &[\n",
    );
    for (first, last) in &ranges {
        writeln!(out, "    ({first:#x}, {last:#x}),").unwrap();
    }
    out.push_str("];\n");
    let path = Path::new(&env::var("OUT_DIR").unwrap()).join("wide.rs");
    fs::write(path, out).unwrap();
    println!("cargo:rerun-if-changed=build.rs");
}
