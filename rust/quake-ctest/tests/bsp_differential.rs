//! Differential tests: the Rust brush/BSP loaders (quake-capi model_parse
//! shims + quake-formats::bsp) vs the brush half of model_parse.c compiled as
//! `c_ref_*` (Phase 3 M3, AC4).
//!
//! Each case drives both sides through the same call order gl_model.c's
//! `Mod_LoadBrushModel` uses, over a synthetic BSP built in-process for one of
//! the five dialects (BSP29, BSP30/Valve, 2PSB, BSP2, Quake64), then compares
//! a canonical deep-walk snapshot of the resulting `qmodel_t` graph (see
//! `support/model_hash.rs`) plus the console log. Pointer values necessarily
//! differ between the two `Mem_Alloc` heaps, so the snapshot resolves every
//! pointer to an index or a blob offset.
//!
//! The four `Host_Error`-capable entry points are status-returning on the Rust
//! side (PLAN.md §4.3), so the C side runs under `ctest_try_host` and the two
//! error strings are compared directly.

use core::ffi::{c_char, c_int, c_void};
use quake_c_sys::FILE;
use quake_ctest::fs as ctfs;
use quake_ctest::fs::Side;
use quake_types::bspfile::{
    LumpT, BSP2VERSION_2PSB, BSP2VERSION_BSP2, BSPVERSION, BSPVERSION_QUAKE64, BSPVERSION_VALVE,
    CONTENTS_EMPTY, CONTENTS_SOLID, HEADER_LUMPS, LUMP_CLIPNODES, LUMP_EDGES, LUMP_ENTITIES,
    LUMP_FACES, LUMP_LEAFS, LUMP_LIGHTING, LUMP_MARKSURFACES, LUMP_MODELS, LUMP_NODES, LUMP_PLANES,
    LUMP_SURFEDGES, LUMP_TEXINFO, LUMP_TEXTURES, LUMP_VERTEXES, LUMP_VISIBILITY, PLANE_ANYX,
    PLANE_X, PLANE_Y, PLANE_Z, TEX_SPECIAL,
};
use quake_types::model_mem::{MSurface, QModel, MAX_QPATH};
use std::sync::Once;

#[path = "support/model_hash.rs"]
mod model_hash;
use model_hash::{snapshot, BlobLens, Snapshot};

// ---------------------------------------------------------------------------
// the c_ref half of the seam plus the M3 stub controls

extern "C" {
    fn c_ref_Mod_DecompressVis(in_: *mut u8, model: *mut QModel) -> *mut u8;
    fn c_ref_Mod_ParseTextures(
        m: *mut QModel,
        mod_base: *mut u8,
        l: *const LumpT,
        wads: *mut c_void,
    );
    fn c_ref_Mod_LoadLighting(m: *mut QModel, mod_base: *mut u8, l: *const LumpT);
    fn c_ref_Mod_LoadVisibility(m: *mut QModel, mod_base: *mut u8, l: *const LumpT);
    fn c_ref_Mod_LoadEntities(m: *mut QModel, mod_base: *mut u8, l: *const LumpT);
    fn c_ref_Mod_LoadVertexes(m: *mut QModel, mod_base: *mut u8, l: *const LumpT);
    fn c_ref_Mod_LoadEdges(m: *mut QModel, mod_base: *mut u8, l: *const LumpT, bsp2: c_int);
    fn c_ref_Mod_LoadTexinfo(m: *mut QModel, mod_base: *mut u8, l: *const LumpT);
    fn c_ref_CalcSurfaceExtents(m: *mut QModel, s: *mut MSurface);
    fn c_ref_Mod_ParseFaces(m: *mut QModel, mod_base: *mut u8, l: *const LumpT, bsp2: bool);
    fn c_ref_Mod_LoadNodes(m: *mut QModel, mod_base: *mut u8, l: *const LumpT, bsp2: c_int);
    fn c_ref_Mod_LoadLeafs(m: *mut QModel, mod_base: *mut u8, l: *const LumpT, bsp2: c_int);
    fn c_ref_Mod_LoadClipnodes(m: *mut QModel, mod_base: *mut u8, l: *const LumpT, bsp2: bool);
    fn c_ref_Mod_MakeHull0(m: *mut QModel);
    fn c_ref_Mod_LoadMarksurfaces(m: *mut QModel, mod_base: *mut u8, l: *const LumpT, bsp2: c_int);
    fn c_ref_Mod_LoadSurfedges(m: *mut QModel, mod_base: *mut u8, l: *const LumpT);
    fn c_ref_Mod_LoadPlanes(m: *mut QModel, mod_base: *mut u8, l: *const LumpT);
    fn c_ref_Mod_LoadSubmodels(m: *mut QModel, mod_base: *mut u8, l: *const LumpT);
    fn c_ref_Mod_SetupSubmodels(m: *mut QModel);
    fn c_ref_Mod_FindVisibilityExternal(m: *mut QModel, loadname: *const c_char) -> *mut FILE;
    fn c_ref_Mod_LoadVisibilityExternal(f: *mut FILE) -> *mut u8;
    fn c_ref_Mod_LoadLeafsExternal(m: *mut QModel, f: *mut FILE);

    fn ctest_try_host(f: unsafe extern "C" fn(*mut c_void), arg: *mut c_void) -> c_int;
    fn ctest_host_error_message() -> *const c_char;
    fn ctest_set_sv_modelname(name: *const c_char);
    fn ctest_set_external_ents(value: f32);
    fn ctest_fill_dummy_textures(m: *mut QModel);
    fn ctest_mod_pool_reset();
    fn ctest_mod_pool_get(i: c_int) -> *mut QModel;
    fn ctest_mod_pool_len() -> c_int;
    fn fclose(f: *mut FILE) -> c_int;
}

/// The infallible half of the seam, per side.
struct Seam {
    vertexes: unsafe extern "C" fn(*mut QModel, *mut u8, *const LumpT),
    edges: unsafe extern "C" fn(*mut QModel, *mut u8, *const LumpT, c_int),
    surfedges: unsafe extern "C" fn(*mut QModel, *mut u8, *const LumpT),
    entities: unsafe extern "C" fn(*mut QModel, *mut u8, *const LumpT),
    textures: unsafe extern "C" fn(*mut QModel, *mut u8, *const LumpT, *mut c_void),
    lighting: unsafe extern "C" fn(*mut QModel, *mut u8, *const LumpT),
    planes: unsafe extern "C" fn(*mut QModel, *mut u8, *const LumpT),
    texinfo: unsafe extern "C" fn(*mut QModel, *mut u8, *const LumpT),
    faces: unsafe extern "C" fn(*mut QModel, *mut u8, *const LumpT, bool),
    extents: unsafe extern "C" fn(*mut QModel, *mut MSurface),
    visibility: unsafe extern "C" fn(*mut QModel, *mut u8, *const LumpT),
    nodes: unsafe extern "C" fn(*mut QModel, *mut u8, *const LumpT, c_int),
    submodels: unsafe extern "C" fn(*mut QModel, *mut u8, *const LumpT),
    make_hull0: unsafe extern "C" fn(*mut QModel),
    decompress_vis: unsafe extern "C" fn(*mut u8, *mut QModel) -> *mut u8,
    find_vis_external: unsafe extern "C" fn(*mut QModel, *const c_char) -> *mut FILE,
    load_vis_external: unsafe extern "C" fn(*mut FILE) -> *mut u8,
}

fn seam(side: Side) -> Seam {
    use quake_rs::model_parse as rs;
    match side {
        Side::C => Seam {
            vertexes: c_ref_Mod_LoadVertexes,
            edges: c_ref_Mod_LoadEdges,
            surfedges: c_ref_Mod_LoadSurfedges,
            entities: c_ref_Mod_LoadEntities,
            textures: c_ref_Mod_ParseTextures,
            lighting: c_ref_Mod_LoadLighting,
            planes: c_ref_Mod_LoadPlanes,
            texinfo: c_ref_Mod_LoadTexinfo,
            faces: c_ref_Mod_ParseFaces,
            extents: c_ref_CalcSurfaceExtents,
            visibility: c_ref_Mod_LoadVisibility,
            nodes: c_ref_Mod_LoadNodes,
            submodels: c_ref_Mod_LoadSubmodels,
            make_hull0: c_ref_Mod_MakeHull0,
            decompress_vis: c_ref_Mod_DecompressVis,
            find_vis_external: c_ref_Mod_FindVisibilityExternal,
            load_vis_external: c_ref_Mod_LoadVisibilityExternal,
        },
        Side::Rust => Seam {
            vertexes: rs::Mod_LoadVertexes,
            edges: rs::Mod_LoadEdges,
            surfedges: rs::Mod_LoadSurfedges,
            entities: rs::Mod_LoadEntities,
            textures: rs::Mod_ParseTextures,
            lighting: rs::Mod_LoadLighting,
            planes: rs::Mod_LoadPlanes,
            texinfo: rs::Mod_LoadTexinfo,
            faces: rs::Mod_ParseFaces,
            extents: rs::CalcSurfaceExtents,
            visibility: rs::Mod_LoadVisibility,
            nodes: rs::Mod_LoadNodes,
            submodels: rs::Mod_LoadSubmodels,
            make_hull0: rs::Mod_MakeHull0,
            decompress_vis: rs::Mod_DecompressVis,
            find_vis_external: rs::Mod_FindVisibilityExternal,
            load_vis_external: rs::Mod_LoadVisibilityExternal,
        },
    }
}

// ---------------------------------------------------------------------------
// Host_Error plumbing: C longjmps out of the seam, Rust returns a status

/// Runs `f` with the stub's `Host_Error` trap armed, returning the message if
/// it fired. The longjmp skips this Rust trampoline's frame, exactly like
/// [`ctfs::catch_sys_error`] — only ever used for `Side::C`.
fn c_host_try(f: &mut dyn FnMut()) -> Option<String> {
    unsafe extern "C" fn trampoline(arg: *mut c_void) {
        // SAFETY: arg is the &mut &mut dyn FnMut passed below, alive for the
        // whole call
        let f = unsafe { &mut *arg.cast::<&mut dyn FnMut()>() };
        f();
    }
    let mut f = f;
    let arg = (&raw mut f).cast::<c_void>();
    // SAFETY: the trampoline receives the pointer to `f` built just above
    let hit = unsafe { ctest_try_host(trampoline, arg) };
    if hit == 0 {
        return None;
    }
    // SAFETY: the stub's message buffer is static and NUL-terminated
    let msg = unsafe { std::ffi::CStr::from_ptr(ctest_host_error_message()) };
    Some(msg.to_string_lossy().into_owned())
}

fn err_buf() -> Vec<c_char> {
    vec![0; 256]
}

fn err_string(buf: &[c_char]) -> String {
    // SAFETY: the shim writes a NUL-terminated string into the 256-byte buffer
    let msg = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
    msg.to_string_lossy().into_owned()
}

/// # Safety
/// `m`/`base`/`l` must be the live model, file image and lump of one side.
unsafe fn seam_leafs(
    side: Side,
    m: *mut QModel,
    base: *mut u8,
    l: *const LumpT,
    bsp2: c_int,
) -> Option<String> {
    match side {
        // SAFETY: caller contract; the trap is armed around the C call
        Side::C => c_host_try(&mut || unsafe { c_ref_Mod_LoadLeafs(m, base, l, bsp2) }),
        Side::Rust => {
            let mut err = err_buf();
            // SAFETY: caller contract plus a 256-byte err buffer
            let ok = unsafe {
                quake_rs::model_parse::quake_rs_mod_load_leafs(m, base, l, bsp2, err.as_mut_ptr())
            };
            (ok == 0).then(|| err_string(&err))
        }
    }
}

/// # Safety
/// As [`seam_leafs`].
unsafe fn seam_clipnodes(
    side: Side,
    m: *mut QModel,
    base: *mut u8,
    l: *const LumpT,
    bsp2: bool,
) -> Option<String> {
    match side {
        // SAFETY: caller contract; the trap is armed around the C call
        Side::C => c_host_try(&mut || unsafe { c_ref_Mod_LoadClipnodes(m, base, l, bsp2) }),
        Side::Rust => {
            let mut err = err_buf();
            // SAFETY: caller contract plus a 256-byte err buffer
            let ok = unsafe {
                quake_rs::model_parse::quake_rs_mod_load_clipnodes(
                    m,
                    base,
                    l,
                    bsp2,
                    err.as_mut_ptr(),
                )
            };
            (ok == 0).then(|| err_string(&err))
        }
    }
}

/// # Safety
/// As [`seam_leafs`].
unsafe fn seam_marksurfaces(
    side: Side,
    m: *mut QModel,
    base: *mut u8,
    l: *const LumpT,
    bsp2: c_int,
) -> Option<String> {
    match side {
        // SAFETY: caller contract; the trap is armed around the C call
        Side::C => c_host_try(&mut || unsafe { c_ref_Mod_LoadMarksurfaces(m, base, l, bsp2) }),
        Side::Rust => {
            let mut err = err_buf();
            // SAFETY: caller contract plus a 256-byte err buffer
            let ok = unsafe {
                quake_rs::model_parse::quake_rs_mod_load_marksurfaces(
                    m,
                    base,
                    l,
                    bsp2,
                    err.as_mut_ptr(),
                )
            };
            (ok == 0).then(|| err_string(&err))
        }
    }
}

/// # Safety
/// `m` must be a fully loaded model; `sv_modelname` NUL-terminated.
unsafe fn seam_setup_submodels(
    side: Side,
    m: *mut QModel,
    sv_modelname: &std::ffi::CStr,
) -> Option<String> {
    match side {
        // SAFETY: caller contract; the trap is armed around the C call. The C
        // reads sv.modelname itself, which the stub setter has already filled.
        Side::C => c_host_try(&mut || unsafe { c_ref_Mod_SetupSubmodels(m) }),
        Side::Rust => {
            let mut err = err_buf();
            // SAFETY: caller contract plus a 256-byte err buffer
            let ok = unsafe {
                quake_rs::model_parse::quake_rs_mod_setup_submodels(
                    m,
                    sv_modelname.as_ptr(),
                    err.as_mut_ptr(),
                )
            };
            (ok == 0).then(|| err_string(&err))
        }
    }
}

/// # Safety
/// `m` must be a live model and `f` an open `.vis` file positioned after the
/// visibility block.
unsafe fn seam_leafs_external(side: Side, m: *mut QModel, f: *mut FILE) -> Option<String> {
    match side {
        // SAFETY: caller contract; the trap is armed around the C call
        Side::C => c_host_try(&mut || unsafe { c_ref_Mod_LoadLeafsExternal(m, f) }),
        Side::Rust => {
            let mut err = err_buf();
            // SAFETY: caller contract plus a 256-byte err buffer
            let ok = unsafe {
                quake_rs::model_parse::quake_rs_mod_load_leafs_external(m, f, err.as_mut_ptr())
            };
            (ok == 0).then(|| err_string(&err))
        }
    }
}

// ---------------------------------------------------------------------------
// synthetic BSP builder

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dialect {
    Bsp29,
    Valve,
    P2psb,
    Bsp2,
    Q64,
}

const DIALECTS: [Dialect; 5] = [
    Dialect::Bsp29,
    Dialect::Valve,
    Dialect::P2psb,
    Dialect::Bsp2,
    Dialect::Q64,
];

impl Dialect {
    fn version(self) -> i32 {
        match self {
            Dialect::Bsp29 => BSPVERSION,
            Dialect::Valve => BSPVERSION_VALVE,
            Dialect::P2psb => BSP2VERSION_2PSB,
            Dialect::Bsp2 => BSP2VERSION_BSP2,
            Dialect::Q64 => BSPVERSION_QUAKE64,
        }
    }

    /// The `bsp2` int gl_model.c derives from the version (0/1/2).
    fn bsp2(self) -> c_int {
        match self {
            Dialect::P2psb => 1,
            Dialect::Bsp2 => 2,
            _ => 0,
        }
    }

    fn long_records(self) -> bool {
        self.bsp2() != 0
    }
}

#[derive(Default)]
struct Buf(Vec<u8>);

impl Buf {
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    fn i16(&mut self, v: i16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn i32(&mut self, v: i32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn f32(&mut self, v: f32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn raw(&mut self, v: &[u8]) -> &mut Self {
        self.0.extend_from_slice(v);
        self
    }
    /// fixed-size, NUL-padded name field
    fn name16(&mut self, v: &str) -> &mut Self {
        let mut n = [0u8; 16];
        n[..v.len()].copy_from_slice(v.as_bytes());
        self.0.extend_from_slice(&n);
        self
    }
    /// node/clipnode child in the dialect's width; `v` is already encoded
    /// (>= 0 node index, < 0 the leaf/contents form)
    fn child(&mut self, long: bool, v: i32) -> &mut Self {
        if long {
            self.i32(v)
        } else {
            self.u16((v & 0xffff) as u16)
        }
    }
}

const NUM_PLANES: i32 = 4;
const NUM_VERTEXES: i32 = 8;
const NUM_EDGES: i32 = 6;
const NUM_SURFEDGES: i32 = 12;
const NUM_TEXINFO: i32 = 4;
const NUM_FACES: i32 = 4;
const NUM_MARKSURFACES: i32 = 6;
const NUM_LEAFS: i32 = 5;
const NUM_NODES: i32 = 2;
const NUM_CLIPNODES: i32 = 3;
const NUM_SUBMODELS: i32 = 2;
const NUM_MIPTEX: i32 = 5;
/// 16x16: 256 + 64 + 16 + 4
const MIP_BYTES: usize = 340;

/// leaf index -> the child encoding both the short and long node formats use
fn leaf_child(leaf: i32) -> i32 {
    -1 - leaf
}

struct Bsp {
    data: Vec<u8>,
    /// byte length of the visibility lump (== `mod->visdata`)
    vis_len: usize,
    /// byte length of the expanded lighting block
    light_len: usize,
    /// the entity lump exactly as it appears in the file, NUL included
    entities: Vec<u8>,
}

fn planes_lump() -> Vec<u8> {
    let mut b = Buf::default();
    for (normal, dist, kind) in [
        ([1.0f32, 0.0, 0.0], 16.0f32, PLANE_X),
        ([0.0, 1.0, 0.0], -8.0, PLANE_Y),
        ([0.0, 0.0, 1.0], 0.0, PLANE_Z),
        ([0.6, 0.8, 0.0], 3.5, PLANE_ANYX),
    ] {
        for n in normal {
            b.f32(n);
        }
        b.f32(dist).i32(kind);
    }
    b.0
}

fn vertexes_lump() -> Vec<u8> {
    let mut b = Buf::default();
    for i in 0..NUM_VERTEXES {
        let f = i as f32;
        b.f32(f * 8.0 - 12.0).f32(f * -4.0 + 3.5).f32(f * 2.25);
    }
    b.0
}

fn edges_lump(d: Dialect) -> Vec<u8> {
    let mut b = Buf::default();
    for i in 0..NUM_EDGES {
        let (a, c) = ((i as u32) % 8, ((i as u32) * 3 + 1) % 8);
        if d.long_records() {
            b.u32(a).u32(c);
        } else {
            b.u16(a as u16).u16(c as u16);
        }
    }
    b.0
}

fn surfedges_lump() -> Vec<u8> {
    let mut b = Buf::default();
    for i in 0..NUM_SURFEDGES {
        let e = (i % (NUM_EDGES - 1)) + 1;
        b.i32(if i % 3 == 2 { -e } else { e });
    }
    b.0
}

/// 5 miptex entries: a plain texture, a `*` (water) one, a `sky` one, an
/// external (offsets[0] == 0, wad lookup fails in ctest) one, and a
/// `dataofs == -1` hole.
fn textures_lump(d: Dialect) -> Vec<u8> {
    let names = ["wtex", "*water", "sky1", "extern1"];
    let hdr = if d == Dialect::Q64 { 44 } else { 40 };
    let pal = if d == Dialect::Valve { 2 + 3 * 4 } else { 0 };
    let rec = hdr + MIP_BYTES + pal;
    let dir = 4 + 4 * NUM_MIPTEX as usize;

    let mut b = Buf::default();
    b.i32(NUM_MIPTEX);
    for i in 0..NUM_MIPTEX {
        if i == 4 {
            b.i32(-1);
        } else {
            b.i32((dir + rec * i as usize) as i32);
        }
    }
    for (i, name) in names.iter().enumerate() {
        b.name16(name).u32(16).u32(16);
        if d == Dialect::Q64 {
            b.u32(2 + i as u32);
        }
        // offsets[0] == 0 marks an external texture
        let base = hdr as u32;
        let o0 = if *name == "extern1" { 0 } else { base };
        b.u32(o0)
            .u32(base + 256)
            .u32(base + 256 + 64)
            .u32(base + 256 + 64 + 16);
        for p in 0..MIP_BYTES {
            b.u8((p as u8).wrapping_add(i as u8 * 7));
        }
        if d == Dialect::Valve {
            b.u16(4);
            for p in 0..12u8 {
                b.u8(p.wrapping_mul(5));
            }
        }
    }
    b.0
}

fn texinfo_lump() -> Vec<u8> {
    let mut b = Buf::default();
    // miptex 4 is the dataofs == -1 hole, 9 is out of range (TEX_MISSING)
    for (i, (miptex, flags)) in [(0, 0), (1, 0), (2, TEX_SPECIAL), (9, 0)]
        .into_iter()
        .enumerate()
    {
        let f = i as f32;
        for axis in 0..2 {
            let a = axis as f32;
            b.f32(0.5 + f)
                .f32(a - 0.25)
                .f32(f * 0.125)
                .f32(a * 16.0 - f);
        }
        b.i32(miptex).i32(flags);
    }
    assert_eq!(b.0.len(), NUM_TEXINFO as usize * 40);
    b.0
}

fn faces_lump(d: Dialect) -> Vec<u8> {
    let mut b = Buf::default();
    for (i, (planenum, side, texinfo, styles, lightofs)) in [
        (0i32, 0i32, 0i32, [0u8, 255, 255, 255], 0i32),
        (1, 1, 1, [0, 1, 255, 255], 12),
        (2, 0, 2, [255, 255, 255, 255], -1),
        (3, 0, 3, [2, 255, 255, 255], 24),
    ]
    .into_iter()
    .enumerate()
    {
        let firstedge = i as i32 * 3;
        if d.long_records() {
            b.i32(planenum)
                .i32(side)
                .i32(firstedge)
                .i32(3)
                .i32(texinfo)
                .raw(&styles)
                .i32(lightofs);
        } else {
            b.i16(planenum as i16)
                .i16(side as i16)
                .i32(firstedge)
                .i16(3)
                .i16(texinfo as i16)
                .raw(&styles)
                .i32(lightofs);
        }
    }
    b.0
}

fn lighting_lump() -> Vec<u8> {
    (0..96u16).map(|i| (i * 5 % 251) as u8).collect()
}

/// A well-formed compressed PVS block: one fully-literal row per leaf, so
/// decompression never runs off the end of a row.
fn visibility_lump() -> Vec<u8> {
    let row = ((NUM_LEAFS + 31) / 8) as usize;
    let mut v = Vec::new();
    for leaf in 0..NUM_LEAFS as usize {
        for byte in 0..row {
            v.push(((leaf * 17 + byte * 3) | 1) as u8);
        }
    }
    v
}

fn marksurfaces_lump(d: Dialect) -> Vec<u8> {
    let mut b = Buf::default();
    for i in 0..NUM_MARKSURFACES {
        let s = i % NUM_FACES;
        if d.long_records() {
            b.i32(s);
        } else {
            b.u16(s as u16);
        }
    }
    b.0
}

fn leafs_lump(d: Dialect) -> Vec<u8> {
    let row = (NUM_LEAFS + 31) / 8;
    let mut b = Buf::default();
    for i in 0..NUM_LEAFS {
        let contents = match i {
            0 => CONTENTS_SOLID,
            3 => -3, // CONTENTS_WATER
            _ => CONTENTS_EMPTY,
        };
        let visofs = if i == 0 { -1 } else { i * row };
        b.i32(contents).i32(visofs);
        match d.bsp2() {
            2 => {
                for j in 0..3 {
                    b.f32((i * 8 + j) as f32 - 40.0);
                }
                for j in 0..3 {
                    b.f32((i * 8 + j) as f32 + 40.0);
                }
            }
            _ => {
                for j in 0..3 {
                    b.i16((i * 8 + j) as i16 - 40);
                }
                for j in 0..3 {
                    b.i16((i * 8 + j) as i16 + 40);
                }
            }
        }
        let first = i % 3;
        let num = if i == 0 { 0 } else { 2 };
        if d.long_records() {
            b.u32(first as u32).u32(num as u32);
        } else {
            b.u16(first as u16).u16(num as u16);
        }
        for j in 0..4 {
            b.u8(((i * 4 + j) % 256) as u8);
        }
    }
    b.0
}

fn nodes_lump(d: Dialect) -> Vec<u8> {
    let nodes = [
        (0i32, [1i32, leaf_child(1)]),
        (3, [leaf_child(2), leaf_child(3)]),
    ];
    assert_eq!(nodes.len() as i32, NUM_NODES);
    let mut b = Buf::default();
    for (i, (planenum, children)) in nodes.into_iter().enumerate() {
        b.i32(planenum);
        for c in children {
            b.child(d.long_records(), c);
        }
        match d.bsp2() {
            2 => {
                for j in 0..3 {
                    b.f32((i * 4 + j) as f32 - 64.0);
                }
                for j in 0..3 {
                    b.f32((i * 4 + j) as f32 + 64.0);
                }
            }
            _ => {
                for j in 0..3 {
                    b.i16((i * 4 + j) as i16 - 64);
                }
                for j in 0..3 {
                    b.i16((i * 4 + j) as i16 + 64);
                }
            }
        }
        let (firstface, numfaces) = (i as u32 * 2, 2u32);
        if d.long_records() {
            b.u32(firstface).u32(numfaces);
        } else {
            b.u16(firstface as u16).u16(numfaces as u16);
        }
    }
    b.0
}

fn clipnodes_lump(d: Dialect, bad_planenum: bool) -> Vec<u8> {
    let clipnodes = [
        (0i32, [1i32, CONTENTS_SOLID]),
        (2, [2, CONTENTS_EMPTY]),
        (3, [CONTENTS_SOLID, CONTENTS_EMPTY]),
    ];
    assert_eq!(clipnodes.len() as i32, NUM_CLIPNODES);
    let mut b = Buf::default();
    for (i, (planenum, children)) in clipnodes.into_iter().enumerate() {
        let planenum = if bad_planenum && i == 1 {
            NUM_PLANES + 7
        } else {
            planenum
        };
        b.i32(planenum);
        for c in children {
            b.child(d.long_records(), c);
        }
    }
    b.0
}

fn models_lump() -> Vec<u8> {
    let models = [([0i32, 0, 1, 2], 3i32, 0i32, 2i32), ([1, 2, 0, 0], 1, 2, 2)];
    assert_eq!(models.len() as i32, NUM_SUBMODELS);
    let mut b = Buf::default();
    for (i, (headnode, visleafs, firstface, numfaces)) in models.into_iter().enumerate() {
        let f = i as f32;
        for j in 0..3 {
            b.f32(f * 32.0 - 96.0 + j as f32);
        }
        for j in 0..3 {
            b.f32(f * 32.0 + 96.0 + j as f32);
        }
        for j in 0..3 {
            b.f32(f * 4.0 + j as f32);
        }
        for h in headnode {
            b.i32(h);
        }
        b.i32(visleafs).i32(firstface).i32(numfaces);
    }
    b.0
}

fn entities_lump() -> Vec<u8> {
    let mut v =
        b"{\n\"classname\" \"worldspawn\"\n\"wad\" \"gfx/base.wad\"\n\"message\" \"m3\"\n}\n"
            .to_vec();
    v.push(0);
    v
}

/// Assembles the 15 lumps into a `dheader_t`-prefixed file image.
fn assemble(version: i32, lumps: [Vec<u8>; HEADER_LUMPS]) -> Vec<u8> {
    let mut data = vec![0u8; 4 + HEADER_LUMPS * 8];
    data[..4].copy_from_slice(&version.to_le_bytes());
    for (i, payload) in lumps.iter().enumerate() {
        while !data.len().is_multiple_of(4) {
            data.push(0);
        }
        let ofs = data.len() as i32;
        data.extend_from_slice(payload);
        let at = 4 + i * 8;
        data[at..at + 4].copy_from_slice(&ofs.to_le_bytes());
        data[at + 4..at + 8].copy_from_slice(&(payload.len() as i32).to_le_bytes());
    }
    data
}

fn build_bsp(d: Dialect) -> Bsp {
    build_bsp_with(d, false)
}

fn build_bsp_with(d: Dialect, bad_clip_planenum: bool) -> Bsp {
    let entities = entities_lump();
    let lighting = lighting_lump();
    let visibility = visibility_lump();
    let mut lumps: [Vec<u8>; HEADER_LUMPS] = Default::default();
    lumps[LUMP_ENTITIES] = entities.clone();
    lumps[LUMP_PLANES] = planes_lump();
    lumps[LUMP_TEXTURES] = textures_lump(d);
    lumps[LUMP_VERTEXES] = vertexes_lump();
    lumps[LUMP_VISIBILITY] = visibility.clone();
    lumps[LUMP_NODES] = nodes_lump(d);
    lumps[LUMP_TEXINFO] = texinfo_lump();
    lumps[LUMP_FACES] = faces_lump(d);
    lumps[LUMP_LIGHTING] = lighting.clone();
    lumps[LUMP_CLIPNODES] = clipnodes_lump(d, bad_clip_planenum);
    lumps[LUMP_LEAFS] = leafs_lump(d);
    lumps[LUMP_MARKSURFACES] = marksurfaces_lump(d);
    lumps[LUMP_EDGES] = edges_lump(d);
    lumps[LUMP_SURFEDGES] = surfedges_lump();
    lumps[LUMP_MODELS] = models_lump();

    let light_len = match d {
        Dialect::Q64 => lighting.len() / 2 * 3,
        Dialect::Valve => lighting.len(),
        _ => lighting.len() * 3,
    };
    Bsp {
        data: assemble(d.version(), lumps),
        vis_len: visibility.len(),
        light_len,
        entities,
    }
}

/// Reads lump `i` out of an assembled file image.
fn lump_of(data: &[u8], i: usize) -> LumpT {
    let at = 4 + i * 8;
    LumpT {
        fileofs: i32::from_le_bytes(data[at..at + 4].try_into().unwrap()),
        filelen: i32::from_le_bytes(data[at + 4..at + 8].try_into().unwrap()),
    }
}

// ---------------------------------------------------------------------------
// per-side driver

const MAP_NAME: &str = "maps/test.bsp";

fn new_model(name: &str, version: i32) -> Box<QModel> {
    // SAFETY: QModel is a #[repr(C)] mirror of a C struct the engine itself
    // zero-initializes; all-zero is a valid (null-pointer) value for it
    let mut m: Box<QModel> = Box::new(unsafe { core::mem::zeroed() });
    assert!(name.len() < MAX_QPATH);
    for (i, c) in name.bytes().enumerate() {
        m.name[i] = c as c_char;
    }
    m.bspversion = version;
    m
}

/// Everything one side produces from a full brush load.
struct Loaded {
    /// the main model plus every `Mod_FindName` clone, in pool order
    snaps: Vec<Snapshot>,
    con_log: Vec<String>,
    error: Option<String>,
}

/// Drives one side through `Mod_LoadBrushModel`'s call order. Caller must hold
/// [`ctfs::lock`] and have mounted the fixture dir on both sides.
fn load_side(side: Side, bsp: &Bsp, sv_modelname: &std::ffi::CStr) -> Loaded {
    let s = seam(side);
    let d = bsp.data.clone();
    let base = d.as_ptr() as *mut u8;
    let mut model = new_model(MAP_NAME, i32::from_le_bytes(d[..4].try_into().unwrap()));
    let m: *mut QModel = &raw mut *model;
    let bsp2 = match model.bspversion {
        v if v == BSP2VERSION_2PSB => 1,
        v if v == BSP2VERSION_BSP2 => 2,
        _ => 0,
    };
    let lump = |i: usize| lump_of(&d, i);
    let lens = BlobLens {
        visdata: bsp.vis_len,
        lightdata: bsp.light_len,
    };

    // SAFETY: ctest_set_sv_modelname copies into the stub's `sv` global,
    // which only the model_parse seam reads, under the fs lock
    unsafe {
        ctest_set_sv_modelname(sv_modelname.as_ptr());
        ctest_mod_pool_reset();
    }
    ctfs::clear_logs();

    let mut error = None;
    // SAFETY: every call below gets the live model, the file image that
    // outlives it, and a lump read out of that image's own header
    unsafe {
        (s.vertexes)(m, base, &lump(LUMP_VERTEXES));
        (s.edges)(m, base, &lump(LUMP_EDGES), bsp2);
        (s.surfedges)(m, base, &lump(LUMP_SURFEDGES));
        (s.entities)(m, base, &lump(LUMP_ENTITIES));
        (s.textures)(m, base, &lump(LUMP_TEXTURES), core::ptr::null_mut());
        ctest_fill_dummy_textures(m);
        (s.lighting)(m, base, &lump(LUMP_LIGHTING));
        (s.planes)(m, base, &lump(LUMP_PLANES));
        (s.texinfo)(m, base, &lump(LUMP_TEXINFO));
        (s.faces)(m, base, &lump(LUMP_FACES), bsp2 != 0);
        for i in 0..(*m).numsurfaces {
            (s.extents)(m, (*m).surfaces.offset(i as isize));
        }
        error = error.or(seam_marksurfaces(
            side,
            m,
            base,
            &lump(LUMP_MARKSURFACES),
            bsp2,
        ));
        (s.visibility)(m, base, &lump(LUMP_VISIBILITY));
        error = error.or(seam_leafs(side, m, base, &lump(LUMP_LEAFS), bsp2));
        (s.nodes)(m, base, &lump(LUMP_NODES), bsp2);
        error = error.or(seam_clipnodes(
            side,
            m,
            base,
            &lump(LUMP_CLIPNODES),
            bsp2 != 0,
        ));
        (s.submodels)(m, base, &lump(LUMP_MODELS));
        (s.make_hull0)(m);
        (*m).numframes = 2;
        error = error.or(seam_setup_submodels(side, m, sv_modelname));
    }

    let mut snaps = Vec::new();
    // SAFETY: the model and every pool clone stay alive until the snapshot is
    // taken; `lens` describes the blobs the loaders allocated
    unsafe {
        snaps.push(snapshot(m, lens));
        for i in 0..ctest_mod_pool_len() {
            snaps.push(snapshot(ctest_mod_pool_get(i), lens));
        }
    }
    Loaded {
        snaps,
        con_log: ctfs::con_log(),
        error,
    }
}

/// Pulls one `key = value` line out of a snapshot, so a case can assert the
/// fixture actually took the branch it means to cover.
fn field<'a>(snap: &'a Snapshot, key: &str) -> &'a str {
    let prefix = format!("{key} = ");
    snap.lines
        .iter()
        .find(|l| l.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no `{key}` line in snapshot"))
}

fn compare(what: &str, c: &Loaded, r: &Loaded) {
    assert_eq!(c.error, r.error, "{what}: Host_Error parity");
    assert_eq!(c.con_log, r.con_log, "{what}: console log parity");
    assert_eq!(
        c.snaps.len(),
        r.snaps.len(),
        "{what}: model count (main + Mod_FindName clones)"
    );
    for (i, (a, b)) in c.snaps.iter().zip(r.snaps.iter()).enumerate() {
        a.assert_eq(b, &format!("{what} model {i}"));
    }
}

// ---------------------------------------------------------------------------
// fixture filesystem

static SETUP: Once = Once::new();

/// Shared fixture dir mounted as a searchpath on both sides. Caller must hold
/// [`ctfs::lock`]. The loaders always probe for `.lit`/`.ent` files, so the fs
/// has to be live even for the cases that supply none.
fn file_dir() -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("quake-ctest-bsp-{}", std::process::id()));
    let dir = root.join("bspgame");
    SETUP.call_once(|| {
        std::fs::create_dir_all(dir.join("maps")).unwrap();
        for side in ctfs::BOTH {
            ctfs::setup(side, &[&root], 0, c"bspgame");
        }
    });
    dir
}

/// Removes every fixture file the external-file cases may have written, so
/// the base cases see a bare gamedir.
fn clear_maps_dir() {
    let dir = file_dir().join("maps");
    for e in std::fs::read_dir(&dir).unwrap() {
        std::fs::remove_file(e.unwrap().path()).unwrap();
    }
}

// ---------------------------------------------------------------------------
// cases

#[test]
fn bsp_parity_all_dialects() {
    let _g = ctfs::lock();
    let _ = file_dir();
    clear_maps_dir();
    for d in DIALECTS {
        let bsp = build_bsp(d);
        let c = load_side(Side::C, &bsp, c"maps/other.bsp");
        let r = load_side(Side::Rust, &bsp, c"maps/other.bsp");
        compare(&format!("{d:?}"), &c, &r);
        assert!(
            c.error.is_none(),
            "{d:?}: unexpected Host_Error {:?}",
            c.error
        );
        assert!(c.snaps.len() == 2, "{d:?}: expected one submodel clone");
        // guards against a snapshot that silently stops walking the graph
        assert!(
            c.snaps[0].lines.len() > 300 && c.snaps[1].lines.len() > 200,
            "{d:?}: snapshot truncated ({} / {} lines)",
            c.snaps[0].lines.len(),
            c.snaps[1].lines.len()
        );
    }
}

/// The `i > 0 || mod->name != sv.modelname` branch in Mod_SetupSubmodels:
/// naming the model as the server's current map skips the clipbox copy for
/// submodel 0.
#[test]
fn worldmodel_clipbox_branch_parity() {
    let _g = ctfs::lock();
    let _ = file_dir();
    clear_maps_dir();
    let bsp = build_bsp(Dialect::Bsp29);
    let name = std::ffi::CString::new(MAP_NAME).unwrap();
    let c = load_side(Side::C, &bsp, &name);
    let r = load_side(Side::Rust, &bsp, &name);
    compare("worldmodel clipbox", &c, &r);
}

/// External `.lit` colored lighting: the accepted file plus each of the three
/// rejection messages.
#[test]
fn lit_parity() {
    let _g = ctfs::lock();
    let dir = file_dir();
    let bsp = build_bsp(Dialect::Bsp29);
    let samples = lighting_lump().len();

    let good = {
        let mut v = b"QLIT".to_vec();
        v.extend_from_slice(&1i32.to_le_bytes());
        v.extend((0..samples * 3).map(|i| (i * 7 % 253) as u8));
        v
    };
    let mut short_body = good.clone();
    short_body.truncate(good.len() - 3);
    let mut bad_version = good.clone();
    bad_version[4..8].copy_from_slice(&7i32.to_le_bytes());

    clear_maps_dir();
    let baseline = load_side(Side::C, &bsp, c"maps/other.bsp");
    let plain = field(&baseline.snaps[0], "lightdata").to_string();

    for (label, body, want) in [
        ("ok", good.clone(), None),
        ("wrong size", short_body, Some("Outdated .lit file")),
        (
            "bad version",
            bad_version,
            Some("Unknown .lit file version (7)"),
        ),
        (
            "not qlit",
            b"BLIT\x01\x00\x00\x00junk".to_vec(),
            Some("Corrupt .lit file"),
        ),
        (
            "truncated header",
            b"QLI".to_vec(),
            Some("Corrupt .lit file"),
        ),
    ] {
        clear_maps_dir();
        std::fs::write(dir.join("maps/test.lit"), &body).unwrap();
        let c = load_side(Side::C, &bsp, c"maps/other.bsp");
        let r = load_side(Side::Rust, &bsp, c"maps/other.bsp");
        compare(&format!("lit {label}"), &c, &r);
        match want {
            // the accepted file must actually replace the expanded white light
            None => assert_ne!(
                field(&c.snaps[0], "lightdata"),
                plain,
                "lit {label}: the .lit file was not picked up"
            ),
            Some(msg) => assert!(
                c.con_log.iter().any(|l| l.contains(msg)),
                "lit {label}: expected {msg:?} in {:?}",
                c.con_log
            ),
        }
    }
    clear_maps_dir();
}

/// External entity files: the CRC-versioned name wins over the plain one, and
/// `external_ents 0` ignores both.
#[test]
fn ent_parity() {
    let _g = ctfs::lock();
    let dir = file_dir();
    let bsp = build_bsp(Dialect::Bsp29);
    let crc = quake_ctest::c_crc_block(&bsp.entities[..bsp.entities.len() - 1]);
    let versioned = format!("maps/test@{crc:04x}.ent");

    type EntCase<'a> = (&'a str, Vec<(String, &'a [u8])>, f32);
    let cases: [EntCase; 4] = [
        ("none", vec![], 1.0),
        (
            "plain",
            vec![("maps/test.ent".to_string(), b"{ plain ent }\n".as_slice())],
            1.0,
        ),
        (
            "versioned wins",
            vec![
                (versioned.clone(), b"{ versioned ent }\n".as_slice()),
                ("maps/test.ent".to_string(), b"{ plain ent }\n".as_slice()),
            ],
            1.0,
        ),
        (
            "disabled",
            vec![(versioned.clone(), b"{ versioned ent }\n".as_slice())],
            0.0,
        ),
    ];

    let mut embedded = String::new();
    for (label, files, ents) in cases {
        clear_maps_dir();
        for (name, body) in &files {
            std::fs::write(dir.join(name), body).unwrap();
        }
        // SAFETY: the stub cvar is shared by both sides, written under the lock
        unsafe { ctest_set_external_ents(ents) };
        let c = load_side(Side::C, &bsp, c"maps/other.bsp");
        let r = load_side(Side::Rust, &bsp, c"maps/other.bsp");
        compare(&format!("ent {label}"), &c, &r);

        let got = field(&c.snaps[0], "entities");
        match label {
            "none" => embedded = got.to_string(),
            "plain" => assert!(got.contains("plain ent"), "ent {label}: {got}"),
            "versioned wins" => assert!(got.contains("versioned ent"), "ent {label}: {got}"),
            _ => assert_eq!(got, embedded, "ent {label}: should use the embedded lump"),
        }
    }
    // SAFETY: restore the stub cvar default for the other cases
    unsafe { ctest_set_external_ents(1.0) };
    clear_maps_dir();
}

/// `Mod_DecompressVis`: the run-length expansion plus the cache-growth path
/// (a second, larger model must reallocate the shared scratch buffer).
#[test]
fn decompress_vis_parity() {
    let _g = ctfs::lock();
    let _ = file_dir();
    // (numleafs, compressed row): the row must decompress to exactly
    // (numleafs + 31) / 8 bytes, which is what the C loop bounds itself by.
    let cases: [(i32, Vec<u8>); 4] = [
        (33, vec![0xff, 0x00, 0x03, 0x7f, 0x01, 0x80, 0x40, 0x22]),
        (33, vec![0x00, 0x08]),
        (129, {
            let mut v = vec![0x00, 0x10];
            v.extend([0xa5u8; 4]);
            v
        }),
        (33, vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]),
    ];

    for (i, (numleafs, row)) in cases.into_iter().enumerate() {
        let mut out = Vec::new();
        for side in ctfs::BOTH {
            let s = seam(side);
            let mut model = new_model(MAP_NAME, BSPVERSION);
            model.numleafs = numleafs;
            let mut data = row.clone();
            model.visdata = data.as_mut_ptr();
            ctfs::clear_logs();
            let want = ((numleafs + 31) / 8) as usize;
            // SAFETY: `data` outlives the call; the shim writes into its own
            // static scratch buffer and returns it
            let got = unsafe {
                let p = (s.decompress_vis)(data.as_mut_ptr(), &raw mut *model);
                core::slice::from_raw_parts(p, want).to_vec()
            };
            out.push((got, ctfs::con_log()));
        }
        assert_eq!(out[0], out[1], "decompress_vis case {i}");
    }
}

/// Host_Error parity for the two reachable brush-loader messages, and the
/// Sys_Error variant of the same marksurfaces check on non-BSP2 maps.
#[test]
fn host_error_parity() {
    let _g = ctfs::lock();
    let _ = file_dir();
    clear_maps_dir();

    // clipnodes: planenum out of bounds (identical on every dialect)
    for d in [Dialect::Bsp29, Dialect::Bsp2] {
        let bsp = build_bsp_with(d, true);
        let c = load_side(Side::C, &bsp, c"maps/other.bsp");
        let r = load_side(Side::Rust, &bsp, c"maps/other.bsp");
        assert_eq!(
            c.error.as_deref(),
            Some("Mod_LoadClipnodes: planenum out of bounds"),
            "{d:?}: expected the C bounds check to fire"
        );
        assert_eq!(c.error, r.error, "{d:?}: clipnode Host_Error parity");
        assert_eq!(c.con_log, r.con_log, "{d:?}: clipnode console parity");
    }

    // marksurfaces: bad surface number is Host_Error under BSP2 only
    let bsp = build_bsp(Dialect::Bsp2);
    let mut data = bsp.data.clone();
    let l = lump_of(&data, LUMP_MARKSURFACES);
    let at = l.fileofs as usize;
    data[at..at + 4].copy_from_slice(&(NUM_FACES + 3).to_le_bytes());
    let broken = Bsp { data, ..bsp };
    let c = load_side(Side::C, &broken, c"maps/other.bsp");
    let r = load_side(Side::Rust, &broken, c"maps/other.bsp");
    assert_eq!(
        c.error.as_deref(),
        Some("Mod_LoadMarksurfaces: bad surface number")
    );
    assert_eq!(c.error, r.error, "marksurfaces Host_Error parity");
}

/// Sys_Error parity: a funny lump size aborts on both sides with the same
/// message (Rust out-of-process, per PLAN.md §4.3).
#[test]
fn sys_error_parity() {
    let _g = ctfs::lock();
    let _ = file_dir();
    clear_maps_dir();

    for case in ["edges", "planes"] {
        let bsp = build_bsp(Dialect::Bsp29);
        let mut model = new_model(MAP_NAME, BSPVERSION);
        let mut data = bsp.data.clone();
        let base = data.as_mut_ptr();
        let l = funny_lump(&data, case);
        let c_msg = ctfs::catch_sys_error(|| {
            // SAFETY: the model and file image outlive the call; the lump has
            // a deliberately unaligned length, so the C aborts inside
            unsafe {
                match case {
                    "edges" => c_ref_Mod_LoadEdges(&raw mut *model, base, &l, 0),
                    _ => c_ref_Mod_LoadPlanes(&raw mut *model, base, &l),
                }
            }
        });
        let rust_msg = ctfs::rust_fatal_in_child("rust_fatal_child", case, &[]);
        assert!(c_msg.is_some(), "{case}: the C side did not Sys_Error");
        assert_eq!(c_msg, rust_msg, "{case}: Sys_Error message parity");
    }
}

/// Trims one byte off a lump so its length is no longer a multiple of the
/// record size.
fn funny_lump(data: &[u8], case: &str) -> LumpT {
    let mut l = lump_of(
        data,
        match case {
            "edges" => LUMP_EDGES,
            _ => LUMP_PLANES,
        },
    );
    l.filelen -= 1;
    l
}

/// Child-process half of [`sys_error_parity`]: runs the Rust shim with the
/// Sys_Error trap unarmed so the stub prints and aborts. A no-op unless
/// CTEST_FATAL_CASE selects a case.
#[test]
fn rust_fatal_child() {
    let Some(case) = ctfs::fatal_child_case() else {
        return;
    };
    let _g = ctfs::lock();
    let bsp = build_bsp(Dialect::Bsp29);
    let mut model = new_model(MAP_NAME, BSPVERSION);
    let mut data = bsp.data.clone();
    let base = data.as_mut_ptr();
    let l = funny_lump(&data, &case);
    // SAFETY: as the C side above; this call is expected to abort
    unsafe {
        match case.as_str() {
            "edges" => quake_rs::model_parse::Mod_LoadEdges(&raw mut *model, base, &l, 0),
            _ => quake_rs::model_parse::Mod_LoadPlanes(&raw mut *model, base, &l),
        }
    }
    unreachable!("{case}: the Rust shim returned instead of calling Sys_Error");
}

/// The external-vis trio: `maps/<name>.vis` supplies visdata and a full leaf
/// lump, replacing the in-BSP ones.
#[test]
fn external_vis_parity() {
    let _g = ctfs::lock();
    let dir = file_dir();
    clear_maps_dir();

    let vis = visibility_lump();
    let leafs = leafs_lump(Dialect::Bsp29);
    let mut body = Vec::new();
    body.extend_from_slice(&(vis.len() as i32).to_le_bytes());
    body.extend_from_slice(&vis);
    body.extend_from_slice(&(leafs.len() as i32).to_le_bytes());
    body.extend_from_slice(&leafs);

    // vispatch: 32-byte map name + payload length per entry. A non-matching
    // entry goes first so the skip loop is exercised; the loaders match on
    // COM_SkipPath(mod->name), extension included.
    let entry = |name: &str, payload: &[u8]| {
        let mut v = [0u8; 32];
        v[..name.len()].copy_from_slice(name.as_bytes());
        let mut out = v.to_vec();
        out.extend_from_slice(&(payload.len() as i32).to_le_bytes());
        out.extend_from_slice(payload);
        out
    };
    let mut file = entry("other.bsp", &[0xa5u8; 8]);
    file.extend_from_slice(&entry("test.bsp", &body));
    std::fs::write(dir.join("maps/test.vis"), &file).unwrap();

    let bsp = build_bsp(Dialect::Bsp29);
    let mut out = Vec::new();
    for side in ctfs::BOTH {
        let s = seam(side);
        let mut d = bsp.data.clone();
        let base = d.as_mut_ptr();
        let mut model = new_model(MAP_NAME, BSPVERSION);
        let m: *mut QModel = &raw mut *model;
        ctfs::clear_logs();
        // SAFETY: the file image and model outlive every call; the marksurface
        // and face lumps are loaded first because the leaf loader indexes them
        let (err, snap) = unsafe {
            (s.textures)(m, base, &lump_of(&d, LUMP_TEXTURES), core::ptr::null_mut());
            ctest_fill_dummy_textures(m);
            (s.texinfo)(m, base, &lump_of(&d, LUMP_TEXINFO));
            (s.faces)(m, base, &lump_of(&d, LUMP_FACES), false);
            let _ = seam_marksurfaces(side, m, base, &lump_of(&d, LUMP_MARKSURFACES), 0);
            let f = (s.find_vis_external)(m, c"test".as_ptr());
            assert!(!f.is_null(), "{side:?}: external .vis not found");
            (*m).visdata = (s.load_vis_external)(f);
            assert!(!(*m).visdata.is_null(), "{side:?}: external visdata");
            let err = seam_leafs_external(side, m, f);
            fclose(f);
            (
                err,
                snapshot(
                    m,
                    BlobLens {
                        visdata: vis.len(),
                        lightdata: 0,
                    },
                ),
            )
        };
        out.push((err, snap, ctfs::con_log()));
    }
    let (c, r) = (&out[0], &out[1]);
    assert_eq!(c.0, r.0, "external vis Host_Error parity");
    assert_eq!(c.2, r.2, "external vis console parity");
    c.1.assert_eq(&r.1, "external vis");
    clear_maps_dir();
}
