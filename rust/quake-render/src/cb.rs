//! The `glquake.h` command-buffer inline helpers (`R_BindPipeline`,
//! `R_PushConstants`, `R_BeginDebugUtilsLabel`, `R_EndDebugUtilsLabel`) over
//! a [`CbContext`] and the Rust-owned `vulkan_globals` (Phase 8 M6).
//!
//! These go through the `vk_cmd_*` entry points `gl_vidsdl.c` loaded into
//! `vulkan_globals`, not ash's own table, so the C and Rust builds resolve the
//! same functions (the harness hooks them under `-renderhash`).

use core::ffi::{c_int, CStr};

use ash::vk;
use quake_types::render::{CbContext, VulkanPipeline};

use crate::rmisc::{vg, VgPtr};

use crate::vid::{
    RENDER_PASS_INDEX_MAIN_MBOIT, RENDER_PASS_INDEX_MAIN_OIT, RENDER_PASS_INDEX_MBOIT_COMPOSITE,
    RENDER_PASS_INDEX_MBOIT_MOMENTS, RENDER_PASS_INDEX_WBOIT,
};

/// `MAX_PUSH_CONSTANT_SIZE` (`glquake.h`).
const MAX_PUSH_CONSTANT_SIZE: usize = 128;

/// The `vulkan_globals` entry points and state the helpers read, snapshotted
/// so a helper can take a `&mut` context that itself lives inside
/// `vulkan_globals` (`primary_cb_contexts`) without aliasing the struct.
#[derive(Clone, Copy)]
pub struct CmdProcs {
    bind_pipeline: Option<vk::PFN_vkCmdBindPipeline>,
    push_constants: Option<vk::PFN_vkCmdPushConstants>,
    bind_descriptor_sets: Option<vk::PFN_vkCmdBindDescriptorSets>,
    draw: Option<vk::PFN_vkCmdDraw>,
    draw_indexed: Option<vk::PFN_vkCmdDrawIndexed>,
    draw_indexed_indirect: Option<vk::PFN_vkCmdDrawIndexedIndirect>,
    #[cfg(feature = "engine-debug")]
    begin_debug_utils_label: Option<vk::PFN_vkCmdBeginDebugUtilsLabelEXT>,
    #[cfg(feature = "engine-debug")]
    end_debug_utils_label: Option<vk::PFN_vkCmdEndDebugUtilsLabelEXT>,
    mboit_input_attachment_descriptor_set: vk::DescriptorSet,
}

impl CmdProcs {
    pub fn new(vg: VgPtr<'_>) -> Self {
        struct Holder<'a> {
            vg: VgPtr<'a>,
        }
        let h = Holder { vg };
        Self {
            bind_pipeline: vg!(h, vk_cmd_bind_pipeline),
            push_constants: vg!(h, vk_cmd_push_constants),
            bind_descriptor_sets: vg!(h, vk_cmd_bind_descriptor_sets),
            draw: vg!(h, vk_cmd_draw),
            draw_indexed: vg!(h, vk_cmd_draw_indexed),
            draw_indexed_indirect: vg!(h, vk_cmd_draw_indexed_indirect),
            #[cfg(feature = "engine-debug")]
            begin_debug_utils_label: vg!(h, vk_cmd_begin_debug_utils_label),
            #[cfg(feature = "engine-debug")]
            end_debug_utils_label: vg!(h, vk_cmd_end_debug_utils_label),
            mboit_input_attachment_descriptor_set: vg!(h, mboit_input_attachment_descriptor_set),
        }
    }
}

/// `vulkan_globals.vk_cmd_draw (...)`. Every draw goes through the
/// `vulkan_globals` pointer rather than the `ash::Device` table because the
/// `-renderhash` harness (`harness_render.c`) wraps these three entry points
/// to record the draw structure; a direct `ash` call would be invisible to it.
///
/// # Safety
/// `cb` is recording inside a render pass with a graphics pipeline bound.
pub unsafe fn draw(
    procs: &CmdProcs,
    cb: vk::CommandBuffer,
    vertex_count: u32,
    instance_count: u32,
    first_vertex: u32,
    first_instance: u32,
) {
    let f = procs.draw.expect("vkCmdDraw is loaded before any draw");
    // SAFETY: the caller's contract; `f` is the entry point loaded for the
    // device that owns `cb`.
    unsafe {
        f(
            cb,
            vertex_count,
            instance_count,
            first_vertex,
            first_instance,
        )
    }
}

/// `vulkan_globals.vk_cmd_draw_indexed (...)`; see [`draw`].
///
/// # Safety
/// As for [`draw`], with an index buffer bound.
pub unsafe fn draw_indexed(
    procs: &CmdProcs,
    cb: vk::CommandBuffer,
    index_count: u32,
    instance_count: u32,
    first_index: u32,
    vertex_offset: i32,
    first_instance: u32,
) {
    let f = procs
        .draw_indexed
        .expect("vkCmdDrawIndexed is loaded before any draw");
    // SAFETY: as for `draw`.
    unsafe {
        f(
            cb,
            index_count,
            instance_count,
            first_index,
            vertex_offset,
            first_instance,
        )
    }
}

/// `vulkan_globals.vk_cmd_draw_indexed_indirect (...)`; see [`draw`].
///
/// # Safety
/// As for [`draw_indexed`]; `buffer` holds `draw_count` commands at `offset`.
pub unsafe fn draw_indexed_indirect(
    procs: &CmdProcs,
    cb: vk::CommandBuffer,
    buffer: vk::Buffer,
    offset: vk::DeviceSize,
    draw_count: u32,
    stride: u32,
) {
    let f = procs
        .draw_indexed_indirect
        .expect("vkCmdDrawIndexedIndirect is loaded before any draw");
    // SAFETY: as for `draw`.
    unsafe { f(cb, buffer, offset, draw_count, stride) }
}

/// `R_BindPipeline`: binds when the handle changes, zeroes the push-constant
/// range when its shape changes, and binds the MBOIT input-attachment set for
/// composite-pass pipelines that take one.
pub fn bind_pipeline(
    procs: &CmdProcs,
    cbx: &mut CbContext,
    bind_point: vk::PipelineBindPoint,
    pipeline: VulkanPipeline,
) {
    static ZEROES: [u8; MAX_PUSH_CONSTANT_SIZE] = [0; MAX_PUSH_CONSTANT_SIZE];
    debug_assert!(pipeline.handle != vk::Pipeline::null());
    if cbx.current_pipeline.handle != pipeline.handle {
        let bind = procs
            .bind_pipeline
            .expect("vkCmdBindPipeline is loaded before any pipeline is bound");
        // SAFETY: `cbx.cb` is a command buffer in the recording state; `bind`
        // is the entry point loaded for the device that owns it.
        unsafe { bind(cbx.cb, bind_point, pipeline.handle) };

        let new_range = pipeline.layout.push_constant_range;
        let old_range = cbx.current_pipeline.layout.push_constant_range;
        if new_range.size > 0
            && (old_range.stage_flags != new_range.stage_flags || old_range.size != new_range.size)
        {
            let push = procs
                .push_constants
                .expect("vkCmdPushConstants is loaded before any pipeline is bound");
            // SAFETY: as above; `ZEROES` covers `MAX_PUSH_CONSTANT_SIZE`
            // bytes, the largest range any layout declares.
            unsafe {
                push(
                    cbx.cb,
                    pipeline.layout.handle,
                    new_range.stage_flags,
                    0,
                    new_range.size,
                    ZEROES.as_ptr().cast(),
                )
            };
        }

        cbx.current_pipeline = pipeline;

        if cbx.render_pass_index == RENDER_PASS_INDEX_MBOIT_COMPOSITE
            && pipeline.layout.mboit_input_attachment_set >= 0
        {
            let bind_sets = procs
                .bind_descriptor_sets
                .expect("vkCmdBindDescriptorSets is loaded before any pipeline is bound");
            // SAFETY: as above; the descriptor set is the one `gl_vidsdl.c`
            // allocated for the MBOIT composite pass.
            unsafe {
                bind_sets(
                    cbx.cb,
                    bind_point,
                    pipeline.layout.handle,
                    pipeline.layout.mboit_input_attachment_set as u32,
                    1,
                    &procs.mboit_input_attachment_descriptor_set,
                    0,
                    core::ptr::null(),
                )
            };
        }
    }
}

/// `R_PushConstants` against the layout of the pipeline currently bound.
pub fn push_constants(
    procs: &CmdProcs,
    cbx: &CbContext,
    stage_flags: vk::ShaderStageFlags,
    offset: u32,
    data: &[u8],
) {
    let push = procs
        .push_constants
        .expect("vkCmdPushConstants is loaded before any constants are pushed");
    // SAFETY: `cbx.cb` is recording with `current_pipeline` bound; `data` is
    // live for the call.
    unsafe {
        push(
            cbx.cb,
            cbx.current_pipeline.layout.handle,
            stage_flags,
            offset,
            data.len() as u32,
            data.as_ptr().cast(),
        )
    };
}

/// `R_BeginDebugUtilsLabel` (`_DEBUG` only, and only once
/// `VK_EXT_debug_utils` is loaded).
#[allow(unused_variables)]
pub fn begin_debug_utils_label(procs: &CmdProcs, cbx: &CbContext, name: &CStr) {
    #[cfg(feature = "engine-debug")]
    if let Some(begin) = procs.begin_debug_utils_label {
        let label = vk::DebugUtilsLabelEXT::default().label_name(name);
        // SAFETY: `cbx.cb` is recording; `label` and `name` outlive the call.
        unsafe { begin(cbx.cb, &label) };
    }
}

/// `R_EndDebugUtilsLabel` (`_DEBUG` only).
#[allow(unused_variables)]
pub fn end_debug_utils_label(procs: &CmdProcs, cbx: &CbContext) {
    #[cfg(feature = "engine-debug")]
    if let Some(end) = procs.end_debug_utils_label {
        // SAFETY: `cbx.cb` is recording.
        unsafe { end(cbx.cb) };
    }
}

/// `R_MainPassPipelineVariant` (`glquake.h`): the `main_render_pass_variant_t`
/// index for a render pass.
pub fn main_pass_pipeline_variant(render_pass_index: c_int) -> usize {
    if render_pass_index == RENDER_PASS_INDEX_MAIN_OIT {
        1
    } else if render_pass_index == RENDER_PASS_INDEX_MAIN_MBOIT {
        2
    } else {
        0
    }
}

/// `R_PipelineForRenderPass` (`glquake.h`): selects between the standard,
/// WBOIT accumulation, MBOIT moment and MBOIT composite pipelines of a shader
/// family based on the render pass a context records into.
pub fn pipeline_for_render_pass(
    render_pass_index: c_int,
    main: VulkanPipeline,
    wboit: VulkanPipeline,
    mboit_moment: VulkanPipeline,
    mboit_composite: VulkanPipeline,
) -> VulkanPipeline {
    match render_pass_index {
        RENDER_PASS_INDEX_WBOIT => wboit,
        RENDER_PASS_INDEX_MBOIT_MOMENTS => mboit_moment,
        RENDER_PASS_INDEX_MBOIT_COMPOSITE => mboit_composite,
        _ => main,
    }
}
