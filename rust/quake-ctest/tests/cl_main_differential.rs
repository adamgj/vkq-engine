//! Differential/characterization gate for `Quake/cl_main.c` -- the client
//! state machine, dlight table, entity relink and command entry points.
//! Rust migration Phase 7, M7, task T7.4.
//!
//! The oracle fixture is `stubs/cl_main_ref.c`; read its module doc first.
//! The short version: `cl_main_glue.c` is `#ifdef USE_RUST_HOST` and
//! `cl_main.c` is an oracle source, so that file owns the plain twins of the
//! fifteen cvars/objects the glue defines, the 41 `ClMain_Glue_*` trampolines
//! and the re-raising command entry points, and every seeder writes both sides
//! in one call.
//!
//! ## What T7.4 changes, and why it matters for these comparisons
//!
//! ADR-007's `cl`/`cls` row closes with this task: `quake-capi/src/cl_main.rs`
//! now defines the plain pair and `cl_main.c` keeps `c_ref_cl`/`c_ref_cls`.
//! The two sides therefore still read two *different* copies of the client
//! state -- which is exactly what makes a byte-image comparison meaningful --
//! but the plain copy is Rust storage rather than `stubs.c` storage.
//!
//! ## What makes a comparison here non-vacuous
//!
//! Nothing in this link runs `CL_Init` or `CL_ParseServerInfo`, so
//! `cl.entities`, `cl.scores`, `cl_dlights` and `cls.message` are NULL/zero
//! from static init on BOTH sides, and `cvar_t.value` is zero for every cvar
//! because `Cvar_RegisterVariable` never runs. A bit-exact differential passes
//! happily when both sides do nothing to nothing: with `cl.entities` NULL,
//! `CL_PrintEntities_f` prints nothing and `CL_RelinkEntities` walks nothing.
//! `ctest_clmain_reset` therefore publishes a live starting state into both
//! copies -- a 64-entry entity array with models attached, four seeded
//! dlights, an 8KB `cls.message`, `cls.state = ca_connected`, `cls.signon =
//! 1`, `cl.time = 1.5` with `mtime` straddling it, and every `cvar_t.value`
//! filled in by hand -- and every test below asserts something *positive* (a
//! specific console line, a value that actually changed, a non-empty message)
//! alongside the cross-side comparison.
//!
//! The console log is a single shared `stubs.c` buffer, so it is cleared
//! between the two runs rather than merely read afterwards. Running one side
//! before snapshotting the other is precisely the mistake that makes a
//! differential vacuous.
//!
//! ## The abort-stub ceiling, stated as a limit and not as coverage
//!
//! `CL_FreeState` reaches the `PR_ClearProgs` abort stub, `CL_Disconnect`
//! reaches `Host_ShutdownServer`, `CL_RelinkEntities` reaches
//! `R_AllocateEntityBLAS`/`PScript_*`, `CL_SendCmd` reaches
//! `NET_CanSendMessage` and `CL_ReadFromServer` reaches `CL_GetMessage`'s
//! `NET_GetMessage`. Both sides stop at the same stub, so what is compared is
//! *which* stub was reached, with what message, and every observable mutation
//! made before it. For the Rust side that round trip is the whole of ADR-009
//! -- trampoline, `Host_Guard`, status code, `ClMain_Raise`, `Host_Reraise` --
//! so an arm that "only aborts" still proves the raise topology end to end.
//! What it does not prove is what the real callee would have done; that is
//! carried as a coverage gap.
//!
//! ## The seven static handlers, which have NO oracle twin
//!
//! `CL_LegacyColor_f`, the five `CL_ServerExtension_*_f` and
//! `CL_Viewpos_Completion_f` are `static` in `cl_main.c` (lines 1178, 1185,
//! 1190, 1214, 1225, 1345, 1350). `c_ref_prelude.h` only renames non-static
//! symbols, so there is no `c_ref_` twin to link against and no differential
//! is possible. Those seven are exercised Rust-side only, against expected
//! values read off `cl_main.c`. That is a real gap, and the tests concerned
//! say so in their names and doc comments.
//!
//! ## ADR-005
//!
//! Every format specifier reachable from `cl_main.c` is `%i`, `%3i`, `%2i`,
//! `%s`, `%d` or `%5.1f`. There is no `%g` and no `%e`, so the Rust float
//! formatter's documented panic is not reachable from this file.

use core::ffi::{c_char, c_double, c_float, c_int};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// `stubs.c:1465-1467` -- `Host_Guard`'s status codes.
const GUARD_OK: c_int = 0;
const GUARD_HOST_ERROR: c_int = 1;
const GUARD_SYS_ERROR: c_int = 2;

/// `client.h` -- `cactive_t`.
const CA_DEDICATED: c_int = 0;
const CA_DISCONNECTED: c_int = 1;
const CA_CONNECTED: c_int = 2;

/// The oracle runs first so a shared object it dirties is observed before the
/// port's run resets it.
const SIDES: [c_int; 2] = [1 /* oracle */, 0 /* rust */];

fn side_name(side: c_int) -> &'static str {
    if side == 1 {
        "C"
    } else {
        "Rust"
    }
}

extern "C" {
    // stubs/cl_main_ref.c -- seeders (each writes BOTH sides)
    fn ctest_clmain_reset();
    fn ctest_clmain_set_time(time: c_double, oldtime: c_double, mtime0: c_double, mtime1: c_double);
    fn ctest_clmain_set_conn(
        state: c_int,
        signon: c_int,
        demoplayback: c_int,
        demorecording: c_int,
    );
    fn ctest_clmain_set_timedemo(timedemo: c_int);
    fn ctest_clmain_set_demoloop(demonum: c_int, count: c_int, prefix: *const c_char);
    fn ctest_clmain_set_counts(maxclients: c_int, viewentity: c_int, num_entities: c_int);
    fn ctest_clmain_set_paused(paused: c_int, intermission: c_int);
    fn ctest_clmain_set_sv_active(active: c_int);
    fn ctest_clmain_set_nolerp(v: c_float);
    fn ctest_clmain_seed_dlights();
    fn ctest_clmain_set_dlight(
        idx: c_int,
        key: c_int,
        die: c_float,
        radius: c_float,
        decay: c_float,
    );
    fn ctest_clmain_attach_arrays(nedicts: c_int);
    #[allow(clippy::too_many_arguments)]
    fn ctest_clmain_set_entity(
        idx: c_int,
        model: c_int,
        frame: c_int,
        x: c_float,
        y: c_float,
        z: c_float,
        pitch: c_float,
        yaw: c_float,
        roll: c_float,
    );
    fn ctest_clmain_attach_message();
    fn ctest_clmain_set_message_maxsize(maxsize: c_int);
    fn ctest_clmain_set_userinfo(info: *const c_char);
    fn ctest_clmain_set_serverinfo(info: *const c_char);

    // stubs/cl_main_ref.c -- read-backs
    fn ctest_clmain_cl_image_size() -> c_int;
    fn ctest_clmain_get_cl_image(side: c_int, out: *mut u8);
    fn ctest_clmain_cls_image_size() -> c_int;
    fn ctest_clmain_get_cls_image(side: c_int, out: *mut u8);
    fn ctest_clmain_dlight_size() -> c_int;
    fn ctest_clmain_get_dlight(side: c_int, idx: c_int, out: *mut u8);
    fn ctest_clmain_entity_size() -> c_int;
    fn ctest_clmain_get_entity(side: c_int, idx: c_int, out: *mut u8);
    fn ctest_clmain_get_message_size(side: c_int) -> c_int;
    fn ctest_clmain_get_message_data(side: c_int) -> *const u8;
    fn ctest_clmain_get_userinfo(side: c_int) -> *const c_char;
    fn ctest_clmain_get_serverinfo(side: c_int) -> *const c_char;
    fn ctest_clmain_clipboard() -> *const c_char;

    // stubs/cl_main_ref.c -- drivers (all enter through Host_Guard)
    fn ctest_clmain_lerp_point(side: c_int, out: *mut c_float) -> c_int;
    fn ctest_clmain_alloc_dlight(side: c_int, key: c_int, outidx: *mut c_int) -> c_int;
    fn ctest_clmain_decay_lights(side: c_int) -> c_int;
    fn ctest_clmain_print_entities(side: c_int) -> c_int;
    fn ctest_clmain_signon_reply(side: c_int) -> c_int;
    fn ctest_clmain_next_demo(side: c_int) -> c_int;
    fn ctest_clmain_relink_entities(side: c_int) -> c_int;
    fn ctest_clmain_clear_state(side: c_int) -> c_int;
    fn ctest_clmain_free_state(side: c_int) -> c_int;
    fn ctest_clmain_clear_trail_states(side: c_int) -> c_int;
    fn ctest_clmain_disconnect(side: c_int) -> c_int;
    fn ctest_clmain_disconnect_f(side: c_int) -> c_int;
    fn ctest_clmain_read_from_server(side: c_int, out: *mut c_int) -> c_int;
    fn ctest_clmain_send_cmd(side: c_int) -> c_int;
    fn ctest_clmain_accumulate_cmd(side: c_int) -> c_int;
    fn ctest_clmain_set_mvelocity(v0: *const c_float, v1: *const c_float);
    fn ctest_clmain_set_entity_msg(
        idx: c_int,
        msgtime: f64,
        o0: *const c_float,
        o1: *const c_float,
        a0: *const c_float,
        a1: *const c_float,
    );
    fn ctest_pscript_reset();
    fn ctest_pscript_trail_count() -> c_int;
    fn ctest_pscript_last_timeinterval_value() -> c_float;
    fn ctest_scr_zoom_reset();
    fn ctest_scr_zoom_count() -> c_int;
    fn ctest_entity_dlights_reset();
    fn ctest_entity_dlights_count() -> c_int;
    fn ctest_clmain_get_cl_time(side: c_int) -> f64;
    fn ctest_clmain_get_dlight_radius(side: c_int, idx: c_int) -> c_float;
    fn ctest_clmain_get_velocity(side: c_int, out: *mut c_float);
    fn ctest_clmain_attach_world(org: *const c_float, fwd: *const c_float);
    fn ctest_clmain_tracepos(side: c_int) -> c_int;
    fn ctest_clmain_viewpos(side: c_int) -> c_int;

    // stubs/cl_main_ref.c -- Rust-only (the `static` handlers, see module doc)
    fn ctest_clmain_rust_legacy_color() -> c_int;
    fn ctest_clmain_rust_serverext(which: c_int) -> c_int;
    fn ctest_clmain_rust_viewpos_completion(partial: *const c_char) -> c_int;
    fn ctest_clmain_get_score_userinfo(side: c_int, slot: c_int) -> *const c_char;
    fn ctest_clmain_get_score_name(side: c_int, slot: c_int) -> *const c_char;
    fn ctest_clmain_get_score_colors(side: c_int, slot: c_int) -> c_int;
    fn ctest_clmain_register_color_cvars() -> c_int;
    fn ctest_clmain_get_color_cvar(which: c_int) -> c_float;

    // stubs/cl_input_ref.c -- tokenizes BOTH command tables in one call
    fn ctest_clinput_tokenize(text: *const c_char);

    // stubs.c
    fn ctest_sys_error_message() -> *const c_char;
    fn ctest_host_error_message() -> *const c_char;
    fn ctest_clear_con_log();
    // stubs/console_ref.c -- console.c became an oracle source at Phase 7
    // M10c, so Con_AddToTabList is the real port now and the completion
    // list itself is the observation.
    fn ctest_console_reset(side: c_int);
    fn ctest_console_tablist_count(side: c_int) -> c_int;
    fn ctest_console_tablist_entry(
        side: c_int,
        idx: c_int,
        name: *mut c_char,
        namecap: c_int,
        ty: *mut c_char,
        typecap: c_int,
    ) -> c_int;
    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(i: c_int) -> *const c_char;
}

fn guard_message(status: c_int) -> String {
    // SAFETY: both getters return a pointer to a static NUL-terminated buffer
    // in stubs.c with process lifetime; callers hold TEST_LOCK.
    unsafe {
        let p = match status {
            GUARD_HOST_ERROR => ctest_host_error_message(),
            GUARD_SYS_ERROR => ctest_sys_error_message(),
            _ => return String::new(),
        };
        core::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

fn con_log() -> Vec<String> {
    // SAFETY: stubs.c getters over static storage; caller holds TEST_LOCK.
    unsafe {
        let n = ctest_con_log_len().clamp(0, 64);
        (0..n)
            .map(|i| {
                core::ffi::CStr::from_ptr(ctest_con_log_get(i))
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }
}

/// # Safety
/// `p` is NULL or points at a NUL-terminated buffer that outlives the call.
unsafe fn opt_cstr(p: *const c_char) -> Option<String> {
    if p.is_null() {
        None
    } else {
        // SAFETY: caller contract
        Some(
            unsafe { core::ffi::CStr::from_ptr(p) }
                .to_string_lossy()
                .into_owned(),
        )
    }
}

fn tokenize(text: &str) {
    let c = CString::new(text).unwrap();
    // SAFETY: NUL-terminated; the fixture tokenizes both command tables and
    // copies the text. Caller holds TEST_LOCK.
    unsafe { ctest_clinput_tokenize(c.as_ptr()) }
}

/// Everything observable after one driver call on one side.
#[derive(Debug, PartialEq, Eq)]
struct Snap {
    guard: c_int,
    msg: String,
    cl: Vec<u8>,
    cls: Vec<u8>,
    message: Vec<u8>,
    con: Vec<String>,
}

fn snap(side: c_int, guard: c_int) -> Snap {
    // SAFETY: fixture read-backs over static storage sized by the paired
    // *_size() getters; caller holds TEST_LOCK.
    unsafe {
        let mut cl = vec![0u8; ctest_clmain_cl_image_size() as usize];
        ctest_clmain_get_cl_image(side, cl.as_mut_ptr());
        let mut cls = vec![0u8; ctest_clmain_cls_image_size() as usize];
        ctest_clmain_get_cls_image(side, cls.as_mut_ptr());
        let n = ctest_clmain_get_message_size(side).max(0) as usize;
        let p = ctest_clmain_get_message_data(side);
        let message = core::slice::from_raw_parts(p, n).to_vec();
        Snap {
            guard,
            msg: guard_message(guard),
            cl,
            cls,
            message,
            con: con_log(),
        }
    }
}

/// The oracle side. Named because the positive assertions below read one
/// specific side after the cross-side comparison has already passed.
const SIDE_C: c_int = 1;

fn cl_time(side: c_int) -> f64 {
    // SAFETY: fixture read-back over static storage; caller holds TEST_LOCK.
    unsafe { ctest_clmain_get_cl_time(side) }
}

fn dlight_radius(side: c_int, idx: c_int) -> c_float {
    // SAFETY: fixture read-back over static storage; caller holds TEST_LOCK.
    unsafe { ctest_clmain_get_dlight_radius(side, idx) }
}

fn velocity(side: c_int) -> [u32; 3] {
    let mut v = [0f32; 3];
    // SAFETY: `v` is the three floats the fixture writes; caller holds
    // TEST_LOCK.
    unsafe { ctest_clmain_get_velocity(side, v.as_mut_ptr()) };
    v.map(f32::to_bits)
}

fn dlights(side: c_int, count: c_int) -> Vec<u8> {
    // SAFETY: fixture read-back over static storage; caller holds TEST_LOCK.
    unsafe {
        let sz = ctest_clmain_dlight_size() as usize;
        let mut out = vec![0u8; sz * count.max(0) as usize];
        for i in 0..count {
            ctest_clmain_get_dlight(side, i, out[i as usize * sz..].as_mut_ptr());
        }
        out
    }
}

fn entities(side: c_int, count: c_int) -> Vec<u8> {
    // SAFETY: fixture read-back over static storage; caller holds TEST_LOCK.
    unsafe {
        let sz = ctest_clmain_entity_size() as usize;
        let mut out = vec![0u8; sz * count.max(0) as usize];
        for i in 0..count {
            ctest_clmain_get_entity(side, i, out[i as usize * sz..].as_mut_ptr());
        }
        out
    }
}

/// Runs `seed` then `drive` on each side in turn, with a full fixture reset
/// and a cleared console log before each, and returns `(oracle, rust)`.
fn run_both<S, D>(seed: S, drive: D) -> (Snap, Snap)
where
    S: Fn(),
    D: Fn(c_int) -> c_int,
{
    let mut out: Vec<Snap> = Vec::with_capacity(2);
    for side in SIDES {
        // SAFETY: fixture reset over static storage; caller holds TEST_LOCK.
        unsafe {
            ctest_clmain_reset();
            ctest_clear_con_log();
        }
        seed();
        let g = drive(side);
        out.push(snap(side, g));
    }
    let rust = out.pop().unwrap();
    let c = out.pop().unwrap();
    (c, rust)
}

fn assert_same(c: &Snap, rust: &Snap, what: &str) {
    assert_eq!(
        (c.guard, &c.msg),
        (rust.guard, &rust.msg),
        "{what}: guard status/message differ ({} vs {})",
        side_name(1),
        side_name(0)
    );
    assert_eq!(c.con, rust.con, "{what}: console output differs");
    assert_eq!(c.message, rust.message, "{what}: cls.message bytes differ");
    assert_image_eq(&c.cls, &rust.cls, what, "client_static_t");
    assert_image_eq(&c.cl, &rust.cl, what, "client_state_t");
}

/// Byte-compares two struct images. `assert_eq!` on the raw vectors dumps
/// tens of kilobytes per side, which is useless; this reports the first
/// differing offset and a short window around it instead.
fn cl_image_size() -> usize {
    // SAFETY: a pure `sizeof` query with no state.
    unsafe { ctest_clmain_cl_image_size() as usize }
}

fn assert_image_eq(c: &[u8], rust: &[u8], what: &str, label: &str) {
    assert_eq!(c.len(), rust.len(), "{what}: {label} image size differs");
    if let Some(off) = (0..c.len()).find(|&i| c[i] != rust[i]) {
        let lo = off.saturating_sub(8);
        let hi = (off + 24).min(c.len());
        panic!(
            "{what}: {label} images differ at byte {off} (of {})\n  {}: {:?}\n  {}: {:?}",
            c.len(),
            side_name(1),
            &c[lo..hi],
            side_name(0),
            &rust[lo..hi]
        );
    }
}

// ---------------------------------------------------------------------------
// CL_LerpPoint (cl_main.c:452-497)

/// One `CL_LerpPoint` observation: guard status, returned fraction, `cl` image.
type LerpOut = (c_int, f32, Vec<u8>);

/// Drives `CL_LerpPoint` on both sides and returns the two results plus the
/// two post-call `cl` images.
fn lerp_both() -> (LerpOut, LerpOut) {
    let mut res: Vec<LerpOut> = Vec::with_capacity(2);
    for side in SIDES {
        let mut f: c_float = f32::NAN;
        // SAFETY: fixture driver; caller holds TEST_LOCK.
        let g = unsafe { ctest_clmain_lerp_point(side, &mut f) };
        let mut cl = vec![0u8; cl_image_size()];
        // SAFETY: read-back over static storage; caller holds TEST_LOCK.
        unsafe { ctest_clmain_get_cl_image(side, cl.as_mut_ptr()) };
        res.push((g, f, cl));
    }
    let rust = res.pop().unwrap();
    let c = res.pop().unwrap();
    (c, rust)
}

#[test]
fn lerp_point_interpolates_between_mtimes() {
    let _g = lock();
    // SAFETY: fixture seeding over static storage; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clmain_set_time(1.52, 1.5, 1.55, 1.5);
        ctest_clmain_set_conn(CA_CONNECTED, 4, 0, 0);
    }
    let (c, rust) = lerp_both();
    assert_eq!(c.0, GUARD_OK, "oracle should not raise");
    assert_eq!(rust.0, GUARD_OK, "port should not raise");
    assert_eq!(
        c.1.to_bits(),
        rust.1.to_bits(),
        "CL_LerpPoint result differs bit-for-bit: C {} vs Rust {}",
        c.1,
        rust.1
    );
    assert_eq!(c.2, rust.2, "cl image differs after CL_LerpPoint");
    // Positive assertion: this is the interpolating arm, not a clamp.
    assert!(
        c.1 > 0.0 && c.1 < 1.0,
        "expected a strict interpolation fraction, got {}",
        c.1
    );
}

#[test]
fn lerp_point_clamps_and_rewrites_mtime_on_a_large_gap() {
    let _g = lock();
    // A >0.1s server gap makes CL_LerpPoint rewrite cl.mtime[1], which is the
    // only case where the function has a side effect.
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clmain_set_time(9.0, 8.9, 9.5, 8.0);
        ctest_clmain_set_conn(CA_CONNECTED, 4, 0, 0);
    }
    let (c, rust) = lerp_both();
    assert_eq!((c.0, rust.0), (GUARD_OK, GUARD_OK));
    assert_eq!(c.1.to_bits(), rust.1.to_bits(), "clamped fraction differs");
    assert_eq!(c.2, rust.2, "cl image differs (mtime[1] rewrite)");
    // f = 1.5 collapses to 0.1 with mtime[1] = mtime[0] - 0.1 = 9.4, so
    // frac = (9.0 - 9.4) / 0.1 = -4, which clamps to 0 and drags cl.time back
    // to mtime[1] (cl_main.c:456-467).
    assert_eq!(c.1, 0.0, "a backwards gap this large must clamp to 0");
}

#[test]
fn lerp_point_clamps_a_gap_only_just_over_a_tenth() {
    let _g = lock();
    // Pins the `f > 0.1` boundary itself (cl_main.c:454). f = 0.15 is on the
    // clamping side by only 0.05: cl.mtime[1] is rewritten to mtime[0] - 0.1
    // and f collapses to 0.1, so frac = (1.60 - 1.55) / 0.1 = 0.5. Widening
    // the threshold to 0.2 would leave mtime[1] alone and return 0.1 / 0.15
    // instead, which the existing f = 1.5 case does not constrain.
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clmain_set_time(1.60, 1.5, 1.65, 1.5);
        ctest_clmain_set_conn(CA_CONNECTED, 4, 0, 0);
    }
    let (c, rust) = lerp_both();
    assert_eq!((c.0, rust.0), (GUARD_OK, GUARD_OK));
    assert_eq!(c.1.to_bits(), rust.1.to_bits(), "clamped fraction differs");
    assert_eq!(c.2, rust.2, "cl image differs (mtime[1] rewrite)");
    assert_eq!(c.1, 0.5, "f must have been clamped to 0.1 first");
}

#[test]
fn lerp_point_drags_time_forward_on_a_small_negative_frac() {
    let _g = lock();
    // frac = (1.44 - 1.45) / 0.05 = -0.2, past the -0.01 threshold, so
    // cl_main.c:462 rewrites cl.time to cl.mtime[1]. The returned fraction is
    // 0 either way -- only cl.time records whether the rewrite happened, so
    // this needs its own case and its own read-back.
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clmain_set_time(1.44, 1.4, 1.5, 1.45);
        ctest_clmain_set_conn(CA_CONNECTED, 4, 0, 0);
    }
    let (c, rust) = lerp_both();
    assert_eq!((c.0, rust.0), (GUARD_OK, GUARD_OK));
    assert_eq!(c.1.to_bits(), rust.1.to_bits());
    assert_eq!(c.2, rust.2, "cl image differs (cl.time rewrite)");
    assert_eq!(c.1, 0.0, "a negative fraction always returns 0");
    assert_eq!(
        cl_time(SIDE_C),
        1.45,
        "cl.time must be dragged up to cl.mtime[1]"
    );
}

#[test]
fn lerp_point_drags_time_back_on_a_small_overshoot() {
    let _g = lock();
    // The mirror case: frac = (1.51 - 1.45) / 0.05 = 1.2 > 1.01, so
    // cl_main.c:469 rewrites cl.time back to cl.mtime[0].
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clmain_set_time(1.51, 1.4, 1.5, 1.45);
        ctest_clmain_set_conn(CA_CONNECTED, 4, 0, 0);
    }
    let (c, rust) = lerp_both();
    assert_eq!((c.0, rust.0), (GUARD_OK, GUARD_OK));
    assert_eq!(c.1.to_bits(), rust.1.to_bits());
    assert_eq!(c.2, rust.2, "cl image differs (cl.time rewrite)");
    assert_eq!(c.1, 1.0, "an overshooting fraction always returns 1");
    assert_eq!(
        cl_time(SIDE_C),
        1.5,
        "cl.time must be dragged back to cl.mtime[0]"
    );
}

#[test]
fn lerp_point_disabled_by_cl_nolerp() {
    let _g = lock();
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clmain_set_time(1.52, 1.5, 1.55, 1.5);
        ctest_clmain_set_nolerp(1.0);
    }
    let (c, rust) = lerp_both();
    assert_eq!(c.1.to_bits(), rust.1.to_bits());
    assert_eq!(c.2, rust.2);
    assert_eq!(c.1, 1.0, "cl_nolerp must short-circuit to 1");
}

#[test]
fn lerp_point_disabled_by_timedemo() {
    let _g = lock();
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clmain_set_time(1.52, 1.5, 1.55, 1.5);
        ctest_clmain_set_timedemo(1);
    }
    let (c, rust) = lerp_both();
    assert_eq!(c.1.to_bits(), rust.1.to_bits());
    assert_eq!(c.2, rust.2);
    assert_eq!(c.1, 1.0, "cls.timedemo must short-circuit to 1");
}

#[test]
fn lerp_point_disabled_by_local_server() {
    let _g = lock();
    // The `sv.active && !host_netinterval` arm. `sv` is renamed, so the two
    // sides read two different server structs and the seeder writes both.
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clmain_set_time(1.52, 1.5, 1.55, 1.5);
        ctest_clmain_set_sv_active(1);
    }
    let (c, rust) = lerp_both();
    // SAFETY: leave the shared server flag as the fixture found it.
    unsafe { ctest_clmain_set_sv_active(0) };
    assert_eq!(c.1.to_bits(), rust.1.to_bits());
    assert_eq!(c.2, rust.2);
}

#[test]
fn lerp_point_zero_when_mtimes_are_equal() {
    let _g = lock();
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clmain_set_time(1.5, 1.4, 1.5, 1.5);
    }
    let (c, rust) = lerp_both();
    assert_eq!(c.1.to_bits(), rust.1.to_bits());
    assert_eq!(c.2, rust.2);
    assert_eq!(c.1, 1.0, "an identical mtime pair must return 1");
}

// ---------------------------------------------------------------------------
// CL_AllocDlight / CL_DecayLights (cl_main.c:378-450)

/// One `CL_AllocDlight` observation: guard status, slot index, dlight images.
type AllocOut = (c_int, c_int, Vec<u8>);

fn alloc_both(key: c_int) -> (AllocOut, AllocOut) {
    let mut res: Vec<AllocOut> = Vec::with_capacity(2);
    for side in SIDES {
        let mut idx: c_int = -9;
        // SAFETY: fixture driver; caller holds TEST_LOCK.
        let g = unsafe { ctest_clmain_alloc_dlight(side, key, &mut idx) };
        res.push((g, idx, dlights(side, 8)));
    }
    let rust = res.pop().unwrap();
    let c = res.pop().unwrap();
    (c, rust)
}

#[test]
fn alloc_dlight_reuses_the_matching_key() {
    let _g = lock();
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clmain_seed_dlights();
    }
    let (c, rust) = alloc_both(33);
    assert_eq!((c.0, rust.0), (GUARD_OK, GUARD_OK));
    assert_eq!(c.1, rust.1, "chosen dlight slot differs");
    assert_eq!(c.2, rust.2, "cl_dlights table differs");
    assert_eq!(c.1, 3, "key 33 was seeded into slot 3");
}

#[test]
fn alloc_dlight_takes_an_expired_slot_when_no_key_matches() {
    let _g = lock();
    // Slot 1 is seeded with die = 0.5 against cl.time = 1.5.
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clmain_seed_dlights();
    }
    let (c, rust) = alloc_both(4242);
    assert_eq!(c.1, rust.1, "chosen dlight slot differs");
    assert_eq!(c.2, rust.2, "cl_dlights table differs");
    assert_eq!(c.1, 1, "slot 1 is the only expired one");
}

#[test]
fn alloc_dlight_falls_back_to_slot_zero() {
    let _g = lock();
    // Every slot alive and keyed differently -> cl_main.c:411 falls back to
    // dl = &cl_dlights[0], which notably does NOT clear kex_intensity.
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clmain_seed_dlights();
        for i in 0..64 {
            ctest_clmain_set_dlight(i, 1000 + i, 99.0, 10.0, 1.0);
        }
    }
    let (c, rust) = alloc_both(-77);
    assert_eq!(c.1, rust.1, "chosen dlight slot differs");
    assert_eq!(c.2, rust.2, "cl_dlights table differs");
    assert_eq!(c.1, 0, "the fallback is always slot 0");
}

#[test]
fn alloc_dlight_keeps_a_slot_that_dies_exactly_now() {
    let _g = lock();
    // cl_main.c:388 reuses a slot only when `dl->die < cl.time`, strictly. A
    // slot whose die is exactly cl.time (1.5) is still alive, so the search
    // must fall through to the slot-0 fallback; a `<=` there would hand back
    // slot 7 instead. The existing expired-slot case (die 0.5) sits far from
    // the boundary and does not pin it.
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clmain_seed_dlights();
        for i in 0..64 {
            ctest_clmain_set_dlight(i, 1000 + i, 99.0, 10.0, 1.0);
        }
        ctest_clmain_set_dlight(7, 1007, 1.5, 10.0, 1.0);
    }
    let (c, rust) = alloc_both(-77);
    assert_eq!((c.0, rust.0), (GUARD_OK, GUARD_OK));
    assert_eq!(c.1, rust.1, "chosen dlight slot differs");
    assert_eq!(c.2, rust.2, "cl_dlights table differs");
    assert_eq!(
        c.1, 0,
        "die == cl.time is not expired, so slot 7 is skipped"
    );
}

#[test]
fn decay_lights_clamps_an_overshooting_radius_to_zero() {
    let _g = lock();
    // cl_main.c:437 -- radius 1 with decay 100 over a 0.1s frame lands at -9;
    // the clamp is the only thing that keeps it at 0. Every dlight in the
    // default fixture decays by far less than its radius, so nothing there
    // reaches the clamp at all.
    let mut after: Vec<Vec<u8>> = Vec::new();
    let mut radii: Vec<c_float> = Vec::new();
    for side in SIDES {
        // SAFETY: fixture seeding and driver; guarded by TEST_LOCK.
        unsafe {
            ctest_clmain_reset();
            ctest_clmain_seed_dlights();
            ctest_clmain_set_dlight(5, 55, 100.0, 1.0, 100.0);
            let g = ctest_clmain_decay_lights(side);
            assert_eq!(g, GUARD_OK, "{}", side_name(side));
        }
        after.push(dlights(side, 8));
        // Read inside the loop: the next iteration re-seeds BOTH copies, so a
        // read-back taken afterwards would report the fixture, not the result.
        radii.push(dlight_radius(side, 5));
    }
    assert_eq!(after[0], after[1], "cl_dlights differ after CL_DecayLights");
    assert_eq!(
        radii,
        vec![0.0, 0.0],
        "an overshooting radius must clamp to exactly 0 on both sides"
    );
}

#[test]
fn decay_lights_shrinks_live_radii_and_skips_dead_ones() {
    let _g = lock();
    let mut before: Vec<Vec<u8>> = Vec::new();
    let mut after: Vec<Vec<u8>> = Vec::new();
    for side in SIDES {
        // SAFETY: fixture seeding and driver; guarded by TEST_LOCK.
        unsafe {
            ctest_clmain_reset();
            ctest_clmain_seed_dlights();
            ctest_clmain_set_time(2.0, 1.75, 2.0, 1.9);
        }
        before.push(dlights(side, 8));
        // SAFETY: fixture driver.
        let g = unsafe { ctest_clmain_decay_lights(side) };
        assert_eq!(
            g,
            GUARD_OK,
            "{}: CL_DecayLights must not raise",
            side_name(side)
        );
        after.push(dlights(side, 8));
    }
    assert_eq!(after[0], after[1], "cl_dlights differ after CL_DecayLights");
    assert_ne!(
        before[1], after[1],
        "CL_DecayLights made no observable change -- the fixture is degenerate"
    );
}

#[test]
fn decay_lights_returns_early_on_negative_time() {
    let _g = lock();
    let mut after: Vec<Vec<u8>> = Vec::new();
    let mut unchanged = Vec::new();
    for side in SIDES {
        // SAFETY: fixture seeding and driver; guarded by TEST_LOCK.
        unsafe {
            ctest_clmain_reset();
            ctest_clmain_seed_dlights();
            ctest_clmain_set_time(1.0, 2.0, 1.0, 0.9);
        }
        let b = dlights(side, 8);
        // SAFETY: fixture driver.
        let g = unsafe { ctest_clmain_decay_lights(side) };
        assert_eq!(g, GUARD_OK);
        let a = dlights(side, 8);
        unchanged.push(b == a);
        after.push(a);
    }
    assert_eq!(after[0], after[1]);
    assert!(
        unchanged[0] && unchanged[1],
        "cl.time < cl.oldtime must leave cl_dlights untouched"
    );
}

// ---------------------------------------------------------------------------
// CL_PrintEntities_f (cl_main.c:333-360)

#[test]
fn print_entities_lists_the_populated_slots() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_clmain_set_conn(CA_CONNECTED, 4, 0, 0);
                ctest_clmain_set_counts(4, 1, 6);
                ctest_clmain_set_entity(3, -1, 0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
            }
        },
        // SAFETY: fixture driver; guarded by TEST_LOCK.
        |side| unsafe { ctest_clmain_print_entities(side) },
    );
    assert_same(&c, &rust, "CL_PrintEntities_f");
    assert!(
        c.con.iter().any(|l| l.contains("EMPTY")),
        "slot 3 was cleared, so an EMPTY line must be printed: {:?}",
        c.con
    );
    assert!(
        c.con.iter().any(|l| l.contains("progs/ctest")),
        "the populated slots must print their model name: {:?}",
        c.con
    );
}

#[test]
fn print_entities_requires_a_connected_client() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_clmain_set_conn(CA_DISCONNECTED, 0, 0, 0) }
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_print_entities(side) },
    );
    assert_same(&c, &rust, "CL_PrintEntities_f (disconnected)");
    assert!(c.con.is_empty(), "nothing may be printed: {:?}", c.con);
}

// ---------------------------------------------------------------------------
// CL_SignonReply (cl_main.c:257-292) -- the write-batch path

#[test]
fn signon_reply_1_writes_the_name_command() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_clmain_set_conn(CA_CONNECTED, 1, 0, 0) }
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_signon_reply(side) },
    );
    assert_same(&c, &rust, "CL_SignonReply(1)");
    assert!(
        !c.message.is_empty(),
        "signon 1 must emit clc_stringcmd + a name command"
    );
    let text = String::from_utf8_lossy(&c.message).into_owned();
    assert!(
        text.contains("name \"player\""),
        "unexpected bytes: {text:?}"
    );
}

#[test]
fn signon_reply_2_writes_color_and_spawn() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_clmain_set_conn(CA_CONNECTED, 2, 0, 0) }
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_signon_reply(side) },
    );
    assert_same(&c, &rust, "CL_SignonReply(2)");
    let text = String::from_utf8_lossy(&c.message).into_owned();
    assert!(text.contains("color 0 0"), "unexpected bytes: {text:?}");
    assert!(text.contains("spawn"), "unexpected bytes: {text:?}");
}

#[test]
fn signon_reply_3_writes_begin() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_clmain_set_conn(CA_CONNECTED, 3, 0, 0) }
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_signon_reply(side) },
    );
    assert_same(&c, &rust, "CL_SignonReply(3)");
    let text = String::from_utf8_lossy(&c.message).into_owned();
    assert!(text.contains("begin"), "unexpected bytes: {text:?}");
}

#[test]
fn signon_reply_overflow_raises_identically() {
    let _g = lock();
    // A 3-byte cls.message makes SZ_GetSpace Host_Error mid-batch. On the
    // Rust side that travels trampoline -> Host_Guard -> status ->
    // Host_Reraise, which is the whole ADR-009 round trip.
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_clmain_set_conn(CA_CONNECTED, 1, 0, 0);
                ctest_clmain_set_message_maxsize(3);
            }
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_signon_reply(side) },
    );
    assert_same(&c, &rust, "CL_SignonReply(overflow)");
    assert_ne!(
        c.guard, GUARD_OK,
        "a 3-byte buffer must raise, not silently truncate"
    );
}

#[test]
fn signon_reply_2_enumerates_the_userinfo() {
    let _g = lock();
    // cl_main.c:277 -- with a non-empty cl.serverinfo, CL_SignonReply calls
    // Info_Enumerate over cls.userinfo. common.c's info layer is not an oracle
    // source, so stubs.c:7633 makes Info_Enumerate a Sys_Error abort: reaching
    // it IS the observation. What that pins down is the write-batch ordering
    // required by ADR-009 -- the `color` command the port buffers must already
    // have been flushed into cls.message when the abort fires, exactly as the
    // oracle's unbuffered MSG_Write* calls leave it.
    let serverinfo = CString::new("\\sv\\1").unwrap();
    let userinfo = CString::new("\\name\\bob\\*spectator\\1\\team\\red").unwrap();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_clmain_attach_message();
                ctest_clmain_set_conn(CA_CONNECTED, 2, 0, 0);
                ctest_clmain_set_serverinfo(serverinfo.as_ptr());
                ctest_clmain_set_userinfo(userinfo.as_ptr());
            }
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_signon_reply(side) },
    );
    assert_same(&c, &rust, "CL_SignonReply(2, userinfo)");
    assert_eq!(c.guard, GUARD_SYS_ERROR, "{}", c.msg);
    assert!(
        c.msg.contains("Info_Enumerate"),
        "unexpected abort: {}",
        c.msg
    );
    let text = String::from_utf8_lossy(&c.message).into_owned();
    assert!(
        text.contains("color 0 0"),
        "the color command must be flushed before the enumeration: {text:?}"
    );
    assert!(
        !text.contains("spawn"),
        "the spawn command comes after the enumeration: {text:?}"
    );
    // SAFETY: read-back over static storage.
    let echoed = unsafe { opt_cstr(ctest_clmain_get_userinfo(0)) }.unwrap_or_default();
    assert_eq!(
        echoed, "\\name\\bob\\*spectator\\1\\team\\red",
        "CL_SignonReply must not rewrite cls.userinfo"
    );
}

#[test]
fn signon_reply_2_sends_no_userinfo_without_serverinfo() {
    let _g = lock();
    // The same seed with an EMPTY cl.serverinfo takes the other branch. Without
    // this pair the test above would pass even if the `if (*cl.serverinfo)`
    // guard were dropped entirely.
    let userinfo = CString::new("\\name\\bob\\team\\red").unwrap();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_clmain_attach_message();
                ctest_clmain_set_conn(CA_CONNECTED, 2, 0, 0);
                ctest_clmain_set_userinfo(userinfo.as_ptr());
            }
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_signon_reply(side) },
    );
    assert_same(&c, &rust, "CL_SignonReply(2, no serverinfo)");
    let text = String::from_utf8_lossy(&c.message).into_owned();
    assert!(
        !text.contains("setinfo"),
        "an empty cl.serverinfo must suppress the enumeration: {text:?}"
    );
}

#[test]
fn send_cmd_while_paused_matches() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_clmain_set_conn(CA_CONNECTED, 4, 0, 0);
                ctest_clmain_set_paused(1, 1);
            }
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_send_cmd(side) },
    );
    assert_same(&c, &rust, "CL_SendCmd(paused)");
}

#[test]
fn relink_entities_interpolates_the_player_velocity() {
    let _g = lock();
    // cl_main.c:679 interpolates cl.velocity from the two cl.mvelocity
    // updates. Nothing else in the fixture writes mvelocity, so without this
    // seeder the interpolation compares 0 to 0. SCR_UpdateZoom (cl_main.c:681)
    // and R_UpdateEntityDlights (:935) are counting stubs rather than abort
    // stubs, so the whole of CL_RelinkEntities runs on both sides here.
    let v0: [c_float; 3] = [100.0, -40.0, 8.0];
    let v1: [c_float; 3] = [0.0, 60.0, -8.0];
    let mut snaps: Vec<(Snap, [u32; 3])> = Vec::with_capacity(2);
    for side in SIDES {
        // SAFETY: fixture seeding; guarded by TEST_LOCK.
        unsafe {
            ctest_clmain_reset();
            ctest_clear_con_log();
            ctest_clmain_set_conn(CA_CONNECTED, 4, 0, 0);
            ctest_clmain_set_counts(4, 1, 2);
            // frac = (1.52 - 1.5) / 0.05 = 0.4
            ctest_clmain_set_time(1.52, 1.5, 1.55, 1.5);
            ctest_clmain_set_mvelocity(v0.as_ptr(), v1.as_ptr());
            ctest_scr_zoom_reset();
            ctest_entity_dlights_reset();
        }
        // SAFETY: fixture driver.
        let g = unsafe { ctest_clmain_relink_entities(side) };
        snaps.push((snap(side, g), velocity(side)));
    }
    let rust = snaps.pop().unwrap();
    let c = snaps.pop().unwrap();
    assert_same(&c.0, &rust.0, "CL_RelinkEntities(velocity)");
    assert_eq!(c.1, rust.1, "cl.velocity differs bit-for-bit");
    assert_eq!(
        (c.0.guard, rust.0.guard),
        (GUARD_OK, GUARD_OK),
        "CL_RelinkEntities should run to completion on both sides"
    );
    // Both sides share the one SCR_UpdateZoom/R_UpdateEntityDlights definition,
    // so these counts attest that the last side to run -- the port -- reached
    // cl_main.c:681 and then ran past the entity loop to :935.
    // SAFETY: reading counting stubs; guarded by TEST_LOCK.
    unsafe {
        assert_eq!(ctest_scr_zoom_count(), 1, "SCR_UpdateZoom was not reached");
        assert_eq!(
            ctest_entity_dlights_count(),
            1,
            "execution did not run past the entity loop"
        );
    }
    // v1 + 0.4 * (v0 - v1), per axis, computed in f32 exactly as the C does.
    let want = [
        v1[0] + 0.4f32 * (v0[0] - v1[0]),
        v1[1] + 0.4f32 * (v0[1] - v1[1]),
        v1[2] + 0.4f32 * (v0[2] - v1[2]),
    ]
    .map(f32::to_bits);
    assert_eq!(c.1, want, "cl.velocity was not interpolated at frac = 0.4");
}

#[test]
fn relink_entities_clamps_frametime_before_the_particle_trail() {
    let _g = lock();
    // cl_main.c:670 clamps cl.time - cl.oldtime to 0.1 before passing it as the
    // trail time interval. The clamp is only observable through that argument,
    // so this seeds a 0.5s gap -- well over the clamp -- and reads the value
    // PScript_ParticleTrail actually received.
    let mut seen: Vec<(Snap, c_int, u32)> = Vec::with_capacity(2);
    for side in SIDES {
        // SAFETY: fixture seeding; guarded by TEST_LOCK.
        unsafe {
            ctest_clmain_reset();
            ctest_clear_con_log();
            ctest_pscript_reset();
            ctest_clmain_attach_arrays(2);
            ctest_clmain_set_entity(1, 0, 0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
            let o: [c_float; 3] = [1.0, 2.0, 3.0];
            ctest_clmain_set_entity_msg(1, 2.0, o.as_ptr(), o.as_ptr(), o.as_ptr(), o.as_ptr());
            ctest_clmain_set_conn(CA_CONNECTED, 4, 0, 0);
            ctest_clmain_set_counts(4, 1, 2);
            ctest_clmain_set_time(2.0, 1.5, 2.0, 1.95);
        }
        // SAFETY: fixture driver.
        let g = unsafe { ctest_clmain_relink_entities(side) };
        // SAFETY: reading recording stubs; guarded by TEST_LOCK.
        let (n, iv) = unsafe {
            (
                ctest_pscript_trail_count(),
                ctest_pscript_last_timeinterval_value().to_bits(),
            )
        };
        seen.push((snap(side, g), n, iv));
    }
    let rust = seen.pop().unwrap();
    let c = seen.pop().unwrap();
    assert_same(&c.0, &rust.0, "CL_RelinkEntities(frametime clamp)");
    assert_eq!(
        (c.0.guard, rust.0.guard),
        (GUARD_OK, GUARD_OK),
        "CL_RelinkEntities should run to completion on both sides"
    );
    assert_eq!(c.1, rust.1, "particle trail call count differs");
    assert_eq!(c.1, 1, "the entity did not reach the particle trail branch");
    assert_eq!(c.2, rust.2, "trail time interval differs bit-for-bit");
    assert_eq!(
        c.2,
        0.1f32.to_bits(),
        "a 0.5s frame was not clamped to 0.1 before the trail call"
    );
}

#[test]
fn relink_entities_lerps_below_the_teleport_threshold() {
    relink_teleport_case([40.0, 0.0, 0.0], "below");
}

#[test]
fn relink_entities_snaps_above_the_teleport_threshold() {
    relink_teleport_case([140.0, 0.0, 0.0], "above");
}

/// CL_LerpEntity (cl_main.c:484) sets f = 1.0 for the whole entity as soon as
/// any axis of msg_origins[0] - msg_origins[1] exceeds +/-100, so the two
/// cases above straddle that threshold on one axis and differ only in whether
/// the origin lands at the interpolated point or snaps to msg_origins[0].
/// Without both, moving the constant is unobservable.
fn relink_teleport_case(delta: [c_float; 3], label: &str) {
    let _g = lock();
    let o1: [c_float; 3] = [10.0, 20.0, 30.0];
    let o0 = [o1[0] + delta[0], o1[1] + delta[1], o1[2] + delta[2]];
    let a0: [c_float; 3] = [0.0, 30.0, 0.0];
    let a1: [c_float; 3] = [0.0, 10.0, 0.0];
    let mut snaps: Vec<(Snap, Vec<u8>)> = Vec::with_capacity(2);
    for side in SIDES {
        // SAFETY: fixture seeding; guarded by TEST_LOCK.
        unsafe {
            ctest_clmain_reset();
            ctest_clear_con_log();
            ctest_clmain_attach_arrays(2);
            ctest_clmain_set_entity(0, 0, 0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
            ctest_clmain_set_entity(1, 0, 0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
            // msgtime must equal cl.mtime[0] or cl_main.c:664 drops the entity.
            ctest_clmain_set_entity_msg(
                1,
                1.55,
                o0.as_ptr(),
                o1.as_ptr(),
                a0.as_ptr(),
                a1.as_ptr(),
            );
            ctest_clmain_set_conn(CA_CONNECTED, 4, 0, 0);
            ctest_clmain_set_counts(4, 1, 2);
            // frac = (1.52 - 1.5) / 0.05 = 0.4
            ctest_clmain_set_time(1.52, 1.5, 1.55, 1.5);
        }
        // SAFETY: fixture driver.
        let g = unsafe { ctest_clmain_relink_entities(side) };
        snaps.push((snap(side, g), entities(side, 2)));
    }
    let rust = snaps.pop().unwrap();
    let c = snaps.pop().unwrap();
    assert_same(
        &c.0,
        &rust.0,
        &format!("CL_RelinkEntities(teleport {label})"),
    );
    assert_eq!(
        (c.0.guard, rust.0.guard),
        (GUARD_OK, GUARD_OK),
        "CL_RelinkEntities should run to completion on both sides"
    );
    assert_eq!(c.1, rust.1, "entity state differs bit-for-bit ({label})");
}

#[test]
fn relink_entities_over_a_short_entity_array_matches() {
    let _g = lock();
    // cl.max_edicts smaller than cl.num_entities is the clamp arm; both sides
    // must stop at the same entity.
    let mut snaps: Vec<(Snap, Vec<u8>)> = Vec::with_capacity(2);
    for side in SIDES {
        // SAFETY: fixture seeding; guarded by TEST_LOCK.
        unsafe {
            ctest_clmain_reset();
            ctest_clear_con_log();
            ctest_clmain_attach_arrays(2);
            ctest_clmain_set_entity(0, 0, 0, 1.0, 2.0, 3.0, 0.0, 0.0, 0.0);
            ctest_clmain_set_entity(1, 1, 1, 4.0, 5.0, 6.0, 0.0, 90.0, 0.0);
            ctest_clmain_set_conn(CA_CONNECTED, 4, 0, 0);
            ctest_clmain_set_counts(4, 1, 2);
            ctest_clmain_set_time(1.52, 1.5, 1.55, 1.5);
        }
        // SAFETY: fixture driver.
        let g = unsafe { ctest_clmain_relink_entities(side) };
        snaps.push((snap(side, g), entities(side, 2)));
    }
    let rust = snaps.pop().unwrap();
    let c = snaps.pop().unwrap();
    assert_same(&c.0, &rust.0, "CL_RelinkEntities(short array)");
    assert_eq!(c.1, rust.1, "entity array differs");
}

// ---------------------------------------------------------------------------
// CL_NextDemo (cl_main.c:301-331)

#[test]
fn next_demo_with_an_empty_loop_disconnects() {
    let _g = lock();
    let prefix = CString::new("d").unwrap();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_clmain_set_demoloop(0, 0, prefix.as_ptr()) }
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_next_demo(side) },
    );
    assert_same(&c, &rust, "CL_NextDemo(empty)");
    assert!(
        c.con.iter().any(|l| l.contains("No demos listed")),
        "expected the empty-loop message: {:?}",
        c.con
    );
}

#[test]
fn next_demo_returns_immediately_when_the_loop_is_off() {
    let _g = lock();
    let prefix = CString::new("d").unwrap();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_clmain_set_demoloop(-1, 3, prefix.as_ptr()) }
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_next_demo(side) },
    );
    assert_same(&c, &rust, "CL_NextDemo(demonum -1)");
    assert_eq!(c.guard, GUARD_OK);
    assert!(c.con.is_empty(), "nothing may be printed: {:?}", c.con);
}

#[test]
fn next_demo_advances_the_loop_counter() {
    let _g = lock();
    let prefix = CString::new("dm").unwrap();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_clmain_set_demoloop(1, 3, prefix.as_ptr()) }
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_next_demo(side) },
    );
    assert_same(&c, &rust, "CL_NextDemo(advance)");
    // Positive assertion: with a populated loop the function walks past the
    // "No demos listed" branch and into SCR_BeginLoadingPlaque, which is a
    // stubs.c abort stub (gl_screen.c is not an oracle source). Reaching it is
    // what distinguishes this arm from the empty-loop one, which prints and
    // returns without aborting.
    assert_eq!(c.guard, GUARD_SYS_ERROR, "{}", c.msg);
    assert!(
        c.msg.contains("SCR_BeginLoadingPlaque"),
        "unexpected abort: {}",
        c.msg
    );
    assert!(
        !c.con.iter().any(|l| l.contains("No demos listed")),
        "a populated loop must not take the empty branch: {:?}",
        c.con
    );
}

// ---------------------------------------------------------------------------
// CL_ClearState / CL_FreeState / CL_ClearTrailStates / CL_Disconnect
//
// Each of these reaches a shared abort stub; the comparison is of which stub,
// with what message, and of everything mutated before it.

#[test]
fn clear_state_matches_including_the_abort_it_reaches() {
    let _g = lock();
    let (c, rust) = run_both(
        || {},
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_clear_state(side) },
    );
    assert_same(&c, &rust, "CL_ClearState");
}

#[test]
fn free_state_matches_including_the_abort_it_reaches() {
    let _g = lock();
    let (c, rust) = run_both(
        || {},
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_free_state(side) },
    );
    assert_same(&c, &rust, "CL_FreeState");
}

#[test]
fn clear_trail_states_matches() {
    let _g = lock();
    let (c, rust) = run_both(
        || {},
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_clear_trail_states(side) },
    );
    assert_same(&c, &rust, "CL_ClearTrailStates");
}

#[test]
fn disconnect_from_a_disconnected_client_matches() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_clmain_set_conn(CA_DISCONNECTED, 0, 0, 0) }
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_disconnect(side) },
    );
    assert_same(&c, &rust, "CL_Disconnect(disconnected)");
}

#[test]
fn disconnect_f_matches() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_clmain_set_conn(CA_DISCONNECTED, 0, 0, 0) }
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_disconnect_f(side) },
    );
    assert_same(&c, &rust, "CL_Disconnect_f");
}

#[test]
fn disconnect_while_dedicated_matches() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_clmain_set_conn(CA_DEDICATED, 0, 0, 0) }
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_disconnect(side) },
    );
    assert_same(&c, &rust, "CL_Disconnect(dedicated)");
}

// ---------------------------------------------------------------------------
// CL_RelinkEntities (cl_main.c:600-960)

#[test]
fn relink_entities_matches_entity_for_entity() {
    let _g = lock();
    let mut snaps: Vec<(Snap, Vec<u8>)> = Vec::with_capacity(2);
    for side in SIDES {
        // SAFETY: fixture seeding; guarded by TEST_LOCK.
        unsafe {
            ctest_clmain_reset();
            ctest_clear_con_log();
            ctest_clmain_set_conn(CA_CONNECTED, 4, 0, 0);
            ctest_clmain_set_counts(4, 1, 6);
            ctest_clmain_set_time(1.52, 1.5, 1.55, 1.5);
        }
        // SAFETY: fixture driver.
        let g = unsafe { ctest_clmain_relink_entities(side) };
        snaps.push((snap(side, g), entities(side, 8)));
    }
    let rust = snaps.pop().unwrap();
    let c = snaps.pop().unwrap();
    assert_same(&c.0, &rust.0, "CL_RelinkEntities");
    assert_eq!(c.1, rust.1, "entity array differs after CL_RelinkEntities");
}

#[test]
fn relink_entities_with_no_entities_matches() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_clmain_set_conn(CA_CONNECTED, 4, 0, 0);
                ctest_clmain_set_counts(4, 1, 1);
            }
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_relink_entities(side) },
    );
    assert_same(&c, &rust, "CL_RelinkEntities(empty)");
}

// ---------------------------------------------------------------------------
// CL_ReadFromServer / CL_SendCmd / CL_AccumulateCmd

#[test]
fn read_from_server_matches() {
    let _g = lock();
    let mut res: Vec<(Snap, c_int)> = Vec::with_capacity(2);
    for side in SIDES {
        // SAFETY: fixture seeding; guarded by TEST_LOCK.
        unsafe {
            ctest_clmain_reset();
            ctest_clear_con_log();
            ctest_clmain_set_conn(CA_CONNECTED, 4, 0, 0);
        }
        let mut out: c_int = -9;
        // SAFETY: fixture driver.
        let g = unsafe { ctest_clmain_read_from_server(side, &mut out) };
        res.push((snap(side, g), out));
    }
    let rust = res.pop().unwrap();
    let c = res.pop().unwrap();
    assert_same(&c.0, &rust.0, "CL_ReadFromServer");
    assert_eq!(c.1, rust.1, "CL_ReadFromServer return value differs");
}

#[test]
fn send_cmd_matches() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_clmain_set_conn(CA_CONNECTED, 4, 0, 0) }
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_send_cmd(side) },
    );
    assert_same(&c, &rust, "CL_SendCmd");
}

#[test]
fn send_cmd_while_disconnected_matches() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_clmain_set_conn(CA_DISCONNECTED, 0, 0, 0) }
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_send_cmd(side) },
    );
    assert_same(&c, &rust, "CL_SendCmd(disconnected)");
    assert_eq!(c.guard, GUARD_OK, "the disconnected arm returns early");
}

#[test]
fn accumulate_cmd_matches() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_clmain_set_conn(CA_CONNECTED, 4, 0, 0) }
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_accumulate_cmd(side) },
    );
    assert_same(&c, &rust, "CL_AccumulateCmd");
}

// ---------------------------------------------------------------------------
// CL_Tracepos_f / CL_Viewpos_f (cl_main.c:1120-1170)

#[test]
fn tracepos_matches() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_clmain_set_conn(CA_CONNECTED, 4, 0, 0);
                // Inside the synthetic room, aimed straight down +X at its
                // wall, so the 8192-unit ray really is clipped by a plane.
                let org = [0.0f32, 0.0, 0.0];
                let fwd = [1.0f32, 0.0, 0.0];
                ctest_clmain_attach_world(org.as_ptr(), fwd.as_ptr());
            }
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_tracepos(side) },
    );
    assert_same(&c, &rust, "CL_Tracepos_f");
    let line = c
        .con
        .iter()
        .find(|l| l.contains("Tracepos"))
        .unwrap_or_else(|| panic!("expected a Tracepos line: {:?}", c.con))
        .clone();
    // Positive assertion: the ray was clipped, so the reported impact is
    // neither the view origin nor the unclipped 8192-unit endpoint.
    assert!(
        !line.contains("(0 0 0)") && !line.contains("8192"),
        "the trace degenerated: {line:?}"
    );
}

#[test]
fn tracepos_requires_a_connected_client() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_clmain_set_conn(CA_DISCONNECTED, 0, 0, 0);
                let org = [0.0f32, 0.0, 0.0];
                let fwd = [1.0f32, 0.0, 0.0];
                ctest_clmain_attach_world(org.as_ptr(), fwd.as_ptr());
            }
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_tracepos(side) },
    );
    assert_same(&c, &rust, "CL_Tracepos_f(disconnected)");
    assert!(c.con.is_empty(), "nothing may be printed: {:?}", c.con);
}

#[test]
fn viewpos_prints_the_players_position() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe {
                ctest_clmain_set_conn(CA_CONNECTED, 4, 0, 0);
                ctest_clmain_set_counts(4, 5, 8);
                ctest_clmain_set_entity(5, 0, 0, 12.0, -34.0, 56.0, 0.0, 0.0, 0.0);
            }
            tokenize("viewpos");
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_viewpos(side) },
    );
    assert_same(&c, &rust, "CL_Viewpos_f");
    assert!(
        c.con.iter().any(|l| l.contains("Viewpos: (12 -34 56)")),
        "expected the seeded position: {:?}",
        c.con
    );
}

#[test]
fn viewpos_copy_reaches_the_clipboard_shim() {
    let _g = lock();
    // `SDL_SetClipboardText` is not linkable here, so ClMain_Glue_
    // SetClipboardText records the string instead (see cl_main_ref.c). Only
    // the Rust side routes through that shim -- the oracle calls SDL directly
    // and so is NOT driven for this arm. What is asserted is that the port
    // computes the same buffer text `CL_Viewpos_f` prints.
    // SAFETY: fixture seeding and driver; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clear_con_log();
        ctest_clmain_set_conn(CA_CONNECTED, 4, 0, 0);
        ctest_clmain_set_counts(4, 5, 8);
        ctest_clmain_set_entity(5, 0, 0, 7.0, 8.0, 9.0, 0.0, 0.0, 0.0);
    }
    tokenize("viewpos copy");
    // SAFETY: fixture driver.
    let g = unsafe { ctest_clmain_viewpos(0) };
    assert_eq!(g, GUARD_OK, "{}", guard_message(g));
    // SAFETY: read-back over static storage.
    let clip = unsafe { opt_cstr(ctest_clmain_clipboard()) }.unwrap_or_default();
    assert_eq!(clip, "(7 8 9) 0 0 0", "clipboard text: {clip:?}");
    assert!(
        con_log().iter().any(|l| l.contains("Viewpos: (7 8 9)")),
        "console and clipboard must agree: {:?}",
        con_log()
    );
}

#[test]
fn viewpos_requires_a_connected_client() {
    let _g = lock();
    let (c, rust) = run_both(
        || {
            // SAFETY: fixture seeding; guarded by TEST_LOCK.
            unsafe { ctest_clmain_set_conn(CA_DISCONNECTED, 0, 0, 0) }
        },
        // SAFETY: fixture driver.
        |side| unsafe { ctest_clmain_viewpos(side) },
    );
    assert_same(&c, &rust, "CL_Viewpos_f(disconnected)");
    assert!(c.con.is_empty(), "nothing may be printed: {:?}", c.con);
}

// ---------------------------------------------------------------------------
// The seven `static` handlers. NO oracle twin exists (see the module doc), so
// these are Rust-only characterization tests against values read off
// cl_main.c, not differentials.

#[test]
fn rust_only_legacy_color_splits_the_packed_byte() {
    let _g = lock();
    // cl_main.c:1350 -- `_cl_color 100` is 0x64, so topcolor 6 / bottomcolor 4.
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clear_con_log();
        let r = ctest_clmain_register_color_cvars();
        assert_eq!(
            r,
            GUARD_OK,
            "registering the colour cvars: {}",
            guard_message(r)
        );
    }
    tokenize("_cl_color 100");
    // SAFETY: fixture driver.
    let g = unsafe { ctest_clmain_rust_legacy_color() };
    assert_eq!(g, GUARD_OK, "{}", guard_message(g));
    // SAFETY: read-backs over static storage.
    let (top, bottom) = unsafe {
        (
            ctest_clmain_get_color_cvar(0),
            ctest_clmain_get_color_cvar(1),
        )
    };
    assert_eq!(top, 6.0, "topcolor from 0x64");
    assert_eq!(bottom, 4.0, "bottomcolor from 0x64");
}

#[test]
fn rust_only_legacy_color_masks_the_top_nibble_too() {
    let _g = lock();
    // 400 is 0x190. The top colour comes from `(col >> 4) & 0xf` = 0x19 & 0xf
    // = 9; a wider mask there would give 25. `_cl_color 255` only pins the
    // BOTTOM mask, because 0xff >> 4 is already four bits wide.
    //
    // 9 rather than 15 on purpose: ctest_clmain_reset pokes cvar_t.value
    // directly without touching cvar_t.string, so a Cvar_SetValue to the value
    // the PREVIOUS test left in the string is skipped as a no-op and the
    // poked 0 survives. Each of these three cases must therefore ask for a
    // different colour than its neighbours.
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clear_con_log();
        let r = ctest_clmain_register_color_cvars();
        assert_eq!(
            r,
            GUARD_OK,
            "registering the colour cvars: {}",
            guard_message(r)
        );
    }
    tokenize("_cl_color 400");
    // SAFETY: fixture driver.
    let g = unsafe { ctest_clmain_rust_legacy_color() };
    assert_eq!(g, GUARD_OK, "{}", guard_message(g));
    // SAFETY: read-backs over static storage.
    let (top, bottom) = unsafe {
        (
            ctest_clmain_get_color_cvar(0),
            ctest_clmain_get_color_cvar(1),
        )
    };
    assert_eq!(top, 9.0, "the top nibble must be masked to four bits");
    assert_eq!(bottom, 0.0, "bottomcolor from 0x190");
}

#[test]
fn rust_only_legacy_color_masks_to_four_bits() {
    let _g = lock();
    // 255 is 0xff, so both nibbles are 15; a missing `& 0xf` would leave the
    // bottom colour at 255.
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clear_con_log();
        let r = ctest_clmain_register_color_cvars();
        assert_eq!(
            r,
            GUARD_OK,
            "registering the colour cvars: {}",
            guard_message(r)
        );
    }
    tokenize("_cl_color 255");
    // SAFETY: fixture driver.
    let g = unsafe { ctest_clmain_rust_legacy_color() };
    assert_eq!(g, GUARD_OK, "{}", guard_message(g));
    // SAFETY: read-backs over static storage.
    let (top, bottom) = unsafe {
        (
            ctest_clmain_get_color_cvar(0),
            ctest_clmain_get_color_cvar(1),
        )
    };
    assert_eq!((top, bottom), (15.0, 15.0));
}

#[test]
fn rust_only_serverext_full_serverinfo_replaces_cl_serverinfo() {
    let _g = lock();
    let old = CString::new("\\stale\\1").unwrap();
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clear_con_log();
        ctest_clmain_set_serverinfo(old.as_ptr());
    }
    tokenize("fullserverinfo \\maxclients\\8\\deathmatch\\1");
    // SAFETY: fixture driver.
    let g = unsafe { ctest_clmain_rust_serverext(0) };
    assert_eq!(g, GUARD_OK, "{}", guard_message(g));
    // SAFETY: read-back over static storage.
    let info = unsafe { opt_cstr(ctest_clmain_get_serverinfo(0)) }.unwrap_or_default();
    assert_eq!(
        info, "\\maxclients\\8\\deathmatch\\1",
        "cl_main.c:1185 replaces cl.serverinfo wholesale"
    );
}

#[test]
fn rust_only_serverext_serverinfo_update_sets_one_key() {
    let _g = lock();
    let info = CString::new("\\a\\1").unwrap();
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clear_con_log();
        ctest_clmain_set_serverinfo(info.as_ptr());
    }
    tokenize("svi b 2");
    // SAFETY: fixture driver.
    let g = unsafe { ctest_clmain_rust_serverext(1) };
    assert_eq!(g, GUARD_OK, "{}", guard_message(g));
    // SAFETY: read-back over static storage.
    let out = unsafe { opt_cstr(ctest_clmain_get_serverinfo(0)) }.unwrap_or_default();
    assert!(out.contains("\\a\\1"), "existing key lost: {out:?}");
    assert!(out.contains("\\b\\2"), "new key not set: {out:?}");
}

#[test]
fn rust_only_serverext_full_userinfo_writes_the_scoreboard_slot() {
    let _g = lock();
    // cl_main.c:1214 writes cl.scores[slot].userinfo, then calls
    // CL_UserinfoChanged, whose first statement is Info_GetKey -- a stubs.c
    // abort stub (common.c's info layer is not an oracle source). The write
    // therefore has to be observable BEFORE the abort, which is what pins the
    // ordering of the two statements.
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clear_con_log();
        ctest_clmain_set_counts(4, 1, 8);
    }
    tokenize("fullinfo 2 \\name\\tester");
    // SAFETY: fixture driver.
    let g = unsafe { ctest_clmain_rust_serverext(2) };
    assert_eq!(g, GUARD_SYS_ERROR, "expected the Info_GetKey abort stub");
    assert!(
        guard_message(g).contains("Info_GetKey"),
        "unexpected abort: {}",
        guard_message(g)
    );
    // SAFETY: read-backs over static storage.
    let (ui, name, colors) = unsafe {
        (
            opt_cstr(ctest_clmain_get_score_userinfo(0, 2)).unwrap_or_default(),
            opt_cstr(ctest_clmain_get_score_name(0, 2)).unwrap_or_default(),
            ctest_clmain_get_score_colors(0, 2),
        )
    };
    assert_eq!(ui, "\\name\\tester", "scoreboard userinfo");
    assert_eq!(name, "", "the abort fires before sb->name is filled in");
    assert_eq!(colors, 0, "and before sb->colors is recomputed");
}

#[test]
fn rust_only_serverext_full_userinfo_ignores_an_out_of_range_slot() {
    let _g = lock();
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clear_con_log();
        ctest_clmain_set_counts(4, 1, 8);
    }
    tokenize("fullinfo 9 \\name\\nope");
    // SAFETY: fixture driver.
    let g = unsafe { ctest_clmain_rust_serverext(2) };
    assert_eq!(g, GUARD_OK, "{}", guard_message(g));
    // SAFETY: read-back over static storage.
    let ui = unsafe { opt_cstr(ctest_clmain_get_score_userinfo(0, 4)) }.unwrap_or_default();
    assert_eq!(ui, "", "slot 9 >= cl.maxclients must be dropped");
}

#[test]
fn rust_only_serverext_userinfo_update_sets_one_key() {
    let _g = lock();
    // Seed the slot through the fullinfo handler first (it aborts in
    // CL_UserinfoChanged, but only AFTER the write, so the string is there),
    // then merge a second key in through the update arm.
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clear_con_log();
        ctest_clmain_set_counts(4, 1, 8);
    }
    tokenize("fullinfo 1 \\name\\tester");
    // SAFETY: fixture driver.
    assert_eq!(unsafe { ctest_clmain_rust_serverext(2) }, GUARD_SYS_ERROR);
    tokenize("ui 1 team red");
    // SAFETY: fixture driver.
    let g = unsafe { ctest_clmain_rust_serverext(3) };
    assert_eq!(g, GUARD_SYS_ERROR, "expected the Info_GetKey abort stub");
    assert!(
        guard_message(g).contains("Info_GetKey"),
        "unexpected abort: {}",
        guard_message(g)
    );
    // SAFETY: read-back over static storage.
    let ui = unsafe { opt_cstr(ctest_clmain_get_score_userinfo(0, 1)) }.unwrap_or_default();
    // Info_SetKey (stubs.c:1547, a real copy of common.c's) merges rather
    // than replaces, so both keys must be present in the recorded order.
    assert_eq!(
        ui, "\\name\\tester\\team\\red",
        "the update arm must merge into the existing string"
    );
}

#[test]
fn rust_only_serverext_ignore_changes_nothing() {
    let _g = lock();
    let info = CString::new("\\a\\1").unwrap();
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clear_con_log();
        ctest_clmain_set_serverinfo(info.as_ptr());
    }
    // SAFETY: read-back over static storage.
    let mut before = vec![0u8; unsafe { ctest_clmain_cl_image_size() } as usize];
    // SAFETY: read-back over static storage.
    unsafe { ctest_clmain_get_cl_image(0, before.as_mut_ptr()) };
    tokenize("anything at all");
    // SAFETY: fixture driver.
    let g = unsafe { ctest_clmain_rust_serverext(4) };
    assert_eq!(g, GUARD_OK, "{}", guard_message(g));
    // SAFETY: read-back over static storage.
    let mut after = vec![0u8; unsafe { ctest_clmain_cl_image_size() } as usize];
    // SAFETY: read-back over static storage.
    unsafe { ctest_clmain_get_cl_image(0, after.as_mut_ptr()) };
    assert_eq!(before, after, "the ignore handler must change nothing");
    // SAFETY: read-back over static storage.
    let out = unsafe { opt_cstr(ctest_clmain_get_serverinfo(0)) }.unwrap_or_default();
    assert_eq!(out, "\\a\\1");
}

#[test]
fn rust_only_viewpos_completion_returns_early_unless_argc_is_two() {
    let _g = lock();
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clear_con_log();
    }
    tokenize("viewpos");
    let partial = CString::new("").unwrap();
    // SAFETY: fixture driver.
    let g = unsafe { ctest_clmain_rust_viewpos_completion(partial.as_ptr()) };
    assert_eq!(
        g,
        GUARD_OK,
        "argc 1 must return before Con_AddToTabList: {}",
        guard_message(g)
    );
}

#[test]
fn rust_only_viewpos_completion_offers_copy_at_argc_two() {
    let _g = lock();
    // SAFETY: fixture seeding; guarded by TEST_LOCK.
    unsafe {
        ctest_clmain_reset();
        ctest_clear_con_log();
        // Con_AddToTabList appends to the real console port's list now, so
        // start from an empty one.
        ctest_console_reset(0);
    }
    tokenize("viewpos c");
    let partial = CString::new("c").unwrap();
    // SAFETY: fixture driver.
    let g = unsafe { ctest_clmain_rust_viewpos_completion(partial.as_ptr()) };
    assert_eq!(g, GUARD_OK, "{}", guard_message(g));
    // SAFETY: fixture read-back.
    let count = unsafe { ctest_console_tablist_count(0) };
    assert_eq!(
        count, 1,
        "the port must offer exactly one completion at argc 2"
    );
    let mut name = [0u8; 64];
    let mut ty = [0u8; 64];
    // SAFETY: both buffers are live and their capacities are passed alongside.
    unsafe {
        ctest_console_tablist_entry(
            0,
            0,
            name.as_mut_ptr().cast::<c_char>(),
            name.len() as c_int,
            ty.as_mut_ptr().cast::<c_char>(),
            ty.len() as c_int,
        )
    };
    let name = core::ffi::CStr::from_bytes_until_nul(&name)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(name, "copy");
    // SAFETY: hand the list back before the next test in this binary runs.
    unsafe { ctest_console_reset(0) };
}
