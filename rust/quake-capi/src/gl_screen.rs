//! `Quake/gl_screen.c` -- the 2D overlay orchestration, ported for Rust
//! migration Phase 8 M7 (`docs/ai/plans/rust-conversion-phase-8.md`).
//!
//! What lives here: the centerprint/notify text, the FPS/speeds/clock/devstats
//! overlays, the turtle/net/pause/loading pics, the crosshair, the console
//! slide (`SCR_SetUpToDrawConsole`), `SCR_TileClear`, the refdef/fov math
//! (`SCR_CalcRefdef`), the conwidth/relative-scale cvar callbacks, the zoom
//! integrator and `SCR_LoadPics`. Each keeps its C name so
//! `Quake/gl_screen_glue.c` (and the rest of the engine) calls it unchanged.
//!
//! What stays in `gl_screen_glue.c`: the cvar and global definitions (C reads
//! `glwidth`, `scr_vrect`, `scr_con_current`, ... by name), `SCR_Init` and the
//! key-binding commands it registers, `SCR_DrawGUI` (it is a `setjmp`
//! target -- ADR-009 forbids a `longjmp` through a Rust frame), the loading
//! plaque and `SCR_ModalMessage` (both re-enter `SCR_UpdateScreen`), and
//! `SCR_UpdateScreen` itself with its task graph, which is Phase 8 M9.
//!
//! Arithmetic follows the C expression by expression: the fov helpers mix
//! `float` and `double` exactly where `gl_screen.c` does, and every
//! float-to-int store truncates like a C assignment.

use core::ffi::{c_char, c_float, c_int};
use core::ptr;

use quake_c_sys as c;
use quake_types::host::{ClientState, ClientStatic};
use quake_types::render::{CbContext, RefDef, VRect};
use quake_types::wad::QPic;

use crate::gl_draw::{
    Draw_CachePic, Draw_Character, Draw_Fill, Draw_Pic, Draw_PicFromWad, Draw_String,
    Draw_TileClear, GL_SetCanvas, CANVAS_BOTTOMLEFT, CANVAS_BOTTOMRIGHT, CANVAS_CROSSHAIR,
    CANVAS_DEFAULT, CANVAS_MENU, CANVAS_TOPRIGHT,
};

/// `draw.h` -- `CHARACTER_SIZE`.
const CHARACTER_SIZE: c_int = 8;
/// `quakedef.h:124-130` -- the `STAT_*` slots read here.
const STAT_TOTALSECRETS: usize = 11;
const STAT_TOTALMONSTERS: usize = 12;
const STAT_SECRETS: usize = 13;
const STAT_MONSTERS: usize = 14;
const STAT_VIEWZOOM: usize = 21;
/// `keys.h:126-133` -- `keydest_t`.
const KEY_GAME: c_int = 0;
const KEY_CONSOLE: c_int = 1;
const KEY_MESSAGE: c_int = 2;

const M_PI: f64 = core::f64::consts::PI;

extern "C" {
    /// `client.h` -- `client_state_t cl` / `client_static_t cls`.
    static mut cl: ClientState;
    static mut cls: ClientStatic;
    /// `gl_rmain.c:58` -- `refdef_t r_refdef`.
    static mut r_refdef: RefDef;
    /// `gl_rmain.c:42-43` -- the `scr_speeds` overlay lines.
    static mut rs_display_lines: [[c_char; 40]; 3];
    static mut rs_display_numlines: c_int;

    /// `Quake/gl_screen_glue.c` -- the globals `gl_screen.c` defined.
    static mut scr_conlines: c_float;
    static mut scr_vrect: VRect;
    static mut scr_net: *mut QPic;
    static mut scr_turtle: *mut QPic;
    static mut scr_drawloading: c::qboolean;
    static mut clearconsole: c_int;
    static mut scr_notifystring: *const c_char;

    /// `Quake/gl_screen_glue.c` -- the cvars `gl_screen.c` defined.
    static mut scr_menuscale: c::cvar_t;
    static mut scr_sbarscale: c::cvar_t;
    static mut scr_sbaralpha: c::cvar_t;
    static mut scr_conwidth: c::cvar_t;
    static mut scr_conscale: c::cvar_t;
    static mut scr_crosshairscale: c::cvar_t;
    static mut scr_showfps: c::cvar_t;
    static mut scr_clock: c::cvar_t;
    static mut scr_autoclock: c::cvar_t;
    static mut scr_usekfont: c::cvar_t;
    static mut scr_style: c::cvar_t;
    static mut scr_fov_adapt: c::cvar_t;
    static mut scr_zoomfov: c::cvar_t;
    static mut scr_zoomspeed: c::cvar_t;
    static mut scr_conspeed: c::cvar_t;
    static mut scr_conanim: c::cvar_t;
    static mut scr_centertime: c::cvar_t;
    static mut scr_showturtle: c::cvar_t;
    static mut scr_showpause: c::cvar_t;
    static mut scr_printspeed: c::cvar_t;
    static mut scr_relativescale: c::cvar_t;
    static mut scr_relmenuscale: c::cvar_t;
    static mut scr_relsbarscale: c::cvar_t;
    static mut scr_relcrosshairscale: c::cvar_t;
    static mut scr_relconscale: c::cvar_t;
    /// `gl_rmain.c` -- `cvar_t scr_speeds`.
    static mut scr_speeds: c::cvar_t;

    /// `common.h:280` -- `void COM_WordWrap (char *dst, const char *src,
    /// size_t dstsize, int maxcols)`.
    fn COM_WordWrap(dst: *mut c_char, src: *const c_char, dstsize: usize, maxcols: c_int);
    /// `console.h:41,49` -- `Con_DrawConsole` / `Con_DrawNotify` (both keep
    /// plain C in `console_glue.c`).
    fn Con_DrawConsole(cbx: *mut CbContext, lines: c_int, drawinput: c::qboolean);
    fn Con_DrawNotify(cbx: *mut CbContext);
    /// `console.h` -- `void Con_CheckResize (void)`.
    fn Con_CheckResize();
    /// `Quake/gl_screen_glue.c` -- `M_GetCrosshairDef` through an out-pointer
    /// (its by-value `crosshair_t` return has no portable Rust spelling).
    fn SCR_Glue_GetCrosshairDef(crosshair_def_value: c_float, out: *mut c::menu::crosshair_t);
}

// ---------------------------------------------------------------------------
// gl_screen.c:151-157 -- centerprint state (Rust-owned; `cl_demo.c` writes
// `scr_clock_off` by name).

#[no_mangle]
pub static mut scr_centerstring: [c_char; 1024] = [0; 1024];
#[no_mangle]
pub static mut scr_centertime_start: c_float = 0.0; // for slow victory printing
#[no_mangle]
pub static mut scr_centertime_off: c_float = 0.0;
#[no_mangle]
pub static mut scr_clock_off: c_float = 0.0;
#[no_mangle]
pub static mut scr_center_lines: c_int = 0;
#[no_mangle]
pub static mut scr_erase_lines: c_int = 0;
#[no_mangle]
pub static mut scr_erase_center: c_int = 0;

/// C `int` truncation of a float (`as` saturates where C is undefined).
#[inline]
fn as_i(f: f32) -> c_int {
    f as c_int
}

/// `strlen` of a NUL-terminated buffer.
///
/// # Safety
/// `s` must be NUL-terminated.
unsafe fn strlen(s: *const c_char) -> usize {
    // SAFETY: per the contract.
    unsafe { core::ffi::CStr::from_ptr(s) }.to_bytes().len()
}

/// `gl_screen.c:159` -- `void SCR_CenterPrintClear (void)`.
///
/// # Safety
/// Single-threaded engine state.
#[no_mangle]
pub unsafe extern "C" fn SCR_CenterPrintClear() {
    // SAFETY: per the contract.
    unsafe {
        scr_centertime_off = 0.0;
        scr_clock_off = 0.0;
    }
}

/// `gl_screen.c:173` -- `void SCR_CenterPrint (const char *str)`.
///
/// Called for important messages that should stay in the center of the
/// screen for a few moments.
///
/// # Safety
/// `str` must be NUL-terminated; single-threaded engine state.
#[no_mangle]
pub unsafe extern "C" fn SCR_CenterPrint(str: *const c_char) {
    // SAFETY: per the contract.
    unsafe {
        let dst = ptr::addr_of_mut!(scr_centerstring).cast::<c_char>();
        COM_WordWrap(
            dst,
            str,
            1024,
            if scr_usekfont.value != 0.0 { 40 } else { 0 },
        );
        scr_centertime_off = (cl.time + f64::from(scr_centertime.value)) as c_float;
        scr_centertime_start = cl.time as c_float;

        // count the number of lines for centering
        scr_center_lines = 1;
        let mut p = dst.cast_const();
        while *p != 0 {
            if *p == b'\n' as c_char {
                scr_center_lines += 1;
            }
            p = p.add(1);
        }
    }
}

/// `gl_screen.c:191` -- `static void SCR_DrawCenterString (cb_context_t *cbx)`
/// (actually do the drawing).
///
/// # Safety
/// `cbx` must be a live recording context.
unsafe fn scr_draw_center_string(cbx: *mut CbContext) {
    // SAFETY: per the contract.
    unsafe {
        GL_SetCanvas(cbx, CANVAS_MENU); // johnfitz

        // the finale prints the characters one at a time
        let mut remaining: c_int = if cl.intermission != 0 {
            (scr_printspeed.value as f64 * (cl.time - f64::from(scr_centertime_start))) as c_int
        } else {
            9999
        };

        scr_erase_center = 0;
        let mut start = ptr::addr_of!(scr_centerstring).cast::<c_char>();

        let mut y: c_int = if scr_center_lines <= 4 {
            (200.0 * 0.35) as c_int // johnfitz -- 320x200 coordinate system
        } else {
            48
        };
        if c::view::crosshair.value != 0.0 {
            y -= CHARACTER_SIZE;
        }

        loop {
            // scan the width of the line
            let mut l = 0usize;
            while *start.add(l) != 0 {
                if *start.add(l) == b'\n' as c_char {
                    break;
                }
                l += 1;
            }
            let mut x = (320 - l as c_int * CHARACTER_SIZE) / 2; // johnfitz -- 320x200 coordinate system
            for j in 0..l {
                Draw_Character(cbx, x as f32, y as f32, c_int::from(*start.add(j))); // johnfitz -- stretch overlays
                let was = remaining;
                remaining -= 1;
                if was == 0 {
                    return;
                }
                x += CHARACTER_SIZE;
            }

            y += CHARACTER_SIZE;
            start = start.add(l);

            if *start == 0 {
                break;
            }
            start = start.add(1); // skip the \n
        }
    }
}

/// `gl_screen.c:240` -- `static void SCR_CheckDrawCenterString (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live recording context.
#[no_mangle]
pub unsafe extern "C" fn SCR_CheckDrawCenterString(cbx: *mut CbContext) {
    // SAFETY: per the contract.
    unsafe {
        if scr_center_lines > scr_erase_lines {
            scr_erase_lines = scr_center_lines;
        }

        if f64::from(scr_centertime_off) <= cl.time && cl.intermission == 0 {
            return;
        }
        if c::cl_demo::key_dest != KEY_GAME {
            return;
        }
        if cl.paused && (!cls.demoplayback || cls.demospeed > 0.0) {
            // johnfitz -- don't show centerprint during a pause
            return;
        }

        scr_draw_center_string(cbx);
    }
}

//=============================================================================

/// `gl_screen.c:295` -- `void SCR_UpdateZoom (void)`.
///
/// # Safety
/// Single-threaded engine state.
#[no_mangle]
pub unsafe extern "C" fn SCR_UpdateZoom() {
    // SAFETY: per the contract.
    unsafe {
        let delta = (f64::from(cl.zoomdir * scr_zoomspeed.value) * (cl.time - cl.oldtime)) as f32;
        if delta == 0.0 {
            return;
        }
        cl.zoom += delta;
        if cl.zoom >= 1.0 {
            cl.zoom = 1.0;
            cl.zoomdir = 0.0;
        } else if cl.zoom <= 0.0 {
            cl.zoom = 0.0;
            cl.zoomdir = 0.0;
        }
        c::cl_parse::vid.recalc_refdef = 1;
    }
}

/// `gl_screen.c:321` -- `static float AdaptFovx (float fov_x, float width,
/// float height)`. Adapt a 4:3 horizontal FOV to the current screen size
/// using the "Hor+" scaling: `2.0 * atan(width / height * 3.0 / 4.0 *
/// tan(fov_x / 2.0))`.
///
/// # Safety
/// Single-threaded engine state.
unsafe fn adapt_fovx(mut fov_x: f32, width: f32, height: f32) -> f32 {
    // SAFETY: per the contract.
    unsafe {
        if !(1.0..=179.0).contains(&fov_x) {
            c::Sys_Error(c"Bad fov: %f".as_ptr(), f64::from(fov_x));
        }
        if cl.statsf[STAT_VIEWZOOM] != 0.0 {
            fov_x = (f64::from(fov_x) * (f64::from(cl.statsf[STAT_VIEWZOOM]) / 255.0)) as f32;
            fov_x = fov_x.clamp(1.0, 179.0);
        }

        if scr_fov_adapt.value == 0.0 {
            return fov_x;
        }
        let x = height / width;
        if f64::from(x) == 0.75 {
            return fov_x;
        }
        let a = (0.75 / f64::from(x) * (f64::from(fov_x / 360.0) * M_PI).tan()).atan() as f32;
        (f64::from(a * 360.0) / M_PI) as f32
    }
}

/// `gl_screen.c:347` -- `static float CalcFovy (float fov_x, float width,
/// float height)`.
///
/// # Safety
/// Single-threaded engine state.
unsafe fn calc_fovy(fov_x: f32, width: f32, height: f32) -> f32 {
    if !(1.0..=179.0).contains(&fov_x) {
        // SAFETY: per the contract.
        unsafe { c::Sys_Error(c"Bad fov: %f".as_ptr(), f64::from(fov_x)) };
    }

    let x = (f64::from(width) / (f64::from(fov_x / 360.0) * M_PI).tan()) as f32;
    let a = f64::from(height / x).atan() as f32;
    (f64::from(a * 360.0) / M_PI) as f32
}

/// `gl_screen.c:368` -- `static void SCR_CalcRefdef (void)`. Must be called
/// whenever vid changes. Internal use only.
///
/// # Safety
/// Single-threaded engine state.
#[no_mangle]
pub unsafe extern "C" fn SCR_CalcRefdef() {
    // SAFETY: per the contract.
    unsafe {
        let scr_viewsize = ptr::addr_of_mut!(c::console::scr_viewsize);
        let scr_fov = ptr::addr_of_mut!(c::menu::scr_fov);
        let scr_zoomfov_p = ptr::addr_of_mut!(scr_zoomfov);

        // bound viewsize
        if (*scr_viewsize).value < 30.0 {
            c::Cvar_SetQuick(scr_viewsize, c"30".as_ptr());
        }
        if (*scr_viewsize).value > 130.0 {
            c::Cvar_SetQuick(scr_viewsize, c"130".as_ptr());
        }

        // bound fov
        if (*scr_fov).value < 10.0 {
            c::Cvar_SetQuick(scr_fov, c"10".as_ptr());
        }
        if (*scr_fov).value > 170.0 {
            c::Cvar_SetQuick(scr_fov, c"170".as_ptr());
        }
        if (*scr_zoomfov_p).value < 10.0 {
            c::Cvar_SetQuick(scr_zoomfov_p, c"10".as_ptr());
        }
        if (*scr_zoomfov_p).value > 170.0 {
            c::Cvar_SetQuick(scr_zoomfov_p, c"170".as_ptr());
        }

        c::cl_parse::vid.recalc_refdef = 0;

        let glwidth = c::console::glwidth;
        let glheight = c::console::glheight;

        // johnfitz -- rewrote this section
        let mut size = (*scr_viewsize).value;
        let max = f64::from(glwidth as f32) / 320.0;
        let sv = f64::from(scr_sbarscale.value);
        let scale: f32 = (if sv < 1.0 {
            1.0
        } else if sv > max {
            max
        } else {
            sv
        }) as f32;

        let sb_lines = ptr::addr_of_mut!(c::sbar::sb_lines);
        if size >= 120.0
            || cl.intermission != 0
            || scr_sbaralpha.value < 1.0
            || (scr_style.value < 1.0 && cl.qcvm.extfuncs.csqc_draw_hud != 0)
            || scr_style.value >= 2.0
        {
            // johnfitz -- scr_sbaralpha.value. Spike -- simple csqc assumes fullscreen video the same way.
            *sb_lines = 0;
        } else if size >= 110.0 {
            *sb_lines = as_i(24.0 * scale);
        } else {
            *sb_lines = as_i(48.0 * scale);
        }
        let sb_lines = *sb_lines;

        size = (*scr_viewsize).value.min(100.0) / 100.0;
        // johnfitz

        // johnfitz -- rewrote this section
        let w = glwidth as f32 * size;
        r_refdef.vrect.width = as_i(if w > 96.0 { w } else { 96.0 }); // no smaller than 96, for icons
        let h = glheight as f32 * size;
        let room = (glheight - sb_lines) as f32;
        r_refdef.vrect.height = as_i(if h < room { h } else { room }); // make room for sbar
        r_refdef.vrect.x = (glwidth - r_refdef.vrect.width) / 2;
        r_refdef.vrect.y = (glheight - sb_lines - r_refdef.vrect.height) / 2;
        // johnfitz

        let z = cl.zoom;
        let zoom = z * (z * (3.0 - 2.0 * z)); // smoothstep
        r_refdef.basefov = (*scr_fov).value + ((*scr_zoomfov_p).value - (*scr_fov).value) * zoom;
        r_refdef.fov_x = adapt_fovx(
            r_refdef.basefov,
            c::cl_parse::vid.width as f32,
            c::cl_parse::vid.height as f32,
        );
        r_refdef.fov_y = calc_fovy(
            r_refdef.fov_x,
            r_refdef.vrect.width as f32,
            r_refdef.vrect.height as f32,
        );

        scr_vrect = r_refdef.vrect;
    }
}

/// `gl_screen.c:459` -- `static void SCR_Conwidth_f (cvar_t *var)`
/// (johnfitz -- called when scr_conwidth or scr_conscale changes).
///
/// # Safety
/// Single-threaded engine state; `var` is unused.
#[no_mangle]
pub unsafe extern "C" fn SCR_Conwidth_f(_var: *mut c::cvar_t) {
    // SAFETY: per the contract.
    unsafe {
        let vid = ptr::addr_of_mut!(c::cl_parse::vid);
        (*vid).recalc_refdef = 1;
        (*vid).conwidth = if scr_conwidth.value > 0.0 {
            as_i(scr_conwidth.value)
        } else if scr_conscale.value > 0.0 {
            as_i((*vid).width as f32 / scr_conscale.value)
        } else {
            (*vid).width
        };
        (*vid).conwidth = (*vid).conwidth.clamp(320, (*vid).width);
        (*vid).conwidth &= 0xFFFFFFF8_u32 as c_int;
        (*vid).conheight = (*vid).conwidth * (*vid).height / (*vid).width;
    }
}

/// `gl_screen.c:473` -- `void SCR_UpdateRelativeScale ()`.
///
/// # Safety
/// Single-threaded engine state.
#[no_mangle]
pub unsafe extern "C" fn SCR_UpdateRelativeScale() {
    const ARCHIVE: c::cvarflags_t = c::cvarflags_t_CVAR_ARCHIVE;
    const ROM: c::cvarflags_t = c::cvarflags_t_CVAR_ROM;
    // SAFETY: per the contract.
    unsafe {
        let vid_height = c::cl_parse::vid.height as f32;
        let targets: [(*mut c::cvar_t, &core::ffi::CStr, f32); 4] = [
            (
                ptr::addr_of_mut!(scr_menuscale),
                c"scr_menuscale",
                scr_relmenuscale.value,
            ),
            (
                ptr::addr_of_mut!(scr_sbarscale),
                c"scr_sbarscale",
                scr_relsbarscale.value,
            ),
            (
                ptr::addr_of_mut!(scr_crosshairscale),
                c"scr_crosshairscale",
                scr_relcrosshairscale.value,
            ),
            (
                ptr::addr_of_mut!(scr_conscale),
                c"scr_conscale",
                scr_relconscale.value,
            ),
        ];
        if scr_relativescale.value != 0.0 {
            let normalization_scale = 0.0013_f32; // To make scr_relmenuscale etc. more user friendly
            let relative_scale = scr_relativescale.value.clamp(1.0, 3.0) * normalization_scale;

            for (var, name, rel) in targets {
                (*var).flags &= !(ARCHIVE | ROM);
                c::Cvar_SetValue(name.as_ptr(), vid_height * rel * relative_scale);
                (*var).flags |= ROM;
            }
        } else {
            for (var, _, _) in targets {
                (*var).flags |= ARCHIVE;
                (*var).flags &= !ROM;
            }
        }
        SCR_Conwidth_f(ptr::null_mut());
    }
}

//============================================================================

/// `gl_screen.c:527` -- `void SCR_LoadPics (void)` (johnfitz).
///
/// # Safety
/// The draw layer must be initialized.
#[no_mangle]
pub unsafe extern "C" fn SCR_LoadPics() {
    // SAFETY: per the contract.
    unsafe {
        scr_net = Draw_PicFromWad(c"net".as_ptr());
        scr_turtle = Draw_PicFromWad(c"turtle".as_ptr());
    }
}

//============================================================================

/// `gl_screen.c:618-620` -- `SCR_DrawFPS`'s function-local statics.
static mut FPS_OLDTIME: f64 = 0.0;
static mut FPS_LASTFPS: f64 = 0.0;
static mut FPS_OLDFRAMECOUNT: c_int = 0;

/// `gl_screen.c:616` -- `static void SCR_DrawFPS (cb_context_t *cbx)` (johnfitz).
///
/// # Safety
/// `cbx` must be a live recording context.
#[no_mangle]
pub unsafe extern "C" fn SCR_DrawFPS(cbx: *mut CbContext) {
    // SAFETY: per the contract.
    unsafe {
        let realtime = c::console::realtime;
        let host_framecount = c::host_framecount;
        let elapsed_time = realtime - FPS_OLDTIME;
        let frames = host_framecount - FPS_OLDFRAMECOUNT;

        if elapsed_time < 0.0 || frames < 0 {
            FPS_OLDTIME = realtime;
            FPS_OLDFRAMECOUNT = host_framecount;
            return;
        }
        // update value every 3/4 second
        if elapsed_time > 0.75 {
            FPS_LASTFPS = f64::from(frames) / elapsed_time;
            FPS_OLDTIME = realtime;
            FPS_OLDFRAMECOUNT = host_framecount;
        }

        if scr_showfps.value != 0.0 && c::console::scr_viewsize.value < 130.0 {
            let mut st = [0 as c_char; 16];
            c::cl_main::q_snprintf(
                st.as_mut_ptr(),
                st.len(),
                c"%4.0f fps".as_ptr(),
                FPS_LASTFPS,
            );
            let x = 320 - ((strlen(st.as_ptr()) as c_int) << 3);
            let y = 200 - CHARACTER_SIZE;
            GL_SetCanvas(cbx, CANVAS_BOTTOMRIGHT);
            Draw_String(cbx, x as f32, y as f32, st.as_ptr());
        }
    }
}

/// `gl_screen.c:658` -- `static void SCR_DrawSpeeds (cb_context_t *cbx)`
/// (scr_speeds overlay in the top right corner).
///
/// # Safety
/// `cbx` must be a live recording context.
#[no_mangle]
pub unsafe extern "C" fn SCR_DrawSpeeds(cbx: *mut CbContext) {
    // SAFETY: per the contract.
    unsafe {
        if scr_speeds.value == 0.0
            || rs_display_numlines == 0
            || c::console::scr_viewsize.value >= 130.0
        {
            return;
        }

        GL_SetCanvas(cbx, CANVAS_TOPRIGHT);
        let mut y = 0;
        let lines = ptr::addr_of!(rs_display_lines);
        for i in 0..rs_display_numlines.clamp(0, 3) as usize {
            let line = (*lines)[i].as_ptr();
            Draw_String(
                cbx,
                (320 - ((strlen(line) as c_int) << 3)) as f32,
                y as f32,
                line,
            );
            y += CHARACTER_SIZE;
        }
    }
}

/// `gl_screen.c:681` -- `SCR_DrawClock`'s `static qboolean shown_pause`.
static mut CLOCK_SHOWN_PAUSE: bool = false;

/// `gl_screen.c:677` -- `static void SCR_DrawClock (cb_context_t *cbx)` (johnfitz).
///
/// # Safety
/// `cbx` must be a live recording context.
#[no_mangle]
pub unsafe extern "C" fn SCR_DrawClock(cbx: *mut CbContext) {
    // SAFETY: per the contract.
    unsafe {
        let mut str = [0 as c_char; 32];
        let mut y = 200 - CHARACTER_SIZE;

        if cls.demoplayback && cls.demospeed != 1.0 && cls.demospeed != 0.0 {
            // always show if playback speed is modified
            scr_clock_off = 2.0;
            CLOCK_SHOWN_PAUSE = false;
        }

        if cls.demoplayback && cls.demospeed == 0.0 && !CLOCK_SHOWN_PAUSE {
            // show for a bit if paused
            scr_clock_off = 1.5;
            CLOCK_SHOWN_PAUSE = true;
        }

        if (scr_clock.value == 0.0
            && scr_clock_off <= 0.0
            && !(c::sbar::sb_showscores && scr_autoclock.value != 0.0))
            || c::console::scr_viewsize.value >= 130.0
        {
            return;
        }

        let speed = if cls.demospeed != 0.0 {
            cls.demospeed
        } else {
            1.0
        };
        scr_clock_off = (f64::from(scr_clock_off) - c::host_frametime / f64::from(speed)) as f32;

        GL_SetCanvas(cbx, CANVAS_BOTTOMRIGHT);

        if scr_showfps.value != 0.0 {
            y -= CHARACTER_SIZE; // make room for fps counter
        }

        let draw = |s: &[c_char; 32], y: c_int| {
            Draw_String(
                cbx,
                (320 - ((strlen(s.as_ptr()) as c_int) << 3)) as f32,
                y as f32,
                s.as_ptr(),
            );
        };

        if scr_clock.value >= 2.0 {
            c::cl_main::q_snprintf(
                str.as_mut_ptr(),
                str.len(),
                c"%i/%i".as_ptr(),
                cl.stats[STAT_MONSTERS],
                cl.stats[STAT_TOTALMONSTERS],
            );
            draw(&str, y);
            y -= CHARACTER_SIZE;
            c::cl_main::q_snprintf(
                str.as_mut_ptr(),
                str.len(),
                c"%i/%i".as_ptr(),
                cl.stats[STAT_SECRETS],
                cl.stats[STAT_TOTALSECRETS],
            );
            draw(&str, y);
            y -= CHARACTER_SIZE;
        }

        let t = cl.time as c_int;
        c::cl_main::q_snprintf(
            str.as_mut_ptr(),
            str.len(),
            c"%i:%02i".as_ptr(),
            t / 60,
            t % 60,
        );
        draw(&str, y);

        // show playback rate
        if cls.demoplayback && cls.demospeed != 1.0 {
            y -= CHARACTER_SIZE;
            // `%g` is formatted by the C `q_snprintf` (ADR-005 leaves `%g` to C).
            c::cl_main::q_snprintf(
                str.as_mut_ptr(),
                str.len(),
                c"[%gx]".as_ptr(),
                f64::from(cls.demospeed),
            );
            if cls.demospeed == 0.0 {
                c::cl_main::q_snprintf(str.as_mut_ptr(), str.len(), c"[paused]".as_ptr());
            }
            draw(&str, y);
        }
    }
}

/// `gl_screen.c:735` -- `static void SCR_DrawDevStats (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live recording context.
#[no_mangle]
pub unsafe extern "C" fn SCR_DrawDevStats(cbx: *mut CbContext) {
    // SAFETY: per the contract.
    unsafe {
        let mut str = [0 as c_char; 40];
        let mut y: c_int = 25 - 9; // 9=number of lines to print
        let x: c_int = 0; // margin

        if c::host::devstats.value == 0.0 {
            return;
        }

        GL_SetCanvas(cbx, CANVAS_BOTTOMLEFT);

        Draw_Fill(
            cbx,
            x as f32,
            (y * CHARACTER_SIZE) as f32,
            (19 * CHARACTER_SIZE) as f32,
            (9 * CHARACTER_SIZE) as f32,
            0,
            0.5,
        ); // dark rectangle

        let mut line = |s: &[c_char; 40]| {
            Draw_String(cbx, x as f32, (y * CHARACTER_SIZE - x) as f32, s.as_ptr());
            y += 1;
        };

        c::cl_main::q_snprintf(str.as_mut_ptr(), str.len(), c"devstats |Curr Peak".as_ptr());
        line(&str);

        c::cl_main::q_snprintf(str.as_mut_ptr(), str.len(), c"---------+---------".as_ptr());
        line(&str);

        let ds = ptr::addr_of!(c::cl_parse::dev_stats);
        let pk = ptr::addr_of!(c::cl_parse::dev_peakstats);
        let rows: [(&core::ffi::CStr, c_int, c_int); 7] = [
            (c"Edicts   |%4i %4i", (*ds).edicts, (*pk).edicts),
            (c"Packet   |%4i %4i", (*ds).packetsize, (*pk).packetsize),
            (c"Visedicts|%4i %4i", (*ds).visedicts, (*pk).visedicts),
            (c"Efrags   |%4i %4i", (*ds).efrags, (*pk).efrags),
            (c"Dlights  |%4i %4i", (*ds).dlights, (*pk).dlights),
            (c"Beams    |%4i %4i", (*ds).beams, (*pk).beams),
            (c"Tempents |%4i %4i", (*ds).tempents, (*pk).tempents),
        ];
        for (fmt, cur, peak) in rows {
            c::cl_main::q_snprintf(str.as_mut_ptr(), str.len(), fmt.as_ptr(), cur, peak);
            line(&str);
        }
    }
}

/// `gl_screen.c:783` -- `SCR_DrawTurtle`'s `static int count`.
static mut TURTLE_COUNT: c_int = 0;

/// `gl_screen.c:781` -- `static void SCR_DrawTurtle (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live recording context.
#[no_mangle]
pub unsafe extern "C" fn SCR_DrawTurtle(cbx: *mut CbContext) {
    // SAFETY: per the contract.
    unsafe {
        if scr_showturtle.value == 0.0 {
            return;
        }

        if c::host_frametime < 0.1 {
            TURTLE_COUNT = 0;
            return;
        }

        TURTLE_COUNT += 1;
        if TURTLE_COUNT < 3 {
            return;
        }

        GL_SetCanvas(cbx, CANVAS_DEFAULT); // johnfitz

        Draw_Pic(
            cbx,
            scr_vrect.x as f32,
            scr_vrect.y as f32,
            scr_turtle,
            1.0,
            false,
        );
    }
}

/// `gl_screen.c:808` -- `static void SCR_DrawNet (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live recording context.
#[no_mangle]
pub unsafe extern "C" fn SCR_DrawNet(cbx: *mut CbContext) {
    // SAFETY: per the contract.
    unsafe {
        if c::console::realtime - f64::from(cl.last_received_message) < 0.3 {
            return;
        }
        if cls.demoplayback {
            return;
        }

        GL_SetCanvas(cbx, CANVAS_DEFAULT); // johnfitz

        Draw_Pic(
            cbx,
            (scr_vrect.x + 64) as f32,
            scr_vrect.y as f32,
            scr_net,
            1.0,
            false,
        );
    }
}

/// `gl_screen.c:825` -- `static void SCR_DrawPause (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live recording context.
#[no_mangle]
pub unsafe extern "C" fn SCR_DrawPause(cbx: *mut CbContext) {
    // SAFETY: per the contract.
    unsafe {
        if !cl.paused {
            return;
        }

        if scr_showpause.value == 0.0 || c::console::scr_viewsize.value >= 130.0 {
            // turn off for screenshots
            return;
        }

        if cls.demoplayback && cls.demospeed == 0.0 {
            return;
        }

        GL_SetCanvas(cbx, CANVAS_MENU); // johnfitz

        let pic = Draw_CachePic(c"gfx/pause.lmp".as_ptr());
        Draw_Pic(
            cbx,
            ((320 - (*pic).width) / 2) as f32,
            ((240 - 48 - (*pic).height) / 2) as f32,
            pic,
            1.0,
            false,
        ); // johnfitz -- stretched menus
    }
}

/// `gl_screen.c:849` -- `static void SCR_DrawLoading (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live recording context.
#[no_mangle]
pub unsafe extern "C" fn SCR_DrawLoading(cbx: *mut CbContext) {
    // SAFETY: per the contract.
    unsafe {
        if !scr_drawloading {
            return;
        }

        GL_SetCanvas(cbx, CANVAS_MENU); // johnfitz

        let pic = Draw_CachePic(c"gfx/loading.lmp".as_ptr());
        Draw_Pic(
            cbx,
            ((320 - (*pic).width) / 2) as f32,
            ((240 - 48 - (*pic).height) / 2) as f32,
            pic,
            1.0,
            false,
        ); // johnfitz -- stretched menus
    }
}

/// `gl_screen.c:867` -- `static void SCR_DrawCrosshair (cb_context_t *cbx)` (johnfitz).
///
/// # Safety
/// `cbx` must be a live recording context.
#[no_mangle]
pub unsafe extern "C" fn SCR_DrawCrosshair(cbx: *mut CbContext) {
    // SAFETY: per the contract.
    unsafe {
        if c::view::crosshair.value == 0.0 || c::console::scr_viewsize.value >= 130.0 {
            return;
        }

        GL_SetCanvas(cbx, CANVAS_CROSSHAIR);

        if c::view::crosshair.value != 0.0 {
            let mut current = core::mem::MaybeUninit::<c::menu::crosshair_t>::uninit();
            SCR_Glue_GetCrosshairDef(c::view::crosshair_def.value, current.as_mut_ptr());
            let current = current.assume_init();
            Draw_Character(
                cbx,
                current.viewport_x_offset,
                current.viewport_y_offset,
                c_int::from(current.crosshair_char),
            ); // 0,0 is center of viewport
        }
    }
}

//=============================================================================

/// `gl_screen.c:888` -- `static void SCR_SetUpToDrawConsole (void)`.
///
/// # Safety
/// Single-threaded engine state.
#[no_mangle]
pub unsafe extern "C" fn SCR_SetUpToDrawConsole() {
    // SAFETY: per the contract.
    unsafe {
        // johnfitz -- let's hack away the problem of slow console when host_timescale is <0
        Con_CheckResize();

        if scr_drawloading {
            return; // never a console with loading plaque
        }

        let glheight = c::console::glheight;
        let scr_con_current = ptr::addr_of_mut!(c::console::scr_con_current);
        if c::console::con_forcedup {
            scr_conlines = glheight as f32; // full screen //johnfitz -- glheight instead of vid.height
            *scr_con_current = scr_conlines;
        } else if c::cl_demo::key_dest == KEY_CONSOLE {
            scr_conlines = (glheight / 2) as f32; // half screen //johnfitz -- glheight instead of vid.height
        } else {
            scr_conlines = 0.0; // none visible
        }

        let ts = c::host::host_timescale.value;
        let timescale: f32 = if ts > 0.0 { ts } else { 1.0 }; // johnfitz -- timescale

        if scr_conanim.value != 0.0 {
            // ericw -- (glheight/600.0) factor makes conspeed resolution independent, using 800x600 as a baseline
            let step =
                f64::from(scr_conspeed.value) * (f64::from(glheight) / 600.0) * c::host_frametime
                    / f64::from(timescale); // johnfitz -- timescale
            if scr_conlines < *scr_con_current {
                *scr_con_current = (f64::from(*scr_con_current) - step) as f32;
                if scr_conlines > *scr_con_current {
                    *scr_con_current = scr_conlines;
                }
            } else if scr_conlines > *scr_con_current {
                *scr_con_current = (f64::from(*scr_con_current) + step) as f32;
                if scr_conlines < *scr_con_current {
                    *scr_con_current = scr_conlines;
                }
            }
        } else if scr_conlines != *scr_con_current {
            *scr_con_current = scr_conlines;
        }
    }
}

/// `gl_screen.c:943` -- `static void SCR_DrawConsole (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live recording context.
#[no_mangle]
pub unsafe extern "C" fn SCR_DrawConsole(cbx: *mut CbContext) {
    // SAFETY: per the contract.
    unsafe {
        if c::console::scr_con_current != 0.0 {
            Con_DrawConsole(cbx, as_i(c::console::scr_con_current), true);
            clearconsole = 0;
        } else if c::cl_demo::key_dest == KEY_GAME || c::cl_demo::key_dest == KEY_MESSAGE {
            Con_DrawNotify(cbx); // only draw notify in game
        }
    }
}

//=============================================================================

/// `gl_screen.c:1004` -- `static void SCR_DrawNotifyString (cb_context_t *cbx)`.
///
/// # Safety
/// `cbx` must be a live recording context; `scr_notifystring` must be set.
#[no_mangle]
pub unsafe extern "C" fn SCR_DrawNotifyString(cbx: *mut CbContext) {
    // SAFETY: per the contract.
    unsafe {
        GL_SetCanvas(cbx, CANVAS_MENU); // johnfitz

        let mut start = scr_notifystring;

        let mut y = (200.0 * 0.35) as c_int; // johnfitz -- stretched overlays

        loop {
            // scan the width of the line
            let mut l = 0usize;
            while *start.add(l) != 0 {
                if *start.add(l) == b'\n' as c_char {
                    break;
                }
                l += 1;
            }
            let mut x = (320 - l as c_int * CHARACTER_SIZE) / 2; // johnfitz -- 320x200 coordinate system
            for j in 0..l {
                Draw_Character(cbx, x as f32, y as f32, c_int::from(*start.add(j)));
                x += CHARACTER_SIZE;
            }

            y += CHARACTER_SIZE;
            start = start.add(l);

            if *start == 0 {
                break;
            }
            start = start.add(1); // skip the \n
        }
    }
}

//=============================================================================

/// `gl_screen.c:1097` -- `void SCR_TileClear (cb_context_t *cbx)` (johnfitz --
/// modified to use glwidth/glheight instead of vid.width/vid.height; also
/// fixed the dimentions of right and top panels).
///
/// # Safety
/// `cbx` must be a live recording context.
#[no_mangle]
pub unsafe extern "C" fn SCR_TileClear(cbx: *mut CbContext) {
    // SAFETY: per the contract.
    unsafe {
        let v = ptr::addr_of!(r_refdef.vrect);
        let (vx, vy, vw, vh) = ((*v).x, (*v).y, (*v).width, (*v).height);
        let glwidth = c::console::glwidth;
        let glheight = c::console::glheight;
        let sb_lines = c::sbar::sb_lines;

        if vx > 0 || vy > 0 {
            GL_SetCanvas(cbx, CANVAS_DEFAULT);
        }

        if vx > 0 {
            // left
            Draw_TileClear(cbx, 0.0, 0.0, vx as f32, (glheight - sb_lines) as f32);
            // right
            Draw_TileClear(
                cbx,
                (vx + vw) as f32,
                0.0,
                (glwidth - vx - vw) as f32,
                (glheight - sb_lines) as f32,
            );
        }

        if vy > 0 {
            // top
            Draw_TileClear(cbx, vx as f32, 0.0, vw as f32, vy as f32);
            // bottom
            Draw_TileClear(
                cbx,
                vx as f32,
                (vy + vh) as f32,
                vw as f32,
                (glheight - vy - vh - sb_lines) as f32,
            );
        }
    }
}
