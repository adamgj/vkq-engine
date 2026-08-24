//! MD5 fuzzer (Phase 3 M7, D11 / AC6): the MD5 mesh format is text, parsed
//! through the engine's own `COM_Parse` inside the shim, so the tokenizer is
//! not a pure quake-formats function; what *is* pure — and is where MD5
//! parity actually lives — are the numeric kernels the loader runs over the
//! parsed data: the animated-component popcount, the joint-matrix
//! build/invert/transform chain, quaternion-W reconstruction, and
//! `MD5_BakeInfluences`. The full C-via-FFI text-parse differential (with the
//! baked-influence and generated-normal upload payloads) lives in
//! `md5_differential` and the 59-model real re-release `formats_corpus` gate;
//! this target fuzzes the kernels for panics and the one recoverable
//! `BakeError`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use quake_formats::md5;

/// Pull an f32 out of a rolling cursor; wraps to a fixed value when the input
/// runs out so the kernels always get full arrays.
struct Cur<'a> {
    d: &'a [u8],
    i: usize,
}

impl Cur<'_> {
    fn f32(&mut self) -> f32 {
        let mut b = [0u8; 4];
        for x in &mut b {
            *x = self.d.get(self.i).copied().unwrap_or(0);
            self.i += 1;
        }
        f32::from_le_bytes(b)
    }
    fn u32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        for x in &mut b {
            *x = self.d.get(self.i).copied().unwrap_or(0);
            self.i += 1;
        }
        u32::from_le_bytes(b)
    }
    fn vec3(&mut self) -> [f32; 3] {
        [self.f32(), self.f32(), self.f32()]
    }
    fn vec4(&mut self) -> [f32; 4] {
        [self.f32(), self.f32(), self.f32(), self.f32()]
    }
}

fuzz_target!(|data: &[u8]| {
    let mut c = Cur { d: data, i: 0 };

    // animated-component popcount over any flag word
    let flags = c.u32();
    assert!(md5::count_animated_components(flags) <= 6);

    // joint-matrix build/invert/transform chain: must never panic
    let pos = c.vec3();
    let quat = c.vec4();
    let scale = c.vec3();
    let m = md5::gen_matrix_pos_quat4_scale(&pos, &quat, &scale);
    let inv = md5::matrix3x4_invert_simple(&m);
    let v = c.vec4();
    let _ = md5::matrix3x4_rm_transform4(&inv, &v);
    let _ = md5::reconstruct_quat_w(&c.vec3());

    // BakeInfluences over a small fuzzed vertex/weight set: exercises the
    // recoverable weight-index-out-of-bounds error and the in-place write.
    let numverts = (data.first().copied().unwrap_or(0) % 4) as usize;
    let numweights = (data.get(1).copied().unwrap_or(0) % 6) as usize;
    let wide = data.get(2).copied().unwrap_or(0) & 1 == 1;
    let vertex_size = if wide {
        md5::MD5VERT8_SIZE
    } else {
        md5::MD5VERT_SIZE
    };

    let vinfo: Vec<md5::VertInfo> = (0..numverts)
        .map(|_| md5::VertInfo {
            firstweight: c.u32() as usize,
            count: c.u32() % 10,
            st: [c.f32(), c.f32()],
        })
        .collect();
    let weight: Vec<md5::WeightInfo> = (0..numweights)
        .map(|_| md5::WeightInfo {
            joint_index: (c.u32() % 4) as usize,
            pos: c.vec4(),
        })
        .collect();
    // one identity-ish pose per possible joint the weights index
    let outposes = vec![[0.0f32; 12]; 4];
    let mut vertexes = vec![0u8; numverts * vertex_size];

    let _ = md5::bake_influences(
        &outposes,
        &mut vertexes,
        wide,
        &vinfo,
        &weight,
        numverts,
        numweights,
    );
});
