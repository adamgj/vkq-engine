//! Server-coupled QuakeC builtins (Rust migration Phase 7 M5, wave 1:
//! Group A link, Group B trace/movement, Group C PVS), plus the shared
//! plumbing the wave-2 modules (`progs_builtins_sv_msg`,
//! `progs_builtins_sv_fx`, `progs_builtins_cl`) reuse.
//!
//! These bodies come from `Quake/pr_cmds.c` and `Quake/pr_ext.c`, which stay
//! compiled as the oracle; the flip is Pattern C (one `builtin_t` table slot at
//! a time through `pr_cmds_glue.c`'s `RUST_PF` wrappers).
//!
//! # Why this module is `host`-gated, not `progs`-gated
//!
//! Every builtin here bottoms out in `world.c` / `sv_move.c` / `sv_phys.c`
//! cores that only exist under the `host` feature, and `-Duse_rust_progs` and
//! `-Duse_rust_host` are independent Meson options. The C table rows are
//! therefore gated on *both* (`PF_RSH` in `pr_cmds.c` / `pr_ext.c`), and the C
//! frame lives in `Quake/pr_cmds_sv_glue.c`, compiled with the `use_rust_host`
//! sources, so the module gate and the glue's compilation condition are
//! identical in every configuration.
//!
//! # ADR-009 audit
//!
//! Every raise reachable from this module is caught by a `Host_Guard` in
//! `pr_cmds_sv_glue.c` / `world_glue.c` and reported as `PRBI_ERR_GUARD` with
//! the guard status as `detail`; `pr_cmds_glue.c`'s `PRBI_Raise` re-issues it
//! from the C frame. The raising seams are:
//!
//! * `SV_LinkEdict` -> `SV_TouchLinks` (`quake_rs_sv_link_edict`),
//! * `SV_Move` (`quake_rs_sv_move`), `SV_CheckBottom`, `SV_movestep`,
//! * `SetMinMaxSize`'s `PR_RunError ("backwards mins/maxs")`,
//! * `PF_setmodel`'s `G_STRING` / `SV_Precache_Model` / `"no precache"`,
//! * the `traceline` / `tracebox` NAN warning's `NUM_FOR_EDICT`,
//! * `EDICT_NUM` in `PF_newcheckclient` / `PF_checkclient`.
//!
//! No plain reraising wrapper (`SV_LinkEdict`, `SV_Move`, ...) is called from
//! Rust; only the `quake_rs_*` status cores are.
//!
//! # ADR-006
//!
//! The only path here that dispatches QuakeC is `PF_walkmove` ->
//! `SV_movestep (ent, move, true)` -> `SV_LinkEdict (ent, true)` ->
//! `SV_TouchLinks`. Every other `SV_LinkEdict` call passes `false`. No Rust
//! reference is held across that call; the ambient qcvm and its globals are
//! re-derived afterwards.

use core::ffi::{c_char, c_float, c_int, c_uint, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::progs_builtins_sv as g;
use quake_c_sys::sv_phys as pg;
use quake_c_sys::world as wg;
use quake_types::model_mem::{MLeaf, QModel, MOD_BRUSH};
use quake_types::progs::{
    DFunction, Edict, GlobalVars, QcVm, OFS_PARM0, OFS_PARM1, OFS_PARM2, OFS_PARM3, OFS_PARM4,
    OFS_PARM5, OFS_RETURN,
};

use crate::world::{PlaneT, Trace};

/* ---------------------------------------------------------------------------
 * Shared plumbing (reused by the wave-2 modules).
 */

/// `pr_cmds_glue.c:353` `PRBI_OK`.
pub(crate) const PRBI_OK: c_int = 0;
/// `pr_cmds_glue.c:353` `PRBI_ERR_GUARD` -- a guarded seam's jump, replayed by
/// `PRBI_Raise` (ADR-009 rule 3). Wave 1 needs no other status code.
pub(crate) const PRBI_ERR_GUARD: c_int = 3;
/// `quakedef.h:475` `HOST_GUARD_OK`.
const HOST_GUARD_OK: c_int = 0;

/// A pending raise: the `PRBI_*` status `quake_rs_pf_*` returns plus the
/// `detail` word `RUST_PF` hands to `PRBI_Raise`.
pub(crate) struct SvRaise {
    pub(crate) status: c_int,
    pub(crate) detail: c_int,
}

pub(crate) type SvResult = Result<(), SvRaise>;

/// Turn a `Host_Guard` status into a result. Every guarded seam in this module
/// family goes through here so the status/detail convention stays in one place.
#[inline]
pub(crate) fn guarded(status: c_int) -> SvResult {
    if status == HOST_GUARD_OK {
        Ok(())
    } else {
        Err(SvRaise {
            status: PRBI_ERR_GUARD,
            detail: status,
        })
    }
}

/// Which console function a queued message goes to, mirroring
/// `progs_builtins.rs`'s `level`.
#[allow(dead_code)]
pub(crate) mod level {
    pub const PRINT: u8 = 0;
    pub const DPRINT: u8 = 1;
    pub const WARN: u8 = 2;
    pub const DWARN: u8 = 3;
}

/// Deferred console output, same contract as `progs_builtins.rs`'s
/// `EngineBuiltin`: `Con_Printf` is not a leaf, so a builtin queues its
/// messages and [`run_sv`] flushes them after the Rust frame has returned.
///
/// Wave 1 queues nothing -- all of its console output happens inside guarded C
/// helpers, where the format arguments (`NUM_FOR_EDICT`, `PR_GetString`) can
/// themselves raise. The wave-2 message/effect builtins are the users.
#[allow(dead_code)]
pub(crate) struct SvConsole {
    pending: Vec<(u8, Vec<u8>)>,
}

#[allow(dead_code)]
impl SvConsole {
    fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    pub(crate) fn print(&mut self, bytes: &[u8]) {
        self.pending.push((level::PRINT, bytes.to_vec()));
    }

    pub(crate) fn dprint(&mut self, bytes: &[u8]) {
        self.pending.push((level::DPRINT, bytes.to_vec()));
    }

    pub(crate) fn warn(&mut self, bytes: &[u8]) {
        self.pending.push((level::WARN, bytes.to_vec()));
    }

    pub(crate) fn dwarn(&mut self, bytes: &[u8]) {
        self.pending.push((level::DWARN, bytes.to_vec()));
    }

    fn flush(&mut self) {
        for (lvl, mut msg) in core::mem::take(&mut self.pending) {
            msg.retain(|&b| b != 0);
            msg.push(0);
            // SAFETY: NUL-terminated, and every one of these takes a plain
            // `%s` so progs bytes reach the console unmodified.
            unsafe {
                match lvl {
                    level::PRINT => c::Con_Printf(c"%s".as_ptr(), msg.as_ptr()),
                    level::DPRINT => c::Con_DPrintf(c"%s".as_ptr(), msg.as_ptr()),
                    level::WARN => c::Con_Warning(c"%s".as_ptr(), msg.as_ptr()),
                    _ => c::Con_DWarning(c"%s".as_ptr(), msg.as_ptr()),
                }
            }
        }
    }
}

/// Run one server builtin: take the ambient-VM view (ADR-008), defer console
/// output, and turn the result into the `PRBI_*` status `RUST_PF` expects.
///
/// # Safety
/// `detail` must be the `int *` `pr_cmds_glue.c`'s `RUST_PF` macro passes.
pub(crate) unsafe fn run_sv(
    detail: *mut c_int,
    f: impl FnOnce(*mut QcVm, &mut SvConsole) -> SvResult,
) -> c_int {
    let mut con = SvConsole::new();
    // SAFETY: ADR-008 -- a builtin only executes inside PR_ExecuteProgram, so
    // the ambient qcvm is live for the whole call.
    let vm = unsafe { c::qcvm.cast::<QcVm>() };
    let result = f(vm, &mut con);
    con.flush();
    match result {
        Ok(()) => PRBI_OK,
        Err(e) => {
            // SAFETY: caller contract.
            unsafe { *detail = e.detail };
            e.status
        }
    }
}

/* ---------------------------------------------------------------------------
 * Constants (server.h / progs.h / mathlib.h), duplicated locally the way
 * world.rs and sv_move.rs do rather than pulled through a header mirror.
 */

/// `server.h:244`
const SOLID_NOT: c_float = 0.0;
/// `server.h:265`
const DAMAGE_AIM: c_float = 2.0;
/// `server.h:271-272`, `:279`, `:283`
const FL_FLY: c_int = 1;
const FL_SWIM: c_int = 2;
const FL_NOTARGET: c_int = 128;
const FL_ONGROUND: c_int = 512;
/// `server.h` `MOVE_NORMAL` -- `SV_Move`'s `false` argument at every call site
/// in this module.
const MOVE_NORMAL: c_int = 0;
/// `mathlib.h:45` `nanmask`
const NANMASK: u32 = 255 << 23;

/// `memset (&trace, 0, sizeof (trace_t))` -- `world.rs`'s `Trace::zeroed` is
/// module-private, so the same zero value is spelled out here.
#[inline]
fn zero_trace() -> Trace {
    Trace {
        allsolid: false,
        startsolid: false,
        inopen: false,
        inwater: false,
        fraction: 0.0,
        endpos: [0.0; 3],
        plane: PlaneT {
            normal: [0.0; 3],
            dist: 0.0,
        },
        ent: ptr::null_mut(),
        contents: 0,
    }
}

/* ---------------------------------------------------------------------------
 * progs.h macro equivalents.
 */

#[inline]
unsafe fn globals(vm: *mut QcVm) -> *mut c_float {
    // SAFETY: the ambient qcvm is live for the duration of a builtin.
    unsafe { (*vm).globals }
}

#[inline]
unsafe fn gvars(vm: *mut QcVm) -> *mut GlobalVars {
    // SAFETY: `pr_global_struct` is the same storage as `qcvm->globals`.
    unsafe { (*vm).globals.cast::<GlobalVars>() }
}

/// `progs.h` `G_FLOAT (o)`
#[inline]
unsafe fn g_float(vm: *mut QcVm, ofs: usize) -> c_float {
    // SAFETY: `ofs` is a fixed OFS_* slot inside the globals block.
    unsafe { *globals(vm).add(ofs) }
}

/// `progs.h` `G_INT (o)`
#[inline]
unsafe fn g_int(vm: *mut QcVm, ofs: usize) -> c_int {
    // SAFETY: as `g_float`; the C macro type-puns the same word.
    unsafe { *globals(vm).add(ofs).cast::<c_int>() }
}

/// `progs.h` `G_VECTOR (o)`
#[inline]
unsafe fn g_vector(vm: *mut QcVm, ofs: usize) -> *mut c_float {
    // SAFETY: as `g_float`.
    unsafe { globals(vm).add(ofs) }
}

/// `progs.h` `PROG_TO_EDICT (e)` -- byte offset, no bounds check, exactly like
/// the C macro.
#[inline]
unsafe fn prog_to_edict(vm: *mut QcVm, p: c_int) -> *mut Edict {
    // SAFETY: pointer arithmetic only.
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
    // SAFETY: both pointers are into the edict arena (ADR-006).
    unsafe { (e.cast::<u8>() as isize - (*vm).edicts.cast::<u8>() as isize) as c_int }
}

/// `progs.h` `G_EDICT (o)`
#[inline]
unsafe fn g_edict(vm: *mut QcVm, ofs: usize) -> *mut Edict {
    // SAFETY: as `g_int`.
    unsafe { prog_to_edict(vm, g_int(vm, ofs)) }
}

/// `progs.h` `RETURN_EDICT (e)`
#[inline]
unsafe fn return_edict(vm: *mut QcVm, e: *mut Edict) {
    // SAFETY: OFS_RETURN is inside the globals block.
    unsafe { *globals(vm).add(OFS_RETURN).cast::<c_int>() = edict_to_prog(vm, e) }
}

/// The `i`-th edict. The `NEXT_EDICT` walk is expressed as an index stride so a
/// `continue` cannot skip the increment the C `for` header performs
/// (`qcvm->edict_size`, ADR-006).
#[inline]
unsafe fn edict_at(vm: *mut QcVm, i: c_int) -> *mut Edict {
    // SAFETY: callers keep `i` below `qcvm->num_edicts`.
    unsafe {
        (*vm)
            .edicts
            .cast::<u8>()
            .wrapping_offset((i as isize) * ((*vm).edict_size as isize))
            .cast::<Edict>()
    }
}

/// `qcvm->worldmodel` as the ADR-011 mirror.
#[inline]
unsafe fn worldmodel(vm: *mut QcVm) -> *mut QModel {
    // SAFETY: `worldmodel` is an opaque `qmodel_t *` in the mirror.
    unsafe { (*vm).worldmodel.cast::<QModel>() }
}

// COMPAT: ADR-010 -- `mathlib.h:45` `IS_NAN`, which tests only the exponent and
// therefore reports true for infinities as well. `f32::is_nan` would not.
#[inline]
fn is_nan(x: c_float) -> bool {
    (x.to_bits() & NANMASK) == NANMASK
}

// COMPAT: ADR-010 -- C's implicit float->int conversion. Out-of-range values are
// UB in C and saturate in Rust; the same shim sv_phys.rs uses.
#[inline]
fn as_int(x: c_float) -> c_int {
    x as c_int
}

/// A C `qboolean` widened to the `float` the progs globals hold.
#[inline]
fn bool_to_float(b: bool) -> c_float {
    if b {
        1.0
    } else {
        0.0
    }
}

#[inline]
fn sv_aim_value() -> c_float {
    // SAFETY: reading the `.value` field of a C `cvar_t` static owned by
    // pr_cmds.c:1493; cvars are plain single-threaded engine state.
    unsafe { ptr::addr_of!(g::sv_aim).read().value }
}

#[inline]
fn teamplay_value() -> c_float {
    // SAFETY: as above; host.c:84 owns the storage.
    unsafe { ptr::addr_of!(g::teamplay).read().value }
}

#[inline]
fn developer_value() -> c_float {
    // SAFETY: as above; console.c owns the storage.
    unsafe { ptr::addr_of!(c::developer).read().value }
}

/// `EDICT_NUM` through `world_glue.c`'s guard -- it Host_Errors on a bad index
/// (pr_edict.c:1059), so it cannot be expanded in a Rust frame (ADR-009).
#[inline]
unsafe fn edict_num(num: c_int) -> Result<*mut Edict, SvRaise> {
    let mut out: *mut c_void = ptr::null_mut();
    // SAFETY: the glue writes `out` only on success.
    let status = unsafe { wg::World_Glue_EdictNum(num, &mut out) };
    guarded(status)?;
    Ok(out.cast::<Edict>())
}

/* ---------------------------------------------------------------------------
 * Group A -- link (pr_cmds.c:227, :237, :321, :340).
 */

/// `pr_cmds.c:227` `PF_setorigin`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_setorigin(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let e = g_edict(vm, OFS_PARM0);
            let org = g_vector(vm, OFS_PARM1);
            (*e).v.origin = [*org, *org.add(1), *org.add(2)];
            guarded(crate::world::quake_rs_sv_link_edict(e, false))
        })
    }
}

/// `pr_cmds.c:237` `SetMinMaxSize`.
///
/// The `rotate` parameter is dead: pr_cmds.c:250 clears it unconditionally
/// (`rotate = false; // FIXME: implement rotation properly again`), so the
/// whole rotation branch -- and with it the only reader of `e->v.angles` -- is
/// unreachable and is not ported. `e->v.absmin` / `absmax` are NOT set here;
/// `SV_LinkEdict` sets them (world.c), which is why the link call is last.
///
/// # Safety
/// `e` is a live edict; `minvec` / `maxvec` are `vec3_t` snapshots that never
/// alias the edict (they come from the globals block or from model memory).
unsafe fn set_min_max_size(e: *mut Edict, minvec: [c_float; 3], maxvec: [c_float; 3]) -> SvResult {
    // SAFETY: see the doc comment.
    unsafe {
        for i in 0..3 {
            if minvec[i] > maxvec[i] {
                return guarded(g::PRBI_SvGlue_RunErrorBackwardsMinsMaxs());
            }
        }

        (*e).v.mins = minvec;
        (*e).v.maxs = maxvec;
        (*e).v.size = [
            maxvec[0] - minvec[0],
            maxvec[1] - minvec[1],
            maxvec[2] - minvec[2],
        ];

        guarded(crate::world::quake_rs_sv_link_edict(e, false))
    }
}

/// `pr_cmds.c:321` `PF_setsize`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_setsize(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let e = g_edict(vm, OFS_PARM0);
            let minvec = g_vector(vm, OFS_PARM1);
            let maxvec = g_vector(vm, OFS_PARM2);
            set_min_max_size(
                e,
                [*minvec, *minvec.add(1), *minvec.add(2)],
                [*maxvec, *maxvec.add(1), *maxvec.add(2)],
            )
        })
    }
}

/// `pr_cmds.c:340` `PF_sv_setmodel`
///
/// The precache lookup stays in C (`PRBI_SvGlue_SetModelLookup`): it can raise
/// three ways and its `check` aliasing is load-bearing -- see that helper's
/// comment in `Quake/pr_cmds_sv_glue.c`.
///
/// `cvar_t sv_gameplayfix_setmodelrealbox` (pr_cmds.c:338) is declared next to
/// this builtin but never read by it; it keeps its C storage.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_setmodel(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let e = g_edict(vm, OFS_PARM0);
            let handle = g_int(vm, OFS_PARM1);

            let mut name: *const c_char = ptr::null();
            let mut index: c_int = 0;
            let mut model: *mut c_void = ptr::null_mut();
            guarded(g::PRBI_SvGlue_SetModelLookup(
                handle, &mut name, &mut index, &mut model,
            ))?;

            (*e).v.model = g::PR_SetEngineString(name);
            (*e).v.modelindex = index as c_float;

            let m = model.cast::<QModel>();
            if !m.is_null() {
                if (*m).type_ == MOD_BRUSH {
                    set_min_max_size(e, (*m).clipmins, (*m).clipmaxs)
                } else {
                    set_min_max_size(e, (*m).mins, (*m).maxs)
                }
            } else {
                set_min_max_size(e, [0.0; 3], [0.0; 3])
            }
        })
    }
}

/* ---------------------------------------------------------------------------
 * Group B -- trace / movement.
 */

/// The shared `PF_traceline` (pr_cmds.c:740) / `PF_tracebox` (pr_ext.c:1833)
/// body. The two differ only in which globals slots the arguments come from;
/// the NAN warning text says "traceline" in both, tracebox included.
///
/// # Safety
/// All pointers are `vec3_t` slots the NaN clamp below writes through, exactly
/// as the C does -- for `traceline` / `tracebox` those are progs globals.
unsafe fn trace_common(
    vm: *mut QcVm,
    v1: *mut c_float,
    mins: *mut c_float,
    maxs: *mut c_float,
    v2: *mut c_float,
    nomonsters: c_int,
    ent: *mut Edict,
) -> SvResult {
    // SAFETY: see the doc comment.
    unsafe {
        if developer_value() != 0.0
            && (is_nan(*v1)
                || is_nan(*v1.add(1))
                || is_nan(*v1.add(2))
                || is_nan(*v2)
                || is_nan(*v2.add(1))
                || is_nan(*v2.add(2)))
        {
            guarded(g::PRBI_SvGlue_WarnNanTrace(v1, v2, ent.cast::<c_void>()))?;
        }

        if is_nan(*v1) || is_nan(*v1.add(1)) || is_nan(*v1.add(2)) {
            *v1 = 0.0;
            *v1.add(1) = 0.0;
            *v1.add(2) = 0.0;
        }
        if is_nan(*v2) || is_nan(*v2.add(1)) || is_nan(*v2.add(2)) {
            *v2 = 0.0;
            *v2.add(1) = 0.0;
            *v2.add(2) = 0.0;
        }

        let mut trace = zero_trace();
        guarded(crate::world::quake_rs_sv_move(
            &mut trace, v1, mins, maxs, v2, nomonsters, ent,
        ))?;

        let gv = gvars(vm);
        (*gv).trace_allsolid = bool_to_float(trace.allsolid);
        (*gv).trace_startsolid = bool_to_float(trace.startsolid);
        (*gv).trace_fraction = trace.fraction;
        (*gv).trace_inwater = bool_to_float(trace.inwater);
        (*gv).trace_inopen = bool_to_float(trace.inopen);
        (*gv).trace_endpos = trace.endpos;
        (*gv).trace_plane_normal = trace.plane.normal;
        (*gv).trace_plane_dist = trace.plane.dist;

        (*gv).trace_ent = if !trace.ent.is_null() {
            edict_to_prog(vm, trace.ent)
        } else {
            edict_to_prog(vm, (*vm).edicts)
        };

        Ok(())
    }
}

/// `pr_cmds.c:740` `PF_traceline`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_traceline(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let v1 = g_vector(vm, OFS_PARM0);
            let v2 = g_vector(vm, OFS_PARM1);
            let nomonsters = as_int(g_float(vm, OFS_PARM2));
            let ent = g_edict(vm, OFS_PARM3);
            let origin = ptr::addr_of_mut!(crate::mathlib::vec3_origin).cast::<c_float>();
            trace_common(vm, v1, origin, origin, v2, nomonsters, ent)
        })
    }
}

/// `pr_ext.c:1833` `PF_tracebox`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_tracebox(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let v1 = g_vector(vm, OFS_PARM0);
            let mins = g_vector(vm, OFS_PARM1);
            let maxs = g_vector(vm, OFS_PARM2);
            let v2 = g_vector(vm, OFS_PARM3);
            let nomonsters = as_int(g_float(vm, OFS_PARM4));
            let ent = g_edict(vm, OFS_PARM5);
            trace_common(vm, v1, mins, maxs, v2, nomonsters, ent)
        })
    }
}

/// `pr_cmds.c:1017` `PF_findradius`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_findradius(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let mut chain = (*vm).edicts;

            let org = g_vector(vm, OFS_PARM0);
            let mut rad = g_float(vm, OFS_PARM1);
            rad *= rad;

            let mut i: c_int = 1;
            while i < (*vm).num_edicts {
                let ent = edict_at(vm, i);
                i += 1;

                if (*ent).free {
                    continue;
                }
                if (*ent).v.solid == SOLID_NOT {
                    continue;
                }

                // COMPAT: ADR-010 -- `0.5` is a `double` literal, so
                // `origin + 0.5 * (mins + maxs)` and the subtraction from
                // `org` are evaluated in double and narrowed on the assignment
                // to `float d`. The per-axis early-out is load-bearing too: it
                // exits before the later axes are ever touched.
                let mut d = (*org as f64
                    - ((*ent).v.origin[0] as f64
                        + 0.5 * (((*ent).v.mins[0] + (*ent).v.maxs[0]) as f64)))
                    as c_float;
                let mut lensq = d * d;
                if lensq > rad {
                    continue;
                }
                d = (*org.add(1) as f64
                    - ((*ent).v.origin[1] as f64
                        + 0.5 * (((*ent).v.mins[1] + (*ent).v.maxs[1]) as f64)))
                    as c_float;
                lensq += d * d;
                if lensq > rad {
                    continue;
                }
                d = (*org.add(2) as f64
                    - ((*ent).v.origin[2] as f64
                        + 0.5 * (((*ent).v.mins[2] + (*ent).v.maxs[2]) as f64)))
                    as c_float;
                lensq += d * d;
                if lensq > rad {
                    continue;
                }

                (*ent).v.chain = edict_to_prog(vm, chain);
                chain = ent;
            }

            return_edict(vm, chain);
            Ok(())
        })
    }
}

/// `pr_cmds.c:1288` `PF_walkmove`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_walkmove(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let ent = prog_to_edict(vm, (*gvars(vm)).self_);
            let yaw = g_float(vm, OFS_PARM0);
            let dist = g_float(vm, OFS_PARM1);

            if as_int((*ent).v.flags) & (FL_ONGROUND | FL_FLY | FL_SWIM) == 0 {
                *globals(vm).add(OFS_RETURN) = 0.0;
                return Ok(());
            }

            // COMPAT: ADR-010 -- `yaw = yaw * M_PI * 2 / 360` is evaluated in
            // double and stored back into the `float` local, so `cos`/`sin`
            // see the narrowed value. Same trap as SV_StepDirection (M4).
            let yaw = ((yaw as f64 * core::f64::consts::PI) * 2.0 / 360.0) as c_float;

            // COMPAT: ADR-010 -- libm call-through (never `f32::cos`), and
            // `cos (yaw) * dist` is a double product narrowed on the
            // assignment to the `vec3_t` element.
            let mut move_: [c_float; 3] = [
                (c::libm::cos(yaw as f64) * dist as f64) as c_float,
                (c::libm::sin(yaw as f64) * dist as f64) as c_float,
                0.0,
            ];

            // save program state, because SV_movestep may call other progs
            let oldf: *mut DFunction = (*vm).xfunction;
            let oldself: c_int = (*gvars(vm)).self_;

            let mut stepped = false;
            let raised =
                crate::sv_move::quake_rs_sv_movestep(ent, move_.as_mut_ptr(), true, &mut stepped);
            // On a raise C longjmps out of the assignment *and* both restores.
            guarded(raised)?;

            // ADR-006: SV_movestep relinks with touch_triggers=true, which
            // dispatches QuakeC, so the ambient view is re-derived here.
            let vm = c::qcvm.cast::<QcVm>();
            *globals(vm).add(OFS_RETURN) = bool_to_float(stepped);

            // restore program state
            (*vm).xfunction = oldf;
            (*gvars(vm)).self_ = oldself;
            Ok(())
        })
    }
}

/// `pr_cmds.c:1330` `PF_droptofloor`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_droptofloor(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let ent = prog_to_edict(vm, (*gvars(vm)).self_);

            let mut end = (*ent).v.origin;
            end[2] -= 256.0;

            let mut trace = zero_trace();
            guarded(crate::world::quake_rs_sv_move(
                &mut trace,
                (*ent).v.origin.as_mut_ptr(),
                (*ent).v.mins.as_mut_ptr(),
                (*ent).v.maxs.as_mut_ptr(),
                end.as_mut_ptr(),
                MOVE_NORMAL,
                ent,
            ))?;

            if trace.fraction == 1.0 || trace.allsolid {
                *globals(vm).add(OFS_RETURN) = 0.0;
            } else {
                (*ent).v.origin = trace.endpos;
                guarded(crate::world::quake_rs_sv_link_edict(ent, false))?;
                (*ent).v.flags = (as_int((*ent).v.flags) | FL_ONGROUND) as c_float;
                if !trace.ent.is_null() {
                    (*ent).v.groundentity = edict_to_prog(vm, trace.ent);
                }
                *globals(vm).add(OFS_RETURN) = 1.0;
            }
            Ok(())
        })
    }
}

/// `pr_cmds.c:1432` `PF_checkbottom`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_checkbottom(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let ent = g_edict(vm, OFS_PARM0);
            let mut ok = false;
            guarded(crate::sv_move::quake_rs_sv_check_bottom(ent, &mut ok))?;
            *globals(vm).add(OFS_RETURN) = bool_to_float(ok);
            Ok(())
        })
    }
}

/// `pr_cmds.c:1446` `PF_pointcontents`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_pointcontents(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            *globals(vm).add(OFS_RETURN) =
                crate::world::SV_PointContents(g_vector(vm, OFS_PARM0)) as c_float;
            Ok(())
        })
    }
}

/// `pr_cmds.c:1494` `PF_aim`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_aim(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let origin = ptr::addr_of_mut!(crate::mathlib::vec3_origin).cast::<c_float>();

            let ent = g_edict(vm, OFS_PARM0);
            // pr_cmds.c:1504-1505 reads the missile speed and discards it.
            let _speed = g_float(vm, OFS_PARM1);

            let mut start = (*ent).v.origin;
            start[2] += 20.0;

            // try sending a trace straight
            let mut dir = (*gvars(vm)).v_forward;
            // `VectorMA (start, 2048, dir, end)`
            let mut end: [c_float; 3] = [
                start[0] + 2048.0 * dir[0],
                start[1] + 2048.0 * dir[1],
                start[2] + 2048.0 * dir[2],
            ];

            let mut tr = zero_trace();
            guarded(crate::world::quake_rs_sv_move(
                &mut tr,
                start.as_mut_ptr(),
                origin,
                origin,
                end.as_mut_ptr(),
                MOVE_NORMAL,
                ent,
            ))?;

            if !tr.ent.is_null()
                && (*tr.ent).v.takedamage == DAMAGE_AIM
                && (teamplay_value() == 0.0
                    || (*ent).v.team <= 0.0
                    || (*ent).v.team != (*tr.ent).v.team)
            {
                let ret = g_vector(vm, OFS_RETURN);
                let vf = (*gvars(vm)).v_forward;
                *ret = vf[0];
                *ret.add(1) = vf[1];
                *ret.add(2) = vf[2];
                return Ok(());
            }

            // try all possible entities
            let bestdir = dir;
            let mut bestdist = sv_aim_value();
            let mut bestent: *mut Edict = ptr::null_mut();

            let mut i: c_int = 1;
            while i < (*vm).num_edicts {
                let check = edict_at(vm, i);
                i += 1;

                if (*check).v.takedamage != DAMAGE_AIM {
                    continue;
                }
                if check == ent {
                    continue;
                }
                if teamplay_value() != 0.0
                    && (*ent).v.team > 0.0
                    && (*ent).v.team == (*check).v.team
                {
                    continue; // don't aim at teammate
                }
                for (j, e) in end.iter_mut().enumerate() {
                    // COMPAT: ADR-010 -- `0.5` is a `double` literal, so the
                    // whole right-hand side is evaluated in double and
                    // narrowed on the assignment to `end[j]`.
                    *e = ((*check).v.origin[j] as f64
                        + 0.5 * (((*check).v.mins[j] + (*check).v.maxs[j]) as f64))
                        as c_float;
                }
                dir = [end[0] - start[0], end[1] - start[1], end[2] - start[2]];
                crate::mathlib::VectorNormalize(dir.as_mut_ptr());
                let vf = (*gvars(vm)).v_forward;
                let dist = dir[0] * vf[0] + dir[1] * vf[1] + dir[2] * vf[2];
                if dist < bestdist {
                    continue; // to far to turn
                }
                tr = zero_trace();
                guarded(crate::world::quake_rs_sv_move(
                    &mut tr,
                    start.as_mut_ptr(),
                    origin,
                    origin,
                    end.as_mut_ptr(),
                    MOVE_NORMAL,
                    ent,
                ))?;
                if tr.ent == check {
                    // can shoot at this one
                    bestdist = dist;
                    bestent = check;
                }
            }

            let ret = g_vector(vm, OFS_RETURN);
            if !bestent.is_null() {
                dir = [
                    (*bestent).v.origin[0] - (*ent).v.origin[0],
                    (*bestent).v.origin[1] - (*ent).v.origin[1],
                    (*bestent).v.origin[2] - (*ent).v.origin[2],
                ];
                let vf = (*gvars(vm)).v_forward;
                let dist = dir[0] * vf[0] + dir[1] * vf[1] + dir[2] * vf[2];
                end = [vf[0] * dist, vf[1] * dist, vf[2] * dist];
                end[2] = dir[2];
                crate::mathlib::VectorNormalize(end.as_mut_ptr());
                *ret = end[0];
                *ret.add(1) = end[1];
                *ret.add(2) = end[2];
            } else {
                *ret = bestdir[0];
                *ret.add(1) = bestdir[1];
                *ret.add(2) = bestdir[2];
            }
            Ok(())
        })
    }
}

/// `pr_cmds.c:1853` `PF_sv_walkpathtogoal` -- a stub in C too: it always
/// returns `PATH_ERROR` (0) without touching the world.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_walkpathtogoal(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            *globals(vm).add(OFS_RETURN) = 0.0; // PATH_ERROR
            Ok(())
        })
    }
}

/* ---------------------------------------------------------------------------
 * Group C -- PVS (pr_cmds.c:801-895, pr_ext.c:5369).
 */

// COMPAT: ADR-008 -- `checkpvs` / `checkpvs_capacity` are process globals in
// pr_cmds.c:801-802, deliberately NOT per-qcvm: PF_checkclient's round-robin
// cursor lives in `sv` and both VMs share this one buffer. Keeping them here
// preserves that sharing without touching `qcvm_t`.
static mut CHECKPVS: *mut u8 = ptr::null_mut();
static mut CHECKPVS_CAPACITY: c_int = 0;

/// pr_cmds.c:880 `static int c_invis, c_notvis;` -- counted, never read.
static mut C_INVIS: c_int = 0;
static mut C_NOTVIS: c_int = 0;

/// `pr_cmds.c:804` `PF_newcheckclient`
///
/// # Safety
/// The ambient qcvm must have a world model, exactly as in C.
unsafe fn pf_newcheckclient(vm: *mut QcVm, check: c_int) -> Result<c_int, SvRaise> {
    // SAFETY: see the doc comment.
    unsafe {
        let maxclients = pg::SvPhys_Glue_MaxClients();

        // cycle to the next one
        let mut check = check;
        if check < 1 {
            check = 1;
        }
        if check > maxclients {
            check = maxclients;
        }

        let mut i = if check == maxclients { 1 } else { check + 1 };

        let ent;
        loop {
            if i == maxclients + 1 {
                i = 1;
            }

            let e = edict_num(i)?;

            // pr_cmds.c:829 -- the "didn't find anything else" break happens
            // *before* the free / health / notarget skips, so a wrapped-around
            // cursor can land on a free or dead edict.
            if i == check {
                ent = e;
                break;
            }
            if (*e).free {
                i += 1;
                continue;
            }
            if (*e).v.health <= 0.0 {
                i += 1;
                continue;
            }
            if as_int((*e).v.flags) & FL_NOTARGET != 0 {
                i += 1;
                continue;
            }

            // anything that is a client, or has a client as an enemy
            ent = e;
            break;
        }

        // get the PVS for the entity
        let mut org: [c_float; 3] = [
            (*ent).v.origin[0] + (*ent).v.view_ofs[0],
            (*ent).v.origin[1] + (*ent).v.view_ofs[1],
            (*ent).v.origin[2] + (*ent).v.view_ofs[2],
        ];
        let leaf = g::Mod_PointInLeaf(org.as_mut_ptr(), (*vm).worldmodel);
        let pvs = g::Mod_LeafPVS(leaf, (*vm).worldmodel);

        // COMPAT: `(numleafs + 31) >> 3` over-allocates by 4x -- the vanilla
        // expression, kept as-is (pr_cmds.c:849).
        let pvsbytes = ((*worldmodel(vm)).numleafs + 31) >> 3;
        if CHECKPVS.is_null() || pvsbytes > CHECKPVS_CAPACITY {
            CHECKPVS_CAPACITY = pvsbytes;
            CHECKPVS =
                c::Mem_Realloc(CHECKPVS.cast::<c_void>(), CHECKPVS_CAPACITY as usize).cast::<u8>();
            if CHECKPVS.is_null() {
                c::Sys_Error(
                    c"PF_newcheckclient: realloc() failed on %d bytes".as_ptr(),
                    CHECKPVS_CAPACITY,
                );
            }
        }
        ptr::copy_nonoverlapping(pvs, CHECKPVS, pvsbytes as usize);

        Ok(i)
    }
}

/// `pr_cmds.c:881` `PF_sv_checkclient`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_checkclient(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            // find a new check if on a new frame
            if (*vm).time - g::PRBI_SvGlue_SvLastCheckTime() >= 0.1 {
                let next = pf_newcheckclient(vm, g::PRBI_SvGlue_SvLastCheck())?;
                g::PRBI_SvGlue_SetSvLastCheck(next);
                g::PRBI_SvGlue_SetSvLastCheckTime((*vm).time);
            }

            // return check if it might be visible
            let ent = edict_num(g::PRBI_SvGlue_SvLastCheck())?;
            if (*ent).free || (*ent).v.health <= 0.0 {
                return_edict(vm, (*vm).edicts);
                return Ok(());
            }

            // if current entity can't possibly see the check entity, return 0
            let self_ = prog_to_edict(vm, (*gvars(vm)).self_);
            let mut view: [c_float; 3] = [
                (*self_).v.origin[0] + (*self_).v.view_ofs[0],
                (*self_).v.origin[1] + (*self_).v.view_ofs[1],
                (*self_).v.origin[2] + (*self_).v.view_ofs[2],
            ];
            let leaf = g::Mod_PointInLeaf(view.as_mut_ptr(), (*vm).worldmodel);
            // COMPAT: ADR-006 -- C's `(leaf - worldmodel->leafs) - 1` is a raw
            // pointer subtraction with no null check; spelled as an address
            // difference so a NULL leaf (which C also tolerates, falling into
            // the `l < 0` arm) is not Rust UB.
            let leafs = (*worldmodel(vm)).leafs;
            let l = ((leaf as isize - leafs as isize) / core::mem::size_of::<MLeaf>() as isize)
                as c_int
                - 1;
            // COMPAT: `checkpvs` is still NULL if no client has ever been
            // checked; vanilla Quake dereferences it here just the same.
            if l < 0 || (*CHECKPVS.offset((l >> 3) as isize) & (1u8 << (l & 7))) == 0 {
                C_NOTVIS += 1;
                return_edict(vm, (*vm).edicts);
                return Ok(());
            }

            // might be able to see it
            C_INVIS += 1;
            return_edict(vm, ent);
            Ok(())
        })
    }
}

/// `pr_ext.c:5369` `PF_checkpvs`. Note it calls `Mod_LeafPVS` directly rather
/// than sharing `PF_checkclient`'s `checkpvs` buffer.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_checkpvs(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let org = g_vector(vm, OFS_PARM0);
            let ed = g_edict(vm, OFS_PARM1);

            let leaf = g::Mod_PointInLeaf(org, (*vm).worldmodel);
            let pvs = g::Mod_LeafPVS(leaf, (*vm).worldmodel);

            let mut i: c_uint = 0;
            while i < (*ed).num_leafs {
                // COMPAT: `leafnums` is indexed without a MAX_ENT_LEAFS clamp,
                // matching the C; the raw pointer read keeps Rust from adding
                // a bounds panic the C does not have.
                let ln = *(*ed).leafnums.as_ptr().add(i as usize);
                if *pvs.offset((ln >> 3) as isize) & (1u8 << (ln & 7)) != 0 {
                    *globals(vm).add(OFS_RETURN) = 1.0;
                    return Ok(());
                }
                i += 1;
            }

            *globals(vm).add(OFS_RETURN) = 0.0;
            Ok(())
        })
    }
}
