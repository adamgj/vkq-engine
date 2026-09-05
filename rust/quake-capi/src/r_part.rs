//! `Quake/r_part.c` -- the classic particle simulator (Rust migration Phase 7
//! M10f-1, T10.5, Pattern A whole-file swap).
//!
//! Only the *simulation* half lives here. The rendering half
//! (`R_ParticleTextureLookup`, `R_InitParticleTextures`,
//! `R_SetParticleTexture_f`, `R_InitParticleIndexBuffer`,
//! `R_DrawParticlesFaces`, `R_DrawParticles`, `R_DrawParticles_ShowTris`)
//! stays C verbatim in `Quake/r_part_glue.c`: it is Vulkan-typed throughout
//! and the renderer belongs to Phase 8 per `ROADMAP.md`.
//!
//! ## ADR-009 raise-topology audit
//!
//! `r_part.c` has no direct raise site. Its transitive surface is:
//!
//! * `Cvar_RegisterVariable` (`r_part.c:247`, `:249`) -- raise-capable: under
//!   `-Duse_rust_cvar` the plain name is itself a `Host_Reraise` wrapper
//!   (`Quake/cvar_cmd_glue.c`), the same reason `chase.c` and `view.c` guard
//!   it. It goes through the `RPart_Glue_RegisterVariable` `Host_Guard`
//!   trampoline, which makes [`quake_rs_rpart_init_particles`] the module's
//!   only `Raise`-returning core.
//! * `Cvar_SetCallback` (`r_part.c:248`) -- a field store in `cvar.c`; no
//!   raise.
//! * `Con_Printf` (`r_part.c:388`, `:392`, `:406`, `:427`) -- plain and
//!   unguarded, per the standing project decision.
//! * `MSG_ReadCoord`/`MSG_ReadChar`/`MSG_ReadByte` (`r_part.c:415-419`) -- an
//!   overrun only sets `msg_badread`.
//! * `COM_FOpenFile` (`r_part.c:385`) -- reports failure through its out
//!   parameter.
//! * `Mem_Alloc` (`r_part.c:244`) -- a bare `mi_calloc`/`SDL_calloc` wrapper
//!   (`mem.c:88`); on failure it `Sys_Error`s, which aborts rather than
//!   `longjmp`s.
//! * `RPart_Glue_InitRender` (`r_part.c:250-254`) -- `TexMgr_LoadImage` and
//!   the Vulkan buffer setup only `Sys_Error`.
//!
//! Every other core therefore returns `()`.
//!
//! ## ADR-010 notes
//!
//! `sin`/`cos` go through [`libm`], never `f32`/`f64` methods. C's
//! `float`-typed intermediates are reproduced with an explicit `as c_float` at
//! each assignment, and C's `double` promotions (`* 0.01`, `* 0.05`,
//! `* (1.0 / 16)`, `cl.time + ...`) are computed in `f64` before being
//! narrowed, because the narrowing point is observable in the state hash.
//! Float-to-int conversions go through [`as_int`].
//!
//! ## Shared state (ADR-007)
//!
//! The particle pool (`active_particles`, `free_particles`, `particles`,
//! `r_numparticles`) and the two cvars stay C in `Quake/r_part_glue.c` -- the
//! rendering half reads all of them. `ramp1`/`ramp2`/`ramp3` are read only by
//! the simulation and moved here. `cl` and `cls` became Rust-owned in T7.4
//! (storage in [`crate::cl_main`]).

use core::ffi::{c_char, c_float, c_int, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::libm;
use quake_c_sys::r_part as g;
use quake_c_sys::r_part::{particle_t, NUMVERTEXNORMALS};
use quake_c_sys::sv_phys::sv_gravity;
use quake_math::mathlib as m;
use quake_math::mathlib::Vec3;
use quake_types::host::{ClientState, ClientStatic, CA_CONNECTED};

use crate::mathlib::vec3_origin;

/// A `Host_Guard` status: 0 means "no raise". Non-zero must be returned to
/// `Quake/r_part_glue.c` untouched.
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
    /// ADR-007 row closed in T7.4; storage in [`crate::cl_main`].
    static mut cl: ClientState;
    /// ADR-007 row closed in T7.4; storage in [`crate::cl_main`].
    static mut cls: ClientStatic;
}

/// `r_part.c:30` -- `#define MAX_PARTICLES 16384`.
const MAX_PARTICLES: c_int = 16384;
/// `r_part.c:31` -- `#define ABSOLUTE_MIN_PARTICLES 512`.
const ABSOLUTE_MIN_PARTICLES: c_int = 512;
/// `q_types.h:240` -- `#define MAX_QPATH 64`.
const MAX_QPATH: usize = 64;

/// `r_part.c:34` -- `static const int ramp1[8]`.
const RAMP1: [c_int; 8] = [0x6f, 0x6d, 0x6b, 0x69, 0x67, 0x65, 0x63, 0x61];
/// `r_part.c:35` -- `static const int ramp2[8]`.
const RAMP2: [c_int; 8] = [0x6f, 0x6e, 0x6d, 0x6c, 0x6b, 0x6a, 0x68, 0x66];
// `r_part.c:36` declares `ramp3` as `[8]` but supplies six initialisers; C
// zero-fills the tail (C99 6.7.8p21 -- defined, not undefined), so the two
// trailing zeroes are part of the object and are reproduced literally. Not a
// COMPAT exception: nothing is being deviated from. No reader can reach them
// either -- both indexing sites are bounded to 0..=5 (`CL_RunParticles` tests
// `p->ramp >= 6` before indexing, and `R_RocketTrail` seeds `ramp` from
// `COM_Rand () & 3` or that plus 2).
/// `r_part.c:36` -- `static const int ramp3[8] = {0x6d, 0x6b, 6, 5, 4, 3};`.
const RAMP3: [c_int; 8] = [0x6d, 0x6b, 6, 5, 4, 3, 0, 0];

/// `r_part.c:268` -- `vec3_t avelocities[NUMVERTEXNORMALS]`. Non-`static` in C
/// by accident: no other translation unit names it, so it is private here.
static mut AVELOCITIES: [[c_float; 3]; NUMVERTEXNORMALS] = [[0.0; 3]; NUMVERTEXNORMALS];

/// `r_part.c:269` -- `float beamlength = 16;`, never assigned anywhere in the
/// engine. (`r_part.c:270-272`'s `avelocity`, `partstep` and `timescale` are
/// defined but read by nothing in any translation unit, so they are not
/// reproduced.)
const BEAMLENGTH: c_float = 16.0;

/// `r_part.c:702` -- `R_RocketTrail`'s function-local `static int
/// tracercount`, which alternates tracer spin direction across calls.
static mut TRACERCOUNT: c_int = 0;

// COMPAT: ADR-010 -- C's implicit float->int conversion. Out-of-range values
// are UB in C and saturate in Rust; the same shim `progs_builtins_cl.rs` uses.
#[inline]
fn as_int(x: c_float) -> c_int {
    x as c_int
}

/// `r_part.c`'s spawn idiom (`:307-311`, `:376-380`, ...): pop the LIFO free
/// list and push onto the active list. Returns null -- and touches nothing --
/// when the pool is exhausted, which is C's `if (!free_particles)` guard.
///
/// # Safety
/// The pool heads in `Quake/r_part_glue.c` must be consistent, which they are
/// once `R_ClearParticles` has run.
#[inline]
unsafe fn spawn() -> *mut particle_t {
    // SAFETY: single-threaded engine state; `free_particles` is either null or
    // a live pool element whose `next` is in-bounds.
    unsafe {
        if g::free_particles.is_null() {
            return ptr::null_mut();
        }
        let p = g::free_particles;
        g::free_particles = (*p).next;
        (*p).next = g::active_particles;
        g::active_particles = p;
        p
    }
}

/// `cl.time`.
///
/// # Safety
/// Reads a live engine global.
#[inline]
unsafe fn cl_time() -> f64 {
    // SAFETY: a plain `double` field of a Rust-owned static.
    unsafe { ptr::addr_of!(cl.time).read() }
}

/// `r_part.c:227`.
///
/// # Safety
/// C ABI entry point; runs once at startup, before any other core here.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_rpart_init_particles() -> Raise {
    // SAFETY: `com_argv` is the argv the engine parsed and `COM_CheckParm`
    // never returns the last index, so `i + 1` is in-bounds; the two cvars are
    // statics owned by `Quake/r_part_glue.c`.
    unsafe {
        let i = c::COM_CheckParm(c"-particles".as_ptr());

        if i != 0 {
            g::r_numparticles = g::atoi(*c::com_argv.add((i + 1) as usize));
            if g::r_numparticles < ABSOLUTE_MIN_PARTICLES {
                g::r_numparticles = ABSOLUTE_MIN_PARTICLES;
            }
        } else {
            g::r_numparticles = MAX_PARTICLES;
        }

        g::particles =
            c::Mem_Alloc(g::r_numparticles as usize * size_of::<particle_t>()).cast::<particle_t>();

        // johnfitz
        raise!(g::RPart_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::r_particles
        )));
        c::Cvar_SetCallback(
            ptr::addr_of_mut!(g::r_particles),
            Some(g::RPart_Glue_SetParticleTexture_f),
        );
        // johnfitz
        raise!(g::RPart_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::r_quadparticles
        )));

        g::RPart_Glue_InitRender();
        0
    }
}

/// `r_part.c:274` -- `R_EntityParticles (entity_t *ent)`, reduced to the only
/// field it reads: `entity_t` is deliberately outside the bindgen surface.
///
/// # Safety
/// `origin` must point at three readable `float`s.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_rpart_entity_particles(origin: *const c_float) {
    // SAFETY: `origin` per the fn docs; the pool heads are engine statics and
    // `r_avertexnormals` has exactly NUMVERTEXNORMALS rows.
    unsafe {
        let dist: c_float = 64.0;

        let av = ptr::addr_of_mut!(AVELOCITIES);
        if (*av)[0][0] == 0.0 {
            for i in 0..NUMVERTEXNORMALS {
                (*av)[i][0] = ((c::COM_Rand() & 255) as f64 * 0.01) as c_float;
                (*av)[i][1] = ((c::COM_Rand() & 255) as f64 * 0.01) as c_float;
                (*av)[i][2] = ((c::COM_Rand() & 255) as f64 * 0.01) as c_float;
            }
        }

        let time = cl_time();
        for i in 0..NUMVERTEXNORMALS {
            let mut angle: c_float = (time * (*av)[i][0] as f64) as c_float;
            let sy = libm::sin(angle as f64) as c_float;
            let cy = libm::cos(angle as f64) as c_float;
            angle = (time * (*av)[i][1] as f64) as c_float;
            let sp = libm::sin(angle as f64) as c_float;
            let cp = libm::cos(angle as f64) as c_float;
            // C recomputes `angle` from avelocities[i][2] here and never reads
            // it again (upstream's sr/cr pair is commented out), so the store
            // is dead and is not reproduced.

            let forward: Vec3 = [cp * cy, cp * sy, -sp];

            let p = spawn();
            if p.is_null() {
                return;
            }

            (*p).die = (time + 0.01) as c_float;
            (*p).color = 0x6f as c_float;
            (*p).type_ = g::PT_EXPLODE;

            #[allow(clippy::needless_range_loop)] // four parallel triples
            for j in 0..3 {
                (*p).org[j] =
                    *origin.add(j) + g::r_avertexnormals[i][j] * dist + forward[j] * BEAMLENGTH;
            }
        }
    }
}

/// `r_part.c:332`.
///
/// # Safety
/// C ABI entry point; `particles` must be the `R_InitParticles` allocation.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_rpart_clear_particles() {
    // SAFETY: `particles` points at `r_numparticles` elements. The `i + 1`
    // store on the last iteration forms a one-past-the-end pointer, which is
    // in-bounds for `add` and is overwritten with null immediately after --
    // exactly what C does.
    unsafe {
        g::free_particles = g::particles;
        g::active_particles = ptr::null_mut();

        for i in 0..g::r_numparticles as usize {
            (*g::particles.add(i)).next = g::particles.add(i + 1);
        }
        (*g::particles.add(g::r_numparticles as usize - 1)).next = ptr::null_mut();
    }
}

/// `r_part.c:349`.
///
/// # Safety
/// C ABI entry point; a console command.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_rpart_read_point_file_f() {
    // SAFETY: `cl`/`cls` are Rust-owned statics; `name` is a local buffer whose
    // length is handed to `q_snprintf`; `f` is checked before use.
    unsafe {
        if ptr::addr_of!(cls.state).read() != CA_CONNECTED {
            return; // need an active map.
        }

        let mut name = [0 as c_char; MAX_QPATH];
        g::q_snprintf(
            name.as_mut_ptr(),
            MAX_QPATH,
            c"maps/%s.pts".as_ptr(),
            ptr::addr_of!(cl.mapname).cast::<c_char>(),
        );

        let mut f: *mut c::FILE = ptr::null_mut();
        c::COM_FOpenFile(name.as_ptr(), &mut f, ptr::null_mut());
        if f.is_null() {
            c::Con_Printf(c"couldn't open %s\n".as_ptr(), name.as_ptr());
            return;
        }

        c::Con_Printf(c"Reading %s...\n".as_ptr(), name.as_ptr());
        let mut cnt: c_int = 0;
        // silence pesky compiler warnings
        let mut org: Vec3 = [0.0, 0.0, 0.0];
        loop {
            let r = g::RPart_Glue_ScanPoint(f, org.as_mut_ptr());
            if r != 3 {
                break;
            }
            cnt += 1;

            let p = spawn();
            if p.is_null() {
                c::Con_Printf(c"Not enough free particles\n".as_ptr());
                break;
            }

            (*p).die = 99999.0;
            (*p).color = ((-cnt) & 15) as c_float;
            (*p).type_ = g::PT_STATIC;
            (*p).vel = ptr::addr_of!(vec3_origin).read();
            (*p).org = org;
        }

        c::stdio::fclose(f);
        c::Con_Printf(c"%i points read\n".as_ptr(), cnt);
    }
}

/// `r_part.c:409`.
///
/// # Safety
/// C ABI entry point; called from the parse loop with a live `net_message`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_rpart_parse_particle_effect() {
    // SAFETY: the `MSG_Read*` cursor is engine state; an overrun only sets
    // `msg_badread`.
    unsafe {
        let mut org: Vec3 = [0.0; 3];
        let mut dir: Vec3 = [0.0; 3];

        let protocolflags = ptr::addr_of!(cl.protocolflags).read();
        for o in org.iter_mut() {
            *o = g::MSG_ReadCoord(protocolflags);
        }
        for d in dir.iter_mut() {
            *d = (g::MSG_ReadChar() as f64 * (1.0 / 16.0)) as c_float;
        }
        let msgcount = g::MSG_ReadByte();
        let color = g::MSG_ReadByte();

        let count = if msgcount == 255 { 1024 } else { msgcount };

        run_particle_effect(org.as_mut_ptr(), dir.as_mut_ptr(), color, count);
    }
}

/// `r_part.c:433`.
///
/// # Safety
/// `org` must point at three readable `float`s.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_rpart_particle_explosion(org: *mut c_float) {
    // SAFETY: `org` per the fn docs.
    unsafe {
        let time = cl_time();
        for i in 0..1024 {
            let p = spawn();
            if p.is_null() {
                return;
            }

            (*p).die = (time + 5.0) as c_float;
            (*p).color = RAMP1[0] as c_float;
            (*p).ramp = (c::COM_Rand() & 3) as c_float;
            if i & 1 != 0 {
                (*p).type_ = g::PT_EXPLODE;
                for j in 0..3 {
                    (*p).org[j] = *org.add(j) + ((c::COM_Rand() % 32) - 16) as c_float;
                    (*p).vel[j] = ((c::COM_Rand() % 512) - 256) as c_float;
                }
            } else {
                (*p).type_ = g::PT_EXPLODE2;
                for j in 0..3 {
                    (*p).org[j] = *org.add(j) + ((c::COM_Rand() % 32) - 16) as c_float;
                    (*p).vel[j] = ((c::COM_Rand() % 512) - 256) as c_float;
                }
            }
        }
    }
}

/// `r_part.c:476`.
///
/// # Safety
/// `org` must point at three readable `float`s.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_rpart_particle_explosion2(
    org: *mut c_float,
    color_start: c_int,
    color_length: c_int,
) {
    // SAFETY: `org` per the fn docs.
    unsafe {
        let time = cl_time();
        let mut color_mod: c_int = 0;

        #[allow(clippy::explicit_counter_loop)]
        // C's colorMod is a modulus operand, not the loop index
        for _i in 0..512 {
            let p = spawn();
            if p.is_null() {
                return;
            }

            (*p).die = (time + 0.3) as c_float;
            // COMPAT: ADR-004 -- `colorMod % colorLength` with a zero
            // `colorLength` is undefined in C (C99 6.5.5p5) and is reachable
            // from the wire: `svc_temp_entity` TE_EXPLOSION2 reads
            // `colorLength` with `MSG_ReadByte`, so a hostile or corrupt
            // server can send 0. C's behaviour is **not self-consistent across
            // the targets this project builds**: x86-64 `idiv` raises #DE ->
            // SIGFPE, while arm64 `sdiv`/`msub` is architecturally defined to
            // yield 0 for a zero divisor, so the C build *survives* on arm64
            // and continues with `color_start + colorMod`. Rust's `%` panics
            // on both, and `panic = "abort"` makes that an immediate process
            // death. This is therefore an **observable divergence on arm64**,
            // not merely a different way of dying. Following the ADR-010
            // Phase 7 reasoning for the same situation -- when C has no single
            // cross-target behaviour there is nothing to be faithful to -- the
            // port takes the defined Rust behaviour rather than emulating one
            // target's UB (ADR-004: no UB in Rust; `menu.rs:3528` precedent).
            (*p).color = (color_start + (color_mod % color_length)) as c_float;
            color_mod += 1;

            (*p).type_ = g::PT_BLOB;
            for j in 0..3 {
                (*p).org[j] = *org.add(j) + ((c::COM_Rand() % 32) - 16) as c_float;
                (*p).vel[j] = ((c::COM_Rand() % 512) - 256) as c_float;
            }
        }
    }
}

/// `r_part.c:509`.
///
/// # Safety
/// `org` must point at three readable `float`s.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_rpart_blob_explosion(org: *mut c_float) {
    // SAFETY: `org` per the fn docs.
    unsafe {
        let time = cl_time();
        for i in 0..1024 {
            let p = spawn();
            if p.is_null() {
                return;
            }

            (*p).die = (time + 1.0 + (c::COM_Rand() & 8) as f64 * 0.05) as c_float;

            if i & 1 != 0 {
                (*p).type_ = g::PT_BLOB;
                (*p).color = (66 + c::COM_Rand() % 6) as c_float;
                for j in 0..3 {
                    (*p).org[j] = *org.add(j) + ((c::COM_Rand() % 32) - 16) as c_float;
                    (*p).vel[j] = ((c::COM_Rand() % 512) - 256) as c_float;
                }
            } else {
                (*p).type_ = g::PT_BLOB2;
                (*p).color = (150 + c::COM_Rand() % 6) as c_float;
                for j in 0..3 {
                    (*p).org[j] = *org.add(j) + ((c::COM_Rand() % 32) - 16) as c_float;
                    (*p).vel[j] = ((c::COM_Rand() % 512) - 256) as c_float;
                }
            }
        }
    }
}

/// `r_part.c:553`.
///
/// # Safety
/// `org` and `dir` must each point at three readable `float`s.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_rpart_run_particle_effect(
    org: *mut c_float,
    dir: *mut c_float,
    color: c_int,
    count: c_int,
) {
    // SAFETY: pointer contract per the fn docs.
    unsafe { run_particle_effect(org, dir, color, count) }
}

/// The body of `R_RunParticleEffect`, called directly by
/// [`quake_rs_rpart_parse_particle_effect`] the way `r_part.c:426` calls the
/// plain name from inside its own translation unit.
///
/// # Safety
/// As [`quake_rs_rpart_run_particle_effect`].
unsafe fn run_particle_effect(org: *mut c_float, dir: *mut c_float, color: c_int, count: c_int) {
    // SAFETY: pointer contract per the fn docs.
    unsafe {
        let time = cl_time();
        for i in 0..count {
            let p = spawn();
            if p.is_null() {
                return;
            }

            if count == 1024 {
                // rocket explosion
                (*p).die = (time + 5.0) as c_float;
                (*p).color = RAMP1[0] as c_float;
                (*p).ramp = (c::COM_Rand() & 3) as c_float;
                if i & 1 != 0 {
                    (*p).type_ = g::PT_EXPLODE;
                    for j in 0..3 {
                        (*p).org[j] = *org.add(j) + ((c::COM_Rand() % 32) - 16) as c_float;
                        (*p).vel[j] = ((c::COM_Rand() % 512) - 256) as c_float;
                    }
                } else {
                    (*p).type_ = g::PT_EXPLODE2;
                    for j in 0..3 {
                        (*p).org[j] = *org.add(j) + ((c::COM_Rand() % 32) - 16) as c_float;
                        (*p).vel[j] = ((c::COM_Rand() % 512) - 256) as c_float;
                    }
                }
            } else {
                (*p).die = (time + 0.1 * (c::COM_Rand() % 5) as f64) as c_float;
                (*p).color = ((color & !7) + (c::COM_Rand() & 7)) as c_float;
                (*p).type_ = g::PT_SLOWGRAV;
                for j in 0..3 {
                    (*p).org[j] = *org.add(j) + ((c::COM_Rand() & 15) - 8) as c_float;
                    (*p).vel[j] = *dir.add(j) * 15.0; // + (COM_Rand()%300)-150;
                }
            }
        }
    }
}

/// `r_part.c:610`.
///
/// # Safety
/// `org` must point at three readable `float`s.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_rpart_lava_splash(org: *mut c_float) {
    // SAFETY: `org` per the fn docs.
    unsafe {
        let time = cl_time();
        let mut dir: Vec3 = [0.0; 3];

        for i in -16..16 {
            for j in -16..16 {
                for _k in 0..1 {
                    let p = spawn();
                    if p.is_null() {
                        return;
                    }

                    (*p).die = (time + 2.0 + (c::COM_Rand() & 31) as f64 * 0.02) as c_float;
                    (*p).color = (224 + (c::COM_Rand() & 7)) as c_float;
                    (*p).type_ = g::PT_SLOWGRAV;

                    dir[0] = (j * 8 + (c::COM_Rand() & 7)) as c_float;
                    dir[1] = (i * 8 + (c::COM_Rand() & 7)) as c_float;
                    dir[2] = 256.0;

                    (*p).org[0] = *org + dir[0];
                    (*p).org[1] = *org.add(1) + dir[1];
                    (*p).org[2] = *org.add(2) + (c::COM_Rand() & 63) as c_float;

                    m::vector_normalize(&mut dir);
                    let vel = (50 + (c::COM_Rand() & 63)) as c_float;
                    m::vector_scale(&dir, vel, &mut (*p).vel);
                }
            }
        }
    }
}

/// `r_part.c:651`.
///
/// # Safety
/// `org` must point at three readable `float`s.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_rpart_teleport_splash(org: *mut c_float) {
    // SAFETY: `org` per the fn docs.
    unsafe {
        let time = cl_time();
        let mut dir: Vec3 = [0.0; 3];

        let mut i = -16;
        while i < 16 {
            let mut j = -16;
            while j < 16 {
                let mut k = -24;
                while k < 32 {
                    let p = spawn();
                    if p.is_null() {
                        return;
                    }

                    (*p).die = (time + 0.2 + (c::COM_Rand() & 7) as f64 * 0.02) as c_float;
                    (*p).color = (7 + (c::COM_Rand() & 7)) as c_float;
                    (*p).type_ = g::PT_SLOWGRAV;

                    dir[0] = (j * 8) as c_float;
                    dir[1] = (i * 8) as c_float;
                    dir[2] = (k * 8) as c_float;

                    (*p).org[0] = *org + i as c_float + (c::COM_Rand() & 3) as c_float;
                    (*p).org[1] = *org.add(1) + j as c_float + (c::COM_Rand() & 3) as c_float;
                    (*p).org[2] = *org.add(2) + k as c_float + (c::COM_Rand() & 3) as c_float;

                    m::vector_normalize(&mut dir);
                    let vel = (50 + (c::COM_Rand() & 63)) as c_float;
                    m::vector_scale(&dir, vel, &mut (*p).vel);

                    k += 4;
                }
                j += 4;
            }
            i += 4;
        }
    }
}

/// `r_part.c:694`. `start` is advanced in place, exactly as C does.
///
/// # Safety
/// `start` must point at three writable `float`s and `end` at three readable
/// ones.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_rpart_rocket_trail(
    start: *mut c_float,
    end: *mut c_float,
    trail_type: c_int,
) {
    // SAFETY: pointer contract per the fn docs. `start` is mirrored into a
    // local and written back after every `VectorAdd`, so an early return leaves
    // it advanced by exactly the completed iterations, as in C.
    unsafe {
        let mut trail_type = trail_type;
        let time = cl_time();
        let mut vec: Vec3 = [0.0; 3];
        let mut s: Vec3 = [*start, *start.add(1), *start.add(2)];
        let e: Vec3 = [*end, *end.add(1), *end.add(2)];

        m::vector_subtract(&e, &s, &mut vec);
        let mut len = m::vector_normalize(&mut vec);
        let dec: c_int = if trail_type < 128 {
            3
        } else {
            trail_type -= 128;
            1
        };

        while len > 0.0 {
            len -= dec as c_float;

            let p = spawn();
            if p.is_null() {
                return;
            }

            (*p).vel = ptr::addr_of!(vec3_origin).read();
            (*p).die = (time + 2.0) as c_float;

            match trail_type {
                0 => {
                    // rocket trail
                    (*p).ramp = (c::COM_Rand() & 3) as c_float;
                    (*p).color = RAMP3[as_int((*p).ramp) as usize] as c_float;
                    (*p).type_ = g::PT_FIRE;
                    #[allow(clippy::needless_range_loop)] // s and org in lockstep
                    for j in 0..3 {
                        (*p).org[j] = s[j] + ((c::COM_Rand() % 6) - 3) as c_float;
                    }
                }
                1 => {
                    // smoke smoke
                    (*p).ramp = ((c::COM_Rand() & 3) + 2) as c_float;
                    (*p).color = RAMP3[as_int((*p).ramp) as usize] as c_float;
                    (*p).type_ = g::PT_FIRE;
                    #[allow(clippy::needless_range_loop)] // s and org in lockstep
                    for j in 0..3 {
                        (*p).org[j] = s[j] + ((c::COM_Rand() % 6) - 3) as c_float;
                    }
                }
                2 => {
                    // blood
                    (*p).type_ = g::PT_GRAV;
                    (*p).color = (67 + (c::COM_Rand() & 3)) as c_float;
                    #[allow(clippy::needless_range_loop)] // s and org in lockstep
                    for j in 0..3 {
                        (*p).org[j] = s[j] + ((c::COM_Rand() % 6) - 3) as c_float;
                    }
                }
                3 | 5 => {
                    // tracer
                    (*p).die = (time + 0.5) as c_float;
                    (*p).type_ = g::PT_STATIC;
                    if trail_type == 3 {
                        (*p).color = (52 + ((TRACERCOUNT & 4) << 1)) as c_float;
                    } else {
                        (*p).color = (230 + ((TRACERCOUNT & 4) << 1)) as c_float;
                    }

                    TRACERCOUNT += 1;

                    (*p).org = s;
                    if TRACERCOUNT & 1 != 0 {
                        (*p).vel[0] = 30.0 * vec[1];
                        (*p).vel[1] = 30.0 * -vec[0];
                    } else {
                        (*p).vel[0] = 30.0 * -vec[1];
                        (*p).vel[1] = 30.0 * vec[0];
                    }
                }
                4 => {
                    // slight blood
                    (*p).type_ = g::PT_GRAV;
                    (*p).color = (67 + (c::COM_Rand() & 3)) as c_float;
                    #[allow(clippy::needless_range_loop)] // s and org in lockstep
                    for j in 0..3 {
                        (*p).org[j] = s[j] + ((c::COM_Rand() % 6) - 3) as c_float;
                    }
                    len -= 3.0;
                }
                6 => {
                    // voor trail
                    (*p).color = (9 * 16 + 8 + (c::COM_Rand() & 3)) as c_float;
                    (*p).type_ = g::PT_STATIC;
                    (*p).die = (time + 0.3) as c_float;
                    #[allow(clippy::needless_range_loop)] // s and org in lockstep
                    for j in 0..3 {
                        (*p).org[j] = s[j] + ((c::COM_Rand() & 15) - 8) as c_float;
                    }
                }
                _ => {}
            }

            let prev = s;
            m::vector_add(&prev, &vec, &mut s);
            ptr::copy_nonoverlapping(s.as_ptr(), start, 3);
        }
    }
}

/// `r_part.c:806` -- `CL_RunParticles`.
///
/// # Safety
/// C ABI entry point; called once per host frame.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_rpart_run_particles() {
    // SAFETY: the pool heads and `sv_gravity` are live engine statics; every
    // particle reached through `next` is a pool element.
    unsafe {
        let time = cl_time();
        let oldtime = ptr::addr_of!(cl.oldtime).read();
        // C: `q_max (0.0, cl.time - cl.oldtime)` expands to `0.0 > d ? 0.0 : d`.
        let d = time - oldtime;
        let frametime = (if 0.0 > d { 0.0 } else { d }) as c_float;
        let time3 = frametime * 15.0;
        let time2 = frametime * 10.0;
        let time1 = frametime * 5.0;
        let gravity = ptr::addr_of!(sv_gravity.value).read();
        // C: `frametime * sv_gravity.value * 0.05` -- the left product is
        // `float`, `0.05` promotes the result to `double`, and the store to
        // `float grav` narrows it back.
        let grav = ((frametime * gravity) as f64 * 0.05) as c_float;
        let dvel = 4.0 * frametime;

        loop {
            let kill = g::active_particles;
            if !kill.is_null() && ((*kill).die as f64) < time {
                g::active_particles = (*kill).next;
                (*kill).next = g::free_particles;
                g::free_particles = kill;
                continue;
            }
            break;
        }

        let mut p = g::active_particles;
        while !p.is_null() {
            loop {
                let kill = (*p).next;
                if !kill.is_null() && ((*kill).die as f64) < time {
                    (*p).next = (*kill).next;
                    (*kill).next = g::free_particles;
                    g::free_particles = kill;
                    continue;
                }
                break;
            }

            (*p).org[0] += (*p).vel[0] * frametime;
            (*p).org[1] += (*p).vel[1] * frametime;
            (*p).org[2] += (*p).vel[2] * frametime;

            match (*p).type_ {
                g::PT_STATIC => {}
                g::PT_FIRE => {
                    (*p).ramp += time1;
                    if (*p).ramp >= 6.0 {
                        (*p).die = -1.0;
                    } else {
                        (*p).color = RAMP3[as_int((*p).ramp) as usize] as c_float;
                    }
                    (*p).vel[2] += grav;
                }
                g::PT_EXPLODE => {
                    (*p).ramp += time2;
                    if (*p).ramp >= 8.0 {
                        (*p).die = -1.0;
                    } else {
                        (*p).color = RAMP1[as_int((*p).ramp) as usize] as c_float;
                    }
                    for i in 0..3 {
                        (*p).vel[i] += (*p).vel[i] * dvel;
                    }
                    (*p).vel[2] -= grav;
                }
                g::PT_EXPLODE2 => {
                    (*p).ramp += time3;
                    if (*p).ramp >= 8.0 {
                        (*p).die = -1.0;
                    } else {
                        (*p).color = RAMP2[as_int((*p).ramp) as usize] as c_float;
                    }
                    for i in 0..3 {
                        (*p).vel[i] -= (*p).vel[i] * frametime;
                    }
                    (*p).vel[2] -= grav;
                }
                g::PT_BLOB => {
                    for i in 0..3 {
                        (*p).vel[i] += (*p).vel[i] * dvel;
                    }
                    (*p).vel[2] -= grav;
                }
                g::PT_BLOB2 => {
                    for i in 0..2 {
                        (*p).vel[i] -= (*p).vel[i] * dvel;
                    }
                    (*p).vel[2] -= grav;
                }
                g::PT_GRAV | g::PT_SLOWGRAV => {
                    (*p).vel[2] -= grav;
                }
                _ => {}
            }

            p = (*p).next;
        }
    }
}

/// `r_part.c:927` -- `Harness_HashParticles`.
///
/// # Safety
/// C ABI entry point; the pool must be either null or fully initialised.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_rpart_hash_particles(h: u64) -> u64 {
    // SAFETY: every live particle is an element of the `particles` allocation,
    // so `offset_from` is defined and every field read is in-bounds.
    unsafe {
        let mut h = h;
        let mut count: c_int = 0;

        if g::particles.is_null() {
            return h;
        }

        let mut p = g::active_particles;
        while !p.is_null() {
            let index = p.offset_from(g::particles) as c_int;
            h = g::Harness_Hash64(h, ptr::addr_of!(index).cast::<c_void>(), size_of::<c_int>());
            h = g::Harness_Hash64(
                h,
                ptr::addr_of!((*p).org).cast::<c_void>(),
                size_of::<[c_float; 3]>(),
            );
            h = g::Harness_Hash64(
                h,
                ptr::addr_of!((*p).vel).cast::<c_void>(),
                size_of::<[c_float; 3]>(),
            );
            h = g::Harness_Hash64(
                h,
                ptr::addr_of!((*p).color).cast::<c_void>(),
                size_of::<c_float>(),
            );
            h = g::Harness_Hash64(
                h,
                ptr::addr_of!((*p).ramp).cast::<c_void>(),
                size_of::<c_float>(),
            );
            h = g::Harness_Hash64(
                h,
                ptr::addr_of!((*p).die).cast::<c_void>(),
                size_of::<c_float>(),
            );
            h = g::Harness_Hash64(
                h,
                ptr::addr_of!((*p).type_).cast::<c_void>(),
                size_of::<g::ptype_t>(),
            );
            count += 1;
            p = (*p).next;
        }
        h = g::Harness_Hash64(h, ptr::addr_of!(count).cast::<c_void>(), size_of::<c_int>());
        h
    }
}
