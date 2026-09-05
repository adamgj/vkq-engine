//! `Quake/cl_tent_glue.c` declarations (Rust migration Phase 7 M7, T7.2).
//!
//! ADR-011: engine C symbols are declared only in this crate. The glue file
//! owns the three C-visible objects `Quake/cl_tent.c` used to define --
//! `num_temp_entities`, `cl_temp_entities[]` and `cl_beams[]`
//! (`cl_tent.c:26-28`), all three still read by `cl_main.c`, `cl_demo.c` and
//! `host_cmd.c` -- plus the one `Host_Guard` trampoline this port needs
//! (ADR-009) and the `entity_t` accessors Rust cannot express against an
//! opaque blob.
//!
//! The seven `cl_sfx_*` handles were file-static in `cl_tent.c` and move into
//! Rust; nothing outside the file ever named them.
//!
//! `cl` is an ADR-007 dual-view row that stays C-owned for all of T7.2. It is
//! mirror-typed, so it is declared in `quake-capi/src/cl_tent.rs`, which can
//! name `quake_types`; this crate has no `[dependencies]`. `cl_temp_entities`
//! is declared there too, because `entity_t` is opaque and only
//! `quake-types` names the blob. `beam_t` holds no engine struct by value, so
//! it is mirrored here, next to the `cl_beams` extern.

use core::ffi::{c_char, c_float, c_int, c_uint, c_void};

/// `Quake/client.h:71-83` -- `dlight_t`. A flat POD, so it is mirrored rather
/// than reached through accessors; `quake-ctest/tests/cl_tent_differential.rs`
/// pins the layout against C `offsetof`.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(non_camel_case_types)]
pub struct dlight_t {
    pub origin: [c_float; 3],
    pub radius: c_float,
    pub die: c_float,
    pub decay: c_float,
    pub minlight: c_float,
    pub key: c_int,
    pub color: [c_float; 3],
    pub cone_dir: [c_float; 3],
    pub cone_cos: c_float,
    pub kex_intensity: c_float,
}

/// `Quake/client.h:85-96` -- `beam_t`, in its `PSET_SCRIPT` shape
/// (`quakedef.h:38` defines `PSET_SCRIPT` unconditionally, so the two trail
/// fields are always present). `qmodel_t`/`trailstate_s` are opaque, so both
/// are `*mut c_void`.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(non_camel_case_types)]
pub struct beam_t {
    pub entity: c_int,
    pub model: *mut c_void,
    pub endtime: c_float,
    pub start: [c_float; 3],
    pub end: [c_float; 3],
    pub trailname: *const c_char,
    pub trailstate: *mut c_void,
}

extern "C" {
    /* Quake/cl_tent_glue.c data (cl_tent.c:26-28) */

    pub static mut num_temp_entities: c_int;
    pub static mut cl_beams: [beam_t; 32];

    /* Quake/cl_tent_glue.c guard -- returns a Host_Guard status */

    /// `Mod_ForName (name, true)` (`cl_tent.c:236`, `:240`, `:244`, `:249`).
    /// The `crash=true` path `Host_Error`s from `gl_model.c:531` when the
    /// model is missing, so this is the port's only ADR-009 raise site.
    /// `*out` is cleared before the guarded call.
    pub fn ClTent_Glue_ModForName(name: *const c_char, out: *mut *mut c_void) -> c_int;

    /* Quake/cl_tent_glue.c plain shims. None of these can raise. */

    /// `cl_tent.c:314` -- `Sys_Error ("CL_ParseTEnt: bad type")`. `Sys_Error`
    /// terminates rather than longjmping, so this is a noreturn shim and NOT
    /// a `Host_Guard` site -- the same reasoning `sv_send_glue.c` records for
    /// `SvSend_Glue_FatPvsAllocFailed`.
    pub fn ClTent_Glue_BadTEntType() -> !;

    /// `cl_tent.c:275` -- keeps `va ("TE_EXPLOSION2_%i_%i", ...)` on the C
    /// side so the ADR-005 formatter is not re-entered from here.
    pub fn ClTent_Glue_Explosion2Name(color_start: c_int, color_length: c_int) -> *const c_char;

    /// `cl_tent.c:332` -- `memset (ent, 0, sizeof (*ent))`. `entity_t` is
    /// opaque to Rust (ADR-011), so the write stays in C; same shape as
    /// `world_glue.c`'s `World_Glue_EntClipInfo`.
    pub fn ClTent_Glue_ClearTempEntity(ent: *mut c_void);

    /// `cl_tent.c:337` -- `ent->netstate = nullentitystate`. Separate from the
    /// memset because C does it after the three counter updates.
    pub fn ClTent_Glue_SetTempEntityNetstate(ent: *mut c_void);

    /// `cl_tent.c:404-408` -- the five `entity_t` writes `CL_UpdateTEnts`
    /// makes per lightning segment, in source order.
    pub fn ClTent_Glue_SetBeamEntity(
        ent: *mut c_void,
        org: *const c_float,
        model: *mut c_void,
        pitch: c_float,
        yaw: c_float,
        roll: c_float,
    );

    /// `cl_tent.c:370` -- `cl.entities[cl.viewentity].origin`.
    pub fn ClTent_Glue_GetEntityOrigin(ent: *const c_void, out: *mut c_float);

    /* Engine C symbols cl_tent.c calls directly; none of these can raise. */

    /// `cl_main.c:361`. A bounded scan over `cl_dlights[MAX_DLIGHTS]`; no VM,
    /// network or filesystem access on any path.
    pub fn CL_AllocDlight(key: c_int) -> *mut dlight_t;

    /// `client.h:339`, `:341`, `:344` -- owned by `cl_main.c`, still C in
    /// T7.2. `cl_visedicts` is a pointer to a heap array, not an array.
    pub static mut cl_visedicts: *mut *mut c_void;
    pub static mut cl_numvisedicts: c_int;
    pub static mut cl_maxvisedicts: c_int;

    /// `r_part_fte.c:625` -- `float CL_TraceLine (vec3_t start, vec3_t end,
    /// vec3_t impact, vec3_t normal, struct trace_s *trace);`. `r_part_fte.c`
    /// contains no `Host_Error`/`Host_EndGame` on any path.
    pub fn CL_TraceLine(
        start: *mut c_float,
        end: *mut c_float,
        impact: *mut c_float,
        normal: *mut c_float,
        trace: *mut c_void,
    ) -> c_float;

    /// `r_part_fte.c` -- `void PScript_RunParticleWeather (vec3_t minb,
    /// vec3_t maxb, vec3_t dir, float count, int colour, const char *efname);`
    pub fn PScript_RunParticleWeather(
        minb: *mut c_float,
        maxb: *mut c_float,
        dir: *mut c_float,
        count: c_float,
        colour: c_int,
        efname: *const c_char,
    );

    /// `r_part.c` particle entry points, reached from the `TE_*` cases when
    /// the script engine declines the effect.
    pub fn R_ParticleExplosion(org: *mut c_float);
    pub fn R_ParticleExplosion2(org: *mut c_float, color_start: c_int, color_length: c_int);
    pub fn R_BlobExplosion(org: *mut c_float);
    pub fn R_LavaSplash(org: *mut c_float);
    pub fn R_TeleportSplash(org: *mut c_float);

    /// `Quake/common.h:191` -- `unsigned int MSG_ReadEntity (unsigned int
    /// pext2);`. Sets `msg_badread` on underflow; never longjmps.
    pub fn MSG_ReadEntity(pext2: c_uint) -> c_uint;
    /// `Quake/common.h` -- `float MSG_ReadCoord (unsigned int flags);`
    pub fn MSG_ReadCoord(flags: c_uint) -> c_float;
}
