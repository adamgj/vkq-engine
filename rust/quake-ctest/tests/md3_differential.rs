//! Differential tests: the Rust MD3 loader (quake-capi `Mod_LoadMD3Model`
//! over quake-formats::md3) vs the MD3 half of model_parse.c compiled as
//! `c_ref_*` (Phase 3 M5, AC6).
//!
//! Same shape as `sprite_differential.rs`: each side walks its own copy of
//! one synthetic .md3 image into its own `Mem_Alloc` heap. The comparison is
//! three streams:
//!
//! 1. the deep-walk snapshot of the `aliashdr_t` chain
//!    (`quake_ctest::model_hash::mdx_snapshot`),
//! 2. the recorded `GLMesh_UploadBuffers` calls **including a byte copy of
//!    the index/vertex/texcoord buffers** -- the parsed payload never lands
//!    in `aliashdr_t`, so this is where MD3 parity actually lives,
//! 3. the console log and the recorded skin-callback argument stream (which
//!    also proves the in-place `q_strtrim` of the surface name).

use core::ffi::{c_char, c_void};
use quake_ctest::fs as ctfs;
use quake_ctest::fs::Side;
use quake_types::model_mem::{QModel, MAX_QPATH, PV_QUAKE3};

use mdx_record::{
    ctest_mdxstub_reset, ctest_skindefs_count, recorded_skins, recorded_uploads, MdxSkin, Upload,
};
use model_hash::{mdx_snapshot, Snapshot};
use quake_ctest::{mdx_record, model_hash};

const MD3_HEADER_SIZE: usize = 108;
const MD3_FRAME_SIZE: usize = 56;
const MD3_SURFACE_SIZE: usize = 108;
const MD3_VERSION: i32 = 15;
const IDMD3HEADER: i32 = 0x3350_4449;
const MAXALIASFRAMES: i32 = 2048;
const MAX_SURFACES: i32 = 32;

extern "C" {
    fn c_ref_Mod_LoadMD3Model(m: *mut QModel, buffer: *const c_void);
    fn ctest_modelstub_reset(base: *const u8);
}

type LoadFn = unsafe extern "C" fn(*mut QModel, *const c_void);

fn load_fn(side: Side) -> LoadFn {
    match side {
        Side::C => c_ref_Mod_LoadMD3Model,
        Side::Rust => quake_rs::model_parse::Mod_LoadMD3Model,
    }
}

// ---------------------------------------------------------------------------
// fixtures

#[derive(Clone)]
struct Surf {
    /// written into `md3Surface_t::name` verbatim (padded with NULs)
    name: String,
    num_verts: i32,
    num_triangles: i32,
    seed: u8,
    /// overrides `md3Surface_t::ident`
    ident: Option<i32>,
    /// overrides `md3Surface_t::numFrames`
    num_frames: Option<i32>,
}

impl Surf {
    fn new(name: &str, num_verts: i32, num_triangles: i32, seed: u8) -> Self {
        Surf {
            name: name.to_string(),
            num_verts,
            num_triangles,
            seed,
            ident: None,
            num_frames: None,
        }
    }
}

#[derive(Clone)]
struct Md3 {
    version: i32,
    flags: i32,
    frame_names: Vec<String>,
    surfaces: Vec<Surf>,
    /// overrides `md3Header_t::numSurfaces`
    num_surfaces_override: Option<i32>,
    /// overrides `md3Header_t::numFrames`
    num_frames_override: Option<i32>,
}

impl Default for Md3 {
    fn default() -> Self {
        Md3 {
            version: MD3_VERSION,
            flags: 0x2a,
            frame_names: vec!["frame_zero".into(), "frame_one".into()],
            surfaces: vec![Surf::new("body", 5, 3, 7)],
            num_surfaces_override: None,
            num_frames_override: None,
        }
    }
}

fn push_i32(v: &mut Vec<u8>, x: i32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn push_f32(v: &mut Vec<u8>, x: f32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn push_name(v: &mut Vec<u8>, s: &str, cap: usize) {
    let b = s.as_bytes();
    assert!(b.len() <= cap);
    v.extend_from_slice(b);
    v.extend(std::iter::repeat_n(0u8, cap - b.len()));
}

impl Md3 {
    fn num_frames(&self) -> i32 {
        self.num_frames_override
            .unwrap_or(self.frame_names.len() as i32)
    }

    fn num_surfaces(&self) -> i32 {
        self.num_surfaces_override
            .unwrap_or(self.surfaces.len() as i32)
    }

    /// One surface block: header, then triangles, texcoords and the
    /// `numFrames * numVerts` vertex stream, with `ofsEnd` = block size.
    fn surface_block(&self, s: &Surf) -> Vec<u8> {
        let nframes = self.frame_names.len() as i32;
        let tri_bytes = (s.num_triangles.max(0) as usize) * 12;
        let st_bytes = (s.num_verts.max(0) as usize) * 8;
        let xyz_bytes = (nframes.max(0) as usize) * (s.num_verts.max(0) as usize) * 8;

        let ofs_triangles = MD3_SURFACE_SIZE as i32;
        let ofs_st = ofs_triangles + tri_bytes as i32;
        let ofs_xyz = ofs_st + st_bytes as i32;
        let ofs_end = ofs_xyz + xyz_bytes as i32;

        let mut v = Vec::new();
        push_i32(&mut v, s.ident.unwrap_or(IDMD3HEADER));
        push_name(&mut v, &s.name, 64);
        push_i32(&mut v, 0); // flags
        push_i32(&mut v, s.num_frames.unwrap_or(nframes));
        push_i32(&mut v, 0); // numShaders
        push_i32(&mut v, s.num_verts);
        push_i32(&mut v, s.num_triangles);
        push_i32(&mut v, ofs_triangles);
        push_i32(&mut v, ofs_end); // ofsShaders (unused by the loader)
        push_i32(&mut v, ofs_st);
        push_i32(&mut v, ofs_xyz);
        push_i32(&mut v, ofs_end);
        assert_eq!(v.len(), MD3_SURFACE_SIZE);

        // triangles: indices wrap into [0, numVerts) so the fixture is
        // self-consistent, with one deliberately > 0xffff to pin the
        // narrowing the C does
        for t in 0..s.num_triangles.max(0) {
            for j in 0..3 {
                let base = (t * 3 + j) % s.num_verts.max(1);
                let idx = if t == 0 && j == 2 {
                    0x0001_0000 + base
                } else {
                    base
                };
                push_i32(&mut v, idx);
            }
        }
        // texcoords
        for j in 0..s.num_verts.max(0) {
            push_f32(&mut v, s.seed as f32 * 0.125 + j as f32 * 0.0625);
            push_f32(&mut v, 1.0 - j as f32 * 0.03125);
        }
        // vertices: numFrames poses of numVerts md3XyzNormal_t
        for f in 0..nframes.max(0) {
            for j in 0..s.num_verts.max(0) {
                let k = (f * s.num_verts.max(1) + j) as i16;
                for axis in 0..3i16 {
                    let x = k.wrapping_mul(7).wrapping_add(axis * 101) - 300;
                    v.extend_from_slice(&x.to_le_bytes());
                }
                v.push(s.seed.wrapping_add(k as u8));
                v.push(s.seed.wrapping_mul(3).wrapping_add(k as u8));
            }
        }
        assert_eq!(v.len(), ofs_end as usize);
        v
    }

    fn build(&self) -> Vec<u8> {
        let nframes = self.frame_names.len() as i32;
        let ofs_frames = MD3_HEADER_SIZE as i32;
        let ofs_surfaces = ofs_frames + nframes * MD3_FRAME_SIZE as i32;

        let mut v = Vec::new();
        push_i32(&mut v, IDMD3HEADER);
        push_i32(&mut v, self.version);
        push_name(&mut v, "fixture.md3", 64);
        push_i32(&mut v, self.flags);
        push_i32(&mut v, self.num_frames());
        push_i32(&mut v, 0); // numTags
        push_i32(&mut v, self.num_surfaces());
        push_i32(&mut v, 0); // numSkins
        push_i32(&mut v, ofs_frames);
        push_i32(&mut v, ofs_surfaces); // ofsTags (unused by the loader)
        push_i32(&mut v, ofs_surfaces);
        push_i32(&mut v, 0); // ofsEnd (unused by the loader)
        assert_eq!(v.len(), MD3_HEADER_SIZE);

        for (i, name) in self.frame_names.iter().enumerate() {
            for a in 0..6 {
                push_f32(&mut v, i as f32 + a as f32);
            }
            for a in 0..3 {
                push_f32(&mut v, a as f32);
            }
            push_f32(&mut v, 32.0 + i as f32);
            push_name(&mut v, name, 16);
        }
        assert_eq!(v.len(), ofs_surfaces as usize);

        for s in &self.surfaces {
            v.extend_from_slice(&self.surface_block(s));
        }
        v
    }
}

// ---------------------------------------------------------------------------
// per-side driver

const MODEL_NAME: &str = "progs/player.md3";

pub fn new_model(name: &str) -> Box<QModel> {
    // SAFETY: qmodel_t is zero-initialized by the engine too; all-zero is a
    // valid (null-pointer, empty-name) value for every field of the mirror
    let mut m: Box<QModel> = Box::new(unsafe { core::mem::zeroed() });
    assert!(name.len() < MAX_QPATH);
    for (i, c) in name.bytes().enumerate() {
        m.name[i] = c as c_char;
    }
    m
}

struct Loaded {
    snap: Snapshot,
    con_log: Vec<String>,
    uploads: Vec<Upload>,
    skins: Vec<MdxSkin>,
    skindefs: i32,
    /// the image after the load, so the in-place `q_strtrim` is compared too
    image: Vec<u8>,
}

/// Runs one side over its own copy of `image`. Caller must hold [`ctfs::lock`].
fn load_side(side: Side, image: &[u8], skins_result: i32) -> Loaded {
    let mut data = image.to_vec();
    let base = data.as_mut_ptr();
    let mut model = new_model(MODEL_NAME);
    let m: *mut QModel = &raw mut *model;

    ctfs::clear_logs();
    // SAFETY: the recorders are plain statics guarded by the fs lock
    unsafe {
        ctest_modelstub_reset(base);
        ctest_mdxstub_reset(skins_result);
    }

    // SAFETY: `base` points at a whole .md3 image that outlives the call and
    // `m` at a live zeroed qmodel_t
    unsafe { (load_fn(side))(m, base.cast::<c_void>()) };

    // SAFETY: the model and its aliashdr_t chain are still live
    let snap = unsafe { mdx_snapshot(m, PV_QUAKE3 as usize) };
    Loaded {
        snap,
        con_log: ctfs::con_log(),
        uploads: recorded_uploads(),
        skins: recorded_skins(),
        // SAFETY: plain static counter
        skindefs: unsafe { ctest_skindefs_count() },
        image: data,
    }
}

fn compare_with(what: &str, md3: &Md3, skins_result: i32) -> Loaded {
    let image = md3.build();
    let c = load_side(Side::C, &image, skins_result);
    let r = load_side(Side::Rust, &image, skins_result);
    assert_eq!(c.con_log, r.con_log, "{what}: console log parity");
    assert_eq!(c.skins, r.skins, "{what}: skin-callback argument parity");
    assert_eq!(
        c.skindefs, r.skindefs,
        "{what}: Mod_LoadMD3SkinDefinitions call-count parity"
    );
    assert_eq!(
        c.uploads, r.uploads,
        "{what}: GLMesh_UploadBuffers argument/payload parity"
    );
    assert_eq!(
        c.image, r.image,
        "{what}: in-place file-image mutation parity (q_strtrim)"
    );
    c.snap.assert_eq(&r.snap, what);
    r
}

fn compare(what: &str, md3: &Md3) -> Loaded {
    compare_with(what, md3, 0)
}

fn field<'a>(snap: &'a Snapshot, key: &str) -> &'a str {
    let prefix = format!("{key} = ");
    snap.lines
        .iter()
        .find(|l| l.starts_with(&prefix))
        .map(|l| &l[prefix.len()..])
        .unwrap_or_else(|| panic!("no `{key}` line in snapshot"))
}

// ---------------------------------------------------------------------------
// cases

#[test]
fn single_surface() {
    let _g = ctfs::lock();
    let out = compare("single", &Md3::default());

    assert_eq!(field(&out.snap, "mdx.numsurfaces"), "1");
    assert_eq!(field(&out.snap, "mdx.surf[0].nextsurface"), "null");
    assert_eq!(field(&out.snap, "mdx.surf[0].poseverttype"), "3");
    assert_eq!(field(&out.snap, "mdx.surf[0].numverts"), "5");
    assert_eq!(field(&out.snap, "mdx.surf[0].numverts_vbo"), "5");
    assert_eq!(field(&out.snap, "mdx.surf[0].numtris"), "3");
    assert_eq!(field(&out.snap, "mdx.surf[0].numindexes"), "9");
    assert_eq!(field(&out.snap, "mdx.surf[0].numposes"), "1");
    assert_eq!(field(&out.snap, "mdx.surf[0].numframes"), "2");
    // MD3_XYZ_SCALE on all three axes, origin zeroed
    assert_eq!(
        field(&out.snap, "mdx.surf[0].scale"),
        "[0.015625, 0.015625, 0.015625]"
    );
    assert_eq!(
        field(&out.snap, "mdx.surf[0].scale_origin"),
        "[0.0, 0.0, 0.0]"
    );
    assert_eq!(field(&out.snap, "model.flags"), "42");
    assert_eq!(field(&out.snap, "model.type"), "2");
    assert_eq!(field(&out.snap, "model.extradata[3]"), "set");
    assert_eq!(field(&out.snap, "model.extradata[0]"), "null");

    // RA14: the payload copy has to be the size the fixture implies, or the
    // parity assertion above compares nothing
    assert_eq!(out.uploads.len(), 1);
    let u = &out.uploads[0];
    assert_eq!(u.index_bytes, 9 * 2);
    assert_eq!(u.vertex_bytes, 2 * 5 * 8);
    assert_eq!(u.desc_bytes, 5 * 12);
    assert_eq!(u.joint_bytes, 0);
    assert!(u.has_desc && !u.has_joints);
    assert_eq!(u.payload.len(), 18 + 80 + 60);
}

#[test]
fn multi_surface_chain() {
    let _g = ctfs::lock();
    let md3 = Md3 {
        frame_names: vec!["a".into(), "b".into(), "c".into()],
        surfaces: vec![
            Surf::new("head", 4, 2, 3),
            Surf::new("torso", 7, 5, 40),
            Surf::new("legs", 2, 1, 200),
        ],
        ..Default::default()
    };
    let out = compare("multi", &md3);

    assert_eq!(field(&out.snap, "mdx.numsurfaces"), "3");
    assert_eq!(field(&out.snap, "mdx.surf[0].nextsurface"), "#1");
    assert_eq!(field(&out.snap, "mdx.surf[1].nextsurface"), "#2");
    assert_eq!(field(&out.snap, "mdx.surf[2].nextsurface"), "null");
    // hdrsize = sizeof (aliashdr_t) - sizeof (frames) + sizeof (frames) * 3
    let stride: usize = field(&out.snap, "mdx.hdrsize").parse().unwrap();
    assert_eq!(stride, 2544 - 40 + 40 * 3);

    assert_eq!(out.uploads.len(), 3);
    assert_eq!(out.uploads[0].numverts, 4);
    assert_eq!(out.uploads[1].numverts, 7);
    assert_eq!(out.uploads[2].numverts, 2);
    assert!(out.uploads[0].has_next_surface);
    assert!(!out.uploads[2].has_next_surface);
    // bounds come from the concatenation of every surface's vertex stream
    assert_ne!(field(&out.snap, "model.mins"), "[0.0, 0.0, 0.0]");
}

#[test]
fn every_frame_is_named_after_md3_frame_zero() {
    let _g = ctfs::lock();
    let md3 = Md3 {
        frame_names: vec!["zero".into(), "one".into(), "two".into()],
        ..Default::default()
    };
    let out = compare("frame-names", &md3);
    // COMPAT: the C never advances `pinframes`, so frames 1..n get frame 0's
    // name. Both sides agree; this pins the bug so a "fix" fails loudly.
    for i in 0..3 {
        assert_eq!(
            field(&out.snap, &format!("mdx.surf[0].frame[{i}].name")),
            "\"zero\""
        );
        assert_eq!(
            field(&out.snap, &format!("mdx.surf[0].frame[{i}].firstpose")),
            &i.to_string()
        );
        assert_eq!(
            field(&out.snap, &format!("mdx.surf[0].frame[{i}].numposes")),
            "1"
        );
        assert_eq!(
            field(&out.snap, &format!("mdx.surf[0].frame[{i}].bboxmin")),
            "[0, 0, 0]"
        );
        assert_eq!(
            field(&out.snap, &format!("mdx.surf[0].frame[{i}].bboxmax")),
            "[255, 255, 255]"
        );
    }
}

#[test]
fn surface_name_is_trimmed_in_place() {
    let _g = ctfs::lock();
    let md3 = Md3 {
        surfaces: vec![Surf::new("  padded name \t ", 3, 1, 9)],
        ..Default::default()
    };
    let out = compare("trim", &md3);
    assert_eq!(out.skins.len(), 1);
    assert_eq!(out.skins[0].name, "padded name");
    assert_eq!(out.skins[0].surf_index, 0);
    assert_eq!(out.skins[0].numsurfaces, 1);
    assert_eq!(out.skins[0].numskins, 32);
    // kind 1 = the MD3 callback
    assert_eq!(out.skins[0].kind, 1);
    // the warning prints the field, not the trimmed pointer, so the leading
    // whitespace is still there
    assert_eq!(
        out.con_log,
        vec!["[warn] MD3: progs/player.md3, no skins found for surf '  padded name' (0)\n"]
    );
}

#[test]
fn skins_found_emits_no_warning() {
    let _g = ctfs::lock();
    let out = compare_with("skins-found", &Md3::default(), 4);
    assert!(out.con_log.is_empty(), "{:?}", out.con_log);
    assert_eq!(field(&out.snap, "mdx.surf[0].numskins"), "4");
}

#[test]
fn missing_skins_warn_per_surface() {
    let _g = ctfs::lock();
    let md3 = Md3 {
        surfaces: vec![Surf::new("head", 3, 1, 1), Surf::new("legs", 3, 1, 2)],
        ..Default::default()
    };
    let out = compare("no-skins", &md3);
    assert_eq!(out.con_log.len(), 2);
    assert!(out.con_log[1].contains("'legs' (1)"), "{:?}", out.con_log);
}

#[test]
fn zero_triangle_surface() {
    let _g = ctfs::lock();
    let md3 = Md3 {
        surfaces: vec![Surf::new("empty", 2, 0, 11)],
        ..Default::default()
    };
    let out = compare("zero-tris", &md3);
    assert_eq!(field(&out.snap, "mdx.surf[0].numtris"), "0");
    assert_eq!(field(&out.snap, "mdx.surf[0].numindexes"), "0");
    assert_eq!(out.uploads[0].index_bytes, 0);
}

// ---------------------------------------------------------------------------
// Sys_Error parity

fn fatal_fixture(case: &str) -> Md3 {
    match case {
        "bad-version" => Md3 {
            version: 14,
            ..Default::default()
        },
        "too-many-frames" => Md3 {
            num_frames_override: Some(MAXALIASFRAMES + 1),
            ..Default::default()
        },
        "no-surfaces" => Md3 {
            num_surfaces_override: Some(0),
            ..Default::default()
        },
        "too-many-surfaces" => Md3 {
            num_surfaces_override: Some(MAX_SURFACES + 1),
            ..Default::default()
        },
        "corrupt-surface-ident" => {
            let mut s = Surf::new("body", 3, 1, 4);
            s.ident = Some(0x4242_4242);
            Md3 {
                surfaces: vec![s],
                ..Default::default()
            }
        }
        "mismatched-framecounts" => {
            let mut s = Surf::new("body", 3, 1, 4);
            s.num_frames = Some(9);
            Md3 {
                surfaces: vec![s],
                ..Default::default()
            }
        }
        _ => panic!("unknown fatal case {case}"),
    }
}

fn assert_fatal_parity(case: &str) {
    let _g = ctfs::lock();
    let mut data = fatal_fixture(case).build();
    let base = data.as_mut_ptr();
    let mut model = new_model(MODEL_NAME);
    let m: *mut QModel = &raw mut *model;
    // SAFETY: recorder statics, under the fs lock
    unsafe {
        ctest_modelstub_reset(base);
        ctest_mdxstub_reset(0);
    }

    let c_msg = ctfs::catch_sys_error(|| {
        // SAFETY: whole image, live model; the call is expected not to return
        unsafe { c_ref_Mod_LoadMD3Model(m, base.cast::<c_void>()) };
    })
    .unwrap_or_else(|| panic!("{case}: C side must Sys_Error"));

    let rust_msg = ctfs::rust_fatal_in_child("rust_fatal_child", case, &[])
        .unwrap_or_else(|| panic!("{case}: Rust side must Sys_Error"));
    assert_eq!(c_msg, rust_msg, "Sys_Error parity for {case}");
}

#[test]
fn bad_version_fatal_parity() {
    assert_fatal_parity("bad-version");
}

#[test]
fn too_many_frames_fatal_parity() {
    assert_fatal_parity("too-many-frames");
}

#[test]
fn no_surfaces_fatal_parity() {
    assert_fatal_parity("no-surfaces");
}

#[test]
fn too_many_surfaces_fatal_parity() {
    assert_fatal_parity("too-many-surfaces");
}

#[test]
fn corrupt_surface_ident_fatal_parity() {
    assert_fatal_parity("corrupt-surface-ident");
}

#[test]
fn mismatched_framecounts_fatal_parity() {
    assert_fatal_parity("mismatched-framecounts");
}

/// Child half of [`ctfs::rust_fatal_in_child`]: runs the Rust loader with the
/// `Sys_Error` trap unarmed, so the stub prints the message and aborts.
#[test]
fn rust_fatal_child() {
    let Some(case) = ctfs::fatal_child_case() else {
        return;
    };
    let mut data = fatal_fixture(&case).build();
    let base = data.as_mut_ptr();
    let mut model = new_model(MODEL_NAME);
    let m: *mut QModel = &raw mut *model;
    // SAFETY: recorder statics; the child runs single-threaded
    unsafe {
        ctest_modelstub_reset(base);
        ctest_mdxstub_reset(0);
    }
    // SAFETY: whole image, live model; expected not to return
    unsafe { quake_rs::model_parse::Mod_LoadMD3Model(m, base.cast::<c_void>()) };
    panic!("case {case} did not Sys_Error");
}
