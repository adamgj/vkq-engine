//! C ABI shims for `Quake/sv_phys.c` (Rust migration Phase 7 M4).
//!
//! Near-transliteration of the server physics tick: the pushable-entity cache
//! and its spatial hash, `SV_CheckVelocity`/`SV_RunThink`/`SV_Impact`, the
//! `SV_FlyMove` slide solver, the analytic-gravity pair, the pusher-support
//! bookkeeping added for `sv_gameplayfix_elevators 3`, `SV_PushMove`, the
//! client movement path and the `SV_Physics` driver. Everything that was
//! `static` in C is private here.
//!
//! ADR-009 audit. The C paths reachable from this module that can `Host_Error`
//! are: the eight `PR_ExecuteProgram` dispatches (`sv_phys.c:372, 426, 436,
//! 1561, 1613, 2009, 2067, 2339`), `PR_GetString` behind the two NaN warnings
//! (`:318`, `:323`), `NUM_FOR_EDICT` / `EDICT_NUM` (`:855, 897, 987, 994,
//! 1254, 1541`), `COM_Assert_Failed` on the main thread (`:413, 598, 1262,
//! 1983, 2011, 2063`), `SV_StartSound` (`:2139, 2148, 2270`), `Host_EndGame`
//! (`:2055, 2429`) and the whole `world.c` pipeline
//! (`SV_Move`/`SV_ClipMoveToEntity`/`SV_TestEntityPosition`/`SV_LinkEdict`).
//! Each goes through a `SvPhys_Glue_*` or `World_Glue_*` trampoline that wraps
//! the call in `Host_Guard`, or through one of `crate::world`'s `quake_rs_*`
//! status cores. A non-zero status is returned to the caller immediately, so
//! every intermediate function abandons its remaining work exactly where C's
//! `longjmp` would have left it -- including the bookkeeping C skips (see the
//! `// COMPAT: ADR-009` markers on the origin/solid save-restore pairs and on
//! the `SV_Physics` epilogue). No jump ever unwinds a Rust frame.
//!
//! Four entry points are therefore `quake_rs_*` status cores:
//! `quake_rs_sv_check_all_ents`, `quake_rs_sv_check_velocity`,
//! `quake_rs_sv_check_water_transition` and `quake_rs_sv_physics`.
//! `Quake/sv_phys_glue.c` owns every re-raise; nothing here calls
//! `Host_Reraise`. `SV_PushGridEntityLinked` is exported plain: it reaches
//! only the hash map and `Mem_Realloc`, neither of which raises.
//!
//! `Sys_Error` only aborts the process, so the `SV_FlyMove` site is called
//! directly (Phase 5/6 precedent).

use core::ffi::{c_float, c_int, c_uint, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::sv_phys as g;
use quake_c_sys::world as w;
use quake_math::mathlib as m;
use quake_types::progs::{Edict, GlobalVars, QcVm, MAX_EDICTS};
use quake_util::hash_map::{hashers, QHashMap};

use crate::world::{PlaneT, Trace};

/// A `Host_Guard` status: `HOST_GUARD_OK` (0) or the code the guarded frame
/// caught. Non-zero must be returned to `Quake/sv_phys_glue.c` untouched.
type Raise = c_int;

// ---------------------------------------------------------------------------
// engine constants (quakedef.h / server.h / progdefs.q1 / world.h)

/// `progdefs.q1` `MOVETYPE_NONE`
const MOVETYPE_NONE: c_float = 0.0;
/// `MOVETYPE_WALK`
const MOVETYPE_WALK: c_float = 3.0;
/// `MOVETYPE_STEP`
const MOVETYPE_STEP: c_float = 4.0;
/// `MOVETYPE_FLY`
const MOVETYPE_FLY: c_float = 5.0;
/// `MOVETYPE_TOSS`
const MOVETYPE_TOSS: c_float = 6.0;
/// `MOVETYPE_PUSH`
const MOVETYPE_PUSH: c_float = 7.0;
/// `MOVETYPE_NOCLIP`
const MOVETYPE_NOCLIP: c_float = 8.0;
/// `MOVETYPE_FLYMISSILE`
const MOVETYPE_FLYMISSILE: c_float = 9.0;
/// `MOVETYPE_BOUNCE`
const MOVETYPE_BOUNCE: c_float = 10.0;
/// `MOVETYPE_GIB`
const MOVETYPE_GIB: c_float = 11.0;

/// `server.h SOLID_NOT`
const SOLID_NOT: c_float = 0.0;
/// `SOLID_TRIGGER`
const SOLID_TRIGGER: c_float = 1.0;
/// `SOLID_BBOX`
const SOLID_BBOX: c_float = 2.0;
/// `SOLID_SLIDEBOX`
const SOLID_SLIDEBOX: c_float = 3.0;
/// `SOLID_BSP`
const SOLID_BSP: c_float = 4.0;

/// `progdefs.q1 FL_FLY`
const FL_FLY: c_int = 1;
/// `FL_SWIM`
const FL_SWIM: c_int = 2;
/// `FL_ONGROUND`
const FL_ONGROUND: c_int = 512;
/// `FL_WATERJUMP`
const FL_WATERJUMP: c_int = 2048;

/// `world.h MOVE_NORMAL`
const MOVE_NORMAL: c_int = 0;
/// `MOVE_NOMONSTERS`
const MOVE_NOMONSTERS: c_int = 1;
/// `MOVE_MISSILE`
const MOVE_MISSILE: c_int = 2;
/// `world.h CONTENTMASK_ANYSOLID`
const CONTENTMASK_ANYSOLID: c_uint = 260;

/// `bspfile.h CONTENTS_EMPTY`
const CONTENTS_EMPTY: c_int = -1;
/// `CONTENTS_WATER`
const CONTENTS_WATER: c_int = -3;

/// `quakedef.h MAX_PHYSICS_FREQ`
const MAX_PHYSICS_FREQ: f64 = 72.0;

/// `world.h DIST_EPSILON`, spelled `(0.03125)` -- a double in C.
///
/// COMPAT: ADR-010 -- keeping it `f64` preserves the promotions of every
/// expression it appears in.
const DIST_EPSILON: f64 = 0.03125;

/// `sv_phys.c:62` `PUSH_CONTACT_EPSILON (2 * DIST_EPSILON)`
const PUSH_CONTACT_EPSILON: f64 = 2.0 * DIST_EPSILON;
/// `sv_phys.c:63`
const MIN_WALK_NORMAL: c_float = 0.7;
/// `sv_phys.c:64`
const STEPSIZE: c_int = 18;
/// `sv_phys.c:79`
const PUSH_GRID_CELL_SHIFT: c_int = 8;
/// `sv_phys.c:80`
const PUSH_GRID_MAX_LARGE: usize = 1024;
/// `sv_phys.c:453`
const STOP_EPSILON: c_float = 0.1;
/// `sv_phys.c:521`
const MAX_CLIP_PLANES: usize = 5;

/// `mathlib.h nanmask`
const NANMASK: u32 = 255 << 23;

const SYS_ERR_FLYMOVE_NO_TRACE_ENT: &core::ffi::CStr = c"SV_FlyMove: !trace.ent";
const ASSERT_FILE: &core::ffi::CStr = c"sv_phys.c";
const ASSERT_E1_E2_FREE: &core::ffi::CStr = c"!e1->free && !e2->free";
const ASSERT_ENT_FREE: &core::ffi::CStr = c"!ent->free";
const ASSERT_TRACE_ENT_FREE: &core::ffi::CStr = c"!trace.ent->free";

const SND_H2OHIT1: &core::ffi::CStr = c"misc/h2ohit1.wav";
const SND_DLAND2: &core::ffi::CStr = c"demon/dland2.wav";
const FIELD_GRAVITY: &core::ffi::CStr = c"gravity";

// ---------------------------------------------------------------------------
// sv_phys.c-local aggregates (no header, so no ADR-007 mirror)

/// `sv_phys.c:83-87` `push_grid_entry_t`
#[repr(C)]
#[derive(Clone, Copy)]
struct PushGridEntry {
    ent: *mut Edict,
    next: c_int,
}

/// `sv_phys.c:478-483` `sv_pusher_contact_t`
#[derive(Clone, Copy, PartialEq, Eq)]
enum PusherContact {
    None,
    SupportFloor,
    SupportSide,
}

/// `sv_phys.c:709-715` `sv_client_move_frame_state_t`. `None` is 0 so the
/// `memset`-zeroed support records decode to it, exactly as in C.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MoveFrameState {
    None,
    Ground,
    Airborne,
    AirborneWorldVelocity,
}

/// `sv_phys.c:717-723` `sv_client_move_frame_t`
#[derive(Clone, Copy)]
struct ClientMoveFrame {
    pusher: *mut Edict,
    state: MoveFrameState,
    pusher_velocity: [c_float; 3],
    support_normal: [c_float; 3],
}

/// `sv_phys.c:725-732` `sv_pusher_support_record_t`
#[derive(Clone, Copy)]
struct PusherSupportRecord {
    frame: c_uint,
    pusher_entnum: c_int,
    state: MoveFrameState,
    pusher_velocity: [c_float; 3],
    pusher_move: [c_float; 3],
}

impl PusherSupportRecord {
    /// The `memset (..., 0, ...)` state `sv_phys.c:741` and `:751` install.
    const ZERO: Self = Self {
        frame: 0,
        pusher_entnum: 0,
        state: MoveFrameState::None,
        pusher_velocity: [0.0; 3],
        pusher_move: [0.0; 3],
    };
}

/// The two results `SV_PusherAffectsEntity` (`sv_phys.c:1163`) produces,
/// bundled so its Rust signature stays inside the clippy argument limit.
struct PusherAffect {
    affects: bool,
    riding: bool,
}

// ---------------------------------------------------------------------------
// file-private state (sv_phys.c:66-99, :734-736, and the SV_PushMove statics)

/// `static edict_t *sv_walk_support_pusher;`
static mut SV_WALK_SUPPORT_PUSHER: *mut Edict = ptr::null_mut();
/// `static vec3_t sv_walk_support_normal;`
static mut SV_WALK_SUPPORT_NORMAL: [c_float; 3] = [0.0; 3];

/// `static edict_t *pushable_ent_cache[MAX_EDICTS];`
static mut PUSHABLE_ENT_CACHE: [*mut Edict; MAX_EDICTS] = [ptr::null_mut(); MAX_EDICTS];
/// `static int num_pushable_ent_cache;`
static mut NUM_PUSHABLE_ENT_CACHE: c_int = 0;

/// `static hash_map_t *push_grid_map;` -- built lazily, like the C
/// `if (!push_grid_map) push_grid_map = HashMap_Create (...)`.
static mut PUSH_GRID_MAP: Option<QHashMap> = None;
/// `static push_grid_entry_t *push_grid_entries;`
static mut PUSH_GRID_ENTRIES: *mut PushGridEntry = ptr::null_mut();
/// `static int push_grid_entries_capacity;`
static mut PUSH_GRID_ENTRIES_CAPACITY: c_int = 0;
/// `static int push_grid_num_entries;`
static mut PUSH_GRID_NUM_ENTRIES: c_int = 0;
/// `static edict_t *push_grid_large[PUSH_GRID_MAX_LARGE];`
static mut PUSH_GRID_LARGE: [*mut Edict; PUSH_GRID_MAX_LARGE] =
    [ptr::null_mut(); PUSH_GRID_MAX_LARGE];
/// `static int push_grid_num_large;`
static mut PUSH_GRID_NUM_LARGE: c_int = 0;
/// `static int push_grid_tail_start;`
static mut PUSH_GRID_TAIL_START: c_int = 0;
/// `static qboolean push_grid_valid;`
static mut PUSH_GRID_VALID: bool = false;
/// `static qboolean push_grid_active;`
static mut PUSH_GRID_ACTIVE: bool = false;
/// `static qboolean push_cache_active;`
static mut PUSH_CACHE_ACTIVE: bool = false;
/// `static qcvm_t *push_grid_qcvm;`
static mut PUSH_GRID_QCVM: *mut QcVm = ptr::null_mut();

/// `static sv_pusher_support_record_t sv_pusher_support[MAX_EDICTS];`
static mut SV_PUSHER_SUPPORT: [PusherSupportRecord; MAX_EDICTS] =
    [PusherSupportRecord::ZERO; MAX_EDICTS];
/// `static unsigned sv_pusher_support_frame;`
static mut SV_PUSHER_SUPPORT_FRAME: c_uint = 0;
/// `static edict_t *sv_pusher_support_edicts;`
static mut SV_PUSHER_SUPPORT_EDICTS: *mut Edict = ptr::null_mut();

/// `SV_PushMove`: `static edict_t *moved_edict[MAX_EDICTS];`
static mut MOVED_EDICT: [*mut Edict; MAX_EDICTS] = [ptr::null_mut(); MAX_EDICTS];
/// `static vec3_t moved_from[MAX_EDICTS];`
static mut MOVED_FROM: [[c_float; 3]; MAX_EDICTS] = [[0.0; 3]; MAX_EDICTS];
/// `static sv_pusher_support_record_t moved_support[MAX_EDICTS];`
static mut MOVED_SUPPORT: [PusherSupportRecord; MAX_EDICTS] =
    [PusherSupportRecord::ZERO; MAX_EDICTS];
/// `static edict_t *push_candidates[MAX_EDICTS];`
static mut PUSH_CANDIDATES: [*mut Edict; MAX_EDICTS] = [ptr::null_mut(); MAX_EDICTS];

// ---------------------------------------------------------------------------
// small helpers

/// `world.h` `trace_t` zero state. `crate::world::Trace::zeroed` is private to
/// that module, so the same constructor is repeated here.
const fn trace_zeroed() -> Trace {
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

/// `q_max_i` (`q_minmax.h`).
#[inline]
fn q_max_i(a: c_int, b: c_int) -> c_int {
    if a > b {
        a
    } else {
        b
    }
}

#[inline]
fn q_min_f(a: c_float, b: c_float) -> c_float {
    if a < b {
        a
    } else {
        b
    }
}

#[inline]
fn q_max_f(a: c_float, b: c_float) -> c_float {
    if a > b {
        a
    } else {
        b
    }
}

/// `mathlib.h` `IS_NAN (x)`.
///
/// COMPAT: ADR-010 -- the C macro type-puns the float and masks the exponent,
/// so it reports `true` for the infinities too. `f32::is_nan` would not.
#[inline]
fn is_nan(x: c_float) -> bool {
    (x.to_bits() & NANMASK) == NANMASK
}

/// `q_minmax.h` `Q_rint (x)` on a `float` argument: the `+ 0.5` / `- 0.5`
/// literals are doubles, so `x` is promoted before the truncation.
///
/// COMPAT: ADR-010 -- keep the promotion and the truncate-toward-zero cast.
#[inline]
fn q_rint_f(x: c_float) -> c_int {
    if x > 0.0 {
        (x as f64 + 0.5) as c_int
    } else {
        (x as f64 - 0.5) as c_int
    }
}

/// Reads a C `cvar_t`'s `.value` without forming a reference to the static.
#[inline]
unsafe fn cvar_value(var: *const c::cvar_t) -> c_float {
    // SAFETY: `var` always points at a `cvar_t` static owned by
    // `Quake/sv_phys_glue.c` or `Quake/host.c`; cvars are single-threaded
    // engine state.
    unsafe { ptr::addr_of!((*var).value).read() }
}

#[inline]
unsafe fn sv_gravity_value() -> c_float {
    // SAFETY: see `cvar_value`.
    unsafe { cvar_value(ptr::addr_of!(g::sv_gravity)) }
}

#[inline]
unsafe fn sv_elevators_value() -> c_float {
    // SAFETY: see `cvar_value`.
    unsafe { cvar_value(ptr::addr_of!(g::sv_gameplayfix_elevators)) }
}

#[inline]
unsafe fn sv_speeds_on() -> bool {
    // SAFETY: see `cvar_value`. C's `if (sv_speeds.value)` is `!= 0`.
    unsafe { cvar_value(ptr::addr_of!(g::sv_speeds)) != 0.0 }
}

/// `qcvm == &sv.qcvm`.
#[inline]
unsafe fn qcvm_is_server() -> bool {
    // SAFETY: `Quake/sv_phys_glue.c` owns the comparison; `server_t` has no
    // ADR-011 mirror in Phase 7.
    unsafe { g::SvPhys_Glue_QcvmIsServer() != 0 }
}

/// `progs.h` `NEXT_EDICT (e)`
#[inline]
unsafe fn next_edict(vm: *mut QcVm, e: *mut Edict) -> *mut Edict {
    // SAFETY: pointer arithmetic only, byte-for-byte the C macro.
    unsafe {
        e.cast::<u8>()
            .wrapping_offset((*vm).edict_size as isize)
            .cast::<Edict>()
    }
}

/// `progs.h` `PROG_TO_EDICT (e)`
#[inline]
unsafe fn prog_to_edict(vm: *mut QcVm, p: c_int) -> *mut Edict {
    // SAFETY: pointer arithmetic only; the C macro has no bounds check either.
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
    // SAFETY: pointer arithmetic only, byte-for-byte the C macro.
    unsafe { e.cast::<u8>().offset_from((*vm).edicts.cast::<u8>()) as c_int }
}

/// `qcvm->globals` viewed as `globalvars_t` -- the engine keeps
/// `pr_global_struct` and `qcvm->globals` in lockstep.
#[inline]
unsafe fn globals(vm: *mut QcVm) -> *mut GlobalVars {
    // SAFETY: the ambient qcvm is always loaded when sv_phys runs (ADR-008).
    unsafe { (*vm).globals.cast::<GlobalVars>() }
}

/// `mathlib.c` `VectorMA (veca, scale, vecb, vecc)` -- a real function taking a
/// `float` scale, so a `double` argument narrows at the call, not inside.
#[inline]
fn vector_ma(veca: &[c_float; 3], scale: c_float, vecb: &[c_float; 3], out: &mut [c_float; 3]) {
    m::vector_ma(veca, scale, vecb, out);
}

/// `mathlib.h` `DotProduct`
#[inline]
fn dot(x: &[c_float; 3], y: &[c_float; 3]) -> c_float {
    m::dot_product(x, y)
}

/// Casts an entvars float field the way C's `(int)ent->v.flags` does.
///
/// COMPAT: ADR-010 -- C's float-to-int conversion truncates toward zero and is
/// undefined out of range; Rust's `as` saturates instead. Every value that
/// reaches these casts is a small flag/movetype bitfield, so the two agree.
#[inline]
fn as_int(x: c_float) -> c_int {
    x as c_int
}

// ---------------------------------------------------------------------------
// pushable cache + spatial grid (sv_phys.c:101-276)

/// `sv_phys.c:101` `SV_IsPushable`
#[inline]
unsafe fn sv_is_pushable(ent: *mut Edict) -> bool {
    // SAFETY: `ent` is a live edict of the ambient qcvm.
    unsafe {
        (*ent).v.movetype != MOVETYPE_PUSH
            && (*ent).v.movetype != MOVETYPE_NONE
            && (*ent).v.movetype != MOVETYPE_NOCLIP
    }
}

/// `sv_phys.c:106-109` `push_grid_cell_t` as the hash map's 12-byte key blob.
#[inline]
fn push_grid_key(x: c_int, y: c_int, z: c_int) -> [u8; 12] {
    let mut key = [0u8; 12];
    key[0..4].copy_from_slice(&x.to_ne_bytes());
    key[4..8].copy_from_slice(&y.to_ne_bytes());
    key[8..12].copy_from_slice(&z.to_ne_bytes());
    key
}

#[inline]
fn read_i32(b: &[u8]) -> c_int {
    c_int::from_ne_bytes([b[0], b[1], b[2], b[3]])
}

/// `sv_phys.c:111` `PushGrid_HashCell`
fn push_grid_hash_cell(key: &[u8]) -> u32 {
    let x = u32::from_ne_bytes([key[0], key[1], key[2], key[3]]);
    let y = u32::from_ne_bytes([key[4], key[5], key[6], key[7]]);
    let z = u32::from_ne_bytes([key[8], key[9], key[10], key[11]]);
    hashers::hash_combine(
        hashers::hash_int32(x),
        hashers::hash_combine(hashers::hash_int32(y), hashers::hash_int32(z)),
    )
}

/// `sv_phys.c:117` `PushGrid_Cell`
fn push_grid_cell(v: c_float) -> c_int {
    let limit = (1 << 23) as c_float;
    let mut v = v;
    // C: `if (!(v >= -limit))`, which is true for NaN as well as for v < -limit
    if v.is_nan() || v < -limit {
        v = -limit;
    } else if v > limit {
        v = limit;
    }
    // COMPAT: ADR-010 -- C rounds with `floorf`; `floor` on the widened double
    // is bit-identical for every finite float and avoids a second libm
    // declaration. The clamp above keeps the int cast inside +-2^23, where C's
    // truncation and Rust's saturating `as` agree.
    (c::libm::floor(v as f64) as c_int) >> PUSH_GRID_CELL_SHIFT
}

/// `sv_phys.c:128` `PushGrid_CellRange`
fn push_grid_cell_range(
    absmin: &[c_float; 3],
    absmax: &[c_float; 3],
    inflate: c_float,
    lo: &mut [c_int; 3],
    hi: &mut [c_int; 3],
) {
    for i in 0..3 {
        lo[i] = push_grid_cell(absmin[i] - inflate);
        hi[i] = push_grid_cell(absmax[i] + inflate);
    }
}

/// `sv_phys.c:137` `PushGrid_Clear`
unsafe fn push_grid_clear() {
    // SAFETY: single-threaded server state (sv_phys.c:69-70).
    unsafe {
        let map = ptr::addr_of_mut!(PUSH_GRID_MAP);
        if (*map).is_none() {
            // C: `HashMap_Create (push_grid_cell_t, int32_t, &PushGrid_HashCell, NULL)`
            *map = Some(QHashMap::new(12, 4, Box::new(push_grid_hash_cell), None));
        }
        if let Some(m) = (*map).as_mut() {
            m.clear();
        }
        PUSH_GRID_NUM_ENTRIES = 0;
        PUSH_GRID_NUM_LARGE = 0;
        PUSH_GRID_VALID = true;
        PUSH_GRID_ACTIVE = false;
    }
}

/// `sv_phys.c:148` `PushGrid_Insert`
unsafe fn push_grid_insert(ent: *mut Edict) {
    // SAFETY: single-threaded server state; `ent` is a live edict.
    unsafe {
        let mut lo = [0 as c_int; 3];
        let mut hi = [0 as c_int; 3];
        push_grid_cell_range(&(*ent).v.absmin, &(*ent).v.absmax, 0.0, &mut lo, &mut hi);

        // per-axis span check before the multiply so huge boxes can't overflow
        if hi[0] - lo[0] >= 4 || hi[1] - lo[1] >= 4 || hi[2] - lo[2] >= 4 {
            if PUSH_GRID_NUM_LARGE == PUSH_GRID_MAX_LARGE as c_int {
                PUSH_GRID_VALID = false;
            } else {
                let large = ptr::addr_of_mut!(PUSH_GRID_LARGE);
                (*large)[PUSH_GRID_NUM_LARGE as usize] = ent;
                PUSH_GRID_NUM_LARGE += 1;
            }
            return;
        }

        let cells = (hi[0] - lo[0] + 1) * (hi[1] - lo[1] + 1) * (hi[2] - lo[2] + 1);

        if PUSH_GRID_NUM_ENTRIES + cells > PUSH_GRID_ENTRIES_CAPACITY {
            PUSH_GRID_ENTRIES_CAPACITY = q_max_i(PUSH_GRID_ENTRIES_CAPACITY * 2, 4096);
            PUSH_GRID_ENTRIES = c::Mem_Realloc(
                PUSH_GRID_ENTRIES.cast::<c_void>(),
                PUSH_GRID_ENTRIES_CAPACITY as usize * core::mem::size_of::<PushGridEntry>(),
            )
            .cast::<PushGridEntry>();
        }

        let map = ptr::addr_of_mut!(PUSH_GRID_MAP);
        // unreachable: every insert path runs after PushGrid_Clear built the map
        let Some(map) = (*map).as_mut() else { return };

        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    let key = push_grid_key(x, y, z);
                    let index = PUSH_GRID_NUM_ENTRIES;
                    let head = map.lookup(&key);

                    let entry = PUSH_GRID_ENTRIES.add(index as usize);
                    (*entry).ent = ent;
                    (*entry).next = match head {
                        Some(i) => read_i32(map.get_value(i)),
                        None => -1,
                    };
                    match head {
                        Some(i) => map.get_value_mut(i).copy_from_slice(&index.to_ne_bytes()),
                        None => {
                            map.insert(&key, &index.to_ne_bytes());
                        }
                    }
                    PUSH_GRID_NUM_ENTRIES += 1;
                }
            }
        }
    }
}

/// `sv_phys.c:200` `SV_PushGridEntityLinked` -- called from `SV_LinkEdict`.
///
/// Reaches only the hash map and `Mem_Realloc`, so it cannot raise and keeps
/// its plain C name (ADR-009).
///
/// # Safety
/// `ent` must be a live edict.
#[no_mangle]
pub unsafe extern "C" fn SV_PushGridEntityLinked(ent: *mut Edict) {
    // SAFETY: ADR-008 ambient qcvm; single-threaded server state.
    unsafe {
        if !PUSH_GRID_ACTIVE || c::qcvm.cast::<QcVm>() != PUSH_GRID_QCVM || (*ent).free {
            return;
        }
        if !sv_is_pushable(ent) {
            return;
        }
        push_grid_insert(ent);
    }
}

/// `sv_phys.c:218` `PushGrid_GatherCandidates`. Returns the count, or -1 when
/// the grid is unusable this tick.
unsafe fn push_grid_gather_candidates(
    mins: &[c_float; 3],
    maxs: &[c_float; 3],
    out: *mut *mut Edict,
) -> c_int {
    // SAFETY: `out` is the MAX_EDICTS-sized `push_candidates` static.
    unsafe {
        let mut num: c_int = 0;

        if !PUSH_GRID_VALID {
            return -1;
        }

        let mut lo = [0 as c_int; 3];
        let mut hi = [0 as c_int; 3];
        push_grid_cell_range(mins, maxs, 2.0, &mut lo, &mut hi);

        let map = ptr::addr_of_mut!(PUSH_GRID_MAP);
        // unreachable: the grid is only active after PushGrid_Clear
        let Some(map) = (*map).as_mut() else {
            return -1;
        };

        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    let key = push_grid_key(x, y, z);
                    let head = map.lookup(&key);
                    let mut i = match head {
                        Some(h) => read_i32(map.get_value(h)),
                        None => -1,
                    };
                    while i >= 0 {
                        if num == MAX_EDICTS as c_int {
                            return -1; // pathological duplication
                        }
                        let entry = PUSH_GRID_ENTRIES.add(i as usize);
                        *out.add(num as usize) = (*entry).ent;
                        num += 1;
                        i = (*entry).next;
                    }
                }
            }
        }

        if num + PUSH_GRID_NUM_LARGE + (NUM_PUSHABLE_ENT_CACHE - PUSH_GRID_TAIL_START)
            > MAX_EDICTS as c_int
        {
            return -1;
        }

        let large = ptr::addr_of_mut!(PUSH_GRID_LARGE);
        for i in 0..PUSH_GRID_NUM_LARGE {
            *out.add(num as usize) = (*large)[i as usize];
            num += 1;
        }

        // entities allocated after the grid was built
        let cache = ptr::addr_of_mut!(PUSHABLE_ENT_CACHE);
        for i in PUSH_GRID_TAIL_START..NUM_PUSHABLE_ENT_CACHE {
            *out.add(num as usize) = (*cache)[i as usize];
            num += 1;
        }

        // restore vanilla processing order (blocked pushers roll back
        // everything moved so far, so order is observable) and drop duplicates
        for i in 1..num {
            let key = *out.add(i as usize);
            let mut j = i - 1;
            while j >= 0 && *out.add(j as usize) > key {
                *out.add((j + 1) as usize) = *out.add(j as usize);
                j -= 1;
            }
            *out.add((j + 1) as usize) = key;
        }
        let mut unique: c_int = 0;
        for i in 0..num {
            if unique == 0 || *out.add((unique - 1) as usize) != *out.add(i as usize) {
                *out.add(unique as usize) = *out.add(i as usize);
                unique += 1;
            }
        }

        unique
    }
}

/// `host_frametime` without forming a reference to the C static.
#[inline]
unsafe fn host_frametime() -> f64 {
    // SAFETY: single-threaded host state.
    unsafe { ptr::addr_of!(c::host_frametime).read() }
}

/// The `sv_analyticphysics` latch `SV_Physics` writes once per tick.
#[inline]
unsafe fn analytic_frame() -> bool {
    // SAFETY: single-threaded server state; `Quake/sv_phys_glue.c` owns it.
    unsafe { ptr::addr_of!(g::sv_analyticphysics_frame).read() }
}

// ---------------------------------------------------------------------------
// SV_CheckAllEnts / SV_CheckVelocity (sv_phys.c:278-330)

/// `sv_phys.c:283` `SV_CheckAllEnts`.
///
/// # Safety
/// The ambient qcvm must be loaded (ADR-008).
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_check_all_ents() -> Raise {
    // SAFETY: ADR-008 ambient qcvm; every edict walked is a live arena slot.
    unsafe {
        let vm = c::qcvm.cast::<QcVm>();

        // see if any solid entities are inside the final position
        let mut check = next_edict(vm, (*vm).edicts);
        let mut e: c_int = 1;
        while e < (*vm).num_edicts {
            if !(*check).free
                && (*check).v.movetype != MOVETYPE_PUSH
                && (*check).v.movetype != MOVETYPE_NONE
                && (*check).v.movetype != MOVETYPE_NOCLIP
            {
                let mut hit: *mut Edict = ptr::null_mut();
                let raised = crate::world::quake_rs_sv_test_entity_position(check, &mut hit);
                if raised != 0 {
                    return raised;
                }
                if !hit.is_null() {
                    g::SvPhys_Glue_PrintInvalidPosition();
                }
            }
            e += 1;
            check = next_edict(vm, check);
        }

        0
    }
}

/// `sv_phys.c:307` `SV_CheckVelocity`.
///
/// # Safety
/// `ent` must be a live edict of the ambient qcvm.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_check_velocity(ent: *mut Edict) -> Raise {
    // SAFETY: `ent` is a live edict; the NaN warnings run under a guard.
    unsafe { sv_check_velocity(ent) }
}

/// The status core `quake_rs_sv_check_velocity` exports; the rest of this
/// module calls it directly rather than the re-raising C wrapper (ADR-009).
unsafe fn sv_check_velocity(ent: *mut Edict) -> Raise {
    // SAFETY: `ent` is a live edict of the ambient qcvm.
    unsafe {
        //
        // bound velocity
        //
        for i in 0..3 {
            if is_nan((*ent).v.velocity[i]) {
                let raised = g::SvPhys_Glue_WarnNanVelocity(ent.cast::<c_void>());
                if raised != 0 {
                    return raised;
                }
                (*ent).v.velocity[i] = 0.0;
            }
            if is_nan((*ent).v.origin[i]) {
                let raised = g::SvPhys_Glue_WarnNanOrigin(ent.cast::<c_void>());
                if raised != 0 {
                    return raised;
                }
                (*ent).v.origin[i] = 0.0;
            }
            // COMPAT: ADR-010 -- C re-reads the cvar on every axis; the read
            // point is kept where C has it.
            let maxvel = cvar_value(ptr::addr_of!(g::sv_maxvelocity));
            if (*ent).v.velocity[i] > maxvel {
                (*ent).v.velocity[i] = maxvel;
            } else if (*ent).v.velocity[i] < -maxvel {
                (*ent).v.velocity[i] = -maxvel;
            }
        }

        0
    }
}

// ---------------------------------------------------------------------------
// SV_RunThink / SV_Impact / ClipVelocity (sv_phys.c:334-476)

/// `sv_phys.c:346` `SV_RunThink`. `alive` receives C's return value: false
/// when the entity removed itself.
unsafe fn sv_run_think(ent: *mut Edict, alive: &mut bool) -> Raise {
    // SAFETY: `ent` is a live edict; the think dispatch runs under a guard and
    // the ambient qcvm is re-resolved after it (ADR-006, ADR-008).
    unsafe {
        *alive = true;

        let vm = c::qcvm.cast::<QcVm>();
        let mut thinktime = (*ent).v.nextthink;
        // COMPAT: ADR-010 -- `qcvm->time` is a double, so the comparison
        // promotes `thinktime`.
        if thinktime <= 0.0 || thinktime as f64 > (*vm).time + host_frametime() {
            return 0;
        }

        let mut think_start: f64 = 0.0;
        if sv_speeds_on() && qcvm_is_server() {
            think_start = c::Sys_DoubleTime();
        }

        if (thinktime as f64) < (*vm).time {
            // don't let things stay in the past; it is possible to start that
            // way by a trigger with a local time.
            thinktime = (*vm).time as c_float;
        }

        (*ent).oldthinktime = thinktime;
        (*ent).oldframe = (*ent).v.frame; // johnfitz

        (*ent).v.nextthink = 0.0;
        let raised = g::SvPhys_Glue_CallThink(ent.cast::<c_void>(), thinktime);
        if raised != 0 {
            return raised;
        }

        // ADR-006: re-resolve the vm after the dispatch.
        let vm = c::qcvm.cast::<QcVm>();

        (*ent).lastthink = 0.0;
        if !(*ent).free
            && (*ent).v.groundentity != 0
            && (*ent).v.nextthink > 0.0
            && (*ent).v.nextthink - thinktime < 0.105f32
            && (*ent).v.groundentity <= ((*vm).num_edicts - 1) * (*vm).edict_size
        {
            let pusher = prog_to_edict(vm, (*ent).v.groundentity);
            if !(*pusher).free {
                let pusher_remaining = (*pusher).v.nextthink - (*pusher).v.ltime;
                if pusher_remaining > 0.0 {
                    // COMPAT: ADR-010 -- `(int)(...)` truncates a double, the
                    // product is a double, and `q_min` resolves to the double
                    // overload before the result narrows to `float`.
                    let steps =
                        (((*ent).v.nextthink as f64 - (*vm).time) / host_frametime()) as c_int;
                    let stepped = steps as f64 * host_frametime();
                    let remaining = pusher_remaining as f64;
                    let time = (if stepped < remaining {
                        stepped
                    } else {
                        remaining
                    }) as c_float;
                    for i in 0..3 {
                        (*ent).predthinkpos[i] =
                            (*ent).v.origin[i] + (*pusher).v.velocity[i] * time;
                        if (*pusher).v.velocity[i] != 0.0f32 {
                            (*ent).lastthink = thinktime;
                        }
                    }
                }
            }
        }

        if think_start != 0.0 {
            *ptr::addr_of_mut!(g::sv_speeds_think_ms) +=
                (c::Sys_DoubleTime() - think_start) * 1000.0;
            *ptr::addr_of_mut!(g::sv_speeds_thinks) += 1;
        }

        *alive = !(*ent).free;
        0
    }
}

/// `sv_phys.c:406` `SV_Impact` -- run both entities' touch functions.
unsafe fn sv_impact(e1: *mut Edict, e2: *mut Edict) -> Raise {
    // SAFETY: both edicts are live on entry (asserted); the dispatches run
    // under guards and the globals pointer is re-resolved after them.
    unsafe {
        if (*e1).free || (*e2).free {
            let raised =
                w::World_Glue_AssertFailed(ASSERT_E1_E2_FREE.as_ptr(), ASSERT_FILE.as_ptr(), 413);
            if raised != 0 {
                return raised;
            }
        }

        let vm = c::qcvm.cast::<QcVm>();
        let gv = globals(vm);
        let old_self = (*gv).self_;
        let old_other = (*gv).other;

        // COMPAT: ADR-010 -- `qcvm->time` is a double and `pr_global_struct
        // ->time` a float, so the stamp narrows here, once, before either
        // dispatch. QC may overwrite it in between.
        (*gv).time = (*vm).time as c_float;

        if (*e1).v.touch != 0 && (*e1).v.solid != SOLID_NOT {
            let raised = g::SvPhys_Glue_ImpactTouch(e1.cast::<c_void>(), e2.cast::<c_void>());
            // COMPAT: ADR-009 -- C's longjmp leaves `self`/`other` holding the
            // dispatch's values, so the restore below is skipped too.
            if raised != 0 {
                return raised;
            }
        }

        // PR_ExecuteProgram can have side effects on e1, e2 (like freeing) so
        // only execute the next one if e1,e2 are still valid
        if !(*e1).free && !(*e2).free && (*e2).v.touch != 0 && (*e2).v.solid != SOLID_NOT {
            let raised = g::SvPhys_Glue_ImpactTouch(e2.cast::<c_void>(), e1.cast::<c_void>());
            if raised != 0 {
                return raised;
            }
        }

        // ADR-006: re-resolve after the dispatches.
        let gv = globals(c::qcvm.cast::<QcVm>());
        (*gv).self_ = old_self;
        (*gv).other = old_other;

        0
    }
}

/// `sv_phys.c:455` `ClipVelocity` -- slide off the impacting object. Returns
/// the blocked flags (1 = floor, 2 = step / wall).
///
/// Raw pointers because `SV_Physics_Toss` passes `ent->v.velocity` as both
/// `in` and `out`.
unsafe fn clip_velocity(
    in_: *const c_float,
    normal: *const c_float,
    out: *mut c_float,
    overbounce: c_float,
) -> c_int {
    // SAFETY: all three are `vec3_t`s; `in_` and `out` may alias, exactly as
    // in C.
    unsafe {
        let mut blocked: c_int = 0;
        if *normal.add(2) > 0.0 {
            blocked |= 1; // floor
        }
        if *normal.add(2) == 0.0 {
            blocked |= 2; // step
        }

        let backoff =
            (*in_ * *normal + *in_.add(1) * *normal.add(1) + *in_.add(2) * *normal.add(2))
                * overbounce;

        for i in 0..3 {
            let change = *normal.add(i) * backoff;
            *out.add(i) = *in_.add(i) - change;
            if *out.add(i) > -STOP_EPSILON && *out.add(i) < STOP_EPSILON {
                *out.add(i) = 0.0;
            }
        }

        blocked
    }
}

/// `sv_phys.c:485` `SV_ClassifyWalkSupportContact`. Non-raising.
unsafe fn sv_classify_walk_support_contact(trace: *mut Trace) -> PusherContact {
    // SAFETY: `trace` is a live `trace_t`; the support statics are
    // single-threaded server state.
    unsafe {
        let pusher = ptr::addr_of!(SV_WALK_SUPPORT_PUSHER).read();
        if pusher.is_null() || (*trace).ent != pusher {
            return PusherContact::None;
        }

        let support_normal = ptr::addr_of!(SV_WALK_SUPPORT_NORMAL).read();
        let support_dot = dot(&(*trace).plane.normal, &support_normal);
        if support_dot > MIN_WALK_NORMAL {
            return PusherContact::SupportFloor;
        }
        if support_dot <= 0.0 {
            return PusherContact::None;
        }

        // While walking on a pusher, a non-floor contact with that same pusher
        // is lateral support geometry. Clip against its tangent component so it
        // cannot inject velocity away from the support plane.
        let mut tangent_normal = [0.0 as c_float; 3];
        vector_ma(
            &(*trace).plane.normal,
            -support_dot,
            &support_normal,
            &mut tangent_normal,
        );
        // COMPAT: ADR-010 -- `DIST_EPSILON` is a double, so the length promotes.
        if m::vector_normalize(&mut tangent_normal) as f64 <= DIST_EPSILON {
            return PusherContact::None;
        }

        (*trace).plane.normal = tangent_normal;
        PusherContact::SupportSide
    }
}

/// `sv_phys.c:522` `SV_FlyMove` -- the multi-plane slide solver. `clip`
/// receives C's return value.
unsafe fn sv_fly_move(
    ent: *mut Edict,
    time: c_float,
    steptrace: *mut Trace,
    clip: &mut c_int,
) -> Raise {
    // SAFETY: `ent` is a live edict; `steptrace` is either null or a writable
    // `trace_t`.
    unsafe {
        let numbumps = 4;

        let mut blocked: c_int = 0;
        let mut original_velocity = (*ent).v.velocity;
        let primal_velocity = (*ent).v.velocity;
        let mut new_velocity = (*ent).v.velocity;
        let mut numplanes: usize = 0;
        let mut planes = [[0.0 as c_float; 3]; MAX_CLIP_PLANES];
        let mut end = [0.0 as c_float; 3];

        let mut time_left = time;

        for _bumpcount in 0..numbumps {
            if (*ent).v.velocity[0] == 0.0
                && (*ent).v.velocity[1] == 0.0
                && (*ent).v.velocity[2] == 0.0
            {
                break;
            }

            end[0] = (*ent).v.origin[0] + time_left * (*ent).v.velocity[0];
            end[1] = (*ent).v.origin[1] + time_left * (*ent).v.velocity[1];
            end[2] = (*ent).v.origin[2] + time_left * (*ent).v.velocity[2];

            let mut trace = trace_zeroed();
            let raised = crate::world::quake_rs_sv_move(
                &mut trace,
                ptr::addr_of_mut!((*ent).v.origin).cast::<c_float>(),
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
                // entity is trapped in another solid
                (*ent).v.velocity = [0.0; 3];
                *clip = 3;
                return 0;
            }

            if trace.fraction > 0.0 {
                // actually covered some distance
                (*ent).v.origin = trace.endpos;
                original_velocity = (*ent).v.velocity;
                numplanes = 0;
            }

            if trace.fraction == 1.0 {
                break; // moved the entire distance
            }

            if trace.ent.is_null() {
                c::Sys_Error(SYS_ERR_FLYMOVE_NO_TRACE_ENT.as_ptr());
            }

            let pusher_contact = sv_classify_walk_support_contact(&mut trace);

            if pusher_contact != PusherContact::SupportSide
                && trace.plane.normal[2] > MIN_WALK_NORMAL
            {
                blocked |= 1; // floor
                if (*trace.ent).v.solid == SOLID_BSP {
                    (*ent).v.flags = (as_int((*ent).v.flags) | FL_ONGROUND) as c_float;
                    (*ent).v.groundentity = edict_to_prog(c::qcvm.cast::<QcVm>(), trace.ent);
                }
            }
            if pusher_contact == PusherContact::SupportSide || trace.plane.normal[2] == 0.0 {
                blocked |= 2; // step
                if !steptrace.is_null() {
                    *steptrace = trace; // save for player extrafriction
                }
            }

            //
            // run the impact function
            //
            if (*ent).free {
                let raised =
                    w::World_Glue_AssertFailed(ASSERT_ENT_FREE.as_ptr(), ASSERT_FILE.as_ptr(), 598);
                if raised != 0 {
                    return raised;
                }
            }

            let raised = sv_impact(ent, trace.ent);
            if raised != 0 {
                return raised;
            }
            if (*ent).free {
                break; // removed by the impact function
            }

            time_left -= time_left * trace.fraction;

            // cliped to another plane
            if numplanes >= MAX_CLIP_PLANES {
                // this shouldn't really happen
                (*ent).v.velocity = [0.0; 3];
                *clip = 3;
                return 0;
            }

            planes[numplanes] = trace.plane.normal;
            numplanes += 1;

            //
            // modify original_velocity so it parallels all of the clip planes
            //
            let mut i = 0usize;
            while i < numplanes {
                clip_velocity(
                    original_velocity.as_ptr(),
                    planes[i].as_ptr(),
                    new_velocity.as_mut_ptr(),
                    1.0,
                );
                let mut j = 0usize;
                while j < numplanes {
                    if j != i && dot(&new_velocity, &planes[j]) < 0.0 {
                        break; // not ok
                    }
                    j += 1;
                }
                if j == numplanes {
                    break;
                }
                i += 1;
            }

            if i != numplanes {
                // go along this plane
                (*ent).v.velocity = new_velocity;
            } else {
                // go along the crease
                if numplanes != 2 {
                    (*ent).v.velocity = [0.0; 3];
                    *clip = 7;
                    return 0;
                }
                let mut dir = [0.0 as c_float; 3];
                m::cross_product(&planes[0], &planes[1], &mut dir);
                let d = dot(&dir, &(*ent).v.velocity);
                m::vector_scale(&dir, d, &mut (*ent).v.velocity);
            }

            //
            // if original velocity is against the original velocity, stop dead
            // to avoid tiny occilations in sloping corners
            //
            if dot(&(*ent).v.velocity, &primal_velocity) <= 0.0 {
                (*ent).v.velocity = [0.0; 3];
                *clip = blocked;
                return 0;
            }
        }

        *clip = blocked;
        0
    }
}

// ---------------------------------------------------------------------------
// gravity (sv_phys.c:663-690)

/// `sv_phys.c:663` `SV_EntGravity`. The field offset is looked up on every
/// call, exactly as in C.
unsafe fn sv_ent_gravity(ent: *mut Edict) -> c_float {
    // SAFETY: `GetEdictFieldValue` returns null for an absent field; `eval_t`'s
    // first member is the float this reads.
    unsafe {
        let val = g::GetEdictFieldValue(
            ent.cast::<c_void>(),
            g::ED_FindFieldOffset(FIELD_GRAVITY.as_ptr()),
        );
        if !val.is_null() && *val != 0.0 {
            *val
        } else {
            1.0f32
        }
    }
}

/// `sv_phys.c:679` `SV_AddGravity`.
unsafe fn sv_add_gravity(ent: *mut Edict) {
    // SAFETY: `ent` is a live edict; nothing here dispatches progs code.
    unsafe {
        // COMPAT: ADR-010 -- `dt` is a double, so the product promotes and the
        // result narrows only on the store back into the float field.
        let dt: f64 = if analytic_frame() {
            (host_frametime() + 1.0 / MAX_PHYSICS_FREQ) * 0.5
        } else {
            host_frametime()
        };
        let grav = sv_ent_gravity(ent) * sv_gravity_value();
        (*ent).v.velocity[2] = ((*ent).v.velocity[2] as f64 - grav as f64 * dt) as c_float;
    }
}

/// `sv_phys.c:685` `SV_FinishGravity`.
unsafe fn sv_finish_gravity(ent: *mut Edict) {
    // SAFETY: `ent` is a live edict; nothing here dispatches progs code.
    unsafe {
        if !analytic_frame() {
            return;
        }
        // entities that landed during the move keep their clipped velocity,
        // like at 72fps
        if as_int((*ent).v.flags) & FL_ONGROUND != 0 {
            return;
        }
        // COMPAT: ADR-010 -- same promotion chain as `SV_AddGravity`.
        let grav = sv_ent_gravity(ent) * sv_gravity_value();
        (*ent).v.velocity[2] = ((*ent).v.velocity[2] as f64
            - grav as f64 * (host_frametime() - 1.0 / MAX_PHYSICS_FREQ) * 0.5)
            as c_float;
    }
}

// ---------------------------------------------------------------------------
// pusher support bookkeeping (sv_phys.c:734-1130)

/// `&sv_pusher_support[entnum]` without forming a reference to the static.
#[inline]
unsafe fn pusher_support_slot(entnum: c_int) -> *mut PusherSupportRecord {
    // SAFETY: every caller has range-checked `entnum` against `MAX_EDICTS`.
    unsafe {
        ptr::addr_of_mut!(SV_PUSHER_SUPPORT)
            .cast::<PusherSupportRecord>()
            .add(entnum as usize)
    }
}

/// C's `memset (sv_pusher_support, 0, sizeof (sv_pusher_support))`.
#[inline]
unsafe fn clear_pusher_support() {
    // SAFETY: the all-zero bit pattern is a valid `PusherSupportRecord`
    // (`MoveFrameState::None` is discriminant 0).
    unsafe {
        ptr::write_bytes(
            ptr::addr_of_mut!(SV_PUSHER_SUPPORT).cast::<PusherSupportRecord>(),
            0,
            MAX_EDICTS,
        );
    }
}

/// `sv_phys.c:738` `SV_BeginPusherSupportFrame`
unsafe fn sv_begin_pusher_support_frame() {
    // SAFETY: single-threaded server state.
    unsafe {
        if !qcvm_is_server() {
            return;
        }

        let vm = c::qcvm.cast::<QcVm>();
        if SV_PUSHER_SUPPORT_EDICTS != (*vm).edicts {
            clear_pusher_support();
            SV_PUSHER_SUPPORT_EDICTS = (*vm).edicts;
            SV_PUSHER_SUPPORT_FRAME = 1;
            return;
        }

        SV_PUSHER_SUPPORT_FRAME = SV_PUSHER_SUPPORT_FRAME.wrapping_add(1);
        if SV_PUSHER_SUPPORT_FRAME == 0 {
            clear_pusher_support();
            SV_PUSHER_SUPPORT_FRAME = 1;
        }
    }
}

/// `sv_phys.c:758` `SV_TracePusherFloorAtOrigin`. `hit` receives C's return
/// value; `trace` receives the sweep.
unsafe fn sv_trace_pusher_floor_at_origin(
    ent: *mut Edict,
    pusher: *mut Edict,
    pusher_origin: &[c_float; 3],
    probe_distance: c_float,
    trace: &mut Trace,
    hit: &mut bool,
) -> Raise {
    // SAFETY: both edicts are live; the pusher origin swap is undone on the
    // non-raising path exactly as in C.
    unsafe {
        *hit = false;

        let mut old_absmin = [0.0 as c_float; 3];
        let mut old_absmax = [0.0 as c_float; 3];
        for i in 0..3 {
            let delta = pusher_origin[i] - (*pusher).v.origin[i];
            old_absmin[i] = (*pusher).v.absmin[i] + delta;
            old_absmax[i] = (*pusher).v.absmax[i] + delta;
        }

        if (*ent).v.absmin[0] >= old_absmax[0]
            || (*ent).v.absmin[1] >= old_absmax[1]
            || (*ent).v.absmax[0] <= old_absmin[0]
            || (*ent).v.absmax[1] <= old_absmin[1]
        {
            return 0;
        }
        // COMPAT: ADR-010 -- the first compare is all-float; the second one
        // promotes because `PUSH_CONTACT_EPSILON` is a double.
        if (*ent).v.absmin[2] > old_absmax[2] + probe_distance
            || ((*ent).v.absmax[2] as f64) < old_absmin[2] as f64 - PUSH_CONTACT_EPSILON
        {
            return 0;
        }

        let old_origin = (*pusher).v.origin;
        (*pusher).v.origin = *pusher_origin;

        let mut start = (*ent).v.origin;
        start[2] = (start[2] as f64 + PUSH_CONTACT_EPSILON) as c_float;
        let mut end = (*ent).v.origin;
        end[2] -= probe_distance;

        let raised = crate::world::quake_rs_sv_clip_move_to_entity(
            trace,
            pusher,
            start.as_mut_ptr(),
            ptr::addr_of_mut!((*ent).v.mins).cast::<c_float>(),
            ptr::addr_of_mut!((*ent).v.maxs).cast::<c_float>(),
            end.as_mut_ptr(),
            CONTENTMASK_ANYSOLID,
        );
        // COMPAT: ADR-009 -- C's longjmp leaves the pusher parked at the probe
        // origin, so the restore below is skipped on the raising path too.
        if raised != 0 {
            return raised;
        }

        (*pusher).v.origin = old_origin;

        *hit = !trace.startsolid && trace.fraction < 1.0 && trace.plane.normal[2] > MIN_WALK_NORMAL;
        0
    }
}

/// `sv_phys.c:790` `SV_TouchingPusherAtOrigin`
unsafe fn sv_touching_pusher_at_origin(
    ent: *mut Edict,
    pusher: *mut Edict,
    pusher_origin: &[c_float; 3],
    hit: &mut bool,
) -> Raise {
    // SAFETY: see `sv_trace_pusher_floor_at_origin`.
    unsafe {
        let mut trace = trace_zeroed();
        sv_trace_pusher_floor_at_origin(
            ent,
            pusher,
            pusher_origin,
            PUSH_CONTACT_EPSILON as c_float,
            &mut trace,
            hit,
        )
    }
}

/// `sv_phys.c:797` `SV_PusherMoveTimeThisFrame`
unsafe fn sv_pusher_move_time_this_frame(pusher: *mut Edict) -> c_float {
    // SAFETY: `pusher` is a live edict.
    unsafe {
        let thinktime = (*pusher).v.nextthink;
        // COMPAT: ADR-010 -- `host_frametime` is a double, so the sum and the
        // comparison are performed in double.
        if (thinktime as f64) < (*pusher).v.ltime as f64 + host_frametime() {
            let mut movetime = thinktime - (*pusher).v.ltime;
            if movetime < 0.0 {
                movetime = 0.0;
            }
            movetime
        } else {
            host_frametime() as c_float
        }
    }
}

/// `sv_phys.c:813` `SV_IsClientMoveFramePusher`
unsafe fn sv_is_client_move_frame_pusher(pusher: *mut Edict) -> bool {
    // SAFETY: `pusher` is null or a live edict.
    unsafe {
        if pusher.is_null() || (*pusher).free {
            return false;
        }
        if (*pusher).v.movetype != MOVETYPE_PUSH || (*pusher).v.solid != SOLID_BSP {
            return false;
        }
        true
    }
}

/// `sv_phys.c:822` `SV_PusherWillMoveThisFrame`
unsafe fn sv_pusher_will_move_this_frame(pusher: *mut Edict) -> bool {
    // SAFETY: `pusher` is a live edict.
    unsafe {
        if (*pusher).v.velocity[0] == 0.0
            && (*pusher).v.velocity[1] == 0.0
            && (*pusher).v.velocity[2] == 0.0
        {
            return false;
        }
        sv_pusher_move_time_this_frame(pusher) > 0.0
    }
}

/// `sv_phys.c:829` `SV_GetGroundPusher`
unsafe fn sv_get_ground_pusher(ent: *mut Edict) -> *mut Edict {
    // SAFETY: `ent` is a live edict; ADR-008 ambient qcvm.
    unsafe {
        if sv_elevators_value() < 3.0f32 || as_int((*ent).v.flags) & FL_ONGROUND == 0 {
            return ptr::null_mut();
        }
        let vm = c::qcvm.cast::<QcVm>();
        if (*ent).v.groundentity <= 0
            || (*ent).v.groundentity > ((*vm).num_edicts - 1) * (*vm).edict_size
        {
            return ptr::null_mut();
        }

        let ground = prog_to_edict(vm, (*ent).v.groundentity);
        if !sv_is_client_move_frame_pusher(ground) {
            return ptr::null_mut();
        }

        ground
    }
}

/// `sv_phys.c:844` `SV_GetPusherSupportRecord`. `record` is left null when C
/// returns false.
unsafe fn sv_get_pusher_support_record(
    ent: *mut Edict,
    record: &mut *mut PusherSupportRecord,
) -> Raise {
    // SAFETY: `NUM_FOR_EDICT` runs under a guard (it can Host_Error).
    unsafe {
        *record = ptr::null_mut();

        if !qcvm_is_server() {
            return 0;
        }

        let mut entnum: c_int = 0;
        let raised = w::World_Glue_NumForEdict(ent.cast::<c_void>(), &mut entnum);
        if raised != 0 {
            return raised;
        }
        if entnum <= 0 || entnum >= MAX_EDICTS as c_int {
            return 0;
        }

        *record = pusher_support_slot(entnum);
        0
    }
}

/// `sv_phys.c:858` `SV_GetAppliedPusherSupportMove`
unsafe fn sv_get_applied_pusher_support_move(
    ent: *mut Edict,
    move_out: &mut [c_float; 3],
) -> Raise {
    // SAFETY: see `sv_get_pusher_support_record`.
    unsafe {
        *move_out = [0.0; 3];

        let mut support: *mut PusherSupportRecord = ptr::null_mut();
        let raised = sv_get_pusher_support_record(ent, &mut support);
        if raised != 0 {
            return raised;
        }
        if support.is_null() {
            return 0;
        }
        if (*support).frame == SV_PUSHER_SUPPORT_FRAME {
            *move_out = (*support).pusher_move;
        }
        0
    }
}

/// `sv_phys.c:869` `SV_BackupPusherSupport`
unsafe fn sv_backup_pusher_support(ent: *mut Edict, backup: &mut PusherSupportRecord) -> Raise {
    // SAFETY: see `sv_get_pusher_support_record`.
    unsafe {
        *backup = PusherSupportRecord::ZERO;

        let mut support: *mut PusherSupportRecord = ptr::null_mut();
        let raised = sv_get_pusher_support_record(ent, &mut support);
        if raised != 0 {
            return raised;
        }
        if !support.is_null() {
            *backup = *support;
        }
        0
    }
}

/// `sv_phys.c:878` `SV_RestorePusherSupport`
unsafe fn sv_restore_pusher_support(ent: *mut Edict, backup: &PusherSupportRecord) -> Raise {
    // SAFETY: see `sv_get_pusher_support_record`.
    unsafe {
        let mut support: *mut PusherSupportRecord = ptr::null_mut();
        let raised = sv_get_pusher_support_record(ent, &mut support);
        if raised != 0 {
            return raised;
        }
        if !support.is_null() {
            *support = *backup;
        }
        0
    }
}

/// `sv_phys.c:887` `SV_WritePusherSupportRecord`. `wrote` receives C's return
/// value.
unsafe fn sv_write_pusher_support_record(
    ent: *mut Edict,
    pusher: *mut Edict,
    state: MoveFrameState,
    pusher_velocity: &[c_float; 3],
    pusher_move: &[c_float; 3],
    wrote: &mut bool,
) -> Raise {
    // SAFETY: `NUM_FOR_EDICT` runs under a guard.
    unsafe {
        *wrote = false;

        let mut pushernum: c_int = 0;
        let raised = w::World_Glue_NumForEdict(pusher.cast::<c_void>(), &mut pushernum);
        if raised != 0 {
            return raised;
        }
        if pushernum <= 0 || pushernum >= MAX_EDICTS as c_int {
            return 0;
        }

        let mut support: *mut PusherSupportRecord = ptr::null_mut();
        let raised = sv_get_pusher_support_record(ent, &mut support);
        if raised != 0 {
            return raised;
        }
        if support.is_null() {
            return 0;
        }

        (*support).frame = SV_PUSHER_SUPPORT_FRAME;
        (*support).pusher_entnum = pushernum;
        (*support).state = state;
        (*support).pusher_velocity = *pusher_velocity;
        (*support).pusher_move = *pusher_move;
        *wrote = true;
        0
    }
}

/// `sv_phys.c:906` `SV_RecordPusherSupport`
unsafe fn sv_record_pusher_support(
    ent: *mut Edict,
    pusher: *mut Edict,
    pusher_move: &[c_float; 3],
) -> Raise {
    // SAFETY: both edicts are live; every raising call is checked.
    unsafe {
        if !qcvm_is_server() || sv_elevators_value() < 3.0f32 {
            return 0;
        }

        let mut trace = trace_zeroed();
        let mut hit = false;
        let pusher_origin = (*pusher).v.origin;
        let raised = sv_trace_pusher_floor_at_origin(
            ent,
            pusher,
            &pusher_origin,
            PUSH_CONTACT_EPSILON as c_float,
            &mut trace,
            &mut hit,
        );
        if raised != 0 {
            return raised;
        }
        if !hit {
            return 0;
        }

        let mut wrote = false;
        let pusher_velocity = (*pusher).v.velocity;
        let raised = sv_write_pusher_support_record(
            ent,
            pusher,
            MoveFrameState::Ground,
            &pusher_velocity,
            pusher_move,
            &mut wrote,
        );
        if raised != 0 {
            return raised;
        }
        if !wrote {
            return 0;
        }

        if (*ent).v.movetype == MOVETYPE_WALK {
            (*ent).v.flags = (as_int((*ent).v.flags) | FL_ONGROUND) as c_float;
            (*ent).v.groundentity = edict_to_prog(c::qcvm.cast::<QcVm>(), pusher);
        }

        0
    }
}

/// `sv_phys.c:923` `SV_ClearClientMoveFrame`
fn sv_clear_client_move_frame(frame: &mut ClientMoveFrame) {
    frame.pusher = ptr::null_mut();
    frame.state = MoveFrameState::None;
    frame.pusher_velocity = [0.0; 3];
    frame.support_normal = [0.0; 3];
}

/// `sv_phys.c:931` `SV_SetClientPusherMoveFrame`
fn sv_set_client_pusher_move_frame(
    frame: &mut ClientMoveFrame,
    pusher: *mut Edict,
    state: MoveFrameState,
    pusher_velocity: &[c_float; 3],
    support_normal: Option<&[c_float; 3]>,
) {
    frame.pusher = pusher;
    frame.state = state;
    frame.pusher_velocity = *pusher_velocity;
    frame.support_normal = match support_normal {
        Some(n) => *n,
        None => [0.0; 3],
    };
}

/// `sv_phys.c:943` `SV_CaptureRecordedPusherMoveFrame`. `captured` receives
/// C's return value.
unsafe fn sv_capture_recorded_pusher_move_frame(
    ent: *mut Edict,
    frame: &mut ClientMoveFrame,
    record: &PusherSupportRecord,
    pusher: *mut Edict,
    captured: &mut bool,
) -> Raise {
    // SAFETY: both edicts are live; ADR-008 ambient qcvm.
    unsafe {
        *captured = false;

        match record.state {
            MoveFrameState::Airborne => {
                if as_int((*ent).v.flags) & FL_ONGROUND != 0 {
                    return 0;
                }
                sv_set_client_pusher_move_frame(
                    frame,
                    pusher,
                    MoveFrameState::AirborneWorldVelocity,
                    &record.pusher_velocity,
                    None,
                );
                *captured = true;
                0
            }
            MoveFrameState::Ground => {
                if (*ent).v.groundentity != edict_to_prog(c::qcvm.cast::<QcVm>(), pusher) {
                    return 0;
                }
                let mut trace = trace_zeroed();
                let mut hit = false;
                let pusher_origin = (*pusher).v.origin;
                let raised = sv_trace_pusher_floor_at_origin(
                    ent,
                    pusher,
                    &pusher_origin,
                    PUSH_CONTACT_EPSILON as c_float,
                    &mut trace,
                    &mut hit,
                );
                if raised != 0 {
                    return raised;
                }
                if !hit {
                    return 0;
                }

                (*ent).v.flags = (as_int((*ent).v.flags) | FL_ONGROUND) as c_float;
                sv_set_client_pusher_move_frame(
                    frame,
                    pusher,
                    MoveFrameState::Ground,
                    &record.pusher_velocity,
                    Some(&trace.plane.normal),
                );
                *captured = true;
                0
            }
            _ => 0,
        }
    }
}

/// `sv_phys.c:971` `SV_CaptureClientMoveFrameBeforeQC`
unsafe fn sv_capture_client_move_frame_before_qc(
    ent: *mut Edict,
    frame: &mut ClientMoveFrame,
) -> Raise {
    // SAFETY: `ent` is a live edict; `NUM_FOR_EDICT`/`EDICT_NUM` are guarded.
    unsafe {
        sv_clear_client_move_frame(frame);
        if !qcvm_is_server() || sv_elevators_value() < 3.0f32 {
            return 0;
        }

        let mut entnum: c_int = 0;
        let raised = w::World_Glue_NumForEdict(ent.cast::<c_void>(), &mut entnum);
        if raised != 0 {
            return raised;
        }
        if entnum <= 0 || entnum >= MAX_EDICTS as c_int {
            return 0;
        }

        let vm = c::qcvm.cast::<QcVm>();
        let record = *pusher_support_slot(entnum);
        if record.frame != 0
            && record.frame.wrapping_add(1) == SV_PUSHER_SUPPORT_FRAME
            && record.pusher_entnum > 0
            && record.pusher_entnum < (*vm).num_edicts
        {
            let mut pusher_void: *mut c_void = ptr::null_mut();
            let raised = w::World_Glue_EdictNum(record.pusher_entnum, &mut pusher_void);
            if raised != 0 {
                return raised;
            }
            let pusher = pusher_void.cast::<Edict>();
            if sv_is_client_move_frame_pusher(pusher) {
                let mut captured = false;
                let raised = sv_capture_recorded_pusher_move_frame(
                    ent,
                    frame,
                    &record,
                    pusher,
                    &mut captured,
                );
                if raised != 0 {
                    return raised;
                }
                if captured {
                    return 0;
                }
            }
        }

        let pusher = sv_get_ground_pusher(ent);
        if pusher.is_null() || !sv_pusher_will_move_this_frame(pusher) {
            return 0;
        }

        let mut trace = trace_zeroed();
        let mut hit = false;
        let pusher_origin = (*pusher).v.origin;
        let raised = sv_trace_pusher_floor_at_origin(
            ent,
            pusher,
            &pusher_origin,
            PUSH_CONTACT_EPSILON as c_float,
            &mut trace,
            &mut hit,
        );
        if raised != 0 {
            return raised;
        }
        if hit {
            let pusher_velocity = (*pusher).v.velocity;
            sv_set_client_pusher_move_frame(
                frame,
                pusher,
                MoveFrameState::Ground,
                &pusher_velocity,
                Some(&trace.plane.normal),
            );
        }

        0
    }
}

/// `sv_phys.c:1005` `SV_ClientMoveFrameHasGroundSupport`
unsafe fn sv_client_move_frame_has_ground_support(frame: &ClientMoveFrame) -> bool {
    // SAFETY: `frame.pusher` is null or a live edict.
    unsafe {
        if frame.state != MoveFrameState::Ground {
            return false;
        }
        if !sv_is_client_move_frame_pusher(frame.pusher) {
            return false;
        }
        // COMPAT: ADR-010 -- `DIST_EPSILON * DIST_EPSILON` is a double product,
        // so the float dot promotes.
        if (dot(&frame.support_normal, &frame.support_normal) as f64) <= DIST_EPSILON * DIST_EPSILON
        {
            return false;
        }
        true
    }
}

/// `sv_phys.c:1018` `SV_ClientMoveFrameIsAirbornePusher`
unsafe fn sv_client_move_frame_is_airborne_pusher(frame: &ClientMoveFrame) -> bool {
    // SAFETY: `frame.pusher` is null or a live edict.
    unsafe {
        if frame.state != MoveFrameState::Airborne {
            return false;
        }
        sv_is_client_move_frame_pusher(frame.pusher)
    }
}

/// `sv_phys.c:1027` `SV_RecordAirbornePusherMoveFrame`
unsafe fn sv_record_airborne_pusher_move_frame(
    ent: *mut Edict,
    move_frame: &ClientMoveFrame,
) -> Raise {
    // SAFETY: `ent` and `move_frame.pusher` are live edicts.
    unsafe {
        if !qcvm_is_server() || sv_elevators_value() < 3.0f32 {
            return 0;
        }
        if !sv_client_move_frame_is_airborne_pusher(move_frame) {
            return 0;
        }
        if as_int((*ent).v.flags) & FL_ONGROUND != 0 {
            return 0;
        }

        let mut wrote = false;
        let pusher_velocity = (*move_frame.pusher).v.velocity;
        sv_write_pusher_support_record(
            ent,
            move_frame.pusher,
            MoveFrameState::Airborne,
            &pusher_velocity,
            &m::VEC3_ORIGIN,
            &mut wrote,
        )
    }
}

/// `sv_phys.c:1039` `SV_SetWalkMoveFrameClipContext`
unsafe fn sv_set_walk_move_frame_clip_context(move_frame: &ClientMoveFrame) {
    // SAFETY: single-threaded server state.
    unsafe {
        if !sv_client_move_frame_has_ground_support(move_frame) {
            SV_WALK_SUPPORT_PUSHER = ptr::null_mut();
            SV_WALK_SUPPORT_NORMAL = [0.0; 3];
            return;
        }

        SV_WALK_SUPPORT_PUSHER = move_frame.pusher;
        SV_WALK_SUPPORT_NORMAL = move_frame.support_normal;
    }
}

/// `sv_phys.c:1052` `SV_ClearWalkSupportClipContext`
unsafe fn sv_clear_walk_support_clip_context() {
    // SAFETY: single-threaded server state.
    unsafe {
        SV_WALK_SUPPORT_PUSHER = ptr::null_mut();
        SV_WALK_SUPPORT_NORMAL = [0.0; 3];
    }
}

/// `sv_phys.c:1058` `SV_FlyMoveWithMoveFrameClipContext`
unsafe fn sv_fly_move_with_move_frame_clip_context(
    ent: *mut Edict,
    time: c_float,
    move_frame: &ClientMoveFrame,
    steptrace: *mut Trace,
    clip: &mut c_int,
) -> Raise {
    // SAFETY: see `sv_fly_move`.
    unsafe {
        sv_set_walk_move_frame_clip_context(move_frame);
        let raised = sv_fly_move(ent, time, steptrace, clip);
        // COMPAT: ADR-009 -- C's longjmp skips the context teardown as well, so
        // the raising path returns before it.
        if raised != 0 {
            return raised;
        }
        sv_clear_walk_support_clip_context();
        0
    }
}

/// `sv_phys.c:1068` `SV_DropClientMoveFramePusherGround`
unsafe fn sv_drop_client_move_frame_pusher_ground(
    ent: *mut Edict,
    move_frame: &mut ClientMoveFrame,
) {
    // SAFETY: `ent` and `move_frame.pusher` are live edicts.
    unsafe {
        move_frame.state = MoveFrameState::Airborne;
        move_frame.pusher_velocity = (*move_frame.pusher).v.velocity;
        if (*ent).v.groundentity == edict_to_prog(c::qcvm.cast::<QcVm>(), move_frame.pusher) {
            (*ent).v.groundentity = 0;
        }
    }
}

/// `sv_phys.c:1076` `SV_UpdateClientMoveFrameAfterQC`
unsafe fn sv_update_client_move_frame_after_qc(ent: *mut Edict, move_frame: &mut ClientMoveFrame) {
    // SAFETY: `ent` is a live edict.
    unsafe {
        if move_frame.pusher.is_null() {
            return;
        }

        if move_frame.state == MoveFrameState::AirborneWorldVelocity {
            if (*ent).v.movetype != MOVETYPE_WALK {
                move_frame.state = MoveFrameState::None;
            }
            return;
        }

        if move_frame.state != MoveFrameState::Ground {
            return;
        }

        if as_int((*ent).v.flags) & FL_ONGROUND == 0 && (*ent).v.movetype == MOVETYPE_WALK {
            sv_drop_client_move_frame_pusher_ground(ent, move_frame);
        }
    }
}

/// `sv_phys.c:1094` `SV_BeginClientWalkMoveFrame`
unsafe fn sv_begin_client_walk_move_frame(ent: *mut Edict, move_frame: &mut ClientMoveFrame) {
    // SAFETY: `ent` is a live edict.
    unsafe {
        if move_frame.state != MoveFrameState::AirborneWorldVelocity {
            return;
        }
        if !sv_is_client_move_frame_pusher(move_frame.pusher) {
            move_frame.state = MoveFrameState::None;
            return;
        }

        let velocity = (*ent).v.velocity;
        m::vector_subtract(
            &velocity,
            &move_frame.pusher_velocity,
            &mut (*ent).v.velocity,
        );
        move_frame.pusher_velocity = (*move_frame.pusher).v.velocity;
        move_frame.state = MoveFrameState::Airborne;
    }
}

/// `sv_phys.c:1109` `SV_GroundClientOnMoveFramePusher`. `grounded` receives
/// C's return value.
unsafe fn sv_ground_client_on_move_frame_pusher(
    ent: *mut Edict,
    move_frame: &ClientMoveFrame,
    grounded: &mut bool,
) -> Raise {
    // SAFETY: `ent` and the pusher are live edicts; `SV_LinkEdict(false)`
    // dispatches no progs code.
    unsafe {
        *grounded = false;

        if !sv_client_move_frame_has_ground_support(move_frame) {
            return 0;
        }

        let mut trace = trace_zeroed();
        let mut hit = false;
        let pusher_origin = (*move_frame.pusher).v.origin;
        let raised = sv_trace_pusher_floor_at_origin(
            ent,
            move_frame.pusher,
            &pusher_origin,
            STEPSIZE as c_float,
            &mut trace,
            &mut hit,
        );
        if raised != 0 {
            return raised;
        }
        if !hit {
            return 0;
        }

        (*ent).v.origin = trace.endpos;
        let raised = crate::world::quake_rs_sv_link_edict(ent, false);
        if raised != 0 {
            return raised;
        }
        (*ent).v.flags = (as_int((*ent).v.flags) | FL_ONGROUND) as c_float;
        (*ent).v.groundentity = edict_to_prog(c::qcvm.cast::<QcVm>(), move_frame.pusher);
        (*ent).v.velocity[2] = 0.0;
        *grounded = true;
        0
    }
}

/// `sv_phys.c:1125` `SV_TestEntityPositionOnPusher`. `blocked` receives C's
/// return value.
unsafe fn sv_test_entity_position_on_pusher(
    ent: *mut Edict,
    pusher: *mut Edict,
    pusher_origin: &[c_float; 3],
    ent_origin: &[c_float; 3],
    blocked: &mut bool,
) -> Raise {
    // SAFETY: both edicts are live; the pusher origin swap mirrors C.
    unsafe {
        *blocked = false;

        let old_origin = (*pusher).v.origin;
        (*pusher).v.origin = *pusher_origin;
        let mut trace_origin = *ent_origin;
        let mut trace = trace_zeroed();
        let raised = crate::world::quake_rs_sv_clip_move_to_entity(
            &mut trace,
            pusher,
            trace_origin.as_mut_ptr(),
            ptr::addr_of_mut!((*ent).v.mins).cast::<c_float>(),
            ptr::addr_of_mut!((*ent).v.maxs).cast::<c_float>(),
            trace_origin.as_mut_ptr(),
            CONTENTMASK_ANYSOLID,
        );
        // COMPAT: ADR-009 -- the origin restore is skipped on the raising path,
        // exactly where C's longjmp leaves it.
        if raised != 0 {
            return raised;
        }
        (*pusher).v.origin = old_origin;
        *blocked = trace.startsolid;
        0
    }
}

/// `sv_phys.c:1138` `SV_EntityPositionBlockedIgnoringPusher`
unsafe fn sv_entity_position_blocked_ignoring_pusher(
    ent: *mut Edict,
    pusher: *mut Edict,
    blocked: &mut bool,
) -> Raise {
    // SAFETY: both edicts are live; `SV_TestEntityPosition` dispatches no progs
    // code but can raise from the collision pipeline.
    unsafe {
        *blocked = false;

        let solid_backup = (*pusher).v.solid;
        (*pusher).v.solid = SOLID_NOT;
        let mut hit: *mut Edict = ptr::null_mut();
        let raised = crate::world::quake_rs_sv_test_entity_position(ent, &mut hit);
        // COMPAT: ADR-009 -- C's longjmp leaves the pusher SOLID_NOT, so the
        // restore below is skipped on the raising path too.
        if raised != 0 {
            return raised;
        }
        (*pusher).v.solid = solid_backup;
        *blocked = !hit.is_null();
        0
    }
}

/// `sv_phys.c:1149` `SV_EntityRidingPusher`
unsafe fn sv_entity_riding_pusher(ent: *mut Edict, pusher: *mut Edict) -> bool {
    // SAFETY: `ent` is a live edict; ADR-008 ambient qcvm.
    unsafe {
        as_int((*ent).v.flags) & FL_ONGROUND != 0
            && prog_to_edict(c::qcvm.cast::<QcVm>(), (*ent).v.groundentity) == pusher
    }
}

/// `sv_phys.c:1154` `SV_PusherBoundsOverlapEntity`
unsafe fn sv_pusher_bounds_overlap_entity(
    ent: *mut Edict,
    mins: &[c_float; 3],
    maxs: &[c_float; 3],
) -> bool {
    // SAFETY: `ent` is a live edict.
    unsafe {
        !((*ent).v.absmin[0] >= maxs[0]
            || (*ent).v.absmin[1] >= maxs[1]
            || (*ent).v.absmin[2] >= maxs[2]
            || (*ent).v.absmax[0] <= mins[0]
            || (*ent).v.absmax[1] <= mins[1]
            || (*ent).v.absmax[2] <= mins[2])
    }
}

/// `sv_phys.c:1162` `SV_PusherAffectsEntity`. `out` carries both C results.
unsafe fn sv_pusher_affects_entity(
    ent: *mut Edict,
    pusher: *mut Edict,
    pushorig: &[c_float; 3],
    mins: &[c_float; 3],
    maxs: &[c_float; 3],
    robust_push: bool,
    out: &mut PusherAffect,
) -> Raise {
    // SAFETY: both edicts are live; every raising call is checked.
    unsafe {
        out.affects = false;
        out.riding = false;

        if sv_entity_riding_pusher(ent, pusher) {
            let mut touching = false;
            if robust_push {
                let raised = sv_touching_pusher_at_origin(ent, pusher, pushorig, &mut touching);
                if raised != 0 {
                    return raised;
                }
            }
            if !robust_push || touching {
                out.riding = true;
                out.affects = true;
                return 0;
            }
        }

        if robust_push {
            let mut touching = false;
            let raised = sv_touching_pusher_at_origin(ent, pusher, pushorig, &mut touching);
            if raised != 0 {
                return raised;
            }
            if touching
                && ((*ent).v.movetype != MOVETYPE_WALK || as_int((*ent).v.flags) & FL_ONGROUND != 0)
            {
                out.riding = true;
                out.affects = true;
                return 0;
            }
        }

        if !sv_pusher_bounds_overlap_entity(ent, mins, maxs) {
            return 0;
        }

        if !robust_push {
            if (*pusher).v.skin < 0.0 {
                let mut trace = trace_zeroed();
                let raised = crate::world::quake_rs_sv_clip_move_to_entity(
                    &mut trace,
                    pusher,
                    ptr::addr_of_mut!((*ent).v.origin).cast::<c_float>(),
                    ptr::addr_of_mut!((*ent).v.mins).cast::<c_float>(),
                    ptr::addr_of_mut!((*ent).v.maxs).cast::<c_float>(),
                    ptr::addr_of_mut!((*ent).v.origin).cast::<c_float>(),
                    CONTENTMASK_ANYSOLID,
                );
                if raised != 0 {
                    return raised;
                }
                out.affects = trace.startsolid;
                return 0;
            }
            let mut hit: *mut Edict = ptr::null_mut();
            let raised = crate::world::quake_rs_sv_test_entity_position(ent, &mut hit);
            if raised != 0 {
                return raised;
            }
            out.affects = !hit.is_null();
            return 0;
        }

        // Test the active pusher only; SV_TestEntityPosition can report an
        // unrelated platform the entity is already standing on.
        let pusher_origin = (*pusher).v.origin;
        let ent_origin = (*ent).v.origin;
        sv_test_entity_position_on_pusher(
            ent,
            pusher,
            &pusher_origin,
            &ent_origin,
            &mut out.affects,
        )
    }
}

/// `sv_phys.c:1197` `SV_PusherBlockIsPersistentRiderContact`.
///
/// C's `robust_push` and `riding` arguments are only ever used as
/// `robust_push && riding`, so they are merged here to keep the signature at
/// seven parameters. Pure argument folding, no behaviour change.
unsafe fn sv_pusher_block_is_persistent_rider_contact(
    ent: *mut Edict,
    pusher: *mut Edict,
    block: *mut Edict,
    pushorig: &[c_float; 3],
    entorig: &[c_float; 3],
    robust_riding: bool,
    persistent: &mut bool,
) -> Raise {
    // SAFETY: all three edicts are live.
    unsafe {
        *persistent = false;
        if !robust_riding || block != pusher {
            return 0;
        }

        // Existing rider contact with this pusher is not a new crush.
        sv_test_entity_position_on_pusher(ent, pusher, pushorig, entorig, persistent)
    }
}

/// `sv_phys.c:1207` `SV_PushEntityMove`
unsafe fn sv_push_entity_move(
    ent: *mut Edict,
    start: *mut c_float,
    end: *mut c_float,
    trace: &mut Trace,
) -> Raise {
    // SAFETY: `ent` is a live edict; `start`/`end` are `vec3_t`s.
    unsafe {
        let move_type = if (*ent).v.movetype == MOVETYPE_FLYMISSILE {
            MOVE_MISSILE
        } else if (*ent).v.solid == SOLID_TRIGGER || (*ent).v.solid == SOLID_NOT {
            // only clip against bmodels
            MOVE_NOMONSTERS
        } else {
            MOVE_NORMAL
        };

        crate::world::quake_rs_sv_move(
            trace,
            start,
            ptr::addr_of_mut!((*ent).v.mins).cast::<c_float>(),
            ptr::addr_of_mut!((*ent).v.maxs).cast::<c_float>(),
            end,
            move_type,
            ent,
        )
    }
}

/// `sv_phys.c:1224` `SV_PushEntityTo` -- does not change the entity's velocity.
unsafe fn sv_push_entity_to(ent: *mut Edict, end: *mut c_float, trace: &mut Trace) -> Raise {
    // SAFETY: `ent` is a live edict; `SV_LinkEdict(true)` may free `ent` and
    // `trace.ent`, which is why both are re-tested afterwards.
    unsafe {
        let mut origin = (*ent).v.origin;
        let raised = sv_push_entity_move(ent, origin.as_mut_ptr(), end, trace);
        if raised != 0 {
            return raised;
        }

        // a move that starts solid registers no impact, so an entity marginally inside the
        // pusher it rests on would glide through it and fall out the far side. un-embed
        // with a sweep against the pusher and redo the move so it collides normally.
        if trace.startsolid && (*ent).v.groundentity != 0 && sv_elevators_value() >= 3.0f32 {
            let vm = c::qcvm.cast::<QcVm>();
            let ground = prog_to_edict(vm, (*ent).v.groundentity);
            if ground != (*vm).edicts
                && !(*ground).free
                && (*ground).v.movetype == MOVETYPE_PUSH
                && (*ground).v.solid == SOLID_BSP
            {
                let mut embedded = trace_zeroed();
                let raised = crate::world::quake_rs_sv_clip_move_to_entity(
                    &mut embedded,
                    ground,
                    ptr::addr_of_mut!((*ent).v.origin).cast::<c_float>(),
                    ptr::addr_of_mut!((*ent).v.mins).cast::<c_float>(),
                    ptr::addr_of_mut!((*ent).v.maxs).cast::<c_float>(),
                    ptr::addr_of_mut!((*ent).v.origin).cast::<c_float>(),
                    CONTENTMASK_ANYSOLID,
                );
                if raised != 0 {
                    return raised;
                }

                if embedded.startsolid {
                    let mut above = (*ent).v.origin;
                    above[2] = (above[2] as f64 + PUSH_CONTACT_EPSILON) as c_float;

                    let mut exit = trace_zeroed();
                    let raised = crate::world::quake_rs_sv_clip_move_to_entity(
                        &mut exit,
                        ground,
                        above.as_mut_ptr(),
                        ptr::addr_of_mut!((*ent).v.mins).cast::<c_float>(),
                        ptr::addr_of_mut!((*ent).v.maxs).cast::<c_float>(),
                        ptr::addr_of_mut!((*ent).v.origin).cast::<c_float>(),
                        CONTENTMASK_ANYSOLID,
                    );
                    if raised != 0 {
                        return raised;
                    }

                    if !exit.startsolid && exit.fraction < 1.0 {
                        let raised = g::SvPhys_Glue_DPrintUnembedded(
                            ent.cast::<c_void>(),
                            ground.cast::<c_void>(),
                        );
                        if raised != 0 {
                            return raised;
                        }
                        (*ent).v.origin = exit.endpos;
                        let mut origin = (*ent).v.origin;
                        let raised = sv_push_entity_move(ent, origin.as_mut_ptr(), end, trace);
                        if raised != 0 {
                            return raised;
                        }
                    }
                }
            }
        }

        if !trace.ent.is_null() && (*trace.ent).free {
            let raised = w::World_Glue_AssertFailed(
                ASSERT_TRACE_ENT_FREE.as_ptr(),
                ASSERT_FILE.as_ptr(),
                1262,
            );
            if raised != 0 {
                return raised;
            }
        }

        (*ent).v.origin = trace.endpos;

        let raised = crate::world::quake_rs_sv_link_edict(ent, true);
        if raised != 0 {
            return raised;
        }

        // SV_LinkEdict(true) could have freed ent calling its touch program,
        // and also through calling SV_Touch_Links () internally could also free trace.ent.
        if !(*ent).free && !trace.ent.is_null() && !(*trace.ent).free {
            let raised = sv_impact(ent, trace.ent);
            if raised != 0 {
                return raised;
            }
        }

        0
    }
}

/// `sv_phys.c:1272` `SV_PushEntityToIgnoringPusher`
unsafe fn sv_push_entity_to_ignoring_pusher(
    ent: *mut Edict,
    pusher: *mut Edict,
    end: *mut c_float,
    trace: &mut Trace,
) -> Raise {
    // SAFETY: both edicts are live.
    unsafe {
        let solid_backup = (*pusher).v.solid;
        (*pusher).v.solid = SOLID_NOT;
        let raised = sv_push_entity_to(ent, end, trace);
        // COMPAT: ADR-009 -- C's longjmp leaves the pusher SOLID_NOT.
        if raised != 0 {
            return raised;
        }
        (*pusher).v.solid = solid_backup;
        0
    }
}

/// `sv_phys.c:1284` `SV_EndClientMoveFrame`
unsafe fn sv_end_client_move_frame(ent: *mut Edict, move_frame: &ClientMoveFrame) -> Raise {
    // SAFETY: `ent` and the pusher are live edicts; `ent` is re-tested after
    // the move because `SV_PushEntityTo` can free it.
    unsafe {
        if !sv_client_move_frame_is_airborne_pusher(move_frame) {
            return 0;
        }
        if (*ent).free || (*ent).v.movetype != MOVETYPE_WALK {
            return 0;
        }

        let mut move_v = [0.0 as c_float; 3];
        let movetime = sv_pusher_move_time_this_frame(move_frame.pusher);
        if movetime > 0.0 {
            let pusher_velocity = (*move_frame.pusher).v.velocity;
            m::vector_scale(&pusher_velocity, movetime, &mut move_v);
            if move_v[0] != 0.0 || move_v[1] != 0.0 || move_v[2] != 0.0 {
                let mut dest = [0.0 as c_float; 3];
                let origin = (*ent).v.origin;
                m::vector_add(&origin, &move_v, &mut dest);
                let mut trace = trace_zeroed();
                let raised = sv_push_entity_to_ignoring_pusher(
                    ent,
                    move_frame.pusher,
                    dest.as_mut_ptr(),
                    &mut trace,
                );
                if raised != 0 {
                    return raised;
                }
            }
        }

        if (*ent).free || (*ent).v.movetype != MOVETYPE_WALK {
            return 0;
        }

        if as_int((*ent).v.flags) & FL_ONGROUND != 0 {
            let ground = sv_get_ground_pusher(ent);
            if !ground.is_null() {
                let support_move = if ground == move_frame.pusher {
                    move_v
                } else {
                    [0.0; 3]
                };
                let mut wrote = false;
                let ground_velocity = (*ground).v.velocity;
                let raised = sv_write_pusher_support_record(
                    ent,
                    ground,
                    MoveFrameState::Ground,
                    &ground_velocity,
                    &support_move,
                    &mut wrote,
                );
                if raised != 0 {
                    return raised;
                }
            }
            return 0;
        }

        let raised = sv_record_airborne_pusher_move_frame(ent, move_frame);
        if raised != 0 {
            return raised;
        }
        let velocity = (*ent).v.velocity;
        m::vector_add(
            &velocity,
            &move_frame.pusher_velocity,
            &mut (*ent).v.velocity,
        );
        sv_check_velocity(ent)
    }
}

// ---------------------------------------------------------------------------
// SV_PushMove / SV_Physics_Pusher (sv_phys.c:1339-1620)

/// `sv_phys.c:1339` `SV_PushMove`
unsafe fn sv_push_move(pusher: *mut Edict, movetime: c_float) -> Raise {
    // SAFETY: every edict touched is a live arena slot; the `moved_*` arrays
    // are the C statics and are indexed below `MAX_EDICTS` exactly as in C.
    unsafe {
        if (*pusher).v.velocity[0] == 0.0
            && (*pusher).v.velocity[1] == 0.0
            && (*pusher).v.velocity[2] == 0.0
        {
            (*pusher).v.ltime += movetime;
            return 0;
        }

        let robust_push = sv_elevators_value() >= 3.0f32;
        let newltime = (*pusher).v.ltime + movetime;

        let mut move_v = [0.0 as c_float; 3];
        let mut neworigin = [0.0 as c_float; 3];
        let pusher_velocity = (*pusher).v.velocity;
        m::vector_scale(&pusher_velocity, movetime, &mut move_v);
        let pusher_origin = (*pusher).v.origin;
        m::vector_add(&pusher_origin, &move_v, &mut neworigin);

        let mut mins = [0.0 as c_float; 3];
        let mut maxs = [0.0 as c_float; 3];
        let mut querymins = [0.0 as c_float; 3];
        let mut querymaxs = [0.0 as c_float; 3];
        for i in 0..3 {
            mins[i] = (*pusher).v.absmin[i] + move_v[i];
            maxs[i] = (*pusher).v.absmax[i] + move_v[i];
            // the grid query must span the whole sweep: riders rest on the pre-move
            // box and are exempt from the final-box overlap test below
            querymins[i] = q_min_f((*pusher).v.absmin[i], mins[i]);
            querymaxs[i] = q_max_f((*pusher).v.absmax[i], maxs[i]);
        }

        let pushorig = (*pusher).v.origin;

        // move the pusher to it's final position
        (*pusher).v.origin = neworigin;
        (*pusher).v.ltime = newltime;
        let raised = crate::world::quake_rs_sv_link_edict(pusher, false);
        if raised != 0 {
            return raised;
        }

        // see if any solid entities are inside the final position
        let mut num_moved: usize = 0;

        let candidates = ptr::addr_of_mut!(PUSH_CANDIDATES).cast::<*mut Edict>();
        let cache = ptr::addr_of_mut!(PUSHABLE_ENT_CACHE).cast::<*mut Edict>();
        let mut fast_list: *mut *mut Edict = ptr::null_mut();
        let mut fast_count: c_int = 0;

        if PUSH_GRID_ACTIVE {
            fast_count = push_grid_gather_candidates(&querymins, &querymaxs, candidates);
            if fast_count >= 0 {
                fast_list = candidates;
            } else {
                // grid unusable this tick, scan the whole cache
                fast_list = cache;
                fast_count = NUM_PUSHABLE_ENT_CACHE;
            }
        } else if PUSH_CACHE_ACTIVE {
            // sv_pushgrid 0: scan the whole cache
            fast_list = cache;
            fast_count = NUM_PUSHABLE_ENT_CACHE;
        }

        let mut e: c_int = -1;

        // beware, we skip entity 0:
        let vm = c::qcvm.cast::<QcVm>();
        let mut check = next_edict(vm, (*vm).edicts);

        loop {
            let vm = c::qcvm.cast::<QcVm>();
            let limit = if fast_list.is_null() {
                (*vm).num_edicts - 1 - 1
            } else {
                fast_count - 1
            };
            if e >= limit {
                break;
            }

            e += 1;

            if !fast_list.is_null() {
                check = *fast_list.add(e as usize);
            } else if e > 0 {
                check = next_edict(vm, check);
            }

            if (*check).free {
                continue;
            }

            if !sv_is_pushable(check) {
                continue;
            }

            let mut affect = PusherAffect {
                affects: false,
                riding: false,
            };
            let raised = sv_pusher_affects_entity(
                check,
                pusher,
                &pushorig,
                &mins,
                &maxs,
                robust_push,
                &mut affect,
            );
            if raised != 0 {
                return raised;
            }
            if !affect.affects {
                continue;
            }
            let riding = affect.riding;

            // remove the onground flag for non-players
            if (*check).v.movetype != MOVETYPE_WALK {
                (*check).v.flags = (as_int((*check).v.flags) & !FL_ONGROUND) as c_float;
            }

            let entorig = (*check).v.origin;
            *ptr::addr_of_mut!(MOVED_FROM)
                .cast::<[c_float; 3]>()
                .add(num_moved) = entorig;
            let mut backup = PusherSupportRecord::ZERO;
            let raised = sv_backup_pusher_support(check, &mut backup);
            if raised != 0 {
                return raised;
            }
            *ptr::addr_of_mut!(MOVED_SUPPORT)
                .cast::<PusherSupportRecord>()
                .add(num_moved) = backup;
            *ptr::addr_of_mut!(MOVED_EDICT)
                .cast::<*mut Edict>()
                .add(num_moved) = check;
            num_moved += 1;

            // QIP fix for end.bsp
            let solid_backup = (*pusher).v.solid;
            let block: *mut Edict;
            if solid_backup == SOLID_BSP // everything that blocks: bsp models = map brushes = doors, plats, etc.
                || solid_backup == SOLID_BBOX // normally boxes
                || solid_backup == SOLID_SLIDEBOX
            // normally monsters
            {
                let mut dest = [0.0 as c_float; 3];
                if robust_push {
                    let mut applied_move = [0.0 as c_float; 3];

                    if riding {
                        let raised = sv_get_applied_pusher_support_move(check, &mut applied_move);
                        if raised != 0 {
                            return raised;
                        }
                    }

                    // Supported entities are carried by the pusher frame once per
                    // physics frame. Composite movers therefore apply only the
                    // difference from the support motion already applied.
                    for i in 0..3 {
                        dest[i] = entorig[i] + move_v[i] - applied_move[i];
                    }
                } else {
                    m::vector_add(&entorig, &move_v, &mut dest);
                }

                // try moving the contacted entity
                (*pusher).v.solid = SOLID_NOT;
                let mut trace = trace_zeroed();
                let raised = sv_push_entity_to(check, dest.as_mut_ptr(), &mut trace);
                if raised != 0 {
                    return raised;
                }

                // if it is still inside the pusher, block
                let mut hit: *mut Edict = ptr::null_mut();
                if (*pusher).v.skin < 0.0 {
                    // if it has forced contents then do things in a slightly different order, so water can push properly.
                    let raised = crate::world::quake_rs_sv_test_entity_position(check, &mut hit);
                    if raised != 0 {
                        return raised;
                    }
                    (*pusher).v.solid = solid_backup;
                } else {
                    (*pusher).v.solid = solid_backup;
                    let raised = crate::world::quake_rs_sv_test_entity_position(check, &mut hit);
                    if raised != 0 {
                        return raised;
                    }
                }
                block = hit;
            } else {
                block = ptr::null_mut();
            }

            if !block.is_null() {
                // fail the move
                if (*check).v.mins[0] == (*check).v.maxs[0] {
                    continue;
                }

                let mut persistent = false;
                let raised = sv_pusher_block_is_persistent_rider_contact(
                    check,
                    pusher,
                    block,
                    &pushorig,
                    &entorig,
                    robust_push && riding,
                    &mut persistent,
                );
                if raised != 0 {
                    return raised;
                }
                if persistent {
                    if riding {
                        let raised = sv_record_pusher_support(check, pusher, &move_v);
                        if raised != 0 {
                            return raised;
                        }
                    }
                    continue;
                }

                // riders only embed through their ground contact and never deeper than
                // PUSH_CONTACT_EPSILON, so a single sweep from above recovers the exact
                // contact position. must run before the corpse path so items don't get
                // their bbox zeroed over a rounding error; real squeezes still crush.
                if robust_push && riding && block == pusher {
                    let mut pushedorg = (*check).v.origin;
                    let mut above = (*check).v.origin;
                    above[2] = (above[2] as f64 + PUSH_CONTACT_EPSILON) as c_float;
                    let mut settle = trace_zeroed();
                    let raised = sv_push_entity_move(
                        check,
                        above.as_mut_ptr(),
                        pushedorg.as_mut_ptr(),
                        &mut settle,
                    );
                    if raised != 0 {
                        return raised;
                    }
                    if !settle.startsolid {
                        (*check).v.origin = settle.endpos;
                        let mut stuck: *mut Edict = ptr::null_mut();
                        let raised =
                            crate::world::quake_rs_sv_test_entity_position(check, &mut stuck);
                        if raised != 0 {
                            return raised;
                        }
                        if stuck.is_null() {
                            let raised = crate::world::quake_rs_sv_link_edict(check, false);
                            if raised != 0 {
                                return raised;
                            }
                            let raised = sv_record_pusher_support(check, pusher, &move_v);
                            if raised != 0 {
                                return raised;
                            }
                            continue;
                        }
                        (*check).v.origin = pushedorg;
                    }
                }

                if (*check).v.solid == SOLID_NOT || (*check).v.solid == SOLID_TRIGGER {
                    // corpse
                    (*check).v.mins[1] = 0.0;
                    (*check).v.mins[0] = (*check).v.mins[1];
                    (*check).v.maxs = (*check).v.mins;
                    continue;
                }

                // try moving the entity up a bit if it's blocked by the pusher while also standing on it
                if !robust_push && riding && block == pusher {
                    let elevators = sv_elevators_value();
                    let mut nudge = elevators >= 2.0f32;
                    if !nudge && elevators != 0.0 {
                        let mut checknum: c_int = 0;
                        let raised =
                            w::World_Glue_NumForEdict(check.cast::<c_void>(), &mut checknum);
                        if raised != 0 {
                            return raised;
                        }
                        nudge = checknum <= g::SvPhys_Glue_MaxClients();
                    }
                    if nudge {
                        (*check).v.origin[2] =
                            ((*check).v.origin[2] as f64 + DIST_EPSILON) as c_float;
                        let mut stuck: *mut Edict = ptr::null_mut();
                        let raised =
                            crate::world::quake_rs_sv_test_entity_position(check, &mut stuck);
                        if raised != 0 {
                            return raised;
                        }
                        if stuck.is_null() {
                            continue;
                        }
                    }
                }

                (*check).v.origin = entorig;
                let raised = crate::world::quake_rs_sv_link_edict(check, true);
                if raised != 0 {
                    return raised;
                }

                (*pusher).v.origin = pushorig;
                let raised = crate::world::quake_rs_sv_link_edict(pusher, false);
                if raised != 0 {
                    return raised;
                }
                (*pusher).v.ltime -= movetime;

                // if the pusher has a "blocked" function, call it
                // otherwise, just stay in place until the obstacle is gone
                if (*pusher).v.blocked != 0 {
                    let raised =
                        g::SvPhys_Glue_CallBlocked(pusher.cast::<c_void>(), check.cast::<c_void>());
                    if raised != 0 {
                        return raised;
                    }
                }

                // move back any entities we already moved
                for i in 0..num_moved {
                    let moved = *ptr::addr_of!(MOVED_EDICT).cast::<*mut Edict>().add(i);
                    let record = *ptr::addr_of!(MOVED_SUPPORT)
                        .cast::<PusherSupportRecord>()
                        .add(i);
                    let raised = sv_restore_pusher_support(moved, &record);
                    if raised != 0 {
                        return raised;
                    }
                    (*moved).v.origin = *ptr::addr_of!(MOVED_FROM).cast::<[c_float; 3]>().add(i);
                    let raised = crate::world::quake_rs_sv_link_edict(moved, false);
                    if raised != 0 {
                        return raised;
                    }
                }
                break;
            }

            if riding {
                let raised = sv_record_pusher_support(check, pusher, &move_v);
                if raised != 0 {
                    return raised;
                }
            }
        }

        0
    }
}

/// `sv_phys.c:1585` `SV_Physics_Pusher`
unsafe fn sv_physics_pusher(ent: *mut Edict) -> Raise {
    // SAFETY: `ent` is a live edict; the think dispatch runs under a guard.
    unsafe {
        let oldltime = (*ent).v.ltime;

        let thinktime = (*ent).v.nextthink;
        let movetime = sv_pusher_move_time_this_frame(ent);

        let timing =
            sv_speeds_on() && qcvm_is_server() && (movetime != 0.0 || thinktime > oldltime);
        let mut push_start: f64 = 0.0;
        if timing {
            push_start = c::Sys_DoubleTime();
        }

        if movetime != 0.0 {
            // advances ent->v.ltime if not blocked
            let raised = sv_push_move(ent, movetime);
            if raised != 0 {
                return raised;
            }
        }

        if thinktime > oldltime && thinktime <= (*ent).v.ltime {
            (*ent).v.nextthink = 0.0;
            let vm = c::qcvm.cast::<QcVm>();
            let raised = g::SvPhys_Glue_CallThink(ent.cast::<c_void>(), (*vm).time as c_float);
            if raised != 0 {
                return raised;
            }
        }

        if timing {
            let ms = ptr::addr_of_mut!(g::sv_speeds_pusher_ms);
            *ms += (c::Sys_DoubleTime() - push_start) * 1000.0;
            let n = ptr::addr_of_mut!(g::sv_speeds_pushers);
            *n += 1;
        }

        0
    }
}

// ---------------------------------------------------------------------------
// client movement (sv_phys.c:1630-2068)

/// `sv_phys.c:1638` `SV_CheckStuck`
unsafe fn sv_check_stuck(ent: *mut Edict) -> Raise {
    // SAFETY: `ent` is a live edict; `SV_TestEntityPosition` dispatches no
    // progs code.
    unsafe {
        let mut stuck: *mut Edict = ptr::null_mut();
        let raised = crate::world::quake_rs_sv_test_entity_position(ent, &mut stuck);
        if raised != 0 {
            return raised;
        }
        if stuck.is_null() {
            (*ent).v.oldorigin = (*ent).v.origin;
            return 0;
        }

        let org = (*ent).v.origin;
        (*ent).v.origin = (*ent).v.oldorigin;
        let mut stuck: *mut Edict = ptr::null_mut();
        let raised = crate::world::quake_rs_sv_test_entity_position(ent, &mut stuck);
        if raised != 0 {
            return raised;
        }
        if stuck.is_null() {
            g::SvPhys_Glue_DPrintUnstuck();
            return crate::world::quake_rs_sv_link_edict(ent, true);
        }

        for z in 0..18 {
            for i in -1..=1 {
                for j in -1..=1 {
                    (*ent).v.origin[0] = org[0] + i as c_float;
                    (*ent).v.origin[1] = org[1] + j as c_float;
                    (*ent).v.origin[2] = org[2] + z as c_float;
                    let mut stuck: *mut Edict = ptr::null_mut();
                    let raised = crate::world::quake_rs_sv_test_entity_position(ent, &mut stuck);
                    if raised != 0 {
                        return raised;
                    }
                    if stuck.is_null() {
                        g::SvPhys_Glue_DPrintUnstuck();
                        return crate::world::quake_rs_sv_link_edict(ent, true);
                    }
                }
            }
        }

        (*ent).v.origin = org;
        g::SvPhys_Glue_DPrintPlayerStuck();
        0
    }
}

/// `sv_phys.c:1681` `SV_CheckStuckWithMoveFrame`
unsafe fn sv_check_stuck_with_move_frame(ent: *mut Edict, move_frame: &ClientMoveFrame) -> Raise {
    // SAFETY: `ent` and the pusher are live edicts.
    unsafe {
        if !sv_client_move_frame_has_ground_support(move_frame) {
            return sv_check_stuck(ent);
        }

        let mut stuck: *mut Edict = ptr::null_mut();
        let raised = crate::world::quake_rs_sv_test_entity_position(ent, &mut stuck);
        if raised != 0 {
            return raised;
        }
        if stuck.is_null() {
            (*ent).v.oldorigin = (*ent).v.origin;
            return 0;
        }

        let mut blocked = false;
        let raised =
            sv_entity_position_blocked_ignoring_pusher(ent, move_frame.pusher, &mut blocked);
        if raised != 0 {
            return raised;
        }
        if !blocked {
            (*ent).v.oldorigin = (*ent).v.origin;
            return 0;
        }

        sv_check_stuck(ent)
    }
}

/// `sv_phys.c:1707` `SV_CheckWater`
unsafe fn sv_check_water(ent: *mut Edict) -> bool {
    // SAFETY: `ent` is a live edict; `SV_PointContents` cannot raise.
    unsafe {
        let mut point = [0.0 as c_float; 3];
        point[0] = (*ent).v.origin[0];
        point[1] = (*ent).v.origin[1];
        point[2] = (*ent).v.origin[2] + (*ent).v.mins[2] + 1.0;

        (*ent).v.waterlevel = 0.0;
        (*ent).v.watertype = CONTENTS_EMPTY as c_float;
        let mut cont = crate::world::SV_PointContents(point.as_mut_ptr());
        if cont <= CONTENTS_WATER {
            (*ent).v.watertype = cont as c_float;
            (*ent).v.waterlevel = 1.0;
            point[2] = (*ent).v.origin[2] + ((*ent).v.mins[2] + (*ent).v.maxs[2]) * 0.5;
            cont = crate::world::SV_PointContents(point.as_mut_ptr());
            if cont <= CONTENTS_WATER {
                (*ent).v.waterlevel = 2.0;
                point[2] = (*ent).v.origin[2] + (*ent).v.view_ofs[2];
                cont = crate::world::SV_PointContents(point.as_mut_ptr());
                if cont <= CONTENTS_WATER {
                    (*ent).v.waterlevel = 3.0;
                }
            }
        }

        (*ent).v.waterlevel > 1.0
    }
}

/// `sv_phys.c:1740` `SV_WallFriction`
unsafe fn sv_wall_friction(ent: *mut Edict, trace: &Trace) {
    // SAFETY: `ent` is a live edict.
    unsafe {
        let mut forward = [0.0 as c_float; 3];
        let mut right = [0.0 as c_float; 3];
        let mut up = [0.0 as c_float; 3];

        let v_angle = (*ent).v.v_angle;
        m::angle_vectors(&v_angle, &mut forward, &mut right, &mut up);
        let mut d = dot(&trace.plane.normal, &forward);

        // COMPAT: ADR-010 -- `0.5` is a double literal, so the increment is a
        // double add narrowed back to float.
        d = (d as f64 + 0.5) as c_float;
        if d >= 0.0 {
            return;
        }

        // cut the tangential velocity
        let velocity = (*ent).v.velocity;
        let i = dot(&trace.plane.normal, &velocity);
        let mut into = [0.0 as c_float; 3];
        let mut side = [0.0 as c_float; 3];
        m::vector_scale(&trace.plane.normal, i, &mut into);
        m::vector_subtract(&velocity, &into, &mut side);

        (*ent).v.velocity[0] = side[0] * (1.0 + d);
        (*ent).v.velocity[1] = side[1] * (1.0 + d);
    }
}

/// `sv_phys.c:1772` `SV_TryUnstick`. `clip` receives C's return value.
unsafe fn sv_try_unstick(ent: *mut Edict, oldvel: &[c_float; 3], clip: &mut c_int) -> Raise {
    // SAFETY: `ent` is a live edict; `SV_PushEntityTo` may dispatch touch
    // functions, after which nothing cached across the call is reused.
    unsafe {
        *clip = 0;

        let oldorg = (*ent).v.origin;
        let mut dir = [0.0 as c_float; 3];

        for i in 0..8 {
            // try pushing a little in an axial direction
            match i {
                0 => {
                    dir[0] = 2.0;
                    dir[1] = 0.0;
                }
                1 => {
                    dir[0] = 0.0;
                    dir[1] = 2.0;
                }
                2 => {
                    dir[0] = -2.0;
                    dir[1] = 0.0;
                }
                3 => {
                    dir[0] = 0.0;
                    dir[1] = -2.0;
                }
                4 => {
                    dir[0] = 2.0;
                    dir[1] = 2.0;
                }
                5 => {
                    dir[0] = -2.0;
                    dir[1] = 2.0;
                }
                6 => {
                    dir[0] = 2.0;
                    dir[1] = -2.0;
                }
                _ => {
                    dir[0] = -2.0;
                    dir[1] = -2.0;
                }
            }

            let mut dest = [0.0 as c_float; 3];
            let origin = (*ent).v.origin;
            m::vector_add(&origin, &dir, &mut dest);
            let mut trace = trace_zeroed();
            let raised = sv_push_entity_to(ent, dest.as_mut_ptr(), &mut trace);
            if raised != 0 {
                return raised;
            }

            // retry the original move
            (*ent).v.velocity[0] = oldvel[0];
            (*ent).v.velocity[1] = oldvel[1];
            (*ent).v.velocity[2] = 0.0;
            let mut steptrace = trace_zeroed();
            let raised = sv_fly_move(ent, 0.1, &mut steptrace, clip);
            if raised != 0 {
                return raised;
            }

            if c::libm::fabs((oldorg[1] - (*ent).v.origin[1]) as f64) > 4.0
                || c::libm::fabs((oldorg[0] - (*ent).v.origin[0]) as f64) > 4.0
            {
                // Con_DPrintf ("unstuck!\n");
                return 0;
            }

            // go back to the original pos and try again
            (*ent).v.origin = oldorg;
        }

        (*ent).v.velocity = m::VEC3_ORIGIN;
        *clip = 7; // still not moving
        0
    }
}

/// `sv_phys.c:1857` `SV_WalkMove` -- only used by players.
unsafe fn sv_walk_move(ent: *mut Edict, move_frame: &ClientMoveFrame) -> Raise {
    // SAFETY: `ent` is a live edict for the whole call; `SV_PushEntityTo` can
    // free `downtrace.ent`, which is why it is re-tested below.
    unsafe {
        //
        // do a regular slide move unless it looks like you ran into a step
        //
        let oldonground = as_int((*ent).v.flags) & FL_ONGROUND;
        (*ent).v.flags = (as_int((*ent).v.flags) & !FL_ONGROUND) as c_float;

        let oldorg = (*ent).v.origin;
        let oldvel = (*ent).v.velocity;

        let mut steptrace = trace_zeroed();
        let mut clip: c_int = 0;
        let raised = sv_fly_move_with_move_frame_clip_context(
            ent,
            host_frametime() as c_float,
            move_frame,
            &mut steptrace,
            &mut clip,
        );
        if raised != 0 {
            return raised;
        }

        if clip & 2 == 0 {
            let mut grounded = false;
            // move didn't block on a step
            return sv_ground_client_on_move_frame_pusher(ent, move_frame, &mut grounded);
        }

        if oldonground == 0 && (*ent).v.waterlevel == 0.0 {
            return 0; // don't stair up while jumping
        }

        if (*ent).v.movetype != MOVETYPE_WALK {
            return 0; // gibbed by a trigger
        }

        if cvar_value(ptr::addr_of!(g::sv_nostep)) != 0.0 {
            return 0;
        }

        let sv_player = g::SvPhys_Glue_SvPlayer().cast::<Edict>();
        if as_int((*sv_player).v.flags) & FL_WATERJUMP != 0 {
            return 0;
        }

        let nosteporg = (*ent).v.origin;
        let nostepvel = (*ent).v.velocity;

        //
        // try moving up and forward to go up a step
        //
        (*ent).v.origin = oldorg; // back to start pos

        let mut upmove = (*ent).v.origin;
        upmove[2] += STEPSIZE as c_float;

        // move up
        let mut trace = trace_zeroed();
        let raised = sv_push_entity_to(ent, upmove.as_mut_ptr(), &mut trace); // FIXME: don't link?
        if raised != 0 {
            return raised;
        }

        // move forward
        (*ent).v.velocity[0] = oldvel[0];
        (*ent).v.velocity[1] = oldvel[1];
        (*ent).v.velocity[2] = 0.0;
        let raised = sv_fly_move_with_move_frame_clip_context(
            ent,
            host_frametime() as c_float,
            move_frame,
            &mut steptrace,
            &mut clip,
        );
        if raised != 0 {
            return raised;
        }

        // check for stuckness, possibly due to the limited precision of floats
        // in the clipping hulls. Disable when using pr_checkextension to avoid
        // https://github.com/Shpoike/Quakespasm/issues/50.
        if clip != 0 && cvar_value(ptr::addr_of!(w::pr_checkextension)) == 0.0 {
            // stepping up didn't make any progress
            if c::libm::fabs((oldorg[1] - (*ent).v.origin[1]) as f64) < 0.03125
                && c::libm::fabs((oldorg[0] - (*ent).v.origin[0]) as f64) < 0.03125
            {
                let raised = sv_try_unstick(ent, &oldvel, &mut clip);
                if raised != 0 {
                    return raised;
                }
            }
        }

        // extra friction based on view angle
        if clip & 2 != 0 {
            sv_wall_friction(ent, &steptrace);
        }

        // move down
        let mut downmove = (*ent).v.origin;
        // COMPAT: ADR-010 -- `-STEPSIZE` is an int and `oldvel[2] *
        // host_frametime` is a double, so the whole increment is a double.
        downmove[2] = (downmove[2] as f64
            + (-(STEPSIZE as f64) + oldvel[2] as f64 * host_frametime()))
            as c_float;
        let mut downtrace = trace_zeroed();
        let raised = sv_push_entity_to(ent, downmove.as_mut_ptr(), &mut downtrace); // FIXME: don't link?
        if raised != 0 {
            return raised;
        }

        if downtrace.plane.normal[2] > MIN_WALK_NORMAL {
            if (*ent).v.solid == SOLID_BSP
                || (sv_client_move_frame_has_ground_support(move_frame)
                    && downtrace.ent == move_frame.pusher)
            {
                (*ent).v.flags = (as_int((*ent).v.flags) | FL_ONGROUND) as c_float;

                // SV_PushEntityTo() calls SV_LinkEdict (true) that could free downtrace.ent
                if !downtrace.ent.is_null() && !(*downtrace.ent).free {
                    (*ent).v.groundentity = edict_to_prog(c::qcvm.cast::<QcVm>(), downtrace.ent);
                }
            }
        } else {
            // if the push down didn't end up on good ground, use the move without
            // the step up.  This happens near wall / slope combinations, and can
            // cause the player to hop up higher on a slope too steep to climb
            (*ent).v.origin = nosteporg;
            (*ent).v.velocity = nostepvel;
            let mut grounded = false;
            return sv_ground_client_on_move_frame_pusher(ent, move_frame, &mut grounded);
        }

        0
    }
}

/// `sv_phys.c:1966` `SV_Physics_ClientWalk`
unsafe fn sv_physics_client_walk(ent: *mut Edict, move_frame: &mut ClientMoveFrame) -> Raise {
    // SAFETY: `ent` is a live edict; the assert reaches Host_Error through the
    // guarded glue helper.
    unsafe {
        sv_begin_client_walk_move_frame(ent, move_frame);

        let supported_by_pusher = sv_client_move_frame_has_ground_support(move_frame);
        let in_water = sv_check_water(ent);
        let apply_gravity =
            !supported_by_pusher && !in_water && (as_int((*ent).v.flags) & FL_WATERJUMP) == 0;

        if apply_gravity {
            sv_add_gravity(ent);
        } else if supported_by_pusher {
            (*ent).v.velocity[2] = 0.0;
        }

        let raised = sv_check_stuck_with_move_frame(ent, move_frame);
        if raised != 0 {
            return raised;
        }
        if (*ent).free {
            let raised =
                w::World_Glue_AssertFailed(ASSERT_ENT_FREE.as_ptr(), ASSERT_FILE.as_ptr(), 1983);
            if raised != 0 {
                return raised;
            }
        }
        let raised = sv_walk_move(ent, move_frame);
        if raised != 0 {
            return raised;
        }

        if !(*ent).free && apply_gravity {
            sv_finish_gravity(ent);
        }
        if !(*ent).free {
            let raised = sv_end_client_move_frame(ent, move_frame);
            if raised != 0 {
                return raised;
            }
        }

        0
    }
}

/// `sv_phys.c:1991` `SV_Physics_Client`
unsafe fn sv_physics_client(ent: *mut Edict, num: c_int) -> Raise {
    // SAFETY: `ent` is a live edict; the player thinks run under guards and the
    // ambient qcvm is re-resolved after each dispatch (ADR-006, ADR-008).
    unsafe {
        if g::SvPhys_Glue_ClientActive(num) == 0 {
            return 0; // unconnected slot
        }

        if g::SvPhys_Glue_ClientKnownToQc(num) == 0
            && cvar_value(ptr::addr_of!(g::sv_gameplayfix_spawnbeforethinks)) != 0.0
        {
            return 0; // don't spam prethinks before we called putclientinserver.
        }

        let mut move_frame = ClientMoveFrame {
            pusher: ptr::null_mut(),
            state: MoveFrameState::None,
            pusher_velocity: [0.0; 3],
            support_normal: [0.0; 3],
        };
        let raised = sv_capture_client_move_frame_before_qc(ent, &mut move_frame);
        if raised != 0 {
            return raised;
        }

        //
        // call standard client pre-think
        //
        let vm = c::qcvm.cast::<QcVm>();
        let raised = g::SvPhys_Glue_CallPlayerPreThink(ent.cast::<c_void>(), (*vm).time as c_float);
        if raised != 0 {
            return raised;
        }

        if (*ent).free {
            let raised =
                w::World_Glue_AssertFailed(ASSERT_ENT_FREE.as_ptr(), ASSERT_FILE.as_ptr(), 2011);
            if raised != 0 {
                return raised;
            }
        }

        sv_update_client_move_frame_after_qc(ent, &mut move_frame);

        //
        // do a move
        //
        let raised = sv_check_velocity(ent);
        if raised != 0 {
            return raised;
        }

        //
        // decide which move function to call
        //
        let movetype = as_int((*ent).v.movetype) as c_float;
        if movetype == MOVETYPE_NONE {
            let mut alive = false;
            let raised = sv_run_think(ent, &mut alive);
            if raised != 0 {
                return raised;
            }
            if !alive {
                return 0;
            }
        } else if movetype == MOVETYPE_WALK {
            let mut alive = false;
            let raised = sv_run_think(ent, &mut alive);
            if raised != 0 {
                return raised;
            }
            if !alive {
                return 0;
            }
            let raised = sv_physics_client_walk(ent, &mut move_frame);
            if raised != 0 {
                return raised;
            }
        } else if movetype == MOVETYPE_TOSS
            || movetype == MOVETYPE_BOUNCE
            || movetype == MOVETYPE_GIB
        {
            let raised = sv_physics_toss(ent);
            if raised != 0 {
                return raised;
            }
        } else if movetype == MOVETYPE_FLY {
            let mut alive = false;
            let raised = sv_run_think(ent, &mut alive);
            if raised != 0 {
                return raised;
            }
            if !alive {
                return 0;
            }
            let mut clip: c_int = 0;
            let raised = sv_fly_move(ent, host_frametime() as c_float, ptr::null_mut(), &mut clip);
            if raised != 0 {
                return raised;
            }
        } else if movetype == MOVETYPE_NOCLIP {
            let mut alive = false;
            let raised = sv_run_think(ent, &mut alive);
            if raised != 0 {
                return raised;
            }
            if !alive {
                return 0;
            }
            let origin = (*ent).v.origin;
            let velocity = (*ent).v.velocity;
            vector_ma(
                &origin,
                host_frametime() as c_float,
                &velocity,
                &mut (*ent).v.origin,
            );
        } else {
            let raised = g::SvPhys_Glue_EndGameBadClientMovetype(as_int((*ent).v.movetype));
            if raised != 0 {
                return raised;
            }
        }

        //
        // call standard player post-think
        //
        let raised = crate::world::quake_rs_sv_link_edict(ent, true);
        if raised != 0 {
            return raised;
        }

        if (*ent).free {
            let raised =
                w::World_Glue_AssertFailed(ASSERT_ENT_FREE.as_ptr(), ASSERT_FILE.as_ptr(), 2063);
            if raised != 0 {
                return raised;
            }
        }

        let vm = c::qcvm.cast::<QcVm>();
        g::SvPhys_Glue_CallPlayerPostThink(ent.cast::<c_void>(), (*vm).time as c_float)
    }
}

// ---------------------------------------------------------------------------
// SV_Physics_None / SV_Physics_Noclip (sv_phys.c:2078-2104)

/// `sv_phys.c:2078` `SV_Physics_None`
unsafe fn sv_physics_none(ent: *mut Edict) -> Raise {
    // SAFETY: `ent` is a live edict.
    unsafe {
        // regular thinking
        let mut alive = false;
        sv_run_think(ent, &mut alive)
    }
}

/// `sv_phys.c:2090` `SV_Physics_Noclip`
unsafe fn sv_physics_noclip(ent: *mut Edict) -> Raise {
    // SAFETY: `ent` is a live edict.
    unsafe {
        // regular thinking
        let mut alive = false;
        let raised = sv_run_think(ent, &mut alive);
        if raised != 0 {
            return raised;
        }
        if !alive {
            return 0;
        }

        // stationary: the move below would be an exact no-op, skip the relink (and its BSP leaf walk)
        if (*ent).v.velocity[0] == 0.0
            && (*ent).v.velocity[1] == 0.0
            && (*ent).v.velocity[2] == 0.0
            && (*ent).v.avelocity[0] == 0.0
            && (*ent).v.avelocity[1] == 0.0
            && (*ent).v.avelocity[2] == 0.0
        {
            return 0;
        }

        let angles = (*ent).v.angles;
        let avelocity = (*ent).v.avelocity;
        vector_ma(
            &angles,
            host_frametime() as c_float,
            &avelocity,
            &mut (*ent).v.angles,
        );
        let origin = (*ent).v.origin;
        let velocity = (*ent).v.velocity;
        vector_ma(
            &origin,
            host_frametime() as c_float,
            &velocity,
            &mut (*ent).v.origin,
        );

        crate::world::quake_rs_sv_link_edict(ent, false)
    }
}

// ---------------------------------------------------------------------------
// toss / bounce (sv_phys.c:2118-2290)

/// `sv_phys.c:2122` `SV_CheckWaterTransition`.
///
/// # Safety
/// The ambient qcvm must be loaded (ADR-008) and `ent` must be a live edict.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_check_water_transition(ent: *mut Edict) -> Raise {
    // SAFETY: per the fn docs.
    unsafe { sv_check_water_transition(ent) }
}

unsafe fn sv_check_water_transition(ent: *mut Edict) -> Raise {
    // SAFETY: `ent` is a live edict; `SV_StartSound` runs under a guard.
    unsafe {
        let cont =
            crate::world::SV_PointContents(ptr::addr_of_mut!((*ent).v.origin).cast::<c_float>());

        if (*ent).v.watertype == 0.0 {
            // just spawned here
            (*ent).v.watertype = cont as c_float;
            (*ent).v.waterlevel = 1.0;
            return 0;
        }

        if cont <= CONTENTS_WATER {
            if (*ent).v.watertype == CONTENTS_EMPTY as c_float {
                // just crossed into water
                let raised = g::SvPhys_Glue_StartSound(
                    ent.cast::<c_void>(),
                    0,
                    SND_H2OHIT1.as_ptr(),
                    255,
                    1.0,
                );
                if raised != 0 {
                    return raised;
                }
            }
            (*ent).v.watertype = cont as c_float;
            (*ent).v.waterlevel = 1.0;
        } else {
            if (*ent).v.watertype != CONTENTS_EMPTY as c_float {
                // just crossed into water
                let raised = g::SvPhys_Glue_StartSound(
                    ent.cast::<c_void>(),
                    0,
                    SND_H2OHIT1.as_ptr(),
                    255,
                    1.0,
                );
                if raised != 0 {
                    return raised;
                }
            }
            (*ent).v.watertype = CONTENTS_EMPTY as c_float;
            (*ent).v.waterlevel = cont as c_float;
        }

        0
    }
}

/// `sv_phys.c:2166` `SV_Physics_Toss` -- toss, bounce, and fly movement.
unsafe fn sv_physics_toss(ent: *mut Edict) -> Raise {
    // SAFETY: `ent` is a live edict until `SV_PushEntityTo`, after which its
    // `free` flag is re-tested exactly as in C.
    unsafe {
        // regular thinking
        let mut alive = false;
        let raised = sv_run_think(ent, &mut alive);
        if raised != 0 {
            return raised;
        }
        if !alive {
            return 0;
        }

        // if onground, return without moving
        if as_int((*ent).v.flags) & FL_ONGROUND != 0 {
            return 0;
        }

        let raised = sv_check_velocity(ent);
        if raised != 0 {
            return raised;
        }

        // add gravity
        if (*ent).v.movetype != MOVETYPE_FLY && (*ent).v.movetype != MOVETYPE_FLYMISSILE {
            sv_add_gravity(ent);
        }

        // move angles
        let angles = (*ent).v.angles;
        let avelocity = (*ent).v.avelocity;
        vector_ma(
            &angles,
            host_frametime() as c_float,
            &avelocity,
            &mut (*ent).v.angles,
        );

        // move origin
        let mut end = [0.0 as c_float; 3];
        let origin = (*ent).v.origin;
        let velocity = (*ent).v.velocity;
        vector_ma(&origin, host_frametime() as c_float, &velocity, &mut end);
        let mut trace = trace_zeroed();
        let raised = sv_push_entity_to(ent, end.as_mut_ptr(), &mut trace);
        if raised != 0 {
            return raised;
        }

        if (*ent).free {
            return 0;
        }

        if (*ent).v.movetype != MOVETYPE_FLY && (*ent).v.movetype != MOVETYPE_FLYMISSILE {
            sv_finish_gravity(ent);
        }

        if trace.fraction == 1.0 {
            return 0;
        }

        let backoff: c_float = if (*ent).v.movetype == MOVETYPE_BOUNCE {
            1.5
        } else {
            1.0
        };

        clip_velocity(
            ptr::addr_of!((*ent).v.velocity).cast::<c_float>(),
            trace.plane.normal.as_ptr(),
            ptr::addr_of_mut!((*ent).v.velocity).cast::<c_float>(),
            backoff,
        );

        // stop if on ground
        if trace.plane.normal[2] > MIN_WALK_NORMAL {
            let velocity = (*ent).v.velocity;
            let measure = if cvar_value(ptr::addr_of!(g::sv_gameplayfix_bouncedownslopes)) != 0.0 {
                dot(&trace.plane.normal, &velocity)
            } else {
                (*ent).v.velocity[2]
            };
            if (*ent).v.movetype != MOVETYPE_BOUNCE || measure < 60.0 {
                (*ent).v.flags = (as_int((*ent).v.flags) | FL_ONGROUND) as c_float;

                // SV_PushEntityTo() calls SV_LinkEdict (true) that could free trace.ent
                if !trace.ent.is_null() && !(*trace.ent).free {
                    (*ent).v.groundentity = edict_to_prog(c::qcvm.cast::<QcVm>(), trace.ent);
                }

                (*ent).v.velocity = m::VEC3_ORIGIN;
                (*ent).v.avelocity = m::VEC3_ORIGIN;
            }
        }

        // check for in water
        sv_check_water_transition(ent)
    }
}

// ---------------------------------------------------------------------------
// stepping movement (sv_phys.c:2240-2290)

/// `sv_phys.c:2244` `SV_Physics_Step`
unsafe fn sv_physics_step(ent: *mut Edict) -> Raise {
    // SAFETY: `ent` is a live edict; its `free` flag is re-tested after the
    // relink exactly as in C.
    unsafe {
        // freefall if not onground
        if as_int((*ent).v.flags) & (FL_ONGROUND | FL_FLY | FL_SWIM) == 0 {
            // COMPAT: ADR-010 -- `-0.1` is a double literal, so the product and
            // the comparison are performed in double.
            let hitsound = ((*ent).v.velocity[2] as f64) < sv_gravity_value() as f64 * -0.1;

            sv_add_gravity(ent);
            let raised = sv_check_velocity(ent);
            if raised != 0 {
                return raised;
            }
            let mut clip: c_int = 0;
            let raised = sv_fly_move(ent, host_frametime() as c_float, ptr::null_mut(), &mut clip);
            if raised != 0 {
                return raised;
            }
            let raised = crate::world::quake_rs_sv_link_edict(ent, true);
            if raised != 0 {
                return raised;
            }

            if (*ent).free {
                return 0;
            }

            sv_finish_gravity(ent);

            // just hit ground
            if as_int((*ent).v.flags) & FL_ONGROUND != 0 && hitsound {
                let raised = g::SvPhys_Glue_StartSound(
                    ent.cast::<c_void>(),
                    0,
                    SND_DLAND2.as_ptr(),
                    255,
                    1.0,
                );
                if raised != 0 {
                    return raised;
                }
            }
        }

        // regular thinking
        let mut alive = false;
        let raised = sv_run_think(ent, &mut alive);
        if raised != 0 {
            return raised;
        }
        if alive {
            return sv_check_water_transition(ent);
        }

        0
    }
}

// ---------------------------------------------------------------------------
// SV_Physics (sv_phys.c:2293-2462)

/// `sv_phys.c:2296` `SV_Physics_Alloc_Hook` -- tracks `ED_Alloc` during
/// `SV_Physics`.
///
/// # Safety
/// `ED_Alloc` calls the hook as its last statement, after every `Host_Error`
/// it can reach, so no longjmp ever unwinds this frame (ADR-009).
unsafe extern "C" fn sv_physics_alloc_hook(e: *mut c_void) {
    // track the newly allocated edicts in order to add them into the pushable_ent_cache.
    // this is OK because by construction free edicts cannot be reused immediatly,
    // so e is garanteed not to be in pushable_ent_cache already.
    // since they are just allocated, they have a blank state so we add all of them
    // to pushable_ent_cache regardless, and the pushable test will be made later on in SV_PushMove in any case.
    // SAFETY: `NUM_PUSHABLE_ENT_CACHE` is bounded by `MAX_EDICTS`, the arena's
    // own limit, exactly as in C.
    unsafe {
        *ptr::addr_of_mut!(PUSHABLE_ENT_CACHE)
            .cast::<*mut Edict>()
            .add(NUM_PUSHABLE_ENT_CACHE as usize) = e.cast::<Edict>();
        NUM_PUSHABLE_ENT_CACHE += 1;
    }
}

/// `sv_phys.c:2298` `SV_Physics`.
///
/// # Safety
/// The ambient qcvm must be loaded (ADR-008).
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_physics() -> Raise {
    // SAFETY: ADR-008 ambient qcvm; every dispatch runs under a guard and the
    // qcvm is re-resolved afterwards (ADR-006).
    unsafe {
        let mut previous_alloc_hook: g::EdAllocHookFunc = None;

        let vm = c::qcvm.cast::<QcVm>();
        let physics_mode: c_int = if !(*vm).extglobals.physics_mode.is_null() {
            (*(*vm).extglobals.physics_mode) as c_int
        } else if w::World_Glue_QcvmIsClient() != 0 {
            // csqc doesn't run thinks by default. it was meant to simplify implementations, but we just force fields to
            // match ssqc so its not that large a burden.
            0
        } else {
            2
        };

        if qcvm_is_server() && physics_mode != 0 {
            sv_begin_pusher_support_frame();
        }

        if physics_mode == 0 {
            (*vm).time += host_frametime();
            return 0;
        } else if physics_mode == 1 {
            // for dp compat. note that this violates MOVETYPE_PUSH.
            let mut ent = (*vm).edicts;
            let mut i: c_int = 0;
            while i < (*c::qcvm.cast::<QcVm>()).num_edicts {
                if !(*ent).free {
                    let mut alive = false;
                    let raised = sv_run_think(ent, &mut alive);
                    if raised != 0 {
                        return raised;
                    }
                }
                i += 1;
                ent = next_edict(c::qcvm.cast::<QcVm>(), ent);
            }
            let vm = c::qcvm.cast::<QcVm>();
            (*vm).time += host_frametime();
            return 0;
        }

        // let the progs know that a new frame has started
        let vm = c::qcvm.cast::<QcVm>();
        if (*globals(vm)).StartFrame != 0 {
            let raised = g::SvPhys_Glue_CallStartFrame((*vm).time as c_float);
            if raised != 0 {
                return raised;
            }
        }

        // SV_CheckAllEnts ();

        //
        // treat each object in turn
        //
        let vm = c::qcvm.cast::<QcVm>();
        let mut ent = (*vm).edicts;

        let entity_cap: c_int =
            if cvar_value(ptr::addr_of!(g::sv_freezenonclients)) != 0.0 && qcvm_is_server() {
                g::SvPhys_Glue_MaxClients() + 1 // Only run physics on clients and the world
            } else {
                (*vm).num_edicts
            };

        // QC can flip the cvars mid-tick, the whole tick must use one consistent decision
        let fast_pushers = cvar_value(ptr::addr_of!(g::sv_fastpushmove)) > 0.0f32;
        let use_push_grid = fast_pushers && cvar_value(ptr::addr_of!(g::sv_pushgrid)) > 0.0f32;
        ptr::addr_of_mut!(g::sv_analyticphysics_frame)
            .write(cvar_value(ptr::addr_of!(g::sv_analyticphysics)) > 0.0f32);

        // fill the pushable entities cache and the spatial grid over it
        if fast_pushers {
            let mut build_start: f64 = 0.0;
            if sv_speeds_on() && qcvm_is_server() {
                build_start = c::Sys_DoubleTime();
            }

            NUM_PUSHABLE_ENT_CACHE = 0;
            if use_push_grid {
                push_grid_clear();
            }
            // beware, we skip entity 0 here:
            let mut check = next_edict(vm, (*vm).edicts);
            let mut e: c_int = 1;
            while e < (*vm).num_edicts {
                if !(*check).free && sv_is_pushable(check) {
                    *ptr::addr_of_mut!(PUSHABLE_ENT_CACHE)
                        .cast::<*mut Edict>()
                        .add(NUM_PUSHABLE_ENT_CACHE as usize) = check;
                    NUM_PUSHABLE_ENT_CACHE += 1;
                    if use_push_grid {
                        push_grid_insert(check);
                    }
                }
                e += 1;
                check = next_edict(vm, check);
            }
            PUSH_GRID_TAIL_START = NUM_PUSHABLE_ENT_CACHE;
            PUSH_CACHE_ACTIVE = true;
            if use_push_grid {
                PUSH_GRID_QCVM = vm;
                PUSH_GRID_ACTIVE = true;
            }

            if sv_speeds_on() && qcvm_is_server() {
                let ms = ptr::addr_of_mut!(g::sv_speeds_build_ms);
                *ms += (c::Sys_DoubleTime() - build_start) * 1000.0;
                let pushables = ptr::addr_of_mut!(g::sv_speeds_pushables);
                *pushables += NUM_PUSHABLE_ENT_CACHE;
                let entries = ptr::addr_of_mut!(g::sv_speeds_grid_entries);
                *entries += PUSH_GRID_NUM_ENTRIES;
            }

            previous_alloc_hook = g::ED_AllocSetHook(Some(sv_physics_alloc_hook));
        }

        // for (i=0 ; i<sv.num_edicts ; i++, ent = NEXT_EDICT(ent))
        let mut i: c_int = 0;
        while i < entity_cap {
            let vm = c::qcvm.cast::<QcVm>();
            let mut skip = (*ent).free;

            if !skip && (*globals(vm)).force_retouch != 0.0 {
                // force retouch even for stationary
                let raised = crate::world::quake_rs_sv_link_edict(ent, true);
                if raised != 0 {
                    return raised;
                }

                skip = (*ent).free;
            }

            if !skip {
                if i > 0 && i <= g::SvPhys_Glue_MaxClients() && qcvm_is_server() {
                    let raised = sv_physics_client(ent, i);
                    if raised != 0 {
                        return raised;
                    }
                } else if (*ent).v.movetype == MOVETYPE_PUSH {
                    let raised = sv_physics_pusher(ent);
                    if raised != 0 {
                        return raised;
                    }
                } else if (*ent).v.movetype == MOVETYPE_NONE {
                    let raised = sv_physics_none(ent);
                    if raised != 0 {
                        return raised;
                    }
                } else if (*ent).v.movetype == MOVETYPE_NOCLIP {
                    let raised = sv_physics_noclip(ent);
                    if raised != 0 {
                        return raised;
                    }
                } else if (*ent).v.movetype == MOVETYPE_STEP {
                    let raised = sv_physics_step(ent);
                    if raised != 0 {
                        return raised;
                    }
                } else if (*ent).v.movetype == MOVETYPE_TOSS
                    || (*ent).v.movetype == MOVETYPE_GIB
                    || (*ent).v.movetype == MOVETYPE_BOUNCE
                    || (*ent).v.movetype == MOVETYPE_FLY
                    || (*ent).v.movetype == MOVETYPE_FLYMISSILE
                {
                    let raised = sv_physics_toss(ent);
                    if raised != 0 {
                        return raised;
                    }
                } else {
                    let raised = g::SvPhys_Glue_EndGameBadMovetype(as_int((*ent).v.movetype));
                    if raised != 0 {
                        return raised;
                    }
                }

                // johnfitz -- PROTOCOL_FITZQUAKE
                // capture interval to nextthink here and send it to client for better
                // lerp timing; ~0.1 intervals match what the client assumes but thinks
                // fire quantized to server ticks, so the exact value still improves
                // lerp timing where the extra bytes are affordable
                (*ent).sendinterval = false;
                (*ent).sendinterval_default = false;
                let vm = c::qcvm.cast::<QcVm>();
                if !(*ent).free
                    && (*ent).v.nextthink as f64 > (*vm).time
                    && ((*ent).v.movetype == MOVETYPE_STEP
                        || (*ent).v.movetype == MOVETYPE_WALK
                        || (*ent).v.frame != (*ent).oldframe)
                {
                    let j = q_rint_f(((*ent).v.nextthink - (*ent).oldthinktime) * 255.0);
                    if j == 25 || j == 26 {
                        (*ent).sendinterval_default = true;
                    } else if (0..256).contains(&j) {
                        (*ent).sendinterval = true;
                    }
                }
                // johnfitz
            }

            i += 1;
            ent = next_edict(c::qcvm.cast::<QcVm>(), ent);
        }

        let vm = c::qcvm.cast::<QcVm>();
        if (*globals(vm)).force_retouch != 0.0 {
            (*globals(vm)).force_retouch -= 1.0;
        }

        if !(cvar_value(ptr::addr_of!(g::sv_freezenonclients)) != 0.0 && qcvm_is_server()) {
            (*vm).time += host_frametime();
        }

        if fast_pushers {
            PUSH_GRID_ACTIVE = false;
            PUSH_CACHE_ACTIVE = false;
            g::ED_AllocSetHook(previous_alloc_hook);
        }

        0
    }
}
