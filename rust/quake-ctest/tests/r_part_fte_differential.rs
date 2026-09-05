//! Differential gate for `Quake/r_part_fte.c` -- the FTE particle system's
//! simulation half. Rust migration Phase 7, M10f-2 (task T10.5, second half).
//!
//! Read `stubs/r_part_fte_ref.c`'s header first. It explains why `r_part_fte.c`
//! is composed into that TU instead of listed in `build.rs`'s `C_SOURCES`, why
//! the rename prefix is `c_ref_fte_` rather than `c_ref_`, which objects each
//! side owns, and which two engine entry points are counted rather than
//! executed.
//!
//! ## What this gate is for
//!
//! `r_part_fte.c` is a scripted particle system: a text config declares effect
//! types, and every spawn, trail, weather burst and per-frame update is driven
//! from the parsed `part_type_t` table. The table is therefore the compiler
//! output and the particles are the program's result, so this gate compares
//! both:
//!
//! * `ctest_ftepart_hash_types` digests every `part_type_t` by value, with each
//!   pointer member replaced by a side-independent derivation (a type index for
//!   `slooks` and `nexttorun`, a list length for `particles` / `beams` /
//!   `clippeddecals`, a null flag for `sounds` / `ramp` / `looks.texture`) and
//!   the `sounds` and `ramp` arrays hashed by content. Padding is included and
//!   is deterministic, because `r_part_fte.c:960` memsets each entry when it is
//!   created.
//! * `ctest_ftepart_hash_particles` digests every live particle of every type
//!   in list order, as raw bits, so `-0.0` and a NaN payload are differences.
//! * `ctest_ftepart_hash_runlist` digests the run list as a sequence of type
//!   indices, because the order the update visits types in is observable
//!   through the deferred queues even when the set of types is identical.
//! * `ctest_ftepart_read_ints` covers the free lists, the deferred queues, the
//!   flattened update array and the three seam counters.
//!
//! ## Degenerate-gate defences
//!
//! A bit-exact differential passes whenever both sides do nothing, so every
//! test also asserts something positive: a type count that grew, a live
//! particle count above zero, a hash that moved off its seed.
//!
//! The sharpest defence is the shared `COM_Rand` generator (`stubs.c:233`),
//! which `c_ref_prelude.h` does not rename, so both sides draw from the one
//! object. Each side is reseeded with the same value and the generator's *next*
//! output is sampled once that side has finished: equal next-values prove both
//! sides consumed exactly the same number of draws through exactly the same
//! branches. Every spawn path in this module is randomised, so this is a strong
//! check that no amount of agreeing-by-accident can fake.
//!
//! ## Per-side state and test ordering
//!
//! Two things persist for the life of the process: the `part_type` table, which
//! `R_ParticleDesc_Callback` blanks but never shrinks (`R_Particles_KillAllEffects`,
//! `r_part_fte.c:3595`), and the cvar and command registrations
//! `PScript_InitParticles` performs. So no test here asserts an absolute count.
//! Each one drives both sides through the identical sequence under
//! `ctfs::lock`, and compares only side against side or before against after,
//! which stays true whatever order the harness runs them in. Config names are
//! unique per test for the same reason.
//!
//! The pools are per-run rather than per-process: `PScript_Startup` allocates
//! them only while `r_numparticles` is zero and `PScript_Shutdown` sets it back
//! to zero, so `shutdown_differential` re-initialises at the end and every
//! fixture sets the same pool sizes before the first `clear(load)`.
//!
//! ## ADR-010
//!
//! The module is single-precision throughout but C's promotion rules put
//! several expressions through `double`: `p_frametime = cl.time - oldtime`
//! narrowed on store, `particletime = cl.time`, `qdl->die < cl.time`, and every
//! `die` computed from `particletime` plus a `float` lifetime. `cl.time` is set
//! here to values whose narrowing is not exact, and `particletime` /
//! `p_frametime` are compared as raw bits by `ctest_ftepart_read_floats`.
//!
//! `r_part_fte.c:186` replaces `sin`/`cos` with the 128-entry
//! `psintable`/`pcostable` lookups for the whole file, so no libm call-through
//! is involved in the spawn modes; `buildsintable` fills both tables and the
//! port shares the glue's storage for them, which is why the tables themselves
//! are not compared separately -- a wrong table shows up immediately in every
//! circle, ball, spiral and telebox spawn.
//!
//! ## Known gaps
//!
//! The rendering half (`r_part_fte.c:5547-6250`) is compiled but never runs:
//! its six Vulkan entry points are aborting doubles in the stub. The decal
//! clipper (`:3928-4307`) stays C in the engine build and is unreachable here
//! because no world model is mounted, so `FtePart_Glue_ClipDecal` is a counter
//! that every test asserts stayed at zero rather than a forward. `P_LoadTexture`
//! is deliberately shared between the sides -- see the stub header. All three
//! are Phase 8 territory.

use core::ffi::{c_char, c_double, c_float, c_int};
use std::ffi::CString;
use std::path::PathBuf;

use quake_ctest::fs as ctfs;

// ---------------------------------------------------------------------------
// Fixture (stubs/r_part_fte_ref.c) and the shared stubs it builds on.

extern "C" {
    fn ctest_ftepart_fs_setup(side: c_int, dir: *const c_char);
    fn ctest_ftepart_fs_teardown(side: c_int);
    fn ctest_ftepart_set_time(t: c_double);
    fn ctest_ftepart_set_limits(side: c_int, maxparticles: c_float, maxdecals: c_float);
    fn ctest_ftepart_set_enabled(side: c_int, fteparticles: c_float, density: c_float);

    fn ctest_ftepart_init(side: c_int) -> c_int;
    fn ctest_ftepart_shutdown(side: c_int) -> c_int;
    fn ctest_ftepart_clear(side: c_int, load: c_int) -> c_int;
    fn ctest_ftepart_find_type(side: c_int, name: *const c_char, out: *mut c_int) -> c_int;
    fn ctest_ftepart_set_desc(side: c_int, value: *const c_char) -> c_int;
    fn ctest_ftepart_run_effect(
        side: c_int,
        org: *const c_float,
        dir: *const c_float,
        color: c_int,
        count: c_int,
        out: *mut c_int,
    ) -> c_int;
    fn ctest_ftepart_run_effect_state(
        side: c_int,
        org: *const c_float,
        dir: *const c_float,
        count: c_float,
        ty: c_int,
        out: *mut c_int,
    ) -> c_int;
    fn ctest_ftepart_run_effect_string(
        side: c_int,
        org: *const c_float,
        dir: *const c_float,
        count: c_float,
        name: *const c_char,
        out: *mut c_int,
    ) -> c_int;
    fn ctest_ftepart_trail(
        side: c_int,
        start: *const c_float,
        end: *const c_float,
        ty: c_int,
        dlkey: c_int,
        out: *mut c_int,
    ) -> c_int;
    fn ctest_ftepart_weather(
        side: c_int,
        minb: *const c_float,
        maxb: *const c_float,
        dir: *const c_float,
        count: c_float,
        colour: c_int,
        efname: *const c_char,
    ) -> c_int;
    fn ctest_ftepart_queue_effect(
        side: c_int,
        org: *const c_float,
        dir: *const c_float,
        count: c_float,
        ty: c_int,
    );
    fn ctest_ftepart_delink_trailstate(side: c_int) -> c_int;
    fn ctest_ftepart_update(side: c_int) -> c_int;
    fn ctest_ftepart_finish_frame(side: c_int);
    fn ctest_ftepart_run_cmd(side: c_int, which: c_int) -> c_int;
    fn ctest_ftepart_normalize2(side: c_int, v: *const c_float, out: *mut c_float) -> c_float;

    fn ctest_ftepart_hash_types(side: c_int, h: u64) -> u64;
    fn ctest_ftepart_hash_queues(side: c_int, h: u64) -> u64;
    fn ctest_ftepart_hash_slooks(side: c_int, h: u64) -> u64;
    fn ctest_ftepart_hash_runlist(side: c_int, h: u64) -> u64;
    fn ctest_ftepart_hash_particles(side: c_int, h: u64) -> u64;
    fn ctest_ftepart_read_ints(side: c_int, out: *mut c_int);
    fn ctest_ftepart_read_floats(side: c_int, out: *mut c_float);
    fn ctest_ftepart_reset_counters();

    // stubs.c
    fn COM_SeedRand(seed: u64);
    fn COM_Rand() -> c_int;
}

/// `side` 1 is the C oracle, 0 the Rust port; the oracle runs first so a
/// divergence report reads "C then Rust".
const SIDES: [c_int; 2] = [1, 0];

/// `Quake/harness.c:64` -- the FNV-1a basis `Harness_Hash64` starts from.
const HASH_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// Number of `int`s `ctest_ftepart_read_ints` writes; see its comment in the
/// stub for what each slot means.
const NUM_INTS: usize = 16;

/// The clock every fixture starts from, so `particletime` is identical on both
/// sides no matter which gate ran first.
const FIXTURE_TIME: c_double = 0.25;

const I_NUMTYPES: usize = 0;
const I_RUNLIST: usize = 1;
const I_FREE_PARTICLES: usize = 2;
const I_UPDATES: usize = 5;
const I_Q_EFFECTS: usize = 6;
const I_Q_TRAILS: usize = 7;
const I_Q_DLIGHTS: usize = 9;
const I_CLIPDECAL: usize = 13;
const I_LIVE: usize = 15;

/// Everything one side produced during one scenario step.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Snapshot {
    raised: c_int,
    ret: c_int,
    types: u64,
    runlist: u64,
    particles: u64,
    queues: u64,
    ints: [c_int; NUM_INTS],
    floats: [u32; 2],
    next_rand: c_int,
}

impl Snapshot {
    /// Reads one side's whole observable state. `raised` and `ret` come from
    /// the driver that was just run.
    fn read(side: c_int, raised: c_int, ret: c_int) -> Self {
        let mut ints = [0 as c_int; NUM_INTS];
        let mut floats = [0.0 as c_float; 2];
        // SAFETY: `ints` and `floats` are the sizes the fixture documents, and
        // every reader is a pure walk over that side's own storage.
        unsafe {
            ctest_ftepart_read_ints(side, ints.as_mut_ptr());
            ctest_ftepart_read_floats(side, floats.as_mut_ptr());
            Self {
                raised,
                ret,
                types: ctest_ftepart_hash_types(side, HASH_BASIS),
                runlist: ctest_ftepart_hash_runlist(side, HASH_BASIS),
                particles: ctest_ftepart_hash_particles(side, HASH_BASIS),
                queues: ctest_ftepart_hash_queues(side, HASH_BASIS),
                ints,
                // COMPAT: ADR-010 -- compared as raw bits so -0.0 and a NaN
                // payload are differences.
                floats: [floats[0].to_bits(), floats[1].to_bits()],
                next_rand: COM_Rand(),
            }
        }
    }
}

/// A scratch gamedir plus an initialised module on both sides.
///
/// Both sides are pointed at the *same* directory: each has its own
/// `searchpath_t` and its own `com_gamedir`, so one tree on disk is read by two
/// independent filesystems, which is exactly the shape the engine build has.
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("vkq_m10f2_{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("particles")).expect("temp dir");
        let c = CString::new(dir.to_str().expect("utf-8 temp path")).expect("no NUL");

        // SAFETY: `c` outlives the calls, which copy it into each side's own
        // `com_gamedir` and `searchpath_t`.
        unsafe {
            ctest_ftepart_reset_counters();
            // `particletime` is written only by PScript_ClearParticles, from
            // whatever `cl.time` held at the time, and nothing resets it
            // between tests -- so the clock has to be set before *either* side
            // clears, not between them, or the two sides end up carrying
            // different leftovers into the next gate.
            ctest_ftepart_set_time(FIXTURE_TIME);
            for side in SIDES {
                ctest_ftepart_fs_setup(side, c.as_ptr());
                assert_eq!(ctest_ftepart_init(side), 0, "init raised on side {side}");
                // After init, not before: Cvar_RegisterVariable parses the
                // default string into `value` and would undo these. And
                // PScript_Startup sizes the pools while r_numparticles is zero
                // and never again, so they have to agree with every other test
                // in this file.
                ctest_ftepart_set_limits(side, 4096.0, 512.0);
                ctest_ftepart_set_enabled(side, 1.0, 1.0);
                // Puts both sides on the same pools, the same empty free lists
                // and the same `particletime`, whatever ran before.
                assert_eq!(
                    ctest_ftepart_clear(side, 1),
                    0,
                    "clear raised on side {side}"
                );
            }
        }
        Self { dir }
    }

    fn write_cfg(&self, name: &str, body: &str) {
        std::fs::write(self.dir.join("particles").join(format!("{name}.cfg")), body)
            .expect("write particle config");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // SAFETY: restores the globals `new` saved; no arguments.
        unsafe {
            for side in SIDES {
                ctest_ftepart_fs_teardown(side);
            }
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Reseeds the shared generator, runs `body` for one side, then snapshots.
fn run_side<F>(side: c_int, seed: u64, body: F) -> Snapshot
where
    F: FnOnce(c_int) -> (c_int, c_int),
{
    // SAFETY: the generator is a plain stub global; the caller holds the lock.
    unsafe { COM_SeedRand(seed) };
    let (raised, ret) = body(side);
    Snapshot::read(side, raised, ret)
}

/// Runs `body` against the oracle and then the port and asserts the two
/// snapshots are identical field for field, naming the first field that
/// differs.
fn compare<F>(seed: u64, mut body: F) -> Snapshot
where
    F: FnMut(c_int) -> (c_int, c_int),
{
    let c = run_side(SIDES[0], seed, &mut body);
    let rust = run_side(SIDES[1], seed, &mut body);

    assert_eq!(c.raised, rust.raised, "raise status");
    assert_eq!(c.ret, rust.ret, "return value");
    assert_eq!(c.ints, rust.ints, "integer state (C, Rust)");
    assert_eq!(c.floats, rust.floats, "particletime / p_frametime bits");
    assert_eq!(c.types, rust.types, "part_type table hash");
    assert_eq!(c.runlist, rust.runlist, "run list hash");
    assert_eq!(c.particles, rust.particles, "live particle hash");
    assert_eq!(c.queues, rust.queues, "deferred queue contents hash");
    assert_eq!(
        c.next_rand, rust.next_rand,
        "COM_Rand draw count diverged: the two sides took different branches"
    );
    assert_eq!(
        c.ints[I_CLIPDECAL], 0,
        "Mod_ClipDecal became reachable; see the stub header"
    );
    rust
}

/// Both configs set `spawnchance` far above 1 rather than leaving it at its
/// default of 1 (`r_part_fte.c:1400`). `r_part_fte.c:4577` gates every spawn
/// burst on `ptype->spawnchance < frandom ()`, and in this link `frandom ()`
/// does not stay inside `[0, 1]`: the ctest `COM_Rand` (`stubs/stubs.c:252`)
/// returns the raw 32-bit generator word where the engine's returns
/// `result & COM_RAND_MAX` (`Quake/common.c:1835`, `COM_RAND_MAX` is
/// `0xFFFFFF`), so roughly half of the draws are negative and the positive
/// ones run up to ~128. With the default the gate would close on about half
/// the bursts and the count assertions below would depend on where in the
/// stream each test happened to land. Pinning `spawnchance` open costs no
/// coverage -- the field is still parsed and still enters the type digest --
/// and it is the same value on both sides, so the differential is unaffected.
/// `diesubrand` is left out of `CFG_PLAIN` for the same reason: `p->die` is
/// scaled by `frandom ()` at `r_part_fte.c:4616`, and the expiry assertion in
/// `update_frames_differential` needs a lifetime it can predict. See the
/// report note on `stubs.c:252`.
///
/// A config that exercises most of the property parser: both ramp modes, a
/// sound, an association, an emitter, a dlight, a decal-free spawn mode and a
/// `+`-prefixed continuation effect.
///
/// The `+` continuation type carries the parser branches whose *only*
/// observable is the parsed `part_type_t` -- the two-argument `die`
/// (`r_part_fte.c:1812`, which fills `randdie` from the pair rather than from
/// the deprecated `diesubrand`), `scalefactor` (`:1763`, whose
/// `finish_particle_type` arm at `:2723-2729` rewrites `scale`, `scalerand`
/// and `invscalefactor`) and the four-argument `randomvel` (`:1835`, the arm
/// that rescales `velwrand[2]` and re-centres `velbias[2]`). Nothing spawns
/// from a continuation type in this file, so putting them there adds parser
/// coverage without perturbing any spawn-count assertion.
const CFG_RICH: &str = "\
r_part __TAG___base
{
	texture \"\"
	count 12
	scale 1.5
	scalerand 0.5
	alpha 0.75
	alphachange -0.5
	rgb 255 128 64
	rgbrand 16 32 8
	rgbrandsync 4 4 4
	die 1.5
	diesubrand 0.25
	veladd 10
	randomvel 2
	orgadd 3
	orgbias 1 2 3
	velbias 0 0 -1
	orgwrand 2 2 2
	velwrand 1 1 1
	spawnorg 8 4
	spawnvel 24 12
	spawnmode ball
	gravity 80
	friction 0.3
	flurry 2
	rotationstart 0 90
	rotationspeed 10 20
	type spark
	blend add
	stretchfactor 0.5
	spawntime 0.05
	spawnchance 1000
	emit __TAG___child
	emitinterval 0.05
	emitintervalrand 0.02
	emitstart 0.01
	lightradius 100
	lightradiusfade 50
	lightrgb 1 0.5 0.25
	lightrgbfade 0.5 0.25 0.125
	lighttime 0.4
	sound \"\" 1 1 0 100
	assoc __TAG___child
	rampmode lerp
	ramp 255 0 0 1 1
	ramp 128 64 0 0.5 2
	ramp 0 0 0 0 3
}
r_part __TAG___child
{
	texture \"\"
	count 3
	scale 1
	die 0.5
	rgb 32 32 255
	spawnmode circle
	type normal
	blend blend
	rampmode delta
	ramp 8 8 8 1 0.5
}
r_part +__TAG___base
{
	texture \"\"
	count 2
	die 0.3 0.45
	scalefactor 2
	randomvel 2 1 9
	rgb 255 255 255
	spawnmode telebox
	type normal
}
r_part __TAG___trail
{
	texture \"\"
	count 4
	die 0.8
	rgb 200 200 200
	spawnmode tracer
	type normal
	step 4
	veladd 1
}
";

/// A config with no clipping and no dlight, for the tests that run the
/// per-frame update: `cliptype` would reach `CL_TraceLine`, which needs a
/// mounted world model this link does not have.
///
/// `_b` does carry an emitter, because the per-particle emit arm
/// (`r_part_fte.c:6522-6531`) is the only caller of `ufrandom` and is only
/// reachable from an update. It emits into `_t`, which is a `step` type with
/// no `count`, and the effects it queues are never drained here (the drain is
/// `r_part_fte.c:7227-7232`, in the rendering half), so the arm adds the
/// `ufrandom` draw and a queue entry without spawning anything that would
/// move a count assertion. `_t` is declared after `_b`, so the `emit` key
/// allocates it (`:2317`) at the index the later block would have given it
/// anyway.
const CFG_PLAIN: &str = "\
r_part __TAG___a
{
	texture \"\"
	count 8
	scale 2
	alpha 1
	alphachange -1
	rgb 255 200 100
	rgbdelta -10 -20 -30
	die 2
	spawnchance 1000
	veladd 20
	randomvel 5
	spawnorg 16 8
	spawnvel 32 16
	spawnmode box
	gravity 40
	friction 0.2
	type normal
	blend blend
	rotationstart 10 20
	rotationspeed 5 5
	flurry 3
}
r_part __TAG___b
{
	texture \"\"
	count 5
	scale 1
	die 1.25
	spawnchance 1000
	rgb 64 255 64
	spawnmode lavasplash
	type normal
	blend add
	rampmode nearest
	ramp 255 0 0 1 1
	ramp 0 255 0 0.75 1.5
	ramp 0 0 255 0.5 2
	emit __TAG___t
	emitinterval 0.05
	emitintervalrand 0.02
}
r_part __TAG___t
{
	texture \"\"
	step 4
	scale 1
	die 1
	spawnchance 1000
	rgb 200 200 200
	spawnmode tracer
	type normal
	blend blend
}
r_part __TAG___x
{
	texture \"\"
	count 6
	scale 1
	die -1
	spawnchance 1000
	rgb 10 20 30
	spawnmode box
	type normal
	blend blend
}
";

fn cfg_for(tag: &str, template: &str) -> String {
    template.replace("__TAG__", tag)
}

/// Loads `tag`'s config into one side. Returns the raise status.
fn load_cfg(side: c_int, tag: &str) -> c_int {
    let name = CString::new(tag).expect("no NUL");
    // SAFETY: `name` outlives the call, which only reads it.
    unsafe { ctest_ftepart_set_desc(side, name.as_ptr()) }
}

// ---------------------------------------------------------------------------

#[test]
fn init_and_clear_differential() {
    let _guard = ctfs::lock();
    let fixture = Fixture::new("init");
    let _ = &fixture;

    // The first clear(load) is what runs PScript_Startup: it sizes the pools,
    // builds the sin/cos tables and fires the r_particledesc callback.
    let before = compare(0x1111_2222_3333_4444, |side| {
        // SAFETY: fixture is mounted for both sides; the module is initialised.
        (unsafe { ctest_ftepart_clear(side, 1) }, 0)
    });
    assert!(
        before.ints[I_FREE_PARTICLES] > 0,
        "clear(load) did not build the free list"
    );
    assert_eq!(
        before.ints[I_LIVE], 0,
        "clear(load) left live particles behind"
    );

    // A clear without load must not re-run Startup but must still rebuild the
    // free lists.
    let after = compare(0x5555_6666_7777_8888, |side| {
        // SAFETY: as above.
        (unsafe { ctest_ftepart_clear(side, 0) }, 0)
    });
    assert_eq!(
        after.ints[I_FREE_PARTICLES], before.ints[I_FREE_PARTICLES],
        "clear(0) changed the pool size"
    );
}

#[test]
fn parse_rich_config_differential() {
    let _guard = ctfs::lock();
    let fixture = Fixture::new("rich");
    let tag = "m10f2rich";
    fixture.write_cfg(tag, &cfg_for(tag, CFG_RICH));

    let base = {
        let mut ints = [0 as c_int; NUM_INTS];
        // SAFETY: `ints` is NUM_INTS long.
        unsafe { ctest_ftepart_read_ints(SIDES[0], ints.as_mut_ptr()) };
        ints[I_NUMTYPES]
    };

    let snap = compare(0x0f0f_0f0f_0f0f_0f0f, |side| (load_cfg(side, tag), 0));

    // Three types, not four: `r_part +name` re-opens the existing type rather
    // than declaring a new one (`r_part_fte.c:1514`).
    assert!(
        snap.ints[I_NUMTYPES] >= base + 3,
        "the config's effects were not parsed (types went {} -> {})",
        base,
        snap.ints[I_NUMTYPES]
    );
    assert_ne!(
        snap.types, HASH_BASIS,
        "the part_type table hash never moved off its seed"
    );

    // Every declared name must resolve, on both sides, to the same index.
    for suffix in ["_base", "_child", "_trail"] {
        let full = CString::new(format!("{tag}{suffix}")).expect("no NUL");
        let mut found = [0 as c_int; 2];
        for (i, side) in SIDES.iter().enumerate() {
            // SAFETY: `full` outlives the call.
            let raised =
                unsafe { ctest_ftepart_find_type(*side, full.as_ptr(), &mut found[i] as *mut _) };
            assert_eq!(raised, 0, "find_particle_type raised on side {side}");
        }
        assert_eq!(found[0], found[1], "index of {tag}{suffix}");
        assert!(found[0] >= 0, "{tag}{suffix} did not resolve");
    }

    // Re-running the callback wipes every effect (`R_Particles_KillAllEffects`,
    // r_part_fte.c:3595) -- including the `loadedconfigs` list, so the file is
    // genuinely re-parsed -- and then reuses the same slots by name. The type
    // count must therefore not grow, and the table must land bit-identically
    // on both sides a second time.
    let again = compare(0x0f0f_0f0f_0f0f_0f0f, |side| (load_cfg(side, tag), 0));
    assert_eq!(
        again.ints[I_NUMTYPES], snap.ints[I_NUMTYPES],
        "re-loading a config added types"
    );
}

#[test]
fn unknown_config_differential() {
    let _guard = ctfs::lock();
    let fixture = Fixture::new("unknown");
    let _ = &fixture;

    // No file on disk: both sides must take P_LoadParticleSet's not-found path,
    // leave the table alone and not raise.
    let snap = compare(0x2222_3333_4444_5555, |side| {
        (load_cfg(side, "m10f2_no_such_set"), 0)
    });
    assert_eq!(snap.raised, 0, "a missing config raised");

    // A config full of syntax the parser rejects must also be handled the same
    // way. `r_part` with no name, an unknown command and an unknown property
    // are three separate error paths.
    let tag = "m10f2junk";
    fixture.write_cfg(
        tag,
        "r_part\nnot_a_command foo\nr_part m10f2junk_x\n{\n\tnot_a_property 1\n\tcount 2\n\tdie 1\n}\n",
    );
    let snap = compare(0x3333_4444_5555_6666, |side| (load_cfg(side, tag), 0));
    assert_eq!(snap.raised, 0, "a malformed config raised");
}

#[test]
fn spawn_effects_differential() {
    let _guard = ctfs::lock();
    let fixture = Fixture::new("spawn");
    let tag = "m10f2spawn";
    fixture.write_cfg(tag, &cfg_for(tag, CFG_PLAIN));

    // SAFETY: both sides are mounted, initialised and cleared by the fixture.
    unsafe {
        ctest_ftepart_set_time(3.375_000_000_000_1);
        for side in SIDES {
            assert_eq!(ctest_ftepart_set_desc(side, c_str(tag).as_ptr()), 0);
        }
    }

    let org: [c_float; 3] = [16.5, -32.25, 64.125];
    let dir: [c_float; 3] = [0.5, -0.25, 1.0];

    // PScript_RunParticleEffectTypeString is the name-driven entry: it resolves
    // the effect by string every call, which is the path QC and the classic
    // protocol translation both take.
    let name = c_str(&format!("{tag}_a"));
    let snap = compare(0x1234_5678_9abc_def0, |side| {
        let mut out = 0;
        // SAFETY: the arrays are three floats and `name` outlives the call.
        let raised = unsafe {
            ctest_ftepart_run_effect_string(
                side,
                org.as_ptr(),
                dir.as_ptr(),
                4.0,
                name.as_ptr(),
                &mut out,
            )
        };
        (raised, out)
    });
    assert_eq!(snap.ret, 0, "the effect was not found");
    assert!(
        snap.ints[I_LIVE] > 0,
        "run_particle_effect_type_string spawned nothing"
    );
    assert_ne!(snap.particles, HASH_BASIS, "particle hash never moved");
    assert!(
        snap.ints[I_RUNLIST] > 0,
        "the type never joined the run list"
    );

    // The classic-protocol entry, which picks a type from a palette colour.
    let snap = compare(0x0bad_c0de_dead_beef, |side| {
        let mut out = 0;
        // SAFETY: as above.
        let raised =
            unsafe { ctest_ftepart_run_effect(side, org.as_ptr(), dir.as_ptr(), 73, 20, &mut out) };
        (raised, out)
    });
    assert!(snap.ints[I_LIVE] > 0, "run_particle_effect spawned nothing");

    // The index-driven entry with a trailstate, which is what an emitter uses.
    let idx = {
        let mut found = 0;
        // SAFETY: `name` outlives the call.
        unsafe { ctest_ftepart_find_type(SIDES[0], name.as_ptr(), &mut found) };
        found
    };
    assert!(idx >= 0);
    let snap = compare(0x00c0_ffee_0000_0001, |side| {
        let mut out = 0;
        // SAFETY: as above.
        let raised = unsafe {
            ctest_ftepart_run_effect_state(side, org.as_ptr(), dir.as_ptr(), 6.0, idx, &mut out)
        };
        (raised, out)
    });
    assert!(
        snap.ints[I_LIVE] > 0,
        "run_particle_effect_state spawned nothing"
    );

    // ...and releasing that trailstate must be symmetric too.
    compare(0x00c0_ffee_0000_0002, |side| {
        // SAFETY: each side owns its own trailstate slot in the fixture.
        (unsafe { ctest_ftepart_delink_trailstate(side) }, 0)
    });
}

#[test]
fn trail_differential() {
    let _guard = ctfs::lock();
    let fixture = Fixture::new("trail");
    let tag = "m10f2trail";
    fixture.write_cfg(tag, &cfg_for(tag, CFG_PLAIN));

    // SAFETY: both sides are mounted, initialised and cleared by the fixture.
    unsafe {
        ctest_ftepart_set_time(7.100_000_000_000_1);
        for side in SIDES {
            assert_eq!(ctest_ftepart_set_desc(side, c_str(tag).as_ptr()), 0);
        }
    }

    // `_t` rather than `_a`: `PScript_ParticleTrailSpawn` computes its step
    // from `countspacing` when the type has one, and otherwise from
    // `ptype->count * r_part_density.value * timeinterval`
    // (`r_part_fte.c:5112-5133`). This seam passes `timeinterval = 0` -- the
    // engine derives it from `cl.time - cl.oldtime`, which no fixture entry
    // sets -- so a `count`-driven type computes `count = 0` and returns before
    // spawning anything. `_t` sets `step 4`, which takes the `countspacing`
    // branch and is independent of the frame interval.
    let name = c_str(&format!("{tag}_t"));
    let idx = {
        let mut found = 0;
        // SAFETY: `name` outlives the call.
        unsafe { ctest_ftepart_find_type(SIDES[0], name.as_ptr(), &mut found) };
        found
    };
    assert!(idx >= 0);

    // Three consecutive segments, so the trailstate's leftover distance is
    // carried between calls: a port that reset it every call would spawn a
    // different number of particles on the second and third.
    let legs: [([c_float; 3], [c_float; 3]); 3] = [
        ([0.0, 0.0, 0.0], [64.0, 0.0, 0.0]),
        ([64.0, 0.0, 0.0], [64.0, 48.5, 0.0]),
        ([64.0, 48.5, 0.0], [10.25, -3.5, 27.75]),
    ];
    let snap = compare(0x7777_8888_9999_aaaa, |side| {
        let mut last = 0;
        for (start, end) in &legs {
            let mut out = 0;
            // SAFETY: both arrays are three floats; each side has its own
            // trailstate slot.
            let raised = unsafe {
                ctest_ftepart_trail(side, start.as_ptr(), end.as_ptr(), idx, 42, &mut out)
            };
            assert_eq!(raised, 0, "trail raised on side {side}");
            last = out;
        }
        (0, last)
    });
    assert!(snap.ints[I_LIVE] > 0, "the trail spawned nothing");
    assert_ne!(snap.particles, HASH_BASIS, "particle hash never moved");
}

#[test]
fn weather_differential() {
    let _guard = ctfs::lock();
    let fixture = Fixture::new("weather");
    // The tag carries the `te_` prefix because `PScript_RunParticleWeather`
    // resolves its effect as `te_<efname>_<colour>` and then `te_<efname>`
    // (`r_part_fte.c:4994-4998`), falling back to `pe_default` -- which is
    // only assigned by the render half's dirty-looks pass
    // (`r_part_fte.c:6626`) and so stays `P_INVALID` here. Naming the types
    // `te_m10f2weather_*` makes the second lookup hit.
    let tag = "te_m10f2weather";
    fixture.write_cfg(tag, &cfg_for(tag, CFG_PLAIN));

    // SAFETY: both sides are mounted, initialised and cleared by the fixture.
    unsafe {
        ctest_ftepart_set_time(11.062_500_000_000_1);
        for side in SIDES {
            assert_eq!(ctest_ftepart_set_desc(side, c_str(tag).as_ptr()), 0);
        }
    }

    let minb: [c_float; 3] = [-128.0, -128.0, -16.0];
    let maxb: [c_float; 3] = [128.0, 128.0, 16.0];
    let dir: [c_float; 3] = [0.0, 0.0, -1.0];
    // `te_` is what the lookup prepends, so it is stripped from the name here.
    let name = c_str(&format!("{}_b", tag.strip_prefix("te_").expect("te_ tag")));

    let snap = compare(0xaaaa_bbbb_cccc_dddd, |side| {
        // SAFETY: three three-float arrays and a NUL-terminated name, all
        // outliving the call.
        let raised = unsafe {
            ctest_ftepart_weather(
                side,
                minb.as_ptr(),
                maxb.as_ptr(),
                dir.as_ptr(),
                24.0,
                12,
                name.as_ptr(),
            )
        };
        (raised, 0)
    });
    assert_eq!(snap.raised, 0);
    assert!(snap.ints[I_LIVE] > 0, "the weather burst spawned nothing");
}

#[test]
fn update_frames_differential() {
    let _guard = ctfs::lock();
    let fixture = Fixture::new("update");
    let tag = "m10f2update";
    fixture.write_cfg(tag, &cfg_for(tag, CFG_PLAIN));

    // SAFETY: both sides are mounted, initialised and cleared by the fixture.
    unsafe {
        for side in SIDES {
            assert_eq!(ctest_ftepart_set_desc(side, c_str(tag).as_ptr()), 0);
        }
    }

    let org: [c_float; 3] = [1.5, -2.25, 3.125];
    let dir: [c_float; 3] = [0.25, 0.5, 0.75];
    let name_a = c_str(&format!("{tag}_a"));
    let name_b = c_str(&format!("{tag}_b"));

    // Populate, then run several frames at times whose narrowing into the
    // single-precision p_frametime is not exact (ADR-010).
    let times: [c_double; 5] = [
        1.000_000_000_000_1,
        1.033_333_333_333_3,
        1.066_666_666_666_7,
        1.400_000_000_000_1,
        2.900_000_000_000_1,
    ];

    let snap = compare(0xdead_0000_beef_0001, |side| {
        // SAFETY: the arrays are three floats; the names outlive the call.
        unsafe {
            ctest_ftepart_set_time(0.5);
            let mut out = 0;
            assert_eq!(
                ctest_ftepart_run_effect_string(
                    side,
                    org.as_ptr(),
                    dir.as_ptr(),
                    16.0,
                    name_a.as_ptr(),
                    &mut out,
                ),
                0
            );
            assert_eq!(
                ctest_ftepart_run_effect_string(
                    side,
                    org.as_ptr(),
                    dir.as_ptr(),
                    9.0,
                    name_b.as_ptr(),
                    &mut out,
                ),
                0
            );
            for t in times {
                ctest_ftepart_set_time(t);
                let raised = ctest_ftepart_update(side);
                if raised != 0 {
                    return (raised, 0);
                }
            }
            (0, 0)
        }
    });

    assert_eq!(snap.raised, 0);
    assert!(
        snap.ints[I_UPDATES] > 0,
        "the setup task flattened no particles"
    );
    assert!(
        snap.ints[I_LIVE] > 0,
        "every particle expired before the last frame; the update was not exercised"
    );
    assert_ne!(
        snap.floats[1], 0,
        "p_frametime stayed zero, so the physics never ran"
    );
    assert_ne!(snap.particles, HASH_BASIS, "particle hash never moved");

    // One more frame at a `cl.time` far beyond the last, to compare the
    // `p_frametime` clamp: `r_part_fte.c:6610-6613` pins the interval to at
    // most one second however far the clock jumped.
    let snap = compare(0xdead_0000_beef_0002, |side| {
        // SAFETY: as above.
        unsafe {
            ctest_ftepart_set_time(60.0);
            (ctest_ftepart_update(side), 0)
        }
    });
    assert_eq!(
        snap.floats[1],
        1.0f32.to_bits(),
        "p_frametime was not clamped to one second"
    );
    // Nothing expires here, and that is not a bug in the port: a particle is
    // unlinked when `p->die < particletime` (`r_part_fte.c:6721`), and
    // `particletime` only ever advances at `r_part_fte.c:7292`, inside
    // `PScript_UpdateParticleTypes`. That function is the render half's
    // per-type walk -- it builds scenetris and is not one of the entry points
    // this seam exposes -- so across this whole test `particletime` stays at
    // the value `PScript_ClearParticles` gave it. `expiry_differential`
    // covers the unlink path instead, with a type whose lifetime is already
    // spent at spawn time.
    assert!(
        snap.ints[I_LIVE] > 0,
        "particles vanished without particletime advancing"
    );
}

/// The expiry and trailstate-delink half of the setup task
/// (`r_part_fte.c:6717-6748`), reached with a type whose `die` is negative so
/// that `p->die = particletime + ptype->die` (`r_part_fte.c:4838`) is already
/// behind `particletime` when the particle is linked. See the note in
/// `update_frames_differential` for why an ordinary lifetime cannot reach this
/// path through the simulation seam.
#[test]
fn expiry_differential() {
    let _guard = ctfs::lock();
    let fixture = Fixture::new("expiry");
    let tag = "m10f2expire";
    fixture.write_cfg(tag, &cfg_for(tag, CFG_PLAIN));

    // SAFETY: both sides are mounted, initialised and cleared by the fixture.
    unsafe {
        for side in SIDES {
            assert_eq!(ctest_ftepart_set_desc(side, c_str(tag).as_ptr()), 0);
        }
    }

    let org: [c_float; 3] = [4.0, 8.0, 12.0];
    let dir: [c_float; 3] = [0.0, 0.0, 1.0];
    let name = c_str(&format!("{tag}_x"));

    let snap = compare(0x0e0e_1111_2222_3333, |side| {
        let mut out = 0;
        // SAFETY: the arrays are three floats and `name` outlives the call.
        let raised = unsafe {
            ctest_ftepart_run_effect_string(
                side,
                org.as_ptr(),
                dir.as_ptr(),
                8.0,
                name.as_ptr(),
                &mut out,
            )
        };
        (raised, out)
    });
    assert!(
        snap.ints[I_LIVE] > 0,
        "the dead-on-arrival type spawned none"
    );

    let snap = compare(0x0e0e_1111_2222_3334, |side| {
        // SAFETY: no arguments beyond the side.
        unsafe {
            ctest_ftepart_set_time(0.3);
            (ctest_ftepart_update(side), 0)
        }
    });
    assert_eq!(snap.raised, 0);
    assert_eq!(
        snap.ints[I_LIVE], 0,
        "the setup task did not unlink the expired particles"
    );
    assert_eq!(
        snap.ints[I_UPDATES], 0,
        "an expired particle reached the parallel update"
    );
}

#[test]
fn deferred_queue_differential() {
    let _guard = ctfs::lock();
    let fixture = Fixture::new("queue");
    let tag = "m10f2queue";
    fixture.write_cfg(tag, &cfg_for(tag, CFG_PLAIN));

    // SAFETY: both sides are mounted, initialised and cleared by the fixture.
    unsafe {
        ctest_ftepart_set_time(5.0);
        for side in SIDES {
            assert_eq!(ctest_ftepart_set_desc(side, c_str(tag).as_ptr()), 0);
        }
    }

    let org: [c_float; 3] = [8.0, 16.0, 24.0];
    let dir: [c_float; 3] = [1.0, 0.0, 0.0];
    let name = c_str(&format!("{tag}_a"));
    let idx = {
        let mut found = 0;
        // SAFETY: `name` outlives the call.
        unsafe { ctest_ftepart_find_type(SIDES[0], name.as_ptr(), &mut found) };
        found
    };
    assert!(idx >= 0);

    // PScript_QueueEffect is what the parallel update calls instead of spawning
    // directly. Queueing without draining must leave both sides' worker-0 queue
    // in the same state.
    let snap = compare(0x1010_2020_3030_4040, |side| {
        // SAFETY: the arrays are three floats.
        unsafe {
            for i in 0..3 {
                ctest_ftepart_queue_effect(
                    side,
                    org.as_ptr(),
                    dir.as_ptr(),
                    1.0 + i as c_float,
                    idx,
                );
            }
        }
        (0, 0)
    });
    assert!(
        snap.ints[I_Q_EFFECTS] >= 3,
        "the deferred effect queue did not grow"
    );
    assert_eq!(
        snap.ints[I_Q_TRAILS], 0,
        "nothing should have queued a trail"
    );

    // A full update must leave the queue exactly where it is. The effect and
    // trail queues are drained at `r_part_fte.c:7227-7232`, inside
    // `PScript_UpdateParticleTypes`, which belongs to the render half and is
    // not one of this seam's entry points; the setup and parallel tasks that
    // are driven here must not touch them.
    let snap = compare(0x1010_2020_3030_4041, |side| {
        // SAFETY: as above.
        unsafe {
            ctest_ftepart_set_time(5.05);
            (ctest_ftepart_update(side), 0)
        }
    });
    assert_eq!(snap.raised, 0);
    assert!(
        snap.ints[I_Q_EFFECTS] >= 3,
        "the update consumed queue entries it does not own"
    );

    // `ctest_ftepart_update` also runs `PScript_FlushDlightsTask`
    // (`r_part_fte.c:6382`), the one drain this seam does reach, so the dlight
    // queue must come back empty from the same call that left the effect
    // queue alone.
    assert_eq!(
        snap.ints[I_Q_DLIGHTS], 0,
        "PScript_FlushDlightsTask left dlights queued"
    );
}

#[test]
fn console_commands_differential() {
    let _guard = ctfs::lock();
    let fixture = Fixture::new("cmds");
    let tag = "m10f2cmds";
    fixture.write_cfg(tag, &cfg_for(tag, CFG_PLAIN));

    // SAFETY: both sides are mounted, initialised and cleared by the fixture.
    unsafe {
        for side in SIDES {
            assert_eq!(ctest_ftepart_set_desc(side, c_str(tag).as_ptr()), 0);
        }
    }

    // r_partinfo, r_beaminfo and r_partredirect with no arguments: each has an
    // argument-count guard and a table walk. The console text is not compared
    // (see the stub header on the shared Cmd_AddCommand registry), but the
    // raise status and the state they leave behind are.
    for which in 0..3 {
        let snap = compare(0x5a5a_0000_0000_0000 + which as u64, |side| {
            // SAFETY: no arguments; the fixture dispatches on side.
            (unsafe { ctest_ftepart_run_cmd(side, which) }, 0)
        });
        assert_eq!(snap.raised, 0, "command {which} raised");
    }
}

#[test]
fn vector_normalize2_differential() {
    let _guard = ctfs::lock();

    // VectorNormalize2 is `static` in r_part_fte.c and the port demoted it the
    // same way, so it is only reachable through the export. It is pure, so it
    // needs no fixture -- and it is the one place in the module where a
    // zero-length input divides by zero on both sides.
    let cases: [[c_float; 3]; 7] = [
        [3.0, 4.0, 12.0],
        [-1.5, 0.0, 0.0],
        [0.0, 0.0, 0.0],
        [1e-20, 1e-20, 1e-20],
        [1e20, -1e20, 1e20],
        [0.1, 0.2, 0.3],
        [-0.0, 0.0, -0.0],
    ];

    for v in &cases {
        let mut out = [[0.0 as c_float; 3]; 2];
        let mut len = [0.0 as c_float; 2];
        for (i, side) in SIDES.iter().enumerate() {
            // SAFETY: both arrays are three floats.
            len[i] = unsafe { ctest_ftepart_normalize2(*side, v.as_ptr(), out[i].as_mut_ptr()) };
        }
        // COMPAT: ADR-010 -- compared as raw bits, so -0.0 and NaN payloads
        // count as differences.
        assert_eq!(
            len[0].to_bits(),
            len[1].to_bits(),
            "VectorNormalize2 length for {v:?}"
        );
        assert_eq!(
            out[0].map(f32::to_bits),
            out[1].map(f32::to_bits),
            "VectorNormalize2 result for {v:?}"
        );
    }
}

#[test]
fn shutdown_differential() {
    let _guard = ctfs::lock();
    let fixture = Fixture::new("shutdown");
    let tag = "m10f2shut";
    fixture.write_cfg(tag, &cfg_for(tag, CFG_RICH));

    // SAFETY: both sides are mounted, initialised and cleared by the fixture.
    unsafe {
        ctest_ftepart_set_time(2.0);
        for side in SIDES {
            assert_eq!(ctest_ftepart_set_desc(side, c_str(tag).as_ptr()), 0);
        }
    }

    let org: [c_float; 3] = [0.0, 0.0, 0.0];
    let dir: [c_float; 3] = [0.0, 0.0, 1.0];
    let name = c_str(&format!("{tag}_base"));
    let populated = compare(0xfeed_face_0000_0001, |side| {
        let mut out = 0;
        // SAFETY: the arrays are three floats; `name` outlives the call.
        let raised = unsafe {
            ctest_ftepart_run_effect_string(
                side,
                org.as_ptr(),
                dir.as_ptr(),
                8.0,
                name.as_ptr(),
                &mut out,
            )
        };
        (raised, out)
    });
    assert!(populated.ints[I_LIVE] > 0, "nothing to tear down");
    assert!(populated.ints[I_NUMTYPES] > 0);

    // PScript_Shutdown frees the type table, the pools and the loaded-config
    // list. Both sides must end up with the same empty state, and neither may
    // raise.
    let snap = compare(0xfeed_face_0000_0002, |side| {
        // SAFETY: no arguments.
        (unsafe { ctest_ftepart_shutdown(side) }, 0)
    });
    assert_eq!(snap.raised, 0, "shutdown raised");
    assert_eq!(snap.ints[I_NUMTYPES], 0, "shutdown left types behind");
    assert_eq!(snap.ints[I_RUNLIST], 0, "shutdown left a run list behind");
    assert_eq!(snap.ints[I_LIVE], 0, "shutdown left particles behind");

    // Re-initialising after a shutdown has to work: the roadmap's exit criteria
    // include a map change, which is exactly this sequence.
    let snap = compare(0xfeed_face_0000_0003, |side| {
        // SAFETY: as above.
        unsafe {
            let raised = ctest_ftepart_init(side);
            if raised != 0 {
                return (raised, 0);
            }
            (ctest_ftepart_clear(side, 1), 0)
        }
    });
    assert_eq!(snap.raised, 0, "re-init after shutdown raised");
    assert!(
        snap.ints[I_FREE_PARTICLES] > 0,
        "re-init did not rebuild the free list"
    );
}

/// Re-parsing a config that is already loaded, with particles still linked to
/// its types.
///
/// `R_ParticleDesc_Callback` (`r_part_fte.c:3622`) does not free the type
/// table -- it blanks `texname`/`scale`/`loaded` and re-reads the same files,
/// so `P_GetParticleType` finds every type by name and hands it to
/// `P_ResetToDefaults` (`r_part_fte.c:1328`). That is the only caller that
/// reaches the reset with `ptype->particles` non-empty, and it is the one
/// place the whole live list is spliced onto the head of `free_particles`
/// (`r_part_fte.c:1345-1351`) rather than being unlinked one at a time by the
/// update. Loading once and spawning is not enough to cover it: on a first
/// load the types are freshly `memset` and the splice loop never runs.
#[test]
fn reload_config_differential() {
    let _guard = ctfs::lock();
    let fixture = Fixture::new("reload");
    let tag = "m10f2reload";
    fixture.write_cfg(tag, &cfg_for(tag, CFG_PLAIN));

    let org: [c_float; 3] = [-7.0, 3.5, 0.25];
    let dir: [c_float; 3] = [0.0, 1.0, 0.0];
    let name = c_str(&format!("{tag}_a"));

    let snap = compare(0x5eed_0000_0000_0011, |side| {
        // SAFETY: the arrays are three floats; `tag` and `name` outlive the
        // calls, which only read them.
        unsafe {
            let raised = ctest_ftepart_set_desc(side, c_str(tag).as_ptr());
            if raised != 0 {
                return (raised, 0);
            }
            let mut out = 0;
            let raised = ctest_ftepart_run_effect_string(
                side,
                org.as_ptr(),
                dir.as_ptr(),
                12.0,
                name.as_ptr(),
                &mut out,
            );
            (raised, out)
        }
    });
    assert_eq!(snap.raised, 0);
    assert!(snap.ints[I_LIVE] > 0, "nothing was spawned to reset over");
    let live_before = snap.ints[I_LIVE];
    let free_before = snap.ints[I_FREE_PARTICLES];

    let snap = compare(0x5eed_0000_0000_0012, |side| {
        // SAFETY: as above.
        unsafe { (ctest_ftepart_set_desc(side, c_str(tag).as_ptr()), 0) }
    });
    assert_eq!(snap.raised, 0, "the reload raised");
    assert_eq!(
        snap.ints[I_LIVE], 0,
        "P_ResetToDefaults left particles linked to a reset type"
    );
    assert_eq!(
        snap.ints[I_FREE_PARTICLES],
        free_before + live_before,
        "the reset did not splice every live particle back onto the free list"
    );
}

/// A frame whose `cl.time` is *behind* the previous one.
///
/// `r_part_fte.c:6609-6610` clamps `p_frametime` to zero when the clock goes
/// backwards, which happens in the engine on a demo rewind and on the first
/// frame after a level change. Without a frame that actually goes backwards
/// the lower clamp is dead code in every other test here, and a port that
/// dropped it or moved the comparison would still pass them.
#[test]
fn time_rewind_differential() {
    let _guard = ctfs::lock();
    let fixture = Fixture::new("rewind");
    let tag = "m10f2rewind";
    fixture.write_cfg(tag, &cfg_for(tag, CFG_PLAIN));

    // SAFETY: both sides are mounted, initialised and cleared by the fixture.
    unsafe {
        for side in SIDES {
            assert_eq!(ctest_ftepart_set_desc(side, c_str(tag).as_ptr()), 0);
        }
    }

    let org: [c_float; 3] = [0.5, 0.25, -1.0];
    let dir: [c_float; 3] = [0.0, 0.0, 1.0];
    let name = c_str(&format!("{tag}_a"));

    let snap = compare(0x71de_0000_0000_0001, |side| {
        // SAFETY: the arrays are three floats; `name` outlives the call.
        unsafe {
            ctest_ftepart_set_time(3.0);
            let mut out = 0;
            let raised = ctest_ftepart_run_effect_string(
                side,
                org.as_ptr(),
                dir.as_ptr(),
                10.0,
                name.as_ptr(),
                &mut out,
            );
            if raised != 0 {
                return (raised, out);
            }
            // Two frames, not one: `oldtime` (`r_part_fte.c:6598`) is a
            // function-static that no fixture resets, so the interval the
            // first update sees depends on whichever gate in this file ran
            // last. The first update pins it at 3.0; the second then has an
            // interval this test owns.
            let raised = ctest_ftepart_update(side);
            if raised != 0 {
                return (raised, out);
            }
            ctest_ftepart_set_time(4.0);
            (ctest_ftepart_update(side), out)
        }
    });
    assert_eq!(snap.raised, 0);
    assert_eq!(
        snap.floats[1],
        1.0f32.to_bits(),
        "the forward frame did not produce the one-second interval"
    );

    // A quarter of a second back, not a whole one: the clamp under test is
    // `p_frametime < 0` (`r_part_fte.c:6609-6610`), so the rewind has to be
    // small enough that only a zero threshold explains the zero. A large
    // rewind would also read as zero under a threshold of -1.
    let snap = compare(0x71de_0000_0000_0002, |side| {
        // SAFETY: no arguments beyond the side.
        unsafe {
            ctest_ftepart_set_time(3.75);
            (ctest_ftepart_update(side), 0)
        }
    });
    assert_eq!(snap.raised, 0);
    assert_eq!(
        snap.floats[1],
        0.0f32.to_bits(),
        "p_frametime was not clamped to zero when the clock went backwards"
    );
    assert!(
        snap.ints[I_LIVE] > 0,
        "the rewound frame destroyed the pool"
    );
}

/// A whole particle lifetime: spawn, age through the ramp, expire, and come
/// back to the free list.
///
/// This is the only test that closes the frame with
/// `ctest_ftepart_finish_frame`, the fixture's stand-in for the two
/// statements in `PScript_UpdateParticleTypes` the simulation half depends on
/// (`r_part_fte.c:7286-7292`; see the comment on the fixture). Without it
/// `particletime` never moves, so `update_frames_differential` can only ever
/// exercise the physics integrator -- the ramp walks at `r_part_fte.c:6478`,
/// `:6488` and `:6503`, the `rgbchangetime` gate at `:6512` and the unlink at
/// `:6721` all key off `p->die - particletime` and stay pinned at their
/// spawn-time values.
///
/// It also compares `slooks` as an index rather than as the set/unset flag
/// the type digest reduces it to, which is sound only here: the dedup pass at
/// `r_part_fte.c:6622-6636` has just rebuilt every entry and nothing has
/// reallocated the table since. See `ctest_ftepart_hash_slooks`.
#[test]
fn frame_lifecycle_differential() {
    let _guard = ctfs::lock();
    let fixture = Fixture::new("lifecycle");
    let tag = "m10f2life";
    fixture.write_cfg(tag, &cfg_for(tag, CFG_PLAIN));

    // SAFETY: both sides are mounted, initialised and cleared by the fixture.
    unsafe {
        for side in SIDES {
            assert_eq!(ctest_ftepart_set_desc(side, c_str(tag).as_ptr()), 0);
        }
    }

    let org: [c_float; 3] = [11.0, -6.5, 2.0];
    let dir: [c_float; 3] = [0.0, 0.0, 1.0];
    let name_a = c_str(&format!("{tag}_a"));
    let name_b = c_str(&format!("{tag}_b"));

    let idle = compare(0x11fe_0000_0000_0001, |_side| (0, 0));
    let free_idle = idle.ints[I_FREE_PARTICLES];

    // Spawn, run two frames, and read the dedup result while it is fresh.
    let slooks = std::cell::Cell::new([0u64; 2]);
    let snap = compare(0x11fe_0000_0000_0002, |side| {
        // SAFETY: the arrays are three floats; the names outlive the calls.
        unsafe {
            // Pin `oldtime` (`r_part_fte.c:6598`) before anything else. It is
            // a function-static no fixture resets, so without this the first
            // interval below would depend on whichever gate ran last, and the
            // interval is what drives `particletime` here. Zero is safe: every
            // clock this file sets is non-negative, so `cl.time - oldtime` is
            // at most zero and the `:6609` clamp pins it there.
            ctest_ftepart_set_time(0.0);
            let raised = ctest_ftepart_update(side);
            if raised != 0 {
                return (raised, 0);
            }
            ctest_ftepart_finish_frame(side);

            let mut out = 0;
            for name in [&name_a, &name_b] {
                let raised = ctest_ftepart_run_effect_string(
                    side,
                    org.as_ptr(),
                    dir.as_ptr(),
                    10.0,
                    name.as_ptr(),
                    &mut out,
                );
                if raised != 0 {
                    return (raised, out);
                }
            }
            // Two frames, and the second is the interesting one. A particle is
            // born with `nextemit == particletime` (`r_part_fte.c:4627`, where
            // `p->die` still holds the *random* part of the lifetime and is
            // zero for a one-argument `die`), and the emit arm at `:6526`
            // needs `nextemit < particletime` strictly. `particletime` only
            // moves in `ctest_ftepart_finish_frame`, after the update, so the
            // arm cannot fire on the first frame after a spawn no matter what
            // the clock does.
            let mut raised = 0;
            for t in [0.25, 0.5] {
                ctest_ftepart_set_time(t);
                raised = ctest_ftepart_update(side);
                if raised != 0 {
                    return (raised, 0);
                }
                let mut sl = slooks.get();
                sl[usize::from(side == 0)] = ctest_ftepart_hash_slooks(side, HASH_BASIS);
                slooks.set(sl);
                ctest_ftepart_finish_frame(side);
            }
            (raised, 0)
        }
    });
    assert_eq!(snap.raised, 0);
    assert!(snap.ints[I_LIVE] > 0, "the lifecycle spawned nothing");
    assert!(
        snap.ints[I_UPDATES] > 0,
        "the setup task flattened no particles"
    );
    let sl = slooks.get();
    assert_eq!(
        sl[0], sl[1],
        "the shared-looks dedup picked different types (C, Rust)"
    );

    // Age them past the longest `die` in CFG_PLAIN. `p->die` is
    // `particletime + type->die` at spawn (`r_part_fte.c:4838`), and
    // `particletime` now advances by the clamped `p_frametime` each frame,
    // so this walks the ramps to their far end and then over the unlink.
    let mut times = Vec::new();
    let mut t = 1.75;
    while t < 5.5 {
        times.push(t as c_double);
        t += 0.5;
    }
    let snap = compare(0x11fe_0000_0000_0003, |side| {
        // SAFETY: no arguments beyond the side.
        unsafe {
            for t in &times {
                ctest_ftepart_set_time(*t);
                let raised = ctest_ftepart_update(side);
                if raised != 0 {
                    return (raised, 0);
                }
                ctest_ftepart_finish_frame(side);
            }
            (0, 0)
        }
    });
    assert_eq!(snap.raised, 0);
    assert_eq!(
        snap.ints[I_LIVE], 0,
        "particles outlived `die` even though particletime advanced"
    );
    assert_eq!(
        snap.ints[I_UPDATES], 0,
        "an expired particle was still flattened into the update array"
    );
    assert_eq!(
        snap.ints[I_FREE_PARTICLES], free_idle,
        "the kill list did not return every particle to the pool"
    );
    // The types stay on the run list, and that is not a leak in the port: the
    // "delete from run list if necessary" arm is `r_part_fte.c:7205-7216`,
    // inside `PScript_UpdateParticleTypes`, and needs the `lastvalidtype`
    // cursor that walk carries. `ctest_ftepart_finish_frame` deliberately
    // emulates only the two statements the simulation half reads back, so the
    // run list is still populated here.
    assert!(
        snap.ints[I_RUNLIST] > 0,
        "the run list emptied without the render half's walk"
    );
}

fn c_str(s: &str) -> CString {
    CString::new(s).expect("no NUL")
}
