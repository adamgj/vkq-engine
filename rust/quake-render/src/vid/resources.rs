//! The render-resource half of `gl_vidsdl.c`: render passes, the depth,
//! colour, MSAA and OIT attachments, the frame buffers, the per-resolution
//! descriptor sets, and `GL_CreateRenderResources`/`GL_DestroyRenderResources`
//! that sequence them around the swap chain (Phase 8 M6).

use core::ffi::CStr;

use ash::vk;
use quake_types::render::{VulkanMemory, VulkanMemoryType, NUM_COLOR_BUFFERS};

use super::swapchain::create_swap_chain;
use super::{
    scbx_mut, scbx_slots, use_mboit, use_oit, use_wboit, ImageSet, VidEngine, VidState,
    DOUBLE_BUFFERED, MAIN_RENDER_PASS_MBOIT, MAIN_RENDER_PASS_NO_STENCIL, MAIN_RENDER_PASS_OIT,
    MAIN_RENDER_PASS_STANDARD, MAIN_RENDER_PASS_STENCIL_CLEAR, NULL_MEMORY, RENDER_PASS_INDEX_MAIN,
    RENDER_PASS_INDEX_MAIN_OIT, RENDER_PASS_INDEX_UI, SCBX_GUI, SCBX_MAIN_PASS_LAST,
    SCBX_OIT_RESOLVE, SCBX_POST_PROCESS, SCBX_WORLD,
};
use crate::rmisc::descriptors::{allocate_descriptor_set, free_descriptor_set};
use crate::rmisc::memory::{
    allocate_vulkan_memory, c_string, create_buffers, free_vulkan_memory, BufferRequest,
};
use crate::rmisc::shaders::ShaderModules;
use crate::rmisc::{create_pipelines, destroy_pipelines, vg, vg_mut, Ctx, Staging};

const MAIN_RENDER_PASS_VARIANT_COUNT: usize = quake_types::render::MAIN_RENDER_PASS_VARIANT_COUNT;
const MAIN_RENDER_PASS_STENCIL_COUNT: usize = quake_types::render::MAIN_RENDER_PASS_STENCIL_COUNT;

fn resolve<E: VidEngine>(ctx: &Ctx<'_, E>) -> bool {
    vg!(ctx, sample_count) != vk::SampleCountFlags::TYPE_1
}

/// The image/memory/view triple every attachment in `gl_vidsdl.c` is built
/// from, in the C order: image, name, requirements, (dedicated) allocation,
/// name, bind, view, name.
#[allow(clippy::too_many_arguments)]
fn create_attachment<E: VidEngine>(
    ctx: &Ctx<'_, E>,
    format: vk::Format,
    samples: vk::SampleCountFlags,
    usage: vk::ImageUsageFlags,
    aspect: vk::ImageAspectFlags,
    image_name: &CStr,
    memory_name: &CStr,
    view_name: Option<&CStr>,
) -> ImageSet {
    let (width, height) = ctx.engine.vid_size();
    let image_create_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(format)
        .extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(samples)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(usage);
    // SAFETY: `image_create_info` is complete and refers to nothing on the stack.
    let image = match unsafe { ctx.device.create_image(&image_create_info, None) } {
        Ok(image) => image,
        Err(err) => ctx.vk_fail("vkCreateImage", err),
    };
    ctx.name_object(image, image_name);

    // SAFETY: `image` was just created on `ctx.device`.
    let memory_requirements = unsafe { ctx.device.get_image_memory_requirements(image) };
    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let mut memory_allocate_info = vk::MemoryAllocateInfo::default()
        .allocation_size(memory_requirements.size)
        .memory_type_index(ctx.memory_type_from_properties(
            memory_requirements.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            vk::MemoryPropertyFlags::empty(),
        ));
    if vg!(ctx, dedicated_allocation) {
        memory_allocate_info = memory_allocate_info.push_next(&mut dedicated);
    }
    let mut memory = NULL_MEMORY;
    allocate_vulkan_memory(
        ctx,
        &mut memory,
        &memory_allocate_info,
        VulkanMemoryType::Device,
        Some(ctx.counters.misc),
    );
    ctx.name_object(memory.handle, memory_name);

    // SAFETY: `image` and `memory.handle` are live and unbound; the memory
    // type was chosen from `image`'s requirements.
    if let Err(err) = unsafe { ctx.device.bind_image_memory(image, memory.handle, 0) } {
        ctx.vk_fail("vkBindImageMemory", err);
    }

    let view_create_info = vk::ImageViewCreateInfo::default()
        .image(image)
        .format(format)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: aspect,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        })
        .view_type(vk::ImageViewType::TYPE_2D);
    // SAFETY: `view_create_info` is complete and `image` is bound.
    let view = match unsafe { ctx.device.create_image_view(&view_create_info, None) } {
        Ok(view) => view,
        Err(err) => ctx.vk_fail("vkCreateImageView", err),
    };
    if let Some(view_name) = view_name {
        ctx.name_object(view, view_name);
    }
    ImageSet {
        image,
        memory,
        view,
    }
}

fn destroy_attachment<E: VidEngine>(ctx: &Ctx<'_, E>, set: &mut ImageSet) {
    // SAFETY: the handles came from `create_attachment` on `ctx.device` and
    // the caller has waited for the device to go idle.
    unsafe {
        ctx.device.destroy_image_view(set.view, None);
        ctx.device.destroy_image(set.image, None);
    }
    free_vulkan_memory(ctx, &mut set.memory, Some(ctx.counters.misc));
    set.view = vk::ImageView::null();
    set.image = vk::Image::null();
}

/// `GL_CreateRenderPasses`.
pub fn create_render_passes<E: VidEngine>(ctx: &mut Ctx<'_, E>) {
    ctx.engine.sys_printf("Creating render passes\n");

    let resolve = resolve(ctx);

    for (scbx_index, multiplicity) in scbx_slots(SCBX_WORLD, SCBX_OIT_RESOLVE) {
        for i in 0..multiplicity {
            debug_assert!(scbx_mut(ctx.vg, scbx_index, i).render_pass == vk::RenderPass::null());
        }
    }

    for variant in 0..MAIN_RENDER_PASS_VARIANT_COUNT {
        let use_wboit = variant == MAIN_RENDER_PASS_OIT;
        let use_mboit = variant == MAIN_RENDER_PASS_MBOIT;
        let use_oit = use_wboit || use_mboit;

        let scene_color_attachment_index = if resolve { 2 } else { 0 };
        let accum_index = if resolve { 3 } else { 2 };
        let reveal_index = if resolve { 4 } else { 3 };
        let mboit_b0_index = if resolve { 3 } else { 2 };
        let mboit_moments0_index = if resolve { 4 } else { 3 };
        let mboit_color_index = if resolve { 5 } else { 4 };

        let mut attachment_descriptions = [vk::AttachmentDescription::default(); 8];

        attachment_descriptions[0] = vk::AttachmentDescription {
            initial_layout: vk::ImageLayout::UNDEFINED,
            final_layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            samples: vk::SampleCountFlags::TYPE_1,
            format: vg!(ctx, color_format),
            load_op: if resolve {
                vk::AttachmentLoadOp::DONT_CARE
            } else {
                vk::AttachmentLoadOp::CLEAR
            },
            store_op: vk::AttachmentStoreOp::STORE,
            ..Default::default()
        };

        attachment_descriptions[1] = vk::AttachmentDescription {
            initial_layout: vk::ImageLayout::UNDEFINED,
            final_layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
            samples: vg!(ctx, sample_count),
            format: vg!(ctx, depth_format),
            load_op: vk::AttachmentLoadOp::CLEAR,
            store_op: if use_oit {
                vk::AttachmentStoreOp::STORE
            } else {
                vk::AttachmentStoreOp::DONT_CARE
            },
            stencil_load_op: vk::AttachmentLoadOp::CLEAR,
            stencil_store_op: vk::AttachmentStoreOp::DONT_CARE,
            ..Default::default()
        };

        if resolve {
            attachment_descriptions[scene_color_attachment_index] = vk::AttachmentDescription {
                initial_layout: vk::ImageLayout::UNDEFINED,
                final_layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                samples: vg!(ctx, sample_count),
                format: vg!(ctx, color_format),
                load_op: vk::AttachmentLoadOp::CLEAR,
                store_op: if use_oit {
                    vk::AttachmentStoreOp::DONT_CARE
                } else {
                    vk::AttachmentStoreOp::STORE
                },
                ..Default::default()
            };
        }

        let oit_attachment = |format: vk::Format| vk::AttachmentDescription {
            initial_layout: vk::ImageLayout::UNDEFINED,
            final_layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            samples: vg!(ctx, sample_count),
            format,
            load_op: vk::AttachmentLoadOp::CLEAR,
            store_op: vk::AttachmentStoreOp::DONT_CARE,
            ..Default::default()
        };

        if use_wboit {
            attachment_descriptions[accum_index] = oit_attachment(vk::Format::R16G16B16A16_SFLOAT);
            attachment_descriptions[reveal_index] = oit_attachment(vk::Format::R8_UNORM);
        } else if use_mboit {
            attachment_descriptions[mboit_b0_index] = oit_attachment(vk::Format::R32_SFLOAT);
            attachment_descriptions[mboit_moments0_index] =
                oit_attachment(vk::Format::R32G32B32A32_SFLOAT);
            attachment_descriptions[mboit_color_index] =
                oit_attachment(vk::Format::R16G16B16A16_SFLOAT);
        }

        let reference = |attachment: usize, layout: vk::ImageLayout| vk::AttachmentReference {
            attachment: attachment as u32,
            layout,
        };
        let scene_color_attachment_reference = reference(
            scene_color_attachment_index,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        );
        let depth_attachment_reference =
            reference(1, vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
        let resolve_attachment_reference = reference(0, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        let oit_input_attachment_references = [
            reference(accum_index, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL),
            reference(reveal_index, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL),
        ];
        let oit_accum_color_attachment_references = [
            reference(accum_index, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL),
            reference(reveal_index, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL),
        ];
        let mboit_color_attachment_reference =
            reference(mboit_color_index, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        let mboit_moment_color_attachment_references = [
            reference(mboit_b0_index, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL),
            reference(
                mboit_moments0_index,
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            ),
        ];
        let mboit_composite_input_attachment_references = [
            reference(mboit_b0_index, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL),
            reference(
                mboit_moments0_index,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            ),
        ];
        let mboit_resolve_input_attachment_references = [
            reference(mboit_b0_index, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL),
            reference(mboit_color_index, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL),
        ];
        let preserve_attachments = [scene_color_attachment_index as u32];

        let mut subpass_descriptions = [vk::SubpassDescription::default(); 4];
        subpass_descriptions[0] = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(core::slice::from_ref(&scene_color_attachment_reference))
            .depth_stencil_attachment(&depth_attachment_reference);
        if resolve && !use_oit {
            subpass_descriptions[0] = subpass_descriptions[0]
                .resolve_attachments(core::slice::from_ref(&resolve_attachment_reference));
        }

        if use_wboit {
            subpass_descriptions[1] = vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(&oit_accum_color_attachment_references)
                .depth_stencil_attachment(&depth_attachment_reference)
                .preserve_attachments(&preserve_attachments);

            subpass_descriptions[2] = vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(core::slice::from_ref(&scene_color_attachment_reference))
                .depth_stencil_attachment(&depth_attachment_reference)
                .input_attachments(&oit_input_attachment_references);
            if resolve {
                subpass_descriptions[2] = subpass_descriptions[2]
                    .resolve_attachments(core::slice::from_ref(&resolve_attachment_reference));
            }
        } else if use_mboit {
            subpass_descriptions[1] = vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(&mboit_moment_color_attachment_references)
                .depth_stencil_attachment(&depth_attachment_reference)
                .preserve_attachments(&preserve_attachments);

            subpass_descriptions[2] = vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(core::slice::from_ref(&mboit_color_attachment_reference))
                .depth_stencil_attachment(&depth_attachment_reference)
                .preserve_attachments(&preserve_attachments)
                .input_attachments(&mboit_composite_input_attachment_references);

            subpass_descriptions[3] = vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(core::slice::from_ref(&scene_color_attachment_reference))
                .depth_stencil_attachment(&depth_attachment_reference)
                .input_attachments(&mboit_resolve_input_attachment_references);
            if resolve {
                subpass_descriptions[3] = subpass_descriptions[3]
                    .resolve_attachments(core::slice::from_ref(&resolve_attachment_reference));
            }
        }

        const FRAG_TESTS_AND_COLOR: vk::PipelineStageFlags = vk::PipelineStageFlags::from_raw(
            vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS.as_raw()
                | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS.as_raw()
                | vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT.as_raw(),
        );
        const FRAG_TESTS_SHADER_AND_COLOR: vk::PipelineStageFlags =
            vk::PipelineStageFlags::from_raw(
                FRAG_TESTS_AND_COLOR.as_raw() | vk::PipelineStageFlags::FRAGMENT_SHADER.as_raw(),
            );
        let subpass_dependencies = [
            vk::SubpassDependency {
                src_subpass: vk::SUBPASS_EXTERNAL,
                dst_subpass: 0,
                src_stage_mask: FRAG_TESTS_AND_COLOR,
                dst_stage_mask: FRAG_TESTS_AND_COLOR,
                src_access_mask: vk::AccessFlags::COLOR_ATTACHMENT_READ
                    | vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
                dst_access_mask: vk::AccessFlags::COLOR_ATTACHMENT_READ
                    | vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
                dependency_flags: vk::DependencyFlags::BY_REGION,
            },
            vk::SubpassDependency {
                src_subpass: 0,
                dst_subpass: 1,
                src_stage_mask: FRAG_TESTS_AND_COLOR,
                dst_stage_mask: FRAG_TESTS_AND_COLOR,
                src_access_mask: vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE
                    | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                dst_access_mask: vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ
                    | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                dependency_flags: vk::DependencyFlags::BY_REGION,
            },
            vk::SubpassDependency {
                src_subpass: 1,
                dst_subpass: 2,
                src_stage_mask: FRAG_TESTS_AND_COLOR,
                dst_stage_mask: FRAG_TESTS_SHADER_AND_COLOR,
                src_access_mask: vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                dst_access_mask: vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ
                    | vk::AccessFlags::INPUT_ATTACHMENT_READ
                    | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                dependency_flags: vk::DependencyFlags::BY_REGION,
            },
            vk::SubpassDependency {
                src_subpass: 0,
                dst_subpass: 2,
                src_stage_mask: FRAG_TESTS_AND_COLOR,
                dst_stage_mask: FRAG_TESTS_AND_COLOR,
                src_access_mask: vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE
                    | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                dst_access_mask: vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ
                    | vk::AccessFlags::COLOR_ATTACHMENT_READ
                    | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                dependency_flags: vk::DependencyFlags::BY_REGION,
            },
            vk::SubpassDependency {
                src_subpass: 2,
                dst_subpass: 3,
                src_stage_mask: FRAG_TESTS_AND_COLOR,
                dst_stage_mask: FRAG_TESTS_SHADER_AND_COLOR,
                src_access_mask: vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                dst_access_mask: vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ
                    | vk::AccessFlags::INPUT_ATTACHMENT_READ
                    | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                dependency_flags: vk::DependencyFlags::BY_REGION,
            },
        ];

        let attachment_count = if use_mboit {
            if resolve {
                6
            } else {
                5
            }
        } else if use_wboit {
            if resolve {
                5
            } else {
                4
            }
        } else if resolve {
            3
        } else {
            2
        };
        let subpass_count = if use_mboit {
            4
        } else if use_wboit {
            3
        } else {
            1
        };
        let dependency_count = if use_mboit {
            5
        } else if use_wboit {
            4
        } else {
            1
        };

        for stencil in 0..MAIN_RENDER_PASS_STENCIL_COUNT {
            let name: &CStr = if use_wboit {
                if stencil == MAIN_RENDER_PASS_STENCIL_CLEAR {
                    c"main_oit"
                } else {
                    c"main_oit_no_stencil"
                }
            } else if use_mboit {
                if stencil == MAIN_RENDER_PASS_STENCIL_CLEAR {
                    c"main_mboit"
                } else {
                    c"main_mboit_no_stencil"
                }
            } else if stencil == MAIN_RENDER_PASS_STENCIL_CLEAR {
                c"main"
            } else {
                c"main_no_stencil"
            };
            if stencil == MAIN_RENDER_PASS_NO_STENCIL {
                attachment_descriptions[1].stencil_load_op = vk::AttachmentLoadOp::DONT_CARE;
            }

            let render_pass_create_info = vk::RenderPassCreateInfo::default()
                .attachments(&attachment_descriptions[..attachment_count])
                .subpasses(&subpass_descriptions[..subpass_count])
                .dependencies(&subpass_dependencies[..dependency_count]);
            // SAFETY: every pointer in `render_pass_create_info` refers to a
            // local that outlives the call.
            let render_pass = match unsafe {
                ctx.device
                    .create_render_pass(&render_pass_create_info, None)
            } {
                Ok(render_pass) => render_pass,
                Err(err) => ctx.engine.sys_error(&format!(
                    "Couldn't create Vulkan render pass with code {}",
                    err.as_raw()
                )),
            };
            *vg_mut!(ctx, main_render_pass[variant][stencil]) = render_pass;
            ctx.name_object(render_pass, name);
        }
    }

    for (scbx_index, multiplicity) in scbx_slots(SCBX_WORLD, SCBX_MAIN_PASS_LAST) {
        for i in 0..multiplicity {
            let render_pass = vg!(
                ctx,
                main_render_pass[MAIN_RENDER_PASS_STANDARD][MAIN_RENDER_PASS_STENCIL_CLEAR]
            );
            let cbx = scbx_mut(ctx.vg, scbx_index, i);
            cbx.render_pass = render_pass;
            cbx.render_pass_index = RENDER_PASS_INDEX_MAIN;
            cbx.subpass = 0;
        }
    }

    {
        let render_pass = vg!(
            ctx,
            main_render_pass[MAIN_RENDER_PASS_OIT][MAIN_RENDER_PASS_STENCIL_CLEAR]
        );
        let wboit_resolve_cbx = scbx_mut(ctx.vg, SCBX_OIT_RESOLVE as usize, 0);
        wboit_resolve_cbx.render_pass = render_pass;
        wboit_resolve_cbx.render_pass_index = RENDER_PASS_INDEX_MAIN_OIT;
        wboit_resolve_cbx.subpass = 2;
    }

    // UI Render Pass
    {
        let attachment_descriptions = [
            vk::AttachmentDescription {
                initial_layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                final_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                samples: vk::SampleCountFlags::TYPE_1,
                format: vg!(ctx, color_format),
                load_op: vk::AttachmentLoadOp::LOAD,
                store_op: vk::AttachmentStoreOp::DONT_CARE,
                ..Default::default()
            },
            vk::AttachmentDescription {
                initial_layout: vk::ImageLayout::UNDEFINED,
                final_layout: vk::ImageLayout::PRESENT_SRC_KHR,
                samples: vk::SampleCountFlags::TYPE_1,
                format: vg!(ctx, swap_chain_format),
                load_op: vk::AttachmentLoadOp::DONT_CARE,
                store_op: vk::AttachmentStoreOp::STORE,
                ..Default::default()
            },
        ];

        let color_input_attachment_reference = vk::AttachmentReference {
            attachment: 0,
            layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        };
        let ui_color_attachment_reference = vk::AttachmentReference {
            attachment: 0,
            layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        };
        let swap_chain_attachment_reference = vk::AttachmentReference {
            attachment: 1,
            layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        };

        let subpass_descriptions = [
            vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(core::slice::from_ref(&ui_color_attachment_reference)),
            vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(core::slice::from_ref(&swap_chain_attachment_reference))
                .input_attachments(core::slice::from_ref(&color_input_attachment_reference)),
        ];

        let subpass_dependencies = [vk::SubpassDependency {
            src_subpass: 0,
            dst_subpass: 1,
            src_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            dst_stage_mask: vk::PipelineStageFlags::FRAGMENT_SHADER
                | vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            src_access_mask: vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            dst_access_mask: vk::AccessFlags::INPUT_ATTACHMENT_READ
                | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            dependency_flags: vk::DependencyFlags::BY_REGION,
        }];

        let render_pass_create_info = vk::RenderPassCreateInfo::default()
            .attachments(&attachment_descriptions)
            .subpasses(&subpass_descriptions)
            .dependencies(&subpass_dependencies);

        let gui_render_pass = scbx_mut(ctx.vg, SCBX_GUI as usize, 0).render_pass;
        let post_process_render_pass = scbx_mut(ctx.vg, SCBX_POST_PROCESS as usize, 0).render_pass;
        if gui_render_pass != vk::RenderPass::null()
            || post_process_render_pass != vk::RenderPass::null()
        {
            debug_assert!(gui_render_pass == post_process_render_pass);
            debug_assert!(
                scbx_mut(ctx.vg, SCBX_GUI as usize, 0).render_pass_index == RENDER_PASS_INDEX_UI
            );
            debug_assert!(scbx_mut(ctx.vg, SCBX_GUI as usize, 0).subpass == 0);
            debug_assert!(
                scbx_mut(ctx.vg, SCBX_POST_PROCESS as usize, 0).render_pass_index
                    == RENDER_PASS_INDEX_UI
            );
            debug_assert!(scbx_mut(ctx.vg, SCBX_POST_PROCESS as usize, 0).subpass == 1);
        } else {
            // SAFETY: as for the main pass above.
            let render_pass = match unsafe {
                ctx.device
                    .create_render_pass(&render_pass_create_info, None)
            } {
                Ok(render_pass) => render_pass,
                Err(err) => ctx.engine.sys_error(&format!(
                    "Couldn't create Vulkan render pass with code {}",
                    err.as_raw()
                )),
            };
            ctx.name_object(render_pass, c"ui");
            let gui_cbx = scbx_mut(ctx.vg, SCBX_GUI as usize, 0);
            gui_cbx.render_pass = render_pass;
            gui_cbx.render_pass_index = RENDER_PASS_INDEX_UI;
            gui_cbx.subpass = 0;
            let post_process_cbx = scbx_mut(ctx.vg, SCBX_POST_PROCESS as usize, 0);
            post_process_cbx.render_pass = render_pass;
            post_process_cbx.render_pass_index = RENDER_PASS_INDEX_UI;
            post_process_cbx.subpass = 1;
        }
    }

    // Warp render pass
    if vg!(ctx, warp_render_pass) == vk::RenderPass::null() {
        let attachment_description = vk::AttachmentDescription {
            format: vk::Format::R8G8B8A8_UNORM,
            load_op: vk::AttachmentLoadOp::DONT_CARE,
            store_op: vk::AttachmentStoreOp::STORE,
            initial_layout: vk::ImageLayout::UNDEFINED,
            final_layout: vk::ImageLayout::GENERAL,
            samples: vk::SampleCountFlags::TYPE_1,
            ..Default::default()
        };

        let color_attachment_reference = vk::AttachmentReference {
            attachment: 0,
            layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        };

        let subpass_description = vk::SubpassDescription::default()
            .color_attachments(core::slice::from_ref(&color_attachment_reference))
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS);

        let subpass_dependencies = [
            vk::SubpassDependency {
                src_subpass: vk::SUBPASS_EXTERNAL,
                dst_subpass: 0,
                src_stage_mask: vk::PipelineStageFlags::FRAGMENT_SHADER,
                dst_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                src_access_mask: vk::AccessFlags::SHADER_READ,
                dst_access_mask: vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                dependency_flags: vk::DependencyFlags::BY_REGION,
            },
            vk::SubpassDependency {
                src_subpass: 0,
                dst_subpass: vk::SUBPASS_EXTERNAL,
                src_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                dst_stage_mask: vk::PipelineStageFlags::TRANSFER,
                src_access_mask: vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                dst_access_mask: vk::AccessFlags::TRANSFER_READ,
                dependency_flags: vk::DependencyFlags::BY_REGION,
            },
        ];

        let render_pass_create_info = vk::RenderPassCreateInfo::default()
            .attachments(core::slice::from_ref(&attachment_description))
            .subpasses(core::slice::from_ref(&subpass_description))
            .dependencies(&subpass_dependencies);

        // SAFETY: as for the main pass above.
        *vg_mut!(ctx, warp_render_pass) = match unsafe {
            ctx.device
                .create_render_pass(&render_pass_create_info, None)
        } {
            Ok(render_pass) => render_pass,
            Err(err) => ctx.engine.sys_error(&format!(
                "Couldn't create Vulkan render pass with code {}",
                err.as_raw()
            )),
        };
        ctx.name_object(vg!(ctx, warp_render_pass), c"warp");
    }
}

/// `GL_CreateDepthBuffer`.
pub fn create_depth_buffer<E: VidEngine>(ctx: &mut Ctx<'_, E>, vid: &mut VidState) {
    ctx.engine.sys_printf("Creating depth buffer\n");

    if vid.depth_buffer.image != vk::Image::null() {
        return;
    }

    vid.depth_buffer = create_attachment(
        ctx,
        vg!(ctx, depth_format),
        vg!(ctx, sample_count),
        vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
        vk::ImageAspectFlags::DEPTH,
        c"Depth Buffer",
        c"Depth Buffer",
        Some(c"Depth Buffer View"),
    );
}

/// `GL_CreateColorBuffer`: the two colour buffers, the MSAA sample-count
/// decision, the MSAA colour buffer, then the OIT buffers when OIT is on.
pub fn create_color_buffer<E: VidEngine>(ctx: &mut Ctx<'_, E>, vid: &mut VidState) {
    ctx.engine.sys_printf("Creating color buffer\n");

    let usage = vk::ImageUsageFlags::COLOR_ATTACHMENT
        | vk::ImageUsageFlags::INPUT_ATTACHMENT
        | vk::ImageUsageFlags::SAMPLED
        | vk::ImageUsageFlags::STORAGE;

    for i in 0..NUM_COLOR_BUFFERS {
        let name = c_string(&format!("Color Buffer {i}"));
        let view_name = c_string(&format!("Color Buffer View {i}"));
        let set = create_attachment(
            ctx,
            vg!(ctx, color_format),
            vk::SampleCountFlags::TYPE_1,
            usage,
            vk::ImageAspectFlags::COLOR,
            &name,
            &name,
            Some(&view_name),
        );
        *vg_mut!(ctx, color_buffers[i]) = set.image;
        vid.color_buffers_memory[i] = set.memory;
        vid.color_buffers_view[i] = set.view;
    }

    *vg_mut!(ctx, sample_count) = vk::SampleCountFlags::TYPE_1;
    *vg_mut!(ctx, supersampling) = false;

    let fsaa = ctx.engine.vid_fsaa() as i32;

    let color_format = vg!(ctx, color_format);
    // SAFETY: `vid.physical_device` is the device `init_device` selected on
    // this instance.
    let image_format_properties = unsafe {
        vid.instance().get_physical_device_image_format_properties(
            vid.physical_device,
            color_format,
            vk::ImageType::TYPE_2D,
            vk::ImageTiling::OPTIMAL,
            usage,
            vk::ImageCreateFlags::empty(),
        )
    }
    .unwrap_or_default();

    // Workaround: Intel advertises 16 samples but crashes when using it.
    let sample_counts = image_format_properties.sample_counts;
    if fsaa >= 16
        && sample_counts.contains(vk::SampleCountFlags::TYPE_16)
        && vg!(ctx, device_properties.vendor_id) != 0x8086
    {
        *vg_mut!(ctx, sample_count) = vk::SampleCountFlags::TYPE_16;
    } else if fsaa >= 8 && sample_counts.contains(vk::SampleCountFlags::TYPE_8) {
        *vg_mut!(ctx, sample_count) = vk::SampleCountFlags::TYPE_8;
    } else if fsaa >= 4 && sample_counts.contains(vk::SampleCountFlags::TYPE_4) {
        *vg_mut!(ctx, sample_count) = vk::SampleCountFlags::TYPE_4;
    } else if fsaa >= 2 && sample_counts.contains(vk::SampleCountFlags::TYPE_2) {
        *vg_mut!(ctx, sample_count) = vk::SampleCountFlags::TYPE_2;
    }

    match vg!(ctx, sample_count) {
        vk::SampleCountFlags::TYPE_2 => ctx.engine.sys_printf("2 AA Samples\n"),
        vk::SampleCountFlags::TYPE_4 => ctx.engine.sys_printf("4 AA Samples\n"),
        vk::SampleCountFlags::TYPE_8 => ctx.engine.sys_printf("8 AA Samples\n"),
        vk::SampleCountFlags::TYPE_16 => ctx.engine.sys_printf("16 AA Samples\n"),
        _ => {}
    }

    if vg!(ctx, sample_count) != vk::SampleCountFlags::TYPE_1 {
        *vg_mut!(ctx, supersampling) = vg!(ctx, device_features.sample_rate_shading) != vk::FALSE
            && ctx.engine.vid_fsaamode() >= 1.0;

        if vg!(ctx, supersampling) {
            ctx.engine.sys_printf("Supersampling enabled\n");
        }

        vid.msaa_color_buffer = create_attachment(
            ctx,
            vg!(ctx, color_format),
            vg!(ctx, sample_count),
            vk::ImageUsageFlags::COLOR_ATTACHMENT,
            vk::ImageAspectFlags::COLOR,
            c"MSAA Color Buffer",
            c"MSAA Color Buffer",
            None,
        );
    } else {
        ctx.engine.sys_printf("AA disabled\n");
    }

    if use_oit(ctx.engine) {
        create_oit_buffers(ctx, vid);
    }
}

/// `GL_CreateOITImage`.
fn create_oit_image<E: VidEngine>(ctx: &Ctx<'_, E>, format: vk::Format, name: &str) -> ImageSet {
    let image_name = c_string(name);
    let view_name = c_string(&format!("{name} View"));
    create_attachment(
        ctx,
        format,
        vg!(ctx, sample_count),
        vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::INPUT_ATTACHMENT,
        vk::ImageAspectFlags::COLOR,
        &image_name,
        &image_name,
        Some(&view_name),
    )
}

/// `GL_CreateOITBuffers`.
fn create_oit_buffers<E: VidEngine>(ctx: &mut Ctx<'_, E>, vid: &mut VidState) {
    if use_wboit(ctx.engine) {
        let accum = create_oit_image(ctx, vk::Format::R16G16B16A16_SFLOAT, "OIT Accum Buffer");
        *vg_mut!(ctx, oit_accum_buffer) = accum.image;
        vid.oit_accum_buffer_memory = accum.memory;
        vid.oit_accum_buffer_view = accum.view;

        let reveal = create_oit_image(ctx, vk::Format::R8_UNORM, "OIT Reveal Buffer");
        *vg_mut!(ctx, oit_reveal_buffer) = reveal.image;
        vid.oit_reveal_buffer_memory = reveal.memory;
        vid.oit_reveal_buffer_view = reveal.view;
    }

    if use_mboit(ctx.engine) {
        let b0 = create_oit_image(ctx, vk::Format::R32_SFLOAT, "MBOIT B0 Buffer");
        *vg_mut!(ctx, mboit_b0_buffer) = b0.image;
        vid.mboit_b0_buffer_memory = b0.memory;
        vid.mboit_b0_buffer_view = b0.view;

        let moments0 = create_oit_image(
            ctx,
            vk::Format::R32G32B32A32_SFLOAT,
            "MBOIT Moments 0 Buffer",
        );
        *vg_mut!(ctx, mboit_moments0_buffer) = moments0.image;
        vid.mboit_moments0_buffer_memory = moments0.memory;
        vid.mboit_moments0_buffer_view = moments0.view;

        let color = create_oit_image(ctx, vk::Format::R16G16B16A16_SFLOAT, "MBOIT Color Buffer");
        *vg_mut!(ctx, mboit_color_buffer) = color.image;
        vid.mboit_color_buffer_memory = color.memory;
        vid.mboit_color_buffer_view = color.view;
    }
}

fn destroy_oit_image<E: VidEngine>(
    ctx: &Ctx<'_, E>,
    view: &mut vk::ImageView,
    image: &mut vk::Image,
    memory: &mut VulkanMemory,
) {
    if *view != vk::ImageView::null() {
        // SAFETY: the view came from `create_oit_image` on `ctx.device` and
        // the device is idle.
        unsafe { ctx.device.destroy_image_view(*view, None) };
        *view = vk::ImageView::null();
    }
    if *image != vk::Image::null() {
        // SAFETY: as above.
        unsafe { ctx.device.destroy_image(*image, None) };
        *image = vk::Image::null();
    }
    if memory.handle != vk::DeviceMemory::null() {
        free_vulkan_memory(ctx, memory, Some(ctx.counters.misc));
    }
}

/// `GL_DestroyOITBuffers`.
fn destroy_oit_buffers<E: VidEngine>(ctx: &mut Ctx<'_, E>, vid: &mut VidState) {
    let mut accum = vg!(ctx, oit_accum_buffer);
    destroy_oit_image(
        ctx,
        &mut vid.oit_accum_buffer_view,
        &mut accum,
        &mut vid.oit_accum_buffer_memory,
    );
    *vg_mut!(ctx, oit_accum_buffer) = accum;

    let mut reveal = vg!(ctx, oit_reveal_buffer);
    destroy_oit_image(
        ctx,
        &mut vid.oit_reveal_buffer_view,
        &mut reveal,
        &mut vid.oit_reveal_buffer_memory,
    );
    *vg_mut!(ctx, oit_reveal_buffer) = reveal;

    let mut b0 = vg!(ctx, mboit_b0_buffer);
    destroy_oit_image(
        ctx,
        &mut vid.mboit_b0_buffer_view,
        &mut b0,
        &mut vid.mboit_b0_buffer_memory,
    );
    *vg_mut!(ctx, mboit_b0_buffer) = b0;

    let mut moments0 = vg!(ctx, mboit_moments0_buffer);
    destroy_oit_image(
        ctx,
        &mut vid.mboit_moments0_buffer_view,
        &mut moments0,
        &mut vid.mboit_moments0_buffer_memory,
    );
    *vg_mut!(ctx, mboit_moments0_buffer) = moments0;

    let mut color = vg!(ctx, mboit_color_buffer);
    destroy_oit_image(
        ctx,
        &mut vid.mboit_color_buffer_view,
        &mut color,
        &mut vid.mboit_color_buffer_memory,
    );
    *vg_mut!(ctx, mboit_color_buffer) = color;
}

/// `GL_UpdateDescriptorSets`: the post-process, OIT-resolve, screen-effects
/// (and `_DEBUG` ray-debug) descriptor sets over the current attachments.
pub fn update_descriptor_sets<E: VidEngine>(ctx: &mut Ctx<'_, E>, vid: &mut VidState) {
    if !vid.render_resources_created {
        return;
    }

    super::frame::wait_for_device_idle(ctx);

    if vid.postprocess_descriptor_set != vk::DescriptorSet::null() {
        free_descriptor_set(
            ctx,
            vid.postprocess_descriptor_set,
            &vg!(ctx, input_attachment_set_layout),
        );
    }
    vid.postprocess_descriptor_set =
        allocate_descriptor_set(ctx, &vg!(ctx, input_attachment_set_layout));

    let postprocess_image_info = vk::DescriptorImageInfo {
        sampler: vk::Sampler::null(),
        image_view: vid.color_buffers_view[0],
        image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
    };
    let postprocess_writes = [vk::WriteDescriptorSet::default()
        .dst_binding(0)
        .dst_array_element(0)
        .descriptor_type(vk::DescriptorType::INPUT_ATTACHMENT)
        .dst_set(vid.postprocess_descriptor_set)
        .image_info(core::slice::from_ref(&postprocess_image_info))];
    // SAFETY: the write refers to live locals and a set allocated above.
    unsafe { ctx.device.update_descriptor_sets(&postprocess_writes, &[]) };

    if vid.wboit_resolve_descriptor_set != vk::DescriptorSet::null() {
        free_descriptor_set(
            ctx,
            vid.wboit_resolve_descriptor_set,
            &vg!(ctx, oit_input_attachment_set_layout),
        );
        vid.wboit_resolve_descriptor_set = vk::DescriptorSet::null();
    }
    if vg!(ctx, mboit_input_attachment_descriptor_set) != vk::DescriptorSet::null() {
        free_descriptor_set(
            ctx,
            vg!(ctx, mboit_input_attachment_descriptor_set),
            &vg!(ctx, mboit_input_attachment_set_layout),
        );
        *vg_mut!(ctx, mboit_input_attachment_descriptor_set) = vk::DescriptorSet::null();
    }

    if use_wboit(ctx.engine) {
        vid.wboit_resolve_descriptor_set =
            allocate_descriptor_set(ctx, &vg!(ctx, oit_input_attachment_set_layout));

        let image_infos = [
            vk::DescriptorImageInfo {
                sampler: vk::Sampler::null(),
                image_view: vid.oit_accum_buffer_view,
                image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            },
            vk::DescriptorImageInfo {
                sampler: vk::Sampler::null(),
                image_view: vid.oit_reveal_buffer_view,
                image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            },
        ];
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_binding(0)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::INPUT_ATTACHMENT)
                .dst_set(vid.wboit_resolve_descriptor_set)
                .image_info(core::slice::from_ref(&image_infos[0])),
            vk::WriteDescriptorSet::default()
                .dst_binding(1)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::INPUT_ATTACHMENT)
                .dst_set(vid.wboit_resolve_descriptor_set)
                .image_info(core::slice::from_ref(&image_infos[1])),
        ];
        // SAFETY: as above.
        unsafe { ctx.device.update_descriptor_sets(&writes, &[]) };
    } else if use_mboit(ctx.engine) {
        *vg_mut!(ctx, mboit_input_attachment_descriptor_set) =
            allocate_descriptor_set(ctx, &vg!(ctx, mboit_input_attachment_set_layout));

        let image_infos = [
            vk::DescriptorImageInfo {
                sampler: vk::Sampler::null(),
                image_view: vid.mboit_b0_buffer_view,
                image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            },
            vk::DescriptorImageInfo {
                sampler: vk::Sampler::null(),
                image_view: vid.mboit_moments0_buffer_view,
                image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            },
            vk::DescriptorImageInfo {
                sampler: vk::Sampler::null(),
                image_view: vid.mboit_color_buffer_view,
                image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            },
        ];
        let set = vg!(ctx, mboit_input_attachment_descriptor_set);
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_binding(0)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::INPUT_ATTACHMENT)
                .dst_set(set)
                .image_info(core::slice::from_ref(&image_infos[0])),
            vk::WriteDescriptorSet::default()
                .dst_binding(1)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::INPUT_ATTACHMENT)
                .dst_set(set)
                .image_info(core::slice::from_ref(&image_infos[1])),
            vk::WriteDescriptorSet::default()
                .dst_binding(2)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::INPUT_ATTACHMENT)
                .dst_set(set)
                .image_info(core::slice::from_ref(&image_infos[2])),
        ];
        // SAFETY: as above.
        unsafe { ctx.device.update_descriptor_sets(&writes, &[]) };
    }

    if vg!(ctx, screen_effects_desc_set) != vk::DescriptorSet::null() {
        free_descriptor_set(
            ctx,
            vg!(ctx, screen_effects_desc_set),
            &vg!(ctx, screen_effects_set_layout),
        );
    }
    *vg_mut!(ctx, screen_effects_desc_set) =
        allocate_descriptor_set(ctx, &vg!(ctx, screen_effects_set_layout));

    let input_image_info = vk::DescriptorImageInfo {
        image_view: vid.color_buffers_view[1],
        image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        sampler: vg!(ctx, linear_sampler),
    };
    let output_image_info = vk::DescriptorImageInfo {
        image_view: vid.color_buffers_view[0],
        image_layout: vk::ImageLayout::GENERAL,
        sampler: vk::Sampler::null(),
    };
    let palette_octree_info = vk::DescriptorBufferInfo {
        buffer: vid.palette_octree_buffer,
        offset: 0,
        range: vk::WHOLE_SIZE,
    };
    let blue_noise_image_info = vk::DescriptorImageInfo {
        image_view: ctx.engine.bluenoise_image_view(),
        image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        sampler: vg!(ctx, linear_sampler),
    };
    let palette_buffer_view = [vid.palette_buffer_view];

    let set = vg!(ctx, screen_effects_desc_set);
    let screen_effects_writes = [
        vk::WriteDescriptorSet::default()
            .dst_binding(0)
            .dst_array_element(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .dst_set(set)
            .image_info(core::slice::from_ref(&input_image_info)),
        vk::WriteDescriptorSet::default()
            .dst_binding(1)
            .dst_array_element(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .dst_set(set)
            .image_info(core::slice::from_ref(&blue_noise_image_info)),
        vk::WriteDescriptorSet::default()
            .dst_binding(2)
            .dst_array_element(0)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .dst_set(set)
            .image_info(core::slice::from_ref(&output_image_info)),
        vk::WriteDescriptorSet::default()
            .dst_binding(3)
            .dst_array_element(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_TEXEL_BUFFER)
            .dst_set(set)
            .texel_buffer_view(&palette_buffer_view),
        vk::WriteDescriptorSet::default()
            .dst_binding(4)
            .dst_array_element(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .dst_set(set)
            .buffer_info(core::slice::from_ref(&palette_octree_info)),
    ];
    // SAFETY: as above.
    unsafe {
        ctx.device
            .update_descriptor_sets(&screen_effects_writes, &[])
    };

    #[cfg(feature = "engine-debug")]
    if vg!(ctx, ray_query) {
        if vg!(ctx, ray_debug_desc_set) != vk::DescriptorSet::null() {
            free_descriptor_set(
                ctx,
                vg!(ctx, ray_debug_desc_set),
                &vg!(ctx, ray_debug_set_layout),
            );
        }
        *vg_mut!(ctx, ray_debug_desc_set) =
            allocate_descriptor_set(ctx, &vg!(ctx, ray_debug_set_layout));

        let ray_debug_writes = [vk::WriteDescriptorSet::default()
            .dst_binding(0)
            .dst_array_element(0)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .dst_set(vg!(ctx, ray_debug_desc_set))
            .image_info(core::slice::from_ref(&output_image_info))];
        // SAFETY: as above.
        unsafe { ctx.device.update_descriptor_sets(&ray_debug_writes, &[]) };
    }
}

/// `GL_CreateMainFrameBuffers`.
fn create_main_frame_buffers<E: VidEngine>(ctx: &mut Ctx<'_, E>, vid: &mut VidState) {
    let resolve = resolve(ctx);
    let use_wboit = use_wboit(ctx.engine);
    let use_mboit = use_mboit(ctx.engine);
    let (width, height) = ctx.engine.vid_size();

    for i in 0..NUM_COLOR_BUFFERS {
        let variant = if use_mboit {
            MAIN_RENDER_PASS_MBOIT
        } else if use_wboit {
            MAIN_RENDER_PASS_OIT
        } else {
            MAIN_RENDER_PASS_STANDARD
        };
        let attachment_count = if use_mboit {
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

        let mut attachments = [
            vid.color_buffers_view[i],
            vid.depth_buffer.view,
            vid.msaa_color_buffer.view,
            vid.oit_accum_buffer_view,
            vid.oit_reveal_buffer_view,
            vid.mboit_b0_buffer_view,
            vid.mboit_moments0_buffer_view,
            vid.mboit_color_buffer_view,
        ];
        if use_mboit {
            attachments[if resolve { 3 } else { 2 }] = vid.mboit_b0_buffer_view;
            attachments[if resolve { 4 } else { 3 }] = vid.mboit_moments0_buffer_view;
            attachments[if resolve { 5 } else { 4 }] = vid.mboit_color_buffer_view;
        } else if !resolve {
            attachments[2] = vid.oit_accum_buffer_view;
            attachments[3] = vid.oit_reveal_buffer_view;
        }

        let framebuffer_create_info = vk::FramebufferCreateInfo::default()
            .render_pass(vg!(
                ctx,
                main_render_pass[variant][MAIN_RENDER_PASS_STENCIL_CLEAR]
            ))
            .attachments(&attachments[..attachment_count])
            .width(width)
            .height(height)
            .layers(1);
        // SAFETY: `framebuffer_create_info` refers to `attachments`, a live local.
        vid.main_framebuffers[i] = match unsafe {
            ctx.device
                .create_framebuffer(&framebuffer_create_info, None)
        } {
            Ok(framebuffer) => framebuffer,
            Err(err) => ctx.vk_fail("vkCreateFramebuffer", err),
        };
        ctx.name_object(vid.main_framebuffers[i], c"main");
    }
}

/// `GL_DestroyMainFrameBuffers`.
fn destroy_main_frame_buffers<E: VidEngine>(ctx: &mut Ctx<'_, E>, vid: &mut VidState) {
    for framebuffer in vid.main_framebuffers.iter_mut() {
        if *framebuffer != vk::Framebuffer::null() {
            // SAFETY: created on `ctx.device` above; the device is idle.
            unsafe { ctx.device.destroy_framebuffer(*framebuffer, None) };
            *framebuffer = vk::Framebuffer::null();
        }
    }
}

/// `GL_CreateFrameBuffers`.
fn create_frame_buffers<E: VidEngine>(ctx: &mut Ctx<'_, E>, vid: &mut VidState) {
    ctx.engine.sys_printf("Creating frame buffers\n");

    create_main_frame_buffers(ctx, vid);

    let (width, height) = ctx.engine.vid_size();
    for i in 0..vid.num_swap_chain_images {
        let attachments = [vid.color_buffers_view[0], vid.swapchain_images_views[i]];
        let framebuffer_create_info = vk::FramebufferCreateInfo::default()
            .render_pass(scbx_mut(ctx.vg, SCBX_GUI as usize, 0).render_pass)
            .attachments(&attachments)
            .width(width)
            .height(height)
            .layers(1);
        // SAFETY: as in `create_main_frame_buffers`.
        vid.ui_framebuffers[i] = match unsafe {
            ctx.device
                .create_framebuffer(&framebuffer_create_info, None)
        } {
            Ok(framebuffer) => framebuffer,
            Err(err) => ctx.vk_fail("vkCreateFramebuffer", err),
        };
        ctx.name_object(vid.ui_framebuffers[i], c"ui");
    }
}

/// `GL_CreateRenderResources`.
pub fn create_render_resources<E: VidEngine>(ctx: &mut Ctx<'_, E>, vid: &mut VidState) {
    if ctx.engine.skip_render_resources() {
        return;
    }

    if !create_swap_chain(ctx, vid) {
        vid.render_resources_created = false;
        return;
    }

    create_color_buffer(ctx, vid);
    create_depth_buffer(ctx, vid);
    create_render_passes(ctx);
    create_frame_buffers(ctx, vid);
    let mut modules = ShaderModules::new();
    create_pipelines(ctx, &mut modules);

    vid.render_resources_created = true;

    update_descriptor_sets(ctx, vid);
}

/// `GL_DestroyMainRenderPasses`.
fn destroy_main_render_passes<E: VidEngine>(ctx: &mut Ctx<'_, E>) {
    for variant in 0..MAIN_RENDER_PASS_VARIANT_COUNT {
        for stencil in 0..MAIN_RENDER_PASS_STENCIL_COUNT {
            let render_pass = vg!(ctx, main_render_pass[variant][stencil]);
            if render_pass != vk::RenderPass::null() {
                // SAFETY: created on `ctx.device`; the device is idle.
                unsafe { ctx.device.destroy_render_pass(render_pass, None) };
            }
            *vg_mut!(ctx, main_render_pass[variant][stencil]) = vk::RenderPass::null();
        }
    }

    for (scbx_index, multiplicity) in scbx_slots(SCBX_WORLD, SCBX_OIT_RESOLVE) {
        for i in 0..multiplicity {
            scbx_mut(ctx.vg, scbx_index, i).render_pass = vk::RenderPass::null();
        }
    }
}

/// `GL_DestroyRenderResources`.
pub fn destroy_render_resources<E: VidEngine>(ctx: &mut Ctx<'_, E>, vid: &mut VidState) {
    vid.render_resources_created = false;

    super::frame::wait_for_device_idle(ctx);

    destroy_pipelines(ctx);

    if vid.postprocess_descriptor_set != vk::DescriptorSet::null() {
        free_descriptor_set(
            ctx,
            vid.postprocess_descriptor_set,
            &vg!(ctx, input_attachment_set_layout),
        );
        vid.postprocess_descriptor_set = vk::DescriptorSet::null();
    }
    if vid.wboit_resolve_descriptor_set != vk::DescriptorSet::null() {
        free_descriptor_set(
            ctx,
            vid.wboit_resolve_descriptor_set,
            &vg!(ctx, oit_input_attachment_set_layout),
        );
        vid.wboit_resolve_descriptor_set = vk::DescriptorSet::null();
    }
    if vg!(ctx, mboit_input_attachment_descriptor_set) != vk::DescriptorSet::null() {
        free_descriptor_set(
            ctx,
            vg!(ctx, mboit_input_attachment_descriptor_set),
            &vg!(ctx, mboit_input_attachment_set_layout),
        );
        *vg_mut!(ctx, mboit_input_attachment_descriptor_set) = vk::DescriptorSet::null();
    }
    if vg!(ctx, screen_effects_desc_set) != vk::DescriptorSet::null() {
        free_descriptor_set(
            ctx,
            vg!(ctx, screen_effects_desc_set),
            &vg!(ctx, screen_effects_set_layout),
        );
        *vg_mut!(ctx, screen_effects_desc_set) = vk::DescriptorSet::null();
    }

    destroy_main_frame_buffers(ctx, vid);

    if vid.msaa_color_buffer.image != vk::Image::null() {
        destroy_attachment(ctx, &mut vid.msaa_color_buffer);
    }

    destroy_oit_buffers(ctx, vid);

    for i in 0..NUM_COLOR_BUFFERS {
        let mut set = ImageSet {
            image: vg!(ctx, color_buffers[i]),
            memory: vid.color_buffers_memory[i],
            view: vid.color_buffers_view[i],
        };
        destroy_attachment(ctx, &mut set);
        vid.color_buffers_view[i] = set.view;
        vid.color_buffers_memory[i] = set.memory;
        *vg_mut!(ctx, color_buffers[i]) = set.image;
    }

    destroy_attachment(ctx, &mut vid.depth_buffer);

    for i in 0..vid.num_swap_chain_images {
        // SAFETY: created by `create_swap_chain`/`create_frame_buffers` on
        // `ctx.device`; the device is idle.
        unsafe {
            ctx.device
                .destroy_image_view(vid.swapchain_images_views[i], None);
            vid.swapchain_images_views[i] = vk::ImageView::null();
            ctx.device.destroy_framebuffer(vid.ui_framebuffers[i], None);
            vid.ui_framebuffers[i] = vk::Framebuffer::null();
        }
        vid.swapchain_images[i] = vk::Image::null();
    }

    for i in 0..DOUBLE_BUFFERED {
        // SAFETY: as above.
        unsafe {
            ctx.device
                .destroy_semaphore(vid.image_aquired_semaphores[i], None)
        };
        vid.image_aquired_semaphores[i] = vk::Semaphore::null();
    }
    for i in 0..vid.num_swap_chain_images {
        // SAFETY: as above.
        unsafe {
            ctx.device
                .destroy_semaphore(vid.draw_complete_semaphores[i], None)
        };
        vid.draw_complete_semaphores[i] = vk::Semaphore::null();
    }

    let destroy_swapchain = vid
        .procs
        .destroy_swapchain
        .expect("vkDestroySwapchainKHR is loaded with the device");
    let device = vg!(ctx, device);
    // SAFETY: `vid.swapchain` was created on `device` by
    // `create_swap_chain`; every image view and semaphore over it is gone.
    unsafe { destroy_swapchain(device, vid.swapchain, core::ptr::null()) };
    vid.swapchain = vk::SwapchainKHR::null();

    let ui_render_pass = scbx_mut(ctx.vg, SCBX_GUI as usize, 0).render_pass;
    // SAFETY: as above.
    unsafe { ctx.device.destroy_render_pass(ui_render_pass, None) };
    for (scbx_index, multiplicity) in scbx_slots(SCBX_GUI, SCBX_POST_PROCESS) {
        for i in 0..multiplicity {
            scbx_mut(ctx.vg, scbx_index, i).render_pass = vk::RenderPass::null();
        }
    }

    destroy_main_render_passes(ctx);
}

/// `R_CreatePaletteOctreeBuffers`: `colors` and `nodes` are the raw bytes of
/// the `uint32_t` colour table and the `palette_octree_node_t` array.
pub fn create_palette_octree_buffers<E: VidEngine>(
    ctx: &mut Ctx<'_, E>,
    vid: &mut VidState,
    staging: &Staging,
    colors: &[u8],
    nodes: &[u8],
) {
    let colors_size = colors.len();
    let nodes_size = nodes.len();

    let requests = [
        BufferRequest {
            size: colors_size as u64,
            alignment: 0,
            usage: vk::BufferUsageFlags::UNIFORM_TEXEL_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
            mapped: false,
            address: false,
            name: "Palette colors",
        },
        BufferRequest {
            size: nodes_size as u64,
            alignment: 0,
            usage: vk::BufferUsageFlags::UNIFORM_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
            mapped: false,
            address: false,
            name: "Palette octree",
        },
    ];
    // The C keeps `memory` in a local it never frees; the allocation lives for
    // the process.
    let mut memory = NULL_MEMORY;
    let (_, buffers) = create_buffers(
        ctx,
        &requests,
        &mut memory,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
        vk::MemoryPropertyFlags::empty(),
        Some(ctx.counters.misc),
        c"Palette",
    );
    vid.palette_colors_buffer = buffers[0].buffer;
    vid.palette_octree_buffer = buffers[1].buffer;

    {
        let allocation = staging.allocate(ctx, colors_size as i32, 1);
        let region = vk::BufferCopy {
            src_offset: allocation.buffer_offset as u64,
            dst_offset: 0,
            size: colors_size as u64,
        };
        // SAFETY: the staging command buffer is recording and both buffers are live.
        unsafe {
            ctx.device.cmd_copy_buffer(
                allocation.command_buffer,
                allocation.buffer,
                vid.palette_colors_buffer,
                core::slice::from_ref(&region),
            )
        };
        staging.begin_copy();
        // SAFETY: `allocation.data` addresses `colors_size` mapped bytes
        // reserved for this copy.
        unsafe { core::ptr::copy_nonoverlapping(colors.as_ptr(), allocation.data, colors_size) };
        staging.end_copy();
    }

    let buffer_view_create_info = vk::BufferViewCreateInfo::default()
        .buffer(vid.palette_colors_buffer)
        .format(vk::Format::R8G8B8A8_UNORM)
        .range(vk::WHOLE_SIZE);
    // SAFETY: `buffer_view_create_info` is complete over a live buffer.
    vid.palette_buffer_view = match unsafe {
        ctx.device
            .create_buffer_view(&buffer_view_create_info, None)
    } {
        Ok(view) => view,
        Err(err) => ctx.vk_fail("vkCreateBufferView", err),
    };
    ctx.name_object(vid.palette_buffer_view, c"Palette colors");

    {
        let allocation = staging.allocate(ctx, nodes_size as i32, 1);
        let region = vk::BufferCopy {
            src_offset: allocation.buffer_offset as u64,
            dst_offset: 0,
            size: nodes_size as u64,
        };
        // SAFETY: as for the colours above.
        unsafe {
            ctx.device.cmd_copy_buffer(
                allocation.command_buffer,
                allocation.buffer,
                vid.palette_octree_buffer,
                core::slice::from_ref(&region),
            )
        };
        staging.begin_copy();
        // SAFETY: as for the colours above.
        unsafe { core::ptr::copy_nonoverlapping(nodes.as_ptr(), allocation.data, nodes_size) };
        staging.end_copy();
    }
}
