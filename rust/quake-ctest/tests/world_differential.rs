//! Differential test: the Rust `quake-capi` world port vs the original
//! `Quake/world.c` (compiled as `c_ref_*`). Rust migration Phase 7, M3.
//!
//! Both implementations run against ONE shared fixture (`ctest_world_*` in
//! `stubs/stubs.c`): an ambient qcvm with an edict arena and an areanode
//! array, a synthetic brush model with three real clipping hulls and a
//! node/leaf tree, a progs image whose touch functions are genuine bytecode
//! (`OP_CALL0` into a logging builtin), and stub bodies for every
//! `world_glue.c` seam -- `world_glue.c` is a Meson-only translation unit and
//! is not in `build.rs`'s `C_SOURCES`.
//!
//! Because the fixture state is global and mutable, every test follows the
//! `cvar_cmd_differential.rs` idiom: take the file mutex, then for each side
//! reset the fixture from scratch, drive the SAME call sequence through that
//! side's entry points, and snapshot everything observable (full `trace_t`
//! bit patterns, areanode topology, per-areanode link chain ORDER, touched
//! leafs, absmin/absmax, the touch-dispatch log and the console log). The
//! two snapshots must be identical.
//!
//! Phase 7 M4 removed the push-grid log this suite used to compare:
//! `Quake/sv_phys.c` joined `build.rs`'s `C_SOURCES`, so
//! `SV_PushGridEntityLinked` is now the real function under
//! `c_ref_SV_PushGridEntityLinked` and there is no interceptable seam left on
//! the C side. `sv_phys_differential.rs` covers the grid itself instead.
//!
//! Six entry points are raise-capable -- `SV_LinkEdict` (through
//! `SV_TouchLinks` -> `PR_ExecuteProgram`) and `SV_HullForEntity`,
//! `SV_ClipMoveToEntity`, `SV_Move`, `SV_TestEntityPosition`,
//! `SV_PointContentsAllBsps` (through `PR_GetString` in the two
//! `SV_HullForEntity` `Con_Warning` sites, plus `assert_always` for
//! `SV_Move`). Per ADR-009 the Rust side of each is a `quake_rs_*` core
//! returning a `Host_Guard` status, and the re-raise happens in a plain-named
//! C wrapper -- `Quake/world_glue.c` in the engine, `stubs.c` here. The tests
//! drive the plain names on both sides, so no longjmp ever unwinds a Rust
//! frame.

use core::cell::RefCell;
use core::ffi::{c_char, c_float, c_int, c_uint, c_void, CStr};
use std::sync::{Mutex, MutexGuard};

use proptest::prelude::*;
use proptest::test_runner::{Config, FileFailurePersistence, RngAlgorithm, TestRng, TestRunner};
use quake_ctest as _; // links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

// ---------------------------------------------------------------------------
// trace_t / rhtctx_s mirrors. world.h's trace_t has no quake-types mirror
// (it is world.c-local API), so the layout is asserted against the C
// compiler's own offsetof in `trace_layout_matches_the_rust_mirror` rather
// than assumed.

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Trace {
    allsolid: u8,
    startsolid: u8,
    inopen: u8,
    inwater: u8,
    fraction: f32,
    endpos: [f32; 3],
    plane_normal: [f32; 3],
    plane_dist: f32,
    ent: *mut c_void,
    contents: c_int,
}

/// Every field of a `trace_t` reduced to an exactly comparable form: floats
/// as raw bit patterns (so a NaN payload or a -0.0 divergence is caught) and
/// `ent` as an edict index (the two sides hand back their own pointers into
/// the same arena, but comparing the pointer directly would still be right --
/// the index is just readable).
#[derive(Clone, PartialEq, Eq, Debug)]
struct TraceBits {
    allsolid: u8,
    startsolid: u8,
    inopen: u8,
    inwater: u8,
    fraction: u32,
    endpos: [u32; 3],
    plane_normal: [u32; 3],
    plane_dist: u32,
    ent: i32,
    contents: i32,
}

impl Trace {
    fn bits(&self) -> TraceBits {
        TraceBits {
            allsolid: self.allsolid,
            startsolid: self.startsolid,
            inopen: self.inopen,
            inwater: self.inwater,
            fraction: self.fraction.to_bits(),
            endpos: [
                self.endpos[0].to_bits(),
                self.endpos[1].to_bits(),
                self.endpos[2].to_bits(),
            ],
            plane_normal: [
                self.plane_normal[0].to_bits(),
                self.plane_normal[1].to_bits(),
                self.plane_normal[2].to_bits(),
            ],
            plane_dist: self.plane_dist.to_bits(),
            // SAFETY: `ent` is either NULL or a pointer into the fixture's
            // live edict arena; the helper range-checks it either way.
            ent: unsafe { ctest_world_edict_index(self.ent) },
            contents: self.contents,
        }
    }
}

#[repr(C)]
struct RhtCtx {
    hitcontents: c_uint,
    start: [f32; 3],
    end: [f32; 3],
    clipnodes: *mut c_void,
    planes: *mut c_void,
}

// ---------------------------------------------------------------------------
// Harness fixture + oracle + port declarations.

extern "C" {
    // fixture (stubs/stubs.c)
    /// `vm_kind`: 0 standalone server VM, 1 `cl.qcvm`, 2 `sv.qcvm`.
    fn ctest_world_reset(vm_kind: c_int, num_edicts: c_int);
    fn ctest_world_set_cvars(hullcheck: c_float, areanode: c_float, checkext: c_float);
    fn ctest_world_edict(num: c_int) -> *mut c_void;
    fn ctest_world_hull(hullnum: c_int) -> *mut c_void;
    fn ctest_world_edict_index(p: *const c_void) -> c_int;
    fn ctest_world_areanode_index(p: *const c_void) -> c_int;
    fn ctest_world_numareanodes() -> c_int;
    fn ctest_world_reset_areanodes();
    fn ctest_world_world_bounds(out6: *mut c_float);
    fn ctest_world_nodes_root() -> *mut c_void;
    fn ctest_world_hull_arrays(
        hull: *const c_void,
        clipnodes: *mut *mut c_void,
        planes: *mut *mut c_void,
    );
    fn ctest_world_rhtctx_size() -> c_int;
    fn ctest_world_trace_layout(out: *mut c_int, max: c_int) -> c_int;
    #[allow(clippy::too_many_arguments)]
    fn ctest_world_edict_set(
        num: c_int,
        solid: c_float,
        movetype: c_float,
        modelindex: c_float,
        origin: *const c_float,
        mins: *const c_float,
        maxs: *const c_float,
        angles: *const c_float,
        flags: c_float,
        touch_kind: c_int,
        skin: c_float,
        owner: c_int,
        is_free: c_int,
    );
    fn ctest_world_edict_absbox(num: c_int, out6: *mut c_float);
    fn ctest_world_edict_leafs(num: c_int, out: *mut c_int, max: c_int) -> c_int;
    fn ctest_world_edict_is_free(num: c_int) -> c_int;
    fn ctest_world_snapshot_areanodes(out: *mut c_int, max: c_int) -> c_int;
    fn ctest_world_snapshot_links(out: *mut c_int, max: c_int) -> c_int;
    fn ctest_world_snapshot_hull(hull: *const c_void, out: *mut c_float, max: c_int) -> c_int;
    fn ctest_world_touch_log_len() -> c_int;
    fn ctest_world_touch_log_get(
        i: c_int,
        s: *mut c_int,
        o: *mut c_int,
        t: *mut c_float,
        k: *mut c_int,
    ) -> c_int;
    fn ctest_world_touch_log_clear();
    fn ctest_world_set_relink_target(num: c_int);
    fn ctest_world_set_free_target(num: c_int);
    fn ctest_world_set_link_fns(
        link: Option<extern "C" fn(*mut c_void, u8)>,
        unlink: Option<extern "C" fn(*mut c_void)>,
    );
    fn ctest_world_cl_set_num_entities(n: c_int);
    fn ctest_world_cl_set_entity(
        i: c_int,
        modelindex: c_int,
        solidsize: c_uint,
        origin: *const c_float,
        angles: *const c_float,
        skinnum: c_int,
    );
    fn ctest_clear_con_log();
    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(i: c_int) -> *const c_char;
    /// `stubs.c`'s plain-named ADR-009 wrapper over `quake_rs_sv_link_edict`,
    /// mirroring the one `Quake/world_glue.c` ships. Driving it rather than
    /// core-plus-re-raise keeps every longjmp inside C frames.
    fn SV_LinkEdict(ent: *mut c_void, touch_triggers: u8);
    /// Installs the Rust-backed link/unlink pair from C, so the fixture's
    /// re-entrant hook never holds a Rust function pointer that a raise
    /// would unwind through.
    fn ctest_world_set_rust_link_fns();
    /// Installs an all-NULL knownstrings table and points edict `num`'s
    /// classname at slot 0, which is exactly the input PR_GetString raises
    /// Host_Error on (Quake/pr_edict_arena.c:311-318).
    fn ctest_world_arm_bad_classname(num: c_int);
    fn ctest_try_host(f: extern "C" fn(*mut c_void), arg: *mut c_void) -> c_int;
    fn ctest_host_error_message() -> *const c_char;

    // oracle (Quake/world.c, renamed by include/c_ref_prelude.h)
    fn c_ref_SV_InitBoxHull();
    fn c_ref_SV_HullForBox(mins: *const c_float, maxs: *const c_float) -> *mut c_void;
    fn c_ref_SV_HullForEntity(
        ent: *mut c_void,
        mins: *const c_float,
        maxs: *const c_float,
        offset: *mut c_float,
    ) -> *mut c_void;
    fn c_ref_SV_ClearWorld();
    fn c_ref_SV_LinkEdict(ent: *mut c_void, touch_triggers: u8);
    fn c_ref_SV_UnlinkEdict(ent: *mut c_void);
    fn c_ref_SV_FindTouchedLeafs(ent: *mut c_void, node: *mut c_void);
    fn c_ref_SV_HullPointContents(hull: *mut c_void, num: c_int, p: *const c_float) -> c_int;
    fn c_ref_SV_PointContents(p: *const c_float) -> c_int;
    fn c_ref_SV_TruePointContents(p: *const c_float) -> c_int;
    fn c_ref_SV_PointContentsAllBsps(p: *const c_float, forent: *mut c_void) -> c_int;
    fn c_ref_SV_TestEntityPosition(ent: *mut c_void) -> *mut c_void;
    fn c_ref_Q1BSP_RecursiveHullTrace(
        ctx: *mut RhtCtx,
        num: c_int,
        p1f: c_float,
        p2f: c_float,
        p1: *const c_float,
        p2: *const c_float,
        trace: *mut Trace,
    ) -> c_int;
    fn c_ref_SV_RecursiveHullCheck(
        hull: *mut c_void,
        p1: *const c_float,
        p2: *const c_float,
        trace: *mut Trace,
        hitcontents: c_uint,
    ) -> u8;
    fn c_ref_SV_ClipMoveToEntity(
        ent: *mut c_void,
        start: *const c_float,
        mins: *const c_float,
        maxs: *const c_float,
        end: *const c_float,
        hitcontents: c_uint,
    ) -> Trace;
    fn c_ref_SV_MoveBounds(
        start: *const c_float,
        mins: *const c_float,
        maxs: *const c_float,
        end: *const c_float,
        boxmins: *mut c_float,
        boxmaxs: *mut c_float,
    );
    fn c_ref_SV_Move(
        start: *const c_float,
        mins: *const c_float,
        maxs: *const c_float,
        end: *const c_float,
        move_type: c_int,
        passedict: *mut c_void,
    ) -> Trace;
    fn c_ref_SV_CreateAreaNode(
        depth: c_int,
        mins: *const c_float,
        maxs: *const c_float,
    ) -> *mut c_void;

    // port (rust/quake-capi/src/world.rs, plain names per the M3 contract)
    fn SV_InitBoxHull();
    fn SV_HullForBox(mins: *const c_float, maxs: *const c_float) -> *mut c_void;
    fn SV_HullForEntity(
        ent: *mut c_void,
        mins: *const c_float,
        maxs: *const c_float,
        offset: *mut c_float,
    ) -> *mut c_void;
    fn SV_ClearWorld();
    fn SV_UnlinkEdict(ent: *mut c_void);
    fn SV_FindTouchedLeafs(ent: *mut c_void, node: *mut c_void);
    fn SV_HullPointContents(hull: *mut c_void, num: c_int, p: *const c_float) -> c_int;
    fn SV_PointContents(p: *const c_float) -> c_int;
    fn SV_TruePointContents(p: *const c_float) -> c_int;
    fn SV_PointContentsAllBsps(p: *const c_float, forent: *mut c_void) -> c_int;
    fn SV_TestEntityPosition(ent: *mut c_void) -> *mut c_void;
    fn Q1BSP_RecursiveHullTrace(
        ctx: *mut RhtCtx,
        num: c_int,
        p1f: c_float,
        p2f: c_float,
        p1: *const c_float,
        p2: *const c_float,
        trace: *mut Trace,
    ) -> c_int;
    fn SV_RecursiveHullCheck(
        hull: *mut c_void,
        p1: *const c_float,
        p2: *const c_float,
        trace: *mut Trace,
        hitcontents: c_uint,
    ) -> u8;
    fn SV_ClipMoveToEntity(
        ent: *mut c_void,
        start: *const c_float,
        mins: *const c_float,
        maxs: *const c_float,
        end: *const c_float,
        hitcontents: c_uint,
    ) -> Trace;
    fn SV_MoveBounds(
        start: *const c_float,
        mins: *const c_float,
        maxs: *const c_float,
        end: *const c_float,
        boxmins: *mut c_float,
        boxmaxs: *mut c_float,
    );
    fn SV_Move(
        start: *const c_float,
        mins: *const c_float,
        maxs: *const c_float,
        end: *const c_float,
        move_type: c_int,
        passedict: *mut c_void,
    ) -> Trace;
    fn SV_CreateAreaNode(depth: c_int, mins: *const c_float, maxs: *const c_float) -> *mut c_void;
}

// world.h / server.h constants the tests drive.
const CONTENTMASK_ANYSOLID: c_uint = (1u32 << 2) | (1u32 << 8); // SOLID | CLIP
const MOVE_NORMAL: c_int = 0;
const MOVE_NOMONSTERS: c_int = 1;
const MOVE_MISSILE: c_int = 2;
const MOVE_HITALLCONTENTS: c_int = 1 << 9;

const SOLID_NOT: f32 = 0.0;
const SOLID_TRIGGER: f32 = 1.0;
const SOLID_BBOX: f32 = 2.0;
const SOLID_SLIDEBOX: f32 = 3.0;
const SOLID_BSP: f32 = 4.0;

const MOVETYPE_NONE: f32 = 0.0;
const MOVETYPE_PUSH: f32 = 7.0;

const FL_ITEM: f32 = 256.0;

const TOUCH_LOG: c_int = 0;
const TOUCH_RELINK: c_int = 1;
const TOUCH_FREE: c_int = 2;
const TOUCH_FREE_OTHER: c_int = 3;

// ---------------------------------------------------------------------------
// Side dispatch. Each wrapper is a safe fn with the unsafe block (and its
// SAFETY note) inside, so the test bodies below stay unsafe-free.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    C,
    Rust,
}

impl Side {
    /// Points the fixture's re-entrant link/unlink hook (the one a touch
    /// handler calls) at this side's implementation.
    fn install_link_fns(self) {
        // SAFETY: plain C setter over two function pointers.
        unsafe {
            match self {
                Side::C => ctest_world_set_link_fns(None, None),
                Side::Rust => ctest_world_set_rust_link_fns(),
            }
        }
    }

    fn init_box_hull(self) {
        // SAFETY: no arguments; both sides write only their own static hull.
        unsafe {
            match self {
                Side::C => c_ref_SV_InitBoxHull(),
                Side::Rust => SV_InitBoxHull(),
            }
        }
    }

    fn clear_world(self) {
        // SAFETY: the fixture published a qcvm with a worldmodel before this.
        unsafe {
            match self {
                Side::C => c_ref_SV_ClearWorld(),
                Side::Rust => SV_ClearWorld(),
            }
        }
    }

    fn link_edict(self, num: c_int, touch_triggers: bool) {
        let tt = u8::from(touch_triggers);
        // SAFETY: `num` indexes the fixture arena. Both arms are plain C
        // entry points, so a raise unwinds no Rust frame (ADR-009).
        unsafe {
            let ent = ctest_world_edict(num);
            match self {
                Side::C => c_ref_SV_LinkEdict(ent, tt),
                Side::Rust => SV_LinkEdict(ent, tt),
            }
        }
    }

    fn unlink_edict(self, num: c_int) {
        // SAFETY: `num` indexes the fixture arena.
        unsafe {
            let ent = ctest_world_edict(num);
            match self {
                Side::C => c_ref_SV_UnlinkEdict(ent),
                Side::Rust => SV_UnlinkEdict(ent),
            }
        }
    }

    fn find_touched_leafs(self, num: c_int) {
        // SAFETY: the node root is the fixture model's own tree.
        unsafe {
            let ent = ctest_world_edict(num);
            let root = ctest_world_nodes_root();
            match self {
                Side::C => c_ref_SV_FindTouchedLeafs(ent, root),
                Side::Rust => SV_FindTouchedLeafs(ent, root),
            }
        }
    }

    fn hull_point_contents(self, hull: *mut c_void, num: c_int, p: &[f32; 3]) -> c_int {
        // SAFETY: `hull` is a live hull_t, `p` a live 3-float array.
        unsafe {
            match self {
                Side::C => c_ref_SV_HullPointContents(hull, num, p.as_ptr()),
                Side::Rust => SV_HullPointContents(hull, num, p.as_ptr()),
            }
        }
    }

    fn point_contents(self, p: &[f32; 3]) -> c_int {
        // SAFETY: `p` is a live 3-float array.
        unsafe {
            match self {
                Side::C => c_ref_SV_PointContents(p.as_ptr()),
                Side::Rust => SV_PointContents(p.as_ptr()),
            }
        }
    }

    fn true_point_contents(self, p: &[f32; 3]) -> c_int {
        // SAFETY: `p` is a live 3-float array.
        unsafe {
            match self {
                Side::C => c_ref_SV_TruePointContents(p.as_ptr()),
                Side::Rust => SV_TruePointContents(p.as_ptr()),
            }
        }
    }

    fn point_contents_all_bsps(self, p: &[f32; 3], forent: Option<c_int>) -> c_int {
        // SAFETY: `p` is live; `forent` indexes the fixture arena or is NULL.
        unsafe {
            let e = match forent {
                Some(n) => ctest_world_edict(n),
                None => core::ptr::null_mut(),
            };
            match self {
                Side::C => c_ref_SV_PointContentsAllBsps(p.as_ptr(), e),
                Side::Rust => SV_PointContentsAllBsps(p.as_ptr(), e),
            }
        }
    }

    fn test_entity_position(self, num: c_int) -> c_int {
        // SAFETY: `num` indexes the fixture arena; the returned pointer is
        // range-checked back to an index by the helper.
        unsafe {
            let ent = ctest_world_edict(num);
            let r = match self {
                Side::C => c_ref_SV_TestEntityPosition(ent),
                Side::Rust => SV_TestEntityPosition(ent),
            };
            ctest_world_edict_index(r)
        }
    }

    fn hull_for_box(self, mins: &[f32; 3], maxs: &[f32; 3]) -> Vec<u32> {
        // SAFETY: both arrays are live; the returned hull belongs to the side
        // that produced it and is only serialized, never stored.
        unsafe {
            let h = match self {
                Side::C => c_ref_SV_HullForBox(mins.as_ptr(), maxs.as_ptr()),
                Side::Rust => SV_HullForBox(mins.as_ptr(), maxs.as_ptr()),
            };
            serialize_hull(h)
        }
    }

    fn hull_for_entity(self, num: c_int, mins: &[f32; 3], maxs: &[f32; 3]) -> (Vec<u32>, [u32; 3]) {
        let mut offset = [0f32; 3];
        // SAFETY: `num` indexes the fixture arena; `offset` is a live 3-float
        // out-parameter, exactly what SV_HullForEntity writes.
        unsafe {
            let ent = ctest_world_edict(num);
            let h = match self {
                Side::C => {
                    c_ref_SV_HullForEntity(ent, mins.as_ptr(), maxs.as_ptr(), offset.as_mut_ptr())
                }
                Side::Rust => {
                    SV_HullForEntity(ent, mins.as_ptr(), maxs.as_ptr(), offset.as_mut_ptr())
                }
            };
            (
                serialize_hull(h),
                [
                    offset[0].to_bits(),
                    offset[1].to_bits(),
                    offset[2].to_bits(),
                ],
            )
        }
    }

    fn recursive_hull_check(
        self,
        hull: *mut c_void,
        p1: &[f32; 3],
        p2: &[f32; 3],
        hitcontents: c_uint,
    ) -> (u8, TraceBits) {
        // world.c's callers hand SV_RecursiveHullCheck a trace pre-seeded by
        // SV_ClipMoveToEntity (fraction 1, allsolid true, endpos = end), and
        // the routine relies on that seed. Reproduce it exactly.
        let mut trace = Trace {
            allsolid: 1,
            fraction: 1.0,
            endpos: *p2,
            ..Default::default()
        };
        // SAFETY: `hull` is a live hull_t belonging to the fixture model (or
        // the calling side's box hull); the point arrays and `trace` are live
        // locals for the duration of the call.
        let r = unsafe {
            match self {
                Side::C => c_ref_SV_RecursiveHullCheck(
                    hull,
                    p1.as_ptr(),
                    p2.as_ptr(),
                    &mut trace,
                    hitcontents,
                ),
                Side::Rust => {
                    SV_RecursiveHullCheck(hull, p1.as_ptr(), p2.as_ptr(), &mut trace, hitcontents)
                }
            }
        };
        (r, trace.bits())
    }

    #[allow(clippy::too_many_arguments)] // mirrors Q1BSP_RecursiveHullTrace's C signature
    fn q1bsp_hull_trace(
        self,
        hull: *mut c_void,
        num: c_int,
        p1f: f32,
        p2f: f32,
        p1: &[f32; 3],
        p2: &[f32; 3],
        hitcontents: c_uint,
    ) -> (c_int, TraceBits) {
        let mut clipnodes = core::ptr::null_mut();
        let mut planes = core::ptr::null_mut();
        // SAFETY: `hull` is a live hull_t; the helper only reads two members.
        unsafe { ctest_world_hull_arrays(hull, &mut clipnodes, &mut planes) };
        let mut ctx = RhtCtx {
            hitcontents,
            start: *p1,
            end: *p2,
            clipnodes,
            planes,
        };
        let mut trace = Trace {
            allsolid: 1,
            fraction: 1.0,
            endpos: *p2,
            ..Default::default()
        };
        // SAFETY: `ctx` mirrors struct rhtctx_s (size asserted in
        // `trace_layout_matches_the_rust_mirror`) and all pointers are live.
        let r = unsafe {
            match self {
                Side::C => c_ref_Q1BSP_RecursiveHullTrace(
                    &mut ctx,
                    num,
                    p1f,
                    p2f,
                    p1.as_ptr(),
                    p2.as_ptr(),
                    &mut trace,
                ),
                Side::Rust => Q1BSP_RecursiveHullTrace(
                    &mut ctx,
                    num,
                    p1f,
                    p2f,
                    p1.as_ptr(),
                    p2.as_ptr(),
                    &mut trace,
                ),
            }
        };
        (r, trace.bits())
    }

    fn clip_move_to_entity(
        self,
        num: c_int,
        start: &[f32; 3],
        mins: &[f32; 3],
        maxs: &[f32; 3],
        end: &[f32; 3],
        hitcontents: c_uint,
    ) -> TraceBits {
        // SAFETY: `num` indexes the fixture arena; every vector is live.
        unsafe {
            let ent = ctest_world_edict(num);
            let t = match self {
                Side::C => c_ref_SV_ClipMoveToEntity(
                    ent,
                    start.as_ptr(),
                    mins.as_ptr(),
                    maxs.as_ptr(),
                    end.as_ptr(),
                    hitcontents,
                ),
                Side::Rust => SV_ClipMoveToEntity(
                    ent,
                    start.as_ptr(),
                    mins.as_ptr(),
                    maxs.as_ptr(),
                    end.as_ptr(),
                    hitcontents,
                ),
            };
            t.bits()
        }
    }

    fn move_bounds(
        self,
        start: &[f32; 3],
        mins: &[f32; 3],
        maxs: &[f32; 3],
        end: &[f32; 3],
    ) -> [u32; 6] {
        let mut boxmins = [0f32; 3];
        let mut boxmaxs = [0f32; 3];
        // SAFETY: four live inputs and two live 3-float out-parameters.
        unsafe {
            match self {
                Side::C => c_ref_SV_MoveBounds(
                    start.as_ptr(),
                    mins.as_ptr(),
                    maxs.as_ptr(),
                    end.as_ptr(),
                    boxmins.as_mut_ptr(),
                    boxmaxs.as_mut_ptr(),
                ),
                Side::Rust => SV_MoveBounds(
                    start.as_ptr(),
                    mins.as_ptr(),
                    maxs.as_ptr(),
                    end.as_ptr(),
                    boxmins.as_mut_ptr(),
                    boxmaxs.as_mut_ptr(),
                ),
            }
        }
        [
            boxmins[0].to_bits(),
            boxmins[1].to_bits(),
            boxmins[2].to_bits(),
            boxmaxs[0].to_bits(),
            boxmaxs[1].to_bits(),
            boxmaxs[2].to_bits(),
        ]
    }

    fn sv_move(
        self,
        start: &[f32; 3],
        mins: &[f32; 3],
        maxs: &[f32; 3],
        end: &[f32; 3],
        move_type: c_int,
        passedict: Option<c_int>,
    ) -> TraceBits {
        // SAFETY: every vector is a live local; `passedict` indexes the
        // fixture arena when present.
        unsafe {
            let pass = match passedict {
                Some(n) => ctest_world_edict(n),
                None => core::ptr::null_mut(),
            };
            let t = match self {
                Side::C => c_ref_SV_Move(
                    start.as_ptr(),
                    mins.as_ptr(),
                    maxs.as_ptr(),
                    end.as_ptr(),
                    move_type,
                    pass,
                ),
                Side::Rust => SV_Move(
                    start.as_ptr(),
                    mins.as_ptr(),
                    maxs.as_ptr(),
                    end.as_ptr(),
                    move_type,
                    pass,
                ),
            };
            t.bits()
        }
    }

    fn create_area_node(self, depth: c_int, mins: &[f32; 3], maxs: &[f32; 3]) -> c_int {
        // SAFETY: both vectors are live; the routine appends into the
        // fixture qcvm's areanode array, which the caller reset first.
        unsafe {
            let n = match self {
                Side::C => c_ref_SV_CreateAreaNode(depth, mins.as_ptr(), maxs.as_ptr()),
                Side::Rust => SV_CreateAreaNode(depth, mins.as_ptr(), maxs.as_ptr()),
            };
            ctest_world_areanode_index(n)
        }
    }
}

// ---------------------------------------------------------------------------
// Fixture helpers.

fn serialize_hull(h: *mut c_void) -> Vec<u32> {
    let mut buf = vec![0f32; 8 + 9 * 64];
    let len = buf.len() as c_int;
    // SAFETY: `h` is a live hull_t; `buf` is sized for the fixture's largest
    // hull (30 clipnodes) and the helper honours the cap.
    let n = unsafe { ctest_world_snapshot_hull(h, buf.as_mut_ptr(), len) };
    assert!(n > 0, "hull serialization overflowed the buffer");
    buf[..n as usize].iter().map(|f| f.to_bits()).collect()
}

fn hull(n: c_int) -> *mut c_void {
    // SAFETY: 0..MAX_MAP_HULLS is in range for the fixture model.
    unsafe { ctest_world_hull(n) }
}

fn world_bounds() -> ([f32; 3], [f32; 3]) {
    let mut b = [0f32; 6];
    // SAFETY: `b` has room for the six floats the helper writes.
    unsafe { ctest_world_world_bounds(b.as_mut_ptr()) };
    ([b[0], b[1], b[2]], [b[3], b[4], b[5]])
}

fn snapshot_areanodes() -> Vec<i32> {
    // AREA_NODES is 1024 and each node contributes five ints.
    let mut buf = vec![0i32; 5 * 1024];
    let len = buf.len() as c_int;
    // SAFETY: `buf` is sized for the whole array and the helper honours the cap.
    let n = unsafe { ctest_world_snapshot_areanodes(buf.as_mut_ptr(), len) };
    buf.truncate(n as usize);
    buf
}

fn snapshot_links() -> Vec<i32> {
    let mut buf = vec![0i32; 4 * 1024];
    let len = buf.len() as c_int;
    // SAFETY: `buf` is sized above the fixture's edict count and the helper
    // honours the cap.
    let n = unsafe { ctest_world_snapshot_links(buf.as_mut_ptr(), len) };
    buf.truncate(n as usize);
    buf
}

/// absmin/absmax bit patterns, plus the touched-leaf list and free flag, for
/// every edict in the arena. This is the whole of SV_LinkEdict's per-edict
/// output.
fn snapshot_edicts(count: c_int) -> Vec<u32> {
    let mut out = Vec::new();
    for i in 0..count {
        let mut b = [0f32; 6];
        let mut leafs = [0i32; 32];
        // SAFETY: `i` is inside the arena; both buffers are correctly sized
        // and the leaf helper is given its true capacity.
        let n = unsafe {
            ctest_world_edict_absbox(i, b.as_mut_ptr());
            ctest_world_edict_leafs(i, leafs.as_mut_ptr(), 32)
        };
        out.extend(b.iter().map(|f| f.to_bits()));
        out.push(n as u32);
        out.extend(leafs[..(n.min(32)) as usize].iter().map(|v| *v as u32));
        // SAFETY: same index bound.
        out.push(unsafe { ctest_world_edict_is_free(i) } as u32);
    }
    out
}

fn touch_log() -> Vec<(i32, i32, u32, i32)> {
    // SAFETY: plain counter read.
    let n = unsafe { ctest_world_touch_log_len() };
    let mut out = Vec::new();
    for i in 0..n {
        let (mut s, mut o, mut k) = (0i32, 0i32, 0i32);
        let mut t = 0f32;
        // SAFETY: `i < n`; the four out-params are live locals.
        let ok = unsafe { ctest_world_touch_log_get(i, &mut s, &mut o, &mut t, &mut k) };
        assert_eq!(ok, 1, "touch log entry {i} was dropped (overflow)");
        out.push((s, o, t.to_bits(), k));
    }
    out
}

fn con_log() -> Vec<String> {
    // SAFETY: plain counter read.
    let n = unsafe { ctest_con_log_len() };
    (0..n)
        .map(|i| {
            // SAFETY: `i < n`, and the stub returns a NUL-terminated buffer
            // that outlives this borrow.
            unsafe { CStr::from_ptr(ctest_con_log_get(i)) }
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

#[derive(Clone, Copy)]
struct EdictSpec {
    num: c_int,
    solid: f32,
    movetype: f32,
    modelindex: f32,
    origin: [f32; 3],
    mins: [f32; 3],
    maxs: [f32; 3],
    angles: [f32; 3],
    flags: f32,
    touch_kind: c_int,
    skin: f32,
    owner: c_int,
}

impl EdictSpec {
    const fn new(num: c_int, solid: f32, origin: [f32; 3], half: f32) -> Self {
        EdictSpec {
            num,
            solid,
            movetype: MOVETYPE_NONE,
            modelindex: 0.0,
            origin,
            mins: [-half, -half, -half],
            maxs: [half, half, half],
            angles: [0.0, 0.0, 0.0],
            flags: 0.0,
            touch_kind: -1,
            skin: 0.0,
            owner: 0,
        }
    }

    fn touching(mut self, kind: c_int) -> Self {
        self.touch_kind = kind;
        self
    }

    fn brush(mut self, modelindex: f32, movetype: f32) -> Self {
        self.modelindex = modelindex;
        self.movetype = movetype;
        self
    }

    fn angled(mut self, angles: [f32; 3]) -> Self {
        self.angles = angles;
        self
    }

    fn boxed(mut self, mins: [f32; 3], maxs: [f32; 3]) -> Self {
        self.mins = mins;
        self.maxs = maxs;
        self
    }

    fn with_flags(mut self, flags: f32) -> Self {
        self.flags = flags;
        self
    }

    fn apply(&self) {
        // SAFETY: `num` indexes the fixture arena; every vector is a live
        // 3-float array for the duration of the call.
        unsafe {
            ctest_world_edict_set(
                self.num,
                self.solid,
                self.movetype,
                self.modelindex,
                self.origin.as_ptr(),
                self.mins.as_ptr(),
                self.maxs.as_ptr(),
                self.angles.as_ptr(),
                self.flags,
                self.touch_kind,
                self.skin,
                self.owner,
                0,
            );
        }
    }
}

#[derive(Clone, Copy)]
struct Cvars {
    hullcheck: f32,
    areanode: f32,
    checkext: f32,
}

const VANILLA: Cvars = Cvars {
    hullcheck: 0.0,
    areanode: 0.0,
    checkext: 1.0,
};
const FTE: Cvars = Cvars {
    hullcheck: 1.0,
    areanode: 1.0,
    checkext: 1.0,
};
/// pr_checkextension off: world.c takes the vanilla branch in every one of
/// its five gates regardless of the two cvars.
const NO_EXT: Cvars = Cvars {
    hullcheck: 1.0,
    areanode: 1.0,
    checkext: 0.0,
};

const ARENA: c_int = 48;

/// Rebuilds the whole fixture and brings `side` up to a linked world.
fn setup(side: Side, cv: Cvars, client_vm: bool, specs: &[EdictSpec], touch_triggers: bool) {
    // SAFETY: plain fixture setters; the file mutex serializes all callers.
    unsafe {
        ctest_world_reset(c_int::from(client_vm), ARENA);
        ctest_world_set_cvars(cv.hullcheck, cv.areanode, cv.checkext);
    }
    side.install_link_fns();
    side.init_box_hull();
    side.clear_world();
    for s in specs {
        s.apply();
        side.link_edict(s.num, touch_triggers);
    }
    // the console noise from setup is not what a test is asserting on
    // unless it says so; each test clears explicitly where it matters
}

/// Everything observable after one side has run: the body's own return
/// value plus all the global state world.c can touch.
struct Snap<T> {
    value: T,
    areanodes: Vec<i32>,
    links: Vec<i32>,
    edicts: Vec<u32>,
    touch: Vec<(i32, i32, u32, i32)>,
    con: Vec<String>,
}

fn capture<T>(value: T) -> Snap<T> {
    Snap {
        value,
        areanodes: snapshot_areanodes(),
        links: snapshot_links(),
        edicts: snapshot_edicts(ARENA),
        touch: touch_log(),
        con: con_log(),
    }
}

/// Runs `body` once per side against a freshly reset fixture, asserts the two
/// snapshots are identical, and hands the C side's snapshot back so the caller
/// can assert the scenario actually produced signal (a differential that
/// compares two empty logs proves nothing).
fn diff<T, F>(cv: Cvars, client_vm: bool, specs: &[EdictSpec], touch: bool, body: F) -> Snap<T>
where
    T: PartialEq + core::fmt::Debug,
    F: Fn(Side) -> T,
{
    setup(Side::C, cv, client_vm, specs, touch);
    // SAFETY: plain console-log reset.
    unsafe { ctest_clear_con_log() };
    let c = capture(body(Side::C));

    setup(Side::Rust, cv, client_vm, specs, touch);
    // SAFETY: plain console-log reset.
    unsafe { ctest_clear_con_log() };
    let rust = capture(body(Side::Rust));

    assert_eq!(c.value, rust.value, "return value");
    assert_eq!(c.areanodes, rust.areanodes, "areanode tree");
    assert_eq!(c.links, rust.links, "link chain order");
    assert_eq!(c.edicts, rust.edicts, "per-edict absbox / leafs / free");
    assert_eq!(c.touch, rust.touch, "touch dispatch log");
    assert_eq!(c.con, rust.con, "console log");
    c
}

// A representative population: triggers, bboxes, slideboxes, a brush mover,
// an FL_ITEM pickup and a couple of edicts far enough apart to land in
// different areanodes.
fn population() -> Vec<EdictSpec> {
    vec![
        EdictSpec::new(1, SOLID_TRIGGER, [0.0, 0.0, 0.0], 32.0),
        EdictSpec::new(2, SOLID_TRIGGER, [16.0, 0.0, 0.0], 24.0),
        EdictSpec::new(3, SOLID_BBOX, [-64.0, -64.0, -64.0], 16.0),
        EdictSpec::new(4, SOLID_SLIDEBOX, [64.0, 64.0, 0.0], 16.0),
        EdictSpec::new(5, SOLID_BSP, [0.0, 0.0, 0.0], 0.0)
            .boxed([-512.0, -512.0, -256.0], [512.0, 512.0, 256.0])
            .brush(1.0, MOVETYPE_PUSH),
        EdictSpec::new(6, SOLID_BBOX, [-200.0, 180.0, -180.0], 8.0).with_flags(FL_ITEM),
        EdictSpec::new(7, SOLID_TRIGGER, [900.0, 900.0, 0.0], 48.0),
        EdictSpec::new(8, SOLID_NOT, [0.0, 0.0, 100.0], 8.0),
        EdictSpec::new(9, SOLID_BBOX, [-900.0, 900.0, 0.0], 20.0),
        EdictSpec::new(10, SOLID_SLIDEBOX, [190.0, -190.0, -190.0], 12.0),
    ]
}

// ---------------------------------------------------------------------------
// ABI

#[test]
fn trace_layout_matches_the_rust_mirror() {
    let _g = lock();
    let mut lay = [0i32; 11];
    // SAFETY: `lay` has room for the 11 ints the helper writes.
    let n = unsafe { ctest_world_trace_layout(lay.as_mut_ptr(), 11) };
    assert_eq!(n, 11);
    assert_eq!(lay[0] as usize, core::mem::size_of::<Trace>());
    let want = [
        core::mem::offset_of!(Trace, allsolid),
        core::mem::offset_of!(Trace, startsolid),
        core::mem::offset_of!(Trace, inopen),
        core::mem::offset_of!(Trace, inwater),
        core::mem::offset_of!(Trace, fraction),
        core::mem::offset_of!(Trace, endpos),
        core::mem::offset_of!(Trace, plane_normal),
        core::mem::offset_of!(Trace, plane_dist),
        core::mem::offset_of!(Trace, ent),
        core::mem::offset_of!(Trace, contents),
    ];
    for (i, w) in want.iter().enumerate() {
        assert_eq!(lay[i + 1] as usize, *w, "trace_t offset {i}");
    }
    // SAFETY: plain sizeof read from the fixture.
    let rhtctx_size = unsafe { ctest_world_rhtctx_size() };
    assert_eq!(
        rhtctx_size as usize,
        core::mem::size_of::<RhtCtx>(),
        "struct rhtctx_s"
    );
}

// ---------------------------------------------------------------------------
// Point contents

/// A grid dense enough to land inside every synthetic volume, exactly on
/// several splitting planes, and outside the map.
fn contents_probe_points() -> Vec<[f32; 3]> {
    let mut pts = Vec::new();
    let coords = [
        -600.0f32, -448.0, -447.9, -256.0, -192.0, -160.0, -128.0, -64.0, -32.0, -0.0, 0.0, 31.9,
        32.0, 64.0, 96.0, 128.0, 192.0, 256.0, 448.0, 600.0,
    ];
    for x in coords {
        for y in [-256.0f32, -64.0, 0.0, 64.0, 192.0, 256.0] {
            for z in [-256.0f32, -192.0, -64.0, 0.0, 128.0, 192.0] {
                pts.push([x, y, z]);
            }
        }
    }
    pts
}

#[test]
fn hull_point_contents_matches() {
    let _g = lock();
    let pts = contents_probe_points();
    diff(VANILLA, false, &[], false, |side| {
        let mut out = Vec::new();
        for h in 0..3 {
            for p in &pts {
                out.push(side.hull_point_contents(hull(h), 0, p));
            }
            // start from an interior node too, not just the root
            for p in &pts {
                out.push(side.hull_point_contents(hull(h), 6, p));
            }
            // a negative `num` short-circuits straight to a contents value
            out.push(side.hull_point_contents(hull(h), -2, &[0.0, 0.0, 0.0]));
            out.push(side.hull_point_contents(hull(h), -1, &[0.0, 0.0, 0.0]));
        }
        out
    });
}

#[test]
fn point_contents_and_true_point_contents_match() {
    let _g = lock();
    let pts = contents_probe_points();
    let snap = diff(VANILLA, false, &[], false, |side| {
        pts.iter()
            .map(|p| (side.point_contents(p), side.true_point_contents(p)))
            .collect::<Vec<_>>()
    });
    // every synthetic volume has to be reachable, and the non-true call has
    // to remap CONTENTS_CURRENT_0 onto CONTENTS_WATER
    for want in [-1, -2, -3, -5, -9] {
        assert!(
            snap.value.iter().any(|(_, t)| *t == want),
            "contents {want} never observed"
        );
    }
    assert!(
        snap.value.iter().any(|(p, t)| *p == -3 && *t == -9),
        "the current -> water remap was never exercised"
    );
}

#[test]
fn point_contents_all_bsps_matches() {
    let _g = lock();
    let pts = contents_probe_points();
    // two overlapping SOLID_BSP entities plus a non-bsp SOLID_BSP, so both
    // the "skip forent" and the model-type warning branches are reached
    let specs = vec![
        EdictSpec::new(1, SOLID_BSP, [0.0, 0.0, 0.0], 0.0)
            .boxed([-512.0, -512.0, -256.0], [512.0, 512.0, 256.0])
            .brush(1.0, MOVETYPE_PUSH),
        EdictSpec::new(2, SOLID_BSP, [64.0, 0.0, 0.0], 0.0)
            .boxed([-256.0, -256.0, -128.0], [256.0, 256.0, 128.0])
            .brush(1.0, MOVETYPE_PUSH),
        EdictSpec::new(3, SOLID_BBOX, [0.0, 0.0, 0.0], 16.0),
    ];
    diff(FTE, false, &specs, false, |side| {
        let mut out = Vec::new();
        for p in &pts {
            out.push(side.point_contents_all_bsps(p, None));
            out.push(side.point_contents_all_bsps(p, Some(1)));
            out.push(side.point_contents_all_bsps(p, Some(2)));
            out.push(side.point_contents_all_bsps(p, Some(3)));
        }
        out
    });
}

// ---------------------------------------------------------------------------
// Hull selection

#[test]
fn hull_for_box_matches() {
    let _g = lock();
    let boxes: [([f32; 3], [f32; 3]); 5] = [
        ([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
        ([-16.0, -16.0, -24.0], [16.0, 16.0, 32.0]),
        ([-1.0, -2.0, -3.0], [4.0, 5.0, 6.0]),
        ([1.5, -2.5, 3.5], [-1.5, 2.5, -3.5]), // deliberately inverted
        ([-9999.0, -9999.0, -9999.0], [9999.0, 9999.0, 9999.0]),
    ];
    diff(VANILLA, false, &[], false, |side| {
        boxes
            .iter()
            .map(|(mins, maxs)| side.hull_for_box(mins, maxs))
            .collect::<Vec<_>>()
    });
}

#[test]
fn hull_for_entity_matches_every_solid_type() {
    let _g = lock();
    // 1: SOLID_BSP + MOVETYPE_PUSH on the brush model (the normal case)
    // 2: SOLID_BSP without MOVETYPE_PUSH -> Con_Warning under checkext 0
    // 3: SOLID_BSP with a non-brush model -> the nohitmeshsupport goto
    // 4: SOLID_BSP with NO model at all -> same goto, via GetModel == NULL
    // 5..8: the non-BSP solid types
    let specs = vec![
        EdictSpec::new(1, SOLID_BSP, [8.0, -8.0, 4.0], 0.0)
            .boxed([-512.0, -512.0, -256.0], [512.0, 512.0, 256.0])
            .brush(1.0, MOVETYPE_PUSH),
        EdictSpec::new(2, SOLID_BSP, [0.0, 0.0, 0.0], 0.0)
            .boxed([-64.0, -64.0, -64.0], [64.0, 64.0, 64.0])
            .brush(1.0, MOVETYPE_NONE),
        EdictSpec::new(3, SOLID_BSP, [1.0, 2.0, 3.0], 0.0)
            .boxed([-16.0, -16.0, -16.0], [16.0, 16.0, 16.0])
            .brush(2.0, MOVETYPE_PUSH),
        EdictSpec::new(4, SOLID_BSP, [-3.0, 5.0, 7.0], 0.0)
            .boxed([-16.0, -16.0, -16.0], [16.0, 16.0, 16.0])
            .brush(0.0, MOVETYPE_PUSH),
        EdictSpec::new(5, SOLID_NOT, [10.0, 20.0, 30.0], 8.0),
        EdictSpec::new(6, SOLID_TRIGGER, [-10.0, 20.0, -30.0], 12.0),
        EdictSpec::new(7, SOLID_BBOX, [4.0, -4.0, 4.0], 16.0),
        EdictSpec::new(8, SOLID_SLIDEBOX, [0.5, 0.25, -0.125], 24.0),
    ];
    // the three size buckets SV_HullForEntity selects on: <3, <=32, >32
    let sizes: [([f32; 3], [f32; 3]); 4] = [
        ([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
        ([-16.0, -16.0, -24.0], [16.0, 16.0, 32.0]),
        ([-16.0, -16.0, -24.0], [16.0, 16.0, 32.0]),
        ([-32.0, -32.0, -24.0], [32.0, 32.0, 64.0]),
    ];
    for cv in [VANILLA, FTE, NO_EXT] {
        diff(cv, false, &specs, false, |side| {
            let mut out = Vec::new();
            for spec in &specs {
                for (mins, maxs) in &sizes {
                    out.push(side.hull_for_entity(spec.num, mins, maxs));
                }
            }
            out
        });
    }
}

/// The two `Con_Warning` sites in `SV_HullForEntity` (world.c:145 and :152).
/// `hull_for_entity_matches_every_solid_type` already compares the console
/// log, but it never asserts the log is non-empty, so a port that simply
/// never warned would pass it. This one pins the text and the ordering.
#[test]
fn hull_for_entity_warn_paths_match() {
    let _g = lock();
    // 1: SOLID_BSP, movetype != MOVETYPE_PUSH, checkext 0 -> the no-push warn
    //    (and nothing else: modelindex 1 is the brush model)
    // 2: SOLID_BSP + MOVETYPE_PUSH on a non-brush model -> the non-bsp warn
    //    only, since the no-push gate does not fire
    // 3: both at once -- no-push warn, then non-bsp warn, in that order
    // 4: SOLID_BSP + MOVETYPE_PUSH with no model at all -> GetModel returns
    //    NULL, so the non-bsp warn fires off the !model half of the test
    let specs = vec![
        EdictSpec::new(1, SOLID_BSP, [1.0, 2.0, 3.0], 0.0)
            .boxed([-64.0, -64.0, -64.0], [64.0, 64.0, 64.0])
            .brush(1.0, MOVETYPE_NONE),
        EdictSpec::new(2, SOLID_BSP, [4.0, 5.0, 6.0], 0.0)
            .boxed([-16.0, -16.0, -16.0], [16.0, 16.0, 16.0])
            .brush(2.0, MOVETYPE_PUSH),
        EdictSpec::new(3, SOLID_BSP, [7.0, 8.0, 9.0], 0.0)
            .boxed([-16.0, -16.0, -16.0], [16.0, 16.0, 16.0])
            .brush(2.0, MOVETYPE_NONE),
        EdictSpec::new(4, SOLID_BSP, [-1.0, -2.0, -3.0], 0.0)
            .boxed([-16.0, -16.0, -16.0], [16.0, 16.0, 16.0])
            .brush(0.0, MOVETYPE_PUSH),
    ];
    let size = ([-16.0f32, -16.0, -24.0], [16.0f32, 16.0, 32.0]);

    // checkext 1 gates the no-push warn off; checkext 0 lets it through.
    // Running both proves the gate itself is ported, not just the message.
    for cv in [NO_EXT, FTE] {
        let snap = diff(cv, false, &specs, false, |side| {
            let mut out = Vec::new();
            for spec in &specs {
                out.push(side.hull_for_entity(spec.num, &size.0, &size.1));
            }
            out
        });

        let nopush: Vec<&String> = snap
            .con
            .iter()
            .filter(|l| l.contains("SOLID_BSP without MOVETYPE_PUSH"))
            .collect();
        let nonbsp: Vec<&String> = snap
            .con
            .iter()
            .filter(|l| l.contains("SOLID_BSP with a non bsp model"))
            .collect();

        // edicts 2 and 4 are MOVETYPE_PUSH, so only 1 and 3 can take the
        // no-push branch, and only when pr_checkextension is off
        assert_eq!(
            nopush.len(),
            if cv.checkext == 0.0 { 2 } else { 0 },
            "no-push warn count under checkext {}: {:?}",
            cv.checkext,
            snap.con
        );
        // edicts 2, 3 and 4 all reach the goto: two non-brush models and one
        // missing model. That half is not gated on pr_checkextension.
        assert_eq!(
            nonbsp.len(),
            3,
            "non-bsp warn count under checkext {}: {:?}",
            cv.checkext,
            snap.con
        );
        // the exact formatted origin, so a port that dropped or reordered the
        // three %f arguments is caught, not just one that dropped the call
        assert!(
            nonbsp[0].contains("4.000000 5.000000 6.000000"),
            "non-bsp warn should carry ent 2's origin: {:?}",
            nonbsp[0]
        );
        assert!(
            nonbsp.iter().all(|l| l.contains("ctest_ent")),
            "every warn resolves classname through PR_GetString: {nonbsp:?}"
        );
        if cv.checkext == 0.0 {
            // ordering: for edict 3 the no-push warn precedes the non-bsp one
            let i_nopush = snap
                .con
                .iter()
                .position(|l| l.contains("7.000000 8.000000 9.000000"))
                .expect("edict 3 no-push warn");
            let i_nonbsp = snap
                .con
                .iter()
                .rposition(|l| l.contains("7.000000 8.000000 9.000000"))
                .expect("edict 3 non-bsp warn");
            assert!(
                i_nopush < i_nonbsp,
                "warn order for edict 3: {:?}",
                snap.con
            );
            assert!(
                snap.con[i_nopush].contains("without MOVETYPE_PUSH"),
                "the first of edict 3's two warns is the no-push one"
            );
        }
    }
}

/// The reason those two warns had to become ADR-009 status cores: the
/// `PR_GetString (ent->v.classname)` argument reaches `Host_Error`
/// (Quake/pr_edict_arena.c:315). The C oracle longjmps straight out of
/// `SV_HullForEntity`; the Rust side has to catch it in `World_Glue_Warn*`,
/// propagate the status up through `quake_rs_sv_hull_for_entity` and re-raise
/// from the C wrapper. Both must land in the same trap with the same message,
/// and neither may return a hull.
#[test]
fn hull_for_entity_warn_raise_propagates() {
    let _g = lock();

    struct Args {
        side: Side,
        ent: *mut c_void,
        mins: [f32; 3],
        maxs: [f32; 3],
        offset: [f32; 3],
        hull: *mut c_void,
    }

    extern "C" fn call(p: *mut c_void) {
        // SAFETY: `p` is the `Args` the caller below pinned on its own stack
        // for the duration of ctest_try_host.
        let a = unsafe { &mut *p.cast::<Args>() };
        // SAFETY: `ent` came out of the fixture arena; `offset` is a live
        // 3-float out-parameter. Both sides raise before returning.
        a.hull = unsafe {
            match a.side {
                Side::C => c_ref_SV_HullForEntity(
                    a.ent,
                    a.mins.as_ptr(),
                    a.maxs.as_ptr(),
                    a.offset.as_mut_ptr(),
                ),
                Side::Rust => SV_HullForEntity(
                    a.ent,
                    a.mins.as_ptr(),
                    a.maxs.as_ptr(),
                    a.offset.as_mut_ptr(),
                ),
            }
        };
    }

    // Two entities, one per warn site, so both raise-capable call sites are
    // proven to propagate rather than only the first one.
    //   5: no-push warn (needs checkext 0)
    //   6: non-bsp-model warn (fires regardless of checkext)
    for (num, cv, what) in [
        (5, NO_EXT, "no-push"),
        (6, NO_EXT, "non-bsp"),
        (6, FTE, "non-bsp, extensions on"),
    ] {
        let specs = vec![
            EdictSpec::new(5, SOLID_BSP, [1.0, 2.0, 3.0], 0.0)
                .boxed([-64.0, -64.0, -64.0], [64.0, 64.0, 64.0])
                .brush(1.0, MOVETYPE_NONE),
            EdictSpec::new(6, SOLID_BSP, [4.0, 5.0, 6.0], 0.0)
                .boxed([-16.0, -16.0, -16.0], [16.0, 16.0, 16.0])
                .brush(2.0, MOVETYPE_PUSH),
        ];

        let mut results = Vec::new();
        for side in [Side::C, Side::Rust] {
            setup(side, cv, false, &specs, false);
            // SAFETY: arms the knownstrings table the fixture owns and points
            // this edict's classname at its NULL slot.
            unsafe { ctest_world_arm_bad_classname(num) };
            // SAFETY: plain console-log reset.
            unsafe { ctest_clear_con_log() };

            let mut args = Args {
                side,
                // SAFETY: `num` indexes the fixture arena.
                ent: unsafe { ctest_world_edict(num) },
                mins: [-16.0, -16.0, -24.0],
                maxs: [16.0, 16.0, 32.0],
                offset: [0.0; 3],
                hull: core::ptr::null_mut(),
            };
            // SAFETY: `call` only touches `args`, which outlives the call.
            let raised = unsafe { ctest_try_host(call, (&mut args as *mut Args).cast::<c_void>()) };
            // SAFETY: the trap's message buffer is a static NUL-terminated
            // C string, only rewritten by the next Host_Error.
            let msg = unsafe {
                CStr::from_ptr(ctest_host_error_message())
                    .to_string_lossy()
                    .into_owned()
            };
            results.push((raised, msg, args.hull.is_null(), con_log()));
        }

        let (c, rust) = (&results[0], &results[1]);
        assert_eq!(
            c.0, 1,
            "the C oracle must actually raise for the {what} site -- otherwise \
             this test proves nothing about propagation"
        );
        assert_eq!(c.0, rust.0, "Host_Error fired on both sides ({what})");
        assert_eq!(c.1, rust.1, "Host_Error message ({what})");
        assert!(
            c.1.contains("PR_GetString"),
            "the raise came from PR_GetString, not somewhere else: {:?}",
            c.1
        );
        assert_eq!(c.2, rust.2, "no hull was produced on either side ({what})");
        assert!(c.2, "a raising call must not hand back a hull ({what})");
        // Con_Warning never ran: the argument raised before the call.
        assert_eq!(c.3, rust.3, "console log up to the raise ({what})");
        assert!(
            c.3.iter().all(|l| !l.contains("SOLID_BSP")),
            "the warning must not have been printed ({what}): {:?}",
            c.3
        );
    }
}

// ---------------------------------------------------------------------------
// Hull tracing

/// The curated trace scenarios the contract calls out: starts-in-solid,
/// ends-in-solid, entirely-in-solid, zero length, exactly along a plane, and
/// a move that has to bisect back through DIST_EPSILON.
fn trace_scenarios() -> Vec<([f32; 3], [f32; 3], &'static str)> {
    vec![
        // clean miss through the open room
        ([-400.0, 0.0, 0.0], [-100.0, 0.0, 0.0], "open"),
        // straight into the -X face of the pillar
        ([0.0, 64.0, 0.0], [200.0, 64.0, 0.0], "hits pillar"),
        // starts inside the pillar, ends outside
        ([64.0, 64.0, 0.0], [300.0, 64.0, 0.0], "starts in solid"),
        // starts outside, ends inside the pillar
        ([-200.0, 64.0, 0.0], [64.0, 64.0, 0.0], "ends in solid"),
        // entirely inside the pillar
        ([48.0, 48.0, 0.0], [80.0, 80.0, 0.0], "all solid"),
        // entirely outside the map
        ([-2000.0, 0.0, 0.0], [-1000.0, 0.0, 0.0], "outside"),
        // zero length, in open space
        ([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], "zero open"),
        // zero length, inside the pillar
        ([64.0, 64.0, 0.0], [64.0, 64.0, 0.0], "zero solid"),
        // zero length, exactly on the pillar's -X plane
        ([32.0, 64.0, 0.0], [32.0, 64.0, 0.0], "zero on plane"),
        // exactly along the pillar's -X plane
        ([32.0, -200.0, 0.0], [32.0, 200.0, 0.0], "along plane x"),
        // exactly along the pillar's -Y plane
        ([-200.0, 32.0, 0.0], [200.0, 32.0, 0.0], "along plane y"),
        // grazes the open room's ceiling plane
        ([-400.0, 0.0, 192.0], [400.0, 0.0, 192.0], "along ceiling"),
        // a long diagonal that clips the pillar corner: the DIST_EPSILON
        // back-off has to bisect the midfrac
        (
            [-300.0, -300.0, 0.0],
            [300.0, 300.0, 0.0],
            "corner diagonal",
        ),
        // out of the water box into open air (a contents transition, not a
        // solid one -- only visible with a wider hitcontents mask)
        ([-160.0, -160.0, -160.0], [0.0, 0.0, 0.0], "water to air"),
        // through the lava box
        (
            [-192.0, 300.0, -192.0],
            [-192.0, 100.0, -192.0],
            "through lava",
        ),
        // out of the map, into the map
        ([0.0, 0.0, 600.0], [0.0, 0.0, 0.0], "solid to open"),
        // into the map, out of the map
        ([0.0, 0.0, 0.0], [0.0, 0.0, 600.0], "open to solid"),
        // tiny sub-epsilon move straddling a plane
        ([31.99, 64.0, 0.0], [32.01, 64.0, 0.0], "sub-epsilon"),
    ]
}

fn run_hull_traces(cv: Cvars) {
    let _g = lock();
    let scenarios = trace_scenarios();
    let snap = diff(cv, false, &[], false, |side| {
        let mut out = Vec::new();
        for h in 0..3 {
            for (p1, p2, name) in &scenarios {
                for mask in [CONTENTMASK_ANYSOLID, !0u32, 1u32 << 3] {
                    out.push((
                        *name,
                        h,
                        mask,
                        side.recursive_hull_check(hull(h), p1, p2, mask),
                    ));
                }
            }
        }
        out
    });
    // guard against a vacuous pass: the scenario set has to actually produce
    // impacts, start-solid hits and clean completions
    let traces: Vec<&TraceBits> = snap.value.iter().map(|(_, _, _, (_, t))| t).collect();
    assert!(
        traces.iter().any(|t| t.fraction != 1.0f32.to_bits()),
        "no trace was ever blocked"
    );
    assert!(
        traces.iter().any(|t| t.startsolid != 0),
        "no start-solid trace"
    );
    assert!(traces.iter().any(|t| t.allsolid != 0), "no all-solid trace");
    assert!(
        traces.iter().any(|t| t.plane_normal != [0u32; 3]),
        "no trace ever recorded a plane"
    );
    assert!(
        snap.value.iter().any(|(_, _, _, (r, _))| *r != 0),
        "no trace ever completed"
    );
}

#[test]
fn recursive_hull_check_matches_vanilla_path() {
    run_hull_traces(VANILLA);
}

#[test]
fn recursive_hull_check_matches_fte_path() {
    run_hull_traces(FTE);
}

#[test]
fn recursive_hull_check_matches_with_extensions_disabled() {
    // pr_checkextension 0 forces the vanilla branch even with the cvar at 1
    run_hull_traces(NO_EXT);
}

#[test]
fn q1bsp_recursive_hull_trace_matches_directly() {
    let _g = lock();
    let scenarios = trace_scenarios();
    diff(FTE, false, &[], false, |side| {
        let mut out = Vec::new();
        for h in 0..3 {
            for (p1, p2, name) in &scenarios {
                // non-trivial p1f/p2f windows exercise the fraction
                // interpolation the public wrapper always calls with 0..1
                for (p1f, p2f) in [(0.0f32, 1.0f32), (0.25, 0.75), (0.5, 0.5)] {
                    out.push((
                        *name,
                        h,
                        p1f.to_bits(),
                        p2f.to_bits(),
                        side.q1bsp_hull_trace(hull(h), 0, p1f, p2f, p1, p2, CONTENTMASK_ANYSOLID),
                    ));
                }
            }
        }
        out
    });
}

// ---------------------------------------------------------------------------
// Randomised sweep

/// A coordinate generator that mixes exact plane coordinates, integers and
/// arbitrary floats, so the sweep hits the on-plane and epsilon cases as well
/// as the generic ones.
fn coord() -> impl Strategy<Value = f32> {
    prop_oneof![
        3 => prop::sample::select(vec![
            -448.0f32, -256.0, -192.0, -128.0, -64.0, -32.0, 0.0, 32.0, 64.0, 96.0, 128.0, 192.0,
            256.0, 448.0,
        ]),
        3 => (-600i32..600i32).prop_map(|v| v as f32),
        4 => -600.0f32..600.0f32,
    ]
}

fn point() -> impl Strategy<Value = [f32; 3]> {
    [coord(), coord(), coord()]
}

/// One generated hull-trace case: hull index, FTE-path flag, both endpoints
/// and which hitcontents mask to use.
type SweepCase = (usize, bool, [f32; 3], [f32; 3], u32);

/// Fixed so the sweep is reproducible run to run (proptest otherwise seeds
/// from the OS RNG, which would make a failure unrepeatable and the suite
/// non-deterministic -- an explicit harness requirement).
const SWEEP_SEED: [u8; 32] = *b"vkqr-phase7-m3-world-sweep-seed!";

#[test]
fn recursive_hull_check_property_sweep() {
    let _g = lock();
    let mut runner = TestRunner::new_with_rng(
        Config {
            cases: 384,
            failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
            ..Config::default()
        },
        TestRng::from_seed(RngAlgorithm::ChaCha, &SWEEP_SEED),
    );

    // one fixture per side, reused across every generated case: the traces
    // are pure reads of the hulls, so nothing accumulates between cases
    let collected: RefCell<Vec<SweepCase>> = RefCell::new(Vec::new());
    runner
        .run(
            &(0usize..3, any::<bool>(), point(), point(), 0u32..3u32),
            |case| {
                collected.borrow_mut().push(case);
                Ok(())
            },
        )
        .expect("sweep generation");
    let inputs = collected.into_inner();

    for cv in [VANILLA, FTE] {
        let cases: Vec<_> = inputs
            .iter()
            .copied()
            .filter(|(_, fte, _, _, _)| *fte == (cv.hullcheck > 0.0))
            .collect();
        diff(cv, false, &[], false, |side| {
            cases
                .iter()
                .map(|&(h, _, p1, p2, maskpick)| {
                    let mask = match maskpick {
                        0 => CONTENTMASK_ANYSOLID,
                        1 => !0u32,
                        _ => 1u32 << 3,
                    };
                    side.recursive_hull_check(hull(h as c_int), &p1, &p2, mask)
                })
                .collect::<Vec<_>>()
        });
    }
}

// ---------------------------------------------------------------------------
// Area nodes

fn run_area_tree(cv: Cvars) {
    let _g = lock();
    diff(cv, false, &[], false, |_side| ());
}

#[test]
fn clear_world_tree_matches_vanilla() {
    run_area_tree(VANILLA);
}

#[test]
fn clear_world_tree_matches_fte() {
    run_area_tree(FTE);
}

#[test]
fn clear_world_tree_matches_with_extensions_disabled() {
    run_area_tree(NO_EXT);
}

#[test]
fn create_area_node_matches_at_every_depth() {
    let _g = lock();
    let (wmins, wmaxs) = {
        // SAFETY / setup: read the fixture's world bounds once, after a reset
        // so the model exists.
        // SAFETY: plain fixture reset.
        unsafe { ctest_world_reset(0, ARENA) };
        world_bounds()
    };
    // boxes that pick each axis, and one that trips the FTE `size < 500`
    // early-out immediately
    let boxes: Vec<([f32; 3], [f32; 3])> = vec![
        (wmins, wmaxs),
        ([-100.0, -900.0, -50.0], [100.0, 900.0, 50.0]),
        ([-900.0, -100.0, -50.0], [900.0, 100.0, 50.0]),
        ([-100.0, -100.0, -100.0], [100.0, 100.0, 100.0]),
        ([-250.0, -250.0, -8.0], [250.0, 250.0, 8.0]),
    ];
    for cv in [VANILLA, FTE, NO_EXT] {
        // depths above the terminating one never terminate: the vanilla
        // branch stops on `depth == VANILLA_AREA_DEPTH` exactly, so a start
        // depth past 4 would recurse forever in BOTH implementations
        for depth in [0, 1, 3, 4] {
            diff(cv, false, &[], false, |side| {
                let mut out = Vec::new();
                for (mins, maxs) in &boxes {
                    // SAFETY: plain fixture reset of the areanode array.
                    unsafe { ctest_world_reset_areanodes() };
                    out.push(side.create_area_node(depth, mins, maxs));
                    // SAFETY: plain counter read.
                    out.push(unsafe { ctest_world_numareanodes() });
                    out.extend(snapshot_areanodes());
                }
                // leave a valid tree behind for diff()'s own snapshot
                // SAFETY: plain fixture reset of the areanode array.
                unsafe { ctest_world_reset_areanodes() };
                side.create_area_node(0, &wmins, &wmaxs);
                out
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Linking

#[test]
fn link_edict_chain_order_matches() {
    let _g = lock();
    let specs = population();
    // link order is fixed and deliberately not sorted: InsertLinkBefore puts
    // each new edict at the head, so the chains come out reversed and any
    // difference in insertion side shows up immediately
    let vanilla = diff(VANILLA, false, &specs, false, |_side| ());
    let fte = diff(FTE, false, &specs, false, |_side| ());
    // 4 ints per link; every non-SOLID_NOT edict in the population links
    assert_eq!(
        vanilla.links.len(),
        4 * 9,
        "vanilla links: {:?}",
        vanilla.links
    );
    assert_eq!(fte.links.len(), 4 * 9, "fte links: {:?}", fte.links);
    assert_ne!(
        vanilla.links, fte.links,
        "the two areanode depths must home edicts differently"
    );
}

#[test]
fn relink_and_unlink_chain_order_matches() {
    let _g = lock();
    let specs = population();
    diff(FTE, false, &specs, false, |side| {
        // unlink a few out of the middle, relink one at a new position, then
        // relink everything: the resulting chain order is entirely a
        // function of the implementation's insert/remove ordering
        side.unlink_edict(4);
        side.unlink_edict(1);
        side.unlink_edict(7);
        // an unlink of an already-unlinked edict must stay a no-op
        side.unlink_edict(7);
        // and of an edict that was never linked at all
        side.unlink_edict(20);
        side.link_edict(4, false);
        side.link_edict(1, false);
        for s in population() {
            side.link_edict(s.num, false);
        }
        // moving an edict across an areanode boundary re-homes it
        let mut moved = EdictSpec::new(3, SOLID_BBOX, [1500.0, -1500.0, 200.0], 16.0);
        moved.movetype = MOVETYPE_NONE;
        moved.apply();
        side.link_edict(3, false);
    });
}

#[test]
fn link_edict_rotation_expansion_matches() {
    let _g = lock();
    // the SOLID_BSP + pr_checkextension + non-axis-aligned-angles branch:
    // absmin/absmax come from a radius instead of the bbox
    let specs = vec![
        EdictSpec::new(1, SOLID_BSP, [10.0, -20.0, 30.0], 0.0)
            .boxed([-40.0, -24.0, -16.0], [40.0, 24.0, 16.0])
            .brush(1.0, MOVETYPE_PUSH)
            .angled([0.0, 45.0, 0.0]),
        // axis-aligned angles take the plain bbox path
        EdictSpec::new(2, SOLID_BSP, [10.0, -20.0, 30.0], 0.0)
            .boxed([-40.0, -24.0, -16.0], [40.0, 24.0, 16.0])
            .brush(1.0, MOVETYPE_PUSH)
            .angled([0.0, 90.0, 0.0]),
        // origin outside the bbox disables the rotation expansion
        EdictSpec::new(3, SOLID_BSP, [10.0, -20.0, 30.0], 0.0)
            .boxed([100.0, 100.0, 100.0], [140.0, 140.0, 140.0])
            .brush(1.0, MOVETYPE_PUSH)
            .angled([15.0, 25.0, 35.0]),
        EdictSpec::new(4, SOLID_BSP, [0.0, 0.0, 0.0], 0.0)
            .boxed([-13.0, -29.0, -7.0], [11.0, 3.0, 47.0])
            .brush(1.0, MOVETYPE_PUSH)
            .angled([12.5, -33.25, 180.0]),
    ];
    // only reachable with pr_checkextension on; NO_EXT must take the other
    // branch on both sides
    diff(FTE, false, &specs, false, |_side| ());
    diff(NO_EXT, false, &specs, false, |_side| ());
}

#[test]
fn find_touched_leafs_matches() {
    let _g = lock();
    let specs = population();
    diff(FTE, false, &specs, false, |side| {
        for s in population() {
            side.find_touched_leafs(s.num);
        }
    });
}

// ---------------------------------------------------------------------------
// Touch dispatch

fn touch_population() -> Vec<EdictSpec> {
    vec![
        EdictSpec::new(1, SOLID_TRIGGER, [0.0, 0.0, 0.0], 48.0).touching(TOUCH_LOG),
        EdictSpec::new(2, SOLID_TRIGGER, [8.0, 0.0, 0.0], 48.0).touching(TOUCH_LOG),
        EdictSpec::new(3, SOLID_TRIGGER, [-8.0, 8.0, 0.0], 48.0).touching(TOUCH_LOG),
        // a trigger with no touch function: must be skipped
        EdictSpec::new(4, SOLID_TRIGGER, [0.0, 0.0, 0.0], 48.0),
        // a touch function on a non-trigger: must be skipped
        EdictSpec::new(5, SOLID_BBOX, [0.0, 0.0, 0.0], 48.0).touching(TOUCH_LOG),
        // out of range
        EdictSpec::new(6, SOLID_TRIGGER, [400.0, 400.0, 0.0], 16.0).touching(TOUCH_LOG),
        // the mover
        EdictSpec::new(11, SOLID_SLIDEBOX, [0.0, 0.0, 0.0], 16.0),
    ]
}

#[test]
fn touch_links_dispatch_order_matches() {
    let _g = lock();
    let specs = touch_population();
    let fte = diff(FTE, false, &specs, true, |_side| ());
    let vanilla = diff(VANILLA, false, &specs, true, |_side| ());
    // edicts 1..3 are overlapping triggers with a touch function; 4 has no
    // function, 5 is not a trigger and 6 is out of range
    // every spec is linked with touch_triggers set, so each link dispatches
    // against the triggers already in the chain: 1, then 1+2, then 1+2+3 ...
    let selves: Vec<i32> = fte.touch.iter().map(|e| e.0).collect();
    assert_eq!(
        selves,
        vec![1, 1, 2, 1, 2, 3, 1, 2, 3, 1, 2, 3],
        "trigger dispatch order"
    );
    assert!(
        fte.touch.iter().rev().take(3).all(|e| e.1 == 11),
        "the mover's own link touched all three triggers last"
    );
    assert!(
        fte.touch.iter().all(|e| e.2 == 4.5f32.to_bits()),
        "self.time is the qcvm's time"
    );
    assert_eq!(vanilla.touch.len(), 12);
}

#[test]
fn touch_links_reentrant_relink_matches() {
    let _g = lock();
    // handler on edict 2 relinks edict 3 mid-dispatch: SV_TouchLinks must
    // survive the chain mutating under it (that is why it snapshots to a
    // list of edict numbers first)
    let mut specs = touch_population();
    specs[1] = specs[1].touching(TOUCH_RELINK);
    let snap = diff(FTE, false, &specs, false, |side| {
        // SAFETY: plain fixture setter.
        unsafe { ctest_world_set_relink_target(3) };
        side.link_edict(11, true);
    });
    assert!(!snap.touch.is_empty(), "the relink handler never ran");
}

#[test]
fn touch_links_free_during_touch_matches() {
    let _g = lock();
    // handler on edict 1 frees edict 3 (later in the list): the re-validation
    // in SV_TouchLinks must skip it
    let mut specs = touch_population();
    specs[0] = specs[0].touching(TOUCH_FREE);
    let snap = diff(FTE, false, &specs, false, |side| {
        // SAFETY: plain fixture setter.
        unsafe { ctest_world_set_free_target(3) };
        side.link_edict(11, true);
    });
    assert!(!snap.touch.is_empty(), "the free handler never ran");
    // edict 3 was freed mid-dispatch and must not have been touched after
    assert!(
        snap.touch.iter().all(|e| e.0 != 3),
        "a freed edict still got touched: {:?}",
        snap.touch
    );
}

#[test]
fn touch_links_free_self_during_touch_matches() {
    let _g = lock();
    // handler on edict 2 frees the edict being linked: the loop must `break`
    // and the remaining handlers must NOT run
    let mut specs = touch_population();
    specs[1] = specs[1].touching(TOUCH_FREE_OTHER);
    let snap = diff(FTE, false, &specs, false, |side| {
        side.link_edict(11, true);
    });
    // edict 2 frees the linking edict, so dispatch must stop there: only the
    // handlers up to and including it ran
    assert!(!snap.touch.is_empty(), "no handler ran");
    assert!(
        snap.touch.len() < 3,
        "dispatch did not break: {:?}",
        snap.touch
    );
}

#[test]
fn touch_links_repeated_relink_matches() {
    let _g = lock();
    let mut specs = touch_population();
    specs[0] = specs[0].touching(TOUCH_RELINK);
    specs[2] = specs[2].touching(TOUCH_RELINK);
    diff(FTE, false, &specs, false, |side| {
        // SAFETY: plain fixture setter.
        unsafe { ctest_world_set_relink_target(4) };
        for _ in 0..3 {
            side.link_edict(11, true);
            // SAFETY: plain fixture setter.
            unsafe { ctest_world_touch_log_clear() };
        }
        side.link_edict(11, true);
    });
}

// ---------------------------------------------------------------------------
// Moves

/// start, mins, maxs, end.
type MoveBoundsCase = ([f32; 3], [f32; 3], [f32; 3], [f32; 3]);

#[test]
fn move_bounds_matches() {
    let _g = lock();
    let cases: [MoveBoundsCase; 6] = [
        (
            [0.0, 0.0, 0.0],
            [-16.0, -16.0, -24.0],
            [16.0, 16.0, 32.0],
            [100.0, 0.0, 0.0],
        ),
        (
            [100.0, 50.0, -25.0],
            [-16.0, -16.0, -24.0],
            [16.0, 16.0, 32.0],
            [0.0, 0.0, 0.0],
        ),
        (
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
        ),
        (
            [-1.5, 2.25, -3.125],
            [-0.5, -0.25, -0.125],
            [0.5, 0.25, 0.125],
            [-1.5, 2.25, -3.125],
        ),
        (
            [-9999.0, 9999.0, 0.0],
            [-15.0, -15.0, -15.0],
            [15.0, 15.0, 15.0],
            [9999.0, -9999.0, 0.0],
        ),
        (
            [1e30, -1e30, 0.0],
            [-1.0, -1.0, -1.0],
            [1.0, 1.0, 1.0],
            [-1e30, 1e30, 0.0],
        ),
    ];
    diff(VANILLA, false, &[], false, |side| {
        cases
            .iter()
            .map(|(s, mn, mx, e)| side.move_bounds(s, mn, mx, e))
            .collect::<Vec<_>>()
    });
}

#[test]
fn clip_move_to_entity_matches() {
    let _g = lock();
    let specs = population();
    let scenarios = trace_scenarios();
    let sizes: [([f32; 3], [f32; 3]); 3] = [
        ([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
        ([-16.0, -16.0, -24.0], [16.0, 16.0, 32.0]),
        ([-32.0, -32.0, -24.0], [32.0, 32.0, 64.0]),
    ];
    for cv in [VANILLA, FTE] {
        diff(cv, false, &specs, false, |side| {
            let mut out = Vec::new();
            // the world edict (0), a bbox, and the brush mover
            for ent in [0, 3, 5] {
                for (start, end, name) in &scenarios {
                    for (mins, maxs) in &sizes {
                        out.push((
                            *name,
                            ent,
                            side.clip_move_to_entity(
                                ent,
                                start,
                                mins,
                                maxs,
                                end,
                                CONTENTMASK_ANYSOLID,
                            ),
                        ));
                    }
                }
            }
            out
        });
    }
}

fn move_matrix(
    side: Side,
    scenarios: &[([f32; 3], [f32; 3], &'static str)],
) -> Vec<(String, TraceBits)> {
    let sizes: [([f32; 3], [f32; 3]); 3] = [
        ([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
        ([-16.0, -16.0, -24.0], [16.0, 16.0, 32.0]),
        ([-32.0, -32.0, -24.0], [32.0, 32.0, 64.0]),
    ];
    let types = [
        ("normal", MOVE_NORMAL),
        ("nomonsters", MOVE_NOMONSTERS),
        ("missile", MOVE_MISSILE),
        ("hitall", MOVE_NORMAL | MOVE_HITALLCONTENTS),
        ("nomonsters+hitall", MOVE_NOMONSTERS | MOVE_HITALLCONTENTS),
    ];
    let mut out = Vec::new();
    for (start, end, name) in scenarios {
        for (mins, maxs) in &sizes {
            for (tname, ty) in types {
                for pass in [None, Some(3), Some(5)] {
                    out.push((
                        format!("{name}/{tname}/{}/{pass:?}", mins[0]),
                        side.sv_move(start, mins, maxs, end, ty, pass),
                    ));
                }
            }
        }
    }
    out
}

#[test]
fn sv_move_matches_server_vm() {
    let _g = lock();
    let specs = population();
    let scenarios = trace_scenarios();
    for cv in [VANILLA, FTE, NO_EXT] {
        let snap = diff(cv, false, &specs, false, |side| {
            move_matrix(side, &scenarios)
        });
        assert!(
            snap.value
                .iter()
                .any(|(_, t)| t.fraction != 1.0f32.to_bits()),
            "no SV_Move was ever blocked"
        );
        assert!(
            snap.value.iter().any(|(_, t)| t.ent > 0),
            "no SV_Move ever hit a non-world entity"
        );
        assert!(
            snap.value.iter().any(|(_, t)| t.startsolid != 0),
            "no SV_Move ever started in solid"
        );
    }
}

#[test]
fn sv_move_matches_client_vm_with_network_entities() {
    let _g = lock();
    let specs = population();
    let scenarios = trace_scenarios();

    // ES_SOLID_* encodings from protocol.h: NOT, BSP, and the two packed
    // hull sizes vanilla clients send
    const ES_SOLID_NOT: c_uint = 0;
    const ES_SOLID_BSP: c_uint = 31;
    const ES_SOLID_HULL1: c_uint = 0x8020_1810;
    const ES_SOLID_HULL2: c_uint = 0x8040_1820;

    for cv in [VANILLA, FTE] {
        let snap = diff(cv, true, &specs, false, |side| {
            // SAFETY: plain fixture setters; index 0 is the local player slot
            // World_ClipToNetwork deliberately skips.
            unsafe {
                ctest_world_cl_set_num_entities(6);
                let z = [0.0f32; 3];
                ctest_world_cl_set_entity(0, 1, ES_SOLID_BSP, z.as_ptr(), z.as_ptr(), 0);
                ctest_world_cl_set_entity(
                    1,
                    1,
                    ES_SOLID_BSP,
                    [0.0f32, 0.0, 0.0].as_ptr(),
                    z.as_ptr(),
                    0,
                );
                ctest_world_cl_set_entity(
                    2,
                    2,
                    ES_SOLID_HULL1,
                    [40.0f32, 0.0, 0.0].as_ptr(),
                    z.as_ptr(),
                    0,
                );
                ctest_world_cl_set_entity(
                    3,
                    2,
                    ES_SOLID_HULL2,
                    [-40.0f32, 40.0, 0.0].as_ptr(),
                    z.as_ptr(),
                    1,
                );
                // solidsize NOT: skipped
                ctest_world_cl_set_entity(
                    4,
                    2,
                    ES_SOLID_NOT,
                    [0.0f32, -40.0, 0.0].as_ptr(),
                    z.as_ptr(),
                    0,
                );
                // no model: skipped
                ctest_world_cl_set_entity(
                    5,
                    0,
                    ES_SOLID_HULL1,
                    [0.0f32, 0.0, -40.0].as_ptr(),
                    z.as_ptr(),
                    0,
                );
            }
            move_matrix(side, &scenarios)
        });
        assert!(
            snap.value
                .iter()
                .any(|(_, t)| t.fraction != 1.0f32.to_bits()),
            "no client-VM move was ever blocked"
        );
    }
}

#[test]
fn test_entity_position_matches() {
    let _g = lock();
    let specs = population();
    for cv in [VANILLA, FTE] {
        let snap = diff(cv, false, &specs, false, |side| {
            population()
                .iter()
                .map(|s| side.test_entity_position(s.num))
                .collect::<Vec<_>>()
        });
        assert!(
            snap.value.iter().any(|r| *r >= 0),
            "no edict was ever found stuck"
        );
    }
}
