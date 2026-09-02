//! `Quake/keys_glue.c` declarations (Rust migration Phase 7 M10b).
//!
//! ADR-011: engine C symbols are declared only in this crate. The glue file
//! owns every C-visible object `Quake/keys.c` used to define -- the console
//! edit line, the key-state and binding tables, `key_dest` and the
//! `keynames[]` name table -- plus the eleven `Host_Guard` trampolines this
//! port needs (ADR-009).
//!
//! ## Why the storage stays C
//!
//! ADR-007: every object below had external linkage in `keys.c`, so the
//! glue keeps the definition and Rust reaches it through these externs. That
//! is not merely conservative for the ones with live C readers --
//! `console.c:37` and `menu.c:114` read `keydown[]`, `console.c:743-752`
//! reads and writes `history_line`, `console.c` drives the whole
//! `key_lines`/`key_linepos`/`key_insert`/`key_blinktime`/`edit_line` edit
//! line while drawing it, and twenty files read `key_dest` -- and it keeps
//! `keynames[]` a genuinely separate object from the oracle's copy in the
//! differential build.
//!
//! `cls` is mirror-typed and so is declared in `quake-capi/src/keys.rs`,
//! which can name `quake_types`; this crate has no `[dependencies]` (the
//! same finding `quake-c-sys/src/sv_user.rs` records).

use crate::{cvar_t, qboolean, FILE};
use core::ffi::{c_char, c_int};

/// `keys.h:143` -- `#define MAX_KEYS 256`.
pub const MAX_KEYS: usize = 256;
/// `keys.h:145` -- `#define MAXCMDLINE 256`.
pub const MAXCMDLINE: usize = 256;
/// `keys.h:158` -- `#define CMDLINES 1024`.
pub const CMDLINES: usize = 1024;

/// `keys.c:48-52` -- `typedef struct {const char *name; int keynum;}
/// keyname_t;`. The type is file-local in `keys.c`, so it is mirrored here
/// next to the `keynames` extern rather than pulled from `quake-types`.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(non_camel_case_types)]
pub struct keyname_t {
    pub name: *const c_char,
    pub keynum: c_int,
}

extern "C" {
    /* Quake/keys_glue.c data (keys.c:31-46, :54-142, :539) */

    pub static mut key_lines: [[c_char; MAXCMDLINE]; CMDLINES];
    pub static mut key_tabhint: [c_char; MAXCMDLINE];
    pub static mut key_linepos: c_int;
    pub static mut key_insert: c_int;
    pub static mut key_blinktime: f64;
    pub static mut edit_line: c_int;
    pub static mut history_line: c_int;
    /// `keys.h:154` -- `keydest_t key_dest`. Declared `c_int` to agree with
    /// the existing mirrors in `cl_demo.rs`, `host_cmd.rs`, `sv_user.rs` and
    /// `net_dgrm_orch.rs`; `key_game == 0`.
    pub static mut key_dest: c_int;
    pub static mut keybindings: [*mut c_char; MAX_KEYS];
    pub static mut consolekeys: [qboolean; MAX_KEYS];
    pub static mut menubound: [qboolean; MAX_KEYS];
    pub static mut keydown: [qboolean; MAX_KEYS];
    pub static mut chat_team: qboolean;

    /// `keys.c:54` -- the `{NULL, 0}`-terminated key name table. Declared
    /// with a zero length because only the base address is used; the walk
    /// stops on a NULL `name`, exactly like `keys.c:606` and `:641`.
    pub static keynames: [keyname_t; 0];

    /* ---------------------------------------------------------------------
     * Quake/keys_glue.c ADR-009 trampolines -- each returns a Host_Guard
     * status that must reach the glue's Host_Reraise frame untouched.
     */

    /// `keys.c:285` -- `SCR_UpdateScreen (false)`.
    pub fn Keys_Glue_UpdateScreen() -> c_int;
    /// `keys.c:289` etc -- `Con_TabComplete (mode)`.
    pub fn Keys_Glue_TabComplete(mode: c_int) -> c_int;
    /// `keys.c:352`, `:357` -- `Con_Scroll (lines)`.
    pub fn Keys_Glue_Scroll(lines: c_int) -> c_int;
    /// `keys.c:339`, `:349` -- `Con_ForceMouseMove ()`.
    pub fn Keys_Glue_ForceMouseMove() -> c_int;
    /// `keys.c:451` -- `Con_SelectAll ()`.
    pub fn Keys_Glue_SelectAll() -> c_int;
    /// `keys.c:437`, `:461` -- `Con_CopySelectionToClipboard ()`; the
    /// `qboolean` result is written through `out` only when the guard is OK.
    pub fn Keys_Glue_CopySelectionToClipboard(out: *mut qboolean) -> c_int;
    /// `keys.c:1077` -- `Con_ToggleConsole_f ()`.
    pub fn Keys_Glue_ToggleConsole() -> c_int;
    /// `keys.c:1087`, `:1182` -- `M_Keydown (key)`.
    pub fn Keys_Glue_MenuKeydown(key: c_int) -> c_int;
    /// `keys.c:1226` -- `M_Charinput (key)`.
    pub fn Keys_Glue_MenuCharinput(key: c_int) -> c_int;
    /// `keys.c:1091`, `:1145` -- `M_ToggleMenu_f ()`.
    pub fn Keys_Glue_ToggleMenu() -> c_int;
    /// `keys.c:1040` -- `VID_Toggle ()`.
    pub fn Keys_Glue_VidToggle() -> c_int;

    /* ---------------------------------------------------------------------
     * Non-raising C callees keys.c reaches that the committed bindings do
     * not carry. None of these can longjmp: the clipboard shim is SDL plus
     * Mem_Alloc, the console query functions only read state, the menu
     * predicates only read `m_state`, and `IN_*` is SDL plus Con_Printf.
     */

    /// `platform.h:34` -- `char *PL_GetClipboardData (void)`.
    pub fn PL_GetClipboardData() -> *mut c_char;
    /// `menu.h:67` -- `qboolean M_TextEntry (void)`.
    pub fn M_TextEntry() -> qboolean;
    /// `menu.h:68` -- `qboolean M_WaitingForKeyBinding (void)`.
    pub fn M_WaitingForKeyBinding() -> qboolean;
    /// `input.h:40` -- `void IN_UpdateInputMode (void)`.
    pub fn IN_UpdateInputMode();
    /// `input.h:50` -- `void IN_Activate (void)`.
    pub fn IN_Activate();
    /// `input.h:56` -- `void IN_DeactivateForConsole (void)`.
    pub fn IN_DeactivateForConsole();

    /* Quake/console.c state the console edit line reads (declared in keys.c
    itself at :245-246, and in console.h for the last three) */
    pub static mut con_text: *mut c_char;
    pub static mut key_tabpartial: [c_char; MAXCMDLINE];
    pub static mut con_current: c_int;
    pub static mut con_linewidth: c_int;
    pub static mut con_vislines: c_int;
    pub static mut con_totallines: c_int;
    pub static mut con_backscroll: c_int;
    pub static mut con_forcedup: qboolean;

    /// `glquake.h` -- the framebuffer height `keys.c:334` clamps against.
    pub static mut glheight: c_int;
    /// `menu.h:58` -- `qboolean m_is_quitting`.
    pub static mut m_is_quitting: qboolean;
    /// `host.c` -- `double realtime`.
    pub static mut realtime: f64;
    /// `common.c` -- `cvar_t cfg_unbindall`.
    pub static mut cfg_unbindall: cvar_t;

    /* libc / common.c helpers */
    pub fn q_snprintf(str_: *mut c_char, size: usize, format: *const c_char, ...) -> c_int;
    pub fn q_strcasecmp(s1: *const c_char, s2: *const c_char) -> c_int;
    pub fn q_strdup(str_: *const c_char) -> *mut c_char;
    pub fn va(format: *const c_char, ...) -> *mut c_char;
    pub fn fprintf(stream: *mut FILE, fmt: *const c_char, ...) -> c_int;
    pub fn ungetc(c: c_int, stream: *mut FILE) -> c_int;
}
