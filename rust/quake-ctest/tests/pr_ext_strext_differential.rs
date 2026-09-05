//! Differential test: `Quake/pr_ext.c`'s string-extension group -- `PF_strconv`,
//! `PF_infoadd`, `PF_infoget`, the qc tokenizer, `PF_strftime` and `PF_stov` --
//! against the Rust port in `rust/quake-capi/src/progs_builtins_strext.rs`.
//! Rust migration Phase 7, M9f, group B.
//!
//! # How a comparison is made
//!
//! Same shape as `pr_ext_differential.rs` (T9f.0's wiring proof): every
//! scenario runs twice from a freshly reset fixture -- once through
//! `stubs/pr_ext_ref.c`'s `ctest_cref_pr_ext_strext_run` (the real,
//! statically-scoped `pr_ext.c` bodies, composed into that TU), once through
//! `quake_rs_pf_*` -- and the two observations must be equal. Both sides
//! intern through the same `c_ref_PR_SetEngineString`, both reach the same
//! `PR_GetTempString` ring, and `ctest_world_reset` memsets the whole qcvm, so
//! handles are handed out in the same order and are directly comparable.
//!
//! Group B owns dispatch indices 20-39 and has its own runner rather than more
//! cases in `ctest_cref_pr_ext_run`; see the `M9F GROUP B` block in
//! `stubs/pr_ext_ref.c` for why.
//!
//! # Observing the token table
//!
//! `qctoken` / `qctoken_count` are `pr_ext.c` file statics on the C side and
//! Rust statics on the other, so there is no single accessor for both. The
//! comparisons below read tokenizer state *through the ported builtins*
//! (`PF_ArgC`, `PF_ArgV`, `PF_argv_start_index`, `PF_argv_end_index`), which is
//! symmetric by construction. `token_table_observation_channel_is_faithful`
//! separately checks that channel against `pr_ext_ref.c`'s raw
//! `ctest_pr_ext_strext_token_*` accessors, so a misreading shared by both
//! sides cannot hide a difference.
//!
//! # The temp-string ring is an observable, not just plumbing
//!
//! `PR_GetTempString` has exactly one definition in this link, so both sides
//! step the same `pr_string_tempindex`. Every `Obs` carries it, which is what
//! pins `PF_infoadd`'s quirk of taking a temp string *before* its empty-key
//! early return while `PF_infoget` takes one only on a hit --
//! `infoadd_with_empty_key_still_steps_the_temp_ring` and
//! `infoget_miss_does_not_step_the_temp_ring` would both pass vacuously
//! without it.
//!
//! # Raise topology (ADR-009)
//!
//! The C side is driven through `Host_Guard`, which arms the `Host_Error` trap
//! in a C frame; the Rust side is status-returning. The two status *codes*
//! differ by construction on a raise (`Host_Guard`'s versus `PRBI_ERR_NO_STRING`),
//! so `Obs::raised` is a boolean and the raw code is deliberately not compared;
//! see `cleared_known_string_raises_on_both_sides`.

use core::ffi::{c_char, c_float, c_int, CStr};
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// `pr_comp.h`'s reserved global offsets.
const OFS_RETURN: c_int = 1;
const OFS_PARM0: c_int = 4;
const OFS_PARM1: c_int = 7;
const OFS_PARM2: c_int = 10;
const OFS_PARM3: c_int = 13;

/// `ctest_cref_pr_ext_strext_dispatch`'s switch indices
/// (`stubs/pr_ext_ref.c`, the `M9F GROUP B` block). Group B owns 20-39.
mod pf {
    pub const STRCONV: i32 = 20;
    pub const INFOADD: i32 = 21;
    pub const INFOGET: i32 = 22;
    pub const TOKENIZE: i32 = 23;
    pub const TOKENIZE_CONSOLE: i32 = 24;
    pub const TOKENIZEBYSEPARATOR: i32 = 25;
    pub const ARGC: i32 = 26;
    pub const ARGV: i32 = 27;
    pub const ARGV_START_INDEX: i32 = 28;
    pub const ARGV_END_INDEX: i32 = 29;
    pub const STRFTIME: i32 = 30;
    pub const STOV: i32 = 31;
    /// Not a builtin: `PR_ShutdownExtensions` calls `tokenize_flush` directly.
    pub const TOKENIZE_FLUSH: i32 = 32;
}

extern "C" {
    // --- fixture (stubs/pr_ext_ref.c) ------------------------------------
    fn ctest_pr_ext_reset_fixture(num_edicts: c_int);
    fn ctest_pr_ext_intern(s: *const c_char) -> c_int;
    fn ctest_pr_ext_set_argc(argc: c_int);
    fn ctest_pr_ext_set_global_int(ofs: c_int, v: c_int);
    fn ctest_pr_ext_get_global_int(ofs: c_int) -> c_int;
    fn ctest_pr_ext_get_string(handle: c_int) -> *const c_char;

    // --- fixture, M9F GROUP B block --------------------------------------
    fn ctest_pr_ext_set_global_float(ofs: c_int, v: c_float);
    fn ctest_pr_ext_get_global_float(ofs: c_int) -> c_float;
    fn ctest_pr_ext_strext_reset();
    fn ctest_pr_ext_strext_tempindex() -> c_int;
    fn ctest_pr_ext_strext_token_count() -> c_int;
    fn ctest_pr_ext_strext_token_start(i: c_int) -> c_int;
    fn ctest_pr_ext_strext_token_end(i: c_int) -> c_int;
    fn ctest_pr_ext_strext_token_text(i: c_int) -> *const c_char;
    fn ctest_pr_ext_strext_arm_bad_string() -> c_int;

    // --- oracle dispatcher (stubs/pr_ext_ref.c) --------------------------
    fn ctest_cref_pr_ext_strext_run(which: c_int) -> c_int;

    // --- console capture (stubs.c) ---------------------------------------
    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(i: c_int) -> *const c_char;

    // --- the Rust port under test ----------------------------------------
    fn quake_rs_pf_strconv(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_infoadd(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_infoget(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_Tokenize(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_tokenize_console(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_tokenizebyseparator(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_ArgC(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_ArgV(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_argv_start_index(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_argv_end_index(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_strftime(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_stov(detail: *mut c_int) -> c_int;
    fn quake_rs_pr_tokenize_flush(detail: *mut c_int) -> c_int;
}

// ---------------------------------------------------------------------------
// Safe wrappers. Every scenario below is safe code; the FFI justification for
// the whole file lives here (ADR-004). The fixture owns a private qcvm that
// `reset` reinitialises before each half of a comparison, the tests are
// serialised on `TEST_LOCK`, and no pointer handed across the boundary
// outlives the call it is passed to.

/// Reinitialises the fixture's qcvm, string pool and console log, then rewinds
/// *both* sides' tokenizer state and the shared temp-string ring.
///
/// `ctest_pr_ext_reset_fixture` cannot do the last part: `qctoken` is a
/// `pr_ext.c` static on one side and a Rust static on the other, and neither
/// lives in the qcvm that gets memset. Both are flushed on every reset so a
/// scenario can never inherit the *other* side's leftovers.
fn reset(num_edicts: c_int) {
    // SAFETY: no arguments to validate; the fixture allocates and zeroes its
    // own state, `ctest_pr_ext_strext_reset` only frees this link's own token
    // strings, and `quake_rs_pr_tokenize_flush` only frees the Rust module's.
    // Serialised by `TEST_LOCK`.
    unsafe {
        ctest_pr_ext_reset_fixture(num_edicts);
        ctest_pr_ext_strext_reset();
        let mut detail: c_int = 0;
        quake_rs_pr_tokenize_flush(&mut detail);
        // The Rust flush cannot raise, but rewind the ring after it anyway so
        // both halves of a comparison start at index 0 regardless of order.
        ctest_pr_ext_strext_reset();
    }
}

/// Copies `bytes` into the fixture's string blob and returns its `string_t`.
/// Byte-level rather than `&str` because the interesting `PF_strconv` and
/// tokenizer inputs contain bytes >= 128.
fn intern_bytes(bytes: &[u8]) -> c_int {
    let c = std::ffi::CString::new(bytes).expect("no interior NUL");
    // SAFETY: `c` is NUL-terminated and outlives the call; the fixture only
    // reads it (it copies into its own pool).
    unsafe { ctest_pr_ext_intern(c.as_ptr()) }
}

fn intern(s: &str) -> c_int {
    intern_bytes(s.as_bytes())
}

fn set_argc(argc: c_int) {
    // SAFETY: `argc` is a plain int the fixture range-checks against its own
    // globals block.
    unsafe { ctest_pr_ext_set_argc(argc) }
}

fn set_global(ofs: c_int, v: c_int) {
    // SAFETY: `ofs` is one of pr_comp.h's reserved offsets, inside the globals
    // block the fixture allocated.
    unsafe { ctest_pr_ext_set_global_int(ofs, v) }
}

fn get_global(ofs: c_int) -> c_int {
    // SAFETY: as `set_global`.
    unsafe { ctest_pr_ext_get_global_int(ofs) }
}

fn set_global_f(ofs: c_int, v: f32) {
    // SAFETY: as `set_global`.
    unsafe { ctest_pr_ext_set_global_float(ofs, v) }
}

fn get_global_f(ofs: c_int) -> f32 {
    // SAFETY: as `set_global`.
    unsafe { ctest_pr_ext_get_global_float(ofs) }
}

fn tempindex() -> c_int {
    // SAFETY: a plain read of `pr_string_tempindex`.
    unsafe { ctest_pr_ext_strext_tempindex() }
}

fn read_string(handle: c_int) -> String {
    // SAFETY: `handle` was produced by this same string table and has not been
    // cleared. The returned pointer is NUL-terminated and stays valid until
    // the next `reset`, which cannot run before this copy completes.
    unsafe {
        let p = ctest_pr_ext_get_string(handle);
        assert!(!p.is_null(), "PR_GetString returned null for {handle}");
        CStr::from_ptr(p).to_bytes().to_vec()
    }
    .iter()
    .map(|&b| b as char)
    .collect()
}

/// The console log, split into the diagnostics the builtins emit (in order)
/// and a count of `PR_AllocStringSlots`' growth notices.
///
/// The split exists because the Rust side routes its own diagnostics through
/// `run_sv`'s deferred console, which flushes them after the builtin body
/// returns, while the C side prints them inline. The only other line a group B
/// builtin can produce is `Con_DPrintf2`'s slot-growth notice, and that comes
/// from the *shared* C `PR_SetEngineString`, so it prints inline on both
/// sides. The two raw sequences therefore differ only in where that notice
/// sits relative to a builtin's own warnings -- e.g. `PF_infoadd` warns and
/// then interns. Comparing the builtin diagnostics in order, plus the notice
/// count, keeps every observable except that one interleaving, which the
/// deferred-console design makes non-comparable by construction.
fn console() -> (Vec<String>, usize) {
    // SAFETY: `ctest_con_log_len` bounds the index, and each entry is a
    // NUL-terminated buffer owned by the log until the next `reset`.
    let all: Vec<String> = unsafe {
        (0..ctest_con_log_len())
            .map(|i| {
                CStr::from_ptr(ctest_con_log_get(i))
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    };
    let slot_allocs = all
        .iter()
        .filter(|l| l.contains("PR_AllocStringSlots"))
        .count();
    let lines = all
        .into_iter()
        .filter(|l| !l.contains("PR_AllocStringSlots"))
        .collect();
    (lines, slot_allocs)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    C,
    Rust,
}

/// Runs one builtin on one side and returns its raw status. Zero means "no
/// raise" on both sides: `Host_Guard`'s success value and `PRBI_OK` are both 0.
/// The *non-zero* values are not comparable across sides (see the module
/// header), so callers store `status != 0`.
fn invoke(side: Side, which: i32) -> c_int {
    match side {
        // SAFETY: `which` is one of the group B dispatcher indices, and the C
        // body runs inside `Host_Guard`, so a `Host_Error` unwinds in a C frame
        // and never longjmps past this call (ADR-009).
        Side::C => unsafe { ctest_cref_pr_ext_strext_run(which) },
        Side::Rust => {
            let mut detail: c_int = 0;
            // SAFETY: `detail` is a live, initialised `c_int`; these entry
            // points are status-returning and read the ambient qcvm the
            // fixture has just reset (ADR-008).
            unsafe {
                match which {
                    pf::STRCONV => quake_rs_pf_strconv(&mut detail),
                    pf::INFOADD => quake_rs_pf_infoadd(&mut detail),
                    pf::INFOGET => quake_rs_pf_infoget(&mut detail),
                    pf::TOKENIZE => quake_rs_pf_Tokenize(&mut detail),
                    pf::TOKENIZE_CONSOLE => quake_rs_pf_tokenize_console(&mut detail),
                    pf::TOKENIZEBYSEPARATOR => quake_rs_pf_tokenizebyseparator(&mut detail),
                    pf::ARGC => quake_rs_pf_ArgC(&mut detail),
                    pf::ARGV => quake_rs_pf_ArgV(&mut detail),
                    pf::ARGV_START_INDEX => quake_rs_pf_argv_start_index(&mut detail),
                    pf::ARGV_END_INDEX => quake_rs_pf_argv_end_index(&mut detail),
                    pf::STRFTIME => quake_rs_pf_strftime(&mut detail),
                    pf::STOV => quake_rs_pf_stov(&mut detail),
                    pf::TOKENIZE_FLUSH => quake_rs_pr_tokenize_flush(&mut detail),
                    _ => panic!("bad dispatch index {which}"),
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Observations.

/// What a string-returning builtin is compared on.
#[derive(Debug, PartialEq, Eq)]
struct Obs {
    raised: bool,
    ret: c_int,
    text: String,
    tempindex: c_int,
    console: Vec<String>,
    slot_allocs: usize,
}

fn obs_after(status: c_int) -> Obs {
    let ret = get_global(OFS_RETURN);
    let (console, slot_allocs) = console();
    Obs {
        raised: status != 0,
        ret,
        text: read_string(ret),
        tempindex: tempindex(),
        console,
        slot_allocs,
    }
}

/// One token, as the builtins report it.
#[derive(Debug, PartialEq, Eq)]
struct Tok {
    start: i32,
    end: i32,
    text: String,
}

/// What a tokenizing builtin is compared on. `tokens` is read back through
/// `PF_ArgC` / `PF_argv_start_index` / `PF_argv_end_index` / `PF_ArgV`, so this
/// covers those four builtins as well as whichever one filled the table.
#[derive(Debug, PartialEq, Eq)]
struct TokObs {
    raised: bool,
    count: i32,
    tokens: Vec<Tok>,
    tempindex: c_int,
    console: Vec<String>,
    slot_allocs: usize,
}

/// Reads the whole token table through the builtins. `f32` round-trips every
/// index used here exactly (counts are <= 64), so the index never loses bits.
fn read_tokens(side: Side) -> (i32, Vec<Tok>) {
    set_argc(1);
    assert_eq!(invoke(side, pf::ARGC), 0, "PF_ArgC cannot raise");
    let count = get_global_f(OFS_RETURN) as i32;

    let tokens = (0..count)
        .map(|i| {
            set_global_f(OFS_PARM0, i as f32);
            set_argc(1);
            assert_eq!(invoke(side, pf::ARGV_START_INDEX), 0);
            let start = get_global_f(OFS_RETURN) as i32;

            set_global_f(OFS_PARM0, i as f32);
            set_argc(1);
            assert_eq!(invoke(side, pf::ARGV_END_INDEX), 0);
            let end = get_global_f(OFS_RETURN) as i32;

            set_global_f(OFS_PARM0, i as f32);
            set_argc(1);
            assert_eq!(invoke(side, pf::ARGV), 0);
            let text = read_string(get_global(OFS_RETURN));

            Tok { start, end, text }
        })
        .collect();
    (count, tokens)
}

// ---------------------------------------------------------------------------
// Scenarios: PF_strconv.

/// `PF_strconv (ccase, redalpha, rednum, ...)` -- the varargs start at PARM3,
/// which is `PF_VarString (3)`.
fn run_strconv(side: Side, ccase: f32, redalpha: f32, rednum: f32, args: &[&[u8]]) -> Obs {
    reset(4);
    set_global_f(OFS_PARM0, ccase);
    set_global_f(OFS_PARM1, redalpha);
    set_global_f(OFS_PARM2, rednum);
    for (i, a) in args.iter().enumerate() {
        let h = intern_bytes(a);
        set_global(OFS_PARM3 + (i as c_int) * 3, h);
    }
    set_argc(3 + args.len() as c_int);

    let status = invoke(side, pf::STRCONV);
    obs_after(status)
}

fn both_strconv(ccase: f32, redalpha: f32, rednum: f32, args: &[&[u8]]) -> (Obs, Obs) {
    (
        run_strconv(Side::C, ccase, redalpha, rednum, args),
        run_strconv(Side::Rust, ccase, redalpha, rednum, args),
    )
}

#[test]
fn strconv_identity_mode_matches() {
    let _g = lock();
    let (c, rs) = both_strconv(0.0, 0.0, 0.0, &[b"Hello 42! ~\x7f"]);
    assert_eq!(c.text, "Hello 42! ~\u{7f}", "mode 0/0/0 is a copy");
    assert_eq!(c, rs);
}

#[test]
fn strconv_case_conversion_matches() {
    let _g = lock();
    let (c, rs) = both_strconv(1.0, 0.0, 0.0, &[b"MiXeD Case 99"]);
    assert_eq!(c.text, "mixed case 99");
    assert_eq!(c, rs);

    let (c, rs) = both_strconv(2.0, 0.0, 0.0, &[b"MiXeD Case 99"]);
    assert_eq!(c.text, "MIXED CASE 99");
    assert_eq!(c, rs);
}

#[test]
fn strconv_colour_conversion_matches() {
    let _g = lock();
    // redalpha 2 / rednum 2 add the 128 bit; redalpha 1 / rednum 1 strip it.
    for (redalpha, rednum) in [(1.0, 1.0), (2.0, 2.0), (2.0, 0.0), (0.0, 2.0)] {
        let (c, rs) = both_strconv(0.0, redalpha, rednum, &[b"abc XYZ 019 !?"]);
        assert_eq!(c, rs, "redalpha {redalpha} rednum {rednum}");
    }
}

#[test]
fn strconv_alternating_modes_match() {
    let _g = lock();
    // COMPAT: chrchar_alpha's `128 * ((charnum & 1) == (convt - 5))` -- modes 5
    // and 6 redden opposite parities, and chrconv_number's `case 5: case 6:`
    // arms fall through to the no-op default, so digits are left alone.
    for redalpha in [5.0, 6.0] {
        for rednum in [0.0, 5.0, 6.0] {
            let (c, rs) = both_strconv(0.0, redalpha, rednum, &[b"abcdef 012345"]);
            assert_eq!(c, rs, "redalpha {redalpha} rednum {rednum}");
        }
    }
}

#[test]
fn strconv_special_number_bases_match() {
    let _g = lock();
    // rednum 3/4 map onto the '0' - 30 / '0' + 128 - 30 "special" digit runs.
    for rednum in [3.0, 4.0] {
        let (c, rs) = both_strconv(0.0, 0.0, rednum, &[b"0123456789"]);
        assert_eq!(c, rs, "rednum {rednum}");
    }
}

#[test]
fn strconv_high_bytes_match() {
    let _g = lock();
    // Every byte >= 128 the ladder can reach: the +128 digit run (176..185),
    // the +128-30 run (146..155), the +128 letter runs, and the punctuation
    // catch-alls on both sides of 128.
    let mut payload = Vec::new();
    payload.extend(0x80u8..=0xff);
    let (c, rs) = both_strconv(0.0, 2.0, 2.0, &[&payload]);
    assert_eq!(c, rs);

    let (c, rs) = both_strconv(1.0, 5.0, 3.0, &[&payload]);
    assert_eq!(c, rs);
}

#[test]
fn strconv_low_control_bytes_are_passed_through() {
    let _g = lock();
    // `(*string & 127) < 16` short-circuits ahead of both chrconv_punct arms.
    let payload: Vec<u8> = (1u8..16).chain(129u8..144).collect();
    let (c, rs) = both_strconv(0.0, 2.0, 2.0, &[&payload]);
    assert_eq!(c, rs);
}

#[test]
fn strconv_concatenates_all_varargs() {
    let _g = lock();
    let (c, rs) = both_strconv(2.0, 0.0, 0.0, &[b"ab", b"", b"cd", b"ef"]);
    assert_eq!(c.text, "ABCDEF");
    assert_eq!(c, rs);
}

#[test]
fn strconv_with_no_varargs_matches() {
    let _g = lock();
    let (c, rs) = both_strconv(1.0, 1.0, 1.0, &[]);
    assert_eq!(c.text, "", "argc 3 means PF_VarString (3) returns empty");
    assert_eq!(c, rs);
}

#[test]
fn strconv_varstring_overflow_warns_identically() {
    let _g = lock();
    // PF_VarString truncates at its own 1024-byte buffer and warns; the
    // warning has to land on both sides, and the truncated text has to match.
    let big = vec![b'q'; 600];
    let (c, rs) = both_strconv(2.0, 0.0, 0.0, &[&big, &big]);
    assert!(
        c.console
            .iter()
            .any(|l| l.contains("PF_VarString: overflow")),
        "expected the truncation warning, got {:?}",
        c.console
    );
    assert_eq!(c.text.len(), 1023, "q_strlcat stops one short of 1024");
    assert_eq!(c, rs);
}

// ---------------------------------------------------------------------------
// Scenarios: PF_infoadd / PF_infoget.

fn run_infoadd(side: Side, info: &str, key: &str, value: &str) -> Obs {
    reset(4);
    let i = intern(info);
    let k = intern(key);
    let v = intern(value);
    set_global(OFS_PARM0, i);
    set_global(OFS_PARM1, k);
    set_global(OFS_PARM2, v);
    set_argc(3);

    let status = invoke(side, pf::INFOADD);
    obs_after(status)
}

fn both_infoadd(info: &str, key: &str, value: &str) -> (Obs, Obs) {
    (
        run_infoadd(Side::C, info, key, value),
        run_infoadd(Side::Rust, info, key, value),
    )
}

fn run_infoget(side: Side, info: &str, key: &str) -> Obs {
    reset(4);
    let i = intern(info);
    let k = intern(key);
    set_global(OFS_PARM0, i);
    set_global(OFS_PARM1, k);
    set_argc(2);

    let status = invoke(side, pf::INFOGET);
    obs_after(status)
}

fn both_infoget(info: &str, key: &str) -> (Obs, Obs) {
    (
        run_infoget(Side::C, info, key),
        run_infoget(Side::Rust, info, key),
    )
}

#[test]
fn infoadd_into_empty_info_matches() {
    let _g = lock();
    let (c, rs) = both_infoadd("", "name", "player");
    assert_eq!(c.text, "\\name\\player");
    assert_eq!(c, rs);
}

#[test]
fn infoadd_replaces_an_existing_key() {
    let _g = lock();
    let (c, rs) = both_infoadd("\\a\\1\\name\\old\\b\\2", "name", "new");
    assert_eq!(
        c.text, "\\a\\1\\b\\2\\name\\new",
        "the old pair is stripped"
    );
    assert_eq!(c, rs);
}

#[test]
fn infoadd_with_empty_value_removes_the_key() {
    let _g = lock();
    let (c, rs) = both_infoadd("\\a\\1\\name\\old", "name", "");
    assert_eq!(c.text, "\\a\\1", "the `nothing needed` arm still strips");
    assert_eq!(c, rs);
}

#[test]
fn infoadd_with_empty_key_still_steps_the_temp_ring() {
    let _g = lock();
    // COMPAT: C evaluates `PR_GetTempString ()` in the declaration list, before
    // the `if (!*key)` early return, so the ring moves even on this path. Both
    // `ret` (the untouched PARM0 handle) and `tempindex` are compared.
    let (c, rs) = both_infoadd("\\a\\1", "", "x");
    assert_eq!(c.text, "\\a\\1", "the input handle is returned unchanged");
    assert_eq!(c.tempindex, 1, "the ring moved despite the early return");
    assert_eq!(c, rs);
}

#[test]
fn infoadd_rejects_backslashes_in_key_or_value() {
    let _g = lock();
    for (key, value) in [("na\\me", "player"), ("name", "pl\\ayer")] {
        let (c, rs) = both_infoadd("\\a\\1", key, value);
        assert!(
            c.console
                .iter()
                .any(|l| l.contains("PF_infoadd: invalid key/value")),
            "expected the key/value warning for {key:?}/{value:?}, got {:?}",
            c.console
        );
        assert_eq!(c, rs);
    }
}

#[test]
fn infoadd_rejects_malformed_source_info() {
    let _g = lock();
    // No leading backslash, so the first `*info++ != '\\'` breaks with bytes
    // left over and the "invalid source info" arm fires.
    let (c, rs) = both_infoadd("garbage", "name", "player");
    assert!(
        c.console
            .iter()
            .any(|l| l.contains("PF_infoadd: invalid source info")),
        "got {:?}",
        c.console
    );
    assert_eq!(c, rs);
}

#[test]
fn infoadd_truncates_a_key_only_pair_identically() {
    let _g = lock();
    // A key with no value terminator: the else branch's `if (*info++ != '\\')`
    // breaks mid-walk, so the copy stops there.
    let (c, rs) = both_infoadd("\\a\\1\\dangling", "name", "player");
    assert_eq!(c, rs);
}

#[test]
fn infoadd_length_overflow_warns_identically() {
    let _g = lock();
    // Long enough that `o + 2 + keylen + valuelen >= e` fires with the source
    // copied in full (STRINGTEMP_LENGTH is 1024, and PF_VarString's own buffer
    // caps `value` well below that).
    let mut info = String::new();
    for i in 0..64 {
        info.push_str(&format!("\\k{i:02}\\{}", "v".repeat(10)));
    }
    assert!(info.len() > 900 && info.len() < 1024);
    let (c, rs) = both_infoadd(&info, "name", &"p".repeat(60));
    assert!(
        c.console
            .iter()
            .any(|l| l.contains("PF_infoadd: length exceeds max")),
        "got {:?}",
        c.console
    );
    assert_eq!(c, rs);
}

#[test]
fn infoadd_source_copy_overflow_breaks_identically() {
    let _g = lock();
    // Here the *copy* loop's `o + (info - l) >= e` is what stops the walk, so
    // the result is a prefix of the source and the trailing `if (*info)` arm
    // reports invalid source info.
    let mut info = String::new();
    for i in 0..40 {
        info.push_str(&format!("\\k{i:02}\\{}", "v".repeat(24)));
    }
    assert!(info.len() > 1024);
    let (c, rs) = both_infoadd(&info, "name", "player");
    assert_eq!(c, rs);
}

#[test]
fn infoget_hit_matches() {
    let _g = lock();
    let (c, rs) = both_infoget("\\a\\1\\name\\player\\b\\2", "name");
    assert_eq!(c.text, "player");
    assert_eq!(c.tempindex, 1, "a hit takes exactly one temp string");
    assert_eq!(c, rs);
}

#[test]
fn infoget_miss_does_not_step_the_temp_ring() {
    let _g = lock();
    // COMPAT: PF_infoget calls PR_GetTempString only inside the match branch,
    // unlike PF_infoadd. A miss must leave the ring where it was.
    let (c, rs) = both_infoget("\\a\\1\\b\\2", "name");
    assert_eq!(c.ret, 0, "a miss returns the null string handle");
    assert_eq!(c.tempindex, 0, "no temp string is taken on a miss");
    assert_eq!(c, rs);
}

#[test]
fn infoget_does_not_match_a_key_prefix() {
    let _g = lock();
    // `info[keylen] == '\\'` is what stops "na" from matching "\\name\\x".
    let (c, rs) = both_infoget("\\name\\player", "na");
    assert_eq!(c.ret, 0);
    assert_eq!(c, rs);

    // ... and the reverse: a longer key must not match a shorter stored one.
    let (c, rs) = both_infoget("\\na\\player", "name");
    assert_eq!(c.ret, 0);
    assert_eq!(c, rs);
}

#[test]
fn infoget_empty_key_matches_the_first_empty_key_pair() {
    let _g = lock();
    let (c, rs) = both_infoget("\\\\v\\a\\1", "");
    assert_eq!(c, rs);
}

#[test]
fn infoget_on_malformed_info_matches() {
    let _g = lock();
    for info in ["", "garbage", "\\a", "\\a\\", "\\a\\1\\dangling"] {
        let (c, rs) = both_infoget(info, "a");
        assert_eq!(c, rs, "info {info:?}");
    }
}

#[test]
fn infoget_truncates_a_long_value_identically() {
    let _g = lock();
    // The copy loop's `o < e` guard is the only bound; STRINGTEMP_LENGTH is
    // 1024 but the fixture's string pool is 2048, so a >1023-byte value fits.
    let info = format!("\\k\\{}", "z".repeat(1100));
    let (c, rs) = both_infoget(&info, "k");
    assert_eq!(c.text.len(), 1023, "truncated at destbuf + 1024 - 1");
    assert_eq!(c, rs);
}

// ---------------------------------------------------------------------------
// Scenarios: the qc tokenizer.

fn run_tokenize(side: Side, which: i32, input: &[u8]) -> TokObs {
    reset(4);
    let h = intern_bytes(input);
    set_global(OFS_PARM0, h);
    set_argc(1);

    let status = invoke(side, which);
    let returned = get_global_f(OFS_RETURN) as i32;
    let (count, tokens) = read_tokens(side);
    assert_eq!(returned, count, "the builtin returns qctoken_count");
    let (console, slot_allocs) = console();
    TokObs {
        raised: status != 0,
        count,
        tokens,
        tempindex: tempindex(),
        console,
        slot_allocs,
    }
}

fn both_tokenize(which: i32, input: &[u8]) -> (TokObs, TokObs) {
    (
        run_tokenize(Side::C, which, input),
        run_tokenize(Side::Rust, which, input),
    )
}

fn run_tokenizebyseparator(side: Side, input: &str, seps: &[&str]) -> TokObs {
    reset(4);
    let h = intern(input);
    set_global(OFS_PARM0, h);
    for (i, s) in seps.iter().enumerate() {
        let sh = intern(s);
        set_global(OFS_PARM1 + (i as c_int) * 3, sh);
    }
    set_argc(1 + seps.len() as c_int);

    let status = invoke(side, pf::TOKENIZEBYSEPARATOR);
    let returned = get_global_f(OFS_RETURN) as i32;
    let (count, tokens) = read_tokens(side);
    assert_eq!(returned, count);
    let (console, slot_allocs) = console();
    TokObs {
        raised: status != 0,
        count,
        tokens,
        tempindex: tempindex(),
        console,
        slot_allocs,
    }
}

fn both_tokenizebyseparator(input: &str, seps: &[&str]) -> (TokObs, TokObs) {
    (
        run_tokenizebyseparator(Side::C, input, seps),
        run_tokenizebyseparator(Side::Rust, input, seps),
    )
}

#[test]
fn tokenize_simple_words_match() {
    let _g = lock();
    let (c, rs) = both_tokenize(pf::TOKENIZE, b"alpha beta gamma");
    assert_eq!(c.count, 3);
    assert_eq!(
        c.tokens.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(),
        ["alpha", "beta", "gamma"]
    );
    assert_eq!(
        c.tokens[0],
        Tok {
            start: 0,
            end: 5,
            text: "alpha".into()
        }
    );
    assert_eq!(c, rs);
}

#[test]
fn tokenize_and_tokenize_console_agree_on_every_input() {
    let _g = lock();
    // `tokenizeqc`'s `dpfuckage` parameter selects a punctuation branch that is
    // commented out in the C (pr_ext.c:1636-1639), so the two builtins are
    // currently identical. Pinned here so a future divergence is caught.
    for input in [
        &b"a b c"[..],
        b"\"quoted string\" tail",
        b"   leading and trailing   ",
        b"a//comment\nb",
        b"a\tb\nc\rd",
    ] {
        let (c, rs) = both_tokenize(pf::TOKENIZE, input);
        assert_eq!(c, rs, "PF_Tokenize {input:?}");
        let (cc, rc) = both_tokenize(pf::TOKENIZE_CONSOLE, input);
        assert_eq!(cc, rc, "PF_tokenize_console {input:?}");
        assert_eq!(c.tokens, cc.tokens, "dpfuckage is currently inert");
    }
}

#[test]
fn tokenize_empty_and_whitespace_only_match() {
    let _g = lock();
    for input in [&b""[..], b"   ", b"\t\n\r "] {
        let (c, rs) = both_tokenize(pf::TOKENIZE, input);
        assert_eq!(c.count, 0, "{input:?}");
        assert_eq!(c, rs, "{input:?}");
    }
}

#[test]
fn tokenize_high_bytes_reproduce_the_signedness_quirk() {
    let _g = lock();
    // COMPAT: tokenizeqc's pre-skip reads through `const unsigned char *`, so a
    // byte >= 128 is NOT whitespace and fixes `.start` at its own offset;
    // COM_Parse's own skip reads through a (signed) `const char *` and does
    // treat it as whitespace, so the token text starts later. `.start` is
    // therefore *before* the first byte of the token. Preserved on both sides.
    let (c, rs) = both_tokenize(pf::TOKENIZE, b"\x80\x81abc def");
    assert_eq!(c.count, 2);
    assert_eq!(c.tokens[0].text, "abc");
    assert_eq!(
        c.tokens[0].start, 0,
        "start points at the 0x80, not at the 'a'"
    );
    assert_eq!(c, rs);

    let (c, rs) = both_tokenize(pf::TOKENIZE, b"a \xff\xfe b");
    assert_eq!(c, rs);
}

#[test]
fn tokenize_saturates_at_maxqctokens() {
    let _g = lock();
    // MAXQCTOKENS is 64; the `while (qctoken_count < MAXQCTOKENS)` guard is the
    // only thing standing between qctoken_count and a 64-entry array.
    let input: Vec<u8> = (0..80)
        .map(|i| format!("t{i}"))
        .collect::<Vec<_>>()
        .join(" ")
        .into_bytes();
    let (c, rs) = both_tokenize(pf::TOKENIZE, &input);
    assert_eq!(c.count, 64, "capped at MAXQCTOKENS");
    assert_eq!(c, rs);
}

#[test]
fn tokenizebyseparator_simple_split_matches() {
    let _g = lock();
    let (c, rs) = both_tokenizebyseparator("a,b,c", &[","]);
    assert_eq!(
        c.tokens.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    assert_eq!(c, rs);
}

#[test]
fn tokenizebyseparator_reproduces_the_extra_advance_bug() {
    let _g = lock();
    // COMPAT (bug preserved): after a separator match the cursor advances by
    // `seplen[s]` *and* by the loop's unconditional `str++`, so the byte after
    // a separator is swallowed. Two adjacent separators therefore do not yield
    // an empty token: "a,,b" splits as "a" and ",b".
    let (c, rs) = both_tokenizebyseparator("a,,b", &[","]);
    assert_eq!(
        c.tokens.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(),
        ["a", ",b"],
        "not [\"a\", \"\", \"b\"] -- this is the C's behaviour, kept"
    );
    assert_eq!(c, rs);
}

#[test]
fn tokenizebyseparator_trailing_and_leading_separators_match() {
    let _g = lock();
    for input in ["a,b,", ",a,b", ",", ",,", "a,"] {
        let (c, rs) = both_tokenizebyseparator(input, &[","]);
        assert_eq!(c, rs, "{input:?}");
    }
}

#[test]
fn tokenizebyseparator_with_no_separators_returns_the_whole_string() {
    let _g = lock();
    // argc 1 means the `seps` loop never runs, so the only `found` is the
    // end-of-string one.
    let (c, rs) = both_tokenizebyseparator("a,b c", &[]);
    assert_eq!(c.count, 1);
    assert_eq!(c.tokens[0].text, "a,b c");
    assert_eq!(c, rs);
}

#[test]
fn tokenizebyseparator_empty_input_matches() {
    let _g = lock();
    let (c, rs) = both_tokenizebyseparator("", &[","]);
    assert_eq!(c.count, 0, "the `if (*str)` guard skips the loop entirely");
    assert_eq!(c, rs);
}

#[test]
fn tokenizebyseparator_empty_separator_matches_everywhere() {
    let _g = lock();
    // strncmp with n == 0 returns 0, so an empty separator matches at every
    // position; combined with the extra `str++` this emits one empty token per
    // two input bytes.
    let (c, rs) = both_tokenizebyseparator("abcdef", &[""]);
    assert_eq!(c, rs);
}

#[test]
fn tokenizebyseparator_multiple_and_multibyte_separators_match() {
    let _g = lock();
    let (c, rs) = both_tokenizebyseparator("a::b--c::d", &["::", "--"]);
    assert_eq!(c, rs);

    // More than the seven the fixed arrays hold: only the first seven are read.
    let seps = ["1", "2", "3", "4", "5", "6", "7", "8", "9"];
    let (c, rs) = both_tokenizebyseparator("a1b2c8d9e", &seps);
    assert_eq!(c, rs);
}

#[test]
fn tokenizebyseparator_saturates_at_maxqctokens() {
    let _g = lock();
    let input = "x,".repeat(100);
    let (c, rs) = both_tokenizebyseparator(&input, &[","]);
    assert_eq!(c.count, 64);
    assert_eq!(c, rs);
}

#[test]
fn tokenize_flush_clears_the_table_on_both_sides() {
    let _g = lock();
    // TOKENIZE_FLUSH is `tokenize_flush` itself, the non-builtin
    // PR_ShutdownExtensions calls directly.
    for side in [Side::C, Side::Rust] {
        reset(4);
        let h = intern("a b c");
        set_global(OFS_PARM0, h);
        set_argc(1);
        assert_eq!(invoke(side, pf::TOKENIZE), 0);
        assert_eq!(read_tokens(side).0, 3, "{side:?}");

        assert_eq!(invoke(side, pf::TOKENIZE_FLUSH), 0, "{side:?}");
        assert_eq!(read_tokens(side).0, 0, "{side:?} after flush");
    }
}

#[test]
fn tokenize_is_idempotent_across_repeated_calls() {
    let _g = lock();
    // tokenizeqc flushes before refilling; running it three times in a row must
    // leave exactly one table's worth of tokens, not three.
    for side in [Side::C, Side::Rust] {
        reset(4);
        let h = intern("one two");
        for _ in 0..3 {
            set_global(OFS_PARM0, h);
            set_argc(1);
            assert_eq!(invoke(side, pf::TOKENIZE), 0);
        }
        assert_eq!(read_tokens(side).0, 2, "{side:?}");
    }
}

#[test]
fn argv_index_normalisation_matches() {
    let _g = lock();
    // Negative indexes are relative to the end; out-of-range returns -1 (or the
    // null handle for PF_ArgV). 1e30 exercises ADR-006's C float->int cast:
    // on x86_64 an out-of-range conversion yields INT_MIN, which is negative,
    // so it takes the `idx += qctoken_count` path and then fails the unsigned
    // range check.
    for idx in [
        0.0f32, 1.0, 2.0, 3.0, -1.0, -3.0, -4.0, -100.0, 1e30, -1e30, 2.9, -0.5,
    ] {
        let mut obs = Vec::new();
        for side in [Side::C, Side::Rust] {
            reset(4);
            let h = intern("aa bb cc");
            set_global(OFS_PARM0, h);
            set_argc(1);
            assert_eq!(invoke(side, pf::TOKENIZE), 0);

            let mut got = Vec::new();
            for which in [pf::ARGV_START_INDEX, pf::ARGV_END_INDEX] {
                set_global_f(OFS_PARM0, idx);
                set_argc(1);
                assert_eq!(invoke(side, which), 0);
                got.push(format!("{}", get_global_f(OFS_RETURN)));
            }
            set_global_f(OFS_PARM0, idx);
            set_argc(1);
            assert_eq!(invoke(side, pf::ARGV), 0);
            got.push(read_string(get_global(OFS_RETURN)));
            obs.push(got);
        }
        assert_eq!(obs[0], obs[1], "idx {idx}");
    }
}

#[test]
fn argv_on_an_empty_table_matches() {
    let _g = lock();
    let mut obs = Vec::new();
    for side in [Side::C, Side::Rust] {
        reset(4);
        set_global_f(OFS_PARM0, 0.0);
        set_argc(1);
        assert_eq!(invoke(side, pf::ARGV), 0);
        let ret = get_global(OFS_RETURN);
        obs.push((ret, read_string(ret), tempindex()));
    }
    assert_eq!(obs[0].0, 0, "no tokens means the null handle");
    assert_eq!(obs[0].2, 0, "and no temp string is taken");
    assert_eq!(obs[0], obs[1]);
}

#[test]
fn token_table_observation_channel_is_faithful() {
    let _g = lock();
    // The C-only raw accessors exist for exactly this: prove that reading the
    // table through PF_ArgC/PF_ArgV/PF_argv_*_index reports what pr_ext.c's
    // qctoken array actually holds, so a misreading shared by both sides
    // cannot hide a real difference in the comparisons above.
    reset(4);
    let h = intern_bytes(b"\x80\x81abc \"d e\" f");
    set_global(OFS_PARM0, h);
    set_argc(1);
    assert_eq!(invoke(Side::C, pf::TOKENIZE), 0);

    // SAFETY: the raw accessors range-check `i` against qctoken_count
    // themselves and return NULL / -1 out of range.
    let raw: Vec<Tok> = unsafe {
        let n = ctest_pr_ext_strext_token_count();
        (0..n)
            .map(|i| {
                let p = ctest_pr_ext_strext_token_text(i);
                assert!(!p.is_null());
                Tok {
                    start: ctest_pr_ext_strext_token_start(i),
                    end: ctest_pr_ext_strext_token_end(i),
                    text: CStr::from_ptr(p)
                        .to_bytes()
                        .iter()
                        .map(|&b| b as char)
                        .collect(),
                }
            })
            .collect()
    };

    let (count, via_builtins) = read_tokens(Side::C);
    assert_eq!(count as usize, raw.len());
    assert_eq!(via_builtins, raw);
}

// ---------------------------------------------------------------------------
// Scenarios: PF_stov.

fn run_stov(side: Side, s: &str) -> (bool, [f32; 3], c_int) {
    reset(4);
    let h = intern(s);
    set_global(OFS_PARM0, h);
    set_argc(1);
    let status = invoke(side, pf::STOV);
    (
        status != 0,
        [
            get_global_f(OFS_RETURN),
            get_global_f(OFS_RETURN + 1),
            get_global_f(OFS_RETURN + 2),
        ],
        tempindex(),
    )
}

#[test]
fn stov_parses_identically() {
    let _g = lock();
    for s in [
        "1 2 3",
        "1.5 -2.5 3.25",
        "",
        "   ",
        "7",
        "7 8",
        "1,2,3",
        "\"1 2\" 3",
        "0.1 0.2 0.3",
        "1e10 -1e10 1e-10",
        "nan inf -inf",
        "  +4   -5   +6  ",
        "abc def ghi",
        "1abc 2def 3ghi",
        "999999999999999999999 1 1",
    ] {
        // NaN never compares equal, so the halves are compared bitwise.
        let c = run_stov(Side::C, s);
        let rs = run_stov(Side::Rust, s);
        assert_eq!(c.0, rs.0, "{s:?} raised");
        assert_eq!(c.2, rs.2, "{s:?} tempindex");
        for i in 0..3 {
            assert_eq!(
                c.1[i].to_bits(),
                rs.1[i].to_bits(),
                "{s:?} component {i}: {} vs {}",
                c.1[i],
                rs.1[i]
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Scenarios: PF_strftime.

fn run_strftime(side: Side, local: f32, fmt: &str) -> (bool, String, c_int) {
    reset(4);
    let h = intern(fmt);
    set_global_f(OFS_PARM0, local);
    set_global(OFS_PARM1, h);
    set_argc(2);
    let status = invoke(side, pf::STRFTIME);
    let ret = get_global(OFS_RETURN);
    (status != 0, read_string(ret), tempindex())
}

#[test]
fn strftime_literal_formats_match_exactly() {
    let _g = lock();
    // Wall-clock-free formats, so these are exactly reproducible.
    for fmt in ["quake", "", "%%", "a%%b", "no specifiers here"] {
        for local in [0.0f32, 1.0] {
            let c = run_strftime(Side::C, local, fmt);
            let rs = run_strftime(Side::Rust, local, fmt);
            assert_eq!(c, rs, "fmt {fmt:?} local {local}");
        }
    }
}

#[test]
fn strftime_clock_formats_match() {
    let _g = lock();
    // These do read the wall clock. The two halves run microseconds apart, so
    // in principle a field could tick between them; the coarsest available
    // fields are used to make that window as small as it can be while still
    // exercising real conversion specifiers -- and %F / %R specifically cover
    // the `#ifdef _WIN32` rewrite in pr_ext.c:1804-1809 that the Rust port
    // reproduces in `strftime_fmt_fixup`.
    for fmt in ["%Y", "%Y-%m-%d", "%F", "%R", "%j", "%B"] {
        for local in [0.0f32, 1.0] {
            let c = run_strftime(Side::C, local, fmt);
            let rs = run_strftime(Side::Rust, local, fmt);
            assert!(!c.0, "strftime must not raise");
            assert_eq!(c, rs, "fmt {fmt:?} local {local}");
        }
    }
    // %F and %R must actually expand, not pass through as literals.
    assert_ne!(run_strftime(Side::Rust, 0.0, "%F").1, "%F");
    assert_ne!(run_strftime(Side::Rust, 0.0, "%R").1, "%R");
}

// ---------------------------------------------------------------------------
// Raise topology.

#[test]
fn out_of_range_string_handle_is_silently_tolerated() {
    let _g = lock();
    // COMPAT: PR_GetString's invalid-offset arm returns `qcvm->strings` and the
    // Host_Error behind it is dead code (pr_edict_arena.c:319-322), so a
    // positive handle past `stringssize` resolves to the empty string at the
    // head of the blob rather than raising. `resolve_string` in
    // quake-progs/src/arena.rs keeps that; this pins it end to end.
    for which in [pf::INFOGET, pf::TOKENIZE, pf::STOV] {
        let mut obs = Vec::new();
        for side in [Side::C, Side::Rust] {
            reset(4);
            set_global(OFS_PARM0, 1 << 20);
            set_global(OFS_PARM1, 1 << 20);
            set_argc(2);
            let status = invoke(side, which);
            obs.push((status != 0, get_global(OFS_RETURN), tempindex()));
        }
        assert!(!obs[0].0, "which {which} must not raise");
        assert_eq!(obs[0], obs[1], "which {which}");
    }
}

#[test]
fn cleared_known_string_raises_on_both_sides() {
    let _g = lock();
    // The one live raise in PR_GetString: a negative handle inside the
    // knownstrings range whose slot is NULL. The C side reports it through
    // Host_Guard's status, the Rust side through PRBI_ERR_NO_STRING; the codes
    // are different by construction (ADR-009), so only "did it raise", the
    // temp-ring position and the console are compared.
    for which in [pf::INFOADD, pf::INFOGET, pf::TOKENIZE, pf::STOV] {
        let mut obs = Vec::new();
        for side in [Side::C, Side::Rust] {
            reset(4);
            // SAFETY: installs a static 4-entry knownstrings table on the
            // fixture qcvm. Safe because every group B builtin resolves its
            // arguments before interning anything, so the raise below happens
            // before PR_SetEngineString could try to Z_Realloc that table.
            let bad = unsafe { ctest_pr_ext_strext_arm_bad_string() };
            set_global(OFS_PARM0, bad);
            set_global(OFS_PARM1, bad);
            set_global(OFS_PARM2, bad);
            set_argc(3);
            let status = invoke(side, which);
            obs.push((status != 0, tempindex(), console()));
        }
        assert!(obs[0].0, "the C side must raise for {which}");
        assert_eq!(obs[0], obs[1], "which {which}");
    }
}
