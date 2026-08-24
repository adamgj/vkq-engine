//! Phase 3 M6: the threading proof for the formats/image seam (AC8, R5,
//! ADR-016 — "Rust code scheduled onto C task workers must be `Send`-safe pure
//! functions over byte slices").
//!
//! `gl_model.c` submits four indexed tasks during a load, and every one of them
//! reaches Rust once `-Duse_rust_formats` / `-Duse_rust_image` are on:
//!
//! | site | task | Rust reached |
//! |---|---|---|
//! | gl_model.c:1019 | `Mod_LoadTextureTask` | `Image_Decode{PCX,LMP}` via `Image_LoadImage` |
//! | gl_model.c:1201 | `Mod_CalcSurfaceExtentsTask` | `CalcSurfaceExtents` |
//! | gl_model.c:1997 | `Mod_LoadSkinTask` | `Image_Decode*` via `Image_LoadImage` |
//! | gl_model.c:2205 | `Mod_LoadMDXSkinTask` | `Image_Decode*` via `Image_LoadImage` |
//!
//! The demo harness cannot carry the first two: both submits sit behind
//! `if (!no_rendering)` and `-headless` sets `no_rendering = true`
//! (harness.c:69). So `CalcSurfaceExtents` — the only Rust the unreachable
//! pair adds — is proven here instead, sharded across threads with the same
//! `(limit + workers - 1) / workers` partitioning `tasks.c:532` uses.
//!
//! A fifth, per-frame worker entry point the milestone plan never listed is
//! covered too: `Mod_DecompressVis` runs under `R_MarkSurfacesPrepare`
//! (r_world.c:1037), which is a task.
//!
//! Not covered here, by design: `Mod_ParseAliasModel` / `Mod_LoadMD3Model` /
//! `Mod_LoadMD5MeshModel` are *not* thread-safe on either side — they share the
//! `stverts`/`triangles`/`poseverts` scratch with `gl_mesh.c`. Racing them would
//! test a contract the engine does not offer; the contract itself (one parse at
//! a time, never on a worker) is mechanized by `assert (!Tasks_IsWorker ())` in
//! `Mod_LoadModel`, past its `!needload` early return.

use core::ffi::{c_char, c_int, c_uint};
use quake_ctest::fs as ctfs;
use quake_ctest::fs::Side;
use quake_types::bspfile::TEX_SPECIAL;
use quake_types::model_mem::{MEdge, MSurface, MTexInfo, MVertex, QModel};
use std::sync::{Mutex, Once};

#[path = "support/image_fixture.rs"]
mod image_fixture;
use image_fixture::{build_lmp, build_pcx};

extern "C" {
    fn c_ref_CalcSurfaceExtents(m: *mut QModel, s: *mut MSurface);
    fn c_ref_Mod_DecompressVis(in_: *mut u8, model: *mut QModel) -> *mut u8;
}

/// A raw pointer handed to worker threads. Sound only for the specific sharing
/// each use site argues for; every one below either points at immutable data
/// for the duration of the scope, or at an element no other shard touches —
/// which is exactly the contract `Task_AllocateAssignIndexedFuncAndSubmit`
/// gives a C task body.
#[derive(Clone, Copy)]
struct Shared<T>(*mut T);
// SAFETY: see the type's doc comment; each construction site states which of
// the two arguments applies.
unsafe impl<T> Send for Shared<T> {}

impl<T> Shared<T> {
    /// Unwraps inside the worker closure. Taking `self` by value is what makes
    /// the closure capture the wrapper rather than the bare pointer field
    /// (edition-2021 closures capture the narrowest path they use).
    fn ptr(self) -> *mut T {
        self.0
    }
}

const WORKER_COUNTS: [usize; 4] = [2, 3, 8, 17];

// ---------------------------------------------------------------------------
// CalcSurfaceExtents (dispatch site 2)

const NUM_SURFACES: usize = 2048;

/// Sentinel written before every pass so an unwritten surface is a failure
/// rather than a stale pass.
const UNWRITTEN: ([i16; 2], [i16; 2]) = ([i16::MIN, i16::MIN], [i16::MIN, i16::MIN]);

/// The inputs `CalcSurfaceExtents` reads (`m->surfedges`, `m->edges`,
/// `m->vertexes`, `s->texinfo`) and the surfaces it writes.
///
/// Hand-built rather than parsed out of a synthetic BSP: dialect coverage is
/// `bsp_differential`'s job (AC4), and this fixture needs the opposite
/// emphasis — many surfaces with *pairwise distinct* results, so that a shared
/// piece of mutable state inside the port shows up as cross-talk between
/// shards instead of hiding behind identical answers.
///
/// Every non-special surface stays well inside the `extents > 2000`
/// `Sys_Error` limit: a fatal path must not be reachable from a worker thread
/// here, because the stub's `Sys_Error` trap is a `longjmp` and the console log
/// it would write is not synchronized.
struct ExtentsFixture {
    // kept alive for the raw pointers in `model` / `surfaces`
    _vertexes: Vec<MVertex>,
    _edges: Vec<MEdge>,
    _surfedges: Vec<i32>,
    _texinfo: Vec<MTexInfo>,
    surfaces: Vec<MSurface>,
    model: Box<QModel>,
}

impl ExtentsFixture {
    fn new() -> Self {
        // 4 texinfo variants; the last one carries TEX_SPECIAL, which skips the
        // extents size check, so its surfaces can be huge
        let vecs: [[[f32; 4]; 2]; 4] = [
            [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]],
            [[0.5, 0.25, 0.0, 3.5], [0.0, 0.5, 0.25, -7.25]],
            [[0.125, -0.75, 0.375, 11.0], [-0.625, 0.0, 0.5, -2.5]],
            [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]],
        ];
        let mut texinfo: Vec<MTexInfo> = vecs
            .iter()
            .enumerate()
            .map(|(i, v)| MTexInfo {
                vecs: *v,
                texture: core::ptr::null_mut(),
                flags: if i == 3 { TEX_SPECIAL } else { 0 },
                tex_idx: i as i32,
            })
            .collect();

        let mut vertexes: Vec<MVertex> = Vec::with_capacity(NUM_SURFACES * 4);
        // edge 0 is left unused so every surfedge can be negated (the `e < 0`
        // arm reads edges[-e].v[1], which -0 could not select)
        let mut edges: Vec<MEdge> = vec![MEdge {
            v: [0, 0],
            cachededgeoffset: 0,
        }];
        let mut surfedges: Vec<i32> = Vec::with_capacity(NUM_SURFACES * 4);
        let mut surfaces: Vec<MSurface> = Vec::with_capacity(NUM_SURFACES);

        for s in 0..NUM_SURFACES {
            let special = s % 64 == 63;
            let ti = if special { 3 } else { s % 3 };

            // pairwise-distinct geometry; fractional offsets so floor()/ceil()
            // land differently from surface to surface
            let (w, h) = if special {
                (4096.0f32, 2048.0f32)
            } else {
                (16.0 + (s % 97) as f32, 16.0 + (s * 7 % 89) as f32)
            };
            let ox = (s % 31) as f32 * 13.0 + (s % 5) as f32 * 0.37;
            let oy = (s % 17) as f32 * 11.0 - (s % 7) as f32 * 0.61;
            let oz = (s % 11) as f32;

            let base = vertexes.len() as u32;
            for (dx, dy) in [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)] {
                vertexes.push(MVertex {
                    position: [ox + dx, oy + dy, oz],
                });
            }

            let first_edge = edges.len() as i32;
            for k in 0..4u32 {
                edges.push(MEdge {
                    v: [base + k, base + (k + 1) % 4],
                    cachededgeoffset: 0,
                });
            }

            let firstsurfedge = surfedges.len() as i32;
            for k in 0..4 {
                let e = first_edge + k;
                // alternate the sign so both surfedge arms run on every shard
                surfedges.push(if (s + k as usize).is_multiple_of(2) {
                    e
                } else {
                    -e
                });
            }

            // SAFETY: MSurface is #[repr(C)] and all-zero is a valid value for
            // every field (its pointers become null); only the fields set below
            // are read by CalcSurfaceExtents.
            let mut surf: MSurface = unsafe { core::mem::zeroed() };
            surf.firstedge = firstsurfedge;
            surf.numedges = 4;
            surf.texinfo = texinfo.as_mut_ptr().wrapping_add(ti);
            surfaces.push(surf);
        }

        // SAFETY: QModel is #[repr(C)] and all-zero is a valid value for every
        // field; the loaders' other inputs are unread by CalcSurfaceExtents.
        let mut model: Box<QModel> = Box::new(unsafe { core::mem::zeroed() });
        model.numvertexes = vertexes.len() as i32;
        model.vertexes = vertexes.as_mut_ptr();
        model.numedges = edges.len() as i32;
        model.edges = edges.as_mut_ptr();
        model.numsurfedges = surfedges.len() as i32;
        model.surfedges = surfedges.as_mut_ptr();
        model.numtexinfo = texinfo.len() as i32;
        model.texinfo = texinfo.as_mut_ptr();
        model.numsurfaces = surfaces.len() as i32;
        model.surfaces = surfaces.as_mut_ptr();

        Self {
            _vertexes: vertexes,
            _edges: edges,
            _surfedges: surfedges,
            _texinfo: texinfo,
            surfaces,
            model,
        }
    }

    fn arm(&mut self) {
        for s in &mut self.surfaces {
            s.texturemins = UNWRITTEN.0;
            s.extents = UNWRITTEN.1;
        }
    }

    fn results(&self) -> Vec<([i16; 2], [i16; 2])> {
        self.surfaces
            .iter()
            .map(|s| (s.texturemins, s.extents))
            .collect()
    }
}

type ExtentsFn = unsafe extern "C" fn(*mut QModel, *mut MSurface);

fn extents_serial(fix: &mut ExtentsFixture, f: ExtentsFn) {
    let m: *mut QModel = &mut *fix.model;
    for i in 0..fix.surfaces.len() {
        // SAFETY: the C ABI contract — `m`'s edge/vertex/surfedge arrays are
        // populated and the surface belongs to it
        unsafe { f(m, fix.surfaces.as_mut_ptr().add(i)) };
    }
}

/// `Mod_CalcSurfaceExtentsTask` under `Task_AllocateAssignIndexedFuncAndSubmit`,
/// reproduced: one contiguous shard of the surface array per worker.
fn extents_sharded(fix: &mut ExtentsFixture, f: ExtentsFn, workers: usize) {
    let count = fix.surfaces.len();
    let per_worker = count.div_ceil(workers);
    // SAFETY (Shared): `model` and its arrays are only read for the duration of
    // the scope; each shard writes only `surfaces[start..end]`, and the ranges
    // are disjoint by construction.
    let model = Shared(&mut *fix.model as *mut QModel);
    let surfaces = Shared(fix.surfaces.as_mut_ptr());
    std::thread::scope(|scope| {
        for w in 0..workers {
            scope.spawn(move || {
                let start = (w * per_worker).min(count);
                let end = ((w + 1) * per_worker).min(count);
                for i in start..end {
                    // SAFETY: as above; `i` is inside this shard alone
                    unsafe { f(model.ptr(), surfaces.ptr().add(i)) };
                }
            });
        }
    });
}

#[test]
fn extents_concurrent_matches_serial_and_c() {
    let _guard = ctfs::lock();
    ctfs::clear_logs();
    let mut fix = ExtentsFixture::new();

    fix.arm();
    extents_serial(&mut fix, c_ref_CalcSurfaceExtents);
    let expected = fix.results();
    assert!(
        expected.iter().all(|r| *r != UNWRITTEN),
        "the C reference must write every surface"
    );
    let distinct: std::collections::HashSet<_> = expected.iter().collect();
    assert!(
        distinct.len() > NUM_SURFACES / 2,
        "fixture must produce mostly-distinct results or cross-talk would hide: {} of {}",
        distinct.len(),
        NUM_SURFACES
    );

    fix.arm();
    extents_serial(&mut fix, quake_rs::model_parse::CalcSurfaceExtents);
    assert_eq!(expected, fix.results(), "Rust serial vs C serial");

    for workers in WORKER_COUNTS {
        for iteration in 0..8 {
            fix.arm();
            extents_sharded(&mut fix, quake_rs::model_parse::CalcSurfaceExtents, workers);
            assert_eq!(
                expected,
                fix.results(),
                "Rust sharded over {workers} workers (iteration {iteration})"
            );
        }
    }

    assert!(
        ctfs::con_log().is_empty(),
        "the extents fixture must not reach a diagnostic path: {:?}",
        ctfs::con_log()
    );
}

// ---------------------------------------------------------------------------
// Image_Decode{PCX,LMP} (dispatch sites 1, 3 and 4)

static IMAGE_SETUP: Once = Once::new();
/// `allocHandle` (stubs.c:361) scans a shared free-slot table, unlike the real
/// `Sys_FileOpenRead`, so the *open* is serialized here. The decode — the code
/// under test — is not. The decoder's own `COM_CloseFile` still runs
/// unserialized, which is sound because a slot is only ever marked free by the
/// thread that owns it, and `allocHandle`, the only reader of that flag, is
/// behind this lock.
static OPEN_LOCK: Mutex<()> = Mutex::new(());

fn image_dir() -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("quake-ctest-thr-{}", std::process::id()));
    let dir = root.join("thrgame");
    IMAGE_SETUP.call_once(|| {
        std::fs::create_dir_all(dir.join("gfx")).unwrap();
        for side in ctfs::BOTH {
            ctfs::setup(side, &[&root], 0, c"thrgame");
        }
    });
    dir
}

/// One open + decode, entirely on the calling thread — `com_filesize` is
/// `THREAD_LOCAL` (stubs.c:247, mirroring common.c:2120), so the open must
/// happen where the decode does. This is also why the engine's
/// `Image_LoadImage` opens on the worker rather than handing a handle over.
fn decode_one(side: Side, name: &std::ffi::CStr, pcx: bool, buf_len: usize) -> Vec<u8> {
    let mut handle: c_int = -1;
    let mut path_id: c_uint = 0;
    let size = {
        let _open = OPEN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: the side's searchpaths are mounted; out-params are valid
        unsafe { (ctfs::fns(side).open_file)(name.as_ptr(), &mut handle, &mut path_id) }
    };
    assert!(size >= 0, "fixture {name:?} must open on {side:?}");

    let decoder: unsafe extern "C" fn(c_int, *mut c_int, *mut c_int, *const c_char) -> *mut u8 =
        match (side, pcx) {
            (Side::C, true) => c_ref_image_decode_pcx,
            (Side::C, false) => c_ref_image_decode_lmp,
            (Side::Rust, true) => quake_rs::image_decode::Image_DecodePCX,
            (Side::Rust, false) => quake_rs::image_decode::Image_DecodeLMP,
        };
    let mut width: c_int = -1;
    let mut height: c_int = -1;
    // SAFETY: open handle at the resource start, valid out-pointers,
    // NUL-terminated name — the C ABI contract
    let data = unsafe { decoder(handle, &mut width, &mut height, name.as_ptr()) };
    assert!(!data.is_null(), "fixture {name:?} must decode on {side:?}");
    // SAFETY: the decoder returned a Mem_Alloc'd buffer of exactly the size its
    // C original allocates, which `buf_len` reproduces
    let bytes = unsafe { core::slice::from_raw_parts(data, buf_len) }.to_vec();
    // SAFETY: the buffer came from the stub Mem_Alloc inside the decoder
    unsafe { quake_c_sys::Mem_Free(data.cast()) };
    bytes
}

extern "C" {
    #[link_name = "c_ref_Image_DecodePCX"]
    fn c_ref_image_decode_pcx(h: c_int, w: *mut c_int, ht: *mut c_int, n: *const c_char)
        -> *mut u8;
    #[link_name = "c_ref_Image_DecodeLMP"]
    fn c_ref_image_decode_lmp(h: c_int, w: *mut c_int, ht: *mut c_int, n: *const c_char)
        -> *mut u8;
}

/// Rounds per worker. High enough that the window between a would-be shared
/// scratch buffer being filled and being copied out is actually hit: at 16
/// rounds a deliberately race-y decoder passed this test, at this count it
/// fails it (see the M6 mutation log).
const IMAGE_ROUNDS: usize = 4000;

/// The four fixtures the concurrent decode cycles through: (name, is_pcx,
/// allocation size). All are happy-path — a reject would write the
/// unsynchronized console log, or (for the PCX header gates) `Sys_Error`.
const IMAGE_FIXTURES: [(&std::ffi::CStr, bool, usize); 4] = [
    (c"gfx/thr_a.pcx", true, (8 * 6 + 1) * 4),
    (c"gfx/thr_b.pcx", true, (5 * 4 + 1) * 4),
    (c"gfx/thr_a.lmp", false, 8 * 6),
    (c"gfx/thr_b.lmp", false, 5 * 4),
];

#[test]
fn image_decode_concurrent_matches_serial_and_c() {
    let _guard = ctfs::lock();
    ctfs::clear_logs();
    let dir = image_dir();
    // thr_a: a literal row then five run-encoded rows (bytes_per_line == w, so
    // each row consumes exactly w pixels). thr_b: bytes_per_line == w + 1, so
    // every row spills one pixel into the next row's region and the last row
    // lands in the +1 padding slot the C decoder allocates.
    std::fs::write(
        dir.join("gfx/thr_a.pcx"),
        build_pcx(
            8,
            6,
            8,
            &[
                1, 2, 3, 4, 5, 6, 7, 8, 0xC8, 11, 0xC8, 13, 0xC8, 17, 0xC8, 19, 0xC8, 23,
            ],
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("gfx/thr_b.pcx"),
        build_pcx(5, 4, 6, &[0xC6, 21, 0xC6, 22, 0xC6, 23, 0xC6, 24]),
    )
    .unwrap();
    let px_a: Vec<u8> = (0..8 * 6u32).map(|i| (i * 3 % 251) as u8).collect();
    let px_b: Vec<u8> = (0..5 * 4u32).map(|i| (i * 17 % 241) as u8).collect();
    std::fs::write(dir.join("gfx/thr_a.lmp"), build_lmp(8, 6, &px_a)).unwrap();
    std::fs::write(dir.join("gfx/thr_b.lmp"), build_lmp(5, 4, &px_b)).unwrap();

    let expected: Vec<Vec<u8>> = IMAGE_FIXTURES
        .iter()
        .map(|(name, pcx, len)| decode_one(Side::C, name, *pcx, *len))
        .collect();
    for (i, a) in expected.iter().enumerate() {
        for b in &expected[i + 1..] {
            assert_ne!(a, b, "fixtures must decode to distinct bytes");
        }
    }
    for (i, (name, pcx, len)) in IMAGE_FIXTURES.iter().enumerate() {
        assert_eq!(
            expected[i],
            decode_one(Side::Rust, name, *pcx, *len),
            "Rust serial vs C serial for {name:?}"
        );
    }
    assert_eq!(
        ctfs::open_handle_count(),
        0,
        "decoders must close their handles"
    );
    let baseline = ctfs::con_log();
    assert!(
        baseline.is_empty(),
        "happy-path fixtures must not log: {baseline:?}"
    );

    for workers in WORKER_COUNTS {
        std::thread::scope(|scope| {
            for w in 0..workers {
                let expected = &expected;
                scope.spawn(move || {
                    for round in 0..IMAGE_ROUNDS {
                        let idx = (w + round) % IMAGE_FIXTURES.len();
                        let (name, pcx, len) = IMAGE_FIXTURES[idx];
                        assert_eq!(
                            expected[idx],
                            decode_one(Side::Rust, name, pcx, len),
                            "concurrent decode of {name:?} on worker {w}"
                        );
                    }
                });
            }
        });
    }
    assert_eq!(
        ctfs::open_handle_count(),
        0,
        "every concurrently opened handle must be closed"
    );
    assert!(
        ctfs::con_log().is_empty(),
        "concurrent decode must not log: {:?}",
        ctfs::con_log()
    );
}

// ---------------------------------------------------------------------------
// Mod_DecompressVis (the renderer's R_MarkSurfacesPrepare task)

/// `Mod_DecompressVis` owns a process-global scratch cache (`MOD_DECOMPRESSED`,
/// inherited from the C `mod_decompressed`), so this does **not** race two
/// callers — see the SAFETY note at the shim. What it does check is the part
/// that is new for Rust: the function runs correctly off the main thread, which
/// is where the engine actually calls it (`R_MarkSurfacesPrepare` is a task).
#[test]
fn decompress_vis_off_main_thread_matches_c() {
    let _guard = ctfs::lock();
    ctfs::clear_logs();

    // numleafs 200 -> row = 28 bytes. The stream must decode to *exactly* that
    // many bytes: the loop only terminates on `out - base >= row`, so a stream
    // that comes up short keeps reading past the input buffer on both sides,
    // and the two would then be comparing two different heaps' leftovers.
    // 1 + 3 + 2 + 10 + 2 + 8 + 2 = 28.
    let compressed: Vec<u8> = vec![
        0xAB, // literal
        0x00, 3, // zero run of 3
        0xCD, 0xEF, // two literals
        0x00, 10, // zero run of 10
        0x11, 0x22, // two literals
        0x00, 8, // zero run of 8
        0x33, 0x44, // two literals -> row complete
    ];
    #[rustfmt::skip]
    let compressed_expected: [u8; 28] = [
        0xAB, 0, 0, 0, 0xCD, 0xEF, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0x11, 0x22, 0, 0, 0, 0, 0, 0, 0, 0, 0x33, 0x44,
    ];
    let mut model: Box<QModel> = Box::new(
        // SAFETY: QModel is #[repr(C)] and all-zero is valid for every field
        unsafe { core::mem::zeroed() },
    );
    model.numleafs = 200;
    let row = ((model.numleafs + 31) / 8) as usize;

    let run = |f: unsafe extern "C" fn(*mut u8, *mut QModel) -> *mut u8,
               m: *mut QModel,
               input: Option<&[u8]>| {
        let mut buf = input.map(<[u8]>::to_vec);
        let p = buf
            .as_mut()
            .map_or(core::ptr::null_mut(), |b| b.as_mut_ptr());
        // SAFETY: `p` is NULL or a row inside a live buffer, `m` a live model
        let out = unsafe { f(p, m) };
        assert!(!out.is_null());
        // SAFETY: the shim returns its `row`-byte scratch buffer
        unsafe { core::slice::from_raw_parts(out, row) }.to_vec()
    };

    for input in [None, Some(compressed.as_slice())] {
        let expected = run(c_ref_Mod_DecompressVis, &mut *model, input);
        // pinning the C answer against a literal is what proves the fixture is
        // row-complete; agreement between the two sides alone would not, since
        // an over-reading stream can agree by accident
        match input {
            None => assert_eq!(expected, vec![0xffu8; row], "no-vis row is all-visible"),
            Some(_) => assert_eq!(expected, compressed_expected, "decoded row"),
        }
        // SAFETY (Shared): the model is only read; the closure runs to
        // completion inside the scope
        let m = Shared(&mut *model as *mut QModel);
        let got = std::thread::scope(|scope| {
            scope
                .spawn(move || run(quake_rs::model_parse::Mod_DecompressVis, m.ptr(), input))
                .join()
                .unwrap()
        });
        assert_eq!(
            expected, got,
            "Mod_DecompressVis off-main-thread, input {input:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// ADR-016 Send-purity, mechanized

/// ADR-016 requires everything scheduled onto a C task worker to be `Send`-safe
/// and pure over byte slices. The parsers themselves are `#![forbid(unsafe_code)]`
/// free functions, so what has to hold is that nothing they *return* is pinned
/// to a thread (an `Rc`, a `Cell`, a raw pointer field). Asserting it here
/// turns a future regression into a build failure instead of a review catch.
#[test]
fn parser_outputs_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}

    use quake_formats::{bsp, md3, md5, mdl, spr};
    assert_send_sync::<bsp::FunnySize>();
    assert_send_sync::<bsp::Bsp2>();
    assert_send_sync::<Vec<bsp::lumps::PlaneRec>>();
    assert_send_sync::<Vec<bsp::lumps::TexInfoRec>>();
    assert_send_sync::<bsp::lumps::TexInfoResolve>();
    assert_send_sync::<Vec<bsp::lumps::FaceRec>>();
    assert_send_sync::<bsp::lumps::FaceClassify>();
    assert_send_sync::<Vec<bsp::lumps::NodeRec>>();
    assert_send_sync::<bsp::lumps::NodeChild>();
    assert_send_sync::<Result<Vec<bsp::lumps::LeafRec>, bsp::lumps::LeafError>>();
    assert_send_sync::<Vec<bsp::lumps::ClipnodeRec>>();
    assert_send_sync::<bsp::lumps::MarkResult>();
    assert_send_sync::<Vec<bsp::lumps::SubmodelRec>>();
    assert_send_sync::<Vec<bsp::textures::TexWork>>();
    assert_send_sync::<bsp::textures::TexRec>();
    assert_send_sync::<bsp::lighting::LitCheck>();
    assert_send_sync::<bsp::vis::VisStatus>();
    assert_send_sync::<bsp::extents::SurfaceExtents>();
    assert_send_sync::<bsp::extents::BadExtents>();
    assert_send_sync::<mdl::MdlHeader>();
    assert_send_sync::<Vec<mdl::Diag>>();
    assert_send_sync::<mdl::Triangle>();
    assert_send_sync::<mdl::FrameHeader>();
    assert_send_sync::<mdl::GroupHeader>();
    assert_send_sync::<mdl::AliasBounds>();
    assert_send_sync::<spr::SpriteHeader>();
    assert_send_sync::<spr::FrameGeom>();
    assert_send_sync::<md3::HeaderCounts>();
    assert_send_sync::<md3::HeaderOffsets>();
    assert_send_sync::<md3::SurfaceHeader>();
    assert_send_sync::<md5::VertInfo>();
    assert_send_sync::<md5::WeightInfo>();
    assert_send_sync::<Result<md5::BakeOutcome, md5::BakeError>>();

    use quake_image::{lmp, pcx};
    assert_send_sync::<pcx::Header>();
    assert_send_sync::<pcx::Error>();
    assert_send_sync::<lmp::Lmp<'static>>();
    assert_send_sync::<lmp::Error>();
}
