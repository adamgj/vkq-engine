//! `Quake/keys.c` -- key event dispatch, console line editing, the binding
//! table and the command history file (Rust migration Phase 7 M10b,
//! Pattern A whole-file swap).
//!
//! ## ADR-009 raise-topology audit
//!
//! `keys.c` has no direct `Host_Error`/`Host_EndGame` site. Its two
//! `Sys_Error ("Bad key_dest")` sites (`keys.c:1102`, `:1211`) terminate
//! rather than jumping, so they are called straight through.
//!
//! Eleven transitive raise sites go through `Quake/keys_glue.c`
//! `Host_Guard` trampolines: `SCR_UpdateScreen` (`keys.c:285`),
//! `Con_TabComplete`, `Con_Scroll`, `Con_ForceMouseMove`, `Con_SelectAll`,
//! `Con_CopySelectionToClipboard`, `Con_ToggleConsole_f`, `M_Keydown`,
//! `M_Charinput`, `M_ToggleMenu_f` and `VID_Toggle`. Console redraws reach
//! `Mod_LoadModel` (`gl_model.c:531`) and the menus run console commands and
//! load maps, so none of them could be closed by inspection; per the
//! `cl_parse_glue.c` doctrine they are all guarded.
//!
//! That makes eight entry points `Raise`-returning status cores:
//! [`quake_rs_key_console`], [`quake_rs_char_console`],
//! [`quake_rs_key_event`], [`quake_rs_key_event_with_keycode`],
//! [`quake_rs_char_event`], [`quake_rs_key_clear_states`],
//! [`quake_rs_key_begin_input_grab`] and [`quake_rs_key_end_input_grab`].
//! Everything else is exported under its plain C name: `Cbuf_AddText`
//! (`crate::cmd:146` pre-checks the sizebuf so `SZ_Write` never reaches
//! `Host_Error`), `Cmd_AddCommand2`, `Cmd_Argc`/`Cmd_Argv`,
//! `Con_Printf`/`Con_SafePrintf`, `PL_GetClipboardData`, `M_TextEntry`,
//! `M_WaitingForKeyBinding`, `IN_UpdateInputMode`/`IN_Activate`/
//! `IN_DeactivateForConsole`, `q_strdup`/`q_strcasecmp`/`Mem_Free` and the
//! history file's stdio calls cannot longjmp.
//!
//! `Key_WriteBindings` is the one exception to the plain-export rule: its
//! `FILE *` parameter has no cbindgen spelling, so the public name is a C
//! forward in the glue over [`quake_rs_key_write_bindings`]. It needs no
//! guard.
//!
//! ## Ownership (ADR-007)
//!
//! Every C-visible object `keys.c` defined keeps its storage in
//! `Quake/keys_glue.c` -- all fourteen had external linkage in the original,
//! and `console.c:37`/`:690`/`:743`/`:2056` plus `menu.c:114` resolve
//! `keydown[]` and `history_line` by local re-declaration. `keynames[]` stays
//! C too, which keeps it a genuinely separate object from the oracle's copy
//! in the ctest differential build. The six file-statics were not C-visible
//! and live here: [`CHAT_BUFFER`], [`CHAT_BUFFERLEN`], [`KEY_INPUTGRAB`],
//! `Key_Console`'s `current`, `Key_KeynumToString`'s `tinystr` and
//! `Key_UpdateForDest`'s `forced`.

use core::ffi::{c_char, c_int, c_void, CStr};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::host as hg;
use quake_c_sys::keys as g;
use quake_types::host::{ClientStatic, CA_CONNECTED, CA_DEDICATED, CA_DISCONNECTED};

/// A `Host_Guard` status: 0 means "no raise". Non-zero must be returned to
/// `Quake/keys_glue.c` untouched.
type Raise = c_int;

macro_rules! raise {
    ($e:expr) => {{
        let r: Raise = $e;
        if r != 0 {
            return r;
        }
    }};
}

extern "C" {
    /// ADR-007 row closed in T7.4; storage in [`crate::cl_main`].
    static mut cls: ClientStatic;
}

/// `keys.c:29`.
const HISTORY_FILE_NAME: &CStr = c"history.txt";

const MAXCMDLINE: c_int = g::MAXCMDLINE as c_int;
const CMDLINES: c_int = g::CMDLINES as c_int;
const MAX_KEYS: c_int = g::MAX_KEYS as c_int;

/// `console.h:57-61` -- `tabcomplete_t`.
const TABCOMPLETE_AUTOHINT: c_int = 0;
const TABCOMPLETE_USER: c_int = 1;

/// `keys.h:126-133` -- `keydest_t`.
const KEY_GAME: c_int = 0;
const KEY_CONSOLE: c_int = 1;
const KEY_MESSAGE: c_int = 2;
const KEY_MENU: c_int = 3;

// `keys.h:32-124` -- keycode_t. Spelled here rather than exported so cbindgen
// never emits a second set of definitions into quake_rs.h.
const K_TAB: c_int = 9;
const K_ENTER: c_int = 13;
const K_ESCAPE: c_int = 27;
const K_BACKSPACE: c_int = 127;
const K_UPARROW: c_int = 128;
const K_DOWNARROW: c_int = 129;
const K_LEFTARROW: c_int = 130;
const K_RIGHTARROW: c_int = 131;
const K_ALT: c_int = 132;
const K_CTRL: c_int = 133;
const K_SHIFT: c_int = 134;
const K_F1: c_int = 135;
const K_INS: c_int = 147;
const K_DEL: c_int = 148;
const K_PGDN: c_int = 149;
const K_PGUP: c_int = 150;
const K_HOME: c_int = 151;
const K_END: c_int = 152;
const K_KP_NUMLOCK: c_int = 153;
const K_KP_SLASH: c_int = 154;
const K_KP_STAR: c_int = 155;
const K_KP_MINUS: c_int = 156;
const K_KP_HOME: c_int = 157;
const K_KP_UPARROW: c_int = 158;
const K_KP_PGUP: c_int = 159;
const K_KP_PLUS: c_int = 160;
const K_KP_LEFTARROW: c_int = 161;
const K_KP_5: c_int = 162;
const K_KP_RIGHTARROW: c_int = 163;
const K_KP_END: c_int = 164;
const K_KP_DOWNARROW: c_int = 165;
const K_KP_PGDN: c_int = 166;
const K_KP_ENTER: c_int = 167;
const K_KP_INS: c_int = 168;
const K_KP_DEL: c_int = 169;
#[cfg(any(target_os = "macos", target_os = "ios"))]
const K_COMMAND: c_int = 170;
const K_MOUSE1: c_int = 200;
const K_MOUSE2: c_int = 201;
const K_MOUSE3: c_int = 202;
const K_MWHEELUP: c_int = 205;
const K_MWHEELDOWN: c_int = 206;

// ---------------------------------------------------------------------------
// small helpers

/// `Quake/q_minmax.h` `CLAMP` at `int`.
#[inline]
fn clamp_i(minval: c_int, val: c_int, maxval: c_int) -> c_int {
    if val < minval {
        minval
    } else if val > maxval {
        maxval
    } else {
        val
    }
}

/// `Quake/q_minmax.h` `CLAMP` at `float`.
#[inline]
fn clamp_f(minval: f32, val: f32, maxval: f32) -> f32 {
    if val < minval {
        minval
    } else if val > maxval {
        maxval
    } else {
        val
    }
}

/// `strlen` over an engine C string.
///
/// # Safety
/// `p` must point at a NUL-terminated string.
#[inline]
unsafe fn strlen(p: *const c_char) -> usize {
    // SAFETY: caller contract
    unsafe { CStr::from_ptr(p) }.to_bytes().len()
}

/// `&key_lines[i][0]`.
///
/// COMPAT: raw offsetting rather than a bounds-checked index -- `keys.c`
/// indexes `key_lines` with `edit_line`/`history_line` unchecked (`:263`,
/// `:387`, `:410`), and only `Char_Console` (`:501`) clamps.
///
/// # Safety
/// `i` must be within `0 .. CMDLINES`.
#[inline]
unsafe fn key_line(i: c_int) -> *mut c_char {
    // SAFETY: caller contract; key_lines is the glue-owned CMDLINES x
    // MAXCMDLINE array.
    unsafe {
        (&raw mut g::key_lines)
            .cast::<c_char>()
            .offset(i as isize * MAXCMDLINE as isize)
    }
}

/// `&keybindings[i]`.
///
/// # Safety
/// `i` must be within `0 .. MAX_KEYS`.
#[inline]
unsafe fn keybinding(i: c_int) -> *mut *mut c_char {
    // SAFETY: caller contract; keybindings is the glue-owned MAX_KEYS array.
    unsafe {
        (&raw mut g::keybindings)
            .cast::<*mut c_char>()
            .offset(i as isize)
    }
}

/// `keydown[i]`.
#[inline]
fn keydown(i: c_int) -> bool {
    // SAFETY: every caller passes a keycode constant or a key already range
    // checked by Key_EventWithKeycode (keys.c:1037).
    unsafe { *(&raw const g::keydown).cast::<bool>().offset(i as isize) }
}

/// `cmd.h:110` -- `Cmd_AddCommand (name, func)`.
fn add_command(name: *const c_char, func: unsafe extern "C" fn()) {
    // SAFETY: `name` is a static NUL-terminated literal and `func` has the
    // `xcommand_t` signature.
    unsafe {
        c::Cmd_AddCommand2(name, Some(func), c::cmd_source_t_src_command, false);
    }
}

/// `keys.c:248`.
#[inline]
fn get_history_prev_line(line: c_int) -> c_int {
    (line + (CMDLINES - 1)) % CMDLINES
}

/// `keys.c:253`.
#[inline]
fn get_history_next_line(line: c_int) -> c_int {
    (line + 1) % CMDLINES
}

// ---------------------------------------------------------------------------
// LINE TYPING INTO THE CONSOLE

/// `keys.c:152`.
///
/// # Safety
/// The console edit line must be initialised.
unsafe fn paste_to_console() {
    // SAFETY: PL_GetClipboardData returns a Mem_Alloc'd NUL-terminated string
    // or NULL; the edit line is the glue-owned key_lines array.
    unsafe {
        if g::key_linepos == MAXCMDLINE - 1 {
            return;
        }

        let cbd = g::PL_GetClipboardData();
        if cbd.is_null() {
            return;
        }

        let mut p = cbd;
        while *p != 0 {
            if *p == b'\n' as c_char || *p == b'\r' as c_char || *p == 8 {
                *p = 0;
                break;
            }
            p = p.add(1);
        }

        let mut inslen = p.offset_from(cbd) as c_int;
        if inslen + g::key_linepos > MAXCMDLINE - 1 {
            inslen = MAXCMDLINE - 1 - g::key_linepos;
        }
        if inslen > 0 {
            let workline = key_line(g::edit_line).offset(g::key_linepos as isize);
            let mut mvlen = strlen(workline) as c_int;
            if mvlen + inslen + g::key_linepos > MAXCMDLINE - 1 {
                mvlen = MAXCMDLINE - 1 - g::key_linepos - inslen;
                if mvlen < 0 {
                    mvlen = 0;
                }
            }

            // insert the string
            if mvlen != 0 {
                ptr::copy(workline, workline.offset(inslen as isize), mvlen as usize);
            }
            ptr::copy_nonoverlapping(cbd, workline, inslen as usize);
            g::key_linepos += inslen;
            *workline.offset((mvlen + inslen) as isize) = 0;
        }
        c::Mem_Free(cbd as *const c_void);
    }
}

/// `keys.c:200`.
#[inline]
fn key_is_word_separator(ch: c_char) -> bool {
    matches!(ch as u8, b' ' | b'_' | b'\t' | b';')
}

/// `keys.c:214`.
///
/// # Safety
/// The console edit line must be initialised.
unsafe fn key_find_word_boundary(dir: c_int) -> c_int {
    // SAFETY: the edit line is the glue-owned key_lines array.
    unsafe {
        let workline = key_line(g::edit_line);
        let len = strlen(workline) as c_int;
        let mut pos = g::key_linepos;

        if dir < 0 {
            while pos > 1 && key_is_word_separator(*workline.offset(pos as isize - 1)) {
                pos -= 1;
            }
            while pos > 1 && !key_is_word_separator(*workline.offset(pos as isize - 1)) {
                pos -= 1;
            }
        } else {
            while pos < len && !key_is_word_separator(*workline.offset(pos as isize)) {
                pos += 1;
            }
            while pos < len && key_is_word_separator(*workline.offset(pos as isize)) {
                pos += 1;
            }
        }

        pos
    }
}

/// `Key_Console`'s `static char current[MAXCMDLINE]` (`keys.c:260`).
static mut CURRENT: [c_char; g::MAXCMDLINE] = [0; g::MAXCMDLINE];

/// `keys.c:258` -- interactive line editing and console scrollback.
///
/// # Safety
/// C ABI entry point; call only from `Quake/keys_glue.c`'s `Key_Console`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_key_console(key: c_int) -> Raise {
    // SAFETY: caller contract
    unsafe { key_console(key) }
}

unsafe fn key_console(key: c_int) -> Raise {
    // SAFETY: the console edit line, keydown[] and the con_* scrollback state
    // are all C-owned objects the glue and console.c define.
    unsafe {
        let mut workline = key_line(g::edit_line);

        match key {
            K_ENTER | K_KP_ENTER => {
                g::key_tabpartial[0] = 0;
                c::Cbuf_AddText(workline.add(1)); // skip the prompt
                c::Cbuf_AddText(c"\n".as_ptr());
                c::Con_Printf(c"%s\n".as_ptr(), workline);

                // If the last two lines are identical, skip storing this line
                // in history by not incrementing edit_line
                let prev = key_line(get_history_prev_line(g::edit_line));
                if CStr::from_ptr(workline) != CStr::from_ptr(prev) {
                    g::edit_line = get_history_next_line(g::edit_line);
                }

                g::history_line = g::edit_line;
                let line = key_line(g::edit_line);
                *line = b']' as c_char;
                *line.add(1) = 0; // johnfitz -- otherwise old history items show up in the new edit line
                g::key_linepos = 1;
                g::key_tabhint[0] = 0;
                if cls.state == CA_DISCONNECTED {
                    // force an update, because the command may take some time
                    raise!(g::Keys_Glue_UpdateScreen());
                }
                0
            }

            K_TAB => {
                raise!(g::Keys_Glue_TabComplete(TABCOMPLETE_USER));
                0
            }

            K_BACKSPACE => {
                g::key_tabpartial[0] = 0;
                if g::key_linepos > 1 {
                    let numchars = if keydown(K_CTRL) {
                        g::key_linepos - key_find_word_boundary(-1)
                    } else {
                        1
                    };
                    workline = workline.offset((g::key_linepos - numchars) as isize);
                    let len = strlen(workline);
                    ptr::copy(
                        workline.offset(numchars as isize),
                        workline,
                        len + 1 - numchars as usize,
                    );
                    g::key_linepos -= numchars;
                    raise!(g::Keys_Glue_TabComplete(TABCOMPLETE_AUTOHINT));
                }
                0
            }

            K_DEL => {
                g::key_tabpartial[0] = 0;
                workline = workline.offset(g::key_linepos as isize);
                if *workline != 0 {
                    let numchars = if keydown(K_CTRL) {
                        key_find_word_boundary(1) - g::key_linepos
                    } else {
                        1
                    };
                    let len = strlen(workline);
                    ptr::copy(
                        workline.offset(numchars as isize),
                        workline,
                        len + 1 - numchars as usize,
                    );
                    raise!(g::Keys_Glue_TabComplete(TABCOMPLETE_AUTOHINT));
                }
                0
            }

            K_HOME => {
                if keydown(K_CTRL) {
                    // skip initial empty lines
                    let mut i = g::con_current - g::con_totallines + 1;
                    let mut x = 0;
                    while i <= g::con_current {
                        let line = g::con_text
                            .offset((i % g::con_totallines) as isize * g::con_linewidth as isize);
                        x = 0;
                        while x < g::con_linewidth {
                            if *line.offset(x as isize) != b' ' as c_char {
                                break;
                            }
                            x += 1;
                        }
                        if x != g::con_linewidth {
                            break;
                        }
                        i += 1;
                    }
                    let _ = x;
                    g::con_backscroll = clamp_i(
                        0,
                        g::con_current - i % g::con_totallines - 2,
                        g::con_totallines - (g::glheight >> 3) - 1,
                    );
                } else {
                    g::key_linepos = 1;
                }
                raise!(g::Keys_Glue_TabComplete(TABCOMPLETE_AUTOHINT));
                raise!(g::Keys_Glue_ForceMouseMove());
                0
            }

            K_END => {
                if keydown(K_CTRL) {
                    g::con_backscroll = 0;
                } else {
                    g::key_linepos = strlen(workline) as c_int;
                }
                raise!(g::Keys_Glue_TabComplete(TABCOMPLETE_AUTOHINT));
                raise!(g::Keys_Glue_ForceMouseMove());
                0
            }

            K_PGUP | K_MWHEELUP => {
                let lines = if keydown(K_CTRL) {
                    (g::con_vislines >> 3) - 4
                } else {
                    2
                };
                raise!(g::Keys_Glue_Scroll(lines));
                0
            }

            K_PGDN | K_MWHEELDOWN => {
                let lines = if keydown(K_CTRL) {
                    -((g::con_vislines >> 3) - 4)
                } else {
                    -2
                };
                raise!(g::Keys_Glue_Scroll(lines));
                0
            }

            K_LEFTARROW => {
                if g::key_linepos > 1 {
                    if keydown(K_CTRL) {
                        g::key_linepos = key_find_word_boundary(-1);
                    } else {
                        g::key_linepos -= 1;
                    }
                    g::key_blinktime = g::realtime;
                    raise!(g::Keys_Glue_TabComplete(TABCOMPLETE_AUTOHINT));
                }
                0
            }

            K_RIGHTARROW => {
                let len = strlen(workline) as c_int;
                if len == g::key_linepos {
                    let prev = key_line(get_history_prev_line(g::edit_line));
                    if strlen(prev) as c_int <= g::key_linepos {
                        return 0; // no character to get
                    }
                    workline = workline.offset(g::key_linepos as isize);
                    *workline = *prev.offset(g::key_linepos as isize);
                    *workline.add(1) = 0;
                    g::key_linepos += 1;
                } else {
                    if keydown(K_CTRL) {
                        g::key_linepos = key_find_word_boundary(1);
                    } else {
                        g::key_linepos += 1;
                    }
                    g::key_blinktime = g::realtime;
                }
                raise!(g::Keys_Glue_TabComplete(TABCOMPLETE_AUTOHINT));
                0
            }

            K_UPARROW => {
                if g::history_line == g::edit_line {
                    let len = strlen(workline);
                    ptr::copy_nonoverlapping(
                        workline,
                        (&raw mut CURRENT).cast::<c_char>(),
                        len + 1,
                    );
                }

                let history_line_last = g::history_line;
                loop {
                    g::history_line = get_history_prev_line(g::history_line);
                    if g::history_line == g::edit_line || *key_line(g::history_line).add(1) != 0 {
                        break;
                    }
                }

                if g::history_line == g::edit_line {
                    g::history_line = history_line_last;
                    return 0;
                }

                g::key_tabpartial[0] = 0;
                let src = key_line(g::history_line);
                let len = strlen(src);
                ptr::copy(src, workline, len + 1);
                g::key_linepos = len as c_int;
                raise!(g::Keys_Glue_TabComplete(TABCOMPLETE_AUTOHINT));
                0
            }

            K_DOWNARROW => {
                if g::history_line == g::edit_line {
                    return 0;
                }

                g::key_tabpartial[0] = 0;

                loop {
                    g::history_line = get_history_next_line(g::history_line);
                    if g::history_line == g::edit_line || *key_line(g::history_line).add(1) != 0 {
                        break;
                    }
                }

                let len = if g::history_line == g::edit_line {
                    let cur = (&raw const CURRENT).cast::<c_char>();
                    let len = strlen(cur);
                    ptr::copy_nonoverlapping(cur, workline, len + 1);
                    len
                } else {
                    let src = key_line(g::history_line);
                    let len = strlen(src);
                    ptr::copy(src, workline, len + 1);
                    len
                };
                g::key_linepos = len as c_int;
                raise!(g::Keys_Glue_TabComplete(TABCOMPLETE_AUTOHINT));
                0
            }

            K_INS => {
                if keydown(K_SHIFT) {
                    /* Shift-Ins paste */
                    paste_to_console();
                } else if keydown(K_CTRL) {
                    let mut copied = false;
                    raise!(g::Keys_Glue_CopySelectionToClipboard(&mut copied));
                    let _ = copied;
                    return 0;
                } else {
                    g::key_insert ^= 1;
                }
                raise!(g::Keys_Glue_TabComplete(TABCOMPLETE_AUTOHINT));
                0
            }

            _ if key == b'v' as c_int || key == b'V' as c_int => {
                if paste_modifier_down() {
                    /* Ctrl+v paste */
                    paste_to_console();
                    raise!(g::Keys_Glue_TabComplete(TABCOMPLETE_AUTOHINT));
                }
                0
            }

            _ if key == b'a' as c_int || key == b'A' as c_int => {
                if keydown(K_CTRL) {
                    /* Ctrl+A: select whole buffer */
                    raise!(g::Keys_Glue_SelectAll());
                }
                0
            }

            _ if key == b'c' as c_int || key == b'C' as c_int => {
                if keydown(K_CTRL) {
                    /* Ctrl+C: abort the line -- S.A */
                    let mut copied = false;
                    raise!(g::Keys_Glue_CopySelectionToClipboard(&mut copied));
                    if copied {
                        return 0;
                    }
                    c::Con_Printf(c"%s\n".as_ptr(), workline);
                    *workline = b']' as c_char;
                    *workline.add(1) = 0;
                    g::key_linepos = 1;
                    g::history_line = g::edit_line;
                    g::key_tabhint[0] = 0;
                }
                0
            }

            _ => 0,
        }
    }
}

/// `keys.c:452-456` -- the paste chord is Cmd on Apple platforms, Ctrl
/// everywhere else.
#[inline]
fn paste_modifier_down() -> bool {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        keydown(K_COMMAND)
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    {
        keydown(K_CTRL)
    }
}

/// `keys.c:499`.
///
/// # Safety
/// C ABI entry point; call only from `Quake/keys_glue.c`'s `Char_Console`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_char_console(key: c_int) -> Raise {
    // SAFETY: the console edit line is the glue-owned key_lines array.
    unsafe {
        let mut workline = key_line(clamp_i(0, g::edit_line, CMDLINES - 1));

        if g::key_linepos < MAXCMDLINE - 1 {
            let endpos = *workline.offset(g::key_linepos as isize) == 0;

            g::key_tabpartial[0] = 0; // johnfitz
                                      // if inserting, move the text to the right
            if g::key_insert != 0 && !endpos {
                *workline.offset(MAXCMDLINE as isize - 2) = 0;
                workline = workline.offset(g::key_linepos as isize);
                let len = strlen(workline) + 1;
                ptr::copy(workline, workline.add(1), len);
                *workline = key as c_char;
            } else {
                workline = workline.offset(g::key_linepos as isize);
                *workline = key as c_char;
                // null terminate if at the end
                if endpos {
                    *workline.add(1) = 0;
                }
            }
            g::key_linepos += 1;

            raise!(g::Keys_Glue_TabComplete(TABCOMPLETE_AUTOHINT));
        }
        0
    }
}

// ---------------------------------------------------------------------------

/// `keys.c:540`.
static mut CHAT_BUFFER: [c_char; g::MAXCMDLINE] = [0; g::MAXCMDLINE];
/// `keys.c:541`.
static mut CHAT_BUFFERLEN: c_int = 0;

/// `keys.c:543`.
#[no_mangle]
pub extern "C" fn Key_GetChatBuffer() -> *const c_char {
    (&raw const CHAT_BUFFER).cast::<c_char>()
}

/// `keys.c:548`.
#[no_mangle]
pub extern "C" fn Key_GetChatMsgLen() -> c_int {
    // SAFETY: single-threaded engine state, like the C file-static it replaces
    unsafe { CHAT_BUFFERLEN }
}

/// `keys.c:553`.
#[no_mangle]
pub extern "C" fn Key_EndChat() {
    // SAFETY: key_dest is glue-owned; the chat buffer is this module's.
    unsafe {
        g::key_dest = KEY_GAME;
        CHAT_BUFFERLEN = 0;
        CHAT_BUFFER[0] = 0;
    }
}

/// `keys.c:560`.
#[no_mangle]
pub extern "C" fn Key_Message(key: c_int) {
    // SAFETY: Cbuf_AddText takes NUL-terminated strings; chat_buffer is this
    // module's storage and is always NUL-terminated.
    unsafe {
        match key {
            K_ENTER | K_KP_ENTER => {
                if g::chat_team {
                    c::Cbuf_AddText(c"say_team \"".as_ptr());
                } else {
                    c::Cbuf_AddText(c"say \"".as_ptr());
                }
                c::Cbuf_AddText((&raw const CHAT_BUFFER).cast::<c_char>());
                c::Cbuf_AddText(c"\"\n".as_ptr());

                Key_EndChat();
            }

            K_ESCAPE => Key_EndChat(),

            K_BACKSPACE if CHAT_BUFFERLEN != 0 => {
                CHAT_BUFFERLEN -= 1;
                CHAT_BUFFER[CHAT_BUFFERLEN as usize] = 0;
            }

            _ => {}
        }
    }
}

/// `keys.c:584`.
#[no_mangle]
pub extern "C" fn Char_Message(key: c_int) {
    // SAFETY: chat_buffer is this module's storage; the length check below is
    // the C bound.
    unsafe {
        if CHAT_BUFFERLEN == MAXCMDLINE - 1 {
            return; // all full
        }

        CHAT_BUFFER[CHAT_BUFFERLEN as usize] = key as c_char;
        CHAT_BUFFERLEN += 1;
        CHAT_BUFFER[CHAT_BUFFERLEN as usize] = 0;
    }
}

// ---------------------------------------------------------------------------

/// `keys.c:602` -- a key number to index `keybindings[]` with. Single ASCII
/// characters return themselves, the `K_*` names are matched up.
///
/// # Safety
/// `str_` must be NULL or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn Key_StringToKeynum(str_: *const c_char) -> c_int {
    // SAFETY: caller contract; keynames is the glue-owned {NULL,0}-terminated
    // table.
    unsafe {
        if str_.is_null() || *str_ == 0 {
            return -1;
        }
        // COMPAT: `return str[0]` promotes a plain `char`, so the sign of a
        // high-bit byte follows the platform's char signedness exactly as the
        // C does (keys.c:610).
        if *str_.add(1) == 0 {
            return *str_ as c_int;
        }

        let mut kn = (&raw const g::keynames).cast::<g::keyname_t>();
        while !(*kn).name.is_null() {
            if g::q_strcasecmp(str_, (*kn).name) == 0 {
                return (*kn).keynum;
            }
            kn = kn.add(1);
        }
        -1
    }
}

/// `Key_KeynumToString`'s `static char tinystr[2]` (`keys.c:630`).
static mut TINYSTR: [c_char; 2] = [0; 2];

/// `keys.c:628` -- a string (either a single ASCII char, or a `K_*` name) for
/// the given keynum.
#[no_mangle]
pub extern "C" fn Key_KeynumToString(keynum: c_int) -> *const c_char {
    // SAFETY: keynames is the glue-owned {NULL,0}-terminated table; tinystr
    // replaces the C function-static of the same name.
    unsafe {
        if keynum == -1 {
            return c"".as_ptr();
        }
        if keynum > 32 && keynum < 127 {
            // printable ascii
            TINYSTR[0] = keynum as c_char;
            TINYSTR[1] = 0;
            return (&raw const TINYSTR).cast::<c_char>();
        }

        let mut kn = (&raw const g::keynames).cast::<g::keyname_t>();
        while !(*kn).name.is_null() {
            if keynum == (*kn).keynum {
                return (*kn).name;
            }
            kn = kn.add(1);
        }

        c"<UNKNOWN KEYNUM>".as_ptr()
    }
}

/// `keys.c:656`.
///
/// # Safety
/// `binding` must be NULL or a NUL-terminated string, and `keynum` must be
/// `-1` or within `0 .. MAX_KEYS`.
#[no_mangle]
pub unsafe extern "C" fn Key_SetBinding(keynum: c_int, binding: *const c_char) {
    // SAFETY: caller contract; keybindings is the glue-owned array and its
    // entries are q_strdup allocations.
    unsafe {
        if keynum == -1 {
            return;
        }

        // free old bindings
        let slot = keybinding(keynum);
        if !(*slot).is_null() {
            c::Mem_Free(*slot as *const c_void);
            *slot = ptr::null_mut();
        }

        // allocate memory for new binding
        if !binding.is_null() {
            *slot = g::q_strdup(binding);
        }
    }
}

/// `keys.c:678`.
///
/// # Safety
/// C ABI `xcommand_t`; call only through the command system.
#[no_mangle]
pub unsafe extern "C" fn Key_Unbind_f() {
    // SAFETY: Cmd_Argv returns NUL-terminated argv strings.
    unsafe {
        if c::Cmd_Argc() != 2 {
            c::Con_Printf(c"unbind <key> : remove commands from a key\n".as_ptr());
            return;
        }

        let b = Key_StringToKeynum(c::Cmd_Argv(1));
        if b == -1 {
            c::Con_Printf(c"\"%s\" isn't a valid key\n".as_ptr(), c::Cmd_Argv(1));
            return;
        }

        Key_SetBinding(b, ptr::null());
    }
}

/// `keys.c:698`.
///
/// # Safety
/// C ABI `xcommand_t`; call only through the command system.
#[no_mangle]
pub unsafe extern "C" fn Key_Unbindall_f() {
    // SAFETY: keybindings is the glue-owned MAX_KEYS array.
    unsafe {
        for i in 0..MAX_KEYS {
            if !(*keybinding(i)).is_null() {
                Key_SetBinding(i, ptr::null());
            }
        }
    }
}

/// `keys.c:714` -- johnfitz.
///
/// # Safety
/// C ABI `xcommand_t`; call only through the command system.
#[no_mangle]
pub unsafe extern "C" fn Key_Bindlist_f() {
    // SAFETY: keybindings is the glue-owned array of NUL-terminated strings.
    unsafe {
        let mut count = 0;
        for i in 0..MAX_KEYS {
            let kb = *keybinding(i);
            if !kb.is_null() && *kb != 0 {
                c::Con_SafePrintf(c"   %s \"%s\"\n".as_ptr(), Key_KeynumToString(i), kb);
                count += 1;
            }
        }
        c::Con_SafePrintf(c"%i bindings\n".as_ptr(), count);
    }
}

/// `keys.c:735`.
///
/// # Safety
/// C ABI `xcommand_t`; call only through the command system.
#[no_mangle]
pub unsafe extern "C" fn Key_Bind_f() {
    // SAFETY: Cmd_Argv returns NUL-terminated argv strings; `cmd` is a local
    // 1024-byte buffer exactly like the C.
    unsafe {
        let c_argc = c::Cmd_Argc();

        if c_argc < 2 {
            c::Con_Printf(c"bind <key> [command] : attach a command to a key\n".as_ptr());
            return;
        }
        let b = Key_StringToKeynum(c::Cmd_Argv(1));
        if b == -1 {
            c::Con_Printf(c"\"%s\" isn't a valid key\n".as_ptr(), c::Cmd_Argv(1));
            return;
        }

        if c_argc == 2 {
            let kb = *keybinding(b);
            if !kb.is_null() {
                c::Con_Printf(c"\"%s\" = \"%s\"\n".as_ptr(), c::Cmd_Argv(1), kb);
            } else {
                c::Con_Printf(c"\"%s\" is not bound\n".as_ptr(), c::Cmd_Argv(1));
            }
            return;
        }

        // copy the rest of the command line
        let mut cmd = [0u8; 1024];
        for i in 2..c_argc {
            quake_util::strl::strlcat(&mut cmd, CStr::from_ptr(c::Cmd_Argv(i)).to_bytes());
            if i != c_argc - 1 {
                quake_util::strl::strlcat(&mut cmd, b" ");
            }
        }

        Key_SetBinding(b, cmd.as_ptr() as *const c_char);
    }
}

/// `keys.c:782` -- writes lines containing "bind key value".
///
/// COMPAT: `config.cfg` is a byte-diff gate (PLAN.md section 7), so the two
/// `fprintf` calls stay call-throughs to libc rather than being reformatted in
/// Rust -- the same reasoning as `Cvar_WriteVariables` (`cvar.rs:854`).
///
/// # Safety
/// `f` must be an open, writable `FILE *`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_key_write_bindings(f: *mut c::FILE) {
    // SAFETY: caller contract; keybindings is the glue-owned array.
    unsafe {
        // unbindall before loading stored bindings:
        if g::cfg_unbindall.value != 0.0 {
            g::fprintf(f, c"unbindall\n".as_ptr());
        }
        for i in 0..MAX_KEYS {
            let kb = *keybinding(i);
            if !kb.is_null() && *kb != 0 {
                g::fprintf(
                    f,
                    c"bind \"%s\" \"%s\"\n".as_ptr(),
                    Key_KeynumToString(i),
                    kb,
                );
            }
        }
    }
}

/// `keys.c:796`.
///
/// # Safety
/// `mode` must be a NUL-terminated stdio mode string.
unsafe fn history_open_file(mode: *const c_char) -> *mut c::FILE {
    // SAFETY: caller contract; host_parms is set before Key_Init runs.
    unsafe {
        /* harness runs must not read or write real per-user state:
        COM_FOpenPrefFile redirects into the disposable gamedir while
        harness_active */
        if c::harness_active {
            return c::COM_FOpenPrefFile(HISTORY_FILE_NAME.as_ptr(), mode);
        }

        let parms = hg::host_parms;
        if (*parms).userdir != (*parms).basedir {
            return c::Sys_fopen(
                g::va(
                    c"%s/%s".as_ptr(),
                    (*parms).userdir,
                    HISTORY_FILE_NAME.as_ptr(),
                ),
                mode,
            );
        }

        let pref_path = c::Sys_GetPrefPath(c"vkQuake".as_ptr(), c"".as_ptr());
        if pref_path.is_null() {
            return c::Sys_fopen(
                g::va(
                    c"%s/%s".as_ptr(),
                    (*parms).userdir,
                    HISTORY_FILE_NAME.as_ptr(),
                ),
                mode,
            );
        }

        let hf = c::Sys_fopen(
            g::va(c"%s%s".as_ptr(), pref_path, HISTORY_FILE_NAME.as_ptr()),
            mode,
        );
        c::Mem_Free(pref_path as *const c_void);
        hf
    }
}

/// `keys.c:814`.
///
/// # Safety
/// C ABI entry point; the edit line and `host_parms` must be usable.
#[no_mangle]
pub unsafe extern "C" fn History_Init() {
    // SAFETY: the edit line is the glue-owned key_lines array; hf is a stdio
    // handle this function opens and closes.
    unsafe {
        for i in 0..CMDLINES {
            let line = key_line(i);
            *line = b']' as c_char;
            *line.add(1) = 0;
        }
        g::key_linepos = 1;

        let hf = history_open_file(c"rt".as_ptr());
        if hf.is_null() {
            return;
        }

        let mut ch;
        loop {
            let mut i = 1;
            let line = key_line(g::edit_line);
            loop {
                ch = c::stdio::fgetc(hf);
                // COMPAT: the int from fgetc is stored into a char, so EOF
                // lands as (char)-1 before the terminator below overwrites it
                // (keys.c:832).
                *line.offset(i as isize) = ch as c_char;
                i += 1;
                if ch == b'\r' as c_int || ch == b'\n' as c_int || ch == -1 || i >= MAXCMDLINE {
                    break;
                }
            }
            *line.offset(i as isize - 1) = 0;
            g::edit_line = get_history_next_line(g::edit_line);
            /* for people using a windows-generated history file on unix: */
            if ch == b'\r' as c_int || ch == b'\n' as c_int {
                loop {
                    ch = c::stdio::fgetc(hf);
                    if ch != b'\r' as c_int && ch != b'\n' as c_int {
                        break;
                    }
                }
                if ch != -1 {
                    g::ungetc(ch, hf);
                } else {
                    ch = 0; /* loop once more, otherwise last line is lost */
                }
            }
            // COMPAT: `edit_line < CMDLINES` can never be false -- edit_line
            // is always taken mod CMDLINES -- but the test is kept as written
            // (keys.c:851).
            if ch == -1 || g::edit_line >= CMDLINES {
                break;
            }
        }
        c::stdio::fclose(hf);

        g::edit_line = get_history_prev_line(g::edit_line);
        g::history_line = g::edit_line;
        let line = key_line(g::edit_line);
        *line = b']' as c_char;
        *line.add(1) = 0;
    }
}

/// `keys.c:862`.
///
/// # Safety
/// C ABI entry point; the edit line must be initialised.
#[no_mangle]
pub unsafe extern "C" fn History_Shutdown() {
    // SAFETY: the edit line is the glue-owned key_lines array; hf is a stdio
    // handle this function opens and closes.
    unsafe {
        let hf = history_open_file(c"wt".as_ptr());
        if hf.is_null() {
            return;
        }

        let mut i = g::edit_line;
        loop {
            i = get_history_next_line(i);
            if i == g::edit_line || *key_line(i).add(1) != 0 {
                break;
            }
        }

        while i != g::edit_line && *key_line(i).add(1) != 0 {
            g::fprintf(hf, c"%s\n".as_ptr(), key_line(i).add(1));
            i = get_history_next_line(i);
        }
        c::stdio::fclose(hf);
    }
}

/// `keys.c:888`.
///
/// # Safety
/// C ABI entry point; call once during bring-up.
#[no_mangle]
pub unsafe extern "C" fn Key_Init() {
    // SAFETY: consolekeys/menubound are the glue-owned MAX_KEYS arrays.
    unsafe {
        History_Init();

        g::key_blinktime = g::realtime; // johnfitz

        //
        // initialize consolekeys[]
        //
        for i in 32..127 {
            // ascii characters
            g::consolekeys[i] = true;
        }
        g::consolekeys[b'`' as usize] = false;
        g::consolekeys[b'~' as usize] = false;
        for k in [
            K_TAB,
            K_ENTER,
            K_ESCAPE,
            K_BACKSPACE,
            K_UPARROW,
            K_DOWNARROW,
            K_LEFTARROW,
            K_RIGHTARROW,
            K_CTRL,
            K_SHIFT,
            K_INS,
            K_DEL,
            K_PGDN,
            K_PGUP,
            K_HOME,
            K_END,
            K_KP_NUMLOCK,
            K_KP_SLASH,
            K_KP_STAR,
            K_KP_MINUS,
            K_KP_HOME,
            K_KP_UPARROW,
            K_KP_PGUP,
            K_KP_PLUS,
            K_KP_LEFTARROW,
            K_KP_5,
            K_KP_RIGHTARROW,
            K_KP_END,
            K_KP_DOWNARROW,
            K_KP_PGDN,
            K_KP_ENTER,
            K_KP_INS,
            K_KP_DEL,
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            K_COMMAND,
            K_MWHEELUP,
            K_MWHEELDOWN,
            K_MOUSE1,
            K_MOUSE2,
            K_MOUSE3,
        ] {
            g::consolekeys[k as usize] = true;
        }

        //
        // initialize menubound[]
        //
        g::menubound[K_ESCAPE as usize] = true;
        for i in 0..12 {
            g::menubound[(K_F1 + i) as usize] = true;
        }
        //
        // register our functions
        //
        add_command(c"bindlist".as_ptr(), Key_Bindlist_f); // johnfitz
        add_command(c"bind".as_ptr(), Key_Bind_f);
        add_command(c"unbind".as_ptr(), Key_Unbind_f);
        add_command(c"unbindall".as_ptr(), Key_Unbindall_f);
    }
}

/// `keys.c:960` -- the anonymous file-static `key_inputgrab`.
struct InputGrab {
    active: bool,
    lastkey: c_int,
    lastchar: c_int,
}

static mut KEY_INPUTGRAB: InputGrab = InputGrab {
    active: false,
    lastkey: -1,
    lastchar: -1,
};

/// `keys.c:967`.
///
/// # Safety
/// C ABI entry point; call only from `Quake/keys_glue.c`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_key_begin_input_grab() -> Raise {
    // SAFETY: KEY_INPUTGRAB replaces the C file-static of the same name.
    unsafe {
        raise!(key_clear_states());

        KEY_INPUTGRAB.active = true;
        KEY_INPUTGRAB.lastkey = -1;
        KEY_INPUTGRAB.lastchar = -1;

        g::IN_UpdateInputMode();
        0
    }
}

/// `keys.c:981`.
///
/// # Safety
/// C ABI entry point; call only from `Quake/keys_glue.c`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_key_end_input_grab() -> Raise {
    // SAFETY: KEY_INPUTGRAB replaces the C file-static of the same name.
    unsafe {
        raise!(key_clear_states());

        KEY_INPUTGRAB.active = false;

        g::IN_UpdateInputMode();
        0
    }
}

/// `keys.c:994`.
///
/// # Safety
/// `lastkey` and `lastchar` must be NULL or point at writable `int`s.
#[no_mangle]
pub unsafe extern "C" fn Key_GetGrabbedInput(lastkey: *mut c_int, lastchar: *mut c_int) {
    // SAFETY: caller contract
    unsafe {
        if !lastkey.is_null() {
            *lastkey = KEY_INPUTGRAB.lastkey;
        }
        if !lastchar.is_null() {
            *lastchar = KEY_INPUTGRAB.lastchar;
        }
    }
}

/// `keys.c:1017`.
///
/// # Safety
/// C ABI entry point; call only from `Quake/keys_glue.c`'s `Key_Event`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_key_event(key: c_int, down: bool) -> Raise {
    // SAFETY: caller contract
    unsafe { key_event_with_keycode(key, down, 0) }
}

/// `keys.c:1033`.
///
/// # Safety
/// C ABI entry point; call only from `Quake/keys_glue.c`'s
/// `Key_EventWithKeycode`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_key_event_with_keycode(
    key: c_int,
    down: bool,
    keycode: c_int,
) -> Raise {
    // SAFETY: caller contract
    unsafe { key_event_with_keycode(key, down, keycode) }
}

unsafe fn key_event_with_keycode(key: c_int, down: bool, keycode: c_int) -> Raise {
    // SAFETY: keydown/keybindings/consolekeys/menubound are glue-owned arrays
    // and `key` is range-checked below before any of them is indexed.
    unsafe {
        let mut cmd = [0u8; 1024];

        if !(0..MAX_KEYS).contains(&key) {
            return 0;
        }

        // handle fullscreen toggle
        if down && (key == K_ENTER || key == K_KP_ENTER) && keydown(K_ALT) {
            raise!(g::Keys_Glue_VidToggle());
            return 0;
        }

        // handle autorepeats and stray key up events
        if down {
            if keydown(key) && g::key_dest == KEY_GAME && !g::con_forcedup {
                return 0; // ignore autorepeats in game mode
            }
        } else if !keydown(key) {
            return 0; // ignore stray key up events
        }

        g::keydown[key as usize] = down;

        if KEY_INPUTGRAB.active {
            if down {
                KEY_INPUTGRAB.lastkey = key;
                if keycode > 0 {
                    KEY_INPUTGRAB.lastchar = keycode;
                }
            }
            return 0;
        }

        // handle escape specialy, so the user can never unbind it
        if key == K_ESCAPE {
            if !down {
                return 0;
            }

            if keydown(K_SHIFT) {
                raise!(g::Keys_Glue_ToggleConsole());
                return 0;
            }

            match g::key_dest {
                KEY_MESSAGE => Key_Message(key),
                KEY_MENU => raise!(g::Keys_Glue_MenuKeydown(key)),
                KEY_GAME | KEY_CONSOLE => raise!(g::Keys_Glue_ToggleMenu()),
                // SAFETY: Sys_Error is noreturn and does not longjmp
                _ => c::Sys_Error(c"Bad key_dest".as_ptr()),
            }

            return 0;
        }

        // key up events only generate commands if the game key binding is
        // a button command (leading + sign).  These will occur even in console mode,
        // to keep the character from continuing an action started before a console
        // switch.  Button commands include the kenum as a parameter, so multiple
        // downs can be matched with ups
        if !down {
            let kb = *keybinding(key);
            if !kb.is_null() && *kb == b'+' as c_char {
                g::q_snprintf(
                    cmd.as_mut_ptr() as *mut c_char,
                    cmd.len(),
                    c"-%s %i\n".as_ptr(),
                    kb.add(1),
                    key,
                );
                c::Cbuf_AddText(cmd.as_ptr() as *const c_char);
            }
            return 0;
        }

        // during demo playback, most keys bring up the main menu
        if cls.demoplayback
            && down
            && g::consolekeys[key as usize]
            && g::key_dest == KEY_GAME
            && key != K_TAB
        {
            cmd[0] = 0;
            let seektime = if keydown(K_SHIFT) {
                c"30".as_ptr()
            } else {
                c"10".as_ptr()
            };

            if key == K_LEFTARROW {
                g::q_snprintf(
                    cmd.as_mut_ptr() as *mut c_char,
                    cmd.len(),
                    c"seek -%s\n".as_ptr(),
                    seektime,
                );
            } else if key == K_RIGHTARROW {
                g::q_snprintf(
                    cmd.as_mut_ptr() as *mut c_char,
                    cmd.len(),
                    c"seek +%s\n".as_ptr(),
                    seektime,
                );
            } else if key == K_DOWNARROW {
                if !cls.demopaused {
                    cls.demospeed /= 2.0f32;
                    if cls.demospeed < 0.5f32 {
                        cls.demospeed = 0.0f32;
                        g::q_snprintf(
                            cmd.as_mut_ptr() as *mut c_char,
                            cmd.len(),
                            c"pause\n".as_ptr(),
                        );
                    }
                }
            } else if key == K_UPARROW {
                if cls.demospeed == 0.0f32 && cls.demopaused {
                    g::q_snprintf(
                        cmd.as_mut_ptr() as *mut c_char,
                        cmd.len(),
                        c"pause\n".as_ptr(),
                    );
                }
                if cls.demospeed == 0.0f32 || !cls.demopaused {
                    cls.demospeed = clamp_f(0.5f32, cls.demospeed * 2.0f32, 64.0f32);
                }
            } else if key != K_SHIFT {
                raise!(g::Keys_Glue_ToggleMenu());
            }

            c::Cbuf_AddText(cmd.as_ptr() as *const c_char);
            return 0;
        }

        // if not a consolekey, send to the interpreter no matter what mode is
        if (g::key_dest == KEY_MENU && g::menubound[key as usize] && !g::M_WaitingForKeyBinding())
            || (g::key_dest == KEY_CONSOLE && !g::consolekeys[key as usize])
            || (g::key_dest == KEY_GAME && (!g::con_forcedup || !g::consolekeys[key as usize]))
        {
            let kb = *keybinding(key);
            if !kb.is_null() {
                if *kb == b'+' as c_char {
                    // button commands add keynum as a parm
                    g::q_snprintf(
                        cmd.as_mut_ptr() as *mut c_char,
                        cmd.len(),
                        c"%s %i\n".as_ptr(),
                        kb,
                        key,
                    );
                    c::Cbuf_AddText(cmd.as_ptr() as *const c_char);
                } else {
                    c::Cbuf_AddText(kb);
                    c::Cbuf_AddText(c"\n".as_ptr());
                }
            }
            return 0;
        }

        if !down {
            return 0; // other systems only care about key down events
        }

        match g::key_dest {
            KEY_MESSAGE => Key_Message(key),
            KEY_MENU => raise!(g::Keys_Glue_MenuKeydown(key)),
            KEY_GAME | KEY_CONSOLE => raise!(key_console(key)),
            // SAFETY: Sys_Error is noreturn and does not longjmp
            _ => c::Sys_Error(c"Bad key_dest".as_ptr()),
        }

        0
    }
}

/// `keys.c:1215` -- called by the backend when the user has input a character.
///
/// # Safety
/// C ABI entry point; call only from `Quake/keys_glue.c`'s `Char_Event`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_char_event(key: c_int) -> Raise {
    // SAFETY: keydown is the glue-owned array; the two callees are this
    // module's cores.
    unsafe {
        if !(32..=126).contains(&key) {
            return 0;
        }

        #[cfg(any(target_os = "macos", target_os = "ios"))]
        if keydown(K_COMMAND) {
            return 0;
        }
        if keydown(K_CTRL) {
            return 0;
        }

        if KEY_INPUTGRAB.active {
            KEY_INPUTGRAB.lastchar = key;
            return 0;
        }

        match g::key_dest {
            KEY_MESSAGE => Char_Message(key),
            KEY_MENU => raise!(g::Keys_Glue_MenuCharinput(key)),
            KEY_GAME => {
                if g::con_forcedup {
                    raise!(quake_rs_char_console(key));
                }
            }
            KEY_CONSOLE => raise!(quake_rs_char_console(key)),
            _ => {}
        }

        0
    }
}

/// `keys.c:1252`.
#[no_mangle]
pub extern "C" fn Key_TextEntry() -> bool {
    // SAFETY: m_is_quitting and key_dest are C-owned; M_TextEntry only reads
    // the menu state.
    unsafe {
        if KEY_INPUTGRAB.active {
            // This path is used for simple single-letter inputs (y/n prompts) that also
            // accept controller input, so we don't want an onscreen keyboard for this case.
            return false;
        }

        // key_dest == key_console for a moment while quitting. Don't let that
        // cause SDL_StartTextInput.
        if g::m_is_quitting {
            return false;
        }

        match g::key_dest {
            KEY_MESSAGE => true,
            KEY_MENU => g::M_TextEntry(),
            // Don't return true even during con_forcedup, because that happens while starting a
            // game and we don't to trigger text input (and the onscreen keyboard on some devices)
            // during this.
            KEY_GAME => false,
            KEY_CONSOLE => true,
            _ => false,
        }
    }
}

/// `keys.c:1284`.
///
/// # Safety
/// C ABI entry point; call only from `Quake/keys_glue.c`'s `Key_ClearStates`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_key_clear_states() -> Raise {
    // SAFETY: caller contract
    unsafe { key_clear_states() }
}

unsafe fn key_clear_states() -> Raise {
    // SAFETY: keydown is the glue-owned MAX_KEYS array.
    unsafe {
        for i in 0..MAX_KEYS {
            if keydown(i) {
                raise!(key_event_with_keycode(i, false, 0));
            }
        }
        0
    }
}

/// `keys.c:1301`.
///
/// # Safety
/// C ABI entry point.
#[no_mangle]
pub unsafe extern "C" fn Key_UpdateForDest() {
    // SAFETY: key_dest is glue-owned and cls is Rust-owned (crate::cl_main);
    // IN_Activate/IN_DeactivateForConsole cannot longjmp.
    unsafe {
        if cls.state == CA_DEDICATED {
            return;
        }

        match g::key_dest {
            KEY_CONSOLE => {
                if FORCED && cls.state == CA_CONNECTED {
                    FORCED = false;
                    g::IN_Activate();
                    g::key_dest = KEY_GAME;
                }
            }
            KEY_GAME if cls.state != CA_CONNECTED => {
                FORCED = true;
                g::IN_DeactivateForConsole();
                g::key_dest = KEY_CONSOLE;
            }
            // COMPAT: key_game with cls.state == ca_connected falls through to
            // the default arm in the C (keys.c:1322), so it clears `forced`.
            _ => FORCED = false,
        }
    }
}

/// `Key_UpdateForDest`'s `static qboolean forced` (`keys.c:1303`).
static mut FORCED: bool = false;
