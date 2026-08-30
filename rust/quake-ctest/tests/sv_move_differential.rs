//! Differential test: the Rust `quake-capi` monster-movement port vs the
//! original `Quake/sv_move.c` (compiled as `c_ref_*`). Rust migration
//! Phase 7, M4 (task T4.1).
//!
//! Both implementations run against ONE shared fixture (`ctest_phys_*` and
//! `ctest_world_*` in `stubs/stubs.c`): the M3 synthetic room -- a brush
//! model with three real clipping hulls, a solid pillar, a water box and a
//! lava box -- republished on `sv.qcvm`, plus an edict arena, an areanode
//! tree and a progs image whose think/touch functions are genuine bytecode.
//!
//! Because the fixture state is global and mutable, every test follows the
//! `world_differential.rs` idiom: take the file mutex, then for each side
//! reset the fixture from scratch, drive the SAME call sequence through that
//! side's entry points, and snapshot everything observable (every entvars
//! field either function writes, as exact bit patterns, plus the areanode
//! topology, the per-areanode link chain ORDER, the touch-dispatch log, the
//! QC globals and the console log). The two snapshots must be identical.
//!
//! `SV_NewChaseDir` and `SV_MoveToGoal` consume `COM_Rand`, so
//! `ctest_phys_reset` re-seeds it with a fixed constant before each side
//! runs. A branch that adds or drops a single `COM_Rand` call therefore
//! desynchronises the stream and shows up as a snapshot mismatch, not as
//! flake (ADR-010).
//!
//! Five of the seven entry points are raise-capable, all of them through
//! `SV_Move`/`SV_LinkEdict`: `assert_always` reaches `Sys_Error`, and the
//! two `SV_HullForEntity` `Con_Warning` sites reach `Host_Error` by way of
//! `PR_GetString`. Per ADR-009 the Rust side of each is a `quake_rs_*` core
//! returning a `Host_Guard` status and the re-raise happens in a plain-named
//! C wrapper -- `Quake/sv_move_glue.c` in the engine, `stubs.c` here. The
//! tests drive the plain names on both sides, so no longjmp ever unwinds a
//! Rust frame. `SV_FixCheckBottom` and `SV_CloseEnough` cannot raise and are
//! plain exports on both sides.

use core::ffi::{c_char, c_float, c_int, c_void, CStr};
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

// ---------------------------------------------------------------------------
// server.h enumerators the fixture spells as floats (entvars_t stores them
// that way). Kept as literals rather than a mirror: ADR-011 mirrors are not
// in scope for M4, and `abi_probe.c` already pins these against the C
// compiler's own view.

const MOVETYPE_STEP: f32 = 4.0;
const MOVETYPE_FLY: f32 = 5.0;

const SOLID_NOT: f32 = 0.0;
const SOLID_BBOX: f32 = 2.0;
const SOLID_SLIDEBOX: f32 = 3.0;
const SOLID_BSP: f32 = 4.0;

const FL_FLY: c_int = 1;
const FL_SWIM: c_int = 2;
const FL_MONSTER: c_int = 32;
const FL_ONGROUND: c_int = 512;
const FL_PARTIALGROUND: c_int = 1024;

/// The fixture's touch/think function kinds (`ctest_world_touch_func`).
const KIND_LOG: c_int = 0;

/// Hull 1's floor in the synthetic room: the open box bottoms out at
/// z = -192 and the hull-1 clip box has mins[2] = -24.
const FLOOR: f32 = -168.0;

extern "C" {
    // --- fixture ----------------------------------------------------------
    fn ctest_phys_reset(
        num_edicts: c_int,
        maxclients: c_int,
        frametime: f64,
        vmtime: f64,
        physics_mode: c_int,
    );
    fn ctest_phys_set_cvars(v: *const c_float);
    fn ctest_phys_edict_set(
        num: c_int,
        scalars: *const c_float,
        vectors: *const c_float,
        think_kind: c_int,
        touch_kind: c_int,
        blocked_kind: c_int,
        is_free: c_int,
    );
    fn ctest_phys_edict_set_refs(num: c_int, enemy: c_int, goalentity: c_int, extra_flags: c_int);
    fn ctest_phys_set_self(num: c_int);
    fn ctest_phys_set_parm0(v: c_float);
    fn ctest_phys_globals(out3: *mut c_float);
    fn ctest_phys_edict_snapshot(num: c_int, out: *mut u32, max: c_int) -> c_int;

    fn ctest_world_reset(vm_kind: c_int, num_edicts: c_int);
    fn ctest_world_edict(num: c_int) -> *mut c_void;
    fn ctest_world_set_cvars(
        recursivehullcheck: c_float,
        createareanode: c_float,
        checkext: c_float,
    );
    fn ctest_world_set_link_fns(
        link: Option<extern "C" fn(*mut c_void, u8)>,
        unlink: Option<extern "C" fn(*mut c_void)>,
    );
    fn ctest_world_set_rust_link_fns();
    fn ctest_world_arm_bad_classname(num: c_int);
    fn ctest_world_snapshot_areanodes(out: *mut c_int, max: c_int) -> c_int;
    fn ctest_world_snapshot_links(out: *mut c_int, max: c_int) -> c_int;
    fn ctest_world_touch_log_len() -> c_int;
    fn ctest_world_touch_log_get(
        i: c_int,
        self_: *mut c_int,
        other: *mut c_int,
        time: *mut c_float,
        kind: *mut c_int,
    ) -> c_int;

    fn ctest_clear_con_log();
    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(i: c_int) -> *const c_char;
    fn ctest_try_host(f: extern "C" fn(*mut c_void), arg: *mut c_void) -> c_int;
    fn ctest_host_error_message() -> *const c_char;

    // --- oracle (Quake/sv_move.c, renamed by include/c_ref_prelude.h) -----
    fn c_ref_SV_CheckBottom(ent: *mut c_void) -> u8;
    fn c_ref_SV_movestep(ent: *mut c_void, move_: *mut c_float, relink: u8) -> u8;
    fn c_ref_SV_StepDirection(ent: *mut c_void, yaw: c_float, dist: c_float) -> u8;
    fn c_ref_SV_NewChaseDir(actor: *mut c_void, enemy: *mut c_void, dist: c_float);
    fn c_ref_SV_MoveToGoal();
    fn c_ref_SV_FixCheckBottom(ent: *mut c_void);
    fn c_ref_SV_CloseEnough(ent: *mut c_void, goal: *mut c_void, dist: c_float) -> u8;
    fn c_ref_SV_InitBoxHull();
    fn c_ref_SV_ClearWorld();
    fn c_ref_SV_LinkEdict(ent: *mut c_void, touch_triggers: u8);

    // --- port (rust/quake-capi/src/sv_move.rs, plain names per ADR-009) ---
    // The plain names are the re-raising C wrappers in stubs.c over the
    // quake_rs_sv_* status cores, mirroring Quake/sv_move_glue.c.
    fn SV_CheckBottom(ent: *mut c_void) -> u8;
    fn SV_movestep(ent: *mut c_void, move_: *mut c_float, relink: u8) -> u8;
    fn SV_StepDirection(ent: *mut c_void, yaw: c_float, dist: c_float) -> u8;
    fn SV_NewChaseDir(actor: *mut c_void, enemy: *mut c_void, dist: c_float);
    fn SV_MoveToGoal();
    fn SV_FixCheckBottom(ent: *mut c_void);
    fn SV_CloseEnough(ent: *mut c_void, goal: *mut c_void, dist: c_float) -> u8;
    fn SV_InitBoxHull();
    fn SV_ClearWorld();
    fn SV_LinkEdict(ent: *mut c_void, touch_triggers: u8);
}

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

    fn check_bottom(self, num: c_int) -> bool {
        // SAFETY: `num` indexes the fixture arena; plain entry points.
        let r = unsafe {
            let ent = ctest_world_edict(num);
            match self {
                Side::C => c_ref_SV_CheckBottom(ent),
                Side::Rust => SV_CheckBottom(ent),
            }
        };
        r != 0
    }

    fn movestep(self, num: c_int, mv: [f32; 3], relink: bool) -> bool {
        let mut mv = mv;
        let r = u8::from(relink);
        // SAFETY: `num` indexes the fixture arena and `mv` is a live vec3_t
        // buffer that outlives the call.
        let out = unsafe {
            let ent = ctest_world_edict(num);
            match self {
                Side::C => c_ref_SV_movestep(ent, mv.as_mut_ptr(), r),
                Side::Rust => SV_movestep(ent, mv.as_mut_ptr(), r),
            }
        };
        out != 0
    }

    fn step_direction(self, num: c_int, yaw: f32, dist: f32) -> bool {
        // SAFETY: `num` indexes the fixture arena; plain entry points.
        let r = unsafe {
            let ent = ctest_world_edict(num);
            match self {
                Side::C => c_ref_SV_StepDirection(ent, yaw, dist),
                Side::Rust => SV_StepDirection(ent, yaw, dist),
            }
        };
        r != 0
    }

    fn new_chase_dir(self, actor: c_int, enemy: c_int, dist: f32) {
        // SAFETY: both numbers index the fixture arena; plain entry points.
        unsafe {
            let a = ctest_world_edict(actor);
            let e = ctest_world_edict(enemy);
            match self {
                Side::C => c_ref_SV_NewChaseDir(a, e, dist),
                Side::Rust => SV_NewChaseDir(a, e, dist),
            }
        }
    }

    fn move_to_goal(self) {
        // SAFETY: no arguments; both read pr_global_struct->self and
        // OFS_PARM0, which the caller set through the fixture.
        unsafe {
            match self {
                Side::C => c_ref_SV_MoveToGoal(),
                Side::Rust => SV_MoveToGoal(),
            }
        }
    }

    fn fix_check_bottom(self, num: c_int) {
        // SAFETY: `num` indexes the fixture arena. Neither side can raise.
        unsafe {
            let ent = ctest_world_edict(num);
            match self {
                Side::C => c_ref_SV_FixCheckBottom(ent),
                Side::Rust => SV_FixCheckBottom(ent),
            }
        }
    }

    fn close_enough(self, ent: c_int, goal: c_int, dist: f32) -> bool {
        // SAFETY: both numbers index the fixture arena. Neither side raises.
        let r = unsafe {
            let e = ctest_world_edict(ent);
            let g = ctest_world_edict(goal);
            match self {
                Side::C => c_ref_SV_CloseEnough(e, g, dist),
                Side::Rust => SV_CloseEnough(e, g, dist),
            }
        };
        r != 0
    }
}

// ---------------------------------------------------------------------------
// Fixture population

/// One edict's initial state. `scalars`/`vectors` are exactly the two blocks
/// `ctest_phys_edict_set` documents.
#[derive(Clone, Copy)]
struct Spec {
    num: c_int,
    scalars: [f32; 16],
    vectors: [f32; 18],
    think: c_int,
    touch: c_int,
    blocked: c_int,
    free: c_int,
    enemy: c_int,
    goal: c_int,
    extra_flags: c_int,
}

impl Spec {
    fn blank(num: c_int) -> Self {
        let mut s = Spec {
            num,
            scalars: [0.0; 16],
            vectors: [0.0; 18],
            think: -1,
            touch: -1,
            blocked: -1,
            free: 0,
            enemy: -1,
            goal: -1,
            extra_flags: 0,
        };
        s.scalars[6] = -1.0; // groundentity: none
        s.scalars[10] = -1.0; // owner: none
        s
    }

    /// A walking monster with the standard player-sized bbox, which is what
    /// hull 1 of the fixture's brush model clips against.
    fn monster(num: c_int, origin: [f32; 3]) -> Self {
        let mut s = Spec::blank(num);
        s.scalars[0] = MOVETYPE_STEP;
        s.scalars[1] = SOLID_SLIDEBOX;
        s.scalars[3] = (FL_MONSTER | FL_ONGROUND) as f32;
        s.scalars[11] = 20.0; // yaw_speed
        s.origin(origin)
            .bbox([-16.0, -16.0, -24.0], [16.0, 16.0, 32.0])
    }

    fn origin(mut self, o: [f32; 3]) -> Self {
        self.vectors[0..3].copy_from_slice(&o);
        self
    }

    fn bbox(mut self, mins: [f32; 3], maxs: [f32; 3]) -> Self {
        self.vectors[3..6].copy_from_slice(&mins);
        self.vectors[6..9].copy_from_slice(&maxs);
        self
    }

    fn solid(mut self, v: f32) -> Self {
        self.scalars[1] = v;
        self
    }

    fn movetype(mut self, v: f32) -> Self {
        self.scalars[0] = v;
        self
    }

    fn modelindex(mut self, v: f32) -> Self {
        self.scalars[2] = v;
        self
    }

    fn flags(mut self, v: c_int) -> Self {
        self.scalars[3] = v as f32;
        self
    }

    fn ideal_yaw(mut self, v: f32) -> Self {
        self.scalars[12] = v;
        self
    }

    fn refs(mut self, enemy: c_int, goal: c_int) -> Self {
        self.enemy = enemy;
        self.goal = goal;
        self
    }

    fn touching(mut self, kind: c_int) -> Self {
        self.touch = kind;
        self
    }

    fn freed(mut self) -> Self {
        self.free = 1;
        self
    }

    fn apply(&self) {
        // SAFETY: `num` indexes the fixture arena and both buffers have the
        // documented widths (16 scalars, 18 vector floats).
        unsafe {
            ctest_phys_edict_set(
                self.num,
                self.scalars.as_ptr(),
                self.vectors.as_ptr(),
                self.think,
                self.touch,
                self.blocked,
                self.free,
            );
            ctest_phys_edict_set_refs(self.num, self.enemy, self.goal, self.extra_flags);
        }
    }
}

const ARENA: c_int = 24;

/// `ctest_phys_set_cvars`' 13-float block. sv_move.c itself reads none of
/// these; they reach it through `SV_Move`, and pinning them keeps a later
/// sv_phys change from silently moving this suite's ground.
const CVARS: [f32; 13] = [
    4.0,    // sv_friction
    100.0,  // sv_stopspeed
    800.0,  // sv_gravity
    2000.0, // sv_maxvelocity
    0.0,    // sv_nostep
    0.0,    // sv_freezenonclients
    0.0,    // sv_gameplayfix_spawnbeforethinks
    1.0,    // sv_gameplayfix_bouncedownslopes
    3.0,    // sv_gameplayfix_elevators
    1.0,    // sv_fastpushmove
    1.0,    // sv_pushgrid
    1.0,    // sv_analyticphysics
    0.0,    // sv_speeds
];

/// Rebuilds the whole fixture on `sv.qcvm` and brings `side` up to a linked
/// world.
fn setup(side: Side, specs: &[Spec]) {
    // SAFETY: plain fixture setters; the file mutex serializes all callers.
    unsafe {
        ctest_phys_reset(ARENA, 1, 0.05, 4.5, -1);
        ctest_phys_set_cvars(CVARS.as_ptr());
        ctest_world_set_cvars(0.0, 0.0, 1.0);
    }
    side.install_link_fns();
    side.init_box_hull();
    side.clear_world();
    for s in specs {
        s.apply();
        if s.free == 0 {
            side.link_edict(s.num, false);
        }
    }
}

// ---------------------------------------------------------------------------
// Snapshots

/// `ctest_phys_edict_snapshot`'s fixed width. Asserted, not assumed.
const EDICT_WORDS: usize = 50;

fn snapshot_edicts(count: c_int) -> Vec<u32> {
    let mut out = Vec::with_capacity(count as usize * EDICT_WORDS);
    for i in 0..count {
        let mut buf = [0u32; EDICT_WORDS];
        // SAFETY: `i` is inside the arena and `buf` has the full width the
        // helper requires (it returns 0 rather than writing short).
        let n = unsafe { ctest_phys_edict_snapshot(i, buf.as_mut_ptr(), EDICT_WORDS as c_int) };
        assert_eq!(
            n as usize, EDICT_WORDS,
            "ctest_phys_edict_snapshot width changed; update EDICT_WORDS"
        );
        out.extend_from_slice(&buf);
    }
    out
}

fn snapshot_areanodes() -> Vec<i32> {
    let mut buf = vec![0i32; 5 * 1024];
    let len = buf.len() as c_int;
    // SAFETY: `buf` is sized for the whole AREA_NODES array and the helper
    // honours the cap.
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

fn qc_globals() -> [u32; 3] {
    let mut g = [0f32; 3];
    // SAFETY: `g` has room for the three floats the helper writes.
    unsafe { ctest_phys_globals(g.as_mut_ptr()) };
    [g[0].to_bits(), g[1].to_bits(), g[2].to_bits()]
}

/// Everything observable after one side has run.
struct Snap<T> {
    value: T,
    edicts: Vec<u32>,
    areanodes: Vec<i32>,
    links: Vec<i32>,
    touch: Vec<(i32, i32, u32, i32)>,
    globals: [u32; 3],
    con: Vec<String>,
}

fn capture<T>(value: T) -> Snap<T> {
    Snap {
        value,
        edicts: snapshot_edicts(ARENA),
        areanodes: snapshot_areanodes(),
        links: snapshot_links(),
        touch: touch_log(),
        globals: qc_globals(),
        con: con_log(),
    }
}

/// Runs `body` once per side against a freshly reset fixture, asserts the two
/// snapshots are identical, and hands the C side's snapshot back so the
/// caller can assert the scenario actually produced signal.
fn diff<T, F>(specs: &[Spec], body: F) -> Snap<T>
where
    T: PartialEq + core::fmt::Debug,
    F: Fn(Side) -> T,
{
    setup(Side::C, specs);
    // SAFETY: plain console-log reset.
    unsafe { ctest_clear_con_log() };
    let c = capture(body(Side::C));

    setup(Side::Rust, specs);
    // SAFETY: plain console-log reset.
    unsafe { ctest_clear_con_log() };
    let rust = capture(body(Side::Rust));

    assert_eq!(c.value, rust.value, "return value");
    assert_eq!(c.edicts, rust.edicts, "per-edict entvars");
    assert_eq!(c.areanodes, rust.areanodes, "areanode tree");
    assert_eq!(c.links, rust.links, "link chain order");
    assert_eq!(c.touch, rust.touch, "touch dispatch log");
    assert_eq!(c.globals, rust.globals, "QC globals (OFS_RETURN / self)");
    assert_eq!(c.con, rust.con, "console log");
    c
}

/// The monster under test is always edict 1; edicts 2 and 3 are its enemy and
/// goal, and 4 is a solid obstacle near the pillar.
const ACTOR: c_int = 1;
const ENEMY: c_int = 2;
const GOAL: c_int = 3;

fn base_population(actor_origin: [f32; 3]) -> Vec<Spec> {
    vec![
        Spec::monster(ACTOR, actor_origin).refs(ENEMY, GOAL),
        Spec::monster(ENEMY, [200.0, 0.0, FLOOR]),
        Spec::blank(GOAL)
            .solid(SOLID_BBOX)
            .origin([260.0, 40.0, FLOOR])
            .bbox([-16.0, -16.0, -24.0], [16.0, 16.0, 32.0]),
        Spec::blank(4)
            .solid(SOLID_BBOX)
            .origin([120.0, 0.0, FLOOR])
            .bbox([-24.0, -24.0, -24.0], [24.0, 24.0, 40.0]),
    ]
}

// ===========================================================================
// SV_CheckBottom

/// Positions chosen so the C oracle takes both of `SV_CheckBottom`'s paths:
/// the four-corner easy-out (all corners over solid world) and the
/// `realcheck` label, with `realcheck` itself reaching all three of its exits
/// -- the `fraction == 1.0` bail, a clean pass, and the `mid - endpos >
/// STEPSIZE` dangling-corner bail.
fn check_bottom_cases() -> Vec<([f32; 3], &'static str)> {
    vec![
        ([0.0, 0.0, FLOOR], "standing on the room floor (easy-out)"),
        (
            [0.0, 0.0, -100.0],
            "hovering well clear of the floor (realcheck, nothing below)",
        ),
        (
            [0.0, 0.0, -160.0],
            "hovering one step above the floor (realcheck, passes)",
        ),
        (
            [40.0, 56.0, -146.0],
            "straddling the pillar corner (realcheck, dangling corner)",
        ),
        ([-160.0, -160.0, FLOOR], "over the water box"),
        ([300.0, 300.0, FLOOR], "far corner of the room"),
        ([440.0, 0.0, FLOOR], "bbox poking through the room wall"),
    ]
}

#[test]
fn check_bottom_matches() {
    let _g = lock();

    let mut results = Vec::new();
    for (origin, what) in check_bottom_cases() {
        let specs = base_population(origin);
        let c = diff(&specs, |side| side.check_bottom(ACTOR));
        results.push((what, c.value));
    }

    assert!(
        results.iter().any(|r| r.1),
        "no SV_CheckBottom case returned true: {results:?}"
    );
    assert!(
        results.iter().any(|r| !r.1),
        "no SV_CheckBottom case returned false, so the realcheck exits are \
         untested: {results:?}"
    );
}

#[test]
fn check_bottom_is_unaffected_by_partialground() {
    let _g = lock();

    // SV_CheckBottom reads no flags, so the two runs must agree; this is the
    // guard against a port that "helpfully" consults FL_PARTIALGROUND.
    let plain = base_population([0.0, 0.0, -160.0]);
    let flagged = vec![
        Spec::monster(ACTOR, [0.0, 0.0, -160.0])
            .flags(FL_MONSTER | FL_ONGROUND | FL_PARTIALGROUND)
            .refs(ENEMY, GOAL),
        plain[1],
        plain[2],
        plain[3],
    ];

    let a = diff(&plain, |side| side.check_bottom(ACTOR));
    let b = diff(&flagged, |side| side.check_bottom(ACTOR));
    assert_eq!(
        a.value, b.value,
        "FL_PARTIALGROUND must not change the answer"
    );
}

// ===========================================================================
// SV_FixCheckBottom / SV_CloseEnough -- the two non-raising plain exports

#[test]
fn fix_check_bottom_matches() {
    let _g = lock();

    for flags in [0, FL_MONSTER, FL_MONSTER | FL_PARTIALGROUND, -1] {
        let specs = vec![Spec::monster(ACTOR, [0.0, 0.0, FLOOR]).flags(flags)];
        diff(&specs, |side| side.fix_check_bottom(ACTOR));
    }
}

#[test]
fn close_enough_matches() {
    let _g = lock();

    let mut sawtrue = false;
    let mut sawfalse = false;
    for dx in [0.0f32, 20.0, 64.0, 200.0, -200.0] {
        for dist in [0.0f32, 1.0, 32.0, 1000.0] {
            let specs = vec![
                Spec::monster(ACTOR, [0.0, 0.0, FLOOR]),
                Spec::blank(GOAL)
                    .solid(SOLID_BBOX)
                    .origin([dx, 0.0, FLOOR])
                    .bbox([-16.0, -16.0, -24.0], [16.0, 16.0, 32.0]),
            ];
            let c = diff(&specs, |side| side.close_enough(ACTOR, GOAL, dist));
            sawtrue |= c.value;
            sawfalse |= !c.value;
        }
    }
    assert!(sawtrue && sawfalse, "SV_CloseEnough never went both ways");
}

// ===========================================================================
// SV_movestep

/// (origin, move delta, entity flags, modelindex, label).
type MovestepCase = ([f32; 3], [f32; 3], c_int, f32, &'static str);

fn movestep_cases() -> Vec<MovestepCase> {
    // (origin, move, flags, modelindex-unused, label)
    vec![
        (
            [0.0, 0.0, FLOOR],
            [24.0, 0.0, 0.0],
            FL_MONSTER | FL_ONGROUND,
            0.0,
            "clear step across the floor",
        ),
        (
            [0.0, 0.0, FLOOR],
            [0.0, 0.0, 0.0],
            FL_MONSTER | FL_ONGROUND,
            0.0,
            "zero-length step",
        ),
        (
            [80.0, 0.0, FLOOR],
            [24.0, 0.0, 0.0],
            FL_MONSTER | FL_ONGROUND,
            0.0,
            "step into the solid obstacle (edict 4)",
        ),
        (
            [0.0, 0.0, FLOOR],
            [-400.0, 0.0, 0.0],
            FL_MONSTER | FL_ONGROUND,
            0.0,
            "step long enough to hit the room wall",
        ),
        (
            [0.0, 0.0, -100.0],
            [24.0, 0.0, 0.0],
            FL_MONSTER,
            0.0,
            "walked off an edge: trace.fraction == 1, no FL_PARTIALGROUND",
        ),
        (
            [0.0, 0.0, -100.0],
            [24.0, 0.0, 0.0],
            FL_MONSTER | FL_PARTIALGROUND,
            0.0,
            "fall down: trace.fraction == 1 with FL_PARTIALGROUND",
        ),
        (
            [40.0, 56.0, -146.0],
            [8.0, 0.0, 0.0],
            FL_MONSTER | FL_PARTIALGROUND,
            0.0,
            "CheckBottom fails with FL_PARTIALGROUND (correcting)",
        ),
        (
            [40.0, 56.0, -146.0],
            [8.0, 0.0, 0.0],
            FL_MONSTER,
            0.0,
            "CheckBottom fails without FL_PARTIALGROUND (origin restored)",
        ),
        (
            [-160.0, -160.0, -140.0],
            [24.0, 0.0, 0.0],
            FL_MONSTER | FL_FLY,
            0.0,
            "flying monster, vertical-motion pass",
        ),
        (
            [-160.0, -160.0, -140.0],
            [24.0, 0.0, 0.0],
            FL_MONSTER | FL_SWIM,
            0.0,
            "swimming monster inside the water box",
        ),
        (
            [-160.0, -160.0, -140.0],
            [400.0, 0.0, 0.0],
            FL_MONSTER | FL_SWIM,
            0.0,
            "swim monster leaving water (returns false)",
        ),
        (
            [64.0, 64.0, 100.0],
            [0.0, 0.0, -8.0],
            FL_MONSTER,
            0.0,
            "start inside the pillar (allsolid / startsolid retry)",
        ),
    ]
}

#[test]
fn movestep_matches_without_relink() {
    let _g = lock();
    run_movestep(false);
}

#[test]
fn movestep_matches_with_relink() {
    let _g = lock();
    run_movestep(true);
}

fn run_movestep(relink: bool) {
    let mut results = Vec::new();
    for (origin, mv, flags, _, what) in movestep_cases() {
        let mut specs = base_population(origin);
        specs[0] = Spec::monster(ACTOR, origin)
            .flags(flags)
            .movetype(if flags & (FL_FLY | FL_SWIM) != 0 {
                MOVETYPE_FLY
            } else {
                MOVETYPE_STEP
            })
            .refs(ENEMY, GOAL);
        let c = diff(&specs, |side| side.movestep(ACTOR, mv, relink));
        results.push((what, c.value));
    }

    assert!(
        results.iter().any(|r| r.1),
        "no SV_movestep case succeeded (relink={relink}): {results:?}"
    );
    assert!(
        results.iter().any(|r| !r.1),
        "no SV_movestep case failed (relink={relink}): {results:?}"
    );
}

#[test]
fn movestep_relink_fires_touch_functions() {
    let _g = lock();

    // A trigger straddling the destination, so the relinking form dispatches
    // a touch function and the non-relinking form does not. This is what
    // proves `relink` is actually wired through rather than ignored.
    let mut specs = base_population([0.0, 0.0, FLOOR]);
    specs.push(
        Spec::blank(5)
            .solid(1.0) // SOLID_TRIGGER
            .origin([24.0, 0.0, FLOOR])
            .bbox([-32.0, -32.0, -32.0], [32.0, 32.0, 32.0])
            .touching(KIND_LOG),
    );

    let with = diff(&specs, |side| side.movestep(ACTOR, [24.0, 0.0, 0.0], true));
    let without = diff(&specs, |side| side.movestep(ACTOR, [24.0, 0.0, 0.0], false));

    assert!(with.value, "the relinking step should have succeeded");
    assert!(
        !with.touch.is_empty(),
        "relink=true must have dispatched the trigger's touch function"
    );
    assert!(
        without.touch.is_empty(),
        "relink=false must not dispatch any touch function, got {:?}",
        without.touch
    );
}

#[test]
fn movestep_ignores_free_edicts() {
    let _g = lock();

    // A free edict is never in the areanode tree, so SV_Move must not see it.
    // Running the same blocker live and freed proves the two sides agree on
    // the skip, and that the blocker was doing something in the first place.
    let mut live = base_population([0.0, 0.0, FLOOR]);
    live.push(
        Spec::blank(5)
            .solid(SOLID_BBOX)
            .origin([24.0, 0.0, FLOOR])
            .bbox([-24.0, -24.0, -24.0], [24.0, 24.0, 40.0]),
    );
    let mut dead = live.clone();
    dead[4] = dead[4].freed();

    let blocked = diff(&live, |side| side.movestep(ACTOR, [24.0, 0.0, 0.0], true));
    let clear = diff(&dead, |side| side.movestep(ACTOR, [24.0, 0.0, 0.0], true));

    assert!(
        !blocked.value,
        "the live blocker should have stopped the step"
    );
    assert!(clear.value, "the freed blocker should have been invisible");
}

// ===========================================================================
// SV_StepDirection

#[test]
fn step_direction_matches_every_yaw() {
    let _g = lock();

    let mut results = Vec::new();
    for yaw in [0.0f32, 45.0, 90.0, 135.0, 180.0, 215.0, 270.0, 315.0] {
        for dist in [0.0f32, 8.0, 24.0, 200.0] {
            let specs = base_population([0.0, 0.0, FLOOR]);
            let c = diff(&specs, |side| side.step_direction(ACTOR, yaw, dist));
            results.push(((yaw, dist), c.value));
        }
    }
    assert!(
        results.iter().any(|r| r.1) && results.iter().any(|r| !r.1),
        "SV_StepDirection never went both ways: {results:?}"
    );
}

#[test]
fn step_direction_turn_limit_matches() {
    let _g = lock();

    // yaw_speed controls how far PF_changeyaw turns in one call, so a small
    // yaw_speed with a large requested turn is exactly the `delta > 45 &&
    // delta < 315` branch that takes the step back.
    let mut results = Vec::new();
    for yaw_speed in [1.0f32, 20.0, 90.0, 360.0] {
        for (start_yaw, target) in [
            (0.0f32, 180.0f32),
            (0.0, 90.0),
            (270.0, 45.0),
            (10.0, 350.0),
        ] {
            let mut specs = base_population([0.0, 0.0, FLOOR]);
            let mut actor = Spec::monster(ACTOR, [0.0, 0.0, FLOOR]).refs(ENEMY, GOAL);
            actor.scalars[11] = yaw_speed;
            actor.vectors[13] = start_yaw; // angles[YAW]
            specs[0] = actor;
            let c = diff(&specs, |side| side.step_direction(ACTOR, target, 24.0));
            results.push(((yaw_speed, start_yaw, target), c.value));
        }
    }
    assert!(
        results.iter().any(|r| r.1),
        "no SV_StepDirection turn case succeeded: {results:?}"
    );
}

// ===========================================================================
// SV_NewChaseDir -- every branch of the direction search

/// Enemy offsets covering all nine (deltax, deltay) sign/deadband
/// combinations, so `d[1]`/`d[2]` take each of their three values including
/// DI_NODIR, and with them the direct-route, swapped, `d[1]`, `d[2]`,
/// `olddir`, ascending-sweep, descending-sweep and turnaround branches.
fn chase_offsets() -> Vec<([f32; 3], &'static str)> {
    vec![
        ([120.0, 120.0, 0.0], "+x +y"),
        ([120.0, -120.0, 0.0], "+x -y"),
        ([-120.0, 120.0, 0.0], "-x +y"),
        ([-120.0, -120.0, 0.0], "-x -y"),
        ([120.0, 0.0, 0.0], "+x, y in deadband"),
        ([-120.0, 0.0, 0.0], "-x, y in deadband"),
        ([0.0, 120.0, 0.0], "x in deadband, +y"),
        ([0.0, -120.0, 0.0], "x in deadband, -y"),
        ([4.0, -4.0, 0.0], "both in deadband (DI_NODIR twice)"),
    ]
}

#[test]
fn new_chase_dir_matches_every_direction() {
    let _g = lock();

    for (delta, what) in chase_offsets() {
        for olddir in [0.0f32, 45.0, 90.0, 180.0, 270.0, 315.0, -1.0] {
            for dist in [8.0f32, 24.0] {
                let origin = [0.0, 0.0, FLOOR];
                let enemy_origin = [
                    origin[0] + delta[0],
                    origin[1] + delta[1],
                    origin[2] + delta[2],
                ];
                let specs = vec![
                    Spec::monster(ACTOR, origin)
                        .ideal_yaw(olddir)
                        .refs(ENEMY, GOAL),
                    Spec::monster(ENEMY, enemy_origin),
                    Spec::blank(GOAL)
                        .solid(SOLID_BBOX)
                        .origin(enemy_origin)
                        .bbox([-16.0, -16.0, -24.0], [16.0, 16.0, 32.0]),
                    Spec::blank(4)
                        .solid(SOLID_BBOX)
                        .origin([120.0, 0.0, FLOOR])
                        .bbox([-24.0, -24.0, -24.0], [24.0, 24.0, 40.0]),
                ];
                let c = diff(&specs, |side| {
                    side.new_chase_dir(ACTOR, ENEMY, dist);
                });
                assert_eq!(
                    c.edicts.len(),
                    ARENA as usize * EDICT_WORDS,
                    "snapshot truncated for {what} / olddir {olddir}"
                );
            }
        }
    }
}

#[test]
fn new_chase_dir_boxed_in_reaches_fix_check_bottom() {
    let _g = lock();

    // Walled in on all four sides one step away, and hovering so
    // SV_CheckBottom's realcheck fails: every SV_StepDirection attempt is
    // refused, so SV_NewChaseDir falls through to `actor->v.ideal_yaw =
    // olddir` and SV_FixCheckBottom. That is the only path that sets
    // FL_PARTIALGROUND, which the edict snapshot shows.
    let origin = [0.0, 0.0, -100.0];
    let wall = |num: c_int, o: [f32; 3]| {
        Spec::blank(num)
            .solid(SOLID_BSP)
            .movetype(7.0) // MOVETYPE_PUSH
            .modelindex(1.0)
            .origin(o)
            .bbox([-24.0, -24.0, -64.0], [24.0, 24.0, 64.0])
    };
    let specs = vec![
        Spec::monster(ACTOR, origin)
            .ideal_yaw(90.0)
            .refs(ENEMY, GOAL),
        Spec::monster(ENEMY, [120.0, 120.0, -100.0]),
        Spec::blank(GOAL)
            .solid(SOLID_NOT)
            .origin([120.0, 120.0, -100.0]),
        wall(4, [48.0, 0.0, -100.0]),
        wall(5, [-48.0, 0.0, -100.0]),
        wall(6, [0.0, 48.0, -100.0]),
        wall(7, [0.0, -48.0, -100.0]),
    ];

    let c = diff(&specs, |side| {
        side.new_chase_dir(ACTOR, ENEMY, 24.0);
    });
    // `flags` is word 33 of the actor's 50-word record: free, num_leafs,
    // nine vec3s (27 floats), then movetype, ideal_yaw, yaw_speed, solid,
    // flags.
    let flags_word = (ACTOR as usize) * EDICT_WORDS + 2 + 27 + 4;
    let flags = f32::from_bits(c.edicts[flags_word]) as c_int;
    assert!(
        flags & FL_PARTIALGROUND != 0,
        "the boxed-in actor should have reached SV_FixCheckBottom; flags = {flags}"
    );
}

// ===========================================================================
// SV_MoveToGoal

#[test]
fn move_to_goal_matches() {
    let _g = lock();

    let mut returns = Vec::new();
    for flags in [
        FL_MONSTER, // none of ONGROUND/FLY/SWIM: early return
        FL_MONSTER | FL_ONGROUND,
        FL_MONSTER | FL_FLY,
        FL_MONSTER | FL_SWIM,
    ] {
        for (goal_at, enemy_num, what) in [
            ([24.0f32, 0.0, FLOOR], ENEMY, "goal adjacent, enemy set"),
            (
                [24.0, 0.0, FLOOR],
                0,
                "goal adjacent, enemy is the world edict",
            ),
            ([300.0, 200.0, FLOOR], ENEMY, "goal far away"),
        ] {
            for dist in [0.0f32, 8.0, 200.0] {
                let specs = vec![
                    Spec::monster(ACTOR, [0.0, 0.0, FLOOR])
                        .flags(flags)
                        .movetype(if flags & (FL_FLY | FL_SWIM) != 0 {
                            MOVETYPE_FLY
                        } else {
                            MOVETYPE_STEP
                        })
                        .refs(enemy_num, GOAL),
                    Spec::monster(ENEMY, [200.0, 0.0, FLOOR]),
                    Spec::blank(GOAL)
                        .solid(SOLID_BBOX)
                        .origin(goal_at)
                        .bbox([-16.0, -16.0, -24.0], [16.0, 16.0, 32.0]),
                    Spec::blank(4)
                        .solid(SOLID_BBOX)
                        .origin([120.0, 0.0, FLOOR])
                        .bbox([-24.0, -24.0, -24.0], [24.0, 24.0, 40.0]),
                ];
                let c = diff(&specs, |side| {
                    // SAFETY: plain fixture setters; ACTOR indexes the arena.
                    unsafe {
                        ctest_phys_set_self(ACTOR);
                        ctest_phys_set_parm0(dist);
                    }
                    side.move_to_goal();
                });
                returns.push(((flags, what, dist), c.globals[0]));
            }
        }
    }

    assert!(
        returns.iter().any(|r| r.1 == 0f32.to_bits()),
        "no SV_MoveToGoal case took the not-on-ground early return: {returns:?}"
    );
}

// ===========================================================================
// Raise propagation (ADR-009)
//
// sv_move.c reaches Host_Error only through the world: SV_Move ->
// SV_ClipMoveToEntity -> SV_HullForEntity's Con_Warning sites, whose
// PR_GetString argument raises when the classname string_t points at a NULL
// knownstrings slot (Quake/pr_edict_arena.c:311-318). Both raise tests below
// arm exactly that, on an edict placed inside the move bounds so the trace
// has to consult it.

struct RaiseArgs {
    side: Side,
    which: c_int,
    mv: [f32; 3],
    result: u8,
}

extern "C" fn raise_call(p: *mut c_void) {
    // SAFETY: `p` is the `RaiseArgs` the caller pinned on its own stack for
    // the duration of ctest_try_host.
    let a = unsafe { &mut *p.cast::<RaiseArgs>() };
    a.result = match a.which {
        0 => u8::from(a.side.check_bottom(ACTOR)),
        1 => u8::from(a.side.movestep(ACTOR, a.mv, true)),
        _ => u8::from(a.side.step_direction(ACTOR, 0.0, 24.0)),
    };
}

/// A SOLID_BSP edict whose model is not a brush model: SV_HullForEntity
/// warns, and the warning formats PR_GetString (ent->v.classname).
fn bad_hull_edict(num: c_int, origin: [f32; 3]) -> Spec {
    Spec::blank(num)
        .solid(SOLID_BSP)
        .movetype(7.0) // MOVETYPE_PUSH
        .modelindex(2.0) // a non-brush model in the fixture
        .origin(origin)
        .bbox([-64.0, -64.0, -64.0], [64.0, 64.0, 64.0])
}

fn run_raise_case(which: c_int, origin: [f32; 3], mv: [f32; 3], what: &str) {
    let specs = vec![
        Spec::monster(ACTOR, origin).refs(ENEMY, GOAL),
        bad_hull_edict(ENEMY, [origin[0] + 16.0, origin[1], origin[2]]),
    ];

    let mut results = Vec::new();
    for side in [Side::C, Side::Rust] {
        setup(side, &specs);
        // SAFETY: installs the fixture's own knownstrings table and points
        // this edict's classname at its NULL slot.
        unsafe { ctest_world_arm_bad_classname(ENEMY) };
        // SAFETY: plain console-log reset.
        unsafe { ctest_clear_con_log() };

        let mut args = RaiseArgs {
            side,
            which,
            mv,
            result: 0,
        };
        // SAFETY: `raise_call` only touches `args`, which outlives the call.
        let raised =
            unsafe { ctest_try_host(raise_call, (&mut args as *mut RaiseArgs).cast::<c_void>()) };
        // SAFETY: the trap's message buffer is a static NUL-terminated C
        // string, only rewritten by the next Host_Error.
        let msg = unsafe {
            CStr::from_ptr(ctest_host_error_message())
                .to_string_lossy()
                .into_owned()
        };
        results.push((raised, msg, con_log()));
    }

    let (c, rust) = (&results[0], &results[1]);
    assert_eq!(
        c.0, 1,
        "the C oracle must actually raise for {what} -- otherwise this test \
         proves nothing about propagation"
    );
    assert_eq!(c.0, rust.0, "Host_Error fired on both sides ({what})");
    assert_eq!(c.1, rust.1, "Host_Error message ({what})");
    assert!(
        c.1.contains("PR_GetString"),
        "the raise came from PR_GetString, not somewhere else ({what}): {:?}",
        c.1
    );
    assert_eq!(c.2, rust.2, "console log up to the raise ({what})");
}

#[test]
fn check_bottom_raise_propagates() {
    let _g = lock();
    // Hovering, so SV_CheckBottom skips the easy-out and its realcheck
    // SV_Move sweeps the armed edict.
    run_raise_case(0, [0.0, 0.0, -100.0], [0.0; 3], "SV_CheckBottom realcheck");
}

#[test]
fn movestep_raise_propagates() {
    let _g = lock();
    run_raise_case(
        1,
        [0.0, 0.0, -100.0],
        [24.0, 0.0, 0.0],
        "SV_movestep step trace",
    );
}

#[test]
fn step_direction_raise_propagates() {
    let _g = lock();
    run_raise_case(2, [0.0, 0.0, -100.0], [0.0; 3], "SV_StepDirection");
}

// ===========================================================================
// Fixture self-checks

#[test]
fn fixture_publishes_the_server_qcvm() {
    let _g = lock();

    // ctest_phys_reset must have used vm_kind 2 (sv.qcvm). If it had not,
    // every physics test would silently run against a look-alike VM.
    // SAFETY: plain fixture setters.
    unsafe {
        ctest_world_reset(2, ARENA);
        ctest_phys_reset(ARENA, 1, 0.05, 4.5, -1);
    }
    let mut buf = [0u32; EDICT_WORDS];
    // SAFETY: `buf` has the full width the helper requires.
    let n = unsafe { ctest_phys_edict_snapshot(0, buf.as_mut_ptr(), EDICT_WORDS as c_int) };
    assert_eq!(n as usize, EDICT_WORDS);
}
