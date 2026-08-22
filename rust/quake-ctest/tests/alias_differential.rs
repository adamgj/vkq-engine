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
//! `stverts`, `triangles` and `poseverts` keep C linkage, so both sides write
//! through the *same* arrays; the growth counter behind `triangles` does not
//! (C owns `triangles_size`, Rust owns `TRIANGLES_SIZE`). Both apply
//! `q_max (size * 2, numtris)` only when `numtris > size`, so a side whose
//! counter ran ahead can skip a realloc after the other side shrank the
//! buffer. Every case therefore runs C and then Rust over the same fixture
//! while holding [`ctfs::lock`], which keeps the two counters in lockstep,
//! and the fatal fixtures use the smallest legal `numtris` -- C reaches
//! `check_tris_size` before its "invalid # of frames" `Sys_Error` while Rust
//! reaches it after, so that is the one asymmetric call in the suite.

use core::ffi::{c_char, c_void};
use quake_ctest::fs as ctfs;
use quake_ctest::fs::Side;
use quake_types::model_mem::{AliasHdr, MTriangle, QModel, MAX_QPATH};
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
    // the vertex/triangle QS-limit warnings need multi-megabyte fixtures, so
    // they are covered by quake-formats' unit tests over `mdl::validate`
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
// Sys_Error parity

/// `numtris` stays at the legal minimum in every fatal fixture: the C reaches
/// `check_tris_size` before its "invalid # of frames" `Sys_Error` and the
/// Rust does not, so this is the one place the two growth counters can drift.
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
