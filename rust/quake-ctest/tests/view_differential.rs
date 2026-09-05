//! Differential/characterization gate for `Quake/view.c` -- player eye
//! positioning, screen blends and the weapon-model placement. Rust migration
//! Phase 7, M7, task T7.2a.
//!
//! The oracle half is `Quake/view.c` itself, compiled by `build.rs` with
//! `include/c_ref_prelude.h` force-included so every entry point becomes
//! `c_ref_V_*`. The plain (Rust-routed) half and the whole fixture live in
//! `stubs/view_ref.c`, which is shaped exactly like `Quake/view_glue.c`; read
//! that file's module doc for what is genuinely two-sided and what is one
//! shared object.
//!
//! ## What is compared
//!
//! Every `view.c` entry point except `V_CalcRoll` is driven through its plain
//! name (`stubs/view_ref.c` -> `rust/quake-capi/src/view.rs`) and its
//! `c_ref_` twin, and the resulting client/refdef/blend state is snapshotted
//! from both halves and compared field by field on raw bit patterns.
//!
//! `V_CalcRoll` is the exception: `stubs.c:6913-6962` already owns a plain
//! `V_CalcRoll` (a hand transcription made for the M6 `sv_user` wave, which
//! the Rust `SV_ClientThink` still calls), so defining a second one here would
//! be `LNK2005`. This suite therefore drives `quake_rs_v_calc_roll` -- the
//! actual Rust core -- directly against `c_ref_V_CalcRoll`. Report item 7
//! recommends collapsing that stub into a forward once the flip lands.
//!
//! ## Degenerate-gate guard
//!
//! A bit-exact differential passes whenever both sides degenerate the same
//! way, so every group below is defended twice:
//!
//! 1. `Cvar_RegisterVariable` does not run outside `v_init_registers_...`, so
//!    a `cvar_t` left at its static initializer reads `.value == 0.0` on BOTH
//!    halves. `ctest_view_set_cvars` writes one explicit table into both, and
//!    `ctest_view_reset` restores the defaults; no test relies on an
//!    unregistered cvar carrying a value.
//! 2. Each group additionally asserts something *positive* about the shared
//!    result (a bob that is non-zero, a roll that changes sign, a blend that
//!    is not all-zero, a refdef that actually moved), so a mutation that
//!    flattens both sides identically still fails.
//!
//! Every group names, in its doc comment, the mutation that was applied to
//! `rust/quake-capi/src/view.rs` to prove it is not vacuous.
//!
//! ## Hidden state
//!
//! `V_CalcRefdef`'s `static float oldz` / `static vec3_t punch` and
//! `CalcGunAngle`'s `oldyaw`/`oldpitch` exist twice and cannot be written from
//! outside. `ctest_view_reset` drives both sets to the same known value with a
//! deterministic prologue (see `view_ref.c`'s "Function-local-static
//! lockstep" comment) and is called before EVERY side of EVERY comparison.
//! That prologue's proof for `oldyaw`/`oldpitch` needs `host_frametime >= 0`,
//! so no fixture here uses a negative frametime.
//!
//! Float comparisons are on `to_bits()`, never an epsilon: the point of the
//! gate is bit-exactness, and an approximate compare would hide exactly the
//! reassociation and contraction regressions ADR-010 exists to prevent.

use core::ffi::{c_char, c_double, c_float, c_int, c_uint, c_void};
use core::mem::{offset_of, size_of};
use std::ffi::CStr;
use std::sync::{Mutex, MutexGuard};

// The [lib] name of the quake-capi package is quake_rs.
use quake_rs::view::{EntLerp, Entity, LightCache, RefDef, VRect};
use quake_types::host::CShift;
use quake_types::progs::EntityState;

use quake_ctest as _; // links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

const NUM_CSHIFTS: usize = 4;
/// `view_ref.c`'s `CTEST_VIEW_CVARS`.
const NCVARS: usize = 33;

// Indices into the cvar table, in `ctest_view_cvar`'s order (view.c
// declaration order, then the two cl_input.c cvars V_DriftPitch reads, then
// chase_active).
const CV_SCR_OFSX: usize = 0;
const CV_SCR_OFSY: usize = 1;
const CV_SCR_OFSZ: usize = 2;
const CV_CL_ROLLSPEED: usize = 3;
const CV_CL_ROLLANGLE: usize = 4;
const CV_CL_BOB: usize = 5;
const CV_CL_BOBCYCLE: usize = 6;
const CV_CL_BOBUP: usize = 7;
const CV_V_KICKTIME: usize = 8;
const CV_V_KICKROLL: usize = 9;
const CV_V_KICKPITCH: usize = 10;
const CV_V_GUNKICK: usize = 11;
const CV_V_AUTOPITCH: usize = 12;
const CV_V_IDLESCALE: usize = 19;
const CV_GL_CSHIFTPERCENT: usize = 22;
const CV_GL_CSHIFTPERCENT_CONTENTS: usize = 23;
const CV_GL_CSHIFTPERCENT_DAMAGE: usize = 24;
const CV_GL_CSHIFTPERCENT_BONUS: usize = 25;
const CV_GL_CSHIFTPERCENT_POWERUP: usize = 26;
const CV_R_VIEWMODEL_QUAKE: usize = 27;
const CV_V_CENTERMOVE: usize = 28;
const CV_V_CENTERSPEED: usize = 29;
const CV_CL_FORWARDSPEED: usize = 30;
const CV_LOOKSPRING: usize = 31;
const CV_CHASE_ACTIVE: usize = 32;

/// The 30 cvars `V_Init` registers occupy indices 0..30; 30/31/32 belong to
/// `cl_input.c` and `chase.c` and must stay unregistered by `V_Init`.
const V_INIT_CVARS: usize = 30;

// `quakedef.h` contents values, `client.h` cshift slots, `quakedef.h` items.
const CONTENTS_EMPTY: c_int = -1;
const CONTENTS_SOLID: c_int = -2;
const CONTENTS_WATER: c_int = -3;
const CONTENTS_SLIME: c_int = -4;
const CONTENTS_LAVA: c_int = -5;
const CONTENTS_SKY: c_int = -6;

const CSHIFT_CONTENTS: usize = 0;
const CSHIFT_DAMAGE: usize = 1;
const CSHIFT_BONUS: usize = 2;
const CSHIFT_POWERUP: usize = 3;

const IT_INVISIBILITY: c_int = 524_288;
const IT_INVULNERABILITY: c_int = 1_048_576;
const IT_SUIT: c_int = 2_097_152;
const IT_QUAD: c_int = 4_194_304;

/// `view_ref.c`'s `ctest_view_state_t`. Size is pinned by `abi_mirrors`.
#[repr(C)]
#[derive(Clone, Copy)]
struct ViewState {
    time: c_double,
    oldtime: c_double,
    mtime0: c_double,
    mtime1: c_double,
    laststop: c_double,
    host_frametime: c_double,
    velocity: [c_float; 3],
    viewangles: [c_float; 3],
    punchangle: [c_float; 3],
    ent_origin: [c_float; 3],
    ent_angles: [c_float; 3],
    ent_msg_angles: [c_float; 3],
    viewent_origin: [c_float; 3],
    viewent_angles: [c_float; 3],
    viewent_frame_change_time: c_double,
    viewent_frame_duration: c_double,
    viewent_frame_finish_time: c_double,
    viewent_frame: c_int,
    viewent_prev_frame: c_int,
    viewent_snap_frames: c_int,
    viewent_model_idx: c_int,
    viewent_colormap: c_int,
    stat_health: c_int,
    stat_weapon: c_int,
    stat_weaponframe: c_int,
    stat_viewheight: c_int,
    statsf_idealpitch: c_float,
    items: c_int,
    intermission: c_int,
    maxclients: c_int,
    viewentity: c_int,
    onground: c_int,
    inwater: c_int,
    paused: c_int,
    nodrift: c_int,
    demoplayback: c_int,
    demoseeking: c_int,
    pitchvel: c_float,
    driftmove: c_float,
    idealpitch: c_float,
    faceanimtime: c_float,
    v_dmg_time: c_float,
    v_dmg_roll: c_float,
    v_dmg_pitch: c_float,
    movemessages: c_int,
    movecmd_forwardmove: c_float,
    cshifts_dest: [[c_int; 3]; NUM_CSHIFTS],
    cshifts_pct: [c_float; NUM_CSHIFTS],
    prev_dest: [[c_int; 3]; NUM_CSHIFTS],
    prev_pct: [c_float; NUM_CSHIFTS],
    empty_dest: [c_int; 3],
    empty_pct: c_float,
    punchangles: [[c_float; 3]; 2],
    punchangles_times: [c_double; 2],
    blend: [c_int; 4],
    noclip_anglehack: c_int,
    con_forcedup: c_int,
    needs_relink: c_int,
    scr_viewsize: c_float,
    vieworg: [c_float; 3],
    refdef_viewangles: [c_float; 3],
    trace_line_cache_counter: c_int,
    render_scale: c_int,
    render_warp: c_int,
    protocolflags: c_int,
}

/// `view_ref.c`'s `ctest_view_snap_t`. Size is pinned by `abi_mirrors`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Snap {
    vieworg: [c_float; 3],
    viewangles: [c_float; 3],
    cl_viewangles: [c_float; 3],
    cl_punchangle: [c_float; 3],
    pitchvel: c_float,
    driftmove: c_float,
    idealpitch: c_float,
    faceanimtime: c_float,
    nodrift: c_int,
    laststop: c_double,
    v_dmg_time: c_float,
    v_dmg_roll: c_float,
    v_dmg_pitch: c_float,
    cshifts_dest: [[c_int; 3]; NUM_CSHIFTS],
    cshifts_pct: [c_float; NUM_CSHIFTS],
    prev_dest: [[c_int; 3]; NUM_CSHIFTS],
    prev_pct: [c_float; NUM_CSHIFTS],
    empty_dest: [c_int; 3],
    empty_pct: c_float,
    viewent_origin: [c_float; 3],
    viewent_angles: [c_float; 3],
    viewent_frame: c_int,
    viewent_prev_frame: c_int,
    viewent_snap_frames: c_int,
    viewent_frame_change_time: c_double,
    viewent_frame_duration: c_double,
    viewent_frame_finish_time: c_double,
    viewent_model_idx: c_int,
    viewent_colormap: c_int,
    ent_origin: [c_float; 3],
    ent_angles: [c_float; 3],
    blend: [c_int; 4],
    punchangles: [[c_float; 3]; 2],
    punchangles_times: [c_double; 2],
    trace_line_cache_counter: c_int,
    render_scale: c_int,
    render_warp: c_int,
}

/// `view_ref.c`'s `ctest_view_cvar_info_t`. Size is pinned by `abi_mirrors`.
#[repr(C)]
#[derive(Clone, Copy)]
struct CvarInfo {
    found: c_int,
    flags: c_uint,
    value: c_float,
    name: [c_char; 64],
    string: [c_char; 64],
}

extern "C" {
    // ---- fixture (stubs/view_ref.c) ----
    fn ctest_view_reset(oldz_seed: c_float);
    fn ctest_view_apply(state: *const ViewState);
    fn ctest_view_snapshot(out: *mut Snap, oracle: c_int);
    fn ctest_view_set_cvars(values: *const c_float);
    fn ctest_view_default_cvar_table(out: *mut c_float);
    fn ctest_view_cvar_count() -> c_int;
    fn ctest_view_cvar_info(idx: c_int, oracle: c_int, out: *mut CvarInfo);
    fn ctest_view_cmd_handler(name: *const c_char) -> c_int;
    fn ctest_view_load_message(data: *const u8, len: c_int);
    fn ctest_view_tokenize(text: *const c_char);
    fn ctest_view_abi(idx: c_int) -> c_int;

    // ---- the Sys_Error trap (stubs.c) ----
    fn ctest_try(f: unsafe extern "C" fn(*mut c_void), arg: *mut c_void) -> c_int;
    fn ctest_sys_error_message() -> *const c_char;

    // ---- oracle half (Quake/view.c through c_ref_prelude.h) ----
    fn c_ref_V_CalcRoll(angles: *mut c_float, velocity: *mut c_float) -> c_float;
    fn c_ref_V_CalcBob() -> c_float;
    fn c_ref_V_StartPitchDrift();
    fn c_ref_V_StopPitchDrift();
    fn c_ref_V_DriftPitch();
    fn c_ref_V_ResetBlend();
    fn c_ref_V_ParseDamage();
    fn c_ref_V_cshift_f();
    fn c_ref_V_BonusFlash_f();
    fn c_ref_V_SetContentsColor(contents: c_int);
    fn c_ref_V_CalcPowerupCshift();
    fn c_ref_V_CalcBlend();
    fn c_ref_angledelta(a: c_float) -> c_float;
    fn c_ref_CalcGunAngle();
    fn c_ref_V_BoundOffsets();
    fn c_ref_V_AddIdle();
    fn c_ref_V_CalcViewRoll();
    fn c_ref_V_CalcIntermissionRefdef();
    fn c_ref_V_CalcRefdef();
    fn c_ref_V_RestoreAngles();
    fn c_ref_V_SetupFrame();
    fn c_ref_V_RenderView(use_tasks: bool, begin: u64, setup: u64, done: u64);
    fn c_ref_V_Init();

    // ---- plain half (stubs/view_ref.c -> rust/quake-capi/src/view.rs) ----
    fn quake_rs_v_calc_roll(angles: *mut c_float, velocity: *mut c_float) -> c_float;
    fn V_CalcBob() -> c_float;
    fn V_StartPitchDrift();
    fn V_StopPitchDrift();
    fn V_DriftPitch();
    fn V_ResetBlend();
    fn V_ParseDamage();
    fn V_cshift_f();
    fn V_BonusFlash_f();
    fn V_SetContentsColor(contents: c_int);
    fn V_CalcPowerupCshift();
    fn V_CalcBlend();
    fn angledelta(a: c_float) -> c_float;
    fn CalcGunAngle();
    fn V_BoundOffsets();
    fn V_AddIdle();
    fn V_CalcViewRoll();
    fn V_CalcIntermissionRefdef();
    fn V_CalcRefdef();
    fn V_RestoreAngles();
    fn V_SetupFrame();
    fn V_RenderView(use_tasks: bool, begin: u64, setup: u64, done: u64);
    fn V_Init();
}

// ---------------------------------------------------------------------------
// Bit-exact comparison plumbing.

/// Raw-bit equality. `f32::to_bits` separates `-0.0` from `0.0` and every NaN
/// payload, which `==` would not.
trait BitsEq {
    fn bits_eq(&self, other: &Self) -> bool;
}

impl BitsEq for c_float {
    fn bits_eq(&self, other: &Self) -> bool {
        self.to_bits() == other.to_bits()
    }
}

impl BitsEq for c_double {
    fn bits_eq(&self, other: &Self) -> bool {
        self.to_bits() == other.to_bits()
    }
}

impl BitsEq for c_int {
    fn bits_eq(&self, other: &Self) -> bool {
        self == other
    }
}

impl<T: BitsEq, const N: usize> BitsEq for [T; N] {
    fn bits_eq(&self, other: &Self) -> bool {
        (0..N).all(|i| self[i].bits_eq(&other[i]))
    }
}

macro_rules! diff_fields {
    ($out:expr, $a:expr, $b:expr, $($f:ident),* $(,)?) => {
        $(
            if !$a.$f.bits_eq(&$b.$f) {
                $out.push(format!(
                    "  {}: rust {:?} != c {:?}",
                    stringify!($f), $a.$f, $b.$f
                ));
            }
        )*
    };
}

/// Compares every field of the two snapshots and reports all mismatches at
/// once, so one failing run names the whole divergence rather than the first
/// field of it.
#[track_caller]
fn assert_snap_eq(label: &str, got: &Snap, want: &Snap) {
    let mut d: Vec<String> = Vec::new();
    diff_fields!(
        d,
        got,
        want,
        vieworg,
        viewangles,
        cl_viewangles,
        cl_punchangle,
        pitchvel,
        driftmove,
        idealpitch,
        faceanimtime,
        nodrift,
        laststop,
        v_dmg_time,
        v_dmg_roll,
        v_dmg_pitch,
        cshifts_dest,
        cshifts_pct,
        prev_dest,
        prev_pct,
        empty_dest,
        empty_pct,
        viewent_origin,
        viewent_angles,
        viewent_frame,
        viewent_prev_frame,
        viewent_snap_frames,
        viewent_frame_change_time,
        viewent_frame_duration,
        viewent_frame_finish_time,
        viewent_model_idx,
        viewent_colormap,
        ent_origin,
        ent_angles,
        blend,
        punchangles,
        punchangles_times,
        trace_line_cache_counter,
        render_scale,
        render_warp,
    );
    assert!(
        d.is_empty(),
        "{label}: snapshot divergence\n{}",
        d.join("\n")
    );
}

#[track_caller]
fn assert_bits(label: &str, got: c_float, want: c_float) {
    assert_eq!(
        got.to_bits(),
        want.to_bits(),
        "{label}: rust {got:?} (0x{:08x}) != c {want:?} (0x{:08x})",
        got.to_bits(),
        want.to_bits()
    );
}

fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: the fixture returns NUL-terminated storage with static lifetime.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

fn arr_str(a: &[c_char; 64]) -> String {
    cstr(a.as_ptr())
}

// ---------------------------------------------------------------------------
// Fixture helpers.

fn zeroed_state() -> ViewState {
    // SAFETY: ViewState is a POD mirror of a C struct the fixture memsets
    // itself; all-zero is a valid bit pattern for every field.
    unsafe { core::mem::zeroed() }
}

fn zeroed_snap() -> Snap {
    // SAFETY: as above; ctest_view_snapshot memsets before writing.
    unsafe { core::mem::zeroed() }
}

fn default_cvars() -> [c_float; NCVARS] {
    let mut t = [0.0f32; NCVARS];
    // SAFETY: the fixture copies exactly NCVARS floats, asserted below.
    unsafe {
        assert_eq!(ctest_view_cvar_count() as usize, NCVARS);
        ctest_view_default_cvar_table(t.as_mut_ptr());
    }
    t
}

/// A live-player baseline: on the ground, healthy, one weapon model, a frame
/// of history behind it. Individual groups override only what they exercise.
fn base_state() -> ViewState {
    let mut s = zeroed_state();
    s.time = 2.0;
    s.oldtime = 1.9;
    s.mtime0 = 2.0;
    s.mtime1 = 1.9;
    s.host_frametime = 0.1;
    s.viewentity = 1;
    s.maxclients = 1;
    s.stat_health = 100;
    s.stat_viewheight = 22;
    s.stat_weapon = 1;
    s.stat_weaponframe = 3;
    s.viewent_model_idx = 1;
    s.viewent_frame = 3;
    s.viewent_prev_frame = 3;
    s.ent_origin = [64.0, -32.0, 24.0];
    s.ent_angles = [5.0, 90.0, 0.0];
    s.ent_msg_angles = [7.0, 91.0, 2.0];
    s.viewangles = [5.0, 90.0, 0.0];
    s.velocity = [120.0, -40.0, 0.0];
    s.onground = 1;
    s.movemessages = 4;
    s.movecmd_forwardmove = 100.0;
    s.scr_viewsize = 100.0;
    s.render_scale = 1;
    s.vieworg = [1.0, 2.0, 3.0];
    s.refdef_viewangles = [4.0, 5.0, 6.0];
    s
}

/// Runs `f(oracle)` once per half, each from an identical reset + cvar table +
/// applied state, snapshotting between the two runs so the objects the halves
/// share (the entity array, `r_refdef`, the render globals) cannot leak the
/// first run's result into the second.
fn both<F: Fn(bool)>(
    cvars: &[c_float; NCVARS],
    s: &ViewState,
    oldz: c_float,
    f: F,
) -> (Snap, Snap) {
    let mut got = zeroed_snap();
    let mut want = zeroed_snap();
    // SAFETY: every fixture entry point takes plain pointers to locals that
    // outlive the call, and both halves are single-threaded under TEST_LOCK.
    unsafe {
        ctest_view_reset(oldz);
        ctest_view_set_cvars(cvars.as_ptr());
        ctest_view_apply(s);
        f(false);
        ctest_view_snapshot(&mut got, 0);

        ctest_view_reset(oldz);
        ctest_view_set_cvars(cvars.as_ptr());
        ctest_view_apply(s);
        f(true);
        ctest_view_snapshot(&mut want, 1);
    }
    (got, want)
}

// ---------------------------------------------------------------------------
// Group 1 -- ADR-011 mirrors.

/// `view.rs` defines local `#[repr(C)]` mirrors of `entity_t`, `entlerp_t`,
/// `lightcache_t`, `refdef_t` and `vrect_t` because `quake-types::host` treats
/// `entity_t` as an opaque blob and per-field accessors would cost ~30 FFI
/// round trips per frame. `abi_probe.c` / `host_abi.rs` are shared with the
/// concurrent T7.2b task, so the field-level probes live in `view_ref.c` and
/// the assertions here.
///
/// This group also pins the three fixture structs mirrored above, so a later
/// edit to `ctest_view_state_t` that is not mirrored in this file fails loudly
/// instead of silently reading past the end of a struct.
///
/// MUTATION USED: swapped `vieworg` and `viewangles` in the `view::RefDef`
/// ADR-011 mirror -- both are `[c_float; 3]`, so the port still compiled, and
/// this test failed on the `refdef_t.vieworg` offset assertion (taking
/// `refdef_matches_oracle` down with it). Restored.
#[test]
fn abi_mirrors_match_the_c_headers() {
    let _g = lock();
    // SAFETY: ctest_view_abi is a pure switch over sizeof/offsetof.
    let probe = |i: c_int| unsafe { ctest_view_abi(i) } as usize;

    assert_eq!(size_of::<Entity>(), probe(0), "sizeof entity_t");
    assert_eq!(offset_of!(Entity, origin), probe(1), "entity_t.origin");
    assert_eq!(offset_of!(Entity, angles), probe(2), "entity_t.angles");
    assert_eq!(
        offset_of!(Entity, msg_angles),
        probe(3),
        "entity_t.msg_angles"
    );
    assert_eq!(offset_of!(Entity, model), probe(4), "entity_t.model");
    assert_eq!(offset_of!(Entity, frame), probe(5), "entity_t.frame");
    assert_eq!(offset_of!(Entity, lerp), probe(6), "entity_t.lerp");
    assert_eq!(offset_of!(Entity, netstate), probe(7), "entity_t.netstate");

    assert_eq!(size_of::<EntLerp>(), probe(8), "sizeof entlerp_t");
    assert_eq!(
        offset_of!(EntLerp, prev_frame),
        probe(9),
        "entlerp_t.prev_frame"
    );
    assert_eq!(
        offset_of!(EntLerp, frame_change_time),
        probe(10),
        "entlerp_t.frame_change_time"
    );
    assert_eq!(
        offset_of!(EntLerp, frame_duration),
        probe(11),
        "entlerp_t.frame_duration"
    );
    assert_eq!(
        offset_of!(EntLerp, frame_finish_time),
        probe(12),
        "entlerp_t.frame_finish_time"
    );
    assert_eq!(
        offset_of!(EntLerp, snap_frames),
        probe(13),
        "entlerp_t.snap_frames"
    );

    assert_eq!(size_of::<RefDef>(), probe(14), "sizeof refdef_t");
    assert_eq!(offset_of!(RefDef, vieworg), probe(15), "refdef_t.vieworg");
    assert_eq!(
        offset_of!(RefDef, viewangles),
        probe(16),
        "refdef_t.viewangles"
    );

    assert_eq!(size_of::<VRect>(), probe(17), "sizeof vrect_t");
    assert_eq!(size_of::<LightCache>(), probe(18), "sizeof lightcache_t");
    assert_eq!(
        offset_of!(Entity, lightcache),
        probe(19),
        "entity_t.lightcache"
    );
    assert_eq!(
        offset_of!(EntityState, colormap),
        probe(20),
        "entity_state_t.colormap"
    );
    assert_eq!(size_of::<CShift>(), probe(21), "sizeof cshift_t");

    assert_eq!(size_of::<ViewState>(), probe(22), "ctest_view_state_t");
    assert_eq!(size_of::<Snap>(), probe(23), "ctest_view_snap_t");
    assert_eq!(size_of::<CvarInfo>(), probe(24), "ctest_view_cvar_info_t");
    assert_eq!(
        offset_of!(Entity, msg_origins),
        probe(25),
        "entity_t.msg_origins"
    );

    // Not vacuous: probe() must be returning real numbers, not the -1 default.
    assert!(probe(0) > 64, "entity_t looks implausibly small");
    assert_eq!(
        // SAFETY: out-of-range index returns the -1 default branch.
        unsafe { ctest_view_abi(999) },
        -1,
        "probe table lost its default arm"
    );
}

// ---------------------------------------------------------------------------
// Group 2 -- V_CalcRoll (view.c:87).

/// Drives `quake_rs_v_calc_roll` (the real Rust core) against
/// `c_ref_V_CalcRoll`. Covers velocity exactly along and exactly perpendicular
/// to `right`, both signs, `|side|` below / exactly at / above `cl_rollspeed`,
/// a zero `cl_rollspeed` (the `side < 0` compare then falls to the `else`),
/// and a negative `cl_rollangle`.
///
/// MUTATION USED: `sign = if side < 0.0 { -1.0 } else { 1.0 }` flipped to
/// `>` -- killed. Second mutation: replaced `libm::fabs(side)` with a plain
/// cast (a no-op) -- killed. Third mutation: `<` relaxed to `<=` in the
/// `side < cl_rollspeed.value` test -- this SURVIVED twice. The first survival
/// was a real gap: the case table drove velocity along y, but for yaw 90
/// `right` is +x, so `side` never reached the boundary; the table was
/// rewritten to drive +x. The second survival was an equivalent mutant
/// (`side * value / rollspeed` with `side == rollspeed` collapses to exactly
/// `value`), killed by adding the two boundary cases marked below, where the
/// product overflows to +inf and where the round trip does not land back on
/// `value`. Restored.
#[test]
fn v_calc_roll_matches_oracle() {
    let _g = lock();

    // (angles, velocity, rollspeed, rollangle)
    //
    // For pitch/roll 0 and yaw 90, AngleVectors gives right == (1, 4.4e-8, 0),
    // so the axis V_CalcRoll projects onto is +x. Driving velocity along y
    // instead -- the obvious-looking choice -- yields a side of 4e-6 and every
    // case degenerates into the same near-zero arm; the exactly-at-rollspeed
    // boundary in particular is then never reached at all.
    let cases: &[([c_float; 3], [c_float; 3], c_float, c_float)] = &[
        // along -right and +right.
        ([0.0, 90.0, 0.0], [-100.0, 0.0, 0.0], 200.0, 2.0),
        ([0.0, 90.0, 0.0], [100.0, 0.0, 0.0], 200.0, 2.0),
        // exactly perpendicular: right[2] is exactly 0, so side is exactly
        // +0.0 and `side < 0` takes the positive arm.
        ([0.0, 90.0, 0.0], [0.0, 0.0, 100.0], 200.0, 2.0),
        // along forward -- side is 4.4e-6 rather than 0, the near-degenerate
        // case a naive fixture would use for "perpendicular".
        ([0.0, 90.0, 0.0], [0.0, 100.0, 0.0], 200.0, 2.0),
        // zero velocity.
        ([0.0, 90.0, 0.0], [0.0, 0.0, 0.0], 200.0, 2.0),
        // exactly at cl_rollspeed, both signs: the `<` boundary.
        ([0.0, 90.0, 0.0], [-200.0, 0.0, 0.0], 200.0, 2.0),
        ([0.0, 90.0, 0.0], [200.0, 0.0, 0.0], 200.0, 2.0),
        // one ulp either side of the boundary.
        ([0.0, 90.0, 0.0], [-199.99998, 0.0, 0.0], 200.0, 2.0),
        ([0.0, 90.0, 0.0], [-200.00002, 0.0, 0.0], 200.0, 2.0),
        // The `<` in `side < cl_rollspeed.value` is invisible at the ordinary
        // boundary, because `side * value / rollspeed` with side == rollspeed
        // collapses to exactly `value` -- relaxing it to `<=` is an equivalent
        // mutation there. These two make the arms genuinely disagree at the
        // boundary: the first overflows the product to +inf, the second picks
        // a value/rollspeed pair whose round trip does not land back on value.
        ([0.0, 90.0, 0.0], [1e30, 0.0, 0.0], 1e30, 1e20),
        ([0.0, 90.0, 0.0], [3.0, 0.0, 0.0], 3.0, 0.1),
        // past cl_rollspeed (clamped to cl_rollangle).
        ([0.0, 90.0, 0.0], [-900.0, 0.0, 0.0], 200.0, 2.0),
        // cl_rollspeed == 0: |side| >= 0 always, so the else arm.
        ([0.0, 90.0, 0.0], [-100.0, 0.0, 0.0], 0.0, 2.0),
        // negative cl_rollangle.
        ([0.0, 90.0, 0.0], [-100.0, 0.0, 0.0], 200.0, -3.5),
        // arbitrary angles, all three axes non-trivial.
        ([12.5, 33.25, -7.75], [37.5, -122.25, 88.0], 200.0, 2.0),
        ([-45.0, 200.0, 180.0], [-1.5, 2.25, -3.125], 17.0, 0.75),
        // tiny velocity: the multiply-then-divide order is observable.
        ([0.0, 90.0, 0.0], [-1e-30, 0.0, 0.0], 200.0, 2.0),
    ];

    let mut cvars = default_cvars();
    let mut seen_positive = false;
    let mut seen_negative = false;

    for (i, (angles, velocity, rollspeed, rollangle)) in cases.iter().enumerate() {
        cvars[CV_CL_ROLLSPEED] = *rollspeed;
        cvars[CV_CL_ROLLANGLE] = *rollangle;

        let mut a = *angles;
        let mut v = *velocity;
        // SAFETY: both callees only read three floats through each pointer.
        let (got, want) = unsafe {
            ctest_view_set_cvars(cvars.as_ptr());
            let g = quake_rs_v_calc_roll(a.as_mut_ptr(), v.as_mut_ptr());
            let w = c_ref_V_CalcRoll(a.as_mut_ptr(), v.as_mut_ptr());
            (g, w)
        };
        assert_bits(&format!("V_CalcRoll case {i}"), got, want);
        if want > 0.0 {
            seen_positive = true;
        }
        if want < 0.0 {
            seen_negative = true;
        }
    }

    // Not vacuous: a degenerate cvar table would make every result +0.0.
    assert!(
        seen_positive && seen_negative,
        "V_CalcRoll produced no signed output -- cl_rollangle is probably 0"
    );
}

// ---------------------------------------------------------------------------
// Group 3 -- V_CalcBob (view.c:117).

/// Covers `cl_bobcycle == 0` (the early `return 0`), the two halves of the
/// cycle split at `cl_bobup`, the exact `cycle == cl_bobup` boundary, the
/// `bob > 4` and `bob < -7` clamps, `cl.time` discontinuities (including a
/// negative time, where C's `(int)` truncation rounds toward zero) and a zero
/// horizontal velocity.
///
/// MUTATION USED: `bob * 0.3 + bob * 0.7 * sin(cycle)` rewritten as
/// `bob * (0.3 + 0.7 * sin(cycle))` -- an algebraically identical form with a
/// different rounding -- and 9 cases failed. That is the exact class of
/// regression ADR-010 forbids. Second mutation: the `bob > 4` clamp dropped --
/// killed. Third: the `bob < -7` clamp dropped -- killed. Fourth: the
/// `cl_bobcycle == 0` early return dropped -- killed. Fifth:
/// `cycle < cl_bobup` relaxed to `<=` -- this SURVIVED, and is an equivalent
/// mutant at any ordinary boundary (both arms collapse to PI). It was killed
/// by adding the `cycle == cl_bobup == 0` case marked below, where the if arm
/// evaluates `PI * 0 / 0`. Restored.
#[test]
fn v_calc_bob_matches_oracle() {
    let _g = lock();

    // (time, velocity, cl_bob, cl_bobcycle, cl_bobup)
    let cases: &[(c_double, [c_float; 3], c_float, c_float, c_float)] = &[
        (0.0, [100.0, 0.0, 0.0], 0.02, 0.6, 0.5),
        (1.0, [100.0, 0.0, 0.0], 0.02, 0.6, 0.5),
        // cycle lands exactly on cl_bobup (0.3 / 0.6 == 0.5).
        (0.3, [100.0, 0.0, 0.0], 0.02, 0.6, 0.5),
        // just below and just above that boundary.
        (0.29999998, [100.0, 0.0, 0.0], 0.02, 0.6, 0.5),
        (0.30000001, [100.0, 0.0, 0.0], 0.02, 0.6, 0.5),
        // cl_bobcycle == 0 -> early return.
        (1.0, [100.0, 0.0, 0.0], 0.02, 0.0, 0.5),
        // zero horizontal velocity, non-zero vertical (ignored by the sqrt).
        (1.0, [0.0, 0.0, 400.0], 0.02, 0.6, 0.5),
        // negative velocity components -- the sqrt is on squares.
        (1.0, [-300.0, -400.0, 0.0], 0.02, 0.6, 0.5),
        // clamps: bob > 4 and bob < -7.
        (1.0, [3000.0, 4000.0, 0.0], 0.02, 0.6, 0.5),
        (1.0, [3000.0, 4000.0, 0.0], -0.05, 0.6, 0.5),
        // cl.time discontinuity: large jump, and a negative time.
        (12345.678, [200.0, 200.0, 0.0], 0.02, 0.6, 0.5),
        (-3.25, [200.0, 200.0, 0.0], 0.02, 0.6, 0.5),
        // cl_bobup == 0 -> the else arm divides by (1.0 - 0.0).
        (0.1, [200.0, 0.0, 0.0], 0.02, 0.6, 0.0),
        // cycle == cl_bobup == 0. This is the ONLY input that distinguishes
        // `cycle < cl_bobup` from `<=`: at any ordinary boundary the two arms
        // agree exactly (`PI*c/b` with c == b collapses to PI, which is also
        // what `PI + PI*0/(1-b)` gives), so relaxing the compare is an
        // equivalent mutation everywhere else. Here the if arm computes
        // `PI * 0 / 0` -> NaN and the else arm returns PI.
        (0.0, [200.0, 0.0, 0.0], 0.02, 0.6, 0.0),
        // cl_bobup == 1 -> the if arm for every cycle < 1.
        (0.1, [200.0, 0.0, 0.0], 0.02, 0.6, 1.0),
    ];

    let mut nonzero = 0;
    let mut clamped_high = false;
    let mut clamped_low = false;

    for (i, (time, velocity, bob, bobcycle, bobup)) in cases.iter().enumerate() {
        let mut cvars = default_cvars();
        cvars[CV_CL_BOB] = *bob;
        cvars[CV_CL_BOBCYCLE] = *bobcycle;
        cvars[CV_CL_BOBUP] = *bobup;

        let mut s = base_state();
        s.time = *time;
        s.velocity = *velocity;

        // SAFETY: single-threaded under TEST_LOCK; the fixture republishes
        // every input both halves read before each run.
        let (got, want) = unsafe {
            ctest_view_reset(0.0);
            ctest_view_set_cvars(cvars.as_ptr());
            ctest_view_apply(&s);
            let g = V_CalcBob();

            ctest_view_reset(0.0);
            ctest_view_set_cvars(cvars.as_ptr());
            ctest_view_apply(&s);
            (g, c_ref_V_CalcBob())
        };
        assert_bits(&format!("V_CalcBob case {i}"), got, want);
        if want != 0.0 {
            nonzero += 1;
        }
        if want == 4.0 {
            clamped_high = true;
        }
        if want == -7.0 {
            clamped_low = true;
        }
    }

    // Not vacuous: cl_bob == 0 on both sides would make every result +0.0.
    assert!(
        nonzero >= 8,
        "only {nonzero} non-zero bobs -- cl_bob is dead"
    );
    assert!(clamped_high, "the bob > 4 clamp was never reached");
    assert!(clamped_low, "the bob < -7 clamp was never reached");
}

// ---------------------------------------------------------------------------
// Group 4 -- pitch drift (view.c:150, :166, :185).

/// Covers `V_StartPitchDrift` from both `laststop == cl.time` (the no-op) and
/// otherwise; `V_StopPitchDrift`; and every arm of `V_DriftPitch`: the
/// `noclip_anglehack` / `!onground` / `demoplayback` early return, the
/// `cl.nodrift` re-arm path through `cl_forwardspeed` and `v_centermove` /
/// `lookspring`, `v_autopitch` on and off, `delta == 0`, and both sides of the
/// `delta > 0` / `delta < 0` clamps including the overshoot cutoffs.
///
/// MUTATION USED (five, all killed on the first attempt): dropped the
/// `if cl.laststop == cl.time { return; }` guard at the top of
/// `V_StartPitchDrift`; changed `cl.pitchvel += host_frametime *
/// v_centerspeed.value` to `-=`; dropped the `delta == 0` early return;
/// dropped the `lookspring` gate; and dropped `cls.demoplayback` from the
/// four-term early-out. Restored.
#[test]
fn pitch_drift_matches_oracle() {
    let _g = lock();

    struct DriftCase {
        label: &'static str,
        mutate: fn(&mut ViewState),
        autopitch: c_float,
        centermove: c_float,
        centerspeed: c_float,
        forwardspeed: c_float,
        lookspring: c_float,
    }

    const CASES: &[DriftCase] = &[
        DriftCase {
            label: "grounded, drifting down to level",
            mutate: |s| {
                s.viewangles[0] = 20.0;
                s.pitchvel = 30.0;
            },
            autopitch: 0.0,
            centermove: 0.15,
            centerspeed: 500.0,
            forwardspeed: 200.0,
            lookspring: 0.0,
        },
        DriftCase {
            label: "grounded, drifting up to level",
            mutate: |s| {
                s.viewangles[0] = -20.0;
                s.pitchvel = 30.0;
            },
            autopitch: 0.0,
            centermove: 0.15,
            centerspeed: 500.0,
            forwardspeed: 200.0,
            lookspring: 0.0,
        },
        DriftCase {
            label: "delta == 0 -> pitchvel zeroed",
            mutate: |s| {
                s.viewangles[0] = 0.0;
                s.pitchvel = 55.0;
            },
            autopitch: 0.0,
            centermove: 0.15,
            centerspeed: 500.0,
            forwardspeed: 200.0,
            lookspring: 0.0,
        },
        DriftCase {
            label: "overshoot: move exceeds delta (positive)",
            mutate: |s| {
                s.viewangles[0] = 1.0;
                s.pitchvel = 400.0;
                s.host_frametime = 0.5;
            },
            autopitch: 0.0,
            centermove: 0.15,
            centerspeed: 500.0,
            forwardspeed: 200.0,
            lookspring: 0.0,
        },
        DriftCase {
            label: "overshoot: move exceeds delta (negative)",
            mutate: |s| {
                s.viewangles[0] = -1.0;
                s.pitchvel = 400.0;
                s.host_frametime = 0.5;
            },
            autopitch: 0.0,
            centermove: 0.15,
            centerspeed: 500.0,
            forwardspeed: 200.0,
            lookspring: 0.0,
        },
        DriftCase {
            label: "not on ground -> early return",
            mutate: |s| {
                s.onground = 0;
                s.viewangles[0] = 20.0;
                s.pitchvel = 30.0;
                s.driftmove = 5.0;
            },
            autopitch: 0.0,
            centermove: 0.15,
            centerspeed: 500.0,
            forwardspeed: 200.0,
            lookspring: 0.0,
        },
        DriftCase {
            label: "noclip_anglehack -> early return",
            mutate: |s| {
                s.noclip_anglehack = 1;
                s.viewangles[0] = 20.0;
                s.pitchvel = 30.0;
                s.driftmove = 5.0;
            },
            autopitch: 0.0,
            centermove: 0.15,
            centerspeed: 500.0,
            forwardspeed: 200.0,
            lookspring: 0.0,
        },
        DriftCase {
            label: "demo playback -> early return",
            mutate: |s| {
                s.demoplayback = 1;
                s.viewangles[0] = 20.0;
                s.pitchvel = 30.0;
            },
            autopitch: 0.0,
            centermove: 0.15,
            centerspeed: 500.0,
            forwardspeed: 200.0,
            lookspring: 0.0,
        },
        DriftCase {
            label: "nodrift, moving fast -> driftmove reset",
            mutate: |s| {
                s.nodrift = 1;
                s.movecmd_forwardmove = 300.0;
                s.driftmove = 9.0;
                s.viewangles[0] = 20.0;
            },
            autopitch: 0.0,
            centermove: 0.15,
            centerspeed: 500.0,
            forwardspeed: 200.0,
            lookspring: 0.0,
        },
        DriftCase {
            label: "nodrift, slow, over centermove, lookspring on",
            mutate: |s| {
                s.nodrift = 1;
                s.movecmd_forwardmove = 10.0;
                s.driftmove = 0.2;
                s.viewangles[0] = 20.0;
            },
            autopitch: 0.0,
            centermove: 0.15,
            centerspeed: 500.0,
            forwardspeed: 200.0,
            lookspring: 1.0,
        },
        DriftCase {
            label: "nodrift, slow, over centermove, lookspring off",
            mutate: |s| {
                s.nodrift = 1;
                s.movecmd_forwardmove = 10.0;
                s.driftmove = 0.2;
                s.viewangles[0] = 20.0;
            },
            autopitch: 0.0,
            centermove: 0.15,
            centerspeed: 500.0,
            forwardspeed: 200.0,
            lookspring: 0.0,
        },
        DriftCase {
            label: "v_autopitch on, ideal above current",
            mutate: |s| {
                s.viewangles[0] = -5.0;
                s.statsf_idealpitch = 12.5;
                s.pitchvel = 30.0;
            },
            autopitch: 1.0,
            centermove: 0.15,
            centerspeed: 500.0,
            forwardspeed: 200.0,
            lookspring: 0.0,
        },
        DriftCase {
            label: "v_autopitch on, ideal below current",
            mutate: |s| {
                s.viewangles[0] = 12.5;
                s.statsf_idealpitch = -5.0;
                s.pitchvel = 30.0;
            },
            autopitch: 1.0,
            centermove: 0.15,
            centerspeed: 500.0,
            forwardspeed: 200.0,
            lookspring: 0.0,
        },
        DriftCase {
            label: "zero centerspeed -> pitchvel stays put",
            mutate: |s| {
                s.viewangles[0] = 20.0;
                s.pitchvel = 0.0;
            },
            autopitch: 0.0,
            centermove: 0.15,
            centerspeed: 0.0,
            forwardspeed: 200.0,
            lookspring: 0.0,
        },
    ];

    let mut moved = 0;

    for (i, c) in CASES.iter().enumerate() {
        let mut cvars = default_cvars();
        cvars[CV_V_AUTOPITCH] = c.autopitch;
        cvars[CV_V_CENTERMOVE] = c.centermove;
        cvars[CV_V_CENTERSPEED] = c.centerspeed;
        cvars[CV_CL_FORWARDSPEED] = c.forwardspeed;
        cvars[CV_LOOKSPRING] = c.lookspring;

        let mut s = base_state();
        (c.mutate)(&mut s);
        let before = s.viewangles[0];

        let (got, want) = both(&cvars, &s, 0.0, |oracle| {
            // SAFETY: no arguments; both halves read only the state the
            // fixture just republished.
            unsafe {
                if oracle {
                    c_ref_V_DriftPitch();
                } else {
                    V_DriftPitch();
                }
            }
        });
        assert_snap_eq(&format!("V_DriftPitch[{i}] {}", c.label), &got, &want);
        if !want.cl_viewangles[0].bits_eq(&before) {
            moved += 1;
        }
    }

    // V_StartPitchDrift, both arms of the `laststop == cl.time` guard.
    for (label, laststop, nodrift) in [
        ("start: fresh", 0.0f64, 1),
        ("start: laststop == cl.time", 2.0f64, 1),
        ("start: already drifting", 0.0f64, 0),
    ] {
        let cvars = default_cvars();
        let mut s = base_state();
        s.laststop = laststop;
        s.nodrift = nodrift;
        s.pitchvel = 42.0;
        let (got, want) = both(&cvars, &s, 0.0, |oracle| {
            // SAFETY: as above.
            unsafe {
                if oracle {
                    c_ref_V_StartPitchDrift();
                } else {
                    V_StartPitchDrift();
                }
            }
        });
        assert_snap_eq(label, &got, &want);
    }

    // V_StopPitchDrift.
    {
        let cvars = default_cvars();
        let mut s = base_state();
        s.pitchvel = 42.0;
        s.nodrift = 0;
        let (got, want) = both(&cvars, &s, 0.0, |oracle| {
            // SAFETY: as above.
            unsafe {
                if oracle {
                    c_ref_V_StopPitchDrift();
                } else {
                    V_StopPitchDrift();
                }
            }
        });
        assert_snap_eq("V_StopPitchDrift", &got, &want);
        assert_eq!(want.nodrift, 1, "V_StopPitchDrift did not set cl.nodrift");
        assert_bits("V_StopPitchDrift pitchvel", want.pitchvel, 0.0);
        assert!(
            want.laststop.bits_eq(&2.0f64),
            "V_StopPitchDrift did not stamp cl.laststop"
        );
    }

    // Not vacuous: a dead v_centerspeed would leave the pitch untouched in
    // every case, and the comparison above would still pass.
    assert!(
        moved >= 5,
        "only {moved} V_DriftPitch cases moved the pitch -- the drift is inert"
    );
}

// ---------------------------------------------------------------------------
// Group 5 -- the colour-shift pipeline (view.c:271, :281 .. :448).

/// `V_ResetBlend`, `V_SetContentsColor` over all six `CONTENTS_*` values,
/// `V_CalcPowerupCshift` over every powerup combination, `V_CalcBlend` in and
/// out of intermission with saturating and zero percentages, `V_BonusFlash_f`
/// and `V_cshift_f` with 4, 2 and 0 arguments.
///
/// MUTATION USED: swapped the `CONTENTS_SLIME` and `CONTENTS_LAVA` arms of
/// `quake_rs_v_set_contents_color` -- killed. Second mutation: changed
/// `IT_QUAD`'s blue channel from 255 to 254 -- killed (and took
/// `refdef_matches_oracle` with it). Third: dropped the
/// `if a2 == 0.0 { continue; }` guard in `v_calc_blend` -- this SURVIVED, and
/// is an equivalent mutant whenever the running alpha is already non-zero
/// (`a2 = 0 / a` is 0 and changes nothing). It was killed by adding the
/// "first shift weightless, later shifts opaque" case marked below, where the
/// unguarded arm evaluates 0/0 while `a` is still exactly 0 and poisons the
/// blend with NaN. Restored.
#[test]
fn blend_pipeline_matches_oracle() {
    let _g = lock();

    // --- V_ResetBlend ---
    {
        let cvars = default_cvars();
        let mut s = base_state();
        s.empty_dest = [11, 22, 33];
        s.empty_pct = 44.0;
        s.v_dmg_time = 0.4;
        s.v_dmg_roll = 5.0;
        s.v_dmg_pitch = -6.0;
        for i in 0..NUM_CSHIFTS {
            s.cshifts_dest[i] = [7, 8, 9];
            s.cshifts_pct[i] = 50.0;
        }
        let (got, want) = both(&cvars, &s, 0.0, |oracle| {
            // SAFETY: no arguments; state republished by the fixture.
            unsafe {
                if oracle {
                    c_ref_V_ResetBlend();
                } else {
                    V_ResetBlend();
                }
            }
        });
        assert_snap_eq("V_ResetBlend", &got, &want);
        assert_bits("V_ResetBlend cleared empty_pct", want.empty_pct, 0.0);
        assert_eq!(want.empty_dest, [0, 0, 0], "cshift_empty not cleared");
    }

    // --- V_SetContentsColor ---
    for contents in [
        CONTENTS_EMPTY,
        CONTENTS_SOLID,
        CONTENTS_WATER,
        CONTENTS_SLIME,
        CONTENTS_LAVA,
        CONTENTS_SKY,
        0,
        7,
    ] {
        let cvars = default_cvars();
        let mut s = base_state();
        s.empty_dest = [13, 17, 19];
        s.empty_pct = 23.0;
        let (got, want) = both(&cvars, &s, 0.0, |oracle| {
            // SAFETY: `contents` is a plain int.
            unsafe {
                if oracle {
                    c_ref_V_SetContentsColor(contents);
                } else {
                    V_SetContentsColor(contents);
                }
            }
        });
        assert_snap_eq(&format!("V_SetContentsColor({contents})"), &got, &want);
    }

    // Not vacuous: the four content classes must not all resolve to the same
    // shift -- the T7.0 "both sides return the same constant" trap.
    {
        let cvars = default_cvars();
        let s = base_state();
        let mut seen: Vec<[c_int; 3]> = Vec::new();
        for contents in [
            CONTENTS_EMPTY,
            CONTENTS_WATER,
            CONTENTS_SLIME,
            CONTENTS_LAVA,
        ] {
            let (_, want) = both(&cvars, &s, 0.0, |oracle| {
                // SAFETY: as above.
                unsafe {
                    if oracle {
                        c_ref_V_SetContentsColor(contents);
                    } else {
                        V_SetContentsColor(contents);
                    }
                }
            });
            seen.push(want.cshifts_dest[CSHIFT_CONTENTS]);
        }
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(
            seen.len(),
            4,
            "the four contents classes collapsed to {} distinct shifts",
            seen.len()
        );
    }

    // --- V_CalcPowerupCshift ---
    for items in [
        0,
        IT_QUAD,
        IT_SUIT,
        IT_INVISIBILITY,
        IT_INVULNERABILITY,
        IT_QUAD | IT_SUIT,
        IT_INVISIBILITY | IT_INVULNERABILITY,
        IT_QUAD | IT_SUIT | IT_INVISIBILITY | IT_INVULNERABILITY,
    ] {
        let cvars = default_cvars();
        let mut s = base_state();
        s.items = items;
        s.cshifts_dest[CSHIFT_POWERUP] = [1, 2, 3];
        s.cshifts_pct[CSHIFT_POWERUP] = 99.0;
        let (got, want) = both(&cvars, &s, 0.0, |oracle| {
            // SAFETY: as above.
            unsafe {
                if oracle {
                    c_ref_V_CalcPowerupCshift();
                } else {
                    V_CalcPowerupCshift();
                }
            }
        });
        assert_snap_eq(
            &format!("V_CalcPowerupCshift(items={items:#x})"),
            &got,
            &want,
        );
    }

    // --- V_CalcBlend ---
    struct BlendCase {
        label: &'static str,
        intermission: c_int,
        pct: [c_float; NUM_CSHIFTS],
        dest: [[c_int; 3]; NUM_CSHIFTS],
        percent_cvars: [c_float; 5],
    }

    const BLEND: &[BlendCase] = &[
        BlendCase {
            label: "all zero -> transparent",
            intermission: 0,
            pct: [0.0; NUM_CSHIFTS],
            dest: [[0, 0, 0]; NUM_CSHIFTS],
            percent_cvars: [100.0; 5],
        },
        BlendCase {
            label: "contents only",
            intermission: 0,
            pct: [50.0, 0.0, 0.0, 0.0],
            dest: [[130, 80, 50], [0, 0, 0], [0, 0, 0], [0, 0, 0]],
            percent_cvars: [100.0; 5],
        },
        BlendCase {
            label: "all four stacked",
            intermission: 0,
            pct: [50.0, 90.0, 40.0, 30.0],
            dest: [[130, 80, 50], [255, 0, 0], [215, 186, 69], [0, 0, 255]],
            percent_cvars: [100.0; 5],
        },
        BlendCase {
            label: "intermission drops all but contents",
            intermission: 1,
            pct: [50.0, 90.0, 40.0, 30.0],
            dest: [[130, 80, 50], [255, 0, 0], [215, 186, 69], [0, 0, 255]],
            percent_cvars: [100.0; 5],
        },
        BlendCase {
            label: "saturating percentages",
            intermission: 0,
            pct: [150.0, 150.0, 150.0, 150.0],
            dest: [[255, 255, 255]; NUM_CSHIFTS],
            percent_cvars: [100.0; 5],
        },
        BlendCase {
            label: "gl_cshiftpercent 0 -> nothing survives",
            intermission: 0,
            pct: [50.0, 90.0, 40.0, 30.0],
            dest: [[130, 80, 50], [255, 0, 0], [215, 186, 69], [0, 0, 255]],
            percent_cvars: [0.0, 100.0, 100.0, 100.0, 100.0],
        },
        BlendCase {
            label: "per-class percentages differ",
            intermission: 0,
            pct: [50.0, 90.0, 40.0, 30.0],
            dest: [[130, 80, 50], [255, 0, 0], [215, 186, 69], [0, 0, 255]],
            percent_cvars: [100.0, 25.0, 200.0, 0.0, 75.0],
        },
        BlendCase {
            label: "negative percent",
            intermission: 0,
            pct: [-40.0, 0.0, 0.0, 0.0],
            dest: [[130, 80, 50], [0, 0, 0], [0, 0, 0], [0, 0, 0]],
            percent_cvars: [100.0; 5],
        },
        // The `if a2 == 0.0 { continue; }` guard is invisible unless the
        // running alpha is still exactly zero when a weightless shift is
        // reached: once `a != 0` the unguarded arm computes `a2 = 0 / a == 0`
        // and leaves r/g/b alone, so dropping the guard is an equivalent
        // mutation everywhere else. Here CSHIFT_CONTENTS weighs nothing while
        // `a` is still 0, so the unguarded arm evaluates 0/0 and poisons the
        // rest of the blend with NaN.
        BlendCase {
            label: "first shift weightless, later shifts opaque",
            intermission: 0,
            pct: [0.0, 90.0, 40.0, 30.0],
            dest: [[130, 80, 50], [255, 0, 0], [215, 186, 69], [0, 0, 255]],
            percent_cvars: [100.0; 5],
        },
    ];

    let mut nonzero_blends = 0;
    for (i, b) in BLEND.iter().enumerate() {
        let mut cvars = default_cvars();
        cvars[CV_GL_CSHIFTPERCENT] = b.percent_cvars[0];
        cvars[CV_GL_CSHIFTPERCENT_CONTENTS] = b.percent_cvars[1];
        cvars[CV_GL_CSHIFTPERCENT_DAMAGE] = b.percent_cvars[2];
        cvars[CV_GL_CSHIFTPERCENT_BONUS] = b.percent_cvars[3];
        cvars[CV_GL_CSHIFTPERCENT_POWERUP] = b.percent_cvars[4];

        let mut s = base_state();
        s.intermission = b.intermission;
        s.cshifts_pct = b.pct;
        s.cshifts_dest = b.dest;
        s.blend = [9, 9, 9, 9];

        let (got, want) = both(&cvars, &s, 0.0, |oracle| {
            // SAFETY: as above.
            unsafe {
                if oracle {
                    c_ref_V_CalcBlend();
                } else {
                    V_CalcBlend();
                }
            }
        });
        assert_snap_eq(&format!("V_CalcBlend[{i}] {}", b.label), &got, &want);
        if want.blend != [0, 0, 0, 0] {
            nonzero_blends += 1;
        }
    }

    // Not vacuous: a zeroed gl_cshiftpercent table would blank every blend.
    assert!(
        nonzero_blends >= 4,
        "only {nonzero_blends} blends were non-zero -- the cshift cvars are dead"
    );

    // --- V_BonusFlash_f ---
    {
        let cvars = default_cvars();
        let s = base_state();
        let (got, want) = both(&cvars, &s, 0.0, |oracle| {
            // SAFETY: as above.
            unsafe {
                if oracle {
                    c_ref_V_BonusFlash_f();
                } else {
                    V_BonusFlash_f();
                }
            }
        });
        assert_snap_eq("V_BonusFlash_f", &got, &want);
        assert_eq!(
            want.cshifts_dest[CSHIFT_BONUS],
            [215, 186, 69],
            "V_BonusFlash_f did not set the bonus colour"
        );
        assert_bits(
            "V_BonusFlash_f percent",
            want.cshifts_pct[CSHIFT_BONUS],
            50.0,
        );
    }

    // --- V_cshift_f ---
    for text in [
        c"v_cshift 10 20 30 40",
        c"v_cshift -5 300 0 125",
        c"v_cshift 1 2",
        c"v_cshift",
        c"v_cshift abc 12 xyz 7",
    ] {
        let cvars = default_cvars();
        let mut s = base_state();
        s.empty_dest = [99, 98, 97];
        s.empty_pct = 96.0;
        let (got, want) = both(&cvars, &s, 0.0, |oracle| {
            // SAFETY: `text` is a 'static NUL-terminated literal; the fixture
            // tokenizes into both argv stores before each half runs.
            unsafe {
                ctest_view_tokenize(text.as_ptr());
                if oracle {
                    c_ref_V_cshift_f();
                } else {
                    V_cshift_f();
                }
            }
        });
        assert_snap_eq(&format!("V_cshift_f({text:?})"), &got, &want);
    }

    // Not vacuous: a broken tokenizer on both sides would leave every result
    // at the pre-applied value.
    {
        let cvars = default_cvars();
        let mut s = base_state();
        s.empty_dest = [99, 98, 97];
        s.empty_pct = 96.0;
        let (_, want) = both(&cvars, &s, 0.0, |oracle| {
            // SAFETY: as above.
            unsafe {
                ctest_view_tokenize(c"v_cshift 10 20 30 40".as_ptr());
                if oracle {
                    c_ref_V_cshift_f();
                } else {
                    V_cshift_f();
                }
            }
        });
        assert_eq!(
            want.empty_dest,
            [10, 20, 30],
            "V_cshift_f never read its arguments"
        );
        assert_bits("V_cshift_f percent", want.empty_pct, 40.0);
    }
}

// ---------------------------------------------------------------------------
// Group 6 -- V_ParseDamage (view.c:281).

/// Feeds the same eight-byte damage message to both halves (each side has its
/// own `net_message` buffer and read cursor -- `ctest_view_load_message`
/// rewinds both) and compares the resulting kick, face-anim time and damage
/// cshift. Covers `armor > blood`, `armor` alone, `blood` alone, the
/// `count < 10` floor, the 150-percent ceiling, `cls.demoseeking` (the early
/// return), a damage source directly ahead / behind / to each side, and a
/// source exactly at the view origin (`VectorNormalize` of a zero vector).
///
/// MUTATION USED (four, all killed on the first attempt): lowered the
/// `count < 10` floor to `count < 5`; relaxed `armor > blood` to `>=`; dropped
/// the `* v_kickroll.value` factor from `cl.v_dmg_roll`; and dropped the
/// `cls.demoseeking` early return. Restored.
#[test]
fn v_parse_damage_matches_oracle() {
    let _g = lock();

    /// armor, blood, and the three coords as protocol shorts (units of 1/8).
    fn msg(armor: u8, blood: u8, from: [i16; 3]) -> [u8; 8] {
        let mut m = [0u8; 8];
        m[0] = armor;
        m[1] = blood;
        for (i, v) in from.iter().enumerate() {
            let b = v.to_le_bytes();
            m[2 + i * 2] = b[0];
            m[3 + i * 2] = b[1];
        }
        m
    }

    struct DamageCase {
        label: &'static str,
        msg: [u8; 8],
        demoseeking: c_int,
        pct: c_float,
    }

    let ent = [64.0f32, -32.0, 24.0]; // base_state's ent_origin
    let ahead = [
        ((ent[0] + 100.0) * 8.0) as i16,
        (ent[1] * 8.0) as i16,
        (ent[2] * 8.0) as i16,
    ];
    let behind = [
        ((ent[0] - 100.0) * 8.0) as i16,
        (ent[1] * 8.0) as i16,
        (ent[2] * 8.0) as i16,
    ];
    let left = [
        (ent[0] * 8.0) as i16,
        ((ent[1] + 100.0) * 8.0) as i16,
        (ent[2] * 8.0) as i16,
    ];
    let right = [
        (ent[0] * 8.0) as i16,
        ((ent[1] - 100.0) * 8.0) as i16,
        (ent[2] * 8.0) as i16,
    ];
    let coincident = [
        (ent[0] * 8.0) as i16,
        (ent[1] * 8.0) as i16,
        (ent[2] * 8.0) as i16,
    ];

    let cases = [
        DamageCase {
            label: "armor > blood, from ahead",
            msg: msg(40, 10, ahead),
            demoseeking: 0,
            pct: 0.0,
        },
        DamageCase {
            label: "armor only, from behind",
            msg: msg(40, 0, behind),
            demoseeking: 0,
            pct: 0.0,
        },
        DamageCase {
            label: "blood only, from the left",
            msg: msg(0, 40, left),
            demoseeking: 0,
            pct: 0.0,
        },
        DamageCase {
            label: "blood only, from the right",
            msg: msg(0, 40, right),
            demoseeking: 0,
            pct: 0.0,
        },
        DamageCase {
            label: "tiny hit -> count floors at 10",
            msg: msg(1, 1, ahead),
            demoseeking: 0,
            pct: 0.0,
        },
        DamageCase {
            label: "zero hit -> count floors at 10",
            msg: msg(0, 0, ahead),
            demoseeking: 0,
            pct: 0.0,
        },
        DamageCase {
            label: "huge hit against an existing shift -> 150 ceiling",
            msg: msg(255, 255, ahead),
            demoseeking: 0,
            pct: 120.0,
        },
        DamageCase {
            label: "demoseeking -> early return after faceanimtime",
            msg: msg(40, 10, ahead),
            demoseeking: 1,
            pct: 0.0,
        },
        DamageCase {
            label: "source coincident with the entity",
            msg: msg(20, 20, coincident),
            demoseeking: 0,
            pct: 0.0,
        },
    ];

    let mut kicked = 0;
    for (i, c) in cases.iter().enumerate() {
        let cvars = default_cvars();
        let mut s = base_state();
        s.demoseeking = c.demoseeking;
        s.cshifts_pct[CSHIFT_DAMAGE] = c.pct;
        s.cshifts_dest[CSHIFT_DAMAGE] = [1, 2, 3];

        let (got, want) = both(&cvars, &s, 0.0, |oracle| {
            // SAFETY: `c.msg` outlives the call; the fixture copies it into
            // both net_message buffers and rewinds both cursors.
            unsafe {
                ctest_view_load_message(c.msg.as_ptr(), c.msg.len() as c_int);
                if oracle {
                    c_ref_V_ParseDamage();
                } else {
                    V_ParseDamage();
                }
            }
        });
        assert_snap_eq(&format!("V_ParseDamage[{i}] {}", c.label), &got, &want);
        if want.v_dmg_roll != 0.0 || want.v_dmg_pitch != 0.0 {
            kicked += 1;
        }
    }

    // The kick scale cvars themselves: zero, negative and oversized.
    for (label, kickroll, kickpitch) in [
        ("kick scales zeroed", 0.0f32, 0.0f32),
        ("kick scales negated", -0.6, -0.6),
        ("kick scales exaggerated", 12.5, -7.25),
    ] {
        let mut cvars = default_cvars();
        cvars[CV_V_KICKROLL] = kickroll;
        cvars[CV_V_KICKPITCH] = kickpitch;
        let s = base_state();
        let m = msg(40, 10, ahead);
        let (got, want) = both(&cvars, &s, 0.0, |oracle| {
            // SAFETY: the message outlives the call; the fixture rewinds both
            // read cursors before each half runs.
            unsafe {
                ctest_view_load_message(m.as_ptr(), m.len() as c_int);
                if oracle {
                    c_ref_V_ParseDamage();
                } else {
                    V_ParseDamage();
                }
            }
        });
        assert_snap_eq(&format!("V_ParseDamage: {label}"), &got, &want);
        if want.v_dmg_roll != 0.0 || want.v_dmg_pitch != 0.0 {
            kicked += 1;
        }
    }

    // Not vacuous: v_kickroll/v_kickpitch at 0 would silence every kick.
    assert!(
        kicked >= 4,
        "only {kicked} damage cases produced a kick -- v_kick* are dead"
    );
}

// ---------------------------------------------------------------------------
// Group 7 -- refdef assembly (view.c:538 .. :888).

/// The whole eye-placement chain: `angledelta`, `V_AddIdle`, `V_CalcViewRoll`,
/// `V_BoundOffsets`, `CalcGunAngle`, `V_CalcIntermissionRefdef`,
/// `V_CalcRefdef`, `V_RestoreAngles` and `V_SetupFrame`. `V_CalcRefdef` is the
/// widest one and is driven over the stair-smoothing branch (both arms), the
/// three `v_gunkick` modes, the weapon-model change and snap-frame arms, the
/// `r_viewmodel_quake` viewsize ladder, `cl.maxclients <= 1` on and off, and
/// `chase_active` on (which hands off to `Chase_UpdateForDrawing`).
///
/// MUTATION USED (five, all killed on the first attempt and all by this test
/// alone): changed `r_refdef.vieworg[i] += 1.0 / 32` to `1.0 / 16`; changed
/// `V_BoundOffsets`' `+30` upper Z bound to `+22`; flipped the sign of the
/// `v_idlescale * sin(...) * v_iroll_level` term in `V_AddIdle`'s ROLL
/// component; changed `V_CalcViewRoll`'s dead-view `roll = 80` to `70`; and
/// changed the stair-smoothing rate `oldz += steptime * 80` to `* 40`.
/// Restored.
#[test]
fn refdef_matches_oracle() {
    let _g = lock();

    // --- angledelta: pure, no state ---
    for a in [
        0.0f32, 1.0, 179.0, 180.0, 180.5, 181.0, 359.0, 360.0, 361.0, -1.0, -180.0, -181.0, -359.5,
        720.25, -720.25, 1e7, -1e7,
    ] {
        // SAFETY: both are pure float->float.
        let (got, want) = unsafe { (angledelta(a), c_ref_angledelta(a)) };
        assert_bits(&format!("angledelta({a})"), got, want);
    }
    // Not vacuous: the function must actually wrap.
    // SAFETY: pure.
    assert_bits("angledelta(270) wraps", unsafe { angledelta(270.0) }, -90.0);

    // --- V_AddIdle / V_CalcViewRoll / V_BoundOffsets / CalcGunAngle ---
    struct SimpleCase {
        label: &'static str,
        mutate: fn(&mut ViewState),
        idlescale: c_float,
    }

    const SIMPLE: &[SimpleCase] = &[
        SimpleCase {
            label: "idle off",
            mutate: |_| {},
            idlescale: 0.0,
        },
        SimpleCase {
            label: "idle on",
            mutate: |_| {},
            idlescale: 1.0,
        },
        SimpleCase {
            label: "idle on, negative time",
            mutate: |s| s.time = -12.5,
            idlescale: 1.0,
        },
        SimpleCase {
            label: "idle on, large time",
            mutate: |s| s.time = 98765.4321,
            idlescale: 2.5,
        },
    ];

    for (i, c) in SIMPLE.iter().enumerate() {
        let mut cvars = default_cvars();
        cvars[CV_V_IDLESCALE] = c.idlescale;
        let mut s = base_state();
        (c.mutate)(&mut s);

        let (got, want) = both(&cvars, &s, 0.0, |oracle| {
            // SAFETY: no arguments.
            unsafe {
                if oracle {
                    c_ref_V_AddIdle();
                } else {
                    V_AddIdle();
                }
            }
        });
        assert_snap_eq(&format!("V_AddIdle[{i}] {}", c.label), &got, &want);

        let (got, want) = both(&cvars, &s, 0.0, |oracle| {
            // SAFETY: no arguments.
            unsafe {
                if oracle {
                    c_ref_CalcGunAngle();
                } else {
                    CalcGunAngle();
                }
            }
        });
        assert_snap_eq(&format!("CalcGunAngle[{i}] {}", c.label), &got, &want);
    }

    // Not vacuous: with v_idlescale > 0 the idle sway must actually move
    // r_refdef.viewangles away from the applied value.
    {
        let mut cvars = default_cvars();
        cvars[CV_V_IDLESCALE] = 1.0;
        let mut s = base_state();
        s.time = 1.0;
        s.refdef_viewangles = [4.0, 5.0, 6.0];
        let (_, want) = both(&cvars, &s, 0.0, |oracle| {
            // SAFETY: no arguments.
            unsafe {
                if oracle {
                    c_ref_V_AddIdle();
                } else {
                    V_AddIdle();
                }
            }
        });
        assert!(
            !want.viewangles.bits_eq(&[4.0, 5.0, 6.0]),
            "V_AddIdle left r_refdef.viewangles untouched -- v_i*_level are dead"
        );
    }

    // V_CalcViewRoll: with and without an active damage kick, and dead.
    for (label, health, dmg_time, kicktime) in [
        ("alive, no kick", 100, 0.0f32, 0.5f32),
        ("alive, kick in progress", 100, 0.4, 0.5),
        ("alive, kick expiring", 100, 0.05, 0.5),
        ("dead -> roll pinned to 80", 0, 0.4, 0.5),
        ("negative health", -25, 0.0, 0.5),
    ] {
        let mut cvars = default_cvars();
        cvars[CV_V_KICKTIME] = kicktime;
        let mut s = base_state();
        s.stat_health = health;
        s.v_dmg_time = dmg_time;
        s.v_dmg_roll = 7.5;
        s.v_dmg_pitch = -3.25;

        let (got, want) = both(&cvars, &s, 0.0, |oracle| {
            // SAFETY: no arguments.
            unsafe {
                if oracle {
                    c_ref_V_CalcViewRoll();
                } else {
                    V_CalcViewRoll();
                }
            }
        });
        assert_snap_eq(&format!("V_CalcViewRoll: {label}"), &got, &want);
        if health <= 0 {
            assert_bits("dead view roll", want.viewangles[2], 80.0);
        }
    }

    // V_BoundOffsets: refdef pushed outside the clipping box on every axis.
    for (label, vieworg) in [
        ("inside the box", [64.0f32, -32.0, 24.0]),
        ("past -x", [0.0, -32.0, 24.0]),
        ("past +x", [200.0, -32.0, 24.0]),
        ("past -y", [64.0, -300.0, 24.0]),
        ("past +y", [64.0, 300.0, 24.0]),
        ("past -z", [64.0, -32.0, -300.0]),
        ("past +z", [64.0, -32.0, 300.0]),
        ("exactly on the -14 edge", [50.0, -32.0, 24.0]),
        ("exactly on the +30 edge", [64.0, -32.0, 54.0]),
    ] {
        let cvars = default_cvars();
        let mut s = base_state();
        s.vieworg = vieworg;
        let (got, want) = both(&cvars, &s, 0.0, |oracle| {
            // SAFETY: no arguments.
            unsafe {
                if oracle {
                    c_ref_V_BoundOffsets();
                } else {
                    V_BoundOffsets();
                }
            }
        });
        assert_snap_eq(&format!("V_BoundOffsets: {label}"), &got, &want);
    }
    // Not vacuous: a dropped clamp would leave the far-away origin untouched.
    {
        let cvars = default_cvars();
        let mut s = base_state();
        s.vieworg = [1000.0, 1000.0, 1000.0];
        let (_, want) = both(&cvars, &s, 0.0, |oracle| {
            // SAFETY: no arguments.
            unsafe {
                if oracle {
                    c_ref_V_BoundOffsets();
                } else {
                    V_BoundOffsets();
                }
            }
        });
        assert_bits("bounded x", want.vieworg[0], 64.0 + 14.0);
        assert_bits("bounded y", want.vieworg[1], -32.0 + 14.0);
        assert_bits("bounded z", want.vieworg[2], 24.0 + 30.0);
    }

    // --- V_RestoreAngles ---
    {
        let cvars = default_cvars();
        let mut s = base_state();
        s.ent_angles = [1.0, 2.0, 3.0];
        s.ent_msg_angles = [40.0, 50.0, 60.0];
        let (got, want) = both(&cvars, &s, 0.0, |oracle| {
            // SAFETY: no arguments.
            unsafe {
                if oracle {
                    c_ref_V_RestoreAngles();
                } else {
                    V_RestoreAngles();
                }
            }
        });
        assert_snap_eq("V_RestoreAngles", &got, &want);
        assert!(
            want.ent_angles.bits_eq(&[40.0, 50.0, 60.0]),
            "V_RestoreAngles did not copy msg_angles[0]"
        );
    }

    // --- V_CalcIntermissionRefdef ---
    {
        let mut cvars = default_cvars();
        cvars[CV_V_IDLESCALE] = 0.25; // saved/restored around the forced 1
        let mut s = base_state();
        s.ent_angles = [11.0, 22.0, 33.0];
        s.viewent_model_idx = 2;
        let (got, want) = both(&cvars, &s, 0.0, |oracle| {
            // SAFETY: no arguments.
            unsafe {
                if oracle {
                    c_ref_V_CalcIntermissionRefdef();
                } else {
                    V_CalcIntermissionRefdef();
                }
            }
        });
        assert_snap_eq("V_CalcIntermissionRefdef", &got, &want);
        assert_eq!(
            want.viewent_model_idx, -1,
            "intermission did not drop the weapon model"
        );
        assert_ne!(
            want.trace_line_cache_counter, 0,
            "InvalidateTraceLineCache never ran"
        );
    }

    // --- V_CalcRefdef, the wide one ---
    struct RefdefCase {
        label: &'static str,
        mutate: fn(&mut ViewState),
        gunkick: c_float,
        viewmodel_quake: c_float,
        idlescale: c_float,
        maxclients: c_int,
        scr_ofs: [c_float; 3],
        oldz: c_float,
    }

    const REFDEF: &[RefdefCase] = &[
        RefdefCase {
            label: "baseline, gunkick 1",
            mutate: |s| s.punchangle = [-2.0, 0.5, 0.0],
            gunkick: 1.0,
            viewmodel_quake: 0.0,
            idlescale: 0.0,
            maxclients: 1,
            scr_ofs: [0.0, 0.0, 0.0],
            oldz: 24.0,
        },
        RefdefCase {
            label: "gunkick 0 -> neither kick arm",
            mutate: |s| s.punchangle = [-2.0, 0.5, 0.0],
            gunkick: 0.0,
            viewmodel_quake: 0.0,
            idlescale: 0.0,
            maxclients: 1,
            scr_ofs: [0.0, 0.0, 0.0],
            oldz: 24.0,
        },
        RefdefCase {
            label: "gunkick 2 -> lerped kick, positive delta",
            mutate: |s| {
                s.punchangles[0] = [-4.0, 1.0, 0.5];
                s.punchangles[1] = [0.0, 0.0, 0.0];
                s.punchangles_times = [1.05, 1.0];
            },
            gunkick: 2.0,
            viewmodel_quake: 0.0,
            idlescale: 0.0,
            maxclients: 1,
            scr_ofs: [0.0, 0.0, 0.0],
            oldz: 24.0,
        },
        RefdefCase {
            label: "gunkick 2 -> interval clamped to 0.1",
            mutate: |s| {
                s.punchangles[0] = [-4.0, 1.0, 0.5];
                s.punchangles[1] = [0.0, 0.0, 0.0];
                s.punchangles_times = [5.0, 1.0];
            },
            gunkick: 2.0,
            viewmodel_quake: 0.0,
            idlescale: 0.0,
            maxclients: 1,
            scr_ofs: [0.0, 0.0, 0.0],
            oldz: 24.0,
        },
        RefdefCase {
            label: "single player -> scr_ofs applies",
            mutate: |_| {},
            gunkick: 1.0,
            viewmodel_quake: 0.0,
            idlescale: 0.0,
            maxclients: 1,
            scr_ofs: [3.0, -4.0, 5.0],
            oldz: 24.0,
        },
        RefdefCase {
            label: "multiplayer -> scr_ofs suppressed",
            mutate: |_| {},
            gunkick: 1.0,
            viewmodel_quake: 0.0,
            idlescale: 0.0,
            maxclients: 8,
            scr_ofs: [3.0, -4.0, 5.0],
            oldz: 24.0,
        },
        RefdefCase {
            label: "stair smoothing: stepped up",
            mutate: |s| {
                s.onground = 1;
                s.ent_origin[2] = 40.0;
                s.time = 2.0;
                s.oldtime = 1.9;
            },
            gunkick: 1.0,
            viewmodel_quake: 0.0,
            idlescale: 0.0,
            maxclients: 1,
            scr_ofs: [0.0, 0.0, 0.0],
            oldz: 24.0,
        },
        RefdefCase {
            label: "stair smoothing: 12-unit cap",
            mutate: |s| {
                s.onground = 1;
                s.ent_origin[2] = 500.0;
            },
            gunkick: 1.0,
            viewmodel_quake: 0.0,
            idlescale: 0.0,
            maxclients: 1,
            scr_ofs: [0.0, 0.0, 0.0],
            oldz: 24.0,
        },
        RefdefCase {
            label: "stair smoothing: negative steptime clamps to 0",
            mutate: |s| {
                s.onground = 1;
                s.ent_origin[2] = 40.0;
                s.time = 1.0;
                s.oldtime = 2.0;
            },
            gunkick: 1.0,
            viewmodel_quake: 0.0,
            idlescale: 0.0,
            maxclients: 1,
            scr_ofs: [0.0, 0.0, 0.0],
            oldz: 24.0,
        },
        RefdefCase {
            label: "airborne -> else arm resets oldz",
            mutate: |s| {
                s.onground = 0;
                s.ent_origin[2] = 40.0;
            },
            gunkick: 1.0,
            viewmodel_quake: 0.0,
            idlescale: 0.0,
            maxclients: 1,
            scr_ofs: [0.0, 0.0, 0.0],
            oldz: 24.0,
        },
        RefdefCase {
            label: "weapon model changed",
            mutate: |s| {
                s.stat_weapon = 2;
                s.viewent_model_idx = 1;
                s.stat_weaponframe = 9;
            },
            gunkick: 1.0,
            viewmodel_quake: 0.0,
            idlescale: 0.0,
            maxclients: 1,
            scr_ofs: [0.0, 0.0, 0.0],
            oldz: 24.0,
        },
        RefdefCase {
            label: "same model, frame changed, no snap credit",
            mutate: |s| {
                s.stat_weapon = 1;
                s.viewent_model_idx = 1;
                s.viewent_frame = 3;
                s.stat_weaponframe = 4;
                s.viewent_snap_frames = 0;
                s.viewent_frame_finish_time = 2.4;
            },
            gunkick: 1.0,
            viewmodel_quake: 0.0,
            idlescale: 0.0,
            maxclients: 1,
            scr_ofs: [0.0, 0.0, 0.0],
            oldz: 24.0,
        },
        RefdefCase {
            label: "same model, frame changed, snap credit spent",
            mutate: |s| {
                s.stat_weapon = 1;
                s.viewent_model_idx = 1;
                s.viewent_frame = 3;
                s.stat_weaponframe = 4;
                s.viewent_snap_frames = 2;
                s.viewent_frame_finish_time = 1.0;
            },
            gunkick: 1.0,
            viewmodel_quake: 0.0,
            idlescale: 0.0,
            maxclients: 1,
            scr_ofs: [0.0, 0.0, 0.0],
            oldz: 24.0,
        },
        RefdefCase {
            label: "r_viewmodel_quake at viewsize 110",
            mutate: |s| s.scr_viewsize = 110.0,
            gunkick: 1.0,
            viewmodel_quake: 1.0,
            idlescale: 0.0,
            maxclients: 1,
            scr_ofs: [0.0, 0.0, 0.0],
            oldz: 24.0,
        },
        RefdefCase {
            label: "r_viewmodel_quake at viewsize 80",
            mutate: |s| s.scr_viewsize = 80.0,
            gunkick: 1.0,
            viewmodel_quake: 1.0,
            idlescale: 0.0,
            maxclients: 1,
            scr_ofs: [0.0, 0.0, 0.0],
            oldz: 24.0,
        },
        RefdefCase {
            label: "r_viewmodel_quake at an unlisted viewsize",
            mutate: |s| s.scr_viewsize = 95.0,
            gunkick: 1.0,
            viewmodel_quake: 1.0,
            idlescale: 0.0,
            maxclients: 1,
            scr_ofs: [0.0, 0.0, 0.0],
            oldz: 24.0,
        },
        RefdefCase {
            label: "idle sway on top of everything",
            mutate: |s| s.punchangle = [-2.0, 0.5, 0.0],
            gunkick: 1.0,
            viewmodel_quake: 0.0,
            idlescale: 1.75,
            maxclients: 1,
            scr_ofs: [0.0, 0.0, 0.0],
            oldz: 24.0,
        },
        RefdefCase {
            label: "dead player -> roll pinned",
            mutate: |s| s.stat_health = 0,
            gunkick: 1.0,
            viewmodel_quake: 0.0,
            idlescale: 0.0,
            maxclients: 1,
            scr_ofs: [0.0, 0.0, 0.0],
            oldz: 24.0,
        },
        RefdefCase {
            label: "negative view height",
            mutate: |s| s.stat_viewheight = -16,
            gunkick: 1.0,
            viewmodel_quake: 0.0,
            idlescale: 0.0,
            maxclients: 1,
            scr_ofs: [0.0, 0.0, 0.0],
            oldz: 24.0,
        },
    ];

    let mut refdef_moved = 0;
    for (i, c) in REFDEF.iter().enumerate() {
        let mut cvars = default_cvars();
        cvars[CV_V_GUNKICK] = c.gunkick;
        cvars[CV_R_VIEWMODEL_QUAKE] = c.viewmodel_quake;
        cvars[CV_V_IDLESCALE] = c.idlescale;
        cvars[CV_SCR_OFSX] = c.scr_ofs[0];
        cvars[CV_SCR_OFSY] = c.scr_ofs[1];
        cvars[CV_SCR_OFSZ] = c.scr_ofs[2];

        let mut s = base_state();
        s.maxclients = c.maxclients;
        (c.mutate)(&mut s);

        let (got, want) = both(&cvars, &s, c.oldz, |oracle| {
            // SAFETY: no arguments.
            unsafe {
                if oracle {
                    c_ref_V_CalcRefdef();
                } else {
                    V_CalcRefdef();
                }
            }
        });
        assert_snap_eq(&format!("V_CalcRefdef[{i}] {}", c.label), &got, &want);
        if !want.vieworg.bits_eq(&s.vieworg) {
            refdef_moved += 1;
        }
    }

    // chase_active hands off to Chase_UpdateForDrawing (chase.rs).
    {
        let mut cvars = default_cvars();
        cvars[CV_CHASE_ACTIVE] = 1.0;
        let s = base_state();
        let (got, want) = both(&cvars, &s, 24.0, |oracle| {
            // SAFETY: no arguments; the fixture republished cl.worldmodel on
            // both halves so both traces see stubs.c's synthetic room.
            unsafe {
                if oracle {
                    c_ref_V_CalcRefdef();
                } else {
                    V_CalcRefdef();
                }
            }
        });
        assert_snap_eq("V_CalcRefdef with chase_active", &got, &want);
    }

    // Not vacuous: V_CalcRefdef must actually place the eye somewhere new.
    assert!(
        refdef_moved >= 15,
        "only {refdef_moved} V_CalcRefdef cases moved r_refdef.vieworg"
    );

    // --- V_SetupFrame: the three arms of its dispatch ---
    for (label, con_forcedup, intermission, paused) in [
        ("normal -> V_CalcRefdef", 0, 0, 0),
        ("intermission -> V_CalcIntermissionRefdef", 0, 1, 0),
        ("paused -> neither", 0, 0, 1),
        ("console forced up -> neither", 1, 0, 0),
        ("console forced up during intermission", 1, 1, 0),
    ] {
        let cvars = default_cvars();
        let mut s = base_state();
        s.con_forcedup = con_forcedup;
        s.intermission = intermission;
        s.paused = paused;
        s.cshifts_pct = [50.0, 90.0, 40.0, 30.0];
        s.cshifts_dest = [[130, 80, 50], [255, 0, 0], [215, 186, 69], [0, 0, 255]];
        s.prev_pct = [0.0; NUM_CSHIFTS];
        s.items = IT_QUAD;

        let (got, want) = both(&cvars, &s, 24.0, |oracle| {
            // SAFETY: no arguments.
            unsafe {
                if oracle {
                    c_ref_V_SetupFrame();
                } else {
                    V_SetupFrame();
                }
            }
        });
        assert_snap_eq(&format!("V_SetupFrame: {label}"), &got, &want);
        // V_UpdateBlend always runs, and always decays the damage/bonus
        // shifts -- proof the shared prologue is not being skipped.
        assert!(
            want.cshifts_pct[CSHIFT_DAMAGE] < 90.0,
            "V_UpdateBlend did not decay the damage shift"
        );
        assert!(
            want.cshifts_pct[CSHIFT_BONUS] < 40.0,
            "V_UpdateBlend did not decay the bonus shift"
        );
    }

    // V_UpdateBlend's decay floors at zero rather than going negative.
    {
        let cvars = default_cvars();
        let mut s = base_state();
        s.host_frametime = 10.0;
        s.cshifts_pct = [0.0, 5.0, 5.0, 0.0];
        let (got, want) = both(&cvars, &s, 24.0, |oracle| {
            // SAFETY: no arguments.
            unsafe {
                if oracle {
                    c_ref_V_SetupFrame();
                } else {
                    V_SetupFrame();
                }
            }
        });
        assert_snap_eq("V_SetupFrame: decay floors at zero", &got, &want);
        assert_bits("damage floor", want.cshifts_pct[CSHIFT_DAMAGE], 0.0);
        assert_bits("bonus floor", want.cshifts_pct[CSHIFT_BONUS], 0.0);
    }
}

// ---------------------------------------------------------------------------
// Group 8 -- V_RenderView (view.c:908) and its ADR-009 topology.

/// The three arms:
///
/// 1. `con_forcedup` -- both halves must zero `render_warp` and set
///    `render_scale = 1` and return without touching anything else.
/// 2. `needs_relink && use_tasks` -- both must `Sys_Error` with the identical
///    message. The oracle calls `Sys_Error` from C; the port calls it directly
///    from Rust (ADR-009: `Sys_Error` aborts, it does not longjmp, so no guard
///    is needed -- the `world.rs` / `sv_phys.rs` / `sv_send.c` precedent). In
///    the ctest link `stubs.c` turns it into a `setjmp` trap so the message can
///    be compared instead of aborting the binary.
/// 3. The ordinary path -- both reach `stubs.c`'s `R_RenderView` stand-in,
///    which `Sys_Error`s because `gl_rmain.c` is not an oracle source. The
///    port's route is longer (`View_Glue_RenderView` -> `Host_Guard` -> status
///    2 -> `Host_Reraise` re-issues the same message), which is exactly the
///    ADR-009 topology under test: the guard must preserve the message, and
///    the re-raise must happen in a C frame.
///
/// MUTATION USED (three, all killed on the first attempt and all by this test
/// alone): changed `render_scale = 1` to `= 2` in the `con_forcedup` arm;
/// inverted the `if use_tasks` guard that selects the relink `Sys_Error`; and
/// dropped the `View_Glue_RelinkEntities` call from the `needs_relink` arm.
/// Restored.
#[test]
fn v_render_view_matches_oracle() {
    let _g = lock();

    #[repr(C)]
    struct RvArgs {
        use_tasks: bool,
    }

    unsafe extern "C" fn rv_plain(p: *mut c_void) {
        // SAFETY: `p` is a live `RvArgs` for the duration of ctest_try.
        unsafe {
            let a = &*p.cast::<RvArgs>();
            V_RenderView(a.use_tasks, 0, 0, 0);
        }
    }

    unsafe extern "C" fn rv_oracle(p: *mut c_void) {
        // SAFETY: as above.
        unsafe {
            let a = &*p.cast::<RvArgs>();
            c_ref_V_RenderView(a.use_tasks, 0, 0, 0);
        }
    }

    struct RvCase {
        label: &'static str,
        con_forcedup: c_int,
        needs_relink: c_int,
        use_tasks: bool,
        expect_raise: bool,
    }

    const RV: &[RvCase] = &[
        RvCase {
            label: "console forced up -> early return",
            con_forcedup: 1,
            needs_relink: 0,
            use_tasks: false,
            expect_raise: false,
        },
        RvCase {
            label: "console forced up wins over needs_relink",
            con_forcedup: 1,
            needs_relink: 1,
            use_tasks: true,
            expect_raise: false,
        },
        RvCase {
            label: "needs_relink under tasks -> Sys_Error",
            con_forcedup: 0,
            needs_relink: 1,
            use_tasks: true,
            expect_raise: true,
        },
        RvCase {
            label: "needs_relink without tasks -> relink then render",
            con_forcedup: 0,
            needs_relink: 1,
            use_tasks: false,
            expect_raise: true,
        },
        RvCase {
            label: "ordinary path -> R_RenderView stand-in",
            con_forcedup: 0,
            needs_relink: 0,
            use_tasks: false,
            expect_raise: true,
        },
    ];

    let mut messages_seen = 0;
    for (i, c) in RV.iter().enumerate() {
        let cvars = default_cvars();
        let mut s = base_state();
        s.con_forcedup = c.con_forcedup;
        s.needs_relink = c.needs_relink;
        s.render_warp = 1;
        s.render_scale = 7;

        let mut args = RvArgs {
            use_tasks: c.use_tasks,
        };
        let arg: *mut c_void = (&raw mut args).cast();

        let mut got = zeroed_snap();
        let mut want = zeroed_snap();
        // SAFETY: `args` outlives both ctest_try calls; ctest_try arms the
        // Sys_Error trap around exactly one call each time.
        let (raised_rs, msg_rs, raised_c, msg_c) = unsafe {
            ctest_view_reset(24.0);
            ctest_view_set_cvars(cvars.as_ptr());
            ctest_view_apply(&s);
            let r_rs = ctest_try(rv_plain, arg);
            let m_rs = cstr(ctest_sys_error_message());
            ctest_view_snapshot(&mut got, 0);

            ctest_view_reset(24.0);
            ctest_view_set_cvars(cvars.as_ptr());
            ctest_view_apply(&s);
            let r_c = ctest_try(rv_oracle, arg);
            let m_c = cstr(ctest_sys_error_message());
            ctest_view_snapshot(&mut want, 1);
            (r_rs, m_rs, r_c, m_c)
        };

        assert_eq!(
            raised_rs, raised_c,
            "V_RenderView[{i}] {}: raise disagreement (rust {raised_rs}, c {raised_c})",
            c.label
        );
        assert_eq!(
            raised_rs != 0,
            c.expect_raise,
            "V_RenderView[{i}] {}: expected raise {}, got {raised_rs}",
            c.label,
            c.expect_raise
        );
        assert_eq!(
            msg_rs, msg_c,
            "V_RenderView[{i}] {}: Sys_Error message disagreement",
            c.label
        );
        if c.expect_raise {
            assert!(
                !msg_rs.is_empty(),
                "V_RenderView[{i}] {}: raised with an empty message",
                c.label
            );
            messages_seen += 1;
        } else {
            assert_snap_eq(&format!("V_RenderView[{i}] {}", c.label), &got, &want);
            assert_eq!(want.render_warp, 0, "render_warp not cleared");
            assert_eq!(want.render_scale, 1, "render_scale not reset");
        }
    }

    // Not vacuous: the trap must actually have fired, with real text, on the
    // arms that are supposed to raise.
    assert_eq!(messages_seen, 3, "the Sys_Error arms did not all raise");

    // And the relink-under-tasks message must be view.c's own, not the
    // R_RenderView stand-in's -- otherwise arm 2 and arm 3 would be
    // indistinguishable and the guard topology would be untested.
    {
        let cvars = default_cvars();
        let mut s = base_state();
        s.needs_relink = 1;
        let mut args = RvArgs { use_tasks: true };
        let arg: *mut c_void = (&raw mut args).cast();
        // SAFETY: as above.
        let msg = unsafe {
            ctest_view_reset(24.0);
            ctest_view_set_cvars(cvars.as_ptr());
            ctest_view_apply(&s);
            assert_eq!(ctest_try(rv_plain, arg), 1);
            cstr(ctest_sys_error_message())
        };
        assert_eq!(
            msg, "V_RenderView: entities needed relink in main draw",
            "the port raised the wrong Sys_Error"
        );
    }
}

// ---------------------------------------------------------------------------
// Group 9 -- V_Init (view.c:940).

/// `V_Init` registers 3 commands and 30 cvars. The two halves use genuinely
/// different registries -- the oracle's `c_ref_Cvar_RegisterVariable` walks
/// `Quake/cvar.c`'s list, the port's plain `Cvar_RegisterVariable` walks
/// `quake-capi`'s -- so this compares registry membership, name, default
/// string, flags and the parsed `.value` for each of the 30, and asserts the
/// three cl_input/chase cvars stay out.
///
/// `Cmd_AddCommand` has no plain twin in the ctest link (`stubs.c` provides
/// only the renamed `Cmd_AddCommand2`), so both halves register into
/// `Quake/cmd.c`'s single table -- an identity mapping like `R_RenderView`.
/// Duplicate registration there is non-fatal (`Con_Printf` + `return NULL`),
/// so the plain half runs FIRST and `ctest_view_cmd_handler` then proves the
/// plain (Rust-routed) function pointers are the ones installed.
///
/// MUTATION USED: dropped the `View_Glue_RegisterVariable(&cl_bobup)` call
/// from `quake_rs_v_init` -- killed at `cvar[7] cl_bobup flags`, 0 on the
/// unregistered Rust side against the oracle's CVAR_REGISTERED. Second
/// mutation: swapped the `V_cshift_f` and `V_BonusFlash_f` handlers in
/// `stubs/view_ref.c`'s plain `Cmd_AddCommand` calls -- killed at
/// `command "v_cshift": expected the plain handler 1, got 2`. Restored.
///
/// A third mutation -- swapping the `v_centermove` and `v_centerspeed`
/// registration order -- SURVIVED, and is an equivalent mutant:
/// `Quake/cvar.c:663-679` inserts alphabetically, so the order in which two
/// differently named cvars are registered is not observable in the list, in
/// `cvarlist`, or in `config.cfg`. The source order is preserved in
/// `quake_rs_v_init` for faithfulness, not because it can be tested.
#[test]
fn v_init_registers_like_the_oracle() {
    let _g = lock();

    // SAFETY: single-threaded under TEST_LOCK. The plain half is registered
    // first so it wins the shared cmd.c table (see the doc comment).
    unsafe {
        ctest_view_reset(0.0);
        V_Init();
        c_ref_V_Init();
    }

    for idx in 0..V_INIT_CVARS {
        let mut rs = CvarInfo {
            found: 0,
            flags: 0,
            value: 0.0,
            name: [0; 64],
            string: [0; 64],
        };
        let mut c = rs;
        // SAFETY: the fixture memsets and fills both structs.
        unsafe {
            ctest_view_cvar_info(idx as c_int, 0, &mut rs);
            ctest_view_cvar_info(idx as c_int, 1, &mut c);
        }

        let name = arr_str(&c.name);
        assert_eq!(arr_str(&rs.name), name, "cvar[{idx}] name");
        assert_eq!(
            arr_str(&rs.string),
            arr_str(&c.string),
            "cvar[{idx}] {name} default string"
        );
        assert_eq!(rs.flags, c.flags, "cvar[{idx}] {name} flags");
        assert_bits(&format!("cvar[{idx}] {name} value"), rs.value, c.value);
        assert_eq!(
            rs.found, 1,
            "cvar[{idx}] {name} missing from the Rust registry"
        );
        assert_eq!(
            c.found, 1,
            "cvar[{idx}] {name} missing from the oracle registry"
        );
    }

    // The three cvars V_Init must NOT register (cl_input.c owns two,
    // chase.c the third). If either half registered them the ownership
    // boundary this task drew would be wrong.
    for idx in [CV_CL_FORWARDSPEED, CV_LOOKSPRING, CV_CHASE_ACTIVE] {
        let mut rs = CvarInfo {
            found: 0,
            flags: 0,
            value: 0.0,
            name: [0; 64],
            string: [0; 64],
        };
        let mut c = rs;
        // SAFETY: as above.
        unsafe {
            ctest_view_cvar_info(idx as c_int, 0, &mut rs);
            ctest_view_cvar_info(idx as c_int, 1, &mut c);
        }
        assert_eq!(
            rs.found,
            0,
            "V_Init registered {} on the Rust side",
            arr_str(&rs.name)
        );
        assert_eq!(
            c.found,
            0,
            "V_Init registered {} on the oracle side",
            arr_str(&c.name)
        );
    }

    // Commands: name AND installed handler.
    for (name, want) in [(c"v_cshift", 1), (c"bf", 2), (c"centerview", 3)] {
        // SAFETY: `name` is a 'static NUL-terminated literal.
        let got = unsafe { ctest_view_cmd_handler(name.as_ptr()) };
        assert_eq!(
            got, want,
            "command {name:?}: expected the plain handler {want}, got {got}"
        );
    }
    // A name view.c never registers must stay absent, or ctest_view_cmd_handler
    // is answering 1/2/3 for everything.
    // SAFETY: as above.
    let absent = unsafe { ctest_view_cmd_handler(c"v_not_a_command".as_ptr()) };
    assert_eq!(
        absent, 0,
        "the command probe reports hits for unregistered names"
    );

    // Not vacuous: registration must have parsed the initializer strings.
    // A registry that silently did nothing would leave every .value at the
    // seeded default and every .string empty.
    {
        let mut rs = CvarInfo {
            found: 0,
            flags: 0,
            value: 0.0,
            name: [0; 64],
            string: [0; 64],
        };
        // SAFETY: as above.
        unsafe { ctest_view_cvar_info(CV_CL_ROLLANGLE as c_int, 0, &mut rs) };
        assert_eq!(arr_str(&rs.name), "cl_rollangle");
        assert_eq!(arr_str(&rs.string), "2.0");
        assert_bits("cl_rollangle parsed", rs.value, 2.0);

        // SAFETY: as above.
        unsafe { ctest_view_cvar_info(CV_V_CENTERSPEED as c_int, 0, &mut rs) };
        assert_eq!(arr_str(&rs.string), "500");
        assert_bits("v_centerspeed parsed", rs.value, 500.0);
    }
}
