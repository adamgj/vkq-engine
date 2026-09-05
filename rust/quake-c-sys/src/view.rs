//! `Quake/view_glue.c` declarations (Rust migration Phase 7 M7, T7.2a).
//!
//! ADR-011: engine C symbols are declared only in this crate. `Quake/view.c`
//! defined 30 cvars plus `v_punchangles`/`v_punchangles_times`/`v_blend`;
//! under `-Duse_rust_host` that storage moves to `Quake/view_glue.c` (C keeps
//! ownership so `Cvar_RegisterVariable` still receives stable `cvar_t`
//! addresses and `menu.c`/`gl_screen.c`/`cl_parse.c`/`gl_vidsdl.c` keep
//! resolving the same plain names), and Rust reaches it through the externs
//! below -- the `Quake/sv_user_glue.c` arrangement, unchanged.
//!
//! `cl_rollspeed`/`cl_rollangle` are here too even though `sv_user.c` reads
//! them: `quake-c-sys/src/sv_user.rs` only declared `V_CalcRoll`, never the
//! pair, so there is no duplicate declaration.
//!
//! Symbols `view.c` merely referenced (`scr_viewsize`, `cl_forwardspeed`,
//! `lookspring`, `chase_active`, `needs_relink`, `noclip_anglehack`,
//! `con_forcedup`, `render_warp`, `render_scale`,
//! `r_trace_line_cache_counter`, `CL_AngleLocked`) stay owned by their own
//! translation units and are declared, not defined, here.
//! `chase_active`/`Chase_UpdateForDrawing` are NOT among them: `chase.c` is
//! ported by the same task, so `quake-capi`'s `view` module calls its `chase`
//! sibling directly and reads `crate::chase::chase_active`.
//!
//! Per the `sv_user.rs` finding, this crate has no `[dependencies]` and
//! therefore cannot name the ADR-011 mirrors; `r_refdef`, `cl` and `cls` are
//! declared in `quake-capi/src/view.rs` instead, where `quake-types` is in
//! scope.

use crate::qboolean;
use core::ffi::{c_float, c_int, c_uint};

extern "C" {
    /* Quake/view_glue.c data -- view.c:35-71, :147-148, :75-76, :262. */
    pub static mut scr_ofsx: crate::cvar_t;
    pub static mut scr_ofsy: crate::cvar_t;
    pub static mut scr_ofsz: crate::cvar_t;

    pub static mut cl_rollspeed: crate::cvar_t;
    pub static mut cl_rollangle: crate::cvar_t;

    pub static mut cl_bob: crate::cvar_t;
    pub static mut cl_bobcycle: crate::cvar_t;
    pub static mut cl_bobup: crate::cvar_t;

    pub static mut v_kicktime: crate::cvar_t;
    pub static mut v_kickroll: crate::cvar_t;
    pub static mut v_kickpitch: crate::cvar_t;
    pub static mut v_gunkick: crate::cvar_t;

    pub static mut v_autopitch: crate::cvar_t;

    pub static mut v_iyaw_cycle: crate::cvar_t;
    pub static mut v_iroll_cycle: crate::cvar_t;
    pub static mut v_ipitch_cycle: crate::cvar_t;
    pub static mut v_iyaw_level: crate::cvar_t;
    pub static mut v_iroll_level: crate::cvar_t;
    pub static mut v_ipitch_level: crate::cvar_t;

    pub static mut v_idlescale: crate::cvar_t;

    pub static mut crosshair: crate::cvar_t;
    pub static mut crosshair_def: crate::cvar_t;

    pub static mut gl_cshiftpercent: crate::cvar_t;
    pub static mut gl_cshiftpercent_contents: crate::cvar_t;
    pub static mut gl_cshiftpercent_damage: crate::cvar_t;
    pub static mut gl_cshiftpercent_bonus: crate::cvar_t;
    pub static mut gl_cshiftpercent_powerup: crate::cvar_t;

    pub static mut r_viewmodel_quake: crate::cvar_t;

    pub static mut v_centermove: crate::cvar_t;
    pub static mut v_centerspeed: crate::cvar_t;

    /// `view.c:75` -- 0 is current, 1 is previous.
    pub static mut v_punchangles: [[c_float; 3]; 2];
    /// `view.c:76`.
    pub static mut v_punchangles_times: [f64; 2];
    /// `view.c:262` -- rgba 0-255.
    pub static mut v_blend: [u8; 4];

    /* Owned elsewhere; referenced by view.c. */

    /// `gl_screen.c`.
    pub static mut scr_viewsize: crate::cvar_t;
    /// `cl_input.c` (`Quake/cl_input_glue.c` under `-Duse_rust_host`).
    pub static mut cl_forwardspeed: crate::cvar_t;
    /// `cl_input.c` (`Quake/cl_input_glue.c` under `-Duse_rust_host`).
    pub static mut lookspring: crate::cvar_t;
    /// `cl_main.c` -- set when an entity update arrived after the relink.
    pub static mut needs_relink: qboolean;
    /// `cl_main.c`.
    pub static mut noclip_anglehack: qboolean;
    /// `console.c`.
    pub static mut con_forcedup: qboolean;
    /// `gl_screen.c`.
    pub static mut render_warp: qboolean;
    /// `gl_screen.c`.
    pub static mut render_scale: c_int;
    /// `gl_rmain.c` -- bumped by `InvalidateTraceLineCache()`
    /// (`glquake.h:134`, a `++` macro, not a call).
    pub static mut r_trace_line_cache_counter: c_int;

    /// `cl_input.c`. Non-raising (reads `in_mlook`/`cl.fixangle_time`).
    pub fn CL_AngleLocked() -> qboolean;

    /// C runtime `atoi` (`view.c:355-358`). Declared here as well as in
    /// `sv_main.rs` so this module is self-contained; both name the same
    /// symbol.
    pub fn atoi(nptr: *const core::ffi::c_char) -> c_int;

    /// `net_msg.c` (or `quake-capi`'s `net` module under `-Duse_rust_net`).
    /// Returns -1 and sets `msg_badread` on underflow; never longjmps.
    /// Declared here as well as in `sv_user.rs` so this module's `view.c`
    /// port is self-contained; both declarations name the same C symbol.
    pub fn MSG_ReadByte() -> c_int;
    /// `Quake/common.h`: `float MSG_ReadCoord (unsigned int flags);`.
    pub fn MSG_ReadCoord(flags: c_uint) -> c_float;

    /* ---------------------------------------------------------------------
     * ADR-009 trampolines defined by Quake/view_glue.c (and mirrored, for the
     * ctest oracle link, by rust/quake-ctest/stubs/view_ref.c). Each returns
     * a Host_Guard status (0 = returned normally, 1/2 = a jump was caught and
     * must be re-raised from a C frame).
     */

    /// Wraps one `Cvar_RegisterVariable`, which is itself a `Host_Reraise`
    /// wrapper under `-Duse_rust_cvar`.
    pub fn View_Glue_RegisterVariable(var: *mut crate::cvar_t) -> c_int;
    /// Wraps the three `Cmd_AddCommand` calls of `V_Init` (`view.c:929-931`).
    pub fn View_Glue_AddCommands() -> c_int;
    /// Wraps `CL_RelinkEntities` (`view.c:924`), which reaches `Mem_Realloc`,
    /// `R_AddEfrags` and the `PScript_*` particle system.
    pub fn View_Glue_RelinkEntities() -> c_int;
    /// Wraps `R_RenderView` (`view.c:927`).
    pub fn View_Glue_RenderView(
        use_tasks: qboolean,
        begin_rendering_task: u64,
        setup_frame_task: u64,
        draw_done_task: u64,
    ) -> c_int;
}
