//! Differential test: the Rust `quake-capi` message QuakeC builtins
//! (`rust/quake-capi/src/progs_builtins_sv_msg.rs`) vs the original
//! `Quake/pr_cmds.c` / `Quake/pr_ext.c` bodies. Rust migration Phase 7, M5
//! wave 2 Group D (`PF_stuffcmd`, `PF_bprint`, `PF_sprint`, `PF_centerprint`,
//! the six extended message writers).
//!
//! `pr_cmds.c` / `pr_ext.c` are not in `build.rs`'s `C_SOURCES` (same as wave
//! 1), so the oracle is a set of statement-for-statement C transcriptions in
//! `stubs/pf_msg_ref.c` (`ctest_msgref_pf_*_run`), independent of the
//! `PRBI_MsgGlue_*` functions the port itself calls (see that file's header
//! comment for why circularity is avoided). `MSG_Write*`, `SZ_Write`,
//! `PR_GetString`, `PR_SetEngineString`, `LOC_*` are the real, compiled
//! primitives on both sides (`c_ref_prelude.h`'s rename macros), so only each
//! builtin's own control flow is hand-transcribed.
//!
//! # Raise topology (ADR-009), three distinct shapes -- do not conflate them
//!
//! 1. **Guarded seams** (`PF_stuffcmd`'s "Parm 0 not a client", `G_STRING`):
//!    the port returns `PRBI_ERR_GUARD` (3) with `detail ==
//!    CTEST_GUARD_HOST_ERROR` (1), and a real `Host_Error` ran underneath
//!    (inside `PRBI_MsgGlue_*`'s `Host_Guard` call), so
//!    `ctest_host_error_message()` is populated identically on both sides.
//! 2. **`write_dest`'s own raises** (`MSG_ONE` "not a client" / bad
//!    destination): the port never calls `Host_Error` for these -- it
//!    returns `PRBI_ERR_WRITEDEST_NOT_CLIENT` (5) / `_BAD_DEST` (6) directly
//!    (`progs_builtins_sv_msg.rs`'s module doc: "`WriteDest()` itself is not
//!    called from Rust"). The C oracle's `ctest_msgref_oracle_writedest`
//!    *does* call `Host_Error`, so its message is real but has **no Rust
//!    counterpart to compare against** -- only the status code and the
//!    absence of any wire/console side effect are checked on the Rust side.
//! 3. **Soft warnings** (`PF_sprint` / `PF_centerprint`'s "tried to sprint to
//!    a non-client"): neither side raises; both print through `Con_Printf`
//!    and return normally. Compared via the console log, not `Outcome`.
//!
//! # Preserved bugs under test (`progs_builtins_sv_msg.rs`'s own doc)
//!
//! 1. `PF_WriteInt` (and `"WriteUInt"`'s shared table slot) writes an 8-byte
//!    double, not a 4-byte int32.
//! 2. Every extended writer reads its payload from `OFS_PARM0` -- the same
//!    slot `write_dest` already consumed for the destination selector -- not
//!    a distinct payload argument slot. For the four-byte writers
//!    (`WriteFloat`, `WriteInt`) this makes the payload literally equal to
//!    the destination float's bits; for the eight-byte writers
//!    (`WriteDouble`, `WriteInt64`, `WriteUInt64`, all `*(T *)
//!    &qcvm->globals[OFS_PARM0]` pointer casts per `progs.h:166-170`) the
//!    payload's low 4 bytes are the destination float and the high 4 bytes
//!    spill in from `globals[OFS_PARM0 + 1]` -- the padding word of
//!    `OFS_PARM0`'s own 3-word slot (`progs.h`'s OFS_PARM stride is 3), NOT
//!    `OFS_PARM1` (`globals[7]`), which sits two words further out and is
//!    never read by any of these builtins.
//! 3. `"WriteUInt"` is `PF_WriteInt` under a different table entry -- there
//!    is no separate `quake_rs_pf_WriteUInt` FFI symbol to call, so this is
//!    documented, not independently exercised here.
//!
//! # A structural finding NOT covered by a failing test (documented, see the
//! report): `PF_WriteString2`'s two raising reads run in the opposite order
//! between the original C (`G_STRING` then `WriteDest()`,
//! `pr_ext.c:2589-2590`) and the port (`write_dest` then `GetString`,
//! `progs_builtins_sv_msg.rs`'s `quake_rs_pf_WriteString2`). This is
//! unobservable through any float `OFS_PARM0` value a test can construct:
//! `PR_GetString` (`pr_edict_arena.c:307-326`) only raises for a *negative*
//! handle landing on a null known-string slot, and no destination-selector
//! float (whose bit pattern must decode as a small int to mean anything)
//! reinterprets as such a handle. Recorded as a risk, not exercised.

use core::ffi::{c_char, c_int, c_longlong, c_ulonglong, CStr};
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

// server.h:308-313.
const MSG_BROADCAST: c_int = 0;
const MSG_ONE: c_int = 1;
const MSG_ALL: c_int = 2;
const MSG_INIT: c_int = 3;
const MSG_EXT_MULTICAST: c_int = 4;
const MSG_EXT_ENTITY: c_int = 5;

// protocol.h.
const SVC_PRINT: u8 = 8;
const SVC_STUFFTEXT: u8 = 9;
const SVC_CENTERPRINT: u8 = 26;

/// `pr_cmds_glue.c:353` `PRBI_OK`.
const PRBI_OK: c_int = 0;
/// `pr_cmds_glue.c:353` `PRBI_ERR_GUARD`.
const PRBI_ERR_GUARD: c_int = 3;
/// `pr_cmds_glue.c:353` `PRBI_ERR_WRITEDEST_NOT_CLIENT`.
const PRBI_ERR_WRITEDEST_NOT_CLIENT: c_int = 5;
/// `pr_cmds_glue.c:353` `PRBI_ERR_WRITEDEST_BAD_DEST`.
const PRBI_ERR_WRITEDEST_BAD_DEST: c_int = 6;
/// `CTEST_GUARD_HOST_ERROR` (`stubs.c`).
const CTEST_GUARD_HOST_ERROR: c_int = 1;

extern "C" {
    // --- shared fixture (world/edict arena + qcvm; message builtins do not
    // touch physics, but need a live progs VM for G_STRING/G_EDICTNUM). ----
    fn ctest_phys_reset(
        num_edicts: c_int,
        maxclients: c_int,
        frametime: f64,
        vmtime: f64,
        physics_mode: c_int,
    );
    fn ctest_pf_edict_prog(num: c_int) -> c_int;
    fn ctest_pf_set_global_bits(float_ofs: c_int, bits: u32);

    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(i: c_int) -> *const c_char;
    fn ctest_host_error_message() -> *const c_char;

    // --- Group D fixture (stubs/pf_msg_ref.c) ------------------------------
    fn ctest_msgref_reset();
    fn ctest_msgref_set_maxclients(n: c_int);
    fn ctest_msgref_set_client_active(idx1based: c_int, active: c_int);
    fn ctest_msgref_set_argc(n: c_int);
    fn ctest_msgref_set_msg_entity(edict_prog_num: c_int);
    fn ctest_msgref_intern_string(s: *const c_char) -> c_int;
    fn ctest_msgref_client_len(idx1based: c_int) -> c_int;
    fn ctest_msgref_client_byte(idx1based: c_int, off: c_int) -> c_int;
    fn ctest_msgref_dest_len(dest: c_int, entnum: c_int) -> c_int;
    fn ctest_msgref_dest_byte(dest: c_int, entnum: c_int, off: c_int) -> c_int;

    // --- oracle (stubs/pf_msg_ref.c, job 2 -- independent of PRBI_MsgGlue_*)
    fn ctest_try_host(
        f: unsafe extern "C" fn(*mut core::ffi::c_void),
        arg: *mut core::ffi::c_void,
    ) -> c_int;
    fn ctest_msgref_pf_stuffcmd_run(unused: *mut core::ffi::c_void);
    fn ctest_msgref_pf_bprint_run(unused: *mut core::ffi::c_void);
    fn ctest_msgref_pf_sprint_run(unused: *mut core::ffi::c_void);
    fn ctest_msgref_pf_centerprint_run(unused: *mut core::ffi::c_void);
    fn ctest_msgref_pf_writefloat_run(unused: *mut core::ffi::c_void);
    fn ctest_msgref_pf_writedouble_run(unused: *mut core::ffi::c_void);
    fn ctest_msgref_pf_writeint_run(unused: *mut core::ffi::c_void);
    fn ctest_msgref_pf_writeint64_run(unused: *mut core::ffi::c_void);
    fn ctest_msgref_pf_writeuint64_run(unused: *mut core::ffi::c_void);
    fn ctest_msgref_pf_writestring2_run(unused: *mut core::ffi::c_void);

    // --- port (rust/quake-capi/src/progs_builtins_sv_msg.rs) --------------
    fn quake_rs_pf_stuffcmd(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_bprint(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_sprint(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_centerprint(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_WriteFloat(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_WriteDouble(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_WriteInt(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_WriteInt64(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_WriteUInt64(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_WriteString2(detail: *mut c_int) -> c_int;
}

// ---------------------------------------------------------------------------
// Small typed wrappers.

fn set_global_f(ofs: c_int, v: f32) {
    // SAFETY: `ofs` is a fixed OFS_* slot inside the globals block.
    unsafe { ctest_pf_set_global_bits(ofs, v.to_bits()) }
}

fn set_global_i(ofs: c_int, v: i32) {
    // SAFETY: as `set_global_f`; an int global is the same 4-byte slot.
    unsafe { ctest_pf_set_global_bits(ofs, v as u32) }
}

fn edict_prog(num: c_int) -> i32 {
    // SAFETY: `num` indexes the fixture arena.
    unsafe { ctest_pf_edict_prog(num) }
}

/// Interns `s` and sets QC variadic argument `idx` (0-based, `OFS_PARM0 +
/// idx*3`) to point at it -- matches `pf_msg_ref.c`'s `OFS_PARM0 + idx * 3`
/// stride for `PF_GetStringArg`/`PF_VarString`.
fn set_string_arg(idx: c_int, s: &CStr) {
    // SAFETY: `s` is NUL-terminated; the helper only reads it and returns a
    // handle into the engine's own string arena.
    let handle = unsafe { ctest_msgref_intern_string(s.as_ptr()) };
    set_global_i(OFS_PARM0 + idx * 3, handle);
}

fn con_log() -> Vec<String> {
    // SAFETY: plain counter read.
    let n = unsafe { ctest_con_log_len() };
    (0..n)
        .map(|i| {
            // SAFETY: `i < n`; NUL-terminated buffer outlives the borrow.
            unsafe { CStr::from_ptr(ctest_con_log_get(i)) }
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

fn host_error_message() -> String {
    // SAFETY: the stub returns a NUL-terminated buffer that outlives this.
    unsafe { CStr::from_ptr(ctest_host_error_message()) }
        .to_string_lossy()
        .into_owned()
}

fn client_bytes(idx1based: c_int) -> Vec<u8> {
    // SAFETY: `idx1based` is within CTEST_MSGREF_MAX_CLIENTS by test
    // construction.
    unsafe {
        let n = ctest_msgref_client_len(idx1based);
        (0..n)
            .map(|i| ctest_msgref_client_byte(idx1based, i) as u8)
            .collect()
    }
}

fn dest_bytes(dest: c_int, entnum: c_int) -> Vec<u8> {
    // SAFETY: `dest`/`entnum` name a valid PRBI_MsgWriteDest case by test
    // construction.
    unsafe {
        let n = ctest_msgref_dest_len(dest, entnum);
        (0..n)
            .map(|i| ctest_msgref_dest_byte(dest, entnum, i) as u8)
            .collect()
    }
}

/// Fixture reset shared by every test: a live qcvm/edict arena (message
/// builtins need one for `G_STRING`/`G_EDICTNUM`, not for physics) plus
/// Group D's private client/dest sizebuf_t fixture.
fn setup(num_edicts: c_int, maxclients: c_int, active: &[c_int]) {
    // SAFETY: plain fixture setters; the file mutex serializes all callers.
    unsafe {
        ctest_phys_reset(num_edicts, 0, 0.05, 0.0, -1);
        ctest_msgref_reset();
        ctest_msgref_set_maxclients(maxclients);
        for &idx in active {
            ctest_msgref_set_client_active(idx, 1);
        }
    }
}

// ===========================================================================
// PF_stuffcmd (pr_cmds.c:931)
// ===========================================================================

#[test]
fn stuffcmd_writes_stufftext_to_named_client() {
    let _g = lock();

    for (side_name, run) in [
        ("c", RunKind::COracle(ctest_msgref_pf_stuffcmd_run)),
        ("rust", RunKind::Rust(quake_rs_pf_stuffcmd)),
    ] {
        setup(4, 2, &[1, 2]);
        set_global_i(OFS_PARM0, edict_prog(1));
        set_string_arg(1, c"echo hi\n");

        let outcome = run.call();
        assert!(!outcome.raised, "{side_name}: unexpected raise");
        assert_eq!(
            client_bytes(1),
            [&[SVC_STUFFTEXT][..], b"echo hi\n", &[0]].concat(),
            "{side_name}: stufftext wire bytes"
        );
        assert!(
            client_bytes(2).is_empty(),
            "{side_name}: only the named client should receive the command"
        );
    }
}

#[test]
fn stuffcmd_invalid_client_raises_and_writes_nothing() {
    let _g = lock();

    for (side_name, run) in [
        ("c", RunKind::COracle(ctest_msgref_pf_stuffcmd_run)),
        ("rust", RunKind::Rust(quake_rs_pf_stuffcmd)),
    ] {
        // edict 3 exists (arena has 4 edicts) but maxclients is 1, so entnum
        // 3 is a valid edict, not a valid client.
        setup(4, 1, &[1]);
        set_global_i(OFS_PARM0, edict_prog(3));
        set_string_arg(1, c"echo hi\n");

        let outcome = run.call();
        assert!(outcome.raised, "{side_name}: must raise");
        assert_eq!(outcome.message.as_deref(), Some("Parm 0 not a client"));
        assert!(client_bytes(1).is_empty(), "{side_name}: nothing written");
    }
}

// ===========================================================================
// PF_bprint (pr_cmds.c:396)
// ===========================================================================

#[test]
fn bprint_broadcasts_to_active_clients_only() {
    let _g = lock();

    for (side_name, run) in [
        ("c", RunKind::COracle(ctest_msgref_pf_bprint_run)),
        ("rust", RunKind::Rust(quake_rs_pf_bprint)),
    ] {
        setup(2, 3, &[1, 3]); // client 2 stays inactive
                              // SAFETY: plain C fixture setter; sets the QC argc counter for this call.
        unsafe { ctest_msgref_set_argc(1) };
        set_string_arg(0, c"gg\n");

        let outcome = run.call();
        assert!(!outcome.raised, "{side_name}: unexpected raise");
        let want = [&[SVC_PRINT][..], b"gg\n", &[0]].concat();
        assert_eq!(client_bytes(1), want, "{side_name}: client 1");
        assert!(client_bytes(2).is_empty(), "{side_name}: inactive client 2");
        assert_eq!(client_bytes(3), want, "{side_name}: client 3");
    }
}

#[test]
fn bprint_concatenates_multiple_args() {
    let _g = lock();

    for (side_name, run) in [
        ("c", RunKind::COracle(ctest_msgref_pf_bprint_run)),
        ("rust", RunKind::Rust(quake_rs_pf_bprint)),
    ] {
        setup(2, 1, &[1]);
        // SAFETY: plain C fixture setter; sets the QC argc counter for this call.
        unsafe { ctest_msgref_set_argc(2) };
        set_string_arg(0, c"a");
        set_string_arg(1, c"b\n");

        let outcome = run.call();
        assert!(!outcome.raised, "{side_name}: unexpected raise");
        assert_eq!(
            client_bytes(1),
            [&[SVC_PRINT][..], b"ab\n", &[0]].concat(),
            "{side_name}: concatenated args"
        );
    }
}

// ===========================================================================
// PF_sprint (pr_cmds.c:413) / PF_centerprint (pr_cmds.c:443)
// ===========================================================================

#[test]
fn sprint_writes_svc_print_to_named_client() {
    let _g = lock();

    for (side_name, run) in [
        ("c", RunKind::COracle(ctest_msgref_pf_sprint_run)),
        ("rust", RunKind::Rust(quake_rs_pf_sprint)),
    ] {
        setup(4, 2, &[1, 2]);
        set_global_i(OFS_PARM0, edict_prog(2));
        // SAFETY: plain C fixture setter; sets the QC argc counter for this call.
        unsafe { ctest_msgref_set_argc(2) };
        set_string_arg(1, c"hi\n");

        let outcome = run.call();
        assert!(!outcome.raised, "{side_name}: unexpected raise");
        assert_eq!(
            client_bytes(2),
            [&[SVC_PRINT][..], b"hi\n", &[0]].concat(),
            "{side_name}: sprint wire bytes"
        );
        assert!(
            client_bytes(1).is_empty(),
            "{side_name}: only client 2 targeted"
        );
    }
}

#[test]
fn centerprint_writes_svc_centerprint_to_named_client() {
    let _g = lock();

    for (side_name, run) in [
        ("c", RunKind::COracle(ctest_msgref_pf_centerprint_run)),
        ("rust", RunKind::Rust(quake_rs_pf_centerprint)),
    ] {
        setup(4, 1, &[1]);
        set_global_i(OFS_PARM0, edict_prog(1));
        // SAFETY: plain C fixture setter; sets the QC argc counter for this call.
        unsafe { ctest_msgref_set_argc(2) };
        set_string_arg(1, c"congrats\n");

        let outcome = run.call();
        assert!(!outcome.raised, "{side_name}: unexpected raise");
        assert_eq!(
            client_bytes(1),
            [&[SVC_CENTERPRINT][..], b"congrats\n", &[0]].concat(),
            "{side_name}: centerprint wire bytes"
        );
    }
}

#[test]
fn sprint_invalid_client_is_soft_warning_not_raise() {
    let _g = lock();

    for (side_name, run) in [
        ("c", RunKind::COracle(ctest_msgref_pf_sprint_run)),
        ("rust", RunKind::Rust(quake_rs_pf_sprint)),
    ] {
        setup(4, 1, &[1]);
        set_global_i(OFS_PARM0, edict_prog(3)); // valid edict, not a client
                                                // SAFETY: plain C fixture setter; sets the QC argc counter for this call.
        unsafe { ctest_msgref_set_argc(2) };
        set_string_arg(1, c"hi\n");

        let outcome = run.call();
        assert!(!outcome.raised, "{side_name}: soft warning must not raise");
        assert!(client_bytes(1).is_empty(), "{side_name}: nothing written");
        // The log's *last* entry, not the whole vector: PR_SetEngineString's
        // one-time "realloc'ing for 256 slots" dprint (pr_edict_arena.c) can
        // land anywhere in the log depending on how many strings earlier
        // tests in this same process happened to intern first -- it is a
        // real, shared, cross-process-lifetime side effect, not part of
        // either side's PF_sprint behaviour.
        assert_eq!(
            con_log().last().map(String::as_str),
            Some("[con] tried to sprint to a non-client\n"),
            "{side_name}: warning text"
        );
    }
}

#[test]
fn centerprint_invalid_client_reuses_sprint_warning_text() {
    let _g = lock();

    // COMPAT: pr_cmds.c:454's "tried to sprint..." text is copy-pasted from
    // PF_sprint, not "tried to centerprint..." -- both sides must reproduce
    // the identical (wrong) literal.
    for (side_name, run) in [
        ("c", RunKind::COracle(ctest_msgref_pf_centerprint_run)),
        ("rust", RunKind::Rust(quake_rs_pf_centerprint)),
    ] {
        setup(4, 1, &[1]);
        set_global_i(OFS_PARM0, edict_prog(3));
        // SAFETY: plain C fixture setter; sets the QC argc counter for this call.
        unsafe { ctest_msgref_set_argc(2) };
        set_string_arg(1, c"hi\n");

        let outcome = run.call();
        assert!(!outcome.raised, "{side_name}: soft warning must not raise");
        assert_eq!(
            con_log().last().map(String::as_str),
            Some("[con] tried to sprint to a non-client\n"),
            "{side_name}: copy-pasted warning text"
        );
    }
}

// ===========================================================================
// Extended message writers (pr_ext.c:2587-2611): dest resolution
// ===========================================================================

#[test]
fn writefloat_msg_one_writes_to_client_message() {
    let _g = lock();

    for (side_name, run) in [
        ("c", RunKind::COracle(ctest_msgref_pf_writefloat_run)),
        ("rust", RunKind::Rust(quake_rs_pf_WriteFloat)),
    ] {
        setup(4, 2, &[1, 2]);
        // SAFETY: plain C fixture setter; sets pr_global_struct->msg_entity via a prog-entity offset.
        unsafe { ctest_msgref_set_msg_entity(edict_prog(2)) };
        set_global_f(OFS_PARM0, MSG_ONE as f32);

        let outcome = run.call();
        assert!(!outcome.raised, "{side_name}: unexpected raise");
        // COMPAT bug 2: the payload is the dest float itself, MSG_ONE = 1.0.
        assert_eq!(
            client_bytes(2),
            (MSG_ONE as f32).to_le_bytes().to_vec(),
            "{side_name}: payload equals the destination selector's bits"
        );
        assert!(
            client_bytes(1).is_empty(),
            "{side_name}: only client 2 targeted"
        );
    }
}

#[test]
fn writefloat_msg_broadcast_writes_to_datagram_fixture() {
    let _g = lock();

    for (side_name, run) in [
        ("c", RunKind::COracle(ctest_msgref_pf_writefloat_run)),
        ("rust", RunKind::Rust(quake_rs_pf_WriteFloat)),
    ] {
        setup(2, 0, &[]);
        set_global_f(OFS_PARM0, MSG_BROADCAST as f32);

        let outcome = run.call();
        assert!(!outcome.raised, "{side_name}: unexpected raise");
        assert_eq!(
            dest_bytes(MSG_BROADCAST, 0),
            (MSG_BROADCAST as f32).to_le_bytes().to_vec(),
            "{side_name}: MSG_BROADCAST payload"
        );
    }
}

#[test]
fn writefloat_msg_one_invalid_client_raises_writedest_not_client() {
    let _g = lock();

    for (side_name, run) in [
        ("c", RunKind::COracle(ctest_msgref_pf_writefloat_run)),
        ("rust", RunKind::Rust(quake_rs_pf_WriteFloat)),
    ] {
        setup(4, 1, &[1]);
        // msg_entity names a valid edict (3) that is not a client (maxclients=1).
        // SAFETY: plain C fixture setter; sets pr_global_struct->msg_entity via a prog-entity offset.
        unsafe { ctest_msgref_set_msg_entity(edict_prog(3)) };
        set_global_f(OFS_PARM0, MSG_ONE as f32);

        let outcome = run.call();
        assert!(outcome.raised, "{side_name}: must raise");
        assert!(client_bytes(1).is_empty(), "{side_name}: nothing written");
        match run {
            RunKind::Rust(_) => {
                assert_eq!(outcome.status, Some(PRBI_ERR_WRITEDEST_NOT_CLIENT));
                assert_eq!(outcome.detail, Some(0));
            }
            RunKind::COracle(_) => {
                assert_eq!(outcome.message.as_deref(), Some("WriteDest: not a client"));
            }
        }
    }
}

#[test]
fn writefloat_msg_one_entnum_zero_raises_writedest_not_client() {
    let _g = lock();

    // Mutation testing (one mutation at a time, reverted after observing the
    // result) found that dropping `entnum < 1 ||` from write_dest's MSG_ONE
    // bound check (`progs_builtins_sv_msg.rs`) was NOT caught by any test in
    // this file -- every other raise test used an entnum *above*
    // svs.maxclients, never below 1. msg_entity = the world edict (prog 0)
    // resolves to entnum 0 via NUM_FOR_EDICT, closing that gap.
    for (side_name, run) in [
        ("c", RunKind::COracle(ctest_msgref_pf_writefloat_run)),
        ("rust", RunKind::Rust(quake_rs_pf_WriteFloat)),
    ] {
        setup(4, 2, &[1, 2]);
        // SAFETY: plain C fixture setter; sets pr_global_struct->msg_entity via a prog-entity offset.
        unsafe { ctest_msgref_set_msg_entity(0) }; // world edict, prog offset 0 -> entnum 0
        set_global_f(OFS_PARM0, MSG_ONE as f32);

        let outcome = run.call();
        assert!(outcome.raised, "{side_name}: must raise");
        assert!(client_bytes(1).is_empty(), "{side_name}: nothing written");
        assert!(client_bytes(2).is_empty(), "{side_name}: nothing written");
        match run {
            RunKind::Rust(_) => {
                assert_eq!(outcome.status, Some(PRBI_ERR_WRITEDEST_NOT_CLIENT));
            }
            RunKind::COracle(_) => {
                assert_eq!(outcome.message.as_deref(), Some("WriteDest: not a client"));
            }
        }
    }
}

#[test]
fn writefloat_bad_dest_raises_writedest_bad_dest() {
    let _g = lock();

    for (side_name, run) in [
        ("c", RunKind::COracle(ctest_msgref_pf_writefloat_run)),
        ("rust", RunKind::Rust(quake_rs_pf_WriteFloat)),
    ] {
        setup(2, 0, &[]);
        set_global_f(OFS_PARM0, 99.0); // not one of the six MSG_* values

        let outcome = run.call();
        assert!(outcome.raised, "{side_name}: must raise");
        match run {
            RunKind::Rust(_) => {
                assert_eq!(outcome.status, Some(PRBI_ERR_WRITEDEST_BAD_DEST));
                assert_eq!(outcome.detail, Some(0));
            }
            RunKind::COracle(_) => {
                assert_eq!(
                    outcome.message.as_deref(),
                    Some("WriteDest: bad destination")
                );
            }
        }
    }
}

// ===========================================================================
// Preserved bug 1: PF_WriteInt writes an 8-byte double, not an int32.
// ===========================================================================

#[test]
fn writeint_writes_double_not_int32() {
    let _g = lock();

    for (side_name, run) in [
        ("c", RunKind::COracle(ctest_msgref_pf_writeint_run)),
        ("rust", RunKind::Rust(quake_rs_pf_WriteInt)),
    ] {
        setup(2, 0, &[]);
        set_global_f(OFS_PARM0, MSG_ALL as f32); // dest = 2.0, bits = 0x40000000

        let outcome = run.call();
        assert!(!outcome.raised, "{side_name}: unexpected raise");
        let bytes = dest_bytes(MSG_ALL, 0);
        assert_eq!(
            bytes.len(),
            8,
            "{side_name}: an int32 writer would be 4 bytes"
        );
        // COMPAT bug 2: g_int(OFS_PARM0) reinterprets the dest float's 4
        // bytes as an int32, which C's implicit conversion then widens to
        // double (bug 1).
        let dest_bits = (MSG_ALL as f32).to_bits() as i32;
        let expected = (dest_bits as f64).to_le_bytes();
        assert_eq!(bytes, expected, "{side_name}: int-as-double payload");
    }
}

// ===========================================================================
// Preserved bug 2, 8-byte writers: the payload spans `globals[OFS_PARM0]`
// (the dest selector) and `globals[OFS_PARM0 + 1]` -- the *padding* word of
// PARM0's own 3-word slot (`progs.h`'s OFS_PARM stride is 3), reachable
// because `G_DOUBLE`/`G_INT64`/`G_UINT64` (`progs.h:166-170`) are raw
// `*(T *) &qcvm->globals[o]` pointer casts reading 8 bytes starting at
// `globals[o]`. `OFS_PARM1` (`globals[7]`) sits two words further out and is
// never touched -- an earlier pass of this test mistakenly used `OFS_PARM1`
// as the spillover source and got zero bytes back for every case, since
// nothing sets `globals[5]` (the real spillover word) in that scenario.
//
// `MSG_WriteDouble` (`net_msg.c:131-149`) is a fixed 8-byte little-endian
// dump, so its expected bytes can be hand-computed directly from the two
// source words. `MSG_WriteInt64`/`MSG_WriteUInt64` (`net_msg.c:94-115`) are
// NOT fixed-width: they are a variable-length, sign-zigzagged varint
// ("0* 10*,*, 110*,*,* etc, up to 0xff followed by 8 continuation bytes",
// `net_msg.c:95`), so this test does not attempt to hand-predict their wire
// bytes -- it relies on true differential comparison (Rust's payload word
// pair vs the C oracle's, fed through the same real varint encoder) instead.
// ===========================================================================

const OFS_PARM0_SPILL: c_int = OFS_PARM0 + 1; // globals[5]

#[test]
fn writedouble_payload_spans_ofs_parm0_and_its_padding_word() {
    let _g = lock();

    for (side_name, run) in [
        ("c", RunKind::COracle(ctest_msgref_pf_writedouble_run)),
        ("rust", RunKind::Rust(quake_rs_pf_WriteDouble)),
    ] {
        setup(2, 0, &[]);
        set_global_f(OFS_PARM0, MSG_INIT as f32); // dest = 3.0
        set_global_f(OFS_PARM0_SPILL, 2.5); // the real spillover word
        set_global_f(OFS_PARM1, 999.0); // decoy: OFS_PARM1 itself must be irrelevant

        let outcome = run.call();
        assert!(!outcome.raised, "{side_name}: unexpected raise");
        let bytes = dest_bytes(MSG_INIT, 0);
        assert_eq!(bytes.len(), 8);
        let mut raw = [0u8; 8];
        raw[0..4].copy_from_slice(&(MSG_INIT as f32).to_le_bytes());
        raw[4..8].copy_from_slice(&2.5f32.to_le_bytes());
        // MSG_WriteDouble serialises dat.l (the double's raw bits
        // reinterpreted as int64_t) byte 0 first -- on this little-endian
        // platform that is a verbatim memcpy of the 8 source bytes.
        assert_eq!(
            bytes, raw,
            "{side_name}: double payload = dest bits (low 4) + globals[OFS_PARM0+1] bits (high 4)"
        );
    }
}

#[test]
fn writeint64_payload_incorporates_ofs_parm0_padding_word() {
    let _g = lock();

    let mut wire = [None, None];
    for (i, (side_name, run)) in [
        ("c", RunKind::COracle(ctest_msgref_pf_writeint64_run)),
        ("rust", RunKind::Rust(quake_rs_pf_WriteInt64)),
    ]
    .into_iter()
    .enumerate()
    {
        setup(2, 0, &[]);
        set_global_f(OFS_PARM0, MSG_EXT_MULTICAST as f32); // dest = 4.0
        set_global_i(OFS_PARM0_SPILL, -7); // nonzero spillover, must show up in the varint

        let outcome = run.call();
        assert!(!outcome.raised, "{side_name}: unexpected raise");
        wire[i] = Some(dest_bytes(MSG_EXT_MULTICAST, 0));
    }
    assert_eq!(
        wire[0], wire[1],
        "c vs rust: int64 varint wire bytes must match"
    );

    // Cross-check against a zero spillover word: the encoded length/bytes
    // must differ, proving globals[OFS_PARM0+1] really is incorporated
    // (not just silently ignored the way OFS_PARM1 is).
    setup(2, 0, &[]);
    set_global_f(OFS_PARM0, MSG_EXT_MULTICAST as f32);
    set_global_i(OFS_PARM0_SPILL, 0);
    let outcome = RunKind::Rust(quake_rs_pf_WriteInt64).call();
    assert!(!outcome.raised);
    let zero_spill_wire = dest_bytes(MSG_EXT_MULTICAST, 0);
    assert_ne!(
        wire[1].as_ref().unwrap(),
        &zero_spill_wire,
        "rust: nonzero globals[OFS_PARM0+1] must change the encoded payload"
    );
}

#[test]
fn writeuint64_payload_incorporates_ofs_parm0_padding_word() {
    let _g = lock();

    let mut wire = [None, None];
    for (i, (side_name, run)) in [
        ("c", RunKind::COracle(ctest_msgref_pf_writeuint64_run)),
        ("rust", RunKind::Rust(quake_rs_pf_WriteUInt64)),
    ]
    .into_iter()
    .enumerate()
    {
        setup(2, 0, &[]);
        set_global_f(OFS_PARM0, MSG_EXT_ENTITY as f32); // dest = 5.0, shares sv.multicast with MSG_EXT_MULTICAST
        set_global_i(OFS_PARM0_SPILL, 12345);

        let outcome = run.call();
        assert!(!outcome.raised, "{side_name}: unexpected raise");
        wire[i] = Some(dest_bytes(MSG_EXT_MULTICAST, 0));
    }
    assert_eq!(
        wire[0], wire[1],
        "c vs rust: uint64 varint wire bytes must match"
    );
}

// ===========================================================================
// PF_WriteString2 (pr_ext.c:2587): the "string" is G_STRING(OFS_PARM0), the
// same slot as the dest selector -- OFS_PARM1's real intended string
// argument is never read at all (unlike the double-width writers, which at
// least incorporate OFS_PARM1's bits).
// ===========================================================================

#[test]
fn writestring2_msg_all_dest_ignores_ofs_parm1_argument() {
    let _g = lock();

    for (side_name, run) in [
        ("c", RunKind::COracle(ctest_msgref_pf_writestring2_run)),
        ("rust", RunKind::Rust(quake_rs_pf_WriteString2)),
    ] {
        setup(2, 0, &[]);
        set_global_f(OFS_PARM0, MSG_ALL as f32); // dest = 2.0, doubles as the string_t handle
        set_string_arg(1, c"REAL_ARG_SHOULD_BE_IGNORED");

        let outcome = run.call();
        assert!(!outcome.raised, "{side_name}: unexpected raise");
        let bytes = dest_bytes(MSG_ALL, 0);
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("REAL_ARG_SHOULD_BE_IGNORED"),
            "{side_name}: OFS_PARM1's real string argument must not appear on the wire"
        );
    }
}

#[test]
fn writestring2_bad_dest_raises_writedest_bad_dest() {
    let _g = lock();

    for (side_name, run) in [
        ("c", RunKind::COracle(ctest_msgref_pf_writestring2_run)),
        ("rust", RunKind::Rust(quake_rs_pf_WriteString2)),
    ] {
        setup(2, 0, &[]);
        set_global_f(OFS_PARM0, 42.0); // not one of the six MSG_* values

        let outcome = run.call();
        assert!(outcome.raised, "{side_name}: must raise");
        match run {
            RunKind::Rust(_) => {
                assert_eq!(outcome.status, Some(PRBI_ERR_WRITEDEST_BAD_DEST));
            }
            RunKind::COracle(_) => {
                assert_eq!(
                    outcome.message.as_deref(),
                    Some("WriteDest: bad destination")
                );
            }
        }
    }
}

// ===========================================================================
// Dispatch plumbing.
// ===========================================================================

/// Normalises the two very different raise-reporting conventions (see the
/// module doc's three raise shapes). `status`/`detail` are populated only
/// for the Rust side; `message` only when a real `Host_Error` ran underneath
/// (the C oracle always; the Rust port only through a `PRBI_ERR_GUARD` seam).
#[derive(Debug)]
struct Outcome {
    raised: bool,
    status: Option<c_int>,
    detail: Option<c_int>,
    message: Option<String>,
}

enum RunKind {
    COracle(unsafe extern "C" fn(*mut core::ffi::c_void)),
    Rust(unsafe extern "C" fn(*mut c_int) -> c_int),
}

impl RunKind {
    fn call(&self) -> Outcome {
        match self {
            RunKind::COracle(f) => {
                // SAFETY: `ctest_try_host` arms the Host_Error trap in a C
                // frame before calling `f`; no longjmp crosses a Rust frame.
                let r = unsafe { ctest_try_host(*f, core::ptr::null_mut()) };
                assert!(r == 0 || r == 1, "oracle raised Sys_Error ({r})");
                let raised = r == 1;
                Outcome {
                    raised,
                    status: None,
                    detail: None,
                    message: if raised {
                        Some(host_error_message())
                    } else {
                        None
                    },
                }
            }
            RunKind::Rust(f) => {
                let mut detail: c_int = -1;
                // SAFETY: every port entry point takes `&detail` exactly as
                // `RUST_PF` passes it and returns a PRBI_* status.
                let status = unsafe { f(&mut detail) };
                if status == PRBI_OK {
                    Outcome {
                        raised: false,
                        status: Some(status),
                        detail: None,
                        message: None,
                    }
                } else if status == PRBI_ERR_GUARD {
                    assert_eq!(detail, CTEST_GUARD_HOST_ERROR, "unexpected guard detail");
                    Outcome {
                        raised: true,
                        status: Some(status),
                        detail: Some(detail),
                        // A real Host_Error ran inside PRBI_MsgGlue_*'s
                        // Host_Guard, so the message is real and comparable.
                        message: Some(host_error_message()),
                    }
                } else {
                    // write_dest's own status codes: no Host_Error ran, so
                    // there is no message to compare (see the module doc).
                    Outcome {
                        raised: true,
                        status: Some(status),
                        detail: Some(detail),
                        message: None,
                    }
                }
            }
        }
    }
}

// Silence "unused" for the c_longlong/c_ulonglong imports, kept for
// signature clarity even though the writer bodies never construct these
// directly (the payload always comes from the aliased OFS_PARM0 float, per
// preserved bug 2).
#[allow(dead_code)]
fn _assert_ffi_types_present(_a: c_longlong, _b: c_ulonglong) {}
