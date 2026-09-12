//! `r_sprite.c` -- sprite model rendering (Rust migration Phase 8 M7).
//!
//! The two C entry points, `R_DrawSpriteModel` and
//! `R_DrawSpriteModel_ShowTris`, stay as thin wrappers in
//! `Quake/r_sprite_glue.c`: they call `Mod_Extradata (e->model)`, which can
//! reach `Host_Error` through `Mod_LoadModel`, in C (ADR-009 -- no longjmp
//! through Rust frames) and hand the resolved `msprite_t *` to the exports
//! below. Everything else is transcribed statement for statement.
//!
//! COMPAT: ADR-010. `R_GetSpriteFrame` and `R_CreateSpriteVertices` mix
//! `float` and `double` exactly where the C does (`atan2`/`sin`/`cos` are the
//! CRT's `double` functions with `float` arguments promoted, results narrowed).

use core::ffi::c_int;
use core::ptr;

use ash::vk;
use quake_c_sys as c;
use quake_math::mathlib::{
    self as m, angle_vectors, cross_product, dot_product, vector_ma, vector_scale, Vec3, ROLL,
};
use quake_render::cb::{self, CmdProcs};
use quake_types::host::{ClientState, Entity};
use quake_types::model_mem::{MSprite, MSpriteFrame, MSpriteGroup};
use quake_types::render::{BasicVertex, CbContext, GlTexture};
use quake_types::spritegn::{
    SPR_ANGLED, SPR_FACING_UPRIGHT, SPR_ORIENTED, SPR_SINGLE, SPR_VP_PARALLEL,
    SPR_VP_PARALLEL_ORIENTED, SPR_VP_PARALLEL_UPRIGHT,
};

use crate::gl_rmisc::{device, vg, with_ctx, CEngine, DYN};
use quake_render::rmisc::Ctx;

extern "C" {
    /// `client_state_t cl` (ADR-007 row closed in Phase 7).
    static mut cl: ClientState;
}

/// `M_PI` (`mathlib.h`).
const M_PI: f64 = core::f64::consts::PI;
/// `M_PI_DIV_180` (`mathlib.h`).
const M_PI_DIV_180: f64 = M_PI / 180.0;

/// `OFFSET_NONE` / `OFFSET_DECAL` (`glquake.h`) -- depth bias constant factors.
const OFFSET_NONE: f32 = 0.0;
const OFFSET_DECAL: f32 = 1.0;

/// `ENTSCALE_DECODE (es)` (`protocol.h`): `(es) / 16.0f`.
fn entscale_decode(es: u8) -> f32 {
    f32::from(es) / 16.0
}

/// `VectorNormalizeFast` (`mathlib.h:107`): the `0x5f3759df` inverse square
/// root with one Newton step, applied through a float/int union.
fn vector_normalize_fast(v: &mut Vec3) {
    let number = dot_product(v, v);
    if number != 0.0 {
        let yi = 0x5f37_59df_i32.wrapping_sub(number.to_bits() as i32 >> 1);
        let mut y = f32::from_bits(yi as u32);
        y *= 1.5 - (number * 0.5 * y * y);
        let input = *v;
        vector_scale(&input, y, v);
    }
}

/// `R_SpritePipelineForRenderPass`.
fn sprite_pipeline_for_render_pass(
    ctx: &mut Ctx<'_, CEngine>,
    cbx: &CbContext,
) -> quake_types::render::VulkanPipeline {
    let idx = cbx.render_pass_index;
    let variant = cb::main_pass_pipeline_variant(idx);
    let main = vg!(ctx, sprite_pipeline[variant]);
    let oit = vg!(ctx, sprite_oit_pipeline);
    let moment = vg!(ctx, sprite_mboit_moment_pipeline);
    let composite = vg!(ctx, sprite_mboit_composite_pipeline);
    cb::pipeline_for_render_pass(idx, main, oit, moment, composite)
}

/// `R_GetSpriteFrame`.
///
/// # Safety
/// `e` and `psprite` must be valid; `psprite` is `Mod_Extradata (e->model)`.
unsafe fn get_sprite_frame(e: *mut Entity, psprite: *mut MSprite) -> *mut MSpriteFrame {
    // SAFETY: the caller's contract (above); `frame` is range-checked before
    // indexing `frames`, and group frame tables are `numframes` long.
    unsafe {
        let mut frame = (*e).frame;

        if frame >= (*psprite).numframes || frame < 0 {
            c::Con_DPrintf(
                c"R_DrawSprite: no such frame %d for '%s'\n".as_ptr(),
                frame,
                (*(*e).model).name.as_ptr(),
            );
            frame = 0;
        }

        let desc = (*psprite).frames.as_mut_ptr().add(frame as usize);
        if (*desc).type_ == SPR_SINGLE {
            (*desc).frameptr
        } else if (*desc).type_ == SPR_ANGLED {
            let pspritegroup = (*desc).frameptr.cast::<MSpriteGroup>();
            let mut axis = [[0.0f32; 3]; 3];
            let (a0, rest) = axis.split_at_mut(1);
            let (a1, a2) = rest.split_at_mut(1);
            angle_vectors(&(*e).angles, &mut a0[0], &mut a1[0], &mut a2[0]);
            let f = dot_product(&*ptr::addr_of!(c::cl_main::vpn), &axis[0]);
            let r = dot_product(&*ptr::addr_of!(c::host::vright), &axis[0]);
            let dir = ((c::libm::atan2(f64::from(r), f64::from(f)) + 1.125 * M_PI) * (4.0 / M_PI))
                as c_int;
            *(*pspritegroup).frames.as_mut_ptr().add((dir & 7) as usize)
        } else {
            let pspritegroup = (*desc).frameptr.cast::<MSpriteGroup>();
            let pintervals = (*pspritegroup).intervals;
            let numframes = (*pspritegroup).numframes;

            let fullinterval: f32 = *pintervals.add((numframes - 1) as usize);

            let time: f32 = ((*ptr::addr_of!(cl)).time + f64::from((*e).syncbase)) as f32;

            // when loading in Mod_LoadSpriteGroup, we guaranteed all interval values
            // are positive, so we don't have to worry about division by 0
            let targettime: f32 = time - ((time / fullinterval) as c_int) as f32 * fullinterval;

            let mut i = 0usize;
            while i < (numframes - 1) as usize {
                if *pintervals.add(i) > targettime {
                    break;
                }
                i += 1;
            }

            *(*pspritegroup).frames.as_mut_ptr().add(i)
        }
    }
}

/// `R_CreateSpriteVertices`.
///
/// # Safety
/// `e`, `frame`, `psprite` valid; `vertices` points at four writable
/// `basicvertex_t`s.
unsafe fn create_sprite_vertices(
    e: *mut Entity,
    frame: *mut MSpriteFrame,
    psprite: *mut MSprite,
    vertices: *mut BasicVertex,
) {
    // SAFETY: the caller's contract (above); `vertices` is a 4-element
    // allocation from the dynamic vertex buffer.
    unsafe {
        let mut point: Vec3 = [0.0; 3];
        let mut v_forward: Vec3 = [0.0; 3];
        let mut v_right: Vec3 = [0.0; 3];
        let mut v_up: Vec3 = [0.0; 3];
        let s_up: Vec3;
        let s_right: Vec3;

        let scale = entscale_decode((*e).netstate.scale);

        let vpn: Vec3 = *ptr::addr_of!(c::cl_main::vpn);
        let vright: Vec3 = *ptr::addr_of!(c::host::vright);
        let vup: Vec3 = *ptr::addr_of!(c::host::vup);
        let r_origin: Vec3 = *ptr::addr_of!(c::host::r_origin);

        match (*psprite).type_ {
            SPR_VP_PARALLEL_UPRIGHT => {
                // faces view plane, up is towards the heavens
                v_up = [0.0, 0.0, 1.0];
                cross_product(&vpn, &v_up, &mut v_right);
                vector_normalize_fast(&mut v_right);
                s_up = v_up;
                s_right = v_right;
            }
            SPR_FACING_UPRIGHT => {
                // faces camera origin, up is towards the heavens
                m::vector_subtract(&(*e).origin, &r_origin, &mut v_forward);
                v_forward[2] = 0.0;
                vector_normalize_fast(&mut v_forward);
                v_right = [v_forward[1], -v_forward[0], 0.0];
                v_up = [0.0, 0.0, 1.0];
                s_up = v_up;
                s_right = v_right;
            }
            SPR_VP_PARALLEL => {
                // faces view plane, up is towards the top of the screen
                s_up = vup;
                s_right = vright;
            }
            SPR_ORIENTED => {
                // pitch yaw roll are independent of camera
                angle_vectors(&(*e).angles, &mut v_forward, &mut v_right, &mut v_up);
                s_up = v_up;
                s_right = v_right;
            }
            SPR_VP_PARALLEL_ORIENTED => {
                // faces view plane, but obeys roll value
                let angle: f32 = (f64::from((*e).angles[ROLL]) * M_PI_DIV_180) as f32;
                let sr: f32 = c::libm::sin(f64::from(angle)) as f32;
                let cr: f32 = c::libm::cos(f64::from(angle)) as f32;
                for i in 0..3 {
                    v_right[i] = vright[i] * cr + vup[i] * sr;
                    v_up[i] = vright[i] * -sr + vup[i] * cr;
                }
                s_up = v_up;
                s_right = v_right;
            }
            _ => return,
        }

        ptr::write_bytes(vertices, 255, 4);

        vector_ma(&(*e).origin, (*frame).down * scale, &s_up, &mut point);
        let p = point;
        vector_ma(&p, (*frame).left * scale, &s_right, &mut point);
        (*vertices).position = point;
        (*vertices).texcoord = [0.0, (*frame).tmax];

        vector_ma(&(*e).origin, (*frame).up * scale, &s_up, &mut point);
        let p = point;
        vector_ma(&p, (*frame).left * scale, &s_right, &mut point);
        (*vertices.add(1)).position = point;
        (*vertices.add(1)).texcoord = [0.0, 0.0];

        vector_ma(&(*e).origin, (*frame).up * scale, &s_up, &mut point);
        let p = point;
        vector_ma(&p, (*frame).right * scale, &s_right, &mut point);
        (*vertices.add(2)).position = point;
        (*vertices.add(2)).texcoord = [(*frame).smax, 0.0];

        vector_ma(&(*e).origin, (*frame).down * scale, &s_up, &mut point);
        let p = point;
        vector_ma(&p, (*frame).right * scale, &s_right, &mut point);
        (*vertices.add(3)).position = point;
        (*vertices.add(3)).texcoord = [(*frame).smax, (*frame).tmax];
    }
}

/// The shared front half of both draw paths: allocate the four vertices,
/// fill them, bind vertex/index buffers, bind the sprite pipeline. Returns
/// the frame drawn.
///
/// # Safety
/// As for [`get_sprite_frame`].
unsafe fn setup_sprite(
    cbx: *mut CbContext,
    e: *mut Entity,
    psprite: *mut MSprite,
) -> *mut MSpriteFrame {
    with_ctx(|ctx| {
        let procs = CmdProcs::new(ctx.vg);
        let fan_index_buffer = vg!(ctx, fan_index_buffer);
        let a = DYN.vertex_allocate(ctx, 4 * core::mem::size_of::<BasicVertex>() as u32);
        let device = device();

        // SAFETY: the caller's contract; `a.data` is a fresh 96-byte
        // dynamic-buffer allocation.
        unsafe {
            let pipeline = sprite_pipeline_for_render_pass(ctx, &*cbx);
            let frame = get_sprite_frame(e, psprite);
            create_sprite_vertices(e, frame, psprite, a.data.cast::<BasicVertex>());

            device.cmd_bind_vertex_buffers((*cbx).cb, 0, &[a.buffer], &[a.buffer_offset]);
            device.cmd_bind_index_buffer((*cbx).cb, fan_index_buffer, 0, vk::IndexType::UINT16);

            cb::bind_pipeline(&procs, &mut *cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
            frame
        }
    })
}

/// The shared back half: bind the frame's texture and draw the two triangles.
///
/// # Safety
/// `frame->gltexture` must be a live `gltexture_t`.
unsafe fn draw_sprite_quad(cbx: *mut CbContext, frame: *mut MSpriteFrame) {
    let layout = with_ctx(|ctx| vg!(ctx, basic_pipeline_layout.handle));
    let device = device();
    // SAFETY: the caller's contract; `cbx` is recording inside a render pass.
    unsafe {
        let set = (*(*frame).gltexture.cast::<GlTexture>()).descriptor_set;
        device.cmd_bind_descriptor_sets(
            (*cbx).cb,
            vk::PipelineBindPoint::GRAPHICS,
            layout,
            0,
            &[set],
            &[],
        );
        with_ctx(|ctx| cb::draw_indexed(&CmdProcs::new(ctx.vg), (*cbx).cb, 6, 1, 0, 0, 0));
    }
}

/// `R_DrawSpriteModel` minus the `Mod_Extradata` call (done by the C
/// wrapper in `r_sprite_glue.c`).
///
/// # Safety
/// `cbx` is a live command-buffer context inside a render pass; `e` is a
/// sprite entity and `psprite` its `Mod_Extradata`.
#[no_mangle]
pub unsafe extern "C" fn RSprite_DrawSpriteModel(
    cbx: *mut CbContext,
    e: *mut Entity,
    psprite: *mut MSprite,
) {
    // SAFETY: the caller's contract (above).
    unsafe {
        let frame = setup_sprite(cbx, e, psprite);

        let device = device();
        if (*psprite).type_ == SPR_ORIENTED {
            device.cmd_set_depth_bias((*cbx).cb, OFFSET_DECAL, 0.0, 1.0);
        } else {
            device.cmd_set_depth_bias((*cbx).cb, OFFSET_NONE, 0.0, 0.0);
        }

        draw_sprite_quad(cbx, frame);
    }
}

/// `R_DrawSpriteModel_ShowTris` minus the `Mod_Extradata` call.
///
/// # Safety
/// As for [`RSprite_DrawSpriteModel`].
#[no_mangle]
pub unsafe extern "C" fn RSprite_DrawSpriteModel_ShowTris(
    cbx: *mut CbContext,
    e: *mut Entity,
    psprite: *mut MSprite,
) {
    // SAFETY: the caller's contract (above); `r_showtris` is defined by
    // gl_rmain.c for the whole process.
    let (frame, variant, showtris_value) = unsafe {
        (
            setup_sprite(cbx, e, psprite),
            cb::main_pass_pipeline_variant((*cbx).render_pass_index),
            (*ptr::addr_of!(c::render::r_showtris)).value,
        )
    };

    with_ctx(|ctx| {
        let procs = CmdProcs::new(ctx.vg);
        let showtris = vg!(ctx, showtris_pipeline[variant]);
        let showtris_depth = vg!(ctx, showtris_depth_test_pipeline[variant]);
        let pipeline = if showtris_value == 1.0 {
            showtris
        } else {
            showtris_depth
        };
        // SAFETY: `cbx` is the caller's live context (contract above).
        unsafe {
            cb::bind_pipeline(&procs, &mut *cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
        }
    });

    // SAFETY: as above.
    unsafe { draw_sprite_quad(cbx, frame) }
}
