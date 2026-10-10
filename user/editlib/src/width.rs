//! How many screen columns a character takes, so the editor's line-wrapping agrees with what
//! `CONSOLE_DRAW` actually puts on screen: 0 for a zero-width mark, 2 for a wide (East Asian
//! fullwidth) glyph, 1 otherwise. This is the same logic as the kernel console's `cell_width`, kept as
//! a second copy here rather than shared, since a user program cannot depend on the kernel crate
//! -- `unifont` (a public crate, not kernel-internal) is what actually has to agree, and both
//! sides depend on it directly. Unlike the console's version, this one is never asked about `\n`,
//! `\r`, `\t` or `\u{8}`: a [`crate::buffer::Buffer`] never puts a control character in a cell (a
//! tab is expanded to spaces' worth of columns before layout sees it, and a newline is a line
//! break, never a character within a line) -- see `abi::ioctl::Cell`'s doc comment for the same
//! rule on the kernel side.
//!
//! Pure, so it is tested on the host (`cargo test`).

use unifont::Glyph;

/// Code points that take no column and draw nothing: zero-width space, joiners and directional
/// marks, the word joiner, variation selectors, and the byte-order mark. The same set
/// the kernel console's `font.rs` uses.
pub fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{2060}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}')
}

/// The glyph Unifont draws `c` with, or `None` for a code point the font doesn't cover (everything
/// above U+FFFF, or unassigned) -- mirrors the kernel console's `glyph_for` fallback to U+FFFD's
/// glyph without needing it here: only [`Glyph::is_fullwidth`] is asked of it, and the replacement
/// glyph (a box shape) is narrow, the same as most of what it stands in for.
fn glyph_for(c: char) -> Option<&'static Glyph> {
    if c.is_control() {
        None
    } else {
        unifont::get_glyph(c)
    }
}

/// How many screen columns `c` takes: 0, 1, or 2.
pub fn cell_width(c: char) -> usize {
    if is_zero_width(c) {
        0
    } else if glyph_for(c).is_some_and(Glyph::is_fullwidth) {
        2
    } else {
        1
    }
}

/// The screen columns `s` takes end to end -- the sum of [`cell_width`] over its characters.
pub fn text_width(s: &str) -> usize {
    s.chars().map(cell_width).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_one_column_and_cjk_is_two() {
        assert_eq!(cell_width('a'), 1);
        assert_eq!(cell_width(' '), 1);
        assert_eq!(cell_width('~'), 1);
        assert_eq!(cell_width('é'), 1);
        assert_eq!(cell_width('日'), 2);
        assert_eq!(cell_width('한'), 2);
        assert_eq!(cell_width('Ａ'), 2); // fullwidth Latin
    }

    #[test]
    fn the_zero_width_set_takes_no_column() {
        for c in [
            '\u{200B}', '\u{200D}', '\u{200F}', '\u{2060}', '\u{FE0F}', '\u{FEFF}',
        ] {
            assert!(is_zero_width(c), "{c:?}");
            assert_eq!(cell_width(c), 0);
        }
        assert!(!is_zero_width('a'));
    }

    #[test]
    fn a_character_the_font_lacks_is_narrow() {
        // Unlike the console, this module never falls back to U+FFFD's own glyph -- it just treats
        // an uncovered code point as width 1 (the replacement glyph is narrow too, so the two
        // functions still agree on the number that matters).
        for c in ['😀', '\u{10000}', '\u{10FFFF}'] {
            assert_eq!(cell_width(c), 1, "{c:?}");
        }
    }

    #[test]
    fn text_width_sums_its_characters() {
        assert_eq!(text_width(""), 0);
        assert_eq!(text_width("abc"), 3);
        assert_eq!(text_width("日本語"), 6);
        assert_eq!(text_width("a日b"), 4);
        assert_eq!(text_width("a\u{200B}b"), 2);
    }
}
