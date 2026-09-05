//! Differential test: the Rust `quake-capi` client-coupled QuakeC builtins
//! (`rust/quake-capi/src/progs_builtins_cl.rs`) vs the original `Quake/pr_cmds.c`
//! bodies. Rust migration Phase 7, M5 wave 2, Group F: `PF_cl_sound`,
//! `PF_cl_ambientsound`, `PF_cl_precache_sound`, `PF_cl_makestatic`,
//! `PF_cl_particle`.
//!
//! `pr_cmds.c` is not in `build.rs`'s `C_SOURCES`, so there is no `c_ref_PF_*`
//! symbol to call. The oracle is instead a from-scratch C transcription in
//! `stubs/pf_cl_ref.c` (`ctest_cref_pf_cl_*`), driven through that file's own
//! dispatcher (`ctest_cref_pf_cl_run`). See `pf_cl_ref.c`'s header comment for
//! the full design rationale.
//!
//! # Scope limitation (read before extending this file)
//!
//! `Quake/snd_dma.c` (the real `S_StartSound` / `S_StaticSound` /
//! `S_PrecacheSound`) is a real, still-C-side engine file, compiled into this
//! binary alongside `rust/quake-capi/src/snd_dma.rs`'s already-ported, real,
//! unconditionally-linked Rust implementations of the exact same three
//! functions (both dual-linked behind `c_ref_*` renaming for the oracle side
//! vs the plain name for the Rust side -- the standard M5 dual-implementation
//! pattern). Because `sound_started` defaults false in this harness (no
//! `S_Init` / DMA bring-up is performed), BOTH sides' real implementations
//! silently no-op through `PF_cl_sound` / `PF_cl_ambientsound`'s sound calls
//! with no observable side effect reachable from this fixture -- there is no
//! interception seam left to capture what `entnum` / `origin` / volume /
//! attenuation each side computed and handed to `S_StartSound` /
//! `S_StaticSound`. The `sound_*` / `ambientsound_*` tests below are
//! therefore only **control-flow parity** checks (does each side raise or not
//! raise, does it read through to the end without crashing) and do **not**
//! verify the entnum sign flip, the mins/maxs midpoint-origin computation, or
//! the volume*255 truncation. Building a real two-sided fixture for that would
//! need a full DMA channel-state capture (`stubs.c`'s existing
//! `ctest_snd_*` helpers are FNV-1a paint-hashes for `snd_dma_differential.rs`,
//! not per-call channel/argument capture) -- judged disproportionate to this
//! group's scope and left undone.
//!
//! Also out of scope, matching Group E's own documented precedent
//! (`pf_fx_ref.c`'s header, "PR_GetString's Host_Error-raising branch... is
//! not exercised"): a "bad string handle raises" test for `PF_cl_sound`'s
//! `sample` or `PF_cl_ambientsound`'s `samp` parameters. This fixture's
//! `ctest_cl_intern` only ever hands out valid positive offsets, so that raise
//! path is not constructible here.
//!
//! `PF_cl_makestatic` has no oracle dispatch index (ADR-007: kept whole in C,
//! `PRBI_ClGlue_MakeStatic` in `pf_cl_ref.c` is a guard-plumbing probe, not a
//! second independent transcription -- `entity_t` / `cl.static_entities` /
//! `SV_BuildEntityState` / `R_AddEfrags` have no ctest fixture equivalent).
//! Its tests below are Rust-port-vs-probe checks, not two-sided oracle
//! differentials.
//!
//! `PF_cl_particle` IS a true two-sided differential: `Quake/r_part.c` /
//! `r_part_fte.c` are not in `build.rs`'s `C_SOURCES` and never linked, so the
//! `PScript_RunParticleEffectTypeString` / `PScript_RunParticleEffect` /
//! `R_RunParticleEffect` recording doubles in `pf_cl_ref.c` are called by
//! *both* the C oracle transcription and the Rust port with no collision
//! risk, and every branch of its argument marshaling is directly comparable.
//!
//! Raise topology (ADR-009): every entry point here is status-returning on
//! the Rust side (`quake_rs_pf_cl_*(&mut detail)`), and the C oracle is driven
//! through `ctest_cref_pf_cl_run`, which arms the `Host_Error` trap in a C
//! frame. No `longjmp` crosses a Rust frame on either side.

use core::ffi::{c_int, c_void, CStr};
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

// ---------------------------------------------------------------------------
// progs.h offsets (Quake/pr_comp.h:64-73).

const OFS_RETURN: c_int = 1;
const OFS_PARM0: c_int = 4;
const OFS_PARM1: c_int = 7;
const OFS_PARM2: c_int = 10;
const OFS_PARM3: c_int = 13;
const OFS_PARM4: c_int = 16;

/// `ctest_cref_pf_cl_run` / `ctest_cref_pf_cl_dispatch` dispatch indices;
/// must match `pf_cl_ref.c`'s switch.
mod pf {
    pub const SOUND: i32 = 0;
    pub const AMBIENTSOUND: i32 = 1;
    pub const PRECACHE_SOUND: i32 = 2;
    pub const PARTICLE: i32 = 3;
}

extern "C" {
    // --- fixture ------------------------------------------------------
    fn ctest_cl_reset_fixture(num_edicts: c_int);
    fn ctest_cl_intern(s: *const core::ffi::c_char) -> c_int;
    fn ctest_cl_set_global_float(ofs: c_int, v: f32);
    fn ctest_cl_set_global_int(ofs: c_int, v: c_int);
    fn ctest_cl_set_global_vector(ofs: c_int, x: f32, y: f32, z: f32);
    fn ctest_cl_get_global_int(ofs: c_int) -> c_int;
    fn ctest_cl_edict_to_prog(num: c_int) -> c_int;
    fn ctest_cl_edict_ptr(num: c_int) -> *mut c_void;
    fn ctest_cl_edict_set_physics(
        num: c_int,
        mins: *const f32,
        maxs: *const f32,
        origin: *const f32,
    );

    fn ctest_host_error_message() -> *const core::ffi::c_char;
    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(i: c_int) -> *const core::ffi::c_char;

    // --- oracle dispatcher ---------------------------------------------
    fn ctest_cref_pf_cl_run(which: c_int) -> c_int;

    // --- PScript_RunParticleEffectTypeString recorder -------------------
    fn ctest_cl_pscript_typestring_called() -> c_int;
    fn ctest_cl_pscript_typestring_count() -> f32;
    fn ctest_cl_pscript_typestring_name() -> *const core::ffi::c_char;
    fn ctest_cl_pscript_typestring_set_return(ret: c_int);

    // --- PScript_RunParticleEffect recorder ------------------------------
    fn ctest_cl_pscript_effect_called() -> c_int;
    fn ctest_cl_pscript_effect_color() -> c_int;
    fn ctest_cl_pscript_effect_count() -> c_int;
    fn ctest_cl_pscript_effect_set_return(ret: c_int);

    // --- R_RunParticleEffect recorder ------------------------------------
    fn ctest_cl_runparticleeffect_called() -> c_int;
    fn ctest_cl_runparticleeffect_color() -> c_int;
    fn ctest_cl_runparticleeffect_count() -> c_int;

    // --- PF_cl_makestatic probe (pf_cl_ref.c) ----------------------------
    fn ctest_cl_makestatic_set_fail(fail: c_int);
    fn ctest_cl_makestatic_calls_get() -> c_int;
    fn ctest_cl_makestatic_last_ent_get() -> *mut c_void;

    // --- Rust builtins under test ----------------------------------------
    fn quake_rs_pf_cl_sound(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_cl_ambientsound(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_cl_precache_sound(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_cl_makestatic(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_cl_particle(detail: *mut c_int) -> c_int;
}

// ---------------------------------------------------------------------------
// Small typed wrappers, so the test bodies stay unsafe-free.

fn reset(num_edicts: i32) {
    // SAFETY: plain C fixture reset, no arguments beyond a count.
    unsafe { ctest_cl_reset_fixture(num_edicts) };
}

fn intern(s: &str) -> i32 {
    let c = std::ffi::CString::new(s).unwrap();
    // SAFETY: `c` is NUL-terminated and outlives the call.
    unsafe { ctest_cl_intern(c.as_ptr()) }
}

fn set_global_i(ofs: i32, v: i32) {
    // SAFETY: `ofs` is a progs.h OFS_ constant, always in-range for this fixture.
    unsafe { ctest_cl_set_global_int(ofs, v) };
}

fn set_global_f(ofs: i32, v: f32) {
    // SAFETY: as `set_global_i`.
    unsafe { ctest_cl_set_global_float(ofs, v) };
}

fn set_global_vec(ofs: i32, v: [f32; 3]) {
    // SAFETY: as `set_global_i`; the 3 floats at ofs..ofs+3 stay in bounds.
    unsafe { ctest_cl_set_global_vector(ofs, v[0], v[1], v[2]) };
}

fn get_ofs_return_i() -> i32 {
    // SAFETY: as above.
    unsafe { ctest_cl_get_global_int(OFS_RETURN) }
}

fn edict_prog(num: i32) -> i32 {
    // SAFETY: `num` indexes the fixture arena built by `ctest_cl_reset_fixture`.
    unsafe { ctest_cl_edict_to_prog(num) }
}

fn edict_ptr(num: i32) -> *mut c_void {
    // SAFETY: as `edict_prog`.
    unsafe { ctest_cl_edict_ptr(num) }
}

fn edict_set_physics(num: i32, mins: [f32; 3], maxs: [f32; 3], origin: [f32; 3]) {
    // SAFETY: fixed-size 3-float arrays, matches the C signature exactly.
    unsafe { ctest_cl_edict_set_physics(num, mins.as_ptr(), maxs.as_ptr(), origin.as_ptr()) };
}

fn con_log() -> Vec<String> {
    // SAFETY: `ctest_con_log_get` returns a NUL-terminated buffer that
    // outlives this call.
    unsafe {
        (0..ctest_con_log_len())
            .map(|i| {
                CStr::from_ptr(ctest_con_log_get(i))
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }
}

fn typestring_called() -> i32 {
    // SAFETY: plain C getter, no arguments.
    unsafe { ctest_cl_pscript_typestring_called() }
}

fn typestring_count() -> f32 {
    // SAFETY: as above.
    unsafe { ctest_cl_pscript_typestring_count() }
}

fn typestring_name() -> String {
    // SAFETY: returns a NUL-terminated buffer that outlives this call.
    unsafe { CStr::from_ptr(ctest_cl_pscript_typestring_name()) }
        .to_string_lossy()
        .into_owned()
}

fn typestring_set_return(ret: bool) {
    // SAFETY: plain C setter.
    unsafe { ctest_cl_pscript_typestring_set_return(if ret { 1 } else { 0 }) };
}

fn effect_called() -> i32 {
    // SAFETY: plain C getter.
    unsafe { ctest_cl_pscript_effect_called() }
}

fn effect_color() -> i32 {
    // SAFETY: as above.
    unsafe { ctest_cl_pscript_effect_color() }
}

fn effect_count() -> i32 {
    // SAFETY: as above.
    unsafe { ctest_cl_pscript_effect_count() }
}

fn effect_set_return(ret: bool) {
    // SAFETY: plain C setter.
    unsafe { ctest_cl_pscript_effect_set_return(if ret { 1 } else { 0 }) };
}

fn runparticle_called() -> i32 {
    // SAFETY: plain C getter.
    unsafe { ctest_cl_runparticleeffect_called() }
}

fn runparticle_color() -> i32 {
    // SAFETY: as above.
    unsafe { ctest_cl_runparticleeffect_color() }
}

fn runparticle_count() -> i32 {
    // SAFETY: as above.
    unsafe { ctest_cl_runparticleeffect_count() }
}

fn makestatic_set_fail(fail: bool) {
    // SAFETY: plain C setter.
    unsafe { ctest_cl_makestatic_set_fail(if fail { 1 } else { 0 }) };
}

fn makestatic_calls() -> i32 {
    // SAFETY: plain C getter.
    unsafe { ctest_cl_makestatic_calls_get() }
}

fn makestatic_last_ent() -> *mut c_void {
    // SAFETY: as above.
    unsafe { ctest_cl_makestatic_last_ent_get() }
}

// ---------------------------------------------------------------------------
// Side dispatch (PF_cl_sound / PF_cl_ambientsound / PF_cl_precache_sound /
// PF_cl_particle only -- PF_cl_makestatic has no oracle index, see module doc).

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    C,
    Rust,
}

/// A builtin's raise outcome, normalised across the two very different
/// reporting conventions: the oracle longjmps and `ctest_cref_pf_cl_run`
/// reports 1, the port returns `PRBI_ERR_GUARD` with
/// `detail == CTEST_GUARD_HOST_ERROR` (ADR-009). Both leave the text in
/// `ctest_host_error_message()`.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Outcome {
    raised: bool,
    message: String,
}

/// `PRBI_ERR_GUARD` (`rust/quake-capi/src/progs_builtins_sv.rs:68`, mirrors
/// `Quake/pr_cmds_glue.c`'s enum).
const PRBI_ERR_GUARD: c_int = 3;
/// `CTEST_GUARD_HOST_ERROR` (`stubs.c`).
const CTEST_GUARD_HOST_ERROR: c_int = 1;

impl Side {
    /// Runs one of the four oracle-backed builtins and normalises its raise
    /// reporting.
    fn run(self, which: i32) -> Outcome {
        let raised = match self {
            Side::C => {
                // SAFETY: the dispatcher arms the Host_Error trap inside a C
                // frame and dispatches on `which`.
                let r = unsafe { ctest_cref_pf_cl_run(which) };
                assert!(r == 0 || r == 1, "oracle raised Sys_Error ({r})");
                r == 1
            }
            Side::Rust => {
                let mut detail: c_int = -1;
                // SAFETY: every port entry point takes `&detail` exactly as
                // `RUST_PF` passes it and returns a PRBI_* status.
                let status = unsafe {
                    match which {
                        pf::SOUND => quake_rs_pf_cl_sound(&mut detail),
                        pf::AMBIENTSOUND => quake_rs_pf_cl_ambientsound(&mut detail),
                        pf::PRECACHE_SOUND => quake_rs_pf_cl_precache_sound(&mut detail),
                        pf::PARTICLE => quake_rs_pf_cl_particle(&mut detail),
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

/// Resets the shared fixture, runs `setup` then the given builtin on `side`,
/// and returns the outcome. `setup` must be side-independent (same intern
/// order / same global writes) so that both sides see identical state.
fn run_side(side: Side, num_edicts: i32, setup: impl Fn(), which: i32) -> Outcome {
    reset(num_edicts);
    setup();
    side.run(which)
}

// ===========================================================================
// PF_cl_sound
// ===========================================================================

#[test]
fn sound_bad_edict_raises_num_for_edict_and_matches_oracle() {
    let _g = lock();
    let setup = || {
        // A prog offset far past the fixture's tiny arena: NUM_FOR_EDICT
        // range-checks against qcvm->edicts/num_edicts and raises before any
        // sound call is reached (progs.h's G_EDICT itself does not
        // range-check, so this is the first validation either side hits).
        set_global_i(OFS_PARM0, 1_000_000);
        set_global_f(OFS_PARM1, 1.0);
        let sample = intern("weapons/x.wav");
        set_global_i(OFS_PARM2, sample);
        set_global_f(OFS_PARM3, 1.0);
        set_global_f(OFS_PARM4, 1.0);
    };

    let c_outcome = run_side(Side::C, 1, setup, pf::SOUND);
    let r_outcome = run_side(Side::Rust, 1, setup, pf::SOUND);

    assert!(c_outcome.raised);
    assert_eq!(c_outcome, r_outcome);
    assert!(c_outcome.message.contains("NUM_FOR_EDICT: bad pointer"));
}

#[test]
fn sound_happy_path_does_not_raise_on_either_side() {
    let _g = lock();
    // Scope limitation (see module doc): both sides' real S_StartSound /
    // S_PrecacheSound silently no-op (sound_started == false in this
    // harness), so this is a control-flow parity check only -- it does NOT
    // verify the entnum sign flip, the mins/maxs midpoint origin, or any
    // sound-call argument.
    let setup = || {
        edict_set_physics(
            1,
            [-8.0, -8.0, -8.0],
            [8.0, 8.0, 8.0],
            [100.0, 200.0, 300.0],
        );
        set_global_i(OFS_PARM0, edict_prog(1));
        set_global_f(OFS_PARM1, 2.0);
        let sample = intern("weapons/tst.wav");
        set_global_i(OFS_PARM2, sample);
        set_global_f(OFS_PARM3, 0.7);
        set_global_f(OFS_PARM4, 0.9);
    };

    let c_outcome = run_side(Side::C, 2, setup, pf::SOUND);
    let r_outcome = run_side(Side::Rust, 2, setup, pf::SOUND);

    assert_eq!(c_outcome, r_outcome);
    assert!(
        !c_outcome.raised,
        "a normal PF_cl_sound call must not raise"
    );
}

// ===========================================================================
// PF_cl_ambientsound
// ===========================================================================

#[test]
fn ambientsound_happy_path_does_not_raise_on_either_side() {
    let _g = lock();
    // Same scope limitation as sound_happy_path_does_not_raise_on_either_side:
    // control-flow parity only, no sound-call argument verification.
    let setup = || {
        set_global_vec(OFS_PARM0, [1.0, 2.0, 3.0]);
        let samp = intern("ambience/hum.wav");
        set_global_i(OFS_PARM1, samp);
        set_global_f(OFS_PARM2, 0.5);
        set_global_f(OFS_PARM3, 1.0);
    };

    let c_outcome = run_side(Side::C, 1, setup, pf::AMBIENTSOUND);
    let r_outcome = run_side(Side::Rust, 1, setup, pf::AMBIENTSOUND);

    assert_eq!(c_outcome, r_outcome);
    assert!(
        !c_outcome.raised,
        "a normal PF_cl_ambientsound call must not raise"
    );
}

// ===========================================================================
// PF_cl_precache_sound
// ===========================================================================

#[test]
fn precache_sound_success_echoes_handle_and_does_not_raise() {
    let _g = lock();
    // Both sides reset then intern this one string first, so the offset
    // (1: offset 0 is reserved as "") is identical and deterministic on
    // both sides -- do not re-derive it with a third intern() call after
    // run_side, which would append to whichever fixture is currently live
    // and return an unrelated later offset (caught by this test's first
    // draft, which asserted against exactly that bogus recomputed value).
    let setup = || {
        let s = intern("ambience/hum1.wav");
        set_global_i(OFS_PARM0, s);
    };

    let c_outcome = run_side(Side::C, 1, setup, pf::PRECACHE_SOUND);
    let c_ret = get_ofs_return_i();

    let r_outcome = run_side(Side::Rust, 1, setup, pf::PRECACHE_SOUND);
    let r_ret = get_ofs_return_i();

    assert_eq!(c_outcome, r_outcome);
    assert!(!c_outcome.raised);
    assert_eq!(c_ret, r_ret);
    // pr_cmds.c:1875 echoes G_INT(OFS_PARM0) into OFS_RETURN unconditionally,
    // before PR_CheckEmptyString even runs -- the handle value itself, not a
    // boolean or a recomputed offset.
    assert_eq!(
        c_ret, 1,
        "offset 1: the first string interned after a reset"
    );
}

#[test]
fn precache_sound_empty_string_raises_program_error_and_matches_oracle() {
    let _g = lock();
    let setup = || {
        set_global_i(OFS_PARM0, 0); // offset 0 == ""
    };

    let c_outcome = run_side(Side::C, 1, setup, pf::PRECACHE_SOUND);
    let c_ret = get_ofs_return_i();
    let c_log = con_log();

    let r_outcome = run_side(Side::Rust, 1, setup, pf::PRECACHE_SOUND);
    let r_ret = get_ofs_return_i();
    let r_log = con_log();

    assert!(c_outcome.raised);
    assert_eq!(c_outcome, r_outcome);
    // PR_RunError (pr_exec.c:190-207) always hands Host_Error the literal
    // "Program error" -- the formatted message ("Bad string" here) only ever
    // reaches Con_Printf, never Host_Error itself. Bug-for-bug engine
    // behavior (see pf_cl_ref.c's ctest_cref_pf_cl_precache_sound comment,
    // and progs_builtins_sv_fx_differential.rs's identical precedent): an
    // earlier draft of this oracle transcription called
    // Host_Error("Bad string") directly and would have passed a weaker
    // assertion here while diverging from the real engine's behavior.
    assert_eq!(c_outcome.message, "Program error");
    assert_eq!(c_log, r_log);
    assert!(
        c_log.iter().any(|l| l.contains("Bad string")),
        "expected PR_RunError's formatted message on the console, got {c_log:?}"
    );
    // pr_cmds.c:1875 writes OFS_RETURN unconditionally, before the check.
    assert_eq!(c_ret, 0);
    assert_eq!(r_ret, 0);
}

// ===========================================================================
// PF_cl_makestatic (no oracle dispatch index -- Rust port vs C probe, see
// module doc's ADR-007 note)
// ===========================================================================

#[test]
fn makestatic_guard_ok_forwards_correct_edict_pointer() {
    let _g = lock();
    reset(2);
    let expected_ptr = edict_ptr(1);
    set_global_i(OFS_PARM0, edict_prog(1));

    let mut detail: c_int = -1;
    // SAFETY: quake_rs_pf_cl_makestatic takes &detail exactly as RUST_PF
    // passes it and returns a PRBI_* status.
    let status = unsafe { quake_rs_pf_cl_makestatic(&mut detail) };

    assert_eq!(status, 0, "guard-ok path must return PRBI_OK");
    assert_eq!(makestatic_calls(), 1);
    assert_eq!(makestatic_last_ent(), expected_ptr);
}

#[test]
fn makestatic_guard_raises_too_many_static_entities() {
    let _g = lock();
    reset(2);
    makestatic_set_fail(true);
    set_global_i(OFS_PARM0, edict_prog(1));

    let mut detail: c_int = -1;
    // SAFETY: as above.
    let status = unsafe { quake_rs_pf_cl_makestatic(&mut detail) };

    assert_eq!(status, PRBI_ERR_GUARD);
    assert_eq!(detail, CTEST_GUARD_HOST_ERROR);
    // SAFETY: the stub returns a NUL-terminated buffer that outlives this.
    let message = unsafe { CStr::from_ptr(ctest_host_error_message()) }
        .to_string_lossy()
        .into_owned();
    assert_eq!(message, "Too many static entities");
    assert_eq!(makestatic_calls(), 1);
}

// ===========================================================================
// PF_cl_particle
// ===========================================================================

#[test]
fn particle_count_255_typestring_succeeds_forwards_1024() {
    let _g = lock();
    let setup = || {
        typestring_set_return(true);
        set_global_vec(OFS_PARM0, [1.0, 2.0, 3.0]);
        set_global_vec(OFS_PARM1, [0.0, 0.0, 1.0]);
        set_global_f(OFS_PARM2, 73.0); // color, unused on this branch
        set_global_f(OFS_PARM3, 255.0); // count == 255 selects the typestring branch
    };

    let c_outcome = run_side(Side::C, 1, setup, pf::PARTICLE);
    let c_ts_called = typestring_called();
    let c_ts_count = typestring_count();
    let c_ts_name = typestring_name();
    let c_run_called = runparticle_called();
    let c_run_count = runparticle_count();
    let c_run_color = runparticle_color();

    let r_outcome = run_side(Side::Rust, 1, setup, pf::PARTICLE);
    let r_ts_called = typestring_called();
    let r_ts_count = typestring_count();
    let r_ts_name = typestring_name();
    let r_run_called = runparticle_called();
    let r_run_count = runparticle_count();
    let r_run_color = runparticle_color();

    assert_eq!(c_outcome, r_outcome);
    assert!(!c_outcome.raised);
    assert_eq!(c_ts_called, 1);
    assert_eq!(c_ts_called, r_ts_called);
    assert_eq!(
        c_ts_count, 1.0,
        "PScript_RunParticleEffectTypeString's count arg is always 1"
    );
    assert_eq!(c_ts_count, r_ts_count);
    assert_eq!(c_ts_name, "te_explosion");
    assert_eq!(c_ts_name, r_ts_name);
    assert_eq!(c_run_called, 1);
    assert_eq!(c_run_called, r_run_called);
    assert_eq!(c_run_count, 1024, "typestring success forwards count=1024");
    assert_eq!(c_run_count, r_run_count);
    assert_eq!(c_run_color, 73);
    assert_eq!(c_run_color, r_run_color);
}

#[test]
fn particle_count_255_typestring_fails_forwards_0() {
    let _g = lock();
    let setup = || {
        typestring_set_return(false);
        set_global_vec(OFS_PARM0, [1.0, 2.0, 3.0]);
        set_global_vec(OFS_PARM1, [0.0, 0.0, 1.0]);
        set_global_f(OFS_PARM2, 9.0);
        set_global_f(OFS_PARM3, 255.0);
    };

    let c_outcome = run_side(Side::C, 1, setup, pf::PARTICLE);
    let c_run_count = runparticle_count();

    let r_outcome = run_side(Side::Rust, 1, setup, pf::PARTICLE);
    let r_run_count = runparticle_count();

    assert_eq!(c_outcome, r_outcome);
    assert!(!c_outcome.raised);
    assert_eq!(c_run_count, 0, "typestring failure forwards count=0");
    assert_eq!(c_run_count, r_run_count);
}

#[test]
fn particle_count_not_255_effect_succeeds_forwards_unchanged_count() {
    let _g = lock();
    let setup = || {
        effect_set_return(true);
        set_global_vec(OFS_PARM0, [4.0, 5.0, 6.0]);
        set_global_vec(OFS_PARM1, [1.0, 0.0, 0.0]);
        set_global_f(OFS_PARM2, 20.0);
        set_global_f(OFS_PARM3, 30.0); // count != 255 selects the effect branch
    };

    let c_outcome = run_side(Side::C, 1, setup, pf::PARTICLE);
    let c_eff_called = effect_called();
    let c_eff_color = effect_color();
    let c_eff_count = effect_count();
    let c_run_count = runparticle_count();
    let c_run_color = runparticle_color();

    let r_outcome = run_side(Side::Rust, 1, setup, pf::PARTICLE);
    let r_eff_called = effect_called();
    let r_eff_color = effect_color();
    let r_eff_count = effect_count();
    let r_run_count = runparticle_count();
    let r_run_color = runparticle_color();

    assert_eq!(c_outcome, r_outcome);
    assert!(!c_outcome.raised);
    assert_eq!(c_eff_called, 1);
    assert_eq!(c_eff_called, r_eff_called);
    assert_eq!(c_eff_color, 20);
    assert_eq!(c_eff_color, r_eff_color);
    assert_eq!(c_eff_count, 30);
    assert_eq!(c_eff_count, r_eff_count);
    assert_eq!(
        c_run_count, 30,
        "effect success forwards count UNCHANGED (not reset to a fixed value)"
    );
    assert_eq!(c_run_count, r_run_count);
    assert_eq!(c_run_color, 20);
    assert_eq!(c_run_color, r_run_color);
}

#[test]
fn particle_count_not_255_effect_fails_forwards_0() {
    let _g = lock();
    let setup = || {
        effect_set_return(false);
        set_global_vec(OFS_PARM0, [4.0, 5.0, 6.0]);
        set_global_vec(OFS_PARM1, [1.0, 0.0, 0.0]);
        set_global_f(OFS_PARM2, 20.0);
        set_global_f(OFS_PARM3, 30.0);
    };

    let c_outcome = run_side(Side::C, 1, setup, pf::PARTICLE);
    let c_run_count = runparticle_count();

    let r_outcome = run_side(Side::Rust, 1, setup, pf::PARTICLE);
    let r_run_count = runparticle_count();

    assert_eq!(c_outcome, r_outcome);
    assert!(!c_outcome.raised);
    assert_eq!(c_run_count, 0, "effect failure forwards count=0");
    assert_eq!(c_run_count, r_run_count);
}
