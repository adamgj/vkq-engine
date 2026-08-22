//! Differential tests: the Rust MDL loader (quake-capi `Mod_ParseAliasModel`
//! over quake-formats::mdl) vs the alias half of model_parse.c compiled as
//! `c_ref_*` (Phase 3 M4, AC5).
//!
//! Both sides are handed their own copy of the same synthetic .mdl image and
//! fill their own `Mem_Alloc` heap; the comparison is the canonical deep-walk
//! snapshot from `support/model_hash.rs` (pointers resolved to indices/blob
//! offsets, renderer fields masked) plus the console log and the
//! `Mod_LoadAllSkins` argument stream the shared stub records.
//!
//! `stverts`, `triangles`, `poseverts` and `triangles_size` all keep C
//! linkage, so both sides write through the *same* arrays and realloc the
//! `triangles` allocation against the *same* grow counter. That is a
//! mechanism rather than a property of the fixture set: a private counter on
//! either side could shrink the buffer below the length the other side
//! believes it owns, because `q_max (size * 2, numtris)` only runs when
//! `numtris > size`. Cases still run C before Rust under [`ctfs::lock`], so
//! the console log and the shared scratch are compared over one ordering.

use core::ffi::{c_char, c_int, c_void};
use quake_ctest::fs as ctfs;
use quake_ctest::fs::Side;
use quake_types::model_mem::{
    AliasHdr, MTriangle, Md3XyzNormal, Md5Vert, Md5Vert8, QModel, MAX_QPATH, PV_MD5, PV_MD5_8,
    PV_QUAKE1, PV_QUAKE3,
};
use quake_types::modelgen::{StVert, TriVertX, ALIAS_SINGLE, ALIAS_VERSION};

#[path = "support/model_hash.rs"]
mod model_hash;
use model_hash::{alias_snapshot, AliasScratch, Snapshot};

/// `gl_model.h` -- the shared scratch arrays are declared with these bounds.
const MAXALIASVERTS: usize = 0x7fff;
const MAXALIASFRAMES: usize = 2048;

const MDL_T_SIZE: usize = 84;

extern "C" {
    fn c_ref_Mod_ParseAliasModel(m: *mut QModel, buffer: *mut c_void) -> *mut AliasHdr;
    fn c_ref_Mod_CalcAliasBounds(
        m: *mut QModel,
        a: *mut AliasHdr,
        numvertexes: c_int,
        vertexes: *mut u8,
    );

    // shared by both sides: model_parse.c defines these unconditionally
    static mut stverts: [StVert; MAXALIASVERTS];
    static mut triangles: *mut MTriangle;
    static mut poseverts: [*mut TriVertX; MAXALIASFRAMES];

    fn ctest_modelstub_reset(base: *const u8);
    fn ctest_allskins_count() -> i32;
    fn ctest_allskins_calls() -> *const AllSkinsCall;
}

/// Mirror of `ctest_allskins_call_t` in `stubs/stubs.c`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct AllSkinsCall {
    numskins: i32,
    pskintype_ofs: i64,
}

type ParseFn = unsafe extern "C" fn(*mut QModel, *mut c_void) -> *mut AliasHdr;

fn parse_fn(side: Side) -> ParseFn {
    match side {
        Side::C => c_ref_Mod_ParseAliasModel,
        Side::Rust => quake_rs::model_parse::Mod_ParseAliasModel,
    }
}

// ---------------------------------------------------------------------------
// fixtures

enum Frame {
    Single {
        name: &'static str,
        bboxmin: [u8; 4],
        bboxmax: [u8; 4],
        seed: u8,
    },
    /// any `frametype != ALIAS_SINGLE`; only the first interval is kept
    Group {
        intervals: Vec<f32>,
        bboxmin: [u8; 4],
        bboxmax: [u8; 4],
        seed: u8,
    },
}

struct Mdl {
    version: i32,
    numverts: i32,
    numtris: i32,
    numskins: i32,
    skinwidth: i32,
    skinheight: i32,
    synctype: i32,
    flags: i32,
    size: f32,
    /// read with `ReadLongUnaligned` and *converted* into the float field
    boundingradius_bits: i32,
    scale: [f32; 3],
    scale_origin: [f32; 3],
    eyeposition: [f32; 3],
    frames: Vec<Frame>,
}

impl Default for Mdl {
    fn default() -> Self {
        Mdl {
            version: ALIAS_VERSION,
            numverts: 5,
            numtris: 4,
            numskins: 0,
            skinwidth: 32,
            skinheight: 32,
            synctype: 1,
            // the high byte is masked off again by Mod_SetExtraFlags
            flags: 0x0108,
            size: 11.0,
            boundingradius_bits: 37,
            scale: [0.5, 0.25, 2.0],
            scale_origin: [-1.5, 0.0, 3.5],
            eyeposition: [0.0, 0.0, 24.0],
            frames: vec![Frame::Single {
                name: "idle1",
                bboxmin: [1, 2, 3, 4],
                bboxmax: [250, 251, 252, 253],
                seed: 7,
            }],
        }
    }
}

fn push_i32(v: &mut Vec<u8>, x: i32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn push_f32(v: &mut Vec<u8>, x: f32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn push_poseverts(v: &mut Vec<u8>, numverts: i32, seed: u8) {
    for i in 0..numverts.max(0) {
        let b = seed.wrapping_add(i as u8);
        v.extend_from_slice(&[b, b.wrapping_mul(3), b.wrapping_add(11), b & 0x7f]);
    }
}

/// `daliasframe_t` (bboxmin, bboxmax, name[16]) followed by its pose vertices.
fn push_dalias_frame(
    v: &mut Vec<u8>,
    name: &str,
    min: [u8; 4],
    max: [u8; 4],
    numverts: i32,
    seed: u8,
) {
    v.extend_from_slice(&min);
    v.extend_from_slice(&max);
    let mut n = [0u8; 16];
    for (i, b) in name.bytes().take(16).enumerate() {
        n[i] = b;
    }
    v.extend_from_slice(&n);
    push_poseverts(v, numverts, seed);
}

impl Mdl {
    fn build(&self) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"IDPO");
        push_i32(&mut v, self.version);
        for x in self.scale {
            push_f32(&mut v, x);
        }
        for x in self.scale_origin {
            push_f32(&mut v, x);
        }
        push_i32(&mut v, self.boundingradius_bits);
        for x in self.eyeposition {
            push_f32(&mut v, x);
        }
        push_i32(&mut v, self.numskins);
        push_i32(&mut v, self.skinwidth);
        push_i32(&mut v, self.skinheight);
        push_i32(&mut v, self.numverts);
        push_i32(&mut v, self.numtris);
        push_i32(&mut v, self.frames.len() as i32);
        push_i32(&mut v, self.synctype);
        push_i32(&mut v, self.flags);
        push_f32(&mut v, self.size);
        assert_eq!(v.len(), MDL_T_SIZE);

        // every fixture uses numskins = 0, so the Mod_LoadAllSkins stub hands
        // the cursor straight back and no skin bytes follow the header

        for i in 0..self.numverts.max(0) {
            push_i32(&mut v, if i % 2 == 0 { 0 } else { 0x20 });
            push_i32(&mut v, i * 3);
            push_i32(&mut v, 100 - i);
        }
        for i in 0..self.numtris.max(0) {
            push_i32(&mut v, i % 2);
            for j in 0..3 {
                push_i32(&mut v, (i + j) % self.numverts.max(1));
            }
        }

        for f in &self.frames {
            match f {
                Frame::Single {
                    name,
                    bboxmin,
                    bboxmax,
                    seed,
                } => {
                    push_i32(&mut v, ALIAS_SINGLE);
                    push_dalias_frame(&mut v, name, *bboxmin, *bboxmax, self.numverts, *seed);
                }
                Frame::Group {
                    intervals,
                    bboxmin,
                    bboxmax,
                    seed,
                } => {
                    push_i32(&mut v, ALIAS_SINGLE + 1);
                    push_i32(&mut v, intervals.len() as i32);
                    v.extend_from_slice(bboxmin);
                    v.extend_from_slice(bboxmax);
                    for x in intervals {
                        push_f32(&mut v, *x);
                    }
                    for k in 0..intervals.len() {
                        push_dalias_frame(
                            &mut v,
                            "grp",
                            *bboxmin,
                            *bboxmax,
                            self.numverts,
                            seed.wrapping_add(k as u8),
                        );
                    }
                }
            }
        }
        v
    }
}

// ---------------------------------------------------------------------------
// per-side driver

const MODEL_NAME: &str = "progs/test.mdl";

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

struct Loaded {
    snap: Snapshot,
    con_log: Vec<String>,
    skins: Vec<AllSkinsCall>,
}

/// Runs one side over its own copy of `image`. Caller must hold
/// [`ctfs::lock`], and must run [`Side::C`] before [`Side::Rust`].
fn load_side(side: Side, image: &[u8]) -> Loaded {
    let mut data = image.to_vec();
    let base = data.as_mut_ptr();
    let mut model = new_model(MODEL_NAME);
    let m: *mut QModel = &raw mut *model;

    ctfs::clear_logs();
    // SAFETY: the recorder is a set of plain statics guarded by the fs lock
    unsafe { ctest_modelstub_reset(base) };

    // SAFETY: `base` points at a whole .mdl image that outlives the call and
    // `m` at a live zeroed qmodel_t
    let h = unsafe { (parse_fn(side))(m, base.cast::<c_void>()) };
    assert!(!h.is_null(), "{side:?}: parse returned null");

    // SAFETY: the header, the model and the shared scratch arrays are all
    // still live, and `data` is the image this side walked
    let snap = unsafe {
        alias_snapshot(
            m,
            h,
            AliasScratch {
                stverts: core::ptr::addr_of!(stverts).cast::<StVert>(),
                triangles: triangles.cast_const(),
                poseverts: core::ptr::addr_of!(poseverts).cast::<*const TriVertX>(),
                base,
                base_len: data.len(),
            },
        )
    };
    // SAFETY: the recorder holds `ctest_allskins_count` valid entries
    let skins = unsafe {
        let n = ctest_allskins_count();
        let p = ctest_allskins_calls();
        (0..n as isize).map(|i| *p.offset(i)).collect()
    };
    Loaded {
        snap,
        con_log: ctfs::con_log(),
        skins,
    }
}

fn compare(what: &str, mdl: &Mdl) -> Loaded {
    let image = mdl.build();
    let c = load_side(Side::C, &image);
    let r = load_side(Side::Rust, &image);
    assert_eq!(c.con_log, r.con_log, "{what}: console log parity");
    assert_eq!(c.skins, r.skins, "{what}: Mod_LoadAllSkins argument parity");
    c.snap.assert_eq(&r.snap, what);
    r
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
fn single_frame_model() {
    let _g = ctfs::lock();
    let out = compare("single", &Mdl::default());
    assert_eq!(field(&out.snap, "alias.numposes"), "1");
    assert_eq!(field(&out.snap, "alias.frame[0].numposes"), "1");
    assert_eq!(field(&out.snap, "alias.frame[0].name"), "\"idle1\"");
    // boundingradius is read as a long and converted, not reinterpreted
    assert_eq!(field(&out.snap, "alias.boundingradius"), "37");
    assert_eq!(field(&out.snap, "model.synctype"), "1");
    // Mod_SetExtraFlags masks everything above the low byte (plus MF_HOLEY)
    assert_eq!(field(&out.snap, "model.flags"), "8");
    assert_eq!(out.skins.len(), 1);
    assert_eq!(out.skins[0].numskins, 0);
    assert_eq!(out.skins[0].pskintype_ofs, MDL_T_SIZE as i64);
}

#[test]
fn group_frames_keep_only_the_first_interval() {
    let _g = ctfs::lock();
    let mdl = Mdl {
        frames: vec![Frame::Group {
            intervals: vec![0.1, 0.2, 0.3],
            bboxmin: [10, 11, 12, 0],
            bboxmax: [200, 201, 202, 0],
            seed: 40,
        }],
        ..Default::default()
    };
    let out = compare("group", &mdl);
    assert_eq!(field(&out.snap, "alias.numposes"), "3");
    assert_eq!(field(&out.snap, "alias.frame[0].numposes"), "3");
    assert_eq!(field(&out.snap, "alias.frame[0].interval"), "0.1");
    // the group path never touches frame->name
    assert_eq!(field(&out.snap, "alias.frame[0].name"), "\"\"");
}

#[test]
fn mixed_frames_and_bounds() {
    let _g = ctfs::lock();
    let mdl = Mdl {
        // asymmetric scales so the three bounding boxes and the two radii all
        // come out different
        scale: [1.5, -0.5, 0.125],
        scale_origin: [-8.0, 4.0, -2.0],
        frames: vec![
            Frame::Single {
                name: "aaaaaaaaaaaaaaaa", // exactly 16: q_strlcpy truncates
                bboxmin: [0, 0, 0, 0],
                bboxmax: [255, 255, 255, 255],
                seed: 1,
            },
            Frame::Group {
                intervals: vec![0.05, 0.5],
                bboxmin: [5, 6, 7, 8],
                bboxmax: [9, 10, 11, 12],
                seed: 90,
            },
            Frame::Single {
                name: "z",
                bboxmin: [3, 3, 3, 3],
                bboxmax: [4, 4, 4, 4],
                seed: 200,
            },
        ],
        ..Default::default()
    };
    let out = compare("mixed", &mdl);
    assert_eq!(field(&out.snap, "alias.numposes"), "4");
    assert_eq!(
        field(&out.snap, "alias.frame[0].name"),
        "\"aaaaaaaaaaaaaaa\""
    );
    assert_eq!(field(&out.snap, "model.numframes"), "3");
    assert_eq!(field(&out.snap, "alias.frame[1].firstpose"), "1");
    assert_eq!(field(&out.snap, "alias.frame[2].firstpose"), "3");
}

#[test]
fn tall_skin_warns() {
    let _g = ctfs::lock();
    let mdl = Mdl {
        skinheight: 481,
        ..Default::default()
    };
    let out = compare("tall-skin", &mdl);
    assert!(
        out.con_log.iter().any(|l| l.contains("skin taller than")),
        "expected the tall-skin warning, got {:?}",
        out.con_log
    );
}

#[test]
fn vertex_count_over_qs_limit_warns() {
    let _g = ctfs::lock();
    let mdl = Mdl {
        numverts: 2001,
        ..Default::default()
    };
    let out = compare("verts-over-qs", &mdl);
    assert!(
        out.con_log
            .iter()
            .any(|l| l.contains("vertex count of 2001")),
        "expected the QS vertex-limit warning, got {:?}",
        out.con_log
    );
}

#[test]
fn triangle_count_over_qs_limit_warns() {
    let _g = ctfs::lock();
    let mdl = Mdl {
        numtris: 4097,
        ..Default::default()
    };
    let out = compare("tris-over-qs", &mdl);
    assert!(
        out.con_log
            .iter()
            .any(|l| l.contains("triangle count of 4097")),
        "expected the QS triangle-limit warning, got {:?}",
        out.con_log
    );
}

/// Pins the emission order `mdl::validate`'s push order encodes: a fixture
/// that trips three non-fatal diagnostics at once must produce them in the
/// same sequence the C's interleaved checks do.
#[test]
fn multiple_warnings_keep_c_order() {
    let _g = ctfs::lock();
    let mdl = Mdl {
        skinheight: 481,
        numverts: 2500,
        numtris: 5000,
        ..Default::default()
    };
    let out = compare("many-warnings", &mdl);
    let hits: Vec<&String> = out
        .con_log
        .iter()
        .filter(|l| {
            l.contains("skin taller than")
                || l.contains("vertex count of")
                || l.contains("triangle count of")
        })
        .collect();
    assert_eq!(
        hits.len(),
        3,
        "expected all three warnings, got {:?}",
        out.con_log
    );
    assert!(hits[0].contains("skin taller than"), "{hits:?}");
    assert!(hits[1].contains("vertex count of"), "{hits:?}");
    assert!(hits[2].contains("triangle count of"), "{hits:?}");
}

#[test]
fn triangles_buffer_grows_in_lockstep() {
    let _g = ctfs::lock();
    for numtris in [4, 5, 40, 41] {
        let mdl = Mdl {
            numtris,
            ..Default::default()
        };
        compare(&format!("grow-{numtris}"), &mdl);
    }
}

// ---------------------------------------------------------------------------
// Mod_CalcAliasBounds: the MD5/MD3 arms
//
// Under -Duse_rust_formats this export also serves the MD5 and MD3 loaders in
// gl_model.c, which stay C until M5, so `PV_MD5`/`PV_MD5_8`/`PV_QUAKE3` are
// live in the engine even though M4 ports no MD5/MD3 parsing. It is a leaf
// function -- no file image, no Mem_Alloc heap, no shared scratch -- so the
// arms are driven directly off a vertex buffer.

fn bounds_bits(m: &QModel) -> Vec<u32> {
    [m.mins, m.maxs, m.ymins, m.ymaxs, m.rmins, m.rmaxs]
        .iter()
        .flat_map(|v| v.iter().map(|x| x.to_bits()))
        .collect()
}

fn md5_bytes(xyz: &[[f32; 3]]) -> Vec<u8> {
    let stride = core::mem::size_of::<Md5Vert>();
    let mut v = vec![0u8; xyz.len() * stride];
    for (i, p) in xyz.iter().enumerate() {
        for (j, x) in p.iter().enumerate() {
            let o = i * stride + j * 4;
            v[o..o + 4].copy_from_slice(&x.to_le_bytes());
        }
    }
    v
}

fn md5_8_bytes(xyz: &[[f32; 3]]) -> Vec<u8> {
    let stride = core::mem::size_of::<Md5Vert8>();
    let mut v = vec![0u8; xyz.len() * stride];
    for (i, p) in xyz.iter().enumerate() {
        for (j, x) in p.iter().enumerate() {
            let o = i * stride + j * 4;
            v[o..o + 4].copy_from_slice(&x.to_le_bytes());
        }
    }
    v
}

fn md3_bytes(xyz: &[[i16; 3]]) -> Vec<u8> {
    let stride = core::mem::size_of::<Md3XyzNormal>();
    let mut v = vec![0u8; xyz.len() * stride];
    for (i, p) in xyz.iter().enumerate() {
        for (j, x) in p.iter().enumerate() {
            let o = i * stride + j * 2;
            v[o..o + 2].copy_from_slice(&x.to_le_bytes());
        }
    }
    v
}

/// Runs both sides of `Mod_CalcAliasBounds` over its own copy of `bytes` and
/// compares the six output vectors bit-for-bit (so `-0.0` and `FLT_MAX`
/// seeding are not smoothed over by float equality).
fn compare_bounds(what: &str, poseverttype: c_int, numvertexes: c_int, bytes: &[u8]) {
    let _g = ctfs::lock();
    let mut cm = new_model(MODEL_NAME);
    let mut rm = new_model(MODEL_NAME);
    // SAFETY: aliashdr_t comes out of a zeroing Mem_Alloc in the engine too,
    // and Mod_CalcAliasBounds only reads poseverttype/numposes/numverts and
    // the two scale vectors, for all of which all-zero is a valid value
    let mut ch: Box<AliasHdr> = Box::new(unsafe { core::mem::zeroed() });
    // SAFETY: as above
    let mut rh: Box<AliasHdr> = Box::new(unsafe { core::mem::zeroed() });
    ch.poseverttype = poseverttype;
    rh.poseverttype = poseverttype;
    let mut cb = bytes.to_vec();
    let mut rb = bytes.to_vec();
    // SAFETY: both headers and models are live, and each buffer holds
    // `numvertexes` records of the stride `poseverttype` selects
    unsafe {
        c_ref_Mod_CalcAliasBounds(&raw mut *cm, &raw mut *ch, numvertexes, cb.as_mut_ptr());
        quake_rs::model_parse::Mod_CalcAliasBounds(
            &raw mut *rm,
            &raw mut *rh,
            numvertexes,
            rb.as_mut_ptr(),
        );
    }
    assert_eq!(bounds_bits(&cm), bounds_bits(&rm), "{what}: bounds parity");
}

/// asymmetric about every axis, with a zero and a negative-zero component
const XYZ: [[f32; 3]; 5] = [
    [1.0, -2.0, 0.5],
    [-3.5, 4.0, -0.0],
    [0.0, 0.25, 12.0],
    [7.5, -7.5, -1.5],
    [-0.125, 0.0, 3.0],
];

#[test]
fn calc_bounds_md5_parity() {
    compare_bounds("md5", PV_MD5, XYZ.len() as c_int, &md5_bytes(&XYZ));
}

#[test]
fn calc_bounds_md5_8_parity() {
    compare_bounds("md5_8", PV_MD5_8, XYZ.len() as c_int, &md5_8_bytes(&XYZ));
}

#[test]
fn calc_bounds_quake3_parity() {
    // MD3_XYZ_SCALE (1/64) is applied on this arm and nowhere else
    let xyz = [
        [64i16, -128, 32],
        [-1, 1, 0],
        [i16::MAX, i16::MIN, 7],
        [0, 0, 0],
    ];
    compare_bounds("quake3", PV_QUAKE3, xyz.len() as c_int, &md3_bytes(&xyz));
}

/// The empty-stream seeding (`mins = FLT_MAX`, `ymins = [-0.0, -0.0, FLT_MAX]`)
/// is otherwise only reached transitively through the QUAKE1 path.
#[test]
fn calc_bounds_empty_parity() {
    compare_bounds("md5-empty", PV_MD5, 0, &md5_bytes(&[]));
    compare_bounds("quake3-empty", PV_QUAKE3, 0, &md3_bytes(&[]));
    // numposes stays 0 on the zeroed header, so the QUAKE1 arm sees an empty
    // pose stream without touching `poseverts`
    compare_bounds("quake1-empty", PV_QUAKE1, 0, &[]);
}

// ---------------------------------------------------------------------------
// Sys_Error parity

/// The C reaches `check_tris_size` before its "invalid # of frames"
/// `Sys_Error` and the Rust reaches it after, so these fixtures are the one
/// place the two sides call it a different number of times. `triangles_size`
/// is shared, so that costs at most one extra grow, never a shrink.
fn fatal_fixture(case: &str) -> Mdl {
    let base = Mdl {
        numtris: 1,
        ..Default::default()
    };
    match case {
        "bad-version" => Mdl { version: 5, ..base },
        "no-vertices" => Mdl {
            numverts: 0,
            ..base
        },
        "no-triangles" => Mdl { numtris: 0, ..base },
        "no-frames" => Mdl {
            frames: Vec::new(),
            ..base
        },
        "too-many-vertices" => Mdl {
            numverts: MAXALIASVERTS as i32 + 1,
            frames: Vec::new(),
            ..base
        },
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
    unsafe { ctest_modelstub_reset(base) };

    let c_msg = ctfs::catch_sys_error(|| {
        // SAFETY: whole image, live model; the call is expected not to return
        unsafe { c_ref_Mod_ParseAliasModel(m, base.cast::<c_void>()) };
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
fn no_vertices_fatal_parity() {
    assert_fatal_parity("no-vertices");
}

#[test]
fn no_triangles_fatal_parity() {
    assert_fatal_parity("no-triangles");
}

#[test]
fn no_frames_fatal_parity() {
    assert_fatal_parity("no-frames");
}

#[test]
fn too_many_vertices_fatal_parity() {
    assert_fatal_parity("too-many-vertices");
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
    unsafe { ctest_modelstub_reset(base) };
    // SAFETY: whole image, live model; expected not to return
    unsafe { quake_rs::model_parse::Mod_ParseAliasModel(m, base.cast::<c_void>()) };
    panic!("case {case} did not Sys_Error");
}
