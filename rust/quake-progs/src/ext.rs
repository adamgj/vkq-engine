//! The self-contained `pr_ext.c` builtins (Phase 6 M9): maths, the scalar and
//! string type conversions, and the string manipulation set.
//!
//! `pr_ext.c` is 6800 lines and most of it is not self-contained — the CSQC
//! 2D drawing block and `PF_getsurface*` reach the renderer (Phase 8), the
//! temp-entity and particle blocks reach the server, and the extension
//! machinery has its own milestone. What is here is the part whose behaviour
//! is entirely its own, which is also where the compatibility quirks are
//! densest: C's implicit float→int conversions on almost every argument, the
//! shared 1024-byte temp-string ring's truncation rules, and three
//! outright bugs that shipping mods depend on (see the `COMPAT` notes on
//! [`pf_strireplace`], [`pf_str2chr`] and [`pf_strncmp`]).
//!
//! Everything routes through [`crate::builtins::BuiltinSys`], so the seam
//! rules recorded there apply unchanged: no ported builtin calls anything that
//! can `Host_Error` except through a guarded seam.

use core::ffi::c_int;

use quake_types::progs::{OFS_PARM0, OFS_PARM1, OFS_PARM2, OFS_RETURN};
use quake_util::printf::{format, Arg};
use quake_util::qctype::{q_tolower, q_toupper};

use crate::arena::VmRaw;
use crate::builtins::{string_arg, BuiltinError, BuiltinSys, STRINGTEMP_LENGTH};
use crate::exec::c_cast_i32;

/// Parameter offsets past the third; QuakeC parameters are three words apart.
fn parm(i: usize) -> usize {
    OFS_PARM0 + i * 3
}

// ---------------------------------------------------------------------------
// maths
//
// Every one of these takes and returns `float` but computes in `double`,
// because the libm functions do. The `as f32` at the end is the C assignment
// back into the globals block.

macro_rules! unary_math {
    ($name:ident, $doc:literal, $call:ident) => {
        #[doc = $doc]
        pub fn $name(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
            let v = f64::from(vm.g_f32(OFS_PARM0));
            let r = sys.$call(v);
            vm.set_g_f32(OFS_RETURN, r as f32);
        }
    };
}

unary_math!(pf_sin, "`float sin (float)`", sin);
unary_math!(pf_cos, "`float cos (float)`", cos);
unary_math!(pf_tan, "`float tan (float)`", tan);
unary_math!(pf_asin, "`float asin (float)`", asin);
unary_math!(pf_acos, "`float acos (float)`", acos);
unary_math!(pf_atan, "`float atan (float)`", atan);
unary_math!(pf_sqrt, "`float sqrt (float)`", sqrt);

/// `float atan2 (float y, float x)`
pub fn pf_atan2(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
    let (y, x) = (
        f64::from(vm.g_f32(OFS_PARM0)),
        f64::from(vm.g_f32(OFS_PARM1)),
    );
    let r = sys.atan2(y, x);
    vm.set_g_f32(OFS_RETURN, r as f32);
}

/// `float pow (float value, float exp)`
pub fn pf_pow(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
    let (a, b) = (
        f64::from(vm.g_f32(OFS_PARM0)),
        f64::from(vm.g_f32(OFS_PARM1)),
    );
    let r = sys.pow(a, b);
    vm.set_g_f32(OFS_RETURN, r as f32);
}

/// `float log (float value, optional float base)` — `log2 (v) = ln (v) / ln (2)`.
///
/// COMPAT: the division is done in `double` and only then narrowed, and a
/// second argument of 1 divides by `ln (1) == 0` rather than being rejected.
pub fn pf_logarithm(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
    let mut r = sys.log(f64::from(vm.g_f32(OFS_PARM0)));
    if vm.argc() > 1 {
        r /= sys.log(f64::from(vm.g_f32(OFS_PARM1)));
    }
    vm.set_g_f32(OFS_RETURN, r as f32);
}

/// `float mod (float a, float n)`
///
/// COMPAT: "because QC is inherantly floaty, lets use floats" —
/// `a - (n * (int)(a / n))`, so the quotient goes through the per-arch
/// float→int conversion, and mod-by-zero warns and returns 0 rather than
/// producing a NaN.
pub fn pf_mod(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
    let a = vm.g_f32(OFS_PARM0);
    let n = vm.g_f32(OFS_PARM1);
    if n == 0.0 {
        sys.dwarn(b"PF_mod: mod by zero\n");
        vm.set_g_f32(OFS_RETURN, 0.0);
    } else {
        vm.set_g_f32(OFS_RETURN, a - (n * c_cast_i32(a / n) as f32));
    }
}

/// `float min (float a, float b, ...)`
///
/// COMPAT: the fold starts at the *first* argument and compares with `>`, so a
/// NaN anywhere after the first argument never replaces the running value, and
/// a NaN first argument is never replaced either.
pub fn pf_min(vm: &mut VmRaw) {
    let mut r = vm.g_f32(OFS_PARM0);
    for i in 1..vm.argc().max(0) as usize {
        let v = vm.g_f32(parm(i));
        if r > v {
            r = v;
        }
    }
    vm.set_g_f32(OFS_RETURN, r);
}

/// `float max (float a, float b, ...)` — see [`pf_min`] on NaN.
pub fn pf_max(vm: &mut VmRaw) {
    let mut r = vm.g_f32(OFS_PARM0);
    for i in 1..vm.argc().max(0) as usize {
        let v = vm.g_f32(parm(i));
        if r < v {
            r = v;
        }
    }
    vm.set_g_f32(OFS_RETURN, r);
}

/// `float bound (float minimum, float val, float maximum)`
///
/// COMPAT: the max clamp runs first, so `bound (10, x, 5)` returns 10, not 5.
pub fn pf_bound(vm: &mut VmRaw) {
    let minval = vm.g_f32(OFS_PARM0);
    let mut cur = vm.g_f32(OFS_PARM1);
    let maxval = vm.g_f32(OFS_PARM2);
    if cur > maxval {
        cur = maxval;
    }
    if cur < minval {
        cur = minval;
    }
    vm.set_g_f32(OFS_RETURN, cur);
}

/// `float anglemod (float value)`
///
/// COMPAT: this is **not** `mathlib.c`'s `anglemod` — it is a subtract-loop,
/// so it is exact for small angles where the mathlib version quantises to
/// 1/65536 of a turn, and it does not terminate in bounded time for a very
/// large input. `PF_changeyaw` uses the mathlib one; this builtin does not.
pub fn pf_anglemod(vm: &mut VmRaw) {
    let mut v = vm.g_f32(OFS_PARM0);
    while v >= 360.0 {
        v -= 360.0;
    }
    while v < 0.0 {
        v += 360.0;
    }
    vm.set_g_f32(OFS_RETURN, v);
}

/// `float bitshift (float bitmask, float shift)`
///
/// COMPAT: both arguments go through the per-arch float→int conversion, and
/// the shift is a C shift on `int` — so a shift of 32 or more is UB in C and
/// is reproduced here as the platform's wrapping shift, which is what the
/// hardware does on both targets this engine builds for.
pub fn pf_bitshift(vm: &mut VmRaw) {
    let bitmask = c_cast_i32(vm.g_f32(OFS_PARM0));
    let shift = c_cast_i32(vm.g_f32(OFS_PARM1));
    let r = if shift < 0 {
        bitmask.wrapping_shr(shift.unsigned_abs())
    } else {
        bitmask.wrapping_shl(shift as u32)
    };
    vm.set_g_f32(OFS_RETURN, r as f32);
}

/// `vector crossproduct (vector a, vector b)`
pub fn pf_crossproduct(vm: &mut VmRaw) {
    let a = vm.g_vec3(OFS_PARM0);
    let b = vm.g_vec3(OFS_PARM1);
    vm.set_g_vec3(
        OFS_RETURN,
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ],
    );
}

/// `void vectorvectors (vector forward)` — fills `v_forward`/`v_right`/`v_up`
/// from a forward vector alone.
pub fn pf_vectorvectors(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
    sys.vector_vectors(vm.g_vec3(OFS_PARM0));
}

/// `vector vectoangles2 (vector fwd, optional vector up)`
///
/// COMPAT: the pitch is negated afterwards — "models have an inverted pitch.
/// consistency with makevectors would never do!"
pub fn pf_ext_vectoangles(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
    let fwd = vm.g_vec3(OFS_PARM0);
    let up = (vm.argc() >= 2).then(|| vm.g_vec3(OFS_PARM1));
    let mut out = sys.vector_angles(fwd, up);
    out[0] *= -1.0;
    vm.set_g_vec3(OFS_RETURN, out);
}

// ---------------------------------------------------------------------------
// type conversions

/// `float stof (string)` — the platform `atof` (ADR-010).
pub fn pf_stof(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    let s = string_arg(vm, vm.g_i32(OFS_PARM0))?;
    let v = sys.atof(&s);
    vm.set_g_f32(OFS_RETURN, v as f32);
    Ok(())
}

/// `int stoi (string)` — the platform `atoi`.
pub fn pf_stoi(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    let s = string_arg(vm, vm.g_i32(OFS_PARM0))?;
    let v = sys.atoi(&s);
    vm.set_g_i32(OFS_RETURN, v);
    Ok(())
}

/// `int stoh (string)` — `strtoul (s, NULL, 16)`, so no `0x` prefix is needed
/// and one is tolerated, and the result is truncated to `int`.
pub fn pf_stoh(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    let s = string_arg(vm, vm.g_i32(OFS_PARM0))?;
    let v = sys.strtoul_hex(&s);
    vm.set_g_i32(OFS_RETURN, v as c_int);
    Ok(())
}

/// `string itos (int)`
pub fn pf_itos(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
    let bytes = format(b"%i", &[Arg::I32(vm.g_i32(OFS_PARM0))]);
    let handle = sys.store_temp_string(&bytes);
    vm.set_g_i32(OFS_RETURN, handle);
}

/// `string htos (int)`
pub fn pf_htos(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
    let bytes = format(b"%x", &[Arg::U32(vm.g_i32(OFS_PARM0) as u32)]);
    let handle = sys.store_temp_string(&bytes);
    vm.set_g_i32(OFS_RETURN, handle);
}

/// `string etos (entity)` — `"entity %i"`, "yes, this is lame".
pub fn pf_etos(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    let num = crate::builtins::num_for_edict(vm, vm.g_i32(OFS_PARM0))?;
    let bytes = format(b"entity %i", &[Arg::I32(num)]);
    let handle = sys.store_temp_string(&bytes);
    vm.set_g_i32(OFS_RETURN, handle);
    Ok(())
}

/// `int ftoi (float)` — a bare C cast, so per-arch.
pub fn pf_ftoi(vm: &mut VmRaw) {
    let v = c_cast_i32(vm.g_f32(OFS_PARM0));
    vm.set_g_i32(OFS_RETURN, v);
}

/// `float itof (int)`
pub fn pf_itof(vm: &mut VmRaw) {
    let v = vm.g_i32(OFS_PARM0);
    vm.set_g_f32(OFS_RETURN, v as f32);
}

/// `float etof (entity)` — `G_EDICTNUM`, so it range-checks.
pub fn pf_num_for_edict(vm: &mut VmRaw) -> Result<(), BuiltinError> {
    let num = crate::builtins::num_for_edict(vm, vm.g_i32(OFS_PARM0))?;
    vm.set_g_f32(OFS_RETURN, num as f32);
    Ok(())
}

/// `entity ftoe (float)` — `EDICT_TO_PROG (EDICT_NUM (n))`.
///
/// COMPAT: `EDICT_NUM`'s range check is `n < 0 || n >= max_edicts` and raises
/// in release builds, and the argument reaches it through the per-arch
/// float→int conversion.
pub fn pf_edict_for_num(vm: &mut VmRaw) -> Result<(), BuiltinError> {
    let num = c_cast_i32(vm.g_f32(OFS_PARM0));
    if num < 0 || num >= vm.max_edicts() {
        return Err(BuiltinError::BadEdictNum(num));
    }
    let prog = vm.to_prog_num(num);
    vm.set_g_i32(OFS_RETURN, prog);
    Ok(())
}

// ---------------------------------------------------------------------------
// strings
//
// Everything below formats into the process-global temp-string ring, so every
// one of them truncates at STRINGTEMP_LENGTH - 1 and the truncation point is
// observable.

/// `float strlen (string)`
pub fn pf_strlen(vm: &mut VmRaw) -> Result<(), BuiltinError> {
    let s = string_arg(vm, vm.g_i32(OFS_PARM0))?;
    vm.set_g_f32(OFS_RETURN, s.len() as f32);
    Ok(())
}

/// `string strcat (string, ...)`
///
/// COMPAT: `q_strlcat` returns the length it *would* have produced, so the
/// overflow test fires on the argument that first exceeded the buffer and the
/// loop breaks — leaving the truncated concatenation, not an empty string.
pub fn pf_strcat(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    let mut out: Vec<u8> = Vec::new();
    for i in 0..vm.argc().max(0) as usize {
        let piece = string_arg(vm, vm.g_i32(parm(i)))?;
        let would = out.len() + piece.len();
        let room = STRINGTEMP_LENGTH - 1 - out.len().min(STRINGTEMP_LENGTH - 1);
        out.extend_from_slice(&piece[..piece.len().min(room)]);
        if would >= STRINGTEMP_LENGTH {
            sys.warn(b"PF_strcat: overflow (string truncated)\n");
            break;
        }
    }
    let handle = sys.store_temp_string(&out);
    vm.set_g_i32(OFS_RETURN, handle);
    Ok(())
}

/// `string substring (string s, float start, float length)`
///
/// COMPAT: a negative `start` counts from the end; a negative `length` means
/// "up to `length + 1` from the end". `start` is re-clamped to 0 *after* the
/// length adjustment, so `substring (s, -100, -1)` on a short string keeps the
/// whole thing. Both arguments arrive through the per-arch float→int cast.
pub fn pf_substring(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    let s = string_arg(vm, vm.g_i32(OFS_PARM0))?;
    let mut start = c_cast_i32(vm.g_f32(OFS_PARM1));
    let mut length = c_cast_i32(vm.g_f32(OFS_PARM2));
    let mut slen = s.len() as c_int;

    if start < 0 {
        start += slen;
    }
    if length < 0 {
        length = slen - start + (length + 1);
    }
    if start < 0 {
        start = 0;
    }

    if start >= slen || length <= 0 {
        // COMPAT: C's empty arm is `PR_SetEngineString ("")` on the literal,
        // not `PR_GetTempString`, so it neither steps the process-global temp
        // ring nor returns a fresh handle. See `BuiltinSys::empty_engine_string`.
        let handle = sys.empty_engine_string();
        vm.set_g_i32(OFS_RETURN, handle);
        return Ok(());
    }

    slen -= start;
    if length > slen {
        length = slen;
    }

    if length >= STRINGTEMP_LENGTH as c_int {
        length = STRINGTEMP_LENGTH as c_int - 1;
        sys.warn(b"PF_substring: truncation\n");
    }

    let at = start as usize;
    let handle = sys.store_temp_string(&s[at..at + length as usize]);
    vm.set_g_i32(OFS_RETURN, handle);
    Ok(())
}

/// `float str2chr (string, optional float index)`
///
/// COMPAT (bug preserved): the range test is `ofs && (ofs < 0 || ofs > len)`,
/// so index 0 is *never* rejected — `str2chr ("", 0)` reads the NUL and
/// returns 0 rather than taking the error arm — and an index exactly equal to
/// the length is accepted, also reading the NUL.
pub fn pf_str2chr(vm: &mut VmRaw) -> Result<(), BuiltinError> {
    let s = string_arg(vm, vm.g_i32(OFS_PARM0))?;
    let len = s.len() as c_int;
    let mut ofs = if vm.argc() > 1 {
        c_cast_i32(vm.g_f32(OFS_PARM1))
    } else {
        0
    };
    if ofs < 0 {
        ofs += len;
    }

    let out = if ofs != 0 && (ofs < 0 || ofs > len) {
        0.0
    } else if ofs == len {
        // the NUL terminator, which C reads because the bound is inclusive
        0.0
    } else {
        f32::from(s[ofs as usize])
    };
    vm.set_g_f32(OFS_RETURN, out);
    Ok(())
}

/// `string chr2str (float, ...)`
///
/// COMPAT: the loop bound is `STRINGTEMP_LENGTH - 6`, not `- 1`, so at most
/// 1018 characters are produced however many arguments are passed; values in
/// `0xe000..0xe100` are the Quake charset and are truncated to a byte.
///
/// COMPAT: the ASCII test is `pr_ext.c`'s own `qc_isascii`, which is
/// `u < 256` — **not** `q_ctype.h`'s `q_isascii`, which is `(c & ~0x7f) == 0`.
/// The comment on it says why: "should be just \n and 32-127, but we don't
/// actually support any actual unicode and we don't really want to make things
/// worse." Using the `q_ctype.h` one would turn every high-bit byte, i.e. the
/// whole coloured-text charset, into `?`.
pub fn pf_chr2str(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
    let mut out = Vec::new();
    let argc = vm.argc().max(0) as usize;
    for i in 0..argc {
        if out.len() >= STRINGTEMP_LENGTH - 6 {
            break;
        }
        let u = c_cast_i32(vm.g_f32(parm(i))) as u32;
        out.push(if (0xe000..0xe100).contains(&u) {
            (u & 0xff) as u8
        } else if u < 256 {
            u as u8
        } else {
            b'?'
        });
    }
    let handle = sys.store_temp_string(&out);
    vm.set_g_i32(OFS_RETURN, handle);
}

/// `string strpad (float pad, string, ...)`
///
/// COMPAT: a negative `pad` pads on the left. The left arm computes
/// `-pad - strlen (src)` and then `q_strlcpy`s the source at that offset, so
/// a source longer than the field is *not* truncated to the field width — it
/// simply gets no padding.
pub fn pf_strpad(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
    let mut pad = c_cast_i32(vm.g_f32(OFS_PARM0));
    let src = sys.var_string(1);
    let cap = STRINGTEMP_LENGTH as c_int;
    let mut out = Vec::new();

    if pad < 0 {
        pad = -pad - src.len() as c_int;
        if pad >= cap {
            pad = cap - 1;
        }
        if pad < 0 {
            pad = 0;
        }
        out.resize(pad as usize, b' ');
        let room = (cap - pad - 1).max(0) as usize;
        out.extend_from_slice(&src[..src.len().min(room)]);
    } else {
        if pad >= cap {
            pad = cap - 1;
        }
        pad -= src.len() as c_int;
        if pad < 0 {
            pad = 0;
        }
        let room = cap as usize - 1;
        out.extend_from_slice(&src[..src.len().min(room)]);
        out.resize((out.len() + pad as usize).min(room), b' ');
    }

    let handle = sys.store_temp_string(&out);
    vm.set_g_i32(OFS_RETURN, handle);
}

/// The offset clamp `PF_strncmp`/`PF_strncasecmp` share.
///
/// COMPAT (bug preserved): the test is `ofs < 0 || (ofs && ofs > len)`, so a
/// *zero* offset skips the upper-bound clamp — harmless — but an out-of-range
/// offset is clamped to `len`, i.e. to the terminator, rather than rejected.
fn cmp_offset(ofs: c_int, len: usize) -> usize {
    if ofs < 0 || (ofs != 0 && ofs > len as c_int) {
        len
    } else {
        ofs as usize
    }
}

/// `float strncmp (string a, string b, optional float len, optional float aofs,
/// optional float bofs)`
///
/// COMPAT (bug preserved): only `a` is offset by `aofs`. `bofs` is computed
/// and clamped and then **never used** — `strncmp (a + aofs, b, len)`.
pub fn pf_strncmp(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    cmp_impl(vm, sys, false)
}

/// `float strncasecmp (...)` — as [`pf_strncmp`], including the unused `bofs`.
pub fn pf_strncasecmp(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    cmp_impl(vm, sys, true)
}

fn cmp_impl(vm: &mut VmRaw, sys: &mut dyn BuiltinSys, fold_case: bool) -> Result<(), BuiltinError> {
    let a = string_arg(vm, vm.g_i32(OFS_PARM0))?;
    let b = string_arg(vm, vm.g_i32(OFS_PARM1))?;

    // The result is `strcmp`'s raw return value stored into a float slot, so
    // the platform's magnitude is observable to QuakeC (ADR-010) -- the same
    // argument as OP_NE_S in the interpreter.
    let r = if vm.argc() > 2 {
        let len = c_cast_i32(vm.g_f32(OFS_PARM2));
        let aofs = if vm.argc() > 3 {
            c_cast_i32(vm.g_f32(parm(3)))
        } else {
            0
        };
        // bofs is read and clamped by C and then discarded; reproduced so the
        // argument reads have the same shape, and named so the omission is
        // visibly deliberate.
        let _bofs = if vm.argc() > 4 {
            cmp_offset(c_cast_i32(vm.g_f32(parm(4))), b.len())
        } else {
            0
        };
        let aofs = cmp_offset(aofs, a.len());
        sys.strncmp(&a[aofs..], &b, len, fold_case)
    } else {
        sys.strcmp(&a, &b, fold_case)
    };
    vm.set_g_f32(OFS_RETURN, r as f32);
    Ok(())
}

/// `float strstrofs (string haystack, string needle, optional float start)`
///
/// COMPAT: as in the compare builtins, a zero `start` skips the range test.
pub fn pf_strstrofs(vm: &mut VmRaw) -> Result<(), BuiltinError> {
    let hay = string_arg(vm, vm.g_i32(OFS_PARM0))?;
    let needle = string_arg(vm, vm.g_i32(OFS_PARM1))?;
    let first = if vm.argc() > 2 {
        c_cast_i32(vm.g_f32(OFS_PARM2))
    } else {
        0
    };

    if first != 0 && (first < 0 || first > hay.len() as c_int) {
        vm.set_g_f32(OFS_RETURN, -1.0);
        return Ok(());
    }

    let at = first as usize;
    let found = if needle.is_empty() {
        Some(at)
    } else {
        hay[at..]
            .windows(needle.len())
            .position(|w| w == needle)
            .map(|p| p + at)
    };
    vm.set_g_f32(OFS_RETURN, found.map_or(-1.0, |p| p as f32));
    Ok(())
}

/// `string strtrim (string)` — strips spaces, tabs, newlines and carriage
/// returns from both ends. Not `q_isspace`: vertical tab and form feed are
/// *not* stripped.
pub fn pf_strtrim(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    const WS: &[u8] = b" \t\n\r";
    let s = string_arg(vm, vm.g_i32(OFS_PARM0))?;
    let start = s.iter().position(|b| !WS.contains(b)).unwrap_or(s.len());
    let end = s
        .iter()
        .rposition(|b| !WS.contains(b))
        .map_or(start, |p| p + 1);
    let trimmed = &s[start..end.max(start)];
    let handle = sys.store_temp_string(&trimmed[..trimmed.len().min(STRINGTEMP_LENGTH - 1)]);
    vm.set_g_i32(OFS_RETURN, handle);
    Ok(())
}

/// `string strreplace (string search, string replace, string subject)`
pub fn pf_strreplace(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    replace_impl(vm, sys, false)
}

/// `string strireplace (string search, string replace, string subject)`
///
/// COMPAT (bug preserved, and it is a big one): the loop bound in `pr_ext.c`
/// is `result < resultbuf + sizeof (resultbuf) - replacelen - 2`, where
/// `resultbuf` is a `char *` — so `sizeof` is the **pointer size, 8**, not
/// `STRINGTEMP_LENGTH`. The case-insensitive variant therefore produces at
/// most `8 - replacelen - 2` bytes, and none at all once the replacement is
/// six bytes or longer. `strreplace` next door uses the constant and is fine.
pub fn pf_strireplace(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    replace_impl(vm, sys, true)
}

fn replace_impl(
    vm: &mut VmRaw,
    sys: &mut dyn BuiltinSys,
    fold_case: bool,
) -> Result<(), BuiltinError> {
    let search = string_arg(vm, vm.g_i32(OFS_PARM0))?;
    let replace = string_arg(vm, vm.g_i32(OFS_PARM1))?;
    let subject = string_arg(vm, vm.g_i32(OFS_PARM2))?;

    if search.is_empty() {
        // C hands `PR_SetEngineString` the *subject* pointer itself, not a
        // temp string. For a blob offset that returns `s - qcvm->strings`, and
        // for an engine string it finds the same slot -- either way the
        // caller's own handle comes back, not a fresh one. Allocating a temp
        // string here would hand QuakeC a different handle for the same bytes,
        // which `strunzone` and handle comparisons can see.
        let handle = vm.g_i32(OFS_PARM2);
        vm.set_g_i32(OFS_RETURN, handle);
        return Ok(());
    }

    // See the COMPAT note on pf_strireplace: the two variants have different
    // capacities because of the `sizeof (char *)` bug.
    let capacity = if fold_case {
        core::mem::size_of::<*const u8>()
    } else {
        STRINGTEMP_LENGTH
    };
    let limit = (capacity as isize) - (replace.len() as isize) - 2;

    let mut out: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < subject.len() && (out.len() as isize) < limit {
        let hit = subject.len() - i >= search.len()
            && if fold_case {
                subject[i..i + search.len()]
                    .iter()
                    .zip(search.iter())
                    .all(|(&x, &y)| q_tolower(i32::from(x)) == q_tolower(i32::from(y)))
            } else {
                subject[i..i + search.len()] == search[..]
            };
        if hit {
            i += search.len();
            out.extend_from_slice(&replace);
        } else {
            out.push(subject[i]);
            i += 1;
        }
    }

    let handle = sys.store_temp_string(&out);
    vm.set_g_i32(OFS_RETURN, handle);
    Ok(())
}

/// `string strtoupper (string)` — `q_toupper`, which is ASCII-only, so the
/// Quake charset's high-bit letters are left alone.
pub fn pf_strtoupper(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    case_impl(vm, sys, q_toupper)
}

/// `string strtolower (string)`
pub fn pf_strtolower(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    case_impl(vm, sys, q_tolower)
}

fn case_impl(
    vm: &mut VmRaw,
    sys: &mut dyn BuiltinSys,
    f: fn(c_int) -> c_int,
) -> Result<(), BuiltinError> {
    let s = string_arg(vm, vm.g_i32(OFS_PARM0))?;
    let n = s.len().min(STRINGTEMP_LENGTH - 1);
    // C's loop writes `q_toupper (*in++)` into a `char`, so the conversion is
    // over the *signed* char value on platforms where char is signed. Both
    // targets sign-extend, and q_toupper only touches 'a'..'z', so the byte
    // round-trips either way.
    let out: Vec<u8> = s[..n].iter().map(|&b| f(i32::from(b)) as u8).collect();
    let handle = sys.store_temp_string(&out);
    vm.set_g_i32(OFS_RETURN, handle);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmp_offset_clamps_out_of_range_to_the_terminator() {
        assert_eq!(cmp_offset(0, 5), 0, "zero always passes");
        assert_eq!(cmp_offset(3, 5), 3);
        assert_eq!(cmp_offset(5, 5), 5, "exactly the length is in range");
        assert_eq!(cmp_offset(6, 5), 5, "past the end clamps to the NUL");
        assert_eq!(
            cmp_offset(-1, 5),
            5,
            "negative clamps too, it does not wrap"
        );
    }
}
