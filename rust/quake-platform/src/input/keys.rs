//! `Quake/keys.h` -- the `K_*` values the input layer emits. `keys.h` is not
//! in the bindgen wrapper (it is not core-header clean), so they are spelled
//! locally, as `quake-capi`'s `keys` module and the ctest key tests do.

#![allow(dead_code)]

use core::ffi::c_int;

pub const K_TAB: c_int = 9;
pub const K_ENTER: c_int = 13;
pub const K_ESCAPE: c_int = 27;
pub const K_SPACE: c_int = 32;
pub const K_BACKSPACE: c_int = 127;
pub const K_UPARROW: c_int = 128;
pub const K_DOWNARROW: c_int = 129;
pub const K_LEFTARROW: c_int = 130;
pub const K_RIGHTARROW: c_int = 131;
pub const K_ALT: c_int = 132;
pub const K_CTRL: c_int = 133;
pub const K_SHIFT: c_int = 134;
pub const K_F1: c_int = 135;
pub const K_F12: c_int = 146;
pub const K_INS: c_int = 147;
pub const K_DEL: c_int = 148;
pub const K_PGDN: c_int = 149;
pub const K_PGUP: c_int = 150;
pub const K_HOME: c_int = 151;
pub const K_END: c_int = 152;
pub const K_KP_NUMLOCK: c_int = 153;
pub const K_KP_SLASH: c_int = 154;
pub const K_KP_STAR: c_int = 155;
pub const K_KP_MINUS: c_int = 156;
pub const K_KP_HOME: c_int = 157;
pub const K_KP_UPARROW: c_int = 158;
pub const K_KP_PGUP: c_int = 159;
pub const K_KP_PLUS: c_int = 160;
pub const K_KP_LEFTARROW: c_int = 161;
pub const K_KP_5: c_int = 162;
pub const K_KP_RIGHTARROW: c_int = 163;
pub const K_KP_END: c_int = 164;
pub const K_KP_DOWNARROW: c_int = 165;
pub const K_KP_PGDN: c_int = 166;
pub const K_KP_ENTER: c_int = 167;
pub const K_KP_INS: c_int = 168;
pub const K_KP_DEL: c_int = 169;
pub const K_COMMAND: c_int = 170;
pub const K_MOUSE1: c_int = 200;
pub const K_MOUSE2: c_int = 201;
pub const K_MOUSE3: c_int = 202;
pub const K_MOUSE4: c_int = 203;
pub const K_MOUSE5: c_int = 204;
pub const K_MWHEELUP: c_int = 205;
pub const K_MWHEELDOWN: c_int = 206;
pub const K_LTHUMB: c_int = 207;
pub const K_RTHUMB: c_int = 208;
pub const K_LSHOULDER: c_int = 209;
pub const K_RSHOULDER: c_int = 210;
pub const K_ABUTTON: c_int = 211;
pub const K_BBUTTON: c_int = 212;
pub const K_XBUTTON: c_int = 213;
pub const K_YBUTTON: c_int = 214;
pub const K_LTRIGGER: c_int = 215;
pub const K_RTRIGGER: c_int = 216;
pub const K_MISC1: c_int = 217;
pub const K_PADDLE1: c_int = 218;
pub const K_PADDLE2: c_int = 219;
pub const K_PADDLE3: c_int = 220;
pub const K_PADDLE4: c_int = 221;
pub const K_TOUCHPAD: c_int = 222;
pub const K_PAUSE: c_int = 223;
