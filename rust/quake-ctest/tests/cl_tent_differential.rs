//! Differential/characterization gate for `Quake/cl_tent.c` -- client-side
//! temporary entities. Rust migration Phase 7, M7, task T7.2.
//!
//! The oracle fixture lives in `stubs/cl_tent_ref.c`; read its module doc
//! first. The short version: `cl_tent_glue.c` is `#ifdef USE_RUST_HOST` and
//! `cl_main.c` is an oracle source, so that file owns the plain twins of
//! `num_temp_entities`, `cl_temp_entities[]`, `cl_beams[]`, the visedict trio
//! and `CL_AllocDlight`, and every seeder writes both sides in one call.
//!
//! ## What is and is not observable here
//!
//! `cl_tent.c` reaches nine functions this harness does not implement for
//! real. Three of them are *recording doubles* in `stubs/pf_cl_ref.c`
//! (`PScript_RunParticleEffectTypeString`, `PScript_RunParticleEffect`,
//! `R_RunParticleEffect`), so their arguments are fully comparable. The other
//! six -- `Mod_ForName`, `CL_TraceLine`, `PScript_RunParticleWeather`,
//! `R_ParticleExplosion`, `R_ParticleExplosion2`, `R_BlobExplosion`,
//! `R_LavaSplash`, `R_TeleportSplash` -- are unconditional `Sys_Error` abort
//! stubs in `stubs.c`. Rather than declare the arms behind them untestable,
//! every driver enters through `Host_Guard`, so reaching an abort stub is an
//! *observation*: the suite compares which stub each side reached, with what
//! message, and how many bytes each side had consumed from `net_message` when
//! it got there. That turns "this arm aborts" into a real assertion about
//! branch selection and read ordering, which is most of what `CL_ParseTEnt`
//! does. What it cannot check is what the real particle/model code would have
//! produced -- see the residual-risk note at the end of this doc.
//!
//! Sound is not observable at all: `S_PrecacheSound`/`S_StartSound`'s plain
//! names are `quake-capi/src/snd_dma.rs` exports and the oracle calls
//! `c_ref_S_*`, but both take their `sound_started == false` early return here
//! (the reasoning `stubs/pf_cl_ref.c`'s header records at length). The
//! substitute is the shared `COM_Rand` generator: `TE_SPIKE`/`TE_SUPERSPIKE`
//! pick their
//! ricochet sample with `COM_Rand () % 5` and `COM_Rand () & 3`, and
//! `CL_UpdateTEnts` rolls a per-segment `COM_Rand () % 360`, so the value the
//! generator would yield next proves both sides consumed exactly the same
//! number of draws through exactly the same branches. Every test that can
//! consume a draw reseeds before each side and compares that value.
//!
//! ## Degenerate-gate defences
//!
//! A bit-exact differential passes whenever both sides degenerate identically,
//! so each test asserts something *positive* alongside the cross-side
//! comparison: a non-zero `readcount`, a recorder that was actually called, a
//! specific guard status, a non-empty temp-entity list. `cl_maxvisedicts` is
//! the sharpest example -- it is 0 from static init in this link, which makes
//! `CL_NewTempEntity` return NULL on its first line forever, so
//! `ctest_cltent_reset` seeds it and the boundary tests assert the non-NULL
//! allocations really happened.
//!
//! ## ADR-010
//!
//! `CL_UpdateTEnts` is the file's only float-shaped code: `atan2`/`sqrt` in
//! double, `* 180 / M_PI` in double, then an `(int)` truncation before the
//! store to a `float`. `beam_geometry_*` below drives it at values where the
//! truncation and the double-width division both matter (a yaw whose exact
//! degrees land just under an integer, a negative yaw that needs the `+= 360`
//! fixup, the `dist[0] == dist[1] == 0` degenerate arm in both `dist[2]`
//! signs), and compares the resulting `entity_t` images bit for bit.
//!
//! ## Residual risk
//!
//! The four `TE_LIGHTNING*`/`TE_BEAM` cases stop at `Mod_ForName`, so
//! `CL_ParseBeam`'s six `MSG_ReadCoord` calls and `CL_UpdateBeam`'s beam-table
//! search are never driven through `CL_ParseTEnt`. `CL_UpdateBeam` itself
//! stops at `CL_TraceLine`. The beam table is therefore seeded directly and
//! exercised through `CL_UpdateTEnts`, which covers the reader of the table
//! but not its writer. Lifting this needs settable returns for `Mod_ForName`
//! and `CL_TraceLine` in `stubs.c`, which this task is not allowed to edit;
//! it is reported to the milestone owner instead.

use core::ffi::{c_char, c_double, c_float, c_int, c_uint};
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

// ---------------------------------------------------------------------------
// protocol.h:355-374 temp-entity types, and the two protocolflags this suite
// drives MSG_ReadCoord with (protocol.h:44-51).

const TE_SPIKE: u8 = 0;
const TE_SUPERSPIKE: u8 = 1;
const TE_GUNSHOT: u8 = 2;
const TE_EXPLOSION: u8 = 3;
const TE_TAREXPLOSION: u8 = 4;
const TE_LIGHTNING1: u8 = 5;
const TE_LIGHTNING2: u8 = 6;
const TE_WIZSPIKE: u8 = 7;
const TE_KNIGHTSPIKE: u8 = 8;
const TE_LIGHTNING3: u8 = 9;
const TE_LAVASPLASH: u8 = 10;
const TE_TELEPORT: u8 = 11;
const TE_EXPLOSION2: u8 = 12;
const TE_BEAM: u8 = 13;
const TEDP_PARTICLERAIN: u8 = 55;
const TEDP_PARTICLESNOW: u8 = 56;

const PRFL_FLOATCOORD: c_uint = 1 << 4;

/// `client.h:85` / `:330` / `:70`.
const MAX_BEAMS: c_int = 32;
const MAX_TEMP_ENTITIES: c_int = 256;

extern "C" {
    // stubs/cl_tent_ref.c
    fn ctest_cltent_reset();
    fn ctest_cltent_model(idx: c_int) -> *mut core::ffi::c_void;
    fn ctest_cltent_attach_entities(attach: c_int);
    fn ctest_cltent_set_entity_origin(idx: c_int, org: *const c_float);
    fn ctest_cltent_set_time(time: c_double);
    fn ctest_cltent_set_paused(paused: c_int);
    fn ctest_cltent_set_viewentity(viewentity: c_int);
    fn ctest_cltent_set_protocol(protocolflags: c_uint, pext2: c_uint);
    fn ctest_cltent_set_visedicts(maxvisedicts: c_int, numvisedicts: c_int);
    fn ctest_cltent_get_numvisedicts(side: c_int) -> c_int;
    fn ctest_cltent_visedict_index(side: c_int, i: c_int) -> c_int;
    fn ctest_cltent_set_num_temp_entities(n: c_int);
    fn ctest_cltent_get_num_temp_entities(side: c_int) -> c_int;
    fn ctest_cltent_set_beam(
        side: c_int,
        idx: c_int,
        entity: c_int,
        model: *mut core::ffi::c_void,
        endtime: c_float,
        start: *const c_float,
        end: *const c_float,
    );
    fn ctest_cltent_entity_size() -> c_int;
    fn ctest_cltent_beam_size() -> c_int;
    fn ctest_cltent_dlight_size() -> c_int;
    fn ctest_cltent_get_temp_entity(side: c_int, idx: c_int, out: *mut u8);
    fn ctest_cltent_get_beam(side: c_int, idx: c_int, out: *mut u8);
    fn ctest_cltent_get_dlight(side: c_int, idx: c_int, out: *mut u8);
    fn ctest_cltent_begin_reading(side: c_int);
    fn ctest_cltent_get_readcount(side: c_int) -> c_int;
    fn ctest_cltent_get_badread(side: c_int) -> c_int;
    fn ctest_cltent_parse_tent(side: c_int) -> c_int;
    fn ctest_cltent_update_tents(side: c_int) -> c_int;
    fn ctest_cltent_init_tents(side: c_int) -> c_int;
    fn ctest_cltent_new_temp_entity(side: c_int) -> c_int;

    // stubs/sv_user_ref.c -- seeds BOTH net_message buffers.
    fn ctest_svuser_load_message(data: *const u8, len: c_int);

    // stubs/pf_cl_ref.c recording doubles.
    fn ctest_cl_reset();
    fn ctest_cl_pscript_typestring_called() -> c_int;
    fn ctest_cl_pscript_typestring_count() -> c_float;
    fn ctest_cl_pscript_typestring_name() -> *const c_char;
    fn ctest_cl_pscript_typestring_org(i: c_int) -> c_float;
    fn ctest_cl_pscript_typestring_set_return(ret: c_int);
    fn ctest_cl_runparticleeffect_called() -> c_int;
    fn ctest_cl_runparticleeffect_color() -> c_int;
    fn ctest_cl_runparticleeffect_count() -> c_int;
    fn ctest_cl_runparticleeffect_org(i: c_int) -> c_float;
    fn ctest_cl_runparticleeffect_dir(i: c_int) -> c_float;

    // stubs.c
    fn ctest_sys_error_message() -> *const c_char;
    fn ctest_host_error_message() -> *const c_char;
    fn COM_SeedRand(seed: u64);
    fn COM_Rand() -> c_int;

    // The port's CL_UpdateBeam is a #[no_mangle] export; the oracle's is
    // renamed. Both are driven only through the guarded helper below.
    fn CL_UpdateBeam(
        m: *mut core::ffi::c_void,
        trailname: *const c_char,
        impactname: *const c_char,
        ent: c_int,
        start: *mut c_float,
        end: *mut c_float,
    );
    fn c_ref_CL_UpdateBeam(
        m: *mut core::ffi::c_void,
        trailname: *const c_char,
        impactname: *const c_char,
        ent: c_int,
        start: *mut c_float,
        end: *mut c_float,
    );
}

/// `stubs.c:1465-1467` -- `Host_Guard`'s status codes.
const GUARD_OK: c_int = 0;
const GUARD_HOST_ERROR: c_int = 1;
const GUARD_SYS_ERROR: c_int = 2;

const SIDES: [c_int; 2] = [1 /* oracle */, 0 /* rust */];

fn side_name(side: c_int) -> &'static str {
    if side == 1 {
        "C"
    } else {
        "Rust"
    }
}

fn guard_message(status: c_int) -> String {
    // SAFETY: both getters return a pointer to a static NUL-terminated buffer
    // in stubs.c with process lifetime.
    unsafe {
        let p = match status {
            GUARD_HOST_ERROR => ctest_host_error_message(),
            GUARD_SYS_ERROR => ctest_sys_error_message(),
            _ => return String::new(),
        };
        core::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

/// The next value the shared generator would yield.
///
/// `stubs.c` exposes `COM_Rand`/`COM_SeedRand` but not `COM_RandState`, so
/// "how far did this run advance the generator" has to be observed by taking
/// one draw afterwards and looking it up in the sequence the seed produces
/// (`draws_consumed` below). Sampling is itself a draw, so it is always the
/// last thing a run records.
fn rand_next() -> c_int {
    // SAFETY: no arguments; advances a static generator in stubs.c, and every
    // caller holds TEST_LOCK.
    unsafe { COM_Rand() }
}

/// The first `n` values a fresh `seed` yields. Leaves the generator advanced;
/// callers reseed before it matters.
fn rand_sequence(seed: u64, n: usize) -> Vec<c_int> {
    // SAFETY: as above.
    unsafe { COM_SeedRand(seed) };
    (0..n).map(|_| rand_next()).collect()
}

/// How many draws a run made, deduced from the value it would have yielded
/// next. `None` means the run consumed more than `limit` draws.
fn draws_consumed(seed: u64, next: c_int, limit: usize) -> Option<usize> {
    rand_sequence(seed, limit + 1)
        .iter()
        .position(|&v| v == next)
}

// ---------------------------------------------------------------------------
// Message construction. protocolflags 0 means MSG_ReadCoord == MSG_ReadCoord16
// (net_msg.c:392-395, a 13.3 fixed-point short); PRFL_FLOATCOORD means
// MSG_ReadFloat. Coordinates are written as raw encodings, not as floats to be
// re-derived, so the decoded value is something the differential OBSERVES
// rather than something this test predicts.

#[derive(Default)]
struct Msg(Vec<u8>);

impl Msg {
    fn byte(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
    fn short(mut self, v: i16) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn float(mut self, v: f32) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    /// Three 13.3 fixed-point coords.
    fn coords16(self, a: i16, b: i16, c: i16) -> Self {
        self.short(a).short(b).short(c)
    }
}

// ---------------------------------------------------------------------------
// Observation records. Everything float-shaped is compared as raw bits.

#[derive(Clone, PartialEq, Eq, Debug)]
struct ParseObs {
    status: c_int,
    message: String,
    readcount: c_int,
    badread: c_int,
    ts_called: c_int,
    ts_count_bits: u32,
    ts_name: String,
    ts_org_bits: [u32; 3],
    rpe_called: c_int,
    rpe_color: c_int,
    rpe_count: c_int,
    rpe_org_bits: [u32; 3],
    rpe_dir_bits: [u32; 3],
    num_temp_entities: c_int,
    dlight0: Vec<u8>,
    rand_next: c_int,
}

/// Runs one side of `CL_ParseTEnt` over `msg` and records everything the
/// harness can see. `setup` runs after `ctest_cl_reset` so a test can change
/// the particle-script return without it being reset out from under the run.
fn run_parse(side: c_int, msg: &[u8], seed: u64, setup: &dyn Fn()) -> ParseObs {
    // SAFETY: every callee is a fixture entry point in stubs/cl_tent_ref.c,
    // stubs/pf_cl_ref.c or stubs.c, all operating on static storage with
    // process lifetime, and the whole sequence is serialised by TEST_LOCK.
    // ctest_cltent_parse_tent enters through Host_Guard, so an abort stub's
    // longjmp is caught in a pure C frame (ADR-009).
    unsafe {
        ctest_cl_reset();
        setup();
        COM_SeedRand(seed);
        ctest_svuser_load_message(msg.as_ptr(), msg.len() as c_int);
        ctest_cltent_begin_reading(side);

        let status = ctest_cltent_parse_tent(side);

        let dl_size = ctest_cltent_dlight_size() as usize;
        let mut dlight0 = vec![0u8; dl_size];
        ctest_cltent_get_dlight(side, 0, dlight0.as_mut_ptr());

        ParseObs {
            status,
            message: guard_message(status),
            readcount: ctest_cltent_get_readcount(side),
            badread: ctest_cltent_get_badread(side),
            ts_called: ctest_cl_pscript_typestring_called(),
            ts_count_bits: ctest_cl_pscript_typestring_count().to_bits(),
            ts_name: core::ffi::CStr::from_ptr(ctest_cl_pscript_typestring_name())
                .to_string_lossy()
                .into_owned(),
            ts_org_bits: [
                ctest_cl_pscript_typestring_org(0).to_bits(),
                ctest_cl_pscript_typestring_org(1).to_bits(),
                ctest_cl_pscript_typestring_org(2).to_bits(),
            ],
            rpe_called: ctest_cl_runparticleeffect_called(),
            rpe_color: ctest_cl_runparticleeffect_color(),
            rpe_count: ctest_cl_runparticleeffect_count(),
            rpe_org_bits: [
                ctest_cl_runparticleeffect_org(0).to_bits(),
                ctest_cl_runparticleeffect_org(1).to_bits(),
                ctest_cl_runparticleeffect_org(2).to_bits(),
            ],
            rpe_dir_bits: [
                ctest_cl_runparticleeffect_dir(0).to_bits(),
                ctest_cl_runparticleeffect_dir(1).to_bits(),
                ctest_cl_runparticleeffect_dir(2).to_bits(),
            ],
            num_temp_entities: ctest_cltent_get_num_temp_entities(side),
            dlight0,
            rand_next: rand_next(),
        }
    }
}

/// Drives both sides over one message and returns the (identical, once
/// asserted) observation so the caller can make its own positive assertions.
fn parse_both(msg: &[u8], seed: u64, setup: &dyn Fn()) -> ParseObs {
    // SAFETY: fixture reset over static storage; serialised by TEST_LOCK.
    unsafe { ctest_cltent_reset() };
    let c = run_parse(SIDES[0], msg, seed, setup);
    let rust = run_parse(SIDES[1], msg, seed, setup);
    assert_eq!(c, rust, "CL_ParseTEnt diverged (C vs Rust) for {msg:02x?}");
    c
}

// ---------------------------------------------------------------------------
// CL_InitTEnts

#[test]
fn init_tents_status_parity() {
    let _g = lock();
    // SAFETY: fixture entry points over static storage, serialised.
    unsafe {
        ctest_cltent_reset();
        for side in SIDES {
            let status = ctest_cltent_init_tents(side);
            assert_eq!(
                status,
                GUARD_OK,
                "{} CL_InitTEnts raised: {}",
                side_name(side),
                guard_message(status)
            );
        }
    }
    // Deliberately weak, and recorded as such: the seven cl_sfx_* handles are
    // file-static in C and private in Rust, and S_PrecacheSound no-ops in this
    // harness (sound_started == false), so the ONLY comparable property of
    // CL_InitTEnts here is that neither side raises. cl_tent_ref.c's module
    // doc explains why standing up a real DMA fixture is out of scope.
}

// ---------------------------------------------------------------------------
// The five "particle + sound" cases that run to completion.

#[test]
fn parse_tent_spike_family_script_declines() {
    let _g = lock();
    // PScript_RunParticleEffectTypeString returns NON-ZERO when it did not
    // handle the effect, and that is what makes cl_tent.c fall back. Declining
    // runs the R_RunParticleEffect recording double, so colour/count/org/dir
    // are all comparable.
    let cases: [(u8, &str, c_int, c_int); 5] = [
        (TE_WIZSPIKE, "TE_WIZSPIKE", 20, 30),
        (TE_KNIGHTSPIKE, "TE_KNIGHTSPIKE", 226, 20),
        (TE_SPIKE, "TE_SPIKE", 0, 10),
        (TE_SUPERSPIKE, "TE_SUPERSPIKE", 0, 20),
        (TE_GUNSHOT, "TE_GUNSHOT", 0, 20),
    ];

    for (ty, name, color, count) in cases {
        let msg = Msg::default().byte(ty).coords16(129, -3001, 17).0;
        let obs = parse_both(&msg, 0x1234_5678_9abc_def0, &|| {
            // SAFETY: sets a static in stubs/pf_cl_ref.c.
            unsafe { ctest_cl_pscript_typestring_set_return(1) };
        });

        assert_eq!(obs.status, GUARD_OK, "{name}: {}", obs.message);
        assert_eq!(obs.readcount, 7, "{name}: type byte + three 13.3 coords");
        assert_eq!(obs.badread, 0, "{name}");
        assert_eq!(obs.ts_called, 1, "{name}: script was consulted");
        assert_eq!(obs.ts_name, name);
        assert_eq!(obs.rpe_called, 1, "{name}: fallback ran");
        assert_eq!(obs.rpe_color, color, "{name}");
        assert_eq!(obs.rpe_count, count, "{name}");
        // 129 / 8, -3001 / 8, 17 / 8 -- decoded, not predicted; asserted only
        // to prove the coordinates really reached the recorder rather than
        // both sides recording a default-zero vector.
        assert_eq!(
            obs.rpe_org_bits,
            [
                16.125f32.to_bits(),
                (-375.125f32).to_bits(),
                2.125f32.to_bits()
            ],
            "{name}"
        );
        assert_eq!(obs.rpe_dir_bits, [0u32; 3], "{name}: vec3_origin");
        assert_eq!(obs.ts_org_bits, obs.rpe_org_bits, "{name}");
    }
}

#[test]
fn parse_tent_spike_family_script_handles() {
    let _g = lock();
    // Script handles the effect (return 0) -> R_RunParticleEffect is skipped,
    // and TE_SPIKE/TE_SUPERSPIKE still draw their ricochet samples.
    for ty in [
        TE_WIZSPIKE,
        TE_KNIGHTSPIKE,
        TE_SPIKE,
        TE_SUPERSPIKE,
        TE_GUNSHOT,
    ] {
        let msg = Msg::default().byte(ty).coords16(8, 16, 24).0;
        let obs = parse_both(&msg, 7, &|| {
            // SAFETY: sets a static in stubs/pf_cl_ref.c.
            unsafe { ctest_cl_pscript_typestring_set_return(0) };
        });
        assert_eq!(obs.status, GUARD_OK, "type {ty}: {}", obs.message);
        assert_eq!(obs.ts_called, 1, "type {ty}");
        assert_eq!(obs.rpe_called, 0, "type {ty}: fallback must be skipped");
    }
}

#[test]
fn parse_tent_spike_ricochet_draw_counts() {
    let _g = lock();
    // TE_SPIKE/TE_SUPERSPIKE consume one COM_Rand for the `% 5` test and a
    // second for the `& 3` sample pick, but only on the 1-in-5 branch. Sweeping
    // seeds drives both arms; the generator state after the run is what proves
    // each side took the same one, since the sound calls themselves no-op.
    let mut seen_one_draw = false;
    let mut seen_two_draws = false;

    for seed in 1u64..40 {
        for ty in [TE_SPIKE, TE_SUPERSPIKE] {
            let msg = Msg::default().byte(ty).coords16(1, 2, 3).0;
            let obs = parse_both(&msg, seed, &|| {
                // SAFETY: sets a static in stubs/pf_cl_ref.c.
                unsafe { ctest_cl_pscript_typestring_set_return(0) };
            });
            assert_eq!(obs.status, GUARD_OK);

            let draws = draws_consumed(seed, obs.rand_next, 4)
                .unwrap_or_else(|| panic!("seed {seed} type {ty}: more than 4 draws"));
            match draws {
                1 => seen_one_draw = true,
                2 => seen_two_draws = true,
                n => panic!("seed {seed} type {ty}: unexpected {n} draws"),
            }
        }
    }

    assert!(
        seen_one_draw,
        "the COM_Rand () % 5 != 0 arm was never taken"
    );
    assert!(seen_two_draws, "the ricochet arm was never taken");
}

// ---------------------------------------------------------------------------
// The four explosion/splash cases, both arms of their `if (PScript_...)`.

#[test]
fn parse_tent_explosion_family_script_declines() {
    let _g = lock();
    // Script declines -> the R_* fallback is an abort stub, so BOTH sides stop
    // there. Comparing which stub was reached and the read cursor at that
    // point is the assertion: it proves the branch and the read order, which
    // is all cl_tent.c itself decides.
    let cases: [(u8, &str); 4] = [
        (TE_EXPLOSION, "R_ParticleExplosion"),
        (TE_TAREXPLOSION, "R_BlobExplosion"),
        (TE_LAVASPLASH, "R_LavaSplash"),
        (TE_TELEPORT, "R_TeleportSplash"),
    ];

    for (ty, stub) in cases {
        let msg = Msg::default().byte(ty).coords16(-64, 0, 4096).0;
        let obs = parse_both(&msg, 11, &|| {
            // SAFETY: sets a static in stubs/pf_cl_ref.c.
            unsafe { ctest_cl_pscript_typestring_set_return(1) };
        });
        assert_eq!(obs.status, GUARD_SYS_ERROR, "type {ty}");
        assert!(
            obs.message.contains(stub),
            "type {ty}: expected {stub}, got {:?}",
            obs.message
        );
        assert_eq!(
            obs.readcount, 7,
            "type {ty}: stopped after the three coords"
        );
        assert_eq!(obs.ts_called, 1, "type {ty}");
    }
}

#[test]
fn parse_tent_explosion_family_script_handles() {
    let _g = lock();
    // Script handles it -> the abort stub is skipped and TE_EXPLOSION goes on
    // to allocate a dlight. cl_tent_ref.c's hand-transcribed CL_AllocDlight is
    // what the Rust side calls; comparing the whole dlight_t byte image against
    // the oracle's c_ref_cl_dlights[0] gates that transcription too.
    let handled = || {
        // SAFETY: sets a static in stubs/pf_cl_ref.c.
        unsafe { ctest_cl_pscript_typestring_set_return(0) };
    };
    let msg = Msg::default().byte(TE_EXPLOSION).coords16(80, -80, 8).0;
    let obs = parse_both(&msg, 3, &handled);
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.readcount, 7);
    assert_eq!(obs.ts_called, 1);
    assert_eq!(obs.rpe_called, 0);
    assert_ne!(
        obs.dlight0,
        vec![0u8; obs.dlight0.len()],
        "CL_AllocDlight must have written slot 0"
    );

    // TE_TAREXPLOSION allocates no dlight; TE_LAVASPLASH/TE_TELEPORT read
    // three coords and stop.
    for ty in [TE_TAREXPLOSION, TE_LAVASPLASH, TE_TELEPORT] {
        let msg = Msg::default().byte(ty).coords16(1, 2, 3).0;
        let obs = parse_both(&msg, 3, &handled);
        assert_eq!(obs.status, GUARD_OK, "type {ty}: {}", obs.message);
        assert_eq!(obs.readcount, 7, "type {ty}");
        assert_eq!(obs.dlight0, vec![0u8; obs.dlight0.len()], "type {ty}");
    }
}

#[test]
fn parse_tent_explosion2_name_and_dlight() {
    let _g = lock();
    // TE_EXPLOSION2 is the only case that formats its effect name
    // (ClTent_Glue_Explosion2Name -> va ("TE_EXPLOSION2_%i_%i", ...)), and the
    // only one that reads two extra bytes after the coords.
    for (start, len) in [(0u8, 0u8), (73, 32), (255, 1)] {
        let msg = Msg::default()
            .byte(TE_EXPLOSION2)
            .coords16(24, -24, 0)
            .byte(start)
            .byte(len)
            .0;
        let obs = parse_both(&msg, 5, &|| {
            // SAFETY: sets a static in stubs/pf_cl_ref.c.
            unsafe { ctest_cl_pscript_typestring_set_return(0) };
        });
        assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
        assert_eq!(obs.readcount, 9, "type byte + three coords + two bytes");
        assert_eq!(obs.ts_name, format!("TE_EXPLOSION2_{start}_{len}"));
        assert_ne!(obs.dlight0, vec![0u8; obs.dlight0.len()]);
    }

    // And the declining arm reaches R_ParticleExplosion2 with the same cursor.
    let msg = Msg::default()
        .byte(TE_EXPLOSION2)
        .coords16(24, -24, 0)
        .byte(7)
        .byte(8)
        .0;
    let obs = parse_both(&msg, 5, &|| {
        // SAFETY: sets a static in stubs/pf_cl_ref.c.
        unsafe { ctest_cl_pscript_typestring_set_return(1) };
    });
    assert_eq!(obs.status, GUARD_SYS_ERROR);
    assert!(
        obs.message.contains("R_ParticleExplosion2"),
        "{:?}",
        obs.message
    );
    assert_eq!(obs.readcount, 9);
}

// ---------------------------------------------------------------------------
// The four beam cases and the two weather cases.

#[test]
fn parse_tent_beam_cases_stop_at_mod_forname() {
    let _g = lock();
    // Mod_ForName is evaluated as CL_ParseBeam's ARGUMENT, so it runs before
    // any of CL_ParseBeam's own reads. Both sides must therefore stop with the
    // read cursor still at 1 (the type byte alone) -- that ordering is the
    // whole assertion, and it is exactly what an ADR-009 mistake would break:
    // a Rust port that read the beam's coordinates before calling the guard
    // would show readcount 15 here.
    for ty in [TE_LIGHTNING1, TE_LIGHTNING2, TE_LIGHTNING3, TE_BEAM] {
        let msg = Msg::default()
            .byte(ty)
            .short(3) // entity
            .coords16(1, 2, 3)
            .coords16(4, 5, 6)
            .0;
        let obs = parse_both(&msg, 2, &|| {});
        assert_eq!(obs.status, GUARD_SYS_ERROR, "type {ty}");
        assert!(
            obs.message.contains("Mod_ForName"),
            "type {ty}: {:?}",
            obs.message
        );
        assert_eq!(
            obs.readcount, 1,
            "type {ty}: stopped before CL_ParseBeam's reads"
        );
        assert_eq!(obs.ts_called, 0, "type {ty}");
    }
}

#[test]
fn parse_tent_weather_cases_read_everything_first() {
    let _g = lock();
    // TEDP_PARTICLERAIN/SNOW reach an abort stub too, but only AFTER eleven
    // reads: nine coords, a short count and a colour byte. The read cursor at
    // the abort is what proves the decode order.
    for ty in [TEDP_PARTICLERAIN, TEDP_PARTICLESNOW] {
        let msg = Msg::default()
            .byte(ty)
            .coords16(-100, -200, -300)
            .coords16(100, 200, 300)
            .coords16(0, 0, -80)
            .short(-2) // (unsigned short) count == 65534
            .byte(73)
            .0;
        let obs = parse_both(&msg, 4, &|| {});
        assert_eq!(obs.status, GUARD_SYS_ERROR, "type {ty}");
        assert!(
            obs.message.contains("PScript_RunParticleWeather"),
            "type {ty}: {:?}",
            obs.message
        );
        assert_eq!(obs.readcount, 22, "type {ty}: 1 + 9*2 + 2 + 1");
        assert_eq!(obs.badread, 0, "type {ty}");
    }
}

// ---------------------------------------------------------------------------
// Failure arms.

#[test]
fn parse_tent_bad_type_and_truncation() {
    let _g = lock();

    // The default arm: ClTent_Glue_BadTEntType on the Rust side, a direct
    // Sys_Error on the oracle's. Same message, same cursor.
    for ty in [14u8, 54, 57, 99, 255] {
        let msg = Msg::default().byte(ty).0;
        let obs = parse_both(&msg, 1, &|| {});
        assert_eq!(obs.status, GUARD_SYS_ERROR, "type {ty}");
        assert_eq!(obs.message, "CL_ParseTEnt: bad type", "type {ty}");
        assert_eq!(obs.readcount, 1, "type {ty}");
    }

    // An empty message: MSG_ReadByte underflows, sets msg_badread and returns
    // -1, which is not a known type -- so the bad-type arm again, but with
    // badread set on both sides.
    let obs = parse_both(&[], 1, &|| {});
    assert_eq!(obs.status, GUARD_SYS_ERROR);
    assert_eq!(obs.message, "CL_ParseTEnt: bad type");
    assert_eq!(obs.badread, 1, "an empty message must set msg_badread");

    // A truncated coordinate run: the type byte decodes, the coords underflow.
    let msg = Msg::default().byte(TE_GUNSHOT).short(5).0;
    let obs = parse_both(&msg, 1, &|| {
        // SAFETY: sets a static in stubs/pf_cl_ref.c.
        unsafe { ctest_cl_pscript_typestring_set_return(1) };
    });
    assert_eq!(
        obs.badread, 1,
        "a short read must set msg_badread on both sides"
    );
    assert_eq!(obs.rpe_called, 1, "the case still runs to its fallback");
}

#[test]
fn parse_tent_float_coord_protocol() {
    let _g = lock();
    // PRFL_FLOATCOORD swaps MSG_ReadCoord16 for MSG_ReadFloat (net_msg.c:409).
    // cl.protocolflags is read from the C-owned `cl` on both sides, so this
    // also proves the port reads the flags rather than hardcoding a decoder.
    // SAFETY: fixture reset + protocol seeding over static storage.
    unsafe { ctest_cltent_reset() };
    let msg = Msg::default()
        .byte(TE_GUNSHOT)
        .float(f32::from_bits(0x3fc0_0001)) // 1.5000001, an awkward mantissa
        .float(-0.0)
        .float(1234.5678)
        .0;

    let setup = || {
        // SAFETY: publishes cl.protocolflags to both sides and sets a static.
        unsafe {
            ctest_cltent_set_protocol(PRFL_FLOATCOORD, 0);
            ctest_cl_pscript_typestring_set_return(1);
        }
    };
    let c = run_parse(SIDES[0], &msg, 9, &setup);
    let rust = run_parse(SIDES[1], &msg, 9, &setup);
    assert_eq!(c, rust);
    assert_eq!(c.status, GUARD_OK, "{}", c.message);
    assert_eq!(c.readcount, 13, "type byte + three 32-bit floats");
    assert_eq!(
        c.rpe_org_bits,
        [0x3fc0_0001, 0x8000_0000, 1234.5678f32.to_bits()],
        "coordinates must survive bit-exactly, -0.0 included"
    );
}

// ---------------------------------------------------------------------------
// CL_NewTempEntity boundaries.

/// One side's `CL_NewTempEntity` run: the returned slot indices,
/// `cl_numvisedicts`, `num_temp_entities`, the visedict slot indices and the
/// allocated entities' byte images.
type AllocObs = (Vec<c_int>, c_int, c_int, Vec<c_int>, Vec<Vec<u8>>);

#[test]
fn new_temp_entity_allocation_sequence() {
    let _g = lock();
    let ent_size = {
        // SAFETY: a pure sizeof accessor.
        unsafe { ctest_cltent_entity_size() as usize }
    };

    let mut per_side: Vec<AllocObs> = Vec::new();
    // SAFETY: fixture reset over static storage, serialised by TEST_LOCK.
    unsafe { ctest_cltent_reset() };

    for side in SIDES {
        // SAFETY: fixture seeding/driving over static storage.
        unsafe {
            ctest_cltent_set_num_temp_entities(0);
            ctest_cltent_set_visedicts(4, 0);

            let indices: Vec<c_int> = (0..6).map(|_| ctest_cltent_new_temp_entity(side)).collect();
            let numvis = ctest_cltent_get_numvisedicts(side);
            let numtemp = ctest_cltent_get_num_temp_entities(side);
            let vis: Vec<c_int> = (0..6)
                .map(|i| ctest_cltent_visedict_index(side, i))
                .collect();
            let images: Vec<Vec<u8>> = (0..4)
                .map(|i| {
                    let mut b = vec![0u8; ent_size];
                    ctest_cltent_get_temp_entity(side, i, b.as_mut_ptr());
                    b
                })
                .collect();
            per_side.push((indices, numvis, numtemp, vis, images));
        }
    }

    assert_eq!(per_side[0], per_side[1], "CL_NewTempEntity diverged");

    let (indices, numvis, numtemp, vis, images) = &per_side[0];
    // cl_maxvisedicts == 4, so the fifth and sixth calls must fail on the
    // `cl_numvisedicts == cl_maxvisedicts` guard -- the positive assertion
    // that keeps an all-NULL degenerate run from passing.
    assert_eq!(indices, &vec![0, 1, 2, 3, -1, -1]);
    assert_eq!(*numvis, 4);
    assert_eq!(*numtemp, 4);
    assert_eq!(vis, &vec![0, 1, 2, 3, -1, -1]);
    // The images are compared across sides above but cannot be asserted
    // non-zero: CL_NewTempEntity memsets the slot and copies `nullentitystate`,
    // which is all-zero in this link, so a correct allocation is genuinely
    // indistinguishable from an untouched slot by content alone. The positive
    // evidence that the allocations happened is the index/counter sequence
    // asserted above; the non-zero-content case is covered by the
    // update_tents_* tests, which fill each slot with an origin, angles and a
    // model pointer.
    assert_eq!(images.len(), 4);
}

#[test]
fn new_temp_entity_temp_table_boundary() {
    let _g = lock();
    // The other early return: num_temp_entities == MAX_TEMP_ENTITIES, checked
    // second, so a slot is still refused even with visedict room to spare.
    let mut per_side: Vec<(c_int, c_int, c_int, c_int)> = Vec::new();
    // SAFETY: fixture reset over static storage.
    unsafe { ctest_cltent_reset() };

    for side in SIDES {
        // SAFETY: fixture seeding/driving over static storage.
        unsafe {
            ctest_cltent_set_visedicts(MAX_TEMP_ENTITIES + 8, 0);
            ctest_cltent_set_num_temp_entities(MAX_TEMP_ENTITIES - 1);
            let last = ctest_cltent_new_temp_entity(side);
            let refused = ctest_cltent_new_temp_entity(side);
            per_side.push((
                last,
                refused,
                ctest_cltent_get_num_temp_entities(side),
                ctest_cltent_get_numvisedicts(side),
            ));
        }
    }

    assert_eq!(per_side[0], per_side[1]);
    assert_eq!(
        per_side[0],
        (MAX_TEMP_ENTITIES - 1, -1, MAX_TEMP_ENTITIES, 1)
    );
}

// ---------------------------------------------------------------------------
// CL_UpdateTEnts

/// Everything `CL_UpdateTEnts` can be seen to produce, per side.
#[derive(Clone, PartialEq, Eq, Debug)]
struct UpdateObs {
    status: c_int,
    message: String,
    num_temp_entities: c_int,
    numvisedicts: c_int,
    visedicts: Vec<c_int>,
    entities: Vec<Vec<u8>>,
    beams: Vec<Vec<u8>>,
    rand_next: c_int,
}

fn run_update(side: c_int, seed: u64, entity_count: usize, beam_count: usize) -> UpdateObs {
    // SAFETY: fixture entry points over static storage, serialised by
    // TEST_LOCK; the driver enters through Host_Guard.
    unsafe {
        COM_SeedRand(seed);
        let status = ctest_cltent_update_tents(side);

        let ent_size = ctest_cltent_entity_size() as usize;
        let beam_size = ctest_cltent_beam_size() as usize;

        UpdateObs {
            status,
            message: guard_message(status),
            num_temp_entities: ctest_cltent_get_num_temp_entities(side),
            numvisedicts: ctest_cltent_get_numvisedicts(side),
            visedicts: (0..entity_count as c_int)
                .map(|i| ctest_cltent_visedict_index(side, i))
                .collect(),
            entities: (0..entity_count as c_int)
                .map(|i| {
                    let mut b = vec![0u8; ent_size];
                    ctest_cltent_get_temp_entity(side, i, b.as_mut_ptr());
                    b
                })
                .collect(),
            beams: (0..beam_count as c_int)
                .map(|i| {
                    let mut b = vec![0u8; beam_size];
                    ctest_cltent_get_beam(side, i, b.as_mut_ptr());
                    b
                })
                .collect(),
            rand_next: rand_next(),
        }
    }
}

/// Seeds one beam into BOTH sides' tables and runs `CL_UpdateTEnts` on each.
fn update_with_beam(
    entity: c_int,
    model_idx: c_int,
    endtime: f32,
    start: [f32; 3],
    end: [f32; 3],
    seed: u64,
) -> UpdateObs {
    // SAFETY: fixture reset/seeding over static storage, serialised.
    unsafe { ctest_cltent_reset() };
    let mut obs: Vec<UpdateObs> = Vec::new();
    for side in SIDES {
        // SAFETY: as above.
        unsafe {
            ctest_cltent_set_num_temp_entities(0);
            ctest_cltent_set_visedicts(64, 0);
            let model = if model_idx < 0 {
                core::ptr::null_mut()
            } else {
                ctest_cltent_model(model_idx)
            };
            ctest_cltent_set_beam(
                side,
                0,
                entity,
                model,
                endtime,
                start.as_ptr(),
                end.as_ptr(),
            );
        }
        obs.push(run_update(side, seed, 16, MAX_BEAMS as usize));
    }
    assert_eq!(obs[0], obs[1], "CL_UpdateTEnts diverged");
    obs.remove(0)
}

#[test]
fn update_tents_beam_geometry() {
    let _g = lock();
    // ADR-010: yaw/pitch go through atan2/sqrt in DOUBLE, are scaled by
    // `180 / M_PI` in double, truncated by `(int)`, then stored to a float.
    // Each case below lands on a different corner of that chain.
    let cases: [(&str, [f32; 3], [f32; 3]); 6] = [
        // Straight up and straight down: the dist[0] == dist[1] == 0 arm.
        ("up", [0.0, 0.0, 0.0], [0.0, 0.0, 90.0]),
        ("down", [0.0, 0.0, 0.0], [0.0, 0.0, -90.0]),
        // A 45-degree diagonal: atan2 lands exactly on 45, so the (int)
        // truncation of a value the double division renders as 44.999... vs
        // 45.0 is decided by the double-width arithmetic.
        ("diag", [0.0, 0.0, 0.0], [100.0, 100.0, 0.0]),
        // Negative yaw, needing the `yaw += 360` fixup.
        ("negyaw", [0.0, 0.0, 0.0], [-100.0, -1.0, 0.0]),
        // Negative pitch, needing the `pitch += 360` fixup, with a length that
        // is not a multiple of 30 so the final segment is a partial step.
        ("negpitch", [10.0, 20.0, 30.0], [90.0, 20.0, -37.0]),
        // A long beam: 300 units at 30 per segment is ten temp entities, each
        // consuming one COM_Rand draw.
        ("long", [0.0, 0.0, 0.0], [300.0, 0.0, 0.0]),
    ];

    for (name, start, end) in cases {
        let obs = update_with_beam(7, 0, 10.0, start, end, 0xdead_beef);
        assert_eq!(obs.status, GUARD_OK, "{name}: {}", obs.message);
        assert!(
            obs.num_temp_entities > 0,
            "{name}: the beam must have produced segments"
        );
        assert_eq!(
            obs.num_temp_entities, obs.numvisedicts,
            "{name}: every segment is also a visedict"
        );
        // One COM_Rand draw per segment: the run must not have left the
        // generator where a zero-segment run would have.
        let draws = draws_consumed(0xdead_beef, obs.rand_next, 16)
            .unwrap_or_else(|| panic!("{name}: more than 16 draws"));
        assert_eq!(
            draws, obs.num_temp_entities as usize,
            "{name}: one COM_Rand per segment"
        );
        assert!(draws > 0, "{name}");
    }
}

#[test]
fn update_tents_expiry_boundary() {
    let _g = lock();
    // `if (!b->model || b->endtime < cl.time) continue;` -- endtime EQUAL to
    // cl.time is NOT expired. cl.time is 1.0 out of ctest_cltent_reset.
    for (endtime, expect_segments) in [(1.0f32, true), (0.999_999_9f32, false), (2.0f32, true)] {
        let obs = update_with_beam(7, 0, endtime, [0.0, 0.0, 0.0], [60.0, 0.0, 0.0], 5);
        assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
        assert_eq!(
            obs.num_temp_entities > 0,
            expect_segments,
            "endtime {endtime} vs cl.time 1.0"
        );
    }

    // A NULL model is skipped whatever the endtime.
    let obs = update_with_beam(7, -1, 10.0, [0.0, 0.0, 0.0], [60.0, 0.0, 0.0], 5);
    assert_eq!(
        obs.num_temp_entities, 0,
        "a model-less beam must be skipped"
    );
}

#[test]
fn update_tents_viewentity_start_override() {
    let _g = lock();
    // `if (b->entity == cl.viewentity && cl.entities)` rewrites b->start from
    // the view entity's origin -- a write back into the beam table, so the
    // beam byte images are part of the comparison.
    let origin = [11.5f32, -22.25, 33.125];

    for (entity, attach, expect_override) in [(1, 1, true), (2, 1, false), (1, 0, false)] {
        // SAFETY: fixture reset/seeding over static storage, serialised.
        unsafe { ctest_cltent_reset() };
        let mut obs: Vec<UpdateObs> = Vec::new();
        for side in SIDES {
            // SAFETY: as above.
            unsafe {
                ctest_cltent_set_viewentity(1);
                ctest_cltent_attach_entities(attach);
                ctest_cltent_set_entity_origin(1, origin.as_ptr());
                ctest_cltent_set_num_temp_entities(0);
                ctest_cltent_set_visedicts(64, 0);
                ctest_cltent_set_beam(
                    side,
                    0,
                    entity,
                    ctest_cltent_model(0),
                    10.0,
                    [0.0f32, 0.0, 0.0].as_ptr(),
                    [90.0f32, 0.0, 0.0].as_ptr(),
                );
            }
            obs.push(run_update(side, 17, 8, MAX_BEAMS as usize));
        }
        assert_eq!(obs[0], obs[1], "entity {entity} attach {attach}");

        let beam0 = &obs[0].beams[0];
        let overridden = beam0
            .windows(4)
            .any(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]) == origin[0].to_bits());
        assert_eq!(
            overridden, expect_override,
            "entity {entity} attach {attach}: b->start override"
        );
    }
}

#[test]
fn update_tents_paused_reseeds_generator() {
    let _g = lock();
    // `if (cl.paused) COM_SeedRand ((uint64_t)(cl.time * 1000))` freezes the
    // beams. The cast is of a DOUBLE product, so cl.time is chosen to make the
    // truncation load-bearing: 1.9999 * 1000 truncates to 1999, not 2000.
    // SAFETY: fixture reset/seeding over static storage, serialised.
    unsafe { ctest_cltent_reset() };
    let mut obs: Vec<UpdateObs> = Vec::new();
    for side in SIDES {
        // SAFETY: as above.
        unsafe {
            ctest_cltent_set_time(1.999_9);
            ctest_cltent_set_paused(1);
            ctest_cltent_set_num_temp_entities(0);
            ctest_cltent_set_visedicts(64, 0);
            ctest_cltent_set_beam(
                side,
                0,
                7,
                ctest_cltent_model(0),
                10.0,
                [0.0f32, 0.0, 0.0].as_ptr(),
                [120.0f32, 0.0, 0.0].as_ptr(),
            );
        }
        obs.push(run_update(side, 999, 8, MAX_BEAMS as usize));
    }
    assert_eq!(obs[0], obs[1], "paused CL_UpdateTEnts diverged");
    assert_eq!(obs[0].status, GUARD_OK, "{}", obs[0].message);
    assert!(obs[0].num_temp_entities > 0);

    // The reseed must have discarded the 999 seed: the value the run would
    // yield next has to be the one seed 1999 yields after one draw per
    // segment. `999` would not match, and neither would 2000, so this pins
    // the (uint64_t)(cl.time * 1000) truncation as well as the reseed itself.
    let expect = rand_sequence(1999, obs[0].num_temp_entities as usize + 1);
    assert_eq!(
        obs[0].rand_next, expect[obs[0].num_temp_entities as usize],
        "cl.paused must reseed with (uint64_t)(cl.time * 1000) == 1999"
    );
}

#[test]
fn update_tents_stops_when_temp_table_fills() {
    let _g = lock();
    // `if (!ent) return;` -- CL_UpdateTEnts abandons the whole loop, not just
    // the current beam, the moment CL_NewTempEntity refuses. Two live beams
    // and only three visedict slots proves the early return: the second beam
    // must contribute nothing.
    // SAFETY: fixture reset/seeding over static storage, serialised.
    unsafe { ctest_cltent_reset() };
    let mut obs: Vec<UpdateObs> = Vec::new();
    for side in SIDES {
        // SAFETY: as above.
        unsafe {
            ctest_cltent_set_num_temp_entities(0);
            ctest_cltent_set_visedicts(3, 0);
            for idx in 0..2 {
                ctest_cltent_set_beam(
                    side,
                    idx,
                    7 + idx,
                    ctest_cltent_model(0),
                    10.0,
                    [0.0f32, 0.0, 0.0].as_ptr(),
                    [300.0f32, 0.0, 0.0].as_ptr(),
                );
            }
        }
        obs.push(run_update(side, 21, 8, MAX_BEAMS as usize));
    }
    assert_eq!(obs[0], obs[1]);
    assert_eq!(obs[0].status, GUARD_OK, "{}", obs[0].message);
    assert_eq!(
        obs[0].num_temp_entities, 3,
        "filled exactly cl_maxvisedicts"
    );
    assert_eq!(obs[0].numvisedicts, 3);
}

// ---------------------------------------------------------------------------
// CL_UpdateBeam

#[test]
fn update_beam_traces_before_touching_the_table() {
    let _g = lock();
    // CL_UpdateBeam's PSET_SCRIPT block runs unconditionally (quakedef.h:38
    // defines PSET_SCRIPT), and CL_TraceLine is an abort stub, so both sides
    // stop there with the beam table untouched. Weak on its own -- but it is
    // the only available check that the port kept the trace BEFORE the
    // override/allocate scan, which is the one ordering decision the function
    // makes that a reader could plausibly get wrong.
    // SAFETY: fixture reset/seeding over static storage, serialised.
    unsafe { ctest_cltent_reset() };

    let beam_size = {
        // SAFETY: a pure sizeof accessor.
        unsafe { ctest_cltent_beam_size() as usize }
    };
    let mut obs: Vec<(c_int, String, Vec<u8>)> = Vec::new();

    for side in SIDES {
        let mut start = [0.0f32, 0.0, 0.0];
        let mut end = [64.0f32, 0.0, 0.0];
        // SAFETY: CL_UpdateBeam takes vec3_t (float*) in/out params backed by
        // these locals; the abort stub's longjmp is caught by Host_Guard,
        // whose setjmp lives in stubs.c's pure C frame.
        let status = unsafe {
            ctest_cltent_set_beam(
                side,
                0,
                9,
                ctest_cltent_model(1),
                10.0,
                start.as_ptr(),
                end.as_ptr(),
            );
            guarded_update_beam(side, &mut start, &mut end)
        };
        let mut image = vec![0u8; beam_size];
        // SAFETY: byte-image read-back over static storage.
        unsafe { ctest_cltent_get_beam(side, 0, image.as_mut_ptr()) };
        obs.push((status, guard_message(status), image));
    }

    assert_eq!(obs[0], obs[1], "CL_UpdateBeam diverged");
    assert_eq!(obs[0].0, GUARD_SYS_ERROR);
    assert!(obs[0].1.contains("CL_TraceLine"), "{:?}", obs[0].1);
}

/// Runs one side's `CL_UpdateBeam` inside `Host_Guard`.
///
/// # Safety
/// `start`/`end` must be valid three-float buffers; callers hold `TEST_LOCK`.
unsafe fn guarded_update_beam(side: c_int, start: &mut [f32; 3], end: &mut [f32; 3]) -> c_int {
    struct Arg {
        side: c_int,
        start: *mut c_float,
        end: *mut c_float,
    }

    extern "C" fn invoke(p: *mut core::ffi::c_void) {
        // SAFETY: `p` is the `Arg` Host_Guard was handed below, still live on
        // the calling frame.
        let a = unsafe { &*(p as *const Arg) };
        // SAFETY: the model handle is a fixture sentinel that neither side
        // dereferences before CL_TraceLine aborts.
        unsafe {
            let m = ctest_cltent_model(1);
            let trail = c"TE_BEAM".as_ptr();
            let impact = c"TE_BEAM_END".as_ptr();
            if a.side == 1 {
                c_ref_CL_UpdateBeam(m, trail, impact, 9, a.start, a.end);
            } else {
                CL_UpdateBeam(m, trail, impact, 9, a.start, a.end);
            }
        }
    }

    extern "C" {
        fn Host_Guard(
            f: extern "C" fn(*mut core::ffi::c_void),
            arg: *mut core::ffi::c_void,
        ) -> c_int;
    }

    let mut arg = Arg {
        side,
        start: start.as_mut_ptr(),
        end: end.as_mut_ptr(),
    };
    // SAFETY: `arg` outlives the guarded call; Host_Guard's setjmp is in C.
    unsafe { Host_Guard(invoke, &mut arg as *mut Arg as *mut core::ffi::c_void) }
}
