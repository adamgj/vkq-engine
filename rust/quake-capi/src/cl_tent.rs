//! `Quake/cl_tent.c` -- client-side temporary entities (Rust migration Phase
//! 7 M7, T7.2, Pattern A whole-file swap).
//!
//! ## ADR-009 raise-topology audit
//!
//! `cl_tent.c` has exactly two raise-shaped sites, and only one of them is a
//! guard site:
//!
//! - `Mod_ForName (name, true)` (`cl_tent.c:236`, `:240`, `:244`, `:249`).
//!   The `crash` arm `Host_Error`s from `gl_model.c:531`, so all four calls
//!   go through [`g::ClTent_Glue_ModForName`].
//! - `Sys_Error ("CL_ParseTEnt: bad type")` (`cl_tent.c:314`). `Sys_Error`
//!   terminates rather than longjmping, so it is a plain noreturn shim
//!   ([`g::ClTent_Glue_BadTEntType`]), not a `Host_Guard` frame -- the same
//!   reasoning `sv_send_glue.c` records for `SvSend_Glue_FatPvsAllocFailed`.
//!
//! Everything else this file calls was checked transitively and cannot
//! longjmp: `MSG_ReadByte`/`MSG_ReadShort`/`MSG_ReadCoord`/`MSG_ReadEntity`
//! only set `msg_badread`; `snd_dma.c`/`snd_mem.c` (`S_PrecacheSound`,
//! `S_StartSound`), `r_part.c`/`r_part_fte.c` (`R_*`, `PScript_*`,
//! `CL_TraceLine`) and `gl_rlight.c` (`CL_AllocDlight`) contain no
//! `Host_Error`/`Host_EndGame` on any path (verified by grep).
//!
//! So [`quake_rs_cl_parse_tent`] is the file's only status core, and
//! `Quake/cl_tent_glue.c`'s `CL_ParseTEnt` is the only `Host_Reraise` frame.
//! `CL_InitTEnts`, `CL_NewTempEntity` and `CL_UpdateTEnts` are plain, and
//! `CL_UpdateBeam` is plain too -- its `Mod_ForName` argument is evaluated by
//! the caller.
//!
//! ## Ownership
//!
//! ADR-007: `cl` stays C-owned for all of T7.2; the row closes at T7.4.
//! `num_temp_entities`, `cl_temp_entities[]` and `cl_beams[]` keep C-visible
//! storage in `Quake/cl_tent_glue.c`, because `cl_main.c`, `cl_demo.c` and
//! `host_cmd.c` still read them by name. The seven `cl_sfx_*` handles were
//! file-static and move into Rust; nothing outside the file names them.
//!
//! `entity_t` is opaque to Rust (ADR-011), so every write into one goes
//! through a `ClTent_Glue_*` accessor, the shape `world_glue.c` established.

use core::ffi::{c_char, c_float, c_int, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::cl_tent as g;
use quake_c_sys::libm;
use quake_c_sys::progs_builtins_cl::{
    PScript_RunParticleEffectTypeString, R_RunParticleEffect, S_PrecacheSound, S_StartSound,
};
use quake_c_sys::sv_main::realtime;
use quake_c_sys::sv_user::{MSG_ReadByte, MSG_ReadShort};
use quake_math::mathlib as m;
use quake_types::host::{ClientState, EntityOpaque};

use crate::mathlib::vec3_origin;

/// A `Host_Guard` status: 0, or the code the guarded frame caught. Non-zero
/// must reach `Quake/cl_tent_glue.c` untouched.
type Raise = c_int;

/// Propagate a non-zero `Host_Guard` status, abandoning the rest of the body
/// exactly where C's `longjmp` would have left it.
macro_rules! raise {
    ($e:expr) => {{
        let r: Raise = $e;
        if r != 0 {
            return r;
        }
    }};
}

extern "C" {
    /// `Quake/cl_main.c` -- ADR-007 dual-view row, C-owned until T7.4.
    static mut cl: ClientState;
    /// `Quake/cl_tent_glue.c` -- `entity_t` is opaque, so the array is
    /// declared here rather than in `quake-c-sys` (which cannot name
    /// `quake-types`).
    static mut cl_temp_entities: [EntityOpaque; MAX_TEMP_ENTITIES];
}

/// `glquake.h:616-621` -- `overflowtimes_t`. `quake-capi/src/sv_send.rs`
/// mirrors the same C object for its own overflow warning; the two mirrors are
/// structurally identical, which is what `clashing_extern_declarations`
/// compares.
#[repr(C)]
struct OverflowTimes {
    packetsize: f64,
    efrags: f64,
    beams: f64,
    varstring: f64,
}

extern "C" {
    /// `Quake/gl_rmisc.c` -- last time each overflow warning was printed.
    static mut dev_overflows: OverflowTimes;
}

/// `glquake.h:623` -- seconds between repeated overflow warnings.
const CONSOLE_RESPAM_TIME: f64 = 3.0;

/// `client.h:86`.
const MAX_BEAMS: usize = 32;
/// `client.h:330`.
const MAX_TEMP_ENTITIES: usize = 256;

/* protocol.h:355-374 */
const TE_SPIKE: c_int = 0;
const TE_SUPERSPIKE: c_int = 1;
const TE_GUNSHOT: c_int = 2;
const TE_EXPLOSION: c_int = 3;
const TE_TAREXPLOSION: c_int = 4;
const TE_LIGHTNING1: c_int = 5;
const TE_LIGHTNING2: c_int = 6;
const TE_WIZSPIKE: c_int = 7;
const TE_KNIGHTSPIKE: c_int = 8;
const TE_LIGHTNING3: c_int = 9;
const TE_LAVASPLASH: c_int = 10;
const TE_TELEPORT: c_int = 11;
const TE_EXPLOSION2: c_int = 12;
const TE_BEAM: c_int = 13;
const TEDP_PARTICLERAIN: c_int = 55;
const TEDP_PARTICLESNOW: c_int = 56;

/// `cl_tent.c:30-36`. File-static in C; nothing outside the file names them.
static mut CL_SFX: [*mut c_void; 7] = [ptr::null_mut(); 7];
const SFX_WIZHIT: usize = 0;
const SFX_KNIGHTHIT: usize = 1;
const SFX_TINK1: usize = 2;
const SFX_RIC1: usize = 3;
const SFX_RIC2: usize = 4;
const SFX_RIC3: usize = 5;
const SFX_R_EXP3: usize = 6;

/// `cl_tent.c:43`.
#[no_mangle]
pub extern "C" fn CL_InitTEnts() {
    // SAFETY: `S_PrecacheSound` takes a NUL-terminated name and returns an
    // engine-owned handle; `CL_SFX` is process-lifetime storage.
    unsafe {
        let sfx = ptr::addr_of_mut!(CL_SFX) as *mut *mut c_void;
        *sfx.add(SFX_WIZHIT) = S_PrecacheSound(c"wizard/hit.wav".as_ptr());
        *sfx.add(SFX_KNIGHTHIT) = S_PrecacheSound(c"hknight/hit.wav".as_ptr());
        *sfx.add(SFX_TINK1) = S_PrecacheSound(c"weapons/tink1.wav".as_ptr());
        *sfx.add(SFX_RIC1) = S_PrecacheSound(c"weapons/ric1.wav".as_ptr());
        *sfx.add(SFX_RIC2) = S_PrecacheSound(c"weapons/ric2.wav".as_ptr());
        *sfx.add(SFX_RIC3) = S_PrecacheSound(c"weapons/ric3.wav".as_ptr());
        *sfx.add(SFX_R_EXP3) = S_PrecacheSound(c"weapons/r_exp3.wav".as_ptr());
    }
}

#[inline]
unsafe fn sfx(i: usize) -> *mut c_void {
    // SAFETY: `i` is one of the seven SFX_* constants.
    unsafe { (ptr::addr_of!(CL_SFX) as *const *mut c_void).add(i).read() }
}

// ---------------------------------------------------------------------------
// Beams.

/// `cl_tent.c:59`. Called from `cl_parse.c` as well as from this file.
///
/// # Safety
/// `start` and `end` must each point at three readable floats; `trailname`
/// and `impactname` must be NUL-terminated or null.
#[no_mangle]
pub unsafe extern "C" fn CL_UpdateBeam(
    model: *mut c_void,
    trailname: *const c_char,
    impactname: *const c_char,
    ent: c_int,
    start: *mut c_float,
    end: *mut c_float,
) {
    // `trailname` is unused in C too; the parameter is kept so the two
    // signatures stay identical for `cl_parse.c`'s caller.
    let _ = trailname;

    // SAFETY: `start`/`end` are caller-owned vec3_t; `cl_beams` is glue-owned
    // with process lifetime; neither `CL_TraceLine` nor
    // `PScript_RunParticleEffectTypeString` can raise.
    unsafe {
        {
            let mut normal = [0.0f32; 3];
            let mut extra = [0.0f32; 3];
            let mut impact = [0.0f32; 3];
            let s = *(start as *const [c_float; 3]);
            let e = *(end as *const [c_float; 3]);
            m::vector_subtract(&e, &s, &mut normal);
            m::vector_normalize(&mut normal);
            m::vector_ma(&e, 4.0, &normal, &mut extra); // extend the end-point by four
            if g::CL_TraceLine(
                start,
                extra.as_mut_ptr(),
                impact.as_mut_ptr(),
                normal.as_mut_ptr(),
                ptr::null_mut(),
            ) < 1.0
            {
                PScript_RunParticleEffectTypeString(
                    impact.as_mut_ptr(),
                    normal.as_mut_ptr(),
                    1.0,
                    impactname,
                );
            }
        }

        let beams = ptr::addr_of_mut!(g::cl_beams) as *mut g::beam_t;

        // override any beam with the same entity
        for i in 0..MAX_BEAMS {
            let b = beams.add(i);
            if (*b).entity == ent {
                (*b).entity = ent;
                (*b).model = model;
                (*b).endtime = (cl.time + 0.2) as c_float;
                (*b).start = *(start as *const [c_float; 3]);
                (*b).end = *(end as *const [c_float; 3]);
                return;
            }
        }

        // find a free beam
        for i in 0..MAX_BEAMS {
            let b = beams.add(i);
            if (*b).model.is_null() || ((*b).endtime as f64) < cl.time {
                (*b).entity = ent;
                (*b).model = model;
                (*b).endtime = (cl.time + 0.2) as c_float;
                (*b).start = *(start as *const [c_float; 3]);
                (*b).end = *(end as *const [c_float; 3]);
                return;
            }
        }

        // johnfitz -- less spammy overflow message
        if dev_overflows.beams == 0.0 || dev_overflows.beams + CONSOLE_RESPAM_TIME < realtime {
            c::Con_Printf(c"Beam list overflow!\n".as_ptr());
            dev_overflows.beams = realtime;
        }
        // johnfitz
    }
}

/// `cl_tent.c:106` (file-static in C).
///
/// # Safety
/// Reads `net_message` through the plain `MSG_Read*` shims.
unsafe fn cl_parse_beam(model: *mut c_void, trailname: *const c_char, impactname: *const c_char) {
    // SAFETY: `cl` is C-owned with process lifetime; the two vectors are
    // locals whose addresses do not escape `CL_UpdateBeam`.
    unsafe {
        let ent = g::MSG_ReadEntity(cl.protocol_pext2) as c_int;

        let mut start = [0.0f32; 3];
        let mut end = [0.0f32; 3];

        start[0] = g::MSG_ReadCoord(cl.protocolflags);
        start[1] = g::MSG_ReadCoord(cl.protocolflags);
        start[2] = g::MSG_ReadCoord(cl.protocolflags);

        end[0] = g::MSG_ReadCoord(cl.protocolflags);
        end[1] = g::MSG_ReadCoord(cl.protocolflags);
        end[2] = g::MSG_ReadCoord(cl.protocolflags);

        CL_UpdateBeam(
            model,
            trailname,
            impactname,
            ent,
            start.as_mut_ptr(),
            end.as_mut_ptr(),
        );
    }
}

// ---------------------------------------------------------------------------
// CL_ParseTEnt (cl_tent.c:133) -- the file's only status core.

/// The `TE_SPIKE`/`TE_SUPERSPIKE` ricochet pick (`cl_tent.c:167-178`),
/// verbatim: one `COM_Rand ()` for the 4-in-5 tink, a second for the
/// three-way ricochet.
///
/// # Safety
/// `pos` must point at three readable floats.
unsafe fn ricochet(pos: *mut c_float) {
    // SAFETY: `pos` is a caller-owned vec3_t; `S_StartSound` on an
    // uninitialised sound engine is a no-op and cannot raise.
    unsafe {
        if c::COM_Rand() % 5 != 0 {
            S_StartSound(-1, 0, sfx(SFX_TINK1), pos, 1.0, 1.0);
        } else {
            let rnd = c::COM_Rand() & 3;
            if rnd == 1 {
                S_StartSound(-1, 0, sfx(SFX_RIC1), pos, 1.0, 1.0);
            } else if rnd == 2 {
                S_StartSound(-1, 0, sfx(SFX_RIC2), pos, 1.0, 1.0);
            } else {
                S_StartSound(-1, 0, sfx(SFX_RIC3), pos, 1.0, 1.0);
            }
        }
    }
}

/// `cl_tent.c:225-249` -- the four `Mod_ForName (name, true)` calls, the
/// port's only ADR-009 raise site.
unsafe fn parse_beam_model(
    name: *const c_char,
    trailname: *const c_char,
    impactname: *const c_char,
) -> Raise {
    // SAFETY: `name` is a static literal; `model` is written before use.
    unsafe {
        let mut model: *mut c_void = ptr::null_mut();
        raise!(g::ClTent_Glue_ModForName(name, &mut model));
        cl_parse_beam(model, trailname, impactname);
        0
    }
}

/// `cl_tent.c:133`. The non-reraising core behind
/// `Quake/cl_tent_glue.c`'s `CL_ParseTEnt`; Rust never calls the plain name.
///
/// # Safety
/// Reads `net_message` through the plain `MSG_Read*` shims.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cl_parse_tent() -> Raise {
    // SAFETY: `cl` is C-owned with process lifetime; `pos`/`pos2`/`dir` are
    // locals whose addresses never escape the recording callees.
    unsafe {
        let mut pos = [0.0f32; 3];

        let ty = MSG_ReadByte();
        match ty {
            TE_WIZSPIKE => {
                // spike hitting wall
                pos[0] = g::MSG_ReadCoord(cl.protocolflags);
                pos[1] = g::MSG_ReadCoord(cl.protocolflags);
                pos[2] = g::MSG_ReadCoord(cl.protocolflags);
                if PScript_RunParticleEffectTypeString(
                    pos.as_mut_ptr(),
                    ptr::null_mut(),
                    1.0,
                    c"TE_WIZSPIKE".as_ptr(),
                ) != 0
                {
                    R_RunParticleEffect(
                        pos.as_mut_ptr(),
                        ptr::addr_of_mut!(vec3_origin) as *mut c_float,
                        20,
                        30,
                    );
                }
                S_StartSound(-1, 0, sfx(SFX_WIZHIT), pos.as_mut_ptr(), 1.0, 1.0);
            }

            TE_KNIGHTSPIKE => {
                // spike hitting wall
                pos[0] = g::MSG_ReadCoord(cl.protocolflags);
                pos[1] = g::MSG_ReadCoord(cl.protocolflags);
                pos[2] = g::MSG_ReadCoord(cl.protocolflags);
                if PScript_RunParticleEffectTypeString(
                    pos.as_mut_ptr(),
                    ptr::null_mut(),
                    1.0,
                    c"TE_KNIGHTSPIKE".as_ptr(),
                ) != 0
                {
                    R_RunParticleEffect(
                        pos.as_mut_ptr(),
                        ptr::addr_of_mut!(vec3_origin) as *mut c_float,
                        226,
                        20,
                    );
                }
                S_StartSound(-1, 0, sfx(SFX_KNIGHTHIT), pos.as_mut_ptr(), 1.0, 1.0);
            }

            TE_SPIKE => {
                // spike hitting wall
                pos[0] = g::MSG_ReadCoord(cl.protocolflags);
                pos[1] = g::MSG_ReadCoord(cl.protocolflags);
                pos[2] = g::MSG_ReadCoord(cl.protocolflags);
                if PScript_RunParticleEffectTypeString(
                    pos.as_mut_ptr(),
                    ptr::null_mut(),
                    1.0,
                    c"TE_SPIKE".as_ptr(),
                ) != 0
                {
                    R_RunParticleEffect(
                        pos.as_mut_ptr(),
                        ptr::addr_of_mut!(vec3_origin) as *mut c_float,
                        0,
                        10,
                    );
                }
                ricochet(pos.as_mut_ptr());
            }
            TE_SUPERSPIKE => {
                // super spike hitting wall
                pos[0] = g::MSG_ReadCoord(cl.protocolflags);
                pos[1] = g::MSG_ReadCoord(cl.protocolflags);
                pos[2] = g::MSG_ReadCoord(cl.protocolflags);
                if PScript_RunParticleEffectTypeString(
                    pos.as_mut_ptr(),
                    ptr::null_mut(),
                    1.0,
                    c"TE_SUPERSPIKE".as_ptr(),
                ) != 0
                {
                    R_RunParticleEffect(
                        pos.as_mut_ptr(),
                        ptr::addr_of_mut!(vec3_origin) as *mut c_float,
                        0,
                        20,
                    );
                }

                ricochet(pos.as_mut_ptr());
            }

            TE_GUNSHOT => {
                // bullet hitting wall
                let rnd: c_int = 20;
                pos[0] = g::MSG_ReadCoord(cl.protocolflags);
                pos[1] = g::MSG_ReadCoord(cl.protocolflags);
                pos[2] = g::MSG_ReadCoord(cl.protocolflags);
                if PScript_RunParticleEffectTypeString(
                    pos.as_mut_ptr(),
                    ptr::null_mut(),
                    rnd as c_float,
                    c"TE_GUNSHOT".as_ptr(),
                ) != 0
                {
                    R_RunParticleEffect(
                        pos.as_mut_ptr(),
                        ptr::addr_of_mut!(vec3_origin) as *mut c_float,
                        0,
                        rnd,
                    );
                }
            }

            TE_EXPLOSION => {
                // rocket explosion
                pos[0] = g::MSG_ReadCoord(cl.protocolflags);
                pos[1] = g::MSG_ReadCoord(cl.protocolflags);
                pos[2] = g::MSG_ReadCoord(cl.protocolflags);
                if PScript_RunParticleEffectTypeString(
                    pos.as_mut_ptr(),
                    ptr::null_mut(),
                    1.0,
                    c"TE_EXPLOSION".as_ptr(),
                ) != 0
                {
                    g::R_ParticleExplosion(pos.as_mut_ptr());
                }
                let dl = g::CL_AllocDlight(0);
                (*dl).origin = pos;
                (*dl).radius = 350.0;
                (*dl).die = (cl.time + 0.5) as c_float;
                (*dl).decay = 300.0;
                S_StartSound(-1, 0, sfx(SFX_R_EXP3), pos.as_mut_ptr(), 1.0, 1.0);
            }

            TE_TAREXPLOSION => {
                // tarbaby explosion
                pos[0] = g::MSG_ReadCoord(cl.protocolflags);
                pos[1] = g::MSG_ReadCoord(cl.protocolflags);
                pos[2] = g::MSG_ReadCoord(cl.protocolflags);
                if PScript_RunParticleEffectTypeString(
                    pos.as_mut_ptr(),
                    ptr::null_mut(),
                    1.0,
                    c"TE_TAREXPLOSION".as_ptr(),
                ) != 0
                {
                    g::R_BlobExplosion(pos.as_mut_ptr());
                }

                S_StartSound(-1, 0, sfx(SFX_R_EXP3), pos.as_mut_ptr(), 1.0, 1.0);
            }

            TE_LIGHTNING1 => {
                // lightning bolts
                raise!(parse_beam_model(
                    c"progs/bolt.mdl".as_ptr(),
                    c"TE_LIGHTNING1".as_ptr(),
                    c"TE_LIGHTNING1_END".as_ptr()
                ));
            }

            TE_LIGHTNING2 => {
                // lightning bolts
                raise!(parse_beam_model(
                    c"progs/bolt2.mdl".as_ptr(),
                    c"TE_LIGHTNING2".as_ptr(),
                    c"TE_LIGHTNING2_END".as_ptr()
                ));
            }

            TE_LIGHTNING3 => {
                // lightning bolts
                raise!(parse_beam_model(
                    c"progs/bolt3.mdl".as_ptr(),
                    c"TE_LIGHTNING3".as_ptr(),
                    c"TE_LIGHTNING3_END".as_ptr()
                ));
            }

            // PGM 01/21/97
            TE_BEAM => {
                // grappling hook beam
                raise!(parse_beam_model(
                    c"progs/beam.mdl".as_ptr(),
                    c"TE_BEAM".as_ptr(),
                    c"TE_BEAM_END".as_ptr()
                ));
            }
            // PGM 01/21/97
            TE_LAVASPLASH => {
                pos[0] = g::MSG_ReadCoord(cl.protocolflags);
                pos[1] = g::MSG_ReadCoord(cl.protocolflags);
                pos[2] = g::MSG_ReadCoord(cl.protocolflags);
                if PScript_RunParticleEffectTypeString(
                    pos.as_mut_ptr(),
                    ptr::null_mut(),
                    1.0,
                    c"TE_LAVASPLASH".as_ptr(),
                ) != 0
                {
                    g::R_LavaSplash(pos.as_mut_ptr());
                }
            }

            TE_TELEPORT => {
                pos[0] = g::MSG_ReadCoord(cl.protocolflags);
                pos[1] = g::MSG_ReadCoord(cl.protocolflags);
                pos[2] = g::MSG_ReadCoord(cl.protocolflags);
                if PScript_RunParticleEffectTypeString(
                    pos.as_mut_ptr(),
                    ptr::null_mut(),
                    1.0,
                    c"TE_TELEPORT".as_ptr(),
                ) != 0
                {
                    g::R_TeleportSplash(pos.as_mut_ptr());
                }
            }

            TE_EXPLOSION2 => {
                // color mapped explosion
                pos[0] = g::MSG_ReadCoord(cl.protocolflags);
                pos[1] = g::MSG_ReadCoord(cl.protocolflags);
                pos[2] = g::MSG_ReadCoord(cl.protocolflags);
                let color_start = MSG_ReadByte();
                let color_length = MSG_ReadByte();
                if PScript_RunParticleEffectTypeString(
                    pos.as_mut_ptr(),
                    ptr::null_mut(),
                    1.0,
                    g::ClTent_Glue_Explosion2Name(color_start, color_length),
                ) != 0
                {
                    g::R_ParticleExplosion2(pos.as_mut_ptr(), color_start, color_length);
                }
                let dl = g::CL_AllocDlight(0);
                (*dl).origin = pos;
                (*dl).radius = 350.0;
                (*dl).die = (cl.time + 0.5) as c_float;
                (*dl).decay = 300.0;
                S_StartSound(-1, 0, sfx(SFX_R_EXP3), pos.as_mut_ptr(), 1.0, 1.0);
            }

            TEDP_PARTICLERAIN | TEDP_PARTICLESNOW => {
                let mut dir = [0.0f32; 3];
                let mut pos2 = [0.0f32; 3];

                // min
                pos[0] = g::MSG_ReadCoord(cl.protocolflags);
                pos[1] = g::MSG_ReadCoord(cl.protocolflags);
                pos[2] = g::MSG_ReadCoord(cl.protocolflags);

                // max
                pos2[0] = g::MSG_ReadCoord(cl.protocolflags);
                pos2[1] = g::MSG_ReadCoord(cl.protocolflags);
                pos2[2] = g::MSG_ReadCoord(cl.protocolflags);

                // dir
                dir[0] = g::MSG_ReadCoord(cl.protocolflags);
                dir[1] = g::MSG_ReadCoord(cl.protocolflags);
                dir[2] = g::MSG_ReadCoord(cl.protocolflags);

                let cnt = MSG_ReadShort() as u16 as c_int; // count
                let colour = MSG_ReadByte(); // colour

                g::PScript_RunParticleWeather(
                    pos.as_mut_ptr(),
                    pos2.as_mut_ptr(),
                    dir.as_mut_ptr(),
                    cnt as c_float,
                    colour,
                    if ty == TEDP_PARTICLESNOW {
                        c"snow".as_ptr()
                    } else {
                        c"rain".as_ptr()
                    },
                );
            }

            _ => g::ClTent_Glue_BadTEntType(),
        }

        0
    }
}

// ---------------------------------------------------------------------------
// Temp-entity list.

/// `cl_tent.c:319`.
#[no_mangle]
pub extern "C" fn CL_NewTempEntity() -> *mut EntityOpaque {
    // SAFETY: every object touched is glue-owned with process lifetime;
    // `cl_visedicts` is sized by `cl_maxvisedicts`, which bounds the write.
    unsafe {
        if g::cl_numvisedicts == g::cl_maxvisedicts {
            return ptr::null_mut();
        }
        if g::num_temp_entities == MAX_TEMP_ENTITIES as c_int {
            return ptr::null_mut();
        }
        let ent = (ptr::addr_of_mut!(cl_temp_entities) as *mut EntityOpaque)
            .add(g::num_temp_entities as usize);
        // `entity_t` is opaque here (ADR-011), so the memset and the netstate
        // store are C-side -- and stay separate, because C runs the three
        // counter updates between them.
        g::ClTent_Glue_ClearTempEntity(ent as *mut c_void);
        g::num_temp_entities += 1;
        g::cl_visedicts
            .add(g::cl_numvisedicts as usize)
            .write(ent as *mut c_void);
        g::cl_numvisedicts += 1;

        g::ClTent_Glue_SetTempEntityNetstate(ent as *mut c_void);
        ent
    }
}

/// `cl_tent.c:340`.
#[no_mangle]
pub extern "C" fn CL_UpdateTEnts() {
    // SAFETY: `cl` is C-owned and `cl_beams` glue-owned, both with process
    // lifetime; `cl.entities` is checked before it is indexed, exactly as C
    // does.
    unsafe {
        g::num_temp_entities = 0;

        if cl.paused {
            c::COM_SeedRand((cl.time * 1000.0) as u64); // johnfitz -- freeze beams when paused
        }

        let beams = ptr::addr_of_mut!(g::cl_beams) as *mut g::beam_t;

        // update lightning
        for i in 0..MAX_BEAMS {
            let b = beams.add(i);
            if (*b).model.is_null() || ((*b).endtime as f64) < cl.time {
                continue;
            }

            // if coming from the player, update the start position
            if (*b).entity == cl.viewentity && !cl.entities.is_null() {
                g::ClTent_Glue_GetEntityOrigin(
                    cl.entities.add(cl.viewentity as usize) as *const c_void,
                    (*b).start.as_mut_ptr(),
                );
            }

            // calculate pitch and yaw
            let mut dist = [0.0f32; 3];
            m::vector_subtract(&(*b).end, &(*b).start, &mut dist);

            let yaw: c_float;
            let pitch: c_float;
            if dist[1] == 0.0 && dist[0] == 0.0 {
                yaw = 0.0;
                if dist[2] > 0.0 {
                    pitch = 90.0;
                } else {
                    pitch = 270.0;
                }
            } else {
                // COMPAT: ADR-010 -- `atan2`/`sqrt` go through the engine's
                // libm, never `f64::atan2`/`f32::sqrt`. The double-width
                // product/quotient and the `(int)` truncation before the
                // store to `float` are C's, transcribed literally.
                let mut y = (libm::atan2(dist[1] as f64, dist[0] as f64) * 180.0
                    / core::f64::consts::PI) as c_int as c_float;
                if y < 0.0 {
                    y += 360.0;
                }
                yaw = y;

                let forward = libm::sqrt((dist[0] * dist[0] + dist[1] * dist[1]) as f64) as c_float;
                let mut p = (libm::atan2(dist[2] as f64, forward as f64) * 180.0
                    / core::f64::consts::PI) as c_int as c_float;
                if p < 0.0 {
                    p += 360.0;
                }
                pitch = p;
            }

            // add new entities for the lightning
            let mut org = (*b).start;
            let mut d = m::vector_normalize(&mut dist);
            while d > 0.0 {
                let ent = CL_NewTempEntity();
                if ent.is_null() {
                    return;
                }
                g::ClTent_Glue_SetBeamEntity(
                    ent as *mut c_void,
                    org.as_ptr(),
                    (*b).model,
                    pitch,
                    yaw,
                    (c::COM_Rand() % 360) as c_float,
                );

                // johnfitz -- use j instead of using i twice, so we don't corrupt memory
                for j in 0..3 {
                    org[j] += dist[j] * 30.0;
                }
                d -= 30.0;
            }
        }
    }
}
