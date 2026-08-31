//! C ABI shims for `Quake/world.c` (Rust migration Phase 7 M3).
//!
//! Near-transliteration of the world queries: the box hull, the areanode
//! tree, `SV_LinkEdict`/`SV_TouchLinks`, both hull-check implementations
//! (QuakeSpasm's `SV_SlowRecursiveHullCheck` and FTE's
//! `Q1BSP_RecursiveHullTrace`), the `SV_Move` pipeline and the CSQC
//! `World_ClipToNetwork` pass. `box_hull`/`box_clipnodes`/`box_planes` were
//! `static` in C and are private here.
//!
//! ADR-009 audit. Three C paths reachable from this module can `Host_Error`:
//! `PR_ExecuteProgram` (the `SV_TouchLinks` touch dispatch), `EDICT_NUM` /
//! `NUM_FOR_EDICT` (bad index / bad pointer) and `COM_Assert_Failed` on the
//! main thread. Each goes through a `World_Glue_*` helper that wraps the call
//! in `Host_Guard`, and the caught status is returned to `Quake/world_glue.c`,
//! which re-issues the jump from a pure C frame. The only status core is
//! `quake_rs_sv_link_edict`; the frozen M3 contract exports everything else
//! plain.
//!
//! `PR_GetString` (pr_edict_arena.c:315) is a fourth raising path, reached by
//! the two `Con_Warning`s in `SV_HullForEntity`, so those warnings are guarded
//! too. Six entry points are therefore `quake_rs_*` status cores:
//! `quake_rs_sv_link_edict`, `quake_rs_sv_hull_for_entity`,
//! `quake_rs_sv_clip_move_to_entity`, `quake_rs_sv_move`,
//! `quake_rs_sv_test_entity_position` and
//! `quake_rs_sv_point_contents_all_bsps`. A non-zero status is returned
//! immediately by every intermediate function -- the `SV_ClipToLinks` walk and
//! the `World_ClipToNetwork` loop abandon their remaining work on the spot --
//! so the Rust side stops exactly where C's `longjmp` would have left it, and
//! no jump ever unwinds a Rust frame. Nothing in this module calls
//! `Host_Reraise`; `Quake/world_glue.c` owns every re-raise.
//!
//! `Sys_Error` only aborts the process, so its three sites are called
//! directly (Phase 5/6 precedent).

use core::ffi::{c_float, c_int, c_uint, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::world as g;
use quake_math::mathlib as m;
use quake_types::model_mem::{Hull, MClipnode, MLeaf, MNode, QModel, MOD_BRUSH};
use quake_types::progs::{AreaNode, Edict, GlobalVars, Link, QcVm, AREA_NODES, MAX_ENT_LEAFS};
use quake_types::MPlane;

/// Guard status carried back to `Quake/world_glue.c`; 0 means "no raise".
type Raise = c_int;

// ---------------------------------------------------------------------------
// engine constants this module compares against
//
// `entvars_t` stores solid/movetype/flags as floats, so the server.h enums are
// spelled as the float literals the C comparisons promote them to.

/// `bspfile.h` `CONTENTS_EMPTY`
const CONTENTS_EMPTY: c_int = -1;
/// `bspfile.h` `CONTENTS_SOLID`
const CONTENTS_SOLID: c_int = -2;
/// `bspfile.h` `CONTENTS_WATER`
const CONTENTS_WATER: c_int = -3;
/// `bspfile.h` `CONTENTS_CURRENT_0`
const CONTENTS_CURRENT_0: c_int = -9;
/// `bspfile.h` `CONTENTS_CURRENT_DOWN`
const CONTENTS_CURRENT_DOWN: c_int = -14;

/// `server.h` `SOLID_NOT`
const SOLID_NOT: c_float = 0.0;
/// `server.h` `SOLID_TRIGGER`
const SOLID_TRIGGER: c_float = 1.0;
/// `server.h` `SOLID_BSP`
const SOLID_BSP: c_float = 4.0;
/// `server.h` `MOVETYPE_PUSH`
const MOVETYPE_PUSH: c_float = 7.0;
/// `server.h` `FL_MONSTER`
const FL_MONSTER: c_int = 32;
/// `server.h` `FL_ITEM`
const FL_ITEM: c_int = 256;

/// `world.h` `MOVE_NOMONSTERS`
const MOVE_NOMONSTERS: c_int = 1;
/// `world.h` `MOVE_MISSILE`
const MOVE_MISSILE: c_int = 2;
/// `world.h` `MOVE_HITALLCONTENTS`
const MOVE_HITALLCONTENTS: c_int = 1 << 9;
/// `world.h` `CONTENTMASK_ANYSOLID` == `(1u << 2) | (1u << 8)`
const CONTENTMASK_ANYSOLID: c_uint = 260;

/// `world.h` `rht_solid`
const RHT_SOLID: c_int = 0;
/// `world.h` `rht_empty`
const RHT_EMPTY: c_int = 1;
/// `world.h` `rht_impact`
const RHT_IMPACT: c_int = 2;

/// `protocol.h` `ES_SOLID_NOT`
const ES_SOLID_NOT: c_uint = 0;
/// `protocol.h` `ES_SOLID_BSP`
const ES_SOLID_BSP: c_uint = 31;

/// `progs.h` `VANILLA_AREA_DEPTH` (not mirrored in quake-types)
const VANILLA_AREA_DEPTH: c_int = 4;
/// `progs.h` `MAX_AREA_DEPTH`
const MAX_AREA_DEPTH: c_int = quake_types::progs::MAX_AREA_DEPTH as c_int;

/// `quakedef.h:66` `#define DIST_EPSILON (0.03125)`
// COMPAT: ADR-010 -- the macro is an unsuffixed literal, so every expression it
// appears in is evaluated in `double` even when the operands are `float`.
const DIST_EPSILON: f64 = 0.03125;

const SYS_ERR_HULL_POINT_CONTENTS: &core::ffi::CStr = c"SV_HullPointContents: bad node number";
const SYS_ERR_RECURSIVE_HULL_CHECK: &core::ffi::CStr = c"SV_RecursiveHullCheck: bad node number";
const SYS_ERR_TRIGGER_IN_CLIP_LIST: &core::ffi::CStr = c"Trigger in clipping list";
const ASSERT_FILE: &core::ffi::CStr = c"world.c";
const ASSERT_ENT_FREE: &core::ffi::CStr = c"!ent->free";
const ASSERT_TRACE_ENT_FREE: &core::ffi::CStr = c"!clip.trace.ent->free";

// ---------------------------------------------------------------------------
// world.h aggregates (world.c-local; no quake-types mirror)

/// `world.h` `plane_t`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct PlaneT {
    pub normal: [c_float; 3],
    pub dist: c_float,
}

/// `world.h` `trace_t`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Trace {
    pub allsolid: bool,
    pub startsolid: bool,
    pub inopen: bool,
    pub inwater: bool,
    pub fraction: c_float,
    pub endpos: [c_float; 3],
    pub plane: PlaneT,
    pub ent: *mut Edict,
    pub contents: c_int,
}

impl Trace {
    /// `memset (&trace, 0, sizeof (trace_t))`
    const fn zeroed() -> Self {
        Self {
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
}

/// `world.h` `struct rhtctx_s`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RhtCtx {
    pub hitcontents: c_uint,
    pub start: [c_float; 3],
    pub end: [c_float; 3],
    pub clipnodes: *mut MClipnode,
    pub planes: *mut MPlane,
}

/// `world.c:45` `moveclip_t` (file-private in C, private here).
struct MoveClip {
    boxmins: [c_float; 3],
    boxmaxs: [c_float; 3],
    mins: *mut c_float,
    maxs: *mut c_float,
    mins2: [c_float; 3],
    maxs2: [c_float; 3],
    start: *mut c_float,
    end: *mut c_float,
    trace: Trace,
    type_: c_int,
    hitcontents: c_uint,
    passedict: *mut Edict,
}

// ---------------------------------------------------------------------------
// file-private hull state (world.c:63-65)

/// `static hull_t box_hull;`
static mut BOX_HULL: Hull = Hull {
    clipnodes: ptr::null_mut(),
    planes: ptr::null_mut(),
    firstclipnode: 0,
    lastclipnode: 0,
    clip_mins: [0.0; 3],
    clip_maxs: [0.0; 3],
};

/// `static mclipnode_t box_clipnodes[6];`
static mut BOX_CLIPNODES: [MClipnode; 6] = [MClipnode {
    planenum: 0,
    children: [0; 2],
}; 6];

/// `static mplane_t box_planes[6];`
static mut BOX_PLANES: [MPlane; 6] = [MPlane {
    normal: [0.0; 3],
    dist: 0.0,
    type_: 0,
    signbits: 0,
    pad: [0; 2],
}; 6];

// ---------------------------------------------------------------------------
// small helpers

/// `mathlib.h` `DotProduct` -- all-`float`, left-associated.
// COMPAT: ADR-010 -- accumulation order and intermediate type are the macro's.
#[inline]
fn dot(a: &[c_float; 3], b: &[c_float; 3]) -> c_float {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `mathlib.h` `DoublePrecisionDotProduct` -- each product widened to `double`
/// before the adds.
// COMPAT: ADR-010 -- Q1BSP_RecursiveHullTrace deliberately mixes this with the
// plain float DotProduct inside one function; the two must not be unified.
#[inline]
fn dp_dot(a: &[c_float; 3], b: &[c_float; 3]) -> f64 {
    f64::from(a[0]) * f64::from(b[0])
        + f64::from(a[1]) * f64::from(b[1])
        + f64::from(a[2]) * f64::from(b[2])
}

/// `world.h` `CONTENTMASK_FROMQ1 (c)` == `1u << (-(c))`.
// COMPAT: ADR-010 -- a shift count >= 32 is UB in C and masked to 5 bits by the
// hardware the engine ships on; `wrapping_shl` reproduces that instead of
// panicking (contract rule 8).
#[inline]
fn contentmask_fromq1(contents: c_int) -> c_uint {
    1u32.wrapping_shl(contents.wrapping_neg() as c_uint)
}

/// `q_minmax.h` `CLAMP` resolved to `clamp_f`: NaN falls through both
/// comparisons and is returned unchanged, exactly as the C helper does.
// COMPAT: ADR-010 -- not `f32::clamp`, whose NaN handling differs.
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

/// `q_minmax.h` `q_max_f`
#[inline]
fn q_max_f(a: c_float, b: c_float) -> c_float {
    if a > b {
        a
    } else {
        b
    }
}

/// `pr_ext.c`'s `pr_checkextension`; C's `if (x)` on a float cvar is `x != 0`.
#[inline]
fn pr_checkextension_on() -> bool {
    // SAFETY: reading the `.value` field of a C `cvar_t` static owned by
    // pr_ext.c; cvars are plain single-threaded engine state.
    unsafe { ptr::addr_of!(g::pr_checkextension).read().value != 0.0 }
}

#[inline]
fn sv_fte_createareanode_value() -> c_float {
    // SAFETY: as above; world_glue.c owns the storage.
    unsafe { ptr::addr_of!(g::sv_fte_createareanode).read().value }
}

#[inline]
fn sv_fte_recursivehullckeck_value() -> c_float {
    // SAFETY: as above; world_glue.c owns the storage.
    unsafe { ptr::addr_of!(g::sv_fte_recursivehullckeck).read().value }
}

/// `common.c:82` `ClearLink`
#[inline]
unsafe fn clear_link(l: *mut Link) {
    // SAFETY: `l` is an areanode list head or a live edict's `area` member.
    unsafe {
        (*l).next = l;
        (*l).prev = l;
    }
}

/// `common.c:88` `RemoveLink`
#[inline]
unsafe fn remove_link(l: *mut Link) {
    // SAFETY: `l` is linked into a well-formed circular list; callers check
    // `ent->area.prev` exactly as world.c does.
    unsafe {
        (*(*l).next).prev = (*l).prev;
        (*(*l).prev).next = (*l).next;
    }
}

/// `common.c:94` `InsertLinkBefore`
#[inline]
unsafe fn insert_link_before(l: *mut Link, before: *mut Link) {
    // SAFETY: as above; `before` is an areanode list head.
    unsafe {
        (*l).next = before;
        (*l).prev = (*before).prev;
        (*(*l).prev).next = l;
        (*(*l).next).prev = l;
    }
}

/// `progs.h` `EDICT_FROM_AREA (l)`
#[inline]
fn edict_from_area(l: *mut Link) -> *mut Edict {
    l.cast::<u8>()
        .wrapping_sub(core::mem::offset_of!(Edict, area))
        .cast::<Edict>()
}

/// `progs.h` `PROG_TO_EDICT (e)`
#[inline]
unsafe fn prog_to_edict(vm: *mut QcVm, p: c_int) -> *mut Edict {
    // SAFETY: pointer arithmetic only, byte-for-byte the C macro (which has no
    // bounds check either).
    unsafe {
        (*vm)
            .edicts
            .cast::<u8>()
            .wrapping_offset(p as isize)
            .cast::<Edict>()
    }
}

/// `mathlib.h` `BOX_ON_PLANE_SIDE`
#[inline]
unsafe fn box_on_plane_side(emins: *mut c_float, emaxs: *mut c_float, p: *mut MPlane) -> c_int {
    // SAFETY: `p` is a BSP node splitplane, `emins`/`emaxs` are `vec3_t`.
    unsafe {
        let t = (*p).type_;
        if t < 3 {
            let i = t as usize;
            if (*p).dist <= *emins.add(i) {
                1
            } else if (*p).dist >= *emaxs.add(i) {
                2
            } else {
                3
            }
        } else {
            crate::mathlib::BoxOnPlaneSide(emins, emaxs, p)
        }
    }
}

#[inline]
unsafe fn read3(p: *const c_float) -> [c_float; 3] {
    // SAFETY: every caller passes a `vec3_t`.
    unsafe { [*p, *p.add(1), *p.add(2)] }
}

#[inline]
unsafe fn write3(p: *mut c_float, v: [c_float; 3]) {
    // SAFETY: every caller passes a `vec3_t`.
    unsafe {
        *p = v[0];
        *p.add(1) = v[1];
        *p.add(2) = v[2];
    }
}

// ---------------------------------------------------------------------------
// hull boxes (world.c:68-181)

/// # Safety
/// Single-threaded server state; call only from the engine's main thread.
#[no_mangle]
pub unsafe extern "C" fn SV_InitBoxHull() {
    // SAFETY: the three statics replace world.c's file-private objects and,
    // like them, are touched only from the server's single thread.
    unsafe {
        let clipnodes = ptr::addr_of_mut!(BOX_CLIPNODES).cast::<MClipnode>();
        let planes = ptr::addr_of_mut!(BOX_PLANES).cast::<MPlane>();
        let hull = ptr::addr_of_mut!(BOX_HULL);

        (*hull).clipnodes = clipnodes;
        (*hull).planes = planes;
        (*hull).firstclipnode = 0;
        (*hull).lastclipnode = 5;

        for i in 0..6i32 {
            let node = clipnodes.offset(i as isize);
            (*node).planenum = i;

            let side = (i & 1) as usize;
            (*node).children[side] = CONTENTS_EMPTY;
            if i != 5 {
                (*node).children[side ^ 1] = i + 1;
            } else {
                (*node).children[side ^ 1] = CONTENTS_SOLID;
            }

            let plane = planes.offset(i as isize);
            (*plane).type_ = (i >> 1) as u8;
            (*plane).normal[(i >> 1) as usize] = 1.0;
        }
    }
}

/// # Safety
/// `mins`/`maxs` must each point to three readable floats.
#[no_mangle]
pub unsafe extern "C" fn SV_HullForBox(mins: *mut c_float, maxs: *mut c_float) -> *mut Hull {
    // SAFETY: `vec3_t` contracts per the fn docs; the statics are
    // server-thread-only.
    unsafe {
        let planes = ptr::addr_of_mut!(BOX_PLANES).cast::<MPlane>();
        (*planes.add(0)).dist = *maxs;
        (*planes.add(1)).dist = *mins;
        (*planes.add(2)).dist = *maxs.add(1);
        (*planes.add(3)).dist = *mins.add(1);
        (*planes.add(4)).dist = *maxs.add(2);
        (*planes.add(5)).dist = *mins.add(2);
        ptr::addr_of_mut!(BOX_HULL)
    }
}

/// ADR-009 status core for `SV_HullForEntity`; `Quake/world_glue.c` re-raises.
///
/// # Safety
/// `ent` must be a live edict; `mins`/`maxs`/`offset` must be `vec3_t`; `out`
/// must be a writable `hull_t *` slot.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_hull_for_entity(
    ent: *mut Edict,
    mins: *mut c_float,
    maxs: *mut c_float,
    offset: *mut c_float,
    out: *mut *mut Hull,
) -> Raise {
    // SAFETY: pointer contracts per the fn docs; `qcvm` is the ambient VM
    // (ADR-008), resolved once for this entry point.
    unsafe {
        let vm = c::qcvm.cast::<QcVm>();

        // the labelled block reproduces world.c's `goto nohitmeshsupport`: both
        // SOLID_BSP failure paths fall into the bounding-box branch below.
        'bsp: {
            if (*ent).v.solid != SOLID_BSP {
                break 'bsp;
            }

            if (*ent).v.movetype != MOVETYPE_PUSH && !pr_checkextension_on() {
                // ADR-009: PR_GetString can raise (pr_edict_arena.c:315), so the
                // warning is guarded. ADR-005: the `%f` conversions stay in C.
                let raised = g::World_Glue_WarnSolidBspNoPush(ent.cast());
                if raised != 0 {
                    return raised;
                }
            }

            // COMPAT: ADR-010 -- `qcvm->GetModel (ent->v.modelindex)` truncates
            // a float toward zero; `as` saturates where C is UB (rule 8).
            let model = match (*vm).get_model {
                Some(f) => f((*ent).v.modelindex as c_int).cast::<QModel>(),
                // COMPAT: ADR-004 -- C calls through the function pointer
                // unconditionally; taking the "no model" branch is the closest
                // defined behaviour to a null `GetModel`.
                None => ptr::null_mut(),
            };

            if model.is_null() || (*model).type_ != MOD_BRUSH {
                let raised = g::World_Glue_WarnSolidBspNonBspModel(ent.cast());
                if raised != 0 {
                    return raised;
                }
                break 'bsp;
            }

            let size0 = *maxs - *mins;
            let hulls = ptr::addr_of_mut!((*model).hulls).cast::<Hull>();
            let hull = if size0 < 3.0 {
                hulls
            } else if size0 <= 32.0 {
                hulls.add(1)
            } else {
                hulls.add(2)
            };

            // calculate an offset value to center the origin
            *offset = (*hull).clip_mins[0] - *mins;
            *offset.add(1) = (*hull).clip_mins[1] - *mins.add(1);
            *offset.add(2) = (*hull).clip_mins[2] - *mins.add(2);
            *offset += (*ent).v.origin[0];
            *offset.add(1) += (*ent).v.origin[1];
            *offset.add(2) += (*ent).v.origin[2];

            *out = hull;
            return 0;
        }

        // create a temp hull from bounding box sizes
        let mut hullmins = [
            (*ent).v.mins[0] - *maxs,
            (*ent).v.mins[1] - *maxs.add(1),
            (*ent).v.mins[2] - *maxs.add(2),
        ];
        let mut hullmaxs = [
            (*ent).v.maxs[0] - *mins,
            (*ent).v.maxs[1] - *mins.add(1),
            (*ent).v.maxs[2] - *mins.add(2),
        ];
        let hull = SV_HullForBox(hullmins.as_mut_ptr(), hullmaxs.as_mut_ptr());
        write3(offset, (*ent).v.origin);
        *out = hull;
        0
    }
}

// ---------------------------------------------------------------------------
// entity area checking (world.c:190-421)

/// # Safety
/// `mins`/`maxs` must be `vec3_t`; the ambient `qcvm` must be selected.
#[no_mangle]
pub unsafe extern "C" fn SV_CreateAreaNode(
    depth: c_int,
    mins: *mut c_float,
    maxs: *mut c_float,
) -> *mut AreaNode {
    // SAFETY: ADR-008 ambient qcvm; the areanode array lives inside it and is
    // filled front to back exactly as C does (no bounds check there either).
    unsafe {
        let vm = c::qcvm.cast::<QcVm>();
        let base = ptr::addr_of_mut!((*vm).areanodes).cast::<AreaNode>();
        let anode = base.offset((*vm).numareanodes as isize);
        (*vm).numareanodes += 1;

        clear_link(ptr::addr_of_mut!((*anode).trigger_edicts));
        clear_link(ptr::addr_of_mut!((*anode).solid_edicts));

        let lo = read3(mins);
        let hi = read3(maxs);
        let size = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];

        (*anode).axis = if size[0] > size[1] { 0 } else { 1 };

        let max_depth_reached = if pr_checkextension_on() && sv_fte_createareanode_value() > 0.0 {
            depth == MAX_AREA_DEPTH || size[(*anode).axis as usize] < 500.0
        } else {
            depth == VANILLA_AREA_DEPTH
        };

        if max_depth_reached {
            (*anode).axis = -1;
            (*anode).children[1] = ptr::null_mut();
            (*anode).children[0] = ptr::null_mut();
            return anode;
        }

        let ax = (*anode).axis as usize;
        // COMPAT: ADR-010 -- `0.5` is a double literal, so the multiply happens
        // in double and narrows once on the store to `float dist`. The add
        // itself stays in `float`, matching C's two `float` operands; widening
        // it would diverge when the sum overflows `float`.
        (*anode).dist = (0.5f64 * (hi[ax] + lo[ax]) as f64) as c_float;

        let mut mins1 = lo;
        let mut mins2 = lo;
        let mut maxs1 = hi;
        let mut maxs2 = hi;
        maxs1[ax] = (*anode).dist;
        mins2[ax] = (*anode).dist;

        (*anode).children[0] = SV_CreateAreaNode(depth + 1, mins2.as_mut_ptr(), maxs2.as_mut_ptr());
        (*anode).children[1] = SV_CreateAreaNode(depth + 1, mins1.as_mut_ptr(), maxs1.as_mut_ptr());

        anode
    }
}

/// # Safety
/// The ambient `qcvm` must be selected and its worldmodel loaded.
#[no_mangle]
pub unsafe extern "C" fn SV_ClearWorld() {
    // SAFETY: ADR-008 ambient qcvm; `areanodes` is an inline array in it.
    unsafe {
        SV_InitBoxHull();

        let vm = c::qcvm.cast::<QcVm>();
        ptr::write_bytes(
            ptr::addr_of_mut!((*vm).areanodes).cast::<AreaNode>(),
            0,
            AREA_NODES,
        );
        (*vm).numareanodes = 0;

        let wm = (*vm).worldmodel.cast::<QModel>();
        SV_CreateAreaNode(
            0,
            ptr::addr_of_mut!((*wm).mins).cast::<c_float>(),
            ptr::addr_of_mut!((*wm).maxs).cast::<c_float>(),
        );
    }
}

/// # Safety
/// `ent` must be a live edict.
#[no_mangle]
pub unsafe extern "C" fn SV_UnlinkEdict(ent: *mut Edict) {
    // SAFETY: `area` is an inline member of the edict.
    unsafe {
        if (*ent).area.prev.is_null() {
            return; // not linked in anywhere
        }
        remove_link(ptr::addr_of_mut!((*ent).area));
        (*ent).area.next = ptr::null_mut();
        (*ent).area.prev = ptr::null_mut();
    }
}

/// `world.c:293` `SV_AreaTriggerEdicts` (file-private in C).
unsafe fn sv_area_trigger_edicts(
    ent: *mut Edict,
    node: *mut AreaNode,
    list: &mut Vec<u16>,
    listspace: c_int,
) -> Raise {
    // SAFETY: the area tree is a well-formed circular list built by
    // SV_CreateAreaNode/SV_LinkEdict. Nothing here dispatches progs code, so no
    // arena pointer can be invalidated mid-walk (ADR-006).
    unsafe {
        let head = ptr::addr_of_mut!((*node).trigger_edicts);
        let mut l = (*head).next;
        while l != head {
            let next = (*l).next;
            let touch = edict_from_area(l);
            l = next;

            if touch == ent {
                continue;
            }
            if (*touch).v.touch == 0 || (*touch).v.solid != SOLID_TRIGGER {
                continue;
            }
            if (*ent).v.absmin[0] > (*touch).v.absmax[0]
                || (*ent).v.absmin[1] > (*touch).v.absmax[1]
                || (*ent).v.absmin[2] > (*touch).v.absmax[2]
                || (*ent).v.absmax[0] < (*touch).v.absmin[0]
                || (*ent).v.absmax[1] < (*touch).v.absmin[1]
                || (*ent).v.absmax[2] < (*touch).v.absmin[2]
            {
                continue;
            }

            if list.len() as c_int == listspace {
                return 0; // should never happen
            }

            let mut num: c_int = 0;
            let raised = g::World_Glue_NumForEdict(touch.cast(), &mut num);
            if raised != 0 {
                return raised;
            }
            // COMPAT: ADR-010 -- C stores an `int` into a `uint16_t` slot; `as`
            // reproduces that truncating conversion.
            list.push(num as u16);
        }

        if (*node).axis == -1 {
            return 0;
        }

        let ax = (*node).axis as usize;
        if (*ent).v.absmax[ax] > (*node).dist {
            let raised = sv_area_trigger_edicts(ent, (*node).children[0], list, listspace);
            if raised != 0 {
                return raised;
            }
        }
        if (*ent).v.absmin[ax] < (*node).dist {
            let raised = sv_area_trigger_edicts(ent, (*node).children[1], list, listspace);
            if raised != 0 {
                return raised;
            }
        }
        0
    }
}

/// `world.c:333` `SV_TouchLinks` (file-private in C).
unsafe fn sv_touch_links(ent: *mut Edict) -> Raise {
    // SAFETY: ADR-006 -- no pointer derived from the edict arena survives a
    // `World_Glue_CallTouch`; the loop carries only edict *numbers* and
    // re-resolves `qcvm` and the edict on every iteration.
    unsafe {
        if (*ent).free {
            // world.c:336 `assert_always (!ent->free);`
            let raised =
                g::World_Glue_AssertFailed(ASSERT_ENT_FREE.as_ptr(), ASSERT_FILE.as_ptr(), 336);
            if raised != 0 {
                return raised;
            }
            // COM_Assert_Failed never returns on the main thread (Host_Error)
            // and aborts on a worker, so falling through is unreachable.
        }

        let vm = c::qcvm.cast::<QcVm>();
        let listspace = (*vm).num_edicts;

        // COMPAT: ADR-006 -- world.c uses TEMP_ALLOC (alloca) for this list. The
        // allocation strategy is unobservable; only the contents and the
        // `listspace` cap matter, and MAX_EDICTS fits a uint16_t.
        let mut list: Vec<u16> = Vec::with_capacity(listspace.max(0) as usize);

        let raised = sv_area_trigger_edicts(
            ent,
            ptr::addr_of_mut!((*vm).areanodes).cast::<AreaNode>(),
            &mut list,
            listspace,
        );
        if raised != 0 {
            return raised;
        }

        // `pr_global_struct` is `qcvm->globals` viewed as `globalvars_t`; the
        // engine keeps the two in lockstep and quake-progs uses the same view.
        let globals = (*vm).globals.cast::<GlobalVars>();
        let old_self = (*globals).self_;
        let old_other = (*globals).other;

        for &num in &list {
            let vm = c::qcvm.cast::<QcVm>();

            let mut touch_raw: *mut c_void = ptr::null_mut();
            let raised = g::World_Glue_EdictNum(c_int::from(num), &mut touch_raw);
            if raised != 0 {
                return raised;
            }
            let touch = touch_raw.cast::<Edict>();

            // re-validate: PR_ExecuteProgram may have made later entries stale
            if (*touch).free || touch == ent {
                continue;
            }
            if (*touch).v.touch == 0 || (*touch).v.solid != SOLID_TRIGGER {
                continue;
            }
            if (*ent).v.absmin[0] > (*touch).v.absmax[0]
                || (*ent).v.absmin[1] > (*touch).v.absmax[1]
                || (*ent).v.absmin[2] > (*touch).v.absmax[2]
                || (*ent).v.absmax[0] < (*touch).v.absmin[0]
                || (*ent).v.absmax[1] < (*touch).v.absmin[1]
                || (*ent).v.absmax[2] < (*touch).v.absmin[2]
            {
                continue;
            }

            // COMPAT: ADR-010 -- `pr_global_struct->time = qcvm->time` narrows a
            // double to float exactly once, at the store.
            let time = (*vm).time as c_float;
            let raised = g::World_Glue_CallTouch(touch.cast(), ent.cast(), time);
            if raised != 0 {
                // world.c's longjmp skips the self/other restore below, so
                // returning here is the faithful behaviour.
                return raised;
            }

            // bail out if ent got freed as a side effect of v.touch
            if (*ent).free {
                break;
            }
        }

        let vm = c::qcvm.cast::<QcVm>();
        let globals = (*vm).globals.cast::<GlobalVars>();
        (*globals).self_ = old_self;
        (*globals).other = old_other;
        0
    }
}

/// # Safety
/// `ent` must be a live edict and `node` a node of the worldmodel BSP.
#[no_mangle]
pub unsafe extern "C" fn SV_FindTouchedLeafs(ent: *mut Edict, node: *mut MNode) {
    // SAFETY: ADR-008 ambient qcvm; `node` walks the worldmodel node array.
    unsafe {
        if (*node).contents == CONTENTS_SOLID {
            return;
        }
        if (*ent).num_leafs as usize == MAX_ENT_LEAFS {
            return;
        }

        // add an efrag if the node is a leaf
        if (*node).contents < 0 {
            let leaf = node.cast::<MLeaf>();
            let vm = c::qcvm.cast::<QcVm>();
            let wm = (*vm).worldmodel.cast::<QModel>();
            // COMPAT: ADR-010 -- C narrows the ptrdiff_t to `int` on the store.
            let leafnum = (leaf.offset_from((*wm).leafs) - 1) as c_int;

            // COMPAT: ADR-004 -- C indexes leafnums[] unchecked. The MAX_ENT_LEAFS
            // guard above makes an out-of-range index unreachable; dropping the
            // write rather than panicking keeps "no panic across FFI" if it is not.
            let idx = (*ent).num_leafs as usize;
            if let Some(slot) = (*ent).leafnums.get_mut(idx) {
                *slot = leafnum;
            }
            (*ent).num_leafs += 1;
            return;
        }

        // NODE_MIXED
        let splitplane = (*node).plane;
        let sides = box_on_plane_side(
            ptr::addr_of_mut!((*ent).v.absmin).cast::<c_float>(),
            ptr::addr_of_mut!((*ent).v.absmax).cast::<c_float>(),
            splitplane,
        );

        if sides & 1 != 0 {
            SV_FindTouchedLeafs(ent, (*node).children[0]);
        }
        if sides & 2 != 0 {
            SV_FindTouchedLeafs(ent, (*node).children[1]);
        }
    }
}

/// `world.c:428` `SV_LinkEdict`, split per ADR-009: the touch dispatch can
/// `Host_Error`, so this core returns the guard status and
/// `Quake/world_glue.c`'s `SV_LinkEdict` wrapper re-issues it.
///
/// # Safety
/// `ent` must be a live edict of the ambient qcvm.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_link_edict(ent: *mut Edict, touch_triggers: bool) -> c_int {
    // SAFETY: ADR-008 ambient qcvm. The only call that can invalidate arena
    // pointers is the SV_TouchLinks dispatch at the very end, after which
    // nothing here dereferences `ent` again.
    unsafe {
        if !(*ent).area.prev.is_null() {
            SV_UnlinkEdict(ent); // unlink from old position
        }

        let vm = c::qcvm.cast::<QcVm>();
        if ent == (*vm).edicts {
            return 0; // don't add the world
        }
        if (*ent).free {
            return 0;
        }

        // set the abs box
        if (*ent).v.solid == SOLID_BSP
            && pr_checkextension_on()
            && m::is_origin_within_min_max(&(*ent).v.origin, &(*ent).v.mins, &(*ent).v.maxs)
            && !m::is_axis_aligned_deg(&(*ent).v.angles)
        {
            // expand for rotation the lame way (q2 method); hopefully there's an
            // origin brush in there.
            // COMPAT: ADR-010 -- `fabs` is the double-precision libm entry
            // point (float arg promoted, result narrowed on the store),
            // called through the platform libm rather than `f32::abs`.
            let mut max = [0.0f32; 3];
            for (i, slot) in max.iter_mut().enumerate() {
                let v1 = c::libm::fabs(f64::from((*ent).v.mins[i])) as c_float;
                let v2 = c::libm::fabs(f64::from((*ent).v.maxs[i])) as c_float;
                *slot = q_max_f(v1, v2);
            }
            // COMPAT: ADR-010 -- the float DotProduct is widened once for the
            // double `sqrt` and narrowed once on the store, via platform libm.
            let v1 = c::libm::sqrt(f64::from(dot(&max, &max))) as c_float;
            (*ent).v.absmin[0] = (*ent).v.origin[0] - v1;
            (*ent).v.absmin[1] = (*ent).v.origin[1] - v1;
            (*ent).v.absmin[2] = (*ent).v.origin[2] - v1;
            (*ent).v.absmax[0] = (*ent).v.origin[0] + v1;
            (*ent).v.absmax[1] = (*ent).v.origin[1] + v1;
            (*ent).v.absmax[2] = (*ent).v.origin[2] + v1;
        } else {
            (*ent).v.absmin[0] = (*ent).v.origin[0] + (*ent).v.mins[0];
            (*ent).v.absmin[1] = (*ent).v.origin[1] + (*ent).v.mins[1];
            (*ent).v.absmin[2] = (*ent).v.origin[2] + (*ent).v.mins[2];
            (*ent).v.absmax[0] = (*ent).v.origin[0] + (*ent).v.maxs[0];
            (*ent).v.absmax[1] = (*ent).v.origin[1] + (*ent).v.maxs[1];
            (*ent).v.absmax[2] = (*ent).v.origin[2] + (*ent).v.maxs[2];
        }

        // to make items easier to pick up and allow them to be grabbed off of
        // shelves, the abs sizes are expanded
        //
        // COMPAT: ADR-010 -- `(int)ent->v.flags` truncates toward zero; `as`
        // saturates where C is UB for out-of-range floats (rule 8).
        if (*ent).v.flags as c_int & FL_ITEM != 0 {
            (*ent).v.absmin[0] -= 15.0;
            (*ent).v.absmin[1] -= 15.0;
            (*ent).v.absmax[0] += 15.0;
            (*ent).v.absmax[1] += 15.0;
        } else {
            // because movement is clipped an epsilon away from an actual edge,
            // we must fully check even when bounding boxes don't quite touch
            (*ent).v.absmin[0] -= 1.0;
            (*ent).v.absmin[1] -= 1.0;
            (*ent).v.absmin[2] -= 1.0;
            (*ent).v.absmax[0] += 1.0;
            (*ent).v.absmax[1] += 1.0;
            (*ent).v.absmax[2] += 1.0;
        }

        // link to PVS leafs
        (*ent).num_leafs = 0;
        if (*ent).v.modelindex != 0.0 {
            let wm = (*vm).worldmodel.cast::<QModel>();
            SV_FindTouchedLeafs(ent, (*wm).nodes);
        }

        g::World_Glue_PushGridEntityLinked(ent.cast());

        if (*ent).v.solid == SOLID_NOT {
            return 0;
        }

        // sv_phys.c is still C in M3 and is free to switch the VM; re-resolve.
        let vm = c::qcvm.cast::<QcVm>();

        // find the first node that the ent's box crosses
        let mut node = ptr::addr_of_mut!((*vm).areanodes).cast::<AreaNode>();
        loop {
            if (*node).axis == -1 {
                break;
            }
            let ax = (*node).axis as usize;
            if (*ent).v.absmin[ax] > (*node).dist {
                node = (*node).children[0];
            } else if (*ent).v.absmax[ax] < (*node).dist {
                node = (*node).children[1];
            } else {
                break; // crosses the node
            }
        }

        // link it in
        if (*ent).v.solid == SOLID_TRIGGER {
            insert_link_before(
                ptr::addr_of_mut!((*ent).area),
                ptr::addr_of_mut!((*node).trigger_edicts),
            );
        } else {
            insert_link_before(
                ptr::addr_of_mut!((*ent).area),
                ptr::addr_of_mut!((*node).solid_edicts),
            );
        }

        // if touch_triggers, touch all entities at this node and descend for more
        if touch_triggers {
            return sv_touch_links(ent);
        }
        0
    }
}

// ---------------------------------------------------------------------------
// point testing in hulls (world.c:536-611)

/// # Safety
/// `hull` must be a valid hull and `p` a `vec3_t`.
#[no_mangle]
pub unsafe extern "C" fn SV_HullPointContents(
    hull: *mut Hull,
    num: c_int,
    p: *mut c_float,
) -> c_int {
    // SAFETY: pointer contracts per the fn docs; the node index is range-checked
    // against the hull exactly as C does. Sys_Error is noreturn and does not
    // longjmp, so calling it from Rust is allowed (contract rule 5).
    unsafe {
        let mut num = num;
        while num >= 0 {
            if num < (*hull).firstclipnode || num > (*hull).lastclipnode {
                c::Sys_Error(SYS_ERR_HULL_POINT_CONTENTS.as_ptr());
            }

            let node = (*hull).clipnodes.offset(num as isize);
            let plane = (*hull).planes.offset((*node).planenum as isize);

            let d: c_float = if (*plane).type_ < 3 {
                *p.add((*plane).type_ as usize) - (*plane).dist
            } else {
                // COMPAT: ADR-010 -- DoublePrecisionDotProduct: the subtraction
                // happens in double and narrows once on the store to `float d`.
                (dp_dot(&(*plane).normal, &read3(p)) - f64::from((*plane).dist)) as c_float
            };

            num = if d < 0.0 {
                (*node).children[1]
            } else {
                (*node).children[0]
            };
        }
        num
    }
}

/// # Safety
/// `p` must be a `vec3_t`; the ambient qcvm's worldmodel must be loaded.
#[no_mangle]
pub unsafe extern "C" fn SV_PointContents(p: *mut c_float) -> c_int {
    // SAFETY: ADR-008 ambient qcvm.
    unsafe {
        let vm = c::qcvm.cast::<QcVm>();
        let wm = (*vm).worldmodel.cast::<QModel>();
        let mut cont = SV_HullPointContents(ptr::addr_of_mut!((*wm).hulls).cast::<Hull>(), 0, p);
        if (CONTENTS_CURRENT_DOWN..=CONTENTS_CURRENT_0).contains(&cont) {
            cont = CONTENTS_WATER;
        }
        cont
    }
}

/// # Safety
/// As `SV_PointContents`.
#[no_mangle]
pub unsafe extern "C" fn SV_TruePointContents(p: *mut c_float) -> c_int {
    // SAFETY: ADR-008 ambient qcvm.
    unsafe {
        let vm = c::qcvm.cast::<QcVm>();
        let wm = (*vm).worldmodel.cast::<QModel>();
        SV_HullPointContents(ptr::addr_of_mut!((*wm).hulls).cast::<Hull>(), 0, p)
    }
}

/// ADR-009 status core for `SV_PointContentsAllBsps`; `Quake/world_glue.c`
/// re-raises.
///
/// # Safety
/// `out` must be a writable `int`; `p` must be a `vec3_t`; `forent` may be
/// null.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_point_contents_all_bsps(
    out: *mut c_int,
    p: *mut c_float,
    forent: *mut Edict,
) -> Raise {
    // SAFETY: `vec3_origin` is quake-capi's own exported global.
    unsafe {
        let origin = ptr::addr_of_mut!(crate::mathlib::vec3_origin).cast::<c_float>();
        let mut trace = Trace::zeroed();
        let raised = quake_rs_sv_move(
            &mut trace,
            p,
            origin,
            origin,
            p,
            MOVE_NOMONSTERS | MOVE_HITALLCONTENTS,
            forent,
        );
        // world.c:590 reads trace.contents only after SV_Move returns, so a
        // raise leaves `*out` untouched -- exactly what the longjmp does.
        if raised != 0 {
            return raised;
        }
        if (CONTENTS_CURRENT_DOWN..=CONTENTS_CURRENT_0).contains(&trace.contents) {
            trace.contents = CONTENTS_WATER;
        }
        *out = trace.contents;
        0
    }
}

/// ADR-009 status core for `SV_TestEntityPosition`; `Quake/world_glue.c`
/// re-raises.
///
/// # Safety
/// `ent` must be a live edict of the ambient qcvm; `out` must be a writable
/// `edict_t *` slot.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_test_entity_position(
    ent: *mut Edict,
    out: *mut *mut Edict,
) -> Raise {
    // SAFETY: ADR-008 ambient qcvm; SV_Move dispatches no progs code.
    unsafe {
        let mut trace = Trace::zeroed();
        let raised = quake_rs_sv_move(
            &mut trace,
            ptr::addr_of_mut!((*ent).v.origin).cast::<c_float>(),
            ptr::addr_of_mut!((*ent).v.mins).cast::<c_float>(),
            ptr::addr_of_mut!((*ent).v.maxs).cast::<c_float>(),
            ptr::addr_of_mut!((*ent).v.origin).cast::<c_float>(),
            0,
            ent,
        );
        if raised != 0 {
            return raised;
        }

        if trace.startsolid {
            if !trace.ent.is_null() {
                *out = trace.ent;
                return 0;
            }
            let vm = c::qcvm.cast::<QcVm>();
            *out = (*vm).edicts;
            return 0;
        }
        *out = ptr::null_mut();
        0
    }
}

// ---------------------------------------------------------------------------
// line testing in hulls (world.c:618-918)

/// FTE's numerically-stable hull trace (world.c:641).
///
/// # Safety
/// `ctx`, `p1`, `p2` and `trace` must be valid; `num` indexes `ctx->clipnodes`.
#[no_mangle]
pub unsafe extern "C" fn Q1BSP_RecursiveHullTrace(
    ctx: *mut RhtCtx,
    num: c_int,
    p1f: c_float,
    p2f: c_float,
    p1: *mut c_float,
    p2: *mut c_float,
    trace: *mut Trace,
) -> c_int {
    // SAFETY: pointer contracts per the fn docs; the clipnode/plane arrays come
    // from a loaded hull.
    unsafe {
        let mut num = num;

        // world.c's `reenter:` loop; only `num` changes across iterations, and
        // the t1/t2 it computes are recomputed from ctx->start/end after it.
        let (node, plane) = loop {
            if num < 0 {
                /*hit a leaf*/
                (*trace).contents = num;
                if (*ctx).hitcontents & contentmask_fromq1(num) != 0 {
                    if (*trace).allsolid {
                        (*trace).startsolid = true;
                    }
                    return RHT_SOLID;
                }
                (*trace).allsolid = false;
                if num == CONTENTS_EMPTY {
                    (*trace).inopen = true;
                } else if num != CONTENTS_SOLID {
                    (*trace).inwater = true;
                }
                return RHT_EMPTY;
            }

            /*its a node; get the node info*/
            let node = (*ctx).clipnodes.offset(num as isize);
            let plane = (*ctx).planes.offset((*node).planenum as isize);

            let (t1, t2): (c_float, c_float) = if (*plane).type_ < 3 {
                let i = (*plane).type_ as usize;
                (*p1.add(i) - (*plane).dist, *p2.add(i) - (*plane).dist)
            } else {
                // COMPAT: ADR-010 -- DoublePrecisionDotProduct here, but the
                // *plain float* DotProduct for the ctx->start/ctx->end pass just
                // below (world.c:707-716); the two must not be unified.
                let n = (*plane).normal;
                let d = f64::from((*plane).dist);
                (
                    (dp_dot(&n, &read3(p1)) - d) as c_float,
                    (dp_dot(&n, &read3(p2)) - d) as c_float,
                )
            };

            /*if its completely on one side, resume on that side*/
            if t1 >= 0.0 && t2 >= 0.0 {
                num = (*node).children[0];
                continue;
            }
            if t1 < 0.0 && t2 < 0.0 {
                num = (*node).children[1];
                continue;
            }
            break (node, plane);
        };

        // COMPAT: ADR-010 -- plain (float) DotProduct against ctx->start/end,
        // feeding the first `midf = t1 / (t1 - t2)` in single precision.
        let (t1, t2): (c_float, c_float) = if (*plane).type_ < 3 {
            let i = (*plane).type_ as usize;
            (
                (*ctx).start[i] - (*plane).dist,
                (*ctx).end[i] - (*plane).dist,
            )
        } else {
            let n = (*plane).normal;
            (
                dot(&n, &(*ctx).start) - (*plane).dist,
                dot(&n, &(*ctx).end) - (*plane).dist,
            )
        };

        let side: usize = usize::from(t1 < 0.0);

        let mut midf: c_float = t1 / (t1 - t2);
        if midf < p1f {
            midf = p1f;
        }
        if midf > p2f {
            midf = p2f;
        }
        // VectorInterpolate (ctx->start, midf, ctx->end, mid)
        let mut mid = [
            (*ctx).start[0] + ((*ctx).end[0] - (*ctx).start[0]) * midf,
            (*ctx).start[1] + ((*ctx).end[1] - (*ctx).start[1]) * midf,
            (*ctx).start[2] + ((*ctx).end[2] - (*ctx).start[2]) * midf,
        ];

        let rht = Q1BSP_RecursiveHullTrace(
            ctx,
            (*node).children[side],
            p1f,
            midf,
            p1,
            mid.as_mut_ptr(),
            trace,
        );
        if rht != RHT_EMPTY && !(*trace).allsolid {
            return rht;
        }
        let rht = Q1BSP_RecursiveHullTrace(
            ctx,
            (*node).children[side ^ 1],
            midf,
            p2f,
            mid.as_mut_ptr(),
            p2,
            trace,
        );
        if rht != RHT_SOLID {
            return rht;
        }

        if side != 0 {
            /*we impacted the back of the node, so flip the plane*/
            (*trace).plane.dist = -(*plane).dist;
            (*trace).plane.normal[0] = -(*plane).normal[0];
            (*trace).plane.normal[1] = -(*plane).normal[1];
            (*trace).plane.normal[2] = -(*plane).normal[2];
        } else {
            /*we impacted the front of the node*/
            (*trace).plane.dist = (*plane).dist;
            (*trace).plane.normal = (*plane).normal;
        }
        // world.c:740/746 also assign `midf` here from the float t1/t2 pair and
        // then overwrite it unconditionally three lines down. Those dead stores
        // have no side effects and are omitted rather than transliterated (they
        // would trip the workspace's fatal `unused_assignments` lint).

        // COMPAT: ADR-010 -- DoublePrecisionDotProduct against the (possibly
        // flipped) trace plane, then a double-precision epsilon division that
        // narrows once on the store to `float midf` (world.c:749-751).
        let tn = (*trace).plane.normal;
        let td = f64::from((*trace).plane.dist);
        let t1 = (dp_dot(&tn, &(*ctx).start) - td) as c_float;
        let t2 = (dp_dot(&tn, &(*ctx).end) - td) as c_float;
        let mut midf = ((f64::from(t1) - DIST_EPSILON) / f64::from(t1 - t2)) as c_float;

        midf = clamp_f(0.0, midf, 1.0);
        (*trace).fraction = midf;
        (*trace).endpos = mid; // dead store in C too; kept (it is a real write)
        (*trace).endpos[0] = (*ctx).start[0] + ((*ctx).end[0] - (*ctx).start[0]) * midf;
        (*trace).endpos[1] = (*ctx).start[1] + ((*ctx).end[1] - (*ctx).start[1]) * midf;
        (*trace).endpos[2] = (*ctx).start[2] + ((*ctx).end[2] - (*ctx).start[2]) * midf;

        RHT_IMPACT
    }
}

/// `world.c:758` `SV_SlowRecursiveHullCheck` (file-private in C).
unsafe fn sv_slow_recursive_hull_check(
    hull: *mut Hull,
    num: c_int,
    p1f: c_float,
    p2f: c_float,
    p1: *mut c_float,
    p2: *mut c_float,
    trace: *mut Trace,
) -> bool {
    // SAFETY: pointer contracts as SV_RecursiveHullCheck's; Sys_Error is
    // noreturn (contract rule 5).
    unsafe {
        // check for empty
        if num < 0 {
            if num != CONTENTS_SOLID {
                (*trace).allsolid = false;
                if num == CONTENTS_EMPTY {
                    (*trace).inopen = true;
                } else {
                    (*trace).inwater = true;
                }
            } else {
                (*trace).startsolid = true;
            }
            return true; // empty
        }

        if num < (*hull).firstclipnode || num > (*hull).lastclipnode {
            c::Sys_Error(SYS_ERR_RECURSIVE_HULL_CHECK.as_ptr());
        }

        // find the point distances
        let node = (*hull).clipnodes.offset(num as isize);
        let plane = (*hull).planes.offset((*node).planenum as isize);

        let (t1, t2): (c_float, c_float) = if (*plane).type_ < 3 {
            let i = (*plane).type_ as usize;
            (*p1.add(i) - (*plane).dist, *p2.add(i) - (*plane).dist)
        } else {
            // COMPAT: ADR-010 -- DoublePrecisionDotProduct, narrowed on the store.
            let n = (*plane).normal;
            let d = f64::from((*plane).dist);
            (
                (dp_dot(&n, &read3(p1)) - d) as c_float,
                (dp_dot(&n, &read3(p2)) - d) as c_float,
            )
        };

        // world.c:804-816 keeps a disabled `#else` alternative; only the live
        // `#if 1` arm is ported.
        if t1 >= 0.0 && t2 >= 0.0 {
            return sv_slow_recursive_hull_check(
                hull,
                (*node).children[0],
                p1f,
                p2f,
                p1,
                p2,
                trace,
            );
        }
        if t1 < 0.0 && t2 < 0.0 {
            return sv_slow_recursive_hull_check(
                hull,
                (*node).children[1],
                p1f,
                p2f,
                p1,
                p2,
                trace,
            );
        }

        // put the crosspoint DIST_EPSILON pixels on the near side
        // COMPAT: ADR-010 -- DIST_EPSILON is a double literal, so both variants
        // evaluate in double and narrow once on the store to `float frac`.
        let mut frac: c_float = if t1 < 0.0 {
            ((f64::from(t1) + DIST_EPSILON) / f64::from(t1 - t2)) as c_float
        } else {
            ((f64::from(t1) - DIST_EPSILON) / f64::from(t1 - t2)) as c_float
        };
        // world.c's two guards (`if (frac < 0) frac = 0; if (frac > 1) frac = 1;`)
        // are exactly clamp_f's branch order, NaN behaviour included.
        frac = clamp_f(0.0, frac, 1.0);

        let mut midf: c_float = p1f + (p2f - p1f) * frac;
        let mut mid = [
            *p1 + frac * (*p2 - *p1),
            *p1.add(1) + frac * (*p2.add(1) - *p1.add(1)),
            *p1.add(2) + frac * (*p2.add(2) - *p1.add(2)),
        ];

        let side: usize = usize::from(t1 < 0.0);

        // move up to the node
        if !sv_slow_recursive_hull_check(
            hull,
            (*node).children[side],
            p1f,
            midf,
            p1,
            mid.as_mut_ptr(),
            trace,
        ) {
            return false;
        }

        if SV_HullPointContents(hull, (*node).children[side ^ 1], mid.as_mut_ptr())
            != CONTENTS_SOLID
        {
            // go past the node
            return sv_slow_recursive_hull_check(
                hull,
                (*node).children[side ^ 1],
                midf,
                p2f,
                mid.as_mut_ptr(),
                p2,
                trace,
            );
        }

        if (*trace).allsolid {
            return false; // never got out of the solid area
        }

        // the other side of the node is solid, this is the impact point
        if side == 0 {
            (*trace).plane.normal = (*plane).normal;
            (*trace).plane.dist = (*plane).dist;
        } else {
            // VectorSubtract (vec3_origin, plane->normal, trace->plane.normal)
            let origin = ptr::addr_of!(crate::mathlib::vec3_origin).read();
            (*trace).plane.normal[0] = origin[0] - (*plane).normal[0];
            (*trace).plane.normal[1] = origin[1] - (*plane).normal[1];
            (*trace).plane.normal[2] = origin[2] - (*plane).normal[2];
            (*trace).plane.dist = -(*plane).dist;
        }

        while SV_HullPointContents(hull, (*hull).firstclipnode, mid.as_mut_ptr()) == CONTENTS_SOLID
        {
            // shouldn't really happen, but does occasionally
            // COMPAT: ADR-010 -- `frac -= 0.1` promotes to double because the
            // literal is a double; writing `frac -= 0.1f32` would diverge.
            frac = (f64::from(frac) - 0.1f64) as c_float;
            if frac < 0.0 {
                (*trace).fraction = midf;
                (*trace).endpos = mid;
                g::World_Glue_DPrintBackupPast0();
                return false;
            }
            midf = p1f + (p2f - p1f) * frac;
            mid[0] = *p1 + frac * (*p2 - *p1);
            mid[1] = *p1.add(1) + frac * (*p2.add(1) - *p1.add(1));
            mid[2] = *p1.add(2) + frac * (*p2.add(2) - *p1.add(2));
        }

        (*trace).fraction = midf;
        (*trace).endpos = mid;

        false
    }
}

/// # Safety
/// `hull`, `p1`, `p2` and `trace` must be valid.
#[no_mangle]
pub unsafe extern "C" fn SV_RecursiveHullCheck(
    hull: *mut Hull,
    p1: *mut c_float,
    p2: *mut c_float,
    trace: *mut Trace,
    hitcontents: c_uint,
) -> bool {
    // SAFETY: pointer contracts per the fn docs.
    unsafe {
        // COMPAT: ADR-010 -- the dispatch gates on BOTH cvars; `!x` on a float
        // cvar value is `x == 0`, and pr_checkextension is pr_ext.c's, read
        // through quake-c-sys rather than duplicated.
        if sv_fte_recursivehullckeck_value() <= 0.0 || !pr_checkextension_on() {
            return sv_slow_recursive_hull_check(
                hull,
                (*hull).firstclipnode,
                0.0,
                1.0,
                p1,
                p2,
                trace,
            );
        }

        // COMPAT: ADR-010 -- exact float equality, NaN semantics included: a NaN
        // component sends the move down the trace path, not the point shortcut.
        if *p1 == *p2 && *p1.add(1) == *p2.add(1) && *p1.add(2) == *p2.add(2) {
            /*points cannot cross planes, so do it faster*/
            let cont = SV_HullPointContents(hull, (*hull).firstclipnode, p1);
            (*trace).contents = cont;
            if hitcontents & contentmask_fromq1(cont) != 0 {
                (*trace).startsolid = true;
            } else {
                (*trace).allsolid = false;
                if cont == CONTENTS_EMPTY {
                    (*trace).inopen = true;
                } else if cont != CONTENTS_SOLID {
                    (*trace).inwater = true;
                }
            }
            return true;
        }

        let mut ctx = RhtCtx {
            hitcontents,
            start: read3(p1),
            end: read3(p2),
            clipnodes: (*hull).clipnodes,
            planes: (*hull).planes,
        };
        Q1BSP_RecursiveHullTrace(&mut ctx, (*hull).firstclipnode, 0.0, 1.0, p1, p2, trace)
            != RHT_IMPACT
    }
}

// ---------------------------------------------------------------------------
// the move pipeline (world.c:927-1309)

/// `world.c:951` `DotProductTranspose (v, m, a)`
#[inline]
fn dot_transpose(v: &[c_float; 3], axis: &[[c_float; 3]; 3], a: usize) -> c_float {
    v[0] * axis[0][a] + v[1] * axis[1][a] + v[2] * axis[2][a]
}

/// Shared tail of the two rotated-BSP clip blocks (world.c:948-970 and the
/// hand-inlined copy at world.c:1168-1190). The two C blocks are textually
/// identical apart from the source of `angles`, so factoring them is
/// operation-for-operation equivalent.
unsafe fn clip_rotated(
    angles: &[c_float; 3],
    hull: *mut Hull,
    start_l: &[c_float; 3],
    end_l: &[c_float; 3],
    trace: *mut Trace,
    hitcontents: c_uint,
) {
    // SAFETY: `hull`/`trace` are valid; the rotation only touches locals.
    unsafe {
        let mut forward = [0.0f32; 3];
        let mut right = [0.0f32; 3];
        let mut up = [0.0f32; 3];
        m::angle_vectors(angles, &mut forward, &mut right, &mut up);
        // VectorInverse (axis[1])
        right[0] = -right[0];
        right[1] = -right[1];
        right[2] = -right[2];
        let axis = [forward, right, up];

        let mut start_r = [
            dot(start_l, &axis[0]),
            dot(start_l, &axis[1]),
            dot(start_l, &axis[2]),
        ];
        let mut end_r = [
            dot(end_l, &axis[0]),
            dot(end_l, &axis[1]),
            dot(end_l, &axis[2]),
        ];

        SV_RecursiveHullCheck(
            hull,
            start_r.as_mut_ptr(),
            end_r.as_mut_ptr(),
            trace,
            hitcontents,
        );

        let tmp = (*trace).endpos;
        (*trace).endpos[0] = dot_transpose(&tmp, &axis, 0);
        (*trace).endpos[1] = dot_transpose(&tmp, &axis, 1);
        (*trace).endpos[2] = dot_transpose(&tmp, &axis, 2);

        let tmp = (*trace).plane.normal;
        (*trace).plane.normal[0] = dot_transpose(&tmp, &axis, 0);
        (*trace).plane.normal[1] = dot_transpose(&tmp, &axis, 1);
        (*trace).plane.normal[2] = dot_transpose(&tmp, &axis, 2);
    }
}

/// ADR-009 status core for `SV_ClipMoveToEntity`; `Quake/world_glue.c`
/// re-raises.
///
/// # Safety
/// `out` must be a writable `trace_t`; `ent` must be a live edict;
/// `start`/`mins`/`maxs`/`end` must be `vec3_t`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_clip_move_to_entity(
    out: *mut Trace,
    ent: *mut Edict,
    start: *mut c_float,
    mins: *mut c_float,
    maxs: *mut c_float,
    end: *mut c_float,
    hitcontents: c_uint,
) -> Raise {
    // SAFETY: ADR-008 ambient qcvm; pointer contracts per the fn docs.
    unsafe {
        // fill in a default trace
        let mut trace = Trace::zeroed();
        trace.fraction = 1.0;
        trace.allsolid = true;
        trace.endpos = read3(end);

        // get the clipping hull
        let mut offset = [0.0f32; 3];
        let mut hull: *mut Hull = ptr::null_mut();
        let raised = quake_rs_sv_hull_for_entity(ent, mins, maxs, offset.as_mut_ptr(), &mut hull);
        if raised != 0 {
            return raised;
        }

        let mut start_l = [
            *start - offset[0],
            *start.add(1) - offset[1],
            *start.add(2) - offset[2],
        ];
        let mut end_l = [
            *end - offset[0],
            *end.add(1) - offset[1],
            *end.add(2) - offset[2],
        ];

        // trace a line through the apropriate clipping hull
        let vm = c::qcvm.cast::<QcVm>();
        if (*ent).v.solid == SOLID_BSP
            && pr_checkextension_on()
            && !m::is_axis_aligned_deg(&(*ent).v.angles)
            && (*vm).edicts != ent
        {
            // don't rotate the world entity's collisions (its not networked, and
            // some maps are buggy, resulting in screwed collisions)
            clip_rotated(
                &(*ent).v.angles,
                hull,
                &start_l,
                &end_l,
                &mut trace,
                hitcontents,
            );
        } else {
            SV_RecursiveHullCheck(
                hull,
                start_l.as_mut_ptr(),
                end_l.as_mut_ptr(),
                &mut trace,
                hitcontents,
            );
        }

        // fix trace up by the offset
        if trace.fraction != 1.0 {
            trace.endpos[0] += offset[0];
            trace.endpos[1] += offset[1];
            trace.endpos[2] += offset[2];
        }

        // did we clip the move?
        if trace.fraction < 1.0 || trace.startsolid {
            trace.ent = ent;
        }

        *out = trace;
        0
    }
}

/// `world.c:992` `SV_ClipToLinks` (file-private in C).
///
/// ADR-009: a non-zero clip status abandons the entity walk and both recursive
/// descents immediately, leaving the state exactly where C's `longjmp` would.
unsafe fn sv_clip_to_links(node: *mut AreaNode, clip: &mut MoveClip) -> Raise {
    // SAFETY: nothing in this walk dispatches progs code, so the area list
    // cannot be mutated under it (ADR-006). Sys_Error is noreturn.
    unsafe {
        let vm = c::qcvm.cast::<QcVm>();

        // touch linked edicts
        let head = ptr::addr_of_mut!((*node).solid_edicts);
        let mut l = (*head).next;
        while l != head {
            let next = (*l).next;
            let touch = edict_from_area(l);
            l = next;

            if (*touch).v.solid == SOLID_NOT {
                continue;
            }
            if touch == clip.passedict {
                continue;
            }
            if (*touch).v.solid == SOLID_TRIGGER {
                c::Sys_Error(SYS_ERR_TRIGGER_IN_CLIP_LIST.as_ptr());
            }

            if clip.type_ == MOVE_NOMONSTERS && (*touch).v.solid != SOLID_BSP {
                continue;
            }

            if clip.boxmins[0] > (*touch).v.absmax[0]
                || clip.boxmins[1] > (*touch).v.absmax[1]
                || clip.boxmins[2] > (*touch).v.absmax[2]
                || clip.boxmaxs[0] < (*touch).v.absmin[0]
                || clip.boxmaxs[1] < (*touch).v.absmin[1]
                || clip.boxmaxs[2] < (*touch).v.absmin[2]
            {
                continue;
            }

            if !clip.passedict.is_null()
                && (*clip.passedict).v.size[0] != 0.0
                && (*touch).v.size[0] == 0.0
            {
                continue; // points never interact
            }

            // might intersect, so do an exact clip
            if clip.trace.allsolid {
                return 0;
            }
            if !clip.passedict.is_null() {
                if prog_to_edict(vm, (*touch).v.owner) == clip.passedict {
                    continue; // don't clip against own missiles
                }
                if prog_to_edict(vm, (*clip.passedict).v.owner) == touch {
                    continue; // don't clip against owner
                }
            }

            // COMPAT: ADR-010 -- `(int)touch->v.flags` truncates toward zero and
            // saturates in Rust where C is UB (rule 8).
            let monster = (*touch).v.flags as c_int & FL_MONSTER != 0;
            let (bmins, bmaxs) = if monster {
                (clip.mins2.as_mut_ptr(), clip.maxs2.as_mut_ptr())
            } else {
                (clip.mins, clip.maxs)
            };

            let mut trace = Trace::zeroed();
            if (*touch).v.skin < 0.0 {
                // COMPAT: ADR-010 -- `1 << -(int)touch->v.skin` is a *signed* int
                // shift in C, converted to unsigned by the `&`; the float
                // truncation saturates here where C is UB (rule 8).
                let bit = 1i32.wrapping_shl(((*touch).v.skin as c_int).wrapping_neg() as c_uint);
                if clip.hitcontents & (bit as c_uint) == 0 {
                    continue; // not solid, don't bother trying to clip.
                }
                let raised = quake_rs_sv_clip_move_to_entity(
                    &mut trace,
                    touch,
                    clip.start,
                    bmins,
                    bmaxs,
                    clip.end,
                    !(1u32 << 1), /* ~(1u << -CONTENTS_EMPTY) */
                );
                if raised != 0 {
                    return raised;
                }
                if trace.contents != CONTENTS_EMPTY {
                    trace.contents = (*touch).v.skin as c_int;
                }
            } else {
                let raised = quake_rs_sv_clip_move_to_entity(
                    &mut trace,
                    touch,
                    clip.start,
                    bmins,
                    bmaxs,
                    clip.end,
                    clip.hitcontents,
                );
                if raised != 0 {
                    return raised;
                }
            }

            if trace.allsolid || trace.startsolid || trace.fraction < clip.trace.fraction {
                trace.ent = touch;
                if clip.trace.startsolid {
                    clip.trace = trace;
                    clip.trace.startsolid = true;
                } else {
                    clip.trace = trace;
                }
            } else if trace.startsolid {
                clip.trace.startsolid = true;
            }
        }

        // recurse down both sides
        if (*node).axis == -1 {
            return 0;
        }

        let ax = (*node).axis as usize;
        if clip.boxmaxs[ax] > (*node).dist {
            let raised = sv_clip_to_links((*node).children[0], clip);
            if raised != 0 {
                return raised;
            }
        }
        if clip.boxmins[ax] < (*node).dist {
            return sv_clip_to_links((*node).children[1], clip);
        }
        0
    }
}

/// `world.c:1076` `World_ClipToNetwork` (file-private in C).
///
/// The C body hand-inlines `SV_ClipMoveToEntity` and `SV_HullForEntity` against
/// `entity_t` rather than `edict_t`; that inlined form is kept here, with only
/// the rotated-clip tail shared with `SV_ClipMoveToEntity` via `clip_rotated`.
///
/// ADR-009: returns a `Host_Guard` status for symmetry with `sv_clip_to_links`.
/// No call on this path is raise-capable today -- the hand-inlined hull work
/// runs against `entity_t`, never `PR_GetString` -- so the status is always 0.
unsafe fn world_clip_to_network(clip: &mut MoveClip) -> Raise {
    // SAFETY: `cl` and `entity_t` have no ADR-011 mirror before M7, so every
    // read goes through world_glue.c's accessors; `qmodel_t` is mirrored.
    unsafe {
        let num_entities = g::World_Glue_ClNumEntities();
        let mut i: c_int = 1;
        while i < num_entities {
            let ent = g::World_Glue_ClEntity(i);
            i += 1;
            if ent.is_null() {
                continue;
            }

            let mut solidsize: c_uint = 0;
            let mut model_raw: *mut c_void = ptr::null_mut();
            let mut origin = [0.0f32; 3];
            let mut angles = [0.0f32; 3];
            let mut skinnum: c_int = 0;
            g::World_Glue_EntClipInfo(
                ent,
                &mut solidsize,
                &mut model_raw,
                origin.as_mut_ptr(),
                angles.as_mut_ptr(),
                &mut skinnum,
            );
            let model = model_raw.cast::<QModel>();

            if model.is_null() {
                continue;
            }
            if solidsize == ES_SOLID_NOT {
                continue;
            }
            if clip.type_ == MOVE_NOMONSTERS && solidsize != ES_SOLID_BSP {
                continue;
            }

            // might intersect, so do an exact clip
            if clip.trace.allsolid {
                return 0;
            }

            // fill in a default trace
            let mut trace = Trace::zeroed();
            trace.fraction = 1.0;
            trace.allsolid = true;
            trace.endpos = read3(clip.end);

            // get the clipping hull
            let mut offset;
            let hull: *mut Hull;
            if solidsize == ES_SOLID_BSP && (*model).type_ == MOD_BRUSH {
                // explicit hulls in the BSP model
                let size0 = *clip.maxs - *clip.mins;
                let hulls = ptr::addr_of_mut!((*model).hulls).cast::<Hull>();
                hull = if size0 < 3.0 {
                    hulls
                } else if size0 <= 32.0 {
                    hulls.add(1)
                } else {
                    hulls.add(2)
                };

                // calculate an offset value to center the origin
                offset = [
                    (*hull).clip_mins[0] - *clip.mins,
                    (*hull).clip_mins[1] - *clip.mins.add(1),
                    (*hull).clip_mins[2] - *clip.mins.add(2),
                ];
                offset[0] += origin[0];
                offset[1] += origin[1];
                offset[2] += origin[2];
            } else {
                // create a temp hull from bounding box sizes
                let mut touch_mins = [0.0f32; 3];
                let mut touch_maxs = [0.0f32; 3];
                touch_maxs[1] = (solidsize & 255) as c_float;
                touch_maxs[0] = touch_maxs[1];
                touch_mins[1] = -touch_maxs[0];
                touch_mins[0] = touch_mins[1];
                touch_mins[2] = -(((solidsize >> 8) & 255) as c_int) as c_float;
                // COMPAT: ADR-010 -- `((solidsize >> 16) & 65535) - 32768` is
                // *unsigned* arithmetic in C (the int 32768 converts up), so a
                // "negative" result wraps to a huge unsigned before the float
                // conversion; `wrapping_sub` reproduces that exactly.
                touch_maxs[2] = ((solidsize >> 16) & 65535).wrapping_sub(32768) as c_float;

                let mut hullmins = [
                    touch_mins[0] - *clip.maxs,
                    touch_mins[1] - *clip.maxs.add(1),
                    touch_mins[2] - *clip.maxs.add(2),
                ];
                let mut hullmaxs = [
                    touch_maxs[0] - *clip.mins,
                    touch_maxs[1] - *clip.mins.add(1),
                    touch_maxs[2] - *clip.mins.add(2),
                ];
                hull = SV_HullForBox(hullmins.as_mut_ptr(), hullmaxs.as_mut_ptr());
                offset = origin;
            }

            let mut start_l = [
                *clip.start - offset[0],
                *clip.start.add(1) - offset[1],
                *clip.start.add(2) - offset[2],
            ];
            let mut end_l = [
                *clip.end - offset[0],
                *clip.end.add(1) - offset[1],
                *clip.end.add(2) - offset[2],
            ];

            // trace a line through the apropriate clipping hull
            if solidsize == ES_SOLID_BSP
                && (angles[0] != 0.0 || angles[1] != 0.0 || angles[2] != 0.0)
                && pr_checkextension_on()
            {
                clip_rotated(
                    &angles,
                    hull,
                    &start_l,
                    &end_l,
                    &mut trace,
                    clip.hitcontents,
                );
            } else {
                SV_RecursiveHullCheck(
                    hull,
                    start_l.as_mut_ptr(),
                    end_l.as_mut_ptr(),
                    &mut trace,
                    clip.hitcontents,
                );
            }

            // fix trace up by the offset
            if trace.fraction != 1.0 {
                trace.endpos[0] += offset[0];
                trace.endpos[1] += offset[1];
                trace.endpos[2] += offset[2];
            }

            let vm = c::qcvm.cast::<QcVm>();

            // did we clip the move?
            if trace.fraction < 1.0 || trace.startsolid {
                trace.ent = (*vm).edicts;
            }

            if trace.contents == CONTENTS_SOLID && skinnum < 0 {
                trace.contents = skinnum;
            }
            // COMPAT: ADR-010 -- signed `1 << -trace.contents`, converted to
            // unsigned by the `&`; wrapping_shl keeps C's hardware behaviour.
            let bit = 1i32.wrapping_shl(trace.contents.wrapping_neg() as c_uint);
            if (bit as c_uint) & clip.hitcontents == 0 {
                continue;
            }

            if trace.allsolid || trace.startsolid || trace.fraction < clip.trace.fraction {
                trace.ent = (*vm).edicts; // no real way to return entity number.
                if clip.trace.startsolid {
                    clip.trace = trace;
                    clip.trace.startsolid = true;
                } else {
                    clip.trace = trace;
                }
            } else if trace.startsolid {
                clip.trace.startsolid = true;
            }
        }
        0
    }
}

/// # Safety
/// All six pointers must be `vec3_t`.
#[no_mangle]
pub unsafe extern "C" fn SV_MoveBounds(
    start: *mut c_float,
    mins: *mut c_float,
    maxs: *mut c_float,
    end: *mut c_float,
    boxmins: *mut c_float,
    boxmaxs: *mut c_float,
) {
    // SAFETY: vec3_t contracts per the fn docs. world.c keeps a disabled `#if 0`
    // "test against everything" variant; only the live arm is ported.
    unsafe {
        for i in 0..3usize {
            if *end.add(i) > *start.add(i) {
                *boxmins.add(i) = *start.add(i) + *mins.add(i) - 1.0;
                *boxmaxs.add(i) = *end.add(i) + *maxs.add(i) + 1.0;
            } else {
                *boxmins.add(i) = *end.add(i) + *mins.add(i) - 1.0;
                *boxmaxs.add(i) = *start.add(i) + *maxs.add(i) + 1.0;
            }
        }
    }
}

/// ADR-009 status core for `SV_Move`; `Quake/world_glue.c` re-raises.
///
/// # Safety
/// `out` must be a writable `trace_t`; `start`/`mins`/`maxs`/`end` must be
/// `vec3_t`; `passedict` may be null.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_move(
    out: *mut Trace,
    start: *mut c_float,
    mins: *mut c_float,
    maxs: *mut c_float,
    end: *mut c_float,
    type_: c_int,
    passedict: *mut Edict,
) -> Raise {
    // SAFETY: ADR-008 ambient qcvm; nothing here dispatches progs code, so no
    // arena pointer is invalidated mid-call (ADR-006).
    unsafe {
        let vm = c::qcvm.cast::<QcVm>();

        let hitcontents = if type_ & MOVE_HITALLCONTENTS != 0 {
            !0u32
        } else {
            CONTENTMASK_ANYSOLID
        };

        // clip to world
        let mut world_trace = Trace::zeroed();
        let raised = quake_rs_sv_clip_move_to_entity(
            &mut world_trace,
            (*vm).edicts,
            start,
            mins,
            maxs,
            end,
            hitcontents,
        );
        if raised != 0 {
            return raised;
        }

        let mut clip = MoveClip {
            boxmins: [0.0; 3],
            boxmaxs: [0.0; 3],
            mins: ptr::null_mut(),
            maxs: ptr::null_mut(),
            mins2: [0.0; 3],
            maxs2: [0.0; 3],
            start: ptr::null_mut(),
            end: ptr::null_mut(),
            trace: world_trace,
            type_: 0,
            hitcontents,
            passedict: ptr::null_mut(),
        };

        clip.start = start;
        clip.end = end;
        clip.mins = mins;
        clip.maxs = maxs;
        clip.type_ = type_ & 3;
        clip.passedict = passedict;

        if type_ == MOVE_MISSILE {
            for i in 0..3usize {
                clip.mins2[i] = -15.0;
                clip.maxs2[i] = 15.0;
            }
        } else {
            clip.mins2 = read3(mins);
            clip.maxs2 = read3(maxs);
        }

        // create the bounding box of the entire move
        let mut mins2 = clip.mins2;
        let mut maxs2 = clip.maxs2;
        let mut boxmins = [0.0f32; 3];
        let mut boxmaxs = [0.0f32; 3];
        SV_MoveBounds(
            start,
            mins2.as_mut_ptr(),
            maxs2.as_mut_ptr(),
            end,
            boxmins.as_mut_ptr(),
            boxmaxs.as_mut_ptr(),
        );
        clip.boxmins = boxmins;
        clip.boxmaxs = boxmaxs;

        // clip to entities
        let raised = sv_clip_to_links(
            ptr::addr_of_mut!((*vm).areanodes).cast::<AreaNode>(),
            &mut clip,
        );
        if raised != 0 {
            return raised;
        }

        if g::World_Glue_QcvmIsClient() != 0 {
            let raised = world_clip_to_network(&mut clip);
            if raised != 0 {
                return raised;
            }
        }

        if !clip.trace.ent.is_null() && (*clip.trace.ent).free {
            // ADR-009: world.c:1306's assert_always reaches Host_Error, so it
            // goes through the guard; the expression / file / line text is the
            // one world.c would have produced.
            let raised = g::World_Glue_AssertFailed(
                ASSERT_TRACE_ENT_FREE.as_ptr(),
                ASSERT_FILE.as_ptr(),
                1306,
            );
            if raised != 0 {
                return raised;
            }
        }

        *out = clip.trace;
        0
    }
}
