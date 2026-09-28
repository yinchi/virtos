//! Turns raw VirtIO keyboard events (Linux evdev `KEY_*` codes and press/release/repeat values --
//! confirmed against QEMU's `virtio-keyboard-device`, which reports codes in that numbering, not
//! raw PS/2 scancodes or USB HID usage IDs) into ASCII characters.
//!
//! Deliberately smaller than the lab's equivalent: no name table or key-state display (nothing
//! here needs to show "which keys are held"), no NumLock/keypad handling, no per-key held-state
//! array, and no wire encoding for a userspace `KeyEvent` -- there's no userspace yet. Just enough
//! to type a line of text: Shift and CapsLock (letter case is shift XOR caps, exactly as on a real
//! keyboard), a base-character table, and a shifted-symbol table for the keys whose shifted form
//! isn't just uppercase.
//!
//! Pure -- no dependency on `virtio-drivers` or any other hardware crate, so it builds and runs
//! under `cargo test --target <host triple>` (`just test-host`) exactly as it does on target.

const KEY_LSHIFT: u16 = 42;
const KEY_RSHIFT: u16 = 54;
const KEY_CAPSLOCK: u16 = 58;
const KEY_ENTER: u16 = 28;
const KEY_BACKSPACE: u16 = 14;
const KEY_TAB: u16 = 15;
const KEY_SPACE: u16 = 57;

/// `(code, unshifted char)` for every letter, digit and punctuation key. Letters are stored
/// lowercase; `translate` derives the actual case from Shift/CapsLock.
const BASE: &[(u16, char)] = &[
    (2, '1'),
    (3, '2'),
    (4, '3'),
    (5, '4'),
    (6, '5'),
    (7, '6'),
    (8, '7'),
    (9, '8'),
    (10, '9'),
    (11, '0'),
    (12, '-'),
    (13, '='),
    (16, 'q'),
    (17, 'w'),
    (18, 'e'),
    (19, 'r'),
    (20, 't'),
    (21, 'y'),
    (22, 'u'),
    (23, 'i'),
    (24, 'o'),
    (25, 'p'),
    (26, '['),
    (27, ']'),
    (30, 'a'),
    (31, 's'),
    (32, 'd'),
    (33, 'f'),
    (34, 'g'),
    (35, 'h'),
    (36, 'j'),
    (37, 'k'),
    (38, 'l'),
    (39, ';'),
    (40, '\''),
    (41, '`'),
    (43, '\\'),
    (44, 'z'),
    (45, 'x'),
    (46, 'c'),
    (47, 'v'),
    (48, 'b'),
    (49, 'n'),
    (50, 'm'),
    (51, ','),
    (52, '.'),
    (53, '/'),
];

/// Shifted variant for keys whose `BASE` character isn't a letter -- the number row and
/// punctuation, whose Shift form isn't just "uppercase".
const SHIFTED: &[(u16, char)] = &[
    (2, '!'),
    (3, '@'),
    (4, '#'),
    (5, '$'),
    (6, '%'),
    (7, '^'),
    (8, '&'),
    (9, '*'),
    (10, '('),
    (11, ')'),
    (12, '_'),
    (13, '+'),
    (26, '{'),
    (27, '}'),
    (39, ':'),
    (40, '"'),
    (41, '~'),
    (43, '|'),
    (51, '<'),
    (52, '>'),
    (53, '?'),
];

/// The character `code` types, given whether Shift is held and CapsLock is on -- `None` for keys
/// with no character (function keys, arrows, modifiers themselves, ...) or that this layer
/// doesn't resolve yet (the numeric keypad).
pub fn translate(code: u16, shift: bool, caps: bool) -> Option<char> {
    match code {
        KEY_ENTER => return Some('\n'),
        KEY_BACKSPACE => return Some('\x08'),
        KEY_TAB => return Some('\t'),
        KEY_SPACE => return Some(' '),
        _ => {}
    }
    let base = BASE.iter().find(|&&(c, _)| c == code)?.1;
    Some(if base.is_ascii_alphabetic() {
        if shift ^ caps {
            base.to_ascii_uppercase()
        } else {
            base
        }
    } else if shift {
        SHIFTED
            .iter()
            .find(|&&(c, _)| c == code)
            .map_or(base, |&(_, hi)| hi)
    } else {
        base
    })
}

/// Tracks the two modifiers `translate` needs: Shift as currently held, CapsLock as currently
/// toggled on.
pub struct KeyState {
    shift_held: bool,
    caps_lock: bool,
}

impl KeyState {
    pub const fn new() -> Self {
        Self {
            shift_held: false,
            caps_lock: false,
        }
    }

    /// Applies one raw event (`code`, evdev `value`: 0 release, 1 press, 2 auto-repeat) and
    /// returns the character it types, if any. Shift/CapsLock update this state and never
    /// produce a character themselves; CapsLock flips only on a genuine press (`value == 1`), not
    /// on release or auto-repeat; every other key ignores release (`value == 0`) the same way.
    pub fn apply(&mut self, code: u16, value: u32) -> Option<char> {
        match code {
            KEY_LSHIFT | KEY_RSHIFT => {
                self.shift_held = value != 0;
                return None;
            }
            KEY_CAPSLOCK => {
                if value == 1 {
                    self.caps_lock = !self.caps_lock;
                }
                return None;
            }
            _ => {}
        }
        if value == 0 {
            return None;
        }
        translate(code, self.shift_held, self.caps_lock)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: u16 = 30;
    const KEY_2: u16 = 3;

    #[test]
    fn letters_follow_shift_xor_caps() {
        assert_eq!(translate(KEY_A, false, false), Some('a'));
        assert_eq!(translate(KEY_A, true, false), Some('A'));
        assert_eq!(translate(KEY_A, false, true), Some('A'));
        assert_eq!(translate(KEY_A, true, true), Some('a'));
    }

    #[test]
    fn shift_resolves_the_shifted_symbol_table_not_a_case_change() {
        assert_eq!(translate(KEY_2, false, false), Some('2'));
        assert_eq!(translate(KEY_2, true, false), Some('@'));
        // CapsLock alone (no Shift) never affects a non-letter key.
        assert_eq!(translate(KEY_2, false, true), Some('2'));
    }

    #[test]
    fn control_keys_resolve_without_needing_the_base_table() {
        assert_eq!(translate(KEY_ENTER, false, false), Some('\n'));
        assert_eq!(translate(KEY_BACKSPACE, true, true), Some('\x08'));
        assert_eq!(translate(KEY_TAB, false, false), Some('\t'));
        assert_eq!(translate(KEY_SPACE, true, false), Some(' '));
    }

    #[test]
    fn an_unmapped_key_has_no_character() {
        assert_eq!(translate(59, false, false), None); // F1
    }

    #[test]
    fn shift_keys_update_state_and_produce_no_character() {
        let mut keys = KeyState::new();
        assert_eq!(keys.apply(KEY_LSHIFT, 1), None);
        assert_eq!(keys.apply(KEY_A, 1), Some('A'));
        assert_eq!(keys.apply(KEY_LSHIFT, 0), None);
        assert_eq!(keys.apply(KEY_A, 1), Some('a'));
    }

    #[test]
    fn capslock_toggles_only_on_a_genuine_press() {
        let mut keys = KeyState::new();
        assert_eq!(keys.apply(KEY_CAPSLOCK, 1), None);
        assert_eq!(keys.apply(KEY_A, 1), Some('A'));
        // Auto-repeat and release of CapsLock itself must not flip it again.
        keys.apply(KEY_CAPSLOCK, 2);
        keys.apply(KEY_CAPSLOCK, 0);
        assert_eq!(keys.apply(KEY_A, 1), Some('A'));
    }

    #[test]
    fn release_of_an_ordinary_key_produces_no_character() {
        let mut keys = KeyState::new();
        assert_eq!(keys.apply(KEY_A, 0), None);
    }

    #[test]
    fn auto_repeat_of_an_ordinary_key_still_types() {
        let mut keys = KeyState::new();
        assert_eq!(keys.apply(KEY_A, 2), Some('a'));
    }
}
