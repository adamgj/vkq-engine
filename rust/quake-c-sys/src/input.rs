//! `Quake/in_sdl_glue.c` declarations (Rust migration Phase 9 M3).
//!
//! ADR-011: engine C symbols are declared only in this crate. `in_sdl.c`
//! defined `in_debugkeys` plus twelve `joy_*` cvars; under
//! `-Duse_rust_platform` that storage moves to `Quake/in_sdl_glue.c` (C keeps
//! ownership so `Cvar_RegisterVariable` still receives stable `cvar_t`
//! addresses, ADR-007) and `quake-platform::input` reaches it through the
//! externs below.
//!
//! The callees the SDL event pump reaches that are `Host_Reraise` wrappers
//! under `-Duse_rust_host`/`-Duse_rust_cvar` (`Key_Event`,
//! `Key_EventWithKeycode`, `Char_Event`, `CL_Disconnect`, `Sys_Quit`, the
//! `scr_conscale` cvar callback, `Cvar_RegisterVariable`) cross through
//! `Host_Guard` trampolines so the status flows back as an `int` (ADR-009
//! rule 3). Everything else here (`VID_*`, `S_*BlockSound`, `Key_TextEntry`,
//! `Con_Mousemove`) cannot raise and is called directly.
//!
//! Symbols `in_sdl.c` merely referenced (`key_dest`, `vid`, `host_parms`,
//! `cl_*`/`m_*`/`joy`-adjacent cvars, `in_mlook`/`in_strafe`/`in_speed`,
//! `noclip_anglehack`, `CL_AngleLocked`, `V_StopPitchDrift`, `scr_fov`,
//! `sv_maxspeed`) are already declared by the sibling modules; `cl`, `cls`
//! and `r_refdef` need `quake-types` and are declared in `quake-platform`.

use crate::{cvar_t, qboolean};
use core::ffi::{c_int, c_void};

extern "C" {
    /* Quake/in_sdl_glue.c data -- in_sdl.c:34-49. */
    pub static mut in_debugkeys: cvar_t;
    pub static mut joy_deadzone_look: cvar_t;
    pub static mut joy_deadzone_move: cvar_t;
    pub static mut joy_outer_threshold_look: cvar_t;
    pub static mut joy_outer_threshold_move: cvar_t;
    pub static mut joy_deadzone_trigger: cvar_t;
    pub static mut joy_sensitivity_yaw: cvar_t;
    pub static mut joy_sensitivity_pitch: cvar_t;
    pub static mut joy_invert: cvar_t;
    pub static mut joy_exponent: cvar_t;
    pub static mut joy_exponent_move: cvar_t;
    pub static mut joy_swapmovelook: cvar_t;
    pub static mut joy_enable: cvar_t;

    /* Guarded callbacks (ADR-009 rule 3) -- Quake/in_sdl_glue.c. */
    /// `Key_Event (key, down)`.
    pub fn InSdl_Glue_KeyEvent(key: c_int, down: qboolean) -> c_int;
    /// `Key_EventWithKeycode (key, down, keycode)`.
    pub fn InSdl_Glue_KeyEventWithKeycode(key: c_int, down: qboolean, keycode: c_int) -> c_int;
    /// `Char_Event (key)`.
    pub fn InSdl_Glue_CharEvent(key: c_int) -> c_int;
    /// `CL_Disconnect ()` (the `SDL_EVENT_QUIT` arm).
    pub fn InSdl_Glue_CLDisconnect() -> c_int;
    /// `Sys_Quit ()` (runs `Host_Shutdown` before `exit (0)`).
    pub fn InSdl_Glue_SysQuit() -> c_int;
    /// `Cvar_FindVar ("scr_conscale")->callback (NULL)` on a window size
    /// change.
    pub fn InSdl_Glue_ConscaleCallback() -> c_int;
    /// `Cvar_RegisterVariable (var)` (`IN_Init`).
    pub fn InSdl_Glue_RegisterVariable(var: *mut cvar_t) -> c_int;

    /* Direct callees that cannot raise. */
    /// `vid.h:88` -- the `SDL_Window *`, opaque here.
    pub fn VID_GetWindow() -> *mut c_void;
    /// `vid.h:102`.
    pub fn VID_FocusGained();
    /// `vid.h:103`.
    pub fn VID_FocusLost();
    /// `q_sound.h:104`.
    pub fn S_BlockSound();
    /// `q_sound.h:105`.
    pub fn S_UnblockSound();
    /// `keys.h:169`.
    pub fn Key_TextEntry() -> qboolean;
    /// `console.h:69`.
    pub fn Con_Mousemove(x: c_int, y: c_int);
}
