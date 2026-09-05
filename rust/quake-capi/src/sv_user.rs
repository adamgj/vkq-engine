//! `Quake/sv_user.c` -- server code for moving users (Rust migration Phase 7
//! M6, T6.4, Pattern A whole-file swap).
//!
//! ## ADR-009 raise-topology audit
//!
//! `sv_user.c` itself has zero direct raise sites (no `Host_Error`,
//! `Host_EndGame` or `Sys_Error`). Its whole ADR-009 surface is transitive:
//!
//! - `sv_set_ideal_pitch` / `sv_user_friction` call `SV_Move`
//!   (`world::quake_rs_sv_move`), which can raise.
//! - `sv_air_move` calls `sv_user_friction` and so inherits its raise.
//! - `sv_client_think` calls `sv_air_move` and so inherits its raise.
//! - `sv_read_client_message`'s `clc_stringcmd` case dispatches to
//!   `PR_ExecuteProgram`/`Cmd_ExecuteString` through the
//!   `SvUser_Glue_StringCmd` guard (`Quake/sv_user_glue.c`).
//! - `sv_run_clients` calls `sv_read_client_message`, `SV_DropClient`
//!   (through the `SvUser_Glue_DropClient` guard) and `sv_client_think`.
//!
//! `sv_accelerate`, `sv_air_accelerate`, `drop_punch_angle`, `sv_water_move`,
//! `sv_water_jump`, `sv_noclip_move` and `sv_read_client_move` are pure --
//! every call they make (`MSG_Read*`, libm, mathlib) is non-raising, so they
//! return `()`, not `Raise`.
//!
//! ## Finding: mirror-typed externs live here, not in `quake-c-sys`
//!
//! `m6-wave.md` #2 says Rust reaches `sv`/`svs`/`cls`/`host_client` through
//! externs "in quake-c-sys". `rust/quake-c-sys/Cargo.toml` has no
//! `[dependencies]` at all, so that crate cannot name
//! `quake_types::host::{Server, ServerStatic, ClientStatic, Client}`. This
//! module declares those four externs directly instead, since it already
//! depends on `quake-types` -- preserving the substantive mandate (direct
//! mirror field access, no per-field accessor functions) while deviating
//! from the literal crate placement. Reported as a finding, not a silent
//! workaround; T6.2/T6.5 face the identical constraint for the same four
//! symbols.
//!
//! ## Deliberate simplification: no module-scope `static mut` shadow state
//!
//! `Quake/sv_user.c` keeps `forward`/`right`/`up`/`cmd`/`origin`/`velocity`/
//! `onground`/`angles` as file-scope `static` variables. Auditing every use
//! site shows none of them is ever read by a call that did not originate,
//! this tick, from `SV_ClientThink` (no cross-frame or cross-client
//! persistence) -- `angles` in particular is written but never read outside
//! `SV_ClientThink` itself. This port threads the same values through plain
//! function parameters/locals instead of `static mut` globals: identical
//! values reach identical call sites, with a smaller unsafe surface and no
//! risk of stale cross-call state. This is a storage-mechanism change only;
//! every arithmetic operation below preserves the original's exact operand
//! order and float/double promotions.

use core::ffi::{c_float, c_int, c_uint, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::qboolean;
use quake_c_sys::sv_phys as sp;
use quake_c_sys::sv_user as cu;
use quake_math::mathlib as m;
use quake_math::mathlib::Vec3;
use quake_types::host::{Client, ClientStatic, UserCmd};
use quake_types::progs::{Edict, QcVm};

type Raise = c_int;

// `sv`/`svs` became Rust-owned storage at T6.6 (ADR-007 row closed), so they
// are a sibling module's items now rather than C externs.
use crate::sv_main::{sv, svs};

extern "C" {
    // `host_client` and `cls` stay permanent plain externs (host.c,
    // cl_main.c), unrelated to this wave, but need the ADR-011 mirror
    // types, which only this crate can name.
    pub static mut cls: ClientStatic;
    pub static mut host_client: *mut Client;
}

const MOVETYPE_NONE: c_float = 0.0;
const MOVETYPE_WALK_I: c_int = 3;
const MOVETYPE_NOCLIP: c_float = 8.0;
const FL_ONGROUND: c_int = 512;
const FL_WATERJUMP: c_int = 2048;
const MOVE_NOMONSTERS: c_int = 1;
/// `Quake/quakedef.h:68` -- "don't modify! Has to be 72.0".
const MAX_PHYSICS_FREQ: f64 = 72.0;
/// `Quake/quakedef.h:64`.
const ON_EPSILON: f64 = 0.1;
const MAX_FORWARD: usize = 6;
/// `Quake/keys.h:136` -- `key_game` is the first `keydest_t` enumerator.
const KEY_GAME: c_int = 0;
const CLC_NOP: c_int = 1;
const CLC_DISCONNECT: c_int = 2;
const CLC_MOVE: c_int = 3;
const CLC_STRINGCMD: c_int = 4;
const CLCDP_ACKFRAME: c_int = 50;
const PEXT2_PREDINFO: c_uint = 0x0000_0020;
const PROTOCOL_NETQUAKE: c_uint = 15;
const PITCH: usize = 0;
const YAW: usize = 1;
const ROLL: usize = 2;

const ZERO_USERCMD: UserCmd = UserCmd {
    servertime: 0.0,
    seconds: 0.0,
    viewangles: [0.0; 3],
    forwardmove: 0.0,
    sidemove: 0.0,
    upmove: 0.0,
    forwardmove_accumulator: 0.0,
    sidemove_accumulator: 0.0,
    upmove_accumulator: 0.0,
    buttons: 0,
    impulse: 0,
    sequence: 0,
    weapon: 0,
};

/// Casts an entvars float field the way C's `(int)ent->v.field` does.
/// COMPAT: ADR-010 -- C's float-to-int cast truncates toward zero and is UB
/// out of range; Rust's `as` truncates toward zero and saturates. Field
/// values here are always in-range game state, so the difference is
/// unobservable, but the cast direction itself must not be "reassociated"
/// away (see sv_air_move's movetype check, one of several superficially
/// similar comparisons that is deliberately int-cast where the others are
/// not).
#[inline]
fn as_int(x: c_float) -> c_int {
    x as c_int
}

/// Reads a C `cvar_t`'s `.value` without forming a reference to the static.
#[inline]
unsafe fn cvar_value(var: *const c::cvar_t) -> c_float {
    // SAFETY: caller guarantees `var` points at a live, initialized cvar_t
    // (every call site passes the address of a static cvar owned by the C
    // glue), so the field read is in-bounds and properly aligned.
    unsafe { ptr::addr_of!((*var).value).read() }
}

#[inline]
unsafe fn host_frametime() -> f64 {
    // SAFETY: `c::host_frametime` is a C global written once per frame on the
    // host's own thread before any Rust entry point runs; reading it here
    // never races and the static is always initialized.
    unsafe { ptr::addr_of!(c::host_frametime).read() }
}

#[inline]
unsafe fn qcvm_ptr() -> *mut QcVm {
    // SAFETY: ADR-008 ambient qcvm -- `c::qcvm` is loaded by the host before
    // any server-frame code runs, so the cast target is always a live VM.
    unsafe { c::qcvm.cast::<QcVm>() }
}

/// Reinterprets a raw 3-float pointer (always an entvars `vec3_t` field) as
/// a `Vec3` reference, matching every `float *` alias sv_user.c takes into
/// an edict's fields.
#[inline]
unsafe fn vec3_from_ptr<'a>(p: *mut c_float) -> &'a mut Vec3 {
    // SAFETY: caller guarantees `p` is a live entvars vec3_t field (a 3-float
    // array embedded in a valid edict), matching every `float *` alias
    // sv_user.c takes into an edict's fields; the reinterpretation as `Vec3`
    // preserves that layout exactly.
    unsafe { &mut *p.cast::<Vec3>() }
}

#[inline]
unsafe fn vec3_origin_ptr() -> *mut c_float {
    ptr::addr_of_mut!(crate::mathlib::vec3_origin).cast::<c_float>()
}

fn zero_trace() -> crate::world::Trace {
    crate::world::Trace {
        allsolid: false,
        startsolid: false,
        inopen: false,
        inwater: false,
        fraction: 0.0,
        endpos: [0.0; 3],
        plane: crate::world::PlaneT {
            normal: [0.0; 3],
            dist: 0.0,
        },
        ent: ptr::null_mut(),
        contents: 0,
    }
}

/* ---------------------------------------------------------------------------
 * sv_user.c:52 SV_SetIdealPitch
 */
fn sv_set_ideal_pitch() -> Raise {
    // SAFETY: called only on the host's own thread with `sv_player` valid
    // (ADR-008 ambient qcvm invariant), matching every original
    // SV_SetIdealPitch call site.
    unsafe {
        let ent = cu::sv_player.cast::<Edict>();

        if as_int((*ent).v.flags) & FL_ONGROUND == 0 {
            return 0;
        }

        // COMPAT: ADR-010 -- sv_user.c:64-66. `angleval` is computed in
        // double precision (M_PI is a double macro) then narrowed to float
        // *before* sin/cos see it; sin/cos themselves run in double and
        // narrow their results back to float.
        let yaw = (*ent).v.angles[YAW];
        let angleval = (yaw as f64 * core::f64::consts::PI * 2.0 / 360.0) as f32;
        let sinval = c::libm::sin(angleval as f64) as f32;
        let cosval = c::libm::cos(angleval as f64) as f32;

        let mut z = [0.0f32; MAX_FORWARD];
        let mut i = 0usize;
        while i < MAX_FORWARD {
            let mut top: Vec3 = [0.0; 3];
            let mut bottom: Vec3 = [0.0; 3];

            top[0] = (*ent).v.origin[0] + cosval * (i as f32 + 3.0) * 12.0;
            top[1] = (*ent).v.origin[1] + sinval * (i as f32 + 3.0) * 12.0;
            top[2] = (*ent).v.origin[2] + (*ent).v.view_ofs[2];

            bottom[0] = top[0];
            bottom[1] = top[1];
            bottom[2] = top[2] - 160.0;

            let mut tr = zero_trace();
            let r = crate::world::quake_rs_sv_move(
                &mut tr,
                top.as_mut_ptr(),
                vec3_origin_ptr(),
                vec3_origin_ptr(),
                bottom.as_mut_ptr(),
                MOVE_NOMONSTERS,
                ent,
            );
            if r != 0 {
                return r;
            }

            if tr.allsolid {
                return 0; // looking at a wall, leave ideal the way is was
            }
            if tr.fraction == 1.0 {
                return 0; // near a dropoff
            }

            z[i] = top[2] + tr.fraction * (bottom[2] - top[2]);
            i += 1;
        }

        let mut dir: c_int = 0;
        let mut steps: c_int = 0;
        for j in 1..i {
            // COMPAT: sv_user.c:92 -- `int step = z[j] - z[j-1];` truncates
            // the float difference toward zero before the ON_EPSILON
            // compares run in double.
            let step = (z[j] - z[j - 1]) as c_int;
            if (step as f64) > -ON_EPSILON && (step as f64) < ON_EPSILON {
                continue;
            }
            if dir != 0
                && (((step - dir) as f64) > ON_EPSILON || ((step - dir) as f64) < -ON_EPSILON)
            {
                return 0; // mixed changes
            }
            steps += 1;
            dir = step;
        }

        if dir == 0 {
            (*ent).v.idealpitch = 0.0;
            return 0;
        }
        if steps < 2 {
            return 0;
        }
        (*ent).v.idealpitch = (-dir) as f32 * cvar_value(ptr::addr_of!(cu::sv_idealpitchscale));
        0
    }
}

/* ---------------------------------------------------------------------------
 * sv_user.c:120 SV_UserFriction
 */
fn sv_user_friction(ent: *mut Edict) -> Raise {
    // SAFETY: `ent` is the live sv_player edict passed in by the caller
    // (ADR-008 ambient qcvm invariant); every field access below stays
    // within that edict's entvars.
    unsafe {
        let origin = ptr::addr_of_mut!((*ent).v.origin).cast::<c_float>();
        let velocity = ptr::addr_of_mut!((*ent).v.velocity).cast::<c_float>();
        let vel = vec3_from_ptr(velocity);
        let org = vec3_from_ptr(origin);

        // COMPAT: ADR-010 -- sv_user.c:130.
        let speed = c::libm::sqrt((vel[0] * vel[0] + vel[1] * vel[1]) as f64) as f32;
        if speed == 0.0 {
            return 0;
        }

        let mut start: Vec3 = [0.0; 3];
        let mut stop: Vec3 = [0.0; 3];
        start[0] = org[0] + vel[0] / speed * 16.0;
        stop[0] = start[0];
        start[1] = org[1] + vel[1] / speed * 16.0;
        stop[1] = start[1];
        start[2] = org[2] + (*ent).v.mins[2];
        stop[2] = start[2] - 34.0;

        let mut tr = zero_trace();
        let r = crate::world::quake_rs_sv_move(
            &mut tr,
            start.as_mut_ptr(),
            vec3_origin_ptr(),
            vec3_origin_ptr(),
            stop.as_mut_ptr(),
            MOVE_NOMONSTERS,
            ent,
        );
        if r != 0 {
            return r;
        }

        let friction: f32 = if tr.fraction == 1.0 {
            cvar_value(ptr::addr_of!(sp::sv_friction))
                * cvar_value(ptr::addr_of!(cu::sv_edgefriction))
        } else {
            cvar_value(ptr::addr_of!(sp::sv_friction))
        };

        // Apply friction, matching the canonical 72Hz decay for any frame
        // duration: exponential while speed is above stopspeed, then linear
        // below it (sv_user.c:149-182).
        let analytic = sp::sv_analyticphysics_frame;
        let tau = 1.0_f64 / MAX_PHYSICS_FREQ;
        let mut s = host_frametime() / tau;
        let r0 = 1.0 - (friction as f64) * tau;
        let mut ns = speed as f64;

        if !analytic || r0 <= 0.0 {
            let stopspeed = cvar_value(ptr::addr_of!(sp::sv_stopspeed));
            let control = if speed < stopspeed { stopspeed } else { speed };
            ns = (speed as f64) - host_frametime() * (control as f64) * (friction as f64);
        } else {
            let stopspeed = cvar_value(ptr::addr_of!(sp::sv_stopspeed));
            if ns >= stopspeed as f64 {
                // COMPAT: ADR-010 -- sv_user.c:167, :170.
                let k_cross = c::libm::log((stopspeed as f64) / ns) / c::libm::log(r0);
                if s <= k_cross {
                    ns *= c::libm::pow(r0, s);
                    s = 0.0;
                } else {
                    ns = stopspeed as f64;
                    s -= k_cross;
                }
            }
            ns -= s * tau * (friction as f64) * (stopspeed as f64);
        }

        let mut newspeed = ns as f32;
        if newspeed < 0.0 {
            newspeed = 0.0;
        }
        newspeed /= speed;

        vel[0] *= newspeed;
        vel[1] *= newspeed;
        vel[2] *= newspeed;
        0
    }
}

/* ---------------------------------------------------------------------------
 * sv_user.c:200 SV_Accelerate
 */
fn sv_accelerate(ent: *mut Edict, wishspeed: f32, wishdir: &Vec3) {
    // SAFETY: `ent` is a live edict supplied by the caller; the velocity
    // field is a valid entvars vec3_t for the duration of this call.
    unsafe {
        let velocity = ptr::addr_of_mut!((*ent).v.velocity).cast::<c_float>();
        let vel = vec3_from_ptr(velocity);

        let currentspeed = m::dot_product(vel, wishdir);
        let addspeed = wishspeed - currentspeed;
        if addspeed <= 0.0 {
            return;
        }
        // sv_user.c:209 -- value*host_frametime pairs first (both promote to
        // double immediately), *then* wishspeed; contrast sv_air_accelerate.
        let mut accelspeed = (cvar_value(ptr::addr_of!(cu::sv_accelerate)) as f64
            * host_frametime()
            * wishspeed as f64) as f32;
        if accelspeed > addspeed {
            accelspeed = addspeed;
        }

        for i in 0..3 {
            vel[i] += accelspeed * wishdir[i];
        }
    }
}

/* ---------------------------------------------------------------------------
 * sv_user.c:217 SV_AirAccelerate
 */
fn sv_air_accelerate(ent: *mut Edict, wishspeed: f32, wishveloc: &mut Vec3) {
    // SAFETY: `ent` is a live edict supplied by the caller; the velocity
    // field is a valid entvars vec3_t for the duration of this call.
    unsafe {
        let velocity = ptr::addr_of_mut!((*ent).v.velocity).cast::<c_float>();
        let vel = vec3_from_ptr(velocity);

        let mut wishspd = m::vector_normalize(wishveloc);
        if wishspd > 30.0 {
            wishspd = 30.0;
        }
        let currentspeed = m::dot_product(vel, wishveloc);
        let addspeed = wishspd - currentspeed;
        if addspeed <= 0.0 {
            return;
        }
        // COMPAT: sv_user.c:230 multiplies by the *parameter* `wishspeed`,
        // not the locally recomputed `wishspd` from VectorNormalize -- a
        // preserved quirk, adjacent to the dead `//accelspeed = ...` line at
        // :229 that hints a simpler form was once intended. Also note the
        // multiplication grouping: value*wishspeed pair first, in float
        // precision (both operands float, no promotion), *then* the double
        // multiply by host_frametime -- the opposite grouping from
        // sv_accelerate, where host_frametime pairs first.
        let vw = cvar_value(ptr::addr_of!(cu::sv_accelerate)) * wishspeed;
        let mut accelspeed = (vw as f64 * host_frametime()) as f32;
        if accelspeed > addspeed {
            accelspeed = addspeed;
        }

        for i in 0..3 {
            vel[i] += accelspeed * wishveloc[i];
        }
    }
}

/* ---------------------------------------------------------------------------
 * sv_user.c:238 DropPunchAngle
 */
fn drop_punch_angle(ent: *mut Edict) {
    // SAFETY: `ent` is a live edict supplied by the caller; punchangle is a
    // valid entvars vec3_t for the duration of this call.
    unsafe {
        let punchangle = &mut (*ent).v.punchangle;
        let mut len = m::vector_normalize(punchangle);
        len = (len as f64 - 10.0 * host_frametime()) as f32;
        if len < 0.0 {
            len = 0.0;
        }
        for c in punchangle.iter_mut() {
            *c *= len;
        }
    }
}

/* ---------------------------------------------------------------------------
 * sv_user.c:256 SV_WaterMove
 */
fn sv_water_move(ent: *mut Edict, cmd: &UserCmd) {
    // SAFETY: `ent` is the live sv_player edict; `cmd` is a valid caller-owned
    // usercmd_t snapshot. All field accesses stay within `ent`'s entvars.
    unsafe {
        let mut forward: Vec3 = [0.0; 3];
        let mut right: Vec3 = [0.0; 3];
        let mut up: Vec3 = [0.0; 3];
        m::angle_vectors(&(*ent).v.v_angle, &mut forward, &mut right, &mut up);

        let mut wishvel: Vec3 = [0.0; 3];
        for i in 0..3 {
            wishvel[i] = forward[i] * cmd.forwardmove + right[i] * cmd.sidemove;
        }
        if cmd.forwardmove == 0.0 && cmd.sidemove == 0.0 && cmd.upmove == 0.0 {
            wishvel[2] -= 60.0; // drift towards bottom
        } else {
            wishvel[2] += cmd.upmove;
        }

        let mut wishspeed = m::vector_length(&wishvel);
        let maxspeed = cvar_value(ptr::addr_of!(cu::sv_maxspeed));
        if wishspeed > maxspeed {
            let scale = maxspeed / wishspeed;
            for c in wishvel.iter_mut() {
                *c *= scale;
            }
            wishspeed = maxspeed;
        }
        wishspeed = (wishspeed as f64 * 0.7) as f32;

        let velocity = ptr::addr_of_mut!((*ent).v.velocity).cast::<c_float>();
        let vel = vec3_from_ptr(velocity);
        let speed = m::vector_length(vel);
        let newspeed: f32;
        if speed != 0.0 {
            let analytic = sp::sv_analyticphysics_frame;
            let tau = 1.0_f64 / MAX_PHYSICS_FREQ;
            let friction = cvar_value(ptr::addr_of!(sp::sv_friction));
            let r0 = 1.0 - (friction as f64) * tau;
            let mut ns;
            if !analytic || r0 <= 0.0 {
                ns = (speed as f64 - host_frametime() * (speed as f64) * (friction as f64)) as f32;
            } else {
                // COMPAT: ADR-010 -- sv_user.c:296.
                ns = (speed as f64 * c::libm::pow(r0, host_frametime() / tau)) as f32;
            }
            if ns < 0.0 {
                ns = 0.0;
            }
            newspeed = ns;
            let scale = newspeed / speed;
            for c in vel.iter_mut() {
                *c *= scale;
            }
        } else {
            newspeed = 0.0;
        }

        if wishspeed == 0.0 {
            return;
        }
        let addspeed = wishspeed - newspeed;
        if addspeed <= 0.0 {
            return;
        }

        m::vector_normalize(&mut wishvel);
        // Same grouping note as sv_air_accelerate: value*wishspeed pair
        // first in float precision, then the double multiply.
        let vw = cvar_value(ptr::addr_of!(cu::sv_accelerate)) * wishspeed;
        let mut accelspeed = (vw as f64 * host_frametime()) as f32;
        if accelspeed > addspeed {
            accelspeed = addspeed;
        }

        for i in 0..3 {
            vel[i] += accelspeed * wishvel[i];
        }
    }
}

/* ---------------------------------------------------------------------------
 * sv_user.c:323 SV_WaterJump
 */
fn sv_water_jump(ent: *mut Edict) {
    // SAFETY: `ent` is the live sv_player edict; every field access below
    // stays within that edict's entvars.
    unsafe {
        // COMPAT: ADR-010 -- `qcvm->time` is a double; the comparison
        // against the entvars float runs in double, per sv_phys.rs precedent.
        let qtime = (*qcvm_ptr()).time;
        if qtime > (*ent).v.teleport_time as f64 || (*ent).v.waterlevel == 0.0 {
            (*ent).v.flags = (as_int((*ent).v.flags) & !FL_WATERJUMP) as f32;
            (*ent).v.teleport_time = 0.0;
        }
        let movedir = (*ent).v.movedir;
        (*ent).v.velocity[0] = movedir[0];
        (*ent).v.velocity[1] = movedir[1];
    }
}

/* ---------------------------------------------------------------------------
 * sv_user.c:341 SV_NoclipMove -- johnfitz
 */
fn sv_noclip_move(ent: *mut Edict, cmd: &UserCmd) {
    // SAFETY: `ent` is the live sv_player edict; `cmd` is a valid caller-owned
    // usercmd_t snapshot. All field accesses stay within `ent`'s entvars.
    unsafe {
        let mut forward: Vec3 = [0.0; 3];
        let mut right: Vec3 = [0.0; 3];
        let mut up: Vec3 = [0.0; 3];
        m::angle_vectors(&(*ent).v.v_angle, &mut forward, &mut right, &mut up);

        let velocity = ptr::addr_of_mut!((*ent).v.velocity).cast::<c_float>();
        let vel = vec3_from_ptr(velocity);
        vel[0] = forward[0] * cmd.forwardmove + right[0] * cmd.sidemove;
        vel[1] = forward[1] * cmd.forwardmove + right[1] * cmd.sidemove;
        vel[2] = forward[2] * cmd.forwardmove + right[2] * cmd.sidemove;
        vel[2] += cmd.upmove * 2.0; // doubled to match running speed

        let maxspeed = cvar_value(ptr::addr_of!(cu::sv_maxspeed));
        if m::vector_length(vel) > maxspeed {
            m::vector_normalize(vel);
            for c in vel.iter_mut() {
                *c *= maxspeed;
            }
        }
    }
}

/* ---------------------------------------------------------------------------
 * sv_user.c:362 SV_AirMove
 */
fn sv_air_move(ent: *mut Edict, cmd: &UserCmd, onground: bool) -> Raise {
    // SAFETY: `ent` is the live sv_player edict; `cmd` is a valid caller-owned
    // usercmd_t snapshot. All field accesses stay within `ent`'s entvars.
    unsafe {
        let mut forward: Vec3 = [0.0; 3];
        let mut right: Vec3 = [0.0; 3];
        let mut up: Vec3 = [0.0; 3];
        m::angle_vectors(&(*ent).v.angles, &mut forward, &mut right, &mut up);

        let mut fmove = cmd.forwardmove;
        let smove = cmd.sidemove;

        // hack to not let you back into teleporter
        // COMPAT: ADR-010 -- `qcvm->time` is a double; see sv_water_jump.
        let qtime = (*qcvm_ptr()).time;
        if qtime < (*ent).v.teleport_time as f64 && fmove < 0.0 {
            fmove = 0.0;
        }

        let mut wishvel: Vec3 = [0.0; 3];
        for i in 0..3 {
            wishvel[i] = forward[i] * fmove + right[i] * smove;
        }

        // COMPAT: sv_user.c:381 -- this is the one movetype comparison in
        // the file that int-casts first; the other three below compare the
        // raw float. Preserved verbatim.
        if as_int((*ent).v.movetype) != MOVETYPE_WALK_I {
            wishvel[2] = cmd.upmove;
        } else {
            wishvel[2] = 0.0;
        }

        let mut wishdir = wishvel;
        let mut wishspeed = m::vector_normalize(&mut wishdir);
        let maxspeed = cvar_value(ptr::addr_of!(cu::sv_maxspeed));
        if wishspeed > maxspeed {
            let scale = maxspeed / wishspeed;
            for c in wishvel.iter_mut() {
                *c *= scale;
            }
            wishspeed = maxspeed;
        }

        if (*ent).v.movetype == MOVETYPE_NOCLIP {
            let velocity = ptr::addr_of_mut!((*ent).v.velocity).cast::<c_float>();
            *vec3_from_ptr(velocity) = wishvel;
        } else if onground {
            let r = sv_user_friction(ent);
            if r != 0 {
                return r;
            }
            sv_accelerate(ent, wishspeed, &wishdir);
        } else {
            // not on ground, so little effect on velocity
            sv_air_accelerate(ent, wishspeed, &mut wishvel);
        }
        0
    }
}

/* ---------------------------------------------------------------------------
 * sv_user.c:417 SV_ClientThink
 */
fn sv_client_think() -> Raise {
    // SAFETY: called only on the host's own thread with `sv_player` and
    // `host_client` valid (ADR-008 ambient qcvm invariant), matching every
    // original SV_ClientThink call site.
    unsafe {
        let ent = cu::sv_player.cast::<Edict>();

        if (*ent).v.movetype == MOVETYPE_NONE {
            return 0;
        }

        let onground = (as_int((*ent).v.flags) & FL_ONGROUND) != 0;

        drop_punch_angle(ent);

        // if dead, behave differently
        if (*ent).v.health <= 0.0 {
            return 0;
        }

        // angles: show 1/3 the pitch angle and all the roll angle
        let cmd = (*host_client).cmd;

        let mut v_angle: Vec3 = [0.0; 3];
        m::vector_add(&(*ent).v.v_angle, &(*ent).v.punchangle, &mut v_angle);

        let roll = cu::V_CalcRoll(
            ptr::addr_of_mut!((*ent).v.angles).cast::<c_float>(),
            ptr::addr_of_mut!((*ent).v.velocity).cast::<c_float>(),
        ) * 4.0;
        (*ent).v.angles[ROLL] = roll;
        if (*ent).v.fixangle == 0.0 {
            (*ent).v.angles[PITCH] = -v_angle[PITCH] / 3.0;
            (*ent).v.angles[YAW] = v_angle[YAW];
        }

        if (as_int((*ent).v.flags) & FL_WATERJUMP) != 0 {
            sv_water_jump(ent);
            return 0;
        }

        // walk -- johnfitz: alternate noclip
        if (*ent).v.movetype == MOVETYPE_NOCLIP
            && cvar_value(ptr::addr_of!(cu::sv_altnoclip)) != 0.0
        {
            sv_noclip_move(ent, &cmd);
            0
        } else if (*ent).v.waterlevel >= 2.0 && (*ent).v.movetype != MOVETYPE_NOCLIP {
            sv_water_move(ent, &cmd);
            0
        } else {
            sv_air_move(ent, &cmd, onground)
        }
    }
}

/* ---------------------------------------------------------------------------
 * sv_user.c:474 SV_ReadClientMove
 */
fn sv_read_client_move(move_: *mut UserCmd) {
    // SAFETY: `move_` is a valid caller-owned usercmd_t (always
    // `&mut host_client->cmd`); `host_client` is the live client set by the
    // caller before dispatch.
    unsafe {
        let hc = host_client;
        let mut angle: Vec3 = [0.0; 3];
        let mut sequence: c_int;
        let mut drop = false;

        if (*hc).protocol_pext2 & PEXT2_PREDINFO != 0 {
            let i = (cu::MSG_ReadShort() as u16) as i32;
            let seq_u = (((*hc).lastmovemessage as u32) & 0xffff_0000u32) | ((i as u32) & 0xffff);
            sequence = seq_u as i32;

            // tolerance of a few old frames, so we can have redundancy for packetloss
            if sequence.wrapping_add(0x100) < (*hc).lastmovemessage {
                sequence = sequence.wrapping_add(0x10000);
            }
            if sequence <= (*hc).lastmovemessage {
                drop = true;
            }
        } else {
            sequence = 0;
        }

        // read ping time
        // COMPAT: ADR-010 -- `qcvm->time` is a double; the subtraction runs
        // in double and narrows to the float `ping_times[]` slot.
        let ping = ((*qcvm_ptr()).time - cu::MSG_ReadFloat() as f64) as f32;
        let idx = ((*hc).num_pings as usize) % quake_types::host::NUM_PING_TIMES;
        (*hc).ping_times[idx] = ping;
        (*hc).num_pings = (*hc).num_pings.wrapping_add(1);

        for a in angle.iter_mut() {
            // preserve the exact short-circuit order: the ProQuake angle
            // hack read must not run unless sv.protocol is NETQUAKE.
            let use_narrow = sv.protocol == PROTOCOL_NETQUAKE
                && !cu::NET_QSocketGetProQuakeAngleHack(cls.netcon.cast())
                && ((*hc).protocol_pext2 & PEXT2_PREDINFO) == 0;
            *a = if use_narrow {
                cu::MSG_ReadAngle(sv.protocolflags)
            } else {
                cu::MSG_ReadAngle16(sv.protocolflags) // johnfitz -- 16-bit angles for PROTOCOL_FITZQUAKE
            };
        }
        let mut movevalues: Vec3 = [0.0; 3];
        movevalues[0] = cu::MSG_ReadShort() as f32;
        movevalues[1] = cu::MSG_ReadShort() as f32;
        movevalues[2] = cu::MSG_ReadShort() as f32;
        let buttonbits = cu::MSG_ReadByte();
        let newimpulse = cu::MSG_ReadByte();

        if drop {
            return; // okay, we don't care about that then
        }

        // calc ping times
        (*hc).lastmovemessage = sequence;

        // read movement
        (*(*hc).edict).v.v_angle = angle;
        (*move_).forwardmove = movevalues[0];
        (*move_).sidemove = movevalues[1];
        (*move_).upmove = movevalues[2];

        // read buttons
        // COMPAT: sv_user.c:529 -- `>> 0` is a genuine upstream no-op, kept
        // verbatim for structural symmetry with the `button2` line below.
        #[allow(clippy::identity_op)]
        {
            (*(*hc).edict).v.button0 = ((buttonbits & 1) >> 0) as f32;
        }
        // button1 was meant to be 'use', but got reused by too many mods to get implemented now
        (*(*hc).edict).v.button2 = ((buttonbits & 2) >> 1) as f32;

        if newimpulse != 0 {
            (*(*hc).edict).v.impulse = newimpulse as f32;
        }
    }
}

/* ---------------------------------------------------------------------------
 * sv_user.c:544 SV_ReadClientMessage -- Ok(bool) mirrors the qboolean
 * return (false = kill the client); Err propagates a caught raise.
 */
fn sv_read_client_message() -> Result<bool, Raise> {
    // SAFETY: called only from sv_run_clients with `host_client` set to the
    // client currently owning the just-received network message.
    unsafe {
        cu::MSG_BeginReading();

        loop {
            if !(*host_client).active {
                return Ok(false); // a command caused an error
            }
            if c::msg_badread {
                c::Sys_Printf(c"SV_ReadClientMessage: badread\n".as_ptr());
                return Ok(false);
            }

            let ccmd = cu::MSG_ReadChar();
            match ccmd {
                -1 => return Ok(true), // msg_badread, meaning we just hit eof.
                CLC_NOP => {}
                CLC_STRINGCMD => {
                    // MSG_ReadString cannot raise; the dispatch it feeds can.
                    let s = cu::MSG_ReadString();
                    let r = cu::SvUser_Glue_StringCmd(s);
                    if r != 0 {
                        return Err(r);
                    }
                }
                CLC_DISCONNECT => return Ok(false),
                CLC_MOVE => {
                    if !(*host_client).spawned {
                        // this is to suck up any stale moves on map changes
                        return Ok(true);
                    }
                    sv_read_client_move(ptr::addr_of_mut!((*host_client).cmd));
                }
                CLCDP_ACKFRAME => {
                    let seq = cu::MSG_ReadLong();
                    cu::SVFTE_Ack(host_client.cast::<c_void>(), seq);
                }
                _ => {
                    c::Sys_Printf(c"SV_ReadClientMessage: unknown command char\n".as_ptr());
                    return Ok(false);
                }
            }
        }
    }
}

/* ---------------------------------------------------------------------------
 * sv_user.c:618 SV_RunClients
 */
fn sv_run_clients() -> Raise {
    // SAFETY: called only on the host's own thread; `svs.clients` is the
    // live, fully-initialized client array for the duration of the server
    // frame (ADR-008 ambient qcvm invariant).
    unsafe {
        // receive from clients first
        loop {
            // ADR-009: guarded, not a bare extern call. The datagram
            // driver's QGetAnyMessage reaches SV_ConnectClient and
            // SV_DropClient beneath this, and both longjmp
            // (Quake/sv_user_glue.c documents the chain).
            let mut sock: *mut c_void = ptr::null_mut();
            let r = cu::SvUser_Glue_GetServerMessage(&mut sock);
            if r != 0 {
                return r;
            }
            if sock.is_null() {
                break; // no more this frame
            }

            let mut i: c_int = 0;
            host_client = svs.clients;
            while i < svs.maxclients {
                if (*host_client).netconnection == sock.cast() {
                    cu::sv_player = (*host_client).edict.cast::<c_void>();
                    match sv_read_client_message() {
                        Ok(true) => {}
                        Ok(false) => {
                            let r = cu::SvUser_Glue_DropClient(false as qboolean); // client misbehaved...
                            if r != 0 {
                                return r;
                            }
                            break;
                        }
                        Err(r) => return r,
                    }
                }
                i += 1;
                host_client = host_client.add(1);
            }
        }

        // then do the per-frame stuff
        let mut i: c_int = 0;
        host_client = svs.clients;
        while i < svs.maxclients {
            if !(*host_client).active {
                i += 1;
                host_client = host_client.add(1);
                continue;
            }

            cu::sv_player = (*host_client).edict.cast::<c_void>();

            if !(*host_client).spawned {
                // clear client movement until a new packet is received
                (*host_client).cmd = ZERO_USERCMD;
                i += 1;
                host_client = host_client.add(1);
                continue;
            }

            if (*host_client).netconnection.is_null() {
                (*host_client).cmd.viewangles = (*(*host_client).edict).v.v_angle;
            }

            // always pause in single player if in console or menus
            if !sv.paused && (svs.maxclients > 1 || cu::key_dest == KEY_GAME) {
                let r = sv_client_think();
                if r != 0 {
                    return r;
                }
            }

            i += 1;
            host_client = host_client.add(1);
        }
        0
    }
}

/* ---------------------------------------------------------------------------
 * Public entry points -- exact sv_user.c signatures; the C glue wrapper
 * (Quake/sv_user_glue.c) re-raises what these caught (ADR-009).
 */

/// `sv_user.c:52` `SV_SetIdealPitch`.
///
/// # Safety
/// Must be called only from the C glue wrapper (`Quake/sv_user_glue.c`) on
/// the host's own thread, with `sv_player` valid (ADR-008 ambient qcvm
/// invariant).
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_set_ideal_pitch() -> Raise {
    // SAFETY: called only from the C glue wrapper on the host's own thread,
    // with sv_player valid, matching the original SV_SetIdealPitch call
    // sites. sv_set_ideal_pitch is itself a safe fn (its unsafe body is
    // scoped internally), so no inner unsafe block is needed here.
    sv_set_ideal_pitch()
}

/// `sv_user.c:417` `SV_ClientThink`.
///
/// # Safety
/// See [`quake_rs_sv_set_ideal_pitch`]; additionally requires `host_client`
/// valid.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_client_think() -> Raise {
    // SAFETY: see quake_rs_sv_set_ideal_pitch.
    sv_client_think()
}

/// `sv_user.c:618` `SV_RunClients`.
///
/// # Safety
/// See [`quake_rs_sv_set_ideal_pitch`]; additionally requires `svs.clients`
/// valid for `svs.maxclients` entries.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_run_clients() -> Raise {
    // SAFETY: see quake_rs_sv_set_ideal_pitch.
    sv_run_clients()
}
