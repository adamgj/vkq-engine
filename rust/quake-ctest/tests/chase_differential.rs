//! Differential/characterization gate for `Quake/chase.c` -- the chase
//! (third-person) camera. Rust migration Phase 7, M7, task T7.2a.
//!
//! The oracle side lives in `stubs/chase_ref.c`, which also carries the plain
//! (Rust-routed) half of the link and the fixture. Read that file's module doc
//! for why each of the three comparisons below is genuinely two-sided.
//!
//! `chase.c` has four entry points. Three are covered here:
//!
//!   * `Chase_Init` -- registration into two independent cvar registries
//!     (Quake/cvar.c's list vs quake-capi's), asserted on name, string, flags,
//!     parsed value AND registry membership.
//!   * `TraceLine` -- Quake/world.c's `SV_RecursiveHullCheck` vs quake-capi's,
//!     over the one shared synthetic room from `stubs.c`.
//!   * `Chase_UpdateForDrawing` -- the whole camera solve, including the
//!     `VectorLength(temp) != 0` wall fallbacks, the `1 << 20` crosshair ray
//!     and the `PITCH == +-90` yaw fixup.
//!
//! `Chase_UpdateForClient` has an empty body in `chase.c` (comments only), so
//! there is nothing to compare; `chase_ref.c` still routes it so the ABI
//! matches `Quake/chase_glue.c`.
//!
//! Every float assertion is on `to_bits()`, never an epsilon: the point of the
//! gate is bit-exactness, and an approximate comparison would hide exactly the
//! reassociation/contraction regressions ADR-010 exists to prevent.
//!
//! DEGENERATE-GATE GUARD. `Cvar_RegisterVariable` does not run for these cvars
//! outside `chase_init_matches_oracle`, so `chase_ref.c`'s
//! `ctest_chase_set_cvars` seeds `.value` on BOTH sides from one argument list
//! -- a cvar left at its static initializer would read `0.0` on both halves
//! and `Chase_UpdateForDrawing` would degenerate to a zero-offset camera that
//! agrees while measuring nothing. Likewise `ctest_chase_reset` republishes
//! `cl.worldmodel` on the PLAIN copy of `cl`, which `ctest_world_reset` (in
//! `stubs.c`) sets only on `c_ref_cl`; without it the port would trace through
//! a null worldmodel. Each test group below names the mutation that was run
//! against it to prove it is not vacuous.

use core::ffi::{c_char, c_float, c_int, c_uint};
use std::ffi::CStr;
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

extern "C" {
    // Fixture (stubs/chase_ref.c).
    fn ctest_chase_reset();
    fn ctest_chase_set_cvars(back: c_float, up: c_float, right: c_float, active: c_float);
    fn ctest_chase_set_client(viewangles: *const c_float, viewent_origin: *const c_float);
    fn ctest_chase_set_refdef(vieworg: *const c_float, viewangles: *const c_float);
    fn ctest_chase_get_refdef(out6: *mut c_float);
    fn ctest_chase_cvar_count() -> c_int;
    fn ctest_chase_cvar_found(idx: c_int, oracle: c_int) -> c_int;
    fn ctest_chase_cvar_value(idx: c_int, oracle: c_int) -> c_float;
    fn ctest_chase_cvar_flags(idx: c_int, oracle: c_int) -> c_uint;
    fn ctest_chase_cvar_name(idx: c_int, oracle: c_int) -> *const c_char;
    fn ctest_chase_cvar_string(idx: c_int, oracle: c_int) -> *const c_char;

    // Oracle entry points (Quake/chase.c through c_ref_prelude.h's rename).
    fn c_ref_Chase_Init();
    fn c_ref_TraceLine(start: *mut c_float, end: *mut c_float, impact: *mut c_float);
    fn c_ref_Chase_UpdateForDrawing();

    // Plain entry points (stubs/chase_ref.c -> rust/quake-capi/src/chase.rs).
    fn Chase_Init();
    fn TraceLine(start: *mut c_float, end: *mut c_float, impact: *mut c_float);
    fn Chase_UpdateForDrawing();
}

/// Bit-exact vector comparison. `f32::to_bits` distinguishes `-0.0` from `0.0`
/// and every NaN payload, which `==` would not.
#[track_caller]
fn assert_bits3(label: &str, got: &[c_float; 3], want: &[c_float; 3]) {
    for i in 0..3 {
        assert_eq!(
            got[i].to_bits(),
            want[i].to_bits(),
            "{label}[{i}]: rust {:?} (0x{:08x}) != oracle {:?} (0x{:08x})",
            got[i],
            got[i].to_bits(),
            want[i],
            want[i].to_bits(),
        );
    }
}

#[track_caller]
fn assert_bits6(label: &str, got: &[c_float; 6], want: &[c_float; 6]) {
    for i in 0..6 {
        assert_eq!(
            got[i].to_bits(),
            want[i].to_bits(),
            "{label}[{i}]: rust {:?} (0x{:08x}) != oracle {:?} (0x{:08x})",
            got[i],
            got[i].to_bits(),
            want[i],
            want[i].to_bits(),
        );
    }
}

fn cstr(p: *const c_char) -> String {
    assert!(!p.is_null());
    // SAFETY: null-checked above; the fixture only ever returns pointers to
    // NUL-terminated static or cvar_t-owned strings.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

// ---------------------------------------------------------------------------
// Group 1: Chase_Init.
//
// Two registries, two parsers, two `cvar_t` object sets. `ctest_m7_linkproof`
// (stubs.c:7655) already proves `c_ref_Cvar_RegisterVariable` really parses
// `.value` in this link, so a match here is a real agreement and not two
// no-ops.
//
// MUTATION USED: changed `chase_up`'s plain initializer in `stubs/chase_ref.c`
// from "16" to "17" -- `chase_init_matches_oracle` failed at the `cvar 1
// string` assertion. Restored.
//
// The registration order (back, up, right, active) is preserved by
// `quake_rs_chase_init` but is NOT observable: `Quake/cvar.c:663-679` inserts
// alphabetically and none of the four is `CVAR_SERVERINFO`, so the test
// compares per-variable state rather than list position.

#[test]
fn chase_init_matches_oracle() {
    let _g = lock();

    // SAFETY: single-threaded under TEST_LOCK; every call below is a fixture
    // entry point in stubs/chase_ref.c taking no borrowed state.
    unsafe {
        let n = ctest_chase_cvar_count();
        assert_eq!(n, 4);

        // Nothing is registered yet in either registry.
        for i in 0..n {
            assert_eq!(
                ctest_chase_cvar_found(i, 0),
                0,
                "plain cvar {i} pre-registered"
            );
            assert_eq!(
                ctest_chase_cvar_found(i, 1),
                0,
                "oracle cvar {i} pre-registered"
            );
        }

        Chase_Init(); // -> quake_rs_chase_init -> Chase_Glue_RegisterVariable
        c_ref_Chase_Init();

        for i in 0..n {
            assert_eq!(
                ctest_chase_cvar_found(i, 0),
                1,
                "port did not register cvar {i}"
            );
            assert_eq!(
                ctest_chase_cvar_found(i, 1),
                1,
                "oracle did not register cvar {i}"
            );

            assert_eq!(
                cstr(ctest_chase_cvar_name(i, 0)),
                cstr(ctest_chase_cvar_name(i, 1)),
                "cvar {i} name",
            );
            assert_eq!(
                cstr(ctest_chase_cvar_string(i, 0)),
                cstr(ctest_chase_cvar_string(i, 1)),
                "cvar {i} string",
            );
            assert_eq!(
                ctest_chase_cvar_flags(i, 0),
                ctest_chase_cvar_flags(i, 1),
                "cvar {i} flags",
            );
            assert_eq!(
                ctest_chase_cvar_value(i, 0).to_bits(),
                ctest_chase_cvar_value(i, 1).to_bits(),
                "cvar {i} value",
            );
        }

        // Not vacuous: registration must actually have parsed the strings.
        // chase_back "100" and chase_up "16" are the two non-zero defaults, so
        // a registry that silently did nothing would leave both at 0.0.
        assert_eq!(
            ctest_chase_cvar_value(0, 0),
            100.0,
            "chase_back not parsed (port)"
        );
        assert_eq!(
            ctest_chase_cvar_value(0, 1),
            100.0,
            "chase_back not parsed (oracle)"
        );
        assert_eq!(
            ctest_chase_cvar_value(1, 0),
            16.0,
            "chase_up not parsed (port)"
        );
        assert_eq!(
            ctest_chase_cvar_value(1, 1),
            16.0,
            "chase_up not parsed (oracle)"
        );
    }
}

// ---------------------------------------------------------------------------
// Group 2: TraceLine.
//
// Quake/world.c's `c_ref_SV_RecursiveHullCheck` vs quake-capi's plain
// `SV_RecursiveHullCheck`, both over `stubs.c`'s synthetic room
// (CTEST_WORLD_BOXES, stubs.c:3505): a water box at (-256..-64)^3, a lava box,
// a current box, a CONTENTS_SOLID pillar at x/y 32..96 spanning z -256..256,
// and a CONTENTS_EMPTY room shell at +-448/+-192.
//
// The cases below straddle the pillar so both the "hit" and the "no hit" exit
// of the recursion are taken, and include a degenerate zero-length trace and a
// trace that starts inside solid.
//
// MUTATION USED: swapped the `start` and `end` arguments passed to
// `SV_RecursiveHullCheck` in `quake_rs_trace_line` -- `trace_line_matches_oracle`
// failed on case 1 (impact[0] 96.03125 vs the oracle's 31.968748), and both
// `chase_update_for_drawing_matches_oracle` and
// `chase_update_for_drawing_takes_the_pitch_fixup_branch` fell with it.
// Restored.

const TRACE_CASES: &[([c_float; 3], [c_float; 3])] = &[
    // straight through empty space, no obstruction
    ([-200.0, -200.0, 0.0], [-100.0, -200.0, 0.0]),
    // into the solid pillar from -x
    ([0.0, 64.0, 0.0], [200.0, 64.0, 0.0]),
    // into the solid pillar from +y
    ([64.0, 200.0, 0.0], [64.0, 0.0, 0.0]),
    // exactly along the pillar's -x face (grazing)
    ([-100.0, 32.0, 0.0], [200.0, 32.0, 0.0]),
    // degenerate: zero-length
    ([10.0, 10.0, 10.0], [10.0, 10.0, 10.0]),
    // starting inside the solid pillar
    ([64.0, 64.0, 0.0], [300.0, 64.0, 0.0]),
    // out through the room shell into the void
    ([0.0, 0.0, 0.0], [0.0, 0.0, 4000.0]),
    // long diagonal crossing the water box corner
    ([-400.0, -400.0, -400.0], [400.0, 400.0, 400.0]),
    // negative-only, entirely inside the water box
    ([-200.0, -200.0, -200.0], [-100.0, -100.0, -100.0]),
    // the 1<<20 ray Chase_UpdateForDrawing itself fires
    ([0.0, 0.0, 0.0], [1_048_576.0, 0.0, 0.0]),
];

#[test]
fn trace_line_matches_oracle() {
    let _g = lock();

    // SAFETY: single-threaded under TEST_LOCK; the trace entry points take
    // three writable `[c_float; 3]` locals, which is exactly what they get.
    unsafe {
        for (idx, (start, end)) in TRACE_CASES.iter().enumerate() {
            ctest_chase_reset();
            let mut s = *start;
            let mut e = *end;
            let mut oracle = [0.0f32; 3];
            c_ref_TraceLine(s.as_mut_ptr(), e.as_mut_ptr(), oracle.as_mut_ptr());

            ctest_chase_reset();
            let mut s = *start;
            let mut e = *end;
            let mut port = [0.0f32; 3];
            TraceLine(s.as_mut_ptr(), e.as_mut_ptr(), port.as_mut_ptr());

            assert_bits3(&format!("TraceLine case {idx} impact"), &port, &oracle);
        }

        // Not vacuous: the pillar case must genuinely stop short of `end`,
        // otherwise every impact would just be `end` and the comparison would
        // be measuring the argument rather than the traversal.
        ctest_chase_reset();
        let mut s = [0.0f32, 64.0, 0.0];
        let mut e = [200.0f32, 64.0, 0.0];
        let mut port = [0.0f32; 3];
        TraceLine(s.as_mut_ptr(), e.as_mut_ptr(), port.as_mut_ptr());
        assert!(
            port[0] < 200.0,
            "trace into the CONTENTS_SOLID pillar did not clip (impact {port:?}); \
             the synthetic room is not being reached, so this suite would prove nothing",
        );
    }
}

// ---------------------------------------------------------------------------
// Group 3: Chase_UpdateForDrawing.
//
// Covers the two `VectorLength(...) != 0` fallbacks, the `1 << 20` crosshair
// ray, `VectorAngles` and the `PITCH == +-90` yaw fixup. `r_refdef` is a
// single shared object (gl_rmain.c is not an oracle source), so the fixture
// republishes its inputs before each side's run and reads the result in
// between.
//
// Case 5 is the yaw-fixup case: the camera ends up directly above/below the
// crosshair point, so `VectorAngles` returns pitch exactly +-90 and the
// oracle's `r_refdef.viewangles[YAW] = cl.viewangles[YAW]` branch must fire on
// both sides.
//
// MUTATION USED: changed the yaw-fixup comparison in
// `quake_rs_chase_update_for_drawing` from `pitch == 90.0 || pitch == -90.0`
// to `pitch == 90.0` -- `chase_update_for_drawing_matches_oracle` failed on
// the "looking straight up: PITCH == -90 yaw fixup" case (YAW 0.0 vs the
// oracle's -23.0) and `chase_update_for_drawing_takes_the_pitch_fixup_branch`
// failed with "yaw fixup did not fire on the straight-up case". Restored.
// Second mutation: dropped `+ right[i] * chase_right` from the `ideal[i]`
// expression -- the "chase_right non-zero" case failed on vieworg[0]
// (-200.0 vs -152.0). Restored.

struct ChaseCase {
    name: &'static str,
    back: c_float,
    up: c_float,
    right: c_float,
    viewangles: [c_float; 3],
    viewent_origin: [c_float; 3],
    vieworg: [c_float; 3],
}

const CHASE_CASES: &[ChaseCase] = &[
    ChaseCase {
        name: "defaults, open space",
        back: 100.0,
        up: 16.0,
        right: 0.0,
        viewangles: [0.0, 0.0, 0.0],
        viewent_origin: [-200.0, -200.0, 0.0],
        vieworg: [-200.0, -200.0, 0.0],
    },
    ChaseCase {
        name: "camera pushed into the solid pillar",
        back: 100.0,
        up: 16.0,
        right: 0.0,
        viewangles: [0.0, 180.0, 0.0],
        viewent_origin: [-40.0, 64.0, 0.0],
        vieworg: [-40.0, 64.0, 0.0],
    },
    ChaseCase {
        name: "chase_right non-zero",
        back: 100.0,
        up: 16.0,
        right: 48.0,
        viewangles: [0.0, 90.0, 0.0],
        viewent_origin: [-200.0, -200.0, 0.0],
        vieworg: [-200.0, -200.0, 0.0],
    },
    ChaseCase {
        name: "zero chase_back (ideal == origin)",
        back: 0.0,
        up: 0.0,
        right: 0.0,
        viewangles: [30.0, 45.0, 0.0],
        viewent_origin: [-200.0, -200.0, 0.0],
        vieworg: [-200.0, -200.0, 0.0],
    },
    ChaseCase {
        name: "negative chase_back and chase_up",
        back: -100.0,
        up: -16.0,
        right: -32.0,
        viewangles: [-15.0, 200.0, 10.0],
        viewent_origin: [-200.0, -200.0, 0.0],
        vieworg: [-200.0, -200.0, 0.0],
    },
    ChaseCase {
        name: "looking straight down: PITCH == 90 yaw fixup",
        back: 0.0,
        up: 64.0,
        right: 0.0,
        viewangles: [90.0, 137.0, 0.0],
        // -300 rather than -200 is load-bearing. AngleVectors takes the
        // cosine of a *float* pi/2, which is -4.37e-8 rather than 0, so the
        // downward crosshair ray drifts ~6e-6 units sideways before it reaches
        // the floor at z == -192. At |x| == 200 that is more than half an ulp
        // and the impact lands one float off the camera, giving pitch
        // 89.99999 and skipping the fixup entirely; at |x| == 300 the ulp is
        // 3.05e-5 and the drift rounds away, so the delta is exactly vertical
        // and VectorAngles returns exactly 90.
        viewent_origin: [-300.0, -300.0, 0.0],
        vieworg: [-300.0, -300.0, 0.0],
    },
    ChaseCase {
        name: "looking straight up: PITCH == -90 yaw fixup",
        back: 0.0,
        up: -64.0,
        right: 0.0,
        viewangles: [-90.0, -23.0, 0.0],
        // same rounding argument as the case above.
        viewent_origin: [-300.0, -300.0, 0.0],
        vieworg: [-300.0, -300.0, 0.0],
    },
    ChaseCase {
        name: "player origin at the world origin, crosshair ray into the void",
        back: 100.0,
        up: 16.0,
        right: 0.0,
        viewangles: [0.0, 0.0, 0.0],
        viewent_origin: [0.0, 0.0, 0.0],
        vieworg: [0.0, 0.0, 0.0],
    },
];

fn run_chase_case(c: &ChaseCase, oracle: bool) -> [c_float; 6] {
    // SAFETY: called only from #[test] bodies holding TEST_LOCK; every pointer
    // handed to the fixture is a live `[c_float; 3]`/`[c_float; 6]` local that
    // outlives the call.
    unsafe {
        ctest_chase_reset();
        ctest_chase_set_cvars(c.back, c.up, c.right, 1.0);
        ctest_chase_set_client(c.viewangles.as_ptr(), c.viewent_origin.as_ptr());
        ctest_chase_set_refdef(c.vieworg.as_ptr(), c.viewangles.as_ptr());

        if oracle {
            c_ref_Chase_UpdateForDrawing();
        } else {
            Chase_UpdateForDrawing();
        }

        let mut out = [0.0f32; 6];
        ctest_chase_get_refdef(out.as_mut_ptr());
        out
    }
}

#[test]
fn chase_update_for_drawing_matches_oracle() {
    let _g = lock();

    for c in CHASE_CASES {
        let oracle = run_chase_case(c, true);
        let port = run_chase_case(c, false);
        assert_bits6(
            &format!("Chase_UpdateForDrawing [{}]", c.name),
            &port,
            &oracle,
        );
    }
}

/// The yaw fixup is the one branch a naive port silently drops, so it gets its
/// own non-vacuity assertion: pitch really must land on exactly +-90 and yaw
/// really must be overwritten with `cl.viewangles[YAW]`.
#[test]
fn chase_update_for_drawing_takes_the_pitch_fixup_branch() {
    let _g = lock();

    let down = &CHASE_CASES[5];
    let up = &CHASE_CASES[6];

    let d = run_chase_case(down, false);
    assert_eq!(
        d[3].to_bits(),
        90.0f32.to_bits(),
        "expected PITCH == 90, got {}",
        d[3]
    );
    assert_eq!(
        d[4].to_bits(),
        down.viewangles[1].to_bits(),
        "yaw fixup did not fire on the straight-down case",
    );

    let u = run_chase_case(up, false);
    assert_eq!(
        u[3].to_bits(),
        (-90.0f32).to_bits(),
        "expected PITCH == -90, got {}",
        u[3]
    );
    assert_eq!(
        u[4].to_bits(),
        up.viewangles[1].to_bits(),
        "yaw fixup did not fire on the straight-up case",
    );
}

/// The camera really must move: if `chase_back`/`chase_up` were reading 0.0 on
/// both sides (the T7.0 zeroed-cvar trap) `vieworg` would come back unchanged
/// and every comparison above would pass while measuring nothing.
#[test]
fn chase_update_for_drawing_actually_moves_the_camera() {
    let _g = lock();

    let c = &CHASE_CASES[0];
    let port = run_chase_case(c, false);
    let oracle = run_chase_case(c, true);

    for side in [("port", port), ("oracle", oracle)] {
        let moved = (0..3).any(|i| side.1[i].to_bits() != c.vieworg[i].to_bits());
        assert!(
            moved,
            "{}: Chase_UpdateForDrawing left r_refdef.vieworg at {:?} -- the chase cvars are \
             reading zero, so this suite is degenerate",
            side.0, c.vieworg,
        );
    }
}
