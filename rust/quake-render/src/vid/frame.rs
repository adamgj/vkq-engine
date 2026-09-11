//! The `gl_vidsdl.c` per-frame path: `GL_BeginRenderingTask`,
//! `GL_AcquireNextSwapChainImage`, `GL_EndRenderingTask` (screen effects,
//! OIT resolve, screenshot copy, submit, present) and `GL_WaitForDeviceIdle`
//! (Phase 8 M6).
//!
//! `GL_BeginRendering`/`GL_EndRendering` themselves -- task allocation,
//! `VID_Restart`, the `r_oit` mode latch and the main-thread sampling of
//! [`EndRenderingParms`] -- stay in `quake-capi`, next to the task system.

use core::ffi::c_int;
use core::mem::size_of;
use core::slice;

use ash::vk;
use quake_types::render::{VulkanMemory, VulkanPipeline, PCBX_NUM, PCBX_RENDER_PASSES};

use super::{
    fail, scbx_ptr, scbx_slots, use_mboit, use_oit, use_wboit, PresentId2KHR, PresentWait2InfoKHR,
    VidEngine, VidState, CANVAS_INVALID, CANVAS_NONE, DOUBLE_BUFFERED, MAIN_RENDER_PASS_MBOIT,
    MAIN_RENDER_PASS_NO_STENCIL, MAIN_RENDER_PASS_OIT, MAIN_RENDER_PASS_STANDARD,
    MAIN_RENDER_PASS_STENCIL_CLEAR, RENDER_PASS_INDEX_MAIN, RENDER_PASS_INDEX_MAIN_MBOIT,
    RENDER_PASS_INDEX_MAIN_OIT, RENDER_PASS_INDEX_MBOIT_COMPOSITE, RENDER_PASS_INDEX_MBOIT_MOMENTS,
    RENDER_PASS_INDEX_WBOIT, SCBX_ALPHA_ENTITIES_ACROSS_WATER, SCBX_FTE_PARTICLES_BLEND, SCBX_GUI,
    SCBX_MAIN_OPAQUE_PASS_LAST, SCBX_MAIN_PASS_LAST, SCBX_MBOIT_COMPOSITE_PASS_FIRST,
    SCBX_MBOIT_COMPOSITE_PASS_LAST, SCBX_OIT_RESOLVE, SCBX_POST_PROCESS, SCBX_VIEW_MODEL,
    SCBX_WORLD, SECONDARY_CB_MULTIPLICITY, STRUCTURE_TYPE_PRESENT_ID_2_KHR,
    STRUCTURE_TYPE_PRESENT_WAIT_2_INFO_KHR,
};
use crate::cb::{self, CmdProcs};
use crate::rmisc::memory::{create_buffer, free_buffer};
use crate::rmisc::Ctx;

/// `end_rendering_parms_t`: what `GL_EndRendering` samples on the main thread
/// for the `GL_EndRenderingTask` payload (the C bitfields are plain fields;
/// the struct only ever crosses Rust frames).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct EndRenderingParms {
    pub swapchain: bool,
    pub use_oit: bool,
    pub use_mboit: bool,
    pub render_warp: bool,
    pub vid_palettize: bool,
    pub polyblend: bool,
    pub menu: bool,
    pub ray_debug: bool,
    pub screenshot: bool,
    pub render_scale: u32,
    pub vid_width: u32,
    pub vid_height: u32,
    pub time: f32,
    pub color_clear_value: vk::ClearValue,
    pub v_blend: [u8; 4],
    pub origin: [f32; 3],
    pub forward: [f32; 3],
    pub right: [f32; 3],
    pub down: [f32; 3],
}

// `SCREEN_EFFECT_FLAG_*` (`gl_vidsdl.c`).
const SCREEN_EFFECT_FLAG_SCALE_2X: u32 = 0x1;
const SCREEN_EFFECT_FLAG_SCALE_4X: u32 = 0x2;
const SCREEN_EFFECT_FLAG_SCALE_8X: u32 = 0x3;
const SCREEN_EFFECT_FLAG_WATER_WARP: u32 = 0x4;
const SCREEN_EFFECT_FLAG_PALETTIZE: u32 = 0x8;
const SCREEN_EFFECT_FLAG_MENU: u32 = 0x10;

/// `screen_effect_constants_t` (all 4-byte fields: no padding).
#[repr(C)]
#[derive(Clone, Copy)]
struct ScreenEffectConstants {
    clamp_size_x: u32,
    clamp_size_y: u32,
    screen_size_rcp_x: f32,
    screen_size_rcp_y: f32,
    aspect_ratio: f32,
    time: f32,
    flags: u32,
    poly_blend: [f32; 4],
}

/// `ray_debug_constants_t` (all 4-byte fields: no padding).
#[cfg(feature = "engine-debug")]
#[repr(C)]
#[derive(Clone, Copy)]
struct RayDebugConstants {
    screen_size_rcp_x: f32,
    screen_size_rcp_y: f32,
    aspect_ratio: f32,
    origin: [f32; 3],
    forward: [f32; 3],
    right: [f32; 3],
    down: [f32; 3],
}

/// The bytes of a padding-free `repr(C)` push-constant block.
fn constants_bytes<T: Copy>(constants: &T) -> &[u8] {
    // SAFETY: the callers' structs are `repr(C)` with only `u32`/`f32`
    // fields, so every byte of the value is initialized; the slice borrows
    // `constants` for its lifetime.
    unsafe { slice::from_raw_parts((constants as *const T).cast::<u8>(), size_of::<T>()) }
}

/// The main-pass `(render_pass_index, subpass)` a secondary context in
/// `SCBX_WORLD..=SCBX_OIT_RESOLVE` records for, per the frame's OIT mode.
fn main_pass_subpass(scbx: c_int, mboit: bool, wboit: bool) -> (c_int, c_int) {
    if mboit {
        if scbx == SCBX_OIT_RESOLVE || scbx == SCBX_FTE_PARTICLES_BLEND {
            (RENDER_PASS_INDEX_MAIN_MBOIT, 3)
        } else if (SCBX_MBOIT_COMPOSITE_PASS_FIRST..=SCBX_MBOIT_COMPOSITE_PASS_LAST).contains(&scbx)
        {
            (RENDER_PASS_INDEX_MBOIT_COMPOSITE, 2)
        } else if scbx > SCBX_MAIN_OPAQUE_PASS_LAST && scbx <= SCBX_MAIN_PASS_LAST {
            (RENDER_PASS_INDEX_MBOIT_MOMENTS, 1)
        } else {
            (RENDER_PASS_INDEX_MAIN_MBOIT, 0)
        }
    } else if wboit {
        if scbx == SCBX_OIT_RESOLVE || scbx == SCBX_FTE_PARTICLES_BLEND {
            (RENDER_PASS_INDEX_MAIN_OIT, 2)
        } else if scbx > SCBX_MAIN_OPAQUE_PASS_LAST && scbx <= SCBX_MAIN_PASS_LAST {
            (RENDER_PASS_INDEX_WBOIT, 1)
        } else {
            (RENDER_PASS_INDEX_MAIN_OIT, 0)
        }
    } else {
        (RENDER_PASS_INDEX_MAIN, 0)
    }
}

/// `GL_BeginRenderingTask`: retires the frame that last used this
/// command-buffer set, collects garbage, and opens every primary and
/// secondary command buffer for recording.
pub fn begin_rendering_task<E: VidEngine>(ctx: &mut Ctx<'_, E>, vid: &mut VidState) {
    let engine = ctx.engine;
    let cur = vid.current_cb_index;
    let first_query = (cur * 2) as u32;

    if vid.frame_submitted[cur] {
        let wait_start = engine.double_time();
        let fences = [vid.command_buffer_fences[cur]];
        // SAFETY: the fence belongs to this device and was passed to the
        // `vkQueueSubmit` that set `frame_submitted`.
        if let Err(err) = unsafe { ctx.device.wait_for_fences(&fences, true, u64::MAX) } {
            ctx.vk_fail("vkWaitForFences", err);
        }
        engine.add_gpu_wait_us(((engine.double_time() - wait_start) * 1_000_000.0) as u32);
        // SAFETY: as above; the fence is signalled and not pending.
        if let Err(err) = unsafe { ctx.device.reset_fences(&fences) } {
            ctx.vk_fail("vkResetFences", err);
        }

        if vid.timestamp_query_pool != vk::QueryPool::null() && vid.timestamps_written[cur] {
            let mut timestamps = [0u64; 2];
            // SAFETY: queries `first_query .. first_query + 2` were written by
            // the frame the fence just retired; `timestamps` is the out-buffer
            // for the 64-bit results.
            let got = unsafe {
                ctx.device.get_query_pool_results(
                    vid.timestamp_query_pool,
                    first_query,
                    &mut timestamps,
                    vk::QueryResultFlags::TYPE_64,
                )
            };
            if got.is_ok() {
                let period = ctx.vg.device_properties.limits.timestamp_period as f64;
                engine.set_gpu_time_us(
                    (timestamps[1].wrapping_sub(timestamps[0]) as f64 * period / 1000.0) as u32,
                );
            }
        }
    }

    engine.dyn_buffers().collect_garbage(ctx);
    engine.collect_mesh_buffer_garbage();
    engine.collect_tlas_garbage();
    engine.texmgr_collect_garbage();

    let procs = CmdProcs::new(ctx.vg);

    for pcbx in 0..PCBX_NUM {
        let cb = vid.primary_command_buffers[pcbx][cur];
        let cbx = &mut ctx.vg.primary_cb_contexts[pcbx];
        cbx.cb = cb;
        cbx.current_canvas = CANVAS_INVALID;
        cbx.current_pipeline = VulkanPipeline::ZEROED;

        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        // SAFETY: `cb` came from `init_command_buffers`; its previous
        // submission retired with the fence above (or it was never submitted).
        if let Err(err) = unsafe { ctx.device.begin_command_buffer(cb, &begin_info) } {
            ctx.vk_fail("vkBeginCommandBuffer", err);
        }
        cb::begin_debug_utils_label(&procs, &ctx.vg.primary_cb_contexts[pcbx], c"Primary CB");
    }

    if vid.timestamp_query_pool != vk::QueryPool::null() {
        let first_cb = ctx.vg.primary_cb_contexts[0].cb;
        // SAFETY: `first_cb` is recording; the queries are this frame's pair.
        unsafe {
            ctx.device
                .cmd_reset_query_pool(first_cb, vid.timestamp_query_pool, first_query, 2);
            ctx.device.cmd_write_timestamp(
                first_cb,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vid.timestamp_query_pool,
                first_query,
            );
        }
    }

    let (vid_width, vid_height) = engine.vid_size();
    let mboit = use_mboit(engine);
    let wboit = use_wboit(engine);
    let oit = use_oit(engine);
    let variant = if mboit {
        MAIN_RENDER_PASS_MBOIT
    } else if wboit {
        MAIN_RENDER_PASS_OIT
    } else {
        MAIN_RENDER_PASS_STANDARD
    };
    let stencil = if engine.sky_need_stencil() {
        MAIN_RENDER_PASS_STENCIL_CLEAR
    } else {
        MAIN_RENDER_PASS_NO_STENCIL
    };
    let main_render_pass = ctx.vg.main_render_pass[variant][stencil];
    let scissor = vk::Rect2D {
        offset: vk::Offset2D { x: 0, y: 0 },
        extent: vk::Extent2D {
            width: vid_width,
            height: vid_height,
        },
    };
    let viewport = vk::Viewport {
        x: 0.0,
        y: 0.0,
        width: vid_width as f32,
        height: vid_height as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    };

    for (scbx, multiplicity) in SECONDARY_CB_MULTIPLICITY.iter().copied().enumerate() {
        let scbx_index = scbx as c_int;
        let (render_pass_index, subpass) = main_pass_subpass(scbx_index, mboit, wboit);
        for i in 0..multiplicity {
            let cb = vid.secondary_command_buffers[scbx][cur][i];
            // SAFETY: the secondary contexts are heap allocations distinct
            // from `vulkan_globals` (see `scbx_mut`); this task is the only
            // code touching them until the frame's recording tasks, which
            // depend on it, start.
            let cbx = unsafe { &mut *scbx_ptr(ctx.vg, scbx, i) };
            cbx.cb = cb;
            cbx.current_canvas = CANVAS_INVALID;
            cbx.current_pipeline = VulkanPipeline::ZEROED;
            if scbx_index <= SCBX_OIT_RESOLVE {
                cbx.render_pass = main_render_pass;
                cbx.render_pass_index = render_pass_index;
                cbx.subpass = subpass;
            }

            let inheritance_info = vk::CommandBufferInheritanceInfo::default()
                .render_pass(cbx.render_pass)
                .subpass(cbx.subpass as u32);
            let begin_info = vk::CommandBufferBeginInfo::default()
                .flags(
                    vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT
                        | vk::CommandBufferUsageFlags::RENDER_PASS_CONTINUE,
                )
                .inheritance_info(&inheritance_info);
            // SAFETY: as for the primary buffers; `inheritance_info` outlives
            // the call.
            if let Err(err) = unsafe { ctx.device.begin_command_buffer(cb, &begin_info) } {
                ctx.vk_fail("vkBeginCommandBuffer", err);
            }
            #[cfg(feature = "engine-debug")]
            {
                let name = std::ffi::CString::new(format!("CBX {scbx}")).expect("no NUL");
                cb::begin_debug_utils_label(&procs, cbx, &name);
            }

            // SAFETY: `cb` is recording; the scissor/viewport are locals.
            unsafe {
                ctx.device.cmd_set_scissor(cb, 0, &[scissor]);
                ctx.device.cmd_set_viewport(cb, 0, &[viewport]);
            }

            if scbx_index != SCBX_OIT_RESOLVE && !(scbx_index == SCBX_FTE_PARTICLES_BLEND && oit) {
                let pipeline = ctx.vg.basic_blend_pipeline[cbx.render_pass_index as usize];
                if pipeline.handle != vk::Pipeline::null() {
                    cb::bind_pipeline(&procs, cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
                    engine.set_canvas(cbx, CANVAS_NONE);
                }
            }
        }
    }

    engine.dyn_buffers().swap();
}

/// `VK_ERROR_FULL_SCREEN_EXCLUSIVE_MODE_LOST_EXT`, which C only tests where
/// `VK_EXT_full_screen_exclusive` is declared (`vulkan_win32.h`).
fn full_screen_exclusive_lost(err: vk::Result) -> bool {
    #[cfg(windows)]
    {
        err == vk::Result::ERROR_FULL_SCREEN_EXCLUSIVE_MODE_LOST_EXT
    }
    #[cfg(not(windows))]
    {
        let _ = err;
        false
    }
}

/// `GL_AcquireNextSwapChainImage`: `false` when no image is available (or
/// the swap chain needs recreating -- `vid.restart_next_frame` is raised).
pub fn acquire_next_swap_chain_image<E: VidEngine>(
    ctx: &mut Ctx<'_, E>,
    vid: &mut VidState,
) -> bool {
    let engine = ctx.engine;
    if vid.num_images_acquired >= vid.num_swap_chain_images - 1 {
        return false;
    }

    #[cfg(windows)]
    {
        let vg = &mut *ctx.vg;
        if engine.fullscreen()
            && vg.want_full_screen_exclusive
            && vg.swap_chain_full_screen_exclusive
            && !vg.swap_chain_full_screen_acquired
        {
            let acquire = vid.procs.acquire_full_screen_exclusive_mode.expect(
                "GL_InitDevice loads vkAcquireFullScreenExclusiveModeEXT with the extension",
            );
            // SAFETY: the swap chain was created with the exclusive-mode chain.
            if unsafe { acquire(vg.device, vid.swapchain) } == vk::Result::SUCCESS {
                vg.swap_chain_full_screen_acquired = true;
                engine.sys_printf("Full screen exclusive acquired\n");
            }
        } else if !vg.want_full_screen_exclusive
            && vg.swap_chain_full_screen_exclusive
            && vg.swap_chain_full_screen_acquired
        {
            let release = vid.procs.release_full_screen_exclusive_mode.expect(
                "GL_InitDevice loads vkReleaseFullScreenExclusiveModeEXT with the extension",
            );
            // SAFETY: as above; exclusive mode is currently acquired.
            if unsafe { release(vg.device, vid.swapchain) } == vk::Result::SUCCESS {
                vg.swap_chain_full_screen_acquired = false;
                engine.sys_printf("Full screen exclusive released\n");
            }
        }
    }

    let acquire_next_image = vid
        .procs
        .acquire_next_image
        .expect("GL_InitDevice loads vkAcquireNextImageKHR");
    // SAFETY: the swap chain and semaphore are live objects of this device;
    // the out-pointer is a `VidState` field.
    let err = unsafe {
        acquire_next_image(
            ctx.vg.device,
            vid.swapchain,
            u64::MAX,
            vid.image_aquired_semaphores[vid.current_cb_index],
            vk::Fence::null(),
            &mut vid.current_swapchain_buffer,
        )
    };
    if err == vk::Result::ERROR_OUT_OF_DATE_KHR
        || err == vk::Result::ERROR_SURFACE_LOST_KHR
        || full_screen_exclusive_lost(err)
    {
        engine.set_restart_next_frame();
        return false;
    } else if err == vk::Result::SUBOPTIMAL_KHR {
        engine.set_restart_next_frame();
    } else if err != vk::Result::SUCCESS {
        fail(engine, "Couldn't acquire next image", err);
    }

    vid.num_images_acquired += 1;
    true
}

/// `GL_ScreenEffects`: the post-scene compute pass over `color_buffers`
/// (or just the attachment barrier when it is disabled).
fn screen_effects<E: VidEngine>(
    ctx: &mut Ctx<'_, E>,
    procs: &CmdProcs,
    enabled: bool,
    parms: &EndRenderingParms,
) {
    let engine = ctx.engine;
    let cb = ctx.vg.primary_cb_contexts[PCBX_RENDER_PASSES].cb;

    if !enabled {
        let barrier = vk::MemoryBarrier::default()
            .src_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            )
            .dst_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            );
        // SAFETY: `cb` is recording outside a render pass.
        unsafe {
            ctx.device.cmd_pipeline_barrier(
                cb,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::DependencyFlags::empty(),
                &[barrier],
                &[],
                &[],
            )
        };
        return;
    }

    cb::begin_debug_utils_label(
        procs,
        &ctx.vg.primary_cb_contexts[PCBX_RENDER_PASSES],
        c"Screen Effects",
    );

    let subresource_range = vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    };
    let barriers = [
        vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_WRITE)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::GENERAL)
            .image(ctx.vg.color_buffers[0])
            .subresource_range(subresource_range),
        vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ)
            .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .image(ctx.vg.color_buffers[1])
            .subresource_range(subresource_range),
    ];
    // SAFETY: `cb` is recording outside a render pass; the images are the
    // live colour buffers.
    unsafe {
        ctx.device.cmd_pipeline_barrier(
            cb,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::PipelineStageFlags::COMPUTE_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &barriers,
        )
    };

    engine.set_canvas(
        &mut ctx.vg.primary_cb_contexts[PCBX_RENDER_PASSES],
        CANVAS_NONE,
    );

    let pipeline = if cfg!(feature = "engine-debug") && parms.ray_debug {
        ctx.vg.ray_debug_pipeline
    } else if parms.render_scale >= 2 {
        if ctx.vg.screen_effects_sops && engine.r_usesops() != 0.0 {
            ctx.vg.screen_effects_scale_sops_pipeline
        } else {
            ctx.vg.screen_effects_scale_pipeline
        }
    } else {
        ctx.vg.screen_effects_pipeline
    };
    cb::bind_pipeline(
        procs,
        &mut ctx.vg.primary_cb_contexts[PCBX_RENDER_PASSES],
        vk::PipelineBindPoint::COMPUTE,
        pipeline,
    );
    let cbx = &ctx.vg.primary_cb_contexts[PCBX_RENDER_PASSES];

    let width = parms.vid_width;
    let height = parms.vid_height;
    let screen_size_rcp_x = 1.0 / width as f32;
    let screen_size_rcp_y = 1.0 / height as f32;
    let aspect_ratio = width as f32 / height as f32;

    let ray_debug = cfg!(feature = "engine-debug")
        && parms.ray_debug
        && engine.bmodel_tlas() != vk::AccelerationStructureKHR::null();
    if !ray_debug {
        // SAFETY: `cb` is recording with `pipeline` bound; the set is live.
        unsafe {
            ctx.device.cmd_bind_descriptor_sets(
                cb,
                vk::PipelineBindPoint::COMPUTE,
                pipeline.layout.handle,
                0,
                &[ctx.vg.screen_effects_desc_set],
                &[],
            )
        };
        let mut flags = 0;
        if parms.render_warp {
            flags |= SCREEN_EFFECT_FLAG_WATER_WARP;
        }
        if parms.render_scale >= 8 {
            flags |= SCREEN_EFFECT_FLAG_SCALE_8X;
        } else if parms.render_scale >= 4 {
            flags |= SCREEN_EFFECT_FLAG_SCALE_4X;
        } else if parms.render_scale >= 2 {
            flags |= SCREEN_EFFECT_FLAG_SCALE_2X;
        }
        if parms.vid_palettize {
            flags |= SCREEN_EFFECT_FLAG_PALETTIZE;
        }
        if parms.menu {
            flags |= SCREEN_EFFECT_FLAG_MENU;
        }
        let push_constants = ScreenEffectConstants {
            clamp_size_x: width - 1,
            clamp_size_y: height - 1,
            screen_size_rcp_x,
            screen_size_rcp_y,
            aspect_ratio,
            time: parms.time,
            flags,
            poly_blend: [
                parms.v_blend[0] as f32 / 255.0,
                parms.v_blend[1] as f32 / 255.0,
                parms.v_blend[2] as f32 / 255.0,
                parms.v_blend[3] as f32 / 255.0,
            ],
        };
        cb::push_constants(
            procs,
            cbx,
            vk::ShaderStageFlags::COMPUTE,
            0,
            constants_bytes(&push_constants),
        );
    } else {
        #[cfg(feature = "engine-debug")]
        {
            // SAFETY: as above.
            unsafe {
                ctx.device.cmd_bind_descriptor_sets(
                    cb,
                    vk::PipelineBindPoint::COMPUTE,
                    pipeline.layout.handle,
                    0,
                    &[ctx.vg.ray_debug_desc_set],
                    &[],
                )
            };
            let bmodel_tlas = [engine.bmodel_tlas()];
            let mut tlas_info = vk::WriteDescriptorSetAccelerationStructureKHR::default()
                .acceleration_structures(&bmodel_tlas);
            let tlas_write = vk::WriteDescriptorSet::default()
                .push_next(&mut tlas_info)
                .dst_binding(0)
                .descriptor_count(1)
                .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR);
            let push_descriptor_set = ctx
                .vg
                .vk_cmd_push_descriptor_set
                .expect("ray debug requires VK_KHR_push_descriptor");
            // SAFETY: `cb` is recording with `pipeline` bound; the write and
            // its chain are locals that outlive the call.
            unsafe {
                push_descriptor_set(
                    cb,
                    vk::PipelineBindPoint::COMPUTE,
                    pipeline.layout.handle,
                    1,
                    1,
                    &tlas_write,
                )
            };
            let push_constants = RayDebugConstants {
                screen_size_rcp_x,
                screen_size_rcp_y,
                aspect_ratio,
                origin: parms.origin,
                forward: parms.forward,
                right: parms.right,
                down: parms.down,
            };
            cb::push_constants(
                procs,
                cbx,
                vk::ShaderStageFlags::COMPUTE,
                0,
                constants_bytes(&push_constants),
            );
        }
    }

    // SAFETY: `cb` is recording with the compute pipeline and its sets bound.
    unsafe {
        ctx.device
            .cmd_dispatch(cb, width.div_ceil(8), height.div_ceil(8), 1)
    };

    let barrier = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::SHADER_WRITE)
        .dst_access_mask(
            vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
        )
        .old_layout(vk::ImageLayout::GENERAL)
        .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        .image(ctx.vg.color_buffers[0])
        .subresource_range(subresource_range);
    // SAFETY: as for the barriers above.
    unsafe {
        ctx.device.cmd_pipeline_barrier(
            cb,
            vk::PipelineStageFlags::COMPUTE_SHADER,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier],
        )
    };

    cb::end_debug_utils_label(procs, cbx);
}

/// `ScheduleScreenshotCopy`: records a copy of the presented swap-chain image
/// into a fresh host-cached buffer.
fn schedule_screenshot_copy<E: VidEngine>(
    ctx: &Ctx<'_, E>,
    vid: &VidState,
    cb: vk::CommandBuffer,
    width: u32,
    height: u32,
) -> (vk::Buffer, VulkanMemory) {
    let mut memory = super::NULL_MEMORY;
    let (buffer, _) = create_buffer(
        ctx,
        &mut memory,
        u64::from(width) * u64::from(height) * 4,
        vk::BufferUsageFlags::TRANSFER_DST,
        vk::MemoryPropertyFlags::HOST_VISIBLE,
        vk::MemoryPropertyFlags::HOST_CACHED,
        None,
        false,
        "Screenshot",
    );

    let image = vid.swapchain_images[vid.current_swapchain_buffer as usize];
    let subresource_range = vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    };
    let to_transfer = vk::ImageMemoryBarrier::default()
        .src_access_mask(
            vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
        )
        .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
        .old_layout(vk::ImageLayout::PRESENT_SRC_KHR)
        .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
        .image(image)
        .subresource_range(subresource_range);
    // SAFETY: `cb` is recording outside a render pass; the swap-chain image
    // is the one this frame acquired.
    unsafe {
        ctx.device.cmd_pipeline_barrier(
            cb,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_transfer],
        )
    };

    let region = vk::BufferImageCopy {
        buffer_offset: 0,
        buffer_row_length: width,
        buffer_image_height: height,
        image_subresource: vk::ImageSubresourceLayers {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        },
        image_offset: vk::Offset3D { x: 0, y: 0, z: 0 },
        image_extent: vk::Extent3D {
            width,
            height,
            depth: 1,
        },
    };
    // SAFETY: as above; `buffer` holds `width * height * 4` bytes.
    unsafe {
        ctx.device.cmd_copy_image_to_buffer(
            cb,
            image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            buffer,
            &[region],
        )
    };

    let to_present = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::TRANSFER_READ)
        .dst_access_mask(vk::AccessFlags::empty())
        .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
        .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
        .image(image)
        .subresource_range(subresource_range);
    // SAFETY: as above.
    unsafe {
        ctx.device.cmd_pipeline_barrier(
            cb,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_present],
        )
    };

    (buffer, memory)
}

/// `WriteScreenshot`: waits for the copy, swizzles BGRA swap chains to RGBA
/// and hands the pixels to the engine's screenshot writer.
fn write_screenshot<E: VidEngine>(
    ctx: &mut Ctx<'_, E>,
    width: u32,
    height: u32,
    buffer: vk::Buffer,
    mut memory: VulkanMemory,
) {
    let engine = ctx.engine;
    // SAFETY: only this task submits work; C ignores the result too.
    let _ = unsafe { ctx.device.device_wait_idle() };
    ctx.vg.device_idle = true;

    let size = u64::from(width) * u64::from(height) * 4;
    // SAFETY: `memory` is the host-visible allocation `schedule_screenshot_copy`
    // made, of at least `size` bytes, and is not mapped.
    let mapped = match unsafe {
        ctx.device
            .map_memory(memory.handle, 0, size, vk::MemoryMapFlags::empty())
    } {
        Ok(ptr) => ptr,
        Err(err) => ctx.vk_fail("vkMapMemory", err),
    };
    let range = vk::MappedMemoryRange::default()
        .memory(memory.handle)
        .size(vk::WHOLE_SIZE);
    // SAFETY: `range` describes the mapping just made; C ignores the result.
    let _ = unsafe { ctx.device.invalidate_mapped_memory_ranges(&[range]) };

    // SAFETY: the mapping covers `size` bytes the GPU finished writing (the
    // device is idle) and stays mapped until the buffer is freed below.
    let pixels = unsafe { slice::from_raw_parts_mut(mapped.cast::<u8>(), size as usize) };
    let bgra = ctx.vg.swap_chain_format == vk::Format::B8G8R8A8_UNORM
        || ctx.vg.swap_chain_format == vk::Format::B8G8R8A8_SRGB;
    if bgra {
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    }
    engine.write_screenshot(pixels, width, height);

    // C frees the buffer without unmapping; freeing mapped memory is allowed.
    free_buffer(ctx, buffer, &mut memory, None);
}

/// `GL_SubmitContexts`: executes the secondary buffers of `first..=last`.
fn submit_contexts<E: VidEngine>(
    ctx: &Ctx<'_, E>,
    cb: vk::CommandBuffer,
    first: c_int,
    last: c_int,
) {
    for (scbx, multiplicity) in scbx_slots(first, last) {
        for i in 0..multiplicity {
            // SAFETY: see `begin_rendering_task`; the recording tasks this
            // task depends on have finished with the context.
            let secondary = unsafe { (*scbx_ptr(ctx.vg, scbx, i)).cb };
            // SAFETY: `cb` is inside a render pass begun with
            // `SECONDARY_COMMAND_BUFFERS`; `secondary` was ended by
            // `end_rendering_task`.
            unsafe { ctx.device.cmd_execute_commands(cb, &[secondary]) };
        }
    }
}

/// `GL_RecordOITResolveContext`: the full-screen resolve triangle in the
/// `SCBX_OIT_RESOLVE` context.
fn record_oit_resolve_context<E: VidEngine>(
    ctx: &Ctx<'_, E>,
    vid: &VidState,
    procs: &CmdProcs,
    parms: &EndRenderingParms,
    render_area: vk::Rect2D,
) {
    if !parms.use_oit {
        return;
    }
    // SAFETY: see `submit_contexts`; nothing else records into this context
    // once the recording tasks have finished.
    let cbx = unsafe { &mut *scbx_ptr(ctx.vg, SCBX_OIT_RESOLVE as usize, 0) };
    let viewport = vk::Viewport {
        x: 0.0,
        y: 0.0,
        width: parms.vid_width as f32,
        height: parms.vid_height as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    };
    // SAFETY: `cbx.cb` is recording.
    unsafe {
        ctx.device.cmd_set_scissor(cbx.cb, 0, &[render_area]);
        ctx.device.cmd_set_viewport(cbx.cb, 0, &[viewport]);
    }
    let (pipeline, descriptor_set) = if parms.use_mboit {
        (
            ctx.vg.mboit_resolve_pipeline,
            ctx.vg.mboit_input_attachment_descriptor_set,
        )
    } else {
        (
            ctx.vg.wboit_resolve_pipeline,
            vid.wboit_resolve_descriptor_set,
        )
    };
    cb::bind_pipeline(procs, cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
    // SAFETY: `cbx.cb` is recording with `pipeline` bound; the set is live.
    unsafe {
        ctx.device.cmd_bind_descriptor_sets(
            cbx.cb,
            vk::PipelineBindPoint::GRAPHICS,
            pipeline.layout.handle,
            0,
            &[descriptor_set],
            &[],
        );
        ctx.device.cmd_draw(cbx.cb, 3, 1, 0, 0);
    }
}

/// `GL_EndRenderingTask`: flushes the frame's uploads, acquires the swap-chain
/// image, closes every secondary buffer, records the render passes, screen
/// effects, GUI/post-process and optional screenshot copy into the primary
/// buffers, submits them and presents.
pub fn end_rendering_task<E: VidEngine>(
    ctx: &mut Ctx<'_, E>,
    vid: &mut VidState,
    parms: &EndRenderingParms,
) {
    let engine = ctx.engine;
    engine.staging().submit(ctx);
    engine
        .dyn_buffers()
        .flush(ctx, engine.frame_upload_buffers_memory());

    let cb_index = vid.current_cb_index;
    let render_area = vk::Rect2D {
        offset: vk::Offset2D { x: 0, y: 0 },
        extent: vk::Extent2D {
            width: parms.vid_width,
            height: parms.vid_height,
        },
    };

    let display_wait_start = engine.double_time();
    let max_frame_latency = u64::from((engine.vid_maxframelatency() as i32).clamp(1, 8) as u32);
    if vid.swapchain_present_wait
        && parms.swapchain
        && engine.vid_maxframelatency() > 0.0
        && engine.vid_vsync() > 0.0
        && vid.current_present_id + 1 > max_frame_latency
    {
        let wait_info = PresentWait2InfoKHR {
            s_type: STRUCTURE_TYPE_PRESENT_WAIT_2_INFO_KHR,
            p_next: core::ptr::null(),
            present_id: vid.current_present_id + 1 - max_frame_latency,
            timeout: 50_000_000,
        };
        let wait_for_present2 = vid
            .procs
            .wait_for_present2
            .expect("swapchain_present_wait implies vkWaitForPresent2KHR was loaded");
        // SAFETY: `wait_info` is a local; the result is ignored like C.
        let _ = unsafe { wait_for_present2(ctx.vg.device, vid.swapchain, &wait_info) };
    }

    let swapchain_acquired = parms.swapchain && acquire_next_swap_chain_image(ctx, vid);
    engine.add_gpu_wait_us(((engine.double_time() - display_wait_start) * 1_000_000.0) as u32);

    let procs = CmdProcs::new(ctx.vg);

    if swapchain_acquired {
        let (vid_width, vid_height) = engine.vid_size();
        // SAFETY: see `submit_contexts`.
        let cbx = unsafe { &mut *scbx_ptr(ctx.vg, SCBX_POST_PROCESS as usize, 0) };
        engine.viewport(cbx, 0.0, 0.0, vid_width as f32, vid_height as f32, 0.0, 1.0);
        let postprocess_values = [engine.vid_gamma(), engine.vid_contrast().clamp(1.0, 2.0)];
        let pipeline = ctx.vg.postprocess_pipeline;
        cb::bind_pipeline(&procs, cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
        // SAFETY: `cbx.cb` is recording with `pipeline` bound; the set is live.
        unsafe {
            ctx.device.cmd_bind_descriptor_sets(
                cbx.cb,
                vk::PipelineBindPoint::GRAPHICS,
                pipeline.layout.handle,
                0,
                &[vid.postprocess_descriptor_set],
                &[],
            )
        };
        cb::push_constants(
            &procs,
            cbx,
            vk::ShaderStageFlags::FRAGMENT,
            0,
            constants_bytes(&postprocess_values),
        );
        // SAFETY: as above.
        unsafe { ctx.device.cmd_draw(cbx.cb, 3, 1, 0, 0) };
    }

    record_oit_resolve_context(ctx, vid, &procs, parms, render_area);

    for (scbx, multiplicity) in SECONDARY_CB_MULTIPLICITY.iter().copied().enumerate() {
        for i in 0..multiplicity {
            // SAFETY: see `submit_contexts`.
            let cbx = unsafe { &*scbx_ptr(ctx.vg, scbx, i) };
            cb::end_debug_utils_label(&procs, cbx);
            // SAFETY: `cbx.cb` is recording.
            if let Err(err) = unsafe { ctx.device.end_command_buffer(cbx.cb) } {
                ctx.vk_fail("vkEndCommandBuffer", err);
            }
        }
    }

    let render_passes_cb = ctx.vg.primary_cb_contexts[PCBX_RENDER_PASSES].cb;

    let screen_effects_enabled = parms.render_warp
        || parms.render_scale >= 2
        || parms.vid_palettize
        || (parms.polyblend && parms.v_blend[3] != 0)
        || parms.menu
        || parms.ray_debug;
    let resolve = ctx.vg.sample_count != vk::SampleCountFlags::TYPE_1;
    let use_mboit = parms.use_mboit;
    let use_wboit = parms.use_oit && !use_mboit;
    let scene_color_index = if resolve { 2 } else { 0 };
    let accum_index = if resolve { 3 } else { 2 };
    let reveal_index = if resolve { 4 } else { 3 };

    let depth_clear_value = vk::ClearValue {
        depth_stencil: vk::ClearDepthStencilValue {
            depth: 0.0,
            stencil: 0,
        },
    };
    let zero_color = vk::ClearValue {
        color: vk::ClearColorValue { float32: [0.0; 4] },
    };
    let mut clear_values = [zero_color; 6];
    clear_values[0] = parms.color_clear_value;
    clear_values[1] = depth_clear_value;
    if resolve {
        clear_values[scene_color_index] = parms.color_clear_value;
    }
    if use_wboit {
        clear_values[accum_index] = zero_color;
        clear_values[reveal_index] = vk::ClearValue {
            color: vk::ClearColorValue { float32: [1.0; 4] },
        };
    } else if use_mboit {
        let (first, last) = if resolve { (3, 5) } else { (2, 4) };
        for value in &mut clear_values[first..=last] {
            *value = zero_color;
        }
    }
    let clear_value_count = if use_mboit {
        if resolve {
            6
        } else {
            5
        }
    } else if resolve {
        if use_wboit {
            5
        } else {
            3
        }
    } else if use_wboit {
        4
    } else {
        2
    };

    let variant = if use_mboit {
        MAIN_RENDER_PASS_MBOIT
    } else if use_wboit {
        MAIN_RENDER_PASS_OIT
    } else {
        MAIN_RENDER_PASS_STANDARD
    };
    let stencil = if engine.sky_need_stencil() {
        MAIN_RENDER_PASS_STENCIL_CLEAR
    } else {
        MAIN_RENDER_PASS_NO_STENCIL
    };
    let render_pass_begin_info = vk::RenderPassBeginInfo::default()
        .render_pass(ctx.vg.main_render_pass[variant][stencil])
        .framebuffer(vid.main_framebuffers[usize::from(screen_effects_enabled)])
        .render_area(render_area)
        .clear_values(&clear_values[..clear_value_count]);
    // SAFETY: `render_passes_cb` is recording outside a render pass; the
    // render pass, framebuffer and clear values are live for the call.
    unsafe {
        ctx.device.cmd_begin_render_pass(
            render_passes_cb,
            &render_pass_begin_info,
            vk::SubpassContents::SECONDARY_COMMAND_BUFFERS,
        )
    };

    let main_pass_last = if parms.use_oit {
        SCBX_VIEW_MODEL
    } else {
        SCBX_MAIN_PASS_LAST
    };
    submit_contexts(ctx, render_passes_cb, SCBX_WORLD, main_pass_last);
    if use_wboit {
        // SAFETY: the OIT render pass has the subpasses these advance through.
        unsafe {
            ctx.device.cmd_next_subpass(
                render_passes_cb,
                vk::SubpassContents::SECONDARY_COMMAND_BUFFERS,
            );
            submit_contexts(
                ctx,
                render_passes_cb,
                SCBX_ALPHA_ENTITIES_ACROSS_WATER,
                SCBX_MAIN_PASS_LAST,
            );
            ctx.device.cmd_next_subpass(
                render_passes_cb,
                vk::SubpassContents::SECONDARY_COMMAND_BUFFERS,
            );
        }
        submit_contexts(ctx, render_passes_cb, SCBX_OIT_RESOLVE, SCBX_OIT_RESOLVE);
        submit_contexts(
            ctx,
            render_passes_cb,
            SCBX_FTE_PARTICLES_BLEND,
            SCBX_FTE_PARTICLES_BLEND,
        );
    } else if use_mboit {
        // SAFETY: the MBOIT render pass has the subpasses these advance through.
        unsafe {
            ctx.device.cmd_next_subpass(
                render_passes_cb,
                vk::SubpassContents::SECONDARY_COMMAND_BUFFERS,
            );
            submit_contexts(
                ctx,
                render_passes_cb,
                SCBX_ALPHA_ENTITIES_ACROSS_WATER,
                SCBX_MAIN_PASS_LAST,
            );
            ctx.device.cmd_next_subpass(
                render_passes_cb,
                vk::SubpassContents::SECONDARY_COMMAND_BUFFERS,
            );
            submit_contexts(
                ctx,
                render_passes_cb,
                SCBX_MBOIT_COMPOSITE_PASS_FIRST,
                SCBX_MBOIT_COMPOSITE_PASS_LAST,
            );
            ctx.device.cmd_next_subpass(
                render_passes_cb,
                vk::SubpassContents::SECONDARY_COMMAND_BUFFERS,
            );
        }
        submit_contexts(ctx, render_passes_cb, SCBX_OIT_RESOLVE, SCBX_OIT_RESOLVE);
        submit_contexts(
            ctx,
            render_passes_cb,
            SCBX_FTE_PARTICLES_BLEND,
            SCBX_FTE_PARTICLES_BLEND,
        );
    }
    // SAFETY: the render pass begun above is at its last subpass.
    unsafe { ctx.device.cmd_end_render_pass(render_passes_cb) };

    screen_effects(ctx, &procs, screen_effects_enabled, parms);

    // SAFETY: see `submit_contexts`.
    let (gui_cb, gui_render_pass) = unsafe {
        let gui = &*scbx_ptr(ctx.vg, SCBX_GUI as usize, 0);
        (gui.cb, gui.render_pass)
    };
    // SAFETY: as above.
    let post_process_cb = unsafe { (*scbx_ptr(ctx.vg, SCBX_POST_PROCESS as usize, 0)).cb };
    let ui_render_pass_begin_info = vk::RenderPassBeginInfo::default()
        .render_pass(gui_render_pass)
        .framebuffer(vid.ui_framebuffers[vid.current_swapchain_buffer as usize])
        .render_area(render_area);
    // SAFETY: `render_passes_cb` is recording outside a render pass; the GUI
    // and post-process buffers were ended above.
    unsafe {
        ctx.device.cmd_begin_render_pass(
            render_passes_cb,
            &ui_render_pass_begin_info,
            vk::SubpassContents::SECONDARY_COMMAND_BUFFERS,
        );
        ctx.device.cmd_execute_commands(render_passes_cb, &[gui_cb]);
        ctx.device.cmd_next_subpass(
            render_passes_cb,
            vk::SubpassContents::SECONDARY_COMMAND_BUFFERS,
        );
        ctx.device
            .cmd_execute_commands(render_passes_cb, &[post_process_cb]);
        ctx.device.cmd_end_render_pass(render_passes_cb);
    }

    let screenshot = if parms.screenshot {
        Some(schedule_screenshot_copy(
            ctx,
            vid,
            render_passes_cb,
            parms.vid_width,
            parms.vid_height,
        ))
    } else {
        None
    };

    if vid.timestamp_query_pool != vk::QueryPool::null() {
        // SAFETY: `render_passes_cb` is recording outside a render pass.
        unsafe {
            ctx.device.cmd_write_timestamp(
                render_passes_cb,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vid.timestamp_query_pool,
                (cb_index * 2 + 1) as u32,
            )
        };
        vid.timestamps_written[cb_index] = true;
    }

    let mut submit_cbs = [vk::CommandBuffer::null(); PCBX_NUM];
    for (pcbx, submit_cb) in submit_cbs.iter_mut().enumerate() {
        let cbx = &ctx.vg.primary_cb_contexts[pcbx];
        *submit_cb = cbx.cb;
        cb::end_debug_utils_label(&procs, cbx);
        // SAFETY: `cbx.cb` is recording.
        if let Err(err) = unsafe { ctx.device.end_command_buffer(cbx.cb) } {
            ctx.vk_fail("vkEndCommandBuffer", err);
        }
    }

    let wait_semaphores = [vid.image_aquired_semaphores[cb_index]];
    let signal_semaphores = [vid.draw_complete_semaphores[vid.current_swapchain_buffer as usize]];
    let wait_dst_stage_mask = [vk::PipelineStageFlags::TOP_OF_PIPE];
    let semaphore_count = usize::from(swapchain_acquired);
    let submit_info = vk::SubmitInfo::default()
        .command_buffers(&submit_cbs)
        .wait_semaphores(&wait_semaphores[..semaphore_count])
        .signal_semaphores(&signal_semaphores[..semaphore_count])
        .wait_dst_stage_mask(&wait_dst_stage_mask[..semaphore_count]);
    // SAFETY: every buffer was ended above; the fence was reset by
    // `begin_rendering_task`; the semaphores are this frame's pair.
    if let Err(err) = unsafe {
        ctx.device.queue_submit(
            ctx.vg.queue,
            &[submit_info],
            vid.command_buffer_fences[cb_index],
        )
    } {
        ctx.vk_fail("vkQueueSubmit", err);
    }
    ctx.vg.device_idle = false;

    if let Some((buffer, memory)) = screenshot {
        if buffer != vk::Buffer::null() {
            write_screenshot(ctx, parms.vid_width, parms.vid_height, buffer, memory);
        }
    }

    if swapchain_acquired {
        let swapchains = [vid.swapchain];
        let image_indices = [vid.current_swapchain_buffer];
        let mut present_info = vk::PresentInfoKHR::default()
            .swapchains(&swapchains)
            .image_indices(&image_indices)
            .wait_semaphores(&signal_semaphores);
        let next_present_id = vid.current_present_id + 1;
        let mut present_id_info = PresentId2KHR {
            s_type: STRUCTURE_TYPE_PRESENT_ID_2_KHR,
            p_next: core::ptr::null(),
            swapchain_count: 1,
            p_present_ids: &next_present_id,
        };
        if vid.swapchain_present_wait {
            present_info = present_info.push_next(&mut present_id_info);
        }
        let queue_present = vid
            .procs
            .queue_present
            .expect("GL_InitDevice loads vkQueuePresentKHR");
        // SAFETY: `present_info` and everything it points at are locals that
        // outlive the call; the image was acquired this frame.
        let err = unsafe { queue_present(ctx.vg.queue, &present_info) };
        if vid.swapchain_present_wait {
            vid.current_present_id = next_present_id;
        }
        if err == vk::Result::ERROR_OUT_OF_DATE_KHR
            || err == vk::Result::ERROR_SURFACE_LOST_KHR
            || err == vk::Result::SUBOPTIMAL_KHR
            || full_screen_exclusive_lost(err)
        {
            engine.set_restart_next_frame();
        } else if err != vk::Result::SUCCESS {
            ctx.vk_fail("vkQueuePresentKHR", err);
        }
        if err == vk::Result::SUCCESS
            || err == vk::Result::ERROR_OUT_OF_DATE_KHR
            || err == vk::Result::ERROR_SURFACE_LOST_KHR
        {
            vid.num_images_acquired -= 1;
        }
    }

    vid.frame_submitted[cb_index] = true;
    vid.current_cb_index = (vid.current_cb_index + 1) % DOUBLE_BUFFERED;
}

/// `GL_WaitForDeviceIdle`: joins the in-flight end-rendering task, then
/// flushes staging and waits for the device (main thread only).
pub fn wait_for_device_idle<E: VidEngine>(ctx: &mut Ctx<'_, E>) {
    let engine = ctx.engine;
    engine.synchronize_end_rendering_task();
    if !ctx.vg.device_idle {
        engine.staging().submit(ctx);
        // SAFETY: nothing else is submitting; C ignores the result too.
        let _ = unsafe { ctx.device.device_wait_idle() };
    }
    ctx.vg.device_idle = true;
}
