//! Shared per-format load drivers over the model_parse seam (Phase 3 M7).
//!
//! The differential test suites each carry their own driver tuned to their
//! fixtures; this module is the reusable equivalent for the two M7 consumers
//! that cannot live in `tests/`: the `formats_corpus` binary (D9 corpus gate)
//! and the `rust/fuzz` differential targets (D11). Every driver walks one
//! side — the `c_ref_*` oracle or the Rust shims — through the exact call
//! order `gl_model.c` uses and returns the comparable observation streams
//! (deep-walk snapshot(s), console log, recorded stub callbacks, trapped
//! error messages).
//!
//! C-side fatal paths run under the stub's `setjmp` traps with a C-only
//! frame between the trap and the seam call (PLAN.md §4.3: a longjmp must
//! never unwind a Rust frame, so each trapped closure wraps exactly one
//! seam call). The Rust side is never run under a trap: its `Sys_Error`
//! aborts, so callers must only drive it over inputs whose accept/reject
//! decision was established first (the fuzz targets do that with the pure
//! quake-formats/quake-image predicates; the corpus binary only feeds
//! assets the C side accepted).

use core::ffi::{c_char, c_int, c_uint, c_void, CStr};

use crate::fs::{self as ctfs, Side};
use crate::mdx_record::{ctest_mdxstub_reset, recorded_skins, recorded_uploads, MdxSkin, Upload};
use crate::model_hash::{alias_snapshot, mdx_snapshot, snapshot, AliasScratch, BlobLens, Snapshot};
use quake_types::bspfile::{
    LumpT, BSP2VERSION_2PSB, BSP2VERSION_BSP2, BSPVERSION_QUAKE64, BSPVERSION_VALVE, HEADER_LUMPS,
    LUMP_CLIPNODES, LUMP_EDGES, LUMP_ENTITIES, LUMP_FACES, LUMP_LEAFS, LUMP_LIGHTING,
    LUMP_MARKSURFACES, LUMP_MODELS, LUMP_NODES, LUMP_PLANES, LUMP_SURFEDGES, LUMP_TEXINFO,
    LUMP_TEXTURES, LUMP_VERTEXES, LUMP_VISIBILITY,
};
use quake_types::model_mem::{AliasHdr, MSurface, MTriangle, QModel, MAX_QPATH};
use quake_types::modelgen::{StVert, TriVertX};

/// `gl_model.h` bounds of the shared alias scratch arrays.
pub const MAXALIASVERTS: usize = 0x7fff;
pub const MAXALIASFRAMES: usize = 2048;

/// Mirrors `CTEST_MODELSTUB_MAX` in `stubs/stubs.c`: the capacity of the
/// `Mod_LoadAllSkins` / `TexMgr_LoadImage` call logs.
const CTEST_MODELSTUB_MAX: i32 = 64;

extern "C" {
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

    fn c_ref_Mod_ParseAliasModel(m: *mut QModel, buffer: *mut c_void) -> *mut AliasHdr;
    fn c_ref_Mod_LoadSpriteModel(m: *mut QModel, buffer: *mut c_void);
    fn c_ref_Mod_LoadMD3Model(m: *mut QModel, buffer: *const c_void);
    fn c_ref_Mod_LoadMD5MeshModel(m: *mut QModel, buffer: *const c_void) -> bool;

    fn c_ref_Image_DecodePCX(
        file_handle: c_int,
        width: *mut c_int,
        height: *mut c_int,
        image_name: *const c_char,
    ) -> *mut u8;
    fn c_ref_Image_DecodeLMP(
        file_handle: c_int,
        width: *mut c_int,
        height: *mut c_int,
        image_name: *const c_char,
    ) -> *mut u8;
    fn c_ref_Image_DecodeSTB(
        file_handle: c_int,
        width: *mut c_int,
        height: *mut c_int,
        image_name: *const c_char,
    ) -> *mut u8;

    fn ctest_try_host(f: unsafe extern "C" fn(*mut c_void), arg: *mut c_void) -> c_int;
    fn ctest_host_error_message() -> *const c_char;
    fn ctest_set_sv_modelname(name: *const c_char);
    fn ctest_set_external_ents(value: f32);
    fn ctest_fill_dummy_textures(m: *mut QModel);
    fn ctest_mod_pool_reset();
    fn ctest_mod_pool_get(i: c_int) -> *mut QModel;
    fn ctest_mod_pool_len() -> c_int;
    fn ctest_modelstub_reset(base: *const u8);
    fn ctest_allskins_count() -> i32;
    fn ctest_allskins_set_advance(on: c_int);
    fn ctest_allskins_calls() -> *const AllSkinsCall;
    fn ctest_teximage_count() -> i32;
    fn ctest_teximage_calls() -> *const TexImageCall;

    // shared by both sides: model_parse.c defines these unconditionally
    static mut stverts: [StVert; MAXALIASVERTS];
    static mut triangles: *mut MTriangle;
    static mut poseverts: [*mut TriVertX; MAXALIASFRAMES];
}

/// A fatal path one C-side loader took; the Rust side signals `Host` through
/// the status-returning shims and can never report `Sys` (its `Sys_Error`
/// aborts the process, by design).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    Sys(String),
    Host(String),
}

pub fn new_model(name: &str, bspversion: i32) -> Box<QModel> {
    // SAFETY: qmodel_t is zero-initialized by the engine too; all-zero is a
    // valid (null-pointer, empty-name) value for every field of the mirror
    let mut m: Box<QModel> = Box::new(unsafe { core::mem::zeroed() });
    assert!(name.len() < MAX_QPATH);
    for (i, c) in name.bytes().enumerate() {
        m.name[i] = c as c_char;
    }
    m.bspversion = bspversion;
    m
}

/// Runs `f` (which must wrap exactly one C seam call: C frames only under
/// the longjmp) with the `Host_Error` trap armed.
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
    let msg = unsafe { CStr::from_ptr(ctest_host_error_message()) };
    Some(msg.to_string_lossy().into_owned())
}

fn err_string(buf: &[c_char]) -> String {
    // SAFETY: the shim writes a NUL-terminated string into the 256-byte buffer
    let msg = unsafe { CStr::from_ptr(buf.as_ptr()) };
    msg.to_string_lossy().into_owned()
}

/// Reads lump `i` out of an assembled BSP file image.
pub fn lump_of(data: &[u8], i: usize) -> LumpT {
    let at = 4 + i * 8;
    LumpT {
        fileofs: i32::from_le_bytes(data[at..at + 4].try_into().unwrap()),
        filelen: i32::from_le_bytes(data[at + 4..at + 8].try_into().unwrap()),
    }
}

/// The `bsp2` int gl_model.c derives from the version field (0/1/2).
pub fn bsp2_of(version: i32) -> c_int {
    match version {
        v if v == BSP2VERSION_2PSB => 1,
        v if v == BSP2VERSION_BSP2 => 2,
        _ => 0,
    }
}

/// The byte length `Mod_LoadLighting` allocates for `lightdata` when no
/// `.lit` file replaces it (the fuzz fixtures mount an empty dir, and the
/// corpus driver checks for a `.lit` separately via [`lit_replacement_len`]).
pub fn expanded_light_len(version: i32, filelen: i32) -> usize {
    let filelen = filelen.max(0) as usize;
    if filelen == 0 {
        0
    } else if version == BSPVERSION_QUAKE64 {
        (filelen / 2) * 3
    } else if version == BSPVERSION_VALVE {
        filelen
    } else {
        filelen * 3
    }
}

/// Replicates the `.lit` accept test of `Mod_LoadLighting` for one side:
/// returns the replaced `lightdata` length when the side would take the
/// `.lit`, i.e. the file exists at an equal-or-higher searchpath priority,
/// starts with `QLIT`, has version 1 and exactly `8 + filelen * 3` bytes.
pub fn lit_replacement_len(
    side: Side,
    map_name: &str,
    map_path_id: u32,
    filelen: i32,
) -> Option<usize> {
    let stem = map_name.strip_suffix(".bsp").unwrap_or(map_name);
    let lit = std::ffi::CString::new(format!("{stem}.lit")).ok()?;
    let (data, size, _, path_id) = ctfs::load_file(side, &lit)?;
    if path_id < map_path_id || data.len() < 8 || &data[..4] != b"QLIT" {
        return None;
    }
    let ver = i32::from_le_bytes(data[4..8].try_into().unwrap());
    if ver != 1 {
        return None;
    }
    let want = 8i64 + i64::from(filelen) * 3;
    (size == want).then(|| (filelen.max(0) as usize) * 3)
}

/// Everything one side produces from a full brush load.
pub struct BspLoaded {
    /// the main model plus every `Mod_FindName` clone, in pool order; empty
    /// when a C-side `Sys_Error` stopped the sequence
    pub snaps: Vec<Snapshot>,
    pub con_log: Vec<String>,
    pub error: Option<LoadError>,
}

/// Drives one side through `Mod_LoadBrushModel`'s call order over `data`
/// (a whole `dheader_t`-prefixed file image whose 15 lump descriptors lie
/// inside it). Caller must hold [`ctfs::lock`], have mounted this side's
/// searchpaths, and guarantee the image is inside the C loaders' defined
/// domain (every cross-lump reference the C does not itself validate must
/// be in range). A trapped C-side `Sys_Error` stops the sequence (the
/// engine would have aborted); a Host_Error records the first message and
/// continues, exactly like `Mod_LoadBrushModel`'s ctest differential.
///
/// # Safety
/// `data` must satisfy the domain contract above; on `Side::Rust` the
/// caller must have established (via the pure-parser predicates) that no
/// seam call reaches `Sys_Error`, which would abort the process.
pub unsafe fn bsp_load_side(
    side: Side,
    name: &str,
    data: &[u8],
    sv_modelname: &CStr,
    external_ents: f32,
    lens: BlobLens,
) -> BspLoaded {
    let mut d = data.to_vec();
    let version = i32::from_le_bytes(d[..4].try_into().unwrap());
    // snapshot the lump descriptors before taking the mutable pointer, so no
    // shared reborrow of `d` is live across the C calls that write through it
    let lumps: [LumpT; HEADER_LUMPS] = core::array::from_fn(|i| lump_of(&d, i));
    let base = d.as_mut_ptr();
    // the name is a loader input, not a label: Mod_LoadLighting derives the
    // `.lit` sidecar path from it, Mod_LoadEntities the `.ent` one, and
    // Mod_SetupSubmodels compares it against sv.modelname for the
    // world-model clipbox branch
    let mut model = new_model(name, version);
    let m: *mut QModel = &raw mut *model;
    let bsp2 = bsp2_of(version);
    let lump = |i: usize| lumps[i];

    // SAFETY: stub globals written under the fs lock the caller holds
    unsafe {
        ctest_set_sv_modelname(sv_modelname.as_ptr());
        ctest_set_external_ents(external_ents);
        ctest_mod_pool_reset();
    }
    ctfs::clear_logs();

    let mut sys: Option<String> = None;
    let mut host: Option<String> = None;

    // On the C side every infallible-in-Rust seam call still runs under the
    // Sys_Error trap (funny lump sizes, bad surface extents, ...); each
    // closure wraps exactly one C call so the longjmp crosses C frames only.
    // On the Rust side the shims are called directly: the caller's domain
    // contract makes Sys_Error unreachable, and the Host_Error-capable four
    // return status + message instead of jumping.
    macro_rules! step {
        ($c:expr, $r:expr) => {
            if sys.is_none() {
                match side {
                    Side::C => {
                        // SAFETY: single C seam call under the trap; the
                        // model, image and lump outlive it
                        let msg = ctfs::catch_sys_error(|| unsafe { $c });
                        if let Some(msg) = msg {
                            sys = Some(msg);
                        }
                    }
                    // SAFETY: caller contract (domain-checked input)
                    Side::Rust => unsafe { $r },
                }
            }
        };
    }
    macro_rules! host_step {
        ($c:expr, $r:expr) => {
            if sys.is_none() && host.is_none() {
                match side {
                    Side::C => {
                        // Host_Error and Sys_Error can both fire inside the
                        // fallible loaders; arm both traps (host innermost,
                        // matching the engine's longjmp nesting).
                        let mut host_msg = None;
                        let msg = ctfs::catch_sys_error(|| {
                            // SAFETY: single C seam call under both traps;
                            // the model, image and lump outlive it
                            host_msg = c_host_try(&mut || unsafe { $c });
                        });
                        if let Some(msg) = msg {
                            sys = Some(msg);
                        } else {
                            host = host_msg;
                        }
                    }
                    Side::Rust => {
                        let mut err = vec![0 as c_char; 256];
                        let f = $r;
                        let ok = f(err.as_mut_ptr());
                        if ok == 0 {
                            host = Some(err_string(&err));
                        }
                    }
                }
            }
        };
    }

    use quake_rs::model_parse as rs;
    step!(
        c_ref_Mod_LoadVertexes(m, base, &lump(LUMP_VERTEXES)),
        rs::Mod_LoadVertexes(m, base, &lump(LUMP_VERTEXES))
    );
    step!(
        c_ref_Mod_LoadEdges(m, base, &lump(LUMP_EDGES), bsp2),
        rs::Mod_LoadEdges(m, base, &lump(LUMP_EDGES), bsp2)
    );
    step!(
        c_ref_Mod_LoadSurfedges(m, base, &lump(LUMP_SURFEDGES)),
        rs::Mod_LoadSurfedges(m, base, &lump(LUMP_SURFEDGES))
    );
    step!(
        c_ref_Mod_LoadEntities(m, base, &lump(LUMP_ENTITIES)),
        rs::Mod_LoadEntities(m, base, &lump(LUMP_ENTITIES))
    );
    step!(
        c_ref_Mod_ParseTextures(m, base, &lump(LUMP_TEXTURES), core::ptr::null_mut()),
        rs::Mod_ParseTextures(m, base, &lump(LUMP_TEXTURES), core::ptr::null_mut())
    );
    if sys.is_none() {
        // SAFETY: the model's texture array was just filled by either side
        unsafe { ctest_fill_dummy_textures(m) };
    }
    step!(
        c_ref_Mod_LoadLighting(m, base, &lump(LUMP_LIGHTING)),
        rs::Mod_LoadLighting(m, base, &lump(LUMP_LIGHTING))
    );
    step!(
        c_ref_Mod_LoadPlanes(m, base, &lump(LUMP_PLANES)),
        rs::Mod_LoadPlanes(m, base, &lump(LUMP_PLANES))
    );
    step!(
        c_ref_Mod_LoadTexinfo(m, base, &lump(LUMP_TEXINFO)),
        rs::Mod_LoadTexinfo(m, base, &lump(LUMP_TEXINFO))
    );
    step!(
        c_ref_Mod_ParseFaces(m, base, &lump(LUMP_FACES), bsp2 != 0),
        rs::Mod_ParseFaces(m, base, &lump(LUMP_FACES), bsp2 != 0)
    );
    if sys.is_none() {
        // SAFETY: surfaces were just allocated by Mod_ParseFaces
        let numsurfaces = unsafe { (*m).numsurfaces };
        for i in 0..numsurfaces {
            step!(
                c_ref_CalcSurfaceExtents(m, (*m).surfaces.offset(i as isize)),
                rs::CalcSurfaceExtents(m, (*m).surfaces.offset(i as isize))
            );
        }
    }
    host_step!(
        c_ref_Mod_LoadMarksurfaces(m, base, &lump(LUMP_MARKSURFACES), bsp2),
        // SAFETY: caller contract plus a 256-byte err buffer
        |err| unsafe {
            rs::quake_rs_mod_load_marksurfaces(m, base, &lump(LUMP_MARKSURFACES), bsp2, err)
        }
    );
    step!(
        c_ref_Mod_LoadVisibility(m, base, &lump(LUMP_VISIBILITY)),
        rs::Mod_LoadVisibility(m, base, &lump(LUMP_VISIBILITY))
    );
    host_step!(
        c_ref_Mod_LoadLeafs(m, base, &lump(LUMP_LEAFS), bsp2),
        // SAFETY: caller contract plus a 256-byte err buffer
        |err| unsafe { rs::quake_rs_mod_load_leafs(m, base, &lump(LUMP_LEAFS), bsp2, err) }
    );
    step!(
        c_ref_Mod_LoadNodes(m, base, &lump(LUMP_NODES), bsp2),
        rs::Mod_LoadNodes(m, base, &lump(LUMP_NODES), bsp2)
    );
    host_step!(
        c_ref_Mod_LoadClipnodes(m, base, &lump(LUMP_CLIPNODES), bsp2 != 0),
        // SAFETY: caller contract plus a 256-byte err buffer
        |err| unsafe {
            rs::quake_rs_mod_load_clipnodes(m, base, &lump(LUMP_CLIPNODES), bsp2 != 0, err)
        }
    );
    step!(
        c_ref_Mod_LoadSubmodels(m, base, &lump(LUMP_MODELS)),
        rs::Mod_LoadSubmodels(m, base, &lump(LUMP_MODELS))
    );
    step!(c_ref_Mod_MakeHull0(m), rs::Mod_MakeHull0(m));
    if sys.is_none() {
        // gl_model.c sets numframes = 2 on every brush model before setup
        // SAFETY: the model is live
        unsafe { (*m).numframes = 2 };
        host_step!(
            c_ref_Mod_SetupSubmodels(m),
            // SAFETY: caller contract plus a 256-byte err buffer
            |err| unsafe { rs::quake_rs_mod_setup_submodels(m, sv_modelname.as_ptr(), err) }
        );
    }

    let mut snaps = Vec::new();
    if sys.is_none() {
        // SAFETY: the model and every pool clone stay alive until the
        // snapshot is taken; `lens` describes the blobs the loaders allocated
        unsafe {
            snaps.push(snapshot(m, lens));
            for i in 0..ctest_mod_pool_len() {
                snaps.push(snapshot(ctest_mod_pool_get(i), lens));
            }
        }
    }
    BspLoaded {
        snaps,
        con_log: ctfs::con_log(),
        error: sys.map(LoadError::Sys).or(host.map(LoadError::Host)),
    }
}

/// Mirror of `ctest_allskins_call_t` in `stubs/stubs.c`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AllSkinsCall {
    pub numskins: i32,
    pub pskintype_ofs: i64,
}

/// Everything one side produces from an alias (.mdl) parse.
pub struct AliasLoaded {
    /// `None` when the C side `Sys_Error`ed (trapped)
    pub snap: Option<Snapshot>,
    pub con_log: Vec<String>,
    pub skins: Vec<AllSkinsCall>,
    pub error: Option<String>,
}

/// Runs one side's `Mod_ParseAliasModel` over its own copy of `image`.
/// Caller must hold [`ctfs::lock`] and must run [`Side::C`] before
/// [`Side::Rust`] (the sides share the C scratch arrays and the `triangles`
/// grow counter). With `advance_skins` the stub `Mod_LoadAllSkins`
/// replicates the frozen gl_model.c cursor walk so real skinned models
/// parse; without it the stub returns the cursor unmoved (the synthetic
/// fixtures' `numskins == 0` contract).
///
/// # Safety
/// `image` must lie inside the C parser's defined domain (complete layout
/// for the counts it declares, and `numskins == 0` unless `advance_skins`);
/// on `Side::Rust` the caller must have established the parse does not
/// reach `Sys_Error`.
pub unsafe fn alias_load_side(
    side: Side,
    name: &str,
    image: &[u8],
    advance_skins: bool,
) -> AliasLoaded {
    let mut data = image.to_vec();
    let base = data.as_mut_ptr();
    // the name is a loader input: Mod_SetExtraFlags matches it against
    // r_nolerp_list and the flame/boss fullbright hack, and it prefixes the
    // per-frame texture names
    let mut model = new_model(name, 0);
    let m: *mut QModel = &raw mut *model;

    ctfs::clear_logs();
    // SAFETY: the recorder is a set of plain statics guarded by the fs lock
    unsafe {
        ctest_modelstub_reset(base);
        ctest_allskins_set_advance(advance_skins.into());
    }

    let mut h: *mut AliasHdr = core::ptr::null_mut();
    let error = match side {
        // SAFETY: single C call under the trap; base/m outlive it
        Side::C => ctfs::catch_sys_error(|| unsafe {
            h = c_ref_Mod_ParseAliasModel(m, base.cast::<c_void>());
        }),
        Side::Rust => {
            // SAFETY: caller contract (domain-checked, non-fatal input)
            h = unsafe { quake_rs::model_parse::Mod_ParseAliasModel(m, base.cast::<c_void>()) };
            None
        }
    };

    let snap = if error.is_none() {
        assert!(!h.is_null(), "{side:?}: parse returned null");
        // SAFETY: the header, the model and the shared scratch arrays are
        // all still live, and `data` is the image this side walked
        Some(unsafe {
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
        })
    } else {
        None
    };
    // SAFETY: the assert establishes that all `n` entries were written —
    // the stub increments its counter unconditionally but only writes below
    // the cap, so an unclamped read would walk off the end of the log
    let skins = unsafe {
        let n = ctest_allskins_count();
        assert!(
            n <= CTEST_MODELSTUB_MAX,
            "Mod_LoadAllSkins recorder overflowed ({n} calls > {CTEST_MODELSTUB_MAX})"
        );
        let p = ctest_allskins_calls();
        (0..n as isize).map(|i| *p.offset(i)).collect()
    };
    AliasLoaded {
        snap,
        con_log: ctfs::con_log(),
        skins,
        error,
    }
}

/// Mirror of `ctest_teximage_call_t` in `stubs/stubs.c`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TexImageCall {
    pub name: [c_char; 64],
    pub width: i32,
    pub height: i32,
    pub format: i32,
    pub data_ofs: i64,
    pub source_file: [c_char; 64],
    pub source_offset: u64,
    pub flags: u32,
}

/// Comparable, printable form of one recorded `TexMgr_LoadImage` call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TexImage {
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub format: i32,
    pub data_ofs: i64,
    pub source_file: String,
    pub source_offset: u64,
    pub flags: u32,
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

fn recorded_teximages() -> Vec<TexImage> {
    // SAFETY: the assert establishes that all `n` entries were written —
    // TexMgr_LoadImage fires once per sprite frame, so real assets can
    // outrun the 64-entry log where the synthetic fixtures never did
    unsafe {
        let n = ctest_teximage_count();
        assert!(
            n <= CTEST_MODELSTUB_MAX,
            "TexMgr_LoadImage recorder overflowed ({n} calls > {CTEST_MODELSTUB_MAX})"
        );
        let p = ctest_teximage_calls();
        (0..n as isize)
            .map(|i| TexImage::from(&*p.offset(i)))
            .collect()
    }
}

/// Everything one side produces from a sprite (.spr) load.
pub struct SpriteLoaded {
    pub snap: Option<Snapshot>,
    pub con_log: Vec<String>,
    pub textures: Vec<TexImage>,
    pub error: Option<String>,
}

/// Runs one side's `Mod_LoadSpriteModel` over its own copy of `image`.
/// Caller must hold [`ctfs::lock`].
///
/// # Safety
/// Same contract as [`alias_load_side`]: domain-checked image, and on
/// `Side::Rust` a pre-established non-fatal decision.
pub unsafe fn sprite_load_side(side: Side, name: &str, image: &[u8]) -> SpriteLoaded {
    let mut data = image.to_vec();
    let base = data.as_mut_ptr();
    let mut model = new_model(name, 0);
    let m: *mut QModel = &raw mut *model;

    ctfs::clear_logs();
    // SAFETY: the recorder is a set of plain statics guarded by the fs lock
    unsafe { ctest_modelstub_reset(base) };

    let error = match side {
        // SAFETY: single C call under the trap; base/m outlive it
        Side::C => ctfs::catch_sys_error(|| unsafe {
            c_ref_Mod_LoadSpriteModel(m, base.cast::<c_void>());
        }),
        Side::Rust => {
            // SAFETY: caller contract (domain-checked, non-fatal input)
            unsafe { quake_rs::model_parse::Mod_LoadSpriteModel(m, base.cast::<c_void>()) };
            None
        }
    };

    let snap = if error.is_none() {
        // SAFETY: the model and its sprite graph are still live
        Some(unsafe { crate::model_hash::sprite_snapshot(m) })
    } else {
        None
    };
    SpriteLoaded {
        snap,
        con_log: ctfs::con_log(),
        textures: recorded_teximages(),
        error,
    }
}

/// Everything one side produces from an MD3 load.
pub struct Md3Loaded {
    pub snap: Option<Snapshot>,
    pub con_log: Vec<String>,
    pub uploads: Vec<Upload>,
    pub skins: Vec<MdxSkin>,
    /// the image after the load (the loader `q_strtrim`s surface names in
    /// place)
    pub image: Vec<u8>,
    pub error: Option<String>,
}

/// Runs one side's `Mod_LoadMD3Model` over its own copy of `image`.
/// Caller must hold [`ctfs::lock`].
///
/// # Safety
/// Same contract as [`alias_load_side`].
pub unsafe fn md3_load_side(side: Side, name: &str, image: &[u8], skins_result: i32) -> Md3Loaded {
    let mut data = image.to_vec();
    let base = data.as_mut_ptr();
    let mut model = new_model(name, 0);
    let m: *mut QModel = &raw mut *model;

    ctfs::clear_logs();
    // SAFETY: the recorders are plain statics guarded by the fs lock
    unsafe {
        ctest_modelstub_reset(base);
        ctest_mdxstub_reset(skins_result);
    }

    let error = match side {
        // SAFETY: single C call under the trap; base/m outlive it
        Side::C => ctfs::catch_sys_error(|| unsafe {
            c_ref_Mod_LoadMD3Model(m, base.cast::<c_void>());
        }),
        Side::Rust => {
            // SAFETY: caller contract (domain-checked, non-fatal input)
            unsafe { quake_rs::model_parse::Mod_LoadMD3Model(m, base.cast::<c_void>()) };
            None
        }
    };

    let snap = if error.is_none() {
        // SAFETY: the model and its aliashdr_t chain are still live
        Some(unsafe { mdx_snapshot(m, quake_types::model_mem::PV_QUAKE3 as usize) })
    } else {
        None
    };
    Md3Loaded {
        snap,
        con_log: ctfs::con_log(),
        uploads: recorded_uploads(),
        skins: recorded_skins(),
        image: data,
        error,
    }
}

/// Everything one side produces from an MD5 mesh load (recoverable format:
/// failures return `ok == false` on both sides, no traps involved).
pub struct Md5Loaded {
    pub ok: bool,
    pub snap: Snapshot,
    pub con_log: Vec<String>,
    pub uploads: Vec<Upload>,
    pub skins: Vec<MdxSkin>,
}

/// Runs one side's `Mod_LoadMD5MeshModel` over its own copy of `image`
/// (which gets the NUL terminator `COM_LoadFile` appends). Caller must hold
/// [`ctfs::lock`] and have mounted this side's searchpaths (the loader
/// reads the companion `.md5anim`/`.mdl` through the filesystem).
///
/// # Safety
/// `model_name` selects the companion-file paths; the image is text and the
/// parser is bounded by the NUL. A model whose mesh count outruns the
/// upload recorder's capacity fails loudly, not unsafely: `recorded_uploads`
/// asserts on overflow rather than clamping (see `mdx_record.rs`).
pub unsafe fn md5_load_side(
    side: Side,
    model_name: &str,
    image: &[u8],
    skins_result: i32,
) -> Md5Loaded {
    let mut data = image.to_vec();
    data.push(0);
    let base = data.as_mut_ptr();
    let mut model = new_model(model_name, 0);
    let m: *mut QModel = &raw mut *model;

    ctfs::clear_logs();
    // SAFETY: the recorders are plain statics guarded by the fs lock
    unsafe {
        ctest_modelstub_reset(base);
        ctest_mdxstub_reset(skins_result);
    }

    let f = match side {
        Side::C => c_ref_Mod_LoadMD5MeshModel,
        Side::Rust => quake_rs::model_parse::Mod_LoadMD5MeshModel,
    };
    // SAFETY: `base` points at a whole NUL-terminated .md5mesh image that
    // outlives the call, and `m` at a live zeroed qmodel_t
    let ok = unsafe { f(m, base.cast::<c_void>()) };

    // SAFETY: the model and its aliashdr_t chain are still live
    let snap = unsafe { mdx_snapshot(m, quake_types::model_mem::PV_MD5 as usize) };
    Md5Loaded {
        ok,
        snap,
        con_log: ctfs::con_log(),
        uploads: recorded_uploads(),
        skins: recorded_skins(),
    }
}

/// Which Image_Decode* seam function a driver call exercises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Pcx,
    Lmp,
    /// Image_DecodeSTB: the PNG/TGA/JPG sniff-and-dispatch seam (M8). Unlike
    /// PCX/LMP it never Sys_Errors — reject is a Con_Warning + NULL.
    Stb,
}

/// Everything one side observes from an image decode call — the same
/// streams `image_differential::Outcome` compares, so the corpus gate is
/// not weaker than the fixture suite on this seam.
#[derive(Debug, PartialEq, Eq)]
pub struct ImageOutcome {
    pub width: c_int,
    pub height: c_int,
    /// `None` for a NULL return or a trapped `Sys_Error`
    pub data: Option<Vec<u8>>,
    /// `com_filesize` after the open — the value the LMP size gate keys on
    pub file_size: i64,
    /// change in open handles across the call (after the force-close on a
    /// trapped fatal): 0 proves the decoder closed its handle. A delta, not
    /// an absolute count — mounted pak files hold handles open for the
    /// process lifetime
    pub open_handles: i32,
    pub con_log: Vec<String>,
    pub error: Option<String>,
}

/// Opens `name` on `side`, runs that side's `format` decoder over the
/// handle, and snapshots the observable state; `buf_len` maps the
/// out-dimensions to the byte count of the returned allocation. The C side
/// runs under the `Sys_Error` trap (a fatal decode leaves the handle open;
/// it is closed here). Caller must hold [`ctfs::lock`] and have mounted the
/// fixture.
///
/// # Safety
/// On `Side::Rust` with Pcx/Lmp the caller must have established via the
/// pure `quake-image` decoders that the input does not reach `Sys_Error`
/// (Stb never Sys_Errors on either side).
pub unsafe fn image_decode_side(
    side: Side,
    name: &CStr,
    format: ImageFormat,
    buf_len: impl Fn(c_int, c_int) -> usize,
) -> ImageOutcome {
    ctfs::clear_logs();
    let handles_before = ctfs::open_handle_count();
    let mut handle: c_int = -1;
    let mut path_id: c_uint = 0;
    // SAFETY: side's searchpaths are mounted; out-params are valid
    let size = unsafe { (ctfs::fns(side).open_file)(name.as_ptr(), &mut handle, &mut path_id) };
    assert!(size >= 0, "fixture {name:?} must open on {side:?}");
    let file_size = ctfs::thread_file_size();

    let mut width: c_int = -1;
    let mut height: c_int = -1;
    let mut data: *mut u8 = core::ptr::null_mut();
    let error = match (side, format) {
        // SAFETY: single C call under the trap; open handle, valid pointers
        (Side::C, ImageFormat::Pcx) => ctfs::catch_sys_error(|| unsafe {
            data = c_ref_Image_DecodePCX(handle, &mut width, &mut height, name.as_ptr());
        }),
        // SAFETY: as above
        (Side::C, ImageFormat::Lmp) => ctfs::catch_sys_error(|| unsafe {
            data = c_ref_Image_DecodeLMP(handle, &mut width, &mut height, name.as_ptr());
        }),
        // SAFETY: as above (the STB seam itself never Sys_Errors; the trap
        // covers stub fatals like allocation failure)
        (Side::C, ImageFormat::Stb) => ctfs::catch_sys_error(|| unsafe {
            data = c_ref_Image_DecodeSTB(handle, &mut width, &mut height, name.as_ptr());
        }),
        (Side::Rust, ImageFormat::Pcx) => {
            // SAFETY: caller contract (pure decoder says non-fatal)
            data = unsafe {
                quake_rs::image_decode::Image_DecodePCX(
                    handle,
                    &mut width,
                    &mut height,
                    name.as_ptr(),
                )
            };
            None
        }
        (Side::Rust, ImageFormat::Lmp) => {
            // SAFETY: caller contract (pure decoder says non-fatal)
            data = unsafe {
                quake_rs::image_decode::Image_DecodeLMP(
                    handle,
                    &mut width,
                    &mut height,
                    name.as_ptr(),
                )
            };
            None
        }
        (Side::Rust, ImageFormat::Stb) => {
            // SAFETY: open handle, valid pointers; this seam soft-fails
            data = unsafe {
                quake_rs::image_decode::Image_DecodeSTB(
                    handle,
                    &mut width,
                    &mut height,
                    name.as_ptr(),
                )
            };
            None
        }
    };
    if error.is_some() {
        // the decoder fataled before COM_CloseFile; drop the handle so the
        // per-iteration handle count stays balanced
        // SAFETY: handle is still open
        unsafe { (ctfs::fns(side).close_file)(handle) };
    }
    let data = if data.is_null() {
        None
    } else {
        let len = buf_len(width, height);
        // SAFETY: the decoder returned a Mem_Alloc'd buffer of exactly the
        // size its C original allocates, which buf_len reproduces
        let bytes = unsafe { core::slice::from_raw_parts(data, len) }.to_vec();
        // SAFETY: buffer came from the stub Mem_Alloc just above
        unsafe { quake_c_sys::Mem_Free(data.cast()) };
        Some(bytes)
    };
    ImageOutcome {
        width,
        height,
        data,
        file_size,
        open_handles: ctfs::open_handle_count() - handles_before,
        con_log: ctfs::con_log(),
        error,
    }
}

/// Sanity re-export so driver consumers need only this module.
pub use crate::model_hash::BlobLens as BspBlobLens;

const _: () = assert!(HEADER_LUMPS == 15);
