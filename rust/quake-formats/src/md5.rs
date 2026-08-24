//! MD5 (skeletal replacement model) kernels, ported from the MD5 range of
//! `Quake/model_parse.c` (Rust migration Phase 3 M5).
//!
//! Unlike the other formats in this crate, MD5 is a *text* format read
//! through the engine's own `COM_Parse` tokenizer over the thread-local
//! `com_token`. Duplicating that tokenizer here would put a second copy of a
//! `common.c` function (not in Phase 3's scope) in the binary, so the token
//! walk stays in the `quake-capi` shim and this module holds what is
//! actually compat-critical and pure: the matrix/quaternion kernels, the
//! influence baking and the normal generation.
//!
//! ADR-010: every kernel below reproduces the C's exact float/double mix and
//! operand order. `Matrix3x4_Invert_Simple` really does compute in `double`
//! and narrow once at the end, and `MD5_ComputeNormals` really does take the
//! `acos` of a float in `double`; both are load-bearing for bit-exactness.

use quake_c_sys::libm;
use quake_math::mathlib::{cross_product, dot_product, vector_normalize, vector_scale, Vec3};
use quake_util::hash_map::{hashers::hash_vec3, QHashMap};

/// `sizeof (md5vert_t)`
pub const MD5VERT_SIZE: usize = 88;
/// `sizeof (md5vert8_t)`
pub const MD5VERT8_SIZE: usize = 144;
pub const NUM_JOINT_INFLUENCES_4_WEIGHT: usize = 4;
pub const NUM_JOINT_INFLUENCES_8_WEIGHT: usize = 8;

// field offsets shared by md5vert_t and md5vert8_t
const OFS_XYZ: usize = 0;
const OFS_NORM: usize = 12;
const OFS_ST: usize = 24;
// md5vert_t (4 influences)
const V4_OFS_WEIGHTS: usize = 32;
const V4_OFS_INDICES: usize = 36;
const V4_OFS_POS_X: usize = 40;
const V4_OFS_POS_Y: usize = 56;
const V4_OFS_POS_Z: usize = 72;
// md5vert8_t (8 influences)
const V8_OFS_WEIGHTS: usize = 32;
const V8_OFS_INDICES: usize = 40;
const V8_OFS_POS_X: usize = 48;
const V8_OFS_POS_Y: usize = 80;
const V8_OFS_POS_Z: usize = 112;

fn get_f32(b: &[u8], o: usize) -> f32 {
    f32::from_ne_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn put_f32(b: &mut [u8], o: usize, v: f32) {
    b[o..o + 4].copy_from_slice(&v.to_ne_bytes());
}

fn get_vec3(b: &[u8], o: usize) -> Vec3 {
    [get_f32(b, o), get_f32(b, o + 4), get_f32(b, o + 8)]
}

fn put_vec3(b: &mut [u8], o: usize, v: &Vec3) {
    put_f32(b, o, v[0]);
    put_f32(b, o + 4, v[1]);
    put_f32(b, o + 8, v[2]);
}

/// C `MD5_CountAnimatedComponents`: population count over bits 0..5 (the C
/// loop runs `bit = 1; bit <= 32; bit <<= 1`, so bit 6 and above never
/// count).
pub fn count_animated_components(flags: u32) -> usize {
    let mut count = 0;
    let mut bit: u32 = 1;
    while bit <= 32 {
        if flags & bit != 0 {
            count += 1;
        }
        bit <<= 1;
    }
    count
}

/// C `Matrix3x4_RM_Transform4`
pub fn matrix3x4_rm_transform4(matrix: &[f32; 12], vector: &[f32; 4]) -> Vec3 {
    [
        matrix[0] * vector[0]
            + matrix[1] * vector[1]
            + matrix[2] * vector[2]
            + matrix[3] * vector[3],
        matrix[4] * vector[0]
            + matrix[5] * vector[1]
            + matrix[6] * vector[2]
            + matrix[7] * vector[3],
        matrix[8] * vector[0]
            + matrix[9] * vector[1]
            + matrix[10] * vector[2]
            + matrix[11] * vector[3],
    ]
}

/// C `GenMatrixPosQuat4Scale`
pub fn gen_matrix_pos_quat4_scale(pos: &Vec3, quat: &[f32; 4], scale: &Vec3) -> [f32; 12] {
    let x2 = quat[0] + quat[0];
    let y2 = quat[1] + quat[1];
    let z2 = quat[2] + quat[2];

    let xx = quat[0] * x2;
    let xy = quat[0] * y2;
    let xz = quat[0] * z2;
    let yy = quat[1] * y2;
    let yz = quat[1] * z2;
    let zz = quat[2] * z2;
    let xw = quat[3] * x2;
    let yw = quat[3] * y2;
    let zw = quat[3] * z2;

    let mut r = [0.0f32; 12];
    r[0] = scale[0] * (1.0 - (yy + zz));
    r[4] = scale[0] * (xy + zw);
    r[8] = scale[0] * (xz - yw);

    r[1] = scale[1] * (xy - zw);
    r[5] = scale[1] * (1.0 - (xx + zz));
    r[9] = scale[1] * (yz + xw);

    r[2] = scale[2] * (xz + yw);
    r[6] = scale[2] * (yz - xw);
    r[10] = scale[2] * (1.0 - (xx + yy));

    r[3] = pos[0];
    r[7] = pos[1];
    r[11] = pos[2];
    r
}

/// C `Matrix3x4_Invert_Simple`.
///
/// COMPAT (ADR-010): the reciprocal scale and the whole transpose are
/// computed in `double` and narrowed once, at the end. Doing the arithmetic
/// in `f32` gives different low bits.
pub fn matrix3x4_invert_simple(in1: &[f32; 12]) -> [f32; 12] {
    // we only support uniform scaling, so assume the first row is enough
    // (note the lack of sqrt here, because we're trying to undo the scaling,
    // this means multiplying by the inverse scale twice - squaring it, which
    // makes the sqrt a waste of time)
    let scale = 1.0f64
        / (in1[0] as f64 * in1[0] as f64
            + in1[1] as f64 * in1[1] as f64
            + in1[2] as f64 * in1[2] as f64);

    // invert the rotation by transposing and multiplying by the squared
    // reciprocal of the input matrix scale as described above
    let mut temp = [0.0f64; 12];
    temp[0] = in1[0] as f64 * scale;
    temp[1] = in1[4] as f64 * scale;
    temp[2] = in1[8] as f64 * scale;
    temp[4] = in1[1] as f64 * scale;
    temp[5] = in1[5] as f64 * scale;
    temp[6] = in1[9] as f64 * scale;
    temp[8] = in1[2] as f64 * scale;
    temp[9] = in1[6] as f64 * scale;
    temp[10] = in1[10] as f64 * scale;

    // invert the translate
    temp[3] = -(in1[3] as f64 * temp[0] + in1[7] as f64 * temp[1] + in1[11] as f64 * temp[2]);
    temp[7] = -(in1[3] as f64 * temp[4] + in1[7] as f64 * temp[5] + in1[11] as f64 * temp[6]);
    temp[11] = -(in1[3] as f64 * temp[8] + in1[7] as f64 * temp[9] + in1[11] as f64 * temp[10]);

    let mut out = [0.0f32; 12];
    for i in 0..12 {
        out[i] = temp[i] as f32;
    }
    out
}

/// The quaternion-w reconstruction the joint and baseframe/frame readers all
/// share: `w = -sqrtf (max (0, 1 - x*x - y*y - z*z))`. Note the C clamps to
/// zero *before* the sqrt and always takes the negative root.
pub fn reconstruct_quat_w(xyz: &Vec3) -> f32 {
    let mut w = 1.0f32 - (xyz[0] * xyz[0] + xyz[1] * xyz[1] + xyz[2] * xyz[2]);
    if w < 0.0 {
        // we have no imagination.
        w = 0.0;
    }
    -libm::sqrtf(w)
}

/// C `MD5_ResolveMappedMeshParent`: walk a mesh joint's parent chain up to
/// the nearest ancestor the anim file also has, memoised through `state`
/// (0 = unvisited, 1 = in progress, 2 = done). Returns false on a cycle or
/// an out-of-range parent, exactly like the C.
pub fn resolve_mapped_mesh_parent(
    parents: &[isize],
    numjoints: usize,
    mesh_to_anim: &[isize],
    mapped_mesh_parent: &mut [isize],
    state: &mut [u8],
    mesh_index: usize,
) -> bool {
    if state[mesh_index] == 2 {
        return true;
    }
    if state[mesh_index] == 1 {
        return false;
    }

    state[mesh_index] = 1;
    let parent = parents[mesh_index];
    if parent < 0 {
        mapped_mesh_parent[mesh_index] = -1;
    } else if parent as usize >= numjoints {
        return false;
    } else if mesh_to_anim[parent as usize] >= 0 {
        mapped_mesh_parent[mesh_index] = parent;
    } else {
        if !resolve_mapped_mesh_parent(
            parents,
            numjoints,
            mesh_to_anim,
            mapped_mesh_parent,
            state,
            parent as usize,
        ) {
            return false;
        }
        mapped_mesh_parent[mesh_index] = mapped_mesh_parent[parent as usize];
    }

    state[mesh_index] = 2;
    true
}

/// `struct md5vertinfo_s`
#[derive(Clone, Copy, Default, Debug)]
pub struct VertInfo {
    pub firstweight: usize,
    pub count: u32,
    pub st: [f32; 2],
}

/// `struct md5weightinfo_s`
#[derive(Clone, Copy, Default, Debug)]
pub struct WeightInfo {
    pub joint_index: usize,
    pub pos: [f32; 4],
}

/// What `MD5_BakeInfluences` reports back for the trailing `Con_DWarning`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BakeOutcome {
    pub max_influences: u32,
    pub scale_imprecision: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BakeError {
    /// `"%s: weight index out of bounds\n"`
    WeightIndexOutOfBounds,
}

/// C `CLAMP` (`q_minmax.h`): `(val < minval) ? minval : ((val > maxval) ?
/// maxval : val)` — NaN falls through both comparisons and is returned.
fn clamp_f(minval: f32, val: f32, maxval: f32) -> f32 {
    if val < minval {
        minval
    } else if val > maxval {
        maxval
    } else {
        val
    }
}

/// C `MD5_BakeInfluences`: fold each vertex's weight list into the fixed
/// 4- or 8-influence GPU layout, accumulating the skinned position on the
/// way.
///
/// `vertexes` is the raw `numverts * vertex_size` block the shim will hand to
/// `GLMesh_UploadBuffers`; it is written in place, memset per vertex first,
/// exactly as the C does.
pub fn bake_influences(
    outposes: &[[f32; 12]],
    vertexes: &mut [u8],
    wide: bool,
    vinfo: &[VertInfo],
    weight: &[WeightInfo],
    numverts: usize,
    numweights: usize,
) -> Result<BakeOutcome, BakeError> {
    let mut max_influences: u32 = 0;
    let mut scale_imprecision: f32 = 1.0;
    let stored_influences = if wide {
        NUM_JOINT_INFLUENCES_8_WEIGHT
    } else {
        NUM_JOINT_INFLUENCES_4_WEIGHT
    };
    let vertex_size = if wide { MD5VERT8_SIZE } else { MD5VERT_SIZE };

    for v in 0..numverts {
        let vi = vinfo[v];
        let vertex = &mut vertexes[v * vertex_size..(v + 1) * vertex_size];

        let mut weights = [0.0f32; NUM_JOINT_INFLUENCES_8_WEIGHT];
        let mut joint_indices = [0usize; NUM_JOINT_INFLUENCES_8_WEIGHT];
        let mut joint_positions = [[0.0f32; 4]; NUM_JOINT_INFLUENCES_8_WEIGHT];

        vertex.fill(0);
        put_f32(vertex, OFS_ST, vi.st[0]);
        put_f32(vertex, OFS_ST + 4, vi.st[1]);

        // COMPAT: the C is `size_t + unsigned int` with no overflow check, so
        // a `firstweight` near SIZE_MAX (it comes straight off the file via
        // strtoull) wraps rather than trapping, and the wrapped sum is what
        // decides the recoverable reject below. A checked add here would
        // abort a debug build on the very input this reject path exists for.
        if vi.firstweight.wrapping_add(vi.count as usize) > numweights {
            return Err(BakeError::WeightIndexOutOfBounds);
        }
        if max_influences < vi.count {
            max_influences = vi.count;
        }

        let mut xyz = [0.0f32; 3];
        for i in 0..vi.count as usize {
            let w = weight[vi.firstweight + i];
            let pos = matrix3x4_rm_transform4(&outposes[w.joint_index], &w.pos);
            xyz[0] += pos[0];
            xyz[1] += pos[1];
            xyz[2] += pos[2];

            // COMPAT: the C splits `i < 4` and `i < 8` into two arms with
            // identical bodies; kept as one, the behaviour is the same.
            if i < NUM_JOINT_INFLUENCES_8_WEIGHT {
                weights[i] = w.pos[3];
                joint_indices[i] = w.joint_index;
                joint_positions[i] = w.pos;
            } else {
                // obnoxious code to find the lowest of the current possible
                // joint indexes.
                let mut lowval = weights[0];
                let mut lowidx = 0usize;
                for (k, &wk) in weights.iter().enumerate().skip(1) {
                    if wk < lowval {
                        lowval = wk;
                        lowidx = k;
                    }
                }
                if weights[lowidx] < w.pos[3] {
                    // found a lower/unset weight, replace it.
                    weights[lowidx] = w.pos[3];
                    joint_indices[lowidx] = w.joint_index;
                    joint_positions[lowidx] = w.pos;
                }
            }
        }
        put_vec3(vertex, OFS_XYZ, &xyz);

        // normalize in case we dropped some weights.
        let mut scale = 0.0f32;
        for &w in weights.iter().take(stored_influences) {
            scale += w;
        }
        if scale > 0.0 {
            if scale_imprecision < scale {
                scale_imprecision = scale;
            }
            scale = 1.0 / scale;
            for (w, jp) in weights
                .iter_mut()
                .zip(joint_positions.iter_mut())
                .take(stored_influences)
            {
                *w *= scale;
                for c in jp.iter_mut() {
                    *c *= scale;
                }
            }
        } else {
            // something bad...
            weights[0] = 1.0;
            for w in weights.iter_mut().take(stored_influences).skip(1) {
                *w = 0.0;
            }
            joint_positions[0][3] = 1.0;
        }

        let (ofs_w, ofs_i, ofs_px, ofs_py, ofs_pz) = if wide {
            (
                V8_OFS_WEIGHTS,
                V8_OFS_INDICES,
                V8_OFS_POS_X,
                V8_OFS_POS_Y,
                V8_OFS_POS_Z,
            )
        } else {
            (
                V4_OFS_WEIGHTS,
                V4_OFS_INDICES,
                V4_OFS_POS_X,
                V4_OFS_POS_Y,
                V4_OFS_POS_Z,
            )
        };
        for j in 0..stored_influences {
            // COMPAT: `(byte)(clamped * 255.0f)` truncates towards zero; a
            // NaN weight is UB in the C and lands on 0 here.
            vertex[ofs_w + j] = (clamp_f(0.0, weights[j], 1.0) * 255.0f32) as u8;
            vertex[ofs_i + j] = joint_indices[j] as u8;
            put_f32(vertex, ofs_px + j * 4, joint_positions[j][0]);
            put_f32(vertex, ofs_py + j * 4, joint_positions[j][1]);
            put_f32(vertex, ofs_pz + j * 4, joint_positions[j][2]);
        }
    }

    Ok(BakeOutcome {
        max_influences,
        scale_imprecision,
    })
}

/// C `MD5_ComputeNormals`: area/angle-weighted vertex normals, accumulated
/// per *position* through the engine's own hash map so that coincident
/// vertices share a normal.
///
/// The map is `quake_util`'s port of `hash_map.c` with the header's
/// `HashVec3` hasher and byte-wise key comparison, so bucket order, insertion
/// order and the dense value array all match the C.
pub fn compute_normals(vertexes: &mut [u8], vertex_size: usize, numverts: usize, indexes: &[u16]) {
    let mut map = QHashMap::new(
        core::mem::size_of::<Vec3>() as u32,
        core::mem::size_of::<Vec3>() as u32,
        Box::new(|k: &[u8]| {
            hash_vec3(&[
                f32::from_ne_bytes([k[0], k[1], k[2], k[3]]),
                f32::from_ne_bytes([k[4], k[5], k[6], k[7]]),
                f32::from_ne_bytes([k[8], k[9], k[10], k[11]]),
            ])
        }),
        None,
    );
    map.reserve(numverts as i32);

    for v in 0..numverts {
        put_vec3(&mut vertexes[v * vertex_size..], OFS_NORM, &[0.0, 0.0, 0.0]);
    }

    // COMPAT: the C strides `t += 3` up to `numindexes` and would read one
    // or two indices past the end if `numindexes` were not a multiple of
    // three. It always is (`numindexes = numtris * 3`), so bounding the walk
    // here only removes an unreachable out-of-bounds read.
    let mut t = 0usize;
    while t + 3 <= indexes.len() {
        let xyz: [Vec3; 3] = [
            get_vec3(&vertexes[indexes[t] as usize * vertex_size..], OFS_XYZ),
            get_vec3(&vertexes[indexes[t + 1] as usize * vertex_size..], OFS_XYZ),
            get_vec3(&vertexes[indexes[t + 2] as usize * vertex_size..], OFS_XYZ),
        ];

        let mut d1 = [
            xyz[2][0] - xyz[0][0],
            xyz[2][1] - xyz[0][1],
            xyz[2][2] - xyz[0][2],
        ];
        let mut d2 = [
            xyz[1][0] - xyz[0][0],
            xyz[1][1] - xyz[0][1],
            xyz[1][2] - xyz[0][2],
        ];
        vector_normalize(&mut d1);
        vector_normalize(&mut d2);

        let mut norm: Vec3 = [0.0; 3];
        cross_product(&d1, &d2, &mut norm);
        vector_normalize(&mut norm);

        // COMPAT (ADR-010): `acos` is the double-precision libm call and the
        // float dot product is promoted to feed it, then narrowed back.
        let angle = libm::acos(dot_product(&d1, &d2) as f64) as f32;
        let scaled = norm;
        vector_scale(&scaled, angle, &mut norm);

        for v in xyz.iter() {
            let key = vec3_key(v);
            match map.lookup(&key) {
                Some(i) => {
                    let slot = map.get_value_mut(i);
                    let cur = [
                        f32::from_ne_bytes([slot[0], slot[1], slot[2], slot[3]]),
                        f32::from_ne_bytes([slot[4], slot[5], slot[6], slot[7]]),
                        f32::from_ne_bytes([slot[8], slot[9], slot[10], slot[11]]),
                    ];
                    let sum = [norm[0] + cur[0], norm[1] + cur[1], norm[2] + cur[2]];
                    slot.copy_from_slice(&vec3_key(&sum));
                }
                None => {
                    map.insert(&key, &vec3_key(&norm));
                }
            }
        }
        t += 3;
    }

    for i in 0..map.size() {
        let slot = map.get_value_mut(i);
        let mut n = [
            f32::from_ne_bytes([slot[0], slot[1], slot[2], slot[3]]),
            f32::from_ne_bytes([slot[4], slot[5], slot[6], slot[7]]),
            f32::from_ne_bytes([slot[8], slot[9], slot[10], slot[11]]),
        ];
        vector_normalize(&mut n);
        let bytes = vec3_key(&n);
        map.get_value_mut(i).copy_from_slice(&bytes);
    }

    for v in 0..numverts {
        let key = vec3_key(&get_vec3(&vertexes[v * vertex_size..], OFS_XYZ));
        if let Some(i) = map.lookup(&key) {
            let slot = map.get_value(i);
            let n = [
                f32::from_ne_bytes([slot[0], slot[1], slot[2], slot[3]]),
                f32::from_ne_bytes([slot[4], slot[5], slot[6], slot[7]]),
                f32::from_ne_bytes([slot[8], slot[9], slot[10], slot[11]]),
            ];
            put_vec3(&mut vertexes[v * vertex_size..], OFS_NORM, &n);
        }
    }
}

fn vec3_key(v: &Vec3) -> [u8; 12] {
    let mut k = [0u8; 12];
    k[0..4].copy_from_slice(&v[0].to_ne_bytes());
    k[4..8].copy_from_slice(&v[1].to_ne_bytes());
    k[8..12].copy_from_slice(&v[2].to_ne_bytes());
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animated_component_count_stops_at_bit_five() {
        assert_eq!(count_animated_components(0), 0);
        assert_eq!(count_animated_components(63), 6);
        assert_eq!(count_animated_components(0x21), 2);
        // bits above 5 are never counted: the C loop's guard is `bit <= 32`
        assert_eq!(count_animated_components(0xffff_ffff), 6);
    }

    #[test]
    fn quat_w_is_the_negative_root_and_clamps() {
        assert_eq!(reconstruct_quat_w(&[0.0, 0.0, 0.0]), -1.0);
        // over-long imaginary part clamps to zero, keeping the sign
        assert_eq!(
            reconstruct_quat_w(&[1.0, 1.0, 1.0]).to_bits(),
            (-0.0f32).to_bits()
        );
        let w = reconstruct_quat_w(&[0.5, 0.5, 0.5]);
        assert!((w - -0.5).abs() < 1e-6, "{w}");
    }

    #[test]
    fn identity_pose_round_trips_through_invert_simple() {
        let ident = gen_matrix_pos_quat4_scale(&[0.0; 3], &[0.0, 0.0, 0.0, 1.0], &[1.0; 3]);
        let inv = matrix3x4_invert_simple(&ident);
        for (a, b) in ident.iter().zip(inv.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
        let p = matrix3x4_rm_transform4(&ident, &[1.0, 2.0, 3.0, 1.0]);
        assert_eq!(p, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn translation_inverts() {
        let m = gen_matrix_pos_quat4_scale(&[3.0, -4.0, 5.0], &[0.0, 0.0, 0.0, 1.0], &[1.0; 3]);
        let inv = matrix3x4_invert_simple(&m);
        let p = matrix3x4_rm_transform4(&inv, &[3.0, -4.0, 5.0, 1.0]);
        for c in p {
            assert!(c.abs() < 1e-5, "{p:?}");
        }
    }

    #[test]
    fn parent_chain_skips_joints_the_anim_lacks() {
        // 0 -> 1 -> 2, only joints 0 and 2 exist in the anim
        let parents = [-1isize, 0, 1];
        let mesh_to_anim = [0isize, -1, 1];
        let mut mapped = [0isize; 3];
        let mut state = [0u8; 3];
        for j in 0..3 {
            assert!(resolve_mapped_mesh_parent(
                &parents,
                3,
                &mesh_to_anim,
                &mut mapped,
                &mut state,
                j
            ));
        }
        assert_eq!(mapped, [-1, 0, 0]);
    }

    #[test]
    fn parent_cycle_is_rejected() {
        let parents = [1isize, 0];
        let mesh_to_anim = [-1isize, -1];
        let mut mapped = [0isize; 2];
        let mut state = [0u8; 2];
        assert!(!resolve_mapped_mesh_parent(
            &parents,
            2,
            &mesh_to_anim,
            &mut mapped,
            &mut state,
            0
        ));
    }

    #[test]
    fn out_of_range_parent_is_rejected() {
        let parents = [7isize];
        let mesh_to_anim = [-1isize];
        let mut mapped = [0isize; 1];
        let mut state = [0u8; 1];
        assert!(!resolve_mapped_mesh_parent(
            &parents,
            1,
            &mesh_to_anim,
            &mut mapped,
            &mut state,
            0
        ));
    }

    #[test]
    fn zero_total_weight_falls_back_to_a_single_unit_influence() {
        let poses = [gen_matrix_pos_quat4_scale(
            &[0.0; 3],
            &[0.0, 0.0, 0.0, 1.0],
            &[1.0; 3],
        )];
        let vinfo = [VertInfo {
            firstweight: 0,
            count: 1,
            st: [0.25, 0.75],
        }];
        let weight = [WeightInfo {
            joint_index: 0,
            pos: [0.0, 0.0, 0.0, 0.0],
        }];
        let mut verts = vec![0u8; MD5VERT_SIZE];
        let out = bake_influences(&poses, &mut verts, false, &vinfo, &weight, 1, 1).unwrap();
        assert_eq!(out.max_influences, 1);
        assert_eq!(out.scale_imprecision, 1.0);
        // weights[0] = 1 -> 255, everything else zero
        assert_eq!(verts[V4_OFS_WEIGHTS], 255);
        assert_eq!(&verts[V4_OFS_WEIGHTS + 1..V4_OFS_WEIGHTS + 4], &[0, 0, 0]);
        // joint_positions[0][3] = 1 is not stored (only x/y/z are), so the
        // stored position stays zero
        assert_eq!(get_f32(&verts, V4_OFS_POS_X), 0.0);
        assert_eq!(get_f32(&verts, OFS_ST), 0.25);
        assert_eq!(get_f32(&verts, OFS_ST + 4), 0.75);
    }

    #[test]
    fn firstweight_overflow_takes_the_reject_path_instead_of_trapping() {
        // COMPAT regression: the C's `firstweight + count` is a wrapping
        // size_t add, and a file-controlled firstweight can reach SIZE_MAX.
        // SIZE_MAX + 5 == 4, which still exceeds numweights, so the C rejects
        // -- a checked add here would abort a debug build instead.
        let poses = [[0.0f32; 12]];
        let vinfo = [VertInfo {
            firstweight: usize::MAX,
            count: 5,
            st: [0.0, 0.0],
        }];
        let weight = [WeightInfo::default(); 3];
        let mut verts = vec![0u8; MD5VERT_SIZE];
        assert_eq!(
            bake_influences(&poses, &mut verts, false, &vinfo, &weight, 1, 3),
            Err(BakeError::WeightIndexOutOfBounds)
        );
    }

    #[test]
    fn weight_index_past_the_table_is_rejected() {
        let poses = [[0.0f32; 12]];
        let vinfo = [VertInfo {
            firstweight: 1,
            count: 2,
            st: [0.0, 0.0],
        }];
        let weight = [WeightInfo::default(); 2];
        let mut verts = vec![0u8; MD5VERT_SIZE];
        assert_eq!(
            bake_influences(&poses, &mut verts, false, &vinfo, &weight, 1, 2),
            Err(BakeError::WeightIndexOutOfBounds)
        );
    }

    #[test]
    fn coincident_positions_share_one_normal() {
        // two triangles meeting along an edge, with a duplicated vertex at
        // the same position: the duplicate must end up with the shared normal
        let mut verts = vec![0u8; 4 * MD5VERT_SIZE];
        let pos = [
            [0.0f32, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        for (i, p) in pos.iter().enumerate() {
            put_vec3(&mut verts[i * MD5VERT_SIZE..], OFS_XYZ, p);
        }
        compute_normals(&mut verts, MD5VERT_SIZE, 4, &[0, 1, 2]);
        let n2 = get_vec3(&verts[2 * MD5VERT_SIZE..], OFS_NORM);
        let n3 = get_vec3(&verts[3 * MD5VERT_SIZE..], OFS_NORM);
        assert_eq!(n2, n3);
        // the triangle lies in z = 0, so the normal is +/- Z
        assert_eq!([n2[0], n2[1]], [0.0, 0.0]);
        assert!(n2[2].abs() > 0.99, "{n2:?}");
    }

    #[test]
    fn vertices_no_triangle_touches_keep_a_zero_normal() {
        let mut verts = vec![0u8; 2 * MD5VERT_SIZE];
        put_vec3(&mut verts[0..], OFS_XYZ, &[1.0, 2.0, 3.0]);
        put_vec3(&mut verts[MD5VERT_SIZE..], OFS_XYZ, &[4.0, 5.0, 6.0]);
        put_vec3(&mut verts[MD5VERT_SIZE..], OFS_NORM, &[9.0, 9.0, 9.0]);
        compute_normals(&mut verts, MD5VERT_SIZE, 2, &[]);
        assert_eq!(get_vec3(&verts[MD5VERT_SIZE..], OFS_NORM), [0.0, 0.0, 0.0]);
    }
}
