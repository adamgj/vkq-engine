//! Differential gate for `Quake/r_part.c` -- the classic particle simulator.
//! Rust migration Phase 7, M10f-1 (task T10.5, first half).
//!
//! Read `stubs/r_part_ref.c`'s header first; it explains why `r_part.c` is
//! composed into that TU instead of listed in `build.rs`'s `C_SOURCES`, and
//! which objects each side owns.
//!
//! ## What this gate is for
//!
//! `Harness_HashParticles` (`r_part.c:927`) feeds `Harness_HashClient`
//! (`harness.c:312`), which feeds the per-frame state hash (`harness.c:524`),
//! which every one of the eleven demo-corpus goldens compares. The whole
//! simulator is therefore inside the determinism contract, every field of
//! every live particle, every frame. So nothing here compares a summary: each
//! check takes a **whole-pool snapshot** -- the active list in traversal
//! order (pool index, `org`, `vel`, `color`, `ramp`, `die`, `type`, each read
//! as raw bits, so `-0.0` and a NaN payload are differences), the free list as
//! an index sequence, `r_numparticles`, and `Harness_HashParticles` -- and
//! compares the two sides' snapshots as a unit.
//!
//! The free list is in there because it is not redundant with the active list:
//! the allocator is strictly LIFO, so the free list's order decides which slot
//! the *next* spawn takes, and the pool index is hashed. A port that spawned
//! correct particles into the wrong slots would pass an active-list-only
//! comparison and fail the corpus.
//!
//! ## Degenerate-gate defences
//!
//! A bit-exact differential passes whenever both sides do nothing, so every
//! test asserts something positive as well: a specific particle count, a
//! specific free-list length, a hash that actually moved off its seed, a
//! console line with expected text, a non-zero `msg_readcount`.
//!
//! The sharpest one is the shared `COM_Rand` generator (`stubs.c:233-262`).
//! Almost every spawn path draws from it, so each side is reseeded with the
//! same value and the generator's *next* output is sampled after each side
//! finishes: equal next-values prove both sides consumed exactly the same
//! number of draws through exactly the same branches, which no amount of
//! agreeing-by-accident can fake.
//!
//! ## Hidden per-side state
//!
//! Two objects have no reset entry point and no accessor:
//! `avelocities` (`r_part.c:268`, lazily filled on the first
//! `R_EntityParticles` call, 486 `COM_Rand` draws) and `R_RocketTrail`'s
//! function-local `tracercount` (`r_part.c:702`, which alternates tracer
//! direction). Exporting a reset hook would mean adding test-only code to the
//! production port, so instead all `R_EntityParticles` driving lives in
//! `entity_particles_differential` and all tracer-type (`3`, `5`, `131`,
//! `133`) `R_RocketTrail` driving lives in `rocket_trail_differential`. Each
//! runs its full call sequence against one side and then the identical
//! sequence against the other, so both sides' hidden state advances in
//! lockstep regardless of the order the test binary happens to run its tests
//! in.
//!
//! ## ADR-010
//!
//! Every float in the simulator is single-precision, but C's promotion rules
//! put several expressions through `double`: `(COM_Rand () & 255) * 0.01`,
//! `p->die = cl.time + 0.01` (a `double` `cl.time` narrowed on store),
//! `grav = frametime * sv_gravity.value * 0.05`, `p->die = cl.time + 1 +
//! (COM_Rand () & 8) * 0.05`. Those are compared bit for bit here, at
//! `sv_gravity` and `cl.time` values chosen so the narrowing is not exact.
//! `ramp3` is the other ADR-010 item: `r_part.c:36` declares it `[8]` and
//! supplies six initialisers, so `ramp3[6]` and `ramp3[7]` are zero, and
//! `R_RocketTrail`'s `(COM_Rand () & 3) + 2` reaches index 5 while
//! `CL_RunParticles`' `p->ramp += time1` reaches 6 only after the `>= 6`
//! test has already killed the particle. `rocket_trail_differential` and
//! `run_particles_mixed_pool_differential` cover both readers.
//!
//! ## Known gaps
//!
//! `R_ParticleExplosion2` with `colorLength == 0` is `% 0`: undefined in C
//! (SIGFPE on x86) and a panic in Rust, which under `panic = "abort"` ends the
//! process. It is not driven here -- there is no shared behaviour to compare,
//! only two different crashes -- and the port carries a `// COMPAT: ADR-010`
//! note saying so.
//!
//! The rendering half is compiled but never runs (`no_rendering` is true), so
//! `R_SetParticleTexture_f`'s effect on `particletexture` /
//! `texturescalefactor` is not compared; only the fact that the port routed
//! the callback and the render tail through the glue seam is
//! (`ctest_rpart_texcb_count`, `ctest_rpart_initrender_count`, and the
//! `CVAR_CALLBACK` flag `Cvar_SetCallback` leaves behind). That is Phase 8
//! territory.

use core::ffi::{c_char, c_double, c_float, c_int, c_uint, c_void, CStr};
use std::ffi::CString;

use quake_ctest::fs as ctfs;

// ---------------------------------------------------------------------------
// Fixture (stubs/r_part_ref.c) and the shared stubs it builds on.

/// Mirrors `ctest_rpart_rec_t` in `stubs/r_part_ref.c`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Rec {
    index: c_int,
    org: [c_float; 3],
    vel: [c_float; 3],
    color: c_float,
    ramp: c_float,
    die: c_float,
    ty: c_int,
}

extern "C" {
    fn ctest_rpart_alloc(n: c_int);
    fn ctest_rpart_drop_pool();
    fn ctest_rpart_numparticles(side: c_int) -> c_int;
    fn ctest_rpart_rec_size() -> c_int;
    fn ctest_rpart_active(side: c_int, out: *mut Rec, max: c_int) -> c_int;
    fn ctest_rpart_free(side: c_int, out: *mut c_int, max: c_int) -> c_int;
    fn ctest_rpart_hash(side: c_int, h: u64) -> u64;
    fn ctest_rpart_seed_client(
        time: c_double,
        oldtime: c_double,
        state: c_int,
        mapname: *const c_char,
        protocolflags: c_uint,
    );
    fn ctest_rpart_set_gravity(value: c_float);
    fn ctest_rpart_cvar_flags(side: c_int, which: c_int) -> c_uint;
    fn ctest_rpart_cvar_value(side: c_int, which: c_int) -> c_float;
    fn ctest_rpart_pool_allocated(side: c_int) -> c_int;
    fn ctest_rpart_reset_glue_counters();
    fn ctest_rpart_initrender_count() -> c_int;
    fn ctest_rpart_texcb_count() -> c_int;

    fn ctest_rpart_init(side: c_int) -> c_int;
    fn ctest_rpart_entity_particles(side: c_int, origin: *mut c_float);
    fn ctest_rpart_clear_particles(side: c_int);
    fn ctest_rpart_read_point_file(side: c_int);
    fn ctest_rpart_parse_particle_effect(side: c_int);
    fn ctest_rpart_particle_explosion(side: c_int, org: *mut c_float);
    fn ctest_rpart_particle_explosion2(
        side: c_int,
        org: *mut c_float,
        color_start: c_int,
        color_length: c_int,
    );
    fn ctest_rpart_blob_explosion(side: c_int, org: *mut c_float);
    fn ctest_rpart_run_particle_effect(
        side: c_int,
        org: *mut c_float,
        dir: *mut c_float,
        color: c_int,
        count: c_int,
    );
    fn ctest_rpart_lava_splash(side: c_int, org: *mut c_float);
    fn ctest_rpart_teleport_splash(side: c_int, org: *mut c_float);
    fn ctest_rpart_rocket_trail(side: c_int, start: *mut c_float, end: *mut c_float, ty: c_int);
    fn ctest_rpart_run_particles(side: c_int);

    // stubs.c
    fn COM_SeedRand(seed: u64);
    fn COM_Rand() -> c_int;
    fn ctest_set_args(argc: c_int, argv: *mut *mut c_char);
    fn ctest_clear_con_log();
    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(i: c_int) -> *const c_char;

    // stubs/sv_user_ref.c -- seeds BOTH net_message buffers.
    fn ctest_svuser_load_message(data: *const u8, len: c_int);
    // stubs/cl_tent_ref.c -- rewinds / reads one side's message cursor.
    fn ctest_cltent_begin_reading(side: c_int);
    fn ctest_cltent_get_readcount(side: c_int) -> c_int;
    fn ctest_cltent_get_badread(side: c_int) -> c_int;
}

/// `side` 1 is the C oracle, 0 the Rust port; the oracle runs first so a
/// divergence report reads "C then Rust".
const SIDES: [c_int; 2] = [1, 0];

fn side_name(side: c_int) -> &'static str {
    if side == 1 {
        "C"
    } else {
        "Rust"
    }
}

/// `client.h:106`.
const CA_CONNECTED: c_int = 2;
/// `client.h:105`.
const CA_DISCONNECTED: c_int = 1;
/// `cvar.h:76-77`.
const CVAR_REGISTERED: c_uint = 1 << 10;
const CVAR_CALLBACK: c_uint = 1 << 16;
/// `protocol.h:48`.
const PRFL_FLOATCOORD: c_uint = 1 << 4;
/// `r_part.c:28`, `:32`.
const MAX_PARTICLES: c_int = 16384;
const ABSOLUTE_MIN_PARTICLES: c_int = 512;
/// An arbitrary non-zero hash seed, so "the hash never ran" is visible.
const HASH_SEED: u64 = 0xdead_beef_0bad_f00d;

// ---------------------------------------------------------------------------
// Whole-pool snapshots, compared as raw bits.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct RecBits {
    index: i32,
    org: [u32; 3],
    vel: [u32; 3],
    color: u32,
    ramp: u32,
    die: u32,
    ty: i32,
}

impl From<&Rec> for RecBits {
    fn from(r: &Rec) -> Self {
        RecBits {
            index: r.index,
            org: [r.org[0].to_bits(), r.org[1].to_bits(), r.org[2].to_bits()],
            vel: [r.vel[0].to_bits(), r.vel[1].to_bits(), r.vel[2].to_bits()],
            color: r.color.to_bits(),
            ramp: r.ramp.to_bits(),
            die: r.die.to_bits(),
            ty: r.ty,
        }
    }
}

/// Everything one side's simulator owns that a demo replay can observe.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Snap {
    numparticles: c_int,
    active: Vec<RecBits>,
    free: Vec<c_int>,
    hash: u64,
}

impl Snap {
    fn take(side: c_int) -> Snap {
        let cap = 4200usize;
        let mut recs = vec![
            Rec {
                index: 0,
                org: [0.0; 3],
                vel: [0.0; 3],
                color: 0.0,
                ramp: 0.0,
                die: 0.0,
                ty: 0,
            };
            cap
        ];
        let mut free = vec![0 as c_int; cap];

        // SAFETY: both buffers hold `cap` elements and `cap` is passed as the
        // limit; the fixture returns the true length even when it truncates,
        // which the assertions below turn into a failure rather than a silent
        // short read.
        let (n_active, n_free) = unsafe {
            (
                ctest_rpart_active(side, recs.as_mut_ptr(), cap as c_int),
                ctest_rpart_free(side, free.as_mut_ptr(), cap as c_int),
            )
        };
        assert!(
            (n_active as usize) <= cap && (n_free as usize) <= cap,
            "{}: pool snapshot truncated ({} active, {} free)",
            side_name(side),
            n_active,
            n_free
        );

        recs.truncate(n_active as usize);
        free.truncate(n_free as usize);

        // SAFETY: fixture accessors over the side's own globals.
        let (numparticles, hash) = unsafe {
            (
                ctest_rpart_numparticles(side),
                ctest_rpart_hash(side, HASH_SEED),
            )
        };

        Snap {
            numparticles,
            active: recs.iter().map(RecBits::from).collect(),
            free,
            hash,
        }
    }
}

/// Compares two snapshots and, on a mismatch, names the first differing
/// particle rather than dumping thousands of records.
fn assert_snap_eq(what: &str, c: &Snap, rust: &Snap) {
    assert_eq!(
        c.numparticles, rust.numparticles,
        "{what}: r_numparticles diverged"
    );
    for (i, (a, b)) in c.active.iter().zip(rust.active.iter()).enumerate() {
        assert_eq!(a, b, "{what}: active particle {i} diverged (C vs Rust)");
    }
    assert_eq!(
        c.active.len(),
        rust.active.len(),
        "{what}: active-list length diverged"
    );
    for (i, (a, b)) in c.free.iter().zip(rust.free.iter()).enumerate() {
        assert_eq!(a, b, "{what}: free-list slot {i} diverged (C vs Rust)");
    }
    assert_eq!(
        c.free.len(),
        rust.free.len(),
        "{what}: free-list length diverged"
    );
    assert_eq!(c.hash, rust.hash, "{what}: Harness_HashParticles diverged");
}

fn con_log() -> Vec<String> {
    // SAFETY: shared capture log from stubs.c, read under the harness lock.
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

/// One side's whole observable result: pool, console output and how far the
/// shared RNG advanced.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Run {
    snap: Snap,
    con: Vec<String>,
    rand_after: c_int,
}

/// Reseeds the shared RNG and the console log, runs `body` against one side,
/// then samples both. `seed` is applied per side, so the next-value sampled
/// afterwards is a direct count of how many draws that side consumed.
fn run_side(side: c_int, seed: u64, body: impl FnOnce(c_int)) -> Run {
    // SAFETY: shared stub state, mutated under the harness lock.
    unsafe {
        COM_SeedRand(seed);
        ctest_clear_con_log();
    }
    body(side);
    let snap = Snap::take(side);
    let con = con_log();
    // SAFETY: shared RNG; sampling advances it, which is why it is sampled
    // exactly once per side, after the snapshot.
    let rand_after = unsafe { COM_Rand() };
    Run {
        snap,
        con,
        rand_after,
    }
}

/// Drives `body` against both sides from the same seed and compares
/// everything. Returns the C side's run so a test can add positive assertions.
fn compare(what: &str, seed: u64, mut body: impl FnMut(c_int)) -> Run {
    let mut runs = Vec::with_capacity(2);
    for &side in &SIDES {
        runs.push(run_side(side, seed, &mut body));
    }
    assert_snap_eq(what, &runs[0].snap, &runs[1].snap);
    assert_eq!(runs[0].con, runs[1].con, "{what}: console output diverged");
    assert_eq!(
        runs[0].rand_after, runs[1].rand_after,
        "{what}: the two sides consumed different numbers of COM_Rand draws"
    );
    runs.remove(0)
}

/// `ctest_rpart_alloc` resets both pools, so it runs once per test; each side
/// then builds its own free list through the function under test.
fn fresh_pool(n: c_int) {
    // SAFETY: fixture reset of both sides' pool pointers.
    unsafe { ctest_rpart_alloc(n) };
}

fn seed_client(time: c_double, oldtime: c_double) {
    // SAFETY: writes both sides' `cl`/`cls`; `c"..."` is NUL-terminated.
    unsafe { ctest_rpart_seed_client(time, oldtime, CA_CONNECTED, c"e1m1".as_ptr(), 0) };
}

// ---------------------------------------------------------------------------

#[test]
fn fixture_record_abi() {
    let _guard = ctfs::lock();
    // SAFETY: pure size query.
    let sz = unsafe { ctest_rpart_rec_size() };
    assert_eq!(
        sz as usize,
        core::mem::size_of::<Rec>(),
        "ctest_rpart_rec_t and the Rust mirror disagree on size"
    );
}

/// `R_InitParticles` (`r_part.c:228`). The only entry point that can raise:
/// `Cvar_RegisterVariable` is a `Host_Reraise` wrapper under
/// `-Duse_rust_cvar`, so the port routes both calls through
/// `RPart_Glue_RegisterVariable` and returns a `Raise`. Both sides are entered
/// through `Host_Guard` here.
///
/// Registration is not idempotent-by-error -- `cvar.c:649-653` prints and
/// returns -- so the three phases can share one test, and phases two and three
/// double as a comparison of that console path.
#[test]
fn init_particles_differential() {
    let _guard = ctfs::lock();

    let phases: [(&[&str], c_int, bool); 3] = [
        (&["quake"], MAX_PARTICLES, false),
        (&["quake", "-particles", "37"], ABSOLUTE_MIN_PARTICLES, true),
        (&["quake", "-particles", "2048"], 2048, true),
    ];

    for (phase, (args, expect_num, expect_already_defined)) in phases.iter().enumerate() {
        let owned: Vec<CString> = args.iter().map(|a| CString::new(*a).unwrap()).collect();
        let mut ptrs: Vec<*mut c_char> = owned.iter().map(|c| c.as_ptr() as *mut c_char).collect();
        // SAFETY: `ctest_set_args` copies the pointers; `owned` outlives the
        // whole phase, and every string is NUL-terminated.
        unsafe { ctest_set_args(ptrs.len() as c_int, ptrs.as_mut_ptr()) };

        // SAFETY: fixture reset of the glue-seam counters.
        unsafe { ctest_rpart_reset_glue_counters() };

        let mut status = [0 as c_int; 2];
        let mut flags = [[0 as c_uint; 2]; 2];
        let mut values = [[0.0 as c_float; 2]; 2];
        let mut allocated = [0 as c_int; 2];
        let mut cons: Vec<Vec<String>> = Vec::with_capacity(2);

        for (slot, &side) in SIDES.iter().enumerate() {
            // SAFETY: shared console log, cleared under the harness lock.
            unsafe { ctest_clear_con_log() };
            // SAFETY: the driver enters both sides through `Host_Guard`.
            status[slot] = unsafe { ctest_rpart_init(side) };
            for which in 0..2usize {
                // SAFETY: fixture accessors over that side's cvar objects.
                unsafe {
                    flags[slot][which] = ctest_rpart_cvar_flags(side, which as c_int);
                    values[slot][which] = ctest_rpart_cvar_value(side, which as c_int);
                }
            }
            // SAFETY: fixture accessor.
            allocated[slot] = unsafe { ctest_rpart_pool_allocated(side) };
            cons.push(con_log());
        }

        assert_eq!(status[0], status[1], "phase {phase}: guard status diverged");
        assert_eq!(status[0], 0, "phase {phase}: R_InitParticles raised");
        assert_eq!(
            // SAFETY: fixture accessors.
            unsafe { (ctest_rpart_numparticles(1), ctest_rpart_numparticles(0),) },
            (*expect_num, *expect_num),
            "phase {phase}: r_numparticles"
        );
        assert_eq!(allocated, [1, 1], "phase {phase}: Mem_Alloc did not run");
        assert_eq!(flags[0], flags[1], "phase {phase}: cvar flags diverged");
        assert_eq!(values[0], values[1], "phase {phase}: cvar values diverged");
        assert_eq!(cons[0], cons[1], "phase {phase}: console output diverged");

        // Positive controls: registration really happened, and
        // Cvar_SetCallback really tagged r_particles (and only r_particles).
        assert_ne!(
            flags[0][0] & CVAR_REGISTERED,
            0,
            "phase {phase}: r_particles was never registered"
        );
        assert_ne!(
            flags[0][0] & CVAR_CALLBACK,
            0,
            "phase {phase}: Cvar_SetCallback never ran for r_particles"
        );
        assert_eq!(
            flags[0][1] & CVAR_CALLBACK,
            0,
            "phase {phase}: r_quadparticles gained a callback it should not have"
        );
        assert_eq!(values[0][0], 1.0, "phase {phase}: r_particles value");
        assert_eq!(values[0][1], 1.0, "phase {phase}: r_quadparticles value");

        if *expect_already_defined {
            assert_eq!(
                cons[0].len(),
                2,
                "phase {phase}: expected both re-registrations to warn, got {:?}",
                cons[0]
            );
            assert!(
                cons[0].iter().all(|l| l.contains("already defined")),
                "phase {phase}: unexpected console output {:?}",
                cons[0]
            );
        } else {
            assert!(
                cons[0].is_empty(),
                "phase {phase}: unexpected console output {:?}",
                cons[0]
            );
        }
    }

    // The seam itself: the port hands R_InitParticles' rendering tail back to
    // its C frame once per call, and that tail is a no-op because
    // no_rendering is true. The oracle's `if (!no_rendering)` is inlined in
    // r_part.c and has nothing to count, so this is a one-sided check of the
    // glue contract, not a cross-side comparison.
    // SAFETY: fixture accessors.
    let (init_render, texcb) =
        unsafe { (ctest_rpart_initrender_count(), ctest_rpart_texcb_count()) };
    assert_eq!(
        init_render, 1,
        "the port did not reach RPart_Glue_InitRender exactly once in the last phase"
    );
    assert_eq!(texcb, 0, "the texture callback ran without a cvar change");

    // SAFETY: leave no argv pointing at this test's freed CStrings.
    unsafe { ctest_set_args(0, core::ptr::null_mut()) };
}

/// `R_ClearParticles` (`r_part.c:333`) -- the free list it builds is the
/// allocator's whole state, and its order is what every later spawn depends
/// on.
#[test]
fn clear_particles_differential() {
    let _guard = ctfs::lock();
    for n in [1 as c_int, 2, 7, 512, 2048] {
        fresh_pool(n);
        let run = compare(&format!("clear_particles n={n}"), 1, |side| {
            // SAFETY: driver over that side's pool.
            unsafe { ctest_rpart_clear_particles(side) };
        });
        assert!(run.snap.active.is_empty(), "n={n}: active list not empty");
        assert_eq!(
            run.snap.free,
            (0..n).collect::<Vec<c_int>>(),
            "n={n}: free list is not slot 0 first, ascending"
        );
    }
}

/// `R_ParticleExplosion` (`r_part.c:434`): 1024 particles, the `i & 1`
/// explode/explode2 split, `ramp1[0]`, and `COM_Rand () % 32` / `% 512` -- the
/// two moduli whose negative-remainder behaviour a naive port gets wrong,
/// since `COM_Rand` returns a signed `int32_t`.
#[test]
fn particle_explosion_differential() {
    let _guard = ctfs::lock();

    // Full: the pool is larger than the 1024 the loop wants.
    fresh_pool(2048);
    seed_client(12.25, 12.0);
    let mut org = [11.5 as c_float, -204.75, 3.125];
    let run = compare("particle_explosion full", 0x51a1, |side| {
        // SAFETY: drivers over that side's pool; `org` is a live [f32; 3].
        unsafe {
            ctest_rpart_clear_particles(side);
            ctest_rpart_particle_explosion(side, org.as_mut_ptr());
        }
    });
    assert_eq!(run.snap.active.len(), 1024, "expected 1024 particles");
    assert_eq!(run.snap.free.len(), 1024, "expected 1024 slots left");
    assert_ne!(run.snap.hash, HASH_SEED, "the hash never moved");
    // Positive control on the branch split: the active list is reverse spawn
    // order, so the head is i = 1023 (odd -> pt_explode).
    assert_eq!(run.snap.active[0].ty, 4, "head should be pt_explode");
    assert_eq!(run.snap.active[1].ty, 5, "next should be pt_explode2");

    // Exhaustion mid-loop: the `if (!free_particles) return;` arm.
    fresh_pool(300);
    let run = compare("particle_explosion exhausted", 0x51a2, |side| {
        // SAFETY: as above.
        unsafe {
            ctest_rpart_clear_particles(side);
            ctest_rpart_particle_explosion(side, org.as_mut_ptr());
        }
    });
    assert_eq!(run.snap.active.len(), 300, "the pool should be exhausted");
    assert!(run.snap.free.is_empty(), "free list should be empty");
}

/// `R_ParticleExplosion2` (`r_part.c:477`) -- `colorStart + (colorMod %
/// colorLength)` cycling, at lengths that divide 512 and lengths that do not,
/// and at a length larger than the particle count so the modulo never wraps.
#[test]
fn particle_explosion2_differential() {
    let _guard = ctfs::lock();
    let cases: [(c_int, c_int); 6] = [(0, 1), (100, 3), (224, 7), (0, 8), (250, 256), (16, 600)];
    for (start, length) in cases {
        fresh_pool(2048);
        seed_client(3.5, 3.0);
        let mut org = [0.0 as c_float, 64.5, -8.25];
        let run = compare(
            &format!("particle_explosion2 start={start} length={length}"),
            0x2 + start as u64,
            |side| {
                // SAFETY: drivers over that side's pool.
                unsafe {
                    ctest_rpart_clear_particles(side);
                    ctest_rpart_particle_explosion2(side, org.as_mut_ptr(), start, length);
                }
            },
        );
        assert_eq!(run.snap.active.len(), 512, "expected 512 particles");
        // Positive control on the cycle: the head is colorMod 511.
        let expect = (start + (511 % length)) as f32;
        assert_eq!(
            run.snap.active[0].color,
            expect.to_bits(),
            "head colour should be {expect}"
        );
        assert!(
            run.snap.active.iter().all(|p| p.ty == 6),
            "every particle should be pt_blob"
        );
    }

    // Exhaustion arm.
    fresh_pool(64);
    let mut org = [0.0 as c_float; 3];
    let run = compare("particle_explosion2 exhausted", 0x2f, |side| {
        // SAFETY: as above.
        unsafe {
            ctest_rpart_clear_particles(side);
            ctest_rpart_particle_explosion2(side, org.as_mut_ptr(), 9, 5);
        }
    });
    assert_eq!(run.snap.active.len(), 64);
}

/// `R_BlobExplosion` (`r_part.c:510`): the `i & 1` blob/blob2 split, the two
/// `66 + COM_Rand () % 6` / `150 + COM_Rand () % 6` colour draws, and
/// `cl.time + 1 + (COM_Rand () & 8) * 0.05` -- a `double`-width expression
/// narrowed onto a `float` field.
#[test]
fn blob_explosion_differential() {
    let _guard = ctfs::lock();

    fresh_pool(2048);
    // A cl.time whose sum with the 0.05-scaled term is not representable
    // exactly in either width.
    seed_client(103.7, 103.6);
    let mut org = [-13.375 as c_float, 7.0, 512.5];
    let run = compare("blob_explosion full", 0xb10b, |side| {
        // SAFETY: drivers over that side's pool.
        unsafe {
            ctest_rpart_clear_particles(side);
            ctest_rpart_blob_explosion(side, org.as_mut_ptr());
        }
    });
    assert_eq!(run.snap.active.len(), 1024);
    assert_eq!(run.snap.active[0].ty, 6, "head should be pt_blob");
    assert_eq!(run.snap.active[1].ty, 7, "next should be pt_blob2");

    fresh_pool(9);
    let run = compare("blob_explosion exhausted", 0xb10c, |side| {
        // SAFETY: as above.
        unsafe {
            ctest_rpart_clear_particles(side);
            ctest_rpart_blob_explosion(side, org.as_mut_ptr());
        }
    });
    assert_eq!(run.snap.active.len(), 9);
    assert!(run.snap.free.is_empty());
}

/// `R_RunParticleEffect` (`r_part.c:554`) -- both arms. `count == 1024` is the
/// rocket-explosion arm (identical body to `R_ParticleExplosion`); everything
/// else is the `pt_slowgrav` arm with `(color & ~7) + (COM_Rand () & 7)` and
/// `dir[j] * 15`.
#[test]
fn run_particle_effect_differential() {
    let _guard = ctfs::lock();
    let cases: [(c_int, c_int); 7] = [
        (0, 0),
        (73, 1),
        (73, 5),
        (255, 20),
        (0, 1023),
        (12, 1024),
        (7, 2000),
    ];
    for (color, count) in cases {
        fresh_pool(2048);
        seed_client(41.1, 41.0);
        let mut org = [1.5 as c_float, -2.25, 3.75];
        let mut dir = [0.375 as c_float, -1.0, 0.0625];
        let run = compare(
            &format!("run_particle_effect color={color} count={count}"),
            0x4321 + count as u64,
            |side| {
                // SAFETY: drivers over that side's pool; both arrays are live.
                unsafe {
                    ctest_rpart_clear_particles(side);
                    ctest_rpart_run_particle_effect(
                        side,
                        org.as_mut_ptr(),
                        dir.as_mut_ptr(),
                        color,
                        count,
                    );
                }
            },
        );
        let expect = count.clamp(0, 2048) as usize;
        assert_eq!(
            run.snap.active.len(),
            expect,
            "color={color} count={count}: particle count"
        );
        if count > 0 && count != 1024 {
            assert!(
                run.snap.active.iter().all(|p| p.ty == 2),
                "color={color} count={count}: every particle should be pt_slowgrav"
            );
        }
        if count == 1024 {
            assert_eq!(run.snap.active[0].ty, 4, "head should be pt_explode");
        }
    }
}

/// `R_ParseParticleEffect` (`r_part.c:409`) -- the wire decode: three
/// `MSG_ReadCoord` under two protocol-flag settings, three `MSG_ReadChar` at
/// `1.0 / 16`, and the `msgcount == 255 -> 1024` promotion. Reading is
/// compared through `msg_readcount` and `msg_badread` as well as through the
/// pool, so a decode that lands on the same particles by consuming a different
/// number of bytes still fails.
#[test]
fn parse_particle_effect_differential() {
    let _guard = ctfs::lock();

    // (protocolflags, coord bytes per axis)
    for &(flags, floatcoord) in &[(0 as c_uint, false), (PRFL_FLOATCOORD, true)] {
        for &(msgcount, color) in &[(0u8, 0u8), (10, 73), (255, 12), (37, 255)] {
            let mut msg: Vec<u8> = Vec::new();
            if floatcoord {
                for v in [1.5f32, -2.25, 300.125] {
                    msg.extend_from_slice(&v.to_le_bytes());
                }
            } else {
                // protocol.h short coords: value * 8, little-endian.
                for v in [12i16, -344, 4095] {
                    msg.extend_from_slice(&v.to_le_bytes());
                }
            }
            msg.extend_from_slice(&[0x0f, 0xf1, 0x00]); // dir chars: +15, -15, 0
            msg.push(msgcount);
            msg.push(color);

            fresh_pool(2048);
            // SAFETY: writes both net_message buffers; `msg` is live.
            unsafe {
                ctest_svuser_load_message(msg.as_ptr(), msg.len() as c_int);
                ctest_rpart_seed_client(9.5, 9.4, CA_CONNECTED, c"e1m1".as_ptr(), flags);
            }

            let mut reads = [(0 as c_int, 0 as c_int); 2];
            let mut snaps: Vec<Run> = Vec::with_capacity(2);
            for (slot, &side) in SIDES.iter().enumerate() {
                let run = run_side(side, 0x9911 + msgcount as u64, |side| {
                    // SAFETY: fixture drivers; the message is loaded above.
                    unsafe {
                        ctest_rpart_clear_particles(side);
                        ctest_cltent_begin_reading(side);
                        ctest_rpart_parse_particle_effect(side);
                    }
                });
                // SAFETY: fixture accessors over that side's cursor.
                reads[slot] = unsafe {
                    (
                        ctest_cltent_get_readcount(side),
                        ctest_cltent_get_badread(side),
                    )
                };
                snaps.push(run);
            }

            let what = format!("parse_particle_effect flags={flags} count={msgcount}");
            assert_snap_eq(&what, &snaps[0].snap, &snaps[1].snap);
            assert_eq!(
                snaps[0].rand_after, snaps[1].rand_after,
                "{what}: COM_Rand consumption diverged"
            );
            assert_eq!(reads[0], reads[1], "{what}: msg cursor / badread diverged");

            // Positive controls: the decode really consumed the whole record,
            // never went bad, and the 255 promotion really happened.
            assert_eq!(
                reads[0].0,
                msg.len() as c_int,
                "{what}: expected the whole message to be consumed"
            );
            assert_eq!(reads[0].1, 0, "{what}: msg_badread was set");
            let expect = if msgcount == 255 {
                1024
            } else {
                msgcount as usize
            };
            assert_eq!(snaps[0].snap.active.len(), expect, "{what}: particle count");
        }
    }
}

/// `R_LavaSplash` (`r_part.c:611`) and `R_TeleportSplash` (`r_part.c:652`) --
/// the two nested-loop splashes. Both end in `VectorNormalize` +
/// `VectorScale`, so they are the file's only `sqrt` path, and
/// `R_TeleportSplash`'s `i = j = 0` iteration normalises a vector whose first
/// two components are zero.
#[test]
fn splash_differential() {
    let _guard = ctfs::lock();

    fresh_pool(2048);
    seed_client(77.125, 77.0);
    let mut org = [64.0 as c_float, -128.5, 16.25];

    let run = compare("lava_splash", 0x1a7a, |side| {
        // SAFETY: drivers over that side's pool.
        unsafe {
            ctest_rpart_clear_particles(side);
            ctest_rpart_lava_splash(side, org.as_mut_ptr());
        }
    });
    assert_eq!(run.snap.active.len(), 32 * 32, "lava splash particle count");
    assert!(
        run.snap.active.iter().all(|p| p.ty == 2),
        "lava splash should be pt_slowgrav"
    );

    let run = compare("teleport_splash", 0x7e1e, |side| {
        // SAFETY: as above.
        unsafe {
            ctest_rpart_clear_particles(side);
            ctest_rpart_teleport_splash(side, org.as_mut_ptr());
        }
    });
    assert_eq!(
        run.snap.active.len(),
        8 * 8 * 14,
        "teleport splash particle count"
    );

    // Exhaustion arms.
    fresh_pool(101);
    let run = compare("lava_splash exhausted", 0x1a7b, |side| {
        // SAFETY: as above.
        unsafe {
            ctest_rpart_clear_particles(side);
            ctest_rpart_lava_splash(side, org.as_mut_ptr());
        }
    });
    assert_eq!(run.snap.active.len(), 101);

    fresh_pool(45);
    let run = compare("teleport_splash exhausted", 0x7e1f, |side| {
        // SAFETY: as above.
        unsafe {
            ctest_rpart_clear_particles(side);
            ctest_rpart_teleport_splash(side, org.as_mut_ptr());
        }
    });
    assert_eq!(run.snap.active.len(), 45);
}

/// `R_RocketTrail` (`r_part.c:695`) -- all six trail types, the `type >= 128`
/// `dec = 1` variant, the `case 4` extra `len -= 3`, the zero-length early
/// exit, and the in-place advance of `start`.
///
/// The whole sequence runs against one side and then, identically, against the
/// other, because `tracercount` (`r_part.c:702`) is a function-local static
/// with no accessor: running the same calls in the same order is what keeps
/// the two sides' copies in step. Nothing else in this file uses trail types
/// 3, 5, 131 or 133.
#[test]
fn rocket_trail_differential() {
    let _guard = ctfs::lock();

    // (start, end, type). Lengths are short so the whole sequence fits the
    // pool until the deliberate exhaustion at the end.
    let calls: [([c_float; 3], [c_float; 3], c_int); 13] = [
        ([0.0, 0.0, 0.0], [30.0, 0.0, 0.0], 0),     // rocket trail
        ([0.0, 0.0, 0.0], [0.0, 21.0, 0.0], 1),     // smoke
        ([4.5, -3.25, 1.0], [4.5, -3.25, 25.0], 2), // blood
        ([0.0, 0.0, 0.0], [18.0, 18.0, 0.0], 3),    // tracer
        ([1.0, 2.0, 3.0], [40.0, 2.0, 3.0], 4),     // slight blood
        ([0.0, 0.0, 0.0], [0.0, 0.0, 27.0], 5),     // tracer, other hue
        ([-8.0, 0.5, 0.5], [8.0, 0.5, 0.5], 6),     // voor trail
        ([0.0, 0.0, 0.0], [12.0, 5.0, 2.0], 128),   // dec = 1, type 0
        ([0.0, 0.0, 0.0], [9.0, 0.0, 0.0], 131),    // dec = 1, tracer
        ([0.0, 0.0, 0.0], [0.0, 9.0, 0.0], 133),    // dec = 1, tracer
        ([7.0, 7.0, 7.0], [7.0, 7.0, 7.0], 0),      // zero length: no-op
        ([0.0, 0.0, 0.0], [0.5, 0.0, 0.0], 2),      // len < dec: one step
        ([0.0, 0.0, 0.0], [4000.0, 0.0, 0.0], 1),   // exhausts the pool
    ];

    fresh_pool(1200);
    seed_client(5.5, 5.4);

    let mut per_side: Vec<(Run, Vec<[u32; 3]>)> = Vec::with_capacity(2);
    for &side in &SIDES {
        let mut starts: Vec<[u32; 3]> = Vec::with_capacity(calls.len());
        let run = run_side(side, 0x40c4, |side| {
            // SAFETY: fixture drivers; every buffer below is a live [f32; 3]
            // and `start` is written in place by design.
            unsafe { ctest_rpart_clear_particles(side) };
            for (start, end, ty) in calls.iter() {
                let mut s = *start;
                let mut e = *end;
                // SAFETY: as above.
                unsafe { ctest_rpart_rocket_trail(side, s.as_mut_ptr(), e.as_mut_ptr(), *ty) };
                starts.push([s[0].to_bits(), s[1].to_bits(), s[2].to_bits()]);
            }
        });
        per_side.push((run, starts));
    }

    assert_snap_eq("rocket_trail", &per_side[0].0.snap, &per_side[1].0.snap);
    assert_eq!(
        per_side[0].0.rand_after, per_side[1].0.rand_after,
        "rocket_trail: COM_Rand consumption diverged"
    );
    for (i, (a, b)) in per_side[0].1.iter().zip(per_side[1].1.iter()).enumerate() {
        assert_eq!(a, b, "rocket_trail call {i}: the advanced `start` diverged");
    }

    // Positive controls.
    let snap = &per_side[0].0.snap;
    assert!(
        snap.free.is_empty(),
        "the last trail should exhaust the pool"
    );
    assert_eq!(snap.active.len(), 1200, "the pool should be full");
    // Call 10 is zero-length: `start` must come back untouched.
    assert_eq!(
        per_side[0].1[10],
        [7.0f32.to_bits(); 3],
        "the zero-length trail moved `start`"
    );
    // Every pt_fire particle's colour must come out of ramp3, whose last two
    // of eight entries are the zeros r_part.c:36 leaves implicit.
    let ramp3: [f32; 8] = [0x6d as f32, 0x6b as f32, 6.0, 5.0, 4.0, 3.0, 0.0, 0.0];
    assert!(
        snap.active
            .iter()
            .filter(|p| p.ty == 3)
            .all(|p| ramp3.iter().any(|c| c.to_bits() == p.color)),
        "a pt_fire particle has a colour outside ramp3"
    );
    assert!(
        snap.active.iter().any(|p| p.ty == 3),
        "no pt_fire particles were produced"
    );
}

/// `R_EntityParticles` (`r_part.c:271`) -- the 162-normal halo. Its `angle =
/// cl.time * avelocities[i][k]` / `sin` / `cos` chain is the file's only
/// trigonometry, and `avelocities` is filled lazily on the first call from 486
/// `COM_Rand` draws.
///
/// `avelocities` has no reset entry point, so this test owns every
/// `R_EntityParticles` call in the file: it runs its whole sequence against
/// one side and then against the other, from the same seed, so both sides fill
/// their copy from the same 486 draws.
#[test]
fn entity_particles_differential() {
    let _guard = ctfs::lock();

    let origins: [[c_float; 3]; 3] = [
        [0.0, 0.0, 0.0],
        [128.5, -64.25, 32.0],
        [-1000.125, 4.0, 0.5],
    ];
    // Distinct cl.time values, so the sin/cos arguments differ per call and a
    // stale-angle bug cannot hide.
    let times: [c_double; 3] = [0.0, 13.375, 250.7];

    // 400 slots: the first two calls place all 162 of their particles, the
    // third runs out part-way, so the `if (!free_particles) return;` arm is
    // covered too.
    fresh_pool(400);

    let mut runs: Vec<Run> = Vec::with_capacity(2);
    for &side in &SIDES {
        let run = run_side(side, 0xe27, |side| {
            // SAFETY: fixture drivers; each `org` is a live [f32; 3].
            unsafe { ctest_rpart_clear_particles(side) };
            for (org, t) in origins.iter().zip(times.iter()) {
                let mut org = *org;
                seed_client(*t, *t - 0.1);
                // SAFETY: as above.
                unsafe { ctest_rpart_entity_particles(side, org.as_mut_ptr()) };
            }
        });
        runs.push(run);
    }

    assert_snap_eq("entity_particles", &runs[0].snap, &runs[1].snap);
    assert_eq!(
        runs[0].rand_after, runs[1].rand_after,
        "entity_particles: COM_Rand consumption diverged (the lazy avelocities \
         fill is 486 draws)"
    );

    // Positive controls: 162 from the first call, 162 from the second, then
    // the third exhausts the 400-slot pool 76 particles in.
    let snap = &runs[0].snap;
    assert_eq!(snap.active.len(), 400, "the pool should be exhausted");
    assert!(snap.free.is_empty());
    assert!(
        snap.active.iter().all(|p| p.ty == 4),
        "every particle should be pt_explode"
    );
    assert!(
        snap.active
            .iter()
            .all(|p| p.color == (0x6f as f32).to_bits()),
        "every particle should be colour 0x6f"
    );
}

/// `CL_RunParticles` (`r_part.c:803`) over a pool holding all eight particle
/// types: the per-type ramp advance, the `ramp3`/`ramp1`/`ramp2` lookups, the
/// `die = -1` kill, the two unlink loops (head and mid-list), and the gravity
/// term `frametime * sv_gravity.value * 0.05`, which is computed in `double`
/// and stored to a `float`.
#[test]
fn run_particles_mixed_pool_differential() {
    let _guard = ctfs::lock();

    fresh_pool(2048);
    // A gravity that is not a round binary value, so the double-width product
    // narrows inexactly.
    // SAFETY: writes both sides' sv_gravity.
    unsafe { ctest_rpart_set_gravity(800.3) };

    let trail_a = [0.0 as c_float, 0.0, 0.0];
    let mut trail_b = [90.0 as c_float, 0.0, 0.0];
    let mut org = [10.5 as c_float, -3.25, 60.0];
    let mut dir = [0.5 as c_float, 0.25, -1.0];

    // 14 frames at 0.1s: time1 = 0.5, time2 = 1.0, time3 = 1.5, so pt_fire
    // crosses its `>= 6` gate at frame 12, pt_explode its `>= 8` at frame 8
    // and pt_explode2 at frame 6 -- every ramp arm and every kill arm runs.
    let frames = 14usize;

    let mut runs: Vec<Run> = Vec::with_capacity(2);
    let mut per_frame: Vec<Vec<Snap>> = Vec::with_capacity(2);
    for &side in &SIDES {
        let mut snaps: Vec<Snap> = Vec::with_capacity(frames);
        let run = run_side(side, 0xc1ea, |side| {
            seed_client(0.0, 0.0);
            // SAFETY: fixture drivers; every buffer is a live [f32; 3].
            unsafe {
                ctest_rpart_clear_particles(side);
                // pt_fire (30), pt_grav (10), pt_static (10)
                let mut a = trail_a;
                ctest_rpart_rocket_trail(side, a.as_mut_ptr(), trail_b.as_mut_ptr(), 0);
                let mut a = trail_a;
                let mut b = [30.0 as c_float, 0.0, 0.0];
                ctest_rpart_rocket_trail(side, a.as_mut_ptr(), b.as_mut_ptr(), 2);
                let mut a = trail_a;
                let mut b = [0.0 as c_float, 30.0, 0.0];
                ctest_rpart_rocket_trail(side, a.as_mut_ptr(), b.as_mut_ptr(), 6);
                // pt_slowgrav (20)
                ctest_rpart_run_particle_effect(side, org.as_mut_ptr(), dir.as_mut_ptr(), 73, 20);
                // pt_blob / pt_blob2 (1024)
                ctest_rpart_blob_explosion(side, org.as_mut_ptr());
                // pt_explode / pt_explode2 (fills the rest)
                ctest_rpart_particle_explosion(side, org.as_mut_ptr());
            }

            for f in 0..frames {
                let t = (f + 1) as c_double * 0.1;
                seed_client(t, t - 0.1);
                // SAFETY: fixture driver.
                unsafe { ctest_rpart_run_particles(side) };
                snaps.push(Snap::take(side));
            }
        });
        per_frame.push(snaps);
        runs.push(run);
    }

    for (f, (c, rust)) in per_frame[0].iter().zip(per_frame[1].iter()).enumerate() {
        assert_snap_eq(&format!("run_particles frame {f}"), c, rust);
    }
    assert_snap_eq("run_particles final", &runs[0].snap, &runs[1].snap);
    assert_eq!(
        runs[0].rand_after, runs[1].rand_after,
        "run_particles: COM_Rand consumption diverged"
    );

    // Positive controls: the pool really started full and really drained, and
    // every one of the eight types was present at the start.
    let first = &per_frame[0][0];
    assert!(
        first.active.len() > 2000,
        "the seeded pool should be nearly full, got {}",
        first.active.len()
    );
    let mut seen = [false; 8];
    for p in &first.active {
        seen[p.ty as usize] = true;
    }
    assert_eq!(
        seen, [true; 8],
        "the seeded pool does not cover all eight ptype_t values"
    );
    let last = &per_frame[0][frames - 1];
    assert!(
        last.active.len() < first.active.len(),
        "no particles expired over {frames} frames"
    );
    assert!(
        !last.free.is_empty(),
        "expired particles were never returned to the free list"
    );
    assert_ne!(
        first.hash, last.hash,
        "the particle hash did not move across the run"
    );
}

/// `CL_RunParticles`' unlink loops on a pool small enough to name every
/// particle, plus the `q_max (0.0, cl.time - cl.oldtime)` clamp on a frame
/// that runs backwards.
#[test]
fn run_particles_expiry_order_differential() {
    let _guard = ctfs::lock();

    fresh_pool(8);
    // SAFETY: writes both sides' sv_gravity.
    unsafe { ctest_rpart_set_gravity(800.0) };

    let mut org = [0.0 as c_float; 3];
    let mut dir = [1.0 as c_float, 0.0, 0.0];

    let mut per_frame: Vec<Vec<Snap>> = Vec::with_capacity(2);
    let mut runs: Vec<Run> = Vec::with_capacity(2);
    for &side in &SIDES {
        let mut snaps: Vec<Snap> = Vec::with_capacity(4);
        let run = run_side(side, 0x8e8e, |side| {
            seed_client(0.0, 0.0);
            // SAFETY: fixture drivers. `die = cl.time + 0.1 * (COM_Rand () %
            // 5)` gives this pool a spread of lifetimes, so the frames below
            // kill particles at the head of the list and in the middle of it.
            unsafe {
                ctest_rpart_clear_particles(side);
                ctest_rpart_run_particle_effect(side, org.as_mut_ptr(), dir.as_mut_ptr(), 73, 8);
            }
            for t in [0.05 as c_double, 0.15, 0.25, 0.45] {
                seed_client(t, t - 0.05);
                // SAFETY: fixture driver.
                unsafe { ctest_rpart_run_particles(side) };
                snaps.push(Snap::take(side));
            }
            // A frame that runs backwards: q_max clamps frametime to 0, so
            // nothing moves, but the expiry loops still run against cl.time.
            seed_client(0.40, 0.60);
            // SAFETY: fixture driver.
            unsafe { ctest_rpart_run_particles(side) };
            snaps.push(Snap::take(side));
        });
        per_frame.push(snaps);
        runs.push(run);
    }

    for (f, (c, rust)) in per_frame[0].iter().zip(per_frame[1].iter()).enumerate() {
        assert_snap_eq(&format!("run_particles small pool frame {f}"), c, rust);
    }
    assert_eq!(runs[0].rand_after, runs[1].rand_after);

    // Positive controls: particles really died, and the free list really
    // received them (LIFO, so its head is the most recently killed slot).
    let first = &per_frame[0][0];
    let last = per_frame[0].last().unwrap();
    assert_eq!(first.numparticles, 8);
    assert!(
        last.active.len() < 8,
        "no particle expired across the five frames"
    );
    assert_eq!(
        last.active.len() + last.free.len(),
        8,
        "particles went missing: {} active + {} free",
        last.active.len(),
        last.free.len()
    );
}

/// `Harness_HashParticles` (`r_part.c:927`) -- the entry point the demo
/// state-hash chain actually calls. Its `if (!particles)` early return is the
/// arm every other test here cannot reach, and the seed pass-through is what
/// keeps `Harness_HashClient`'s chain intact.
#[test]
fn hash_particles_differential() {
    let _guard = ctfs::lock();

    // Populated pool: the hash must move, and must move to the same place.
    fresh_pool(64);
    seed_client(2.5, 2.4);
    let mut org = [3.5 as c_float, 4.25, -5.0];
    let run = compare("hash_particles populated", 0x4a54, |side| {
        // SAFETY: fixture drivers.
        unsafe {
            ctest_rpart_clear_particles(side);
            ctest_rpart_particle_explosion(side, org.as_mut_ptr());
        }
    });
    assert_eq!(run.snap.active.len(), 64);
    assert_ne!(run.snap.hash, HASH_SEED, "the hash did not absorb the pool");

    // Chaining: a different seed must give a different digest, on both sides.
    // SAFETY: fixture accessors over each side's pool.
    let (c1, r1) = unsafe { (ctest_rpart_hash(1, 1), ctest_rpart_hash(0, 1)) };
    // SAFETY: as above -- the hash never mutates the pool.
    let (c2, r2) = unsafe { (ctest_rpart_hash(1, 2), ctest_rpart_hash(0, 2)) };
    assert_eq!(c1, r1, "hash_particles diverged at seed 1");
    assert_eq!(c2, r2, "hash_particles diverged at seed 2");
    assert_ne!(c1, c2, "the seed is not part of the digest");

    // Empty active list, pool present: the count-only digest.
    // SAFETY: fixture drivers.
    unsafe {
        ctest_rpart_clear_particles(1);
        ctest_rpart_clear_particles(0);
    }
    // SAFETY: fixture accessors.
    let (c_empty, r_empty) = unsafe {
        (
            ctest_rpart_hash(1, HASH_SEED),
            ctest_rpart_hash(0, HASH_SEED),
        )
    };
    assert_eq!(c_empty, r_empty, "empty-pool hash diverged");
    assert_ne!(
        c_empty, HASH_SEED,
        "an empty pool should still hash its zero count"
    );

    // No pool at all: `if (!particles) return h;`.
    // SAFETY: fixture reset of both sides' pool pointers.
    unsafe { ctest_rpart_drop_pool() };
    // SAFETY: fixture accessors.
    let (c_none, r_none) = unsafe {
        (
            ctest_rpart_hash(1, HASH_SEED),
            ctest_rpart_hash(0, HASH_SEED),
        )
    };
    assert_eq!(c_none, r_none, "no-pool hash diverged");
    assert_eq!(
        c_none, HASH_SEED,
        "the no-pool arm must pass the seed through"
    );
}

/// `R_ReadPointFile_f` (`r_part.c:350`) -- the `.pts` loader. Four paths: the
/// `cls.state != ca_connected` early return, the open failure, a successful
/// parse (`(-c) & 15` colours, `die = 99999`, `pt_static`, zeroed `vel`), and
/// the "Not enough free particles" break. The console text is part of the
/// comparison, since three of the four paths produce nothing else.
#[test]
fn read_point_file_differential() {
    let _guard = ctfs::lock();

    let root = std::env::temp_dir().join(format!("quake-ctest-rpart-{}", std::process::id()));
    let dir = root.join("ptsgame");
    std::fs::create_dir_all(dir.join("maps")).unwrap();
    for side in ctfs::BOTH {
        ctfs::setup(side, &[&root], 0, c"ptsgame");
    }

    // Five well-formed points, then a line fscanf cannot complete.
    std::fs::write(
        dir.join("maps").join("pointy.pts"),
        "1 2 3\n-4.5 6.25 0\n100 -200 300\n0.125 0.25 0.5\n7 8 9\nstop\n",
    )
    .unwrap();

    // 1. Not connected.
    fresh_pool(64);
    // SAFETY: writes both sides' cl/cls; the name is NUL-terminated.
    unsafe { ctest_rpart_seed_client(1.0, 0.9, CA_DISCONNECTED, c"pointy".as_ptr(), 0) };
    let run = compare("read_point_file disconnected", 1, |side| {
        // SAFETY: fixture drivers.
        unsafe {
            ctest_rpart_clear_particles(side);
            ctest_rpart_read_point_file(side);
        }
    });
    assert!(
        run.snap.active.is_empty(),
        "the early return spawned particles"
    );
    assert!(run.con.is_empty(), "the early return printed {:?}", run.con);

    // 2. Connected, file missing.
    fresh_pool(64);
    // SAFETY: as above.
    unsafe { ctest_rpart_seed_client(1.0, 0.9, CA_CONNECTED, c"nosuch".as_ptr(), 0) };
    let run = compare("read_point_file missing", 1, |side| {
        // SAFETY: fixture drivers.
        unsafe {
            ctest_rpart_clear_particles(side);
            ctest_rpart_read_point_file(side);
        }
    });
    assert!(run.snap.active.is_empty());
    assert_eq!(
        run.con.len(),
        1,
        "expected one console line, got {:?}",
        run.con
    );
    assert!(
        run.con[0].contains("couldn't open maps/nosuch.pts"),
        "unexpected console line {:?}",
        run.con
    );

    // 3. Connected, five points, room for all of them.
    fresh_pool(64);
    // SAFETY: as above.
    unsafe { ctest_rpart_seed_client(1.0, 0.9, CA_CONNECTED, c"pointy".as_ptr(), 0) };
    let run = compare("read_point_file ok", 1, |side| {
        // SAFETY: fixture drivers.
        unsafe {
            ctest_rpart_clear_particles(side);
            ctest_rpart_read_point_file(side);
        }
    });
    assert_eq!(run.snap.active.len(), 5, "expected five points");
    assert_eq!(run.con.len(), 2, "unexpected console output {:?}", run.con);
    assert!(run.con[1].contains("5 points read"), "{:?}", run.con);
    assert!(
        run.snap.active.iter().all(|p| p.ty == 0),
        "points should be pt_static"
    );
    assert!(
        run.snap
            .active
            .iter()
            .all(|p| p.die == (99999.0f32).to_bits()),
        "points should never expire"
    );
    assert!(
        run.snap.active.iter().all(|p| p.vel == [0u32; 3]),
        "points should have zero velocity"
    );
    // Reverse spawn order: the head is c = 5, colour (-5) & 15 == 11.
    assert_eq!(run.snap.active[0].color, (11.0f32).to_bits());
    assert_eq!(
        run.snap.active[0].org,
        [7.0f32.to_bits(), 8.0f32.to_bits(), 9.0f32.to_bits()]
    );

    // 4. Connected, three free slots, five points: the break arm.
    fresh_pool(3);
    let run = compare("read_point_file exhausted", 1, |side| {
        // SAFETY: fixture drivers.
        unsafe {
            ctest_rpart_clear_particles(side);
            ctest_rpart_read_point_file(side);
        }
    });
    assert_eq!(run.snap.active.len(), 3);
    assert_eq!(run.con.len(), 3, "unexpected console output {:?}", run.con);
    assert!(
        run.con[1].contains("Not enough free particles"),
        "{:?}",
        run.con
    );
    // `c` is incremented before the free-list test, so the count reported is
    // one higher than the number of particles that made it in.
    assert!(run.con[2].contains("4 points read"), "{:?}", run.con);

    let _ = std::fs::remove_dir_all(&root);
}

// Keeps the linker honest about the archive this suite depends on.
const _: fn() = || {
    let _ = quake_ctest::fs::lock;
    let _: Option<*mut c_void> = None;
};
