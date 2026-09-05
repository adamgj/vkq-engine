//! `pr_ext.c` sprintf group (Phase 7 M9f group A): `PF_sprintf_internal`
//! and `PF_sprintf` (`Quake/pr_ext.c:1110-1589`).
//!
//! One `builtin_t` slot (`sprintf`) plus the private formatter behind it.
//! `PF_sprintf_internal` is a full printf-style engine driven by a *progs*
//! format string, so the whole directive grammar -- positional `%N$`, `*`
//! width/precision, flags, length modifiers -- is attacker-controlled QC data
//! and every path here is reachable from a mod.
//!
//! # ADR-005 audit (float formatter) -- BLOCKING, the slot is NOT flipped
//!
//! `quake_util::printf` is the sanctioned C-printf-compatible formatter
//! (ADR-005). It implements `d i u x X f F s c` and *deliberately* does not
//! implement `%e`/`%g`; `printf.rs:211` is
//! `other => panic!("printf: conversion '%{}' unsupported", ...)`. With
//! `panic = "abort"` in every profile that is a process kill, not a raise.
//!
//! Auditing every conversion `PF_sprintf_internal` can reach:
//!
//! | directive | `pr_ext.c` | status |
//! |---|---|---|
//! | `d` `i` | :1412 | ported (`%lld`) |
//! | `u` `x` `X` `p` `P` | :1421 | ported (`p`->`x`, `P`->`X`) |
//! | `f` `F` | :1434 | ported |
//! | `c` | :1461 | ported |
//! | `s` | :1546 | ported |
//! | `S` | :1485 | ported (escape+quote, then `%s`) |
//! | `o` | :1420 | **unsupported** -- no octal conversion in `printf.rs` |
//! | `e` `E` `g` `G` | :1432-1437 | **unsupported** -- ADR-005 |
//! | `v` `V` | :1444-1446 | **unsupported** -- `f[-2] += 'g' - 'v'` rewrites |
//! | | | the conversion char to `g`/`G`, a second, |
//! | | | independent `%g` path |
//! | `I` | :1379 | unreachable in the *output* switch: `I` takes the |
//! | | | length-prefix arm but has no case at :1410, so it |
//! | | | falls to `default:` (:1564) and warns, as C does |
//!
//! `{o, e, E, g, G, v, V}` therefore have **no faithful implementation** in
//! this milestone. They are not papered over and they are not passed to
//! `quake_util::printf` (which would abort the process): the port returns
//! `PRBI_ERR_SPRINTF_UNSUPPORTED_CONV`, which `PRBI_Raise`'s `default:` arm
//! (`pr_cmds_glue.c`) turns into a `PR_RunError` -- a *defined* QC-visible
//! raise instead of an abort. That is still a behaviour change versus C, so
//! **the `sprintf` row in `pr_ext.c`'s builtin table must stay `PF_sprintf`
//! until ADR-005 grows `%e`/`%g` and an octal conversion** (and the ADR's
//! conformance suite is extended first, as the ADR requires). This module only
//! declares its entry point; it does not flip the row.
//!
//! # ADR-009 audit
//!
//! Three seams can raise; all are reported as statuses, none is called from a
//! Rust frame that a `longjmp` could cross:
//!
//! * `PR_GetString` for the format string (`G_STRING (OFS_PARM0)`, :1586) and
//!   for `%s`/`%S` arguments (:1487, :1551) -- `VmRaw::get_string` is the port
//!   of `pr_edict_arena.c`'s lookup and returns `StringError` instead of
//!   `Host_Error`ing, re-issued as `PRBI_ERR_NO_STRING`.
//! * `PR_SetEngineString` (:1587) is the C function; its own `Host_Error` is
//!   in an arm unreachable for a non-blob pointer, exactly as relied on by
//!   `progs_builtins_zone.rs`.
//! * `PR_GetTempString` (:1585) cannot raise -- it is an index into a static
//!   ring.
//!
//! `Con_Warning` is *not* called from the Rust frame: warnings are queued on
//! `SvConsole` and flushed by `run_sv` after the frame returns.
//!
//! # Console-ordering deviation
//!
//! C emits each `Con_Warning` at the point it happens, i.e. before
//! `PR_SetEngineString` interns the result. Here the warnings are deferred to
//! `run_sv`'s flush, which happens *after* `PR_SetEngineString`, so if that
//! call prints (`PR_AllocStringSlots`' `Con_DPrintf2`) the two console lines
//! come out in the opposite order. The warning *text* and count are identical.
//! This is inherent to the `SvConsole` convention shared by this module family,
//! not specific to sprintf; the differential test compares warning lines only.
//!
//! # Bounds / panic audit
//!
//! `panic = "abort"`, so every implicit panic is a process kill. What was
//! checked:
//!
//! * **Output buffer.** `OutBuf` is the only writer. It mirrors C's
//!   `if (o < end - 1)` gate and `o += strlen (o)` advance and can never index
//!   past `outbuflen`; see its own comments for the invariant.
//! * **Format-string reads.** `at()` returns 0 for any index at or past the
//!   end, so no read can panic. This is also where C's one genuine
//!   out-of-bounds read is *not* reproduced -- see the `%`-at-end note below.
//! * **Argument indices.** Every `GETARG_*` is gated on
//!   `a >= firstarg && a < argc` exactly as C is, so the global offset
//!   `OFS_PARM0 + 3 * a` is only ever computed for a small non-negative `a`.
//! * **Integer overflow.** `thisarg = width + (firstarg - 1)` (:1181),
//!   `argpos++`, `thisarg + 1` (:1523) and `width = -width` (:1243, :1258) all
//!   use wrapping arithmetic; C's `int` overflow there is UB but in practice
//!   wraps, and the results only ever feed the range-gated `GETARG_*`.
//! * **`strtol` overflow.** `strtol_dec` saturates to `c_long::MAX` and then
//!   narrows with `as`, reproducing glibc/MSVCRT saturation plus C's
//!   implementation-defined `long`->`int` narrowing. Because `c_long` is 32-bit
//!   on Windows and 64-bit elsewhere, `%99999999999d` yields `INT_MAX` on
//!   Windows and `-1` on Unix -- a real, pre-existing platform split that the
//!   port preserves rather than normalises.
//! * **Allocation size.** `quake_util::printf` materialises the padded field
//!   before truncation, so an unclamped `%*d` with a `1e9` argument would ask
//!   for a gigabyte, where C's `q_snprintf` only ever writes `end - o` bytes.
//!   Width and precision are therefore clamped to `FIELD_CLAMP` (64 KiB), well
//!   past the 1 KiB output buffer. Every width/precision at or below the clamp
//!   is bit-exact. Above it the result is still bit-exact whenever the
//!   *unpadded* conversion is shorter than `FIELD_CLAMP - (end - o)`, because
//!   the surviving bytes are then all padding either way; the only divergent
//!   corner is a >64 KiB width combined with a >63 KiB body, which no format
//!   string that reaches a 1 KiB buffer can distinguish in practice. Recorded
//!   as a known gap rather than a silent equivalence claim.
//!
//! # The one deliberate non-reproduction: C's `%`-at-end overrun
//!
//! When the output buffer is already full (`o >= end - 1`), C skips the whole
//! `if (o < end - 1)` block at :1355, so the `default: goto finished` at :1564
//! never runs; `++s` at :1569 then steps *past* the format string's NUL and the
//! next iteration reads past the end of the progs string. That is a genuine
//! read past the end of a real object (reachable from QC as e.g.
//! `sprintf("<1023 chars>%")`), which is the one class of C defect this port
//! does not reproduce: `at()` reads 0 there and the loop finishes. Reported as
//! a finding rather than transcribed.

use core::ffi::{c_char, c_int, c_long};

use quake_c_sys::progs_builtins_sv as g;
use quake_progs::arena::{StringError, VmRaw};
use quake_types::progs::{QcVm, OFS_PARM0, OFS_RETURN};
use quake_util::printf::{format as c_printf, Arg};

use crate::progs_builtins_sv::{run_sv, SvConsole, SvRaise, SvResult};

// M9f integration note: `PR_GetTempString` (`Quake/pr_cmds.c:133`, declared
// `Quake/progs.h:206`) has no `quake-c-sys` binding -- nothing ported before
// this milestone touched the temp-string ring. It is declared here rather than
// added to `quake-c-sys`, whose bindgen inputs this milestone must not change.
// The ctest oracle TU (`stubs/pr_ext_ref.c`) defines it non-static, so the
// link resolves on both sides.
extern "C" {
    /// C: `char *PR_GetTempString (void)`
    fn PR_GetTempString() -> *mut c_char;
}

/// `progs.h:210`
const STRINGTEMP_LENGTH: usize = 1024;

/// `pr_ext.c:1124` `static char quotedbuf[65536]`
const QUOTEDBUF_SIZE: usize = 65536;

/// `pr_cmds_glue.c:38` `PRBI_ERR_NO_STRING`
const PRBI_ERR_NO_STRING: c_int = 2;

/// Upper bound on the width/precision handed to `quake_util::printf`, which
/// allocates the whole padded field where C's `q_snprintf` truncates. 64 KiB is
/// 64x the output buffer, so no reachable format string can tell the clamp
/// apart from C (see the allocation note in the module docs).
const FIELD_CLAMP: usize = 1 << 16;

/// Not a `PRBI_*` code: there is no allocated status for "this port cannot
/// format that conversion", so it lands in `PRBI_Raise`'s `default:` arm and
/// becomes `PR_RunError ("PF_sprintf: unknown status 100")`. See the ADR-005
/// audit above -- the builtin row stays C until this can be deleted.
const PRBI_ERR_SPRINTF_UNSUPPORTED_CONV: c_int = 100;

/// `pr_ext.c:1126-1130`
const PRINTF_ALTERNATE: c_int = 1;
const PRINTF_ZEROPAD: c_int = 2;
const PRINTF_LEFT: c_int = 4;
const PRINTF_SPACEPOSITIVE: c_int = 8;
const PRINTF_SIGNPOSITIVE: c_int = 16;

fn no_string(e: StringError) -> SvRaise {
    match e {
        StringError::NonExistent(num) => SvRaise {
            status: PRBI_ERR_NO_STRING,
            detail: num,
        },
    }
}

fn unsupported_conv(conv: u8) -> SvRaise {
    SvRaise {
        status: PRBI_ERR_SPRINTF_UNSUPPORTED_CONV,
        detail: c_int::from(conv),
    }
}

/* ---------------------------------------------------------------------------
 * C float -> integer conversions (ADR-010).
 *
 * The C original casts QC floats straight to `int` / `unsigned int` /
 * `int64_t` / `uint64_t`; out-of-range and NaN inputs are UB there and QC can
 * supply both. These reproduce the x86-64 SSE2 codegen the reference build
 * emits, the same way `quake_progs::exec::c_cast_i32` does for OP_CONV.
 */

// `cvttss2si`'s representable range: `[INT64_MIN, 2^63)`. Both bounds are
// exactly representable in `f32` and `f64`. The lower bounds only matter to
// the x86-64 range check; the upper ones are also the unsigned bias.
#[cfg(target_arch = "x86_64")]
const I64_LO_F32: f32 = -9223372036854775808.0;
const I64_HI_F32: f32 = 9223372036854775808.0;
#[cfg(target_arch = "x86_64")]
const I64_LO_F64: f64 = -9223372036854775808.0;
const I64_HI_F64: f64 = 9223372036854775808.0;

/// COMPAT: ADR-010 -- `cvttss2si` (64-bit form) yields the "integer
/// indefinite" value for anything it cannot represent, NaN included.
fn c_cast_i64(v: f32) -> i64 {
    #[cfg(target_arch = "x86_64")]
    {
        if (I64_LO_F32..I64_HI_F32).contains(&v) {
            v as i64
        } else {
            i64::MIN
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        v as i64
    }
}

/// COMPAT: ADR-010 -- as `c_cast_i64`, for the `q` (64-bit) argument forms.
fn c_cast_i64_f64(v: f64) -> i64 {
    #[cfg(target_arch = "x86_64")]
    {
        if (I64_LO_F64..I64_HI_F64).contains(&v) {
            v as i64
        } else {
            i64::MIN
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        v as i64
    }
}

/// COMPAT: ADR-010 -- float -> `uint64_t`. x86-64 compilers emit
/// `comiss` + `jb` around a bias-subtract, and NaN compares unordered (CF set),
/// so NaN takes the direct `cvttss2si` arm.
fn c_cast_u64(v: f32) -> u64 {
    if v.is_nan() || v < I64_HI_F32 {
        c_cast_i64(v) as u64
    } else {
        (c_cast_i64(v - I64_HI_F32) as u64).wrapping_add(1u64 << 63)
    }
}

/// COMPAT: ADR-010 -- as `c_cast_u64`, for the `q` (64-bit) argument forms.
fn c_cast_u64_f64(v: f64) -> u64 {
    if v.is_nan() || v < I64_HI_F64 {
        c_cast_i64_f64(v) as u64
    } else {
        (c_cast_i64_f64(v - I64_HI_F64) as u64).wrapping_add(1u64 << 63)
    }
}

/// COMPAT: ADR-010 -- float -> `unsigned int` is the 64-bit `cvttss2si`
/// truncated to 32 bits on x86-64, which is why `%c` of `3e9` is not saturated.
fn c_cast_u32(v: f32) -> u32 {
    c_cast_i64(v) as u32
}

/// COMPAT: ADR-010 -- float -> `int`, shared with OP_CONV.
fn c_cast_i32(v: f32) -> i32 {
    quake_progs::exec::c_cast_i32(v)
}

/* ---------------------------------------------------------------------------
 * The `GETARG_*` family (`pr_ext.c:1134-1145`). Every one is gated on
 * `a >= firstarg && a < qcvm->argc`, which is also what keeps the global
 * offset in range.
 */

fn arg_in_range(raw: &VmRaw, firstarg: c_int, a: c_int) -> bool {
    a >= firstarg && a < raw.argc()
}

fn arg_ofs(a: c_int) -> usize {
    OFS_PARM0 + 3 * (a as usize)
}

fn getarg_float(raw: &VmRaw, firstarg: c_int, a: c_int) -> f32 {
    if arg_in_range(raw, firstarg, a) {
        raw.g_f32(arg_ofs(a))
    } else {
        0.0
    }
}

fn getarg_int(raw: &VmRaw, firstarg: c_int, a: c_int) -> i32 {
    if arg_in_range(raw, firstarg, a) {
        raw.g_i32(arg_ofs(a))
    } else {
        0
    }
}

/// The 64-bit `G_*` macros reinterpret two consecutive globals as one object.
/// Concatenating the two slots' native-endian bytes reproduces the exact
/// in-memory image on either endianness, so no new C binding is needed.
fn g_raw64(raw: &VmRaw, ofs: usize) -> [u8; 8] {
    let lo = raw.g_i32(ofs).to_ne_bytes();
    let hi = raw.g_i32(ofs + 1).to_ne_bytes();
    [lo[0], lo[1], lo[2], lo[3], hi[0], hi[1], hi[2], hi[3]]
}

fn getarg_double(raw: &VmRaw, firstarg: c_int, a: c_int) -> f64 {
    if arg_in_range(raw, firstarg, a) {
        f64::from_ne_bytes(g_raw64(raw, arg_ofs(a)))
    } else {
        0.0
    }
}

fn getarg_int64(raw: &VmRaw, firstarg: c_int, a: c_int) -> i64 {
    if arg_in_range(raw, firstarg, a) {
        i64::from_ne_bytes(g_raw64(raw, arg_ofs(a)))
    } else {
        0
    }
}

fn getarg_uint64(raw: &VmRaw, firstarg: c_int, a: c_int) -> u64 {
    if arg_in_range(raw, firstarg, a) {
        u64::from_ne_bytes(g_raw64(raw, arg_ofs(a)))
    } else {
        0
    }
}

fn getarg_string(raw: &VmRaw, firstarg: c_int, a: c_int) -> Result<&[u8], SvRaise> {
    if arg_in_range(raw, firstarg, a) {
        raw.get_string_bytes(raw.g_i32(arg_ofs(a)))
            .map_err(no_string)
    } else {
        // C's `GETARG_STRING` yields the literal `""` out of range.
        Ok(&[])
    }
}

/// `GETARG_SNUMERIC (int64_t, a)` (`pr_ext.c:1144`).
fn getarg_snumeric_i64(
    raw: &VmRaw,
    firstarg: c_int,
    a: c_int,
    isfloat: bool,
    is64bit: bool,
) -> i64 {
    match (is64bit, isfloat) {
        (true, true) => c_cast_i64_f64(getarg_double(raw, firstarg, a)),
        (true, false) => getarg_int64(raw, firstarg, a),
        (false, true) => c_cast_i64(getarg_float(raw, firstarg, a)),
        (false, false) => i64::from(getarg_int(raw, firstarg, a)),
    }
}

/// `GETARG_UNUMERIC (uint64_t, a)` (`pr_ext.c:1145`). Note the non-float,
/// non-64-bit arm is `GETARG_UINT`, i.e. a *zero*-extended 32-bit read.
fn getarg_unumeric_u64(
    raw: &VmRaw,
    firstarg: c_int,
    a: c_int,
    isfloat: bool,
    is64bit: bool,
) -> u64 {
    match (is64bit, isfloat) {
        (true, true) => c_cast_u64_f64(getarg_double(raw, firstarg, a)),
        (true, false) => getarg_uint64(raw, firstarg, a),
        (false, true) => c_cast_u64(getarg_float(raw, firstarg, a)),
        (false, false) => u64::from(getarg_int(raw, firstarg, a) as u32),
    }
}

/// `GETARG_SNUMERIC (double, a)` (`pr_ext.c:1439`).
fn getarg_snumeric_f64(
    raw: &VmRaw,
    firstarg: c_int,
    a: c_int,
    isfloat: bool,
    is64bit: bool,
) -> f64 {
    match (is64bit, isfloat) {
        (true, true) => getarg_double(raw, firstarg, a),
        (true, false) => getarg_int64(raw, firstarg, a) as f64,
        (false, true) => f64::from(getarg_float(raw, firstarg, a)),
        (false, false) => f64::from(getarg_int(raw, firstarg, a)),
    }
}

/* ---------------------------------------------------------------------------
 * The output buffer.
 */

/// C's `o` / `end` pair over `outbuf[0 .. outbuflen]`.
///
/// Invariant: `o <= len - 1`, so `terminate` is always in bounds. `o` only
/// advances from a state where `o + 1 < len` (C's `o < end - 1` gate), and
/// `write_conv` never advances past the NUL it wrote.
struct OutBuf<'a> {
    buf: &'a mut [u8],
    o: usize,
}

impl<'a> OutBuf<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, o: 0 }
    }

    /// C's `o < end - 1`.
    fn has_room(&self) -> bool {
        self.o + 1 < self.buf.len()
    }

    /// C's `end - o`, i.e. the `size` handed to `q_snprintf`; >= 2 whenever
    /// `has_room()` holds.
    fn size(&self) -> usize {
        self.buf.len() - self.o
    }

    /// C's `verbatim:` arm, `if (o < end - 1) *o++ = *s;`.
    fn push_verbatim(&mut self, b: u8) {
        if self.has_room() {
            self.buf[self.o] = b;
            self.o += 1;
        }
    }

    /// C's `q_snprintf (o, end - o, ...); o += strlen (o);`.
    ///
    /// `q_snprintf` truncates to `size - 1` bytes and NUL-terminates
    /// (`common.c:617-643`); `strlen` then stops at the first NUL, so a
    /// conversion that emitted an embedded NUL (`%c` of 0) halts the advance.
    fn write_conv(&mut self, bytes: &[u8]) {
        let size = self.size();
        debug_assert!(size >= 2);
        let n = bytes.len().min(size - 1);
        self.buf[self.o..self.o + n].copy_from_slice(&bytes[..n]);
        self.buf[self.o + n] = 0;
        let l = bytes[..n].iter().position(|&b| b == 0).unwrap_or(n);
        self.o += l;
    }

    /// C's `finished: *o = 0;`.
    fn terminate(&mut self) {
        self.buf[self.o] = 0;
    }
}

/* ---------------------------------------------------------------------------
 * Directive parsing helpers.
 */

/// `strtol (s, &err, 10)` over the digits at `start`.
///
/// Returns the `long` value (saturated on overflow, as C's `strtol` is
/// specified to do) and the index C's `err` would point at. The caller narrows
/// to `int`; see the module's overflow note for why that is platform-visible.
fn strtol_dec(fmt: &[u8], start: usize) -> (c_long, usize) {
    let mut j = start;
    let mut acc: c_long = 0;
    let mut overflow = false;
    while let Some(&d) = fmt.get(j) {
        if !d.is_ascii_digit() {
            break;
        }
        if !overflow {
            match acc
                .checked_mul(10)
                .and_then(|a| a.checked_add(c_long::from(d - b'0')))
            {
                Some(v) => acc = v,
                None => overflow = true,
            }
        }
        j += 1;
    }
    if overflow {
        acc = c_long::MAX;
    }
    (acc, j)
}

fn push_dec(spec: &mut Vec<u8>, v: usize) {
    let mut tmp = [0u8; 20];
    let mut n = 0;
    let mut v = v;
    if v == 0 {
        spec.push(b'0');
        return;
    }
    while v > 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    while n > 0 {
        n -= 1;
        spec.push(tmp[n]);
    }
}

fn warn_invalid(con: &mut SvConsole, tail: &[u8]) {
    con.warn(&c_printf(
        b"PF_sprintf: invalid format string: %s\n",
        &[Arg::Str(tail)],
    ));
}

/* ---------------------------------------------------------------------------
 * `pr_ext.c:1110` `PF_sprintf_internal`
 */

#[allow(clippy::too_many_lines)]
fn pf_sprintf_internal(
    raw: &VmRaw,
    fmt: &[u8],
    firstarg: c_int,
    out: &mut OutBuf,
    con: &mut SvConsole,
) -> SvResult {
    let mut i = 0usize;
    let mut argpos = firstarg;

    // COMPAT: C indexes a `const char *s` that walks off the end of the progs
    // string in one reachable case (see the module's overrun note); reading 0
    // past the end is the bounded stand-in.
    let at = |k: usize| -> u8 { fmt.get(k).copied().unwrap_or(0) };

    'outer: loop {
        let s0 = i;
        let c = at(i);
        if c == 0 {
            break;
        }
        if c != b'%' {
            // `default: verbatim:` (:1571)
            out.push_verbatim(c);
            i += 1;
            continue;
        }

        i += 1;
        if at(i) == b'%' {
            // `goto verbatim` with `s` on the second '%' (:1157)
            out.push_verbatim(at(i));
            i += 1;
            continue;
        }

        let mut width: c_int = -1;
        let mut precision: c_int = -1;
        let mut thisarg: c_int = -1;
        let mut flags: c_int = 0;
        let mut isfloat: c_int = -1;
        let mut is64bit = false;

        // `%N$` positional prefix, or a leading width (:1171)
        if at(i).is_ascii_digit() {
            let (v, e) = strtol_dec(fmt, i);
            width = v as c_int;
            // COMPAT: C's `if (!err)` at :1174 is dead code -- `strtol` always
            // stores a non-null end pointer -- so the "bad format string"
            // warning at :1176 can never fire. Reproduced as dead.
            if at(e) == b'$' {
                thisarg = width.wrapping_add(firstarg.wrapping_sub(1));
                width = -1;
                i = e + 1;
            } else {
                if at(i) == b'0' {
                    flags |= PRINTF_ZEROPAD;
                    if width == 0 {
                        width = -1; // it was just a flag
                    }
                }
                i = e;
            }
        }

        if width < 0 {
            // flags (:1199)
            loop {
                match at(i) {
                    b'#' => flags |= PRINTF_ALTERNATE,
                    b'0' => flags |= PRINTF_ZEROPAD,
                    b'-' => flags |= PRINTF_LEFT,
                    b' ' => flags |= PRINTF_SPACEPOSITIVE,
                    b'+' => flags |= PRINTF_SIGNPOSITIVE,
                    _ => break,
                }
                i += 1;
            }
            if at(i) == b'*' {
                i += 1;
                if at(i).is_ascii_digit() {
                    let (v, e) = strtol_dec(fmt, i);
                    if at(e) != b'$' {
                        warn_invalid(con, &fmt[s0..]);
                        break 'outer;
                    }
                    width = v as c_int;
                    i = e + 1;
                } else {
                    width = argpos;
                    argpos = argpos.wrapping_add(1);
                }
                // COMPAT: ADR-010 -- `int width = GETARG_FLOAT (width)` (:1239)
                width = c_cast_i32(getarg_float(raw, firstarg, width));
                if width < 0 {
                    flags |= PRINTF_LEFT;
                    width = width.wrapping_neg();
                }
            } else if at(i).is_ascii_digit() {
                let (v, e) = strtol_dec(fmt, i);
                // C's `if (!err)` at :1249 is dead, as above.
                width = v as c_int;
                i = e;
                if width < 0 {
                    flags |= PRINTF_LEFT;
                    width = width.wrapping_neg();
                }
            }
            // otherwise width stays -1
        }

        // precision (:1264)
        if at(i) == b'.' {
            i += 1;
            if at(i) == b'*' {
                i += 1;
                if at(i).is_ascii_digit() {
                    let (v, e) = strtol_dec(fmt, i);
                    if at(e) != b'$' {
                        warn_invalid(con, &fmt[s0..]);
                        break 'outer;
                    }
                    precision = v as c_int;
                    i = e + 1;
                } else {
                    precision = argpos;
                    argpos = argpos.wrapping_add(1);
                }
                // COMPAT: ADR-010 -- as the width cast above (:1282). A
                // negative result is not clamped here; the `precision >= 0`
                // tests below simply treat it as "not set", like C.
                precision = c_cast_i32(getarg_float(raw, firstarg, precision));
            } else if at(i).is_ascii_digit() {
                let (v, e) = strtol_dec(fmt, i);
                precision = v as c_int;
                i = e;
            } else {
                warn_invalid(con, &fmt[s0..]);
                break 'outer;
            }
        }

        // length modifiers (:1301)
        loop {
            match at(i) {
                b'h' => isfloat = 1,
                b'l' | b'L' => isfloat = 0,
                b'q' => is64bit = true,
                b'j' | b'z' | b't' => {}
                _ => break,
            }
            i += 1;
        }

        let conv = at(i);
        if conv == b'p' || conv == b'P' {
            flags |= PRINTF_ZEROPAD;
            if width < 0 {
                width = 8;
            }
            if isfloat < 0 {
                isfloat = 0;
            }
        } else if conv == b'i' && isfloat < 0 {
            isfloat = 0;
        }
        if isfloat < 0 {
            isfloat = 1;
        }
        let isfloat = isfloat != 0;

        if thisarg < 0 {
            thisarg = argpos;
            argpos = argpos.wrapping_add(1);
        }

        if out.has_room() {
            // C builds `formatbuf` with `*` placeholders and passes width /
            // precision as varargs. `quake_util::printf` has no `*`, so the
            // values are substituted as literal digits instead. C's
            // `if (width < 0) width = 0;` (:1407) is hoisted above the spec
            // build; nothing between the two points reads `width`.
            if width < 0 {
                width = 0;
            }

            // Clamp the field sizes so a hostile `%*d` cannot ask `printf`
            // for a gigabyte (see the allocation note in the module docs).
            let w = (width as usize).min(FIELD_CLAMP);
            let p = if precision >= 0 {
                Some((precision as usize).min(FIELD_CLAMP))
            } else {
                None
            };

            let mut spec: Vec<u8> = Vec::with_capacity(24);
            spec.push(b'%');
            if conv != b's' && conv != b'c' && (flags & PRINTF_ALTERNATE) != 0 {
                spec.push(b'#');
            }
            if (flags & PRINTF_ZEROPAD) != 0 {
                spec.push(b'0');
            }
            if (flags & PRINTF_LEFT) != 0 {
                spec.push(b'-');
            }
            if (flags & PRINTF_SPACEPOSITIVE) != 0 {
                spec.push(b' ');
            }
            if (flags & PRINTF_SIGNPOSITIVE) != 0 {
                spec.push(b'+');
            }
            // A zero width must emit *no* digits: `%0d` would re-read the `0`
            // as the zero-pad flag.
            if w > 0 {
                push_dec(&mut spec, w);
            }
            if let Some(p) = p {
                spec.push(b'.');
                push_dec(&mut spec, p);
            }

            match conv {
                // :1412 -- `d`, `i`
                b'd' | b'i' => {
                    spec.extend_from_slice(b"ll");
                    spec.push(conv);
                    let v = getarg_snumeric_i64(raw, firstarg, thisarg, isfloat, is64bit);
                    out.write_conv(&c_printf(&spec, &[Arg::I64(v)]));
                }
                // :1420 -- `o` has no `quake_util::printf` conversion.
                b'o' => return Err(unsupported_conv(conv)),
                // :1421 -- `u`, `x`, `X`, `p`, `P`
                b'u' | b'x' | b'X' | b'p' | b'P' => {
                    spec.extend_from_slice(b"ll");
                    spec.push(match conv {
                        b'p' => b'x',
                        b'P' => b'X',
                        other => other,
                    });
                    let v = getarg_unumeric_u64(raw, firstarg, thisarg, isfloat, is64bit);
                    out.write_conv(&c_printf(&spec, &[Arg::U64(v)]));
                }
                // :1434 -- `f`, `F`
                b'f' | b'F' => {
                    spec.push(conv);
                    let v = getarg_snumeric_f64(raw, firstarg, thisarg, isfloat, is64bit);
                    out.write_conv(&c_printf(&spec, &[Arg::F64(v)]));
                }
                // :1432/:1436 -- `e`, `E`, `g`, `G` (ADR-005), and :1444 --
                // `v`, `V`, which rewrite themselves into `g`/`G`.
                b'e' | b'E' | b'g' | b'G' | b'v' | b'V' => return Err(unsupported_conv(conv)),
                // :1461 -- `c`
                b'c' => {
                    spec.push(b'c');
                    let v = if isfloat {
                        c_cast_u32(getarg_float(raw, firstarg, thisarg))
                    } else {
                        getarg_int(raw, firstarg, thisarg) as u32
                    };
                    out.write_conv(&c_printf(&spec, &[Arg::I32(v as i32)]));
                }
                // :1485 -- `S`, a tokenizable (escaped, quoted) string
                b'S' => {
                    let quotedarg = getarg_string(raw, firstarg, thisarg)?;
                    let (quoted, warn, truncated) = quote_arg(quotedarg);
                    if warn || truncated {
                        con.warn(&c_printf(
                            b"PF_sprintf: unable to safely escape arg: %i\n",
                            &[Arg::I32(thisarg.wrapping_add(1))],
                        ));
                    }
                    // C picks `quotedbuf` (with the leading '\\') when it had
                    // to warn, and `quotedbuf + 1` otherwise.
                    let shown: &[u8] = if warn || truncated {
                        &quoted
                    } else {
                        &quoted[1..]
                    };
                    spec.push(b's');
                    out.write_conv(&c_printf(&spec, &[Arg::Str(shown)]));
                }
                // :1546 -- `s`
                b's' => {
                    spec.push(b's');
                    let s = getarg_string(raw, firstarg, thisarg)?;
                    out.write_conv(&c_printf(&spec, &[Arg::Str(s)]));
                }
                // :1564 -- everything else, `I` included
                _ => {
                    warn_invalid(con, &fmt[s0..]);
                    break 'outer;
                }
            }
        }
        i += 1;
    }

    out.terminate();
    Ok(())
}

/// `pr_ext.c:1489-1527`, the `%S` escape pass.
///
/// Returns the escaped buffer (leading `\` included), whether an escape was
/// emitted, and whether the input was cut short by the `countof (quotedbuf) - 4`
/// cap -- C's `*quotedarg` test at :1521.
///
/// The C loop is in bounds: `l < 65532` on entry, at most two bytes are written
/// per iteration, so the trailing `quotedbuf[l] = '"'` / `quotedbuf[l + 1] = 0`
/// touch at most indices 65534/65535.
fn quote_arg(arg: &[u8]) -> (Vec<u8>, bool, bool) {
    let mut q: Vec<u8> = Vec::with_capacity(arg.len() + 4);
    q.push(b'\\');
    q.push(b'"');
    let mut l = 2usize;
    let mut warn = false;
    let mut k = 0usize;
    while k < arg.len() && l < QUOTEDBUF_SIZE - 4 {
        match arg[k] {
            b'\n' => {
                q.push(b'\\');
                q.push(b'n');
                l += 2;
                warn = true;
            }
            b'\r' => {
                q.push(b'\\');
                q.push(b'r');
                l += 2;
                warn = true;
            }
            b'"' => {
                q.push(b'\\');
                q.push(b'"');
                l += 2;
                warn = true;
            }
            other => {
                q.push(other);
                l += 1;
            }
        }
        k += 1;
    }
    q.push(b'"');
    (q, warn, k < arg.len())
}

/* ---------------------------------------------------------------------------
 * `pr_ext.c:1583` `PF_sprintf`
 */

fn pf_sprintf(vm: *mut QcVm, con: &mut SvConsole) -> SvResult {
    // SAFETY: ADR-008 -- a builtin only runs inside PR_ExecuteProgram, so the
    // ambient qcvm and its lumps are live for the whole call.
    let mut raw = unsafe { VmRaw::new(vm) };

    // COMPAT: C initialises `outbuf` before evaluating `G_STRING (OFS_PARM0)`
    // as the call's argument, so the temp-string ring steps even on the call
    // that raises on a bad format handle.
    // SAFETY: `PR_GetTempString` is an index into a static ring; the returned
    // buffer is `STRINGTEMP_LENGTH` bytes and stays valid for this builtin.
    let outbuf = unsafe { PR_GetTempString() };
    // SAFETY: as above -- `STRINGTEMP_LENGTH` bytes of a live static array,
    // uniquely borrowed for the duration of this call.
    let buf = unsafe { core::slice::from_raw_parts_mut(outbuf.cast::<u8>(), STRINGTEMP_LENGTH) };

    let fmt = raw
        .get_string_bytes(raw.g_i32(OFS_PARM0))
        .map_err(no_string)?;

    let mut out = OutBuf::new(buf);
    pf_sprintf_internal(&raw, fmt, 1, &mut out, con)?;

    // SAFETY: `outbuf` is the temp-string ring, not the progs string blob, so
    // PR_SetEngineString takes its `known strings` path (as in zone.rs).
    let handle = unsafe { g::PR_SetEngineString(outbuf) };
    raw.set_g_i32(OFS_RETURN, handle);
    Ok(())
}

/// `pr_ext.c:1583` `PF_sprintf`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sprintf(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, pf_sprintf) }
}
