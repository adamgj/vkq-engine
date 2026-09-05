//! Differential test: the Rust `quake-capi` particle QuakeC builtins
//! (`rust/quake-capi/src/progs_builtins_particles.rs`) vs the original
//! `Quake/pr_ext.c` bodies (`Quake/pr_ext.c:4720-4944`). Rust migration
//! Phase 7, M9f group E.
//!
//! `pr_ext.c` is not in `build.rs`'s `C_SOURCES`; the six `PF_*` bodies are
//! `static` and are reached through `#include "pr_ext.c"` in
//! `stubs/pr_ext_ref.c`, called directly by `ctest_m9fe_run` (dispatch
//! indices 100-106). `sv`/`svs`/`cl` are force-renamed to
//! `c_ref_sv`/`c_ref_svs`/`c_ref_cl` for that whole TU (`c_ref_prelude.h`),
//! so the C oracle writes storage genuinely independent of the plain-named
//! `sv`/`svs` (`quake-capi/src/sv_main.rs`, T6.6) and `cl`
//! (`quake-capi/src/cl_main.rs`, T7.4) the `quake_rs_pf_*` entry points
//! write. Both are read back through the `ctest_m9fe_*` accessors, with
//! `side` 0 = C oracle and 1 = Rust.
//!
//! # Raise topology (ADR-009)
//!
//! `PF_sv_particleeffectnum` (`pr_ext.c:4769`) and `PF_cl_particleeffectnum`
//! (`pr_ext.c:4903`) both end in `PR_RunError`. The two sides report that
//! differently *by construction*, so the raw status codes are deliberately
//! not compared:
//!
//! * The C side runs inside `Host_Guard` (`ctest_m9fe_run`), so it reports
//!   `CTEST_GUARD_HOST_ERROR`. `PR_RunError` is real compiled C here
//!   (`Quake/pr_exec.c:191-207` is in `C_SOURCES`): it prints the *real*
//!   message with `Con_Printf` and only then calls `Host_Error ("Program
//!   error")`. So `ctest_host_error_message()` is `"Program error"` and the
//!   overflow text is observable only on the console channel.
//! * The Rust side is driven through `quake_rs_pf_*` directly, never through
//!   the `rust_pf_*` C wrapper in `pr_cmds_glue.c`, so `PRBI_Raise` never
//!   fires: it returns `PRBI_ERR_SV_PARTICLEEFFECTNUM_OVERFLOW` /
//!   `..._CL_...` and emits neither console line nor `Host_Error`.
//!
//! What is compared on a raise is therefore: a `raised` boolean, the exact
//! `PRBI_ERR_*` code on the Rust side, the exact `Host_Error`/console text on
//! the C side, and all observable state. This follows
//! `pr_ext_strext_differential.rs`'s convention.
//!
//! # Console comparison is `[warn]`-filtered
//!
//! `pr_ext_warned_particleeffectnum` (`pr_ext.c:52`) exists so the
//! "Precache should only be done in spawn functions" warning only spams three
//! times per map, which makes the console a *behaviour* here, not noise --
//! `warn_budget_is_three_then_silent` asserts it. Only `[warn]` lines are
//! compared: the C side additionally prints the raise text through
//! `Con_Printf` (see above), which the Rust side has no counterpart for.
//!
//! # Shared instruments (not part of what is compared)
//!
//! `r_particledesc` (`stubs/pr_ext_ref.c:223`), `host_frametime`
//! (`stubs.c:2718`) and the `PScript_ParticleTrail` /
//! `PScript_RunParticleEffectState` recorders (`stubs.c:7538`/`:7551`) are
//! not prelude-renamed, so both sides read and drive the identical storage.
//! They are inputs and instruments. `PScript_FindParticleType`
//! (`stubs.c:7517`) is a `Sys_Error` abort double, so every `cl` scenario
//! below keeps off the `PF_CL_ForceParticlePrecache` allocating branch --
//! `fill_cl_precache` fills the *local* table for exactly that reason.

use core::ffi::{c_char, c_float, c_int, c_uint};
use std::ffi::CStr;
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

// ---------------------------------------------------------------------------
// progs.h offsets (OFS_PARM stride is 3 floats/12 bytes per slot).

const OFS_RETURN: c_int = 1;
const OFS_PARM0: c_int = 4;
const OFS_PARM1: c_int = 7;
const OFS_PARM2: c_int = 10;
const OFS_PARM3: c_int = 13;

/// `protocol.h`'s `svcdp_precache` / `svcdp_trailparticles` /
/// `svcdp_pointparticles` / `svcdp_pointparticles1`.
const SVCDP_PRECACHE: u8 = 54;
/// `protocol.h`'s `PEXT2_REPLACEMENTDELTAS`.
const PEXT2_REPLACEMENTDELTAS: c_uint = 0x0000_0008;

/// `server.h`'s `server_state_t`.
const SS_LOADING: c_int = 0;
const SS_ACTIVE: c_int = 1;

/// `pr_cmds_glue.c` `PRBI_OK`.
const PRBI_OK: c_int = 0;
/// `pr_cmds_glue.c` `PRBI_ERR_SV_PARTICLEEFFECTNUM_OVERFLOW`.
const PRBI_ERR_SV_PARTICLEEFFECTNUM_OVERFLOW: c_int = 9;
/// `pr_cmds_glue.c` `PRBI_ERR_CL_PARTICLEEFFECTNUM_OVERFLOW`.
const PRBI_ERR_CL_PARTICLEEFFECTNUM_OVERFLOW: c_int = 10;
/// `CTEST_GUARD_HOST_ERROR` (`stubs.c`).
const CTEST_GUARD_HOST_ERROR: c_int = 1;

/// `stubs/pr_ext_ref.c`'s `ctest_m9fe_dispatch` switch indices.
mod pf {
    pub const SV_PARTICLEEFFECTNUM: i32 = 100;
    pub const SV_TRAILPARTICLES: i32 = 101;
    pub const SV_POINTPARTICLES: i32 = 102;
    pub const CL_PARTICLEEFFECTNUM: i32 = 103;
    pub const CL_TRAILPARTICLES: i32 = 104;
    pub const CL_POINTPARTICLES: i32 = 105;
    pub const RESET_WARN_COUNT: i32 = 106;
}

extern "C" {
    // --- shared fixture (stubs/pr_ext_ref.c, base block) -------------------
    fn ctest_pr_ext_intern(s: *const c_char) -> c_int;
    fn ctest_pr_ext_set_argc(argc: c_int);
    fn ctest_pr_ext_set_global_int(ofs: c_int, v: c_int);
    fn ctest_pr_ext_set_global_float(ofs: c_int, v: c_float);
    fn ctest_pr_ext_get_global_float(ofs: c_int) -> c_float;

    // --- group D (reused: the edict-offset helper) ------------------------
    fn ctest_pr_ext_te_edict_prog(num: c_int) -> c_int;

    // --- fixture, M9F GROUP E block ---------------------------------------
    fn ctest_m9fe_reset(
        num_edicts: c_int,
        maxclients: c_int,
        pext2: c_uint,
        sv_state: c_int,
        checkextension: c_float,
    );
    fn ctest_m9fe_set_particledesc(s: *const c_char);
    fn ctest_m9fe_set_host_frametime(v: f64);
    fn ctest_m9fe_set_sv_precache(idx: c_int, name: *const c_char);
    fn ctest_m9fe_fill_sv_precache();
    fn ctest_m9fe_set_cl_precache(idx: c_int, name: *const c_char, index: c_int);
    fn ctest_m9fe_set_cl_local_precache(idx: c_int, name: *const c_char, index: c_int);
    fn ctest_m9fe_fill_cl_precache();
    fn ctest_m9fe_sv_precache(side: c_int, idx: c_int) -> *const c_char;
    fn ctest_m9fe_cl_local_precache(side: c_int, idx: c_int) -> *const c_char;
    fn ctest_m9fe_datagram_len(side: c_int) -> c_int;
    fn ctest_m9fe_datagram_byte(side: c_int, i: c_int) -> c_int;
    fn ctest_m9fe_multicast_len(side: c_int) -> c_int;
    fn ctest_m9fe_multicast_byte(side: c_int, i: c_int) -> c_int;
    fn ctest_m9fe_client_message_len(side: c_int, idx0based: c_int) -> c_int;
    fn ctest_m9fe_client_message_byte(side: c_int, idx0based: c_int, i: c_int) -> c_int;
    fn ctest_m9fe_client_datagram_len(side: c_int, idx0based: c_int) -> c_int;
    fn ctest_m9fe_client_datagram_byte(side: c_int, idx0based: c_int, i: c_int) -> c_int;
    fn ctest_m9fe_warn_count() -> c_int;

    // --- oracle dispatcher (stubs/pr_ext_ref.c, GROUP E block) ------------
    fn ctest_m9fe_run(which: c_int) -> c_int;

    // --- console capture + Host_Error channel (stubs.c) -------------------
    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(i: c_int) -> *const c_char;
    fn ctest_host_error_message() -> *const c_char;

    // --- PScript recorders (stubs.c; shared by both sides) ----------------
    // (`ctest_pscript_reset` is called from `ctest_m9fe_reset`, not from here.)
    fn ctest_pscript_trail_count() -> c_int;
    fn ctest_pscript_last_timeinterval_value() -> c_float;
    fn ctest_pscript_state_count() -> c_int;
    fn ctest_pscript_last_count_value() -> c_float;

    // --- the Rust port under test (progs_builtins_particles.rs) -----------
    fn quake_rs_pf_sv_particleeffectnum(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_trailparticles(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_pointparticles(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_cl_particleeffectnum(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_cl_trailparticles(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_cl_pointparticles(detail: *mut c_int) -> c_int;
    fn quake_rs_pr_reset_particle_warn_count();
}

// ---------------------------------------------------------------------------
// Safe wrappers.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    C,
    Rust,
}

impl Side {
    fn idx(self) -> c_int {
        match self {
            Side::C => 0,
            Side::Rust => 1,
        }
    }
}

/// Full per-side reset. Also zeroes the *Rust* warn counter, which
/// `ctest_m9fe_reset` cannot reach (it is a Rust-side `static mut`, separate
/// from the composed TU's `pr_ext_warned_particleeffectnum`); without this the
/// second half of every comparison would start with a spent warning budget.
fn reset(num_edicts: c_int, maxclients: c_int, pext2: c_uint, sv_state: c_int, checkext: f32) {
    // SAFETY: plain scalar arguments; the fixture allocates and zeroes its own
    // state on both sides. Serialised by `TEST_LOCK`.
    unsafe {
        ctest_m9fe_reset(num_edicts, maxclients, pext2, sv_state, checkext);
        quake_rs_pr_reset_particle_warn_count();
    }
}

fn intern(s: &str) -> c_int {
    let c = std::ffi::CString::new(s).expect("no interior NUL");
    // SAFETY: `c` is NUL-terminated and outlives the call; the fixture copies
    // it into its own pool.
    unsafe { ctest_pr_ext_intern(c.as_ptr()) }
}

fn set_argc(argc: c_int) {
    // SAFETY: a plain int the fixture range-checks against its globals block.
    unsafe { ctest_pr_ext_set_argc(argc) }
}

fn set_global_i(ofs: c_int, v: c_int) {
    // SAFETY: `ofs` is a reserved OFS_* offset inside the globals block.
    unsafe { ctest_pr_ext_set_global_int(ofs, v) }
}

fn set_global_f(ofs: c_int, v: f32) {
    // SAFETY: as `set_global_i`.
    unsafe { ctest_pr_ext_set_global_float(ofs, v) }
}

fn get_global_f(ofs: c_int) -> f32 {
    // SAFETY: as `set_global_i`.
    unsafe { ctest_pr_ext_get_global_float(ofs) }
}

fn set_vector(ofs: c_int, v: [f32; 3]) {
    set_global_f(ofs, v[0]);
    set_global_f(ofs + 1, v[1]);
    set_global_f(ofs + 2, v[2]);
}

fn edict_prog(num: c_int) -> c_int {
    // SAFETY: `num` indexes the fixture arena reset above.
    unsafe { ctest_pr_ext_te_edict_prog(num) }
}

/// `c"..."` literals only: these pointers are stored *into* the precache
/// tables and read back after the builtin runs, so they must be `'static`.
fn set_sv_precache(idx: c_int, name: &'static CStr) {
    // SAFETY: `name` is a `'static` NUL-terminated literal; `idx` is in
    // `1..MAX_PARTICLETYPES` by test construction.
    unsafe { ctest_m9fe_set_sv_precache(idx, name.as_ptr()) }
}

fn set_cl_precache(idx: c_int, name: &'static CStr, index: c_int) {
    // SAFETY: as `set_sv_precache`.
    unsafe { ctest_m9fe_set_cl_precache(idx, name.as_ptr(), index) }
}

fn set_cl_local_precache(idx: c_int, name: &'static CStr, index: c_int) {
    // SAFETY: as `set_sv_precache`.
    unsafe { ctest_m9fe_set_cl_local_precache(idx, name.as_ptr(), index) }
}

fn read_c_str(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: every producer here is either a `'static` literal the test
    // stored or a `q_strdup` allocation the builtin made; both are
    // NUL-terminated and outlive this copy.
    Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

fn sv_precache(side: Side, idx: c_int) -> Option<String> {
    // SAFETY: `side`/`idx` are in range by test construction.
    read_c_str(unsafe { ctest_m9fe_sv_precache(side.idx(), idx) })
}

fn cl_local_precache(side: Side, idx: c_int) -> Option<String> {
    // SAFETY: as `sv_precache`.
    read_c_str(unsafe { ctest_m9fe_cl_local_precache(side.idx(), idx) })
}

fn datagram_bytes(side: Side) -> Vec<u8> {
    // SAFETY: the length accessor bounds the index.
    unsafe {
        let n = ctest_m9fe_datagram_len(side.idx());
        (0..n)
            .map(|i| ctest_m9fe_datagram_byte(side.idx(), i) as u8)
            .collect()
    }
}

fn multicast_bytes(side: Side) -> Vec<u8> {
    // SAFETY: as `datagram_bytes`.
    unsafe {
        let n = ctest_m9fe_multicast_len(side.idx());
        (0..n)
            .map(|i| ctest_m9fe_multicast_byte(side.idx(), i) as u8)
            .collect()
    }
}

fn client_message_bytes(side: Side, idx0based: c_int) -> Vec<u8> {
    // SAFETY: `idx0based` names a live fixture client
    // (`CTEST_M9FE_CLIENTS == 2`).
    unsafe {
        let n = ctest_m9fe_client_message_len(side.idx(), idx0based);
        (0..n)
            .map(|i| ctest_m9fe_client_message_byte(side.idx(), idx0based, i) as u8)
            .collect()
    }
}

/// The unreliable half of the fan-out. `MULTICAST_PHS_U`/`PVS_U` with a
/// non-zero `requireext2` write here, not into `.message`.
fn client_datagram_bytes(side: Side, idx0based: c_int) -> Vec<u8> {
    // SAFETY: as `client_message_bytes`.
    unsafe {
        let n = ctest_m9fe_client_datagram_len(side.idx(), idx0based);
        (0..n)
            .map(|i| ctest_m9fe_client_datagram_byte(side.idx(), idx0based, i) as u8)
            .collect()
    }
}

fn host_error_message() -> String {
    // SAFETY: the stub returns a NUL-terminated buffer that outlives this.
    unsafe { CStr::from_ptr(ctest_host_error_message()) }
        .to_string_lossy()
        .into_owned()
}

/// Every console line, in order.
fn console_all() -> Vec<String> {
    // SAFETY: `ctest_con_log_len` bounds the index and each entry is a
    // NUL-terminated buffer owned by the log until the next reset.
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

/// The `Con_Warning` lines only -- see the module doc's console section.
fn console_warnings() -> Vec<String> {
    console_all()
        .into_iter()
        .filter(|l| l.starts_with("[warn]"))
        .collect()
}

/// The shared `PScript_*` recorder state
/// (trail calls, last timeinterval, state calls, last count).
fn pscript() -> (c_int, f32, c_int, f32) {
    // SAFETY: plain reads of `stubs.c` file-scope statics.
    unsafe {
        (
            ctest_pscript_trail_count(),
            ctest_pscript_last_timeinterval_value(),
            ctest_pscript_state_count(),
            ctest_pscript_last_count_value(),
        )
    }
}

/// Runs one builtin on one side and returns its status.
fn invoke(side: Side, which: i32) -> c_int {
    match side {
        // SAFETY: `which` is one of the GROUP E dispatcher indices; the C body
        // runs inside `Host_Guard`, so a `Host_Error` unwinds in a C frame and
        // never longjmps past this call (ADR-009).
        Side::C => unsafe { ctest_m9fe_run(which) },
        Side::Rust => {
            let mut detail: c_int = 0;
            // SAFETY: `detail` is a live, initialised `c_int`; these entry
            // points are status-returning and read the ambient `sv`/`svs`/`cl`
            // and qcvm ports the fixture has just reset (ADR-008).
            unsafe {
                match which {
                    pf::SV_PARTICLEEFFECTNUM => quake_rs_pf_sv_particleeffectnum(&mut detail),
                    pf::SV_TRAILPARTICLES => quake_rs_pf_sv_trailparticles(&mut detail),
                    pf::SV_POINTPARTICLES => quake_rs_pf_sv_pointparticles(&mut detail),
                    pf::CL_PARTICLEEFFECTNUM => quake_rs_pf_cl_particleeffectnum(&mut detail),
                    pf::CL_TRAILPARTICLES => quake_rs_pf_cl_trailparticles(&mut detail),
                    pf::CL_POINTPARTICLES => quake_rs_pf_cl_pointparticles(&mut detail),
                    pf::RESET_WARN_COUNT => {
                        quake_rs_pr_reset_particle_warn_count();
                        PRBI_OK
                    }
                    other => panic!("no Rust entry point for dispatch index {other}"),
                }
            }
        }
    }
}

/// Everything a non-raising scenario can observe.
#[derive(Debug, PartialEq)]
struct Obs {
    status: c_int,
    ret: f32,
    datagram: Vec<u8>,
    multicast: Vec<u8>,
    client0: Vec<u8>,
    client1: Vec<u8>,
    client0_dg: Vec<u8>,
    client1_dg: Vec<u8>,
    warnings: Vec<String>,
    pscript: (c_int, u32, c_int, u32),
}

fn observe(side: Side, status: c_int) -> Obs {
    let (trails, interval, states, count) = pscript();
    Obs {
        status,
        ret: get_global_f(OFS_RETURN),
        datagram: datagram_bytes(side),
        multicast: multicast_bytes(side),
        client0: client_message_bytes(side, 0),
        client1: client_message_bytes(side, 1),
        client0_dg: client_datagram_bytes(side, 0),
        client1_dg: client_datagram_bytes(side, 1),
        warnings: console_warnings(),
        // Bit patterns, so a NaN would compare rather than silently pass.
        pscript: (trails, interval.to_bits(), states, count.to_bits()),
    }
}

/// Runs `setup` then `which` on both sides in turn and returns
/// `(c_observation, rust_observation)`. `setup` re-runs per side because the
/// fixture reset in between clears everything it did.
fn run_both(
    which: i32,
    num_edicts: c_int,
    maxclients: c_int,
    pext2: c_uint,
    sv_state: c_int,
    checkext: f32,
    setup: impl Fn(),
) -> (Obs, Obs) {
    let ((c, ()), (rust, ())) = run_both_probed(
        which,
        num_edicts,
        maxclients,
        pext2,
        sv_state,
        checkext,
        setup,
        |_| (),
    );
    (c, rust)
}

/// `run_both` plus a per-side `probe`, run immediately after that side's
/// invocation. Anything read from the fixture AFTER the loop is worthless:
/// the next iteration's reset clears both storages, so a C-side readback taken
/// at the end sees only what the Rust iteration's `setup` happened to seed.
#[allow(clippy::too_many_arguments)]
fn run_both_probed<T>(
    which: i32,
    num_edicts: c_int,
    maxclients: c_int,
    pext2: c_uint,
    sv_state: c_int,
    checkext: f32,
    setup: impl Fn(),
    probe: impl Fn(Side) -> T,
) -> ((Obs, T), (Obs, T)) {
    let mut out = Vec::new();
    for side in [Side::C, Side::Rust] {
        reset(num_edicts, maxclients, pext2, sv_state, checkext);
        setup();
        let status = invoke(side, which);
        out.push((observe(side, status), probe(side)));
    }
    let rust = out.pop().unwrap();
    let c = out.pop().unwrap();
    (c, rust)
}

// ---------------------------------------------------------------------------
// PF_sv_particleeffectnum

#[test]
fn sv_particleeffectnum_empty_string_returns_zero() {
    let _g = lock();
    let (c, rust) = run_both(
        pf::SV_PARTICLEEFFECTNUM,
        4,
        0,
        PEXT2_REPLACEMENTDELTAS,
        SS_ACTIVE,
        0.0,
        || {
            set_global_i(OFS_PARM0, intern(""));
            set_argc(1);
        },
    );
    assert_eq!(c.status, PRBI_OK);
    assert_eq!(rust.status, PRBI_OK);
    assert_eq!(c.ret, 0.0);
    assert_eq!(c, rust);
}

#[test]
fn sv_particleeffectnum_finds_existing_while_loading() {
    let _g = lock();
    let (c, rust) = run_both(
        pf::SV_PARTICLEEFFECTNUM,
        4,
        0,
        PEXT2_REPLACEMENTDELTAS,
        SS_LOADING,
        0.0,
        || {
            set_sv_precache(1, c"other");
            set_sv_precache(2, c"blood");
            set_global_i(OFS_PARM0, intern("blood"));
            set_argc(1);
        },
    );
    assert_eq!(c.status, PRBI_OK);
    assert_eq!(rust.status, PRBI_OK);
    assert_eq!(c.ret, 2.0, "the matching slot index is returned");
    // ss_loading suppresses the warning even with pr_checkextension 0.
    assert!(c.warnings.is_empty());
    assert_eq!(c, rust);
}

#[test]
fn sv_particleeffectnum_existing_match_warns_when_not_loading() {
    let _g = lock();
    let (c, rust) = run_both(
        pf::SV_PARTICLEEFFECTNUM,
        4,
        0,
        PEXT2_REPLACEMENTDELTAS,
        SS_ACTIVE,
        0.0,
        || {
            set_sv_precache(1, c"other");
            set_sv_precache(2, c"blood");
            set_global_i(OFS_PARM0, intern("blood"));
            set_argc(1);
        },
    );
    assert_eq!(c.ret, 2.0);
    assert_eq!(
        c.warnings,
        vec![
            "[warn] PF_sv_particleeffectnum(blood): Precache should only be done in spawn \
             functions\n"
                .to_string()
        ],
        "the C oracle's exact warning text"
    );
    assert_eq!(c, rust);
}

#[test]
fn sv_particleeffectnum_existing_match_is_silent_with_pr_checkextension() {
    let _g = lock();
    let (c, rust) = run_both(
        pf::SV_PARTICLEEFFECTNUM,
        4,
        0,
        PEXT2_REPLACEMENTDELTAS,
        SS_ACTIVE,
        1.0,
        || {
            set_sv_precache(1, c"blood");
            set_global_i(OFS_PARM0, intern("blood"));
            set_argc(1);
        },
    );
    assert_eq!(c.ret, 1.0);
    assert!(c.warnings.is_empty(), "pr_checkextension 1 suppresses it");
    assert_eq!(c, rust);
}

#[test]
fn sv_particleeffectnum_allocates_while_loading_without_broadcasting() {
    let _g = lock();
    let ((c, c_slot2), (rust, rust_slot2)) = run_both_probed(
        pf::SV_PARTICLEEFFECTNUM,
        4,
        1,
        PEXT2_REPLACEMENTDELTAS,
        SS_LOADING,
        0.0,
        || {
            // Slot 1 taken and non-matching: keeps COM_Effectinfo_Enumerate off
            // the path (`!sv.particle_precache[1] && ...`), so slot 2 is free.
            set_sv_precache(1, c"other");
            set_global_i(OFS_PARM0, intern("blood"));
            set_argc(1);
        },
        |side| sv_precache(side, 2),
    );
    assert_eq!(c.ret, 2.0);
    assert!(c.warnings.is_empty());
    assert!(
        c.multicast.is_empty() && c.client0.is_empty() && c.client0_dg.is_empty(),
        "ss_loading skips the svcdp_precache broadcast entirely"
    );
    assert_eq!(c, rust);
    assert_eq!(c_slot2.as_deref(), Some("blood"));
    assert_eq!(rust_slot2.as_deref(), Some("blood"));
}

#[test]
fn sv_particleeffectnum_allocates_and_broadcasts_reliably() {
    let _g = lock();
    let ((c, c_slot2), (rust, rust_slot2)) = run_both_probed(
        pf::SV_PARTICLEEFFECTNUM,
        4,
        2,
        PEXT2_REPLACEMENTDELTAS,
        SS_ACTIVE,
        0.0,
        || {
            set_sv_precache(1, c"other");
            set_global_i(OFS_PARM0, intern("blood"));
            set_argc(1);
        },
        |side| sv_precache(side, 2),
    );
    assert_eq!(c.ret, 2.0);
    assert_eq!(c.warnings.len(), 1);

    // MULTICAST_ALL_R fans out into svs.clients[i].message, then SZ_Clears
    // sv.multicast -- so the payload is observable only on the clients.
    let expected: Vec<u8> = {
        let mut v = vec![SVCDP_PRECACHE];
        v.extend_from_slice(&(2i16 | 0x4000).to_le_bytes());
        v.extend_from_slice(b"blood\0");
        v
    };
    assert_eq!(c.client0, expected, "C oracle wire bytes");
    assert_eq!(c.client1, expected, "both active clients get it");
    assert!(
        c.multicast.is_empty(),
        "SV_Multicast SZ_Clears sv.multicast"
    );
    assert!(c.datagram.is_empty(), "requireext2 != 0 skips sv.datagram");
    assert_eq!(c, rust);
    assert_eq!(c_slot2.as_deref(), Some("blood"));
    assert_eq!(rust_slot2.as_deref(), Some("blood"));
}

#[test]
fn sv_particleeffectnum_broadcast_skips_clients_without_the_extension() {
    let _g = lock();
    // pext2 == 0 on the clients: PF_multicast_internal's requireext2 filter
    // drops every one of them, so nothing lands anywhere.
    let (c, rust) = run_both(pf::SV_PARTICLEEFFECTNUM, 4, 2, 0, SS_ACTIVE, 0.0, || {
        set_sv_precache(1, c"other");
        set_global_i(OFS_PARM0, intern("blood"));
        set_argc(1);
    });
    assert_eq!(c.ret, 2.0);
    assert!(c.client0.is_empty() && c.client1.is_empty());
    assert!(c.datagram.is_empty());
    assert_eq!(c, rust);
}

#[test]
fn sv_particleeffectnum_overflow_raises() {
    let _g = lock();
    let mut obs = Vec::new();
    for side in [Side::C, Side::Rust] {
        reset(4, 0, PEXT2_REPLACEMENTDELTAS, SS_LOADING, 0.0);
        // Every slot taken and none matching: both loops fall through to
        // PR_RunError. Slot 1 non-NULL also keeps COM_Effectinfo_Enumerate off
        // the path.
        // SAFETY: no arguments; fills the fixture's own tables.
        unsafe { ctest_m9fe_fill_sv_precache() };
        set_global_i(OFS_PARM0, intern("nomatch"));
        set_argc(1);
        let status = invoke(side, pf::SV_PARTICLEEFFECTNUM);
        // Captured per side: the next iteration's fixture reset clears the
        // console log and the Host_Error channel along with everything else.
        obs.push((
            status,
            observe(side, status),
            console_all(),
            host_error_message(),
        ));
    }

    let (c_status, c_obs, c_console, c_host_error) = &obs[0];
    let (rust_status, rust_obs, rust_console, _) = &obs[1];

    // See the module doc: the codes differ by construction, the raise does not.
    assert_eq!(*c_status, CTEST_GUARD_HOST_ERROR, "the C side must raise");
    assert_eq!(*rust_status, PRBI_ERR_SV_PARTICLEEFFECTNUM_OVERFLOW);
    assert_eq!(c_host_error.as_str(), "Program error");

    assert!(
        c_console
            .iter()
            .any(|l| l.contains("PF_sv_particleeffectnum: overflow")),
        "PR_RunError prints the real message on the console channel, got {c_console:?}"
    );
    assert!(
        !rust_console
            .iter()
            .any(|l| l.contains("PF_sv_particleeffectnum: overflow")),
        "the Rust half returns a status; PRBI_Raise is a C frame this test          never enters, so nothing is printed, got {rust_console:?}"
    );

    assert_eq!(c_obs.datagram, rust_obs.datagram);
    assert_eq!(c_obs.multicast, rust_obs.multicast);
    assert_eq!(c_obs.client0, rust_obs.client0);
    assert_eq!(c_obs.warnings, rust_obs.warnings);
    assert_eq!(c_obs.pscript, rust_obs.pscript);
}

#[test]
fn warn_budget_is_three_then_silent_and_reset_restores_it() {
    let _g = lock();
    let mut per_side = Vec::new();
    for side in [Side::C, Side::Rust] {
        reset(4, 0, PEXT2_REPLACEMENTDELTAS, SS_ACTIVE, 0.0);
        set_sv_precache(1, c"blood");

        let mut counts = Vec::new();
        for _ in 0..4 {
            set_global_i(OFS_PARM0, intern("blood"));
            set_argc(1);
            let before = console_warnings().len();
            assert_eq!(invoke(side, pf::SV_PARTICLEEFFECTNUM), PRBI_OK);
            counts.push(console_warnings().len() - before);
        }

        // PR_ShutdownExtensions' reset entry point.
        invoke(side, pf::RESET_WARN_COUNT);

        set_global_i(OFS_PARM0, intern("blood"));
        set_argc(1);
        let before = console_warnings().len();
        assert_eq!(invoke(side, pf::SV_PARTICLEEFFECTNUM), PRBI_OK);
        counts.push(console_warnings().len() - before);

        // Read here, not after the loop: the Rust iteration's reset zeroes the
        // C counter again.
        // SAFETY: a plain read of the composed TU's pr_ext.c:52 static.
        let warn_count = unsafe { ctest_m9fe_warn_count() };
        per_side.push((counts, warn_count));
    }

    assert_eq!(
        per_side[0].0,
        vec![1, 1, 1, 0, 1],
        "warns three times, then goes quiet, then warns again after the reset"
    );
    assert_eq!(per_side[0].0, per_side[1].0, "C vs Rust warning budget");
    assert_eq!(
        per_side[0].1, 1,
        "the C counter advanced once past the reset"
    );
}

#[test]
fn sv_particleeffectnum_effectinfo_enumerate_seam_is_reachable() {
    let _g = lock();
    // Slot 1 empty + an "effectinfo" r_particledesc is the only route into
    // COM_Effectinfo_Enumerate (pr_ext.c:4728). The ctest
    // PRExt_Glue_EffectinfoEnumerate mirror must therefore agree with the C
    // oracle's direct call -- including that COM_Effectinfo_Enumerate finds no
    // effectinfo.txt under the test gamedir and precaches nothing.
    let ((c, c_slot1), (rust, rust_slot1)) = run_both_probed(
        pf::SV_PARTICLEEFFECTNUM,
        4,
        0,
        PEXT2_REPLACEMENTDELTAS,
        SS_LOADING,
        0.0,
        || {
            // SAFETY: a `'static` literal that outlives the call.
            unsafe { ctest_m9fe_set_particledesc(c"effectinfo".as_ptr()) };
            set_global_i(OFS_PARM0, intern("blood"));
            set_argc(1);
        },
        |side| sv_precache(side, 1),
    );
    assert_eq!(c.status, PRBI_OK);
    assert_eq!(rust.status, PRBI_OK);
    assert_eq!(c, rust);
    assert_eq!(c_slot1.as_deref(), Some("blood"));
    assert_eq!(rust_slot1.as_deref(), Some("blood"));
}

// ---------------------------------------------------------------------------
// PF_sv_trailparticles / PF_sv_pointparticles

#[test]
fn sv_trailparticles_writes_and_fans_out() {
    let _g = lock();
    let (c, rust) = run_both(
        pf::SV_TRAILPARTICLES,
        4,
        2,
        PEXT2_REPLACEMENTDELTAS,
        SS_ACTIVE,
        0.0,
        || {
            set_global_f(OFS_PARM0, 7.0);
            set_global_i(OFS_PARM1, edict_prog(2));
            set_vector(OFS_PARM2, [1.0, -2.5, 3.25]);
            set_vector(OFS_PARM3, [-8.0, 16.5, 0.0]);
            set_argc(4);
        },
    );
    assert_eq!(c.status, PRBI_OK);
    assert_eq!(rust.status, PRBI_OK);
    assert!(
        !c.client0_dg.is_empty() && !c.client1_dg.is_empty(),
        "MULTICAST_PHS_U with requireext2 fans out into every client's \
         UNRELIABLE datagram (pr_ext.c:4283)"
    );
    assert!(c.client0.is_empty(), "nothing goes to the reliable message");
    assert!(c.multicast.is_empty(), "sv.multicast is cleared afterwards");
    assert_eq!(c, rust);
}

#[test]
fn sv_trailparticles_dp_compat_swaps_the_first_two_args() {
    let _g = lock();
    // A huge OFS_PARM1 takes the `(unsigned)G_INT(OFS_PARM1) >= MAX_EDICTS *
    // edict_size` branch, where OFS_PARM0 is the edict and OFS_PARM1 the
    // effect number (pr_ext.c:4780).
    let (c, rust) = run_both(
        pf::SV_TRAILPARTICLES,
        4,
        2,
        PEXT2_REPLACEMENTDELTAS,
        SS_ACTIVE,
        0.0,
        || {
            set_global_i(OFS_PARM0, edict_prog(3));
            set_global_f(OFS_PARM1, 9.0);
            set_vector(OFS_PARM2, [4.0, 5.0, 6.0]);
            set_vector(OFS_PARM3, [7.0, 8.0, 9.0]);
            set_argc(4);
        },
    );
    assert!(!c.client0_dg.is_empty());
    assert_eq!(c, rust);
}

#[test]
fn sv_trailparticles_nonpositive_effect_writes_nothing() {
    let _g = lock();
    for efnum in [0.0f32, -1.0] {
        let (c, rust) = run_both(
            pf::SV_TRAILPARTICLES,
            4,
            2,
            PEXT2_REPLACEMENTDELTAS,
            SS_ACTIVE,
            0.0,
            || {
                set_global_f(OFS_PARM0, efnum);
                set_global_i(OFS_PARM1, edict_prog(2));
                set_vector(OFS_PARM2, [1.0, 2.0, 3.0]);
                set_vector(OFS_PARM3, [4.0, 5.0, 6.0]);
                set_argc(4);
            },
        );
        assert!(
            c.client0_dg.is_empty() && c.multicast.is_empty(),
            "efnum {efnum}"
        );
        assert_eq!(c, rust, "efnum {efnum}");
    }
}

#[test]
fn sv_pointparticles_defaults_velocity_and_count() {
    let _g = lock();
    // argc 2: `vel` falls back to the shared vec3_origin and `count` to 1
    // (pr_ext.c:4806-4808).
    let (c, rust) = run_both(
        pf::SV_POINTPARTICLES,
        4,
        2,
        PEXT2_REPLACEMENTDELTAS,
        SS_ACTIVE,
        0.0,
        || {
            set_global_f(OFS_PARM0, 5.0);
            set_vector(OFS_PARM1, [10.0, 20.0, 30.0]);
            // Deliberately poisoned: argc 2 must make these unreadable.
            set_vector(OFS_PARM2, [111.0, 222.0, 333.0]);
            set_global_f(OFS_PARM3, 99.0);
            set_argc(2);
        },
    );
    assert_eq!(c.status, PRBI_OK);
    assert!(!c.client0_dg.is_empty());
    assert_eq!(c, rust);
}

#[test]
fn sv_pointparticles_with_explicit_velocity_and_count() {
    let _g = lock();
    let (c, rust) = run_both(
        pf::SV_POINTPARTICLES,
        4,
        2,
        PEXT2_REPLACEMENTDELTAS,
        SS_ACTIVE,
        0.0,
        || {
            set_global_f(OFS_PARM0, 5.0);
            set_vector(OFS_PARM1, [10.0, 20.0, 30.0]);
            set_vector(OFS_PARM2, [-1.5, 0.0, 2.75]);
            set_global_f(OFS_PARM3, 42.0);
            set_argc(4);
        },
    );
    assert!(!c.client0_dg.is_empty());
    assert_eq!(c, rust);
}

#[test]
fn sv_pointparticles_nonpositive_effect_writes_nothing() {
    let _g = lock();
    let (c, rust) = run_both(
        pf::SV_POINTPARTICLES,
        4,
        2,
        PEXT2_REPLACEMENTDELTAS,
        SS_ACTIVE,
        0.0,
        || {
            set_global_f(OFS_PARM0, 0.0);
            set_vector(OFS_PARM1, [1.0, 2.0, 3.0]);
            set_argc(2);
        },
    );
    assert!(c.client0_dg.is_empty() && c.multicast.is_empty());
    assert_eq!(c, rust);
}

// ---------------------------------------------------------------------------
// PF_cl_particleeffectnum

#[test]
fn cl_particleeffectnum_empty_string_returns_zero_without_raising() {
    let _g = lock();
    let (c, rust) = run_both(
        pf::CL_PARTICLEEFFECTNUM,
        4,
        0,
        PEXT2_REPLACEMENTDELTAS,
        SS_ACTIVE,
        0.0,
        || {
            set_global_i(OFS_PARM0, intern(""));
            set_argc(1);
        },
    );
    assert_eq!(
        c.status, PRBI_OK,
        "the empty-string early return precedes the raise"
    );
    assert_eq!(rust.status, PRBI_OK);
    assert_eq!(c.ret, 0.0);
    assert_eq!(c, rust);
}

#[test]
fn cl_particleeffectnum_finds_ssqc_precache() {
    let _g = lock();
    let (c, rust) = run_both(
        pf::CL_PARTICLEEFFECTNUM,
        4,
        0,
        PEXT2_REPLACEMENTDELTAS,
        SS_ACTIVE,
        0.0,
        || {
            set_cl_precache(1, c"other", 11);
            set_cl_precache(2, c"blood", 22);
            set_global_i(OFS_PARM0, intern("blood"));
            set_argc(1);
        },
    );
    assert_eq!(c.ret, 2.0, "positive: an ssqc-originated index");
    assert_eq!(c, rust);
}

#[test]
fn cl_particleeffectnum_finds_csqc_precache_as_negative() {
    let _g = lock();
    // cl.particle_precache[1] stays NULL so the ssqc loop breaks immediately;
    // a populated local table then matches, returning -i. This keeps
    // PScript_FindParticleType (an abort double) off the path.
    let ((c, c_local2), (rust, rust_local2)) = run_both_probed(
        pf::CL_PARTICLEEFFECTNUM,
        4,
        0,
        PEXT2_REPLACEMENTDELTAS,
        SS_ACTIVE,
        0.0,
        || {
            set_cl_local_precache(1, c"other", 33);
            set_cl_local_precache(2, c"blood", 44);
            set_global_i(OFS_PARM0, intern("blood"));
            set_argc(1);
        },
        |side| cl_local_precache(side, 2),
    );
    assert_eq!(c.ret, -2.0, "negative: a csqc-originated index");
    assert_eq!(c, rust);
    assert_eq!(
        c_local2.as_deref(),
        Some("blood"),
        "the match allocates nothing"
    );
    assert_eq!(rust_local2.as_deref(), Some("blood"));
}

#[test]
fn cl_particleeffectnum_overflow_raises() {
    let _g = lock();
    let mut obs = Vec::new();
    for side in [Side::C, Side::Rust] {
        reset(4, 0, PEXT2_REPLACEMENTDELTAS, SS_ACTIVE, 0.0);
        // Both tables full and non-matching: PF_CL_ForceParticlePrecache
        // returns 0 without ever reaching PScript_FindParticleType.
        // SAFETY: no arguments; fills the fixture's own tables.
        unsafe { ctest_m9fe_fill_cl_precache() };
        set_global_i(OFS_PARM0, intern("nomatch"));
        set_argc(1);
        let status = invoke(side, pf::CL_PARTICLEEFFECTNUM);
        // Captured per side: the next iteration's fixture reset clears the
        // console log and the Host_Error channel along with everything else.
        obs.push((
            status,
            observe(side, status),
            console_all(),
            host_error_message(),
        ));
    }

    assert_eq!(obs[0].0, CTEST_GUARD_HOST_ERROR, "the C side must raise");
    assert_eq!(obs[1].0, PRBI_ERR_CL_PARTICLEEFFECTNUM_OVERFLOW);
    assert_eq!(obs[0].3.as_str(), "Program error");
    assert!(
        obs[0]
            .2
            .iter()
            .any(|l| l.contains("PF_cl_particleeffectnum: overflow")),
        "got {:?}",
        obs[0].2
    );
    assert!(
        !obs[1]
            .2
            .iter()
            .any(|l| l.contains("PF_cl_particleeffectnum: overflow")),
        "the Rust half returns a status and prints nothing, got {:?}",
        obs[1].2
    );

    assert_eq!(obs[0].1.ret, 0.0, "OFS_RETURN is 0 when the raise fires");
    assert_eq!(obs[0].1.ret, obs[1].1.ret);
    assert_eq!(obs[0].1.warnings, obs[1].1.warnings);
    assert_eq!(obs[0].1.pscript, obs[1].1.pscript);
}

// ---------------------------------------------------------------------------
// PF_cl_trailparticles / PF_cl_pointparticles (observed through the shared
// PScript_* recorders).

#[test]
fn cl_trailparticles_passes_host_frametime_through() {
    let _g = lock();
    let (c, rust) = run_both(
        pf::CL_TRAILPARTICLES,
        4,
        0,
        PEXT2_REPLACEMENTDELTAS,
        SS_ACTIVE,
        0.0,
        || {
            // SAFETY: a plain double store into stubs.c's shared global.
            unsafe { ctest_m9fe_set_host_frametime(0.125) };
            set_cl_precache(1, c"other", 11);
            set_cl_precache(2, c"blood", 22);
            set_global_f(OFS_PARM0, 2.0);
            set_global_i(OFS_PARM1, edict_prog(2));
            set_vector(OFS_PARM2, [1.0, 2.0, 3.0]);
            set_vector(OFS_PARM3, [4.0, 5.0, 6.0]);
            set_argc(4);
        },
    );
    assert_eq!(c.status, PRBI_OK);
    assert_eq!(rust.status, PRBI_OK);
    assert_eq!(c.pscript.0, 1, "one PScript_ParticleTrail call");
    assert_eq!(
        f32::from_bits(c.pscript.1),
        0.125,
        "host_frametime reaches the trail as timeinterval"
    );
    assert_eq!(c, rust);
}

#[test]
fn cl_trailparticles_dp_compat_swaps_the_first_two_args() {
    let _g = lock();
    let (c, rust) = run_both(
        pf::CL_TRAILPARTICLES,
        4,
        0,
        PEXT2_REPLACEMENTDELTAS,
        SS_ACTIVE,
        0.0,
        || {
            // SAFETY: as above.
            unsafe { ctest_m9fe_set_host_frametime(0.05) };
            set_cl_local_precache(1, c"csqc", 77);
            set_global_i(OFS_PARM0, edict_prog(3));
            set_global_f(OFS_PARM1, 1.0);
            set_vector(OFS_PARM2, [0.0, 0.0, 0.0]);
            set_vector(OFS_PARM3, [1.0, 1.0, 1.0]);
            set_argc(4);
        },
    );
    assert_eq!(c.pscript.0, 1);
    assert_eq!(c, rust);
}

#[test]
fn cl_trailparticles_nonpositive_effect_is_a_no_op() {
    let _g = lock();
    let (c, rust) = run_both(
        pf::CL_TRAILPARTICLES,
        4,
        0,
        PEXT2_REPLACEMENTDELTAS,
        SS_ACTIVE,
        0.0,
        || {
            set_global_f(OFS_PARM0, 0.0);
            set_global_i(OFS_PARM1, edict_prog(2));
            set_vector(OFS_PARM2, [1.0, 2.0, 3.0]);
            set_vector(OFS_PARM3, [4.0, 5.0, 6.0]);
            set_argc(4);
        },
    );
    assert_eq!(c.pscript.0, 0, "no trail call at all");
    assert_eq!(c, rust);
}

#[test]
fn cl_pointparticles_defaults_velocity_and_count() {
    let _g = lock();
    let (c, rust) = run_both(
        pf::CL_POINTPARTICLES,
        4,
        0,
        PEXT2_REPLACEMENTDELTAS,
        SS_ACTIVE,
        0.0,
        || {
            set_cl_precache(1, c"blood", 55);
            set_global_f(OFS_PARM0, 1.0);
            set_vector(OFS_PARM1, [10.0, 20.0, 30.0]);
            set_vector(OFS_PARM2, [111.0, 222.0, 333.0]);
            set_global_f(OFS_PARM3, 99.0);
            set_argc(2);
        },
    );
    assert_eq!(c.pscript.2, 1, "one PScript_RunParticleEffectState call");
    assert_eq!(
        f32::from_bits(c.pscript.3),
        1.0,
        "argc < 4 defaults count to 1, ignoring the poisoned OFS_PARM3"
    );
    assert_eq!(c, rust);
}

#[test]
fn cl_pointparticles_truncates_the_count_toward_zero() {
    let _g = lock();
    // `int count = (int)G_FLOAT(OFS_PARM3)` -- C truncates; the Rust port must
    // reproduce that rather than round (ADR-010).
    for (input, expect) in [(3.9f32, 3.0f32), (7.0, 7.0), (1.2, 1.0)] {
        let (c, rust) = run_both(
            pf::CL_POINTPARTICLES,
            4,
            0,
            PEXT2_REPLACEMENTDELTAS,
            SS_ACTIVE,
            0.0,
            || {
                set_cl_precache(1, c"blood", 55);
                set_global_f(OFS_PARM0, 1.0);
                set_vector(OFS_PARM1, [1.0, 2.0, 3.0]);
                set_vector(OFS_PARM2, [0.0, 0.0, 0.0]);
                set_global_f(OFS_PARM3, input);
                set_argc(4);
            },
        );
        assert_eq!(c.pscript.2, 1, "input {input}");
        assert_eq!(f32::from_bits(c.pscript.3), expect, "input {input}");
        assert_eq!(c, rust, "input {input}");
    }
}

#[test]
fn cl_pointparticles_rejects_count_below_one() {
    let _g = lock();
    // `if (count < 1) return;` after the truncation, so 0.9 is also rejected.
    for input in [0.9f32, 0.0, -3.0] {
        let (c, rust) = run_both(
            pf::CL_POINTPARTICLES,
            4,
            0,
            PEXT2_REPLACEMENTDELTAS,
            SS_ACTIVE,
            0.0,
            || {
                set_cl_precache(1, c"blood", 55);
                set_global_f(OFS_PARM0, 1.0);
                set_vector(OFS_PARM1, [1.0, 2.0, 3.0]);
                set_vector(OFS_PARM2, [0.0, 0.0, 0.0]);
                set_global_f(OFS_PARM3, input);
                set_argc(4);
            },
        );
        assert_eq!(c.pscript.2, 0, "input {input}");
        assert_eq!(c, rust, "input {input}");
    }
}

#[test]
fn cl_pointparticles_nonpositive_effect_is_a_no_op() {
    let _g = lock();
    let (c, rust) = run_both(
        pf::CL_POINTPARTICLES,
        4,
        0,
        PEXT2_REPLACEMENTDELTAS,
        SS_ACTIVE,
        0.0,
        || {
            set_global_f(OFS_PARM0, -1.0);
            set_vector(OFS_PARM1, [1.0, 2.0, 3.0]);
            set_argc(2);
        },
    );
    assert_eq!(c.pscript.2, 0);
    assert_eq!(c, rust);
}
