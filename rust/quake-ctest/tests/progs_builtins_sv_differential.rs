//! Differential test: the Rust `quake-capi` server-coupled QuakeC builtins
//! (`rust/quake-capi/src/progs_builtins_sv.rs`) vs the original
//! `Quake/pr_cmds.c` / `Quake/pr_ext.c` bodies. Rust migration Phase 7, M5
//! wave 1 (contract groups A, B and C).
//!
//! `pr_cmds.c` and `pr_ext.c` are deliberately **not** in `build.rs`'s
//! `C_SOURCES` (adding them is far larger than M5), so there is no
//! `c_ref_PF_*` symbol to call. The oracle is instead a set of statement-for-
//! statement C transcriptions at the bottom of `stubs/stubs.c`
//! (`ctest_cref_pf_*`). They are a real oracle rather than a second copy of
//! the port because
//!
//!   * every primitive they call is the renamed C original -- `SV_Move`,
//!     `SV_LinkEdict`, `SV_movestep`, `SV_CheckBottom`, `SV_PointContents`,
//!     `VectorNormalize`, `PR_GetString` (`include/c_ref_prelude.h`), and
//!   * their float arithmetic is evaluated by the C compiler, so the
//!     double-promotion sites this port has to preserve bit-for-bit
//!     (`(mins + maxs) * 0.5`, `yaw * M_PI * 2 / 360`, `cos (yaw) * dist`,
//!     `0.5 * (mins[j] + maxs[j])`) are compared against C's own answer, not
//!     against a Rust transcription of them (ADR-010).
//!
//! Both sides run against ONE shared fixture (`ctest_phys_*` / `ctest_world_*`
//! / `ctest_pf_*`): the M3 synthetic room republished on `sv.qcvm`, with a
//! real areanode tree, real clipping hulls and real progs bytecode for the
//! touch functions `SV_TouchLinks` dispatches. Per the `world_differential.rs`
//! idiom every test resets the fixture from scratch for each side, drives the
//! same call sequence, and snapshots everything observable: the whole progs
//! globals block as exact bit patterns (which covers `OFS_RETURN` and the
//! `trace_*` block), every edict's physics fields plus the entvars the
//! builtins write that `ctest_phys_edict_snapshot` does not carry
//! (`model`, `modelindex`, `chain`, `absmin`, `absmax`, `size`), the areanode
//! topology and per-node link order, the touch-dispatch log, and the console
//! log.
//!
//! Raise topology (ADR-009): every entry point here is status-returning on the
//! Rust side (`quake_rs_pf_*(&mut detail)`), and the C oracle is driven
//! through `ctest_cref_pf_run`, a C trampoline that arms the `Host_Error` trap
//! in a C frame. No `longjmp` crosses a Rust frame on either side. Three
//! raises are reachable: `SetMinMaxSize`'s "backwards mins/maxs",
//! `PF_setmodel`'s "no precache", and anything `SV_Move`/`SV_LinkEdict` can
//! raise underneath.

use core::ffi::{c_char, c_float, c_int, c_void, CStr};
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

// ---------------------------------------------------------------------------
// progs.h offsets and server.h enumerators the fixture spells as floats.

const OFS_RETURN: c_int = 1;
const OFS_PARM0: c_int = 4;
const OFS_PARM1: c_int = 7;
const OFS_PARM2: c_int = 10;
const OFS_PARM3: c_int = 13;
const OFS_PARM4: c_int = 16;
const OFS_PARM5: c_int = 19;

const MOVETYPE_STEP: f32 = 4.0;

const SOLID_NOT: f32 = 0.0;
const SOLID_BBOX: f32 = 2.0;
const SOLID_SLIDEBOX: f32 = 3.0;

const FL_MONSTER: c_int = 32;
const FL_ONGROUND: c_int = 512;

const DAMAGE_AIM: f32 = 2.0;

/// Where a hull-1 downward trace stops on the fixture room's floor: the brush
/// face is at z = -192, hull 1's `clipmins[2]` lifts it to -168, and
/// `SV_RecursiveHullCheck`'s `DIST_EPSILON` backs off the last 1/32 unit.
const FLOOR_Z: f32 = -167.968_75;

/// `modtype_t` in `Quake/modelgen.h`/`gl_model.h` is
/// `{mod_brush, mod_sprite, mod_alias}`.
const MOD_BRUSH: c_int = 0;
const MOD_ALIAS: c_int = 2;

/// `ctest_cref_pf_run` dispatch indices; must match `stubs.c`'s switch.
mod pf {
    pub const SETORIGIN: i32 = 0;
    pub const SETSIZE: i32 = 1;
    pub const SETMODEL: i32 = 2;
    pub const TRACELINE: i32 = 3;
    pub const TRACEBOX: i32 = 4;
    pub const FINDRADIUS: i32 = 5;
    pub const WALKMOVE: i32 = 6;
    pub const DROPTOFLOOR: i32 = 7;
    pub const CHECKBOTTOM: i32 = 8;
    pub const POINTCONTENTS: i32 = 9;
    pub const AIM: i32 = 10;
    pub const WALKPATHTOGOAL: i32 = 11;
    pub const CHECKCLIENT: i32 = 12;
    pub const CHECKPVS: i32 = 13;
}

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
    fn ctest_phys_edict_snapshot(num: c_int, out: *mut u32, max: c_int) -> c_int;

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

    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(i: c_int) -> *const c_char;
    fn ctest_host_error_message() -> *const c_char;

    // --- M5 fixture seams -------------------------------------------------
    fn ctest_pf_reset();
    fn ctest_pf_reset_checkclient();
    fn ctest_pf_add_precache(name: *const c_char, model: *mut c_void);
    fn ctest_pf_model(idx: c_int) -> *mut c_void;
    fn ctest_pf_set_model_boxes(
        model: *mut c_void,
        type_: c_int,
        mins: *const c_float,
        maxs: *const c_float,
        clipmins: *const c_float,
        clipmaxs: *const c_float,
    );
    fn ctest_pf_set_leaf_pvs(pvs: *mut u8);
    fn ctest_pf_set_point_leaf_index(idx: c_int);
    fn ctest_pf_set_developer(v: c_float);
    fn ctest_pf_set_sv_aim(v: c_float);
    fn ctest_pf_set_teamplay(v: c_float);
    fn ctest_pf_entvars_offset(name: *const c_char) -> c_int;
    fn ctest_pf_globals_offset(name: *const c_char) -> c_int;
    fn ctest_pf_edict_bits(num: c_int, float_ofs: c_int) -> u32;
    fn ctest_pf_set_edict_bits(num: c_int, float_ofs: c_int, bits: u32);
    fn ctest_pf_set_global_bits(float_ofs: c_int, bits: u32);
    fn ctest_pf_num_globals() -> c_int;
    fn ctest_pf_snapshot_globals(out: *mut u32, max: c_int) -> c_int;
    fn ctest_pf_edict_prog(num: c_int) -> c_int;

    // --- oracle (stubs.c transcriptions of pr_cmds.c / pr_ext.c) ----------
    fn ctest_cref_pf_run(which: c_int) -> c_int;

    // --- shared world entry points (per-side) -----------------------------
    fn c_ref_SV_InitBoxHull();
    fn c_ref_SV_ClearWorld();
    fn c_ref_SV_LinkEdict(ent: *mut c_void, touch_triggers: u8);
    fn SV_InitBoxHull();
    fn SV_ClearWorld();
    fn SV_LinkEdict(ent: *mut c_void, touch_triggers: u8);

    // --- port (rust/quake-capi/src/progs_builtins_sv.rs) ------------------
    fn quake_rs_pf_setorigin(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_setsize(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_setmodel(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_traceline(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_tracebox(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_findradius(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_walkmove(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_droptofloor(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_checkbottom(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_pointcontents(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_aim(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_walkpathtogoal(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_checkclient(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_checkpvs(detail: *mut c_int) -> c_int;
}

// ---------------------------------------------------------------------------
// Small typed wrappers, so the test bodies stay unsafe-free.

fn entvars_ofs(name: &CStr) -> c_int {
    // SAFETY: `name` is NUL-terminated; the helper only reads it.
    let o = unsafe { ctest_pf_entvars_offset(name.as_ptr()) };
    assert!(o >= 0, "unknown entvars field {name:?}");
    o
}

fn globals_ofs(name: &CStr) -> c_int {
    // SAFETY: `name` is NUL-terminated; the helper only reads it.
    let o = unsafe { ctest_pf_globals_offset(name.as_ptr()) };
    assert!(o >= 0, "unknown global {name:?}");
    o
}

fn set_global_f(ofs: c_int, v: f32) {
    // SAFETY: `ofs` is inside the globals block (resolved from C's own layout
    // or a progs.h OFS_ constant).
    unsafe { ctest_pf_set_global_bits(ofs, v.to_bits()) }
}

fn set_global_vec(ofs: c_int, v: [f32; 3]) {
    for (i, c) in v.iter().enumerate() {
        set_global_f(ofs + i as c_int, *c);
    }
}

fn set_global_i(ofs: c_int, v: i32) {
    // SAFETY: as `set_global_f`; an int global is the same 4-byte slot.
    unsafe { ctest_pf_set_global_bits(ofs, v as u32) }
}

fn set_edict_f(num: c_int, field: &CStr, v: f32) {
    let ofs = entvars_ofs(field);
    // SAFETY: `num` indexes the fixture arena, `ofs` came from C's layout.
    unsafe { ctest_pf_set_edict_bits(num, ofs, v.to_bits()) }
}

fn edict_prog(num: c_int) -> i32 {
    // SAFETY: `num` indexes the fixture arena.
    unsafe { ctest_pf_edict_prog(num) }
}

// ---------------------------------------------------------------------------
// Side dispatch.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    C,
    Rust,
}

/// A builtin's raise outcome, normalised across the two very different
/// reporting conventions: the oracle longjmps and `ctest_try_host` reports
/// 1, the port returns `PRBI_ERR_GUARD` with `detail == CTEST_GUARD_HOST_ERROR`
/// (ADR-009). Both leave the text in `ctest_host_error_message()`.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Outcome {
    raised: bool,
    message: String,
}

/// `PRBI_ERR_GUARD` (`Quake/pr_cmds_glue.c`'s enum, mirrored in the port).
const PRBI_ERR_GUARD: c_int = 3;
/// `CTEST_GUARD_HOST_ERROR` (`stubs.c`).
const CTEST_GUARD_HOST_ERROR: c_int = 1;

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
        // SAFETY: no arguments; each side writes only its own static hull.
        unsafe {
            match self {
                Side::C => c_ref_SV_InitBoxHull(),
                Side::Rust => SV_InitBoxHull(),
            }
        }
    }

    fn clear_world(self) {
        // SAFETY: the fixture published a qcvm with a worldmodel first.
        unsafe {
            match self {
                Side::C => c_ref_SV_ClearWorld(),
                Side::Rust => SV_ClearWorld(),
            }
        }
    }

    fn link_edict(self, num: c_int) {
        // SAFETY: `num` indexes the fixture arena; both arms are plain C
        // entry points, so a raise unwinds no Rust frame (ADR-009).
        unsafe {
            let ent = ctest_world_edict(num);
            match self {
                Side::C => c_ref_SV_LinkEdict(ent, 0),
                Side::Rust => SV_LinkEdict(ent, 0),
            }
        }
    }

    /// Runs one builtin and normalises its raise reporting.
    fn run(self, which: i32) -> Outcome {
        let raised = match self {
            Side::C => {
                // SAFETY: the trampoline arms the Host_Error trap inside a C
                // frame and dispatches on `which`.
                let r = unsafe { ctest_cref_pf_run(which) };
                assert!(r == 0 || r == 1, "oracle raised Sys_Error ({r})");
                r == 1
            }
            Side::Rust => {
                let mut detail: c_int = -1;
                // SAFETY: every port entry point takes `&detail` exactly as
                // `RUST_PF` passes it and returns a PRBI_* status.
                let status = unsafe {
                    match which {
                        pf::SETORIGIN => quake_rs_pf_setorigin(&mut detail),
                        pf::SETSIZE => quake_rs_pf_setsize(&mut detail),
                        pf::SETMODEL => quake_rs_pf_sv_setmodel(&mut detail),
                        pf::TRACELINE => quake_rs_pf_traceline(&mut detail),
                        pf::TRACEBOX => quake_rs_pf_tracebox(&mut detail),
                        pf::FINDRADIUS => quake_rs_pf_findradius(&mut detail),
                        pf::WALKMOVE => quake_rs_pf_walkmove(&mut detail),
                        pf::DROPTOFLOOR => quake_rs_pf_droptofloor(&mut detail),
                        pf::CHECKBOTTOM => quake_rs_pf_checkbottom(&mut detail),
                        pf::POINTCONTENTS => quake_rs_pf_pointcontents(&mut detail),
                        pf::AIM => quake_rs_pf_aim(&mut detail),
                        pf::WALKPATHTOGOAL => quake_rs_pf_sv_walkpathtogoal(&mut detail),
                        pf::CHECKCLIENT => quake_rs_pf_sv_checkclient(&mut detail),
                        pf::CHECKPVS => quake_rs_pf_checkpvs(&mut detail),
                        _ => unreachable!("bad builtin index {which}"),
                    }
                };
                if status == 0 {
                    false
                } else {
                    assert_eq!(status, PRBI_ERR_GUARD, "unexpected PRBI_* status");
                    assert_eq!(detail, CTEST_GUARD_HOST_ERROR, "unexpected guard detail");
                    true
                }
            }
        };

        // SAFETY: the stub returns a NUL-terminated buffer that outlives this.
        let message = unsafe { CStr::from_ptr(ctest_host_error_message()) }
            .to_string_lossy()
            .into_owned();
        Outcome {
            raised,
            message: if raised { message } else { String::new() },
        }
    }
}

// ---------------------------------------------------------------------------
// Fixture population (the `ctest_phys_edict_set` blocks, as in
// `sv_move_differential.rs`).

#[derive(Clone, Copy)]
struct Spec {
    num: c_int,
    scalars: [f32; 16],
    vectors: [f32; 18],
    touch: c_int,
    free: c_int,
}

impl Spec {
    fn blank(num: c_int) -> Self {
        let mut s = Spec {
            num,
            scalars: [0.0; 16],
            vectors: [0.0; 18],
            touch: -1,
            free: 0,
        };
        s.scalars[6] = -1.0; // groundentity: none
        s.scalars[10] = -1.0; // owner: none
        s
    }

    fn monster(num: c_int, origin: [f32; 3]) -> Self {
        let mut s = Spec::blank(num);
        s.scalars[0] = MOVETYPE_STEP;
        s.scalars[1] = SOLID_SLIDEBOX;
        s.scalars[3] = (FL_MONSTER | FL_ONGROUND) as f32;
        s.scalars[11] = 20.0; // yaw_speed
        s.scalars[13] = 100.0; // health
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

    fn health(mut self, v: f32) -> Self {
        self.scalars[13] = v;
        self
    }

    fn takedamage(mut self, v: f32) -> Self {
        self.scalars[14] = v;
        self
    }

    fn modelindex(mut self, v: f32) -> Self {
        self.scalars[2] = v;
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
                -1,
                self.touch,
                -1,
                self.free,
            );
            ctest_phys_edict_set_refs(self.num, -1, -1, 0);
        }
    }
}

const ARENA: c_int = 16;

/// `ctest_phys_set_cvars`' 13-float block; the builtins read none of these
/// directly, they reach `SV_Move`/`SV_movestep` through them.
const CVARS: [f32; 13] = [
    4.0, 100.0, 800.0, 2000.0, 0.0, 0.0, 0.0, 1.0, 3.0, 1.0, 1.0, 1.0, 0.0,
];

/// The PVS byte the fixture hands `Mod_LeafPVS`. Bit 0 covers leaf index 1
/// (`l = (leaf - leafs) - 1`), which is what `ctest_pf_set_point_leaf_index(2)`
/// selects.
static mut PVS_ALL: [u8; 8] = [0xff; 8];
static mut PVS_NONE: [u8; 8] = [0x00; 8];

struct Fixture {
    maxclients: c_int,
    vmtime: f64,
    developer: f32,
    sv_aim: f32,
    teamplay: f32,
}

impl Default for Fixture {
    fn default() -> Self {
        Fixture {
            maxclients: 1,
            vmtime: 4.5,
            developer: 0.0,
            sv_aim: 1.0,
            teamplay: 0.0,
        }
    }
}

/// Rebuilds the whole fixture on `sv.qcvm` and brings `side` up to a linked
/// world.
fn setup(side: Side, fx: &Fixture, specs: &[Spec]) {
    // SAFETY: plain fixture setters; the file mutex serializes all callers.
    unsafe {
        ctest_phys_reset(ARENA, fx.maxclients, 0.05, fx.vmtime, -1);
        ctest_phys_set_cvars(CVARS.as_ptr());
        ctest_world_set_cvars(0.0, 0.0, 1.0);
        ctest_pf_reset();
        ctest_pf_reset_checkclient();
        ctest_pf_set_developer(fx.developer);
        ctest_pf_set_sv_aim(fx.sv_aim);
        ctest_pf_set_teamplay(fx.teamplay);
    }
    side.install_link_fns();
    side.init_box_hull();
    side.clear_world();
    for s in specs {
        s.apply();
        if s.free == 0 {
            side.link_edict(s.num);
        }
    }
}

// ---------------------------------------------------------------------------
// Snapshots

const EDICT_WORDS: usize = 50;

/// The entvars `ctest_phys_edict_snapshot` does not carry but these builtins
/// write or chain through.
const EXTRA_FIELDS: [&CStr; 6] = [
    c"model",
    c"modelindex",
    c"chain",
    c"absmin",
    c"absmax",
    c"size",
];

fn snapshot_edicts() -> Vec<u32> {
    let mut out = Vec::new();
    for i in 0..ARENA {
        let mut buf = [0u32; EDICT_WORDS];
        // SAFETY: `i` is inside the arena; `buf` has the full width.
        let n = unsafe { ctest_phys_edict_snapshot(i, buf.as_mut_ptr(), EDICT_WORDS as c_int) };
        assert_eq!(
            n as usize, EDICT_WORDS,
            "ctest_phys_edict_snapshot width changed; update EDICT_WORDS"
        );
        out.extend_from_slice(&buf);
        for f in EXTRA_FIELDS {
            let ofs = entvars_ofs(f);
            // vectors read three slots; scalars read one and ignore the rest
            // (the fields are contiguous, so over-reading a scalar's two
            // neighbours is harmless and keeps the shape uniform).
            for k in 0..3 {
                // SAFETY: `i` indexes the arena; `ofs + k` stays inside
                // entvars_t (every field in EXTRA_FIELDS has at least three
                // float slots after it).
                out.push(unsafe { ctest_pf_edict_bits(i, ofs + k) });
            }
        }
    }
    out
}

fn snapshot_globals() -> Vec<u32> {
    // SAFETY: plain counter read.
    let n = unsafe { ctest_pf_num_globals() } as usize;
    let mut buf = vec![0u32; n];
    // SAFETY: `buf` has exactly `numglobals` slots and the helper caps.
    let got = unsafe { ctest_pf_snapshot_globals(buf.as_mut_ptr(), n as c_int) };
    assert_eq!(got as usize, n);
    buf
}

fn snapshot_areanodes() -> Vec<i32> {
    let mut buf = vec![0i32; 5 * 1024];
    let len = buf.len() as c_int;
    // SAFETY: `buf` is sized for the whole AREA_NODES array; helper caps.
    let n = unsafe { ctest_world_snapshot_areanodes(buf.as_mut_ptr(), len) };
    buf.truncate(n as usize);
    buf
}

fn snapshot_links() -> Vec<i32> {
    let mut buf = vec![0i32; 4 * 1024];
    let len = buf.len() as c_int;
    // SAFETY: `buf` is sized above the fixture's edict count; helper caps.
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
            // SAFETY: `i < n`; NUL-terminated buffer outlives the borrow.
            unsafe { CStr::from_ptr(ctest_con_log_get(i)) }
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

#[derive(PartialEq, Debug)]
struct Snap {
    outcome: Outcome,
    edicts: Vec<u32>,
    globals: Vec<u32>,
    areanodes: Vec<i32>,
    links: Vec<i32>,
    touch: Vec<(i32, i32, u32, i32)>,
    con: Vec<String>,
}

fn capture(outcome: Outcome) -> Snap {
    Snap {
        outcome,
        edicts: snapshot_edicts(),
        globals: snapshot_globals(),
        areanodes: snapshot_areanodes(),
        links: snapshot_links(),
        touch: touch_log(),
        con: con_log(),
    }
}

/// Runs `body` once per side against a freshly reset fixture and asserts the
/// two snapshots are identical. Returns the shared snapshot so a test can add
/// its own absolute assertions on top of the differential (a differential
/// alone cannot catch "both sides did nothing").
fn differential(fx: &Fixture, specs: &[Spec], body: impl Fn(Side) -> Outcome) -> Snap {
    let c = {
        setup(Side::C, fx, specs);
        let o = body(Side::C);
        capture(o)
    };
    let r = {
        setup(Side::Rust, fx, specs);
        let o = body(Side::Rust);
        capture(o)
    };

    assert_eq!(c.outcome, r.outcome, "raise/return mismatch");
    assert_eq!(c.con, r.con, "console output mismatch");
    assert_eq!(c.globals, r.globals, "progs globals mismatch");
    assert_eq!(c.edicts, r.edicts, "edict state mismatch");
    assert_eq!(c.areanodes, r.areanodes, "areanode topology mismatch");
    assert_eq!(c.links, r.links, "areanode link order mismatch");
    assert_eq!(c.touch, r.touch, "touch dispatch mismatch");
    c
}

fn ret_f(snap: &Snap) -> f32 {
    f32::from_bits(snap.globals[OFS_RETURN as usize])
}

fn ret_vec(snap: &Snap) -> [f32; 3] {
    let i = OFS_RETURN as usize;
    [
        f32::from_bits(snap.globals[i]),
        f32::from_bits(snap.globals[i + 1]),
        f32::from_bits(snap.globals[i + 2]),
    ]
}

fn global_f(snap: &Snap, name: &CStr) -> f32 {
    f32::from_bits(snap.globals[globals_ofs(name) as usize])
}

fn edict_field(num: c_int, name: &CStr) -> f32 {
    edict_field_at(num, name, 0)
}

/// An `int`-typed entvar, read without the float conversion. `chain`
/// (`Quake/progdefs.h:102`) is `int`, so `ent->v.chain = EDICT_TO_PROG (...)`
/// stores the prog offset as a raw bit pattern, not as `(float)offset`.
fn edict_field_i(num: c_int, name: &CStr) -> i32 {
    // SAFETY: `num` indexes the arena; the offset came from C's own layout.
    unsafe { ctest_pf_edict_bits(num, entvars_ofs(name)) as i32 }
}

/// Component `k` of a vector entvar (`origin[2]`, `absmax[1]`, ...).
fn edict_field_at(num: c_int, name: &CStr, k: c_int) -> f32 {
    // SAFETY: `num` indexes the arena; the offset came from C's own layout
    // and `k` stays inside the named vector field.
    f32::from_bits(unsafe { ctest_pf_edict_bits(num, entvars_ofs(name) + k) })
}

// ===========================================================================
// Group A -- link
// ===========================================================================

#[test]
fn setorigin_moves_and_relinks() {
    let _g = lock();
    let specs = [Spec::monster(1, [0.0, 0.0, 0.0])];
    let snap = differential(&Fixture::default(), &specs, |side| {
        set_global_i(OFS_PARM0, edict_prog(1));
        set_global_vec(OFS_PARM1, [200.0, -100.0, 40.0]);
        side.run(pf::SETORIGIN)
    });

    assert!(!snap.outcome.raised);
    // SV_LinkEdict, not PF_setorigin, is what writes absmin/absmax
    // (pr_cmds.c:227-235 sets only origin), so the move must be visible there.
    assert_eq!(edict_field(1, c"origin"), 200.0);
    assert_eq!(edict_field(1, c"absmin"), 200.0 - 16.0 - 1.0);
    assert!(!snap.links.is_empty(), "the edict was never linked");
}

#[test]
fn setsize_writes_mins_maxs_size_then_links() {
    let _g = lock();
    let specs = [Spec::monster(1, [0.0, 0.0, 0.0])];
    let snap = differential(&Fixture::default(), &specs, |side| {
        set_global_i(OFS_PARM0, edict_prog(1));
        set_global_vec(OFS_PARM1, [-24.0, -24.0, -32.0]);
        set_global_vec(OFS_PARM2, [24.0, 24.0, 64.0]);
        side.run(pf::SETSIZE)
    });

    assert!(!snap.outcome.raised);
    assert_eq!(edict_field(1, c"mins"), -24.0);
    assert_eq!(edict_field(1, c"maxs"), 24.0);
    assert_eq!(edict_field(1, c"size"), 48.0);
    // absmin/absmax come from SV_LinkEdict, which SetMinMaxSize calls last.
    assert_eq!(edict_field(1, c"absmin"), -24.0 - 1.0);
    assert_eq!(edict_field(1, c"absmax"), 24.0 + 1.0);
}

#[test]
fn setsize_backwards_mins_maxs_raises() {
    let _g = lock();
    let specs = [Spec::monster(1, [0.0, 0.0, 0.0])];
    let snap = differential(&Fixture::default(), &specs, |side| {
        set_global_i(OFS_PARM0, edict_prog(1));
        set_global_vec(OFS_PARM1, [-16.0, 40.0, -24.0]); // y: min > max
        set_global_vec(OFS_PARM2, [16.0, 16.0, 32.0]);
        side.run(pf::SETSIZE)
    });

    assert!(snap.outcome.raised, "backwards mins/maxs must raise");
    assert_eq!(snap.outcome.message, "backwards mins/maxs");
    // the raise happens before any field is written (pr_cmds.c:247-249)
    assert_eq!(edict_field(1, c"mins"), -16.0);
    assert_eq!(edict_field(1, c"maxs"), 16.0);
}

#[test]
fn setmodel_brush_uses_clipbox_alias_uses_modelbox() {
    let _g = lock();
    let name = c"ctest_ent"; // the fixture's string 1
    let specs = [Spec::monster(1, [0.0, 0.0, 0.0])];

    for (kind, want_min, want_max) in [
        (MOD_BRUSH, -48.0f32, 48.0f32), // clipmins/clipmaxs
        (MOD_ALIAS, -8.0f32, 8.0f32),   // mins/maxs
    ] {
        let snap = differential(&Fixture::default(), &specs, |side| {
            // SAFETY: fixture setters; `name` outlives the call and the
            // precache table stores the pointer, exactly as sv.model_precache
            // does.
            unsafe {
                let m = ctest_pf_model(0);
                let mins = [-8.0f32, -8.0, -8.0];
                let maxs = [8.0f32, 8.0, 8.0];
                let cmins = [-48.0f32, -48.0, -48.0];
                let cmaxs = [48.0f32, 48.0, 48.0];
                ctest_pf_set_model_boxes(
                    m,
                    kind,
                    mins.as_ptr(),
                    maxs.as_ptr(),
                    cmins.as_ptr(),
                    cmaxs.as_ptr(),
                );
                ctest_pf_add_precache(name.as_ptr(), m);
            }
            set_global_i(OFS_PARM0, edict_prog(1));
            set_global_i(OFS_PARM1, 1); // string offset of "ctest_ent"
            side.run(pf::SETMODEL)
        });

        assert!(!snap.outcome.raised);
        assert_eq!(edict_field(1, c"modelindex"), 0.0);
        assert_eq!(edict_field(1, c"mins"), want_min);
        assert_eq!(edict_field(1, c"maxs"), want_max);
    }
}

#[test]
fn setmodel_null_model_zeroes_the_box() {
    let _g = lock();
    let name = c"ctest_ent";
    let specs = [Spec::monster(1, [0.0, 0.0, 0.0])];
    let snap = differential(&Fixture::default(), &specs, |side| {
        // SAFETY: a precache entry whose model pointer is NULL, which is what
        // sv.models[] holds for a non-model precache slot.
        unsafe { ctest_pf_add_precache(name.as_ptr(), core::ptr::null_mut()) };
        set_global_i(OFS_PARM0, edict_prog(1));
        set_global_i(OFS_PARM1, 1);
        side.run(pf::SETMODEL)
    });

    assert!(!snap.outcome.raised);
    assert_eq!(edict_field(1, c"mins"), 0.0);
    assert_eq!(edict_field(1, c"maxs"), 0.0);
    assert_eq!(edict_field(1, c"size"), 0.0);
}

#[test]
fn setmodel_not_precached_raises() {
    let _g = lock();
    let other = c"something_else";
    let specs = [Spec::monster(1, [0.0, 0.0, 0.0])];
    let snap = differential(&Fixture::default(), &specs, |side| {
        // SAFETY: the table gets one entry that does not match string 1.
        unsafe { ctest_pf_add_precache(other.as_ptr(), core::ptr::null_mut()) };
        set_global_i(OFS_PARM0, edict_prog(1));
        set_global_i(OFS_PARM1, 1); // "ctest_ent"
        side.run(pf::SETMODEL)
    });

    assert!(snap.outcome.raised);
    assert_eq!(snap.outcome.message, "no precache: ctest_ent");
}

// ===========================================================================
// Group B -- trace / movement
// ===========================================================================

#[test]
fn traceline_publishes_the_trace_globals() {
    let _g = lock();
    let specs = [
        Spec::monster(1, [0.0, 0.0, 0.0]),
        Spec::blank(2)
            .solid(SOLID_BBOX)
            .origin([200.0, 0.0, 0.0])
            .bbox([-32.0, -32.0, -32.0], [32.0, 32.0, 32.0]),
    ];
    let snap = differential(&Fixture::default(), &specs, |side| {
        set_global_vec(OFS_PARM0, [0.0, 0.0, 0.0]);
        set_global_vec(OFS_PARM1, [400.0, 0.0, 0.0]);
        set_global_f(OFS_PARM2, 0.0); // nomonsters
        set_global_i(OFS_PARM3, edict_prog(1));
        side.run(pf::TRACELINE)
    });

    assert!(!snap.outcome.raised);
    let frac = global_f(&snap, c"trace_fraction");
    assert!(
        frac > 0.0 && frac < 1.0,
        "the trace must hit edict 2: {frac}"
    );
    assert_eq!(
        snap.globals[globals_ofs(c"trace_ent") as usize] as i32,
        edict_prog(2)
    );
}

#[test]
fn traceline_nan_is_clamped_and_warned() {
    let _g = lock();
    let fx = Fixture {
        developer: 1.0,
        ..Fixture::default()
    };
    let specs = [Spec::monster(1, [0.0, 0.0, 0.0])];
    let snap = differential(&fx, &specs, |side| {
        set_global_vec(OFS_PARM0, [f32::NAN, 0.0, 0.0]);
        set_global_vec(OFS_PARM1, [0.0, 0.0, -400.0]);
        set_global_f(OFS_PARM2, 0.0);
        set_global_i(OFS_PARM3, edict_prog(1));
        side.run(pf::TRACELINE)
    });

    assert!(!snap.outcome.raised);
    assert_eq!(
        snap.con.len(),
        1,
        "developer 1 must warn once: {:?}",
        snap.con
    );
    assert!(
        snap.con[0].contains("NAN in traceline"),
        "unexpected warning: {}",
        snap.con[0]
    );
    // the clamp writes through the globals slot itself (pr_cmds.c:754-757)
    assert_eq!(f32::from_bits(snap.globals[OFS_PARM0 as usize]), 0.0);
}

#[test]
fn traceline_nan_without_developer_is_silent() {
    let _g = lock();
    let specs = [Spec::monster(1, [0.0, 0.0, 0.0])];
    let snap = differential(&Fixture::default(), &specs, |side| {
        set_global_vec(OFS_PARM0, [f32::NAN, 0.0, 0.0]);
        set_global_vec(OFS_PARM1, [0.0, 0.0, -400.0]);
        set_global_f(OFS_PARM2, 0.0);
        set_global_i(OFS_PARM3, edict_prog(1));
        side.run(pf::TRACELINE)
    });

    assert!(snap.con.is_empty(), "developer 0 must stay silent");
    assert_eq!(f32::from_bits(snap.globals[OFS_PARM0 as usize]), 0.0);
}

#[test]
fn tracebox_uses_its_own_mins_maxs() {
    let _g = lock();
    let specs = [Spec::monster(1, [0.0, 0.0, 0.0])];
    let snap = differential(&Fixture::default(), &specs, |side| {
        set_global_vec(OFS_PARM0, [0.0, 0.0, 0.0]);
        set_global_vec(OFS_PARM1, [-16.0, -16.0, -24.0]);
        set_global_vec(OFS_PARM2, [16.0, 16.0, 32.0]);
        set_global_vec(OFS_PARM3, [0.0, 0.0, -400.0]);
        set_global_f(OFS_PARM4, 0.0);
        set_global_i(OFS_PARM5, edict_prog(1));
        side.run(pf::TRACEBOX)
    });

    assert!(!snap.outcome.raised);
    // the room's floor is at z = -192 and hull 1 shrinks it to -168
    let endz = f32::from_bits(snap.globals[globals_ofs(c"trace_endpos") as usize + 2]);
    assert_eq!(endz, FLOOR_Z, "tracebox did not use hull 1");
}

#[test]
fn findradius_builds_the_chain_in_reverse_order() {
    let _g = lock();
    let specs = [
        Spec::blank(1)
            .solid(SOLID_BBOX)
            .origin([10.0, 0.0, 0.0])
            .bbox([-8.0, -8.0, -8.0], [8.0, 8.0, 8.0]),
        Spec::blank(2)
            .solid(SOLID_BBOX)
            .origin([20.0, 0.0, 0.0])
            .bbox([-8.0, -8.0, -8.0], [8.0, 8.0, 8.0]),
        // out of range
        Spec::blank(3)
            .solid(SOLID_BBOX)
            .origin([400.0, 0.0, 0.0])
            .bbox([-8.0, -8.0, -8.0], [8.0, 8.0, 8.0]),
        // SOLID_NOT is skipped even though it is in range
        Spec::blank(4)
            .solid(SOLID_NOT)
            .origin([15.0, 0.0, 0.0])
            .bbox([-8.0, -8.0, -8.0], [8.0, 8.0, 8.0]),
        // free edicts are skipped
        Spec::blank(5)
            .solid(SOLID_BBOX)
            .origin([12.0, 0.0, 0.0])
            .bbox([-8.0, -8.0, -8.0], [8.0, 8.0, 8.0])
            .freed(),
    ];
    let snap = differential(&Fixture::default(), &specs, |side| {
        set_global_vec(OFS_PARM0, [0.0, 0.0, 0.0]);
        set_global_f(OFS_PARM1, 64.0);
        side.run(pf::FINDRADIUS)
    });

    assert!(!snap.outcome.raised);
    // chain head is the LAST match found (pr_cmds.c:1040-1041)
    assert_eq!(snap.globals[OFS_RETURN as usize] as i32, edict_prog(2));
    assert_eq!(edict_field_i(2, c"chain"), edict_prog(1));
    assert_eq!(edict_field_i(1, c"chain"), edict_prog(0));
    assert_eq!(edict_field_i(3, c"chain"), 0, "out of radius");
    assert_eq!(edict_field_i(4, c"chain"), 0, "SOLID_NOT is skipped");
    assert_eq!(edict_field_i(5, c"chain"), 0, "free edicts are skipped");
}

#[test]
fn walkmove_without_ground_or_flight_returns_zero() {
    let _g = lock();
    let mut m = Spec::monster(1, [0.0, 0.0, -160.0]);
    m.scalars[3] = FL_MONSTER as f32; // no FL_ONGROUND/FL_FLY/FL_SWIM
    let specs = [m];
    let snap = differential(&Fixture::default(), &specs, |side| {
        ctest_set_self(1);
        set_global_f(OFS_PARM0, 90.0);
        set_global_f(OFS_PARM1, 16.0);
        side.run(pf::WALKMOVE)
    });

    assert!(!snap.outcome.raised);
    assert_eq!(ret_f(&snap), 0.0);
    assert_eq!(edict_field(1, c"origin"), 0.0, "the monster must not move");
}

#[test]
fn walkmove_steps_and_dispatches_touch() {
    let _g = lock();
    let specs = [
        Spec::monster(1, [0.0, 0.0, -160.0]),
        // a trigger the step walks into, so SV_TouchLinks re-enters the VM
        Spec::blank(2)
            .solid(1.0) // SOLID_TRIGGER
            .origin([32.0, 0.0, -160.0])
            .bbox([-32.0, -32.0, -32.0], [32.0, 32.0, 32.0])
            .touching(0),
    ];
    let snap = differential(&Fixture::default(), &specs, |side| {
        ctest_set_self(1);
        set_global_f(OFS_PARM0, 0.0); // due east
        set_global_f(OFS_PARM1, 24.0);
        side.run(pf::WALKMOVE)
    });

    assert!(!snap.outcome.raised);
    assert_eq!(ret_f(&snap), 1.0, "the step should succeed");
    assert!(
        edict_field(1, c"origin") > 0.0,
        "the monster should have moved east"
    );
    assert!(
        !snap.touch.is_empty(),
        "the trigger should have been dispatched"
    );
}

#[test]
fn droptofloor_lands_on_the_room_floor() {
    let _g = lock();
    // FL_ONGROUND has to start clear, otherwise "must be set" below would pass
    // whether or not the builtin sets it. PF_droptofloor never reads flags.
    let mut m = Spec::monster(1, [0.0, 0.0, -100.0]);
    m.scalars[3] = FL_MONSTER as f32;
    let specs = [m];
    let snap = differential(&Fixture::default(), &specs, |side| {
        ctest_set_self(1);
        side.run(pf::DROPTOFLOOR)
    });

    assert!(!snap.outcome.raised);
    assert_eq!(ret_f(&snap), 1.0);
    assert_eq!(edict_field_at(1, c"origin", 2), FLOOR_Z);
    assert_ne!(
        (edict_field(1, c"flags") as i32) & FL_ONGROUND,
        0,
        "FL_ONGROUND must be set"
    );
}

#[test]
fn droptofloor_with_nothing_below_returns_zero() {
    let _g = lock();
    // starting inside the floor: the trace comes back allsolid. FL_ONGROUND
    // has to start clear, otherwise "must not be set" would pass vacuously.
    let mut m = Spec::monster(1, [0.0, 0.0, -190.0]);
    m.scalars[3] = FL_MONSTER as f32;
    let specs = [m];
    let snap = differential(&Fixture::default(), &specs, |side| {
        ctest_set_self(1);
        side.run(pf::DROPTOFLOOR)
    });

    assert!(!snap.outcome.raised);
    assert_eq!(ret_f(&snap), 0.0);
    assert_eq!(
        (edict_field(1, c"flags") as i32) & FL_ONGROUND,
        0,
        "FL_ONGROUND must not be set on failure"
    );
}

#[test]
fn checkbottom_matches_sv_checkbottom() {
    let _g = lock();
    for (z, _label) in [(-168.0f32, "on the floor"), (0.0f32, "in mid air")] {
        let specs = [Spec::monster(1, [0.0, 0.0, z])];
        let snap = differential(&Fixture::default(), &specs, |side| {
            set_global_i(OFS_PARM0, edict_prog(1));
            side.run(pf::CHECKBOTTOM)
        });
        assert!(!snap.outcome.raised);
        let want = if z < -100.0 { 1.0 } else { 0.0 };
        assert_eq!(ret_f(&snap), want, "checkbottom at z={z}");
    }
}

#[test]
fn pointcontents_reads_the_world_hull() {
    let _g = lock();
    // CONTENTS_EMPTY -1, CONTENTS_SOLID -2, CONTENTS_WATER -3
    for (p, want) in [
        ([0.0f32, 0.0, 0.0], -1.0f32),
        ([64.0f32, 64.0, 0.0], -2.0f32),
        ([-160.0f32, -160.0, -160.0], -3.0f32),
    ] {
        let specs = [Spec::monster(1, [0.0, 0.0, 0.0])];
        let snap = differential(&Fixture::default(), &specs, |side| {
            set_global_vec(OFS_PARM0, p);
            side.run(pf::POINTCONTENTS)
        });
        assert!(!snap.outcome.raised);
        assert_eq!(ret_f(&snap), want, "pointcontents at {p:?}");
    }
}

#[test]
fn aim_straight_ahead_returns_v_forward() {
    let _g = lock();
    let specs = [
        Spec::monster(1, [0.0, 0.0, 0.0]),
        Spec::blank(2)
            .solid(SOLID_BBOX)
            .origin([200.0, 0.0, 20.0])
            .bbox([-16.0, -16.0, -24.0], [16.0, 16.0, 32.0])
            .takedamage(DAMAGE_AIM)
            .health(100.0),
    ];
    let snap = differential(&Fixture::default(), &specs, |side| {
        set_global_i(OFS_PARM0, edict_prog(1));
        set_global_f(OFS_PARM1, 1000.0);
        set_global_vec(globals_ofs(c"v_forward"), [1.0, 0.0, 0.0]);
        side.run(pf::AIM)
    });

    assert!(!snap.outcome.raised);
    assert_eq!(ret_vec(&snap), [1.0, 0.0, 0.0]);
}

#[test]
fn aim_scans_for_an_off_axis_target() {
    let _g = lock();
    let specs = [
        Spec::monster(1, [0.0, 0.0, 0.0]),
        Spec::blank(2)
            .solid(SOLID_BBOX)
            .origin([200.0, 24.0, 20.0])
            .bbox([-16.0, -16.0, -24.0], [16.0, 16.0, 32.0])
            .takedamage(DAMAGE_AIM)
            .health(100.0),
    ];
    let fx = Fixture {
        sv_aim: 0.93,
        ..Fixture::default()
    };
    let snap = differential(&fx, &specs, |side| {
        set_global_i(OFS_PARM0, edict_prog(1));
        set_global_f(OFS_PARM1, 1000.0);
        set_global_vec(globals_ofs(c"v_forward"), [1.0, 0.0, 0.0]);
        side.run(pf::AIM)
    });

    assert!(!snap.outcome.raised);
    let r = ret_vec(&snap);
    // The autoaim adjustment is pitch-only: `VectorScale (v_forward, dist,
    // end); end[2] = dir[2];` -- yaw is never pulled toward the target, so
    // the y component stays 0 even though the target is off-axis in y.
    assert_eq!(r[1], 0.0, "autoaim must not adjust yaw: {r:?}");
    assert!(r[2] > 0.0, "autoaim should have pitched up: {r:?}");
    assert_ne!(r, [1.0, 0.0, 0.0]);
}

#[test]
fn aim_skips_teammates_when_teamplay_is_on() {
    let _g = lock();
    let specs = [
        Spec::monster(1, [0.0, 0.0, 0.0]),
        Spec::blank(2)
            .solid(SOLID_BBOX)
            .origin([200.0, 24.0, 20.0])
            .bbox([-16.0, -16.0, -24.0], [16.0, 16.0, 32.0])
            .takedamage(DAMAGE_AIM)
            .health(100.0),
    ];
    let fx = Fixture {
        sv_aim: 0.93,
        teamplay: 1.0,
        ..Fixture::default()
    };
    let snap = differential(&fx, &specs, |side| {
        set_edict_f(1, c"team", 1.0);
        set_edict_f(2, c"team", 1.0);
        set_global_i(OFS_PARM0, edict_prog(1));
        set_global_f(OFS_PARM1, 1000.0);
        set_global_vec(globals_ofs(c"v_forward"), [1.0, 0.0, 0.0]);
        side.run(pf::AIM)
    });

    assert!(!snap.outcome.raised);
    assert_eq!(
        ret_vec(&snap),
        [1.0, 0.0, 0.0],
        "a teammate must not be aimed at"
    );
}

#[test]
fn walkpathtogoal_is_path_error() {
    let _g = lock();
    let specs = [Spec::monster(1, [0.0, 0.0, 0.0])];
    let snap = differential(&Fixture::default(), &specs, |side| {
        set_global_f(OFS_RETURN, 12345.0);
        side.run(pf::WALKPATHTOGOAL)
    });
    assert!(!snap.outcome.raised);
    assert_eq!(ret_f(&snap), 0.0);
}

// ===========================================================================
// Group C -- PVS
// ===========================================================================

#[test]
fn checkclient_returns_the_visible_client() {
    let _g = lock();
    let fx = Fixture {
        maxclients: 2,
        ..Fixture::default()
    };
    let specs = [
        Spec::monster(1, [0.0, 0.0, 0.0]).health(100.0),
        Spec::monster(2, [100.0, 0.0, 0.0]).health(100.0),
    ];
    let snap = differential(&fx, &specs, |side| {
        // SAFETY: `PVS_ALL` is a live static for the whole test and the leaf
        // index is inside the fixture model's five leafs.
        unsafe {
            ctest_pf_set_leaf_pvs(core::ptr::addr_of_mut!(PVS_ALL).cast());
            ctest_pf_set_point_leaf_index(2);
        }
        ctest_set_self(1);
        side.run(pf::CHECKCLIENT)
    });

    assert!(!snap.outcome.raised);
    // sv.lastcheck starts at 0, so PF_newcheckclient clamps check to 1 and,
    // since 1 != maxclients, starts scanning at i = check + 1 = 2
    // (pr_cmds.c:812-819) -- the first client it can return is #2, never #1.
    assert_eq!(snap.globals[OFS_RETURN as usize] as i32, edict_prog(2));
}

#[test]
fn checkclient_returns_world_when_not_in_pvs() {
    let _g = lock();
    let fx = Fixture {
        maxclients: 2,
        ..Fixture::default()
    };
    let specs = [
        Spec::monster(1, [0.0, 0.0, 0.0]).health(100.0),
        Spec::monster(2, [100.0, 0.0, 0.0]).health(100.0),
    ];
    let snap = differential(&fx, &specs, |side| {
        // SAFETY: as above; an all-zero PVS makes every leaf invisible.
        unsafe {
            ctest_pf_set_leaf_pvs(core::ptr::addr_of_mut!(PVS_NONE).cast());
            ctest_pf_set_point_leaf_index(2);
        }
        ctest_set_self(1);
        side.run(pf::CHECKCLIENT)
    });

    assert!(!snap.outcome.raised);
    assert_eq!(snap.globals[OFS_RETURN as usize] as i32, edict_prog(0));
}

#[test]
fn checkclient_skips_a_dead_client() {
    let _g = lock();
    let fx = Fixture {
        maxclients: 2,
        ..Fixture::default()
    };
    let specs = [
        Spec::monster(1, [0.0, 0.0, 0.0]).health(100.0),
        Spec::monster(2, [100.0, 0.0, 0.0]).health(0.0),
    ];
    let snap = differential(&fx, &specs, |side| {
        // SAFETY: as above.
        unsafe {
            ctest_pf_set_leaf_pvs(core::ptr::addr_of_mut!(PVS_ALL).cast());
            ctest_pf_set_point_leaf_index(2);
        }
        ctest_set_self(1);
        side.run(pf::CHECKCLIENT)
    });

    assert!(!snap.outcome.raised);
    // the scan starts at client 2, skips it for health <= 0, wraps to 1 and
    // stops there on `i == check` -- so the answer moves off the default.
    assert_eq!(snap.globals[OFS_RETURN as usize] as i32, edict_prog(1));
}

#[test]
fn checkpvs_reports_leaf_visibility() {
    let _g = lock();
    for (pvs_all, want) in [(true, 1.0f32), (false, 0.0f32)] {
        // world.c's SV_LinkEdict only runs SV_FindTouchedLeafs when
        // `ent->v.modelindex` is non-zero, and PF_checkpvs iterates
        // `ed->num_leafs` -- without a modelindex the loop body never runs and
        // the builtin would answer false whatever the PVS says.
        let specs = [
            Spec::monster(1, [0.0, 0.0, 0.0]),
            Spec::monster(2, [100.0, 0.0, 0.0]).modelindex(1.0),
        ];
        let snap = differential(&Fixture::default(), &specs, |side| {
            // SAFETY: both statics live for the whole test.
            unsafe {
                let p = if pvs_all {
                    core::ptr::addr_of_mut!(PVS_ALL).cast()
                } else {
                    core::ptr::addr_of_mut!(PVS_NONE).cast()
                };
                ctest_pf_set_leaf_pvs(p);
                ctest_pf_set_point_leaf_index(1);
            }
            set_global_vec(OFS_PARM0, [0.0, 0.0, 0.0]);
            set_global_i(OFS_PARM1, edict_prog(2));
            side.run(pf::CHECKPVS)
        });

        assert!(!snap.outcome.raised);
        assert_eq!(ret_f(&snap), want, "checkpvs with pvs_all={pvs_all}");
    }
}

/// `pr_global_struct->self`, which `PF_walkmove` / `PF_droptofloor` /
/// `PF_checkclient` all read.
fn ctest_set_self(num: c_int) {
    // SAFETY: `num` indexes the fixture arena.
    unsafe { ctest_phys_set_self(num) }
}
