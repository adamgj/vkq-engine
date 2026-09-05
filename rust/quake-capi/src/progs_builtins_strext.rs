//! `pr_ext.c` string-extension group (Phase 7 M9f group B): strconv,
//! infoadd/infoget, the qc tokenizer, strftime and stov
//! (`Quake/pr_ext.c:683-965`, `:1600-1815`, `:1821`).
//!
//! # What is ported
//!
//! * `chrconv_number` (`pr_ext.c:683`), `chrconv_punct` (`:709`),
//!   `chrchar_alpha` (`:727`) and `PF_strconv` (`:766`). `chrchar_alpha` is
//!   not a builtin and has no call site outside `PF_strconv`; it is ported
//!   with the other two tables because the four functions are one unit.
//! * `PF_infoadd` (`:854`) and `PF_infoget` (`:926`).
//! * The qc tokenizer: the `qctoken[MAXQCTOKENS]` / `qctoken_count` statics
//!   (`:1592-1598`), `tokenize_flush` (`:1600`), `PF_ArgC` (`:1610`),
//!   `tokenizeqc` (`:1615`), `PF_Tokenize` (`:1651`), `PF_tokenize_console`
//!   (`:1656`), `PF_tokenizebyseparator` (`:1661`), `PF_argv_start_index`
//!   (`:1724`), `PF_argv_end_index` (`:1738`) and `PF_ArgV` (`:1752`).
//! * `PF_strftime` (`:1790`) and `PF_stov` (`:1821`).
//!
//! `PF_strpad` (`:813`), `PF_strtoupper` (`:1771`), `PF_strtolower` (`:1780`),
//! `PF_stof` (`:1817`) and `PF_stoi` (`:1831`) sit inside the same line range
//! but were already flipped in Phase 6 M9 (`progs_builtins.rs`); they are not
//! touched here.
//!
//! # The tokenizer statics move to Rust wholesale
//!
//! `qctoken` / `qctoken_count` are file statics with exactly six readers, and
//! all six are ported here -- but one of them, `tokenize_flush`, is *not* a
//! builtin: `PR_ShutdownExtensions` (`pr_ext.c:6177`) calls it directly, and
//! that function stays C in this milestone. So the state can only live on one
//! side, and the C call site has to be flipped with it. That is done exactly
//! the way M9d did it for `PR_UnzoneAll` (`pr_ext.c:332-343`): a
//! `rust_pr_tokenize_flush` frame in `pr_cmds_glue.c`, a `PR_RSH_` macro pair
//! in `pr_ext.c`'s declaration block, and `PR_RSH_tokenize_flush ()` at the
//! call site. Flip the tokenizer builtins and that call site together or not
//! at all: a half-flip would leave two disjoint token tables.
//!
//! # Why this module is `host`-gated, not `progs`-gated
//!
//! Same reason as `progs_builtins_zone.rs`: nothing here needs the host
//! stratum on its own merits, but the plumbing it reuses (`run_sv`,
//! `SvConsole`, the `RUST_PF` wrappers, `PRBI_MsgGlue_VarString`) is
//! `all(host, progs-host)`, and the C table rows are `PF_RSH` to match.
//!
//! # ADR-009 audit (no longjmp across a Rust frame)
//!
//! Three seams in this group can raise, and none is called bare:
//!
//! * `PR_GetString` (`pr_edict_arena.c:307`) -- reached from every `G_STRING`
//!   here. Ported in `quake_progs::arena` (`VmRaw::get_string`), so the check
//!   runs in Rust and the raise comes back as `PRBI_ERR_NO_STRING` for
//!   `PRBI_Raise` to re-issue, as in `progs_builtins_zone.rs`.
//! * `PF_VarString` (`pr_cmds.c:155`) -- its `G_STRING`/`LOC_GetString` pass
//!   can `Host_Error`. Called through `PRBI_MsgGlue_VarString`, the guarded
//!   glue `progs_builtins_sv_msg.rs` already uses, which runs the whole body
//!   inside `Host_Guard` and returns a status. Used by `PF_strconv` (`:771`,
//!   `PF_VarString (3)`) and `PF_infoadd` (`:858`, `PF_VarString (2)`).
//!   `PF_strpad`'s `PF_VarString (1)` (`:818`) belongs to the already-flipped
//!   Phase 6 builtin and is not this module's.
//!
//! Everything else is a leaf: `PR_GetTempString` is a ring index and a
//! subscript, `PR_SetEngineString`'s one `Host_Error` is inside an `#if 0`
//! (`pr_edict_arena.c:351-353`), `Mem_Alloc`/`Mem_Free` only `Sys_Error`
//! (ADR-013), `q_strdup` is `Mem_Alloc` plus `memcpy`, `COM_Parse`
//! (`common.c` `COM_ParseEx`) has no error path at all, `atof` is `strtod`,
//! and the three `Con_Warning`s take constant format strings and are queued on
//! `SvConsole` for `run_sv` to flush after the Rust frame returns.
//!
//! # ADR-005 audit (the Rust float formatter has no `%g`/`%e`)
//!
//! No float is formatted in this module. `PF_stov` *parses* three floats and
//! does it with C's `atof` (`strtod`), not a Rust parser. `PF_strftime`
//! formats a `struct tm` with C's `strftime`, whose conversion specifiers are
//! a disjoint alphabet from `printf`'s -- `%e` there is the space-padded day
//! of month and `%g` the ISO-8601 two-digit week-based year, both handled
//! entirely inside libc. The one place a `%g`/`%e` *printf* specifier could
//! enter is `PF_VarString`'s localisation pass, which runs in C behind
//! `PRBI_MsgGlue_VarString`. ADR-005's panic path is not reachable from here.
//!
//! # ADR-010 audit
//!
//! * Every `(int)G_FLOAT (...)` truncation goes through
//!   `quake_progs::exec::c_cast_i32`, which reproduces the target's
//!   out-of-range behaviour rather than Rust's saturating `as`.
//! * `PF_stov` stores `atof`'s `double` into a `float` global, exactly like
//!   C's implicit conversion; the parse itself is libc's.
//! * `PF_strftime` calls libc `time`/`gmtime`/`localtime`/`strftime` rather
//!   than reimplementing a calendar, so the locale- and platform-dependent
//!   output is by construction the same bytes the C original produced.
//! * `chrconv_number` / `chrconv_punct` / `chrchar_alpha` are pure `int`
//!   arithmetic over inputs bounded by `0..=255` and bases bounded by
//!   `-30..=176`, so no expression can overflow and none is reassociated.
//!
//! # Bounds / panic audit (`panic = "abort"` in every profile)
//!
//! `qctoken` is a fixed 64-entry array that C indexes with `qctoken_count`, so
//! every write was re-derived:
//!
//! * `tokenizeqc` guards its whole body with `while (qctoken_count <
//!   MAXQCTOKENS)`, so the index is `0..=63` at every access.
//! * `PF_tokenizebyseparator` writes `qctoken[qctoken_count].start` once
//!   *before* the loop, immediately after `tokenize_flush ()` has set the
//!   count to 0 -- index 0. Inside the loop the index is the count at the top
//!   of an iteration, and the iteration that increments it to `MAXQCTOKENS`
//!   `break`s before the next one, so the index is again `0..=63`. There is no
//!   out-of-bounds write in the C to reproduce.
//! * `PF_ArgV` / `PF_argv_start_index` / `PF_argv_end_index` all subscript
//!   behind `if ((unsigned int)idx >= qctoken_count)`.
//!
//! The temp-string buffers are the other fixed-size target. `PF_strconv`
//! clamps `len` to `STRINGTEMP_LENGTH - 1` before the loop and writes the NUL
//! at `len`; `PF_infoadd`'s two `>= e` tests keep `o` strictly below
//! `destbuf + STRINGTEMP_LENGTH - 1` so its `*o = 0` is in bounds;
//! `PF_infoget`'s `o < e` loop condition leaves `*o++ = 0` writing at worst
//! the last byte of the buffer; `PF_ArgV`'s `q_strlcpy` truncation is
//! reproduced as an explicit `min`.
//!
//! Elsewhere: `idx += qctoken_count` is `wrapping_add` (C converts the `int`
//! to `unsigned` and back, which wraps in practice); `qctoken_count--` in
//! `tokenize_flush` is guarded by `> 0`; no slice range, `unwrap` or fixed
//! array is indexed by a value derived from progs data without a prior
//! comparison. `VmRaw::new`'s `assert!(!vm.is_null())` is the one abort left,
//! and a null ambient qcvm would already have crashed the C original.

use core::ffi::{c_char, c_float, c_int, c_uint, c_void, CStr};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::progs_builtins_sv as g;
use quake_c_sys::progs_builtins_sv_msg as gmsg;
use quake_progs::arena::{StringError, VmRaw};
use quake_progs::exec::c_cast_i32;
use quake_types::progs::{QcVm, OFS_PARM0, OFS_PARM1, OFS_PARM2, OFS_RETURN};

use crate::progs_builtins_sv::{guarded, run_sv, SvConsole, SvRaise, SvResult};

/// `pr_cmds_glue.c:38` `PRBI_ERR_NO_STRING`.
const PRBI_ERR_NO_STRING: c_int = 2;

/// `progs.h:210`.
const STRINGTEMP_LENGTH: usize = 1024;

/// `pr_ext.c:1591`.
const MAXQCTOKENS: usize = 64;

/// `PF_VarString`'s `static char out[1024]` (`pr_cmds.c:157`), which is also
/// the size `PRBI_MsgGlue_VarString` copies back.
const VARSTRING_LENGTH: usize = 1024;

/* ---------------------------------------------------------------------------
 * M9f integration note: engine C symbols this module needs that `quake-c-sys`
 * does not declare. They are declared here rather than added to `quake-c-sys`
 * so the five parallel M9f group modules do not collide in one shared file;
 * fold them into `quake-c-sys` when the wave lands.
 *
 * `PR_GetTempString` (`pr_cmds.c:132`) is non-static in the engine
 * (`pr_cmds_glue.c` already calls it) and is defined by
 * `quake-ctest/stubs/pr_ext_ref.c:122` in the differential link.
 *
 * The four C library entry points are `PF_strftime`'s, and they are called
 * rather than reimplemented because `strftime`'s output is locale- and
 * platform-dependent (ADR-010) and because a failed `strftime` leaves the
 * destination buffer's contents unspecified -- the only way to reproduce that
 * is to hand libc the same `PR_GetTempString ()` buffer C hands it. The MSVC
 * CRT reaches `time`/`gmtime`/`localtime` through inline forwarders to the
 * 64-bit-`time_t` entry points, so those are named directly on that target,
 * exactly as the C compiler resolves them there.
 *
 * `time_t` is deliberately not modelled: its width varies by target and libc.
 * The value is never inspected here, only round-tripped through
 * `TimeStorage`, which is over-sized and over-aligned for every ABI in the
 * platform matrix, and `time`'s return value is dropped by declaring it
 * `-> ()` (the caller ignores the return register either way).
 */
extern "C" {
    /// C: `char *PR_GetTempString (void)` (`Quake/pr_cmds.c:132`).
    #[allow(non_snake_case)]
    fn PR_GetTempString() -> *mut c_char;

    /// C: `time_t time (time_t *)`; only the out-parameter is used.
    #[cfg_attr(target_env = "msvc", link_name = "_time64")]
    fn time(t: *mut c_void);

    /// C: `struct tm *gmtime (const time_t *)`.
    #[cfg_attr(target_env = "msvc", link_name = "_gmtime64")]
    fn gmtime(t: *const c_void) -> *mut c_void;

    /// C: `struct tm *localtime (const time_t *)`.
    #[cfg_attr(target_env = "msvc", link_name = "_localtime64")]
    fn localtime(t: *const c_void) -> *mut c_void;

    /// C: `size_t strftime (char *, size_t, const char *, const struct tm *)`.
    fn strftime(s: *mut c_char, max: usize, fmt: *const c_char, tm: *const c_void) -> usize;
}

/// Backing store for the one `time_t` `PF_strftime` needs, sized and aligned
/// for any `time_t` in the platform matrix (see the integration note above).
#[repr(C, align(16))]
struct TimeStorage([u8; 16]);

/* ---------------------------------------------------------------------------
 * Shared helpers.
 */

/// `PR_GetString`'s one live failure, as the status `PRBI_Raise` decodes.
fn no_string(e: StringError) -> SvRaise {
    match e {
        StringError::NonExistent(num) => SvRaise {
            status: PRBI_ERR_NO_STRING,
            detail: num,
        },
    }
}

/// `progs.h:174` `G_STRING (o)`.
fn g_string(raw: &VmRaw, ofs: usize) -> Result<*const c_char, SvRaise> {
    raw.get_string(raw.g_i32(ofs)).map_err(no_string)
}

/// `strlen`, on a pointer into the engine's string arena.
fn c_strlen(s: *const c_char) -> usize {
    // SAFETY: every caller passes a `PR_GetString` result or an engine-owned
    // C string, both NUL-terminated.
    unsafe { CStr::from_ptr(s) }.to_bytes().len()
}

/// `PF_VarString (first)`, run whole inside `Host_Guard`
/// (`PRBI_MsgGlue_VarString`, ADR-009 rule 3) and returned as its bytes.
fn var_string(first: c_int) -> Result<Vec<u8>, SvRaise> {
    let mut out = [0 as c_char; VARSTRING_LENGTH];
    // SAFETY: `out` is a live 1024-byte buffer, exactly the size the glue
    // copies back, and the C body runs in its own guarded frame.
    guarded(unsafe { gmsg::PRBI_MsgGlue_VarString(first, out.as_mut_ptr()) })?;
    // SAFETY: the glue always leaves a NUL in the buffer (`out[0] = 0` before
    // the call, and `q_strlcat` terminates within 1024 after it).
    Ok(unsafe { CStr::from_ptr(out.as_ptr()) }.to_bytes().to_vec())
}

/// C's `*p++`, on a byte cursor into a progs string.
///
/// # Safety
/// `*p` must be inside a NUL-terminated string, and stepping past the NUL is
/// the caller's responsibility (C's own loops never do).
#[inline]
unsafe fn peek(p: *const c_char) -> u8 {
    // SAFETY: caller contract.
    unsafe { p.read() as u8 }
}

/* ---------------------------------------------------------------------------
 * chrconv_number (pr_ext.c:683), chrconv_punct (:709), chrchar_alpha (:727).
 *
 * Transcribed switch-for-switch. The unlisted `conv` values fall to
 * `default`, which leaves `base` alone -- including chrconv_number's explicit
 * `case 5: case 6:` no-op arms, which exist only so that PF_strconv's
 * "alternate" modes are a no-op for digits while chrchar_alpha treats them as
 * a per-character colour flip.
 */

fn chrconv_number(i: c_int, base: c_int, conv: c_int) -> c_int {
    let i = i - base;
    let base = match conv {
        1 => b'0' as c_int,
        2 => b'0' as c_int + 128,
        3 => b'0' as c_int - 30,
        4 => b'0' as c_int + 128 - 30,
        _ => base,
    };
    i + base
}

fn chrconv_punct(i: c_int, base: c_int, conv: c_int) -> c_int {
    let i = i - base;
    let base = match conv {
        1 => 0,
        2 => 128,
        _ => base,
    };
    i + base
}

fn chrchar_alpha(
    i: c_int,
    basec: c_int,
    baset: c_int,
    convc: c_int,
    convt: c_int,
    charnum: c_int,
) -> c_int {
    // convert case and colour seperatly...
    let i = i - (baset + basec);
    let baset = match convt {
        1 => 0,
        2 => 128,
        // COMPAT: ADR-010 -- C's `==` yields int 0/1, and the multiply is the
        // whole expression: convt 5 reddens even-indexed characters, convt 6
        // odd-indexed ones.
        5 | 6 => 128 * c_int::from((charnum & 1) == (convt - 5)),
        _ => baset,
    };
    let basec = match convc {
        1 => b'a' as c_int,
        2 => b'A' as c_int,
        _ => basec,
    };
    i + basec + baset
}

/* ---------------------------------------------------------------------------
 * PF_strconv (pr_ext.c:766).
 */

/// One iteration of `PF_strconv`'s else-if ladder. The order of the tests is
/// load-bearing and is not reordered or merged: the four digit ranges overlap
/// nothing, but `'0' + 128 - 30 ..= '9' + 128 - 30` (146..=155) is tested
/// *after* `'0' + 128 ..= '9' + 128` (176..=185), and the final
/// `(c & 127) < 16 || !redalpha` catch-all runs before either `chrconv_punct`
/// arm.
fn strconv_byte(c: u8, i: c_int, ccase: c_int, redalpha: c_int, rednum: c_int) -> u8 {
    const ZERO: c_int = b'0' as c_int;
    const NINE: c_int = b'9' as c_int;
    const LOWER_A: c_int = b'a' as c_int;
    const LOWER_Z: c_int = b'z' as c_int;
    const UPPER_A: c_int = b'A' as c_int;
    const UPPER_Z: c_int = b'Z' as c_int;

    let v = c as c_int;
    let out = if (ZERO..=NINE).contains(&v) {
        chrconv_number(v, ZERO, rednum)
    } else if (ZERO + 128..=NINE + 128).contains(&v) {
        chrconv_number(v, ZERO + 128, rednum)
    } else if (ZERO + 128 - 30..=NINE + 128 - 30).contains(&v) {
        chrconv_number(v, ZERO + 128 - 30, rednum)
    } else if (ZERO - 30..=NINE - 30).contains(&v) {
        chrconv_number(v, ZERO - 30, rednum)
    } else if (LOWER_A..=LOWER_Z).contains(&v) {
        chrchar_alpha(v, LOWER_A, 0, ccase, redalpha, i)
    } else if (UPPER_A..=UPPER_Z).contains(&v) {
        chrchar_alpha(v, UPPER_A, 0, ccase, redalpha, i)
    } else if (LOWER_A + 128..=LOWER_Z + 128).contains(&v) {
        chrchar_alpha(v, LOWER_A, 128, ccase, redalpha, i)
    } else if (UPPER_A + 128..=UPPER_Z + 128).contains(&v) {
        chrchar_alpha(v, UPPER_A, 128, ccase, redalpha, i)
    } else if (v & 127) < 16 || redalpha == 0 {
        v
    } else if v < 128 {
        chrconv_punct(v, 0, redalpha)
    } else {
        chrconv_punct(v, 128, redalpha)
    };
    // C stores through an `unsigned char *`.
    out as u8
}

fn pf_strconv(vm: *mut QcVm) -> SvResult {
    // SAFETY: ADR-008 -- a builtin only runs inside PR_ExecuteProgram, so the
    // ambient qcvm and its lumps are live for the whole call.
    let mut raw = unsafe { VmRaw::new(vm) };

    let ccase = c_cast_i32(raw.g_f32(OFS_PARM0));
    let redalpha = c_cast_i32(raw.g_f32(OFS_PARM1));
    let rednum = c_cast_i32(raw.g_f32(OFS_PARM2));
    let string = var_string(3)?;

    // C: `int len = strlen (string);` then the clamp. `PR_GetTempString` is
    // called after `PF_VarString`, so the ring is stepped exactly once and
    // only after the guarded call could have raised.
    let mut len = string.len();
    if len >= STRINGTEMP_LENGTH {
        len = STRINGTEMP_LENGTH - 1;
    }
    // SAFETY: leaf -- a ring index and a subscript into a static array.
    let resbuf = unsafe { PR_GetTempString() };

    for (i, &sb) in string.iter().enumerate().take(len) {
        let b = strconv_byte(sb, i as c_int, ccase, redalpha, rednum);
        // SAFETY: `i < len <= STRINGTEMP_LENGTH - 1`, inside the buffer.
        unsafe { resbuf.add(i).write(b as c_char) };
    }
    // SAFETY: `len <= STRINGTEMP_LENGTH - 1`, so this is the last byte at
    // worst -- C's `*result = '\0'` after the same loop.
    unsafe { resbuf.add(len).write(0) };

    // SAFETY: `resbuf` is NUL-terminated and outside the progs string blob, so
    // PR_SetEngineString interns it; its Host_Error is `#if 0`'d out.
    let handle = unsafe { g::PR_SetEngineString(resbuf) };
    raw.set_g_i32(OFS_RETURN, handle);
    Ok(())
}

/// `pr_ext.c:766` `PF_strconv`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_strconv(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| pf_strconv(vm)) }
}

/* ---------------------------------------------------------------------------
 * PF_infoadd (pr_ext.c:854) and PF_infoget (:926).
 *
 * Both walk `info` with a raw cursor rather than copying it into a Vec first.
 * That is deliberate: `info` is a `PR_GetString` result and can therefore be a
 * *temp string*, i.e. a pointer into the same `pr_string_temp` ring
 * `PR_GetTempString ()` hands back here. C reads it byte by byte while writing
 * the destination, so a self-overlapping call is observable; snapshotting the
 * source would silently fix it.
 */

fn pf_infoadd(vm: *mut QcVm, con: &mut SvConsole) -> SvResult {
    // SAFETY: as pf_strconv.
    let mut raw = unsafe { VmRaw::new(vm) };

    let mut info = g_string(&raw, OFS_PARM0)?;
    let key = g_string(&raw, OFS_PARM1)?;
    let value = var_string(2)?;

    // COMPAT: C evaluates `PR_GetTempString ()` in the declaration list, i.e.
    // *before* the empty-key early return below, so the temp-string ring is
    // stepped even on the error path. Do not sink this into the success arm.
    // SAFETY: leaf.
    let destbuf = unsafe { PR_GetTempString() };
    let mut o = destbuf;
    // SAFETY: one past the last writable byte, exactly C's
    // `destbuf + STRINGTEMP_LENGTH - 1`; never dereferenced.
    let e = unsafe { destbuf.add(STRINGTEMP_LENGTH - 1) };

    let keylen = c_strlen(key);
    let valuelen = value.len();

    // SAFETY: `key` is NUL-terminated.
    if unsafe { peek(key) } == 0 {
        // error
        raw.set_g_i32(OFS_RETURN, raw.g_i32(OFS_PARM0));
        return Ok(());
    }

    // copy the string to the output, stripping the named key
    // SAFETY: the whole walk stays inside `info`'s NUL-terminated extent and
    // inside `destbuf`'s STRINGTEMP_LENGTH bytes; every cursor step is
    // guarded by the same test C uses.
    unsafe {
        while peek(info) != 0 {
            let l = info;
            let c0 = peek(info);
            info = info.add(1);
            if c0 != b'\\' {
                break; // error / end-of-string
            }

            if c_strncmp(info, key, keylen) == 0 && peek(info.add(keylen)) == b'\\' {
                // skip the key name
                info = info.add(keylen + 1);
                // this is the old value for the key. skip over it
                while peek(info) != 0 && peek(info) != b'\\' {
                    info = info.add(1);
                }
            } else {
                // skip the key
                while peek(info) != 0 && peek(info) != b'\\' {
                    info = info.add(1);
                }

                // validate that its a value now
                let c1 = peek(info);
                info = info.add(1);
                if c1 != b'\\' {
                    break; // error
                }
                // skip the value
                while peek(info) != 0 && peek(info) != b'\\' {
                    info = info.add(1);
                }

                // copy them over
                let span = info as usize - l as usize;
                if o as usize + span >= e as usize {
                    break; // exceeds maximum length
                }
                let mut src = l;
                while (src as usize) < info as usize {
                    o.write(src.read());
                    o = o.add(1);
                    src = src.add(1);
                }
            }
        }

        // COMPAT: the `!*key` arm below is dead -- the empty-key case returned
        // above. Kept in place so the ladder reads as C's does.
        if peek(info) != 0 {
            con.warn(b"PF_infoadd: invalid source info\n");
        } else if value.is_empty() {
            // nothing needed
        } else if peek(key) == 0 || c_strchr(key, b'\\') || value.contains(&b'\\') {
            con.warn(b"PF_infoadd: invalid key/value\n");
        } else if o as usize + 2 + keylen + valuelen >= e as usize {
            con.warn(b"PF_infoadd: length exceeds max\n");
        } else {
            o.write(b'\\' as c_char);
            o = o.add(1);
            ptr::copy_nonoverlapping(key.cast::<u8>(), o.cast::<u8>(), keylen);
            o = o.add(keylen);
            o.write(b'\\' as c_char);
            o = o.add(1);
            ptr::copy_nonoverlapping(value.as_ptr(), o.cast::<u8>(), valuelen);
            o = o.add(valuelen);
        }

        o.write(0);
        // SAFETY: NUL-terminated temp string, as pf_strconv.
        let handle = g::PR_SetEngineString(destbuf);
        raw.set_g_i32(OFS_RETURN, handle);
    }
    Ok(())
}

/// `pr_ext.c:854` `PF_infoadd`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_infoadd(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, pf_infoadd) }
}

fn pf_infoget(vm: *mut QcVm) -> SvResult {
    // SAFETY: as pf_strconv.
    let mut raw = unsafe { VmRaw::new(vm) };

    let mut info = g_string(&raw, OFS_PARM0)?;
    let key = g_string(&raw, OFS_PARM1)?;
    let keylen = c_strlen(key);

    // SAFETY: as pf_infoadd's walk.
    unsafe {
        while peek(info) != 0 {
            let c0 = peek(info);
            info = info.add(1);
            if c0 != b'\\' {
                break; // error / end-of-string
            }

            if c_strncmp(info, key, keylen) == 0 && peek(info.add(keylen)) == b'\\' {
                // COMPAT: the temp string is taken *here*, not at entry, so a
                // lookup that misses does not step the ring at all.
                let destbuf = PR_GetTempString();
                let mut o = destbuf;
                let e = destbuf.add(STRINGTEMP_LENGTH - 1);

                // skip the key name
                info = info.add(keylen + 1);
                // this is the old value for the key. copy it to the result
                while peek(info) != 0 && peek(info) != b'\\' && (o as usize) < e as usize {
                    o.write(info.read());
                    o = o.add(1);
                    info = info.add(1);
                }
                // `o <= e`, so this writes the last byte at worst.
                o.write(0);

                // success!
                let handle = g::PR_SetEngineString(destbuf);
                raw.set_g_i32(OFS_RETURN, handle);
                return Ok(());
            }

            // skip the key
            while peek(info) != 0 && peek(info) != b'\\' {
                info = info.add(1);
            }

            // validate that its a value now
            let c1 = peek(info);
            info = info.add(1);
            if c1 != b'\\' {
                break; // error
            }
            // skip the value
            while peek(info) != 0 && peek(info) != b'\\' {
                info = info.add(1);
            }
        }
    }
    raw.set_g_i32(OFS_RETURN, 0);
    Ok(())
}

/// `pr_ext.c:926` `PF_infoget`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_infoget(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| pf_infoget(vm)) }
}

/// C's `strncmp (a, b, n)`, reduced to the equality test both info builtins
/// actually use.
///
/// # Safety
/// `a` must have at least `n` readable bytes or a NUL within them; `b` must be
/// NUL-terminated with at least `n` bytes before its NUL when the caller
/// passes `strlen (b)` as `n`, which is the only way it is called here.
unsafe fn c_strncmp(a: *const c_char, b: *const c_char, n: usize) -> c_int {
    for i in 0..n {
        // SAFETY: caller contract.
        let (x, y) = unsafe { (peek(a.add(i)), peek(b.add(i))) };
        if x != y {
            return if x < y { -1 } else { 1 };
        }
        if x == 0 {
            return 0;
        }
    }
    0
}

/// C's `strchr (s, ch) != NULL`, for the one `'\\'` test in `PF_infoadd`.
///
/// # Safety
/// `s` must be NUL-terminated.
unsafe fn c_strchr(s: *const c_char, ch: u8) -> bool {
    let mut p = s;
    loop {
        // SAFETY: caller contract.
        let b = unsafe { peek(p) };
        if b == ch {
            return true;
        }
        if b == 0 {
            return false;
        }
        // SAFETY: not past the NUL.
        p = unsafe { p.add(1) };
    }
}

/* ---------------------------------------------------------------------------
 * The qc tokenizer (pr_ext.c:1591-1769).
 *
 * `qctoken` / `qctoken_count` move here wholesale; see the module header for
 * why the non-builtin `tokenize_flush` has to move with them.
 */

/// `pr_ext.c:1593-1597`.
#[derive(Clone, Copy)]
struct QcToken {
    token: *mut c_char,
    start: c_uint,
    end: c_uint,
}

static mut QCTOKEN: [QcToken; MAXQCTOKENS] = [QcToken {
    token: ptr::null_mut(),
    start: 0,
    end: 0,
}; MAXQCTOKENS];
static mut QCTOKEN_COUNT: c_uint = 0;

/// `&qctoken[i]`. Single-threaded engine discipline (the qcvm is not
/// re-entered from another thread), the same rule `cl_parse.rs`'s
/// `ptr::addr_of_mut!` accessors rely on.
///
/// # Safety
/// `i` must be `< MAXQCTOKENS`. Every caller is covered by the module
/// header's bounds audit.
#[inline]
unsafe fn tok(i: usize) -> *mut QcToken {
    // SAFETY: caller contract; the array is a live static for the process
    // lifetime.
    unsafe { ptr::addr_of_mut!(QCTOKEN).cast::<QcToken>().add(i) }
}

#[inline]
fn tok_count() -> c_uint {
    // SAFETY: a plain read of a `Copy` static; no reference is formed.
    unsafe { QCTOKEN_COUNT }
}

#[inline]
fn set_tok_count(n: c_uint) {
    // SAFETY: as `tok_count`.
    unsafe { QCTOKEN_COUNT = n }
}

/// `pr_ext.c:1600` `tokenize_flush`.
fn tokenize_flush() {
    let mut n = tok_count();
    while n > 0 {
        n -= 1;
        set_tok_count(n);
        // SAFETY: `n < MAXQCTOKENS`; every live slot's `token` came from
        // `Mem_Alloc`/`q_strdup`, and `Mem_Free` tolerates null.
        unsafe { c::Mem_Free((*tok(n as usize)).token.cast()) };
    }
    set_tok_count(0);
}

/// `pr_ext.c:1600` `tokenize_flush` -- not a builtin. `PR_ShutdownExtensions`
/// (`pr_ext.c:6177`) calls it directly and stays C, so it gets a hand-written
/// `rust_pr_tokenize_flush` frame in `pr_cmds_glue.c`, exactly as M9d did for
/// `PR_UnzoneAll`.
///
/// # Safety
/// `detail` must point at a writable `int`, as `pr_cmds_glue.c`'s
/// `rust_pr_tokenize_flush` frame passes. Nothing here can raise, so the
/// status is always `PRBI_OK` and `detail` is never written.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pr_tokenize_flush(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract. The body touches only this module's statics and
    // `Mem_Free`, so it needs neither the ambient qcvm nor the console queue,
    // but it is routed through `run_sv` anyway so the status/detail convention
    // stays the one `PRBI_Raise` expects.
    unsafe {
        run_sv(detail, |_vm, _con| {
            tokenize_flush();
            Ok(())
        })
    }
}

/// `pr_ext.c:1615` `tokenizeqc`.
///
/// The `dpfuckage` parameter is C's and is unused there too: the punctuation
/// branch it selects is commented out (`pr_ext.c:1636-1639`), so
/// `PF_Tokenize` and `PF_tokenize_console` currently do the same thing.
///
/// # Safety
/// `s` must be a NUL-terminated C string that stays live for the call.
unsafe fn tokenizeqc(s: *const c_char, _dpfuckage: bool) -> c_int {
    let start = s;
    let mut cur = s;

    // C repeats tokenize_flush's loop inline here rather than calling it; the
    // two are statement-for-statement identical.
    tokenize_flush();

    // SAFETY: the walk stays inside `s`'s NUL-terminated extent, and every
    // `tok ()` index is `< MAXQCTOKENS` by the loop condition.
    unsafe {
        while tok_count() < MAXQCTOKENS as c_uint {
            /*skip whitespace here so the token's start is accurate*/
            // COMPAT: the cast is to `const unsigned char *`, so bytes >= 128
            // are NOT whitespace here -- but COM_Parse's own skip reads
            // through a `const char *` (signed on every target in the matrix)
            // and does treat them as whitespace. A high byte therefore sets
            // `.start` to its own offset and is then skipped by COM_Parse, so
            // the recorded start can precede the token's first byte. Preserved.
            while peek(cur) != 0 && peek(cur) <= b' ' {
                cur = cur.add(1);
            }

            if peek(cur) == 0 {
                break;
            }

            let i = tok_count() as usize;
            (*tok(i)).start = (cur as usize - start as usize) as c_uint;

            cur = c::COM_Parse(cur);
            if cur.is_null() {
                break;
            }

            (*tok(i)).token = c::cvar_cmd::q_strdup(c::COM_ThreadToken());
            (*tok(i)).end = (cur as usize - start as usize) as c_uint;
            set_tok_count(tok_count() + 1);
        }
    }
    tok_count() as c_int
}

fn pf_argc(vm: *mut QcVm) -> SvResult {
    // SAFETY: as pf_strconv.
    let mut raw = unsafe { VmRaw::new(vm) };
    raw.set_g_f32(OFS_RETURN, tok_count() as c_float);
    Ok(())
}

/// `pr_ext.c:1610` `PF_ArgC`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
#[allow(non_snake_case)]
pub unsafe extern "C" fn quake_rs_pf_ArgC(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| pf_argc(vm)) }
}

fn pf_tokenize_common(vm: *mut QcVm, dpfuckage: bool) -> SvResult {
    // SAFETY: as pf_strconv.
    let mut raw = unsafe { VmRaw::new(vm) };
    let s = g_string(&raw, OFS_PARM0)?;
    // SAFETY: `s` is a live NUL-terminated progs string for the call.
    let n = unsafe { tokenizeqc(s, dpfuckage) };
    raw.set_g_f32(OFS_RETURN, n as c_float);
    Ok(())
}

/// `pr_ext.c:1651` `PF_Tokenize`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
#[allow(non_snake_case)]
pub unsafe extern "C" fn quake_rs_pf_Tokenize(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| pf_tokenize_common(vm, true)) }
}

/// `pr_ext.c:1656` `PF_tokenize_console`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_tokenize_console(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| pf_tokenize_common(vm, false)) }
}

fn pf_tokenizebyseparator(vm: *mut QcVm) -> SvResult {
    // SAFETY: as pf_strconv.
    let mut raw = unsafe { VmRaw::new(vm) };

    let start = g_string(&raw, OFS_PARM0)?;
    let mut sep: [*const c_char; 7] = [ptr::null(); 7];
    let mut seplen: [usize; 7] = [0; 7];
    let mut seps: usize = 0;

    while (seps as c_int) < raw.argc() - 1 && seps < 7 {
        sep[seps] = g_string(&raw, OFS_PARM1 + seps * 3)?;
        seplen[seps] = c_strlen(sep[seps]);
        seps += 1;
    }

    tokenize_flush();

    let mut cur = start;
    // SAFETY: `tokenize_flush` has just set the count to 0, so this is
    // `qctoken[0]`; the rest of the walk stays inside `start`'s NUL-terminated
    // extent and every index is `< MAXQCTOKENS` (module bounds audit).
    unsafe {
        (*tok(0)).start = 0;

        if peek(cur) != 0 {
            loop {
                let mut found = false;
                let i = tok_count() as usize;

                /*see if its a separator*/
                if peek(cur) == 0 {
                    (*tok(i)).end = (cur as usize - start as usize) as c_uint;
                    found = true;
                } else {
                    for s in 0..seps {
                        if c_strncmp(cur, sep[s], seplen[s]) == 0 {
                            (*tok(i)).end = (cur as usize - start as usize) as c_uint;
                            cur = cur.add(seplen[s]);
                            found = true;
                            break;
                        }
                    }
                }

                /*it was, split it out*/
                if found {
                    let tlen = ((*tok(i)).end.wrapping_sub((*tok(i)).start)) as c_int;
                    let buf = c::Mem_Alloc(tlen as usize + 1).cast::<c_char>();
                    ptr::copy_nonoverlapping(
                        start.add((*tok(i)).start as usize).cast::<u8>(),
                        buf.cast::<u8>(),
                        tlen as usize,
                    );
                    buf.add(tlen as usize).write(0);
                    (*tok(i)).token = buf;

                    let n = tok_count() + 1;
                    set_tok_count(n);

                    if peek(cur) != 0 && n < MAXQCTOKENS as c_uint {
                        (*tok(n as usize)).start = (cur as usize - start as usize) as c_uint;
                    } else {
                        break;
                    }
                }
                // COMPAT (bug preserved): the cursor advances one byte after a
                // separator match too, on top of the `str += seplen[s]` above.
                // Two adjacent separators therefore do not yield an empty
                // token -- the byte after the first separator is swallowed
                // into the *next* token's start offset, so "a,,b" tokenizes as
                // "a" and ",b", not "a", "" and "b".
                cur = cur.add(1);
            }
        }
    }

    raw.set_g_f32(OFS_RETURN, tok_count() as c_float);
    Ok(())
}

/// `pr_ext.c:1661` `PF_tokenizebyseparator`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_tokenizebyseparator(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| pf_tokenizebyseparator(vm)) }
}

/// The index normalisation `PF_argv_start_index` / `PF_argv_end_index` /
/// `PF_ArgV` share: negative indexes are relative to the end, then the result
/// is range-checked as `unsigned`.
///
/// COMPAT: ADR-010 -- C's `idx += qctoken_count` converts the `int` to
/// `unsigned int` and back, which wraps; `wrapping_add` is that conversion.
fn argv_index(raw: &VmRaw) -> Option<usize> {
    let mut idx = c_cast_i32(raw.g_f32(OFS_PARM0));
    if idx < 0 {
        idx = idx.wrapping_add(tok_count() as c_int);
    }
    if (idx as c_uint) >= tok_count() {
        None
    } else {
        Some(idx as usize)
    }
}

fn pf_argv_index(vm: *mut QcVm, end: bool) -> SvResult {
    // SAFETY: as pf_strconv.
    let mut raw = unsafe { VmRaw::new(vm) };
    let v = match argv_index(&raw) {
        None => -1.0,
        // SAFETY: `argv_index` returned `Some`, so the index is
        // `< qctoken_count <= MAXQCTOKENS`.
        Some(i) => unsafe {
            if end {
                (*tok(i)).end as c_float
            } else {
                (*tok(i)).start as c_float
            }
        },
    };
    raw.set_g_f32(OFS_RETURN, v);
    Ok(())
}

/// `pr_ext.c:1724` `PF_argv_start_index`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_argv_start_index(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| pf_argv_index(vm, false)) }
}

/// `pr_ext.c:1738` `PF_argv_end_index`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_argv_end_index(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| pf_argv_index(vm, true)) }
}

fn pf_argv(vm: *mut QcVm) -> SvResult {
    // SAFETY: as pf_strconv.
    let mut raw = unsafe { VmRaw::new(vm) };
    match argv_index(&raw) {
        None => raw.set_g_i32(OFS_RETURN, 0),
        Some(i) => {
            // SAFETY: `i < qctoken_count`, so `.token` was set by
            // `tokenizeqc` / `PF_tokenizebyseparator` before the count that
            // covers it, and is NUL-terminated.
            let handle = unsafe {
                let src = (*tok(i)).token;
                let len = c_strlen(src);
                let ret = PR_GetTempString();
                // C: `q_strlcpy (ret, ..., STRINGTEMP_LENGTH)`.
                let n = len.min(STRINGTEMP_LENGTH - 1);
                ptr::copy_nonoverlapping(src.cast::<u8>(), ret.cast::<u8>(), n);
                ret.add(n).write(0);
                g::PR_SetEngineString(ret)
            };
            raw.set_g_i32(OFS_RETURN, handle);
        }
    }
    Ok(())
}

/// `pr_ext.c:1752` `PF_ArgV`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
#[allow(non_snake_case)]
pub unsafe extern "C" fn quake_rs_pf_ArgV(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| pf_argv(vm)) }
}

/* ---------------------------------------------------------------------------
 * PF_strftime (pr_ext.c:1790).
 */

fn pf_strftime(vm: *mut QcVm) -> SvResult {
    // SAFETY: as pf_strconv.
    let mut raw = unsafe { VmRaw::new(vm) };

    let input = g_string(&raw, OFS_PARM1)?;
    // SAFETY: leaf.
    let result = unsafe { PR_GetTempString() };

    let mut curtime = TimeStorage([0; 16]);
    // SAFETY: `time` writes one `time_t` through the pointer; the storage is
    // over-sized and over-aligned for every target's `time_t`.
    unsafe { time(curtime.0.as_mut_ptr().cast()) };

    // C's `if (G_FLOAT (OFS_PARM0))` -- any non-zero float, NaN included.
    let use_local = raw.g_f32(OFS_PARM0) != 0.0;
    // SAFETY: `curtime` holds a value `time` just wrote; both entry points
    // return a pointer to libc's own static `struct tm` (or null), which is
    // only handed straight back to `strftime`.
    let tm = unsafe {
        if use_local {
            localtime(curtime.0.as_ptr().cast())
        } else {
            gmtime(curtime.0.as_ptr().cast())
        }
    };

    // SAFETY: `input` is a NUL-terminated progs string and the replacements
    // are static NUL-terminated literals.
    let input = unsafe { strftime_fmt_fixup(input) };

    // SAFETY: `result` is a STRINGTEMP_LENGTH-byte temp buffer, `input` is
    // NUL-terminated, and `tm` is whatever gmtime/localtime returned -- C
    // passes it on unchecked too. On failure strftime leaves the buffer's
    // contents unspecified; handing it the same buffer C hands it is the only
    // way to reproduce that, which is why this is not a Rust reimplementation.
    unsafe { strftime(result, STRINGTEMP_LENGTH, input, tm) };

    // SAFETY: as pf_strconv.
    let handle = unsafe { g::PR_SetEngineString(result) };
    raw.set_g_i32(OFS_RETURN, handle);
    Ok(())
}

/// `pr_ext.c:1804-1809`'s `#ifdef _WIN32` workaround ("msvc sucks. this is a
/// crappy workaround."), which rewrites the two format strings the Microsoft
/// CRT does not implement.
///
/// COMPAT: C guards this with `#ifdef _WIN32`, which the toolchain defines for
/// MinGW as well as MSVC; `cfg(windows)` is that same set. The rewrite has to
/// stay platform-conditional -- glibc *does* implement `%R` and `%F`, so
/// expanding them everywhere would change what `strftime` is handed on Linux
/// and macOS.
///
/// # Safety
/// `in_` must be NUL-terminated.
#[cfg(windows)]
unsafe fn strftime_fmt_fixup(in_: *const c_char) -> *const c_char {
    // SAFETY: caller contract; the literals are NUL-terminated.
    unsafe {
        if c_streq(in_, c"%R".as_ptr()) {
            c"%H:%M".as_ptr()
        } else if c_streq(in_, c"%F".as_ptr()) {
            c"%Y-%m-%d".as_ptr()
        } else {
            in_
        }
    }
}

/// The non-Windows arm of `pr_ext.c:1804-1809`: the `#ifdef _WIN32` block is
/// not compiled there, so the format string passes through untouched.
///
/// # Safety
/// Nothing is dereferenced; `unsafe` only so both arms share one call site.
#[cfg(not(windows))]
unsafe fn strftime_fmt_fixup(in_: *const c_char) -> *const c_char {
    in_
}

/// C's `!strcmp (a, b)`.
///
/// # Safety
/// Both operands must be NUL-terminated.
#[cfg(windows)]
unsafe fn c_streq(a: *const c_char, b: *const c_char) -> bool {
    let (mut a, mut b) = (a, b);
    loop {
        // SAFETY: caller contract; neither cursor steps past its own NUL.
        unsafe {
            let (x, y) = (peek(a), peek(b));
            if x != y {
                return false;
            }
            if x == 0 {
                return true;
            }
            a = a.add(1);
            b = b.add(1);
        }
    }
}

/// `pr_ext.c:1790` `PF_strftime`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_strftime(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| pf_strftime(vm)) }
}

/* ---------------------------------------------------------------------------
 * PF_stov (pr_ext.c:1821).
 */

fn pf_stov(vm: *mut QcVm) -> SvResult {
    // SAFETY: as pf_strconv.
    let mut raw = unsafe { VmRaw::new(vm) };
    let mut s = g_string(&raw, OFS_PARM0)?;

    for i in 0..3 {
        // SAFETY: COM_Parse tolerates null (`COM_ParseEx` returns NULL
        // immediately, leaving com_token empty) and has no error path;
        // COM_ThreadToken returns this thread's NUL-terminated buffer, valid
        // until the next parse.
        let v = unsafe {
            s = c::COM_Parse(s);
            c::cvar_cmd::atof(c::COM_ThreadToken())
        };
        // COMPAT: ADR-010 -- C assigns atof's `double` into a `float` global,
        // so the narrowing happens here, not inside the parse.
        raw.set_g_f32(OFS_RETURN + i, v as c_float);
    }
    Ok(())
}

/// `pr_ext.c:1821` `PF_stov`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_stov(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| pf_stov(vm)) }
}
