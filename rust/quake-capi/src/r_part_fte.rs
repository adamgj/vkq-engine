//! `Quake/r_part_fte.c` -- FTE's scriptable particle system (Rust migration
//! Phase 7 M10f-2, T10.5, Pattern A whole-file swap).
//!
//! Only the *simulation* half lives here: `r_part_fte.c:88-5583` less
//! `P_LoadTexture` (`:1146-1327`), plus the deferred-queue and
//! particle-update block at `:6325-6784`. The rendering and emit half
//! (`:5585-6324`), `P_LoadTexture`, and everything from
//! `PScript_UpdateParticleTypes` (`:6786`) down stay C verbatim in
//! `Quake/r_part_fte_glue.c`: they are Vulkan- and `gltexture_t`-typed
//! throughout and the renderer belongs to Phase 8 per `ROADMAP.md`.
//!
//! The `PSET_CLASSIC` block at `:3559-3568` is not ported. `PSET_CLASSIC` is
//! never defined anywhere in the tree, so the `fallback`/`pe_classic` vtable
//! is dead code in every build configuration.
//!
//! ## ADR-009 raise-topology audit
//!
//! `r_part_fte.c` has no direct raise site. Four of its callees can re-raise,
//! across six calling functions, and each goes through a `Host_Guard`
//! trampoline in the glue (ADR-009 rule 3):
//!
//! * `Cvar_RegisterVariable` x16 (`:3266-3281`, in `PScript_InitParticles`)
//!   -- `Quake/cvar_cmd_glue.c:300` makes the plain name a `Host_Reraise`
//!   wrapper under `-Duse_rust_cvar`. Guarded by
//!   [`g::FtePart_Glue_RegisterVariable`].
//! * `CL_ClearTrailStates` (`:3315` in `PScript_Shutdown`, `:3532` in
//!   `PScript_ClearParticles`) -- `Quake/cl_main_glue.c:839` makes it a
//!   `ClMain_Raise` wrapper. Guarded by
//!   [`g::FtePart_Glue_ClearTrailStates`].
//! * `CL_RegisterParticles` (`:3644` in `R_ParticleDesc_Callback`'s reload
//!   path, `:6645`) -- `Quake/cl_parse_glue.c:625` makes it a `ClParse_Raise`
//!   wrapper. Guarded by [`g::FtePart_Glue_RegisterParticles`].
//! * `CL_EntityNum` (`:4458`) -- `Quake/cl_parse_glue.c:596` makes it a
//!   `ClParse_Raise` wrapper. Guarded by [`g::FtePart_Glue_EntityNum`], which
//!   only writes its out-parameter on a zero return, so the ADR-009
//!   post-guard invariant holds.
//!
//! Everything else the port reaches is raise-free and is called straight
//! through per ADR-009 rule 4: `CL_AllocDlight`, `Cvar_SetCallback`,
//! `Cmd_AddCommand2`, `Cmd_TokenizeString`, `COM_LoadFile`, `COM_Parse`,
//! `COM_FileBase`, `S_StartSound`, `S_PrecacheSound`, `SV_HullPointContents`,
//! `Q1BSP_RecursiveHullTrace`, `Mem_Alloc`/`Mem_Free`/`Mem_Realloc`, `va`,
//! the `Tasks_*` queries and the `Atomic_*` shims. The seven `Sys_Error`
//! calls in the file are all in the rendering half and abort rather than
//! jumping. `Con_Printf`, `Con_SafePrintf`, `Con_DPrintf` and `Con_DPrintf2`
//! are plain and unguarded, per the standing project decision.
//!
//! `r_part_fte.c:35` is `#define Con_Printf Con_SafePrintf`, so every
//! `Con_Printf` below that line is really `Con_SafePrintf`. The port calls
//! [`c::Con_SafePrintf`] directly at each of those sites and keeps
//! [`c::Con_Printf`] only where the original spelled `Con_Printf` *above* the
//! `#define` -- which is nowhere in the ported range.
//!
//! ## ADR-010 notes
//!
//! `r_part_fte.c:186-187` redefines `sin` and `cos` as 128-entry table
//! lookups for the whole rest of the file:
//!
//! ```c
//! #define sin(x) (psintable[(size_t)(int)((x) * ((SINTABLE_ENTRIES / 2) / M_PI)) % SINTABLE_ENTRIES])
//! ```
//!
//! Every `sin`/`cos` below line 187 is therefore [`psin`]/[`pcos`], not
//! libm. Only `buildsintable` itself (`:177-185`, above the macros) and
//! `R_Part_SkyTri`'s `acos` call libm.
//!
//! C's `float`-typed intermediates are reproduced with an explicit
//! `as c_float` at each assignment, and float-to-int conversions go through
//! [`as_int`]/[`as_uint`]/[`as_usize`].
//!
//! `PerpendicularVector` and `RotatePointAroundVector` (`:4390`, `:4485`)
//! are the two functions covered by ADR-010's NaN-sign exception; the
//! amendment in its Phase 7 (2026-09-03) section narrowed that exception
//! rather than removing it, and both are reached here through
//! [`quake_math::mathlib`], which already implements it.
//!
//! ## Shared state (ADR-007)
//!
//! The seam is bidirectional -- the C tail reads objects this half writes --
//! so a large slice of the file's storage stays C in the glue. The full
//! ownership table, with the citation for each row, is in
//! [`quake_c_sys::r_part_fte`]'s module docs.

use core::ffi::{c_char, c_float, c_int, c_uint, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::libm;
use quake_c_sys::r_part_fte as g;
use quake_c_sys::r_part_fte::{
    beamseg_t, clippeddecal_t, part_type_t, particle_emit_meta_t, particle_t, partsounds_t, pcfg_t,
    plooks_t, ramp_t, skytriblock_t, skytris_t, trailstate_t, BM_ADDA, BM_ADDC, BM_BLEND,
    BM_BLENDCOLOUR, BM_INVMODA, BM_INVMODC, BM_PREMUL, BM_SUBTRACT, BS_DEAD, BS_LASTSEG, BS_NODRAW,
    FTECONTENTS_EMPTY, FTECONTENTS_FLUID, FTECONTENTS_LAVA, FTECONTENTS_PLAYERCLIP,
    FTECONTENTS_SKY, FTECONTENTS_SLIME, FTECONTENTS_SOLID, FTECONTENTS_WATER, MAX_BEAMSEGS,
    MAX_DECALS, MAX_PARTICLES, MAX_QPATH, MAX_TRAILSTATES, PARTICLE_UPDATE_CHUNK_SIZE,
    PS_INRUNLIST, PT_AVERAGETRAIL, PT_BEAM, PT_CDECAL, PT_CITRACER, PT_FRICTION, PT_INVFRAMETIME,
    PT_INVISIBLE, PT_NODLSHADOW, PT_NORMAL, PT_NOSPREADFIRST, PT_NOSPREADLAST, PT_NOSTATE,
    PT_SPARK, PT_SPARKFAN, PT_TEXTUREDSPARK, PT_TROVERWATER, PT_TRUNDERWATER, PT_UDECAL,
    PT_VELOCITY, PT_WORLDSPACERAND, RAMP_DELTA, RAMP_LERP, RAMP_NEAREST, RAMP_NONE,
    SINTABLE_ENTRIES, SM_BALL, SM_BOX, SM_CIRCLE, SM_DISTBALL, SM_FIELD, SM_LAVASPLASH, SM_SPIRAL,
    SM_TELEBOX, SM_TRACER, SM_UNICIRCLE, TASKS_MAX_WORKERS,
};
use quake_math::anorms::R_AVERTEXNORMALS;
use quake_math::mathlib as m;
use quake_math::mathlib::Vec3;
use quake_types::model_mem::{MSurface, QModel, MOD_BRUSH, SURF_PLANEBACK};

use quake_types::host::{EntityOpaque, CA_CONNECTED, MAX_MODELS};

use crate::view::{cl, cls, cvar_value, r_refdef, Entity};
use crate::world::{RhtCtx, Trace, RHT_IMPACT};

/// `Quake/common.h:264` -- `#define COM_PARSE_MAX_TOKEN_SIZE 4096`, the
/// length of the `com_token` buffer `COM_ThreadToken` addresses.
const COM_PARSE_MAX_TOKEN_SIZE: usize = 4096;

/// A `Host_Guard` status: 0 means "no raise". Non-zero must be returned to
/// `Quake/r_part_fte_glue.c` untouched.
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

/// `glquake.h:107` -- `#define P_INVALID -1`.
const P_INVALID: c_int = -1;
/// `common.h:332` -- `#define COM_RAND_MAX 0xFFFFFF`.
const COM_RAND_MAX: c_float = 0xFF_FFFF as c_float;
/// `world.h:79-80` -- `CONTENTMASK_FROMQ1 (CONTENTS_SOLID)`, i.e.
/// `1u << -(-1)`.
const CONTENTMASK_SOLID: c_uint = 1 << 1;
/// `r_part_fte.c:3757` -- `#define NUMVERTEXNORMALS 162`.
const NUMVERTEXNORMALS: usize = 162;

// COMPAT: ADR-010 -- C's implicit float->int conversion. Out-of-range values
// are UB in C and saturate in Rust; the same shim `crate::r_part` uses.
#[inline]
fn as_int(x: c_float) -> c_int {
    x as c_int
}

// COMPAT: ADR-010 -- C's `(size_t)(int)` cast chain in the `sin`/`cos` macros
// (`r_part_fte.c:186-187`). The intermediate `(int)` is what truncates, and
// the `(size_t)` widening of a negative `int` is what makes the following
// `% SINTABLE_ENTRIES` land back in range on two's-complement targets. Both
// steps are reproduced literally.
#[inline]
fn table_index(x: c_float) -> usize {
    // `(SINTABLE_ENTRIES / 2) / M_PI` is `int / double`, so the scale is a
    // `double` and `x` is promoted before the multiply -- not an `f32` one.
    table_index_f64(f64::from(x))
}

/// [`table_index`] for the call sites whose argument is already a `double`
/// (`cl.time + j + m` in `SM_FIELD`), where C never narrows to `float` first.
#[inline]
fn table_index_f64(x: f64) -> usize {
    ((x * ((SINTABLE_ENTRIES as f64 / 2.0) / core::f64::consts::PI)) as c_int) as usize
        % SINTABLE_ENTRIES
}

/// `r_part_fte.c:186` -- `#define sin(x)`, the table lookup that shadows libm
/// for the rest of the file.
///
/// # Safety
/// [`buildsintable`] must have run, which `PScript_Startup` guarantees.
#[inline]
unsafe fn psin(x: c_float) -> c_float {
    // SAFETY: `table_index` is reduced modulo the table length.
    unsafe { g::psintable[table_index(x)] }
}

/// `r_part_fte.c:186` -- `#define sin(x)` applied to a `double` argument.
///
/// # Safety
/// [`buildsintable`] must have run.
#[inline]
unsafe fn psin_f64(x: f64) -> c_float {
    // SAFETY: `table_index_f64` is reduced modulo the table length.
    unsafe { g::psintable[table_index_f64(x)] }
}

/// `r_part_fte.c:187` -- `#define cos(x)`.
///
/// # Safety
/// [`buildsintable`] must have run.
#[inline]
unsafe fn pcos(x: c_float) -> c_float {
    // SAFETY: `table_index` is reduced modulo the table length.
    unsafe { g::pcostable[table_index(x)] }
}

/// `r_part_fte.c:37` -- `#define frandom() (COM_Rand () * (1.0f / (float)COM_RAND_MAX))`.
///
/// # Safety
/// C ABI call into `COM_Rand`.
#[inline]
unsafe fn frandom() -> c_float {
    // SAFETY: `COM_Rand` takes no arguments and touches only its own state.
    unsafe { c::COM_Rand() as c_float * (1.0f32 / COM_RAND_MAX) }
}

/// `r_part_fte.c:38` -- `#define crandom() (COM_Rand () * (2.0f / (float)COM_RAND_MAX) - 1.0f)`.
///
/// # Safety
/// C ABI call into `COM_Rand`.
#[inline]
unsafe fn crandom() -> c_float {
    // SAFETY: as `frandom`.
    unsafe { c::COM_Rand() as c_float * (2.0f32 / COM_RAND_MAX) - 1.0f32 }
}

/// `r_part_fte.c:39` -- `#define hrandom() (COM_Rand () * (1.0f / (float)COM_RAND_MAX) - 0.5f)`.
///
/// # Safety
/// C ABI call into `COM_Rand`.
#[inline]
unsafe fn hrandom() -> c_float {
    // SAFETY: as `frandom`.
    unsafe { c::COM_Rand() as c_float * (1.0f32 / COM_RAND_MAX) - 0.5f32 }
}

/// `r_part_fte.c:452` -- `#define crand() (COM_Rand () % 32767 / 16383.5f - 1)`.
/// The division is `int / float`, so C promotes the left operand to `float`
/// after the integer remainder, not before.
///
/// # Safety
/// C ABI call into `COM_Rand`.
#[inline]
unsafe fn crand() -> c_float {
    // SAFETY: as `frandom`.
    unsafe { (c::COM_Rand() % 32767) as c_float / 16383.5f32 - 1.0f32 }
}

// ---------------------------------------------------------------------------
// State that moved to Rust. Everything in `Quake/r_part_fte_glue.c` stays C
// because a live C reader survives the port (ADR-007); see
// `quake_c_sys::r_part_fte`.

// `r_part_fte.c:168-169` -- `psintable`/`pcostable` stay C storage in
// `Quake/r_part_fte_glue.c` (ADR-007): the render half that stays C reaches
// them through the same `sin`/`cos` table macros, so a live C reader survives
// the port. They are reached here as `g::psintable`/`g::pcostable`.

/// `r_part_fte.c:161` -- `static int pe_default = P_INVALID;`.
static mut PE_DEFAULT: c_int = P_INVALID;
/// `r_part_fte.c:162` -- `static int pe_size2 = P_INVALID;`.
static mut PE_SIZE2: c_int = P_INVALID;
/// `r_part_fte.c:163` -- `static int pe_size3 = P_INVALID;`.
static mut PE_SIZE3: c_int = P_INVALID;
/// `r_part_fte.c:164` -- `static int pe_defaulttrail = P_INVALID;`.
static mut PE_DEFAULTTRAIL: c_int = P_INVALID;

/// `r_part_fte.c:460` -- base of the `Mem_Alloc`'d particle pool.
static mut PARTICLES: *mut particle_t = ptr::null_mut();
/// `r_part_fte.c:461` -- pool length.
static mut R_NUMPARTICLES: c_int = 0;
/// `r_part_fte.c:462` -- cyclic recycle cursor.
static mut R_PARTICLERECYCLE: c_int = 0;

/// `r_part_fte.c:465` -- base of the beamseg pool.
static mut BEAMS: *mut beamseg_t = ptr::null_mut();
/// `r_part_fte.c:466` -- beamseg pool length.
static mut R_NUMBEAMS: c_int = 0;

/// `r_part_fte.c:470` -- base of the decal pool.
static mut DECALS: *mut clippeddecal_t = ptr::null_mut();
/// `r_part_fte.c:471` -- decal pool length.
static mut R_NUMDECALS: c_int = 0;
/// `r_part_fte.c:472` -- cyclic decal recycle cursor.
static mut R_DECALRECYCLE: c_int = 0;

/// `r_part_fte.c:474` -- base of the trailstate pool.
static mut TRAILSTATES: *mut trailstate_t = ptr::null_mut();
/// `r_part_fte.c:475` -- current cyclic index of trailstates.
static mut TS_CYCLE: c_int = 0;
/// `r_part_fte.c:476` -- trailstate pool length.
static mut R_NUMTRAILSTATES: c_int = 0;

/// `r_part_fte.c:478` -- a particle effect was changed, re-evaluate shared
/// looks.
static mut R_PLOOKSDIRTY: bool = false;

/// `r_part_fte.c:448` -- `static pcfg_t *loadedconfigs`.
static mut LOADEDCONFIGS: *mut pcfg_t = ptr::null_mut();

/// `r_part_fte.c:781` -- `static struct partalias_s *partaliaslist`. The node
/// is file-private and never crosses the seam, so it is a plain Rust struct
/// rather than an ADR-011 mirror.
struct PartAlias {
    next: *mut PartAlias,
    from: *mut c_char,
    to: *mut c_char,
}

/// `r_part_fte.c:781`.
static mut PARTALIASLIST: *mut PartAlias = ptr::null_mut();

/// `r_part_fte.c:786-788` -- the anonymous `type` enum inside
/// `associatedeffect_t`.
const AE_TRAIL: c_int = 0;
/// `r_part_fte.c:787`.
const AE_EMIT: c_int = 1;

/// `r_part_fte.c:778-793` -- `associatedeffect_t`. File-private; never
/// crosses the seam.
#[repr(C)]
struct AssociatedEffect {
    next: *mut AssociatedEffect,
    mname: [c_char; MAX_QPATH],
    pname: [c_char; MAX_QPATH],
    flags: c_uint,
    type_: c_int,
}

/// `r_part_fte.c:794` -- `static associatedeffect_t *associatedeffect`.
static mut ASSOCIATEDEFFECT: *mut AssociatedEffect = ptr::null_mut();

/// `r_part_fte.c:764-776` -- the `{oldn, newn}` legacy alias table,
/// NULL-terminated in C. The terminator is dropped: every walk of it in C
/// stops on `oldn == NULL`, which is `.iter()` here.
const LEGACYNAMES: [(&core::ffi::CStr, &core::ffi::CStr); 5] = [
    (c"t_rocket", c"TR_ROCKET"),
    (c"t_grenade", c"TR_GRENADE"),
    (c"t_gib", c"TR_BLOOD"),
    (c"te_plasma", c"TE_TEI_PLASMAHIT"),
    (c"te_smoke", c"TE_TEI_SMOKE"),
];

/// `r_part_fte.c:553-556` -- `trace_line_bounds_t`. Conservative world-space
/// bounds, kept separate from the cold rotation data so the cull loop streams
/// a dense array.
#[repr(C)]
#[derive(Clone, Copy)]
struct TraceLineBounds {
    mins: [c_float; 3],
    maxs: [c_float; 3],
}

/// `r_part_fte.c:558-563` -- `trace_line_ent_t`.
#[repr(C)]
#[derive(Clone, Copy)]
struct TraceLineEnt {
    entnum: c_int,
    rotated: bool,
    /// forward/right/up of the entity, matching the renderer's brush model
    /// rotation
    axis: [[c_float; 3]; 3],
}

/// `r_part_fte.c:565`.
static mut TRACE_LINE_BOUNDS: *mut TraceLineBounds = ptr::null_mut();
/// `r_part_fte.c:566`.
static mut TRACE_LINE_ENTS: *mut TraceLineEnt = ptr::null_mut();
/// `r_part_fte.c:567`.
static mut NUM_TRACE_LINE_ENTS: c_int = 0;
/// `r_part_fte.c:567`.
static mut MAX_TRACE_LINE_ENTS: c_int = 0;
/// `r_part_fte.c:568` -- `= -1`.
static mut TRACE_LINE_CACHE_VALID_COUNT: c_int = -1;
/// `r_part_fte.c:569` -- `= -1`.
static mut TRACE_LINE_PREPARED_FRAMECOUNT: c_int = -1;

/// `r_part_fte.c:3758` -- `static vec2_t avelocities[NUMVERTEXNORMALS]`.
static mut AVELOCITIES: [[c_float; 2]; NUMVERTEXNORMALS] = [[0.0; 2]; NUMVERTEXNORMALS];

/// `r_part_fte.c:6588` -- `static qboolean p_doflurry`.
static mut P_DOFLURRY: bool = false;

/// `r_part_fte.c:6324` -- `static uint32_t particle_trace_limit`.
static mut PARTICLE_TRACE_LIMIT: u32 = 0;
/// `r_part_fte.c:6324` -- `static uint32_t particle_update_seed`.
static mut PARTICLE_UPDATE_SEED: u32 = 0;

// ---------------------------------------------------------------------------
// gl_model.h flags the effect association writes into `qmodel_t::flags`.

/// `gl_model.h:610` -- particle effect completely replaces the model.
const MOD_EMITREPLACE: c_uint = 2048;
/// `gl_model.h:611` -- particle effect is emitted forwards, rather than
/// downwards.
const MOD_EMITFORWARDS: c_uint = 4096;

/// `q_minmax.h` -- `q_max` for `int`.
#[inline]
fn q_max_i(a: c_int, b: c_int) -> c_int {
    if a > b {
        a
    } else {
        b
    }
}

/// `q_minmax.h` -- `q_min` for `float`.
#[inline]
fn q_min_f(a: c_float, b: c_float) -> c_float {
    if a < b {
        a
    } else {
        b
    }
}

/// `q_minmax.h` -- `q_max` for `float`.
#[inline]
fn q_max_f(a: c_float, b: c_float) -> c_float {
    if a > b {
        a
    } else {
        b
    }
}

// ---------------------------------------------------------------------------
// r_part_fte.c:88-127 -- the two vector helpers the file defines for itself.
// Both have external linkage in C but no caller outside this translation
// unit, so they are private here and the oracle exports neither name.

/// `r_part_fte.c:88` -- `vec_t VectorNormalize2 (const vec3_t v, vec3_t out)`.
/// Note the return is the *pre-reciprocal* length, and that the zero branch
/// returns the un-square-rooted zero -- both reproduced literally.
#[inline]
fn vector_normalize2(v: &Vec3, out: &mut Vec3) -> c_float {
    let mut length = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];

    if length != 0.0 {
        length = libm::sqrt(length as f64) as c_float;
        let ilength = 1.0 / length;
        out[0] = v[0] * ilength;
        out[1] = v[1] * ilength;
        out[2] = v[2] * ilength;
    } else {
        out[0] = 0.0;
        out[1] = 0.0;
        out[2] = 0.0;
    }

    length
}

/// `r_part_fte.c:109` -- `void VectorVectors (const vec3_t forward, vec3_t
/// right, vec3_t up)`.
#[inline]
fn vector_vectors(forward: &Vec3, right: &mut Vec3, up: &mut Vec3) {
    if forward[0] == 0.0 && forward[1] == 0.0 {
        if forward[2] != 0.0 {
            right[1] = -1.0;
        } else {
            right[1] = 0.0;
        }
        right[0] = 0.0;
        right[2] = 0.0;
    } else {
        right[0] = forward[1];
        right[1] = -forward[0];
        right[2] = 0.0;
        m::vector_normalize(right);
    }
    let r = *right;
    m::cross_product(&r, forward, up);
}

/// `r_part_fte.c:177` -- fills the two lookup tables. This is the one place
/// in the file where `sin`/`cos` are still libm: it sits *above* the
/// `#define`s at `:186-187`.
///
/// # Safety
/// Writes the two file-scope tables; single-threaded (`PScript_Startup`).
unsafe fn buildsintable() {
    // SAFETY: exclusive access during startup.
    unsafe {
        for i in 0..SINTABLE_ENTRIES {
            let a = (i as f64 * core::f64::consts::PI) / (SINTABLE_ENTRIES as f64 / 2.0);
            g::psintable[i] = libm::sin(a) as c_float;
            g::pcostable[i] = libm::cos(a) as c_float;
        }
    }
}

// ---------------------------------------------------------------------------
// r_part_fte.c:538-758 -- the trace-line cache, CL_TraceLine and the
// contents probe.

/// `client.h` -- `&cl.entities[i]`. `cl.entities` is an opaque blob whose
/// stride is authoritative (see [`crate::view::Entity`]).
///
/// # Safety
/// `i` must be below `cl.num_entities`.
#[inline]
unsafe fn cl_entity(i: c_int) -> *mut Entity {
    // SAFETY: caller bounds `i`; the stride comes from the opaque mirror.
    unsafe { crate::view::cl_entity(i) }
}

/// `r_part_fte.c:538` -- a drop-in replacement for FTE's
/// `SV_RecursiveHullCheck`, built on the ported `Q1BSP_RecursiveHullTrace`.
///
/// # Safety
/// `hull` must be a loaded hull; `p1`, `p2` and `trace` must be valid.
unsafe fn q1bsp_recursive_hull_check(
    hull: *mut quake_types::model_mem::Hull,
    num: c_int,
    p1f: c_float,
    p2f: c_float,
    p1: *mut c_float,
    p2: *mut c_float,
    trace: *mut Trace,
) -> bool {
    // SAFETY: pointer contracts per the fn docs.
    unsafe {
        let mut ctx = RhtCtx {
            hitcontents: CONTENTMASK_SOLID,
            start: [*p1, *p1.add(1), *p1.add(2)],
            end: [*p2, *p2.add(1), *p2.add(2)],
            clipnodes: (*hull).clipnodes,
            planes: (*hull).planes,
        };

        crate::world::Q1BSP_RecursiveHullTrace(&mut ctx, num, p1f, p2f, p1, p2, trace) != RHT_IMPACT
    }
}

/// `r_part_fte.c:574` -- rebuilds the brush-entity list if it is stale and
/// refreshes the per-frame entity transforms. Must be called from a single
/// thread before [`quake_rs_ftepart_trace_line`] can be used concurrently.
///
/// # Safety
/// `cl.entities` must be sized by `cl.num_entities`.
unsafe fn cl_prepare_trace_line_entities() {
    // SAFETY: `cl` is Rust-owned (crate::cl_main); the pool pointers below
    // come from Mem_Realloc and are sized by MAX_TRACE_LINE_ENTS.
    unsafe {
        if TRACE_LINE_CACHE_VALID_COUNT != c::view::r_trace_line_cache_counter {
            NUM_TRACE_LINE_ENTS = 0;
            for i in 1..cl.num_entities {
                let ent = cl_entity(i);
                let model = (*ent).model;
                if model.is_null()
                    || (*model).needload
                    || (*model).type_ != MOD_BRUSH
                    || model == cl.worldmodel
                {
                    continue;
                }
                if NUM_TRACE_LINE_ENTS == MAX_TRACE_LINE_ENTS {
                    MAX_TRACE_LINE_ENTS = q_max_i(256, MAX_TRACE_LINE_ENTS * 2);
                    TRACE_LINE_ENTS = c::Mem_Realloc(
                        TRACE_LINE_ENTS.cast::<c_void>(),
                        MAX_TRACE_LINE_ENTS as usize * core::mem::size_of::<TraceLineEnt>(),
                    )
                    .cast::<TraceLineEnt>();
                    TRACE_LINE_BOUNDS = c::Mem_Realloc(
                        TRACE_LINE_BOUNDS.cast::<c_void>(),
                        MAX_TRACE_LINE_ENTS as usize * core::mem::size_of::<TraceLineBounds>(),
                    )
                    .cast::<TraceLineBounds>();
                }
                (*TRACE_LINE_ENTS.add(NUM_TRACE_LINE_ENTS as usize)).entnum = i;
                NUM_TRACE_LINE_ENTS += 1;
            }
            TRACE_LINE_CACHE_VALID_COUNT = c::view::r_trace_line_cache_counter;
            TRACE_LINE_PREPARED_FRAMECOUNT = -1;
        }

        // entity origins and angles change without invalidating the list;
        // refresh once per frame
        if TRACE_LINE_PREPARED_FRAMECOUNT == c::host_framecount {
            return;
        }
        TRACE_LINE_PREPARED_FRAMECOUNT = c::host_framecount;

        for i in 0..NUM_TRACE_LINE_ENTS {
            let tent = TRACE_LINE_ENTS.add(i as usize);
            let tbounds = TRACE_LINE_BOUNDS.add(i as usize);
            let ent = cl_entity((*tent).entnum);
            (*tent).rotated =
                (*ent).angles[0] != 0.0 || (*ent).angles[1] != 0.0 || (*ent).angles[2] != 0.0;
            let model = (*ent).model;
            if (*tent).rotated {
                let angles = (*ent).angles;
                let (mut f, mut r, mut u) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 3]);
                m::angle_vectors(&angles, &mut f, &mut r, &mut u);
                (*tent).axis[0] = f;
                (*tent).axis[1] = r;
                (*tent).axis[2] = u;
                // rmins/rmaxs are the radius cube, valid for any rotation
                for j in 0..3 {
                    (*tbounds).mins[j] = (*ent).origin[j] + (*model).rmins[j];
                    (*tbounds).maxs[j] = (*ent).origin[j] + (*model).rmaxs[j];
                }
            } else {
                for j in 0..3 {
                    (*tbounds).mins[j] = (*ent).origin[j] + (*model).mins[j];
                    (*tbounds).maxs[j] = (*ent).origin[j] + (*model).maxs[j];
                }
            }
        }
    }
}

/// `r_part_fte.c:625` -- `float CL_TraceLine (vec3_t start, vec3_t end,
/// vec3_t impact, vec3_t normal, int *entnum)`. External linkage in C
/// (`client.h:435`, called from `cl_tent.c:70`); the glue re-exports the
/// plain name as a thin wrapper over this core, because cbindgen cannot
/// spell `vec3_t`.
///
/// # Safety
/// C ABI entry point. `start`, `end`, `impact` and `normal` must each address
/// three writable `float`s; `entnum` may be null.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_trace_line(
    start: *mut c_float,
    end: *mut c_float,
    impact: *mut c_float,
    normal: *mut c_float,
    entnum: *mut c_int,
) -> c_float {
    // SAFETY: pointer contracts per the fn docs.
    unsafe {
        for i in 0..3 {
            *impact.add(i) = *end.add(i);
        }
        *normal = 0.0;
        *normal.add(1) = 0.0;
        *normal.add(2) = 1.0;
        if !entnum.is_null() {
            *entnum = 0;
        }

        // no-op during the parallel particle update: the list is prepared
        // beforehand
        cl_prepare_trace_line_entities();

        // the world usually clips the line the most; trace it first and only
        // test brush entities whose bounds overlap the remaining segment
        let world = cl.worldmodel;
        let mut trace = core::mem::zeroed::<Trace>();
        trace.fraction = 1.0;
        let hull = ptr::addr_of_mut!((*world).hulls[0]);
        q1bsp_recursive_hull_check(
            hull,
            (*hull).firstclipnode,
            0.0,
            1.0,
            start,
            end,
            &mut trace,
        );
        let mut frac = trace.fraction;
        if frac < 1.0 {
            for i in 0..3 {
                *impact.add(i) = trace.endpos[i];
                *normal.add(i) = trace.plane.normal[i];
            }
            if frac <= 0.0 {
                return frac;
            }
        }

        let mut seg_mins = [0.0f32; 3];
        let mut seg_maxs = [0.0f32; 3];
        for i in 0..3 {
            seg_mins[i] = q_min_f(*start.add(i), *impact.add(i)) - 1.0;
            seg_maxs[i] = q_max_f(*start.add(i), *impact.add(i)) + 1.0;
        }

        for i in 0..NUM_TRACE_LINE_ENTS {
            let tbounds = &*TRACE_LINE_BOUNDS.add(i as usize);
            if tbounds.mins[0] > seg_maxs[0]
                || tbounds.maxs[0] < seg_mins[0]
                || tbounds.mins[1] > seg_maxs[1]
                || tbounds.maxs[1] < seg_mins[1]
                || tbounds.mins[2] > seg_maxs[2]
                || tbounds.maxs[2] < seg_mins[2]
            {
                continue;
            }

            let tent = &*TRACE_LINE_ENTS.add(i as usize);
            let ent = cl_entity(tent.entnum);
            let mut relstart = [0.0f32; 3];
            let mut relend = [0.0f32; 3];
            if tent.rotated {
                // rotate the segment into entity space, matching how the
                // renderer rotates brush models
                let mut temp = [0.0f32; 3];
                #[allow(clippy::needless_range_loop)] // temp and ent->origin in lockstep
                for j in 0..3 {
                    temp[j] = *start.add(j) - (*ent).origin[j];
                }
                relstart[0] = m::dot_product(&temp, &tent.axis[0]);
                relstart[1] = -m::dot_product(&temp, &tent.axis[1]);
                relstart[2] = m::dot_product(&temp, &tent.axis[2]);
                #[allow(clippy::needless_range_loop)] // temp and ent->origin in lockstep
                for j in 0..3 {
                    temp[j] = *end.add(j) - (*ent).origin[j];
                }
                relend[0] = m::dot_product(&temp, &tent.axis[0]);
                relend[1] = -m::dot_product(&temp, &tent.axis[1]);
                relend[2] = m::dot_product(&temp, &tent.axis[2]);
            } else {
                for j in 0..3 {
                    relstart[j] = *start.add(j) - (*ent).origin[j];
                    relend[j] = *end.add(j) - (*ent).origin[j];
                }
            }

            trace = core::mem::zeroed::<Trace>();
            trace.fraction = 1.0;
            let ehull = ptr::addr_of_mut!((*(*ent).model).hulls[0]);
            q1bsp_recursive_hull_check(
                ehull,
                (*ehull).firstclipnode,
                0.0,
                1.0,
                relstart.as_mut_ptr(),
                relend.as_mut_ptr(),
                &mut trace,
            );

            if frac > trace.fraction {
                frac = trace.fraction;

                if tent.rotated {
                    // rotate the impact point and normal back to world space
                    for j in 0..3 {
                        *impact.add(j) = (*ent).origin[j] + (trace.endpos[0] * tent.axis[0][j])
                            - (trace.endpos[1] * tent.axis[1][j])
                            + (trace.endpos[2] * tent.axis[2][j]);
                        *normal.add(j) = (trace.plane.normal[0] * tent.axis[0][j])
                            - (trace.plane.normal[1] * tent.axis[1][j])
                            + (trace.plane.normal[2] * tent.axis[2][j]);
                    }
                } else {
                    for j in 0..3 {
                        *impact.add(j) = trace.endpos[j] + (*ent).origin[j];
                        *normal.add(j) = trace.plane.normal[j];
                    }
                }

                if !entnum.is_null() {
                    *entnum = tent.entnum;
                }
                if frac <= 0.0 {
                    break;
                }

                // shrink the segment bounds to the new impact point
                for j in 0..3 {
                    seg_mins[j] = q_min_f(*start.add(j), *impact.add(j)) - 1.0;
                    seg_maxs[j] = q_max_f(*start.add(j), *impact.add(j)) + 1.0;
                }
            }
        }
        frac
    }
}

/// `r_part_fte.c:740` -- `static unsigned int CL_PointContentsMask (vec3_t p)`.
///
/// # Safety
/// `p` must address three `float`s and `cl.worldmodel` must be loaded.
unsafe fn cl_point_contents_mask(p: *mut c_float) -> c_uint {
    /// `r_part_fte.c:742-749` -- `static const unsigned int cont_qtof[]`.
    const CONT_QTOF: [c_uint; 7] = [
        0, // invalid
        FTECONTENTS_EMPTY,
        FTECONTENTS_SOLID,
        FTECONTENTS_WATER,
        FTECONTENTS_SLIME,
        FTECONTENTS_LAVA,
        FTECONTENTS_SKY,
    ];

    // SAFETY: `p` per the fn docs; the world hull is loaded.
    unsafe {
        let hull = ptr::addr_of_mut!((*cl.worldmodel).hulls[0]);
        // C computes `unsigned int cont = -SV_HullPointContents (...)`, i.e.
        // the negation happens in `int` and is then converted to `unsigned`.
        let cont = (-crate::world::SV_HullPointContents(hull, 0, p)) as c_uint;
        if (cont as usize) < CONT_QTOF.len() {
            CONT_QTOF[cont as usize]
        } else {
            // `cont_qtof[-(CONTENTS_WATER)]` == `cont_qtof[3]` -- assume water
            CONT_QTOF[3]
        }
    }
}

// ---------------------------------------------------------------------------
// r_part_fte.c:796-1145 -- effect association, aliasing and type allocation.

/// `r_part_fte.c:796` -- `static void PScript_AssociateEffect_f (void)`.
/// Bound to `r_trail` and `r_effect`; both are registered through the glue so
/// that a raise out of `Cmd_Argv` unwinds through a C frame (ADR-009 rule 3).
///
/// # Safety
/// Console command body; runs on the main thread.
unsafe fn pscript_associate_effect_f() {
    // SAFETY: `Cmd_Argv` returns NUL-terminated storage owned by the command
    // buffer, valid until the next tokenize.
    unsafe {
        let modelname = c::Cmd_Argv(1);
        let effectname = c::Cmd_Argv(2);
        let mut flags: c_uint = 0;
        let type_;
        let mut ae: *mut AssociatedEffect;

        if g::strcmp(c::Cmd_Argv(0), c"r_trail".as_ptr()) == 0 {
            type_ = AE_TRAIL;
        } else {
            type_ = AE_EMIT;
            for i in 3..c::Cmd_Argc() {
                let fn_ = c::Cmd_Argv(i);
                if g::strcmp(fn_, c"replace".as_ptr()) == 0 || g::strcmp(fn_, c"1".as_ptr()) == 0 {
                    flags |= MOD_EMITREPLACE;
                } else if g::strcmp(fn_, c"forwards".as_ptr()) == 0
                    || g::strcmp(fn_, c"forward".as_ptr()) == 0
                {
                    flags |= MOD_EMITFORWARDS;
                } else if g::strcmp(fn_, c"0".as_ptr()) == 0 {
                    // 1 or 0 are legacy, meaning replace or not
                } else {
                    c::Con_DPrintf(
                        c"%s %s: unknown flag %s\n".as_ptr(),
                        c::Cmd_Argv(0),
                        modelname,
                        fn_,
                    );
                }
            }
        }

        if !g::strstr(modelname, c"player".as_ptr()).is_null()
            || !g::strstr(modelname, c"eyes".as_ptr()).is_null()
            || !g::strstr(modelname, c"flag".as_ptr()).is_null()
            || !g::strstr(modelname, c"tf_stan".as_ptr()).is_null()
            || !g::strstr(modelname, c".bsp".as_ptr()).is_null()
            || !g::strstr(modelname, c"turr".as_ptr()).is_null()
        {
            // there is a very real possibility of attaching 'large' effects to
            // models so that they become more visible
            c::Con_SafePrintf(
                c"Sorry: Not allowed to attach effects to model \"%s\"\n".as_ptr(),
                modelname,
            );
            return;
        }

        if g::strlen(modelname) >= MAX_QPATH || g::strlen(effectname) >= MAX_QPATH {
            return;
        }

        // replace the old one if it exists
        ae = ASSOCIATEDEFFECT;
        while !ae.is_null() {
            if g::strcmp(ptr::addr_of!((*ae).mname).cast::<c_char>(), modelname) == 0
                && ((*ae).type_ == AE_TRAIL) == (type_ == AE_TRAIL)
            {
                break;
            }
            ae = (*ae).next;
        }
        if ae.is_null() {
            ae = c::Mem_Alloc(core::mem::size_of::<AssociatedEffect>()).cast::<AssociatedEffect>();
            g::strcpy(ptr::addr_of_mut!((*ae).mname).cast::<c_char>(), modelname);
            (*ae).next = ASSOCIATEDEFFECT;
            ASSOCIATEDEFFECT = ae;
        }
        g::strcpy(ptr::addr_of_mut!((*ae).pname).cast::<c_char>(), effectname);
        (*ae).type_ = type_;
        (*ae).flags = flags;

        R_PLOOKSDIRTY = true;
    }
}

/// `r_part_fte.c:855` -- `static void P_PartRedirect_f (void)`.
///
/// # Safety
/// Console command body; runs on the main thread.
unsafe fn p_part_redirect_f() {
    // SAFETY: `Cmd_Argv` storage per `pscript_associate_effect_f`; the alias
    // list is file-private and single-threaded.
    unsafe {
        let from = c::Cmd_Argv(1);
        let to = c::Cmd_Argv(2);

        // user wants to list all
        if *from == 0 {
            let mut l = PARTALIASLIST;
            while !l.is_null() {
                c::Con_SafePrintf(c"%s -> %s\n".as_ptr(), (*l).from, (*l).to);
                l = (*l).next;
            }
            return;
        }

        // unlink the current value
        let mut link = ptr::addr_of_mut!(PARTALIASLIST);
        loop {
            let l = *link;
            if l.is_null() {
                break;
            }
            if g::q_strcasecmp((*l).from, from) == 0 {
                // they didn't specify a to, so just print out this one effect
                // without removing it.
                if c::Cmd_Argc() == 2 {
                    c::Con_SafePrintf(
                        c"particle %s is currently remapped to %s\n".as_ptr(),
                        (*l).from,
                        (*l).to,
                    );
                    return;
                }
                *link = (*l).next;
                c::Mem_Free(l.cast::<c_void>());
                break;
            }
            link = ptr::addr_of_mut!((*l).next);
        }

        // create a new entry.
        if *to != 0 && g::q_strcasecmp(from, to) != 0 {
            let l = c::Mem_Alloc(
                core::mem::size_of::<PartAlias>() + g::strlen(from) + g::strlen(to) + 2,
            )
            .cast::<PartAlias>();
            (*l).from = l.add(1).cast::<c_char>();
            g::strcpy((*l).from, from);
            (*l).to = (*l).from.add(g::strlen((*l).from) + 1);
            g::strcpy((*l).to, to);
            (*l).next = PARTALIASLIST;
            PARTALIASLIST = l;
        }

        R_PLOOKSDIRTY = true;
    }
}

/// `r_part_fte.c:898` -- `void PScript_UpdateModelEffects (qmodel_t *mod)`.
///
/// # Safety
/// `mod` must be a loaded model.
unsafe fn pscript_update_model_effects(model: *mut QModel) {
    // SAFETY: `model` per the fn docs; the association list is file-private.
    unsafe {
        (*model).emiteffect = P_INVALID;
        (*model).traileffect = P_INVALID;
        let mut ae = ASSOCIATEDEFFECT;
        while !ae.is_null() {
            if g::strcmp(
                ptr::addr_of!((*ae).mname).cast::<c_char>(),
                ptr::addr_of!((*model).name).cast::<c_char>(),
            ) == 0
            {
                match (*ae).type_ {
                    AE_TRAIL => {
                        (*model).traileffect =
                            pscript_find_particle_type(ptr::addr_of!((*ae).pname).cast::<c_char>());
                    }
                    _ => {
                        (*model).emiteffect =
                            pscript_find_particle_type(ptr::addr_of!((*ae).pname).cast::<c_char>());
                        (*model).flags &= !((MOD_EMITREPLACE | MOD_EMITFORWARDS) as c_int);
                        (*model).flags |= (*ae).flags as c_int;
                    }
                }
            }
            ae = (*ae).next;
        }
    }
}

/// `r_part_fte.c:922` -- `static part_type_t *P_GetParticleType (const char
/// *config, const char *name)`. Reallocating `part_type` moves the whole
/// array, so every interior pointer (`part_run_list` and each `nexttorun`)
/// is rebased by the same byte delta -- reproduced exactly, including the
/// `char *` arithmetic.
///
/// # Safety
/// `config` and `name` must be NUL-terminated.
unsafe fn p_get_particle_type(
    mut config: *const c_char,
    mut name: *const c_char,
) -> *mut part_type_t {
    // SAFETY: string contracts per the fn docs; `part_type` is the glue-owned
    // array (ADR-007) and is only ever resized here, on the main thread.
    unsafe {
        let oldlist = g::part_type;
        let mut cfgbuf = [0 as c_char; MAX_QPATH];
        let dot = g::strchr(name, b'.' as c_int);
        if !dot.is_null() && (dot.offset_from(name) as usize) < MAX_QPATH - 1 {
            let n = dot.offset_from(name) as usize;
            config = cfgbuf.as_ptr();
            g::memcpy(
                cfgbuf.as_mut_ptr().cast::<c_void>(),
                name.cast::<c_void>(),
                n,
            );
            *cfgbuf.as_mut_ptr().add(n) = 0;
            name = dot.add(1);
        }

        for (oldn, newn) in LEGACYNAMES.iter() {
            if g::strcmp(name, oldn.as_ptr()) == 0 {
                name = newn.as_ptr();
                break;
            }
        }
        for i in 0..g::numparticletypes {
            let ptype = g::part_type.add(i as usize);
            if g::q_strcasecmp(ptr::addr_of!((*ptype).name).cast::<c_char>(), name) == 0
                // must be an exact match.
                && g::q_strcasecmp(ptr::addr_of!((*ptype).config).cast::<c_char>(), config) == 0
            {
                return ptype;
            }
        }

        g::part_type = c::Mem_Realloc(
            g::part_type.cast::<c_void>(),
            core::mem::size_of::<part_type_t>() * (g::numparticletypes as usize + 1),
        )
        .cast::<part_type_t>();
        let ptype = g::part_type.add(g::numparticletypes as usize);
        g::numparticletypes += 1;
        g::memset(
            ptype.cast::<c_void>(),
            0,
            core::mem::size_of::<part_type_t>(),
        );
        g::q_strlcpy(
            ptr::addr_of_mut!((*ptype).name).cast::<c_char>(),
            name,
            core::mem::size_of_val(&(*ptype).name),
        );
        g::q_strlcpy(
            ptr::addr_of_mut!((*ptype).config).cast::<c_char>(),
            config,
            core::mem::size_of_val(&(*ptype).config),
        );
        (*ptype).assoc = P_INVALID;
        (*ptype).inwater = P_INVALID;
        (*ptype).cliptype = P_INVALID;
        (*ptype).emit = P_INVALID;

        if !oldlist.is_null() {
            let delta = g::part_type
                .cast::<c_char>()
                .offset_from(oldlist.cast::<c_char>());
            if !g::part_run_list.is_null() {
                g::part_run_list = g::part_run_list
                    .cast::<c_char>()
                    .offset(delta)
                    .cast::<part_type_t>();
            }

            for i in 0..g::numparticletypes {
                let pt = g::part_type.add(i as usize);
                if !(*pt).nexttorun.is_null() {
                    (*pt).nexttorun = (*pt)
                        .nexttorun
                        .cast::<c_char>()
                        .offset(delta)
                        .cast::<part_type_t>();
                }
            }
        }

        (*ptype).loaded = 0;
        (*ptype).ramp = ptr::null_mut();
        (*ptype).particles = ptr::null_mut();
        (*ptype).beams = ptr::null_mut();

        R_PLOOKSDIRTY = true;
        ptype
    }
}

/// `r_part_fte.c:980` -- unconditionally allocates a particle object, so
/// out-of-order allocations work. Returns the index.
///
/// # Safety
/// String contracts as [`p_get_particle_type`].
unsafe fn p_allocate_particle_type(config: *const c_char, name: *const c_char) -> c_int {
    // SAFETY: per the fn docs.
    unsafe {
        let pt = p_get_particle_type(config, name);
        pt.offset_from(g::part_type) as c_int
    }
}

/// `r_part_fte.c:986` -- `static void PScript_RetintEffect (part_type_t *to,
/// part_type_t *from, const char *colourcodes)`.
///
/// # Safety
/// `to` and `from` must be live entries of `part_type`; `to` must already be
/// purged. `colourcodes` must be NUL-terminated.
unsafe fn pscript_retint_effect(
    to: *mut part_type_t,
    from: *mut part_type_t,
    colourcodes: *const c_char,
) {
    // SAFETY: per the fn docs.
    unsafe {
        let mut name = [0 as c_char; MAX_QPATH];
        let mut config = [0 as c_char; MAX_QPATH];

        g::q_strlcpy(
            name.as_mut_ptr(),
            ptr::addr_of!((*to).name).cast::<c_char>(),
            core::mem::size_of_val(&(*to).name),
        );
        g::q_strlcpy(
            config.as_mut_ptr(),
            ptr::addr_of!((*to).config).cast::<c_char>(),
            core::mem::size_of_val(&(*to).config),
        );

        // 'to' was already purged, so we don't need to care about that.
        g::memcpy(
            to.cast::<c_void>(),
            from.cast::<c_void>(),
            core::mem::size_of::<part_type_t>(),
        );

        g::q_strlcpy(
            ptr::addr_of_mut!((*to).name).cast::<c_char>(),
            name.as_ptr(),
            core::mem::size_of_val(&(*to).name),
        );
        g::q_strlcpy(
            ptr::addr_of_mut!((*to).config).cast::<c_char>(),
            config.as_ptr(),
            core::mem::size_of_val(&(*to).config),
        );

        // make sure 'to' has its own copy of any lists, so that we don't have
        // issues when freeing this memory again.
        if !(*to).sounds.is_null() {
            let n = (*to).numsounds as usize * core::mem::size_of::<partsounds_t>();
            (*to).sounds = c::Mem_Alloc(n).cast::<partsounds_t>();
            g::memcpy(
                (*to).sounds.cast::<c_void>(),
                (*from).sounds.cast::<c_void>(),
                n,
            );
        }
        if !(*to).ramp.is_null() {
            let n = (*to).rampindexes as usize * core::mem::size_of::<ramp_t>();
            (*to).ramp = c::Mem_Alloc(n).cast::<ramp_t>();
            g::memcpy(
                (*to).ramp.cast::<c_void>(),
                (*from).ramp.cast::<c_void>(),
                n,
            );
        }

        // 'from' might still have some links so we need to clear those out.
        (*to).nexttorun = ptr::null_mut();
        (*to).particles = ptr::null_mut();
        (*to).clippeddecals = ptr::null_mut();
        (*to).beams = ptr::null_mut();
        (*to).slooks = ptr::addr_of_mut!((*to).looks);
        R_PLOOKSDIRTY = true;

        let mut end: *mut c_char = ptr::null_mut();
        (*to).colorindex = g::strtoul(colourcodes, &mut end, 10) as c_int;
        if *end == b'_' as c_char {
            end = end.add(1);
        }
        (*to).colorrand = g::strtoul(end, &mut end, 10) as c_int;
    }
}

/// `r_part_fte.c:1024` -- `int PScript_FindParticleType (const char
/// *fullname)`. Public interface: get without creating.
///
/// # Safety
/// `fullname` must be NUL-terminated.
unsafe fn pscript_find_particle_type(fullname: *const c_char) -> c_int {
    // SAFETY: per the fn docs; all lists walked here are file-private or the
    // glue-owned `part_type` array.
    unsafe {
        let mut i: c_int;
        let mut ptype: *mut part_type_t = ptr::null_mut();
        let mut cfg = [0 as c_char; MAX_QPATH];
        let mut name = fullname;

        // check particle aliases, mostly for tex_sky1 -> weather.te_rain for
        // example, or whatever
        let mut recurselimit = 5;
        let mut l = PARTALIASLIST;
        while !l.is_null() {
            if g::q_strcasecmp((*l).from, name) == 0 {
                name = (*l).to;

                recurselimit -= 1;
                if recurselimit + 1 > 0 {
                    l = PARTALIASLIST;
                } else {
                    return P_INVALID;
                }
            } else {
                l = (*l).next;
            }
        }

        let dot = g::strchr(name, b'.' as c_int);
        if !dot.is_null() && (dot.offset_from(name) as usize) < MAX_QPATH - 1 {
            let n = dot.offset_from(name) as usize;
            g::memcpy(cfg.as_mut_ptr().cast::<c_void>(), name.cast::<c_void>(), n);
            cfg[n] = 0;
            name = dot.add(1);
        } else {
            cfg[0] = 0;
        }

        for (oldn, newn) in LEGACYNAMES.iter() {
            if g::strcmp(name, oldn.as_ptr()) == 0 {
                name = newn.as_ptr();
                break;
            }
        }

        if cfg[0] != 0 {
            // favour the namespace if one is specified
            i = 0;
            while i < g::numparticletypes {
                let pt = g::part_type.add(i as usize);
                if g::q_strcasecmp(ptr::addr_of!((*pt).name).cast::<c_char>(), name) == 0
                    && g::q_strcasecmp(ptr::addr_of!((*pt).config).cast::<c_char>(), cfg.as_ptr())
                        == 0
                {
                    ptype = pt;
                    break;
                }
                i += 1;
            }
        } else {
            // but be prepared to load it from any namespace if its not got a
            // namespace specified.
            i = 0;
            while i < g::numparticletypes {
                let pt = g::part_type.add(i as usize);
                if g::q_strcasecmp(ptr::addr_of!((*pt).name).cast::<c_char>(), name) == 0 {
                    ptype = pt;
                    if (*ptype).loaded != 0 {
                        // (mostly) ignore ones that are not currently loaded
                        break;
                    }
                }
                i += 1;
            }
        }
        if ptype.is_null() || (*ptype).loaded == 0 {
            if g::q_strncasecmp(name, c"te_explosion2_".as_ptr(), 14) == 0 {
                let from =
                    pscript_find_particle_type(g::va(c"%s.te_explosion2".as_ptr(), cfg.as_ptr()));
                if from != P_INVALID {
                    let to = p_allocate_particle_type(cfg.as_ptr(), name);
                    pscript_retint_effect(
                        g::part_type.add(to as usize),
                        g::part_type.add(from as usize),
                        name.add(14),
                    );
                    return to;
                }
            }
            if cfg[0] != 0 && p_load_particle_set(cfg.as_mut_ptr(), true, true) {
                return pscript_find_particle_type(fullname);
            }

            return P_INVALID;
        }
        i
    }
}

/// `r_part_fte.c:1131` -- `static int CheckAssosiation (const char *config,
/// const char *name, int from)`. The console message keeps the original
/// spelling.
///
/// # Safety
/// String contracts as [`p_get_particle_type`].
unsafe fn check_assosiation(config: *const c_char, name: *const c_char, from: c_int) -> c_int {
    // SAFETY: per the fn docs.
    unsafe {
        let orig = p_allocate_particle_type(config, name);
        let mut to = orig;

        while to != P_INVALID {
            if to == from {
                c::Con_SafePrintf(
                    c"Assosiation of %s would cause infinate loop\n".as_ptr(),
                    name,
                );
                return P_INVALID;
            }
            to = (*g::part_type.add(to as usize)).assoc;
        }
        orig
    }
}

// ---------------------------------------------------------------------------
// r_part_fte.c:1328-1435 -- effect reset and the config line reader.

/// `r_part_fte.c:1328` -- `static void P_ResetToDefaults (part_type_t
/// *ptype)`.
///
/// # Safety
/// `ptype` must be a live entry of the glue-owned `part_type` array.
unsafe fn p_reset_to_defaults(ptype: *mut part_type_t) {
    // SAFETY: `ptype` per the fn docs. The free lists and the run list are
    // glue-owned but only ever touched from the main thread here.
    unsafe {
        // go with a lazy clear of list.. mark everything as DEAD and let
        // the beam rendering handle removing nodes
        let mut beamsegs = (*ptype).beams;
        while !beamsegs.is_null() {
            (*beamsegs).flags |= BS_DEAD;
            beamsegs = (*beamsegs).next;
        }

        // forget any particles before its wiped
        while !(*ptype).particles.is_null() {
            let parts = (*(*ptype).particles).next;
            (*(*ptype).particles).next = g::free_particles;
            g::free_particles = (*ptype).particles;
            (*ptype).particles = parts;
        }

        // if we're in the runstate loop through and remove from linked list
        if (*ptype).state & PS_INRUNLIST != 0 {
            if g::part_run_list == ptype {
                g::part_run_list = (*g::part_run_list).nexttorun;
            } else {
                let mut torun = g::part_run_list;
                while !torun.is_null() {
                    if (*torun).nexttorun == ptype {
                        (*torun).nexttorun = (*(*torun).nexttorun).nexttorun;
                    }
                    torun = (*torun).nexttorun;
                }
            }
        }

        // some things need to be preserved before we clear everything.
        beamsegs = (*ptype).beams;
        let mut tnamebuf = [0 as c_char; MAX_QPATH];
        let mut tconfbuf = [0 as c_char; MAX_QPATH];
        g::strcpy(
            tnamebuf.as_mut_ptr(),
            ptr::addr_of!((*ptype).name).cast::<c_char>(),
        );
        g::strcpy(
            tconfbuf.as_mut_ptr(),
            ptr::addr_of!((*ptype).config).cast::<c_char>(),
        );

        // free uneeded info
        if !(*ptype).ramp.is_null() {
            c::Mem_Free((*ptype).ramp.cast::<c_void>());
        }
        if !(*ptype).sounds.is_null() {
            c::Mem_Free((*ptype).sounds.cast::<c_void>());
        }

        // reset everything we're too lazy to specifically set
        g::memset(
            ptype.cast::<c_void>(),
            0,
            core::mem::size_of::<part_type_t>(),
        );

        // now set any non-0 defaults.

        (*ptype).beams = beamsegs;
        (*ptype).rainfrequency = 1.0;
        g::strcpy(
            ptr::addr_of_mut!((*ptype).name).cast::<c_char>(),
            tnamebuf.as_ptr(),
        );
        g::strcpy(
            ptr::addr_of_mut!((*ptype).config).cast::<c_char>(),
            tconfbuf.as_ptr(),
        );
        (*ptype).assoc = P_INVALID;
        (*ptype).inwater = P_INVALID;
        (*ptype).cliptype = P_INVALID;
        (*ptype).emit = P_INVALID;
        (*ptype).fluidmask = FTECONTENTS_FLUID;
        (*ptype).alpha = 1.0;
        (*ptype).alphachange = 1.0;
        (*ptype).clipbounce = 0.8;
        (*ptype).clipcount = 1.0;
        (*ptype).colorindex = -1;
        // start with a random angle. `M_PI` is a `double` in C, so the
        // negation and the following subtraction both happen in `double` and
        // are narrowed on assignment.
        (*ptype).rotationstartmin = (-core::f64::consts::PI) as c_float;
        (*ptype).rotationstartrand =
            (core::f64::consts::PI - (*ptype).rotationstartmin as f64) as c_float;
        (*ptype).spawnchance = 1.0;
        (*ptype).dl_time = 0.0;
        (*ptype).dl_rgb = [1.0, 1.0, 1.0];
        (*ptype).dl_corona_intensity = 0.25;
        (*ptype).dl_corona_scale = 0.5;
        (*ptype).dl_scales = [0.0, 1.0, 1.0];
        (*ptype).looks.stretch = 0.05;

        (*ptype).randsmax = 1;
        (*ptype).s2 = 1.0;
        (*ptype).t2 = 1.0;
    }
}

/// `r_part_fte.c:1411` -- `char *PScript_ReadLine (char *buffer, size_t
/// buffersize, const char *filedata, size_t filesize, size_t *offset)`.
///
/// # Safety
/// `buffer` must address `buffersize` writable bytes, `filedata` `filesize`
/// readable bytes, and `*offset` must be within `filesize`.
unsafe fn pscript_read_line(
    buffer: *mut c_char,
    buffersize: usize,
    filedata: *const c_char,
    filesize: usize,
    offset: *mut usize,
) -> *mut c_char {
    // SAFETY: pointer contracts per the fn docs.
    unsafe {
        let start = filedata.add(*offset);
        let mut f = start;
        let e = filedata.add(filesize);
        if f >= e {
            return ptr::null_mut(); // eof
        }
        while f < e {
            let ch = *f;
            f = f.add(1);
            if ch == b'\n' as c_char {
                break;
            }
        }

        *offset = f.offset_from(filedata) as usize;

        let mut n = buffersize - 1;
        if n >= f.offset_from(start) as usize {
            n = f.offset_from(start) as usize;
        }
        g::memcpy(buffer.cast::<c_void>(), start.cast::<c_void>(), n);
        *buffer.add(n) = 0; // null terminate it

        buffer
    }
}

// ---------------------------------------------------------------------------
// r_part_fte.c:1435-2591 -- the effect description parser.
//
// The C body is built out of four labels (`nexteffect`, `reparse`,
// `skipread`, `parsefluid`). They are reproduced as labelled loops rather
// than restructured, so the control flow stays line-for-line comparable with
// the oracle:
//
//   `goto nexteffect` -> `continue 'nexteffect` with `reparse == false`
//   `goto reparse`    -> `continue 'nexteffect` with `reparse == true`
//   `goto skipread`   -> `continue 'skipread`
//   `goto parsefluid` -> `parsefluid = true` then fall into the shared block

/// `r_part_fte.c:1425` -- `atof` on a token, kept `double` all the way to the
/// assignment so the C promotion rules survive.
///
/// # Safety
/// `s` must be NUL-terminated.
#[inline]
unsafe fn atofv(s: *const c_char) -> f64 {
    // SAFETY: per the fn docs.
    unsafe { g::atof(s) }
}

/// `r_part_fte.c` -- `atoi` on a token.
///
/// # Safety
/// `s` must be NUL-terminated.
#[inline]
unsafe fn atoiv(s: *const c_char) -> c_int {
    // SAFETY: per the fn docs.
    unsafe { g::atoi(s) }
}

/// `r_part_fte.c:1435` -- `void PScript_ParseParticleEffectFile (const char
/// *config, qboolean part_parseweak, char *context, size_t filesize)`. This
/// is the function that loads the effect descriptions.
///
/// # Safety
/// `config` must be NUL-terminated and `context` must address `filesize`
/// readable bytes.
#[allow(clippy::too_many_lines)]
unsafe fn pscript_parse_particle_effect_file(
    config_in: *const c_char,
    mut part_parseweak: bool,
    context: *mut c_char,
    filesize: usize,
) {
    // SAFETY: pointer contracts per the fn docs; every list touched below is
    // either file-private or the glue-owned `part_type` array, and the parser
    // only ever runs on the main thread.
    unsafe {
        let mut var: *const c_char;
        let mut value: *const c_char;
        let mut buf: *mut c_char;
        let mut settype: bool;
        let mut setalphadelta: bool;
        let mut setbeamlen: bool;

        let mut ptype: *mut part_type_t;
        let mut pnum: c_int;
        let mut assoc: c_int;
        let mut line = [0 as c_char; 512];
        let mut part_parsenamespace = [0 as c_char; MAX_QPATH];

        let palrgba = ptr::addr_of_mut!(g::d_8to24table).cast::<u8>();
        let mut offset: usize = 0;

        g::q_strlcpy(part_parsenamespace.as_mut_ptr(), config_in, MAX_QPATH);
        let config: *const c_char = part_parsenamespace.as_ptr();

        let mut reparse = false;

        'nexteffect: loop {
            if !reparse
                && pscript_read_line(
                    line.as_mut_ptr(),
                    core::mem::size_of_val(&line),
                    context,
                    filesize,
                    &mut offset,
                )
                .is_null()
            {
                return; // eof
            }
            reparse = false;

            g::Cmd_TokenizeString(line.as_ptr());

            var = c::Cmd_Argv(0);

            if g::strcmp(var, c"r_effect".as_ptr()) == 0 || g::strcmp(var, c"r_trail".as_ptr()) == 0
            {
                // add an emit/trail effect to all ents using said model
                pscript_associate_effect_f();
                continue 'nexteffect;
            } else if g::strcmp(var, c"r_partredirect".as_ptr()) == 0 {
                // add an emit/trail effect to all ents using said model
                p_part_redirect_f();
                continue 'nexteffect;
            } else if g::strcmp(var, c"r_part".as_ptr()) != 0 {
                if *var != 0 {
                    c::Con_SafePrintf(c"Unknown particle command \"%s\"\n".as_ptr(), var);
                }
                continue 'nexteffect;
            }

            settype = false;
            setalphadelta = false;
            setbeamlen = false;

            if c::Cmd_Argc() != 2 {
                if g::strcmp(c::Cmd_Argv(1), c"namespace".as_ptr()) == 0 {
                    g::q_strlcpy(part_parsenamespace.as_mut_ptr(), c::Cmd_Argv(2), MAX_QPATH);
                    if c::Cmd_Argc() >= 4 {
                        part_parseweak = atoiv(c::Cmd_Argv(3)) != 0;
                    }
                    continue 'nexteffect;
                }
                c::Con_SafePrintf(c"No name for particle effect\n".as_ptr());
                continue 'nexteffect;
            }

            buf = pscript_read_line(
                line.as_mut_ptr(),
                core::mem::size_of_val(&line),
                context,
                filesize,
                &mut offset,
            );
            if buf.is_null() {
                return; // eof
            }
            while *buf != 0 && *buf <= b' ' as c_char {
                buf = buf.add(1); // no whitespace please.
            }
            if *buf != b'{' as c_char {
                c::Con_SafePrintf(
                    c"This is a multiline command and should be used within config files\n"
                        .as_ptr(),
                );
                reparse = true;
                continue 'nexteffect;
            }

            var = c::Cmd_Argv(1);
            if *var == b'+' as c_char {
                ptype = p_get_particle_type(config, var.add(1));
            } else {
                ptype = p_get_particle_type(config, var);
            }

            // 'weak' configs do not replace 'strong' configs
            // we allow weak to replace weak as a solution to the +assoc chain
            // thing (to add, we effectively need to 'replace').
            if part_parseweak && (*ptype).loaded == 2 {
                let mut depth = 1;
                loop {
                    buf = pscript_read_line(
                        line.as_mut_ptr(),
                        core::mem::size_of_val(&line),
                        context,
                        filesize,
                        &mut offset,
                    );
                    if buf.is_null() {
                        return;
                    }

                    while *buf != 0 && *buf <= b' ' as c_char {
                        buf = buf.add(1); // no whitespace please.
                    }
                    if *buf == b'{' as c_char {
                        depth += 1;
                    } else if *buf == b'}' as c_char {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
                continue 'nexteffect;
            }

            if *var == b'+' as c_char {
                if (*ptype).loaded != 0 {
                    let mut newname = [0 as c_char; 256];
                    let mut i = 0;
                    while i < 64 {
                        let parenttype = ptype.offset_from(g::part_type) as c_int;
                        g::q_snprintf(
                            newname.as_mut_ptr(),
                            core::mem::size_of_val(&newname),
                            c"+%i%s".as_ptr(),
                            i,
                            ptr::addr_of!((*ptype).name).cast::<c_char>(),
                        );
                        ptype = p_get_particle_type(config, newname.as_ptr());
                        if (*ptype).loaded == 0 {
                            if (*g::part_type.add(parenttype as usize)).assoc != P_INVALID {
                                c::Con_SafePrintf(
                                    c"warning: assoc on particle chain %s overridden\n".as_ptr(),
                                    var.add(1),
                                );
                            }
                            (*g::part_type.add(parenttype as usize)).assoc =
                                ptype.offset_from(g::part_type) as c_int;
                            break;
                        }
                        i += 1;
                    }
                    if i == 64 {
                        c::Con_SafePrintf(c"Too many duplicate names, gave up\n".as_ptr());
                        return;
                    }
                }
            } else if (*ptype).loaded != 0 {
                assoc = (*ptype).assoc;
                while assoc != P_INVALID && assoc < g::numparticletypes {
                    let pt = g::part_type.add(assoc as usize);
                    if (*pt).name[0] == b'+' as c_char {
                        (*pt).loaded = 0;
                        assoc = (*pt).assoc;
                    } else {
                        break;
                    }
                }
            }
            if ptype.is_null() {
                c::Con_SafePrintf(c"Bad name\n".as_ptr());
                return;
            }

            pnum = ptype.offset_from(g::part_type) as c_int;

            p_reset_to_defaults(ptype);

            'props: loop {
                buf = pscript_read_line(
                    line.as_mut_ptr(),
                    core::mem::size_of_val(&line),
                    context,
                    filesize,
                    &mut offset,
                );
                if buf.is_null() {
                    c::Con_SafePrintf(
                        c"Unexpected end of buffer with effect %s\n".as_ptr(),
                        ptr::addr_of!((*ptype).name).cast::<c_char>(),
                    );
                    return;
                }
                'skipread: loop {
                    while *buf != 0 && *buf <= b' ' as c_char {
                        buf = buf.add(1); // no whitespace please.
                    }
                    if *buf == b'}' as c_char {
                        break 'props;
                    }

                    g::Cmd_TokenizeString(buf);
                    var = c::Cmd_Argv(0);
                    value = c::Cmd_Argv(1);

                    // TODO: switch this mess to some sort of binary tree to
                    // increase parse speed
                    if g::strcmp(var, c"shader".as_ptr()) == 0 {
                        g::q_strlcpy(
                            ptr::addr_of_mut!((*ptype).texname).cast::<c_char>(),
                            ptr::addr_of!((*ptype).name).cast::<c_char>(),
                            MAX_QPATH,
                        );

                        buf = pscript_read_line(
                            line.as_mut_ptr(),
                            core::mem::size_of_val(&line),
                            context,
                            filesize,
                            &mut offset,
                        );
                        if buf.is_null() {
                            continue 'props;
                        }
                        while *buf != 0 && *buf <= b' ' as c_char {
                            buf = buf.add(1); // no leading whitespace please.
                        }
                        if *buf == b'{' as c_char {
                            let mut nest = 1;
                            let mut str_ = c::Mem_Alloc(3).cast::<c_char>();
                            let mut slen: usize = 2;
                            *str_ = b'{' as c_char;
                            *str_.add(1) = b'\n' as c_char;
                            *str_.add(2) = 0;
                            while nest != 0 {
                                buf = pscript_read_line(
                                    line.as_mut_ptr(),
                                    core::mem::size_of_val(&line),
                                    context,
                                    filesize,
                                    &mut offset,
                                );
                                if buf.is_null() {
                                    c::Con_SafePrintf(
                                        c"Unexpected end of buffer with effect %s\n".as_ptr(),
                                        ptr::addr_of!((*ptype).name).cast::<c_char>(),
                                    );
                                    break;
                                }
                                while *buf != 0 && *buf <= b' ' as c_char {
                                    buf = buf.add(1); // no leading whitespace please.
                                }
                                if *buf == b'}' as c_char {
                                    nest -= 1;
                                }
                                if *buf == b'{' as c_char {
                                    nest += 1;
                                }
                                str_ = c::Mem_Realloc(
                                    str_.cast::<c_void>(),
                                    slen + g::strlen(buf) + 2,
                                )
                                .cast::<c_char>();
                                g::strcpy(str_.add(slen), buf);
                                slen += g::strlen(str_.add(slen));
                                *str_.add(slen) = b'\n' as c_char;
                                slen += 1;
                            }
                            *str_.add(slen) = 0;
                            c::Mem_Free(str_.cast::<c_void>());
                        } else {
                            continue 'skipread;
                        }
                    } else if g::strcmp(var, c"texture".as_ptr()) == 0
                        || g::strcmp(var, c"linear_texture".as_ptr()) == 0
                        || g::strcmp(var, c"nearest_texture".as_ptr()) == 0
                        || g::strcmp(var, c"nearesttexture".as_ptr()) == 0
                    {
                        g::q_strlcpy(
                            ptr::addr_of_mut!((*ptype).texname).cast::<c_char>(),
                            value,
                            MAX_QPATH,
                        );
                        (*ptype).looks.nearest = g::strncmp(var, c"nearest".as_ptr(), 7) == 0;
                    } else if g::strcmp(var, c"tcoords".as_ptr()) == 0 {
                        let mut tscale = atofv(c::Cmd_Argv(5)) as c_float;
                        if tscale <= 0.0 {
                            tscale = 1.0;
                        }

                        (*ptype).s1 = (atofv(value) / tscale as f64) as c_float;
                        (*ptype).t1 = (atofv(c::Cmd_Argv(2)) / tscale as f64) as c_float;
                        (*ptype).s2 = (atofv(c::Cmd_Argv(3)) / tscale as f64) as c_float;
                        (*ptype).t2 = (atofv(c::Cmd_Argv(4)) / tscale as f64) as c_float;

                        (*ptype).randsmax = atoiv(c::Cmd_Argv(6));
                        if c::Cmd_Argc() > 7 {
                            // FIXME: divide-by-tscale missing
                            (*ptype).texsstride = atofv(c::Cmd_Argv(7)) as c_float;
                        } else {
                            (*ptype).texsstride = (1.0 / tscale as f64) as c_float;
                        }

                        if (*ptype).randsmax < 1 || (*ptype).texsstride == 0.0 {
                            (*ptype).randsmax = 1;
                        }
                    } else if g::strcmp(var, c"atlas".as_ptr()) == 0 {
                        // atlas countineachaxis first [last]
                        // COMPAT: ADR-010 -- `dims` is `int dims = atof (...)`
                        // in C: a `double` narrowed by the implicit
                        // conversion, not `atoi`.
                        let mut dims = atofv(c::Cmd_Argv(1)) as c_int;
                        let i = atoiv(c::Cmd_Argv(2));
                        let mut m = atoiv(c::Cmd_Argv(3));
                        if dims < 1 {
                            dims = 1;
                        }

                        if m > (m / dims) * dims + dims - 1 {
                            m = (m / dims) * dims + dims - 1;
                            c::Con_SafePrintf(
                                c"effect %s wraps across an atlased line\n".as_ptr(),
                                ptr::addr_of!((*ptype).name).cast::<c_char>(),
                            );
                        }
                        if m < i {
                            m = i;
                        }

                        (*ptype).s1 = (1.0 / dims as f64 * (i % dims) as f64) as c_float;
                        (*ptype).s2 = (1.0 / dims as f64 * (1 + (i % dims)) as f64) as c_float;
                        (*ptype).t1 = (1.0 / dims as f64 * (i / dims) as f64) as c_float;
                        (*ptype).t2 = (1.0 / dims as f64 * (1 + (i / dims)) as f64) as c_float;

                        (*ptype).randsmax = m - i;
                        (*ptype).texsstride = (*ptype).s2 - (*ptype).s1;

                        // its modulo
                        (*ptype).randsmax += 1;
                    } else if g::strcmp(var, c"rotation".as_ptr()) == 0 {
                        (*ptype).rotationstartmin =
                            (atofv(value) * core::f64::consts::PI / 180.0) as c_float;
                        if c::Cmd_Argc() > 2 {
                            (*ptype).rotationstartrand =
                                (atofv(c::Cmd_Argv(2)) * core::f64::consts::PI / 180.0
                                    - (*ptype).rotationstartmin as f64)
                                    as c_float;
                        } else {
                            (*ptype).rotationstartrand = 0.0;
                        }

                        (*ptype).rotationmin =
                            (atofv(c::Cmd_Argv(3)) * core::f64::consts::PI / 180.0) as c_float;
                        if c::Cmd_Argc() > 4 {
                            (*ptype).rotationrand = (atofv(c::Cmd_Argv(4)) * core::f64::consts::PI
                                / 180.0
                                - (*ptype).rotationmin as f64)
                                as c_float;
                        } else {
                            (*ptype).rotationrand = 0.0;
                        }
                    } else if g::strcmp(var, c"rotationstart".as_ptr()) == 0 {
                        (*ptype).rotationstartmin =
                            (atofv(value) * core::f64::consts::PI / 180.0) as c_float;
                        if c::Cmd_Argc() > 2 {
                            (*ptype).rotationstartrand =
                                (atofv(c::Cmd_Argv(2)) * core::f64::consts::PI / 180.0
                                    - (*ptype).rotationstartmin as f64)
                                    as c_float;
                        } else {
                            (*ptype).rotationstartrand = 0.0;
                        }
                    } else if g::strcmp(var, c"rotationspeed".as_ptr()) == 0 {
                        (*ptype).rotationmin =
                            (atofv(value) * core::f64::consts::PI / 180.0) as c_float;
                        if c::Cmd_Argc() > 2 {
                            (*ptype).rotationrand = (atofv(c::Cmd_Argv(2)) * core::f64::consts::PI
                                / 180.0
                                - (*ptype).rotationmin as f64)
                                as c_float;
                        } else {
                            (*ptype).rotationrand = 0.0;
                        }
                    } else if g::strcmp(var, c"beamtexstep".as_ptr()) == 0 {
                        (*ptype).rotationstartmin = (1.0 / atofv(value)) as c_float;
                        (*ptype).rotationstartrand = 0.0;
                        setbeamlen = true;
                    } else if g::strcmp(var, c"beamtexspeed".as_ptr()) == 0 {
                        (*ptype).rotationmin = atofv(value) as c_float;
                    } else if g::strcmp(var, c"scale".as_ptr()) == 0 {
                        (*ptype).scale = atofv(value) as c_float;
                        if c::Cmd_Argc() > 2 {
                            (*ptype).scalerand =
                                (atofv(c::Cmd_Argv(2)) - (*ptype).scale as f64) as c_float;
                        }
                    } else if g::strcmp(var, c"scalerand".as_ptr()) == 0 {
                        (*ptype).scalerand = atofv(value) as c_float;
                    } else if g::strcmp(var, c"scalefactor".as_ptr()) == 0 {
                        (*ptype).looks.scalefactor = atofv(value) as c_float;
                    } else if g::strcmp(var, c"scaledelta".as_ptr()) == 0 {
                        (*ptype).scaledelta = atofv(value) as c_float;
                    } else if g::strcmp(var, c"stretchfactor".as_ptr()) == 0 {
                        // affects sparks
                        (*ptype).looks.stretch = atofv(value) as c_float;
                        (*ptype).looks.minstretch = if c::Cmd_Argc() > 2 {
                            atofv(c::Cmd_Argv(2)) as c_float
                        } else {
                            0.0
                        };
                    } else if g::strcmp(var, c"step".as_ptr()) == 0 {
                        (*ptype).countspacing = atofv(value) as c_float;
                        (*ptype).count = (1.0 / atofv(value)) as c_float;
                        if c::Cmd_Argc() > 2 {
                            (*ptype).countrand = (1.0 / atofv(c::Cmd_Argv(2))) as c_float;
                        }
                        if c::Cmd_Argc() > 3 {
                            (*ptype).countextra = atofv(c::Cmd_Argv(3)) as c_float;
                        }
                    } else if g::strcmp(var, c"count".as_ptr()) == 0 {
                        (*ptype).countspacing = 0.0;
                        (*ptype).count = atofv(value) as c_float;
                        if c::Cmd_Argc() > 2 {
                            (*ptype).countrand = atofv(c::Cmd_Argv(2)) as c_float;
                        }
                        if c::Cmd_Argc() > 3 {
                            (*ptype).countextra = atofv(c::Cmd_Argv(3)) as c_float;
                        }
                    } else if g::strcmp(var, c"rainfrequency".as_ptr()) == 0 {
                        // multiplier to ramp up the effect or whatever
                        // (without affecting spawn patterns).
                        (*ptype).rainfrequency = atofv(value) as c_float;
                    } else if g::strcmp(var, c"alpha".as_ptr()) == 0 {
                        (*ptype).alpha = atofv(value) as c_float;
                    } else if g::strcmp(var, c"alpharand".as_ptr()) == 0 {
                        (*ptype).alpharand = atofv(value) as c_float;
                    } else if g::strcmp(var, c"alphachange".as_ptr()) == 0 {
                        c::Con_DPrintf(
                            c"%s.%s: alphachange is deprecated, use alphadelta\n".as_ptr(),
                            ptr::addr_of!((*ptype).config).cast::<c_char>(),
                            ptr::addr_of!((*ptype).name).cast::<c_char>(),
                        );
                        (*ptype).alphachange = atofv(value) as c_float;
                    } else if g::strcmp(var, c"alphadelta".as_ptr()) == 0 {
                        (*ptype).alphachange = atofv(value) as c_float;
                        setalphadelta = true;
                    } else if g::strcmp(var, c"die".as_ptr()) == 0 {
                        (*ptype).die = atofv(value) as c_float;
                        if c::Cmd_Argc() > 2 {
                            let mut mn = (*ptype).die;
                            let mut mx = atofv(c::Cmd_Argv(2)) as c_float;
                            if mn > mx {
                                mn = mx;
                                mx = (*ptype).die;
                            }
                            (*ptype).die = mx;
                            (*ptype).randdie = mx - mn;
                        }
                    } else if g::strcmp(var, c"diesubrand".as_ptr()) == 0 {
                        c::Con_DPrintf(
                            c"%s.%s: diesubrand is deprecated, use die with two arguments\n"
                                .as_ptr(),
                            ptr::addr_of!((*ptype).config).cast::<c_char>(),
                            ptr::addr_of!((*ptype).name).cast::<c_char>(),
                        );
                        (*ptype).randdie = atofv(value) as c_float;
                    } else if g::strcmp(var, c"randomvel".as_ptr()) == 0 {
                        // shortcut for velwrand (and velbias for z bias)
                        (*ptype).velbias[1] = 0.0;
                        (*ptype).velbias[0] = 0.0;
                        (*ptype).velwrand[1] = atofv(value) as c_float;
                        (*ptype).velwrand[0] = (*ptype).velwrand[1];
                        if c::Cmd_Argc() > 3 {
                            (*ptype).velbias[2] = atofv(c::Cmd_Argv(2)) as c_float;
                            (*ptype).velwrand[2] = atofv(c::Cmd_Argv(3)) as c_float;
                            // make vert be the total range
                            (*ptype).velwrand[2] -= (*ptype).velbias[2];
                            // vert is actually +/- 1, not 0 to 1, so rescale it
                            (*ptype).velwrand[2] /= 2.0;
                            // and bias must be centered to the range
                            (*ptype).velbias[2] += (*ptype).velwrand[2];
                        } else if c::Cmd_Argc() > 2 {
                            (*ptype).velwrand[2] = atofv(c::Cmd_Argv(2)) as c_float;
                            (*ptype).velbias[2] = 0.0;
                        } else {
                            (*ptype).velwrand[2] = (*ptype).velwrand[0];
                            (*ptype).velbias[2] = 0.0;
                        }
                    } else if g::strcmp(var, c"veladd".as_ptr()) == 0 {
                        (*ptype).veladd = atofv(value) as c_float;
                        (*ptype).randomveladd = 0.0;
                        if c::Cmd_Argc() > 2 {
                            (*ptype).randomveladd =
                                (atofv(c::Cmd_Argv(2)) - (*ptype).veladd as f64) as c_float;
                        }
                    } else if g::strcmp(var, c"orgadd".as_ptr()) == 0 {
                        (*ptype).orgadd = atofv(value) as c_float;
                        (*ptype).randomorgadd = 0.0;
                        if c::Cmd_Argc() > 2 {
                            (*ptype).randomorgadd =
                                (atofv(c::Cmd_Argv(2)) - (*ptype).orgadd as f64) as c_float;
                        }
                    } else if g::strcmp(var, c"orgbias".as_ptr()) == 0 {
                        (*ptype).orgbias[0] = atofv(value) as c_float;
                        (*ptype).orgbias[1] = atofv(c::Cmd_Argv(2)) as c_float;
                        (*ptype).orgbias[2] = atofv(c::Cmd_Argv(3)) as c_float;
                    } else if g::strcmp(var, c"orgwrand".as_ptr()) == 0 {
                        (*ptype).orgwrand[0] = atofv(value) as c_float;
                        (*ptype).orgwrand[1] = atofv(c::Cmd_Argv(2)) as c_float;
                        (*ptype).orgwrand[2] = atofv(c::Cmd_Argv(3)) as c_float;
                    } else if g::strcmp(var, c"velbias".as_ptr()) == 0 {
                        (*ptype).velbias[0] = atofv(value) as c_float;
                        (*ptype).velbias[1] = atofv(c::Cmd_Argv(2)) as c_float;
                        (*ptype).velbias[2] = atofv(c::Cmd_Argv(3)) as c_float;
                    } else if g::strcmp(var, c"velwrand".as_ptr()) == 0 {
                        (*ptype).velwrand[0] = atofv(value) as c_float;
                        (*ptype).velwrand[1] = atofv(c::Cmd_Argv(2)) as c_float;
                        (*ptype).velwrand[2] = atofv(c::Cmd_Argv(3)) as c_float;
                    } else if g::strcmp(var, c"friction".as_ptr()) == 0 {
                        (*ptype).friction[0] = atofv(value) as c_float;
                        (*ptype).friction[1] = (*ptype).friction[0];
                        (*ptype).friction[2] = (*ptype).friction[1];

                        if c::Cmd_Argc() > 3 {
                            (*ptype).friction[2] = atofv(c::Cmd_Argv(3)) as c_float;
                            (*ptype).friction[1] = atofv(c::Cmd_Argv(2)) as c_float;
                        } else if c::Cmd_Argc() > 2 {
                            (*ptype).friction[2] = atofv(c::Cmd_Argv(2)) as c_float;
                        }
                    } else if g::strcmp(var, c"gravity".as_ptr()) == 0 {
                        (*ptype).gravity = atofv(value) as c_float;
                    } else if g::strcmp(var, c"flurry".as_ptr()) == 0 {
                        (*ptype).flurry = atofv(value) as c_float;
                    } else if g::strcmp(var, c"assoc".as_ptr()) == 0 {
                        // careful - this can realloc all the particle types
                        assoc = check_assosiation(config, value, pnum);
                        ptype = g::part_type.add(pnum as usize);
                        (*ptype).assoc = assoc;
                    } else if g::strcmp(var, c"inwater".as_ptr()) == 0 {
                        // the underwater effect switch should only occur for
                        // 1 level so the standard assoc check works
                        assoc = check_assosiation(config, value, pnum);
                        ptype = g::part_type.add(pnum as usize);
                        (*ptype).inwater = assoc;
                    } else if g::strcmp(var, c"underwater".as_ptr()) == 0
                        || g::strcmp(var, c"notunderwater".as_ptr()) == 0
                    {
                        // `notunderwater` is `goto parsefluid` in C; the two
                        // branches are merged because the label has no other
                        // entry.
                        if g::strcmp(var, c"underwater".as_ptr()) == 0 {
                            (*ptype).flags |= PT_TRUNDERWATER;
                        } else {
                            (*ptype).flags |= PT_TROVERWATER;
                        }

                        // parsefluid:
                        if (*ptype).flags & (PT_TRUNDERWATER | PT_TROVERWATER)
                            == (PT_TRUNDERWATER | PT_TROVERWATER)
                        {
                            (*ptype).flags &= !PT_TRUNDERWATER;
                            c::Con_SafePrintf(
                                c"%s.%s: both over and under water\n".as_ptr(),
                                ptr::addr_of!((*ptype).config).cast::<c_char>(),
                                ptr::addr_of!((*ptype).name).cast::<c_char>(),
                            );
                        }
                        if c::Cmd_Argc() == 1 {
                            (*ptype).fluidmask = FTECONTENTS_FLUID;
                        } else {
                            let mut i = c::Cmd_Argc();
                            (*ptype).fluidmask = 0;
                            loop {
                                i -= 1;
                                if i < 1 {
                                    break;
                                }
                                let value_i = c::Cmd_Argv(i);
                                if g::strcmp(value_i, c"water".as_ptr()) == 0 {
                                    (*ptype).fluidmask |= FTECONTENTS_WATER;
                                } else if g::strcmp(value_i, c"slime".as_ptr()) == 0 {
                                    (*ptype).fluidmask |= FTECONTENTS_SLIME;
                                } else if g::strcmp(value_i, c"lava".as_ptr()) == 0 {
                                    (*ptype).fluidmask |= FTECONTENTS_LAVA;
                                } else if g::strcmp(value_i, c"sky".as_ptr()) == 0 {
                                    (*ptype).fluidmask |= FTECONTENTS_SKY;
                                } else if g::strcmp(value_i, c"fluid".as_ptr()) == 0 {
                                    (*ptype).fluidmask |= FTECONTENTS_FLUID;
                                } else if g::strcmp(value_i, c"solid".as_ptr()) == 0 {
                                    (*ptype).fluidmask |= FTECONTENTS_SOLID;
                                } else if g::strcmp(value_i, c"playerclip".as_ptr()) == 0 {
                                    (*ptype).fluidmask |= FTECONTENTS_PLAYERCLIP;
                                } else if g::strcmp(value_i, c"none".as_ptr()) == 0 {
                                    (*ptype).fluidmask |= 0;
                                } else {
                                    c::Con_SafePrintf(
                                        c"%s.%s: unknown contents: %s\n".as_ptr(),
                                        ptr::addr_of!((*ptype).config).cast::<c_char>(),
                                        ptr::addr_of!((*ptype).name).cast::<c_char>(),
                                        value_i,
                                    );
                                }
                            }
                        }
                    } else if g::strcmp(var, c"model".as_ptr()) == 0 {
                        c::Con_DPrintf(
                            c"%s.%s: model particles are not supported in this build\n".as_ptr(),
                            ptr::addr_of!((*ptype).config).cast::<c_char>(),
                            ptr::addr_of!((*ptype).name).cast::<c_char>(),
                        );
                    } else if g::strcmp(var, c"sound".as_ptr()) == 0 {
                        (*ptype).sounds = c::Mem_Realloc(
                            (*ptype).sounds.cast::<c_void>(),
                            core::mem::size_of::<partsounds_t>()
                                * ((*ptype).numsounds as usize + 1),
                        )
                        .cast::<partsounds_t>();
                        let snd = (*ptype).sounds.add((*ptype).numsounds as usize);
                        g::q_strlcpy(
                            ptr::addr_of_mut!((*snd).name).cast::<c_char>(),
                            c::Cmd_Argv(1),
                            core::mem::size_of_val(&(*snd).name),
                        );
                        if (*snd).name[0] != 0 {
                            g::S_PrecacheSound(ptr::addr_of!((*snd).name).cast::<c_char>());
                        }

                        (*snd).vol = 1.0;
                        (*snd).atten = 1.0;
                        (*snd).pitch = 100.0;
                        (*snd).delay = 0.0;
                        (*snd).weight = 0.0;

                        let mut e: *mut c_char = ptr::null_mut();
                        g::strtoul(c::Cmd_Argv(2), &mut e, 0);
                        while *e == b' ' as c_char || *e == b'\t' as c_char {
                            e = e.add(1);
                        }
                        if *e != 0 {
                            for p in 2..c::Cmd_Argc() {
                                let mut ea = c::Cmd_Argv(p);

                                if g::q_strncasecmp(ea, c"vol=".as_ptr(), 4) == 0
                                    || g::q_strncasecmp(ea, c"volume=".as_ptr(), 7) == 0
                                {
                                    (*snd).vol =
                                        atofv(g::strchr(ea, b'=' as c_int).add(1)) as c_float;
                                } else if g::q_strncasecmp(ea, c"attn=".as_ptr(), 5) == 0
                                    || g::q_strncasecmp(ea, c"atten=".as_ptr(), 6) == 0
                                    || g::q_strncasecmp(ea, c"attenuation=".as_ptr(), 12) == 0
                                {
                                    ea = g::strchr(ea, b'=' as c_int).add(1);
                                    if g::strcmp(ea, c"none".as_ptr()) == 0 {
                                        (*snd).atten = 0.0;
                                    } else if g::strcmp(ea, c"normal".as_ptr()) == 0 {
                                        (*snd).atten = 1.0;
                                    } else {
                                        (*snd).atten = atofv(ea) as c_float;
                                    }
                                } else if g::q_strncasecmp(ea, c"pitch=".as_ptr(), 6) == 0 {
                                    (*snd).pitch =
                                        atofv(g::strchr(ea, b'=' as c_int).add(1)) as c_float;
                                } else if g::q_strncasecmp(ea, c"delay=".as_ptr(), 6) == 0 {
                                    (*snd).delay =
                                        atofv(g::strchr(ea, b'=' as c_int).add(1)) as c_float;
                                } else if g::q_strncasecmp(ea, c"weight=".as_ptr(), 7) == 0 {
                                    (*snd).weight =
                                        atofv(g::strchr(ea, b'=' as c_int).add(1)) as c_float;
                                } else {
                                    c::Con_SafePrintf(c"Bad named argument: %s\n".as_ptr(), ea);
                                }
                            }
                        } else {
                            (*snd).vol = atofv(c::Cmd_Argv(2)) as c_float;
                            if (*snd).vol == 0.0 {
                                (*snd).vol = 1.0;
                            }
                            (*snd).atten = atofv(c::Cmd_Argv(3)) as c_float;
                            if (*snd).atten == 0.0 {
                                (*snd).atten = 1.0;
                            }
                            (*snd).pitch = atofv(c::Cmd_Argv(4)) as c_float;
                            if (*snd).pitch == 0.0 {
                                (*snd).pitch = 100.0;
                            }
                            (*snd).delay = atofv(c::Cmd_Argv(5)) as c_float;
                            if (*snd).delay == 0.0 {
                                (*snd).delay = 0.0;
                            }
                            (*snd).weight = atofv(c::Cmd_Argv(6)) as c_float;
                        }
                        if (*snd).weight == 0.0 {
                            (*snd).weight = 1.0;
                        }
                        (*ptype).numsounds += 1;
                    } else if g::strcmp(var, c"colorindex".as_ptr()) == 0 {
                        if c::Cmd_Argc() > 2 {
                            (*ptype).colorrand =
                                g::strtoul(c::Cmd_Argv(2), ptr::null_mut(), 0) as c_int;
                        }
                        (*ptype).colorindex = g::strtoul(value, ptr::null_mut(), 0) as c_int;
                    } else if g::strcmp(var, c"colorrand".as_ptr()) == 0 {
                        (*ptype).colorrand = atoiv(value); // now obsolete
                    } else if g::strcmp(var, c"citracer".as_ptr()) == 0 {
                        (*ptype).flags |= PT_CITRACER;
                    } else if g::strcmp(var, c"red".as_ptr()) == 0 {
                        (*ptype).rgb[0] = (atofv(value) / 255.0) as c_float;
                    } else if g::strcmp(var, c"green".as_ptr()) == 0 {
                        (*ptype).rgb[1] = (atofv(value) / 255.0) as c_float;
                    } else if g::strcmp(var, c"blue".as_ptr()) == 0 {
                        (*ptype).rgb[2] = (atofv(value) / 255.0) as c_float;
                    } else if g::strcmp(var, c"rgb".as_ptr()) == 0 {
                        // byte version
                        (*ptype).rgb[2] = (atofv(value) / 255.0) as c_float;
                        (*ptype).rgb[1] = (*ptype).rgb[2];
                        (*ptype).rgb[0] = (*ptype).rgb[1];
                        if c::Cmd_Argc() > 3 {
                            (*ptype).rgb[1] = (atofv(c::Cmd_Argv(2)) / 255.0) as c_float;
                            (*ptype).rgb[2] = (atofv(c::Cmd_Argv(3)) / 255.0) as c_float;
                        }
                    } else if g::strcmp(var, c"rgbf".as_ptr()) == 0 {
                        // float version
                        (*ptype).rgb[2] = atofv(value) as c_float;
                        (*ptype).rgb[1] = (*ptype).rgb[2];
                        (*ptype).rgb[0] = (*ptype).rgb[1];
                        if c::Cmd_Argc() > 3 {
                            (*ptype).rgb[1] = atofv(c::Cmd_Argv(2)) as c_float;
                            (*ptype).rgb[2] = atofv(c::Cmd_Argv(3)) as c_float;
                        }
                    } else if g::strcmp(var, c"reddelta".as_ptr()) == 0 {
                        (*ptype).rgbchange[0] = (atofv(value) / 255.0) as c_float;
                        if (*ptype).rgbchangetime == 0.0 {
                            (*ptype).rgbchangetime = (*ptype).die;
                        }
                    } else if g::strcmp(var, c"greendelta".as_ptr()) == 0 {
                        (*ptype).rgbchange[1] = (atofv(value) / 255.0) as c_float;
                        if (*ptype).rgbchangetime == 0.0 {
                            (*ptype).rgbchangetime = (*ptype).die;
                        }
                    } else if g::strcmp(var, c"bluedelta".as_ptr()) == 0 {
                        (*ptype).rgbchange[2] = (atofv(value) / 255.0) as c_float;
                        if (*ptype).rgbchangetime == 0.0 {
                            (*ptype).rgbchangetime = (*ptype).die;
                        }
                    } else if g::strcmp(var, c"rgbdelta".as_ptr()) == 0 {
                        // byte version
                        (*ptype).rgbchange[2] = (atofv(value) / 255.0) as c_float;
                        (*ptype).rgbchange[1] = (*ptype).rgbchange[2];
                        (*ptype).rgbchange[0] = (*ptype).rgbchange[1];
                        if c::Cmd_Argc() > 3 {
                            (*ptype).rgbchange[1] = (atofv(c::Cmd_Argv(2)) / 255.0) as c_float;
                            (*ptype).rgbchange[2] = (atofv(c::Cmd_Argv(3)) / 255.0) as c_float;
                        }
                        if (*ptype).rgbchangetime == 0.0 {
                            (*ptype).rgbchangetime = (*ptype).die;
                        }
                    } else if g::strcmp(var, c"rgbdeltaf".as_ptr()) == 0 {
                        // float version
                        (*ptype).rgbchange[2] = atofv(value) as c_float;
                        (*ptype).rgbchange[1] = (*ptype).rgbchange[2];
                        (*ptype).rgbchange[0] = (*ptype).rgbchange[1];
                        if c::Cmd_Argc() > 3 {
                            (*ptype).rgbchange[1] = atofv(c::Cmd_Argv(2)) as c_float;
                            (*ptype).rgbchange[2] = atofv(c::Cmd_Argv(3)) as c_float;
                        }
                        if (*ptype).rgbchangetime == 0.0 {
                            (*ptype).rgbchangetime = (*ptype).die;
                        }
                    } else if g::strcmp(var, c"rgbdeltatime".as_ptr()) == 0 {
                        (*ptype).rgbchangetime = atofv(value) as c_float;
                    } else if g::strcmp(var, c"redrand".as_ptr()) == 0 {
                        (*ptype).rgbrand[0] = (atofv(value) / 255.0) as c_float;
                    } else if g::strcmp(var, c"greenrand".as_ptr()) == 0 {
                        (*ptype).rgbrand[1] = (atofv(value) / 255.0) as c_float;
                    } else if g::strcmp(var, c"bluerand".as_ptr()) == 0 {
                        (*ptype).rgbrand[2] = (atofv(value) / 255.0) as c_float;
                    } else if g::strcmp(var, c"rgbrand".as_ptr()) == 0 {
                        // byte version
                        (*ptype).rgbrand[2] = (atofv(value) / 255.0) as c_float;
                        (*ptype).rgbrand[1] = (*ptype).rgbrand[2];
                        (*ptype).rgbrand[0] = (*ptype).rgbrand[1];
                        if c::Cmd_Argc() > 3 {
                            (*ptype).rgbrand[1] = (atofv(c::Cmd_Argv(2)) / 255.0) as c_float;
                            (*ptype).rgbrand[2] = (atofv(c::Cmd_Argv(3)) / 255.0) as c_float;
                        }
                    } else if g::strcmp(var, c"rgbrandf".as_ptr()) == 0 {
                        // float version
                        (*ptype).rgbrand[2] = atofv(value) as c_float;
                        (*ptype).rgbrand[1] = (*ptype).rgbrand[2];
                        (*ptype).rgbrand[0] = (*ptype).rgbrand[1];
                        if c::Cmd_Argc() > 3 {
                            (*ptype).rgbrand[1] = atofv(c::Cmd_Argv(2)) as c_float;
                            (*ptype).rgbrand[2] = atofv(c::Cmd_Argv(3)) as c_float;
                        }
                    } else if g::strcmp(var, c"rgbrandsync".as_ptr()) == 0 {
                        (*ptype).rgbrandsync[2] = atofv(value) as c_float;
                        (*ptype).rgbrandsync[1] = (*ptype).rgbrandsync[2];
                        (*ptype).rgbrandsync[0] = (*ptype).rgbrandsync[1];
                        if c::Cmd_Argc() > 3 {
                            (*ptype).rgbrandsync[1] = atofv(c::Cmd_Argv(2)) as c_float;
                            (*ptype).rgbrandsync[2] = atofv(c::Cmd_Argv(3)) as c_float;
                        }
                    } else if g::strcmp(var, c"redrandsync".as_ptr()) == 0 {
                        (*ptype).rgbrandsync[0] = atofv(value) as c_float;
                    } else if g::strcmp(var, c"greenrandsync".as_ptr()) == 0 {
                        (*ptype).rgbrandsync[1] = atofv(value) as c_float;
                    } else if g::strcmp(var, c"bluerandsync".as_ptr()) == 0 {
                        (*ptype).rgbrandsync[2] = atofv(value) as c_float;
                    } else if g::strcmp(var, c"stains".as_ptr()) == 0 {
                        (*ptype).stainonimpact = atofv(value) as c_float;
                    } else if g::strcmp(var, c"blend".as_ptr()) == 0 {
                        // small note: use premultiplied alpha where possible.
                        // this reduces the required state switches.
                        (*ptype).looks.premul = 0;
                        if g::strcmp(value, c"adda".as_ptr()) == 0
                            || g::strcmp(value, c"add".as_ptr()) == 0
                        {
                            (*ptype).looks.blendmode = BM_ADDA;
                        } else if g::strcmp(value, c"addc".as_ptr()) == 0 {
                            (*ptype).looks.blendmode = BM_ADDC;
                        } else if g::strcmp(value, c"subtract".as_ptr()) == 0 {
                            (*ptype).looks.blendmode = BM_SUBTRACT;
                        } else if g::strcmp(value, c"invmoda".as_ptr()) == 0
                            || g::strcmp(value, c"invmod".as_ptr()) == 0
                        {
                            (*ptype).looks.blendmode = BM_INVMODA;
                        } else if g::strcmp(value, c"invmodc".as_ptr()) == 0 {
                            (*ptype).looks.blendmode = BM_INVMODC;
                        } else if g::strcmp(value, c"blendcolour".as_ptr()) == 0
                            || g::strcmp(value, c"blendcolor".as_ptr()) == 0
                        {
                            (*ptype).looks.blendmode = BM_BLENDCOLOUR;
                        } else if g::strcmp(value, c"blendalpha".as_ptr()) == 0
                            || g::strcmp(value, c"blend".as_ptr()) == 0
                        {
                            (*ptype).looks.blendmode = BM_BLEND;
                        } else if g::strcmp(value, c"premul_subtract".as_ptr()) == 0 {
                            (*ptype).looks.premul = 1;
                            (*ptype).looks.blendmode = BM_INVMODC;
                        } else if g::strcmp(value, c"premul_add".as_ptr()) == 0 {
                            (*ptype).looks.premul = 2;
                            (*ptype).looks.blendmode = BM_PREMUL;
                        } else if g::strcmp(value, c"premul_blend".as_ptr()) == 0 {
                            (*ptype).looks.premul = 1;
                            (*ptype).looks.blendmode = BM_PREMUL;
                        } else {
                            c::Con_DPrintf(
                                c"%s.%s: uses unknown blend type '%s', assuming legacy 'blendalpha'\n"
                                    .as_ptr(),
                                ptr::addr_of!((*ptype).config).cast::<c_char>(),
                                ptr::addr_of!((*ptype).name).cast::<c_char>(),
                                value,
                            );
                            (*ptype).looks.blendmode = BM_BLEND; // fallback
                        }
                    } else if g::strcmp(var, c"spawnmode".as_ptr()) == 0 {
                        if g::strcmp(value, c"circle".as_ptr()) == 0 {
                            (*ptype).spawnmode = SM_CIRCLE;
                        } else if g::strcmp(value, c"ball".as_ptr()) == 0 {
                            (*ptype).spawnmode = SM_BALL;
                        } else if g::strcmp(value, c"spiral".as_ptr()) == 0 {
                            (*ptype).spawnmode = SM_SPIRAL;
                        } else if g::strcmp(value, c"tracer".as_ptr()) == 0 {
                            (*ptype).spawnmode = SM_TRACER;
                        } else if g::strcmp(value, c"telebox".as_ptr()) == 0 {
                            (*ptype).spawnmode = SM_TELEBOX;
                        } else if g::strcmp(value, c"lavasplash".as_ptr()) == 0 {
                            (*ptype).spawnmode = SM_LAVASPLASH;
                        } else if g::strcmp(value, c"uniformcircle".as_ptr()) == 0 {
                            (*ptype).spawnmode = SM_UNICIRCLE;
                        } else if g::strcmp(value, c"syncfield".as_ptr()) == 0 {
                            (*ptype).spawnmode = SM_FIELD;
                            (*ptype).spawnparam1 = 16.0;
                            (*ptype).spawnparam2 = 0.0;
                        } else if g::strcmp(value, c"distball".as_ptr()) == 0 {
                            (*ptype).spawnmode = SM_DISTBALL;
                        } else if g::strcmp(value, c"box".as_ptr()) == 0 {
                            (*ptype).spawnmode = SM_BOX;
                        } else {
                            c::Con_DPrintf(
                                c"%s.%s: uses unknown spawn type '%s', assuming 'box'\n".as_ptr(),
                                ptr::addr_of!((*ptype).config).cast::<c_char>(),
                                ptr::addr_of!((*ptype).name).cast::<c_char>(),
                                value,
                            );
                            (*ptype).spawnmode = SM_BOX;
                        }

                        if c::Cmd_Argc() > 2 {
                            if c::Cmd_Argc() > 3 {
                                (*ptype).spawnparam2 = atofv(c::Cmd_Argv(3)) as c_float;
                            }
                            (*ptype).spawnparam1 = atofv(c::Cmd_Argv(2)) as c_float;
                        }
                    } else if g::strcmp(var, c"type".as_ptr()) == 0 {
                        if g::strcmp(value, c"beam".as_ptr()) == 0 {
                            (*ptype).looks.type_ = PT_BEAM;
                        } else if g::strcmp(value, c"spark".as_ptr()) == 0
                            || g::strcmp(value, c"linespark".as_ptr()) == 0
                        {
                            (*ptype).looks.type_ = PT_SPARK;
                        } else if g::strcmp(value, c"sparkfan".as_ptr()) == 0
                            || g::strcmp(value, c"trianglefan".as_ptr()) == 0
                        {
                            (*ptype).looks.type_ = PT_SPARKFAN;
                        } else if g::strcmp(value, c"texturedspark".as_ptr()) == 0 {
                            (*ptype).looks.type_ = PT_TEXTUREDSPARK;
                        } else if g::strcmp(value, c"decal".as_ptr()) == 0
                            || g::strcmp(value, c"cdecal".as_ptr()) == 0
                        {
                            (*ptype).looks.type_ = PT_CDECAL;
                        } else if g::strcmp(value, c"udecal".as_ptr()) == 0 {
                            (*ptype).looks.type_ = PT_UDECAL;
                        } else if g::strcmp(value, c"normal".as_ptr()) == 0 {
                            (*ptype).looks.type_ = PT_NORMAL;
                        } else {
                            c::Con_DPrintf(
                                c"%s.%s: uses unknown render type '%s', assuming 'normal'\n"
                                    .as_ptr(),
                                ptr::addr_of!((*ptype).config).cast::<c_char>(),
                                ptr::addr_of!((*ptype).name).cast::<c_char>(),
                                value,
                            );
                            (*ptype).looks.type_ = PT_NORMAL; // fallback
                        }
                        settype = true;
                    } else if g::strcmp(var, c"clippeddecal".as_ptr()) == 0 {
                        // mask, match
                        if c::Cmd_Argc() >= 2 {
                            // decal only appears where: (surfflags&mask)==match
                            (*ptype).surfflagmask =
                                g::strtoul(c::Cmd_Argv(1), ptr::null_mut(), 0) as c_int;
                            (*ptype).surfflagmatch = (*ptype).surfflagmask;
                            if c::Cmd_Argc() >= 3 {
                                (*ptype).surfflagmatch =
                                    g::strtoul(c::Cmd_Argv(2), ptr::null_mut(), 0) as c_int;
                            }
                        }
                        (*ptype).looks.type_ = PT_CDECAL;
                        settype = true;
                    } else if g::strcmp(var, c"isbeam".as_ptr()) == 0 {
                        c::Con_DPrintf(
                            c"%s.%s: isbeam is deprecated, use type beam\n".as_ptr(),
                            ptr::addr_of!((*ptype).config).cast::<c_char>(),
                            ptr::addr_of!((*ptype).name).cast::<c_char>(),
                        );
                        (*ptype).looks.type_ = PT_BEAM;
                        settype = true;
                    } else if g::strcmp(var, c"spawntime".as_ptr()) == 0 {
                        (*ptype).spawntime = atofv(value) as c_float;
                    } else if g::strcmp(var, c"spawnchance".as_ptr()) == 0 {
                        (*ptype).spawnchance = atofv(value) as c_float;
                    } else if g::strcmp(var, c"cliptype".as_ptr()) == 0 {
                        // careful - this can realloc all the particle types
                        assoc = p_allocate_particle_type(config, value);
                        ptype = g::part_type.add(pnum as usize);
                        (*ptype).cliptype = assoc;
                    } else if g::strcmp(var, c"clipcount".as_ptr()) == 0 {
                        (*ptype).clipcount = atofv(value) as c_float;
                    } else if g::strcmp(var, c"clipbounce".as_ptr()) == 0 {
                        (*ptype).clipbounce = atofv(value) as c_float;
                        if (*ptype).clipbounce < 0.0 && (*ptype).cliptype == P_INVALID {
                            (*ptype).cliptype = pnum;
                        }
                    } else if g::strcmp(var, c"bounce".as_ptr()) == 0 {
                        (*ptype).cliptype = pnum;
                        (*ptype).clipbounce = atofv(value) as c_float;
                    } else if g::strcmp(var, c"emit".as_ptr()) == 0 {
                        // careful - this can realloc all the particle types
                        assoc = p_allocate_particle_type(config, value);
                        ptype = g::part_type.add(pnum as usize);
                        (*ptype).emit = assoc;
                    } else if g::strcmp(var, c"emitinterval".as_ptr()) == 0 {
                        (*ptype).emittime = atofv(value) as c_float;
                    } else if g::strcmp(var, c"emitintervalrand".as_ptr()) == 0 {
                        (*ptype).emitrand = atofv(value) as c_float;
                    } else if g::strcmp(var, c"emitstart".as_ptr()) == 0 {
                        (*ptype).emitstart = atofv(value) as c_float;
                    }
                    // old names
                    else if g::strcmp(var, c"areaspread".as_ptr()) == 0 {
                        c::Con_DPrintf(
                            c"%s.%s: areaspread is deprecated, use spawnorg\n".as_ptr(),
                            ptr::addr_of!((*ptype).config).cast::<c_char>(),
                            ptr::addr_of!((*ptype).name).cast::<c_char>(),
                        );
                        (*ptype).areaspread = atofv(value) as c_float;
                    } else if g::strcmp(var, c"areaspreadvert".as_ptr()) == 0 {
                        c::Con_DPrintf(
                            c"%s.%s: areaspreadvert is deprecated, use spawnorg\n".as_ptr(),
                            ptr::addr_of!((*ptype).config).cast::<c_char>(),
                            ptr::addr_of!((*ptype).name).cast::<c_char>(),
                        );
                        (*ptype).areaspreadvert = atofv(value) as c_float;
                    } else if g::strcmp(var, c"offsetspread".as_ptr()) == 0 {
                        c::Con_DPrintf(
                            c"%s.%s: offsetspread is deprecated, use spawnvel\n".as_ptr(),
                            ptr::addr_of!((*ptype).config).cast::<c_char>(),
                            ptr::addr_of!((*ptype).name).cast::<c_char>(),
                        );
                        (*ptype).spawnvel = atofv(value) as c_float;
                    } else if g::strcmp(var, c"offsetspreadvert".as_ptr()) == 0 {
                        c::Con_DPrintf(
                            c"%s.%s: offsetspreadvert is deprecated, use spawnvel\n".as_ptr(),
                            ptr::addr_of!((*ptype).config).cast::<c_char>(),
                            ptr::addr_of!((*ptype).name).cast::<c_char>(),
                        );
                        (*ptype).spawnvelvert = atofv(value) as c_float;
                    }
                    // current names
                    else if g::strcmp(var, c"spawnorg".as_ptr()) == 0 {
                        (*ptype).areaspread = atofv(value) as c_float;
                        (*ptype).areaspreadvert = (*ptype).areaspread;

                        if c::Cmd_Argc() > 2 {
                            (*ptype).areaspreadvert = atofv(c::Cmd_Argv(2)) as c_float;
                        }
                    } else if g::strcmp(var, c"spawnvel".as_ptr()) == 0 {
                        (*ptype).spawnvel = atofv(value) as c_float;
                        (*ptype).spawnvelvert = (*ptype).spawnvel;

                        if c::Cmd_Argc() > 2 {
                            (*ptype).spawnvelvert = atofv(c::Cmd_Argv(2)) as c_float;
                        }
                    }
                    // spawn mode param fields
                    else if g::strcmp(var, c"spawnparam1".as_ptr()) == 0 {
                        (*ptype).spawnparam1 = atofv(value) as c_float;
                        c::Con_DPrintf(
                            c"%s.%s: 'spawnparam1' is deprecated, use 'spawnmode foo X'\n".as_ptr(),
                            ptr::addr_of!((*ptype).config).cast::<c_char>(),
                            ptr::addr_of!((*ptype).name).cast::<c_char>(),
                        );
                    } else if g::strcmp(var, c"spawnparam2".as_ptr()) == 0 {
                        (*ptype).spawnparam2 = atofv(value) as c_float;
                        c::Con_DPrintf(
                            c"%s.%s: 'spawnparam2' is deprecated, use 'spawnmode foo X Y'\n"
                                .as_ptr(),
                            ptr::addr_of!((*ptype).config).cast::<c_char>(),
                            ptr::addr_of!((*ptype).name).cast::<c_char>(),
                        );
                    } else if g::strcmp(var, c"up".as_ptr()) == 0 {
                        (*ptype).orgbias[2] = atofv(value) as c_float;
                        c::Con_DPrintf(
                            c"%s.%s: up is deprecated, use orgbias 0 0 Z\n".as_ptr(),
                            ptr::addr_of!((*ptype).config).cast::<c_char>(),
                            ptr::addr_of!((*ptype).name).cast::<c_char>(),
                        );
                    } else if g::strcmp(var, c"rampmode".as_ptr()) == 0 {
                        if g::strcmp(value, c"none".as_ptr()) == 0 {
                            (*ptype).rampmode = RAMP_NONE;
                        } else if g::strcmp(value, c"absolute".as_ptr()) == 0 {
                            c::Con_DPrintf(
                                c"%s.%s: 'rampmode absolute' is deprecated, use 'rampmode nearest'\n"
                                    .as_ptr(),
                                ptr::addr_of!((*ptype).config).cast::<c_char>(),
                                ptr::addr_of!((*ptype).name).cast::<c_char>(),
                            );
                            (*ptype).rampmode = RAMP_NEAREST;
                        } else if g::strcmp(value, c"nearest".as_ptr()) == 0 {
                            (*ptype).rampmode = RAMP_NEAREST;
                        } else if g::strcmp(value, c"lerp".as_ptr()) == 0 {
                            // don't use the name 'linear'. ramps are there to
                            // avoid linear...
                            (*ptype).rampmode = RAMP_LERP;
                        } else if g::strcmp(value, c"delta".as_ptr()) == 0 {
                            (*ptype).rampmode = RAMP_DELTA;
                        } else {
                            c::Con_DPrintf(
                                c"%s.%s: uses unknown ramp mode '%s', assuming 'delta'\n".as_ptr(),
                                ptr::addr_of!((*ptype).config).cast::<c_char>(),
                                ptr::addr_of!((*ptype).name).cast::<c_char>(),
                                value,
                            );
                            (*ptype).rampmode = RAMP_DELTA;
                        }
                    } else if g::strcmp(var, c"rampindexlist".as_ptr()) == 0 {
                        // better not use this with delta ramps...
                        let mut i = 1;
                        while i < c::Cmd_Argc() {
                            (*ptype).ramp = c::Mem_Realloc(
                                (*ptype).ramp.cast::<c_void>(),
                                core::mem::size_of::<ramp_t>()
                                    * ((*ptype).rampindexes as usize + 1),
                            )
                            .cast::<ramp_t>();

                            let r = (*ptype).ramp.add((*ptype).rampindexes as usize);
                            let mut cidx = atoiv(c::Cmd_Argv(i));
                            (*r).alpha = if cidx > 255 { 0.5 } else { 1.0 };

                            cidx = (cidx & 0xff) * 4;
                            (*r).rgb[0] =
                                (*palrgba.add(cidx as usize) as f64 * (1.0 / 255.0)) as c_float;
                            (*r).rgb[1] =
                                (*palrgba.add(cidx as usize + 1) as f64 * (1.0 / 255.0)) as c_float;
                            (*r).rgb[2] =
                                (*palrgba.add(cidx as usize + 2) as f64 * (1.0 / 255.0)) as c_float;

                            (*r).scale = (*ptype).scale;

                            (*ptype).rampindexes += 1;
                            i += 1;
                        }
                    } else if g::strcmp(var, c"rampindex".as_ptr()) == 0 {
                        (*ptype).ramp = c::Mem_Realloc(
                            (*ptype).ramp.cast::<c_void>(),
                            core::mem::size_of::<ramp_t>() * ((*ptype).rampindexes as usize + 1),
                        )
                        .cast::<ramp_t>();

                        let r = (*ptype).ramp.add((*ptype).rampindexes as usize);
                        let mut cidx = atoiv(value);
                        (*r).alpha = if cidx > 255 { 0.5 } else { 1.0 };

                        if c::Cmd_Argc() > 2 {
                            // they gave alpha
                            (*r).alpha = ((*r).alpha as f64 * atofv(c::Cmd_Argv(2))) as c_float;
                        }

                        cidx = (cidx & 0xff) * 4;
                        (*r).rgb[0] =
                            (*palrgba.add(cidx as usize) as f64 * (1.0 / 255.0)) as c_float;
                        (*r).rgb[1] =
                            (*palrgba.add(cidx as usize + 1) as f64 * (1.0 / 255.0)) as c_float;
                        (*r).rgb[2] =
                            (*palrgba.add(cidx as usize + 2) as f64 * (1.0 / 255.0)) as c_float;

                        if c::Cmd_Argc() > 3 {
                            // they gave scale
                            (*r).scale = atofv(c::Cmd_Argv(3)) as c_float;
                        } else {
                            (*r).scale = (*ptype).scale;
                        }

                        (*ptype).rampindexes += 1;
                    } else if g::strcmp(var, c"ramp".as_ptr()) == 0 {
                        (*ptype).ramp = c::Mem_Realloc(
                            (*ptype).ramp.cast::<c_void>(),
                            core::mem::size_of::<ramp_t>() * ((*ptype).rampindexes as usize + 1),
                        )
                        .cast::<ramp_t>();

                        let r = (*ptype).ramp.add((*ptype).rampindexes as usize);
                        (*r).rgb[0] = (atofv(value) / 255.0) as c_float;
                        if c::Cmd_Argc() > 3 {
                            // seperate rgb
                            (*r).rgb[1] = (atofv(c::Cmd_Argv(2)) / 255.0) as c_float;
                            (*r).rgb[2] = (atofv(c::Cmd_Argv(3)) / 255.0) as c_float;

                            if c::Cmd_Argc() > 4 {
                                // have we alpha and scale changes?
                                (*r).alpha = atofv(c::Cmd_Argv(4)) as c_float;
                                if c::Cmd_Argc() > 5 {
                                    // have we scale changes?
                                    (*r).scale = atofv(c::Cmd_Argv(5)) as c_float;
                                } else {
                                    (*r).scale = (*ptype).scaledelta;
                                }
                            } else {
                                (*r).alpha = (*ptype).alpha;
                                (*r).scale = (*ptype).scaledelta;
                            }
                        } else {
                            // they only gave one value
                            (*r).rgb[1] = (*r).rgb[0];
                            (*r).rgb[2] = (*r).rgb[0];

                            (*r).alpha = (*ptype).alpha;
                            (*r).scale = (*ptype).scaledelta;
                        }

                        (*ptype).rampindexes += 1;
                    } else if g::strcmp(var, c"viewspace".as_ptr()) == 0 {
                        c::Con_DPrintf(
                            c"%s.%s: viewspace particles are not supported in this build\n"
                                .as_ptr(),
                            ptr::addr_of!((*ptype).config).cast::<c_char>(),
                            ptr::addr_of!((*ptype).name).cast::<c_char>(),
                        );
                    } else if g::strcmp(var, c"perframe".as_ptr()) == 0 {
                        (*ptype).flags |= PT_INVFRAMETIME;
                    } else if g::strcmp(var, c"averageout".as_ptr()) == 0 {
                        (*ptype).flags |= PT_AVERAGETRAIL;
                    } else if g::strcmp(var, c"nostate".as_ptr()) == 0 {
                        (*ptype).flags |= PT_NOSTATE;
                    } else if g::strcmp(var, c"nospreadfirst".as_ptr()) == 0 {
                        (*ptype).flags |= PT_NOSPREADFIRST;
                    } else if g::strcmp(var, c"nospreadlast".as_ptr()) == 0 {
                        (*ptype).flags |= PT_NOSPREADLAST;
                    } else if g::strcmp(var, c"lightradius".as_ptr()) == 0 {
                        // float version
                        (*ptype).dl_radius[1] = atofv(value) as c_float;
                        (*ptype).dl_radius[0] = (*ptype).dl_radius[1];
                        if c::Cmd_Argc() > 2 {
                            (*ptype).dl_radius[1] = atofv(c::Cmd_Argv(2)) as c_float;
                        }
                        (*ptype).dl_radius[1] -= (*ptype).dl_radius[0];
                    } else if g::strcmp(var, c"lightradiusfade".as_ptr()) == 0 {
                        (*ptype).dl_decay[3] = atofv(value) as c_float;
                    } else if g::strcmp(var, c"lightrgb".as_ptr()) == 0 {
                        (*ptype).dl_rgb[0] = atofv(value) as c_float;
                        (*ptype).dl_rgb[1] = atofv(c::Cmd_Argv(2)) as c_float;
                        (*ptype).dl_rgb[2] = atofv(c::Cmd_Argv(3)) as c_float;
                    } else if g::strcmp(var, c"lightrgbfade".as_ptr()) == 0 {
                        (*ptype).dl_decay[0] = atofv(value) as c_float;
                        (*ptype).dl_decay[1] = atofv(c::Cmd_Argv(2)) as c_float;
                        (*ptype).dl_decay[2] = atofv(c::Cmd_Argv(3)) as c_float;
                    } else if g::strcmp(var, c"lightcorona".as_ptr()) == 0 {
                        (*ptype).dl_corona_intensity = atofv(value) as c_float;
                        (*ptype).dl_corona_scale = atofv(c::Cmd_Argv(2)) as c_float;
                    } else if g::strcmp(var, c"lighttime".as_ptr()) == 0 {
                        (*ptype).dl_time = atofv(value) as c_float;
                    } else if g::strcmp(var, c"lightshadows".as_ptr()) == 0 {
                        (*ptype).flags = ((*ptype).flags & !PT_NODLSHADOW)
                            | if atofv(value) != 0.0 {
                                0
                            } else {
                                PT_NODLSHADOW
                            };
                    } else if g::strcmp(var, c"lightcubemap".as_ptr()) == 0 {
                        (*ptype).dl_cubemapnum = atoiv(value);
                    } else if g::strcmp(var, c"lightscales".as_ptr()) == 0 {
                        // ambient diffuse specular
                        (*ptype).dl_scales[0] = atofv(value) as c_float;
                        (*ptype).dl_scales[1] = atofv(c::Cmd_Argv(2)) as c_float;
                        (*ptype).dl_scales[2] = atofv(c::Cmd_Argv(3)) as c_float;
                    } else if g::strcmp(var, c"spawnstain".as_ptr()) == 0 {
                        c::Con_DPrintf(
                            c"%s.%s: spawnstain is not supported in this build\n".as_ptr(),
                            ptr::addr_of!((*ptype).config).cast::<c_char>(),
                            ptr::addr_of!((*ptype).name).cast::<c_char>(),
                        );
                    } else if c::Cmd_Argc() != 0 {
                        c::Con_DPrintf(
                            c"%s.%s: %s is not a recognised particle type field\n".as_ptr(),
                            ptr::addr_of!((*ptype).config).cast::<c_char>(),
                            ptr::addr_of!((*ptype).name).cast::<c_char>(),
                            var,
                        );
                    }

                    break 'skipread;
                }
            }

            (*ptype).loaded = if part_parseweak { 1 } else { 2 };
            if (*ptype).clipcount < 1.0 {
                (*ptype).clipcount = 1.0;
            }

            if !settype {
                if (*ptype).looks.type_ == PT_NORMAL && (*ptype).texname[0] == 0 {
                    if (*ptype).scale != 0.0 {
                        (*ptype).looks.type_ = PT_SPARKFAN;
                        c::Con_DPrintf(
                            c"%s.%s: effect lacks a texture. assuming type sparkfan.\n".as_ptr(),
                            ptr::addr_of!((*ptype).config).cast::<c_char>(),
                            ptr::addr_of!((*ptype).name).cast::<c_char>(),
                        );
                    } else {
                        (*ptype).looks.type_ = PT_SPARK;
                        c::Con_DPrintf(
                            c"%s.%s: effect lacks a texture. assuming type spark.\n".as_ptr(),
                            ptr::addr_of!((*ptype).config).cast::<c_char>(),
                            ptr::addr_of!((*ptype).name).cast::<c_char>(),
                        );
                    }
                } else if (*ptype).looks.type_ == PT_SPARK {
                    if (*ptype).texname[0] != 0 {
                        (*ptype).looks.type_ = PT_TEXTUREDSPARK;
                    } else if (*ptype).scale != 0.0 {
                        (*ptype).looks.type_ = PT_SPARKFAN;
                    }
                }
            }

            // use old behavior if not using alphadelta
            if !setalphadelta {
                (*ptype).alphachange = (-(*ptype).alphachange / (*ptype).die) * (*ptype).alpha;
            }

            finish_particle_type(ptype);

            if (*ptype).looks.type_ == PT_BEAM && !setbeamlen {
                (*ptype).rotationstartmin = (1.0 / 128.0) as c_float;
            }
        }
    }
}

/// `r_part_fte.c:2594` -- `P_BeamInfo_f`, the `r_beaminfo` console command.
///
/// Raise-free: every callee is `Con_SafePrintf` (`r_part_fte.c:35` renames
/// `Con_Printf` to it), so this needs no `Host_Guard`.
unsafe fn p_beam_info_f() {
    // SAFETY: `free_beams`, `part_type` and `numparticletypes` are the
    // glue-owned globals declared in `quake_c_sys::r_part_fte`; the walk only
    // follows `next` links the allocator itself built.
    unsafe {
        let mut i: c_int = 0;

        let mut bs = g::free_beams;
        while !bs.is_null() {
            i += 1;
            bs = (*bs).next;
        }

        c::Con_SafePrintf(c"%i free beams\n".as_ptr(), i);

        for i in 0..g::numparticletypes {
            let mut m: c_int = 0;
            let mut l: c_int = 0;
            let mut k: c_int = 0;
            let mut j: c_int = 0;

            let mut bs = (*g::part_type.add(i as usize)).beams;
            while !bs.is_null() {
                if (*bs).p.is_null() {
                    k += 1;
                }

                if (*bs).flags & BS_DEAD != 0 {
                    l += 1;
                }

                if (*bs).flags & BS_LASTSEG != 0 {
                    m += 1;
                }

                j += 1;
                bs = (*bs).next;
            }

            if j != 0 {
                c::Con_SafePrintf(
                    c"Type %i = %i NULL p, %i DEAD, %i LASTSEG, %i total\n".as_ptr(),
                    i,
                    k,
                    l,
                    m,
                    j,
                );
            }
        }
    }
}

/// `r_part_fte.c:2627` -- `P_PartInfo_f`, the `r_partinfo` console command.
///
/// Raise-free for the same reason as `p_beam_info_f`.
unsafe fn p_part_info_f() {
    // SAFETY: as `p_beam_info_f` -- read-only walks of the glue-owned type
    // table, run list and free lists.
    unsafe {
        let mut totalp: c_int = 0;
        let mut totald: c_int = 0;
        let mut runningp: c_int = 0;
        let mut runningd: c_int = 0;
        let mut runninge: c_int = 0;
        let mut runningt: c_int = 0;

        c::Con_DPrintf(c"Full list of  effects:\n".as_ptr());
        for i in 0..g::numparticletypes {
            let pt = g::part_type.add(i as usize);

            let mut j: c_int = 0;
            let mut p = (*pt).particles;
            while !p.is_null() {
                j += 1;
                p = (*p).next;
            }
            totalp += j;

            let mut k: c_int = 0;
            let mut d = (*pt).clippeddecals;
            while !d.is_null() {
                k += 1;
                d = (*d).next;
            }
            totald += k;

            if j != 0 || k != 0 {
                c::Con_DPrintf(
                    c"Type %s.%s = %i+%i total\n".as_ptr(),
                    ptr::addr_of!((*pt).config).cast::<c_char>(),
                    ptr::addr_of!((*pt).name).cast::<c_char>(),
                    j,
                    k,
                );
                if (*pt).state & PS_INRUNLIST == 0 {
                    // `r_part_fte.c:155` -- `#define CON_WARNING "Warning: "`.
                    c::Con_SafePrintf(
                        c"Warning: %s.%s NOT RUNNING\n".as_ptr(),
                        ptr::addr_of!((*pt).config).cast::<c_char>(),
                        ptr::addr_of!((*pt).name).cast::<c_char>(),
                    );
                }
            }
        }

        c::Con_SafePrintf(c"Running effects:\n".as_ptr());
        // maintain run list
        let mut ptype = g::part_run_list;
        while !ptype.is_null() {
            c::Con_SafePrintf(
                c"Type %s.%s".as_ptr(),
                ptr::addr_of!((*ptype).config).cast::<c_char>(),
                ptr::addr_of!((*ptype).name).cast::<c_char>(),
            );

            let mut j: c_int = 0;
            let mut p = (*ptype).particles;
            while !p.is_null() {
                j += 1;
                p = (*p).next;
            }
            if j != 0 {
                c::Con_SafePrintf(c"\t%i particles".as_ptr(), j);
                if (*ptype).cliptype >= 0 || (*ptype).stainonimpact != 0.0 {
                    c::Con_SafePrintf(c"(+traceline)".as_ptr());
                    runningt += j;
                }
            }
            runningp += j;

            let mut k: c_int = 0;
            let mut d = (*ptype).clippeddecals;
            while !d.is_null() {
                k += 1;
                d = (*d).next;
            }
            if k != 0 {
                c::Con_SafePrintf(
                    c"%s%i decals".as_ptr(),
                    if (*ptype).particles.is_null() {
                        c"\t".as_ptr()
                    } else {
                        c", ".as_ptr()
                    },
                    k,
                );
            }
            runningd += k;

            c::Con_SafePrintf(c"\n".as_ptr());
            runninge += 1;

            ptype = (*ptype).nexttorun;
        }
        c::Con_SafePrintf(c"End of list\n".as_ptr());

        let mut freep: c_int = 0;
        let mut p = g::free_particles;
        while !p.is_null() {
            freep += 1;
            p = (*p).next;
        }
        let mut freed: c_int = 0;
        let mut d = g::free_decals;
        while !d.is_null() {
            freed += 1;
            d = (*d).next;
        }

        c::Con_DPrintf(c"%i running effects.\n".as_ptr(), runninge);
        c::Con_SafePrintf(
            c"%i particles, %i free, %i traces.\n".as_ptr(),
            runningp,
            freep,
            runningt,
        );
        c::Con_SafePrintf(c"%i decals, %i free.\n".as_ptr(), runningd, freed);

        if totalp != runningp {
            c::Con_SafePrintf(
                c"%i particles unaccounted for\n".as_ptr(),
                totalp - runningp,
            );
        }
        if totald != runningd {
            c::Con_SafePrintf(c"%i decals unaccounted for\n".as_ptr(), totald - runningd);
        }
    }
}

/// `r_part_fte.c:2701` -- `FinishParticleType`.
///
/// `P_LoadTexture` (`r_part_fte.c:1146-1327`) stays C because it is pure
/// renderer/texture-manager work, so it is reached through the
/// `FtePart_Glue_LoadTexture` shim. That shim is raise-free by construction:
/// the glue swallows nothing, it simply forwards, and `P_LoadTexture`'s own
/// callees (`TexMgr_*`, `Image_*`) do not `Host_Error`.
unsafe fn finish_particle_type(ptype: *mut part_type_t) {
    // SAFETY: `ptype` points into the glue-owned `part_type` array, and every
    // field read below is declared by the ADR-011 mirror.
    unsafe {
        // if there is a chance that it moves
        if (*ptype).gravity != 0.0
            || (*ptype).veladd != 0.0
            || (*ptype).spawnvel != 0.0
            || (*ptype).spawnvelvert != 0.0
            || m::dot_product(&(*ptype).velwrand, &(*ptype).velwrand) != 0.0
            || m::dot_product(&(*ptype).velbias, &(*ptype).velbias) != 0.0
            || (*ptype).flurry != 0.0
        {
            (*ptype).flags |= PT_VELOCITY;
        }
        if m::dot_product(&(*ptype).velbias, &(*ptype).velbias) != 0.0
            || m::dot_product(&(*ptype).velwrand, &(*ptype).velwrand) != 0.0
            || m::dot_product(&(*ptype).orgwrand, &(*ptype).orgwrand) != 0.0
        {
            (*ptype).flags |= PT_WORLDSPACERAND;
        }
        // if it has friction
        if (*ptype).friction[0] != 0.0 || (*ptype).friction[1] != 0.0 || (*ptype).friction[2] != 0.0
        {
            (*ptype).flags |= PT_FRICTION;
        }

        g::FtePart_Glue_LoadTexture(ptype, true);
        if (*ptype).dl_decay[3] != 0.0 && (*ptype).dl_time == 0.0 {
            (*ptype).dl_time = (*ptype).dl_radius[0] / (*ptype).dl_decay[3];
        }
        if (*ptype).looks.scalefactor > 1.0 && (*ptype).looks.invscalefactor == 0.0 {
            (*ptype).scale *= (*ptype).looks.scalefactor;
            (*ptype).scalerand *= (*ptype).looks.scalefactor;
            // too lazy to go through ramps
            (*ptype).looks.scalefactor = 1.0;
        }
        (*ptype).looks.invscalefactor = 1.0 - (*ptype).looks.scalefactor;

        if (*ptype).looks.type_ == PT_TEXTUREDSPARK && (*ptype).looks.stretch == 0.0 {
            (*ptype).looks.stretch = 0.05; // the old default.
        }

        if (*ptype).looks.type_ == PT_SPARK && cvar_value(ptr::addr_of!(g::r_part_sparks)) < 0.0 {
            (*ptype).looks.type_ = PT_INVISIBLE;
        }
        if (*ptype).looks.type_ == PT_TEXTUREDSPARK
            && cvar_value(ptr::addr_of!(g::r_part_sparks_textured)) == 0.0
        {
            (*ptype).looks.type_ = PT_SPARK;
        }
        if (*ptype).looks.type_ == PT_SPARKFAN
            && cvar_value(ptr::addr_of!(g::r_part_sparks_trifan)) == 0.0
        {
            (*ptype).looks.type_ = PT_SPARK;
        }
        if (*ptype).looks.type_ == PT_SPARK && cvar_value(ptr::addr_of!(g::r_part_sparks)) == 0.0 {
            (*ptype).looks.type_ = PT_INVISIBLE;
        }
        if (*ptype).looks.type_ == PT_BEAM && cvar_value(ptr::addr_of!(g::r_part_beams)) <= 0.0 {
            (*ptype).looks.type_ = PT_INVISIBLE;
        }

        if (*ptype).rampmode != RAMP_NONE && (*ptype).ramp.is_null() {
            (*ptype).rampmode = RAMP_NONE;
            c::Con_SafePrintf(
                c"%s.%s: Particle has a ramp mode but no ramp\n".as_ptr(),
                ptr::addr_of!((*ptype).config).cast::<c_char>(),
                ptr::addr_of!((*ptype).name).cast::<c_char>(),
            );
        } else if !(*ptype).ramp.is_null() && (*ptype).rampmode == RAMP_NONE {
            c::Con_SafePrintf(
                c"%s.%s: Particle has a ramp but no ramp mode\n".as_ptr(),
                ptr::addr_of!((*ptype).config).cast::<c_char>(),
                ptr::addr_of!((*ptype).name).cast::<c_char>(),
            );
        }
        R_PLOOKSDIRTY = true;
    }
}

/// `r_part_fte.c:2748` -- `FinishEffectinfoParticleType`.
///
/// COMPAT: ADR-010 -- every `*=` below whose right-hand side is a C `double`
/// literal (`1 / 1.414213562373095`, `0.04`, `0.000001`, `0.05`) is evaluated
/// in `f64` and narrowed on store, exactly as the C compiler does under
/// `-ffp-contract=off`; the ones whose operands are all `float` stay in `f32`.
unsafe fn finish_effectinfo_particle_type(ptype: *mut part_type_t, blooddecalonimpact: bool) {
    // SAFETY: as `finish_particle_type` -- `ptype` is a live entry of the
    // glue-owned type array.
    unsafe {
        if (*ptype).looks.type_ == PT_CDECAL {
            if (*ptype).die == 9999.0 {
                (*ptype).die = 20.0;
            }
            (*ptype).alphachange = -((*ptype).alpha / (*ptype).die);
        } else if (*ptype).looks.type_ == PT_UDECAL {
            // dp's decals have a size as a radius. fte's udecals are 'just'
            // quads. also, dp uses 'stretch'.
            // COMPAT: ADR-010 -- `r_part_fte.c:2770` writes the literal
            // `1.414213562373095`, which is one ULP below `f64::consts::SQRT_2`
            // (`0x3FF6A09E667F3BCC` vs `...CD`). Using the constant would change
            // the result bit pattern, so the literal is reproduced verbatim.
            #[allow(clippy::approx_constant)]
            let inv_sqrt2 = 1.0 / 1.414_213_562_373_095_f64;
            (*ptype).looks.stretch = (f64::from((*ptype).looks.stretch) * inv_sqrt2) as c_float;
            (*ptype).scale *= (*ptype).looks.stretch;
            (*ptype).scalerand *= (*ptype).looks.stretch;
            (*ptype).scaledelta *= (*ptype).looks.stretch;
            (*ptype).looks.stretch = 1.0;
        } else if (*ptype).looks.type_ == PT_NORMAL {
            // fte's textured particles are *0.25 for some reason.
            // but fte also uses radiuses, while dp uses total size so we only
            // need to double it here..
            (*ptype).scale *= 2.0 * (*ptype).looks.stretch;
            (*ptype).scalerand *= 2.0 * (*ptype).looks.stretch;
            (*ptype).scaledelta *= 2.0 * 2.0 * (*ptype).looks.stretch;
            (*ptype).looks.stretch = 1.0;
        }
        if blooddecalonimpact {
            // DP blood particles generate decals unconditionally (and prevent
            // blood from bouncing)
            (*ptype).clipbounce = -2.0;
        }
        if (*ptype).looks.type_ == PT_TEXTUREDSPARK {
            (*ptype).looks.stretch = (f64::from((*ptype).looks.stretch) * 0.04_f64) as c_float;
            if (*ptype).looks.stretch < 0.0 {
                (*ptype).looks.stretch = 0.000_001;
            }
        }

        if (*ptype).die == 9999.0 {
            // internal: means unspecified.
            if (*ptype).alphachange != 0.0 {
                (*ptype).die = ((*ptype).alpha + (*ptype).alpharand) / -(*ptype).alphachange;
            } else {
                (*ptype).die = 15.0;
            }
        }
        (*ptype).looks.minstretch = 0.5;
        finish_particle_type(ptype);
    }
}

/// `r_part_fte.c:2804` -- `P_ImportEffectInfo`, the DarkPlaces `effectinfo.txt`
/// importer.
///
/// # Safety
///
/// `config` must be NUL-terminated. `line` must point at a writable,
/// NUL-terminated buffer: the C original writes a NUL over each `'\n'` it
/// finds, and this port does the same.
///
/// COMPAT: ADR-010 -- the DP tokens whose right-hand side mixes an `atof`
/// result (a C `double`) with a `float` field are evaluated in `f64` and
/// narrowed on store, matching the C compiler under `-ffp-contract=off`.
///
/// COMPAT: ADR-004 -- the C original indexes `teximages[atoi (token)]` with
/// no bounds check, which is an out-of-bounds access (undefined behaviour,
/// C99 6.5.6p8) for a malformed file. There is no defined C behaviour to be
/// faithful to, and a panic would abort the process (every profile is
/// `panic = "abort"`), so an out-of-range index is ignored here instead.
#[allow(clippy::too_many_lines)]
// The C original spells several `type` values as separate arms with identical
// bodies (`static`/`smoke`, for one); the arms are kept one-for-one.
#[allow(clippy::if_same_then_else)]
unsafe fn p_import_effect_info(config: *const c_char, line: *mut c_char, part_parseweak: bool) {
    // SAFETY: string and buffer contracts per the fn docs; every `part_type_t`
    // field written below is declared by the ADR-011 mirror.
    unsafe {
        let mut line = line;
        let mut ptype: *mut part_type_t = ptr::null_mut();
        let mut arg = [[0 as c_char; 1024]; 8];
        let mut args: usize = 0;
        // tracked separately because it needs to override another field
        let mut blooddecalonimpact = false;

        let mut teximages = [[0.0 as c_float; 4]; 256];

        {
            // default assumes 8*8 grid, but we allow more
            #[allow(clippy::needless_range_loop)] // the index is also the grid cell
            for i in 0..256usize {
                let i_i = i as c_int;
                teximages[i][0] = ((1.0 / 8.0) * f64::from(i_i & 7)) as c_float;
                teximages[i][1] = ((1.0 / 8.0) * f64::from(1 + (i_i & 7))) as c_float;
                teximages[i][2] = ((1.0 / 8.0) * f64::from(1 + (i_i >> 3))) as c_float;
                teximages[i][3] = ((1.0 / 8.0) * f64::from(i_i >> 3)) as c_float;
            }

            let file = c::COM_LoadFile(c"particles/particlefont.txt".as_ptr(), ptr::null_mut())
                .cast::<c_char>();
            if !file.is_null() {
                let filesize = c::COM_ThreadFileSize() as usize;
                let mut linebuf = [0 as c_char; 1024];
                let mut offset: usize = 0;
                while !pscript_read_line(
                    linebuf.as_mut_ptr(),
                    core::mem::size_of_val(&linebuf),
                    file,
                    filesize,
                    ptr::addr_of_mut!(offset),
                )
                .is_null()
                {
                    let mut font_line = c::COM_Parse(linebuf.as_ptr());
                    let i = g::atoi(c::COM_ThreadToken());
                    font_line = c::COM_Parse(font_line);
                    let s1 = g::atof(c::COM_ThreadToken()) as c_float;
                    font_line = c::COM_Parse(font_line);
                    let t1 = g::atof(c::COM_ThreadToken()) as c_float;
                    font_line = c::COM_Parse(font_line);
                    let s2 = g::atof(c::COM_ThreadToken()) as c_float;
                    font_line = c::COM_Parse(font_line);
                    let t2 = g::atof(c::COM_ThreadToken()) as c_float;
                    if !font_line.is_null() {
                        if let Some(slot) =
                            usize::try_from(i).ok().and_then(|i| teximages.get_mut(i))
                        {
                            slot[0] = s1;
                            slot[1] = s2;
                            slot[2] = t2;
                            slot[3] = t1;
                        }
                    }
                }
                c::Mem_Free(file.cast::<c_void>());
            }
        }

        'outer: while !line.is_null() && *line != 0 {
            // multi-line comments need special handling.
            while *line == b' ' as c_char || *line == b'\t' as c_char {
                line = line.add(1);
            }
            if *line == b'/' as c_char && *line.add(1) == b'*' as c_char {
                line = line.add(2);
                while *line != 0 {
                    if *line == b'*' as c_char && *line.add(1) == b'/' as c_char {
                        line = line.add(2);
                        break;
                    }
                    line = line.add(1);
                }
                continue;
            }

            let mut eol = g::strchr(line, c_int::from(b'\n'));
            if !eol.is_null() {
                *eol = 0;
                eol = eol.add(1);
            }
            args = 0;
            while !line.is_null() {
                line = c::COM_Parse(line).cast_mut();
                if !line.is_null() && args < arg.len() {
                    g::q_strlcpy(
                        arg[args].as_mut_ptr(),
                        c::COM_ThreadToken(),
                        core::mem::size_of_val(&arg[args]),
                    );
                    args += 1;
                }
            }
            line = eol;

            if args == 0 {
                continue;
            }

            if g::strcmp(arg[0].as_ptr(), c"effect".as_ptr()) == 0 {
                let mut newname = [0 as c_char; 64];

                if !ptype.is_null() {
                    finish_effectinfo_particle_type(ptype, blooddecalonimpact);
                }
                blooddecalonimpact = false;

                ptype = p_get_particle_type(config, arg[1].as_ptr());
                if (*ptype).loaded != 0 {
                    let mut i: c_int = 0;
                    while i < 64 {
                        let parenttype = ptype.offset_from(g::part_type) as c_int;
                        g::q_snprintf(
                            newname.as_mut_ptr(),
                            core::mem::size_of_val(&newname),
                            c"%i+%s".as_ptr(),
                            i,
                            arg[1].as_ptr(),
                        );
                        ptype = p_get_particle_type(config, newname.as_ptr());
                        if (*ptype).loaded == 0 {
                            (*g::part_type.add(parenttype as usize)).assoc =
                                ptype.offset_from(g::part_type) as c_int;
                            break;
                        }
                        i += 1;
                    }
                    if i == 64 {
                        c::Con_SafePrintf(c"Too many duplicate names, gave up\n".as_ptr());
                        break 'outer;
                    }
                }
                p_reset_to_defaults(ptype);
                (*ptype).loaded = if part_parseweak { 1 } else { 2 };
                (*ptype).scale = 1.0;
                (*ptype).alpha = 0.0;
                (*ptype).alpharand = 1.0;
                (*ptype).alphachange = -1.0;
                (*ptype).die = 9999.0;
                g::strcpy(
                    ptr::addr_of_mut!((*ptype).texname).cast::<c_char>(),
                    c"particles/particlefont".as_ptr(),
                );
                (*ptype).rgb[0] = 1.0;
                (*ptype).rgb[1] = 1.0;
                (*ptype).rgb[2] = 1.0;

                (*ptype).colorindex = -1;
                (*ptype).spawnchance = 1.0;
                (*ptype).looks.scalefactor = 2.0;
                (*ptype).looks.invscalefactor = 0.0;
                (*ptype).looks.type_ = PT_NORMAL;
                (*ptype).looks.blendmode = BM_PREMUL;
                (*ptype).looks.premul = 1;
                (*ptype).looks.stretch = 1.0;

                (*ptype).dl_time = 0.0;

                // default texture is 63.
                (*ptype).s1 = teximages[63][0];
                (*ptype).s2 = teximages[63][1];
                (*ptype).t1 = teximages[63][2];
                (*ptype).t2 = teximages[63][3];
                (*ptype).texsstride = 0.0;
                (*ptype).randsmax = 1;
            } else if ptype.is_null() {
                c::Con_SafePrintf(c"Bad effectinfo file\n".as_ptr());
                break 'outer;
            } else if g::strcmp(arg[0].as_ptr(), c"countabsolute".as_ptr()) == 0 && args == 2 {
                (*ptype).countextra = g::atof(arg[1].as_ptr()) as c_float;
            } else if g::strcmp(arg[0].as_ptr(), c"count".as_ptr()) == 0 && args == 2 {
                (*ptype).count = g::atof(arg[1].as_ptr()) as c_float;
            } else if g::strcmp(arg[0].as_ptr(), c"type".as_ptr()) == 0 && args == 2 {
                if g::strcmp(arg[1].as_ptr(), c"decal".as_ptr()) == 0
                    || g::strcmp(arg[1].as_ptr(), c"cdecal".as_ptr()) == 0
                {
                    (*ptype).looks.type_ = PT_CDECAL;
                    (*ptype).looks.blendmode = BM_INVMODC;
                    (*ptype).looks.premul = 2;
                } else if g::strcmp(arg[1].as_ptr(), c"udecal".as_ptr()) == 0 {
                    (*ptype).looks.type_ = PT_UDECAL;
                    (*ptype).looks.blendmode = BM_INVMODC;
                    (*ptype).looks.premul = 2;
                } else if g::strcmp(arg[1].as_ptr(), c"alphastatic".as_ptr()) == 0 {
                    (*ptype).looks.type_ = PT_NORMAL;
                    (*ptype).looks.blendmode = BM_PREMUL; // BM_BLEND;
                    (*ptype).looks.premul = 1;
                } else if g::strcmp(arg[1].as_ptr(), c"static".as_ptr()) == 0 {
                    (*ptype).looks.type_ = PT_NORMAL;
                    (*ptype).looks.blendmode = BM_PREMUL; // BM_ADDA;
                    (*ptype).looks.premul = 2;
                } else if g::strcmp(arg[1].as_ptr(), c"smoke".as_ptr()) == 0 {
                    (*ptype).looks.type_ = PT_NORMAL;
                    (*ptype).looks.blendmode = BM_PREMUL; // BM_ADDA;
                    (*ptype).looks.premul = 2;
                } else if g::strcmp(arg[1].as_ptr(), c"spark".as_ptr()) == 0 {
                    (*ptype).looks.type_ = PT_TEXTUREDSPARK;
                    (*ptype).looks.blendmode = BM_PREMUL; // BM_ADDA;
                    (*ptype).looks.premul = 2;
                } else if g::strcmp(arg[1].as_ptr(), c"bubble".as_ptr()) == 0 {
                    (*ptype).looks.type_ = PT_NORMAL;
                    (*ptype).looks.blendmode = BM_PREMUL; // BM_ADDA;
                    (*ptype).looks.premul = 2;
                } else if g::strcmp(arg[1].as_ptr(), c"blood".as_ptr()) == 0 {
                    (*ptype).looks.type_ = PT_NORMAL;
                    (*ptype).looks.blendmode = BM_INVMODC;
                    (*ptype).looks.premul = 2;
                    (*ptype).gravity = 800.0;
                    blooddecalonimpact = true;
                } else if g::strcmp(arg[1].as_ptr(), c"beam".as_ptr()) == 0 {
                    (*ptype).looks.type_ = PT_BEAM;
                    (*ptype).looks.blendmode = BM_PREMUL; // BM_ADDA;
                    (*ptype).looks.premul = 2;
                } else if g::strcmp(arg[1].as_ptr(), c"snow".as_ptr()) == 0 {
                    (*ptype).looks.type_ = PT_NORMAL;
                    (*ptype).looks.blendmode = BM_PREMUL; // BM_ADDA;
                    (*ptype).looks.premul = 2;
                    // may not still be valid later, but at least it would be an
                    // obvious issue with the original.
                    (*ptype).flurry = 32.0;
                } else {
                    c::Con_SafePrintf(
                        c"effectinfo type %s not supported\n".as_ptr(),
                        arg[1].as_ptr(),
                    );
                }
            } else if g::strcmp(arg[0].as_ptr(), c"tex".as_ptr()) == 0 && args == 3 {
                let mini = g::atoi(arg[1].as_ptr());
                let maxi = g::atoi(arg[2].as_ptr());
                if let Some(mini_u) = usize::try_from(mini).ok().filter(|m| *m < teximages.len()) {
                    (*ptype).s1 = teximages[mini_u][0];
                    (*ptype).s2 = teximages[mini_u][1];
                    (*ptype).t1 = teximages[mini_u][2];
                    (*ptype).t2 = teximages[mini_u][3];
                    (*ptype).texsstride =
                        teximages[(mini_u + 1) & (teximages.len() - 1)][0] - teximages[mini_u][0];
                    (*ptype).randsmax = maxi - mini;
                    if (*ptype).randsmax < 1 {
                        (*ptype).randsmax = 1;
                    }
                }
            } else if g::strcmp(arg[0].as_ptr(), c"size".as_ptr()) == 0 && args == 3 {
                let s1 = g::atof(arg[1].as_ptr()) as c_float;
                let s2 = g::atof(arg[2].as_ptr()) as c_float;
                (*ptype).scale = s1;
                (*ptype).scalerand = s2 - s1;
            } else if g::strcmp(arg[0].as_ptr(), c"sizeincrease".as_ptr()) == 0 && args == 2 {
                (*ptype).scaledelta = g::atof(arg[1].as_ptr()) as c_float;
            } else if g::strcmp(arg[0].as_ptr(), c"color".as_ptr()) == 0 && args == 3 {
                let rgb1 = g::strtoul(arg[1].as_ptr(), ptr::null_mut(), 0) as c_uint;
                let rgb2 = g::strtoul(arg[2].as_ptr(), ptr::null_mut(), 0) as c_uint;
                for i in 0..3usize {
                    let shift = 16 - (i as c_uint) * 8;
                    let c1 = (rgb1 >> shift) & 0xff;
                    let c2 = (rgb2 >> shift) & 0xff;
                    (*ptype).rgb[i] = (f64::from(c1) / 255.0) as c_float;
                    (*ptype).rgbrand[i] =
                        (f64::from(c2.wrapping_sub(c1) as c_int) / 255.0) as c_float;
                    (*ptype).rgbrandsync[i] = 1.0;
                }
            } else if g::strcmp(arg[0].as_ptr(), c"alpha".as_ptr()) == 0 && args == 4 {
                let a1 = g::atof(arg[1].as_ptr()) as c_float;
                let a2 = g::atof(arg[2].as_ptr()) as c_float;
                let f = g::atof(arg[3].as_ptr()) as c_float;
                if a1 > a2 {
                    // backwards
                    (*ptype).alpha = a2 / 256.0;
                    (*ptype).alpharand = (a1 - a2) / 256.0;
                } else {
                    (*ptype).alpha = a1 / 256.0;
                    (*ptype).alpharand = (a2 - a1) / 256.0;
                }
                (*ptype).alphachange = -f / 256.0;
            } else if g::strcmp(arg[0].as_ptr(), c"velocityoffset".as_ptr()) == 0 && args == 4 {
                // a 3d world-coord addition
                (*ptype).velbias[0] = g::atof(arg[1].as_ptr()) as c_float;
                (*ptype).velbias[1] = g::atof(arg[2].as_ptr()) as c_float;
                (*ptype).velbias[2] = g::atof(arg[3].as_ptr()) as c_float;
            } else if g::strcmp(arg[0].as_ptr(), c"velocityjitter".as_ptr()) == 0 && args == 4 {
                (*ptype).velwrand[0] = g::atof(arg[1].as_ptr()) as c_float;
                (*ptype).velwrand[1] = g::atof(arg[2].as_ptr()) as c_float;
                (*ptype).velwrand[2] = g::atof(arg[3].as_ptr()) as c_float;
            } else if g::strcmp(arg[0].as_ptr(), c"originoffset".as_ptr()) == 0 && args == 4 {
                // a 3d world-coord addition
                (*ptype).orgbias[0] = g::atof(arg[1].as_ptr()) as c_float;
                (*ptype).orgbias[1] = g::atof(arg[2].as_ptr()) as c_float;
                (*ptype).orgbias[2] = g::atof(arg[3].as_ptr()) as c_float;
            } else if g::strcmp(arg[0].as_ptr(), c"originjitter".as_ptr()) == 0 && args == 4 {
                (*ptype).orgwrand[0] = g::atof(arg[1].as_ptr()) as c_float;
                (*ptype).orgwrand[1] = g::atof(arg[2].as_ptr()) as c_float;
                (*ptype).orgwrand[2] = g::atof(arg[3].as_ptr()) as c_float;
            } else if g::strcmp(arg[0].as_ptr(), c"gravity".as_ptr()) == 0 && args == 2 {
                (*ptype).gravity = (800.0 * g::atof(arg[1].as_ptr())) as c_float;
            } else if g::strcmp(arg[0].as_ptr(), c"bounce".as_ptr()) == 0 && args == 2 {
                (*ptype).clipbounce = g::atof(arg[1].as_ptr()) as c_float;
                if (*ptype).clipbounce < 0.0 {
                    (*ptype).cliptype = ptype.offset_from(g::part_type) as c_int;
                }
            } else if g::strcmp(arg[0].as_ptr(), c"airfriction".as_ptr()) == 0 && args == 2 {
                let f = g::atof(arg[1].as_ptr()) as c_float;
                (*ptype).friction[0] = f;
                (*ptype).friction[1] = f;
                (*ptype).friction[2] = f;
            } else if g::strcmp(arg[0].as_ptr(), c"liquidfriction".as_ptr()) == 0 && args == 2 {
                // deliberately ignored, as in the C original
            } else if g::strcmp(arg[0].as_ptr(), c"underwater".as_ptr()) == 0 && args == 1 {
                (*ptype).flags |= PT_TRUNDERWATER;
            } else if g::strcmp(arg[0].as_ptr(), c"notunderwater".as_ptr()) == 0 && args == 1 {
                (*ptype).flags |= PT_TROVERWATER;
            } else if g::strcmp(arg[0].as_ptr(), c"velocitymultiplier".as_ptr()) == 0 && args == 2 {
                (*ptype).veladd = g::atof(arg[1].as_ptr()) as c_float;
            } else if g::strcmp(arg[0].as_ptr(), c"trailspacing".as_ptr()) == 0 && args == 2 {
                (*ptype).countspacing = g::atof(arg[1].as_ptr()) as c_float;
                (*ptype).count = 1.0 / (*ptype).countspacing;
            } else if g::strcmp(arg[0].as_ptr(), c"time".as_ptr()) == 0 && args == 3 {
                (*ptype).die = g::atof(arg[1].as_ptr()) as c_float;
                (*ptype).randdie = (g::atof(arg[2].as_ptr()) - f64::from((*ptype).die)) as c_float;
                if (*ptype).randdie < 0.0 {
                    (*ptype).die = g::atof(arg[2].as_ptr()) as c_float;
                    (*ptype).randdie =
                        (g::atof(arg[1].as_ptr()) - f64::from((*ptype).die)) as c_float;
                }
            } else if g::strcmp(arg[0].as_ptr(), c"stretchfactor".as_ptr()) == 0 && args == 2 {
                (*ptype).looks.stretch = g::atof(arg[1].as_ptr()) as c_float;
            } else if g::strcmp(arg[0].as_ptr(), c"blend".as_ptr()) == 0 && args == 2 {
                if g::strcmp(arg[1].as_ptr(), c"invmod".as_ptr()) == 0 {
                    (*ptype).looks.blendmode = BM_INVMODC;
                    (*ptype).looks.premul = 2;
                } else if g::strcmp(arg[1].as_ptr(), c"alpha".as_ptr()) == 0 {
                    (*ptype).looks.blendmode = BM_PREMUL;
                    (*ptype).looks.premul = 1;
                } else if g::strcmp(arg[1].as_ptr(), c"add".as_ptr()) == 0 {
                    (*ptype).looks.blendmode = BM_PREMUL;
                    (*ptype).looks.premul = 2;
                } else {
                    c::Con_SafePrintf(
                        c"effectinfo 'blend %s' not supported\n".as_ptr(),
                        arg[1].as_ptr(),
                    );
                }
            } else if g::strcmp(arg[0].as_ptr(), c"orientation".as_ptr()) == 0 && args == 2 {
                if g::strcmp(arg[1].as_ptr(), c"billboard".as_ptr()) == 0 {
                    (*ptype).looks.type_ = PT_NORMAL;
                } else if g::strcmp(arg[1].as_ptr(), c"spark".as_ptr()) == 0 {
                    (*ptype).looks.type_ = PT_TEXTUREDSPARK;
                } else if g::strcmp(arg[1].as_ptr(), c"oriented".as_ptr()) == 0 {
                    // FIXME: not sure this points the right way. also, its
                    // double-sided in dp.
                    if (*ptype).looks.type_ != PT_CDECAL {
                        (*ptype).looks.type_ = PT_UDECAL;
                    }
                } else if g::strcmp(arg[1].as_ptr(), c"beam".as_ptr()) == 0 {
                    (*ptype).looks.type_ = PT_BEAM;
                } else {
                    c::Con_SafePrintf(
                        c"effectinfo 'orientation %s' not supported\n".as_ptr(),
                        arg[1].as_ptr(),
                    );
                }
            } else if g::strcmp(arg[0].as_ptr(), c"lightradius".as_ptr()) == 0 && args == 2 {
                (*ptype).dl_radius[0] = g::atof(arg[1].as_ptr()) as c_float;
                (*ptype).dl_radius[1] = 0.0;
            } else if g::strcmp(arg[0].as_ptr(), c"lightradiusfade".as_ptr()) == 0 && args == 2 {
                (*ptype).dl_decay[3] = g::atof(arg[1].as_ptr()) as c_float;
            } else if g::strcmp(arg[0].as_ptr(), c"lightcolor".as_ptr()) == 0 && args == 4 {
                (*ptype).dl_rgb[0] = g::atof(arg[1].as_ptr()) as c_float;
                (*ptype).dl_rgb[1] = g::atof(arg[2].as_ptr()) as c_float;
                (*ptype).dl_rgb[2] = g::atof(arg[3].as_ptr()) as c_float;
            } else if g::strcmp(arg[0].as_ptr(), c"lighttime".as_ptr()) == 0 && args == 2 {
                (*ptype).dl_time = g::atof(arg[1].as_ptr()) as c_float;
            } else if g::strcmp(arg[0].as_ptr(), c"lightshadow".as_ptr()) == 0 && args == 2 {
                (*ptype).flags = ((*ptype).flags & !PT_NODLSHADOW)
                    | if g::atoi(arg[1].as_ptr()) == 0 {
                        PT_NODLSHADOW
                    } else {
                        0
                    };
            } else if g::strcmp(arg[0].as_ptr(), c"lightcubemapnum".as_ptr()) == 0 && args == 2 {
                (*ptype).dl_cubemapnum = g::atoi(arg[1].as_ptr());
            } else if g::strcmp(arg[0].as_ptr(), c"lightcorona".as_ptr()) == 0 && args == 3 {
                // dp scales them by 0.25
                (*ptype).dl_corona_intensity = (g::atof(arg[1].as_ptr()) * 0.25) as c_float;
                (*ptype).dl_corona_scale = g::atof(arg[2].as_ptr()) as c_float;
            } else if (g::strcmp(arg[0].as_ptr(), c"staincolor".as_ptr()) == 0 && args == 3)
                || (g::strcmp(arg[0].as_ptr(), c"stainalpha".as_ptr()) == 0 && args == 3)
                || (g::strcmp(arg[0].as_ptr(), c"stainsize".as_ptr()) == 0 && args == 3)
                || (g::strcmp(arg[0].as_ptr(), c"staintex".as_ptr()) == 0 && args == 3)
                || (g::strcmp(arg[0].as_ptr(), c"stainless".as_ptr()) == 0 && args == 2)
            {
                // stainmaps multiplier / stain-decals: not supported here.
                c::Con_DPrintf2(
                    c"Particle effect token %s not supported\n".as_ptr(),
                    arg[0].as_ptr(),
                );
            } else if g::strcmp(arg[0].as_ptr(), c"rotate".as_ptr()) == 0 && args == 5 {
                (*ptype).rotationstartmin = g::atof(arg[1].as_ptr()) as c_float;
                (*ptype).rotationstartrand =
                    (g::atof(arg[2].as_ptr()) - f64::from((*ptype).rotationstartmin)) as c_float;
                (*ptype).rotationmin = g::atof(arg[3].as_ptr()) as c_float;
                (*ptype).rotationrand =
                    (g::atof(arg[4].as_ptr()) - f64::from((*ptype).rotationmin)) as c_float;
                (*ptype).rotationstartmin = (f64::from((*ptype).rotationstartmin)
                    * (core::f64::consts::PI / 180.0))
                    as c_float;
                (*ptype).rotationstartrand = (f64::from((*ptype).rotationstartrand)
                    * (core::f64::consts::PI / 180.0))
                    as c_float;
                (*ptype).rotationmin =
                    (f64::from((*ptype).rotationmin) * (core::f64::consts::PI / 180.0)) as c_float;
                (*ptype).rotationrand =
                    (f64::from((*ptype).rotationrand) * (core::f64::consts::PI / 180.0)) as c_float;
                (*ptype).rotationstartmin = (f64::from((*ptype).rotationstartmin)
                    + (core::f64::consts::PI / 4.0))
                    as c_float;
            } else {
                let opt = |n: usize| -> *const c_char {
                    if args < n + 1 {
                        c"".as_ptr()
                    } else {
                        arg[n].as_ptr()
                    }
                };
                c::Con_SafePrintf(
                    c"Particle effect token not recognised, or invalid args: %s %s %s %s %s %s\n"
                        .as_ptr(),
                    arg[0].as_ptr(),
                    opt(1),
                    opt(2),
                    opt(3),
                    opt(4),
                    opt(5),
                );
            }
            args = 0;
        }
        let _ = args;

        if !ptype.is_null() {
            finish_effectinfo_particle_type(ptype, blooddecalonimpact);
        }

        R_PLOOKSDIRTY = true;
    }
}

/// `r_part_fte.c:3243` -- `P_ImportEffectInfo_Name`.
///
/// # Safety
///
/// `config` must be NUL-terminated.
unsafe fn p_import_effect_info_name(config: *mut c_char) -> bool {
    // SAFETY: string contract per the fn docs; `COM_LoadFile` returns either
    // NULL or a `Mem_Free`-able block the callee owns.
    unsafe {
        let file =
            c::COM_LoadFile(g::va(c"%s.txt".as_ptr(), config), ptr::null_mut()).cast::<c_char>();
        if file.is_null() {
            c::Con_SafePrintf(c"%s.txt not found\n".as_ptr(), config);
            return false;
        }
        p_import_effect_info(config, file, false);
        c::Mem_Free(file.cast::<c_void>());
        true
    }
}

/// `r_part_fte.c:3262` -- `PScript_InitParticles`.
///
/// ADR-009 rule 3: `Cvar_RegisterVariable` can `Host_Error`, so all sixteen
/// registrations go through `FtePart_Glue_RegisterVariable`, which
/// `cvar_cmd_glue.c:300` builds as a `Host_Guard` wrapper. The three
/// `Cmd_AddCommand` calls register the glue's stable C thunks so a raise out
/// of a console command never unwinds through a Rust frame.
unsafe fn pscript_init_particles() -> Raise {
    // SAFETY: the sixteen cvars are the glue-owned `cvar_t` objects declared
    // in `quake_c_sys::r_part_fte`; the command thunks are glue functions.
    unsafe {
        for var in [
            ptr::addr_of_mut!(g::r_fteparticles), // johnfitz
            ptr::addr_of_mut!(g::r_bouncysparks),
            ptr::addr_of_mut!(g::r_part_rain),
            ptr::addr_of_mut!(g::r_decal_noperpendicular),
            ptr::addr_of_mut!(g::r_particledesc),
            ptr::addr_of_mut!(g::r_part_rain_quantity),
            ptr::addr_of_mut!(g::r_particle_tracelimit),
            ptr::addr_of_mut!(g::r_part_sparks),
            ptr::addr_of_mut!(g::r_part_sparks_trifan),
            ptr::addr_of_mut!(g::r_part_sparks_textured),
            ptr::addr_of_mut!(g::r_part_beams),
            ptr::addr_of_mut!(g::r_part_contentswitch),
            ptr::addr_of_mut!(g::r_part_density),
            ptr::addr_of_mut!(g::r_part_maxparticles),
            ptr::addr_of_mut!(g::r_part_maxdecals),
            ptr::addr_of_mut!(g::r_lightflicker),
        ] {
            let r = g::FtePart_Glue_RegisterVariable(var);
            if r != 0 {
                return r;
            }
        }

        c::Cmd_AddCommand2(
            c"r_partredirect".as_ptr(),
            Some(g::FtePart_Glue_PartRedirect_f),
            c::cmd_source_t_src_command,
            false,
        );

        c::Cmd_AddCommand2(
            c"r_partinfo".as_ptr(),
            Some(g::FtePart_Glue_PartInfo_f),
            c::cmd_source_t_src_command,
            false,
        );
        c::Cmd_AddCommand2(
            c"r_beaminfo".as_ptr(),
            Some(g::FtePart_Glue_BeamInfo_f),
            c::cmd_source_t_src_command,
            false,
        );

        0
    }
}

/// `r_part_fte.c:3286` -- `PScript_ClearSurfaceParticles`.
///
/// # Safety
///
/// `md` must point at a live `qmodel_t`.
unsafe fn pscript_clear_surface_particles(md: *mut QModel) {
    // SAFETY: `md` per the fn docs; the `skytrimem` chain is the pool this
    // module itself allocated, so every `next` link is a live block.
    unsafe {
        (*md).skytime = 0.0;
        (*md).skytris = ptr::null_mut();
        while !(*md).skytrimem.is_null() {
            let f = (*md).skytrimem;
            (*md).skytrimem = (*f.cast::<skytriblock_t>()).next.cast::<c_void>();
            c::Mem_Free(f.cast::<c_void>());
        }
    }
}

/// `r_part_fte.c:3296` -- `PScript_ClearAllSurfaceParticles`.
///
/// Reaches `mod_known[i]` through `FtePart_Glue_ModKnown` so the port carries
/// no dependency on the C `qmodel_t` array stride.
unsafe fn pscript_clear_all_surface_particles() {
    // SAFETY: the glue returns `&mod_known[i]` for every `i < mod_numknown`.
    unsafe {
        // make sure we hit all models, even ones from the previous map. maybe
        // this is overkill
        for i in 0..g::mod_numknown {
            pscript_clear_surface_particles(g::FtePart_Glue_ModKnown(i).cast::<QModel>());
        }
    }
}

/// `r_part_fte.c:3306` -- `PScript_Shutdown`.
///
/// ADR-009 rule 3: `CL_ClearTrailStates` can raise, so it is reached through
/// `FtePart_Glue_ClearTrailStates`. Everything after that call is a plain
/// teardown, and the C original would have skipped it on a `Host_Error`, so
/// the raise is propagated at the point it happens.
unsafe fn pscript_shutdown() -> Raise {
    // SAFETY: every list walked below is either file-private to this module
    // or the glue-owned `part_type` array.
    unsafe {
        c::Cvar_SetCallback(ptr::addr_of_mut!(g::r_particledesc), None);

        let r = g::FtePart_Glue_ClearTrailStates();
        if r != 0 {
            return r;
        }

        PE_DEFAULT = P_INVALID;
        PE_SIZE2 = P_INVALID;
        PE_SIZE3 = P_INVALID;
        PE_DEFAULTTRAIL = P_INVALID;

        while !LOADEDCONFIGS.is_null() {
            let cfg = LOADEDCONFIGS;
            LOADEDCONFIGS = (*cfg).next;
            c::Mem_Free(cfg.cast::<c_void>());
        }

        while g::numparticletypes > 0 {
            g::numparticletypes -= 1;
            let pt = g::part_type.add(g::numparticletypes as usize);
            if !(*pt).sounds.is_null() {
                c::Mem_Free((*pt).sounds.cast::<c_void>());
            }
            if !(*pt).ramp.is_null() {
                c::Mem_Free((*pt).ramp.cast::<c_void>());
            }
        }
        c::Mem_Free(g::part_type.cast::<c_void>());
        g::part_type = ptr::null_mut();
        g::part_run_list = ptr::null_mut();

        c::Mem_Free(PARTICLES.cast::<c_void>());
        PARTICLES = ptr::null_mut();
        c::Mem_Free(BEAMS.cast::<c_void>());
        BEAMS = ptr::null_mut();
        c::Mem_Free(DECALS.cast::<c_void>());
        DECALS = ptr::null_mut();
        c::Mem_Free(TRAILSTATES.cast::<c_void>());
        TRAILSTATES = ptr::null_mut();

        g::free_particles = ptr::null_mut();
        g::free_decals = ptr::null_mut();
        g::free_beams = ptr::null_mut();

        pscript_clear_all_surface_particles();

        R_NUMPARTICLES = 0;
        R_NUMDECALS = 0;

        0
    }
}

/// `r_part_fte.c:3361` -- `PScript_Startup`. The C original always returns
/// `true`, so only the raise code crosses the seam.
///
/// ADR-009 rule 3: the C tail ends with `r_particledesc.callback
/// (&r_particledesc)`, an indirect call whose target is always
/// `R_ParticleDesc_Callback` (`Cvar_SetCallback` above is the only writer,
/// and `PScript_Shutdown` is the only other). Calling the Rust body directly
/// keeps the `CL_RegisterParticles` raise inside it from having to `longjmp`
/// out through the glue thunk and back across this Rust frame.
///
/// COMPAT: ADR-010 -- `newmaxp = r_part_maxparticles.value` is C's implicit
/// `float`->`int` conversion, via `as_int`.
// The two `if` chains below are the C original's clamps; `clamp` would read
// better but the transliteration stays literal.
#[allow(clippy::manual_clamp)]
unsafe fn pscript_startup() -> Raise {
    // SAFETY: the cvars are the glue-owned objects, and the four pools are
    // this module's own statics.
    unsafe {
        let mut newmaxp = as_int(cvar_value(ptr::addr_of!(g::r_part_maxparticles)));
        if newmaxp < 1 {
            newmaxp = 1;
        }
        if newmaxp > MAX_PARTICLES {
            newmaxp = MAX_PARTICLES;
        }
        let mut newmaxd = as_int(cvar_value(ptr::addr_of!(g::r_part_maxdecals)));
        if newmaxd < 1 {
            newmaxd = 1;
        }
        if newmaxd > MAX_DECALS {
            newmaxd = MAX_DECALS;
        }

        if R_NUMPARTICLES == 0 {
            // already inited
            R_NUMPARTICLES = newmaxp;
            R_NUMDECALS = newmaxd;

            buildsintable();

            R_NUMBEAMS = MAX_BEAMSEGS;
            R_NUMTRAILSTATES = MAX_TRAILSTATES;

            PARTICLES = c::Mem_Alloc(R_NUMPARTICLES as usize * core::mem::size_of::<particle_t>())
                .cast::<particle_t>();

            BEAMS = c::Mem_Alloc(R_NUMBEAMS as usize * core::mem::size_of::<beamseg_t>())
                .cast::<beamseg_t>();

            DECALS = c::Mem_Alloc(R_NUMDECALS as usize * core::mem::size_of::<clippeddecal_t>())
                .cast::<clippeddecal_t>();

            TRAILSTATES =
                c::Mem_Alloc(R_NUMTRAILSTATES as usize * core::mem::size_of::<trailstate_t>())
                    .cast::<trailstate_t>();
            g::memset(
                TRAILSTATES.cast::<c_void>(),
                0,
                R_NUMTRAILSTATES as usize * core::mem::size_of::<trailstate_t>(),
            );
            TS_CYCLE = 0;

            c::Cvar_SetCallback(
                ptr::addr_of_mut!(g::r_particledesc),
                Some(g::FtePart_Glue_ParticleDesc_Callback),
            );
        }
        r_particle_desc_callback(ptr::addr_of_mut!(g::r_particledesc))
    }
}

/// `r_part_fte.c:3403` -- `PScript_RecalculateSkyTris`.
///
/// The C original reads `key[strlen (key) - 1]` without first checking that
/// `key` is non-empty, which is an out-of-bounds read for an empty worldspawn
/// key. The length check below is the minimum needed to keep the port free of
/// undefined behaviour; a non-empty key behaves identically.
unsafe fn pscript_recalculate_sky_tris() {
    // SAFETY: `cl.model_precache` entries are either NULL or live models; the
    // `remaps` block is this function's own allocation.
    unsafe {
        pscript_clear_all_surface_particles();

        for modidx in 0..MAX_MODELS {
            let md = cl.model_precache[modidx];

            if md.is_null() || (*md).needload || (*md).type_ != MOD_BRUSH {
                continue;
            }

            let mut key = [0 as c_char; 128];
            let mut data = c::COM_Parse((*md).entities);
            let remaps = c::Mem_Alloc(core::mem::size_of::<c_int>() * (*md).numtextures as usize)
                .cast::<c_int>();
            if remaps.is_null() {
                break;
            }
            for t in 0..(*md).numtextures {
                *remaps.add(t as usize) = P_INVALID;
            }

            // parse the worldspawn entity fields for "_texpart_FOO" keys to
            // give texture "FOO" particles from the effect specified by the
            // value
            if !data.is_null() && *c::COM_ThreadToken() == b'{' as c_char {
                loop {
                    data = c::COM_Parse(data);
                    if data.is_null() {
                        break; // error
                    }
                    if *c::COM_ThreadToken() == b'}' as c_char {
                        break; // end of worldspawn
                    }
                    if *c::COM_ThreadToken() == b'_' as c_char {
                        g::strcpy(key.as_mut_ptr(), c::COM_ThreadToken().add(1));
                    } else {
                        g::strcpy(key.as_mut_ptr(), c::COM_ThreadToken());
                    }
                    // remove trailing spaces
                    loop {
                        let l = g::strlen(key.as_ptr());
                        if l == 0 || *key.as_ptr().add(l - 1) != b' ' as c_char {
                            break;
                        }
                        *key.as_mut_ptr().add(l - 1) = 0;
                    }
                    data = c::COM_Parse(data);
                    if data.is_null() {
                        break; // error
                    }
                    if g::q_strncasecmp(c"texpart_".as_ptr(), key.as_ptr(), 8) == 0 {
                        // in quakespasm there are always two textures added on
                        // the end (rather than pointing to textures outside the
                        // model)
                        for t in 0..(*md).numtextures - 2 {
                            let tex = *(*md).textures.add(t as usize);
                            if tex.is_null() {
                                continue;
                            }
                            if g::q_strcasecmp(
                                key.as_ptr().add(8),
                                ptr::addr_of!((*tex).name).cast::<c_char>(),
                            ) == 0
                            {
                                *remaps.add(t as usize) =
                                    pscript_find_particle_type(c::COM_ThreadToken());
                            }
                        }
                    }
                }
            }

            for t in 0..(*md).numtextures {
                let tex = *(*md).textures.add(t as usize);
                let mut ptype = *remaps.add(t as usize);
                if ptype == P_INVALID && !tex.is_null() {
                    ptype = pscript_find_particle_type(g::va(
                        c"tex_%s".as_ptr(),
                        ptr::addr_of!((*tex).name).cast::<c_char>(),
                    ));
                }

                if ptype >= 0 {
                    for i in 0..(*md).nummodelsurfaces {
                        let surf = (*md)
                            .surfaces
                            .offset((i + (*md).firstmodelsurface) as isize);
                        if (*(*surf).texinfo).texture == tex {
                            // FIXME: it would be a good idea to determine the
                            // surface's (midpoint) pvs cluster so that we're
                            // not spamming for the entire map
                            pscript_emit_sky_effect_tris(md, surf, ptype);
                        }
                    }
                }
            }
            c::Mem_Free(remaps.cast::<c_void>());
        }
    }
}

/// `r_part_fte.c:3484` -- `PScript_ClearParticles`.
unsafe fn pscript_clear_particles(load: bool) -> Raise {
    // SAFETY: the four pools are this module's statics and are sized by
    // `pscript_startup`; `part_type` is the glue-owned array.
    unsafe {
        if load {
            let r = pscript_startup();
            if r != 0 {
                return r;
            }
        }

        g::free_particles = PARTICLES;
        for i in 0..R_NUMPARTICLES {
            (*PARTICLES.add(i as usize)).next = PARTICLES.add(i as usize + 1);
        }
        (*PARTICLES.add(R_NUMPARTICLES as usize - 1)).next = ptr::null_mut();

        g::free_decals = DECALS;
        for i in 0..R_NUMDECALS {
            (*DECALS.add(i as usize)).next = DECALS.add(i as usize + 1);
        }
        (*DECALS.add(R_NUMDECALS as usize - 1)).next = ptr::null_mut();

        g::free_beams = BEAMS;
        for i in 0..R_NUMBEAMS {
            let b = BEAMS.add(i as usize);
            (*b).p = ptr::null_mut();
            (*b).flags = BS_DEAD;
            (*b).next = BEAMS.add(i as usize + 1);
        }
        (*BEAMS.add(R_NUMBEAMS as usize - 1)).next = ptr::null_mut();

        g::particletime = cl.time as c_float;

        if load {
            for i in 0..g::numparticletypes {
                g::FtePart_Glue_LoadTexture(g::part_type.add(i as usize), false);
            }
        }

        for i in 0..g::numparticletypes {
            let pt = g::part_type.add(i as usize);
            (*pt).clippeddecals = ptr::null_mut();
            (*pt).particles = ptr::null_mut();
            (*pt).beams = ptr::null_mut();
        }

        pscript_clear_all_surface_particles();
        R_PLOOKSDIRTY = load;

        g::FtePart_Glue_ClearTrailStates()
    }
}

/// `r_part_fte.c:3535` -- `P_LoadParticleSet`.
///
/// The `PSET_CLASSIC` arm (`r_part_fte.c:3559-3568`) is compiled out in this
/// engine, so the `"classic"` name is still consumed and still returns `true`,
/// but there is no fallback particle system to switch to.
///
/// # Safety
///
/// `name` must be NUL-terminated.
unsafe fn p_load_particle_set(name: *mut c_char, implicit: bool, showwarning: bool) -> bool {
    // SAFETY: string contract per the fn docs; `loadedconfigs` is this
    // module's own list.
    unsafe {
        if *name == 0 {
            return false;
        }

        // protect against configs being loaded multiple times. this can easily
        // happen with namespaces (especially if an effect is missing).
        let mut cfg = LOADEDCONFIGS;
        while !cfg.is_null() {
            // already loaded?
            if g::strcmp(ptr::addr_of!((*cfg).name).cast::<c_char>(), name) == 0 {
                return false;
            }
            cfg = (*cfg).next;
        }
        let cfg = c::Mem_Alloc(core::mem::size_of::<pcfg_t>() + g::strlen(name)).cast::<pcfg_t>();
        if cfg.is_null() {
            return false;
        }
        g::strcpy(ptr::addr_of_mut!((*cfg).name).cast::<c_char>(), name);
        (*cfg).next = LOADEDCONFIGS;
        LOADEDCONFIGS = cfg;

        if g::strcmp(name, c"classic".as_ptr()) == 0 {
            return true;
        }

        let mut file = c::COM_LoadFile(g::va(c"particles/%s.cfg".as_ptr(), name), ptr::null_mut())
            .cast::<c_char>();
        if file.is_null() {
            file =
                c::COM_LoadFile(g::va(c"%s.cfg".as_ptr(), name), ptr::null_mut()).cast::<c_char>();
        }
        if !file.is_null() {
            pscript_parse_particle_effect_file(
                name,
                implicit,
                file,
                c::COM_ThreadFileSize() as usize,
            );
            c::Mem_Free(file.cast::<c_void>());
        } else {
            if g::strcmp(name, c"effectinfo".as_ptr()) == 0
                || g::strncmp(name, c"effectinfo_".as_ptr(), 11) == 0
            {
                // FIXME: we're loading this too early to deal with per-map
                // stuff.
                // FIXME: wait until after particle precache info has been
                // received, and only reload if the loaded configs actually
                // changed.
                p_import_effect_info_name(name);
                return true;
            }
            if showwarning {
                // `r_part_fte.c:155` -- `#define CON_WARNING "Warning: "`.
                c::Con_SafePrintf(
                    c"Warning: Couldn't find particle description %s\n".as_ptr(),
                    name,
                );
            }
            return false;
        }
        true
    }
}

/// `r_part_fte.c:3597` -- `R_Particles_KillAllEffects`.
///
/// The C loop body writes `part_type->ramp` and `part_type->rampmode` -- the
/// first element -- where `part_type[i]` was plainly meant. That is preserved
/// verbatim: the migration is bug-for-bug, and the aliasing is observable in
/// a `r_particledesc` change with more than one effect loaded.
unsafe fn r_particles_kill_all_effects() {
    // SAFETY: `part_type` is the glue-owned array of `numparticletypes`
    // entries; `loadedconfigs` is this module's own list.
    unsafe {
        for i in 0..g::numparticletypes {
            let pt = g::part_type.add(i as usize);
            *ptr::addr_of_mut!((*pt).texname).cast::<c_char>() = 0;
            (*pt).scale = 0.0;
            (*pt).loaded = 0;
            let first = g::part_type;
            if !(*first).ramp.is_null() {
                c::Mem_Free((*first).ramp.cast::<c_void>());
            }
            (*first).ramp = ptr::null_mut();
            (*first).rampmode = RAMP_NONE;
        }

        while !LOADEDCONFIGS.is_null() {
            let cfg = LOADEDCONFIGS;
            LOADEDCONFIGS = (*cfg).next;
            c::Mem_Free(cfg.cast::<c_void>());
        }
    }
}

/// `r_part_fte.c:3622` -- `R_ParticleDesc_Callback`, the `r_particledesc`
/// cvar callback.
///
/// ADR-009 rule 3: `CL_RegisterParticles` can raise, so it goes through
/// `FtePart_Glue_RegisterParticles` and the code is propagated to the caller
/// -- either `pscript_startup` above or the glue's
/// `FtePart_Glue_ParticleDesc_Callback` thunk, which re-raises it on the C
/// side.
///
/// The per-map arm builds its config name in `com_token` exactly as the C
/// original does. `com_token` is `THREAD_LOCAL` so it has no plain extern;
/// `COM_ThreadToken` returns the address of that same writable buffer.
///
/// # Safety
///
/// `var` must point at a live `cvar_t`.
unsafe fn r_particle_desc_callback(var: *mut c::cvar_t) -> Raise {
    // SAFETY: `var` per the fn docs. `COM_ThreadToken` addresses this
    // thread's `com_token[COM_PARSE_MAX_TOKEN_SIZE]`, which is writable
    // storage the parser itself fills, so the 4-byte prefix write and the
    // `COM_FileBase` write into `+ 4` are both in bounds.
    unsafe {
        r_particles_kill_all_effects();
        R_PLOOKSDIRTY = true;

        let mut ch: *const c_char = (*var).string;
        loop {
            ch = c::COM_Parse(ch);
            if ch.is_null() {
                break;
            }
            if *c::COM_ThreadToken() != 0 {
                p_load_particle_set(c::COM_ThreadToken().cast_mut(), false, true);
            }
        }

        if cls.state == CA_CONNECTED && !cl.model_precache[1].is_null() {
            // per-map configs. because we can.
            let token = c::COM_ThreadToken().cast_mut();
            g::memcpy(token.cast::<c_void>(), c"map_".as_ptr().cast::<c_void>(), 4);
            c::COM_FileBase(
                ptr::addr_of!((*cl.model_precache[1]).name).cast::<c_char>(),
                token.add(4),
                COM_PARSE_MAX_TOKEN_SIZE - 4,
            );
            p_load_particle_set(token, false, false);
        }

        // make sure nothing is stale.
        g::FtePart_Glue_RegisterParticles()
    }
}

/// `r_part_fte.c:3646` -- `P_AddRainParticles`.
///
/// COMPAT: ADR-010 -- `st->nexttime += 10000.0 / (...)` divides a `double`
/// literal by a `float` product, so the product is formed in `f32` and only
/// then widened, exactly as the C original does.
///
/// # Safety
///
/// `md` must be a live model whose `skytris` chain this module built, and
/// `axis` must address three `vec3_t`.
unsafe fn p_add_rain_particles(
    md: *mut QModel,
    axis: *const Vec3,
    eorg: &Vec3,
    contribution: c_float,
) -> Raise {
    // SAFETY: `md` and `axis` per the fn docs; `part_type` is the glue-owned
    // array and `st->ptype` is range-checked against `numparticletypes`.
    unsafe {
        if cvar_value(ptr::addr_of!(g::r_part_rain_quantity)) == 0.0 {
            return 0;
        }

        (*md).skytime += f64::from(contribution);

        let mut st = (*md).skytris.cast::<skytris_t>();
        while !st.is_null() {
            if (*st).ptype as c_uint >= g::numparticletypes as c_uint {
                st = (*st).next;
                continue;
            }
            let type_ = g::part_type.add((*st).ptype as usize);
            if (*type_).loaded == 0 {
                // woo, batch skipping.
                st = (*st).next;
                continue;
            }

            while (*st).nexttime < (*md).skytime {
                if g::free_particles.is_null() {
                    return 0;
                }

                (*st).nexttime += 10000.0
                    / f64::from(
                        (*st).area
                            * cvar_value(ptr::addr_of!(g::r_part_rain_quantity))
                            * (*type_).rainfrequency,
                    );

                let x = frandom() * frandom();
                let y = frandom() * (1.0 - x);
                let mut org: Vec3 = [0.0; 3];
                m::vector_ma(&(*st).org, x, &(*st).x, &mut org);
                let org_in = org;
                m::vector_ma(&org_in, y, &(*st).y, &mut org);

                let a0 = *axis;
                let a1 = *axis.add(1);
                let a2 = *axis.add(2);

                let mut worg: Vec3 = [
                    m::dot_product(&org, &a0) + eorg[0],
                    -m::dot_product(&org, &a1) + eorg[1],
                    m::dot_product(&org, &a2) + eorg[2],
                ];

                // ignore it if its too far away
                let mut vdist: Vec3 = [0.0; 3];
                m::vector_subtract(&worg, &ptr::addr_of!(r_refdef.vieworg).read(), &mut vdist);
                if m::vector_length(&vdist) > (1024.0 + 512.0) * frandom() {
                    continue;
                }

                let face = (*st).face.cast::<MSurface>();
                if (*face).flags & SURF_PLANEBACK != 0 {
                    m::vector_scale(&(*(*face).plane).normal, -1.0, &mut vdist);
                } else {
                    vdist = (*(*face).plane).normal;
                }

                let wnorm: Vec3 = [
                    m::dot_product(&vdist, &a0),
                    -m::dot_product(&vdist, &a1),
                    m::dot_product(&vdist, &a2),
                ];

                let worg_in = worg;
                m::vector_ma(&worg_in, 0.5, &wnorm, &mut worg);
                if cl_point_contents_mask(worg.as_mut_ptr()) & FTECONTENTS_SOLID == 0 {
                    // should be paranoia, at least for the world.
                    raise!(pscript_run_particle_effect_state(
                        &worg,
                        &wnorm,
                        1.0,
                        (*st).ptype,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    ));
                }
            }

            st = (*st).next;
        }

        0
    }
}

/// `r_part_fte.c:3708` -- `R_Part_SkyTri`.
///
/// COMPAT: ADR-010 -- `acos` is a real libm call (routed through
/// `quake_c_sys::libm`), but `sin` here resolves to `r_part_fte.c:186`'s
/// 128-entry table macro, not to libm. See `psin`.
///
/// # Safety
///
/// `md` must be a live model, `v1`/`v2`/`v3` three `vec3_t`, and `surf` a live
/// surface of `md`.
unsafe fn r_part_sky_tri(
    md: *mut QModel,
    v1: &Vec3,
    v2: &Vec3,
    v3: &Vec3,
    surf: *mut MSurface,
    ptype: c_int,
) {
    // SAFETY: pointer contracts per the fn docs; the `skytrimem` block is
    // this module's own allocation and `count` is kept below `tris.len()`.
    unsafe {
        let mut mem = (*md).skytrimem.cast::<skytriblock_t>();
        if mem.is_null() || (*mem).count as usize == (*mem).tris.len() {
            (*md).skytrimem = c::Mem_Alloc(core::mem::size_of::<skytriblock_t>());
            let fresh = (*md).skytrimem.cast::<skytriblock_t>();
            (*fresh).next = mem;
            (*fresh).count = 0;
            mem = fresh;
        }

        let st = ptr::addr_of_mut!((*mem).tris[(*mem).count as usize]);
        (*st).org = *v1;
        m::vector_subtract(v2, &(*st).org, &mut (*st).x);
        m::vector_subtract(v3, &(*st).org, &mut (*st).y);

        let xd = (*st).x;
        let yd = (*st).y;

        let xm = m::vector_length(&xd);
        let ym = m::vector_length(&yd);

        let dot = m::dot_product(&xd, &yd);
        let theta = libm::acos(f64::from(dot / (xm * ym))) as c_float;
        (*st).area = psin(theta) * xm * ym;
        (*st).nexttime = (*md).skytime;
        (*st).face = surf.cast::<c_void>();
        (*st).ptype = ptype;

        if (*st).area <= 0.0 {
            return; // bummer.
        }
        (*mem).count += 1;

        (*st).next = (*md).skytris.cast::<skytris_t>();
        (*md).skytris = st.cast::<c_void>();
    }
}

/// `r_part_fte.c:3754` -- `PScript_EmitSkyEffectTris`.
///
/// # Safety
///
/// `md` must be a live brush model and `fa` one of its surfaces.
unsafe fn pscript_emit_sky_effect_tris(md: *mut QModel, fa: *mut MSurface, ptype: c_int) {
    // SAFETY: pointer contracts per the fn docs; the edge indices come from
    // the model's own `surfedges` table.
    unsafe {
        let mut verts: [Vec3; 64] = [[0.0; 3]; 64];

        if ptype < 0 || ptype >= g::numparticletypes {
            return;
        }

        //
        // convert edges back to a normal polygon
        //
        let mut numverts: usize = 0;
        for i in 0..(*fa).numedges {
            let lindex = *(*md).surfedges.offset(((*fa).firstedge + i) as isize);

            let vec = if lindex > 0 {
                ptr::addr_of!(
                    (*(*md)
                        .vertexes
                        .add((*(*md).edges.add(lindex as usize)).v[0] as usize))
                    .position
                )
            } else {
                ptr::addr_of!(
                    (*(*md)
                        .vertexes
                        .add((*(*md).edges.add((-lindex) as usize)).v[1] as usize))
                    .position
                )
            };
            verts[numverts] = *vec;
            numverts += 1;

            if numverts >= 64 {
                c::Con_SafePrintf(c"Too many verts on sky surface\n".as_ptr());
                return;
            }
        }

        let v1 = 0usize;
        let mut v2 = 1usize;
        let mut v3 = 2usize;
        while v3 < numverts {
            r_part_sky_tri(md, &verts[v1], &verts[v2], &verts[v3], fa, ptype);

            v2 = v3;
            v3 += 1;
        }
    }
}

// Trailstate functions

/// `r_part_fte.c:3796` -- `P_CleanTrailstate`.
///
/// # Safety
///
/// `ts` must point at a live trailstate from the pool.
unsafe fn p_clean_trailstate(ts: *mut trailstate_t) {
    // SAFETY: `ts` per the fn docs; `lastbeam` is either NULL or a live
    // beamseg from the pool.
    unsafe {
        // clear LASTSEG flag from lastbeam so it can be reused
        if !(*ts).lastbeam.is_null() {
            (*(*ts).lastbeam).flags &= !BS_LASTSEG;
            (*(*ts).lastbeam).flags |= BS_NODRAW;
        }

        // clean structure
        g::memset(ts.cast::<c_void>(), 0, core::mem::size_of::<trailstate_t>());
    }
}

/// `r_part_fte.c:3808` -- `PScript_DelinkTrailstate`.
///
/// # Safety
///
/// `tsk` must point at a writable `trailstate_t *`.
unsafe fn pscript_delink_trailstate(tsk: *mut *mut trailstate_t) {
    // SAFETY: `tsk` per the fn docs. The `key != tsk` test is the C
    // original's recycle guard: a trailstate that has already been handed to
    // another owner is left alone.
    unsafe {
        if (*tsk).is_null() {
            return; // not linked to a trailstate
        }

        let ts = *tsk; // store old pointer
        *tsk = ptr::null_mut(); // clear pointer

        if (*ts).key != tsk {
            return; // prevent overwrite
        }

        let mut assoc = (*ts).assoc; // store assoc
        p_clean_trailstate(ts); // clean directly linked trailstate

        // clean trailstates assoc linked
        while !assoc.is_null() {
            let ts = (*assoc).assoc;
            p_clean_trailstate(assoc);
            assoc = ts;
        }
    }
}

/// `r_part_fte.c:3832` -- `P_NewTrailstate`.
///
/// # Safety
///
/// `key` must outlive the returned trailstate, or be delinked before it dies.
unsafe fn p_new_trailstate(key: *mut *mut trailstate_t) -> *mut trailstate_t {
    // SAFETY: `ts_cycle` is bounds-checked against `r_numtrailstates` before
    // the pool index, exactly as the C original does.
    unsafe {
        // bounds check here in case r_numtrailstates changed
        if TS_CYCLE >= R_NUMTRAILSTATES {
            TS_CYCLE = 0;
        }

        // get trailstate
        let ts = TRAILSTATES.offset(TS_CYCLE as isize);

        // clear trailstate
        p_clean_trailstate(ts);

        // set key
        (*ts).key = key;

        // advance index cycle
        TS_CYCLE += 1;

        // return clean trailstate
        ts
    }
}

// ---------------------------------------------------------------------------
// r_part_fte.c:3863-3925 -- the vertex-normal tables and PScript_EffectSpawned.
//
// r_part_fte.c:3928-4308 (decalctx_t, PScript_AddDecals, Fragment_ClipPolyToPlane,
// Fragment_ClipPoly, Q1BSP_Fragment_Surface, Q1BSP_ClipDecalToNodes and
// Mod_ClipDecal) deliberately stay C in `Quake/r_part_fte_glue.c`: every object
// they touch is already C-owned on the render side of the seam, and the family
// is pure qmodel_t BSP geometry with no simulation state. Rust reaches it
// through [`g::FtePart_Glue_ClipDecal`], handing over a [`g::decalctx_t`].

/// `COM_Rand () % n`.
///
/// COMPAT: ADR-004 -- `COM_Rand () % n` with `n == 0` is undefined in C
/// (C99 6.5.5p5), and C's behaviour is **not self-consistent across the
/// targets this project builds**: x86-64 `idiv` raises #DE -> SIGFPE, while
/// arm64 `sdiv`/`msub` is architecturally defined to yield 0. Rust's `%`
/// panics on both, and `panic = "abort"` makes that an immediate process
/// death, so `n == 0` is folded to zero here -- which is what the C build
/// already does on arm64. Same reasoning, and the same ADR, as
/// `crate::r_part`'s `colorMod % colorLength`.
///
/// Reachable through exactly one of the three callers.
/// `r_part_fte.c:4547-4548` guards with `if (ptype->spawnparam2)` -- a
/// *float* test -- and then truncates, so any `0 < |spawnparam2| < 1` from a
/// particle script passes the guard and yields `spawnspc == 0`. The other two
/// are already safe in C and stay so here: every `randsmax` assignment
/// (`r_part_fte.c:1188`, `:1408`, `:1681`, `:1712`, `:2957`, `:3049`) yields
/// at least 1, and `colorrand` is guarded `> 0` at its only use
/// (`r_part_fte.c:3999`).
///
/// # Safety
/// C ABI call into `COM_Rand`.
#[inline]
unsafe fn com_rand_mod(n: c_int) -> c_int {
    // SAFETY: `COM_Rand` takes no arguments and touches only its own state.
    unsafe {
        if n == 0 {
            0
        } else {
            c::COM_Rand().wrapping_rem(n)
        }
    }
}

/// Write a C return value through an optional out-parameter.
///
/// The ported entry points return both a `Host_Guard` status and the C
/// function's own `int`/`qboolean` result, so the latter travels in `out`.
///
/// # Safety
/// `out` must be null or address a writable `int`.
#[inline]
unsafe fn store(out: *mut c_int, v: c_int) {
    if !out.is_null() {
        // SAFETY: `out` per the fn docs.
        unsafe { *out = v };
    }
}

/// `r_part_fte.c:3874` -- `PScript_EffectSpawned`.
///
/// `axis` and `countscale` are unused by the C body; they are kept in the
/// signature so the call sites read like the original.
///
/// # Safety
///
/// `ptype` must be a live particle type and `org` a `vec3_t`.
unsafe fn pscript_effect_spawned(
    ptype: *mut part_type_t,
    org: *const Vec3,
    _axis: *const Vec3,
    dlkey: c_int,
    _countscale: c_float,
) {
    /// `r_part_fte.c:3878` -- `static int flickertime`.
    static mut FLICKERTIME: c_int = 0;
    /// `r_part_fte.c:3879` -- `static int flicker`.
    static mut FLICKER: c_int = 0;

    // SAFETY: pointer contracts per the fn docs. `sounds` holds `numsounds`
    // entries by construction (`P_ParticleEffect_f`'s `sound` handler).
    unsafe {
        if (*ptype).dl_radius[0] != 0.0 || (*ptype).dl_radius[1] != 0.0
        // && r_rocketlight.value
        {
            // COMPAT: ADR-010 -- `int i = realtime * 20` truncates a `double`
            // towards zero; Rust's `as` saturates instead of trapping.
            let i = (g::realtime * 20.0) as c_int;
            if FLICKERTIME != i {
                FLICKERTIME = i;
                FLICKER = c::COM_Rand();
            }

            // The conditional operator's two arms are `float` and `double`,
            // so the whole selection -- and the multiply that follows -- is
            // performed in `double` and only narrows on the assignment.
            let sel: f64 = if cvar_value(ptr::addr_of!(g::r_lightflicker)) != 0.0 {
                f64::from(
                    ((FLICKER.wrapping_add(dlkey.wrapping_mul(2000))) & 0xffff) as c_float
                        * (1.0f32 / 65535.0f32),
                )
            } else {
                0.5
            };
            let radius = (f64::from((*ptype).dl_radius[0]) + sel * f64::from((*ptype).dl_radius[1]))
                as c_float;

            if g::Tasks_IsWorker() {
                // the deferred spawn drain runs inside the task graph,
                // concurrently with tasks that read cl_dlights - queue for
                // PScript_FlushDlightsTask instead
                pscript_queue_dlight(
                    dlkey,
                    org,
                    radius,
                    (cl.time + f64::from((*ptype).dl_time)) as c_float,
                    (*ptype).dl_decay[3],
                    ptr::addr_of!((*ptype).dl_rgb),
                );
            } else {
                let dl = c::cl_tent::CL_AllocDlight(dlkey);
                (*dl).origin = *org;
                (*dl).radius = radius;
                (*dl).minlight = 0.0;
                (*dl).die = (cl.time + f64::from((*ptype).dl_time)) as c_float;
                (*dl).decay = (*ptype).dl_decay[3];
                (*dl).color = (*ptype).dl_rgb;
            }
        }

        if (*ptype).numsounds != 0 {
            let mut tw: c_float = 0.0;
            for i in 0..(*ptype).numsounds {
                tw += (*(*ptype).sounds.add(i as usize)).weight;
            }
            let w = frandom() * tw; // select the sound by weight
                                    // and figure out which one that weight corresponds to
            tw = 0.0;
            for i in 0..(*ptype).numsounds {
                let s = (*ptype).sounds.add(i as usize);
                tw += (*s).weight;
                if w <= tw {
                    if (*s).name[0] != 0 && (*s).vol > 0.0 {
                        // FIXME: no delay, no pitch
                        g::S_StartSound(
                            0,
                            0,
                            g::S_PrecacheSound(ptr::addr_of!((*s).name).cast::<c_char>()),
                            org.cast::<c_float>().cast_mut(),
                            (*s).vol,
                            (*s).atten,
                        );
                    }
                    break;
                }
            }
        }
    }
}

/// `r_part_fte.c:4311` -- `PScript_RunParticleEffectState`.
///
/// The C function returns `1` when the effect could not run and `0`
/// otherwise; that result travels through `out` because the return slot
/// carries the `Host_Guard` status of [`g::FtePart_Glue_EntityNum`]
/// (ADR-009 rule 3).
///
/// COMPAT: ADR-010 -- `PerpendicularVector` and `RotatePointAroundVector` are
/// both called below. ADR-010's Phase 7 (2026-09-03) amendment *narrowed* the
/// NaN-sign exception rather than removing it, so `quake_math`'s ports stay
/// the sanctioned implementations here.
///
/// # Safety
///
/// `org` must address a `vec3_t`, `dir` must be null or address one, `tsk`
/// must be null or address a writable `trailstate_t *`, and `out` must be
/// null or address a writable `int`.
#[allow(clippy::too_many_lines)]
unsafe fn pscript_run_particle_effect_state(
    org: *const Vec3,
    mut dir: *const Vec3,
    count: c_float,
    typenum: c_int,
    mut tsk: *mut *mut trailstate_t,
    out: *mut c_int,
) -> Raise {
    // SAFETY: pointer contracts per the fn docs. `part_type` is the
    // glue-owned array and every index into it is checked against
    // `numparticletypes` or comes from a validated `assoc`/`inwater` link.
    unsafe {
        let mut axis: [Vec3; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]];
        let mut ofsvec: Vec3 = [0.0; 3]; // offsetspread vec
        let mut arsvec: Vec3 = [0.0; 3]; // areaspread vec
        let mut bestdir: Vec3 = [0.0; 3];
        let palrgba = ptr::addr_of!(g::d_8to24table).cast::<u8>();

        // C forms `&part_type[typenum]` before the range test; the pointer is
        // never dereferenced until after it, so it is formed here instead.
        if typenum < 0 || typenum >= g::numparticletypes {
            store(out, 1);
            return 0;
        }
        let mut ptype = g::part_type.add(typenum as usize);

        if (*ptype).loaded == 0 {
            store(out, 1);
            return 0;
        }

        // inwater check, switch only once
        if cvar_value(ptr::addr_of!(g::r_part_contentswitch)) != 0.0
            && (*ptype).inwater >= 0
            && !cl.worldmodel.is_null()
        {
            let cont = cl_point_contents_mask(org.cast::<c_float>().cast_mut());

            if cont & FTECONTENTS_FLUID != 0 {
                ptype = g::part_type.add((*ptype).inwater as usize);
            }
        }

        // eliminate trailstate if flag set
        if (*ptype).flags & PT_NOSTATE != 0 {
            tsk = ptr::null_mut();
        }

        // trailstate allocation/deallocation
        let mut ts: *mut trailstate_t;
        if tsk.is_null() {
            ts = ptr::null_mut();
        } else if (*tsk).is_null() {
            // if *tsk = NULL get a new one
            ts = p_new_trailstate(tsk);
            *tsk = ts;
        } else {
            ts = *tsk;

            if (*ts).key != tsk {
                // trailstate was overwritten
                ts = p_new_trailstate(tsk); // so get a new one
                *tsk = ts;
            }
        }

        // get msvc to shut up
        let mut j: c_int = 0;
        let mut k: c_int = 0;
        let mut l: c_int = 0;
        let mut m: c_float = 0.0;
        // C declares `i` with the rest of the block; every read is preceded
        // by the `i = 0` that opens the spawning loop.
        let mut i: c_int;

        'run: while !ptype.is_null() {
            'skip: {
                if cvar_value(ptr::addr_of!(g::r_part_contentswitch)) != 0.0
                    && (*ptype).flags & (PT_TRUNDERWATER | PT_TROVERWATER) != 0
                    && !cl.worldmodel.is_null()
                {
                    let cont = cl_point_contents_mask(org.cast::<c_float>().cast_mut());

                    if (*ptype).flags & PT_TROVERWATER != 0 && cont & (*ptype).fluidmask != 0 {
                        break 'skip;
                    }
                    if (*ptype).flags & PT_TRUNDERWATER != 0 && cont & (*ptype).fluidmask == 0 {
                        break 'skip;
                    }
                }

                if !dir.is_null() && ((*dir)[0] != 0.0 || (*dir)[1] != 0.0 || (*dir)[2] != 0.0) {
                    axis[2] = *dir;
                    m::vector_normalize(&mut axis[2]);
                    let a2 = axis[2];
                    m::perpendicular_vector(&mut axis[0], &a2);
                    m::vector_normalize(&mut axis[0]);
                    let a0 = axis[0];
                    m::cross_product(&a2, &a0, &mut axis[1]);
                    m::vector_normalize(&mut axis[1]);
                }
                pscript_effect_spawned(ptype, org, axis.as_ptr(), 0, count);

                if (*ptype).looks.type_ == PT_CDECAL {
                    let mut vec: Vec3 = [0.5, 0.5, 0.5];
                    let mut start: Vec3 = [0.0; 3];
                    let mut end: Vec3 = [0.0; 3];
                    let mut ctx = g::decalctx_t {
                        ptype: ptr::null_mut(),
                        entity: 0,
                        model: ptr::null_mut(),
                        center: [0.0; 3],
                        normal: [0.0; 3],
                        tangent1: [0.0; 3],
                        tangent2: [0.0; 3],
                        scale0: 0.0,
                        scale1: 0.0,
                        scale2: 0.0,
                        bias1: 0.0,
                        bias2: 0.0,
                    };

                    if g::free_decals.is_null() {
                        store(out, 0);
                        return 0;
                    }

                    ctx.entity = 0;

                    ctx.center = *org;
                    if dir.is_null() || ((*dir)[0] == 0.0 && (*dir)[1] == 0.0 && (*dir)[2] == 0.0) {
                        let mut bestfrac: c_float = 1.0;
                        bestdir[0] = 0.0;
                        bestdir[1] = 0.73;
                        bestdir[2] = 0.73;
                        m::vector_normalize(&mut bestdir);
                        for n in 0..6 {
                            if n >= 3 {
                                end[0] = (i32::from(n == 3) * 16) as c_float;
                                end[1] = (i32::from(n == 4) * 16) as c_float;
                                end[2] = (i32::from(n == 5) * 16) as c_float;
                            } else {
                                end[0] = (-i32::from(n == 0) * 16) as c_float;
                                end[1] = (-i32::from(n == 1) * 16) as c_float;
                                end[2] = (-i32::from(n == 2) * 16) as c_float;
                            }
                            let ends = end;
                            m::vector_subtract(&*org, &ends, &mut start);
                            m::vector_add(&*org, &ends, &mut end);

                            let mut impact: Vec3 = [0.0; 3];
                            let mut normal: Vec3 = [0.0; 3];
                            let mut what: c_int = 0;
                            let frac = quake_rs_ftepart_trace_line(
                                start.as_mut_ptr(),
                                end.as_mut_ptr(),
                                impact.as_mut_ptr(),
                                normal.as_mut_ptr(),
                                ptr::addr_of_mut!(what),
                            );
                            if bestfrac > frac {
                                bestfrac = frac;
                                bestdir = normal;
                                ctx.center = impact;
                                ctx.entity = what;
                            }
                        }
                        dir = ptr::addr_of!(bestdir);
                    } else {
                        // try to get it exactly on the plane, otherwise
                        // network or collision inprecisions can leave us
                        // further away from the surface than the radius of
                        // the decal
                        m::vector_subtract(&*org, &*dir, &mut start);
                        m::vector_add(&*org, &*dir, &mut end);
                        quake_rs_ftepart_trace_line(
                            start.as_mut_ptr(),
                            end.as_mut_ptr(),
                            ptr::addr_of_mut!(ctx.center).cast::<c_float>(),
                            bestdir.as_mut_ptr(),
                            ptr::addr_of_mut!(ctx.entity),
                        );
                    }
                    if ctx.entity != 0 {
                        let mut entp: *mut c_void = ptr::null_mut();
                        raise!(g::FtePart_Glue_EntityNum(
                            ctx.entity,
                            ptr::addr_of_mut!(entp)
                        ));
                        let ent = entp.cast::<Entity>();
                        if (*ent).model.is_null() {
                            ctx.entity = 0;
                            ctx.model = cl.worldmodel.cast::<c_void>();
                        } else {
                            // looks like its active.
                            ctx.model = (*ent).model.cast::<c_void>();
                            // FIXME: rotate normal
                            let centre = ctx.center;
                            m::vector_subtract(&centre, &(*ent).origin, &mut ctx.center);
                        }
                    } else {
                        ctx.entity = 0;
                        ctx.model = cl.worldmodel.cast::<c_void>();
                    }
                    if ctx.model.is_null() {
                        store(out, 0);
                        return 0;
                    }

                    m::vector_scale(&*dir, -1.0, &mut ctx.normal);
                    m::vector_normalize(&mut ctx.normal);

                    // we know the normal now. pick two random tangents.
                    m::vector_normalize(&mut vec);
                    let normal = ctx.normal;
                    m::cross_product(&normal, &vec, &mut ctx.tangent1);
                    let tangent1 = ctx.tangent1;
                    m::rotate_point_around_vector(
                        &mut ctx.tangent2,
                        &normal,
                        &tangent1,
                        frandom() * 360.0,
                    );
                    let tangent2 = ctx.tangent2;
                    m::cross_product(&normal, &tangent2, &mut ctx.tangent1);

                    m::vector_normalize(&mut ctx.tangent1);
                    m::vector_normalize(&mut ctx.tangent2);

                    ctx.ptype = ptype;
                    ctx.scale1 = (*ptype).s2 - (*ptype).s1;
                    ctx.bias1 = (*ptype).s1 + ctx.scale1 / 2.0;
                    ctx.scale2 = (*ptype).t2 - (*ptype).t1;
                    ctx.bias2 = (*ptype).t1 + ctx.scale2 / 2.0;
                    m = (*ptype).scale + frandom() * (*ptype).scalerand;
                    ctx.scale0 = (2.0 / f64::from(m)) as c_float;
                    ctx.scale1 /= m;
                    ctx.scale2 /= m;

                    if (*ptype).randsmax != 1 {
                        ctx.bias1 +=
                            (*ptype).texsstride * com_rand_mod((*ptype).randsmax) as c_float;
                    }

                    // inserts decals through a callback.
                    g::FtePart_Glue_ClipDecal(
                        ctx.model,
                        ptr::addr_of_mut!(ctx.center).cast::<c_float>(),
                        ptr::addr_of_mut!(ctx.normal).cast::<c_float>(),
                        ptr::addr_of_mut!(ctx.tangent2).cast::<c_float>(),
                        ptr::addr_of_mut!(ctx.tangent1).cast::<c_float>(),
                        m,
                        (*ptype).surfflagmask as c_uint,
                        (*ptype).surfflagmatch as c_uint,
                        ptr::addr_of_mut!(ctx).cast::<c_void>(),
                    );

                    if (*ptype).assoc < 0 {
                        break 'run;
                    }
                    ptype = g::part_type.add((*ptype).assoc as usize);
                    continue 'run;
                }

                // init spawn specific variables
                let mut b: *mut beamseg_t = ptr::null_mut();
                let mut bfirst: *mut beamseg_t = ptr::null_mut();
                let mut spawnspc: c_int = 8;
                let mut pcount = (*ptype).countextra
                    + cvar_value(ptr::addr_of!(g::r_part_density))
                        * count
                        * ((*ptype).count + (*ptype).countrand * frandom());
                if (*ptype).flags & PT_INVFRAMETIME != 0 {
                    // `host_frametime` is a `double`, so the division widens.
                    pcount = (f64::from(pcount) / c::host_frametime) as c_float;
                }
                if !ts.is_null() {
                    pcount += (*ts).state2;
                }

                match (*ptype).spawnmode {
                    SM_UNICIRCLE => {
                        m = pcount;
                        if (*ptype).looks.type_ == PT_BEAM {
                            m -= 1.0;
                        }

                        if m < 1.0 {
                            m = 0.0;
                        } else {
                            m = ((core::f64::consts::PI * 2.0) / f64::from(m)) as c_float;
                        }

                        if (*ptype).spawnparam1 != 0.0 {
                            // use for weird shape hacks
                            m *= (*ptype).spawnparam1;
                        }
                    }
                    SM_TELEBOX | SM_LAVASPLASH => {
                        if (*ptype).spawnmode == SM_TELEBOX {
                            // C falls through from SM_TELEBOX into
                            // SM_LAVASPLASH; these two lines are the only
                            // telebox-specific part of the prologue.
                            spawnspc = 4;
                            l = as_int(-(*ptype).areaspreadvert);
                        }
                        k = as_int(-(*ptype).areaspread);
                        j = k;
                        if (*ptype).spawnparam1 != 0.0 {
                            m = (*ptype).spawnparam1;
                        } else {
                            // default weird number for tele/lavasplash used
                            // in vanilla Q1
                            m = 0.55752_f64 as c_float;
                        }

                        if (*ptype).spawnparam2 != 0.0 {
                            spawnspc = as_int((*ptype).spawnparam2);
                        }
                    }
                    SM_FIELD => {
                        let av_base = ptr::addr_of_mut!(AVELOCITIES).cast::<[c_float; 2]>();
                        if (*av_base)[0] == 0.0 {
                            for k in 0..NUMVERTEXNORMALS {
                                let a = av_base.add(k);
                                (*a)[0] = (f64::from(c::COM_Rand() & 255) * 0.01) as c_float;
                                (*a)[1] = (f64::from(c::COM_Rand() & 255) * 0.01) as c_float;
                            }
                        }

                        j = 0;
                        m = 0.0;
                    }
                    // others don't need intitialisation
                    _ => {}
                }

                // time limit (for completeness)
                if (*ptype).spawntime != 0.0 && !ts.is_null() {
                    if (*ts).state1 > g::particletime {
                        store(out, 0);
                        return 0; // timelimit still in effect
                    }

                    // record old time
                    (*ts).state1 = g::particletime + (*ptype).spawntime;
                }

                // random chance for point effects
                if (*ptype).spawnchance < frandom() {
                    // C sets `i = ceil (pcount)` here, but the `break` leaves
                    // the loop for the function's `return 0` without ever
                    // reading `i` again.
                    break 'run;
                }

                // this is a hack, use countextra=1, count=0
                if (*ptype).die == 0.0
                    && (*ptype).count == 1.0
                    && (*ptype).countrand == 0.0
                    && pcount < 1.0
                {
                    pcount = 1.0;
                }

                // particle spawning loop
                i = 0;
                while (i as c_float) < pcount {
                    if g::free_particles.is_null() {
                        break;
                    }
                    let p = g::free_particles;
                    if (*ptype).looks.type_ == PT_BEAM {
                        if g::free_beams.is_null() {
                            break;
                        }
                        if b.is_null() {
                            b = g::free_beams;
                            bfirst = b;
                        } else {
                            (*b).next = g::free_beams;
                            b = (*b).next;
                        }
                        g::free_beams = (*g::free_beams).next;
                        (*b).texture_s = i as c_float; // TODO: FIX THIS NUMBER
                        (*b).flags = 0;
                        (*b).p = p;
                        (*b).dir = [0.0; 3];
                    }
                    g::free_particles = (*p).next;
                    (*p).next = (*ptype).particles;
                    (*ptype).particles = p;

                    (*p).die = (*ptype).randdie * frandom();
                    (*p).scale = (*ptype).scale + (*ptype).scalerand * frandom();
                    if (*ptype).die != 0.0 {
                        (*p).rgba[3] = (*ptype).alpha + (*p).die * (*ptype).alphachange;
                    } else {
                        (*p).rgba[3] = (*ptype).alpha;
                    }
                    (*p).rgba[3] += (*ptype).alpharand * frandom();
                    if (*ptype).emittime < 0.0 {
                        (*p).state.trailstate = ptr::null_mut();
                    } else {
                        (*p).state.nextemit = g::particletime + (*ptype).emitstart - (*p).die;
                    }

                    (*p).rotationspeed = (*ptype).rotationmin + frandom() * (*ptype).rotationrand;
                    (*p).angle = (*ptype).rotationstartmin + frandom() * (*ptype).rotationstartrand;
                    (*p).s1 = (*ptype).s1;
                    (*p).t1 = (*ptype).t1;
                    (*p).s2 = (*ptype).s2;
                    (*p).t2 = (*ptype).t2;
                    if (*ptype).randsmax != 1 {
                        m = (*ptype).texsstride * com_rand_mod((*ptype).randsmax) as c_float;
                        (*p).s1 += m;
                        (*p).s2 += m;
                    }

                    if (*ptype).colorindex >= 0 {
                        let mut cidx = if (*ptype).colorrand > 0 {
                            com_rand_mod((*ptype).colorrand)
                        } else {
                            0
                        };
                        cidx = (*ptype).colorindex.wrapping_add(cidx);
                        if cidx > 255 {
                            (*p).rgba[3] /= 2.0; // Hexen 2 style transparency
                        }
                        cidx = (cidx & 0xff) * 4;
                        let base = cidx as usize;
                        (*p).rgba[0] = (f64::from(*palrgba.add(base)) * (1.0 / 255.0)) as c_float;
                        (*p).rgba[1] =
                            (f64::from(*palrgba.add(base + 1)) * (1.0 / 255.0)) as c_float;
                        (*p).rgba[2] =
                            (f64::from(*palrgba.add(base + 2)) * (1.0 / 255.0)) as c_float;
                    } else {
                        (*p).rgba[0] = (*ptype).rgb[0];
                        (*p).rgba[1] = (*ptype).rgb[1];
                        (*p).rgba[2] = (*ptype).rgb[2];
                    }

                    // use org temporarily for rgbsync
                    (*p).org[2] = frandom();
                    (*p).org[0] = (*p).org[2] * (*ptype).rgbrandsync[0]
                        + frandom() * (1.0 - (*ptype).rgbrandsync[0]);
                    (*p).org[1] = (*p).org[2] * (*ptype).rgbrandsync[1]
                        + frandom() * (1.0 - (*ptype).rgbrandsync[1]);
                    (*p).org[2] = (*p).org[2] * (*ptype).rgbrandsync[2]
                        + frandom() * (1.0 - (*ptype).rgbrandsync[2]);

                    (*p).rgba[0] +=
                        (*p).org[0] * (*ptype).rgbrand[0] + (*ptype).rgbchange[0] * (*p).die;
                    (*p).rgba[1] +=
                        (*p).org[1] * (*ptype).rgbrand[1] + (*ptype).rgbchange[1] * (*p).die;
                    (*p).rgba[2] +=
                        (*p).org[2] * (*ptype).rgbrand[2] + (*ptype).rgbchange[2] * (*p).die;

                    (*p).vel[0] = 0.0;
                    (*p).vel[1] = 0.0;
                    (*p).vel[2] = 0.0;

                    // handle spawn modes (org/vel)
                    match (*ptype).spawnmode {
                        SM_BOX => {
                            ofsvec[0] = crandom();
                            ofsvec[1] = crandom();
                            ofsvec[2] = crandom();

                            arsvec[0] = ofsvec[0] * (*ptype).areaspread;
                            arsvec[1] = ofsvec[1] * (*ptype).areaspread;
                            arsvec[2] = ofsvec[2] * (*ptype).areaspreadvert;
                        }
                        SM_TELEBOX => {
                            ofsvec[0] = k as c_float;
                            ofsvec[1] = j as c_float;
                            ofsvec[2] = (l + 4) as c_float;
                            m::vector_normalize(&mut ofsvec);
                            let ofs = ofsvec;
                            // `1.0` is a `double`, so the scale is computed
                            // in `double` and narrows at the call.
                            m::vector_scale(
                                &ofs,
                                (1.0 - f64::from(frandom() * m)) as c_float,
                                &mut ofsvec,
                            );

                            // org is just like the original
                            arsvec[0] = j.wrapping_add(com_rand_mod(spawnspc)) as c_float;
                            arsvec[1] = k.wrapping_add(com_rand_mod(spawnspc)) as c_float;
                            arsvec[2] = l.wrapping_add(com_rand_mod(spawnspc)) as c_float;

                            // advance telebox loop
                            j = j.wrapping_add(spawnspc);
                            if j as c_float >= (*ptype).areaspread {
                                j = as_int(-(*ptype).areaspread);
                                k = k.wrapping_add(spawnspc);
                                if k as c_float >= (*ptype).areaspread {
                                    k = as_int(-(*ptype).areaspread);
                                    l = l.wrapping_add(spawnspc);
                                    if l as c_float >= (*ptype).areaspreadvert {
                                        l = as_int(-(*ptype).areaspreadvert);
                                    }
                                }
                            }
                        }
                        SM_LAVASPLASH => {
                            // calc directions, org with temp vector
                            ofsvec[0] = k.wrapping_add(com_rand_mod(spawnspc)) as c_float;
                            ofsvec[1] = j.wrapping_add(com_rand_mod(spawnspc)) as c_float;
                            ofsvec[2] = 256.0;

                            arsvec[0] = ofsvec[0];
                            arsvec[1] = ofsvec[1];
                            arsvec[2] = frandom() * (*ptype).areaspreadvert;

                            m::vector_normalize(&mut ofsvec);
                            let ofs = ofsvec;
                            m::vector_scale(
                                &ofs,
                                (1.0 - f64::from(frandom() * m)) as c_float,
                                &mut ofsvec,
                            );

                            // advance splash loop
                            j = j.wrapping_add(spawnspc);
                            if j as c_float >= (*ptype).areaspread {
                                j = as_int(-(*ptype).areaspread);
                                k = k.wrapping_add(spawnspc);
                                if k as c_float >= (*ptype).areaspread {
                                    k = as_int(-(*ptype).areaspread);
                                }
                            }
                        }
                        SM_UNICIRCLE => {
                            ofsvec[0] = pcos(m * i as c_float);
                            ofsvec[1] = psin(m * i as c_float);
                            ofsvec[2] = 0.0;
                            let ofs = ofsvec;
                            m::vector_scale(&ofs, (*ptype).areaspread, &mut arsvec);
                        }
                        SM_FIELD => {
                            // COMPAT: ADR-004 -- `r_part_fte.c:4740` indexes
                            // `avelocities` (162 entries, `:3867`) with the
                            // spawn-loop counter `i`, bounded only by
                            // `pcount` (`:4587`), so a particle script with a
                            // `count` above 162 makes C read adjacent static
                            // storage -- an out-of-bounds read, undefined in
                            // C (C99 6.5.6p8) and with no defined value to be
                            // faithful to. Rust reads zeroes there instead of
                            // panicking (`panic = "abort"` would kill the
                            // process). This is an **observable divergence**
                            // for such a config, recorded rather than tested:
                            // the differential does not drive `count > 162`.
                            //
                            // `r_avertexnormals[j]` below needs no exception:
                            // C wraps `j` itself at `r_part_fte.c:4755-4759`,
                            // so the `% NUMVERTEXNORMALS` is a faithful
                            // no-op, kept only to carry the bound in the type
                            // system.
                            let av = if (i as usize) < NUMVERTEXNORMALS {
                                ptr::addr_of!(AVELOCITIES)
                                    .cast::<[c_float; 2]>()
                                    .add(i as usize)
                                    .read()
                            } else {
                                [0.0, 0.0]
                            };
                            arsvec[0] = ((cl.time * f64::from(av[0])) + f64::from(m)) as c_float;
                            arsvec[1] = ((cl.time * f64::from(av[1])) + f64::from(m)) as c_float;
                            arsvec[2] = pcos(arsvec[1]);

                            ofsvec[0] = arsvec[2] * pcos(arsvec[0]);
                            ofsvec[1] = arsvec[2] * psin(arsvec[0]);
                            ofsvec[2] = -psin(arsvec[1]);

                            let orgadd = (*ptype).spawnparam2
                                * psin_f64(cl.time + f64::from(j) + f64::from(m));
                            let jj = (j as usize) % NUMVERTEXNORMALS;
                            arsvec[0] = R_AVERTEXNORMALS[jj][0] * ((*ptype).areaspread + orgadd)
                                + ofsvec[0] * (*ptype).spawnparam1;
                            arsvec[1] = R_AVERTEXNORMALS[jj][1] * ((*ptype).areaspread + orgadd)
                                + ofsvec[1] * (*ptype).spawnparam1;
                            arsvec[2] = R_AVERTEXNORMALS[jj][2]
                                * ((*ptype).areaspreadvert + orgadd)
                                + ofsvec[2] * (*ptype).spawnparam1;

                            m::vector_normalize(&mut ofsvec);

                            j += 1;
                            if j as usize >= NUMVERTEXNORMALS {
                                j = 0;
                                // some BS number to try to "randomize" things
                                m = (f64::from(m) + 0.1762891) as c_float;
                            }
                        }
                        SM_DISTBALL => {
                            // this is a strange spawntype, which is based on
                            // the fact that crandom()*crandom() provides
                            // something similar to an exponential
                            // probability curve
                            let rdist = (*ptype).spawnparam2
                                - crandom() * (1.0 - (crandom() * (*ptype).spawnparam1));

                            ofsvec[0] = hrandom();
                            ofsvec[1] = hrandom();
                            if (*ptype).areaspreadvert != 0.0 {
                                ofsvec[2] = hrandom();
                            } else {
                                ofsvec[2] = 0.0;
                            }

                            m::vector_normalize(&mut ofsvec);
                            let ofs = ofsvec;
                            m::vector_scale(&ofs, rdist, &mut ofsvec);

                            arsvec[0] = ofsvec[0] * (*ptype).areaspread;
                            arsvec[1] = ofsvec[1] * (*ptype).areaspread;
                            arsvec[2] = ofsvec[2] * (*ptype).areaspreadvert;
                        }
                        // SM_BALL, SM_CIRCLE
                        _ => {
                            ofsvec[0] = hrandom();
                            ofsvec[1] = hrandom();
                            if (*ptype).areaspreadvert != 0.0 {
                                ofsvec[2] = hrandom();
                            } else {
                                ofsvec[2] = 0.0;
                            }

                            m::vector_normalize(&mut ofsvec);
                            if (*ptype).spawnmode != SM_CIRCLE {
                                let ofs = ofsvec;
                                m::vector_scale(&ofs, frandom(), &mut ofsvec);
                            }

                            arsvec[0] = ofsvec[0] * (*ptype).areaspread;
                            arsvec[1] = ofsvec[1] * (*ptype).areaspread;
                            arsvec[2] = ofsvec[2] * (*ptype).areaspreadvert;
                        }
                    }

                    // apply arsvec+ofsvec
                    let orgadd = (*ptype).orgadd + frandom() * (*ptype).randomorgadd;
                    let mut veladd = (*ptype).veladd + frandom() * (*ptype).randomveladd;

                    if !dir.is_null() {
                        veladd *= m::vector_length(&*dir);
                    }
                    let vel = (*p).vel;
                    m::vector_ma(&vel, ofsvec[0] * (*ptype).spawnvel, &axis[0], &mut (*p).vel);
                    let vel = (*p).vel;
                    m::vector_ma(&vel, ofsvec[1] * (*ptype).spawnvel, &axis[1], &mut (*p).vel);
                    let vel = (*p).vel;
                    m::vector_ma(
                        &vel,
                        veladd + ofsvec[2] * (*ptype).spawnvelvert,
                        &axis[2],
                        &mut (*p).vel,
                    );

                    m::vector_ma(&*org, arsvec[0], &axis[0], &mut (*p).org);
                    let o = (*p).org;
                    m::vector_ma(&o, arsvec[1], &axis[1], &mut (*p).org);
                    let o = (*p).org;
                    m::vector_ma(&o, orgadd + arsvec[2], &axis[2], &mut (*p).org);

                    if (*ptype).flags & PT_WORLDSPACERAND != 0 {
                        loop {
                            ofsvec[0] = crand();
                            ofsvec[1] = crand();
                            ofsvec[2] = crand();
                            // crap, but I'm trying to mimic dp
                            if m::dot_product(&ofsvec, &ofsvec) <= 1.0 {
                                break;
                            }
                        }
                        (*p).org[0] += ofsvec[0] * (*ptype).orgwrand[0];
                        (*p).org[1] += ofsvec[1] * (*ptype).orgwrand[1];
                        (*p).org[2] += ofsvec[2] * (*ptype).orgwrand[2];
                        (*p).vel[0] += ofsvec[0] * (*ptype).velwrand[0];
                        (*p).vel[1] += ofsvec[1] * (*ptype).velwrand[1];
                        (*p).vel[2] += ofsvec[2] * (*ptype).velwrand[2];
                        let vel = (*p).vel;
                        m::vector_add(&vel, &(*ptype).velbias, &mut (*p).vel);
                    }
                    let o = (*p).org;
                    m::vector_add(&o, &(*ptype).orgbias, &mut (*p).org);

                    (*p).die = g::particletime + (*ptype).die - (*p).die;

                    (*p).oldorg = (*p).org;

                    i += 1;
                }

                // update beam list
                if (*ptype).looks.type_ == PT_BEAM && !b.is_null() {
                    // update dir for bfirst for certain modes since it will
                    // never get updated
                    if (*ptype).spawnmode == SM_UNICIRCLE {
                        // kinda hackish here, assuming ofsvec contains the
                        // point at i-1
                        arsvec[0] = pcos(m * (i - 2) as c_float);
                        arsvec[1] = psin(m * (i - 2) as c_float);
                        arsvec[2] = 0.0;
                        m::vector_subtract(&(*(*b).p).org, &arsvec, &mut (*bfirst).dir);
                        m::vector_normalize(&mut (*bfirst).dir);
                    }

                    (*b).flags |= BS_NODRAW;
                    (*b).next = (*ptype).beams;
                    (*ptype).beams = bfirst;
                }

                // save off emit times in trailstate
                if !ts.is_null() {
                    (*ts).state2 = pcount - i as c_float;
                }

                // maintain run list
                if (*ptype).state & PS_INRUNLIST == 0
                    && (!(*ptype).particles.is_null() || !(*ptype).clippeddecals.is_null())
                {
                    if g::part_run_list.is_null() {
                        (*ptype).nexttorun = g::part_run_list;
                        g::part_run_list = ptype;
                    } else {
                        // insert after, to try to avoid edge-case weirdness
                        (*ptype).nexttorun = (*g::part_run_list).nexttorun;
                        (*g::part_run_list).nexttorun = ptype;
                    }
                    (*ptype).state |= PS_INRUNLIST;
                }
            }

            // skip:

            // go to next associated effect
            if (*ptype).assoc < 0 {
                break 'run;
            }

            // new trailstate
            if !ts.is_null() {
                tsk = ptr::addr_of_mut!((*ts).assoc);
                // if *tsk = NULL get a new one
                if (*tsk).is_null() {
                    ts = p_new_trailstate(tsk);
                    *tsk = ts;
                } else {
                    ts = *tsk;

                    if (*ts).key != tsk {
                        // trailstate was overwritten
                        ts = p_new_trailstate(tsk); // so get a new one
                        *tsk = ts;
                    }
                }
            }

            ptype = g::part_type.add((*ptype).assoc as usize);
        }

        store(out, 0);
        0
    }
}

/// `r_part_fte.c:159` -- `#define PART_VALID(part) ((part) >= 0 && (part) < numparticletypes)`.
///
/// # Safety
/// Reads the glue-owned `numparticletypes`.
#[inline]
unsafe fn part_valid(part: c_int) -> bool {
    // SAFETY: a plain `int` owned by the glue half of `r_part_fte.c`.
    unsafe { part >= 0 && part < g::numparticletypes }
}

/// `r_part_fte.c:4925` -- `PScript_RunParticleEffectTypeString`.
///
/// The C `int` result travels in `out`; the return slot is the ADR-009
/// raise status.
///
/// # Safety
///
/// `org`/`dir` must address `vec3_t`, `name` a NUL-terminated string, and
/// `out` must be NULL or writable.
unsafe fn pscript_run_particle_effect_type_string(
    org: *const Vec3,
    dir: *const Vec3,
    count: c_float,
    name: *const c_char,
    out: *mut c_int,
) -> Raise {
    // SAFETY: pointer contracts per the fn docs.
    unsafe {
        if cvar_value(ptr::addr_of!(g::r_fteparticles)) == 0.0 {
            store(out, 1);
            return 0;
        }
        let type_ = pscript_find_particle_type(name);
        if type_ < 0 {
            store(out, 1);
            return 0;
        }

        pscript_run_particle_effect_state(org, dir, count, type_, ptr::null_mut(), out)
    }
}

/// `r_part_fte.c:4934` -- `PScript_EntParticleTrail`.
///
/// # Safety
///
/// `oldorg` must address a `vec3_t`, `ent` must point inside the live
/// `cl.entities` array, and `name` must be NUL-terminated.
unsafe fn pscript_ent_particle_trail(
    oldorg: *const Vec3,
    ent: *mut Entity,
    name: *const c_char,
    out: *mut c_int,
) -> Raise {
    // SAFETY: pointer contracts per the fn docs. The entity index is the C
    // original's `ent - cl.entities`, recomputed against the authoritative
    // opaque `entity_t` stride.
    unsafe {
        if cvar_value(ptr::addr_of!(g::r_fteparticles)) == 0.0 {
            store(out, 1);
            return 0;
        }

        // COMPAT: ADR-010 -- `cl.time - cl.oldtime` is a `double` subtraction
        // narrowed on assignment into the `float` timeinterval.
        let timeinterval: c_float = (cl.time - cl.oldtime) as c_float;

        let type_ = pscript_find_particle_type(name);
        if type_ < 0 {
            store(out, 1);
            return 0;
        }

        let angles = ptr::addr_of!((*ent).angles).read();
        let mut fwd: Vec3 = [0.0; 3];
        let mut right: Vec3 = [0.0; 3];
        let mut up: Vec3 = [0.0; 3];
        m::angle_vectors(&angles, &mut fwd, &mut right, &mut up);
        let axis: [Vec3; 3] = [fwd, right, up];

        let base = ptr::addr_of!(cl.entities).read().cast::<u8>();
        let dlkey = (ent.cast::<u8>().offset_from(base) as usize
            / core::mem::size_of::<EntityOpaque>()) as c_int;

        store(
            out,
            pscript_particle_trail(
                oldorg,
                ptr::addr_of!((*ent).origin),
                type_,
                timeinterval,
                dlkey,
                axis.as_ptr(),
                ptr::addr_of_mut!((*ent).trailstate).cast::<*mut trailstate_t>(),
            ),
        );
        0
    }
}

/// `r_part_fte.c:4956` -- `PScript_RunParticleEffect`.
///
/// # Safety
///
/// `org`/`dir` must address `vec3_t` and `out` must be NULL or writable.
unsafe fn pscript_run_particle_effect(
    org: *const Vec3,
    dir: *const Vec3,
    color: c_int,
    count: c_int,
    out: *mut c_int,
) -> Raise {
    // SAFETY: pointer contracts per the fn docs; every `part_type` index used
    // here is gated by `part_valid`.
    unsafe {
        if cvar_value(ptr::addr_of!(g::r_fteparticles)) == 0.0 {
            store(out, 0);
            return 0;
        }

        let ptype = pscript_find_particle_type(g::va(c"pe_%i".as_ptr(), color));
        let mut ran: c_int = 0;
        raise!(pscript_run_particle_effect_state(
            org,
            dir,
            count as c_float,
            ptype,
            ptr::null_mut(),
            ptr::addr_of_mut!(ran),
        ));
        if ran != 0 {
            let fallback = if count > 130 && part_valid(PE_SIZE3) {
                PE_SIZE3
            } else if count > 20 && part_valid(PE_SIZE2) {
                PE_SIZE2
            } else if part_valid(PE_DEFAULT) {
                PE_DEFAULT
            } else {
                store(out, 1);
                return 0;
            };

            let t = g::part_type.add(fallback as usize);
            (*t).colorindex = color & !0x7;
            (*t).colorrand = 8;
            return pscript_run_particle_effect_state(
                org,
                dir,
                count as c_float,
                fallback,
                ptr::null_mut(),
                out,
            );
        }

        store(out, 0);
        0
    }
}

/// `r_part_fte.c:4987` -- `PScript_RunParticleWeather`.
///
/// # Safety
///
/// `minb`/`maxb`/`dir` must address `vec3_t` and `efname` must be
/// NUL-terminated.
unsafe fn pscript_run_particle_weather(
    minb: *const Vec3,
    maxb: *const Vec3,
    dir: *const Vec3,
    count: c_float,
    colour: c_int,
    efname: *const c_char,
) -> Raise {
    // SAFETY: pointer contracts per the fn docs; `ptype` is gated by
    // `part_valid` before it indexes `part_type`.
    unsafe {
        let mut org: Vec3 = [0.0; 3];

        let mut ptype = pscript_find_particle_type(g::va(c"te_%s_%i".as_ptr(), efname, colour));
        if !part_valid(ptype) {
            ptype = pscript_find_particle_type(g::va(c"te_%s".as_ptr(), efname));
            if !part_valid(ptype) {
                ptype = PE_DEFAULT;
            }
            if !part_valid(ptype) {
                return 0;
            }
            (*g::part_type.add(ptype as usize)).colorindex = colour;
        }

        let invcount = 1.0f32 / (*g::part_type.add(ptype as usize)).count;
        let count = count * (*g::part_type.add(ptype as usize)).count;

        let mut i: c_int = 0;
        while (i as c_float) < count {
            if g::free_particles.is_null() {
                return 0;
            }

            #[allow(clippy::needless_range_loop)] // org, minb and maxb in lockstep
            for j in 0..3usize {
                let num = c::COM_Rand() as c_float / COM_RAND_MAX;
                org[j] = (*minb)[j] + num * ((*maxb)[j] - (*minb)[j]);
            }

            raise!(pscript_run_particle_effect_state(
                &org,
                dir,
                invcount,
                ptype,
                ptr::null_mut(),
                ptr::null_mut(),
            ));

            i += 1;
        }

        0
    }
}

/// `r_part_fte.c:5022` -- `PScript_ParticleTrailSpawn`.
///
/// COMPAT: ADR-010 -- `tdegree`/`sdegree` are computed from `M_PI`, a
/// `double`, so those expressions run in `f64` and narrow only on assignment,
/// and `ceil` is a real libm call. Every `sin`/`cos` below is
/// `r_part_fte.c:186`'s 128-entry table macro, not libm; see `psin`/`pcos`.
///
/// # Safety
///
/// `startpos`/`end` must address `vec3_t`, `ptype` must be a live particle
/// type, `tsk` must be NULL or a writable `trailstate_t *`, and `dlaxis` must
/// be NULL or address three `vec3_t`.
#[allow(clippy::too_many_lines)]
unsafe fn pscript_particle_trail_spawn(
    startpos: *const Vec3,
    end: *const Vec3,
    ptype: *mut part_type_t,
    timeinterval: c_float,
    mut tsk: *mut *mut trailstate_t,
    dlkey: c_int,
    dlaxis: *const Vec3,
) {
    // SAFETY: pointer contracts per the fn docs. The particle/beam pools are
    // this module own allocations and every pop is guarded by a NULL test,
    // exactly as the C original does.
    unsafe {
        let veladd: c_float = -(*ptype).veladd;
        let mut step: c_float;
        let stop: c_float;
        // COMPAT: ADR-010 -- `2.0 * M_PI / 256` is a `double` constant
        // expression narrowed on assignment. /* MSVC whine */
        let mut tdegree: c_float = (2.0 * core::f64::consts::PI / 256.0) as c_float;
        let mut sdegree: c_float = 0.0;
        #[allow(clippy::needless_late_init)] // assigned by the C original's `if`
        let nrfirst: c_float;
        #[allow(clippy::needless_late_init)] // assigned by the C original's `if`
        let nrlast: c_float;
        let palrgba = ptr::addr_of!(g::d_8to24table).cast::<u8>();

        let mut start: Vec3 = *startpos;

        // eliminate trailstate if flag set
        if (*ptype).flags & PT_NOSTATE != 0 {
            tsk = ptr::null_mut();
        }

        // trailstate allocation/deallocation
        let mut ts: *mut trailstate_t;
        if tsk.is_null() {
            ts = ptr::null_mut();
        } else {
            // if *tsk = NULL get a new one
            if (*tsk).is_null() {
                ts = p_new_trailstate(tsk);
                *tsk = ts;
            } else {
                ts = *tsk;

                if (*ts).key != tsk {
                    // trailstate was overwritten
                    ts = p_new_trailstate(tsk); // so get a new one
                    *tsk = ts;
                }
            }
        }

        pscript_effect_spawned(ptype, &start, dlaxis, dlkey, 1.0);

        if (*ptype).assoc >= 0 {
            if ts.is_null() {
                pscript_particle_trail(
                    &start,
                    end,
                    (*ptype).assoc,
                    timeinterval,
                    dlkey,
                    ptr::null(),
                    ptr::null_mut(),
                );
            } else {
                pscript_particle_trail(
                    &start,
                    end,
                    (*ptype).assoc,
                    timeinterval,
                    dlkey,
                    ptr::null(),
                    ptr::addr_of_mut!((*ts).assoc),
                );
            }
        }

        if cvar_value(ptr::addr_of!(g::r_part_contentswitch)) != 0.0
            && (*ptype).flags & (PT_TRUNDERWATER | PT_TROVERWATER) != 0
            && !cl.worldmodel.is_null()
        {
            let cont = cl_point_contents_mask(startpos.cast::<c_float>().cast_mut());

            if (*ptype).flags & PT_TROVERWATER != 0 && cont & (*ptype).fluidmask != 0 {
                return;
            }
            if (*ptype).flags & PT_TRUNDERWATER != 0 && cont & (*ptype).fluidmask == 0 {
                return;
            }
        }

        // time limit for trails
        if (*ptype).spawntime != 0.0 && !ts.is_null() {
            if (*ts).state1 > g::particletime {
                return; // timelimit still in effect
            }

            (*ts).state1 = g::particletime + (*ptype).spawntime; // record old time
            ts = ptr::null_mut(); // clear trailstate so we do not save length/lastseg
        }

        // random chance for trails
        if (*ptype).spawnchance < frandom() {
            return; // do not spawn but return success
        }

        if (*ptype).die == 0.0 {
            ts = ptr::null_mut();
        }

        let mut vec: Vec3 = [0.0; 3];
        m::vector_subtract(&*end, &start, &mut vec);
        let mut len = m::vector_normalize(&mut vec);

        let mut count: c_float;

        // use ptype step to calc step vector and step size
        if (*ptype).countspacing != 0.0 {
            step = (*ptype).countspacing; // particles per qu
            step /= cvar_value(ptr::addr_of!(g::r_part_density)); // scaled...

            if (*ptype).countextra != 0.0 {
                count = (*ptype).countextra;
                if step > 0.0 {
                    count += len / step;
                }
                step = len / count;
            }
        } else {
            step = (*ptype).count * cvar_value(ptr::addr_of!(g::r_part_density)) * timeinterval;
            step += (*ptype).countextra; // particles per frame
            step += (*ptype).countoverflow;
            // COMPAT: ADR-010 -- C float-to-int conversion is undefined out of
            // range; Rust `as` saturates.
            count = (step as c_int) as c_float;
            // the part that we are forgetting, to add to the next frame...
            (*ptype).countoverflow = step - count;
            if count <= 0.0 {
                return;
            }
            step = len / count; // particles per second
        }

        if (*ptype).flags & PT_AVERAGETRAIL != 0 {
            // mangle len/step to get last point to be at end
            let mut tavg = len / step;
            // COMPAT: ADR-010 -- `ceil` is `double ceil (double)`, so the
            // division runs in `f64` and narrows on assignment.
            tavg = (f64::from(tavg) / libm::ceil(f64::from(tavg))) as c_float;
            step *= tavg;
            len += step;
        }

        let mut vstep: Vec3 = [0.0; 3];
        m::vector_scale(&vec, step, &mut vstep);

        // add offset
        //	VectorAdd(start, ptype->orgbias, start);

        let mut right: Vec3 = [0.0; 3];
        let mut up: Vec3 = [0.0; 3];

        // spawn mode precalculations
        if (*ptype).spawnmode == SM_SPIRAL {
            vector_vectors(&vec, &mut right, &mut up);

            // precalculate degree of rotation
            if (*ptype).spawnparam1 != 0.0 {
                // distance per rotation inversed
                tdegree =
                    (2.0 * core::f64::consts::PI / f64::from((*ptype).spawnparam1)) as c_float;
            }
            sdegree =
                (f64::from((*ptype).spawnparam2) * (core::f64::consts::PI / 180.0)) as c_float;
        } else if (*ptype).spawnmode == SM_CIRCLE {
            vector_vectors(&vec, &mut right, &mut up);
        }

        // store last stop here for lack of a better solution besides vectors
        if ts.is_null() {
            stop = len;
            len = 0.0;
        } else {
            (*ts).state2 += len; // when to stop
            stop = (*ts).state2;
            len = (*ts).state1;
        }

        //	len = ts->lastdist/step;
        //	len = (len - (int)len)*step;
        //	VectorMA (start, -len, vec, start);

        if (*ptype).flags & PT_NOSPREADFIRST != 0 {
            nrfirst = len + step * 1.5;
        } else {
            nrfirst = len;
        }

        if (*ptype).flags & PT_NOSPREADLAST != 0 {
            nrlast = stop;
        } else {
            nrlast = stop + step;
        }

        let mut b: *mut beamseg_t = ptr::null_mut();
        let mut bfirst: *mut beamseg_t = ptr::null_mut();

        if len < stop {
            count = (stop - len) / step;
        } else {
            count = 0.0;
            step = 0.0;
            vstep = [0.0; 3];
        }
        //	count += ptype->countextra;

        while count > 0.0 {
            count -= 1.0;

            len += step;

            if g::free_particles.is_null() {
                len = stop;
                break;
            }

            let p = g::free_particles;
            if (*ptype).looks.type_ == PT_BEAM {
                if g::free_beams.is_null() {
                    len = stop;
                    break;
                }
                if b.is_null() {
                    b = g::free_beams;
                    bfirst = g::free_beams;
                    g::free_beams = (*g::free_beams).next;
                } else {
                    (*b).next = g::free_beams;
                    b = g::free_beams;
                    g::free_beams = (*g::free_beams).next;
                }
                (*b).texture_s = len; // not sure how to calc this
                (*b).flags = 0;
                (*b).p = p;
                (*b).dir = vec;
            }

            g::free_particles = (*p).next;
            (*p).next = (*ptype).particles;
            (*ptype).particles = p;

            (*p).die = (*ptype).randdie * frandom();
            (*p).scale = (*ptype).scale + (*ptype).scalerand * frandom();
            if (*ptype).die != 0.0 {
                (*p).rgba[3] = (*ptype).alpha + (*p).die * (*ptype).alphachange;
            } else {
                (*p).rgba[3] = (*ptype).alpha;
            }
            (*p).rgba[3] += (*ptype).alpharand * frandom();
            //		p->color = 0;

            //		if (ptype->spawnmode == SM_TRACER)
            // COMPAT: ADR-010 -- C float-to-int conversion is undefined out of
            // range; Rust `as` saturates.
            let tcount: c_int = if (*ptype).spawnparam1 != 0.0 {
                (len * (*ptype).count / (*ptype).spawnparam1) as c_int
            } else {
                (len * (*ptype).count) as c_int
            };

            if (*ptype).colorindex >= 0 {
                let mut cidx: c_int = if (*ptype).colorrand > 0 {
                    c::COM_Rand().wrapping_rem((*ptype).colorrand)
                } else {
                    0
                };
                if (*ptype).flags & PT_CITRACER != 0 {
                    // colorindex behavior as per tracers in std Q1
                    cidx += (tcount & 4) << 1;
                }

                cidx += (*ptype).colorindex;
                if cidx > 255 {
                    (*p).rgba[3] /= 2.0;
                }
                cidx = (cidx & 0xff) * 4;
                // COMPAT: ADR-010 -- `1 / 255.0` is a `double`, so each channel
                // is scaled in `f64` and narrows on assignment.
                (*p).rgba[0] =
                    (f64::from(palrgba.offset(cidx as isize).read()) * (1.0 / 255.0)) as c_float;
                (*p).rgba[1] = (f64::from(palrgba.offset(cidx as isize + 1).read()) * (1.0 / 255.0))
                    as c_float;
                (*p).rgba[2] = (f64::from(palrgba.offset(cidx as isize + 2).read()) * (1.0 / 255.0))
                    as c_float;
            } else {
                (*p).rgba[0] = (*ptype).rgb[0];
                (*p).rgba[1] = (*ptype).rgb[1];
                (*p).rgba[2] = (*ptype).rgb[2];
            }

            // use org temporarily for rgbsync
            (*p).org[2] = frandom();
            (*p).org[0] =
                (*p).org[2] * (*ptype).rgbrandsync[0] + frandom() * (1.0 - (*ptype).rgbrandsync[0]);
            (*p).org[1] =
                (*p).org[2] * (*ptype).rgbrandsync[1] + frandom() * (1.0 - (*ptype).rgbrandsync[1]);
            (*p).org[2] =
                (*p).org[2] * (*ptype).rgbrandsync[2] + frandom() * (1.0 - (*ptype).rgbrandsync[2]);

            (*p).rgba[0] += (*p).org[0] * (*ptype).rgbrand[0] + (*ptype).rgbchange[0] * (*p).die;
            (*p).rgba[1] += (*p).org[1] * (*ptype).rgbrand[1] + (*ptype).rgbchange[1] * (*p).die;
            (*p).rgba[2] += (*p).org[2] * (*ptype).rgbrand[2] + (*ptype).rgbchange[2] * (*p).die;

            (*p).vel = [0.0; 3];
            if (*ptype).emittime < 0.0 {
                (*p).state.trailstate = ptr::null_mut(); // init trailstate
            } else {
                (*p).state.nextemit = g::particletime + (*ptype).emitstart - (*p).die;
            }

            (*p).rotationspeed = (*ptype).rotationmin + frandom() * (*ptype).rotationrand;
            (*p).angle = (*ptype).rotationstartmin + frandom() * (*ptype).rotationstartrand;
            (*p).s1 = (*ptype).s1;
            (*p).t1 = (*ptype).t1;
            (*p).s2 = (*ptype).s2;
            (*p).t2 = (*ptype).t2;
            if (*ptype).randsmax != 1 {
                let offs = (*ptype).texsstride * com_rand_mod((*ptype).randsmax) as c_float;
                (*p).s1 += offs;
                (*p).s2 += offs;
                while (*p).s1 >= 1.0 {
                    (*p).s1 -= 1.0;
                    (*p).s2 -= 1.0;
                    (*p).t1 += (*ptype).texsstride;
                    (*p).t2 += (*ptype).texsstride;
                }
            }

            if len < nrfirst || len >= nrlast {
                // no offset or areaspread for these particles...
                (*p).vel[0] = vec[0] * veladd;
                (*p).vel[1] = vec[1] * veladd;
                (*p).vel[2] = vec[2] * veladd;

                (*p).org = start;
            } else {
                match (*ptype).spawnmode {
                    SM_TRACER => {
                        if tcount & 1 != 0 {
                            (*p).vel[0] = vec[1] * (*ptype).spawnvel;
                            (*p).vel[1] = -vec[0] * (*ptype).spawnvel;
                            (*p).org[0] = vec[1] * (*ptype).areaspread;
                            (*p).org[1] = -vec[0] * (*ptype).areaspread;
                        } else {
                            (*p).vel[0] = -vec[1] * (*ptype).spawnvel;
                            (*p).vel[1] = vec[0] * (*ptype).spawnvel;
                            (*p).org[0] = -vec[1] * (*ptype).areaspread;
                            (*p).org[1] = vec[0] * (*ptype).areaspread;
                        }

                        (*p).vel[0] += vec[0] * veladd;
                        (*p).vel[1] += vec[1] * veladd;
                        (*p).vel[2] = vec[2] * veladd;

                        (*p).org[0] += start[0];
                        (*p).org[1] += start[1];
                        (*p).org[2] = start[2];
                    }
                    SM_SPIRAL => {
                        let tcos = pcos(len * tdegree + sdegree);
                        let tsin = psin(len * tdegree + sdegree);

                        let mut tright = tcos * (*ptype).areaspread;
                        let mut tup = tsin * (*ptype).areaspread;

                        (*p).org[0] = start[0] + right[0] * tright + up[0] * tup;
                        (*p).org[1] = start[1] + right[1] * tright + up[1] * tup;
                        (*p).org[2] = start[2] + right[2] * tright + up[2] * tup;

                        tright = tcos * (*ptype).spawnvel;
                        tup = tsin * (*ptype).spawnvel;

                        (*p).vel[0] = vec[0] * veladd + right[0] * tright + up[0] * tup;
                        (*p).vel[1] = vec[1] * veladd + right[1] * tright + up[1] * tup;
                        (*p).vel[2] = vec[2] * veladd + right[2] * tright + up[2] * tup;
                    }
                    // TODO: directionalize SM_BALL/SM_CIRCLE/SM_DISTBALL
                    SM_BALL => {
                        (*p).org[0] = crandom();
                        (*p).org[1] = crandom();
                        (*p).org[2] = crandom();
                        let mut o = (*p).org;
                        m::vector_normalize(&mut o);
                        let oin = o;
                        m::vector_scale(&oin, frandom(), &mut o);
                        (*p).org = o;

                        (*p).vel[0] = vec[0] * veladd + (*p).org[0] * (*ptype).spawnvel;
                        (*p).vel[1] = vec[1] * veladd + (*p).org[1] * (*ptype).spawnvel;
                        (*p).vel[2] = vec[2] * veladd + (*p).org[2] * (*ptype).spawnvelvert;

                        (*p).org[0] = (*p).org[0] * (*ptype).areaspread + start[0];
                        (*p).org[1] = (*p).org[1] * (*ptype).areaspread + start[1];
                        (*p).org[2] = (*p).org[2] * (*ptype).areaspreadvert + start[2];
                    }
                    SM_CIRCLE => {
                        let mut tcos = pcos(len * tdegree) * (*ptype).areaspread;
                        let mut tsin = psin(len * tdegree) * (*ptype).areaspread;

                        (*p).org[0] =
                            start[0] + right[0] * tcos + up[0] * tsin + vstep[0] * (len * tdegree);
                        (*p).org[1] =
                            start[1] + right[1] * tcos + up[1] * tsin + vstep[1] * (len * tdegree);
                        (*p).org[2] = start[2]
                            + right[2] * tcos
                            + up[2] * tsin
                            + vstep[2] * (len * tdegree) * 50.0;

                        tcos = pcos(len * tdegree) * (*ptype).spawnvel;
                        tsin = psin(len * tdegree) * (*ptype).spawnvel;

                        (*p).vel[0] = vec[0] * veladd + right[0] * tcos + up[0] * tsin;
                        (*p).vel[1] = vec[1] * veladd + right[1] * tcos + up[1] * tsin;
                        (*p).vel[2] = vec[2] * veladd + right[2] * tcos + up[2] * tsin;
                    }
                    SM_DISTBALL => {
                        let rdist = (*ptype).spawnparam2
                            - crandom() * (1.0 - (crandom() * (*ptype).spawnparam1));

                        // this is a strange spawntype, which is based on the fact that
                        // crandom()*crandom() provides something similar to an exponential
                        // probability curve
                        (*p).org[0] = crandom();
                        (*p).org[1] = crandom();
                        (*p).org[2] = crandom();

                        let mut o = (*p).org;
                        m::vector_normalize(&mut o);
                        let oin = o;
                        m::vector_scale(&oin, rdist, &mut o);
                        (*p).org = o;

                        (*p).vel[0] = vec[0] * veladd + (*p).org[0] * (*ptype).spawnvel;
                        (*p).vel[1] = vec[1] * veladd + (*p).org[1] * (*ptype).spawnvel;
                        (*p).vel[2] = vec[2] * veladd + (*p).org[2] * (*ptype).spawnvelvert;

                        (*p).org[0] = (*p).org[0] * (*ptype).areaspread + start[0];
                        (*p).org[1] = (*p).org[1] * (*ptype).areaspread + start[1];
                        (*p).org[2] = (*p).org[2] * (*ptype).areaspreadvert + start[2];
                    }
                    _ => {
                        (*p).org[0] = crandom();
                        (*p).org[1] = crandom();
                        (*p).org[2] = crandom();

                        (*p).vel[0] = vec[0] * veladd + (*p).org[0] * (*ptype).spawnvel;
                        (*p).vel[1] = vec[1] * veladd + (*p).org[1] * (*ptype).spawnvel;
                        (*p).vel[2] = vec[2] * veladd + (*p).org[2] * (*ptype).spawnvelvert;

                        (*p).org[0] = (*p).org[0] * (*ptype).areaspread + start[0];
                        (*p).org[1] = (*p).org[1] * (*ptype).areaspread + start[1];
                        (*p).org[2] = (*p).org[2] * (*ptype).areaspreadvert + start[2];
                    }
                }

                if (*ptype).orgadd != 0.0 {
                    (*p).org[0] += vec[0] * (*ptype).orgadd;
                    (*p).org[1] += vec[1] * (*ptype).orgadd;
                    (*p).org[2] += vec[2] * (*ptype).orgadd;
                }
            }

            if (*ptype).flags & PT_WORLDSPACERAND != 0 {
                let mut vtmp: Vec3;
                loop {
                    vtmp = [crand(), crand(), crand()];
                    // crap, but I am trying to mimic dp
                    if m::dot_product(&vtmp, &vtmp) <= 1.0 {
                        break;
                    }
                }
                (*p).org[0] += vtmp[0] * (*ptype).orgwrand[0];
                (*p).org[1] += vtmp[1] * (*ptype).orgwrand[1];
                (*p).org[2] += vtmp[2] * (*ptype).orgwrand[2];
                (*p).vel[0] += vtmp[0] * (*ptype).velwrand[0];
                (*p).vel[1] += vtmp[1] * (*ptype).velwrand[1];
                (*p).vel[2] += vtmp[2] * (*ptype).velwrand[2];
                let vin = (*p).vel;
                m::vector_add(&vin, &(*ptype).velbias, &mut (*p).vel);
            }
            let oin = (*p).org;
            m::vector_add(&oin, &(*ptype).orgbias, &mut (*p).org);

            let sin_ = start;
            m::vector_add(&sin_, &vstep, &mut start);

            if (*ptype).countrand != 0.0 {
                let rstep = frandom() / (*ptype).countrand;
                let sin2 = start;
                m::vector_ma(&sin2, rstep, &vec, &mut start);
                step += rstep;
            }

            (*p).die = g::particletime + (*ptype).die - (*p).die;
            (*p).oldorg = (*p).org;
        }

        if ts.is_null() {
            if (*ptype).looks.type_ == PT_BEAM && !b.is_null() {
                (*b).flags |= BS_NODRAW;
                (*b).next = (*ptype).beams;
                (*ptype).beams = bfirst;
            }
        } else {
            (*ts).state1 = len;

            // update beamseg list
            if (*ptype).looks.type_ == PT_BEAM {
                if !b.is_null() {
                    if (*ptype).beams.is_null() {
                        (*ptype).beams = bfirst;
                        (*b).next = ptr::null_mut();
                    } else if (*ts).lastbeam.is_null() {
                        (*b).next = (*ptype).beams;
                        (*ptype).beams = bfirst;
                    } else {
                        (*b).next = (*(*ts).lastbeam).next;
                        (*(*ts).lastbeam).next = bfirst;
                        (*(*ts).lastbeam).flags &= !BS_LASTSEG;
                    }

                    (*b).flags |= BS_LASTSEG;
                    (*ts).lastbeam = b;
                }

                if (g::free_particles.is_null() || g::free_beams.is_null())
                    && !(*ts).lastbeam.is_null()
                {
                    (*(*ts).lastbeam).flags &= !BS_LASTSEG;
                    (*(*ts).lastbeam).flags |= BS_NODRAW;
                    (*ts).lastbeam = ptr::null_mut();
                }
            }
        }

        // maintain run list
        if (*ptype).state & PS_INRUNLIST == 0 {
            (*ptype).nexttorun = g::part_run_list;
            g::part_run_list = ptype;
            (*ptype).state |= PS_INRUNLIST;
        }
    }
}

/// `r_part_fte.c:5552` -- `PScript_ParticleTrail`.
///
/// The C original forms `&part_type[type]` before the range check; the
/// pointer is never dereferenced on the failing paths, so it is computed
/// after the check here (forming an out-of-range pointer is UB in Rust).
///
/// # Safety
///
/// `startpos`/`end` must address `vec3_t`, `axis` must be NULL or address
/// three `vec3_t`, and `tsk` must be NULL or a writable `trailstate_t *`.
unsafe fn pscript_particle_trail(
    startpos: *const Vec3,
    end: *const Vec3,
    type_: c_int,
    timeinterval: c_float,
    dlkey: c_int,
    axis: *const Vec3,
    tsk: *mut *mut trailstate_t,
) -> c_int {
    // SAFETY: pointer contracts per the fn docs; `type_` and `inwater` are
    // both range-checked against `numparticletypes` before indexing
    // `part_type`.
    unsafe {
        if cvar_value(ptr::addr_of!(g::r_fteparticles)) == 0.0 {
            return 1;
        }

        if type_ < 0 || type_ >= g::numparticletypes {
            return 1; // bad value
        }

        let mut ptype = g::part_type.add(type_ as usize);

        if (*ptype).loaded == 0 {
            return 1;
        }

        // inwater check, switch only once
        if cvar_value(ptr::addr_of!(g::r_part_contentswitch)) != 0.0
            && (*ptype).inwater >= 0
            && !cl.worldmodel.is_null()
        {
            let cont = cl_point_contents_mask(startpos.cast::<c_float>().cast_mut());

            if cont & FTECONTENTS_FLUID != 0 {
                ptype = g::part_type.add((*ptype).inwater as usize);
            }
        }

        pscript_particle_trail_spawn(startpos, end, ptype, timeinterval, tsk, dlkey, axis);
        0
    }
}

// ---------------------------------------------------------------------------
// Deferred queues and the parallel particle update (`r_part_fte.c:6295-6784`)
// ---------------------------------------------------------------------------

/// `r_part_fte.c:6295` -- `DEFERRED_PUSH`. The C macro is textual over the
/// element type; here the element type is the generic parameter.
///
/// # Safety
///
/// `array` and `max` must address the two members of one `deferred_queues_t`
/// slot (or the `particle_updates`/`max_particle_updates` pair), and `num`
/// must be their current fill.
#[inline]
unsafe fn deferred_push<T>(array: *mut *mut T, num: c_int, max: *mut c_int) {
    // SAFETY: pointer contracts per the fn docs; `Mem_Realloc` returns storage
    // for `*max` elements or does not return at all.
    unsafe {
        if num == *max {
            *max = q_max_i(256, (*max).wrapping_mul(2));
            *array = c::Mem_Realloc(
                (*array).cast::<c_void>(),
                (*max) as usize * core::mem::size_of::<T>(),
            )
            .cast::<T>();
        }
    }
}

/// `r_part_fte.c:6325` -- `PScript_QueueEffect`.
///
/// # Safety
/// `org` and `dir` must address `vec3_t`.
unsafe fn pscript_queue_effect(org: *const Vec3, dir: *const Vec3, count: c_float, type_: c_int) {
    // SAFETY: `Tasks_GetWorkerIndex` returns an index below `TASKS_MAX_WORKERS`
    // and the queue arrays are grown by `deferred_push` before the write.
    unsafe {
        let queue = ptr::addr_of_mut!(g::deferred_queues[g::Tasks_GetWorkerIndex() as usize]);
        deferred_push(
            ptr::addr_of_mut!((*queue).effects),
            (*queue).num_effects,
            ptr::addr_of_mut!((*queue).max_effects),
        );
        let fx = (*queue).effects.add((*queue).num_effects as usize);
        (*queue).num_effects += 1;
        (*fx).org = *org;
        (*fx).dir = *dir;
        (*fx).count = count;
        (*fx).type_ = type_;
    }
}

/// `r_part_fte.c:6336` -- `PScript_QueueTrail`.
///
/// # Safety
/// `start` and `end` must address `vec3_t`; `tsk` is stored unvalidated, as C
/// does.
unsafe fn pscript_queue_trail(
    start: *const Vec3,
    end: *const Vec3,
    type_: c_int,
    tsk: *mut *mut trailstate_t,
) {
    // SAFETY: as `pscript_queue_effect`.
    unsafe {
        let queue = ptr::addr_of_mut!(g::deferred_queues[g::Tasks_GetWorkerIndex() as usize]);
        deferred_push(
            ptr::addr_of_mut!((*queue).trails),
            (*queue).num_trails,
            ptr::addr_of_mut!((*queue).max_trails),
        );
        let trail = (*queue).trails.add((*queue).num_trails as usize);
        (*queue).num_trails += 1;
        (*trail).start = *start;
        (*trail).end = *end;
        (*trail).type_ = type_;
        (*trail).tsk = tsk;
    }
}

/// `r_part_fte.c:6347` -- `PScript_QueueDecal`.
///
/// # Safety
/// `center` and `normal` must address `vec3_t`; `type_` must be a live
/// particle type.
unsafe fn pscript_queue_decal(
    type_: *mut part_type_t,
    entity: c_int,
    center: *const Vec3,
    normal: *const Vec3,
    scale: c_float,
) {
    // SAFETY: as `pscript_queue_effect`.
    unsafe {
        let queue = ptr::addr_of_mut!(g::deferred_queues[g::Tasks_GetWorkerIndex() as usize]);
        deferred_push(
            ptr::addr_of_mut!((*queue).decals),
            (*queue).num_decals,
            ptr::addr_of_mut!((*queue).max_decals),
        );
        let decal = (*queue).decals.add((*queue).num_decals as usize);
        (*queue).num_decals += 1;
        (*decal).type_ = type_;
        (*decal).entity = entity;
        (*decal).center = *center;
        (*decal).normal = *normal;
        (*decal).scale = scale;
    }
}

/// `r_part_fte.c:6359` -- `PScript_QueueDlight`.
///
/// # Safety
/// `org` and `rgb` must address `vec3_t`.
unsafe fn pscript_queue_dlight(
    key: c_int,
    org: *const Vec3,
    radius: c_float,
    die: c_float,
    decay: c_float,
    rgb: *const Vec3,
) {
    // SAFETY: as `pscript_queue_effect`.
    unsafe {
        let queue = ptr::addr_of_mut!(g::deferred_queues[g::Tasks_GetWorkerIndex() as usize]);
        deferred_push(
            ptr::addr_of_mut!((*queue).dlights),
            (*queue).num_dlights,
            ptr::addr_of_mut!((*queue).max_dlights),
        );
        let dl = (*queue).dlights.add((*queue).num_dlights as usize);
        (*queue).num_dlights += 1;
        (*dl).key = key;
        (*dl).org = *org;
        (*dl).radius = radius;
        (*dl).die = die;
        (*dl).decay = decay;
        (*dl).rgb = *rgb;
    }
}

/// `r_part_fte.c:6383` -- `PScript_FlushDlightsTask`.
///
/// Allocates the dlights queued by the previous frame's deferred effect
/// spawns (which run inside the task graph, where writing `cl_dlights` would
/// race with the tasks reading them). Scheduled at the start of the graph,
/// before anything reads `cl_dlights` and before the next layout task refills
/// the queues.
///
/// # Safety
/// Must run serially, outside the task graph's parallel region.
unsafe fn pscript_flush_dlights_task() {
    // SAFETY: `deferred_queues` is a fixed `TASKS_MAX_WORKERS` array and each
    // `num_dlights` counts entries this module pushed; `CL_AllocDlight` always
    // returns a live slot.
    unsafe {
        for w in 0..TASKS_MAX_WORKERS {
            let queue = ptr::addr_of_mut!(g::deferred_queues[w]);
            for i in 0..(*queue).num_dlights {
                let qdl = (*queue).dlights.add(i as usize);
                // COMPAT: ADR-010 -- `qdl->die < cl.time` compares a `float`
                // against a `double`, so the comparison runs in `f64`.
                if f64::from((*qdl).die) < cl.time {
                    continue;
                }
                let dl = c::cl_tent::CL_AllocDlight((*qdl).key);
                (*dl).origin = (*qdl).org;
                (*dl).radius = (*qdl).radius;
                (*dl).minlight = 0.0;
                (*dl).die = (*qdl).die;
                (*dl).decay = (*qdl).decay;
                (*dl).color = (*qdl).rgb;
            }
            (*queue).num_dlights = 0;
        }
    }
}

/// `r_part_fte.c:6402` -- `P_UpdateRand`. A small local RNG so the parallel
/// update does not contend on (or require thread safety of) `COM_Rand`.
#[inline]
fn p_update_rand(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

/// `r_part_fte.c:6411` -- `ufrandom`.
#[inline]
fn ufrandom(rng: &mut u32) -> c_float {
    p_update_rand(rng) as c_float * (1.0f32 / 4294967296.0f32)
}

/// `r_part_fte.c:6412` -- `ucrandom`.
#[inline]
fn ucrandom(rng: &mut u32) -> c_float {
    p_update_rand(rng) as c_float * (2.0f32 / 4294967296.0f32) - 1.0f32
}

/// `r_part_fte.c:6440` -- `PScript_UpdateParticle`.
///
/// Advances a single live particle: physics, color/scale ramps, emission and
/// BSP collision. Only mutates the particle itself, everything else goes
/// through the deferred queues.
///
/// # Safety
///
/// `p` must be a live particle of `type_`, and this must run on a task worker
/// (or serially), never concurrently with a writer of the same particle.
#[allow(clippy::too_many_lines)]
unsafe fn pscript_update_particle(
    p: *mut particle_t,
    type_: *mut part_type_t,
    pframetime: c_float,
    doflurry: bool,
    rng: &mut u32,
) {
    // SAFETY: pointer contracts per the fn docs. `type->ramp` holds
    // `rampindexes` entries and every index below is clamped to that, and
    // `type->cliptype` is a validated particle-type index.
    unsafe {
        let grav: c_float = (*type_).gravity * pframetime;
        let friction: Vec3 = [
            1.0 - (*type_).friction[0] * pframetime,
            1.0 - (*type_).friction[1] * pframetime,
            1.0 - (*type_).friction[2] * pframetime,
        ];

        let oldorg: Vec3 = (*p).org;
        if (*type_).flags & PT_VELOCITY != 0 {
            (*p).org[0] += (*p).vel[0] * pframetime;
            (*p).org[1] += (*p).vel[1] * pframetime;
            (*p).org[2] += (*p).vel[2] * pframetime;
            (*p).vel[2] -= grav;
            if (*type_).flags & PT_FRICTION != 0 {
                (*p).vel[0] *= friction[0];
                (*p).vel[1] *= friction[1];
                (*p).vel[2] *= friction[2];
            }
            if (*type_).flurry != 0.0 && doflurry {
                // these should probably be partially synced,
                (*p).vel[0] += ucrandom(rng) * (*type_).flurry;
                (*p).vel[1] += ucrandom(rng) * (*type_).flurry;
            }
        }

        (*p).angle += (*p).rotationspeed * pframetime;

        match (*type_).rampmode {
            RAMP_NEAREST => {
                // COMPAT: ADR-010 -- C float-to-int conversion is undefined
                // out of range; Rust `as` saturates.
                let mut rampind = (((*type_).rampindexes as c_float
                    * ((*type_).die - ((*p).die - g::particletime)))
                    / (*type_).die) as c_int;
                if rampind >= (*type_).rampindexes {
                    rampind = (*type_).rampindexes - 1;
                }
                let ramp = (*type_).ramp.offset(rampind as isize);
                (*p).rgba[0] = (*ramp).rgb[0];
                (*p).rgba[1] = (*ramp).rgb[1];
                (*p).rgba[2] = (*ramp).rgb[2];
                (*p).rgba[3] = (*ramp).alpha;
                (*p).scale = (*ramp).scale;
            }
            RAMP_LERP => {
                let mut frac = ((*type_).rampindexes as c_float
                    * ((*type_).die - ((*p).die - g::particletime)))
                    / (*type_).die;
                // COMPAT: ADR-010 -- as above.
                let mut s1 = frac as c_int;
                let mut s2 = s1 + 1;
                if s1 > (*type_).rampindexes - 1 {
                    s1 = (*type_).rampindexes - 1;
                }
                if s2 > (*type_).rampindexes - 1 {
                    s2 = (*type_).rampindexes - 1;
                }
                frac -= s1 as c_float;
                let a = (*type_).ramp.offset(s1 as isize);
                let b = (*type_).ramp.offset(s2 as isize);
                for i in 0..3 {
                    (*p).rgba[i] = (*a).rgb[i] + ((*b).rgb[i] - (*a).rgb[i]) * frac;
                }
                (*p).rgba[3] = (*a).alpha + ((*b).alpha - (*a).alpha) * frac;
                (*p).scale = (*a).scale + ((*b).scale - (*a).scale) * frac;
            }
            RAMP_DELTA => {
                // particle ramps
                // COMPAT: ADR-010 -- as above.
                let mut rampind = (((*type_).rampindexes as c_float
                    * ((*type_).die - ((*p).die - g::particletime)))
                    / (*type_).die) as c_int;
                if rampind >= (*type_).rampindexes {
                    rampind = (*type_).rampindexes - 1;
                }
                let ramp = (*type_).ramp.offset(rampind as isize);
                for i in 0..3 {
                    (*p).rgba[i] += pframetime * (*ramp).rgb[i];
                }
                (*p).rgba[3] -= pframetime * (*ramp).alpha;
                (*p).scale += pframetime * (*ramp).scale;
            }
            // particle changes acording to it's preset properties.
            _ => {
                if g::particletime < ((*p).die - (*type_).die + (*type_).rgbchangetime) {
                    (*p).rgba[0] += pframetime * (*type_).rgbchange[0];
                    (*p).rgba[1] += pframetime * (*type_).rgbchange[1];
                    (*p).rgba[2] += pframetime * (*type_).rgbchange[2];
                }
                (*p).rgba[3] += pframetime * (*type_).alphachange;
                (*p).scale += pframetime * (*type_).scaledelta;
            }
        }

        if (*type_).emit >= 0 {
            if (*type_).emittime < 0.0 {
                pscript_queue_trail(
                    &oldorg,
                    ptr::addr_of!((*p).org),
                    (*type_).emit,
                    ptr::addr_of_mut!((*p).state.trailstate),
                );
            } else if (*p).state.nextemit < g::particletime {
                (*p).state.nextemit =
                    g::particletime + (*type_).emittime + ufrandom(rng) * (*type_).emitrand;
                pscript_queue_effect(
                    ptr::addr_of!((*p).org),
                    ptr::addr_of!((*p).vel),
                    1.0,
                    (*type_).emit,
                );
            }
        }

        if (*type_).cliptype >= 0 && cvar_value(ptr::addr_of!(g::r_bouncysparks)) != 0.0 {
            let mut stop: Vec3 = [0.0; 3];
            m::vector_subtract(&(*p).org, &(*p).oldorg, &mut stop);
            if (*type_).clipbounce == 0.0 || m::dot_product(&stop, &stop) > 10.0 * 10.0 {
                let mut normal: Vec3 = [0.0; 3];
                let mut e: c_int = 0;
                if g::FtePart_Glue_AtomicIncrementU32(
                    ptr::addr_of_mut!(g::particle_traces_used).cast::<c_void>(),
                ) < PARTICLE_TRACE_LIMIT
                    && quake_rs_ftepart_trace_line(
                        ptr::addr_of_mut!((*p).oldorg).cast::<c_float>(),
                        ptr::addr_of_mut!((*p).org).cast::<c_float>(),
                        stop.as_mut_ptr(),
                        normal.as_mut_ptr(),
                        ptr::addr_of_mut!(e),
                    ) < 1.0
                {
                    if (*type_).clipbounce < 0.0 {
                        (*p).die = -1.0;
                        if (*type_).clipbounce == -2.0 {
                            // this type of particle splatters itself as a decal when it hits a wall.
                            pscript_queue_decal(
                                type_,
                                e,
                                ptr::addr_of!((*p).org),
                                &normal,
                                (*p).scale,
                            );
                        }
                        return;
                    } else if g::part_type.offset((*type_).cliptype as isize) == type_ {
                        // bounce
                        // * (-1-(rand()/(float)0x7fff)/2);
                        let mut dist = m::dot_product(&(*p).vel, &normal);
                        dist *= -(*type_).clipbounce;
                        let vin = (*p).vel;
                        m::vector_ma(&vin, dist, &normal, &mut (*p).vel);
                        (*p).org = stop;

                        if (*type_).texname[0] == 0
                            && m::vector_length(&(*p).vel) < 1000.0 * pframetime
                            && (*type_).looks.type_ == PT_NORMAL
                        {
                            (*p).die = -1.0;
                            return;
                        }
                    } else {
                        (*p).die = -1.0;
                        m::vector_normalize(&mut (*p).vel);

                        if (*type_).clipbounce != 0.0 {
                            let nin = normal;
                            m::vector_scale(&nin, (*type_).clipbounce, &mut normal);
                            pscript_queue_effect(
                                &stop,
                                &normal,
                                (*type_).clipcount
                                    / (*g::part_type.offset((*type_).cliptype as isize)).count,
                                (*type_).cliptype,
                            );
                        } else {
                            pscript_queue_effect(
                                &stop,
                                ptr::addr_of!((*p).vel),
                                (*type_).clipcount
                                    / (*g::part_type.offset((*type_).cliptype as isize)).count,
                                (*type_).cliptype,
                            );
                        }
                        return;
                    }
                }
                (*p).oldorg = (*p).org;
            }
        }
    }
}

/// `q_min` over `int`.
#[inline]
fn q_min_i(a: c_int, b: c_int) -> c_int {
    if a < b {
        a
    } else {
        b
    }
}

/// `r_part_fte.c:6596` -- `PScript_UpdateParticlesSetupTask`.
///
/// Serial preparation for the particle update: advances the frame time,
/// spawns rain, unlinks expired particles and flattens the live ones into the
/// update array.
///
/// # Safety
/// Must run serially, outside the task graph's parallel region.
#[allow(clippy::too_many_lines)]
// The `p_frametime` clamp is the C original's two `if`s, kept literal.
#[allow(clippy::manual_clamp)]
unsafe fn pscript_update_particles_setup_task() -> Raise {
    /// `r_part_fte.c:6598` -- `static float oldtime`.
    static mut OLDTIME: c_float = 0.0;
    /// `r_part_fte.c:6599` -- `static float flurrytime`.
    static mut FLURRYTIME: c_float = 0.0;

    // SAFETY: every pool walked here is this module's own allocation and every
    // `part_type` index is derived from the run list, which only ever holds
    // live types.
    unsafe {
        // COMPAT: ADR-010 -- `cl.time - oldtime` is a `double` subtraction
        // narrowed on assignment into the `float` `p_frametime`.
        g::p_frametime = (cl.time - f64::from(OLDTIME)) as c_float;
        if g::p_frametime < 0.0 {
            g::p_frametime = 0.0;
        }
        if g::p_frametime > 1.0 {
            g::p_frametime = 1.0;
        }
        OLDTIME = cl.time as c_float;

        g::num_particle_updates = 0;
        g::p_kill_list = ptr::null_mut();
        g::p_kill_first = ptr::null_mut();

        if cvar_value(ptr::addr_of!(c::r_part::r_particles)) == 0.0 {
            return 0;
        }

        if R_PLOOKSDIRTY {
            PE_DEFAULT = pscript_find_particle_type(c"PE_DEFAULT".as_ptr());
            PE_SIZE2 = pscript_find_particle_type(c"PE_SIZE2".as_ptr());
            PE_SIZE3 = pscript_find_particle_type(c"PE_SIZE3".as_ptr());
            PE_DEFAULTTRAIL = pscript_find_particle_type(c"PE_DEFAULTTRAIL".as_ptr());

            let looks_size = core::mem::size_of::<plooks_t>();
            for j in 0..g::numparticletypes {
                let tj = g::part_type.offset(j as isize);
                // set the fallback
                (*tj).slooks = ptr::addr_of_mut!((*tj).looks);
                let mut k: c_int = j - 1;
                while {
                    let cur = k;
                    k -= 1;
                    cur > 0
                } {
                    let tk = g::part_type.offset(k as isize);
                    let a = core::slice::from_raw_parts(
                        ptr::addr_of!((*tj).looks).cast::<u8>(),
                        looks_size,
                    );
                    let b = core::slice::from_raw_parts(
                        ptr::addr_of!((*tk).looks).cast::<u8>(),
                        looks_size,
                    );
                    if a == b {
                        (*tj).slooks = (*tk).slooks;
                        break;
                    }
                }
            }
            R_PLOOKSDIRTY = false;
            raise!(g::FtePart_Glue_RegisterParticles());
            pscript_recalculate_sky_tris();
        }

        m::vector_scale(
            &ptr::addr_of!(c::host::vup).read(),
            1.5,
            &mut *ptr::addr_of_mut!(g::pup),
        );
        m::vector_scale(
            &ptr::addr_of!(c::host::vright).read(),
            1.5,
            &mut *ptr::addr_of_mut!(g::pright),
        );

        FLURRYTIME -= g::p_frametime;
        if FLURRYTIME < 0.0 {
            P_DOFLURRY = true;
            FLURRYTIME = 0.1 + frandom() * 0.3;
        } else {
            P_DOFLURRY = false;
        }

        if g::free_decals.is_null() {
            // mark some as dead, so we can keep spawning new ones next frame.
            for _ in 0..256 {
                (*DECALS.offset(R_DECALRECYCLE as isize)).die = -1.0;
                R_DECALRECYCLE += 1;
                if R_DECALRECYCLE >= R_NUMDECALS {
                    R_DECALRECYCLE = 0;
                }
            }
        }
        if g::free_particles.is_null() {
            // mark some as dead.
            for _ in 0..256 {
                (*PARTICLES.offset(R_PARTICLERECYCLE as isize)).die = -1.0;
                R_PARTICLERECYCLE += 1;
                if R_PARTICLERECYCLE >= R_NUMPARTICLES {
                    R_PARTICLERECYCLE = 0;
                }
            }
        }

        if cvar_value(ptr::addr_of!(g::r_part_rain)) != 0.0
            && cvar_value(ptr::addr_of!(g::r_fteparticles)) != 0.0
        {
            for j in 0..cl.num_entities {
                let ent = cl_entity(j);
                if (*ent).model.is_null() || (*(*ent).model).needload {
                    continue;
                }
                if (*(*ent).model).skytris.is_null() {
                    continue;
                }
                let angles = ptr::addr_of!((*ent).angles).read();
                let mut fwd: Vec3 = [0.0; 3];
                let mut right: Vec3 = [0.0; 3];
                let mut up: Vec3 = [0.0; 3];
                m::angle_vectors(&angles, &mut fwd, &mut right, &mut up);
                let axis: [Vec3; 3] = [fwd, right, up];
                // this timer, as well as the per-tri timer, are unable to deal with certain
                // rates+sizes. it would be good to fix that... it would also be nice to do
                // mdls too...
                let eorg = ptr::addr_of!((*ent).origin).read();
                raise!(p_add_rain_particles(
                    (*ent).model,
                    axis.as_ptr(),
                    &eorg,
                    g::p_frametime,
                ));
            }
        }

        if g::num_type_emit_meta != g::numparticletypes {
            g::type_emit_meta = c::Mem_Realloc(
                g::type_emit_meta.cast::<c_void>(),
                core::mem::size_of::<particle_emit_meta_t>() * g::numparticletypes as usize,
            )
            .cast::<particle_emit_meta_t>();
            g::num_type_emit_meta = g::numparticletypes;
        }
        g::memset(
            g::type_emit_meta.cast::<c_void>(),
            0,
            core::mem::size_of::<particle_emit_meta_t>() * g::numparticletypes as usize,
        );

        // walk the lists once to unlink expired particles and flatten the live ones, so the
        // update runs over a plain array without touching any list structure
        let mut type_ = g::part_run_list;
        while !type_.is_null() {
            let meta = g::type_emit_meta.offset(type_.offset_from(g::part_type));
            (*meta).start = g::num_particle_updates;

            if (*type_).die == 0.0 {
                // types without a lifetime are drained during drawing
                type_ = (*type_).nexttorun;
                continue;
            }

            loop {
                let kill = (*type_).particles;
                if !kill.is_null() && (*kill).die < g::particletime {
                    if (*type_).emittime < 0.0 {
                        pscript_delink_trailstate(ptr::addr_of_mut!((*kill).state.trailstate));
                    }
                    (*type_).particles = (*kill).next;
                    (*kill).next = g::p_kill_list;
                    g::p_kill_list = kill;
                    if g::p_kill_first.is_null() {
                        g::p_kill_first = kill;
                    }
                    continue;
                }
                break;
            }
            let mut p = (*type_).particles;
            while !p.is_null() {
                loop {
                    let kill = (*p).next;
                    if !kill.is_null() && (*kill).die < g::particletime {
                        if (*type_).emittime < 0.0 {
                            pscript_delink_trailstate(ptr::addr_of_mut!((*kill).state.trailstate));
                        }
                        (*p).next = (*kill).next;
                        (*kill).next = g::p_kill_list;
                        g::p_kill_list = kill;
                        if g::p_kill_first.is_null() {
                            g::p_kill_first = kill;
                        }
                        continue;
                    }
                    break;
                }
                deferred_push(
                    ptr::addr_of_mut!(g::particle_updates),
                    g::num_particle_updates,
                    ptr::addr_of_mut!(g::max_particle_updates),
                );
                let upd = g::particle_updates.offset(g::num_particle_updates as isize);
                (*upd).p = p;
                (*upd).type_ = type_;
                g::num_particle_updates += 1;

                p = (*p).next;
            }
            (*meta).count = g::num_particle_updates - (*meta).start;

            type_ = (*type_).nexttorun;
        }

        // COMPAT: ADR-010 -- C float-to-int conversion is undefined out of
        // range; Rust `as` saturates.
        PARTICLE_TRACE_LIMIT = q_max_i(
            cvar_value(ptr::addr_of!(g::r_particle_tracelimit)) as c_int,
            0,
        ) as u32;
        g::FtePart_Glue_AtomicStoreU32(
            ptr::addr_of_mut!(g::particle_traces_used).cast::<c_void>(),
            0,
        );
        PARTICLE_UPDATE_SEED = c::COM_Rand() as u32;
        cl_prepare_trace_line_entities();

        0
    }
}

/// `r_part_fte.c:6771` -- `PScript_UpdateParticlesTask`.
///
/// Indexed over the worker count: pure particle local work plus read only BSP
/// traces. Each index updates an interleaved set of chunks so uneven trace
/// costs still balance.
///
/// # Safety
/// One call per task index; the flattened `particle_updates` array must not be
/// mutated while these run.
unsafe fn pscript_update_particles_task(index: c_int) {
    // SAFETY: `particle_updates` holds `num_particle_updates` live entries for
    // the duration of the parallel region, and the chunking below never leaves
    // that range.
    unsafe {
        let stride = q_max_i(g::Tasks_NumWorkers(), 1) * PARTICLE_UPDATE_CHUNK_SIZE;
        let mut rng: u32 =
            (PARTICLE_UPDATE_SEED ^ (index as u32).wrapping_mul(2_654_435_761u32)) | 1;
        let mut start = index * PARTICLE_UPDATE_CHUNK_SIZE;
        while start < g::num_particle_updates {
            let end = q_min_i(start + PARTICLE_UPDATE_CHUNK_SIZE, g::num_particle_updates);
            for upd in start..end {
                let u = g::particle_updates.offset(upd as isize);
                pscript_update_particle((*u).p, (*u).type_, g::p_frametime, P_DOFLURRY, &mut rng);
            }
            start += stride;
        }
    }
}

// ---------------------------------------------------------------------------
// Exported cores
//
// Every plain `PScript_*` name in `glquake.h:109-131`, plus `CL_TraceLine`,
// stays a thin wrapper in `Quake/r_part_fte_glue.c`: cbindgen cannot spell
// `qmodel_t *`, `msurface_t *`, `entity_t *`, `trailstate_t **` or `vec3_t`,
// and the raise-capable ones have to turn a status code back into a
// `Host_Error` on the C side of the frame (ADR-009 rule 3).
//
// `VectorNormalize2` and `PScript_QueueEffect` are exported for the opposite
// direction: they were ported, but `r_part_fte.c:5846` and `:7071` (both in
// the render/emit half, which stays C) still call them.
// ---------------------------------------------------------------------------

/// `r_part_fte.c:88` -- `VectorNormalize2`. Exported only because the render
/// half (`:5846`) still calls it.
///
/// # Safety
///
/// `v` and `out` must address three `float`s each.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_vector_normalize2(
    v: *const c_float,
    out: *mut c_float,
) -> c_float {
    // SAFETY: pointer contracts per the fn docs.
    unsafe { vector_normalize2(&*v.cast::<Vec3>(), &mut *out.cast::<Vec3>()) }
}

/// `r_part_fte.c:3252` -- `PScript_InitParticles`.
///
/// # Safety
///
/// Called once from `Quake/r_part_fte_glue.c`, which turns the status code
/// back into a raise.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_init_particles() -> Raise {
    // SAFETY: the callee's contract.
    unsafe { pscript_init_particles() }
}

/// `r_part_fte.c:3306` -- `PScript_Shutdown`.
///
/// # Safety
///
/// See [`quake_rs_ftepart_init_particles`].
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_shutdown() -> Raise {
    // SAFETY: the callee's contract.
    unsafe { pscript_shutdown() }
}

/// `r_part_fte.c:3286` -- `PScript_ClearSurfaceParticles`.
///
/// # Safety
///
/// `md` must point at a live `qmodel_t`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_clear_surface_particles(md: *mut c_void) {
    // SAFETY: `md` per the fn docs.
    unsafe { pscript_clear_surface_particles(md.cast::<QModel>()) }
}

/// `r_part_fte.c:3421` -- `PScript_ClearParticles`.
///
/// # Safety
///
/// See [`quake_rs_ftepart_init_particles`].
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_clear_particles(load: bool) -> Raise {
    // SAFETY: the callee's contract.
    unsafe { pscript_clear_particles(load) }
}

/// `r_part_fte.c:3585` -- `PScript_UpdateModelEffects`.
///
/// # Safety
///
/// `md` must point at a live `qmodel_t`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_update_model_effects(md: *mut c_void) {
    // SAFETY: `md` per the fn docs.
    unsafe { pscript_update_model_effects(md.cast::<QModel>()) }
}

/// `r_part_fte.c:3609` -- `PScript_FindParticleType`.
///
/// # Safety
///
/// `fullname` must be a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_find_particle_type(fullname: *const c_char) -> c_int {
    // SAFETY: `fullname` per the fn docs.
    unsafe { pscript_find_particle_type(fullname) }
}

/// `r_part_fte.c:3676` -- `R_ParticleDesc_Callback`, the `r_particledesc`
/// cvar callback.
///
/// # Safety
///
/// `var` must point at the glue-owned `r_particledesc`. The glue thunk turns
/// the status code back into a raise.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_particle_desc_callback(var: *mut c::cvar_t) -> Raise {
    // SAFETY: `var` per the fn docs.
    unsafe { r_particle_desc_callback(var) }
}

/// `r_part_fte.c:857` -- `P_PartRedirect_f`, the `r_partredirect` command.
///
/// # Safety
///
/// Called only from the `Cmd_AddCommand2` thunk in the glue.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_part_redirect_f() {
    // SAFETY: the callee's contract.
    unsafe { p_part_redirect_f() }
}

/// `r_part_fte.c:2629` -- `P_PartInfo_f`, the `r_partinfo` command.
///
/// # Safety
///
/// See [`quake_rs_ftepart_part_redirect_f`].
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_part_info_f() {
    // SAFETY: the callee's contract.
    unsafe { p_part_info_f() }
}

/// `r_part_fte.c:2595` -- `P_BeamInfo_f`, the `r_beaminfo` command.
///
/// # Safety
///
/// See [`quake_rs_ftepart_part_redirect_f`].
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_beam_info_f() {
    // SAFETY: the callee's contract.
    unsafe { p_beam_info_f() }
}

/// `r_part_fte.c:4680` -- `PScript_EmitSkyEffectTris`.
///
/// # Safety
///
/// `md` must point at a live `qmodel_t` and `fa` at one of its surfaces.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_emit_sky_effect_tris(
    md: *mut c_void,
    fa: *mut c_void,
    ptype: c_int,
) {
    // SAFETY: pointer contracts per the fn docs.
    unsafe { pscript_emit_sky_effect_tris(md.cast::<QModel>(), fa.cast::<MSurface>(), ptype) }
}

/// `r_part_fte.c:4757` -- `PScript_DelinkTrailstate`.
///
/// # Safety
///
/// `tsk` must be a writable `trailstate_t *`. Spelled `void **` because
/// `trailstate_t` has no engine-header declaration -- it is private to
/// `r_part_fte.c` -- so cbindgen would emit an undeclared name into
/// `quake_rs.h`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_delink_trailstate(tsk: *mut *mut c_void) {
    // SAFETY: `tsk` per the fn docs.
    unsafe { pscript_delink_trailstate(tsk.cast::<*mut trailstate_t>()) }
}

/// `r_part_fte.c:4311` -- `PScript_RunParticleEffectState`.
///
/// # Safety
///
/// `org`/`dir` must address three `float`s each (`dir` may be NULL), `tsk`
/// may be NULL, and `out` must be a writable `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_run_particle_effect_state(
    org: *const c_float,
    dir: *const c_float,
    count: c_float,
    typenum: c_int,
    tsk: *mut *mut c_void,
    out: *mut c_int,
) -> Raise {
    // SAFETY: pointer contracts per the fn docs.
    unsafe {
        pscript_run_particle_effect_state(
            org.cast::<Vec3>(),
            dir.cast::<Vec3>(),
            count,
            typenum,
            tsk.cast::<*mut trailstate_t>(),
            out,
        )
    }
}

/// `r_part_fte.c:5552` -- `PScript_ParticleTrail`. Cannot raise, so it
/// returns the C result directly.
///
/// # Safety
///
/// `startpos`/`end` must address three `float`s each; `axis` must be NULL or
/// address three `vec3_t`; `tsk` may be NULL.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_particle_trail(
    startpos: *const c_float,
    end: *const c_float,
    type_: c_int,
    timeinterval: c_float,
    dlkey: c_int,
    axis: *const c_float,
    tsk: *mut *mut c_void,
) -> c_int {
    // SAFETY: pointer contracts per the fn docs.
    unsafe {
        pscript_particle_trail(
            startpos.cast::<Vec3>(),
            end.cast::<Vec3>(),
            type_,
            timeinterval,
            dlkey,
            axis.cast::<Vec3>(),
            tsk.cast::<*mut trailstate_t>(),
        )
    }
}

/// `r_part_fte.c:4926` -- `PScript_RunParticleEffectTypeString`.
///
/// # Safety
///
/// `org`/`dir` must address three `float`s each, `name` must be a
/// NUL-terminated string, and `out` must be a writable `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_run_particle_effect_type_string(
    org: *const c_float,
    dir: *const c_float,
    count: c_float,
    name: *const c_char,
    out: *mut c_int,
) -> Raise {
    // SAFETY: pointer contracts per the fn docs.
    unsafe {
        pscript_run_particle_effect_type_string(
            org.cast::<Vec3>(),
            dir.cast::<Vec3>(),
            count,
            name,
            out,
        )
    }
}

/// `r_part_fte.c:4938` -- `PScript_EntParticleTrail`.
///
/// # Safety
///
/// `oldorg` must address three `float`s, `ent` must point into `cl.entities`,
/// `name` must be NUL-terminated, and `out` must be a writable `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_ent_particle_trail(
    oldorg: *const c_float,
    ent: *mut c_void,
    name: *const c_char,
    out: *mut c_int,
) -> Raise {
    // SAFETY: pointer contracts per the fn docs.
    unsafe { pscript_ent_particle_trail(oldorg.cast::<Vec3>(), ent.cast::<Entity>(), name, out) }
}

/// `r_part_fte.c:4952` -- `PScript_RunParticleEffect`.
///
/// # Safety
///
/// `org`/`dir` must address three `float`s each and `out` must be a writable
/// `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_run_particle_effect(
    org: *const c_float,
    dir: *const c_float,
    color: c_int,
    count: c_int,
    out: *mut c_int,
) -> Raise {
    // SAFETY: pointer contracts per the fn docs.
    unsafe {
        pscript_run_particle_effect(org.cast::<Vec3>(), dir.cast::<Vec3>(), color, count, out)
    }
}

/// `r_part_fte.c:4986` -- `PScript_RunParticleWeather`.
///
/// # Safety
///
/// `minb`/`maxb`/`dir` must address three `float`s each and `efname` must be
/// a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_run_particle_weather(
    minb: *const c_float,
    maxb: *const c_float,
    dir: *const c_float,
    count: c_float,
    colour: c_int,
    efname: *const c_char,
) -> Raise {
    // SAFETY: pointer contracts per the fn docs.
    unsafe {
        pscript_run_particle_weather(
            minb.cast::<Vec3>(),
            maxb.cast::<Vec3>(),
            dir.cast::<Vec3>(),
            count,
            colour,
            efname,
        )
    }
}

/// `r_part_fte.c:6325` -- `PScript_QueueEffect`. Exported only because the
/// render/emit half (`:7071`) still calls it.
///
/// # Safety
///
/// `org` and `dir` must address three `float`s each.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_queue_effect(
    org: *const c_float,
    dir: *const c_float,
    count: c_float,
    type_: c_int,
) {
    // SAFETY: pointer contracts per the fn docs.
    unsafe { pscript_queue_effect(org.cast::<Vec3>(), dir.cast::<Vec3>(), count, type_) }
}

/// `r_part_fte.c:6383` -- `PScript_FlushDlightsTask`. The C entry point takes
/// an ignored `void *`; the glue drops it.
///
/// # Safety
///
/// Must run serially, outside the task graph's parallel region.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_flush_dlights_task() {
    // SAFETY: the callee's contract.
    unsafe { pscript_flush_dlights_task() }
}

/// `r_part_fte.c:6596` -- `PScript_UpdateParticlesSetupTask`.
///
/// # Safety
///
/// Must run serially, outside the task graph's parallel region. The glue
/// turns the status code back into a raise.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_update_particles_setup_task() -> Raise {
    // SAFETY: the callee's contract.
    unsafe { pscript_update_particles_setup_task() }
}

/// `r_part_fte.c:6771` -- `PScript_UpdateParticlesTask`.
///
/// # Safety
///
/// One call per task index, inside the parallel region.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ftepart_update_particles_task(index: c_int) {
    // SAFETY: the callee's contract.
    unsafe { pscript_update_particles_task(index) }
}
