//! `r_brush.c` (Phase 8 M8): brush-model drawing, the lightmap atlases and
//! their GPU update (compute + upload paths), the indirect-draw structures,
//! the brush dependency tables and the SoA culling data. The ray-tracing
//! half of the file (`R_AllocateTLAS`, the BLAS builds,
//! `R_BuildTopLevelAccelerationStructure`, ...) stays C in
//! `Quake/r_brush_glue.c` until Phase 8 M10 (D5); it reaches this module
//! through the exported `bmodel_vertex_buffer`, `bmodel_numverts` and
//! `bmodel_vertex_buffer_device_address`.
//!
//! Headless mode (`no_rendering`) never reaches any of this: `gl_model.c`
//! and `gl_rmisc.c` gate the lightmap/vertex-buffer builds on
//! `!no_rendering` and the draw paths only run inside a rendered frame.
//!
//! Every draw goes through `quake_render::cb` so the `-renderhash`
//! draw-call structure stays identical to the C build.
#![allow(non_snake_case, non_upper_case_globals)]

use core::ffi::{c_char, c_float, c_int, c_void, CStr};
use core::ptr;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::ffi::CString;

use ash::vk;
use ash::vk::Handle;
use quake_c_sys as c;
use quake_c_sys::cvar_t;
use quake_math::mathlib::{angle_vectors, dot_product, identity_matrix, matrix_multiply, Vec3};
use quake_net::cnum::c_atoi;
use quake_render::cb::{self, CmdProcs};
use quake_render::rmisc::{
    allocate_descriptor_set, allocate_vulkan_memory, create_buffer, create_buffers, free_buffer,
    free_descriptor_set, free_vulkan_memory, BufferRequest,
};
use quake_types::host::{ClientState, Entity, MAX_MODELS};
use quake_types::model_mem::{
    MEdge, MNode, MSurface, MVertex, QModel, Texture, MOD_BRUSH, SURF_DRAWSKY, SURF_DRAWTILED,
    SURF_DRAWTURB, SURF_PLANEBACK, TEXTYPE_CUTOUT, TEXTYPE_SKY,
};
use quake_types::model_mem::{SoaAabb, SoaPlane};
use quake_types::plane::MPlane;
use quake_types::refdef::RefDef;
use quake_types::render::{
    BModelInstance, BasicVertex, CbContext, GlPoly, GlRect, GlTexture, Lightmap, LmComputeLight,
    LmComputeSurfaceData, LmComputeWorkgroupBounds, VulkanMemory, VulkanMemoryType,
    FAN_INDEX_BUFFER_SIZE, LMBLOCK_HEIGHT, LMBLOCK_WIDTH, LM_CULL_BLOCK_H, LM_CULL_BLOCK_W,
    LM_CULL_COLS, LM_CULL_ROWS, LM_WORKGROUP_SUBMODEL_EMPTY, LM_WORKGROUP_SUBMODEL_MIXED,
    MAXLIGHTMAPS, MAX_LIGHTSTYLES, MAX_SANITY_LIGHTMAPS, PCBX_UPDATE_LIGHTMAPS, TASKS_MAX_WORKERS,
    VERTEXSIZE,
};

use crate::gl_rlight::{lightmap_dlight_origins, R_MarkLights};
use crate::gl_rmisc::{device, num_vulkan_bmodel_allocations, with_ctx, DYN, STAGING};
use crate::gl_texmgr::TexMgr_LoadImage;
use crate::gl_vidsdl::GL_WaitForDeviceIdle;
use crate::r_world::{
    world_pipeline, R_ChainSurface, R_ClearTextureChains, R_DrawTextureChains,
    R_DrawTextureChains_Water,
};

extern "C" {
    /// `client_state_t cl` (ADR-007 row closed in Phase 7).
    static mut cl: ClientState;
    /// `gl_rmain.c:58` -- `refdef_t r_refdef`.
    static mut r_refdef: RefDef;
}

// ---- constants ------------------------------------------------------------

/// `MAX_DLIGHTS` (`glquake.h`).
const MAX_DLIGHTS: usize = 64;
/// `NUM_WORLD_CBX` (`glquake.h`).
const NUM_WORLD_CBX: c_int = 6;
/// `BACKFACE_EPSILON` (`glquake.h`).
const BACKFACE_EPSILON: f32 = 0.01;
/// `MAX_INDIRECT_DRAWS` (`r_brush.c`).
const MAX_INDIRECT_DRAWS: usize = 32768;
/// `INDIRECT_ZBIAS` (`r_brush.c`).
const INDIRECT_ZBIAS: bool = true;
/// `INITIAL_BRUSH_DEPS_SIZE` (`r_brush.c`).
const INITIAL_BRUSH_DEPS_SIZE: c_int = 16384;
/// `SHELF_HEIGHT` / `SHELVES` / `LM_BIN_E` / `LM_BINS` (`r_brush.c`).
const SHELF_HEIGHT: c_int = 256;
const SHELVES: usize = 4;
const LM_BIN_E: c_int = 8;
const LM_BINS: usize = 49;
/// `UPDATE_LIGHTMAP_BATCH_SIZE` (`r_brush.c`).
const UPDATE_LIGHTMAP_BATCH_SIZE: usize = 64;
/// `LIGHTMAP_BYTES` (`glquake.h`).
const LIGHTMAP_BYTES: usize = 4;
/// `WORKGROUP_BOUNDS_BUFFER_SIZE` (`r_brush.c`).
const WORKGROUP_BOUNDS_BUFFER_SIZE: usize =
    (LMBLOCK_WIDTH / 8) * (LMBLOCK_HEIGHT / 8) * core::mem::size_of::<LmComputeWorkgroupBounds>();
/// `TEXTYPE_FIRSTLIQUID` / `TEXTYPE_LASTLIQUID` (`gl_model.h` macros).
const TEXTYPE_FIRSTLIQUID: c_int = 3;
const TEXTYPE_LASTLIQUID: c_int = 6;
/// `ENTALPHA_DEFAULT` (`quakedef.h`).
const ENTALPHA_DEFAULT: u8 = 0;
/// `TEXPREF_*` (`gl_texmgr.h`).
const TEXPREF_LINEAR: u32 = 0x0002;
const TEXPREF_NEAREST: u32 = 0x0004;
const TEXPREF_NOPICMIP: u32 = 0x0080;
/// `SRC_*` (`gl_texmgr.h`).
const SRC_LIGHTMAP: c_int = 1;
const SRC_RGBA: c_int = 2;
const SRC_SURF_INDICES: c_int = 3;
/// `VkDrawIndexedIndirectCommand` stride.
const INDIRECT_CMD_SIZE: usize = core::mem::size_of::<vk::DrawIndexedIndirectCommand>();
const LM_COMPUTE_LIGHT_SIZE: usize = core::mem::size_of::<LmComputeLight>();
const LM_SURFACE_DATA_SIZE: usize = core::mem::size_of::<LmComputeSurfaceData>();
const BMODEL_INSTANCE_SIZE: usize = core::mem::size_of::<BModelInstance>();

const NULL_MEMORY: VulkanMemory = VulkanMemory {
    handle: vk::DeviceMemory::null(),
    size: 0,
    type_: VulkanMemoryType::None,
};

const _: () = {
    assert!(INDIRECT_CMD_SIZE == 20);
    assert!(WORKGROUP_BOUNDS_BUFFER_SIZE == 128 * 128 * 28);
};

// ---- exported globals -------------------------------------------------------

/// `int gl_lightmap_format` (never assigned; kept for the C readers).
#[no_mangle]
pub static mut gl_lightmap_format: c_int = 0;
/// `struct lightmap_s *lightmaps` (read by `cl_parse.c`, `gl_rlight.c`,
/// `gl_rmisc_glue.c`, `model_parse.c`, `sv_main.c`).
#[no_mangle]
pub static mut lightmaps: *mut Lightmap = ptr::null_mut();
/// `int lightmap_count`.
#[no_mangle]
pub static mut lightmap_count: c_int = 0;
/// `qboolean indirect`.
#[no_mangle]
pub static mut indirect: bool = true;
/// `qboolean indirect_ready`.
#[no_mangle]
pub static mut indirect_ready: bool = false;
/// `vulkan_memory_t frame_upload_buffers_memory` (read by `gl_rmisc.rs` and
/// `gl_vidsdl.rs` through the c-sys view).
#[no_mangle]
pub static mut frame_upload_buffers_memory: VulkanMemory = NULL_MEMORY;
/// `VkBuffer bmodel_vertex_buffer` (shared with the RT glue).
#[no_mangle]
pub static mut bmodel_vertex_buffer: vk::Buffer = vk::Buffer::null();
/// `uint32_t bmodel_numverts` (shared with the RT glue).
#[no_mangle]
pub static mut bmodel_numverts: u32 = 0;
/// `VkDeviceAddress bmodel_vertex_buffer_device_address` (shared with the RT
/// glue).
#[no_mangle]
pub static mut bmodel_vertex_buffer_device_address: vk::DeviceAddress = 0;

// ---- private state ----------------------------------------------------------

static mut LAST_LIGHTMAP_ALLOCATED: c_int = 0;
static mut USED_COLUMNS: [[c_int; SHELVES]; MAX_SANITY_LIGHTMAPS as usize] =
    [[0; SHELVES]; MAX_SANITY_LIGHTMAPS as usize];
static mut LIGHTMAP_IDX: [c_int; LM_BINS] = [0; LM_BINS];
static mut SHELF_IDX: [c_int; LM_BINS] = [0; LM_BINS];
static mut COLUMNS: [c_int; LM_BINS] = [0; LM_BINS];
static mut ROWS: [c_int; LM_BINS] = [0; LM_BINS];
/// `unsigned blocklights[LMBLOCK_WIDTH * LMBLOCK_HEIGHT * 3 + 1]` -- the `+1`
/// is what lets the SIMD store read one lane past the last texel.
static mut BLOCKLIGHTS: [u32; LMBLOCK_WIDTH * LMBLOCK_HEIGHT * 3 + 1] =
    [0; LMBLOCK_WIDTH * LMBLOCK_HEIGHT * 3 + 1];

/// `indirectdraw_t`.
#[derive(Clone, Copy)]
struct IndirectDraw {
    texture: *mut Texture,
    lightmap_idx: i16,
    is_bmodel: i16,
    max_indices: c_int,
}
const NO_DRAW: IndirectDraw = IndirectDraw {
    texture: ptr::null_mut(),
    lightmap_idx: 0,
    is_bmodel: 0,
    max_indices: 0,
};
static mut INDIRECT_DRAWS: [IndirectDraw; MAX_INDIRECT_DRAWS] = [NO_DRAW; MAX_INDIRECT_DRAWS];
static mut USED_INDIRECT_DRAWS: c_int = 0;
static mut INDIRECT_BMODEL_START: u32 = 0;
const ZERO_CMD: vk::DrawIndexedIndirectCommand = vk::DrawIndexedIndirectCommand {
    index_count: 0,
    instance_count: 0,
    first_index: 0,
    vertex_offset: 0,
    first_instance: 0,
};
static mut INITIAL_INDIRECT_BUFFER: [vk::DrawIndexedIndirectCommand; MAX_INDIRECT_DRAWS] =
    [ZERO_CMD; MAX_INDIRECT_DRAWS];

/// `combined_brush_deps` -- an 8-byte union: the header entry carries the two
/// counts, then `water_count` warp pointers, then `lm_count` lightmap entries.
#[repr(C)]
#[derive(Clone, Copy)]
struct DepsCounts {
    water_count: c_int,
    lm_count: c_int,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct DepsLightmap {
    lightmap_num: c_int,
    lightmap_styles: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
union CombinedBrushDeps {
    counts: DepsCounts,
    update_warp: *mut u32,
    lm: DepsLightmap,
    bits: u64,
}
const _: () = assert!(core::mem::size_of::<CombinedBrushDeps>() == 8);
const ZERO_DEPS: CombinedBrushDeps = CombinedBrushDeps { bits: 0 };

static mut BRUSH_DEPS_DATA: *mut CombinedBrushDeps = ptr::null_mut();
static mut USED_DEPS_DATA: c_int = 0;
/// `R_AllocDepsData`'s function-local `static int last`.
static mut ALLOC_DEPS_LAST: c_int = 0;
/// `UpdateIndirectStructs`' function-local `static int last`.
static mut UPDATE_INDIRECT_LAST: c_int = 0;

static mut BMODEL_MEMORY: VulkanMemory = NULL_MEMORY;
static mut SURFACE_DATA_BUFFER_MEMORY: VulkanMemory = NULL_MEMORY;
static mut SURFACE_SUBMODELS_BUFFER_MEMORY: VulkanMemory = NULL_MEMORY;
static mut WORKGROUP_BOUNDS_BUFFER_MEMORY: VulkanMemory = NULL_MEMORY;
static mut INDIRECT_BUFFER_MEMORY: VulkanMemory = NULL_MEMORY;
static mut INDIRECT_INDEX_BUFFER_MEMORY: VulkanMemory = NULL_MEMORY;
static mut DYN_VISIBILITY_BUFFER_MEMORY: VulkanMemory = NULL_MEMORY;
static mut VERTEX_SUBMODELS_BUFFER_MEMORY: VulkanMemory = NULL_MEMORY;
static mut SURFACE_DATA_BUFFER: vk::Buffer = vk::Buffer::null();
static mut SURFACE_SUBMODELS_BUFFER: vk::Buffer = vk::Buffer::null();
static mut NUM_SURFACES: c_int = 0;
static mut INDIRECT_BUFFER: vk::Buffer = vk::Buffer::null();
static mut INDIRECT_INDEX_BUFFER: vk::Buffer = vk::Buffer::null();
static mut DYN_VISIBILITY_BUFFER: vk::Buffer = vk::Buffer::null();
static mut DYN_VISIBILITY_OFFSET: u32 = 0;
static mut DYN_VISIBILITY_VIEW: *mut u8 = ptr::null_mut();
static mut LIGHTSTYLES_SCALES_BUFFER: vk::Buffer = vk::Buffer::null();
static mut LIGHTS_BUFFER: vk::Buffer = vk::Buffer::null();
static mut SUBMODEL_TRANSFORMS_BUFFER: vk::Buffer = vk::Buffer::null();
static mut LIGHTSTYLES_SCALES_BUFFER_MAPPED: *mut f32 = ptr::null_mut();
static mut LIGHTS_BUFFER_MAPPED: *mut LmComputeLight = ptr::null_mut();
static mut SUBMODEL_TRANSFORMS_BUFFER_MAPPED: *mut f32 = ptr::null_mut();
static mut VERTEX_SUBMODELS_BUFFER: vk::Buffer = vk::Buffer::null();
static mut BMODEL_INSTANCES_BUFFER: vk::Buffer = vk::Buffer::null();
static mut BMODEL_INSTANCES_BUFFER_MAPPED: *mut BModelInstance = ptr::null_mut();
static BMODEL_INSTANCE_CLAIMS: [AtomicU64; MAX_MODELS] = [const { AtomicU64::new(0) }; MAX_MODELS];
static mut BMODEL_INSTANCES_INDEX: c_int = 0;
static mut NUM_WORLDMODEL_SUBMODELS: c_int = 0;
static mut CURRENT_COMPUTE_BUFFER_INDEX: c_int = 0;

/// `R_UpdateLightmapsAndIndirect`'s function-local statics.
static mut CACHED_DLIGHTS: [LmComputeLight; MAX_DLIGHTS] = [LmComputeLight {
    origin: [0.0; 3],
    radius: 0.0,
    color: [0.0; 3],
    minlight: 0.0,
    cone_dir: [0.0; 3],
    cone_cos: 0.0,
}; MAX_DLIGHTS];
static mut NUM_CACHED_DLIGHTS: c_int = 0;

/// `BuildSurfaceDisplayList` context (`r_pcurrentvertbase`, `currentmodel`).
static mut R_PCURRENTVERTBASE: *mut MVertex = ptr::null_mut();
static mut CURRENTMODEL: *mut QModel = ptr::null_mut();

// ---- small helpers ----------------------------------------------------------

#[inline]
fn cvar_value(cvar: *const cvar_t) -> f32 {
    // SAFETY: the C cvar statics live for the program.
    unsafe { (*cvar).value }
}

#[inline]
fn cheatsafe_lightmap() -> bool {
    // SAFETY: plain bool written by the cheat-safe refresh on the main thread.
    unsafe { *ptr::addr_of!(c::render::r_lightmap_cheatsafe) }
}

#[inline]
fn cheatsafe_fullbright() -> bool {
    // SAFETY: as above.
    unsafe { *ptr::addr_of!(c::render::r_fullbright_cheatsafe) }
}

#[inline]
fn use_simd() -> bool {
    // SAFETY: `use_simd` is a C bool set once at init.
    unsafe { *ptr::addr_of!(c::render::use_simd) }
}

/// `R_UseOIT` (`glquake.h` static inline).
#[inline]
fn r_use_oit() -> bool {
    // SAFETY: `frame_oit_mode` is a plain int written once per frame.
    unsafe { *ptr::addr_of!(c::render::frame_oit_mode) != 0 }
}

#[inline]
fn r_framecount() -> c_int {
    // SAFETY: plain int written on the main thread.
    unsafe { *ptr::addr_of!(c::render::r_framecount) }
}

#[inline]
fn lightstyle_value(style: usize) -> c_int {
    // SAFETY: `d_lightstylevalue` is a C int array written before the frame.
    unsafe { (*ptr::addr_of!(c::render::d_lightstylevalue))[style] }
}

#[inline]
unsafe fn worldmodel() -> *mut QModel {
    // SAFETY: `cl` is the C client state; read on the main thread or inside
    // the frame's tasks, when `worldmodel` is stable.
    unsafe { (*ptr::addr_of!(cl)).worldmodel }
}

#[inline]
unsafe fn cl_time() -> f64 {
    // SAFETY: as above.
    unsafe { (*ptr::addr_of!(cl)).time }
}

#[inline]
unsafe fn vieworg() -> Vec3 {
    // SAFETY: `r_refdef` is the C refresh definition, stable inside a frame.
    unsafe { (*ptr::addr_of!(r_refdef)).vieworg }
}

#[inline]
unsafe fn atomic_u32<'a>(p: *mut u32) -> &'a AtomicU32 {
    // SAFETY: the caller passes a 4-byte-aligned, live `atomic_uint32_t`.
    unsafe { AtomicU32::from_ptr(p) }
}

#[inline]
fn floats_to_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_ne_bytes()).collect()
}

/// `ENTALPHA_DECODE`.
#[inline]
fn entalpha_decode(a: u8) -> f32 {
    if a == 0 {
        1.0
    } else {
        f32::from(a - 1) / 254.0
    }
}

/// `ENTSCALE_DECODE`.
#[inline]
fn entscale_decode(s: u8) -> f32 {
    f32::from(s) / 16.0
}

/// `Q_log2`.
#[inline]
fn q_log2(val: c_int) -> c_int {
    31 - (val as u32).leading_zeros() as c_int
}

/// `q_align`.
#[inline]
fn q_align(value: u64, alignment: u64) -> u64 {
    value.div_ceil(alignment) * alignment
}

#[inline]
fn lightmap_of(index: c_int) -> *mut Lightmap {
    // SAFETY: `lightmaps` has `lightmap_count` entries; the caller's index
    // comes from a surface built against this atlas set.
    unsafe { lightmaps.add(index as usize) }
}

#[inline]
fn deps_at(index: c_int) -> *mut CombinedBrushDeps {
    // SAFETY: `BRUSH_DEPS_DATA` has `USED_DEPS_DATA` entries.
    unsafe { BRUSH_DEPS_DATA.add(index as usize) }
}

#[inline]
unsafe fn model_name_submodel(model: *mut QModel) -> c_int {
    // SAFETY: the caller checked `name[0] == '*'`; the name is NUL-terminated
    // inside its 64-byte array.
    unsafe {
        let name = CStr::from_ptr((*model).name.as_ptr()).to_bytes();
        c_atoi(&name[1..])
    }
}

#[inline]
unsafe fn model_is_submodel(model: *mut QModel) -> bool {
    // SAFETY: the caller passes a loaded model.
    unsafe { (*model).name[0] == b'*' as c_char }
}

/// `TEXTYPE_ISLIQUID`.
#[inline]
fn textype_is_liquid(type_: c_int) -> bool {
    (TEXTYPE_FIRSTLIQUID..=TEXTYPE_LASTLIQUID).contains(&type_)
}

#[inline]
unsafe fn model_precache(index: usize) -> *mut QModel {
    // SAFETY: `cl.model_precache` is the C precache table.
    unsafe { (*ptr::addr_of!(cl)).model_precache[index] }
}

/// Rotates `v` (the view origin relative to the entity) into the entity's
/// local frame when it has any rotation (the `if (e->angles[...])` block that
/// `R_DrawBrushModel` repeats).
unsafe fn local_modelorg(e: *mut Entity, mut modelorg: Vec3) -> Vec3 {
    // SAFETY: `e` is a live entity.
    unsafe {
        let angles = (*e).angles;
        if angles[0] != 0.0 || angles[1] != 0.0 || angles[2] != 0.0 {
            let temp = modelorg;
            let (mut forward, mut right, mut up) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 3]);
            angle_vectors(&angles, &mut forward, &mut right, &mut up);
            modelorg[0] = dot_product(&temp, &forward);
            modelorg[1] = -dot_product(&temp, &right);
            modelorg[2] = dot_product(&temp, &up);
        }
        modelorg
    }
}

// ---- lightmap bins ----------------------------------------------------------

/// `SizeToBin`.
fn size_to_bin(mut size: c_int) -> c_int {
    size -= 1;
    if size < LM_BIN_E * 2 + 1 {
        return size;
    }
    let bc = q_log2(size / LM_BIN_E);
    (size >> bc) + LM_BIN_E * bc + 1
}

/// `BinToSize`.
fn bin_to_size(mut bin: c_int) -> c_int {
    if bin < LM_BIN_E * 2 + 1 {
        return bin + 1;
    }
    bin -= 1;
    let bc = bin / LM_BIN_E - 1;
    (bin % LM_BIN_E + LM_BIN_E + 1) << bc
}

// ---- brush dependencies -----------------------------------------------------

/// `R_AllocDepsData` -- appends the dependency list (header + entries) to the
/// growing table unless it repeats the previous one; returns its index.
unsafe fn alloc_deps_data(items: &[CombinedBrushDeps]) -> c_int {
    // SAFETY: called from the map-load path on the main thread; the table
    // pointer/size statics are only touched here and in `GL_BuildLightmaps`.
    unsafe {
        let item_count = items[0].counts.water_count + items[0].counts.lm_count;
        let last = ALLOC_DEPS_LAST;
        if last < USED_DEPS_DATA {
            let mut same = (*deps_at(last)).bits == items[0].bits;
            if same {
                for i in 0..item_count as usize {
                    if (*deps_at(last + 1 + i as c_int)).bits != items[1 + i].bits {
                        same = false;
                        break;
                    }
                }
            }
            if same {
                return last;
            }
        }
        if USED_DEPS_DATA == 0 {
            BRUSH_DEPS_DATA = c::Mem_Alloc(
                INITIAL_BRUSH_DEPS_SIZE as usize * core::mem::size_of::<CombinedBrushDeps>(),
            )
            .cast::<CombinedBrushDeps>();
        }
        for item in items.iter().take(item_count as usize + 1) {
            if USED_DEPS_DATA >= INITIAL_BRUSH_DEPS_SIZE
                && (USED_DEPS_DATA & (USED_DEPS_DATA - 1)) == 0
            {
                BRUSH_DEPS_DATA = c::Mem_Realloc(
                    BRUSH_DEPS_DATA.cast::<c_void>(),
                    USED_DEPS_DATA as usize * 2 * core::mem::size_of::<CombinedBrushDeps>(),
                )
                .cast::<CombinedBrushDeps>();
            }
            *deps_at(USED_DEPS_DATA) = *item;
            USED_DEPS_DATA += 1;
        }
        ALLOC_DEPS_LAST = USED_DEPS_DATA - item_count - 1;
        ALLOC_DEPS_LAST
    }
}

/// `R_CalcDeps` -- collects the water textures and lightmaps a bmodel (or a
/// world leaf) touches.
unsafe fn calc_deps(model: *mut QModel, leaf: *mut quake_types::model_mem::MLeaf) {
    // SAFETY: map-load path; `model` or `leaf` is live, the world is loaded.
    unsafe {
        const MAX_WATER: usize = 256;
        let mut deps = [ZERO_DEPS; 1 + MAX_WATER + MAX_SANITY_LIGHTMAPS as usize];
        let num_surfs = if !model.is_null() {
            (*model).nummodelsurfaces
        } else {
            (*leaf).nummarksurfaces
        };
        let wm = worldmodel();
        let surf_at = |i: c_int| -> *mut MSurface {
            if !model.is_null() {
                (*model)
                    .surfaces
                    .add(((*model).firstmodelsurface + i) as usize)
            } else {
                (*wm)
                    .surfaces
                    .add(*(*leaf).firstmarksurface.add(i as usize) as usize)
            }
        };
        let mut water_count: c_int = 0;
        for i in 0..num_surfs {
            let psurf = surf_at(i);
            let t = (*(*psurf).texinfo).texture;
            let c0 = (*t).name[0];
            if c0 == b'*' as c_char || c0 == b'!' as c_char {
                let warp = ptr::addr_of_mut!((*t).update_warp);
                let mut found = false;
                for d in deps.iter().take(1 + water_count as usize).skip(1) {
                    if d.update_warp == warp {
                        found = true;
                        break;
                    }
                }
                if !found {
                    water_count += 1;
                    if water_count as usize > MAX_WATER {
                        c::Sys_Error(
                            c"A single bmodel / world leaf is using more than 256 different water textures"
                                .as_ptr(),
                        );
                    }
                    deps[water_count as usize] = ZERO_DEPS;
                    deps[water_count as usize].update_warp = warp;
                }
            }
        }
        let mut lm_count: c_int = 0;
        for i in 0..num_surfs {
            let psurf = surf_at(i);
            let lmnum = (*psurf).lightmaptexturenum;
            if lmnum < 0 {
                continue;
            }
            let base = 1 + water_count as usize;
            let mut found = false;
            for d in deps.iter_mut().skip(base).take(lm_count as usize) {
                if d.lm.lightmap_num == lmnum {
                    d.lm.lightmap_styles |= (*psurf).styles_bitmap;
                    found = true;
                    break;
                }
            }
            if !found {
                lm_count += 1;
                deps[(water_count + lm_count) as usize] = CombinedBrushDeps {
                    lm: DepsLightmap {
                        lightmap_num: lmnum,
                        lightmap_styles: (*psurf).styles_bitmap,
                    },
                };
            }
        }
        deps[0] = CombinedBrushDeps {
            counts: DepsCounts {
                water_count,
                lm_count,
            },
        };
        let index = alloc_deps_data(&deps);
        if !model.is_null() {
            (*model).combined_deps = index;
        } else {
            (*leaf).combined_deps = index;
        }
    }
}

/// `R_MarkDeps` -- flags the water textures for a warp update and the
/// lightmaps as modified for this worker.
///
/// # Safety
/// `model` is a loaded brush model and `leaf` one of its leaves (or null).
#[no_mangle]
pub unsafe extern "C" fn R_MarkDeps(combined_deps: c_int, worker_index: c_int) {
    // SAFETY: `combined_deps` indexes the table built by `GL_SetupIndirectDraws`;
    // `update_warp` is a C `atomic_uint32_t` inside a live texture.
    unsafe {
        let mut deps = deps_at(combined_deps);
        let water_count = (*deps).counts.water_count;
        let lm_count = (*deps).counts.lm_count;
        for _ in 0..water_count {
            deps = deps.add(1);
            atomic_u32((*deps).update_warp).store(1, Ordering::Relaxed);
        }
        deps = deps.add(1);
        for _ in 0..lm_count {
            let lm = (*deps).lm;
            (*lightmap_of(lm.lightmap_num)).modified[worker_index as usize] |= lm.lightmap_styles;
            deps = deps.add(1);
        }
    }
}

// ---- texture animation / polys ----------------------------------------------

/// `R_TextureAnimation` -- returns the proper texture for a given time and
/// base texture.
///
/// # Safety
/// `base` is a live texture of a loaded brush model; `cl.time` is readable.
#[no_mangle]
pub unsafe extern "C" fn R_TextureAnimation(mut base: *mut Texture, frame: c_int) -> *mut Texture {
    // SAFETY: `base` is a live texture of a loaded model; the animation
    // cycle is checked for breakage/infinity exactly like the C.
    unsafe {
        if frame != 0 && !(*base).alternate_anims.is_null() {
            base = (*base).alternate_anims;
        }
        if (*base).anim_total == 0 {
            return base;
        }
        let relative = ((cl_time() * 10.0) as c_int) % (*base).anim_total;
        let mut count = 0;
        while (*base).anim_min > relative || (*base).anim_max <= relative {
            base = (*base).anim_next;
            if base.is_null() {
                c::Sys_Error(c"R_TextureAnimation: broken cycle".as_ptr());
            }
            count += 1;
            if count > 100 {
                c::Sys_Error(c"R_TextureAnimation: infinite cycle".as_ptr());
            }
        }
        base
    }
}

/// `DrawGLPoly` -- a single triangle-fan polygon through the dynamic vertex
/// buffer (and the dynamic index buffer past `FAN_INDEX_BUFFER_SIZE`).
///
/// # Safety
/// Rendering path inside the frame; `cbx` is recording and `p` is a live `glpoly_t` chain.
#[no_mangle]
pub unsafe extern "C" fn DrawGLPoly(
    cbx: *mut CbContext,
    p: *mut GlPoly,
    color: *mut c_float,
    alpha: c_float,
) {
    // SAFETY: rendering path inside the frame; `p` is a surface display list
    // whose `verts` really has `numverts` entries.
    unsafe {
        let numverts = (*p).numverts;
        let numtriangles = numverts - 2;
        let numindices = numtriangles * 3;
        let color = [*color, *color.add(1), *color.add(2)];
        let cmd = (*cbx).cb;
        let device = device();

        let va = with_ctx(|ctx| {
            DYN.vertex_allocate(
                ctx,
                (numverts as usize * core::mem::size_of::<BasicVertex>()) as u32,
            )
        });
        let vertices = va.data.cast::<BasicVertex>();
        let verts = (*p).verts.as_ptr().cast::<f32>();
        for i in 0..numverts as usize {
            let v = verts.add(i * VERTEXSIZE);
            let out = &mut *vertices.add(i);
            out.position = [*v, *v.add(1), *v.add(2)];
            out.texcoord = [*v.add(3), *v.add(4)];
            out.color = [
                (color[0] * 255.0) as u8,
                (color[1] * 255.0) as u8,
                (color[2] * 255.0) as u8,
                (alpha * 255.0) as u8,
            ];
        }

        if numindices as usize > FAN_INDEX_BUFFER_SIZE {
            let ia = with_ctx(|ctx| DYN.index_allocate(ctx, (numindices * 2) as u32));
            let indices = ia.data.cast::<u16>();
            for i in 0..numtriangles as usize {
                *indices.add(i * 3) = 0;
                *indices.add(i * 3 + 1) = 1 + i as u16;
                *indices.add(i * 3 + 2) = 2 + i as u16;
            }
            device.cmd_bind_index_buffer(cmd, ia.buffer, ia.buffer_offset, vk::IndexType::UINT16);
        } else {
            let fan = with_ctx(|ctx| (*ctx.vg.as_ptr()).fan_index_buffer);
            device.cmd_bind_index_buffer(cmd, fan, 0, vk::IndexType::UINT16);
        }
        device.cmd_bind_vertex_buffers(cmd, 0, &[va.buffer], &[va.buffer_offset]);
        let procs = with_ctx(|ctx| CmdProcs::new(ctx.vg));
        cb::draw_indexed(&procs, cmd, numindices as u32, 1, 0, 0, 0);
    }
}

// ---- brush models -----------------------------------------------------------

/// After `R_ChainSurface`: either the CPU dynamic lightmap path or the GPU
/// dirty bits.
#[inline]
unsafe fn after_chain(surf: *mut MSurface, worker_index: c_int) {
    // SAFETY: `surf` is live; `lightmaps` covers its lightmap index.
    unsafe {
        if cvar_value(ptr::addr_of!(c::render::r_gpulightmapupdate)) == 0.0 {
            R_RenderDynamicLightmaps(surf);
        } else if (*surf).lightmaptexturenum >= 0 {
            (*lightmap_of((*surf).lightmaptexturenum)).modified[worker_index as usize] |=
                (*surf).styles_bitmap;
        }
    }
}

#[inline]
unsafe fn surface_faces(surf: *mut MSurface, dot: f32) -> bool {
    // SAFETY: `surf` is live.
    unsafe {
        let back = (*surf).flags & SURF_PLANEBACK != 0;
        (back && dot < -BACKFACE_EPSILON) || (!back && dot > BACKFACE_EPSILON)
    }
}

/// `R_RecursiveNode` -- front-to-back chaining of a bmodel's node tree.
#[allow(clippy::too_many_arguments)]
unsafe fn recursive_node(
    node: *mut MNode,
    model: *mut QModel,
    modelorg: &Vec3,
    chain: c_int,
    brushpolys: &mut c_int,
    surfs_visited: &mut c_int,
    worker_index: c_int,
    water_transparent_only: bool,
) {
    // SAFETY: `node` is inside `model`'s node array; leaves have negative
    // contents and end the recursion.
    unsafe {
        if (*node).contents < 0 {
            return;
        }
        let plane = (*node).plane;
        let dot = if (*plane).type_ < 3 {
            modelorg[(*plane).type_ as usize]
        } else {
            dot_product(modelorg, &(*plane).normal)
        } - (*plane).dist;
        recursive_node(
            (*node).children[usize::from(dot < 0.0)],
            model,
            modelorg,
            chain,
            brushpolys,
            surfs_visited,
            worker_index,
            water_transparent_only,
        );
        let mut surf = (*model).surfaces.add((*node).firstsurface as usize);
        for _ in 0..(*node).numsurfaces {
            if surface_faces(surf, dot)
                && (!water_transparent_only
                    || ((*surf).flags & SURF_DRAWTURB != 0
                        && c::render::GL_WaterAlphaForSurface(surf.cast::<c_void>()) != 1.0))
            {
                R_ChainSurface(surf, chain);
                *brushpolys += 1;
                after_chain(surf, worker_index);
            }
            surf = surf.add(1);
        }
        *surfs_visited += (*node).numsurfaces as c_int;
        recursive_node(
            (*node).children[usize::from(dot >= 0.0)],
            model,
            modelorg,
            chain,
            brushpolys,
            surfs_visited,
            worker_index,
            water_transparent_only,
        );
    }
}

/// `R_ClearBModelInstanceClaims`.
///
/// # Safety
/// Called once per frame before any brush model draw.
#[no_mangle]
pub unsafe extern "C" fn R_ClearBModelInstanceClaims() {
    // SAFETY: main thread, before the frame's entity draws.
    unsafe {
        BMODEL_INSTANCES_INDEX = CURRENT_COMPUTE_BUFFER_INDEX;
        for claim in BMODEL_INSTANCE_CLAIMS
            .iter()
            .take(NUM_WORLDMODEL_SUBMODELS.max(0) as usize)
        {
            claim.store(0, Ordering::Relaxed);
        }
    }
}

/// `R_ClaimBModelInstance` -- the first entity to claim a submodel's instance
/// slot this frame owns it.
fn claim_bmodel_instance(e: *mut Entity, submodel: c_int) -> bool {
    let e_bits = e as usize as u64;
    match BMODEL_INSTANCE_CLAIMS[submodel as usize].compare_exchange(
        0,
        e_bits,
        Ordering::SeqCst,
        Ordering::SeqCst,
    ) {
        Ok(_) => true,
        Err(expected) => expected == e_bits,
    }
}

/// `R_IndirectBrush` -- can this entity go through the indirect path?
///
/// # Safety
/// Rendering path inside the frame; `e` is a live entity with a loaded brush model.
#[no_mangle]
pub unsafe extern "C" fn R_IndirectBrush(e: *mut Entity) -> bool {
    // SAFETY: `e` is a live entity with a loaded brush model.
    unsafe {
        let model = (*e).model;
        let transparent_entity = entalpha_decode((*e).alpha) != 1.0;
        let has_water = (*deps_at((*model).combined_deps)).counts.water_count != 0;
        let fixed_alpha_water = (*e).alpha != ENTALPHA_DEFAULT && has_water;
        let alpha_sorted = !r_use_oit() && (transparent_entity || has_water);
        if !*ptr::addr_of!(indirect)
            || transparent_entity
            || fixed_alpha_water
            || alpha_sorted
            || (*e).frame != 0
            || !model_is_submodel(model)
        {
            return false;
        }
        let o = (*e).origin;
        let a = (*e).angles;
        let transformed = o[0] != 0.0
            || o[1] != 0.0
            || o[2] != 0.0
            || a[0] != 0.0
            || a[1] != 0.0
            || a[2] != 0.0
            || entscale_decode((*e).netstate.scale) != 1.0;
        if transformed && ((*model).used_specials & SURF_DRAWSKY) != 0 {
            return false;
        }
        let submodel = model_name_submodel(model);
        if submodel <= 0 || submodel >= NUM_WORLDMODEL_SUBMODELS {
            return !transformed;
        }
        claim_bmodel_instance(e, submodel)
    }
}

/// `R_DrawBrushModel`.
///
/// # Safety
/// Rendering path inside the frame; `cbx` is recording and `e` is a live entity with a loaded brush model.
#[no_mangle]
pub unsafe extern "C" fn R_DrawBrushModel(
    cbx: *mut CbContext,
    e: *mut Entity,
    chain: c_int,
    brushpolys: *mut c_int,
    sort: bool,
    water_opaque_only: bool,
    water_transparent_only: bool,
) {
    // SAFETY: rendering path inside the frame; `e` carries a loaded brush
    // model; `cbx` is recording.
    unsafe {
        if c::render::R_CullModelForEntity(e.cast::<c_void>()) {
            return;
        }
        let clmodel = (*e).model;
        let wm = worldmodel();
        let view = vieworg();
        let origin = (*e).origin;
        let mut modelorg = [
            view[0] - origin[0],
            view[1] - origin[1],
            view[2] - origin[2],
        ];

        if !water_opaque_only && !water_transparent_only && R_IndirectBrush(e) {
            let submodel = model_name_submodel(clmodel);
            if submodel > 0 && submodel < NUM_WORLDMODEL_SUBMODELS {
                let instance = BMODEL_INSTANCES_BUFFER_MAPPED
                    .add(BMODEL_INSTANCES_INDEX as usize * MAX_MODELS + submodel as usize);
                let mut e_angles = (*e).angles;
                e_angles[0] = -e_angles[0];
                let mut mm = [0.0f32; 16];
                identity_matrix(&mut mm);
                c::render::R_RotateForEntity(
                    mm.as_mut_ptr(),
                    (*e).origin.as_mut_ptr(),
                    e_angles.as_mut_ptr(),
                    (*e).netstate.scale,
                );
                for row in 0..3 {
                    for col in 0..4 {
                        (*instance).transform[row][col] = mm[col * 4 + row];
                    }
                }
                modelorg = local_modelorg(e, modelorg);
                (*instance).local_vieworg = [modelorg[0], modelorg[1], modelorg[2], 0.0];
            }
            let start = (*clmodel).firstmodelsurface as u32;
            let end = start + (*clmodel).nummodelsurfaces as u32;
            let startword = (start / 32) as usize;
            let endword = (end / 32) as usize;
            let surfvis = (*wm).surfvis.cast::<u32>();
            if startword == endword {
                atomic_u32(surfvis.add(startword)).fetch_or(
                    (1u32 << (end % 32)).wrapping_sub(1u32 << (start % 32)),
                    Ordering::Relaxed,
                );
            } else {
                let start_bits = 1u32 << (start % 32);
                atomic_u32(surfvis.add(startword)).fetch_or(
                    ((1u64 << 32) - u64::from(start_bits)) as u32,
                    Ordering::Relaxed,
                );
                for i in startword + 1..endword {
                    atomic_u32(surfvis.add(i)).store(0xFFFF_FFFF, Ordering::Relaxed);
                }
                atomic_u32(surfvis.add(endword))
                    .fetch_or((1u32 << (end % 32)).wrapping_sub(1), Ordering::Relaxed);
            }
            R_MarkDeps((*clmodel).combined_deps, c::tasks::Tasks_GetWorkerIndex());
            return;
        }

        modelorg = local_modelorg(e, modelorg);
        let mut psurf = (*clmodel)
            .surfaces
            .add((*clmodel).firstmodelsurface as usize);

        // calculate dynamic lighting for bmodel if it's not an instanced model
        if cvar_value(ptr::addr_of!(c::render::r_gpulightmapupdate)) == 0.0
            && (*clmodel).firstmodelsurface != 0
        {
            let dlights = ptr::addr_of_mut!(c::cl_main::cl_dlights).cast::<c::cl_tent::dlight_t>();
            let now = cl_time();
            for k in 0..MAX_DLIGHTS {
                let dl = dlights.add(k);
                if f64::from((*dl).die) < now || (*dl).radius == 0.0 {
                    continue;
                }
                let mut local_light = *dl;
                let mut lo = local_light.origin;
                lo[0] -= origin[0];
                lo[1] -= origin[1];
                lo[2] -= origin[2];
                lo = local_modelorg(e, lo);
                local_light.origin = lo;
                (*ptr::addr_of_mut!(lightmap_dlight_origins))[k] = lo;
                R_MarkLights(
                    &mut local_light,
                    k as c_int,
                    (*clmodel)
                        .nodes
                        .add((*clmodel).hulls[0].firstclipnode as usize),
                );
            }
        }

        let mut e_angles = (*e).angles;
        e_angles[0] = -e_angles[0];
        let mut model_matrix = [0.0f32; 16];
        identity_matrix(&mut model_matrix);
        c::render::R_RotateForEntity(
            model_matrix.as_mut_ptr(),
            (*e).origin.as_mut_ptr(),
            e_angles.as_mut_ptr(),
            (*e).netstate.scale,
        );
        let view_projection = with_ctx(|ctx| (*ctx.vg.as_ptr()).view_projection_matrix);
        let mut mvp = view_projection;
        matrix_multiply(&mut mvp, &model_matrix);
        let procs = with_ctx(|ctx| CmdProcs::new(ctx.vg));
        cb::push_constants(
            &procs,
            &*cbx,
            vk::ShaderStageFlags::ALL_GRAPHICS,
            0,
            &floats_to_bytes(&mvp),
        );

        R_ClearTextureChains(clmodel, chain);
        let worker_index = c::tasks::Tasks_GetWorkerIndex();

        if sort && !(*clmodel).bogus_tree {
            let head = (*clmodel)
                .nodes
                .add((*clmodel).hulls[0].firstclipnode as usize);
            let mut surfs_visited = 0;
            recursive_node(
                head,
                clmodel,
                &modelorg,
                chain,
                &mut *brushpolys,
                &mut surfs_visited,
                worker_index,
                water_transparent_only,
            );
            if surfs_visited != (*clmodel).nummodelsurfaces {
                c::Con_DPrintf(
                    c"model %s nummodelsurfaces %d != node tree numsurfaces sum %d\n".as_ptr(),
                    (*clmodel).name.as_ptr(),
                    (*clmodel).nummodelsurfaces,
                    surfs_visited,
                );
                (*clmodel).bogus_tree = true;
                R_ClearTextureChains(clmodel, chain);
            }
        }

        if !sort || (*clmodel).bogus_tree {
            for _ in 0..(*clmodel).nummodelsurfaces {
                let flags = (*psurf).flags;
                if water_opaque_only
                    && flags & SURF_DRAWTURB != 0
                    && c::render::GL_WaterAlphaForSurface(psurf.cast::<c_void>()) != 1.0
                {
                    psurf = psurf.add(1);
                    continue;
                }
                if water_transparent_only
                    && (flags & SURF_DRAWTURB == 0
                        || c::render::GL_WaterAlphaForSurface(psurf.cast::<c_void>()) == 1.0)
                {
                    psurf = psurf.add(1);
                    continue;
                }
                let pplane = (*psurf).plane;
                let dot = dot_product(&modelorg, &(*pplane).normal) - (*pplane).dist;
                if surface_faces(psurf, dot) {
                    R_ChainSurface(psurf, chain);
                    *brushpolys += 1;
                    after_chain(psurf, worker_index);
                }
                psurf = psurf.add(1);
            }
        }

        if !water_transparent_only {
            R_DrawTextureChains(cbx, clmodel, e, chain);
        }
        if (*clmodel).used_specials & SURF_DRAWTURB != 0 {
            R_DrawTextureChains_Water(
                cbx,
                clmodel,
                e,
                chain,
                water_opaque_only,
                water_transparent_only,
            );
        }
        cb::push_constants(
            &procs,
            &*cbx,
            vk::ShaderStageFlags::ALL_GRAPHICS,
            0,
            &floats_to_bytes(&view_projection),
        );
    }
}

/// `R_DrawBrushModel_ShowTris`.
///
/// # Safety
/// As `R_DrawBrushModel`.
#[no_mangle]
pub unsafe extern "C" fn R_DrawBrushModel_ShowTris(cbx: *mut CbContext, e: *mut Entity) {
    // SAFETY: rendering path inside the frame; `e` carries a loaded brush
    // model. The C negates `e->angles[0]` in place around the rotate call.
    unsafe {
        let mut color = [1.0f32, 1.0, 1.0];
        if c::render::R_CullModelForEntity(e.cast::<c_void>()) || R_IndirectBrush(e) {
            return;
        }
        let clmodel = (*e).model;
        let view = vieworg();
        let origin = (*e).origin;
        let modelorg = local_modelorg(
            e,
            [
                view[0] - origin[0],
                view[1] - origin[1],
                view[2] - origin[2],
            ],
        );
        let mut psurf = (*clmodel)
            .surfaces
            .add((*clmodel).firstmodelsurface as usize);

        (*e).angles[0] = -(*e).angles[0];
        let mut model_matrix = [0.0f32; 16];
        identity_matrix(&mut model_matrix);
        c::render::R_RotateForEntity(
            model_matrix.as_mut_ptr(),
            (*e).origin.as_mut_ptr(),
            (*e).angles.as_mut_ptr(),
            (*e).netstate.scale,
        );
        (*e).angles[0] = -(*e).angles[0];

        let view_projection = with_ctx(|ctx| (*ctx.vg.as_ptr()).view_projection_matrix);
        let mut mvp = view_projection;
        matrix_multiply(&mut mvp, &model_matrix);

        let variant = cb::main_pass_pipeline_variant((*cbx).render_pass_index);
        let pipeline = with_ctx(|ctx| {
            if cvar_value(ptr::addr_of!(c::render::r_showtris)) == 1.0 {
                (*ctx.vg.as_ptr()).showtris_pipeline[variant]
            } else {
                (*ctx.vg.as_ptr()).showtris_depth_test_pipeline[variant]
            }
        });
        let procs = with_ctx(|ctx| CmdProcs::new(ctx.vg));
        cb::bind_pipeline(&procs, &mut *cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
        cb::push_constants(
            &procs,
            &*cbx,
            vk::ShaderStageFlags::ALL_GRAPHICS,
            0,
            &floats_to_bytes(&mvp),
        );

        for _ in 0..(*clmodel).nummodelsurfaces {
            let pplane = (*psurf).plane;
            let dot = dot_product(&modelorg, &(*pplane).normal) - (*pplane).dist;
            if surface_faces(psurf, dot) {
                DrawGLPoly(
                    cbx,
                    (*psurf).polys.cast::<GlPoly>(),
                    color.as_mut_ptr(),
                    1.0,
                );
            }
            psurf = psurf.add(1);
        }
        cb::push_constants(
            &procs,
            &*cbx,
            vk::ShaderStageFlags::ALL_GRAPHICS,
            0,
            &floats_to_bytes(&view_projection),
        );
    }
}

/// `R_DrawIndirectBrushes` -- one indirect draw per (texture, lightmap,
/// bmodel-ness) bucket, split across the world command buffers by `index`.
///
/// # Safety
/// Rendering path inside the frame; `cbx` is recording and the indirect buffers were built for the current map.
#[no_mangle]
pub unsafe extern "C" fn R_DrawIndirectBrushes(
    cbx: *mut CbContext,
    draw_water: bool,
    transparent_water: bool,
    draw_sky: bool,
    index: c_int,
) {
    // SAFETY: rendering path inside the frame; the indirect buffers were
    // built by `GL_SetupIndirectDraws` for the current world.
    unsafe {
        let device = device();
        let cmd = (*cbx).cb;
        let procs = with_ctx(|ctx| CmdProcs::new(ctx.vg));
        cb::begin_debug_utils_label(&procs, &*cbx, c"Indirect Brushes");

        device.cmd_bind_vertex_buffers(cmd, 0, &[*ptr::addr_of!(bmodel_vertex_buffer)], &[0]);
        device.cmd_bind_index_buffer(cmd, INDIRECT_INDEX_BUFFER, 0, vk::IndexType::UINT32);

        let (layout, null_set, grey_set, bmodel_set, grey_lightmap, depth_format) =
            with_ctx(|ctx| {
                (
                    (*ctx.vg.as_ptr()).world_pipeline_layout.handle,
                    (*(*ptr::addr_of!(c::render::nulltexture)).cast::<GlTexture>()).descriptor_set,
                    (*(*ptr::addr_of!(c::render::greytexture)).cast::<GlTexture>()).descriptor_set,
                    (*ctx.vg.as_ptr()).bmodel_instances_desc_set,
                    (*ptr::addr_of!(c::render::greylightmap)).cast::<GlTexture>(),
                    (*ctx.vg.as_ptr()).depth_format,
                )
            });
        let gfx = vk::PipelineBindPoint::GRAPHICS;
        if !draw_sky {
            device.cmd_bind_descriptor_sets(cmd, gfx, layout, 2, &[null_set], &[]);
            if cheatsafe_lightmap() {
                device.cmd_bind_descriptor_sets(cmd, gfx, layout, 0, &[grey_set], &[]);
            }
            device.cmd_bind_descriptor_sets(cmd, gfx, layout, 4, &[bmodel_set], &[]);
            let instance_base = (BMODEL_INSTANCES_INDEX as u32) * (MAX_MODELS as u32) + 1;
            cb::push_constants(
                &procs,
                &*cbx,
                vk::ShaderStageFlags::ALL_GRAPHICS,
                21 * 4,
                &instance_base.to_ne_bytes(),
            );
        }

        let mut lastfullbright: *mut GlTexture = ptr::null_mut();
        let mut lastlightmap: *mut GlTexture = ptr::null_mut();
        let mut lasttexture: *mut GlTexture = ptr::null_mut();
        let mut last_alpha = f32::MAX;
        let mut last_constant_factor = f32::MAX;

        let used = USED_INDIRECT_DRAWS;
        let part_size = (used + NUM_WORLD_CBX - 1) / NUM_WORLD_CBX;
        let start = if index < 0 { 0 } else { part_size * index };
        let end = if index < 0 {
            used
        } else {
            (part_size * (index + 1)).min(used)
        };
        let palettized = cvar_value(ptr::addr_of!(c::menu::vid_filter)) != 0.0
            && cvar_value(ptr::addr_of!(c::menu::vid_palettize)) != 0.0;
        let fullbrights = cvar_value(ptr::addr_of!(c::render::gl_fullbrights)) != 0.0;
        let zfix = cvar_value(ptr::addr_of!(c::render::gl_zfix)) != 0.0;

        for i in start..end {
            let draw = INDIRECT_DRAWS[i as usize];
            let t = draw.texture;
            let texture = R_TextureAnimation(t, 0);
            let gl_texture = if draw_water {
                (*texture).warpimage
            } else {
                (*texture).gltexture
            }
            .cast::<GlTexture>();
            if !draw_sky && gl_texture.is_null() {
                continue;
            }
            let type_ = (*texture).type_;
            if draw_water != textype_is_liquid(type_) {
                continue;
            }
            if draw_sky != (type_ == TEXTYPE_SKY) {
                continue;
            }
            if !draw_sky && !cheatsafe_lightmap() && lasttexture != gl_texture {
                device.cmd_bind_descriptor_sets(
                    cmd,
                    gfx,
                    layout,
                    0,
                    &[(*gl_texture).descriptor_set],
                    &[],
                );
                lasttexture = gl_texture;
            }

            let mut alpha = 1.0f32;
            if draw_water {
                alpha = c::render::GL_WaterAlphaForTextureType(type_);
                if (alpha < 1.0) != transparent_water {
                    continue;
                }
                if alpha != last_alpha {
                    cb::push_constants(
                        &procs,
                        &*cbx,
                        vk::ShaderStageFlags::ALL_GRAPHICS,
                        20 * 4,
                        &alpha.to_ne_bytes(),
                    );
                    last_alpha = alpha;
                }
            }

            let mut fullbright_enabled = false;
            if !draw_sky && fullbrights {
                let fullbright = (*R_TextureAnimation(t, 0)).fullbright.cast::<GlTexture>();
                if !fullbright.is_null() && !cheatsafe_lightmap() {
                    fullbright_enabled = true;
                    if lastfullbright != fullbright {
                        device.cmd_bind_descriptor_sets(
                            cmd,
                            gfx,
                            layout,
                            2,
                            &[(*fullbright).descriptor_set],
                            &[],
                        );
                        lastfullbright = fullbright;
                    }
                }
            }

            if !draw_sky {
                let alpha_test = type_ == TEXTYPE_CUTOUT;
                let alpha_blend = alpha < 1.0;
                let pipeline_index = usize::from(fullbright_enabled)
                    + if alpha_test { 2 } else { 0 }
                    + if alpha_blend { 4 } else { 0 }
                    + if palettized { 8 } else { 0 };
                let pipeline = world_pipeline(cbx, pipeline_index);
                cb::bind_pipeline(&procs, &mut *cbx, gfx, pipeline);

                let use_zbias = INDIRECT_ZBIAS && zfix && draw.is_bmodel != 0;
                let (mut constant_factor, mut slope_factor) = (0.0f32, 0.0f32);
                if use_zbias {
                    if depth_format == vk::Format::D32_SFLOAT_S8_UINT
                        || depth_format == vk::Format::D32_SFLOAT
                    {
                        constant_factor = -4.0;
                        slope_factor = -0.125;
                    } else {
                        constant_factor = -1.0;
                        slope_factor = -0.25;
                    }
                }
                if last_constant_factor != constant_factor {
                    device.cmd_set_depth_bias(cmd, constant_factor, 0.0, slope_factor);
                    last_constant_factor = constant_factor;
                }

                let lm_idx = draw.lightmap_idx;
                let lightmap_texture = if cheatsafe_fullbright() || lm_idx < 0 {
                    grey_lightmap
                } else {
                    (*lightmap_of(c_int::from(lm_idx))).texture
                };
                if lastlightmap != lightmap_texture {
                    device.cmd_bind_descriptor_sets(
                        cmd,
                        gfx,
                        layout,
                        1,
                        &[(*lightmap_texture).descriptor_set],
                        &[],
                    );
                    lastlightmap = lightmap_texture;
                }
            }

            cb::draw_indexed_indirect(
                &procs,
                cmd,
                INDIRECT_BUFFER,
                (i as usize * INDIRECT_CMD_SIZE) as vk::DeviceSize,
                1,
                0,
            );
        }
        cb::end_debug_utils_label(&procs, &*cbx);
    }
}

/// `R_DrawIndirectBrushes_ShowTris`.
///
/// # Safety
/// As `R_DrawIndirectBrushes`.
#[no_mangle]
pub unsafe extern "C" fn R_DrawIndirectBrushes_ShowTris(cbx: *mut CbContext) {
    // SAFETY: rendering path inside the frame.
    unsafe {
        let device = device();
        let cmd = (*cbx).cb;
        let procs = with_ctx(|ctx| CmdProcs::new(ctx.vg));
        let variant = cb::main_pass_pipeline_variant((*cbx).render_pass_index);
        let (pipeline, bmodel_set, multi_draw_indirect) = with_ctx(|ctx| {
            (
                if cvar_value(ptr::addr_of!(c::render::r_showtris)) == 1.0 {
                    (*ctx.vg.as_ptr()).showtris_indirect_pipeline[variant]
                } else {
                    (*ctx.vg.as_ptr()).showtris_indirect_depth_test_pipeline[variant]
                },
                (*ctx.vg.as_ptr()).bmodel_instances_desc_set,
                (*ctx.vg.as_ptr()).multi_draw_indirect,
            )
        });
        cb::bind_pipeline(&procs, &mut *cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
        device.cmd_bind_descriptor_sets(
            cmd,
            vk::PipelineBindPoint::GRAPHICS,
            pipeline.layout.handle,
            4,
            &[bmodel_set],
            &[],
        );
        let instance_base = (BMODEL_INSTANCES_INDEX as u32) * (MAX_MODELS as u32) + 1;
        cb::push_constants(
            &procs,
            &*cbx,
            vk::ShaderStageFlags::ALL_GRAPHICS,
            21 * 4,
            &instance_base.to_ne_bytes(),
        );
        device.cmd_bind_vertex_buffers(cmd, 0, &[*ptr::addr_of!(bmodel_vertex_buffer)], &[0]);
        device.cmd_bind_index_buffer(cmd, INDIRECT_INDEX_BUFFER, 0, vk::IndexType::UINT32);
        let used = USED_INDIRECT_DRAWS;
        if multi_draw_indirect {
            cb::draw_indexed_indirect(
                &procs,
                cmd,
                INDIRECT_BUFFER,
                0,
                used as u32,
                INDIRECT_CMD_SIZE as u32,
            );
        } else {
            for i in 0..used {
                cb::draw_indexed_indirect(
                    &procs,
                    cmd,
                    INDIRECT_BUFFER,
                    (i as usize * INDIRECT_CMD_SIZE) as vk::DeviceSize,
                    1,
                    0,
                );
            }
        }
    }
}

/// `R_RenderDynamicLightmaps` -- the CPU lightmap path: rebuilds the surface's
/// lightmap block when a style changed or a dlight touches it.
///
/// # Safety
/// `fa` is a lit surface of a loaded brush model; called from the frame tasks after the dlight marking.
#[no_mangle]
pub unsafe extern "C" fn R_RenderDynamicLightmaps(fa: *mut MSurface) {
    // SAFETY: `fa` is a live surface with a lightmap; `lightmaps` covers it.
    unsafe {
        if (*fa).flags & SURF_DRAWTILED != 0 {
            return;
        }
        let mut dynamic = false;
        for maps in 0..MAXLIGHTMAPS as usize {
            let style = (*fa).styles[maps];
            if style == 255 {
                break;
            }
            if lightstyle_value(style as usize) != (*fa).cached_light[maps] {
                dynamic = true;
                break;
            }
        }
        if !(dynamic || (*fa).dlightframe == r_framecount() || (*fa).cached_dlight) {
            return;
        }
        if cvar_value(ptr::addr_of!(c::render::r_dynamic)) == 0.0 {
            return;
        }
        let lm = lightmap_of((*fa).lightmaptexturenum);
        (*lm).modified[c::tasks::Tasks_GetWorkerIndex() as usize] = 1;
        let the_rect: *mut GlRect = ptr::addr_of_mut!((*lm).rectchange);
        let light_s = (*fa).light_s;
        let light_t = (*fa).light_t;
        if light_t < c_int::from((*the_rect).t) {
            if (*the_rect).h != 0 {
                (*the_rect).h =
                    ((*the_rect).h as c_int + c_int::from((*the_rect).t) - light_t) as u16;
            }
            (*the_rect).t = light_t as u16;
        }
        if light_s < c_int::from((*the_rect).l) {
            if (*the_rect).w != 0 {
                (*the_rect).w =
                    ((*the_rect).w as c_int + c_int::from((*the_rect).l) - light_s) as u16;
            }
            (*the_rect).l = light_s as u16;
        }
        let smax = (c_int::from((*fa).extents[0]) >> 4) + 1;
        let tmax = (c_int::from((*fa).extents[1]) >> 4) + 1;
        if c_int::from((*the_rect).w) + c_int::from((*the_rect).l) < light_s + smax {
            (*the_rect).w = ((light_s - c_int::from((*the_rect).l)) + smax) as u16;
        }
        if c_int::from((*the_rect).h) + c_int::from((*the_rect).t) < light_t + tmax {
            (*the_rect).h = ((light_t - c_int::from((*the_rect).t)) + tmax) as u16;
        }
        let base = (*lm).data.add(
            light_t as usize * LMBLOCK_WIDTH * LIGHTMAP_BYTES + light_s as usize * LIGHTMAP_BYTES,
        );
        R_BuildLightMap(fa, base, (LMBLOCK_WIDTH * LIGHTMAP_BYTES) as c_int);
    }
}

// ---- lightmap allocation ----------------------------------------------------

/// `AllocBlock` -- shelf allocator over the 1024x1024 atlases, binned by
/// width; returns the lightmap index and the block position.
unsafe fn alloc_block(w: c_int, h: c_int, x: &mut c_int, y: &mut c_int) -> c_int {
    // SAFETY: map-load path on the main thread; `lightmaps` is grown here
    // with `Mem_Realloc` and every new entry is fully initialised.
    unsafe {
        let mut texnum = LAST_LIGHTMAP_ALLOCATED;
        while texnum < MAX_SANITY_LIGHTMAPS as c_int {
            if texnum == lightmap_count {
                lightmap_count += 1;
                lightmaps = c::Mem_Realloc(
                    lightmaps.cast::<c_void>(),
                    core::mem::size_of::<Lightmap>() * lightmap_count as usize,
                )
                .cast::<Lightmap>();
                let lm = lightmap_of(texnum);
                ptr::write_bytes(lm, 0, 1);
                let atlas_bytes = LIGHTMAP_BYTES * LMBLOCK_WIDTH * LMBLOCK_HEIGHT;
                (*lm).data = c::Mem_Alloc(atlas_bytes).cast::<u8>();
                for i in 0..3 {
                    (*lm).lightstyle_data[i] = c::Mem_Alloc(atlas_bytes).cast::<u8>();
                }
                (*lm).surface_indices =
                    c::Mem_Alloc(core::mem::size_of::<u32>() * LMBLOCK_WIDTH * LMBLOCK_HEIGHT)
                        .cast::<u32>();
                ptr::write_bytes((*lm).surface_indices, 0xFF, LMBLOCK_WIDTH * LMBLOCK_HEIGHT);
                (*lm).workgroup_bounds =
                    c::Mem_Alloc(WORKGROUP_BOUNDS_BUFFER_SIZE).cast::<LmComputeWorkgroupBounds>();
                for i in 0..(LMBLOCK_WIDTH / 8) * (LMBLOCK_HEIGHT / 8) {
                    let b = (*lm).workgroup_bounds.add(i);
                    (*b).mins = [f32::MAX; 3];
                    (*b).maxs = [-f32::MAX; 3];
                    (*b).submodel = LM_WORKGROUP_SUBMODEL_EMPTY;
                }
                for l in 0..LM_CULL_ROWS {
                    for k in 0..LM_CULL_COLS {
                        (*lm).global_bounds[l][k].mins = [f32::MAX; 3];
                        (*lm).global_bounds[l][k].maxs = [-f32::MAX; 3];
                    }
                }
                (*lm).cached_light = [-1; MAX_LIGHTSTYLES];
                USED_COLUMNS[texnum as usize] = [0; SHELVES];
                LAST_LIGHTMAP_ALLOCATED = texnum;
            }

            let i = size_to_bin(w) as usize;
            if COLUMNS[i] < 0 || ROWS[i] + h - SHELF_IDX[i] * SHELF_HEIGHT > SHELF_HEIGHT {
                while USED_COLUMNS[LIGHTMAP_IDX[i] as usize][SHELF_IDX[i] as usize]
                    + bin_to_size(i as c_int)
                    > LMBLOCK_WIDTH as c_int
                {
                    SHELF_IDX[i] += 1;
                    if SHELF_IDX[i] < SHELVES as c_int {
                        continue;
                    }
                    SHELF_IDX[i] = 0;
                    LIGHTMAP_IDX[i] += 1;
                    if LIGHTMAP_IDX[i] == lightmap_count {
                        break;
                    }
                }
                if LIGHTMAP_IDX[i] == lightmap_count {
                    texnum += 1;
                    continue;
                }
                COLUMNS[i] = USED_COLUMNS[LIGHTMAP_IDX[i] as usize][SHELF_IDX[i] as usize];
                USED_COLUMNS[LIGHTMAP_IDX[i] as usize][SHELF_IDX[i] as usize] +=
                    bin_to_size(i as c_int);
                ROWS[i] = SHELF_IDX[i] * SHELF_HEIGHT;
            }
            *x = COLUMNS[i];
            *y = ROWS[i];
            ROWS[i] += h;
            return LIGHTMAP_IDX[i];
        }
        c::Sys_Error(c"AllocBlock: full".as_ptr());
    }
}

/// `R_AssignSurfaceIndex`.
unsafe fn assign_surface_index(
    surf: *mut MSurface,
    index: u32,
    mut surface_indices: *mut u32,
    mut stride: c_int,
) {
    // SAFETY: `surface_indices` points inside the atlas' index plane at the
    // surface's block.
    unsafe {
        let width = (c_int::from((*surf).extents[0]) >> 4) + 1;
        let mut height = (c_int::from((*surf).extents[1]) >> 4) + 1;
        stride -= width;
        while height > 0 {
            height -= 1;
            for _ in 0..width {
                *surface_indices = index;
                surface_indices = surface_indices.add(1);
            }
            surface_indices = surface_indices.add(stride as usize);
        }
    }
}

#[inline]
fn step_clamp(remaining: c_int) -> c_int {
    remaining.clamp(1, 8)
}

/// `R_FillLightstyleTextures` -- copies the per-style lightmap samples into
/// the three lightstyle planes (style 3 goes into their alpha channels) and
/// records which styles each cull block uses.
unsafe fn fill_lightstyle_textures(surf: *mut MSurface, lightstyles: &[*mut u8; 3], stride: c_int) {
    // SAFETY: `surf->samples` holds `smax*tmax*3` bytes per style; the
    // destination planes were allocated 1024x1024x4.
    unsafe {
        let smax = (c_int::from((*surf).extents[0]) >> 4) + 1;
        let tmax = (c_int::from((*surf).extents[1]) >> 4) + 1;
        let mut lightmap = (*surf).samples;
        if lightmap.is_null() {
            return;
        }
        let lm = lightmap_of((*surf).lightmaptexturenum);
        let light_s = (*surf).light_s;
        let light_t = (*surf).light_t;
        for maps in 0..MAXLIGHTMAPS as usize {
            let style = (*surf).styles[maps];
            if style == 255 {
                break;
            }
            let mut s = 0;
            while s < smax {
                let mut t = 0;
                while t < tmax {
                    (*lm).used_lightstyles[((light_t + t) as usize) / LM_CULL_BLOCK_H]
                        [((light_s + s) as usize) / LM_CULL_BLOCK_W][style as usize] = 1;
                    t += step_clamp(tmax - t - 1);
                }
                s += step_clamp(smax - s - 1);
            }
            if maps % 4 != 3 {
                let mut outptr = lightstyles[maps / 4 * 3 + maps % 4];
                let mut height = tmax;
                while height > 0 {
                    height -= 1;
                    for _ in 0..smax {
                        *outptr = *lightmap;
                        *outptr.add(1) = *lightmap.add(1);
                        *outptr.add(2) = *lightmap.add(2);
                        *outptr.add(3) = 0;
                        outptr = outptr.add(4);
                        lightmap = lightmap.add(3);
                    }
                    outptr = outptr.add((stride - smax * 4) as usize);
                }
            } else {
                for height in 0..tmax {
                    for i in 0..smax {
                        let off = (i * 4 + 3 + stride * height) as usize;
                        *lightstyles[maps / 4].add(off) = *lightmap;
                        *lightstyles[maps / 4 + 1].add(off) = *lightmap.add(1);
                        *lightstyles[maps / 4 + 2].add(off) = *lightmap.add(2);
                        lightmap = lightmap.add(3);
                    }
                }
            }
        }
    }
}

#[inline]
fn merge_bounds(b: &mut LmComputeWorkgroupBounds, mins: &Vec3, maxs: &Vec3) {
    for i in 0..3 {
        b.mins[i] = b.mins[i].min(mins[i]);
        b.maxs[i] = b.maxs[i].max(maxs[i]);
    }
}

/// `R_AssignWorkgroupBounds` -- grows the compute workgroup / cull block
/// bounds by the surface's poly and tags the blocks with its submodel.
unsafe fn assign_workgroup_bounds(surf: *mut MSurface, submodel: u32) {
    // SAFETY: `surf->polys` was just built; the bounds arrays cover the
    // whole atlas.
    unsafe {
        let lm = lightmap_of((*surf).lightmaptexturenum);
        let bounds = (*lm).workgroup_bounds;
        let smax = (c_int::from((*surf).extents[0]) >> 4) + 1;
        let tmax = (c_int::from((*surf).extents[1]) >> 4) + 1;
        let mut surf_mins = [f32::MAX; 3];
        let mut surf_maxs = [-f32::MAX; 3];
        let poly = (*surf).polys.cast::<GlPoly>();
        let verts = (*poly).verts.as_ptr().cast::<f32>();
        for i in 0..(*poly).numverts as usize {
            let v = verts.add(i * VERTEXSIZE);
            for j in 0..3 {
                surf_mins[j] = surf_mins[j].min(*v.add(j));
                surf_maxs[j] = surf_maxs[j].max(*v.add(j));
            }
        }
        let light_s = (*surf).light_s;
        let light_t = (*surf).light_t;
        let mut s = 0;
        while s < smax {
            let mut t = 0;
            while t < tmax {
                let wx = ((light_s + s) / 8) as usize;
                let wy = ((light_t + t) / 8) as usize;
                let wb = &mut *bounds.add(wx + wy * (LMBLOCK_WIDTH / 8));
                let cx = ((light_s + s) as usize) / LM_CULL_BLOCK_W;
                let cy = ((light_t + t) as usize) / LM_CULL_BLOCK_H;
                if wb.submodel == LM_WORKGROUP_SUBMODEL_EMPTY || wb.submodel == submodel {
                    wb.submodel = submodel;
                } else {
                    wb.submodel = LM_WORKGROUP_SUBMODEL_MIXED;
                }
                merge_bounds(wb, &surf_mins, &surf_maxs);
                if submodel == 0 {
                    merge_bounds(&mut (*lm).global_bounds[cy][cx], &surf_mins, &surf_maxs);
                } else {
                    (*lm).block_has_submodels[cy][cx] = 1;
                }
                t += step_clamp(tmax - t - 1);
            }
            s += step_clamp(smax - s - 1);
        }
    }
}

/// `UpdateIndirectStructs` -- buckets the surface into an indirect draw by
/// (lightmap, texture, bmodel-ness).
unsafe fn update_indirect_structs(surf: *mut MSurface, is_bmodel: bool) {
    // SAFETY: map-load path; `INDIRECT_DRAWS` is sized `MAX_INDIRECT_DRAWS`.
    unsafe {
        let lmnum = (*surf).lightmaptexturenum;
        let texture = (*(*surf).texinfo).texture;
        let extra = 3 * ((*surf).numedges - 2);
        let matches = |d: &IndirectDraw| {
            c_int::from(d.lightmap_idx) == lmnum
                && d.texture == texture
                && (d.is_bmodel != 0) == is_bmodel
        };
        let last = UPDATE_INDIRECT_LAST;
        if last < USED_INDIRECT_DRAWS && matches(&INDIRECT_DRAWS[last as usize]) {
            (*surf).indirect_idx = last;
            INDIRECT_DRAWS[last as usize].max_indices += extra;
            return;
        }
        let mut i = 0;
        while i < USED_INDIRECT_DRAWS {
            if matches(&INDIRECT_DRAWS[i as usize]) {
                (*surf).indirect_idx = i;
                UPDATE_INDIRECT_LAST = i;
                INDIRECT_DRAWS[i as usize].max_indices += extra;
                return;
            }
            i += 1;
        }
        if i == MAX_INDIRECT_DRAWS as c_int - 1 {
            indirect_ready = false;
            return;
        }
        USED_INDIRECT_DRAWS += 1;
        (*surf).indirect_idx = i;
        UPDATE_INDIRECT_LAST = i;
        INDIRECT_DRAWS[i as usize] = IndirectDraw {
            texture,
            lightmap_idx: lmnum as i16,
            is_bmodel: i16::from(is_bmodel),
            max_indices: extra,
        };
    }
}

/// `PrepareIndirectDraws`.
unsafe fn prepare_indirect_draws() {
    // SAFETY: map-load path.
    unsafe {
        let mut total: u32 = 0;
        for i in 0..USED_INDIRECT_DRAWS as usize {
            INITIAL_INDIRECT_BUFFER[i] = vk::DrawIndexedIndirectCommand {
                index_count: 0,
                instance_count: 1,
                first_index: total,
                vertex_offset: 0,
                first_instance: 0,
            };
            total += INDIRECT_DRAWS[i].max_indices as u32;
        }
    }
}

/// `GL_CreateSurfaceLightmap`.
unsafe fn create_surface_lightmap(surf: *mut MSurface, surface_index: u32) {
    // SAFETY: map-load path; the surface's block was allocated by
    // `GL_SortSurfaces`.
    unsafe {
        let lm = lightmap_of((*surf).lightmaptexturenum);
        let block = (*surf).light_t as usize * LMBLOCK_WIDTH + (*surf).light_s as usize;
        let base = (*lm).data.add(block * LIGHTMAP_BYTES);
        R_BuildLightMap(surf, base, (LMBLOCK_WIDTH * LIGHTMAP_BYTES) as c_int);
        let surface_indices = (*lm).surface_indices.add(block);
        assign_surface_index(surf, surface_index, surface_indices, LMBLOCK_WIDTH as c_int);
        let lightstyles = [
            (*lm).lightstyle_data[0].add(block * LIGHTMAP_BYTES),
            (*lm).lightstyle_data[1].add(block * LIGHTMAP_BYTES),
            (*lm).lightstyle_data[2].add(block * LIGHTMAP_BYTES),
        ];
        fill_lightstyle_textures(
            surf,
            &lightstyles,
            (LMBLOCK_WIDTH * LIGHTMAP_BYTES) as c_int,
        );
    }
}

/// `BuildSurfaceDisplayList` -- builds the surface's `glpoly_t` (positions,
/// texture and lightmap coordinates).
unsafe fn build_surface_display_list(fa: *mut MSurface) {
    // SAFETY: `CURRENTMODEL`/`R_PCURRENTVERTBASE` were set by the caller for
    // the model that owns `fa`; the poly is over-allocated for `numedges`.
    unsafe {
        let pedges: *mut MEdge = (*CURRENTMODEL).edges;
        let lnumverts = (*fa).numedges;
        let poly = c::Mem_Alloc(
            core::mem::size_of::<GlPoly>()
                + ((lnumverts as isize - 4)
                    * VERTEXSIZE as isize
                    * core::mem::size_of::<f32>() as isize) as usize,
        )
        .cast::<GlPoly>();
        (*poly).next = (*fa).polys.cast::<GlPoly>();
        (*fa).polys = poly.cast::<c_void>();
        (*poly).numverts = lnumverts;
        let texinfo = (*fa).texinfo;
        let texture = (*texinfo).texture;
        let (s0, t0, sdiv, tdiv) = if (*fa).flags & SURF_DRAWTURB != 0 {
            (0.0f32, 0.0f32, 128.0f32, 128.0f32)
        } else {
            (
                (*texinfo).vecs[0][3],
                (*texinfo).vecs[1][3],
                (*texture).width as f32,
                (*texture).height as f32,
            )
        };
        let vecs0: Vec3 = [
            (*texinfo).vecs[0][0],
            (*texinfo).vecs[0][1],
            (*texinfo).vecs[0][2],
        ];
        let vecs1: Vec3 = [
            (*texinfo).vecs[1][0],
            (*texinfo).vecs[1][1],
            (*texinfo).vecs[1][2],
        ];
        let verts = (*poly).verts.as_mut_ptr().cast::<f32>();
        for i in 0..lnumverts {
            let lindex = *(*CURRENTMODEL)
                .surfedges
                .add(((*fa).firstedge + i) as usize);
            let vec: Vec3 = if lindex > 0 {
                (*R_PCURRENTVERTBASE.add((*pedges.add(lindex as usize)).v[0] as usize)).position
            } else {
                (*R_PCURRENTVERTBASE.add((*pedges.add((-lindex) as usize)).v[1] as usize)).position
            };
            let mut s = (dot_product(&vec, &vecs0) + s0) / sdiv;
            let mut t = (dot_product(&vec, &vecs1) + t0) / tdiv;
            let pv = verts.add(i as usize * VERTEXSIZE);
            *pv = vec[0];
            *pv.add(1) = vec[1];
            *pv.add(2) = vec[2];
            *pv.add(3) = s;
            *pv.add(4) = t;
            if (*texture).shift > 0 {
                *pv.add(3) /= (2 * (*texture).shift) as f32;
                *pv.add(4) /= (2 * (*texture).shift) as f32;
            }
            // lightmap texture coordinates
            s = dot_product(&vec, &vecs0) + (*texinfo).vecs[0][3];
            s -= f32::from((*fa).texturemins[0]);
            s += ((*fa).light_s * 16) as f32;
            s += 8.0;
            s /= (LMBLOCK_WIDTH * 16) as f32;
            t = dot_product(&vec, &vecs1) + (*texinfo).vecs[1][3];
            t -= f32::from((*fa).texturemins[1]);
            t += ((*fa).light_t * 16) as f32;
            t += 8.0;
            t /= (LMBLOCK_HEIGHT * 16) as f32;
            *pv.add(5) = s;
            *pv.add(6) = t;
        }
    }
}

// ---- GPU buffers ------------------------------------------------------------

fn bmodel_counter() -> Option<&'static AtomicU32> {
    Some(&num_vulkan_bmodel_allocations)
}

/// Creates a device-local buffer into `memory` (the `R_CreateBuffer` calls
/// with `VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT` required and no preference).
unsafe fn create_device_buffer(
    memory: *mut VulkanMemory,
    size: usize,
    usage: vk::BufferUsageFlags,
    device_address: bool,
    name: &str,
) -> (vk::Buffer, Option<vk::DeviceAddress>) {
    // SAFETY: `memory` is one of this module's `VulkanMemory` statics.
    unsafe {
        with_ctx(|ctx| {
            create_buffer(
                ctx,
                &mut *memory,
                size as u64,
                usage,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
                vk::MemoryPropertyFlags::empty(),
                bmodel_counter(),
                device_address,
                name,
            )
        })
    }
}

unsafe fn free_module_buffer(buffer: *mut vk::Buffer, memory: *mut VulkanMemory) {
    // SAFETY: as above; `free_buffer` is a no-op on a null buffer.
    unsafe {
        with_ctx(|ctx| free_buffer(ctx, *buffer, &mut *memory, bmodel_counter()));
        *buffer = vk::Buffer::null();
    }
}

/// Records a staging-to-buffer copy of `size` bytes and returns the staging
/// pointer to fill (between `begin_copy`/`end_copy`).
unsafe fn stage_copy_to_buffer(dst: vk::Buffer, size: usize) -> *mut u8 {
    // SAFETY: the staging allocation's command buffer is recording.
    unsafe {
        let a = with_ctx(|ctx| STAGING.allocate(ctx, size as i32, 1));
        let region = vk::BufferCopy {
            src_offset: a.buffer_offset as vk::DeviceSize,
            dst_offset: 0,
            size: size as vk::DeviceSize,
        };
        device().cmd_copy_buffer(a.command_buffer, a.buffer, dst, &[region]);
        a.data
    }
}

/// `R_AllocateLightmapComputeBuffers` -- the per-frame host-visible upload
/// buffers shared by the lightmap compute and the indirect dispatch.
///
/// # Safety
/// Main thread during renderer init, after the device is up.
#[no_mangle]
pub unsafe extern "C" fn R_AllocateLightmapComputeBuffers() {
    // SAFETY: init path (`R_InitDynamicBuffers` era) on the main thread.
    unsafe {
        let lightstyles_size = MAX_LIGHTSTYLES * core::mem::size_of::<f32>() * 2;
        let lights_size = MAX_DLIGHTS * 2 * LM_COMPUTE_LIGHT_SIZE * 2;
        let submodel_transforms_size = MAX_MODELS * 12 * core::mem::size_of::<f32>() * 2;
        let bmodel_instances_size = MAX_MODELS * BMODEL_INSTANCE_SIZE * 2;
        c::Sys_Printf(
            c"Allocating lightstyles buffer (%u KB)\n".as_ptr(),
            (lightstyles_size / 1024) as u32,
        );
        c::Sys_Printf(
            c"Allocating lights buffer (%u KB)\n".as_ptr(),
            (lights_size / 1024) as u32,
        );
        c::Sys_Printf(
            c"Allocating submodel transforms buffer (%u KB)\n".as_ptr(),
            (submodel_transforms_size / 1024) as u32,
        );
        c::Sys_Printf(
            c"Allocating bmodel instances buffer (%u KB)\n".as_ptr(),
            (bmodel_instances_size / 1024) as u32,
        );
        let requests = [
            BufferRequest {
                size: lightstyles_size as u64,
                alignment: 1,
                usage: vk::BufferUsageFlags::UNIFORM_BUFFER,
                mapped: true,
                address: false,
                name: "Lightstyle scales",
            },
            BufferRequest {
                size: lights_size as u64,
                alignment: 1,
                usage: vk::BufferUsageFlags::UNIFORM_BUFFER,
                mapped: true,
                address: false,
                name: "Lights",
            },
            BufferRequest {
                size: submodel_transforms_size as u64,
                alignment: 1,
                usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                mapped: true,
                address: false,
                name: "Submodel transforms",
            },
            BufferRequest {
                size: bmodel_instances_size as u64,
                alignment: 1,
                usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                mapped: true,
                address: false,
                name: "BModel instances",
            },
        ];
        let (_, results) = with_ctx(|ctx| {
            create_buffers(
                ctx,
                &requests,
                &mut *ptr::addr_of_mut!(frame_upload_buffers_memory),
                vk::MemoryPropertyFlags::HOST_VISIBLE,
                vk::MemoryPropertyFlags::HOST_CACHED,
                bmodel_counter(),
                c"Frame upload buffers",
            )
        });
        LIGHTSTYLES_SCALES_BUFFER = results[0].buffer;
        LIGHTSTYLES_SCALES_BUFFER_MAPPED = results[0].mapped.cast::<f32>();
        LIGHTS_BUFFER = results[1].buffer;
        LIGHTS_BUFFER_MAPPED = results[1].mapped.cast::<LmComputeLight>();
        SUBMODEL_TRANSFORMS_BUFFER = results[2].buffer;
        SUBMODEL_TRANSFORMS_BUFFER_MAPPED = results[2].mapped.cast::<f32>();
        BMODEL_INSTANCES_BUFFER = results[3].buffer;
        BMODEL_INSTANCES_BUFFER_MAPPED = results[3].mapped.cast::<BModelInstance>();
    }
}

/// `GL_AllocateSurfaceDataBuffer` -- returns the staging pointer for
/// `num_surfaces` surface records.
unsafe fn allocate_surface_data_buffer() -> *mut LmComputeSurfaceData {
    // SAFETY: map-load path.
    unsafe {
        let size = NUM_SURFACES as usize * LM_SURFACE_DATA_SIZE;
        free_module_buffer(
            ptr::addr_of_mut!(SURFACE_DATA_BUFFER),
            ptr::addr_of_mut!(SURFACE_DATA_BUFFER_MEMORY),
        );
        c::Sys_Printf(
            c"Allocating lightmap compute surface data (%u KB)\n".as_ptr(),
            (size / 1024) as u32,
        );
        SURFACE_DATA_BUFFER = create_device_buffer(
            ptr::addr_of_mut!(SURFACE_DATA_BUFFER_MEMORY),
            size,
            vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
            false,
            "Lightmap compute surface data",
        )
        .0;
        stage_copy_to_buffer(SURFACE_DATA_BUFFER, size).cast::<LmComputeSurfaceData>()
    }
}

/// `GL_AllocateSurfaceSubmodelsBuffer`.
unsafe fn allocate_surface_submodels_buffer() {
    // SAFETY: map-load path.
    unsafe {
        let size = NUM_SURFACES as usize * core::mem::size_of::<u32>();
        free_module_buffer(
            ptr::addr_of_mut!(SURFACE_SUBMODELS_BUFFER),
            ptr::addr_of_mut!(SURFACE_SUBMODELS_BUFFER_MEMORY),
        );
        c::Sys_Printf(
            c"Allocating surface submodel indices (%u KB)\n".as_ptr(),
            (size / 1024) as u32,
        );
        SURFACE_SUBMODELS_BUFFER = create_device_buffer(
            ptr::addr_of_mut!(SURFACE_SUBMODELS_BUFFER_MEMORY),
            size,
            vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
            false,
            "Surface submodel indices",
        )
        .0;
    }
}

/// `GL_AllocateIndirectBuffer` -- returns the staging pointer for
/// `num_draws` indirect commands.
unsafe fn allocate_indirect_buffer(num_draws: c_int) -> *mut vk::DrawIndexedIndirectCommand {
    // SAFETY: map-load path.
    unsafe {
        let size = num_draws as usize * INDIRECT_CMD_SIZE;
        free_module_buffer(
            ptr::addr_of_mut!(INDIRECT_BUFFER),
            ptr::addr_of_mut!(INDIRECT_BUFFER_MEMORY),
        );
        c::Sys_Printf(
            c"Allocating indirect draw data (%u KB, %d draws)\n".as_ptr(),
            (size / 1024) as u32,
            num_draws,
        );
        INDIRECT_BUFFER = create_device_buffer(
            ptr::addr_of_mut!(INDIRECT_BUFFER_MEMORY),
            size,
            vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::TRANSFER_DST
                | vk::BufferUsageFlags::INDIRECT_BUFFER,
            false,
            "Indirect draw data",
        )
        .0;
        stage_copy_to_buffer(INDIRECT_BUFFER, size).cast::<vk::DrawIndexedIndirectCommand>()
    }
}

/// `GL_AllocateWorkgroupBoundsBuffers` -- one buffer per lightmap, all bound
/// to a single allocation.
unsafe fn allocate_workgroup_bounds_buffers() {
    // SAFETY: map-load path; every lightmap gets its buffer before the memory
    // is bound.
    unsafe {
        let memory = ptr::addr_of_mut!(WORKGROUP_BOUNDS_BUFFER_MEMORY);
        if (*memory).handle != vk::DeviceMemory::null() {
            with_ctx(|ctx| free_vulkan_memory(ctx, &mut *memory, bmodel_counter()));
        }
        let device = device();
        let count = lightmap_count as usize;
        for i in 0..count {
            let info = vk::BufferCreateInfo::default()
                .size(WORKGROUP_BOUNDS_BUFFER_SIZE as u64)
                .usage(vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST);
            let buffer = match device.create_buffer(&info, None) {
                Ok(b) => b,
                Err(err) => {
                    c::Sys_Error(c"vkCreateBuffer failed with code %i".as_ptr(), err.as_raw())
                }
            };
            (*lightmap_of(i as c_int)).workgroup_bounds_buffer = buffer;
            with_ctx(|ctx| ctx.name_object(buffer, c"Workgroup bounds buffer"));
        }
        let mut aligned_size: u64 = 0;
        if count > 0 {
            let reqs =
                device.get_buffer_memory_requirements((*lightmap_of(0)).workgroup_bounds_buffer);
            aligned_size = q_align(reqs.size, reqs.alignment);
            with_ctx(|ctx| {
                let memory_type = ctx.memory_type_from_properties(
                    reqs.memory_type_bits,
                    vk::MemoryPropertyFlags::DEVICE_LOCAL,
                    vk::MemoryPropertyFlags::empty(),
                );
                let info = vk::MemoryAllocateInfo::default()
                    .allocation_size(count as u64 * aligned_size)
                    .memory_type_index(memory_type);
                allocate_vulkan_memory(
                    ctx,
                    &mut *memory,
                    &info,
                    VulkanMemoryType::Device,
                    bmodel_counter(),
                );
                ctx.name_object((*memory).handle, c"Workgroup bounds memory");
            });
        }
        for i in 0..count {
            if let Err(err) = device.bind_buffer_memory(
                (*lightmap_of(i as c_int)).workgroup_bounds_buffer,
                (*memory).handle,
                aligned_size * i as u64,
            ) {
                c::Sys_Error(
                    c"vkBindBufferMemory failed with code %i".as_ptr(),
                    err.as_raw(),
                );
            }
        }
    }
}

/// `R_InitIndirectIndexBuffer`.
unsafe fn init_indirect_index_buffer(size: usize) {
    // SAFETY: map-load path.
    unsafe {
        free_module_buffer(
            ptr::addr_of_mut!(INDIRECT_INDEX_BUFFER),
            ptr::addr_of_mut!(INDIRECT_INDEX_BUFFER_MEMORY),
        );
        c::Sys_Printf(
            c"Allocating indirect IBs (%u KB)\n".as_ptr(),
            (size / 1024) as u32,
        );
        INDIRECT_INDEX_BUFFER = create_device_buffer(
            ptr::addr_of_mut!(INDIRECT_INDEX_BUFFER_MEMORY),
            size,
            vk::BufferUsageFlags::INDEX_BUFFER | vk::BufferUsageFlags::STORAGE_BUFFER,
            false,
            "Indirect indices",
        )
        .0;
    }
}

/// `R_InitVisibilityBuffers` -- a double-buffered host-visible visibility
/// bitset, persistently mapped.
unsafe fn init_visibility_buffers(size: usize) {
    // SAFETY: map-load path.
    unsafe {
        free_module_buffer(
            ptr::addr_of_mut!(DYN_VISIBILITY_BUFFER),
            ptr::addr_of_mut!(DYN_VISIBILITY_BUFFER_MEMORY),
        );
        let size = size.div_ceil(256) * 256 * 2;
        c::Sys_Printf(
            c"Allocating visibility buffers (%u KB)\n".as_ptr(),
            (size / 1024) as u32,
        );
        let memory = ptr::addr_of_mut!(DYN_VISIBILITY_BUFFER_MEMORY);
        DYN_VISIBILITY_BUFFER = with_ctx(|ctx| {
            create_buffer(
                ctx,
                &mut *memory,
                size as u64,
                vk::BufferUsageFlags::STORAGE_BUFFER,
                vk::MemoryPropertyFlags::HOST_VISIBLE,
                vk::MemoryPropertyFlags::HOST_CACHED,
                bmodel_counter(),
                false,
                "Dynamic visibility",
            )
        })
        .0;
        let data = match device().map_memory(
            (*memory).handle,
            0,
            size as u64,
            vk::MemoryMapFlags::empty(),
        ) {
            Ok(p) => p,
            Err(err) => c::Sys_Error(c"vkMapMemory failed with code %i".as_ptr(), err.as_raw()),
        };
        DYN_VISIBILITY_VIEW = data.cast::<u8>();
        DYN_VISIBILITY_OFFSET = (size / 2) as u32;
    }
}

/// `R_UploadVisibility`.
unsafe fn upload_visibility(data: *const u8, size: usize) {
    // SAFETY: the mapped view has `2 * DYN_VISIBILITY_OFFSET` bytes.
    unsafe {
        ptr::copy_nonoverlapping(
            data,
            DYN_VISIBILITY_VIEW
                .add(CURRENT_COMPUTE_BUFFER_INDEX as usize * DYN_VISIBILITY_OFFSET as usize),
            size,
        );
        let range = vk::MappedMemoryRange::default()
            .memory((*ptr::addr_of!(DYN_VISIBILITY_BUFFER_MEMORY)).handle)
            .size(vk::WHOLE_SIZE);
        if let Err(err) = device().flush_mapped_memory_ranges(&[range]) {
            c::Sys_Error(
                c"vkFlushMappedMemoryRanges failed with code %i".as_ptr(),
                err.as_raw(),
            );
        }
    }
}

// ---- lightmap build ---------------------------------------------------------

#[derive(Clone, Copy)]
struct SurfSort {
    surf: *mut MSurface,
    sortkey: u64,
}

/// `prepare_3d_interleave`.
fn prepare_3d_interleave(mut x: u32) -> u32 {
    x = (x | (x << 16)) & 0x0300_00FF;
    x = (x | (x << 8)) & 0x0300_F00F;
    x = (x | (x << 4)) & 0x030C_30C3;
    x = (x | (x << 2)) & 0x0924_9249;
    x
}

/// `GL_SortSurfaces` -- radix-sorts the lit surfaces by (style count,
/// submodel, Morton position) and allocates their lightmap blocks in that
/// order.
unsafe fn sort_surfaces() {
    // SAFETY: map-load path on the main thread; all models are loaded.
    unsafe {
        let wm = worldmodel();
        let mut surfs = vec![
            SurfSort {
                surf: ptr::null_mut(),
                sortkey: 0
            };
            NUM_SURFACES as usize * 2
        ];
        let mut used: usize = 0;
        let mut sort_bins = [[0i32; 256]; 6];
        let scale = |axis: usize| -> f32 {
            500.0 / (1.0f32).max((*wm).mins[axis].abs().max((*wm).maxs[axis].abs()))
        };
        let (scale_x, scale_y, scale_z) = (scale(0), scale(1), scale(2));
        let mut current_submodel: usize = 0;
        for j in 1..MAX_MODELS {
            let m = model_precache(j);
            if m.is_null() {
                break;
            }
            if model_is_submodel(m) {
                continue;
            }
            for i in 0..(*m).numsurfaces as usize {
                let surf = (*m).surfaces.add(i);
                let mut submodel: u64 = 0;
                if j == 1 {
                    while current_submodel + 1 < (*m).numsubmodels as usize
                        && i as c_int >= (*(*m).submodels.add(current_submodel + 1)).firstface
                    {
                        current_submodel += 1;
                    }
                    submodel = current_submodel.min(0xFFFF) as u64;
                }
                if (*surf).flags & SURF_DRAWTILED != 0 {
                    continue;
                }
                let lindex = *(*m).surfedges.add((*surf).firstedge as usize);
                let vec: Vec3 = if lindex > 0 {
                    (*(*m)
                        .vertexes
                        .add((*(*m).edges.add(lindex as usize)).v[0] as usize))
                    .position
                } else {
                    (*(*m)
                        .vertexes
                        .add((*(*m).edges.add((-lindex) as usize)).v[1] as usize))
                    .position
                };
                let x = prepare_3d_interleave(((vec[0] * scale_x) as c_int + 512) as u32);
                let y = prepare_3d_interleave(((vec[1] * scale_y) as c_int + 512) as u32);
                let z = prepare_3d_interleave(((vec[2] * scale_z) as c_int + 512) as u32);
                let mut ll = 0u64;
                while ll < 3 {
                    if (*surf).styles[ll as usize] == 0xFF {
                        break;
                    }
                    ll += 1;
                }
                let sortkey =
                    ((3 - ll) << 46) | (submodel << 30) | u64::from(z | (y << 1) | (x << 2));
                surfs[used] = SurfSort { surf, sortkey };
                used += 1;
                for (pass, bins) in sort_bins.iter_mut().enumerate() {
                    bins[((sortkey >> (8 * pass)) % 256) as usize] += 1;
                }
            }
        }
        let n = NUM_SURFACES as usize;
        for (pass, bins) in sort_bins.iter_mut().enumerate() {
            let (from_off, to_off) = if pass % 2 == 1 { (n, 0) } else { (0, n) };
            for k in 1..256 {
                bins[k] += bins[k - 1];
            }
            for i in (0..used).rev() {
                let item = surfs[from_off + i];
                let key = ((item.sortkey >> (8 * pass)) % 256) as usize;
                bins[key] -= 1;
                surfs[to_off + bins[key] as usize] = item;
            }
        }
        for &item in &surfs[..used] {
            let surf = item.surf;
            let w = (c_int::from((*surf).extents[0]) >> 4) + 1;
            let h = (c_int::from((*surf).extents[1]) >> 4) + 1;
            let mut light_s = 0;
            let mut light_t = 0;
            (*surf).lightmaptexturenum = alloc_block(w, h, &mut light_s, &mut light_t);
            (*surf).light_s = light_s;
            (*surf).light_t = light_t;
            let lm = lightmap_of((*surf).lightmaptexturenum);
            let styles = (3 - (item.sortkey >> 46) as usize) + 1;
            for j in 0..styles {
                let used = &mut (*lm).lightstyle_rectused[j];
                used.w = used.w.max((w + light_s) as u16);
                used.h = used.h.max((h + light_t) as u16);
            }
        }
    }
}

// ---- map-load setup -------------------------------------------------------

/// `GL_BuildLightmaps` -- called at level load time: builds the lightmap
/// atlases, the surface display lists and the lightmap-compute surface data.
///
/// # Safety
/// Main thread during map load with the device idle; every model in `cl.model_precache` is loaded.
#[no_mangle]
pub unsafe extern "C" fn GL_BuildLightmaps() {
    // SAFETY: map-load path on the main thread with the device idle; every
    // model pointer comes from `cl.model_precache`.
    unsafe {
        GL_WaitForDeviceIdle();
        *ptr::addr_of_mut!(c::render::r_framecount) = 1; // no dlightcache

        for i in 0..lightmap_count {
            let lm = lightmap_of(i);
            c::Mem_Free((*lm).data.cast());
            with_ctx(|ctx| {
                let layout = ptr::addr_of!((*ctx.vg.as_ptr()).lightmap_compute_set_layout);
                free_descriptor_set(ctx, (*lm).descriptor_set, &*layout);
            });
            if (*lm).workgroup_bounds_buffer != vk::Buffer::null() {
                device().destroy_buffer((*lm).workgroup_bounds_buffer, None);
            }
        }
        c::Mem_Free(lightmaps.cast());
        lightmaps = ptr::null_mut();
        LAST_LIGHTMAP_ALLOCATED = 0;
        lightmap_count = 0;
        NUM_SURFACES = 0;
        COLUMNS = [-1; LM_BINS];
        LIGHTMAP_IDX = [0; LM_BINS];
        SHELF_IDX = [0; LM_BINS];
        USED_INDIRECT_DRAWS = 0;
        indirect_ready = true;
        INDIRECT_BMODEL_START = c_int::MAX as u32;
        USED_DEPS_DATA = 0;
        c::Mem_Free(BRUSH_DEPS_DATA.cast());

        NUM_WORLDMODEL_SUBMODELS = (*model_precache(1)).numsubmodels.min(MAX_MODELS as c_int);
        for i in 1..MAX_MODELS {
            let m = model_precache(i);
            if m.is_null() {
                break;
            }
            if model_is_submodel(m) {
                INDIRECT_BMODEL_START = INDIRECT_BMODEL_START.min((*m).firstmodelsurface as u32);
                continue;
            }
            NUM_SURFACES += (*m).numsurfaces;
        }

        sort_surfaces();
        allocate_surface_submodels_buffer();
        let mut surface_submodels = vec![0u32; NUM_SURFACES as usize];
        let surface_data = allocate_surface_data_buffer();
        STAGING.begin_copy();

        let mut varray_index: u32 = 0;
        let mut current_submodel: c_int = 0;
        let mut surface_index: u32 = 0;
        for j in 1..MAX_MODELS {
            let m = model_precache(j);
            if m.is_null() {
                break;
            }
            if model_is_submodel(m) {
                continue;
            }
            R_PCURRENTVERTBASE = (*m).vertexes;
            CURRENTMODEL = m;
            for i in 0..(*m).numsurfaces {
                let mut submodel: u32 = 0;
                if j == 1 {
                    // the worldmodel surface array also contains all movable submodel surfaces
                    while current_submodel + 1 < (*m).numsubmodels
                        && i >= (*(*m).submodels.add(current_submodel as usize + 1)).firstface
                    {
                        current_submodel += 1;
                    }
                    if current_submodel < NUM_WORLDMODEL_SUBMODELS {
                        submodel = current_submodel as u32;
                    }
                }
                surface_submodels[surface_index as usize] = submodel;
                let surf = (*m).surfaces.add(i as usize);
                if (*surf).flags & SURF_DRAWTILED == 0 {
                    let no_dlights = j > 1;
                    create_surface_lightmap(
                        surf,
                        surface_index | (0x8000_0000 * u32::from(no_dlights)),
                    );
                    build_surface_display_list(surf);
                    if !no_dlights {
                        assign_workgroup_bounds(surf, submodel);
                    }
                }
                if indirect_ready {
                    update_indirect_structs(
                        surf,
                        INDIRECT_ZBIAS && surface_index >= INDIRECT_BMODEL_START,
                    );
                }
                let sd = surface_data.add(surface_index as usize);
                let styles = (*surf).styles;
                (*sd).packed_lightstyles = u32::from(styles[0])
                    | u32::from(styles[1]) << 8
                    | u32::from(styles[2]) << 16
                    | u32::from(styles[3]) << 24;
                let plane = (*surf).plane;
                (*sd).normal = (*plane).normal;
                (*sd).dist = (*plane).dist;
                (*sd).packed_light_st =
                    ((*surf).light_s as u32 & 0xFFFF) | (((*surf).light_t as u32 & 0xFFFF) << 16);
                (*sd).packed_tex_edgecount = (*surf).indirect_idx as u32
                    | u32::from((*surf).flags & SURF_PLANEBACK != 0) << 15
                    | ((*surf).numedges as u32) << 16;
                (*surf).vbo_firstvert = varray_index as c_int;
                (*sd).vbo_offset = (*surf).vbo_firstvert as u32;
                if (*surf).numedges > 65535 {
                    indirect_ready = false;
                }
                varray_index += (*surf).numedges as u32;
                (*sd).vecs = (*(*surf).texinfo).vecs;
                (*sd).vecs[0][3] -= f32::from((*surf).texturemins[0]);
                (*sd).vecs[1][3] -= f32::from((*surf).texturemins[1]);
                surface_index += 1;
            }
        }
        STAGING.end_copy();
        let bytes = core::slice::from_raw_parts(
            surface_submodels.as_ptr().cast::<u8>(),
            surface_submodels.len() * core::mem::size_of::<u32>(),
        );
        with_ctx(|ctx| STAGING.upload_buffer(ctx, SURFACE_SUBMODELS_BUFFER, bytes));
    }
}

/// `GL_SetupIndirectDraws`: the indirect-draw buffer, its index buffer, the
/// visibility ring and the indirect-compute descriptor set.
///
/// # Safety
/// Main thread during map load, after `GL_BuildLightmaps`.
#[no_mangle]
pub unsafe extern "C" fn GL_SetupIndirectDraws() {
    // SAFETY: map-load path, after `GL_BuildLightmaps`.
    unsafe {
        if !indirect_ready {
            c::Con_Warning(c"map exceeds indirect dispatch limits\n".as_ptr());
            return;
        }
        prepare_indirect_draws();
        let used = USED_INDIRECT_DRAWS as usize;
        let hw_indirect_buffer = allocate_indirect_buffer(USED_INDIRECT_DRAWS);
        STAGING.begin_copy();
        ptr::copy_nonoverlapping(
            ptr::addr_of!(INITIAL_INDIRECT_BUFFER).cast::<vk::DrawIndexedIndirectCommand>(),
            hw_indirect_buffer,
            used,
        );
        STAGING.end_copy();

        let last = INITIAL_INDIRECT_BUFFER[used - 1].first_index
            + INDIRECT_DRAWS[used - 1].max_indices as u32;
        init_indirect_index_buffer(last as usize * core::mem::size_of::<u32>());
        init_visibility_buffers(((*worldmodel()).numsurfaces as usize + 31) / 8);

        with_ctx(|ctx| {
            let vgp = ctx.vg.as_ptr();
            let layout = ptr::addr_of!((*vgp).indirect_compute_set_layout);
            if (*vgp).indirect_compute_desc_set != vk::DescriptorSet::null() {
                free_descriptor_set(ctx, (*vgp).indirect_compute_desc_set, &*layout);
            }
            (*vgp).indirect_compute_desc_set = allocate_descriptor_set(ctx, &*layout);
        });
        let set = with_ctx(|ctx| (*ctx.vg.as_ptr()).indirect_compute_desc_set);

        let infos = [
            buffer_info(INDIRECT_BUFFER, vk::WHOLE_SIZE),
            buffer_info(
                SURFACE_DATA_BUFFER,
                (NUM_SURFACES as usize * LM_SURFACE_DATA_SIZE) as u64,
            ),
            buffer_info(DYN_VISIBILITY_BUFFER, vk::WHOLE_SIZE),
            buffer_info(INDIRECT_INDEX_BUFFER, vk::WHOLE_SIZE),
            buffer_info(
                SURFACE_SUBMODELS_BUFFER,
                (NUM_SURFACES as usize * core::mem::size_of::<u32>()) as u64,
            ),
            buffer_info(BMODEL_INSTANCES_BUFFER, vk::WHOLE_SIZE),
        ];
        let writes: [vk::WriteDescriptorSet<'_>; 6] = core::array::from_fn(|i| {
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(i as u32)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(core::slice::from_ref(&infos[i]))
        });
        device().update_descriptor_sets(&writes, &[]);

        let world = worldmodel();
        for i in 0..(*world).numleafs {
            calc_deps(ptr::null_mut(), (*world).leafs.add(i as usize + 1));
        }
        for j in 2..MAX_MODELS {
            let m = model_precache(j);
            if m.is_null() {
                break;
            }
            if model_is_submodel(m) {
                calc_deps(m, ptr::null_mut());
            }
        }
    }
}

#[inline]
fn buffer_info(buffer: vk::Buffer, range: vk::DeviceSize) -> vk::DescriptorBufferInfo {
    vk::DescriptorBufferInfo {
        buffer,
        offset: 0,
        range,
    }
}

/// `GL_UpdateLightmapDescriptorSets`: (re)writes every lightmap's compute
/// descriptor set. A no-op until the lightmap textures exist.
///
/// # Safety
/// Main thread with the device idle; `lightmaps` and their textures are live.
#[no_mangle]
pub unsafe extern "C" fn GL_UpdateLightmapDescriptorSets() {
    // SAFETY: main thread with the device idle; `lightmaps` and their
    // textures are live.
    unsafe {
        GL_WaitForDeviceIdle();
        if lightmap_count > 0 && (*(*lightmaps).texture).target_image_view == vk::ImageView::null()
        {
            return;
        }
        for i in 0..lightmap_count {
            let lm = lightmap_of(i);
            let set = with_ctx(|ctx| {
                let layout = ptr::addr_of!((*ctx.vg.as_ptr()).lightmap_compute_set_layout);
                if (*lm).descriptor_set != vk::DescriptorSet::null() {
                    free_descriptor_set(ctx, (*lm).descriptor_set, &*layout);
                }
                let set = allocate_descriptor_set(ctx, &*layout);
                let name =
                    CString::new(format!("lightmap{i:07} compute desc set")).unwrap_or_default();
                ctx.name_object(set, &name);
                set
            });
            (*lm).descriptor_set = set;

            let output_image_info = [vk::DescriptorImageInfo {
                sampler: vk::Sampler::null(),
                image_view: (*(*lm).texture).target_image_view,
                image_layout: vk::ImageLayout::GENERAL,
            }];
            let surface_indices_image_info = [vk::DescriptorImageInfo {
                sampler: vk::Sampler::null(),
                image_view: (*(*lm).surface_indices_texture).image_view,
                image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            }];
            let lightmap_images_infos: [vk::DescriptorImageInfo; 3] =
                core::array::from_fn(|j| vk::DescriptorImageInfo {
                    sampler: vk::Sampler::null(),
                    image_view: (*(*lm).lightstyle_textures[j]).image_view,
                    image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                });
            let surfaces_data = [buffer_info(
                SURFACE_DATA_BUFFER,
                (NUM_SURFACES as usize * LM_SURFACE_DATA_SIZE) as u64,
            )];
            let workgroup_bounds = [buffer_info(
                (*lm).workgroup_bounds_buffer,
                WORKGROUP_BOUNDS_BUFFER_SIZE as u64,
            )];
            let lightstyle_scales = [buffer_info(
                LIGHTSTYLES_SCALES_BUFFER,
                (MAX_LIGHTSTYLES * core::mem::size_of::<f32>()) as u64,
            )];
            let lights = [buffer_info(
                LIGHTS_BUFFER,
                (MAX_DLIGHTS * 2 * LM_COMPUTE_LIGHT_SIZE) as u64,
            )];
            let world_vertex = [buffer_info(bmodel_vertex_buffer, vk::WHOLE_SIZE)];
            let surface_submodels = [buffer_info(
                SURFACE_SUBMODELS_BUFFER,
                (NUM_SURFACES as usize * core::mem::size_of::<u32>()) as u64,
            )];
            let submodel_transforms = [buffer_info(SUBMODEL_TRANSFORMS_BUFFER, vk::WHOLE_SIZE)];

            let write = |binding: u32, ty: vk::DescriptorType| {
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(binding)
                    .dst_array_element(0)
                    .descriptor_type(ty)
            };
            let writes = [
                write(0, vk::DescriptorType::STORAGE_IMAGE).image_info(&output_image_info),
                write(1, vk::DescriptorType::SAMPLED_IMAGE).image_info(&surface_indices_image_info),
                write(2, vk::DescriptorType::SAMPLED_IMAGE).image_info(&lightmap_images_infos),
                write(3, vk::DescriptorType::STORAGE_BUFFER).buffer_info(&surfaces_data),
                write(4, vk::DescriptorType::STORAGE_BUFFER).buffer_info(&workgroup_bounds),
                write(5, vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                    .buffer_info(&lightstyle_scales),
                write(6, vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC).buffer_info(&lights),
                write(7, vk::DescriptorType::STORAGE_BUFFER).buffer_info(&world_vertex),
                write(8, vk::DescriptorType::STORAGE_BUFFER).buffer_info(&surface_submodels),
                write(9, vk::DescriptorType::STORAGE_BUFFER).buffer_info(&submodel_transforms),
            ];
            device().update_descriptor_sets(&writes, &[]);
        }
    }
}

/// `GL_SetupLightmapCompute`: uploads the lightmap atlases, the per-style
/// textures, the surface-index textures and the workgroup bounds.
///
/// # Safety
/// Main thread during map load, after `GL_BuildLightmaps`.
#[no_mangle]
pub unsafe extern "C" fn GL_SetupLightmapCompute() {
    // SAFETY: map-load path after `GL_BuildLightmaps`; the CPU-side atlas
    // buffers are live until freed below.
    unsafe {
        allocate_workgroup_bounds_buffers();
        let world = worldmodel().cast::<c_void>();
        let empty = c"".as_ptr();
        for i in 0..lightmap_count {
            let lm = lightmap_of(i);
            (*lm).modified = [0; TASKS_MAX_WORKERS];
            (*lm).rectchange = GlRect {
                l: LMBLOCK_WIDTH as u16,
                t: LMBLOCK_HEIGHT as u16,
                w: 0,
                h: 0,
            };
            let name = CString::new(format!("lightmap_{i:07}")).unwrap_or_default();
            (*lm).texture = TexMgr_LoadImage(
                world,
                name.as_ptr(),
                LMBLOCK_WIDTH as c_int,
                LMBLOCK_HEIGHT as c_int,
                SRC_LIGHTMAP,
                (*lm).data,
                empty,
                (*lm).data as usize,
                TEXPREF_LINEAR | TEXPREF_NOPICMIP,
            );
            for j in 0..3usize {
                let name = CString::new(format!("lightstyle{j}_{i:07}")).unwrap_or_default();
                let mut size_w = c_int::from((*lm).lightstyle_rectused[j + 1].w);
                let size_h = c_int::from((*lm).lightstyle_rectused[j + 1].h);
                if LMBLOCK_WIDTH as c_int - size_w < 16 {
                    size_w = LMBLOCK_WIDTH as c_int;
                }
                if size_h == 0 {
                    (*lm).lightstyle_textures[j] =
                        (*ptr::addr_of!(c::render::nulltexture)).cast::<GlTexture>();
                } else {
                    let data = (*lm).lightstyle_data[j];
                    if size_w < LMBLOCK_WIDTH as c_int {
                        for row in 1..size_h as usize {
                            ptr::copy(
                                data.add(LMBLOCK_WIDTH * row * 4),
                                data.add(size_w as usize * row * 4),
                                size_w as usize * 4,
                            );
                        }
                    }
                    (*lm).lightstyle_textures[j] = TexMgr_LoadImage(
                        world,
                        name.as_ptr(),
                        size_w,
                        size_h,
                        SRC_RGBA,
                        data,
                        empty,
                        (*lm).data as usize,
                        TEXPREF_NEAREST | TEXPREF_NOPICMIP,
                    );
                }
                c::Mem_Free((*lm).lightstyle_data[j].cast());
                (*lm).lightstyle_data[j] = ptr::null_mut();
            }

            let w = (c_int::from((*lm).lightstyle_rectused[0].w) + 7) / 8 * 8;
            let h = (c_int::from((*lm).lightstyle_rectused[0].h) + 7) / 8 * 8;
            (*lm).lightstyle_rectused[0].w = w as u16;
            (*lm).lightstyle_rectused[0].h = h as u16;
            let indices = (*lm).surface_indices;
            if w < LMBLOCK_WIDTH as c_int {
                for row in 1..h as usize {
                    ptr::copy(
                        indices.add(LMBLOCK_WIDTH * row),
                        indices.add(w as usize * row),
                        w as usize,
                    );
                }
            }
            let name = CString::new(format!("surfindices_{i:07}")).unwrap_or_default();
            (*lm).surface_indices_texture = TexMgr_LoadImage(
                world,
                name.as_ptr(),
                w,
                h,
                SRC_SURF_INDICES,
                indices.cast::<u8>(),
                empty,
                indices as usize,
                TEXPREF_NEAREST | TEXPREF_NOPICMIP,
            );
            c::Mem_Free(indices.cast());
            (*lm).surface_indices = ptr::null_mut();

            for y in 0..LM_CULL_ROWS {
                for x in 0..LM_CULL_COLS {
                    for l in 0..MAX_LIGHTSTYLES {
                        if (*lm).used_lightstyles[y][x][l] != 0 {
                            let n = (*lm).num_used_lightstyles[y][x] as usize;
                            (*lm).used_lightstyles[y][x][n] = l as u8;
                            (*lm).num_used_lightstyles[y][x] += 1;
                        }
                    }
                }
            }
        }

        for i in 0..lightmap_count {
            let lm = lightmap_of(i);
            let a = with_ctx(|ctx| STAGING.allocate(ctx, WORKGROUP_BOUNDS_BUFFER_SIZE as i32, 1));
            let region = vk::BufferCopy {
                src_offset: a.buffer_offset as vk::DeviceSize,
                dst_offset: 0,
                size: WORKGROUP_BOUNDS_BUFFER_SIZE as vk::DeviceSize,
            };
            device().cmd_copy_buffer(
                a.command_buffer,
                a.buffer,
                (*lm).workgroup_bounds_buffer,
                &[region],
            );
            STAGING.begin_copy();
            ptr::copy_nonoverlapping(
                (*lm).workgroup_bounds.cast::<u8>(),
                a.data,
                WORKGROUP_BOUNDS_BUFFER_SIZE,
            );
            let staged = a.data.cast::<LmComputeWorkgroupBounds>();
            for j in 0..(LMBLOCK_WIDTH / 128) * (LMBLOCK_HEIGHT / 128) {
                let b = staged.add(j);
                if (*b).submodel == LM_WORKGROUP_SUBMODEL_EMPTY {
                    (*b).submodel = 0;
                }
            }
            STAGING.end_copy();
            c::Mem_Free((*lm).workgroup_bounds.cast());
            (*lm).workgroup_bounds = ptr::null_mut();
        }

        let i = lightmap_count * ((LMBLOCK_WIDTH / 128) * (LMBLOCK_HEIGHT / 128)) as c_int;
        if i > 64 {
            c::Con_DWarning(c"%i lightmaps exceeds standard limit of 64.\n".as_ptr(), i);
        }
    }
}

/// `GL_DeleteBModelVertexBuffer`.
///
/// # Safety
/// Main thread with the device idle.
#[no_mangle]
pub unsafe extern "C" fn GL_DeleteBModelVertexBuffer() {
    // SAFETY: main thread with the device idle.
    unsafe {
        GL_WaitForDeviceIdle();
        free_module_buffer(
            ptr::addr_of_mut!(bmodel_vertex_buffer),
            ptr::addr_of_mut!(BMODEL_MEMORY),
        );
        free_module_buffer(
            ptr::addr_of_mut!(VERTEX_SUBMODELS_BUFFER),
            ptr::addr_of_mut!(VERTEX_SUBMODELS_BUFFER_MEMORY),
        );
    }
}

/// `GL_BuildBModelVertexBuffer`: one big VBO of every brush model's
/// `glpoly_t` vertices, plus the per-vertex submodel index buffer.
///
/// # Safety
/// Main thread during map load, after `GL_BuildLightmaps` (display lists built).
#[no_mangle]
pub unsafe extern "C" fn GL_BuildBModelVertexBuffer() {
    // SAFETY: map-load path after `GL_BuildLightmaps` (display lists built).
    unsafe {
        bmodel_numverts = 0;
        for j in 1..MAX_MODELS {
            let m = model_precache(j);
            if m.is_null() || model_is_submodel(m) || (*m).type_ != MOD_BRUSH {
                continue;
            }
            for i in 0..(*m).numsurfaces as usize {
                bmodel_numverts += (*(*m).surfaces.add(i)).numedges as u32;
            }
        }
        let numverts = bmodel_numverts as usize;
        let varray_bytes = VERTEXSIZE * core::mem::size_of::<f32>() * numverts;
        let mut varray = vec![0f32; VERTEXSIZE * numverts];
        let mut vertex_submodels = vec![0u32; numverts];
        let mut current_submodel: c_int = 0;
        for j in 1..MAX_MODELS {
            let m = model_precache(j);
            if m.is_null() || model_is_submodel(m) || (*m).type_ != MOD_BRUSH {
                continue;
            }
            for i in 0..(*m).numsurfaces {
                let s = (*m).surfaces.add(i as usize);
                let first = (*s).vbo_firstvert as usize;
                let numedges = (*s).numedges as usize;
                let poly = (*s).polys.cast::<GlPoly>();
                ptr::copy_nonoverlapping(
                    (*poly).verts.as_ptr().cast::<f32>(),
                    varray.as_mut_ptr().add(VERTEXSIZE * first),
                    VERTEXSIZE * numedges,
                );
                let mut submodel: u32 = 0;
                if j == 1 {
                    // the worldmodel surface array also contains all movable submodel surfaces
                    while current_submodel + 1 < (*m).numsubmodels
                        && i >= (*(*m).submodels.add(current_submodel as usize + 1)).firstface
                    {
                        current_submodel += 1;
                    }
                    if current_submodel < NUM_WORLDMODEL_SUBMODELS {
                        submodel = current_submodel as u32;
                    }
                }
                for v in 0..numedges {
                    vertex_submodels[first + v] = submodel;
                }
            }
        }

        let ray_query = with_ctx(|ctx| (*ctx.vg.as_ptr()).ray_query);
        let mut usage = vk::BufferUsageFlags::VERTEX_BUFFER
            | vk::BufferUsageFlags::TRANSFER_DST
            | vk::BufferUsageFlags::STORAGE_BUFFER;
        if ray_query {
            usage |= vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR;
        }
        let (buffer, address) = create_device_buffer(
            ptr::addr_of_mut!(BMODEL_MEMORY),
            varray_bytes,
            usage,
            true,
            "BModel vertices",
        );
        bmodel_vertex_buffer = buffer;
        bmodel_vertex_buffer_device_address = address.unwrap_or(0);
        let varray_bytes_slice =
            core::slice::from_raw_parts(varray.as_ptr().cast::<u8>(), varray_bytes);
        with_ctx(|ctx| STAGING.upload_buffer(ctx, bmodel_vertex_buffer, varray_bytes_slice));

        VERTEX_SUBMODELS_BUFFER = create_device_buffer(
            ptr::addr_of_mut!(VERTEX_SUBMODELS_BUFFER_MEMORY),
            numverts * core::mem::size_of::<u32>(),
            vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
            false,
            "BModel vertex submodels",
        )
        .0;
        let submodel_bytes = core::slice::from_raw_parts(
            vertex_submodels.as_ptr().cast::<u8>(),
            numverts * core::mem::size_of::<u32>(),
        );
        with_ctx(|ctx| STAGING.upload_buffer(ctx, VERTEX_SUBMODELS_BUFFER, submodel_bytes));
        drop(vertex_submodels);
        drop(varray);

        let set = with_ctx(|ctx| {
            let vgp = ctx.vg.as_ptr();
            let layout = ptr::addr_of!((*vgp).bmodel_instances_set_layout);
            if (*vgp).bmodel_instances_desc_set != vk::DescriptorSet::null() {
                free_descriptor_set(ctx, (*vgp).bmodel_instances_desc_set, &*layout);
            }
            (*vgp).bmodel_instances_desc_set = allocate_descriptor_set(ctx, &*layout);
            (*vgp).bmodel_instances_desc_set
        });
        let vertex_submodels_info = [buffer_info(VERTEX_SUBMODELS_BUFFER, vk::WHOLE_SIZE)];
        let bmodel_instances_info = [buffer_info(BMODEL_INSTANCES_BUFFER, vk::WHOLE_SIZE)];
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(0)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&vertex_submodels_info),
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(1)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&bmodel_instances_info),
        ];
        device().update_descriptor_sets(&writes, &[]);
    }
}

// ---- SoA culling data -----------------------------------------------------

/// `SoA_FillBoxLane`.
unsafe fn soa_fill_box_lane(boxes: *mut SoaAabb, index: usize, mins: &[f32], maxs: &[f32]) {
    // SAFETY: `boxes` has `(count + 7) / 8` lanes and `index < count`.
    unsafe {
        let dst = &mut *boxes.add(index >> 3);
        let i = index & 7;
        dst[i] = mins[0];
        dst[i + 8] = maxs[0];
        dst[i + 16] = mins[1];
        dst[i + 24] = maxs[1];
        dst[i + 32] = mins[2];
        dst[i + 40] = maxs[2];
    }
}

/// `SoA_FillPlaneLane`.
unsafe fn soa_fill_plane_lane(planes: *mut SoaPlane, index: usize, src: *const MPlane, flip: bool) {
    // SAFETY: as above; `src` is the surface's plane.
    unsafe {
        let side: f32 = if flip { -1.0 } else { 1.0 };
        let dst = &mut *planes.add(index >> 3);
        let i = index & 7;
        dst[i] = side * (*src).normal[0];
        dst[i + 8] = side * (*src).normal[1];
        dst[i + 16] = side * (*src).normal[2];
        dst[i + 24] = side * (*src).dist;
    }
}

/// `GL_PrepareSIMDAndParallelData`: the world's surface-visibility bitmap
/// and, on SIMD targets, the SoA leaf bounds / surface planes.
///
/// # Safety
/// Main thread during map load; the world model is loaded.
#[no_mangle]
pub unsafe extern "C" fn GL_PrepareSIMDAndParallelData() {
    // SAFETY: map-load path; the world model is loaded.
    unsafe {
        let world = worldmodel();
        let numsurfaces = (*world).numsurfaces as usize;
        (*world).surfvis = c::Mem_Alloc((numsurfaces + 31) / 8).cast::<u8>();
        #[cfg(any(target_arch = "x86_64", target_arch = "x86", target_arch = "aarch64"))]
        {
            let numleafs = (*world).numleafs as usize;
            (*world).soa_leafbounds =
                c::Mem_Alloc(6 * core::mem::size_of::<f32>() * ((numleafs + 31) & !7))
                    .cast::<SoaAabb>();
            (*world).soa_surfplanes =
                c::Mem_Alloc(4 * core::mem::size_of::<f32>() * ((numsurfaces + 31) & !7))
                    .cast::<SoaPlane>();
            for i in 0..numleafs {
                let leaf = (*world).leafs.add(i + 1);
                let minmaxs = (*leaf).minmaxs;
                soa_fill_box_lane((*world).soa_leafbounds, i, &minmaxs[0..3], &minmaxs[3..6]);
            }
            for i in 0..numsurfaces {
                let surf = (*world).surfaces.add(i);
                soa_fill_plane_lane(
                    (*world).soa_surfplanes,
                    i,
                    (*surf).plane,
                    (*surf).flags & SURF_PLANEBACK != 0,
                );
            }
        }
    }
}

// ---- lightmap building (CPU path) -----------------------------------------

/// `R_AddDynamicLights`: accumulates the surface's marked dlights into
/// `BLOCKLIGHTS`.
unsafe fn add_dynamic_lights(surf: *mut MSurface) {
    // SAFETY: `surf` is live; `cl_dlights`/`lightmap_dlight_origins` are
    // stable for the frame; `BLOCKLIGHTS` covers `smax * tmax * 3`.
    unsafe {
        let smax = (c_int::from((*surf).extents[0]) >> 4) + 1;
        let tmax = (c_int::from((*surf).extents[1]) >> 4) + 1;
        let tex = (*surf).texinfo;
        let plane = (*surf).plane;
        let dlights = ptr::addr_of!(c::cl_main::cl_dlights).cast::<c::cl_tent::dlight_t>();
        let origins = ptr::addr_of!(lightmap_dlight_origins);
        for lnum in 0..MAX_DLIGHTS {
            if (*surf).dlightbits[lnum >> 5] & (1 << (lnum & 31)) == 0 {
                continue; // not lit by this light
            }
            let dl = dlights.add(lnum);
            let origin = (*origins)[lnum];
            let mut rad = (*dl).radius;
            let dist = dot_product(&origin, &(*plane).normal) - (*plane).dist;
            rad -= dist.abs();
            let mut minlight = (*dl).minlight;
            if rad < minlight {
                continue;
            }
            minlight = rad - minlight;

            let mut impact = [0f32; 3];
            for i in 0..3 {
                impact[i] = origin[i] - (*plane).normal[i] * dist;
            }
            let vecs = (*tex).vecs;
            let mut local = [
                dot_product(&impact, &[vecs[0][0], vecs[0][1], vecs[0][2]]) + vecs[0][3],
                dot_product(&impact, &[vecs[1][0], vecs[1][1], vecs[1][2]]) + vecs[1][3],
            ];
            local[0] -= f32::from((*surf).texturemins[0]);
            local[1] -= f32::from((*surf).texturemins[1]);

            let mut bl = ptr::addr_of_mut!(BLOCKLIGHTS).cast::<u32>();
            let cred = (*dl).color[0] * 256.0;
            let cgreen = (*dl).color[1] * 256.0;
            let cblue = (*dl).color[2] * 256.0;
            for t in 0..tmax {
                let mut td = (local[1] - (t * 16) as f32) as c_int;
                if td < 0 {
                    td = -td;
                }
                for s in 0..smax {
                    let mut sd = (local[0] - (s * 16) as f32) as c_int;
                    if sd < 0 {
                        sd = -sd;
                    }
                    let dist = (if sd > td {
                        sd + (td >> 1)
                    } else {
                        td + (sd >> 1)
                    }) as f32;
                    if dist < minlight {
                        let brightness = rad - dist;
                        *bl = (*bl).wrapping_add((brightness * cred) as c_int as u32);
                        *bl.add(1) =
                            (*bl.add(1)).wrapping_add((brightness * cgreen) as c_int as u32);
                        *bl.add(2) =
                            (*bl.add(2)).wrapping_add((brightness * cblue) as c_int as u32);
                    }
                    bl = bl.add(3);
                }
            }
        }
    }
}

/// SSE2 lanes of `R_AccumulateLightmap` / `R_StoreLightmap` (`USE_SSE2`).
#[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
mod lm_lanes {
    #[cfg(target_arch = "x86")]
    use core::arch::x86 as arch;
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64 as arch;

    use arch::{
        _mm_add_epi32, _mm_cvtsi128_si32, _mm_loadl_epi64, _mm_loadu_si128, _mm_mulhi_epu16,
        _mm_mullo_epi16, _mm_packs_epi32, _mm_packus_epi16, _mm_set1_epi16, _mm_setzero_si128,
        _mm_srli_epi32, _mm_storeu_si128, _mm_unpackhi_epi16, _mm_unpacklo_epi16,
        _mm_unpacklo_epi8,
    };

    pub(super) const AVAILABLE: bool = true;

    /// Accumulates whole groups of eight samples; returns the remaining
    /// `(lightmap, bl, size)` for the scalar tail.
    pub(super) unsafe fn accumulate(
        mut lightmap: *const u8,
        mut bl: *mut u32,
        scale: u32,
        mut size: i32,
    ) -> (*const u8, *mut u32, i32) {
        // SAFETY: the caller guarantees `size` samples at `lightmap` and
        // `size + 1` words at `bl` (the `BLOCKLIGHTS` padding).
        unsafe {
            // COMPAT (ADR-010): the C truncates `scale` to a 16-bit lane.
            let vscale = _mm_set1_epi16(scale as u16 as i16);
            let vzero = _mm_setzero_si128();
            while size >= 8 {
                let vsrc = _mm_loadl_epi64(lightmap.cast());
                let v = _mm_unpacklo_epi8(vsrc, vzero);
                let vlo = _mm_mullo_epi16(v, vscale);
                let vhi = _mm_mulhi_epu16(v, vscale);
                let mut vdst = _mm_loadu_si128(bl.cast());
                vdst = _mm_add_epi32(vdst, _mm_unpacklo_epi16(vlo, vhi));
                _mm_storeu_si128(bl.cast(), vdst);
                bl = bl.add(4);
                vdst = _mm_loadu_si128(bl.cast());
                vdst = _mm_add_epi32(vdst, _mm_unpackhi_epi16(vlo, vhi));
                _mm_storeu_si128(bl.cast(), vdst);
                bl = bl.add(4);
                lightmap = lightmap.add(8);
                size -= 8;
            }
            (lightmap, bl, size)
        }
    }

    /// Packs `width * height` RGB triples into RGBA8 rows of `stride` bytes.
    pub(super) unsafe fn store(
        mut src: *const u32,
        mut dest: *mut u8,
        width: i32,
        mut height: i32,
        stride: i32,
    ) {
        // SAFETY: `src` has `width * height * 3 + 1` words (the
        // `BLOCKLIGHTS` padding covers the 4-wide load of the last triple);
        // `dest` rows are `stride` bytes apart with `width * 4` writable.
        unsafe {
            let vzero = _mm_setzero_si128();
            while height > 0 {
                height -= 1;
                for i in 0..width as usize {
                    let mut v = _mm_srli_epi32(_mm_loadu_si128(src.cast()), 8);
                    v = _mm_packs_epi32(v, vzero);
                    v = _mm_packus_epi16(v, vzero);
                    dest.cast::<u32>()
                        .add(i)
                        .write_unaligned(_mm_cvtsi128_si32(v) as u32 | 0xff00_0000);
                    src = src.add(3);
                }
                dest = dest.add(stride as usize);
            }
        }
    }
}

/// NEON lanes of `R_AccumulateLightmap` / `R_StoreLightmap` (`USE_NEON`).
#[cfg(target_arch = "aarch64")]
mod lm_lanes {
    use core::arch::aarch64::{
        vcombine_u16, vcreate_u16, vget_high_u16, vget_lane_u32, vget_low_u16, vld1_u8, vld1q_u32,
        vmlal_n_u16, vmovl_u8, vqmovn_u16, vreinterpret_u32_u8, vset_lane_u16, vshrn_n_u32,
        vst1q_u32,
    };

    pub(super) const AVAILABLE: bool = true;

    pub(super) unsafe fn accumulate(
        mut lightmap: *const u8,
        mut bl: *mut u32,
        scale: u32,
        mut size: i32,
    ) -> (*const u8, *mut u32, i32) {
        // SAFETY: as the SSE2 lanes.
        unsafe {
            while size >= 8 {
                let lm8 = vld1_u8(lightmap);
                let lm16 = vmovl_u8(lm8);
                let old = vld1q_u32(bl);
                // COMPAT (ADR-010): the C truncates `scale` to a 16-bit lane.
                let acc = vmlal_n_u16(old, vget_low_u16(lm16), scale as u16);
                vst1q_u32(bl, acc);
                bl = bl.add(4);
                let old = vld1q_u32(bl);
                let acc = vmlal_n_u16(old, vget_high_u16(lm16), scale as u16);
                vst1q_u32(bl, acc);
                bl = bl.add(4);
                lightmap = lightmap.add(8);
                size -= 8;
            }
            (lightmap, bl, size)
        }
    }

    pub(super) unsafe fn store(
        mut src: *const u32,
        mut dest: *mut u8,
        width: i32,
        mut height: i32,
        stride: i32,
    ) {
        // SAFETY: as the SSE2 lanes.
        unsafe {
            while height > 0 {
                height -= 1;
                for i in 0..width as usize {
                    let lm = vld1q_u32(src);
                    // COMPAT (ADR-010): `vshrn_n_u32` is a truncating narrow
                    // (the C NEON path), unlike the SSE2/scalar saturation.
                    let s16 = vshrn_n_u32::<8>(lm);
                    let masked = vset_lane_u16::<3>(0xFF, s16);
                    let s16x8 = vcombine_u16(masked, vcreate_u16(0));
                    let sat = vqmovn_u16(s16x8);
                    dest.cast::<u32>()
                        .add(i)
                        .write_unaligned(vget_lane_u32::<0>(vreinterpret_u32_u8(sat)));
                    src = src.add(3);
                }
                dest = dest.add(stride as usize);
            }
        }
    }
}

/// No SIMD lanes on other targets (the C `USE_SIMD` is undefined there).
#[cfg(not(any(target_arch = "x86_64", target_arch = "x86", target_arch = "aarch64")))]
mod lm_lanes {
    pub(super) const AVAILABLE: bool = false;

    pub(super) unsafe fn accumulate(
        lightmap: *const u8,
        bl: *mut u32,
        _scale: u32,
        size: i32,
    ) -> (*const u8, *mut u32, i32) {
        (lightmap, bl, size)
    }

    pub(super) unsafe fn store(
        _src: *const u32,
        _dest: *mut u8,
        _width: i32,
        _height: i32,
        _stride: i32,
    ) {
        unreachable!("no SIMD lanes on this target")
    }
}

/// `R_AccumulateLightmap`: `BLOCKLIGHTS[i] += lightmap[i] * scale` over
/// `texels * 3` samples.
unsafe fn accumulate_lightmap(mut lightmap: *const u8, scale: u32, texels: c_int) {
    // SAFETY: `lightmap` has `texels * 3` samples; `BLOCKLIGHTS` is sized for
    // the largest surface plus the SIMD padding word.
    unsafe {
        let mut bl = ptr::addr_of_mut!(BLOCKLIGHTS).cast::<u32>();
        let mut size = texels * 3;
        if lm_lanes::AVAILABLE && use_simd() && size >= 8 {
            (lightmap, bl, size) = lm_lanes::accumulate(lightmap, bl, scale, size);
        }
        while size > 0 {
            *bl = (*bl).wrapping_add(u32::from(*lightmap) * scale);
            bl = bl.add(1);
            lightmap = lightmap.add(1);
            size -= 1;
        }
    }
}

/// `R_StoreLightmap`: packs `BLOCKLIGHTS` (8.8 fixed point) into the RGBA8
/// atlas rows at `dest`.
unsafe fn store_lightmap(mut dest: *mut u8, width: c_int, mut height: c_int, mut stride: c_int) {
    // SAFETY: `dest` rows are `stride` bytes apart inside the lightmap atlas.
    unsafe {
        let mut src = ptr::addr_of!(BLOCKLIGHTS).cast::<u32>();
        if lm_lanes::AVAILABLE && use_simd() {
            lm_lanes::store(src, dest, width, height, stride);
            return;
        }
        stride -= width * 4;
        while height > 0 {
            height -= 1;
            for _ in 0..width {
                for _ in 0..3 {
                    let c = *src >> 8;
                    src = src.add(1);
                    *dest = c.min(255) as u8;
                    dest = dest.add(1);
                }
                *dest = 255;
                dest = dest.add(1);
            }
            dest = dest.add(stride as usize);
        }
    }
}

/// `R_BuildLightMap`: combines and scales multiple lightmaps into the 8.8
/// format in `BLOCKLIGHTS`, then stores them into `dest`.
///
/// # Safety
/// `surf` is a lit surface of a loaded brush model and `dest` addresses its atlas block; callers serialise per surface as in C.
#[no_mangle]
pub unsafe extern "C" fn R_BuildLightMap(surf: *mut MSurface, dest: *mut u8, stride: c_int) {
    // SAFETY: `surf` is a lit surface; `dest` points at its atlas block;
    // `BLOCKLIGHTS` is private to the calling worker's surface (the callers
    // serialise per surface as in C).
    unsafe {
        (*surf).cached_dlight = (*surf).dlightframe == r_framecount();
        let smax = (c_int::from((*surf).extents[0]) >> 4) + 1;
        let tmax = (c_int::from((*surf).extents[1]) >> 4) + 1;
        let size = smax * tmax;
        let mut lightmap = (*surf).samples.cast_const();

        if (*worldmodel()).lightdata.is_null() {
            // set to full bright if no light data
            ptr::write_bytes(
                ptr::addr_of_mut!(BLOCKLIGHTS).cast::<u32>(),
                255,
                (size * 3) as usize,
            );
        } else {
            // clear to no light
            ptr::write_bytes(
                ptr::addr_of_mut!(BLOCKLIGHTS).cast::<u32>(),
                0,
                (size * 3) as usize,
            );
            // add all the lightmaps
            if !lightmap.is_null() {
                for maps in 0..MAXLIGHTMAPS as usize {
                    let style = (*surf).styles[maps];
                    if style == 255 {
                        break;
                    }
                    let scale = lightstyle_value(style as usize) as u32;
                    (*surf).cached_light[maps] = scale as c_int; // 8.8 fraction
                    accumulate_lightmap(lightmap, scale, size);
                    lightmap = lightmap.add((size * 3) as usize); // skip to next lightmap
                }
            }
            // add all the dynamic lights
            if (*surf).dlightframe == r_framecount() {
                add_dynamic_lights(surf);
            }
        }
        store_lightmap(dest, smax, tmax, stride);
    }
}

/// `R_UploadLightmap`: copies the modified rows of one atlas to its texture.
unsafe fn upload_lightmap(lmap: c_int, lightmap_tex: *mut GlTexture) {
    // SAFETY: main thread after the frame's lightmap builds; the staging
    // allocation's command buffer is recording.
    unsafe {
        let lm = lightmap_of(lmap);
        let mut modified = false;
        for i in 0..TASKS_MAX_WORKERS {
            if (*lm).modified[i] != 0 {
                modified = true;
            }
            (*lm).modified[i] = 0;
        }
        if !modified {
            return;
        }
        let staging_size = LMBLOCK_WIDTH as i32 * i32::from((*lm).rectchange.h) * 4;
        if staging_size == 0 {
            // Empty copies are not valid. This can happen for a single frame when toggling r_gpulightmapupdate from 1 to 0
            return;
        }
        let a = with_ctx(|ctx| STAGING.allocate(ctx, staging_size, 4));
        let region = vk::BufferImageCopy {
            buffer_offset: a.buffer_offset as vk::DeviceSize,
            buffer_row_length: 0,
            buffer_image_height: 0,
            image_subresource: vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            },
            image_offset: vk::Offset3D {
                x: 0,
                y: i32::from((*lm).rectchange.t),
                z: 0,
            },
            image_extent: vk::Extent3D {
                width: LMBLOCK_WIDTH as u32,
                height: u32::from((*lm).rectchange.h),
                depth: 1,
            },
        };
        let mut barrier = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::SHADER_READ)
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image((*lightmap_tex).image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        let dev = device();
        dev.cmd_pipeline_barrier(
            a.command_buffer,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            core::slice::from_ref(&barrier),
        );
        dev.cmd_copy_buffer_to_image(
            a.command_buffer,
            a.buffer,
            (*lightmap_tex).image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &[region],
        );
        barrier.old_layout = vk::ImageLayout::TRANSFER_DST_OPTIMAL;
        barrier.new_layout = vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL;
        barrier.src_access_mask = vk::AccessFlags::TRANSFER_WRITE;
        barrier.dst_access_mask = vk::AccessFlags::SHADER_READ;
        dev.cmd_pipeline_barrier(
            a.command_buffer,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            core::slice::from_ref(&barrier),
        );

        STAGING.begin_copy();
        let data = (*lm)
            .data
            .add(usize::from((*lm).rectchange.t) * LMBLOCK_WIDTH * LIGHTMAP_BYTES);
        let color_format = with_ctx(|ctx| (*ctx.vg.as_ptr()).color_format);
        if color_format == vk::Format::A2B10G10R10_UNORM_PACK32 {
            let mut staging = a.data;
            let mut p = data;
            let end = data.add(staging_size as usize);
            while p < end {
                let packed =
                    u32::from(*p) | u32::from(*p.add(1)) << 10 | u32::from(*p.add(2)) << 20;
                staging.cast::<u32>().write_unaligned(packed);
                p = p.add(4);
                staging = staging.add(4);
            }
        } else {
            ptr::copy_nonoverlapping(data, a.data, staging_size as usize);
        }
        STAGING.end_copy();

        (*lm).rectchange = GlRect {
            l: LMBLOCK_WIDTH as u16,
            t: LMBLOCK_HEIGHT as u16,
            w: 0,
            h: 0,
        };
    }
}

// ---- GPU lightmap update + indirect dispatch ------------------------------

type LightmapRegions = [[u8; LM_CULL_COLS]; LM_CULL_ROWS];

/// `R_FlushUpdateLightmaps`: dispatches the lightmap compute for one batch,
/// merging the per-block regions into rectangles.
#[allow(clippy::too_many_arguments)]
unsafe fn flush_update_lightmaps(
    cbx: *mut CbContext,
    num_batch: usize,
    pre_barriers: &[vk::ImageMemoryBarrier<'static>],
    post_barriers: &[vk::ImageMemoryBarrier<'static>],
    lightmap_indexes: &[c_int],
    lightmap_regions: &mut [LightmapRegions],
    current_dlights: u32,
    cached_dlights: u32,
) {
    // SAFETY: `cbx` is the update-lightmaps primary context (recording);
    // every lightmap index is in range.
    unsafe {
        let (procs, rt, pipeline, push_descriptor) = with_ctx(|ctx| {
            let vgp = ctx.vg.as_ptr();
            let rt = cvar_value(ptr::addr_of!(c::menu::r_rtshadows)) != 0.0
                && *ptr::addr_of!(c::render::bmodel_tlas) != 0;
            let pipeline = if rt {
                (*vgp).update_lightmap_rt_pipeline
            } else {
                (*vgp).update_lightmap_pipeline
            };
            (
                CmdProcs::new(ctx.vg),
                rt,
                pipeline,
                (*vgp).vk_cmd_push_descriptor_set,
            )
        });
        let dev = device();
        let cb = (*cbx).cb;
        dev.cmd_pipeline_barrier(
            cb,
            vk::PipelineStageFlags::ALL_COMMANDS,
            vk::PipelineStageFlags::COMPUTE_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &pre_barriers[..num_batch],
        );
        cb::bind_pipeline(&procs, &mut *cbx, vk::PipelineBindPoint::COMPUTE, pipeline);
        if rt {
            let tlas = [vk::AccelerationStructureKHR::from_raw(*ptr::addr_of!(
                c::render::bmodel_tlas
            ))];
            let mut tlas_info = vk::WriteDescriptorSetAccelerationStructureKHR::default()
                .acceleration_structures(&tlas);
            let tlas_write = vk::WriteDescriptorSet::default()
                .push_next(&mut tlas_info)
                .dst_binding(0)
                .descriptor_count(1)
                .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR);
            let push = push_descriptor.expect("vkCmdPushDescriptorSetKHR is loaded with ray query");
            push(
                cb,
                vk::PipelineBindPoint::COMPUTE,
                pipeline.layout.handle,
                1,
                1,
                &tlas_write,
            );
        }
        let cur = CURRENT_COMPUTE_BUFFER_INDEX as u32;
        let offsets = [
            cur * (MAX_LIGHTSTYLES * core::mem::size_of::<f32>()) as u32,
            cur * (MAX_DLIGHTS * 2 * LM_COMPUTE_LIGHT_SIZE) as u32,
        ];
        let shadow_samples =
            1u32 << ((cvar_value(ptr::addr_of!(c::menu::r_rtshadows)) as c_int) + 1);
        let vieworg = vieworg();
        for j in 0..num_batch {
            let lm = lightmap_of(lightmap_indexes[j]);
            dev.cmd_bind_descriptor_sets(
                cb,
                vk::PipelineBindPoint::COMPUTE,
                pipeline.layout.handle,
                0,
                &[(*lm).descriptor_set],
                &offsets,
            );
            let regions = &mut lightmap_regions[j];
            for y in 0..LM_CULL_ROWS {
                for x in 0..LM_CULL_COLS {
                    if regions[y][x] == 0 {
                        continue;
                    }
                    let mut w = 1usize;
                    let mut h = 1usize;
                    let ty = regions[y][x];
                    while x + w < LM_CULL_COLS && regions[y][x + w] == ty {
                        regions[y][x + w] = 0;
                        w += 1;
                    }
                    while y + h < LM_CULL_ROWS && regions[y + h][x] == ty {
                        let ok = (x + 1..x + w).all(|i| regions[y + h][i] == ty);
                        if !ok {
                            break;
                        }
                        if x > 0
                            && regions[y + h][x - 1] == ty
                            && x + w < LM_CULL_COLS
                            && regions[y + h][x + w] == ty
                        {
                            break; // don't split if it continues both sides, (locally) turns 2 rectangles into 3
                        }
                        regions[y + h][x..x + w].fill(0);
                        h += 1;
                    }
                    let mut push_constants: [u32; 12] = [
                        current_dlights,
                        LMBLOCK_WIDTH as u32,
                        (x * LM_CULL_BLOCK_W / 8) as u32,
                        (y * LM_CULL_BLOCK_H / 8) as u32,
                        u32::from(ty == 1),
                        cached_dlights,
                        cur * MAX_MODELS as u32,
                        (1 - cur) * MAX_MODELS as u32,
                        vieworg[0].to_bits(),
                        vieworg[1].to_bits(),
                        vieworg[2].to_bits(),
                        0,
                    ];
                    let mut push_size = 11 * core::mem::size_of::<u32>();
                    if rt {
                        push_constants[11] = shadow_samples;
                        push_size = 12 * core::mem::size_of::<u32>();
                    }
                    let bytes: Vec<u8> = push_constants
                        .iter()
                        .flat_map(|v| v.to_ne_bytes())
                        .collect();
                    cb::push_constants(
                        &procs,
                        &*cbx,
                        vk::ShaderStageFlags::COMPUTE,
                        0,
                        &bytes[..push_size],
                    );
                    let rect_w = c_int::from((*lm).lightstyle_rectused[0].w);
                    let rect_h = c_int::from((*lm).lightstyle_rectused[0].h);
                    let dw = (rect_w / 8 - (x * LM_CULL_BLOCK_W / 8) as c_int)
                        .min((w * LM_CULL_BLOCK_W / 8) as c_int);
                    let dh = (rect_h / 8 - (y * LM_CULL_BLOCK_H / 8) as c_int)
                        .min((h * LM_CULL_BLOCK_H / 8) as c_int);
                    dev.cmd_dispatch(cb, dw as u32, dh as u32, 1);
                }
            }
        }
        dev.cmd_pipeline_barrier(
            cb,
            vk::PipelineStageFlags::COMPUTE_SHADER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &post_barriers[..num_batch],
        );
    }
}

/// `R_IndirectComputeDispatch`: clears and rebuilds the indirect draw
/// commands from the frame's surface visibility.
unsafe fn indirect_compute_dispatch(cbx: *mut CbContext) {
    // SAFETY: `cbx` is recording; the world's `surfvis` was written by the
    // frame's mark tasks.
    unsafe {
        if !indirect {
            return;
        }
        let procs = with_ctx(|ctx| CmdProcs::new(ctx.vg));
        let dev = device();
        let cb = (*cbx).cb;
        cb::begin_debug_utils_label(&procs, &*cbx, c"Indirect Compute");
        let world = worldmodel();
        let numsurfaces = (*world).numsurfaces;
        upload_visibility((*world).surfvis, (numsurfaces as usize + 31) / 8);

        let (clear_pipeline, draw_pipeline, desc_set, max_dispatch) = with_ctx(|ctx| {
            let vgp = ctx.vg.as_ptr();
            (
                (*vgp).indirect_clear_pipeline,
                (*vgp).indirect_draw_pipeline,
                (*vgp).indirect_compute_desc_set,
                (*vgp).device_properties.limits.max_compute_work_group_count[0],
            )
        });
        cb::bind_pipeline(
            &procs,
            &mut *cbx,
            vk::PipelineBindPoint::COMPUTE,
            clear_pipeline,
        );
        dev.cmd_bind_descriptor_sets(
            cb,
            vk::PipelineBindPoint::COMPUTE,
            clear_pipeline.layout.handle,
            0,
            &[desc_set],
            &[],
        );
        let num_indirect_draws = USED_INDIRECT_DRAWS as u32;
        cb::push_constants(
            &procs,
            &*cbx,
            vk::ShaderStageFlags::COMPUTE,
            0,
            &num_indirect_draws.to_ne_bytes(),
        );
        dev.cmd_dispatch(cb, num_indirect_draws.div_ceil(64), 1, 1);

        let memory_barrier = vk::MemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
            .dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE);
        dev.cmd_pipeline_barrier(
            cb,
            vk::PipelineStageFlags::COMPUTE_SHADER,
            vk::PipelineStageFlags::COMPUTE_SHADER,
            vk::DependencyFlags::empty(),
            &[memory_barrier],
            &[],
            &[],
        );

        cb::bind_pipeline(
            &procs,
            &mut *cbx,
            vk::PipelineBindPoint::COMPUTE,
            draw_pipeline,
        );
        let vieworg = vieworg();
        let visibility_offset = CURRENT_COMPUTE_BUFFER_INDEX as u32 * DYN_VISIBILITY_OFFSET / 4;
        let instance_base = BMODEL_INSTANCES_INDEX as u32 * MAX_MODELS as u32;
        let mut push = Vec::with_capacity(28);
        push.extend_from_slice(&((*model_precache(1)).numsurfaces).to_ne_bytes());
        push.extend_from_slice(&0u32.to_ne_bytes());
        push.extend_from_slice(&visibility_offset.to_ne_bytes());
        push.extend_from_slice(&vieworg[0].to_ne_bytes());
        push.extend_from_slice(&vieworg[1].to_ne_bytes());
        push.extend_from_slice(&vieworg[2].to_ne_bytes());
        push.extend_from_slice(&instance_base.to_ne_bytes());
        cb::push_constants(&procs, &*cbx, vk::ShaderStageFlags::COMPUTE, 0, &push);

        let num_workgroups = (numsurfaces as u32).div_ceil(64);
        let mut start: u32 = 0;
        loop {
            dev.cmd_dispatch(cb, max_dispatch.min(num_workgroups - start), 1, 1);
            start += max_dispatch;
            if start >= num_workgroups {
                break;
            }
            let start_offset = start * 64;
            cb::push_constants(
                &procs,
                &*cbx,
                vk::ShaderStageFlags::COMPUTE,
                4,
                &start_offset.to_ne_bytes(),
            );
        }

        let memory_barrier = vk::MemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
            .dst_access_mask(vk::AccessFlags::MEMORY_READ);
        dev.cmd_pipeline_barrier(
            cb,
            vk::PipelineStageFlags::COMPUTE_SHADER,
            vk::PipelineStageFlags::DRAW_INDIRECT | vk::PipelineStageFlags::VERTEX_INPUT,
            vk::DependencyFlags::empty(),
            &[memory_barrier],
            &[],
            &[],
        );
        cb::end_debug_utils_label(&procs, &*cbx);
    }
}

/// `R_UpdateLightmapsAndIndirect`: the frame task that uploads the light
/// styles/dlights/submodel transforms, dispatches the lightmap compute for
/// every modified atlas and rebuilds the indirect draws.
///
/// # Safety
/// Runs as one frame task after the mark/lightmap tasks; the upload buffers are mapped and the entity lists are stable for the frame.
#[no_mangle]
pub unsafe extern "C" fn R_UpdateLightmapsAndIndirect(_unused: *mut c_void) {
    // SAFETY: runs as one task per frame after the mark/lightmap tasks; the
    // upload buffers are host-visible and mapped for the process lifetime;
    // `cl`, `cl_dlights`, `d_lightstylevalue` and the entity lists are stable
    // for the frame.
    unsafe {
        let (cbx, procs) = with_ctx(|ctx| {
            (
                ptr::addr_of_mut!((*ctx.vg.as_ptr()).primary_cb_contexts[PCBX_UPDATE_LIGHTMAPS]),
                CmdProcs::new(ctx.vg),
            )
        });
        cb::begin_debug_utils_label(&procs, &*cbx, c"Update Lightmaps");
        let cur = CURRENT_COMPUTE_BUFFER_INDEX as usize;
        let styles = ptr::addr_of!(c::render::d_lightstylevalue);
        for i in 0..MAX_LIGHTSTYLES {
            *LIGHTSTYLES_SCALES_BUFFER_MAPPED.add(i + cur * MAX_LIGHTSTYLES) =
                (*styles)[i] as f32 / 256.0;
        }
        ptr::copy_nonoverlapping(
            ptr::addr_of!(CACHED_DLIGHTS).cast::<LmComputeLight>(),
            LIGHTS_BUFFER_MAPPED.add(cur * MAX_DLIGHTS * 2 + MAX_DLIGHTS),
            NUM_CACHED_DLIGHTS as usize,
        );

        let dlights = ptr::addr_of!(c::cl_main::cl_dlights).cast::<c::cl_tent::dlight_t>();
        let r_dynamic = cvar_value(ptr::addr_of!(c::render::r_dynamic));
        let time = cl_time();
        let mut num_used_dlights = 0usize;
        let mut used_dlights = [0usize; MAX_DLIGHTS];
        let mut squared_radius = [0f32; MAX_DLIGHTS];
        for i in 0..MAX_DLIGHTS {
            let dl = dlights.add(i);
            if r_dynamic == 0.0
                || f64::from((*dl).die) < time
                || (*dl).radius == 0.0
                || (*dl).radius < (*dl).minlight
            {
                continue;
            }
            let light = &mut CACHED_DLIGHTS[num_used_dlights];
            light.origin = (*dl).origin;
            light.radius = (*dl).radius;
            light.color = (*dl).color;
            // rerelease dynamiclights don't use minlight, so its sign packs the KEX intensity
            light.minlight = if (*dl).kex_intensity > 0.0 {
                -(*dl).kex_intensity
            } else {
                (*dl).minlight
            };
            light.cone_dir = (*dl).cone_dir;
            light.cone_cos = (*dl).cone_cos;
            squared_radius[num_used_dlights] = (*dl).radius * (*dl).radius;
            used_dlights[num_used_dlights] = i;
            num_used_dlights += 1;
        }
        ptr::copy_nonoverlapping(
            ptr::addr_of!(CACHED_DLIGHTS).cast::<LmComputeLight>(),
            LIGHTS_BUFFER_MAPPED.add(cur * MAX_DLIGHTS * 2),
            num_used_dlights,
        );

        // Movable brush submodels are lit in entity space: upload the current model to world transform for each submodel.
        // The GPU culls dlights against the transformed workgroup bounds, the CPU only schedules updates for the cull
        // blocks containing submodel surfaces while dlights are active. The transforms are only read while dlight
        // updates are dispatched, skip all of it when no dlights are active
        let any_dlight_updates = num_used_dlights > 0 || NUM_CACHED_DLIGHTS > 0;
        if any_dlight_updates {
            let transforms = SUBMODEL_TRANSFORMS_BUFFER_MAPPED.add(cur * MAX_MODELS * 12);
            for i in 0..NUM_WORLDMODEL_SUBMODELS as usize {
                let rows = transforms.add(i * 12);
                ptr::write_bytes(rows, 0, 12);
                *rows = 1.0;
                *rows.add(5) = 1.0;
                *rows.add(10) = 1.0;
            }
            let clp = ptr::addr_of!(cl);
            let num_entities = (*clp).num_entities;
            let num_statics = (*clp).num_statics;
            let world_surfaces = (*worldmodel()).surfaces;
            for i in 0..num_entities + num_statics {
                let e: *mut Entity = if i < num_entities {
                    (*clp).entities.cast::<Entity>().add(i as usize)
                } else {
                    (*(*clp).static_entities.add((i - num_entities) as usize)).cast::<Entity>()
                };
                let model = (*e).model;
                if model.is_null()
                    || (*model).needload
                    || !model_is_submodel(model)
                    || (*model).surfaces != world_surfaces
                {
                    continue;
                }
                let submodel = model_name_submodel(model);
                if submodel <= 0 || submodel >= NUM_WORLDMODEL_SUBMODELS {
                    continue;
                }
                let mut angles = (*e).angles;
                angles[0] = -angles[0]; // stupid quake bug
                let mut model_matrix = [0f32; 16];
                identity_matrix(&mut model_matrix);
                c::render::R_RotateForEntity(
                    model_matrix.as_mut_ptr(),
                    (*e).origin.as_mut_ptr(),
                    angles.as_mut_ptr(),
                    (*e).netstate.scale,
                );
                let rows = transforms.add(submodel as usize * 12);
                for row in 0..3 {
                    for col in 0..4 {
                        *rows.add(row * 4 + col) = model_matrix[col * 4 + row];
                    }
                }
            }
        }

        let mut num_lightmaps: u32 = 0;
        let mut num_batch = 0usize;
        // SAFETY: an all-zero `VkImageMemoryBarrier` is a valid (if
        // meaningless) value; every used entry is fully written below.
        let mut pre_barriers: [vk::ImageMemoryBarrier<'static>; UPDATE_LIGHTMAP_BATCH_SIZE] =
            [core::mem::zeroed(); UPDATE_LIGHTMAP_BATCH_SIZE];
        let mut post_barriers: [vk::ImageMemoryBarrier<'static>; UPDATE_LIGHTMAP_BATCH_SIZE] =
            [core::mem::zeroed(); UPDATE_LIGHTMAP_BATCH_SIZE];
        let mut lightmap_indexes = [0 as c_int; UPDATE_LIGHTMAP_BATCH_SIZE];
        let mut lightmap_regions: [LightmapRegions; UPDATE_LIGHTMAP_BATCH_SIZE] =
            [[[0u8; LM_CULL_COLS]; LM_CULL_ROWS]; UPDATE_LIGHTMAP_BATCH_SIZE];
        let framecount = r_framecount();
        for lightmap_index in 0..lightmap_count {
            let lm = lightmap_of(lightmap_index);
            let mut modified: u32 = 0;
            let mut regions: LightmapRegions = [[0u8; LM_CULL_COLS]; LM_CULL_ROWS]; // 1: dlights update only; 2: unconditional update
            for i in 0..TASKS_MAX_WORKERS {
                modified |= (*lm).modified[i];
                (*lm).modified[i] = 0;
            }
            if modified == 0 {
                continue;
            }
            let mut any_needs_dlight_update = false;
            let mut used_lightstyles: u32 = 0;
            let mut num_blocks: u32 = 0;
            #[allow(clippy::needless_range_loop)]
            for y in 0..LM_CULL_ROWS {
                for x in 0..LM_CULL_COLS {
                    let mut needs_update = false;
                    for i in 0..num_used_dlights {
                        let mut sq_dist = 0f32;
                        let origin = (*dlights.add(used_dlights[i])).origin;
                        let bounds = &(*lm).global_bounds[y][x];
                        for (j, &v) in origin.iter().enumerate() {
                            let mins = bounds.mins[j];
                            let maxs = bounds.maxs[j];
                            if v < mins {
                                sq_dist += (mins - v) * (mins - v);
                            }
                            if v > maxs {
                                sq_dist += (v - maxs) * (v - maxs);
                            }
                            if sq_dist > squared_radius[i] {
                                break;
                            }
                        }
                        if sq_dist <= squared_radius[i] {
                            (*lm).active_dlights[y][x] = 1;
                            needs_update = true;
                        }
                    }
                    if any_dlight_updates && (*lm).block_has_submodels[y][x] != 0 {
                        needs_update = true;
                    }
                    if !needs_update && (*lm).active_dlights[y][x] != 0 {
                        (*lm).active_dlights[y][x] = 0;
                        needs_update = true;
                    }
                    if needs_update {
                        any_needs_dlight_update = true;
                        regions[y][x] = if (*lm).cached_framecount == framecount - 1 {
                            1
                        } else {
                            2
                        };
                        num_blocks += 1;
                    }
                    if regions[y][x] != 2 {
                        for i in 0..(*lm).num_used_lightstyles[y][x] as usize {
                            let l = (*lm).used_lightstyles[y][x][i] as usize;
                            if (*lm).cached_light[l] != (*styles)[l] {
                                if regions[y][x] == 0 {
                                    num_blocks += 1;
                                }
                                regions[y][x] = 2;
                                if !any_needs_dlight_update {
                                    used_lightstyles |= 1 << (if l < 16 { l } else { l % 16 + 16 });
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            if !any_needs_dlight_update && (used_lightstyles & modified) == 0 {
                continue;
            }
            (*lm).cached_light = *styles;
            (*lm).cached_framecount = framecount;
            num_lightmaps += num_blocks;

            let batch_index = num_batch;
            num_batch += 1;
            lightmap_indexes[batch_index] = lightmap_index;
            lightmap_regions[batch_index] = regions;
            let range = vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            };
            let image = (*(*lm).texture).image;
            pre_barriers[batch_index] = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::SHADER_WRITE)
                .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .new_layout(vk::ImageLayout::GENERAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(range);
            post_barriers[batch_index] = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ)
                .old_layout(vk::ImageLayout::GENERAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(range);
            if num_batch == UPDATE_LIGHTMAP_BATCH_SIZE {
                flush_update_lightmaps(
                    cbx,
                    num_batch,
                    &pre_barriers,
                    &post_barriers,
                    &lightmap_indexes,
                    &mut lightmap_regions,
                    num_used_dlights as u32,
                    NUM_CACHED_DLIGHTS as u32,
                );
                num_batch = 0;
            }
        }
        if num_batch > 0 {
            flush_update_lightmaps(
                cbx,
                num_batch,
                &pre_barriers,
                &post_barriers,
                &lightmap_indexes,
                &mut lightmap_regions,
                num_used_dlights as u32,
                NUM_CACHED_DLIGHTS as u32,
            );
        }
        NUM_CACHED_DLIGHTS = num_used_dlights as c_int;
        atomic_u32(ptr::addr_of_mut!(c::render::rs_dynamiclightmaps))
            .fetch_add(num_lightmaps, Ordering::SeqCst);
        cb::end_debug_utils_label(&procs, &*cbx);
        indirect_compute_dispatch(cbx);
        CURRENT_COMPUTE_BUFFER_INDEX = (CURRENT_COMPUTE_BUFFER_INDEX + 1) % 2;
    }
}

/// `R_UploadLightmaps`: the CPU-lightmap path's per-frame texture uploads.
///
/// # Safety
/// Main thread after the frame's lightmap builds.
#[no_mangle]
pub unsafe extern "C" fn R_UploadLightmaps() {
    // SAFETY: main thread after the frame's lightmap builds.
    unsafe {
        let mut num_uploads: u32 = 0;
        for lmap in 0..lightmap_count {
            let lm = lightmap_of(lmap);
            let modified = (*lm).modified.iter().any(|m| *m != 0);
            if !modified {
                continue;
            }
            num_uploads += 1;
            upload_lightmap(lmap, (*lm).texture);
        }
        atomic_u32(ptr::addr_of_mut!(c::render::rs_dynamiclightmaps))
            .fetch_add(num_uploads, Ordering::SeqCst);
    }
}
