//! `Quake/console.c` (Rust migration Phase 7 M10c, Pattern A whole-file swap).
//!
//! `Quake/console_glue.c` is the C frame this module sits in. It owns every
//! C-visible object `console.c` used to define, the eight C-variadic entry
//! points (so libc still does the formatting -- ADR-005's Rust formatter is
//! deliberately *not* used for console output), `Con_Printf`'s screen-update
//! tail, `Con_NotifyBox`, and the ADR-009 trampolines. `Quake/console.c`
//! survives as the differential oracle (`quake-ctest/stubs/console_ref.c`).
//!
//! ## ADR-009 raise-topology audit
//!
//! Three callees below can longjmp out through `Host_Error`, and each is
//! reached only through a `Host_Guard` trampoline in the glue:
//!
//! * `console.c:763` `M_Menu_Main_f ()` -> `Console_Glue_MenuMain`
//! * `console.c:1858` `cvar->completion (cvar, partial)` ->
//!   `Console_Glue_CvarCompletion`
//! * `console.c:1866` `cmd->completion (partial)` ->
//!   `Console_Glue_CmdCompletion`
//!
//! That makes exactly two entry points raise-capable. Both are status cores
//! named `quake_rs_*`, wrapped by plain-named `Host_Reraise` functions in the
//! glue: `Con_ToggleConsole_f` (`console_glue.c:213`) and `Con_TabComplete`
//! (`console_glue.c:220`). `Con_TabComplete`'s tail-recursive self-call
//! (`console.c:2097`) goes to the *core*, not the wrapper, so no jump ever
//! crosses a Rust frame.
//!
//! Four more entry points are exported under `quake_rs_*` names even though
//! they cannot raise -- `quake_rs_con_scroll`, `quake_rs_con_select_all`,
//! `quake_rs_con_force_mouse_move` and
//! `quake_rs_con_copy_selection_to_clipboard`. `keys.c` calls all four (plus
//! the two raise-capable ones), and the ctest oracle has to keep counting
//! those calls from `stubs/keys_ref.c`, so the plain names stay with that
//! TU's link doubles and `console_glue.c` supplies the engine's plain
//! wrappers instead.
//!
//! Everything else the port calls was audited non-raising:
//! `S_LocalSound` (`snd_dma.c:1135`), `Sys_Explore`, `SCR_EndLoadingPlaque`
//! (`gl_screen.c:993`), `IN_*`, `VID_SetMouseCursor`, the `Draw_*`/`GL_*`
//! renderer entry points, `Cvar_RegisterVariable`, `Cmd_AddCommand2`,
//! `Cmd_TokenizeString`, `Vec_*`, `Mem_*` and `Sys_fopen`.
//!
//! `Con_Printf` / `Con_SafePrintf` are called from Rust here exactly as
//! hundreds of already-ported call sites call them: the M3 doctrine
//! (`quake-ctest/stubs/stubs.c:5052`) treats the plain `Con_Printf` as
//! non-raising, and decision 2 of the M10c contract keeps its
//! `SCR_UpdateScreen` tail in the glue so no Rust frame ever names it.
//!
//! ## Ownership (ADR-007)
//!
//! The scrollback and its geometry, the six cvars, the notify ring, the rcon
//! redirect buffer and `con_mutex` stay defined in `console_glue.c` and are
//! reached through [`quake_c_sys::console`]. Only the objects nothing outside
//! `console.c` ever named move here: the file statics, plus `tablist` and
//! `tab_t`, which had external linkage but no external reader.

use core::ffi::{c_char, c_int, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::console as g;
use quake_c_sys::keys as k;
use quake_types::host::{ClientState, ClientStatic, FileListItem, CA_CONNECTED};
use quake_util::qctype::{q_isspace, q_toupper};

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
 * Constants transcribed from the headers console.c includes.
 */

/// `console.c:42` -- `#define CON_TEXTSIZE (1024 * 1024)`.
const CON_TEXTSIZE: c_int = 1024 * 1024;
/// `console.c:43` -- `#define CON_MINSIZE 16384`.
const CON_MINSIZE: c_int = 16384;
/// `console.c:67` -- `#define NUM_CON_TIMES 4`.
const NUM_CON_TIMES: c_int = 4;
/// `console.c:77` -- `#define CON_MARGIN 1`.
const CON_MARGIN: c_int = 1;
/// `draw.h:26` -- `#define CHARACTER_SIZE 8`.
const CHARACTER_SIZE: c_int = 8;
/// `console.c:78` -- `#define CON_SCROLL_ZONE (CHARACTER_SIZE * 2)`.
const CON_SCROLL_ZONE: c_int = CHARACTER_SIZE * 2;
/// `console.c:79` -- `#define CON_MAX_SCROLL_SPEED 32.f`.
const CON_MAX_SCROLL_SPEED: f32 = 32.0;
/// `console.c:123` -- `static const double DOUBLECLICK_TIME = 0.5;`.
const DOUBLECLICK_TIME: f64 = 0.5;
/// `console.c:1245` -- `#define MAXPRINTMSG 4096`.
const MAXPRINTMSG: usize = 4096;
/// `keys.h:134`.
const MAXCMDLINE: usize = g::MAXCMDLINE;

/// `console.c:107-115` -- `contest_t`.
const CT_INSIDE: c_int = 0;
const CT_NEAREST: c_int = 1;

/// `console.c:117-122` -- `conmouse_t`.
const CMS_NOTPRESSED: c_int = 0;
const CMS_PRESSED: c_int = 1;
const CMS_DRAGGING: c_int = 2;

/// `console.h:44-48` -- `tabcomplete_t`.
const TABCOMPLETE_AUTOHINT: c_int = 0;
const TABCOMPLETE_USER: c_int = 1;

/// `quakedef.h:237-249` -- `CANVAS_CONSOLE` is the third `canvastype`.
const CANVAS_CONSOLE: c_int = 2;

/// `vid.h:93-98` -- `mousecursor_t`.
const MOUSECURSOR_DEFAULT: c_int = 0;
const MOUSECURSOR_HAND: c_int = 1;
const MOUSECURSOR_IBEAM: c_int = 2;

/// `protocol.h:240` -- `#define GAME_DEATHMATCH 1`.
const GAME_DEATHMATCH: c_int = 1;

/// `keys.h:136-142` -- `keydest_t`.
const KEY_GAME: c_int = 0;
const KEY_CONSOLE: c_int = 1;
const KEY_MESSAGE: c_int = 2;

/// `keys.h:48`, `:95`, `:132`.
const K_SHIFT: c_int = 134;
const K_MOUSE1: c_int = 200;
const MAX_KEYS: c_int = 256;

/* ------------------------------------------------------------------------
 * Types console.c keeps to itself.
 */

/// `console.c:87-91` -- `conofs_t`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct ConOfs {
    line: c_int,
    col: c_int,
}

/// `console.c:93-98` -- `conlink_t`. The path bytes are allocated in the
/// same block, immediately after the struct.
#[repr(C)]
struct ConLink {
    path: *const c_char,
    begin: ConOfs,
    end: ConOfs,
}

/// `console.c:100-104` -- `conselection_t`.
#[repr(C)]
#[derive(Clone, Copy)]
struct ConSelection {
    begin: ConOfs,
    end: ConOfs,
}

const ZERO_OFS: ConOfs = ConOfs { line: 0, col: 0 };
const ZERO_SEL: ConSelection = ConSelection {
    begin: ZERO_OFS,
    end: ZERO_OFS,
};

/* ------------------------------------------------------------------------
 * The file statics (console.c:81-140, :1211-1213, :1598-1600, :718, :1123,
 * :1994).
 */

static mut CON_SCROLLSPEED: f32 = 0.0;
static mut CON_SCROLLDELTA: f32 = 0.0;
static mut CON_LINKS: *mut *mut ConLink = ptr::null_mut();
static mut CON_HOTLINK: *mut ConLink = ptr::null_mut();
static mut CON_MOUSECLICKDELAY: f64 = 0.0;
static mut CON_MOUSECLICKS: c_int = 0;
static mut CON_MOUSESTATE: c_int = CMS_NOTPRESSED;
static mut CON_MOUSESELECTION: ConSelection = ZERO_SEL;
static mut CON_SELECTION: ConSelection = ZERO_SEL;
static mut CON_CLICKX: c_int = 0;
static mut CON_CLICKY: c_int = 0;

/// `console.c:1211` -- `static char logfilename[MAX_OSPATH]`.
static mut LOGFILENAME: [c_char; c::MAX_OSPATH] = [0; c::MAX_OSPATH];
/// `console.c:1212` -- `static FILE *log_file`.
static mut LOG_FILE: *mut c::FILE = ptr::null_mut();

/// `console.c:1599` -- `static char bash_partial[80]`.
static mut BASH_PARTIAL: [c_char; 80] = [0; 80];
/// `console.c:1600` -- `static qboolean bash_singlematch`.
static mut BASH_SINGLEMATCH: bool = false;

/// `Con_Print`'s `static int cr` (`console.c:1123`).
static mut PRINT_CR: c_int = 0;
/// `Con_TabComplete`'s `static char *c` (`console.c:1994`).
static mut TAB_C: *mut c_char = ptr::null_mut();
/// `Con_Quakebar`'s `static char bar[42]` (`console.c:716`).
static mut QUAKEBAR: [c_char; 42] = [0; 42];

/* ------------------------------------------------------------------------
 * Small helpers.
 */

/// Integer division that yields 0 instead of trapping when the divisor is
/// zero. C divides straight through at `console.c:173`, `:190`, `:968` and
/// `:2300`; `glheight` and `con_linewidth` are non-zero in every state the
/// engine can actually reach, and where they are not, C is undefined
/// (SIGFPE in practice) while Rust would panic -- which `panic = "abort"`
/// turns into a hard abort inside an FFI frame. Returning 0 keeps the
/// process alive on a path that was already broken.
#[inline]
fn idiv(a: c_int, b: c_int) -> c_int {
    if b == 0 {
        0
    } else {
        a.wrapping_div(b)
    }
}

/// The `%` counterpart of [`idiv`]; see that function for the rationale.
#[inline]
fn imod(a: c_int, b: c_int) -> c_int {
    if b == 0 {
        0
    } else {
        a.wrapping_rem(b)
    }
}

/// `q_minmax.h:49` -- `clamp_i`.
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

/// `q_minmax.h:41` -- `q_min_f`. `f32::min` differs from the C macro on NaN.
#[inline]
fn min_f(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}

/// `q_minmax.h:45` -- `q_max_f`.
#[inline]
fn max_f(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

/// # Safety
/// `s` must point at a NUL-terminated string.
unsafe fn strlen(s: *const c_char) -> usize {
    // SAFETY: the caller guarantees the pointer contract documented above.
    unsafe {
        let mut n = 0usize;
        while *s.add(n) != 0 {
            n += 1;
        }
        n
    }
}

/// # Safety
/// Both arguments must point at NUL-terminated strings.
unsafe fn streq(a: *const c_char, b: *const c_char) -> bool {
    // SAFETY: the caller guarantees the pointer contract documented above.
    unsafe {
        let mut i = 0usize;
        loop {
            let (ca, cb) = (*a.add(i), *b.add(i));
            if ca != cb {
                return false;
            }
            if ca == 0 {
                return true;
            }
            i += 1;
        }
    }
}

/// `common.h:129` -- `VEC_SIZE`.
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

/// `common.h:126` -- `VEC_PUSH` (`Vec_Grow` + a direct write, *not*
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

/// `keys.c:31` -- `key_lines[i]`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
#[inline]
unsafe fn key_line(i: c_int) -> *mut c_char {
    // SAFETY: the caller guarantees the pointer contract documented above.
    unsafe {
        (&raw mut k::key_lines)
            .cast::<c_char>()
            .add(i as usize * MAXCMDLINE)
    }
}

#[inline]
unsafe fn keydown(i: c_int) -> bool {
    // SAFETY: the caller guarantees the pointer contract documented above.
    unsafe { *(&raw const k::keydown).cast::<bool>().add(i as usize) }
}

#[inline]
fn add_command(name: *const c_char, func: unsafe extern "C" fn()) {
    // SAFETY: `name` is a static NUL-terminated literal and `func` has the
    // xcommand_t signature.
    unsafe {
        c::Cmd_AddCommand2(name, Some(func), c::cmd_source_t_src_command, false);
    }
}

/* ------------------------------------------------------------------------
 * Scrollback geometry.
 */

/// `console.c:143`.
///
/// COMPAT (`console.c:145`): `line % con_totallines` keeps C's truncating
/// remainder, so a negative `line` yields a pointer *before* `con_text`.
/// `Con_DrawConsole` and `Con_CopySelectionToClipboard` both rely on their
/// callers having clamped `line` to `>= 0` first.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn con_get_line(line: c_int) -> *const c_char {
    // SAFETY: mirrors console.c:145; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        g::con_text.offset(imod(line, g::con_totallines).wrapping_mul(g::con_linewidth) as isize)
    }
}

/// `console.c:153`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn con_str_len(line: c_int) -> usize {
    // SAFETY: mirrors console.c:153; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        if line > g::con_current {
            return 0;
        }
        let text = con_get_line(line);
        let mut len = g::con_linewidth as usize;
        while len > 0 && ((*text.add(len - 1) as c_int) & 0x7f) == b' ' as c_int {
            len -= 1;
        }
        len
    }
}

/// `console.c:173`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn con_screen_to_canvas(x: c_int, y: c_int, outx: *mut c_int, outy: *mut c_int) {
    // SAFETY: mirrors console.c:173; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let conw = c::cl_parse::vid.conwidth;
        let conh = c::cl_parse::vid.conheight;
        let lines = conh
            - idiv(
                (g::scr_con_current as c_int).wrapping_mul(conh),
                g::glheight,
            );

        *outx = idiv(x.wrapping_mul(conw), g::glwidth);
        *outy = lines + idiv(y.wrapping_mul(conh), g::glheight);
    }
}

/// `console.c:190`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn con_canvas_to_offset(x: c_int, y: c_int, ofs: *mut ConOfs, testmode: c_int) -> bool {
    // SAFETY: mirrors console.c:190; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let mut ret = true;
        let mut x = x;
        let mut y = c::cl_parse::vid.conheight - y;

        if testmode == CT_NEAREST {
            x += 4;
        }

        x >>= 3;
        y >>= 3;

        x -= CON_MARGIN;
        y -= 2;

        if testmode == CT_INSIDE {
            if x < 0 || x >= g::con_linewidth {
                ret = false;
            }
            if y < 0 || y >= g::con_vislines {
                ret = false;
            }
            if g::con_backscroll != 0 && y < 2 {
                ret = false;
            }
        } else {
            x = clamp_i(0, x, g::con_linewidth);
            y = clamp_i(-1, y, g::con_vislines);
            if y < 0 {
                x = 0;
            }
            if g::con_backscroll != 0 && y < 2 {
                x = 0;
                y = 1;
            }
        }

        y += g::con_backscroll;
        y = g::con_current - y;

        (*ofs).line = y;
        (*ofs).col = x;

        ret
    }
}

/// `console.c:256`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn con_screen_to_offset(x: c_int, y: c_int, ofs: *mut ConOfs, testmode: c_int) -> bool {
    // SAFETY: mirrors console.c:256; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let (mut cx, mut cy) = (0, 0);
        con_screen_to_canvas(x, y, &mut cx, &mut cy);
        con_canvas_to_offset(cx, cy, ofs, testmode)
    }
}

/// `console.c:269`.
#[inline]
fn con_ofs_compare(lhs: ConOfs, rhs: ConOfs) -> c_int {
    if lhs.line != rhs.line {
        return lhs.line.wrapping_sub(rhs.line);
    }
    lhs.col.wrapping_sub(rhs.col)
}

/// `console.c:283`.
#[inline]
fn con_ofs_in_range(ofs: ConOfs, begin: ConOfs, end: ConOfs) -> bool {
    con_ofs_compare(ofs, begin) >= 0 && con_ofs_compare(ofs, end) < 0
}

/// `console.c:293`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn con_get_current_range(begin: &mut ConOfs, end: &mut ConOfs) {
    // SAFETY: mirrors console.c:293; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        begin.line = g::con_current - g::con_totallines + 1;
        begin.col = 0;
        end.line = g::con_current + 1;
        end.col = 0;
    }
}

/// `console.c:306`.
fn con_intersect_ranges(
    begin: &mut ConOfs,
    end: &mut ConOfs,
    selbegin: ConOfs,
    selend: ConOfs,
) -> bool {
    if con_ofs_compare(selend, *begin) <= 0 {
        return false;
    }
    if con_ofs_compare(*end, selbegin) <= 0 {
        return false;
    }

    if con_ofs_compare(*begin, selbegin) < 0 {
        *begin = selbegin;
    }
    if con_ofs_compare(selend, *end) < 0 {
        *end = selend;
    }

    true
}

/// `console.c:328`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn con_get_link_at_ofs(ofs: ConOfs) -> *mut ConLink {
    // SAFETY: mirrors console.c:328; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let mut lo = 0usize;
        let mut hi = vec_size(CON_LINKS);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if con_ofs_compare(ofs, (**CON_LINKS.add(mid)).end) >= 0 {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        if lo == vec_size(CON_LINKS) {
            return ptr::null_mut();
        }

        if con_ofs_compare(ofs, (**CON_LINKS.add(lo)).begin) >= 0 {
            return *CON_LINKS.add(lo);
        }

        ptr::null_mut()
    }
}

/// `console.c:360`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn con_get_link_at_pixel(x: c_int, y: c_int) -> *mut ConLink {
    // SAFETY: mirrors console.c:360; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let mut ofs = ZERO_OFS;
        if !con_screen_to_offset(x, y, &mut ofs, CT_INSIDE) {
            return ptr::null_mut();
        }
        con_get_link_at_ofs(ofs)
    }
}

/// `console.c:373`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
#[inline]
unsafe fn con_set_hot_link(link: *mut ConLink) {
    // SAFETY: mirrors console.c:373; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        CON_HOTLINK = link;
    }
}

/// `console.c:383`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
#[inline]
unsafe fn con_clear_selection() {
    // SAFETY: mirrors console.c:383; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        CON_SELECTION = ZERO_SEL;
    }
}

/// `console.c:393`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
#[inline]
unsafe fn con_has_selection() -> bool {
    // SAFETY: mirrors console.c:393; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe { con_ofs_compare(CON_SELECTION.begin, CON_SELECTION.end) != 0 }
}

/// `console.c:403`.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_con_select_all() {
    // SAFETY: mirrors console.c:403; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let (mut b, mut e) = (ZERO_OFS, ZERO_OFS);
        con_get_current_range(&mut b, &mut e);
        CON_SELECTION.begin = b;
        CON_SELECTION.end = e;
        while con_has_selection() && con_str_len(CON_SELECTION.begin.line) == 0 {
            CON_SELECTION.begin.line += 1;
        }
    }
}

/// `console.c:415`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn con_get_normalized_selection(begin: &mut ConOfs, end: &mut ConOfs) -> bool {
    // SAFETY: mirrors console.c:415; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let (mut selbegin, mut selend) = (CON_SELECTION.begin, CON_SELECTION.end);

        if con_ofs_compare(selbegin, selend) > 0 {
            core::mem::swap(&mut selbegin, &mut selend);
        }
        *begin = selbegin;
        *end = selend;

        let (mut tbegin, mut tend) = (ZERO_OFS, ZERO_OFS);
        con_get_current_range(&mut tbegin, &mut tend);

        con_intersect_ranges(begin, end, tbegin, tend)
    }
}

/// `console.c:445`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn con_test_word_boundary(pos: c_int, text: *const c_char, len: c_int) -> c_int {
    // SAFETY: mirrors console.c:445; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        if pos <= 0 {
            return 1;
        }
        if pos >= len {
            return -1;
        }
        let l = q_isspace((*text.offset((pos - 1) as isize) as c_int) & 0x7f) as c_int;
        let r = q_isspace((*text.offset(pos as isize) as c_int) & 0x7f) as c_int;
        l - r
    }
}

/// `console.c:454`.
#[inline]
fn int_sign(i: c_int) -> c_int {
    if i < 0 {
        return -1;
    }
    if i > 0 {
        return 1;
    }
    i
}

/// `console.c:468`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn con_apply_mouse_selection() {
    // SAFETY: mirrors console.c:468; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        CON_SELECTION = CON_MOUSESELECTION;

        let mut line = con_get_line(CON_SELECTION.begin.line);
        let mut len = con_str_len(CON_SELECTION.begin.line) as c_int;

        CON_SELECTION.begin.col = CON_SELECTION.begin.col.min(len);

        if CON_MOUSECLICKS == 2 {
            let boundary = int_sign(con_test_word_boundary(CON_SELECTION.begin.col, line, len));
            let dir = int_sign(con_ofs_compare(CON_SELECTION.end, CON_SELECTION.begin));
            if boundary != 0 && boundary != dir {
                CON_SELECTION.begin.col += boundary;
            }
        }

        if con_ofs_compare(CON_SELECTION.begin, CON_SELECTION.end) > 0 {
            core::ptr::swap(&raw mut CON_SELECTION.begin, &raw mut CON_SELECTION.end);
        }

        len = con_str_len(CON_SELECTION.begin.line) as c_int;
        if CON_SELECTION.begin.col > len {
            CON_SELECTION.begin.line += 1;
            CON_SELECTION.begin.col = 0;
            if con_ofs_compare(CON_SELECTION.begin, CON_SELECTION.end) > 0 {
                CON_SELECTION.end = CON_SELECTION.begin;
            }
        }

        if CON_MOUSECLICKS <= 1 {
            return;
        }

        if CON_MOUSECLICKS >= 4 {
            quake_rs_con_select_all();
            return;
        }

        if CON_MOUSECLICKS == 3 {
            CON_SELECTION.begin.col = 0;
            CON_SELECTION.end.col = 0;
            CON_SELECTION.end.line = CON_SELECTION.end.line.min(g::con_current) + 1;
            return;
        }

        line = con_get_line(CON_SELECTION.begin.line);
        len = con_str_len(CON_SELECTION.begin.line) as c_int;
        while con_test_word_boundary(CON_SELECTION.begin.col, line, len) == 0 {
            CON_SELECTION.begin.col -= 1;
        }

        if CON_SELECTION.end.line <= g::con_current {
            line = con_get_line(CON_SELECTION.end.line);
            len = con_str_len(CON_SELECTION.end.line) as c_int;
            while con_test_word_boundary(CON_SELECTION.end.col, line, len) == 0 {
                CON_SELECTION.end.col += 1;
            }
        }
    }
}

/// `console.c:557`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn con_set_mouse_state(state: c_int) {
    // SAFETY: mirrors console.c:557; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        if CON_MOUSESTATE == state {
            return;
        }

        match state {
            CMS_PRESSED => {
                let (mut x, mut y) = (0, 0);
                g::IN_GetMousePos(&mut x, &mut y);
                con_screen_to_canvas(x, y, &raw mut CON_CLICKX, &raw mut CON_CLICKY);
                let mut pos = ZERO_OFS;
                con_canvas_to_offset(CON_CLICKX, CON_CLICKY, &mut pos, CT_NEAREST);

                if CON_MOUSECLICKS == 0
                    || CON_MOUSECLICKDELAY >= DOUBLECLICK_TIME
                    || con_ofs_compare(pos, CON_MOUSESELECTION.end) != 0
                {
                    CON_MOUSECLICKS = 1;
                } else {
                    CON_MOUSECLICKS += 1;
                }
                CON_MOUSECLICKDELAY = 0.0;
                CON_MOUSESELECTION.end = pos;
                CON_MOUSESELECTION.begin = pos;

                con_apply_mouse_selection();

                if CON_MOUSECLICKS >= 2 {
                    g::VID_SetMouseCursor(MOUSECURSOR_IBEAM);
                }
            }
            CMS_DRAGGING => {
                con_set_hot_link(ptr::null_mut());
                g::VID_SetMouseCursor(MOUSECURSOR_IBEAM);
            }
            CMS_NOTPRESSED => {
                if CON_MOUSESTATE != CMS_DRAGGING
                    && !CON_HOTLINK.is_null()
                    && !g::Sys_Explore((*CON_HOTLINK).path)
                {
                    g::S_LocalSound(c"misc/menu2.wav".as_ptr());
                }
                CON_SCROLLDELTA = 0.0;
                CON_SCROLLSPEED = 0.0;
            }
            _ => {}
        }

        CON_MOUSESTATE = state;
        quake_rs_con_force_mouse_move();
    }
}

/// `console.c:611`.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn Con_Mousemove(x: c_int, y: c_int) {
    // SAFETY: mirrors console.c:611; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        if CON_MOUSESTATE == CMS_NOTPRESSED {
            let mut ofs = ZERO_OFS;
            let inside = con_screen_to_offset(x, y, &mut ofs, CT_INSIDE);
            con_set_hot_link(con_get_link_at_pixel(x, y));
            g::VID_SetMouseCursor(if !CON_HOTLINK.is_null() {
                MOUSECURSOR_HAND
            } else if inside {
                MOUSECURSOR_IBEAM
            } else {
                MOUSECURSOR_DEFAULT
            });
        } else {
            let (mut cx, mut cy) = (0, 0);
            con_screen_to_canvas(x, y, &mut cx, &mut cy);
            con_canvas_to_offset(cx, cy, &raw mut CON_MOUSESELECTION.end, CT_NEAREST);
            con_apply_mouse_selection();
            if con_ofs_compare(CON_MOUSESELECTION.begin, CON_MOUSESELECTION.end) != 0 {
                con_set_mouse_state(CMS_DRAGGING);
            }

            let half = g::con_vislines / 2;
            let mut delta = cy + half - c::cl_parse::vid.conheight;
            if delta.wrapping_abs() < half - CON_SCROLL_ZONE {
                delta = 0;
            } else {
                delta -= int_sign(delta) * (half - CON_SCROLL_ZONE);
            }
            delta = clamp_i(-CON_SCROLL_ZONE, delta, CON_SCROLL_ZONE);

            if delta < 0 {
                let moved = cy - CON_CLICKY;
                let scrolled = (CON_MOUSESELECTION.end.line - CON_MOUSESELECTION.begin.line).min(0)
                    * CHARACTER_SIZE;
                delta = delta.max(moved + scrolled / 4);
                delta = delta.min(0);
            }

            // COMPAT (console.c:658): `frac *= fabs (frac)` promotes to double,
            // so the multiply happens in double precision and rounds once on the
            // way back into the float.
            let mut frac = delta as f32 / CON_SCROLL_ZONE as f32;
            frac = ((frac as f64) * (frac as f64).abs()) as f32;
            CON_SCROLLSPEED = -CON_MAX_SCROLL_SPEED * frac;
            if delta == 0 {
                CON_SCROLLDELTA = 0.0;
            }
        }
    }
}

/// `console.c:665`.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_con_force_mouse_move() {
    // SAFETY: mirrors console.c:665; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let (mut x, mut y) = (0, 0);
        g::IN_GetMousePos(&mut x, &mut y);
        Con_Mousemove(x, y);
    }
}

/// `console.c:679`.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn Con_UpdateMouseState() {
    // SAFETY: mirrors console.c:679; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        if k::key_dest != KEY_CONSOLE {
            con_set_hot_link(ptr::null_mut());
            con_set_mouse_state(CMS_NOTPRESSED);
            con_clear_selection();
            g::VID_SetMouseCursor(MOUSECURSOR_DEFAULT);
            return;
        }

        if !keydown(K_MOUSE1) {
            con_set_mouse_state(CMS_NOTPRESSED);
        } else if CON_MOUSESTATE == CMS_NOTPRESSED {
            con_set_mouse_state(CMS_PRESSED);
        }

        CON_MOUSECLICKDELAY += g::host_rawframetime;

        CON_SCROLLDELTA =
            (CON_SCROLLDELTA as f64 + CON_SCROLLSPEED as f64 * g::host_rawframetime) as f32;
        if (CON_SCROLLDELTA as f64).abs() >= 1.0 {
            let lines = CON_SCROLLDELTA as c_int;
            quake_rs_con_scroll(lines);
            CON_SCROLLDELTA -= lines as f32;
        }
    }
}

/// `console.c:714`.
///
/// COMPAT (`console.c:727`, `:733`): with `len <= 0` the C original writes
/// `bar[len - 1]` and `bar[len]`, i.e. before the start of its own static
/// buffer. Reproducing an out-of-bounds store is not possible in Rust
/// without genuine UB (ADR-004), so those two stores are skipped when the
/// index is negative. `len` is `q_min (40, con_linewidth)` and every
/// in-tree caller passes 40, so this only differs from C on a
/// `con_linewidth <= 0` console, which is already undefined.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn Con_Quakebar(len: c_int) -> *const c_char {
    // SAFETY: mirrors console.c:727; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let bar = (&raw mut QUAKEBAR).cast::<c_char>();
        let len = len.min(42 - 2).min(g::con_linewidth);

        *bar = 0x1d;
        let mut i = 1;
        while i < len - 1 {
            *bar.offset(i as isize) = 0x1e;
            i += 1;
        }
        if len > 0 {
            *bar.offset((len - 1) as isize) = 0x1f;
        }

        if len < g::con_linewidth {
            if len >= 0 {
                *bar.offset(len as isize) = b'\n' as c_char;
                *bar.offset((len + 1) as isize) = 0;
            }
        } else if len >= 0 {
            *bar.offset(len as isize) = 0;
        }

        bar
    }
}

/// `console.c:750` -- the status core behind `console_glue.c:213`.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_con_toggle_console_f() -> Raise {
    // SAFETY: mirrors console.c:750; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        if k::key_dest == KEY_CONSOLE {
            *key_line(k::edit_line).add(1) = 0;
            k::key_linepos = 1;
            g::con_backscroll = 0;
            k::history_line = k::edit_line;
            (&raw mut k::key_tabhint).cast::<c_char>().write(0);
            con_set_hot_link(ptr::null_mut());

            if cls.state == CA_CONNECTED {
                g::IN_Activate();
                k::key_dest = KEY_GAME;
            } else {
                raise!(g::Console_Glue_MenuMain());
            }
        } else {
            g::IN_DeactivateForConsole();
            k::key_dest = KEY_CONSOLE;
        }

        g::SCR_EndLoadingPlaque();
        ptr::write_bytes((&raw mut g::con_times).cast::<u8>(), 0, 4 * 4);
        0
    }
}

/// `console.c:781`. `static` in C; exported so the differential fixture can
/// drive the port's copy the way the oracle TU drives `console.c`'s.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn Con_Clear_f() {
    // SAFETY: mirrors console.c:781; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        if !g::con_text.is_null() {
            ptr::write_bytes(g::con_text, b' ', g::con_buffersize.max(0) as usize);
        }
        g::con_backscroll = 0;

        con_set_hot_link(ptr::null_mut());
        for i in 0..vec_size(CON_LINKS) {
            c::Mem_Free((*CON_LINKS.add(i)).cast());
        }
        g::Vec_Clear((&raw mut CON_LINKS).cast::<*mut c_void>());
    }
}

/// `console.c:800`.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_con_copy_selection_to_clipboard() -> bool {
    // SAFETY: mirrors console.c:800; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let (mut selbegin, mut selend) = (ZERO_OFS, ZERO_OFS);
        let mut qtext: *mut c_char = ptr::null_mut();

        g::S_LocalSound(c"misc/menu2.wav".as_ptr());

        if !con_get_normalized_selection(&mut selbegin, &mut selend) {
            return false;
        }

        let mut cursor = selbegin;
        while con_ofs_compare(cursor, selend) <= 0 {
            let text = con_get_line(cursor.line);
            let mut eol = ConOfs {
                line: cursor.line,
                col: con_str_len(cursor.line) as c_int,
            };
            if cursor.line == selend.line {
                eol.col = eol.col.min(selend.col);
            }
            // COMPAT (console.c:816): `eol.col - cursor.col` is an int widened to
            // size_t, so a backwards range becomes an enormous count exactly as
            // it does in C. Con_GetNormalizedSelection keeps that from happening.
            g::Vec_Append(
                (&raw mut qtext).cast::<*mut c_void>(),
                1,
                text.offset(cursor.col as isize).cast(),
                (eol.col - cursor.col) as usize,
            );
            if eol.line != selend.line {
                vec_push(&raw mut qtext, b'\n' as c_char);
            }
            cursor.line += 1;
            cursor.col = 0;
        }
        vec_push(&raw mut qtext, 0);

        let maxsize = g::UTF8_FromQuake(ptr::null_mut(), 0, qtext);
        let utf8 = c::Mem_Alloc(maxsize).cast::<c_char>();
        g::UTF8_FromQuake(utf8, maxsize, qtext);

        g::Console_Glue_SetClipboardText(utf8);

        c::Mem_Free(utf8.cast());
        g::Vec_Free((&raw mut qtext).cast::<*mut c_void>());

        con_clear_selection();

        true
    }
}

/// `console.c:850`. `static` in C; see [`Con_Clear_f`].
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn Con_Dump_f() {
    // SAFETY: mirrors console.c:850; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let mut buffer: [c_char; 1024] = [0; 1024];
        let mut relname: [c_char; c::MAX_OSPATH] = [0; c::MAX_OSPATH];
        let mut name: [c_char; c::MAX_OSPATH] = [0; c::MAX_OSPATH];

        let arg = if c::Cmd_Argc() >= 2 {
            c::Cmd_Argv(1)
        } else {
            c"condump.txt".as_ptr()
        };
        g::q_strlcpy(relname.as_mut_ptr(), arg, relname.len());
        c::COM_AddExtension(relname.as_mut_ptr(), c".txt".as_ptr(), relname.len());
        g::q_snprintf(
            name.as_mut_ptr(),
            name.len(),
            c"%s/%s".as_ptr(),
            (&raw const c::com_gamedir).cast::<c_char>(),
            relname.as_ptr(),
        );
        let f = c::Sys_fopen(name.as_ptr(), c"w".as_ptr());
        if f.is_null() {
            c::Con_Printf(
                c"ERROR: couldn't open file %s.\n".as_ptr(),
                relname.as_ptr(),
            );
            return;
        }

        // skip initial empty lines
        let mut l = g::con_current - g::con_totallines + 1;
        while l <= g::con_current {
            let line = con_get_line(l);
            let mut x = 0;
            while x < g::con_linewidth {
                if *line.offset(x as isize) != b' ' as c_char {
                    break;
                }
                x += 1;
            }
            if x != g::con_linewidth {
                break;
            }
            l += 1;
        }

        // COMPAT (console.c:876): `char buffer[1024]` is indexed at
        // `con_linewidth`, which C does not bound. The clamp only bites on a
        // console wider than 1023 columns, which `vid.conwidth` cannot produce.
        let width = clamp_i(0, g::con_linewidth, buffer.len() as c_int - 1);
        buffer[width as usize] = 0;
        while l <= g::con_current {
            let line = con_get_line(l);
            g::strncpy(buffer.as_mut_ptr(), line, width as usize);
            let mut x = width - 1;
            while x >= 0 {
                if buffer[x as usize] == b' ' as c_char {
                    buffer[x as usize] = 0;
                } else {
                    break;
                }
                x -= 1;
            }
            let mut x = 0usize;
            while buffer[x] != 0 {
                buffer[x] &= 0x7f;
                x += 1;
            }

            g::fprintf(f, c"%s\n".as_ptr(), buffer.as_ptr());
            l += 1;
        }

        g::fclose(f);
        c::Con_SafePrintf(c"Dumped console text to ".as_ptr());
        g::Con_LinkPrintf(name.as_ptr(), c"%s".as_ptr(), relname.as_ptr());
        c::Con_SafePrintf(c".\n".as_ptr());
    }
}

/// `console.c:910`.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn Con_ClearNotify() {
    // SAFETY: mirrors console.c:910; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        for i in 0..NUM_CON_TIMES as usize {
            g::con_times[i] = 0.0;
        }
    }
}

/// `console.c:923`. `static` in C; see [`Con_Clear_f`].
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn Con_MessageMode_f() {
    // SAFETY: mirrors console.c:923; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        if cls.state != CA_CONNECTED || cls.demoplayback {
            return;
        }
        k::chat_team = false;
        k::key_dest = KEY_MESSAGE;
    }
}

/// `console.c:936`. `static` in C; see [`Con_Clear_f`].
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn Con_MessageMode2_f() {
    // SAFETY: mirrors console.c:936; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        if cls.state != CA_CONNECTED || cls.demoplayback {
            return;
        }
        k::chat_team = true;
        k::key_dest = KEY_MESSAGE;
    }
}

/// `console.c:949`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn con_recalc_offset(ofs: &mut ConOfs, oldnumlines: c_int) {
    // SAFETY: mirrors console.c:949; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        ofs.col = ofs.col.min(g::con_linewidth);
        ofs.line += g::con_totallines - 1 - oldnumlines;
    }
}

/// `console.c:962`.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn Con_CheckResize() {
    // SAFETY: mirrors console.c:962; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let width = (c::cl_parse::vid.conwidth >> 3) - 2;

        if width == g::con_linewidth {
            return;
        }

        c::QMutex_Lock(g::con_mutex);

        let oldwidth = g::con_linewidth;
        g::con_linewidth = width;
        let oldtotallines = g::con_totallines;
        g::con_totallines = idiv(g::con_buffersize, g::con_linewidth);
        let mut numlines = oldtotallines;

        if g::con_totallines < numlines {
            numlines = g::con_totallines;
        }

        let mut numchars = oldwidth;

        if g::con_linewidth < numchars {
            numchars = g::con_linewidth;
        }

        let size = g::con_buffersize.max(0) as usize;
        let tbuf = c::Mem_Alloc(size).cast::<c_char>();

        ptr::copy_nonoverlapping(g::con_text, tbuf, size);
        ptr::write_bytes(g::con_text, b' ', size);

        for i in 0..numlines {
            for j in 0..numchars {
                *g::con_text
                    .offset(((g::con_totallines - 1 - i) * g::con_linewidth + j) as isize) = *tbuf
                    .offset(
                        (imod(g::con_current - i + oldtotallines, oldtotallines) * oldwidth + j)
                            as isize,
                    );
            }
        }

        c::Mem_Free(tbuf.cast());

        for i in 0..vec_size(CON_LINKS) {
            let link = *CON_LINKS.add(i);
            let cur = g::con_current;
            con_recalc_offset(&mut (*link).begin, cur);
            con_recalc_offset(&mut (*link).end, cur);
        }

        Con_ClearNotify();

        g::con_backscroll = 0;
        g::con_current = g::con_totallines - 1;

        c::QMutex_Unlock(g::con_mutex);
    }
}

/// `console.c:1023`.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_con_scroll(lines: c_int) {
    // SAFETY: mirrors console.c:1023; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        if lines == 0 {
            return;
        }

        g::con_backscroll += lines;

        if lines > 0 {
            let limit = g::con_totallines - (c::cl_parse::vid.height >> 3) - 1;
            if g::con_backscroll > limit {
                g::con_backscroll = limit;
            }
        } else if g::con_backscroll < 0 {
            g::con_backscroll = 0;
        }

        quake_rs_con_force_mouse_move();
    }
}

/// `console.c:1049`.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn Con_Init() {
    // SAFETY: mirrors console.c:1049; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        g::con_mutex = c::QMutex_Create();

        let i = c::COM_CheckParm(c"-consize".as_ptr());
        if i != 0 && i < c::com_argc - 1 {
            g::con_buffersize =
                CON_MINSIZE.max(g::atoi(*c::com_argv.offset((i + 1) as isize)).wrapping_mul(1024));
        } else {
            g::con_buffersize = CON_TEXTSIZE;
        }

        g::con_text = c::Mem_Alloc(g::con_buffersize.max(0) as usize).cast();
        ptr::write_bytes(g::con_text, b' ', g::con_buffersize.max(0) as usize);
        g::con_linewidth = -1;

        // COMPAT (console.c:1068): the fixed 50 columns are what a headless run
        // keeps for its whole life -- Con_CheckResize's only caller sits inside
        // the SCR_UpdateScreen path that -headless skips.
        g::con_linewidth = 50;
        g::con_totallines = idiv(g::con_buffersize, g::con_linewidth);
        g::con_backscroll = 0;
        g::con_current = g::con_totallines - 1;

        c::Con_Printf(c"Console initialized.\n".as_ptr());

        c::Cvar_RegisterVariable(&raw mut g::con_notifytime);
        c::Cvar_RegisterVariable(&raw mut g::con_logcenterprint);
        c::Cvar_RegisterVariable(&raw mut g::con_notifycenter);
        c::Cvar_RegisterVariable(&raw mut g::con_notifyfade);
        c::Cvar_RegisterVariable(&raw mut g::con_notifyfadetime);
        c::Cvar_RegisterVariable(&raw mut g::con_maxcols);

        // the glue's re-raising wrapper, never the status core (ADR-009)
        add_command(c"toggleconsole".as_ptr(), g::Con_ToggleConsole_f);
        add_command(c"messagemode".as_ptr(), Con_MessageMode_f);
        add_command(c"messagemode2".as_ptr(), Con_MessageMode2_f);
        add_command(c"clear".as_ptr(), Con_Clear_f);
        add_command(c"condump".as_ptr(), Con_Dump_f);
        g::con_initialized = true;
    }
}

/// `console.c:1096`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn con_linefeed() {
    // SAFETY: mirrors console.c:1096; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        if g::con_backscroll != 0 {
            g::con_backscroll += 1;
        }
        let limit = g::con_totallines - (g::glheight >> 3) - 1;
        if g::con_backscroll > limit {
            g::con_backscroll = limit;
        }

        g::con_x = 0;
        g::con_current += 1;
        if g::con_linewidth > 0 {
            ptr::write_bytes(
                g::con_text.offset(
                    imod(g::con_current, g::con_totallines).wrapping_mul(g::con_linewidth) as isize,
                ),
                b' ',
                g::con_linewidth as usize,
            );
        }
    }
}

/// `console.c:1119` -- the core behind the glue's `Con_Printf`
/// (`console_glue.c:262`).
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_con_print(txt: *const c_char) {
    // SAFETY: mirrors console.c:1119; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let mut txt = txt;
        let mask;

        c::QMutex_Lock(g::con_mutex);

        if *txt as c_int == 1 {
            mask = 128;
            // COMPAT (console.c:1133): S_LocalSound runs with con_mutex held.
            g::S_LocalSound(c"misc/talk.wav".as_ptr());
            txt = txt.add(1);
        } else if *txt as c_int == 2 {
            mask = 128;
            txt = txt.add(1);
        } else {
            mask = 0;
        }

        let mut boundary = true;

        loop {
            // COMPAT (console.c:1147): `int c = *txt` keeps the platform's char
            // signedness, so bytes >= 0x80 compare as <= ' ' wherever char is
            // signed -- and do not where it is not.
            let ch = *txt as c_int;
            if ch == 0 {
                break;
            }

            if ch <= b' ' as c_int {
                boundary = true;
            } else if boundary {
                let mut l = 0;
                while l < g::con_linewidth {
                    if (*txt.offset(l as isize) as c_int) <= b' ' as c_int {
                        break;
                    }
                    l += 1;
                }

                if l != g::con_linewidth && (g::con_x + l > g::con_linewidth) {
                    g::con_x = 0;
                }

                boundary = false;
            }

            txt = txt.add(1);

            if PRINT_CR != 0 {
                g::con_current -= 1;
                PRINT_CR = 0;
            }

            if g::con_x == 0 {
                con_linefeed();
                if g::con_current >= 0 {
                    g::con_times[(g::con_current % NUM_CON_TIMES) as usize] = g::realtime as f32;
                }
            }

            match ch {
                10 => g::con_x = 0,
                13 => {
                    g::con_x = 0;
                    PRINT_CR = 1;
                }
                _ => {
                    let y = imod(g::con_current, g::con_totallines);
                    *g::con_text.offset((y.wrapping_mul(g::con_linewidth) + g::con_x) as isize) =
                        (ch | mask) as c_char;
                    g::con_x += 1;
                    if g::con_x >= g::con_linewidth {
                        g::con_x = 0;
                    }
                }
            }
        }

        c::QMutex_Unlock(g::con_mutex);
    }
}

/// `console.c:1216`.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn Con_DebugLog(msg: *const c_char) {
    // SAFETY: mirrors console.c:1216; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        if LOG_FILE.is_null() {
            return;
        }

        g::fwrite(msg.cast(), 1, strlen(msg), LOG_FILE);
    }
}

/// `console.c:1229` -- `static` in C; the glue calls it from `Con_Printf`.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_con_strip_control_prefixes(txt: *const c_char) -> *const c_char {
    // SAFETY: mirrors console.c:1229; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        if *txt as c_int == 1 || *txt as c_int == 2 {
            return txt.add(1);
        }
        txt
    }
}

/// `console.c:1383` -- the core behind the glue's `Con_LinkPrintf`
/// (`console_glue.c:344`). `msg` is already formatted.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_con_link_print(addr: *const c_char, msg: *const c_char) {
    // SAFETY: mirrors console.c:1383; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let len = strlen(addr);
        let link = c::Mem_Alloc(core::mem::size_of::<ConLink>() + len + 1).cast::<ConLink>();

        // COMPAT (console.c:1393): con_mutex is held across Con_SafePrintf, which
        // locks it again. The engine's qmutex is recursive; this port keeps the
        // same lock/unlock ordering rather than flattening it.
        c::QMutex_Lock(g::con_mutex);

        let payload = link.add(1).cast::<c_char>();
        ptr::copy_nonoverlapping(addr, payload, len + 1);
        (*link).path = payload;
        (*link).begin = ConOfs {
            line: g::con_current,
            col: g::con_x,
        };
        (*link).end = (*link).begin;

        c::Con_SafePrintf(c"\x02%s".as_ptr(), msg);

        (*link).end.line = g::con_current;
        (*link).end.col = g::con_x;
        vec_push(&raw mut CON_LINKS, link);

        // Because of wrapping our text might actually start on the next line, so
        // we skip leading spaces
        let mut text = g::con_text.offset(
            (imod((*link).begin.line, g::con_totallines).wrapping_mul(g::con_linewidth)
                + (*link).begin.col) as isize,
        );
        while con_ofs_compare((*link).begin, (*link).end) < 0 {
            if (*text as c_int) & 0x7f != b' ' as c_int {
                break;
            }
            text = text.add(1);
            (*link).begin.col += 1;
            if (*link).begin.col == g::con_linewidth {
                (*link).begin.col = 0;
                (*link).begin.line += 1;
            }
        }

        c::QMutex_Unlock(g::con_mutex);
    }
}

/// `console.c:1460` -- the core behind the glue's `Con_CenterPrintf`
/// (`console_glue.c:375`). `msg` is already formatted.
///
/// COMPAT (`console.c:1487`): `char spaces[21]` is indexed at
/// `(linewidth - len) / 2`, which C does not bound. Every in-tree caller
/// passes `linewidth == 40`, giving a maximum index of 20 -- exactly the
/// last slot. The clamp only bites past that.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_con_center_print(linewidth: c_int, msg: *const c_char) {
    // SAFETY: mirrors console.c:1487; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let mut line: [c_char; MAXPRINTMSG] = [0; MAXPRINTMSG];
        let mut spaces: [c_char; 21] = [0; 21];

        let linewidth = linewidth.min(g::con_linewidth);
        let mut src = msg;
        while *src != 0 {
            let mut dst = 0usize;
            while *src != 0 && *src != b'\n' as c_char {
                line[dst] = *src;
                dst += 1;
                src = src.add(1);
            }
            line[dst] = 0;
            if *src == b'\n' as c_char {
                src = src.add(1);
            }

            let len = dst as c_int;
            if len < linewidth {
                let s = clamp_i(0, (linewidth - len) / 2, spaces.len() as c_int - 1) as usize;
                spaces[..s].fill(b' ' as c_char);
                spaces[s] = 0;
                c::Con_Printf(c"%s%s\n".as_ptr(), spaces.as_ptr(), line.as_ptr());
            } else {
                c::Con_Printf(c"%s\n".as_ptr(), line.as_ptr());
            }
        }
    }
}

/// `console.c:1501`.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn Con_LogCenterPrint(str_: *const c_char) {
    // SAFETY: mirrors console.c:1501; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        if cls.demoseeking {
            return;
        }

        let last = (&raw mut g::con_lastcenterstring).cast::<c_char>();
        if streq(str_, last) {
            return;
        }

        if cl.gametype == GAME_DEATHMATCH && g::con_logcenterprint.value != 2.0 {
            return;
        }

        // COMPAT (console.c:1512): C uses strcpy into con_lastcenterstring[1024].
        // scr_centerstring is itself 1024 bytes, so the copy always fits; the
        // clamp is defence in depth on a server-supplied string.
        let len = strlen(str_).min(1023);
        ptr::copy_nonoverlapping(str_, last, len);
        *last.add(len) = 0;

        if g::con_logcenterprint.value != 0.0 {
            let slen = strlen(str_);
            let trailing_newline = slen != 0 && *str_.add(slen - 1) == b'\n' as c_char;
            c::Con_Printf(c"%s".as_ptr(), Con_Quakebar(40));
            g::Con_CenterPrintf(
                40,
                if trailing_newline {
                    c"%s".as_ptr()
                } else {
                    c"%s\n".as_ptr()
                },
                str_,
            );
            c::Con_Printf(c"%s".as_ptr(), Con_Quakebar(40));
            Con_ClearNotify();
        }
    }
}

/// `console.c:1524`.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn Con_IsRedirected() -> bool {
    // SAFETY: mirrors console.c:1524; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let flush = g::con_redirect_flush;
        flush.is_some()
    }
}

/// `console.c:1528`.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn Con_Redirect(flush: Option<unsafe extern "C" fn(*const c_char)>) {
    // SAFETY: mirrors console.c:1528; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let buf = (&raw mut g::con_redirect_buffer).cast::<c_char>();
        if let Some(f) = g::con_redirect_flush {
            f(buf);
        }
        *buf = 0;
        g::con_redirect_flush = flush;
    }
}

/* ------------------------------------------------------------------------
 * TAB COMPLETION (console.c:1538-2110)
 */

/// `console.c:1573`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn con_clear_tab_list() {
    // SAFETY: mirrors console.c:1573; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        if g::tablist.is_null() {
            return;
        }

        (*(*g::tablist).prev).next = ptr::null_mut(); // break the loop
        let mut t = g::tablist;
        while !t.is_null() {
            let next = (*t).next;
            c::Mem_Free(t.cast());
            t = next;
        }
        g::tablist = ptr::null_mut();
    }
}

/// `console.c:1602`.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn Con_AddToTabList(
    name: *const c_char,
    partial: *const c_char,
    ty: *const c_char,
) {
    // SAFETY: mirrors console.c:1602; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        if !Con_Match(name, partial) {
            return;
        }

        let bash = (&raw mut BASH_PARTIAL).cast::<c_char>();

        if *bash == 0 && BASH_SINGLEMATCH {
            g::q_strlcpy(bash, name, 80);
        } else {
            BASH_SINGLEMATCH = false;
            let mut i_bash = g::q_strcasestr(bash, partial);
            let mut i_name = g::q_strcasestr(name, partial);
            if !i_name.is_null() && !i_bash.is_null() {
                let mut i_bash2 = i_bash;
                let mut i_name2 = i_name;
                // find max common between bash_partial and name (right side)
                while *i_bash != 0 && q_toupper(*i_bash as c_int) == q_toupper(*i_name as c_int) {
                    i_bash = i_bash.add(1);
                    i_name = i_name.add(1);
                }
                *i_bash = 0;
                // find max common between bash_partial and name (left side)
                while i_bash2 != bash
                    && i_name2 != name.cast_mut()
                    && q_toupper(*i_bash2.offset(-1) as c_int)
                        == q_toupper(*i_name2.offset(-1) as c_int)
                {
                    i_bash2 = i_bash2.offset(-1);
                    i_name2 = i_name2.offset(-1);
                }
                if i_bash2 != bash {
                    ptr::copy(i_bash2, bash, strlen(i_bash2) + 1);
                }
            }
        }

        let namelen = strlen(name) + 1;
        let typelen = if ty.is_null() { 0 } else { strlen(ty) + 1 };
        let t =
            c::Mem_Alloc(core::mem::size_of::<g::tab_t>() + namelen + typelen).cast::<g::tab_t>();
        let payload = t.add(1).cast::<c_char>();
        (*t).name = payload;
        ptr::copy_nonoverlapping(name, payload, namelen);
        if !ty.is_null() {
            (*t).ty = payload.add(namelen);
            ptr::copy_nonoverlapping(ty, payload.add(namelen), typelen);
        } else {
            (*t).ty = ptr::null();
        }
        (*t).count = 1;

        if g::tablist.is_null() {
            g::tablist = t;
            (*t).next = t;
            (*t).prev = t;
        } else if g::q_strnaturalcmp(name, (*g::tablist).name) < 0 {
            (*t).next = g::tablist;
            (*t).prev = (*g::tablist).prev;
            (*(*t).next).prev = t;
            (*(*t).prev).next = t;
            g::tablist = t;
        } else {
            let mut insert = g::tablist;
            loop {
                let cmp = g::q_strnaturalcmp(name, (*insert).name);
                if cmp == 0 && streq(name, (*insert).name) {
                    c::Mem_Free(t.cast());
                    (*insert).count += 1;
                    return;
                }
                if cmp < 0 {
                    break;
                }
                insert = (*insert).next;
                if insert == g::tablist {
                    break;
                }
            }

            (*t).next = insert;
            (*t).prev = (*insert).prev;
            (*(*t).next).prev = t;
            (*(*t).prev).next = t;
        }
    }
}

/// `console.c:1698`.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn Con_Match(str_: *const c_char, partial: *const c_char) -> bool {
    // SAFETY: mirrors console.c:1698; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe { !g::q_strcasestr(str_, partial).is_null() }
}

/// `console.c:1708`. The return value is unused by the one caller
/// (`console.c:1848`), so it is dropped here.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn parse_command() {
    // SAFETY: mirrors console.c:1848; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let mut buf: [c_char; MAXCMDLINE] = [0; MAXCMDLINE];
        let line = key_line(k::edit_line);
        let mut str_ = line.add(1) as *const c_char;
        let mut end = str_.offset((k::key_linepos - 1) as isize);
        let mut ret = str_;
        let mut quote: *const c_char = ptr::null();

        while *str_ != 0 && str_ != end {
            let ch = *str_;
            str_ = str_.add(1);
            if ch == b'"' as c_char {
                if quote.is_null() {
                    quote = ret;
                    ret = str_;
                } else {
                    ret = quote;
                    quote = ptr::null();
                }
            } else if ch == b';' as c_char {
                ret = str_;
            } else if quote.is_null() && ch == b'/' as c_char && *str_ == b'/' as c_char {
                break;
            }
        }

        while *ret == b' ' as c_char {
            ret = ret.add(1);
        }

        g::q_strlcpy(buf.as_mut_ptr(), ret, buf.len());
        let span = end.offset_from(ret) as usize;
        if span < buf.len() {
            buf[span] = 0;
        }
        end = buf.as_ptr().add(strlen(buf.as_ptr()));

        g::Cmd_TokenizeString(buf.as_ptr());
        // last arg should always be the one we're trying to complete, so we add a
        // new empty one if the command ends with a space
        if end != buf.as_ptr() && *end.offset(-1) == b' ' as c_char {
            g::Cmd_AddArg(c"".as_ptr());
        }
    }
}

/// `console.c:1755`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn complete_file_list(partial: *const c_char, param: *mut c_void) -> bool {
    // SAFETY: mirrors console.c:1755; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let list = param.cast::<*mut FileListItem>();
        let mut file = *list;
        while !file.is_null() {
            Con_AddToTabList((*file).name.as_ptr(), partial, ptr::null());
            file = (*file).next;
        }
        true
    }
}

/// `console.c:1763`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn complete_file_list_single(partial: *const c_char, param: *mut c_void) -> bool {
    // SAFETY: mirrors console.c:1763; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        if c::Cmd_Argc() < 3 {
            complete_file_list(partial, param);
        }
        true
    }
}

/// `console.c:1770`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn complete_bind_keys(partial: *const c_char, _param: *mut c_void) -> bool {
    // SAFETY: mirrors console.c:1770; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        if c::Cmd_Argc() > 2 {
            return false;
        }

        for i in 0..MAX_KEYS {
            let name = crate::keys::Key_KeynumToString(i);
            if !streq(name, c"<UNKNOWN KEYNUM>".as_ptr()) {
                Con_AddToTabList(name, partial, k::keybindings[i as usize]);
            }
        }

        true
    }
}

/// `console.c:1788`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn complete_unbind_keys(partial: *const c_char, _param: *mut c_void) -> bool {
    // SAFETY: mirrors console.c:1788; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        if c::Cmd_Argc() > 2 {
            return true;
        }

        for i in 0..MAX_KEYS {
            if !k::keybindings[i as usize].is_null() {
                let name = crate::keys::Key_KeynumToString(i);
                if !streq(name, c"<UNKNOWN KEYNUM>".as_ptr()) {
                    Con_AddToTabList(name, partial, k::keybindings[i as usize]);
                }
            }
        }

        true
    }
}

/// The `void *param` column of `arg_completion_types` (`console.c:1817`).
/// The C table stores `&extralevels` and friends, which are not
/// const-evaluable addresses in Rust, so the head is named here and
/// resolved at call time.
#[derive(Clone, Copy)]
enum ListParam {
    None,
    ExtraLevels,
    ModList,
    DemoList,
    SaveList,
}

fn list_param_ptr(p: ListParam) -> *mut c_void {
    match p {
        ListParam::None => ptr::null_mut(),
        ListParam::ExtraLevels => (&raw mut c::host_cmd::extralevels).cast(),
        ListParam::ModList => (&raw mut c::host_cmd::modlist).cast(),
        ListParam::DemoList => (&raw mut c::host_cmd::demolist).cast(),
        ListParam::SaveList => (&raw mut c::host_cmd::savelist).cast(),
    }
}

type CompletionFn = unsafe fn(*const c_char, *mut c_void) -> bool;

/// `console.c:1817-1829` -- `arg_completion_types`, in declaration order.
static ARG_COMPLETION_TYPES: &[(&core::ffi::CStr, CompletionFn, ListParam)] = &[
    (c"map", complete_file_list_single, ListParam::ExtraLevels),
    (
        c"changelevel",
        complete_file_list_single,
        ListParam::ExtraLevels,
    ),
    (c"game", complete_file_list, ListParam::ModList),
    (c"record", complete_file_list_single, ListParam::DemoList),
    (c"playdemo", complete_file_list_single, ListParam::DemoList),
    (c"timedemo", complete_file_list_single, ListParam::DemoList),
    (c"load", complete_file_list_single, ListParam::SaveList),
    (c"save", complete_file_list_single, ListParam::SaveList),
    (c"fastload", complete_file_list_single, ListParam::SaveList),
    (c"bind", complete_bind_keys, ListParam::None),
    (c"unbind", complete_unbind_keys, ListParam::None),
];

/// `console.c:1837`. Raise-capable through the two completion callbacks.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn build_tab_list(partial: *const c_char) -> Raise {
    // SAFETY: mirrors console.c:1837; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        con_clear_tab_list();

        (&raw mut BASH_PARTIAL).cast::<c_char>().write(0);
        BASH_SINGLEMATCH = true;

        parse_command();

        if c::Cmd_Argc() >= 2 {
            let cvar = g::Cvar_FindVar(c::Cmd_Argv(0));
            if !cvar.is_null() {
                // cvars can only have one argument
                if c::Cmd_Argc() == 2 && (*cvar).completion.is_some() {
                    raise!(g::Console_Glue_CvarCompletion(cvar, partial));
                }
                return 0;
            }

            let cmd = g::Cmd_FindCommand(c::Cmd_Argv(0));

            if !cmd.is_null() && (*cmd).completion.is_some() {
                raise!(g::Console_Glue_CmdCompletion((*cmd).completion, partial));
                return 0;
            }

            if !cmd.is_null() {
                for &(command, function, param) in ARG_COMPLETION_TYPES {
                    if g::q_strcasecmp(c::Cmd_Argv(0), command.as_ptr()) == 0 {
                        if function(partial, list_param_ptr(param)) {
                            return 0;
                        }
                        break;
                    }
                }
            }
        }

        if *partial == 0 {
            return 0;
        }

        let mut cvar = g::Cvar_FindVarAfter(c"".as_ptr(), 0);
        while !cvar.is_null() {
            if !g::q_strcasestr((*cvar).name, partial).is_null() {
                Con_AddToTabList((*cvar).name, partial, c"cvar".as_ptr());
            }
            cvar = (*cvar).next;
        }

        let mut cmd = g::cmd_functions;
        while !cmd.is_null() {
            if (*cmd).srctype != c::cmd_source_t_src_server
                && !g::q_strcasestr((*cmd).name, partial).is_null()
                && !g::Cmd_IsReservedName((*cmd).name)
            {
                Con_AddToTabList((*cmd).name, partial, c"command".as_ptr());
            }
            cmd = (*cmd).next;
        }

        let mut alias = g::cmd_alias;
        while !alias.is_null() {
            let name = (&raw const (*alias).name).cast::<c_char>();
            if !g::q_strcasestr(name, partial).is_null() {
                Con_AddToTabList(name, partial, c"alias".as_ptr());
            }
            alias = (*alias).next;
        }

        0
    }
}

/// `console.c:1910`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn con_format_tab_match(t: *const g::tab_t, dst: *mut c_char, dstsize: usize) {
    // SAFETY: mirrors console.c:1910; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let mut tinted: [c_char; MAXCMDLINE] = [0; MAXCMDLINE];

        g::COM_TintSubstring(
            (*t).name,
            (&raw const BASH_PARTIAL).cast::<c_char>(),
            tinted.as_mut_ptr(),
            tinted.len(),
        );

        if (*t).ty.is_null() {
            g::q_strlcpy(dst, tinted.as_ptr(), dstsize);
        } else if *(*t).ty == b'#' as c_char && *(*t).ty.add(1) == 0 {
            g::q_snprintf(
                dst,
                dstsize,
                c"%s (%d)".as_ptr(),
                tinted.as_ptr(),
                (*t).count,
            );
        } else {
            g::q_snprintf(dst, dstsize, c"%s (%s)".as_ptr(), tinted.as_ptr(), (*t).ty);
        }
    }
}

/// `console.c:1929`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn con_print_tab_list() {
    // SAFETY: mirrors console.c:1929; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let mut buf: [c_char; MAXCMDLINE] = [0; MAXCMDLINE];

        // determine maximum item length
        let mut matches = 0;
        let mut maxlen = 0;
        let mut t = g::tablist;
        loop {
            con_format_tab_match(t, buf.as_mut_ptr(), buf.len());
            let total = strlen(buf.as_ptr()) as c_int;
            maxlen = maxlen.max(total);
            t = (*t).next;
            matches += 1;
            if t == g::tablist {
                break;
            }
        }

        // determine number of columns
        if maxlen == 0 {
            return;
        }
        maxlen += 3; // indent
        maxlen = maxlen.max(8); // min width
        maxlen = (maxlen + 3) & !3; // round up to multiple of 4
        let mut cols = idiv(g::con_linewidth.max(maxlen), maxlen);
        if g::con_maxcols.value >= 1.0 {
            cols = cols.min(g::con_maxcols.value as c_int);
        }
        if matches < 6 {
            cols = 1;
        }

        // print all matches
        c::Con_SafePrintf(c"\n".as_ptr());
        let mut i = 0;
        let mut total = 0;
        t = g::tablist;
        loop {
            con_format_tab_match(t, buf.as_mut_ptr(), buf.len());
            i += 1;
            if i == cols {
                i = 0;
                c::Con_SafePrintf(c"   %s\n".as_ptr(), buf.as_ptr());
            } else {
                c::Con_SafePrintf(c"   %*s".as_ptr(), -(maxlen - 3), buf.as_ptr());
            }
            if !(*t).ty.is_null() && *(*t).ty == b'#' as c_char && *(*t).ty.add(1) == 0 {
                total += (*t).count;
            }
            t = (*t).next;
            if t == g::tablist {
                break;
            }
        }
        if i != 0 {
            c::Con_SafePrintf(c"\n".as_ptr());
        }

        if total > 0 {
            c::Con_SafePrintf(
                c"   %d unique matches (%d total)\n".as_ptr(),
                matches,
                total,
            );
        }

        c::Con_SafePrintf(c"\n".as_ptr());
    }
}

/// `console.c:1991` -- the status core behind `console_glue.c:220`.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_con_tab_complete(mode: c_int) -> Raise {
    // SAFETY: mirrors console.c:1991; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let mut partial: [c_char; MAXCMDLINE] = [0; MAXCMDLINE];
        let mut matched: *const c_char;

        let tabhint = (&raw mut k::key_tabhint).cast::<c_char>();
        let tabpartial = (&raw mut g::key_tabpartial).cast::<c_char>();

        *tabhint = 0;
        if mode == TABCOMPLETE_AUTOHINT {
            *tabpartial = 0;

            // only show completion hint when the cursor is at the end of the line
            if k::key_linepos as usize >= MAXCMDLINE
                || *key_line(k::edit_line).offset(k::key_linepos as isize) != 0
            {
                return 0;
            }
        }

        // if editline is empty, return
        if *key_line(k::edit_line).add(1) == 0 {
            return 0;
        }

        // get partial string (space -> cursor)
        if *tabpartial == 0 {
            // work back from cursor until you find a space, quote, semicolon, or prompt
            let line = key_line(k::edit_line);
            TAB_C = line.offset((k::key_linepos - 1) as isize);
            while *TAB_C != b' ' as c_char
                && *TAB_C != b'"' as c_char
                && *TAB_C != b';' as c_char
                && TAB_C != line
            {
                TAB_C = TAB_C.offset(-1);
            }
            TAB_C = TAB_C.add(1);
        }
        let mut i = 0isize;
        while TAB_C.offset(i) < key_line(k::edit_line).offset(k::key_linepos as isize) {
            partial[i as usize] = *TAB_C.offset(i);
            i += 1;
        }
        partial[i as usize] = 0;

        // trim trailing space becuase it screws up string comparisons
        if i > 0 && partial[(i - 1) as usize] == b' ' as c_char {
            partial[(i - 1) as usize] = 0;
        }

        // find a match
        if *tabpartial == 0 {
            g::q_strlcpy(tabpartial, partial.as_ptr(), MAXCMDLINE);
            raise!(build_tab_list(tabpartial));

            if g::tablist.is_null() {
                return 0;
            }

            // print list if length > 1 and action is user-initiated
            if (*g::tablist).next != g::tablist && mode == TABCOMPLETE_USER {
                con_print_tab_list();
            }

            // First time, just show maximum matching chars -- S.A.
            matched = if BASH_SINGLEMATCH {
                (*g::tablist).name
            } else {
                (&raw const BASH_PARTIAL).cast::<c_char>()
            };
        } else {
            raise!(build_tab_list(tabpartial));

            if g::tablist.is_null() {
                return 0;
            }

            // find current match -- can't save a pointer because the list will be
            // rebuilt each time
            let mut t = g::tablist;
            matched = if keydown(K_SHIFT) {
                (*(*t).prev).name
            } else {
                (*t).name
            };
            loop {
                if g::q_strcasecmp((*t).name, partial.as_ptr()) == 0 {
                    matched = if keydown(K_SHIFT) {
                        (*(*t).prev).name
                    } else {
                        (*(*t).next).name
                    };
                    break;
                }
                t = (*t).next;
                if t == g::tablist {
                    break;
                }
            }
        }

        if mode == TABCOMPLETE_AUTOHINT {
            let len = strlen(partial.as_ptr());
            matched = g::q_strcasestr(matched, partial.as_ptr());
            if !matched.is_null() && *matched.add(len) != 0 {
                g::q_strlcat(tabhint, matched.add(len), MAXCMDLINE);
            }
            con_clear_tab_list();
            *tabpartial = 0;
            return 0;
        }

        // insert new match into edit line
        let line = key_line(k::edit_line);
        g::q_strlcpy(partial.as_mut_ptr(), matched, MAXCMDLINE);
        g::q_strlcat(
            partial.as_mut_ptr(),
            line.offset(k::key_linepos as isize),
            MAXCMDLINE,
        );
        *TAB_C = 0;
        g::q_strlcat(line, partial.as_ptr(), MAXCMDLINE);
        k::key_linepos = (TAB_C.offset_from(line) + strlen(matched) as isize) as c_int;
        if k::key_linepos >= MAXCMDLINE as c_int {
            k::key_linepos = MAXCMDLINE as c_int - 1;
        }

        con_clear_tab_list();

        // if cursor is at end of string, let's append a space to make life easier
        if k::key_linepos < MAXCMDLINE as c_int - 1
            && *line.offset(k::key_linepos as isize) == 0
            && BASH_SINGLEMATCH
        {
            *line.offset(k::key_linepos as isize) = b' ' as c_char;
            k::key_linepos += 1;
            *line.offset(k::key_linepos as isize) = 0;
            *tabpartial = 0; // restart cycle
            TAB_C = line.offset(k::key_linepos as isize);

            // ADR-009: the self-call goes to the core, never through the glue's
            // Host_Reraise wrapper, so no jump crosses this Rust frame.
            raise!(quake_rs_con_tab_complete(TABCOMPLETE_AUTOHINT));
        }

        0
    }
}

/* ------------------------------------------------------------------------
 * DRAWING (console.c:2113-2373)
 */

/// `console.c:2121`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn con_notify_alpha(time: f32) -> f32 {
    // SAFETY: mirrors console.c:2121; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let notifytime = g::con_notifytime.value
            / if g::scr_viewsize.value >= 130.0 {
                4.0
            } else {
                1.0
            };
        if time == 0.0 {
            return 0.0;
        }
        let fade = max_f(g::con_notifyfade.value * g::con_notifyfadetime.value, 0.0);
        // COMPAT (console.c:2128): `time += notifytime + fade - realtime` widens
        // to double for the subtraction, then rounds once on the way back.
        let time = (time as f64 + (notifytime + fade) as f64 - g::realtime) as f32;
        if time <= 0.0 {
            return 0.0;
        }
        if fade == 0.0 {
            return 1.0;
        }
        min_f(time / fade, 1.0)
    }
}

/// `console.c:2144` -- reached through the glue's `Con_DrawNotify`
/// (`console_glue.c:392`), which owns the `cb_context_t *` spelling.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_con_draw_notify(cbx: *mut c_void) {
    // SAFETY: mirrors console.c:2144; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        g::GL_SetCanvas(cbx, CANVAS_CONSOLE);
        let mut v = c::cl_parse::vid.conheight;

        let mut i = g::con_current - NUM_CON_TIMES + 1;
        while i <= g::con_current {
            if i < 0 {
                i += 1;
                continue;
            }
            let alpha = con_notify_alpha(g::con_times[(i % NUM_CON_TIMES) as usize]);
            if alpha <= 0.0 {
                i += 1;
                continue;
            }
            let text = con_get_line(i);

            g::GL_SetCanvasColor(1.0, 1.0, 1.0, alpha);
            if g::con_notifycenter.value != 0.0 {
                let mut len = g::con_linewidth;
                while len > 0 && *text.offset((len - 1) as isize) == b' ' as c_char {
                    len -= 1;
                }
                for x in 0..len {
                    g::Draw_Character(
                        cbx,
                        ((g::con_linewidth - len) * 4 + (x + 1) * 8) as f32,
                        v as f32,
                        *text.offset(x as isize) as c_int,
                    );
                }
            } else {
                for x in 0..g::con_linewidth {
                    g::Draw_Character(
                        cbx,
                        ((x + 1) << 3) as f32,
                        v as f32,
                        *text.offset(x as isize) as c_int,
                    );
                }
            }
            g::GL_SetCanvasColor(1.0, 1.0, 1.0, 1.0);

            v += 8;
            i += 1;
        }

        if k::key_dest == KEY_MESSAGE {
            let mut x;
            if k::chat_team {
                g::Draw_String(cbx, 8.0, v as f32, c"say_team:".as_ptr());
                x = 11;
            } else {
                g::Draw_String(cbx, 8.0, v as f32, c"say:".as_ptr());
                x = 6;
            }

            let mut text = crate::keys::Key_GetChatBuffer();
            let len = crate::keys::Key_GetChatMsgLen();
            if len > g::con_linewidth - x - 1 {
                text = text.offset((len - g::con_linewidth + x + 1) as isize);
            }

            while *text != 0 {
                g::Draw_Character(cbx, (x << 3) as f32, v as f32, *text as c_int);
                x += 1;
                text = text.add(1);
            }

            g::Draw_Character(
                cbx,
                (x << 3) as f32,
                v as f32,
                10 + (((g::realtime * g::con_cursorspeed as f64) as c_int) & 1),
            );
        }
    }
}

/// `console.c:2218` -- reached through the glue's `Con_DrawInput`
/// (`console_glue.c:398`).
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_con_draw_input(cbx: *mut c_void) {
    // SAFETY: mirrors console.c:2218; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let workline = key_line(k::edit_line);

        if k::key_dest != KEY_CONSOLE && !g::con_forcedup {
            return; // don't draw anything
        }

        // prestep if horizontally scrolling
        let ofs = if k::key_linepos >= g::con_linewidth {
            1 + k::key_linepos - g::con_linewidth
        } else {
            0
        };

        let len = strlen(workline) as c_int;
        let conheight = c::cl_parse::vid.conheight;

        // draw input string
        let mut i = 0;
        while i + ofs < len && i < g::con_linewidth {
            g::Draw_Character(
                cbx,
                ((i + 1) << 3) as f32,
                (conheight - 16) as f32,
                *workline.offset((i + ofs) as isize) as c_int,
            );
            i += 1;
        }

        // draw tab completion hint
        let tabhint = (&raw const k::key_tabhint).cast::<c_char>();
        if *tabhint != 0 {
            g::GL_SetCanvasColor(1.0, 1.0, 1.0, 0.75);
            let mut i = 0;
            while *tabhint.offset(i as isize) != 0 && i + 1 + len - ofs < g::con_linewidth + 2 {
                g::Draw_Character(
                    cbx,
                    ((i + 1 + len - ofs) << 3) as f32,
                    (conheight - 16) as f32,
                    (*tabhint.offset(i as isize) as c_int) | 0x80,
                );
                i += 1;
            }
            g::GL_SetCanvasColor(1.0, 1.0, 1.0, 1.0);
        }

        // johnfitz -- new cursor handling
        if ((((g::realtime - k::key_blinktime) * g::con_cursorspeed as f64) as c_int) & 1) == 0 {
            let i = k::key_linepos - ofs;
            g::Draw_Pic(
                cbx,
                ((i + 1) << 3) as f32,
                (conheight - 16) as f32,
                if k::key_insert != 0 {
                    g::pic_ins
                } else {
                    g::pic_ovr
                },
                1.0,
                false,
            );
        }
    }
}

/// `console.c:2260`.
///
/// # Safety
/// The console must have been initialized by `Con_Init`; pointer
/// arguments must be valid for the access the C original performs.
unsafe fn con_draw_selection_highlight(cbx: *mut c_void, x: c_int, y: c_int, line: c_int) {
    // SAFETY: mirrors console.c:2260; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let (mut selbegin, mut selend) = (ZERO_OFS, ZERO_OFS);

        if !con_get_normalized_selection(&mut selbegin, &mut selend) {
            return;
        }

        let len = con_str_len(line);
        let mut begin = ConOfs { line, col: 0 };
        let mut end = ConOfs {
            line,
            col: len as c_int,
        };

        // Highlight line ends (as in Notepad, Visual Studio etc.)
        if end.line != selend.line && end.col == len as c_int {
            end.col += 1;
        }

        // ...unless we would end up overlapping the console margin
        end.col = end.col.min(g::con_linewidth);

        if !con_intersect_ranges(&mut begin, &mut end, selbegin, selend) {
            return;
        }

        g::Draw_Fill(
            cbx,
            (x + begin.col * 8) as f32,
            y as f32,
            ((end.col - begin.col) * 8) as f32,
            8.0,
            220,
            1.0,
        );
    }
}

/// `console.c:2296` -- reached through the glue's `Con_DrawConsole`
/// (`console_glue.c:404`).
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_con_draw_console(
    cbx: *mut c_void,
    lines: c_int,
    drawinput: bool,
) {
    // SAFETY: mirrors console.c:2296; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let mut ver: [c_char; 32] = [0; 32];

        if lines <= 0 {
            return;
        }

        let conheight = c::cl_parse::vid.conheight;
        g::con_vislines = idiv(lines.wrapping_mul(conheight), g::glheight);
        g::GL_SetCanvas(cbx, CANVAS_CONSOLE);

        // draw the background
        g::Draw_ConsoleBackground(cbx);

        // draw the selection highlight
        let mut rows = (g::con_vislines + 7) / 8;
        let mut y = conheight - rows * 8;
        rows -= 2; // for input and version lines
        let sb = if g::con_backscroll != 0 { 2 } else { 0 };

        let mut i = g::con_current - rows + 1;
        while i <= g::con_current - sb {
            let mut j = i - g::con_backscroll;
            if j < 0 {
                j = 0;
            }
            con_draw_selection_highlight(cbx, 8, y, j);
            i += 1;
            y += 8;
        }

        // draw the buffer text
        y = conheight - (rows + 2) * 8;
        let mut i = g::con_current - rows + 1;
        while i <= g::con_current - sb {
            let mut j = i - g::con_backscroll;
            if j < 0 {
                j = 0;
            }
            let text = con_get_line(j);
            let mut ofs = ConOfs { line: j, col: 0 };

            for x in 0..g::con_linewidth {
                let mut ch = *text.offset(x as isize);
                ofs.col = x;
                if !CON_HOTLINK.is_null()
                    && con_ofs_in_range(ofs, (*CON_HOTLINK).begin, (*CON_HOTLINK).end)
                {
                    if keydown(K_MOUSE1) {
                        ch &= 0x7f;
                    }
                    g::Draw_Character(
                        cbx,
                        ((x + 1) << 3) as f32,
                        (y + 2) as f32,
                        (b'_' as c_int) | ((ch as c_int) & 0x80),
                    );
                }
                g::Draw_Character(cbx, ((x + 1) << 3) as f32, y as f32, ch as c_int);
            }
            i += 1;
            y += 8;
        }

        // draw scrollback arrows
        if g::con_backscroll != 0 {
            y += 8; // blank line
            let mut x = 0;
            while x < g::con_linewidth {
                g::Draw_Character(cbx, ((x + 1) << 3) as f32, y as f32, b'^' as c_int);
                x += 4;
            }
            y += 8;
        }

        // draw the input prompt, user text, and cursor
        if drawinput {
            quake_rs_con_draw_input(cbx);
        }

        // draw version number in bottom right
        y += 8;
        g::q_snprintf(
            ver.as_mut_ptr(),
            ver.len(),
            c"%s".as_ptr(),
            g::Console_Glue_EngineNameAndVer(),
        );
        let verlen = strlen(ver.as_ptr());
        for (x, &vc) in ver.iter().enumerate().take(verlen) {
            // COMPAT (console.c:2371): `con_linewidth - strlen (ver)` is done in
            // size_t, so a console narrower than the version string wraps around
            // to an enormous x instead of going negative.
            let px = (g::con_linewidth as usize)
                .wrapping_sub(verlen)
                .wrapping_add(x)
                .wrapping_add(2)
                << 3;
            g::Draw_Character(cbx, px as f32, y as f32, vc as c_int);
        }
    }
}

/* ------------------------------------------------------------------------
 * LOGGING (console.c:2408-2451)
 */

/// `console.c:2419` -- the tail of `LOG_Init`; the glue owns the `-condebug`
/// check, the `quakeparms_t` spelling and the `strftime` session stamp
/// (`console_glue.c:415`).
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_log_init(basedir: *const c_char, session: *const c_char) {
    // SAFETY: mirrors console.c:2419; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        let logfilename = (&raw mut LOGFILENAME).cast::<c_char>();

        /* harness runs keep the log beside the game data instead of in the user's
        pref dir (LOG_Init runs before COM_InitFilesystem, so com_gamedir is not
        available yet -- basedir is the disposable staging dir either way) */
        let pref_path = if c::harness_active {
            ptr::null_mut()
        } else {
            c::Sys_GetPrefPath(c"vkQuake".as_ptr(), c"".as_ptr())
        };
        if !pref_path.is_null() {
            g::q_snprintf(
                logfilename,
                c::MAX_OSPATH,
                c"%sqconsole.log".as_ptr(),
                pref_path,
            );
            c::Mem_Free(pref_path.cast());
        } else {
            g::q_snprintf(
                logfilename,
                c::MAX_OSPATH,
                c"%s/qconsole.log".as_ptr(),
                basedir,
            );
        }

        LOG_FILE = c::Sys_fopen(logfilename, c"w".as_ptr());
        if LOG_FILE.is_null() {
            g::Console_Glue_LogOpenFailed(logfilename);
            return;
        }
        g::Console_Glue_LogSetUnbuffered(LOG_FILE);

        g::con_debuglog = true;
        Con_DebugLog(g::va(c"LOG started on: %s \n".as_ptr(), session));
    }
}

/// `console.c:2445`.
///
/// # Safety
/// C ABI entry point. The console must have been initialized by
/// `Con_Init`, and any pointer argument must be a valid C string or
/// object of the documented type.
#[no_mangle]
pub unsafe extern "C" fn LOG_Close() {
    // SAFETY: mirrors console.c:2445; the operands are the glue-owned
    // console state (ADR-007) and the caller-supplied C pointers.
    unsafe {
        if LOG_FILE.is_null() {
            return;
        }
        g::fclose(LOG_FILE);
        LOG_FILE = ptr::null_mut();
    }
}
