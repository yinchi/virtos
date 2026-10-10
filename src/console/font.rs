//! Which glyph draws a character, and how wide it is: GNU Unifont's, via the `unifont` crate
//! (compiled into the kernel, no disk or MMIO access needed). 8x16 for most characters, 16x16 --
//! two cells -- for the wide ones (CJK and friends). The crate covers the Basic Multilingual
//! Plane only; anything else draws U+FFFD.
//!
//! Pure apart from the `unifont` crate itself, so it builds and runs identically under
//! `cargo test --target <host triple>` (`just test-host`) as it does on the real target.

use unifont::Glyph;

/// The width of a one-cell glyph in pixels; a wide glyph is two cells, `2 * GLYPH_WIDTH`.
pub const GLYPH_WIDTH: usize = 8;
/// The height of every glyph in pixels.
pub const GLYPH_HEIGHT: usize = 16;

const REPLACEMENT: char = '\u{FFFD}';

/// Code points that take no cell and draw nothing: zero-width space, joiners and directional marks,
/// the word joiner, variation selectors, and the byte-order mark.
pub fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{2060}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}')
}

/// The glyph that draws `c` -- Unifont's, or U+FFFD's if the font has none (anything outside the
/// Basic Multilingual Plane, or unassigned) or `c` is a control character (`console` interprets
/// the few it uses before ever asking for a glyph).
pub fn glyph_for(c: char) -> &'static Glyph {
    let found = if c.is_control() {
        None
    } else {
        unifont::get_glyph(c)
    };
    found
        .unwrap_or_else(|| unifont::get_glyph(REPLACEMENT).expect("Unifont has a glyph for U+FFFD"))
}

/// How many cells `c` takes when written: 0 (`is_zero_width`), 2 (a wide glyph), or 1. The four
/// characters the console interprets rather than draws (newline, carriage return, tab, backspace)
/// count as 1, like everything else the input layout (`input_layout.rs`) may be asked to lay out.
pub fn cell_width(c: char) -> usize {
    if is_zero_width(c) {
        0
    } else if glyph_for(c).is_fullwidth() && !matches!(c, '\n' | '\r' | '\t' | '\u{8}') {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn characters_the_font_lacks_draw_the_replacement_glyph() {
        let replacement = glyph_for('\u{FFFD}');
        for c in ['😀', '\u{10000}', '\u{0}', '\u{1B}', '\u{7F}'] {
            assert_eq!(glyph_for(c), replacement, "{c:?}");
        }
    }

    #[test]
    fn ordinary_characters_have_their_own_distinct_glyphs() {
        let replacement = glyph_for('\u{FFFD}');
        for c in ['a', 'Z', '0', ' ', '!'] {
            assert_ne!(glyph_for(c), replacement, "{c:?}");
        }
        assert_ne!(glyph_for('a'), glyph_for('b'));
    }

    #[test]
    fn ascii_is_one_cell_and_cjk_is_two() {
        assert_eq!(cell_width('a'), 1);
        assert_eq!(cell_width(' '), 1);
        assert_eq!(cell_width('~'), 1);
        assert_eq!(cell_width('é'), 1);
        assert_eq!(cell_width('日'), 2);
        assert_eq!(cell_width('한'), 2);
        assert_eq!(cell_width('Ａ'), 2); // fullwidth Latin
    }

    #[test]
    fn the_zero_width_set_takes_no_cell() {
        for c in [
            '\u{200B}', '\u{200D}', '\u{200F}', '\u{2060}', '\u{FE0F}', '\u{FEFF}',
        ] {
            assert!(is_zero_width(c), "{c:?}");
            assert_eq!(cell_width(c), 0);
        }
        assert!(!is_zero_width('a'));
        assert!(!is_zero_width('\u{FFFD}'));
    }

    #[test]
    fn the_interpreted_controls_count_as_one_cell() {
        for c in ['\n', '\r', '\t', '\u{8}'] {
            assert_eq!(cell_width(c), 1);
        }
    }
}
