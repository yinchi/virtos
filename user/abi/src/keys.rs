//! The keyboard's vocabulary, shared by the kernel (which turns key events into tokens) and, later,
//! programs that read keys directly: the evdev key codes used by name, and `effective_code`, the
//! one place that says what the numeric keypad means.
//!
//! The codes are Linux's `KEY_*` (evdev) numbers, which is what the virtio keyboard reports. Only
//! the ones something names are listed.

pub const KEY_1: u16 = 2;
pub const KEY_2: u16 = 3;
pub const KEY_3: u16 = 4;
pub const KEY_4: u16 = 5;
pub const KEY_5: u16 = 6;
pub const KEY_6: u16 = 7;
pub const KEY_7: u16 = 8;
pub const KEY_8: u16 = 9;
pub const KEY_9: u16 = 10;
pub const KEY_0: u16 = 11;
pub const KEY_MINUS: u16 = 12;
pub const KEY_BACKSPACE: u16 = 14;
pub const KEY_TAB: u16 = 15;
/// Ctrl+W: erase the word before the cursor.
pub const KEY_W: u16 = 17;
/// Ctrl+E: move to the end of the line.
pub const KEY_E: u16 = 18;
/// Ctrl+U: erase from the cursor to the start of the line.
pub const KEY_U: u16 = 22;
/// Ctrl+P: the previous history entry, as Up.
pub const KEY_P: u16 = 25;
pub const KEY_ENTER: u16 = 28;
/// Ctrl+A: move to the start of the line.
pub const KEY_A: u16 = 30;
/// Ctrl+D on an empty line is `read(0)`'s end-of-file.
pub const KEY_D: u16 = 32;
/// Ctrl+F: one character right (Alt+F: one word right).
pub const KEY_F: u16 = 33;
/// Ctrl+H: Backspace.
pub const KEY_H: u16 = 35;
/// Ctrl+K: erase from the cursor to the end of the line.
pub const KEY_K: u16 = 37;
/// Ctrl+B: one character left (Alt+B: one word left).
pub const KEY_B: u16 = 48;
/// Ctrl+N: the next history entry, as Down.
pub const KEY_N: u16 = 49;
pub const KEY_DOT: u16 = 52;
pub const KEY_SLASH: u16 = 53;
pub const KEY_KPASTERISK: u16 = 55;
pub const KEY_SPACE: u16 = 57;
pub const KEY_KP7: u16 = 71;
pub const KEY_KP8: u16 = 72;
pub const KEY_KP9: u16 = 73;
pub const KEY_KPMINUS: u16 = 74;
pub const KEY_KP4: u16 = 75;
pub const KEY_KP5: u16 = 76;
pub const KEY_KP6: u16 = 77;
pub const KEY_KPPLUS: u16 = 78;
pub const KEY_KP1: u16 = 79;
pub const KEY_KP2: u16 = 80;
pub const KEY_KP3: u16 = 81;
pub const KEY_KP0: u16 = 82;
pub const KEY_KPDOT: u16 = 83;
pub const KEY_KPENTER: u16 = 96;
pub const KEY_KPSLASH: u16 = 98;
pub const KEY_HOME: u16 = 102;
pub const KEY_UP: u16 = 103;
pub const KEY_PAGEUP: u16 = 104;
pub const KEY_LEFT: u16 = 105;
pub const KEY_RIGHT: u16 = 106;
pub const KEY_END: u16 = 107;
pub const KEY_DOWN: u16 = 108;
pub const KEY_PAGEDOWN: u16 = 109;
pub const KEY_INSERT: u16 = 110;
pub const KEY_DELETE: u16 = 111;

/// What a key means, with the numeric keypad resolved the way a PC's is. `num` is whether NumLock
/// is on.
///
/// - `KpEnter` is Enter, and the keypad's `/`, `-` are the main block's `KEY_SLASH`, `KEY_MINUS` --
///   always, as on a PC, where the operator keys ignore NumLock. (`*` and `+` have no unshifted
///   main-block twin and keep their own codes; the character they type is `Token::char`'s
///   business.)
/// - With NumLock **on**, the digits and the dot are the main block's (`KEY_0`..`KEY_9`,
///   `KEY_DOT`), so `Alt+Numpad6` is `Alt+6`, as in a terminal, where the keypad just sends the
///   character.
/// - With NumLock **off**, they are navigation: 7 Home, 8 Up, 9 PgUp, 4 Left, 6 Right, 1 End,
///   2 Down, 3 PgDn, 0 Insert, dot Delete. `Kp5` has no navigation meaning and stays `KEY_KP5`.
///
/// Every other code is returned unchanged. Shift is not consulted: a keypad key is the same key
/// shifted.
pub const fn effective_code(code: u16, num: bool) -> u16 {
    // (keypad code, NumLock-on meaning, NumLock-off meaning)
    const KEYPAD: [(u16, u16, u16); 11] = [
        (KEY_KP7, KEY_7, KEY_HOME),
        (KEY_KP8, KEY_8, KEY_UP),
        (KEY_KP9, KEY_9, KEY_PAGEUP),
        (KEY_KP4, KEY_4, KEY_LEFT),
        (KEY_KP5, KEY_5, KEY_KP5),
        (KEY_KP6, KEY_6, KEY_RIGHT),
        (KEY_KP1, KEY_1, KEY_END),
        (KEY_KP2, KEY_2, KEY_DOWN),
        (KEY_KP3, KEY_3, KEY_PAGEDOWN),
        (KEY_KP0, KEY_0, KEY_INSERT),
        (KEY_KPDOT, KEY_DOT, KEY_DELETE),
    ];
    match code {
        KEY_KPENTER => return KEY_ENTER,
        KEY_KPSLASH => return KEY_SLASH,
        KEY_KPMINUS => return KEY_MINUS,
        _ => {}
    }
    let mut i = 0;
    while i < KEYPAD.len() {
        if KEYPAD[i].0 == code {
            return if num { KEYPAD[i].1 } else { KEYPAD[i].2 };
        }
        i += 1;
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_operator_keys_and_enter_ignore_numlock() {
        for num in [true, false] {
            assert_eq!(effective_code(KEY_KPENTER, num), KEY_ENTER);
            assert_eq!(effective_code(KEY_KPSLASH, num), KEY_SLASH);
            assert_eq!(effective_code(KEY_KPMINUS, num), KEY_MINUS);
        }
    }

    #[test]
    fn numlock_on_makes_the_digits_and_dot_the_main_blocks() {
        assert_eq!(effective_code(KEY_KP7, true), KEY_7);
        assert_eq!(effective_code(KEY_KP0, true), KEY_0);
        assert_eq!(effective_code(KEY_KPDOT, true), KEY_DOT);
    }

    #[test]
    fn numlock_off_makes_them_navigation() {
        assert_eq!(effective_code(KEY_KP7, false), KEY_HOME);
        assert_eq!(effective_code(KEY_KP8, false), KEY_UP);
        assert_eq!(effective_code(KEY_KP9, false), KEY_PAGEUP);
        assert_eq!(effective_code(KEY_KP4, false), KEY_LEFT);
        assert_eq!(effective_code(KEY_KP6, false), KEY_RIGHT);
        assert_eq!(effective_code(KEY_KP1, false), KEY_END);
        assert_eq!(effective_code(KEY_KP2, false), KEY_DOWN);
        assert_eq!(effective_code(KEY_KP3, false), KEY_PAGEDOWN);
        assert_eq!(effective_code(KEY_KP0, false), KEY_INSERT);
        assert_eq!(effective_code(KEY_KPDOT, false), KEY_DELETE);
        assert_eq!(effective_code(KEY_KP5, false), KEY_KP5);
    }

    #[test]
    fn other_codes_pass_through() {
        assert_eq!(effective_code(KEY_A, true), KEY_A);
        assert_eq!(effective_code(KEY_LEFT, false), KEY_LEFT);
        assert_eq!(effective_code(KEY_KPASTERISK, false), KEY_KPASTERISK);
    }
}
