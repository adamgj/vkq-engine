//! `Quake/chase.c` -- chase-camera code (Rust migration Phase 7 M7, T7.2a,
//! Pattern A whole-file swap).
//!
//! ## ADR-009 raise-topology audit
//!
//! `chase.c` has no direct raise site (no `Host_Error`, `Host_EndGame` or
//! `Sys_Error`) and exactly one transitive one: `Chase_Init`'s four
//! `Cvar_RegisterVariable` calls, which are themselves `Host_Reraise`
//! wrappers under `-Duse_rust_cvar` (`Quake/cvar_cmd_glue.c`). They go
//! through the `Chase_Glue_RegisterVariable` `Host_Guard` trampoline, so
//! `quake_rs_chase_init` is the module's only `Raise`-returning core.
//!
//! `TraceLine` and `Chase_UpdateForDrawing` reach only
//! `SV_RecursiveHullCheck` (`crate::world`, non-raising -- it can `Sys_Error`,
//! which aborts rather than jumping) and `quake-math`, so they return `()`.
//! `Chase_UpdateForClient` is empty upstream and stays empty here.
//!
//! ## Shared state
//!
//! `cl` became Rust-owned in T7.4 (ADR-007 row closed; storage in
//! `crate::cl_main`), while `r_refdef` belongs to `gl_rmain.c`,
//! which is Phase 8). Both are reached through `crate::view`'s externs and
//! its ADR-011-shaped `RefDef`/`Entity` mirrors rather than being duplicated
//! here.

use core::ffi::{c_float, c_int};
use core::ptr;

use quake_c_sys::chase as g;
use quake_math::mathlib as m;
use quake_math::mathlib::Vec3;

use crate::view::{cl, cvar_value, r_refdef, PITCH, YAW};
use crate::world::{SV_RecursiveHullCheck, Trace};

/// A `Host_Guard` status: 0 means "no raise". Non-zero must be returned to
/// `Quake/chase_glue.c` untouched.
type Raise = c_int;

/// `world.h` `CONTENTMASK_ANYSOLID` == `(1u << 2) | (1u << 8)`. Spelled here
/// rather than imported because `crate::world`'s copy is module-private.
const CONTENTMASK_ANYSOLID: core::ffi::c_uint = 260;

/// `chase.c:35` -- registers the four chase cvars in source order. The order
/// is preserved for faithfulness even though `Cvar_RegisterVariable` inserts
/// alphabetically (`Quake/cvar.c:663`), so it is not observable in
/// `config.cfg` or `cvarlist`.
///
/// # Safety
/// C ABI entry point; call only from `Quake/chase_glue.c`'s `Chase_Init`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_chase_init() -> Raise {
    // SAFETY: the four cvars are C-owned statics in Quake/chase_glue.c; the
    // trampoline only forwards their addresses to Cvar_RegisterVariable.
    unsafe {
        let r = g::Chase_Glue_RegisterVariable(ptr::addr_of_mut!(g::chase_back));
        if r != 0 {
            return r;
        }
        let r = g::Chase_Glue_RegisterVariable(ptr::addr_of_mut!(g::chase_up));
        if r != 0 {
            return r;
        }
        let r = g::Chase_Glue_RegisterVariable(ptr::addr_of_mut!(g::chase_right));
        if r != 0 {
            return r;
        }
        let r = g::Chase_Glue_RegisterVariable(ptr::addr_of_mut!(g::chase_active));
        if r != 0 {
            return r;
        }
        0
    }
}

/// `chase.c:50`.
///
/// # Safety
/// `start`, `end` and `impact` must each point at three writable `float`s,
/// and `cl.worldmodel` must be a live brush model.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_trace_line(
    start: *mut c_float,
    end: *mut c_float,
    impact: *mut c_float,
) {
    // SAFETY: pointer contract per the fn docs; cl.worldmodel is read only to
    // take the address of its hull array, exactly as C's `cl.worldmodel->hulls`
    // decays to `&hulls[0]`.
    unsafe {
        // C: `trace_t trace; memset (&trace, 0, sizeof (trace));`
        let mut trace: Trace = core::mem::zeroed();
        let worldmodel = ptr::addr_of!(cl.worldmodel).read();
        let hulls = ptr::addr_of_mut!((*worldmodel).hulls).cast();
        SV_RecursiveHullCheck(hulls, start, end, &mut trace, CONTENTMASK_ANYSOLID);
        ptr::copy_nonoverlapping(trace.endpos.as_ptr(), impact, 3);
    }
}

/// `chase.c:65` -- empty upstream (all four statements are comments).
///
/// # Safety
/// C ABI entry point; no state is touched.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_chase_update_for_client() {
    // place camera
    // assign client angles to camera
    // see where camera points
    // adjust client angles to point at the same place
}

/// `chase.c:83`.
///
/// # Safety
/// C ABI entry point; `cl` and `r_refdef` must be live and `cl.worldmodel`
/// must be a brush model (`TraceLine` dereferences it).
#[no_mangle]
pub unsafe extern "C" fn quake_rs_chase_update_for_drawing() {
    // SAFETY: `cl`/`r_refdef` are the engine's own C storage; every access
    // below is a plain field read/write through their ADR-011 mirrors.
    unsafe {
        chase_update_for_drawing();
    }
}

/// The body, callable from `crate::view` without an FFI round trip
/// (`view.c:864` calls `Chase_UpdateForDrawing` directly).
///
/// # Safety
/// As [`quake_rs_chase_update_for_drawing`].
pub(crate) unsafe fn chase_update_for_drawing() {
    // SAFETY: see the caller docs.
    unsafe {
        let clp = ptr::addr_of_mut!(cl);
        let rd = ptr::addr_of_mut!(r_refdef);

        let mut forward: Vec3 = [0.0; 3];
        let mut right: Vec3 = [0.0; 3];
        let mut up: Vec3 = [0.0; 3];
        let mut ideal: Vec3 = [0.0; 3];
        let mut crosshair_vec: Vec3 = [0.0; 3];
        let mut temp: Vec3 = [0.0; 3];

        let viewangles = ptr::addr_of!((*clp).viewangles).read();
        m::angle_vectors(&viewangles, &mut forward, &mut right, &mut up);

        let viewent = ptr::addr_of_mut!((*clp).viewent).cast::<crate::view::Entity>();
        let vorigin = ptr::addr_of!((*viewent).origin).read();

        let chase_back = cvar_value(ptr::addr_of!(g::chase_back));
        let chase_right = cvar_value(ptr::addr_of!(g::chase_right));
        let chase_up = cvar_value(ptr::addr_of!(g::chase_up));

        // calc ideal camera location before checking for walls
        for i in 0..3 {
            ideal[i] = vorigin[i] - forward[i] * chase_back + right[i] * chase_right;
        }
        //+ up[i]*chase_up.value;
        ideal[2] = vorigin[2] + chase_up;

        // make sure camera is not in or behind a wall
        quake_rs_trace_line(
            ptr::addr_of_mut!((*rd).vieworg).cast(),
            ideal.as_mut_ptr(),
            temp.as_mut_ptr(),
        );
        if m::vector_length(&temp) != 0.0 {
            ideal = temp;
        }

        // place camera
        ptr::addr_of_mut!((*rd).vieworg).write(ideal);

        // find the spot the player is looking at
        // COMPAT: ADR-010 -- `1 << 20` is an int literal; C converts it to
        // float for the VectorMA scale, so the constant is exact.
        m::vector_ma(&vorigin, (1i32 << 20) as c_float, &forward, &mut temp);
        let mut vorigin_arg = vorigin;
        quake_rs_trace_line(
            vorigin_arg.as_mut_ptr(),
            temp.as_mut_ptr(),
            crosshair_vec.as_mut_ptr(),
        );

        if m::vector_length(&crosshair_vec) == 0.0 {
            // didn't hit anything
            crosshair_vec = temp;
        }

        // calculate camera angles to look at the same spot
        let vieworg = ptr::addr_of!((*rd).vieworg).read();
        m::vector_subtract(&crosshair_vec, &vieworg, &mut temp);
        let mut angles: Vec3 = ptr::addr_of!((*rd).viewangles).read();
        m::vector_angles(&temp, None, &mut angles);
        ptr::addr_of_mut!((*rd).viewangles).write(angles);
        let pitch = ptr::addr_of!((*rd).viewangles[PITCH]).read();
        if pitch == 90.0 || pitch == -90.0 {
            let yaw = ptr::addr_of!((*clp).viewangles[YAW]).read();
            ptr::addr_of_mut!((*rd).viewangles[YAW]).write(yaw);
        }
    }
}
