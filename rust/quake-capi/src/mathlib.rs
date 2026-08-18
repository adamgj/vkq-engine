//! C ABI shims for `Quake/mathlib.c` (declarations stay in `Quake/mathlib.h`;
//! `PerpendicularVector` is header-undeclared but called from r_part_fte.c).
//!
//! Inputs are copied to locals before outputs are written: for the
//! element-wise vector ops this is bit-identical to the C even when callers
//! alias input and output arrays (each element is read before it is written).

use core::ffi::{c_char, c_double, c_float, c_int};
use quake_math::mathlib as m;
use quake_types::MPlane;

/// C: `vec3_t vec3_origin` (mathlib.h) — read-only in practice, but the C
/// symbol is a mutable array.
#[no_mangle]
pub static mut vec3_origin: [f32; 3] = [0.0, 0.0, 0.0];

#[inline]
unsafe fn v3(p: *const c_float) -> [f32; 3] {
    // SAFETY: caller contract (vec3_t parameter) guarantees 3 readable floats
    unsafe { *(p as *const [f32; 3]) }
}

#[inline]
unsafe fn store3(p: *mut c_float, v: [f32; 3]) {
    // SAFETY: caller contract (vec3_t parameter) guarantees 3 writable floats
    unsafe { *(p as *mut [f32; 3]) = v }
}

// ---------------------------------------------------------------------------
// Pointer-conversion combinators.
//
// `v3`/`store3` above are the only places a raw `vec3_t` is dereferenced, and
// these combinators are the only callers. Every shim below goes through one of
// them, so the "is this pointer really three readable/writable floats?"
// argument is made once here rather than restated at each of the ~25 entry
// points -- which is what ADR-004 means by concentrating unsafe.
//
// All of them copy inputs into locals before writing any output. That keeps
// the engine's very common aliasing calls -- `VectorAdd (ent->origin, delta,
// ent->origin)` -- well-defined, and element-for-element identical to the C.
// ---------------------------------------------------------------------------

/// `out = f(a)`
///
/// # Safety
/// `a` must point to 3 readable floats, `out` to 3 writable floats.
#[inline]
unsafe fn map1(a: *const c_float, out: *mut c_float, f: impl FnOnce(&[f32; 3], &mut [f32; 3])) {
    // SAFETY: forwarded to the caller's vec3_t contract
    unsafe {
        let a = v3(a);
        let mut o = [0.0f32; 3];
        f(&a, &mut o);
        store3(out, o);
    }
}

/// `out = f(a, b)`
///
/// # Safety
/// `a`/`b` must point to 3 readable floats, `out` to 3 writable floats.
#[inline]
unsafe fn map2(
    a: *const c_float,
    b: *const c_float,
    out: *mut c_float,
    f: impl FnOnce(&[f32; 3], &[f32; 3], &mut [f32; 3]),
) {
    // SAFETY: forwarded to the caller's vec3_t contract
    unsafe {
        let (a, b) = (v3(a), v3(b));
        let mut o = [0.0f32; 3];
        f(&a, &b, &mut o);
        store3(out, o);
    }
}

/// `f(&mut v)`, writing the result back over `v`.
///
/// # Safety
/// `v` must point to 3 readable and writable floats.
#[inline]
unsafe fn inplace<R>(v: *mut c_float, f: impl FnOnce(&mut [f32; 3]) -> R) -> R {
    // SAFETY: forwarded to the caller's vec3_t contract
    unsafe {
        let mut a = v3(v);
        let r = f(&mut a);
        store3(v, a);
        r
    }
}

/// `f(a)` with no output vector.
///
/// # Safety
/// `a` must point to 3 readable floats.
#[inline]
unsafe fn read1<R>(a: *const c_float, f: impl FnOnce(&[f32; 3]) -> R) -> R {
    // SAFETY: forwarded to the caller's vec3_t contract
    f(&unsafe { v3(a) })
}

/// `f(a, b)` with no output vector.
///
/// # Safety
/// `a`/`b` must point to 3 readable floats.
#[inline]
unsafe fn read2<R>(
    a: *const c_float,
    b: *const c_float,
    f: impl FnOnce(&[f32; 3], &[f32; 3]) -> R,
) -> R {
    // SAFETY: forwarded to the caller's vec3_t contract
    let (a, b) = unsafe { (v3(a), v3(b)) };
    f(&a, &b)
}

/// `f(a, b, c)` with no output vector.
///
/// # Safety
/// `a`/`b`/`c` must point to 3 readable floats.
#[inline]
unsafe fn read3<R>(
    a: *const c_float,
    b: *const c_float,
    c: *const c_float,
    f: impl FnOnce(&[f32; 3], &[f32; 3], &[f32; 3]) -> R,
) -> R {
    // SAFETY: forwarded to the caller's vec3_t contract
    let (a, b, c) = unsafe { (v3(a), v3(b), v3(c)) };
    f(&a, &b, &c)
}

/// `f(&mut out)` over a C `float[16]`.
///
/// # Safety
/// `p` must point to 16 writable floats.
#[inline]
unsafe fn mat16_out(p: *mut c_float, f: impl FnOnce(&mut [f32; 16])) {
    let mut out = [0.0f32; 16];
    f(&mut out);
    // SAFETY: caller contract (float[16] parameter)
    unsafe { *(p as *mut [f32; 16]) = out };
}

/// # Safety
/// `dst` must point to 3 writable floats; `src` to 3 readable floats.
#[no_mangle]
pub unsafe extern "C" fn PerpendicularVector(dst: *mut c_float, src: *const c_float) {
    // SAFETY: vec3_t contracts per mathlib.h; see this fn's Safety section
    unsafe { map1(src, dst, |s, d| m::perpendicular_vector(d, s)) }
}

/// # Safety
/// `dst`/`dir`/`point` must each point to 3 valid floats.
#[no_mangle]
pub unsafe extern "C" fn RotatePointAroundVector(
    dst: *mut c_float,
    dir: *const c_float,
    point: *const c_float,
    degrees: c_float,
) {
    // SAFETY: vec3_t contracts per mathlib.h; see this fn's Safety section
    unsafe {
        map2(dir, point, dst, |d, p, o| {
            m::rotate_point_around_vector(o, d, p, degrees)
        })
    }
}

#[no_mangle]
pub extern "C" fn anglemod(a: c_float) -> c_float {
    m::anglemod(a)
}

/// # Safety
/// `emins`/`emaxs` must point to 3 valid floats; `p` to a valid mplane_t.
#[no_mangle]
pub unsafe extern "C" fn BoxOnPlaneSide(
    emins: *mut c_float,
    emaxs: *mut c_float,
    p: *mut MPlane,
) -> c_int {
    // SAFETY: p is a valid mplane_t per the mathlib.h contract
    let plane = unsafe { &*p };
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    let (mins, maxs) = unsafe { (v3(emins), v3(emaxs)) };
    match m::box_on_plane_side(&mins, &maxs, &plane.normal, plane.dist, plane.signbits) {
        Ok(sides) => sides,
        Err(msg) => {
            let msg_z: Vec<u8> = msg.bytes().chain([0]).collect();
            // SAFETY: NUL-terminated format + string arg; Sys_Error never returns
            unsafe { quake_c_sys::Sys_Error(c"%s".as_ptr(), msg_z.as_ptr() as *const c_char) }
        }
    }
}

/// # Safety
/// `forward` must point to 3 valid floats, `angles` to 3 writable floats;
/// `up` is optional (NULL) and points to 3 valid floats when set.
#[no_mangle]
pub unsafe extern "C" fn VectorAngles(
    forward: *const c_float,
    up: *mut c_float,
    angles: *mut c_float,
) {
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    let f = unsafe { v3(forward) };
    let u = if up.is_null() {
        None
    } else {
        // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
        Some(unsafe { v3(up) })
    };
    let mut out = [0.0f32; 3];
    m::vector_angles(&f, u.as_ref(), &mut out);
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    unsafe { store3(angles, out) };
}

/// # Safety
/// All four parameters must point to 3 valid floats.
#[no_mangle]
pub unsafe extern "C" fn AngleVectors(
    angles: *mut c_float,
    forward: *mut c_float,
    right: *mut c_float,
    up: *mut c_float,
) {
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    let a = unsafe { v3(angles) };
    let (mut f, mut r, mut u) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 3]);
    m::angle_vectors(&a, &mut f, &mut r, &mut u);
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    unsafe {
        store3(forward, f);
        store3(right, r);
        store3(up, u);
    }
}

/// # Safety
/// `v1`/`v2` must point to 3 valid floats.
#[no_mangle]
pub unsafe extern "C" fn VectorCompare(v1: *const c_float, v2: *const c_float) -> c_int {
    // SAFETY: vec3_t contracts per mathlib.h; see this fn's Safety section
    unsafe { read2(v1, v2, |a, b| m::vector_compare(a, b) as c_int) }
}

/// # Safety
/// `veca`/`vecb` must point to 3 valid floats, `vecc` to 3 writable floats.
#[no_mangle]
pub unsafe extern "C" fn VectorMA(
    veca: *const c_float,
    scale: c_float,
    vecb: *const c_float,
    vecc: *mut c_float,
) {
    // SAFETY: vec3_t contracts per mathlib.h; see this fn's Safety section
    unsafe { map2(veca, vecb, vecc, |a, b, c| m::vector_ma(a, scale, b, c)) }
}

/// # Safety
/// `v1`/`v2` must point to 3 valid floats.
#[no_mangle]
pub unsafe extern "C" fn _DotProduct(v1: *const c_float, v2: *const c_float) -> c_float {
    // SAFETY: vec3_t contracts per mathlib.h; see this fn's Safety section
    unsafe { read2(v1, v2, m::dot_product) }
}

/// # Safety
/// `veca`/`vecb` must point to 3 valid floats, `out` to 3 writable floats.
#[no_mangle]
pub unsafe extern "C" fn _VectorSubtract(
    veca: *const c_float,
    vecb: *const c_float,
    out: *mut c_float,
) {
    // SAFETY: vec3_t contracts per mathlib.h; see this fn's Safety section
    unsafe { map2(veca, vecb, out, m::vector_subtract) }
}

/// # Safety
/// `veca`/`vecb` must point to 3 valid floats, `out` to 3 writable floats.
#[no_mangle]
pub unsafe extern "C" fn _VectorAdd(veca: *const c_float, vecb: *const c_float, out: *mut c_float) {
    // SAFETY: vec3_t contracts per mathlib.h; see this fn's Safety section
    unsafe { map2(veca, vecb, out, m::vector_add) }
}

/// # Safety
/// `input` must point to 3 valid floats, `out` to 3 writable floats.
#[no_mangle]
pub unsafe extern "C" fn _VectorCopy(input: *const c_float, out: *mut c_float) {
    // SAFETY: vec3_t contracts per mathlib.h; see this fn's Safety section
    unsafe { map1(input, out, |i, o| *o = *i) }
}

/// # Safety
/// `v1`/`v2` must point to 3 valid floats, `cross` to 3 writable floats.
#[no_mangle]
pub unsafe extern "C" fn CrossProduct(v1: *const c_float, v2: *const c_float, cross: *mut c_float) {
    // SAFETY: vec3_t contracts per mathlib.h; see this fn's Safety section
    unsafe { map2(v1, v2, cross, m::cross_product) }
}

/// # Safety
/// `v` must point to 3 valid floats.
#[no_mangle]
pub unsafe extern "C" fn VectorLength(v: *const c_float) -> c_float {
    // SAFETY: vec3_t contracts per mathlib.h; see this fn's Safety section
    unsafe { read1(v, m::vector_length) }
}

/// # Safety
/// `v` must point to 3 writable floats.
#[no_mangle]
pub unsafe extern "C" fn VectorNormalize(v: *mut c_float) -> c_float {
    // SAFETY: vec3_t contracts per mathlib.h; see this fn's Safety section
    unsafe { inplace(v, m::vector_normalize) }
}

/// # Safety
/// `v` must point to 3 writable floats.
#[no_mangle]
pub unsafe extern "C" fn VectorInverse(v: *mut c_float) {
    // SAFETY: vec3_t contracts per mathlib.h; see this fn's Safety section
    unsafe { inplace(v, m::vector_inverse) }
}

/// # Safety
/// `input` must point to 3 valid floats, `out` to 3 writable floats.
#[no_mangle]
pub unsafe extern "C" fn VectorScale(input: *const c_float, scale: c_float, out: *mut c_float) {
    // SAFETY: vec3_t contracts per mathlib.h; see this fn's Safety section
    unsafe { map1(input, out, |i, o| m::vector_scale(i, scale, o)) }
}

/// # Safety
/// `in1`/`in2` must point to 3 valid rows of 3 floats; `out` to 3 writable
/// rows (must not alias the inputs, same as the C).
#[no_mangle]
pub unsafe extern "C" fn R_ConcatRotations(
    in1: *mut [c_float; 3],
    in2: *mut [c_float; 3],
    out: *mut [c_float; 3],
) {
    // SAFETY: float[3][3] parameters per the mathlib.h contract
    let (a, b) = unsafe {
        (
            *(in1 as *const [[f32; 3]; 3]),
            *(in2 as *const [[f32; 3]; 3]),
        )
    };
    let mut o = [[0.0f32; 3]; 3];
    m::r_concat_rotations(&a, &b, &mut o);
    // SAFETY: out points to a writable float[3][3]
    unsafe { *(out as *mut [[f32; 3]; 3]) = o };
}

/// # Safety
/// `in1`/`in2` must point to 3 valid rows of 4 floats; `out` to 3 writable
/// rows (must not alias the inputs, same as the C).
#[no_mangle]
pub unsafe extern "C" fn R_ConcatTransforms(
    in1: *mut [c_float; 4],
    in2: *mut [c_float; 4],
    out: *mut [c_float; 4],
) {
    // SAFETY: float[3][4] parameters per the mathlib.h contract
    let (a, b) = unsafe {
        (
            *(in1 as *const [[f32; 4]; 3]),
            *(in2 as *const [[f32; 4]; 3]),
        )
    };
    let mut o = [[0.0f32; 4]; 3];
    m::r_concat_transforms(&a, &b, &mut o);
    // SAFETY: out points to a writable float[3][4]
    unsafe { *(out as *mut [[f32; 4]; 3]) = o };
}

/// # Safety
/// `quotient`/`rem` must be valid writable pointers.
#[no_mangle]
pub unsafe extern "C" fn FloorDivMod(
    numer: c_double,
    denom: c_double,
    quotient: *mut c_int,
    rem: *mut c_int,
) {
    match m::floor_div_mod(numer, denom) {
        // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
        Ok((q, r)) => unsafe {
            *quotient = q;
            *rem = r;
        },
        // SAFETY: NUL-terminated format; Sys_Error never returns
        Err(bad) => unsafe {
            quake_c_sys::Sys_Error(c"FloorDivMod: bad denominator %f\n".as_ptr(), bad)
        },
    }
}

#[no_mangle]
pub extern "C" fn GreatestCommonDivisor(i1: c_int, i2: c_int) -> c_int {
    m::greatest_common_divisor(i1, i2)
}

#[no_mangle]
pub extern "C" fn Invert24To16(val: c_int) -> c_int {
    m::invert24to16(val)
}

/// # Safety
/// `left` must point to 16 writable floats, `right` to 16 valid floats.
#[no_mangle]
pub unsafe extern "C" fn MatrixMultiply(left: *mut c_float, right: *mut c_float) {
    // SAFETY: float[16] parameters per the mathlib.h contract
    let mut l = unsafe { *(left as *const [f32; 16]) };
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    let r = unsafe { *(right as *const [f32; 16]) };
    m::matrix_multiply(&mut l, &r);
    // SAFETY: left points to a writable float[16]
    unsafe { *(left as *mut [f32; 16]) = l };
}

/// # Safety
/// `matrix` must point to 16 writable floats.
#[no_mangle]
pub unsafe extern "C" fn RotationMatrix(
    matrix: *mut c_float,
    angle: c_float,
    x: c_float,
    y: c_float,
    z: c_float,
) {
    // SAFETY: float[16] contract per mathlib.h; see this fn's Safety section
    unsafe { mat16_out(matrix, |o| m::rotation_matrix(o, angle, x, y, z)) }
}

/// # Safety
/// `matrix` must point to 16 writable floats.
#[no_mangle]
pub unsafe extern "C" fn TranslationMatrix(
    matrix: *mut c_float,
    x: c_float,
    y: c_float,
    z: c_float,
) {
    // SAFETY: float[16] contract per mathlib.h; see this fn's Safety section
    unsafe { mat16_out(matrix, |o| m::translation_matrix(o, x, y, z)) }
}

/// # Safety
/// `matrix` must point to 16 writable floats.
#[no_mangle]
pub unsafe extern "C" fn ScaleMatrix(matrix: *mut c_float, x: c_float, y: c_float, z: c_float) {
    // SAFETY: float[16] contract per mathlib.h; see this fn's Safety section
    unsafe { mat16_out(matrix, |o| m::scale_matrix(o, x, y, z)) }
}

/// # Safety
/// `matrix` must point to 16 writable floats.
#[no_mangle]
pub unsafe extern "C" fn IdentityMatrix(matrix: *mut c_float) {
    // SAFETY: float[16] contract per mathlib.h; see this fn's Safety section
    unsafe { mat16_out(matrix, m::identity_matrix) }
}

/// # Safety
/// All parameters must point to 3 valid floats.
#[no_mangle]
pub unsafe extern "C" fn IsOriginWithinMinMax(
    origin: *const c_float,
    mins: *const c_float,
    maxs: *const c_float,
) -> bool {
    // SAFETY: vec3_t contracts per mathlib.h; see this fn's Safety section
    unsafe { read3(origin, mins, maxs, m::is_origin_within_min_max) }
}

/// # Safety
/// `angle` must point to 3 valid floats.
#[no_mangle]
pub unsafe extern "C" fn IsAxisAlignedDeg(angle: *const c_float) -> bool {
    // SAFETY: vec3_t contracts per mathlib.h; see this fn's Safety section
    unsafe { read1(angle, m::is_axis_aligned_deg) }
}
