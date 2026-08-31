//! `Quake/pr_cmds_cl_glue.c` declarations, plus the engine symbols the
//! client-coupled builtins reach directly (Rust migration Phase 7 M5,
//! Group F: `PF_cl_sound`, `PF_cl_ambientsound`, `PF_cl_precache_sound`,
//! `PF_cl_makestatic`, `PF_cl_particle`).
//!
//! ADR-011: engine C symbols are declared only in this crate.
//!
//! `S_PrecacheSound`, `S_StartSound`, `S_StaticSound`,
//! `PScript_RunParticleEffect`, `PScript_RunParticleEffectTypeString` and
//! `R_RunParticleEffect` need no guard: none of them reach `Host_Error`, only
//! `S_FindName`'s pathological-path `Sys_Error`, which is fatal and not a
//! `Host_Guard`-caught longjmp (ADR-009 only concerns `Host_Error` /
//! `PR_RunError` / `Host_EndGame`).
//!
//! `Host_Reraise` is deliberately absent (ADR-009): every helper here that can
//! raise returns a `Host_Guard` status, and `pr_cmds_glue.c`'s `PRBI_Raise`
//! re-issues the jump from the C frame.

use core::ffi::{c_char, c_float, c_int, c_void};

extern "C" {
    /* Engine entry points that cannot longjmp -- called directly from Rust. */

    /// C: `sfxcache_t *S_PrecacheSound (const char *sample)` (`Quake/snd_dma.c`).
    pub fn S_PrecacheSound(sample: *const c_char) -> *mut c_void;

    /// C: `void S_StartSound (int entnum, int entchannel, sfxcache_t *sfx,
    /// vec3_t origin, float fvol, float attenuation)` (`Quake/snd_dma.c`).
    pub fn S_StartSound(
        entnum: c_int,
        entchannel: c_int,
        sfx: *mut c_void,
        origin: *mut c_float,
        fvol: c_float,
        attenuation: c_float,
    );

    /// C: `void S_StaticSound (sfx_t *sfx, vec3_t origin, int vol, float
    /// attenuation)` (`Quake/snd_dma.c:619`). Note `vol` is `int`, not
    /// `float` -- `PF_cl_ambientsound` converts implicitly at the call site.
    pub fn S_StaticSound(sfx: *mut c_void, origin: *mut c_float, vol: c_int, attenuation: c_float);

    /// C: `qboolean PScript_RunParticleEffectTypeString (vec3_t org, vec3_t
    /// dir, float count, const char *name)` (`Quake/r_part_fte.c`).
    pub fn PScript_RunParticleEffectTypeString(
        org: *mut c_float,
        dir: *mut c_float,
        count: c_float,
        name: *const c_char,
    ) -> c_int;

    /// C: `qboolean PScript_RunParticleEffect (vec3_t org, vec3_t dir, int
    /// color, int count)` (`Quake/r_part_fte.c`).
    pub fn PScript_RunParticleEffect(
        org: *mut c_float,
        dir: *mut c_float,
        color: c_int,
        count: c_int,
    ) -> c_int;

    /// C: `void R_RunParticleEffect (vec3_t org, vec3_t dir, int color, int
    /// count)` (`Quake/r_part.c`).
    pub fn R_RunParticleEffect(org: *mut c_float, dir: *mut c_float, color: c_int, count: c_int);

    /* Quake/pr_cmds_cl_glue.c helpers -- each returns a Host_Guard status. */

    /// `PR_GetString`'s out-of-range-handle `Host_Error`
    /// (`Quake/pr_edict_arena.c`), used by every `G_STRING` in this group's
    /// builtins.
    pub fn PRBI_ClGlue_GetString(handle: c_int, out: *mut *const c_char) -> c_int;

    /// `PR_CheckEmptyString`'s `PR_RunError ("Bad string")` (`Quake/pr_cmds.c`),
    /// used by `PF_cl_precache_sound`.
    pub fn PRBI_ClGlue_CheckEmptyString(s: *const c_char) -> c_int;

    /// The whole `PF_cl_makestatic` body (`Quake/pr_cmds.c`), kept in C
    /// (ADR-007): `entity_t` / `cl.static_entities` have no ADR-011 mirror in
    /// Phase 7. `ent` is an `edict_t *`.
    pub fn PRBI_ClGlue_MakeStatic(ent: *mut c_void) -> c_int;
}
