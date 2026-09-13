//! `r_world.c` -- world model rendering (Rust migration Phase 8 M8).
//!
//! Leaf/surface marking (scalar, SSE2 and NEON lanes, gated on the C
//! `use_simd` exactly as before -- ADR-010 cull parity is checked through
//! `-renderhash`), the world texture-chain draws, and the task graph entry
//! `R_MarkSurfaces`. `R_StoreEfrags` can raise through `Host_Guard`
//! (see `gl_refrag.rs`), so the synchronous path returns the guard code from
//! [`RWorld_MarkSurfaces`] for `Quake/r_world_glue.c` to `Host_Reraise`
//! (ADR-009); on the task paths there is no C frame to reraise into and the
//! C original would have longjmp'd a main-thread `jmp_buf` from a worker, so a
//! raised code becomes `Sys_Error` (recorded in the plan amendment log).

use core::ffi::{c_float, c_int, c_void};
use core::ptr;
use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use ash::vk;
use quake_c_sys as c;
use quake_c_sys::cvar_t;
use quake_math::mathlib::dot_product;
use quake_render::cb::{self, CmdProcs};
use quake_types::host::{ClientState, Efrag, Entity};
use quake_types::model_mem::{
    MLeaf, MSurface, QModel, Texture, SURF_DRAWLAVA, SURF_DRAWSLIME, SURF_DRAWTELE, SURF_DRAWTILED,
    SURF_DRAWTURB, SURF_DRAWWATER, SURF_PLANEBACK, TEXTYPE_CUTOUT, TEXTYPE_SKY,
};
use quake_types::plane::MPlane;
use quake_types::refdef::RefDef;
use quake_types::render::{CbContext, GlTexture, VulkanPipeline, MAX_BATCH_SIZE};

use crate::gl_refrag::RRefrag_StoreEfrags;
use crate::gl_rmisc::{device, vg, with_ctx, DYN};
use crate::r_brush::{
    bmodel_vertex_buffer, indirect, lightmaps, DrawGLPoly, R_DrawIndirectBrushes, R_MarkDeps,
    R_RenderDynamicLightmaps, R_TextureAnimation, R_UploadLightmaps,
};

extern "C" {
    /// `client_state_t cl` (ADR-007 row closed in Phase 7).
    static mut cl: ClientState;
    /// `gl_rmain.c:58` -- `refdef_t r_refdef`.
    static mut r_refdef: RefDef;
}

/// `texchain_t` (`gl_model.h:76`).
const CHAIN_WORLD: c_int = 0;
const CONTENTS_SOLID: c_int = -2;
const CONTENTS_SKY: c_int = -6;
/// `TEXTYPE_FIRSTLIQUID` / `TEXTYPE_LASTLIQUID` (`gl_model.h` macros).
const TEXTYPE_FIRSTLIQUID: c_int = 3;
const TEXTYPE_LASTLIQUID: c_int = 6;
const NUM_WORLD_CBX: usize = 6;
const MARK_SURFACE_CALLS_PER_WORKER: c_int = 4;
const ENTALPHA_DEFAULT: u8 = 0;
const CVAR_NONE: u32 = 0;
/// `soa_aabb_t` / `soa_plane_t` lane counts (`gl_model.h`).
const SOA_AABB_FLOATS: usize = 48;
const SOA_PLANE_FLOATS: usize = 32;

mod private_cvars {
    use quake_c_sys::cvar_t;

    pub const fn cvar(
        name: &'static core::ffi::CStr,
        string: &'static core::ffi::CStr,
        flags: u32,
    ) -> cvar_t {
        cvar_t {
            name: name.as_ptr(),
            string: string.as_ptr(),
            flags,
            value: 0.0,
            default_string: core::ptr::null(),
            callback: None,
            completion: None,
            next: core::ptr::null_mut(),
        }
    }
}

/// `r_world.c:27` -- `cvar_t r_parallelmark` (registered by `gl_rmisc_glue.c`).
#[no_mangle]
pub static mut r_parallelmark: cvar_t = private_cvars::cvar(c"r_parallelmark", c"1", CVAR_NONE);

static mut WORLD_TEXSTART: [c_int; NUM_WORLD_CBX] = [0; NUM_WORLD_CBX];
static mut WORLD_TEXEND: [c_int; NUM_WORLD_CBX] = [0; NUM_WORLD_CBX];

/// First non-OK `Host_Guard` code raised by `R_StoreEfrags` during the
/// current `R_MarkSurfaces` (see the module doc).
static PENDING_RAISE: AtomicI32 = AtomicI32::new(0);

/// `mark_surfaces_state_t` (`r_world.c:32`): the SIMD lanes are kept as plain
/// arrays and (re)loaded per use, which is bit-identical to holding the
/// registers.
#[derive(Clone, Copy)]
struct MarkSurfacesState {
    frustum_px: [[f32; 4]; 4],
    frustum_py: [[f32; 4]; 4],
    frustum_pz: [[f32; 4]; 4],
    frustum_pd: [[f32; 4]; 4],
    vieworg_px: [f32; 4],
    vieworg_py: [f32; 4],
    vieworg_pz: [f32; 4],
    frustum_ofsx: [usize; 4],
    frustum_ofsy: [usize; 4],
    frustum_ofsz: [usize; 4],
    vis: *mut u8,
}

static mut MARK_SURFACES_STATE: MarkSurfacesState = MarkSurfacesState {
    frustum_px: [[0.0; 4]; 4],
    frustum_py: [[0.0; 4]; 4],
    frustum_pz: [[0.0; 4]; 4],
    frustum_pd: [[0.0; 4]; 4],
    vieworg_px: [0.0; 4],
    vieworg_py: [0.0; 4],
    vieworg_pz: [0.0; 4],
    frustum_ofsx: [0; 4],
    frustum_ofsy: [0; 4],
    frustum_ofsz: [0; 4],
    vis: ptr::null_mut(),
};

// ---- small helpers --------------------------------------------------------

#[inline]
unsafe fn worldmodel() -> *mut QModel {
    // SAFETY: `cl` is the C client state; read on the main thread or inside
    // the frame's mark tasks, when `worldmodel` is stable.
    unsafe { (*ptr::addr_of!(cl)).worldmodel }
}

/// Returns `true` when `R_StoreEfrags` raised: the C original `longjmp`s out
/// of `R_MarkSurfaces` at that point (`Host_Error` disconnects and nulls
/// `cl.worldmodel`), so every marking loop must stop at once instead of
/// chaining the remaining leaves against a disconnected client.
#[inline]
#[must_use]
unsafe fn store_efrags(ppefrag: *mut *mut Efrag) -> bool {
    // SAFETY: the caller's contract (a leaf efrag list head).
    let raise = unsafe { RRefrag_StoreEfrags(ppefrag) };
    if raise != 0 {
        let _ = PENDING_RAISE.compare_exchange(0, raise, Ordering::SeqCst, Ordering::SeqCst);
    }
    raise != 0
}

/// Task-path handling of a raised `Host_Guard` code (module doc).
unsafe fn abort_on_pending_raise() {
    if PENDING_RAISE.load(Ordering::SeqCst) != 0 {
        // SAFETY: `Sys_Error` takes a C format string; it never returns.
        unsafe { c::Sys_Error(c"R_StoreEfrags: Host_Error raised on a task worker".as_ptr()) };
    }
}

#[inline]
fn cheatsafe_drawworld() -> bool {
    // SAFETY: plain bool written by the cheat-safe refresh on the main thread.
    unsafe { *ptr::addr_of!(c::render::r_drawworld_cheatsafe) }
}

#[inline]
fn cheatsafe_lightmap() -> bool {
    // SAFETY: as above.
    unsafe { *ptr::addr_of!(c::render::r_lightmap_cheatsafe) }
}

#[inline]
fn cheatsafe_fullbright() -> bool {
    // SAFETY: as above.
    unsafe { *ptr::addr_of!(c::render::r_fullbright_cheatsafe) }
}

#[inline]
fn cvar_value(cvar: *const cvar_t) -> f32 {
    // SAFETY: the C cvar statics live for the program.
    unsafe { (*cvar).value }
}

#[inline]
fn use_simd() -> bool {
    // SAFETY: `use_simd` is a C bool set once at init.
    unsafe { *ptr::addr_of!(c::render::use_simd) }
}

#[inline]
unsafe fn atomic_u32<'a>(p: *mut u32) -> &'a AtomicU32 {
    // SAFETY: the caller passes a 4-byte-aligned, live `atomic_uint32_t`.
    unsafe { AtomicU32::from_ptr(p) }
}

#[inline]
unsafe fn add_stat(stat: *mut u32, n: u32) {
    // SAFETY: the `rs_*` counters are C `atomic_uint32_t`s.
    unsafe { atomic_u32(stat).fetch_add(n, Ordering::SeqCst) };
}

// ---- texture chains -------------------------------------------------------

/// `R_ClearTextureChains` -- ericw
///
/// # Safety
/// `mod_` is a loaded brush model.
#[no_mangle]
pub unsafe extern "C" fn R_ClearTextureChains(mod_: *mut QModel, chain: c_int) {
    // SAFETY: the caller's contract.
    unsafe {
        let chain = chain as usize;
        for i in 0..(*mod_).numtextures as usize {
            let t = *(*mod_).textures.add(i);
            if !t.is_null() {
                (*t).texturechains[chain] = ptr::null_mut();
                (*t).chain_size[chain] = 0;
            }
        }
    }
}

/// `R_ChainSurface` -- ericw -- adds the given surface to its texture chain.
///
/// # Safety
/// `surf` belongs to a loaded brush model.
#[no_mangle]
pub unsafe extern "C" fn R_ChainSurface(surf: *mut MSurface, chain: c_int) {
    // SAFETY: the caller's contract.
    unsafe {
        let chain = chain as usize;
        let t = (*(*surf).texinfo).texture;
        (*surf).texturechains[chain] = (*t).texturechains[chain];
        (*t).texturechains[chain] = surf;
        (*t).chain_size[chain] += 1;
    }
}

/// `R_BackFaceCull` -- returns true if the surface is facing away from the
/// viewer (the C `double dot` receives a float difference, so the sign is
/// decided in single precision).
#[inline]
unsafe fn backface_cull(surf: *const MSurface) -> bool {
    // SAFETY: the caller passes a live surface.
    unsafe {
        let plane = (*surf).plane;
        let vieworg = &(*ptr::addr_of!(r_refdef)).vieworg;
        let dot = if (*plane).type_ < 3 {
            vieworg[(*plane).type_ as usize] - (*plane).dist
        } else {
            dot_product(vieworg, &(*plane).normal) - (*plane).dist
        };
        (dot < 0.0) ^ ((*surf).flags & SURF_PLANEBACK != 0)
    }
}

/// `R_SetupWorldCBXTexRanges` -- splits the world texture list across the
/// `NUM_WORLD_CBX` command buffers.
unsafe fn setup_world_cbx_tex_ranges(use_tasks: bool) {
    // SAFETY: main thread / marking task; `cl.worldmodel` is loaded.
    unsafe {
        let texstart = &mut *ptr::addr_of_mut!(WORLD_TEXSTART);
        let texend = &mut *ptr::addr_of_mut!(WORLD_TEXEND);
        *texstart = [0; NUM_WORLD_CBX];
        *texend = [0; NUM_WORLD_CBX];
        let wm = worldmodel();
        let num_textures = (*wm).texofs[TEXTYPE_SKY as usize];
        if !use_tasks {
            texstart[0] = 0;
            texend[0] = num_textures;
            return;
        }
        let skip = |i: c_int| -> Option<*mut Texture> {
            let t = *(*wm)
                .textures
                .add(*(*wm).usedtextures.add(i as usize) as usize);
            if t.is_null()
                || (*t).texturechains[CHAIN_WORLD as usize].is_null()
                || (*(*t).texturechains[CHAIN_WORLD as usize]).flags & SURF_DRAWTILED != 0
            {
                None
            } else {
                Some(t)
            }
        };
        let mut total_world_surfs: c_int = 0;
        for i in 0..num_textures {
            if let Some(t) = skip(i) {
                total_world_surfs += (*t).chain_size[CHAIN_WORLD as usize] as c_int;
            }
        }
        let num_surfs_per_cbx =
            (total_world_surfs + NUM_WORLD_CBX as c_int - 1) / NUM_WORLD_CBX as c_int;
        let mut current_cbx = 0usize;
        let mut num_assigned: c_int = 0;
        for i in 0..num_textures {
            let Some(t) = skip(i) else {
                continue;
            };
            if current_cbx >= NUM_WORLD_CBX {
                break;
            }
            texend[current_cbx] = i + 1;
            num_assigned += (*t).chain_size[CHAIN_WORLD as usize] as c_int;
            if num_assigned >= num_surfs_per_cbx {
                current_cbx += 1;
                if current_cbx < NUM_WORLD_CBX {
                    texstart[current_cbx] = i + 1;
                }
                num_assigned = 0;
            }
        }
    }
}

// ---- SIMD lane culling ----------------------------------------------------

/// SSE2 lanes (`r_world.c` `USE_SSE2`).
#[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
mod lanes {
    #[cfg(target_arch = "x86")]
    use core::arch::x86 as arch;
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64 as arch;

    use super::MarkSurfacesState;
    use arch::{__m128, _mm_add_ps, _mm_cmplt_ps, _mm_loadu_ps, _mm_movemask_ps, _mm_mul_ps};

    #[inline]
    unsafe fn load(p: *const f32) -> __m128 {
        // SAFETY: the callers index inside the SoA arrays.
        unsafe { _mm_loadu_ps(p) }
    }

    #[inline]
    unsafe fn movemask(a: __m128, b: __m128) -> u32 {
        // SAFETY: pure register ops; SSE is a baseline feature of the targets
        // this module is compiled for.
        unsafe { _mm_movemask_ps(_mm_cmplt_ps(a, b)) as u32 }
    }

    /// `R_BackFaceCullSIMD`: `planes` points at four `soa_plane_t`.
    pub(super) unsafe fn backface_cull(planes: *const f32, st: &MarkSurfacesState) -> u32 {
        // SAFETY: the caller's contract.
        unsafe {
            let px = load(st.vieworg_px.as_ptr());
            let py = load(st.vieworg_py.as_ptr());
            let pz = load(st.vieworg_pz.as_ptr());
            let mut activelanes = 0u32;
            for plane_index in 0..4usize {
                let plane = planes.add(plane_index * super::SOA_PLANE_FLOATS);
                let mut v0 = _mm_mul_ps(load(plane), px);
                let mut v1 = _mm_mul_ps(load(plane.add(4)), px);
                v0 = _mm_add_ps(v0, _mm_mul_ps(load(plane.add(8)), py));
                v1 = _mm_add_ps(v1, _mm_mul_ps(load(plane.add(12)), py));
                v0 = _mm_add_ps(v0, _mm_mul_ps(load(plane.add(16)), pz));
                v1 = _mm_add_ps(v1, _mm_mul_ps(load(plane.add(20)), pz));
                let pd0 = load(plane.add(24));
                let pd1 = load(plane.add(28));
                let plane_lanes = movemask(pd0, v0) | (movemask(pd1, v1) << 4);
                activelanes |= plane_lanes << (plane_index * 8);
            }
            activelanes
        }
    }

    /// `R_CullBoxSIMD`: `boxes` points at four `soa_aabb_t`.
    pub(super) unsafe fn cull_box(
        boxes: *const f32,
        mut activelanes: u32,
        st: &MarkSurfacesState,
    ) -> u32 {
        // SAFETY: the caller's contract.
        unsafe {
            for frustum_index in 0..4usize {
                if activelanes == 0 {
                    break;
                }
                let ofsx = st.frustum_ofsx[frustum_index];
                let ofsy = st.frustum_ofsy[frustum_index];
                let ofsz = st.frustum_ofsz[frustum_index];
                let px = load(st.frustum_px[frustum_index].as_ptr());
                let py = load(st.frustum_py[frustum_index].as_ptr());
                let pz = load(st.frustum_pz[frustum_index].as_ptr());
                let pd = load(st.frustum_pd[frustum_index].as_ptr());
                let mut frustum_lanes = 0u32;
                for boxes_index in 0..4usize {
                    let b = boxes.add(boxes_index * super::SOA_AABB_FLOATS);
                    let mut v0 = _mm_mul_ps(load(b.add(ofsx)), px);
                    let mut v1 = _mm_mul_ps(load(b.add(ofsx + 4)), px);
                    v0 = _mm_add_ps(v0, _mm_mul_ps(load(b.add(ofsy)), py));
                    v1 = _mm_add_ps(v1, _mm_mul_ps(load(b.add(ofsy + 4)), py));
                    v0 = _mm_add_ps(v0, _mm_mul_ps(load(b.add(ofsz)), pz));
                    v1 = _mm_add_ps(v1, _mm_mul_ps(load(b.add(ofsz + 4)), pz));
                    frustum_lanes |=
                        (movemask(pd, v0) | (movemask(pd, v1) << 4)) << (boxes_index * 8);
                }
                activelanes &= frustum_lanes;
            }
            activelanes
        }
    }
}

/// NEON lanes (`r_world.c` `USE_NEON`).
///
/// COMPAT (ADR-010): `vmlaq_f32` is the unfused multiply-add on both sides.
/// The C build compiles with `-ffp-contract=off` (`meson.build`), so clang's
/// `arm_neon.h` `a + b * c` definition stays a separate `fmul`/`fadd`, and
/// `core::arch::aarch64::vmlaq_f32` lowers to the same `simd_mul`/`simd_add`
/// pair (rustc never enables contraction). Do not switch to `vfmaq_f32`.
/// Not executed locally: the arm64 CI legs only build this module.
#[cfg(target_arch = "aarch64")]
mod lanes {
    use core::arch::aarch64::{
        float32x4_t, int32x4_t, uint32x4_t, vaddvq_u32, vcltq_f32, vld1q_f32, vld1q_s32, vmlaq_f32,
        vmulq_f32, vshlq_u32, vshrq_n_u32,
    };

    use super::MarkSurfacesState;

    #[inline]
    unsafe fn load(p: *const f32) -> float32x4_t {
        // SAFETY: the callers index inside the SoA arrays.
        unsafe { vld1q_f32(p) }
    }

    /// `NeonMoveMask`.
    #[inline]
    unsafe fn neon_movemask(input: uint32x4_t) -> u32 {
        // SAFETY: NEON is baseline on aarch64.
        unsafe {
            let shift: int32x4_t = vld1q_s32([0i32, 1, 2, 3].as_ptr());
            vaddvq_u32(vshlq_u32(vshrq_n_u32::<31>(input), shift))
        }
    }

    #[inline]
    unsafe fn movemask(a: float32x4_t, b: float32x4_t) -> u32 {
        // SAFETY: as above.
        unsafe { neon_movemask(vcltq_f32(a, b)) }
    }

    pub(super) unsafe fn backface_cull(planes: *const f32, st: &MarkSurfacesState) -> u32 {
        // SAFETY: the caller's contract.
        unsafe {
            let px = load(st.vieworg_px.as_ptr());
            let py = load(st.vieworg_py.as_ptr());
            let pz = load(st.vieworg_pz.as_ptr());
            let mut activelanes = 0u32;
            for plane_index in 0..4usize {
                let plane = planes.add(plane_index * super::SOA_PLANE_FLOATS);
                let mut v0 = vmulq_f32(load(plane), px);
                let mut v1 = vmulq_f32(load(plane.add(4)), px);
                v0 = vmlaq_f32(v0, load(plane.add(8)), py);
                v1 = vmlaq_f32(v1, load(plane.add(12)), py);
                v0 = vmlaq_f32(v0, load(plane.add(16)), pz);
                v1 = vmlaq_f32(v1, load(plane.add(20)), pz);
                let pd0 = load(plane.add(24));
                let pd1 = load(plane.add(28));
                let plane_lanes = movemask(pd0, v0) | (movemask(pd1, v1) << 4);
                activelanes |= plane_lanes << (plane_index * 8);
            }
            activelanes
        }
    }

    pub(super) unsafe fn cull_box(
        boxes: *const f32,
        mut activelanes: u32,
        st: &MarkSurfacesState,
    ) -> u32 {
        // SAFETY: the caller's contract.
        unsafe {
            for frustum_index in 0..4usize {
                if activelanes == 0 {
                    break;
                }
                let ofsx = st.frustum_ofsx[frustum_index];
                let ofsy = st.frustum_ofsy[frustum_index];
                let ofsz = st.frustum_ofsz[frustum_index];
                let px = load(st.frustum_px[frustum_index].as_ptr());
                let py = load(st.frustum_py[frustum_index].as_ptr());
                let pz = load(st.frustum_pz[frustum_index].as_ptr());
                let pd = load(st.frustum_pd[frustum_index].as_ptr());
                let mut frustum_lanes = 0u32;
                for boxes_index in 0..4usize {
                    let b = boxes.add(boxes_index * super::SOA_AABB_FLOATS);
                    let mut v0 = vmulq_f32(load(b.add(ofsx)), px);
                    let mut v1 = vmulq_f32(load(b.add(ofsx + 4)), px);
                    v0 = vmlaq_f32(v0, load(b.add(ofsy)), py);
                    v1 = vmlaq_f32(v1, load(b.add(ofsy + 4)), py);
                    v0 = vmlaq_f32(v0, load(b.add(ofsz)), pz);
                    v1 = vmlaq_f32(v1, load(b.add(ofsz + 4)), pz);
                    frustum_lanes |=
                        (movemask(pd, v0) | (movemask(pd, v1) << 4)) << (boxes_index * 8);
                }
                activelanes &= frustum_lanes;
            }
            activelanes
        }
    }
}

/// Scalar reference lanes (ADR-004/ADR-010): the same per-lane arithmetic in
/// the same order, for targets without an intrinsic implementation.
#[cfg(not(any(target_arch = "x86_64", target_arch = "x86", target_arch = "aarch64")))]
mod lanes {
    use super::MarkSurfacesState;

    #[inline]
    unsafe fn load(p: *const f32) -> [f32; 4] {
        // SAFETY: the callers index inside the SoA arrays.
        unsafe { [*p, *p.add(1), *p.add(2), *p.add(3)] }
    }

    #[inline]
    fn mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
        [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]]
    }

    #[inline]
    fn add(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
        [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
    }

    #[inline]
    fn movemask(a: [f32; 4], b: [f32; 4]) -> u32 {
        let mut m = 0u32;
        for i in 0..4 {
            if a[i] < b[i] {
                m |= 1 << i;
            }
        }
        m
    }

    pub(super) unsafe fn backface_cull(planes: *const f32, st: &MarkSurfacesState) -> u32 {
        // SAFETY: the caller's contract.
        unsafe {
            let (px, py, pz) = (st.vieworg_px, st.vieworg_py, st.vieworg_pz);
            let mut activelanes = 0u32;
            for plane_index in 0..4usize {
                let plane = planes.add(plane_index * super::SOA_PLANE_FLOATS);
                let mut v0 = mul(load(plane), px);
                let mut v1 = mul(load(plane.add(4)), px);
                v0 = add(v0, mul(load(plane.add(8)), py));
                v1 = add(v1, mul(load(plane.add(12)), py));
                v0 = add(v0, mul(load(plane.add(16)), pz));
                v1 = add(v1, mul(load(plane.add(20)), pz));
                let pd0 = load(plane.add(24));
                let pd1 = load(plane.add(28));
                let plane_lanes = movemask(pd0, v0) | (movemask(pd1, v1) << 4);
                activelanes |= plane_lanes << (plane_index * 8);
            }
            activelanes
        }
    }

    pub(super) unsafe fn cull_box(
        boxes: *const f32,
        mut activelanes: u32,
        st: &MarkSurfacesState,
    ) -> u32 {
        // SAFETY: the caller's contract.
        unsafe {
            for frustum_index in 0..4usize {
                if activelanes == 0 {
                    break;
                }
                let ofsx = st.frustum_ofsx[frustum_index];
                let ofsy = st.frustum_ofsy[frustum_index];
                let ofsz = st.frustum_ofsz[frustum_index];
                let px = st.frustum_px[frustum_index];
                let py = st.frustum_py[frustum_index];
                let pz = st.frustum_pz[frustum_index];
                let pd = st.frustum_pd[frustum_index];
                let mut frustum_lanes = 0u32;
                for boxes_index in 0..4usize {
                    let b = boxes.add(boxes_index * super::SOA_AABB_FLOATS);
                    let mut v0 = mul(load(b.add(ofsx)), px);
                    let mut v1 = mul(load(b.add(ofsx + 4)), px);
                    v0 = add(v0, mul(load(b.add(ofsy)), py));
                    v1 = add(v1, mul(load(b.add(ofsy + 4)), py));
                    v0 = add(v0, mul(load(b.add(ofsz)), pz));
                    v1 = add(v1, mul(load(b.add(ofsz + 4)), pz));
                    frustum_lanes |=
                        (movemask(pd, v0) | (movemask(pd, v1) << 4)) << (boxes_index * 8);
                }
                activelanes &= frustum_lanes;
            }
            activelanes
        }
    }
}

#[inline]
unsafe fn leaf_bounds(wm: *mut QModel, soa_index: usize) -> *const f32 {
    // SAFETY: `soa_leafbounds` holds `(numleafs + 31) / 32 * 4` entries.
    unsafe { (*wm).soa_leafbounds.add(soa_index).cast::<f32>() }
}

#[inline]
unsafe fn surf_planes(wm: *mut QModel, soa_index: usize) -> *const f32 {
    // SAFETY: `soa_surfplanes` holds `(numsurfaces + 31) / 32 * 4` entries.
    unsafe { (*wm).soa_surfplanes.add(soa_index).cast::<f32>() }
}

/// Marks a chained, front-facing world surface the way every marking path
/// does after `R_ChainSurface`: dynamic lightmaps or the GPU lightmap dirty
/// bits, then the warp flag.
#[inline]
unsafe fn note_surface_lightmap_and_warp(
    surf: *mut MSurface,
    worker_index: usize,
    gpu_lightmap_update: bool,
) {
    // SAFETY: the caller passes a live world surface.
    unsafe {
        if !gpu_lightmap_update {
            R_RenderDynamicLightmaps(surf);
        } else if (*surf).lightmaptexturenum >= 0 {
            (*lightmaps.add((*surf).lightmaptexturenum as usize)).modified[worker_index] |=
                (*surf).styles_bitmap;
        }
        note_surface_warp(surf);
    }
}

#[inline]
unsafe fn note_surface_warp(surf: *mut MSurface) {
    // SAFETY: the caller passes a live world surface.
    unsafe {
        let t = (*(*surf).texinfo).texture;
        if !(*t).warpimage.is_null() {
            atomic_u32(ptr::addr_of_mut!((*t).update_warp)).store(1, Ordering::Relaxed);
        }
    }
}

#[inline]
unsafe fn mark_lightmap_modified(surf: *mut MSurface, worker_index: usize) {
    // SAFETY: the caller passes a live world surface.
    unsafe {
        if (*surf).lightmaptexturenum >= 0 {
            (*lightmaps.add((*surf).lightmaptexturenum as usize)).modified[worker_index] |=
                (*surf).styles_bitmap;
        }
    }
}

/// `R_MarkVisSurfacesSIMD` -- the single-task SIMD marking path.
unsafe fn mark_vis_surfaces_simd(use_tasks: bool) {
    // SAFETY: main thread or the marking task; `R_MarkSurfacesPrepare` ran.
    unsafe {
        let wm = worldmodel();
        let numleafs = (*wm).numleafs as u32;
        let numsurfaces = (*wm).numsurfaces as u32;
        let st = *ptr::addr_of!(MARK_SURFACES_STATE);
        let vis = st.vis.cast::<u32>();
        let surfvis = (*wm).surfvis.cast::<u32>();
        let mut current_combined_dep_index = c_int::MAX;
        let cst_r_drawworld_cheatsafe = cheatsafe_drawworld();
        let cst_indirect = *ptr::addr_of!(indirect);
        let cst_r_oldskyleaf = cvar_value(ptr::addr_of!(c::render::r_oldskyleaf)) > 0.0;
        let cst_r_gpulightmapupdate =
            cvar_value(ptr::addr_of!(c::render::r_gpulightmapupdate)) > 0.0;

        let mut i = 0u32;
        while i < numleafs {
            let mut mask = *vis.add((i / 32) as usize);
            if mask != 0 {
                mask = lanes::cull_box(leaf_bounds(wm, (i / 8) as usize), mask, &st);
                while mask != 0 {
                    let j = mask.trailing_zeros();
                    mask &= !(1u32 << j);
                    let leaf = (*wm).leafs.add((1 + i + j) as usize);
                    if cst_r_drawworld_cheatsafe
                        && ((*leaf).contents != CONTENTS_SKY || cst_r_oldskyleaf)
                    {
                        for k in 0..(*leaf).nummarksurfaces as usize {
                            let index = *(*leaf).firstmarksurface.add(k) as u32;
                            *surfvis.add((index / 32) as usize) |= 1u32 << (index % 32);
                        }
                        if cst_indirect && current_combined_dep_index != (*leaf).combined_deps {
                            R_MarkDeps((*leaf).combined_deps, 0);
                            current_combined_dep_index = (*leaf).combined_deps;
                        }
                    }
                    if !(*leaf).efrags.is_null()
                        && store_efrags(ptr::addr_of_mut!((*leaf).efrags).cast::<*mut Efrag>())
                    {
                        return;
                    }
                }
            }
            i += 32;
        }

        if cst_indirect {
            return;
        }

        let mut brushpolys = 0u32;
        let mut i = 0u32;
        while i < numsurfaces {
            let mut mask = *surfvis.add((i / 32) as usize);
            if mask != 0 {
                mask &= lanes::backface_cull(surf_planes(wm, (i / 8) as usize), &st);
                while mask != 0 {
                    let j = mask.trailing_zeros();
                    mask &= !(1u32 << j);
                    let surf = (*wm).surfaces.add((i + j) as usize);
                    brushpolys += 1;
                    R_ChainSurface(surf, CHAIN_WORLD);
                    note_surface_lightmap_and_warp(surf, 0, cst_r_gpulightmapupdate);
                }
            }
            i += 32;
        }
        add_stat(ptr::addr_of_mut!(c::render::rs_brushpolys), brushpolys);
        setup_world_cbx_tex_ranges(use_tasks);
    }
}

/// Batch split shared by the indexed marking tasks: `(nominal, count)` for
/// `batch_index` over `num_32` 32-element words.
#[inline]
unsafe fn batch_range(batch_index: c_int, num_items: u32) -> (u32, u32) {
    // SAFETY: `Tasks_NumWorkers` is a plain C getter.
    let nb_batchs = unsafe { MARK_SURFACE_CALLS_PER_WORKER * c::tasks::Tasks_NumWorkers() } as u32;
    let num_32 = num_items.div_ceil(32);
    let nominal = num_32 / nb_batchs;
    let count = if batch_index as u32 != nb_batchs - 1 {
        nominal
    } else {
        num_32 - (nb_batchs - 1) * nominal
    };
    (nominal, count)
}

/// `R_MarkLeafsSIMD` -- indexed task.
unsafe extern "C" fn mark_leafs_simd(index: c_int, _unused: *mut c_void) {
    // SAFETY: task worker; `R_MarkSurfacesPrepare` ran before this task.
    unsafe {
        let wm = worldmodel();
        let (nominal, nb_32leaf_in_batch) = batch_range(index, (*wm).numleafs as u32);
        let surfvis = (*wm).surfvis.cast::<u32>();
        let st = *ptr::addr_of!(MARK_SURFACES_STATE);
        let vis = st.vis.cast::<u32>();
        let mut current_surfvis_index_written = 0u32;
        let mut current_surfvis_written = 0u32;
        let mut current_combined_dep_index = c_int::MAX;
        let cst_r_drawworld_cheatsafe = cheatsafe_drawworld();
        let cst_indirect = *ptr::addr_of!(indirect);
        let cst_r_oldskyleaf = cvar_value(ptr::addr_of!(c::render::r_oldskyleaf)) > 0.0;
        let worker_index = c::tasks::Tasks_GetWorkerIndex();

        for k in 0..nb_32leaf_in_batch {
            let index_32leaf = index as u32 * nominal + k;
            let first_leaf = index_32leaf * 32 + 1;
            let mask = vis.add(index_32leaf as usize);
            if *mask == 0 {
                continue;
            }
            *mask = lanes::cull_box(leaf_bounds(wm, (index_32leaf * 4) as usize), *mask, &st);
            let mut mask_iter = *mask;
            while mask_iter != 0 {
                let i = mask_iter.trailing_zeros();
                let leaf = (*wm).leafs.add((first_leaf + i) as usize);
                if cst_r_drawworld_cheatsafe
                    && ((*leaf).contents != CONTENTS_SKY || cst_r_oldskyleaf)
                {
                    for j in 0..(*leaf).nummarksurfaces as usize {
                        let surf_index = *(*leaf).firstmarksurface.add(j) as u32;
                        if surf_index / 32 != current_surfvis_index_written {
                            atomic_u32(surfvis.add(current_surfvis_index_written as usize))
                                .fetch_or(current_surfvis_written, Ordering::Relaxed);
                            current_surfvis_index_written = surf_index / 32;
                            current_surfvis_written = 0;
                        }
                        current_surfvis_written |= 1u32 << (surf_index % 32);
                    }
                    if cst_indirect && current_combined_dep_index != (*leaf).combined_deps {
                        R_MarkDeps((*leaf).combined_deps, worker_index);
                        current_combined_dep_index = (*leaf).combined_deps;
                    }
                }
                let bit_mask = !(1u32 << i);
                if (*leaf).efrags.is_null() {
                    *mask &= bit_mask;
                }
                mask_iter &= bit_mask;
            }
        }
        atomic_u32(surfvis.add(current_surfvis_index_written as usize))
            .fetch_or(current_surfvis_written, Ordering::SeqCst);
    }
}

/// `R_BackfaceCullSurfacesSIMD` -- indexed task.
unsafe extern "C" fn backface_cull_surfaces_simd(index: c_int, _unused: *mut c_void) {
    // SAFETY: task worker after the leaf marking tasks.
    unsafe {
        let wm = worldmodel();
        let surfvis = (*wm).surfvis.cast::<u32>();
        let (nominal, nb_32surf_in_batch) = batch_range(index, (*wm).numsurfaces as u32);
        let st = *ptr::addr_of!(MARK_SURFACES_STATE);
        let worker_index = c::tasks::Tasks_GetWorkerIndex() as usize;
        for k in 0..nb_32surf_in_batch {
            let index_32surf = index as u32 * nominal + k;
            let mask = surfvis.add(index_32surf as usize);
            if *mask == 0 {
                continue;
            }
            *mask &= lanes::backface_cull(surf_planes(wm, (index_32surf * 4) as usize), &st);
            let mut mask_iter = *mask;
            while mask_iter != 0 {
                let i = mask_iter.trailing_zeros();
                let surf = (*wm).surfaces.add((index_32surf * 32 + i) as usize);
                mark_lightmap_modified(surf, worker_index);
                note_surface_warp(surf);
                mask_iter &= !(1u32 << i);
            }
        }
    }
}

/// `R_StoreLeafEFrags` -- task.
unsafe extern "C" fn store_leaf_efrags(_unused: *mut c_void) {
    // SAFETY: task after the leaf marking tasks; `vis` only keeps leaves with
    // efrags on the parallel paths.
    unsafe {
        let wm = worldmodel();
        let numleafs = (*wm).numleafs as u32;
        let vis = (*ptr::addr_of!(MARK_SURFACES_STATE)).vis.cast::<u32>();
        let mut i = 0u32;
        'words: while i < numleafs {
            let mut mask = *vis.add((i / 32) as usize);
            while mask != 0 {
                let j = mask.trailing_zeros();
                mask &= !(1u32 << j);
                let leaf = (*wm).leafs.add((1 + i + j) as usize);
                if store_efrags(ptr::addr_of_mut!((*leaf).efrags).cast::<*mut Efrag>()) {
                    break 'words;
                }
            }
            i += 32;
        }
        abort_on_pending_raise();
    }
}

/// `R_ChainVisSurfaces` -- task (payload: the `use_tasks` flag).
unsafe fn chain_vis_surfaces(use_tasks: bool) {
    // SAFETY: after the backface-cull tasks.
    unsafe {
        let wm = worldmodel();
        let numsurfaces = (*wm).numsurfaces as u32;
        let surfvis = (*wm).surfvis.cast::<u32>();
        let mut brushpolys = 0u32;
        let mut i = 0u32;
        while i < numsurfaces {
            let mut mask = *surfvis.add((i / 32) as usize);
            while mask != 0 {
                let j = mask.trailing_zeros();
                mask &= !(1u32 << j);
                let surf = (*wm).surfaces.add((i + j) as usize);
                brushpolys += 1;
                R_ChainSurface(surf, CHAIN_WORLD);
            }
            i += 32;
        }
        add_stat(ptr::addr_of_mut!(c::render::rs_brushpolys), brushpolys);
        setup_world_cbx_tex_ranges(use_tasks);
    }
}

unsafe extern "C" fn chain_vis_surfaces_task(payload: *mut c_void) {
    // SAFETY: the payload is the copied `qboolean use_tasks`.
    unsafe { chain_vis_surfaces(*payload.cast::<bool>()) }
}

/// `R_GetTransparentWaterTypes`.
fn transparent_water_types() -> c_int {
    // SAFETY: the `map_*alpha` floats are set by the C fog/worldspawn parser.
    let (lava, tele, slime, water, fallback) = unsafe {
        (
            *ptr::addr_of!(c::render::map_lavaalpha),
            *ptr::addr_of!(c::render::map_telealpha),
            *ptr::addr_of!(c::render::map_slimealpha),
            *ptr::addr_of!(c::render::map_wateralpha),
            *ptr::addr_of!(c::render::map_fallbackalpha),
        )
    };
    let mut types = 0;
    if (if lava > 0.0 { lava } else { fallback }) != 1.0 {
        types |= SURF_DRAWLAVA;
    }
    if (if tele > 0.0 { tele } else { fallback }) != 1.0 {
        types |= SURF_DRAWTELE;
    }
    if (if slime > 0.0 { slime } else { fallback }) != 1.0 {
        types |= SURF_DRAWSLIME;
    }
    if water != 1.0 {
        types |= SURF_DRAWWATER;
    }
    types
}

/// `R_PrepareTransparentWaterSurfList`.
unsafe fn prepare_transparent_water_surf_list() {
    // SAFETY: main thread; `cl.worldmodel` is loaded.
    unsafe {
        let types = transparent_water_types();
        let wm = worldmodel();
        if (*wm).water_surfs_specials != types {
            if (*wm).water_surfs.is_null() {
                (*wm).water_surfs =
                    c::Mem_Realloc((*wm).water_surfs.cast::<c_void>(), 8192 * 4).cast::<i32>();
            }
            (*wm).used_water_surfs = 0;
            for i in 0..(*wm).numsurfaces {
                if (*(*wm).surfaces.add(i as usize)).flags & types != 0 {
                    let used = (*wm).used_water_surfs;
                    if used >= 8192 && used & (used - 1) == 0 {
                        (*wm).water_surfs = c::Mem_Realloc(
                            (*wm).water_surfs.cast::<c_void>(),
                            used as usize * 2 * 4,
                        )
                        .cast::<i32>();
                    }
                    *(*wm).water_surfs.add(used as usize) = i;
                    (*wm).used_water_surfs += 1;
                }
            }
            (*wm).water_surfs_specials = types;
        }
    }
}

/// `R_ChainVisSurfaces_TransparentWater`.
unsafe fn chain_vis_surfaces_transparent_water() {
    // SAFETY: main thread inside the frame.
    unsafe {
        prepare_transparent_water_surf_list();
        let wm = worldmodel();
        let surfvis = (*wm).surfvis.cast::<u32>();
        for i in 0..(*wm).used_water_surfs as usize {
            let j = *(*wm).water_surfs.add(i) as u32;
            let surf = (*wm).surfaces.add(j as usize);
            if *surfvis.add((j / 32) as usize) & (1u32 << (j % 32)) != 0 && !backface_cull(surf) {
                R_ChainSurface(surf, CHAIN_WORLD);
            }
        }
    }
}

/// `R_MarkLeafsParallel` -- indexed task (scalar path).
unsafe extern "C" fn mark_leafs_parallel(index: c_int, _unused: *mut c_void) {
    // SAFETY: task worker; `R_MarkSurfacesPrepare` ran before this task.
    unsafe {
        let wm = worldmodel();
        let (nominal, nb_32leaf_in_batch) = batch_range(index, (*wm).numleafs as u32);
        let surfvis = (*wm).surfvis.cast::<u32>();
        let vis = (*ptr::addr_of!(MARK_SURFACES_STATE)).vis.cast::<u32>();
        let mut current_surfvis_index_written = 0u32;
        let mut current_surfvis_written = 0u32;
        let mut current_combined_dep_index = c_int::MAX;
        let cst_r_drawworld_cheatsafe = cheatsafe_drawworld();
        let cst_indirect = *ptr::addr_of!(indirect);
        let cst_r_oldskyleaf = cvar_value(ptr::addr_of!(c::render::r_oldskyleaf)) > 0.0;
        let worker_index = c::tasks::Tasks_GetWorkerIndex();

        for k in 0..nb_32leaf_in_batch {
            let index_32leaf = index as u32 * nominal + k;
            let first_leaf = index_32leaf * 32 + 1;
            let mask = vis.add(index_32leaf as usize);
            if *mask == 0 {
                continue;
            }
            let mut mask_iter = *mask;
            while mask_iter != 0 {
                let i = mask_iter.trailing_zeros();
                let bit_mask = !(1u32 << i);
                mask_iter &= bit_mask;
                let leaf = (*wm).leafs.add((first_leaf + i) as usize);
                if c::render::R_CullBox((*leaf).minmaxs.as_ptr(), (*leaf).minmaxs.as_ptr().add(3)) {
                    *mask &= bit_mask;
                    continue;
                }
                if (*leaf).efrags.is_null() {
                    *mask &= bit_mask;
                }
                if cst_r_drawworld_cheatsafe
                    && ((*leaf).contents != CONTENTS_SKY || cst_r_oldskyleaf)
                {
                    for j in 0..(*leaf).nummarksurfaces as usize {
                        let surf_index = *(*leaf).firstmarksurface.add(j) as u32;
                        if surf_index / 32 != current_surfvis_index_written {
                            atomic_u32(surfvis.add(current_surfvis_index_written as usize))
                                .fetch_or(current_surfvis_written, Ordering::Relaxed);
                            current_surfvis_index_written = surf_index / 32;
                            current_surfvis_written = 0;
                        }
                        current_surfvis_written |= 1u32 << (surf_index % 32);
                    }
                    if cst_indirect && current_combined_dep_index != (*leaf).combined_deps {
                        R_MarkDeps((*leaf).combined_deps, worker_index);
                        current_combined_dep_index = (*leaf).combined_deps;
                    }
                }
            }
        }
        atomic_u32(surfvis.add(current_surfvis_index_written as usize))
            .fetch_or(current_surfvis_written, Ordering::SeqCst);
    }
}

/// `R_BackfaceCullSurfacesParallel` -- indexed task (scalar path).
unsafe extern "C" fn backface_cull_surfaces_parallel(index: c_int, _unused: *mut c_void) {
    // SAFETY: task worker after the leaf marking tasks.
    unsafe {
        let wm = worldmodel();
        let surfvis = (*wm).surfvis.cast::<u32>();
        let (nominal, nb_32surf_in_batch) = batch_range(index, (*wm).numsurfaces as u32);
        let worker_index = c::tasks::Tasks_GetWorkerIndex() as usize;
        for k in 0..nb_32surf_in_batch {
            let index_32surf = index as u32 * nominal + k;
            let mask = surfvis.add(index_32surf as usize);
            if *mask == 0 {
                continue;
            }
            let mut mask_iter = *mask;
            while mask_iter != 0 {
                let i = mask_iter.trailing_zeros();
                let bit_mask = !(1u32 << i);
                mask_iter &= bit_mask;
                let surf = (*wm).surfaces.add((index_32surf * 32 + i) as usize);
                if backface_cull(surf) {
                    // COMPAT: r_world.c:851 writes `*surfvis &= bit_mask` --
                    // word 0, not `*mask` -- and the draw path still chains
                    // the back-facing surface. Preserved (plan amendment log).
                    *surfvis &= bit_mask;
                } else {
                    mark_lightmap_modified(surf, worker_index);
                    note_surface_warp(surf);
                }
            }
        }
    }
}

/// `R_MarkVisSurfaces` -- the single-task scalar marking path.
unsafe fn mark_vis_surfaces(use_tasks: bool) {
    // SAFETY: main thread or the marking task; `R_MarkSurfacesPrepare` ran.
    unsafe {
        let wm = worldmodel();
        let mut brushpolys = 0u32;
        let vis = (*ptr::addr_of!(MARK_SURFACES_STATE)).vis;
        let surfvis = (*wm).surfvis.cast::<u32>();
        let mut current_combined_dep_index = c_int::MAX;
        let cst_r_drawworld_cheatsafe = cheatsafe_drawworld();
        let cst_indirect = *ptr::addr_of!(indirect);
        let cst_r_oldskyleaf = cvar_value(ptr::addr_of!(c::render::r_oldskyleaf)) > 0.0;
        let cst_r_gpulightmapupdate =
            cvar_value(ptr::addr_of!(c::render::r_gpulightmapupdate)) > 0.0;
        let r_visframecount = *ptr::addr_of!(c::render::r_visframecount);

        for i in 0..(*wm).numleafs as usize {
            let leaf = (*wm).leafs.add(1 + i);
            if *vis.add(i / 8) & (1u8 << (i % 8)) == 0 {
                continue;
            }
            if c::render::R_CullBox((*leaf).minmaxs.as_ptr(), (*leaf).minmaxs.as_ptr().add(3)) {
                continue;
            }
            if cst_r_drawworld_cheatsafe && ((*leaf).contents != CONTENTS_SKY || cst_r_oldskyleaf) {
                if cst_indirect && current_combined_dep_index != (*leaf).combined_deps {
                    R_MarkDeps((*leaf).combined_deps, 0);
                    current_combined_dep_index = (*leaf).combined_deps;
                }
                for j in 0..(*leaf).nummarksurfaces as usize {
                    if cst_indirect {
                        let surf_index = *(*leaf).firstmarksurface.add(j) as u32;
                        *surfvis.add((surf_index / 32) as usize) |= 1u32 << (surf_index % 32);
                        continue;
                    }
                    let surf = (*wm)
                        .surfaces
                        .add(*(*leaf).firstmarksurface.add(j) as usize);
                    if (*surf).visframe != r_visframecount {
                        (*surf).visframe = r_visframecount;
                        if !backface_cull(surf) {
                            brushpolys += 1;
                            R_ChainSurface(surf, CHAIN_WORLD);
                            note_surface_lightmap_and_warp(surf, 0, cst_r_gpulightmapupdate);
                        }
                    }
                }
            }
            if !(*leaf).efrags.is_null()
                && store_efrags(ptr::addr_of_mut!((*leaf).efrags).cast::<*mut Efrag>())
            {
                return;
            }
        }

        if cst_indirect {
            return;
        }
        add_stat(ptr::addr_of_mut!(c::render::rs_brushpolys), brushpolys);
        setup_world_cbx_tex_ranges(use_tasks);
    }
}

unsafe extern "C" fn mark_vis_surfaces_task(payload: *mut c_void) {
    // SAFETY: the payload is the copied `qboolean use_tasks`.
    unsafe {
        mark_vis_surfaces(*payload.cast::<bool>());
        abort_on_pending_raise();
    }
}

unsafe extern "C" fn mark_vis_surfaces_simd_task(payload: *mut c_void) {
    // SAFETY: the payload is the copied `qboolean use_tasks`.
    unsafe {
        mark_vis_surfaces_simd(*payload.cast::<bool>());
        abort_on_pending_raise();
    }
}

/// `R_MarkSurfacesPrepare` -- PVS selection, chain reset, SIMD frustum setup.
unsafe extern "C" fn mark_surfaces_prepare(_unused: *mut c_void) {
    // SAFETY: main thread or the first marking task; the frustum, view leaf
    // and world model are set for this frame.
    unsafe {
        let wm = worldmodel();
        let numleafs = (*wm).numleafs as u32;
        let r_viewleaf = (*ptr::addr_of!(c::render::r_viewleaf)).cast::<MLeaf>();
        let mut nearwaterportal = false;
        for i in 0..(*r_viewleaf).nummarksurfaces as usize {
            let surf = (*wm)
                .surfaces
                .add(*(*r_viewleaf).firstmarksurface.add(i) as usize);
            if (*surf).flags & SURF_DRAWTURB != 0 {
                nearwaterportal = true;
            }
        }

        let st = &mut *ptr::addr_of_mut!(MARK_SURFACES_STATE);
        if cvar_value(ptr::addr_of!(c::render::r_novis)) != 0.0
            || (*r_viewleaf).contents == CONTENTS_SOLID
            || (*r_viewleaf).contents == CONTENTS_SKY
        {
            st.vis = c::render::Mod_NoVisPVS(wm.cast());
        } else if nearwaterportal {
            st.vis = c::render::SV_FatPVS(ptr::addr_of!(c::host::r_origin).cast(), wm.cast());
        } else {
            st.vis = c::progs_builtins_sv::Mod_LeafPVS(r_viewleaf.cast(), wm.cast());
        }

        let vis32 = st.vis.cast::<u32>();
        if !numleafs.is_multiple_of(32) {
            *vis32.add((numleafs / 32) as usize) &= (1u32 << (numleafs % 32)) - 1;
        }

        *ptr::addr_of_mut!(c::render::r_visframecount) += 1;

        for i in 0..(*wm).numtextures as usize {
            let t = *(*wm).textures.add(i);
            if !t.is_null() {
                (*t).texturechains[CHAIN_WORLD as usize] = ptr::null_mut();
                (*t).chain_size[CHAIN_WORLD as usize] = 0;
            }
        }

        let surfvis_bytes = ((*wm).numsurfaces as usize + 31) / 8;
        if use_simd() {
            ptr::write_bytes((*wm).surfvis, 0, surfvis_bytes);
            let frustum = ptr::addr_of_mut!(c::render::frustum).cast::<MPlane>();
            for frustum_index in 0..4usize {
                let p = frustum.add(frustum_index);
                let signbits = (*p).signbits;
                st.frustum_ofsx[frustum_index] = if signbits & 1 != 0 { 0 } else { 8 };
                st.frustum_ofsy[frustum_index] = if signbits & 2 != 0 { 16 } else { 24 };
                st.frustum_ofsz[frustum_index] = if signbits & 4 != 0 { 32 } else { 40 };
                st.frustum_px[frustum_index] = [(*p).normal[0]; 4];
                st.frustum_py[frustum_index] = [(*p).normal[1]; 4];
                st.frustum_pz[frustum_index] = [(*p).normal[2]; 4];
                st.frustum_pd[frustum_index] = [(*p).dist; 4];
            }
            let vieworg = (*ptr::addr_of!(r_refdef)).vieworg;
            st.vieworg_px = [vieworg[0]; 4];
            st.vieworg_py = [vieworg[1]; 4];
            st.vieworg_pz = [vieworg[2]; 4];
        } else if cvar_value(ptr::addr_of!(r_parallelmark)) != 0.0 || *ptr::addr_of!(indirect) {
            ptr::write_bytes((*wm).surfvis, 0, surfvis_bytes);
        }
    }
}

#[inline]
unsafe fn allocate_and_assign_func(
    func: unsafe extern "C" fn(*mut c_void),
    payload: *mut c_void,
    payload_size: usize,
) -> u64 {
    // SAFETY: `Task_AllocateAndAssignFunc` (`tasks.h`) is exactly these two calls.
    unsafe {
        let handle = c::tasks::Task_Allocate();
        c::tasks::Task_AssignFunc(handle, Some(func), payload, payload_size);
        handle
    }
}

#[inline]
unsafe fn allocate_and_assign_indexed_func(
    func: unsafe extern "C" fn(c_int, *mut c_void),
    limit: u32,
    payload: *mut c_void,
    payload_size: usize,
) -> u64 {
    // SAFETY: `Task_AllocateAndAssignIndexedFunc` (`tasks.h`) is exactly these two calls.
    unsafe {
        let handle = c::tasks::Task_Allocate();
        c::tasks::Task_AssignIndexedFunc(handle, Some(func), limit, payload, payload_size);
        handle
    }
}

/// `R_MarkSurfaces` -- johnfitz -- mark surfaces based on PVS and rebuild
/// texture chains. Returns the `Host_Guard` code raised by `R_StoreEfrags` on
/// the synchronous path (0 otherwise) for `r_world_glue.c` to `Host_Reraise`.
///
/// # Safety
/// Main thread inside `R_RenderView`; the out-pointers are live task handles.
#[no_mangle]
pub unsafe extern "C" fn RWorld_MarkSurfaces(
    mut use_tasks: bool,
    before_mark: u64,
    store_efrags_out: *mut u64,
    cull_surfaces: *mut u64,
    chain_surfaces: *mut u64,
) -> c_int {
    PENDING_RAISE.store(0, Ordering::SeqCst);
    // SAFETY: the caller's contract.
    unsafe {
        if use_tasks {
            let payload = ptr::addr_of_mut!(use_tasks).cast::<c_void>();
            let prepare_mark = allocate_and_assign_func(mark_surfaces_prepare, ptr::null_mut(), 0);
            c::tasks::Task_AddDependency(before_mark, prepare_mark);
            c::tasks::Task_Submit(prepare_mark);
            let num_workers = c::tasks::Tasks_NumWorkers();
            if cvar_value(ptr::addr_of!(r_parallelmark)) != 0.0 {
                let mark_surfaces = allocate_and_assign_indexed_func(
                    if use_simd() {
                        mark_leafs_simd
                    } else {
                        mark_leafs_parallel
                    },
                    (MARK_SURFACE_CALLS_PER_WORKER * num_workers) as u32,
                    ptr::null_mut(),
                    0,
                );
                c::tasks::Task_AddDependency(prepare_mark, mark_surfaces);
                c::tasks::Task_Submit(mark_surfaces);
                *store_efrags_out = allocate_and_assign_func(store_leaf_efrags, ptr::null_mut(), 0);
                c::tasks::Task_AddDependency(mark_surfaces, *store_efrags_out);
                if !*ptr::addr_of!(indirect) && cheatsafe_drawworld() {
                    *cull_surfaces = allocate_and_assign_indexed_func(
                        if use_simd() {
                            backface_cull_surfaces_simd
                        } else {
                            backface_cull_surfaces_parallel
                        },
                        (MARK_SURFACE_CALLS_PER_WORKER * num_workers) as u32,
                        ptr::null_mut(),
                        0,
                    );
                    c::tasks::Task_AddDependency(mark_surfaces, *cull_surfaces);
                    *chain_surfaces = allocate_and_assign_func(
                        chain_vis_surfaces_task,
                        payload,
                        core::mem::size_of::<bool>(),
                    );
                    c::tasks::Task_AddDependency(*cull_surfaces, *chain_surfaces);
                } else {
                    *cull_surfaces = mark_surfaces;
                    *chain_surfaces = mark_surfaces;
                }
            } else {
                let mark_surfaces = allocate_and_assign_func(
                    if use_simd() {
                        mark_vis_surfaces_simd_task
                    } else {
                        mark_vis_surfaces_task
                    },
                    payload,
                    core::mem::size_of::<bool>(),
                );
                c::tasks::Task_AddDependency(prepare_mark, mark_surfaces);
                *store_efrags_out = mark_surfaces;
                *chain_surfaces = mark_surfaces;
                *cull_surfaces = mark_surfaces;
            }
            0
        } else {
            mark_surfaces_prepare(ptr::null_mut());
            if use_simd() {
                mark_vis_surfaces_simd(use_tasks);
            } else {
                mark_vis_surfaces(use_tasks);
            }
            PENDING_RAISE.load(Ordering::SeqCst)
        }
    }
}

// ---- batching -------------------------------------------------------------

#[inline]
unsafe fn num_triangle_indices_for_surf(s: *const MSurface) -> u32 {
    // SAFETY: live surface.
    unsafe { 3 * ((*s).numedges as u32 - 2) }
}

#[inline]
unsafe fn triangle_indices_for_surf(s: *const MSurface, dest: *mut u32) {
    // SAFETY: live surface; `dest` has room for the triangle fan.
    unsafe {
        let mut dest = dest;
        let first = (*s).vbo_firstvert as u32;
        for i in 2..(*s).numedges as u32 {
            *dest = first;
            *dest.add(1) = first + i - 1;
            *dest.add(2) = first + i;
            dest = dest.add(3);
        }
    }
}

#[inline]
unsafe fn clear_batch(cbx: *mut CbContext) {
    // SAFETY: live command-buffer context.
    unsafe { (*cbx).num_vbo_indices = 0 }
}

/// Everything `R_FlushBatch` needs beyond the batch itself.
#[derive(Clone, Copy)]
struct BatchState {
    fullbright_enabled: bool,
    alpha_test: bool,
    alpha_blend: bool,
    use_zbias: bool,
    lightmap_texture: *mut GlTexture,
}

pub(crate) unsafe fn world_pipeline(
    cbx: *const CbContext,
    pipeline_index: usize,
) -> VulkanPipeline {
    // SAFETY: rendering path, device is up.
    let idx = unsafe { (*cbx).render_pass_index };
    let variant = cb::main_pass_pipeline_variant(idx);
    with_ctx(|ctx| {
        cb::pipeline_for_render_pass(
            idx,
            vg!(ctx, world_pipelines[variant][pipeline_index]),
            vg!(ctx, world_wboit_pipelines[pipeline_index]),
            vg!(ctx, world_mboit_moment_pipelines[pipeline_index]),
            vg!(ctx, world_mboit_composite_pipelines[pipeline_index]),
        )
    })
}

/// `R_FlushBatch` -- draws the current batch if it isn't empty.
unsafe fn flush_batch(
    procs: &CmdProcs,
    cbx: *mut CbContext,
    b: &BatchState,
    brushpasses: &mut u32,
) {
    // SAFETY: rendering path inside the frame; `cbx` is recording.
    unsafe {
        let num_vbo_indices = (*cbx).num_vbo_indices;
        if num_vbo_indices == 0 {
            return;
        }
        let palettized = cvar_value(ptr::addr_of!(c::menu::vid_filter)) != 0.0
            && cvar_value(ptr::addr_of!(c::menu::vid_palettize)) != 0.0;
        let pipeline_index = usize::from(b.fullbright_enabled)
            + if b.alpha_test { 2 } else { 0 }
            + if b.alpha_blend { 4 } else { 0 }
            + if palettized { 8 } else { 0 };
        let pipeline = world_pipeline(cbx, pipeline_index);
        cb::bind_pipeline(procs, &mut *cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);

        let device = device();
        let cmd = (*cbx).cb;
        let (mut constant_factor, mut slope_factor) = (0.0f32, 0.0f32);
        if b.use_zbias {
            let depth_format = with_ctx(|ctx| (*ctx.vg.as_ptr()).depth_format);
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
        device.cmd_set_depth_bias(cmd, constant_factor, 0.0, slope_factor);

        let (layout, grey_lightmap_set) = with_ctx(|ctx| {
            (
                (*ctx.vg.as_ptr()).world_pipeline_layout.handle,
                (*(*ptr::addr_of!(c::render::greylightmap)).cast::<GlTexture>()).descriptor_set,
            )
        });
        let lightmap_set = if cheatsafe_fullbright() {
            grey_lightmap_set
        } else {
            (*b.lightmap_texture).descriptor_set
        };
        device.cmd_bind_descriptor_sets(
            cmd,
            vk::PipelineBindPoint::GRAPHICS,
            layout,
            1,
            &[lightmap_set],
            &[],
        );

        let a = with_ctx(|ctx| DYN.index_allocate(ctx, num_vbo_indices * 4));
        ptr::copy_nonoverlapping(
            (*cbx).vbo_indices.as_ptr(),
            a.data.cast::<u32>(),
            num_vbo_indices as usize,
        );
        device.cmd_bind_index_buffer(cmd, a.buffer, a.buffer_offset, vk::IndexType::UINT32);
        cb::draw_indexed(procs, cmd, num_vbo_indices, 1, 0, 0, 0);
        clear_batch(cbx);
        *brushpasses += 1;
    }
}

/// `R_BatchSurface` -- adds the surface's triangles to the batch, flushing
/// first if it would overflow.
unsafe fn batch_surface(
    procs: &CmdProcs,
    cbx: *mut CbContext,
    s: *const MSurface,
    b: &BatchState,
    brushpasses: &mut u32,
) {
    // SAFETY: rendering path; live surface.
    unsafe {
        let num_surf_indices = num_triangle_indices_for_surf(s);
        if (*cbx).num_vbo_indices + num_surf_indices > MAX_BATCH_SIZE as u32 {
            flush_batch(procs, cbx, b, brushpasses);
        }
        let dest = (*cbx)
            .vbo_indices
            .as_mut_ptr()
            .add((*cbx).num_vbo_indices as usize);
        triangle_indices_for_surf(s, dest);
        (*cbx).num_vbo_indices += num_surf_indices;
    }
}

/// `GL_WaterAlphaForEntityTextureType`.
///
/// # Safety
/// `ent` is null or a live entity.
#[no_mangle]
pub unsafe extern "C" fn GL_WaterAlphaForEntityTextureType(
    ent: *mut Entity,
    type_: c_int,
) -> c_float {
    // SAFETY: the caller's contract.
    unsafe {
        if cheatsafe_lightmap() {
            1.0
        } else if ent.is_null() || (*ent).alpha == ENTALPHA_DEFAULT {
            c::render::GL_WaterAlphaForTextureType(type_)
        } else {
            entalpha_decode((*ent).alpha)
        }
    }
}

#[inline]
fn entalpha_decode(a: u8) -> f32 {
    if a == 0 {
        1.0
    } else {
        f32::from(a - 1) / 254.0
    }
}

/// `R_DrawTextureChains_ShowTris` -- johnfitz
unsafe fn draw_texture_chains_showtris(cbx: *mut CbContext, model: *mut QModel, chain: c_int) {
    // SAFETY: rendering path; loaded model.
    unsafe {
        let mut color = [1.0f32, 1.0, 1.0];
        for i in 0..(*model).numtextures as usize {
            let t = *(*model).textures.add(i);
            if t.is_null() {
                continue;
            }
            let mut s = (*t).texturechains[chain as usize];
            while !s.is_null() {
                DrawGLPoly(cbx, (*s).polys.cast(), color.as_mut_ptr(), 1.0);
                s = (*s).texturechains[chain as usize];
            }
        }
    }
}

/// Shared prologue of the two chain draws: vertex buffer, the null texture in
/// set 2, the grey texture in set 0 under `r_lightmap`, the bmodel instance
/// set in set 4 and a zero instance base. Returns the pipeline layout handle.
unsafe fn bind_world_common(procs: &CmdProcs, cbx: *mut CbContext) -> vk::PipelineLayout {
    // SAFETY: rendering path; device is up.
    unsafe {
        let device = device();
        let cmd = (*cbx).cb;
        let (layout, null_set, grey_set, bmodel_set) = with_ctx(|ctx| {
            (
                (*ctx.vg.as_ptr()).world_pipeline_layout.handle,
                (*(*ptr::addr_of!(c::render::nulltexture)).cast::<GlTexture>()).descriptor_set,
                (*(*ptr::addr_of!(c::render::greytexture)).cast::<GlTexture>()).descriptor_set,
                (*ctx.vg.as_ptr()).bmodel_instances_desc_set,
            )
        });
        device.cmd_bind_vertex_buffers(cmd, 0, &[*ptr::addr_of!(bmodel_vertex_buffer)], &[0]);
        device.cmd_bind_descriptor_sets(
            cmd,
            vk::PipelineBindPoint::GRAPHICS,
            layout,
            2,
            &[null_set],
            &[],
        );
        if cheatsafe_lightmap() {
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                layout,
                0,
                &[grey_set],
                &[],
            );
        }
        device.cmd_bind_descriptor_sets(
            cmd,
            vk::PipelineBindPoint::GRAPHICS,
            layout,
            4,
            &[bmodel_set],
            &[],
        );
        let instance_base = 0u32;
        cb::push_constants(
            procs,
            &*cbx,
            vk::ShaderStageFlags::ALL_GRAPHICS,
            21 * 4,
            &instance_base.to_ne_bytes(),
        );
        layout
    }
}

/// `R_DrawTextureChains_Water` -- johnfitz
///
/// # Safety
/// Rendering path; `model` is loaded; `ent` is null or live.
#[no_mangle]
pub unsafe extern "C" fn R_DrawTextureChains_Water(
    cbx: *mut CbContext,
    model: *mut QModel,
    ent: *mut Entity,
    chain: c_int,
    opaque_only: bool,
    transparent_only: bool,
) {
    // SAFETY: the caller's contract.
    unsafe {
        let procs = with_ctx(|ctx| CmdProcs::new(ctx.vg));
        let layout = bind_world_common(&procs, cbx);
        let device = device();
        let cmd = (*cbx).cb;
        let chain_idx = chain as usize;
        let mut brushpasses = 0u32;
        let wm = worldmodel();
        let lightmap_cheatsafe = cheatsafe_lightmap();
        let grey_lightmap = (*ptr::addr_of!(c::render::greylightmap)).cast::<GlTexture>();

        for type_ in TEXTYPE_FIRSTLIQUID..=TEXTYPE_LASTLIQUID {
            let alpha = GL_WaterAlphaForEntityTextureType(ent, type_);
            let alpha_blend = alpha < 1.0;
            if opaque_only && alpha_blend {
                continue;
            }
            if transparent_only && !alpha_blend {
                continue;
            }
            let mut b = BatchState {
                fullbright_enabled: false,
                alpha_test: false,
                alpha_blend,
                use_zbias: false,
                lightmap_texture: ptr::null_mut(),
            };
            for i in (*model).texofs[type_ as usize]..(*model).texofs[type_ as usize + 1] {
                let t = *(*model)
                    .textures
                    .add(*(*model).usedtextures.add(i as usize) as usize);
                if t.is_null() || (*t).texturechains[chain_idx].is_null() {
                    continue;
                }
                b.lightmap_texture = ptr::null_mut();
                clear_batch(cbx);
                let mut lastlightmap: c_int = -2;
                let gl_texture = (*t).warpimage.cast::<GlTexture>();
                if !lightmap_cheatsafe {
                    device.cmd_bind_descriptor_sets(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        layout,
                        0,
                        &[(*gl_texture).descriptor_set],
                        &[],
                    );
                }
                if model != wm {
                    atomic_u32(ptr::addr_of_mut!((*t).update_warp)).store(1, Ordering::SeqCst);
                }
                let mut s = (*t).texturechains[chain_idx];
                while !s.is_null() {
                    if (*s).lightmaptexturenum != lastlightmap {
                        if alpha_blend {
                            cb::push_constants(
                                &procs,
                                &*cbx,
                                vk::ShaderStageFlags::ALL_GRAPHICS,
                                20 * 4,
                                &alpha.to_ne_bytes(),
                            );
                        }
                        flush_batch(&procs, cbx, &b, &mut brushpasses);
                        b.lightmap_texture = if (*s).lightmaptexturenum >= 0 {
                            (*lightmaps.add((*s).lightmaptexturenum as usize)).texture
                        } else {
                            grey_lightmap
                        };
                        lastlightmap = (*s).lightmaptexturenum;
                    }
                    batch_surface(&procs, cbx, s, &b, &mut brushpasses);
                    s = (*s).texturechains[chain_idx];
                }
                if alpha_blend {
                    cb::push_constants(
                        &procs,
                        &*cbx,
                        vk::ShaderStageFlags::ALL_GRAPHICS,
                        20 * 4,
                        &alpha.to_ne_bytes(),
                    );
                }
                flush_batch(&procs, cbx, &b, &mut brushpasses);
            }
        }
        add_stat(ptr::addr_of_mut!(c::render::rs_brushpasses), brushpasses);
    }
}

/// `R_DrawTextureChains_Multitexture` -- ericw
unsafe fn draw_texture_chains_multitexture(
    cbx: *mut CbContext,
    model: *mut QModel,
    ent: *mut Entity,
    chain: c_int,
    alpha: f32,
    texstart: c_int,
    texend: c_int,
) {
    // SAFETY: rendering path; loaded model; `ent` null or live.
    unsafe {
        let procs = with_ctx(|ctx| CmdProcs::new(ctx.vg));
        let layout = bind_world_common(&procs, cbx);
        let device = device();
        let cmd = (*cbx).cb;
        let chain_idx = chain as usize;
        let wm = worldmodel();
        let lightmap_cheatsafe = cheatsafe_lightmap();
        let alpha_blend = alpha < 1.0;
        let use_zbias = cvar_value(ptr::addr_of!(c::render::gl_zfix)) != 0.0 && model != wm;
        let ent_frame = if ent.is_null() { 0 } else { (*ent).frame };
        let gl_fullbrights = cvar_value(ptr::addr_of!(c::render::gl_fullbrights)) != 0.0;

        if alpha_blend {
            cb::push_constants(
                &procs,
                &*cbx,
                vk::ShaderStageFlags::ALL_GRAPHICS,
                20 * 4,
                &alpha.to_ne_bytes(),
            );
        }

        let mut brushpasses = 0u32;
        for i in texstart..texend {
            let t = *(*model)
                .textures
                .add(*(*model).usedtextures.add(i as usize) as usize);
            if t.is_null()
                || (*t).texturechains[chain_idx].is_null()
                || (*(*t).texturechains[chain_idx]).flags & SURF_DRAWTILED != 0
            {
                continue;
            }
            let mut b = BatchState {
                fullbright_enabled: false,
                alpha_test: false,
                alpha_blend,
                use_zbias,
                lightmap_texture: ptr::null_mut(),
            };
            let fullbright = if gl_fullbrights {
                (*R_TextureAnimation(t, ent_frame))
                    .fullbright
                    .cast::<GlTexture>()
            } else {
                ptr::null_mut()
            };
            if gl_fullbrights && !fullbright.is_null() && !lightmap_cheatsafe {
                b.fullbright_enabled = true;
                device.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    layout,
                    2,
                    &[(*fullbright).descriptor_set],
                    &[],
                );
            }
            clear_batch(cbx);
            let mut lastlightmap: c_int = -1;
            b.alpha_test = (*t).type_ == TEXTYPE_CUTOUT;
            let texture = R_TextureAnimation(t, ent_frame);
            let gl_texture = (*texture).gltexture.cast::<GlTexture>();
            if !lightmap_cheatsafe {
                device.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    layout,
                    0,
                    &[(*gl_texture).descriptor_set],
                    &[],
                );
            }
            let mut s = (*t).texturechains[chain_idx];
            while !s.is_null() {
                if (*s).lightmaptexturenum != lastlightmap {
                    flush_batch(&procs, cbx, &b, &mut brushpasses);
                    b.lightmap_texture = (*lightmaps.add((*s).lightmaptexturenum as usize)).texture;
                }
                lastlightmap = (*s).lightmaptexturenum;
                batch_surface(&procs, cbx, s, &b, &mut brushpasses);
                s = (*s).texturechains[chain_idx];
            }
            flush_batch(&procs, cbx, &b, &mut brushpasses);
        }
        add_stat(ptr::addr_of_mut!(c::render::rs_brushpasses), brushpasses);
    }
}

/// `R_DrawTextureChains` -- ericw
///
/// # Safety
/// Rendering path; `model` is loaded; `ent` is null or live.
#[no_mangle]
pub unsafe extern "C" fn R_DrawTextureChains(
    cbx: *mut CbContext,
    model: *mut QModel,
    ent: *mut Entity,
    chain: c_int,
) {
    // SAFETY: the caller's contract.
    unsafe {
        let entalpha = if ent.is_null() {
            1.0
        } else {
            entalpha_decode((*ent).alpha)
        };
        if cvar_value(ptr::addr_of!(c::render::r_gpulightmapupdate)) == 0.0 {
            R_UploadLightmaps();
        }
        draw_texture_chains_multitexture(
            cbx,
            model,
            ent,
            chain,
            entalpha,
            0,
            (*model).texofs[TEXTYPE_SKY as usize],
        );
    }
}

unsafe fn begin_label(cbx: *mut CbContext, name: &core::ffi::CStr) {
    with_ctx(|ctx| {
        let procs = CmdProcs::new(ctx.vg);
        // SAFETY: `cbx` is a live, recording command-buffer context.
        unsafe { cb::begin_debug_utils_label(&procs, &*cbx, name) };
    });
}

unsafe fn end_label(cbx: *mut CbContext) {
    with_ctx(|ctx| {
        let procs = CmdProcs::new(ctx.vg);
        // SAFETY: `cbx` is a live, recording command-buffer context.
        unsafe { cb::end_debug_utils_label(&procs, &*cbx) };
    });
}

/// `R_DrawWorld` -- johnfitz -- rewritten
///
/// # Safety
/// Rendering path; `index` selects one of the `NUM_WORLD_CBX` texture ranges.
#[no_mangle]
pub unsafe extern "C" fn R_DrawWorld(cbx: *mut CbContext, index: c_int) {
    if !cheatsafe_drawworld() {
        return;
    }
    // SAFETY: the caller's contract.
    unsafe {
        begin_label(cbx, c"World");
        if cvar_value(ptr::addr_of!(c::render::r_gpulightmapupdate)) == 0.0 {
            R_UploadLightmaps();
        }
        let idx = index as usize;
        draw_texture_chains_multitexture(
            cbx,
            worldmodel(),
            ptr::null_mut(),
            CHAIN_WORLD,
            1.0,
            (*ptr::addr_of!(WORLD_TEXSTART))[idx],
            (*ptr::addr_of!(WORLD_TEXEND))[idx],
        );
        end_label(cbx);
    }
}

/// `R_DrawWorld_Water` -- ericw
///
/// # Safety
/// Rendering path.
#[no_mangle]
pub unsafe extern "C" fn R_DrawWorld_Water(cbx: *mut CbContext, transparent: bool) {
    if !cheatsafe_drawworld() {
        return;
    }
    // SAFETY: the caller's contract.
    unsafe {
        begin_label(
            cbx,
            if transparent {
                c"Transparent World Water"
            } else {
                c"Opaque World Water"
            },
        );
        if *ptr::addr_of!(indirect) {
            if !transparent || c::render::R_UseIndirectTransparentWater() {
                R_DrawIndirectBrushes(cbx, true, transparent, false, -1);
            } else {
                chain_vis_surfaces_transparent_water();
                R_DrawTextureChains_Water(
                    cbx,
                    worldmodel(),
                    ptr::null_mut(),
                    CHAIN_WORLD,
                    false,
                    true,
                );
            }
        } else {
            R_DrawTextureChains_Water(
                cbx,
                worldmodel(),
                ptr::null_mut(),
                CHAIN_WORLD,
                !transparent,
                transparent,
            );
        }
        end_label(cbx);
    }
}

/// `R_DrawWorld_ShowTris` -- johnfitz
///
/// # Safety
/// Rendering path.
#[no_mangle]
pub unsafe extern "C" fn R_DrawWorld_ShowTris(cbx: *mut CbContext) {
    // SAFETY: the caller's contract.
    unsafe {
        let variant = cb::main_pass_pipeline_variant((*cbx).render_pass_index);
        let showtris_one = cvar_value(ptr::addr_of!(c::render::r_showtris)) == 1.0;
        let (pipeline, fan_index_buffer) = with_ctx(|ctx| {
            (
                if showtris_one {
                    (*ctx.vg.as_ptr()).showtris_pipeline[variant]
                } else {
                    (*ctx.vg.as_ptr()).showtris_depth_test_pipeline[variant]
                },
                (*ctx.vg.as_ptr()).fan_index_buffer,
            )
        });
        let procs = with_ctx(|ctx| CmdProcs::new(ctx.vg));
        cb::bind_pipeline(&procs, &mut *cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
        device().cmd_bind_index_buffer((*cbx).cb, fan_index_buffer, 0, vk::IndexType::UINT16);
        if !cheatsafe_drawworld() {
            return;
        }
        draw_texture_chains_showtris(cbx, worldmodel(), CHAIN_WORLD);
    }
}
