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

/// # Safety
/// `dst` must point to 3 writable floats; `src` to 3 readable floats.
#[no_mangle]
pub unsafe extern "C" fn PerpendicularVector(dst: *mut c_float, src: *const c_float) {
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    let s = unsafe { v3(src) };
    let mut d = [0.0f32; 3];
    m::perpendicular_vector(&mut d, &s);
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    unsafe { store3(dst, d) };
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
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    let (dir, point) = unsafe { (v3(dir), v3(point)) };
    let mut d = [0.0f32; 3];
    m::rotate_point_around_vector(&mut d, &dir, &point, degrees);
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    unsafe { store3(dst, d) };
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
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    let (a, b) = unsafe { (v3(v1), v3(v2)) };
    m::vector_compare(&a, &b) as c_int
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
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    let (a, b) = unsafe { (v3(veca), v3(vecb)) };
    let mut c = [0.0f32; 3];
    m::vector_ma(&a, scale, &b, &mut c);
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    unsafe { store3(vecc, c) };
}

/// # Safety
/// `v1`/`v2` must point to 3 valid floats.
#[no_mangle]
pub unsafe extern "C" fn _DotProduct(v1: *const c_float, v2: *const c_float) -> c_float {
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    let (a, b) = unsafe { (v3(v1), v3(v2)) };
    m::dot_product(&a, &b)
}

/// # Safety
/// `veca`/`vecb` must point to 3 valid floats, `out` to 3 writable floats.
#[no_mangle]
pub unsafe extern "C" fn _VectorSubtract(
    veca: *const c_float,
    vecb: *const c_float,
    out: *mut c_float,
) {
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    let (a, b) = unsafe { (v3(veca), v3(vecb)) };
    let mut o = [0.0f32; 3];
    m::vector_subtract(&a, &b, &mut o);
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    unsafe { store3(out, o) };
}

/// # Safety
/// `veca`/`vecb` must point to 3 valid floats, `out` to 3 writable floats.
#[no_mangle]
pub unsafe extern "C" fn _VectorAdd(veca: *const c_float, vecb: *const c_float, out: *mut c_float) {
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    let (a, b) = unsafe { (v3(veca), v3(vecb)) };
    let mut o = [0.0f32; 3];
    m::vector_add(&a, &b, &mut o);
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    unsafe { store3(out, o) };
}

/// # Safety
/// `input` must point to 3 valid floats, `out` to 3 writable floats.
#[no_mangle]
pub unsafe extern "C" fn _VectorCopy(input: *const c_float, out: *mut c_float) {
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    let i = unsafe { v3(input) };
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    unsafe { store3(out, i) };
}

/// # Safety
/// `v1`/`v2` must point to 3 valid floats, `cross` to 3 writable floats.
#[no_mangle]
pub unsafe extern "C" fn CrossProduct(v1: *const c_float, v2: *const c_float, cross: *mut c_float) {
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    let (a, b) = unsafe { (v3(v1), v3(v2)) };
    let mut c = [0.0f32; 3];
    m::cross_product(&a, &b, &mut c);
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    unsafe { store3(cross, c) };
}

/// # Safety
/// `v` must point to 3 valid floats.
#[no_mangle]
pub unsafe extern "C" fn VectorLength(v: *const c_float) -> c_float {
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    let a = unsafe { v3(v) };
    m::vector_length(&a)
}

/// # Safety
/// `v` must point to 3 writable floats.
#[no_mangle]
pub unsafe extern "C" fn VectorNormalize(v: *mut c_float) -> c_float {
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    let mut a = unsafe { v3(v) };
    let len = m::vector_normalize(&mut a);
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    unsafe { store3(v, a) };
    len
}

/// # Safety
/// `v` must point to 3 writable floats.
#[no_mangle]
pub unsafe extern "C" fn VectorInverse(v: *mut c_float) {
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    let mut a = unsafe { v3(v) };
    m::vector_inverse(&mut a);
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    unsafe { store3(v, a) };
}

/// # Safety
/// `input` must point to 3 valid floats, `out` to 3 writable floats.
#[no_mangle]
pub unsafe extern "C" fn VectorScale(input: *const c_float, scale: c_float, out: *mut c_float) {
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    let i = unsafe { v3(input) };
    let mut o = [0.0f32; 3];
    m::vector_scale(&i, scale, &mut o);
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    unsafe { store3(out, o) };
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
    let mut out = [0.0f32; 16];
    m::rotation_matrix(&mut out, angle, x, y, z);
    // SAFETY: matrix points to a writable float[16]
    unsafe { *(matrix as *mut [f32; 16]) = out };
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
    let mut out = [0.0f32; 16];
    m::translation_matrix(&mut out, x, y, z);
    // SAFETY: matrix points to a writable float[16]
    unsafe { *(matrix as *mut [f32; 16]) = out };
}

/// # Safety
/// `matrix` must point to 16 writable floats.
#[no_mangle]
pub unsafe extern "C" fn ScaleMatrix(matrix: *mut c_float, x: c_float, y: c_float, z: c_float) {
    let mut out = [0.0f32; 16];
    m::scale_matrix(&mut out, x, y, z);
    // SAFETY: matrix points to a writable float[16]
    unsafe { *(matrix as *mut [f32; 16]) = out };
}

/// # Safety
/// `matrix` must point to 16 writable floats.
#[no_mangle]
pub unsafe extern "C" fn IdentityMatrix(matrix: *mut c_float) {
    let mut out = [0.0f32; 16];
    m::identity_matrix(&mut out);
    // SAFETY: matrix points to a writable float[16]
    unsafe { *(matrix as *mut [f32; 16]) = out };
}

/// # Safety
/// All parameters must point to 3 valid floats.
#[no_mangle]
pub unsafe extern "C" fn IsOriginWithinMinMax(
    origin: *const c_float,
    mins: *const c_float,
    maxs: *const c_float,
) -> bool {
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    let (o, mi, ma) = unsafe { (v3(origin), v3(mins), v3(maxs)) };
    m::is_origin_within_min_max(&o, &mi, &ma)
}

/// # Safety
/// `angle` must point to 3 valid floats.
#[no_mangle]
pub unsafe extern "C" fn IsAxisAlignedDeg(angle: *const c_float) -> bool {
    // SAFETY: vec3_t/float-array pointer contracts per mathlib.h (see fn docs)
    let a = unsafe { v3(angle) };
    m::is_axis_aligned_deg(&a)
}
