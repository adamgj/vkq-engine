//! Differential test: `Quake/pr_ext.c`'s zoned-string trio against the Rust
//! port in `rust/quake-capi/src/progs_builtins_zone.rs`. Rust migration
//! Phase 7, M9f, task T9f.0.
//!
//! # What this file is for
//!
//! T9f.0 is the gate-first half of M9f (ADR-019): its real deliverable is
//! `stubs/pr_ext_ref.c`, which makes `pr_ext.c` a differential-oracle
//! translation unit for the first time (`docs/rust-migration/ROADMAP.md:158`
//! records "`pr_ext.c` is in no oracle" as coverage gap #1). This suite exists
//! to prove that oracle is wired correctly end to end -- reset, argument
//! marshalling, `Host_Guard` status, string-table and console observation --
//! **not** to cover pr_ext.c. The M9f port wave adds the coverage.
//!
//! # Why these three builtins and no others
//!
//! `quake-ctest`'s `quake-capi` dependency enables `host` + `progs-host` but
//! NOT `progs`. `progs_builtins_zone.rs` is gated
//! `#[cfg(all(feature = "host", feature = "progs-host"))]`, so its three entry
//! points are linked here. Everything else pr_ext.c-shaped that is already
//! Rust -- `quake_rs_pf_stof` / `stoi` / `stoh` / `itos` / `htos` / `ftoi` /
//! `itof` / `strlen` / `str2chr` / `strstrofs` in
//! `quake-capi/src/progs_builtins.rs` -- is `#[cfg(feature = "progs")]` and is
//! therefore not compiled into these test binaries at all. Those were tried
//! first and dropped for that reason: comparing them needs
//! `quake-ctest/Cargo.toml` to turn on `progs`, which also pulls in
//! quake-capi's `pr_edict_arena` shim on top of the `pr_edict_arena.c` oracle
//! this link already has, so it is not a one-line change and is out of T9f.0's
//! scope.
//!
//! This is genuinely new coverage rather than a restatement: M9d flipped
//! `PF_strzone` / `PF_strunzone` / `PR_UnzoneAll` with no C oracle available,
//! so until now nothing compared them against `pr_ext.c` itself.
//!
//! # How a comparison is made
//!
//! Each scenario runs twice from a freshly reset fixture -- once through
//! `stubs/pr_ext_ref.c`'s `ctest_cref_pr_ext_run` (the real, statically-scoped
//! `pr_ext.c` bodies), once through `quake_rs_pf_*` -- and the two
//! observations must be equal. Both sides intern through the same
//! `c_ref_PR_SetEngineString` (`stubs.c:6002-6019` forwards the plain-named
//! entry points the Rust port imports straight back to the `c_ref_*` pair) and
//! `ctest_world_reset` memsets the whole qcvm, so handles are handed out in the
//! same order on both sides and are directly comparable.
//!
//! Raise topology (ADR-009): the C side is driven through `Host_Guard`, which
//! arms the `Host_Error` trap in a C frame; the Rust side is status-returning
//! (`quake_rs_pf_*(&mut detail)`). No `longjmp` crosses a Rust frame.

use core::ffi::{c_char, c_int, CStr};
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// `pr_comp.h`'s reserved global offsets.
const OFS_RETURN: c_int = 1;
const OFS_PARM0: c_int = 4;

/// `ctest_cref_pr_ext_dispatch`'s switch indices (`stubs/pr_ext_ref.c`).
mod pf {
    pub const STRZONE: i32 = 0;
    pub const STRUNZONE: i32 = 1;
    pub const UNZONE_ALL: i32 = 2;
}

extern "C" {
    // --- fixture (stubs/pr_ext_ref.c) ------------------------------------
    fn ctest_pr_ext_reset_fixture(num_edicts: c_int);
    fn ctest_pr_ext_intern(s: *const c_char) -> c_int;
    fn ctest_pr_ext_set_argc(argc: c_int);
    fn ctest_pr_ext_set_global_int(ofs: c_int, v: c_int);
    fn ctest_pr_ext_get_global_int(ofs: c_int) -> c_int;
    fn ctest_pr_ext_get_string(handle: c_int) -> *const c_char;
    fn ctest_pr_ext_knownzone_size() -> usize;
    fn ctest_pr_ext_knownzone_allocated() -> c_int;
    fn ctest_pr_ext_knownzone_test(id: usize) -> c_int;

    // --- oracle dispatcher (stubs/pr_ext_ref.c) --------------------------
    fn ctest_cref_pr_ext_run(which: c_int) -> c_int;

    // --- console capture (stubs.c) ---------------------------------------
    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(i: c_int) -> *const c_char;

    // --- the Rust port under test ----------------------------------------
    fn quake_rs_pf_strzone(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_strunzone(detail: *mut c_int) -> c_int;
    fn quake_rs_pr_unzone_all(detail: *mut c_int) -> c_int;
}

// ---------------------------------------------------------------------------
// Safe wrappers. Every scenario below is safe code; the FFI justification for
// the whole file lives here (ADR-004). The fixture owns a private qcvm that
// `reset` reinitialises before each half of a comparison, the tests are
// serialised on `TEST_LOCK`, and no pointer handed across the boundary
// outlives the call it is passed to.

/// Reinitialises the fixture's qcvm, string pool and console log.
fn reset(num_edicts: c_int) {
    // SAFETY: no arguments to validate; the fixture allocates and zeroes its
    // own state. Serialised by `TEST_LOCK`.
    unsafe { ctest_pr_ext_reset_fixture(num_edicts) }
}

/// Copies `s` into the fixture's string blob and returns its `string_t`.
fn intern(s: &str) -> c_int {
    let c = std::ffi::CString::new(s).expect("no interior NUL");
    // SAFETY: `c` is NUL-terminated and outlives the call; the fixture only
    // reads it (it copies into its own pool).
    unsafe { ctest_pr_ext_intern(c.as_ptr()) }
}

fn set_argc(argc: c_int) {
    // SAFETY: `argc` is a plain int the fixture range-checks against its own
    // globals block.
    unsafe { ctest_pr_ext_set_argc(argc) }
}

fn set_global(ofs: c_int, v: c_int) {
    // SAFETY: `ofs` is one of pr_comp.h's reserved offsets, inside the
    // globals block the fixture allocated.
    unsafe { ctest_pr_ext_set_global_int(ofs, v) }
}

fn get_global(ofs: c_int) -> c_int {
    // SAFETY: as `set_global`.
    unsafe { ctest_pr_ext_get_global_int(ofs) }
}

fn zone_size() -> usize {
    // SAFETY: a plain read of `qcvm->knownzonesize`.
    unsafe { ctest_pr_ext_knownzone_size() }
}

fn zone_allocated() -> c_int {
    // SAFETY: a plain null-test of `qcvm->knownzone`.
    unsafe { ctest_pr_ext_knownzone_allocated() }
}

fn zone_test(id: usize) -> c_int {
    // SAFETY: the fixture bounds-checks `id` against `knownzonesize` itself,
    // exactly as `PF_strunzone` does.
    unsafe { ctest_pr_ext_knownzone_test(id) }
}

fn read_string(handle: c_int) -> String {
    // SAFETY: `handle` was produced by this same string table and has not been
    // cleared (no caller reads a handle back after `PF_strunzone`; see `Obs`).
    // The returned pointer is NUL-terminated and stays valid until the next
    // `reset`, which cannot run before this copy completes.
    unsafe {
        let p = ctest_pr_ext_get_string(handle);
        assert!(!p.is_null(), "PR_GetString returned null for {handle}");
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

fn console() -> Vec<String> {
    // SAFETY: `ctest_con_log_len` bounds the index, and each entry is a
    // NUL-terminated buffer owned by the log until the next `reset`.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    C,
    Rust,
}

/// Runs one builtin on one side and returns its status. Zero means "no raise"
/// on both sides: `Host_Guard`'s success value and `PRBI_OK` are both 0.
fn invoke(side: Side, which: i32) -> c_int {
    match side {
        // SAFETY: `which` is one of the three dispatcher indices below, and
        // the C body runs inside `Host_Guard`, so a `Host_Error` unwinds in a
        // C frame and never longjmps past this call (ADR-009).
        Side::C => unsafe { ctest_cref_pr_ext_run(which) },
        Side::Rust => {
            let mut detail: c_int = 0;
            // SAFETY: `detail` is a live, initialised `c_int`; these entry
            // points are status-returning and read the ambient qcvm the
            // fixture has just reset (ADR-008).
            unsafe {
                match which {
                    pf::STRZONE => quake_rs_pf_strzone(&mut detail),
                    pf::STRUNZONE => quake_rs_pf_strunzone(&mut detail),
                    pf::UNZONE_ALL => quake_rs_pr_unzone_all(&mut detail),
                    _ => panic!("bad dispatch index {which}"),
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------

/// Everything a scenario compares. `text` is deliberately optional: after
/// `PF_strunzone` the handle's known-string slot is NULL, and reading it back
/// would make `PR_GetString` `Host_Error` outside any guard.
#[derive(Debug, PartialEq, Eq)]
struct Obs {
    status: c_int,
    ret: c_int,
    text: Option<String>,
    zone_size: usize,
    zone_allocated: c_int,
    zone_bit: Option<c_int>,
    console: Vec<String>,
}

/// `pr_ext.c:584` `id = -1 - G_INT (OFS_RETURN)`, in the same wrapping
/// arithmetic (`int` subtraction, then widened to `size_t`).
fn zone_id(handle: c_int) -> usize {
    (-1i32).wrapping_sub(handle) as usize
}

/// `PF_strzone` over `args`, from a fresh fixture.
fn run_strzone(side: Side, args: &[&str]) -> Obs {
    reset(4);
    for (i, a) in args.iter().enumerate() {
        let h = intern(a);
        set_global(OFS_PARM0 + (i as c_int) * 3, h);
    }
    set_argc(args.len() as c_int);

    let status = invoke(side, pf::STRZONE);
    let ret = get_global(OFS_RETURN);
    Obs {
        status,
        ret,
        text: Some(read_string(ret)),
        zone_size: zone_size(),
        zone_allocated: zone_allocated(),
        zone_bit: Some(zone_test(zone_id(ret))),
        console: console(),
    }
}

/// `PF_strzone`, then `PF_strunzone` on the handle it returned.
fn run_strzone_then_strunzone(side: Side) -> Obs {
    reset(4);
    let h = intern("zoned payload");
    set_global(OFS_PARM0, h);
    set_argc(1);
    assert_eq!(invoke(side, pf::STRZONE), 0, "setup strzone must not raise");
    let zoned = get_global(OFS_RETURN);

    set_global(OFS_PARM0, zoned);
    set_argc(1);
    let status = invoke(side, pf::STRUNZONE);
    Obs {
        status,
        ret: zoned,
        text: None,
        zone_size: zone_size(),
        zone_allocated: zone_allocated(),
        zone_bit: Some(zone_test(zone_id(zoned))),
        console: console(),
    }
}

/// `PF_strunzone` on a handle that was never strzoned (or on 0).
fn run_strunzone_raw(side: Side, handle_of: Option<&str>) -> Obs {
    reset(4);
    let h = match handle_of {
        Some(s) => intern(s),
        None => 0,
    };
    set_global(OFS_PARM0, h);
    set_argc(1);
    let status = invoke(side, pf::STRUNZONE);
    Obs {
        status,
        ret: h,
        text: None,
        zone_size: zone_size(),
        zone_allocated: zone_allocated(),
        zone_bit: None,
        console: console(),
    }
}

/// Three `PF_strzone` calls, then `PR_UnzoneAll`.
fn run_unzone_all(side: Side) -> Obs {
    reset(4);
    let mut last = 0;
    for s in ["alpha", "beta", "gamma"] {
        let h = intern(s);
        set_global(OFS_PARM0, h);
        set_argc(1);
        assert_eq!(invoke(side, pf::STRZONE), 0, "setup strzone must not raise");
        last = get_global(OFS_RETURN);
    }

    let status = invoke(side, pf::UNZONE_ALL);
    Obs {
        status,
        ret: last,
        text: None,
        zone_size: zone_size(),
        zone_allocated: zone_allocated(),
        zone_bit: None,
        console: console(),
    }
}

// ---------------------------------------------------------------------------

#[test]
fn strzone_single_argument_matches() {
    let _g = lock();
    let c = run_strzone(Side::C, &["hello world"]);
    let rs = run_strzone(Side::Rust, &["hello world"]);
    assert_eq!(c.status, 0);
    assert_eq!(c.text.as_deref(), Some("hello world"));
    assert_eq!(c.zone_bit, Some(1), "the knownzone bit must be set");
    assert_eq!(c, rs);
}

#[test]
fn strzone_concatenates_all_arguments() {
    let _g = lock();
    let args = ["abc", "", "XY", "!"];
    let c = run_strzone(Side::C, &args);
    let rs = run_strzone(Side::Rust, &args);
    assert_eq!(c.text.as_deref(), Some("abcXY!"));
    assert_eq!(c, rs);
}

#[test]
fn strzone_with_no_arguments_matches() {
    let _g = lock();
    let c = run_strzone(Side::C, &[]);
    let rs = run_strzone(Side::Rust, &[]);
    assert_eq!(c.text.as_deref(), Some(""), "argc 0 zones the empty string");
    assert_eq!(c, rs);
}

#[test]
fn strunzone_releases_a_zoned_string_identically() {
    let _g = lock();
    let c = run_strzone_then_strunzone(Side::C);
    let rs = run_strzone_then_strunzone(Side::Rust);
    assert_eq!(c.zone_bit, Some(0), "the knownzone bit must be cleared");
    assert_eq!(
        c.zone_allocated, 1,
        "strunzone only clears a bit; only PR_UnzoneAll frees the bitmap"
    );
    assert!(
        !c.console.iter().any(|l| l.contains("wasn't strzoned")),
        "no warning on a legitimate unzone (got {:?})",
        c.console
    );
    assert_eq!(c, rs);
}

#[test]
fn strunzone_of_a_null_handle_is_a_no_op_on_both_sides() {
    let _g = lock();
    let c = run_strunzone_raw(Side::C, None);
    let rs = run_strunzone_raw(Side::Rust, None);
    assert_eq!(c.ret, 0);
    assert_eq!(c.zone_size, 0);
    assert!(
        c.console.is_empty(),
        "the null handle returns before warning"
    );
    assert_eq!(c, rs);
}

#[test]
fn strunzone_of_an_unzoned_handle_warns_identically() {
    let _g = lock();
    let c = run_strunzone_raw(Side::C, Some("never zoned"));
    let rs = run_strunzone_raw(Side::Rust, Some("never zoned"));
    assert_eq!(
        c.console,
        vec!["[warn] PF_strunzone: string wasn't strzoned\n".to_string()],
        "the [warn] tag is stubs.c's Con_Warning marker, not part of the text"
    );
    assert_eq!(c, rs);
}

#[test]
fn unzone_all_tears_the_bitmap_down_identically() {
    let _g = lock();
    let c = run_unzone_all(Side::C);
    let rs = run_unzone_all(Side::Rust);
    assert_eq!(c.zone_size, 0, "PR_UnzoneAll resets knownzonesize");
    assert_eq!(c.zone_allocated, 0, "PR_UnzoneAll frees the bitmap");
    assert_eq!(c, rs);
}
