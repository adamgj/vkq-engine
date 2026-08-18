//! Port of `Quake/mathlib.c`, function-for-function in file order.
//!
//! // COMPAT: ADR-010 — bit-exact with the C build on the same platform.
//! Transcendentals go through the platform libm (`quake_c_sys::libm`), with
//! C's implicit float<->double conversions replicated exactly at each site:
//! a C expression like `sy = sin (angle)` with float operands is
//! `(f32)sin((f64)angle)` here, and mixed float/double arithmetic promotes
//! exactly where C promotes. Plain float arithmetic maps 1:1 to f32 ops
//! (FLT_EVAL_METHOD == 0 on every supported target, no FMA contraction in
//! either language by default).
//!
//! Casts `(int)double` are `as i32` in Rust: identical for all in-range
//! values; out-of-range/NaN inputs are UB in C (platform-dependent garbage)
//! and saturate/zero in Rust — only reachable outside the engine's value
//! domain.

use quake_c_sys::libm;

pub type Vec3 = [f32; 3];

pub const VEC3_ORIGIN: Vec3 = [0.0, 0.0, 0.0];

const ARCSECS_PER_RIGHT_ANGLE: i32 = 324000;
const ARRSECS_PER_DEGREE: f32 = 3600.0;

const M_PI: f64 = std::f64::consts::PI;
const M_PI_DIV_180: f64 = M_PI / 180.0;

pub const PITCH: usize = 0;
pub const YAW: usize = 1;
pub const ROLL: usize = 2;

#[inline]
pub fn dot_product(x: &Vec3, y: &Vec3) -> f32 {
    x[0] * y[0] + x[1] * y[1] + x[2] * y[2]
}

#[inline]
fn dot_product2(x: &Vec3, y: &Vec3) -> f32 {
    x[0] * y[0] + x[1] * y[1]
}

pub fn project_point_on_plane(dst: &mut Vec3, p: &Vec3, normal: &Vec3) {
    let inv_denom = 1.0f32 / dot_product(normal, normal);
    let d = dot_product(normal, p) * inv_denom;
    let n = [
        normal[0] * inv_denom,
        normal[1] * inv_denom,
        normal[2] * inv_denom,
    ];
    dst[0] = p[0] - d * n[0];
    dst[1] = p[1] - d * n[1];
    dst[2] = p[2] - d * n[2];
}

/// assumes `src` is normalized
pub fn perpendicular_vector(dst: &mut Vec3, src: &Vec3) {
    let mut pos = 0;
    let mut minelem = 1.0f32;

    // find the smallest magnitude axially aligned vector; C compares
    // `fabs (src[i]) < minelem` with double promotion
    for (i, &s) in src.iter().enumerate() {
        if libm::fabs(s as f64) < minelem as f64 {
            pos = i;
            minelem = libm::fabs(s as f64) as f32;
        }
    }
    let mut tempvec = [0.0f32; 3];
    tempvec[pos] = 1.0;

    project_point_on_plane(dst, &tempvec, src);
    vector_normalize(dst);
}

pub fn rotate_point_around_vector(dst: &mut Vec3, dir: &Vec3, point: &Vec3, degrees: f32) {
    let vf = *dir;
    let mut vr = [0.0f32; 3];
    perpendicular_vector(&mut vr, dir);
    let mut vu = [0.0f32; 3];
    cross_product(&vr, &vf, &mut vu);

    let m = [
        [vr[0], vu[0], vf[0]],
        [vr[1], vu[1], vf[1]],
        [vr[2], vu[2], vf[2]],
    ];

    let mut im = m;
    im[0][1] = m[1][0];
    im[0][2] = m[2][0];
    im[1][0] = m[0][1];
    im[1][2] = m[2][1];
    im[2][0] = m[0][2];
    im[2][1] = m[1][2];

    let mut zrot = [[0.0f32; 3]; 3];
    zrot[2][2] = 1.0;
    let rad = degrees as f64 * M_PI_DIV_180;
    zrot[0][0] = libm::cos(rad) as f32;
    zrot[0][1] = libm::sin(rad) as f32;
    zrot[1][0] = -libm::sin(rad) as f32;
    zrot[1][1] = libm::cos(rad) as f32;

    let mut tmpmat = [[0.0f32; 3]; 3];
    r_concat_rotations(&m, &zrot, &mut tmpmat);
    let mut rot = [[0.0f32; 3]; 3];
    r_concat_rotations(&tmpmat, &im, &mut rot);

    for i in 0..3 {
        dst[i] = rot[i][0] * point[0] + rot[i][1] * point[1] + rot[i][2] * point[2];
    }
}

pub fn anglemod(a: f32) -> f32 {
    // C: a = (360.0 / 65536) * ((int)(a * (65536 / 360.0)) & 65535);
    // float * double promotes to double before the truncating cast
    ((360.0f64 / 65536.0) * (((a as f64 * (65536.0 / 360.0)) as i32 & 65535) as f64)) as f32
}

/// Returns 1, 2, or 1 + 2. The axial fast path lives in the C
/// BOX_ON_PLANE_SIDE macro, not here. `Err` reports the debug-build
/// `Sys_Error` conditions (bad signbits / zero sides) for the shim to raise.
///
/// The C guards those with `#if defined(DEBUG) || defined(_DEBUG)`, i.e. the
/// *engine's* build type; the `engine-debug` feature mirrors it. Gating on
/// `debug_assertions` (the Rust profile) instead would diverge — Meson sets
/// `-D_DEBUG` for `debugoptimized` while building Rust in release, and
/// `sides == 0` is reachable with NaN inputs, so the mismatch is "abort"
/// vs "return 0 and keep playing".
pub fn box_on_plane_side(
    emins: &Vec3,
    emaxs: &Vec3,
    normal: &Vec3,
    dist: f32,
    signbits: u8,
) -> Result<i32, &'static str> {
    if cfg!(feature = "engine-debug") && signbits & !7 != 0 {
        return Err("BoxOnPlaneSide:  Bad signbits");
    }

    let xneg = signbits & 1 != 0;
    let yneg = (signbits >> 1) & 1 != 0;
    let zneg = (signbits >> 2) & 1 != 0;

    let pick = |neg: bool| if neg { emins } else { emaxs };
    let dist1 = normal[0] * pick(xneg)[0] + normal[1] * pick(yneg)[1] + normal[2] * pick(zneg)[2];
    let pick2 = |neg: bool| if neg { emaxs } else { emins };
    let dist2 =
        normal[0] * pick2(xneg)[0] + normal[1] * pick2(yneg)[1] + normal[2] * pick2(zneg)[2];

    let mut sides = 0;
    if dist1 >= dist {
        sides = 1;
    }
    if dist2 < dist {
        sides |= 2;
    }

    if cfg!(feature = "engine-debug") && sides == 0 {
        return Err("BoxOnPlaneSide: sides==0");
    }
    Ok(sides)
}

/// johnfitz -- the opposite of AngleVectors; Spike: optional `up` to derive
/// yaw at gimbal lock and roll.
pub fn vector_angles(forward: &Vec3, up: Option<&Vec3>, angles: &mut Vec3) {
    if forward[0] == 0.0 && forward[1] == 0.0 {
        // either vertically up or down
        if forward[2] > 0.0 {
            angles[PITCH] = -90.0;
            angles[YAW] = match up {
                Some(up) => (libm::atan2(-up[1] as f64, -up[0] as f64) / M_PI_DIV_180) as f32,
                None => 0.0,
            };
        } else {
            angles[PITCH] = 90.0;
            angles[YAW] = match up {
                Some(up) => (libm::atan2(up[1] as f64, up[0] as f64) / M_PI_DIV_180) as f32,
                None => 0.0,
            };
        }
        angles[ROLL] = 0.0;
    } else {
        // C stores the radian atan2 results into the float array first, then
        // divides in place — the intermediate f32 truncation is observable
        angles[PITCH] = (-libm::atan2(
            forward[2] as f64,
            libm::sqrt(dot_product2(forward, forward) as f64),
        )) as f32;
        angles[YAW] = libm::atan2(forward[1] as f64, forward[0] as f64) as f32;

        match up {
            Some(up) => {
                let cp = libm::cos(angles[PITCH] as f64) as f32;
                let sp = libm::sin(angles[PITCH] as f64) as f32;
                let cy = libm::cos(angles[YAW] as f64) as f32;
                let sy = libm::sin(angles[YAW] as f64) as f32;
                let tleft = [-sy, cy, 0.0f32];
                let tup = [sp * cy, sp * sy, cp];
                angles[ROLL] =
                    (-libm::atan2(dot_product(up, &tleft) as f64, dot_product(up, &tup) as f64)
                        / M_PI_DIV_180) as f32;
            }
            None => angles[ROLL] = 0.0,
        }

        angles[PITCH] = (angles[PITCH] as f64 / M_PI_DIV_180) as f32;
        angles[YAW] = (angles[YAW] as f64 / M_PI_DIV_180) as f32;
    }
}

pub fn angle_vectors(angles: &Vec3, forward: &mut Vec3, right: &mut Vec3, up: &mut Vec3) {
    let mut angle: f32;

    angle = (angles[YAW] as f64 * (M_PI * 2.0 / 360.0)) as f32;
    let sy = libm::sin(angle as f64) as f32;
    let cy = libm::cos(angle as f64) as f32;
    angle = (angles[PITCH] as f64 * (M_PI * 2.0 / 360.0)) as f32;
    let sp = libm::sin(angle as f64) as f32;
    let cp = libm::cos(angle as f64) as f32;
    angle = (angles[ROLL] as f64 * (M_PI * 2.0 / 360.0)) as f32;
    let sr = libm::sin(angle as f64) as f32;
    let cr = libm::cos(angle as f64) as f32;

    forward[0] = cp * cy;
    forward[1] = cp * sy;
    forward[2] = -sp;
    // C: -1 * sr * sp * cy + -1 * cr * -sy, grouped left-to-right; unary
    // negation of an IEEE float is exact, so -sr == -1 * sr bit-for-bit
    right[0] = -sr * sp * cy + -cr * -sy;
    right[1] = -sr * sp * sy + -cr * cy;
    right[2] = -sr * cp;
    up[0] = cr * sp * cy + -sr * -sy;
    up[1] = cr * sp * sy + -sr * cy;
    up[2] = cr * cp;
}

pub fn vector_compare(v1: &Vec3, v2: &Vec3) -> bool {
    v1[0] == v2[0] && v1[1] == v2[1] && v1[2] == v2[2]
}

pub fn vector_ma(veca: &Vec3, scale: f32, vecb: &Vec3, vecc: &mut Vec3) {
    vecc[0] = veca[0] + scale * vecb[0];
    vecc[1] = veca[1] + scale * vecb[1];
    vecc[2] = veca[2] + scale * vecb[2];
}

pub fn vector_subtract(veca: &Vec3, vecb: &Vec3, out: &mut Vec3) {
    out[0] = veca[0] - vecb[0];
    out[1] = veca[1] - vecb[1];
    out[2] = veca[2] - vecb[2];
}

pub fn vector_add(veca: &Vec3, vecb: &Vec3, out: &mut Vec3) {
    out[0] = veca[0] + vecb[0];
    out[1] = veca[1] + vecb[1];
    out[2] = veca[2] + vecb[2];
}

pub fn cross_product(v1: &Vec3, v2: &Vec3, cross: &mut Vec3) {
    cross[0] = v1[1] * v2[2] - v1[2] * v2[1];
    cross[1] = v1[2] * v2[0] - v1[0] * v2[2];
    cross[2] = v1[0] * v2[1] - v1[1] * v2[0];
}

pub fn vector_length(v: &Vec3) -> f32 {
    // C: sqrt(DotProduct(v,v)) — f32 dot, double sqrt, f32 return
    libm::sqrt(dot_product(v, v) as f64) as f32
}

pub fn vector_normalize(v: &mut Vec3) -> f32 {
    let length = libm::sqrt(dot_product(v, v) as f64) as f32;
    if length != 0.0 {
        let ilength = 1.0f32 / length;
        v[0] *= ilength;
        v[1] *= ilength;
        v[2] *= ilength;
    }
    length
}

pub fn vector_inverse(v: &mut Vec3) {
    v[0] = -v[0];
    v[1] = -v[1];
    v[2] = -v[2];
}

pub fn vector_scale(input: &Vec3, scale: f32, out: &mut Vec3) {
    out[0] = input[0] * scale;
    out[1] = input[1] * scale;
    out[2] = input[2] * scale;
}

pub fn r_concat_rotations(in1: &[[f32; 3]; 3], in2: &[[f32; 3]; 3], out: &mut [[f32; 3]; 3]) {
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = in1[i][0] * in2[0][j] + in1[i][1] * in2[1][j] + in1[i][2] * in2[2][j];
        }
    }
}

pub fn r_concat_transforms(in1: &[[f32; 4]; 3], in2: &[[f32; 4]; 3], out: &mut [[f32; 4]; 3]) {
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = in1[i][0] * in2[0][j] + in1[i][1] * in2[1][j] + in1[i][2] * in2[2][j];
        }
        out[i][3] =
            in1[i][0] * in2[0][3] + in1[i][1] * in2[1][3] + in1[i][2] * in2[2][3] + in1[i][3];
    }
}

/// Floor-based quotient and remainder. `Err` is the C `Sys_Error` path
/// (non-positive denominator), raised by the shim.
pub fn floor_div_mod(numer: f64, denom: f64) -> Result<(i32, i32), f64> {
    if denom <= 0.0 {
        return Err(denom);
    }
    let q;
    let r;
    if numer >= 0.0 {
        let x = libm::floor(numer / denom);
        q = x as i32;
        r = libm::floor(numer - (x * denom)) as i32;
    } else {
        // perform operations with positive values, and fix mod to make
        // floor-based
        let x = libm::floor(-numer / denom);
        let mut q2 = -(x as i32);
        let mut r2 = libm::floor(-numer - (x * denom)) as i32;
        if r2 != 0 {
            q2 -= 1;
            r2 = denom as i32 - r2;
        }
        q = q2;
        r = r2;
    }
    Ok((q, r))
}

pub fn greatest_common_divisor(i1: i32, i2: i32) -> i32 {
    if i1 > i2 {
        if i2 == 0 {
            return i1;
        }
        greatest_common_divisor(i2, i1 % i2)
    } else {
        if i1 == 0 {
            return i2;
        }
        greatest_common_divisor(i1, i2 % i1)
    }
}

/// Inverts an 8.24 value to a 16.16 value.
pub fn invert24to16(val: i32) -> i32 {
    if val < 256 {
        return -1; // C: 0xFFFFFFFF assigned to fixed16_t (int)
    }
    (((0x10000 as f64) * (0x1000000 as f64) / val as f64) + 0.5) as i32
}

pub fn matrix_multiply(left: &mut [f32; 16], right: &[f32; 16]) {
    let temp = *left;
    for row in 0..4 {
        for column in 0..4 {
            let mut value = 0.0f32;
            for i in 0..4 {
                value += temp[i * 4 + row] * right[column * 4 + i];
            }
            left[column * 4 + row] = value;
        }
    }
}

pub fn rotation_matrix(matrix: &mut [f32; 16], angle: f32, x: f32, y: f32, z: f32) {
    // C uses cosf/sinf (single-precision libm) here, unlike the rest of the
    // file
    let c = libm::cosf(angle);
    let s = libm::sinf(angle);

    matrix[0] = x * x * (1.0 - c) + c;
    matrix[1] = y * x * (1.0 - c) + z * s;
    matrix[2] = x * z * (1.0 - c) - y * s;
    matrix[3] = 0.0;

    matrix[4] = x * y * (1.0 - c) - z * s;
    matrix[5] = y * y * (1.0 - c) + c;
    matrix[6] = y * z * (1.0 - c) + x * s;
    matrix[7] = 0.0;

    matrix[8] = x * z * (1.0 - c) + y * s;
    matrix[9] = y * z * (1.0 - c) - x * s;
    matrix[10] = z * z * (1.0 - c) + c;
    matrix[11] = 0.0;

    matrix[12] = 0.0;
    matrix[13] = 0.0;
    matrix[14] = 0.0;
    matrix[15] = 1.0;
}

pub fn translation_matrix(matrix: &mut [f32; 16], x: f32, y: f32, z: f32) {
    *matrix = [0.0; 16];
    matrix[0] = 1.0;
    matrix[5] = 1.0;
    matrix[10] = 1.0;
    matrix[12] = x;
    matrix[13] = y;
    matrix[14] = z;
    matrix[15] = 1.0;
}

pub fn scale_matrix(matrix: &mut [f32; 16], x: f32, y: f32, z: f32) {
    *matrix = [0.0; 16];
    matrix[0] = x;
    matrix[5] = y;
    matrix[10] = z;
    matrix[15] = 1.0;
}

pub fn identity_matrix(matrix: &mut [f32; 16]) {
    *matrix = [0.0; 16];
    matrix[0] = 1.0;
    matrix[5] = 1.0;
    matrix[10] = 1.0;
    matrix[15] = 1.0;
}

pub fn is_origin_within_min_max(origin: &Vec3, mins: &Vec3, maxs: &Vec3) -> bool {
    origin[0] > mins[0]
        && origin[1] > mins[1]
        && origin[2] > mins[2]
        && origin[0] < maxs[0]
        && origin[1] < maxs[1]
        && origin[2] < maxs[2]
}

/// is angle (in degrees) within an arcsec of a multiple of 90 degrees
/// (ignoring gimbal lock)
pub fn is_axis_aligned_deg(angle: &Vec3) -> bool {
    // C: ((int)(angle[i] * 3600.f) + 1) % 324000 — single-precision multiply
    let rem = |i: usize| ((angle[i] * ARRSECS_PER_DEGREE) as i32 + 1) % ARCSECS_PER_RIGHT_ANGLE;
    rem(0) <= 2 && rem(1) <= 2 && rem(2) <= 2
}
