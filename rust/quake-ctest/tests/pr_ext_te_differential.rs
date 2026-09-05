//! Differential test: the Rust `quake-capi` temp-entity QuakeC builtins
//! (`rust/quake-capi/src/progs_builtins_te.rs`) vs the original
//! `Quake/pr_ext.c` bodies (`Quake/pr_ext.c:2647-3061`). Rust migration
//! Phase 7, M9f group D.
//!
//! `pr_ext.c` is not in `build.rs`'s `C_SOURCES`, but the `PF_sv_te_*`
//! bodies this file drives are `static` and reached via `#include
//! "pr_ext.c"` in `stubs/pr_ext_ref.c` -- called directly through
//! `ctest_cref_pr_ext_te_run`, with no hand transcription (see that file's
//! "GROUP D" header comment for why this is safe: the statics remain
//! ordinary file-scope names for the rest of that translation unit). `sv` /
//! `svs` are force-renamed to `c_ref_sv` / `c_ref_svs` for that whole TU
//! (`c_ref_prelude.h`), so the C oracle's `PF_sv_te_*` calls write into a
//! storage genuinely independent of `quake-capi`'s plain-named `sv` / `svs`
//! (`rust/quake-capi/src/sv_main.rs`, T6.6) that the `quake_rs_pf_*` port
//! entry points write into -- both are read back through
//! `ctest_pr_ext_te_datagram_*` / `ctest_pr_ext_te_client_datagram_*`
//! (`side` 0 = C oracle, 1 = Rust).
//!
//! # Scope: 19 of 33 builtins are exercised here
//!
//! The 3 `SV_StartParticle`-based builtins (`sv_te_blooddp` / `bloodqw` /
//! `lightningblood`) and the 16 `PF_sv_te_*` network writers all produce wire
//! bytes observable on `sv.datagram` (directly, or via `SV_Multicast`'s
//! collapse onto it) and are compared byte-for-byte here.
//!
//! The 14 `cl_te_*` builtins (`pr_ext.c:3062-3282`, roughly) are pure
//! client-side rendering/audio side effects -- dlights, particle systems,
//! `S_StartSound`, no wire bytes -- and are **not** exercised in this file.
//! They are ported (`progs_builtins_te.rs`'s `quake_rs_pf_cl_te_*`) but not
//! differentially verified; see the M9f group D report.
//!
//! # PVS/PHS fanout scope
//!
//! Every scenario below runs with `maxclients == 0` except the two
//! `particlerain` / `particlesnow` cases: `MULTICAST_PVS_U`
//! (`spike`/`superspike`/`gunshot`) and `MULTICAST_PHS_U` (the other 13
//! `sv_te_*` writers) both collapse onto `sv.datagram` unconditionally
//! before/independent of the empty client loop (`pr_ext.c`'s
//! `SV_Multicast`), so their primary payload is observable with zero
//! clients. `MULTICAST_ALL_U` (particlerain/particlesnow) has no such
//! fallback -- nothing is written anywhere without at least one active,
//! `PEXT2_REPLACEMENTDELTAS`-flagged client, so those two scenarios use
//! `ctest_pr_ext_te_reset`'s `maxclients`/`pext2` parameters and read the
//! client's own datagram back instead of `sv`'s.
//!
//! `Mod_PointInLeaf` / `Mod_LeafPVS` (the PVS_U path's real call, `pr_ext.c`
//! `SV_Multicast`'s `MULTICAST_PVS_U` case) are settable test doubles in
//! `stubs.c`, not renamed by the prelude (they belong to no oracle
//! `C_SOURCES` file), so both sides call the identical shared fake --
//! neutral infrastructure, not part of what this differential compares.
//!
//! # Preserved bug under test: `sv_te_explosion2`'s `OFS_PARM1` reuse
//!
//! `PF_sv_te_explosion2` (`pr_ext.c:2932`) reads `palcount` from the same
//! `OFS_PARM1` slot as `palstart`, not from `OFS_PARM2` -- see
//! `progs_builtins_te.rs`'s module doc. `explosion2_palcount_reuses_palstart_slot`
//! demonstrates this on both sides.

use core::ffi::{c_char, c_int, c_uint};
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

// ---------------------------------------------------------------------------
// progs.h offsets (progs.h's OFS_PARM stride is 3 floats/12 bytes per slot).

const OFS_PARM0: c_int = 4;
const OFS_PARM1: c_int = 7;
const OFS_PARM2: c_int = 10;
const OFS_PARM3: c_int = 13;
const OFS_PARM4: c_int = 16;

// protocol.h / pr_ext.c's local TE_* enum (progs_builtins_te.rs mirrors the
// same values).
const SVC_TEMP_ENTITY: u8 = 23;
const SVC_PARTICLE: u8 = 18;
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
const PEXT2_REPLACEMENTDELTAS: c_uint = 0x0000_0008;

/// `pr_cmds_glue.c:353` `PRBI_OK`.
const PRBI_OK: c_int = 0;
/// `pr_cmds_glue.c:353` `PRBI_ERR_GUARD`.
const PRBI_ERR_GUARD: c_int = 3;
/// `CTEST_GUARD_HOST_ERROR` (`stubs.c`).
const CTEST_GUARD_HOST_ERROR: c_int = 1;

/// `stubs/pr_ext_ref.c`'s `ctest_cref_pr_ext_te_dispatch`'s switch indices.
mod pf {
    pub const SV_TE_BLOODDP: i32 = 60;
    pub const SV_TE_BLOODQW: i32 = 61;
    pub const SV_TE_LIGHTNINGBLOOD: i32 = 62;
    pub const SV_TE_SPIKE: i32 = 63;
    pub const SV_TE_SUPERSPIKE: i32 = 64;
    pub const SV_TE_GUNSHOT: i32 = 65;
    pub const SV_TE_EXPLOSION: i32 = 66;
    pub const SV_TE_TAREXPLOSION: i32 = 67;
    pub const SV_TE_LIGHTNING1: i32 = 68;
    pub const SV_TE_LIGHTNING2: i32 = 69;
    pub const SV_TE_WIZSPIKE: i32 = 70;
    pub const SV_TE_KNIGHTSPIKE: i32 = 71;
    pub const SV_TE_LIGHTNING3: i32 = 72;
    pub const SV_TE_LAVASPLASH: i32 = 73;
    pub const SV_TE_TELEPORT: i32 = 74;
    pub const SV_TE_BEAM: i32 = 75;
    pub const SV_TE_EXPLOSION2: i32 = 76;
    pub const SV_TE_PARTICLERAIN: i32 = 77;
    pub const SV_TE_PARTICLESNOW: i32 = 78;
}

extern "C" {
    // --- fixture (stubs/pr_ext_ref.c, GROUP D block) -----------------------
    fn ctest_pr_ext_te_reset(num_edicts: c_int, maxclients: c_int, pext2: c_uint);
    fn ctest_pr_ext_te_datagram_len(side: c_int) -> c_int;
    fn ctest_pr_ext_te_datagram_byte(side: c_int, i: c_int) -> c_int;
    fn ctest_pr_ext_te_client_datagram_len(side: c_int, idx0based: c_int) -> c_int;
    fn ctest_pr_ext_te_client_datagram_byte(side: c_int, idx0based: c_int, i: c_int) -> c_int;
    fn ctest_pr_ext_te_edict_prog(num: c_int) -> c_int;

    // --- shared fixture setters (stubs.c) -----------------------------------
    fn ctest_pf_set_global_bits(float_ofs: c_int, bits: u32);
    fn ctest_host_error_message() -> *const c_char;

    // --- oracle dispatcher (stubs/pr_ext_ref.c, GROUP D block) --------------
    fn ctest_cref_pr_ext_te_run(which: c_int) -> c_int;

    // --- the Rust port under test (quake-capi/src/progs_builtins_te.rs) ----
    fn quake_rs_pf_sv_te_blooddp(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_te_bloodqw(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_te_lightningblood(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_te_spike(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_te_superspike(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_te_gunshot(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_te_explosion(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_te_tarexplosion(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_te_lightning1(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_te_lightning2(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_te_wizspike(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_te_knightspike(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_te_lightning3(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_te_lavasplash(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_te_teleport(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_te_beam(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_te_explosion2(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_te_particlerain(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sv_te_particlesnow(detail: *mut c_int) -> c_int;
}

// ---------------------------------------------------------------------------
// Safe wrappers.

fn reset(num_edicts: c_int, maxclients: c_int, pext2: c_uint) {
    // SAFETY: no arguments to validate beyond plain ints; the fixture
    // allocates and zeroes its own state (both `c_ref_sv`/`c_ref_svs` and
    // the Rust-owned `sv`/`svs`). Serialised by `TEST_LOCK`.
    unsafe { ctest_pr_ext_te_reset(num_edicts, maxclients, pext2) }
}

fn set_global_f(ofs: c_int, v: f32) {
    // SAFETY: `ofs` is a fixed OFS_* slot inside the globals block.
    unsafe { ctest_pf_set_global_bits(ofs, v.to_bits()) }
}

fn set_global_i(ofs: c_int, v: i32) {
    // SAFETY: as `set_global_f`; an int global is the same 4-byte slot.
    unsafe { ctest_pf_set_global_bits(ofs, v as u32) }
}

fn set_vector(ofs: c_int, v: [f32; 3]) {
    set_global_f(ofs, v[0]);
    set_global_f(ofs + 1, v[1]);
    set_global_f(ofs + 2, v[2]);
}

fn edict_prog(num: c_int) -> i32 {
    // SAFETY: `num` indexes the fixture arena.
    unsafe { ctest_pr_ext_te_edict_prog(num) }
}

fn host_error_message() -> String {
    // SAFETY: the stub returns a NUL-terminated buffer that outlives this.
    unsafe {
        core::ffi::CStr::from_ptr(ctest_host_error_message())
            .to_string_lossy()
            .into_owned()
    }
}

fn datagram_bytes(side: c_int) -> Vec<u8> {
    // SAFETY: `side` is 0 or 1 by test construction.
    unsafe {
        let n = ctest_pr_ext_te_datagram_len(side);
        (0..n)
            .map(|i| ctest_pr_ext_te_datagram_byte(side, i) as u8)
            .collect()
    }
}

fn client_datagram_bytes(side: c_int, idx0based: c_int) -> Vec<u8> {
    // SAFETY: `side`/`idx0based` name a live fixture client by test
    // construction (`CTEST_PR_EXT_TE_CLIENTS == 2`).
    unsafe {
        let n = ctest_pr_ext_te_client_datagram_len(side, idx0based);
        (0..n)
            .map(|i| ctest_pr_ext_te_client_datagram_byte(side, idx0based, i) as u8)
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    C,
    Rust,
}

/// Runs one builtin on one side. `status` is `PRBI_OK`/`Host_Guard`'s 0 on
/// both sides for the non-raising scenarios below; `detail` mirrors
/// `RUST_PF`'s `&detail` and is unused on the C side (always 0).
fn invoke(side: Side, which: i32) -> c_int {
    match side {
        // SAFETY: `which` is one of the GROUP D dispatcher indices above; the
        // C body runs inside `Host_Guard` (`ctest_cref_pr_ext_te_run`), so a
        // `Host_Error` unwinds in a C frame and never longjmps past this
        // call (ADR-009).
        Side::C => unsafe { ctest_cref_pr_ext_te_run(which) },
        Side::Rust => {
            let mut detail: c_int = 0;
            // SAFETY: `detail` is a live, initialised `c_int`; these entry
            // points are status-returning and read the ambient ports of
            // `sv`/`svs`/qcvm the fixture has just reset (ADR-008).
            unsafe {
                match which {
                    pf::SV_TE_BLOODDP => quake_rs_pf_sv_te_blooddp(&mut detail),
                    pf::SV_TE_BLOODQW => quake_rs_pf_sv_te_bloodqw(&mut detail),
                    pf::SV_TE_LIGHTNINGBLOOD => quake_rs_pf_sv_te_lightningblood(&mut detail),
                    pf::SV_TE_SPIKE => quake_rs_pf_sv_te_spike(&mut detail),
                    pf::SV_TE_SUPERSPIKE => quake_rs_pf_sv_te_superspike(&mut detail),
                    pf::SV_TE_GUNSHOT => quake_rs_pf_sv_te_gunshot(&mut detail),
                    pf::SV_TE_EXPLOSION => quake_rs_pf_sv_te_explosion(&mut detail),
                    pf::SV_TE_TAREXPLOSION => quake_rs_pf_sv_te_tarexplosion(&mut detail),
                    pf::SV_TE_LIGHTNING1 => quake_rs_pf_sv_te_lightning1(&mut detail),
                    pf::SV_TE_LIGHTNING2 => quake_rs_pf_sv_te_lightning2(&mut detail),
                    pf::SV_TE_WIZSPIKE => quake_rs_pf_sv_te_wizspike(&mut detail),
                    pf::SV_TE_KNIGHTSPIKE => quake_rs_pf_sv_te_knightspike(&mut detail),
                    pf::SV_TE_LIGHTNING3 => quake_rs_pf_sv_te_lightning3(&mut detail),
                    pf::SV_TE_LAVASPLASH => quake_rs_pf_sv_te_lavasplash(&mut detail),
                    pf::SV_TE_TELEPORT => quake_rs_pf_sv_te_teleport(&mut detail),
                    pf::SV_TE_BEAM => quake_rs_pf_sv_te_beam(&mut detail),
                    pf::SV_TE_EXPLOSION2 => quake_rs_pf_sv_te_explosion2(&mut detail),
                    pf::SV_TE_PARTICLERAIN => quake_rs_pf_sv_te_particlerain(&mut detail),
                    pf::SV_TE_PARTICLESNOW => quake_rs_pf_sv_te_particlesnow(&mut detail),
                    _ => panic!("bad dispatch index {which}"),
                }
            }
        }
    }
}

/// Runs `which` on both sides from a fresh `maxclients == 0` fixture and
/// asserts the resulting `sv.datagram` bytes match exactly. `setup` sets the
/// QC globals for the call (run once per side, since each side owns
/// independent storage).
fn assert_datagram_matches(num_edicts: c_int, which: i32, setup: impl Fn()) -> Vec<u8> {
    reset(num_edicts, 0, 0);
    setup();
    let c_status = invoke(Side::C, which);
    let c_bytes = datagram_bytes(0);

    reset(num_edicts, 0, 0);
    setup();
    let r_status = invoke(Side::Rust, which);
    let r_bytes = datagram_bytes(1);

    assert_eq!(c_status, PRBI_OK, "C oracle: unexpected raise/status");
    assert_eq!(r_status, PRBI_OK, "Rust port: unexpected raise/status");
    assert_eq!(
        c_bytes, r_bytes,
        "sv.datagram bytes differ between C and Rust"
    );
    c_bytes
}

// ===========================================================================
// SV_StartParticle-based (pr_ext.c:2647-2670): SVC_PARTICLE straight onto
// sv.datagram, no multicast involved.
// ===========================================================================

#[test]
fn te_blooddp_matches() {
    let _g = lock();
    let bytes = assert_datagram_matches(2, pf::SV_TE_BLOODDP, || {
        set_vector(OFS_PARM0, [1.0, -2.0, 3.5]);
        set_vector(OFS_PARM1, [0.5, 0.0, -0.5]);
        set_global_f(OFS_PARM2, 30.0);
    });
    assert_eq!(bytes[0], SVC_PARTICLE, "leading opcode");
}

#[test]
fn te_bloodqw_matches() {
    let _g = lock();
    let bytes = assert_datagram_matches(2, pf::SV_TE_BLOODQW, || {
        set_vector(OFS_PARM0, [10.0, 20.0, 30.0]);
        set_global_f(OFS_PARM1, 4.0); // count = 4 * 20 = 80
    });
    assert_eq!(bytes[0], SVC_PARTICLE, "leading opcode");
}

#[test]
fn te_lightningblood_matches() {
    let _g = lock();
    let bytes = assert_datagram_matches(2, pf::SV_TE_LIGHTNINGBLOOD, || {
        set_vector(OFS_PARM0, [-5.0, 0.0, 12.0]);
    });
    assert_eq!(bytes[0], SVC_PARTICLE, "leading opcode");
}

// ===========================================================================
// PVS_U (pr_ext.c:2671-2740): spike/superspike/gunshot.
// ===========================================================================

#[test]
fn te_spike_matches() {
    let _g = lock();
    let bytes = assert_datagram_matches(2, pf::SV_TE_SPIKE, || {
        set_vector(OFS_PARM0, [16.0, -32.0, 64.0]);
    });
    assert_eq!(bytes[0], SVC_TEMP_ENTITY);
    assert_eq!(bytes[1], TE_SPIKE);
}

#[test]
fn te_superspike_matches() {
    let _g = lock();
    let bytes = assert_datagram_matches(2, pf::SV_TE_SUPERSPIKE, || {
        set_vector(OFS_PARM0, [1.0, 1.0, 1.0]);
    });
    assert_eq!(bytes[0], SVC_TEMP_ENTITY);
    assert_eq!(bytes[1], TE_SUPERSPIKE);
}

#[test]
fn te_gunshot_matches() {
    let _g = lock();
    let bytes = assert_datagram_matches(2, pf::SV_TE_GUNSHOT, || {
        set_vector(OFS_PARM0, [0.0, 0.0, 0.0]);
    });
    assert_eq!(bytes[0], SVC_TEMP_ENTITY);
    assert_eq!(bytes[1], TE_GUNSHOT);
}

// ===========================================================================
// PHS_U, single org vector (pr_ext.c:2749-2907).
// ===========================================================================

#[test]
fn te_explosion_matches() {
    let _g = lock();
    let bytes = assert_datagram_matches(2, pf::SV_TE_EXPLOSION, || {
        set_vector(OFS_PARM0, [100.0, -100.0, 0.0]);
    });
    assert_eq!(bytes[0], SVC_TEMP_ENTITY);
    assert_eq!(bytes[1], TE_EXPLOSION);
}

#[test]
fn te_tarexplosion_matches() {
    let _g = lock();
    let bytes = assert_datagram_matches(2, pf::SV_TE_TAREXPLOSION, || {
        set_vector(OFS_PARM0, [1.0, 2.0, 3.0]);
    });
    assert_eq!(bytes[1], TE_TAREXPLOSION);
}

#[test]
fn te_wizspike_matches() {
    let _g = lock();
    let bytes = assert_datagram_matches(2, pf::SV_TE_WIZSPIKE, || {
        set_vector(OFS_PARM0, [1.0, 2.0, 3.0]);
    });
    assert_eq!(bytes[1], TE_WIZSPIKE);
}

#[test]
fn te_knightspike_matches() {
    let _g = lock();
    let bytes = assert_datagram_matches(2, pf::SV_TE_KNIGHTSPIKE, || {
        set_vector(OFS_PARM0, [1.0, 2.0, 3.0]);
    });
    assert_eq!(bytes[1], TE_KNIGHTSPIKE);
}

#[test]
fn te_lavasplash_matches() {
    let _g = lock();
    let bytes = assert_datagram_matches(2, pf::SV_TE_LAVASPLASH, || {
        set_vector(OFS_PARM0, [1.0, 2.0, 3.0]);
    });
    assert_eq!(bytes[1], TE_LAVASPLASH);
}

// ===========================================================================
// lightning1/lightning2/lightning3: edict num + start + end, written
// directly onto sv.datagram (pr_ext.c:2791-2897 odd entries).
// ===========================================================================

#[test]
fn te_lightning1_matches() {
    let _g = lock();
    let bytes = assert_datagram_matches(4, pf::SV_TE_LIGHTNING1, || {
        set_global_i(OFS_PARM0, edict_prog(2));
        set_vector(OFS_PARM1, [0.0, 0.0, 0.0]);
        set_vector(OFS_PARM2, [0.0, 0.0, 64.0]);
    });
    assert_eq!(bytes[0], SVC_TEMP_ENTITY);
    assert_eq!(bytes[1], TE_LIGHTNING1);
}

#[test]
fn te_lightning2_matches() {
    let _g = lock();
    let bytes = assert_datagram_matches(4, pf::SV_TE_LIGHTNING2, || {
        set_global_i(OFS_PARM0, edict_prog(2));
        set_vector(OFS_PARM1, [1.0, 2.0, 3.0]);
        set_vector(OFS_PARM2, [4.0, 5.0, 6.0]);
    });
    assert_eq!(bytes[1], TE_LIGHTNING2);
}

#[test]
fn te_lightning3_matches() {
    let _g = lock();
    let bytes = assert_datagram_matches(4, pf::SV_TE_LIGHTNING3, || {
        set_global_i(OFS_PARM0, edict_prog(2));
        set_vector(OFS_PARM1, [1.0, 2.0, 3.0]);
        set_vector(OFS_PARM2, [4.0, 5.0, 6.0]);
    });
    assert_eq!(bytes[1], TE_LIGHTNING3);
}

#[test]
fn te_lightning1_invalid_edict_raises_on_both_sides() {
    let _g = lock();

    reset(4, 0, 0);
    set_global_i(OFS_PARM0, 999_999); // not a valid EDICT_TO_PROG value
    set_vector(OFS_PARM1, [0.0, 0.0, 0.0]);
    set_vector(OFS_PARM2, [0.0, 0.0, 0.0]);
    let c_status = invoke(Side::C, pf::SV_TE_LIGHTNING1);
    // The C dispatcher (`ctest_cref_pr_ext_te_run`) returns Host_Guard's raw
    // 0/1 convention directly, not the PRBI_OK/PRBI_ERR_GUARD wrapper the
    // Rust port's own entry points use below -- matching
    // progs_builtins_sv_differential.rs's `Side::C` arm.
    assert_eq!(c_status, CTEST_GUARD_HOST_ERROR, "C oracle must raise");
    let c_msg = host_error_message();

    reset(4, 0, 0);
    set_global_i(OFS_PARM0, 999_999);
    set_vector(OFS_PARM1, [0.0, 0.0, 0.0]);
    set_vector(OFS_PARM2, [0.0, 0.0, 0.0]);
    let mut detail: c_int = 0;
    // SAFETY: same contract as `invoke`'s Rust arm.
    let r_status = unsafe { quake_rs_pf_sv_te_lightning1(&mut detail) };
    assert_eq!(r_status, PRBI_ERR_GUARD, "Rust port must raise");
    assert_eq!(detail, CTEST_GUARD_HOST_ERROR);
    let r_msg = host_error_message();

    assert_eq!(c_msg, r_msg, "Host_Error message must match");
}

// ===========================================================================
// teleport / beam / explosion2: written to sv.multicast, then PHS_U's
// requireext2==0 branch unconditionally copies it onto sv.datagram
// (pr_ext.c:2916-2977).
// ===========================================================================

#[test]
fn te_teleport_matches() {
    let _g = lock();
    let bytes = assert_datagram_matches(2, pf::SV_TE_TELEPORT, || {
        set_vector(OFS_PARM0, [8.0, 16.0, 24.0]);
    });
    assert_eq!(bytes[0], SVC_TEMP_ENTITY);
    assert_eq!(bytes[1], TE_TELEPORT);
}

#[test]
fn te_beam_matches() {
    let _g = lock();
    let bytes = assert_datagram_matches(4, pf::SV_TE_BEAM, || {
        set_global_i(OFS_PARM0, edict_prog(3));
        set_vector(OFS_PARM1, [0.0, 0.0, 0.0]);
        set_vector(OFS_PARM2, [100.0, 0.0, 0.0]);
    });
    assert_eq!(bytes[0], SVC_TEMP_ENTITY);
    assert_eq!(bytes[1], TE_BEAM);
}

#[test]
fn te_explosion2_matches() {
    let _g = lock();
    let bytes = assert_datagram_matches(2, pf::SV_TE_EXPLOSION2, || {
        set_vector(OFS_PARM0, [1.0, 2.0, 3.0]);
        set_global_f(OFS_PARM1, 70.0);
        set_global_f(OFS_PARM2, 8.0); // never read -- see next test
    });
    assert_eq!(bytes[0], SVC_TEMP_ENTITY);
    assert_eq!(bytes[1], TE_EXPLOSION2);
}

/// Preserved bug (`progs_builtins_te.rs`'s module doc): `palcount` reads
/// `OFS_PARM1` again, not `OFS_PARM2`. Demonstrated by varying only
/// `OFS_PARM2` and observing the trailing two bytes stay identical (and
/// equal to `OFS_PARM1`'s truncated value) on both sides.
#[test]
fn explosion2_palcount_reuses_palstart_slot() {
    let _g = lock();

    let run = |ofs_parm2: f32| {
        reset(2, 0, 0);
        set_vector(OFS_PARM0, [0.0, 0.0, 0.0]);
        set_global_f(OFS_PARM1, 42.0);
        set_global_f(OFS_PARM2, ofs_parm2);
        assert_eq!(invoke(Side::C, pf::SV_TE_EXPLOSION2), PRBI_OK);
        datagram_bytes(0)
    };

    let a = run(1.0);
    let b = run(200.0);
    assert_eq!(
        a, b,
        "OFS_PARM2 must not affect the wire bytes (preserved bug)"
    );
    let n = a.len();
    assert_eq!(a[n - 2], 42, "palstart == OFS_PARM1 truncated to int");
    assert_eq!(
        a[n - 1],
        42,
        "palcount == OFS_PARM1 truncated to int (reused)"
    );

    // Same bug on the Rust side, and identical to the C oracle's bytes.
    reset(2, 0, 0);
    set_vector(OFS_PARM0, [0.0, 0.0, 0.0]);
    set_global_f(OFS_PARM1, 42.0);
    set_global_f(OFS_PARM2, 200.0);
    let mut detail: c_int = 0;
    // SAFETY: same contract as `invoke`'s Rust arm.
    let status = unsafe { quake_rs_pf_sv_te_explosion2(&mut detail) };
    assert_eq!(status, PRBI_OK);
    assert_eq!(
        datagram_bytes(1),
        a,
        "Rust port must reproduce the same bug"
    );
}

// ===========================================================================
// particlerain / particlesnow: MULTICAST_ALL_U + PEXT2_REPLACEMENTDELTAS --
// the only two scenarios needing an active, ext2-flagged client (pr_ext.c:
// 2987-3044).
// ===========================================================================

fn setup_particle_weather(min: [f32; 3], max: [f32; 3], vel: [f32; 3], count: f32, colour: f32) {
    set_vector(OFS_PARM0, min);
    set_vector(OFS_PARM1, max);
    set_vector(OFS_PARM2, vel);
    set_global_f(OFS_PARM3, count);
    set_global_f(OFS_PARM4, colour);
}

#[test]
fn te_particlerain_matches_active_ext2_client() {
    let _g = lock();

    reset(2, 1, PEXT2_REPLACEMENTDELTAS);
    setup_particle_weather(
        [-100.0, -100.0, 0.0],
        [100.0, 100.0, 0.0],
        [0.0, 0.0, -100.0],
        50.0,
        12.0,
    );
    let c_status = invoke(Side::C, pf::SV_TE_PARTICLERAIN);
    let c_bytes = client_datagram_bytes(0, 0);

    reset(2, 1, PEXT2_REPLACEMENTDELTAS);
    setup_particle_weather(
        [-100.0, -100.0, 0.0],
        [100.0, 100.0, 0.0],
        [0.0, 0.0, -100.0],
        50.0,
        12.0,
    );
    let r_status = invoke(Side::Rust, pf::SV_TE_PARTICLERAIN);
    let r_bytes = client_datagram_bytes(1, 0);

    assert_eq!(c_status, PRBI_OK);
    assert_eq!(r_status, PRBI_OK);
    assert_eq!(
        c_bytes, r_bytes,
        "client datagram bytes differ between C and Rust"
    );
    assert_eq!(c_bytes[0], SVC_TEMP_ENTITY);
    assert_eq!(c_bytes[1], TEDP_PARTICLERAIN);
    assert!(
        datagram_bytes(0).is_empty(),
        "C: sv.datagram itself must stay untouched"
    );
    assert!(
        datagram_bytes(1).is_empty(),
        "Rust: sv.datagram itself must stay untouched"
    );
}

#[test]
fn te_particlesnow_matches_active_ext2_client() {
    let _g = lock();

    reset(2, 1, PEXT2_REPLACEMENTDELTAS);
    setup_particle_weather(
        [-50.0, -50.0, 0.0],
        [50.0, 50.0, 0.0],
        [0.0, 0.0, -20.0],
        30.0,
        6.0,
    );
    let c_status = invoke(Side::C, pf::SV_TE_PARTICLESNOW);
    let c_bytes = client_datagram_bytes(0, 0);

    reset(2, 1, PEXT2_REPLACEMENTDELTAS);
    setup_particle_weather(
        [-50.0, -50.0, 0.0],
        [50.0, 50.0, 0.0],
        [0.0, 0.0, -20.0],
        30.0,
        6.0,
    );
    let r_status = invoke(Side::Rust, pf::SV_TE_PARTICLESNOW);
    let r_bytes = client_datagram_bytes(1, 0);

    assert_eq!(c_status, PRBI_OK);
    assert_eq!(r_status, PRBI_OK);
    assert_eq!(
        c_bytes, r_bytes,
        "client datagram bytes differ between C and Rust"
    );
    assert_eq!(c_bytes[1], TEDP_PARTICLESNOW);
}

#[test]
fn te_particlerain_zero_active_clients_writes_nothing() {
    let _g = lock();

    for side in [Side::C, Side::Rust] {
        reset(2, 0, PEXT2_REPLACEMENTDELTAS);
        setup_particle_weather([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0], 10.0, 1.0);
        let status = invoke(side, pf::SV_TE_PARTICLERAIN);
        assert_eq!(status, PRBI_OK);
        let raw = if side == Side::C { 0 } else { 1 };
        assert!(
            datagram_bytes(raw).is_empty(),
            "{side:?}: MULTICAST_ALL_U has no broadcast-when-empty fallback"
        );
    }
}

#[test]
fn te_particlerain_count_below_one_is_a_no_op() {
    let _g = lock();

    for side in [Side::C, Side::Rust] {
        reset(2, 1, PEXT2_REPLACEMENTDELTAS);
        setup_particle_weather([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0], 0.5, 1.0);
        let status = invoke(side, pf::SV_TE_PARTICLERAIN);
        assert_eq!(status, PRBI_OK);
        let raw = if side == Side::C { 0 } else { 1 };
        assert!(
            client_datagram_bytes(raw, 0).is_empty(),
            "{side:?}: count < 1 must early-return before any write"
        );
    }
}
