//! Differential test: `Quake/pr_ext.c`'s FRIK_FILE + string-buffer group
//! against the Rust port in `quake-capi/src/progs_builtins_filebuf.rs`. Rust
//! migration Phase 7, M9f, group C (`pr_ext.c:3130-3773`).
//!
//! # How a comparison is made
//!
//! Every scenario runs twice from a freshly reset fixture -- once through
//! `stubs/pr_ext_ref.c`'s `ctest_cref_pr_ext_run_m9f_c` (the real,
//! statically-scoped `pr_ext.c` bodies, inside `Host_Guard`), once through the
//! `quake_rs_*` entry points -- and the two observations must be equal. This
//! follows `pr_ext_differential.rs`; the differences are all forced by what
//! this group owns:
//!
//! * **Per-side teardown.** `qcfiles` / `qcfiles_max` and `strbuflist` are
//!   `pr_ext.c` file statics that outlive `ctest_pr_ext_reset_fixture`'s
//!   `memset` of the qcvm, and the port's copies are module statics that
//!   outlive it too. `reset_side` therefore drives the side under test through
//!   its own `PF_frikfile_shutdown` / `PF_buf_shutdown` before each half, so
//!   handle numbering starts from the same place on both sides.
//! * **Observation through the builtins, not through accessors.** M9d could
//!   read `qcvm->knownzone` because the bitmap stayed in the qcvm and both
//!   sides shared it. This group's tables did *not* stay shared -- the C
//!   statics and the Rust statics are separate objects -- so a C-side accessor
//!   could only ever report the C side. Everything here is observed through
//!   the group's own public surface instead: return values, `bufstr_get` /
//!   `buf_getsize` / `buf_implode` / `fgets` read-back, the console log, and
//!   (for the file half) the bytes actually on disk. That is a real coverage
//!   boundary and is spelled out per scenario.
//!
//! # Real filesystem I/O
//!
//! The file half is not stubbed. `Sys_fopen` / `Sys_fseek` / `Sys_ftell` /
//! `Sys_FileType` in `stubs/stubs.c` are the genuine libc calls, and
//! `Quake/common_fs.c` is in `build.rs`'s `C_SOURCES`, so `COM_FOpenFile`
//! really walks `com_searchpaths`. `ctest_pr_ext_fs_setup` points `com_gamedir`
//! at a private temp directory, creates the `data/` subdirectory `PF_fopen`
//! writes into, and mounts that directory as the one non-pak searchpath. Each
//! side gets its own directory, so a read scenario never sees the other side's
//! bytes.
//!
//! Raise topology (ADR-009): the C side is driven through `Host_Guard`; the
//! Rust side is status-returning. `Host_Guard`'s success value and `PRBI_OK`
//! are both 0, so statuses compare directly.

use core::ffi::{c_char, c_float, c_int, c_uint, CStr};
use std::sync::{Mutex, MutexGuard, Once};

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

/// A value planted in `OFS_RETURN` before a call that is expected not to write
/// it, so "left untouched" is observable rather than assumed.
const SENTINEL: c_int = 0x5eed_1234;

/// `ctest_cref_pr_ext_dispatch_m9f_c`'s switch indices (`stubs/pr_ext_ref.c`,
/// the `M9F GROUP C` block). 40-59 are reserved for this group.
mod pf {
    pub const FOPEN: i32 = 40;
    pub const FGETS: i32 = 41;
    pub const FPUTS: i32 = 42;
    pub const FCLOSE: i32 = 43;
    pub const FSEEK: i32 = 44;
    pub const WHICHPACK: i32 = 45;
    pub const BUF_CREATE: i32 = 46;
    pub const BUF_DEL: i32 = 47;
    pub const BUF_GETSIZE: i32 = 48;
    pub const BUF_COPY: i32 = 49;
    pub const BUF_SORT: i32 = 50;
    pub const BUF_IMPLODE: i32 = 51;
    pub const BUFSTR_GET: i32 = 52;
    pub const BUFSTR_SET: i32 = 53;
    pub const BUFSTR_ADD: i32 = 54;
    pub const BUFSTR_FREE: i32 = 55;
    pub const BUF_CVARLIST: i32 = 56;
    pub const FRIKFILE_SHUTDOWN: i32 = 57;
    pub const BUF_SHUTDOWN: i32 = 58;
}

/// `cvar.h` `cvar_t`, only as much of it as `Cvar_RegisterVariable` and
/// `PF_buf_cvarlist` touch. Laid out to match `generated.rs`'s `cvar_s`.
#[repr(C)]
struct CvarT {
    name: *const c_char,
    string: *const c_char,
    flags: c_uint,
    value: c_float,
    default_string: *const c_char,
    callback: Option<unsafe extern "C" fn(*mut CvarT)>,
    completion: Option<unsafe extern "C" fn(*mut CvarT, *const c_char)>,
    next: *mut CvarT,
}

extern "C" {
    // --- fixture (stubs/pr_ext_ref.c) ------------------------------------
    fn ctest_pr_ext_reset_fixture(num_edicts: c_int);
    fn ctest_pr_ext_intern(s: *const c_char) -> c_int;
    fn ctest_pr_ext_set_argc(argc: c_int);
    fn ctest_pr_ext_set_global_int(ofs: c_int, v: c_int);
    fn ctest_pr_ext_get_global_int(ofs: c_int) -> c_int;
    fn ctest_pr_ext_get_string(handle: c_int) -> *const c_char;
    fn ctest_pr_ext_fs_setup(dir: *const c_char);
    fn ctest_pr_ext_fs_teardown();
    fn ctest_pr_ext_cvar_register(var: *mut CvarT);
    fn ctest_pr_ext_rs_fs_setup(dir: *const c_char);
    fn ctest_pr_ext_rs_fs_teardown();
    fn ctest_pr_ext_rs_cvar_register(var: *mut CvarT) -> c_int;

    // --- oracle dispatcher (stubs/pr_ext_ref.c) --------------------------
    fn ctest_cref_pr_ext_run_m9f_c(which: c_int) -> c_int;

    // --- console capture (stubs.c) ---------------------------------------
    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(i: c_int) -> *const c_char;

    // --- the Rust port under test ----------------------------------------
    fn quake_rs_pf_fopen(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_fgets(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_fputs(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_fclose(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_fseek(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_whichpack(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_buf_create(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_buf_del(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_buf_getsize(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_buf_copy(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_buf_sort(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_buf_implode(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_bufstr_get(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_bufstr_set(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_bufstr_add(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_bufstr_free(detail: *mut c_int) -> c_int;
    fn quake_rs_pf_buf_cvarlist(detail: *mut c_int) -> c_int;
    fn quake_rs_pr_frikfile_shutdown(detail: *mut c_int) -> c_int;
    fn quake_rs_pr_buf_shutdown(detail: *mut c_int) -> c_int;
}

// ---------------------------------------------------------------------------
// Safe wrappers. Every scenario below is safe code; the FFI justification for
// the whole file lives here (ADR-004). The fixture owns a private qcvm that
// `reset` reinitialises before each half of a comparison, the tests are
// serialised on `TEST_LOCK`, and no pointer handed across the boundary
// outlives the call it is passed to.

fn reset(num_edicts: c_int) {
    // SAFETY: no arguments to validate; the fixture allocates and zeroes its
    // own state. Serialised by `TEST_LOCK`.
    unsafe { ctest_pr_ext_reset_fixture(num_edicts) }
}

fn intern(s: &str) -> c_int {
    let c = std::ffi::CString::new(s).expect("no interior NUL");
    // SAFETY: `c` is NUL-terminated and outlives the call; the fixture copies
    // into its own pool.
    unsafe { ctest_pr_ext_intern(c.as_ptr()) }
}

fn set_argc(argc: c_int) {
    // SAFETY: a plain int the fixture stores in `qcvm->argc`.
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

/// `G_FLOAT` through the same 32-bit slot `G_INT` uses -- the fixture only
/// exposes the integer accessor, and a QC global is a union of the two.
fn set_global_f(ofs: c_int, v: f32) {
    set_global(ofs, v.to_bits() as c_int);
}

fn get_global_f(ofs: c_int) -> f32 {
    f32::from_bits(get_global(ofs) as u32)
}

fn read_string(handle: c_int) -> Option<String> {
    if handle == 0 {
        return None;
    }
    // SAFETY: `handle` was produced by this same string table during this
    // scenario and has not been cleared. The pointer is NUL-terminated and
    // stays valid until the next `reset`.
    unsafe {
        let p = ctest_pr_ext_get_string(handle);
        assert!(!p.is_null(), "PR_GetString returned null for {handle}");
        Some(CStr::from_ptr(p).to_string_lossy().into_owned())
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

/// Drops the `[dcon2] PR_AllocStringSlots: ...` line, which the known-string
/// table emits on either side depending only on how many handles have been
/// interned so far and is not part of what a builtin does.
fn console_filtered() -> Vec<String> {
    console()
        .into_iter()
        .filter(|l| !l.starts_with("[dcon2] "))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    C,
    Rust,
}

impl Side {
    fn tag(self) -> &'static str {
        match self {
            Side::C => "c",
            Side::Rust => "rs",
        }
    }
}

/// Runs one builtin on one side and returns its status. Zero means "no raise"
/// on both sides: `Host_Guard`'s success value and `PRBI_OK` are both 0.
fn invoke(side: Side, which: i32) -> c_int {
    match side {
        // SAFETY: `which` is one of the dispatcher indices in `mod pf`, and
        // the C body runs inside `Host_Guard`, so a `Host_Error` unwinds in a
        // C frame and never longjmps past this call (ADR-009).
        Side::C => unsafe { ctest_cref_pr_ext_run_m9f_c(which) },
        Side::Rust => {
            let mut detail: c_int = 0;
            // SAFETY: `detail` is a live, initialised `c_int`; these entry
            // points are status-returning and read the ambient qcvm the
            // fixture has just reset (ADR-008).
            unsafe {
                match which {
                    pf::FOPEN => quake_rs_pf_fopen(&mut detail),
                    pf::FGETS => quake_rs_pf_fgets(&mut detail),
                    pf::FPUTS => quake_rs_pf_fputs(&mut detail),
                    pf::FCLOSE => quake_rs_pf_fclose(&mut detail),
                    pf::FSEEK => quake_rs_pf_fseek(&mut detail),
                    pf::WHICHPACK => quake_rs_pf_whichpack(&mut detail),
                    pf::BUF_CREATE => quake_rs_pf_buf_create(&mut detail),
                    pf::BUF_DEL => quake_rs_pf_buf_del(&mut detail),
                    pf::BUF_GETSIZE => quake_rs_pf_buf_getsize(&mut detail),
                    pf::BUF_COPY => quake_rs_pf_buf_copy(&mut detail),
                    pf::BUF_SORT => quake_rs_pf_buf_sort(&mut detail),
                    pf::BUF_IMPLODE => quake_rs_pf_buf_implode(&mut detail),
                    pf::BUFSTR_GET => quake_rs_pf_bufstr_get(&mut detail),
                    pf::BUFSTR_SET => quake_rs_pf_bufstr_set(&mut detail),
                    pf::BUFSTR_ADD => quake_rs_pf_bufstr_add(&mut detail),
                    pf::BUFSTR_FREE => quake_rs_pf_bufstr_free(&mut detail),
                    pf::BUF_CVARLIST => quake_rs_pf_buf_cvarlist(&mut detail),
                    pf::FRIKFILE_SHUTDOWN => quake_rs_pr_frikfile_shutdown(&mut detail),
                    pf::BUF_SHUTDOWN => quake_rs_pr_buf_shutdown(&mut detail),
                    _ => panic!("bad dispatch index {which}"),
                }
            }
        }
    }
}

/// Resets the qcvm *and* the side's own file/buffer tables. Both are `pr_ext.c`
/// file statics (C) or module statics (Rust) that survive the qcvm memset, so
/// without this a scenario would inherit the previous scenario's handles.
fn reset_side(side: Side) {
    reset(4);
    assert_eq!(invoke(side, pf::FRIKFILE_SHUTDOWN), 0);
    assert_eq!(invoke(side, pf::BUF_SHUTDOWN), 0);
    reset(4); // clears the console log the teardowns may have written to
}

/* ---------------------------------------------------------------------------
 * String-buffer helpers. Each returns the builtin's own observable result, so
 * the same helper drives both sides.
 */

fn buf_create(side: Side, type_: Option<&str>) -> f32 {
    match type_ {
        Some(t) => {
            set_global(OFS_PARM0, intern(t));
            set_argc(1);
        }
        None => set_argc(0),
    }
    set_global(OFS_RETURN, SENTINEL);
    assert_eq!(invoke(side, pf::BUF_CREATE), 0);
    get_global_f(OFS_RETURN)
}

fn bufstr_add(side: Side, buf: f32, s: &str, ordered: f32) -> f32 {
    set_global_f(OFS_PARM0, buf);
    set_global(OFS_PARM1, intern(s));
    set_global_f(OFS_PARM2, ordered);
    set_argc(3);
    set_global(OFS_RETURN, SENTINEL);
    assert_eq!(invoke(side, pf::BUFSTR_ADD), 0);
    get_global_f(OFS_RETURN)
}

fn bufstr_set(side: Side, buf: f32, index: f32, s: &str) {
    set_global_f(OFS_PARM0, buf);
    set_global_f(OFS_PARM1, index);
    set_global(OFS_PARM2, intern(s));
    set_argc(3);
    assert_eq!(invoke(side, pf::BUFSTR_SET), 0);
}

fn bufstr_free(side: Side, buf: f32, index: f32) {
    set_global_f(OFS_PARM0, buf);
    set_global_f(OFS_PARM1, index);
    set_argc(2);
    assert_eq!(invoke(side, pf::BUFSTR_FREE), 0);
}

fn bufstr_get(side: Side, buf: f32, index: f32) -> Option<String> {
    set_global_f(OFS_PARM0, buf);
    set_global_f(OFS_PARM1, index);
    set_argc(2);
    set_global(OFS_RETURN, SENTINEL);
    assert_eq!(invoke(side, pf::BUFSTR_GET), 0);
    read_string(get_global(OFS_RETURN))
}

/// `buf_getsize` returns `SENTINEL` unchanged on a dead or out-of-range
/// handle: C returns without touching `OFS_RETURN` at all.
fn buf_getsize_raw(side: Side, buf: f32) -> c_int {
    set_global_f(OFS_PARM0, buf);
    set_argc(1);
    set_global(OFS_RETURN, SENTINEL);
    assert_eq!(invoke(side, pf::BUF_GETSIZE), 0);
    get_global(OFS_RETURN)
}

fn buf_getsize(side: Side, buf: f32) -> f32 {
    f32::from_bits(buf_getsize_raw(side, buf) as u32)
}

fn buf_sort(side: Side, buf: f32, prefixlen: f32, backwards: f32) {
    set_global_f(OFS_PARM0, buf);
    set_global_f(OFS_PARM1, prefixlen);
    set_global_f(OFS_PARM2, backwards);
    set_argc(3);
    assert_eq!(invoke(side, pf::BUF_SORT), 0);
}

fn buf_implode(side: Side, buf: f32, glue: &str) -> Option<String> {
    set_global_f(OFS_PARM0, buf);
    set_global(OFS_PARM1, intern(glue));
    set_argc(2);
    set_global(OFS_RETURN, 0);
    assert_eq!(invoke(side, pf::BUF_IMPLODE), 0);
    read_string(get_global(OFS_RETURN))
}

fn buf_copy(side: Side, from: f32, to: f32) {
    set_global_f(OFS_PARM0, from);
    set_global_f(OFS_PARM1, to);
    set_argc(2);
    assert_eq!(invoke(side, pf::BUF_COPY), 0);
}

fn buf_del(side: Side, buf: f32) {
    set_global_f(OFS_PARM0, buf);
    set_argc(1);
    assert_eq!(invoke(side, pf::BUF_DEL), 0);
}

fn buf_cvarlist(side: Side, buf: f32, pattern: &str, antipattern: &str) {
    set_global_f(OFS_PARM0, buf);
    set_global(OFS_PARM1, intern(pattern));
    set_global(OFS_PARM2, intern(antipattern));
    set_argc(3);
    assert_eq!(invoke(side, pf::BUF_CVARLIST), 0);
}

/// Every live slot of a buffer, read back through `bufstr_get`.
fn dump(side: Side, buf: f32) -> Vec<Option<String>> {
    let n = buf_getsize(side, buf) as i32;
    (0..n).map(|i| bufstr_get(side, buf, i as f32)).collect()
}

/* ---------------------------------------------------------------------------
 * FRIK_FILE helpers.
 */

fn fopen(side: Side, name: &str, mode: f32) -> f32 {
    set_global(OFS_PARM0, intern(name));
    set_global_f(OFS_PARM1, mode);
    set_argc(2);
    set_global(OFS_RETURN, SENTINEL);
    assert_eq!(invoke(side, pf::FOPEN), 0);
    get_global_f(OFS_RETURN)
}

fn fputs(side: Side, handle: f32, s: &str) {
    set_global_f(OFS_PARM0, handle);
    set_global(OFS_PARM1, intern(s));
    set_argc(2);
    assert_eq!(invoke(side, pf::FPUTS), 0);
}

fn fgets(side: Side, handle: f32) -> Option<String> {
    set_global_f(OFS_PARM0, handle);
    set_argc(1);
    set_global(OFS_RETURN, 0);
    assert_eq!(invoke(side, pf::FGETS), 0);
    read_string(get_global(OFS_RETURN))
}

/// `fseek` with one argument reports the position; with two it also seeks.
fn ftell(side: Side, handle: f32) -> c_int {
    set_global_f(OFS_PARM0, handle);
    set_argc(1);
    set_global(OFS_RETURN, SENTINEL);
    assert_eq!(invoke(side, pf::FSEEK), 0);
    get_global(OFS_RETURN)
}

fn fseek(side: Side, handle: f32, pos: c_int) -> c_int {
    set_global_f(OFS_PARM0, handle);
    set_global(OFS_PARM1, pos);
    set_argc(2);
    set_global(OFS_RETURN, SENTINEL);
    assert_eq!(invoke(side, pf::FSEEK), 0);
    get_global(OFS_RETURN)
}

fn fclose(side: Side, handle: f32) {
    set_global_f(OFS_PARM0, handle);
    set_argc(1);
    assert_eq!(invoke(side, pf::FCLOSE), 0);
}

/// A private `com_gamedir` for one side of one scenario, torn down on drop so
/// a failing assertion cannot leave the searchpath mounted for the next test.
struct FsFixture {
    dir: std::path::PathBuf,
    side: Side,
}

impl FsFixture {
    fn new(side: Side, scenario: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "vkq_m9fc_{}_{}_{}",
            std::process::id(),
            scenario,
            side.tag()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("data")).expect("temp dir");
        let c = std::ffi::CString::new(dir.to_str().expect("utf-8 temp path")).expect("no NUL");
        // SAFETY: `c` is NUL-terminated and outlives the call, which copies it
        // into the side's `com_gamedir` and into its own `searchpath_t`.
        unsafe {
            match side {
                Side::C => ctest_pr_ext_fs_setup(c.as_ptr()),
                Side::Rust => ctest_pr_ext_rs_fs_setup(c.as_ptr()),
            }
        }
        Self { dir, side }
    }

    fn read(&self, rel: &str) -> Option<Vec<u8>> {
        std::fs::read(self.dir.join("data").join(rel)).ok()
    }
}

impl Drop for FsFixture {
    fn drop(&mut self) {
        // SAFETY: restores the globals `new` saved; no arguments.
        unsafe {
            match self.side {
                Side::C => ctest_pr_ext_fs_teardown(),
                Side::Rust => ctest_pr_ext_rs_fs_teardown(),
            }
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/* ---------------------------------------------------------------------------
 * Scenarios. Each `run_*` is side-agnostic; each `#[test]` runs it twice and
 * compares.
 */

#[derive(Debug, PartialEq)]
struct Obs {
    values: Vec<String>,
    console: Vec<String>,
}

fn obs(values: Vec<String>) -> Obs {
    Obs {
        values,
        console: console_filtered(),
    }
}

fn compare(f: impl Fn(Side) -> Obs) {
    let _g = lock();
    let c = f(Side::C);
    let rs = f(Side::Rust);
    assert_eq!(c, rs);
}

// --- buffer lifecycle ------------------------------------------------------

fn run_create_and_del(side: Side) -> Obs {
    reset_side(side);
    let mut v = Vec::new();
    // default type (argc 0 -> "string"), then an explicit "string", then a
    // rejected type.
    let a = buf_create(side, None);
    let b = buf_create(side, Some("string"));
    let bad = buf_create(side, Some("banana"));
    v.push(format!("{a} {b} {bad}"));

    // `buf_del` frees the slot, and the next create reuses the lowest free one.
    buf_del(side, a);
    let c = buf_create(side, None);
    v.push(format!("{c}"));

    // A dead handle leaves OFS_RETURN alone -- C's `return` never writes it.
    buf_del(side, c);
    buf_del(side, b);
    v.push(format!("{}", buf_getsize_raw(side, b)));
    obs(v)
}

#[test]
fn create_and_del_match() {
    compare(run_create_and_del);
}

fn run_bad_handles(side: Side) -> Obs {
    reset_side(side);
    let mut v = Vec::new();
    // handle 0 is `bufno = -1`, which C reads as 0xffffffff and rejects. A
    // saturating f32->u32 cast would turn it into buffer 0 instead; this is
    // the regression test for that narrowing (see `buf_no`'s COMPAT note).
    let live = buf_create(side, None);
    bufstr_add(side, live, "occupied", 1.0);
    // NaN is deliberately absent: float->int conversion of a NaN is UB in C
    // (x86 yields INT_MIN) while Rust's `as` defines it as 0, so it is not a
    // comparable case. Every value here is inside i32's range, which is the
    // range clang's cvttss2si actually reproduces.
    for h in [0.0f32, -1.0, -1e9, 65.0, 1e9] {
        v.push(format!("{h} -> {}", buf_getsize_raw(side, h)));
        v.push(format!("{h} get -> {:?}", bufstr_get(side, h, 0.0)));
    }
    // the live buffer must be untouched by all of that
    v.push(format!("{:?}", dump(side, live)));
    obs(v)
}

#[test]
fn bad_handles_match() {
    compare(run_bad_handles);
}

fn run_add_get_free(side: Side) -> Obs {
    reset_side(side);
    let b = buf_create(side, None);
    let mut v = Vec::new();
    for s in ["alpha", "bravo", "charlie", ""] {
        v.push(format!("{}", bufstr_add(side, b, s, 1.0)));
    }
    v.push(format!("size {}", buf_getsize(side, b)));
    v.push(format!("{:?}", dump(side, b)));

    // free a middle slot: the hole stays, size does not shrink
    bufstr_free(side, b, 1.0);
    v.push(format!("size {}", buf_getsize(side, b)));
    v.push(format!("{:?}", dump(side, b)));

    // ordered = 0 finds the hole; ordered = 1 appends
    v.push(format!("hole {}", bufstr_add(side, b, "delta", 0.0)));
    v.push(format!("end  {}", bufstr_add(side, b, "echo", 1.0)));
    v.push(format!("{:?}", dump(side, b)));

    // freeing past `used` is a no-op, and a second free of a hole too
    bufstr_free(side, b, 99.0);
    bufstr_free(side, b, 1.0);
    v.push(format!("{:?}", dump(side, b)));

    // `qboolean ordered` is `bool`, so any non-zero float appends
    v.push(format!("half {}", bufstr_add(side, b, "foxtrot", 0.5)));
    obs(v)
}

#[test]
fn add_get_free_match() {
    compare(run_add_get_free);
}

fn run_sparse_set(side: Side) -> Obs {
    reset_side(side);
    let b = buf_create(side, None);
    let mut v = Vec::new();
    // `bufstr_set` past `allocated` grows to index + 256 and zero-fills, so
    // `used` becomes index + 1 with a long run of NULL holes behind it.
    bufstr_set(side, b, 300.0, "far");
    v.push(format!("size {}", buf_getsize(side, b)));
    v.push(format!("{:?}", bufstr_get(side, b, 300.0)));
    v.push(format!("{:?}", bufstr_get(side, b, 299.0)));
    v.push(format!("{:?}", bufstr_get(side, b, 301.0)));

    // overwriting frees the old string first
    bufstr_set(side, b, 300.0, "replaced");
    v.push(format!("{:?}", bufstr_get(side, b, 300.0)));

    // a hole fill inside the existing allocation
    bufstr_set(side, b, 5.0, "near");
    v.push(format!("size {}", buf_getsize(side, b)));
    v.push(format!("{:?}", bufstr_get(side, b, 5.0)));

    // ordered = 0 now finds slot 0, the first hole
    v.push(format!("hole {}", bufstr_add(side, b, "first", 0.0)));
    obs(v)
}

#[test]
fn sparse_set_match() {
    compare(run_sparse_set);
}

// --- the sort (ADR-010, the group's headline) ------------------------------

/// The words are chosen so that a `sortprefixlen` of 2 makes several of them
/// compare *equal* -- `strncmp` returns 0 on a shared prefix -- which is
/// exactly the tie case whose ordering `qsort` decides. Both sides must agree
/// on that ordering, which they can only do if the same platform `qsort` with
/// the same comparator signs runs on both.
const TIE_WORDS: &[&str] = &[
    "prefix_zulu",
    "prefix_alpha",
    "praline",
    "prefix_mike",
    "quark",
    "prawn",
    "prefix_bravo",
    "quiche",
    "prefix_alpha",
];

fn run_sort(side: Side, prefixlen: f32, backwards: f32) -> Obs {
    reset_side(side);
    let b = buf_create(side, None);
    for w in TIE_WORDS {
        bufstr_add(side, b, w, 1.0);
    }
    // a hole in the middle: buf_sort compacts NULLs out before sorting, so
    // `used` shrinks by one.
    bufstr_free(side, b, 4.0);
    buf_sort(side, b, prefixlen, backwards);
    let mut v = vec![format!("size {}", buf_getsize(side, b))];
    v.push(format!("{:?}", dump(side, b)));
    v.push(format!("{:?}", buf_implode(side, b, "|")));
    obs(v)
}

#[test]
fn sort_full_ascending_matches() {
    compare(|s| run_sort(s, 0.0, 0.0));
}

#[test]
fn sort_full_descending_matches() {
    compare(|s| run_sort(s, 0.0, 1.0));
}

#[test]
fn sort_tied_prefix_ascending_matches() {
    compare(|s| run_sort(s, 2.0, 0.0));
}

#[test]
fn sort_tied_prefix_descending_matches() {
    compare(|s| run_sort(s, 2.0, 1.0));
}

#[test]
fn sort_negative_prefix_matches() {
    // <= 0 is rewritten to INT_MAX, i.e. a full comparison
    compare(|s| run_sort(s, -3.0, 0.0));
}

// --- implode ---------------------------------------------------------------

fn run_implode(side: Side) -> Obs {
    reset_side(side);
    let b = buf_create(side, None);
    let mut v = Vec::new();

    // The glue test is `if (retlen)`, not "is this the first element", so a
    // leading empty string suppresses the glue before the *second* element
    // too. That quirk is asserted here.
    for s in ["", "one", "two", ""] {
        bufstr_add(side, b, s, 1.0);
    }
    bufstr_free(side, b, 2.0);
    v.push(format!("{:?}", buf_implode(side, b, "--")));
    v.push(format!("{:?}", buf_implode(side, b, "")));

    // overflow: the loop breaks and keeps whatever it had written so far
    let big = buf_create(side, None);
    for i in 0..40 {
        bufstr_add(side, big, &format!("{i:0>40}"), 1.0);
    }
    let out = buf_implode(side, big, "....");
    v.push(format!("len {:?}", out.as_ref().map(|s| s.len())));
    v.push(format!("{out:?}"));

    // an empty buffer implodes to ""
    let empty = buf_create(side, None);
    v.push(format!("{:?}", buf_implode(side, empty, "x")));

    // a dead handle leaves OFS_RETURN as the caller set it (0 here)
    buf_del(side, empty);
    v.push(format!("{:?}", buf_implode(side, empty, "x")));
    obs(v)
}

#[test]
fn implode_match() {
    compare(run_implode);
}

// --- copy ------------------------------------------------------------------

fn run_copy(side: Side) -> Obs {
    reset_side(side);
    let a = buf_create(side, None);
    let b = buf_create(side, None);
    let mut v = Vec::new();
    for s in ["one", "two", "three"] {
        bufstr_add(side, a, s, 1.0);
    }
    bufstr_free(side, a, 1.0); // a NULL survives the copy as a NULL
    bufstr_add(side, b, "clobbered", 1.0);

    buf_copy(side, a, b);
    v.push(format!("{:?}", dump(side, b)));
    v.push(format!("{:?}", dump(side, a)));

    // self-copy is a no-op, and is tested *before* the range checks, so even a
    // pair of equal bogus handles returns quietly
    buf_copy(side, a, a);
    v.push(format!("{:?}", dump(side, a)));
    buf_copy(side, 999.0, 999.0);
    buf_copy(side, a, 999.0);
    buf_copy(side, 999.0, a);
    v.push(format!("{:?}", dump(side, a)));

    // the copy is deep: mutating the source does not touch the destination
    bufstr_set(side, a, 0.0, "mutated");
    v.push(format!("{:?}", dump(side, b)));
    obs(v)
}

#[test]
fn copy_match() {
    compare(run_copy);
}

// --- cvarlist --------------------------------------------------------------

static CVAR_INIT: Once = Once::new();

const CVAR_NAMES: &[&str] = &[
    "m9fc_zulu\0",
    "m9fc_alpha\0",
    "m9fc_mike\0",
    "m9fc_skip_me\0",
    "m9fd_other\0",
];

fn new_cvar(name: &'static str) -> *mut CvarT {
    Box::leak(Box::new(CvarT {
        name: name.as_ptr().cast::<c_char>(),
        string: c"0".as_ptr(),
        flags: 0,
        value: 0.0,
        default_string: std::ptr::null(),
        callback: None,
        completion: None,
        next: std::ptr::null_mut(),
    }))
}

/// Registers a fixed set of cvars exactly once for the whole process.
///
/// There are two registries in this link -- cvar.c's (which the oracle reaches
/// as `c_ref_Cvar_FindVarAfter`) and quake-capi's `CVAR_VARS` (which the port
/// reaches) -- and a `cvar_t` carries the list's `next` pointer inside itself,
/// so one object cannot live in both. Each name therefore gets two independent
/// leaked `cvar_t`s, one per registry.
fn register_cvars() {
    CVAR_INIT.call_once(|| {
        for name in CVAR_NAMES {
            // SAFETY: both cvars are leaked, so each registry's `next` chain
            // stays valid for the life of the process; `name` and `string` are
            // 'static NUL-terminated bytes.
            unsafe {
                ctest_pr_ext_cvar_register(new_cvar(name));
                assert_eq!(ctest_pr_ext_rs_cvar_register(new_cvar(name)), 0);
            }
        }
    });
}

fn run_cvarlist(side: Side) -> Obs {
    register_cvars();
    reset_side(side);
    let mut v = Vec::new();

    // Each listing gets its own *fresh* buffer. PF_buf_cvarlist frees
    // strbuflist[].strings and then leaves the pointer dangling with
    // used = allocated = 0, so a second cvarlist (or a buf_del) on a buffer
    // that already holds a listing hands a freed pointer to Mem_Realloc /
    // Mem_Free. That is genuine UB in the C, so the differential stays off it;
    // the deviation is reported instead of being asserted.
    let listing = |pattern: &str, antipattern: &str| {
        let b = buf_create(side, None);
        buf_cvarlist(side, b, pattern, antipattern);
        format!("{pattern:?} !{antipattern:?} -> {:?}", dump(side, b))
    };

    // literal prefix, no wildcard
    v.push(listing("m9fc_", ""));
    // literal prefix plus a literal antipattern
    v.push(listing("m9fc_", "m9fc_s"));
    // wildcards on both sides
    v.push(listing("m9f?_*", "*_me"));
    // An empty pattern is skipped entirely (`plen &&`) and would list the whole
    // registry -- but the two registries also hold whatever unrelated engine
    // cvars each side's init happened to register, so only the m9f-prefixed
    // patterns above are comparable. Noted as a coverage limit rather than
    // asserted.

    // adding after a listing is fine -- `strings` is live at that point -- and
    // exercises the grown allocation the listing left behind
    let b = buf_create(side, None);
    buf_cvarlist(side, b, "m9fc_", "");
    v.push(format!("{}", bufstr_add(side, b, "appended", 1.0)));
    v.push(format!("{:?}", dump(side, b)));
    obs(v)
}

#[test]
fn cvarlist_match() {
    compare(run_cvarlist);
}

/// `PF_buf_cvarlist` never sets `PF_buf_sort_sortprefixlen`; it sorts with
/// whatever the last `PF_buf_sort` left in the global. This drives a sort with
/// a short prefix first, so the cvar list that follows is sorted under a
/// comparator that ties everything -- the quirk the port keeps a process
/// global to preserve.
fn run_cvarlist_stale_prefix(side: Side) -> Obs {
    register_cvars();
    reset_side(side);
    let b = buf_create(side, None);
    for w in TIE_WORDS {
        bufstr_add(side, b, w, 1.0);
    }
    buf_sort(side, b, 3.0, 0.0);

    let l = buf_create(side, None);
    buf_cvarlist(side, l, "m9f", "");
    obs(vec![format!("{:?}", dump(side, l))])
}

#[test]
fn cvarlist_stale_prefix_matches() {
    compare(run_cvarlist_stale_prefix);
}

// --- buf_shutdown ----------------------------------------------------------

fn run_buf_shutdown(side: Side) -> Obs {
    reset_side(side);
    let a = buf_create(side, None);
    let b = buf_create(side, None);
    bufstr_add(side, a, "one", 1.0);
    bufstr_add(side, b, "two", 1.0);

    assert_eq!(invoke(side, pf::BUF_SHUTDOWN), 0);

    let mut v = vec![
        format!("{}", buf_getsize_raw(side, a)),
        format!("{}", buf_getsize_raw(side, b)),
    ];
    // both slots are free again, so the next creates reuse them in order
    v.push(format!(
        "{} {}",
        buf_create(side, None),
        buf_create(side, None)
    ));
    obs(v)
}

#[test]
fn buf_shutdown_match() {
    compare(run_buf_shutdown);
}

// --- files: paths that need no filesystem ----------------------------------

fn run_fopen_rejects(side: Side) -> Obs {
    reset_side(side);
    let mut v = Vec::new();
    // QC_FixFileName rejects each of these, and the message prints the
    // *unfixed* name.
    for bad in ["", "c:/evil", "back\\slash", "/absolute", "up/../down"] {
        v.push(format!("{bad:?} -> {}", fopen(side, bad, 0.0)));
    }
    // a name that passes the filter but has no file behind it
    v.push(format!(
        "missing -> {}",
        fopen(side, "nothing_here.txt", 0.0)
    ));
    // unsupported modes
    for m in [3.0f32, -1.0, 7.0] {
        v.push(format!("mode {m} -> {}", fopen(side, "ok.txt", m)));
    }
    obs(v)
}

#[test]
fn fopen_rejects_match() {
    compare(run_fopen_rejects);
}

fn run_file_bad_handles(side: Side) -> Obs {
    reset_side(side);
    let mut v = Vec::new();
    // handle 0 is `fileid = -1`, which C reads as SIZE_MAX. As with `buf_no`,
    // a saturating cast would alias file 0; this is that regression test.
    for h in [0.0f32, -1.0, 1.0, 1e9] {
        v.push(format!("{h} fgets  {:?}", fgets(side, h)));
        v.push(format!("{h} ftell  {}", ftell(side, h)));
        fputs(side, h, "x");
        fclose(side, h);
    }
    obs(v)
}

#[test]
fn file_bad_handles_match() {
    compare(run_file_bad_handles);
}

fn run_whichpack_missing(side: Side) -> Obs {
    reset_side(side);
    let _fs = FsFixture::new(side, "whichpack");
    set_global(OFS_PARM0, intern("definitely/not/here.txt"));
    set_argc(1);
    set_global(OFS_RETURN, SENTINEL);
    assert_eq!(invoke(side, pf::WHICHPACK), 0);
    obs(vec![format!("{}", get_global(OFS_RETURN))])
}

#[test]
fn whichpack_missing_match() {
    compare(run_whichpack_missing);
}

// --- files: real I/O -------------------------------------------------------

fn run_write_then_read(side: Side) -> Obs {
    reset_side(side);
    let fs = FsFixture::new(side, "roundtrip");
    let mut v = Vec::new();

    // mode 2 = write (truncating)
    let w = fopen(side, "round.txt", 2.0);
    v.push(format!("open w {w}"));
    v.push(format!("tell0 {}", ftell(side, w)));
    fputs(side, w, "first line\n");
    fputs(side, w, "second\r\n");
    fputs(side, w, "no newline at eof");
    v.push(format!("tell1 {}", ftell(side, w)));
    // reading from a write handle is refused
    v.push(format!("fgets on w {:?}", fgets(side, w)));
    fclose(side, w);

    v.push(format!("bytes {:?}", fs.read("round.txt")));

    // mode 0 = read, through COM_FOpenFile and the mounted searchpath
    let r = fopen(side, "round.txt", 0.0);
    v.push(format!("open r {r}"));
    v.push(format!("l1 {:?}", fgets(side, r)));
    v.push(format!("tell {}", ftell(side, r)));
    v.push(format!("l2 {:?}", fgets(side, r)));
    v.push(format!("l3 {:?}", fgets(side, r)));
    v.push(format!("eof {:?}", fgets(side, r)));
    // writing to a read handle is refused, with PF_fgets's message
    fputs(side, r, "nope");
    // seek back to the top and re-read
    v.push(format!("seek {}", fseek(side, r, 0)));
    v.push(format!("again {:?}", fgets(side, r)));
    // seek into the middle of a line
    v.push(format!("seek6 {}", fseek(side, r, 6)));
    v.push(format!("mid {:?}", fgets(side, r)));
    fclose(side, r);
    // double close warns
    fclose(side, r);
    obs(v)
}

#[test]
fn write_then_read_match() {
    compare(run_write_then_read);
}

fn run_append_truncates(side: Side) -> Obs {
    reset_side(side);
    let fs = FsFixture::new(side, "append");
    let mut v = Vec::new();

    let w = fopen(side, "app.txt", 2.0);
    fputs(side, w, "original contents\n");
    fclose(side, w);
    v.push(format!("before {:?}", fs.read("app.txt")));

    // mode 1 is documented as "append" but opens "w+b", which truncates. The
    // port keeps that; this asserts it.
    let a = fopen(side, "app.txt", 1.0);
    v.push(format!("open a {a}"));
    v.push(format!("tell {}", ftell(side, a)));
    fputs(side, a, "appended?\n");
    fclose(side, a);
    v.push(format!("after {:?}", fs.read("app.txt")));
    obs(v)
}

#[test]
fn append_truncates_match() {
    compare(run_append_truncates);
}

fn run_long_line_truncation(side: Side) -> Obs {
    reset_side(side);
    let fs = FsFixture::new(side, "longline");
    let mut v = Vec::new();

    // > STRINGTEMP_LENGTH on one line, so `PF_fgets`'s `if (s == end) s--`
    // rewind runs and the line is truncated to the last byte read.
    let long: String = std::iter::repeat_n("0123456789", 200).collect();
    let w = fopen(side, "long.txt", 2.0);
    fputs(side, w, &long);
    fputs(side, w, "\ntail\n");
    fclose(side, w);
    v.push(format!("wrote {:?}", fs.read("long.txt").map(|b| b.len())));

    let r = fopen(side, "long.txt", 0.0);
    let l1 = fgets(side, r);
    v.push(format!("len {:?}", l1.as_ref().map(|s| s.len())));
    v.push(format!("l1 {l1:?}"));
    v.push(format!("l2 {:?}", fgets(side, r)));
    fclose(side, r);
    obs(v)
}

#[test]
fn long_line_truncation_matches() {
    compare(run_long_line_truncation);
}

fn run_frikfile_shutdown(side: Side) -> Obs {
    reset_side(side);
    let _fs = FsFixture::new(side, "shutdown");
    let mut v = Vec::new();

    let a = fopen(side, "a.txt", 2.0);
    let b = fopen(side, "b.txt", 2.0);
    v.push(format!("{a} {b}"));
    fputs(side, a, "aaa\n");

    assert_eq!(invoke(side, pf::FRIKFILE_SHUTDOWN), 0);

    // both handles are closed now
    v.push(format!("{:?}", fgets(side, a)));
    v.push(format!("{:?}", fgets(side, b)));
    // and the slots are reused from the bottom
    v.push(format!("reopen {}", fopen(side, "c.txt", 2.0)));
    obs(v)
}

#[test]
fn frikfile_shutdown_match() {
    compare(run_frikfile_shutdown);
}
