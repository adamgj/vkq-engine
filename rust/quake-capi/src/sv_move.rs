//! C ABI shims for `Quake/sv_move.c` (Rust migration Phase 7 M4).
//!
//! Near-transliteration of the monster movement code: `SV_CheckBottom`'s
//! easy-out/realcheck ground test, `SV_movestep`'s swim/fly and step-down
//! logic, `SV_StepDirection`'s yaw turn plus move, `SV_NewChaseDir`'s
//! direction table and `SV_MoveToGoal`'s dispatch. `c_yes`/`c_no` were
//! `static` in C and are private here.
//!
//! ADR-009 audit. Every raising path in this file funnels through the
//! `world` module's `quake_rs_sv_move` and `quake_rs_sv_link_edict` cores
//! (both already status-returning per the frozen M3 contract), plus
//! `Quake/world_glue.c`'s `World_Glue_AssertFailed` for this file's three
//! `assert_always (!ent->free)` sites (`sv_move.c:119,257,261`).
//! `SV_PointContents` (called at `sv_move.c:58,145`) and `PF_changeyaw`
//! (`sv_move.c:242`, a leaf builtin under `USE_RUST_PROGS` -- see
//! `rust/quake-capi/src/progs_builtins.rs::quake_rs_pf_changeyaw`) are both
//! non-raising, so they are called directly. Five entry points are therefore
//! `quake_rs_*` status cores: `quake_rs_sv_check_bottom`,
//! `quake_rs_sv_movestep`, `quake_rs_sv_step_direction`,
//! `quake_rs_sv_new_chase_dir` and `quake_rs_sv_move_to_goal`. A non-zero
//! status is returned immediately by every intermediate function -- exactly
//! where C's `longjmp` would have left it, including skipping whatever
//! bookkeeping the abandoned C code would also have skipped -- so no jump
//! ever unwinds a Rust frame. `SV_FixCheckBottom` and `SV_CloseEnough` reach
//! nothing that can raise and are exported plain, matching the frozen M4
//! contract. Nothing in this module calls `Host_Reraise`;
//! `Quake/sv_move_glue.c` owns every re-raise.

use core::ffi::{c_float, c_int};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::sv_move as g;
use quake_c_sys::world as wg;
use quake_math::mathlib as m;
use quake_types::progs::{Edict, GlobalVars, QcVm, OFS_PARM0, OFS_RETURN};

/// Guard status carried back to `Quake/sv_move_glue.c`; 0 means "no raise".
type Raise = c_int;

// ---------------------------------------------------------------------------
// engine constants this module compares against

/// `bspfile.h` `CONTENTS_SOLID`
const CONTENTS_SOLID: c_int = -2;
/// `bspfile.h` `CONTENTS_EMPTY`
const CONTENTS_EMPTY: c_int = -1;

/// `world.h` `MOVE_NORMAL` -- `SV_Move`'s `type` argument.
const MOVE_NORMAL: c_int = 0;
/// `world.h` `MOVE_NOMONSTERS`
const MOVE_NOMONSTERS: c_int = 1;

/// `sv_move.c:26` `#define STEPSIZE 18` -- every use pairs it with a `float`
/// operand, never a `double`, so a plain `c_float` reproduces the C
/// promotion (int literal converted to the other operand's `float` type).
const STEPSIZE: c_float = 18.0;

/// `server.h` `eflags_t` `FL_FLY`
const FL_FLY: c_int = 1;
/// `server.h` `eflags_t` `FL_SWIM`
const FL_SWIM: c_int = 2;
/// `server.h` `eflags_t` `FL_ONGROUND`
const FL_ONGROUND: c_int = 512;
/// `server.h` `eflags_t` `FL_PARTIALGROUND`
const FL_PARTIALGROUND: c_int = 1024;

/// `quakedef.h` `YAW` angle-vector index.
const YAW: usize = 1;

/// `sv_move.c:285` `#define DI_NODIR -1` -- `d[]`/`olddir`/`turnaround` are
/// `float` locals, so the sentinel is a float literal, not a `c_int`.
const DI_NODIR: c_float = -1.0;

// ---------------------------------------------------------------------------
// file-private telemetry (sv_move.c:37 `static int c_yes, c_no;`)
//
// Never read back by the engine; kept only for parity with the C build.

static mut C_YES: c_int = 0;
static mut C_NO: c_int = 0;

// ---------------------------------------------------------------------------
// small helpers

/// `progs.h` `PROG_TO_EDICT (e)`
#[inline]
unsafe fn prog_to_edict(vm: *mut QcVm, p: c_int) -> *mut Edict {
    // SAFETY: pointer arithmetic only, byte-for-byte the C macro (which has
    // no bounds check either).
    unsafe {
        (*vm)
            .edicts
            .cast::<u8>()
            .wrapping_offset(p as isize)
            .cast::<Edict>()
    }
}

/// `progs.h` `EDICT_TO_PROG (e)`
#[inline]
unsafe fn edict_to_prog(vm: *mut QcVm, e: *mut Edict) -> c_int {
    // SAFETY: pointer arithmetic only, byte-for-byte the C macro (which has
    // no bounds check either).
    unsafe { e.cast::<u8>().offset_from((*vm).edicts.cast::<u8>()) as c_int }
}

/// `world.h` `trace_t`, zero-initialized (`memset (&trace, 0, sizeof
/// (trace_t))` at each of this file's `SV_Move` call sites). `world.rs`'s own
/// `Trace::zeroed()` is module-private, so this file carries its own copy.
#[inline]
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

const ASSERT_FILE: &core::ffi::CStr = c"sv_move.c";
const ASSERT_ENT_FREE: &core::ffi::CStr = c"!ent->free";

/// Runs `World_Glue_AssertFailed` for one of this file's three
/// `assert_always (!ent->free)` sites. `COM_Assert_Failed` never returns on
/// the main thread (it `Host_Error`s) and aborts on a worker, so a non-zero
/// result here is the only way this ever returns to a live caller.
#[inline]
unsafe fn assert_ent_not_free(ent: *mut Edict, line: c_int) -> Raise {
    // SAFETY: `ent` is the caller's live edict pointer; this only reads
    // `(*ent).free`.
    unsafe {
        if (*ent).free {
            wg::World_Glue_AssertFailed(ASSERT_ENT_FREE.as_ptr(), ASSERT_FILE.as_ptr(), line)
        } else {
            0
        }
    }
}

// ---------------------------------------------------------------------------
// sv_move.c:39-99 SV_CheckBottom

/// ADR-009 status core for `SV_CheckBottom`; `Quake/sv_move_glue.c`
/// re-raises.
///
/// # Safety
/// `ent` must be a live edict of the ambient qcvm; `out` must be a writable
/// bool slot.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_check_bottom(ent: *mut Edict, out: *mut bool) -> Raise {
    // SAFETY: ADR-008 ambient qcvm; nothing reachable from here dispatches
    // progs code (`SV_Move`/`SV_PointContents` are pure BSP queries).
    unsafe {
        *out = false;

        let origin = (*ent).v.origin;
        let entmins = (*ent).v.mins;
        let entmaxs = (*ent).v.maxs;
        let mut mins = [0.0f32; 3];
        let mut maxs = [0.0f32; 3];
        for i in 0..3 {
            mins[i] = origin[i] + entmins[i];
            maxs[i] = origin[i] + entmaxs[i];
        }

        let mut start = [0.0f32; 3];
        let mut stop = [0.0f32; 3];

        // if all of the points under the corners are solid world, don't
        // bother with the tougher checks -- the corners must be within 16 of
        // the midpoint (sv_move.c:49-60)
        start[2] = mins[2] - 1.0;
        let mut easy = true;
        'search: for x in 0..2 {
            for y in 0..2 {
                start[0] = if x != 0 { maxs[0] } else { mins[0] };
                start[1] = if y != 0 { maxs[1] } else { mins[1] };
                if crate::world::SV_PointContents(start.as_mut_ptr()) != CONTENTS_SOLID {
                    easy = false;
                    break 'search;
                }
            }
        }

        if easy {
            *ptr::addr_of_mut!(C_YES) += 1;
            *out = true;
            return 0; // we got out easy
        }

        // realcheck: check it for real... (sv_move.c:65-98)
        *ptr::addr_of_mut!(C_NO) += 1;

        start[2] = mins[2];

        // the midpoint must be within 16 of the bottom
        //
        // COMPAT: ADR-010 -- `0.5` is a double literal, so C promotes for the
        // multiply, but it adds `mins[i] + maxs[i]` in `float` first (both
        // operands already are `float`) and promotes only that rounded sum.
        // The add stays in `f32` here for that reason. Widening both operands
        // first is value-identical across the normal range -- a power-of-two
        // multiplier commutes with rounding -- but not when the `float` sum
        // overflows, where C yields an infinite midpoint and a `double` add
        // yields a finite one.
        start[0] = ((mins[0] + maxs[0]) as f64 * 0.5) as c_float;
        stop[0] = start[0];
        start[1] = ((mins[1] + maxs[1]) as f64 * 0.5) as c_float;
        stop[1] = start[1];
        stop[2] = start[2] - 2.0 * STEPSIZE;

        let origin_zero = ptr::addr_of_mut!(crate::mathlib::vec3_origin).cast::<c_float>();

        let mut trace = zero_trace();
        let raised = crate::world::quake_rs_sv_move(
            &mut trace,
            start.as_mut_ptr(),
            origin_zero,
            origin_zero,
            stop.as_mut_ptr(),
            MOVE_NOMONSTERS,
            ent,
        );
        if raised != 0 {
            return raised;
        }

        if trace.fraction == 1.0 {
            *out = false;
            return 0;
        }
        let mid = trace.endpos[2];
        let mut bottom = mid;

        // the corners must be within 16 of the midpoint
        for x in 0..2 {
            for y in 0..2 {
                start[0] = if x != 0 { maxs[0] } else { mins[0] };
                stop[0] = start[0];
                start[1] = if y != 0 { maxs[1] } else { mins[1] };
                stop[1] = start[1];

                let mut trace = zero_trace();
                let raised = crate::world::quake_rs_sv_move(
                    &mut trace,
                    start.as_mut_ptr(),
                    origin_zero,
                    origin_zero,
                    stop.as_mut_ptr(),
                    MOVE_NOMONSTERS,
                    ent,
                );
                if raised != 0 {
                    return raised;
                }

                if trace.fraction != 1.0 && trace.endpos[2] > bottom {
                    bottom = trace.endpos[2];
                }
                if trace.fraction == 1.0 || mid - trace.endpos[2] > STEPSIZE {
                    *out = false;
                    return 0;
                }
            }
        }

        let _ = bottom; // tracked only for the per-corner checks above, as in C

        *ptr::addr_of_mut!(C_YES) += 1;
        *out = true;
        0
    }
}

// ---------------------------------------------------------------------------
// sv_move.c:111-222 SV_movestep

/// ADR-009 status core for `SV_movestep`; `Quake/sv_move_glue.c` re-raises.
///
/// # Safety
/// `ent` must be a live edict of the ambient qcvm; `move_` must be a
/// `vec3_t`; `out` must be a writable bool slot.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_movestep(
    ent: *mut Edict,
    move_: *mut c_float,
    relink: bool,
    out: *mut bool,
) -> Raise {
    // SAFETY: ADR-008 ambient qcvm; see the module doc for why a raw `ent`
    // pointer stays valid across the `SV_LinkEdict` calls below exactly as it
    // does in the C original.
    unsafe {
        *out = false;

        // sv_move.c:119 `assert_always (!ent->free);`
        let raised = assert_ent_not_free(ent, 119);
        if raised != 0 {
            return raised;
        }

        let move3 = [*move_, *move_.add(1), *move_.add(2)];
        let oldorg = (*ent).v.origin;
        let mut neworg = [
            oldorg[0] + move3[0],
            oldorg[1] + move3[1],
            oldorg[2] + move3[2],
        ];

        // flying monsters don't step up
        if (*ent).v.flags as c_int & (FL_SWIM | FL_FLY) != 0 {
            let vm = c::qcvm.cast::<QcVm>();

            // try one move with vertical motion, then one without
            for i in 0..2 {
                let origin = (*ent).v.origin;
                neworg = [
                    origin[0] + move3[0],
                    origin[1] + move3[1],
                    origin[2] + move3[2],
                ];
                let enemy = prog_to_edict(vm, (*ent).v.enemy);
                if i == 0 && enemy != (*vm).edicts {
                    let dz = (*ent).v.origin[2] - (*enemy).v.origin[2];
                    if dz > 40.0 {
                        neworg[2] -= 8.0;
                    }
                    if dz < 30.0 {
                        neworg[2] += 8.0;
                    }
                }

                let mut trace = zero_trace();
                let raised = crate::world::quake_rs_sv_move(
                    &mut trace,
                    ptr::addr_of_mut!((*ent).v.origin).cast::<c_float>(),
                    ptr::addr_of_mut!((*ent).v.mins).cast::<c_float>(),
                    ptr::addr_of_mut!((*ent).v.maxs).cast::<c_float>(),
                    neworg.as_mut_ptr(),
                    MOVE_NORMAL,
                    ent,
                );
                if raised != 0 {
                    return raised;
                }

                if trace.fraction == 1.0 {
                    if (*ent).v.flags as c_int & FL_SWIM != 0 {
                        let mut endpos = trace.endpos;
                        if crate::world::SV_PointContents(endpos.as_mut_ptr()) == CONTENTS_EMPTY {
                            *out = false; // swim monster left water
                            return 0;
                        }
                    }

                    (*ent).v.origin = trace.endpos;
                    if relink {
                        let raised = crate::world::quake_rs_sv_link_edict(ent, true);
                        if raised != 0 {
                            return raised;
                        }
                    }
                    *out = true;
                    return 0;
                }

                if enemy == (*vm).edicts {
                    break;
                }
            }

            *out = false;
            return 0;
        }

        // push down from a step height above the wished position
        neworg[2] += STEPSIZE;
        let mut end = neworg;
        end[2] -= STEPSIZE * 2.0;

        let mut trace = zero_trace();
        let raised = crate::world::quake_rs_sv_move(
            &mut trace,
            neworg.as_mut_ptr(),
            ptr::addr_of_mut!((*ent).v.mins).cast::<c_float>(),
            ptr::addr_of_mut!((*ent).v.maxs).cast::<c_float>(),
            end.as_mut_ptr(),
            MOVE_NORMAL,
            ent,
        );
        if raised != 0 {
            return raised;
        }

        if trace.allsolid {
            *out = false;
            return 0;
        }

        if trace.startsolid {
            neworg[2] -= STEPSIZE;
            let mut trace2 = zero_trace();
            let raised = crate::world::quake_rs_sv_move(
                &mut trace2,
                neworg.as_mut_ptr(),
                ptr::addr_of_mut!((*ent).v.mins).cast::<c_float>(),
                ptr::addr_of_mut!((*ent).v.maxs).cast::<c_float>(),
                end.as_mut_ptr(),
                MOVE_NORMAL,
                ent,
            );
            if raised != 0 {
                return raised;
            }
            if trace2.allsolid || trace2.startsolid {
                *out = false;
                return 0;
            }
            trace = trace2;
        }

        if trace.fraction == 1.0 {
            // if monster had the ground pulled out, go ahead and fall
            if (*ent).v.flags as c_int & FL_PARTIALGROUND != 0 {
                let origin = (*ent).v.origin;
                (*ent).v.origin = [
                    origin[0] + move3[0],
                    origin[1] + move3[1],
                    origin[2] + move3[2],
                ];
                if relink {
                    let raised = crate::world::quake_rs_sv_link_edict(ent, true);
                    if raised != 0 {
                        return raised;
                    }
                }
                (*ent).v.flags = ((*ent).v.flags as c_int & !FL_ONGROUND) as c_float;
                *out = true;
                return 0;
            }

            *out = false; // walked off an edge
            return 0;
        }

        // check point traces down for dangling corners
        (*ent).v.origin = trace.endpos;

        let mut on_ground = false;
        let raised = quake_rs_sv_check_bottom(ent, &mut on_ground);
        if raised != 0 {
            return raised;
        }
        if !on_ground {
            // entity had floor mostly pulled out from underneath it and is
            // trying to correct
            if (*ent).v.flags as c_int & FL_PARTIALGROUND != 0 {
                if relink {
                    let raised = crate::world::quake_rs_sv_link_edict(ent, true);
                    if raised != 0 {
                        return raised;
                    }
                }
                *out = true;
                return 0;
            }
            (*ent).v.origin = oldorg;
            *out = false;
            return 0;
        }

        if (*ent).v.flags as c_int & FL_PARTIALGROUND != 0 {
            (*ent).v.flags = ((*ent).v.flags as c_int & !FL_PARTIALGROUND) as c_float;
        }
        if !trace.ent.is_null() {
            let vm = c::qcvm.cast::<QcVm>();
            (*ent).v.groundentity = edict_to_prog(vm, trace.ent);
        }

        // the move is ok
        if relink {
            let raised = crate::world::quake_rs_sv_link_edict(ent, true);
            if raised != 0 {
                return raised;
            }
        }
        *out = true;
        0
    }
}

// ---------------------------------------------------------------------------
// sv_move.c:236-264 SV_StepDirection

/// ADR-009 status core for `SV_StepDirection`; `Quake/sv_move_glue.c`
/// re-raises.
///
/// # Safety
/// `ent` must be a live edict of the ambient qcvm; `out` must be a writable
/// bool slot.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_step_direction(
    ent: *mut Edict,
    yaw: c_float,
    dist: c_float,
    out: *mut bool,
) -> Raise {
    // SAFETY: ADR-008 ambient qcvm.
    unsafe {
        *out = false;

        (*ent).v.ideal_yaw = yaw;
        g::PF_changeyaw();

        // COMPAT: ADR-010 -- `yaw = yaw * M_PI * 2 / 360` (sv_move.c:244)
        // evaluates in `double` (`M_PI` is a double macro) but assigns back
        // into the `float` parameter `yaw`, so the angle is narrowed to f32
        // before `cos`/`sin` promote it again. The narrowing is observable:
        // at 90 degrees C's cos sees 1.5707964f and yields -4.371139e-08,
        // where an unnarrowed f64 angle yields 6.1232e-17.
        let yaw_rad = f64::from((f64::from(yaw) * core::f64::consts::PI * 2.0 / 360.0) as c_float);

        // COMPAT: ADR-010 -- `cos (yaw) * dist` promotes `cos`'s `double`
        // result and `dist` together for the multiply, narrowed once on the
        // store into the `vec3_t`.
        let mut move_ = [
            (c::libm::cos(yaw_rad) * f64::from(dist)) as c_float,
            (c::libm::sin(yaw_rad) * f64::from(dist)) as c_float,
            0.0,
        ];

        let oldorigin = (*ent).v.origin;

        let mut stepped = false;
        let raised = quake_rs_sv_movestep(ent, move_.as_mut_ptr(), false, &mut stepped);
        if raised != 0 {
            return raised;
        }

        if stepped {
            let delta = (*ent).v.angles[YAW] - (*ent).v.ideal_yaw;
            if delta > 45.0 && delta < 315.0 {
                // not turned far enough, so don't take the step
                (*ent).v.origin = oldorigin;
            }
            // sv_move.c:257 `assert_always (!ent->free);`
            let raised = assert_ent_not_free(ent, 257);
            if raised != 0 {
                return raised;
            }
            let raised = crate::world::quake_rs_sv_link_edict(ent, true);
            if raised != 0 {
                return raised;
            }
            *out = true;
            return 0;
        }

        // sv_move.c:261 `assert_always (!ent->free);`
        let raised = assert_ent_not_free(ent, 261);
        if raised != 0 {
            return raised;
        }
        let raised = crate::world::quake_rs_sv_link_edict(ent, true);
        if raised != 0 {
            return raised;
        }
        *out = false;
        0
    }
}

// ---------------------------------------------------------------------------
// sv_move.c:272-277 SV_FixCheckBottom (non-raising)

/// # Safety
/// `ent` must be a live edict.
#[no_mangle]
pub unsafe extern "C" fn SV_FixCheckBottom(ent: *mut Edict) {
    // SAFETY: writes a single field of the caller's live edict.
    unsafe {
        (*ent).v.flags = ((*ent).v.flags as c_int | FL_PARTIALGROUND) as c_float;
    }
}

// ---------------------------------------------------------------------------
// sv_move.c:286-364 SV_NewChaseDir

/// ADR-009 status core for `SV_NewChaseDir`; `Quake/sv_move_glue.c`
/// re-raises.
///
/// # Safety
/// `actor` and `enemy` must be live edicts of the ambient qcvm.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_new_chase_dir(
    actor: *mut Edict,
    enemy: *mut Edict,
    dist: c_float,
) -> Raise {
    // SAFETY: ADR-008 ambient qcvm; see the module doc for raw-pointer
    // validity across `SV_StepDirection`'s `SV_LinkEdict` calls.
    unsafe {
        // sv_move.c:289 `float d[3]` uses only indices 1 and 2 in C; named
        // locals here instead of an array with a dead slot 0.
        // COMPAT: ADR-010 -- C's `(int)` truncates toward zero and is undefined
        // out of range; Rust's `as` saturates and maps NaN to 0. Left as `as`
        // deliberately: ADR-010 is per-platform, and on arm64 C lowers to
        // `fcvtzs`, which saturates and maps NaN to 0 exactly as Rust does, so
        // emulating x86-64's `cvttss2si` (NaN/out-of-range -> INT_MIN) would
        // break arm64 parity to fix x86-64's. `ideal_yaw` is mod-settable, so
        // the two disagree on x86-64 only for a NaN or |yaw| >= 45 * 2^31,
        // neither of which an `anglemod`-fed yaw reaches.
        let step = (((*actor).v.ideal_yaw / 45.0) as i32).wrapping_mul(45);
        let olddir = m::anglemod(step as c_float);
        let turnaround = m::anglemod(olddir - 180.0);

        let deltax = (*enemy).v.origin[0] - (*actor).v.origin[0];
        let deltay = (*enemy).v.origin[1] - (*actor).v.origin[1];

        let mut d1 = if deltax > 10.0 {
            0.0
        } else if deltax < -10.0 {
            180.0
        } else {
            DI_NODIR
        };
        let mut d2 = if deltay < -10.0 {
            270.0
        } else if deltay > 10.0 {
            90.0
        } else {
            DI_NODIR
        };

        // try direct route
        if d1 != DI_NODIR && d2 != DI_NODIR {
            let tdir = if d1 == 0.0 {
                if d2 == 90.0 {
                    45.0
                } else {
                    315.0
                }
            } else if d2 == 90.0 {
                135.0
            } else {
                215.0
            };

            if tdir != turnaround {
                let mut stepped = false;
                let raised = quake_rs_sv_step_direction(actor, tdir, dist, &mut stepped);
                if raised != 0 {
                    return raised;
                }
                if stepped {
                    return 0;
                }
            }
        }

        // try other directions -- ericw: explicit int cast to suppress clang
        // suggestion to use fabsf (sv_move.c:323)
        //
        // COMPAT: ADR-010 -- see the `ideal_yaw` cast above for why these stay
        // saturating `as`; the deltas are differences of entity origins,
        // bounded well inside `i32` for any coordinate a map or `setorigin`
        // produces.
        if (c::COM_Rand() & 3) & 1 != 0
            || (deltay as i32).wrapping_abs() > (deltax as i32).wrapping_abs()
        {
            core::mem::swap(&mut d1, &mut d2);
        }

        if d1 != DI_NODIR && d1 != turnaround {
            let mut stepped = false;
            let raised = quake_rs_sv_step_direction(actor, d1, dist, &mut stepped);
            if raised != 0 {
                return raised;
            }
            if stepped {
                return 0;
            }
        }

        if d2 != DI_NODIR && d2 != turnaround {
            let mut stepped = false;
            let raised = quake_rs_sv_step_direction(actor, d2, dist, &mut stepped);
            if raised != 0 {
                return raised;
            }
            if stepped {
                return 0;
            }
        }

        // there is no direct path to the player, so pick another direction
        if olddir != DI_NODIR {
            let mut stepped = false;
            let raised = quake_rs_sv_step_direction(actor, olddir, dist, &mut stepped);
            if raised != 0 {
                return raised;
            }
            if stepped {
                return 0;
            }
        }

        if c::COM_Rand() & 1 != 0 {
            // randomly determine direction of search
            let mut tdir = 0.0f32;
            while tdir <= 315.0 {
                if tdir != turnaround {
                    let mut stepped = false;
                    let raised = quake_rs_sv_step_direction(actor, tdir, dist, &mut stepped);
                    if raised != 0 {
                        return raised;
                    }
                    if stepped {
                        return 0;
                    }
                }
                tdir += 45.0;
            }
        } else {
            let mut tdir = 315.0f32;
            while tdir >= 0.0 {
                if tdir != turnaround {
                    let mut stepped = false;
                    let raised = quake_rs_sv_step_direction(actor, tdir, dist, &mut stepped);
                    if raised != 0 {
                        return raised;
                    }
                    if stepped {
                        return 0;
                    }
                }
                tdir -= 45.0;
            }
        }

        if turnaround != DI_NODIR {
            let mut stepped = false;
            let raised = quake_rs_sv_step_direction(actor, turnaround, dist, &mut stepped);
            if raised != 0 {
                return raised;
            }
            if stepped {
                return 0;
            }
        }

        (*actor).v.ideal_yaw = olddir; // can't move

        // if a bridge was pulled out from underneath a monster, it may not
        // have a valid standing position at all
        let mut on_ground = false;
        let raised = quake_rs_sv_check_bottom(actor, &mut on_ground);
        if raised != 0 {
            return raised;
        }
        if !on_ground {
            SV_FixCheckBottom(actor);
        }

        0
    }
}

// ---------------------------------------------------------------------------
// sv_move.c:372-384 SV_CloseEnough (non-raising)

/// # Safety
/// `ent` and `goal` must be live edicts.
#[no_mangle]
pub unsafe extern "C" fn SV_CloseEnough(ent: *mut Edict, goal: *mut Edict, dist: c_float) -> bool {
    // SAFETY: reads only, from the caller's two live edicts.
    unsafe {
        for i in 0..3 {
            if (*goal).v.absmin[i] > (*ent).v.absmax[i] + dist {
                return false;
            }
            if (*goal).v.absmax[i] < (*ent).v.absmin[i] - dist {
                return false;
            }
        }
        true
    }
}

// ---------------------------------------------------------------------------
// sv_move.c:392-416 SV_MoveToGoal

/// ADR-009 status core for `SV_MoveToGoal`; `Quake/sv_move_glue.c`
/// re-raises.
///
/// # Safety
/// Must run with a valid ambient qcvm and `pr_global_struct->self` set to a
/// live edict with a valid `goalentity` -- the QuakeC builtin calling
/// convention this is dispatched under.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_move_to_goal() -> Raise {
    // SAFETY: ADR-008 ambient qcvm.
    unsafe {
        let vm = c::qcvm.cast::<QcVm>();
        let globals = (*vm).globals.cast::<GlobalVars>();

        let ent = prog_to_edict(vm, (*globals).self_);
        let goal = prog_to_edict(vm, (*ent).v.goalentity);
        let dist = *(*vm).globals.add(OFS_PARM0);

        if (*ent).v.flags as c_int & (FL_ONGROUND | FL_FLY | FL_SWIM) == 0 {
            *(*vm).globals.add(OFS_RETURN) = 0.0;
            return 0;
        }

        // if the next step hits the enemy, return immediately
        if prog_to_edict(vm, (*ent).v.enemy) != (*vm).edicts && SV_CloseEnough(ent, goal, dist) {
            return 0;
        }

        // bump around...
        if c::COM_Rand() & 3 == 1 {
            return quake_rs_sv_new_chase_dir(ent, goal, dist);
        }

        let mut stepped = false;
        let raised = quake_rs_sv_step_direction(ent, (*ent).v.ideal_yaw, dist, &mut stepped);
        if raised != 0 {
            return raised;
        }
        if !stepped {
            return quake_rs_sv_new_chase_dir(ent, goal, dist);
        }

        0
    }
}
