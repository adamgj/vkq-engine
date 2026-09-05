//! `Quake/pr_cmds_sv_fx_glue.c` declarations, plus the one non-raising engine
//! entry point (`SV_StartParticle`) the Group E world-effect builtins call
//! directly (Rust migration Phase 7 M5 wave 2, Group E: world-effect
//! builtins).
//!
//! ADR-011: engine C symbols are declared only in this crate. `pr_cmds.c`
//! stays compiled in every configuration (Pattern C flips one table slot at
//! a time), so `server_t sv` / `server_static_t svs` keep their existing
//! storage there; they have no ADR-011 mirror in Phase 7 (same situation
//! `progs_builtins_sv.rs` documents for `sv.lastcheck`), so every builtin
//! that touches them is kept whole behind one of the glue helpers below
//! instead of being partially ported.
//!
//! `Host_Reraise` is deliberately absent (ADR-009): every guarded helper here
//! returns a `Host_Guard` status, and `pr_cmds_glue.c`'s `PRBI_Raise`
//! re-issues the jump from the C frame.
//!
//! Engine aggregates (`edict_t *`) are passed as `c_void` pointers, matching
//! `progs_builtins_sv.rs`; `quake-capi` casts them to the ADR-011 mirror at
//! the call sites. `float` vectors are passed as raw `*mut c_float` pointing
//! into the ambient qcvm's globals block (ADR-008), exactly like
//! `progs_builtins_sv.rs`'s link builtins.

use core::ffi::{c_float, c_int, c_void};

extern "C" {
    /// C: `void SV_StartParticle (vec3_t org, vec3_t dir, int color, int count)`
    /// (`Quake/server.h:337`, `Quake/sv_main.c:1231`). Never raises: it
    /// clamps `org`/`dir`/`count` and writes network bytes directly, so
    /// `PF_particle` (`Quake/pr_cmds.c:614-625`) calls it with no guard.
    pub fn SV_StartParticle(org: *mut c_float, dir: *mut c_float, color: c_int, count: c_int);

    /* Quake/pr_cmds_sv_fx_glue.c helpers -- each returns a Host_Guard status. */

    /// The whole `PF_sound` body (`Quake/pr_cmds.c:692-713`): the `G_STRING`
    /// (`PR_GetString`) fetch, the empty-string `PR_RunWarning`, and
    /// `SV_StartSound` (Host_Errors on a bad volume/attenuation/channel, or
    /// on a bad `entity` via its internal `NUM_FOR_EDICT`).
    pub fn PRBI_FxGlue_Sound(
        entity: *mut c_void,
        channel: c_int,
        sample_handle: c_int,
        volume: c_int,
        attenuation: c_float,
    ) -> c_int;

    /// The whole `PF_sv_ambientsound` body (`Quake/pr_cmds.c:633-675`): the
    /// `sv.sound_precache` scan (no ADR-011 mirror), the "no precache"
    /// `Con_Printf`, and the `sv.ambientsounds` growth, which `PR_RunError`s
    /// on a failed `Mem_Realloc`.
    pub fn PRBI_FxGlue_AmbientSound(
        pos: *mut c_float,
        sample_handle: c_int,
        vol: c_float,
        attenuation: c_float,
    ) -> c_int;

    /// The whole `PF_sv_lightstyle` body (`Quake/pr_cmds.c:1364-1405`): the
    /// bounds check's `Con_DWarning`, the `sv.lightstyles` write, and the
    /// per-client `svs.clients` broadcast loop (none of which have an
    /// ADR-011 mirror). Never actually raises; guarded anyway because the
    /// `G_STRING` fetch can.
    pub fn PRBI_FxGlue_LightStyle(style: c_int, val_handle: c_int) -> c_int;

    /// The whole `PF_sv_makestatic` body (`Quake/pr_cmds.c:1708-1734`): the
    /// `sv.static_entities` growth (`PR_RunError` on a failed `Mem_Realloc`),
    /// `SV_BuildEntityState`, and `ED_Free` (which can itself `Host_Error`).
    pub fn PRBI_FxGlue_MakeStatic(ent: *mut c_void) -> c_int;

    /// The whole `PF_sv_setspawnparms` body (`Quake/pr_cmds.c:1743-1759`):
    /// `NUM_FOR_EDICT` (Host_Errors on a bad edict) and the
    /// `"Entity is not a client"` `PR_RunError`.
    pub fn PRBI_FxGlue_SetSpawnParms(ent: *mut c_void) -> c_int;

    /// The `G_STRING` fetch and `Cbuf_AddText (va (...))` half of
    /// `PF_sv_changelevel` (`Quake/pr_cmds.c:1766-1777`); the
    /// `svs.changelevel_issued` check-and-set is done in Rust via the
    /// existing `PRBI_Glue_ChangelevelIssued`.
    pub fn PRBI_FxGlue_ChangeLevel(level_handle: c_int) -> c_int;

    /// The whole `PF_sv_precache_sound` body (`Quake/pr_cmds.c:1188-1198`):
    /// the inlined `PR_CheckEmptyString` (`"Bad string"`, `pr_cmds.c:1148`)
    /// and `SV_Precache_Sound`'s `"PF_precache_sound: overflow"`.
    pub fn PRBI_FxGlue_PrecacheSound(handle: c_int) -> c_int;

    /// The whole `PF_sv_precache_model` body (`Quake/pr_cmds.c:1225-1259`):
    /// its own precache scan -- not `SV_Precache_Model`, whose warning
    /// behaviour differs (`sv.model_precache` has no ADR-011 mirror),
    /// `Mod_ForName`, and the `"PF_precache_model: overflow"` `PR_RunError`.
    pub fn PRBI_FxGlue_PrecacheModel(handle: c_int) -> c_int;

    /// The whole `PF_sv_localsound` body (`Quake/pr_cmds.c:1857-1870`):
    /// `NUM_FOR_EDICT` (Host_Errors on a bad edict), the non-client
    /// `Con_Printf`, and `SV_LocalSound`.
    pub fn PRBI_FxGlue_LocalSound(ent: *mut c_void, sample_handle: c_int) -> c_int;
}
