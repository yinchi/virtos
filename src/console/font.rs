//! Which glyph draws a character: GNU Unifont's, via the `unifont` crate (compiled into the
//! kernel, no disk or MMIO access needed). Only the halfwidth (8x16, Latin/ASCII) glyphs are
//! used for now -- nothing upstream of this ever produces anything else (see `keyboard`, which
//! only emits ASCII), so wide-glyph handling is left for whenever something actually needs to
//! draw non-Latin text.
//!
//! Pure apart from the `unifont` crate itself, so it builds and runs identically under
//! `cargo test --target <host triple>` (`just test-host`) as it does on the real target.

use unifont::Glyph;

/// The width of a glyph cell in pixels.
pub const GLYPH_WIDTH: usize = 8;
/// The height of a glyph cell in pixels.
pub const GLYPH_HEIGHT: usize = 16;

const REPLACEMENT: char = '\u{FFFD}';

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
}
