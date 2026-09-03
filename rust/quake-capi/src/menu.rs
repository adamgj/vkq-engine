//! `Quake/menu.c` -- the whole menu system.
//!
//! Rust migration Phase 7 M10e, Pattern A (whole-file swap): `Quake/menu.c`
//! is replaced by this module plus `Quake/menu_glue.c` under
//! `-Duse_rust_host`. Every plain `M_*` name stays in the glue; this module
//! exports only `quake_rs_menu_*` cores.
//!
//! ## ADR-009 raise-topology audit
//!
//! `menu.c` names none of `Host_Error`, `Host_EndGame`, `PR_RunError`,
//! `Sys_Error`, `PR_ExecuteProgram` or `Mod_LoadModel` -- the raise surface
//! is entirely indirect, through five callees:
//!
//! * `menu.c:485` `Con_ToggleConsole_f ()`. Under `-Duse_rust_host` the plain
//!   name is itself a `Host_Reraise` wrapper (`console_glue.c:234`), so
//!   calling it from a Rust frame would re-issue a jump across that frame.
//!   Entered through `Menu_Glue_ToggleConsole`.
//! * `menu.c:676` `CL_NextDemo ()` -- the demo loop reaches `Host_EndGame`.
//!   Entered through `Menu_Glue_NextDemo`.
//! * `menu.c:793`, `:2274` `SCR_ModalMessage (...)` and `menu.c:952`, `:4387`
//!   `SCR_BeginLoadingPlaque ()`. Both drive `SCR_UpdateScreen (false)`
//!   (`gl_screen.c:1044+`), which reaches `Mod_LoadModel (mod, true)` ->
//!   `Host_Error` (`gl_model.c:531`). Entered through
//!   `Menu_Glue_ModalMessage` / `Menu_Glue_BeginLoadingPlaque`.
//! * `menu.c:4436` `NET_Poll ()` -- it runs `pp->procedure (pp->arg)` over an
//!   arbitrary scheduled list. The existing `Host_Glue_NET_Poll`
//!   (`host_glue.c:423`, already propagated by `host.rs:1667`) is reused; no
//!   second trampoline is added.
//! * `menu.c:2284`, `:4736`, `:4860` -- the video menu. `M_Menu_Video_f`,
//!   `M_Video_Draw` and `M_Video_Key` all live in `Quake/gl_vidsdl.c`
//!   (`:5387`, `:5328`, `:5216`), not in `menu.c`, and every one of them runs
//!   `VID_SyncCvars ()`, which writes cvars and so reaches `Host_Error`
//!   through `Cvar_CallCallback`. Entered through `Menu_Glue_MenuVideo`,
//!   `Menu_Glue_VideoDraw` and `Menu_Glue_VideoKey`.
//!
//! `menu.c:4420` `NET_Slist_f ()` is **not** raise-capable and gets no guard.
//! Its whole body (`net_main.c:452-467`) is a `slistInProgress` test,
//! `Con_Printf` + `PrintSlistHeader`, `Sys_DoubleTime`, two
//! `SchedulePollProcedure` calls -- pure list insertion at
//! `net_main.c:1310+` -- and `hostCacheCount = 0`. Nothing on that path
//! longjmps except `Con_Printf`, which is the standing project exposure
//! below.
//!
//! `Draw_CachePic` is likewise not raise-capable: it ends in `Sys_Error`
//! (`gl_draw.c:428`), which ends in `exit (1)` (`sys_sdl_win.c:654`) --
//! process exit, not a longjmp. `Cbuf_AddText` / `Cbuf_InsertText` only
//! append to the command buffer; nothing runs inline.
//!
//! Accepted, pre-existing exposure: `Con_SafePrintf` (`menu.c:3038`) is left
//! plain and unguarded. `Con_Printf`'s screen-update tail is raise-capable in
//! principle, but the project's standing decision keeps it plain from Rust
//! and that exposure is documented across every ported module.
//!
//! ## Ownership (ADR-007)
//!
//! Eight of `menu.c`'s file-scope objects have live C readers, so their
//! storage stays in `Quake/menu_glue.c` and is reached through
//! [`quake_c_sys::menu`]: `m_state`, `m_return_state`, `m_entersound`,
//! `m_is_quitting`, `m_return_onerror`, `m_return_reason`, `vid_menucmdfn`
//! and `vid_menukeyfn`. The reader for each is cited in that module's doc.
//!
//! Every other file-scope object in `menu.c` had external linkage only by
//! accident -- no other translation unit names `m_save_demonum`,
//! `load_cursor`, `m_multiplayer_cursor`, `setup_cursor`,
//! `setup_cursor_table`, `setup_hostname`, `setup_myname`,
//! `m_singleplayer_cursor`, `m_filenames`, `loadable`, `setup_oldtop`,
//! `setup_oldbottom`, `setup_top`, `setup_bottom`, `m_net_cursor`,
//! `m_first_net_item`, `m_net_items`, `bindnames` or `help_page` -- so all of
//! them move here alongside the file's statics.
//!
//! ## ADR-005
//!
//! Every menu format string goes through the C `va` / `q_snprintf`, never the
//! Rust formatter. `menu.c:2038` formats with `%g`
//! (`va ("on (%gx)", ...)`), which the ADR-005 subset does not implement and
//! deliberately panics on; the precision forms `%.0f`, `%.1f`, `%.2f`,
//! `%.0f%%`, `%ix` and `1/%i` are used throughout the options menus. Keeping
//! the whole family on libc's formatter keeps the emitted bytes identical and
//! keeps a panic from ever reaching an `extern "C"` boundary.

use core::ffi::{c_char, c_float, c_int, c_uint, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::menu as g;
use quake_types::host::{ClientState, ClientStatic, CA_CONNECTED};

/// A `Host_Guard` status: 0 means the guarded call returned normally, any
/// other value is a pending longjmp that must reach a C frame untouched.
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
    static mut cls: ClientStatic;
    static mut cl: ClientState;
}

/* ------------------------------------------------------------------------
 * Constants transcribed from the headers menu.c includes.
 */

/// `draw.h:26` -- `#define CHARACTER_SIZE 8`.
const CHARACTER_SIZE: c_int = 8;

/// `menu.h:88-94` -- the menu layout grid.
const MENU_TOP: c_int = 40;
const MENU_CURSOR_X: c_int = 60;
const MENU_LABEL_X: c_int = 70;
const MENU_VALUE_X: c_int = 204;
const MENU_SLIDER_X: c_int = MENU_VALUE_X + 6;
const MENU_SCROLLBAR_X: c_int = 312 + 48;
const MAX_MENU_LINES: c_int = 14;

/// `menu.h:97-99` -- the max-fps slider bounds.
const MIN_FPS_MENU_VALUE: f32 = 10.0;
const FPS_MENU_VALUE_STEP: f32 = 2.0;
const MAX_FPS_MENU_VALUE: f32 = 1000.0;

/// `menu.c:359-362` -- the slider geometry.
const SLIDER_SIZE: c_int = 10;
const SLIDER_EXTENT: c_int = (SLIDER_SIZE - 1) * 8;
const SLIDER_START: c_int = MENU_SLIDER_X + 4;
/// `menu.c:371` -- `#define SLIDER_END (SLIDER_START + SLIDER_EXTENT)`.
const SLIDER_END: c_int = SLIDER_START + SLIDER_EXTENT;

/// `keys.h:143-149` -- `keydest_t`.
const KEY_GAME: c_int = 0;
const KEY_CONSOLE: c_int = 1;
const KEY_MENU: c_int = 3;

/// `menu.h:26-51` -- `enum m_state_e`.
const M_NONE: c_int = 0;
const M_MAIN: c_int = 1;
const M_SINGLEPLAYER: c_int = 2;
const M_LOAD: c_int = 3;
const M_SAVE: c_int = 4;
const M_MULTIPLAYER: c_int = 5;
const M_SETUP: c_int = 6;
const M_NET: c_int = 7;
const M_OPTIONS: c_int = 8;
const M_GAME: c_int = 9;
const M_SOUND: c_int = 10;
const M_VIDEO: c_int = 11;
const M_GRAPHICS: c_int = 12;
const M_KEYS: c_int = 13;
const M_HELP: c_int = 14;
const M_QUIT: c_int = 15;
const M_LANCONFIG: c_int = 16;
const M_MPGAMEOPTIONS: c_int = 17;
const M_SEARCH: c_int = 18;
const M_SLIST: c_int = 19;
const M_MODS: c_int = 20;
const M_MAPS: c_int = 21;
const M_SKILL: c_int = 22;

// `keys.h:32-124` -- keycode_t. Spelled here rather than exported so cbindgen
// never emits a second set of definitions into quake_rs.h.
const K_ENTER: c_int = 13;
const K_ESCAPE: c_int = 27;
const K_SPACE: c_int = 32;
const K_BACKSPACE: c_int = 127;
const K_UPARROW: c_int = 128;
const K_DOWNARROW: c_int = 129;
const K_LEFTARROW: c_int = 130;
const K_RIGHTARROW: c_int = 131;
const K_CTRL: c_int = 133;
const K_DEL: c_int = 148;
const K_PGDN: c_int = 149;
const K_PGUP: c_int = 150;
const K_HOME: c_int = 151;
const K_END: c_int = 152;
const K_KP_HOME: c_int = 157;
const K_KP_UPARROW: c_int = 158;
const K_KP_PGUP: c_int = 159;
const K_KP_END: c_int = 164;
const K_KP_DOWNARROW: c_int = 165;
const K_KP_PGDN: c_int = 166;
const K_KP_ENTER: c_int = 167;
#[cfg(any(target_os = "macos", target_os = "ios"))]
const K_COMMAND: c_int = 170;
const K_MOUSE1: c_int = 200;
const K_MOUSE2: c_int = 201;
const K_MOUSE4: c_int = 203;
const K_MWHEELUP: c_int = 205;
const K_MWHEELDOWN: c_int = 206;
const K_ABUTTON: c_int = 211;
const K_BBUTTON: c_int = 212;

/* ------------------------------------------------------------------------
 * Small helpers.
 */

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

/// `Quake/q_minmax.h` `CLAMP` at `double` -- reached wherever one of the
/// three arguments is a C double literal and `_Generic` picks `clamp_d`.
#[inline]
fn clamp_f64(minval: f64, val: f64, maxval: f64) -> f64 {
    if val < minval {
        minval
    } else if val > maxval {
        maxval
    } else {
        val
    }
}

/// `Quake/q_minmax.h` `q_min` at `int`.
#[inline]
fn min_i(a: c_int, b: c_int) -> c_int {
    if a < b {
        a
    } else {
        b
    }
}

/// `Quake/q_minmax.h` `q_min` at `size_t`. `_Generic` resolves `q_min (int,
/// size_t)` to the 64-bit unsigned overload on every platform the engine
/// builds for (LLP64 picks `q_min_ull`, LP64 picks `q_min_ul`).
#[inline]
fn min_u64(a: u64, b: u64) -> u64 {
    if a < b {
        a
    } else {
        b
    }
}

/// `Quake/q_minmax.h` `q_max` at `size_t`; see [`min_u64`].
#[inline]
fn max_u64(a: u64, b: u64) -> u64 {
    if a > b {
        a
    } else {
        b
    }
}

/// `Quake/q_minmax.h` `q_max` at `int`.
#[inline]
fn max_i(a: c_int, b: c_int) -> c_int {
    if a > b {
        a
    } else {
        b
    }
}

/// `Quake/q_minmax.h` `q_max` at `double`.
#[inline]
fn max_f64(a: f64, b: f64) -> f64 {
    if a > b {
        a
    } else {
        b
    }
}

/// `wad.h:52-56` -- `pic->width`. Called only on pointers the renderer just
/// returned.
///
/// # Safety
/// `pic` must be a live `qpic_t *`.
unsafe fn pic_width(pic: *mut c_void) -> c_int {
    // SAFETY: caller contract; `width` is the first member.
    unsafe { ptr::addr_of!((*pic.cast::<g::qpic_t>()).width).read() }
}

/// `common.h:132` -- `VEC_SIZE`.
///
/// # Safety
/// `v` must be null or a `VEC_*` payload pointer.
#[inline]
unsafe fn vec_size<T>(v: *const T) -> usize {
    // SAFETY: the caller guarantees the pointer contract documented above.
    unsafe {
        if v.is_null() {
            0
        } else {
            (*v.cast::<g::vec_header_t>().sub(1)).size
        }
    }
}

/// `common.h:126` -- `VEC_PUSH` (`Vec_Grow` plus a direct write, *not*
/// `Vec_Append`).
///
/// # Safety
/// `pv` must point at a `VEC_*` head of element type `T`.
#[inline]
unsafe fn vec_push<T>(pv: *mut *mut T, value: T) {
    // SAFETY: the caller guarantees the pointer contract documented above.
    unsafe {
        g::Vec_Grow(pv.cast::<*mut c_void>(), core::mem::size_of::<T>(), 1);
        let v = *pv;
        let hdr = &mut *v.cast::<g::vec_header_t>().sub(1);
        v.add(hdr.size).write(value);
        hdr.size += 1;
    }
}

/// `(int)f` with C's truncate-toward-zero conversion.
///
/// // COMPAT: ADR-004 -- C's float-to-int conversion is undefined when the
/// value does not fit; Rust's `as` saturates instead. Every menu site feeds
/// this a value already clamped into range, so saturation is unobservable and
/// no UB is committed either way.
#[inline]
fn as_i(f: f32) -> c_int {
    f as c_int
}

/// `(int)d` on a C `double`; see [`as_i`].
#[inline]
fn as_i_d(d: f64) -> c_int {
    d as c_int
}

/// `sv.active` (`Quake/server.h`). The server struct is Rust-owned under the
/// `host` feature (`sv_main.rs:91`), so this reads it through the sibling
/// module rather than an ADR-011 extern.
///
/// # Safety
/// Reads the single-threaded server state.
#[inline]
unsafe fn sv_active() -> bool {
    // SAFETY: scalar read of the single-threaded server state.
    unsafe { ptr::addr_of!(crate::sv_main::sv.active).read() }
}

/// `svs.maxclients` (`Quake/server.h`); see [`sv_active`].
///
/// # Safety
/// Reads the single-threaded server state.
#[inline]
unsafe fn svs_maxclients() -> c_int {
    // SAFETY: scalar read of the single-threaded server state.
    unsafe { ptr::addr_of!(crate::sv_main::svs.maxclients).read() }
}

/// `menu.c:4245` -- `svs.maxclientslimit`.
///
/// # Safety
/// Reads the single-threaded server state.
#[inline]
unsafe fn svs_maxclientslimit() -> c_int {
    // SAFETY: scalar read of the single-threaded server state.
    unsafe { ptr::addr_of!(crate::sv_main::svs.maxclientslimit).read() }
}

/// A `&str` literal as a NUL-terminated C string pointer.
macro_rules! cstr {
    ($s:literal) => {
        concat!($s, "\0").as_ptr().cast::<c_char>()
    };
}

/* ------------------------------------------------------------------------
 * File-scope state that moves to Rust (see the module doc's ADR-007 note).
 */

/// `menu.c:86` -- `static qboolean m_recursiveDraw`.
static mut M_RECURSIVE_DRAW: bool = false;
/// `menu.c:96` -- `static int m_main_cursor`.
static mut M_MAIN_CURSOR: c_int = 0;
/// `menu.c:97` -- `static qboolean m_mouse_moved`.
static mut M_MOUSE_MOVED: bool = false;
/// `menu.c:98` -- `static qboolean menu_changed`.
static mut MENU_CHANGED: bool = false;
/// `menu.c:99-102` -- the mouse position in menu-canvas and pixel space.
static mut M_MOUSE_X: c_int = -1;
static mut M_MOUSE_Y: c_int = -1;
static mut M_MOUSE_X_PIXELS: c_int = -1;
static mut M_MOUSE_Y_PIXELS: c_int = -1;
/// `menu.c:104-106` -- the scrollbar hit box the last `M_DrawScrollbar` set.
static mut SCROLLBAR_X: c_int = 0;
static mut SCROLLBAR_Y: c_int = 0;
static mut SCROLLBAR_SIZE: c_int = 0;
/// `menu.c:139-140` -- the two drag latches.
static mut SLIDER_GRAB: bool = false;
static mut SCROLLBAR_GRAB: bool = false;
/// `menu.c:177` -- `M_GetScale`'s function-local `static float
/// latched_menuscale`.
static mut LATCHED_MENUSCALE: f32 = 0.0;
/// `menu.c:459` -- `int m_save_demonum;`. Externally linked in C but named by
/// no other translation unit.
static mut M_SAVE_DEMONUM: c_int = 0;

/// `menu.c:144-157` -- `static const crosshair_t crosshair_defs[]`.
static CROSSHAIR_DEFS: [g::crosshair_t; 6] = [
    g::crosshair_t {
        crosshair_char: b'+' as c_char,
        viewport_x_offset: -(CHARACTER_SIZE as f32) * 0.5,
        viewport_y_offset: -(CHARACTER_SIZE as f32) * 0.5,
        menu_x_offset: 0,
        menu_y_offset: 0,
    },
    g::crosshair_t {
        crosshair_char: b'.' as c_char,
        viewport_x_offset: -(CHARACTER_SIZE as f32) * 0.5 + 1.75,
        viewport_y_offset: -(CHARACTER_SIZE as f32) * 0.5 - 1.5,
        menu_x_offset: 2,
        menu_y_offset: -1,
    },
    g::crosshair_t {
        crosshair_char: b'x' as c_char,
        viewport_x_offset: -(CHARACTER_SIZE as f32) * 0.5,
        viewport_y_offset: -(CHARACTER_SIZE as f32) * 0.5,
        menu_x_offset: 0,
        menu_y_offset: 0,
    },
    g::crosshair_t {
        crosshair_char: b'o' as c_char,
        viewport_x_offset: -(CHARACTER_SIZE as f32) * 0.5,
        viewport_y_offset: -(CHARACTER_SIZE as f32) * 0.5,
        menu_x_offset: 0,
        menu_y_offset: 0,
    },
    g::crosshair_t {
        crosshair_char: b'^' as c_char,
        viewport_x_offset: -(CHARACTER_SIZE as f32) * 0.5,
        viewport_y_offset: -(CHARACTER_SIZE as f32) * 0.5 + 5.0,
        menu_x_offset: 0,
        menu_y_offset: 2,
    },
    g::crosshair_t {
        crosshair_char: b'v' as c_char,
        viewport_x_offset: -(CHARACTER_SIZE as f32) * 0.5 - 0.65,
        viewport_y_offset: -(CHARACTER_SIZE as f32) * 0.5 + 0.75,
        menu_x_offset: 0,
        menu_y_offset: 0,
    },
];

/* ------------------------------------------------------------------------
 * menu.c:165-201 -- scale and coordinate helpers.
 */

/// `menu.c:165` -- `crosshair_t M_GetCrosshairDef (float)`.
///
/// The value is returned through `out` rather than by value: the glue owns
/// the plain name and cbindgen has no spelling for a by-value `crosshair_t`.
///
/// # Safety
/// `out` must point at a writable `crosshair_t`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_get_crosshair_def(
    crosshair_def_value: c_float,
    out: *mut g::crosshair_t,
) {
    // COMPAT: ADR-004 -- `num_crosshair_defs` is a `size_t`, so C's
    // `(int)crosshair_def_value % num_crosshair_defs` promotes the int to
    // unsigned 64-bit before the modulo: a negative cvar wraps rather than
    // indexing out of bounds. That wrap is reproduced exactly here (`as i64
    // as u64`), which is why the result is always in range. The only genuine
    // UB left is C's out-of-range float-to-int conversion, which `as_i`
    // replaces with Rust's saturation; every value C defines behaviour on is
    // unaffected.
    let i = as_i(crosshair_def_value) as i64 as u64;
    let i = (i % CROSSHAIR_DEFS.len() as u64) as usize;
    // SAFETY: caller contract; `i` is in bounds by construction.
    unsafe { *out = CROSSHAIR_DEFS[i] };
}

/// `menu.c:175` -- `float M_GetScale ()`.
///
/// # Safety
/// The engine must be initialized; reads the menu latch and `scr_menuscale`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_get_scale() -> c_float {
    // SAFETY: single-threaded menu state plus a registered cvar.
    unsafe {
        if !SLIDER_GRAB {
            LATCHED_MENUSCALE = g::scr_menuscale.value;
        }
        LATCHED_MENUSCALE
    }
}

/// `menu.c:188` -- `static void M_PixelToMenuCanvasCoord (int *x, int *y)`.
///
/// # Safety
/// `x` and `y` must point at writable `int`s.
unsafe fn m_pixel_to_menu_canvas_coord(x: *mut c_int, y: *mut c_int) {
    // SAFETY: caller contract; glwidth/glheight are renderer-owned scalars.
    unsafe {
        // `320.0` and `200.0` are C doubles, so the two ratios and the
        // q_min are evaluated at double precision and only then narrowed by
        // the assignment to `float s` -- reproduced literally so the
        // narrowing rounds the same way.
        let a = g::glwidth as f32 as f64 / 320.0;
        let b = g::glheight as f32 as f64 / 200.0;
        let s = (if a < b { a } else { b }) as f32;
        // CLAMP promotes to double here as well (`1.0` is a double).
        let s = clamp_f64(1.0, quake_rs_menu_get_scale() as f64, s as f64) as f32;
        *x = ((*x as f32 - (g::glwidth as f32 - 320.0 * s) / 2.0) / s) as c_int;
        *y = ((*y as f32 - (g::glheight as f32 - 200.0 * s) / 2.0) / s) as c_int;
    }
}

/* ------------------------------------------------------------------------
 * menu.c:203-300 -- text and pic primitives.
 */

/// `menu.c:203` -- `static void M_PrintHighlighted (cb_context_t *, int, int,
/// const char *)`.
///
/// # Safety
/// `str` must be a NUL-terminated C string; `cbx` a live `cb_context_t *`.
unsafe fn m_print_highlighted(cbx: *mut c_void, mut cx: c_int, cy: c_int, mut str_: *const c_char) {
    // SAFETY: caller contract.
    unsafe {
        while *str_ != 0 {
            g::Draw_Character(cbx, cx as c_float, cy as c_float, *str_ as c_int);
            str_ = str_.add(1);
            cx += CHARACTER_SIZE;
        }
    }
}

/// `menu.c:218` -- `void M_Print (cb_context_t *, int, int, const char *)`.
///
/// # Safety
/// `str` must be a NUL-terminated C string; `cbx` a live `cb_context_t *`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_print(
    cbx: *mut c_void,
    mut cx: c_int,
    cy: c_int,
    mut str_: *const c_char,
) {
    // SAFETY: caller contract. `*str_ as c_int` reproduces C's `(*str) + 128`
    // on a plain `char`, sign included -- the source of Quake's brown menu
    // text for bytes that already have the high bit set.
    unsafe {
        while *str_ != 0 {
            g::Draw_Character(cbx, cx as c_float, cy as c_float, *str_ as c_int + 128);
            str_ = str_.add(1);
            cx += CHARACTER_SIZE;
        }
    }
}

/// `menu.c:233` -- `static void M_PrintElided (cb_context_t *, int, int, const
/// char *, const int)`.
///
/// # Safety
/// `str` must be a NUL-terminated C string; `cbx` a live `cb_context_t *`.
unsafe fn m_print_elided(
    cbx: *mut c_void,
    mut cx: c_int,
    cy: c_int,
    str_: *const c_char,
    max_length: c_int,
) {
    // SAFETY: caller contract.
    unsafe {
        let mut i: c_int = 0;
        while *str_.offset(i as isize) != 0 && i < max_length {
            g::Draw_Character(
                cbx,
                cx as c_float,
                cy as c_float,
                *str_.offset(i as isize) as c_int + 128,
            );
            i += 1;
            cx += CHARACTER_SIZE;
        }
        if *str_.offset(i as isize) != 0 {
            for _ in 0..3 {
                g::Draw_Character(cbx, cx as c_float, cy as c_float, b'.' as c_int + 128);
                cx += CHARACTER_SIZE / 2;
            }
        }
    }
}

/// `menu.c:257` -- `static void M_PrintWhite (cb_context_t *, int, int, const
/// char *)`.
///
/// # Safety
/// `str` must be a NUL-terminated C string; `cbx` a live `cb_context_t *`.
unsafe fn m_print_white(cbx: *mut c_void, mut cx: c_int, cy: c_int, mut str_: *const c_char) {
    // SAFETY: caller contract.
    unsafe {
        while *str_ != 0 {
            g::Draw_Character(cbx, cx as c_float, cy as c_float, *str_ as c_int);
            str_ = str_.add(1);
            cx += CHARACTER_SIZE;
        }
    }
}

/// `menu.c:272` -- `void M_DrawTransPic (cb_context_t *, int, int, qpic_t *)`.
///
/// # Safety
/// `cbx` and `pic` must be live `cb_context_t *` / `qpic_t *`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_draw_trans_pic(
    cbx: *mut c_void,
    x: c_int,
    y: c_int,
    pic: *mut c_void,
) {
    // SAFETY: caller contract.
    unsafe { g::Draw_Pic(cbx, x as c_float, y as c_float, pic, 1.0, false) }
}

/// `menu.c:282` -- `void M_DrawPic (cb_context_t *, int, int, qpic_t *)`.
///
/// # Safety
/// `cbx` and `pic` must be live `cb_context_t *` / `qpic_t *`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_draw_pic(
    cbx: *mut c_void,
    x: c_int,
    y: c_int,
    pic: *mut c_void,
) {
    // SAFETY: caller contract.
    unsafe { g::Draw_Pic(cbx, x as c_float, y as c_float, pic, 1.0, false) }
}

/// `menu.c:292` -- `static void M_DrawTransPicTranslate (cb_context_t *, int,
/// int, qpic_t *, int, int)`.
///
/// # Safety
/// `cbx` and `pic` must be live `cb_context_t *` / `qpic_t *`.
unsafe fn m_draw_trans_pic_translate(
    cbx: *mut c_void,
    x: c_int,
    y: c_int,
    pic: *mut c_void,
    top: c_int,
    bottom: c_int,
) {
    // SAFETY: caller contract.
    unsafe { g::Draw_TransPicTranslate(cbx, x as c_float, y as c_float, pic, top, bottom) }
}

/// `menu.c:302` -- `static void M_DrawTextBox (cb_context_t *, int, int, int,
/// int)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_draw_text_box(cbx: *mut c_void, x: c_int, y: c_int, mut width: c_int, lines: c_int) {
    // SAFETY: caller contract; every path name is a static literal.
    unsafe {
        // draw left side
        let mut cx = x;
        let mut cy = y;
        let mut p = g::Draw_CachePic(cstr!("gfx/box_tl.lmp"));
        quake_rs_menu_draw_trans_pic(cbx, cx, cy, p);
        p = g::Draw_CachePic(cstr!("gfx/box_ml.lmp"));
        for _ in 0..lines {
            cy += 8;
            quake_rs_menu_draw_trans_pic(cbx, cx, cy, p);
        }
        p = g::Draw_CachePic(cstr!("gfx/box_bl.lmp"));
        quake_rs_menu_draw_trans_pic(cbx, cx, cy + 8, p);

        // draw middle
        cx += 8;
        while width > 0 {
            cy = y;
            p = g::Draw_CachePic(cstr!("gfx/box_tm.lmp"));
            quake_rs_menu_draw_trans_pic(cbx, cx, cy, p);
            p = g::Draw_CachePic(cstr!("gfx/box_mm.lmp"));
            for n in 0..lines {
                cy += 8;
                if n == 1 {
                    p = g::Draw_CachePic(cstr!("gfx/box_mm2.lmp"));
                }
                quake_rs_menu_draw_trans_pic(cbx, cx, cy, p);
            }
            p = g::Draw_CachePic(cstr!("gfx/box_bm.lmp"));
            quake_rs_menu_draw_trans_pic(cbx, cx, cy + 8, p);
            width -= 2;
            cx += 16;
        }

        // draw right side
        cy = y;
        p = g::Draw_CachePic(cstr!("gfx/box_tr.lmp"));
        quake_rs_menu_draw_trans_pic(cbx, cx, cy, p);
        p = g::Draw_CachePic(cstr!("gfx/box_mr.lmp"));
        for _ in 0..lines {
            cy += 8;
            quake_rs_menu_draw_trans_pic(cbx, cx, cy, p);
        }
        p = g::Draw_CachePic(cstr!("gfx/box_br.lmp"));
        quake_rs_menu_draw_trans_pic(cbx, cx, cy + 8, p);
    }
}

/// `menu.c:362` -- `void M_MenuChanged ()`.
///
/// # Safety
/// Writes the glue-owned `m_entersound` and the menu-local change flag.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_menu_changed() {
    // SAFETY: single-threaded menu state.
    unsafe {
        g::m_entersound = true;
        MENU_CHANGED = true;
    }
}

/// `menu.c:378` -- `static void M_DrawSlider (cb_context_t *, int, int, float,
/// const char *)`.
///
/// # Safety
/// `label` must be a NUL-terminated C string; `cbx` a live `cb_context_t *`.
unsafe fn m_draw_slider(
    cbx: *mut c_void,
    x: c_int,
    y: c_int,
    value: c_float,
    label: *const c_char,
) {
    // SAFETY: caller contract.
    unsafe {
        let value = clamp_f(0.0, value, 1.0);
        g::Draw_Character(cbx, (x - CHARACTER_SIZE) as c_float, y as c_float, 128);

        for i in 0..SLIDER_SIZE {
            g::Draw_Character(cbx, (x + i * CHARACTER_SIZE) as c_float, y as c_float, 129);
        }

        g::Draw_Character(
            cbx,
            (x + SLIDER_SIZE * CHARACTER_SIZE) as c_float,
            y as c_float,
            130,
        );
        g::Draw_Character(
            cbx,
            x as c_float + ((SLIDER_SIZE - 1) * CHARACTER_SIZE) as c_float * value,
            y as c_float,
            131,
        );

        quake_rs_menu_print(cbx, x + (SLIDER_SIZE + 1) * CHARACTER_SIZE, y, label);
    }
}

/// `menu.c:398` -- `static float M_GetSliderPos (...)`.
#[allow(clippy::too_many_arguments)]
fn m_get_slider_pos(
    low: c_float,
    high: c_float,
    current: c_float,
    backward: bool,
    mouse: bool,
    clamped_mouse: c_float,
    dir: c_int,
    step: c_float,
    snap_start: c_float,
) -> c_float {
    let mut f: c_float;

    if mouse {
        if backward {
            f = high
                + (low - high) * (clamped_mouse - SLIDER_START as c_float)
                    / SLIDER_EXTENT as c_float;
        } else {
            f = low
                + (high - low) * (clamped_mouse - SLIDER_START as c_float)
                    / SLIDER_EXTENT as c_float;
        }
    } else if backward {
        f = current - dir as c_float * step;
    } else {
        f = current + dir as c_float * step;
    }
    if !mouse || f > snap_start {
        f = as_i(f / step + 0.5) as c_float * step;
    }
    if f < low {
        f = low;
    } else if f > high {
        f = high;
    }

    f
}

/// `menu.c:431` -- `static void M_DrawScrollbar (cb_context_t *, int, int,
/// float, float)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_draw_scrollbar(cbx: *mut c_void, x: c_int, y: c_int, value: c_float, size: c_float) {
    // SAFETY: caller contract plus single-threaded menu state.
    unsafe {
        SCROLLBAR_X = x;
        SCROLLBAR_Y = y - 8;
        SCROLLBAR_SIZE = as_i((size + 2.0) * CHARACTER_SIZE as c_float);
        let value = clamp_f(0.0, value, 1.0);
        g::Draw_Character(
            cbx,
            x as c_float,
            (y - CHARACTER_SIZE) as c_float,
            128 + 256,
        );
        // C's `for (int i = 0; i < size; i++)` promotes `i` to float for the
        // comparison; kept literal so a fractional `size` iterates the same.
        let mut i: c_int = 0;
        while (i as c_float) < size {
            g::Draw_Character(
                cbx,
                x as c_float,
                y as c_float + (i * CHARACTER_SIZE) as c_float,
                129 + 256,
            );
            i += 1;
        }
        g::Draw_Character(
            cbx,
            x as c_float,
            y as c_float + size * CHARACTER_SIZE as c_float,
            130 + 256,
        );
        g::Draw_Character(
            cbx,
            x as c_float,
            y as c_float + (size - 1.0) * CHARACTER_SIZE as c_float * value,
            131 + 256,
        );
    }
}

/// `menu.c:449` -- `static void M_DrawCheckbox (cb_context_t *, int, int,
/// int)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_draw_checkbox(cbx: *mut c_void, x: c_int, y: c_int, on: c_int) {
    // SAFETY: caller contract.
    unsafe {
        if on != 0 {
            quake_rs_menu_print(cbx, x, y, cstr!("on"));
        } else {
            quake_rs_menu_print(cbx, x, y, cstr!("off"));
        }
    }
}

/* ------------------------------------------------------------------------
 * menu.c:466-612 -- the menu toggle, the scrollbar and the mouse cursor
 * helpers every list menu shares.
 */

/// `menu.c:466` -- `void M_ToggleMenu_f (void)`.
///
/// # Safety
/// Touches glue-owned menu state and the input subsystem.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_toggle_menu_f() -> Raise {
    // SAFETY: single-threaded menu state.
    unsafe {
        quake_rs_menu_menu_changed();

        if g::key_dest == KEY_MENU {
            if g::m_state != M_MAIN {
                quake_rs_menu_menu_main_f();
                return 0;
            }

            g::IN_Activate();
            g::key_dest = KEY_GAME;
            g::m_state = M_NONE;
            return 0;
        }
        if g::key_dest == KEY_CONSOLE {
            raise!(g::Menu_Glue_ToggleConsole());
        } else {
            quake_rs_menu_menu_main_f();
        }
    }
    0
}

/// `menu.c:498` -- `static qboolean M_InScrollbar ()`.
///
/// # Safety
/// Reads the menu-local scrollbar hit box and mouse position.
unsafe fn m_in_scrollbar() -> bool {
    // SAFETY: single-threaded menu state. The duplicated final term is
    // menu.c's own copy-paste; it is kept so the expression matches
    // one-for-one.
    unsafe {
        SCROLLBAR_GRAB
            || (SCROLLBAR_SIZE != 0
                && M_MOUSE_X >= SCROLLBAR_X
                && M_MOUSE_X <= SCROLLBAR_X + 8
                && M_MOUSE_Y >= SCROLLBAR_Y
                && M_MOUSE_Y <= SCROLLBAR_Y + SCROLLBAR_SIZE
                && M_MOUSE_Y <= SCROLLBAR_Y + SCROLLBAR_SIZE)
    }
}

/// `menu.c:509` -- `qboolean M_HandleScrollBarKeys (const int, int *, int *,
/// const int, const int)`.
///
/// # Safety
/// `cursor` and `first_drawn` must point at writable `int`s.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_handle_scroll_bar_keys(
    key: c_int,
    cursor: *mut c_int,
    first_drawn: *mut c_int,
    num_total: c_int,
    max_on_screen: c_int,
) -> c::qboolean {
    // SAFETY: caller contract plus single-threaded menu state.
    unsafe {
        let prev_cursor = *cursor;
        let mut handled_mouse = false;

        if num_total == 0 {
            *cursor = 0;
            *first_drawn = 0;
            return false;
        }

        match key {
            K_MOUSE1 => {
                if m_in_scrollbar() && (num_total - max_on_screen) > 0 && !SLIDER_GRAB {
                    handled_mouse = true;
                    SCROLLBAR_GRAB = true;
                    let clamped_mouse =
                        clamp_i(SCROLLBAR_Y + 8, M_MOUSE_Y, SCROLLBAR_Y + SCROLLBAR_SIZE - 8);
                    *first_drawn = as_i(
                        (clamped_mouse as c_float - SCROLLBAR_Y as c_float - 8.0)
                            / (SCROLLBAR_SIZE - 16) as c_float
                            * (num_total - max_on_screen) as c_float
                            + 0.5,
                    );
                    if *cursor < *first_drawn {
                        *cursor = *first_drawn;
                    } else if *cursor >= *first_drawn + max_on_screen {
                        *cursor = *first_drawn + max_on_screen - 1;
                    }
                }
            }

            K_HOME => {
                *cursor = 0;
                *first_drawn = 0;
            }

            K_END => {
                *cursor = num_total - 1;
                *first_drawn = num_total - max_on_screen;
            }

            K_PGUP => {
                *cursor = max_i(0, *cursor - max_on_screen);
                *first_drawn = max_i(0, *first_drawn - max_on_screen);
            }

            K_PGDN => {
                *cursor = min_i(num_total - 1, *cursor + max_on_screen);
                *first_drawn = min_i(*first_drawn + max_on_screen, num_total - max_on_screen);
            }

            K_UPARROW => {
                if *cursor == 0 {
                    *cursor = num_total - 1;
                } else {
                    *cursor -= 1;
                }
            }

            K_DOWNARROW => {
                if *cursor == num_total - 1 {
                    *cursor = 0;
                } else {
                    *cursor += 1;
                }
            }

            K_MWHEELUP => {
                *first_drawn = max_i(0, *first_drawn - 1);
                *cursor = min_i(*cursor, *first_drawn + max_on_screen - 1);
            }

            K_MWHEELDOWN => {
                *first_drawn = min_i(*first_drawn + 1, num_total - max_on_screen);
                *cursor = max_i(*cursor, *first_drawn);
            }

            _ => {}
        }

        if *cursor != prev_cursor {
            g::S_LocalSound(cstr!("misc/menu1.wav"));
        }

        if num_total <= max_on_screen {
            *first_drawn = 0;
        } else {
            *first_drawn = clamp_i(*cursor - max_on_screen + 1, *first_drawn, *cursor);
        }

        handled_mouse
    }
}

/// `menu.c:598` -- `static void M_Mouse_UpdateListCursor (int *, int, int,
/// int, int, int, int)`.
///
/// # Safety
/// `cursor` must point at a writable `int`.
#[allow(clippy::too_many_arguments)]
unsafe fn m_mouse_update_list_cursor(
    cursor: *mut c_int,
    left: c_int,
    right: c_int,
    top: c_int,
    item_height: c_int,
    num_items: c_int,
    scroll_offset: c_int,
) {
    // SAFETY: caller contract plus single-threaded menu state. `item_height`
    // is a positive literal at every call site, so the division cannot trap.
    unsafe {
        if !SCROLLBAR_GRAB
            && !SLIDER_GRAB
            && M_MOUSE_MOVED
            && num_items > 0
            && M_MOUSE_X >= left
            && M_MOUSE_X <= right
            && M_MOUSE_Y >= top
            && M_MOUSE_Y <= top + item_height * num_items
        {
            *cursor = scroll_offset + clamp_i(0, (M_MOUSE_Y - top) / item_height, num_items - 1);
        }
    }
}

/// `menu.c:610` -- `void M_Mouse_UpdateCursor (int *, int, int, int, int,
/// int)`.
///
/// # Safety
/// `cursor` must point at a writable `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_mouse_update_cursor(
    cursor: *mut c_int,
    left: c_int,
    right: c_int,
    top: c_int,
    item_height: c_int,
    index: c_int,
) {
    // SAFETY: caller contract plus single-threaded menu state.
    unsafe {
        if M_MOUSE_MOVED
            && M_MOUSE_X >= left
            && M_MOUSE_X <= right
            && M_MOUSE_Y >= top
            && M_MOUSE_Y <= top + item_height
        {
            *cursor = index;
        }
    }
}

/* ------------------------------------------------------------------------
 * menu.c:616-722 -- MAIN MENU.
 */

/// `menu.c:619` -- `#define MAIN_ITEMS 5`.
const MAIN_ITEMS: c_int = 5;

/// `menu.c:621` -- `void M_Menu_Main_f (void)`.
///
/// Provably non-raising: it only touches `cls.demonum`, the input state and
/// the two menu-state words. The existing `Console_Glue_MenuMain` guard
/// (`console_glue.c:145`) is left in place and simply never fires.
///
/// # Safety
/// Touches glue-owned menu state, `cls` and the input subsystem.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_menu_main_f() {
    // SAFETY: single-threaded menu and client state.
    unsafe {
        quake_rs_menu_menu_changed();
        if g::key_dest != KEY_MENU {
            M_SAVE_DEMONUM = cls.demonum;
            cls.demonum = -1;
        }
        g::IN_Deactivate(true);
        g::key_dest = KEY_MENU;
        g::m_state = M_MAIN;
    }
}

/// `menu.c:634` -- `static qpic_t *Get_Menu2 ()`.
///
/// # Safety
/// Calls the renderer's pic cache.
unsafe fn get_menu2() -> *mut c_void {
    // SAFETY: `COM_GetGameNames` always returns a NUL-terminated string.
    unsafe {
        let base_game = *g::COM_GetGameNames(false) == 0;
        // Check if user has actually installed vkquake.pak, otherwise fall
        // back to old menu
        if base_game && c::registered.value != 0.0 {
            g::Draw_TryCachePic(
                cstr!("gfx/mainmenu2.lmp"),
                g::TEXPREF_ALPHA | g::TEXPREF_PAD | g::TEXPREF_NOPICMIP,
                g::PICFLAG_AUTO,
            )
        } else {
            ptr::null_mut()
        }
    }
}

/// `menu.c:641` -- `void M_Main_Draw (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_main_draw(cbx: *mut c_void) {
    // SAFETY: caller contract plus single-threaded menu state.
    unsafe {
        let menu2 = get_menu2();
        let main_items = MAIN_ITEMS + if !menu2.is_null() { 1 } else { 0 };

        quake_rs_menu_draw_trans_pic(cbx, 16, 4, g::Draw_CachePic(cstr!("gfx/qplaque.lmp")));
        let p = g::Draw_CachePic(cstr!("gfx/ttl_main.lmp"));
        quake_rs_menu_draw_pic(cbx, (320 - pic_width(p)) / 2, 4, p);

        quake_rs_menu_draw_trans_pic(
            cbx,
            72,
            32,
            if !menu2.is_null() {
                menu2
            } else {
                g::Draw_CachePic(cstr!("gfx/mainmenu.lmp"))
            },
        );

        let f = as_i_d(g::realtime * 10.0) % 6;

        m_mouse_update_list_cursor(
            ptr::addr_of_mut!(M_MAIN_CURSOR),
            70,
            320,
            32,
            20,
            main_items,
            0,
        );
        quake_rs_menu_draw_trans_pic(
            cbx,
            54,
            32 + M_MAIN_CURSOR * 20,
            g::Draw_CachePic(g::va(cstr!("gfx/menudot%i.lmp"), f + 1)),
        );
    }
}

/// `menu.c:660` -- `void M_Main_Key (int key)`.
///
/// # Safety
/// Dispatches into the other menus and, on escape, the demo loop.
unsafe fn m_main_key(key: c_int) -> Raise {
    // SAFETY: single-threaded menu and client state.
    unsafe {
        let menu2 = get_menu2();

        match key {
            K_MOUSE2 | K_ESCAPE | K_BBUTTON => {
                g::IN_Activate();
                g::key_dest = KEY_GAME;
                g::m_state = M_NONE;
                cls.demonum = M_SAVE_DEMONUM;
                if c::cl_main::cl_startdemos.value == 0.0 {
                    /* QuakeSpasm customization: */
                    return 0;
                }
                if cls.demonum != -1 && !cls.demoplayback && cls.state != CA_CONNECTED {
                    raise!(g::Menu_Glue_NextDemo());
                }
            }

            K_DOWNARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                M_MAIN_CURSOR += 1;
                if M_MAIN_CURSOR >= MAIN_ITEMS + if !menu2.is_null() { 1 } else { 0 } {
                    M_MAIN_CURSOR = 0;
                }
            }

            K_UPARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                M_MAIN_CURSOR -= 1;
                if M_MAIN_CURSOR < 0 {
                    M_MAIN_CURSOR = (MAIN_ITEMS + if !menu2.is_null() { 1 } else { 0 }) - 1;
                }
            }

            K_ENTER | K_KP_ENTER | K_ABUTTON | K_MOUSE1 => match M_MAIN_CURSOR {
                0 => m_menu_singleplayer_f(),
                1 => m_menu_multiplayer_f(),
                2 => quake_rs_menu_menu_options_f(),
                3 => m_menu_help_f(),
                4 => {
                    if !menu2.is_null() {
                        m_menu_mods_f();
                    } else {
                        quake_rs_menu_menu_quit_f();
                    }
                }
                5 => quake_rs_menu_menu_quit_f(),
                _ => {}
            },

            _ => {}
        }
    }
    0
}

/* ------------------------------------------------------------------------
 * menu.c:724-818 -- SINGLE PLAYER MENU.
 */

/// `menu.c:727` -- `int m_singleplayer_cursor;`.
static mut M_SINGLEPLAYER_CURSOR: c_int = 0;
/// `menu.c:728` -- `static qboolean m_singleplayer_showlevels;`.
static mut M_SINGLEPLAYER_SHOWLEVELS: bool = false;

/// `menu.c:729` -- `#define SINGLEPLAYER_ITEMS (3 + (m_singleplayer_showlevels
/// ? 1 : 0))`.
///
/// # Safety
/// Reads the menu-local flag.
#[inline]
unsafe fn singleplayer_items() -> c_int {
    // SAFETY: single-threaded menu state.
    unsafe { 3 + if M_SINGLEPLAYER_SHOWLEVELS { 1 } else { 0 } }
}

/// `menu.c:731` -- `static void M_Menu_SinglePlayer_f (void)`.
///
/// # Safety
/// Touches glue-owned menu state and the input subsystem.
unsafe fn m_menu_singleplayer_f() {
    // SAFETY: single-threaded menu state.
    unsafe {
        quake_rs_menu_menu_changed();
        g::IN_Deactivate(true);
        g::key_dest = KEY_MENU;
        g::m_state = M_SINGLEPLAYER;
        if M_SINGLEPLAYER_CURSOR >= singleplayer_items() {
            M_SINGLEPLAYER_CURSOR = 0;
        }
    }
}

/// `menu.c:741` -- `static void M_SinglePlayer_Draw (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_singleplayer_draw(cbx: *mut c_void) {
    // SAFETY: caller contract plus single-threaded menu state.
    unsafe {
        quake_rs_menu_draw_trans_pic(cbx, 16, 4, g::Draw_CachePic(cstr!("gfx/qplaque.lmp")));
        let p = g::Draw_CachePic(cstr!("gfx/ttl_sgl.lmp"));
        quake_rs_menu_draw_pic(cbx, (320 - pic_width(p)) / 2, 4, p);
        quake_rs_menu_draw_trans_pic(cbx, 72, 32, g::Draw_CachePic(cstr!("gfx/sp_menu.lmp")));
        if M_SINGLEPLAYER_SHOWLEVELS {
            quake_rs_menu_draw_trans_pic(cbx, 72, 92, g::Draw_CachePic(cstr!("gfx/sp_maps.lmp")));
        }

        let f = as_i_d(g::realtime * 10.0) % 6;

        m_mouse_update_list_cursor(
            ptr::addr_of_mut!(M_SINGLEPLAYER_CURSOR),
            70,
            320,
            32,
            20,
            singleplayer_items(),
            0,
        );
        quake_rs_menu_draw_trans_pic(
            cbx,
            54,
            32 + M_SINGLEPLAYER_CURSOR * 20,
            g::Draw_CachePic(g::va(cstr!("gfx/menudot%i.lmp"), f + 1)),
        );
    }
}

/// `menu.c:761` -- `static void M_SinglePlayer_Key (int key)`.
///
/// # Safety
/// Reaches `SCR_ModalMessage` through the glue guard.
unsafe fn m_singleplayer_key(key: c_int) -> Raise {
    // SAFETY: single-threaded menu, client and server state.
    unsafe {
        match key {
            K_MOUSE2 | K_ESCAPE | K_BBUTTON => {
                quake_rs_menu_menu_main_f();
            }

            K_DOWNARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                M_SINGLEPLAYER_CURSOR += 1;
                if M_SINGLEPLAYER_CURSOR >= singleplayer_items() {
                    M_SINGLEPLAYER_CURSOR = 0;
                }
            }

            K_UPARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                M_SINGLEPLAYER_CURSOR -= 1;
                if M_SINGLEPLAYER_CURSOR < 0 {
                    M_SINGLEPLAYER_CURSOR = singleplayer_items() - 1;
                }
            }

            K_MOUSE1 | K_ENTER | K_KP_ENTER | K_ABUTTON => {
                g::m_entersound = true;

                match M_SINGLEPLAYER_CURSOR {
                    0 => {
                        if sv_active() {
                            let mut answer: c_int = 0;
                            raise!(g::Menu_Glue_ModalMessage(
                                cstr!("Are you sure you want to\nstart a new game? (y/n)\n"),
                                0.0,
                                ptr::addr_of_mut!(answer),
                            ));
                            if answer == 0 {
                                return 0;
                            }
                        }
                        g::IN_Activate();
                        g::key_dest = KEY_GAME;
                        if sv_active() {
                            g::Cbuf_AddText(cstr!("disconnect\n"));
                        }
                        g::Cbuf_AddText(cstr!("maxplayers 1\n"));
                        g::Cbuf_AddText(cstr!("deathmatch 0\n")); // johnfitz
                        g::Cbuf_AddText(cstr!("coop 0\n")); // johnfitz
                        g::Cbuf_AddText(cstr!("map start\n"));
                    }

                    1 => m_menu_load_f(),

                    2 => m_menu_save_f(),

                    3 => m_menu_maps_f(),

                    _ => {}
                }
            }

            _ => {}
        }
    }
    0
}

/* ------------------------------------------------------------------------
 * menu.c:820-1012 -- LOAD/SAVE MENU.
 */

/// `quakedef.h:97` -- `#define SAVEGAME_COMMENT_LENGTH 39`.
const SAVEGAME_COMMENT_LENGTH: usize = 39;
/// `menu.c:825` -- `#define MAX_SAVEGAMES 20`, johnfitz -- increased from 12.
const MAX_SAVEGAMES: c_int = 20;

/// `menu.c:823` -- `int load_cursor;` (0 < load_cursor < MAX_SAVEGAMES).
static mut LOAD_CURSOR: c_int = 0;
/// `menu.c:826` -- `char
/// m_filenames[MAX_SAVEGAMES][SAVEGAME_COMMENT_LENGTH + 1];`.
static mut M_FILENAMES: [[c_char; SAVEGAME_COMMENT_LENGTH + 1]; MAX_SAVEGAMES as usize] =
    [[0; SAVEGAME_COMMENT_LENGTH + 1]; MAX_SAVEGAMES as usize];
/// `menu.c:827` -- `int loadable[MAX_SAVEGAMES];`.
static mut LOADABLE: [c_int; MAX_SAVEGAMES as usize] = [0; MAX_SAVEGAMES as usize];

/// `menu.c:829` -- `static void M_ScanSaves (void)`.
///
/// # Safety
/// Opens and reads save files through the engine's stdio shims.
unsafe fn m_scan_saves() {
    // SAFETY: single-threaded menu state; every buffer is sized as in C.
    unsafe {
        let mut name: [c_char; c::MAX_OSPATH] = [0; c::MAX_OSPATH];
        let save_path = if g::multiuser {
            g::Sys_GetPrefPath(cstr!("vkQuake"), g::COM_GetGameNames(true))
        } else {
            ptr::null_mut()
        };

        for i in 0..MAX_SAVEGAMES as usize {
            g::strcpy(
                ptr::addr_of_mut!(M_FILENAMES[i]).cast::<c_char>(),
                cstr!("--- UNUSED SLOT ---"),
            );
            LOADABLE[i] = 0;
            let mut j = if g::multiuser { 0 } else { 1 };
            while j < 2 {
                if j == 0 {
                    g::q_snprintf(
                        name.as_mut_ptr(),
                        name.len(),
                        cstr!("%ss%i.sav"),
                        save_path,
                        i as c_int,
                    );
                } else {
                    g::q_snprintf(
                        name.as_mut_ptr(),
                        name.len(),
                        cstr!("%s/s%i.sav"),
                        ptr::addr_of!(c::com_gamedir).cast::<c_char>(),
                        i as c_int,
                    );
                }
                let f = g::Sys_fopen(name.as_ptr(), cstr!("r"));
                if f.is_null() {
                    j += 1;
                    continue;
                }
                let mut version: c_int = 0;
                if g::fscanf(f, cstr!("%i\n"), ptr::addr_of_mut!(version)) != 1 {
                    // COMPAT: menu.c:846-849 `continue`s here without closing
                    // `f`, leaking the handle on a malformed save. The leak is
                    // reproduced rather than fixed so the two builds keep the
                    // same file-descriptor behaviour under the harness.
                    j += 1;
                    continue;
                }
                if g::fscanf(f, cstr!("%79s\n"), name.as_mut_ptr()) != 1 {
                    j += 1;
                    continue;
                }
                g::q_strlcpy(
                    ptr::addr_of_mut!(M_FILENAMES[i]).cast::<c_char>(),
                    name.as_ptr(),
                    SAVEGAME_COMMENT_LENGTH + 1,
                );

                // change _ back to space
                let row = ptr::addr_of_mut!(M_FILENAMES[i]).cast::<c_char>();
                for k in 0..SAVEGAME_COMMENT_LENGTH {
                    if *row.add(k) == b'_' as c_char {
                        *row.add(k) = b' ' as c_char;
                    }
                }
                LOADABLE[i] = 1;
                g::fclose(f);
                break;
            }
        }

        g::Mem_Free(save_path.cast::<c_void>());
    }
}

/// `menu.c:868` -- `static void M_Menu_Load_f (void)`.
///
/// # Safety
/// Touches glue-owned menu state and rescans the save directory.
unsafe fn m_menu_load_f() {
    // SAFETY: single-threaded menu state.
    unsafe {
        quake_rs_menu_menu_changed();
        g::m_state = M_LOAD;

        g::IN_Deactivate(true);
        g::key_dest = KEY_MENU;
        m_scan_saves();
    }
}

/// `menu.c:878` -- `static void M_Menu_Save_f (void)`.
///
/// # Safety
/// Touches glue-owned menu state and rescans the save directory.
unsafe fn m_menu_save_f() {
    // SAFETY: single-threaded menu, client and server state.
    unsafe {
        if !sv_active() {
            return;
        }
        if cl.intermission != 0 {
            return;
        }
        if svs_maxclients() != 1 {
            return;
        }
        quake_rs_menu_menu_changed();
        g::m_state = M_SAVE;

        g::IN_Deactivate(true);
        g::key_dest = KEY_MENU;
        m_scan_saves();
    }
}

/// `menu.c:893` -- `static void M_Load_Draw (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_load_draw(cbx: *mut c_void) {
    // SAFETY: caller contract plus single-threaded menu state.
    unsafe {
        let p = g::Draw_CachePic(cstr!("gfx/p_load.lmp"));
        quake_rs_menu_draw_pic(cbx, (320 - pic_width(p)) / 2, 4, p);

        for i in 0..MAX_SAVEGAMES {
            quake_rs_menu_print(
                cbx,
                16,
                32 + 8 * i,
                ptr::addr_of!(M_FILENAMES[i as usize]).cast::<c_char>(),
            );
        }

        // line cursor
        m_mouse_update_list_cursor(
            ptr::addr_of_mut!(LOAD_CURSOR),
            16,
            320,
            32,
            8,
            MAX_SAVEGAMES,
            0,
        );
        g::Draw_Character(
            cbx,
            8.0,
            (32 + LOAD_CURSOR * 8) as c_float,
            12 + (as_i_d(g::realtime * 4.0) & 1),
        );
    }
}

/// `menu.c:910` -- `static void M_Save_Draw (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_save_draw(cbx: *mut c_void) {
    // SAFETY: caller contract plus single-threaded menu state.
    unsafe {
        let p = g::Draw_CachePic(cstr!("gfx/p_save.lmp"));
        quake_rs_menu_draw_pic(cbx, (320 - pic_width(p)) / 2, 4, p);

        for i in 0..MAX_SAVEGAMES {
            quake_rs_menu_print(
                cbx,
                16,
                32 + 8 * i,
                ptr::addr_of!(M_FILENAMES[i as usize]).cast::<c_char>(),
            );
        }

        // line cursor
        m_mouse_update_list_cursor(
            ptr::addr_of_mut!(LOAD_CURSOR),
            16,
            320,
            32,
            8,
            MAX_SAVEGAMES,
            0,
        );
        g::Draw_Character(
            cbx,
            8.0,
            (32 + LOAD_CURSOR * 8) as c_float,
            12 + (as_i_d(g::realtime * 4.0) & 1),
        );
    }
}

/// `menu.c:929` -- `static void M_Load_Key (int k)`.
///
/// # Safety
/// Reaches `SCR_BeginLoadingPlaque` through the glue guard.
unsafe fn m_load_key(k: c_int) -> Raise {
    // SAFETY: single-threaded menu state.
    unsafe {
        match k {
            K_MOUSE2 | K_ESCAPE | K_BBUTTON => {
                m_menu_singleplayer_f();
            }

            K_MOUSE1 | K_ENTER | K_KP_ENTER | K_ABUTTON => {
                g::S_LocalSound(cstr!("misc/menu2.wav"));
                if LOADABLE[LOAD_CURSOR as usize] == 0 {
                    return 0;
                }
                g::m_state = M_NONE;
                g::IN_Activate();
                g::key_dest = KEY_GAME;

                // Host_Loadgame_f can't bring up the loading plaque because
                // too much stack space has been used, so do it now
                raise!(g::Menu_Glue_BeginLoadingPlaque());

                // issue the load command
                g::Cbuf_AddText(g::va(cstr!("load s%i\n"), LOAD_CURSOR));
                return 0;
            }

            K_UPARROW | K_LEFTARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                LOAD_CURSOR -= 1;
                if LOAD_CURSOR < 0 {
                    LOAD_CURSOR = MAX_SAVEGAMES - 1;
                }
            }

            K_DOWNARROW | K_RIGHTARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                LOAD_CURSOR += 1;
                if LOAD_CURSOR >= MAX_SAVEGAMES {
                    LOAD_CURSOR = 0;
                }
            }

            _ => {}
        }
    }
    0
}

/// `menu.c:971` -- `static void M_Save_Key (int k)`.
///
/// # Safety
/// Touches glue-owned menu state and the command buffer.
unsafe fn m_save_key(k: c_int) {
    // SAFETY: single-threaded menu state.
    unsafe {
        match k {
            K_MOUSE2 | K_ESCAPE | K_BBUTTON => {
                m_menu_singleplayer_f();
            }

            K_MOUSE1 | K_ENTER | K_KP_ENTER | K_ABUTTON => {
                g::m_state = M_NONE;
                g::IN_Activate();
                g::key_dest = KEY_GAME;
                g::Cbuf_AddText(g::va(cstr!("save s%i\n"), LOAD_CURSOR));
            }

            K_UPARROW | K_LEFTARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                LOAD_CURSOR -= 1;
                if LOAD_CURSOR < 0 {
                    LOAD_CURSOR = MAX_SAVEGAMES - 1;
                }
            }

            K_DOWNARROW | K_RIGHTARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                LOAD_CURSOR += 1;
                if LOAD_CURSOR >= MAX_SAVEGAMES {
                    LOAD_CURSOR = 0;
                }
            }

            _ => {}
        }
    }
}

/* ------------------------------------------------------------------------
 * menu.c:1014-1090 -- MULTIPLAYER MENU.
 */

/// `menu.c:1017` -- `int m_multiplayer_cursor;`.
static mut M_MULTIPLAYER_CURSOR: c_int = 0;
/// `menu.c:1018` -- `#define MULTIPLAYER_ITEMS 3`.
const MULTIPLAYER_ITEMS: c_int = 3;

/// `menu.c:1020` -- `static void M_Menu_MultiPlayer_f (void)`.
///
/// Unlike its siblings this one does not call `M_MenuChanged ()`; it sets
/// `m_entersound` itself.
///
/// # Safety
/// Touches glue-owned menu state and the input subsystem.
unsafe fn m_menu_multiplayer_f() {
    // SAFETY: single-threaded menu state.
    unsafe {
        g::IN_Deactivate(true);
        g::key_dest = KEY_MENU;
        g::m_state = M_MULTIPLAYER;
        g::m_entersound = true;
    }
}

/// `menu.c:1028` -- `static void M_MultiPlayer_Draw (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_multiplayer_draw(cbx: *mut c_void) {
    // SAFETY: caller contract plus single-threaded menu state.
    unsafe {
        quake_rs_menu_draw_trans_pic(cbx, 16, 4, g::Draw_CachePic(cstr!("gfx/qplaque.lmp")));
        let p = g::Draw_CachePic(cstr!("gfx/p_multi.lmp"));
        quake_rs_menu_draw_pic(cbx, (320 - pic_width(p)) / 2, 4, p);
        quake_rs_menu_draw_trans_pic(cbx, 72, 32, g::Draw_CachePic(cstr!("gfx/mp_menu.lmp")));

        let f = as_i_d(g::realtime * 10.0) % 6;

        m_mouse_update_list_cursor(
            ptr::addr_of_mut!(M_MULTIPLAYER_CURSOR),
            70,
            320,
            32,
            20,
            MULTIPLAYER_ITEMS,
            0,
        );
        quake_rs_menu_draw_trans_pic(
            cbx,
            54,
            32 + M_MULTIPLAYER_CURSOR * 20,
            g::Draw_CachePic(g::va(cstr!("gfx/menudot%i.lmp"), f + 1)),
        );

        if g::ipv4Available || g::ipv6Available {
            return;
        }
        m_print_white(
            cbx,
            (320 / 2) - ((27 * 8) / 2),
            148,
            cstr!("No Communications Available"),
        );
    }
}

/// `menu.c:1048` -- `static void M_MultiPlayer_Key (int key)`.
///
/// # Safety
/// Touches glue-owned menu state.
unsafe fn m_multiplayer_key(key: c_int) {
    // SAFETY: single-threaded menu state.
    unsafe {
        match key {
            K_MOUSE2 | K_ESCAPE | K_BBUTTON => {
                quake_rs_menu_menu_main_f();
            }

            K_DOWNARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                M_MULTIPLAYER_CURSOR += 1;
                if M_MULTIPLAYER_CURSOR >= MULTIPLAYER_ITEMS {
                    M_MULTIPLAYER_CURSOR = 0;
                }
            }

            K_UPARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                M_MULTIPLAYER_CURSOR -= 1;
                if M_MULTIPLAYER_CURSOR < 0 {
                    M_MULTIPLAYER_CURSOR = MULTIPLAYER_ITEMS - 1;
                }
            }

            K_MOUSE1 | K_ENTER | K_KP_ENTER | K_ABUTTON => {
                g::m_entersound = true;
                match M_MULTIPLAYER_CURSOR {
                    0 | 1 => {
                        if g::ipv4Available || g::ipv6Available {
                            m_menu_net_f();
                        }
                    }

                    2 => m_menu_setup_f(),

                    _ => {}
                }
            }

            _ => {}
        }
    }
}

/* ------------------------------------------------------------------------
 * menu.c:1092-1272 -- SETUP MENU.
 */

/// `menu.c:1095` -- `int setup_cursor = 4;`.
static mut SETUP_CURSOR: c_int = 4;
/// `menu.c:1096` -- `int setup_cursor_table[] = {40, 56, 80, 104, 140};`.
static SETUP_CURSOR_TABLE: [c_int; 5] = [40, 56, 80, 104, 140];

/// `menu.c:1098` -- `char setup_hostname[16];`.
static mut SETUP_HOSTNAME: [c_char; 16] = [0; 16];
/// `menu.c:1099` -- `char setup_myname[16];`.
static mut SETUP_MYNAME: [c_char; 16] = [0; 16];
/// `menu.c:1100` -- `int setup_oldtop;`.
static mut SETUP_OLDTOP: c_int = 0;
/// `menu.c:1101` -- `int setup_oldbottom;`.
static mut SETUP_OLDBOTTOM: c_int = 0;
/// `menu.c:1102` -- `int setup_top;`.
static mut SETUP_TOP: c_int = 0;
/// `menu.c:1103` -- `int setup_bottom;`.
static mut SETUP_BOTTOM: c_int = 0;

/// `menu.c:1105` -- `#define NUM_SETUP_CMDS 5`.
const NUM_SETUP_CMDS: c_int = 5;

/// `menu.c:1107` -- `static void M_Menu_Setup_f (void)`.
///
/// # Safety
/// Touches glue-owned menu state and copies two cvar strings into the
/// fixed-size edit buffers exactly as the C does.
unsafe fn m_menu_setup_f() {
    // SAFETY: single-threaded menu state; the two `strcpy`s are the C's own.
    unsafe {
        g::IN_Deactivate(true);
        g::key_dest = KEY_MENU;
        g::m_state = M_SETUP;
        g::m_entersound = true;
        g::strcpy(
            ptr::addr_of_mut!(SETUP_MYNAME).cast::<c_char>(),
            g::cl_name.string,
        );
        g::strcpy(
            ptr::addr_of_mut!(SETUP_HOSTNAME).cast::<c_char>(),
            g::hostname.string,
        );
        SETUP_OLDTOP = as_i(g::cl_topcolor.value);
        SETUP_TOP = SETUP_OLDTOP;
        SETUP_OLDBOTTOM = as_i(g::cl_bottomcolor.value);
        SETUP_BOTTOM = SETUP_OLDBOTTOM;
    }
}

/// `menu.c:1119` -- `static void M_Setup_Draw (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_setup_draw(cbx: *mut c_void) {
    // SAFETY: caller contract plus single-threaded menu state.
    unsafe {
        quake_rs_menu_draw_trans_pic(cbx, 16, 4, g::Draw_CachePic(cstr!("gfx/qplaque.lmp")));
        let mut p = g::Draw_CachePic(cstr!("gfx/p_multi.lmp"));
        quake_rs_menu_draw_pic(cbx, (320 - pic_width(p)) / 2, 4, p);

        let hostname_buf = ptr::addr_of!(SETUP_HOSTNAME).cast::<c_char>();
        let myname_buf = ptr::addr_of!(SETUP_MYNAME).cast::<c_char>();

        quake_rs_menu_print(cbx, 64, 40, cstr!("Hostname"));
        m_draw_text_box(cbx, 160, 32, 16, 1);
        quake_rs_menu_print(cbx, 168, 40, hostname_buf);

        quake_rs_menu_print(cbx, 64, 56, cstr!("Your name"));
        m_draw_text_box(cbx, 160, 48, 16, 1);
        quake_rs_menu_print(cbx, 168, 56, myname_buf);

        quake_rs_menu_print(cbx, 64, 80, cstr!("Shirt color"));
        quake_rs_menu_print(cbx, 64, 104, cstr!("Pants color"));

        m_draw_text_box(cbx, 64, 140 - 8, 14, 1);
        quake_rs_menu_print(cbx, 72, 140, cstr!("Accept Changes"));

        p = g::Draw_CachePic(cstr!("gfx/bigbox.lmp"));
        quake_rs_menu_draw_trans_pic(cbx, 160, 64, p);
        p = g::Draw_CachePic(cstr!("gfx/menuplyr.lmp"));
        m_draw_trans_pic_translate(cbx, 172, 72, p, SETUP_TOP, SETUP_BOTTOM);

        for i in 0..5 {
            quake_rs_menu_mouse_update_cursor(
                ptr::addr_of_mut!(SETUP_CURSOR),
                0,
                400,
                SETUP_CURSOR_TABLE[i as usize],
                8,
                i,
            );
        }
        g::Draw_Character(
            cbx,
            56.0,
            SETUP_CURSOR_TABLE[SETUP_CURSOR as usize] as c_float,
            12 + (as_i_d(g::realtime * 4.0) & 1),
        );

        if SETUP_CURSOR == 0 {
            g::Draw_Character(
                cbx,
                (168 + 8 * g::strlen(hostname_buf)) as c_float,
                SETUP_CURSOR_TABLE[SETUP_CURSOR as usize] as c_float,
                10 + (as_i_d(g::realtime * 4.0) & 1),
            );
        }

        if SETUP_CURSOR == 1 {
            g::Draw_Character(
                cbx,
                (168 + 8 * g::strlen(myname_buf)) as c_float,
                SETUP_CURSOR_TABLE[SETUP_CURSOR as usize] as c_float,
                10 + (as_i_d(g::realtime * 4.0) & 1),
            );
        }
    }
}

/// The run of statements `menu.c:1190` labels `forward:` and `menu.c:1214`
/// jumps into with `goto forward`.
///
/// # Safety
/// Touches the setup-menu state.
unsafe fn m_setup_forward() {
    // SAFETY: single-threaded menu state.
    unsafe {
        g::S_LocalSound(cstr!("misc/menu3.wav"));
        if SETUP_CURSOR == 2 {
            SETUP_TOP += 1;
        }
        if SETUP_CURSOR == 3 {
            SETUP_BOTTOM += 1;
        }
    }
}

/// `menu.c:1160` -- `static void M_Setup_Key (int k)`.
///
/// # Safety
/// Writes the `hostname` cvar through the glue guard.
unsafe fn m_setup_key(k: c_int) -> Raise {
    // SAFETY: single-threaded menu state.
    unsafe {
        match k {
            K_MOUSE2 | K_ESCAPE | K_BBUTTON => {
                m_menu_multiplayer_f();
            }

            K_UPARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                SETUP_CURSOR -= 1;
                if SETUP_CURSOR < 0 {
                    SETUP_CURSOR = NUM_SETUP_CMDS - 1;
                }
            }

            K_DOWNARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                SETUP_CURSOR += 1;
                if SETUP_CURSOR >= NUM_SETUP_CMDS {
                    SETUP_CURSOR = 0;
                }
            }

            K_LEFTARROW => {
                if SETUP_CURSOR < 2 {
                    return 0;
                }
                g::S_LocalSound(cstr!("misc/menu3.wav"));
                if SETUP_CURSOR == 2 {
                    SETUP_TOP -= 1;
                }
                if SETUP_CURSOR == 3 {
                    SETUP_BOTTOM -= 1;
                }
            }

            K_RIGHTARROW => {
                if SETUP_CURSOR < 2 {
                    return 0;
                }
                m_setup_forward();
            }

            K_MOUSE1 | K_ENTER | K_KP_ENTER | K_ABUTTON => {
                if SETUP_CURSOR == 0 || SETUP_CURSOR == 1 {
                    return 0;
                }

                if SETUP_CURSOR == 2 || SETUP_CURSOR == 3 {
                    // COMPAT: `menu.c:1214` `goto forward;` jumps into the
                    // middle of the `K_RIGHTARROW` case. Rust has no `goto`,
                    // so the jumped-to run of statements is factored into
                    // `m_setup_forward` and called from both sites; the
                    // `break` that ends the label's block lands on the same
                    // colour-clamp tail this arm falls through to.
                    m_setup_forward();
                } else {
                    // setup_cursor == 4 (OK)
                    if g::strcmp(
                        g::cl_name.string,
                        ptr::addr_of!(SETUP_MYNAME).cast::<c_char>(),
                    ) != 0
                    {
                        g::Cbuf_AddText(g::va(
                            cstr!("name \"%s\"\n"),
                            ptr::addr_of!(SETUP_MYNAME).cast::<c_char>(),
                        ));
                    }
                    if g::strcmp(
                        g::hostname.string,
                        ptr::addr_of!(SETUP_HOSTNAME).cast::<c_char>(),
                    ) != 0
                    {
                        raise!(g::Menu_Glue_CvarSet(
                            cstr!("hostname"),
                            ptr::addr_of!(SETUP_HOSTNAME).cast::<c_char>(),
                        ));
                    }
                    if SETUP_TOP != SETUP_OLDTOP || SETUP_BOTTOM != SETUP_OLDBOTTOM {
                        g::Cbuf_AddText(g::va(cstr!("color %i %i\n"), SETUP_TOP, SETUP_BOTTOM));
                    }
                    g::m_entersound = true;
                    m_menu_multiplayer_f();
                }
            }

            K_BACKSPACE => {
                if SETUP_CURSOR == 0 {
                    let l = g::strlen(ptr::addr_of!(SETUP_HOSTNAME).cast::<c_char>());
                    if l != 0 {
                        SETUP_HOSTNAME[l - 1] = 0;
                    }
                }

                if SETUP_CURSOR == 1 {
                    let l = g::strlen(ptr::addr_of!(SETUP_MYNAME).cast::<c_char>());
                    if l != 0 {
                        SETUP_MYNAME[l - 1] = 0;
                    }
                }
            }

            _ => {}
        }

        if SETUP_TOP > 13 {
            SETUP_TOP = 0;
        }
        if SETUP_TOP < 0 {
            SETUP_TOP = 13;
        }
        if SETUP_BOTTOM > 13 {
            SETUP_BOTTOM = 0;
        }
        if SETUP_BOTTOM < 0 {
            SETUP_BOTTOM = 13;
        }
    }
    0
}

/// `menu.c:1240` -- `static void M_Setup_Char (int k)`.
///
/// # Safety
/// Touches the setup-menu edit buffers.
unsafe fn m_setup_char(k: c_int) {
    // SAFETY: single-threaded menu state; `l < 15` keeps both writes inside
    // the 16-byte buffers, exactly as the C bound does.
    unsafe {
        match SETUP_CURSOR {
            0 => {
                let l = g::strlen(ptr::addr_of!(SETUP_HOSTNAME).cast::<c_char>());
                if l < 15 {
                    SETUP_HOSTNAME[l + 1] = 0;
                    SETUP_HOSTNAME[l] = k as c_char;
                }
            }
            1 => {
                let l = g::strlen(ptr::addr_of!(SETUP_MYNAME).cast::<c_char>());
                if l < 15 {
                    SETUP_MYNAME[l + 1] = 0;
                    SETUP_MYNAME[l] = k as c_char;
                }
            }
            _ => {}
        }
    }
}

/// `menu.c:1265` -- `qboolean M_Setup_TextEntry (void)`.
///
/// # Safety
/// Reads the setup-menu cursor.
unsafe fn m_setup_text_entry() -> bool {
    // SAFETY: scalar read of single-threaded menu state.
    unsafe { SETUP_CURSOR == 0 || SETUP_CURSOR == 1 }
}

/* ------------------------------------------------------------------------
 * menu.c:1274-1373 -- NET MENU.
 */

/// `menu.c:1277` -- `int m_net_cursor;`.
static mut M_NET_CURSOR: c_int = 0;
/// `menu.c:1278` -- `int m_first_net_item;`.
static mut M_FIRST_NET_ITEM: c_int = 0;
/// `menu.c:1279` -- `int m_net_items;`.
static mut M_NET_ITEMS: c_int = 0;

/// `menu.c:1281` -- `static const char *net_helpMessage[]`.
const NET_HELP_MESSAGE: [*const c_char; 8] = [
    /* .........1.........2.... */
    cstr!(" Novell network LANs    "),
    cstr!(" or Windows 95 DOS-box. "),
    cstr!("                        "),
    cstr!("(LAN=Local Area Network)"),
    cstr!(" Commonly used to play  "),
    cstr!(" over the Internet, but "),
    cstr!(" also used on a Local   "),
    cstr!(" Area Network.          "),
];

/// `menu.c:1288` -- `static void M_Menu_Net_f (void)`.
///
/// # Safety
/// Touches glue-owned menu state and the input subsystem.
unsafe fn m_menu_net_f() {
    // SAFETY: single-threaded menu state.
    unsafe {
        quake_rs_menu_menu_changed();
        g::IN_Deactivate(true);
        g::key_dest = KEY_MENU;
        g::m_state = M_NET;

        M_NET_ITEMS = 1;
        M_FIRST_NET_ITEM = 1;
        if !g::ipv4Available && !g::ipv6Available {
            M_NET_ITEMS -= 1;
        }

        M_NET_CURSOR = clamp_i(
            M_FIRST_NET_ITEM,
            M_NET_CURSOR,
            M_FIRST_NET_ITEM + M_NET_ITEMS,
        );
    }
}

/// `menu.c:1303` -- `static void M_Net_Draw (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_net_draw(cbx: *mut c_void) {
    // SAFETY: caller contract plus single-threaded menu state. `m_net_cursor`
    // is only ever 1 here: `M_Menu_Net_f` clamps it into
    // `[m_first_net_item, m_first_net_item + m_net_items]` = `[1, 1]` or
    // `[1, 2]`, and both `M_Net_Key` and `M_Mouse_UpdateListCursor` keep it
    // strictly below `m_first_net_item + m_net_items` (<= 2), so
    // `m_net_cursor * 4 + 3` never exceeds index 7.
    unsafe {
        quake_rs_menu_draw_trans_pic(cbx, 16, 4, g::Draw_CachePic(cstr!("gfx/qplaque.lmp")));
        let mut p = g::Draw_CachePic(cstr!("gfx/p_multi.lmp"));
        quake_rs_menu_draw_pic(cbx, (320 - pic_width(p)) / 2, 4, p);

        let mut f = 32;

        p = g::Draw_CachePic(cstr!("gfx/dim_ipx.lmp"));
        quake_rs_menu_draw_trans_pic(cbx, 72, f, p);

        f += 19;
        if g::ipv4Available || g::ipv6Available {
            p = g::Draw_CachePic(cstr!("gfx/netmen4.lmp"));
        } else {
            p = g::Draw_CachePic(cstr!("gfx/dim_tcp.lmp"));
        }
        quake_rs_menu_draw_trans_pic(cbx, 72, f, p);

        f = (320 - 26 * 8) / 2;
        m_draw_text_box(cbx, f, 96, 24, 4);
        f += 8;
        let h = (M_NET_CURSOR * 4) as usize;
        quake_rs_menu_print(cbx, f, 104, NET_HELP_MESSAGE[h]);
        quake_rs_menu_print(cbx, f, 112, NET_HELP_MESSAGE[h + 1]);
        quake_rs_menu_print(cbx, f, 120, NET_HELP_MESSAGE[h + 2]);
        quake_rs_menu_print(cbx, f, 128, NET_HELP_MESSAGE[h + 3]);

        f = as_i_d(g::realtime * 10.0) % 6;
        m_mouse_update_list_cursor(
            ptr::addr_of_mut!(M_NET_CURSOR),
            70,
            320,
            32,
            20,
            M_NET_ITEMS,
            M_FIRST_NET_ITEM,
        );
        quake_rs_menu_draw_trans_pic(
            cbx,
            54,
            32 + M_NET_CURSOR * 20,
            g::Draw_CachePic(g::va(cstr!("gfx/menudot%i.lmp"), f + 1)),
        );
    }
}

/// `menu.c:1341` -- `static void M_Net_Key (int k)`.
///
/// # Safety
/// Touches glue-owned menu state.
unsafe fn m_net_key(k: c_int) {
    // SAFETY: single-threaded menu state.
    unsafe {
        match k {
            K_MOUSE2 | K_ESCAPE | K_BBUTTON => {
                m_menu_multiplayer_f();
            }

            K_DOWNARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                // wrap within [m_first_net_item, m_first_net_item +
                // m_net_items): with IPX removed the first row is permanently
                // dead, so wrapping to 0 would park the cursor on it
                M_NET_CURSOR += 1;
                if M_NET_CURSOR >= M_FIRST_NET_ITEM + M_NET_ITEMS {
                    M_NET_CURSOR = M_FIRST_NET_ITEM;
                }
            }

            K_UPARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                M_NET_CURSOR -= 1;
                if M_NET_CURSOR < M_FIRST_NET_ITEM {
                    M_NET_CURSOR = M_FIRST_NET_ITEM + M_NET_ITEMS - 1;
                }
            }

            K_MOUSE1 | K_ENTER | K_KP_ENTER | K_ABUTTON => {
                g::m_entersound = true;
                m_menu_lanconfig_f();
            }

            _ => {}
        }
    }
}

/* ------------------------------------------------------------------------
 * menu.c:1375-1710 -- GAME OPTIONS MENU.
 */

/// `menu.c:1379-1397` -- the anonymous game-options enum.
const GAME_OPT_SCALE: c_int = 0;
const GAME_OPT_SBALPHA: c_int = 1;
const GAME_OPT_MOUSESPEED: c_int = 2;
const GAME_OPT_VIEWBOB: c_int = 3;
const GAME_OPT_VIEWROLL: c_int = 4;
const GAME_OPT_GUNKICK: c_int = 5;
const GAME_OPT_SHOWGUN: c_int = 6;
const GAME_OPT_ALWAYRUN: c_int = 7;
const GAME_OPT_INVMOUSE: c_int = 8;
const GAME_OPT_HUD_DETAIL: c_int = 9;
const GAME_OPT_HUD_STYLE: c_int = 10;
const GAME_OPT_CROSSHAIR: c_int = 11;
const GAME_OPT_FAST_LOADING: c_int = 12;
const GAME_OPT_AUTOLOAD: c_int = 13;
const GAME_OPT_STARTUP_DEMOS: c_int = 14;
const GAME_OPT_SHOWFPS: c_int = 15;
const GAME_OPT_CONFIRMQUIT: c_int = 16;
const GAME_OPTIONS_ITEMS: c_int = 17;

/// `menu.c:1401` -- `#define GAME_OPTIONS_PER_PAGE MAX_MENU_LINES`.
const GAME_OPTIONS_PER_PAGE: c_int = MAX_MENU_LINES;
/// `menu.c:1402` -- `static int game_options_cursor = 0;`.
static mut GAME_OPTIONS_CURSOR: c_int = 0;
/// `menu.c:1403` -- `static int first_game_option = 0;`.
static mut FIRST_GAME_OPTION: c_int = 0;

/// `menu.c:1405` -- `static void M_Menu_GameOptions_f (void)`.
///
/// # Safety
/// Touches glue-owned menu state and the input subsystem.
unsafe fn m_menu_gameoptions_f() {
    // SAFETY: single-threaded menu state.
    unsafe {
        g::IN_Deactivate(true);
        g::key_dest = KEY_MENU;
        g::m_state = M_GAME;
        g::m_entersound = true;
    }
}

/// `menu.c:1413` -- `static void M_GameOptions_AdjustSliders (int dir,
/// qboolean mouse)`.
///
/// # Safety
/// Every cvar write goes through the ADR-009 glue guard.
unsafe fn m_gameoptions_adjust_sliders(dir: c_int, mut mouse: bool) -> Raise {
    // SAFETY: single-threaded menu and cvar state.
    unsafe {
        if SCROLLBAR_GRAB {
            return 0;
        }

        let f: c_float;
        let clamped_mouse = clamp_f(
            SLIDER_START as c_float,
            M_MOUSE_X as c_float,
            SLIDER_END as c_float,
        );

        if (clamped_mouse - M_MOUSE_X as c_float).abs() > 12.0 {
            mouse = false;
        }

        if dir != 0 {
            g::S_LocalSound(cstr!("misc/menu3.wav"));
        }

        if mouse {
            SLIDER_GRAB = true;
        }

        match GAME_OPTIONS_CURSOR {
            GAME_OPT_SCALE => {
                // console and menu scale
                if g::scr_relativescale.value != 0.0 {
                    f = m_get_slider_pos(
                        1.0,
                        3.0,
                        g::scr_relativescale.value,
                        false,
                        mouse,
                        clamped_mouse,
                        dir,
                        0.1,
                        999.0,
                    );
                    raise!(g::Menu_Glue_CvarSetValue(cstr!("scr_relativescale"), f));
                } else {
                    f = m_get_slider_pos(
                        1.0,
                        (((g::vid.width + 31) / 32) as f64 / 10.0) as c_float,
                        g::scr_conscale.value,
                        false,
                        mouse,
                        clamped_mouse,
                        dir,
                        0.1,
                        999.0,
                    );
                    raise!(g::Menu_Glue_CvarSetValue(cstr!("scr_conscale"), f));
                    raise!(g::Menu_Glue_CvarSetValue(cstr!("scr_menuscale"), f));
                    raise!(g::Menu_Glue_CvarSetValue(cstr!("scr_sbarscale"), f));
                }
            }
            GAME_OPT_MOUSESPEED => {
                // mouse speed
                f = m_get_slider_pos(
                    1.0,
                    11.0,
                    g::sensitivity.value,
                    false,
                    mouse,
                    clamped_mouse,
                    dir,
                    0.5,
                    999.0,
                );
                raise!(g::Menu_Glue_CvarSetValue(cstr!("sensitivity"), f));
            }
            GAME_OPT_SBALPHA => {
                // statusbar alpha
                f = m_get_slider_pos(
                    0.0,
                    1.0,
                    1.0 - g::scr_sbaralpha.value,
                    true,
                    mouse,
                    clamped_mouse,
                    dir,
                    0.05,
                    999.0,
                );
                raise!(g::Menu_Glue_CvarSetValue(cstr!("scr_sbaralpha"), 1.0 - f));
            }
            GAME_OPT_VIEWBOB => {
                // statusbar alpha
                f =
                    (1.0 - m_get_slider_pos(
                        0.0,
                        1.0,
                        1.0 - (g::cl_bob.value * 20.0),
                        true,
                        mouse,
                        clamped_mouse,
                        dir,
                        0.05,
                        999.0,
                    )) / 20.0;
                raise!(g::Menu_Glue_CvarSetValue(cstr!("cl_bob"), f));
            }
            GAME_OPT_VIEWROLL => {
                // statusbar alpha
                f =
                    (1.0 - m_get_slider_pos(
                        0.0,
                        1.0,
                        1.0 - (g::cl_rollangle.value * 0.2),
                        true,
                        mouse,
                        clamped_mouse,
                        dir,
                        0.05,
                        999.0,
                    )) / 0.2;
                raise!(g::Menu_Glue_CvarSetValue(cstr!("cl_rollangle"), f));
            }
            GAME_OPT_GUNKICK => {
                // gun kick
                raise!(g::Menu_Glue_CvarSetValue(
                    cstr!("v_gunkick"),
                    ((as_i(g::v_gunkick.value) + 3 + dir) % 3) as c_float,
                ));
            }
            GAME_OPT_SHOWGUN => {
                // gun kick
                raise!(g::Menu_Glue_CvarSetValue(
                    cstr!("r_drawviewmodel"),
                    ((as_i(g::r_drawviewmodel.value) + 2 + dir) % 2) as c_float,
                ));
            }
            GAME_OPT_ALWAYRUN => {
                // always run
                let on = g::cl_alwaysrun.value != 0.0 || g::cl_forwardspeed.value > 200.0;
                raise!(g::Menu_Glue_CvarSetValue(
                    cstr!("cl_alwaysrun"),
                    if on { 0.0 } else { 1.0 },
                ));

                // The past vanilla "always run" option set these two CVARs, so
                // reset them too when changing in case the user previously
                // used that option.
                raise!(g::Menu_Glue_CvarSetValue(cstr!("cl_forwardspeed"), 200.0));
                raise!(g::Menu_Glue_CvarSetValue(cstr!("cl_backspeed"), 200.0));
            }
            GAME_OPT_INVMOUSE => {
                raise!(g::Menu_Glue_CvarSetValue(
                    cstr!("m_pitch"),
                    -g::m_pitch.value,
                ));
            }
            GAME_OPT_CROSSHAIR => {
                let ndefs = CROSSHAIR_DEFS.len();
                if g::crosshair.value != 0.0 {
                    if (g::crosshair_def.value == (ndefs - 1) as c_float) && (dir == 1) {
                        // fold to off
                        raise!(g::Menu_Glue_CvarSetValue(cstr!("crosshair"), 0.0));
                        raise!(g::Menu_Glue_CvarSetValue(cstr!("crosshair_def"), 0.0));
                    } else if (g::crosshair_def.value == 0.0) && (dir == -1) {
                        // fold to off
                        raise!(g::Menu_Glue_CvarSetValue(cstr!("crosshair"), 0.0));
                    } else {
                        // COMPAT: ADR-004 -- `num_crosshair_defs` is a
                        // `size_t`, so `menu.c:1494` evaluates
                        // `((int)crosshair_def.value + num_crosshair_defs +
                        // dir) % num_crosshair_defs` entirely in unsigned
                        // 64-bit. Both guarded branches above rule out the
                        // only operand combination that could make the sum
                        // negative before promotion, so the unsigned result is
                        // always in `0..num_crosshair_defs` and no UB is
                        // committed reproducing it.
                        let n = ndefs as u64;
                        let v = (as_i(g::crosshair_def.value) as i64 as u64)
                            .wrapping_add(n)
                            .wrapping_add(dir as i64 as u64)
                            % n;
                        raise!(g::Menu_Glue_CvarSetValue(
                            cstr!("crosshair_def"),
                            v as c_float,
                        ));
                    }
                } else {
                    // off => on
                    if dir == -1 {
                        // fold to max
                        raise!(g::Menu_Glue_CvarSetValue(cstr!("crosshair"), 1.0));
                        raise!(g::Menu_Glue_CvarSetValue(
                            cstr!("crosshair_def"),
                            (ndefs - 1) as c_float,
                        ));
                    } else if dir == 1 {
                        raise!(g::Menu_Glue_CvarSetValue(cstr!("crosshair"), 1.0));
                        raise!(g::Menu_Glue_CvarSetValue(cstr!("crosshair_def"), 0.0));
                    }
                }
            }
            GAME_OPT_HUD_DETAIL => {
                // interface detail
                // cycles through 120 (none), 110 (standard), 100 (full)
                if g::scr_viewsize.value <= 100.0 {
                    raise!(g::Menu_Glue_CvarSetValue(
                        cstr!("viewsize"),
                        if dir < 0 { 110.0 } else { 120.0 },
                    ));
                } else if g::scr_viewsize.value <= 110.0 {
                    raise!(g::Menu_Glue_CvarSetValue(
                        cstr!("viewsize"),
                        if dir < 0 { 120.0 } else { 100.0 },
                    ));
                } else {
                    raise!(g::Menu_Glue_CvarSetValue(
                        cstr!("viewsize"),
                        if dir < 0 { 100.0 } else { 110.0 },
                    ));
                }
            }
            GAME_OPT_HUD_STYLE => {
                raise!(g::Menu_Glue_CvarSetValue(
                    cstr!("scr_style"),
                    ((as_i(g::scr_style.value) + 3 + dir) % 3) as c_float,
                ));
            }
            GAME_OPT_FAST_LOADING => {
                // use fast loading when possible
                raise!(g::Menu_Glue_CvarSetValue(
                    cstr!("autofastload"),
                    ((as_i(g::autofastload.value) + 2 + dir) % 2) as c_float,
                ));
            }
            GAME_OPT_AUTOLOAD => {
                // load last save on death
                raise!(g::Menu_Glue_CvarSetValue(
                    cstr!("autoload"),
                    ((as_i(g::autoload.value) + 2 + dir) % 2) as c_float,
                ));
            }
            GAME_OPT_STARTUP_DEMOS => {
                raise!(g::Menu_Glue_CvarSetValue(
                    cstr!("cl_startdemos"),
                    ((as_i(g::cl_startdemos.value) + 2 + dir) % 2) as c_float,
                ));
            }
            GAME_OPT_SHOWFPS => {
                raise!(g::Menu_Glue_CvarSetValue(
                    cstr!("scr_showfps"),
                    ((as_i(g::scr_showfps.value) + 2 + dir) % 2) as c_float,
                ));
            }
            GAME_OPT_CONFIRMQUIT => {
                raise!(g::Menu_Glue_CvarSetValue(
                    cstr!("cl_confirmquit"),
                    ((as_i(g::cl_confirmquit.value) + 2 + dir) % 2) as c_float,
                ));
            }
            _ => {}
        }
    }
    0
}

/// `menu.c:1542` -- `static void M_GameOptions_Key (int k)`.
///
/// # Safety
/// Propagates the slider adjuster's ADR-009 status.
unsafe fn m_gameoptions_key(k: c_int) -> Raise {
    // SAFETY: single-threaded menu state.
    unsafe {
        if quake_rs_menu_handle_scroll_bar_keys(
            k,
            ptr::addr_of_mut!(GAME_OPTIONS_CURSOR),
            ptr::addr_of_mut!(FIRST_GAME_OPTION),
            GAME_OPTIONS_ITEMS,
            GAME_OPTIONS_PER_PAGE,
        ) {
            return 0;
        }

        match k {
            K_MOUSE2 | K_ESCAPE | K_BBUTTON => {
                quake_rs_menu_menu_options_f();
            }

            K_MOUSE1 | K_ENTER | K_KP_ENTER | K_ABUTTON => {
                g::m_entersound = true;
                return m_gameoptions_adjust_sliders(1, k == K_MOUSE1);
            }

            K_LEFTARROW => {
                raise!(m_gameoptions_adjust_sliders(-1, false));
            }

            K_RIGHTARROW => {
                raise!(m_gameoptions_adjust_sliders(1, false));
            }

            _ => {}
        }
    }
    0
}

/// `menu.c:1572` -- `static void M_GameOptions_Draw (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_gameoptions_draw(cbx: *mut c_void) {
    // SAFETY: caller contract plus single-threaded menu and cvar state.
    unsafe {
        let top = MENU_TOP;

        quake_rs_menu_draw_trans_pic(cbx, 16, 4, g::Draw_CachePic(cstr!("gfx/qplaque.lmp")));
        let p = g::Draw_CachePic(cstr!("gfx/p_option.lmp"));
        quake_rs_menu_draw_pic(cbx, (320 - pic_width(p)) / 2, 4, p);

        // Draw the items in the order of the enum defined above:

        for i in 0..GAME_OPTIONS_PER_PAGE.min(GAME_OPTIONS_ITEMS) {
            let y = top + i * CHARACTER_SIZE;
            match i + FIRST_GAME_OPTION {
                GAME_OPT_SCALE => {
                    quake_rs_menu_print(cbx, MENU_LABEL_X, y, cstr!("Interface Scale"));
                    let l = if g::scr_relativescale.value != 0.0 {
                        2.0
                    } else {
                        (g::vid.width as f64 / 320.0 - 1.0) as c_float
                    };
                    let r = if l > 0.0 {
                        (if g::scr_relativescale.value != 0.0 {
                            g::scr_relativescale.value
                        } else {
                            g::scr_conscale.value
                        } - 1.0)
                            / l
                    } else {
                        0.0
                    };
                    m_draw_slider(cbx, MENU_SLIDER_X, y, r, g::va(cstr!("%.1f"), r as f64));
                }

                GAME_OPT_SBALPHA => {
                    quake_rs_menu_print(cbx, MENU_LABEL_X, y, cstr!("HUD Opacity"));
                    let r = g::scr_sbaralpha.value; // scr_sbaralpha range is 1.0 to 0.0
                    m_draw_slider(cbx, MENU_SLIDER_X, y, r, g::va(cstr!("%.2f"), r as f64));
                }

                GAME_OPT_MOUSESPEED => {
                    quake_rs_menu_print(cbx, MENU_LABEL_X, y, cstr!("Mouse Speed"));
                    let r = (g::sensitivity.value - 1.0) / 10.0;
                    m_draw_slider(cbx, MENU_SLIDER_X, y, r, g::va(cstr!("%.1f"), r as f64));
                }

                GAME_OPT_VIEWBOB => {
                    quake_rs_menu_print(cbx, MENU_LABEL_X, y, cstr!("View Bob"));
                    let r = g::cl_bob.value * 20.0;
                    m_draw_slider(cbx, MENU_SLIDER_X, y, r, g::va(cstr!("%.2f"), r as f64));
                }

                GAME_OPT_VIEWROLL => {
                    quake_rs_menu_print(cbx, MENU_LABEL_X, y, cstr!("View Roll"));
                    let r = g::cl_rollangle.value * 0.2;
                    m_draw_slider(cbx, MENU_SLIDER_X, y, r, g::va(cstr!("%.2f"), r as f64));
                }

                GAME_OPT_GUNKICK => {
                    quake_rs_menu_print(cbx, MENU_LABEL_X, y, cstr!("Gun Kick"));
                    quake_rs_menu_print(
                        cbx,
                        MENU_VALUE_X,
                        y,
                        if g::v_gunkick.value == 2.0 {
                            cstr!("smooth")
                        } else if g::v_gunkick.value == 1.0 {
                            cstr!("classic")
                        } else {
                            cstr!("off")
                        },
                    );
                }

                GAME_OPT_SHOWGUN => {
                    quake_rs_menu_print(cbx, MENU_LABEL_X, y, cstr!("Show Gun"));
                    m_draw_checkbox(cbx, MENU_VALUE_X, y, as_i(g::r_drawviewmodel.value));
                }

                GAME_OPT_ALWAYRUN => {
                    quake_rs_menu_print(cbx, MENU_LABEL_X, y, cstr!("Always Run"));
                    m_draw_checkbox(
                        cbx,
                        MENU_VALUE_X,
                        y,
                        (g::cl_alwaysrun.value != 0.0 || g::cl_forwardspeed.value as f64 > 200.0)
                            as c_int,
                    );
                }

                GAME_OPT_INVMOUSE => {
                    quake_rs_menu_print(cbx, MENU_LABEL_X, y, cstr!("Invert Mouse"));
                    m_draw_checkbox(cbx, MENU_VALUE_X, y, (g::m_pitch.value < 0.0) as c_int);
                }

                GAME_OPT_HUD_DETAIL => {
                    quake_rs_menu_print(cbx, MENU_LABEL_X, y, cstr!("HUD Detail"));
                    if g::scr_viewsize.value >= 120.0 {
                        quake_rs_menu_print(cbx, MENU_VALUE_X, y, cstr!("None"));
                    } else if g::scr_viewsize.value >= 110.0 {
                        quake_rs_menu_print(cbx, MENU_VALUE_X, y, cstr!("Minimal"));
                    } else {
                        quake_rs_menu_print(cbx, MENU_VALUE_X, y, cstr!("Full"));
                    }
                }

                GAME_OPT_HUD_STYLE => {
                    quake_rs_menu_print(cbx, MENU_LABEL_X, y, cstr!("HUD Style"));
                    if g::scr_style.value < 1.0 {
                        quake_rs_menu_print(cbx, MENU_VALUE_X, y, cstr!("Mod"));
                    } else if g::scr_style.value < 2.0 {
                        quake_rs_menu_print(cbx, MENU_VALUE_X, y, cstr!("Classic"));
                    } else {
                        quake_rs_menu_print(cbx, MENU_VALUE_X, y, cstr!("Modern"));
                    }
                }

                GAME_OPT_CROSSHAIR => {
                    quake_rs_menu_print(cbx, MENU_LABEL_X, y, cstr!("Crosshair"));
                    if g::crosshair.value == 0.0 {
                        quake_rs_menu_print(cbx, MENU_VALUE_X, y, cstr!("off"));
                    } else {
                        let mut crosshair_as_string: [c_char; 2] = [0; 2];
                        let mut current = g::crosshair_t {
                            crosshair_char: 0,
                            viewport_x_offset: 0.0,
                            viewport_y_offset: 0.0,
                            menu_x_offset: 0,
                            menu_y_offset: 0,
                        };
                        quake_rs_menu_get_crosshair_def(
                            g::crosshair_def.value,
                            ptr::addr_of_mut!(current),
                        );
                        crosshair_as_string[0] = current.crosshair_char;
                        m_print_highlighted(
                            cbx,
                            MENU_VALUE_X + current.menu_x_offset,
                            y + current.menu_y_offset,
                            crosshair_as_string.as_ptr(),
                        );
                    }
                }

                GAME_OPT_FAST_LOADING => {
                    quake_rs_menu_print(cbx, MENU_LABEL_X, y, cstr!("Fast loading"));
                    m_draw_checkbox(cbx, MENU_VALUE_X, y, as_i(g::autofastload.value));
                }

                GAME_OPT_AUTOLOAD => {
                    quake_rs_menu_print(cbx, MENU_LABEL_X, y, cstr!("Load last save"));
                    m_draw_checkbox(cbx, MENU_VALUE_X, y, as_i(g::autoload.value));
                }

                GAME_OPT_STARTUP_DEMOS => {
                    quake_rs_menu_print(cbx, MENU_LABEL_X, y, cstr!("Startup Demos"));
                    m_draw_checkbox(cbx, MENU_VALUE_X, y, as_i(g::cl_startdemos.value));
                }

                GAME_OPT_SHOWFPS => {
                    quake_rs_menu_print(cbx, MENU_LABEL_X, y, cstr!("Show FPS"));
                    m_draw_checkbox(cbx, MENU_VALUE_X, y, as_i(g::scr_showfps.value));
                }

                GAME_OPT_CONFIRMQUIT => {
                    quake_rs_menu_print(cbx, MENU_LABEL_X, y, cstr!("Quit Prompt"));
                    m_draw_checkbox(cbx, MENU_VALUE_X, y, as_i(g::cl_confirmquit.value));
                }

                _ => {}
            }
        }

        if GAME_OPTIONS_ITEMS > GAME_OPTIONS_PER_PAGE {
            m_draw_scrollbar(
                cbx,
                MENU_SCROLLBAR_X,
                MENU_TOP + CHARACTER_SIZE,
                FIRST_GAME_OPTION as c_float
                    / (GAME_OPTIONS_ITEMS - GAME_OPTIONS_PER_PAGE) as c_float,
                (GAME_OPTIONS_PER_PAGE - 2) as c_float,
            );
        }

        // cursor
        m_mouse_update_list_cursor(
            ptr::addr_of_mut!(GAME_OPTIONS_CURSOR),
            MENU_CURSOR_X,
            320,
            top,
            CHARACTER_SIZE,
            GAME_OPTIONS_PER_PAGE,
            FIRST_GAME_OPTION,
        );
        g::Draw_Character(
            cbx,
            MENU_CURSOR_X as c_float,
            (top + (GAME_OPTIONS_CURSOR - FIRST_GAME_OPTION) * CHARACTER_SIZE) as c_float,
            12 + (as_i_d(g::realtime * 4.0) & 1),
        );
    }
}

/* ------------------------------------------------------------------------
 * menu.c:1713-2069 -- GRAPHICS OPTIONS MENU.
 */

/// `menu.c:1715-1732` -- the anonymous graphics-options enum.
const GRAPHICS_OPT_GAMMA: c_int = 0;
const GRAPHICS_OPT_CONTRAST: c_int = 1;
const GRAPHICS_OPT_FOV: c_int = 2;
const GRAPHICS_OPT_8BIT_COLOR: c_int = 3;
const GRAPHICS_OPT_FILTER: c_int = 4;
const GRAPHICS_OPT_MAX_FPS: c_int = 5;
const GRAPHICS_OPT_ANTIALIASING_SAMPLES: c_int = 6;
const GRAPHICS_OPT_ANTIALIASING_MODE: c_int = 7;
const GRAPHICS_OPT_RENDER_SCALE: c_int = 8;
const GRAPHICS_OPT_ANISOTROPY: c_int = 9;
const GRAPHICS_OPT_UNDERWATER: c_int = 10;
const GRAPHICS_OPT_TRANSPARENCY: c_int = 11;
const GRAPHICS_OPT_MODELS: c_int = 12;
const GRAPHICS_OPT_MODEL_INTERPOLATION: c_int = 13;
const GRAPHICS_OPT_PARTICLES: c_int = 14;
const GRAPHICS_OPT_SHADOWS: c_int = 15;
const GRAPHICS_OPTIONS_ITEMS: c_int = 16;

/// `menu.c:1736` -- `static int M_GraphicsOptions_NumItems ()`.
///
/// # Safety
/// Queries the Vulkan device through the glue.
unsafe fn m_graphicsoptions_numitems() -> c_int {
    // SAFETY: the glue reads one already-initialised scalar.
    unsafe { GRAPHICS_OPTIONS_ITEMS - if g::Menu_Glue_RayQuery() { 0 } else { 1 } }
}

/// `menu.c:1741` -- `static int graphics_options_cursor = 0;`.
static mut GRAPHICS_OPTIONS_CURSOR: c_int = 0;

/// `menu.c:1743` -- `static void M_Menu_GraphicsOptions_f (void)`.
///
/// # Safety
/// Touches glue-owned menu state and the input subsystem.
unsafe fn m_menu_graphicsoptions_f() {
    // SAFETY: single-threaded menu state.
    unsafe {
        g::IN_Deactivate(true);
        g::key_dest = KEY_MENU;
        g::m_state = M_GRAPHICS;
        g::m_entersound = true;
    }
}

/// `menu.c:1751` -- `static void M_GraphicsOptions_ChooseNextAASamples (int
/// dir)`.
///
/// # Safety
/// The cvar write goes through the ADR-009 glue guard.
unsafe fn m_graphicsoptions_choose_next_aasamples(dir: c_int) -> Raise {
    // SAFETY: single-threaded cvar state.
    unsafe {
        let mut value = as_i(g::vid_fsaa.value);

        if dir > 0 {
            if value >= 16 {
                value = 0;
            } else if value >= 8 {
                value = 16;
            } else if value >= 4 {
                value = 8;
            } else if value >= 2 {
                value = 4;
            } else {
                value = 2;
            }
        } else if value <= 0 {
            value = 16;
        } else if value <= 2 {
            value = 0;
        } else if value <= 4 {
            value = 2;
        } else if value <= 8 {
            value = 4;
        } else if value <= 16 {
            value = 8;
        } else {
            value = 16;
        }

        g::Menu_Glue_CvarSetValueQuick(ptr::addr_of_mut!(g::vid_fsaa), value as c_float)
    }
}

/// `menu.c:1784` -- `static void M_GraphicsOptions_ChooseNextRenderScale (int
/// dir)`.
///
/// # Safety
/// The cvar write goes through the ADR-009 glue guard.
unsafe fn m_graphicsoptions_choose_next_renderscale(dir: c_int) -> Raise {
    // SAFETY: single-threaded cvar state.
    unsafe {
        let mut value = as_i(g::r_scale.value);

        if dir > 0 {
            if value >= 8 {
                value = 0;
            } else if value >= 4 {
                value = 8;
            } else if value >= 2 {
                value = 4;
            } else {
                value = 2;
            }
        } else if value <= 0 {
            value = 8;
        } else if value <= 2 {
            value = 0;
        } else if value <= 4 {
            value = 2;
        } else if value <= 8 {
            value = 4;
        } else {
            value = 8;
        }

        g::Menu_Glue_CvarSetValueQuick(ptr::addr_of_mut!(g::r_scale), value as c_float)
    }
}

/// `menu.c:1813` -- `static void M_GraphicsOptions_ChooseNextParticles (int
/// dir)`.
///
/// # Safety
/// The cvar write goes through the ADR-009 glue guard.
unsafe fn m_graphicsoptions_choose_next_particles(dir: c_int) -> Raise {
    // SAFETY: single-threaded cvar state.
    unsafe {
        let mut value = as_i(g::r_particles.value);

        if dir > 0 {
            if value == 0 {
                value = 2;
            } else if value == 2 {
                value = 1;
            } else {
                value = 0;
            }
        } else if value == 0 {
            value = 1;
        } else if value == 2 {
            value = 0;
        } else {
            value = 2;
        }

        g::Menu_Glue_CvarSetValueQuick(ptr::addr_of_mut!(g::r_particles), value as c_float)
    }
}

/// `menu.c:1838` -- `static void M_GraphicsOptions_AdjustSliders (int dir,
/// qboolean mouse)`.
///
/// # Safety
/// Every cvar write goes through the ADR-009 glue guard.
unsafe fn m_graphicsoptions_adjust_sliders(dir: c_int, mut mouse: bool) -> Raise {
    // SAFETY: single-threaded menu and cvar state.
    unsafe {
        let f: c_float;
        let clamped_mouse = clamp_f(
            SLIDER_START as c_float,
            M_MOUSE_X as c_float,
            SLIDER_END as c_float,
        );

        if (clamped_mouse - M_MOUSE_X as c_float).abs() > 12.0 {
            mouse = false;
        }

        if dir != 0 {
            g::S_LocalSound(cstr!("misc/menu3.wav"));
        }

        if mouse {
            SLIDER_GRAB = true;
        }

        match GRAPHICS_OPTIONS_CURSOR {
            GRAPHICS_OPT_GAMMA => {
                f = m_get_slider_pos(
                    0.5,
                    1.0,
                    g::vid_gamma.value,
                    true,
                    mouse,
                    clamped_mouse,
                    dir,
                    0.05,
                    999.0,
                );
                raise!(g::Menu_Glue_CvarSetValue(cstr!("gamma"), f));
            }
            GRAPHICS_OPT_CONTRAST => {
                f = m_get_slider_pos(
                    1.0,
                    2.0,
                    g::vid_contrast.value,
                    false,
                    mouse,
                    clamped_mouse,
                    dir,
                    0.1,
                    999.0,
                );
                raise!(g::Menu_Glue_CvarSetValue(cstr!("contrast"), f));
            }
            GRAPHICS_OPT_FOV => {
                f = m_get_slider_pos(
                    80.0,
                    130.0,
                    g::scr_fov.value,
                    false,
                    mouse,
                    clamped_mouse,
                    dir,
                    5.0,
                    999.0,
                );
                raise!(g::Menu_Glue_CvarSetValue(cstr!("fov"), f));
            }
            GRAPHICS_OPT_8BIT_COLOR => {
                raise!(g::Menu_Glue_CvarSetValueQuick(
                    ptr::addr_of_mut!(g::vid_palettize),
                    ((as_i(g::vid_palettize.value) + 2 + dir) % 2) as c_float,
                ));
            }
            GRAPHICS_OPT_FILTER => {
                raise!(g::Menu_Glue_CvarSetValueQuick(
                    ptr::addr_of_mut!(g::vid_filter),
                    ((as_i(g::vid_filter.value) + 2 + dir) % 2) as c_float,
                ));
            }
            GRAPHICS_OPT_MAX_FPS => {
                let clamped_host_maxfps =
                    clamp_f(MIN_FPS_MENU_VALUE, g::host_maxfps.value, MAX_FPS_MENU_VALUE);

                let host_fps_slider_value = if g::host_maxfps.value <= 0.0 {
                    MAX_FPS_MENU_VALUE + FPS_MENU_VALUE_STEP
                } else {
                    clamped_host_maxfps
                };

                let f = g::roundf(m_get_slider_pos(
                    MIN_FPS_MENU_VALUE,
                    MAX_FPS_MENU_VALUE + FPS_MENU_VALUE_STEP,
                    host_fps_slider_value,
                    false,
                    mouse,
                    clamped_mouse,
                    dir,
                    FPS_MENU_VALUE_STEP,
                    2.0 * MAX_FPS_MENU_VALUE,
                ));

                let changed_host_maxfps = if f >= MAX_FPS_MENU_VALUE + FPS_MENU_VALUE_STEP {
                    0.0
                } else {
                    f
                };

                raise!(g::Menu_Glue_CvarSetValueQuick(
                    ptr::addr_of_mut!(g::host_maxfps),
                    changed_host_maxfps,
                ));
            }
            GRAPHICS_OPT_ANTIALIASING_SAMPLES => {
                raise!(m_graphicsoptions_choose_next_aasamples(dir));
                g::Cbuf_AddText(cstr!("vid_restart\n"));
            }
            GRAPHICS_OPT_ANTIALIASING_MODE => {
                if g::Menu_Glue_SampleRateShading() {
                    raise!(g::Menu_Glue_CvarSetValueQuick(
                        ptr::addr_of_mut!(g::vid_fsaamode),
                        ((as_i(g::vid_fsaamode.value) + 2 + dir) % 2) as c_float,
                    ));
                }
            }
            GRAPHICS_OPT_RENDER_SCALE => {
                raise!(m_graphicsoptions_choose_next_renderscale(dir));
            }
            GRAPHICS_OPT_ANISOTROPY => {
                raise!(g::Menu_Glue_CvarSetValueQuick(
                    ptr::addr_of_mut!(g::vid_anisotropic),
                    ((as_i(g::vid_anisotropic.value) + 2 + dir) % 2) as c_float,
                ));
            }
            GRAPHICS_OPT_UNDERWATER => {
                raise!(g::Menu_Glue_CvarSetValueQuick(
                    ptr::addr_of_mut!(g::r_waterwarp),
                    ((as_i(g::r_waterwarp.value) + 3 + dir) % 3) as c_float,
                ));
            }
            GRAPHICS_OPT_TRANSPARENCY => {
                raise!(g::Menu_Glue_CvarSetValueQuick(
                    ptr::addr_of_mut!(g::r_oit),
                    ((as_i(clamp_f(0.0, g::r_oit.value, 2.0)) + 3 + dir) % 3) as c_float,
                ));
            }
            GRAPHICS_OPT_MODELS => {
                raise!(g::Menu_Glue_CvarSetValueQuick(
                    ptr::addr_of_mut!(g::r_enhancedmodels),
                    ((as_i(g::r_enhancedmodels.value) + 2 + dir) % 2) as c_float,
                ));
            }
            GRAPHICS_OPT_MODEL_INTERPOLATION => {
                raise!(g::Menu_Glue_CvarSetValueQuick(
                    ptr::addr_of_mut!(g::r_lerpmodels),
                    ((as_i(g::r_lerpmodels.value) + 2 + dir) % 2) as c_float,
                ));
                raise!(g::Menu_Glue_CvarSetValueQuick(
                    ptr::addr_of_mut!(g::r_lerpmove),
                    g::r_lerpmodels.value,
                ));
                raise!(g::Menu_Glue_CvarSetValueQuick(
                    ptr::addr_of_mut!(g::r_lerpturn),
                    g::r_lerpmodels.value,
                ));
            }
            GRAPHICS_OPT_PARTICLES => {
                raise!(m_graphicsoptions_choose_next_particles(dir));
            }
            // `menu.c:1906` gates the write on `vulkan_globals.ray_query`;
            // written as a match guard so the ungated case falls through to
            // the same no-op C's `switch` reaches.
            GRAPHICS_OPT_SHADOWS if g::Menu_Glue_RayQuery() => {
                raise!(g::Menu_Glue_CvarSetValueQuick(
                    ptr::addr_of_mut!(g::r_rtshadows),
                    ((as_i(g::r_rtshadows.value) + 4 + dir) % 4) as c_float,
                ));
            }
            _ => {}
        }
    }
    0
}

/// `menu.c:1922` -- `static void M_GraphicsOptions_Key (int k)`.
///
/// # Safety
/// Reaches the ADR-009 guarded cvar writes.
unsafe fn m_graphicsoptions_key(k: c_int) -> Raise {
    // SAFETY: single-threaded menu state.
    unsafe {
        match k {
            K_MOUSE2 | K_ESCAPE | K_BBUTTON => {
                quake_rs_menu_menu_options_f();
            }

            K_MOUSE1 | K_ENTER | K_KP_ENTER | K_ABUTTON => {
                g::m_entersound = true;
                return m_graphicsoptions_adjust_sliders(1, k == K_MOUSE1);
            }

            K_UPARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                GRAPHICS_OPTIONS_CURSOR -= 1;
                if GRAPHICS_OPTIONS_CURSOR < 0 {
                    GRAPHICS_OPTIONS_CURSOR = m_graphicsoptions_numitems() - 1;
                }
            }

            K_DOWNARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                GRAPHICS_OPTIONS_CURSOR += 1;
                if GRAPHICS_OPTIONS_CURSOR >= m_graphicsoptions_numitems() {
                    GRAPHICS_OPTIONS_CURSOR = 0;
                }
            }

            K_LEFTARROW => {
                raise!(m_graphicsoptions_adjust_sliders(-1, false));
            }

            K_RIGHTARROW => {
                raise!(m_graphicsoptions_adjust_sliders(1, false));
            }

            _ => {}
        }
    }
    0
}

/// `menu.c:1963` -- `static void M_GraphicsOptions_Draw (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_graphicsoptions_draw(cbx: *mut c_void) {
    // SAFETY: caller contract plus single-threaded menu and cvar state.
    unsafe {
        let top = MENU_TOP;

        quake_rs_menu_draw_trans_pic(cbx, 16, 4, g::Draw_CachePic(cstr!("gfx/qplaque.lmp")));
        let p = g::Draw_CachePic(cstr!("gfx/p_option.lmp"));
        quake_rs_menu_draw_pic(cbx, (320 - pic_width(p)) / 2, 4, p);

        // Draw the items in the order of the enum defined above:
        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_GAMMA,
            cstr!("Gamma"),
        );
        let r = ((1.0 - g::vid_gamma.value as f64) / 0.5) as c_float;
        m_draw_slider(
            cbx,
            MENU_SLIDER_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_GAMMA,
            r,
            g::va(cstr!("%.1f"), g::vid_gamma.value as f64),
        );

        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_CONTRAST,
            cstr!("Contrast"),
        );
        let r = (g::vid_contrast.value as f64 - 1.0) as c_float;
        m_draw_slider(
            cbx,
            MENU_SLIDER_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_CONTRAST,
            r,
            g::va(cstr!("%.1f"), g::vid_contrast.value as f64),
        );

        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_FOV,
            cstr!("Field of View"),
        );
        let r = (g::scr_fov.value - 80.0) / (130 - 80) as c_float;
        m_draw_slider(
            cbx,
            MENU_SLIDER_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_FOV,
            r,
            g::va(cstr!("%.0f"), g::scr_fov.value as f64),
        );

        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_8BIT_COLOR,
            cstr!("8-bit Color"),
        );
        m_draw_checkbox(
            cbx,
            MENU_VALUE_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_8BIT_COLOR,
            as_i(g::vid_palettize.value),
        );

        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_FILTER,
            cstr!("Textures"),
        );
        quake_rs_menu_print(
            cbx,
            MENU_VALUE_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_FILTER,
            if g::vid_filter.value == 0.0 {
                cstr!("smooth")
            } else {
                cstr!("classic")
            },
        );

        // Max FPS special display
        {
            quake_rs_menu_print(
                cbx,
                MENU_LABEL_X,
                top + CHARACTER_SIZE * GRAPHICS_OPT_MAX_FPS,
                cstr!("Max FPS"),
            );

            if g::host_maxfps.value <= 0.0 {
                m_draw_slider(
                    cbx,
                    MENU_SLIDER_X,
                    top + CHARACTER_SIZE * GRAPHICS_OPT_MAX_FPS,
                    1.0,
                    cstr!("no limit"),
                );
            } else {
                let max_r_value: c_float =
                    (1.0 - (FPS_MENU_VALUE_STEP / MAX_FPS_MENU_VALUE) as f64) as c_float;

                // slider knob normal range is [0.0, max_r_value] because 1.0 is
                // reserved for "no limit"
                let clamped_fps =
                    clamp_f(MIN_FPS_MENU_VALUE, g::host_maxfps.value, MAX_FPS_MENU_VALUE);
                let r = (max_r_value * (clamped_fps - MIN_FPS_MENU_VALUE))
                    / (MAX_FPS_MENU_VALUE - MIN_FPS_MENU_VALUE);

                // label displays the real host_fps value if > 0
                m_draw_slider(
                    cbx,
                    MENU_SLIDER_X,
                    top + CHARACTER_SIZE * GRAPHICS_OPT_MAX_FPS,
                    r,
                    g::va(cstr!("%.0f"), g::host_maxfps.value as f64),
                );
            }
        }

        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_ANTIALIASING_SAMPLES,
            cstr!("Antialiasing"),
        );
        quake_rs_menu_print(
            cbx,
            MENU_VALUE_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_ANTIALIASING_SAMPLES,
            if as_i(g::vid_fsaa.value) >= 2 {
                g::va(cstr!("%ix"), clamp_i(2, as_i(g::vid_fsaa.value), 16))
            } else {
                cstr!("off")
            },
        );

        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_ANTIALIASING_MODE,
            cstr!("AA mode"),
        );
        quake_rs_menu_print(
            cbx,
            MENU_VALUE_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_ANTIALIASING_MODE,
            if as_i(g::vid_fsaamode.value) == 0 || !g::Menu_Glue_SampleRateShading() {
                cstr!("Multisample")
            } else {
                cstr!("Supersample")
            },
        );

        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_RENDER_SCALE,
            cstr!("Render Scale"),
        );
        quake_rs_menu_print(
            cbx,
            MENU_VALUE_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_RENDER_SCALE,
            if g::r_scale.value >= 2.0 {
                g::va(cstr!("1/%i"), as_i(g::r_scale.value))
            } else {
                cstr!("off")
            },
        );

        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_ANISOTROPY,
            cstr!("Anisotropic"),
        );
        quake_rs_menu_print(
            cbx,
            MENU_VALUE_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_ANISOTROPY,
            if g::vid_anisotropic.value == 0.0 {
                cstr!("off")
            } else {
                // COMPAT: ADR-005 -- `%g` is outside the Rust formatter's
                // supported set, so this label is built by C's `va ()`
                // exactly as `menu.c:2038` does.
                g::va(
                    cstr!("on (%gx)"),
                    g::Menu_Glue_MaxSamplerAnisotropy() as f64,
                )
            },
        );

        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_UNDERWATER,
            cstr!("Underwater FX"),
        );
        quake_rs_menu_print(
            cbx,
            MENU_VALUE_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_UNDERWATER,
            if g::r_waterwarp.value == 0.0 {
                cstr!("off")
            } else if g::r_waterwarp.value == 1.0 {
                cstr!("Classic")
            } else {
                cstr!("glQuake")
            },
        );

        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_TRANSPARENCY,
            cstr!("Transparency"),
        );
        {
            const TRANSPARENCY_MODES: [*const c_char; 3] =
                [cstr!("Classic"), cstr!("Low"), cstr!("High")];
            quake_rs_menu_print(
                cbx,
                MENU_VALUE_X,
                top + CHARACTER_SIZE * GRAPHICS_OPT_TRANSPARENCY,
                TRANSPARENCY_MODES[as_i(clamp_f(0.0, g::r_oit.value, 2.0)) as usize],
            );
        }

        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_MODELS,
            cstr!("Models"),
        );
        quake_rs_menu_print(
            cbx,
            MENU_VALUE_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_MODELS,
            if g::r_enhancedmodels.value == 0.0 {
                cstr!("classic")
            } else {
                cstr!("enhanced")
            },
        );

        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_MODEL_INTERPOLATION,
            cstr!("Animations"),
        );
        quake_rs_menu_print(
            cbx,
            MENU_VALUE_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_MODEL_INTERPOLATION,
            if g::r_lerpmodels.value == 0.0 {
                cstr!("classic")
            } else {
                cstr!("smooth")
            },
        );

        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_PARTICLES,
            cstr!("Particles"),
        );
        quake_rs_menu_print(
            cbx,
            MENU_VALUE_X,
            top + CHARACTER_SIZE * GRAPHICS_OPT_PARTICLES,
            if as_i(g::r_particles.value) == 0 {
                cstr!("off")
            } else if as_i(g::r_particles.value) == 2 {
                cstr!("Classic")
            } else {
                cstr!("glQuake")
            },
        );

        if g::Menu_Glue_RayQuery() {
            quake_rs_menu_print(
                cbx,
                MENU_LABEL_X,
                top + CHARACTER_SIZE * GRAPHICS_OPT_SHADOWS,
                cstr!("Dynamic Shadows"),
            );
            const SHADOW_MODES: [*const c_char; 4] =
                [cstr!("off"), cstr!("low"), cstr!("medium"), cstr!("high")];
            // COMPAT: ADR-004 -- `menu.c:2062` indexes the four-entry
            // `shadow_modes` with the raw `(int)r_rtshadows.value`, which is
            // out of bounds (undefined behaviour) for any cvar value outside
            // `0..3`. The only writer inside the menu keeps it in range
            // (`menu.c:1907`, `% 4`), so the two builds agree on every value
            // C actually defines; the index is clamped here rather than
            // reproducing the out-of-bounds read.
            quake_rs_menu_print(
                cbx,
                MENU_VALUE_X,
                top + CHARACTER_SIZE * GRAPHICS_OPT_SHADOWS,
                SHADOW_MODES[clamp_i(0, as_i(g::r_rtshadows.value), 3) as usize],
            );
        }

        // cursor
        m_mouse_update_list_cursor(
            ptr::addr_of_mut!(GRAPHICS_OPTIONS_CURSOR),
            MENU_CURSOR_X,
            320,
            top,
            CHARACTER_SIZE,
            m_graphicsoptions_numitems(),
            0,
        );
        g::Draw_Character(
            cbx,
            MENU_CURSOR_X as c_float,
            (top + GRAPHICS_OPTIONS_CURSOR * CHARACTER_SIZE) as c_float,
            12 + (as_i_d(g::realtime * 4.0) & 1),
        );
    }
}

/* ------------------------------------------------------------------------
 * menu.c:2072-2199 -- SOUND OPTIONS MENU.
 */

/// `menu.c:2075-2082` -- the anonymous sound-options enum.
const SOUND_OPT_SNDVOL: c_int = 0;
const SOUND_OPT_MUSICVOL: c_int = 1;
const SOUND_OPT_MUSICEXT: c_int = 2;
const SOUND_OPT_WATERFX: c_int = 3;
const SOUND_OPTIONS_ITEMS: c_int = 4;

/// `menu.c:2084` -- `static int sound_options_cursor = 0;`.
static mut SOUND_OPTIONS_CURSOR: c_int = 0;

/// `menu.c:2086` -- `static void M_Menu_SoundOptions_f (void)`.
///
/// # Safety
/// Touches glue-owned menu state and the input subsystem.
unsafe fn m_menu_soundoptions_f() {
    // SAFETY: single-threaded menu state.
    unsafe {
        g::IN_Deactivate(true);
        g::key_dest = KEY_MENU;
        g::m_state = M_SOUND;
        g::m_entersound = true;
    }
}

/// `menu.c:2094` -- `static void M_SoundOptions_AdjustSliders (int dir,
/// qboolean mouse)`.
///
/// # Safety
/// Every cvar write goes through the ADR-009 glue guard.
unsafe fn m_soundoptions_adjust_sliders(dir: c_int, mut mouse: bool) -> Raise {
    // SAFETY: single-threaded menu and cvar state.
    unsafe {
        let f: c_float;
        let clamped_mouse = clamp_f(
            SLIDER_START as c_float,
            M_MOUSE_X as c_float,
            SLIDER_END as c_float,
        );

        if (clamped_mouse - M_MOUSE_X as c_float).abs() > 12.0 {
            mouse = false;
        }

        if dir != 0 {
            g::S_LocalSound(cstr!("misc/menu3.wav"));
        }

        if mouse {
            SLIDER_GRAB = true;
        }

        match SOUND_OPTIONS_CURSOR {
            SOUND_OPT_SNDVOL => {
                f = m_get_slider_pos(
                    0.0,
                    1.0,
                    g::sfxvolume.value,
                    false,
                    mouse,
                    clamped_mouse,
                    dir,
                    0.01,
                    999.0,
                );
                raise!(g::Menu_Glue_CvarSetValue(cstr!("volume"), f));
            }
            SOUND_OPT_MUSICVOL => {
                f = m_get_slider_pos(
                    0.0,
                    1.0,
                    g::bgmvolume.value,
                    false,
                    mouse,
                    clamped_mouse,
                    dir,
                    0.01,
                    999.0,
                );
                raise!(g::Menu_Glue_CvarSetValue(cstr!("bgmvolume"), f));
            }
            SOUND_OPT_MUSICEXT => {
                raise!(g::Menu_Glue_CvarSetValueQuick(
                    ptr::addr_of_mut!(g::bgm_extmusic),
                    ((as_i(g::bgm_extmusic.value) + 2 + dir) % 2) as c_float,
                ));
            }
            SOUND_OPT_WATERFX => {
                raise!(g::Menu_Glue_CvarSetValueQuick(
                    ptr::addr_of_mut!(g::snd_waterfx),
                    ((as_i(g::snd_waterfx.value) + 2 + dir) % 2) as c_float,
                ));
            }
            _ => {}
        }
    }
    0
}

/// `menu.c:2128` -- `static void M_SoundOptions_Key (int k)`.
///
/// # Safety
/// Reaches the ADR-009 guarded cvar writes.
unsafe fn m_soundoptions_key(k: c_int) -> Raise {
    // SAFETY: single-threaded menu state.
    unsafe {
        match k {
            K_MOUSE2 | K_ESCAPE | K_BBUTTON => {
                quake_rs_menu_menu_options_f();
            }

            K_MOUSE1 | K_ENTER | K_KP_ENTER | K_ABUTTON => {
                g::m_entersound = true;
                return m_soundoptions_adjust_sliders(1, k == K_MOUSE1);
            }

            K_UPARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                SOUND_OPTIONS_CURSOR -= 1;
                if SOUND_OPTIONS_CURSOR < 0 {
                    SOUND_OPTIONS_CURSOR = SOUND_OPTIONS_ITEMS - 1;
                }
            }

            K_DOWNARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                SOUND_OPTIONS_CURSOR += 1;
                if SOUND_OPTIONS_CURSOR >= SOUND_OPTIONS_ITEMS {
                    SOUND_OPTIONS_CURSOR = 0;
                }
            }

            K_LEFTARROW => {
                raise!(m_soundoptions_adjust_sliders(-1, false));
            }

            K_RIGHTARROW => {
                raise!(m_soundoptions_adjust_sliders(1, false));
            }

            _ => {}
        }
    }
    0
}

/// `menu.c:2168` -- `static void M_SoundOptions_Draw (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_soundoptions_draw(cbx: *mut c_void) {
    // SAFETY: caller contract plus single-threaded menu and cvar state.
    unsafe {
        let top = MENU_TOP;

        quake_rs_menu_draw_trans_pic(cbx, 16, 4, g::Draw_CachePic(cstr!("gfx/qplaque.lmp")));
        let p = g::Draw_CachePic(cstr!("gfx/p_option.lmp"));
        quake_rs_menu_draw_pic(cbx, (320 - pic_width(p)) / 2, 4, p);

        // Draw the items in the order of the enum defined above:
        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * SOUND_OPT_SNDVOL,
            cstr!("Sound Volume"),
        );
        let label_value = g::sfxvolume.value * 100.0;
        m_draw_slider(
            cbx,
            MENU_SLIDER_X,
            top + CHARACTER_SIZE * SOUND_OPT_SNDVOL,
            g::sfxvolume.value,
            g::va(cstr!("%.0f%%"), label_value as f64),
        );

        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * SOUND_OPT_MUSICVOL,
            cstr!("Music Volume"),
        );
        let label_value = g::bgmvolume.value * 100.0;
        m_draw_slider(
            cbx,
            MENU_SLIDER_X,
            top + CHARACTER_SIZE * SOUND_OPT_MUSICVOL,
            g::bgmvolume.value,
            g::va(cstr!("%.0f%%"), label_value as f64),
        );

        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * SOUND_OPT_MUSICEXT,
            cstr!("External Music"),
        );
        m_draw_checkbox(
            cbx,
            MENU_VALUE_X,
            top + CHARACTER_SIZE * SOUND_OPT_MUSICEXT,
            as_i(g::bgm_extmusic.value),
        );

        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * SOUND_OPT_WATERFX,
            cstr!("Underwater FX"),
        );
        m_draw_checkbox(
            cbx,
            MENU_VALUE_X,
            top + CHARACTER_SIZE * SOUND_OPT_WATERFX,
            as_i(g::snd_waterfx.value),
        );

        // cursor
        m_mouse_update_list_cursor(
            ptr::addr_of_mut!(SOUND_OPTIONS_CURSOR),
            MENU_CURSOR_X,
            320,
            top,
            CHARACTER_SIZE,
            SOUND_OPTIONS_ITEMS,
            0,
        );
        g::Draw_Character(
            cbx,
            MENU_CURSOR_X as c_float,
            (top + SOUND_OPTIONS_CURSOR * CHARACTER_SIZE) as c_float,
            12 + (as_i_d(g::realtime * 4.0) & 1),
        );
    }
}

/* ------------------------------------------------------------------------
 * menu.c:2201-2312 -- OPTIONS MENU.
 */

/// `menu.c:2204-2214` -- the anonymous options enum.
const OPT_GAME: c_int = 0;
const OPT_CONTROLS: c_int = 1;
const OPT_VIDEO: c_int = 2;
const OPT_GRAPHICS: c_int = 3;
const OPT_SOUND: c_int = 4;
const OPT_PADDING: c_int = 5;
const OPT_DEFAULTS: c_int = 6;
const OPTIONS_ITEMS: c_int = 7;

/// `menu.c:2216` -- `static int options_cursor;`.
static mut OPTIONS_CURSOR: c_int = 0;

/// `menu.c:2218` -- `void M_Menu_Options_f (void)`.
///
/// # Safety
/// Touches glue-owned menu state and the input subsystem.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_menu_options_f() {
    // SAFETY: single-threaded menu state.
    unsafe {
        quake_rs_menu_menu_changed();
        g::IN_Deactivate(true);
        g::key_dest = KEY_MENU;
        g::m_state = M_OPTIONS;
    }
}

/// `menu.c:2226` -- `static void M_Options_Draw (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_options_draw(cbx: *mut c_void) {
    // SAFETY: caller contract plus single-threaded menu state.
    unsafe {
        let top = 40;

        quake_rs_menu_draw_trans_pic(cbx, 16, 4, g::Draw_CachePic(cstr!("gfx/qplaque.lmp")));
        let p = g::Draw_CachePic(cstr!("gfx/p_option.lmp"));
        quake_rs_menu_draw_pic(cbx, (320 - pic_width(p)) / 2, 4, p);

        // Draw the items in the order of the enum defined above:
        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * OPT_GAME,
            cstr!("Game"),
        );
        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * OPT_CONTROLS,
            cstr!("Key Bindings"),
        );
        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * OPT_VIDEO,
            cstr!("Video"),
        );
        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * OPT_GRAPHICS,
            cstr!("Graphics"),
        );
        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * OPT_SOUND,
            cstr!("Sound"),
        );
        quake_rs_menu_print(
            cbx,
            MENU_LABEL_X,
            top + CHARACTER_SIZE * OPT_DEFAULTS,
            cstr!("Reset config"),
        );

        // cursor
        m_mouse_update_list_cursor(
            ptr::addr_of_mut!(OPTIONS_CURSOR),
            MENU_LABEL_X,
            320,
            top,
            CHARACTER_SIZE,
            OPTIONS_ITEMS,
            0,
        );
        if OPTIONS_CURSOR == OPT_PADDING {
            OPTIONS_CURSOR = OPT_SOUND;
        }
        g::Draw_Character(
            cbx,
            MENU_CURSOR_X as c_float,
            (top + OPTIONS_CURSOR * CHARACTER_SIZE) as c_float,
            12 + (as_i_d(g::realtime * 4.0) & 1),
        );
    }
}

/// `menu.c:2249` -- `void M_Options_Key (int k)`.
///
/// # Safety
/// Reaches `SCR_ModalMessage` and the video menu through the ADR-009 guards.
unsafe fn m_options_key(k: c_int) -> Raise {
    // SAFETY: single-threaded menu state.
    unsafe {
        match k {
            K_MOUSE2 | K_ESCAPE | K_BBUTTON => {
                quake_rs_menu_menu_main_f();
            }

            K_MOUSE1 | K_ENTER | K_KP_ENTER | K_ABUTTON => {
                g::m_entersound = true;
                M_MOUSE_X_PIXELS = -1;
                match OPTIONS_CURSOR {
                    OPT_GAME => {
                        m_menu_gameoptions_f();
                    }
                    OPT_CONTROLS => {
                        m_menu_keys_f();
                    }
                    OPT_DEFAULTS => {
                        let mut answer: c_int = 0;
                        raise!(g::Menu_Glue_ModalMessage(
                            cstr!(
                                "This will reset all controls\nand stored cvars. Continue? (y/n)\n"
                            ),
                            15.0,
                            ptr::addr_of_mut!(answer),
                        ));
                        if answer != 0 {
                            g::Cbuf_AddText(cstr!("resetcfg\n"));
                            g::Cbuf_AddText(cstr!("exec default.cfg\n"));
                        }
                    }
                    OPT_VIDEO => {
                        raise!(g::Menu_Glue_MenuVideo());
                    }
                    OPT_GRAPHICS => {
                        m_menu_graphicsoptions_f();
                    }
                    OPT_SOUND => {
                        m_menu_soundoptions_f();
                    }
                    _ => {}
                }
                return 0;
            }

            K_UPARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                OPTIONS_CURSOR -= 1;
                if OPTIONS_CURSOR == OPT_PADDING {
                    OPTIONS_CURSOR -= 1;
                }
                if OPTIONS_CURSOR < 0 {
                    OPTIONS_CURSOR = OPTIONS_ITEMS - 1;
                }
            }

            K_DOWNARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                OPTIONS_CURSOR += 1;
                if OPTIONS_CURSOR == OPT_PADDING {
                    OPTIONS_CURSOR += 1;
                }
                if OPTIONS_CURSOR >= OPTIONS_ITEMS {
                    OPTIONS_CURSOR = 0;
                }
            }

            _ => {}
        }
    }
    0
}

/* ------------------------------------------------------------------------
 * menu.c:2314-2521 -- KEYS MENU.
 */

/// `menu.c:2317-2318` -- the two macro-expanded bind strings.
const QUICKSAVE: *const c_char = cstr!("echo Quicksaving...; wait; save quick");
const QUICKLOAD: *const c_char = cstr!("echo Quickloading...; wait; load quick");

/// `menu.c:2320-2353` -- `const char *bindnames[][2]`. C gives this external
/// linkage, but nothing outside `menu.c` names it, so it becomes private Rust
/// state rather than a glue-owned object.
const BINDNAMES: [[*const c_char; 2]; 32] = [
    [cstr!("+forward"), cstr!("Move Forward")],
    [cstr!("+back"), cstr!("Move Backward")],
    [cstr!("+moveleft"), cstr!("Strafe Left")],
    [cstr!("+moveright"), cstr!("Strafe Right")],
    [cstr!("+jump"), cstr!("Jump / Swim up")],
    [cstr!("+attack"), cstr!("Attack")],
    [cstr!("+speed"), cstr!("Run")],
    [cstr!("+zoom"), cstr!("Quick zoom")],
    [cstr!("+moveup"), cstr!("Swim up")],
    [cstr!("+movedown"), cstr!("Swim down")],
    [cstr!("+showscores"), cstr!("Show Scores")],
    [cstr!("impulse 10"), cstr!("Next weapon")],
    [cstr!("impulse 12"), cstr!("Previous weapon")],
    [cstr!("impulse 1"), cstr!("Axe")],
    [cstr!("impulse 2"), cstr!("Shotgun")],
    [cstr!("impulse 3"), cstr!("Super Shotgun")],
    [cstr!("impulse 4"), cstr!("Nailgun")],
    [cstr!("impulse 5"), cstr!("Super Nailgun")],
    [cstr!("impulse 6"), cstr!("Grenade Launcher")],
    [cstr!("impulse 7"), cstr!("Rocket Launcher")],
    [cstr!("impulse 8"), cstr!("Thunderbolt")],
    [cstr!(""), cstr!("")], // placeholder used as separator
    [QUICKSAVE, cstr!("Quick save")],
    [QUICKLOAD, cstr!("Quick load")],
    [cstr!("menu_save"), cstr!("Save menu")],
    [cstr!("menu_load"), cstr!("Load menu")],
    [cstr!("menu_options"), cstr!("Options menu")],
    [cstr!("menu_multiplayer"), cstr!("Multiplayer menu")],
    [cstr!("quit"), cstr!("Quit")],
    [cstr!("help"), cstr!("Help")],
    [cstr!("screenshot"), cstr!("Screenshot")],
    [cstr!("toggleconsole"), cstr!("Toggle console")],
];

/// `menu.c:2355` -- `#define NUMCOMMANDS countof (bindnames)`. C's `countof`
/// yields a `size_t`, which is what `_Generic` sees at `menu.c:2420`.
const NUMCOMMANDS: usize = BINDNAMES.len();

/// `menu.c:2357` -- `static int keys_cursor;`.
static mut KEYS_CURSOR: c_int = 0;
/// `menu.c:2358` -- `static qboolean bind_grab;`.
static mut BIND_GRAB: bool = false;

/// `menu.c:2360` -- `void M_Menu_Keys_f (void)`.
///
/// # Safety
/// Touches glue-owned menu state and the input subsystem.
unsafe fn m_menu_keys_f() {
    // SAFETY: single-threaded menu state.
    unsafe {
        quake_rs_menu_menu_changed();
        g::IN_Deactivate(true);
        g::key_dest = KEY_MENU;
        g::m_state = M_KEYS;
    }
}

/// `menu.c:2368` -- `void M_FindKeysForCommand (const char *command, int
/// *twokeys)`. C gives it external linkage; nothing outside `menu.c` calls
/// it, so it stays private here.
///
/// # Safety
/// `twokeys` must point at two writable `int`s.
unsafe fn m_find_keys_for_command(command: *const c_char, twokeys: *mut c_int) {
    // SAFETY: caller contract; `keybindings` is the glue-owned MAX_KEYS array.
    unsafe {
        *twokeys = -1;
        *twokeys.add(1) = -1;
        let mut count = 0usize;

        for j in 0..g::MAX_KEYS {
            let b = ptr::addr_of!(g::keybindings[j]).read();
            if b.is_null() {
                continue;
            }
            if g::strcmp(b, command) == 0 {
                *twokeys.add(count) = j as c_int;
                count += 1;
                if count == 2 {
                    break;
                }
            }
        }
    }
}

/// `menu.c:2389` -- `void M_UnbindCommand (const char *command)`; see
/// [`m_find_keys_for_command`] on the linkage.
///
/// # Safety
/// `command` must be a NUL-terminated string.
unsafe fn m_unbind_command(command: *const c_char) {
    // SAFETY: caller contract; `keybindings` is the glue-owned MAX_KEYS array.
    unsafe {
        for j in 0..g::MAX_KEYS {
            let b = ptr::addr_of!(g::keybindings[j]).read();
            if b.is_null() {
                continue;
            }
            if g::strcmp(b, command) == 0 {
                g::Key_SetBinding(j as c_int, ptr::null());
            }
        }
    }
}

/// `menu.c:2410` -- `#define BINDS_PER_PAGE 19`.
const BINDS_PER_PAGE: c_int = 19;
/// `menu.c:2412` -- `static int first_key;`.
static mut FIRST_KEY: c_int = 0;
/// `menu.c:2432` -- `#define KEY_STRING_DRAW_POS (160)`.
const KEY_STRING_DRAW_POS: c_int = 160;

/// `menu.c:2414` -- `static void M_Keys_Draw (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_keys_draw(cbx: *mut c_void) {
    // SAFETY: caller contract plus single-threaded menu state.
    unsafe {
        let mut keys: [c_int; 2] = [0; 2];
        // COMPAT: ADR-010 -- `menu.c:2420` mixes `int` and `size_t`, so
        // `_Generic` picks the 64-bit unsigned `q_min`; the subtraction is
        // evaluated in that width here too.
        let keys_height =
            min_u64(BINDS_PER_PAGE as u64, NUMCOMMANDS as u64 - FIRST_KEY as u64) as c_int;

        let p = g::Draw_CachePic(cstr!("gfx/ttl_cstm.lmp"));
        quake_rs_menu_draw_pic(cbx, (320 - pic_width(p)) / 2, 4, p);

        if BIND_GRAB {
            quake_rs_menu_print(cbx, 12, 32, cstr!("Press a key or button for this action"));
        } else {
            quake_rs_menu_print(cbx, 18, 32, cstr!("Enter to change, backspace to clear"));
        }

        // search for known bindings
        let mut i: c_int = 0;
        while i < BINDS_PER_PAGE && i < NUMCOMMANDS as c_int {
            let y = 48 + 8 * i;
            let row = &BINDNAMES[(i + FIRST_KEY) as usize];

            quake_rs_menu_print(cbx, 10, y, row[1]);

            m_find_keys_for_command(row[0], keys.as_mut_ptr());

            // do not draw anything if the bindnames is empty, it means a
            // plceholder separator.
            if g::strlen(row[0]) != 0 && keys[0] == -1 {
                quake_rs_menu_print(cbx, KEY_STRING_DRAW_POS, y, cstr!("???"));
            } else {
                let name = g::Key_KeynumToString(keys[0]);
                quake_rs_menu_print(cbx, KEY_STRING_DRAW_POS, y, name);
                let x = g::strlen(name) as c_int * 8;
                if keys[1] != -1 {
                    let name = g::Key_KeynumToString(keys[1]);
                    m_print_highlighted(cbx, (KEY_STRING_DRAW_POS - 2) + x, y, cstr!(","));
                    quake_rs_menu_print(cbx, (KEY_STRING_DRAW_POS - 2) + x + 12, y, name);
                    // `menu.c:2456` recomputes `x` here; the value is dead
                    // before the next read, so the assignment is dropped.
                }
            }
            i += 1;
        }

        if NUMCOMMANDS > BINDS_PER_PAGE as usize {
            m_draw_scrollbar(
                cbx,
                MENU_SCROLLBAR_X,
                56,
                FIRST_KEY as c_float / (NUMCOMMANDS as c_int - BINDS_PER_PAGE) as c_float,
                (BINDS_PER_PAGE - 2) as c_float,
            );
        }

        if BIND_GRAB {
            g::Draw_Character(
                cbx,
                (KEY_STRING_DRAW_POS - 10) as c_float,
                (48 + (KEYS_CURSOR - FIRST_KEY) * 8) as c_float,
                b'=' as c_int,
            );
        } else {
            m_mouse_update_list_cursor(
                ptr::addr_of_mut!(KEYS_CURSOR),
                12,
                400,
                48,
                8,
                keys_height,
                FIRST_KEY,
            );
            g::Draw_Character(
                cbx,
                0.0,
                (48 + (KEYS_CURSOR - FIRST_KEY) * 8) as c_float,
                12 + (as_i_d(g::realtime * 4.0) & 1),
            );
        }
    }
}

/// `menu.c:2471` -- `void M_Keys_Key (int k)`. C gives it external linkage;
/// nothing outside `menu.c` calls it.
///
/// # Safety
/// Touches glue-owned menu state and the key-binding table.
unsafe fn m_keys_key(k: c_int) {
    // SAFETY: single-threaded menu state.
    unsafe {
        let mut cmd: [c_char; 80] = [0; 80];
        let mut keys: [c_int; 2] = [0; 2];

        if BIND_GRAB {
            // defining a key
            g::S_LocalSound(cstr!("misc/menu1.wav"));
            if k != K_ESCAPE && k != b'`' as c_int {
                g::q_snprintf(
                    cmd.as_mut_ptr(),
                    cmd.len(),
                    cstr!("bind \"%s\" \"%s\"\n"),
                    g::Key_KeynumToString(k),
                    BINDNAMES[KEYS_CURSOR as usize][0],
                );
                g::Cbuf_InsertText(cmd.as_ptr());
            }

            BIND_GRAB = false;
            g::IN_Deactivate(true); // deactivate because we're returning to the menu
            return;
        }

        if quake_rs_menu_handle_scroll_bar_keys(
            k,
            ptr::addr_of_mut!(KEYS_CURSOR),
            ptr::addr_of_mut!(FIRST_KEY),
            NUMCOMMANDS as c_int,
            BINDS_PER_PAGE,
        ) {
            return;
        }

        match k {
            K_MOUSE2 | K_ESCAPE | K_BBUTTON => {
                quake_rs_menu_menu_options_f();
            }

            // go into bind mode
            K_MOUSE1 | K_ENTER | K_KP_ENTER | K_ABUTTON => {
                m_find_keys_for_command(BINDNAMES[KEYS_CURSOR as usize][0], keys.as_mut_ptr());
                // if bindnames is empty, it means as a placeholder separator
                if g::strlen(BINDNAMES[KEYS_CURSOR as usize][0]) == 0 {
                    return;
                }
                g::S_LocalSound(cstr!("misc/menu2.wav"));
                if keys[1] != -1 {
                    m_unbind_command(BINDNAMES[KEYS_CURSOR as usize][0]);
                }
                BIND_GRAB = true;
                g::IN_Activate(); // activate to allow mouse key binding
            }

            // delete bindings
            K_BACKSPACE | K_DEL => {
                g::S_LocalSound(cstr!("misc/menu2.wav"));
                m_unbind_command(BINDNAMES[KEYS_CURSOR as usize][0]);
            }

            _ => {}
        }
    }
}

/* ------------------------------------------------------------------------
 * menu.c:2523-2573 -- HELP MENU.
 */

/// `menu.c:2526` -- `int help_page;`. C gives it external linkage, but
/// nothing outside `menu.c` names it, so it becomes private Rust state.
static mut HELP_PAGE: c_int = 0;
/// `menu.c:2527` -- `#define NUM_HELP_PAGES 6`.
const NUM_HELP_PAGES: c_int = 6;

/// `menu.c:2529` -- `static void M_Menu_Help_f (void)`.
///
/// # Safety
/// Touches glue-owned menu state and the input subsystem.
unsafe fn m_menu_help_f() {
    // SAFETY: single-threaded menu state.
    unsafe {
        quake_rs_menu_menu_changed();
        g::IN_Deactivate(true);
        g::key_dest = KEY_MENU;
        g::m_state = M_HELP;
        g::m_entersound = true;
        HELP_PAGE = 0;
    }
}

/// `menu.c:2539` -- `static void M_Help_Draw (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_help_draw(cbx: *mut c_void) {
    // SAFETY: caller contract plus single-threaded menu state.
    unsafe {
        quake_rs_menu_draw_pic(
            cbx,
            0,
            0,
            g::Draw_CachePic(g::va(cstr!("gfx/help%i.lmp"), HELP_PAGE)),
        );
    }
}

/// `menu.c:2544` -- `static void M_Help_Key (int key)`.
///
/// # Safety
/// Touches glue-owned menu state.
unsafe fn m_help_key(key: c_int) {
    // SAFETY: single-threaded menu state.
    unsafe {
        match key {
            K_MOUSE2 | K_ESCAPE | K_BBUTTON => {
                quake_rs_menu_menu_main_f();
            }

            K_MOUSE1 | K_MWHEELDOWN | K_UPARROW | K_RIGHTARROW => {
                g::m_entersound = true;
                HELP_PAGE += 1;
                if HELP_PAGE >= NUM_HELP_PAGES {
                    HELP_PAGE = 0;
                }
            }

            K_MWHEELUP | K_DOWNARROW | K_LEFTARROW => {
                g::m_entersound = true;
                HELP_PAGE -= 1;
                if HELP_PAGE < 0 {
                    HELP_PAGE = NUM_HELP_PAGES - 1;
                }
            }

            _ => {}
        }
    }
}

/* ------------------------------------------------------------------------
 * menu.c:2575-2652 -- MODS MENU.
 */

/// `menu.c:2578` -- `#define MAX_MODS_ON_SCREEN MAX_MENU_LINES`.
const MAX_MODS_ON_SCREEN: c_int = MAX_MENU_LINES;

/// `menu.c:2580-2583` -- the mods-menu statics.
static mut NUM_MODS: c_int = 0;
static mut FIRST_MOD: c_int = 0;
static mut MODS_CURSOR: c_int = 0;
static mut MOD_LOADED_FROM_MENU: c_int = 0;

/// `menu.c:2585` -- `static void M_Menu_Mods_f (void)`.
///
/// # Safety
/// Walks the engine-owned `modlist` chain.
unsafe fn m_menu_mods_f() {
    // SAFETY: single-threaded menu state; `modlist` is a stable list the
    // filesystem layer builds once.
    unsafe {
        quake_rs_menu_menu_changed();
        g::IN_Deactivate(true);
        g::key_dest = KEY_MENU;
        g::m_state = M_MODS;
        g::m_entersound = true;
        NUM_MODS = 0;
        let mut item = g::modlist;
        while !item.is_null() {
            NUM_MODS += 1;
            item = (*item).next;
        }
        FIRST_MOD = 0;
        MODS_CURSOR = 0;
    }
}

/// `menu.c:2598` -- `static void M_Mods_Draw (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_mods_draw(cbx: *mut c_void) {
    // SAFETY: caller contract plus the stable `modlist` chain.
    unsafe {
        quake_rs_menu_draw_trans_pic(cbx, 16, 4, g::Draw_CachePic(cstr!("gfx/qplaque.lmp")));
        let p = g::Draw_CachePic(cstr!("gfx/p_mods.lmp"));
        quake_rs_menu_draw_pic(cbx, (320 - pic_width(p)) / 2, 4, p);
        let mut mod_index = -FIRST_MOD;
        let mods_height = min_i(MAX_MODS_ON_SCREEN, NUM_MODS - FIRST_MOD);

        let mut item = g::modlist;
        while !item.is_null() {
            if mod_index >= MAX_MODS_ON_SCREEN {
                break;
            }
            if mod_index >= 0 {
                let fullname = g::Modlist_GetFullName(item);
                m_print_elided(
                    cbx,
                    MENU_LABEL_X,
                    32 + mod_index * CHARACTER_SIZE,
                    if !fullname.is_null() {
                        fullname
                    } else {
                        ptr::addr_of!((*item).name).cast::<c_char>()
                    },
                    23,
                );
            }
            mod_index += 1;
            item = (*item).next;
        }

        m_mouse_update_list_cursor(
            ptr::addr_of_mut!(MODS_CURSOR),
            12,
            400,
            32,
            CHARACTER_SIZE,
            mods_height,
            FIRST_MOD,
        );
        g::Draw_Character(
            cbx,
            MENU_CURSOR_X as c_float,
            (32 + (MODS_CURSOR - FIRST_MOD) * CHARACTER_SIZE) as c_float,
            12 + (as_i_d(g::realtime * 4.0) & 1),
        );
        if NUM_MODS > MAX_MODS_ON_SCREEN {
            m_draw_scrollbar(
                cbx,
                260,
                32 + 8,
                FIRST_MOD as c_float / (NUM_MODS - MAX_MODS_ON_SCREEN) as c_float,
                (MAX_MODS_ON_SCREEN - 2) as c_float,
            );
        }
    }
}

/// `menu.c:2623` -- `static void M_Mods_Key (int key)`.
///
/// # Safety
/// Walks the engine-owned `modlist` chain.
unsafe fn m_mods_key(key: c_int) {
    // SAFETY: single-threaded menu state; `modlist` is stable.
    unsafe {
        let mut mod_index: c_int = 0;

        if quake_rs_menu_handle_scroll_bar_keys(
            key,
            ptr::addr_of_mut!(MODS_CURSOR),
            ptr::addr_of_mut!(FIRST_MOD),
            NUM_MODS,
            MAX_MODS_ON_SCREEN,
        ) {
            return;
        }

        match key {
            K_MOUSE2 | K_ESCAPE | K_BBUTTON => {
                quake_rs_menu_menu_main_f();
            }

            K_MOUSE1 | K_ENTER | K_KP_ENTER | K_ABUTTON => {
                let mut item = g::modlist;
                while !item.is_null() {
                    let this = mod_index;
                    mod_index += 1;
                    if this == MODS_CURSOR {
                        g::Cbuf_AddText(cstr!("game \""));
                        g::Cbuf_AddText(ptr::addr_of!((*item).name).cast::<c_char>());
                        g::Cbuf_AddText(cstr!("\"\n"));
                        MOD_LOADED_FROM_MENU = 1;
                        g::m_state = M_MAIN;
                    }
                    item = (*item).next;
                }
            }

            _ => {}
        }
    }
}

/* ------------------------------------------------------------------------
 * menu.c:2657-2957 -- MAPS MENU (from Ironwail): the ticker, the scrolling
 * text primitives, and the map-list model.
 */

/// `menu.c:2660-2664` -- the maps-list layout constants.
const MAPLIST_X: c_int = 8;
/// See [`MAPLIST_X`].
const MAPLIST_TOP: c_int = 32;
/// See [`MAPLIST_X`].
const MAPLIST_COLS: c_int = 38 + 6;
/// See [`MAPLIST_X`].
const MAPLIST_NAMECOLS: c_int = 14 + 4;
/// See [`MAPLIST_X`].
const MAPLIST_VIEWSIZE: c_int = 19;

/// `menu.c:2666-2670` -- `typedef struct { double scroll_time,
/// scroll_wait_time; } menuticker_t;`.
#[repr(C)]
#[derive(Clone, Copy)]
struct MenuTicker {
    scroll_time: f64,
    scroll_wait_time: f64,
}

/// `menu.c:2672` -- `static void M_Ticker_Init (menuticker_t *ticker)`.
///
/// # Safety
/// `ticker` must point at a live `menuticker_t`.
unsafe fn m_ticker_init(ticker: *mut MenuTicker) {
    // SAFETY: caller contract.
    unsafe {
        (*ticker).scroll_time = 0.0;
        (*ticker).scroll_wait_time = 1.0;
    }
}

/// `menu.c:2678` -- `static void M_Ticker_Update (menuticker_t *ticker)`.
///
/// # Safety
/// `ticker` must point at a live `menuticker_t`.
unsafe fn m_ticker_update(ticker: *mut MenuTicker) {
    // SAFETY: caller contract; `host_rawframetime` is a plain scalar the host
    // loop updates once per frame.
    unsafe {
        if (*ticker).scroll_wait_time <= 0.0 {
            (*ticker).scroll_time += g::host_rawframetime;
        } else {
            (*ticker).scroll_wait_time =
                max_f64(0.0, (*ticker).scroll_wait_time - g::host_rawframetime);
        }
    }
}

/// `menu.c:2686` -- `static qboolean M_Ticker_Key (menuticker_t *ticker, int
/// key)`.
///
/// # Safety
/// `ticker` must point at a live `menuticker_t`.
unsafe fn m_ticker_key(ticker: *mut MenuTicker, key: c_int) -> bool {
    // SAFETY: caller contract.
    unsafe {
        match key {
            K_RIGHTARROW => {
                (*ticker).scroll_time += 0.25;
                (*ticker).scroll_wait_time = 1.5;
                true
            }

            K_LEFTARROW => {
                (*ticker).scroll_time -= 0.25;
                (*ticker).scroll_wait_time = 1.5;
                true
            }

            _ => false,
        }
    }
}

/// `menu.c:2733` -- the five-character gap `" /// "` the scroller wraps
/// through. Indexed only for `ofs - len` in `0 ..= 4`, so the terminating NUL
/// C's array subscript could also reach is never selected.
const SCROLL_GAP: [c_char; 5] = [
    b' ' as c_char,
    b'/' as c_char,
    b'/' as c_char,
    b'/' as c_char,
    b' ' as c_char,
];

/// `menu.c:2710` -- `static void M_PrintScroll (cb_context_t *cbx, int x, int
/// y, int maxwidth, const char *str, double time, qboolean color)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`; `str_` a NUL-terminated C string.
unsafe fn m_print_scroll(
    cbx: *mut c_void,
    mut x: c_int,
    y: c_int,
    maxwidth: c_int,
    str_: *const c_char,
    time: f64,
    color: bool,
) {
    // SAFETY: caller contract.
    unsafe {
        let maxchars = maxwidth / CHARACTER_SIZE;
        let len = g::strlen(str_) as c_int;
        let mask: c_char = if color { 0x80u8 as c_char } else { 0 };

        if len <= maxchars {
            if color {
                quake_rs_menu_print(cbx, x, y, str_);
            } else {
                m_print_white(cbx, x, y, str_);
            }
            return;
        }

        // COMPAT: ADR-004 -- `menu.c:2727` casts `floor (time * 4.0)` to
        // `int`, and `time` is an unbounded accumulator, so C's conversion is
        // undefined once the menu has been open long enough to overflow.
        // `as_i_d` saturates instead of committing UB; the following `%` and
        // the `< 0` fixup keep the result in range either way.
        let mut ofs = as_i_d(c::libm::floor(time * 4.0));
        ofs %= len + 5;
        if ofs < 0 {
            ofs += len + 5;
        }

        for _ in 0..maxchars {
            let ch: c_char = if ofs < len {
                *str_.add(ofs as usize)
            } else {
                SCROLL_GAP[(ofs - len) as usize]
            };
            // `c ^ mask` in C promotes both `char`s to `int` first, so a set
            // high bit in either operand sign-extends. Reproduced exactly.
            g::Draw_Character(
                cbx,
                x as c_float,
                y as c_float,
                (ch as c_int) ^ (mask as c_int),
            );
            x += CHARACTER_SIZE;
            ofs += 1;
            if ofs >= len + 5 {
                ofs = 0;
            }
        }
    }
}

/// `menu.c:2745` -- `static void M_DrawQuakeBar (cb_context_t *cbx, int x,
/// int y, int cols)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_draw_quake_bar(cbx: *mut c_void, mut x: c_int, y: c_int, cols: c_int) {
    // SAFETY: caller contract.
    unsafe {
        g::Draw_Character(cbx, x as c_float, y as c_float, 0o35);
        x += CHARACTER_SIZE;
        // C's `while (cols-- > 0)` runs the body exactly `cols` times.
        let mut cols = cols - 2;
        while cols > 0 {
            cols -= 1;
            g::Draw_Character(cbx, x as c_float, y as c_float, 0o36);
            x += CHARACTER_SIZE;
        }
        g::Draw_Character(cbx, x as c_float, y as c_float, 0o37);
    }
}

/// `menu.c:2764` -- `static void M_DrawEllipsisBar (cb_context_t *cbx, int x,
/// int y, int cols)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_draw_ellipsis_bar(cbx: *mut c_void, mut x: c_int, y: c_int, cols: c_int) {
    // SAFETY: caller contract.
    unsafe {
        let mut cols = cols;
        while cols > 0 {
            g::Draw_Character(cbx, x as c_float, y as c_float, b'.' as c_int | 0x80);
            cols -= 2;
            x += CHARACTER_SIZE * 2;
        }
    }
}

/// `menu.c:2773-2779` -- `mapitem_t`. `#[repr(C)]` so `size_of` matches the
/// `sizeof ((v)[0])` the C `VEC_PUSH` hands `Vec_Grow`.
#[repr(C)]
#[derive(Clone, Copy)]
struct MapItem {
    name: *const c_char,
    source: *const g::filelist_item_t,
    mapidx: c_int,
    active: bool,
}

/// `menu.c:2798` -- `char text[33]` inside `mapsmenu.search`.
const MAPS_SEARCH_TEXT_SIZE: usize = 33;

/// `menu.c:2795-2799` -- the anonymous `search` sub-struct of `mapsmenu`.
#[repr(C)]
struct MapsSearch {
    len: c_int,
    text: [c_char; MAPS_SEARCH_TEXT_SIZE],
}

/// `menu.c:2781-2800` -- `static struct { ... } mapsmenu;`.
#[repr(C)]
struct MapsMenu {
    cursor: c_int,
    scroll: c_int,
    numitems: c_int,
    /// not all items represent actual maps!
    mapcount: c_int,
    prev_cursor: c_int,
    ticker: MenuTicker,
    items: *mut MapItem,
    search: MapsSearch,
}

/// `menu.c:2800` -- `mapsmenu`, zero-initialized exactly like the C static.
static mut MAPSMENU: MapsMenu = MapsMenu {
    cursor: 0,
    scroll: 0,
    numitems: 0,
    mapcount: 0,
    prev_cursor: 0,
    ticker: MenuTicker {
        scroll_time: 0.0,
        scroll_wait_time: 0.0,
    },
    items: ptr::null_mut(),
    search: MapsSearch {
        len: 0,
        text: [0; MAPS_SEARCH_TEXT_SIZE],
    },
};

/// `menu.c:2795` -- `mapsmenu.search.text`, as a bare pointer so no reference
/// to the `static mut` is ever formed.
///
/// # Safety
/// Names the module-owned `MAPSMENU` static; no reference is created.
#[inline]
unsafe fn maps_search_text() -> *mut c_char {
    // SAFETY: address-of only, on single-threaded module-owned storage.
    unsafe { ptr::addr_of_mut!(MAPSMENU.search.text).cast::<c_char>() }
}

/// `menu.c:2802` -- `static const char *M_Maps_GetMessage (const mapitem_t
/// *item)`.
///
/// # Safety
/// `item` must point at a live `mapitem_t`.
unsafe fn m_maps_get_message(item: *const MapItem) -> *const c_char {
    // SAFETY: caller contract; `source` is a node of the engine-owned
    // `extralevels` chain.
    unsafe {
        if (*item).source.is_null() {
            return (*item).name;
        }
        g::ExtraMaps_GetMessage((*item).source)
    }
}

/// `menu.c:2809` -- `static qboolean M_Maps_IsActive (const char *map)`.
///
/// # Safety
/// `map` must be a NUL-terminated C string.
unsafe fn m_maps_is_active(map: *const c_char) -> bool {
    // SAFETY: caller contract plus the single-threaded client state.
    unsafe {
        cls.state == CA_CONNECTED
            && cls.signon == g::SIGNONS
            && g::strcmp(ptr::addr_of!(cl.mapname).cast::<c_char>(), map) == 0
    }
}

/// `menu.c:2814` -- `static void M_Maps_AddDecoration (const char *text)`.
///
/// # Safety
/// `text` must be a NUL-terminated C string that outlives the menu list.
unsafe fn m_maps_add_decoration(text: *const c_char) {
    // SAFETY: `MAPSMENU.items` is a `VEC_*` head this module owns end to end.
    unsafe {
        let mut item: MapItem = core::mem::zeroed();
        item.name = text;
        item.mapidx = -1;
        vec_push(ptr::addr_of_mut!(MAPSMENU.items), item);
        MAPSMENU.numitems += 1;
    }
}

/// `menu.c:2823` -- `static void M_Maps_AddSeparator (maptype_t before,
/// maptype_t after)`.
///
/// # Safety
/// Appends to the module-owned map list.
unsafe fn m_maps_add_separator(before: c_int, after: c_int) {
    // SAFETY: see `m_maps_add_decoration`; the literals are `'static`.
    unsafe {
        if after >= g::MAPTYPE_ID_START {
            if before < g::MAPTYPE_ID_START {
                m_maps_add_decoration(cstr!(""));
                m_maps_add_decoration(cstr!("\x1d\x1e\x1f Original Quake levels \x1d\x1e\x1f"));
            }
            m_maps_add_decoration(cstr!(""));
        } else if after >= g::MAPTYPE_CUSTOM_ID_START && before < g::MAPTYPE_CUSTOM_ID_START {
            m_maps_add_decoration(cstr!(""));
            m_maps_add_decoration(cstr!("\x1d\x1e\x1f Custom Quake levels \x1d\x1e\x1f"));
            m_maps_add_decoration(cstr!(""));
        } else if after >= g::MAPTYPE_MOD_START && before < g::MAPTYPE_MOD_START {
            m_maps_add_decoration(cstr!(""));
            m_maps_add_decoration(cstr!("\x1d\x1e\x1f Official mod levels \x1d\x1e\x1f"));
            m_maps_add_decoration(cstr!(""));
        }
    }
}

/// `menu.c:2848` -- `static qboolean M_Maps_IsSelectable (int index)`.
///
/// # Safety
/// `index` must be in `0 .. mapsmenu.numitems`.
unsafe fn m_maps_is_selectable(index: c_int) -> bool {
    // SAFETY: caller contract.
    unsafe { !(*MAPSMENU.items.add(index as usize)).source.is_null() }
}

/// `menu.c:2853` -- `static qboolean M_Maps_Match (int index)`.
///
/// # Safety
/// `index` must be in `0 .. mapsmenu.numitems`.
unsafe fn m_maps_match(index: c_int) -> bool {
    // SAFETY: caller contract.
    unsafe {
        let item = MAPSMENU.items.add(index as usize);
        if (*item).mapidx < 0 {
            return false;
        }

        if !g::q_strcasestr((*item).name, maps_search_text()).is_null() {
            return true;
        }

        let message = m_maps_get_message(item);
        !message.is_null() && !g::q_strcasestr(message, maps_search_text()).is_null()
    }
}

/// `menu.c:2865` -- `static void M_Maps_ClearSearch (void)`.
///
/// # Safety
/// Writes the module-owned search buffer.
unsafe fn m_maps_clear_search() {
    // SAFETY: `MAPSMENU.search` is module-owned storage.
    unsafe {
        MAPSMENU.search.len = 0;
        *maps_search_text() = 0;
    }
}

/// `menu.c:2871` -- `static int M_Maps_GetOverflow (void)`.
///
/// # Safety
/// Reads the module-owned list state.
unsafe fn m_maps_get_overflow() -> c_int {
    // SAFETY: scalar read of module-owned state.
    unsafe { MAPSMENU.numitems - MAPLIST_VIEWSIZE }
}

/// `menu.c:2876` -- `static void M_Maps_ClampScroll (void)`.
///
/// # Safety
/// Writes the module-owned list state.
unsafe fn m_maps_clamp_scroll() {
    // SAFETY: scalar update of module-owned state.
    unsafe {
        MAPSMENU.scroll = clamp_i(0, MAPSMENU.scroll, max_i(m_maps_get_overflow(), 0));
    }
}

/// `menu.c:2881` -- `static void M_Maps_AutoScroll (void)`.
///
/// # Safety
/// Reads the module-owned item array; the cursor must be a valid index.
unsafe fn m_maps_auto_scroll() {
    // SAFETY: the cursor is kept in range by every writer.
    unsafe {
        if MAPSMENU.numitems <= MAPLIST_VIEWSIZE {
            return;
        }
        if MAPSMENU.cursor < MAPSMENU.scroll {
            MAPSMENU.scroll = MAPSMENU.cursor;
            // show decorations right above the selected item (e.g. a section header)
            while MAPSMENU.scroll > 0
                && MAPSMENU.scroll > MAPSMENU.cursor - MAPLIST_VIEWSIZE + 1
                && !m_maps_is_selectable(MAPSMENU.scroll - 1)
            {
                MAPSMENU.scroll -= 1;
            }
        } else if MAPSMENU.cursor >= MAPSMENU.scroll + MAPLIST_VIEWSIZE {
            MAPSMENU.scroll = MAPSMENU.cursor - MAPLIST_VIEWSIZE + 1;
        }
        m_maps_clamp_scroll();
    }
}

/// `menu.c:2896` -- `static void M_Maps_CenterCursor (void)`.
///
/// # Safety
/// Writes the module-owned list state.
unsafe fn m_maps_center_cursor() {
    // SAFETY: scalar update of module-owned state.
    unsafe {
        if MAPSMENU.cursor >= MAPLIST_VIEWSIZE {
            MAPSMENU.scroll = MAPSMENU.cursor - MAPLIST_VIEWSIZE / 2; // keep centered
        } else {
            MAPSMENU.scroll = 0;
        }
        m_maps_clamp_scroll();
    }
}

/// `menu.c:2905` -- `static qboolean M_Maps_SelectNextMatch (qboolean
/// (*match_fn) (int idx), int start, int dir, qboolean wrap)`.
///
/// # Safety
/// Reads the module-owned item array.
unsafe fn m_maps_select_next_match(
    match_fn: Option<unsafe fn(c_int) -> bool>,
    start: c_int,
    dir: c_int,
    wrap: bool,
) -> bool {
    // SAFETY: every index handed to `m_maps_is_selectable` is bounds-checked
    // by the same wrap/clamp logic the C uses.
    unsafe {
        if MAPSMENU.numitems <= 0 {
            return false;
        }

        let mut start = start;
        if !wrap {
            start = clamp_i(0, start, MAPSMENU.numitems - 1);
        }

        let mut j = start;
        let mut i = 0;
        while i < MAPSMENU.numitems {
            if j < 0 {
                if !wrap {
                    return false;
                }
                j = MAPSMENU.numitems - 1;
            } else if j >= MAPSMENU.numitems {
                if !wrap {
                    return false;
                }
                j = 0;
            }
            if m_maps_is_selectable(j) {
                match match_fn {
                    None => {
                        MAPSMENU.cursor = j;
                        m_maps_auto_scroll();
                        return true;
                    }
                    Some(f) => {
                        if f(j) {
                            MAPSMENU.cursor = j;
                            m_maps_auto_scroll();
                            return true;
                        }
                    }
                }
            }
            i += 1;
            j += dir;
        }

        false
    }
}

/// `menu.c:2938` -- `static qboolean M_Maps_SelectNextSearchMatch (int start,
/// int dir)`.
///
/// # Safety
/// See [`m_maps_select_next_match`].
unsafe fn m_maps_select_next_search_match(start: c_int, dir: c_int) -> bool {
    // SAFETY: see `m_maps_select_next_match`.
    unsafe {
        m_maps_select_next_match(
            Some(m_maps_match as unsafe fn(c_int) -> bool),
            start,
            dir,
            true,
        )
    }
}

/// `menu.c:2943` -- `static qboolean M_Maps_SelectNextActive (int start, int
/// dir, qboolean wrap)`.
///
/// # Safety
/// See [`m_maps_select_next_match`].
unsafe fn m_maps_select_next_active(start: c_int, dir: c_int, wrap: bool) -> bool {
    // SAFETY: see `m_maps_select_next_match`.
    unsafe { m_maps_select_next_match(None, start, dir, wrap) }
}

/// `menu.c:2948` -- `static void M_Maps_UpdateMouseSelection (void)`.
///
/// # Safety
/// See [`m_maps_select_next_match`].
unsafe fn m_maps_update_mouse_selection() {
    // SAFETY: see `m_maps_select_next_match`.
    unsafe {
        if MAPSMENU.cursor < MAPSMENU.scroll {
            m_maps_select_next_active(MAPSMENU.scroll, 1, false);
        } else if MAPSMENU.cursor >= MAPSMENU.scroll + MAPLIST_VIEWSIZE {
            m_maps_select_next_active(MAPSMENU.scroll + MAPLIST_VIEWSIZE - 1, -1, false);
        }
    }
}

/// `menu.c:2963` -- `static void M_Maps_Init (void)`.
///
/// `maptype_t` (`quakedef.h:423-452`) has only non-negative enumerators, so C
/// ranks it as `int` or `unsigned int` depending on the compiler; either way
/// the `(maptype_t)-1` sentinel compares equal only to itself and every real
/// value stays in `0 ..= 20`, so carrying it as a plain `c_int` here is
/// observationally identical.
///
/// # Safety
/// Walks the engine-owned `extralevels_sorted` index.
unsafe fn m_maps_init() {
    // SAFETY: `extralevels_sorted` is a NULL-terminated array the filesystem
    // layer builds; `MAPSMENU.items` is module-owned `VEC_*` storage.
    unsafe {
        m_maps_clear_search();
        MAPSMENU.cursor = -1;
        MAPSMENU.scroll = 0;
        MAPSMENU.numitems = 0;
        MAPSMENU.mapcount = 0;
        g::Vec_Clear(ptr::addr_of_mut!(MAPSMENU.items).cast::<*mut c_void>());

        m_ticker_init(ptr::addr_of_mut!(MAPSMENU.ticker));

        let mut active: c_int = -1;
        let mut prev_type: c_int = -1;
        let mut i: usize = 0;
        while !g::extralevels_sorted.is_null() && !(*g::extralevels_sorted.add(i)).is_null() {
            let item: *const g::filelist_item_t = *g::extralevels_sorted.add(i);
            let type_ = g::ExtraMaps_GetType(item);
            if type_ >= g::MAPTYPE_BMODEL {
                i += 1;
                continue;
            }
            if prev_type != -1 && prev_type != type_ {
                m_maps_add_separator(prev_type, type_);
            }
            prev_type = type_;

            let name = ptr::addr_of!((*item).name).cast::<c_char>();
            let map = MapItem {
                name,
                source: item,
                mapidx: {
                    let idx = MAPSMENU.mapcount;
                    MAPSMENU.mapcount += 1;
                    idx
                },
                active: m_maps_is_active(name),
            };
            if map.active {
                active = vec_size(MAPSMENU.items) as c_int;
            }
            if (map.active && !cls.demoplayback)
                || (MAPSMENU.cursor == -1 && g::ExtraMaps_IsStart(type_))
            {
                MAPSMENU.cursor = vec_size(MAPSMENU.items) as c_int;
            }
            vec_push(ptr::addr_of_mut!(MAPSMENU.items), map);
            MAPSMENU.numitems += 1;
            i += 1;
        }

        if MAPSMENU.cursor == -1 {
            MAPSMENU.cursor = if active != -1 { active } else { 0 };
        }

        m_maps_center_cursor();

        MAPSMENU.prev_cursor = MAPSMENU.cursor;
    }
}

/// `menu.c:3009` -- `static void M_Menu_Maps_f (void)`.
///
/// # Safety
/// See [`m_maps_init`].
unsafe fn m_menu_maps_f() {
    // SAFETY: see `m_maps_init`; the rest is scalar menu state.
    unsafe {
        quake_rs_menu_menu_changed();
        g::IN_Deactivate(true);
        g::key_dest = KEY_MENU;
        g::m_state = M_MAPS;
        g::m_entersound = true;
        m_maps_init();
    }
}

/// `menu.c:3019` -- `static void M_Menu_Maps_Cmd_f (void)`, the `menu_maps`
/// console command's body.
///
/// # Safety
/// See [`m_maps_init`]; runs only from the command layer, where `Cmd_Argv`
/// is valid.
unsafe fn m_menu_maps_cmd_f() {
    // SAFETY: see `m_maps_init`.
    unsafe {
        m_menu_maps_f();

        // handle optional map argument
        if g::Cmd_Argc() >= 2 {
            let mut mapname: [c_char; quake_types::fs::MAX_QPATH] = [0; quake_types::fs::MAX_QPATH];
            g::COM_StripExtension(
                g::Cmd_Argv(1),
                mapname.as_mut_ptr(),
                core::mem::size_of_val(&mapname),
            );

            let n = vec_size(MAPSMENU.items);
            let mut i: usize = 0;
            while i < n {
                if g::q_strcasecmp(mapname.as_ptr(), (*MAPSMENU.items.add(i)).name) == 0 {
                    break;
                }
                i += 1;
            }

            if i == n {
                g::Con_SafePrintf(cstr!("Couldn't find map \"%s\".\n"), mapname.as_ptr());
                return;
            }

            MAPSMENU.cursor = i as c_int;
            m_maps_center_cursor();
            m_set_skill_menu_map(mapname.as_ptr());
            m_menu_skill_f();
        }
    }
}

/// `menu.c:3048` -- `static void M_Maps_UpdateMouse (void)`.
///
/// # Safety
/// Reads the module-owned item array.
unsafe fn m_maps_update_mouse() {
    // SAFETY: every index reaching `m_maps_is_selectable` is bounded by
    // `numvis`, which C derives from `numitems` the same way.
    unsafe {
        if SCROLLBAR_GRAB || SLIDER_GRAB || !M_MOUSE_MOVED {
            return;
        }
        if M_MOUSE_X < MAPLIST_X - CHARACTER_SIZE
            || M_MOUSE_X > MAPLIST_X + MAPLIST_COLS * CHARACTER_SIZE
        {
            return;
        }

        let mut yrel = M_MOUSE_Y - MAPLIST_TOP;
        let numvis = min_i(MAPSMENU.scroll + MAPLIST_VIEWSIZE, MAPSMENU.numitems) - MAPSMENU.scroll;
        if numvis == 0 || yrel < 0 {
            return;
        }
        let mut i = yrel / CHARACTER_SIZE;
        if i >= numvis {
            return;
        }

        i += MAPSMENU.scroll;
        if MAPSMENU.cursor == i {
            return;
        }

        if !m_maps_is_selectable(i) {
            // snap to the closest selectable item instead (from Ironwail)
            let firstvis = MAPSMENU.scroll;
            yrel += firstvis * CHARACTER_SIZE;

            let mut before = i - 1;
            while before >= firstvis {
                if m_maps_is_selectable(before) {
                    break;
                }
                before -= 1;
            }
            let mut after = i + 1;
            while after < firstvis + numvis {
                if m_maps_is_selectable(after) {
                    break;
                }
                after += 1;
            }

            if before >= firstvis && after < firstvis + numvis {
                let distbefore = yrel - CHARACTER_SIZE / 2 - before * CHARACTER_SIZE;
                let distafter = after * CHARACTER_SIZE + CHARACTER_SIZE / 2 - yrel;
                i = if distbefore < distafter {
                    before
                } else {
                    after
                };
            } else if before >= firstvis {
                i = before;
            } else if after < firstvis + numvis {
                i = after;
            } else {
                return;
            }

            if MAPSMENU.cursor == i {
                return;
            }
        }

        MAPSMENU.cursor = i;
    }
}

/// `menu.c:3102` -- `static void M_Maps_Draw (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_maps_draw(cbx: *mut c_void) {
    // SAFETY: caller contract plus the module-owned item array.
    unsafe {
        m_maps_update_mouse();

        let x = MAPLIST_X;
        let cols = MAPLIST_COLS;
        let namecols = MAPLIST_NAMECOLS;
        let desccols = cols - 1 - namecols;

        if MAPSMENU.prev_cursor != MAPSMENU.cursor {
            MAPSMENU.prev_cursor = MAPSMENU.cursor;
            m_ticker_init(ptr::addr_of_mut!(MAPSMENU.ticker));
        } else {
            m_ticker_update(ptr::addr_of_mut!(MAPSMENU.ticker));
        }

        m_print_white(cbx, x, 8, cstr!("Levels"));
        m_draw_quake_bar(cbx, x - 8, 16, namecols + 1);
        m_draw_quake_bar(cbx, x + namecols * CHARACTER_SIZE, 16, cols + 1 - namecols);

        let y = MAPLIST_TOP;

        let mut firstvismap = -1;
        let mut numvismaps = 0;
        let firstvis = MAPSMENU.scroll;
        let numvis = min_i(firstvis + MAPLIST_VIEWSIZE, MAPSMENU.numitems) - firstvis;
        for i in 0..numvis {
            let idx = i + firstvis;
            let item: *const MapItem = MAPSMENU.items.add(idx as usize);
            let message = m_maps_get_message(item);
            let mask = if (*item).active { 128 } else { 0 };
            let selected = idx == MAPSMENU.cursor;

            if (*item).source.is_null() {
                // COMPAT: ADR-004 -- `menu.c:3147` computes `x + (cols -
                // strlen (name)) / 2 * CHARACTER_SIZE` in `size_t`, so a name
                // longer than `cols` wraps through unsigned 64-bit arithmetic
                // before the implicit narrowing to `int`. Reproduced with
                // explicit `u64` maths and a truncating cast rather than the
                // signed arithmetic that would differ.
                let cx = (x as u64).wrapping_add(
                    (cols as u64).wrapping_sub(g::strlen((*item).name) as u64) / 2
                        * CHARACTER_SIZE as u64,
                ) as c_int;
                m_print_white(cbx, cx, y + i * CHARACTER_SIZE, (*item).name);
            } else {
                let mut buf: [c_char; 256] = [0; 256];
                if MAPSMENU.search.len > 0 {
                    g::COM_TintSubstring(
                        (*item).name,
                        maps_search_text(),
                        buf.as_mut_ptr(),
                        core::mem::size_of_val(&buf),
                    );
                } else {
                    g::q_strlcpy(buf.as_mut_ptr(), (*item).name, core::mem::size_of_val(&buf));
                }

                if firstvismap == -1 {
                    firstvismap = (*item).mapidx;
                }
                numvismaps += 1;

                let mut j = 0;
                while j < namecols - 2 && buf[j as usize] != 0 {
                    g::Draw_Character(
                        cbx,
                        (x + j * CHARACTER_SIZE) as c_float,
                        (y + i * CHARACTER_SIZE) as c_float,
                        (buf[j as usize] as c_int) ^ mask,
                    );
                    j += 1;
                }

                if message.is_null() || *message != 0 {
                    if message.is_null() {
                        // still parsing, show a fully dotted line
                        g::memset(
                            buf.as_mut_ptr().cast::<c_void>(),
                            b'.' as c_int | 0x80,
                            desccols as usize,
                        );
                        buf[desccols as usize] = 0;
                    } else if MAPSMENU.search.len > 0 {
                        g::COM_TintSubstring(
                            message,
                            maps_search_text(),
                            buf.as_mut_ptr(),
                            core::mem::size_of_val(&buf),
                        );
                    } else {
                        g::q_strlcpy(buf.as_mut_ptr(), message, core::mem::size_of_val(&buf));
                    }

                    g::GL_SetCanvasColor(1.0, 1.0, 1.0, 0.375);
                    while j < namecols {
                        g::Draw_Character(
                            cbx,
                            (x + j * CHARACTER_SIZE) as c_float,
                            (y + i * CHARACTER_SIZE) as c_float,
                            b'.' as c_int | mask,
                        );
                        j += 1;
                    }
                    if !message.is_null() {
                        g::GL_SetCanvasColor(1.0, 1.0, 1.0, 1.0);
                    }

                    m_print_scroll(
                        cbx,
                        x + namecols * CHARACTER_SIZE,
                        y + i * CHARACTER_SIZE,
                        desccols * CHARACTER_SIZE,
                        buf.as_ptr(),
                        if selected {
                            MAPSMENU.ticker.scroll_time
                        } else {
                            0.0
                        },
                        true,
                    );

                    if message.is_null() {
                        g::GL_SetCanvasColor(1.0, 1.0, 1.0, 1.0);
                    }
                }
            }

            if selected {
                g::Draw_Character(
                    cbx,
                    (x - CHARACTER_SIZE) as c_float,
                    (y + i * CHARACTER_SIZE) as c_float,
                    12 + (as_i_d(g::realtime * 4.0) & 1),
                );
            }
        }

        let str_ = g::va(
            cstr!("%d-%d of %d"),
            firstvismap + 1,
            firstvismap + numvismaps,
            MAPSMENU.mapcount,
        );
        // COMPAT: ADR-004 -- see the `size_t` note above; `menu.c:3195` runs
        // `x + (cols - strlen (str)) * CHARACTER_SIZE` in unsigned 64-bit.
        let sx = (x as u64).wrapping_add(
            (cols as u64).wrapping_sub(g::strlen(str_) as u64) * CHARACTER_SIZE as u64,
        ) as c_int;
        quake_rs_menu_print(cbx, sx, 8, str_);

        if m_maps_get_overflow() > 0 {
            m_draw_scrollbar(
                cbx,
                x + cols * CHARACTER_SIZE - CHARACTER_SIZE,
                y + CHARACTER_SIZE,
                MAPSMENU.scroll as c_float / m_maps_get_overflow() as c_float,
                (MAPLIST_VIEWSIZE - 2) as c_float,
            );

            if MAPSMENU.scroll > 0 {
                m_draw_ellipsis_bar(cbx, x, y - CHARACTER_SIZE, cols);
            }
            if MAPSMENU.scroll + MAPLIST_VIEWSIZE < MAPSMENU.numitems {
                m_draw_ellipsis_bar(cbx, x, y + MAPLIST_VIEWSIZE * CHARACTER_SIZE, cols);
            }
        }

        if MAPSMENU.search.len > 0 {
            let ofs = max_i(0, MAPSMENU.search.len + 1 - namecols);
            let cy = y + MAPLIST_VIEWSIZE * CHARACTER_SIZE + 4;
            m_draw_text_box(cbx, x - CHARACTER_SIZE, cy - CHARACTER_SIZE, namecols, 1);
            let text = maps_search_text();
            let mut i = ofs;
            while i < MAPSMENU.search.len {
                g::Draw_Character(
                    cbx,
                    (x + (i - ofs) * CHARACTER_SIZE) as c_float,
                    cy as c_float,
                    *text.add(i as usize) as c_int,
                );
                i += 1;
            }
            g::Draw_Character(
                cbx,
                (x + (i - ofs) * CHARACTER_SIZE) as c_float,
                cy as c_float,
                10 + (as_i_d(g::realtime * 4.0) & 1),
            );
        }
    }
}

/// `menu.c:3219` -- `static qboolean M_Maps_ListKey (int key)`.
///
/// # Safety
/// Reads the module-owned item array and the glue-owned `keydown[]`.
unsafe fn m_maps_list_key(key: c_int) -> bool {
    // SAFETY: single-threaded menu state; every index is bounded by the
    // wrap/clamp logic inside the selection helpers.
    unsafe {
        let overflow = m_maps_get_overflow() > 0;

        match key {
            K_BACKSPACE => {
                if MAPSMENU.search.len != 0 {
                    if g::keydown[K_CTRL as usize] {
                        m_maps_clear_search();
                    } else {
                        MAPSMENU.search.len -= 1;
                        *maps_search_text().add(MAPSMENU.search.len as usize) = 0;
                    }
                    return true;
                }
                false
            }

            K_ESCAPE | K_BBUTTON | K_MOUSE4 | K_MOUSE2 => {
                if MAPSMENU.search.len != 0 {
                    m_maps_clear_search();
                    return true;
                }
                false
            }

            K_HOME | K_KP_HOME => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                if MAPSMENU.search.len != 0 {
                    m_maps_select_next_search_match(0, 1);
                } else {
                    m_maps_select_next_active(0, 1, false);
                    MAPSMENU.scroll = 0;
                    m_maps_auto_scroll();
                }
                true
            }

            K_END | K_KP_END => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                if MAPSMENU.search.len != 0 {
                    m_maps_select_next_search_match(MAPSMENU.numitems - 1, -1);
                } else {
                    m_maps_select_next_active(MAPSMENU.numitems - 1, -1, false);
                }
                true
            }

            K_PGDN | K_KP_PGDN => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                if MAPSMENU.search.len != 0 {
                    m_maps_select_next_search_match(MAPSMENU.cursor + 1, 1);
                } else {
                    let sel = if MAPSMENU.cursor - MAPSMENU.scroll < MAPLIST_VIEWSIZE - 1 {
                        m_maps_select_next_active(MAPSMENU.scroll + MAPLIST_VIEWSIZE - 1, 1, false)
                    } else {
                        m_maps_select_next_active(MAPSMENU.cursor + MAPLIST_VIEWSIZE - 1, 1, false)
                    };
                    if !sel {
                        m_maps_select_next_active(MAPSMENU.numitems - 1, -1, false);
                    }
                }
                true
            }

            K_PGUP | K_KP_PGUP => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                if MAPSMENU.search.len != 0 {
                    m_maps_select_next_search_match(MAPSMENU.cursor - 1, -1);
                } else {
                    let sel = if MAPSMENU.cursor > MAPSMENU.scroll {
                        m_maps_select_next_active(MAPSMENU.scroll, -1, false)
                    } else {
                        m_maps_select_next_active(MAPSMENU.cursor - MAPLIST_VIEWSIZE + 1, -1, false)
                    };
                    if !sel {
                        m_maps_select_next_active(0, 1, false);
                    }
                }
                true
            }

            K_UPARROW | K_KP_UPARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                if MAPSMENU.search.len != 0 {
                    m_maps_select_next_search_match(MAPSMENU.cursor - 1, -1);
                } else {
                    m_maps_select_next_active(MAPSMENU.cursor - 1, -1, true);
                }
                true
            }

            K_DOWNARROW | K_KP_DOWNARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                if MAPSMENU.search.len != 0 {
                    m_maps_select_next_search_match(MAPSMENU.cursor + 1, 1);
                } else {
                    m_maps_select_next_active(MAPSMENU.cursor + 1, 1, true);
                }
                true
            }

            K_MWHEELUP => {
                if !overflow {
                    return false;
                }
                MAPSMENU.scroll -= 3;
                m_maps_clamp_scroll();
                m_maps_update_mouse_selection();
                true
            }

            K_MWHEELDOWN => {
                if !overflow {
                    return false;
                }
                MAPSMENU.scroll += 3;
                m_maps_clamp_scroll();
                m_maps_update_mouse_selection();
                true
            }

            _ => false,
        }
    }
}

/// `menu.c:3344` -- `static void M_Maps_Key (int key)`.
///
/// Non-raising: the deepest callees are `M_Menu_SinglePlayer_f`,
/// `S_LocalSound`, `Mod_LoadMapDescription` and the selection helpers, none
/// of which reach `Host_Error`/`Sys_Error`.
///
/// # Safety
/// Reads the module-owned item array.
unsafe fn m_maps_key(key: c_int) {
    // SAFETY: `mapsmenu.cursor` is only dereferenced under `numitems > 0`,
    // exactly as in the C.
    unsafe {
        if m_maps_list_key(key) {
            return;
        }

        if m_ticker_key(ptr::addr_of_mut!(MAPSMENU.ticker), key) {
            return;
        }

        match key {
            K_MOUSE2 | K_ESCAPE | K_BBUTTON => {
                m_menu_singleplayer_f();
            }

            K_MOUSE1 | K_ENTER | K_KP_ENTER | K_ABUTTON => {
                // `menu.c:3362` falls through from `K_MOUSE1` into the
                // accept case when the click is not on the scrollbar; the
                // guard is folded into this arm so the fallthrough is
                // explicit rather than implicit.
                if key == K_MOUSE1 && m_in_scrollbar() && m_maps_get_overflow() > 0 && !SLIDER_GRAB
                {
                    SCROLLBAR_GRAB = true;
                    let clamped_mouse =
                        clamp_i(SCROLLBAR_Y + 8, M_MOUSE_Y, SCROLLBAR_Y + SCROLLBAR_SIZE - 8);
                    MAPSMENU.scroll = as_i(
                        (clamped_mouse as c_float - SCROLLBAR_Y as c_float - 8.0)
                            / (SCROLLBAR_SIZE - 16) as c_float
                            * m_maps_get_overflow() as c_float
                            + 0.5,
                    );
                    m_maps_clamp_scroll();
                    m_maps_update_mouse_selection();
                    return;
                }

                if MAPSMENU.numitems > 0
                    && !(*MAPSMENU.items.add(MAPSMENU.cursor as usize))
                        .source
                        .is_null()
                {
                    let mapname = (*MAPSMENU.items.add(MAPSMENU.cursor as usize)).name;
                    m_maps_clear_search();
                    g::m_entersound = true;
                    m_set_skill_menu_map(mapname);
                    m_menu_skill_f();
                } else {
                    g::S_LocalSound(cstr!("misc/menu3.wav"));
                }
            }

            _ => {}
        }
    }
}

/// `menu.c:3390` -- `static void M_Maps_Char (int key)`.
///
/// # Safety
/// Writes the module-owned search buffer.
unsafe fn m_maps_char(key: c_int) {
    // SAFETY: the length check keeps the write inside `search.text`.
    unsafe {
        if MAPSMENU.numitems <= 0 {
            return;
        }

        // don't allow starting with a space
        if MAPSMENU.search.len <= 0 && key == b' ' as c_int {
            return;
        }

        if MAPSMENU.search.len >= MAPS_SEARCH_TEXT_SIZE as c_int - 1 {
            g::S_LocalSound(cstr!("misc/menu2.wav"));
            return;
        }

        let text = maps_search_text();
        *text.add(MAPSMENU.search.len as usize) = key as c_char;
        MAPSMENU.search.len += 1;
        *text.add(MAPSMENU.search.len as usize) = 0;

        if MAPSMENU.cursor < 0 {
            MAPSMENU.cursor = 0;
        }

        let mut start = MAPSMENU.cursor;
        if MAPSMENU.search.len == 1 {
            start += 1;
        }

        if !m_maps_select_next_search_match(start, 1) {
            MAPSMENU.search.len -= 1;
            *text.add(MAPSMENU.search.len as usize) = 0;
            g::S_LocalSound(cstr!("misc/menu2.wav"));
        }
    }
}

/// `menu.c:3428` -- `static qboolean M_Maps_TextEntry (void)`.
fn m_maps_text_entry() -> bool {
    true
}

/* ------------------------------------------------------------------------
 * menu.c:3433-3546 -- SKILL MENU.
 */

/// `menu.c:3436-3442` -- the skill-menu statics.
static mut M_SKILL_CURSOR: c_int = 0;
static mut M_SKILL_USEGFX: bool = false;
static mut M_SKILL_USECUSTOMTITLE: bool = false;
static mut M_SKILL_MAPNAME: [c_char; quake_types::fs::MAX_QPATH] = [0; quake_types::fs::MAX_QPATH];
/// `menu.c:3441` -- `char m_skill_maptitle[1024]`.
const M_SKILL_MAPTITLE_SIZE: usize = 1024;
static mut M_SKILL_MAPTITLE: [c_char; M_SKILL_MAPTITLE_SIZE] = [0; M_SKILL_MAPTITLE_SIZE];
static mut M_SKILL_TICKER: MenuTicker = MenuTicker {
    scroll_time: 0.0,
    scroll_wait_time: 0.0,
};
static mut M_SKILL_PREVMENU: c_int = 0;

/// `menu.c:3444` -- `static void M_SetSkillMenuMap (const char *name)`.
///
/// # Safety
/// `name` must be a NUL-terminated C string.
unsafe fn m_set_skill_menu_map(name: *const c_char) {
    // SAFETY: caller contract; both destinations are module-owned buffers.
    unsafe {
        let mapname = ptr::addr_of_mut!(M_SKILL_MAPNAME).cast::<c_char>();
        let maptitle = ptr::addr_of_mut!(M_SKILL_MAPTITLE).cast::<c_char>();
        g::q_strlcpy(mapname, name, quake_types::fs::MAX_QPATH);
        if !g::Mod_LoadMapDescription(maptitle, M_SKILL_MAPTITLE_SIZE, name) || *maptitle == 0 {
            g::q_strlcpy(maptitle, name, M_SKILL_MAPTITLE_SIZE);
        }
    }
}

/// `menu.c:3451` -- `static void M_Menu_Skill_f (void)`.
///
/// # Safety
/// Touches glue-owned menu state and the input subsystem.
unsafe fn m_menu_skill_f() {
    // SAFETY: single-threaded menu state.
    unsafe {
        quake_rs_menu_menu_changed();
        g::IN_Deactivate(true);
        g::key_dest = KEY_MENU;
        M_SKILL_PREVMENU = g::m_state;
        g::m_state = M_SKILL;
        g::m_entersound = true;
        m_ticker_init(ptr::addr_of_mut!(M_SKILL_TICKER));

        M_SKILL_CURSOR = as_i(g::skill.value);
        M_SKILL_CURSOR = clamp_i(0, M_SKILL_CURSOR, 3);
    }
}

/// `menu.c:3475` -- the non-gfx skill labels.
const SKILLS: [*const c_char; 4] = [
    cstr!("EASY"),
    cstr!("NORMAL"),
    cstr!("HARD"),
    cstr!("NIGHTMARE"),
];

/// `menu.c:3465` -- `static void M_Skill_Draw (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_skill_draw(cbx: *mut c_void) {
    // SAFETY: caller contract plus module-owned menu state.
    unsafe {
        quake_rs_menu_draw_trans_pic(cbx, 16, 4, g::Draw_CachePic(cstr!("gfx/qplaque.lmp")));
        let p = g::Draw_CachePic(if M_SKILL_USECUSTOMTITLE {
            cstr!("gfx/p_skill.lmp")
        } else {
            cstr!("gfx/ttl_sgl.lmp")
        });
        quake_rs_menu_draw_pic(cbx, (320 - pic_width(p)) / 2, 4, p);

        let x = 72;
        let mut y = 32;

        m_ticker_update(ptr::addr_of_mut!(M_SKILL_TICKER));
        m_print_scroll(
            cbx,
            x,
            y,
            30 * CHARACTER_SIZE,
            ptr::addr_of!(M_SKILL_MAPTITLE).cast::<c_char>(),
            M_SKILL_TICKER.scroll_time,
            false,
        );

        y += 16;

        if M_SKILL_USEGFX {
            quake_rs_menu_draw_trans_pic(cbx, x, y, g::Draw_CachePic(cstr!("gfx/skillmenu.lmp")));
            m_mouse_update_list_cursor(ptr::addr_of_mut!(M_SKILL_CURSOR), x, 320, y, 20, 4, 0);
            let f = as_i_d(g::realtime * 10.0) % 6;
            quake_rs_menu_draw_trans_pic(
                cbx,
                x - 18,
                y + M_SKILL_CURSOR * 20,
                g::Draw_CachePic(g::va(cstr!("gfx/menudot%i.lmp"), f + 1)),
            );
        } else {
            for f in 0..4 {
                quake_rs_menu_print(cbx, x, y + f * 16 + 2, SKILLS[f as usize]);
            }

            m_mouse_update_list_cursor(ptr::addr_of_mut!(M_SKILL_CURSOR), x, 320, y, 16, 4, 0);
            g::Draw_Character(
                cbx,
                (x - 16) as c_float,
                (y + M_SKILL_CURSOR * 16 + 4) as c_float,
                12 + (as_i_d(g::realtime * 4.0) & 1),
            );
        }
    }
}

/// `menu.c:3506` -- `static void M_Skill_Key (int key)`.
///
/// Non-raising: `Cbuf_AddText` is Rust and cannot longjmp, and the rest is
/// scalar menu state plus `S_LocalSound`.
///
/// # Safety
/// Touches glue-owned menu state and the input subsystem.
unsafe fn m_skill_key(key: c_int) {
    // SAFETY: single-threaded menu state.
    unsafe {
        if m_ticker_key(ptr::addr_of_mut!(M_SKILL_TICKER), key) {
            return;
        }

        match key {
            K_MOUSE2 | K_ESCAPE | K_BBUTTON => {
                g::m_state = M_SKILL_PREVMENU;
                g::m_entersound = true;
            }

            K_DOWNARROW | K_KP_DOWNARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                M_SKILL_CURSOR += 1;
                if M_SKILL_CURSOR > 3 {
                    M_SKILL_CURSOR = 0;
                }
            }

            K_UPARROW | K_KP_UPARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                M_SKILL_CURSOR -= 1;
                if M_SKILL_CURSOR < 0 {
                    M_SKILL_CURSOR = 3;
                }
            }

            K_MOUSE1 | K_ENTER | K_KP_ENTER | K_ABUTTON => {
                g::IN_Activate();
                g::key_dest = KEY_GAME;
                g::m_state = M_NONE;
                if sv_active() {
                    g::Cbuf_AddText(cstr!("disconnect\n"));
                }
                g::Cbuf_AddText(g::va(cstr!("skill %d\n"), M_SKILL_CURSOR));
                g::Cbuf_AddText(cstr!("maxplayers 1\n"));
                g::Cbuf_AddText(cstr!("deathmatch 0\n"));
                g::Cbuf_AddText(cstr!("coop 0\n"));
                g::Cbuf_AddText(g::va(
                    cstr!("map \"%s\"\n"),
                    ptr::addr_of!(M_SKILL_MAPNAME).cast::<c_char>(),
                ));
            }

            _ => {}
        }
    }
}

/* ------------------------------------------------------------------------
 * menu.c:3552-3663 -- QUIT MENU.
 */

/// `menu.c:3554-3556` -- the quit-menu statics.
static mut MSG_NUMBER: c_int = 0;
static mut M_QUIT_PREVSTATE: c_int = 0;
static mut WAS_IN_MENUS: bool = false;

/// `menu.c:3558` -- `void M_Menu_Quit_f (void)`.
///
/// Non-raising, and so returns `()` rather than a `Raise`: the body reaches
/// only `IN_Deactivate`, `COM_Rand` and `Cbuf_AddText`, and `Cbuf_AddText` is
/// Rust under `-Duse_rust_cvar` (`cvar_cmd.rs`), which cannot longjmp. The
/// pre-existing `HostCmd_Glue_M_Menu_Quit_f` guard (`host_cmd_glue.c:219`)
/// stays in place and simply never fires.
///
/// # Safety
/// Touches glue-owned menu state and the input subsystem.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_menu_quit_f() {
    // SAFETY: single-threaded menu state.
    unsafe {
        if g::m_state == M_QUIT {
            return;
        }
        if MOD_LOADED_FROM_MENU == 0 {
            WAS_IN_MENUS = g::key_dest == KEY_MENU;
            g::IN_Deactivate(true);
            g::key_dest = KEY_MENU;
            M_QUIT_PREVSTATE = g::m_state;
            g::m_state = M_QUIT;
            g::m_entersound = true;
            MSG_NUMBER = g::COM_Rand() & 7;
        } else {
            MOD_LOADED_FROM_MENU = 0;
            // `quakedef.h:36` -- `GAMENAME "id1"`, concatenated into the
            // literal by the C preprocessor.
            g::Cbuf_AddText(cstr!("game id1\n"));
        }
    }
}

/// `menu.c:3579` -- `static void M_Quit_Key (int key)`.
///
/// # Safety
/// Touches glue-owned menu state and the input subsystem.
unsafe fn m_quit_key(key: c_int) {
    // SAFETY: single-threaded menu state.
    unsafe {
        if key == K_ESCAPE {
            if WAS_IN_MENUS {
                g::m_state = M_QUIT_PREVSTATE;
                g::m_entersound = true;
            } else {
                g::IN_Activate();
                g::key_dest = KEY_GAME;
                g::m_state = M_NONE;
            }
        }
    }
}

/// `menu.c:3596` -- `static void M_Quit_Char (int key)`.
///
/// # Safety
/// Touches glue-owned menu state and the input subsystem.
unsafe fn m_quit_char(key: c_int) {
    // `menu.c:3598-3617` switches on plain character literals; spelled as
    // named `c_int` constants because Rust patterns cannot carry a cast.
    const N_LOWER: c_int = b'n' as c_int;
    const N_UPPER: c_int = b'N' as c_int;
    const Y_LOWER: c_int = b'y' as c_int;
    const Y_UPPER: c_int = b'Y' as c_int;

    // SAFETY: single-threaded menu state.
    unsafe {
        match key {
            N_LOWER | N_UPPER => {
                if WAS_IN_MENUS {
                    g::m_state = M_QUIT_PREVSTATE;
                    g::m_entersound = true;
                } else {
                    g::IN_Activate();
                    g::key_dest = KEY_GAME;
                    g::m_state = M_NONE;
                }
            }

            Y_LOWER | Y_UPPER | K_SPACE => {
                g::m_is_quitting = true;
                g::IN_Deactivate(true);
                g::key_dest = KEY_CONSOLE;
                g::Cbuf_InsertText(cstr!("quit"));
            }

            _ => {}
        }
    }
}

/// `menu.c:3624` -- `static qboolean M_Quit_TextEntry (void)`.
fn m_quit_text_entry() -> bool {
    true
}

/// `menu.c:3631` -- `char msg2[] = "by Axel Gneiting and devs";`, carried with
/// its NUL so `QUIT_MSG2.len()` is C's `sizeof (msg2)`.
const QUIT_MSG2: &[u8] = b"by Axel Gneiting and devs\0";
/// `menu.c:3632` -- `char msg3[] = "Press y/space to quit";`; see
/// [`QUIT_MSG2`].
const QUIT_MSG3: &[u8] = b"Press y/space to quit\0";

/// `menu.c:3629` -- `static void M_Quit_Draw (cb_context_t *cbx)`.
///
/// Propagates a `Raise`: `menu.c:3642` re-enters `M_Draw`, which reaches
/// `M_Video_Draw` (`gl_vidsdl.c`) and so is raise-capable.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_quit_draw(cbx: *mut c_void) -> Raise {
    // SAFETY: caller contract; `msg1` is a local buffer `q_snprintf` bounds.
    unsafe {
        let mut msg1: [c_char; 40] = [0; 40];

        if g::cl_confirmquit.value == 0.0 {
            return 0;
        }

        if WAS_IN_MENUS {
            g::m_state = M_QUIT_PREVSTATE;
            M_RECURSIVE_DRAW = true;
            raise!(quake_rs_menu_draw(cbx));
            g::m_state = M_QUIT;
        }

        // `ENGINE_NAME_AND_VER` is passed as the *format* string in the C, so
        // it keeps going through `q_snprintf` here rather than being copied.
        g::q_snprintf(
            msg1.as_mut_ptr(),
            core::mem::size_of_val(&msg1),
            g::Menu_Glue_EngineNameAndVer(),
        );

        // okay, this is kind of fucked up.  M_DrawTextBox will always act as if
        // width is even. Also, the width and lines values are for the interior of the box,
        // but the x and y values include the border.
        //
        // COMPAT: ADR-004 -- `menu.c:3653` evaluates the nested `q_max` in
        // `size_t` (`_Generic` picks the 64-bit unsigned overload for
        // `strlen`/`sizeof` operands), then narrows to `int`.
        let mut boxlen = (max_u64(
            g::strlen(msg1.as_ptr()) as u64,
            max_u64(QUIT_MSG2.len() as u64 - 1, QUIT_MSG3.len() as u64 - 1),
        ) + 1) as c_int;
        if boxlen & 1 != 0 {
            boxlen += 1;
        }
        m_draw_text_box(cbx, 160 - 4 * (boxlen + 2), 76, boxlen, 4);

        // now do the text
        //
        // COMPAT: ADR-004 -- `menu.c:3659-3661` compute `160 - 4 * strlen
        // (...)` in `size_t`, so an over-long string wraps through unsigned
        // 64-bit before narrowing to the `int` parameter.
        quake_rs_menu_print(
            cbx,
            160u64.wrapping_sub(4 * g::strlen(msg1.as_ptr()) as u64) as c_int,
            88,
            msg1.as_ptr(),
        );
        quake_rs_menu_print(
            cbx,
            160u64.wrapping_sub(4 * (QUIT_MSG2.len() as u64 - 1)) as c_int,
            96,
            QUIT_MSG2.as_ptr().cast::<c_char>(),
        );
        m_print_white(
            cbx,
            160u64.wrapping_sub(4 * (QUIT_MSG3.len() as u64 - 1)) as c_int,
            104,
            QUIT_MSG3.as_ptr().cast::<c_char>(),
        );

        0
    }
}

/* ------------------------------------------------------------------------
 * menu.c:3671-3976 -- LAN CONFIG MENU.
 */

/// `menu.c:95` -- `#define StartingGame (m_multiplayer_cursor == 1)`.
///
/// # Safety
/// Reads the single-threaded multiplayer-menu cursor.
#[inline]
unsafe fn starting_game() -> bool {
    // SAFETY: single-threaded menu state.
    unsafe { M_MULTIPLAYER_CURSOR == 1 }
}

/// `menu.c:96` -- `#define JoiningGame (m_multiplayer_cursor == 0)`.
///
/// # Safety
/// Reads the single-threaded multiplayer-menu cursor.
#[inline]
unsafe fn joining_game() -> bool {
    // SAFETY: single-threaded menu state.
    unsafe { M_MULTIPLAYER_CURSOR == 0 }
}

/// `menu.c:97` -- `#define TCPIPConfig (m_net_cursor == 1)`.
///
/// # Safety
/// Reads the single-threaded net-menu cursor.
#[inline]
unsafe fn tcpip_config() -> bool {
    // SAFETY: single-threaded menu state.
    unsafe { M_NET_CURSOR == 1 }
}

/// `menu.c:3673` -- `static int lan_config_cursor = -1;`.
static mut LAN_CONFIG_CURSOR: c_int = -1;
/// `menu.c:3674` -- `#define NUM_LANCONFIG_CMDS 4`.
const NUM_LANCONFIG_CMDS: c_int = 4;

/// `menu.c:3676` -- `static int lan_config_port;`.
static mut LAN_CONFIG_PORT: c_int = 0;
/// `menu.c:3677` -- `static char lan_config_portname[5 + 1];`.
const LAN_CONFIG_PORTNAME_SIZE: usize = 5 + 1;
static mut LAN_CONFIG_PORTNAME: [c_char; LAN_CONFIG_PORTNAME_SIZE] = [0; LAN_CONFIG_PORTNAME_SIZE];
/// `menu.c:3678` -- `static char lan_config_joinname[36 + 1];`.
const LAN_CONFIG_JOINNAME_SIZE: usize = 36 + 1;
static mut LAN_CONFIG_JOINNAME: [c_char; LAN_CONFIG_JOINNAME_SIZE] = [0; LAN_CONFIG_JOINNAME_SIZE];

/// Base pointer to [`LAN_CONFIG_PORTNAME`] without forming a reference to a
/// `static mut` (`static_mut_refs`).
///
/// # Safety
/// The buffer is single-threaded menu state.
#[inline]
unsafe fn lan_config_portname() -> *mut c_char {
    // Address-of only; no reference to the `static mut` is created.
    ptr::addr_of_mut!(LAN_CONFIG_PORTNAME).cast::<c_char>()
}

/// Base pointer to [`LAN_CONFIG_JOINNAME`]; see [`lan_config_portname`].
///
/// # Safety
/// The buffer is single-threaded menu state.
#[inline]
unsafe fn lan_config_joinname() -> *mut c_char {
    // Address-of only; no reference to the `static mut` is created.
    ptr::addr_of_mut!(LAN_CONFIG_JOINNAME).cast::<c_char>()
}

/// `menu.c:3680` -- `static void M_Menu_LanConfig_f (void)`.
///
/// # Safety
/// Touches glue-owned menu state and the input subsystem.
unsafe fn m_menu_lanconfig_f() {
    // SAFETY: single-threaded menu state.
    unsafe {
        quake_rs_menu_menu_changed();
        g::IN_Deactivate(true);
        g::key_dest = KEY_MENU;
        g::m_state = M_LANCONFIG;
        if LAN_CONFIG_CURSOR == -1 {
            if joining_game() && tcpip_config() {
                LAN_CONFIG_CURSOR = 2;
            } else {
                LAN_CONFIG_CURSOR = 1;
            }
        }
        if starting_game() && LAN_CONFIG_CURSOR >= 2 {
            LAN_CONFIG_CURSOR = 1;
        }
        LAN_CONFIG_PORT = g::DEFAULTnet_hostport;
        g::q_snprintf(
            lan_config_portname(),
            LAN_CONFIG_PORTNAME_SIZE,
            cstr!("%u"),
            LAN_CONFIG_PORT,
        );

        g::m_return_onerror = false;
        g::m_return_reason[0] = 0;
    }
}

/// `menu.c:3702` -- `static void M_LanConfig_Draw (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_lan_config_draw(cbx: *mut c_void) {
    // SAFETY: caller contract; every path name is a static literal and the
    // address array is a local `qhostaddr_t[16]` handed to `NET_ListAddresses`
    // exactly as the C does.
    unsafe {
        let mut addresses: [[c_char; g::NET_NAMELEN]; 16] = [[0; g::NET_NAMELEN]; 16];

        quake_rs_menu_draw_trans_pic(cbx, 16, 4, g::Draw_CachePic(cstr!("gfx/qplaque.lmp")));
        let p = g::Draw_CachePic(cstr!("gfx/p_multi.lmp"));
        let mut basex = (320 - pic_width(p)) / 2;
        quake_rs_menu_draw_pic(cbx, basex, 4, p);
        //
        basex = 72 - (8 * 2);

        let start_join = if starting_game() {
            cstr!("New Game")
        } else {
            cstr!("Join Game")
        };
        let protocol = cstr!("TCP/IP");
        quake_rs_menu_print(
            cbx,
            basex + 8 * 4,
            32,
            g::va(cstr!("%s - %s"), start_join, protocol),
        );
        basex += 8;

        let mut y = 52;
        quake_rs_menu_print(cbx, basex, y, cstr!("Address:"));
        let numaddresses = g::NET_ListAddresses(addresses.as_mut_ptr().cast::<c_void>(), 16);
        if numaddresses == 0 {
            quake_rs_menu_print(cbx, basex + 9 * 8, y, cstr!("NONE KNOWN"));
            y += 8;
        } else {
            for i in 0..numaddresses {
                quake_rs_menu_print(
                    cbx,
                    basex + 9 * 8,
                    y,
                    addresses[i as usize].as_ptr().cast::<c_char>(),
                );
                y += 8;
            }
        }

        y += 8; // for the port's box
        quake_rs_menu_print(cbx, basex, y, cstr!("Port"));
        m_draw_text_box(
            cbx,
            basex + 8 * 8,
            y - 8,
            LAN_CONFIG_PORTNAME_SIZE as c_int,
            1,
        );
        quake_rs_menu_print(cbx, basex + 9 * 8, y, lan_config_portname());
        quake_rs_menu_mouse_update_cursor(
            ptr::addr_of_mut!(LAN_CONFIG_CURSOR),
            basex,
            320,
            y,
            8,
            0,
        );
        if LAN_CONFIG_CURSOR == 0 {
            g::Draw_Character(
                cbx,
                (basex as usize + 9 * 8 + 8 * g::strlen(lan_config_portname())) as c_float,
                y as c_float,
                10 + (as_i_d(g::realtime * 4.0) & 1),
            );
            g::Draw_Character(
                cbx,
                (basex - 8) as c_float,
                y as c_float,
                12 + (as_i_d(g::realtime * 4.0) & 1),
            );
        }
        y += 20;

        if joining_game() {
            quake_rs_menu_print(cbx, basex, y, cstr!("Search for local games..."));
            quake_rs_menu_mouse_update_cursor(
                ptr::addr_of_mut!(LAN_CONFIG_CURSOR),
                basex,
                320,
                y,
                8,
                1,
            );
            if LAN_CONFIG_CURSOR == 1 {
                g::Draw_Character(
                    cbx,
                    (basex - 8) as c_float,
                    y as c_float,
                    12 + (as_i_d(g::realtime * 4.0) & 1),
                );
            }
            y += 8;

            quake_rs_menu_print(cbx, basex, y, cstr!("Search for public games..."));
            quake_rs_menu_mouse_update_cursor(
                ptr::addr_of_mut!(LAN_CONFIG_CURSOR),
                basex,
                320,
                y,
                8,
                2,
            );
            if LAN_CONFIG_CURSOR == 2 {
                g::Draw_Character(
                    cbx,
                    (basex - 8) as c_float,
                    y as c_float,
                    12 + (as_i_d(g::realtime * 4.0) & 1),
                );
            }
            y += 24;

            quake_rs_menu_print(cbx, basex, y, cstr!("Join game at:"));
            y += 12;
            m_draw_text_box(cbx, basex + 8, y - 8, LAN_CONFIG_JOINNAME_SIZE as c_int, 1);
            quake_rs_menu_print(cbx, basex + 16, y, lan_config_joinname());
            quake_rs_menu_mouse_update_cursor(
                ptr::addr_of_mut!(LAN_CONFIG_CURSOR),
                basex,
                320,
                y,
                8,
                3,
            );
            if LAN_CONFIG_CURSOR == 3 {
                g::Draw_Character(
                    cbx,
                    (basex as usize + 16 + 8 * g::strlen(lan_config_joinname())) as c_float,
                    y as c_float,
                    10 + (as_i_d(g::realtime * 4.0) & 1),
                );
                g::Draw_Character(
                    cbx,
                    (basex - 8) as c_float,
                    y as c_float,
                    12 + (as_i_d(g::realtime * 4.0) & 1),
                );
            }
            y += 16;
        } else {
            m_draw_text_box(cbx, basex, y - 8, 2, 1);
            quake_rs_menu_print(cbx, basex + 8, y, cstr!("OK"));
            quake_rs_menu_mouse_update_cursor(
                ptr::addr_of_mut!(LAN_CONFIG_CURSOR),
                basex,
                320,
                y,
                8,
                1,
            );
            if LAN_CONFIG_CURSOR == 1 {
                g::Draw_Character(
                    cbx,
                    (basex - 8) as c_float,
                    y as c_float,
                    12 + (as_i_d(g::realtime * 4.0) & 1),
                );
            }
            y += 16;
        }

        // `menu.c:3778`/`:3787` -- the trailing `y += 16` in both arms is a dead
        // store, exactly as in the C.
        let _ = y;

        if g::m_return_reason[0] != 0 {
            m_print_white(
                cbx,
                basex,
                148,
                ptr::addr_of!(g::m_return_reason).cast::<c_char>(),
            );
        }
    }
}

/// `menu.c:3794` -- `static void validate_LanConfig (void)`.
///
/// # Safety
/// Rewrites the single-threaded LAN-config buffers.
unsafe fn validate_lan_config() {
    // SAFETY: single-threaded menu state; `q_strsplit` writes only inside the
    // local copy and returns a `Mem_Alloc`'d index the C frees the same way.
    unsafe {
        // make a copy of lan_config_joinname because of q_strsplit / q_strtrim on-place modification.
        let mut raw_join_address: [c_char; LAN_CONFIG_JOINNAME_SIZE] =
            [0; LAN_CONFIG_JOINNAME_SIZE];
        g::q_strlcpy(
            raw_join_address.as_mut_ptr(),
            lan_config_joinname(),
            LAN_CONFIG_JOINNAME_SIZE,
        );

        // Check if the resulting raw_join_address is of form 'address:port', in this case overwrite lan_config_portname with it
        let mut nb_parts: usize = 0;

        let split_address = g::q_strsplit(
            raw_join_address.as_mut_ptr(),
            cstr!(":"),
            ptr::addr_of_mut!(nb_parts),
        );

        if nb_parts == 2
            && g::atoi(*split_address.add(1)) > 0
            && g::atoi(*split_address.add(1)) <= 65535
        {
            // set join name from the first part:
            g::q_strlcpy(
                lan_config_joinname(),
                g::q_strtrim(*split_address),
                LAN_CONFIG_JOINNAME_SIZE,
            );

            // overwrite existing port value from the second part:
            g::q_strlcpy(
                lan_config_portname(),
                g::q_strtrim(*split_address.add(1)),
                LAN_CONFIG_PORTNAME_SIZE,
            );
        } else {
            g::q_strlcpy(
                lan_config_joinname(),
                g::q_strtrim(raw_join_address.as_mut_ptr()),
                LAN_CONFIG_JOINNAME_SIZE,
            );
        }
        g::Mem_Free(split_address.cast::<c_void>());
    }
}

/// `menu.c:3821` -- `static void M_LanConfig_Key (int key)`.
///
/// Non-raising: the deepest callees are `M_Menu_Net_f`, `S_LocalSound`,
/// `M_ConfigureNetSubsystem` (`Cbuf_AddText` plus one `int` store),
/// `M_Menu_MPGameOptions_f`, and `M_Menu_Search_f`. `M_Menu_Search_f` ends in
/// `NET_Slist_f`, which stays an unguarded C call by the M10e seam decision --
/// its trace is recorded in the module doc.
///
/// # Safety
/// Touches glue-owned menu state, the clipboard and the input subsystem.
unsafe fn m_lan_config_key(key: c_int) {
    // `menu.c:3898-3899` switches on plain character literals; spelled as named
    // `c_int` constants because Rust patterns cannot carry a cast.
    const V_LOWER: c_int = b'v' as c_int;
    const V_UPPER: c_int = b'V' as c_int;

    // SAFETY: single-threaded menu state; every buffer write is bounded exactly
    // as in the C.
    unsafe {
        match key {
            K_MOUSE2 | K_ESCAPE | K_BBUTTON => {
                m_menu_net_f();
            }

            K_UPARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                LAN_CONFIG_CURSOR -= 1;
                if LAN_CONFIG_CURSOR < 0 {
                    LAN_CONFIG_CURSOR = NUM_LANCONFIG_CMDS - 1;
                }
            }

            K_DOWNARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                LAN_CONFIG_CURSOR += 1;
                if LAN_CONFIG_CURSOR >= NUM_LANCONFIG_CMDS {
                    LAN_CONFIG_CURSOR = 0;
                }
            }

            K_MOUSE1 | K_ENTER | K_KP_ENTER | K_ABUTTON => 'enter: {
                if LAN_CONFIG_CURSOR == 0 {
                    break 'enter;
                }

                g::m_entersound = true;

                m_configure_net_subsystem();

                if starting_game() {
                    if LAN_CONFIG_CURSOR == 1 {
                        m_menu_mpgameoptions_f();
                    }
                } else if LAN_CONFIG_CURSOR == 1 {
                    m_menu_search_f(g::SLIST_LAN);
                } else if LAN_CONFIG_CURSOR == 2 {
                    m_menu_search_f(g::SLIST_INTERNET);
                } else if LAN_CONFIG_CURSOR == 3 {
                    validate_lan_config();
                    g::m_return_state = g::m_state;
                    g::m_return_onerror = true;
                    g::IN_Activate();
                    g::key_dest = KEY_GAME;
                    g::m_state = M_NONE;
                    g::Cbuf_AddText(g::va(cstr!("connect \"%s\"\n"), lan_config_joinname()));
                }
            }

            K_BACKSPACE => {
                if LAN_CONFIG_CURSOR == 0 {
                    let l = g::strlen(lan_config_portname());
                    if l != 0 {
                        *lan_config_portname().add(l - 1) = 0;
                    }
                }

                if LAN_CONFIG_CURSOR == 3 {
                    let l = g::strlen(lan_config_joinname());
                    if l != 0 {
                        *lan_config_joinname().add(l - 1) = 0;
                    }
                }
            }

            V_LOWER | V_UPPER => 'paste: {
                // Ctrl + v : paste a hostname
                if LAN_CONFIG_CURSOR != 3 || !lan_config_paste_modifier_down() {
                    break 'paste;
                }
                {
                    // COMPAT: ADR-004 -- `menu.c:3908` narrows `strlen` to
                    // `int`; the buffer is 37 bytes so the value is always in
                    // range, and `joinname_remaining_room_size` below is the
                    // same `int` subtraction the C performs.
                    let current_joinname_size = g::strlen(lan_config_joinname()) as c_int;

                    let joinname_remaining_room_size =
                        LAN_CONFIG_JOINNAME_SIZE as c_int - 1 - current_joinname_size;

                    if joinname_remaining_room_size > 0 {
                        let clipboard_text = g::PL_GetClipboardData();

                        // append the existing clipboard text
                        if !clipboard_text.is_null() {
                            g::q_strlcpy(
                                lan_config_joinname().add(current_joinname_size as usize),
                                clipboard_text,
                                joinname_remaining_room_size as usize,
                            );
                        }

                        // Check if the resulting raw_join_address is of form 'address:port', in this case overwrite lan_config_portname with it
                        validate_lan_config();

                        g::Mem_Free(clipboard_text.cast::<c_void>());
                    } // end if enough room to Ctrl+v
                }
            }

            _ => {}
        }

        if starting_game() && LAN_CONFIG_CURSOR >= 2 {
            if key == K_UPARROW {
                LAN_CONFIG_CURSOR = 1;
            } else {
                LAN_CONFIG_CURSOR = 0;
            }
        }

        let mut l = g::atoi(lan_config_portname());
        if l > 65535 {
            l = LAN_CONFIG_PORT;
        } else {
            LAN_CONFIG_PORT = l;
        }
        g::q_snprintf(
            lan_config_portname(),
            LAN_CONFIG_PORTNAME_SIZE,
            cstr!("%u"),
            LAN_CONFIG_PORT,
        );
        let _ = l;
    }
}

/// `menu.c:3903-3907` -- the paste chord is `K_COMMAND` on Apple platforms and
/// `K_CTRL` everywhere else.
///
/// # Safety
/// Reads the engine's single-threaded `keydown` array.
#[inline]
unsafe fn lan_config_paste_modifier_down() -> bool {
    // SAFETY: `keydown` is a 256-entry array and both indices are constants
    // below 256.
    unsafe {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            g::keydown[K_COMMAND as usize]
        }
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        {
            g::keydown[K_CTRL as usize]
        }
    }
}

/// `menu.c:3945` -- `static void M_LanConfig_Char (int key)`.
///
/// # Safety
/// Writes the single-threaded LAN-config buffers.
unsafe fn m_lan_config_char(key: c_int) {
    // `menu.c:3952` compares against the digit characters.
    const ZERO: c_int = b'0' as c_int;
    const NINE: c_int = b'9' as c_int;

    // SAFETY: single-threaded menu state; both writes stay inside the buffer
    // because `l + 1 < countof (buf)`.
    unsafe {
        match LAN_CONFIG_CURSOR {
            0 => {
                if !(ZERO..=NINE).contains(&key) {
                    return;
                }
                let l = g::strlen(lan_config_portname());
                // append one character, assure null-termination
                if l < LAN_CONFIG_PORTNAME_SIZE - 1 {
                    *lan_config_portname().add(l + 1) = 0;
                    *lan_config_portname().add(l) = key as c_char;
                }
            }
            3 => {
                let l = g::strlen(lan_config_joinname());
                if l < LAN_CONFIG_JOINNAME_SIZE - 1 {
                    *lan_config_joinname().add(l + 1) = 0;
                    *lan_config_joinname().add(l) = key as c_char;
                }
            }
            _ => {}
        }
    }
}

/// `menu.c:3973` -- `static qboolean M_LanConfig_TextEntry (void)`.
///
/// # Safety
/// Reads the single-threaded LAN-config cursor.
unsafe fn m_lan_config_text_entry() -> bool {
    // SAFETY: single-threaded menu state.
    unsafe { LAN_CONFIG_CURSOR == 0 || LAN_CONFIG_CURSOR == 3 }
}

/* ------------------------------------------------------------------------
 * menu.c:3978-4402 -- GAME OPTIONS MENU.
 */

/// `menu.c:3981-3985` -- `typedef struct { const char *name; const char
/// *description; } level_t;`.
struct Level {
    name: *const c_char,
    description: *const c_char,
}

/// `menu.c:3987` -- `static level_t levels[]`.
const LEVELS: [Level; 38] = [
    Level {
        name: cstr!("start"),
        description: cstr!("Entrance"),
    }, // 0
    Level {
        name: cstr!("e1m1"),
        description: cstr!("Slipgate Complex"),
    }, // 1
    Level {
        name: cstr!("e1m2"),
        description: cstr!("Castle of the Damned"),
    },
    Level {
        name: cstr!("e1m3"),
        description: cstr!("The Necropolis"),
    },
    Level {
        name: cstr!("e1m4"),
        description: cstr!("The Grisly Grotto"),
    },
    Level {
        name: cstr!("e1m5"),
        description: cstr!("Gloom Keep"),
    },
    Level {
        name: cstr!("e1m6"),
        description: cstr!("The Door To Chthon"),
    },
    Level {
        name: cstr!("e1m7"),
        description: cstr!("The House of Chthon"),
    },
    Level {
        name: cstr!("e1m8"),
        description: cstr!("Ziggurat Vertigo"),
    },
    Level {
        name: cstr!("e2m1"),
        description: cstr!("The Installation"),
    }, // 9
    Level {
        name: cstr!("e2m2"),
        description: cstr!("Ogre Citadel"),
    },
    Level {
        name: cstr!("e2m3"),
        description: cstr!("Crypt of Decay"),
    },
    Level {
        name: cstr!("e2m4"),
        description: cstr!("The Ebon Fortress"),
    },
    Level {
        name: cstr!("e2m5"),
        description: cstr!("The Wizard's Manse"),
    },
    Level {
        name: cstr!("e2m6"),
        description: cstr!("The Dismal Oubliette"),
    },
    Level {
        name: cstr!("e2m7"),
        description: cstr!("Underearth"),
    },
    Level {
        name: cstr!("e3m1"),
        description: cstr!("Termination Central"),
    }, // 16
    Level {
        name: cstr!("e3m2"),
        description: cstr!("The Vaults of Zin"),
    },
    Level {
        name: cstr!("e3m3"),
        description: cstr!("The Tomb of Terror"),
    },
    Level {
        name: cstr!("e3m4"),
        description: cstr!("Satan's Dark Delight"),
    },
    Level {
        name: cstr!("e3m5"),
        description: cstr!("Wind Tunnels"),
    },
    Level {
        name: cstr!("e3m6"),
        description: cstr!("Chambers of Torment"),
    },
    Level {
        name: cstr!("e3m7"),
        description: cstr!("The Haunted Halls"),
    },
    Level {
        name: cstr!("e4m1"),
        description: cstr!("The Sewage System"),
    }, // 23
    Level {
        name: cstr!("e4m2"),
        description: cstr!("The Tower of Despair"),
    },
    Level {
        name: cstr!("e4m3"),
        description: cstr!("The Elder God Shrine"),
    },
    Level {
        name: cstr!("e4m4"),
        description: cstr!("The Palace of Hate"),
    },
    Level {
        name: cstr!("e4m5"),
        description: cstr!("Hell's Atrium"),
    },
    Level {
        name: cstr!("e4m6"),
        description: cstr!("The Pain Maze"),
    },
    Level {
        name: cstr!("e4m7"),
        description: cstr!("Azure Agony"),
    },
    Level {
        name: cstr!("e4m8"),
        description: cstr!("The Nameless City"),
    },
    Level {
        name: cstr!("end"),
        description: cstr!("Shub-Niggurath's Pit"),
    }, // 31
    Level {
        name: cstr!("dm1"),
        description: cstr!("Place of Two Deaths"),
    }, // 32
    Level {
        name: cstr!("dm2"),
        description: cstr!("Claustrophobopolis"),
    },
    Level {
        name: cstr!("dm3"),
        description: cstr!("The Abandoned Base"),
    },
    Level {
        name: cstr!("dm4"),
        description: cstr!("The Bad Place"),
    },
    Level {
        name: cstr!("dm5"),
        description: cstr!("The Cistern"),
    },
    Level {
        name: cstr!("dm6"),
        description: cstr!("The Dark Zone"),
    },
];

/// `menu.c:4034` -- `static level_t hipnoticlevels[]`.
// MED 01/06/97 added hipnotic levels
const HIPNOTICLEVELS: [Level; 18] = [
    Level {
        name: cstr!("start"),
        description: cstr!("Command HQ"),
    }, // 0
    Level {
        name: cstr!("hip1m1"),
        description: cstr!("The Pumping Station"),
    }, // 1
    Level {
        name: cstr!("hip1m2"),
        description: cstr!("Storage Facility"),
    },
    Level {
        name: cstr!("hip1m3"),
        description: cstr!("The Lost Mine"),
    },
    Level {
        name: cstr!("hip1m4"),
        description: cstr!("Research Facility"),
    },
    Level {
        name: cstr!("hip1m5"),
        description: cstr!("Military Complex"),
    },
    Level {
        name: cstr!("hip2m1"),
        description: cstr!("Ancient Realms"),
    }, // 6
    Level {
        name: cstr!("hip2m2"),
        description: cstr!("The Black Cathedral"),
    },
    Level {
        name: cstr!("hip2m3"),
        description: cstr!("The Catacombs"),
    },
    Level {
        name: cstr!("hip2m4"),
        description: cstr!("The Crypt"),
    },
    Level {
        name: cstr!("hip2m5"),
        description: cstr!("Mortum's Keep"),
    },
    Level {
        name: cstr!("hip2m6"),
        description: cstr!("The Gremlin's Domain"),
    },
    Level {
        name: cstr!("hip3m1"),
        description: cstr!("Tur Torment"),
    }, // 12
    Level {
        name: cstr!("hip3m2"),
        description: cstr!("Pandemonium"),
    },
    Level {
        name: cstr!("hip3m3"),
        description: cstr!("Limbo"),
    },
    Level {
        name: cstr!("hip3m4"),
        description: cstr!("The Gauntlet"),
    },
    Level {
        name: cstr!("hipend"),
        description: cstr!("Armagon's Lair"),
    }, // 16
    Level {
        name: cstr!("hipdm1"),
        description: cstr!("The Edge of Oblivion"),
    }, // 17
];

/// `menu.c:4062` -- `static level_t roguelevels[]`.
// PGM 01/07/97 added rogue levels
// PGM 03/02/97 added dmatch level
const ROGUELEVELS: [Level; 17] = [
    Level {
        name: cstr!("start"),
        description: cstr!("Split Decision"),
    },
    Level {
        name: cstr!("r1m1"),
        description: cstr!("Deviant's Domain"),
    },
    Level {
        name: cstr!("r1m2"),
        description: cstr!("Dread Portal"),
    },
    Level {
        name: cstr!("r1m3"),
        description: cstr!("Judgement Call"),
    },
    Level {
        name: cstr!("r1m4"),
        description: cstr!("Cave of Death"),
    },
    Level {
        name: cstr!("r1m5"),
        description: cstr!("Towers of Wrath"),
    },
    Level {
        name: cstr!("r1m6"),
        description: cstr!("Temple of Pain"),
    },
    Level {
        name: cstr!("r1m7"),
        description: cstr!("Tomb of the Overlord"),
    },
    Level {
        name: cstr!("r2m1"),
        description: cstr!("Tempus Fugit"),
    },
    Level {
        name: cstr!("r2m2"),
        description: cstr!("Elemental Fury I"),
    },
    Level {
        name: cstr!("r2m3"),
        description: cstr!("Elemental Fury II"),
    },
    Level {
        name: cstr!("r2m4"),
        description: cstr!("Curse of Osiris"),
    },
    Level {
        name: cstr!("r2m5"),
        description: cstr!("Wizard's Keep"),
    },
    Level {
        name: cstr!("r2m6"),
        description: cstr!("Blood Sacrifice"),
    },
    Level {
        name: cstr!("r2m7"),
        description: cstr!("Last Bastion"),
    },
    Level {
        name: cstr!("r2m8"),
        description: cstr!("Source of Evil"),
    },
    Level {
        name: cstr!("ctf1"),
        description: cstr!("Division of Change"),
    },
];

/// `menu.c:4068-4073` -- `typedef struct { const char *description; int
/// firstLevel; int levels; } episode_t;`.
struct Episode {
    description: *const c_char,
    first_level: c_int,
    levels: c_int,
}

/// `menu.c:4075` -- `static episode_t episodes[]`.
const EPISODES: [Episode; 7] = [
    Episode {
        description: cstr!("Welcome to Quake"),
        first_level: 0,
        levels: 1,
    },
    Episode {
        description: cstr!("Doomed Dimension"),
        first_level: 1,
        levels: 8,
    },
    Episode {
        description: cstr!("Realm of Black Magic"),
        first_level: 9,
        levels: 7,
    },
    Episode {
        description: cstr!("Netherworld"),
        first_level: 16,
        levels: 7,
    },
    Episode {
        description: cstr!("The Elder World"),
        first_level: 23,
        levels: 8,
    },
    Episode {
        description: cstr!("Final Level"),
        first_level: 31,
        levels: 1,
    },
    Episode {
        description: cstr!("Deathmatch Arena"),
        first_level: 32,
        levels: 6,
    },
];

/// `menu.c:4079` -- `static episode_t hipnoticepisodes[]`.
// MED 01/06/97  added hipnotic episodes
const HIPNOTICEPISODES: [Episode; 6] = [
    Episode {
        description: cstr!("Scourge of Armagon"),
        first_level: 0,
        levels: 1,
    },
    Episode {
        description: cstr!("Fortress of the Dead"),
        first_level: 1,
        levels: 5,
    },
    Episode {
        description: cstr!("Dominion of Darkness"),
        first_level: 6,
        levels: 6,
    },
    Episode {
        description: cstr!("The Rift"),
        first_level: 12,
        levels: 4,
    },
    Episode {
        description: cstr!("Final Level"),
        first_level: 16,
        levels: 1,
    },
    Episode {
        description: cstr!("Deathmatch Arena"),
        first_level: 17,
        levels: 1,
    },
];

/// `menu.c:4084` -- `static episode_t rogueepisodes[]`.
// PGM 01/07/97 added rogue episodes
// PGM 03/02/97 added dmatch episode
const ROGUEEPISODES: [Episode; 4] = [
    Episode {
        description: cstr!("Introduction"),
        first_level: 0,
        levels: 1,
    },
    Episode {
        description: cstr!("Hell's Fortress"),
        first_level: 1,
        levels: 7,
    },
    Episode {
        description: cstr!("Corridors of Time"),
        first_level: 8,
        levels: 8,
    },
    Episode {
        description: cstr!("Deathmatch Arena"),
        first_level: 16,
        levels: 1,
    },
];

/// `menu.c:4086-4088` -- the game-options selection statics.
static mut STARTEPISODE: c_int = 0;
static mut STARTLEVEL: c_int = 0;
static mut MAXPLAYERS: c_int = 0;

/// `menu.c:4090` -- `static int mpgameoptions_cursor_table[]`; never written,
/// so it is a `const` here.
const MPGAMEOPTIONS_CURSOR_TABLE: [c_int; 9] = [40, 56, 64, 72, 80, 88, 96, 112, 120];
/// `menu.c:4091` -- `#define NUM_MPGAMEOPTIONS 9`.
const NUM_MPGAMEOPTIONS: c_int = 9;
/// `menu.c:4092` -- `static int mpgameoptions_cursor;`.
static mut MPGAMEOPTIONS_CURSOR: c_int = 0;

/// `menu.c:4094` -- `static void M_Menu_MPGameOptions_f (void)`.
///
/// # Safety
/// Touches glue-owned menu state and the input subsystem.
unsafe fn m_menu_mpgameoptions_f() {
    // SAFETY: single-threaded menu state.
    unsafe {
        quake_rs_menu_menu_changed();
        g::IN_Deactivate(true);
        g::key_dest = KEY_MENU;
        g::m_state = M_MPGAMEOPTIONS;
        if MAXPLAYERS == 0 {
            MAXPLAYERS = svs_maxclients();
        }
        if MAXPLAYERS < 2 {
            MAXPLAYERS = 4;
        }
    }
}

/// `menu.c:4109` -- `#define OPTION_NAMES_CX 64`.
const OPTION_NAMES_CX: c_int = 64;
/// `menu.c:4110` -- `#define OPTION_VALUES_CX 176`.
const OPTION_VALUES_CX: c_int = 176;

/// `menu.c:4106` -- `static void M_MPGameOptions_Draw (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_mpgameoptions_draw(cbx: *mut c_void) {
    // SAFETY: caller contract; every table index is bounded by the same
    // `M_NetStart_Change` clamping the C relies on.
    unsafe {
        quake_rs_menu_draw_trans_pic(cbx, 16, 4, g::Draw_CachePic(cstr!("gfx/qplaque.lmp")));
        let p = g::Draw_CachePic(cstr!("gfx/p_multi.lmp"));
        quake_rs_menu_draw_pic(cbx, (320 - pic_width(p)) / 2, 4, p);

        m_draw_text_box(cbx, OPTION_NAMES_CX - 4, 32, 10, 1);
        quake_rs_menu_print(cbx, OPTION_NAMES_CX + 4, 40, cstr!("begin game"));

        quake_rs_menu_print(cbx, OPTION_NAMES_CX, 56, cstr!("Max players"));
        quake_rs_menu_print(cbx, OPTION_VALUES_CX, 56, g::va(cstr!("%i"), MAXPLAYERS));

        quake_rs_menu_print(cbx, OPTION_NAMES_CX, 64, cstr!("Game Type"));
        if g::coop.value != 0.0 {
            quake_rs_menu_print(cbx, OPTION_VALUES_CX, 64, cstr!("Cooperative"));
        } else {
            quake_rs_menu_print(cbx, OPTION_VALUES_CX, 64, cstr!("Deathmatch"));
        }

        quake_rs_menu_print(cbx, OPTION_NAMES_CX, 72, cstr!("Teamplay"));
        if g::rogue {
            // COMPAT: ADR-004 -- `menu.c:4133` casts `teamplay.value` to `int`,
            // which is UB for an out-of-range float; `as_i` saturates instead
            // and every saturated result lands in the `default` arm, exactly
            // where an in-range absurd value would.
            let msg = match as_i(g::teamplay.value) {
                1 => cstr!("No Friendly Fire"),
                2 => cstr!("Friendly Fire"),
                3 => cstr!("Tag"),
                4 => cstr!("Capture the Flag"),
                5 => cstr!("One Flag CTF"),
                6 => cstr!("Three Team CTF"),
                _ => cstr!("Off"),
            };
            quake_rs_menu_print(cbx, OPTION_VALUES_CX, 72, msg);
        } else {
            // COMPAT: ADR-004 -- see the `rogue` arm above; `menu.c:4163`.
            let msg = match as_i(g::teamplay.value) {
                1 => cstr!("No Friendly Fire"),
                2 => cstr!("Friendly Fire"),
                _ => cstr!("Off"),
            };
            quake_rs_menu_print(cbx, OPTION_VALUES_CX, 72, msg);
        }

        quake_rs_menu_print(cbx, OPTION_NAMES_CX, 80, cstr!("Skill"));
        if g::skill.value == 0.0 {
            quake_rs_menu_print(cbx, OPTION_VALUES_CX, 80, cstr!("Easy difficulty"));
        } else if g::skill.value == 1.0 {
            quake_rs_menu_print(cbx, OPTION_VALUES_CX, 80, cstr!("Normal difficulty"));
        } else if g::skill.value == 2.0 {
            quake_rs_menu_print(cbx, OPTION_VALUES_CX, 80, cstr!("Hard difficulty"));
        } else {
            quake_rs_menu_print(cbx, OPTION_VALUES_CX, 80, cstr!("Nightmare difficulty"));
        }

        quake_rs_menu_print(cbx, OPTION_NAMES_CX, 88, cstr!("Frag Limit"));
        if g::fraglimit.value == 0.0 {
            quake_rs_menu_print(cbx, OPTION_VALUES_CX, 88, cstr!("none"));
        } else {
            // COMPAT: ADR-004 -- `menu.c:4192` casts `fraglimit.value` to
            // `int`; `as_i` saturates rather than committing UB.
            quake_rs_menu_print(
                cbx,
                OPTION_VALUES_CX,
                88,
                g::va(cstr!("%i frags"), as_i(g::fraglimit.value)),
            );
        }

        quake_rs_menu_print(cbx, OPTION_NAMES_CX, 96, cstr!("Time Limit"));
        if g::timelimit.value == 0.0 {
            quake_rs_menu_print(cbx, OPTION_VALUES_CX, 96, cstr!("none"));
        } else {
            // COMPAT: ADR-004 -- see `fraglimit` above; `menu.c:4198`.
            quake_rs_menu_print(
                cbx,
                OPTION_VALUES_CX,
                96,
                g::va(cstr!("%i minutes"), as_i(g::timelimit.value)),
            );
        }

        quake_rs_menu_print(cbx, OPTION_NAMES_CX, 112, cstr!("Episode"));
        // MED 01/06/97 added hipnotic episodes
        if g::hipnotic {
            quake_rs_menu_print(
                cbx,
                OPTION_VALUES_CX,
                112,
                HIPNOTICEPISODES[STARTEPISODE as usize].description,
            );
        }
        // PGM 01/07/97 added rogue episodes
        else if g::rogue {
            quake_rs_menu_print(
                cbx,
                OPTION_VALUES_CX,
                112,
                ROGUEEPISODES[STARTEPISODE as usize].description,
            );
        } else {
            quake_rs_menu_print(
                cbx,
                OPTION_VALUES_CX,
                112,
                EPISODES[STARTEPISODE as usize].description,
            );
        }

        quake_rs_menu_print(cbx, OPTION_NAMES_CX, 120, cstr!("Level"));
        // MED 01/06/97 added hipnotic episodes
        if g::hipnotic {
            let idx = (HIPNOTICEPISODES[STARTEPISODE as usize].first_level + STARTLEVEL) as usize;
            quake_rs_menu_print(cbx, OPTION_VALUES_CX, 120, HIPNOTICLEVELS[idx].description);
            quake_rs_menu_print(cbx, OPTION_VALUES_CX, 128, HIPNOTICLEVELS[idx].name);
        }
        // PGM 01/07/97 added rogue episodes
        else if g::rogue {
            let idx = (ROGUEEPISODES[STARTEPISODE as usize].first_level + STARTLEVEL) as usize;
            quake_rs_menu_print(cbx, OPTION_VALUES_CX, 120, ROGUELEVELS[idx].description);
            quake_rs_menu_print(cbx, OPTION_VALUES_CX, 128, ROGUELEVELS[idx].name);
        } else {
            let idx = (EPISODES[STARTEPISODE as usize].first_level + STARTLEVEL) as usize;
            quake_rs_menu_print(cbx, OPTION_VALUES_CX, 120, LEVELS[idx].description);
            quake_rs_menu_print(cbx, OPTION_VALUES_CX, 128, LEVELS[idx].name);
        }

        // line cursor
        for i in 0..NUM_MPGAMEOPTIONS {
            quake_rs_menu_mouse_update_cursor(
                ptr::addr_of_mut!(MPGAMEOPTIONS_CURSOR),
                0,
                400,
                MPGAMEOPTIONS_CURSOR_TABLE[i as usize],
                8,
                i,
            );
        }
        g::Draw_Character(
            cbx,
            (OPTION_NAMES_CX - 8) as c_float,
            MPGAMEOPTIONS_CURSOR_TABLE[MPGAMEOPTIONS_CURSOR as usize] as c_float,
            12 + (as_i_d(g::realtime * 4.0) & 1),
        );
    }
}

/// `menu.c:4239` -- `static void M_NetStart_Change (int dir)`.
///
/// Propagates a `Raise`: `Cvar_Set` / `Cvar_SetValue` are `Host_Guard`
/// trampolines under `-Duse_rust_cvar` (`Menu_Glue_CvarSet` /
/// `Menu_Glue_CvarSetValue`).
///
/// # Safety
/// Touches glue-owned menu state and engine cvars.
unsafe fn m_netstart_change(dir: c_int) -> Raise {
    // SAFETY: single-threaded menu state; every table index is clamped below
    // exactly as the C clamps it.
    unsafe {
        let count: c_int;
        let mut f: c_float;

        match MPGAMEOPTIONS_CURSOR {
            1 => {
                MAXPLAYERS += dir;
                let limit = svs_maxclientslimit();
                if MAXPLAYERS > limit {
                    MAXPLAYERS = limit;
                }
                if MAXPLAYERS < 2 {
                    MAXPLAYERS = 2;
                }
            }

            2 => {
                raise!(g::Menu_Glue_CvarSet(
                    cstr!("coop"),
                    if g::coop.value != 0.0 {
                        cstr!("0")
                    } else {
                        cstr!("1")
                    },
                ));
            }

            3 => {
                count = if g::rogue { 6 } else { 2 };
                f = g::teamplay.value + dir as c_float;
                if f > count as c_float {
                    f = 0.0;
                } else if f < 0.0 {
                    f = count as c_float;
                }
                raise!(g::Menu_Glue_CvarSetValue(cstr!("teamplay"), f));
            }

            4 => {
                f = g::skill.value + dir as c_float;
                if f > 3.0 {
                    f = 0.0;
                } else if f < 0.0 {
                    f = 3.0;
                }
                raise!(g::Menu_Glue_CvarSetValue(cstr!("skill"), f));
            }

            5 => {
                f = g::fraglimit.value + (dir * 10) as c_float;
                if f > 100.0 {
                    f = 0.0;
                } else if f < 0.0 {
                    f = 100.0;
                }
                raise!(g::Menu_Glue_CvarSetValue(cstr!("fraglimit"), f));
            }

            6 => {
                f = g::timelimit.value + (dir * 5) as c_float;
                if f > 60.0 {
                    f = 0.0;
                } else if f < 0.0 {
                    f = 60.0;
                }
                raise!(g::Menu_Glue_CvarSetValue(cstr!("timelimit"), f));
            }

            7 => {
                STARTEPISODE += dir;
                // MED 01/06/97 added hipnotic count
                count = if g::hipnotic {
                    6
                }
                // PGM 01/07/97 added rogue count
                // PGM 03/02/97 added 1 for dmatch episode
                else if g::rogue {
                    4
                } else if g::registered.value != 0.0 {
                    7
                } else {
                    2
                };

                if STARTEPISODE < 0 {
                    STARTEPISODE = count - 1;
                }

                if STARTEPISODE >= count {
                    STARTEPISODE = 0;
                }

                STARTLEVEL = 0;
            }

            8 => {
                STARTLEVEL += dir;
                // MED 01/06/97 added hipnotic episodes
                count = if g::hipnotic {
                    HIPNOTICEPISODES[STARTEPISODE as usize].levels
                }
                // PGM 01/06/97 added hipnotic episodes
                else if g::rogue {
                    ROGUEEPISODES[STARTEPISODE as usize].levels
                } else {
                    EPISODES[STARTEPISODE as usize].levels
                };

                if STARTLEVEL < 0 {
                    STARTLEVEL = count - 1;
                }

                if STARTLEVEL >= count {
                    STARTLEVEL = 0;
                }
            }

            _ => {}
        }

        0
    }
}

/// `menu.c:4338` -- `static void M_MPGameOptions_Key (int key)`.
///
/// Propagates a `Raise` from `M_NetStart_Change` and from
/// `SCR_BeginLoadingPlaque` (`Menu_Glue_BeginLoadingPlaque`).
///
/// # Safety
/// Touches glue-owned menu state, the console buffer and engine cvars.
unsafe fn m_mpgameoptions_key(key: c_int) -> Raise {
    // SAFETY: single-threaded menu state.
    unsafe {
        match key {
            K_MOUSE2 | K_ESCAPE | K_BBUTTON => {
                m_menu_net_f();
            }

            K_UPARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                MPGAMEOPTIONS_CURSOR -= 1;
                if MPGAMEOPTIONS_CURSOR < 0 {
                    MPGAMEOPTIONS_CURSOR = NUM_MPGAMEOPTIONS - 1;
                }
            }

            K_DOWNARROW => {
                g::S_LocalSound(cstr!("misc/menu1.wav"));
                MPGAMEOPTIONS_CURSOR += 1;
                if MPGAMEOPTIONS_CURSOR >= NUM_MPGAMEOPTIONS {
                    MPGAMEOPTIONS_CURSOR = 0;
                }
            }

            K_LEFTARROW => 'left: {
                if MPGAMEOPTIONS_CURSOR == 0 {
                    break 'left;
                }
                g::S_LocalSound(cstr!("misc/menu3.wav"));
                raise!(m_netstart_change(-1));
            }

            K_RIGHTARROW => 'right: {
                if MPGAMEOPTIONS_CURSOR == 0 {
                    break 'right;
                }
                g::S_LocalSound(cstr!("misc/menu3.wav"));
                raise!(m_netstart_change(1));
            }

            K_MOUSE1 | K_ENTER | K_KP_ENTER | K_ABUTTON => {
                g::S_LocalSound(cstr!("misc/menu2.wav"));
                if MPGAMEOPTIONS_CURSOR == 0 {
                    if sv_active() {
                        g::Cbuf_AddText(cstr!("disconnect\n"));
                    }
                    g::Cbuf_AddText(cstr!("listen 0\n")); // so host_netport will be re-examined
                    g::Cbuf_AddText(g::va(cstr!("maxplayers %u\n"), MAXPLAYERS));
                    raise!(g::Menu_Glue_BeginLoadingPlaque());

                    if g::hipnotic {
                        let idx = (HIPNOTICEPISODES[STARTEPISODE as usize].first_level + STARTLEVEL)
                            as usize;
                        g::Cbuf_AddText(g::va(cstr!("map %s\n"), HIPNOTICLEVELS[idx].name));
                    } else if g::rogue {
                        let idx = (ROGUEEPISODES[STARTEPISODE as usize].first_level + STARTLEVEL)
                            as usize;
                        g::Cbuf_AddText(g::va(cstr!("map %s\n"), ROGUELEVELS[idx].name));
                    } else {
                        let idx =
                            (EPISODES[STARTEPISODE as usize].first_level + STARTLEVEL) as usize;
                        g::Cbuf_AddText(g::va(cstr!("map %s\n"), LEVELS[idx].name));
                    }

                    return 0;
                }

                raise!(m_netstart_change(1));
            }

            _ => {}
        }

        0
    }
}

/* ------------------------------------------------------------------------
 * menu.c:4404-4538 -- SEARCH MENU and SLIST MENU.
 */

/// `menu.c:4407-4409` -- the search-menu statics.
static mut SEARCH_COMPLETE: bool = false;
static mut SEARCH_COMPLETE_TIME: f64 = 0.0;
static mut SEARCH_LAST_SCOPE: c_int = g::SLIST_LAN;

/// `menu.c:4411` -- `static void M_Menu_Search_f (enum slistScope_e scope)`.
///
/// Non-raising. The tail call is `NET_Slist_f`, which stays an ordinary
/// unguarded C call by the M10e seam decision recorded in the module doc: it
/// reaches `Con_Printf`/`NET_Slist` only, never `Host_Error` or `Sys_Error`.
///
/// # Safety
/// Touches glue-owned menu state, the input subsystem and the server browser.
unsafe fn m_menu_search_f(scope: c_int) {
    // SAFETY: single-threaded menu state.
    unsafe {
        quake_rs_menu_menu_changed();
        g::IN_Deactivate(true);
        g::key_dest = KEY_MENU;
        g::m_state = M_SEARCH;
        g::slist_silent = true;
        SEARCH_LAST_SCOPE = scope;
        g::slist_scope = SEARCH_LAST_SCOPE;
        SEARCH_COMPLETE = false;
        g::NET_Slist_f();
    }
}

/// `menu.c:4423` -- `static void M_Search_Draw (cb_context_t *cbx)`.
///
/// Propagates a `Raise`: `menu.c:4436` calls `NET_Poll`, which runs
/// `pp->procedure (pp->arg)` over the pollset and so can reach `Host_Error`.
/// It goes through the existing `Host_Glue_NET_Poll` trampoline
/// (`host_glue.c:423`) rather than a second menu-owned one.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_search_draw(cbx: *mut c_void) -> Raise {
    // SAFETY: caller contract; every path name is a static literal.
    unsafe {
        let p = g::Draw_CachePic(cstr!("gfx/p_multi.lmp"));
        quake_rs_menu_draw_pic(cbx, (320 - pic_width(p)) / 2, 4, p);
        let x = (320 / 2) - ((12 * 8) / 2) + 4;
        m_draw_text_box(cbx, x - 8, 32, 12, 1);
        quake_rs_menu_print(cbx, x, 40, cstr!("Searching..."));

        if g::slistInProgress {
            raise!(g::Host_Glue_NET_Poll());
            return 0;
        }

        if !SEARCH_COMPLETE {
            SEARCH_COMPLETE = true;
            SEARCH_COMPLETE_TIME = g::realtime;
        }

        if g::hostCacheCount != 0 {
            m_menu_serverlist_f();
            return 0;
        }

        m_print_white(
            cbx,
            (320 / 2) - ((22 * 8) / 2),
            64,
            cstr!("No Quake servers found"),
        );
        if (g::realtime - SEARCH_COMPLETE_TIME) < 3.0 {
            return 0;
        }

        m_menu_lanconfig_f();

        0
    }
}

/// `menu.c:4459` -- `static void M_Search_Key (int key) {}`.
fn m_search_key(_key: c_int) {}

/// `menu.c:4464-4466` -- the server-list statics.
static mut SLIST_CURSOR: c_int = 0;
static mut SLIST_FIRST: c_int = 0;
static mut SLIST_SORTED: bool = false;
/// `menu.c:4467` -- `#define SERVER_LIST_MAX_ON_SCREEN 21`.
const SERVER_LIST_MAX_ON_SCREEN: c_int = 21;

/// `menu.c:4469` -- `static void M_Menu_ServerList_f (void)`.
///
/// # Safety
/// Touches glue-owned menu state and the input subsystem.
unsafe fn m_menu_serverlist_f() {
    // SAFETY: single-threaded menu state.
    unsafe {
        quake_rs_menu_menu_changed();
        g::IN_Deactivate(true);
        g::key_dest = KEY_MENU;
        g::m_state = M_SLIST;
        SLIST_CURSOR = 0;
        SLIST_FIRST = 0;
        g::m_return_onerror = false;
        g::m_return_reason[0] = 0;
        SLIST_SORTED = false;
    }
}

/// `menu.c:4482` -- `static void M_ServerList_Draw (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
unsafe fn m_serverlist_draw(cbx: *mut c_void) {
    // SAFETY: caller contract; the loop bound is `min (21, hostCacheCount)`
    // exactly as in the C, so `NET_SlistPrintServer` is never called past the
    // cache.
    unsafe {
        if !SLIST_SORTED {
            SLIST_SORTED = true;
            g::NET_SlistSort();
        }

        if g::hostCacheCount > SERVER_LIST_MAX_ON_SCREEN as usize {
            m_draw_scrollbar(
                cbx,
                0,
                40,
                SLIST_FIRST as c_float
                    / (g::hostCacheCount - SERVER_LIST_MAX_ON_SCREEN as usize) as c_float,
                (SERVER_LIST_MAX_ON_SCREEN - 2) as c_float,
            );
        }
        m_mouse_update_list_cursor(
            ptr::addr_of_mut!(SLIST_CURSOR),
            12,
            400,
            32,
            8,
            SERVER_LIST_MAX_ON_SCREEN,
            SLIST_FIRST,
        );

        let p = g::Draw_CachePic(cstr!("gfx/p_multi.lmp"));
        quake_rs_menu_draw_pic(cbx, (320 - pic_width(p)) / 2, 4, p);
        let mut n: usize = 0;
        while n < SERVER_LIST_MAX_ON_SCREEN as usize && n < g::hostCacheCount {
            // COMPAT: ADR-004 -- `menu.c:4498` evaluates `32 + 8 * n` in
            // `size_t` and then narrows it to `M_Print`'s `int y`; the loop
            // bound caps `n` at 20 so the value is always in range.
            quake_rs_menu_print(
                cbx,
                28,
                (32 + 8 * n) as c_int,
                g::NET_SlistPrintServer(SLIST_FIRST as usize + n),
            );
            n += 1;
        }
        g::Draw_Character(
            cbx,
            16.0,
            (32 + (SLIST_CURSOR - SLIST_FIRST) * 8) as c_float,
            12 + (as_i_d(g::realtime * 4.0) & 1),
        );

        if g::m_return_reason[0] != 0 {
            m_print_white(
                cbx,
                16,
                148,
                ptr::addr_of!(g::m_return_reason).cast::<c_char>(),
            );
        }
    }
}

/// `menu.c:4507` -- `static void M_ServerList_Key (int k)`.
///
/// # Safety
/// Touches glue-owned menu state, the console buffer and the input subsystem.
unsafe fn m_serverlist_key(k: c_int) {
    // SAFETY: single-threaded menu state.
    unsafe {
        // COMPAT: ADR-004 -- `menu.c:4509` passes the `size_t hostCacheCount`
        // to a `const int` parameter; the conversion is implementation-defined
        // rather than undefined, and the cache never exceeds `INT_MAX`, so the
        // truncating cast reproduces it exactly.
        if quake_rs_menu_handle_scroll_bar_keys(
            k,
            ptr::addr_of_mut!(SLIST_CURSOR),
            ptr::addr_of_mut!(SLIST_FIRST),
            g::hostCacheCount as c_int,
            SERVER_LIST_MAX_ON_SCREEN,
        ) {
            return;
        }

        match k {
            K_MOUSE2 | K_ESCAPE | K_BBUTTON => {
                m_menu_lanconfig_f();
            }

            K_SPACE => {
                m_menu_search_f(SEARCH_LAST_SCOPE);
            }

            K_MOUSE1 | K_ENTER | K_KP_ENTER | K_ABUTTON => {
                g::S_LocalSound(cstr!("misc/menu2.wav"));
                g::m_return_state = g::m_state;
                g::m_return_onerror = true;
                SLIST_SORTED = false;
                g::IN_Activate();
                g::key_dest = KEY_GAME;
                g::m_state = M_NONE;
                g::Cbuf_AddText(g::va(
                    cstr!("connect \"%s\"\n"),
                    g::NET_SlistPrintServerName(SLIST_CURSOR as usize),
                ));
            }

            _ => {}
        }
    }
}

/* ------------------------------------------------------------------------
 * menu.c:4543-4586 -- custom-gfx checks (from Ironwail).
 */

/// `menu.c:4546` -- `static qboolean M_CheckCustomGfx (const char *custompath,
/// const char *basepath, int knownlength, const unsigned int *hashes, int
/// numhashes)`.
///
/// Non-raising. `COM_FileExists` / `COM_OpenFile` / `Sys_FileRead` /
/// `COM_CloseFile` only touch the pak layer, and the `Mem_Alloc` failure path
/// ends in `Sys_Error` -> `exit (1)`, not a longjmp (see the module doc).
///
/// # Safety
/// `custompath` and `basepath` must be NUL-terminated C strings; `hashes`
/// must point at `numhashes` readable `unsigned int`s.
unsafe fn m_check_custom_gfx(
    custompath: *const c_char,
    basepath: *const c_char,
    knownlength: c_int,
    hashes: *const c_uint,
    numhashes: c_int,
) -> bool {
    // SAFETY: caller contract; `h` is only closed on the path where
    // `COM_OpenFile` reported success, exactly as in the C.
    unsafe {
        let mut id_custom: c_uint = 0;
        let mut id_base: c_uint = 0;
        let mut h: c_int = 0;
        let mut ret = false;

        if !g::COM_FileExists(custompath, ptr::addr_of_mut!(id_custom)) {
            return false;
        }

        let length = g::COM_OpenFile(basepath, ptr::addr_of_mut!(h), ptr::addr_of_mut!(id_base));
        if length == -1 {
            return false;
        }

        if id_custom >= id_base {
            ret = true;
        } else if length == knownlength as c::qfilesize_t {
            // `length` equals `knownlength` here, so both narrowings below
            // are exact; the C relies on the same guard.
            let data = g::Mem_Alloc(length as usize);
            if length == g::Sys_FileRead(h, data, length as c_int) as c::qfilesize_t {
                let hash = g::COM_HashBlock(data, length as usize);
                let mut n = numhashes;
                let mut p = hashes;
                loop {
                    let more = n > 0;
                    n -= 1;
                    if !(more && !ret) {
                        break;
                    }
                    let want = *p;
                    p = p.add(1);
                    if hash == want {
                        ret = true;
                    }
                }
            }
            g::Mem_Free(data);
        }

        g::COM_CloseFile(h);

        ret
    }
}

/// `menu.c:4579` -- `void M_CheckMods (void)`.
///
/// Non-raising; the pre-existing `Host_Glue_M_CheckMods` (`host_glue.c:464`)
/// keeps guarding the plain name and simply never fires.
///
/// # Safety
/// Runs on the engine's single thread after the filesystem is up.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_check_mods() {
    // SAFETY: single-threaded engine init / gamedir-change path.
    unsafe {
        const SP_HASHES: [c_uint; 1] = [0x86a6_f086];
        const SGL_HASHES: [c_uint; 1] = [0x7bba_813d];

        M_SINGLEPLAYER_SHOWLEVELS = m_check_custom_gfx(
            cstr!("gfx/sp_maps.lmp"),
            cstr!("gfx/sp_menu.lmp"),
            14856,
            SP_HASHES.as_ptr(),
            SP_HASHES.len() as c_int,
        );
        M_SKILL_USEGFX = m_check_custom_gfx(
            cstr!("gfx/skillmenu.lmp"),
            cstr!("gfx/sp_menu.lmp"),
            14856,
            SP_HASHES.as_ptr(),
            SP_HASHES.len() as c_int,
        );
        M_SKILL_USECUSTOMTITLE = m_check_custom_gfx(
            cstr!("gfx/p_skill.lmp"),
            cstr!("gfx/ttl_sgl.lmp"),
            6728,
            SGL_HASHES.as_ptr(),
            SGL_HASHES.len() as c_int,
        );
    }
}

/* ------------------------------------------------------------------------
 * menu.c:4592-4950 -- menu subsystem.
 *
 * `menu.c:4592` `static void M_Menu_Credits_f (void) {}` and `menu.c:4597`
 * `void M_Init (void)` stay in C: `M_Init` is nothing but a list of
 * `Cmd_AddCommand` registrations, every one of which has to name a C
 * function pointer, and it is already entered through the pre-existing
 * `Host_Glue_M_Init` guard (`host_glue.c:462`).
 */

/// `menu.c:4615` -- `void M_NewGame (void)`.
///
/// # Safety
/// Touches glue-owned menu state.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_new_game() {
    // SAFETY: single-threaded menu state.
    unsafe {
        M_MAIN_CURSOR = 0;
        // the map list is about to be rebuilt
        if g::m_state == M_MAPS || g::m_state == M_SKILL {
            g::m_state = M_MAIN;
        }
    }
}

/// `menu.c:4623` -- `void M_UpdateMouse ()`.
///
/// Propagates a `Raise`: the drag latches re-enter `M_Keydown (K_MOUSE1)` and
/// the three `*_AdjustSliders` helpers, all of which write cvars.
///
/// `SDL_GetMouseState` is reached through `Menu_Glue_GetMouseState` so no Rust
/// translation unit names an SDL symbol; the glue also absorbs the SDL2 `int
/// *` / SDL3 `float *` split, always handing over the SDL3 `float` form.
///
/// # Safety
/// Runs on the engine's single thread from `Host_Frame`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_update_mouse() -> Raise {
    // SAFETY: single-threaded menu state; both out-params are live locals.
    unsafe {
        let mut new_mouse_x: c_float = 0.0;
        let mut new_mouse_y: c_float = 0.0;
        g::Menu_Glue_GetMouseState(
            ptr::addr_of_mut!(new_mouse_x),
            ptr::addr_of_mut!(new_mouse_y),
        );

        // COMPAT: ADR-004 -- `menu.c:4635-4637` and `:4641-4642` convert the
        // SDL float cursor position to `int m_mouse_x_pixels` / `m_mouse_x`
        // implicitly. The value is a window coordinate, so it is always in
        // range; `as_i` saturates where C would be undefined.
        M_MOUSE_MOVED = !MENU_CHANGED
            && ((M_MOUSE_X_PIXELS != as_i(new_mouse_x)) || (M_MOUSE_Y_PIXELS != as_i(new_mouse_y)));
        M_MOUSE_X_PIXELS = as_i(new_mouse_x);
        M_MOUSE_Y_PIXELS = as_i(new_mouse_y);
        MENU_CHANGED = false;

        M_MOUSE_X = as_i(new_mouse_x);
        M_MOUSE_Y = as_i(new_mouse_y);
        m_pixel_to_menu_canvas_coord(ptr::addr_of_mut!(M_MOUSE_X), ptr::addr_of_mut!(M_MOUSE_Y));

        if SCROLLBAR_GRAB {
            if g::keydown[K_MOUSE1 as usize] && m_in_scrollbar() {
                raise!(quake_rs_menu_keydown(K_MOUSE1));
            } else {
                SCROLLBAR_GRAB = false;
            }
        } else if SLIDER_GRAB {
            let graphic_option_has_sliders = ((GRAPHICS_OPTIONS_CURSOR >= GRAPHICS_OPT_GAMMA)
                && (GRAPHICS_OPTIONS_CURSOR <= GRAPHICS_OPT_FOV))
                || (GRAPHICS_OPTIONS_CURSOR == GRAPHICS_OPT_MAX_FPS);

            if g::keydown[K_MOUSE1 as usize]
                && (g::m_state == M_GAME)
                && (GAME_OPTIONS_CURSOR >= GAME_OPT_SCALE)
                && (GAME_OPTIONS_CURSOR <= GAME_OPT_VIEWROLL)
            {
                raise!(m_gameoptions_adjust_sliders(0, true));
            } else if g::keydown[K_MOUSE1 as usize]
                && (g::m_state == M_GRAPHICS)
                && graphic_option_has_sliders
            {
                raise!(m_graphicsoptions_adjust_sliders(0, true));
            } else if g::keydown[K_MOUSE1 as usize]
                && (g::m_state == M_SOUND)
                // `menu.c:4658` gates the *sound* sliders on
                // `graphics_options_cursor`, not on a sound-menu cursor; the
                // two menus share one cursor variable in the C.
                && (GRAPHICS_OPTIONS_CURSOR >= SOUND_OPT_SNDVOL)
                && (GRAPHICS_OPTIONS_CURSOR <= SOUND_OPT_MUSICVOL)
            {
                raise!(m_soundoptions_adjust_sliders(0, true));
            } else {
                SLIDER_GRAB = false;
            }
        }

        SCROLLBAR_SIZE = 0;

        0
    }
}

/// `menu.c:4666` -- `void M_Draw (cb_context_t *cbx)`.
///
/// Propagates a `Raise`: `M_Quit_Draw` and `M_Search_Draw` are raising, and
/// `m_video` dispatches into `Quake/gl_vidsdl.c` through
/// `Menu_Glue_VideoDraw`.
///
/// # Safety
/// `cbx` must be a live `cb_context_t *`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_draw(cbx: *mut c_void) -> Raise {
    // SAFETY: caller contract; single-threaded menu state.
    unsafe {
        if g::m_state == M_NONE || g::key_dest != KEY_MENU {
            return 0;
        }

        if !M_RECURSIVE_DRAW {
            if g::scr_con_current != 0.0 {
                g::Draw_ConsoleBackground(cbx);
                g::S_ExtraUpdate();
            }

            g::Draw_FadeScreen(cbx); // johnfitz -- fade even if console fills screen
        } else {
            M_RECURSIVE_DRAW = false;
        }

        g::GL_SetCanvas(cbx, g::CANVAS_MENU); // johnfitz

        match g::m_state {
            M_NONE => {}
            M_MAIN => m_main_draw(cbx),
            M_SINGLEPLAYER => m_singleplayer_draw(cbx),
            M_LOAD => m_load_draw(cbx),
            M_SAVE => m_save_draw(cbx),
            M_MULTIPLAYER => m_multiplayer_draw(cbx),
            M_SETUP => m_setup_draw(cbx),
            M_NET => m_net_draw(cbx),
            M_OPTIONS => m_options_draw(cbx),
            M_GAME => m_gameoptions_draw(cbx),
            M_KEYS => m_keys_draw(cbx),
            M_VIDEO => raise!(g::Menu_Glue_VideoDraw(cbx)),
            M_GRAPHICS => m_graphicsoptions_draw(cbx),
            M_SOUND => m_soundoptions_draw(cbx),
            M_HELP => m_help_draw(cbx),
            M_MODS => m_mods_draw(cbx),
            M_MAPS => m_maps_draw(cbx),
            M_SKILL => m_skill_draw(cbx),
            M_QUIT => {
                if g::cl_confirmquit.value == 0.0 {
                    /* QuakeSpasm customization: */
                    /* Quit now! S.A. */
                    g::m_is_quitting = true;
                    g::key_dest = KEY_CONSOLE;
                    g::Cbuf_InsertText(cstr!("quit"));
                } else {
                    raise!(m_quit_draw(cbx));
                }
            }
            M_LANCONFIG => m_lan_config_draw(cbx),
            M_MPGAMEOPTIONS => m_mpgameoptions_draw(cbx),
            M_SEARCH => raise!(m_search_draw(cbx)),
            M_SLIST => m_serverlist_draw(cbx),
            _ => {}
        }

        if g::m_entersound {
            g::S_LocalSound(cstr!("misc/menu2.wav"));
            g::m_entersound = false;
        }

        g::S_ExtraUpdate();

        0
    }
}

/// `menu.c:4798` -- `void M_Keydown (int key)`.
///
/// Propagates a `Raise`; `keys_glue.c:273` already guards the plain name.
///
/// # Safety
/// Runs on the engine's single thread from the key handler.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_keydown(key: c_int) -> Raise {
    // SAFETY: single-threaded menu state.
    unsafe {
        match g::m_state {
            M_NONE => {}
            M_MAIN => raise!(m_main_key(key)),
            M_SINGLEPLAYER => raise!(m_singleplayer_key(key)),
            M_LOAD => raise!(m_load_key(key)),
            M_SAVE => m_save_key(key),
            M_MULTIPLAYER => m_multiplayer_key(key),
            M_SETUP => raise!(m_setup_key(key)),
            M_NET => m_net_key(key),
            M_OPTIONS => raise!(m_options_key(key)),
            M_GAME => raise!(m_gameoptions_key(key)),
            M_GRAPHICS => raise!(m_graphicsoptions_key(key)),
            M_MODS => m_mods_key(key),
            M_MAPS => m_maps_key(key),
            M_SKILL => m_skill_key(key),
            M_KEYS => m_keys_key(key),
            M_VIDEO => raise!(g::Menu_Glue_VideoKey(key)),
            M_SOUND => raise!(m_soundoptions_key(key)),
            M_HELP => m_help_key(key),
            M_QUIT => m_quit_key(key),
            M_LANCONFIG => m_lan_config_key(key),
            M_MPGAMEOPTIONS => raise!(m_mpgameoptions_key(key)),
            M_SEARCH => m_search_key(key),
            M_SLIST => m_serverlist_key(key),
            _ => {}
        }

        0
    }
}

/// `menu.c:4886` -- `void M_Charinput (int key)`.
///
/// Non-raising: all four character handlers only edit text buffers. The
/// pre-existing `keys_glue.c` guard on the plain name stays and never fires.
///
/// # Safety
/// Runs on the engine's single thread from the text-input handler.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_charinput(key: c_int) {
    // SAFETY: single-threaded menu state.
    unsafe {
        match g::m_state {
            M_SETUP => m_setup_char(key),
            M_MAPS => m_maps_char(key),
            M_QUIT => m_quit_char(key),
            M_LANCONFIG => m_lan_config_char(key),
            _ => {}
        }
    }
}

/// `menu.c:4906` -- `qboolean M_TextEntry (void)`.
///
/// # Safety
/// Runs on the engine's single thread.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_text_entry() -> bool {
    // SAFETY: single-threaded menu state.
    unsafe {
        match g::m_state {
            M_SETUP => m_setup_text_entry(),
            M_MAPS => m_maps_text_entry(),
            M_QUIT => m_quit_text_entry(),
            M_LANCONFIG => m_lan_config_text_entry(),
            _ => false,
        }
    }
}

/// `menu.c:4938` -- `qboolean M_WaitingForKeyBinding (void)`.
///
/// # Safety
/// Runs on the engine's single thread.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_waiting_for_key_binding() -> bool {
    // SAFETY: single-threaded menu state.
    unsafe { g::key_dest == KEY_MENU && g::m_state == M_KEYS && BIND_GRAB }
}

/// `menu.c:4943` -- `void M_ConfigureNetSubsystem (void)`. The name has
/// external linkage in the C only by accident (`menu.c:111` declares it
/// without `static`); no other translation unit calls it, so it stays
/// private here.
///
/// # Safety
/// Touches the command buffer and the net port.
unsafe fn m_configure_net_subsystem() {
    // SAFETY: single-threaded menu state.
    unsafe {
        // enable/disable net systems to match desired config
        g::Cbuf_AddText(cstr!("stopdemo\n"));

        if tcpip_config() {
            g::net_hostport = LAN_CONFIG_PORT;
        }
    }
}

/* ------------------------------------------------------------------------
 * `M_Init` command entry points.
 *
 * `menu.c:4599-4613` registers these with `Cmd_AddCommand`. They are
 * `static` in the C, but `Quake/menu_glue.c` has to take their address, so
 * each gets a thin `quake_rs_menu_*` export over the private core. The rest
 * of the registration list is already exported above
 * (`quake_rs_menu_toggle_menu_f`, `quake_rs_menu_menu_main_f`,
 * `quake_rs_menu_menu_options_f`, `quake_rs_menu_menu_quit_f`), and
 * `menu_video` / `menu_credits` are registered against C functions.
 */

/// `menu.c:733` -- `M_Menu_SinglePlayer_f`.
///
/// # Safety
/// Runs on the engine's single thread as a console command.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_menu_singleplayer_f() {
    // SAFETY: single-threaded menu state.
    unsafe { m_menu_singleplayer_f() }
}

/// `menu.c:871` -- `M_Menu_Load_f`.
///
/// # Safety
/// Runs on the engine's single thread as a console command.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_menu_load_f() {
    // SAFETY: single-threaded menu state.
    unsafe { m_menu_load_f() }
}

/// `menu.c:881` -- `M_Menu_Save_f`.
///
/// # Safety
/// Runs on the engine's single thread as a console command.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_menu_save_f() {
    // SAFETY: single-threaded menu state.
    unsafe { m_menu_save_f() }
}

/// `menu.c` -- `M_Menu_Maps_Cmd_f`.
///
/// # Safety
/// Runs on the engine's single thread as a console command.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_menu_maps_cmd_f() {
    // SAFETY: single-threaded menu state.
    unsafe { m_menu_maps_cmd_f() }
}

/// `menu.c:1020` -- `M_Menu_MultiPlayer_f`.
///
/// # Safety
/// Runs on the engine's single thread as a console command.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_menu_multiplayer_f() {
    // SAFETY: single-threaded menu state.
    unsafe { m_menu_multiplayer_f() }
}

/// `menu.c:1109` -- `M_Menu_Setup_f`.
///
/// # Safety
/// Runs on the engine's single thread as a console command.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_menu_setup_f() {
    // SAFETY: single-threaded menu state.
    unsafe { m_menu_setup_f() }
}

/// `menu.c` -- `M_Menu_Keys_f`.
///
/// # Safety
/// Runs on the engine's single thread as a console command.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_menu_keys_f() {
    // SAFETY: single-threaded menu state.
    unsafe { m_menu_keys_f() }
}

/// `menu.c` -- `M_Menu_Help_f`.
///
/// # Safety
/// Runs on the engine's single thread as a console command.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_menu_menu_help_f() {
    // SAFETY: single-threaded menu state.
    unsafe { m_menu_help_f() }
}
