//! `Quake/in_sdl.h` -- `IN_SDL_ScancodeToQuakeKey`, keyed on the raw
//! scancode integer. SDL2 and SDL3 share the USB-HID scancode numbering, so
//! one table serves both backends; the unit tests below pin every literal to
//! the `sdl2-sys`/`sdl3-sys` constant it stands for.

use core::ffi::c_int;

// the K_* table mirrors keys.h
#[allow(clippy::wildcard_imports)]
use super::keys::*;

// SDL_SCANCODE_* (SDL_scancode.h). Only the codes the C table names.
const SC_A: c_int = 4;
const SC_Z: c_int = 29;
const SC_1: c_int = 30;
const SC_9: c_int = 38;
const SC_0: c_int = 39;
const SC_RETURN: c_int = 40;
const SC_ESCAPE: c_int = 41;
const SC_BACKSPACE: c_int = 42;
const SC_TAB: c_int = 43;
const SC_SPACE: c_int = 44;
const SC_MINUS: c_int = 45;
const SC_EQUALS: c_int = 46;
const SC_LEFTBRACKET: c_int = 47;
const SC_RIGHTBRACKET: c_int = 48;
const SC_BACKSLASH: c_int = 49;
const SC_NONUSHASH: c_int = 50;
const SC_SEMICOLON: c_int = 51;
const SC_APOSTROPHE: c_int = 52;
const SC_GRAVE: c_int = 53;
const SC_COMMA: c_int = 54;
const SC_PERIOD: c_int = 55;
const SC_SLASH: c_int = 56;
const SC_F1: c_int = 58;
const SC_F12: c_int = 69;
const SC_PAUSE: c_int = 72;
const SC_INSERT: c_int = 73;
const SC_HOME: c_int = 74;
const SC_PAGEUP: c_int = 75;
const SC_DELETE: c_int = 76;
const SC_END: c_int = 77;
const SC_PAGEDOWN: c_int = 78;
const SC_RIGHT: c_int = 79;
const SC_LEFT: c_int = 80;
const SC_DOWN: c_int = 81;
const SC_UP: c_int = 82;
const SC_NUMLOCKCLEAR: c_int = 83;
const SC_KP_DIVIDE: c_int = 84;
const SC_KP_MULTIPLY: c_int = 85;
const SC_KP_MINUS: c_int = 86;
const SC_KP_PLUS: c_int = 87;
const SC_KP_ENTER: c_int = 88;
const SC_KP_1: c_int = 89;
const SC_KP_2: c_int = 90;
const SC_KP_3: c_int = 91;
const SC_KP_4: c_int = 92;
const SC_KP_5: c_int = 93;
const SC_KP_6: c_int = 94;
const SC_KP_7: c_int = 95;
const SC_KP_8: c_int = 96;
const SC_KP_9: c_int = 97;
const SC_KP_0: c_int = 98;
const SC_KP_PERIOD: c_int = 99;
const SC_NONUSBACKSLASH: c_int = 100;
const SC_RETURN2: c_int = 158;
const SC_LCTRL: c_int = 224;
const SC_LSHIFT: c_int = 225;
const SC_LALT: c_int = 226;
const SC_LGUI: c_int = 227;
const SC_RCTRL: c_int = 228;
const SC_RSHIFT: c_int = 229;
const SC_RALT: c_int = 230;
const SC_RGUI: c_int = 231;

/// in_sdl.h `IN_SDL_ScancodeToQuakeKey`: 0 for anything the table omits.
pub fn scancode_to_quake_key(scancode: c_int) -> c_int {
    match scancode {
        SC_TAB => K_TAB,
        SC_RETURN | SC_RETURN2 => K_ENTER,
        SC_ESCAPE => K_ESCAPE,
        SC_SPACE => K_SPACE,

        SC_A..=SC_Z => c_int::from(b'a') + (scancode - SC_A),
        SC_1..=SC_9 => c_int::from(b'1') + (scancode - SC_1),
        SC_0 => c_int::from(b'0'),

        SC_MINUS => c_int::from(b'-'),
        SC_EQUALS => c_int::from(b'='),
        SC_LEFTBRACKET => c_int::from(b'['),
        SC_RIGHTBRACKET => c_int::from(b']'),
        SC_BACKSLASH | SC_NONUSBACKSLASH => c_int::from(b'\\'),
        SC_NONUSHASH => c_int::from(b'#'),
        SC_SEMICOLON => c_int::from(b';'),
        SC_APOSTROPHE => c_int::from(b'\''),
        SC_GRAVE => c_int::from(b'`'),
        SC_COMMA => c_int::from(b','),
        SC_PERIOD => c_int::from(b'.'),
        SC_SLASH => c_int::from(b'/'),

        SC_BACKSPACE => K_BACKSPACE,
        SC_UP => K_UPARROW,
        SC_DOWN => K_DOWNARROW,
        SC_LEFT => K_LEFTARROW,
        SC_RIGHT => K_RIGHTARROW,

        SC_LALT | SC_RALT => K_ALT,
        SC_LCTRL | SC_RCTRL => K_CTRL,
        SC_LSHIFT | SC_RSHIFT => K_SHIFT,

        SC_F1..=SC_F12 => K_F1 + (scancode - SC_F1),
        SC_INSERT => K_INS,
        SC_DELETE => K_DEL,
        SC_PAGEDOWN => K_PGDN,
        SC_PAGEUP => K_PGUP,
        SC_HOME => K_HOME,
        SC_END => K_END,

        SC_NUMLOCKCLEAR => K_KP_NUMLOCK,
        SC_KP_DIVIDE => K_KP_SLASH,
        SC_KP_MULTIPLY => K_KP_STAR,
        SC_KP_MINUS => K_KP_MINUS,
        SC_KP_7 => K_KP_HOME,
        SC_KP_8 => K_KP_UPARROW,
        SC_KP_9 => K_KP_PGUP,
        SC_KP_PLUS => K_KP_PLUS,
        SC_KP_4 => K_KP_LEFTARROW,
        SC_KP_5 => K_KP_5,
        SC_KP_6 => K_KP_RIGHTARROW,
        SC_KP_1 => K_KP_END,
        SC_KP_2 => K_KP_DOWNARROW,
        SC_KP_3 => K_KP_PGDN,
        SC_KP_ENTER => K_KP_ENTER,
        SC_KP_0 => K_KP_INS,
        SC_KP_PERIOD => K_KP_DEL,

        SC_LGUI | SC_RGUI => K_COMMAND,
        SC_PAUSE => K_PAUSE,

        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_matches_in_sdl_h() {
        assert_eq!(scancode_to_quake_key(SC_TAB), K_TAB);
        assert_eq!(scancode_to_quake_key(SC_RETURN), K_ENTER);
        assert_eq!(scancode_to_quake_key(SC_RETURN2), K_ENTER);
        assert_eq!(scancode_to_quake_key(SC_ESCAPE), K_ESCAPE);
        assert_eq!(scancode_to_quake_key(SC_SPACE), K_SPACE);
        assert_eq!(scancode_to_quake_key(SC_A), c_int::from(b'a'));
        assert_eq!(scancode_to_quake_key(SC_Z), c_int::from(b'z'));
        assert_eq!(scancode_to_quake_key(SC_1), c_int::from(b'1'));
        assert_eq!(scancode_to_quake_key(SC_9), c_int::from(b'9'));
        assert_eq!(scancode_to_quake_key(SC_0), c_int::from(b'0'));
        assert_eq!(scancode_to_quake_key(SC_BACKSLASH), c_int::from(b'\\'));
        assert_eq!(scancode_to_quake_key(SC_NONUSBACKSLASH), c_int::from(b'\\'));
        assert_eq!(scancode_to_quake_key(SC_NONUSHASH), c_int::from(b'#'));
        assert_eq!(scancode_to_quake_key(SC_APOSTROPHE), c_int::from(b'\''));
        assert_eq!(scancode_to_quake_key(SC_GRAVE), c_int::from(b'`'));
        assert_eq!(scancode_to_quake_key(SC_F1), K_F1);
        assert_eq!(scancode_to_quake_key(SC_F12), K_F12);
        assert_eq!(scancode_to_quake_key(SC_LALT), K_ALT);
        assert_eq!(scancode_to_quake_key(SC_RGUI), K_COMMAND);
        assert_eq!(scancode_to_quake_key(SC_KP_5), K_KP_5);
        assert_eq!(scancode_to_quake_key(SC_KP_PERIOD), K_KP_DEL);
        assert_eq!(scancode_to_quake_key(SC_PAUSE), K_PAUSE);
        // codes the C table leaves out
        assert_eq!(scancode_to_quake_key(57), 0); // CAPSLOCK
        assert_eq!(scancode_to_quake_key(70), 0); // PRINTSCREEN
        assert_eq!(scancode_to_quake_key(0), 0);
        assert_eq!(scancode_to_quake_key(-1), 0);
        assert_eq!(scancode_to_quake_key(512), 0);
    }

    #[cfg(feature = "sdl3")]
    #[test]
    fn literals_match_sdl3_sys() {
        use sdl3::sys::scancode as s;
        let pairs: &[(c_int, s::SDL_Scancode)] = &[
            (SC_A, s::SDL_SCANCODE_A),
            (SC_Z, s::SDL_SCANCODE_Z),
            (SC_1, s::SDL_SCANCODE_1),
            (SC_9, s::SDL_SCANCODE_9),
            (SC_0, s::SDL_SCANCODE_0),
            (SC_RETURN, s::SDL_SCANCODE_RETURN),
            (SC_ESCAPE, s::SDL_SCANCODE_ESCAPE),
            (SC_BACKSPACE, s::SDL_SCANCODE_BACKSPACE),
            (SC_TAB, s::SDL_SCANCODE_TAB),
            (SC_SPACE, s::SDL_SCANCODE_SPACE),
            (SC_MINUS, s::SDL_SCANCODE_MINUS),
            (SC_EQUALS, s::SDL_SCANCODE_EQUALS),
            (SC_LEFTBRACKET, s::SDL_SCANCODE_LEFTBRACKET),
            (SC_RIGHTBRACKET, s::SDL_SCANCODE_RIGHTBRACKET),
            (SC_BACKSLASH, s::SDL_SCANCODE_BACKSLASH),
            (SC_NONUSHASH, s::SDL_SCANCODE_NONUSHASH),
            (SC_SEMICOLON, s::SDL_SCANCODE_SEMICOLON),
            (SC_APOSTROPHE, s::SDL_SCANCODE_APOSTROPHE),
            (SC_GRAVE, s::SDL_SCANCODE_GRAVE),
            (SC_COMMA, s::SDL_SCANCODE_COMMA),
            (SC_PERIOD, s::SDL_SCANCODE_PERIOD),
            (SC_SLASH, s::SDL_SCANCODE_SLASH),
            (SC_F1, s::SDL_SCANCODE_F1),
            (SC_F12, s::SDL_SCANCODE_F12),
            (SC_PAUSE, s::SDL_SCANCODE_PAUSE),
            (SC_INSERT, s::SDL_SCANCODE_INSERT),
            (SC_HOME, s::SDL_SCANCODE_HOME),
            (SC_PAGEUP, s::SDL_SCANCODE_PAGEUP),
            (SC_DELETE, s::SDL_SCANCODE_DELETE),
            (SC_END, s::SDL_SCANCODE_END),
            (SC_PAGEDOWN, s::SDL_SCANCODE_PAGEDOWN),
            (SC_RIGHT, s::SDL_SCANCODE_RIGHT),
            (SC_LEFT, s::SDL_SCANCODE_LEFT),
            (SC_DOWN, s::SDL_SCANCODE_DOWN),
            (SC_UP, s::SDL_SCANCODE_UP),
            (SC_NUMLOCKCLEAR, s::SDL_SCANCODE_NUMLOCKCLEAR),
            (SC_KP_DIVIDE, s::SDL_SCANCODE_KP_DIVIDE),
            (SC_KP_MULTIPLY, s::SDL_SCANCODE_KP_MULTIPLY),
            (SC_KP_MINUS, s::SDL_SCANCODE_KP_MINUS),
            (SC_KP_PLUS, s::SDL_SCANCODE_KP_PLUS),
            (SC_KP_ENTER, s::SDL_SCANCODE_KP_ENTER),
            (SC_KP_1, s::SDL_SCANCODE_KP_1),
            (SC_KP_2, s::SDL_SCANCODE_KP_2),
            (SC_KP_3, s::SDL_SCANCODE_KP_3),
            (SC_KP_4, s::SDL_SCANCODE_KP_4),
            (SC_KP_5, s::SDL_SCANCODE_KP_5),
            (SC_KP_6, s::SDL_SCANCODE_KP_6),
            (SC_KP_7, s::SDL_SCANCODE_KP_7),
            (SC_KP_8, s::SDL_SCANCODE_KP_8),
            (SC_KP_9, s::SDL_SCANCODE_KP_9),
            (SC_KP_0, s::SDL_SCANCODE_KP_0),
            (SC_KP_PERIOD, s::SDL_SCANCODE_KP_PERIOD),
            (SC_NONUSBACKSLASH, s::SDL_SCANCODE_NONUSBACKSLASH),
            (SC_RETURN2, s::SDL_SCANCODE_RETURN2),
            (SC_LCTRL, s::SDL_SCANCODE_LCTRL),
            (SC_LSHIFT, s::SDL_SCANCODE_LSHIFT),
            (SC_LALT, s::SDL_SCANCODE_LALT),
            (SC_LGUI, s::SDL_SCANCODE_LGUI),
            (SC_RCTRL, s::SDL_SCANCODE_RCTRL),
            (SC_RSHIFT, s::SDL_SCANCODE_RSHIFT),
            (SC_RALT, s::SDL_SCANCODE_RALT),
            (SC_RGUI, s::SDL_SCANCODE_RGUI),
        ];
        for (lit, sc) in pairs {
            assert_eq!(*lit, sc.0, "sdl3 scancode {}", sc.0);
        }
    }

    #[cfg(feature = "sdl2")]
    #[test]
    fn literals_match_sdl2_sys() {
        use sdl2::sys::SDL_Scancode as S;
        let pairs: &[(c_int, S)] = &[
            (SC_A, S::SDL_SCANCODE_A),
            (SC_Z, S::SDL_SCANCODE_Z),
            (SC_1, S::SDL_SCANCODE_1),
            (SC_9, S::SDL_SCANCODE_9),
            (SC_0, S::SDL_SCANCODE_0),
            (SC_RETURN, S::SDL_SCANCODE_RETURN),
            (SC_ESCAPE, S::SDL_SCANCODE_ESCAPE),
            (SC_BACKSPACE, S::SDL_SCANCODE_BACKSPACE),
            (SC_TAB, S::SDL_SCANCODE_TAB),
            (SC_SPACE, S::SDL_SCANCODE_SPACE),
            (SC_MINUS, S::SDL_SCANCODE_MINUS),
            (SC_EQUALS, S::SDL_SCANCODE_EQUALS),
            (SC_LEFTBRACKET, S::SDL_SCANCODE_LEFTBRACKET),
            (SC_RIGHTBRACKET, S::SDL_SCANCODE_RIGHTBRACKET),
            (SC_BACKSLASH, S::SDL_SCANCODE_BACKSLASH),
            (SC_NONUSHASH, S::SDL_SCANCODE_NONUSHASH),
            (SC_SEMICOLON, S::SDL_SCANCODE_SEMICOLON),
            (SC_APOSTROPHE, S::SDL_SCANCODE_APOSTROPHE),
            (SC_GRAVE, S::SDL_SCANCODE_GRAVE),
            (SC_COMMA, S::SDL_SCANCODE_COMMA),
            (SC_PERIOD, S::SDL_SCANCODE_PERIOD),
            (SC_SLASH, S::SDL_SCANCODE_SLASH),
            (SC_F1, S::SDL_SCANCODE_F1),
            (SC_F12, S::SDL_SCANCODE_F12),
            (SC_PAUSE, S::SDL_SCANCODE_PAUSE),
            (SC_INSERT, S::SDL_SCANCODE_INSERT),
            (SC_HOME, S::SDL_SCANCODE_HOME),
            (SC_PAGEUP, S::SDL_SCANCODE_PAGEUP),
            (SC_DELETE, S::SDL_SCANCODE_DELETE),
            (SC_END, S::SDL_SCANCODE_END),
            (SC_PAGEDOWN, S::SDL_SCANCODE_PAGEDOWN),
            (SC_RIGHT, S::SDL_SCANCODE_RIGHT),
            (SC_LEFT, S::SDL_SCANCODE_LEFT),
            (SC_DOWN, S::SDL_SCANCODE_DOWN),
            (SC_UP, S::SDL_SCANCODE_UP),
            (SC_NUMLOCKCLEAR, S::SDL_SCANCODE_NUMLOCKCLEAR),
            (SC_KP_DIVIDE, S::SDL_SCANCODE_KP_DIVIDE),
            (SC_KP_MULTIPLY, S::SDL_SCANCODE_KP_MULTIPLY),
            (SC_KP_MINUS, S::SDL_SCANCODE_KP_MINUS),
            (SC_KP_PLUS, S::SDL_SCANCODE_KP_PLUS),
            (SC_KP_ENTER, S::SDL_SCANCODE_KP_ENTER),
            (SC_KP_1, S::SDL_SCANCODE_KP_1),
            (SC_KP_2, S::SDL_SCANCODE_KP_2),
            (SC_KP_3, S::SDL_SCANCODE_KP_3),
            (SC_KP_4, S::SDL_SCANCODE_KP_4),
            (SC_KP_5, S::SDL_SCANCODE_KP_5),
            (SC_KP_6, S::SDL_SCANCODE_KP_6),
            (SC_KP_7, S::SDL_SCANCODE_KP_7),
            (SC_KP_8, S::SDL_SCANCODE_KP_8),
            (SC_KP_9, S::SDL_SCANCODE_KP_9),
            (SC_KP_0, S::SDL_SCANCODE_KP_0),
            (SC_KP_PERIOD, S::SDL_SCANCODE_KP_PERIOD),
            (SC_NONUSBACKSLASH, S::SDL_SCANCODE_NONUSBACKSLASH),
            (SC_RETURN2, S::SDL_SCANCODE_RETURN2),
            (SC_LCTRL, S::SDL_SCANCODE_LCTRL),
            (SC_LSHIFT, S::SDL_SCANCODE_LSHIFT),
            (SC_LALT, S::SDL_SCANCODE_LALT),
            (SC_LGUI, S::SDL_SCANCODE_LGUI),
            (SC_RCTRL, S::SDL_SCANCODE_RCTRL),
            (SC_RSHIFT, S::SDL_SCANCODE_RSHIFT),
            (SC_RALT, S::SDL_SCANCODE_RALT),
            (SC_RGUI, S::SDL_SCANCODE_RGUI),
        ];
        for (lit, sc) in pairs {
            assert_eq!(*lit, *sc as c_int, "sdl2 scancode {}", *sc as c_int);
        }
    }
}
