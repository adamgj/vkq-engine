//! Differential tests: the Rust SPR loader (quake-capi `Mod_LoadSpriteModel`
//! over quake-formats::spr) vs the sprite half of model_parse.c compiled as
//! `c_ref_*` (Phase 3 M4, AC5).
//!
//! Same shape as `alias_differential.rs`: each side walks its own copy of one
//! synthetic .spr image into its own `Mem_Alloc` heap, and the comparison is
//! the canonical deep-walk snapshot (`quake_ctest::model_hash`) plus the
//! console log plus the `TexMgr_LoadImage` argument stream the shared stub
//! records -- the pixel pointer is compared as a byte offset into that side's
//! own image, so the two heaps' addresses never leak into the diff.
//!
//! The sprite loader touches no shared scratch arrays, so unlike the alias
//! side it has no cross-side ordering requirement; the lock is still held so
//! the stub recorder and console log stay single-writer.

use core::ffi::{c_char, c_void};
use quake_ctest::fs as ctfs;
use quake_ctest::fs::Side;
use quake_types::model_mem::{QModel, MAX_QPATH};

use model_hash::{sprite_snapshot, Snapshot};
use quake_ctest::model_hash;

const DSPRITE_T_SIZE: usize = 36;
const DSPRITEFRAME_T_SIZE: usize = 16;
const SPRITE_VERSION: i32 = 1;
const SPR_SINGLE: i32 = 0;
const SPR_GROUP: i32 = 1;
const SPR_ANGLED: i32 = 2;

extern "C" {
    fn c_ref_Mod_LoadSpriteModel(m: *mut QModel, buffer: *mut c_void);

    fn ctest_modelstub_reset(base: *const u8);
    fn ctest_teximage_count() -> i32;
    fn ctest_teximage_calls() -> *const TexImageCall;
}

/// Mirror of `ctest_teximage_call_t` in `stubs/stubs.c`.
#[repr(C)]
#[derive(Clone, Copy)]
struct TexImageCall {
    name: [c_char; 64],
    width: i32,
    height: i32,
    format: i32,
    data_ofs: i64,
    source_file: [c_char; 64],
    source_offset: u64,
    flags: u32,
}

/// Comparable, printable form of one recorded call.
#[derive(Clone, Debug, PartialEq)]
struct TexImage {
    name: String,
    width: i32,
    height: i32,
    format: i32,
    data_ofs: i64,
    source_file: String,
    source_offset: u64,
    flags: u32,
}

fn cstr(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf.iter().map(|&c| c as u8).collect();
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

impl From<&TexImageCall> for TexImage {
    fn from(c: &TexImageCall) -> Self {
        TexImage {
            name: cstr(&c.name),
            width: c.width,
            height: c.height,
            format: c.format,
            data_ofs: c.data_ofs,
            source_file: cstr(&c.source_file),
            source_offset: c.source_offset,
            flags: c.flags,
        }
    }
}

type LoadFn = unsafe extern "C" fn(*mut QModel, *mut c_void);

fn load_fn(side: Side) -> LoadFn {
    match side {
        Side::C => c_ref_Mod_LoadSpriteModel,
        Side::Rust => quake_rs::model_parse::Mod_LoadSpriteModel,
    }
}

// ---------------------------------------------------------------------------
// fixtures

/// One `dspriteframe_t` plus its `width * height` indexed pixels.
#[derive(Clone, Copy)]
struct FrameImg {
    origin: [i32; 2],
    width: i32,
    height: i32,
    seed: u8,
}

enum SpriteFrame {
    Single(FrameImg),
    /// `frametype != SPR_SINGLE`; `SPR_ANGLED` additionally requires 8 frames
    Group {
        frametype: i32,
        intervals: Vec<f32>,
        frames: Vec<FrameImg>,
    },
}

struct Spr {
    version: i32,
    type_: i32,
    boundingradius: f32,
    width: i32,
    height: i32,
    beamlength: f32,
    synctype: i32,
    frames: Vec<SpriteFrame>,
    /// overrides the frame count written into the header (fatal cases)
    numframes_override: Option<i32>,
}

impl Default for Spr {
    fn default() -> Self {
        Spr {
            version: SPRITE_VERSION,
            type_: 0,
            boundingradius: 12.5,
            // odd, so model_bounds' truncating negate-then-divide is asymmetric
            width: 33,
            height: 17,
            beamlength: 4.0,
            synctype: 1,
            frames: vec![SpriteFrame::Single(FrameImg {
                origin: [-16, 8],
                width: 4,
                height: 3,
                seed: 5,
            })],
            numframes_override: None,
        }
    }
}

fn push_i32(v: &mut Vec<u8>, x: i32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn push_f32(v: &mut Vec<u8>, x: f32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn push_frame(v: &mut Vec<u8>, f: &FrameImg) {
    push_i32(v, f.origin[0]);
    push_i32(v, f.origin[1]);
    push_i32(v, f.width);
    push_i32(v, f.height);
    let n = (f.width.max(0) as usize) * (f.height.max(0) as usize);
    for i in 0..n {
        v.push(f.seed.wrapping_add(i as u8));
    }
}

impl Spr {
    fn build(&self) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"IDSP");
        push_i32(&mut v, self.version);
        push_i32(&mut v, self.type_);
        push_f32(&mut v, self.boundingradius);
        push_i32(&mut v, self.width);
        push_i32(&mut v, self.height);
        push_i32(
            &mut v,
            self.numframes_override.unwrap_or(self.frames.len() as i32),
        );
        push_f32(&mut v, self.beamlength);
        push_i32(&mut v, self.synctype);
        assert_eq!(v.len(), DSPRITE_T_SIZE);

        for f in &self.frames {
            match f {
                SpriteFrame::Single(img) => {
                    push_i32(&mut v, SPR_SINGLE);
                    push_frame(&mut v, img);
                }
                SpriteFrame::Group {
                    frametype,
                    intervals,
                    frames,
                } => {
                    push_i32(&mut v, *frametype);
                    push_i32(&mut v, frames.len() as i32);
                    for x in intervals {
                        push_f32(&mut v, *x);
                    }
                    for img in frames {
                        push_frame(&mut v, img);
                    }
                }
            }
        }
        v
    }
}

fn img(width: i32, height: i32, seed: u8) -> FrameImg {
    FrameImg {
        origin: [-(width / 2), height / 2],
        width,
        height,
        seed,
    }
}

// ---------------------------------------------------------------------------
// per-side driver

const MODEL_NAME: &str = "progs/s_bubble.spr";

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
    textures: Vec<TexImage>,
}

/// Runs one side over its own copy of `image`. Caller must hold [`ctfs::lock`].
fn load_side(side: Side, image: &[u8]) -> Loaded {
    let mut data = image.to_vec();
    let base = data.as_mut_ptr();
    let mut model = new_model(MODEL_NAME);
    let m: *mut QModel = &raw mut *model;

    ctfs::clear_logs();
    // SAFETY: the recorder is a set of plain statics guarded by the fs lock
    unsafe { ctest_modelstub_reset(base) };

    // SAFETY: `base` points at a whole .spr image that outlives the call and
    // `m` at a live zeroed qmodel_t
    unsafe { (load_fn(side))(m, base.cast::<c_void>()) };

    // SAFETY: the model and everything it points at are still live
    let snap = unsafe { sprite_snapshot(m) };
    // SAFETY: the recorder holds `ctest_teximage_count` valid entries
    let textures = unsafe {
        let n = ctest_teximage_count();
        let p = ctest_teximage_calls();
        (0..n as isize)
            .map(|i| TexImage::from(&*p.offset(i)))
            .collect()
    };
    Loaded {
        snap,
        con_log: ctfs::con_log(),
        textures,
    }
}

fn compare(what: &str, spr: &Spr) -> Loaded {
    let image = spr.build();
    let c = load_side(Side::C, &image);
    let r = load_side(Side::Rust, &image);
    assert_eq!(c.con_log, r.con_log, "{what}: console log parity");
    assert_eq!(
        c.textures, r.textures,
        "{what}: TexMgr_LoadImage argument parity"
    );
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
fn single_frame_sprite() {
    let _g = ctfs::lock();
    let out = compare("single", &Spr::default());
    assert_eq!(field(&out.snap, "sprite.numframes"), "1");
    assert_eq!(field(&out.snap, "sprite.frame[0].type"), "0");
    assert_eq!(field(&out.snap, "sprite.frame[0].width"), "4");
    assert_eq!(field(&out.snap, "sprite.frame[0].height"), "3");
    // origin[1] = 8, height = 3
    assert_eq!(field(&out.snap, "sprite.frame[0].up"), "8");
    assert_eq!(field(&out.snap, "sprite.frame[0].down"), "5");
    assert_eq!(field(&out.snap, "sprite.frame[0].left"), "-16");
    assert_eq!(field(&out.snap, "sprite.frame[0].right"), "-12");
    // COMPAT: -maxwidth / 2 truncates towards zero *before* the divide, so an
    // odd maxwidth gives an asymmetric box (33 -> -16 .. 16)
    assert_eq!(field(&out.snap, "model.mins"), "[-16.0, -16.0, -8.0]");
    assert_eq!(field(&out.snap, "model.maxs"), "[16.0, 16.0, 8.0]");
    assert_eq!(field(&out.snap, "model.synctype"), "1");

    assert_eq!(out.textures.len(), 1);
    let t = &out.textures[0];
    assert_eq!(t.name, format!("{MODEL_NAME}:frame0"));
    assert_eq!(t.source_file, MODEL_NAME);
    assert_eq!(
        t.data_ofs,
        (DSPRITE_T_SIZE + 4 + DSPRITEFRAME_T_SIZE) as i64
    );
    assert_eq!(t.source_offset, t.data_ofs as u64);
}

#[test]
fn multiple_single_frames() {
    let _g = ctfs::lock();
    let spr = Spr {
        width: 16,
        height: 16,
        frames: vec![
            SpriteFrame::Single(img(2, 2, 1)),
            SpriteFrame::Single(img(5, 7, 40)),
            SpriteFrame::Single(img(1, 1, 200)),
        ],
        ..Default::default()
    };
    let out = compare("multi-single", &spr);
    assert_eq!(field(&out.snap, "sprite.numframes"), "3");
    assert_eq!(out.textures.len(), 3);
    assert_eq!(out.textures[2].name, format!("{MODEL_NAME}:frame2"));
}

#[test]
fn group_frames() {
    let _g = ctfs::lock();
    let spr = Spr {
        type_: SPR_GROUP,
        frames: vec![SpriteFrame::Group {
            frametype: SPR_GROUP,
            intervals: vec![0.1, 0.25, 1.5],
            frames: vec![img(3, 3, 9), img(4, 2, 60), img(2, 5, 130)],
        }],
        ..Default::default()
    };
    let out = compare("group", &spr);
    assert_eq!(field(&out.snap, "sprite.frame[0].group.numframes"), "3");
    assert_eq!(
        field(&out.snap, "sprite.frame[0].group.interval[1]"),
        "0.25"
    );
    assert_eq!(field(&out.snap, "sprite.frame[0].group[2].width"), "2");
    // framenum is `i * 100 + j` inside a group
    assert_eq!(out.textures[2].name, format!("{MODEL_NAME}:frame2"));
}

#[test]
fn mixed_single_and_group_frames() {
    let _g = ctfs::lock();
    let spr = Spr {
        frames: vec![
            SpriteFrame::Single(img(6, 4, 3)),
            SpriteFrame::Group {
                frametype: SPR_GROUP,
                intervals: vec![0.2, 0.4],
                frames: vec![img(2, 2, 70), img(3, 1, 90)],
            },
            SpriteFrame::Single(img(1, 8, 111)),
        ],
        ..Default::default()
    };
    let out = compare("mixed", &spr);
    assert_eq!(field(&out.snap, "model.numframes"), "3");
    assert_eq!(field(&out.snap, "sprite.frame[1].type"), "1");
    assert_eq!(out.textures.len(), 4);
    // group frame 1 gets framenum 1 * 100 + j
    assert_eq!(out.textures[1].name, format!("{MODEL_NAME}:frame100"));
    assert_eq!(out.textures[2].name, format!("{MODEL_NAME}:frame101"));
    assert_eq!(out.textures[3].name, format!("{MODEL_NAME}:frame2"));
}

#[test]
fn angled_group_of_eight_is_accepted() {
    let _g = ctfs::lock();
    let frames: Vec<FrameImg> = (0..8).map(|i| img(2, 2, 10 * i as u8)).collect();
    let spr = Spr {
        type_: SPR_ANGLED,
        frames: vec![SpriteFrame::Group {
            frametype: SPR_ANGLED,
            intervals: vec![0.1; 8],
            frames,
        }],
        ..Default::default()
    };
    let out = compare("angled-8", &spr);
    assert_eq!(field(&out.snap, "sprite.frame[0].group.numframes"), "8");
    assert_eq!(out.textures.len(), 8);
}

// ---------------------------------------------------------------------------
// Sys_Error parity

fn fatal_fixture(case: &str) -> Spr {
    match case {
        "bad-version" => Spr {
            version: 2,
            ..Default::default()
        },
        "no-frames" => Spr {
            frames: Vec::new(),
            numframes_override: Some(0),
            ..Default::default()
        },
        // SPR_ANGLED with anything but 8 sub-frames
        "angled-bad-count" => Spr {
            type_: SPR_ANGLED,
            frames: vec![SpriteFrame::Group {
                frametype: SPR_ANGLED,
                intervals: vec![0.1, 0.2],
                frames: vec![img(2, 2, 1), img(2, 2, 2)],
            }],
            ..Default::default()
        },
        "interval-zero" => Spr {
            frames: vec![SpriteFrame::Group {
                frametype: SPR_GROUP,
                intervals: vec![0.1, 0.0],
                frames: vec![img(2, 2, 1), img(2, 2, 2)],
            }],
            ..Default::default()
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
        unsafe { c_ref_Mod_LoadSpriteModel(m, base.cast::<c_void>()) };
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
fn no_frames_fatal_parity() {
    assert_fatal_parity("no-frames");
}

#[test]
fn angled_bad_count_fatal_parity() {
    assert_fatal_parity("angled-bad-count");
}

#[test]
fn interval_zero_fatal_parity() {
    assert_fatal_parity("interval-zero");
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
    unsafe { quake_rs::model_parse::Mod_LoadSpriteModel(m, base.cast::<c_void>()) };
    panic!("case {case} did not Sys_Error");
}
