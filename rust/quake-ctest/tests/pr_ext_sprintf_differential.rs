//! Differential test: `Quake/pr_ext.c`'s `PF_sprintf` / `PF_sprintf_internal`
//! (`:1110-1589`) against the Rust port in
//! `rust/quake-capi/src/progs_builtins_sprintf.rs`. Rust migration Phase 7,
//! M9f, group A.
//!
//! # Shape
//!
//! Same construction as `pr_ext_differential.rs`: each scenario runs twice from
//! a freshly reset fixture -- once through `stubs/pr_ext_ref.c`'s
//! `ctest_m9fa_run` (the real, statically-scoped `pr_ext.c` body inside
//! `Host_Guard`), once through `quake_rs_pf_sprintf` (status-returning) -- and
//! the two observations must be equal. No `longjmp` crosses a Rust frame
//! (ADR-009).
//!
//! # What is *deliberately* not compared
//!
//! * **`%o`, `%e`, `%E`, `%g`, `%G`, `%v`, `%V`.** `quake_util::printf`
//!   implements neither octal nor the exponent conversions (ADR-005), so the
//!   port raises instead of formatting them. `unsupported_conversions_diverge`
//!   pins that divergence explicitly rather than hiding it: it asserts the C
//!   side formats and the Rust side raises, which is exactly why the `sprintf`
//!   builtin row must stay C.
//! * **Console line *order*.** `SvConsole` defers warnings to `run_sv`'s flush,
//!   which lands after `PR_SetEngineString`, so a `PR_AllocStringSlots`
//!   `Con_DPrintf2` line can interleave differently. The comparison therefore
//!   filters the log to `[warn]` lines, whose text and count do match.
//! * **A `%` directive that starts with the output buffer already full.** C
//!   skips its `if (o < end - 1)` block and then `++s` walks past the format
//!   string's NUL -- a genuine read past the end of the progs string. Driving
//!   it here would be driving C into UB, so it is reported as a finding
//!   instead (see the port module's docs).
//! * **`strtol` overflow (`%99999999999d`).** The result is
//!   `(int)LONG_MAX`, which differs between the 32-bit-`long` and
//!   64-bit-`long` platforms; the port models it with `c_long`, but exercising
//!   it means handing the platform `snprintf` a two-billion-wide field. Left
//!   untested.

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

/// `ctest_m9fa_dispatch`'s switch indices (`stubs/pr_ext_ref.c`, the
/// `M9F GROUP A` block; 10-19 are this group's reserved range).
const PF_SPRINTF: c_int = 10;

extern "C" {
    // --- fixture (stubs/pr_ext_ref.c) ------------------------------------
    fn ctest_pr_ext_reset_fixture(num_edicts: c_int);
    fn ctest_pr_ext_intern(s: *const c_char) -> c_int;
    fn ctest_pr_ext_set_argc(argc: c_int);
    fn ctest_pr_ext_set_global_int(ofs: c_int, v: c_int);
    fn ctest_pr_ext_get_global_int(ofs: c_int) -> c_int;
    fn ctest_pr_ext_get_string(handle: c_int) -> *const c_char;
    fn ctest_m9fa_set_global_float(ofs: c_int, v: f32);

    // --- oracle dispatcher (stubs/pr_ext_ref.c, M9F GROUP A) --------------
    fn ctest_m9fa_run(which: c_int) -> c_int;

    // --- console capture (stubs.c) ---------------------------------------
    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(i: c_int) -> *const c_char;

    // --- the Rust port under test ----------------------------------------
    fn quake_rs_pf_sprintf(detail: *mut c_int) -> c_int;
}

// ---------------------------------------------------------------------------
// Safe wrappers. The FFI justification for the whole file lives here (ADR-004):
// the fixture owns a private qcvm that `reset` reinitialises before each half
// of a comparison, the tests are serialised on `TEST_LOCK`, and no pointer
// handed across the boundary outlives the call it is passed to.

fn reset(num_edicts: c_int) {
    // SAFETY: no arguments to validate; the fixture allocates and zeroes its
    // own state. Serialised by `TEST_LOCK`.
    unsafe { ctest_pr_ext_reset_fixture(num_edicts) }
}

fn intern(s: &str) -> c_int {
    let c = std::ffi::CString::new(s).expect("no interior NUL");
    // SAFETY: `c` is NUL-terminated and outlives the call; the fixture copies
    // it into its own pool.
    unsafe { ctest_pr_ext_intern(c.as_ptr()) }
}

fn set_argc(argc: c_int) {
    // SAFETY: a plain int stored into `qcvm->argc`.
    unsafe { ctest_pr_ext_set_argc(argc) }
}

fn set_global(ofs: c_int, v: c_int) {
    // SAFETY: every `ofs` below is `OFS_PARM0 + 3 * n` for `n <= 8`, well
    // inside the `globalvars_t`-sized block the fixture allocated.
    unsafe { ctest_pr_ext_set_global_int(ofs, v) }
}

fn set_global_float(ofs: c_int, v: f32) {
    // SAFETY: as `set_global`.
    unsafe { ctest_m9fa_set_global_float(ofs, v) }
}

fn get_global(ofs: c_int) -> c_int {
    // SAFETY: as `set_global`.
    unsafe { ctest_pr_ext_get_global_int(ofs) }
}

fn read_string(handle: c_int) -> String {
    // SAFETY: `handle` came from this same string table and is only read while
    // it is live; the pointer is NUL-terminated and stays valid until the next
    // `reset`, which cannot run before this copy completes.
    unsafe {
        let p = ctest_pr_ext_get_string(handle);
        assert!(!p.is_null(), "PR_GetString returned null for {handle}");
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

/// The console log, filtered to `Con_Warning` lines -- see the module docs for
/// why the unfiltered order is not comparable.
fn warnings() -> Vec<String> {
    // SAFETY: `ctest_con_log_len` bounds the index and each entry is a
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
    all.into_iter()
        .filter(|l| l.starts_with("[warn]"))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    C,
    Rust,
}

fn invoke(side: Side) -> c_int {
    match side {
        // SAFETY: index 10 is this group's `PF_sprintf` case, and the C body
        // runs inside `Host_Guard`, so a `Host_Error` unwinds in a C frame and
        // never longjmps past this call (ADR-009).
        Side::C => unsafe { ctest_m9fa_run(PF_SPRINTF) },
        Side::Rust => {
            let mut detail: c_int = 0;
            // SAFETY: `detail` is a live, initialised `c_int`; the entry point
            // is status-returning and reads the ambient qcvm the fixture has
            // just reset (ADR-008).
            unsafe { quake_rs_pf_sprintf(&mut detail) }
        }
    }
}

// ---------------------------------------------------------------------------

/// One `sprintf` argument, written into `OFS_PARM0 + 3 * (n + 1)`.
#[derive(Clone, Copy, Debug)]
enum Val<'a> {
    /// A QC float, the default interpretation of every numeric directive.
    F(f32),
    /// Raw int bits, for the `l`/`L` (`isfloat = 0`) directives.
    I(i32),
    /// A string, interned into the fixture's blob.
    S(&'a str),
    /// A raw 64-bit payload spanning two globals, for the `q` directives.
    Q(u64),
}

/// Everything a scenario compares.
#[derive(Debug, PartialEq, Eq)]
struct Obs {
    status: c_int,
    /// `G_INT (OFS_RETURN)`; both sides intern through the same
    /// `c_ref_PR_SetEngineString` from the same reset state, so the handles are
    /// directly comparable.
    ret: c_int,
    /// `None` when the builtin raised: C never reached
    /// `PR_SetEngineString`, so `OFS_RETURN` holds stale data.
    text: Option<String>,
    warnings: Vec<String>,
}

fn run_sprintf(side: Side, fmt: &str, args: &[Val]) -> Obs {
    reset(4);
    set_global(OFS_PARM0, intern(fmt));
    for (n, a) in args.iter().enumerate() {
        let ofs = OFS_PARM0 + 3 * (n as c_int + 1);
        match *a {
            Val::F(v) => set_global_float(ofs, v),
            Val::I(v) => set_global(ofs, v),
            Val::S(s) => set_global(ofs, intern(s)),
            Val::Q(v) => {
                set_global(ofs, v as u32 as i32);
                set_global(ofs + 1, (v >> 32) as u32 as i32);
            }
        }
    }
    set_argc(args.len() as c_int + 1);

    let status = invoke(side);
    let ret = get_global(OFS_RETURN);
    Obs {
        status,
        ret,
        text: if status == 0 {
            Some(read_string(ret))
        } else {
            None
        },
        warnings: warnings(),
    }
}

/// Runs one case on both sides and asserts they agree. Returns the shared
/// observation so a caller can additionally pin the exact text.
#[track_caller]
fn same(fmt: &str, args: &[Val]) -> Obs {
    let c = run_sprintf(Side::C, fmt, args);
    let rs = run_sprintf(Side::Rust, fmt, args);
    assert_eq!(c, rs, "sprintf({fmt:?}, {args:?})");
    c
}

#[track_caller]
fn same_text(fmt: &str, args: &[Val], expect: &str) {
    let o = same(fmt, args);
    assert_eq!(o.status, 0, "sprintf({fmt:?}) raised");
    assert_eq!(o.text.as_deref(), Some(expect), "sprintf({fmt:?})");
}

// ---------------------------------------------------------------------------
// Literals and `%%`.

#[test]
fn literal_text_and_percent_escape() {
    let _g = lock();
    same_text("hello world", &[], "hello world");
    same_text("", &[], "");
    same_text("100%% sure", &[], "100% sure");
    same_text("%%%%", &[], "%%");
}

// ---------------------------------------------------------------------------
// Integer conversions.

#[test]
fn signed_integer_conversions() {
    let _g = lock();
    same_text("%d", &[Val::F(3.0)], "3");
    same_text("%d", &[Val::F(-42.75)], "-42");
    same_text("%d", &[Val::F(0.0)], "0");
    // `%i` defaults to ints, not floats (pr_ext.c:1341).
    same_text("%i", &[Val::I(-7)], "-7");
    // `l` forces the int interpretation for `%d` too.
    same_text("%ld", &[Val::I(123_456)], "123456");
}

#[test]
fn integer_flags_width_and_precision() {
    let _g = lock();
    same_text("[%5d]", &[Val::F(42.0)], "[   42]");
    same_text("[%-5d]", &[Val::F(42.0)], "[42   ]");
    same_text("[%05d]", &[Val::F(42.0)], "[00042]");
    same_text("[%+d]", &[Val::F(42.0)], "[+42]");
    same_text("[% d]", &[Val::F(42.0)], "[ 42]");
    same_text("[%.5d]", &[Val::F(42.0)], "[00042]");
    same_text("[%8.5d]", &[Val::F(-42.0)], "[  -00042]");
    // C's rule: '0' is ignored when a precision is given.
    same_text("[%08.5d]", &[Val::F(42.0)], "[   00042]");
    // A bare `0` before a non-digit is just the zero-pad flag (pr_ext.c:1187).
    same_text("[%0d]", &[Val::F(42.0)], "[42]");
}

#[test]
fn unsigned_and_hex_conversions() {
    let _g = lock();
    same_text("%x", &[Val::F(255.0)], "ff");
    same_text("%X", &[Val::F(255.0)], "FF");
    same_text("%#x", &[Val::F(255.0)], "0xff");
    // `#` is dropped for a zero value, as C does.
    same_text("%#x", &[Val::F(0.0)], "0");
    same_text("%08x", &[Val::F(255.0)], "000000ff");
    same_text("%u", &[Val::F(4000.0)], "4000");
    // `GETARG_UINT` is a zero-extended 32-bit read (pr_ext.c:1139).
    same_text("%lu", &[Val::I(-1)], "4294967295");
    same_text("%lx", &[Val::I(-1)], "ffffffff");
    // `%p` forces zero-pad and width 8 (pr_ext.c:1331-1339).
    same_text("%p", &[Val::I(255)], "000000ff");
    same_text("%P", &[Val::I(255)], "000000FF");
}

#[test]
fn sixty_four_bit_conversions() {
    let _g = lock();
    // `lq` -> GETARG_INT64 / GETARG_UINT64 over two consecutive globals.
    same_text("%lqd", &[Val::Q(1_234_567_890_123u64)], "1234567890123");
    same_text("%lqd", &[Val::Q((-5i64) as u64)], "-5");
    same_text("%lqx", &[Val::Q(0xdead_beef_1234_5678)], "deadbeef12345678");
    // `q` alone keeps the float default, i.e. GETARG_DOUBLE.
    same_text("%qd", &[Val::Q(1234.75f64.to_bits())], "1234");
    same_text("%qf", &[Val::Q(0.5f64.to_bits())], "0.500000");
}

// ---------------------------------------------------------------------------
// Float conversions.

#[test]
fn float_conversions() {
    let _g = lock();
    same_text("%f", &[Val::F(1.5)], "1.500000");
    same_text("%.2f", &[Val::F(1.0 / 3.0)], "0.33");
    same_text("%.0f", &[Val::F(2.5)], "2");
    same_text("[%10.2f]", &[Val::F(-1.25)], "[     -1.25]");
    same_text("[%-10.2f]", &[Val::F(-1.25)], "[-1.25     ]");
    same_text("[%010.2f]", &[Val::F(-1.25)], "[-000001.25]");
    same_text("[%+.1f]", &[Val::F(1.25)], "[+1.2]");
    same_text("[%#.0f]", &[Val::F(3.0)], "[3.]");
    same_text("%F", &[Val::F(2.0)], "2.000000");
    // `l` switches to the int interpretation, then widens to double.
    same_text("%lf", &[Val::I(7)], "7.000000");
}

// ---------------------------------------------------------------------------
// Characters and strings.

#[test]
fn character_conversion() {
    let _g = lock();
    same_text("%c", &[Val::F(65.0)], "A");
    same_text("[%3c]", &[Val::F(66.0)], "[  B]");
    same_text("[%-3c]", &[Val::F(66.0)], "[B  ]");
    same_text("%lc", &[Val::I(67)], "C");
    // `%c` of 0 writes a NUL, and `o += strlen (o)` then does not advance --
    // the 'b' overwrites it (pr_ext.c:1470).
    same_text("a%cb", &[Val::F(0.0)], "ab");
}

#[test]
fn string_conversion() {
    let _g = lock();
    same_text("%s", &[Val::S("quake")], "quake");
    same_text("[%10s]", &[Val::S("quake")], "[     quake]");
    same_text("[%-10s]", &[Val::S("quake")], "[quake     ]");
    same_text("[%.3s]", &[Val::S("quake")], "[qua]");
    same_text("[%8.3s]", &[Val::S("quake")], "[     qua]");
    same_text("%s%s", &[Val::S("ab"), Val::S("cd")], "abcd");
    same_text("%s", &[Val::S("")], "");
}

#[test]
fn tokenizable_string_conversion() {
    let _g = lock();
    // No escape needed: C hands `%s` `quotedbuf + 1`, dropping the leading '\'.
    same_text("%S", &[Val::S("plain")], "\"plain\"");
    same_text("%S", &[Val::S("")], "\"\"");

    // An escape was emitted: C keeps the leading '\' *and* warns.
    let o = same("%S", &[Val::S("say \"hi\"")]);
    assert_eq!(o.status, 0);
    assert_eq!(o.text.as_deref(), Some("\\\"say \\\"hi\\\"\""));
    assert_eq!(o.warnings.len(), 1, "{:?}", o.warnings);
    // C reports `thisarg + 1`, and `thisarg` is already the OFS_PARM slot
    // (firstarg == 1, the format string being parm 0), so the *first* vararg
    // is reported as "arg: 2". Transcribed as-is.
    assert!(
        o.warnings[0].contains("unable to safely escape arg: 2"),
        "{:?}",
        o.warnings
    );

    let o = same("%S", &[Val::S("two\nlines\r")]);
    assert_eq!(o.text.as_deref(), Some("\\\"two\\nlines\\r\""));
    assert_eq!(o.warnings.len(), 1);
}

// ---------------------------------------------------------------------------
// Argument selection: positional `%N$`, `*` width/precision, missing args.

#[test]
fn positional_arguments() {
    let _g = lock();
    same_text("%2$s-%1$s", &[Val::S("a"), Val::S("b")], "b-a");
    // A positional directive does not advance argpos, so the plain `%s` still
    // takes argument 1 (pr_ext.c:1181, which never touches argpos).
    same_text("%2$s %s", &[Val::S("a"), Val::S("b")], "b a");
    same_text("[%1$5s]", &[Val::S("q")], "[    q]");
}

#[test]
fn star_width_and_precision() {
    let _g = lock();
    same_text("[%*d]", &[Val::F(6.0), Val::F(42.0)], "[    42]");
    // A negative `*` width flips to left-alignment (pr_ext.c:1240).
    same_text("[%*d]", &[Val::F(-6.0), Val::F(42.0)], "[42    ]");
    same_text("[%.*f]", &[Val::F(3.0), Val::F(1.0 / 3.0)], "[0.333]");
    // A negative `*` precision reads as "not set" (the `precision >= 0` tests).
    same_text("[%.*f]", &[Val::F(-2.0), Val::F(1.5)], "[1.500000]");
    same_text(
        "[%*.*f]",
        &[Val::F(9.0), Val::F(2.0), Val::F(1.5)],
        "[     1.50]",
    );
    // Positional `*`: `%2$*1$d` takes the width from argument 1.
    same_text("[%2$*1$d]", &[Val::F(6.0), Val::F(42.0)], "[    42]");
}

#[test]
fn missing_arguments_default_to_zero_and_empty() {
    let _g = lock();
    // argc is 1 (the format string only), so every GETARG_* is out of range.
    same_text("%d|%s|%f|%x", &[], "0||0.000000|0");
    // Past the end of a short argument list, too.
    same_text("%s %s", &[Val::S("one")], "one ");
}

// ---------------------------------------------------------------------------
// Malformed directives.

#[test]
fn invalid_conversion_warns_and_stops() {
    let _g = lock();
    let o = same("ok %y rest", &[]);
    assert_eq!(o.status, 0);
    assert_eq!(o.text.as_deref(), Some("ok "));
    assert_eq!(o.warnings.len(), 1, "{:?}", o.warnings);
    assert!(
        o.warnings[0].contains("invalid format string: %y rest"),
        "{:?}",
        o.warnings
    );

    // `%I` takes the length-prefix arm (pr_ext.c:1379) but has no output case,
    // so it lands in the same `default:`.
    let o = same("%I", &[Val::F(1.0)]);
    assert_eq!(o.text.as_deref(), Some(""));
    assert_eq!(o.warnings.len(), 1);
}

#[test]
fn malformed_precision_warns_and_stops() {
    let _g = lock();
    // '.' with neither '*' nor a digit (pr_ext.c:1296).
    let o = same("a%.zb", &[]);
    assert_eq!(o.text.as_deref(), Some("a"));
    assert_eq!(o.warnings.len(), 1, "{:?}", o.warnings);

    // `%*N` without the '$' (pr_ext.c:1232).
    let o = same("a%*1d", &[Val::F(1.0)]);
    assert_eq!(o.text.as_deref(), Some("a"));
    assert_eq!(o.warnings.len(), 1, "{:?}", o.warnings);

    // Same rule on the precision side (pr_ext.c:1275).
    let o = same("a%.*1d", &[Val::F(1.0)]);
    assert_eq!(o.text.as_deref(), Some("a"));
    assert_eq!(o.warnings.len(), 1, "{:?}", o.warnings);
}

// ---------------------------------------------------------------------------
// Truncation at STRINGTEMP_LENGTH.

#[test]
fn output_truncates_at_the_temp_string_length() {
    let _g = lock();
    // Literal overflow: 1200 bytes into a 1024-byte buffer.
    let long = "x".repeat(1200);
    let o = same(&long, &[]);
    assert_eq!(o.text.as_deref().map(str::len), Some(1023));

    // A width wider than the whole buffer, exercising the port's clamp.
    let o = same("[%1500d]", &[Val::F(7.0)]);
    assert_eq!(o.text.as_deref().map(str::len), Some(1023));

    // Same for precision.
    let o = same("[%.1500d]", &[Val::F(7.0)]);
    assert_eq!(o.text.as_deref().map(str::len), Some(1023));

    // A string argument padded past the end.
    let o = same("%1500s", &[Val::S("tail")]);
    assert_eq!(o.text.as_deref().map(str::len), Some(1023));

    // Directives that begin after the buffer is already full are skipped
    // wholesale (the `if (o < end - 1)` gate), leaving the earlier text.
    let head = "y".repeat(1100);
    let o = same(&format!("{head}%d done"), &[Val::F(1.0)]);
    assert_eq!(o.text.as_deref().map(str::len), Some(1023));
}

// ---------------------------------------------------------------------------
// The ADR-005 gap, pinned as a divergence rather than hidden.

#[test]
fn unsupported_conversions_diverge() {
    let _g = lock();
    // `pr_cmds_glue.c`'s `PRBI_Raise` has no case for this, so it becomes a
    // `PR_RunError`; see `progs_builtins_sprintf.rs`'s ADR-005 audit.
    const UNSUPPORTED: c_int = 100;

    for fmt in ["%o", "%e", "%E", "%g", "%G", "%v", "%V"] {
        let c = run_sprintf(Side::C, fmt, &[Val::F(1.0)]);
        assert_eq!(c.status, 0, "C must still format {fmt}");
        assert!(c.text.is_some());

        let rs = run_sprintf(Side::Rust, fmt, &[Val::F(1.0)]);
        assert_eq!(
            rs.status, UNSUPPORTED,
            "{fmt} must raise, not abort or format"
        );
    }

    // Everything already written before the unsupported directive is still in
    // the temp buffer on both sides; only the raise differs.
    let rs = run_sprintf(Side::Rust, "ok %g", &[Val::F(1.0)]);
    assert_eq!(rs.status, UNSUPPORTED);
}
