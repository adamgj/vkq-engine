//! Differential tests: quake-math (and the quake-capi shims) vs the original
//! mathlib.c compiled as c_ref_*. All comparisons are bit-exact (to_bits) so
//! NaN propagation and negative zero are covered.

use core::ffi::{c_double, c_float, c_int};
use proptest::prelude::*;
use quake_ctest as _; // links the cc-built c_ref_* archive
use quake_math::mathlib as m;

#[repr(C)]
#[derive(Clone, Copy)]
struct CMPlane {
    normal: [f32; 3],
    dist: f32,
    type_: u8,
    signbits: u8,
    pad: [u8; 2],
}

extern "C" {
    fn c_ref_PerpendicularVector(dst: *mut c_float, src: *const c_float);
    fn c_ref_RotatePointAroundVector(
        dst: *mut c_float,
        dir: *const c_float,
        point: *const c_float,
        degrees: c_float,
    );
    fn c_ref_anglemod(a: c_float) -> c_float;
    fn c_ref_BoxOnPlaneSide(emins: *mut c_float, emaxs: *mut c_float, p: *mut CMPlane) -> c_int;
    fn c_ref_VectorAngles(forward: *const c_float, up: *mut c_float, angles: *mut c_float);
    fn c_ref_AngleVectors(
        angles: *mut c_float,
        forward: *mut c_float,
        right: *mut c_float,
        up: *mut c_float,
    );
    fn c_ref_VectorMA(
        veca: *const c_float,
        scale: c_float,
        vecb: *const c_float,
        vecc: *mut c_float,
    );
    fn c_ref_DotProduct_fn(v1: *const c_float, v2: *const c_float) -> c_float;
    fn c_ref_CrossProduct(v1: *const c_float, v2: *const c_float, cross: *mut c_float);
    fn c_ref_VectorLength(v: *const c_float) -> c_float;
    fn c_ref_VectorNormalize(v: *mut c_float) -> c_float;
    fn c_ref_R_ConcatRotations(
        in1: *mut [c_float; 3],
        in2: *mut [c_float; 3],
        out: *mut [c_float; 3],
    );
    fn c_ref_R_ConcatTransforms(
        in1: *mut [c_float; 4],
        in2: *mut [c_float; 4],
        out: *mut [c_float; 4],
    );
    fn c_ref_FloorDivMod(numer: c_double, denom: c_double, quotient: *mut c_int, rem: *mut c_int);
    fn c_ref_GreatestCommonDivisor(i1: c_int, i2: c_int) -> c_int;
    fn c_ref_Invert24To16(val: c_int) -> c_int;
    fn c_ref_MatrixMultiply(left: *mut c_float, right: *mut c_float);
    fn c_ref_RotationMatrix(
        matrix: *mut c_float,
        angle: c_float,
        x: c_float,
        y: c_float,
        z: c_float,
    );
    fn c_ref_IsOriginWithinMinMax(
        origin: *const c_float,
        mins: *const c_float,
        maxs: *const c_float,
    ) -> bool;
    fn c_ref_IsAxisAlignedDeg(angle: *const c_float) -> bool;
    static c_ref_avertexnormals: [[f32; 162]; 0]; // dummy shape, cast below
}

fn bits3(v: &[f32; 3]) -> [u32; 3] {
    [v[0].to_bits(), v[1].to_bits(), v[2].to_bits()]
}

/// Same as [`bits3`], but with the sign of a NaN lane normalized away.
///
/// COMPAT: ADR-010 (Phase 3 amendment) — a degenerate `dir` (zero length, or
/// large enough that `DotProduct (dir, dir)` overflows to infinity) makes
/// `ProjectPointOnPlane` evaluate `0 * inf`, so every lane downstream is a
/// default NaN. *Which* default NaN is not a property of the source: x86 and
/// arm hardware produce the negative "indefinite" QNaN (0xffc00000) at
/// runtime, a constant-folding compiler produces the positive one
/// (0x7fc00000). C and Rust can therefore disagree on the sign bit alone for
/// inputs no caller is allowed to pass. Both engine callers of these two
/// functions are in the FTE particle renderer (r_part_fte.c), and neither the
/// demo state-hash chain nor savegames carry particle state, so the
/// divergence is unobservable. NaN payloads and NaN-vs-number differences are
/// still compared bit-exactly.
fn bits3_nan_sign_masked(v: &[f32; 3]) -> [u32; 3] {
    v.map(|f| {
        let b = f.to_bits();
        if f.is_nan() {
            b & 0x7fff_ffff
        } else {
            b
        }
    })
}

/// finite floats in the value ranges the engine actually feeds mathlib
fn game_float() -> impl Strategy<Value = f32> {
    prop_oneof![
        (-8192.0f32..8192.0),
        (-1.0f32..1.0),
        Just(0.0f32),
        Just(-0.0f32),
        (-1e30f32..1e30),
        Just(1e-40f32), // subnormal
    ]
}

fn game_vec3() -> impl Strategy<Value = [f32; 3]> {
    [game_float(), game_float(), game_float()]
}

fn game_f16() -> impl Strategy<Value = [f32; 16]> {
    proptest::array::uniform16(game_float())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    #[test]
    fn anglemod_matches(a in prop_oneof![(-1e6f32..1e6), (-720.0f32..720.0)]) {
        // SAFETY: value-only call
        let c = unsafe { c_ref_anglemod(a) };
        prop_assert_eq!(m::anglemod(a).to_bits(), c.to_bits());
    }

    #[test]
    fn angle_vectors_matches(angles in game_vec3()) {
        let (mut cf, mut cr, mut cu) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 3]);
        let mut a = angles;
        // SAFETY: valid vec3 pointers
        unsafe { c_ref_AngleVectors(a.as_mut_ptr(), cf.as_mut_ptr(), cr.as_mut_ptr(), cu.as_mut_ptr()) };
        let (mut rf, mut rr, mut ru) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 3]);
        m::angle_vectors(&angles, &mut rf, &mut rr, &mut ru);
        prop_assert_eq!(bits3(&rf), bits3(&cf));
        prop_assert_eq!(bits3(&rr), bits3(&cr));
        prop_assert_eq!(bits3(&ru), bits3(&cu));
    }

    #[test]
    fn vector_angles_matches(forward in game_vec3(), up in game_vec3(), use_up in any::<bool>()) {
        let mut c_angles = [0.0f32; 3];
        let mut c_up = up;
        // SAFETY: valid vec3 pointers; NULL up exercises the optional path
        unsafe {
            c_ref_VectorAngles(
                forward.as_ptr(),
                if use_up { c_up.as_mut_ptr() } else { core::ptr::null_mut() },
                c_angles.as_mut_ptr(),
            )
        };
        let mut r_angles = [0.0f32; 3];
        m::vector_angles(&forward, if use_up { Some(&up) } else { None }, &mut r_angles);
        prop_assert_eq!(bits3(&r_angles), bits3(&c_angles));
    }

    #[test]
    fn box_on_plane_side_matches(
        normal in game_vec3(),
        dist in game_float(),
        signbits in 0u8..8,
        mins in game_vec3(),
        maxs in game_vec3(),
    ) {
        let mut p = CMPlane { normal, dist, type_: 3, signbits, pad: [0; 2] };
        let mut c_mins = mins;
        let mut c_maxs = maxs;
        // SAFETY: valid plane/vec3 pointers
        let c = unsafe { c_ref_BoxOnPlaneSide(c_mins.as_mut_ptr(), c_maxs.as_mut_ptr(), &mut p) };
        let r = m::box_on_plane_side(&mins, &maxs, &normal, dist, signbits);
        // sides==0 is a Sys_Error in debug C builds; the reference build is
        // compiled without _DEBUG so it returns 0 there — compare raw values
        prop_assert_eq!(r.unwrap_or(0), c);
    }

    #[test]
    fn vector_ops_match(a in game_vec3(), b in game_vec3(), s in game_float()) {
        // SAFETY: valid vec3 pointers throughout
        unsafe {
            let mut c_out = [0.0f32; 3];
            c_ref_VectorMA(a.as_ptr(), s, b.as_ptr(), c_out.as_mut_ptr());
            let mut r_out = [0.0f32; 3];
            m::vector_ma(&a, s, &b, &mut r_out);
            prop_assert_eq!(bits3(&r_out), bits3(&c_out));

            prop_assert_eq!(
                m::dot_product(&a, &b).to_bits(),
                c_ref_DotProduct_fn(a.as_ptr(), b.as_ptr()).to_bits()
            );

            let mut c_cross = [0.0f32; 3];
            c_ref_CrossProduct(a.as_ptr(), b.as_ptr(), c_cross.as_mut_ptr());
            let mut r_cross = [0.0f32; 3];
            m::cross_product(&a, &b, &mut r_cross);
            prop_assert_eq!(bits3(&r_cross), bits3(&c_cross));

            prop_assert_eq!(
                m::vector_length(&a).to_bits(),
                c_ref_VectorLength(a.as_ptr()).to_bits()
            );

            let mut c_norm = a;
            let c_len = c_ref_VectorNormalize(c_norm.as_mut_ptr());
            let mut r_norm = a;
            let r_len = m::vector_normalize(&mut r_norm);
            prop_assert_eq!(r_len.to_bits(), c_len.to_bits());
            prop_assert_eq!(bits3(&r_norm), bits3(&c_norm));
        }
    }

    #[test]
    fn perpendicular_and_rotate_match(dir in game_vec3(), point in game_vec3(), degrees in -720.0f32..720.0) {
        // SAFETY: valid vec3 pointers
        unsafe {
            let mut c_dst = [0.0f32; 3];
            c_ref_PerpendicularVector(c_dst.as_mut_ptr(), dir.as_ptr());
            let mut r_dst = [0.0f32; 3];
            m::perpendicular_vector(&mut r_dst, &dir);
            prop_assert_eq!(bits3_nan_sign_masked(&r_dst), bits3_nan_sign_masked(&c_dst));

            let mut c_rot = [0.0f32; 3];
            c_ref_RotatePointAroundVector(c_rot.as_mut_ptr(), dir.as_ptr(), point.as_ptr(), degrees);
            let mut r_rot = [0.0f32; 3];
            m::rotate_point_around_vector(&mut r_rot, &dir, &point, degrees);
            prop_assert_eq!(bits3_nan_sign_masked(&r_rot), bits3_nan_sign_masked(&c_rot));
        }
    }

    #[test]
    fn matrices_match(a in game_f16(), b in game_f16(), angle in -10.0f32..10.0, axis in game_vec3()) {
        // SAFETY: valid float[16]/float[3][x] pointers
        unsafe {
            let mut c_left = a;
            c_ref_MatrixMultiply(c_left.as_mut_ptr(), b.clone().as_mut_ptr());
            let mut r_left = a;
            m::matrix_multiply(&mut r_left, &b);
            prop_assert_eq!(c_left.map(f32::to_bits), r_left.map(f32::to_bits));

            let mut c_mat = [0.0f32; 16];
            c_ref_RotationMatrix(c_mat.as_mut_ptr(), angle, axis[0], axis[1], axis[2]);
            let mut r_mat = [0.0f32; 16];
            m::rotation_matrix(&mut r_mat, angle, axis[0], axis[1], axis[2]);
            prop_assert_eq!(c_mat.map(f32::to_bits), r_mat.map(f32::to_bits));

            let mut c1 = [[a[0], a[1], a[2]], [a[3], a[4], a[5]], [a[6], a[7], a[8]]];
            let mut c2 = [[b[0], b[1], b[2]], [b[3], b[4], b[5]], [b[6], b[7], b[8]]];
            let mut c_out = [[0.0f32; 3]; 3];
            c_ref_R_ConcatRotations(c1.as_mut_ptr(), c2.as_mut_ptr(), c_out.as_mut_ptr());
            let mut r_out = [[0.0f32; 3]; 3];
            m::r_concat_rotations(&c1, &c2, &mut r_out);
            prop_assert_eq!(format!("{c_out:?}"), format!("{r_out:?}"));

            let mut t1 = [[a[0], a[1], a[2], a[3]], [a[4], a[5], a[6], a[7]], [a[8], a[9], a[10], a[11]]];
            let mut t2 = [[b[0], b[1], b[2], b[3]], [b[4], b[5], b[6], b[7]], [b[8], b[9], b[10], b[11]]];
            let mut ct_out = [[0.0f32; 4]; 3];
            c_ref_R_ConcatTransforms(t1.as_mut_ptr(), t2.as_mut_ptr(), ct_out.as_mut_ptr());
            let mut rt_out = [[0.0f32; 4]; 3];
            m::r_concat_transforms(&t1, &t2, &mut rt_out);
            prop_assert_eq!(format!("{ct_out:?}"), format!("{rt_out:?}"));
        }
    }

    #[test]
    fn misc_match(
        numer in -1e6f64..1e6,
        denom in 1.0f64..1e4,
        i1 in 0i32..100000,
        i2 in 0i32..100000,
        val in prop_oneof![0i32..256, 513i32..16777216],
        origin in game_vec3(),
        mins in game_vec3(),
        maxs in game_vec3(),
        angle in proptest::array::uniform3(-1000.0f32..1000.0f32),
    ) {
        // FloorDivMod's contract is integral inputs; feed it floors
        let numer = numer.floor();
        let denom = denom.floor().max(1.0);
        let (mut cq, mut cr) = (0, 0);
        // SAFETY: valid out pointers
        unsafe { c_ref_FloorDivMod(numer, denom, &mut cq, &mut cr) };
        prop_assert_eq!(m::floor_div_mod(numer, denom).unwrap(), (cq, cr));

        // SAFETY: value-only calls
        unsafe {
            prop_assert_eq!(m::greatest_common_divisor(i1, i2), c_ref_GreatestCommonDivisor(i1, i2));
            prop_assert_eq!(m::invert24to16(val), c_ref_Invert24To16(val));
            prop_assert_eq!(
                m::is_origin_within_min_max(&origin, &mins, &maxs),
                c_ref_IsOriginWithinMinMax(origin.as_ptr(), mins.as_ptr(), maxs.as_ptr())
            );
            prop_assert_eq!(m::is_axis_aligned_deg(&angle), c_ref_IsAxisAlignedDeg(angle.as_ptr()));
        }
    }
}

/// The degenerate case behind the `bits3_nan_sign_masked` exception, pinned
/// deterministically rather than left to the proptest seed corpus: a
/// zero-length `dir` must produce an all-NaN result on both sides.
#[test]
fn zero_dir_is_all_nan_on_both_sides() {
    let dir = [0.0f32; 3];
    let point = [0.0f32; 3];
    // SAFETY: valid vec3 pointers
    unsafe {
        let mut c_dst = [0.0f32; 3];
        c_ref_PerpendicularVector(c_dst.as_mut_ptr(), dir.as_ptr());
        let mut r_dst = [0.0f32; 3];
        m::perpendicular_vector(&mut r_dst, &dir);
        assert!(c_dst.iter().all(|f| f.is_nan()), "C perp: {c_dst:?}");
        assert!(r_dst.iter().all(|f| f.is_nan()), "Rust perp: {r_dst:?}");
        assert_eq!(bits3_nan_sign_masked(&r_dst), bits3_nan_sign_masked(&c_dst));

        let mut c_rot = [0.0f32; 3];
        c_ref_RotatePointAroundVector(c_rot.as_mut_ptr(), dir.as_ptr(), point.as_ptr(), -420.61713);
        let mut r_rot = [0.0f32; 3];
        m::rotate_point_around_vector(&mut r_rot, &dir, &point, -420.61713);
        assert!(c_rot.iter().all(|f| f.is_nan()), "C rot: {c_rot:?}");
        assert!(r_rot.iter().all(|f| f.is_nan()), "Rust rot: {r_rot:?}");
        assert_eq!(bits3_nan_sign_masked(&r_rot), bits3_nan_sign_masked(&c_rot));
    }
}

#[test]
fn anorms_table_matches() {
    // SAFETY: the C object defines float[162][3]; reinterpret the dummy-typed
    // extern accordingly
    let c_table: &[[f32; 3]; 162] =
        unsafe { &*(core::ptr::addr_of!(c_ref_avertexnormals) as *const [[f32; 3]; 162]) };
    for (i, row) in c_table.iter().enumerate() {
        assert_eq!(
            quake_math::anorms::R_AVERTEXNORMALS[i].map(f32::to_bits),
            row.map(f32::to_bits),
            "row {i}"
        );
    }
}

#[test]
fn capi_shims_match_c() {
    let angles = [30.0f32, -120.5, 7.25];
    let (mut rf, mut rr, mut ru) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 3]);
    let (mut cf, mut cr, mut cu) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 3]);
    // SAFETY: valid vec3 pointers on both sides
    unsafe {
        let mut a = angles;
        quake_rs::mathlib::AngleVectors(
            a.as_mut_ptr(),
            rf.as_mut_ptr(),
            rr.as_mut_ptr(),
            ru.as_mut_ptr(),
        );
        let mut a = angles;
        c_ref_AngleVectors(
            a.as_mut_ptr(),
            cf.as_mut_ptr(),
            cr.as_mut_ptr(),
            cu.as_mut_ptr(),
        );
    }
    assert_eq!(bits3(&rf), bits3(&cf));
    assert_eq!(bits3(&rr), bits3(&cr));
    assert_eq!(bits3(&ru), bits3(&cu));

    // BoxOnPlaneSide through the shim, exercising the mplane_t mirror layout
    let mut plane = quake_types::MPlane {
        normal: [0.6, -0.64, 0.48],
        dist: 32.0,
        type_: 3,
        signbits: 2,
        pad: [0; 2],
    };
    let mut c_plane = CMPlane {
        normal: plane.normal,
        dist: plane.dist,
        type_: 3,
        signbits: 2,
        pad: [0; 2],
    };
    let mut mins = [-16.0f32, -16.0, -24.0];
    let mut maxs = [16.0f32, 16.0, 32.0];
    // SAFETY: valid pointers; layouts asserted in quake-types
    unsafe {
        assert_eq!(
            quake_rs::mathlib::BoxOnPlaneSide(mins.as_mut_ptr(), maxs.as_mut_ptr(), &mut plane),
            c_ref_BoxOnPlaneSide(mins.as_mut_ptr(), maxs.as_mut_ptr(), &mut c_plane)
        );
    }
}
