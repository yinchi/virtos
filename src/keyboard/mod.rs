//! Turns raw VirtIO keyboard events (Linux evdev `KEY_*` codes and press/release/repeat values --
//! confirmed against QEMU's `virtio-keyboard-device`, which reports codes in that numbering, not
//! raw PS/2 scancodes or USB HID usage IDs) into `Token`s.
//!
//! Deliberately smaller than the lab's equivalent: no name table or key-state display, and no wire
//! encoding for a userspace `KeyEvent` (nothing reads keys directly yet). It does track Shift/
//! Ctrl/Alt, CapsLock/NumLock, and which keys are currently held -- the last so a held key's
//! resent press can be told apart from a fresh one. A character table covers the main block;
//! the numeric keypad is resolved via `abi::keys::effective_code`.
//!
//! `KeyState::apply` yields a `Token` -- *which key* was pressed, with the modifiers in effect at
//! that moment -- rather than a bare character, so a consumer can tell Ctrl+D from `d`: what a
//! chord *means* is decided by whoever reads it (`line`'s `LineBuffer`).
//!
//! Pure -- no dependency on `virtio-drivers` or any other hardware crate, so it builds and runs
//! under `cargo test --target <host triple>` (`just test-host`) exactly as it does on target.
//!
//! `queue` (hardware-only, unlike this module) holds the tokens this layer produces between the
//! keyboard IRQ and whoever reads them.
pub mod history;
pub mod line;
#[cfg(target_os = "none")]
pub mod line_discipline;
#[cfg(target_os = "none")]
pub mod queue;
#[cfg(target_os = "none")]
pub mod stdin;

use abi::keys::{
    KEY_BACKSPACE, KEY_ENTER, KEY_KP0, KEY_KP1, KEY_KP2, KEY_KP3, KEY_KP4, KEY_KP5, KEY_KP6,
    KEY_KP7, KEY_KP8, KEY_KP9, KEY_KPASTERISK, KEY_KPDOT, KEY_KPMINUS, KEY_KPPLUS, KEY_KPSLASH,
    KEY_SPACE, KEY_TAB, effective_code,
};

const KEY_LSHIFT: u16 = 42;
const KEY_RSHIFT: u16 = 54;
const KEY_CAPSLOCK: u16 = 58;
const KEY_NUMLOCK: u16 = 69;
const KEY_LCTRL: u16 = 29;
const KEY_RCTRL: u16 = 97;
const KEY_LALT: u16 = 56;
const KEY_RALT: u16 = 100;

/// Size of the held-key set: evdev's own ceiling on `EV_KEY` codes (`KEY_MAX`, 0x2ff) plus one --
/// shared with mouse/joystick button codes, which is why it is so much higher than any real
/// keyboard key.
const MAX_CODE: usize = 0x300;

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

/// One key press and the modifiers in effect when it happened. `code` is the raw evdev code, so a
/// consumer can bind a chord (`ctrl` + `KEY_D`) as well as a plain character; match on
/// `effective_code()` to treat the keypad as what it is labelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub code: u16,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// CapsLock's toggle state, not a held key -- captured here so a `Token` is a self-contained
    /// snapshot, like `shift`/`ctrl`/`alt`.
    pub caps: bool,
    /// NumLock's toggle state, captured the same way: it decides what a keypad key is.
    pub num: bool,
    /// The key was already down: the keyboard's own auto-repeat, not a fresh press. A consumer
    /// that must act once per press ignores these; one that acts per keystroke (typing, moving)
    /// takes them.
    pub repeat: bool,
}

/// What a keypad key types: `* - + /` always, as on a PC, and the digits and the dot only with
/// NumLock on (with it off they navigate -- see `abi::keys::effective_code`). Shift is not
/// consulted.
fn keypad_char(code: u16, num: bool) -> Option<char> {
    match code {
        KEY_KPASTERISK => Some('*'),
        KEY_KPMINUS => Some('-'),
        KEY_KPPLUS => Some('+'),
        KEY_KPSLASH => Some('/'),
        _ if !num => None,
        KEY_KP0 => Some('0'),
        KEY_KP1 => Some('1'),
        KEY_KP2 => Some('2'),
        KEY_KP3 => Some('3'),
        KEY_KP4 => Some('4'),
        KEY_KP5 => Some('5'),
        KEY_KP6 => Some('6'),
        KEY_KP7 => Some('7'),
        KEY_KP8 => Some('8'),
        KEY_KP9 => Some('9'),
        KEY_KPDOT => Some('.'),
        _ => None,
    }
}

/// Whether a `repeat` token may enter a queue that already holds `queued` tokens: it may not. A
/// held key repeats faster than a slow consumer can take, and every repeat that waited in the
/// queue would keep moving the cursor after the key was released -- so a repeat is dropped unless
/// the queue has been drained. A fresh press always enters.
pub fn admits(token: &Token, queued: usize) -> bool {
    !token.repeat || queued == 0
}

impl Token {
    /// The key as a consumer means it, with the numeric keypad resolved by NumLock; `code` stays
    /// the raw evdev code.
    pub fn effective_code(&self) -> u16 {
        effective_code(self.code, self.num)
    }

    /// The character this press types, if any: `None` for a key with no character, and for any
    /// Ctrl/Alt chord -- those are shortcuts, never text.
    pub fn char(&self) -> Option<char> {
        if self.ctrl || self.alt {
            return None;
        }
        keypad_char(self.code, self.num).or_else(|| translate(self.code, self.shift, self.caps))
    }
}

/// Tracks the modifiers a `Token` records -- Shift/Ctrl/Alt as currently held, CapsLock/NumLock as
/// currently toggled on -- and which keys are held at all, to recognise a repeat.
pub struct KeyState {
    shift_held: bool,
    ctrl_held: bool,
    alt_held: bool,
    caps_lock: bool,
    /// NumLock starts **on**, as on a PC.
    num_lock: bool,
    held: [bool; MAX_CODE],
}

impl KeyState {
    pub const fn new() -> Self {
        Self {
            shift_held: false,
            ctrl_held: false,
            alt_held: false,
            caps_lock: false,
            num_lock: true,
            held: [false; MAX_CODE],
        }
    }

    /// Applies one raw event (`code`, evdev `value`: 0 release, 1 press, 2 auto-repeat) and
    /// returns the `Token` it produces, if any.
    ///
    /// A press of a key that is *already held* is the keyboard's own auto-repeat, whether the
    /// transport tags it `value == 2` or (as QEMU's virtio-keyboard does) just resends a plain
    /// `value == 1` -- so "was it held" is asked before the held set is updated, and a repeat is a
    /// token with `repeat` set rather than being dropped. Modifiers update this state and never
    /// produce a token; a lock key flips only on a *fresh* press (a resent one must not toggle it
    /// again) and never produces one either. A release produces nothing. Every other press yields
    /// a token, even one with no character (an arrow, F1) -- the consumer ignores what it doesn't
    /// bind.
    pub fn apply(&mut self, code: u16, value: u32) -> Option<Token> {
        let down = value != 0;
        let slot = self.held.get_mut(code as usize);
        let was_held = slot.as_deref().copied().unwrap_or(false);
        if let Some(slot) = slot {
            *slot = down;
        }
        let fresh = down && !was_held;
        match code {
            KEY_LSHIFT | KEY_RSHIFT => {
                self.shift_held = down;
                return None;
            }
            KEY_LCTRL | KEY_RCTRL => {
                self.ctrl_held = down;
                return None;
            }
            KEY_LALT | KEY_RALT => {
                self.alt_held = down;
                return None;
            }
            KEY_CAPSLOCK => {
                if fresh {
                    self.caps_lock = !self.caps_lock;
                }
                return None;
            }
            KEY_NUMLOCK => {
                if fresh {
                    self.num_lock = !self.num_lock;
                }
                return None;
            }
            _ => {}
        }
        if !down {
            return None;
        }
        Some(Token {
            code,
            shift: self.shift_held,
            ctrl: self.ctrl_held,
            alt: self.alt_held,
            caps: self.caps_lock,
            num: self.num_lock,
            repeat: was_held,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: u16 = 30;
    const KEY_2: u16 = 3;
    const KEY_D: u16 = abi::keys::KEY_D;

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

    /// The character `apply` types for `code`/`value`, if any.
    fn typed(keys: &mut KeyState, code: u16, value: u32) -> Option<char> {
        keys.apply(code, value).and_then(|t| t.char())
    }

    #[test]
    fn shift_keys_update_state_and_produce_no_token() {
        let mut keys = KeyState::new();
        assert_eq!(keys.apply(KEY_LSHIFT, 1), None);
        assert_eq!(typed(&mut keys, KEY_A, 1), Some('A'));
        assert_eq!(keys.apply(KEY_LSHIFT, 0), None);
        assert_eq!(typed(&mut keys, KEY_A, 1), Some('a'));
    }

    #[test]
    fn capslock_toggles_only_on_a_genuine_press() {
        let mut keys = KeyState::new();
        assert_eq!(keys.apply(KEY_CAPSLOCK, 1), None);
        assert_eq!(typed(&mut keys, KEY_A, 1), Some('A'));
        // Auto-repeat and release of CapsLock itself must not flip it again.
        keys.apply(KEY_CAPSLOCK, 2);
        keys.apply(KEY_CAPSLOCK, 0);
        assert_eq!(typed(&mut keys, KEY_A, 1), Some('A'));
    }

    #[test]
    fn release_of_an_ordinary_key_produces_no_token() {
        let mut keys = KeyState::new();
        assert_eq!(keys.apply(KEY_A, 0), None);
    }

    #[test]
    fn auto_repeat_of_an_ordinary_key_still_types() {
        let mut keys = KeyState::new();
        assert_eq!(typed(&mut keys, KEY_A, 2), Some('a'));
    }

    #[test]
    fn ctrl_is_recorded_on_the_token_and_suppresses_its_character() {
        let mut keys = KeyState::new();
        assert_eq!(keys.apply(KEY_LCTRL, 1), None);
        let token = keys.apply(KEY_D, 1).expect("Ctrl+D is a token");
        assert_eq!((token.code, token.ctrl, token.alt), (KEY_D, true, false));
        assert_eq!(token.char(), None);
        keys.apply(KEY_LCTRL, 0);
        assert_eq!(typed(&mut keys, KEY_D, 1), Some('d'));
    }

    #[test]
    fn alt_is_recorded_and_either_ctrl_key_counts() {
        let mut keys = KeyState::new();
        keys.apply(KEY_RALT, 1);
        let token = keys.apply(KEY_A, 1).unwrap();
        assert!(token.alt && !token.ctrl);
        assert_eq!(token.char(), None);
        keys.apply(KEY_RALT, 0);
        keys.apply(KEY_RCTRL, 1);
        assert!(keys.apply(KEY_A, 1).unwrap().ctrl);
    }

    #[test]
    fn a_key_with_no_character_is_still_a_token() {
        let mut keys = KeyState::new();
        let token = keys.apply(59, 1).expect("F1 is a token"); // F1
        assert_eq!(token.char(), None);
    }

    #[test]
    fn a_second_press_of_a_held_key_is_a_repeat_not_a_fresh_press() {
        let mut keys = KeyState::new();
        assert!(!keys.apply(KEY_A, 1).unwrap().repeat);
        // QEMU resends a plain press (value 1) for a held key, with no distinct repeat tag.
        assert!(keys.apply(KEY_A, 1).unwrap().repeat);
        assert!(keys.apply(KEY_A, 2).unwrap().repeat);
        // Released, then pressed again: fresh.
        assert_eq!(keys.apply(KEY_A, 0), None);
        assert!(!keys.apply(KEY_A, 1).unwrap().repeat);
    }

    #[test]
    fn a_resent_lock_key_press_does_not_toggle_it_again() {
        let mut keys = KeyState::new();
        keys.apply(KEY_CAPSLOCK, 1);
        keys.apply(KEY_CAPSLOCK, 1); // resent while held
        assert!(keys.apply(KEY_A, 1).unwrap().caps);
        keys.apply(KEY_CAPSLOCK, 0);
        keys.apply(KEY_CAPSLOCK, 1); // a fresh press: off again
        assert!(!keys.apply(KEY_2, 1).unwrap().caps);
    }

    #[test]
    fn numlock_starts_on_and_toggles_on_a_fresh_press() {
        let mut keys = KeyState::new();
        assert!(keys.apply(KEY_A, 1).unwrap().num);
        keys.apply(KEY_A, 0);
        assert_eq!(keys.apply(KEY_NUMLOCK, 1), None);
        keys.apply(KEY_NUMLOCK, 0);
        assert!(!keys.apply(KEY_A, 1).unwrap().num);
    }

    #[test]
    fn keypad_types_digits_only_with_numlock_but_operators_always() {
        let mut keys = KeyState::new();
        assert_eq!(typed(&mut keys, KEY_KP7, 1), Some('7'));
        assert_eq!(typed(&mut keys, KEY_KPDOT, 1), Some('.'));
        keys.apply(KEY_NUMLOCK, 1);
        keys.apply(KEY_NUMLOCK, 0);
        assert_eq!(typed(&mut keys, KEY_KP7, 1), None);
        assert_eq!(typed(&mut keys, KEY_KPPLUS, 1), Some('+'));
        assert_eq!(typed(&mut keys, KEY_KPSLASH, 1), Some('/'));
    }

    #[test]
    fn effective_code_follows_numlock() {
        let mut keys = KeyState::new();
        let on = keys.apply(KEY_KP4, 1).unwrap();
        assert_eq!(on.effective_code(), abi::keys::KEY_4);
        keys.apply(KEY_KP4, 0);
        keys.apply(KEY_NUMLOCK, 1);
        keys.apply(KEY_NUMLOCK, 0);
        let off = keys.apply(KEY_KP4, 1).unwrap();
        assert_eq!(off.effective_code(), abi::keys::KEY_LEFT);
    }

    #[test]
    fn a_repeat_is_admitted_only_into_an_empty_queue() {
        let mut keys = KeyState::new();
        let fresh = keys.apply(KEY_A, 1).unwrap();
        let repeat = keys.apply(KEY_A, 1).unwrap();
        assert!(admits(&fresh, 5));
        assert!(admits(&repeat, 0));
        assert!(!admits(&repeat, 1));
    }
}
