//! `Quake/gl_warp.c` -- the per-frame water-warp texture update, either
//! rasterized through `warp_render_pass` or computed with `cs_tex_warp`
//! (Rust migration Phase 8 M7, Pattern A whole-file swap, ADR-015).
//!
//! `Quake/gl_warp_glue.c` keeps the three cvar definitions. The `turbsin`
//! table lives in [`crate::gl_warp_sin`].

use core::ffi::{c_int, c_void};
use core::ptr;
use core::sync::atomic::{AtomicU32, Ordering};

use ash::vk;
use quake_c_sys as c;
use quake_render::cb::{self, CmdProcs};
use quake_types::host::{ClientState, MAX_MODELS};
use quake_types::model_mem::{QModel, Texture, SURF_DRAWTURB};
use quake_types::render::{BasicVertex, CbContext, GlTexture, MAX_GLTEXTURES, PCBX_UPDATE_WARP};

use crate::gl_rmisc::{device, vg, with_ctx, DYN};
use crate::gl_warp_sin::TURBSIN;

extern "C" {
    /// `client_state_t cl` (ADR-007 row closed in Phase 7).
    static mut cl: ClientState;
}

const WARPIMAGESIZE: u32 = 512;
const WARPIMAGEMIPS: u32 = 5;
use crate::gl_draw::{CANVAS_NONE, CANVAS_WARPIMAGE};

#[inline]
fn cvar_value(cv: *const c::cvar_t) -> f32 {
    // SAFETY: the glue TU defines the cvar for the whole process.
    unsafe { (*cv).value }
}

/// `WARPCALC (s, t)` (`gl_warp.c:36`): `s`/`t` are `float`, the table index
/// is computed in `double` from `cl.time`, and the `1.0 / 64` scale is a
/// `double` multiply; the caller narrows to `float`.
#[inline]
fn warpcalc(s: f32, t: f32, cl_time: f64) -> f64 {
    let idx = (f64::from(t * 2.0) + cl_time * (128.0 / core::f64::consts::PI)) as c_int & 255;
    f64::from(s + TURBSIN[idx as usize]) * (1.0 / 64.0)
}

/// `R_RasterWarpTexture`: renders the warped texture into the top mip of
/// `tx->warpimage` through the raster pipeline.
unsafe fn raster_warp_texture(cbx: *mut CbContext, tx: *mut Texture, warptess: f32) {
    let render_area = vk::Rect2D {
        offset: vk::Offset2D { x: 0, y: 0 },
        extent: vk::Extent2D {
            width: WARPIMAGESIZE,
            height: WARPIMAGESIZE,
        },
    };
    with_ctx(|ctx| {
        let procs = CmdProcs::new(ctx.vg);
        let warp_render_pass = vg!(ctx, warp_render_pass);
        let raster_pipeline = vg!(ctx, raster_tex_warp_pipeline);
        let basic_layout = vg!(ctx, basic_pipeline_layout.handle);
        // SAFETY: `cbx` is `primary_cb_contexts[PCBX_UPDATE_WARP]` and `tx` a
        // live `texture_t` whose `gltexture`/`warpimage` point at live
        // `gltexture_t`s (the texture manager owns them, ADR-007).
        unsafe {
            let warpimage = (*tx).warpimage.cast::<GlTexture>();
            let gltexture = (*tx).gltexture.cast::<GlTexture>();
            let device = device();
            let begin = vk::RenderPassBeginInfo::default()
                .render_area(render_area)
                .render_pass(warp_render_pass)
                .framebuffer((*warpimage).frame_buffer);
            device.cmd_begin_render_pass((*cbx).cb, &begin, vk::SubpassContents::INLINE);

            cb::bind_pipeline(
                &procs,
                &mut *cbx,
                vk::PipelineBindPoint::GRAPHICS,
                raster_pipeline,
            );
            crate::gl_draw::GL_SetCanvas(cbx, CANVAS_WARPIMAGE);
            device.cmd_set_scissor((*cbx).cb, 0, &[render_area]);
            let set = if !(*ptr::addr_of!(c::render::r_lightmap_cheatsafe)) {
                (*gltexture).descriptor_set
            } else {
                (*(*ptr::addr_of!(c::render::whitetexture)).cast::<GlTexture>()).descriptor_set
            };
            device.cmd_bind_descriptor_sets(
                (*cbx).cb,
                vk::PipelineBindPoint::GRAPHICS,
                basic_layout,
                0,
                &[set],
                &[],
            );

            let cl_time = (*ptr::addr_of!(cl)).time;
            let mut num_verts: u32 = 0;
            let mut y: f32 = 0.0;
            while f64::from(y) < 128.01 {
                // .01 for rounding errors
                num_verts += 2;
                y += warptess;
            }

            let mut x: f32 = 0.0;
            while f64::from(x) < 128.0 {
                let a = DYN
                    .vertex_allocate(ctx, num_verts * core::mem::size_of::<BasicVertex>() as u32);
                let vertices = a.data.cast::<BasicVertex>();
                let x2 = x + warptess;
                let mut i = 0usize;
                let mut y: f32 = 0.0;
                while f64::from(y) < 128.01 {
                    vertices.add(i).write(BasicVertex {
                        position: [x, y, 0.0],
                        texcoord: [
                            warpcalc(x, y, cl_time) as f32,
                            (1.0 - warpcalc(y, x, cl_time)) as f32,
                        ],
                        color: [255; 4],
                    });
                    i += 1;
                    vertices.add(i).write(BasicVertex {
                        position: [x2, y, 0.0],
                        texcoord: [
                            warpcalc(x2, y, cl_time) as f32,
                            (1.0 - warpcalc(y, x2, cl_time)) as f32,
                        ],
                        color: [255; 4],
                    });
                    i += 1;
                    y += warptess;
                }

                device.cmd_bind_vertex_buffers((*cbx).cb, 0, &[a.buffer], &[a.buffer_offset]);
                cb::draw(&procs, (*cbx).cb, num_verts, 1, 0, 0);
                x = x2;
            }

            device.cmd_end_render_pass((*cbx).cb);
        }
    })
}

/// `R_ComputeWarpTexture`: dispatches `cs_tex_warp` into the storage view of
/// `tx->warpimage`.
unsafe fn compute_warp_texture(cbx: *mut CbContext, tx: *mut Texture) {
    with_ctx(|ctx| {
        let procs = CmdProcs::new(ctx.vg);
        let pipeline = vg!(ctx, cs_tex_warp_pipeline);
        // SAFETY: as [`raster_warp_texture`].
        unsafe {
            let time = (*ptr::addr_of!(cl)).time as f32;
            cb::bind_pipeline(&procs, &mut *cbx, vk::PipelineBindPoint::COMPUTE, pipeline);
            let mut sets = [
                (*(*tx).gltexture.cast::<GlTexture>()).descriptor_set,
                (*(*tx).warpimage.cast::<GlTexture>()).storage_descriptor_set,
            ];
            if *ptr::addr_of!(c::render::r_lightmap_cheatsafe) {
                sets[0] =
                    (*(*ptr::addr_of!(c::render::whitetexture)).cast::<GlTexture>()).descriptor_set;
            }
            ctx.device.cmd_bind_descriptor_sets(
                (*cbx).cb,
                vk::PipelineBindPoint::COMPUTE,
                pipeline.layout.handle,
                0,
                &sets,
                &[],
            );
            cb::push_constants(
                &procs,
                &*cbx,
                vk::ShaderStageFlags::COMPUTE,
                0,
                &time.to_ne_bytes(),
            );
            ctx.device
                .cmd_dispatch((*cbx).cb, WARPIMAGESIZE / 8, WARPIMAGESIZE / 8, 1);
        }
    })
}

fn image_barrier(
    image: vk::Image,
    src: vk::AccessFlags,
    dst: vk::AccessFlags,
    old_layout: vk::ImageLayout,
    new_layout: vk::ImageLayout,
    base_mip: u32,
    mips: u32,
) -> vk::ImageMemoryBarrier<'static> {
    vk::ImageMemoryBarrier::default()
        .src_access_mask(src)
        .dst_access_mask(dst)
        .old_layout(old_layout)
        .new_layout(new_layout)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(
            vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .base_mip_level(base_mip)
                .level_count(mips)
                .base_array_layer(0)
                .layer_count(1),
        )
}

/// `R_UpdateWarpTextures` -- johnfitz -- each frame, update warping
/// textures (a task function; `unused` is the task payload).
///
/// # Safety
/// Runs on the task system while `vulkan_globals`, `cl` and the texture list
/// are stable for the frame.
#[no_mangle]
pub unsafe extern "C" fn R_UpdateWarpTextures(_unused: *mut c_void) {
    static mut WARP_TEXTURES: [*mut Texture; MAX_GLTEXTURES as usize] =
        [ptr::null_mut(); MAX_GLTEXTURES as usize];
    static mut WARP_IMAGE_BARRIERS: [vk::ImageMemoryBarrier<'static>; MAX_GLTEXTURES as usize] =
        // SAFETY: an all-zero `VkImageMemoryBarrier` is a valid (if
        // meaningless) value; every used entry is fully written below.
        [unsafe { core::mem::zeroed() }; MAX_GLTEXTURES as usize];

    // SAFETY: the statics are only touched from this task, one instance of
    // which runs per frame; `cbx` is the update-warp primary context; the
    // model/texture pointers are live for the frame (contract above).
    unsafe {
        let cbx = with_ctx(|ctx| {
            ptr::addr_of_mut!((*ctx.vg.as_ptr()).primary_cb_contexts[PCBX_UPDATE_WARP])
        });
        crate::gl_draw::GL_SetCanvas(cbx, CANVAS_NONE); // Invalidate canvas so push constants get set later

        if (*ptr::addr_of!(cl)).paused {
            return;
        }

        with_ctx(|ctx| {
            cb::begin_debug_utils_label(&CmdProcs::new(ctx.vg), &*cbx, c"Update Warp Textures")
        });

        let quality = f64::from(cvar_value(ptr::addr_of!(c::render::r_waterquality))).floor();
        let warptess = (128.0 / quality.clamp(3.0, 64.0)) as f32;
        let compute = cvar_value(ptr::addr_of!(c::render::r_waterwarpcompute)) != 0.0;

        let textures = &mut *ptr::addr_of_mut!(WARP_TEXTURES);
        let barriers = &mut *ptr::addr_of_mut!(WARP_IMAGE_BARRIERS);
        let mut num_warp_textures = 0usize;

        // Count warp texture & prepare barrier from undefined to GENERAL if using compute warp
        for j in 1..MAX_MODELS {
            let m: *mut QModel = (*ptr::addr_of!(cl)).model_precache[j];
            if m.is_null() {
                break;
            }
            if (*m).name[0] == b'*' as _ {
                continue;
            }
            if j > 1 && ((*m).used_specials & SURF_DRAWTURB) == 0 {
                continue;
            }
            for i in 0..(*m).numtextures.max(0) as usize {
                let tx = *(*m).textures.add(i);
                if tx.is_null() {
                    continue;
                }
                if AtomicU32::from_ptr(ptr::addr_of_mut!((*tx).update_warp)).load(Ordering::SeqCst)
                    == 0
                {
                    continue;
                }
                if compute {
                    barriers[num_warp_textures] = image_barrier(
                        (*(*tx).warpimage.cast::<GlTexture>()).image,
                        vk::AccessFlags::SHADER_READ,
                        vk::AccessFlags::SHADER_WRITE,
                        vk::ImageLayout::UNDEFINED,
                        vk::ImageLayout::GENERAL,
                        0,
                        WARPIMAGEMIPS,
                    );
                }
                textures[num_warp_textures] = tx;
                num_warp_textures += 1;
            }
        }

        let device = device();
        let cmd = (*cbx).cb;

        // Transfer mips from UNDEFINED to GENERAL layout
        if compute {
            device.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &barriers[..num_warp_textures],
            );
        }

        // Render warp to top mips
        for i in 0..num_warp_textures {
            let tx = textures[i];
            if compute {
                compute_warp_texture(cbx, tx);
            } else {
                raster_warp_texture(cbx, tx, warptess);
            }
            barriers[i] = image_barrier(
                (*(*tx).warpimage.cast::<GlTexture>()).image,
                vk::AccessFlags::empty(),
                vk::AccessFlags::TRANSFER_WRITE,
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::GENERAL,
                1,
                WARPIMAGEMIPS - 1,
            );
        }

        // Make sure that writes are done for top mips we just rendered to
        let mut memory_barrier = vk::MemoryBarrier::default()
            .src_access_mask(if compute {
                vk::AccessFlags::SHADER_WRITE
            } else {
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE
            })
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ);

        // Transfer all other mips from UNDEFINED to GENERAL layout
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::FRAGMENT_SHADER
                | if compute {
                    vk::PipelineStageFlags::COMPUTE_SHADER
                } else {
                    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                },
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[memory_barrier],
            &[],
            &barriers[..num_warp_textures],
        );

        // Generate mip chains
        for mip in 1..WARPIMAGEMIPS {
            let src_size = (WARPIMAGESIZE >> (mip - 1)) as i32;
            let dst_size = (WARPIMAGESIZE >> mip) as i32;

            for &tx in &textures[..num_warp_textures] {
                let image = (*(*tx).warpimage.cast::<GlTexture>()).image;
                let region = vk::ImageBlit {
                    src_subresource: vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: mip - 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    },
                    src_offsets: [
                        vk::Offset3D { x: 0, y: 0, z: 0 },
                        vk::Offset3D {
                            x: src_size,
                            y: src_size,
                            z: 1,
                        },
                    ],
                    dst_subresource: vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: mip,
                        base_array_layer: 0,
                        layer_count: 1,
                    },
                    dst_offsets: [
                        vk::Offset3D { x: 0, y: 0, z: 0 },
                        vk::Offset3D {
                            x: dst_size,
                            y: dst_size,
                            z: 1,
                        },
                    ],
                };
                device.cmd_blit_image(
                    cmd,
                    image,
                    vk::ImageLayout::GENERAL,
                    image,
                    vk::ImageLayout::GENERAL,
                    &[region],
                    vk::Filter::LINEAR,
                );
            }

            if mip < WARPIMAGEMIPS - 1 {
                memory_barrier = memory_barrier
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ);
                device.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[memory_barrier],
                    &[],
                    &[],
                );
            }
        }

        // Transfer all warp texture mips from GENERAL to SHADER_READ_ONLY_OPTIMAL
        for i in 0..num_warp_textures {
            let tx = textures[i];
            barriers[i] = image_barrier(
                (*(*tx).warpimage.cast::<GlTexture>()).image,
                vk::AccessFlags::TRANSFER_WRITE,
                vk::AccessFlags::SHADER_READ,
                vk::ImageLayout::GENERAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                0,
                WARPIMAGEMIPS,
            );
            AtomicU32::from_ptr(ptr::addr_of_mut!((*tx).update_warp)).store(0, Ordering::SeqCst);
        }

        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &barriers[..num_warp_textures],
        );

        with_ctx(|ctx| cb::end_debug_utils_label(&CmdProcs::new(ctx.vg), &*cbx));
    }
}
