//! `Quake/cl_parse_glue.c` declarations (Rust migration Phase 7 M7, T7.3).
//!
//! ADR-011: engine C symbols are declared only in this crate. The glue file
//! owns the one C-visible object `Quake/cl_parse.c` defined --
//! `svc_strings[128]` (`cl_parse.c:30-88`) -- plus the twenty-one
//! `Host_Guard` trampolines this port needs (ADR-009).
//!
//! `cl`/`cls` are ADR-007 dual-view rows that stay C-owned for all of T7.3
//! (that row closes in T7.4 with `cl_main.c`). Both are mirror-typed, so they
//! are declared in `quake-capi/src/cl_parse.rs`, which can name
//! `quake_types`; this crate has no `[dependencies]`. Everything below is
//! either a primitive-signature function or a flat POD that names no
//! `quake-types` item.
//!
//! `Host_Reraise` is deliberately absent: only `cl_parse_glue.c` calls it
//! (ADR-009 rule 3). A `ClParse_Glue_*` returning non-zero is propagated
//! upward as a status and re-issued from that pure-C frame.

use core::ffi::{c_char, c_float, c_int, c_uint, c_void};

/// `Quake/client.h:30-36` -- `lightstyle_t`. A flat POD; `MAX_STYLESTRING` is
/// 64 (`client.h:28`). `quake-ctest/tests/cl_parse_differential.rs` pins the
/// layout against C `sizeof`/`offsetof`.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(non_camel_case_types)]
pub struct lightstyle_t {
    pub length: c_int,
    pub map: [c_char; 64],
    pub average: c_char,
    pub peak: c_char,
}

/// `Quake/glquake.h:600-611` -- `devstats_t`. `CL_ParseServerInfo` only ever
/// `memset`s it, but the whole object must be zeroed, so the real extent
/// matters.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(non_camel_case_types)]
pub struct devstats_t {
    pub packetsize: c_int,
    pub edicts: c_int,
    pub visedicts: c_int,
    pub efrags: c_int,
    pub tempents: c_int,
    pub beams: c_int,
    pub dlights: c_int,
}

/// `Quake/vid.h:53-69` -- `viddef_t`. `cl_parse.c` only writes
/// `vid.recalc_refdef`, but the mirror has to be complete for that field's
/// offset to be right.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(non_camel_case_types)]
pub struct viddef_t {
    pub buffer: *mut u8,
    pub colormap: *mut u8,
    pub colormap16: *mut u16,
    pub fullbright: c_int,
    pub rowbytes: c_int,
    pub width: c_int,
    pub height: c_int,
    pub aspect: c_float,
    pub recalc_refdef: c_int,
    pub conbuffer: *mut u8,
    pub conrowbytes: c_int,
    pub conwidth: c_int,
    pub conheight: c_int,
    pub restart_next_frame: bool,
}

extern "C" {
    /* ---- cl_parse_glue.c data (cl_parse.c:30-88) ---- */

    /// `cl_parse.c:30` -- `const char *svc_strings[128];`. External linkage in
    /// the original; the storage stays in C because the Illegible-server-
    /// message raise formats `svc_strings[lastcmd]` from the glue frame after
    /// the Rust core has already returned.
    pub static svc_strings: [*const c_char; 128];

    /* ---- ADR-009 Host_Guard trampolines (cl_parse_glue.c) ----
     *
     * Each returns the Host_Guard status: 0 for a clean return, non-zero for
     * a caught jump. A non-zero result must be propagated to the glue as
     * CLPARSE_RAISE_GUARD with the status in `a`; it must never be re-raised
     * from a Rust frame.
     */

    /// `cl_parse.c:757`, `:1159`, `:2026` -- `CL_SignonReply`.
    pub fn ClParse_Glue_SignonReply() -> c_int;
    /// `cl_parse.c:945` -- `CL_ClearState`.
    pub fn ClParse_Glue_ClearState() -> c_int;
    /// `cl_parse.c:951` -- `Key_ClearStates`.
    pub fn ClParse_Glue_KeyClearStates() -> c_int;
    /// `cl_parse.c:941` -- `SCR_BeginLoadingPlaque`.
    pub fn ClParse_Glue_BeginLoadingPlaque() -> c_int;
    /// `cl_parse.c:1108` -- `R_NewMap`.
    pub fn ClParse_Glue_NewMap() -> c_int;
    /// `cl_parse.c:2024` -- `R_CheckEfrags`.
    pub fn ClParse_Glue_CheckEfrags() -> c_int;
    /// `cl_parse.c:1993` -- `CL_ParseTEnt`, itself a re-raising wrapper under
    /// `-Duse_rust_host`, so a Rust frame must never call it directly.
    pub fn ClParse_Glue_ParseTEnt() -> c_int;
    /// `cl_parse.c:1662-1663` -- the implicit `effectinfo.` load plus
    /// `COM_Effectinfo_Enumerate (CL_GenerateRandomParticlePrecache)`.
    pub fn ClParse_Glue_EffectinfoEnumerate() -> c_int;
    /// `cl_parse.c:2201-2203` -- the CSQC entry, qcvm switch included
    /// (ADR-008).
    pub fn ClParse_Glue_CsqcParseEvent() -> c_int;
    /// `cl_parse.c:163-171` -- the `sv.active` debug print for an entity that
    /// arrived without a reset; runs against the server vm (ADR-008).
    pub fn ClParse_Glue_DebugNewEntity(entnum: c_uint) -> c_int;
    /// `cl_parse.c:562`, `:601`, `:1227`, `:1338` --
    /// `R_TranslateNewPlayerSkin`.
    pub fn ClParse_Glue_TranslateNewPlayerSkin(playernum: c_int) -> c_int;
    /// `cl_parse.c:1531` -- `R_TranslatePlayerSkin`.
    pub fn ClParse_Glue_TranslatePlayerSkin(playernum: c_int) -> c_int;
    /// `cl_parse.c:1578` -- `R_AddEfrags`.
    pub fn ClParse_Glue_AddEfrags(ent: *mut c_void) -> c_int;
    /// `cl_parse.c:1090`, `:1623` -- `Mod_ForName (name, false)`. `*out` is
    /// set to NULL before the guard, so a caught jump leaves it NULL.
    pub fn ClParse_Glue_ModForName(name: *const c_char, out: *mut *mut c_void) -> c_int;
    /// `cl_parse.c:1968` -- `Cbuf_AddText`.
    pub fn ClParse_Glue_CbufAddText(text: *const c_char) -> c_int;
    /// `cl_parse.c:1964`, `:2087`, `:2095` -- `Cmd_ExecuteString`. `*out` is
    /// set to 0 before the guard.
    pub fn ClParse_Glue_CmdExecuteString(text: *const c_char, src: c_int, out: *mut c_int)
        -> c_int;
    /// `cl_parse.c:1636`, `:1662`, `:1682` -- `PScript_FindParticleType`.
    /// `*out` is set to 0 before the guard.
    pub fn ClParse_Glue_FindParticleType(name: *const c_char, out: *mut c_int) -> c_int;
    /// `cl_parse.c:1693` -- `PScript_UpdateModelEffects`.
    pub fn ClParse_Glue_UpdateModelEffects(model: *mut c_void) -> c_int;
    /// `cl_parse.c:1713` -- `PScript_ParticleTrail`; the glue supplies the
    /// NULL colour argument.
    pub fn ClParse_Glue_ParticleTrail(
        start: *const c_float,
        end: *const c_float,
        type_: c_int,
        timeinterval: c_float,
        dlkey: c_int,
        tsk: *mut *mut c_void,
    ) -> c_int;
    /// `cl_parse.c:1736` -- `PScript_RunParticleEffectState`.
    pub fn ClParse_Glue_RunParticleEffectState(
        org: *const c_float,
        dir: *const c_float,
        count: c_float,
        typenum: c_int,
        tsk: *mut *mut c_void,
    ) -> c_int;
    /// `cl_parse.c:2091` -- `Sky_LoadSkyBox`.
    pub fn ClParse_Glue_LoadSkyBox(name: *const c_char) -> c_int;

    /* ---- Engine C symbols cl_parse.c calls directly ----
     *
     * None of these can Host_Error/Host_EndGame on any reachable path, so no
     * guard is needed: each is a leaf, a console/logging sink, or fails only
     * by Sys_Error, which aborts rather than jumping (the world.c /
     * sv_phys.c precedent).
     */

    /// `Quake/console.h` -- `const char *Con_Quakebar (int len);` returns a
    /// pointer into a console-owned static buffer.
    pub fn Con_Quakebar(len: c_int) -> *const c_char;
    /// `Quake/console.h` -- `void Con_LogCenterPrint (const char *str);`
    pub fn Con_LogCenterPrint(str: *const c_char);
    /// `Quake/screen.h` -- `void SCR_CenterPrint (const char *str);`
    pub fn SCR_CenterPrint(str: *const c_char);
    /// `Quake/console.h` -- `char con_lastcenterstring[1024];`. Only
    /// `con_lastcenterstring[0] = 0` is written (`cl_parse.c:1112`).
    pub static mut con_lastcenterstring: [c_char; 1024];

    /// `Quake/q_stdinc.h` -- `size_t q_strlcpy (char *dst, const char *src,
    /// size_t size);`
    pub fn q_strlcpy(dst: *mut c_char, src: *const c_char, size: usize) -> usize;
    /// `Quake/q_stdinc.h` -- `int q_snprintf (char *str, size_t size, const
    /// char *format, ...);`
    pub fn q_snprintf(str: *mut c_char, size: usize, format: *const c_char, ...) -> c_int;
    /// `Quake/common.h` -- `char *va (const char *format, ...);` returns a
    /// pointer into a rotating static buffer.
    pub fn va(format: *const c_char, ...) -> *mut c_char;

    /// `Quake/sound.h` -- the precache/touch/stop entry points; all bounded
    /// table operations over the sfx cache.
    pub fn S_LocalSound(name: *const c_char);
    pub fn S_TouchSound(name: *const c_char);
    pub fn S_StopSound(entnum: c_int, entchannel: c_int);
    pub fn S_BeginPrecaching();
    pub fn S_EndPrecaching();

    /// `Quake/render.h` -- `void Mod_TouchModel (const char *name);`
    pub fn Mod_TouchModel(name: *const c_char);

    /// `Quake/bgmusic.h` -- the music transport.
    pub fn BGM_Pause();
    pub fn BGM_Resume();
    pub fn BGM_PlayCDtrack(track: u8, looping: bool);
    /// `Quake/cdaudio.h`
    pub fn CDAudio_Pause();
    pub fn CDAudio_Resume();

    /// `Quake/gl_fog.c` -- `void Fog_ParseServerMessage (void);` reads the
    /// message itself.
    pub fn Fog_ParseServerMessage();
    /// `Quake/r_part.c` -- `void R_ParseParticleEffect (void);`
    pub fn R_ParseParticleEffect();
    /// `Quake/gl_rmisc.c` -- `void R_FreeEntityBLAS (entity_t *ent);` drops a
    /// raytracing acceleration structure; `entity_t` is opaque here.
    pub fn R_FreeEntityBLAS(ent: *mut c_void);

    /// `Quake/view.h` -- `void V_ParseDamage (void);` and
    /// `void V_RestoreAngles (void);`. Both are T7.1 Rust exports in the
    /// shipping build and plain C in the ctest link; the extern resolves to
    /// whichever definition the link provides.
    pub fn V_ParseDamage();
    pub fn V_RestoreAngles();

    /// `Quake/input.h` -- `void IN_ClearStates (void);`
    pub fn IN_ClearStates();

    /// `Quake/steam.h` -- `qboolean Steam_SetAchievement (const char *name);`
    pub fn Steam_SetAchievement(name: *const c_char) -> bool;

    /// `Quake/common.h` -- `qboolean COM_GameDirMatches (const char *tdirs);`
    pub fn COM_GameDirMatches(tdirs: *const c_char) -> bool;

    /// `Quake/common.h` -- `qboolean standard_quake;`. Rust-owned under
    /// `-Duse_rust_fs` (`quake-capi/src/fs.rs:45`), C-owned otherwise; the
    /// extern resolves to whichever definition the link provides.
    pub static mut standard_quake: bool;

    /// `Quake/net.h` -- `int NET_QSocketGetSequenceIn (const struct qsocket_s
    /// *s);`. `cls.netcon` is opaque here.
    pub fn NET_QSocketGetSequenceIn(s: *const c_void) -> c_int;

    /// `Quake/client.h` -- `extern lightstyle_t cl_lightstyle[MAX_LIGHTSTYLES];`
    /// (`MAX_LIGHTSTYLES` is 64). `cl_main.c` owns the storage until T7.4.
    pub static mut cl_lightstyle: [lightstyle_t; 64];

    /// `Quake/glquake.h:613` -- `extern devstats_t dev_stats, dev_peakstats;`
    pub static mut dev_stats: devstats_t;
    pub static mut dev_peakstats: devstats_t;

    /// `Quake/vid.h` -- `extern viddef_t vid;`
    pub static mut vid: viddef_t;

    /// `Quake/gl_model.c` -- `qmodel_t mod_known[MAX_MOD_KNOWN]; int
    /// mod_numknown;`. `qmodel_t` is `quake_types::model_mem::QModel`, which
    /// this crate cannot name, so the array uses the zero-length-blob idiom
    /// `com_basedir` uses and the stride math is done in `quake-capi` against
    /// `size_of::<QModel>()` (ABI-pinned by `quake-ctest/tests/bsp_abi.rs`).
    pub static mut mod_known: [u8; 0];
    pub static mut mod_numknown: c_int;

    /// `Quake/client.h` -- `extern cvar_t cl_shownet;`. Owned by `cl_main.c`
    /// until T7.4.
    pub static mut cl_shownet: crate::cvar_t;
}
