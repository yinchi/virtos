//! How many screen columns a character takes, so a program that lays text out in columns (`ls`,
//! `column`, the editor's line wrapping) agrees with what the console actually draws: 0 for a
//! zero-width mark, 2 for a wide (East Asian fullwidth) glyph, 1 otherwise.
//!
//! The set of wide code points is generated at build time from the same `unifont` data the
//! kernel's console renders with (`build.rs`), so the two cannot drift apart, and it costs a few
//! kilobytes instead of the font's bitmaps. A control character or a code point the font does not
//! cover is narrow -- the console draws U+FFFD for those, which is narrow too.
//!
//! A tab is not a character with a width of its own: how far it reaches depends on the column it
//! starts at, so the caller that expands tabs does that (the editor's `layout`), and a name or
//! table cell never holds one.

include!(concat!(env!("OUT_DIR"), "/wide.rs"));

/// Code points that take no column and draw nothing: zero-width space, joiners and directional
/// marks, the word joiner, variation selectors, and the byte-order mark. The same set the kernel
/// console's `font.rs` uses.
pub fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{2060}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}')
}

fn is_wide(c: char) -> bool {
    let cp = c as u32;
    WIDE.binary_search_by(|&(first, last)| {
        if cp < first {
            core::cmp::Ordering::Greater
        } else if cp > last {
            core::cmp::Ordering::Less
        } else {
            core::cmp::Ordering::Equal
        }
    })
    .is_ok()
}

/// How many screen columns `c` takes: 0, 1, or 2.
pub fn cell_width(c: char) -> usize {
    if is_zero_width(c) {
        0
    } else if is_wide(c) {
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
        for c in ['a', ' ', '~', 'é'] {
            assert_eq!(cell_width(c), 1, "{c:?}");
        }
        for c in ['日', '한', 'Ａ'] {
            assert_eq!(cell_width(c), 2, "{c:?}"); // CJK, Hangul, fullwidth Latin
        }
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
    fn controls_and_uncovered_code_points_are_narrow() {
        for c in ['\t', '\n', '\u{7f}', '😀', '\u{10000}', '\u{10FFFF}'] {
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

    #[test]
    fn the_generated_table_is_sorted_and_disjoint() {
        for pair in WIDE.windows(2) {
            assert!(pair[0].1 < pair[1].0, "{pair:?}");
        }
        assert!(WIDE.iter().all(|&(first, last)| first <= last));
        assert!(!WIDE.is_empty());
    }
}
