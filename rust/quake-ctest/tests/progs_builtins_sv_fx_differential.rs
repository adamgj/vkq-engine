//! Differential test: the Rust `quake-capi` world-effect QuakeC builtins
//! (`rust/quake-capi/src/progs_builtins_sv_fx.rs`) vs the original
//! `Quake/pr_cmds.c` bodies. Rust migration Phase 7, M5 wave 2, Group E.
//!
//! `pr_cmds.c` is not in `build.rs`'s `C_SOURCES`, so there is no
//! `c_ref_PF_*` symbol to call. The oracle is instead a set of independent,
//! from-scratch C transcriptions in `stubs/pf_fx_ref.c`
//! (`ctest_fx_oracle_pf_*`), driven through that file's own dispatcher
//! (`ctest_fx_pf_run`). See `pf_fx_ref.c`'s header comment for the full
//! design rationale (why this is not circular, the two-independent-
//! transcriptions structure, and the documented gaps).
//!
//! Only 7 of Group E's 12 builtins have a real oracle: `PF_particle`,
//! `PF_sound`, `PF_sv_precache_sound`, `PF_sv_precache_model`,
//! `PF_sv_finalefinished`, `PF_sv_CheckPlayerEXFlags`, `PF_sv_changelevel`.
//! The other 5 (`PF_sv_ambientsound`, `PF_sv_lightstyle`, `PF_sv_makestatic`,
//! `PF_sv_setspawnparms`, `PF_sv_localsound`) need a `svs`/`client_t`
//! equivalent this ctest fixture does not have (see `pf_fx_ref.c`'s header)
//! and are intentionally not covered here.
//!
//! Both sides run against ONE shared fixture (`ctest_fx_*`), fully rebuilt
//! from scratch (`ctest_fx_reset`) immediately before each side's run, per
//! the idiom `progs_builtins_sv_differential.rs` established in wave 1.
//!
//! Raise topology (ADR-009): every entry point here is status-returning on
//! the Rust side (`quake_rs_pf_*(&mut detail)`), and the C oracle is driven
//! through `ctest_fx_pf_run`, which arms the `Host_Error` trap in a C frame.
//! No `longjmp` crosses a Rust frame on either side.

use core::ffi::{c_int, CStr};
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

/// `ctest_fx_pf_run` / `ctest_fx_pf_dispatch` dispatch indices; must match
/// `pf_fx_ref.c`'s enum.
mod pf {
    pub const PARTICLE: i32 = 0;
    pub const SOUND: i32 = 1;
    pub const PRECACHE_SOUND: i32 = 2;
    pub const PRECACHE_MODEL: i32 = 3;
    pub const FINALEFINISHED: i32 = 4;
    pub const CHECK_PLAYER_EX_FLAGS: i32 = 5;
    pub const CHANGELEVEL: i32 = 6;
}

extern "C" {
    // --- fixture ------------------------------------------------------
    fn ctest_fx_reset(num_edicts: c_int);
    fn ctest_fx_intern(s: *const core::ffi::c_char) -> c_int;
    fn ctest_fx_set_global_float(ofs: c_int, v: f32);
    fn ctest_fx_set_global_int(ofs: c_int, v: c_int);
    fn ctest_fx_set_global_vector(ofs: c_int, x: f32, y: f32, z: f32);
    fn ctest_fx_get_global_float(ofs: c_int) -> f32;
    fn ctest_fx_get_global_int(ofs: c_int) -> c_int;
    fn ctest_fx_edict_to_prog(num: c_int) -> c_int;

    fn ctest_host_error_message() -> *const core::ffi::c_char;
    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(i: c_int) -> *const core::ffi::c_char;

    // --- oracle dispatcher ---------------------------------------------
    fn ctest_fx_pf_run(which: c_int) -> c_int;

    // --- PF_particle recorder ------------------------------------------
    fn ctest_fx_particle_calls() -> c_int;
    fn ctest_fx_particle_get(org: *mut f32, dir: *mut f32, color: *mut c_int, count: *mut c_int);

    // --- SV_StartSound recorder (stubs.c, reused) -----------------------
    fn ctest_phys_sound_arm_raise(on: c_int);
    fn ctest_phys_sound_len() -> c_int;
    fn ctest_phys_sound_get(
        i: c_int,
        ent: *mut c_int,
        channel: *mut c_int,
        volume: *mut c_int,
        attenuation: *mut f32,
        has_origin: *mut c_int,
        sample: *mut *const core::ffi::c_char,
    ) -> c_int;

    // --- SV_Precache_Sound double (stubs.c, reused) ---------------------
    fn ctest_predd_get_last_sound() -> *const core::ffi::c_char;

    // --- PF_sv_precache_model mock --------------------------------------
    fn ctest_fx_set_ss_loading(loading: bool);
    fn ctest_fx_model_slot_used(i: c_int) -> c_int;

    // --- world.c cvars (stubs.c) -- pr_checkextension lives here, not in
    // ctest_fx_reset/ctest_world_reset, which never touch it. recursivehullcheck
    // and createareanode are irrelevant to Group E's builtins; 0.0 is a
    // harmless placeholder.
    fn ctest_world_set_cvars(recursivehullcheck: f32, createareanode: f32, checkextension: f32);

    // --- PF_sv_changelevel mock ------------------------------------------
    fn ctest_fx_changelevel_set_issued(v: bool);
    fn ctest_fx_changelevel_get_issued() -> bool;
    fn ctest_fx_changelevel_calls() -> c_int;
    fn ctest_fx_changelevel_last() -> *const core::ffi::c_char;

    // --- Rust builtins under test ----------------------------------------
    fn quake_rs_pf_particle(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sound(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_precache_sound(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_precache_model(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_finalefinished(detail: *mut c_int) -> c_int;
    #[allow(non_snake_case)]
    fn quake_rs_pf_sv_CheckPlayerEXFlags(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_changelevel(detail: *mut c_int) -> c_int;
}

// ---------------------------------------------------------------------------
// Small typed wrappers, so the test bodies stay unsafe-free.

fn reset(num_edicts: i32) {
    // SAFETY: plain C fixture reset, no arguments beyond a count.
    unsafe { ctest_fx_reset(num_edicts) };
}

fn intern(s: &str) -> i32 {
    let c = std::ffi::CString::new(s).unwrap();
    // SAFETY: `c` is NUL-terminated and outlives the call.
    unsafe { ctest_fx_intern(c.as_ptr()) }
}

fn set_global_i(ofs: i32, v: i32) {
    // SAFETY: `ofs` is a progs.h OFS_ constant, always in-range for this fixture.
    unsafe { ctest_fx_set_global_int(ofs, v) };
}

fn set_global_f(ofs: i32, v: f32) {
    // SAFETY: as `set_global_i`.
    unsafe { ctest_fx_set_global_float(ofs, v) };
}

fn set_global_vec(ofs: i32, v: [f32; 3]) {
    // SAFETY: as `set_global_i`; the 3 floats at ofs..ofs+3 stay in bounds.
    unsafe { ctest_fx_set_global_vector(ofs, v[0], v[1], v[2]) };
}

fn get_ofs_return_f() -> f32 {
    // SAFETY: OFS_RETURN is always in-range for this fixture.
    unsafe { ctest_fx_get_global_float(OFS_RETURN) }
}

fn get_ofs_return_i() -> i32 {
    // SAFETY: as above.
    unsafe { ctest_fx_get_global_int(OFS_RETURN) }
}

fn edict_prog(num: i32) -> i32 {
    // SAFETY: `num` indexes the fixture arena built by `ctest_fx_reset`.
    unsafe { ctest_fx_edict_to_prog(num) }
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

fn particle_calls() -> i32 {
    // SAFETY: plain C getter, no arguments.
    unsafe { ctest_fx_particle_calls() }
}

fn particle_get() -> ([f32; 3], [f32; 3], i32, i32) {
    let mut org = [0f32; 3];
    let mut dir = [0f32; 3];
    let mut color = 0i32;
    let mut count = 0i32;
    // SAFETY: fixed-size out-params, matches the C signature exactly.
    unsafe { ctest_fx_particle_get(org.as_mut_ptr(), dir.as_mut_ptr(), &mut color, &mut count) };
    (org, dir, color, count)
}

#[derive(Debug, PartialEq)]
struct SoundRec {
    ent: i32,
    channel: i32,
    volume: i32,
    attenuation: f32,
    has_origin: bool,
    sample: String,
}

fn sound_len() -> i32 {
    // SAFETY: plain C getter.
    unsafe { ctest_phys_sound_len() }
}

fn sound_get(i: i32) -> Option<SoundRec> {
    let mut ent = 0i32;
    let mut channel = 0i32;
    let mut volume = 0i32;
    let mut attenuation = 0f32;
    let mut has_origin = 0i32;
    let mut sample: *const core::ffi::c_char = core::ptr::null();
    // SAFETY: fixed-size out-params, matches the C signature exactly.
    let ok = unsafe {
        ctest_phys_sound_get(
            i,
            &mut ent,
            &mut channel,
            &mut volume,
            &mut attenuation,
            &mut has_origin,
            &mut sample,
        )
    };
    if ok == 0 {
        return None;
    }
    Some(SoundRec {
        ent,
        channel,
        volume,
        attenuation,
        has_origin: has_origin != 0,
        // SAFETY: `sample` was just filled by `ctest_phys_sound_get` above.
        sample: unsafe { CStr::from_ptr(sample) }
            .to_string_lossy()
            .into_owned(),
    })
}

fn predd_last_sound() -> String {
    // SAFETY: returns a NUL-terminated buffer that outlives this call.
    unsafe { CStr::from_ptr(ctest_predd_get_last_sound()) }
        .to_string_lossy()
        .into_owned()
}

fn model_slot_used(i: i32) -> bool {
    // SAFETY: plain C getter, bounds-checked internally.
    unsafe { ctest_fx_model_slot_used(i) != 0 }
}

fn set_ss_loading(loading: bool) {
    // SAFETY: plain C setter.
    unsafe { ctest_fx_set_ss_loading(loading) };
}

fn set_checkextension(on: bool) {
    // SAFETY: plain C setter; recursivehullcheck/createareanode are unused
    // by Group E's builtins, so 0.0 is a harmless placeholder for both.
    unsafe { ctest_world_set_cvars(0.0, 0.0, if on { 1.0 } else { 0.0 }) };
}

fn changelevel_set_issued(v: bool) {
    // SAFETY: plain C setter.
    unsafe { ctest_fx_changelevel_set_issued(v) };
}

fn changelevel_get_issued() -> bool {
    // SAFETY: plain C getter.
    unsafe { ctest_fx_changelevel_get_issued() }
}

fn changelevel_calls() -> i32 {
    // SAFETY: plain C getter.
    unsafe { ctest_fx_changelevel_calls() }
}

fn changelevel_last() -> String {
    // SAFETY: returns a NUL-terminated buffer that outlives this call.
    unsafe { CStr::from_ptr(ctest_fx_changelevel_last()) }
        .to_string_lossy()
        .into_owned()
}

fn sound_arm_raise(on: bool) {
    // SAFETY: plain C setter.
    unsafe { ctest_phys_sound_arm_raise(if on { 1 } else { 0 }) };
}

// ---------------------------------------------------------------------------
// Side dispatch.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    C,
    Rust,
}

/// A builtin's raise outcome, normalised across the two very different
/// reporting conventions: the oracle longjmps and `ctest_try_host` reports 1,
/// the port returns `PRBI_ERR_GUARD` with `detail == CTEST_GUARD_HOST_ERROR`
/// (ADR-009). Both leave the text in `ctest_host_error_message()`.
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
    /// Runs one builtin and normalises its raise reporting.
    fn run(self, which: i32) -> Outcome {
        let raised = match self {
            Side::C => {
                // SAFETY: the dispatcher arms the Host_Error trap inside a C
                // frame and dispatches on `which`.
                let r = unsafe { ctest_fx_pf_run(which) };
                assert!(r == 0 || r == 1, "oracle raised Sys_Error ({r})");
                r == 1
            }
            Side::Rust => {
                let mut detail: c_int = -1;
                // SAFETY: every port entry point takes `&detail` exactly as
                // `RUST_PF` passes it and returns a PRBI_* status.
                let status = unsafe {
                    match which {
                        pf::PARTICLE => quake_rs_pf_particle(&mut detail),
                        pf::SOUND => quake_rs_pf_sound(&mut detail),
                        pf::PRECACHE_SOUND => quake_rs_pf_sv_precache_sound(&mut detail),
                        pf::PRECACHE_MODEL => quake_rs_pf_sv_precache_model(&mut detail),
                        pf::FINALEFINISHED => quake_rs_pf_sv_finalefinished(&mut detail),
                        pf::CHECK_PLAYER_EX_FLAGS => quake_rs_pf_sv_CheckPlayerEXFlags(&mut detail),
                        pf::CHANGELEVEL => quake_rs_pf_sv_changelevel(&mut detail),
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
// PF_particle
// ===========================================================================

#[test]
fn particle_matches_c_oracle() {
    let _g = lock();
    let setup = || {
        set_global_vec(OFS_PARM0, [10.0, -20.0, 30.0]);
        set_global_vec(OFS_PARM1, [1.0, 0.0, 0.0]);
        set_global_f(OFS_PARM2, 5.0);
        set_global_f(OFS_PARM3, 12.0);
    };

    let c_outcome = run_side(Side::C, 1, setup, pf::PARTICLE);
    let (c_org, c_dir, c_color, c_count) = particle_get();
    let c_calls = particle_calls();

    let r_outcome = run_side(Side::Rust, 1, setup, pf::PARTICLE);
    let (r_org, r_dir, r_color, r_count) = particle_get();
    let r_calls = particle_calls();

    assert_eq!(c_outcome, r_outcome);
    assert!(!c_outcome.raised, "PF_particle must never raise");
    assert_eq!(c_calls, 1);
    assert_eq!(r_calls, 1);
    assert_eq!(c_org, r_org);
    assert_eq!(c_dir, r_dir);
    assert_eq!(c_color, r_color);
    assert_eq!(c_count, r_count);
    assert_eq!(c_org, [10.0, -20.0, 30.0]);
    assert_eq!(c_color, 5);
    assert_eq!(c_count, 12);
}

// ===========================================================================
// PF_sound
// ===========================================================================

#[test]
fn sound_nonempty_starts_sound_matches_oracle() {
    let _g = lock();
    let setup = || {
        let sample = intern("weapons/tst1.wav");
        set_global_i(OFS_PARM0, edict_prog(1));
        set_global_f(OFS_PARM1, 3.0); // channel
        set_global_i(OFS_PARM2, sample);
        set_global_f(OFS_PARM3, 0.5); // volume, *255 -> 127
        set_global_f(OFS_PARM4, 0.8); // attenuation
    };

    let c_outcome = run_side(Side::C, 2, setup, pf::SOUND);
    let c_rec = sound_get(0);
    let c_len = sound_len();

    let r_outcome = run_side(Side::Rust, 2, setup, pf::SOUND);
    let r_rec = sound_get(0);
    let r_len = sound_len();

    assert_eq!(c_outcome, r_outcome);
    assert!(!c_outcome.raised);
    assert_eq!(c_len, 1);
    assert_eq!(r_len, 1);
    assert_eq!(c_rec, r_rec);
    let rec = c_rec.expect("SV_StartSound must have been called");
    assert_eq!(rec.ent, 1);
    assert_eq!(rec.channel, 3);
    assert_eq!(rec.volume, 127); // 0.5 * 255 truncated
    assert!(!rec.has_origin);
    assert_eq!(rec.sample, "weapons/tst1.wav");
}

#[test]
fn sound_empty_sample_warns_and_does_not_start_sound() {
    let _g = lock();
    let setup = || {
        set_global_i(OFS_PARM0, edict_prog(1));
        set_global_f(OFS_PARM1, 1.0);
        set_global_i(OFS_PARM2, 0); // offset 0 == ""
        set_global_f(OFS_PARM3, 1.0);
        set_global_f(OFS_PARM4, 1.0);
    };

    let c_outcome = run_side(Side::C, 2, setup, pf::SOUND);
    let c_len = sound_len();
    let c_log = con_log();

    let r_outcome = run_side(Side::Rust, 2, setup, pf::SOUND);
    let r_len = sound_len();
    let r_log = con_log();

    assert_eq!(c_outcome, r_outcome);
    assert!(!c_outcome.raised, "empty sample only warns, never raises");
    assert_eq!(c_len, 0);
    assert_eq!(r_len, 0);
    assert_eq!(c_log, r_log);
    assert!(
        c_log.iter().any(|l| l.contains("PF_sound: empty string")),
        "expected PR_RunWarning to be observable in the console log, got {c_log:?}"
    );
}

#[test]
fn sound_raises_when_start_sound_raises_and_matches_oracle() {
    let _g = lock();
    let setup = || {
        sound_arm_raise(true);
        let sample = intern("weapons/tst2.wav");
        set_global_i(OFS_PARM0, edict_prog(1));
        set_global_f(OFS_PARM1, 1.0);
        set_global_i(OFS_PARM2, sample);
        set_global_f(OFS_PARM3, 1.0);
        set_global_f(OFS_PARM4, 1.0);
    };

    let c_outcome = run_side(Side::C, 2, setup, pf::SOUND);
    let r_outcome = run_side(Side::Rust, 2, setup, pf::SOUND);

    assert!(c_outcome.raised);
    assert_eq!(c_outcome, r_outcome);
    assert!(c_outcome.message.contains("not precached"));
}

// ===========================================================================
// PF_sv_precache_sound
// ===========================================================================

#[test]
fn precache_sound_success_returns_handle_and_records_sample() {
    let _g = lock();
    let setup = || {
        let s = intern("ambience/hum1.wav");
        set_global_i(OFS_PARM0, s);
    };

    let c_outcome = run_side(Side::C, 1, setup, pf::PRECACHE_SOUND);
    let c_ret = get_ofs_return_i();
    let c_last = predd_last_sound();

    let r_outcome = run_side(Side::Rust, 1, setup, pf::PRECACHE_SOUND);
    let r_ret = get_ofs_return_i();
    let r_last = predd_last_sound();

    assert_eq!(c_outcome, r_outcome);
    assert!(!c_outcome.raised);
    assert_eq!(c_ret, r_ret);
    assert_eq!(c_last, r_last);
    assert_eq!(c_last, "ambience/hum1.wav");
}

#[test]
fn precache_sound_empty_string_raises_bad_string() {
    let _g = lock();
    // `intern("")` allocates a NONZERO offset into the empty string pool
    // (ctest_fx_strings_reset reserves offset 0 separately as PR_GetString(0)
    // == ""; ctest_fx_intern always bumps ctest_fx_strings_len before
    // returning) -- PROVIDED ctest_fx_strings_reset has run at least once in
    // this process; its backing counter is a C static that only becomes 1
    // once ctest_fx_reset has executed, so interning before any reset() could
    // return 0 if this happens to be the first test the (unordered) runner
    // picks. Force a reset first so the offset is deterministic.
    //
    // Deliberately not offset 0 literal: OFS_RETURN's fixture default is also
    // 0, so a handle of 0 here cannot distinguish "wrote the handle
    // unconditionally" from "never wrote OFS_RETURN at all" -- a mutation
    // moving the OFS_RETURN write to after the guarded call (undoing the
    // COMPAT fix documented in progs_builtins_sv_fx.rs) survived with the old
    // handle==0 setup and was only caught once this used a nonzero handle.
    reset(1);
    let empty_handle = intern("");
    assert_ne!(
        empty_handle, 0,
        "test bug: need a nonzero empty-string handle"
    );
    let setup = move || {
        set_global_i(OFS_PARM0, empty_handle);
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
    // behavior, not a stub artifact: an earlier draft of this test expected
    // the formatted text on the Host_Error channel and failed identically on
    // both sides, which is what caught the wrong assumption.
    assert_eq!(c_outcome.message, "Program error");
    assert_eq!(c_log, r_log);
    assert!(
        c_log.iter().any(|l| l.contains("Bad string")),
        "expected PR_RunError's formatted message on the console, got {c_log:?}"
    );
    // pr_cmds.c:1193 writes OFS_RETURN unconditionally, before the check --
    // must equal the (nonzero) pass-through handle, not merely be present.
    assert_eq!(c_ret, empty_handle);
    assert_eq!(r_ret, empty_handle);
}

// ===========================================================================
// PF_sv_precache_model
// ===========================================================================

#[test]
fn precache_model_first_insert_fills_slot0_no_warn_when_loading() {
    let _g = lock();
    let setup = || {
        set_ss_loading(true);
        let m = intern("progs/soldier.mdl");
        set_global_i(OFS_PARM0, m);
    };

    let c_outcome = run_side(Side::C, 1, setup, pf::PRECACHE_MODEL);
    let c_slot0 = model_slot_used(0);
    let c_ret = get_ofs_return_i();
    let c_log = con_log();

    let r_outcome = run_side(Side::Rust, 1, setup, pf::PRECACHE_MODEL);
    let r_slot0 = model_slot_used(0);
    let r_ret = get_ofs_return_i();
    let r_log = con_log();

    assert_eq!(c_outcome, r_outcome);
    assert!(!c_outcome.raised);
    assert!(c_slot0);
    assert_eq!(c_slot0, r_slot0);
    assert_eq!(c_ret, r_ret);
    assert!(
        c_log.is_empty(),
        "ss_loading==true must not warn, got {c_log:?}"
    );
    assert_eq!(c_log, r_log);
}

#[test]
fn precache_model_first_insert_warns_when_not_loading() {
    let _g = lock();
    let setup = || {
        set_ss_loading(false);
        let m = intern("progs/soldier.mdl");
        set_global_i(OFS_PARM0, m);
    };

    let c_outcome = run_side(Side::C, 1, setup, pf::PRECACHE_MODEL);
    let c_log = con_log();

    let r_outcome = run_side(Side::Rust, 1, setup, pf::PRECACHE_MODEL);
    let r_log = con_log();

    assert_eq!(c_outcome, r_outcome);
    assert!(!c_outcome.raised);
    assert_eq!(c_log, r_log);
    assert_eq!(
        c_log.len(),
        1,
        "expected exactly one warning, got {c_log:?}"
    );
    assert!(c_log[0].contains("Precache should only be done in spawn functions"));
}

#[test]
fn precache_model_rematch_same_name_no_warn_with_checkextension() {
    let _g = lock();
    // pr_checkextension is a plain global cvar_t; ctest_fx_reset /
    // ctest_world_reset never touch it (only ctest_world_set_cvars does), so
    // it must be armed explicitly here for a rematch to skip the warning
    // even with ss_loading == false (pr_cmds.c:1251-1253's
    // `!pr_checkextension.value` gate). An earlier draft assumed
    // ctest_world_reset armed it -- it does not, and the omission made this
    // test fail against BOTH sides identically (not a C/Rust divergence).
    //
    // The first-insert branch (pr_cmds.c:1237-1245) warns on
    // `sv.state != ss_loading` alone -- it does NOT consult
    // pr_checkextension, only the rematch branch does. So the discard-run
    // below must use ss_loading == true, or it would leave its own warning
    // in the log before the measured rematch call even runs (a second real
    // bug this test's first draft had, caught the same way: it failed
    // identically on both sides).
    let setup = || {
        set_checkextension(true);
        set_ss_loading(true);
        let m = intern("progs/soldier.mdl");
        set_global_i(OFS_PARM0, m);
        Side::C.run(pf::PRECACHE_MODEL); // first insert, discard its own log
        set_ss_loading(false);
    };
    // The discard-run above executed on whichever fixture is live when
    // `setup` runs, i.e. once per side inside `run_side`, so both sides see
    // an identical two-call sequence.

    let c_outcome = run_side(Side::C, 1, setup, pf::PRECACHE_MODEL);
    let c_log = con_log();
    let c_slot1 = model_slot_used(1);

    let r_outcome = run_side(Side::Rust, 1, setup, pf::PRECACHE_MODEL);
    let r_log = con_log();
    let r_slot1 = model_slot_used(1);

    assert_eq!(c_outcome, r_outcome);
    assert!(!c_outcome.raised);
    assert_eq!(c_log, r_log);
    assert!(
        c_log
            .iter()
            .all(|l| !l.contains("Precache should only be done")),
        "a rematch under pr_checkextension must not warn, got {c_log:?}"
    );
    assert!(!c_slot1, "a rematch must not consume a second slot");
    assert_eq!(c_slot1, r_slot1);
}

#[test]
fn precache_model_bad_string_raises() {
    let _g = lock();
    // Nonzero empty-string handle, not offset 0 literal -- see
    // precache_sound_empty_string_raises_bad_string's comment: OFS_RETURN's
    // fixture default is also 0, so handle==0 cannot distinguish "wrote the
    // handle unconditionally before the raise" from "never wrote it".
    reset(1);
    let empty_handle = intern("");
    assert_ne!(
        empty_handle, 0,
        "test bug: need a nonzero empty-string handle"
    );
    let setup = move || {
        set_ss_loading(true);
        set_global_i(OFS_PARM0, empty_handle);
    };

    let c_outcome = run_side(Side::C, 1, setup, pf::PRECACHE_MODEL);
    let c_ret = get_ofs_return_i();
    let c_log = con_log();

    let r_outcome = run_side(Side::Rust, 1, setup, pf::PRECACHE_MODEL);
    let r_ret = get_ofs_return_i();
    let r_log = con_log();

    assert!(c_outcome.raised);
    assert_eq!(c_outcome, r_outcome);
    // See precache_sound_empty_string_raises_bad_string: PR_RunError always
    // hands Host_Error the literal "Program error"; the formatted text only
    // reaches Con_Printf.
    assert_eq!(c_outcome.message, "Program error");
    assert_eq!(c_log, r_log);
    assert!(
        c_log.iter().any(|l| l.contains("Bad string")),
        "expected PR_RunError's formatted message on the console, got {c_log:?}"
    );
    // pr_cmds.c:1231 writes OFS_RETURN unconditionally, before the check.
    assert_eq!(c_ret, empty_handle);
    assert_eq!(r_ret, empty_handle);
}

#[test]
fn precache_model_overflow_raises_after_filling_all_slots() {
    let _g = lock();
    // CTEST_FX_MODEL_SLOTS == 8 (pf_fx_ref.c); fill all 8 with distinct
    // names, then a 9th distinct name must overflow-raise on both sides.
    // `overflow`'s handle is already guaranteed nonzero (interned after 8
    // other strings), so it also exercises the OFS_RETURN check below without
    // the offset-0 degeneracy noted on the empty-string tests.
    let overflow = std::cell::Cell::new(0);
    let setup = || {
        set_ss_loading(true);
        for i in 0..8 {
            let name = format!("progs/mock{i}.mdl");
            let h = intern(&name);
            set_global_i(OFS_PARM0, h);
            let o = Side::C.run(pf::PRECACHE_MODEL);
            assert!(!o.raised, "filling slot {i} must not raise, got {o:?}");
        }
        let h = intern("progs/mock_overflow.mdl");
        overflow.set(h);
        set_global_i(OFS_PARM0, h);
    };

    let c_outcome = run_side(Side::C, 1, setup, pf::PRECACHE_MODEL);
    let c_ret = get_ofs_return_i();
    let c_overflow_handle = overflow.get();
    let c_log = con_log();

    let r_outcome = run_side(Side::Rust, 1, setup, pf::PRECACHE_MODEL);
    let r_ret = get_ofs_return_i();
    let r_overflow_handle = overflow.get();
    let r_log = con_log();

    assert!(c_outcome.raised);
    assert_eq!(c_outcome, r_outcome);
    // See precache_sound_empty_string_raises_bad_string: PR_RunError always
    // hands Host_Error the literal "Program error"; the formatted text only
    // reaches Con_Printf.
    assert_eq!(c_outcome.message, "Program error");
    assert_eq!(c_log, r_log);
    assert!(
        c_log
            .iter()
            .any(|l| l.contains("PF_precache_model: overflow")),
        "expected PR_RunError's formatted message on the console, got {c_log:?}"
    );
    // pr_cmds.c:1231 writes OFS_RETURN unconditionally, before the scan that
    // raises overflow. Both sides intern the same string in the same order
    // (setup is side-independent per run_side's contract), so the two
    // recorded handles must agree too.
    assert_eq!(c_overflow_handle, r_overflow_handle);
    assert_ne!(
        c_overflow_handle, 0,
        "test bug: need a nonzero overflow handle"
    );
    assert_eq!(c_ret, c_overflow_handle);
    assert_eq!(r_ret, r_overflow_handle);
}

// ===========================================================================
// PF_sv_finalefinished / PF_sv_CheckPlayerEXFlags
// ===========================================================================

#[test]
fn finalefinished_sets_return_zero() {
    let _g = lock();
    let setup = || {
        set_global_f(OFS_RETURN, 42.0); // poison, must be overwritten
    };

    let c_outcome = run_side(Side::C, 1, setup, pf::FINALEFINISHED);
    let c_ret = get_ofs_return_f();

    let r_outcome = run_side(Side::Rust, 1, setup, pf::FINALEFINISHED);
    let r_ret = get_ofs_return_f();

    assert_eq!(c_outcome, r_outcome);
    assert!(!c_outcome.raised);
    assert_eq!(c_ret, 0.0);
    assert_eq!(r_ret, 0.0);
}

#[test]
fn check_player_ex_flags_sets_return_zero() {
    let _g = lock();
    let setup = || {
        set_global_f(OFS_RETURN, 42.0); // poison, must be overwritten
    };

    let c_outcome = run_side(Side::C, 1, setup, pf::CHECK_PLAYER_EX_FLAGS);
    let c_ret = get_ofs_return_f();

    let r_outcome = run_side(Side::Rust, 1, setup, pf::CHECK_PLAYER_EX_FLAGS);
    let r_ret = get_ofs_return_f();

    assert_eq!(c_outcome, r_outcome);
    assert!(!c_outcome.raised);
    assert_eq!(c_ret, 0.0);
    assert_eq!(r_ret, 0.0);
}

// ===========================================================================
// PF_sv_changelevel
// ===========================================================================

#[test]
fn changelevel_first_call_issues_and_records_command() {
    let _g = lock();
    let setup = || {
        changelevel_set_issued(false);
        let level = intern("e1m2");
        set_global_i(OFS_PARM0, level);
    };

    let c_outcome = run_side(Side::C, 1, setup, pf::CHANGELEVEL);
    let c_issued = changelevel_get_issued();
    let c_calls = changelevel_calls();
    let c_cmd = changelevel_last();

    let r_outcome = run_side(Side::Rust, 1, setup, pf::CHANGELEVEL);
    let r_issued = changelevel_get_issued();
    let r_calls = changelevel_calls();
    let r_cmd = changelevel_last();

    assert_eq!(c_outcome, r_outcome);
    assert!(!c_outcome.raised);
    assert!(c_issued, "changelevel must set svs.changelevel_issued");
    assert_eq!(c_issued, r_issued);
    assert_eq!(c_calls, 1);
    assert_eq!(r_calls, 1);
    assert_eq!(c_cmd, r_cmd);
    assert_eq!(c_cmd, "changelevel e1m2\n");
}

#[test]
fn changelevel_second_call_is_noop_when_already_issued() {
    let _g = lock();
    let setup = || {
        changelevel_set_issued(true);
        let level = intern("e1m3");
        set_global_i(OFS_PARM0, level);
    };

    let c_outcome = run_side(Side::C, 1, setup, pf::CHANGELEVEL);
    let c_calls = changelevel_calls();
    let c_issued = changelevel_get_issued();

    let r_outcome = run_side(Side::Rust, 1, setup, pf::CHANGELEVEL);
    let r_calls = changelevel_calls();
    let r_issued = changelevel_get_issued();

    assert_eq!(c_outcome, r_outcome);
    assert!(!c_outcome.raised);
    assert_eq!(
        c_calls, 0,
        "an already-issued changelevel must not re-issue"
    );
    assert_eq!(c_calls, r_calls);
    assert!(c_issued, "the flag must stay set");
    assert_eq!(c_issued, r_issued);
}
