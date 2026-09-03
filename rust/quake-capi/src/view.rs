//! `Quake/view.c` -- player eye positioning (Rust migration Phase 7 M7,
//! T7.2a, Pattern A whole-file swap).
//!
//! ## ADR-009 raise-topology audit
//!
//! `view.c` has exactly one direct raise-class site: the `Sys_Error` at
//! `view.c:922` (`"V_RenderView: entities needed relink in main draw"`).
//! `Sys_Error` aborts the process, it does not `longjmp`, so -- following the
//! `world.rs:32`, `sv_phys.rs:33` and M6 `sv_send.c:1096` precedent -- it is
//! called directly from Rust and needs no guard.
//!
//! The transitive surface is three call sites, all in `V_RenderView` and
//! `V_Init`:
//!
//! - `CL_RelinkEntities` (`view.c:924`) reaches `Mem_Realloc`, `R_AddEfrags`
//!   and the `PScript_*` particle system, all of which can `Host_Error`.
//!   Guarded by `View_Glue_RelinkEntities`.
//! - `R_RenderView` (`view.c:927`) is the whole renderer. Guarded by
//!   `View_Glue_RenderView`.
//! - `Cvar_RegisterVariable` (30x) and `Cmd_AddCommand` (3x) in `V_Init` are
//!   themselves `Host_Reraise` wrappers under `-Duse_rust_cvar`
//!   (`Quake/cvar_cmd_glue.c`), so a Rust frame must never call them
//!   directly. Guarded by `View_Glue_RegisterVariable` and
//!   `View_Glue_AddCommands`.
//!
//! Everything else `view.c` calls is non-raising: `MSG_ReadByte`/
//! `MSG_ReadCoord` (return -1 / set `msg_badread` on underflow),
//! `Cmd_Argv`/`atoi`, `quake-math`, `libm`, and `Chase_UpdateForDrawing`
//! (`crate::chase`, whose own audit is in that module). `quake_rs_v_init` and
//! `quake_rs_v_render_view` are therefore this module's only `Raise`-returning
//! cores; nothing here calls `Host_Reraise`, which `Quake/view_glue.c` owns.
//!
//! ## Ownership (ADR-007 / ADR-011)
//!
//! `cl` and `cls` became Rust-owned in T7.4 (ADR-007 row closed; storage in
//! `crate::cl_main`), while `r_refdef` belongs to `gl_rmain.c`, which stays C
//! until Phase 8. All three are read and written in place through the externs
//! below.
//!
//! `quake-types::host` has no `refdef_t` or `entity_t` field mirror:
//! `ClientState::viewent`/`entities` are the deliberately opaque
//! [`quake_types::host::EntityOpaque`] blob (its 456-byte stride stays
//! authoritative for indexing). `view.c` reads and writes 14 distinct
//! `entity_t` fields and 3 `refdef_t` ones, which per-field accessor calls
//! would turn into ~30 FFI round trips, so this module defines local
//! `#[repr(C)]` [`Entity`]/[`EntLerp`]/[`LightCache`]/[`RefDef`] mirrors
//! instead and only ever reaches them through `*mut` casts of the opaque
//! blob -- never by value. The ADR-011 gate for these three is
//! `offsetof`-based and lives in `rust/quake-ctest/tests/view_differential.rs`
//! (driven by probes in `rust/quake-ctest/stubs/view_ref.c`) rather than in
//! the shared `host_abi.rs`, because `host_abi.rs`/`abi_probe.c` are shared
//! with a concurrent task; folding them in is a follow-up, not a gap in
//! coverage.
//!
//! ## Function-local statics
//!
//! `CalcGunAngle` (`view.c:557`) and `V_CalcRefdef` (`view.c:717`) keep
//! cross-frame state in function-local `static`s. Those become the four
//! module-level `static mut`s below, zero-initialised exactly as C's are.
//! They are unreachable from the oracle's copies, so the ctest fixture
//! (`ctest_view_reset`) drives BOTH sides through a fixed prologue that
//! forces `oldz` and `punch` to the same known values before every test --
//! see `view_ref.c`.

use core::ffi::{c_float, c_int, c_uint, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::chase as gc;
use quake_c_sys::qboolean;
use quake_c_sys::view as g;
use quake_math::mathlib as m;
use quake_math::mathlib::Vec3;
use quake_types::host::{CShift, ClientState, ClientStatic, Efrag, QBoolean};
use quake_types::model_mem::QModel;
use quake_types::progs::EntityState;

/// A `Host_Guard` status: 0 means "no raise". Non-zero must be returned to
/// `Quake/view_glue.c` untouched.
type Raise = c_int;

/// Propagate a non-zero `Host_Guard` status to the caller, abandoning the
/// rest of the body exactly where C's `longjmp` would have left it.
macro_rules! raise {
    ($e:expr) => {{
        let r: Raise = $e;
        if r != 0 {
            return r;
        }
    }};
}

pub(crate) const PITCH: usize = 0;
pub(crate) const YAW: usize = 1;
pub(crate) const ROLL: usize = 2;

/// `Quake/quakedef.h:112-131`.
const STAT_HEALTH: usize = 0;
const STAT_WEAPON: usize = 2;
const STAT_WEAPONFRAME: usize = 5;
const STAT_VIEWHEIGHT: usize = 16;
const STAT_IDEALPITCH: usize = 25;

/// `Quake/client.h:56-60`.
const CSHIFT_CONTENTS: usize = 0;
const CSHIFT_DAMAGE: usize = 1;
const CSHIFT_BONUS: usize = 2;
const CSHIFT_POWERUP: usize = 3;
const NUM_CSHIFTS: usize = 4;

/// `Quake/bspfile.h:162-167`.
const CONTENTS_EMPTY: c_int = -1;
const CONTENTS_SOLID: c_int = -2;
const CONTENTS_SLIME: c_int = -4;
const CONTENTS_LAVA: c_int = -5;
const CONTENTS_SKY: c_int = -6;

/// `Quake/quakedef.h:160-163`.
const IT_INVISIBILITY: c_int = 524288;
const IT_INVULNERABILITY: c_int = 1048576;
const IT_SUIT: c_int = 2097152;
const IT_QUAD: c_int = 4194304;

/// `Quake/client.h:168` -- `countof (cl.movecmds) - 1`.
const MOVECMDS_MASK: usize = 63;

/// `Quake/mathlib.h` `M_PI`. COMPAT: ADR-010 -- `view.c` mixes this `double`
/// literal into otherwise-`float` expressions, promoting them; the promotion
/// points are reproduced exactly below.
const M_PI: f64 = core::f64::consts::PI;

const SYS_ERR_RENDER_VIEW_RELINK: &core::ffi::CStr =
    c"V_RenderView: entities needed relink in main draw";

// ---------------------------------------------------------------------------
// ADR-011 mirrors local to this module (see the module doc).

/// `render.h` `lightcache_t`.
#[repr(C)]
pub struct LightCache {
    pub surfidx: c_int,
    pub pos: [c_float; 3],
    pub ds: i16,
    pub dt: i16,
}

/// `render.h` `entlerp_t`.
#[repr(C)]
pub struct EntLerp {
    pub movestep: QBoolean,
    pub prev_frame: c_int,
    pub frame_change_time: f64,
    pub frame_duration: f64,
    pub frame_finish_time: f64,
    pub snap_frames: c_int,
    pub snap_msgtime: f64,
    pub prev_origin: [c_float; 3],
    pub prev_angles: [c_float; 3],
    pub move_change_time: f64,
    pub move_duration: f64,
}

/// `render.h` `entity_t`. Only ever reached through a `*mut` cast of
/// [`quake_types::host::EntityOpaque`]; never constructed by value, so the
/// opaque blob's stride stays authoritative for `cl.entities` indexing.
///
/// `PSET_SCRIPT` is defined unconditionally by `Quake/quakedef.h:38`, so
/// `trailstate`/`emitstate` are always present.
#[repr(C)]
pub struct Entity {
    pub forcelink: QBoolean,
    pub update_type: c_int,
    pub baseline: EntityState,
    pub netstate: EntityState,
    pub msgtime: f64,
    pub msg_origins: [[c_float; 3]; 2],
    pub origin: [c_float; 3],
    pub msg_angles: [[c_float; 3]; 2],
    pub angles: [c_float; 3],
    pub model: *mut QModel,
    pub efrag: *mut Efrag,
    pub frame: c_int,
    pub syncbase: c_float,
    pub colormap: *mut u8,
    pub effects: c_int,
    pub skinnum: c_int,
    pub visframe: c_int,
    pub dlightframe: c_int,
    pub dlightbits: c_int,
    pub topnode: *mut c_void,
    pub eflags: u8,
    pub alpha: u8,
    pub lerp: EntLerp,
    pub trailstate: *mut c_void,
    pub emitstate: *mut c_void,
    pub traildelay: c_float,
    pub trailorg: [c_float; 3],
    pub lightcache: LightCache,
    pub contentscache: c_int,
    pub contentscache_origin: [c_float; 3],
    pub blas_data: *mut c_void,
}

/// `vid.h` `vrect_t`.
#[repr(C)]
pub struct VRect {
    pub x: c_int,
    pub y: c_int,
    pub width: c_int,
    pub height: c_int,
    pub pnext: *mut VRect,
}

/// `render.h` `refdef_t` -- the global `r_refdef`, owned by `gl_rmain.c`.
#[repr(C)]
pub struct RefDef {
    pub vrect: VRect,
    pub aliasvrect: VRect,
    pub vrectright: c_int,
    pub vrectbottom: c_int,
    pub aliasvrectright: c_int,
    pub aliasvrectbottom: c_int,
    pub vrectrightedge: c_float,
    pub fvrectx: c_float,
    pub fvrecty: c_float,
    pub fvrectx_adj: c_float,
    pub fvrecty_adj: c_float,
    pub vrect_x_adj_shift20: c_int,
    pub vrectright_adj_shift20: c_int,
    pub fvrectright_adj: c_float,
    pub fvrectbottom_adj: c_float,
    pub fvrectright: c_float,
    pub fvrectbottom: c_float,
    pub horizontal_field_of_view: c_float,
    pub x_origin: c_float,
    pub y_origin: c_float,
    pub vieworg: [c_float; 3],
    pub viewangles: [c_float; 3],
    pub basefov: c_float,
    pub fov_x: c_float,
    pub fov_y: c_float,
    pub ambientlight: c_int,
}

extern "C" {
    /// `gl_rmain.c`. Written heavily by `view.c`; stays C-owned until Phase 8.
    pub static mut r_refdef: RefDef;
    /// ADR-007 row closed in T7.4; storage in [`crate::cl_main`].
    pub static mut cl: ClientState;
    /// ADR-007 row closed in T7.4; storage in [`crate::cl_main`].
    pub static mut cls: ClientStatic;
}

// ---------------------------------------------------------------------------
// view.c's function-local statics (view.c:557-558, :717-718).

static mut GUN_OLDYAW: c_float = 0.0;
static mut GUN_OLDPITCH: c_float = 0.0;
static mut REFDEF_OLDZ: c_float = 0.0;
static mut REFDEF_PUNCH: Vec3 = [0.0, 0.0, 0.0];

// ---------------------------------------------------------------------------
// view.c's file-scope const data (view.c:258-260).

const CSHIFT_WATER: CShift = CShift {
    destcolor: [130, 80, 50],
    percent: 128.0,
};
const CSHIFT_SLIME: CShift = CShift {
    destcolor: [0, 25, 5],
    percent: 150.0,
};
const CSHIFT_LAVA: CShift = CShift {
    destcolor: [255, 80, 0],
    percent: 150.0,
};

// ---------------------------------------------------------------------------
// helpers

/// Reads a `cvar_t`'s `.value` without forming a reference to the C static.
///
/// # Safety
/// `var` must point at a live, initialised `cvar_t`.
#[inline]
pub(crate) unsafe fn cvar_value(var: *const c::cvar_t) -> c_float {
    // SAFETY: caller guarantees `var` points at a live cvar_t owned by the
    // C glue, so the field read is in-bounds and properly aligned.
    unsafe { ptr::addr_of!((*var).value).read() }
}

/// `host.c` `host_frametime`.
#[inline]
unsafe fn host_frametime() -> f64 {
    // SAFETY: a plain `double` engine global.
    unsafe { ptr::addr_of!(c::host_frametime).read() }
}

/// `&cl.entities[i]`, striding by the authoritative opaque `entity_t` size.
///
/// # Safety
/// `cl.entities` must be a live array with more than `i` elements.
#[inline]
pub(crate) unsafe fn cl_entity(i: c_int) -> *mut Entity {
    // SAFETY: caller guarantees the index is in bounds; `EntityOpaque` has
    // the verified `sizeof (entity_t)` stride.
    unsafe {
        ptr::addr_of!(cl.entities)
            .read()
            .add(i as usize)
            .cast::<Entity>()
    }
}

/// `&cl.viewent`.
#[inline]
unsafe fn cl_viewent() -> *mut Entity {
    // SAFETY: `cl` is the engine's own storage; `viewent` is an inline field.
    unsafe { ptr::addr_of_mut!(cl.viewent).cast::<Entity>() }
}

/// `glquake.h:134` `InvalidateTraceLineCache()` -- a `++` on an `int`, not a
/// call.
#[inline]
unsafe fn invalidate_trace_line_cache() {
    // SAFETY: a plain `int` engine global.
    // COMPAT: ADR-010 -- signed overflow is UB in C and wraps here; the
    // counter is only ever compared for inequality.
    unsafe {
        let p = ptr::addr_of_mut!(g::r_trace_line_cache_counter);
        p.write(p.read().wrapping_add(1));
    }
}

// ---------------------------------------------------------------------------
// view.c:87 -- V_CalcRoll

/// # Safety
/// `angles` and `velocity` must each point at three readable `float`s.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_v_calc_roll(
    angles: *mut c_float,
    velocity: *mut c_float,
) -> c_float {
    // SAFETY: pointer contract per the fn docs.
    unsafe { v_calc_roll(&*angles.cast::<Vec3>(), &*velocity.cast::<Vec3>()) }
}

/// # Safety
/// Reads `cl_rollangle`/`cl_rollspeed`, which must be live cvars.
unsafe fn v_calc_roll(angles: &Vec3, velocity: &Vec3) -> c_float {
    // SAFETY: the two cvars are C-owned statics in Quake/view_glue.c.
    unsafe {
        let mut forward: Vec3 = [0.0; 3];
        let mut right: Vec3 = [0.0; 3];
        let mut up: Vec3 = [0.0; 3];

        m::angle_vectors(angles, &mut forward, &mut right, &mut up);
        let mut side = m::dot_product(velocity, &right);
        let sign: c_float = if side < 0.0 { -1.0 } else { 1.0 };
        // COMPAT: ADR-010 -- C's `fabs` is the `double` overload; the `float`
        // argument promotes and the result narrows back on assignment.
        side = c::libm::fabs(side as f64) as c_float;

        let value = cvar_value(ptr::addr_of!(g::cl_rollangle));
        //	if (cl.inwater)
        //		value *= 6;

        let rollspeed = cvar_value(ptr::addr_of!(g::cl_rollspeed));
        if side < rollspeed {
            side = side * value / rollspeed;
        } else {
            side = value;
        }

        side * sign
    }
}

// ---------------------------------------------------------------------------
// view.c:117 -- V_CalcBob

/// # Safety
/// `cl` and the `cl_bob*` cvars must be live.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_v_calc_bob() -> c_float {
    // SAFETY: engine globals owned by C.
    unsafe { v_calc_bob() }
}

unsafe fn v_calc_bob() -> c_float {
    // SAFETY: engine globals owned by C.
    unsafe {
        let bobcycle = cvar_value(ptr::addr_of!(g::cl_bobcycle));
        if bobcycle == 0.0 {
            /* Avoid divide-by-zero, don't bob */
            return 0.0;
        }

        let time = ptr::addr_of!(cl.time).read();

        // COMPAT: ADR-010 -- `(int)(cl.time / cl_bobcycle.value)` truncates a
        // double toward zero (UB out of range in C, saturating here); the
        // product is `int * float` -> float, and only the outer subtraction
        // is done in double before narrowing to the float `cycle`.
        let whole = (time / bobcycle as f64) as c_int;
        let mut cycle: c_float = (time - (whole as c_float * bobcycle) as f64) as c_float;
        cycle /= bobcycle;

        let bobup = cvar_value(ptr::addr_of!(g::cl_bobup));
        if cycle < bobup {
            cycle = (M_PI * cycle as f64 / bobup as f64) as c_float;
        } else {
            cycle = (M_PI + M_PI * (cycle - bobup) as f64 / (1.0 - bobup as f64)) as c_float;
        }

        // bob is proportional to velocity in the xy plane
        // (don't count Z, or jumping messes it up)
        let vel = ptr::addr_of!(cl.velocity).read();
        // COMPAT: ADR-010 -- the sum is float, `sqrt` is the double overload,
        // and `* cl_bob.value` stays in double before narrowing.
        let mut bob: c_float = (c::libm::sqrt((vel[0] * vel[0] + vel[1] * vel[1]) as f64)
            * cvar_value(ptr::addr_of!(g::cl_bob)) as f64)
            as c_float;
        // Con_Printf ("speed: %5.1f\n", VectorLength(cl.velocity));
        // COMPAT: ADR-010 -- `0.3`/`0.7` are double literals, so both terms
        // and the sum are computed in double.
        bob = (bob as f64 * 0.3 + bob as f64 * 0.7 * c::libm::sin(cycle as f64)) as c_float;
        // COMPAT: ADR-010 -- view.c:249-252 spells this as two compares, not
        // a clamp; kept literal so the comparison order is auditable.
        #[allow(clippy::manual_clamp)]
        if bob > 4.0 {
            bob = 4.0;
        } else if bob < -7.0 {
            bob = -7.0;
        }
        bob
    }
}

// ---------------------------------------------------------------------------
// view.c:150 / :166 -- V_StartPitchDrift / V_StopPitchDrift

/// # Safety
/// `cl` and `v_centerspeed` must be live.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_v_start_pitch_drift() {
    // SAFETY: engine globals owned by C.
    unsafe { v_start_pitch_drift() }
}

unsafe fn v_start_pitch_drift() {
    // SAFETY: engine globals owned by C.
    unsafe {
        let clp = ptr::addr_of_mut!(cl);
        if ptr::addr_of!((*clp).laststop).read() == ptr::addr_of!((*clp).time).read() {
            return; // something else is keeping it from drifting
        }
        if ptr::addr_of!((*clp).nodrift).read() || ptr::addr_of!((*clp).pitchvel).read() == 0.0 {
            ptr::addr_of_mut!((*clp).pitchvel).write(cvar_value(ptr::addr_of!(g::v_centerspeed)));
            ptr::addr_of_mut!((*clp).nodrift).write(false);
            ptr::addr_of_mut!((*clp).driftmove).write(0.0);
        }
    }
}

/// # Safety
/// `cl` must be live.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_v_stop_pitch_drift() {
    // SAFETY: engine globals owned by C.
    unsafe {
        let clp = ptr::addr_of_mut!(cl);
        let t = ptr::addr_of!((*clp).time).read();
        ptr::addr_of_mut!((*clp).laststop).write(t);
        ptr::addr_of_mut!((*clp).nodrift).write(true);
        ptr::addr_of_mut!((*clp).pitchvel).write(0.0);
    }
}

// ---------------------------------------------------------------------------
// view.c:185 -- V_DriftPitch

/// # Safety
/// `cl`, `cls` and the drift cvars must be live.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_v_drift_pitch() {
    // SAFETY: engine globals owned by C.
    unsafe { v_drift_pitch() }
}

unsafe fn v_drift_pitch() {
    // SAFETY: engine globals owned by C; CL_AngleLocked is non-raising.
    unsafe {
        let clp = ptr::addr_of_mut!(cl);

        if ptr::addr_of!(g::noclip_anglehack).read()
            || !ptr::addr_of!((*clp).onground).read()
            || ptr::addr_of!(cls.demoplayback).read()
            || g::CL_AngleLocked()
        // FIXME: noclip_anglehack is set on the server, so in a nonlocal game this won't work.
        {
            ptr::addr_of_mut!((*clp).driftmove).write(0.0);
            ptr::addr_of_mut!((*clp).pitchvel).write(0.0);
            return;
        }

        // don't count small mouse motion
        if ptr::addr_of!((*clp).nodrift).read() {
            let movemessages = ptr::addr_of!((*clp).movemessages).read();
            // COMPAT: `MOVECMDS_MASK` is `countof(...) - 1`, i.e. `size_t`, so
            // C converts the possibly-negative `movemessages - 1` to unsigned
            // before masking; the wrapping cast below reproduces that.
            let idx = (movemessages.wrapping_sub(1) as usize) & MOVECMDS_MASK;
            let forwardmove = ptr::addr_of!((*clp).movecmds[idx].forwardmove).read();
            // COMPAT: ADR-010 -- `fabs` is the double overload, so the
            // comparison against the float cvar happens in double.
            if c::libm::fabs(forwardmove as f64)
                < cvar_value(ptr::addr_of!(g::cl_forwardspeed)) as f64
            {
                ptr::addr_of_mut!((*clp).driftmove).write(0.0);
            } else {
                let d = ptr::addr_of!((*clp).driftmove).read();
                ptr::addr_of_mut!((*clp).driftmove).write((d as f64 + host_frametime()) as c_float);
            }

            #[allow(clippy::collapsible_if)] // view.c:206-210 nests these
            if ptr::addr_of!((*clp).driftmove).read() > cvar_value(ptr::addr_of!(g::v_centermove)) {
                if cvar_value(ptr::addr_of!(g::lookspring)) != 0.0 {
                    v_start_pitch_drift();
                }
            }
            return;
        }

        let delta: c_float = if cvar_value(ptr::addr_of!(g::v_autopitch)) != 0.0 {
            ptr::addr_of!((*clp).statsf[STAT_IDEALPITCH]).read()
                - ptr::addr_of!((*clp).viewangles[PITCH]).read()
        } else {
            -ptr::addr_of!((*clp).viewangles[PITCH]).read()
        };

        if delta == 0.0 {
            ptr::addr_of_mut!((*clp).pitchvel).write(0.0);
            return;
        }

        let mut move_: c_float =
            (host_frametime() * ptr::addr_of!((*clp).pitchvel).read() as f64) as c_float;
        let pv = ptr::addr_of!((*clp).pitchvel).read();
        ptr::addr_of_mut!((*clp).pitchvel).write(
            (pv as f64 + host_frametime() * cvar_value(ptr::addr_of!(g::v_centerspeed)) as f64)
                as c_float,
        );

        // Con_Printf ("move: %f (%f)\n", move, host_frametime);

        if delta > 0.0 {
            if move_ > delta {
                ptr::addr_of_mut!((*clp).pitchvel).write(0.0);
                move_ = delta;
            }
            let a = ptr::addr_of!((*clp).viewangles[PITCH]).read();
            ptr::addr_of_mut!((*clp).viewangles[PITCH]).write(a + move_);
        } else if delta < 0.0 {
            if move_ > -delta {
                ptr::addr_of_mut!((*clp).pitchvel).write(0.0);
                move_ = -delta;
            }
            let a = ptr::addr_of!((*clp).viewangles[PITCH]).read();
            ptr::addr_of_mut!((*clp).viewangles[PITCH]).write(a - move_);
        }
    }
}

// ---------------------------------------------------------------------------
// view.c:268 -- V_ResetBlend

/// # Safety
/// `cl` must be live.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_v_reset_blend() {
    // SAFETY: engine globals owned by C; the three writes are plain field
    // stores of all-zero, matching C's two `memset`s.
    unsafe {
        let clp = ptr::addr_of_mut!(cl);
        ptr::addr_of_mut!((*clp).cshift_empty).write_bytes(0u8, 1);
        ptr::addr_of_mut!((*clp).cshifts).write_bytes(0u8, 1);
        ptr::addr_of_mut!((*clp).v_dmg_time).write(0.0);
        ptr::addr_of_mut!((*clp).v_dmg_roll).write(0.0);
        ptr::addr_of_mut!((*clp).v_dmg_pitch).write(0.0);
    }
}

// ---------------------------------------------------------------------------
// view.c:280 -- V_ParseDamage

/// # Safety
/// `cl`/`cls` must be live and a message must be open for reading.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_v_parse_damage() {
    // SAFETY: engine globals owned by C; MSG_Read* are non-raising.
    unsafe {
        let clp = ptr::addr_of_mut!(cl);

        let armor = g::MSG_ReadByte();
        let blood = g::MSG_ReadByte();
        let protocolflags: c_uint = ptr::addr_of!((*clp).protocolflags).read();
        let mut from: Vec3 = [0.0; 3];
        for f in from.iter_mut() {
            *f = g::MSG_ReadCoord(protocolflags);
        }

        // COMPAT: ADR-010 -- `0.5` is a double literal, so both products and
        // the sum are computed in double before narrowing to the float
        // `count`.
        let mut count: c_float = (blood as f64 * 0.5 + armor as f64 * 0.5) as c_float;
        if count < 10.0 {
            count = 10.0;
        }

        // but sbar face into pain frame
        let t = ptr::addr_of!((*clp).time).read();
        ptr::addr_of_mut!((*clp).faceanimtime).write((t + 0.2) as c_float);

        if ptr::addr_of!(cls.demoseeking).read() {
            return;
        }

        let dmg = ptr::addr_of_mut!((*clp).cshifts[CSHIFT_DAMAGE]);
        let p = ptr::addr_of!((*dmg).percent).read();
        ptr::addr_of_mut!((*dmg).percent).write(p + 3.0 * count);
        if ptr::addr_of!((*dmg).percent).read() < 0.0 {
            ptr::addr_of_mut!((*dmg).percent).write(0.0);
        }
        if ptr::addr_of!((*dmg).percent).read() > 150.0 {
            ptr::addr_of_mut!((*dmg).percent).write(150.0);
        }

        if armor > blood {
            ptr::addr_of_mut!((*dmg).destcolor).write([200, 100, 100]);
        } else if armor != 0 {
            ptr::addr_of_mut!((*dmg).destcolor).write([220, 50, 50]);
        } else {
            ptr::addr_of_mut!((*dmg).destcolor).write([255, 0, 0]);
        }

        //
        // calculate view angle kicks
        //
        let ent = cl_entity(ptr::addr_of!((*clp).viewentity).read());

        let origin = ptr::addr_of!((*ent).origin).read();
        let mut diff: Vec3 = [0.0; 3];
        m::vector_subtract(&from, &origin, &mut diff);
        from = diff;
        m::vector_normalize(&mut from);

        let mut forward: Vec3 = [0.0; 3];
        let mut right: Vec3 = [0.0; 3];
        let mut up: Vec3 = [0.0; 3];
        let ent_angles = ptr::addr_of!((*ent).angles).read();
        m::angle_vectors(&ent_angles, &mut forward, &mut right, &mut up);

        let mut side = m::dot_product(&from, &right);
        ptr::addr_of_mut!((*clp).v_dmg_roll)
            .write(count * side * cvar_value(ptr::addr_of!(g::v_kickroll)));

        side = m::dot_product(&from, &forward);
        ptr::addr_of_mut!((*clp).v_dmg_pitch)
            .write(count * side * cvar_value(ptr::addr_of!(g::v_kickpitch)));

        ptr::addr_of_mut!((*clp).v_dmg_time).write(cvar_value(ptr::addr_of!(g::v_kicktime)));
    }
}

// ---------------------------------------------------------------------------
// view.c:351 -- V_cshift_f

/// # Safety
/// `cl` must be live and a command must be tokenized.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_v_cshift_f() {
    // SAFETY: engine globals owned by C; Cmd_Argv/atoi are non-raising.
    unsafe {
        let clp = ptr::addr_of_mut!(cl);
        let e = ptr::addr_of_mut!((*clp).cshift_empty);
        ptr::addr_of_mut!((*e).destcolor[0]).write(g::atoi(c::Cmd_Argv(1)));
        ptr::addr_of_mut!((*e).destcolor[1]).write(g::atoi(c::Cmd_Argv(2)));
        ptr::addr_of_mut!((*e).destcolor[2]).write(g::atoi(c::Cmd_Argv(3)));
        ptr::addr_of_mut!((*e).percent).write(g::atoi(c::Cmd_Argv(4)) as c_float);
    }
}

// ---------------------------------------------------------------------------
// view.c:366 -- V_BonusFlash_f

/// # Safety
/// `cl` must be live.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_v_bonus_flash_f() {
    // SAFETY: engine globals owned by C.
    unsafe {
        let bonus = ptr::addr_of_mut!(cl.cshifts[CSHIFT_BONUS]);
        ptr::addr_of_mut!((*bonus).destcolor).write([215, 186, 69]);
        ptr::addr_of_mut!((*bonus).percent).write(50.0);
    }
}

// ---------------------------------------------------------------------------
// view.c:381 -- V_SetContentsColor

/// # Safety
/// `cl` must be live.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_v_set_contents_color(contents: c_int) {
    // SAFETY: engine globals owned by C.
    unsafe {
        let clp = ptr::addr_of_mut!(cl);
        let dst = ptr::addr_of_mut!((*clp).cshifts[CSHIFT_CONTENTS]);
        match contents {
            // johnfitz -- no blend in sky
            CONTENTS_EMPTY | CONTENTS_SOLID | CONTENTS_SKY => {
                // modifiable by server using v_cshift command
                let empty = ptr::addr_of!((*clp).cshift_empty).read();
                dst.write(empty);
            }
            CONTENTS_LAVA => dst.write(CSHIFT_LAVA),
            CONTENTS_SLIME => dst.write(CSHIFT_SLIME),
            _ => dst.write(CSHIFT_WATER),
        }
    }
}

// ---------------------------------------------------------------------------
// view.c:404 -- V_CalcPowerupCshift

/// # Safety
/// `cl` must be live.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_v_calc_powerup_cshift() {
    // SAFETY: engine globals owned by C.
    unsafe { v_calc_powerup_cshift() }
}

unsafe fn v_calc_powerup_cshift() {
    // SAFETY: engine globals owned by C.
    unsafe {
        let clp = ptr::addr_of_mut!(cl);
        let items = ptr::addr_of!((*clp).items).read();
        let p = ptr::addr_of_mut!((*clp).cshifts[CSHIFT_POWERUP]);

        if items & IT_QUAD != 0 {
            ptr::addr_of_mut!((*p).destcolor).write([0, 0, 255]);
            ptr::addr_of_mut!((*p).percent).write(30.0);
        } else if items & IT_SUIT != 0 {
            ptr::addr_of_mut!((*p).destcolor).write([0, 255, 0]);
            ptr::addr_of_mut!((*p).percent).write(20.0);
        } else if items & IT_INVISIBILITY != 0 {
            ptr::addr_of_mut!((*p).destcolor).write([100, 100, 100]);
            ptr::addr_of_mut!((*p).percent).write(100.0);
        } else if items & IT_INVULNERABILITY != 0 {
            ptr::addr_of_mut!((*p).destcolor).write([255, 255, 0]);
            ptr::addr_of_mut!((*p).percent).write(30.0);
        } else {
            ptr::addr_of_mut!((*p).percent).write(0.0);
        }
    }
}

// ---------------------------------------------------------------------------
// view.c:445 -- V_CalcBlend

/// # Safety
/// `cl`, `v_blend` and the `gl_cshiftpercent*` cvars must be live.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_v_calc_blend() {
    // SAFETY: engine globals owned by C.
    unsafe { v_calc_blend() }
}

unsafe fn v_calc_blend() {
    // SAFETY: engine globals owned by C.
    unsafe {
        let clp = ptr::addr_of_mut!(cl);
        let cshiftpercent_cvars: [*const c::cvar_t; NUM_CSHIFTS] = [
            ptr::addr_of!(g::gl_cshiftpercent_contents),
            ptr::addr_of!(g::gl_cshiftpercent_damage),
            ptr::addr_of!(g::gl_cshiftpercent_bonus),
            ptr::addr_of!(g::gl_cshiftpercent_powerup),
        ];

        let mut r: c_float = 0.0;
        let mut gg: c_float = 0.0;
        let mut b: c_float = 0.0;
        let mut a: c_float = 0.0;

        let global_percent = cvar_value(ptr::addr_of!(g::gl_cshiftpercent));
        let intermission = ptr::addr_of!((*clp).intermission).read();

        for (j, cvar) in cshiftpercent_cvars.iter().enumerate() {
            if global_percent == 0.0 {
                continue;
            }

            // johnfitz -- only apply leaf contents color shifts during intermission
            if intermission != 0 && j != CSHIFT_CONTENTS {
                continue;
            }
            // johnfitz

            let shift = ptr::addr_of!((*clp).cshifts[j]);
            let percent = ptr::addr_of!((*shift).percent).read();
            // COMPAT: ADR-010 -- `percent * gl_cshiftpercent.value` is float;
            // both divisions have double literals on the right, so the rest of
            // the expression is double until it narrows into the float `a2`.
            let mut a2: c_float = (((percent * global_percent) as f64 / 100.0) / 255.0) as c_float;
            // QuakeSpasm -- also scale by the specific gl_cshiftpercent_* cvar
            a2 = (a2 as f64 * (cvar_value(*cvar) as f64 / 100.0)) as c_float;
            // QuakeSpasm
            if a2 == 0.0 {
                continue;
            }
            a += a2 * (1.0 - a);
            a2 /= a;
            let destcolor = ptr::addr_of!((*shift).destcolor).read();
            r = r * (1.0 - a2) + destcolor[0] as c_float * a2;
            gg = gg * (1.0 - a2) + destcolor[1] as c_float * a2;
            b = b * (1.0 - a2) + destcolor[2] as c_float * a2;
        }

        // COMPAT: ADR-010 -- `CLAMP` (q_minmax.h) picks the all-float
        // overload here, and C's float-to-uint8 conversion truncates toward
        // zero; the value is already clamped into [0, 255], so `as u8`
        // matches.
        let blend = ptr::addr_of_mut!(g::v_blend);
        ptr::addr_of_mut!((*blend)[0]).write(clamp_f(0.0, r, 255.0) as u8);
        ptr::addr_of_mut!((*blend)[1]).write(clamp_f(0.0, gg, 255.0) as u8);
        ptr::addr_of_mut!((*blend)[2]).write(clamp_f(0.0, b, 255.0) as u8);
        ptr::addr_of_mut!((*blend)[3]).write(clamp_f(0.0, a * 255.0, 255.0) as u8);
    }
}

/// `q_minmax.h` `clamp_f`.
#[inline]
fn clamp_f(minval: c_float, val: c_float, maxval: c_float) -> c_float {
    if val < minval {
        minval
    } else if val > maxval {
        maxval
    } else {
        val
    }
}

// ---------------------------------------------------------------------------
// view.c:493 -- V_UpdateBlend (static in C)

unsafe fn v_update_blend() {
    // SAFETY: engine globals owned by C.
    unsafe {
        let clp = ptr::addr_of_mut!(cl);

        v_calc_powerup_cshift();

        let mut blend_changed = false;

        for i in 0..NUM_CSHIFTS {
            let cur = ptr::addr_of_mut!((*clp).cshifts[i]);
            let prev = ptr::addr_of_mut!((*clp).prev_cshifts[i]);
            if ptr::addr_of!((*cur).percent).read() != ptr::addr_of!((*prev).percent).read() {
                blend_changed = true;
                let v = ptr::addr_of!((*cur).percent).read();
                ptr::addr_of_mut!((*prev).percent).write(v);
            }
            for j in 0..3 {
                if ptr::addr_of!((*cur).destcolor[j]).read()
                    != ptr::addr_of!((*prev).destcolor[j]).read()
                {
                    blend_changed = true;
                    let v = ptr::addr_of!((*cur).destcolor[j]).read();
                    ptr::addr_of_mut!((*prev).destcolor[j]).write(v);
                }
            }
        }

        // drop the damage value
        let dmg = ptr::addr_of_mut!((*clp).cshifts[CSHIFT_DAMAGE].percent);
        // COMPAT: ADR-010 -- `host_frametime * 150` is double, so the
        // compound assignment is done in double and narrowed back.
        dmg.write((dmg.read() as f64 - host_frametime() * 150.0) as c_float);
        if dmg.read() <= 0.0 {
            dmg.write(0.0);
        }

        // drop the bonus value
        let bonus = ptr::addr_of_mut!((*clp).cshifts[CSHIFT_BONUS].percent);
        bonus.write((bonus.read() as f64 - host_frametime() * 100.0) as c_float);
        if bonus.read() <= 0.0 {
            bonus.write(0.0);
        }

        if blend_changed {
            v_calc_blend();
        }
    }
}

// ---------------------------------------------------------------------------
// view.c:538 -- angledelta

#[no_mangle]
pub extern "C" fn quake_rs_angledelta(a: c_float) -> c_float {
    angledelta(a)
}

fn angledelta(a: c_float) -> c_float {
    let mut a = m::anglemod(a);
    if a > 180.0 {
        a -= 360.0;
    }
    a
}

// ---------------------------------------------------------------------------
// view.c:551 -- CalcGunAngle

/// # Safety
/// `cl` and `r_refdef` must be live.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_calc_gun_angle() {
    // SAFETY: engine globals owned by C.
    unsafe { calc_gun_angle() }
}

unsafe fn calc_gun_angle() {
    // SAFETY: engine globals owned by C.
    unsafe {
        let clp = ptr::addr_of_mut!(cl);
        let rd = ptr::addr_of_mut!(r_refdef);

        // COMPAT: the two initialisers below are immediately overwritten by
        // expressions that subtract the same r_refdef.viewangles component
        // back out, so both deltas are always exactly zero. Preserved
        // verbatim (upstream quirk, bug-for-bug).
        let mut yaw = ptr::addr_of!((*rd).viewangles[YAW]).read();
        let mut pitch = -ptr::addr_of!((*rd).viewangles[PITCH]).read();

        yaw =
            (angledelta(yaw - ptr::addr_of!((*rd).viewangles[YAW]).read()) as f64 * 0.4) as c_float;
        // COMPAT: ADR-010 -- view.c:562-567 spells both bounds as separate
        // `if`s (not `else if`, not a clamp); kept literal.
        #[allow(clippy::manual_clamp)]
        if yaw > 10.0 {
            yaw = 10.0;
        }
        #[allow(clippy::manual_clamp)]
        if yaw < -10.0 {
            yaw = -10.0;
        }
        pitch = (angledelta(-pitch - ptr::addr_of!((*rd).viewangles[PITCH]).read()) as f64 * 0.4)
            as c_float;
        // COMPAT: ADR-010 -- as above (view.c:569-574).
        #[allow(clippy::manual_clamp)]
        if pitch > 10.0 {
            pitch = 10.0;
        }
        #[allow(clippy::manual_clamp)]
        if pitch < -10.0 {
            pitch = -10.0;
        }
        let move_: c_float = (host_frametime() * 20.0) as c_float;

        let oldyaw = ptr::addr_of!(GUN_OLDYAW).read();
        if yaw > oldyaw {
            if oldyaw + move_ < yaw {
                yaw = oldyaw + move_;
            }
        } else if oldyaw - move_ > yaw {
            yaw = oldyaw - move_;
        }

        let oldpitch = ptr::addr_of!(GUN_OLDPITCH).read();
        if pitch > oldpitch {
            if oldpitch + move_ < pitch {
                pitch = oldpitch + move_;
            }
        } else if oldpitch - move_ > pitch {
            pitch = oldpitch - move_;
        }

        ptr::addr_of_mut!(GUN_OLDYAW).write(yaw);
        ptr::addr_of_mut!(GUN_OLDPITCH).write(pitch);

        let view = cl_viewent();
        let rd_yaw = ptr::addr_of!((*rd).viewangles[YAW]).read();
        let rd_pitch = ptr::addr_of!((*rd).viewangles[PITCH]).read();
        ptr::addr_of_mut!((*view).angles[YAW]).write(rd_yaw + yaw);
        ptr::addr_of_mut!((*view).angles[PITCH]).write(-(rd_pitch + pitch));

        let time = ptr::addr_of!((*clp).time).read();
        let idlescale = cvar_value(ptr::addr_of!(g::v_idlescale)) as f64;

        // COMPAT: ADR-010 -- `cl.time * cvar.value` is double, `sin` is the
        // double overload, and the whole product stays double until the
        // float `angles[]` component absorbs it.
        let roll = ptr::addr_of_mut!((*view).angles[ROLL]);
        roll.write(
            (roll.read() as f64
                - idlescale
                    * c::libm::sin(time * cvar_value(ptr::addr_of!(g::v_iroll_cycle)) as f64)
                    * cvar_value(ptr::addr_of!(g::v_iroll_level)) as f64) as c_float,
        );
        let pitch_p = ptr::addr_of_mut!((*view).angles[PITCH]);
        pitch_p.write(
            (pitch_p.read() as f64
                - idlescale
                    * c::libm::sin(time * cvar_value(ptr::addr_of!(g::v_ipitch_cycle)) as f64)
                    * cvar_value(ptr::addr_of!(g::v_ipitch_level)) as f64) as c_float,
        );
        let yaw_p = ptr::addr_of_mut!((*view).angles[YAW]);
        yaw_p.write(
            (yaw_p.read() as f64
                - idlescale
                    * c::libm::sin(time * cvar_value(ptr::addr_of!(g::v_iyaw_cycle)) as f64)
                    * cvar_value(ptr::addr_of!(g::v_iyaw_level)) as f64) as c_float,
        );
    }
}

// ---------------------------------------------------------------------------
// view.c:606 -- V_BoundOffsets

/// # Safety
/// `cl` and `r_refdef` must be live.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_v_bound_offsets() {
    // SAFETY: engine globals owned by C.
    unsafe { v_bound_offsets() }
}

unsafe fn v_bound_offsets() {
    // SAFETY: engine globals owned by C.
    unsafe {
        let clp = ptr::addr_of_mut!(cl);
        let rd = ptr::addr_of_mut!(r_refdef);
        let ent = cl_entity(ptr::addr_of!((*clp).viewentity).read());
        let origin = ptr::addr_of!((*ent).origin).read();

        // absolutely bound refresh reletive to entity clipping hull
        // so the view can never be inside a solid wall
        let v = ptr::addr_of_mut!((*rd).vieworg);
        if ptr::addr_of!((*v)[0]).read() < origin[0] - 14.0 {
            ptr::addr_of_mut!((*v)[0]).write(origin[0] - 14.0);
        } else if ptr::addr_of!((*v)[0]).read() > origin[0] + 14.0 {
            ptr::addr_of_mut!((*v)[0]).write(origin[0] + 14.0);
        }
        if ptr::addr_of!((*v)[1]).read() < origin[1] - 14.0 {
            ptr::addr_of_mut!((*v)[1]).write(origin[1] - 14.0);
        } else if ptr::addr_of!((*v)[1]).read() > origin[1] + 14.0 {
            ptr::addr_of_mut!((*v)[1]).write(origin[1] + 14.0);
        }
        if ptr::addr_of!((*v)[2]).read() < origin[2] - 22.0 {
            ptr::addr_of_mut!((*v)[2]).write(origin[2] - 22.0);
        } else if ptr::addr_of!((*v)[2]).read() > origin[2] + 30.0 {
            ptr::addr_of_mut!((*v)[2]).write(origin[2] + 30.0);
        }
    }
}

// ---------------------------------------------------------------------------
// view.c:634 -- V_AddIdle

/// # Safety
/// `cl` and `r_refdef` must be live.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_v_add_idle() {
    // SAFETY: engine globals owned by C.
    unsafe { v_add_idle() }
}

unsafe fn v_add_idle() {
    // SAFETY: engine globals owned by C.
    unsafe {
        let rd = ptr::addr_of_mut!(r_refdef);
        let time = ptr::addr_of!(cl.time).read();
        let idlescale = cvar_value(ptr::addr_of!(g::v_idlescale)) as f64;

        let roll = ptr::addr_of_mut!((*rd).viewangles[ROLL]);
        roll.write(
            (roll.read() as f64
                + idlescale
                    * c::libm::sin(time * cvar_value(ptr::addr_of!(g::v_iroll_cycle)) as f64)
                    * cvar_value(ptr::addr_of!(g::v_iroll_level)) as f64) as c_float,
        );
        let pitch = ptr::addr_of_mut!((*rd).viewangles[PITCH]);
        pitch.write(
            (pitch.read() as f64
                + idlescale
                    * c::libm::sin(time * cvar_value(ptr::addr_of!(g::v_ipitch_cycle)) as f64)
                    * cvar_value(ptr::addr_of!(g::v_ipitch_level)) as f64) as c_float,
        );
        let yaw = ptr::addr_of_mut!((*rd).viewangles[YAW]);
        yaw.write(
            (yaw.read() as f64
                + idlescale
                    * c::libm::sin(time * cvar_value(ptr::addr_of!(g::v_iyaw_cycle)) as f64)
                    * cvar_value(ptr::addr_of!(g::v_iyaw_level)) as f64) as c_float,
        );
    }
}

// ---------------------------------------------------------------------------
// view.c:648 -- V_CalcViewRoll

/// # Safety
/// `cl` and `r_refdef` must be live.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_v_calc_view_roll() {
    // SAFETY: engine globals owned by C.
    unsafe { v_calc_view_roll() }
}

unsafe fn v_calc_view_roll() {
    // SAFETY: engine globals owned by C.
    unsafe {
        let clp = ptr::addr_of_mut!(cl);
        let rd = ptr::addr_of_mut!(r_refdef);

        let ent = cl_entity(ptr::addr_of!((*clp).viewentity).read());
        let angles = ptr::addr_of!((*ent).angles).read();
        let velocity = ptr::addr_of!((*clp).velocity).read();
        let side = v_calc_roll(&angles, &velocity);

        let roll = ptr::addr_of_mut!((*rd).viewangles[ROLL]);
        roll.write(roll.read() + side);

        if ptr::addr_of!((*clp).v_dmg_time).read() > 0.0 {
            let dmg_time = ptr::addr_of!((*clp).v_dmg_time).read();
            let kicktime = cvar_value(ptr::addr_of!(g::v_kicktime));
            roll.write(roll.read() + dmg_time / kicktime * ptr::addr_of!((*clp).v_dmg_roll).read());
            let pitch = ptr::addr_of_mut!((*rd).viewangles[PITCH]);
            pitch.write(
                pitch.read() + dmg_time / kicktime * ptr::addr_of!((*clp).v_dmg_pitch).read(),
            );
            ptr::addr_of_mut!((*clp).v_dmg_time)
                .write((dmg_time as f64 - host_frametime()) as c_float);
        }

        if ptr::addr_of!((*clp).stats[STAT_HEALTH]).read() <= 0 {
            roll.write(80.0); // dead view angle
        }
    }
}

// ---------------------------------------------------------------------------
// view.c:677 -- V_CalcIntermissionRefdef

/// # Safety
/// `cl` and `r_refdef` must be live.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_v_calc_intermission_refdef() {
    // SAFETY: engine globals owned by C.
    unsafe { v_calc_intermission_refdef() }
}

unsafe fn v_calc_intermission_refdef() {
    // SAFETY: engine globals owned by C.
    unsafe {
        let clp = ptr::addr_of_mut!(cl);
        let rd = ptr::addr_of_mut!(r_refdef);

        // ent is the player model (visible when out of body)
        let ent = cl_entity(ptr::addr_of!((*clp).viewentity).read());
        // view is the weapon model (only visible from inside body)
        let view = cl_viewent();

        ptr::addr_of_mut!((*rd).vieworg).write(ptr::addr_of!((*ent).origin).read());
        ptr::addr_of_mut!((*rd).viewangles).write(ptr::addr_of!((*ent).angles).read());
        ptr::addr_of_mut!((*view).model).write(ptr::null_mut());
        invalidate_trace_line_cache();

        // allways idle in intermission
        let idlescale = ptr::addr_of_mut!(g::v_idlescale);
        let old = ptr::addr_of!((*idlescale).value).read();
        ptr::addr_of_mut!((*idlescale).value).write(1.0);
        v_add_idle();
        ptr::addr_of_mut!((*idlescale).value).write(old);
    }
}

// ---------------------------------------------------------------------------
// view.c:709 -- V_CalcRefdef

/// # Safety
/// `cl` and `r_refdef` must be live; `cl.worldmodel` must be a brush model
/// when `chase_active` is set.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_v_calc_refdef() {
    // SAFETY: engine globals owned by C.
    unsafe { v_calc_refdef() }
}

unsafe fn v_calc_refdef() {
    // SAFETY: engine globals owned by C.
    unsafe {
        let clp = ptr::addr_of_mut!(cl);
        let rd = ptr::addr_of_mut!(r_refdef);

        v_drift_pitch();

        // ent is the player model (visible when out of body)
        let ent = cl_entity(ptr::addr_of!((*clp).viewentity).read());
        // view is the weapon model (only visible from inside body)
        let view = cl_viewent();

        // transform the view offset by the model's matrix to get the offset from
        // model origin for the view
        let cl_viewangles = ptr::addr_of!((*clp).viewangles).read();
        // the model should face the view dir
        ptr::addr_of_mut!((*ent).angles[YAW]).write(cl_viewangles[YAW]);
        ptr::addr_of_mut!((*ent).angles[PITCH]).write(-cl_viewangles[PITCH]);

        let bob = v_calc_bob();

        // refresh position
        let ent_origin = ptr::addr_of!((*ent).origin).read();
        ptr::addr_of_mut!((*rd).vieworg).write(ent_origin);
        let viewheight = ptr::addr_of!((*clp).stats[STAT_VIEWHEIGHT]).read();
        let vo2 = ptr::addr_of_mut!((*rd).vieworg[2]);
        vo2.write(vo2.read() + (viewheight as c_float + bob));

        // never let it sit exactly on a node line, because a water plane can
        // dissapear when viewed with the eye exactly on it.
        // the server protocol only specifies to 1/16 pixel, so add 1/32 in each axis
        for i in 0..3 {
            let p = ptr::addr_of_mut!((*rd).vieworg[i]);
            p.write((p.read() as f64 + 1.0 / 32.0) as c_float);
        }

        ptr::addr_of_mut!((*rd).viewangles).write(cl_viewangles);
        v_calc_view_roll();
        v_add_idle();

        // offsets
        let mut angles: Vec3 = [0.0; 3];
        // because entity pitches are actually backward
        angles[PITCH] = -ptr::addr_of!((*ent).angles[PITCH]).read();
        angles[YAW] = ptr::addr_of!((*ent).angles[YAW]).read();
        angles[ROLL] = ptr::addr_of!((*ent).angles[ROLL]).read();

        let mut forward: Vec3 = [0.0; 3];
        let mut right: Vec3 = [0.0; 3];
        let mut up: Vec3 = [0.0; 3];
        m::angle_vectors(&angles, &mut forward, &mut right, &mut up);

        // johnfitz -- moved cheat-protection here from V_RenderView
        if ptr::addr_of!((*clp).maxclients).read() <= 1 {
            let ofsx = cvar_value(ptr::addr_of!(g::scr_ofsx));
            let ofsy = cvar_value(ptr::addr_of!(g::scr_ofsy));
            let ofsz = cvar_value(ptr::addr_of!(g::scr_ofsz));
            for i in 0..3 {
                let p = ptr::addr_of_mut!((*rd).vieworg[i]);
                p.write(p.read() + (ofsx * forward[i] + ofsy * right[i] + ofsz * up[i]));
            }
        }

        v_bound_offsets();

        // set up gun position
        ptr::addr_of_mut!((*view).angles).write(cl_viewangles);

        calc_gun_angle();

        let ent_origin = ptr::addr_of!((*ent).origin).read();
        ptr::addr_of_mut!((*view).origin).write(ent_origin);
        let vp2 = ptr::addr_of_mut!((*view).origin[2]);
        vp2.write(vp2.read() + viewheight as c_float);

        #[allow(clippy::needless_range_loop)] // indexes view->origin too
        for i in 0..3 {
            let p = ptr::addr_of_mut!((*view).origin[i]);
            // COMPAT: ADR-010 -- `0.4` is a double literal, so the product is
            // computed in double and narrowed on the compound assignment.
            p.write((p.read() as f64 + (forward[i] * bob) as f64 * 0.4) as c_float);
        }
        vp2.write(vp2.read() + bob);

        // johnfitz -- removed all gun position fudging code (was used to keep gun from getting covered by sbar)
        // MarkV -- restored this with r_viewmodel_quake cvar
        if cvar_value(ptr::addr_of!(g::r_viewmodel_quake)) != 0.0 {
            let viewsize = cvar_value(ptr::addr_of!(g::scr_viewsize));
            if viewsize == 110.0 {
                vp2.write(vp2.read() + 1.0);
            } else if viewsize == 100.0 {
                vp2.write(vp2.read() + 2.0);
            } else if viewsize == 90.0 {
                vp2.write(vp2.read() + 1.0);
            } else if viewsize == 80.0 {
                vp2.write(vp2.read() + 0.5);
            }
        }

        let ent_finish = ptr::addr_of!((*ent).lerp.frame_finish_time).read();
        ptr::addr_of_mut!((*view).lerp.frame_finish_time).write(ent_finish);

        // the weapon's frames come from stats, so its change detection lives here
        // ericw -- model check is done after the upper 8 bits of cl.stats[STAT_WEAPON] are filled in (broke on large maps like zendar.bsp)
        let weapon = ptr::addr_of!((*clp).stats[STAT_WEAPON]).read();
        let weaponframe = ptr::addr_of!((*clp).stats[STAT_WEAPONFRAME]).read();
        let precached = ptr::addr_of!((*clp).model_precache[weapon as usize]).read();

        if ptr::addr_of!((*view).model).read() != precached {
            // don't lerp animation across model changes
            ptr::addr_of_mut!((*view).frame).write(weaponframe);
            ptr::addr_of_mut!((*view).lerp.prev_frame).write(weaponframe);
            ptr::addr_of_mut!((*view).lerp.frame_change_time).write(0.0);
            ptr::addr_of_mut!((*view).lerp.snap_frames).write(0);
        } else if ptr::addr_of!((*view).frame).read() != weaponframe {
            let snap = ptr::addr_of!((*view).lerp.snap_frames).read();
            if snap > 0 {
                ptr::addr_of_mut!((*view).lerp.snap_frames).write(snap - 1);
                ptr::addr_of_mut!((*view).lerp.prev_frame).write(weaponframe);
            } else {
                let f = ptr::addr_of!((*view).frame).read();
                ptr::addr_of_mut!((*view).lerp.prev_frame).write(f);
            }
            ptr::addr_of_mut!((*view).frame).write(weaponframe);
            let mtime0 = ptr::addr_of!((*clp).mtime[0]).read();
            ptr::addr_of_mut!((*view).lerp.frame_change_time).write(mtime0);
            let finish = ptr::addr_of!((*view).lerp.frame_finish_time).read();
            ptr::addr_of_mut!((*view).lerp.frame_duration).write(if finish > mtime0 {
                finish - mtime0
            } else {
                0.1
            });
        }

        ptr::addr_of_mut!((*view).model).write(precached);
        ptr::addr_of_mut!((*view).netstate.colormap).write(0);

        // johnfitz -- v_gunkick
        let gunkick = cvar_value(ptr::addr_of!(g::v_gunkick));
        if gunkick == 1.0 {
            // original quake kick
            let punchangle = ptr::addr_of!((*clp).punchangle).read();
            #[allow(clippy::needless_range_loop)] // two parallel arrays
            for i in 0..3 {
                let p = ptr::addr_of_mut!((*rd).viewangles[i]);
                p.write(p.read() + punchangle[i]);
            }
        }
        if gunkick == 2.0 {
            // lerped kick
            #[allow(clippy::needless_range_loop)] // three parallel statics
            for i in 0..3 {
                let target = ptr::addr_of!(g::v_punchangles[0][i]).read();
                if ptr::addr_of!(REFDEF_PUNCH[i]).read() != target {
                    let mut interval = ptr::addr_of!(g::v_punchangles_times[0]).read()
                        - ptr::addr_of!(g::v_punchangles_times[1]).read();
                    if interval > 0.1 {
                        interval = 0.1;
                    }

                    // speed determined by how far we need to lerp in 1/10th of a second
                    let prev = ptr::addr_of!(g::v_punchangles[1][i]).read();
                    // COMPAT: ADR-010 -- the difference is float, then
                    // `* host_frametime / interval` runs in double before
                    // narrowing into the float `delta`.
                    let delta: c_float =
                        (((target - prev) as f64 * host_frametime()) / interval) as c_float;

                    let cur = ptr::addr_of!(REFDEF_PUNCH[i]).read();
                    if delta > 0.0 {
                        ptr::addr_of_mut!(REFDEF_PUNCH[i]).write(q_min_f(cur + delta, target));
                    } else if delta < 0.0 {
                        ptr::addr_of_mut!(REFDEF_PUNCH[i]).write(q_max_f(cur + delta, target));
                    }
                }
            }

            #[allow(clippy::needless_range_loop)] // two parallel arrays
            for i in 0..3 {
                let p = ptr::addr_of_mut!((*rd).viewangles[i]);
                p.write(p.read() + ptr::addr_of!(REFDEF_PUNCH[i]).read());
            }
        }
        // johnfitz

        // smooth out stair step ups
        // johnfitz -- added exception for noclip
        // FIXME: noclip_anglehack is set on the server, so in a nonlocal game this won't work.
        let ent_origin2 = ptr::addr_of!((*ent).origin[2]).read();
        let oldz_p = ptr::addr_of_mut!(REFDEF_OLDZ);
        if !ptr::addr_of!(g::noclip_anglehack).read()
            && ptr::addr_of!((*clp).onground).read()
            && ent_origin2 - oldz_p.read() > 0.0
        {
            // COMPAT: ADR-010 -- `cl.time - cl.oldtime` is a double
            // subtraction narrowed into the float `steptime`.
            let mut steptime: c_float = (ptr::addr_of!((*clp).time).read()
                - ptr::addr_of!((*clp).oldtime).read())
                as c_float;
            if steptime < 0.0 {
                // FIXME	I_Error ("steptime < 0");
                steptime = 0.0;
            }

            oldz_p.write(oldz_p.read() + steptime * 80.0);
            if oldz_p.read() > ent_origin2 {
                oldz_p.write(ent_origin2);
            }
            if ent_origin2 - oldz_p.read() > 12.0 {
                oldz_p.write(ent_origin2 - 12.0);
            }
            let delta = oldz_p.read() - ent_origin2;
            let vo2 = ptr::addr_of_mut!((*rd).vieworg[2]);
            vo2.write(vo2.read() + delta);
            let vp2 = ptr::addr_of_mut!((*view).origin[2]);
            vp2.write(vp2.read() + delta);
        } else {
            oldz_p.write(ent_origin2);
        }

        if cvar_value(ptr::addr_of!(gc::chase_active)) != 0.0 {
            crate::chase::chase_update_for_drawing(); // johnfitz
        }
    }
}

/// `q_minmax.h` `q_min_f`.
#[inline]
fn q_min_f(a: c_float, b: c_float) -> c_float {
    if a < b {
        a
    } else {
        b
    }
}

/// `q_minmax.h` `q_max_f`.
#[inline]
fn q_max_f(a: c_float, b: c_float) -> c_float {
    if a > b {
        a
    } else {
        b
    }
}

// ---------------------------------------------------------------------------
// view.c:871 -- V_RestoreAngles

/// # Safety
/// `cl` must be live.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_v_restore_angles() {
    // SAFETY: engine globals owned by C.
    unsafe {
        let ent = cl_entity(ptr::addr_of!(cl.viewentity).read());
        let msg = ptr::addr_of!((*ent).msg_angles[0]).read();
        ptr::addr_of_mut!((*ent).angles).write(msg);
    }
}

// ---------------------------------------------------------------------------
// view.c:882 -- V_SetupFrame

/// # Safety
/// `cl` and `r_refdef` must be live.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_v_setup_frame() {
    // SAFETY: engine globals owned by C.
    unsafe {
        v_update_blend();
        if !ptr::addr_of!(g::con_forcedup).read() {
            if ptr::addr_of!(cl.intermission).read() != 0 {
                v_calc_intermission_refdef();
            } else if !ptr::addr_of!(cl.paused).read()
            /* && (cl.maxclients > 1 || key_dest == key_game) */
            {
                v_calc_refdef();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// view.c:908 -- V_RenderView

/// # Safety
/// C ABI entry point; call only from `Quake/view_glue.c`'s `V_RenderView`,
/// which re-raises a non-zero return.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_v_render_view(
    use_tasks: qboolean,
    begin_rendering_task: u64,
    setup_frame_task: u64,
    draw_done_task: u64,
) -> Raise {
    // SAFETY: engine globals owned by C; both guarded calls re-enter C.
    unsafe {
        if ptr::addr_of!(g::con_forcedup).read() {
            ptr::addr_of_mut!(g::render_warp).write(false);
            ptr::addr_of_mut!(g::render_scale).write(1);
            return 0;
        }

        if ptr::addr_of!(g::needs_relink).read() {
            if use_tasks {
                // ADR-009: Sys_Error aborts, it does not longjmp, so no guard
                // is needed (world.rs / sv_phys.rs / sv_send.c precedent).
                c::Sys_Error(SYS_ERR_RENDER_VIEW_RELINK.as_ptr());
            }
            raise!(g::View_Glue_RelinkEntities());
        }

        raise!(g::View_Glue_RenderView(
            use_tasks,
            begin_rendering_task,
            setup_frame_task,
            draw_done_task
        ));
        0
    }
}

// ---------------------------------------------------------------------------
// view.c:940 -- V_Init

/// # Safety
/// C ABI entry point; call only from `Quake/view_glue.c`'s `V_Init`, which
/// re-raises a non-zero return.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_v_init() -> Raise {
    // SAFETY: the 30 cvars are C-owned statics in Quake/view_glue.c; each
    // trampoline only forwards an address into Cvar_RegisterVariable.
    unsafe {
        raise!(g::View_Glue_AddCommands());

        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::v_centermove
        )));
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::v_centerspeed
        )));

        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::v_iyaw_cycle
        )));
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::v_iroll_cycle
        )));
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::v_ipitch_cycle
        )));
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::v_iyaw_level
        )));
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::v_iroll_level
        )));
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::v_ipitch_level
        )));

        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::v_idlescale
        )));
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::crosshair
        )));
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::crosshair_def
        )));
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::gl_cshiftpercent
        )));
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::gl_cshiftpercent_contents
        ))); // QuakeSpasm
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::gl_cshiftpercent_damage
        ))); // QuakeSpasm
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::gl_cshiftpercent_bonus
        ))); // QuakeSpasm
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::gl_cshiftpercent_powerup
        ))); // QuakeSpasm

        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::scr_ofsx
        )));
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::scr_ofsy
        )));
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::scr_ofsz
        )));
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::cl_rollspeed
        )));
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::cl_rollangle
        )));
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(g::cl_bob)));
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::cl_bobcycle
        )));
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::cl_bobup
        )));

        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::v_kicktime
        )));
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::v_kickroll
        )));
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::v_kickpitch
        )));
        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::v_gunkick
        ))); // johnfitz

        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::v_autopitch
        )));

        raise!(g::View_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::r_viewmodel_quake
        ))); // MarkV

        0
    }
}
