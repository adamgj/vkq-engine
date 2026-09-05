//! `Quake/sbar_glue.c` declarations (Rust migration Phase 7 M10d).
//!
//! ADR-011: engine C symbols are declared only in this crate. The glue file
//! owns the four C-visible objects `Quake/sbar.c` used to define --
//! `sb_showscores`, `sb_lines`, `fragsort[]` and `scoreboardlines` -- plus
//! the plain-name entry points whose signatures mention `cb_context_t *` or
//! `qpic_t *` and therefore have no cbindgen spelling.
//!
//! ## Why the storage stays C
//!
//! ADR-007: all four objects had external linkage in `sbar.c` and all four
//! have live C readers outside it. `gl_screen.c:397-410` writes and reads
//! `sb_lines` and `:682` reads `sb_showscores`; `pr_ext.c:5344-5347`
//! (`PF_cl_playerkey_internal`) reads `fragsort` and `scoreboardlines`. The
//! definitions stay in `Quake/sbar_glue.c` and Rust reaches them through the
//! externs below, which also keeps them genuinely separate objects from the
//! oracle's `c_ref_*` copies in the differential build.
//!
//! Everything else `sbar.c` defined at file scope is `static` with no reader
//! outside the file -- the ~150 `qpic_t *` handles, `hipweapons[]` and
//! `hudtype` -- so those move to Rust.
//!
//! `cl`, `cls` and `vid` are mirror-typed and so are reached through
//! [`crate::cl_parse`] / `quake-capi`, which can name `quake_types`; this
//! crate has no `[dependencies]` (the same finding `quake-c-sys/src/sv_user.rs`
//! records). The renderer entry points, `glwidth`/`glheight`,
//! `scr_con_current`, `scr_viewsize` and `realtime` are already declared by
//! [`crate::console`] and are reused from there rather than redeclared.

use crate::{cvar_t, qboolean};
use core::ffi::{c_char, c_float, c_int, c_void};

/// `quakedef.h:214` -- `#define MAX_SCOREBOARD 16`.
pub const MAX_SCOREBOARD: usize = 16;

/// ADR-011 mirror of `qpic_t` (`wad.h:52-56`). bindgen never sees it because
/// every prototype that mentions it also mentions `cb_context_t`, which the
/// core headers deliberately keep out of the bindgen surface. `sbar.c` only
/// ever reads `width` (`sbar.c:1367`, `:1382`, `:1404`, `:1612`, `:1615`,
/// `:1640`), but the layout is transcribed in full so `size_of` is right.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct qpic_t {
    pub width: c_int,
    pub height: c_int,
    pub data: [u8; 4],
}

extern "C" {
    /* ---------------------------------------------------------------------
     * Glue-owned storage (sbar.c:47, :49, :439, :441).
     */

    /// `sbar.c:47` -- `qboolean sb_showscores;`. Read by `gl_screen.c:682`.
    pub static mut sb_showscores: qboolean;
    /// `sbar.c:49` -- `int sb_lines;`, the scan lines to draw. Written by
    /// `gl_screen.c:397-401` and read by `:408-410`, `:1105-1117`.
    pub static mut sb_lines: c_int;
    /// `sbar.c:439` -- `int fragsort[MAX_SCOREBOARD];`. Read by
    /// `pr_ext.c:5345`.
    pub static mut fragsort: [c_int; MAX_SCOREBOARD];
    /// `sbar.c:441` -- `int scoreboardlines;`. Read by `pr_ext.c:5344`.
    pub static mut scoreboardlines: c_int;

    /* ---------------------------------------------------------------------
     * The two console commands Sbar_Init registers (sbar.c:285-286). Both are
     * `Host_Reraise` wrappers in the glue: each reaches Sbar_CSQCCommand,
     * which runs QC (ADR-009 rule 3). `Sbar_DontShowScores` is `static` in
     * `sbar.c`, so the glue exports it under a `Sbar_Glue_` name.
     */

    /// `Quake/sbar_glue.c` -- `void Sbar_ShowScores (void)`.
    pub fn Sbar_ShowScores();
    /// `Quake/sbar_glue.c` -- `void Sbar_Glue_DontShowScores (void)`.
    pub fn Sbar_Glue_DontShowScores();

    /* ---------------------------------------------------------------------
     * Renderer entry points `console.rs` does not already declare.
     * `cb_context_t *` and `qpic_t *` are opaque `void *` here, exactly as in
     * `crate::console`.
     */

    /// `draw.h:44` -- `qpic_t *Draw_PicFromWad (const char *name)`.
    pub fn Draw_PicFromWad(name: *const c_char) -> *mut c_void;
    /// `draw.h:48` -- `qpic_t *Draw_CachePic (const char *path)`.
    pub fn Draw_CachePic(path: *const c_char) -> *mut c_void;
    /// `draw.h:51` -- `void Draw_TileClear (cb_context_t *, int, int, int, int)`.
    pub fn Draw_TileClear(cbx: *mut c_void, x: c_int, y: c_int, w: c_int, h: c_int);
    /// `draw.h:53` -- the sub-rectangle blit `Sbar_DrawModern` uses for the
    /// modern ammo strip (`sbar.c:1163`).
    #[allow(clippy::too_many_arguments)]
    pub fn Draw_SubPic(
        cbx: *mut c_void,
        x: c_float,
        y: c_float,
        w: c_float,
        h: c_float,
        pic: *mut c_void,
        s1: c_float,
        t1: c_float,
        s2: c_float,
        t2: c_float,
        rgb: *mut c_float,
        alpha: c_float,
    );
    /// `menu.h:78` -- `sbar.c:1461` draws scoreboard names in the menu font.
    pub fn M_Print(cbx: *mut c_void, cx: c_int, cy: c_int, str_: *const c_char);
    /// `menu.h:82` -- `sbar.c:1405`.
    pub fn M_DrawPic(cbx: *mut c_void, x: c_int, y: c_int, pic: *mut c_void);
    /// `draw.h:31` -- `qpic_t *draw_disc`, the loading disc `sbar.c:922` and
    /// `:1018` draw in place of the armor icon while invulnerable.
    pub static mut draw_disc: *mut c_void;
    /// `gl_draw.c:36` -- `qpic_t *pic_nul`, the placeholder
    /// `Sbar_CheckPicFromWad` (`sbar.c:117`) compares against.
    pub static mut pic_nul: *mut c_void;

    /* ---------------------------------------------------------------------
     * Everything else sbar.c reaches.
     */

    /// `wad.h:100` -- `void *W_GetLumpName (const char *, lumpinfo_t **)`.
    /// `lumpinfo_t *` is opaque here: `sbar.c:124` only tests the result.
    pub fn W_GetLumpName(name: *const c_char, out_info: *mut *mut c_void) -> *mut c_void;
    /// `progs.h:207` -- `int PR_MakeTempString (const char *val)`.
    /// `sbar.c:82` hands the result to `G_INT (OFS_PARM0)`.
    pub fn PR_MakeTempString(val: *const c_char) -> c_int;

    /// `gl_screen.c:94` -- `cvar_t scr_style`, the HUD style selector. It is
    /// in no header: `gl_draw.c:29`, `menu.c:119` and `sbar.c:69` each
    /// re-declare it locally.
    pub static mut scr_style: cvar_t;
    /// `screen.h:56` -- `cvar_t scr_sbaralpha`.
    pub static mut scr_sbaralpha: cvar_t;
    /// `screen.h:59` -- `cvar_t scr_sbarscale`.
    pub static mut scr_sbarscale: cvar_t;

    /* string helpers the port needs that other modules already declare;
     * identical signatures, so the duplicates are the same pattern `va`
     * follows across five modules of this crate. */
    pub fn strlen(s: *const c_char) -> usize;
    pub fn q_snprintf(str_: *mut c_char, size: usize, format: *const c_char, ...) -> c_int;
    /// `common.c` -- `char *va (const char *format, ...)`. `Sbar_LoadPics`
    /// (`sbar.c:143`) builds every numbered lump name with it.
    pub fn va(format: *const c_char, ...) -> *mut c_char;
}
