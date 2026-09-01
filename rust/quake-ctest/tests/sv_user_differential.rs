//! Differential/characterization gate for `Quake/sv_user.c` -- server-side
//! player movement/input. Rust migration Phase 7, M6, task T6.4.
//!
//! The oracle side lives in `stubs/sv_user_ref.c`: a self-contained server
//! fixture built on the shared M3 "synthetic room" (`ctest_phys_reset`,
//! reused wholesale from the M4/M5 waves) plus a private `client_t` array
//! (`ctest_svuser_clients`) that BOTH sides read/written through, reset fresh
//! before each side's run (see that file's own module doc for why one shared
//! array is correct here, unlike `sv_phys_differential.rs`'s per-side
//! duplication).
//!
//! Only three of `sv_user.c`'s thirteen functions are reachable from outside
//! the file (`SV_SetIdealPitch`, `SV_ClientThink`, `SV_RunClients` --
//! confirmed by grepping `Quake/*.c` for the other ten names), and only
//! those three are `quake_rs_*` exports on the Rust side. Every test below
//! drives one of these three plain-named entry points; the other ten
//! functions are reached indirectly by shaping fixture state
//! (`SV_UserFriction`/`SV_Accelerate`/`SV_AirAccelerate` through
//! `SV_ClientThink`'s on-ground/off-ground branches, `SV_WaterMove` through
//! the waterlevel>=2 branch, `SV_WaterJump` through the `FL_WATERJUMP` flag,
//! `SV_NoclipMove` through the altnoclip branch, `DropPunchAngle`
//! unconditionally). `SV_ReadClientMove`/`SV_ReadClientMessage` are NOT
//! covered here: `stubs.c`'s `NET_GetServerMessage` always returns NULL
//! (confirmed by reading its body), so `SV_RunClients`' receive loop always
//! breaks on the first iteration in this harness -- those two functions
//! (and `SVFTE_Ack`) are structurally unreachable through any of the three
//! entry points this suite drives. This is a documented, accepted coverage
//! limit, not an oversight: `ctest_svuser_load_message` exists in the oracle
//! file for a future test that could drive `SV_ReadClientMessage` directly,
//! but no such direct entry point exists on the Rust side to pair it with.
//!
//! ADR-010: every float/double promotion boundary `sv_user.c` crosses (the
//! `sin`/`cos` pair in `SV_SetIdealPitch`, `SV_UserFriction`'s `sqrt` and the
//! two `pow` calls in the analytic-friction branches) is exercised at a
//! value where the double-precision intermediate would round differently
//! than a naive `f32`-only computation, so a regression that widens or
//! narrows a computation at the wrong point produces an observable
//! mismatch, not a silently-passing test.

use core::ffi::{c_double, c_float, c_int};
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

// ---------------------------------------------------------------------------
// sv_user.c constants, transcribed.

const MOVETYPE_NONE: f32 = 0.0;
const MOVETYPE_WALK: f32 = 3.0;
const MOVETYPE_NOCLIP: f32 = 8.0;
const FL_ONGROUND: i32 = 512;
const FL_WATERJUMP: i32 = 2048;

const PITCH: usize = 0;
const YAW: usize = 1;
const ROLL: usize = 2;

extern "C" {
    fn ctest_svuser_reset(
        num_edicts: c_int,
        maxclients: c_int,
        frametime: c_double,
        vmtime: c_double,
    );
    fn ctest_svuser_set_client(
        slot: c_int,
        edict_num: c_int,
        active: c_int,
        spawned: c_int,
        has_netconnection: c_int,
    );
    fn ctest_svuser_set_player(num: c_int, scalars: *const c_float, vectors: *const c_float);
    fn ctest_svuser_set_cmd(slot: c_int, forwardmove: c_float, sidemove: c_float, upmove: c_float);
    fn ctest_svuser_set_cvars(
        altnoclip: c_float,
        maxspeed: c_float,
        accelerate: c_float,
        idealpitchscale: c_float,
        edgefriction: c_float,
    );
    fn ctest_svuser_set_sv_paused(paused: c_int);
    fn ctest_svuser_set_key_dest(kd: c_int);
    fn ctest_svuser_get_player(num: c_int, scalars: *mut c_float, vectors: *mut c_float);
    fn ctest_svuser_get_player_buttons(
        num: c_int,
        button0: *mut c_float,
        button2: *mut c_float,
        impulse: *mut c_float,
    );
    fn ctest_svuser_get_cmd(
        slot: c_int,
        forwardmove: *mut c_float,
        sidemove: *mut c_float,
        upmove: *mut c_float,
        viewangles: *mut c_float,
    );
    fn ctest_svuser_host_client_slot() -> c_int;

    fn c_ref_SV_SetIdealPitch();
    fn c_ref_SV_ClientThink();
    fn c_ref_SV_RunClients();
    fn SV_SetIdealPitch();
    fn SV_ClientThink();
    fn SV_RunClients();

    // world.c is a real oracle source since M3 (both sides already ported).
    // ctest_svuser_reset only rebuilds the arena/qcvm (via ctest_phys_reset ->
    // ctest_world_reset in stubs.c), which memsets qcvm's areanode array to
    // all-zero rather than a valid self-referencing sentinel list; walking
    // that list is exactly what SV_Move does for every trace SV_UserFriction
    // issues, so every test must rebuild it before running, mirroring
    // world_differential.rs/sv_move_differential.rs/sv_phys_differential.rs's
    // own per-side reset() call.
    fn c_ref_SV_ClearWorld();
    fn SV_ClearWorld();
}

// ---------------------------------------------------------------------------
// Side dispatch

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    C,
    Rust,
}

const SIDES: [Side; 2] = [Side::C, Side::Rust];

impl Side {
    fn set_ideal_pitch(self) {
        // SAFETY: no arguments; the fixture published sv_player/the M3 room
        // before this. Neither side longjmps out of this frame (ADR-009 --
        // both are plain re-raising wrappers over a status core).
        unsafe {
            match self {
                Side::C => c_ref_SV_SetIdealPitch(),
                Side::Rust => SV_SetIdealPitch(),
            }
        }
    }

    fn client_think(self) {
        // SAFETY: see set_ideal_pitch.
        unsafe {
            match self {
                Side::C => c_ref_SV_ClientThink(),
                Side::Rust => SV_ClientThink(),
            }
        }
    }

    fn run_clients(self) {
        // SAFETY: see set_ideal_pitch.
        unsafe {
            match self {
                Side::C => c_ref_SV_RunClients(),
                Side::Rust => SV_RunClients(),
            }
        }
    }

    /// Rebuilds `qcvm`'s areanode tree over the fixture's already-published
    /// worldmodel (`ctest_svuser_reset` -> `ctest_phys_reset` sets
    /// `qcvm->worldmodel` but never calls `SV_ClearWorld`, so the areanode
    /// array is left all-zero -- a dangling, non-self-referencing sentinel
    /// list that crashes the moment `SV_Move` walks it). Must run after every
    /// `ctest_svuser_reset` and before that side's entry point.
    fn clear_world(self) {
        // SAFETY: qcvm/the worldmodel were just published by
        // ctest_svuser_reset; ClearWorld only reads qcvm->worldmodel's
        // mins/maxs and (re)writes qcvm->areanodes, both already valid.
        unsafe {
            match self {
                Side::C => c_ref_SV_ClearWorld(),
                Side::Rust => SV_ClearWorld(),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Player fixture: the 8 scalars + 7 vec3s ctest_svuser_{set,get}_player carry.

#[derive(Clone, Copy)]
struct Player {
    movetype: f32,
    flags: f32,
    waterlevel: f32,
    watertype: f32,
    health: f32,
    teleport_time: f32,
    idealpitch: f32,
    fixangle: f32,
    origin: [f32; 3],
    velocity: [f32; 3],
    angles: [f32; 3],
    v_angle: [f32; 3],
    punchangle: [f32; 3],
    view_ofs: [f32; 3],
    movedir: [f32; 3],
}

impl Player {
    /// A grounded walking player, standing well inside the M3 room's open
    /// floor area, with a default view_ofs so SV_SetIdealPitch's forward
    /// scan and SV_Move's traces land on real geometry rather than the
    /// world boundary.
    fn blank() -> Self {
        Player {
            movetype: MOVETYPE_WALK,
            flags: FL_ONGROUND as f32,
            waterlevel: 0.0,
            watertype: 0.0,
            health: 100.0,
            teleport_time: 0.0,
            idealpitch: 0.0,
            fixangle: 0.0,
            origin: [0.0, 0.0, -80.0],
            velocity: [0.0, 0.0, 0.0],
            angles: [0.0, 0.0, 0.0],
            v_angle: [0.0, 0.0, 0.0],
            punchangle: [0.0, 0.0, 0.0],
            view_ofs: [0.0, 0.0, 22.0],
            movedir: [0.0, 0.0, 0.0],
        }
    }

    fn scalars(&self) -> [f32; 8] {
        [
            self.movetype,
            self.flags,
            self.waterlevel,
            self.watertype,
            self.health,
            self.teleport_time,
            self.idealpitch,
            self.fixangle,
        ]
    }

    fn vectors(&self) -> [f32; 21] {
        let mut v = [0.0f32; 21];
        v[0..3].copy_from_slice(&self.origin);
        v[3..6].copy_from_slice(&self.velocity);
        v[6..9].copy_from_slice(&self.angles);
        v[9..12].copy_from_slice(&self.v_angle);
        v[12..15].copy_from_slice(&self.punchangle);
        v[15..18].copy_from_slice(&self.view_ofs);
        v[18..21].copy_from_slice(&self.movedir);
        v
    }

    fn apply(&self, num: c_int) {
        // SAFETY: num indexes the fixture arena (num_edicts >= 2 always).
        unsafe { ctest_svuser_set_player(num, self.scalars().as_ptr(), self.vectors().as_ptr()) };
    }
}

/// Snapshot of everything the differential comparison cares about for player
/// edict `num`: the 8 scalars + 7 vectors, plus the three button/impulse
/// fields SV_ReadClientMove would write (never touched by our 3 entry
/// points, but captured anyway so a future SV_ReadClientMove regression
/// through some other path would still show up).
#[derive(Debug, PartialEq)]
struct PlayerSnap {
    scalars: [u32; 8],
    vectors: [u32; 21],
    button0: u32,
    button2: u32,
    impulse: u32,
}

fn get_player(num: c_int) -> PlayerSnap {
    let mut scalars = [0.0f32; 8];
    let mut vectors = [0.0f32; 21];
    let mut button0 = 0.0f32;
    let mut button2 = 0.0f32;
    let mut impulse = 0.0f32;
    // SAFETY: fixed-size out-params matching the C signatures exactly.
    unsafe {
        ctest_svuser_get_player(num, scalars.as_mut_ptr(), vectors.as_mut_ptr());
        ctest_svuser_get_player_buttons(num, &mut button0, &mut button2, &mut impulse);
    }
    PlayerSnap {
        scalars: scalars.map(f32::to_bits),
        vectors: vectors.map(f32::to_bits),
        button0: button0.to_bits(),
        button2: button2.to_bits(),
        impulse: impulse.to_bits(),
    }
}

#[derive(Debug, PartialEq)]
struct CmdSnap {
    forwardmove: u32,
    sidemove: u32,
    upmove: u32,
    viewangles: [u32; 3],
}

fn get_cmd(slot: c_int) -> CmdSnap {
    let (mut f, mut s, mut u) = (0.0f32, 0.0f32, 0.0f32);
    let mut va = [0.0f32; 3];
    // SAFETY: slot is always in [0, MAX_CLIENTS).
    unsafe { ctest_svuser_get_cmd(slot, &mut f, &mut s, &mut u, va.as_mut_ptr()) };
    CmdSnap {
        forwardmove: f.to_bits(),
        sidemove: s.to_bits(),
        upmove: u.to_bits(),
        viewangles: va.map(f32::to_bits),
    }
}

// ---------------------------------------------------------------------------
// Cvars every entry point can read.

#[derive(Clone, Copy)]
struct Cvars {
    altnoclip: f32,
    maxspeed: f32,
    accelerate: f32,
    idealpitchscale: f32,
    edgefriction: f32,
}

impl Cvars {
    /// Upstream defaults (Quake/sv_user_glue.c's initializers).
    fn defaults() -> Self {
        Cvars {
            altnoclip: 1.0,
            maxspeed: 320.0,
            accelerate: 10.0,
            idealpitchscale: 0.8,
            edgefriction: 2.0,
        }
    }

    fn apply(&self) {
        // SAFETY: plain scalar writes to both storages.
        unsafe {
            ctest_svuser_set_cvars(
                self.altnoclip,
                self.maxspeed,
                self.accelerate,
                self.idealpitchscale,
                self.edgefriction,
            )
        };
    }
}

// ---------------------------------------------------------------------------
// One-player setup: player edict is always EDICT_NUM(1), host_client is
// always slot 0 pointed at that same edict -- both already true by default
// after ctest_svuser_reset (see that function's own doc comment), so no
// separate sv_player setter is needed for the SV_SetIdealPitch/SV_ClientThink
// suites below.

fn setup_single(
    side: Side,
    player: Player,
    cmd: (f32, f32, f32),
    cv: Cvars,
    frametime: f64,
    vmtime: f64,
) {
    // SAFETY: resets the shared M3 room + edict arena + client array fresh
    // for this side's run; num_edicts=2 is the minimum (EDICT_NUM(1) valid).
    unsafe { ctest_svuser_reset(2, 1, frametime, vmtime) };
    side.clear_world();
    player.apply(1);
    // SAFETY: slot 0 is in range.
    unsafe { ctest_svuser_set_cmd(0, cmd.0, cmd.1, cmd.2) };
    cv.apply();
}

fn diff_single<F>(
    player: Player,
    cmd: (f32, f32, f32),
    cv: Cvars,
    frametime: f64,
    vmtime: f64,
    body: F,
) -> PlayerSnap
where
    F: Fn(Side),
{
    let mut last: Option<PlayerSnap> = None;
    for side in SIDES {
        setup_single(side, player, cmd, cv, frametime, vmtime);
        body(side);
        let snap = get_player(1);
        if let Some(prev) = &last {
            assert_eq!(prev.scalars, snap.scalars, "player scalars (num=1)");
            assert_eq!(prev.vectors, snap.vectors, "player vectors (num=1)");
            assert_eq!(prev.button0, snap.button0, "button0");
            assert_eq!(prev.button2, snap.button2, "button2");
            assert_eq!(prev.impulse, snap.impulse, "impulse");
        }
        last = Some(snap);
    }
    last.unwrap()
}

// ===========================================================================
// SV_SetIdealPitch

#[test]
fn set_ideal_pitch_off_ground_is_noop() {
    let _g = lock();
    let mut p = Player::blank();
    p.flags = 0.0; // not FL_ONGROUND
    p.idealpitch = 12.5; // a nonzero sentinel that must survive untouched
    let snap = diff_single(p, (0.0, 0.0, 0.0), Cvars::defaults(), 0.05, 10.0, |s| {
        s.set_ideal_pitch()
    });
    // Sanity: the early return (sv_user.c:61-62) really did nothing.
    assert_eq!(
        f32::from_bits(snap.scalars[6]),
        12.5,
        "idealpitch must be untouched off-ground"
    );
}

#[test]
fn set_ideal_pitch_on_ground_facing_variants() {
    let _g = lock();
    // A handful of yaw angles exercise sin/cos at values away from the axes
    // (ADR-010: the double-precision sin/cos narrow-to-float boundary).
    for yaw in [0.0f32, 17.0, 45.0, 90.3, 179.5, 271.25, 359.9] {
        let mut p = Player::blank();
        p.angles[YAW] = yaw;
        diff_single(p, (0.0, 0.0, 0.0), Cvars::defaults(), 0.05, 10.0, |s| {
            s.set_ideal_pitch()
        });
    }
}

#[test]
fn set_ideal_pitch_scale_cvar_boundary() {
    let _g = lock();
    let mut p = Player::blank();
    p.angles[YAW] = 33.0;
    for scale in [0.0f32, 0.8, 1.0, -0.3] {
        let mut cv = Cvars::defaults();
        cv.idealpitchscale = scale;
        diff_single(p, (0.0, 0.0, 0.0), cv, 0.05, 10.0, |s| s.set_ideal_pitch());
    }
}

// ===========================================================================
// SV_ClientThink

#[test]
fn client_think_movetype_none_is_noop() {
    let _g = lock();
    let mut p = Player::blank();
    p.movetype = MOVETYPE_NONE;
    p.velocity = [40.0, -10.0, 5.0]; // must survive untouched
    p.punchangle = [3.0, 0.0, 0.0]; // DropPunchAngle must not run either
    let snap = diff_single(p, (100.0, 0.0, 0.0), Cvars::defaults(), 0.05, 10.0, |s| {
        s.client_think()
    });
    let velocity: [u32; 3] = snap.vectors[3..6].try_into().unwrap();
    assert_eq!(
        velocity.map(f32::from_bits),
        [40.0, -10.0, 5.0],
        "velocity untouched (MOVETYPE_NONE early return)"
    );
}

#[test]
fn client_think_dead_early_return_after_droppunchangle() {
    let _g = lock();
    // DropPunchAngle runs before the health<=0 check (sv_user.c:429-435), so
    // punchangle should decay even though the rest of the function returns.
    for health in [0.0f32, -5.0, -0.001] {
        let mut p = Player::blank();
        p.health = health;
        p.punchangle = [8.0, 0.0, 0.0];
        let snap = diff_single(
            p,
            (100.0, 0.0, 0.0),
            Cvars::defaults(),
            1.0 / 72.0,
            10.0,
            |s| s.client_think(),
        );
        // Sanity: punchangle actually moved (VectorNormalize/decay ran).
        let pa = &snap.vectors[12..15];
        assert_ne!(
            f32::from_bits(pa[0]),
            8.0,
            "DropPunchAngle must still run before the health<=0 return"
        );
    }
}

#[test]
fn client_think_waterjump_branch() {
    let _g = lock();
    let mut p = Player::blank();
    p.flags = (FL_ONGROUND | FL_WATERJUMP) as f32;
    p.waterlevel = 1.0;
    p.teleport_time = 5.0;
    p.movedir = [77.0, -33.0, 0.0];
    // vmtime > teleport_time so SV_WaterJump also clears FL_WATERJUMP.
    diff_single(p, (0.0, 0.0, 0.0), Cvars::defaults(), 0.05, 10.0, |s| {
        s.client_think()
    });

    let mut p2 = Player::blank();
    p2.flags = (FL_ONGROUND | FL_WATERJUMP) as f32;
    p2.waterlevel = 2.0;
    p2.teleport_time = 500.0; // vmtime(10.0) < teleport_time: flag survives
    p2.movedir = [1.0, 2.0, 0.0];
    diff_single(p2, (0.0, 0.0, 0.0), Cvars::defaults(), 0.05, 10.0, |s| {
        s.client_think()
    });
}

#[test]
fn client_think_altnoclip_branch() {
    let _g = lock();
    let mut p = Player::blank();
    p.movetype = MOVETYPE_NOCLIP;
    p.angles = [10.0, 45.0, 0.0];
    p.v_angle = [10.0, 45.0, 0.0];
    let mut cv = Cvars::defaults();
    cv.altnoclip = 1.0;
    diff_single(p, (200.0, -50.0, 30.0), cv, 0.05, 10.0, |s| {
        s.client_think()
    });

    // altnoclip == 0: falls through to the waterlevel/AirMove chain instead
    // (movetype stays NOCLIP, so with waterlevel<2 it takes SV_AirMove's
    // MOVETYPE_NOCLIP branch, which is a different code path than
    // SV_NoclipMove).
    let mut cv0 = cv;
    cv0.altnoclip = 0.0;
    diff_single(p, (200.0, -50.0, 30.0), cv0, 0.05, 10.0, |s| {
        s.client_think()
    });
}

#[test]
fn client_think_watermove_branch() {
    let _g = lock();
    for waterlevel in [2.0f32, 3.0] {
        let mut p = Player::blank();
        p.waterlevel = waterlevel;
        p.watertype = -3.0; // CONTENTS_WATER-ish sentinel, unused by SV_WaterMove itself
        p.velocity = [30.0, 0.0, -10.0];
        p.v_angle = [0.0, 90.0, 0.0];
        diff_single(p, (0.0, 0.0, 0.0), Cvars::defaults(), 0.05, 10.0, |s| {
            s.client_think()
        }); // drift-towards-bottom branch
        diff_single(p, (50.0, 25.0, 40.0), Cvars::defaults(), 0.05, 10.0, |s| {
            s.client_think()
        }); // explicit wish branch
    }
    // waterlevel just under 2 takes SV_AirMove instead -- the branch boundary.
    let mut p = Player::blank();
    p.waterlevel = 1.999;
    diff_single(p, (50.0, 0.0, 0.0), Cvars::defaults(), 0.05, 10.0, |s| {
        s.client_think()
    });
}

#[test]
fn client_think_airmove_onground_branch() {
    let _g = lock();
    let mut p = Player::blank();
    p.flags = FL_ONGROUND as f32;
    p.velocity = [40.0, 40.0, 0.0]; // nonzero, so SV_UserFriction's sqrt path runs
    diff_single(
        p,
        (300.0, 100.0, 0.0),
        Cvars::defaults(),
        1.0 / 72.0,
        10.0,
        |s| s.client_think(),
    );
    // A slower frametime pushes SV_UserFriction's analytic-vs-classic split
    // (sv_analyticphysics_frame is stub-controlled and out of this wave's
    // reach, so both branches are exercised across the differential runs by
    // whatever the stub fixture currently latches -- the assertion is on
    // agreement, not on which branch fired).
    diff_single(p, (300.0, 100.0, 0.0), Cvars::defaults(), 0.2, 10.0, |s| {
        s.client_think()
    });
}

/// sv_user.c:211: `if (accelspeed > addspeed) accelspeed = addspeed;`.
/// Every other onground test in this file keeps the player far below wish
/// speed, so `addspeed` (wishspeed - currentspeed) stays large and this
/// clamp never actually fires. Starting velocity already close to the wish
/// direction/speed shrinks `addspeed` down near
/// `sv_accelerate.value * host_frametime * wishspeed` (the unclamped
/// `accelspeed`), so the clamp is the deciding factor in the result.
#[test]
fn client_think_airmove_onground_accelerate_clamp() {
    let _g = lock();
    let mut p = Player::blank();
    p.flags = FL_ONGROUND as f32;
    p.velocity = [315.0, 0.0, 0.0]; // near the 320.0 default sv_maxspeed
    diff_single(
        p,
        (320.0, 0.0, 0.0),
        Cvars::defaults(),
        1.0 / 72.0,
        10.0,
        |s| s.client_think(),
    );
}

/// sv_user.c:141-146: SV_UserFriction traces 34 units straight down from the
/// player's feet (offset 16 units along the velocity direction). If nothing
/// is hit (trace.fraction == 1.0 -- an unguarded ledge), friction is
/// `sv_friction.value * sv_edgefriction.value`; otherwise (solid ground
/// within reach) it is plain `sv_friction.value`. The synthetic room's floor
/// sits at z=-192 (`ctest_world_boxes`'s open-room box in stubs.c, mins
/// z=-192), so `Player::blank()`'s default origin z=-80 leaves the trace 80+
/// units clear of it (fraction==1.0, the multiply branch -- also exercised
/// by client_think_airmove_onground_branch above), while origin z=-165
/// leaves only 27 units of clearance, so the trace lands on the floor before
/// reaching its full 34-unit length (fraction<1.0, the plain-friction
/// branch). Both x/y stay near the origin, well clear of the pillar box at
/// x/y in [32,96] and the liquid boxes (all at x or y <= -64).
#[test]
fn client_think_airmove_friction_edge_vs_solid_floor() {
    let _g = lock();
    for origin_z in [-80.0f32, -165.0] {
        let mut p = Player::blank();
        p.flags = FL_ONGROUND as f32;
        p.origin[2] = origin_z;
        p.velocity = [40.0, 40.0, 0.0]; // nonzero, so SV_UserFriction's sqrt path runs
        diff_single(
            p,
            (300.0, 100.0, 0.0),
            Cvars::defaults(),
            1.0 / 72.0,
            10.0,
            |s| s.client_think(),
        );
    }
}

#[test]
fn client_think_airmove_airborne_branch() {
    let _g = lock();
    let mut p = Player::blank();
    p.flags = 0.0; // not on ground
    p.velocity = [0.0, 0.0, -50.0];
    diff_single(
        p,
        (300.0, -100.0, 0.0),
        Cvars::defaults(),
        0.05,
        10.0,
        |s| s.client_think(),
    );
}

#[test]
fn client_think_movetype_walk_upmove_ignored() {
    let _g = lock();
    // sv_user.c:381 -- wishvel[2] only takes cmd.upmove when movetype != WALK
    // (int-cast comparison, the one deliberately different from the others).
    let mut walk = Player::blank();
    walk.movetype = MOVETYPE_WALK;
    diff_single(
        walk,
        (0.0, 0.0, 400.0),
        Cvars::defaults(),
        0.05,
        10.0,
        |s| s.client_think(),
    );

    let mut fly = Player::blank();
    fly.movetype = 5.0; // MOVETYPE_FLY, != MOVETYPE_WALK
    diff_single(fly, (0.0, 0.0, 400.0), Cvars::defaults(), 0.05, 10.0, |s| {
        s.client_think()
    });
}

#[test]
fn client_think_fixangle_suppresses_angle_update() {
    let _g = lock();
    let mut p = Player::blank();
    p.v_angle = [30.0, 60.0, 0.0];
    p.punchangle = [6.0, 0.0, 0.0];
    p.angles = [1.0, 2.0, 3.0];
    p.fixangle = 1.0;
    let snap = diff_single(p, (0.0, 0.0, 0.0), Cvars::defaults(), 0.05, 10.0, |s| {
        s.client_think()
    });
    // Sanity: PITCH/YAW must be untouched (only ROLL, always written); the
    // pre-existing values [1,2] survive since fixangle suppressed the update.
    assert_eq!(f32::from_bits(snap.vectors[6 + PITCH]), 1.0);
    assert_eq!(f32::from_bits(snap.vectors[6 + YAW]), 2.0);

    let mut p2 = p;
    p2.fixangle = 0.0;
    let snap2 = diff_single(p2, (0.0, 0.0, 0.0), Cvars::defaults(), 0.05, 10.0, |s| {
        s.client_think()
    });
    assert_ne!(
        f32::from_bits(snap2.vectors[6 + PITCH]),
        1.0,
        "fixangle==0 must recompute PITCH"
    );
}

// ===========================================================================
// SV_RunClients

/// A single client-array setup, applied identically to both sides right
/// before each side's SV_RunClients call.
struct ClientSetup {
    slot: c_int,
    active: bool,
    spawned: bool,
    has_netconnection: bool,
    cmd: (f32, f32, f32),
}

fn setup_run_clients(
    side: Side,
    maxclients: c_int,
    clients: &[ClientSetup],
    sv_paused: bool,
    key_dest_menu: bool,
    frametime: f64,
    vmtime: f64,
) {
    // SAFETY: resets the shared room/arena/client array fresh for this call.
    unsafe { ctest_svuser_reset(2, maxclients, frametime, vmtime) };
    side.clear_world();
    let mut p = Player::blank();
    // V_CalcRoll (view.c:87-108) rolls on the component of velocity along the
    // view's right vector, not on speed alone: with angles=[0,0,0] (blank()'s
    // default) forward points along +X, so a purely-forward [12,0,0] dots to
    // zero against right and produces no roll at all. A sideways component is
    // required for SV_ClientThink's roll calc to actually be observable.
    p.velocity = [0.0, 40.0, 0.0];
    p.apply(1);
    Cvars::defaults().apply();
    for c in clients {
        // SAFETY: slot is caller-provided and always in range in this file.
        unsafe {
            ctest_svuser_set_client(
                c.slot,
                1,
                c.active as c_int,
                c.spawned as c_int,
                c.has_netconnection as c_int,
            );
            ctest_svuser_set_cmd(c.slot, c.cmd.0, c.cmd.1, c.cmd.2);
            ctest_svuser_set_sv_paused(sv_paused as c_int);
            // key_game == 0 (Quake/keys.h); "menu" is any nonzero keydest_t.
            ctest_svuser_set_key_dest(if key_dest_menu { 1 } else { 0 });
        }
    }
}

#[derive(Debug, PartialEq)]
struct RunClientsSnap {
    player: PlayerSnap,
    cmds: Vec<CmdSnap>,
    host_client_slot: c_int,
}

fn diff_run_clients(
    maxclients: c_int,
    clients: Vec<ClientSetup>,
    sv_paused: bool,
    key_dest_menu: bool,
    frametime: f64,
    vmtime: f64,
) -> RunClientsSnap {
    let mut last: Option<RunClientsSnap> = None;
    for side in SIDES {
        setup_run_clients(
            side,
            maxclients,
            &clients,
            sv_paused,
            key_dest_menu,
            frametime,
            vmtime,
        );
        side.run_clients();
        let snap = RunClientsSnap {
            player: get_player(1),
            cmds: (0..maxclients).map(get_cmd).collect(),
            // SAFETY: plain scalar read of the shared host_client pointer.
            host_client_slot: unsafe { ctest_svuser_host_client_slot() },
        };
        if let Some(prev) = &last {
            assert_eq!(
                prev.player, snap.player,
                "player edict state after SV_RunClients"
            );
            assert_eq!(
                prev.cmds, snap.cmds,
                "per-client cmd state after SV_RunClients"
            );
            assert_eq!(
                prev.host_client_slot, snap.host_client_slot,
                "host_client ends on the same slot"
            );
        }
        last = Some(snap);
    }
    last.unwrap()
}

#[test]
fn run_clients_inactive_client_is_skipped() {
    let _g = lock();
    let snap = diff_run_clients(
        2,
        vec![
            ClientSetup {
                slot: 0,
                active: true,
                spawned: true,
                has_netconnection: false,
                cmd: (5.0, 5.0, 5.0),
            },
            ClientSetup {
                slot: 1,
                active: false,
                spawned: true,
                has_netconnection: false,
                cmd: (9.0, 9.0, 9.0), // must survive untouched: !active skips the whole body
            },
        ],
        false,
        false,
        1.0 / 72.0,
        10.0,
    );
    // Sanity: host_client ends ONE PAST the last slot (== maxclients), even
    // though the last slot's body was skipped -- sv_user.c's
    // `for (i = 0, host_client = svs.clients; i < svs.maxclients; i++,
    // host_client++)` still runs its post-increment on the final (inactive)
    // iteration before the loop condition fails.
    assert_eq!(
        snap.host_client_slot, 2,
        "host_client must end one past the last slot (== maxclients)"
    );
    assert_eq!(
        f32::from_bits(snap.cmds[1].forwardmove),
        9.0,
        "inactive client's cmd must be untouched"
    );
}

#[test]
fn run_clients_unspawned_client_clears_cmd() {
    let _g = lock();
    let snap = diff_run_clients(
        1,
        vec![ClientSetup {
            slot: 0,
            active: true,
            spawned: false,
            has_netconnection: false,
            cmd: (11.0, 22.0, 33.0),
        }],
        false,
        false,
        1.0 / 72.0,
        10.0,
    );
    assert_eq!(
        f32::from_bits(snap.cmds[0].forwardmove),
        0.0,
        "!spawned must memset the cmd"
    );
    assert_eq!(f32::from_bits(snap.cmds[0].sidemove), 0.0);
    assert_eq!(f32::from_bits(snap.cmds[0].upmove), 0.0);
}

#[test]
fn run_clients_no_netconnection_republishes_viewangles() {
    let _g = lock();
    let mut p_vangle = Player::blank();
    p_vangle.v_angle = [12.0, 34.0, 0.0];
    p_vangle.apply(1); // overwritten again inside setup_run_clients below, so
                       // instead thread v_angle through a dedicated case:
    let snap = diff_run_clients(
        1,
        vec![ClientSetup {
            slot: 0,
            active: true,
            spawned: true,
            has_netconnection: false,
            cmd: (0.0, 0.0, 0.0),
        }],
        false,
        false,
        1.0 / 72.0,
        10.0,
    );
    // The fixture's blank() player has v_angle == [0,0,0]; with no
    // netconnection, cmd.viewangles is republished from it every frame.
    assert_eq!(snap.cmds[0].viewangles.map(f32::from_bits), [0.0, 0.0, 0.0]);
}

#[test]
fn run_clients_with_netconnection_does_not_touch_viewangles() {
    let _g = lock();
    let snap = diff_run_clients(
        1,
        vec![ClientSetup {
            slot: 0,
            active: true,
            spawned: true,
            has_netconnection: true,
            cmd: (0.0, 0.0, 0.0),
        }],
        false,
        false,
        1.0 / 72.0,
        10.0,
    );
    // cmd.viewangles was zeroed by ctest_svuser_set_cmd and never republished
    // (sv_user.c:661-666's `if (!host_client->netconnection)` gate is false).
    assert_eq!(snap.cmds[0].viewangles.map(f32::from_bits), [0.0, 0.0, 0.0]);
}

#[test]
fn run_clients_paused_suppresses_client_think() {
    let _g = lock();
    let snap = diff_run_clients(
        1,
        vec![ClientSetup {
            slot: 0,
            active: true,
            spawned: true,
            has_netconnection: true,
            cmd: (0.0, 0.0, 0.0),
        }],
        true, // sv.paused
        false,
        1.0 / 72.0,
        10.0,
    );
    // Sanity: with velocity=[12,0,0] set by setup_run_clients and SV_ClientThink
    // suppressed, angles[ROLL] (always written by an un-suppressed call) must
    // stay at the blank() default of 0.
    assert_eq!(
        f32::from_bits(snap.player.vectors[6 + ROLL]),
        0.0,
        "sv.paused must suppress SV_ClientThink entirely"
    );
}

#[test]
fn run_clients_menu_suppresses_think_only_in_single_player() {
    let _g = lock();
    // maxclients == 1 (single player) and key_dest != key_game: suppressed.
    let snap_sp = diff_run_clients(
        1,
        vec![ClientSetup {
            slot: 0,
            active: true,
            spawned: true,
            has_netconnection: true,
            cmd: (0.0, 0.0, 0.0),
        }],
        false,
        true, // key_dest: menu
        1.0 / 72.0,
        10.0,
    );
    assert_eq!(
        f32::from_bits(snap_sp.player.vectors[6 + ROLL]),
        0.0,
        "single-player + menu must suppress SV_ClientThink"
    );

    // maxclients > 1: the key_dest gate is bypassed even in the menu.
    let snap_mp = diff_run_clients(
        2,
        vec![
            ClientSetup {
                slot: 0,
                active: true,
                spawned: true,
                has_netconnection: true,
                cmd: (0.0, 0.0, 0.0),
            },
            ClientSetup {
                slot: 1,
                active: false,
                spawned: false,
                has_netconnection: false,
                cmd: (0.0, 0.0, 0.0),
            },
        ],
        false,
        true, // key_dest: menu
        1.0 / 72.0,
        10.0,
    );
    assert_ne!(
        f32::from_bits(snap_mp.player.vectors[6 + ROLL]),
        0.0,
        "multiplayer must run SV_ClientThink regardless of key_dest"
    );
}

#[test]
fn run_clients_normal_path_runs_client_think() {
    let _g = lock();
    let snap = diff_run_clients(
        1,
        vec![ClientSetup {
            slot: 0,
            active: true,
            spawned: true,
            has_netconnection: true,
            cmd: (100.0, 0.0, 0.0),
        }],
        false,
        false,
        1.0 / 72.0,
        10.0,
    );
    assert_ne!(
        f32::from_bits(snap.player.vectors[6 + ROLL]),
        0.0,
        "an un-suppressed frame must run SV_ClientThink (V_CalcRoll on nonzero velocity)"
    );
}
