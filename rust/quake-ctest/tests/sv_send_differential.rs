//! Differential/characterization gate for `Quake/sv_send.c` -- the FTE
//! replacement-deltas writer, the classic `SV_WriteEntitiesToClient` writer,
//! the stat encoder and the fat-PVS group. Rust migration Phase 7, M6
//! (task T6.2; the port itself lands in T6.3).
//!
//! The oracle side lives in `stubs/sv_send_ref.c`: a self-contained server
//! fixture (qcvm + edict arena, a synthetic two-plane worldmodel, a client
//! array published as `svs.clients`, a strings blob, an `sv` carrying the
//! protocol state, and a `sizebuf_t` the writers emit into) plus one
//! `ctest_svsend_drive_*_c` entry per subject that performs exactly one
//! oracle call.
//!
//! `diff()` is the seam: it runs the SAME fixture through every member of
//! `SIDES`, captures a full observable record per side, asserts the records
//! pairwise-equal, and hands the first back to the test, which then pins it
//! against explicit expected values. `SIDES` is `[Side::C, Side::Rust]` as of
//! T6.3, so every test below is both a differential test against
//! `Quake/sv_send.c` and a characterization test against explicit bytes. The
//! oracle drivers are `ctest_svsend_drive_*_c` and the port's are
//! `ctest_svsend_drive_*_rs`, both in `stubs/sv_send_ref.c`.
//!
//! Fixtures are HAND-CONSTRUCTED, not recorded from a live server stream via
//! `scripts/harness/record_diff.py`. The subjects here are pure encoders over
//! `entity_state_t` / `entvars_t` / `client_t`; a recorded stream would pin
//! one arbitrary combination of field values at the cost of a full capture
//! pipeline, whereas hand-built states reach each field bit, each large-index
//! escape and each ADR-010 rounding boundary directly.
//!
//! ADR-010: every quantizer these writers apply -- `Q_rint` coords and both
//! angle widths, `ENTALPHA_ENCODE`, the truncating `ENTSCALE_ENCODE`, the
//! truncating colormod scale, the lerp and lerpfinish millisecond
//! conversions, and `SV_WriteEntitiesToClient`'s 0.1 origin epsilon -- is
//! exercised at an exact boundary AND at its neighbouring float, not just at
//! round numbers.
//!
//! Four observability limits are forced by `stubs/stubs.c`, which task T6.2
//! may not edit; each is reported, and each is pinned where it is visible:
//!  - `NET_SendUnreliableMessage` discards the packet rather than delivering
//!    it. Since the M6 wave `stubs.c` also *records* every send, so the
//!    bytes `SVFTE_WriteStats` and `SVFTE_WriteEntitiesToClient` emit through
//!    `SV_SendClientDatagram` ARE compared byte-for-byte between the sides
//!    (`NetRec`, folded into `Snap`), alongside their side effects and
//!    `dev_stats.packetsize`. What remains unobservable is only what a real
//!    netchan would do with the packet afterwards.
//!  - `NET_QSocketGetSequenceOut` is a constant 0, so frames are seeded
//!    directly instead of by varying the outgoing sequence.
//!  - `NET_QSocketGetTrueAddressString` never returns `"LOCAL"`, so
//!    `MSGFTE_WriteEntityUpdate`'s LERP_BANDAID block always strips
//!    `UF_UNUSED2` (pinned by `fte_baseline_lerp_bit_is_stripped`).
//!  - `Mod_LeafPVS` returns one buffer for every leaf, so `SV_AddToFatPVS`'
//!    cross-leaf OR is idempotent here; the fixture instead picks `numleafs`
//!    so `fatbytes` is not a multiple of 4, turning sv_send.c:1060's
//!    `i < fatbytes - 3` truncation into an exact byte gate.

use core::ffi::{c_char, c_float, c_int, c_uint, CStr};
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

// ---------------------------------------------------------------------------
// protocol.h / quakedef.h constants, transcribed.

const PEXT2_REPLACEMENTDELTAS: u32 = 0x0000_0008;
const PEXT2_PREDINFO: u32 = 0x0000_0020;
/// `client->protocol_pext2` with replacement deltas but no prediction info.
const RD: u32 = PEXT2_REPLACEMENTDELTAS;
/// ... and with both, which is what QSS clients negotiate.
const RDP: u32 = PEXT2_REPLACEMENTDELTAS | PEXT2_PREDINFO;

const PROTOCOL_NETQUAKE: u32 = 15;
const PROTOCOL_FITZQUAKE: u32 = 666;
const PROTOCOL_RMQ: u32 = 999;

const PRFL_SHORTANGLE: u32 = 1 << 1;
const PRFL_FLOATCOORD: u32 = 1 << 4;
const PRFL_INT32COORD: u32 = 1 << 7;

// UF_* delta bits, as `MSGFTE_DeltaCalcBits` records them in
// `client->pendingentities_bits`. UF_REMOVE/UF_RESET2 are the server-side
// aliases declared at sv_send.c:146-151.
const UF_FRAME: u32 = 1 << 0;
const UF_ORIGINXY: u32 = 1 << 1;
const UF_ORIGINZ: u32 = 1 << 2;
const UF_ANGLESXZ: u32 = 1 << 3;
const UF_ANGLESY: u32 = 1 << 4;
const UF_EFFECTS: u32 = 1 << 5;
const UF_PREDINFO: u32 = 1 << 6;
const UF_RESET2: u32 = 1 << 7;
const UF_RESET: u32 = 1 << 8;
const UF_REMOVE: u32 = 1 << 9;
const UF_MODEL: u32 = 1 << 10;
const UF_SKIN: u32 = 1 << 11;
const UF_COLORMAP: u32 = 1 << 12;
const UF_FLAGS: u32 = 1 << 14;
const UF_ALPHA: u32 = 1 << 16;
const UF_SCALE: u32 = 1 << 17;
const UF_TAGINFO: u32 = 1 << 20;
const UF_TRAILEFFECT: u32 = 1 << 22;
const UF_COLORMOD: u32 = 1 << 24;
const UF_UNUSED2: u32 = 1 << 30;

const SVC_SPAWNSTATIC: u8 = 20;
const SVCFTE_SPAWNSTATIC2: u8 = 21;
const SVC_SPAWNBASELINE: u8 = 22;
const SVC_SPAWNBASELINE2: u8 = 42;
const SVC_SPAWNSTATIC2: u8 = 43;
const SVCFTE_SPAWNBASELINE2: u8 = 66;

const ENTSCALE_DEFAULT: i32 = 16;
const NUM_PING_TIMES: usize = 16;
/// `entity_state_t` flattened by `ctest_svsend_pack`.
const STATEWORDS: usize = 27;
/// How many stat slots the snapshot records; well past every stat sv_send.c
/// touches (the highest is STAT_PUNCHANGLE_Z = 28).
const STATS_CAPTURED: c_int = 40;

// `etype_t` (pr_comp.h), for `sv.customstats[].type`.
const EV_T_STRING: c_int = 1;
const EV_T_FLOAT: c_int = 2;
const EV_T_VECTOR: c_int = 3;
const EV_T_ENTITY: c_int = 4;
const EV_T_EXT_INTEGER: c_int = 8;
const EV_T_EXT_SINT64: c_int = 10;
const EV_T_EXT_DOUBLE: c_int = 12;

// `stat_t` (quakedef.h:112).
const STAT_HEALTH: usize = 0;
const STAT_WEAPON: usize = 2;
const STAT_AMMO: usize = 3;
const STAT_ARMOR: usize = 4;
const STAT_WEAPONFRAME: usize = 5;
const STAT_SHELLS: usize = 6;
const STAT_NAILS: usize = 7;
const STAT_ROCKETS: usize = 8;
const STAT_CELLS: usize = 9;
const STAT_ACTIVEWEAPON: usize = 10;
const STAT_ITEMS: usize = 15;
const STAT_VIEWHEIGHT: usize = 16;
const STAT_VIEWZOOM: usize = 21;
const STAT_IDEALPITCH: usize = 25;

// `ctest_svsend_set_ev` selectors, mirroring sv_send_ref.c's enum.
const EV_MOVETYPE: c_int = 0;
const EV_MODELINDEX: c_int = 1;
const EV_FRAME: c_int = 2;
const EV_COLORMAP: c_int = 3;
const EV_SKIN: c_int = 4;
const EV_EFFECTS: c_int = 5;
const EV_FLAGS: c_int = 6;
const EV_HEALTH: c_int = 7;
const EV_CURRENTAMMO: c_int = 8;
const EV_ARMORVALUE: c_int = 9;
const EV_WEAPONFRAME: c_int = 10;
const EV_AMMO_SHELLS: c_int = 11;
const EV_AMMO_NAILS: c_int = 12;
const EV_AMMO_ROCKETS: c_int = 13;
const EV_AMMO_CELLS: c_int = 14;
const EV_WEAPON: c_int = 15;
const EV_ITEMS: c_int = 16;
const EV_IDEALPITCH: c_int = 17;
const EV_NEXTTHINK: c_int = 18;
const EV_MODEL: c_int = 20;
const EV_WEAPONMODEL: c_int = 21;

// `ctest_svsend_set_evv` selectors.
const EVV_ORIGIN: c_int = 0;
const EVV_ANGLES: c_int = 1;
const EVV_VELOCITY: c_int = 2;
const EVV_VIEW_OFS: c_int = 3;
const EVV_PUNCHANGLE: c_int = 4;

// `ctest_svsend_set_ext` / `ctest_svsend_set_extfields` selectors.
const XF_ALPHA: c_int = 0;
const XF_SCALE: c_int = 1;
const XF_COLORMOD: c_int = 2;
const XF_TAG_ENTITY: c_int = 3;
const XF_TAG_INDEX: c_int = 4;
const XF_MODELFLAGS: c_int = 5;
const XF_TRAILEFFECTNUM: c_int = 6;
const XF_EMITEFFECTNUM: c_int = 7;
const XF_ITEMS2: c_int = 8;
const XF_VIEWZOOM: c_int = 9;
const XF_NODRAWTOCLIENT: c_int = 10;
const XF_DRAWONLYTOCLIENT: c_int = 11;

fn xf(which: c_int) -> u32 {
    1u32 << which
}

const MOVETYPE_STEP: f32 = 4.0;
const FL_ONGROUND: f32 = 512.0;

extern "C" {
    fn ctest_svsend_reset(num_edicts: c_int, maxedicts: c_int);
    fn ctest_svsend_set_globals(serverflags: c_float);
    fn ctest_svsend_set_cvars(
        netsort: c_float,
        smoothlerps: c_float,
        dedicated: c_int,
        now: f64,
        overflowtime: f64,
    );
    fn ctest_svsend_set_server(protocol: c_uint, protocolflags: c_uint, effectsmask: c_int);
    fn ctest_svsend_set_vm(num_edicts: c_int, time: f64);
    fn ctest_svsend_set_extfields(mask: c_uint);
    fn ctest_svsend_set_ext(num: c_int, which: c_int, x: c_float, y: c_float, z: c_float);
    fn ctest_svsend_set_ext_edict(num: c_int, which: c_int, targetnum: c_int);
    fn ctest_svsend_set_ev(num: c_int, field: c_int, v: c_float);
    fn ctest_svsend_set_evv(num: c_int, field: c_int, x: c_float, y: c_float, z: c_float);
    fn ctest_svsend_set_ed(
        num: c_int,
        alpha: c_int,
        sendinterval: c_int,
        sendinterval_default: c_int,
        lastthink: c_float,
        px: c_float,
        py: c_float,
        pz: c_float,
    );
    fn ctest_svsend_set_leafs(num: c_int, leafs: *const c_int, count: c_int);
    fn ctest_svsend_statewords() -> c_int;
    fn ctest_svsend_set_state(slot: c_int, w: *const c_int);
    fn ctest_svsend_get_state(slot: c_int, w: *mut c_int);
    fn ctest_svsend_set_baseline(num: c_int, w: *const c_int);
    fn ctest_svsend_set_string(slot: c_int, s: *const c_char) -> c_int;
    fn ctest_svsend_precache_model(idx: c_int, slot: c_int);
    fn ctest_svsend_set_leafpvs(bytes: *const u8, count: c_int);
    fn ctest_svsend_set_client(
        cl: c_int,
        spawned: c_int,
        pext2: c_uint,
        limit_entities: c_uint,
        limit_models: c_uint,
        limit_unreliable: c_uint,
        edictnum: c_int,
    );
    fn ctest_svsend_set_lastmovemessage(cl: c_int, v: c_int);
    fn ctest_svsend_set_pending(cl: c_int, entnum: c_uint, bits: c_uint);
    fn ctest_svsend_set_lastack(cl: c_int, seq: c_int);
    fn ctest_svsend_seed_frame(
        cl: c_int,
        slot: c_int,
        sequence: c_int,
        timestamp: c_float,
        rsnum: *const c_uint,
        rsstr: *const c_uint,
        ents: *const c_uint,
        numents: c_int,
    );
    fn ctest_svsend_set_resendstats(cl: c_int, rsnum: *const c_uint, rsstr: *const c_uint);
    fn ctest_svsend_set_evalslot(slot: c_int, w0: c_uint, w1: c_uint, w2: c_uint);
    fn ctest_svsend_add_customstat(idx: c_int, ty: c_int, which_extfield: c_int, eval_slot: c_int);
    fn ctest_svsend_msg_reset(maxsize: c_int);
    fn ctest_svsend_msg_copy(out: *mut u8, max: c_int) -> c_int;
    fn ctest_svsend_pending_copy(cl: c_int, out: *mut c_uint, max: c_int) -> c_int;
    fn ctest_svsend_prev_count(cl: c_int) -> c_int;
    fn ctest_svsend_prev_get(cl: c_int, i: c_int, w: *mut c_int) -> c_int;
    fn ctest_svsend_numframes(cl: c_int) -> c_int;
    fn ctest_svsend_frame_get(
        cl: c_int,
        slot: c_int,
        seq: *mut c_int,
        timestamp_bits: *mut c_uint,
        numents: *mut c_int,
        rsnum: *mut c_uint,
        rsstr: *mut c_uint,
    );
    fn ctest_svsend_frame_ent(
        cl: c_int,
        slot: c_int,
        i: c_int,
        num: *mut c_uint,
        ebits: *mut c_uint,
        csqcbits: *mut c_uint,
    ) -> c_int;
    fn ctest_svsend_client_get(
        cl: c_int,
        lastack: *mut c_int,
        snapshotresume: *mut c_uint,
        numpending: *mut c_uint,
        rsnum: *mut c_uint,
        rsstr: *mut c_uint,
        num_pings: *mut c_int,
        pings: *mut c_uint,
    );
    fn ctest_svsend_oldstats_get(
        cl: c_int,
        idx: c_int,
        si: *mut c_int,
        sf: *mut c_uint,
        ss: *mut *const c_char,
    );
    fn ctest_svsend_stats_get(idx: c_int, si: *mut c_int, sf: *mut c_uint, ss: *mut *const c_char);
    fn ctest_svsend_pvs_copy(out: *mut u8, max: c_int) -> c_int;
    fn ctest_svsend_devstats_get(cur: *mut c_int, peak: *mut c_int);
    fn ctest_svsend_fatbytes() -> c_int;

    fn ctest_svsend_drive_baseline_c(
        idx: c_int,
        slot: c_int,
        pext2: c_uint,
        protocol: c_uint,
        protocolflags: c_uint,
    );
    fn ctest_svsend_drive_buildstate_c(ednum: c_int, slot: c_int);
    fn ctest_svsend_drive_calcstats_c(cl: c_int);
    fn ctest_svsend_drive_setupframes_c(cl: c_int);
    fn ctest_svsend_drive_destroyframes_c(cl: c_int);
    fn ctest_svsend_drive_ack_c(cl: c_int, sequence: c_int);
    fn ctest_svsend_drive_fatpvs_c(x: c_float, y: c_float, z: c_float, want: c_int) -> c_int;
    fn ctest_svsend_drive_addtofatpvs_c(
        x: c_float,
        y: c_float,
        z: c_float,
        nodeidx: c_int,
        want: c_int,
    ) -> c_int;
    fn ctest_svsend_drive_visible_c(clientnum: c_int, testnum: c_int) -> c_int;
    fn ctest_svsend_drive_writeents_c(cl: c_int, overflowsize: c_int);
    fn ctest_svsend_drive_presend_c(cl: c_int);
    fn ctest_svsend_drive_senddatagram_c(cl: c_int) -> c_int;

    fn ctest_svsend_drive_baseline_rs(
        idx: c_int,
        slot: c_int,
        pext2: c_uint,
        protocol: c_uint,
        protocolflags: c_uint,
    );
    fn ctest_svsend_drive_buildstate_rs(ednum: c_int, slot: c_int);
    fn ctest_svsend_drive_calcstats_rs(cl: c_int);
    fn ctest_svsend_drive_setupframes_rs(cl: c_int);
    fn ctest_svsend_drive_destroyframes_rs(cl: c_int);
    fn ctest_svsend_drive_ack_rs(cl: c_int, sequence: c_int);
    fn ctest_svsend_drive_fatpvs_rs(x: c_float, y: c_float, z: c_float, want: c_int) -> c_int;
    fn ctest_svsend_drive_addtofatpvs_rs(
        x: c_float,
        y: c_float,
        z: c_float,
        nodeidx: c_int,
        want: c_int,
    ) -> c_int;
    fn ctest_svsend_drive_visible_rs(clientnum: c_int, testnum: c_int) -> c_int;
    fn ctest_svsend_drive_writeents_rs(cl: c_int, overflowsize: c_int);
    fn ctest_svsend_drive_presend_rs(cl: c_int);
    fn ctest_svsend_drive_senddatagram_rs(cl: c_int) -> c_int;

    fn ctest_clear_con_log();
    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(i: c_int) -> *const c_char;
}

// The `stubs.c` datagram recorder (Phase 7 M6). `NET_SendMessage` and
// `NET_SendUnreliableMessage` discard the packet, so without this the
// `SV_SendClientDatagram` tests could only compare how many bytes a side
// *claimed* to send -- a port emitting the right count of wrong bytes would
// pass. The log is the order, per-call size, reliability flag and full
// payload of every send.
//
// Declared with their plain names on purpose: `stubs.c` is not in
// `build.rs`'s `C_SOURCES`, so `c_ref_prelude.h` never sees it and
// `check_ctest_symbols.sh` does not police it. Both sides record through the
// same log; `diff()` resets it immediately before each side runs.
extern "C" {
    fn ctest_net_send_reset();
    fn ctest_net_send_calls() -> c_int;
    fn ctest_net_send_bytes(len: *mut c_int) -> *const u8;
    fn ctest_net_send_call_len(i: c_int) -> c_int;
    fn ctest_net_send_call_reliable(i: c_int) -> c_int;
    fn ctest_net_send_truncated() -> c_int;
}

// ---------------------------------------------------------------------------
// Side dispatch. T6.3 adds `Rust` to the enum, to SIDES and to each `match`.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    C,
    Rust,
}

const SIDES: [Side; 2] = [Side::C, Side::Rust];

fn drive_baseline(side: Side, idx: c_int, slot: c_int, pext2: u32, proto: u32, pflags: u32) {
    // SAFETY: `apply` published qcvm/sv and reset the message buffer.
    unsafe {
        match side {
            Side::C => ctest_svsend_drive_baseline_c(idx, slot, pext2, proto, pflags),
            Side::Rust => ctest_svsend_drive_baseline_rs(idx, slot, pext2, proto, pflags),
        }
    }
}

fn drive_buildstate(side: Side, ednum: c_int, slot: c_int) {
    // SAFETY: `ednum` is inside the fixture's arena.
    unsafe {
        match side {
            Side::C => ctest_svsend_drive_buildstate_c(ednum, slot),
            Side::Rust => ctest_svsend_drive_buildstate_rs(ednum, slot),
        }
    }
}

fn drive_calcstats(side: Side, cl: c_int) {
    // SAFETY: `apply` gave the client an edict; the stat arrays are MAX_CL_STATS.
    unsafe {
        match side {
            Side::C => ctest_svsend_drive_calcstats_c(cl),
            Side::Rust => ctest_svsend_drive_calcstats_rs(cl),
        }
    }
}

fn drive_setupframes(side: Side, cl: c_int) {
    // SAFETY: plain call over fixture-owned client storage.
    unsafe {
        match side {
            Side::C => ctest_svsend_drive_setupframes_c(cl),
            Side::Rust => ctest_svsend_drive_setupframes_rs(cl),
        }
    }
}

fn drive_destroyframes(side: Side, cl: c_int) {
    // SAFETY: plain call over fixture-owned client storage.
    unsafe {
        match side {
            Side::C => ctest_svsend_drive_destroyframes_c(cl),
            Side::Rust => ctest_svsend_drive_destroyframes_rs(cl),
        }
    }
}

fn drive_ack(side: Side, cl: c_int, sequence: c_int) {
    // SAFETY: plain call; the driver sets host_client first.
    unsafe {
        match side {
            Side::C => ctest_svsend_drive_ack_c(cl, sequence),
            Side::Rust => ctest_svsend_drive_ack_rs(cl, sequence),
        }
    }
}

fn drive_fatpvs(side: Side, org: [f32; 3], want: c_int) -> c_int {
    // SAFETY: `apply` published a worldmodel with nodes and leafs.
    unsafe {
        match side {
            Side::C => ctest_svsend_drive_fatpvs_c(org[0], org[1], org[2], want),
            Side::Rust => ctest_svsend_drive_fatpvs_rs(org[0], org[1], org[2], want),
        }
    }
}

fn drive_addtofatpvs(side: Side, org: [f32; 3], nodeidx: c_int, want: c_int) -> c_int {
    // SAFETY: must follow a `drive_fatpvs`; the driver returns -1 otherwise.
    unsafe {
        match side {
            Side::C => ctest_svsend_drive_addtofatpvs_c(org[0], org[1], org[2], nodeidx, want),
            Side::Rust => ctest_svsend_drive_addtofatpvs_rs(org[0], org[1], org[2], nodeidx, want),
        }
    }
}

fn drive_visible(side: Side, clientnum: c_int, testnum: c_int) -> c_int {
    // SAFETY: both edict numbers are inside the fixture's arena.
    unsafe {
        match side {
            Side::C => ctest_svsend_drive_visible_c(clientnum, testnum),
            Side::Rust => ctest_svsend_drive_visible_rs(clientnum, testnum),
        }
    }
}

fn drive_writeents(side: Side, cl: c_int, overflowsize: c_int) {
    // SAFETY: `apply` reset the message buffer and gave the client an edict.
    unsafe {
        match side {
            Side::C => ctest_svsend_drive_writeents_c(cl, overflowsize),
            Side::Rust => ctest_svsend_drive_writeents_rs(cl, overflowsize),
        }
    }
}

fn drive_presend(side: Side, cl: c_int) {
    // SAFETY: plain call; netconnection/spawned/pext2 are guarded inside.
    unsafe {
        match side {
            Side::C => ctest_svsend_drive_presend_c(cl),
            Side::Rust => ctest_svsend_drive_presend_rs(cl),
        }
    }
}

fn drive_senddatagram(side: Side, cl: c_int) -> c_int {
    // SAFETY: plain call; the netchan stub discards the datagram.
    unsafe {
        match side {
            Side::C => ctest_svsend_drive_senddatagram_c(cl),
            Side::Rust => ctest_svsend_drive_senddatagram_rs(cl),
        }
    }
}

// ---------------------------------------------------------------------------
// entity_state_t, flattened to the 27 words `ctest_svsend_pack` uses.

#[derive(Clone, Copy, PartialEq, Debug)]
struct St {
    origin: [f32; 3],
    angles: [f32; 3],
    modelindex: i32,
    frame: i32,
    effects: i32,
    colormap: i32,
    skin: i32,
    scale: i32,
    pmovetype: i32,
    traileffectnum: i32,
    emiteffectnum: i32,
    velocity: [i32; 3],
    eflags: i32,
    tagindex: i32,
    tagentity: i32,
    colormod: [i32; 3],
    alpha: i32,
    solidsize: i32,
    lerp: i32,
}

impl Default for St {
    /// `nullentitystate` as `ctest_progs_setup_nullstate` builds it: this is
    /// the `from` side of every baseline delta, so it is the right zero.
    fn default() -> Self {
        St {
            origin: [0.0; 3],
            angles: [0.0; 3],
            modelindex: 0,
            frame: 0,
            effects: 0,
            colormap: 0,
            skin: 0,
            scale: ENTSCALE_DEFAULT,
            pmovetype: 0,
            traileffectnum: 0,
            emiteffectnum: 0,
            velocity: [0; 3],
            eflags: 0,
            tagindex: 0,
            tagentity: 0,
            colormod: [32, 32, 32],
            alpha: 0,
            solidsize: 0,
            lerp: 0,
        }
    }
}

impl St {
    fn words(&self) -> [i32; STATEWORDS] {
        let mut w = [0i32; STATEWORDS];
        for i in 0..3 {
            w[i] = self.origin[i].to_bits() as i32;
            w[3 + i] = self.angles[i].to_bits() as i32;
            w[15 + i] = self.velocity[i];
            w[21 + i] = self.colormod[i];
        }
        w[6] = self.modelindex;
        w[7] = self.frame;
        w[8] = self.effects;
        w[9] = self.colormap;
        w[10] = self.skin;
        w[11] = self.scale;
        w[12] = self.pmovetype;
        w[13] = self.traileffectnum;
        w[14] = self.emiteffectnum;
        w[18] = self.eflags;
        w[19] = self.tagindex;
        w[20] = self.tagentity;
        w[24] = self.alpha;
        w[25] = self.solidsize;
        w[26] = self.lerp;
        w
    }

    fn from_words(w: &[i32; STATEWORDS]) -> Self {
        let mut s = St {
            scale: w[11],
            colormod: [w[21], w[22], w[23]],
            ..St::default()
        };
        for i in 0..3 {
            s.origin[i] = f32::from_bits(w[i] as u32);
            s.angles[i] = f32::from_bits(w[3 + i] as u32);
            s.velocity[i] = w[15 + i];
        }
        s.modelindex = w[6];
        s.frame = w[7];
        s.effects = w[8];
        s.colormap = w[9];
        s.skin = w[10];
        s.pmovetype = w[12];
        s.traileffectnum = w[13];
        s.emiteffectnum = w[14];
        s.eflags = w[18];
        s.tagindex = w[19];
        s.tagentity = w[20];
        s.alpha = w[24];
        s.solidsize = w[25];
        s.lerp = w[26];
        s
    }
}

// ---------------------------------------------------------------------------
// Fixture description. Everything a test wants to vary lives here so that the
// same declarative value can be replayed into each side.

#[derive(Clone)]
struct Ed {
    num: c_int,
    alpha: c_int,
    sendinterval: c_int,
    sendinterval_default: c_int,
    lastthink: f32,
    predthink: [f32; 3],
    evs: Vec<(c_int, f32)>,
    evvs: Vec<(c_int, [f32; 3])>,
    exts: Vec<(c_int, [f32; 3])>,
    ext_edicts: Vec<(c_int, c_int)>,
    leafs: Vec<c_int>,
}

fn ed(num: c_int) -> Ed {
    Ed {
        num,
        alpha: 0,
        sendinterval: 0,
        sendinterval_default: 0,
        lastthink: 0.0,
        predthink: [0.0; 3],
        evs: Vec::new(),
        evvs: Vec::new(),
        exts: Vec::new(),
        ext_edicts: Vec::new(),
        leafs: Vec::new(),
    }
}

impl Ed {
    fn ev(mut self, field: c_int, v: f32) -> Self {
        self.evs.push((field, v));
        self
    }
    fn evv(mut self, field: c_int, v: [f32; 3]) -> Self {
        self.evvs.push((field, v));
        self
    }
    fn ext(mut self, which: c_int, v: [f32; 3]) -> Self {
        self.exts.push((which, v));
        self
    }
    fn ext1(self, which: c_int, v: f32) -> Self {
        self.ext(which, [v, 0.0, 0.0])
    }
    fn ext_edict(mut self, which: c_int, target: c_int) -> Self {
        self.ext_edicts.push((which, target));
        self
    }
    fn alpha(mut self, a: c_int) -> Self {
        self.alpha = a;
        self
    }
    fn lerpinfo(mut self, si: c_int, sid: c_int) -> Self {
        self.sendinterval = si;
        self.sendinterval_default = sid;
        self
    }
    fn predthink(mut self, lastthink: f32, pos: [f32; 3]) -> Self {
        self.lastthink = lastthink;
        self.predthink = pos;
        self
    }
    fn leafs(mut self, l: &[c_int]) -> Self {
        self.leafs = l.to_vec();
        self
    }
}

#[derive(Clone)]
struct Seed {
    slot: c_int,
    sequence: c_int,
    timestamp: f32,
    rsnum: [u32; 8],
    rsstr: [u32; 8],
    ents: Vec<(u32, u32)>,
}

#[derive(Clone)]
struct Cl {
    idx: c_int,
    spawned: c_int,
    pext2: u32,
    limit_entities: u32,
    limit_models: u32,
    limit_unreliable: u32,
    edictnum: c_int,
    setup_frames: bool,
    lastmovemessage: c_int,
    lastack: Option<c_int>,
    pending: Vec<(u32, u32)>,
    resendstats: Option<([u32; 8], [u32; 8])>,
    seeds: Vec<Seed>,
}

fn cl(idx: c_int) -> Cl {
    Cl {
        idx,
        spawned: 1,
        pext2: 0,
        limit_entities: 8192,
        limit_models: 2048,
        limit_unreliable: 64000,
        edictnum: idx + 1,
        setup_frames: false,
        lastmovemessage: 0,
        lastack: None,
        pending: Vec::new(),
        resendstats: None,
        seeds: Vec::new(),
    }
}

impl Cl {
    fn pext2(mut self, v: u32) -> Self {
        self.pext2 = v;
        self
    }
    fn frames(mut self) -> Self {
        self.setup_frames = true;
        self
    }
    fn limits(mut self, entities: u32, models: u32, unreliable: u32) -> Self {
        self.limit_entities = entities;
        self.limit_models = models;
        self.limit_unreliable = unreliable;
        self
    }
    fn lastmove(mut self, v: c_int) -> Self {
        self.lastmovemessage = v;
        self
    }
    fn lastack(mut self, v: c_int) -> Self {
        self.lastack = Some(v);
        self
    }
    fn pending(mut self, entnum: u32, bits: u32) -> Self {
        self.pending.push((entnum, bits));
        self
    }
    fn resendstats(mut self, rsnum: [u32; 8], rsstr: [u32; 8]) -> Self {
        self.resendstats = Some((rsnum, rsstr));
        self
    }
    fn seed(mut self, s: Seed) -> Self {
        self.seeds.push(s);
        self
    }
}

#[derive(Clone)]
struct Fx {
    num_edicts: c_int,
    maxedicts: c_int,
    serverflags: f32,
    netsort: f32,
    smoothlerps: f32,
    dedicated: c_int,
    now: f64,
    overflowtime: f64,
    protocol: u32,
    protocolflags: u32,
    effectsmask: c_int,
    time: f64,
    extfields: u32,
    strings: Vec<(c_int, &'static str)>,
    precaches: Vec<(c_int, c_int)>,
    leafpvs: Vec<u8>,
    edicts: Vec<Ed>,
    states: Vec<(c_int, St)>,
    baselines: Vec<(c_int, St)>,
    evalslots: Vec<(c_int, u32, u32, u32)>,
    customstats: Vec<(c_int, c_int, c_int, c_int)>,
    clients: Vec<Cl>,
    msgmax: c_int,
}

fn base() -> Fx {
    Fx {
        num_edicts: 2,
        maxedicts: 8,
        serverflags: 0.0,
        netsort: 0.0,
        smoothlerps: 0.0,
        dedicated: 0,
        now: 0.0,
        overflowtime: 0.0,
        protocol: PROTOCOL_FITZQUAKE,
        protocolflags: 0,
        effectsmask: -1,
        time: 0.0,
        extfields: 0,
        strings: Vec::new(),
        precaches: Vec::new(),
        leafpvs: Vec::new(),
        edicts: Vec::new(),
        states: Vec::new(),
        baselines: Vec::new(),
        evalslots: Vec::new(),
        customstats: Vec::new(),
        clients: Vec::new(),
        msgmax: 0,
    }
}

fn apply(side: Side, fx: &Fx) {
    // SAFETY: every pointer below points at a live local, and every index is
    // inside the oracle's fixed-size fixture arrays (checked by the setters).
    unsafe {
        ctest_svsend_reset(fx.num_edicts, fx.maxedicts);
        ctest_svsend_set_extfields(fx.extfields);
        ctest_svsend_set_server(fx.protocol, fx.protocolflags, fx.effectsmask);
        ctest_svsend_set_globals(fx.serverflags);
        ctest_svsend_set_cvars(
            fx.netsort,
            fx.smoothlerps,
            fx.dedicated,
            fx.now,
            fx.overflowtime,
        );
        ctest_svsend_set_vm(fx.num_edicts, fx.time);

        for (slot, text) in &fx.strings {
            let c = std::ffi::CString::new(*text).unwrap();
            ctest_svsend_set_string(*slot, c.as_ptr());
        }
        for (idx, slot) in &fx.precaches {
            ctest_svsend_precache_model(*idx, *slot);
        }
        if !fx.leafpvs.is_empty() {
            ctest_svsend_set_leafpvs(fx.leafpvs.as_ptr(), fx.leafpvs.len() as c_int);
        }

        for e in &fx.edicts {
            for (f, v) in &e.evs {
                ctest_svsend_set_ev(e.num, *f, *v);
            }
            for (f, v) in &e.evvs {
                ctest_svsend_set_evv(e.num, *f, v[0], v[1], v[2]);
            }
            for (w, v) in &e.exts {
                ctest_svsend_set_ext(e.num, *w, v[0], v[1], v[2]);
            }
            for (w, t) in &e.ext_edicts {
                ctest_svsend_set_ext_edict(e.num, *w, *t);
            }
            ctest_svsend_set_ed(
                e.num,
                e.alpha,
                e.sendinterval,
                e.sendinterval_default,
                e.lastthink,
                e.predthink[0],
                e.predthink[1],
                e.predthink[2],
            );
            if !e.leafs.is_empty() {
                ctest_svsend_set_leafs(e.num, e.leafs.as_ptr(), e.leafs.len() as c_int);
            }
        }

        for (slot, st) in &fx.states {
            ctest_svsend_set_state(*slot, st.words().as_ptr());
        }
        for (num, st) in &fx.baselines {
            ctest_svsend_set_baseline(*num, st.words().as_ptr());
        }
        for (slot, w0, w1, w2) in &fx.evalslots {
            ctest_svsend_set_evalslot(*slot, *w0, *w1, *w2);
        }
        for (idx, ty, which, eslot) in &fx.customstats {
            ctest_svsend_add_customstat(*idx, *ty, *which, *eslot);
        }

        for c in &fx.clients {
            ctest_svsend_set_client(
                c.idx,
                c.spawned,
                c.pext2,
                c.limit_entities,
                c.limit_models,
                c.limit_unreliable,
                c.edictnum,
            );
            if c.setup_frames {
                drive_setupframes(side, c.idx);
            }
            ctest_svsend_set_lastmovemessage(c.idx, c.lastmovemessage);
            if let Some(a) = c.lastack {
                ctest_svsend_set_lastack(c.idx, a);
            }
            for (entnum, bits) in &c.pending {
                ctest_svsend_set_pending(c.idx, *entnum, *bits);
            }
            for s in &c.seeds {
                let ents: Vec<c_uint> = s
                    .ents
                    .iter()
                    .flat_map(|(n, b)| [*n as c_uint, *b as c_uint])
                    .collect();
                ctest_svsend_seed_frame(
                    c.idx,
                    s.slot,
                    s.sequence,
                    s.timestamp,
                    s.rsnum.as_ptr(),
                    s.rsstr.as_ptr(),
                    ents.as_ptr(),
                    s.ents.len() as c_int,
                );
            }
            if let Some((rsnum, rsstr)) = &c.resendstats {
                ctest_svsend_set_resendstats(c.idx, rsnum.as_ptr(), rsstr.as_ptr());
            }
        }

        ctest_svsend_msg_reset(fx.msgmax);
        ctest_clear_con_log();
    }
}

// ---------------------------------------------------------------------------
// Observable record.

#[derive(PartialEq, Debug)]
struct FrameRec {
    slot: c_int,
    seq: c_int,
    ts: u32,
    numents: c_int,
    rsnum: [u32; 8],
    rsstr: [u32; 8],
    ents: Vec<(u32, u32, u32)>,
}

type StatRec = (i32, u32, Option<String>);

#[derive(PartialEq, Debug)]
struct ClientRec {
    numframes: c_int,
    lastack: c_int,
    snapshotresume: u32,
    numpending: u32,
    rsnum: [u32; 8],
    rsstr: [u32; 8],
    num_pings: c_int,
    pings: [u32; NUM_PING_TIMES],
    pending: Vec<u32>,
    prev: Vec<(u32, St)>,
    frames: Vec<FrameRec>,
    oldstats: Vec<StatRec>,
}

/// One side's datagram log, captured from `stubs.c`'s recorder.
///
/// `SV_SendClientDatagram` hands its finished packet to `NET_SendMessage` /
/// `NET_SendUnreliableMessage`, both of which discard it, so `dev_stats`
/// alone pins only the byte COUNT. This pins the bytes.
#[derive(PartialEq, Debug)]
struct NetRec {
    calls: c_int,
    /// `(len, reliable)` per call, in call order. `stubs.c` answers -1 past
    /// the 1024-call retained range; that -1 is recorded verbatim rather than
    /// skipped, so a side that outruns the cap can never compare equal to one
    /// that did not.
    per_call: Vec<(c_int, c_int)>,
    /// Every payload, concatenated in call order.
    bytes: Vec<u8>,
    truncated: c_int,
}

fn capture_net() -> NetRec {
    // SAFETY: pure reads of the recorder's own statics; `len` is a live local
    // and `bytes`/`len` describe one buffer stubs.c owns for the process.
    unsafe {
        let calls = ctest_net_send_calls();
        let per_call = (0..calls)
            .map(|i| (ctest_net_send_call_len(i), ctest_net_send_call_reliable(i)))
            .collect();
        let mut len: c_int = 0;
        let p = ctest_net_send_bytes(&mut len);
        let bytes = std::slice::from_raw_parts(p, len.max(0) as usize).to_vec();
        NetRec {
            calls,
            per_call,
            bytes,
            truncated: ctest_net_send_truncated(),
        }
    }
}

#[derive(PartialEq, Debug)]
struct Snap<T> {
    value: T,
    msg: Vec<u8>,
    states: Vec<St>,
    clients: Vec<ClientRec>,
    stats: Vec<StatRec>,
    pvs: Vec<u8>,
    devstats: (c_int, c_int),
    con: Vec<String>,
    net: NetRec,
}

impl<T> Snap<T> {
    fn client(&self, idx: usize) -> &ClientRec {
        &self.clients[idx]
    }
    fn frame(&self, idx: usize, slot: c_int) -> &FrameRec {
        self.clients[idx]
            .frames
            .iter()
            .find(|f| f.slot == slot)
            .unwrap_or_else(|| panic!("client {idx} has no interesting frame in slot {slot}"))
    }
    fn stati(&self, idx: usize) -> i32 {
        self.stats[idx].0
    }
    fn statf(&self, idx: usize) -> f32 {
        f32::from_bits(self.stats[idx].1)
    }
    fn stats_str(&self, idx: usize) -> Option<&str> {
        self.stats[idx].2.as_deref()
    }
}

fn cstr(p: *const c_char) -> Option<String> {
    if p.is_null() {
        None
    } else {
        // SAFETY: the oracle only ever stores pointers into its own strings blob.
        Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
    }
}

fn read_stat(cl_idx: c_int, idx: c_int, old: bool) -> StatRec {
    let mut si: c_int = 0;
    let mut sf: c_uint = 0;
    let mut ss: *const c_char = std::ptr::null();
    // SAFETY: idx < MAX_CL_STATS, cl_idx < CTEST_SVSEND_CLIENTS.
    unsafe {
        if old {
            ctest_svsend_oldstats_get(cl_idx, idx, &mut si, &mut sf, &mut ss);
        } else {
            ctest_svsend_stats_get(idx, &mut si, &mut sf, &mut ss);
        }
    }
    (si, sf, cstr(ss))
}

fn capture_client(idx: c_int) -> ClientRec {
    let mut lastack: c_int = 0;
    let mut snapshotresume: c_uint = 0;
    let mut numpending: c_uint = 0;
    let mut rsnum = [0u32; 8];
    let mut rsstr = [0u32; 8];
    let mut num_pings: c_int = 0;
    let mut pings = [0u32; NUM_PING_TIMES];
    let mut pending = vec![0u32; 512];
    let mut prev = Vec::new();
    let mut frames = Vec::new();
    let numframes: c_int;

    // SAFETY: all out-params are live locals of at least the required size.
    let npending = unsafe {
        ctest_svsend_client_get(
            idx,
            &mut lastack,
            &mut snapshotresume,
            &mut numpending,
            rsnum.as_mut_ptr(),
            rsstr.as_mut_ptr(),
            &mut num_pings,
            pings.as_mut_ptr(),
        );
        ctest_svsend_pending_copy(idx, pending.as_mut_ptr(), 512)
    };
    assert!(
        npending <= 512,
        "fixture outgrew the pending capture buffer"
    );
    pending.truncate(npending as usize);

    // SAFETY: prev_get bounds-checks and returns -1 past the end.
    unsafe {
        for i in 0..ctest_svsend_prev_count(idx) {
            let mut w = [0i32; STATEWORDS];
            let num = ctest_svsend_prev_get(idx, i, w.as_mut_ptr());
            let mut st = St::from_words(&w);
            // SV_BuildEntityState (sv_send.c:811) never assigns solidsize and
            // the snapshot array grows through a non-zeroing Mem_Realloc, so
            // this word is whatever the allocator last left there. It never
            // reaches the wire -- the delta path never sets UF_SOLID -- so mask
            // it rather than let uninitialised memory into the record.
            st.solidsize = 0;
            prev.push((num as u32, st));
        }
        numframes = ctest_svsend_numframes(idx);
        for slot in 0..numframes {
            let mut seq: c_int = 0;
            let mut ts: c_uint = 0;
            let mut numents: c_int = 0;
            let mut frsnum = [0u32; 8];
            let mut frsstr = [0u32; 8];
            ctest_svsend_frame_get(
                idx,
                slot,
                &mut seq,
                &mut ts,
                &mut numents,
                frsnum.as_mut_ptr(),
                frsstr.as_mut_ptr(),
            );
            let interesting = seq != c_int::MIN
                || numents != 0
                || frsnum.iter().any(|&v| v != 0)
                || frsstr.iter().any(|&v| v != 0);
            if !interesting {
                continue;
            }
            let mut ents = Vec::new();
            for i in 0..numents {
                let (mut num, mut ebits, mut csqcbits) = (0u32, 0u32, 0u32);
                if ctest_svsend_frame_ent(idx, slot, i, &mut num, &mut ebits, &mut csqcbits) != 0 {
                    ents.push((num, ebits, csqcbits));
                }
            }
            frames.push(FrameRec {
                slot,
                seq,
                ts,
                numents,
                rsnum: frsnum,
                rsstr: frsstr,
                ents,
            });
        }
    }

    ClientRec {
        numframes,
        lastack,
        snapshotresume,
        numpending,
        rsnum,
        rsstr,
        num_pings,
        pings,
        pending,
        prev,
        frames,
        oldstats: (0..STATS_CAPTURED)
            .map(|i| read_stat(idx, i, true))
            .collect(),
    }
}

fn capture<T>(value: T) -> Snap<T> {
    const MSGMAX: usize = 70000;
    let mut msg = vec![0u8; MSGMAX];
    let mut states = Vec::new();
    let mut pvs = vec![0u8; 64];
    let (mut cur, mut peak) = (0, 0);
    let mut con = Vec::new();

    // SAFETY: every buffer is at least the size the oracle is told it is.
    let (msglen, pvslen) = unsafe {
        let n = ctest_svsend_msg_copy(msg.as_mut_ptr(), MSGMAX as c_int);
        for slot in 0..4 {
            let mut w = [0i32; STATEWORDS];
            ctest_svsend_get_state(slot, w.as_mut_ptr());
            states.push(St::from_words(&w));
        }
        let p = ctest_svsend_pvs_copy(pvs.as_mut_ptr(), 64);
        ctest_svsend_devstats_get(&mut cur, &mut peak);
        for i in 0..ctest_con_log_len() {
            con.push(cstr(ctest_con_log_get(i)).unwrap_or_default());
        }
        (n, p)
    };
    assert!(
        msglen as usize <= MSGMAX,
        "message outgrew the capture buffer"
    );
    msg.truncate(msglen as usize);
    pvs.truncate((pvslen.max(0) as usize).min(64));

    Snap {
        value,
        msg,
        states,
        clients: (0..2).map(capture_client).collect(),
        stats: (0..STATS_CAPTURED)
            .map(|i| read_stat(0, i, false))
            .collect(),
        pvs,
        devstats: (cur, peak),
        con,
        net: capture_net(),
    }
}

/// The seam. Replays `fx` into every side, runs `run` once per side, captures
/// the full observable record per side and asserts the records identical.
/// With `SIDES == [Side::C]` the comparison is vacuous and the caller's
/// explicit expectations are the gate; T6.3 adds `Side::Rust` and every caller
/// becomes a differential test unchanged.
fn diff<T, F>(fx: &Fx, run: F) -> Snap<T>
where
    T: PartialEq + std::fmt::Debug,
    F: Fn(Side) -> T,
{
    let mut out: Vec<(Side, Snap<T>)> = Vec::new();
    for side in SIDES {
        apply(side, fx);
        // SAFETY: clears stubs.c's shared send log so the NetRec captured
        // below belongs to this side's run alone.
        unsafe { ctest_net_send_reset() };
        let value = run(side);
        out.push((side, capture(value)));
    }
    let (first_side, _) = out[0];
    for i in 1..out.len() {
        let (side, _) = out[i];
        assert_eq!(
            out[0].1, out[i].1,
            "{first_side:?} and {side:?} disagree on the observable record"
        );
    }
    out.swap_remove(0).1
}

/// Assert the emitted message equals `want` byte for byte, with a readable
/// hex diff when it does not.
fn assert_msg(got: &[u8], want: &[u8]) {
    if got != want {
        let hex = |b: &[u8]| {
            b.iter()
                .map(|v| format!("{v:02x}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        panic!(
            "message mismatch\n  got:  [{}]\n  want: [{}]",
            hex(got),
            hex(want)
        );
    }
}

// ---------------------------------------------------------------------------
// A. MSG_WriteStaticOrBaseLine, FTE branch. Covers MSGFTE_DeltaCalcBits and
// MSGFTE_WriteEntityUpdate byte for byte: `from` is always nullentitystate, so
// every field of the fixture state that differs from it lights exactly one bit
// of the mask and the mask's promotion/extend cascade is fully visible.

fn baseline(st: St, idx: c_int, pext2: u32, proto: u32, pflags: u32) -> Vec<u8> {
    let fx = Fx {
        states: vec![(0, st)],
        protocol: proto,
        protocolflags: pflags,
        ..base()
    };
    diff(&fx, |side| {
        drive_baseline(side, idx, 0, pext2, proto, pflags)
    })
    .msg
}

fn fte_baseline(st: St) -> Vec<u8> {
    baseline(st, 1, RD, PROTOCOL_FITZQUAKE, 0)
}

#[test]
fn oracle_layout_matches_the_rust_mirrors() {
    let _g = lock();
    // SAFETY: pure reads of compile-time constants in the oracle TU.
    unsafe {
        assert_eq!(ctest_svsend_statewords() as usize, STATEWORDS);
        // worldmodel.numleafs is 9, so `(9 + 31) / 8` is 5: deliberately not a
        // multiple of 4, which is what makes SV_AddToFatPVS's tail bug visible.
        assert_eq!(ctest_svsend_fatbytes(), 5);
    }
}

#[test]
fn fte_baseline_null_state_writes_an_empty_mask() {
    let _g = lock();
    // opcode, MSG_WriteShort of the entity index, then a single zero bits byte
    assert_msg(
        &fte_baseline(St::default()),
        &[SVCFTE_SPAWNBASELINE2, 1, 0, 0],
    );
}

#[test]
fn fte_static_omits_the_index() {
    let _g = lock();
    let st = St {
        frame: 3,
        ..St::default()
    };
    // idx < 0 selects svcfte_spawnstatic2 and writes no entity number.
    assert_msg(
        &baseline(st, -1, RD, PROTOCOL_FITZQUAKE, 0),
        &[SVCFTE_SPAWNSTATIC2, UF_FRAME as u8, 3],
    );
}

#[test]
fn fte_baseline_origin_and_angles_quantize_at_the_boundary() {
    let _g = lock();
    // ADR-010: 0.0625*8 == 0.5 and -0.0625*8 == -0.5 sit exactly on Q_rint's
    // away-from-zero split; 179.296_88 maps to exactly 127.5 (rounds up to
    // 128) while the float below it lands at 127.495 (rounds down to 127).
    let st = St {
        origin: [0.0625, -0.0625, 0.0],
        angles: [179.296_88, 179.29, 0.0],
        ..St::default()
    };
    assert_msg(
        &fte_baseline(st),
        &[
            SVCFTE_SPAWNBASELINE2,
            0x01,
            0x00,
            (UF_ORIGINXY | UF_ANGLESXZ | UF_ANGLESY) as u8, // 0x1a
            0x01,
            0x00, // coord x: Q_rint(0.5) == 1
            0xff,
            0xff, // coord y: Q_rint(-0.5) == -1
            0x80, // angle[0]: Q_rint(127.5) == 128
            0x00, // angle[2]
            0x7f, // angle[1]: Q_rint(127.495) == 127
        ],
    );
}

#[test]
fn fte_baseline_int32coord_and_shortangle() {
    let _g = lock();
    let st = St {
        origin: [0.03125, 0.0, 0.0],
        angles: [0.0, 179.296_88, 0.0],
        ..St::default()
    };
    assert_msg(
        &baseline(st, 2, RD, PROTOCOL_RMQ, PRFL_INT32COORD | PRFL_SHORTANGLE),
        &[
            SVCFTE_SPAWNBASELINE2,
            0x02,
            0x00,
            (UF_ORIGINXY | UF_ANGLESY) as u8, // 0x12
            0x01,
            0x00,
            0x00,
            0x00, // Long(Q_rint(0.03125*16)) == 1
            0x00,
            0x00,
            0x00,
            0x00,
            0x80,
            0x7f, // Short(Q_rint(32640.0))
        ],
    );
}

#[test]
fn fte_baseline_floatcoord() {
    let _g = lock();
    let st = St {
        origin: [0.0625, 0.0, 0.0],
        ..St::default()
    };
    assert_msg(
        &baseline(st, 1, RD, PROTOCOL_RMQ, PRFL_FLOATCOORD),
        &[
            SVCFTE_SPAWNBASELINE2,
            0x01,
            0x00,
            UF_ORIGINXY as u8,
            0x00,
            0x00,
            0x80,
            0x3d, // 0.0625f verbatim
            0x00,
            0x00,
            0x00,
            0x00,
        ],
    );
}

#[test]
fn fte_baseline_promotes_to_16bit_for_large_model_and_frame() {
    let _g = lock();
    let st = St {
        modelindex: 300,
        frame: 400,
        ..St::default()
    };
    // UF_MODEL|UF_FRAME|UF_16BIT == 0x601, which lights UF_EXTEND1.
    assert_msg(
        &fte_baseline(st),
        &[
            SVCFTE_SPAWNBASELINE2,
            0x01,
            0x00,
            0x81,
            0x06,
            0x90,
            0x01, // frame first: Short(400)
            0x2c,
            0x01, // then Short(300)
        ],
    );
}

#[test]
fn fte_baseline_effects_widen_by_magnitude() {
    let _g = lock();
    let one = |e: i32| {
        fte_baseline(St {
            effects: e,
            ..St::default()
        })
    };
    // <= 0xff: UF_EFFECTS, one byte.
    assert_msg(
        &one(0xff),
        &[SVCFTE_SPAWNBASELINE2, 1, 0, UF_EFFECTS as u8, 0xff],
    );
    // 0x100..0xffff: UF_EFFECTS is *replaced* by UF_EFFECTS2 (1<<29), which
    // cascades EXTEND3 -> EXTEND2 -> EXTEND1 because EXTEND3 is itself 1<<23.
    assert_msg(
        &one(0x100),
        &[
            SVCFTE_SPAWNBASELINE2,
            1,
            0,
            0x80,
            0x80,
            0x80,
            0x20,
            0x00,
            0x01,
        ],
    );
    // >= 0x10000: both bits set, written as a long.
    assert_msg(
        &one(0x10000),
        &[
            SVCFTE_SPAWNBASELINE2,
            1,
            0,
            0xa0,
            0x80,
            0x80,
            0x20,
            0x00,
            0x00,
            0x01,
            0x00,
        ],
    );
}

#[test]
fn fte_baseline_alpha_is_biased_by_one() {
    let _g = lock();
    // COMPAT: MSGFTE_WriteEntityUpdate writes (alpha - 1) & 0xff
    // (Quake/sv_send.c:384), so encoded alpha 1 goes on the wire as 0.
    assert_msg(
        &fte_baseline(St {
            alpha: 65,
            scale: 15,
            ..St::default()
        }),
        &[SVCFTE_SPAWNBASELINE2, 1, 0, 0x80, 0x80, 0x03, 0x40, 0x0f],
    );
    assert_msg(
        &fte_baseline(St {
            alpha: 1,
            ..St::default()
        }),
        &[SVCFTE_SPAWNBASELINE2, 1, 0, 0x80, 0x80, 0x01, 0x00],
    );
}

#[test]
fn fte_baseline_taginfo_uses_the_large_entity_escape() {
    let _g = lock();
    // tagentity > 0x7fff with PEXT2_REPLACEMENTDELTAS: Short(0x8000|(n>>8)) + Byte(n).
    assert_msg(
        &fte_baseline(St {
            tagentity: 0x8000,
            tagindex: 7,
            ..St::default()
        }),
        &[
            SVCFTE_SPAWNBASELINE2,
            1,
            0,
            0x80,
            0x80,
            0x10,
            0x80,
            0x80,
            0x00,
            0x07,
        ],
    );
    assert_msg(
        &fte_baseline(St {
            tagentity: 5,
            ..St::default()
        }),
        &[
            SVCFTE_SPAWNBASELINE2,
            1,
            0,
            0x80,
            0x80,
            0x10,
            0x05,
            0x00,
            0x00,
        ],
    );
}

#[test]
fn fte_baseline_traileffect_packs_the_emit_flag() {
    let _g = lock();
    assert_msg(
        &fte_baseline(St {
            traileffectnum: 0x4001,
            ..St::default()
        }),
        &[SVCFTE_SPAWNBASELINE2, 1, 0, 0x80, 0x80, 0x40, 0x01, 0x00],
    );
    // with an emit effect the trail short gains 0x8000 and a second short follows
    assert_msg(
        &fte_baseline(St {
            traileffectnum: 0x4001,
            emiteffectnum: 0x4002,
            ..St::default()
        }),
        &[
            SVCFTE_SPAWNBASELINE2,
            1,
            0,
            0x80,
            0x80,
            0x40,
            0x01,
            0x80,
            0x02,
            0x00,
        ],
    );
}

#[test]
fn fte_baseline_colormod_is_all_three_or_none() {
    let _g = lock();
    assert_msg(
        &fte_baseline(St {
            colormod: [31, 32, 33],
            ..St::default()
        }),
        &[
            SVCFTE_SPAWNBASELINE2,
            1,
            0,
            0x80,
            0x80,
            0x80,
            0x01,
            0x1f,
            0x20,
            0x21,
        ],
    );
}

#[test]
fn fte_baseline_predinfo_movetype_and_velocity() {
    let _g = lock();
    // pmovetype difference sets UF_PREDINFO|UF_MOVETYPE; UF_MOVETYPE is an
    // alias of UF_EFFECTS2 and is converted to UFP_MOVETYPE in the pred byte.
    assert_msg(
        &fte_baseline(St {
            pmovetype: 3,
            ..St::default()
        }),
        &[SVCFTE_SPAWNBASELINE2, 1, 0, UF_PREDINFO as u8, 0x08, 0x03],
    );
    assert_msg(
        &fte_baseline(St {
            velocity: [1, 0, -2],
            ..St::default()
        }),
        &[
            SVCFTE_SPAWNBASELINE2,
            1,
            0,
            UF_PREDINFO as u8,
            0x30, // UFP_VELOCITYXY|UFP_VELOCITYZ
            0x01,
            0x00,
            0x00,
            0x00,
            0xfe,
            0xff,
        ],
    );
}

#[test]
fn fte_baseline_predinfo_selects_the_angle_width() {
    let _g = lock();
    let st = St {
        pmovetype: 1,
        angles: [0.0, 179.296_88, 0.0],
        ..St::default()
    };
    // UF_PREDINFO without PEXT2_PREDINFO: 16-bit angles.
    assert_msg(
        &baseline(st, 1, RD, PROTOCOL_FITZQUAKE, 0),
        &[SVCFTE_SPAWNBASELINE2, 1, 0, 0x50, 0x80, 0x7f, 0x08, 0x01],
    );
    // with PEXT2_PREDINFO the client already has the angles: byte precision.
    assert_msg(
        &baseline(st, 1, RDP, PROTOCOL_FITZQUAKE, 0),
        &[SVCFTE_SPAWNBASELINE2, 1, 0, 0x50, 0x80, 0x08, 0x01],
    );
}

#[test]
fn fte_baseline_strips_the_lerp_bit_for_remote_clients() {
    let _g = lock();
    // COMPAT: MSGFTE_WriteEntityUpdate (Quake/sv_send.c:266) computes
    // UF_UNUSED2 from state->lerp and then drops it again unless the peer
    // address is "LOCAL". The stub address is "ctest", so the bit and the
    // trailing short are always suppressed -- the mask ends up empty, which
    // also means the EXTEND cascade never runs.
    assert_msg(
        &fte_baseline(St {
            lerp: 100,
            ..St::default()
        }),
        &[SVCFTE_SPAWNBASELINE2, 1, 0, 0x00],
    );
}

#[test]
fn fte_baseline_16bit_leaks_from_model_into_skin() {
    let _g = lock();
    assert_msg(
        &fte_baseline(St {
            skin: 5,
            colormap: 7,
            eflags: 1,
            ..St::default()
        }),
        &[SVCFTE_SPAWNBASELINE2, 1, 0, 0x80, 0x58, 0x05, 0x07, 0x01],
    );
    // COMPAT: UF_16BIT is a single mask bit shared by frame, model and skin,
    // so a large modelindex silently widens the skin field too.
    assert_msg(
        &fte_baseline(St {
            modelindex: 300,
            skin: 5,
            ..St::default()
        }),
        &[
            SVCFTE_SPAWNBASELINE2,
            1,
            0,
            0x80,
            0x0e,
            0x2c,
            0x01,
            0x05,
            0x00,
        ],
    );
}

// ---------------------------------------------------------------------------
// B. MSG_WriteStaticOrBaseLine, classic branch.

#[test]
fn classic_baseline_writes_the_fixed_layout() {
    let _g = lock();
    let st = St {
        modelindex: 10,
        frame: 20,
        colormap: 3,
        skin: 4,
        origin: [8.0, -8.0, 0.0625],
        angles: [90.0, 180.0, 270.0],
        ..St::default()
    };
    assert_msg(
        &baseline(st, 5, 0, PROTOCOL_FITZQUAKE, 0),
        &[
            SVC_SPAWNBASELINE,
            0x05,
            0x00,
            0x0a,
            0x14,
            0x03,
            0x04,
            0x40,
            0x00,
            0x40, // origin[0]=8 -> 64, angle 90 -> 64
            0xc0,
            0xff,
            0x80, // origin[1]=-8 -> -64, angle 180 -> 128
            0x01,
            0x00,
            0xc0, // origin[2] Q_rint(0.5) -> 1, angle 270 -> 192
        ],
    );
}

#[test]
fn classic_baseline_extension_bits_depend_on_the_protocol() {
    let _g = lock();
    let st = St {
        modelindex: 300,
        frame: 400,
        alpha: 200,
        scale: 99,
        ..St::default()
    };
    let tail = |extra: &[u8]| {
        let mut v = vec![SVC_SPAWNBASELINE2, 0x01, 0x00];
        v.extend_from_slice(extra);
        v
    };

    // FITZQUAKE: large model, large frame and alpha, but never scale.
    let mut want = tail(&[0x07, 0x2c, 0x01, 0x90, 0x01, 0x00, 0x00]);
    want.extend_from_slice(&[0u8; 9]);
    want.push(0xc8);
    assert_msg(&baseline(st, 1, 0, PROTOCOL_FITZQUAKE, 0), &want);

    // RMQ additionally sends B_SCALE.
    let mut want = tail(&[0x0f, 0x2c, 0x01, 0x90, 0x01, 0x00, 0x00]);
    want.extend_from_slice(&[0u8; 9]);
    want.extend_from_slice(&[0xc8, 0x63]);
    assert_msg(&baseline(st, 1, 0, PROTOCOL_RMQ, 0), &want);

    // COMPAT: under PROTOCOL_NETQUAKE the B_* bits are never computed
    // (Quake/sv_send.c:970), so modelindex 300 and frame 400 are silently
    // truncated to bytes and alpha/scale are dropped entirely.
    let mut want = vec![SVC_SPAWNBASELINE, 0x01, 0x00, 0x2c, 0x90, 0x00, 0x00];
    want.extend_from_slice(&[0u8; 9]);
    assert_msg(&baseline(st, 1, 0, PROTOCOL_NETQUAKE, 0), &want);
}

#[test]
fn classic_static_selects_the_opcode_from_the_bits() {
    let _g = lock();
    let mut want = vec![SVC_SPAWNSTATIC, 0x00, 0x00, 0x00, 0x00];
    want.extend_from_slice(&[0u8; 9]);
    assert_msg(
        &baseline(St::default(), -1, 0, PROTOCOL_FITZQUAKE, 0),
        &want,
    );

    let st = St {
        alpha: 200,
        ..St::default()
    };
    let mut want = vec![SVC_SPAWNSTATIC2, 0x04, 0x00, 0x00, 0x00, 0x00];
    want.extend_from_slice(&[0u8; 9]);
    want.push(0xc8);
    assert_msg(&baseline(st, -1, 0, PROTOCOL_FITZQUAKE, 0), &want);
}

// ---------------------------------------------------------------------------
// C. SV_BuildEntityState: the edict -> entity_state_t quantizer. Observed
// through the packed state slot rather than through bytes.

fn buildstate(fx: &Fx, ednum: c_int) -> St {
    diff(fx, |side| drive_buildstate(side, ednum, 0)).states[0]
}

fn ext_state(mask: u32, e: Ed) -> St {
    let fx = Fx {
        num_edicts: 4,
        extfields: mask,
        edicts: vec![e],
        ..base()
    };
    buildstate(&fx, 1)
}

#[test]
fn buildstate_defaults_and_does_not_clear_the_target() {
    let _g = lock();
    // COMPAT: SV_BuildEntityState (Quake/sv_send.c:811) only clears eflags,
    // pmovetype and velocity; every other field it does not write keeps
    // whatever the caller's state slot already held. solidsize is the one
    // field nothing in this function touches, so it survives verbatim.
    let fx = Fx {
        edicts: vec![ed(1)],
        states: vec![(
            0,
            St {
                solidsize: 123,
                alpha: 200,
                lerp: 999,
                ..St::default()
            },
        )],
        ..base()
    };
    assert_eq!(
        buildstate(&fx, 1),
        St {
            solidsize: 123,
            ..St::default()
        }
    );
}

#[test]
fn buildstate_scale_truncates_and_clamps() {
    let _g = lock();
    // ENTSCALE_ENCODE truncates (int)(16*f) rather than rounding, so the
    // float immediately below 1.0 drops a whole step. ADR-010 boundary.
    let enc = |f: f32| ext_state(xf(XF_SCALE), ed(1).ext1(XF_SCALE, f)).scale;
    assert_eq!(enc(1.0), 16);
    assert_eq!(enc(f32::from_bits(1.0f32.to_bits() - 1)), 15);
    assert_eq!(enc(20.0), 255); // upper clamp
    assert_eq!(enc(0.01), 1); // lower clamp: (int)0.16 == 0 -> 1
    assert_eq!(enc(0.0), ENTSCALE_DEFAULT); // exactly zero means "default"
}

#[test]
fn buildstate_alpha_rounds_at_the_half() {
    let _g = lock();
    // ENTALPHA_ENCODE is Q_rint(CLAMP(1, a*254.0f+1, 255)) and 0.25 lands on
    // 64.5 exactly, which Q_rint takes up to 65. ADR-010 boundary.
    let enc = |f: f32| ext_state(xf(XF_ALPHA), ed(1).ext1(XF_ALPHA, f)).alpha;
    assert_eq!(enc(0.25), 65);
    // COMPAT: the encode runs in float, so the ulp *below* 0.25 still rounds
    // back onto 64.5 (ties-to-even in the `+ 1`) and encodes as 65 as well; it
    // takes two ulps to reach a representable 64.49999237 and drop to 64.
    assert_eq!(enc(f32::from_bits(0.25f32.to_bits() - 1)), 65);
    assert_eq!(enc(f32::from_bits(0.25f32.to_bits() - 2)), 64);
    assert_eq!(enc(0.0), 0);
    assert_eq!(enc(1.0), 255);
    // without the extfield the edict's own alpha byte is used verbatim
    assert_eq!(
        buildstate(
            &Fx {
                edicts: vec![ed(1).alpha(77)],
                ..base()
            },
            1
        )
        .alpha,
        77
    );
}

#[test]
fn buildstate_colormod_truncates() {
    let _g = lock();
    let enc = |v: [f32; 3]| ext_state(xf(XF_COLORMOD), ed(1).ext(XF_COLORMOD, v)).colormod;
    assert_eq!(
        enc([1.0, f32::from_bits(1.0f32.to_bits() - 1), 2.0]),
        [32, 31, 64]
    );
    // an all-zero colormod field is treated as "absent", not as black
    assert_eq!(enc([0.0, 0.0, 0.0]), [32, 32, 32]);
}

#[test]
fn buildstate_effects_are_masked_then_or_ed_with_modelflags() {
    let _g = lock();
    let fx = Fx {
        num_edicts: 4,
        effectsmask: 0x0f,
        extfields: xf(XF_MODELFLAGS),
        edicts: vec![ed(1).ev(EV_EFFECTS, 255.0).ext1(XF_MODELFLAGS, 8.0)],
        ..base()
    };
    assert_eq!(buildstate(&fx, 1).effects, 0x0f | (8 << 24));
}

#[test]
fn buildstate_movetype_step_sets_eflags() {
    let _g = lock();
    let fx = Fx {
        edicts: vec![ed(1).ev(EV_MOVETYPE, MOVETYPE_STEP)],
        ..base()
    };
    assert_eq!(buildstate(&fx, 1).eflags, 1); // EFLAGS_STEP
}

#[test]
fn buildstate_tag_and_effect_fields() {
    let _g = lock();
    let mask = xf(XF_TAG_ENTITY) | xf(XF_TAG_INDEX) | xf(XF_TRAILEFFECTNUM) | xf(XF_EMITEFFECTNUM);
    let st = ext_state(
        mask,
        ed(1)
            .ext_edict(XF_TAG_ENTITY, 3)
            .ext1(XF_TAG_INDEX, 3.7)
            .ext1(XF_TRAILEFFECTNUM, 12.9)
            .ext1(XF_EMITEFFECTNUM, 7.0),
    );
    assert_eq!((st.tagentity, st.tagindex), (3, 3));
    assert_eq!((st.traileffectnum, st.emiteffectnum), (12, 7));
    // a zero .tag_entity reference is "no parent", not edict 0
    let st = ext_state(mask, ed(1).ext_edict(XF_TAG_ENTITY, -1));
    assert_eq!(st.tagentity, 0);
}

#[test]
fn buildstate_lerp_rounds_the_think_interval() {
    let _g = lock();
    // COMPAT: LERP_BANDAID stores Q_rint((nextthink - time) * 1000) + 1 in an
    // `unsigned short`, so a think time in the past wraps rather than clamping.
    let enc = |nextthink: f32, si: c_int, sid: c_int| {
        buildstate(
            &Fx {
                edicts: vec![ed(1).ev(EV_NEXTTHINK, nextthink).lerpinfo(si, sid)],
                ..base()
            },
            1,
        )
        .lerp
    };
    assert_eq!(enc(0.0625, 1, 0), 64); // Q_rint(62.5) == 63
    assert_eq!(enc(0.0624, 1, 0), 63); // Q_rint(62.4) == 62
    assert_eq!(enc(0.0625, 0, 1), 64); // sendinterval_default alone is enough
    assert_eq!(enc(-0.0625, 1, 0), 65474); // (unsigned short)(-62)
    assert_eq!(enc(0.0625, 0, 0), 0); // neither interval set: no lerp at all
}

#[test]
fn buildstate_pred_think_position_gate() {
    let _g = lock();
    let run = |smoothlerps: f32, movetype: f32, flags: f32, time: f64, lastthink: f32| {
        buildstate(
            &Fx {
                smoothlerps,
                time,
                edicts: vec![ed(1)
                    .ev(EV_MOVETYPE, movetype)
                    .ev(EV_FLAGS, flags)
                    .evv(EVV_ORIGIN, [1.0, 2.0, 3.0])
                    .predthink(lastthink, [10.0, 20.0, 30.0])],
                ..base()
            },
            1,
        )
        .origin
    };
    let pred = [10.0, 20.0, 30.0];
    let plain = [1.0, 2.0, 3.0];

    assert_eq!(run(1.0, MOVETYPE_STEP, FL_ONGROUND, 0.0, 0.0), pred);
    assert_eq!(run(1.0, MOVETYPE_STEP, FL_ONGROUND, 0.05, 0.0), pred);
    // ADR-010: the elapsed time is narrowed to float before the `> 0.1`
    // comparison, and 0.1f is strictly greater than the double 0.1, so an
    // interval of exactly 0.1 falls *outside* the window.
    assert_eq!(run(1.0, MOVETYPE_STEP, FL_ONGROUND, 0.1, 0.0), plain);
    assert_eq!(run(1.0, MOVETYPE_STEP, FL_ONGROUND, 0.1, 0.2), plain); // negative
    assert_eq!(run(0.0, MOVETYPE_STEP, FL_ONGROUND, 0.0, 0.0), plain); // cvar off
    assert_eq!(run(1.0, 3.0, FL_ONGROUND, 0.0, 0.0), plain); // wrong movetype
    assert_eq!(run(1.0, MOVETYPE_STEP, 0.0, 0.0, 0.0), plain); // airborne
}

// ---------------------------------------------------------------------------
// D. SV_CalcStats.

fn calcstats(fx: &Fx) -> Snap<()> {
    diff(fx, |side| drive_calcstats(side, 0))
}

/// A fixture whose model precache holds "" at 0 and a weapon model at 1.
/// SV_ModelIndex Sys_Errors on a miss, so the precache list must be dense.
fn weapon_fx(e: Ed, pext2: u32, limit_models: u32) -> Fx {
    Fx {
        num_edicts: 4,
        strings: vec![(1, "progs/v_shot.mdl")],
        precaches: vec![(0, 0), (1, 1)],
        edicts: vec![e],
        clients: vec![cl(0).pext2(pext2).limits(8192, limit_models, 64000)],
        ..base()
    }
}

#[test]
fn calcstats_maps_the_classic_stats() {
    let _g = lock();
    let snap = calcstats(&weapon_fx(
        ed(1)
            .ev(EV_HEALTH, 100.0)
            .ev(EV_WEAPONMODEL, 64.0) // string slot 1
            .ev(EV_CURRENTAMMO, 25.0)
            .ev(EV_ARMORVALUE, 50.0)
            .ev(EV_WEAPONFRAME, 3.0)
            .ev(EV_AMMO_SHELLS, 10.0)
            .ev(EV_AMMO_NAILS, 20.0)
            .ev(EV_AMMO_ROCKETS, 30.0)
            .ev(EV_AMMO_CELLS, 40.0)
            .ev(EV_WEAPON, 8.0)
            .ev(EV_ITEMS, 17.0),
        0,
        2048,
    ));
    assert_eq!(snap.statf(STAT_HEALTH), 100.0);
    assert_eq!(snap.stati(STAT_WEAPON), 1);
    assert_eq!(snap.statf(STAT_AMMO), 25.0);
    assert_eq!(snap.statf(STAT_ARMOR), 50.0);
    assert_eq!(snap.statf(STAT_WEAPONFRAME), 3.0);
    assert_eq!(snap.statf(STAT_SHELLS), 10.0);
    assert_eq!(snap.statf(STAT_NAILS), 20.0);
    assert_eq!(snap.statf(STAT_ROCKETS), 30.0);
    assert_eq!(snap.statf(STAT_CELLS), 40.0);
    assert_eq!(snap.statf(STAT_ACTIVEWEAPON), 8.0);
    // without PEXT2_PREDINFO the items/view stats stay zero
    assert_eq!(snap.stati(STAT_ITEMS), 0);
    assert_eq!(snap.statf(STAT_VIEWHEIGHT), 0.0);
    assert_eq!(snap.stats_str(STAT_HEALTH), None);
}

#[test]
fn calcstats_weapon_model_respects_the_client_limit() {
    let _g = lock();
    let snap = calcstats(&weapon_fx(ed(1).ev(EV_WEAPONMODEL, 64.0), 0, 1));
    assert_eq!(snap.stati(STAT_WEAPON), 0);
    // an empty weaponmodel string never reaches the precache scan
    let snap = calcstats(&weapon_fx(ed(1), 0, 2048));
    assert_eq!(snap.stati(STAT_WEAPON), 0);
}

#[test]
fn calcstats_predinfo_adds_items_and_view_state() {
    let _g = lock();
    let mut fx = weapon_fx(
        ed(1)
            .ev(EV_ITEMS, 17.0)
            .ev(EV_IDEALPITCH, -5.0)
            .evv(EVV_VIEW_OFS, [0.0, 0.0, 22.0])
            .evv(EVV_PUNCHANGLE, [1.0, 2.0, 3.0]),
        RDP,
        2048,
    );
    fx.serverflags = 3.0;
    let snap = calcstats(&fx);
    // items | (serverflags << 28)
    assert_eq!(snap.stati(STAT_ITEMS), 0x3000_0011u32 as i32);
    assert_eq!(snap.statf(STAT_VIEWHEIGHT), 22.0);
    assert_eq!(snap.statf(STAT_IDEALPITCH), -5.0);
    assert_eq!(snap.statf(STAT_IDEALPITCH + 1), 1.0); // STAT_PUNCHANGLE_X
    assert_eq!(snap.statf(STAT_IDEALPITCH + 2), 2.0);
    assert_eq!(snap.statf(STAT_IDEALPITCH + 3), 3.0);
}

#[test]
fn calcstats_items2_replaces_the_serverflags_shift() {
    let _g = lock();
    let mut fx = weapon_fx(ed(1).ev(EV_ITEMS, 17.0).ext1(XF_ITEMS2, 5.0), RDP, 2048);
    fx.serverflags = 3.0;
    fx.extfields = xf(XF_ITEMS2);
    // COMPAT: when .items2 exists serverflags is dropped entirely and the
    // extra bits are shifted by 23, not 28 (Quake/sv_send.c:57).
    assert_eq!(calcstats(&fx).stati(STAT_ITEMS), 0x11 | (5 << 23));
}

#[test]
fn calcstats_viewzoom_clamps_low_but_not_high() {
    let _g = lock();
    let zoom = |v: f32| {
        let mut fx = weapon_fx(ed(1).ext1(XF_VIEWZOOM, v), 0, 2048);
        fx.extfields = xf(XF_VIEWZOOM);
        calcstats(&fx).statf(STAT_VIEWZOOM)
    };
    assert_eq!(zoom(0.5), 127.5);
    assert_eq!(zoom(0.001), 1.0); // lower clamp
                                  // COMPAT: there is no upper clamp, so a viewzoom above 1 overflows the
                                  // byte the client eventually reads.
    assert_eq!(zoom(2.0), 510.0);
    // exactly zero skips the whole block, leaving the stat at 0 rather than 1
    assert_eq!(zoom(0.0), 0.0);
}

#[test]
fn calcstats_custom_stats_numeric_types() {
    let _g = lock();
    let mut fx = weapon_fx(ed(1), 0, 2048);
    fx.evalslots = vec![
        (0, 1.5f32.to_bits(), 0, 0),
        (1, 1.0f32.to_bits(), 2.0f32.to_bits(), 3.0f32.to_bits()),
        (2, 0xdead_beef, 0, 0),
        (3, 0x1122_3344, 5, 0),
    ];
    fx.customstats = vec![
        (30, EV_T_FLOAT, -1, 0),
        (31, EV_T_VECTOR, -1, 1),
        (34, EV_T_EXT_INTEGER, -1, 2),
        (35, EV_T_EXT_SINT64, -1, 3),
    ];
    let snap = calcstats(&fx);
    assert_eq!(snap.statf(30), 1.5);
    assert_eq!(
        (snap.statf(31), snap.statf(32), snap.statf(33)),
        (1.0, 2.0, 3.0)
    );
    assert_eq!(snap.stati(34), 0xdead_beefu32 as i32);
    // a 64-bit stat occupies two consecutive slots, low word first
    assert_eq!((snap.stati(35), snap.stati(36)), (0x1122_3344, 5));
}

#[test]
fn calcstats_custom_stats_double_string_and_entity() {
    let _g = lock();
    let mut fx = weapon_fx(ed(1).ext_edict(XF_TAG_ENTITY, 3), 0, 2048);
    fx.extfields = xf(XF_TAG_ENTITY);
    fx.strings.push((2, "stat-string"));
    fx.evalslots = vec![
        (0, 0x0000_0000, 0x4004_0000, 0), // 2.5 as an ieee754 double
        (1, 128, 0, 0),                   // string slot 2 == offset 128
    ];
    fx.customstats = vec![
        (30, EV_T_EXT_DOUBLE, -1, 0),
        (31, EV_T_STRING, -1, 1),
        (32, EV_T_ENTITY, XF_TAG_ENTITY, -1),
    ];
    let snap = calcstats(&fx);
    assert_eq!(snap.statf(30), 2.5);
    assert_eq!(snap.stats_str(31), Some("stat-string"));
    assert_eq!(snap.stati(32), 3);
}

// ---------------------------------------------------------------------------
// E. SVFTE_SetupFrames / SVFTE_DestroyFrames / SVFTE_Ack. The frame ring is
// pure state, so the record here is `ClientRec`, not bytes.

fn frames_fx(pext2: u32) -> Fx {
    Fx {
        clients: vec![cl(0).pext2(pext2).frames()],
        ..base()
    }
}

#[test]
fn setup_frames_without_replacement_deltas_destroys_instead() {
    let _g = lock();
    let snap = diff(&frames_fx(0), |_| ());
    let c = snap.client(0);
    assert_eq!(c.numframes, 0);
    assert_eq!(c.lastack, 0);
    assert_eq!(c.numpending, 0);
    assert!(c.pending.is_empty());
    assert!(c.prev.is_empty());
}

#[test]
fn setup_frames_allocates_the_ring_and_flags_a_full_resend() {
    let _g = lock();
    let snap = diff(&frames_fx(RD), |_| ());
    let c = snap.client(0);
    assert_eq!(c.numframes, 64);
    // lastacksequence starts at (int)0x80000000, so no real sequence is ever
    // mistaken for a stale ack (sv_send.c:474).
    assert_eq!(c.lastack, i32::MIN);
    assert_eq!(c.numpending, 2); // qcvm->num_edicts
    assert_eq!(c.pending, vec![UF_REMOVE, 0]);
    // every ring slot is pre-stamped with lastacksequence and otherwise empty,
    // which is exactly what capture_client filters out as uninteresting
    assert!(c.frames.is_empty());
}

#[test]
fn destroy_frames_releases_everything_setup_allocated() {
    let _g = lock();
    let snap = diff(&frames_fx(RD), |side| drive_destroyframes(side, 0));
    let c = snap.client(0);
    assert_eq!(c.numframes, 0);
    assert_eq!(c.lastack, 0);
    assert_eq!(c.numpending, 0);
    assert!(c.pending.is_empty());
    assert!(c.prev.is_empty());
}

#[test]
fn ack_is_a_no_op_without_a_frame_ring() {
    let _g = lock();
    let fx = Fx {
        clients: vec![cl(0).pext2(0).frames().lastack(10)],
        ..base()
    };
    let snap = diff(&fx, |side| drive_ack(side, 0, 5));
    assert_eq!(snap.client(0).lastack, 10);
    assert_eq!(snap.client(0).num_pings, 0);
}

#[test]
fn ack_of_minus_one_forces_a_resend_before_the_stale_bail() {
    let _g = lock();
    let fx = Fx {
        clients: vec![cl(0).pext2(RD).frames().lastack(10).pending(0, 0)],
        ..base()
    };
    let snap = diff(&fx, |side| drive_ack(side, 0, -1));
    let c = snap.client(0);
    // COMPAT: sv_send.c:511 sets the full-resend bit *before* the
    // `sequence < lastacksequence` bail at :514, so a -1 ack still lands even
    // though the rest of SVFTE_Ack is skipped.
    assert_eq!(c.pending[0], UF_REMOVE);
    assert_eq!(c.lastack, 10);
    assert_eq!(c.num_pings, 0);
}

#[test]
fn ack_retires_the_acked_frame_and_records_a_ping() {
    let _g = lock();
    let fx = Fx {
        time: 1.0,
        clients: vec![cl(0).pext2(RD).frames().lastack(4).seed(Seed {
            slot: 5,
            sequence: 5,
            timestamp: 0.25,
            rsnum: [0; 8],
            rsstr: [0; 8],
            ents: Vec::new(),
        })],
        ..base()
    };
    let snap = diff(&fx, |side| drive_ack(side, 0, 5));
    let c = snap.client(0);
    assert_eq!(c.lastack, 5);
    assert_eq!(c.frames.len(), 1);
    assert_eq!(snap.frame(0, 5).seq, -1);
    assert_eq!(c.num_pings, 1);
    assert_eq!(f32::from_bits(c.pings[0]), 0.75);
}

#[test]
fn ack_walks_the_whole_ring_when_the_gap_exceeds_it() {
    let _g = lock();
    let fx = Fx {
        num_edicts: 5,
        clients: vec![cl(0).pext2(RD).frames().lastack(6).seed(Seed {
            slot: 7,
            sequence: 7,
            timestamp: 0.0,
            rsnum: [5, 0, 0, 0, 0, 0, 0, 0],
            rsstr: [0; 8],
            ents: vec![(3, UF_FRAME), (4, 0)],
        })],
        ..base()
    };
    let snap = diff(&fx, |side| drive_ack(side, 0, 8));
    let c = snap.client(0);
    // dropseq restarts at `sequence - numframes` == -56, and -56 & 63 == 8, so
    // the drop walk visits all 64 slots exactly once (sv_send.c:518).
    assert_eq!(c.lastack, 8);
    assert_eq!(c.frames.len(), 1);
    assert_eq!(snap.frame(0, 7).seq, -1);
    // the dropped frame's stats and ent bits are folded back into the client
    assert_eq!(c.rsnum[0], 5);
    assert_eq!(c.pending[3], UF_FRAME);
    // an ent logged with no bits is not re-flagged (sv_send.c:497)
    assert_eq!(c.pending[4], 0);
    assert_eq!(c.pending[0], UF_REMOVE);
    // frames[8] is still the untouched sentinel, so no ping is recorded
    assert_eq!(c.num_pings, 0);
}

// ---------------------------------------------------------------------------
// F. SV_FatPVS / SV_AddToFatPVS / SV_VisibleToClient. The oracle worldmodel is
// two planes over three leafs (solid / empty / water) with numleafs 9, so
// fatbytes is 5 -- deliberately not a multiple of 4.

fn pvs_fx(leafpvs: &[u8]) -> Fx {
    Fx {
        leafpvs: leafpvs.to_vec(),
        ..base()
    }
}

#[test]
fn fatpvs_falls_back_to_all_ones_when_only_solid_is_reached() {
    let _g = lock();
    let fx = pvs_fx(&[0xff, 0xee, 0xdd, 0xcc, 0xbb]);
    let snap = diff(&fx, |side| drive_fatpvs(side, [-100.0, 0.0, 0.0], 5));
    assert_eq!(snap.value, 5);
    // fatpvs_any stays false, so sv_send.c:1102 fills the whole thing with 0xff
    assert_eq!(snap.pvs, vec![0xff; 5]);
}

#[test]
fn fatpvs_never_ors_the_trailing_bytes() {
    let _g = lock();
    let fx = pvs_fx(&[0xff, 0xee, 0xdd, 0xcc, 0xbb]);
    for org in [
        [100.0, 100.0, 0.0],  // straight into the empty leaf
        [100.0, -100.0, 0.0], // straight into the water leaf
        [0.0, 100.0, 0.0],    // on plane 0: recurse both children
        [0.0, 0.0, 0.0],      // on both planes: every non-solid leaf ORed
    ] {
        let snap = diff(&fx, |side| drive_fatpvs(side, org, 5));
        // COMPAT: sv_send.c:1059 ORs in uint32 steps bounded by `fatbytes - 3`,
        // so with fatbytes 5 only bytes 0..3 are ever merged and byte 4 keeps
        // the zero from the memset. sv_send.c:1090 also over-allocates, using
        // `(numleafs + 31) / 8` where `(numleafs + 7) / 8` is the real width.
        assert_eq!(snap.pvs, vec![0xff, 0xee, 0xdd, 0xcc, 0x00], "org {org:?}");
    }
}

#[test]
fn fatpvs_zero_visibility_is_not_promoted_to_all_ones() {
    let _g = lock();
    let fx = pvs_fx(&[0; 5]);
    let snap = diff(&fx, |side| drive_fatpvs(side, [100.0, 100.0, 0.0], 5));
    assert_eq!(snap.value, 5);
    // reaching a non-solid leaf sets fatpvs_any even when it contributes no
    // bits, which suppresses the 0xff fallback
    assert_eq!(snap.pvs, vec![0; 5]);
}

#[test]
fn add_to_fat_pvs_accumulates_into_the_live_buffer() {
    let _g = lock();
    let fx = pvs_fx(&[0; 5]);
    let snap = diff(&fx, |side| {
        drive_fatpvs(side, [100.0, 100.0, 0.0], 5);
        // SAFETY: exactly the fixture's 5-byte leaf-PVS width.
        unsafe { ctest_svsend_set_leafpvs([0xffu8; 5].as_ptr(), 5) };
        drive_addtofatpvs(side, [100.0, 100.0, 0.0], 0, 5)
    });
    assert_eq!(snap.value, 5);
    // SV_AddToFatPVS does not clear first, and still stops one byte short
    assert_eq!(snap.pvs, vec![0xff, 0xff, 0xff, 0xff, 0x00]);
}

#[test]
fn visible_to_client_tests_every_leaf_of_the_subject() {
    let _g = lock();
    let mk = |leafs: &[c_int]| Fx {
        num_edicts: 3,
        leafpvs: vec![0x02, 0, 0, 0, 0],
        edicts: vec![
            ed(1).evv(EVV_ORIGIN, [100.0, 100.0, 0.0]),
            ed(2).leafs(leafs),
        ],
        ..base()
    };
    assert_eq!(diff(&mk(&[1]), |side| drive_visible(side, 1, 2)).value, 1);
    assert_eq!(diff(&mk(&[0]), |side| drive_visible(side, 1, 2)).value, 0);
    assert_eq!(
        diff(&mk(&[3, 1]), |side| drive_visible(side, 1, 2)).value,
        1
    );
    // num_leafs 0 never matches, so SV_VisibleToClient reports invisible
    assert_eq!(diff(&mk(&[]), |side| drive_visible(side, 1, 2)).value, 0);
}

#[test]
fn visible_to_client_offsets_the_eye_by_view_ofs() {
    let _g = lock();
    let mk = |ofs: [f32; 3]| Fx {
        num_edicts: 3,
        leafpvs: vec![0x02, 0, 0, 0, 0],
        edicts: vec![
            ed(1)
                .evv(EVV_ORIGIN, [4.0, 100.0, 0.0])
                .evv(EVV_VIEW_OFS, ofs),
            ed(2).leafs(&[3]),
        ],
        ..base()
    };
    // the unshifted eye straddles plane 0, reaches the empty leaf and picks up
    // the fixture PVS, which has only leaf 1 visible
    assert_eq!(
        diff(&mk([0.0; 3]), |side| drive_visible(side, 1, 2)).value,
        0
    );
    // view_ofs pushes the eye into the solid leaf, where the all-ones fallback
    // makes everything visible
    assert_eq!(
        diff(&mk([-104.0, 0.0, 0.0]), |side| drive_visible(side, 1, 2)).value,
        1
    );
}

// ---------------------------------------------------------------------------
// G. SV_WriteEntitiesToClient -- the non-FTE writer. sv_netsort stays 0 in
// every fixture, so entities are emitted in edict order and the byte stream is
// fully determined.

/// Slot 1 of the oracle string blob and the `.model` offset that reaches it.
const MODEL_SLOT: c_int = 1;
const MODEL_OFS: f32 = 64.0;

fn writeents(fx: &Fx) -> Snap<()> {
    diff(fx, |side| drive_writeents(side, 0, 64000))
}

fn classic_fx() -> Fx {
    Fx {
        clients: vec![cl(0)],
        ..base()
    }
}

fn two_ent_fx() -> Fx {
    Fx {
        num_edicts: 3,
        strings: vec![(MODEL_SLOT, "progs/misc.mdl")],
        leafpvs: vec![0xff; 5],
        edicts: vec![
            ed(1),
            ed(2)
                .ev(EV_MODELINDEX, 1.0)
                .ev(EV_MODEL, MODEL_OFS)
                .leafs(&[1]),
        ],
        clients: vec![cl(0)],
        ..base()
    }
}

/// The two bytes SV_WriteEntitiesToClient always emits for the viewer itself.
const ONLY_VIEWER: &[u8] = &[0x80, 0x01];
/// The viewer plus the second fixture entity, when it survives vis culling.
const VIEWER_AND_SECOND: &[u8] = &[0x80, 0x01, 0x81, 0x04, 0x02, 0x01];

#[test]
fn classic_entity_update_always_emits_the_viewer() {
    let _g = lock();
    let snap = writeents(&classic_fx());
    assert_msg(&snap.msg, ONLY_VIEWER);
    assert_eq!(snap.devstats, (2, 2));
}

#[test]
fn classic_entity_update_packs_the_low_word_bits() {
    let _g = lock();
    let fx = Fx {
        edicts: vec![ed(1)
            .evv(EVV_ORIGIN, [8.0, 16.0, -8.0])
            .evv(EVV_ANGLES, [90.0, 0.0, 0.0])
            .ev(EV_MOVETYPE, MOVETYPE_STEP)
            .ev(EV_COLORMAP, 3.0)
            .ev(EV_SKIN, 4.0)
            .ev(EV_FRAME, 5.0)
            .ev(EV_EFFECTS, 16.0)
            .ev(EV_MODELINDEX, 6.0)],
        ..classic_fx()
    };
    let snap = writeents(&fx);
    assert_msg(
        &snap.msg,
        &[
            0xef, 0x3d, // U_SIGNAL | (bits 0x3d6f) low byte, then bits >> 8
            0x01, // entity 1, no U_LONGENTITY
            0x06, 0x05, 0x03, 0x04, 0x10, // model, frame, colormap, skin, effects
            0x40, 0x00, // origin.x 8 -> Q_rint(64)
            0x40, // angles.x 90 -> Q_rint(90 * 256 / 360) == 64
            0x80, 0x00, // origin.y 16 -> Q_rint(128)
            0xc0, 0xff, // origin.z -8 -> Q_rint(-64)
        ],
    );
    assert_eq!(snap.devstats.0, 15);
}

#[test]
fn classic_origin_delta_window_is_exactly_a_tenth() {
    let _g = lock();
    let mk = |x: f32| Fx {
        edicts: vec![ed(1).evv(EVV_ORIGIN, [x, 0.0, 0.0])],
        ..classic_fx()
    };
    // COMPAT/ADR-010: `miss` is a float compared against the double literal
    // 0.1 (sv_send.c:1302), so 0.1f (0.100000001490116...) falls *outside* the
    // no-send window while the ulp below it falls inside.
    assert_msg(&writeents(&mk(0.1)).msg, &[0x82, 0x01, 0x01, 0x00]);
    let just_under = f32::from_bits(0.1f32.to_bits() - 1);
    assert_msg(&writeents(&mk(just_under)).msg, ONLY_VIEWER);
}

#[test]
fn classic_lerpfinish_rounds_the_remaining_think_time() {
    let _g = lock();
    let mk = |nextthink: f32, si: c_int, sid: c_int, unreliable: u32| Fx {
        edicts: vec![ed(1).ev(EV_NEXTTHINK, nextthink).lerpinfo(si, sid)],
        clients: vec![cl(0).limits(8192, 2048, unreliable)],
        ..base()
    };
    // bits 0x88001: U_MOREBITS | U_LERPFINISH | U_EXTEND1
    assert_msg(
        &writeents(&mk(0.5, 1, 0, 64000)).msg,
        &[0x81, 0x80, 0x08, 0x01, 0x80],
    );
    // ADR-010: Q_rint((nextthink - time) * 255) rounds at the half
    let just_under = f32::from_bits(0.5f32.to_bits() - 1);
    assert_msg(
        &writeents(&mk(just_under, 1, 0, 64000)).msg,
        &[0x81, 0x80, 0x08, 0x01, 0x7f],
    );
    // the *default* interval is only worth the byte above the MTU budget
    assert_msg(
        &writeents(&mk(0.5, 0, 1, 64000)).msg,
        &[0x81, 0x80, 0x08, 0x01, 0x80],
    );
    assert_msg(&writeents(&mk(0.5, 0, 1, 1400)).msg, ONLY_VIEWER);
}

#[test]
fn classic_alpha_and_scale_are_protocol_gated() {
    let _g = lock();
    let mk = |proto: u32| Fx {
        protocol: proto,
        extfields: xf(XF_ALPHA) | xf(XF_SCALE),
        edicts: vec![ed(1).ext1(XF_ALPHA, 0.25).ext1(XF_SCALE, 0.5)],
        ..classic_fx()
    };
    // ENTALPHA_ENCODE(0.25) == 65, ENTSCALE_ENCODE(0.5) == 8
    assert_msg(
        &writeents(&mk(PROTOCOL_FITZQUAKE)).msg,
        &[0x81, 0x80, 0x11, 0x01, 0x41, 0x08],
    );
    // COMPAT: PROTOCOL_NETQUAKE skips the whole extension block (sv_send.c:1359)
    // so alpha and scale are silently dropped.
    assert_msg(&writeents(&mk(PROTOCOL_NETQUAKE)).msg, ONLY_VIEWER);
}

#[test]
fn classic_default_scale_is_only_resent_under_rmq() {
    let _g = lock();
    let mk = |proto: u32| Fx {
        protocol: proto,
        extfields: xf(XF_SCALE),
        edicts: vec![ed(1).ext1(XF_SCALE, 1.0)], // ENTSCALE_ENCODE(1.0) == 16
        ..classic_fx()
    };
    // 666 compares against ENTSCALE_DEFAULT because it never put scale in the
    // baseline; RMQ compares against the (zeroed) baseline and so does send it.
    assert_eq!(ENTSCALE_DEFAULT, 16);
    assert_msg(&writeents(&mk(PROTOCOL_FITZQUAKE)).msg, ONLY_VIEWER);
    assert_msg(
        &writeents(&mk(PROTOCOL_RMQ)).msg,
        &[0x81, 0x80, 0x10, 0x01, 0x10],
    );
}

#[test]
fn classic_entity_update_appends_a_visible_second_entity() {
    let _g = lock();
    let snap = writeents(&two_ent_fx());
    assert_msg(&snap.msg, VIEWER_AND_SECOND);
    assert_eq!(snap.devstats, (6, 6));
}

#[test]
fn classic_entity_update_culls_on_every_visibility_gate() {
    let _g = lock();

    let mut fx = two_ent_fx();
    fx.edicts[1] = ed(2).ev(EV_MODEL, MODEL_OFS).leafs(&[1]); // modelindex 0
    assert_msg(&writeents(&fx).msg, ONLY_VIEWER);

    let mut fx = two_ent_fx();
    fx.edicts[1] = ed(2).ev(EV_MODELINDEX, 1.0).leafs(&[1]); // empty model string
    assert_msg(&writeents(&fx).msg, ONLY_VIEWER);

    let mut fx = two_ent_fx();
    fx.clients = vec![cl(0).limits(8192, 1, 64000)]; // modelindex >= limit_models
    assert_msg(&writeents(&fx).msg, ONLY_VIEWER);

    let mut fx = two_ent_fx();
    fx.edicts[1] = ed(2).ev(EV_MODELINDEX, 1.0).ev(EV_MODEL, MODEL_OFS); // num_leafs 0
    assert_msg(&writeents(&fx).msg, ONLY_VIEWER);

    let mut fx = two_ent_fx();
    fx.leafpvs = vec![0; 5]; // leaf not in the PVS
    assert_msg(&writeents(&fx).msg, ONLY_VIEWER);

    let mut fx = two_ent_fx();
    fx.extfields = xf(XF_NODRAWTOCLIENT);
    let e = fx.edicts[1].clone().ext_edict(XF_NODRAWTOCLIENT, 1);
    fx.edicts[1] = e;
    assert_msg(&writeents(&fx).msg, ONLY_VIEWER);

    let mut fx = two_ent_fx();
    fx.extfields = xf(XF_DRAWONLYTOCLIENT);
    let e = fx.edicts[1].clone().ext_edict(XF_DRAWONLYTOCLIENT, 2);
    fx.edicts[1] = e;
    assert_msg(&writeents(&fx).msg, ONLY_VIEWER);

    // ...but naming the viewer keeps the entity
    let mut fx = two_ent_fx();
    fx.extfields = xf(XF_DRAWONLYTOCLIENT);
    let e = fx.edicts[1].clone().ext_edict(XF_DRAWONLYTOCLIENT, 1);
    fx.edicts[1] = e;
    assert_msg(&writeents(&fx).msg, VIEWER_AND_SECOND);
}

#[test]
fn classic_entity_update_drops_alpha_one_entities_at_the_encode_boundary() {
    let _g = lock();
    let mk = |a: f32| {
        let mut fx = two_ent_fx();
        fx.extfields = xf(XF_ALPHA);
        let e = fx.edicts[1].clone().ext1(XF_ALPHA, a);
        fx.edicts[1] = e;
        fx
    };
    // ADR-010: ENTALPHA_ENCODE rounds a * 254 + 1, so 0.00196 lands on 1
    // (== ENTALPHA_ZERO, culled) and 0.002 lands on 2 (kept).
    assert_msg(&writeents(&mk(0.00196)).msg, ONLY_VIEWER);
    assert_msg(
        &writeents(&mk(0.002)).msg,
        &[0x80, 0x01, 0x81, 0x84, 0x01, 0x02, 0x01, 0x02],
    );
}

#[test]
fn classic_entity_update_skips_vis_culling_at_max_ent_leafs() {
    let _g = lock();
    let mut fx = two_ent_fx();
    fx.leafpvs = vec![0; 5];
    let e = fx.edicts[1].clone().leafs(&[1; 32]);
    fx.edicts[1] = e;
    // num_leafs == MAX_ENT_LEAFS means "in too many leafs to judge", so the
    // entity survives an empty PVS (sv_send.c:1210).
    assert_msg(&writeents(&fx).msg, VIEWER_AND_SECOND);
}

#[test]
fn classic_entity_update_rolls_back_and_rate_limits_the_overflow_print() {
    let _g = lock();
    let mut fx = two_ent_fx();
    fx.msgmax = 3; // origmaxsize; the 64000 overflowsize keeps SZ_Write happy
    let snap = writeents(&fx);
    assert_msg(&snap.msg, ONLY_VIEWER);
    assert_eq!(snap.con, vec!["[con] Packet overflow!\n"]);
    assert_eq!(snap.devstats, (2, 2));

    let mut fx = two_ent_fx();
    fx.msgmax = 3;
    fx.overflowtime = 10.0;
    fx.now = 11.0;
    let snap = writeents(&fx);
    // same rollback, but dev_overflows.packetsize + CONSOLE_RESPAM_TIME has
    // not elapsed so the print is suppressed (sv_send.c:1447)
    assert_msg(&snap.msg, ONLY_VIEWER);
    assert!(snap.con.is_empty());
    assert_eq!(snap.devstats, (2, 2));
}

// ---------------------------------------------------------------------------
// H. SV_PresendClientDatagram: SVFTE_BuildSnapshotForClient followed by
// SVFTE_CalcEntityDeltas. The observable is client->pendingentities_bits, i.e.
// exactly the field mask MSGFTE_DeltaCalcBits produced.

fn fte_fx() -> Fx {
    Fx {
        clients: vec![cl(0).pext2(RD).frames()],
        ..base()
    }
}

fn snapshot_fx() -> Fx {
    Fx {
        num_edicts: 3,
        strings: vec![(MODEL_SLOT, "progs/misc.mdl")],
        edicts: vec![ed(1), ed(2).ev(EV_MODELINDEX, 1.0).ev(EV_MODEL, MODEL_OFS)],
        clients: vec![cl(0).pext2(RD).frames()],
        ..base()
    }
}

/// Safe wrappers for the fixture setters, so the per-case mutation table below
/// stays a list of plain `fn()` values.
///
/// SAFETY (all four): every call passes an edict number inside the fixture's
/// arena and a field/extfield id from the oracle's own enums, which is exactly
/// what the setters bounds-check against.
fn put_ev(num: c_int, field: c_int, v: f32) {
    // SAFETY: the edict number is inside the fixture arena and the field id
    // comes from the oracle's own enum, which is what the setter bounds-checks.
    unsafe { ctest_svsend_set_ev(num, field, v) }
}
fn put_evv(num: c_int, field: c_int, v: [f32; 3]) {
    // SAFETY: the edict number is inside the fixture arena and the field id
    // comes from the oracle's own enum, which is what the setter bounds-checks.
    unsafe { ctest_svsend_set_evv(num, field, v[0], v[1], v[2]) }
}
fn put_ext(num: c_int, which: c_int, v: [f32; 3]) {
    // SAFETY: the edict number is inside the fixture arena and the field id
    // comes from the oracle's own enum, which is what the setter bounds-checks.
    unsafe { ctest_svsend_set_ext(num, which, v[0], v[1], v[2]) }
}
fn put_lerpinfo(num: c_int, sendinterval: c_int) {
    // SAFETY: the edict number is inside the fixture arena and the field id
    // comes from the oracle's own enum, which is what the setter bounds-checks.
    unsafe { ctest_svsend_set_ed(num, 0, sendinterval, 0, 0.0, 0.0, 0.0, 0.0) }
}

/// Snapshot once to establish `previousentities`, clear the reset bits that
/// first frame produced, apply `mutate`, then snapshot again. What is left in
/// `pendingentities_bits` is the delta mask for the mutation alone.
fn presend_delta(fx: &Fx, mutate: &dyn Fn()) -> Snap<()> {
    diff(fx, |side| {
        drive_presend(side, 0);
        // SAFETY: entnum < numpendingentities == qcvm->num_edicts.
        unsafe {
            for e in 0..fx.num_edicts as c_uint {
                ctest_svsend_set_pending(0, e, 0);
            }
        }
        mutate();
        drive_presend(side, 0);
    })
}

#[test]
fn presend_first_snapshot_resets_the_world() {
    let _g = lock();
    let snap = diff(&fte_fx(), |side| drive_presend(side, 0));
    let c = snap.client(0);
    assert_eq!(c.pending, vec![UF_REMOVE, UF_RESET]);
    assert_eq!(c.snapshotresume, 0);
    assert_eq!(c.prev.len(), 1);
    assert_eq!(c.prev[0].0, 1);
    // SV_BuildEntityState's defaults: scale 16, colormod 32/32/32, rest zero
    assert_eq!(c.prev[0].1, St::default());
}

#[test]
fn presend_delta_lights_one_bit_per_changed_field() {
    let _g = lock();
    let fx = Fx {
        extfields: xf(XF_ALPHA)
            | xf(XF_SCALE)
            | xf(XF_COLORMOD)
            | xf(XF_TAG_INDEX)
            | xf(XF_TRAILEFFECTNUM),
        ..fte_fx()
    };
    // every mutation is applied to the viewer edict between two snapshots, so
    // each row is one bit of MSGFTE_DeltaCalcBits in isolation
    let cases: &[(&str, fn(), u32)] = &[
        ("frame", || put_ev(1, EV_FRAME, 5.0), UF_FRAME),
        (
            "origin.x",
            || put_evv(1, EVV_ORIGIN, [16.0, 0.0, 0.0]),
            UF_ORIGINXY,
        ),
        (
            "origin.y",
            || put_evv(1, EVV_ORIGIN, [0.0, 16.0, 0.0]),
            UF_ORIGINXY,
        ),
        (
            "origin.z",
            || put_evv(1, EVV_ORIGIN, [0.0, 0.0, 16.0]),
            UF_ORIGINZ,
        ),
        (
            "angles.x",
            || put_evv(1, EVV_ANGLES, [90.0, 0.0, 0.0]),
            UF_ANGLESXZ,
        ),
        (
            "angles.z",
            || put_evv(1, EVV_ANGLES, [0.0, 0.0, 90.0]),
            UF_ANGLESXZ,
        ),
        (
            "angles.y",
            || put_evv(1, EVV_ANGLES, [0.0, 90.0, 0.0]),
            UF_ANGLESY,
        ),
        ("effects", || put_ev(1, EV_EFFECTS, 16.0), UF_EFFECTS),
        ("modelindex", || put_ev(1, EV_MODELINDEX, 1.0), UF_MODEL),
        ("skin", || put_ev(1, EV_SKIN, 3.0), UF_SKIN),
        ("colormap", || put_ev(1, EV_COLORMAP, 3.0), UF_COLORMAP),
        (
            "eflags via MOVETYPE_STEP",
            || put_ev(1, EV_MOVETYPE, MOVETYPE_STEP),
            UF_FLAGS,
        ),
        ("alpha", || put_ext(1, XF_ALPHA, [0.25, 0.0, 0.0]), UF_ALPHA),
        ("scale", || put_ext(1, XF_SCALE, [0.5, 0.0, 0.0]), UF_SCALE),
        (
            "tagindex",
            || put_ext(1, XF_TAG_INDEX, [3.0, 0.0, 0.0]),
            UF_TAGINFO,
        ),
        (
            "traileffectnum",
            || put_ext(1, XF_TRAILEFFECTNUM, [7.0, 0.0, 0.0]),
            UF_TRAILEFFECT,
        ),
        (
            "colormod",
            || put_ext(1, XF_COLORMOD, [2.0, 1.0, 1.0]),
            UF_COLORMOD,
        ),
        (
            "lerp",
            || put_lerpinfo(1, 1),
            // COMPAT: LERP_BANDAID reuses UF_UNUSED2 for the lerp interval
            // (sv_send.c:229); MSGFTE_WriteEntityUpdate strips it again for
            // any address that is not "LOCAL".
            UF_UNUSED2,
        ),
    ];
    for (name, mutate, want) in cases {
        let snap = presend_delta(&fx, mutate);
        assert_eq!(
            snap.client(0).pending,
            vec![0, *want],
            "delta bit for {name}"
        );
    }
}

#[test]
fn presend_forces_origin_bits_from_the_previous_frames_velocity() {
    let _g = lock();
    let snap = diff(&fte_fx(), |side| {
        let mut out = [0u32; 4];
        drive_presend(side, 0);
        // SAFETY: entnum < numpendingentities; `out` is a live 4-word local.
        let first = unsafe {
            ctest_svsend_set_pending(0, 0, 0);
            ctest_svsend_set_pending(0, 1, 0);
            ctest_svsend_set_evv(1, EVV_VELOCITY, 8.0, 0.0, 0.0);
            drive_presend(side, 0);
            ctest_svsend_pending_copy(0, out.as_mut_ptr(), 4);
            let first = out[1];
            ctest_svsend_set_pending(0, 1, 0);
            first
        };
        drive_presend(side, 0);
        first
    });
    // the frame the entity starts moving on only gets UF_PREDINFO...
    assert_eq!(snap.value, UF_PREDINFO);
    // COMPAT: sv_send.c:187 tests `from->velocity`, not `to`'s, so the forced
    // origin resend arrives one frame late and then never stops.
    assert_eq!(
        snap.client(0).pending,
        vec![0, UF_PREDINFO | UF_ORIGINXY | UF_ORIGINZ]
    );
}

#[test]
fn presend_turns_a_pending_removal_back_into_a_reset2() {
    let _g = lock();
    let snap = diff(&fte_fx(), |side| {
        drive_presend(side, 0);
        // SAFETY: entnum < numpendingentities.
        unsafe {
            ctest_svsend_set_pending(0, 0, 0);
            ctest_svsend_set_pending(0, 1, UF_REMOVE);
            ctest_svsend_set_ev(1, EV_FRAME, 5.0);
        }
        drive_presend(side, 0);
    });
    // sv_send.c:677 -- a removal that turns out to still be visible becomes a
    // UF_RESET2 with the ordinary delta bits ORed on top.
    assert_eq!(snap.client(0).pending, vec![0, UF_RESET2 | UF_FRAME]);
}

#[test]
fn presend_reruns_keep_reissuing_the_full_reset() {
    let _g = lock();
    let snap = diff(&fte_fx(), |side| {
        for _ in 0..3 {
            drive_presend(side, 0);
        }
    });
    // pendingentities_bits[0] keeps UF_REMOVE, which wipes numpreviousentities
    // at the top of every SVFTE_CalcEntityDeltas (sv_send.c:642), so the
    // snapshot never converges until the writer consumes the bits.
    assert_eq!(snap.client(0).pending, vec![UF_REMOVE, UF_RESET]);
    assert_eq!(snap.client(0).prev.len(), 1);
}

#[test]
fn snapshot_does_not_vis_cull_entities_with_no_leafs() {
    let _g = lock();
    let mut fx = snapshot_fx();
    fx.leafpvs = vec![0; 5];
    let snap = diff(&fx, |side| drive_presend(side, 0));
    // COMPAT: SVFTE_BuildSnapshotForClient only runs the PVS test when
    // `parent->num_leafs` is non-zero (sv_send.c:904); SV_WriteEntitiesToClient
    // culls a zero-leaf entity outright (sv_send.c:1204). The two writers
    // disagree and both behaviours are load-bearing.
    let nums: Vec<u32> = snap.client(0).prev.iter().map(|(n, _)| *n).collect();
    assert_eq!(nums, vec![1, 2]);
    assert_eq!(snap.client(0).pending, vec![UF_REMOVE, UF_RESET, UF_RESET]);
}

#[test]
fn presend_flags_a_vanished_entity_for_removal() {
    let _g = lock();
    let snap = diff(&snapshot_fx(), |side| {
        drive_presend(side, 0);
        // SAFETY: entnum < numpendingentities == 3.
        unsafe {
            for e in 0..3 {
                ctest_svsend_set_pending(0, e, 0);
            }
            ctest_svsend_set_ev(2, EV_MODELINDEX, 0.0);
        }
        drive_presend(side, 0);
    });
    assert_eq!(snap.client(0).pending, vec![0, 0, UF_REMOVE]);
    assert_eq!(snap.client(0).prev.len(), 1);
}

#[test]
fn presend_flags_a_new_entity_for_reset() {
    let _g = lock();
    let mut fx = snapshot_fx();
    fx.edicts[1] = ed(2).ev(EV_MODEL, MODEL_OFS); // no modelindex yet
    let snap = diff(&fx, |side| {
        drive_presend(side, 0);
        // SAFETY: entnum < numpendingentities == 3.
        unsafe {
            for e in 0..3 {
                ctest_svsend_set_pending(0, e, 0);
            }
            ctest_svsend_set_ev(2, EV_MODELINDEX, 1.0);
        }
        drive_presend(side, 0);
    });
    assert_eq!(snap.client(0).pending, vec![0, 0, UF_RESET]);
    assert_eq!(snap.client(0).prev.len(), 2);
}

#[test]
fn snapshot_honours_the_client_entity_and_model_limits() {
    let _g = lock();
    let mut fx = snapshot_fx();
    fx.clients = vec![cl(0).pext2(RD).frames().limits(1, 2048, 64000)];
    let snap = diff(&fx, |side| drive_presend(side, 0));
    assert!(snap.client(0).prev.is_empty());
    assert_eq!(snap.client(0).pending, vec![UF_REMOVE, 0, 0]);

    let mut fx = snapshot_fx();
    fx.edicts[0] = ed(1).ev(EV_MODELINDEX, 5.0);
    fx.clients = vec![cl(0).pext2(RD).frames().limits(8192, 3, 64000)];
    let snap = diff(&fx, |side| drive_presend(side, 0));
    // COMPAT: an over-limit modelindex is *zeroed*, not culled (sv_send.c:940),
    // which is the opposite of SV_WriteEntitiesToClient's `continue`.
    assert_eq!(snap.client(0).prev.len(), 2);
    assert_eq!(snap.client(0).prev[0].1.modelindex, 0);
    assert_eq!(snap.client(0).prev[1].1.modelindex, 1);
}

#[test]
fn snapshot_drops_alpha_one_entities_but_never_the_viewer() {
    let _g = lock();
    let mut fx = snapshot_fx();
    fx.extfields = xf(XF_ALPHA);
    fx.edicts[0] = ed(1).ext1(XF_ALPHA, 0.00196);
    let e = fx.edicts[1].clone().ext1(XF_ALPHA, 0.00196);
    fx.edicts[1] = e;
    let snap = diff(&fx, |side| drive_presend(side, 0));
    // the viewer takes the `ent == clent` arm and never reaches the alpha test
    let nums: Vec<u32> = snap.client(0).prev.iter().map(|(n, _)| *n).collect();
    assert_eq!(nums, vec![1]);
    assert_eq!(snap.client(0).prev[0].1.alpha, 1);
}

// ---------------------------------------------------------------------------
// I. SV_SendClientDatagram end to end. The FTE datagram is assembled in a
// function-local static buffer and handed to a NET stub that discards it, so
// the byte *count* (dev_stats.packetsize, set at sv_send.c:801) plus the
// resulting frame/stat state is everything the harness can see.

fn datagram_fx(health: f32, pext2: u32) -> Fx {
    Fx {
        edicts: vec![ed(1).ev(EV_HEALTH, health)],
        clients: vec![cl(0).pext2(pext2).frames()],
        ..base()
    }
}

#[test]
fn send_datagram_writes_stats_then_deltas_and_logs_the_frame() {
    let _g = lock();
    let snap = diff(&datagram_fx(100.0, RDP), |side| {
        drive_presend(side, 0);
        drive_senddatagram(side, 0)
    });
    assert_eq!(snap.value, 1);
    // 3 stat bytes (svcdp_updatestatbyte, STAT_HEALTH, 100) followed by 21
    // entity bytes: svcfte_updateentities + lastmovemessage short + timestamp
    // float + the 0x8000 removal short for edict 0 + the entity-1 short +
    // 4 netbits bytes (0x1828180) + scale + colormod[3] + the eom short.
    assert_eq!(snap.devstats, (24, 24));

    let c = snap.client(0);
    assert_eq!(c.oldstats[STAT_HEALTH], (100, 100.0f32.to_bits(), None));
    // the resend flag moves from the client to the frame that carried it
    assert_eq!(c.rsnum, [0; 8]);
    // UF_RESET is re-armed as UF_RESET2 so new entities are reset twice
    assert_eq!(c.pending, vec![0, UF_RESET2]);
    assert_eq!(c.snapshotresume, 2);

    assert_eq!(c.frames.len(), 1);
    let f = snap.frame(0, 0);
    assert_eq!(f.seq, 0);
    assert_eq!(f.ts, 0.0f32.to_bits());
    assert_eq!(f.rsnum[0], 1 << STAT_HEALTH);
    assert_eq!(f.ents, vec![(0, UF_REMOVE, 0), (1, UF_RESET, 0)]);
}

#[test]
fn send_datagram_promotes_a_non_integral_stat_to_a_float() {
    let _g = lock();
    let snap = diff(&datagram_fx(100.5, RDP), |side| {
        drive_presend(side, 0);
        drive_senddatagram(side, 0)
    });
    // svcfte_updatestatfloat costs 6 bytes where the byte form costs 3
    assert_eq!(snap.devstats.0, 27);
    assert_eq!(
        snap.client(0).oldstats[STAT_HEALTH],
        (100, 100.5f32.to_bits(), None)
    );
}

#[test]
fn send_datagram_falls_back_to_svc_time_without_replacement_deltas() {
    let _g = lock();
    let fx = Fx {
        time: 0.25,
        ..datagram_fx(100.0, 0)
    };
    let snap = diff(&fx, |side| drive_senddatagram(side, 0));
    assert_eq!(snap.value, 1);
    // svc_time + float + the viewer's 2-byte classic update; the clientdata
    // that follows is built into client->datagram and never re-stamps
    // dev_stats.packetsize
    assert_eq!(snap.devstats.0, 7);
    // no frame ring exists on this path
    assert_eq!(snap.client(0).numframes, 0);
}

#[test]
fn send_datagram_prefixes_the_move_sequence_under_predinfo_alone() {
    let _g = lock();
    let fx = Fx {
        time: 0.25,
        clients: vec![cl(0).pext2(PEXT2_PREDINFO).frames().lastmove(0x1234)],
        ..datagram_fx(100.0, 0)
    };
    let snap = diff(&fx, |side| drive_senddatagram(side, 0));
    assert_eq!(snap.value, 1);
    // PEXT2_PREDINFO without PEXT2_REPLACEMENTDELTAS still takes the classic
    // branch, which inserts the two-byte lastmovemessage after svc_time
    // (sv_send.c:1795) -- 7 bytes as above plus 2.
    assert_eq!(snap.devstats.0, 9);
}

#[test]
fn send_datagram_honours_a_preflagged_stat_resend() {
    let _g = lock();
    let mut rsnum = [0u32; 8];
    rsnum[0] = 1 << STAT_ARMOR;
    let fx = Fx {
        clients: vec![cl(0).pext2(RDP).frames().resendstats(rsnum, [0; 8])],
        ..datagram_fx(100.0, 0)
    };
    let snap = diff(&fx, |side| {
        drive_presend(side, 0);
        drive_senddatagram(side, 0)
    });
    // STAT_ARMOR is unchanged (0), but the pre-set resend bit still forces the
    // 3-byte byte-form update (sv_send.c:588), so this is I1 plus 3.
    assert_eq!(snap.devstats.0, 27);
    let c = snap.client(0);
    assert_eq!(c.rsnum, [0; 8]);
    assert_eq!(
        snap.frame(0, 0).rsnum[0],
        (1 << STAT_HEALTH) | (1 << STAT_ARMOR)
    );
}
