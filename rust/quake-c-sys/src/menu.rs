//! `Quake/menu_glue.c` declarations (Rust migration Phase 7 M10e).
//!
//! ADR-011: engine C symbols are declared only in this crate. The glue file
//! owns the eight C-visible objects `Quake/menu.c` defined, the plain-name
//! entry points whose signatures mention `cb_context_t *`, `qpic_t *` or a
//! by-value `crosshair_t` (none of which cbindgen can spell), and the
//! ADR-009 `Host_Guard` trampolines.
//!
//! ## Why the storage stays C
//!
//! ADR-007: eight of `menu.c`'s file-scope objects have live readers outside
//! the file, so their storage stays in `Quake/menu_glue.c` and Rust reaches
//! them through the externs below.
//!
//! * `m_state` -- `gl_vidsdl.c:5392` writes it, `net_dgrm.c:1813` writes it.
//! * `m_return_state` -- `net_dgrm.c:1813` reads it.
//! * `m_entersound` -- `gl_vidsdl.c:5292` and `:5393` write it.
//! * `m_is_quitting` -- `keys.c:1259` reads it.
//! * `m_return_onerror` -- `net_dgrm.c:61` declares it, `:1803`/`:1810`/
//!   `:1814` read and write it. Already ported: `net_dgrm_orch.rs:246`
//!   declares the same object and `quake-capi/src/net_dgrm_orch.rs:2596-2599`
//!   reads and writes it by plain name.
//! * `m_return_reason` -- `net_dgrm.c:62` declares it and `:1669`-`:1774`
//!   write it; `net_dgrm_orch.rs:253` declares the same 32-byte array and
//!   `quake-capi/src/net_dgrm_orch.rs:2556` writes it by plain name.
//! * `vid_menucmdfn` / `vid_menukeyfn` -- `vid.h:72-73` declares both. No
//!   translation unit reads or writes them today, but the declarations are
//!   part of the public header, so the definitions stay in C. Rust never
//!   names them, which is why they are absent from the block below.
//!
//! Every other file-scope object in `menu.c` had external linkage only by
//! accident -- `m_save_demonum`, `load_cursor`, `m_multiplayer_cursor`,
//! `setup_cursor`, `setup_cursor_table`, `setup_hostname`, `setup_myname`,
//! `m_singleplayer_cursor`, `m_filenames`, `loadable`, `setup_oldtop`,
//! `setup_oldbottom`, `setup_top`, `setup_bottom`, `m_net_cursor`,
//! `m_first_net_item`, `m_net_items`, `bindnames` and `help_page`. A grep
//! over every `Quake/*.c` and `Quake/*.h` finds no other translation unit
//! that names any of them, so all of them move to Rust.
//!
//! This crate has no `[dependencies]`, so engine aggregates that
//! `quake-types` mirrors (`cl`, `cls`, `vid`) are reached through the
//! modules that already declare them; `cb_context_t *` is `*mut c_void`
//! throughout, exactly as [`crate::console`] spells it.

use crate::{cvar_t, qboolean, FILE};
use core::ffi::{c_char, c_float, c_int, c_uint, c_void};

/// `quakedef.h:241-242` -- `CANVAS_MENU` is the fourth `canvastype`.
pub const CANVAS_MENU: c_int = 3;

/// `gl_texmgr.h:40` -- `TEXPREF_ALPHA = 0x0008`.
pub const TEXPREF_ALPHA: c_uint = 0x0008;
/// `gl_texmgr.h:41` -- `TEXPREF_PAD = 0x0010`.
pub const TEXPREF_PAD: c_uint = 0x0010;
/// `gl_texmgr.h:44` -- `TEXPREF_NOPICMIP = 0x0080`.
pub const TEXPREF_NOPICMIP: c_uint = 0x0080;
/// `draw.h:36` -- `PICFLAG_AUTO = 0`.
pub const PICFLAG_AUTO: c_int = 0;

/// ADR-011 mirror of `crosshair_t` (`menu.h:100-107`). `M_GetCrosshairDef`
/// returns it by value, so the layout has to be exact; the glue owns the
/// plain name and hands the value back through an out-pointer rather than
/// relying on a struct-return ABI that cbindgen cannot describe.
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy)]
pub struct crosshair_t {
    pub crosshair_char: c_char,
    pub viewport_x_offset: c_float,
    pub viewport_y_offset: c_float,
    pub menu_x_offset: c_int,
    pub menu_y_offset: c_int,
}

/// ADR-011 mirror of `filelist_item_t` (`quakedef.h:411-415`). The menu walks
/// `modlist` and `extralevels` as plain singly linked lists.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct filelist_item_t {
    pub name: [c_char; 32],
    pub next: *mut filelist_item_t,
}

/// ADR-011 mirror of `vec_header_t` (`common.h:118-122`), the two-word header
/// the `VEC_*` macros read at `((vec_header_t *)v)[-1]`.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct vec_header_t {
    pub capacity: usize,
    pub size: usize,
}

/// ADR-011 mirror of `qpic_t` (`wad.h:52-56`). The menu only ever reads
/// `width` and `height` off a pointer the renderer just returned, but the
/// layout is transcribed in full so `size_of` is right.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct qpic_t {
    pub width: c_int,
    pub height: c_int,
    pub data: [u8; 4],
}

extern "C" {
    /* ---------------------------------------------------------------------
     * Glue-owned storage (menu.c:29, :85, :89, :91-93).
     */

    /// `menu.c:29` -- `enum m_state_e m_state;`. Declared `c_int`: the enum
    /// (`menu.h:26-51`) has 23 enumerators, all non-negative, so the C
    /// compiler gives it `int` rank on every supported target. Matches the
    /// spelling [`crate::net_dgrm_orch`] already uses for the same object.
    pub static mut m_state: c_int;
    /// `menu.c:91` -- `enum m_state_e m_return_state;`.
    pub static mut m_return_state: c_int;
    /// `menu.c:85` -- `qboolean m_entersound;`.
    pub static mut m_entersound: qboolean;
    /// `menu.c:89` -- `qboolean m_is_quitting;`. Read by `keys.c:1259`.
    pub static mut m_is_quitting: qboolean;
    /// `menu.c:92` -- `qboolean m_return_onerror;`. See the module doc.
    pub static mut m_return_onerror: qboolean;
    /// `menu.c:93` -- `char m_return_reason[32];`. See the module doc.
    pub static mut m_return_reason: [c_char; 32];

    /* ---------------------------------------------------------------------
     * ADR-009 rule 3 trampolines (menu_glue.c). Each one holds its arguments
     * in a C frame, runs the callee under `Host_Guard`, and returns the
     * status for the Rust core to propagate with `raise!`. No longjmp ever
     * crosses a Rust frame.
     */

    /// `menu.c:485` -- `Con_ToggleConsole_f ()`. Under `-Duse_rust_host` the
    /// plain name is itself a `Host_Reraise` wrapper (`console_glue.c:234`),
    /// so calling it from a Rust frame would re-issue the jump across that
    /// frame. Always enter through this guard instead.
    pub fn Menu_Glue_ToggleConsole() -> c_int;
    /// `menu.c:676` -- `CL_NextDemo ()`, which reaches the demo loop and
    /// `Host_EndGame`.
    pub fn Menu_Glue_NextDemo() -> c_int;
    /// `menu.c:793`, `:2274` -- `SCR_ModalMessage (text, timeout)`. It spins
    /// `SCR_UpdateScreen (false)` (`gl_screen.c:1044+`), which reaches
    /// `Mod_LoadModel (mod, true)` -> `Host_Error` (`gl_model.c:531`).
    pub fn Menu_Glue_ModalMessage(text: *const c_char, timeout: c_float, out: *mut c_int) -> c_int;
    /// `menu.c:952`, `:4387` -- `SCR_BeginLoadingPlaque ()`, same
    /// `SCR_UpdateScreen` path as `Menu_Glue_ModalMessage`.
    pub fn Menu_Glue_BeginLoadingPlaque() -> c_int;

    /// `menu.c:1203` -- `Cvar_Set (name, value)`. Every cvar write can reach
    /// `Host_Error` through `Cvar_SetQuick` -> `Cvar_CallCallback`
    /// (`cvar.c:507`), and under `-Duse_rust_cvar` the plain name is itself a
    /// `Host_Reraise` wrapper. Mirrors `HostCmd_Glue_CvarSet`
    /// (`host_cmd_glue.c:548`) rather than reusing it, per the established
    /// per-module-duplication convention.
    pub fn Menu_Glue_CvarSet(name: *const c_char, value: *const c_char) -> c_int;
    /// The 40 `Cvar_SetValue (name, value)` sites in the options menus; same
    /// callback exposure as `Menu_Glue_CvarSet`.
    pub fn Menu_Glue_CvarSetValue(name: *const c_char, value: c_float) -> c_int;
    /// The 17 `Cvar_SetValueQuick (&var, value)` sites in the options menus.
    pub fn Menu_Glue_CvarSetValueQuick(var: *mut cvar_t, value: c_float) -> c_int;

    /// `menu.c:2284` -- `M_Menu_Video_f ()`. The video menu lives in
    /// `Quake/gl_vidsdl.c:5387`, not in `menu.c`, and its body runs
    /// `VID_SyncCvars ()`, which writes cvars and so can reach `Host_Error`
    /// through `Cvar_CallCallback`.
    pub fn Menu_Glue_MenuVideo() -> c_int;

    /// `menu.c:4736` -- `M_Video_Draw (cbx)`. The video menu's draw handler
    /// also lives in `Quake/gl_vidsdl.c:5328`, and it calls `VID_SyncCvars ()`
    /// plus `VID_Menu_RebuildModeList ()`/`VID_Menu_RebuildRateList ()`, so it
    /// is raise-capable for the same reason as `Menu_Glue_MenuVideo`.
    pub fn Menu_Glue_VideoDraw(cbx: *mut c_void) -> c_int;
    /// `menu.c:4860` -- `M_Video_Key (key)` (`Quake/gl_vidsdl.c:5216`); it
    /// runs `VID_SyncCvars ()` and applies mode changes through cvar writes.
    pub fn Menu_Glue_VideoKey(key: c_int) -> c_int;

    /* ---------------------------------------------------------------------
     * Non-guard glue shims: things a Rust translation unit must not name.
     */

    /// `menu.c:4628`/`:4632` -- `SDL_GetMouseState`. Routed through the glue
    /// so no Rust translation unit names an SDL symbol, and so the SDL2/SDL3
    /// `int *` vs `float *` split stays behind one C signature. The glue
    /// always yields the SDL3 `float` form; the SDL2 build widens.
    pub fn Menu_Glue_GetMouseState(x: *mut c_float, y: *mut c_float);
    /// `menu.c:1738`, `:1925`, `:2062` -- `vulkan_globals.ray_query`. The
    /// core headers stay Vulkan-free (`check_headers.sh`), so the Vulkan
    /// device queries are read in C and handed over as scalars.
    pub fn Menu_Glue_RayQuery() -> qboolean;
    /// `menu.c:1898`, `:2030` --
    /// `vulkan_globals.device_features.sampleRateShading`.
    pub fn Menu_Glue_SampleRateShading() -> qboolean;
    /// `menu.c:2038` --
    /// `vulkan_globals.device_properties.limits.maxSamplerAnisotropy`.
    pub fn Menu_Glue_MaxSamplerAnisotropy() -> c_float;
    /// `menu.c:3654` -- `ENGINE_NAME_AND_VER` (`quakever.h:59-61`) is a
    /// build-time string macro with no Rust spelling.
    pub fn Menu_Glue_EngineNameAndVer() -> *const c_char;

    /// `host_glue.c:423` -- `HOST_GUARD_VOID (NET_Poll)`, reused verbatim for
    /// `menu.c:4436`. `NET_Poll` runs `pp->procedure (pp->arg)` over an
    /// arbitrary scheduled list, so it is raise-capable; `host.rs:1667`
    /// already propagates this exact status.
    pub fn Host_Glue_NET_Poll() -> c_int;

    /* ---------------------------------------------------------------------
     * Engine C callees the menu reaches. None of these can longjmp; the
     * per-callee ADR-009 audit is written out in `quake-capi/src/menu.rs`.
     * `Con_SafePrintf` is the one documented exception: `Con_Printf` is
     * raise-capable in principle, but the project's standing decision keeps
     * it plain and unguarded from Rust across every ported module.
     */

    pub fn Con_SafePrintf(fmt: *const c_char, ...);

    /* common.c */
    pub fn COM_Rand() -> c_int;
    pub fn COM_HashBlock(data: *const c_void, size: usize) -> c_uint;
    pub fn COM_FileExists(filename: *const c_char, path_id: *mut c_uint) -> qboolean;
    pub fn COM_OpenFile(
        filename: *const c_char,
        handle: *mut c_int,
        path_id: *mut c_uint,
    ) -> crate::qfilesize_t;
    pub fn COM_CloseFile(h: c_int);
    pub fn COM_StripExtension(in_: *const c_char, out: *mut c_char, outsize: usize);
    pub fn COM_TintSubstring(
        in_: *const c_char,
        substr: *const c_char,
        out: *mut c_char,
        outsize: usize,
    ) -> *mut c_char;
    pub fn COM_GetGameNames(full: qboolean) -> *const c_char;

    /* common.c dynamic arrays (common.h:135-138), behind the `VEC_*` macros */
    pub fn Vec_Grow(pvec: *mut *mut c_void, element_size: usize, count: usize);
    pub fn Vec_Clear(pvec: *mut *mut c_void);
    pub fn Vec_Free(pvec: *mut *mut c_void);

    /* cmd.c / cvar.c */
    pub fn Cbuf_AddText(text: *const c_char);
    pub fn Cbuf_InsertText(text: *const c_char);
    pub fn Cmd_Argc() -> c_int;
    pub fn Cmd_Argv(arg: c_int) -> *const c_char;

    /* gl_draw.c -- `cb_context_t *` and `qpic_t *` are opaque here */
    pub fn Draw_CachePic(path: *const c_char) -> *mut c_void;
    pub fn Draw_TryCachePic(path: *const c_char, texflags: c_uint, picflags: c_int) -> *mut c_void;
    pub fn Draw_Character(cbx: *mut c_void, x: c_float, y: c_float, num: c_int);
    pub fn Draw_Pic(
        cbx: *mut c_void,
        x: c_float,
        y: c_float,
        pic: *mut c_void,
        alpha: c_float,
        alpha_blend: qboolean,
    );
    pub fn Draw_TransPicTranslate(
        cbx: *mut c_void,
        x: c_float,
        y: c_float,
        pic: *mut c_void,
        top: c_int,
        bottom: c_int,
    );
    pub fn Draw_ConsoleBackground(cbx: *mut c_void);
    pub fn Draw_FadeScreen(cbx: *mut c_void);
    pub fn GL_SetCanvas(cbx: *mut c_void, newcanvas: c_int);
    pub fn GL_SetCanvasColor(r: c_float, g: c_float, b: c_float, a: c_float);
    /// `gl_draw.c` -- `qpic_t *pic_up, *pic_down;` (`menu.c:2408`).
    pub static mut pic_up: *mut c_void;
    /// `gl_draw.c` -- `qpic_t *pic_up, *pic_down;` (`menu.c:2408`).
    pub static mut pic_down: *mut c_void;
    /// `gl_vidsdl.c` -- the drawable size the menu canvas is derived from.
    pub static mut glwidth: c_int;
    /// `gl_vidsdl.c` -- see `glwidth`.
    pub static mut glheight: c_int;
    /// `gl_screen.c` -- `float scr_con_current;` (`menu.c:4675`).
    pub static mut scr_con_current: c_float;
    /// `host.c` -- `double realtime;` (`menu.c:657`, the main-menu cursor
    /// animation).
    pub static mut realtime: f64;
    /// `quakedef.h:406` -- `double host_rawframetime;` (unscaled and
    /// unbounded), the maps-menu ticker's time source (`menu.c:2679`,
    /// `:2681`).
    pub static mut host_rawframetime: f64;

    /* keys.c */
    pub fn Key_KeynumToString(keynum: c_int) -> *const c_char;
    pub fn Key_SetBinding(keynum: c_int, binding: *const c_char);
    /// `keys.c` -- `qboolean keydown[256];` (`menu.c:113`).
    pub static mut keydown: [qboolean; 256];
    /// `keys.h:144` -- `keydest_t key_dest;`, an `int`-ranked enum.
    pub static mut key_dest: c_int;

    /* input.c */
    pub fn IN_Activate();
    pub fn IN_Deactivate(free_cursor: qboolean);

    /* platform */
    pub fn PL_GetClipboardData() -> *mut c_char;

    /* net_main.c */
    pub fn NET_Slist_f();
    pub fn NET_SlistSort();
    pub fn NET_SlistPrintServer(n: usize) -> *const c_char;
    pub fn NET_SlistPrintServerName(n: usize) -> *const c_char;
    pub fn NET_ListAddresses(addresses: *mut c_void, maxaddresses: c_int) -> c_int;
    /// `net.h:39` -- `int net_hostport;` (`menu.c:4945`).
    pub static mut net_hostport: c_int;
    /// `net.h:38` -- `int DEFAULTnet_hostport;`, the port the LAN-config menu
    /// resets to on entry (`menu.c:3697`).
    pub static mut DEFAULTnet_hostport: c_int;
    /// `net.h:106` -- `qboolean slistInProgress;`.
    pub static mut slistInProgress: qboolean;
    /// `net.h:107` -- `qboolean slist_silent;`.
    pub static mut slist_silent: qboolean;
    /// `net.h:108` -- `enum slistScope_e slist_scope;`, an `int`-ranked enum.
    pub static mut slist_scope: c_int;
    /// `net.h:110` -- `size_t hostCacheCount;`.
    pub static mut hostCacheCount: usize;
    /// `net.h:119` -- `qboolean ipv4Available;`.
    pub static mut ipv4Available: qboolean;
    /// `net.h:120` -- `qboolean ipv6Available;`.
    pub static mut ipv6Available: qboolean;

    /* snd_dma.c */
    pub fn S_ExtraUpdate();
    pub fn S_LocalSound(name: *const c_char);

    /* gl_model.c / host_cmd.c file lists */
    pub fn Mod_LoadMapDescription(
        desc: *mut c_char,
        maxchars: usize,
        map: *const c_char,
    ) -> qboolean;
    pub fn Modlist_GetFullName(item: *const filelist_item_t) -> *const c_char;
    pub fn ExtraMaps_GetType(item: *const filelist_item_t) -> c_int;
    pub fn ExtraMaps_IsStart(type_: c_int) -> qboolean;
    pub fn ExtraMaps_GetMessage(item: *const filelist_item_t) -> *const c_char;
    /// `quakedef.h:418` -- `filelist_item_t *modlist;`.
    pub static mut modlist: *mut filelist_item_t;
    /// `quakedef.h:419` -- `filelist_item_t *extralevels;`.
    pub static mut extralevels: *mut filelist_item_t;
    /// `quakedef.h:458` -- `filelist_item_t **extralevels_sorted;`, the
    /// type-ordered index the maps menu walks (`menu.c:2971`).
    pub static mut extralevels_sorted: *mut *mut filelist_item_t;

    /* zone.c */
    pub fn Mem_Alloc(size: usize) -> *mut c_void;
    pub fn Mem_Free(ptr: *const c_void);

    /* sys */
    pub fn Sys_GetPrefPath(org: *const c_char, app: *const c_char) -> *mut c_char;
    pub fn Sys_fopen(path: *const c_char, mode: *const c_char) -> *mut FILE;
    pub fn Sys_FileRead(handle: c_int, dest: *mut c_void, count: c_int) -> c_int;
    /// `quakedef.h:519` -- `qboolean multiuser;` (`menu.c:833`).
    pub static mut multiuser: qboolean;

    /* string helpers (common.c / strl_fn.h) */
    pub fn va(format: *const c_char, ...) -> *mut c_char;
    pub fn q_snprintf(str_: *mut c_char, size: usize, format: *const c_char, ...) -> c_int;
    pub fn q_strlcpy(dst: *mut c_char, src: *const c_char, size: usize) -> usize;
    pub fn q_strcasecmp(s1: *const c_char, s2: *const c_char) -> c_int;
    pub fn q_strcasestr(haystack: *const c_char, needle: *const c_char) -> *mut c_char;
    pub fn q_strsplit(
        str_: *mut c_char,
        sep_set: *const c_char,
        nb_substr: *mut usize,
    ) -> *mut *mut c_char;
    pub fn q_strtrim(str_: *mut c_char) -> *mut c_char;

    /* libc */
    pub fn strlen(s: *const c_char) -> usize;
    pub fn strcmp(s1: *const c_char, s2: *const c_char) -> c_int;
    pub fn strcpy(dst: *mut c_char, src: *const c_char) -> *mut c_char;
    pub fn memset(s: *mut c_void, c: c_int, n: usize) -> *mut c_void;
    pub fn atoi(s: *const c_char) -> c_int;
    pub fn fclose(stream: *mut FILE) -> c_int;
    pub fn fscanf(stream: *mut FILE, format: *const c_char, ...) -> c_int;

    /// // COMPAT: ADR-010 -- `menu.c:1861` and `:1888` round slider values
    /// with the platform `roundf`, and those results are written into cvars
    /// that the config file records. Call through to the same libm the C
    /// build linked rather than using `f32::round`, which is an LLVM
    /// intrinsic and need not agree bit-for-bit.
    ///
    /// `menu.c:1420`, `:1849` and `:2100` also call `fabsf`, but that one is
    /// NOT declared here: IEEE-754 makes `fabs` an exact sign-bit clear with
    /// no rounding, so `f32::abs` is bit-identical by definition, and the
    /// MSVC UCRT publishes `fabsf` only as an inline in `<math.h>`, leaving
    /// no symbol for a call-through to bind to.
    pub fn roundf(x: c_float) -> c_float;
}

/// `Quake/vid.h:72` -- `extern viddef_t vid;` (`menu.c:1439`, `:1591`). The
/// ADR-011 mirror already exists in `cl_parse`; re-exported here rather than
/// declared a second time so there is one `viddef_t` layout in the crate.
pub use crate::cl_parse::{vid, viddef_t};

/// `Quake/quakedef.h:423-452` -- the `maptype_t` enumerators the maps menu
/// compares against. The enum has only non-negative enumerators that fit in
/// `int`, so C ranks it as `int`; `ExtraMaps_GetType` is declared above with
/// an `int` return for the same reason.
pub const MAPTYPE_MOD_START: c_int = 4;
/// See `MAPTYPE_MOD_START`.
pub const MAPTYPE_CUSTOM_ID_START: c_int = 8;
/// See `MAPTYPE_MOD_START`.
pub const MAPTYPE_ID_START: c_int = 12;
/// See `MAPTYPE_MOD_START`.
pub const MAPTYPE_BMODEL: c_int = 20;

/// `Quake/net.h:34` -- `NET_NAMELEN`, the length of one `qhostaddr_t`
/// (`net.h:48`); the LAN-config menu keeps a 16-entry array of them on the
/// stack (`menu.c:3707`).
pub const NET_NAMELEN: usize = 64;

/// `Quake/net.h:108` -- `enum slistScope_e { SLIST_LOOP, SLIST_LAN,
/// SLIST_INTERNET }`. All enumerators are non-negative and fit in `int`, so C
/// ranks the enum as `int`; the menu only ever passes it by value.
pub const SLIST_LAN: c_int = 1;
/// See `SLIST_LAN`.
pub const SLIST_INTERNET: c_int = 2;

/// `Quake/client.h:68` -- `SIGNONS`, the signon-message count a fully
/// connected client has received (`menu.c:2803`).
pub const SIGNONS: c_int = 4;

/// `Quake/keys.h:143`/`:145` -- `MAX_KEYS` and the glue-owned
/// `keybindings[]`. Re-exported from the `keys` mirror rather than declared a
/// second time (ADR-011: one declaration per engine symbol).
/// `Quake/common.h:480` -- `extern qboolean rogue, hipnotic;`, the
/// mission-pack gates the MP game-options menu switches its level and episode
/// tables on (`menu.c:4204`, `:4207`, ...). Re-exported from the ADR-011 owner
/// (`host_cmd.rs:398-399`) so the symbol is declared exactly once.
pub use crate::host_cmd::{hipnotic, rogue};

pub use crate::keys::{keybindings, MAX_KEYS};

extern "C" {
    /* ---------------------------------------------------------------------
     * Menu cvars. Every one is defined outside menu.c and read by it
     * (menu.c:115-137, plus scr_menuscale at :178).
     */
    pub static mut scr_fov: cvar_t;
    pub static mut scr_showfps: cvar_t;
    pub static mut cl_confirmquit: cvar_t;
    pub static mut scr_style: cvar_t;
    pub static mut scr_menuscale: cvar_t;
    pub static mut autoload: cvar_t;
    pub static mut autofastload: cvar_t;
    pub static mut r_rtshadows: cvar_t;
    pub static mut r_particles: cvar_t;
    pub static mut r_oit: cvar_t;
    pub static mut r_enhancedmodels: cvar_t;
    pub static mut r_lerpmodels: cvar_t;
    pub static mut r_lerpmove: cvar_t;
    pub static mut r_lerpturn: cvar_t;
    pub static mut vid_filter: cvar_t;
    pub static mut vid_palettize: cvar_t;
    pub static mut vid_anisotropic: cvar_t;
    pub static mut vid_fsaa: cvar_t;
    pub static mut vid_fsaamode: cvar_t;
    pub static mut host_maxfps: cvar_t;
    pub static mut snd_waterfx: cvar_t;
    pub static mut cl_bob: cvar_t;
    pub static mut cl_rollangle: cvar_t;
    pub static mut v_gunkick: cvar_t;
    pub static mut crosshair: cvar_t;
    pub static mut crosshair_def: cvar_t;

    /* Setup-menu cvars (menu.c:1113-1116). */
    pub static mut cl_name: cvar_t;
    pub static mut hostname: cvar_t;
    pub static mut cl_topcolor: cvar_t;
    pub static mut cl_bottomcolor: cvar_t;

    /* Game/graphics/sound options cvars (menu.c:1413-2600). */
    pub static mut scr_relativescale: cvar_t;
    pub static mut scr_conscale: cvar_t;
    pub static mut scr_sbaralpha: cvar_t;
    pub static mut scr_viewsize: cvar_t;
    pub static mut sensitivity: cvar_t;
    pub static mut m_pitch: cvar_t;
    pub static mut r_drawviewmodel: cvar_t;
    pub static mut r_scale: cvar_t;
    pub static mut r_waterwarp: cvar_t;
    pub static mut cl_alwaysrun: cvar_t;
    pub static mut cl_forwardspeed: cvar_t;
    pub static mut cl_startdemos: cvar_t;
    pub static mut vid_gamma: cvar_t;
    pub static mut vid_contrast: cvar_t;
    pub static mut bgmvolume: cvar_t;
    pub static mut bgm_extmusic: cvar_t;
    pub static mut sfxvolume: cvar_t;
    pub static mut registered: cvar_t;

    /* Multiplayer game-options cvars (menu.c:4000-4400). */
    pub static mut coop: cvar_t;
    pub static mut teamplay: cvar_t;
    pub static mut skill: cvar_t;
    pub static mut fraglimit: cvar_t;
    pub static mut timelimit: cvar_t;
}
