//! `Quake/pr_cmds_sv_glue.c` declarations, plus the engine symbols the
//! server-coupled builtins reach directly (Rust migration Phase 7 M5,
//! Groups A/B/C).
//!
//! ADR-011: engine C symbols are declared only in this crate. `pr_cmds.c` and
//! `pr_ext.c` are still compiled in every configuration (Pattern C flips one
//! table slot at a time), so the two cvar objects below keep their existing
//! storage there and in `host.c`; only the guarded seams and the `server_t`
//! reads need a glue file.
//!
//! `Host_Reraise` is deliberately absent (ADR-009): every helper here that can
//! raise returns a `Host_Guard` status, and `pr_cmds_glue.c`'s `PRBI_Raise`
//! re-issues the jump from the C frame.
//!
//! Engine aggregates are passed as `c_void` pointers, matching this crate's
//! `sv_phys` module; `quake-capi` casts them to the ADR-011 mirrors at the
//! call sites.

use crate::cvar_t;
use core::ffi::{c_char, c_double, c_float, c_int, c_void};

extern "C" {
    /* Engine cvar objects the builtins read directly. */

    /// C: `cvar_t sv_aim` (`Quake/pr_cmds.c:1493`), registered by
    /// `sv_main.c:1172`.
    pub static mut sv_aim: cvar_t;

    /// C: `cvar_t teamplay` (`Quake/host.c:84`).
    pub static mut teamplay: cvar_t;

    /* Engine entry points that cannot longjmp. */

    /// C: `string_t PR_SetEngineString (const char *s)`
    /// (`Quake/pr_edict_arena.c`). Only `Sys_Error`s, never `Host_Error`s, so
    /// it needs no guard.
    pub fn PR_SetEngineString(s: *const c_char) -> c_int;

    /// C: `mleaf_t *Mod_PointInLeaf (float *p, qmodel_t *model)`
    /// (`Quake/gl_model.h:740`).
    pub fn Mod_PointInLeaf(p: *mut c_float, model: *mut c_void) -> *mut c_void;

    /// C: `byte *Mod_LeafPVS (mleaf_t *leaf, qmodel_t *model)`
    /// (`Quake/gl_model.h:741`).
    pub fn Mod_LeafPVS(leaf: *mut c_void, model: *mut c_void) -> *mut u8;

    /* Quake/pr_cmds_sv_glue.c helpers -- each returns a Host_Guard status. */

    /// `SetMinMaxSize`'s `PR_RunError ("backwards mins/maxs")`
    /// (`Quake/pr_cmds.c:241`).
    pub fn PRBI_SvGlue_RunErrorBackwardsMinsMaxs() -> c_int;

    /// The whole `PF_setmodel` precache lookup (`Quake/pr_cmds.c:346-370`):
    /// `G_STRING`, the scan, the `Con_Warning`, `SV_Precache_Model` and the
    /// `PR_RunError ("no precache: %s")` fallback. Returns the name pointer
    /// (`*check`, after any precache), the index and `sv.models[index]`.
    pub fn PRBI_SvGlue_SetModelLookup(
        handle: c_int,
        out_name: *mut *const c_char,
        out_index: *mut c_int,
        out_model: *mut *mut c_void,
    ) -> c_int;

    /// The `traceline`/`tracebox` NAN `Con_Warning`
    /// (`Quake/pr_cmds.c:755`, `Quake/pr_ext.c:1851`); its `NUM_FOR_EDICT`
    /// argument Host_Errors on a bad pointer.
    pub fn PRBI_SvGlue_WarnNanTrace(v1: *mut c_float, v2: *mut c_float, ent: *mut c_void) -> c_int;

    /* Quake/pr_cmds_sv_glue.c non-raising shims: `server_t` (server.h:59-60)
    has no ADR-011 mirror in Phase 7. */

    pub fn PRBI_SvGlue_SvLastCheck() -> c_int;
    pub fn PRBI_SvGlue_SetSvLastCheck(value: c_int);
    pub fn PRBI_SvGlue_SvLastCheckTime() -> c_double;
    pub fn PRBI_SvGlue_SetSvLastCheckTime(value: c_double);
}
