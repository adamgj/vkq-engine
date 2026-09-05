//! `Quake/sv_phys_glue.c` declarations (Rust migration Phase 7 M4).
//!
//! ADR-011: engine C symbols are declared only in this crate. The glue file
//! owns the twelve `sv_phys.c` cvars, `sv_analyticphysics_frame`, the
//! `sv_speeds_*` counters host.c reads, and every `Host_Guard` call site
//! reachable from `quake-capi`'s `sv_phys` module (ADR-009).
//!
//! `SV_CheckAllEnts`, `SV_CheckVelocity`, `SV_CheckWaterTransition` and
//! `SV_Physics` are `Quake/sv_phys_glue.c`'s four re-raising wrappers over
//! that module's `quake_rs_sv_*` cores; nothing in `quake-capi` calls them by
//! their plain names (ADR-009), so they are not declared here.
//! `World_Glue_AssertFailed`, `World_Glue_EdictNum`, `World_Glue_NumForEdict`
//! and `World_Glue_QcvmIsClient` (`Quake/world_glue.c`) already cover this
//! file's `assert_always` sites, its `EDICT_NUM` / `NUM_FOR_EDICT` uses and
//! the `qcvm == &cl.qcvm` test, and are declared in `quake_c_sys::world`.
//!
//! Engine aggregates are passed as `c_void` pointers here rather than pulling
//! `quake-types` into this crate; `quake-capi`'s `sv_phys` module casts them
//! to the ADR-011 mirrors at the call sites.

use crate::{cvar_t, qboolean};
use core::ffi::{c_char, c_float, c_int, c_void};

/// C: `typedef void (*ED_AllocHook_func) (edict_t *allocated_ed)`
/// (`Quake/progs.h:132`). Renamed to Rust casing; the C typedef has no
/// linkage of its own.
pub type EdAllocHookFunc = Option<unsafe extern "C" fn(allocated_ed: *mut c_void)>;

extern "C" {
    /* Quake/sv_phys_glue.c data (sv_phys.c:44-56, :345-346, :705-707) */
    pub static mut sv_friction: cvar_t;
    pub static mut sv_stopspeed: cvar_t;
    pub static mut sv_gravity: cvar_t;
    pub static mut sv_maxvelocity: cvar_t;
    pub static mut sv_nostep: cvar_t;
    pub static mut sv_freezenonclients: cvar_t;
    pub static mut sv_gameplayfix_spawnbeforethinks: cvar_t;
    pub static mut sv_gameplayfix_bouncedownslopes: cvar_t;
    pub static mut sv_fastpushmove: cvar_t;
    pub static mut sv_pushgrid: cvar_t;
    pub static mut sv_analyticphysics: cvar_t;
    pub static mut sv_gameplayfix_elevators: cvar_t;

    /// `sv_analyticphysics` latched once per `SV_Physics` tick; QC can flip
    /// the cvar mid-tick, so the latch is the value the whole tick uses.
    pub static mut sv_analyticphysics_frame: qboolean;

    /* host.c reads these after every server frame (sv_phys.c:345-346) */
    pub static mut sv_speeds_think_ms: f64;
    pub static mut sv_speeds_pusher_ms: f64;
    pub static mut sv_speeds_build_ms: f64;
    pub static mut sv_speeds_thinks: c_int;
    pub static mut sv_speeds_pushers: c_int;
    pub static mut sv_speeds_pushables: c_int;
    pub static mut sv_speeds_grid_entries: c_int;

    /// `Quake/host.c:70` -- `extern cvar_t sv_speeds;` at `sv_phys.c:343`.
    pub static mut sv_speeds: cvar_t;

    /* Quake/sv_phys_glue.c guards -- each returns a Host_Guard status */

    /// `Con_DPrintf` behind `PR_GetString`, which can Host_Error
    /// (pr_edict_arena.c:315). sv_phys.c:318 / :323.
    pub fn SvPhys_Glue_WarnNanVelocity(ent: *mut c_void) -> c_int;
    pub fn SvPhys_Glue_WarnNanOrigin(ent: *mut c_void) -> c_int;

    /// Both think dispatches: `SV_RunThink` (sv_phys.c:369-372, `time` is the
    /// clamped think time) and `SV_Physics_Pusher` (sv_phys.c:1610-1613,
    /// `time` is `qcvm->time`). They differ only in the stamped time.
    pub fn SvPhys_Glue_CallThink(ent: *mut c_void, time: c_float) -> c_int;

    /// One `SV_Impact` touch dispatch (sv_phys.c:424-426, :434-436). It sets
    /// `self`/`other` only: `time` is stamped once by the caller before the
    /// first dispatch, and QC may change it in between.
    pub fn SvPhys_Glue_ImpactTouch(self_: *mut c_void, other: *mut c_void) -> c_int;

    /// The `SV_PushEntityTo` un-embed message (sv_phys.c:1254). Its
    /// `NUM_FOR_EDICT` arguments Host_Error on a bad pointer, so the whole
    /// line is guarded.
    pub fn SvPhys_Glue_DPrintUnembedded(ent: *mut c_void, ground: *mut c_void) -> c_int;

    /// `SV_PushMove`'s blocked dispatch (sv_phys.c:1559-1561).
    pub fn SvPhys_Glue_CallBlocked(pusher: *mut c_void, obstacle: *mut c_void) -> c_int;

    /// sv_phys.c:2007-2009 / :2065-2067. Neither sets `->other`.
    pub fn SvPhys_Glue_CallPlayerPreThink(ent: *mut c_void, time: c_float) -> c_int;
    pub fn SvPhys_Glue_CallPlayerPostThink(ent: *mut c_void, time: c_float) -> c_int;

    /// The `StartFrame` dispatch (sv_phys.c:2334-2339). The
    /// `pr_global_struct->StartFrame` test itself stays on the Rust side.
    pub fn SvPhys_Glue_CallStartFrame(time: c_float) -> c_int;

    /// `SV_StartSound (ent, NULL, channel, sample, volume, attenuation)` --
    /// all three sv_phys.c call sites (:2139, :2148, :2270) pass a NULL
    /// origin. `SV_StartSound` Host_Errors three ways (sv_main.c:1282, :1290,
    /// :1293).
    pub fn SvPhys_Glue_StartSound(
        ent: *mut c_void,
        channel: c_int,
        sample: *const c_char,
        volume: c_int,
        attenuation: c_float,
    ) -> c_int;

    /// The two `Host_EndGame` "bad movetype" sites (sv_phys.c:2055, :2429).
    /// Separate helpers because the format strings differ.
    pub fn SvPhys_Glue_EndGameBadClientMovetype(movetype: c_int) -> c_int;
    pub fn SvPhys_Glue_EndGameBadMovetype(movetype: c_int) -> c_int;

    /* Quake/sv_phys_glue.c non-raising shims */

    /// sv_phys.c:298 / :1655 and :1669 / :1676.
    pub fn SvPhys_Glue_PrintInvalidPosition();
    pub fn SvPhys_Glue_DPrintUnstuck();
    pub fn SvPhys_Glue_DPrintPlayerStuck();

    /// `qcvm == &sv.qcvm`. Kept as an accessor: sv_phys.c's port predates the
    /// M6 sv/svs move and reaching the mirror directly is M8's business.
    pub fn SvPhys_Glue_QcvmIsServer() -> c_int;

    /// `svs.maxclients` and the two `svs.clients[num - 1]` flags
    /// (sv_phys.c:1996, :1999). Kept as an accessor for the same reason as
    /// `SvPhys_Glue_QcvmIsServer` above.
    pub fn SvPhys_Glue_MaxClients() -> c_int;
    pub fn SvPhys_Glue_ClientActive(num: c_int) -> c_int;
    pub fn SvPhys_Glue_ClientKnownToQc(num: c_int) -> c_int;

    /// `sv_player` (sv_phys.c:1893).
    pub fn SvPhys_Glue_SvPlayer() -> *mut c_void;

    /* Engine C symbols sv_phys.c calls directly; none of them can raise. */

    /// C: `int ED_FindFieldOffset (const char *name)` (`Quake/pr_edict.c:141`)
    /// -- a hash lookup over `fielddefs_map` that returns -1 when the field is
    /// absent.
    pub fn ED_FindFieldOffset(name: *const c_char) -> c_int;

    /// C: `eval_t *GetEdictFieldValue (edict_t *ed, int fldofs)`
    /// (`Quake/pr_edict.c:95`) -- returns NULL for a negative offset. `eval_t`
    /// is a union whose first member is `float _float`, and `SV_EntGravity`
    /// (sv_phys.c:665) reads only that member, so the result is typed as a
    /// float pointer instead of mirroring the union.
    pub fn GetEdictFieldValue(ed: *mut c_void, fldofs: c_int) -> *mut c_float;

    /// C: `ED_AllocHook_func ED_AllocSetHook (ED_AllocHook_func alloc_hook)`
    /// (`Quake/progs.h:136`, `Quake/pr_edict_arena.c:33`) -- swaps the hook and
    /// returns the previous one. `ED_Alloc` calls the hook as its last
    /// statement, after every `Host_Error` it can reach, so no longjmp ever
    /// unwinds the Rust hook frame `SV_Physics` installs.
    pub fn ED_AllocSetHook(alloc_hook: EdAllocHookFunc) -> EdAllocHookFunc;
}
