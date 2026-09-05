//! Differential gate for `Quake/sv_main.c` -- the connection half left by the
//! T6.1 split: protocol negotiation, the two datagram event writers, the
//! serverinfo handshake, client connect/save-spawnparms and the precache
//! index lookups. Rust migration Phase 7, M6, task T6.5.
//!
//! Both sides run against the SAME fixture in `stubs/sv_main_ref.c`, which
//! keeps two parallel copies of every mutable object the subjects touch:
//! `c_ref_sv`/`c_ref_svs`/`ctest_svmain_clients_c` for the oracle, and
//! `sv`/`svs`/`ctest_svmain_clients_r` for the Rust port. `ctest_svmain_reset`
//! seeds both identically, so a test drives side C, drives side Rust, and
//! compares -- no re-seeding in between, and every buffer diff is a direct
//! memcmp of the two columns.
//!
//! Every driver returns the side's `Host_Guard` status, so an ADR-009 raise
//! is a comparable value rather than a crash. `stubs/stubs.c`'s `Sys_Error`
//! longjmps (unlike the shipping engine, where it terminates), so no test
//! here drives a `Sys_Error` arm -- `sv_main.c:213`, `:784` and `:835` are
//! deliberately NOT covered, and are called out in the report.
//!
//! Observability limits forced by `stubs/stubs.c` (task T6.5 may not edit it),
//! each worked around rather than papered over:
//!  - `NET_CanSendMessage`/`NET_SendMessage` always succeed, so
//!    `SV_SendServerinfo`'s mid-function flush always `SZ_Clear`s
//!    `client->message`; the wire bytes are therefore compared as RESIDUE in
//!    `client->msgbuf` (`ctest_svmain_msgbuf_diff`), which the fixture zeroes
//!    before each drive.
//!  - `NET_QSocketGetTrueAddressString` returns `"ctest"`, never `"LOCAL"`,
//!    so `SV_SendServerinfo`'s local super-size arm (sv_main.c:505) is
//!    unreachable and the `DATAGRAM_MTU` clamp always applies.
//!  - `NET_QSocketGetProQuakeAngleHack` is false, so the NETQUAKE
//!    2048-entity arm (sv_main.c:478) is unreachable and NETQUAKE clients
//!    always land on 600.
//!  - `NET_CheckNewConnections` returns NULL, so `SV_CheckForNewClients` can
//!    only be gated on its immediate loop exit.
//!  - `Mod_ForName`, `PR_LoadProgs`, `ED_LoadFromFile` and `Host_ClearMemory`
//!    are inert or `Sys_Error` stubs and `SV_Precache_Model` is a test
//!    double, so `SV_SpawnServer` is NOT drivable here at all.
//!
//! `SV_Init` needs its own shape. Both registries would fight over `cvar_t`'s
//! single `next` field, so `ctest_svmain_run_init_pair` is one-shot per test
//! binary: it backs up both columns, runs the oracle under `Host_Guard`,
//! snapshots, restores, then runs the Rust core and snapshots again. The
//! ORDER of the 23 registrations at `sv_main.c:164-190` is not visible in the
//! `cvar_vars` chain (`Cvar_RegisterVariable` inserts alphabetically), so the
//! order observable is `svs.serverinfo`: `Info_SetKey` appends the three
//! `CVAR_SERVERINFO` cvars -- `sv_gravity` (2nd), `sv_friction` (3rd),
//! `sv_maxspeed` (6th) -- in REGISTRATION order, which differs from their
//! alphabetical order. That is the cheapest available proof, and it is the
//! only part of the order that is observable at all; see the report.

use core::ffi::{c_char, c_float, c_int, c_uint, CStr};
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

// ---------------------------------------------------------------------------
// Constants transcribed from protocol.h / quakedef.h / server.h.

const SIDE_C: c_int = 0;
const SIDE_RS: c_int = 1;

const PROTOCOL_NETQUAKE: c_int = 15;
const PROTOCOL_FITZQUAKE: c_int = 666;
const PROTOCOL_RMQ: c_int = 999;

const PEXT1_CSQC: c_uint = 0x4000_0000;
const PEXT1_SUPPORTED_SERVER: c_uint = PEXT1_CSQC;
const PEXT2_REPLACEMENTDELTAS: c_uint = 0x0000_0008;
const PEXT2_PREDINFO: c_uint = 0x0000_0020;
const PEXT2_SUPPORTED_SERVER: c_uint = PEXT2_REPLACEMENTDELTAS | PEXT2_PREDINFO;

const NUM_TOTAL_SPAWN_PARMS: usize = 64;
const MAX_MSGLEN: usize = 64000;

const SVC_PRINT: c_int = 8;
const SVC_STUFFTEXT: c_int = 9;
const SVC_PARTICLE: c_int = 18;
const SVC_SOUND: c_int = 6;

const SND_VOLUME: c_int = 1 << 0;
const SND_ATTENUATION: c_int = 1 << 1;
const SND_LARGEENTITY: c_int = 1 << 3;
const SND_LARGESOUND: c_int = 1 << 4;

const PRESPAWN_FLUSH: c_int = 1;

/// `stubs/stubs.c`'s `Host_Guard` status set, which mirrors ADR-009's.
const GUARD_OK: c_int = 0;
const GUARD_HOST_ERROR: c_int = 1;

// ---------------------------------------------------------------------------
// The fixture in stubs/sv_main_ref.c.

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
struct ClientSnap {
    active: c_int,
    spawned: c_int,
    sendsignon: c_int,
    pextknown: c_int,
    protocol_pext2: c_uint,
    limit_entities: c_uint,
    limit_unreliable: c_uint,
    limit_reliable: c_uint,
    limit_models: c_uint,
    limit_sounds: c_uint,
    signon_models: c_uint,
    signon_sounds: c_uint,
    message_cursize: c_int,
    message_maxsize: c_int,
    message_overflowed: c_int,
    datagram_cursize: c_int,
    datagram_maxsize: c_int,
    datagram_overflowed: c_int,
    last_message: f64,
    edictnum: c_int,
    name: [c_char; 32],
    spawn_parms: [c_float; NUM_TOTAL_SPAWN_PARMS],
}

impl Default for ClientSnap {
    fn default() -> Self {
        // SAFETY: `ClientSnap` is a `#[repr(C)]` aggregate of integers, floats
        // and arrays of them; the all-zero bit pattern is a valid value of
        // every field, and the C fixture memsets its own copy the same way.
        unsafe { core::mem::zeroed() }
    }
}

extern "C" {
    fn ctest_svmain_reset(
        protocol: c_int,
        protocolflags: c_uint,
        maxclients: c_int,
        loadgame: c_int,
    );
    fn ctest_svmain_set_pext(pext1: c_uint, pext2: c_uint);
    fn ctest_svmain_set_client_pext(i: c_int, pext2: c_uint, pextknown: c_int);
    fn ctest_svmain_set_client_flags(i: c_int, active: c_int, spawned: c_int);
    fn ctest_svmain_set_client_limits(i: c_int, entities: c_uint, sounds: c_uint);
    fn ctest_svmain_set_spawn_parm(i: c_int, j: c_int, v: c_float);
    fn ctest_svmain_set_global_parms(base: c_float);
    fn ctest_svmain_set_edict_origin(
        num: c_int,
        x: c_float,
        y: c_float,
        z: c_float,
        mins: c_float,
        maxs: c_float,
    );
    fn ctest_svmain_clear_all_bufs();
    fn ctest_svmain_set_model_slot(index: c_int, marker: c_int);

    fn ctest_svmain_snap_client(side: c_int, i: c_int, out: *mut ClientSnap);
    fn ctest_svmain_client_snap_size() -> c_int;
    fn ctest_svmain_msgbuf_diff(i: c_int) -> c_int;
    fn ctest_svmain_client_datagram_diff(i: c_int) -> c_int;
    fn ctest_svmain_sv_datagram_diff() -> c_int;
    fn ctest_svmain_sv_datagram_size(side: c_int) -> c_int;
    fn ctest_svmain_sv_datagram_byte(side: c_int, i: c_int) -> c_int;
    fn ctest_svmain_client_msg_byte(side: c_int, i: c_int, k: c_int) -> c_int;
    fn ctest_svmain_client_datagram_byte(side: c_int, i: c_int, k: c_int) -> c_int;
    fn ctest_svmain_protocol(side: c_int) -> c_int;
    fn ctest_svmain_protocol_pext1(side: c_int) -> c_uint;
    fn ctest_svmain_protocol_pext2(side: c_int) -> c_uint;
    fn ctest_svmain_serverflags(side: c_int) -> c_int;

    fn ctest_svmain_drive_startparticle(
        side: c_int,
        org: *const c_float,
        dir: *const c_float,
        color: c_int,
        count: c_int,
    ) -> c_int;
    fn ctest_svmain_drive_startsound(
        side: c_int,
        entnum: c_int,
        origin: *const c_float,
        channel: c_int,
        sample: *const c_char,
        volume: c_int,
        attenuation: c_float,
    ) -> c_int;
    fn ctest_svmain_drive_localsound(side: c_int, clientnum: c_int, sample: *const c_char)
        -> c_int;
    fn ctest_svmain_drive_serverinfo(side: c_int, clientnum: c_int) -> c_int;
    fn ctest_svmain_drive_connectclient(side: c_int, clientnum: c_int) -> c_int;
    fn ctest_svmain_drive_checknewclients(side: c_int) -> c_int;
    fn ctest_svmain_drive_savespawnparms(side: c_int) -> c_int;
    fn ctest_svmain_drive_cleardatagram(side: c_int);
    fn ctest_svmain_drive_modelindex(side: c_int, name: *const c_char) -> c_int;
    fn ctest_svmain_modelforindex_raw(side: c_int, index: c_int) -> isize;
    fn ctest_svmain_drive_protocol_f(side: c_int, cmdtext: *const c_char) -> c_int;
    fn ctest_svmain_drive_pext_f(
        side: c_int,
        cmdtext: *const c_char,
        from_client: c_int,
        clientnum: c_int,
    ) -> c_int;

    fn ctest_svmain_run_init_pair() -> c_int;
    fn ctest_svmain_init_cvar_count() -> c_int;
    fn ctest_svmain_init_cvar_name(side: c_int, i: c_int) -> *const c_char;
    fn ctest_svmain_init_cvar_string(side: c_int, i: c_int) -> *const c_char;
    fn ctest_svmain_init_cvar_value(side: c_int, i: c_int) -> c_float;
    fn ctest_svmain_init_cvar_flags(side: c_int, i: c_int) -> c_uint;
    fn ctest_svmain_init_serverinfo(side: c_int) -> *const c_char;
    fn ctest_svmain_init_status_get(side: c_int) -> c_int;
    fn ctest_svmain_init_protocol_get(side: c_int) -> c_int;
    fn ctest_svmain_init_pext_get(side: c_int, which: c_int) -> c_uint;

    fn ctest_clear_con_log();
    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(i: c_int) -> *const c_char;
}

// ---------------------------------------------------------------------------
// Thin safe wrappers.

fn reset(protocol: c_int, protocolflags: c_uint, maxclients: c_int, loadgame: bool) {
    // SAFETY: the fixture owns all the storage it writes and takes no
    // pointers from us; the test lock serializes every caller in this binary.
    unsafe { ctest_svmain_reset(protocol, protocolflags, maxclients, c_int::from(loadgame)) }
}

fn snap(side: c_int, i: c_int) -> ClientSnap {
    let mut out = ClientSnap::default();
    // SAFETY: `out` is a live, correctly-typed `#[repr(C)]` mirror of the
    // fixture's `ctest_svmain_client_snap_t`; the layout equality is asserted
    // by `client_snap_layout_matches`.
    unsafe { ctest_svmain_snap_client(side, i, &mut out) };
    out
}

fn con_log() -> Vec<String> {
    // SAFETY: `ctest_con_log_len`/`_get` read a static ring in stubs.c; every
    // returned pointer is a NUL-terminated static buffer valid for the call.
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

fn clear_con() {
    // SAFETY: resets a static ring buffer in stubs.c; no arguments.
    unsafe { ctest_clear_con_log() }
}

/// Drives one side and returns `(guard status, console lines that side printed)`.
fn drive_logged(f: impl FnOnce() -> c_int) -> (c_int, Vec<String>) {
    clear_con();
    let status = f();
    (status, con_log())
}

fn cstr(s: &str) -> std::ffi::CString {
    std::ffi::CString::new(s).unwrap()
}

/// Asserts the two sides produced identical console output.
fn assert_log_eq(what: &str, c: &[String], rs: &[String]) {
    assert_eq!(c, rs, "{what}: console output diverged");
}

fn assert_client_eq(what: &str, i: c_int) {
    assert_eq!(
        snap(SIDE_C, i),
        snap(SIDE_RS, i),
        "{what}: client[{i}] state diverged"
    );
    // SAFETY: both diff helpers memcmp two fixture-owned arrays of equal size.
    let (msg, dg) = unsafe {
        (
            ctest_svmain_msgbuf_diff(i),
            ctest_svmain_client_datagram_diff(i),
        )
    };
    assert_eq!(msg, -1, "{what}: client[{i}] msgbuf diverged at byte {msg}");
    assert_eq!(dg, -1, "{what}: client[{i}] datagram diverged at byte {dg}");
}

fn assert_all_clients_eq(what: &str) {
    for i in 0..4 {
        assert_client_eq(what, i);
    }
}

fn assert_sv_datagram_eq(what: &str) {
    assert_eq!(
        // SAFETY: fixture accessors over `sv`/`c_ref_sv`, no arguments to validate.
        unsafe { ctest_svmain_sv_datagram_size(SIDE_C) },
        // SAFETY: same contract as the paired call above.
        unsafe { ctest_svmain_sv_datagram_size(SIDE_RS) },
        "{what}: sv.datagram.cursize diverged"
    );
    // SAFETY: memcmp of the two fixture-owned `datagram_buf` arrays.
    let d = unsafe { ctest_svmain_sv_datagram_diff() };
    assert_eq!(d, -1, "{what}: sv.datagram bytes diverged at byte {d}");
}

fn sv_datagram_bytes(side: c_int) -> Vec<c_int> {
    // SAFETY: `i` stays below the reported `cursize`, which is the fixture's
    // own bound on `datagram_buf`.
    unsafe {
        (0..ctest_svmain_sv_datagram_size(side))
            .map(|i| ctest_svmain_sv_datagram_byte(side, i))
            .collect()
    }
}

fn client_datagram_bytes(side: c_int, i: c_int, n: c_int) -> Vec<c_int> {
    // SAFETY: `n` is bounded by the client datagram size at every call site.
    unsafe {
        (0..n)
            .map(|k| ctest_svmain_client_datagram_byte(side, i, k))
            .collect()
    }
}

fn client_msg_bytes(side: c_int, i: c_int, n: c_int) -> Vec<c_int> {
    // SAFETY: `n` is bounded by MAX_MSGLEN at every call site below.
    unsafe {
        (0..n)
            .map(|k| ctest_svmain_client_msg_byte(side, i, k))
            .collect()
    }
}

// ===========================================================================
// Layout

#[test]
fn client_snap_layout_matches() {
    let _g = lock();
    // SAFETY: a leaf accessor returning `sizeof` of the C struct.
    assert_eq!(
        // SAFETY: same contract as the paired call above.
        unsafe { ctest_svmain_client_snap_size() } as usize,
        core::mem::size_of::<ClientSnap>()
    );
}

// ===========================================================================
// SV_Init -- the registration-order differential (sv_main.c:137-232)

/// Runs the one-shot pair and yields nothing; every `sv_init_*` test calls it
/// first, and only the first call does any work.
fn init_pair() {
    // SAFETY: idempotent one-shot in the fixture; serialized by the test lock.
    unsafe { ctest_svmain_run_init_pair() };
}

#[test]
fn sv_init_registers_the_same_23_cvars_with_the_same_values() {
    let _g = lock();
    init_pair();
    // SAFETY: leaf accessor over a static count.
    let n = unsafe { ctest_svmain_init_cvar_count() };
    assert_eq!(n, 23, "sv_main.c:164-190 registers 23 cvars");
    for i in 0..n {
        // SAFETY: `i < n`; every returned pointer is a NUL-terminated static
        // snapshot buffer owned by the fixture.
        let (cn, rn, cs, rs, cv, rv, cf, rf) = unsafe {
            (
                CStr::from_ptr(ctest_svmain_init_cvar_name(SIDE_C, i))
                    .to_string_lossy()
                    .into_owned(),
                CStr::from_ptr(ctest_svmain_init_cvar_name(SIDE_RS, i))
                    .to_string_lossy()
                    .into_owned(),
                CStr::from_ptr(ctest_svmain_init_cvar_string(SIDE_C, i))
                    .to_string_lossy()
                    .into_owned(),
                CStr::from_ptr(ctest_svmain_init_cvar_string(SIDE_RS, i))
                    .to_string_lossy()
                    .into_owned(),
                ctest_svmain_init_cvar_value(SIDE_C, i),
                ctest_svmain_init_cvar_value(SIDE_RS, i),
                ctest_svmain_init_cvar_flags(SIDE_C, i),
                ctest_svmain_init_cvar_flags(SIDE_RS, i),
            )
        };
        assert_eq!(cn, rn, "cvar[{i}] name diverged");
        assert_eq!(cs, rs, "cvar[{i}] ({cn}) string diverged");
        assert_eq!(
            cv.to_bits(),
            rv.to_bits(),
            "cvar[{i}] ({cn}) value diverged"
        );
        assert_eq!(cf, rf, "cvar[{i}] ({cn}) flags diverged");
    }
}

#[test]
fn sv_init_leaves_protocol_state_untouched() {
    let _g = lock();
    init_pair();
    // SAFETY: leaf accessors over static snapshots.
    unsafe {
        assert_eq!(
            ctest_svmain_init_status_get(SIDE_C),
            ctest_svmain_init_status_get(SIDE_RS)
        );
        assert_eq!(
            ctest_svmain_init_status_get(SIDE_C),
            GUARD_OK,
            "SV_Init must not raise"
        );
        assert_eq!(
            ctest_svmain_init_protocol_get(SIDE_C),
            ctest_svmain_init_protocol_get(SIDE_RS)
        );
        assert_eq!(
            ctest_svmain_init_pext_get(SIDE_C, 0),
            ctest_svmain_init_pext_get(SIDE_RS, 0)
        );
        assert_eq!(
            ctest_svmain_init_pext_get(SIDE_C, 1),
            ctest_svmain_init_pext_get(SIDE_RS, 1)
        );
    }
}

fn init_serverinfo(side: c_int) -> String {
    // SAFETY: returns a NUL-terminated static snapshot buffer.
    unsafe {
        CStr::from_ptr(ctest_svmain_init_serverinfo(side))
            .to_string_lossy()
            .into_owned()
    }
}

#[test]
fn sv_init_serverinfo_matches_between_sides() {
    let _g = lock();
    init_pair();
    assert_eq!(
        init_serverinfo(SIDE_C),
        init_serverinfo(SIDE_RS),
        "svs.serverinfo diverged"
    );
}

/// The registration-ORDER gate. `Info_SetKey` appends, so the three
/// `CVAR_SERVERINFO` cvars land in `svs.serverinfo` in the order `SV_Init`
/// registered them -- gravity (2nd), friction (3rd), maxspeed (6th) -- which
/// is NOT their alphabetical order (friction < gravity < maxspeed), so a
/// reordered `Cvar_RegisterVariable` sequence moves them here.
#[test]
fn sv_init_serverinfo_key_order_is_registration_order() {
    let _g = lock();
    init_pair();
    for side in [SIDE_C, SIDE_RS] {
        let info = init_serverinfo(side);
        let keys: Vec<&str> = info
            .split('\\')
            .filter(|s| !s.is_empty())
            .step_by(2)
            .collect();
        assert_eq!(
            keys,
            ["sv_gravity", "sv_friction", "sv_maxspeed"],
            "side {side}: serverinfo key order"
        );
    }
}

// ===========================================================================
// SV_StartParticle (sv_main.c:234-260)

fn drive_particle(
    org: [c_float; 3],
    dir: [c_float; 3],
    color: c_int,
    count: c_int,
) -> (c_int, c_int) {
    // SAFETY: both slices are live 3-element arrays for the duration of the
    // call; the fixture copies them before dispatching.
    let c = unsafe {
        ctest_svmain_drive_startparticle(SIDE_C, org.as_ptr(), dir.as_ptr(), color, count)
    };
    // SAFETY: same contract as the paired call above.
    let r = unsafe {
        ctest_svmain_drive_startparticle(SIDE_RS, org.as_ptr(), dir.as_ptr(), color, count)
    };
    (c, r)
}

#[test]
fn start_particle_writes_the_same_datagram() {
    let _g = lock();
    reset(PROTOCOL_FITZQUAKE, 0, 2, false);
    let (c, r) = drive_particle([16.0, -32.5, 64.25], [1.0, -1.0, 0.5], 73, 12);
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_sv_datagram_eq("start_particle");
    let bytes = sv_datagram_bytes(SIDE_C);
    assert_eq!(bytes[0], SVC_PARTICLE);
    // 1 svc + 6 coord + 3 dir + count + color
    assert_eq!(bytes.len(), 12);
    assert_eq!(bytes[7], 16, "dir[0] 1.0*16");
    assert_eq!(bytes[8], 256 - 16, "dir[1] -1.0*16 as a signed char");
    assert_eq!(bytes[9], 8, "dir[2] 0.5*16");
    assert_eq!(bytes[10], 12, "count");
    assert_eq!(bytes[11], 73, "color");
}

/// COMPAT (sv_main.c:253): the clamp is written `if (count > 255.0f) count =
/// 255.0f;` on an `int` -- the comparison promotes to float and the store
/// truncates back. Ported verbatim; the observable is the same 255.
#[test]
fn start_particle_clamps_count_at_255() {
    let _g = lock();
    reset(PROTOCOL_FITZQUAKE, 0, 2, false);
    let (c, r) = drive_particle([0.0; 3], [0.0; 3], 1, 4096);
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_sv_datagram_eq("start_particle count clamp");
    assert_eq!(sv_datagram_bytes(SIDE_C)[10], 255);
}

#[test]
fn start_particle_clamps_direction_to_a_signed_byte() {
    let _g = lock();
    reset(PROTOCOL_FITZQUAKE, 0, 2, false);
    let (c, r) = drive_particle([0.0; 3], [1000.0, -1000.0, 7.9375], 2, 3);
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_sv_datagram_eq("start_particle dir clamp");
    let b = sv_datagram_bytes(SIDE_C);
    assert_eq!(b[7], 127, "clamped high");
    assert_eq!(b[8], 128, "clamped low (-128 as a byte)");
    assert_eq!(b[9], 127, "7.9375*16 == 127 exactly, not clamped");
}

#[test]
fn start_particle_uses_rmq_coordinate_flags() {
    let _g = lock();
    // PRFL_FLOATCOORD widens each coord from 2 to 4 bytes.
    reset(PROTOCOL_RMQ, 1 << 4, 2, false);
    let (c, r) = drive_particle([1.5, 2.5, 3.5], [0.0; 3], 5, 6);
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_sv_datagram_eq("start_particle float coords");
    assert_eq!(sv_datagram_bytes(SIDE_C).len(), 18, "1 + 3*4 + 3 + 1 + 1");
}

#[test]
fn start_particle_is_a_no_op_when_the_datagram_is_nearly_full() {
    let _g = lock();
    reset(PROTOCOL_FITZQUAKE, 0, 2, false);
    // MAX_DATAGRAM is 64000 and a particle costs 12 bytes, so fill until
    // sv_main.c:238's `cursize > maxsize - 18` early-out engages.
    for _ in 0..6000 {
        let (c, r) = drive_particle([0.0; 3], [0.0; 3], 1, 1);
        assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    }
    assert_sv_datagram_eq("start_particle fill");
    let before = sv_datagram_bytes(SIDE_C).len();
    assert!(
        before > 63000,
        "the fill must reach the guard, got {before}"
    );
    let (c, r) = drive_particle([0.0; 3], [0.0; 3], 1, 1);
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_sv_datagram_eq("start_particle overflow guard");
    // The fixture's datagram is 1024 bytes, so 300 * 12 has long since tripped
    // the sv_main.c:238 early-out; the size must have stopped growing.
    assert_eq!(
        sv_datagram_bytes(SIDE_C).len(),
        before,
        "the guard must stop the writer"
    );
}

// ===========================================================================
// SV_StartSound (sv_main.c:277-386)

#[allow(clippy::too_many_arguments)]
fn drive_sound(
    entnum: c_int,
    origin: Option<[c_float; 3]>,
    channel: c_int,
    sample: &str,
    volume: c_int,
    attenuation: c_float,
) -> (c_int, c_int) {
    let s = cstr(sample);
    let org = origin.unwrap_or([0.0; 3]);
    let p = if origin.is_some() {
        org.as_ptr()
    } else {
        core::ptr::null()
    };
    // SAFETY: `s` and `org` outlive both calls; the fixture copies the origin
    // and only borrows `sample` for the duration of the call.
    let c = unsafe {
        ctest_svmain_drive_startsound(SIDE_C, entnum, p, channel, s.as_ptr(), volume, attenuation)
    };
    // SAFETY: same contract as the paired call above.
    let r = unsafe {
        ctest_svmain_drive_startsound(SIDE_RS, entnum, p, channel, s.as_ptr(), volume, attenuation)
    };
    (c, r)
}

/// Two active spawned clients, buffers zeroed, entity 1 at a known origin.
fn sound_fixture(protocol: c_int) {
    reset(protocol, 0, 2, false);
    // SAFETY: leaf setters over fixture-owned arrays, indices in range.
    unsafe {
        ctest_svmain_set_client_flags(0, 1, 1);
        ctest_svmain_set_client_flags(1, 1, 1);
        ctest_svmain_set_edict_origin(1, 10.0, 20.0, 30.0, -16.0, 16.0);
        ctest_svmain_clear_all_bufs();
    }
}

#[test]
fn start_sound_writes_the_same_bytes_to_every_client() {
    let _g = lock();
    sound_fixture(PROTOCOL_FITZQUAKE);
    let (c, r) = drive_sound(1, None, 2, "items/damage.wav", 200, 0.5);
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_all_clients_eq("start_sound");
    let s = snap(SIDE_C, 0);
    assert!(s.datagram_cursize > 0, "client 0 must have received bytes");
    assert_eq!(
        snap(SIDE_C, 2).datagram_cursize,
        0,
        "inactive clients get nothing"
    );
}

#[test]
fn start_sound_default_volume_and_attenuation_clear_the_field_mask() {
    let _g = lock();
    sound_fixture(PROTOCOL_FITZQUAKE);
    // DEFAULT_SOUND_PACKET_VOLUME 255, DEFAULT_SOUND_PACKET_ATTENUATION 1.0
    let (c, r) = drive_sound(1, None, 0, "items/damage.wav", 255, 1.0);
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_all_clients_eq("start_sound default mask");
    // The wire lives in client->datagram_buf, which the snapshot compares; pin
    // the header explicitly from the C side.
    let s = snap(SIDE_C, 0);
    assert_eq!(
        s.datagram_cursize,
        1 + 1 + 2 + 1 + 6,
        "svc+mask+entchan+sound+3 coords"
    );
    let hdr = client_datagram_bytes(SIDE_C, 0, 2);
    assert_eq!(hdr[0], SVC_SOUND);
    assert_eq!(
        hdr[1] & (SND_VOLUME | SND_ATTENUATION),
        0,
        "both default fields are omitted from the mask"
    );
}

/// COMPAT (sv_main.c:349): `MSG_WriteByte (&client->datagram, attenuation *
/// 64)` -- the maximum legal attenuation of 4 gives 256, which the byte writer
/// truncates to 0. Preserved verbatim.
#[test]
fn start_sound_attenuation_4_truncates_to_a_zero_byte() {
    let _g = lock();
    sound_fixture(PROTOCOL_FITZQUAKE);
    let (c, r) = drive_sound(1, None, 0, "items/damage.wav", 255, 4.0);
    assert_eq!(
        (c, r),
        (GUARD_OK, GUARD_OK),
        "attenuation == 4 is legal, not a raise"
    );
    assert_all_clients_eq("start_sound attenuation 4");
    let bytes = client_msg_bytes(SIDE_C, 0, 0); // msgbuf is untouched here
    assert!(bytes.is_empty());
    // byte 1 is the field mask, byte 2 the attenuation (no SND_VOLUME here).
    let s = snap(SIDE_C, 0);
    assert_eq!(s.datagram_cursize, 1 + 1 + 1 + 2 + 1 + 6);
}

#[test]
fn start_sound_volume_over_255_clamps_and_prints() {
    let _g = lock();
    sound_fixture(PROTOCOL_FITZQUAKE);
    let (sc, lc) = drive_logged(|| {
        // SAFETY: the sample string outlives the call.
        let s = cstr("items/damage.wav");
        // SAFETY: same contract as the paired call above.
        unsafe {
            ctest_svmain_drive_startsound(SIDE_C, 1, core::ptr::null(), 0, s.as_ptr(), 300, 1.0)
        }
    });
    let (sr, lr) = drive_logged(|| {
        let s = cstr("items/damage.wav");
        // SAFETY: as above.
        unsafe {
            ctest_svmain_drive_startsound(SIDE_RS, 1, core::ptr::null(), 0, s.as_ptr(), 300, 1.0)
        }
    });
    assert_eq!((sc, sr), (GUARD_OK, GUARD_OK));
    assert_log_eq("start_sound volume clamp", &lc, &lr);
    assert_eq!(lc, ["[con] SV_StartSound: volume = 255\n"]);
    assert_all_clients_eq("start_sound volume clamp");
}

/// COMPAT (sv_main.c:307-311): an unprecached sample is NOT an ADR-009 raise.
/// It prints and returns, and the misspelling "not precacheed" is upstream.
#[test]
fn start_sound_unprecached_sample_prints_not_precacheed_and_returns() {
    let _g = lock();
    sound_fixture(PROTOCOL_FITZQUAKE);
    let (sc, lc) = drive_logged(|| {
        let s = cstr("missing/nope.wav");
        // SAFETY: the sample string outlives the call.
        unsafe {
            ctest_svmain_drive_startsound(SIDE_C, 1, core::ptr::null(), 0, s.as_ptr(), 255, 1.0)
        }
    });
    let (sr, lr) = drive_logged(|| {
        let s = cstr("missing/nope.wav");
        // SAFETY: as above.
        unsafe {
            ctest_svmain_drive_startsound(SIDE_RS, 1, core::ptr::null(), 0, s.as_ptr(), 255, 1.0)
        }
    });
    assert_eq!((sc, sr), (GUARD_OK, GUARD_OK), "must not raise");
    assert_log_eq("start_sound unprecached", &lc, &lr);
    assert_eq!(
        lc,
        ["[con] SV_StartSound: missing/nope.wav not precacheed\n"]
    );
    assert_eq!(snap(SIDE_C, 0).datagram_cursize, 0, "no bytes written");
    assert_all_clients_eq("start_sound unprecached");
}

#[test]
fn start_sound_negative_volume_raises_on_both_sides() {
    let _g = lock();
    sound_fixture(PROTOCOL_FITZQUAKE);
    let (c, r) = drive_sound(1, None, 0, "items/damage.wav", -1, 1.0);
    assert_eq!((c, r), (GUARD_HOST_ERROR, GUARD_HOST_ERROR));
}

#[test]
fn start_sound_out_of_range_attenuation_raises_on_both_sides() {
    let _g = lock();
    sound_fixture(PROTOCOL_FITZQUAKE);
    assert_eq!(
        drive_sound(1, None, 0, "items/damage.wav", 255, -0.5),
        (GUARD_HOST_ERROR, GUARD_HOST_ERROR)
    );
    sound_fixture(PROTOCOL_FITZQUAKE);
    assert_eq!(
        drive_sound(1, None, 0, "items/damage.wav", 255, 4.5),
        (GUARD_HOST_ERROR, GUARD_HOST_ERROR)
    );
}

#[test]
fn start_sound_negative_channel_raises_on_both_sides() {
    let _g = lock();
    sound_fixture(PROTOCOL_FITZQUAKE);
    let (c, r) = drive_sound(1, None, -1, "items/damage.wav", 255, 1.0);
    assert_eq!((c, r), (GUARD_HOST_ERROR, GUARD_HOST_ERROR));
}

#[test]
fn start_sound_channel_over_7_sets_largeentity_without_raising() {
    let _g = lock();
    sound_fixture(PROTOCOL_FITZQUAKE);
    let (c, r) = drive_sound(1, None, 9, "items/damage.wav", 255, 1.0);
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_all_clients_eq("start_sound channel 9");
    // svc + mask + short ent + byte channel + byte sound + 3 coords
    assert_eq!(snap(SIDE_C, 0).datagram_cursize, 1 + 1 + 2 + 1 + 1 + 6);
    let hdr = client_datagram_bytes(SIDE_C, 0, 2);
    assert_eq!(hdr[0], SVC_SOUND);
    assert_eq!(
        hdr[1] & (SND_LARGEENTITY | SND_LARGESOUND),
        SND_LARGEENTITY,
        "channel 9 needs the wide entity/channel field, not a wide sound index"
    );
}

#[test]
fn start_sound_netquake_drops_large_entity_and_large_sound_packets() {
    let _g = lock();
    sound_fixture(PROTOCOL_NETQUAKE);
    let (c, r) = drive_sound(1, None, 9, "items/damage.wav", 255, 1.0);
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_all_clients_eq("start_sound netquake large-entity drop");
    assert_eq!(
        snap(SIDE_C, 0).datagram_cursize,
        0,
        "PROTOCOL_NETQUAKE cannot carry it"
    );
}

#[test]
fn start_sound_skips_clients_whose_limits_exclude_the_entity_or_sound() {
    let _g = lock();
    sound_fixture(PROTOCOL_FITZQUAKE);
    // SAFETY: leaf setters over the fixture's own client arrays.
    unsafe {
        ctest_svmain_set_client_limits(0, 1, 2048); // ent 1 >= limit_entities
        ctest_svmain_set_client_limits(1, 2048, 2); // sound_num 2 >= limit_sounds
    }
    let (c, r) = drive_sound(1, None, 0, "items/damage.wav", 255, 1.0);
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_all_clients_eq("start_sound limits");
    assert_eq!(snap(SIDE_C, 0).datagram_cursize, 0);
    assert_eq!(snap(SIDE_C, 1).datagram_cursize, 0);
}

#[test]
fn start_sound_without_an_origin_uses_the_entity_bbox_centre() {
    let _g = lock();
    sound_fixture(PROTOCOL_FITZQUAKE);
    // asymmetric bbox so origin + 0.5*(mins+maxs) is not just origin
    // SAFETY: leaf setter, edict 1 is inside the fixture's 16-edict arena.
    unsafe { ctest_svmain_set_edict_origin(1, 8.0, 0.0, 0.0, -4.0, 12.0) };
    let (c, r) = drive_sound(1, None, 0, "items/damage.wav", 255, 1.0);
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_all_clients_eq("start_sound bbox centre");
    let explicit = snap(SIDE_C, 0).datagram_cursize;
    sound_fixture(PROTOCOL_FITZQUAKE);
    // SAFETY: as above.
    unsafe { ctest_svmain_set_edict_origin(1, 8.0, 0.0, 0.0, -4.0, 12.0) };
    let (c2, r2) = drive_sound(1, Some([12.0, 4.0, 4.0]), 0, "items/damage.wav", 255, 1.0);
    assert_eq!((c2, r2), (GUARD_OK, GUARD_OK));
    assert_all_clients_eq("start_sound explicit origin");
    assert_eq!(
        snap(SIDE_C, 0).datagram_cursize,
        explicit,
        "same byte count either way"
    );
}

#[test]
fn start_sound_skips_a_client_whose_datagram_is_nearly_full() {
    let _g = lock();
    sound_fixture(PROTOCOL_FITZQUAKE);
    // 11 bytes per sound into a 64000-byte datagram, up to the
    // `cursize > maxsize - 22` guard at sv_main.c:341.
    for _ in 0..6000 {
        let (c, r) = drive_sound(1, None, 0, "items/damage.wav", 255, 1.0);
        assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    }
    assert_all_clients_eq("start_sound datagram fill");
    let before = snap(SIDE_C, 0).datagram_cursize;
    assert!(
        before > 63000,
        "the fill must reach the guard, got {before}"
    );
    let (c, r) = drive_sound(1, None, 0, "items/damage.wav", 255, 1.0);
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_all_clients_eq("start_sound datagram guard");
    assert_eq!(snap(SIDE_C, 0).datagram_cursize, before);
}

#[test]
fn start_sound_selects_the_precache_index_by_name() {
    let _g = lock();
    for (sample, expected_len) in [
        ("weapons/r_exp3.wav", 11),
        ("items/damage.wav", 11),
        ("player/udeath.wav", 11),
    ] {
        sound_fixture(PROTOCOL_FITZQUAKE);
        let (c, r) = drive_sound(1, None, 0, sample, 255, 1.0);
        assert_eq!((c, r), (GUARD_OK, GUARD_OK));
        assert_all_clients_eq(sample);
        assert_eq!(snap(SIDE_C, 0).datagram_cursize, expected_len, "{sample}");
    }
    // and the index itself must match byte-for-byte between the sides, which
    // assert_all_clients_eq already compared.
}

// ===========================================================================
// SV_LocalSound (sv_main.c:388-433)

fn drive_local(clientnum: c_int, sample: &str) -> (c_int, c_int) {
    let s = cstr(sample);
    // SAFETY: `s` outlives both calls.
    let c = unsafe { ctest_svmain_drive_localsound(SIDE_C, clientnum, s.as_ptr()) };
    // SAFETY: same contract as the paired call above.
    let r = unsafe { ctest_svmain_drive_localsound(SIDE_RS, clientnum, s.as_ptr()) };
    (c, r)
}

#[test]
fn local_sound_writes_the_same_client_message() {
    let _g = lock();
    sound_fixture(PROTOCOL_FITZQUAKE);
    let (c, r) = drive_local(0, "items/damage.wav");
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_all_clients_eq("local_sound");
    assert!(snap(SIDE_C, 0).message_cursize > 0);
}

/// Note the spelling: `SV_LocalSound` says "not precached" (sv_main.c:400),
/// while `SV_StartSound` says "not precacheed". Both are upstream; neither is
/// normalized.
#[test]
fn local_sound_unprecached_sample_prints_not_precached() {
    let _g = lock();
    sound_fixture(PROTOCOL_FITZQUAKE);
    let (sc, lc) = drive_logged(|| {
        let s = cstr("missing/nope.wav");
        // SAFETY: `s` outlives the call.
        unsafe { ctest_svmain_drive_localsound(SIDE_C, 0, s.as_ptr()) }
    });
    let (sr, lr) = drive_logged(|| {
        let s = cstr("missing/nope.wav");
        // SAFETY: as above.
        unsafe { ctest_svmain_drive_localsound(SIDE_RS, 0, s.as_ptr()) }
    });
    assert_eq!((sc, sr), (GUARD_OK, GUARD_OK));
    assert_log_eq("local_sound unprecached", &lc, &lr);
    assert_eq!(
        lc,
        ["[con] SV_LocalSound: missing/nope.wav not precached\n"]
    );
    assert_eq!(snap(SIDE_C, 0).message_cursize, 0);
}

#[test]
fn local_sound_respects_the_client_sound_limit() {
    let _g = lock();
    sound_fixture(PROTOCOL_FITZQUAKE);
    // SAFETY: leaf setter, index in range.
    unsafe { ctest_svmain_set_client_limits(0, 2048, 2) };
    let (c, r) = drive_local(0, "items/damage.wav");
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_all_clients_eq("local_sound limits");
}

// ===========================================================================
// SV_SendServerinfo (sv_main.c:435-644)

fn drive_serverinfo(clientnum: c_int) -> (c_int, c_int) {
    // SAFETY: leaf drivers; `clientnum` is validated by the caller against
    // the fixture's CTEST_SVMAIN_CLIENTS.
    let c = unsafe { ctest_svmain_drive_serverinfo(SIDE_C, clientnum) };
    // SAFETY: same contract as the paired call above.
    let r = unsafe { ctest_svmain_drive_serverinfo(SIDE_RS, clientnum) };
    (c, r)
}

fn serverinfo_fixture(protocol: c_int, pextknown: bool, client_pext2: c_uint) {
    reset(protocol, 0, 2, false);
    // SAFETY: leaf setters over fixture-owned storage.
    unsafe {
        ctest_svmain_set_client_flags(0, 1, 1);
        ctest_svmain_set_client_pext(0, client_pext2, c_int::from(pextknown));
        ctest_svmain_clear_all_bufs();
    }
}

/// The unknown-pext arm (sv_main.c:456-462): a `cmd pext` stufftext, a
/// PRESPAWN_FLUSH sendsignon, and an early return before any limits are
/// chosen -- so the safe defaults at :445-450 survive.
#[test]
fn send_serverinfo_requests_pext_when_the_client_is_unknown() {
    let _g = lock();
    serverinfo_fixture(PROTOCOL_RMQ, false, 0);
    let (c, r) = drive_serverinfo(0);
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_client_eq("send_serverinfo unknown pext", 0);
    let s = snap(SIDE_C, 0);
    assert_eq!(s.sendsignon, PRESPAWN_FLUSH);
    assert_eq!(s.spawned, 0);
    assert_eq!(s.limit_unreliable, 1024);
    assert_eq!(s.limit_reliable, 8192);
    assert_eq!(s.limit_entities, 0);
    let bytes = client_msg_bytes(SIDE_C, 0, 12);
    assert_eq!(bytes[0], SVC_STUFFTEXT);
    let text: String = bytes[1..10].iter().map(|&b| b as u8 as char).collect();
    assert_eq!(text, "cmd pext\n");
}

/// The pext-disabled arm (sv_main.c:452-455): the server clears `pextknown`
/// so it retries next map, then falls through to the real handshake.
#[test]
fn send_serverinfo_with_pext_disabled_clears_pextknown_and_proceeds() {
    let _g = lock();
    serverinfo_fixture(PROTOCOL_FITZQUAKE, true, PEXT2_SUPPORTED_SERVER);
    // SAFETY: leaf setter over the two protocol words.
    unsafe { ctest_svmain_set_pext(0, 0) };
    let (c, r) = drive_serverinfo(0);
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_client_eq("send_serverinfo pext disabled", 0);
    let s = snap(SIDE_C, 0);
    assert_eq!(s.pextknown, 0);
    assert_eq!(s.protocol_pext2, 0);
    // sv_main.c:519 then clamps to qcvm->max_edicts == CTEST_SVMAIN_EDICTS.
    assert_eq!(
        s.limit_entities, 16,
        "FITZQUAKE arm, then the max_edicts clamp"
    );
}

#[test]
fn send_serverinfo_limits_match_per_protocol() {
    let _g = lock();
    for (proto, ent, models, sounds) in [
        // `ent` is each protocol's own limit AFTER sv_main.c:519's clamp to
        // qcvm->max_edicts (CTEST_SVMAIN_EDICTS == 16).
        (PROTOCOL_NETQUAKE, 16u32, 256u32, 256u32),
        (PROTOCOL_FITZQUAKE, 16, 2048, 2048),
        (PROTOCOL_RMQ, 16, 2048, 2048),
    ] {
        serverinfo_fixture(proto, true, 0);
        // SAFETY: leaf setter; pext must stay enabled server-side so the
        // pextknown client reaches the protocol switch with pext2 == 0.
        unsafe { ctest_svmain_set_pext(PEXT1_SUPPORTED_SERVER, PEXT2_SUPPORTED_SERVER) };
        let (c, r) = drive_serverinfo(0);
        assert_eq!((c, r), (GUARD_OK, GUARD_OK), "protocol {proto}");
        assert_client_eq("send_serverinfo protocol limits", 0);
        let s = snap(SIDE_C, 0);
        assert_eq!(s.limit_entities, ent, "protocol {proto} limit_entities");
        assert_eq!(s.limit_models, models, "protocol {proto} limit_models");
        assert_eq!(s.limit_sounds, sounds, "protocol {proto} limit_sounds");
        // NET_QSocketGetTrueAddressString is never "LOCAL" here, so the MTU
        // clamp at sv_main.c:512 always applies.
        assert!(
            s.limit_unreliable <= 1400,
            "protocol {proto}: DATAGRAM_MTU clamp"
        );
    }
}

#[test]
fn send_serverinfo_pext_client_gets_the_fte_limits() {
    let _g = lock();
    serverinfo_fixture(PROTOCOL_RMQ, true, PEXT2_SUPPORTED_SERVER);
    let (c, r) = drive_serverinfo(0);
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_client_eq("send_serverinfo fte limits", 0);
    let s = snap(SIDE_C, 0);
    assert_eq!(s.protocol_pext2, PEXT2_SUPPORTED_SERVER);
    assert_eq!(s.limit_reliable, 64000, "NET_MAXMESSAGE");
    assert_eq!(s.limit_models, 8192, "MAX_MODELS");
    assert_eq!(s.limit_sounds, 2048, "MAX_SOUNDS");
    // MAX_EDICTS 32000 exceeds the fixture qcvm's max_edicts, so :519 clamps.
    assert_eq!(s.limit_entities, 16);
}

/// sv_main.c:466 -- PREDINFO without REPLACEMENTDELTAS is dropped, because
/// stats cannot be deltaed without deltas.
#[test]
fn send_serverinfo_drops_predinfo_without_replacement_deltas() {
    let _g = lock();
    serverinfo_fixture(PROTOCOL_RMQ, true, PEXT2_PREDINFO);
    let (c, r) = drive_serverinfo(0);
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_client_eq("send_serverinfo predinfo drop", 0);
    assert_eq!(snap(SIDE_C, 0).protocol_pext2, 0);
}

/// The client can only keep the bits the server still offers (sv_main.c:463).
#[test]
fn send_serverinfo_masks_the_client_pext_against_the_server() {
    let _g = lock();
    serverinfo_fixture(PROTOCOL_RMQ, true, PEXT2_REPLACEMENTDELTAS | PEXT2_PREDINFO);
    // SAFETY: leaf setter.
    unsafe { ctest_svmain_set_pext(PEXT1_SUPPORTED_SERVER, PEXT2_REPLACEMENTDELTAS) };
    let (c, r) = drive_serverinfo(0);
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_client_eq("send_serverinfo pext mask", 0);
    assert_eq!(snap(SIDE_C, 0).protocol_pext2, PEXT2_REPLACEMENTDELTAS);
}

/// The full serverinfo message body. `NET_SendMessage` always succeeds in
/// this harness, so `client->message` is `SZ_Clear`ed on the way out and the
/// bytes are only visible as residue in `msgbuf`; the fixture zeroes both
/// columns beforehand, so a memcmp of the whole array is exact.
#[test]
fn send_serverinfo_writes_the_same_message_bytes() {
    let _g = lock();
    for proto in [PROTOCOL_NETQUAKE, PROTOCOL_FITZQUAKE, PROTOCOL_RMQ] {
        serverinfo_fixture(proto, true, PEXT2_SUPPORTED_SERVER);
        let (c, r) = drive_serverinfo(0);
        assert_eq!((c, r), (GUARD_OK, GUARD_OK), "protocol {proto}");
        assert_client_eq("send_serverinfo bytes", 0);
        assert_eq!(
            client_msg_bytes(SIDE_C, 0, 1)[0],
            SVC_PRINT,
            "protocol {proto}"
        );
        // and prove the residue is non-trivial, so the memcmp is meaningful
        let nonzero = client_msg_bytes(SIDE_C, 0, 256)
            .iter()
            .filter(|&&b| b != 0)
            .count();
        assert!(
            nonzero > 32,
            "protocol {proto}: only {nonzero} non-zero residue bytes"
        );
    }
}

#[test]
fn send_serverinfo_residue_covers_the_whole_msgbuf() {
    let _g = lock();
    serverinfo_fixture(PROTOCOL_RMQ, true, PEXT2_SUPPORTED_SERVER);
    let (c, r) = drive_serverinfo(0);
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    // SAFETY: memcmp over the full MAX_MSGLEN array of both columns.
    let d = unsafe { ctest_svmain_msgbuf_diff(0) };
    assert_eq!(d, -1, "msgbuf diverged at {d} (of {MAX_MSGLEN})");
}

// ===========================================================================
// SV_ConnectClient (sv_main.c:700-761) -- the ADR-009 statusize deliverable

#[test]
fn connect_client_loadgame_preserves_spawn_parms() {
    let _g = lock();
    reset(PROTOCOL_RMQ, 0, 2, true);
    // SAFETY: leaf setters; index and parm slot are in range.
    unsafe {
        for j in 0..NUM_TOTAL_SPAWN_PARMS as c_int {
            ctest_svmain_set_spawn_parm(0, j, 100.0 + j as c_float);
        }
        ctest_svmain_set_client_pext(0, PEXT2_SUPPORTED_SERVER, 1);
        ctest_svmain_clear_all_bufs();
    }
    // SAFETY: leaf drivers.
    let c = unsafe { ctest_svmain_drive_connectclient(SIDE_C, 0) };
    // SAFETY: same contract as the paired call above.
    let r = unsafe { ctest_svmain_drive_connectclient(SIDE_RS, 0) };
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_client_eq("connect_client loadgame", 0);
    let s = snap(SIDE_C, 0);
    assert_eq!(s.active, 1);
    assert_eq!(s.spawned, 0);
    assert_eq!(s.edictnum, 1, "clientnum + 1");
    for j in 0..NUM_TOTAL_SPAWN_PARMS {
        assert_eq!(
            s.spawn_parms[j],
            100.0 + j as c_float,
            "parm {j} survived the memset"
        );
    }
    let name: String = s
        .name
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as u8 as char)
        .collect();
    assert_eq!(name, "unconnected");
}

/// `SV_ConnectClient` clears `pextknown`, so the tail call to
/// `SV_SendServerinfo` always takes the `cmd pext` stufftext arm here.
#[test]
fn connect_client_always_requests_pext_again() {
    let _g = lock();
    reset(PROTOCOL_RMQ, 0, 2, true);
    // SAFETY: leaf setters.
    unsafe {
        ctest_svmain_set_client_pext(0, PEXT2_SUPPORTED_SERVER, 1);
        ctest_svmain_clear_all_bufs();
    }
    // SAFETY: leaf drivers.
    let c = unsafe { ctest_svmain_drive_connectclient(SIDE_C, 0) };
    // SAFETY: same contract as the paired call above.
    let r = unsafe { ctest_svmain_drive_connectclient(SIDE_RS, 0) };
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_client_eq("connect_client pext reset", 0);
    let s = snap(SIDE_C, 0);
    assert_eq!(s.pextknown, 0);
    assert_eq!(s.protocol_pext2, 0);
    assert_eq!(s.sendsignon, PRESPAWN_FLUSH);
    assert_eq!(client_msg_bytes(SIDE_C, 0, 1)[0], SVC_STUFFTEXT);
}

/// The status deliverable. The non-loadgame arm calls
/// `PR_ExecuteProgram (pr_global_struct->SetNewParms)`, which the fixture
/// leaves at 0 -- a NULL function, which `PR_ExecuteProgram` turns into a
/// `Host_Error`. Both sides must report the SAME `Host_Guard` status rather
/// than one of them unwinding through a Rust frame.
#[test]
fn connect_client_propagates_the_setnewparms_raise_identically() {
    let _g = lock();
    reset(PROTOCOL_RMQ, 0, 2, false);
    // SAFETY: leaf setters.
    unsafe { ctest_svmain_clear_all_bufs() };
    // SAFETY: leaf drivers; a raise inside is caught by the side's own guard.
    let c = unsafe { ctest_svmain_drive_connectclient(SIDE_C, 0) };
    // SAFETY: same contract as the paired call above.
    let r = unsafe { ctest_svmain_drive_connectclient(SIDE_RS, 0) };
    assert_eq!(c, r, "SV_ConnectClient status diverged");
    assert!(
        c == GUARD_OK || c == GUARD_HOST_ERROR,
        "unexpected status {c}"
    );
}

// ===========================================================================
// SV_CheckForNewClients (sv_main.c:763-790)

#[test]
fn check_for_new_clients_is_a_no_op_without_connections() {
    let _g = lock();
    reset(PROTOCOL_RMQ, 0, 2, false);
    // SAFETY: leaf drivers. NET_CheckNewConnections is the inert stub, so the
    // loop exits immediately and the Sys_Error arm at :784 is unreachable --
    // which is deliberate, because Sys_Error longjmps in this harness.
    let c = unsafe { ctest_svmain_drive_checknewclients(SIDE_C) };
    // SAFETY: same contract as the paired call above.
    let r = unsafe { ctest_svmain_drive_checknewclients(SIDE_RS) };
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_all_clients_eq("check_for_new_clients");
}

// ===========================================================================
// SV_ClearDatagram (sv_main.c:805-808)

#[test]
fn clear_datagram_clears_both_sides() {
    let _g = lock();
    reset(PROTOCOL_FITZQUAKE, 0, 2, false);
    let (c, r) = drive_particle([1.0, 2.0, 3.0], [0.0; 3], 4, 5);
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert!(sv_datagram_bytes(SIDE_C).len() > 1);
    // SAFETY: leaf drivers with no raise path.
    unsafe {
        ctest_svmain_drive_cleardatagram(SIDE_C);
        ctest_svmain_drive_cleardatagram(SIDE_RS);
    }
    assert_sv_datagram_eq("clear_datagram");
    assert!(sv_datagram_bytes(SIDE_C).is_empty());
}

// ===========================================================================
// SV_ModelIndex / SV_ModelForIndex (sv_main.c:824-845, :872-884)

fn model_index(name: Option<&str>) -> (c_int, c_int) {
    match name {
        None => {
            // SAFETY: the C accepts NULL and returns 0 at sv_main.c:828.
            let c = unsafe { ctest_svmain_drive_modelindex(SIDE_C, core::ptr::null()) };
            // SAFETY: same contract as the paired call above.
            let r = unsafe { ctest_svmain_drive_modelindex(SIDE_RS, core::ptr::null()) };
            (c, r)
        }
        Some(n) => {
            let s = cstr(n);
            // SAFETY: `s` outlives both calls.
            let c = unsafe { ctest_svmain_drive_modelindex(SIDE_C, s.as_ptr()) };
            // SAFETY: same contract as the paired call above.
            let r = unsafe { ctest_svmain_drive_modelindex(SIDE_RS, s.as_ptr()) };
            (c, r)
        }
    }
}

#[test]
fn model_index_finds_every_precached_name() {
    let _g = lock();
    reset(PROTOCOL_RMQ, 0, 2, false);
    for (name, expected) in [
        ("maps/ctest.bsp", 0),
        ("progs/player.mdl", 1),
        ("progs/eyes.mdl", 2),
        ("*1", 3),
    ] {
        let (c, r) = model_index(Some(name));
        assert_eq!(c, r, "{name}: sides diverged");
        assert_eq!(c, expected, "{name}");
    }
}

#[test]
fn model_index_returns_zero_for_an_empty_or_null_name() {
    let _g = lock();
    reset(PROTOCOL_RMQ, 0, 2, false);
    assert_eq!(model_index(Some("")), (0, 0));
    assert_eq!(model_index(None), (0, 0));
}

#[test]
fn model_for_index_returns_the_same_slot() {
    let _g = lock();
    reset(PROTOCOL_RMQ, 0, 2, false);
    for (i, marker) in [(0, 0x1000), (1, 0x2000), (7, 0x7000)] {
        // SAFETY: leaf setter, index inside MAX_MODELS.
        unsafe { ctest_svmain_set_model_slot(i, marker) };
    }
    for i in [0, 1, 7] {
        // SAFETY: leaf accessor; the raw pointer is never dereferenced.
        let (c, r) = unsafe {
            (
                ctest_svmain_modelforindex_raw(SIDE_C, i),
                ctest_svmain_modelforindex_raw(SIDE_RS, i),
            )
        };
        assert_eq!(c, r, "index {i}");
        assert_ne!(c, 0, "index {i} was seeded");
    }
    // out-of-range and negative indices both fall through to NULL
    for i in [-1, 8192, 99999] {
        // SAFETY: as above.
        let (c, r) = unsafe {
            (
                ctest_svmain_modelforindex_raw(SIDE_C, i),
                ctest_svmain_modelforindex_raw(SIDE_RS, i),
            )
        };
        assert_eq!(c, r, "index {i}");
        assert_eq!(c, 0, "index {i} must be NULL");
    }
}

// ===========================================================================
// SV_SaveSpawnparms (sv_main.c:847-870)

#[test]
fn save_spawnparms_copies_serverflags_with_no_active_clients() {
    let _g = lock();
    reset(PROTOCOL_RMQ, 0, 0, false);
    // SAFETY: leaf setter; writes pr_global_struct->parm1..parm16.
    unsafe { ctest_svmain_set_global_parms(7.0) };
    // SAFETY: leaf drivers; svs.maxclients == 0 keeps the loop body unentered.
    let c = unsafe { ctest_svmain_drive_savespawnparms(SIDE_C) };
    // SAFETY: same contract as the paired call above.
    let r = unsafe { ctest_svmain_drive_savespawnparms(SIDE_RS) };
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    // SAFETY: leaf accessors over svs/c_ref_svs.
    assert_eq!(unsafe { ctest_svmain_serverflags(SIDE_C) }, unsafe {
        ctest_svmain_serverflags(SIDE_RS)
    });
}

/// With an active client the loop reaches
/// `PR_ExecuteProgram (pr_global_struct->SetChangeParms)`, which is 0 in this
/// fixture. The gate is that BOTH sides agree on the resulting status and on
/// the client state left behind -- not that it succeeds.
#[test]
fn save_spawnparms_agrees_on_the_setchangeparms_outcome() {
    let _g = lock();
    reset(PROTOCOL_RMQ, 0, 2, false);
    // SAFETY: leaf setters.
    unsafe {
        ctest_svmain_set_client_flags(0, 1, 1);
        ctest_svmain_set_global_parms(3.0);
    }
    // SAFETY: leaf drivers; a raise is caught by the side's own guard.
    let c = unsafe { ctest_svmain_drive_savespawnparms(SIDE_C) };
    // SAFETY: same contract as the paired call above.
    let r = unsafe { ctest_svmain_drive_savespawnparms(SIDE_RS) };
    assert_eq!(c, r, "SV_SaveSpawnparms status diverged");
    assert_client_eq("save_spawnparms", 0);
}

// ===========================================================================
// SV_Protocol_f (sv_main.c:49-135)

fn drive_protocol(cmd: &str) -> (Vec<String>, Vec<String>) {
    let s = cstr(cmd);
    let (_, lc) = drive_logged(|| {
        // SAFETY: `s` outlives the call; the oracle side runs under Host_Guard.
        unsafe { ctest_svmain_drive_protocol_f(SIDE_C, s.as_ptr()) }
    });
    let (_, lr) = drive_logged(|| {
        // SAFETY: as above.
        unsafe { ctest_svmain_drive_protocol_f(SIDE_RS, s.as_ptr()) }
    });
    (lc, lr)
}

/// `SV_Protocol_f` is `static`, so the oracle can only be reached through the
/// command table `SV_Init` filled in; `init_pair` guarantees it is there.
fn protocol_fixture() {
    init_pair();
    reset(PROTOCOL_RMQ, 0, 2, false);
}

fn protocol_state() -> ((c_int, c_uint, c_uint), (c_int, c_uint, c_uint)) {
    // SAFETY: leaf accessors over the two protocol triples.
    unsafe {
        (
            (
                ctest_svmain_protocol(SIDE_C),
                ctest_svmain_protocol_pext1(SIDE_C),
                ctest_svmain_protocol_pext2(SIDE_C),
            ),
            (
                ctest_svmain_protocol(SIDE_RS),
                ctest_svmain_protocol_pext1(SIDE_RS),
                ctest_svmain_protocol_pext2(SIDE_RS),
            ),
        )
    }
}

#[test]
fn protocol_f_with_no_argument_reports_the_current_protocol() {
    let _g = lock();
    protocol_fixture();
    let (lc, lr) = drive_protocol("sv_protocol");
    assert_log_eq("sv_protocol", &lc, &lr);
    assert_eq!(lc, ["[con] \"sv_protocol\" is \"fte999\"\n"]);
    let (c, r) = protocol_state();
    assert_eq!(c, r);
}

#[test]
fn protocol_f_negotiates_every_prefix_form() {
    let _g = lock();
    let full = (PEXT1_SUPPORTED_SERVER, PEXT2_SUPPORTED_SERVER);
    for (cmd, want_proto, want_pext) in [
        ("sv_protocol FTE+666", PROTOCOL_FITZQUAKE, full),
        ("sv_protocol fte-666", PROTOCOL_FITZQUAKE, full),
        ("sv_protocol +15", PROTOCOL_NETQUAKE, full),
        ("sv_protocol Base+999", PROTOCOL_RMQ, (0, 0)),
        ("sv_protocol base-15", PROTOCOL_NETQUAKE, (0, 0)),
        ("sv_protocol -666", PROTOCOL_FITZQUAKE, (0, 0)),
        ("sv_protocol 999", PROTOCOL_RMQ, full),
    ] {
        protocol_fixture();
        let (lc, lr) = drive_protocol(cmd);
        assert_log_eq(cmd, &lc, &lr);
        let (c, r) = protocol_state();
        assert_eq!(c, r, "{cmd}: protocol state diverged");
        assert_eq!(c, (want_proto, want_pext.0, want_pext.1), "{cmd}");
    }
}

/// `strtol` consumes the digits and leaves `s` on the trailing `-`, so a
/// SUFFIX minus also clears both pext words (sv_main.c:96-101).
#[test]
fn protocol_f_trailing_minus_disables_pext() {
    let _g = lock();
    protocol_fixture();
    let (lc, lr) = drive_protocol("sv_protocol 999-");
    assert_log_eq("sv_protocol 999-", &lc, &lr);
    let (c, r) = protocol_state();
    assert_eq!(c, r);
    assert_eq!(c, (PROTOCOL_RMQ, 0, 0));
}

#[test]
fn protocol_f_rejects_an_unknown_protocol_number() {
    let _g = lock();
    protocol_fixture();
    let before = protocol_state();
    let (lc, lr) = drive_protocol("sv_protocol 42");
    assert_log_eq("sv_protocol 42", &lc, &lr);
    assert_eq!(lc.len(), 1);
    assert!(
        lc[0].starts_with("[con] sv_protocol must be 15 or 666 or 999."),
        "got {:?}",
        lc[0]
    );
    assert_eq!(
        protocol_state(),
        before,
        "a rejected protocol must change nothing"
    );
}

#[test]
fn protocol_f_reports_an_already_active_protocol() {
    let _g = lock();
    protocol_fixture();
    // sv.active is true and the fixture starts on RMQ with full pext, which is
    // exactly what "FTE+999" asks for.
    let (lc, lr) = drive_protocol("sv_protocol FTE+999");
    assert_log_eq("sv_protocol FTE+999", &lc, &lr);
    assert_eq!(lc, ["[con] specified protocol already active.\n"]);
}

#[test]
fn protocol_f_warns_that_a_change_needs_a_map_load() {
    let _g = lock();
    protocol_fixture();
    let (lc, lr) = drive_protocol("sv_protocol FTE+666");
    assert_log_eq("sv_protocol FTE+666", &lc, &lr);
    assert_eq!(
        lc,
        ["[con] changes will not take effect until the next level load.\n"]
    );
}

// ===========================================================================
// SV_Pext_f (sv_main.c:646-698)
//
// These compare client state only, never the console. The oracle side reaches
// SV_Pext_f through Cmd_ExecuteString, whose src_client arm at cmd.c:906-916
// prints an extra Con_DPrintf ("%s tried to %s") and then calls the handler
// anyway; that is upstream behaviour of the dispatcher, not of sv_main.c.

#[test]
fn pext_f_from_a_client_records_the_negotiated_extensions() {
    let _g = lock();
    init_pair();
    reset(PROTOCOL_RMQ, 0, 2, false);
    // SAFETY: leaf setters.
    unsafe {
        ctest_svmain_set_client_flags(0, 1, 0);
        ctest_svmain_set_client_pext(0, 0, 0);
        ctest_svmain_clear_all_bufs();
    }
    // PROTOCOL_FTE_PEXT2 is ('F'|'T'<<8|'E'<<16|'2'<<24) == 0x32455446.
    let cmd = cstr("pext 843404358 40");
    // SAFETY: `cmd` outlives both calls.
    let c = unsafe { ctest_svmain_drive_pext_f(SIDE_C, cmd.as_ptr(), 1, 0) };
    // SAFETY: same contract as the paired call above.
    let r = unsafe { ctest_svmain_drive_pext_f(SIDE_RS, cmd.as_ptr(), 1, 0) };
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_client_eq("pext_f", 0);
    let s = snap(SIDE_C, 0);
    assert_eq!(s.pextknown, 1, "pextknown is latched");
    assert_eq!(
        s.protocol_pext2, PEXT2_SUPPORTED_SERVER,
        "0x28 masked by PEXT2_SUPPORTED_SERVER"
    );
    assert!(s.limit_entities > 0, "the tail call ran SV_SendServerinfo");
}

#[test]
fn pext_f_ignores_unknown_keys() {
    let _g = lock();
    init_pair();
    reset(PROTOCOL_RMQ, 0, 2, false);
    // SAFETY: leaf setters.
    unsafe {
        ctest_svmain_set_client_flags(0, 1, 0);
        ctest_svmain_set_client_pext(0, 0, 0);
        ctest_svmain_clear_all_bufs();
    }
    let cmd = cstr("pext 12345 999");
    // SAFETY: `cmd` outlives both calls.
    let c = unsafe { ctest_svmain_drive_pext_f(SIDE_C, cmd.as_ptr(), 1, 0) };
    // SAFETY: same contract as the paired call above.
    let r = unsafe { ctest_svmain_drive_pext_f(SIDE_RS, cmd.as_ptr(), 1, 0) };
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_client_eq("pext_f unknown key", 0);
    let s = snap(SIDE_C, 0);
    assert_eq!(s.pextknown, 1);
    assert_eq!(s.protocol_pext2, 0);
}

/// The guard at sv_main.c:672: an already-known or already-spawned client is
/// left completely alone, so a second `cmd pext` cannot re-run the handshake.
#[test]
fn pext_f_is_inert_for_a_client_that_already_answered() {
    let _g = lock();
    init_pair();
    reset(PROTOCOL_RMQ, 0, 2, false);
    // SAFETY: leaf setters.
    unsafe {
        ctest_svmain_set_client_flags(0, 1, 0);
        ctest_svmain_set_client_pext(0, PEXT2_REPLACEMENTDELTAS, 1);
        ctest_svmain_clear_all_bufs();
    }
    let before = snap(SIDE_C, 0);
    let cmd = cstr("pext 843404358 40");
    // SAFETY: `cmd` outlives both calls.
    let c = unsafe { ctest_svmain_drive_pext_f(SIDE_C, cmd.as_ptr(), 1, 0) };
    // SAFETY: same contract as the paired call above.
    let r = unsafe { ctest_svmain_drive_pext_f(SIDE_RS, cmd.as_ptr(), 1, 0) };
    assert_eq!((c, r), (GUARD_OK, GUARD_OK));
    assert_client_eq("pext_f inert", 0);
    assert_eq!(snap(SIDE_C, 0), before, "nothing may change");
}

#[test]
fn pext_f_from_the_console_prints_the_client_side_report() {
    let _g = lock();
    init_pair();
    reset(PROTOCOL_RMQ, 0, 2, false);
    let cmd = cstr("pext");
    let (_, lc) = drive_logged(|| {
        // SAFETY: `cmd` outlives the call.
        unsafe { ctest_svmain_drive_pext_f(SIDE_C, cmd.as_ptr(), 0, 0) }
    });
    let (_, lr) = drive_logged(|| {
        // SAFETY: as above.
        unsafe { ctest_svmain_drive_pext_f(SIDE_RS, cmd.as_ptr(), 0, 0) }
    });
    assert_log_eq("pext from console", &lc, &lr);
    assert!(!lc.is_empty(), "the console arm must print something");
}
