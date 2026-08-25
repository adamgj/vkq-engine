//! Differential tests: the Rust MD5 loader (quake-capi `Mod_LoadMD5MeshModel`
//! over quake-formats::md5) vs the MD5 half of model_parse.c compiled as
//! `c_ref_*` (Phase 3 M5, AC6).
//!
//! MD5 is a text format read through the engine's own `COM_Parse`, and the
//! companion `.md5anim` (plus the `.mdl` the model-flags hack probes) come
//! from the filesystem, so each side runs against its **own** mounted
//! searchpath over one shared temp tree -- the Phase-2 `ctfs::setup`
//! machinery -- and the C oracle uses the c_ref filesystem while Rust uses
//! the Rust one.
//!
//! Four streams are compared per case:
//!
//! 1. the loader's return value (MD5 failures are recoverable, not fatal),
//! 2. the deep-walk snapshot of the `aliashdr_t` chain (`mdx_snapshot`),
//! 3. the recorded `GLMesh_UploadBuffers` calls *including byte copies of
//!    the skinned vertex block and the joint-pose block* -- the baked
//!    influences and the generated normals exist nowhere else, so this is
//!    where MD5 parity actually lives,
//! 4. the console log and the skin-callback argument stream.

use core::ffi::{c_char, c_void, CStr};
use quake_ctest::fs as ctfs;
use quake_ctest::fs::Side;
use quake_types::model_mem::{QModel, MAX_QPATH, PV_MD5};

use mdx_record::{ctest_mdxstub_reset, recorded_skins, recorded_uploads, MdxSkin, Upload};
use model_hash::{mdx_snapshot, Snapshot};
use quake_ctest::{mdx_record, model_hash};

extern "C" {
    fn c_ref_Mod_LoadMD5MeshModel(m: *mut QModel, buffer: *const c_void) -> bool;
    fn ctest_modelstub_reset(base: *const u8);
}

type LoadFn = unsafe extern "C" fn(*mut QModel, *const c_void) -> bool;

fn load_fn(side: Side) -> LoadFn {
    match side {
        Side::C => c_ref_Mod_LoadMD5MeshModel,
        Side::Rust => quake_rs::model_parse::Mod_LoadMD5MeshModel,
    }
}

// ---------------------------------------------------------------------------
// fixtures

#[derive(Clone)]
struct Joint {
    name: String,
    parent: i32,
    pos: [f32; 3],
    quat: [f32; 3],
}

#[derive(Clone, Copy)]
struct Vert {
    st: [f32; 2],
    firstweight: usize,
    count: usize,
}

/// The weight line's numbers are kept as `f64` and written with Rust's
/// shortest round-trip form, so `strtod` on the C side sees the *double* the
/// literal denotes. That matters: the C computes `strtod(...) * pos[3]` with
/// a full-precision double on the left, which is not the same as narrowing it
/// to float first.
#[derive(Clone, Copy)]
struct Weight {
    joint: usize,
    bias: f64,
    pos: [f64; 3],
}

#[derive(Clone)]
struct Mesh {
    shader: String,
    verts: Vec<Vert>,
    tris: Vec<[usize; 3]>,
    weights: Vec<Weight>,
    /// overrides the `numverts` line
    numverts_override: Option<i64>,
    /// overrides the `numtris` line
    numtris_override: Option<i64>,
    /// overrides the `numweights` line
    numweights_override: Option<i64>,
    /// index written on the first `vert`/`tri`/`weight` line
    bad_vert_index: Option<i64>,
    bad_tri_index: Option<i64>,
    bad_weight_index: Option<i64>,
    /// joint index written on the first weight line
    bad_weight_joint: Option<i64>,
    /// vertex index written into the first triangle
    bad_tri_vertex: Option<i64>,
}

impl Mesh {
    fn new(shader: &str) -> Self {
        Mesh {
            shader: shader.to_string(),
            verts: Vec::new(),
            tris: Vec::new(),
            weights: Vec::new(),
            numverts_override: None,
            numtris_override: None,
            numweights_override: None,
            bad_vert_index: None,
            bad_tri_index: None,
            bad_weight_index: None,
            bad_weight_joint: None,
            bad_tri_vertex: None,
        }
    }
}

#[derive(Clone)]
struct AnimJointLine {
    name: String,
    parent: i32,
    flags: u32,
    offset: u32,
}

#[derive(Clone)]
struct Anim {
    version: String,
    numframes: usize,
    /// overrides the `numJoints` line (the C reads it as animjoints)
    numjoints_override: Option<usize>,
    framerate: u32,
    num_animated_components: Option<usize>,
    hierarchy: Vec<AnimJointLine>,
    /// (pos, quat-xyz) per anim joint
    baseframe: Vec<([f32; 3], [f32; 3])>,
    /// raw component streams, one per frame; the frame index is the position
    frames: Vec<Vec<f32>>,
    /// overrides the index on the first `frame` line
    bad_frame_index: Option<i64>,
    bounds: usize,
}

#[derive(Clone)]
struct Md5 {
    version: String,
    joints: Vec<Joint>,
    meshes: Vec<Mesh>,
    numjoints_override: Option<i64>,
    nummeshes_override: Option<i64>,
    /// replaces the `joints` keyword
    joints_keyword: String,
    /// text written to `<model>.md5anim`, if any
    anim: Option<Anim>,
    /// bytes written to `<model>.mdl`, if any (the model-flags hack)
    mdl: Option<Vec<u8>>,
}

fn joint(name: &str, parent: i32, pos: [f32; 3], quat: [f32; 3]) -> Joint {
    Joint {
        name: name.to_string(),
        parent,
        pos,
        quat,
    }
}

/// A one-joint, one-mesh model: a single triangle skinned rigidly.
fn basic() -> Md5 {
    let mut mesh = Mesh::new("progs/tst_skin");
    mesh.verts = vec![
        Vert {
            st: [0.0, 0.0],
            firstweight: 0,
            count: 1,
        },
        Vert {
            st: [1.0, 0.0],
            firstweight: 1,
            count: 1,
        },
        Vert {
            st: [0.0, 1.0],
            firstweight: 2,
            count: 1,
        },
    ];
    mesh.tris = vec![[0, 1, 2]];
    mesh.weights = vec![
        Weight {
            joint: 0,
            bias: 1.0,
            pos: [0.0, 0.0, 0.0],
        },
        Weight {
            joint: 0,
            bias: 1.0,
            pos: [16.0, 0.0, 0.0],
        },
        Weight {
            joint: 0,
            bias: 1.0,
            pos: [0.0, 24.0, 8.0],
        },
    ];
    Md5 {
        version: "10".into(),
        joints: vec![joint("root", -1, [1.0, 2.0, 3.0], [0.0, 0.0, 0.25])],
        meshes: vec![mesh],
        numjoints_override: None,
        nummeshes_override: None,
        joints_keyword: "joints".into(),
        anim: None,
        mdl: None,
    }
}

fn f(v: f32) -> String {
    // enough digits to round-trip an f32 through strtod
    format!("{v:.9}")
}

/// Shortest round-trip decimal for an f64 literal, so strtod recovers it
/// exactly.
fn fd(v: f64) -> String {
    let s = format!("{v}");
    if s.contains('.') || s.contains('e') {
        s
    } else {
        format!("{s}.0")
    }
}

impl Md5 {
    fn mesh_text(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("MD5Version {}\n", self.version));
        s.push_str("commandline \"fixture\"\n");
        s.push_str(&format!(
            "numJoints {}\n",
            self.numjoints_override.unwrap_or(self.joints.len() as i64)
        ));
        s.push_str(&format!(
            "numMeshes {}\n",
            self.nummeshes_override.unwrap_or(self.meshes.len() as i64)
        ));
        s.push_str(&format!("{} {{\n", self.joints_keyword));
        for j in &self.joints {
            s.push_str(&format!(
                "\t\"{}\" {} ( {} {} {} ) ( {} {} {} )\n",
                j.name,
                j.parent,
                f(j.pos[0]),
                f(j.pos[1]),
                f(j.pos[2]),
                f(j.quat[0]),
                f(j.quat[1]),
                f(j.quat[2])
            ));
        }
        s.push_str("}\n");
        for m in &self.meshes {
            s.push_str("mesh {\n");
            s.push_str(&format!("\tshader \"{}\"\n", m.shader));
            s.push_str(&format!(
                "\tnumverts {}\n",
                m.numverts_override.unwrap_or(m.verts.len() as i64)
            ));
            for (i, v) in m.verts.iter().enumerate() {
                let idx = if i == 0 {
                    m.bad_vert_index.unwrap_or(0)
                } else {
                    i as i64
                };
                s.push_str(&format!(
                    "\tvert {} ( {} {} ) {} {}\n",
                    idx,
                    f(v.st[0]),
                    f(v.st[1]),
                    v.firstweight,
                    v.count
                ));
            }
            s.push_str(&format!(
                "\tnumtris {}\n",
                m.numtris_override.unwrap_or(m.tris.len() as i64)
            ));
            for (i, t) in m.tris.iter().enumerate() {
                let idx = if i == 0 {
                    m.bad_tri_index.unwrap_or(0)
                } else {
                    i as i64
                };
                let a = if i == 0 {
                    m.bad_tri_vertex.unwrap_or(t[0] as i64)
                } else {
                    t[0] as i64
                };
                s.push_str(&format!("\ttri {} {} {} {}\n", idx, a, t[1], t[2]));
            }
            s.push_str(&format!(
                "\tnumweights {}\n",
                m.numweights_override.unwrap_or(m.weights.len() as i64)
            ));
            for (i, w) in m.weights.iter().enumerate() {
                let idx = if i == 0 {
                    m.bad_weight_index.unwrap_or(0)
                } else {
                    i as i64
                };
                let j = if i == 0 {
                    m.bad_weight_joint.unwrap_or(w.joint as i64)
                } else {
                    w.joint as i64
                };
                s.push_str(&format!(
                    "\tweight {} {} {} ( {} {} {} )\n",
                    idx,
                    j,
                    fd(w.bias),
                    fd(w.pos[0]),
                    fd(w.pos[1]),
                    fd(w.pos[2])
                ));
            }
            s.push_str("}\n");
        }
        s
    }
}

impl Anim {
    fn text(&self) -> String {
        let numjoints = self.numjoints_override.unwrap_or(self.hierarchy.len());
        let mut s = String::new();
        s.push_str(&format!("MD5Version {}\n", self.version));
        s.push_str("commandline \"fixture\"\n");
        s.push_str(&format!("numFrames {}\n", self.numframes));
        s.push_str(&format!("numJoints {}\n", numjoints));
        s.push_str(&format!("frameRate {}\n", self.framerate));
        let rawcount = self
            .num_animated_components
            .unwrap_or_else(|| self.frames.first().map(|f| f.len()).unwrap_or(0));
        s.push_str(&format!("numAnimatedComponents {}\n", rawcount));

        s.push_str("hierarchy {\n");
        for h in &self.hierarchy {
            s.push_str(&format!(
                "\t\"{}\" {} {} {}\n",
                h.name, h.parent, h.flags, h.offset
            ));
        }
        s.push_str("}\n");

        s.push_str("bounds {\n");
        for _ in 0..self.bounds {
            s.push_str("\t( -1 -1 -1 ) ( 1 1 1 )\n");
        }
        s.push_str("}\n");

        s.push_str("baseframe {\n");
        for (pos, quat) in &self.baseframe {
            s.push_str(&format!(
                "\t( {} {} {} ) ( {} {} {} )\n",
                f(pos[0]),
                f(pos[1]),
                f(pos[2]),
                f(quat[0]),
                f(quat[1]),
                f(quat[2])
            ));
        }
        s.push_str("}\n");

        for (i, frame) in self.frames.iter().enumerate() {
            let idx = if i == 0 {
                self.bad_frame_index.unwrap_or(0)
            } else {
                i as i64
            };
            s.push_str(&format!("frame {} {{\n", idx));
            for v in frame {
                s.push_str(&format!("\t{}\n", f(*v)));
            }
            s.push_str("}\n");
        }
        s
    }
}

/// A two-frame anim over one root joint, translating on X and rotating.
fn basic_anim() -> Anim {
    Anim {
        version: "10".into(),
        numframes: 2,
        numjoints_override: None,
        framerate: 24,
        num_animated_components: None,
        hierarchy: vec![AnimJointLine {
            name: "root".into(),
            parent: -1,
            // tx | ty | tz | qx | qy | qz
            flags: 63,
            offset: 0,
        }],
        baseframe: vec![([0.0, 0.0, 0.0], [0.0, 0.0, 0.0])],
        frames: vec![
            vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            vec![8.0, -4.0, 2.0, 0.125, 0.25, -0.5],
        ],
        bad_frame_index: None,
        bounds: 2,
    }
}

// ---------------------------------------------------------------------------
// per-side driver

const MODEL_NAME: &str = "progs/tst.md5mesh";
const GAMEDIR: &CStr = c"tg";

fn new_model(name: &str) -> Box<QModel> {
    // SAFETY: qmodel_t is zero-initialized by the engine too; all-zero is a
    // valid (null-pointer, empty-name) value for every field of the mirror
    let mut m: Box<QModel> = Box::new(unsafe { core::mem::zeroed() });
    assert!(name.len() < MAX_QPATH);
    for (i, c) in name.bytes().enumerate() {
        m.name[i] = c as c_char;
    }
    m
}

struct Fixture {
    root: std::path::PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("quake-ctest-md5-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("tg/progs")).unwrap();
        Fixture { root }
    }

    fn write(&self, rel: &str, bytes: &[u8]) {
        let path = self.root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct Loaded {
    ok: bool,
    snap: Snapshot,
    con_log: Vec<String>,
    uploads: Vec<Upload>,
    skins: Vec<MdxSkin>,
}

/// Runs one side over its own copy of `image`, with `root` mounted as that
/// side's only gamedir. Caller must hold [`ctfs::lock`].
fn load_side(side: Side, root: &std::path::Path, image: &[u8], skins_result: i32) -> Loaded {
    ctfs::setup(side, &[root], 0, GAMEDIR);

    // COM_LoadFile hands the loaders a NUL-terminated copy of the file
    let mut data = image.to_vec();
    data.push(0);
    let base = data.as_mut_ptr();
    let mut model = new_model(MODEL_NAME);
    let m: *mut QModel = &raw mut *model;

    ctfs::clear_logs();
    // SAFETY: the recorders are plain statics guarded by the fs lock
    unsafe {
        ctest_modelstub_reset(base);
        ctest_mdxstub_reset(skins_result);
    }

    // SAFETY: `base` points at a whole NUL-terminated .md5mesh image that
    // outlives the call, and `m` at a live zeroed qmodel_t
    let ok = unsafe { (load_fn(side))(m, base.cast::<c_void>()) };

    // SAFETY: the model and its aliashdr_t chain are still live
    let snap = unsafe { mdx_snapshot(m, PV_MD5 as usize) };
    let out = Loaded {
        ok,
        snap,
        con_log: ctfs::con_log(),
        uploads: recorded_uploads(),
        skins: recorded_skins(),
    };
    ctfs::reset(side);
    out
}

fn compare_with(what: &str, md5: &Md5, skins_result: i32) -> Loaded {
    let fx = Fixture::new(what);
    if let Some(anim) = &md5.anim {
        fx.write("tg/progs/tst.md5anim", anim.text().as_bytes());
    }
    if let Some(mdl) = &md5.mdl {
        fx.write("tg/progs/tst.mdl", mdl);
    }
    let image = md5.mesh_text().into_bytes();

    let c = load_side(Side::C, &fx.root, &image, skins_result);
    let r = load_side(Side::Rust, &fx.root, &image, skins_result);
    assert_eq!(c.ok, r.ok, "{what}: return value parity");
    assert_eq!(c.con_log, r.con_log, "{what}: console log parity");
    assert_eq!(c.skins, r.skins, "{what}: skin-callback argument parity");
    assert_eq!(
        c.uploads, r.uploads,
        "{what}: GLMesh_UploadBuffers argument/payload parity"
    );
    c.snap.assert_eq(&r.snap, what);
    r
}

fn compare(what: &str, md5: &Md5) -> Loaded {
    compare_with(what, md5, 0)
}

fn field<'a>(snap: &'a Snapshot, key: &str) -> &'a str {
    let prefix = format!("{key} = ");
    snap.lines
        .iter()
        .find(|l| l.starts_with(&prefix))
        .map(|l| &l[prefix.len()..])
        .unwrap_or_else(|| panic!("no `{key}` line in snapshot"))
}

/// An `mdl_t` header just complete enough for `MD5_HackyModelFlags`.
fn mdl_with_flags(flags: i32) -> Vec<u8> {
    let mut v = vec![0u8; 84];
    v[0..4].copy_from_slice(b"IDPO");
    v[4..8].copy_from_slice(&6i32.to_le_bytes());
    v[76..80].copy_from_slice(&flags.to_le_bytes());
    v
}

// ---------------------------------------------------------------------------
// cases

#[test]
fn single_mesh_without_an_anim() {
    let _g = ctfs::lock();
    let out = compare("basic", &basic());
    assert!(out.ok);
    assert_eq!(field(&out.snap, "mdx.numsurfaces"), "1");
    assert_eq!(field(&out.snap, "mdx.surf[0].poseverttype"), "1"); // PV_MD5
    assert_eq!(field(&out.snap, "mdx.surf[0].numverts"), "3");
    assert_eq!(field(&out.snap, "mdx.surf[0].numtris"), "1");
    assert_eq!(field(&out.snap, "mdx.surf[0].numindexes"), "3");
    assert_eq!(field(&out.snap, "mdx.surf[0].numposes"), "1");
    assert_eq!(field(&out.snap, "mdx.surf[0].numjoints"), "1");
    // no .md5anim on disk: numframes stays at the Mem_Alloc zero
    assert_eq!(field(&out.snap, "mdx.surf[0].numframes"), "0");
    assert_eq!(field(&out.snap, "mdx.surf[0].scale"), "[1.0, 1.0, 1.0]");
    assert_eq!(field(&out.snap, "model.type"), "2");
    assert_eq!(field(&out.snap, "model.synctype"), "2"); // ST_FRAMETIME
    assert_eq!(field(&out.snap, "model.extradata[1]"), "set");
    // no .mdl sibling, so the model-flags hack finds nothing
    assert_eq!(field(&out.snap, "model.flags"), "0");

    // RA14: the recorded payload must be the size the fixture implies
    assert_eq!(out.uploads.len(), 1);
    let u = &out.uploads[0];
    assert_eq!(u.index_bytes, 3 * 2);
    assert_eq!(u.vertex_bytes, 3 * 88);
    assert_eq!(u.desc_bytes, 0);
    assert_eq!(u.joint_bytes, 0);
    assert!(!u.has_desc && u.has_joints);
    assert!(u.vertex_payload().iter().any(|&b| b != 0));
}

#[test]
fn anim_drives_framegroups_and_joint_poses() {
    let _g = ctfs::lock();
    let mut md5 = basic();
    md5.anim = Some(basic_anim());
    let out = compare("anim", &md5);
    assert!(out.ok);
    assert_eq!(field(&out.snap, "mdx.surf[0].numframes"), "2");
    for i in 0..2 {
        assert_eq!(
            field(&out.snap, &format!("mdx.surf[0].frame[{i}].firstpose")),
            &i.to_string()
        );
        assert_eq!(
            field(&out.snap, &format!("mdx.surf[0].frame[{i}].numposes")),
            "1"
        );
        assert_eq!(
            field(&out.snap, &format!("mdx.surf[0].frame[{i}].interval")),
            "0.1"
        );
    }
    let u = &out.uploads[0];
    // one joint x two poses of jointpose_t
    let (joints, poses) = (1, 2);
    assert_eq!(u.joint_bytes, joints * poses * 48);
    assert!(u.joint_payload().iter().any(|&b| b != 0));
}

#[test]
fn eight_influence_vertices_switch_to_the_wide_layout() {
    let _g = ctfs::lock();
    let mut md5 = basic();
    md5.joints = (0..6)
        .map(|i| {
            joint(
                &format!("j{i}"),
                if i == 0 { -1 } else { 0 },
                [i as f32, 0.0, 0.0],
                [0.0, 0.0, 0.0],
            )
        })
        .collect();
    let mesh = &mut md5.meshes[0];
    mesh.verts = vec![Vert {
        st: [0.5, 0.25],
        firstweight: 0,
        count: 5,
    }];
    mesh.tris = vec![[0, 0, 0]];
    mesh.weights = (0..5)
        .map(|i| Weight {
            joint: i,
            bias: 0.2,
            pos: [i as f64, 1.0, 2.0],
        })
        .collect();
    let out = compare("wide", &md5);
    assert!(out.ok);
    assert_eq!(field(&out.snap, "mdx.surf[0].poseverttype"), "2"); // PV_MD5_8
    let u = &out.uploads[0];
    assert_eq!(u.vertex_bytes, 144);
}

#[test]
fn over_eight_influences_warn_and_drop_the_weakest() {
    let _g = ctfs::lock();
    let mut md5 = basic();
    md5.joints = (0..9)
        .map(|i| {
            joint(
                &format!("j{i}"),
                if i == 0 { -1 } else { 0 },
                [i as f32, 0.0, 0.0],
                [0.0, 0.0, 0.0],
            )
        })
        .collect();
    let mesh = &mut md5.meshes[0];
    mesh.verts = vec![Vert {
        st: [0.0, 0.0],
        firstweight: 0,
        count: 9,
    }];
    mesh.tris = vec![[0, 0, 0]];
    mesh.weights = (0..9)
        .map(|i| Weight {
            joint: i,
            // ascending, so the replacement loop keeps dropping the lowest
            bias: 0.05 * (i + 1) as f64,
            pos: [i as f64, 0.0, 0.0],
        })
        .collect();
    let out = compare("influences", &md5);
    assert!(out.ok);
    assert!(
        out.con_log
            .iter()
            .any(|l| l.contains("uses up to 9 influences per vertex")),
        "{:?}",
        out.con_log
    );
}

#[test]
fn multiple_meshes_chain_through_nextsurface() {
    let _g = ctfs::lock();
    let mut md5 = basic();
    let m0 = md5.meshes[0].clone();
    let mut m1 = m0.clone();
    m1.shader = "progs/tst_skin_b".into();
    m1.verts.pop();
    m1.tris = vec![[0, 1, 1]];
    m1.weights.pop();
    md5.meshes = vec![m0, m1];
    let out = compare("multi", &md5);
    assert!(out.ok);
    assert_eq!(field(&out.snap, "mdx.numsurfaces"), "2");
    assert_eq!(field(&out.snap, "mdx.surf[0].nextsurface"), "#1");
    assert_eq!(field(&out.snap, "mdx.surf[1].nextsurface"), "null");
    assert_eq!(out.uploads.len(), 2);
    assert_eq!(out.skins.len(), 2);
    assert_eq!(out.skins[0].name, "progs/tst_skin");
    assert_eq!(out.skins[1].name, "progs/tst_skin_b");
    assert_eq!(out.skins[1].surf_index, 1);
    assert_eq!(out.skins[0].numsurfaces, 2);
    // kind 0 = the MD5 callback
    assert_eq!(out.skins[0].kind, 0);
}

#[test]
fn missing_skins_warn_per_mesh() {
    let _g = ctfs::lock();
    let out = compare("no-skins", &basic());
    assert_eq!(
        out.con_log,
        vec!["[warn] MD5: progs/tst.md5mesh, no skins found for surf 'progs/tst_skin' (0)\n"]
    );
}

#[test]
fn skins_found_emits_no_warning() {
    let _g = ctfs::lock();
    let out = compare_with("skins-found", &basic(), 3);
    assert!(out.con_log.is_empty(), "{:?}", out.con_log);
    assert_eq!(field(&out.snap, "mdx.surf[0].numskins"), "3");
}

#[test]
fn model_flags_come_from_the_mdl_sibling() {
    let _g = ctfs::lock();
    let mut md5 = basic();
    md5.mdl = Some(mdl_with_flags(0x1234));
    let out = compare("hacky-flags", &md5);
    assert!(out.ok);
    assert_eq!(field(&out.snap, "model.flags"), "4660");
}

#[test]
fn a_wrong_version_mdl_sibling_contributes_no_flags() {
    let _g = ctfs::lock();
    let mut md5 = basic();
    let mut mdl = mdl_with_flags(0x1234);
    mdl[4..8].copy_from_slice(&5i32.to_le_bytes());
    md5.mdl = Some(mdl);
    let out = compare("hacky-flags-badver", &md5);
    assert_eq!(field(&out.snap, "model.flags"), "0");
}

#[test]
fn joint_hierarchy_is_remapped_when_the_anim_lacks_a_joint() {
    let _g = ctfs::lock();
    let mut md5 = basic();
    // deliberately off-axis rotations: a joint matrix whose first row is not
    // exactly unit length is what makes Matrix3x4_Invert_Simple's
    // double-precision reciprocal observable
    md5.joints = vec![
        joint("root", -1, [0.5, -1.5, 2.25], [0.3, -0.2, 0.1]),
        joint("mid", 0, [4.0, 1.0, -2.0], [-0.15, 0.35, 0.45]),
        joint("tip", 1, [8.0, -3.0, 0.5], [0.05, 0.55, -0.25]),
    ];
    md5.meshes[0].weights = vec![
        Weight {
            joint: 0,
            bias: 0.5,
            pos: [0.0, 0.0, 0.0],
        },
        Weight {
            joint: 2,
            bias: 0.5,
            pos: [1.0, 2.0, 3.0],
        },
        Weight {
            joint: 1,
            bias: 1.0,
            pos: [0.0, 1.0, 0.0],
        },
    ];
    md5.meshes[0].verts = vec![
        Vert {
            st: [0.0, 0.0],
            firstweight: 0,
            count: 2,
        },
        Vert {
            st: [1.0, 0.0],
            firstweight: 2,
            count: 1,
        },
        Vert {
            st: [0.0, 1.0],
            firstweight: 2,
            count: 1,
        },
    ];
    // the anim knows "root" and "tip" but not "mid": tip's mapped parent
    // must collapse to root
    let mut anim = basic_anim();
    anim.hierarchy = vec![
        AnimJointLine {
            name: "root".into(),
            parent: -1,
            flags: 7,
            offset: 0,
        },
        AnimJointLine {
            name: "tip".into(),
            parent: 0,
            flags: 7,
            offset: 3,
        },
    ];
    anim.baseframe = vec![
        ([0.25, -0.75, 1.5], [0.11, -0.22, 0.33]),
        ([8.0, 2.5, -1.25], [-0.4, 0.15, 0.05]),
    ];
    anim.frames = vec![
        vec![0.5, -0.25, 0.125, 0.0, 0.0, 0.0],
        vec![1.5, 2.25, 3.125, 0.4, 0.5, 0.6],
    ];
    md5.anim = Some(anim);
    let out = compare("remap", &md5);
    assert!(out.ok);
    assert_eq!(field(&out.snap, "mdx.surf[0].numjoints"), "3");
    let u = &out.uploads[0];
    assert_eq!(u.joint_bytes, 3 * 2 * 48);
}

#[test]
fn weight_positions_are_scaled_in_double() {
    let _g = ctfs::lock();
    let mut md5 = basic();
    md5.joints = vec![
        joint("root", -1, [0.5, -1.5, 2.25], [0.3, -0.2, 0.1]),
        joint("mid", 0, [4.0, 1.0, -2.0], [-0.15, 0.35, 0.45]),
    ];
    let mesh = &mut md5.meshes[0];
    // COMPAT: the C computes `strtod(...) * pos[3]` in double and narrows
    // once, so biases and coordinates that are not exactly representable are
    // the ones that tell a double multiply apart from a float one.
    // each (bias, coordinate) pair below is one where
    // `(float)(double(x) * double(bias))` and `(float)x * bias` disagree, so
    // narrowing the strtod result early is caught rather than merely
    // asserted about.
    mesh.weights = vec![
        Weight {
            joint: 0,
            bias: 0.1591,
            pos: [13.906_814_1, 36.552_723_7, -10.503_659_6],
        },
        Weight {
            joint: 1,
            bias: 0.478,
            pos: [36.552_723_7, -5.537_894_4, 18.198_213_7],
        },
        Weight {
            joint: 0,
            bias: 0.9297,
            pos: [18.198_213_7, 13.906_814_1, -5.537_894_4],
        },
    ];
    mesh.verts = vec![
        Vert {
            st: [0.1, 0.9],
            firstweight: 0,
            count: 3,
        },
        Vert {
            st: [0.4, 0.6],
            firstweight: 1,
            count: 2,
        },
        Vert {
            st: [0.7, 0.2],
            firstweight: 2,
            count: 1,
        },
    ];
    let out = compare("weight-scale", &md5);
    assert!(out.ok);
    // the skinned positions are non-trivial, so the payload really carries
    // the scaled weights rather than a run of zeros
    let v = out.uploads[0].vertex_payload();
    assert!(v[..12].iter().any(|&b| b != 0));
}

#[test]
fn zero_bias_weights_fall_back_to_a_unit_influence() {
    let _g = ctfs::lock();
    let mut md5 = basic();
    for w in &mut md5.meshes[0].weights {
        w.bias = 0.0;
    }
    let out = compare("zero-bias", &md5);
    assert!(out.ok);
    // md5vert_t::joint_weights[0] of vertex 0 is at offset 32
    assert_eq!(out.uploads[0].vertex_payload()[32], 255);
}

// ---------------------------------------------------------------------------
// recoverable-failure parity (MD5 never Sys_Errors: it warns and returns
// false so the caller can fall back to the .mdl)

fn reject_fixture(case: &str) -> Md5 {
    let mut md5 = basic();
    match case {
        "bad-version" => md5.version = "9".into(),
        "no-joints" => {
            md5.joints.clear();
            md5.numjoints_override = Some(0);
        }
        "no-meshes" => md5.nummeshes_override = Some(0),
        "expected-joints" => md5.joints_keyword = "jointz".into(),
        "joint-parent-oob" => md5.joints[0].parent = 4,
        "vert-index-oob" => md5.meshes[0].bad_vert_index = Some(99),
        "tri-index-oob" => md5.meshes[0].bad_tri_index = Some(99),
        "tri-vertex-oob" => md5.meshes[0].bad_tri_vertex = Some(99),
        "weight-index-oob" => md5.meshes[0].bad_weight_index = Some(99),
        "weight-joint-oob" => md5.meshes[0].bad_weight_joint = Some(99),
        "bake-weight-oob" => {
            // a vertex whose weight range runs past numweights
            md5.meshes[0].verts[0].count = 4;
        }
        "bake-firstweight-wraps" => {
            // firstweight + count overflows size_t: the C wraps (SIZE_MAX + 5
            // == 4) and the *wrapped* sum still exceeds numweights, so it
            // takes the same recoverable reject. A checked add on the Rust
            // side would abort a debug build here instead.
            md5.meshes[0].verts[0].firstweight = usize::MAX;
            md5.meshes[0].verts[0].count = 5;
        }
        "anim-no-poses" => {
            let mut a = basic_anim();
            a.numframes = 0;
            a.frames.clear();
            md5.anim = Some(a);
        }
        "anim-bad-parent-order" => {
            let mut a = basic_anim();
            a.hierarchy[0].parent = 0;
            md5.anim = Some(a);
        }
        "anim-unsupported-flags" => {
            let mut a = basic_anim();
            a.hierarchy[0].flags = 64;
            md5.anim = Some(a);
        }
        "anim-bad-offset" => {
            let mut a = basic_anim();
            a.hierarchy[0].offset = 4;
            md5.anim = Some(a);
        }
        "anim-duplicate-joint" => {
            let mut a = basic_anim();
            a.hierarchy = vec![
                AnimJointLine {
                    name: "root".into(),
                    parent: -1,
                    flags: 7,
                    offset: 0,
                },
                AnimJointLine {
                    name: "root".into(),
                    parent: 0,
                    flags: 7,
                    offset: 3,
                },
            ];
            a.baseframe = vec![
                ([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
                ([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
            ];
            a.frames = vec![vec![0.0; 6], vec![0.0; 6]];
            md5.anim = Some(a);
        }
        "anim-bad-pose-index" => {
            let mut a = basic_anim();
            a.bad_frame_index = Some(7);
            md5.anim = Some(a);
        }
        "anim-bad-version" => {
            let mut a = basic_anim();
            a.version = "9".into();
            md5.anim = Some(a);
        }
        _ => panic!("unknown reject case {case}"),
    }
    md5
}

fn assert_reject_parity(case: &str, expect_ok: bool) {
    let _g = ctfs::lock();
    let out = compare(case, &reject_fixture(case));
    assert_eq!(out.ok, expect_ok, "{case}: expected ok={expect_ok}");
    assert!(!out.con_log.is_empty(), "{case}: expected a warning");
}

#[test]
fn mesh_level_rejects_match() {
    for case in [
        "bad-version",
        "no-joints",
        "no-meshes",
        "expected-joints",
        "joint-parent-oob",
        "vert-index-oob",
        "tri-index-oob",
        "tri-vertex-oob",
        "weight-index-oob",
        "weight-joint-oob",
        "bake-weight-oob",
        "bake-firstweight-wraps",
    ] {
        assert_reject_parity(case, false);
    }
}

/// `numAnimatedComponents` is `strtoull` off the file with no upper bound, and
/// the C sizes its scratch with `rawcount + 6` -- size_t wraparound, not a
/// trap. With no `frame` blocks the wrapped-small buffer is never indexed, so
/// both sides load the model successfully off the baseframe alone.
#[test]
fn anim_animated_component_count_wraps() {
    let _g = ctfs::lock();
    let mut md5 = basic();
    let mut a = basic_anim();
    a.num_animated_components = Some(usize::MAX);
    a.numframes = 1;
    a.frames.clear();
    md5.anim = Some(a);

    let out = compare("anim-rawcount-wraps", &md5);
    assert!(out.ok, "expected the load to succeed");
    // the only warning is the fixture's stubbed-out skin search; nothing
    // complained about the anim
    assert_eq!(
        out.con_log,
        vec!["[warn] MD5: progs/tst.md5mesh, no skins found for surf 'progs/tst_skin' (0)\n"]
    );
    // one pose, straight from the baseframe
    assert_eq!(field(&out.snap, "mdx.surf[0].numframes"), "1");
}

#[test]
fn anim_level_rejects_match() {
    for case in [
        "anim-no-poses",
        "anim-bad-parent-order",
        "anim-unsupported-flags",
        "anim-bad-offset",
        "anim-duplicate-joint",
        "anim-bad-pose-index",
        "anim-bad-version",
    ] {
        assert_reject_parity(case, false);
    }
}

// ---------------------------------------------------------------------------
// real-asset corpus (env-gated, ADR-019: hashes and comparisons only -- no
// game data is committed or copied)

/// Names of every `.md5mesh` in a PACK file, read without mounting it.
fn pak_md5mesh_names(path: &std::path::Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    if bytes.len() < 12 || &bytes[..4] != b"PACK" {
        return Vec::new();
    }
    let ofs = i32::from_le_bytes(bytes[4..8].try_into().unwrap()).max(0) as usize;
    let len = i32::from_le_bytes(bytes[8..12].try_into().unwrap()).max(0) as usize;
    if ofs + len > bytes.len() {
        return Vec::new();
    }
    bytes[ofs..ofs + len]
        .chunks_exact(64)
        .filter_map(|e| {
            let end = e[..56].iter().position(|&b| b == 0).unwrap_or(56);
            let name = String::from_utf8_lossy(&e[..end]).into_owned();
            name.ends_with(".md5mesh").then_some(name)
        })
        .collect()
}

/// Loads one real `.md5mesh` through one side, with that side's filesystem
/// already mounted (so the loader's own `COM_LoadFile` of the `.md5anim` and
/// the `.mdl` model-flags probe resolve through it).
fn load_real(side: Side, name: &str) -> Option<Loaded> {
    let cname = std::ffi::CString::new(name).unwrap();
    let (mut bytes, ..) = ctfs::load_file(side, &cname)?;
    bytes.push(0);
    let base = bytes.as_mut_ptr();
    let mut model = new_model(name);
    let m: *mut QModel = &raw mut *model;

    ctfs::clear_logs();
    // SAFETY: the recorders are plain statics guarded by the fs lock
    unsafe {
        ctest_modelstub_reset(base);
        ctest_mdxstub_reset(0);
    }
    // SAFETY: whole NUL-terminated image, live zeroed qmodel_t
    let ok = unsafe { (load_fn(side))(m, base.cast::<c_void>()) };
    // SAFETY: the model and its aliashdr_t chain are still live
    let snap = unsafe { mdx_snapshot(m, PV_MD5 as usize) };
    Some(Loaded {
        ok,
        snap,
        con_log: ctfs::con_log(),
        uploads: recorded_uploads(),
        skins: recorded_skins(),
    })
}

/// Runs every real `.md5mesh` in `$QUAKE_GAME_DATA/rerelease/id1/pak0.pak`
/// through both loaders and compares. Skipped when the env var is unset --
/// the assets are not redistributable (ADR-019, RA9), so CI without game
/// data still relies on the synthetic cases above.
#[test]
fn real_rerelease_md5_corpus_parity() {
    let Ok(data) = std::env::var("QUAKE_GAME_DATA") else {
        eprintln!("QUAKE_GAME_DATA unset: skipping the real MD5 corpus");
        return;
    };
    // mount `<depot>/rerelease/id1` as a gamedir: COM_ResetGameDirectories
    // filters a literal "id1" (the engine mounts that one itself, from
    // COM_InitFilesystem, which this harness does not run), so the depot root
    // is the base dir and the relative path is the gamedir name
    let root = std::path::Path::new(&data).to_path_buf();
    let names = pak_md5mesh_names(&root.join("rerelease/id1/pak0.pak"));
    if names.is_empty() {
        eprintln!("no .md5mesh entries under {}: skipping", root.display());
        return;
    }

    let _g = ctfs::lock();
    let mut compared = 0usize;
    const GAME: &CStr = c"rerelease/id1";

    for name in &names {
        ctfs::setup(Side::C, &[&root], 0, GAME);
        let c = load_real(Side::C, name);
        ctfs::reset(Side::C);

        ctfs::setup(Side::Rust, &[&root], 0, GAME);
        let r = load_real(Side::Rust, name);
        ctfs::reset(Side::Rust);

        let (Some(c), Some(r)) = (c, r) else {
            panic!("{name}: COM_LoadFile miss on one side");
        };
        assert_eq!(c.ok, r.ok, "{name}: return value parity");
        assert_eq!(c.con_log, r.con_log, "{name}: console log parity");
        assert_eq!(c.skins, r.skins, "{name}: skin-callback parity");
        assert_eq!(
            c.uploads, r.uploads,
            "{name}: GLMesh_UploadBuffers payload parity"
        );
        c.snap.assert_eq(&r.snap, name);
        assert!(c.ok, "{name}: expected a real asset to load");
        assert!(
            !c.uploads.is_empty() && c.uploads.iter().all(|u| u.vertex_bytes > 0),
            "{name}: no vertex payload was recorded"
        );
        compared += 1;
    }
    eprintln!("real MD5 corpus: {compared} models compared bit-for-bit");
}
