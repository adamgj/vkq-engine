//! Differential/characterization gate for `Quake/cl_parse.c` -- the
//! server-message parser. Rust migration Phase 7, M7, task T7.3.
//!
//! The oracle fixture is `stubs/cl_parse_ref.c`; read its module doc first.
//! The short version: `cl_parse_glue.c` is `#ifdef USE_RUST_HOST` and
//! `cl_main.c` is an oracle source, so that file owns the plain twins of
//! `svc_strings`, `cl_lightstyle`, `cl_shownet`, all 21 `ClParse_Glue_*`
//! trampolines and three `cl_main.c` functions, and every seeder writes both
//! sides in one call.
//!
//! ## What makes a comparison here non-vacuous
//!
//! `cl` and `cls` are C-owned for this task (ADR-007 -- that row closes in
//! T7.4 with `cl_main.c`), so the two sides read two *different* copies:
//! the port reads `stubs.c`'s plain pair, the oracle reads `cl_main.c`'s
//! `c_ref_` pair. Both are zero from static init in a link that never runs
//! `CL_Init`, and a bit-exact differential passes happily when both sides do
//! nothing to nothing. `ctest_clparse_reset` therefore publishes a live
//! starting state into both -- a 64-entry entity array, a 64-slot static list,
//! a scoreboard, six shared model and sound handles, an 8KB `cls.message`,
//! `cl.time = 1.5`, `cls.signon = 2`, protocol `PROTOCOL_FITZQUAKE` -- and
//! every test below asserts something *positive* (a non-zero `readcount`, a
//! specific guard status and message, a value that actually changed)
//! alongside the cross-side comparison.
//!
//! Two shared-object traps are handled explicitly. `vid` and
//! `mod_known`/`mod_numknown` live in `stubs.c` and are single objects both
//! sides write, so `vid.recalc_refdef` is cleared between the two runs rather
//! than merely read afterwards; and the console log is cleared between runs so
//! the line sequences are per-side. Running one side before snapshotting the
//! other is exactly the mistake that makes a differential vacuous.
//!
//! ## The abort-stub ceiling, stated as a limit and not as coverage
//!
//! `cl_parse.c` reaches a dozen functions this harness implements as
//! unconditional `Sys_Error` stubs. Every driver enters through `Host_Guard`,
//! so reaching one is an *observation*: the suite compares which stub each
//! side reached, with what message, and how much of `net_message` each side
//! had consumed when it got there. For the Rust side that round trip is the
//! whole of ADR-009 -- trampoline, `Host_Guard`, `CLPARSE_RAISE_GUARD`,
//! `ClParse_Raise`, `Host_Reraise` -- so an arm that "only aborts" still
//! proves the raise topology end to end.
//!
//! What it does NOT prove is what the real callee would have done. The
//! largest casualty is `CL_ParseServerInfo`: it calls `CL_ClearState` at
//! `cl_parse.c:945`, whose `CL_FreeState` hits the `PR_ClearProgs` abort stub,
//! so the protocol/pext negotiation, gamedir switch and precache loops after
//! that line are NOT differentially covered here. `svc_particle`,
//! `svc_centerprint`, `svc_finale`, `svc_cutscene`, `svc_fog`,
//! `svc_achievement`, `svc_skybox`, `svc_updatecolors`, `svc_setpause` and the
//! three `svcdp_*particles` opcodes stop at their first stub too. Lifting any
//! of that means turning shared abort stubs into no-ops, which would delete
//! the "reached a module that is not an oracle source" tripwire for every
//! other suite in the harness; it is deliberately not done, and is carried as
//! a coverage gap instead.
//!
//! ## Command-buffer divergence, avoided rather than asserted
//!
//! `Cbuf_AddText`/`Cmd_ExecuteString` resolve to the Rust cvar/cmd port on the
//! plain side and to `cmd.c` on the oracle side -- two separate command
//! tables. `svc_stufftext`, `svc_sellscreen` and `svc_bf` are therefore driven
//! only for their read-side effects (`readcount`, `cl`/`cls`), and the tests
//! that touch them say so.
//!
//! ## ADR-010
//!
//! `cl_parse.c`'s only libm call is the `fabs` in `CL_ParseUpdate`'s lerp
//! bandaid (`cl_parse.c:500`), reached when a baselined entity moves more than
//! 100 units on any axis in one update. `update_lerp_bandaid_threshold` drives
//! both sides of that comparison at 13.3 fixed-point coordinates that land
//! just under and just over 100, and compares the resulting `entity_t` images
//! bit for bit.

use core::ffi::{c_char, c_double, c_float, c_int, c_uint};
use std::collections::BTreeSet;
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

// ---------------------------------------------------------------------------
// protocol.h opcode numbers.

const SVC_NOP: u8 = 1;
const SVC_DISCONNECT: u8 = 2;
const SVC_UPDATESTAT: u8 = 3;
const SVC_VERSION: u8 = 4;
const SVC_SETVIEW: u8 = 5;
const SVC_SOUND: u8 = 6;
const SVC_TIME: u8 = 7;
const SVC_PRINT: u8 = 8;
const SVC_SETANGLE: u8 = 10;
const SVC_LIGHTSTYLE: u8 = 12;
const SVC_UPDATENAME: u8 = 13;
const SVC_UPDATEFRAGS: u8 = 14;
const SVC_CLIENTDATA: u8 = 15;
const SVC_STOPSOUND: u8 = 16;
const SVC_UPDATECOLORS: u8 = 17;
const SVC_DAMAGE: u8 = 19;
const SVC_SPAWNSTATIC: u8 = 20;
const SVCFTE_SPAWNSTATIC2: u8 = 21;
const SVC_SPAWNBASELINE: u8 = 22;
const SVC_TEMP_ENTITY: u8 = 23;
const SVC_SIGNONNUM: u8 = 25;
const SVC_KILLEDMONSTER: u8 = 27;
const SVC_FOUNDSECRET: u8 = 28;
const SVC_SPAWNSTATICSOUND: u8 = 29;
const SVC_INTERMISSION: u8 = 30;
const SVC_SPAWNBASELINE2: u8 = 42;
const SVC_SPAWNSTATIC2: u8 = 43;
const SVC_SPAWNSTATICSOUND2: u8 = 44;
const SVC_LOCALSOUND: u8 = 56;
const SVCDP_UPDATESTATBYTE: u8 = 51;
const SVCDP_PRECACHE: u8 = 54;
const SVCFTE_SPAWNBASELINE2: u8 = 66;
const SVCFTE_UPDATESTATSTRING: u8 = 78;
const SVCFTE_UPDATESTATFLOAT: u8 = 79;
const SVCFTE_CGAMEPACKET: u8 = 83;
const SVCFTE_VOICECHAT: u8 = 84;
const SVCFTE_SETANGLEDELTA: u8 = 85;
const SVCFTE_UPDATEENTITIES: u8 = 86;

/// The six opcodes that reach `CL_ParseBaseline` and then
/// `ent->model = cl.model_precache[ent->baseline.modelindex]` (cl_parse.c:1566)
/// with no `MAX_MODELS` guard on the index. `B_LARGEMODEL` lets the wire put a
/// full 16-bit value there, and a truncated message puts `(unsigned short)-1`
/// there, so the load can land half a megabyte past the end of `cl`.
///
/// Both implementations perform exactly that load, so this is not a port
/// difference -- but the oracle's `cl` and the port's `cl` are two distinct
/// objects with different neighbours in the image, so what comes back is
/// unrelated memory and the two sides disagree for reasons the differential
/// cannot attribute. The generated corpora therefore avoid these opcodes and
/// drive baselines from `spawnbaseline_variants_and_spawnstatic_sound`, which
/// uses in-range indices. (`CL_ParseUpdate` guards its own model index, and
/// `CL_ParseStaticSound`'s unguarded `cl.sound_precache[sound_num]` is passed
/// to a `S_StaticSound` that ignores it while the mixer is stopped.)
const BASELINE_SPAWNERS: [u8; 6] = [
    SVC_SPAWNSTATIC,
    SVCFTE_SPAWNSTATIC2,
    SVC_SPAWNBASELINE,
    SVC_SPAWNBASELINE2,
    SVC_SPAWNSTATIC2,
    SVCFTE_SPAWNBASELINE2,
];

/// `protocol.h` -- the extension bits the "not active" arms test.
const PEXT2_REPLACEMENTDELTAS: c_uint = 0x0000_0008;
const PEXT2_PREDINFO: c_uint = 0x0000_0020;
const PEXT2_VOICECHAT: c_uint = 0x0000_0002;
const PEXT1_CSQC: c_uint = 0x4000_0000;

/// `quakedef.h` / `client.h` limits this suite indexes against.
const MAX_LIGHTSTYLES: c_int = 64;
const MAX_SCOREBOARD: c_int = 16;
const MAX_CL_STATS: c_int = 256;
const STAT_HEALTH: c_int = 0;
const STAT_ITEMS: c_int = 15;
const STAT_MONSTERS: c_int = 14;
const STAT_SECRETS: c_int = 13;

/// `stubs.c:1465-1467` -- `Host_Guard`'s status codes.
const GUARD_OK: c_int = 0;
const GUARD_HOST_ERROR: c_int = 1;
const GUARD_SYS_ERROR: c_int = 2;

/// The fixture's entity/static array sizes (`cl_parse_ref.c`).
const FIXTURE_ENTITIES: c_int = 64;
const FIXTURE_STATICS: c_int = 64;

const SIDES: [c_int; 2] = [1 /* oracle */, 0 /* rust */];

fn side_name(side: c_int) -> &'static str {
    if side == 1 {
        "C"
    } else {
        "Rust"
    }
}

extern "C" {
    // stubs/cl_parse_ref.c -- seeders (each writes BOTH sides)
    fn ctest_clparse_reset();
    fn ctest_clparse_set_shownet(v: c_float);
    fn ctest_clparse_set_protocol(protocol: c_int, flags: c_uint, pext1: c_uint, pext2: c_uint);
    fn ctest_clparse_set_time(
        time: c_double,
        oldtime: c_double,
        mtime0: c_double,
        mtime1: c_double,
    );
    fn ctest_clparse_set_counts(
        maxclients: c_int,
        viewentity: c_int,
        num_entities: c_int,
        num_statics: c_int,
    );
    fn ctest_clparse_set_conn(
        state: c_int,
        signon: c_int,
        demoplayback: c_int,
        demorecording: c_int,
    );
    fn ctest_clparse_seed_precaches(nummodels: c_int, numsounds: c_int);
    fn ctest_clparse_set_model_info(
        idx: c_int,
        numframes: c_int,
        flags: c_int,
        ty: c_int,
        synctype: c_int,
    );
    fn ctest_clparse_set_vid_recalc(v: c_int);
    fn ctest_clparse_set_mod_numknown(n: c_int);
    fn ctest_clparse_set_particle_name(idx: c_int, name: *const c_char);
    fn ctest_clparse_set_statstring(idx: c_int, value: *const c_char);
    fn ctest_clparse_set_stat(idx: c_int, value: c_int);
    fn ctest_clparse_set_items(items: c_int);

    // stubs/cl_parse_ref.c -- read-backs
    fn ctest_clparse_cl_image_size() -> c_int;
    fn ctest_clparse_get_cl_image(side: c_int, out: *mut u8);
    fn ctest_clparse_cls_image_size() -> c_int;
    fn ctest_clparse_get_cls_image(side: c_int, out: *mut u8);
    fn ctest_clparse_entity_size() -> c_int;
    fn ctest_clparse_get_entity(side: c_int, idx: c_int, out: *mut u8);
    fn ctest_clparse_get_static_entity(side: c_int, idx: c_int, out: *mut u8);
    fn ctest_clparse_get_viewent(side: c_int, out: *mut u8);
    fn ctest_clparse_score_size() -> c_int;
    fn ctest_clparse_get_score(side: c_int, idx: c_int, out: *mut u8);
    fn ctest_clparse_lightstyle_size() -> c_int;
    fn ctest_clparse_get_lightstyle(side: c_int, idx: c_int, out: *mut u8);
    fn ctest_clparse_get_statstring(side: c_int, idx: c_int) -> *const c_char;
    fn ctest_clparse_get_particle_name(side: c_int, idx: c_int) -> *const c_char;
    fn ctest_clparse_get_particle_index(side: c_int, idx: c_int) -> c_int;
    fn ctest_clparse_get_message_size(side: c_int) -> c_int;
    fn ctest_clparse_get_message_data(side: c_int) -> *const u8;
    fn ctest_clparse_get_readcount(side: c_int) -> c_int;
    fn ctest_clparse_get_badread(side: c_int) -> c_int;
    fn ctest_clparse_get_vid_recalc() -> c_int;
    fn ctest_clparse_get_stat(side: c_int, idx: c_int) -> c_int;

    // stubs/cl_parse_ref.c -- Host_Guard-entered drivers
    fn ctest_clparse_begin_reading(side: c_int);
    fn ctest_clparse_parse_server_message(side: c_int) -> c_int;
    fn ctest_clparse_entity_num(side: c_int, num: c_int, outidx: *mut c_int) -> c_int;
    fn ctest_clparse_parse_local_sound(side: c_int) -> c_int;
    fn ctest_clparse_new_translation(side: c_int, slot: c_int) -> c_int;
    fn ctest_clparse_register_particles(side: c_int) -> c_int;

    // stubs/sv_user_ref.c -- seeds BOTH net_message buffers.
    fn ctest_svuser_load_message(data: *const u8, len: c_int);

    // stubs.c
    fn ctest_sys_error_message() -> *const c_char;
    fn ctest_host_error_message() -> *const c_char;
    fn ctest_blas_free_reset();
    fn ctest_blas_free_count() -> c_int;
    fn ctest_pscript_model_effects_reset();
    fn ctest_pscript_model_effects_count() -> c_int;
    fn ctest_pscript_last_model_name() -> *const c_char;
    fn ctest_clear_con_log();
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

/// A NUL-terminated C string read-back, or `None` for a NULL slot.
///
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

/// The console lines emitted since the last clear. Capped so a pathological
/// fuzz message cannot turn one failure into a megabyte of diff; the count is
/// compared separately and is not capped.
fn con_log() -> (c_int, Vec<String>) {
    // SAFETY: stubs.c getters over static storage; caller holds TEST_LOCK.
    unsafe {
        let n = ctest_con_log_len();
        let shown = n.clamp(0, 64);
        let lines = (0..shown)
            .map(|i| {
                core::ffi::CStr::from_ptr(ctest_con_log_get(i))
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        (n, lines)
    }
}

// ---------------------------------------------------------------------------
// Message construction. Coordinates and angles are written as raw wire
// encodings, never as a float this test re-derives, so the decoded value is
// something the differential OBSERVES rather than something it predicts.

#[derive(Default, Clone)]
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
    fn long(mut self, v: i32) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn float(mut self, v: f32) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn string(mut self, s: &str) -> Self {
        self.0.extend_from_slice(s.as_bytes());
        self.0.push(0);
        self
    }
    /// Three 13.3 fixed-point coords (`protocolflags == 0`).
    fn coords16(self, a: i16, b: i16, c: i16) -> Self {
        self.short(a).short(b).short(c)
    }
}

// ---------------------------------------------------------------------------
// Observations. Everything float-shaped is compared as raw bytes, so a NaN
// payload or a -0.0 is a difference rather than an accidental equality.

#[derive(Clone, PartialEq, Eq)]
struct Obs {
    status: c_int,
    message: String,
    readcount: c_int,
    badread: c_int,
    vid_recalc: c_int,
    cl_image: Vec<u8>,
    cls_image: Vec<u8>,
    entities: Vec<u8>,
    statics: Vec<u8>,
    viewent: Vec<u8>,
    scores: Vec<u8>,
    lightstyles: Vec<u8>,
    statss: Vec<Option<String>>,
    particles: Vec<(Option<String>, c_int)>,
    message_bytes: Vec<u8>,
    con_count: c_int,
    con_lines: Vec<String>,
    blas_frees: c_int,
}

impl core::fmt::Debug for Obs {
    /// The byte images are megabyte-scale and useless in a panic message; the
    /// named fields plus a per-image equality marker are what localises a
    /// failure, and the dedicated comparators below report the exact index.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Obs")
            .field("status", &self.status)
            .field("message", &self.message)
            .field("readcount", &self.readcount)
            .field("badread", &self.badread)
            .field("vid_recalc", &self.vid_recalc)
            .field("statss", &self.statss)
            .field("particles", &self.particles)
            .field("message_bytes", &self.message_bytes)
            .field("con_count", &self.con_count)
            .field("con_lines", &self.con_lines)
            .finish_non_exhaustive()
    }
}

fn snapshot(side: c_int, status: c_int) -> Obs {
    // SAFETY: every callee is a fixture read-back in stubs/cl_parse_ref.c or
    // stubs.c operating on static storage with process lifetime; the buffers
    // handed in are sized from the matching *_size() query; TEST_LOCK is held.
    unsafe {
        let cl_sz = ctest_clparse_cl_image_size() as usize;
        let mut cl_image = vec![0u8; cl_sz];
        ctest_clparse_get_cl_image(side, cl_image.as_mut_ptr());

        let cls_sz = ctest_clparse_cls_image_size() as usize;
        let mut cls_image = vec![0u8; cls_sz];
        ctest_clparse_get_cls_image(side, cls_image.as_mut_ptr());

        let ent_sz = ctest_clparse_entity_size() as usize;
        let mut entities = vec![0u8; ent_sz * FIXTURE_ENTITIES as usize];
        for i in 0..FIXTURE_ENTITIES {
            ctest_clparse_get_entity(side, i, entities.as_mut_ptr().add(ent_sz * i as usize));
        }
        let mut statics = vec![0u8; ent_sz * FIXTURE_STATICS as usize];
        for i in 0..FIXTURE_STATICS {
            ctest_clparse_get_static_entity(side, i, statics.as_mut_ptr().add(ent_sz * i as usize));
        }
        let mut viewent = vec![0u8; ent_sz];
        ctest_clparse_get_viewent(side, viewent.as_mut_ptr());

        let sc_sz = ctest_clparse_score_size() as usize;
        let mut scores = vec![0u8; sc_sz * MAX_SCOREBOARD as usize];
        for i in 0..MAX_SCOREBOARD {
            ctest_clparse_get_score(side, i, scores.as_mut_ptr().add(sc_sz * i as usize));
        }

        let ls_sz = ctest_clparse_lightstyle_size() as usize;
        let mut lightstyles = vec![0u8; ls_sz * MAX_LIGHTSTYLES as usize];
        for i in 0..MAX_LIGHTSTYLES {
            ctest_clparse_get_lightstyle(side, i, lightstyles.as_mut_ptr().add(ls_sz * i as usize));
        }

        let statss = (0..MAX_CL_STATS)
            .map(|i| opt_cstr(ctest_clparse_get_statstring(side, i)))
            .collect();

        // The precache table is 2048 entries; only the low slots are ever
        // written here, and scanning all of them per iteration dominates the
        // fuzz runtime for no added signal.
        let particles = (0..32)
            .map(|i| {
                (
                    opt_cstr(ctest_clparse_get_particle_name(side, i)),
                    ctest_clparse_get_particle_index(side, i),
                )
            })
            .collect();

        let msg_len = ctest_clparse_get_message_size(side).max(0) as usize;
        let msg_ptr = ctest_clparse_get_message_data(side);
        let message_bytes = core::slice::from_raw_parts(msg_ptr, msg_len).to_vec();

        let (con_count, con_lines) = con_log();

        Obs {
            status,
            message: guard_message(status),
            readcount: ctest_clparse_get_readcount(side),
            badread: ctest_clparse_get_badread(side),
            vid_recalc: ctest_clparse_get_vid_recalc(),
            cl_image,
            cls_image,
            entities,
            statics,
            viewent,
            scores,
            lightstyles,
            statss,
            particles,
            message_bytes,
            con_count,
            con_lines,
            blas_frees: ctest_blas_free_count(),
        }
    }
}

/// Runs one side of `CL_ParseServerMessage` over `msg` and records everything
/// the harness can see. `setup` runs after `ctest_clparse_reset`, so a test can
/// seed state without the reset undoing it.
fn run_parse(side: c_int, msg: &[u8], setup: &dyn Fn()) -> Obs {
    // SAFETY: fixture entry points over static storage, serialised by
    // TEST_LOCK. ctest_clparse_parse_server_message enters through Host_Guard,
    // so an abort stub's longjmp is caught in a pure C frame (ADR-009).
    unsafe {
        ctest_clparse_reset();
        setup();
        // Cleared per side, not per test: `vid` and the console log are single
        // shared objects, so reading them after both runs would compare a
        // side's output against the union of both.
        ctest_clparse_set_vid_recalc(0);
        ctest_clear_con_log();
        ctest_blas_free_reset();
        ctest_svuser_load_message(msg.as_ptr(), msg.len() as c_int);

        let status = ctest_clparse_parse_server_message(side);
        snapshot(side, status)
    }
}

/// Drives both sides over one message, asserts they agree field by field, and
/// returns the (now identical) observation so the caller can make its own
/// positive assertions.
fn parse_both(msg: &[u8], setup: &dyn Fn()) -> Obs {
    let c = run_parse(SIDES[0], msg, setup);
    let rust = run_parse(SIDES[1], msg, setup);
    compare(&c, &rust, &format!("{:02x?}", &msg[..msg.len().min(24)]));
    c
}

fn parse_both_default(msg: &[u8]) -> Obs {
    parse_both(msg, &|| {})
}

/// Field-by-field so a failure names the structure that diverged and, for the
/// byte images, the first differing offset -- an `assert_eq!` on the whole
/// `Obs` would print two multi-megabyte blobs.
fn compare(c: &Obs, rust: &Obs, what: &str) {
    assert_eq!(
        c.status, rust.status,
        "guard status diverged for {what}: C {:?} / Rust {:?}",
        c.message, rust.message
    );
    assert_eq!(c.message, rust.message, "raise message diverged for {what}");
    assert_eq!(
        c.readcount, rust.readcount,
        "msg_readcount diverged for {what}"
    );
    assert_eq!(c.badread, rust.badread, "msg_badread diverged for {what}");
    assert_eq!(
        c.vid_recalc, rust.vid_recalc,
        "vid.recalc_refdef diverged for {what}"
    );
    assert_eq!(
        c.blas_frees, rust.blas_frees,
        "R_FreeEntityBLAS call count diverged for {what}"
    );
    assert_eq!(c.statss, rust.statss, "cl.statss diverged for {what}");
    assert_eq!(
        c.particles, rust.particles,
        "cl.particle_precache diverged for {what}"
    );
    assert_eq!(
        c.message_bytes, rust.message_bytes,
        "cls.message diverged for {what}"
    );
    assert_eq!(
        c.con_count, rust.con_count,
        "console line count diverged for {what}"
    );
    assert_eq!(
        c.con_lines, rust.con_lines,
        "console output diverged for {what}"
    );

    let images: [(&str, &Vec<u8>, &Vec<u8>, usize); 6] = [
        ("cl", &c.cl_image, &rust.cl_image, 1),
        ("cls", &c.cls_image, &rust.cls_image, 1),
        ("cl.entities", &c.entities, &rust.entities, 1),
        ("cl.static_entities", &c.statics, &rust.statics, 1),
        ("cl.scores", &c.scores, &rust.scores, 1),
        ("cl_lightstyle", &c.lightstyles, &rust.lightstyles, 1),
    ];
    for (name, a, b, _) in images {
        if a != b {
            let off = a.iter().zip(b.iter()).position(|(x, y)| x != y);
            panic!(
                "{name} image diverged for {what}: first difference at byte {off:?} \
                 (C {:?} vs Rust {:?})",
                off.map(|i| a[i]),
                off.map(|i| b[i]),
            );
        }
    }
    assert_eq!(c.viewent, rust.viewent, "cl.viewent diverged for {what}");
}

// ===========================================================================
// Structural gate: the ADR-009 status table.
//
// cl_parse_glue.c enumerates 34 CLPARSE_* codes and stubs/cl_parse_ref.c
// mirrors ClParse_Raise arm for arm. Codes 2..30 are Host_Error, 31..33 are
// Host_EndGame, and 1 is the re-raise. The tests below reach a raise arm by
// its actual trigger and assert the exact message text, which is the only
// thing that proves the port's status code and the glue's arm agree -- a port
// that returned the wrong code would raise the wrong message, and the oracle
// (which formats the same text inline in cl_parse.c) would not.
// ===========================================================================

#[test]
fn raise_illegible_names_the_previous_command() {
    let _g = lock();
    // 35 is svc_showpic_dp -- present in svc_strings, absent from the switch,
    // so it is the default arm. lastcmd is 0 on the first opcode, hence
    // "svc_bad"; the second case proves lastcmd is carried, which is the only
    // reason the glue's `s` out-pointer exists.
    let obs = parse_both_default(&Msg::default().byte(35).0);
    assert_eq!(obs.status, GUARD_HOST_ERROR);
    assert_eq!(
        obs.message,
        "Illegible server message 35, previous was svc_bad"
    );
    assert_eq!(obs.readcount, 1);

    let obs = parse_both_default(&Msg::default().byte(SVC_NOP).byte(36).0);
    assert_eq!(
        obs.message,
        "Illegible server message 36, previous was svc_nop"
    );
    assert_eq!(obs.readcount, 2, "the nop was consumed before the bad byte");
}

#[test]
fn raise_disconnect_is_an_endgame() {
    let _g = lock();
    let obs = parse_both_default(&Msg::default().byte(SVC_DISCONNECT).0);
    assert_eq!(
        obs.status, GUARD_HOST_ERROR,
        "Host_EndGame forwards to Host_Error in this harness"
    );
    assert_eq!(obs.message, "Host_EndGame: Server disconnected\n");
}

#[test]
fn raise_bad_message_after_a_truncated_arm() {
    let _g = lock();
    // CLPARSE_ERR_BADMESSAGE (cl_parse.c:1806) fires at the TOP of the
    // dispatch loop, not at the end of the message: an arm that reads past
    // cursize sets msg_badread, returns normally, and the next iteration
    // raises. Running off the end at the loop's own MSG_ReadByte is the
    // ordinary exit instead, which is why a clean parse ends with badread == 1
    // and no raise. svc_updatestat with a 1-byte payload is the shortest way
    // to make an arm overrun.
    let obs = parse_both_default(&Msg::default().byte(SVC_UPDATESTAT).byte(1).0);
    assert_eq!(obs.status, GUARD_HOST_ERROR, "{}", obs.message);
    assert_eq!(obs.message, "CL_ParseServerMessage: Bad server message");
    assert_eq!(obs.badread, 1);
}

#[test]
fn raise_bad_version() {
    let _g = lock();
    let obs = parse_both_default(&Msg::default().byte(SVC_VERSION).long(12345).0);
    assert_eq!(obs.status, GUARD_HOST_ERROR);
    assert_eq!(
        obs.message,
        "Server returned version 12345, not 15 or 666 or 999"
    );

    // and the accepted values land in cl.protocol rather than raising
    for good in [15i32, 666, 999] {
        let obs = parse_both_default(&Msg::default().byte(SVC_VERSION).long(good).0);
        assert_eq!(obs.status, GUARD_OK, "protocol {good}: {}", obs.message);
        assert_eq!(obs.readcount, 5);
    }
}

#[test]
fn raise_entitynum_out_of_range() {
    let _g = lock();
    // cl.max_edicts is the fixture's 64.
    for (num, expect) in [(-1i32, "-1"), (64, "64"), (1000, "1000")] {
        let mut idx: c_int = 0;
        // SAFETY: guarded fixture driver over static storage; TEST_LOCK held.
        let statuses: Vec<(c_int, String)> = SIDES
            .iter()
            .map(|&side| unsafe {
                ctest_clparse_reset();
                let s = ctest_clparse_entity_num(side, num, &mut idx);
                (s, guard_message(s))
            })
            .collect();
        assert_eq!(statuses[0], statuses[1], "CL_EntityNum({num}) diverged");
        assert_eq!(statuses[0].0, GUARD_HOST_ERROR);
        assert_eq!(
            statuses[0].1,
            format!("CL_EntityNum: {expect} is an invalid number")
        );
    }
}

#[test]
fn entity_num_grows_num_entities() {
    let _g = lock();
    // The positive half: an in-range index returns the matching slot on both
    // sides and pushes cl.num_entities up to num+1.
    for num in [0i32, 1, 8, 63] {
        let mut idx_c: c_int = -99;
        let mut idx_rust: c_int = -99;
        // SAFETY: guarded fixture drivers over static storage; TEST_LOCK held.
        let (sc, sr) = unsafe {
            ctest_clparse_reset();
            ctest_clparse_set_counts(4, 1, 0, 0);
            let a = ctest_clparse_entity_num(SIDES[0], num, &mut idx_c);
            ctest_clparse_reset();
            ctest_clparse_set_counts(4, 1, 0, 0);
            let b = ctest_clparse_entity_num(SIDES[1], num, &mut idx_rust);
            (a, b)
        };
        assert_eq!(sc, GUARD_OK, "C raised: {}", guard_message(sc));
        assert_eq!(sr, GUARD_OK, "Rust raised: {}", guard_message(sr));
        assert_eq!(idx_c, num, "C returned the wrong slot");
        assert_eq!(idx_rust, num, "Rust returned the wrong slot");
    }
}

#[test]
fn raise_signon_regression() {
    let _g = lock();
    // cls.signon is 2 after reset, so 1 and 2 both raise and 3 is accepted.
    for i in [0u8, 1, 2] {
        let obs = parse_both_default(&Msg::default().byte(SVC_SIGNONNUM).byte(i).0);
        assert_eq!(obs.status, GUARD_HOST_ERROR, "signon {i}");
        assert_eq!(obs.message, format!("Received signon {i} when at 2"));
    }
}

#[test]
fn raise_updatename_frags_colors_bounds() {
    let _g = lock();
    // cl.maxclients is the fixture's 4; the guard is `i >= cl.maxclients`.
    let cases: [(u8, &str); 3] = [
        (SVC_UPDATENAME, "svc_updatename"),
        (SVC_UPDATEFRAGS, "svc_updatefrags"),
        (SVC_UPDATECOLORS, "svc_updatecolors"),
    ];
    for (op, name) in cases {
        let obs = parse_both_default(&Msg::default().byte(op).byte(4).0);
        assert_eq!(obs.status, GUARD_HOST_ERROR, "{name}: {}", obs.message);
        assert_eq!(
            obs.message,
            format!("CL_ParseServerMessage: {name} > MAX_SCOREBOARD")
        );
        assert_eq!(obs.readcount, 2, "{name} read the opcode and the index");
    }
}

#[test]
fn raise_extension_not_active() {
    let _g = lock();
    // Every "extension not active" arm, driven with pext1/pext2 zero (the
    // reset default). These are eight of the 34 status codes and they share a
    // shape, so a port that mixed two of them up would show up here as the
    // wrong message text.
    let cases: [(u8, &str); 8] = [
        (
            SVCDP_PRECACHE,
            "Received svcdp_precache but extension not active",
        ),
        (
            SVCDP_UPDATESTATBYTE,
            "Received svcdp_updatestatbyte but extension not active",
        ),
        (
            SVCFTE_UPDATESTATSTRING,
            "Received svcfte_updatestatstring but extension not active",
        ),
        (
            SVCFTE_UPDATESTATFLOAT,
            "Received svcfte_updatestatfloat but extension not active",
        ),
        (
            SVCFTE_SPAWNSTATIC2,
            "Received svcfte_spawnstatic2 but extension not active",
        ),
        (
            SVCFTE_SPAWNBASELINE2,
            "Received svcfte_spawnbaseline2 but extension not active",
        ),
        (
            SVCFTE_UPDATEENTITIES,
            "Received svcfte_updateentities but extension not active",
        ),
        (
            SVCFTE_VOICECHAT,
            "Received svcfte_voicechat but extension not active",
        ),
    ];
    for (op, expect) in cases {
        let obs = parse_both_default(&Msg::default().byte(op).0);
        assert_eq!(obs.status, GUARD_HOST_ERROR, "opcode {op}: {}", obs.message);
        assert_eq!(obs.message, expect, "opcode {op}");
        assert_eq!(
            obs.readcount, 1,
            "opcode {op} raised before reading a payload"
        );
    }
}

#[test]
fn raise_cgamepacket_without_csqc() {
    let _g = lock();
    // The first of the two arms: PEXT1_CSQC is not negotiated at all. This is
    // the ninth "extension not active" code and the only one not in
    // raise_extension_not_active, because it needs its own opcode ordering.
    let obs = parse_both_default(&Msg::default().byte(SVCFTE_CGAMEPACKET).0);
    assert_eq!(obs.status, GUARD_HOST_ERROR, "{}", obs.message);
    assert_eq!(
        obs.message,
        "Received svcfte_cgamepacket but extension not active"
    );

    // PEXT1_CSQC active but cl.qcvm.extfuncs.CSQC_Parse_Event still 0 -- the
    // second of the two arms, and the only CLPARSE_* code whose message ends
    // in a newline.
    let obs = parse_both(&Msg::default().byte(SVCFTE_CGAMEPACKET).0, &|| {
        // SAFETY: dual-side seeder over static storage; TEST_LOCK held.
        unsafe { ctest_clparse_set_protocol(666, 0, PEXT1_CSQC, 0) };
    });
    assert_eq!(obs.status, GUARD_HOST_ERROR);
    assert_eq!(
        obs.message,
        "CSQC_Parse_Event: Missing or incompatible CSQC\n"
    );
}

#[test]
fn voicechat_consumes_its_payload_when_active() {
    let _g = lock();
    // The accepting half of the same opcode: three bytes, a length short, then
    // that many bytes. Driving it proves the arm's read count, which is the
    // only thing it produces.
    let obs = parse_both(
        &Msg::default()
            .byte(SVCFTE_VOICECHAT)
            .byte(1)
            .byte(2)
            .byte(3)
            .short(5)
            .byte(9)
            .byte(9)
            .byte(9)
            .byte(9)
            .byte(9)
            .0,
        &|| {
            // SAFETY: dual-side seeder; TEST_LOCK held.
            unsafe { ctest_clparse_set_protocol(666, 0, 0, PEXT2_VOICECHAT) };
        },
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(
        obs.readcount, 11,
        "opcode + 3 bytes + short + 5 payload bytes"
    );
    // Every message that runs to completion ends with the loop's own
    // MSG_ReadByte falling off the end (net_msg.c sets msg_badread before
    // returning -1), so badread == 1 is what a *clean* parse looks like here.
    assert_eq!(obs.badread, 1);
}

// ===========================================================================
// Arms that run to completion and change comparable state.
// ===========================================================================

#[test]
fn lightstyle_computes_length_peak_and_average() {
    let _g = lock();
    // The richest pure-computation arm in the file: a strlcpy, a length, a
    // running total, a q_max fold and an integer average, all landing in the
    // plain cl_lightstyle this task owns.
    let cases: [(u8, &str); 5] = [
        (0, "abcdefghijklmnopqrstuvwxyz"),
        (1, ""),
        (2, "z"),
        (3, "mmmmmmmmmm"),
        (63, "aznaznaznazn"),
    ];
    for (idx, map) in cases {
        let obs = parse_both_default(&Msg::default().byte(SVC_LIGHTSTYLE).byte(idx).string(map).0);
        assert_eq!(obs.status, GUARD_OK, "style {idx}: {}", obs.message);
        assert_eq!(obs.readcount as usize, 2 + map.len() + 1);
    }

    // Positive check that the arm really wrote something, so the comparison
    // above is not two zeroed tables agreeing.
    let ls_sz = // SAFETY: fixture size query.
        unsafe { ctest_clparse_lightstyle_size() } as usize;
    let mut before = vec![0u8; ls_sz];
    let mut after = vec![0u8; ls_sz];
    // SAFETY: sized buffers, static fixture storage, TEST_LOCK held.
    unsafe {
        ctest_clparse_reset();
        ctest_clparse_get_lightstyle(SIDES[1], 5, before.as_mut_ptr());
        let m = Msg::default().byte(SVC_LIGHTSTYLE).byte(5).string("abcz").0;
        ctest_svuser_load_message(m.as_ptr(), m.len() as c_int);
        let s = ctest_clparse_parse_server_message(SIDES[1]);
        assert_eq!(s, GUARD_OK, "{}", guard_message(s));
        ctest_clparse_get_lightstyle(SIDES[1], 5, after.as_mut_ptr());
    }
    assert_ne!(
        before, after,
        "svc_lightstyle wrote nothing -- vacuous test"
    );
}

#[test]
fn lightstyle_index_overflow_is_a_sys_error() {
    let _g = lock();
    // cl_parse.c:1926 uses Sys_Error, not Host_Error, so this arm aborts in
    // BOTH implementations rather than going through ClParse_Raise -- the
    // "Sys_Error aborts, so Rust may call it directly" half of ADR-009.
    let obs = parse_both_default(&Msg::default().byte(SVC_LIGHTSTYLE).byte(64).0);
    assert_eq!(obs.status, GUARD_SYS_ERROR);
    assert_eq!(obs.message, "svc_lightstyle > MAX_LIGHTSTYLES");
}

#[test]
fn time_and_setview_and_setangle() {
    let _g = lock();
    let obs = parse_both_default(&Msg::default().byte(SVC_TIME).float(12.5).0);
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.readcount, 5);

    // With PEXT2_PREDINFO the arm eats two extra bytes for the input ack.
    let obs = parse_both(
        &Msg::default().byte(SVC_TIME).float(12.5).short(77).0,
        &|| {
            // SAFETY: dual-side seeder; TEST_LOCK held.
            unsafe { ctest_clparse_set_protocol(666, 0, 0, PEXT2_PREDINFO) };
        },
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.readcount, 7, "PEXT2_PREDINFO adds the sequence ack");

    let obs = parse_both_default(&Msg::default().byte(SVC_SETVIEW).short(37).0);
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.readcount, 3);

    // MSG_ReadAngle with protocolflags 0 is a byte per axis; the delta variant
    // is MSG_ReadAngle16 and accumulates instead of assigning.
    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_SETANGLE)
            .byte(64)
            .byte(128)
            .byte(255)
            .0,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.readcount, 4);

    let obs = parse_both_default(
        &Msg::default()
            .byte(SVCFTE_SETANGLEDELTA)
            .short(1000)
            .short(-2000)
            .short(3)
            .0,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.readcount, 7);
}

#[test]
fn counters_and_intermission() {
    let _g = lock();
    // svc_killedmonster / svc_foundsecret increment an int stat and mirror it
    // into the float table; svc_intermission also sets vid.recalc_refdef,
    // which is the shared-object trap this suite clears per side.
    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_KILLEDMONSTER)
            .byte(SVC_KILLEDMONSTER)
            .byte(SVC_FOUNDSECRET)
            .0,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    // SAFETY: fixture read-back; TEST_LOCK held.
    unsafe {
        assert_eq!(ctest_clparse_get_stat(SIDES[1], STAT_MONSTERS), 2);
        assert_eq!(ctest_clparse_get_stat(SIDES[1], STAT_SECRETS), 1);
    }

    let obs = parse_both_default(&Msg::default().byte(SVC_INTERMISSION).0);
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.vid_recalc, 1, "svc_intermission goes full screen");
}

#[test]
fn updatestat_int_byte_string_and_float() {
    let _g = lock();
    // svc_updatestat is always available; the three svcdp_/svcfte_ variants
    // need PEXT2_REPLACEMENTDELTAS, and the string variant frees whatever was
    // already in cl.statss[i], which is why the slot is pre-seeded.
    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_UPDATESTAT)
            .byte(STAT_HEALTH as u8)
            .long(75)
            .0,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    // SAFETY: fixture read-back; TEST_LOCK held.
    unsafe { assert_eq!(ctest_clparse_get_stat(SIDES[1], STAT_HEALTH), 75) };

    let with_deltas = || {
        // SAFETY: dual-side seeder; TEST_LOCK held.
        unsafe { ctest_clparse_set_protocol(666, 0, 0, PEXT2_REPLACEMENTDELTAS) };
    };

    let obs = parse_both(
        &Msg::default()
            .byte(SVCDP_UPDATESTATBYTE)
            .byte(STAT_HEALTH as u8)
            .byte(200)
            .0,
        &with_deltas,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    // SAFETY: fixture read-back; TEST_LOCK held.
    unsafe { assert_eq!(ctest_clparse_get_stat(SIDES[1], STAT_HEALTH), 200) };

    let obs = parse_both(
        &Msg::default()
            .byte(SVCFTE_UPDATESTATSTRING)
            .byte(9)
            .string("gadget")
            .0,
        &|| {
            with_deltas();
            // SAFETY: dual-side seeder; TEST_LOCK held.
            unsafe { ctest_clparse_set_statstring(9, c"previous".as_ptr()) };
        },
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(
        obs.statss[9].as_deref(),
        Some("gadget"),
        "the replaced string is the observable half of CL_ParseStatString"
    );

    // The top of the string slot range. The wire index is a byte, so 255 is
    // the largest reachable stat and MAX_CL_STATS itself is not; without this
    // the only thing pinning CL_ParseStatString's upper bound is the int path
    // in stat_index_out_of_range_is_ignored, and a `MAX_CL_STATS - 1` bound in
    // the port survives.
    let obs = parse_both(
        &Msg::default()
            .byte(SVCFTE_UPDATESTATSTRING)
            .byte((MAX_CL_STATS - 1) as u8)
            .string("topmost")
            .0,
        &with_deltas,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(
        obs.statss[(MAX_CL_STATS - 1) as usize].as_deref(),
        Some("topmost"),
        "stat 255 is in range and must be stored"
    );

    // A NULL/empty string clears the slot rather than storing "".
    let obs = parse_both(
        &Msg::default()
            .byte(SVCFTE_UPDATESTATSTRING)
            .byte(9)
            .string("")
            .0,
        &|| {
            with_deltas();
            // SAFETY: dual-side seeder; TEST_LOCK held.
            unsafe { ctest_clparse_set_statstring(9, c"previous".as_ptr()) };
        },
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);

    // The float variant truncates to int for cl.stats and keeps the float in
    // cl.statsf. Only in-range values are compared bit-for-bit: NaN and
    // out-of-range floats are the documented ADR-010 exception at
    // cl_parse.rs:cl_parse_stat_float (C's `(int)` is UB and x86-64 yields
    // INT_MIN; Rust's `as` saturates, which is also what arm64 C does), so a
    // differential assert there would encode one platform's answer.
    for v in [1.0f32, -3.75, 1e9, -1e9, 0.0, -0.5] {
        let obs = parse_both(
            &Msg::default()
                .byte(SVCFTE_UPDATESTATFLOAT)
                .byte(11)
                .float(v)
                .0,
            &with_deltas,
        );
        assert_eq!(obs.status, GUARD_OK, "float {v}: {}", obs.message);
        assert_eq!(obs.readcount, 6, "float {v}");
        // SAFETY: fixture read-back; TEST_LOCK held.
        unsafe {
            assert_eq!(
                ctest_clparse_get_stat(SIDES[1], 11),
                v as c_int,
                "cl.stats[11] holds the truncation of {v}"
            )
        };
    }

    // The excepted half, checked on the Rust side alone so the assertion says
    // what the port does rather than what one target's C does.
    for (v, want) in [
        (f32::NAN, 0),
        (f32::INFINITY, i32::MAX),
        (-1e30f32, i32::MIN),
    ] {
        let obs = run_parse(
            SIDES[1],
            &Msg::default()
                .byte(SVCFTE_UPDATESTATFLOAT)
                .byte(11)
                .float(v)
                .0,
            &with_deltas,
        );
        assert_eq!(obs.status, GUARD_OK, "float {v}: {}", obs.message);
        // SAFETY: fixture read-back; TEST_LOCK held.
        unsafe { assert_eq!(ctest_clparse_get_stat(SIDES[1], 11), want, "saturating {v}") };
    }
}

#[test]
fn stat_index_out_of_range_is_ignored() {
    let _g = lock();
    // CL_ParseStat* guards `stat < 0 || stat >= MAX_CL_STATS` and returns
    // silently; the byte read means only 0..255 is reachable from the wire, so
    // the guard is unreachable through svc_updatestat and this test pins the
    // boundary that IS reachable.
    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_UPDATESTAT)
            .byte((MAX_CL_STATS - 1) as u8)
            .long(-5)
            .0,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    // SAFETY: fixture read-back; TEST_LOCK held.
    unsafe { assert_eq!(ctest_clparse_get_stat(SIDES[1], MAX_CL_STATS - 1), -5) };
}

#[test]
fn print_goes_to_the_shared_console() {
    let _g = lock();
    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_PRINT)
            .string("hello from the server")
            .0,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.con_count, 1, "svc_print emitted nothing -- vacuous");
    assert_eq!(obs.con_lines[0], "[con] hello from the server");
}

#[test]
fn updatename_and_updatefrags_write_the_scoreboard() {
    let _g = lock();
    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_UPDATENAME)
            .byte(2)
            .string("player two")
            .byte(SVC_UPDATEFRAGS)
            .byte(2)
            .short(-17)
            .0,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.readcount, 2 + 11 + 4);

    // Positive: the scoreboard image really changed on the Rust side.
    let sc_sz = // SAFETY: fixture size query.
        unsafe { ctest_clparse_score_size() } as usize;
    let mut before = vec![0u8; sc_sz];
    let mut after = vec![0u8; sc_sz];
    // SAFETY: sized buffers over static fixture storage; TEST_LOCK held.
    unsafe {
        ctest_clparse_reset();
        ctest_clparse_get_score(SIDES[1], 2, before.as_mut_ptr());
        let m = Msg::default().byte(SVC_UPDATENAME).byte(2).string("zed").0;
        ctest_svuser_load_message(m.as_ptr(), m.len() as c_int);
        let s = ctest_clparse_parse_server_message(SIDES[1]);
        assert_eq!(s, GUARD_OK, "{}", guard_message(s));
        ctest_clparse_get_score(SIDES[1], 2, after.as_mut_ptr());
    }
    assert_ne!(
        before, after,
        "svc_updatename wrote nothing -- vacuous test"
    );
}

#[test]
fn spawnbaseline_variants_and_spawnstatic_sound() {
    let _g = lock();
    // svc_spawnbaseline routes through CL_EntityNum (which grows
    // cl.num_entities) into CL_ParseBaseline version 1; the FitzQuake variant
    // is version 2 with a bits byte. Both write an entity_t the fixture reads
    // back byte for byte.
    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_SPAWNBASELINE)
            .short(9)
            .byte(2) // modelindex
            .byte(1) // frame
            .byte(0) // colormap
            .byte(0) // skin
            .coords16(100, -200, 300)
            .byte(10)
            .byte(20)
            .byte(30)
            .0,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert!(obs.readcount >= 10, "baseline consumed {}", obs.readcount);

    // svc_spawnstatic is the other CL_ParseBaseline caller: it appends to
    // cl.static_entities and copies the baseline into the live fields. Those
    // copies only show up in the statics image, and without this case a
    // `prev_frame + 1` mutation in CL_ParseStatic is caught by the 256-opcode
    // sweep alone. modelindex 40 is in range but unseeded, so ent->model stays
    // NULL and the parse runs to completion.
    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_SPAWNSTATIC)
            .byte(40) // modelindex: in range, not seeded -> ent->model == NULL
            .byte(2) // frame
            .byte(0) // colormap
            .byte(1) // skin
            .short(100)
            .byte(10)
            .short(-200)
            .byte(20)
            .short(300)
            .byte(30)
            .0,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.readcount, 14, "spawnstatic consumed {}", obs.readcount);

    // A seeded modelindex takes the other branch and reaches
    // ClParse_Glue_AddEfrags, which is a shared abort stub -- so both sides
    // must stop there, at the same readcount, having taken the same route.
    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_SPAWNSTATIC)
            .byte(3) // a seeded slot -> ent->model is non-NULL
            .byte(2)
            .byte(0)
            .byte(1)
            .short(100)
            .byte(10)
            .short(-200)
            .byte(20)
            .short(300)
            .byte(30)
            .0,
    );
    assert_eq!(obs.status, GUARD_SYS_ERROR, "{}", obs.message);
    assert!(
        obs.message.contains("R_AddEfrags"),
        "unexpected {:?}",
        obs.message
    );
    assert_eq!(obs.readcount, 14);

    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_SPAWNBASELINE2)
            .short(9)
            .byte(0) // bits: no large model/frame, no alpha
            .byte(2)
            .byte(1)
            .byte(0)
            .byte(0)
            .coords16(100, -200, 300)
            .byte(10)
            .byte(20)
            .byte(30)
            .0,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);

    // svc_spawnstaticsound calls S_StaticSound, which returns early in this
    // link (sound_started == false), so the observable part is the reads.
    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_SPAWNSTATICSOUND)
            .coords16(8, 16, 24)
            .byte(3)
            .byte(200)
            .byte(64)
            .0,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.readcount, 10);

    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_SPAWNSTATICSOUND2)
            .coords16(8, 16, 24)
            .short(300)
            .byte(200)
            .byte(64)
            .0,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(
        obs.readcount, 11,
        "the 2 variant reads a short sample index"
    );
}

#[test]
fn sound_packet_variants() {
    let _g = lock();
    // CL_ParseStartSoundPacket's field selection: the SND_* bits pick byte vs
    // short for volume/attenuation/entity/sound, and the large-sound bit is
    // the one that can exceed MAX_SOUNDS.
    const SND_VOLUME: u8 = 1;
    const SND_ATTENUATION: u8 = 2;
    const SND_LARGEENTITY: u8 = 8;
    const SND_LARGESOUND: u8 = 16;

    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_SOUND)
            .byte(0) // no optional fields
            .short(3 << 3) // entity 3, channel 0
            .byte(2) // sound number
            .coords16(1, 2, 3)
            .0,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.readcount, 11);

    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_SOUND)
            .byte(SND_VOLUME | SND_ATTENUATION | SND_LARGEENTITY | SND_LARGESOUND)
            .byte(180) // volume
            .byte(96) // attenuation / 64.0
            .short(30) // large entity (must stay under cl.max_edicts)
            .byte(4) // channel
            .short(5) // large sound
            .coords16(1, 2, 3)
            .0,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.readcount, 15);

    // The exact boundary of the entity guard. cl_parse.c:826 is
    // `ent > cl.max_edicts`, not `>=`, so max_edicts itself is legal -- unlike
    // CL_EntityNum at :114, which is `>=`. Both sides of the boundary have to
    // be here or an off-by-one in the port is invisible; a mutation to `>=`
    // survived the rest of this suite.
    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_SOUND)
            .byte(SND_LARGEENTITY)
            .short(64) // == cl.max_edicts, and cl_parse.c:826 lets it through
            .byte(0) // channel
            .byte(1) // sound number
            .coords16(1, 2, 3)
            .0,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.readcount, 12);

    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_SOUND)
            .byte(SND_LARGEENTITY)
            .short(65) // one past cl.max_edicts
            .byte(0)
            .byte(1)
            .coords16(1, 2, 3)
            .0,
    );
    assert_eq!(obs.status, GUARD_HOST_ERROR, "{}", obs.message);
    assert_eq!(obs.message, "CL_ParseStartSoundPacket: ent = 65");

    // Two raise arms of CL_ParseStartSoundPacket.
    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_SOUND)
            .byte(SND_LARGESOUND)
            .short(3 << 3)
            .short(4096)
            .0,
    );
    assert_eq!(obs.status, GUARD_HOST_ERROR);
    assert_eq!(obs.message, "CL_ParseStartSoundPacket: 4096 > MAX_SOUNDS");

    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_SOUND)
            .byte(SND_LARGEENTITY)
            .short(-1)
            .byte(0)
            .byte(1)
            .0,
    );
    assert_eq!(obs.status, GUARD_HOST_ERROR, "{}", obs.message);
    assert!(
        obs.message.starts_with("CL_ParseStartSoundPacket: ent = "),
        "unexpected message {:?}",
        obs.message
    );
}

#[test]
fn localsound_bounds_and_success() {
    let _g = lock();
    // The only CLPARSE_ERR_LOCALSOUND producer, and one of the five exported
    // cores, so it is driven both through the opcode and directly.
    let obs = parse_both_default(&Msg::default().byte(SVC_LOCALSOUND).byte(16).short(4096).0);
    assert_eq!(obs.status, GUARD_HOST_ERROR);
    assert_eq!(obs.message, "CL_ParseLocalSound: 4096 > MAX_SOUNDS");

    let obs = parse_both_default(&Msg::default().byte(SVC_LOCALSOUND).byte(0).byte(3).0);
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.readcount, 3);

    // Direct core entry, same two outcomes.
    for (payload, want) in [
        (Msg::default().byte(16).short(4096).0, GUARD_HOST_ERROR),
        (Msg::default().byte(0).byte(2).0, GUARD_OK),
    ] {
        let results: Vec<(c_int, String)> = SIDES
            .iter()
            .map(|&side| {
                // SAFETY: guarded fixture driver; the message is reloaded for
                // each side and TEST_LOCK is held.
                unsafe {
                    ctest_clparse_reset();
                    ctest_svuser_load_message(payload.as_ptr(), payload.len() as c_int);
                    // CL_ParseLocalSound reads from wherever the cursor is, so
                    // it must be rewound explicitly -- the opcode dispatcher
                    // normally does that via MSG_BeginReading.
                    ctest_clparse_begin_reading(side);
                    let s = ctest_clparse_parse_local_sound(side);
                    (s, guard_message(s))
                }
            })
            .collect();
        assert_eq!(results[0], results[1], "CL_ParseLocalSound core diverged");
        assert_eq!(results[0].0, want);
    }
}

#[test]
fn clientdata_drives_the_stat_block() {
    let _g = lock();
    // svc_clientdata is the densest bitfield in the file. Bits are chosen to
    // exercise the SU_* optional fields on both sides of each conditional.
    const SU_VIEWHEIGHT: u32 = 1;
    const SU_IDEALPITCH: u32 = 2;
    const SU_PUNCH1: u32 = 4;
    const SU_VELOCITY1: u32 = 32;
    const SU_ITEMS: u32 = 512;
    const SU_WEAPONFRAME: u32 = 4096;
    const SU_ARMOR: u32 = 8192;
    const SU_WEAPON: u32 = 16384;
    const SU_EXTEND1: u32 = 1 << 15;

    let bits = SU_VIEWHEIGHT
        | SU_IDEALPITCH
        | SU_PUNCH1
        | SU_VELOCITY1
        | SU_ITEMS
        | SU_WEAPONFRAME
        | SU_ARMOR
        | SU_WEAPON;
    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_CLIENTDATA)
            .short(bits as i16)
            .byte(22) // viewheight
            .byte(200) // idealpitch (signed char)
            .byte(250) // punchangle[0]
            .byte(3) // velocity[0]
            .long(0x0000_1234) // items
            .byte(7) // weaponframe
            .byte(80) // armor
            .byte(2) // weapon
            .short(99) // health
            .byte(45) // ammo
            .byte(1)
            .byte(2)
            .byte(3)
            .byte(4) // shells/nails/rockets/cells
            .byte(1) // activeweapon
            .0,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    // 1, not 0: the parse loop always reads one byte past the end to find the
    // end of the message. A payload that ran short would raise instead.
    assert_eq!(obs.badread, 1, "the message was long enough");
    // SAFETY: fixture read-back; TEST_LOCK held.
    unsafe { assert_eq!(ctest_clparse_get_stat(SIDES[1], STAT_HEALTH), 99) };

    // SU_EXTEND1 pulls in a second bits byte, which is the extension ladder.
    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_CLIENTDATA)
            .short(SU_EXTEND1 as i16)
            .byte(0) // the SU_EXTEND1 byte, bits 16..23
            // `bits |= SU_ITEMS` is unconditional, so the long is always read.
            .long(0x0000_0055)
            .short(50) // health
            .byte(10) // ammo
            .byte(0)
            .byte(0)
            .byte(0)
            .byte(0) // shells/nails/rockets/cells
            .byte(0) // activeweapon
            .0,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
}

#[test]
fn end_of_message_item_gettime_sweep() {
    let _g = lock();
    // The loop's cmd == -1 exit compares cl.items against STAT_ITEMS and
    // stamps cl.time into item_gettime[] for every newly-set bit. Seeding a
    // different stats[STAT_ITEMS] than cl.items is the only way to reach it,
    // and it is pure float/array work with no stub in the way.
    let obs = parse_both(&Msg::default().byte(SVC_NOP).0, &|| {
        // SAFETY: dual-side seeders over static storage; TEST_LOCK held.
        unsafe {
            ctest_clparse_set_items(0x0000_00f0);
            ctest_clparse_set_stat(STAT_ITEMS, 0x0f0f_00ff);
        }
    });
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    // Positive: the sweep ran, so cl.items now mirrors the stat.
    assert_eq!(obs.badread, 1, "the loop ran off the end of the message");
}

#[test]
fn signon_three_writes_begin_into_cls_message() {
    let _g = lock();
    // CL_SignonReply case 3 is the one arm of cl_main.c's reply this link can
    // model on both sides (cl_parse_ref.c's twin says why cases 1 and 2 are
    // not); it writes clc_stringcmd + "begin" into cls.message.
    let obs = parse_both_default(&Msg::default().byte(SVC_SIGNONNUM).byte(3).0);
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(
        obs.message_bytes,
        b"\x04begin\0".to_vec(),
        "clc_stringcmd + \"begin\""
    );
    assert!(
        obs.con_lines
            .iter()
            .any(|l| l.contains("CL_SignonReply: 3")),
        "console log missing the reply trace: {:?}",
        obs.con_lines
    );
}

#[test]
fn signon_two_checks_efrags() {
    let _g = lock();
    // signon 2 is unreachable from the fixture default (cls.signon is already
    // 2), so this drives it from 1 and lands on R_CheckEfrags -- an abort stub
    // reached through ClParse_Glue_CheckEfrags on the Rust side and directly
    // on the C side. Equal status and message is the ADR-009 round trip.
    let obs = parse_both(&Msg::default().byte(SVC_SIGNONNUM).byte(2).0, &|| {
        // SAFETY: dual-side seeder; TEST_LOCK held.
        unsafe {
            ctest_clparse_set_conn(2 /* ca_connected */, 1, 0, 0)
        };
    });
    assert_eq!(obs.status, GUARD_SYS_ERROR, "{}", obs.message);
    assert!(
        obs.message.contains("R_CheckEfrags"),
        "unexpected stop: {:?}",
        obs.message
    );

    // The static-entity warning fires before it when there are more than 128.
    let obs = parse_both(&Msg::default().byte(SVC_SIGNONNUM).byte(2).0, &|| {
        // SAFETY: dual-side seeders; TEST_LOCK held.
        unsafe {
            ctest_clparse_set_conn(2, 1, 0, 0);
            ctest_clparse_set_counts(4, 1, 8, 200);
        }
    });
    assert!(
        obs.con_lines
            .iter()
            .any(|l| l.contains("200 static entities")),
        "missing the over-limit warning: {:?}",
        obs.con_lines
    );
}

// ===========================================================================
// The fast-update path (cmd & 128) -- CL_ParseUpdate, the largest single
// function in the file.
// ===========================================================================

#[test]
fn fast_update_basic_and_signal_bits() {
    let _g = lock();
    const U_MOREBITS: u8 = 1;
    const U_ORIGIN1: u8 = 2;
    const U_ANGLE2: u8 = 1 << 4;
    const U_FRAME: u8 = 1 << 6;
    const U_SIGNAL: u8 = 128;
    const U_ANGLE1: u16 = 1 << 8;
    const U_MODEL: u16 = 1 << 10;
    const U_COLORMAP: u16 = 1 << 11;
    const U_SKIN: u16 = 1 << 12;
    const U_EFFECTS: u16 = 1 << 13;
    const U_LONGENTITY: u16 = 1 << 14;
    const U_EXTEND1: u16 = 1 << 15;
    const U_MODEL2: u32 = 1 << 18;

    // Minimal update: signal + a small entity number in the low bits.
    let obs = parse_both_default(&Msg::default().byte(U_SIGNAL).byte(5).0);
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.readcount, 2);

    // Every simple optional field at once.
    let low = U_MOREBITS | U_ORIGIN1 | U_ANGLE2 | U_FRAME;
    let high = ((U_ANGLE1 | U_MODEL | U_COLORMAP | U_SKIN | U_EFFECTS) >> 8) as u8;
    let obs = parse_both_default(
        &Msg::default()
            .byte(U_SIGNAL | low)
            .byte(high)
            .byte(6) // entity number
            .byte(3) // modelindex
            .byte(2) // frame
            .byte(0) // colormap
            .byte(1) // skin
            .byte(9) // effects
            .short(400) // origin[0]
            .byte(32) // angle[0]
            .byte(64) // angle[1]
            .0,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert!(obs.readcount >= 9);

    // CLPARSE_ERR_BADMODNUM. `modnum` from U_MODEL is a byte and can never
    // reach MAX_MODELS, so the only reachable half of that guard is the
    // U_MODEL2 one at cl_parse.c:1288, which ORs in a high byte.
    let obs = parse_both_default(
        &Msg::default()
            .byte(U_SIGNAL | U_MOREBITS)
            .byte(((U_MODEL | U_EXTEND1) >> 8) as u8)
            .byte((U_MODEL2 >> 16) as u8)
            .byte(8) // entity number
            .byte(2) // modnum low byte
            .byte(0x20) // modnum high byte -> 8194 >= MAX_MODELS
            .0,
    );
    assert_eq!(obs.status, GUARD_HOST_ERROR, "{}", obs.message);
    assert_eq!(obs.message, "CL_ParseModel: bad modnum");
    assert_eq!(obs.readcount, 6);

    // All six coordinate fields in one update. The block above only sets
    // U_ORIGIN1/U_ANGLE2, which leaves the origin[1]/origin[2]/angles[2]
    // branches of the read ladder covered by the two sweeps alone; a
    // transposed or dropped field there deserves a named test. The read order
    // is origin1, angle1, origin2, angle2, origin3, angle3, so the six values
    // are distinct and the entity image is what proves each landed in its own
    // slot.
    const U_ORIGIN2: u8 = 1 << 2;
    const U_ORIGIN3: u8 = 1 << 3;
    const U_ANGLE3: u16 = 1 << 9;
    let obs = parse_both_default(
        &Msg::default()
            .byte(U_SIGNAL | U_MOREBITS | U_ORIGIN1 | U_ORIGIN2 | U_ORIGIN3 | U_ANGLE2)
            .byte(((U_ANGLE1 | U_ANGLE3) >> 8) as u8)
            .byte(7) // entity number
            .short(400) // origin[0]
            .byte(16) // angles[0]
            .short(-500) // origin[1]
            .byte(48) // angles[1]
            .short(600) // origin[2]
            .byte(80) // angles[2]
            .0,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(
        obs.readcount, 12,
        "the six-field read ladder consumed {} bytes",
        obs.readcount
    );

    // U_LONGENTITY takes a short instead of a byte; 300 exceeds the fixture's
    // 64-entity array and must raise identically on both sides.
    let obs = parse_both_default(
        &Msg::default()
            .byte(U_SIGNAL | U_MOREBITS)
            .byte((U_LONGENTITY >> 8) as u8)
            .short(300)
            .0,
    );
    assert_eq!(obs.status, GUARD_HOST_ERROR);
    assert_eq!(obs.message, "CL_EntityNum: 300 is an invalid number");
}

#[test]
fn update_lerp_bandaid_threshold() {
    let _g = lock();
    // cl_parse.c:500 -- the file's ONLY libm call:
    //   if (fabs (ent->msg_origins[0][j] - ent->msg_origins[1][j]) > 100)
    // The subtraction happens in float and promotes to double for fabs, which
    // is the operation order ADR-010 pins. 13.3 fixed point makes 800 == 100.0
    // exactly, so 799/800/801 straddle the boundary in both directions with no
    // rounding slack, and the entity image records which side of it each run
    // landed on.
    const U_SIGNAL: u8 = 128;
    const U_ORIGIN1: u8 = 2;

    for delta in [799i16, 800, 801, -799, -800, -801] {
        // Two updates to the same entity: the first fills msg_origins[1], the
        // second is the one whose difference is measured.
        let msg = Msg::default()
            .byte(U_SIGNAL | U_ORIGIN1)
            .byte(7)
            .short(0)
            .byte(U_SIGNAL | U_ORIGIN1)
            .byte(7)
            .short(delta)
            .0;
        let obs = parse_both_default(&msg);
        assert_eq!(obs.status, GUARD_OK, "delta {delta}: {}", obs.message);
        assert_eq!(obs.readcount, 8, "delta {delta}");
    }

    // Positive: the second update really moved the entity, so the six cases
    // above are not six comparisons of an untouched array.
    let ent_sz = // SAFETY: fixture size query.
        unsafe { ctest_clparse_entity_size() } as usize;
    let mut before = vec![0u8; ent_sz];
    let mut after = vec![0u8; ent_sz];
    // SAFETY: sized buffers over static fixture storage; TEST_LOCK held.
    unsafe {
        ctest_clparse_reset();
        ctest_clparse_get_entity(SIDES[1], 7, before.as_mut_ptr());
        let m = Msg::default()
            .byte(U_SIGNAL | U_ORIGIN1)
            .byte(7)
            .short(801)
            .0;
        ctest_svuser_load_message(m.as_ptr(), m.len() as c_int);
        let s = ctest_clparse_parse_server_message(SIDES[1]);
        assert_eq!(s, GUARD_OK, "{}", guard_message(s));
        ctest_clparse_get_entity(SIDES[1], 7, after.as_mut_ptr());
    }
    assert_ne!(before, after, "the update wrote nothing -- vacuous test");
}

#[test]
fn model_change_selects_a_synctype_branch() {
    let _g = lock();
    // CL_ParseUpdate's model-change block picks ent->syncbase from
    // model->synctype. ST_SYNC gives 0.0 and ST_FRAMETIME gives -cl.time, both
    // deterministic; ST_RAND is deliberately NOT driven, because it consumes
    // COM_Rand() and the two sides draw from separate generators in this link,
    // which would be a harness artefact rather than a port difference.
    const U_SIGNAL: u8 = 128;
    const U_MOREBITS: u8 = 1;
    const U_MODEL: u16 = 1 << 10;

    for (synctype, label) in [(0, "ST_SYNC"), (2, "ST_FRAMETIME")] {
        let obs = parse_both(
            &Msg::default()
                .byte(U_SIGNAL | U_MOREBITS)
                .byte((U_MODEL >> 8) as u8)
                .byte(9)
                .byte(4) // modelindex -> ctest_clparse_models[4]
                .0,
            &move || {
                // SAFETY: dual-side seeders over static fixture storage;
                // TEST_LOCK is held.
                unsafe {
                    ctest_clparse_seed_precaches(6, 6);
                    ctest_clparse_set_model_info(4, 7, 0, 1 /* mod_alias */, synctype);
                    ctest_clparse_set_time(2.25, 2.0, 2.25, 2.0);
                }
            },
        );
        assert_eq!(obs.status, GUARD_OK, "{label}: {}", obs.message);
        assert_eq!(obs.readcount, 4, "{label}");
    }

    // Positive: the model really changed, so the two runs above compared a
    // populated entity rather than an untouched one.
    let ent_sz = // SAFETY: fixture size query.
        unsafe { ctest_clparse_entity_size() } as usize;
    let mut before = vec![0u8; ent_sz];
    let mut after = vec![0u8; ent_sz];
    // SAFETY: sized buffers over static fixture storage; TEST_LOCK held.
    unsafe {
        ctest_clparse_reset();
        ctest_clparse_set_model_info(4, 7, 0, 1, 2);
        ctest_clparse_get_entity(SIDES[1], 9, before.as_mut_ptr());
        let m = Msg::default()
            .byte(U_SIGNAL | U_MOREBITS)
            .byte((U_MODEL >> 8) as u8)
            .byte(9)
            .byte(4)
            .0;
        ctest_svuser_load_message(m.as_ptr(), m.len() as c_int);
        let st = ctest_clparse_parse_server_message(SIDES[1]);
        assert_eq!(st, GUARD_OK, "{}", guard_message(st));
        ctest_clparse_get_entity(SIDES[1], 9, after.as_mut_ptr());
    }
    assert_ne!(
        before, after,
        "the model change wrote nothing -- vacuous test"
    );
}

// ===========================================================================
// The other three exported cores.
// ===========================================================================

#[test]
fn new_translation_bounds_and_stub() {
    let _g = lock();
    // CL_NewTranslation is two lines: a Sys_Error above cl.maxclients, then
    // R_TranslatePlayerSkin, which is an abort stub reached through
    // ClParse_Glue_TranslatePlayerSkin on the Rust side. Both outcomes are
    // compared, so the arm selection is real even though neither completes.
    for (slot, expect_sys_error_text) in [
        (5, "CL_NewTranslation: slot > cl.maxclients"),
        (0, "R_TranslatePlayerSkin"),
        (4, "R_TranslatePlayerSkin"),
    ] {
        let results: Vec<(c_int, String)> = SIDES
            .iter()
            .map(|&side| {
                // SAFETY: guarded fixture driver over static storage; held lock.
                unsafe {
                    ctest_clparse_reset();
                    let s = ctest_clparse_new_translation(side, slot);
                    (s, guard_message(s))
                }
            })
            .collect();
        assert_eq!(results[0], results[1], "CL_NewTranslation({slot}) diverged");
        assert_eq!(results[0].0, GUARD_SYS_ERROR, "slot {slot}");
        assert!(
            results[0].1.contains(expect_sys_error_text),
            "slot {slot}: unexpected {:?}",
            results[0].1
        );
    }
}

#[test]
fn register_particles_fills_indices_with_minus_one() {
    let _g = lock();
    // With no names set and mod_numknown 0 the function is a 2048-iteration
    // fill of -1. That is genuinely all it does here, and it IS observable:
    // the fixture reads the indices back, and both sides must agree that every
    // scanned slot became -1.
    for &side in &SIDES {
        // SAFETY: guarded fixture driver; TEST_LOCK held.
        let s = unsafe {
            ctest_clparse_reset();
            ctest_clparse_set_mod_numknown(0);
            ctest_clparse_register_particles(side)
        };
        assert_eq!(
            s,
            GUARD_OK,
            "{} raised: {}",
            side_name(side),
            guard_message(s)
        );
        // SAFETY: fixture read-back over static storage.
        unsafe {
            for i in 0..32 {
                assert_eq!(
                    ctest_clparse_get_particle_index(side, i),
                    -1,
                    "{} slot {i}",
                    side_name(side)
                );
            }
        }
    }

    // A seeded name takes the other branch and reaches PScript_FindParticleType.
    let results: Vec<(c_int, String)> = SIDES
        .iter()
        .map(|&side| {
            // SAFETY: guarded fixture driver; TEST_LOCK held.
            unsafe {
                ctest_clparse_reset();
                ctest_clparse_set_mod_numknown(0);
                ctest_clparse_set_particle_name(3, c"effect.tele".as_ptr());
                let s = ctest_clparse_register_particles(side);
                (s, guard_message(s))
            }
        })
        .collect();
    assert_eq!(results[0], results[1], "the named-particle arm diverged");
    assert_eq!(results[0].0, GUARD_SYS_ERROR);
    assert!(
        results[0].1.contains("PScript_FindParticleType"),
        "unexpected {:?}",
        results[0].1
    );

    // And a non-zero mod_numknown reaches the model loop, which stubs.c
    // deliberately leaves at zero (stubs.c:7226 asks for both to be raised
    // together). PScript_UpdateModelEffects is a shared stubs.c no-op, so the
    // loop leaves no trace of its own; its call counter is the only thing that
    // makes the bound and the iteration order observable. Without this block
    // the loop would run zero times on both sides and the comparison above
    // would silently cover nothing.
    let results: Vec<(c_int, String, c_int, Option<String>)> = SIDES
        .iter()
        .map(|&side| {
            // SAFETY: guarded fixture driver; TEST_LOCK held.
            unsafe {
                ctest_clparse_reset();
                ctest_clparse_set_mod_numknown(2);
                ctest_pscript_model_effects_reset();
                let s = ctest_clparse_register_particles(side);
                (
                    s,
                    guard_message(s),
                    ctest_pscript_model_effects_count(),
                    opt_cstr(ctest_pscript_last_model_name()),
                )
            }
        })
        .collect();
    assert_eq!(results[0], results[1], "the model-effects loop diverged");
    assert_eq!(results[0].0, GUARD_OK, "{}", results[0].1);
    assert_eq!(
        results[0].2, 2,
        "the model loop did not run mod_numknown times"
    );
    assert_eq!(
        results[0].3.as_deref(),
        Some("progs/known1.mdl"),
        "the loop stopped somewhere other than the last model"
    );

    // Restore, so a later test in the same process does not inherit it.
    // SAFETY: dual-side seeder over static storage; TEST_LOCK held.
    unsafe { ctest_clparse_set_mod_numknown(0) };
}

// ===========================================================================
// Arms whose whole observable content is "which stub did each side reach".
// These are the ADR-009 round trip: on the Rust side every one of them goes
// port -> ClParse_Glue_* -> Host_Guard -> CLPARSE_RAISE_GUARD -> ClParse_Raise
// -> Host_Reraise -> the outer guard, and must land where the plain C call
// lands, with the same message and the same readcount.
// ===========================================================================

#[test]
fn guarded_callees_stop_symmetrically() {
    let _g = lock();
    let cases: [(&str, Vec<u8>, &str); 8] = [
        (
            "svc_serverinfo -> CL_ClearState -> PR_ClearProgs",
            Msg::default().byte(11).long(666).0,
            "PR_ClearProgs",
        ),
        (
            "svc_centerprint -> SCR_CenterPrint",
            Msg::default().byte(26).string("centred").0,
            "SCR_CenterPrint",
        ),
        (
            "svc_particle -> R_ParseParticleEffect",
            Msg::default().byte(18).0,
            "R_ParseParticleEffect",
        ),
        (
            "svc_skybox -> Sky_LoadSkyBox",
            Msg::default().byte(37).string("sky").0,
            "Sky_LoadSkyBox",
        ),
        (
            "svc_fog -> Fog_ParseServerMessage",
            Msg::default().byte(41).0,
            "Fog_ParseServerMessage",
        ),
        (
            "svc_achievement -> Steam_SetAchievement",
            Msg::default().byte(52).string("ach").0,
            "Steam_SetAchievement",
        ),
        (
            "svc_setpause -> CDAudio_Pause",
            Msg::default().byte(24).byte(1).0,
            "CDAudio_Pause",
        ),
        (
            "svc_updatecolors -> R_TranslatePlayerSkin",
            Msg::default().byte(SVC_UPDATECOLORS).byte(1).byte(0x21).0,
            "R_TranslatePlayerSkin",
        ),
    ];

    for (name, msg, stub) in cases {
        let obs = parse_both_default(&msg);
        assert_eq!(obs.status, GUARD_SYS_ERROR, "{name}: {}", obs.message);
        assert!(
            obs.message.contains(stub),
            "{name}: expected to stop at {stub}, got {:?}",
            obs.message
        );
        assert!(obs.readcount >= 1, "{name} consumed nothing");
    }
}

#[test]
fn temp_entity_reaches_the_cl_tent_port() {
    let _g = lock();
    // svc_temp_entity dispatches into CL_ParseTEnt, which is the Rust cl_tent
    // port on the plain side and cl_tent.c on the oracle side (T7.2's own
    // differential proved that pair). TE_GUNSHOT runs to completion in this
    // link, so this checks the hand-off rather than the callee.
    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_TEMP_ENTITY)
            .byte(2)
            .coords16(1, 2, 3)
            .0,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.readcount, 8);
}

#[test]
fn damage_reaches_the_view_port() {
    let _g = lock();
    // V_ParseDamage is view.c on the oracle side and quake-capi/src/view.rs on
    // the plain side -- both real implementations, so this arm is compared for
    // its effects and not just its stopping point.
    let obs = parse_both_default(
        &Msg::default()
            .byte(SVC_DAMAGE)
            .byte(20) // armor
            .byte(30) // blood
            .coords16(64, -64, 8)
            .0,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.readcount, 9);
}

#[test]
fn stufftext_is_compared_for_reads_only() {
    let _g = lock();
    // Cbuf_AddText resolves to two different command buffers (module doc), so
    // the assertion is deliberately restricted to the read side and the client
    // state. A "//" prefix would additionally reach Cmd_ExecuteString and two
    // different command TABLES, so it is not driven here.
    let obs = parse_both_default(&Msg::default().byte(9).string("bind x +jump\n").0);
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.readcount, 15);
}

// ===========================================================================
// Exhaustive opcode sweep and seeded fuzz.
//
// The targeted tests above choose their payloads, which means they can only
// find divergences in arms someone thought to write down. These two do not
// choose: the sweep drives every one of the 256 possible command bytes, and
// the fuzz builds multi-opcode messages from a seeded PRNG. Both compare the
// full observation, so an arm nobody listed still has to agree.
// ===========================================================================

/// xorshift64* -- reproducible across platforms and independent of the
/// harness's own generator, which the port may consume.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn byte(&mut self) -> u8 {
        (self.next_u64() >> 33) as u8
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() >> 33) as usize % n
    }
}

#[test]
fn every_command_byte_agrees() {
    let _g = lock();
    // One opcode, then a fixed 40-byte tail that is the same for every opcode,
    // so an arm that reads too much or too little shows up as a readcount
    // difference rather than as a different payload.
    //
    // Every tail byte is below 20. That gives MSG_ReadString NULs to terminate
    // on, and it keeps every byte pair the tail can form under MAX_MODELS, so
    // the unguarded BASELINE_SPAWNERS load stays inside cl.model_precache
    // instead of landing half a megabyte past `cl` (see BASELINE_SPAWNERS).
    let tail: Vec<u8> = (0u8..40)
        .map(|i| i.wrapping_mul(7).wrapping_add(3) % 20)
        .collect();

    let mut ok = 0usize;
    let mut host_error = 0usize;
    let mut sys_error = 0usize;
    let mut ran_an_arm = 0usize;
    let mut signatures: BTreeSet<(c_int, c_int, String)> = BTreeSet::new();

    for cmd in 0u16..=255 {
        let mut msg = vec![cmd as u8];
        msg.extend_from_slice(&tail);
        let obs = parse_both_default(&msg);
        match obs.status {
            GUARD_OK => ok += 1,
            GUARD_HOST_ERROR => host_error += 1,
            GUARD_SYS_ERROR => sys_error += 1,
            other => panic!("unknown guard status {other} for command byte {cmd}"),
        }
        assert!(obs.readcount >= 1, "command byte {cmd} consumed nothing");
        // readcount > 2 means the opcode's own arm read a payload rather than
        // the dispatch rejecting the byte outright.
        if obs.readcount > 2 {
            ran_an_arm += 1;
        }
        signatures.insert((obs.status, obs.readcount, obs.message.clone()));
    }

    // Anti-degeneracy. The failure this guards against is a fixture that stops
    // at the same place for every input -- then all 256 comparisons hold and
    // none of them means anything. Distinct (status, readcount, message)
    // triples is the direct measure of that, and `ran_an_arm` says the
    // dispatch is reaching payload-reading code rather than bouncing off the
    // first guard. Bounds sit below the values observed at T7.3 so that adding
    // an opcode to the engine does not fail the suite for the wrong reason.
    assert_eq!(ok + host_error + sys_error, 256);
    assert!(
        signatures.len() >= 40,
        "only {} distinct outcomes across 256 command bytes",
        signatures.len()
    );
    assert!(
        ran_an_arm >= 100,
        "only {ran_an_arm}/256 command bytes read a payload"
    );
    assert!(
        host_error >= 20,
        "only {host_error}/256 command bytes reached a Host_Error"
    );
}

#[test]
fn fte_replacement_deltas_entity_update() {
    let _g = lock();
    // svcfte_updateentities -> CLFTE_ParseEntitiesUpdate -> CLFTE_ReadDelta ->
    // CL_EntitiesDeltaed, the PEXT2_REPLACEMENTDELTAS entity path. Nothing
    // else in this suite drives the delta ladder with real field bits: a
    // `+ 0.125` mutation on the UF_ORIGINZ read survived the entire file,
    // fuzz sweep included, before this test existed.
    const UF_FRAME: u8 = 1 << 0;
    const UF_ORIGINXY: u8 = 1 << 1;
    const UF_ORIGINZ: u8 = 1 << 2;
    const UF_ANGLESXZ: u8 = 1 << 3;
    const UF_ANGLESY: u8 = 1 << 4;
    const UF_EXTEND1: u8 = 1 << 7;
    // UF_RESET is bit 8, i.e. the low bit of the byte UF_EXTEND1 pulls in.
    const UF_RESET_HI: u8 = 1 << 0;

    let seed = || {
        // SAFETY: dual-side seeder over static storage; TEST_LOCK held.
        unsafe { ctest_clparse_set_protocol(666, 0, 0, PEXT2_REPLACEMENTDELTAS) };
    };

    // One entity with every positional field present and distinct. The read
    // order is frame, origin[0], origin[1], origin[2], angles[0], angles[2],
    // angles[1]; the entity image is what proves each value landed in its own
    // slot rather than being transposed.
    let obs = parse_both(
        &Msg::default()
            .byte(SVCFTE_UPDATEENTITIES)
            .float(2.5) // newtime
            .short(5) // entity 5: no remove flag, no 0x4000 extension
            .byte(UF_FRAME | UF_ORIGINXY | UF_ORIGINZ | UF_ANGLESXZ | UF_ANGLESY)
            .byte(9) // frame
            .short(400) // origin[0]
            .short(-500) // origin[1]
            .short(600) // origin[2]
            .byte(16) // angles[0]
            .byte(48) // angles[2]
            .byte(80) // angles[1]
            .short(0) // list terminator
            .0,
        &seed,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(
        obs.readcount, 20,
        "the delta ladder consumed {} bytes",
        obs.readcount
    );

    // The removal flag (0x8000) takes the other branch of the loop and reaches
    // InvalidateTraceLineCache.
    let obs = parse_both(
        &Msg::default()
            .byte(SVCFTE_UPDATEENTITIES)
            .float(2.5)
            .short(0x8005u16 as i16) // remove entity 5
            .short(0) // list terminator
            .0,
        &seed,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.readcount, 9);

    // Entity 0 *with* the remove flag is the "reset all" arm, which walks
    // cl.num_entities clearing every slot.
    let obs = parse_both(
        &Msg::default()
            .byte(SVCFTE_UPDATEENTITIES)
            .float(2.5)
            .short(0x8000u16 as i16) // remove world == reset all
            .short(0)
            .0,
        &seed,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.readcount, 9);

    // UF_EXTEND1 pulls in the second bits byte, and UF_RESET there copies the
    // baseline over the netstate instead of the previous copy.
    let obs = parse_both(
        &Msg::default()
            .byte(SVCFTE_UPDATEENTITIES)
            .float(2.5)
            .short(6)
            .byte(UF_EXTEND1)
            .byte(UF_RESET_HI)
            .short(0)
            .0,
        &seed,
    );
    assert_eq!(obs.status, GUARD_OK, "{}", obs.message);
    assert_eq!(obs.readcount, 11);

    // UF_BONEDATA with a low flag bit set is CLPARSE_END_DELTAINFO, the other
    // Host_EndGame inside CLFTE_ReadDelta.
    let obs = parse_both(
        &Msg::default()
            .byte(SVCFTE_UPDATEENTITIES)
            .float(2.5)
            .short(7)
            .byte(0x80) // UF_EXTEND1
            .byte(0x80) // UF_EXTEND2
            .byte(0x04) // UF_BONEDATA (bit 18)
            .byte(0x01) // fl: no 0x80/0x40, but fl & 0x3f is set
            .0,
        &seed,
    );
    assert_eq!(obs.status, GUARD_HOST_ERROR, "{}", obs.message);
    assert_eq!(
        obs.message,
        "Host_EndGame: unsupported entity delta info
"
    );

    // UF_UNUSED1 (bit 31) is the CLPARSE_END_UF_UNUSED1 EndGame, and reaching
    // it means walking the whole UF_EXTEND1/2/3 ladder with no field bits set.
    let obs = parse_both(
        &Msg::default()
            .byte(SVCFTE_UPDATEENTITIES)
            .float(2.5)
            .short(7)
            .byte(0x80) // UF_EXTEND1
            .byte(0x80) // UF_EXTEND2
            .byte(0x80) // UF_EXTEND3
            .byte(0x80) // UF_UNUSED1
            .0,
        &seed,
    );
    assert_eq!(obs.status, GUARD_HOST_ERROR, "{}", obs.message);
    assert_eq!(
        obs.message,
        "Host_EndGame: UF_UNUSED1 bit
"
    );

    // The 0x4000 escape reads a third byte of entity number, and 0x4000|0x3fff
    // with a high byte set is far past the fixture's 64 entities, so both
    // sides must raise from CL_EntityNum with the same number.
    let obs = parse_both(
        &Msg::default()
            .byte(SVCFTE_UPDATEENTITIES)
            .float(2.5)
            .short(0x4001u16 as i16)
            .byte(1) // newnum = 1 | (1 << 14) = 16385
            .0,
        &seed,
    );
    assert_eq!(obs.status, GUARD_HOST_ERROR, "{}", obs.message);
    assert_eq!(obs.message, "CL_EntityNum: 16385 is an invalid number");
}

#[test]
fn seeded_fuzz_sweep() {
    let _g = lock();
    // Multi-opcode messages, so `lastcmd` carries, entities accumulate and the
    // end-of-message item sweep runs against state an earlier opcode wrote --
    // none of which a single-opcode message can reach.
    let interesting: [u8; 22] = [
        SVC_NOP,
        SVC_UPDATESTAT,
        SVC_VERSION,
        SVC_SETVIEW,
        SVC_SOUND,
        SVC_TIME,
        SVC_PRINT,
        SVC_SETANGLE,
        SVC_LIGHTSTYLE,
        SVC_UPDATENAME,
        SVC_UPDATEFRAGS,
        SVC_CLIENTDATA,
        SVC_STOPSOUND,
        SVC_SIGNONNUM,
        SVC_KILLEDMONSTER,
        SVC_FOUNDSECRET,
        SVC_SPAWNSTATICSOUND,
        SVC_SPAWNSTATICSOUND2,
        SVC_INTERMISSION,
        SVC_LOCALSOUND,
        SVCFTE_SETANGLEDELTA,
        0x80, // fast update
    ];

    let mut rng = Rng(0x5eed_c1a2_5e11);
    let mut nonempty = 0usize;

    for case in 0..400 {
        let mut msg: Vec<u8> = Vec::new();
        let ops = 1 + rng.below(5);
        for _ in 0..ops {
            msg.push(interesting[rng.below(interesting.len())]);
            let payload = rng.below(12);
            for _ in 0..payload {
                // A quarter of the payload bytes are NUL so MSG_ReadString
                // terminates inside the message rather than always running to
                // the end and setting badread.
                let mut b = if rng.below(4) == 0 { 0 } else { rng.byte() };
                // BASELINE_SPAWNERS are excluded from the corpus, payload bytes
                // included, because they are the only arms that index
                // cl.model_precache without a MAX_MODELS guard -- see the note
                // on the NUL tail below.
                while BASELINE_SPAWNERS.contains(&b) {
                    b = rng.byte();
                }
                msg.push(b);
            }
        }

        // NUL tail, so no arm runs off the end. The cost is that a fuzz
        // message now ends at svc_bad rather than at end-of-message; the
        // end-of-message sweep is driven deliberately elsewhere.
        msg.extend(std::iter::repeat_n(0u8, 48));

        let pext2 = if case % 3 == 0 {
            PEXT2_REPLACEMENTDELTAS
        } else {
            0
        };
        let obs = parse_both(&msg, &move || {
            // SAFETY: dual-side seeder over static storage; TEST_LOCK held.
            unsafe { ctest_clparse_set_protocol(666, 0, 0, pext2) };
        });
        if obs.readcount > 1 {
            nonempty += 1;
        }
    }

    assert!(
        nonempty >= 350,
        "only {nonempty}/400 fuzz messages got past the first byte"
    );
}

#[test]
fn shownet_tracing_agrees() {
    let _g = lock();
    // cl_shownet is the plain twin this task owns; at 1 it prints the message
    // size and at 2 it prints a rule plus one SHOWNET line per opcode, which
    // is the only place svc_strings is read on the success path.
    for level in [1.0f32, 2.0] {
        let obs = parse_both(
            &Msg::default()
                .byte(SVC_NOP)
                .byte(SVC_KILLEDMONSTER)
                .byte(SVC_FOUNDSECRET)
                .0,
            &move || {
                // SAFETY: dual-side seeder; TEST_LOCK held.
                unsafe { ctest_clparse_set_shownet(level) };
            },
        );
        assert_eq!(obs.status, GUARD_OK, "shownet {level}: {}", obs.message);
        assert!(
            obs.con_count > 0,
            "shownet {level} printed nothing -- the plain cl_shownet is not wired"
        );
    }
}

// ===========================================================================
// Layout pins. quake-c-sys/src/cl_parse.rs's doc comments promise these
// mirrors match the C definitions; a silent layout drift would corrupt every
// byte image above rather than fail loudly, so the sizes are asserted against
// the C compiler's own sizeof.
// ===========================================================================

#[test]
fn fixture_image_sizes_are_sane() {
    let _g = lock();
    // SAFETY: pure size queries into the C fixture.
    unsafe {
        assert!(ctest_clparse_cl_image_size() > 0);
        assert!(ctest_clparse_cls_image_size() > 0);
        assert!(ctest_clparse_entity_size() > 0);
        assert!(ctest_clparse_score_size() > 0);
        // lightstyle_t is `char map[64]; int length; char average, peak;`
        // plus padding -- the exact number is the compiler's, but it must be
        // large enough to hold MAX_STYLESTRING or the read-backs would be
        // truncating and every lightstyle comparison would be partly blind.
        assert!(
            ctest_clparse_lightstyle_size() >= 64 + 4 + 2,
            "lightstyle_t is smaller than MAX_STYLESTRING + length + 2 chars"
        );
    }
}
