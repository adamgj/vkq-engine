//! `gl_rmain.c` -- frame graph and view setup (Rust migration Phase 8 M9).
//!
//! `R_RenderView`'s task graph (the same tasks, edges and `-renderhash`
//! graph-shape digest as the C original), the frustum/view-matrix setup, the
//! entity list draws and the `r_showtris` overlay. The C that stays in
//! `Quake/gl_rmain_glue.c`: the globals the rest of the engine reads by name,
//! `R_PrintStats` (ADR-005 `%g` output), the `r_showbboxes` edict walk and the
//! `Host_Error`-capable model draws. Those draws run under `Host_Guard` on
//! the main thread and their code is returned from [`RRmain_RenderView`] for
//! `R_RenderView` to `Host_Reraise` (ADR-009); on a task worker the glue
//! thunks call the draw directly, which is the C original's cross-thread
//! `longjmp` (plan I2 / the M9 amendment).

use core::ffi::{c_char, c_int, c_void, CStr};
use core::ptr;
use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use ash::vk;
use quake_c_sys as c;
use quake_c_sys::cvar_t;
use quake_c_sys::libm;
use quake_math::mathlib::{
    angle_vectors, dot_product, matrix_multiply, rotation_matrix, scale_matrix, translation_matrix,
    vector_add, vector_ma,
};
use quake_render::cb::{self, CmdProcs};
use quake_types::host::{ClientState, Entity};
use quake_types::model_mem::{
    MLeaf, MOD_ALIAS, MOD_BRUSH, MOD_SPRITE, SURF_DRAWLAVA, SURF_DRAWSLIME, SURF_DRAWTELE,
    SURF_DRAWTURB, SURF_DRAWWATER, TEXTYPE_LAVA, TEXTYPE_SLIME, TEXTYPE_TELE, TEXTYPE_WATER,
};
use quake_types::plane::MPlane;
use quake_types::refdef::RefDef;
use quake_types::render::{
    CbContext, NUM_ENTITIES_CBX, NUM_WORLD_CBX, SCBX_ALPHA_ENTITIES,
    SCBX_ALPHA_ENTITIES_ACROSS_WATER, SCBX_ENTITIES, SCBX_FTE_PARTICLES_BLEND,
    SCBX_MBOIT_COMPOSITE_ALPHA_ENTITIES, SCBX_MBOIT_COMPOSITE_ALPHA_ENTITIES_ACROSS_WATER,
    SCBX_MBOIT_COMPOSITE_PARTICLES, SCBX_MBOIT_COMPOSITE_WATER, SCBX_PARTICLES, SCBX_SKY,
    SCBX_VIEW_MODEL, SCBX_WATER, SCBX_WORLD,
};

use crate::gl_draw::GL_Viewport;
use crate::gl_fog::Fog_EnableGFog;
use crate::gl_rlight::{R_AnimateLight, R_PushDlights};
use crate::gl_rmisc::{vg, with_ctx};
use crate::gl_sky::Sky_DrawSky;
use crate::gl_warp::R_UpdateWarpTextures;
use crate::r_brush::{
    entalpha_decode, entscale_decode, indirect, indirect_ready, R_ClearBModelInstanceClaims,
    R_DrawBrushModel, R_DrawBrushModel_ShowTris, R_DrawIndirectBrushes,
    R_DrawIndirectBrushes_ShowTris, R_UpdateLightmapsAndIndirect,
};
use crate::r_world::{RWorld_MarkSurfaces, R_DrawWorld, R_DrawWorld_ShowTris, R_DrawWorld_Water};

extern "C" {
    /// `client_state_t cl` (ADR-007 row closed in Phase 7).
    static mut cl: ClientState;
    /// `gl_rmain_glue.c` -- `refdef_t r_refdef`.
    static mut r_refdef: RefDef;
    /// `gl_rmain_glue.c` -- `mplane_t frustum[4]`.
    static mut frustum: [MPlane; 4];
    /// `gl_rmain_glue.c` -- `mleaf_t *r_viewleaf, *r_oldviewleaf`.
    static mut r_viewleaf: *mut MLeaf;
    static mut r_oldviewleaf: *mut MLeaf;
    /// `gl_rmain_glue.c` -- `float r_fovx, r_fovy`.
    static mut r_fovx: f32;
    static mut r_fovy: f32;
    /// `gl_rmain_glue.c` -- the `rs_*` counters `quake-c-sys` does not carry.
    static mut rs_aliaspolys: u32;
    static mut rs_aliaspasses: u32;
    static mut rs_fogpolys: u32;
    static mut rs_frame_starttime: f64;
    /// `gl_rmain_glue.c` cvars read only here.
    static mut r_drawworld: cvar_t;
    static mut r_pos: cvar_t;
    static mut r_lightmap: cvar_t;
    static mut r_indirect: cvar_t;
    static mut scr_speeds: cvar_t;
    /// `gl_rmain_glue.c` -- the `r_showbboxes` edict walk (`R_ShowBoundingBoxes`).
    fn RRmain_Glue_ShowBoundingBoxes(cbx: *mut CbContext) -> c_int;
    /// `gl_rmain_glue.c` -- `Host_Guard` thunks over the `r_alias.c` /
    /// `r_sprite.c` draws (main thread) or the draws themselves (workers).
    fn RRmain_Glue_DrawAliasModel(
        cbx: *mut CbContext,
        e: *mut Entity,
        aliaspolys: *mut c_int,
    ) -> c_int;
    fn RRmain_Glue_DrawAliasModel_ShowTris(cbx: *mut CbContext, e: *mut Entity) -> c_int;
    fn RRmain_Glue_DrawSpriteModel(cbx: *mut CbContext, e: *mut Entity) -> c_int;
    fn RRmain_Glue_DrawSpriteModel_ShowTris(cbx: *mut CbContext, e: *mut Entity) -> c_int;
}

const INVALID_TASK_HANDLE: u64 = u64::MAX;
const CONTENTS_WATER: c_int = -3;
const CONTENTS_SLIME: c_int = -4;
const CONTENTS_LAVA: c_int = -5;
const STAT_HEALTH: usize = 0;
const IT_INVISIBILITY: c_int = 524_288;
const EFLAGS_EXTERIORMODEL: u8 = 8;
const ENTALPHA_DEFAULT: u8 = 0;
/// `texchain_t` (`gl_model.h`).
const CHAIN_MODEL_0: c_int = 1;
const CHAIN_ALPHA_MODEL_ACROSS_WATER: c_int = 7;
const CHAIN_ALPHA_MODEL: c_int = 8;
const PLANE_ANYZ: u8 = 5;
const NEARCLIP: f32 = 4.0;
const M_PI: f64 = core::f64::consts::PI;
const M_PI_DIV_180: f64 = M_PI / 180.0;

/// `gl_rmain.c` -- `static atomic_uint32_t next_visedict` (Rust-owned).
static NEXT_VISEDICT: AtomicU32 = AtomicU32::new(0);
/// `R_RenderView`'s `static qboolean stats_ready`.
static STATS_READY: AtomicI32 = AtomicI32::new(0);
/// First non-OK `Host_Guard` code raised by a glue draw thunk during the
/// current `R_RenderView` (see the module doc).
static PENDING_RAISE: AtomicI32 = AtomicI32::new(0);

// ---- small helpers --------------------------------------------------------

#[inline]
fn cvar_value(cvar: *const cvar_t) -> f32 {
    // SAFETY: the C cvar statics live for the program.
    unsafe { (*cvar).value }
}

#[inline]
fn raised() -> bool {
    PENDING_RAISE.load(Ordering::SeqCst) != 0
}

/// Records a glue thunk's `Host_Guard` code; returns `true` when it raised.
#[inline]
#[must_use]
fn note(code: c_int) -> bool {
    if code != 0 {
        let _ = PENDING_RAISE.compare_exchange(0, code, Ordering::SeqCst, Ordering::SeqCst);
    }
    code != 0
}

#[inline]
unsafe fn atomic_u32<'a>(p: *mut u32) -> &'a AtomicU32 {
    // SAFETY: the caller passes a 4-byte-aligned, live `atomic_uint32_t`.
    unsafe { AtomicU32::from_ptr(p) }
}

/// `R_UseOIT` / `R_UseMBOIT` (`glquake.h` static inlines).
#[inline]
fn r_use_oit() -> bool {
    // SAFETY: `frame_oit_mode` is a plain int written once per frame.
    unsafe { *ptr::addr_of!(c::render::frame_oit_mode) != 0 }
}

#[inline]
fn r_use_mboit() -> bool {
    // SAFETY: as above.
    unsafe { *ptr::addr_of!(c::render::frame_oit_mode) == 2 }
}

#[inline]
fn r_use_alpha_sort() -> bool {
    // SAFETY: pure C function over cvars.
    unsafe { c::render::R_UseAlphaSort() }
}

#[inline]
fn is_indirect() -> bool {
    // SAFETY: Rust-owned bool written at the top of `R_RenderView`.
    unsafe { *ptr::addr_of!(indirect) }
}

#[inline]
fn glheight() -> c_int {
    // SAFETY: plain int written by `GL_BeginRendering`.
    unsafe { *ptr::addr_of!(c::menu::glheight) }
}

#[inline]
unsafe fn secondary_cbx(index: usize) -> *mut CbContext {
    // SAFETY: `vulkan_globals` is initialised before any frame is rendered.
    with_ctx(|ctx| vg!(ctx, secondary_cb_contexts[index]))
}

#[inline]
unsafe fn cl_entity(i: c_int) -> *mut Entity {
    // SAFETY: `cl.entities` holds `cl.max_edicts` entities for the frame.
    unsafe {
        (*ptr::addr_of!(cl))
            .entities
            .offset(i as isize)
            .cast::<Entity>()
    }
}

#[inline]
unsafe fn cl_viewent() -> *mut Entity {
    // SAFETY: `cl.viewent` is the opaque mirror of an `entity_t`.
    unsafe { ptr::addr_of_mut!((*ptr::addr_of_mut!(cl)).viewent).cast::<Entity>() }
}

unsafe fn begin_label(cbx: *mut CbContext, name: &CStr) {
    // SAFETY: `cbx` is a live, recording command-buffer context.
    unsafe {
        with_ctx(|ctx| {
            let procs = CmdProcs::new(ctx.vg);
            cb::begin_debug_utils_label(&procs, &*cbx, name);
        });
    }
}

unsafe fn end_label(cbx: *mut CbContext) {
    // SAFETY: `cbx` is a live, recording command-buffer context.
    unsafe {
        with_ctx(|ctx| {
            let procs = CmdProcs::new(ctx.vg);
            cb::end_debug_utils_label(&procs, &*cbx);
        });
    }
}

// ---- culling / transforms -------------------------------------------------

/// `R_CullBox` -- johnfitz -- replaced with new function from lordhavoc.
/// Returns true if the box is completely outside the frustum.
///
/// # Safety
/// `emins`/`emaxs` point at three floats each.
#[no_mangle]
pub unsafe extern "C" fn R_CullBox(emins: *const f32, emaxs: *const f32) -> bool {
    // SAFETY: per the contract; `frustum` is written on the main thread
    // before the frame's tasks run.
    unsafe {
        for p in (*ptr::addr_of!(frustum)).iter() {
            let signbits = p.signbits;
            let pick = |bit: u8, i: usize| {
                if signbits & bit != 0 {
                    *emins.add(i)
                } else {
                    *emaxs.add(i)
                }
            };
            let vec = [pick(1, 0), pick(2, 1), pick(4, 2)];
            if p.normal[0] * vec[0] + p.normal[1] * vec[1] + p.normal[2] * vec[2] < p.dist {
                return true;
            }
        }
        false
    }
}

/// `R_CullModelForEntity` -- johnfitz -- uses correct bounds based on rotation.
///
/// # Safety
/// `e` carries a loaded model.
#[no_mangle]
pub unsafe extern "C" fn R_CullModelForEntity(e: *mut Entity) -> bool {
    // SAFETY: per the contract.
    unsafe {
        let model = (*e).model;
        let (minbounds, maxbounds) = if (*e).angles[0] != 0.0 || (*e).angles[2] != 0.0 {
            ((*model).rmins, (*model).rmaxs)
        } else if (*e).angles[1] != 0.0 {
            ((*model).ymins, (*model).ymaxs)
        } else {
            ((*model).mins, (*model).maxs)
        };

        let mut mins = [0.0f32; 3];
        let mut maxs = [0.0f32; 3];
        let scalefactor = entscale_decode((*e).netstate.scale);
        if scalefactor != 1.0 {
            vector_ma(&(*e).origin, scalefactor, &minbounds, &mut mins);
            vector_ma(&(*e).origin, scalefactor, &maxbounds, &mut maxs);
        } else {
            vector_add(&(*e).origin, &minbounds, &mut mins);
            vector_add(&(*e).origin, &maxbounds, &mut maxs);
        }

        let culled = R_CullBox(mins.as_ptr(), maxs.as_ptr());
        if *ptr::addr_of!(c::render::harness_renderhash) {
            c::render::Harness_RenderCull(e.cast::<c_void>().cast_const(), culled);
        }
        culled
    }
}

/// `R_RotateForEntity` -- johnfitz -- modified to take origin and angles
/// instead of pointer to entity.
///
/// # Safety
/// `matrix` points at 16 floats; `origin`/`angles` at three each.
#[no_mangle]
pub unsafe extern "C" fn R_RotateForEntity(
    matrix: *mut f32,
    origin: *const f32,
    angles: *const f32,
    scale: u8,
) {
    // SAFETY: per the contract.
    unsafe {
        let matrix = &mut *matrix.cast::<[f32; 16]>();
        let origin = &*origin.cast::<[f32; 3]>();
        let angles = &*angles.cast::<[f32; 3]>();
        let mut tmp = [0.0f32; 16];
        translation_matrix(&mut tmp, origin[0], origin[1], origin[2]);
        matrix_multiply(matrix, &tmp);

        rotation_matrix(&mut tmp, deg2rad(angles[1]), 0.0, 0.0, 1.0);
        matrix_multiply(matrix, &tmp);
        rotation_matrix(&mut tmp, deg2rad(-angles[0]), 0.0, 1.0, 0.0);
        matrix_multiply(matrix, &tmp);
        rotation_matrix(&mut tmp, deg2rad(angles[2]), 1.0, 0.0, 0.0);
        matrix_multiply(matrix, &tmp);

        let scalefactor = entscale_decode(scale);
        if scalefactor != 1.0 {
            scale_matrix(&mut tmp, scalefactor, scalefactor, scalefactor);
            matrix_multiply(matrix, &tmp);
        }
    }
}

/// `DEG2RAD(a)` -- `a * M_PI_DIV_180` evaluated in double, then narrowed by
/// the `float` parameter it feeds (ADR-010).
#[inline]
fn deg2rad(a: f32) -> f32 {
    (f64::from(a) * M_PI_DIV_180) as f32
}

// ---- SETUP FRAME ------------------------------------------------------------

/// `SignbitsForPlane` -- for fast box on planeside test.
fn signbits_for_plane(out: &MPlane) -> u8 {
    let mut bits = 0u8;
    for (j, n) in out.normal.iter().enumerate() {
        if *n < 0.0 {
            bits |= 1 << j;
        }
    }
    bits
}

/// `TurnVector` -- johnfitz -- turn forward towards side on the plane defined
/// by forward and side. If angle = 90, the result will be equal to side.
/// Assumes side and forward are perpendicular, and normalized. To turn away
/// from side, use a negative angle.
fn turn_vector(out: &mut [f32; 3], forward: &[f32; 3], side: &[f32; 3], angle: f32) {
    // COMPAT: `cos`/`sin` on the double `DEG2RAD` product, narrowed to float
    // (ADR-010; the C original stores them in `float` locals).
    let scale_forward = libm::cos(f64::from(angle) * M_PI_DIV_180) as f32;
    let scale_side = libm::sin(f64::from(angle) * M_PI_DIV_180) as f32;
    out[0] = scale_forward * forward[0] + scale_side * side[0];
    out[1] = scale_forward * forward[1] + scale_side * side[1];
    out[2] = scale_forward * forward[2] + scale_side * side[2];
}

/// `R_SetFrustum` -- johnfitz -- rewritten.
unsafe fn r_set_frustum(fovx: f32, fovy: f32) {
    // SAFETY: main thread before the frame's tasks; the view vectors were set
    // by `R_SetupViewBeforeMark`.
    unsafe {
        let vpn = *ptr::addr_of!(c::cl_main::vpn);
        let vright = *ptr::addr_of!(c::host::vright);
        let vup = *ptr::addr_of!(c::host::vup);
        let r_origin = *ptr::addr_of!(c::host::r_origin);
        let planes = &mut *ptr::addr_of_mut!(frustum);
        turn_vector(&mut planes[0].normal, &vpn, &vright, fovx / 2.0 - 90.0); // right plane
        turn_vector(&mut planes[1].normal, &vpn, &vright, 90.0 - fovx / 2.0); // left plane
        turn_vector(&mut planes[2].normal, &vpn, &vup, 90.0 - fovy / 2.0); // bottom plane
        turn_vector(&mut planes[3].normal, &vpn, &vup, fovy / 2.0 - 90.0); // top plane

        for p in planes.iter_mut() {
            p.type_ = PLANE_ANYZ;
            p.dist = dot_product(&r_origin, &p.normal); // FIXME: shouldn't this always be zero?
            p.signbits = signbits_for_plane(p);
        }
    }
}

/// `GL_FrustumMatrix`
fn gl_frustum_matrix(matrix: &mut [f32; 16], fovx: f32, fovy: f32) {
    let w = 1.0 / libm::tanf(fovx * 0.5);
    let h = 1.0 / libm::tanf(fovy * 0.5);

    // reduce near clip distance at high FOV's to avoid seeing through walls
    let d = 12.0 * w.min(h);
    let n = d.clamp(0.5, NEARCLIP);
    let f = cvar_value(ptr::addr_of!(c::render::gl_farclip));

    *matrix = [0.0; 16];
    matrix[0] = w;
    matrix[5] = -h;
    matrix[10] = f / (f - n) - 1.0;
    matrix[11] = -1.0;
    matrix[14] = (n * f) / (f - n);
}

/// `R_SetupMatrices`
unsafe fn r_setup_matrices() {
    // SAFETY: main thread; `vulkan_globals` matrices are written before any
    // task reads them.
    unsafe {
        let (fovx, fovy) = (*ptr::addr_of!(r_fovx), *ptr::addr_of!(r_fovy));
        let viewangles = (*ptr::addr_of!(r_refdef)).viewangles;
        let vieworg = (*ptr::addr_of!(r_refdef)).vieworg;
        with_ctx(|ctx| {
            let vg = ctx.vg.as_ptr();
            gl_frustum_matrix(&mut (*vg).projection_matrix, deg2rad(fovx), deg2rad(fovy));

            let view = &mut (*vg).view_matrix;
            let mut tmp = [0.0f32; 16];
            rotation_matrix(view, (-M_PI / 2.0) as f32, 1.0, 0.0, 0.0);
            rotation_matrix(&mut tmp, (M_PI / 2.0) as f32, 0.0, 0.0, 1.0);
            matrix_multiply(view, &tmp);
            rotation_matrix(&mut tmp, deg2rad(-viewangles[2]), 1.0, 0.0, 0.0);
            matrix_multiply(view, &tmp);
            rotation_matrix(&mut tmp, deg2rad(-viewangles[0]), 0.0, 1.0, 0.0);
            matrix_multiply(view, &tmp);
            rotation_matrix(&mut tmp, deg2rad(-viewangles[1]), 0.0, 0.0, 1.0);
            matrix_multiply(view, &tmp);

            translation_matrix(&mut tmp, -vieworg[0], -vieworg[1], -vieworg[2]);
            matrix_multiply(view, &tmp);

            let projection = (*vg).projection_matrix;
            let view = (*vg).view_matrix;
            let vp = &mut (*vg).view_projection_matrix;
            *vp = projection;
            matrix_multiply(vp, &view);
        });
    }
}

/// `R_SetupContext`
unsafe fn r_setup_context(cbx: *mut CbContext) {
    // SAFETY: `cbx` is a live, recording command-buffer context.
    let render_pass_index = unsafe {
        let vrect = (*ptr::addr_of!(r_refdef)).vrect;
        GL_Viewport(
            cbx,
            vrect.x as f32,
            (glheight() - vrect.y - vrect.height) as f32,
            vrect.width as f32,
            vrect.height as f32,
            0.0,
            1.0,
        );
        (*cbx).render_pass_index as usize
    };
    with_ctx(|ctx| {
        let procs = CmdProcs::new(ctx.vg);
        let pipeline = vg!(ctx, basic_blend_pipeline[render_pass_index]);
        let vp = vg!(ctx, view_projection_matrix);
        let bytes: Vec<u8> = vp.iter().flat_map(|v| v.to_ne_bytes()).collect();
        // SAFETY: as above.
        unsafe {
            cb::bind_pipeline(&procs, &mut *cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
            cb::push_constants(&procs, &*cbx, vk::ShaderStageFlags::ALL_GRAPHICS, 0, &bytes);
        }
    });
}

/// `R_SetupViewBeforeMark`
unsafe extern "C" fn r_setup_view_before_mark(_unused: *mut c_void) {
    // SAFETY: the first task of the frame (or the serial path); `cl.worldmodel`
    // was checked non-null by `R_RenderView`.
    unsafe {
        // must happen here: in indirect mode draw_world only depends on this task, latching
        // bmodel_instances_index any later would race the read in R_DrawIndirectBrushes
        if is_indirect() {
            R_ClearBModelInstanceClaims();
        }

        // Need to do those early because we now update dynamic light maps during R_MarkSurfaces
        if cvar_value(ptr::addr_of!(c::render::r_gpulightmapupdate)) == 0.0 {
            R_PushDlights();
        }
        R_AnimateLight();

        // build the transformation matrix for the given view angles
        let refdef = &*ptr::addr_of!(r_refdef);
        *ptr::addr_of_mut!(c::host::r_origin) = refdef.vieworg;
        angle_vectors(
            &refdef.viewangles,
            &mut *ptr::addr_of_mut!(c::cl_main::vpn),
            &mut *ptr::addr_of_mut!(c::host::vright),
            &mut *ptr::addr_of_mut!(c::host::vup),
        );

        // current viewleaf
        *ptr::addr_of_mut!(r_oldviewleaf) = *ptr::addr_of!(r_viewleaf);
        let leaf = c::progs_builtins_sv::Mod_PointInLeaf(
            ptr::addr_of_mut!(c::host::r_origin).cast::<f32>(),
            (*ptr::addr_of!(cl)).worldmodel.cast::<c_void>(),
        )
        .cast::<MLeaf>();
        *ptr::addr_of_mut!(r_viewleaf) = leaf;

        c::render::V_SetContentsColor((*leaf).contents);
        c::render::V_CalcBlend();

        // johnfitz -- calculate r_fovx and r_fovy here
        let mut fovx = refdef.fov_x;
        let mut fovy = refdef.fov_y;
        *ptr::addr_of_mut!(c::view::render_warp) = false;
        *ptr::addr_of_mut!(c::view::render_scale) =
            cvar_value(ptr::addr_of!(c::menu::r_scale)) as c_int;

        let waterwarp = cvar_value(ptr::addr_of!(c::render::r_waterwarp));
        if waterwarp != 0.0 {
            let contents = (*leaf).contents;
            if contents == CONTENTS_WATER || contents == CONTENTS_SLIME || contents == CONTENTS_LAVA
            {
                if waterwarp == 1.0 {
                    *ptr::addr_of_mut!(c::view::render_warp) = true;
                } else {
                    // variance is a percentage of width, where width = 2 * tan(fov / 2) otherwise the effect is too dramatic at high FOV and too subtle at low FOV.
                    // what a mess!
                    // COMPAT: the double-precision C expression, narrowed on
                    // the float store (ADR-010).
                    let time = (*ptr::addr_of!(cl)).time;
                    let wobble = libm::sin(time * 1.5) * 0.03;
                    fovx = (libm::atan(
                        libm::tan(f64::from(refdef.fov_x) * M_PI_DIV_180 / 2.0) * (0.97 + wobble),
                    ) * 2.0
                        / M_PI_DIV_180) as f32;
                    fovy = (libm::atan(
                        libm::tan(f64::from(refdef.fov_y) * M_PI_DIV_180 / 2.0) * (1.03 - wobble),
                    ) * 2.0
                        / M_PI_DIV_180) as f32;
                }
            }
        }
        *ptr::addr_of_mut!(r_fovx) = fovx;
        *ptr::addr_of_mut!(r_fovy) = fovy;
        // johnfitz

        r_set_frustum(fovx, fovy); // johnfitz -- use r_fov* vars
        r_setup_matrices();

        // johnfitz -- cheat-protect some draw modes
        let mut fullbright = false;
        let mut lightmap = false;
        let mut drawworld = true;
        if (*ptr::addr_of!(cl)).maxclients == 1 {
            if cvar_value(ptr::addr_of!(r_drawworld)) == 0.0 {
                drawworld = false;
            }
            if cvar_value(ptr::addr_of!(r_lightmap)) != 0.0 {
                lightmap = true;
            } else if cvar_value(ptr::addr_of!(c::render::r_fullbright)) != 0.0 {
                fullbright = true;
            }
        }
        if (*(*ptr::addr_of!(cl)).worldmodel).lightdata.is_null() {
            fullbright = true;
            lightmap = false;
        }
        *ptr::addr_of_mut!(c::render::r_fullbright_cheatsafe) = fullbright;
        *ptr::addr_of_mut!(c::render::r_lightmap_cheatsafe) = lightmap;
        *ptr::addr_of_mut!(c::render::r_drawworld_cheatsafe) = drawworld;
        // johnfitz
    }
}

// ---- RENDER VIEW ------------------------------------------------------------

/// `R_IsEntityTransparent` -- `(transparent, opaque_with_transparent_water)`.
unsafe fn r_is_entity_transparent(e: *mut Entity) -> (bool, bool) {
    // SAFETY: `e` carries a loaded model.
    unsafe {
        let transparent = entalpha_decode((*e).alpha) != 1.0;
        let model = (*e).model;
        let specials = (*model).used_specials;
        let liquid = |flag: i32, textype: i32| {
            specials & flag != 0 && c::render::GL_WaterAlphaForTextureType(textype) != 1.0
        };
        let opaque_with_transparent_water = !transparent
            && (*model).type_ == MOD_BRUSH
            && specials & SURF_DRAWTURB != 0
            && (*e).alpha == ENTALPHA_DEFAULT
            && (liquid(SURF_DRAWLAVA, TEXTYPE_LAVA)
                || liquid(SURF_DRAWTELE, TEXTYPE_TELE)
                || liquid(SURF_DRAWSLIME, TEXTYPE_SLIME)
                || liquid(SURF_DRAWWATER, TEXTYPE_WATER));
        (transparent, opaque_with_transparent_water)
    }
}

/// `R_DrawEntitiesOnList` -- alphapass 0 for opaque, 1 for transparent
/// overwater, 2 for transparent underwater. Stops at the first raised model
/// draw (the C original `longjmp`ed out at that point).
unsafe fn r_draw_entities_on_list(
    cbx: *mut CbContext,
    alphapass: c_int,
    chain: c_int,
    use_tasks: bool,
) {
    // SAFETY: rendering path inside the frame; the visedict lists are stable.
    unsafe {
        if cvar_value(ptr::addr_of!(c::render::r_drawentities)) == 0.0 {
            return;
        }

        let mut brushpolys: c_int = 0;
        let mut brushpasses: u32 = 0;
        let mut aliaspolys: c_int = 0;
        let mut aliaspasses: u32 = 0;

        let overwater = *ptr::addr_of!(c::cl_main::cl_numvisedicts_alpha_overwater);
        let total = match alphapass {
            0 => *ptr::addr_of!(c::cl_main::cl_numvisedicts),
            1 => overwater,
            _ => *ptr::addr_of!(c::cl_main::cl_numvisedicts_alpha_underwater),
        };
        let alpha_list = (*ptr::addr_of!(c::cl_main::cl_visedicts_alpha)).cast::<*mut Entity>();
        let list = match alphapass {
            0 => (*ptr::addr_of!(c::cl_main::cl_visedicts)).cast::<*mut Entity>(),
            1 => alpha_list,
            _ => alpha_list.offset(overwater as isize),
        };

        begin_label(
            cbx,
            if alphapass != 0 {
                c"Entities Alpha Pass"
            } else {
                c"Entities"
            },
        );
        // johnfitz -- sprites are not a special case
        let mut i: c_int = -1;
        loop {
            if use_tasks {
                i = NEXT_VISEDICT.fetch_add(1, Ordering::SeqCst) as c_int;
            } else {
                i += 1;
            }

            if i >= total {
                break;
            }

            let currententity = *list.offset(i as isize);

            let (transparent, opaque_with_transparent_water) =
                r_is_entity_transparent(currententity);

            // johnfitz -- if alphapass is true, draw only alpha entites this time
            // if alphapass is false, draw only nonalpha entities this time
            if transparent != (alphapass != 0) && !opaque_with_transparent_water {
                continue;
            }

            // johnfitz -- chasecam
            if currententity == cl_entity((*ptr::addr_of!(cl)).viewentity) {
                (*currententity).angles[0] = (f64::from((*currententity).angles[0]) * 0.3) as f32;
            }
            // johnfitz

            // spike -- this would be more efficient elsewhere, but its more correct here.
            if (*currententity).eflags & EFLAGS_EXTERIORMODEL != 0 {
                continue;
            }

            // the sprite arm draws in its body like the alias one rather than
            // in a match guard, so the raise check reads as the side effect it is
            #[allow(clippy::collapsible_match)]
            match (*(*currententity).model).type_ {
                MOD_ALIAS => {
                    if note(RRmain_Glue_DrawAliasModel(
                        cbx,
                        currententity,
                        &mut aliaspolys,
                    )) {
                        return;
                    }
                    aliaspasses += 1;
                }
                MOD_BRUSH => {
                    R_DrawBrushModel(
                        cbx,
                        currententity,
                        chain,
                        &mut brushpolys,
                        alphapass != 0 && r_use_alpha_sort(),
                        alphapass == 0 && opaque_with_transparent_water,
                        alphapass != 0 && opaque_with_transparent_water,
                    );
                    brushpasses += 1;
                }
                MOD_SPRITE => {
                    if note(RRmain_Glue_DrawSpriteModel(cbx, currententity)) {
                        return;
                    }
                }
                _ => {}
            }
        }
        end_label(cbx);

        atomic_u32(ptr::addr_of_mut!(c::render::rs_brushpolys))
            .fetch_add(brushpolys as u32, Ordering::SeqCst);
        atomic_u32(ptr::addr_of_mut!(c::render::rs_brushpasses))
            .fetch_add(brushpasses, Ordering::SeqCst);
        atomic_u32(ptr::addr_of_mut!(rs_aliaspolys)).fetch_add(aliaspolys as u32, Ordering::SeqCst);
        atomic_u32(ptr::addr_of_mut!(rs_aliaspasses)).fetch_add(aliaspasses, Ordering::SeqCst);
    }
}

/// `R_DrawViewModel` -- johnfitz -- gutted
unsafe fn r_draw_view_model(cbx: *mut CbContext) {
    // SAFETY: rendering path inside the frame.
    unsafe {
        if cvar_value(ptr::addr_of!(c::menu::r_drawviewmodel)) == 0.0
            || cvar_value(ptr::addr_of!(c::render::r_drawentities)) == 0.0
            || cvar_value(ptr::addr_of!(c::chase::chase_active)) != 0.0
            || cvar_value(ptr::addr_of!(c::menu::scr_viewsize)) >= 130.0
        {
            return;
        }

        let clp = &*ptr::addr_of!(cl);
        if clp.items & IT_INVISIBILITY != 0 || clp.stats[STAT_HEALTH] <= 0 {
            return;
        }

        let currententity = cl_viewent();
        if (*currententity).model.is_null() {
            return;
        }

        // johnfitz -- this fixes a crash
        if (*(*currententity).model).type_ != MOD_ALIAS {
            return;
        }
        // johnfitz

        begin_label(cbx, c"View Model");

        // hack the depth range to prevent view model from poking into walls
        let vrect = (*ptr::addr_of!(r_refdef)).vrect;
        let y = (glheight() - vrect.y - vrect.height) as f32;
        GL_Viewport(
            cbx,
            vrect.x as f32,
            y,
            vrect.width as f32,
            vrect.height as f32,
            0.7,
            1.0,
        );

        let mut aliaspolys: c_int = 0;
        if note(RRmain_Glue_DrawAliasModel(
            cbx,
            currententity,
            &mut aliaspolys,
        )) {
            return;
        }
        atomic_u32(ptr::addr_of_mut!(rs_aliaspolys)).fetch_add(aliaspolys as u32, Ordering::SeqCst);
        atomic_u32(ptr::addr_of_mut!(rs_aliaspasses)).fetch_add(1, Ordering::SeqCst);

        GL_Viewport(
            cbx,
            vrect.x as f32,
            y,
            vrect.width as f32,
            vrect.height as f32,
            0.0,
            1.0,
        );

        end_label(cbx);
    }
}

/// `R_ShowTris` -- johnfitz
unsafe fn r_show_tris(cbx: *mut CbContext) {
    let non_solid_fill = with_ctx(|ctx| vg!(ctx, non_solid_fill));
    // SAFETY: rendering path inside the frame.
    unsafe {
        let showtris = cvar_value(ptr::addr_of!(c::render::r_showtris));
        if !(1.0..=2.0).contains(&showtris)
            || (*ptr::addr_of!(cl)).maxclients > 1
            || !non_solid_fill
        {
            return;
        }

        begin_label(cbx, c"show tris");
        if is_indirect() {
            R_DrawIndirectBrushes_ShowTris(cbx);
        } else if cvar_value(ptr::addr_of!(r_drawworld)) != 0.0 {
            R_DrawWorld_ShowTris(cbx);
        }

        if cvar_value(ptr::addr_of!(c::render::r_drawentities)) != 0.0 {
            let list = (*ptr::addr_of!(c::cl_main::cl_visedicts)).cast::<*mut Entity>();
            let viewentity = cl_entity((*ptr::addr_of!(cl)).viewentity);
            for i in 0..*ptr::addr_of!(c::cl_main::cl_numvisedicts) {
                let currententity = *list.offset(i as isize);

                if currententity == viewentity {
                    // chasecam
                    (*currententity).angles[0] =
                        (f64::from((*currententity).angles[0]) * 0.3) as f32;
                }

                match (*(*currententity).model).type_ {
                    MOD_BRUSH => R_DrawBrushModel_ShowTris(cbx, currententity),
                    MOD_ALIAS => {
                        if note(RRmain_Glue_DrawAliasModel_ShowTris(cbx, currententity)) {
                            return;
                        }
                    }
                    MOD_SPRITE
                        if note(RRmain_Glue_DrawSpriteModel_ShowTris(cbx, currententity)) =>
                    {
                        return
                    }
                    _ => {}
                }
            }

            // viewmodel
            let clp = &*ptr::addr_of!(cl);
            let currententity = cl_viewent();
            if cvar_value(ptr::addr_of!(c::menu::r_drawviewmodel)) != 0.0
                && cvar_value(ptr::addr_of!(c::chase::chase_active)) == 0.0
                && clp.stats[STAT_HEALTH] > 0
                && clp.items & IT_INVISIBILITY == 0
                && !(*currententity).model.is_null()
                && (*(*currententity).model).type_ == MOD_ALIAS
                && cvar_value(ptr::addr_of!(c::menu::scr_viewsize)) < 130.0
                && note(RRmain_Glue_DrawAliasModel_ShowTris(cbx, currententity))
            {
                return;
            }
        }

        if cvar_value(ptr::addr_of!(c::menu::r_particles)) != 0.0 {
            c::render::R_DrawParticles_ShowTris(cbx.cast::<c_void>());
            c::render::PScript_DrawParticles_ShowTris(cbx.cast::<c_void>());
        }

        end_label(cbx);
    }
}

// ---- task functions ---------------------------------------------------------

/// `R_DrawWorldTask`
unsafe extern "C" fn r_draw_world_task(index: c_int, use_tasks: *mut c_void) {
    // SAFETY: the world contexts were opened by `GL_BeginRendering`.
    unsafe {
        let cbx = secondary_cbx(SCBX_WORLD).offset(index as isize);
        r_setup_context(cbx);
        Fog_EnableGFog(cbx);
        if is_indirect() {
            R_DrawIndirectBrushes(
                cbx,
                false,
                false,
                false,
                if use_tasks.is_null() { -1 } else { index },
            );
        } else {
            R_DrawWorld(cbx, index);
        }
    }
}

/// `R_DrawSkyTask`
unsafe extern "C" fn r_draw_sky_task(_unused: *mut c_void) {
    // SAFETY: as above.
    unsafe {
        let cbx = secondary_cbx(SCBX_SKY);
        r_setup_context(cbx);
        Fog_EnableGFog(cbx);
        R_DrawWorld_Water(cbx, false); // draw opaque water before sky (more likely to occlude)
        Sky_DrawSky(cbx);
    }
}

/// `R_DrawWaterTask`
unsafe extern "C" fn r_draw_water_task(_unused: *mut c_void) {
    // SAFETY: as above.
    unsafe {
        let cbx = secondary_cbx(SCBX_WATER);
        r_setup_context(cbx);
        Fog_EnableGFog(cbx);
        R_DrawWorld_Water(cbx, true); // transparent worldmodel water only

        if r_use_mboit() {
            let cbx = secondary_cbx(SCBX_MBOIT_COMPOSITE_WATER);
            r_setup_context(cbx);
            Fog_EnableGFog(cbx);
            R_DrawWorld_Water(cbx, true);
        }
    }
}

#[derive(Clone, Copy, Default)]
struct TranspSort {
    visedict: c_int,
    sortkey: u32,
}

/// `R_SortAlphaEntitiesTask`
unsafe extern "C" fn r_sort_alpha_entities_task(_unused: *mut c_void) {
    // SAFETY: runs after `store_efrags`; the visedict lists are stable for the
    // frame and `cl_visedicts_alpha` has room for every visedict.
    unsafe {
        let sort_alpha = r_use_alpha_sort();
        let numvisedicts = *ptr::addr_of!(c::cl_main::cl_numvisedicts);
        let overwater = ptr::addr_of_mut!(c::cl_main::cl_numvisedicts_alpha_overwater);
        let underwater_count = ptr::addr_of_mut!(c::cl_main::cl_numvisedicts_alpha_underwater);
        *overwater = 0;
        *underwater_count = 0;
        let list = (*ptr::addr_of!(c::cl_main::cl_visedicts)).cast::<*mut Entity>();
        let alpha_list = (*ptr::addr_of!(c::cl_main::cl_visedicts_alpha)).cast::<*mut Entity>();
        // TEMP_ALLOC_COND (transp_sort, edicts, cl_numvisedicts * 2, sort_alpha)
        let mut edicts: Vec<TranspSort> = if sort_alpha {
            vec![TranspSort::default(); (numvisedicts.max(0) as usize) * 2]
        } else {
            Vec::new()
        };
        let mut sort_bins = [[0i32; 128]; 3];
        let vieworg = (*ptr::addr_of!(r_refdef)).vieworg;
        for i in 0..numvisedicts {
            let currententity = *list.offset(i as isize);

            let (transparent, opaque_with_transparent_water) =
                r_is_entity_transparent(currententity);

            if !transparent && !opaque_with_transparent_water {
                continue;
            }
            if (*currententity).eflags & EFLAGS_EXTERIORMODEL != 0 {
                continue;
            }
            // box culling here is not safe (R_DrawAliasModel updates lerp information)

            if !sort_alpha {
                *alpha_list.offset(*overwater as isize) = currententity;
                *overwater += 1;
                continue;
            }

            let mut center = [0.0f32; 3];
            let scalefactor = entscale_decode((*currententity).netstate.scale);
            let mut dist_squared = 0.0f32;
            let model = (*currententity).model;
            for j in 0..3 {
                let mins = (*currententity).origin[j] + scalefactor * (*model).mins[j];
                let maxs = (*currententity).origin[j] + scalefactor * (*model).maxs[j];
                center[j] = (mins + maxs) / 2.0;
                let dist = 0.0f32.max((mins - vieworg[j]).max(vieworg[j] - maxs));
                dist_squared += dist * dist;
            }
            // COMPAT (ADR-010): the C is a `memcmp`, so `-0.0` and `0.0`
            // differ and a NaN origin matches itself; compare the bits.
            let contents = if (*currententity).contentscache < 0
                && (*currententity)
                    .contentscache_origin
                    .iter()
                    .zip(center.iter())
                    .all(|(a, b)| a.to_bits() == b.to_bits())
            {
                (*currententity).contentscache
            } else {
                let leaf = c::progs_builtins_sv::Mod_PointInLeaf(
                    center.as_mut_ptr(),
                    (*ptr::addr_of!(cl)).worldmodel.cast::<c_void>(),
                )
                .cast::<MLeaf>();
                let contents = (*leaf).contents;
                (*currententity).contentscache = contents;
                (*currententity).contentscache_origin = center;
                contents
            };
            let underwater = contents == CONTENTS_WATER
                || contents == CONTENTS_SLIME
                || contents == CONTENTS_LAVA;
            let dist = (libm::sqrtf(dist_squared) * 2.0) as u32;
            let sortkey = (u32::from(!underwater) << 20) | dist.min((1 << 20) - 1);
            sort_bins[2][(sortkey >> 14) as usize] += 1;
            sort_bins[1][((sortkey >> 7) % 128) as usize] += 1;
            sort_bins[0][(sortkey % 128) as usize] += 1;
            let edict = &mut edicts[(*overwater + *underwater_count) as usize];
            edict.visedict = i;
            edict.sortkey = sortkey;
            if underwater {
                *underwater_count += 1;
            } else {
                *overwater += 1;
            }
        }

        if !sort_alpha {
            return;
        }

        let highest = *underwater_count + *overwater - 1;
        let (lo, hi) = edicts.split_at_mut(numvisedicts.max(0) as usize);
        for pass in 0..3usize {
            let (from, to): (&mut [TranspSort], &mut [TranspSort]) =
                if pass % 2 != 0 { (hi, lo) } else { (lo, hi) };
            for i in 1..128 {
                sort_bins[pass][i] += sort_bins[pass][i - 1];
            }
            let mut i = highest;
            while i >= 0 {
                let key = ((from[i as usize].sortkey >> (7 * pass)) % 128) as usize;
                sort_bins[pass][key] -= 1;
                if pass < 2 {
                    to[sort_bins[pass][key] as usize] = from[i as usize];
                } else {
                    *alpha_list.offset((highest - sort_bins[pass][key]) as isize) =
                        *list.offset(from[i as usize].visedict as isize);
                }
                i -= 1;
            }
        }
    }
}

/// `R_DrawEntitiesTask`
unsafe extern "C" fn r_draw_entities_task(index: c_int, use_tasks: *mut c_void) {
    // SAFETY: the entity contexts were opened by `GL_BeginRendering`.
    unsafe {
        let cbx = secondary_cbx(SCBX_ENTITIES).offset(index as isize);
        r_setup_context(cbx);
        Fog_EnableGFog(cbx); // johnfitz
        r_draw_entities_on_list(cbx, 0, index + CHAIN_MODEL_0, !use_tasks.is_null());
    }
}

/// `R_DrawAlphaEntitiesTask`
unsafe extern "C" fn r_draw_alpha_entities_task(index: c_int, use_tasks: *mut c_void) {
    // SAFETY: as above; `r_viewleaf` was set by `R_SetupViewBeforeMark`.
    unsafe {
        let contents = (**ptr::addr_of!(r_viewleaf)).contents;
        let underwater = r_use_alpha_sort()
            && (contents == CONTENTS_WATER
                || contents == CONTENTS_SLIME
                || contents == CONTENTS_LAVA);
        let (first, last) = if use_tasks.is_null() {
            (0, 1)
        } else {
            (index, index)
        };
        for i in first..=last {
            let cbx = secondary_cbx(if i != 0 {
                SCBX_ALPHA_ENTITIES
            } else {
                SCBX_ALPHA_ENTITIES_ACROSS_WATER
            });
            r_setup_context(cbx);
            Fog_EnableGFog(cbx);
            let alphapass = if underwater { 1 + i } else { 2 - i };
            let chain = if i != 0 {
                CHAIN_ALPHA_MODEL
            } else {
                CHAIN_ALPHA_MODEL_ACROSS_WATER
            };
            r_draw_entities_on_list(cbx, alphapass, chain, false);
            if raised() {
                return;
            }

            if r_use_mboit() {
                let cbx = secondary_cbx(if i != 0 {
                    SCBX_MBOIT_COMPOSITE_ALPHA_ENTITIES
                } else {
                    SCBX_MBOIT_COMPOSITE_ALPHA_ENTITIES_ACROSS_WATER
                });
                r_setup_context(cbx);
                Fog_EnableGFog(cbx);
                r_draw_entities_on_list(cbx, alphapass, chain, false);
                if raised() {
                    return;
                }
            }
        }
    }
}

/// `R_DrawParticlesTask`
unsafe extern "C" fn r_draw_particles_task(_unused: *mut c_void) {
    // SAFETY: the particle contexts were opened by `GL_BeginRendering`.
    unsafe {
        let cbx = secondary_cbx(SCBX_PARTICLES);
        r_setup_context(cbx);
        Fog_EnableGFog(cbx); // johnfitz
        c::render::R_DrawParticles(cbx.cast::<c_void>());

        if r_use_mboit() {
            // MBOIT needs all transparent geometry a second time in the composite pass
            let composite_cbx = secondary_cbx(SCBX_MBOIT_COMPOSITE_PARTICLES);
            r_setup_context(composite_cbx);
            Fog_EnableGFog(composite_cbx);
            c::render::R_DrawParticles(composite_cbx.cast::<c_void>());
        }
        let mut fte_blend_cbx: *mut CbContext = ptr::null_mut();
        if r_use_oit() {
            // the blend context is recorded for the resolve subpass, so only set the viewport here:
            // R_SetupContext would bind a pipeline created for subpass 0
            fte_blend_cbx = secondary_cbx(SCBX_FTE_PARTICLES_BLEND);
            let vrect = (*ptr::addr_of!(r_refdef)).vrect;
            GL_Viewport(
                fte_blend_cbx,
                vrect.x as f32,
                (glheight() - vrect.y - vrect.height) as f32,
                vrect.width as f32,
                vrect.height as f32,
                0.0,
                1.0,
            );
        }
        c::render::PScript_DrawParticles(
            if r_use_oit() { fte_blend_cbx } else { cbx }.cast::<c_void>(),
            ptr::null_mut(),
        );
    }
}

/// `R_DrawViewModelTask`
unsafe extern "C" fn r_draw_view_model_task(_unused: *mut c_void) {
    // SAFETY: the view-model context was opened by `GL_BeginRendering`.
    unsafe {
        let cbx = secondary_cbx(SCBX_VIEW_MODEL);
        r_setup_context(cbx);
        r_draw_view_model(cbx); // johnfitz -- moved here from R_RenderView
        if raised() {
            return;
        }
        r_show_tris(cbx); // johnfitz
        if raised() {
            return;
        }
        let _ = note(RRmain_Glue_ShowBoundingBoxes(cbx)); // johnfitz
    }
}

// ---- R_RenderView -----------------------------------------------------------

/// `Task_AllocateAndAssignFunc` (`tasks.h`) plus the `-renderhash` graph
/// registration of `gl_rmain.c`'s `R_GraphTask`.
unsafe fn graph_task(
    func: unsafe extern "C" fn(*mut c_void),
    payload: *mut c_void,
    payload_size: usize,
    name: *const c_char,
) -> u64 {
    // SAFETY: the task system is running; `name` is a static C string.
    unsafe {
        let handle = c::tasks::Task_Allocate();
        c::tasks::Task_AssignFunc(handle, Some(func), payload, payload_size);
        c::render::Harness_RenderGraphTask(handle, name, 0);
        handle
    }
}

/// `Task_AllocateAndAssignIndexedFunc` plus the graph registration.
unsafe fn graph_indexed_task(
    func: unsafe extern "C" fn(c_int, *mut c_void),
    limit: u32,
    payload: *mut c_void,
    payload_size: usize,
    name: *const c_char,
) -> u64 {
    // SAFETY: as above.
    unsafe {
        let handle = c::tasks::Task_Allocate();
        c::tasks::Task_AssignIndexedFunc(handle, Some(func), limit, payload, payload_size);
        c::render::Harness_RenderGraphTask(handle, name, limit);
        handle
    }
}

/// `R_GraphEdge` -- `Task_AddDependency` plus the graph registration.
unsafe fn graph_edge(before: u64, after: u64) {
    // SAFETY: both handles are live, unsubmitted tasks.
    unsafe {
        c::tasks::Task_AddDependency(before, after);
        c::render::Harness_RenderGraphEdge(before, after);
    }
}

/// `R_RenderView` status core: returns the first `Host_Guard` code raised by
/// a model draw or `R_StoreEfrags` on the serial path (0 otherwise) for
/// `gl_rmain_glue.c`'s `R_RenderView` to `Host_Reraise` (ADR-009).
///
/// # Safety
/// Main thread inside `SCR_UpdateScreen`; the task handles are live when
/// `use_tasks`.
#[no_mangle]
pub unsafe extern "C" fn RRmain_RenderView(
    mut use_tasks: bool,
    begin_rendering_task: u64,
    setup_frame_task: u64,
    draw_done_task: u64,
) -> c_int {
    PENDING_RAISE.store(0, Ordering::SeqCst);
    // SAFETY: the caller's contract.
    unsafe {
        *ptr::addr_of_mut!(indirect) = cvar_value(ptr::addr_of!(r_indirect)) != 0.0
            && *ptr::addr_of!(indirect_ready)
            && cvar_value(ptr::addr_of!(c::render::r_gpulightmapupdate)) != 0.0
            && cvar_value(ptr::addr_of!(scr_speeds)) == 0.0;

        if (*ptr::addr_of!(cl)).worldmodel.is_null() {
            c::Sys_Error(c"R_RenderView: NULL worldmodel".as_ptr());
        }

        let speeds = cvar_value(ptr::addr_of!(scr_speeds)) != 0.0;
        if speeds {
            *ptr::addr_of_mut!(rs_frame_starttime) = c::Sys_DoubleTime();
        }

        if use_tasks
            && (cvar_value(ptr::addr_of!(r_pos)) != 0.0 || STATS_READY.load(Ordering::SeqCst) != 0)
        {
            c::render::R_PrintStats(); // stats and frame times of the last completed frame
        }

        if speeds {
            // johnfitz -- rendering statistics
            atomic_u32(ptr::addr_of_mut!(c::render::rs_brushpolys)).store(0, Ordering::SeqCst);
            atomic_u32(ptr::addr_of_mut!(rs_aliaspolys)).store(0, Ordering::SeqCst);
            atomic_u32(ptr::addr_of_mut!(c::render::rs_skypolys)).store(0, Ordering::SeqCst);
            atomic_u32(ptr::addr_of_mut!(c::render::rs_particles)).store(0, Ordering::SeqCst);
            atomic_u32(ptr::addr_of_mut!(rs_fogpolys)).store(0, Ordering::SeqCst);
            atomic_u32(ptr::addr_of_mut!(c::render::rs_dynamiclightmaps))
                .store(0, Ordering::SeqCst);
            atomic_u32(ptr::addr_of_mut!(rs_aliaspasses)).store(0, Ordering::SeqCst);
            atomic_u32(ptr::addr_of_mut!(c::render::rs_brushpasses)).store(0, Ordering::SeqCst);
            STATS_READY.store(1, Ordering::SeqCst);
        } else {
            STATS_READY.store(0, Ordering::SeqCst);
        }

        if use_tasks {
            let payload = ptr::addr_of_mut!(use_tasks).cast::<c_void>();
            // `sizeof (use_tasks)` in the C: `qboolean` is `_Bool` (q_types.h).
            let payload_size = core::mem::size_of::<c::qboolean>();
            let num_workers = c::tasks::Tasks_NumWorkers();
            let use_indirect = is_indirect();

            let before_mark = graph_task(
                r_setup_view_before_mark,
                ptr::null_mut(),
                0,
                c"before_mark".as_ptr(),
            );
            graph_edge(setup_frame_task, before_mark);

            let mut store_efrags = INVALID_TASK_HANDLE;
            let mut cull_surfaces = INVALID_TASK_HANDLE;
            let mut chain_surfaces = INVALID_TASK_HANDLE;
            RWorld_MarkSurfaces(
                use_tasks,
                before_mark,
                &mut store_efrags,
                &mut cull_surfaces,
                &mut chain_surfaces,
            );

            let update_warp_textures = graph_task(
                R_UpdateWarpTextures,
                ptr::null_mut(),
                0,
                c"update_warp_textures".as_ptr(),
            );
            graph_edge(cull_surfaces, update_warp_textures);
            graph_edge(begin_rendering_task, update_warp_textures);
            graph_edge(update_warp_textures, draw_done_task);

            let draw_world_task = graph_indexed_task(
                r_draw_world_task,
                NUM_WORLD_CBX as u32,
                payload,
                payload_size,
                c"draw_world_task".as_ptr(),
            );
            if use_indirect {
                graph_edge(before_mark, draw_world_task);
            } else {
                graph_edge(chain_surfaces, draw_world_task);
            }
            graph_edge(begin_rendering_task, draw_world_task);
            graph_edge(draw_world_task, draw_done_task);

            let sort_transparents = graph_task(
                r_sort_alpha_entities_task,
                ptr::null_mut(),
                0,
                c"sort_transparents".as_ptr(),
            );
            graph_edge(store_efrags, sort_transparents);

            let draw_sky_task = graph_task(
                r_draw_sky_task,
                ptr::null_mut(),
                0,
                c"draw_sky_task".as_ptr(),
            );
            graph_edge(store_efrags, draw_sky_task);
            graph_edge(chain_surfaces, draw_sky_task);
            graph_edge(begin_rendering_task, draw_sky_task);
            graph_edge(draw_sky_task, draw_done_task);

            let draw_water_task = graph_task(
                r_draw_water_task,
                ptr::null_mut(),
                0,
                c"draw_water_task".as_ptr(),
            );
            graph_edge(chain_surfaces, draw_water_task);
            graph_edge(begin_rendering_task, draw_water_task);
            graph_edge(draw_water_task, draw_done_task);

            let draw_view_model_task = graph_task(
                r_draw_view_model_task,
                ptr::null_mut(),
                0,
                c"draw_view_model_task".as_ptr(),
            );
            graph_edge(before_mark, draw_view_model_task);
            graph_edge(begin_rendering_task, draw_view_model_task);
            graph_edge(draw_view_model_task, draw_done_task);

            NEXT_VISEDICT.store(0, Ordering::SeqCst);
            let draw_entities_task = graph_indexed_task(
                r_draw_entities_task,
                NUM_ENTITIES_CBX as u32,
                payload,
                payload_size,
                c"draw_entities_task".as_ptr(),
            );
            graph_edge(store_efrags, draw_entities_task);
            graph_edge(begin_rendering_task, draw_entities_task);

            let draw_alpha_entities_task = graph_indexed_task(
                r_draw_alpha_entities_task,
                2,
                payload,
                payload_size,
                c"draw_alpha_entities_task".as_ptr(),
            );
            graph_edge(sort_transparents, draw_alpha_entities_task);
            graph_edge(begin_rendering_task, draw_alpha_entities_task);

            // dlights queued by last frame's deferred effect spawns; must run before
            // anything reads cl_dlights and before layout refills the queues
            let flush_dlights_task = graph_task(
                c::render::PScript_FlushDlightsTask,
                ptr::null_mut(),
                0,
                c"flush_dlights_task".as_ptr(),
            );
            graph_edge(flush_dlights_task, draw_view_model_task);
            graph_edge(flush_dlights_task, draw_entities_task);
            graph_edge(flush_dlights_task, draw_alpha_entities_task);

            let update_particles_setup_task = graph_task(
                c::render::PScript_UpdateParticlesSetupTask,
                ptr::null_mut(),
                0,
                c"update_particles_setup_task".as_ptr(),
            );
            graph_edge(before_mark, update_particles_setup_task);

            let update_particles_task = graph_indexed_task(
                c::render::PScript_UpdateParticlesTask,
                num_workers as u32,
                ptr::null_mut(),
                0,
                c"update_particles_task".as_ptr(),
            );
            graph_edge(update_particles_setup_task, update_particles_task);

            // layout is the first task that writes the double buffered vertex/index buffers, it
            // must wait for begin_rendering so the GPU is done reading them from two frames ago
            let layout_particles_task = graph_task(
                c::render::PScript_LayoutParticlesTask,
                ptr::null_mut(),
                0,
                c"layout_particles_task".as_ptr(),
            );
            graph_edge(update_particles_task, layout_particles_task);
            graph_edge(begin_rendering_task, layout_particles_task);
            graph_edge(flush_dlights_task, layout_particles_task);

            let emit_particles_task = graph_indexed_task(
                c::render::PScript_EmitParticlesTask,
                num_workers as u32,
                ptr::null_mut(),
                0,
                c"emit_particles_task".as_ptr(),
            );
            graph_edge(layout_particles_task, emit_particles_task);

            let draw_particles_task = graph_task(
                r_draw_particles_task,
                ptr::null_mut(),
                0,
                c"draw_particles_task".as_ptr(),
            );
            graph_edge(before_mark, draw_particles_task);
            graph_edge(emit_particles_task, draw_particles_task);
            graph_edge(begin_rendering_task, draw_particles_task);
            graph_edge(draw_particles_task, draw_done_task);

            let build_tlas_task = graph_task(
                c::render::R_BuildTopLevelAccelerationStructure,
                ptr::null_mut(),
                0,
                c"build_tlas_task".as_ptr(),
            );
            graph_edge(store_efrags, build_tlas_task);
            graph_edge(begin_rendering_task, build_tlas_task);
            graph_edge(build_tlas_task, draw_done_task);

            let update_lightmaps_task = graph_task(
                R_UpdateLightmapsAndIndirect,
                ptr::null_mut(),
                0,
                c"update_lightmaps_task".as_ptr(),
            );
            graph_edge(cull_surfaces, update_lightmaps_task);
            graph_edge(draw_entities_task, update_lightmaps_task);
            graph_edge(draw_alpha_entities_task, update_lightmaps_task);
            graph_edge(flush_dlights_task, update_lightmaps_task);
            graph_edge(update_lightmaps_task, draw_done_task);

            if cvar_value(ptr::addr_of!(c::render::r_showtris)) != 0.0 {
                if !use_indirect {
                    graph_edge(chain_surfaces, draw_view_model_task);
                }

                graph_edge(draw_entities_task, draw_view_model_task); // not dependent, but mutually exclusive
                graph_edge(draw_alpha_entities_task, draw_view_model_task); // not dependent, but mutually exclusive

                graph_edge(draw_particles_task, draw_view_model_task); // only scriptable particles are dependent
            }

            let mut tasks = [
                before_mark,
                store_efrags,
                update_warp_textures,
                draw_world_task,
                sort_transparents,
                draw_sky_task,
                draw_water_task,
                draw_view_model_task,
                draw_entities_task,
                draw_alpha_entities_task,
                flush_dlights_task,
                update_particles_setup_task,
                update_particles_task,
                layout_particles_task,
                emit_particles_task,
                draw_particles_task,
                build_tlas_task,
                update_lightmaps_task,
            ];
            c::tasks::Tasks_Submit(tasks.len() as c_int, tasks.as_mut_ptr());
            if cull_surfaces != chain_surfaces {
                c::tasks::Task_Submit(cull_surfaces);
                c::tasks::Task_Submit(chain_surfaces);
            }
            0
        } else {
            r_setup_view_before_mark(ptr::null_mut());
            // johnfitz -- create texture chains from PVS
            let code = RWorld_MarkSurfaces(
                use_tasks,
                INVALID_TASK_HANDLE,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            );
            if code != 0 {
                return code;
            }
            R_UpdateWarpTextures(ptr::null_mut());
            r_draw_world_task(0, ptr::null_mut());
            r_draw_sky_task(ptr::null_mut());
            r_draw_water_task(ptr::null_mut());
            r_draw_entities_task(0, ptr::null_mut());
            if raised() {
                return PENDING_RAISE.load(Ordering::SeqCst);
            }
            r_sort_alpha_entities_task(ptr::null_mut());
            r_draw_alpha_entities_task(0, ptr::null_mut());
            if raised() {
                return PENDING_RAISE.load(Ordering::SeqCst);
            }
            // no-op here (spawns run on the main thread), but keeps the queues drained across mode switches
            c::render::PScript_FlushDlightsTask(ptr::null_mut());
            // the C entry is a `Host_Reraise` wrapper over this status core
            let code = c::render::quake_rs_ftepart_update_particles_setup_task();
            if code != 0 {
                return code;
            }
            let particle_workers = c::tasks::Tasks_NumWorkers().max(1);
            for pi in 0..particle_workers {
                c::render::PScript_UpdateParticlesTask(pi, ptr::null_mut());
            }
            c::render::PScript_LayoutParticlesTask(ptr::null_mut());
            for pi in 0..particle_workers {
                c::render::PScript_EmitParticlesTask(pi, ptr::null_mut());
            }
            r_draw_particles_task(ptr::null_mut());
            r_draw_view_model_task(ptr::null_mut());
            if raised() {
                return PENDING_RAISE.load(Ordering::SeqCst);
            }
            if cvar_value(ptr::addr_of!(c::render::r_gpulightmapupdate)) != 0.0 {
                c::render::R_BuildTopLevelAccelerationStructure(ptr::null_mut());
                R_UpdateLightmapsAndIndirect(ptr::null_mut());
            }
            c::render::R_PrintStats();
            0
        }
    }
}
