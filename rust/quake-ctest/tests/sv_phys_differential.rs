//! Differential test: the Rust `quake-capi` server-physics port vs the
//! original `Quake/sv_phys.c` (compiled as `c_ref_*`). Rust migration
//! Phase 7, M4 (task T4.1).
//!
//! Both implementations run against ONE shared fixture (`ctest_phys_*` and
//! `ctest_world_*` in `stubs/stubs.c`): the M3 synthetic room -- a brush
//! model with three real clipping hulls, a solid pillar, a water box and a
//! lava box -- republished on `sv.qcvm` (which is what makes sv_phys.c's
//! server-only half run at all: it gates the pusher support frame, the
//! sv_speeds timing, client physics and `sv_freezenonclients` on
//! `qcvm == &sv.qcvm`), plus an edict arena, an areanode tree, a client
//! array and a progs image whose think/touch/blocked functions are genuine
//! bytecode.
//!
//! Every test follows the `world_differential.rs` idiom: take the file
//! mutex, then for each side reset the fixture from scratch, drive the SAME
//! call sequence through that side's entry points, and snapshot everything
//! observable -- every entvars field physics writes as exact bit patterns,
//! the areanode topology and per-node link chain ORDER, the touch/think
//! dispatch log, the SV_StartSound log, the sv_speeds counters, the latched
//! `sv_analyticphysics_frame`, `qcvm->time`, `force_retouch` and the console
//! log. The two snapshots must be identical.
//!
//! The `sv_speeds` millisecond fields are included, and are deterministic
//! here because the harness' `Sys_DoubleTime` is a constant: any timing
//! block the port adds or drops still shows up in the counters beside them.
//!
//! ADR-009: `SV_CheckAllEnts`, `SV_CheckVelocity`, `SV_CheckWaterTransition`
//! and `SV_Physics` all reach `Host_Error` (through `PR_GetString`,
//! `SV_StartSound`, `Host_EndGame` and the QC dispatch respectively), so the
//! Rust side of each is a `quake_rs_*` status core and the re-raise happens
//! in a plain-named C wrapper -- `Quake/sv_phys_glue.c` in the engine,
//! `stubs.c` here. The tests drive the plain names on both sides, so no
//! longjmp ever unwinds a Rust frame.

use core::ffi::{c_char, c_float, c_int, c_void, CStr};
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

// ---------------------------------------------------------------------------
// server.h enumerators, as the floats entvars_t stores them in.

const MOVETYPE_NONE: f32 = 0.0;
const MOVETYPE_WALK: f32 = 3.0;
const MOVETYPE_STEP: f32 = 4.0;
const MOVETYPE_FLY: f32 = 5.0;
const MOVETYPE_TOSS: f32 = 6.0;
const MOVETYPE_PUSH: f32 = 7.0;
const MOVETYPE_NOCLIP: f32 = 8.0;
const MOVETYPE_BOUNCE: f32 = 10.0;
const MOVETYPE_GIB: f32 = 11.0;
/// Not one of `emovetype_t`: SV_Physics' final `else` calls Host_EndGame.
const MOVETYPE_BOGUS: f32 = 42.0;

const SOLID_NOT: f32 = 0.0;
const SOLID_TRIGGER: f32 = 1.0;
const SOLID_BBOX: f32 = 2.0;
const SOLID_SLIDEBOX: f32 = 3.0;
const SOLID_BSP: f32 = 4.0;

const FL_CLIENT: c_int = 8;
const FL_MONSTER: c_int = 32;
const FL_ONGROUND: c_int = 512;

const CONTENTS_EMPTY: f32 = -1.0;
const CONTENTS_WATER: f32 = -3.0;

/// `ctest_world_touch_func` kinds: 0 logs, 1 relinks, 2 frees the target,
/// 3 frees self.
const KIND_LOG: c_int = 0;
const KIND_RELINK: c_int = 1;
const KIND_FREE_SELF: c_int = 3;
/// ED_Alloc's a live pushable edict; see `ctest_world_builtin_spawn`.
const KIND_SPAWN: c_int = 4;
/// Rewrites `self`'s movetype and nextthink; see `ctest_world_builtin_setself`.
const KIND_SET_SELF: c_int = 5;

/// Hull 1's floor in the synthetic room (the open box bottoms out at
/// z = -192; the hull-1 clip box has mins[2] = -24).
const FLOOR: f32 = -168.0;

/// entvars_t float offsets, for `ctest_phys_edict_poke_bits`. Asserted
/// against the C compiler's own `offsetof` by `entvars_offsets_match`, so a
/// progdefs change breaks that test rather than silently poking a neighbour.
const OFS_ORIGIN: c_int = 10;
const OFS_VELOCITY: c_int = 16;

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
    fn ctest_phys_set_client(slot: c_int, active: c_int, knowntoqc: c_int);
    fn ctest_phys_set_prog_funcs(
        startframe_kind: c_int,
        prethink_kind: c_int,
        postthink_kind: c_int,
        force_retouch: c_float,
    );
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
    fn ctest_phys_edict_poke_bits(num: c_int, float_ofs: c_int, bits: u32);
    fn ctest_phys_edict_snapshot(num: c_int, out: *mut u32, max: c_int) -> c_int;
    fn ctest_phys_entvars_offset(name: *const c_char) -> c_int;
    fn ctest_phys_swap_arena(num_edicts: c_int);
    fn ctest_phys_vm_time() -> f64;
    fn ctest_phys_num_edicts() -> c_int;
    fn ctest_phys_force_retouch() -> c_float;
    fn ctest_phys_speeds_c(ms3: *mut f64, counts4: *mut c_int, analytic_frame: *mut c_int);
    fn ctest_phys_speeds_plain(ms3: *mut f64, counts4: *mut c_int, analytic_frame: *mut c_int);
    fn ctest_phys_sound_arm_raise(on: c_int);
    fn ctest_phys_sound_len() -> c_int;
    fn ctest_phys_sound_get(
        i: c_int,
        ent: *mut c_int,
        channel: *mut c_int,
        volume: *mut c_int,
        attenuation: *mut c_float,
        has_origin: *mut c_int,
        sample: *mut *const c_char,
    ) -> c_int;
    fn ctest_phys_endgame_calls() -> c_int;

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
    fn ctest_world_set_relink_target(num: c_int);
    fn ctest_world_set_spawn(budget: c_int, origin: *const c_float, ground: c_int);
    fn ctest_world_spawned_edict() -> c_int;
    fn ctest_world_spawn_calls() -> c_int;
    fn ctest_world_set_selfpoke(armed: c_int, movetype: c_float, think_delay: c_float);
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

    // --- oracle (Quake/sv_phys.c, renamed by include/c_ref_prelude.h) -----
    fn c_ref_SV_CheckAllEnts();
    fn c_ref_SV_CheckVelocity(ent: *mut c_void);
    fn c_ref_SV_CheckWaterTransition(ent: *mut c_void);
    fn c_ref_SV_Physics();
    fn c_ref_SV_PushGridEntityLinked(ent: *mut c_void);
    fn c_ref_SV_InitBoxHull();
    fn c_ref_SV_ClearWorld();
    fn c_ref_SV_LinkEdict(ent: *mut c_void, touch_triggers: u8);

    // --- port (rust/quake-capi/src/sv_phys.rs, plain names per ADR-009) ---
    // The four raising names are the re-raising C wrappers in stubs.c over
    // the quake_rs_sv_* status cores, mirroring Quake/sv_phys_glue.c;
    // SV_PushGridEntityLinked cannot raise and is a plain Rust export.
    fn SV_CheckAllEnts();
    fn SV_CheckVelocity(ent: *mut c_void);
    fn SV_CheckWaterTransition(ent: *mut c_void);
    fn SV_Physics();
    fn SV_PushGridEntityLinked(ent: *mut c_void);
    fn SV_InitBoxHull();
    fn SV_ClearWorld();
    fn SV_LinkEdict(ent: *mut c_void, touch_triggers: u8);
}

// ---------------------------------------------------------------------------
// Side dispatch

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    C,
    Rust,
}

impl Side {
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
        // SAFETY: `num` indexes the fixture arena; plain entry points, so a
        // raise unwinds no Rust frame (ADR-009).
        unsafe {
            let ent = ctest_world_edict(num);
            match self {
                Side::C => c_ref_SV_LinkEdict(ent, tt),
                Side::Rust => SV_LinkEdict(ent, tt),
            }
        }
    }

    fn check_all_ents(self) {
        // SAFETY: no arguments; walks the published arena.
        unsafe {
            match self {
                Side::C => c_ref_SV_CheckAllEnts(),
                Side::Rust => SV_CheckAllEnts(),
            }
        }
    }

    fn check_velocity(self, num: c_int) {
        // SAFETY: `num` indexes the fixture arena.
        unsafe {
            let ent = ctest_world_edict(num);
            match self {
                Side::C => c_ref_SV_CheckVelocity(ent),
                Side::Rust => SV_CheckVelocity(ent),
            }
        }
    }

    fn check_water_transition(self, num: c_int) {
        // SAFETY: `num` indexes the fixture arena.
        unsafe {
            let ent = ctest_world_edict(num);
            match self {
                Side::C => c_ref_SV_CheckWaterTransition(ent),
                Side::Rust => SV_CheckWaterTransition(ent),
            }
        }
    }

    fn physics(self) {
        // SAFETY: no arguments; the fixture published sv.qcvm, svs.clients
        // and host_frametime before this.
        unsafe {
            match self {
                Side::C => c_ref_SV_Physics(),
                Side::Rust => SV_Physics(),
            }
        }
    }

    fn push_grid_entity_linked(self, num: c_int) {
        // SAFETY: `num` indexes the fixture arena. Neither side can raise.
        unsafe {
            let ent = ctest_world_edict(num);
            match self {
                Side::C => c_ref_SV_PushGridEntityLinked(ent),
                Side::Rust => SV_PushGridEntityLinked(ent),
            }
        }
    }

    /// The sv_speeds counters and the latched `sv_analyticphysics_frame`,
    /// read from whichever storage this side writes: sv_phys.c's own
    /// (renamed `c_ref_*`) or the plain copies `stubs.c` owns for the port.
    fn speeds(self) -> ([u64; 3], [i32; 4], i32) {
        let mut ms = [0f64; 3];
        let mut counts = [0i32; 4];
        let mut analytic = 0i32;
        // SAFETY: all three out-params have the documented widths.
        unsafe {
            match self {
                Side::C => ctest_phys_speeds_c(ms.as_mut_ptr(), counts.as_mut_ptr(), &mut analytic),
                Side::Rust => {
                    ctest_phys_speeds_plain(ms.as_mut_ptr(), counts.as_mut_ptr(), &mut analytic)
                }
            }
        }
        (
            [ms[0].to_bits(), ms[1].to_bits(), ms[2].to_bits()],
            counts,
            analytic,
        )
    }
}

// ---------------------------------------------------------------------------
// Fixture population

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
    link: bool,
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
            link: true,
        };
        s.scalars[6] = -1.0; // groundentity: none
        s.scalars[10] = -1.0; // owner: none
        s.scalars[7] = -1.0; // nextthink: in the past, so no think fires
        s
    }

    fn movetype(mut self, v: f32) -> Self {
        self.scalars[0] = v;
        self
    }
    fn solid(mut self, v: f32) -> Self {
        self.scalars[1] = v;
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
    fn waterlevel(mut self, level: f32, kind: f32) -> Self {
        self.scalars[4] = level;
        self.scalars[5] = kind;
        self
    }
    fn ground(mut self, num: c_int) -> Self {
        self.scalars[6] = num as f32;
        self
    }
    fn nextthink(mut self, t: f32) -> Self {
        self.scalars[7] = t;
        self
    }
    fn ltime(mut self, t: f32) -> Self {
        self.scalars[8] = t;
        self
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
    fn velocity(mut self, v: [f32; 3]) -> Self {
        self.vectors[9..12].copy_from_slice(&v);
        self
    }
    fn avelocity(mut self, v: [f32; 3]) -> Self {
        self.vectors[15..18].copy_from_slice(&v);
        self
    }
    fn thinking(mut self, kind: c_int) -> Self {
        self.think = kind;
        self
    }
    fn touching(mut self, kind: c_int) -> Self {
        self.touch = kind;
        self
    }
    fn blocking(mut self, kind: c_int) -> Self {
        self.blocked = kind;
        self
    }
    fn player_bbox(self) -> Self {
        self.bbox([-16.0, -16.0, -24.0], [16.0, 16.0, 32.0])
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

const ARENA: c_int = 16;

/// The QC entry points a plain tick runs with. sv_phys.c:2009 and :2067
/// dispatch PlayerPreThink and PlayerPostThink *unconditionally* -- unlike
/// StartFrame at :2334, which is guarded -- so leaving them at 0 makes every
/// tick with an active client die in `PR_ExecuteProgram: NULL function`.
/// No shipping progs.dat leaves them unset, so the fixture does not either.
const PROG_TICK: (c_int, c_int, c_int, f32) = (-1, KIND_LOG, KIND_LOG, 0.0);
const MAXCLIENTS: c_int = 1;
const FRAMETIME: f64 = 0.05;
const VMTIME: f64 = 4.5;

/// One `ctest_phys_set_cvars` block. Named fields rather than a bare array
/// so a test can say which knob it is turning.
#[derive(Clone, Copy)]
struct Cvars {
    elevators: f32,
    fastpushmove: f32,
    pushgrid: f32,
    analyticphysics: f32,
    freezenonclients: f32,
    nostep: f32,
    maxvelocity: f32,
    gravity: f32,
    speeds: f32,
    spawnbeforethinks: f32,
    bouncedownslopes: f32,
}

const DEFAULTS: Cvars = Cvars {
    elevators: 3.0,
    fastpushmove: 1.0,
    pushgrid: 1.0,
    analyticphysics: 1.0,
    freezenonclients: 0.0,
    nostep: 0.0,
    maxvelocity: 2000.0,
    gravity: 800.0,
    speeds: 1.0,
    spawnbeforethinks: 0.0,
    bouncedownslopes: 1.0,
};

impl Cvars {
    fn block(&self) -> [f32; 13] {
        [
            4.0,   // sv_friction
            100.0, // sv_stopspeed
            self.gravity,
            self.maxvelocity,
            self.nostep,
            self.freezenonclients,
            self.spawnbeforethinks,
            self.bouncedownslopes,
            self.elevators,
            self.fastpushmove,
            self.pushgrid,
            self.analyticphysics,
            self.speeds,
        ]
    }
}

/// Rebuilds the whole fixture on `sv.qcvm` and brings `side` up to a linked
/// world. `physics_mode` < 0 leaves `qcvm->extglobals.physics_mode` NULL.
fn setup(
    side: Side,
    cv: Cvars,
    specs: &[Spec],
    physics_mode: c_int,
    prog: (c_int, c_int, c_int, f32),
) {
    // SAFETY: plain fixture setters; the file mutex serializes all callers.
    unsafe {
        ctest_phys_reset(ARENA, MAXCLIENTS, FRAMETIME, VMTIME, physics_mode);
        ctest_phys_set_cvars(cv.block().as_ptr());
        ctest_world_set_cvars(0.0, 0.0, 1.0);
        ctest_phys_set_prog_funcs(prog.0, prog.1, prog.2, prog.3);
        ctest_world_set_relink_target(1);
    }
    side.install_link_fns();
    side.init_box_hull();
    side.clear_world();
    for s in specs {
        s.apply();
        if s.free == 0 && s.link {
            side.link_edict(s.num, false);
        }
    }
}

// ---------------------------------------------------------------------------
// Snapshots

const EDICT_WORDS: usize = 50;

fn edict_snapshot(num: c_int) -> [u32; EDICT_WORDS] {
    let mut buf = [0u32; EDICT_WORDS];
    // SAFETY: `num` is inside the arena (`ctest_world_reset` allocates
    // CTEST_WORLD_SPAWN_HEADROOM spare slots above num_edicts) and `buf` has
    // the width the helper requires (it returns 0 rather than writing short).
    let n = unsafe { ctest_phys_edict_snapshot(num, buf.as_mut_ptr(), EDICT_WORDS as c_int) };
    assert_eq!(
        n as usize, EDICT_WORDS,
        "ctest_phys_edict_snapshot width changed; update EDICT_WORDS"
    );
    buf
}

/// Word indices into `edict_snapshot`'s output.
const W_ORIGIN: usize = 2;
const W_MOVETYPE: usize = 29;
const W_SENDINTERVAL: usize = 46;

fn snapshot_edicts(count: c_int) -> Vec<u32> {
    let mut out = Vec::with_capacity(count as usize * EDICT_WORDS);
    for i in 0..count {
        out.extend_from_slice(&edict_snapshot(i));
    }
    out
}

fn snapshot_areanodes() -> Vec<i32> {
    let mut buf = vec![0i32; 5 * 1024];
    let len = buf.len() as c_int;
    // SAFETY: `buf` is sized for the whole AREA_NODES array; the helper caps.
    let n = unsafe { ctest_world_snapshot_areanodes(buf.as_mut_ptr(), len) };
    buf.truncate(n as usize);
    buf
}

fn snapshot_links() -> Vec<i32> {
    let mut buf = vec![0i32; 4 * 1024];
    let len = buf.len() as c_int;
    // SAFETY: `buf` is sized above the fixture's edict count; the helper caps.
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

fn sound_log() -> Vec<(i32, i32, i32, u32, i32, String)> {
    // SAFETY: plain counter read.
    let n = unsafe { ctest_phys_sound_len() };
    let mut out = Vec::new();
    for i in 0..n {
        let (mut ent, mut chan, mut vol, mut has_origin) = (0i32, 0i32, 0i32, 0i32);
        let mut att = 0f32;
        let mut sample: *const c_char = core::ptr::null();
        // SAFETY: `i < n`; every out-param is a live local, and `sample`
        // receives a pointer into the recorder's own NUL-terminated buffer.
        let ok = unsafe {
            ctest_phys_sound_get(
                i,
                &mut ent,
                &mut chan,
                &mut vol,
                &mut att,
                &mut has_origin,
                &mut sample,
            )
        };
        if ok != 1 {
            break; // beyond the recorder's cap; the count still differs
        }
        // SAFETY: `sample` is the recorder's NUL-terminated buffer, valid
        // until the next ctest_phys_sound_clear.
        let s = unsafe { CStr::from_ptr(sample) }
            .to_string_lossy()
            .into_owned();
        out.push((ent, chan, vol, att.to_bits(), has_origin, s));
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

struct Snap<T> {
    value: T,
    edicts: Vec<u32>,
    areanodes: Vec<i32>,
    links: Vec<i32>,
    touch: Vec<(i32, i32, u32, i32)>,
    sounds: Vec<(i32, i32, i32, u32, i32, String)>,
    speeds: ([u64; 3], [i32; 4], i32),
    vm_time: u64,
    num_edicts: c_int,
    force_retouch: u32,
    endgames: c_int,
    con: Vec<String>,
}

/// (label, velocity, raw entvars pokes as (float offset, bit pattern), maxvelocity).
type PokeCase = (&'static str, [f32; 3], Vec<(c_int, u32)>, f32);

/// (label, client slot, `sv_client_move_frame_state_t` value, origin, velocity).
type ClientMoveCase = (&'static str, c_int, c_int, [f32; 3], [f32; 3]);

fn capture<T>(side: Side, value: T) -> Snap<T> {
    Snap {
        value,
        edicts: snapshot_edicts(ARENA),
        areanodes: snapshot_areanodes(),
        links: snapshot_links(),
        touch: touch_log(),
        sounds: sound_log(),
        speeds: side.speeds(),
        // SAFETY: plain scalar read off the published qcvm.
        vm_time: unsafe { ctest_phys_vm_time() }.to_bits(),
        // SAFETY: plain scalar read off the published qcvm.
        num_edicts: unsafe { ctest_phys_num_edicts() },
        // SAFETY: plain scalar read off the published qcvm.
        force_retouch: unsafe { ctest_phys_force_retouch() }.to_bits(),
        // SAFETY: plain scalar read off the fixture's Host_EndGame counter.
        endgames: unsafe { ctest_phys_endgame_calls() },
        con: con_log(),
    }
}

fn diff<T, F>(cv: Cvars, specs: &[Spec], physics_mode: c_int, body: F) -> Snap<T>
where
    T: PartialEq + core::fmt::Debug,
    F: Fn(Side) -> T,
{
    diff_prog(cv, specs, physics_mode, PROG_TICK, body)
}

fn diff_prog<T, F>(
    cv: Cvars,
    specs: &[Spec],
    physics_mode: c_int,
    prog: (c_int, c_int, c_int, f32),
    body: F,
) -> Snap<T>
where
    T: PartialEq + core::fmt::Debug,
    F: Fn(Side) -> T,
{
    setup(Side::C, cv, specs, physics_mode, prog);
    // SAFETY: plain console-log reset.
    unsafe { ctest_clear_con_log() };
    let c = capture(Side::C, body(Side::C));

    setup(Side::Rust, cv, specs, physics_mode, prog);
    // SAFETY: plain console-log reset.
    unsafe { ctest_clear_con_log() };
    let rust = capture(Side::Rust, body(Side::Rust));

    assert_eq!(c.value, rust.value, "return value");
    assert_eq!(c.edicts, rust.edicts, "per-edict entvars");
    assert_eq!(c.areanodes, rust.areanodes, "areanode tree");
    assert_eq!(c.links, rust.links, "link chain order");
    assert_eq!(c.touch, rust.touch, "QC dispatch log");
    assert_eq!(c.sounds, rust.sounds, "SV_StartSound log");
    assert_eq!(c.speeds, rust.speeds, "sv_speeds counters / analytic latch");
    assert_eq!(c.vm_time, rust.vm_time, "qcvm->time");
    assert_eq!(c.num_edicts, rust.num_edicts, "qcvm->num_edicts");
    assert_eq!(c.force_retouch, rust.force_retouch, "force_retouch");
    assert_eq!(c.endgames, rust.endgames, "Host_EndGame call count");
    assert_eq!(c.con, rust.con, "console log");
    c
}

// ===========================================================================
// A representative world: a client, a mover, a rider, some tossables and a
// couple of triggers.

const CLIENT: c_int = 1;
const PUSHER: c_int = 2;
const RIDER: c_int = 3;

fn population() -> Vec<Spec> {
    vec![
        // 1: the client, standing on the pusher
        Spec::blank(CLIENT)
            .movetype(MOVETYPE_WALK)
            .solid(SOLID_SLIDEBOX)
            .flags(FL_CLIENT | FL_ONGROUND)
            .ground(PUSHER)
            .origin([0.0, 0.0, -80.0])
            .player_bbox(),
        // 2: a brush mover heading up
        Spec::blank(PUSHER)
            .movetype(MOVETYPE_PUSH)
            .solid(SOLID_BSP)
            .modelindex(1.0)
            .ltime(VMTIME as f32)
            .origin([0.0, 0.0, -140.0])
            .bbox([-64.0, -64.0, -16.0], [64.0, 64.0, 16.0])
            .velocity([0.0, 0.0, 40.0])
            .avelocity([0.0, 30.0, 0.0])
            .nextthink(VMTIME as f32 + 0.5)
            .thinking(KIND_LOG)
            .blocking(KIND_LOG),
        // 3: a monster riding the pusher
        Spec::blank(RIDER)
            .movetype(MOVETYPE_STEP)
            .solid(SOLID_SLIDEBOX)
            .flags(FL_MONSTER | FL_ONGROUND)
            .ground(PUSHER)
            .origin([40.0, 0.0, -80.0])
            .player_bbox()
            .nextthink(VMTIME as f32 + 0.01)
            .thinking(KIND_LOG),
        // 4: a tossable in mid-air (gravity)
        Spec::blank(4)
            .movetype(MOVETYPE_TOSS)
            .solid(SOLID_BBOX)
            .origin([100.0, 100.0, 0.0])
            .bbox([-8.0, -8.0, -8.0], [8.0, 8.0, 8.0])
            .velocity([10.0, 0.0, 0.0]),
        // 5: a bouncer heading into the floor
        Spec::blank(5)
            .movetype(MOVETYPE_BOUNCE)
            .solid(SOLID_BBOX)
            .origin([-100.0, 100.0, -150.0])
            .bbox([-4.0, -4.0, -4.0], [4.0, 4.0, 4.0])
            .velocity([0.0, 0.0, -300.0]),
        // 6: a gib inside the water box
        Spec::blank(6)
            .movetype(MOVETYPE_GIB)
            .solid(SOLID_NOT)
            .origin([-160.0, -160.0, -160.0])
            .bbox([-2.0, -2.0, -2.0], [2.0, 2.0, 2.0])
            .velocity([0.0, 0.0, -50.0]),
        // 7: a flyer
        Spec::blank(7)
            .movetype(MOVETYPE_FLY)
            .solid(SOLID_SLIDEBOX)
            .origin([-100.0, -100.0, 0.0])
            .player_bbox()
            .velocity([120.0, 0.0, 0.0]),
        // 8: a noclipper
        Spec::blank(8)
            .movetype(MOVETYPE_NOCLIP)
            .solid(SOLID_NOT)
            .origin([200.0, 200.0, 0.0])
            .velocity([50.0, 50.0, 0.0])
            .avelocity([10.0, 20.0, 30.0]),
        // 9: MOVETYPE_NONE with a think due this tick
        Spec::blank(9)
            .movetype(MOVETYPE_NONE)
            .solid(SOLID_NOT)
            .origin([0.0, 200.0, 0.0])
            .nextthink(VMTIME as f32)
            .thinking(KIND_LOG),
        // 10: a trigger the movers pass through
        Spec::blank(10)
            .movetype(MOVETYPE_NONE)
            .solid(SOLID_TRIGGER)
            .origin([100.0, 100.0, -40.0])
            .bbox([-48.0, -48.0, -48.0], [48.0, 48.0, 48.0])
            .touching(KIND_LOG),
        // 11: a free slot, to prove the `ent->free` skips line up
        {
            let mut s = Spec::blank(11).movetype(MOVETYPE_TOSS).solid(SOLID_BBOX);
            s.free = 1;
            s
        },
        // 12: a stepper already on the floor with no think due, so
        // SV_Physics_Step takes its FL_ONGROUND / no-SV_RunThink path.
        // MOVETYPE_WALK is deliberately NOT used here: SV_Physics' else-if
        // chain has no arm for it, so a non-client walker is a genuine
        // `SV_Physics: bad movetype 3` -- that branch is covered on purpose
        // by physics_bad_movetype_raise_propagates instead.
        Spec::blank(12)
            .movetype(MOVETYPE_STEP)
            .solid(SOLID_SLIDEBOX)
            .flags(FL_ONGROUND)
            .origin([-200.0, 0.0, FLOOR])
            .player_bbox(),
    ]
}

// ===========================================================================
// ABI

#[test]
fn entvars_offsets_match() {
    let _g = lock();
    for (name, want) in [(c"velocity", OFS_VELOCITY), (c"origin", OFS_ORIGIN)] {
        // SAFETY: `name` is a NUL-terminated literal the helper only reads.
        let got = unsafe { ctest_phys_entvars_offset(name.as_ptr()) };
        assert_eq!(
            got, want,
            "entvars_t float offset for {name:?} moved; update the OFS_* consts"
        );
    }
}

// ===========================================================================
// SV_CheckVelocity

#[test]
fn check_velocity_matches() {
    let _g = lock();

    let nan = f32::NAN.to_bits();
    let inf = f32::INFINITY.to_bits();
    let cases: Vec<PokeCase> = vec![
        ("in range", [10.0, -10.0, 5.0], vec![], 2000.0),
        ("above +maxvelocity", [5000.0, 0.0, 0.0], vec![], 2000.0),
        ("below -maxvelocity", [0.0, -5000.0, 0.0], vec![], 2000.0),
        ("exactly at the cap", [2000.0, -2000.0, 0.0], vec![], 2000.0),
        ("maxvelocity 0", [1.0, -1.0, 0.5], vec![], 0.0),
        ("negative maxvelocity", [1.0, -1.0, 0.5], vec![], -50.0),
        (
            "NaN in velocity[1]",
            [1.0, 0.0, 0.0],
            vec![(OFS_VELOCITY + 1, nan)],
            2000.0,
        ),
        (
            "NaN in origin[2]",
            [1.0, 0.0, 0.0],
            vec![(OFS_ORIGIN + 2, nan)],
            2000.0,
        ),
        (
            "NaN in both",
            [1.0, 0.0, 0.0],
            vec![(OFS_VELOCITY, nan), (OFS_ORIGIN, nan)],
            2000.0,
        ),
        (
            "infinity is not NaN",
            [1.0, 0.0, 0.0],
            vec![(OFS_VELOCITY, inf)],
            2000.0,
        ),
    ];

    for (what, vel, pokes, maxvel) in cases {
        let mut cv = DEFAULTS;
        cv.maxvelocity = maxvel;
        let specs = vec![Spec::blank(4)
            .movetype(MOVETYPE_TOSS)
            .solid(SOLID_BBOX)
            .origin([0.0, 0.0, 0.0])
            .bbox([-8.0, -8.0, -8.0], [8.0, 8.0, 8.0])
            .velocity(vel)];
        let c = diff(cv, &specs, -1, |side| {
            for (ofs, bits) in &pokes {
                // SAFETY: `4` indexes the arena and `ofs` is a float offset
                // inside entvars_t, checked by entvars_offsets_match.
                unsafe { ctest_phys_edict_poke_bits(4, *ofs, *bits) };
            }
            side.check_velocity(4);
        });
        assert_eq!(
            c.edicts.len(),
            ARENA as usize * EDICT_WORDS,
            "snapshot truncated for {what}"
        );
    }
}

#[test]
fn check_velocity_nan_console_output_matches() {
    let _g = lock();

    let specs = vec![Spec::blank(4)
        .movetype(MOVETYPE_TOSS)
        .solid(SOLID_BBOX)
        .bbox([-8.0, -8.0, -8.0], [8.0, 8.0, 8.0])];
    let c = diff(DEFAULTS, &specs, -1, |side| {
        // SAFETY: `4` indexes the arena; both offsets are inside entvars_t.
        unsafe {
            ctest_phys_edict_poke_bits(4, OFS_VELOCITY, f32::NAN.to_bits());
            ctest_phys_edict_poke_bits(4, OFS_ORIGIN + 1, f32::NAN.to_bits());
        }
        side.check_velocity(4);
    });
    assert_eq!(
        c.con.len(),
        2,
        "both NaN warnings should have been printed, got {:?}",
        c.con
    );
}

// ===========================================================================
// SV_CheckAllEnts

#[test]
fn check_all_ents_matches() {
    let _g = lock();

    // Two entities buried inside the pillar (which SV_TestEntityPosition
    // reports as an invalid position) plus the three movetypes the loop
    // skips, so both the "skipped" and "reported" arms carry traffic.
    let specs = vec![
        Spec::blank(1)
            .movetype(MOVETYPE_STEP)
            .solid(SOLID_SLIDEBOX)
            .origin([64.0, 64.0, 0.0])
            .player_bbox(),
        Spec::blank(2)
            .movetype(MOVETYPE_PUSH)
            .solid(SOLID_BSP)
            .modelindex(1.0)
            .origin([64.0, 64.0, 0.0])
            .bbox([-16.0, -16.0, -16.0], [16.0, 16.0, 16.0]),
        Spec::blank(3)
            .movetype(MOVETYPE_NONE)
            .solid(SOLID_BBOX)
            .origin([64.0, 64.0, 0.0])
            .player_bbox(),
        Spec::blank(4)
            .movetype(MOVETYPE_NOCLIP)
            .solid(SOLID_BBOX)
            .origin([64.0, 64.0, 0.0])
            .player_bbox(),
        Spec::blank(5)
            .movetype(MOVETYPE_TOSS)
            .solid(SOLID_BBOX)
            .origin([64.0, 64.0, 0.0])
            .bbox([-8.0, -8.0, -8.0], [8.0, 8.0, 8.0]),
        Spec::blank(6)
            .movetype(MOVETYPE_WALK)
            .solid(SOLID_SLIDEBOX)
            .origin([0.0, 0.0, FLOOR])
            .player_bbox(),
        {
            let mut s = Spec::blank(7)
                .movetype(MOVETYPE_TOSS)
                .solid(SOLID_BBOX)
                .origin([64.0, 64.0, 0.0])
                .bbox([-8.0, -8.0, -8.0], [8.0, 8.0, 8.0]);
            s.free = 1;
            s
        },
    ];

    let c = diff(DEFAULTS, &specs, -1, |side| side.check_all_ents());
    assert!(
        c.con.iter().any(|l| l.contains("invalid position")),
        "no entity was reported in an invalid position, so the reporting arm \
         is untested: {:?}",
        c.con
    );
}

// ===========================================================================
// SV_CheckWaterTransition

#[test]
fn check_water_transition_matches() {
    let _g = lock();

    let cases: Vec<(&str, [f32; 3], f32, f32)> = vec![
        ("just spawned in air", [0.0, 0.0, 0.0], 0.0, 0.0),
        ("just spawned in water", [-160.0, -160.0, -160.0], 0.0, 0.0),
        ("air -> air", [0.0, 0.0, 0.0], CONTENTS_EMPTY, 0.0),
        (
            "air -> water (splash)",
            [-160.0, -160.0, -160.0],
            CONTENTS_EMPTY,
            0.0,
        ),
        (
            "water -> water",
            [-160.0, -160.0, -160.0],
            CONTENTS_WATER,
            1.0,
        ),
        (
            "water -> air (splash)",
            [0.0, 0.0, 0.0],
            CONTENTS_WATER,
            1.0,
        ),
        ("lava -> air", [-190.0, 190.0, -190.0], -5.0, 1.0),
        ("air -> lava", [-190.0, 190.0, -190.0], CONTENTS_EMPTY, 0.0),
        ("inside solid", [64.0, 64.0, 0.0], CONTENTS_EMPTY, 0.0),
    ];

    let mut splashes = 0;
    for (what, origin, watertype, waterlevel) in cases {
        let specs = vec![Spec::blank(4)
            .movetype(MOVETYPE_TOSS)
            .solid(SOLID_BBOX)
            .origin(origin)
            .bbox([-4.0, -4.0, -4.0], [4.0, 4.0, 4.0])
            .waterlevel(waterlevel, watertype)];
        let c = diff(DEFAULTS, &specs, -1, |side| side.check_water_transition(4));
        splashes += c.sounds.len();
        assert_eq!(
            c.edicts.len(),
            ARENA as usize * EDICT_WORDS,
            "snapshot truncated for {what}"
        );
    }
    assert!(
        splashes > 0,
        "no case reached SV_StartSound, so the splash arms are untested"
    );
}

// ===========================================================================
// SV_Physics -- the whole tick
//
// The gates the contract calls out, each driven over the full population so
// a divergence anywhere in the tick shows up.

fn run_tick(cv: Cvars, physics_mode: c_int, prog: (c_int, c_int, c_int, f32)) -> Snap<()> {
    let specs = population();
    diff_prog(cv, &specs, physics_mode, prog, |side| side.physics())
}

#[test]
fn physics_pusher_modes_match() {
    let _g = lock();

    // sv_gameplayfix_elevators 0..3: off, legacy nudge for clients, legacy
    // nudge for everything, robust pusher contact.
    for mode in [0.0f32, 1.0, 2.0, 3.0] {
        let mut cv = DEFAULTS;
        cv.elevators = mode;
        let c = run_tick(cv, -1, PROG_TICK);
        assert!(
            !c.touch.is_empty(),
            "elevators={mode}: the tick dispatched no QC at all, so the \
             pusher paths carried no traffic"
        );
    }
}

#[test]
fn physics_pusher_modes_move_the_rider_differently() {
    let _g = lock();

    // Non-vacuity for the loop above: mode 0 and mode 3 must not produce the
    // same rider state, or the elevators cvar is not actually being read.
    let mut off = DEFAULTS;
    off.elevators = 0.0;
    let mut robust = DEFAULTS;
    robust.elevators = 3.0;

    let a = run_tick(off, -1, PROG_TICK);
    let b = run_tick(robust, -1, PROG_TICK);
    assert_ne!(
        a.edicts, b.edicts,
        "sv_gameplayfix_elevators 0 and 3 produced identical world state; \
         this suite would not notice the cvar being ignored"
    );
}

#[test]
fn physics_fastpushmove_and_pushgrid_combinations_match() {
    let _g = lock();

    // The grid is a lookup accelerator: it may change how long a tick takes
    // and how many grid entries get counted, but never the resulting world.
    let mut worlds = Vec::new();
    for fast in [0.0f32, 1.0] {
        for grid in [0.0f32, 1.0] {
            let mut cv = DEFAULTS;
            cv.fastpushmove = fast;
            cv.pushgrid = grid;
            let c = run_tick(cv, -1, PROG_TICK);
            worlds.push(((fast, grid), c.edicts, c.touch, c.speeds));
        }
    }

    let reference = &worlds[0];
    for w in &worlds[1..] {
        assert_eq!(
            reference.1, w.1,
            "sv_fastpushmove/sv_pushgrid {:?} changed the resulting world \
             state; the grid must only change speed",
            w.0
        );
        assert_eq!(
            reference.2, w.2,
            "sv_fastpushmove/sv_pushgrid {:?} changed the QC dispatch order",
            w.0
        );
    }

    // ...and it must actually have been built in the one combination that
    // asks for it, otherwise the equality above is vacuous.
    let gridded = worlds
        .iter()
        .find(|w| w.0 == (1.0, 1.0))
        .expect("the fast+grid combination ran");
    assert!(
        gridded.3 .1[3] > 0,
        "sv_pushgrid=1 recorded no grid entries, so the grid path never ran"
    );
    // sv_speeds_grid_entries is deliberately NOT asserted to be 0 for
    // sv_pushgrid=0. sv_phys.c:2394 adds push_grid_num_entries
    // unconditionally, but PushGrid_Clear -- the only thing that resets it --
    // runs only under `if (use_push_grid)`, so with the grid off the counter
    // still carries whatever the last gridded tick left behind. That is real
    // engine behaviour and both sides reproduce it identically; the gate that
    // *is* well defined is the cache itself, which sv_fastpushmove 0 skips
    // building altogether.
    let uncached = worlds
        .iter()
        .find(|w| w.0 == (0.0, 1.0))
        .expect("the no-fastpushmove combination ran");
    assert_eq!(
        uncached.3 .1[2], 0,
        "sv_fastpushmove=0 still built the pushable cache"
    );
    let cached = worlds
        .iter()
        .find(|w| w.0 == (1.0, 0.0))
        .expect("the fast, no-grid combination ran");
    assert!(
        cached.3 .1[2] > 0,
        "sv_fastpushmove=1 built an empty pushable cache"
    );
}

#[test]
fn physics_analytic_gravity_half_step_matches() {
    let _g = lock();

    // SV_AddGravity biases the move by the average of this frame's 72Hz
    // gravity steps and SV_FinishGravity removes the bias afterwards; with
    // sv_analyticphysics 0 both collapse to a plain host_frametime step.
    let mut on = DEFAULTS;
    on.analyticphysics = 1.0;
    let mut off = DEFAULTS;
    off.analyticphysics = 0.0;

    let a = run_tick(on, -1, PROG_TICK);
    let b = run_tick(off, -1, PROG_TICK);

    assert_eq!(
        a.speeds.2, 1,
        "sv_analyticphysics 1 must latch the frame flag"
    );
    assert_eq!(
        b.speeds.2, 0,
        "sv_analyticphysics 0 must clear the frame flag"
    );
    assert_ne!(
        a.edicts, b.edicts,
        "the analytic half-step produced the same trajectory as the plain \
         one; SV_AddGravity/SV_FinishGravity are not being exercised"
    );
}

#[test]
fn physics_analytic_latch_survives_a_mid_tick_cvar_flip() {
    let _g = lock();

    // sv_analyticphysics_frame is latched once per SV_Physics, so two ticks
    // with the cvar flipped between them must show the flag following the
    // cvar exactly, and the port must write the same latch storage.
    let specs = population();
    for (first, second) in [(1.0f32, 0.0f32), (0.0, 1.0)] {
        let mut cv = DEFAULTS;
        cv.analyticphysics = first;
        let c = diff(cv, &specs, -1, |side| {
            side.physics();
            let after_first = side.speeds().2;
            let mut cv2 = DEFAULTS;
            cv2.analyticphysics = second;
            // SAFETY: plain cvar setter over a 13-float block that outlives
            // the call.
            unsafe { ctest_phys_set_cvars(cv2.block().as_ptr()) };
            side.physics();
            (after_first, side.speeds().2)
        });
        assert_eq!(
            c.value,
            (first as i32, second as i32),
            "the analytic latch did not follow the cvar across two ticks"
        );
    }
}

#[test]
fn physics_modes_match() {
    let _g = lock();

    // physics_mode: -1 leaves extglobals.physics_mode NULL (the `qcvm ==
    // &cl.qcvm ? 0 : 2` default), 0 only advances time, 1 is the DP-compat
    // thinks-only pass, 2 is the full tick.
    let mut times = Vec::new();
    for mode in [-1, 0, 1, 2] {
        let c = run_tick(DEFAULTS, mode, PROG_TICK);
        times.push((mode, c.vm_time, c.touch.len()));
    }

    let full = times.iter().find(|t| t.0 == 2).unwrap();
    let none = times.iter().find(|t| t.0 == 0).unwrap();
    let thinks = times.iter().find(|t| t.0 == 1).unwrap();
    assert_eq!(
        none.2, 0,
        "physics_mode 0 must not dispatch any QC, got {} calls",
        none.2
    );
    assert!(
        thinks.2 > 0 && full.2 > 0,
        "physics_mode 1 and 2 dispatched no QC at all: {times:?}"
    );
}

#[test]
fn physics_freezenonclients_matches() {
    let _g = lock();

    let mut frozen = DEFAULTS;
    frozen.freezenonclients = 1.0;
    let a = run_tick(DEFAULTS, -1, PROG_TICK);
    let b = run_tick(frozen, -1, PROG_TICK);
    assert_ne!(
        a.edicts, b.edicts,
        "sv_freezenonclients did not change the entity cap"
    );
}

#[test]
fn physics_nostep_matches() {
    let _g = lock();

    let mut nostep = DEFAULTS;
    nostep.nostep = 1.0;
    run_tick(nostep, -1, PROG_TICK);
}

#[test]
fn physics_startframe_and_player_thinks_match() {
    let _g = lock();

    // StartFrame, PlayerPreThink and PlayerPostThink are three distinct
    // dispatch sites with distinct self/other/time stamps.
    let c = run_tick(DEFAULTS, -1, (KIND_LOG, KIND_LOG, KIND_LOG, 0.0));
    assert!(
        c.touch.len() >= 3,
        "expected at least StartFrame + PlayerPreThink + PlayerPostThink, \
         got {:?}",
        c.touch
    );
}

#[test]
fn physics_force_retouch_matches() {
    let _g = lock();

    let c = run_tick(DEFAULTS, -1, (-1, KIND_LOG, KIND_LOG, 2.0));
    assert!(
        !c.touch.is_empty(),
        "force_retouch should have relinked every entity and fired the trigger"
    );
    assert_ne!(
        c.force_retouch,
        2.0f32.to_bits(),
        "SV_Physics must decrement force_retouch"
    );
}

#[test]
fn physics_client_slot_states_match() {
    let _g = lock();

    // SV_Physics_Client is gated on svs.clients[i-1].active, and the
    // PlayerPreThink dispatch on .knowntoqc.
    let specs = population();
    for (active, known) in [(1, 1), (1, 0), (0, 1), (0, 0)] {
        diff_prog(
            DEFAULTS,
            &specs,
            -1,
            (KIND_LOG, KIND_LOG, KIND_LOG, 0.0),
            |side| {
                // SAFETY: slot 0 is inside CTEST_PHYS_MAX_CLIENTS.
                unsafe { ctest_phys_set_client(0, active, known) };
                side.physics();
            },
        );
    }
}

#[test]
fn physics_multiple_ticks_match() {
    let _g = lock();

    // Four consecutive ticks: SV_BeginPusherSupportFrame takes its
    // frame-increment path on ticks 2..4, and the pusher support records
    // written on one tick are read on the next.
    let specs = population();
    let c = diff(DEFAULTS, &specs, -1, |side| {
        for _ in 0..4 {
            side.physics();
        }
    });
    // accumulated the same way SV_Physics does it (`qcvm->time +=
    // host_frametime`, four times); VMTIME + 4.0 * FRAMETIME is one ULP off
    let mut want = VMTIME;
    for _ in 0..4 {
        want += FRAMETIME;
    }
    assert_eq!(
        c.vm_time,
        want.to_bits(),
        "four ticks should have advanced qcvm->time four frametimes"
    );
}

#[test]
fn physics_arena_change_matches() {
    let _g = lock();

    // SV_BeginPusherSupportFrame's `sv_pusher_support_edicts != qcvm->edicts`
    // branch: run a tick, publish a genuinely different arena, rebuild the
    // world in it and run again. `ctest_phys_swap_arena` leaks the old arena
    // precisely so the new pointer cannot alias it.
    let specs = population();
    let c = diff(DEFAULTS, &specs, -1, |side| {
        side.physics();
        // SAFETY: allocates and publishes a zeroed arena of the same size;
        // every link is rebuilt immediately below, before anything reads the
        // areanode chains again.
        unsafe { ctest_phys_swap_arena(ARENA) };
        side.clear_world();
        for s in &specs {
            s.apply();
            if s.free == 0 && s.link {
                side.link_edict(s.num, false);
            }
        }
        side.physics();
    });
    assert_eq!(
        c.num_edicts, ARENA,
        "the swapped arena kept its edict count"
    );
}

#[test]
fn physics_client_move_frame_states_match() {
    let _g = lock();

    // The four sv_client_move_frame_state_t values, reached by varying what
    // the client is standing on and whether that pusher moves this frame:
    //   NONE                     -- no ground pusher at all
    //   GROUND                   -- standing on a pusher that moves
    //   AIRBORNE                 -- pusher underneath, client not on ground
    //   AIRBORNE_WORLD_VELOCITY  -- as above, with the client carrying the
    //                               pusher's velocity into the air
    let cases: Vec<ClientMoveCase> = vec![
        (
            "no ground pusher",
            -1,
            FL_CLIENT,
            [0.0, 0.0, FLOOR],
            [0.0; 3],
        ),
        (
            "grounded on a moving pusher",
            PUSHER,
            FL_CLIENT | FL_ONGROUND,
            [0.0, 0.0, -80.0],
            [0.0; 3],
        ),
        (
            "airborne above a moving pusher",
            PUSHER,
            FL_CLIENT,
            [0.0, 0.0, -60.0],
            [0.0, 0.0, 20.0],
        ),
        (
            "airborne carrying the pusher's velocity",
            PUSHER,
            FL_CLIENT,
            [0.0, 0.0, -60.0],
            [0.0, 0.0, 40.0],
        ),
        (
            "grounded on the world, not a pusher",
            0,
            FL_CLIENT | FL_ONGROUND,
            [-200.0, 0.0, FLOOR],
            [0.0; 3],
        ),
    ];

    for (what, ground, flags, origin, vel) in cases {
        let mut specs = population();
        specs[0] = Spec::blank(CLIENT)
            .movetype(MOVETYPE_WALK)
            .solid(SOLID_SLIDEBOX)
            .flags(flags)
            .ground(ground)
            .origin(origin)
            .player_bbox()
            .velocity(vel);
        let c = diff_prog(
            DEFAULTS,
            &specs,
            -1,
            (-1, KIND_LOG, KIND_LOG, 0.0),
            |side| side.physics(),
        );
        assert_eq!(
            c.edicts.len(),
            ARENA as usize * EDICT_WORDS,
            "snapshot truncated for {what}"
        );
    }
}

#[test]
fn physics_pusher_blocked_matches() {
    let _g = lock();

    // A rider wedged against the pillar so the pusher cannot complete its
    // move: SV_PushMove backs out and dispatches the pusher's `blocked`.
    let mut specs = population();
    specs[1] = Spec::blank(PUSHER)
        .movetype(MOVETYPE_PUSH)
        .solid(SOLID_BSP)
        .modelindex(1.0)
        .ltime(VMTIME as f32)
        .origin([40.0, 40.0, -120.0])
        .bbox([-48.0, -48.0, -16.0], [48.0, 48.0, 16.0])
        .velocity([0.0, 0.0, 400.0])
        .blocking(KIND_LOG);
    specs[2] = Spec::blank(RIDER)
        .movetype(MOVETYPE_STEP)
        .solid(SOLID_SLIDEBOX)
        .flags(FL_MONSTER | FL_ONGROUND)
        .ground(PUSHER)
        .origin([64.0, 64.0, -80.0])
        .player_bbox();

    let c = diff(DEFAULTS, &specs, -1, |side| side.physics());
    assert!(
        !c.touch.is_empty(),
        "the blocked pusher dispatched no QC at all"
    );
}

#[test]
fn physics_qc_side_effects_match() {
    let _g = lock();

    // The re-entrancy cases ADR-006 cares about: a think that relinks, and a
    // think that frees its own edict mid-tick. Both must leave the two sides
    // with identical arenas.
    for kind in [KIND_LOG, KIND_RELINK, KIND_FREE_SELF] {
        let mut specs = population();
        specs[2] = Spec::blank(RIDER)
            .movetype(MOVETYPE_STEP)
            .solid(SOLID_SLIDEBOX)
            .flags(FL_MONSTER | FL_ONGROUND)
            .ground(PUSHER)
            .origin([40.0, 0.0, -80.0])
            .player_bbox()
            .nextthink(VMTIME as f32)
            .thinking(kind);
        let c = diff(DEFAULTS, &specs, -1, |side| side.physics());
        assert!(
            !c.touch.is_empty(),
            "kind {kind}: the think never ran, so the side effect is untested"
        );
    }
}

#[test]
fn push_grid_entity_linked_matches_outside_a_tick() {
    let _g = lock();

    // world.c:495 calls this on every link. Outside SV_Physics the grid is
    // inactive and it must be a no-op on both sides; the assertion that
    // matters is that it stays one.
    let specs = population();
    diff(DEFAULTS, &specs, -1, |side| {
        for n in 1..ARENA {
            side.push_grid_entity_linked(n);
        }
    });
}

// ===========================================================================
// Raise propagation (ADR-009)

struct RaiseArgs {
    side: Side,
    which: c_int,
}

extern "C" fn raise_call(p: *mut c_void) {
    // SAFETY: `p` is the `RaiseArgs` the caller pinned on its own stack for
    // the duration of ctest_try_host.
    let a = unsafe { &*p.cast::<RaiseArgs>() };
    match a.which {
        0 => a.side.check_velocity(4),
        1 => a.side.check_water_transition(4),
        _ => a.side.physics(),
    }
}

/// Drives one raise scenario through both sides and asserts they agree --
/// including that the C oracle really did raise, so the comparison is not
/// between two quiet returns.
fn run_raise_case(
    which: c_int,
    cv: Cvars,
    specs: &[Spec],
    physics_mode: c_int,
    arm: impl Fn(),
    needle: &str,
    what: &str,
) {
    let mut results = Vec::new();
    for side in [Side::C, Side::Rust] {
        setup(side, cv, specs, physics_mode, PROG_TICK);
        arm();
        // SAFETY: plain console-log reset.
        unsafe { ctest_clear_con_log() };

        let mut args = RaiseArgs { side, which };
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
        results.push((raised, msg, con_log(), sound_log()));
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
        c.1.contains(needle),
        "the raise came from somewhere other than {needle} ({what}): {:?}",
        c.1
    );
    assert_eq!(c.2, rust.2, "console log up to the raise ({what})");
    assert_eq!(c.3, rust.3, "sound log up to the raise ({what})");
}

#[test]
fn check_velocity_nan_raise_propagates() {
    let _g = lock();

    // SV_CheckVelocity's NaN warning formats PR_GetString (ent->v.classname),
    // which Host_Errors on a knownstrings slot that is NULL
    // (Quake/pr_edict_arena.c:311-318).
    let specs = vec![Spec::blank(4)
        .movetype(MOVETYPE_TOSS)
        .solid(SOLID_BBOX)
        .bbox([-8.0, -8.0, -8.0], [8.0, 8.0, 8.0])];
    run_raise_case(
        0,
        DEFAULTS,
        &specs,
        -1,
        || {
            // SAFETY: installs the fixture's knownstrings table and points
            // edict 4's classname at its NULL slot, then makes velocity[0]
            // NaN so the warning is reached at all.
            unsafe {
                ctest_world_arm_bad_classname(4);
                ctest_phys_edict_poke_bits(4, OFS_VELOCITY, f32::NAN.to_bits());
            }
        },
        "PR_GetString",
        "SV_CheckVelocity NaN warning",
    );
}

#[test]
fn check_water_transition_sound_raise_propagates() {
    let _g = lock();

    // SV_StartSound Host_Errors on an unprecached sample; the fixture's
    // recorder can be armed to do the same, which is the only Host_Error
    // SV_CheckWaterTransition can reach.
    let specs = vec![Spec::blank(4)
        .movetype(MOVETYPE_TOSS)
        .solid(SOLID_BBOX)
        .origin([-160.0, -160.0, -160.0])
        .bbox([-4.0, -4.0, -4.0], [4.0, 4.0, 4.0])
        .waterlevel(0.0, CONTENTS_EMPTY)];
    run_raise_case(
        1,
        DEFAULTS,
        &specs,
        -1,
        || {
            // SAFETY: plain flag setter on the sound recorder.
            unsafe { ctest_phys_sound_arm_raise(1) };
        },
        "not precached",
        "SV_CheckWaterTransition splash",
    );
    // SAFETY: disarm, so later tests in this binary see a quiet recorder.
    unsafe { ctest_phys_sound_arm_raise(0) };
}

#[test]
fn physics_bad_movetype_raise_propagates() {
    let _g = lock();

    // SV_Physics' final else: Host_EndGame ("SV_Physics: bad movetype %i").
    let mut specs = population();
    specs[3] = Spec::blank(4)
        .movetype(MOVETYPE_BOGUS)
        .solid(SOLID_BBOX)
        .origin([100.0, 100.0, 0.0])
        .bbox([-8.0, -8.0, -8.0], [8.0, 8.0, 8.0]);
    run_raise_case(
        2,
        DEFAULTS,
        &specs,
        2,
        || {},
        "bad movetype",
        "SV_Physics bad movetype",
    );
}

// ===========================================================================
// ED_AllocSetHook: the mid-tick allocator feeding the pushable cache

/// Where `ctest_world_builtin_spawn` puts the edict it allocates. It is
/// spawned standing on PUSHER (FL_ONGROUND + groundentity), so
/// SV_EntityRidingPusher (sv_phys.c:1153) picks it up and the pusher
/// processed later in the same entity loop carries it -- but only if it made
/// it onto the pusher's candidate list, which for sv_fastpushmove 1 IS the
/// hook-fed cache.
const SPAWN_ORIGIN: [f32; 3] = [0.0, 0.0, -100.0];

type SpawnResult = (c_int, c_int, [u32; EDICT_WORDS]);

fn spawn_tick(cv: Cvars) -> Snap<SpawnResult> {
    let specs = population();
    diff_prog(cv, &specs, -1, (-1, KIND_SPAWN, KIND_LOG, 0.0), |side| {
        // SAFETY: plain fixture setter, serialized by the file mutex.
        unsafe { ctest_world_set_spawn(1, SPAWN_ORIGIN.as_ptr(), PUSHER) };
        side.physics();
        // SAFETY: plain fixture readers.
        let (num, calls) = unsafe { (ctest_world_spawned_edict(), ctest_world_spawn_calls()) };
        let snap = if num >= 0 {
            edict_snapshot(num)
        } else {
            [0u32; EDICT_WORDS]
        };
        (num, calls, snap)
    })
}

#[test]
fn mid_tick_ed_alloc_reaches_the_pushable_cache() {
    let _g = lock();

    // sv_phys.c:2397 installs SV_Physics_Alloc_Hook for the whole of the
    // entity loop, so an edict QC allocates *during* a tick is appended to
    // pushable_ent_cache and spliced onto every later grid query
    // (sv_phys.c:254). PlayerPreThink runs at entity 1 and the pusher at
    // entity 2, so the allocation lands before the pusher is processed.
    //
    // This is the only coverage that hook has. If the Rust side's
    // ED_AllocSetHook did not reach the same single hook slot c_ref_ED_Alloc
    // reads, its cache would be missing this edict, the pusher would never
    // see it, and the returned snapshot would differ from the C side's.
    // sv_gameplayfix_elevators 0: plain rider contact, no robust re-test, so
    // "standing on the pusher" is enough to be carried. The room's brush
    // model makes the robust contact test (mode 3) reject everything in this
    // fixture, riders included.
    //
    // sv_pushgrid 0 matters just as much: with the grid on there is a SECOND
    // route onto the candidate list -- SV_LinkEdict -> SV_PushGridEntityLinked
    // -> PushGrid_Insert (sv_phys.c:202) -- which would carry the new edict
    // even with a dead alloc hook. With the grid off, the hook-fed cache is
    // the only way in.
    let mut cv = DEFAULTS;
    cv.elevators = 0.0;
    cv.pushgrid = 0.0;

    let fast = spawn_tick(cv);
    let (num, calls, snap) = fast.value;

    assert_eq!(calls, 1, "PlayerPreThink never reached the spawn builtin");
    assert_eq!(num, ARENA, "ED_Alloc should have grown the arena by one");
    assert_eq!(
        fast.num_edicts,
        ARENA + 1,
        "qcvm->num_edicts should have grown"
    );

    let landed = &snap[W_ORIGIN..W_ORIGIN + 3];
    let spawned_at: Vec<u32> = SPAWN_ORIGIN.iter().map(|v| v.to_bits()).collect();
    assert_ne!(
        landed,
        &spawned_at[..],
        "the mid-tick allocated edict was never moved, so it never entered \
         the pushable cache and the ED_AllocSetHook path carried no traffic"
    );

    // The gridded control: the second route must land the edict in the same
    // place the hook-fed cache does.
    let mut grid_cv = cv;
    grid_cv.pushgrid = 1.0;
    let gridded = spawn_tick(grid_cv);
    assert_eq!(
        gridded.value.2, snap,
        "the gridded pusher disagreed with the cache-only one about the mid-tick edict"
    );

    // sv_fastpushmove 0 builds no cache at all: SV_PushMove then walks the
    // arena, which already contains the new edict. The two paths must agree,
    // which pins the hook to "keeps the cache complete" rather than merely
    // "makes the cache different".
    let mut slow_cv = cv;
    slow_cv.fastpushmove = 0.0;
    let slow = spawn_tick(slow_cv);
    assert_eq!(
        slow.value.0, num,
        "the arena grew differently without the cache"
    );
    assert_eq!(
        slow.value.2, snap,
        "the cached and uncached pushers disagreed about the mid-tick edict, \
         so the ED_AllocSetHook path is not keeping the cache complete"
    );
}

// ===========================================================================
// The two latches the port called out

#[test]
fn sendinterval_reads_the_movetype_qc_left_behind() {
    let _g = lock();

    // sv_phys.c:2444 re-reads ent->v.movetype AFTER the handler has run, so
    // a think that rewrites its own movetype changes the sendinterval
    // decision without changing which physics handler ran. Entity 4 is a
    // MOVETYPE_TOSS with a think due this tick; SV_RunThink zeroes nextthink
    // before dispatching, so the builtin restores it too.
    let mut specs = population();
    specs[3] = Spec::blank(4)
        .movetype(MOVETYPE_TOSS)
        .solid(SOLID_BBOX)
        .origin([100.0, 100.0, 0.0])
        .bbox([-8.0, -8.0, -8.0], [8.0, 8.0, 8.0])
        .velocity([10.0, 0.0, 0.0])
        .nextthink(VMTIME as f32)
        .thinking(KIND_SET_SELF);

    let mut seen = Vec::new();
    for (label, armed, movetype) in [
        ("left as MOVETYPE_TOSS", 1, MOVETYPE_TOSS),
        ("rewritten to MOVETYPE_STEP", 1, MOVETYPE_STEP),
        ("not rewritten at all", 0, MOVETYPE_TOSS),
    ] {
        let c = diff_prog(DEFAULTS, &specs, -1, PROG_TICK, |side| {
            // SAFETY: plain fixture setter, serialized by the file mutex.
            unsafe { ctest_world_set_selfpoke(armed, movetype, 0.02) };
            side.physics();
            edict_snapshot(4)
        });
        seen.push((label, c.value));
    }

    assert_eq!(
        seen[0].1[W_MOVETYPE],
        MOVETYPE_TOSS.to_bits(),
        "the builtin should have left entity 4 a MOVETYPE_TOSS"
    );
    assert_eq!(
        seen[1].1[W_MOVETYPE],
        MOVETYPE_STEP.to_bits(),
        "the builtin should have rewritten entity 4 to MOVETYPE_STEP"
    );
    assert_ne!(
        seen[0].1[W_SENDINTERVAL], seen[1].1[W_SENDINTERVAL],
        "sendinterval was the same whether QC left the movetype as TOSS ({:?}) \
         or rewrote it to STEP ({:?}), so the post-dispatch movetype re-read \
         is not being exercised",
        seen[0].0, seen[1].0
    );
    assert_eq!(
        seen[2].1[W_SENDINTERVAL], 0,
        "with the write disarmed, nextthink stays 0 and sendinterval must be \
         false ({:?})",
        seen[2].0
    );
}

#[test]
fn impact_touch_stamps_one_time_across_both_dispatches() {
    let _g = lock();

    // SV_Impact sets pr_global_struct->time ONCE and then dispatches both
    // e1->v.touch and e2->v.touch, so the port cannot reuse
    // World_Glue_CallTouch (which restamps `time` per dispatch) for it.
    // A bouncer driven into a solid box, both carrying touch functions,
    // makes SV_Impact fire; the two log records must carry the same stamp.
    let mut specs = population();
    specs[4] = Spec::blank(5)
        .movetype(MOVETYPE_BOUNCE)
        .solid(SOLID_BBOX)
        .origin([-100.0, 100.0, -60.0])
        .bbox([-4.0, -4.0, -4.0], [4.0, 4.0, 4.0])
        .velocity([0.0, 0.0, -600.0])
        .touching(KIND_LOG);
    specs.push(
        Spec::blank(13)
            .movetype(MOVETYPE_NONE)
            .solid(SOLID_BBOX)
            .origin([-100.0, 100.0, -90.0])
            .bbox([-24.0, -24.0, -8.0], [24.0, 24.0, 8.0])
            .touching(KIND_LOG),
    );

    let c = diff(DEFAULTS, &specs, -1, |side| side.physics());

    // the impact pair: two consecutive records naming 5 and 13 in either
    // order, both stamped with the same pr_global_struct->time
    let pair = c
        .touch
        .windows(2)
        .find(|w| (w[0].0, w[0].1) == (5, 13) && (w[1].0, w[1].1) == (13, 5))
        .unwrap_or_else(|| {
            panic!(
                "SV_Impact never dispatched both touch functions: {:?}",
                c.touch
            )
        });
    assert_eq!(
        pair[0].2, pair[1].2,
        "SV_Impact must stamp pr_global_struct->time once for both \
         dispatches, not once per dispatch"
    );
}
