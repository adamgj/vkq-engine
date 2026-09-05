//! `Quake/console_glue.c` declarations (Rust migration Phase 7 M10c).
//!
//! ADR-011: engine C symbols are declared only in this crate. The glue file
//! owns every C-visible object `Quake/console.c` used to define -- the
//! scrollback buffer and its geometry, the six console cvars, the notify
//! ring, the rcon redirect buffer and `con_mutex` -- plus the three
//! `Host_Guard` trampolines this port needs and the three shims that keep
//! SDL, `ENGINE_NAME_AND_VER` and `stderr` out of Rust (ADR-009, ADR-004).
//!
//! ## Why the storage stays C
//!
//! ADR-007: every object below had external linkage in `console.c`, and most
//! have live C readers outside it -- `keys.c:245` reads `con_text` and
//! `key_tabpartial`, `gl_screen.c` reads `con_forcedup`/`con_backscroll`/
//! `con_totallines`, `host.c` reads `con_initialized`, `sv_main.c` and
//! `net_dgrm.c` drive the redirect through `con_redirect_flush`. The glue
//! keeps the definitions and Rust reaches them through these externs, which
//! also keeps them genuinely separate objects from the oracle's copies in
//! the differential build.
//!
//! The two file-local objects with external linkage that nothing outside
//! `console.c` ever named -- `tablist` and the `tab_t` type -- move to Rust
//! instead.
//!
//! `cls`, `cl` and `vid` are mirror-typed and so are reached through
//! [`crate::cl_parse`] / `quake-capi`, which can name `quake_types`; this
//! crate has no `[dependencies]` (the same finding
//! `quake-c-sys/src/sv_user.rs` records).

use crate::{cmd_source_t, cvar_t, qboolean, qmutex_t, xcommand_t, FILE};
use core::ffi::{c_char, c_int, c_uint, c_void};

/// `keys.h:134` -- `#define MAXCMDLINE 256`.
pub const MAXCMDLINE: usize = 256;
/// `cmd.c:35-40` -- `#define MAX_ALIAS_NAME 32`, re-spelled at
/// `console.c:1557`.
pub const MAX_ALIAS_NAME: usize = 32;

/// `cmd.h:90` -- `typedef void (*xtabcommand_t) (const char *partial);`.
#[allow(non_camel_case_types)]
pub type xtabcommand_t = Option<unsafe extern "C" fn(*const c_char)>;

/// `console.c:1547-1554` -- `tab_t`, the node type of the `tablist` loop.
/// The struct is file-local to `console.c` but the list head has external
/// linkage, so the layout is transcribed here alongside the extern.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct tab_t {
    pub name: *const c_char,
    pub ty: *const c_char,
    pub next: *mut tab_t,
    pub prev: *mut tab_t,
    pub count: c_int,
}

/// ADR-011 mirror of `cmd_function_t` (`cmd.h:92-101`). bindgen emits
/// `cmd_function_t` as opaque because no C prototype exposes its fields, so
/// `BuildTabList`'s walk of `cmd_functions` needs the layout transcribed
/// field-for-field from the header. Kept in step with the identical mirror
/// in `quake-capi/src/cmd.rs:41`.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct cmd_function_mirror_t {
    pub next: *mut cmd_function_mirror_t,
    pub name: *const c_char,
    pub function: xcommand_t,
    pub completion: xtabcommand_t,
    pub srctype: cmd_source_t,
    pub dynamic: qboolean,
    pub qcinterceptable: qboolean,
}

/// ADR-011 mirror of `cmd.c`'s private `cmdalias_t` (`cmd.c:35-40`). The
/// same struct is transcribed into `Quake/cvar_cmd_glue.c`, into
/// `console.c:1558-1563` and into `quake-capi/src/cmd.rs:53`.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct cmdalias_mirror_t {
    pub next: *mut cmdalias_mirror_t,
    pub name: [c_char; MAX_ALIAS_NAME],
    pub value: *mut c_char,
}

/// `common.h:117-121` -- the `vec_header_t` that sits immediately before a
/// `VEC_*` array's payload.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(non_camel_case_types)]
pub struct vec_header_t {
    pub capacity: usize,
    pub size: usize,
}

extern "C" {
    /* -----------------------------------------------------------------
     * Quake/console_glue.c data (console.c:38-79, :1546).
     */
    pub static mut con_linewidth: c_int;
    pub static mut con_cursorspeed: f32;
    pub static mut con_buffersize: c_int;
    pub static mut con_forcedup: qboolean;
    pub static mut con_totallines: c_int;
    pub static mut con_backscroll: c_int;
    pub static mut con_current: c_int;
    pub static mut con_x: c_int;
    pub static mut con_text: *mut c_char;

    pub static mut con_notifytime: cvar_t;
    pub static mut con_logcenterprint: cvar_t;
    pub static mut con_notifycenter: cvar_t;
    pub static mut con_notifyfade: cvar_t;
    pub static mut con_notifyfadetime: cvar_t;
    pub static mut con_maxcols: cvar_t;

    pub static mut con_lastcenterstring: [c_char; 1024];
    /// `console.h:53` -- `void (*con_redirect_flush) (const char *buffer)`.
    pub static mut con_redirect_flush: Option<unsafe extern "C" fn(*const c_char)>;
    pub static mut con_redirect_buffer: [c_char; 8192];
    /// `console.c:69` -- `float con_times[NUM_CON_TIMES]`.
    pub static mut con_times: [f32; 4];
    pub static mut con_vislines: c_int;
    pub static mut con_debuglog: qboolean;
    pub static mut con_initialized: qboolean;
    /// `console.c:75` -- `qmutex_t *con_mutex`.
    pub static mut con_mutex: *mut qmutex_t;

    /// `console.c:1546` -- shared with `keys.c:245`.
    pub static mut key_tabpartial: [c_char; MAXCMDLINE];

    /// `console.c:1555` -- `tab_t *tablist`. It had external linkage in the
    /// original, so `Quake/console_glue.c` keeps the storage (ADR-007) and
    /// the ctest oracle can observe tab-completion ordering directly.
    pub static mut tablist: *mut tab_t;

    /* -----------------------------------------------------------------
     * Quake/console_glue.c ADR-009 trampolines -- each returns a Host_Guard
     * status that must reach the glue's Host_Reraise frame untouched.
     */

    /// `console.c:763` -- `M_Menu_Main_f ()`: the menu runs console commands
    /// and loads maps, so it is raise-capable from many directions.
    pub fn Console_Glue_MenuMain() -> c_int;
    /// `console.c:1858` -- `cvar->completion (cvar, partial)`: a QC-supplied
    /// completion callback can reach `PR_GetString` and `Host_Error`.
    pub fn Console_Glue_CvarCompletion(cvar: *mut cvar_t, partial: *const c_char) -> c_int;
    /// `console.c:1866` -- `cmd->completion (partial)`: same reasoning.
    pub fn Console_Glue_CmdCompletion(completion: xtabcommand_t, partial: *const c_char) -> c_int;

    /* -----------------------------------------------------------------
     * Quake/console_glue.c non-guard shims.
     */

    /// `console.c:834` -- `SDL_SetClipboardText`. Routed through the glue so
    /// no Rust translation unit names an SDL symbol (`check_headers.sh`).
    pub fn Console_Glue_SetClipboardText(text: *const c_char);
    /// `quakever.h` -- the `ENGINE_NAME_AND_VER` macro as a string.
    pub fn Console_Glue_EngineNameAndVer() -> *const c_char;
    /// `console.c:2437` -- `fprintf (stderr, ...)`; `stderr` is a macro on
    /// several libcs, so the glue owns the call.
    pub fn Console_Glue_LogOpenFailed(name: *const c_char);
    /// `console.c:2440` -- `setvbuf (log_file, NULL, _IONBF, 0)`. `_IONBF`
    /// is 2 on glibc/musl/BSD but 4 on the MSVC CRT, so the constant is
    /// never spelled in Rust.
    pub fn Console_Glue_LogSetUnbuffered(f: *mut FILE);

    /* -----------------------------------------------------------------
     * The C-variadic console entry points. Decision 1 of the M10c contract
     * keeps every one of them in the glue so libc does the formatting
     * (ADR-005's Rust formatter is deliberately *not* used here); Rust calls
     * back into them through these declarations. `Con_Printf` and
     * `Con_SafePrintf` already come from the bindgen surface.
     */
    pub fn Con_LinkPrintf(addr: *const c_char, fmt: *const c_char, ...);
    pub fn Con_CenterPrintf(linewidth: c_int, fmt: *const c_char, ...);

    /// `console_glue.c:209` -- the re-raising wrapper. `Con_Init` registers
    /// *this* with `Cmd_AddCommand`, never the Rust status core, so the jump
    /// is always issued from a C frame (ADR-009).
    pub fn Con_ToggleConsole_f();

    /* -----------------------------------------------------------------
     * Non-raising C callees the console reaches that the committed bindings
     * do not already carry. The ADR-009 audit for each is written out in
     * `quake-capi/src/console.rs`.
     */

    /// `snd_dma.c:1135` -- reaches only `Con_Printf` and `S_StartSound`.
    pub fn S_LocalSound(name: *const c_char);
    /// `sys.h` -- only `Sys_Printf` on the failure paths.
    pub fn Sys_Explore(path: *const c_char) -> qboolean;
    /// `gl_screen.c:993` -- two assignments.
    pub fn SCR_EndLoadingPlaque();
    /// `input.h:62`.
    pub fn IN_GetMousePos(outx: *mut c_int, outy: *mut c_int);
    /// `input.h:50`.
    pub fn IN_Activate();
    /// `input.h:56`.
    pub fn IN_DeactivateForConsole();
    /// `vid.h:99` -- `void VID_SetMouseCursor (mousecursor_t cursor)`.
    pub fn VID_SetMouseCursor(cursor: c_int);

    /* renderer entry points (gl_draw.c); `cb_context_t *` is opaque here */
    pub fn Draw_Character(cbx: *mut c_void, x: f32, y: f32, num: c_int);
    pub fn Draw_String(cbx: *mut c_void, x: f32, y: f32, str_: *const c_char);
    pub fn Draw_Pic(
        cbx: *mut c_void,
        x: f32,
        y: f32,
        pic: *mut c_void,
        alpha: f32,
        alpha_blend: qboolean,
    );
    pub fn Draw_Fill(cbx: *mut c_void, x: f32, y: f32, w: f32, h: f32, c: c_int, alpha: f32);
    pub fn Draw_ConsoleBackground(cbx: *mut c_void);
    /// `draw.h:64` -- `void GL_SetCanvas (cb_context_t *, canvastype)`.
    pub fn GL_SetCanvas(cbx: *mut c_void, newcanvas: c_int);
    /// `draw.h:65`.
    pub fn GL_SetCanvasColor(r: f32, g: f32, b: f32, a: f32);
    /// `gl_draw.c` -- `qpic_t *pic_ovr, *pic_ins`.
    pub static mut pic_ovr: *mut c_void;
    /// `gl_draw.c` -- `qpic_t *pic_ovr, *pic_ins`.
    pub static mut pic_ins: *mut c_void;

    /* common.c dynamic array helpers (common.h:136-139) */
    pub fn Vec_Grow(pvec: *mut *mut c_void, element_size: usize, count: usize);
    pub fn Vec_Append(
        pvec: *mut *mut c_void,
        element_size: usize,
        data: *const c_void,
        count: usize,
    );
    pub fn Vec_Clear(pvec: *mut *mut c_void);
    pub fn Vec_Free(pvec: *mut *mut c_void);

    /* common.c / strl_fn.h string helpers */
    pub fn q_strlcat(dst: *mut c_char, src: *const c_char, size: usize) -> usize;
    pub fn q_strnaturalcmp(s1: *const c_char, s2: *const c_char) -> c_int;
    pub fn UTF8_FromQuake(dst: *mut c_char, maxbytes: usize, src: *const c_char) -> usize;
    pub fn COM_TintSubstring(
        in_: *const c_char,
        str_: *const c_char,
        out: *mut c_char,
        outsize: usize,
    ) -> *mut c_char;

    /* cvar.c / cmd.c lookups BuildTabList walks */
    pub fn Cmd_TokenizeString(text: *const c_char);
    pub fn Cmd_AddArg(arg: *const c_char);
    pub fn Cvar_FindVar(var_name: *const c_char) -> *mut cvar_t;
    pub fn Cvar_FindVarAfter(prev_name: *const c_char, with_flags: c_uint) -> *mut cvar_t;
    pub fn Cmd_FindCommand(name: *const c_char) -> *mut cmd_function_mirror_t;
    pub fn Cmd_IsReservedName(name: *const c_char) -> qboolean;
    /// `cvar_cmd_glue.c` -- `cmd_function_t *cmd_functions;`.
    pub static mut cmd_functions: *mut cmd_function_mirror_t;
    /// `cvar_cmd_glue.c` -- `cmdalias_t *cmd_alias;`.
    pub static mut cmd_alias: *mut cmdalias_mirror_t;

    /* screen / video state the console reads */
    /// `glquake.h:41`.
    pub static mut glwidth: c_int;
    /// `glquake.h:41`.
    pub static mut glheight: c_int;
    /// `screen.h` -- `float scr_con_current`.
    pub static mut scr_con_current: f32;
    /// `screen.h` -- `cvar_t scr_viewsize`.
    pub static mut scr_viewsize: cvar_t;

    /// `host.c` -- `double realtime`.
    pub static mut realtime: f64;
    /// `host.c` -- `double host_rawframetime`.
    pub static mut host_rawframetime: f64;

    /* string helpers the port needs that other modules already declare;
     * identical signatures, so the duplicates are the same pattern `va`
     * follows across five modules of this crate. */
    pub fn q_strlcpy(dst: *mut c_char, src: *const c_char, size: usize) -> usize;
    pub fn q_snprintf(str_: *mut c_char, size: usize, format: *const c_char, ...) -> c_int;
    pub fn q_strcasecmp(s1: *const c_char, s2: *const c_char) -> c_int;
    /// `console.c:876` -- `strncpy (buffer, line, con_linewidth)`; the source
    /// region is not NUL-terminated, so the exact libc semantics matter.
    pub fn strncpy(dst: *mut c_char, src: *const c_char, n: usize) -> *mut c_char;
    /// `console.c:1058` -- `atoi (com_argv[i + 1])`.
    pub fn atoi(nptr: *const c_char) -> c_int;

    /// `common.c` -- `char *q_strcasestr (const char *haystack, const char *needle)`.
    pub fn q_strcasestr(haystack: *const c_char, needle: *const c_char) -> *mut c_char;

    /* libc the log helpers need */
    pub fn fprintf(stream: *mut FILE, fmt: *const c_char, ...) -> c_int;
    pub fn fwrite(ptr: *const c_void, size: usize, nmemb: usize, stream: *mut FILE) -> usize;
    pub fn fclose(stream: *mut FILE) -> c_int;
    /// `common.c` -- `char *va (const char *format, ...)`.
    pub fn va(format: *const c_char, ...) -> *mut c_char;
}
