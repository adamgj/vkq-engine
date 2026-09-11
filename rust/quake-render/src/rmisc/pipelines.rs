//! Pipeline layouts (`R_CreatePipelineLayouts`), the graphics/compute pipeline
//! families (`R_Create*Pipelines`, `R_CreatePipelines`) and
//! `R_DestroyPipelines`.
//!
//! `pipeline_create_infos_t` becomes [`PipelineInfos`]: a `Copy` bundle of
//! every state structure whose internal pointers are re-aimed at the copy
//! right before `vkCreateGraphicsPipelines`, which is what
//! `R_CopyPipelineCreateInfos` does after its `memcpy`. Vertex attribute
//! tables are `static`s (C fills them in `R_InitVertexAttributes`).

use core::ffi::CStr;
use core::mem::offset_of;
use core::ptr;

use ash::vk;
use quake_types::model_mem::{Md5Vert, Md5Vert8};
use quake_types::render::{
    MeshInterpolatePushConstants, SkinningPushConstants, VulkanGlobals, VulkanPipeline,
    VulkanPipelineLayout, FTE_PARTICLE_PIPELINE_COUNT, MAIN_RENDER_PASS_VARIANT_COUNT,
    MODEL_PIPELINE_COUNT, RENDER_PASS_INDEX_COUNT, WORLD_PIPELINE_COUNT,
};

use super::memory::c_string;
use super::shaders::{Shader, ShaderModules};
use super::{Ctx, Engine};

/// `SCBX_GUI` (`glquake.h`).
const SCBX_GUI: usize = 14;
/// `render_pass_index_t`.
const RENDER_PASS_INDEX_MAIN: usize = 0;
const RENDER_PASS_INDEX_UI: usize = 1;
const RENDER_PASS_INDEX_MAIN_OIT: usize = 2;
const RENDER_PASS_INDEX_MAIN_MBOIT: usize = 3;
const RENDER_PASS_INDEX_WBOIT: usize = 4;
const RENDER_PASS_INDEX_MBOIT_MOMENTS: usize = 5;
const RENDER_PASS_INDEX_MBOIT_COMPOSITE: usize = 6;
/// `main_render_pass_variant_t`.
const MAIN_RENDER_PASS_STANDARD: usize = 0;
const MAIN_RENDER_PASS_OIT: usize = 1;
const MAIN_RENDER_PASS_MBOIT: usize = 2;
/// `MAIN_RENDER_PASS_STENCIL_CLEAR`.
const MAIN_RENDER_PASS_STENCIL_CLEAR: usize = 0;
/// `MAIN_COLOR_ATTACHMENT_COUNT`.
const MAIN_COLOR_ATTACHMENT_COUNT: u32 = 1;
/// `OFFSET_DECAL`.
const OFFSET_DECAL: f32 = 1.0;

const FLOAT: u32 = 4;

// ---------------------------------------------------------------------------
// Vertex attribute tables (`R_InitVertexAttributes`)
// ---------------------------------------------------------------------------

const fn attr(
    binding: u32,
    format: vk::Format,
    location: u32,
    offset: u32,
) -> vk::VertexInputAttributeDescription {
    vk::VertexInputAttributeDescription {
        location,
        binding,
        format,
        offset,
    }
}

const fn binding(binding: u32, stride: u32) -> vk::VertexInputBindingDescription {
    vk::VertexInputBindingDescription {
        binding,
        stride,
        input_rate: vk::VertexInputRate::VERTEX,
    }
}

/// One vertex-input configuration: an attribute table and its bindings.
pub struct VertexInput {
    pub attrs: &'static [vk::VertexInputAttributeDescription],
    pub bindings: &'static [vk::VertexInputBindingDescription],
}

pub static BASIC_INPUT: VertexInput = VertexInput {
    attrs: &[
        attr(0, vk::Format::R32G32B32_SFLOAT, 0, 0),
        attr(0, vk::Format::R32G32_SFLOAT, 1, 12),
        attr(0, vk::Format::R8G8B8A8_UNORM, 2, 20),
    ],
    bindings: &[binding(0, 24)],
};

pub static WORLD_INPUT: VertexInput = VertexInput {
    attrs: &[
        attr(0, vk::Format::R32G32B32_SFLOAT, 0, 0),
        attr(0, vk::Format::R32G32_SFLOAT, 1, 12),
        attr(0, vk::Format::R32G32_SFLOAT, 2, 20),
    ],
    bindings: &[binding(0, 28)],
};

pub static ALIAS_INPUT: VertexInput = VertexInput {
    attrs: &[
        attr(0, vk::Format::R32G32_SFLOAT, 0, 0),
        attr(1, vk::Format::R16G16B16A16_UNORM, 1, 0),
        attr(1, vk::Format::R8G8B8A8_SNORM, 2, 8),
        attr(2, vk::Format::R16G16B16A16_UNORM, 3, 0),
        attr(2, vk::Format::R8G8B8A8_SNORM, 4, 8),
    ],
    bindings: &[binding(0, 8), binding(1, 12), binding(2, 12)],
};

pub static MD5_INPUT: VertexInput = VertexInput {
    attrs: &[
        attr(0, vk::Format::R32G32B32_SFLOAT, 0, 0),
        attr(0, vk::Format::R32G32B32_SFLOAT, 1, 12),
        attr(0, vk::Format::R32G32_SFLOAT, 2, 24),
        attr(0, vk::Format::R8G8B8A8_UNORM, 3, 32),
        attr(0, vk::Format::R8G8B8A8_UINT, 4, 36),
        attr(
            0,
            vk::Format::R32G32B32A32_SFLOAT,
            5,
            offset_of!(Md5Vert, joint_position_x) as u32,
        ),
        attr(
            0,
            vk::Format::R32G32B32A32_SFLOAT,
            6,
            offset_of!(Md5Vert, joint_position_y) as u32,
        ),
        attr(
            0,
            vk::Format::R32G32B32A32_SFLOAT,
            7,
            offset_of!(Md5Vert, joint_position_z) as u32,
        ),
    ],
    bindings: &[binding(0, size_of::<Md5Vert>() as u32)],
};

/// `sizeof (float) * 4`: one attribute covers four influences.
const INFLUENCE_GROUP_BYTES: u32 = 4 * FLOAT;

pub static MD5_8_INPUT: VertexInput = VertexInput {
    attrs: &[
        attr(0, vk::Format::R32G32B32_SFLOAT, 0, 0),
        attr(0, vk::Format::R32G32B32_SFLOAT, 1, 12),
        attr(0, vk::Format::R32G32_SFLOAT, 2, 24),
        attr(0, vk::Format::R8G8B8A8_UNORM, 3, 32),
        attr(0, vk::Format::R8G8B8A8_UNORM, 4, 36),
        attr(0, vk::Format::R8G8B8A8_UINT, 5, 40),
        attr(0, vk::Format::R8G8B8A8_UINT, 6, 44),
        attr(
            0,
            vk::Format::R32G32B32A32_SFLOAT,
            7,
            offset_of!(Md5Vert8, joint_position_x) as u32,
        ),
        attr(
            0,
            vk::Format::R32G32B32A32_SFLOAT,
            8,
            offset_of!(Md5Vert8, joint_position_x) as u32 + INFLUENCE_GROUP_BYTES,
        ),
        attr(
            0,
            vk::Format::R32G32B32A32_SFLOAT,
            9,
            offset_of!(Md5Vert8, joint_position_y) as u32,
        ),
        attr(
            0,
            vk::Format::R32G32B32A32_SFLOAT,
            10,
            offset_of!(Md5Vert8, joint_position_y) as u32 + INFLUENCE_GROUP_BYTES,
        ),
        attr(
            0,
            vk::Format::R32G32B32A32_SFLOAT,
            11,
            offset_of!(Md5Vert8, joint_position_z) as u32,
        ),
        attr(
            0,
            vk::Format::R32G32B32A32_SFLOAT,
            12,
            offset_of!(Md5Vert8, joint_position_z) as u32 + INFLUENCE_GROUP_BYTES,
        ),
    ],
    bindings: &[binding(0, size_of::<Md5Vert8>() as u32)],
};

/// The local tables `R_CreateShowTrisPipelines` builds.
pub static SHOWTRIS_INPUT: VertexInput = VertexInput {
    attrs: &[attr(0, vk::Format::R32G32B32_SFLOAT, 0, 0)],
    bindings: &[binding(0, 24)],
};

/// `R_CreatePostprocessPipelines`: no vertex input at all.
pub static EMPTY_INPUT: VertexInput = VertexInput {
    attrs: &[],
    bindings: &[],
};

// ---------------------------------------------------------------------------
// Blend helpers
// ---------------------------------------------------------------------------

type BlendState = vk::PipelineColorBlendAttachmentState;

const RGBA: vk::ColorComponentFlags = vk::ColorComponentFlags::RGBA;

const fn blend(
    enable: bool,
    write_mask: vk::ColorComponentFlags,
    src_color: vk::BlendFactor,
    dst_color: vk::BlendFactor,
    src_alpha: vk::BlendFactor,
    dst_alpha: vk::BlendFactor,
) -> BlendState {
    BlendState {
        blend_enable: enable as vk::Bool32,
        src_color_blend_factor: src_color,
        dst_color_blend_factor: dst_color,
        color_blend_op: vk::BlendOp::ADD,
        src_alpha_blend_factor: src_alpha,
        dst_alpha_blend_factor: dst_alpha,
        alpha_blend_op: vk::BlendOp::ADD,
        color_write_mask: write_mask,
    }
}

/// `R_SetWBOITBlend`.
fn set_wboit_blend(states: &mut [BlendState; 3]) {
    use vk::BlendFactor as F;
    for state in states.iter_mut() {
        *state = blend(
            false,
            vk::ColorComponentFlags::empty(),
            F::ONE,
            F::ZERO,
            F::ONE,
            F::ZERO,
        );
    }
    states[0] = blend(true, RGBA, F::ONE, F::ONE, F::ONE, F::ONE);
    states[1] = blend(
        true,
        vk::ColorComponentFlags::R,
        F::ZERO,
        F::ONE_MINUS_SRC_COLOR,
        F::ZERO,
        F::ONE_MINUS_SRC_ALPHA,
    );
}

/// `R_SetMBOITMomentBlend`.
fn set_mboit_moment_blend(states: &mut [BlendState; 3]) {
    use vk::BlendFactor as F;
    for state in states.iter_mut().take(2) {
        *state = blend(true, RGBA, F::ONE, F::ONE, F::ONE, F::ONE);
    }
    states[0].color_write_mask = vk::ColorComponentFlags::R;
}

/// `R_SetMBOITCompositeBlend`.
fn set_mboit_composite_blend(states: &mut [BlendState; 3]) {
    use vk::BlendFactor as F;
    states[0] = blend(true, RGBA, F::ONE, F::ONE, F::ONE, F::ONE);
}

/// `R_SetFTEParticleBlend`'s source/destination tables, indexed by `mode`.
pub const FTE_SRC_BLEND: [vk::BlendFactor; 8] = [
    vk::BlendFactor::SRC_ALPHA,
    vk::BlendFactor::SRC_COLOR,
    vk::BlendFactor::SRC_ALPHA,
    vk::BlendFactor::SRC_COLOR,
    vk::BlendFactor::SRC_ALPHA,
    vk::BlendFactor::ZERO,
    vk::BlendFactor::ZERO,
    vk::BlendFactor::ONE,
];
pub const FTE_DST_BLEND: [vk::BlendFactor; 8] = [
    vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
    vk::BlendFactor::ONE_MINUS_SRC_COLOR,
    vk::BlendFactor::ONE,
    vk::BlendFactor::ONE,
    vk::BlendFactor::ONE_MINUS_SRC_COLOR,
    vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
    vk::BlendFactor::ONE_MINUS_SRC_COLOR,
    vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
];

/// `R_SetFTEParticleBlend`.
pub fn fte_particle_blend(mode: usize) -> BlendState {
    // COMPAT: C sets dstAlphaBlendFactor from the *source* table; keep it.
    blend(
        true,
        RGBA,
        FTE_SRC_BLEND[mode],
        FTE_DST_BLEND[mode],
        FTE_SRC_BLEND[mode],
        FTE_SRC_BLEND[mode],
    )
}

// ---------------------------------------------------------------------------
// pipeline_create_infos_t
// ---------------------------------------------------------------------------

/// `pipeline_create_infos_t`.
#[derive(Clone, Copy)]
pub struct PipelineInfos<'a> {
    dynamic_states: [vk::DynamicState; 3],
    dynamic_state: vk::PipelineDynamicStateCreateInfo<'static>,
    /// `'a` is the lifetime of the fragment `SpecializationInfo` aimed at
    /// by [`set_fragment_spec`]; the borrow checker keeps a copy from
    /// outliving it.
    shader_stages: [vk::PipelineShaderStageCreateInfo<'a>; 2],
    vertex_input: vk::PipelineVertexInputStateCreateInfo<'static>,
    input_assembly: vk::PipelineInputAssemblyStateCreateInfo<'static>,
    viewport: vk::PipelineViewportStateCreateInfo<'static>,
    rasterization: vk::PipelineRasterizationStateCreateInfo<'static>,
    multisample: vk::PipelineMultisampleStateCreateInfo<'static>,
    depth_stencil: vk::PipelineDepthStencilStateCreateInfo<'static>,
    blend_attachments: [BlendState; 3],
    color_blend: vk::PipelineColorBlendStateCreateInfo<'static>,
    graphics: vk::GraphicsPipelineCreateInfo<'a>,
}

fn stage(
    stage: vk::ShaderStageFlags,
    module: vk::ShaderModule,
) -> vk::PipelineShaderStageCreateInfo<'static> {
    vk::PipelineShaderStageCreateInfo::default()
        .stage(stage)
        .module(module)
        .name(c"main")
}

impl<'a> PipelineInfos<'a> {
    /// `R_InitDefaultStates`.
    fn defaults(vg: &VulkanGlobals, m: &ShaderModules) -> Self {
        let blend_attachments = core::array::from_fn(|i| {
            blend(
                false,
                if i == 0 {
                    RGBA
                } else {
                    vk::ColorComponentFlags::empty()
                },
                vk::BlendFactor::SRC_ALPHA,
                vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
                vk::BlendFactor::SRC_ALPHA,
                vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
            )
        });
        let stencil = vk::StencilOpState {
            fail_op: vk::StencilOp::KEEP,
            pass_op: vk::StencilOp::KEEP,
            depth_fail_op: vk::StencilOp::KEEP,
            compare_op: vk::CompareOp::ALWAYS,
            compare_mask: 0,
            write_mask: 0,
            reference: 0,
        };
        let mut infos = PipelineInfos {
            dynamic_states: [
                vk::DynamicState::VIEWPORT,
                vk::DynamicState::SCISSOR,
                vk::DynamicState::VIEWPORT,
            ],
            dynamic_state: vk::PipelineDynamicStateCreateInfo::default(),
            shader_stages: [
                stage(vk::ShaderStageFlags::VERTEX, m.get(Shader::basic_vert)),
                stage(vk::ShaderStageFlags::FRAGMENT, m.get(Shader::basic_frag)),
            ],
            vertex_input: vk::PipelineVertexInputStateCreateInfo::default(),
            input_assembly: vk::PipelineInputAssemblyStateCreateInfo::default()
                .topology(vk::PrimitiveTopology::TRIANGLE_LIST),
            viewport: vk::PipelineViewportStateCreateInfo::default()
                .viewport_count(1)
                .scissor_count(1),
            rasterization: vk::PipelineRasterizationStateCreateInfo::default()
                .polygon_mode(vk::PolygonMode::FILL)
                .cull_mode(vk::CullModeFlags::BACK)
                .front_face(vk::FrontFace::CLOCKWISE)
                .line_width(1.0),
            multisample: vk::PipelineMultisampleStateCreateInfo::default()
                .rasterization_samples(vg.sample_count),
            depth_stencil: vk::PipelineDepthStencilStateCreateInfo::default()
                .depth_compare_op(vk::CompareOp::GREATER_OR_EQUAL)
                .front(stencil)
                .back(stencil),
            blend_attachments,
            color_blend: vk::PipelineColorBlendStateCreateInfo::default(),
            graphics: vk::GraphicsPipelineCreateInfo::default()
                .layout(vg.basic_pipeline_layout.handle)
                .render_pass(
                    vg.main_render_pass[MAIN_RENDER_PASS_STANDARD][MAIN_RENDER_PASS_STENCIL_CLEAR],
                ),
        };
        infos.dynamic_state.dynamic_state_count = 2;
        if vg.supersampling {
            infos.multisample.sample_shading_enable = vk::TRUE;
            infos.multisample.min_sample_shading = 1.0;
        }
        infos.color_blend.attachment_count = MAIN_COLOR_ATTACHMENT_COUNT;
        infos.graphics.stage_count = 2;
        infos.set_vertex_input(&BASIC_INPUT);
        infos
    }

    fn set_vertex_input(&mut self, input: &'static VertexInput) {
        self.vertex_input.vertex_attribute_description_count = input.attrs.len() as u32;
        self.vertex_input.p_vertex_attribute_descriptions = if input.attrs.is_empty() {
            ptr::null()
        } else {
            input.attrs.as_ptr()
        };
        self.vertex_input.vertex_binding_description_count = input.bindings.len() as u32;
        self.vertex_input.p_vertex_binding_descriptions = if input.bindings.is_empty() {
            ptr::null()
        } else {
            input.bindings.as_ptr()
        };
    }

    /// The tail of `R_CopyPipelineCreateInfos`: aim every internal pointer at
    /// this copy.
    fn finalize(&mut self) {
        self.dynamic_state.p_dynamic_states = self.dynamic_states.as_ptr();
        self.color_blend.p_attachments = self.blend_attachments.as_ptr();
        self.graphics.p_stages = self.shader_stages.as_ptr();
        self.graphics.p_vertex_input_state = &self.vertex_input;
        self.graphics.p_input_assembly_state = &self.input_assembly;
        self.graphics.p_viewport_state = &self.viewport;
        self.graphics.p_rasterization_state = &self.rasterization;
        self.graphics.p_multisample_state = &self.multisample;
        self.graphics.p_depth_stencil_state = &self.depth_stencil;
        self.graphics.p_color_blend_state = &self.color_blend;
        self.graphics.p_dynamic_state = &self.dynamic_state;
    }

    fn set_render_pass(
        &mut self,
        render_pass: vk::RenderPass,
        subpass: u32,
        attachment_count: u32,
    ) {
        self.graphics.render_pass = render_pass;
        self.graphics.subpass = subpass;
        self.color_blend.attachment_count = attachment_count;
    }

    fn set_fragment(&mut self, module: vk::ShaderModule) {
        self.shader_stages[1].module = module;
    }

    fn set_vertex(&mut self, module: vk::ShaderModule) {
        self.shader_stages[0].module = module;
    }

    fn set_depth(&mut self, test: bool, write: bool) {
        self.depth_stencil.depth_test_enable = test as vk::Bool32;
        self.depth_stencil.depth_write_enable = write as vk::Bool32;
    }

    fn set_depth_bias(&mut self, constant: f32, slope: f32) {
        self.rasterization.depth_bias_enable = vk::TRUE;
        self.rasterization.depth_bias_constant_factor = constant;
        self.rasterization.depth_bias_slope_factor = slope;
    }

    fn add_dynamic_depth_bias(&mut self) {
        let count = self.dynamic_state.dynamic_state_count as usize;
        self.dynamic_states[count] = vk::DynamicState::DEPTH_BIAS;
        self.dynamic_state.dynamic_state_count += 1;
    }
}

// ---------------------------------------------------------------------------
// R_CreateGraphicsPipeline / R_CreateComputePipeline
// ---------------------------------------------------------------------------

fn pipeline_created<E: Engine>(ctx: &Ctx<'_, E>, handle: vk::Pipeline, name: &CStr) {
    ctx.name_object(handle, name);
    if ctx.engine.renderhash() {
        ctx.engine
            .pipeline_created(vk::Handle::as_raw(handle), name);
    }
}

/// `R_CreateGraphicsPipeline`.
fn create_graphics<E: Engine>(
    ctx: &Ctx<'_, E>,
    infos: &mut PipelineInfos,
    layout: VulkanPipelineLayout,
    name: &CStr,
) -> VulkanPipeline {
    infos.graphics.layout = layout.handle;
    infos.finalize();
    // SAFETY: `finalize` aimed every pointer inside `infos` at `infos` itself,
    // the vertex tables are `static`s and the shader modules are live for the
    // whole of `R_CreatePipelines`; the device is live.
    let handle = match unsafe {
        ctx.device
            .create_graphics_pipelines(vk::PipelineCache::null(), &[infos.graphics], None)
    } {
        Ok(pipelines) => pipelines[0],
        Err((_, err)) => ctx.engine.sys_error(&format!(
            "vkCreateGraphicsPipelines failed ({}) with code {}",
            name.to_string_lossy(),
            err.as_raw()
        )),
    };
    pipeline_created(ctx, handle, name);
    VulkanPipeline { handle, layout }
}

/// `R_CreateComputePipeline`.
fn create_compute<E: Engine>(
    ctx: &Ctx<'_, E>,
    layout: VulkanPipelineLayout,
    module: vk::ShaderModule,
    flags: vk::PipelineShaderStageCreateFlags,
    spec: Option<&vk::SpecializationInfo<'_>>,
    name: &CStr,
) -> VulkanPipeline {
    debug_assert!(layout.handle != vk::PipelineLayout::null());
    let mut stage_info = stage(vk::ShaderStageFlags::COMPUTE, module).flags(flags);
    if let Some(spec) = spec {
        stage_info = stage_info.specialization_info(spec);
    }
    let info = vk::ComputePipelineCreateInfo::default()
        .stage(stage_info)
        .layout(layout.handle);
    // SAFETY: `info` borrows `spec` and the `c"main"` literal, both live
    // across the call; the device is live.
    let handle = match unsafe {
        ctx.device
            .create_compute_pipelines(vk::PipelineCache::null(), &[info], None)
    } {
        Ok(pipelines) => pipelines[0],
        Err((_, err)) => ctx.engine.sys_error(&format!(
            "vkCreateComputePipelines failed ({}) with code {}",
            name.to_string_lossy(),
            err.as_raw()
        )),
    };
    pipeline_created(ctx, handle, name);
    VulkanPipeline { handle, layout }
}

// ---------------------------------------------------------------------------
// R_CreatePipelineLayouts
// ---------------------------------------------------------------------------

fn push_range(stage_flags: vk::ShaderStageFlags, size: u32) -> vk::PushConstantRange {
    vk::PushConstantRange {
        stage_flags,
        offset: 0,
        size,
    }
}

fn create_layout<E: Engine>(
    ctx: &Ctx<'_, E>,
    set_layouts: &[vk::DescriptorSetLayout],
    push: Option<vk::PushConstantRange>,
    name: &CStr,
) -> VulkanPipelineLayout {
    let mut info = vk::PipelineLayoutCreateInfo::default().set_layouts(set_layouts);
    let ranges = [push.unwrap_or_default()];
    if push.is_some() {
        info = info.push_constant_ranges(&ranges);
    }
    // SAFETY: `info` borrows `set_layouts` and `ranges`, both live across the
    // call; the device is live.
    let handle = match unsafe { ctx.device.create_pipeline_layout(&info, None) } {
        Ok(handle) => handle,
        Err(err) => ctx.vk_fail("vkCreatePipelineLayout", err),
    };
    ctx.name_object(handle, name);
    VulkanPipelineLayout {
        handle,
        push_constant_range: push.unwrap_or_default(),
        mboit_input_attachment_set: 0,
    }
}

/// `R_CreatePipelineLayouts`.
pub fn create_pipeline_layouts<E: Engine>(ctx: &mut Ctx<'_, E>) {
    use vk::ShaderStageFlags as S;
    ctx.engine.sys_printf("Creating pipeline layouts\n");

    let single_texture = vg!(ctx, single_texture_set_layout.handle);
    let mboit_input = vg!(ctx, mboit_input_attachment_set_layout.handle);
    let ubo = vg!(ctx, ubo_set_layout.handle);
    let joints = vg!(ctx, joints_buffer_set_layout.handle);
    let bmodel_instances = vg!(ctx, bmodel_instances_set_layout.handle);
    let input_attachment = vg!(ctx, input_attachment_set_layout.handle);
    let oit_input = vg!(ctx, oit_input_attachment_set_layout.handle);
    let screen_effects = vg!(ctx, screen_effects_set_layout.handle);
    let single_texture_cs_write = vg!(ctx, single_texture_cs_write_set_layout.handle);
    let lightmap_compute = vg!(ctx, lightmap_compute_set_layout.handle);
    let ray_query_push = vg!(ctx, ray_query_push_set_layout.handle);
    let indirect_compute = vg!(ctx, indirect_compute_set_layout.handle);
    let ray_debug = vg!(ctx, ray_debug_set_layout.handle);
    let ray_query = vg!(ctx, ray_query);

    let mut layout = create_layout(
        ctx,
        &[single_texture, mboit_input],
        Some(push_range(S::ALL_GRAPHICS, 22 * FLOAT)),
        c"basic_pipeline_layout",
    );
    layout.mboit_input_attachment_set = 1;
    *vg_mut!(ctx, basic_pipeline_layout) = layout;

    let mut layout = create_layout(
        ctx,
        &[
            single_texture,
            single_texture,
            single_texture,
            mboit_input,
            bmodel_instances,
        ],
        Some(push_range(S::ALL_GRAPHICS, 22 * FLOAT)),
        c"world_pipeline_layout",
    );
    layout.mboit_input_attachment_set = 3;
    *vg_mut!(ctx, world_pipeline_layout) = layout;

    let mut layout = create_layout(
        ctx,
        &[single_texture, single_texture, ubo, mboit_input],
        Some(push_range(S::ALL_GRAPHICS, 22 * FLOAT)),
        c"alias_pipeline_layout",
    );
    layout.mboit_input_attachment_set = 3;
    *vg_mut!(ctx, alias_pipelines[MAIN_RENDER_PASS_STANDARD][0].layout) = layout;

    let mut layout = create_layout(
        ctx,
        &[single_texture, single_texture, ubo, joints, mboit_input],
        Some(push_range(S::ALL_GRAPHICS, 22 * FLOAT)),
        c"md5_pipeline_layout",
    );
    layout.mboit_input_attachment_set = 4;
    *vg_mut!(ctx, md5_pipelines[MAIN_RENDER_PASS_STANDARD][0].layout) = layout;

    *vg_mut!(ctx, sky_pipeline_layout[0]) = create_layout(
        ctx,
        &[single_texture],
        Some(push_range(S::ALL_GRAPHICS, 27 * FLOAT)),
        c"sky_pipeline_layout",
    );
    *vg_mut!(ctx, sky_pipeline_layout[1]) = create_layout(
        ctx,
        &[single_texture, single_texture],
        Some(push_range(S::ALL_GRAPHICS, 25 * FLOAT)),
        c"sky_layer_pipeline_layout",
    );

    *vg_mut!(ctx, postprocess_pipeline.layout) = create_layout(
        ctx,
        &[input_attachment],
        Some(push_range(S::FRAGMENT, 2 * FLOAT)),
        c"postprocess_pipeline_layout",
    );
    *vg_mut!(ctx, wboit_resolve_pipeline.layout) =
        create_layout(ctx, &[oit_input], None, c"wboit_resolve_pipeline_layout");
    let mut layout = create_layout(ctx, &[mboit_input], None, c"mboit_resolve_pipeline_layout");
    layout.mboit_input_attachment_set = -1;
    *vg_mut!(ctx, mboit_resolve_pipeline.layout) = layout;

    let layout = create_layout(
        ctx,
        &[screen_effects],
        Some(push_range(S::COMPUTE, 3 * FLOAT + 8 * FLOAT)),
        c"screen_effects_pipeline_layout",
    );
    *vg_mut!(ctx, screen_effects_pipeline.layout) = layout;
    *vg_mut!(ctx, screen_effects_scale_pipeline.layout) = layout;
    *vg_mut!(ctx, screen_effects_scale_sops_pipeline.layout) = layout;

    *vg_mut!(ctx, cs_tex_warp_pipeline.layout) = create_layout(
        ctx,
        &[single_texture, single_texture_cs_write],
        Some(push_range(S::COMPUTE, FLOAT)),
        c"cs_tex_warp_pipeline_layout",
    );

    *vg_mut!(ctx, showtris_pipeline[MAIN_RENDER_PASS_STANDARD].layout) =
        create_layout(ctx, &[], None, c"showtris_pipeline_layout");

    *vg_mut!(ctx, update_lightmap_pipeline.layout) = create_layout(
        ctx,
        &[lightmap_compute],
        Some(push_range(S::COMPUTE, 11 * 4)),
        c"update_lightmap_pipeline_layout",
    );
    if ray_query {
        *vg_mut!(ctx, update_lightmap_rt_pipeline.layout) = create_layout(
            ctx,
            &[lightmap_compute, ray_query_push],
            Some(push_range(S::COMPUTE, 12 * 4)),
            c"update_lightmap_rt_pipeline_layout",
        );
    }

    *vg_mut!(ctx, indirect_draw_pipeline.layout) = create_layout(
        ctx,
        &[indirect_compute],
        Some(push_range(S::COMPUTE, 7 * 4)),
        c"indirect_draw_pipeline_layout",
    );
    *vg_mut!(ctx, indirect_clear_pipeline.layout) = create_layout(
        ctx,
        &[indirect_compute],
        Some(push_range(S::COMPUTE, 7 * 4)),
        c"indirect_clear_pipeline_layout",
    );

    if ray_query {
        // C: `40 /* sizeof (mesh_interpolate_push_constants_t) */` and
        // `q_max (sizeof (skinning_push_constants_t), sizeof
        // (mesh_interpolate_push_constants_t))` == 48 (the skinning struct
        // pads to 48; `gl_mesh.c` pushes `sizeof (pc)`). The mirrors pin
        // both sizes.
        let mesh_interpolate_pc = size_of::<MeshInterpolatePushConstants>() as u32;
        let skinning_pc = (size_of::<SkinningPushConstants>() as u32).max(mesh_interpolate_pc);
        *vg_mut!(ctx, mesh_interpolate_pipeline.layout) = create_layout(
            ctx,
            &[],
            Some(push_range(S::COMPUTE, mesh_interpolate_pc)),
            c"mesh_interpolate_pipeline_layout",
        );
        *vg_mut!(ctx, skinning_pipeline.layout) = create_layout(
            ctx,
            &[],
            Some(push_range(S::COMPUTE, skinning_pc)),
            c"skinning_pipeline_layout",
        );
        *vg_mut!(ctx, skinning_8_pipeline.layout) = create_layout(
            ctx,
            &[],
            Some(push_range(S::COMPUTE, skinning_pc)),
            c"skinning_8_pipeline_layout",
        );
    }

    if cfg!(feature = "engine-debug") && ray_query {
        *vg_mut!(ctx, ray_debug_pipeline.layout) = create_layout(
            ctx,
            &[ray_debug, ray_query_push],
            Some(push_range(S::COMPUTE, 15 * FLOAT)),
            c"ray_debug_pipeline_layout",
        );
    }
}

// ---------------------------------------------------------------------------
// Pipeline families
// ---------------------------------------------------------------------------

/// The per-family read-only view of `vulkan_globals` (copied out so the
/// family can write pipelines back through `ctx.vg`).
#[derive(Clone, Copy)]
struct Env {
    /// `main_render_pass[variant][MAIN_RENDER_PASS_STENCIL_CLEAR]`.
    main_rp: [vk::RenderPass; MAIN_RENDER_PASS_VARIANT_COUNT],
    warp_rp: vk::RenderPass,
    gui_rp: vk::RenderPass,
    sample_count: vk::SampleCountFlags,
    msaa: bool,
    non_solid_fill: bool,
    ray_query: bool,
    ten_bit: bool,
    screen_effects_sops: bool,
    basic_layout: VulkanPipelineLayout,
    world_layout: VulkanPipelineLayout,
}

impl Env {
    fn new(vg: &VulkanGlobals) -> Self {
        let cbx = vg.secondary_cb_contexts[SCBX_GUI];
        // SAFETY: `gl_vidsdl.c` points every `secondary_cb_contexts` slot at
        // a live `cb_context_t` (and creates the UI render pass) before
        // `R_CreatePipelines` runs, and those contexts outlive the device.
        let gui_rp = unsafe { (*cbx).render_pass };
        Env {
            main_rp: core::array::from_fn(|variant| {
                vg.main_render_pass[variant][MAIN_RENDER_PASS_STENCIL_CLEAR]
            }),
            warp_rp: vg.warp_render_pass,
            gui_rp,
            sample_count: vg.sample_count,
            msaa: vg.sample_count != vk::SampleCountFlags::TYPE_1,
            non_solid_fill: vg.non_solid_fill,
            ray_query: vg.ray_query,
            ten_bit: vg.color_format == vk::Format::A2B10G10R10_UNORM_PACK32,
            screen_effects_sops: vg.screen_effects_sops,
            basic_layout: vg.basic_pipeline_layout,
            world_layout: vg.world_pipeline_layout,
        }
    }

    /// `sample_count == 1 ? single : msaa`.
    fn pick(&self, single: vk::ShaderModule, msaa: vk::ShaderModule) -> vk::ShaderModule {
        if self.msaa {
            msaa
        } else {
            single
        }
    }

    /// The name suffix `R_CreateSkyPipelines`/`R_CreateShowTrisPipelines` use.
    fn variant_suffix(variant: usize) -> &'static str {
        match variant {
            MAIN_RENDER_PASS_MBOIT => "_main_mboit",
            MAIN_RENDER_PASS_OIT => "_main_oit",
            _ => "",
        }
    }
}

/// `R_CreateBasicPipelines`.
fn create_basic<E: Engine>(
    ctx: &mut Ctx<'_, E>,
    env: &Env,
    m: &ShaderModules,
    base: &PipelineInfos,
) {
    struct Variant {
        index: usize,
        render_pass: vk::RenderPass,
        samples: vk::SampleCountFlags,
        attachments: u32,
        subpass: u32,
    }
    let sc = env.sample_count;
    let variants = [
        Variant {
            index: RENDER_PASS_INDEX_MAIN,
            render_pass: env.main_rp[MAIN_RENDER_PASS_STANDARD],
            samples: sc,
            attachments: 1,
            subpass: 0,
        },
        Variant {
            index: RENDER_PASS_INDEX_UI,
            render_pass: env.gui_rp,
            samples: vk::SampleCountFlags::TYPE_1,
            attachments: 1,
            subpass: 0,
        },
        Variant {
            index: RENDER_PASS_INDEX_MAIN_OIT,
            render_pass: env.main_rp[MAIN_RENDER_PASS_OIT],
            samples: sc,
            attachments: 1,
            subpass: 0,
        },
        Variant {
            index: RENDER_PASS_INDEX_MAIN_MBOIT,
            render_pass: env.main_rp[MAIN_RENDER_PASS_MBOIT],
            samples: sc,
            attachments: 1,
            subpass: 0,
        },
        Variant {
            index: RENDER_PASS_INDEX_WBOIT,
            render_pass: env.main_rp[MAIN_RENDER_PASS_OIT],
            samples: sc,
            attachments: 2,
            subpass: 1,
        },
        Variant {
            index: RENDER_PASS_INDEX_MBOIT_MOMENTS,
            render_pass: env.main_rp[MAIN_RENDER_PASS_MBOIT],
            samples: sc,
            attachments: 2,
            subpass: 1,
        },
        Variant {
            index: RENDER_PASS_INDEX_MBOIT_COMPOSITE,
            render_pass: env.main_rp[MAIN_RENDER_PASS_MBOIT],
            samples: sc,
            attachments: 1,
            subpass: 2,
        },
    ];
    debug_assert_eq!(variants.len(), RENDER_PASS_INDEX_COUNT);
    let apply = |infos: &mut PipelineInfos, v: &Variant| {
        infos.set_render_pass(v.render_pass, v.subpass, v.attachments);
        infos.multisample.rasterization_samples = v.samples;
    };

    for v in &variants {
        let mut infos = *base;
        apply(&mut infos, v);
        infos.set_fragment(m.get(Shader::basic_alphatest_frag));
        *vg_mut!(ctx, basic_alphatest_pipeline[v.index]) =
            create_graphics(ctx, &mut infos, env.basic_layout, c"basic_alphatest");
    }
    for v in &variants {
        let mut infos = *base;
        apply(&mut infos, v);
        infos.set_fragment(m.get(Shader::basic_notex_frag));
        infos.blend_attachments[0].blend_enable = vk::TRUE;
        *vg_mut!(ctx, basic_notex_blend_pipeline[v.index]) =
            create_graphics(ctx, &mut infos, env.basic_layout, c"basic_notex_blend");
    }
    for v in &variants {
        let mut infos = *base;
        apply(&mut infos, v);
        match v.index {
            RENDER_PASS_INDEX_WBOIT => {
                infos.set_fragment(m.get(Shader::basic_oit_frag));
                set_wboit_blend(&mut infos.blend_attachments);
            }
            RENDER_PASS_INDEX_MBOIT_MOMENTS => {
                infos.set_fragment(m.get(Shader::basic_mboit_moment_frag));
                set_mboit_moment_blend(&mut infos.blend_attachments);
            }
            RENDER_PASS_INDEX_MBOIT_COMPOSITE => {
                infos.set_fragment(env.pick(
                    m.get(Shader::basic_mboit_composite_frag),
                    m.get(Shader::basic_mboit_composite_msaa_frag),
                ));
                set_mboit_composite_blend(&mut infos.blend_attachments);
            }
            _ => {
                infos.set_fragment(m.get(Shader::basic_frag));
                infos.blend_attachments[0].blend_enable = vk::TRUE;
            }
        }
        *vg_mut!(ctx, basic_blend_pipeline[v.index]) =
            create_graphics(ctx, &mut infos, env.basic_layout, c"basic_blend");
    }
}

/// `R_CreateWarpPipelines`.
fn create_warp<E: Engine>(
    ctx: &mut Ctx<'_, E>,
    env: &Env,
    m: &ShaderModules,
    base: &PipelineInfos,
) {
    let mut infos = *base;
    infos.multisample.rasterization_samples = vk::SampleCountFlags::TYPE_1;
    infos.input_assembly.topology = vk::PrimitiveTopology::TRIANGLE_STRIP;
    infos.rasterization.front_face = vk::FrontFace::COUNTER_CLOCKWISE;
    infos.color_blend.attachment_count = 1;
    infos.graphics.render_pass = env.warp_rp;
    *vg_mut!(ctx, raster_tex_warp_pipeline) =
        create_graphics(ctx, &mut infos, env.basic_layout, c"warp");

    let layout = vg!(ctx, cs_tex_warp_pipeline.layout);
    *vg_mut!(ctx, cs_tex_warp_pipeline) = create_compute(
        ctx,
        layout,
        m.get(Shader::cs_tex_warp_comp),
        vk::PipelineShaderStageCreateFlags::empty(),
        None,
        c"cs_tex_warp",
    );
}

/// `R_CreateParticlesPipelines`.
fn create_particles<E: Engine>(
    ctx: &mut Ctx<'_, E>,
    env: &Env,
    m: &ShaderModules,
    base: &PipelineInfos,
) {
    let mut family = *base;
    family.set_depth(true, false);
    family.blend_attachments[0].blend_enable = vk::TRUE;

    let mut infos = family;
    *vg_mut!(ctx, particle_pipeline) =
        create_graphics(ctx, &mut infos, env.basic_layout, c"particles");

    for variant in MAIN_RENDER_PASS_OIT..=MAIN_RENDER_PASS_MBOIT {
        let mut infos = family;
        infos.graphics.render_pass = env.main_rp[variant];
        infos.graphics.subpass = if variant == MAIN_RENDER_PASS_MBOIT {
            3
        } else {
            2
        };
        let name = if variant == MAIN_RENDER_PASS_MBOIT {
            c"particles_post_mboit"
        } else {
            c"particles_post_oit"
        };
        *vg_mut!(ctx, particle_post_oit_pipeline[variant]) =
            create_graphics(ctx, &mut infos, env.basic_layout, name);
    }

    let mut infos = family;
    infos.set_render_pass(env.main_rp[MAIN_RENDER_PASS_OIT], 1, 2);
    infos.set_fragment(m.get(Shader::basic_oit_frag));
    set_wboit_blend(&mut infos.blend_attachments);
    *vg_mut!(ctx, particle_oit_pipeline) =
        create_graphics(ctx, &mut infos, env.basic_layout, c"particles_oit");

    let mut infos = family;
    infos.set_render_pass(env.main_rp[MAIN_RENDER_PASS_MBOIT], 1, 2);
    infos.set_fragment(m.get(Shader::basic_mboit_moment_frag));
    set_mboit_moment_blend(&mut infos.blend_attachments);
    *vg_mut!(ctx, particle_mboit_moment_pipeline) =
        create_graphics(ctx, &mut infos, env.basic_layout, c"particles_mboit_moment");

    let mut infos = family;
    infos.set_render_pass(env.main_rp[MAIN_RENDER_PASS_MBOIT], 2, 1);
    infos.set_fragment(env.pick(
        m.get(Shader::basic_mboit_composite_frag),
        m.get(Shader::basic_mboit_composite_msaa_frag),
    ));
    set_mboit_composite_blend(&mut infos.blend_attachments);
    *vg_mut!(ctx, particle_mboit_composite_pipeline) = create_graphics(
        ctx,
        &mut infos,
        env.basic_layout,
        c"particles_mboit_composite",
    );
}

/// `R_CreateFTEParticlesPipelines`.
fn create_fte_particles<E: Engine>(
    ctx: &mut Ctx<'_, E>,
    env: &Env,
    m: &ShaderModules,
    base: &PipelineInfos,
) {
    const NAMES: [&str; 8] = [
        "fte_particles_blend",
        "fte_particles_blend_color",
        "fte_particles_add_color",
        "fte_particles_add_alpha",
        "fte_particles_subtract",
        "fte_particles_inv_modulate_alpha",
        "fte_particles_inv_modulate_color",
        "fte_particles_premultiplied",
    ];
    let mut family = *base;
    family.rasterization.cull_mode = vk::CullModeFlags::NONE;
    family.set_depth_bias(OFFSET_DECAL, 1.0);
    family.set_depth(true, false);
    family.multisample.sample_shading_enable = vk::FALSE;

    let num_topologies = if env.non_solid_fill { 2 } else { 1 };
    for (i, base_name) in NAMES.iter().enumerate() {
        for lines in 0..num_topologies {
            let mode = i + lines * 8;
            let name = if lines == 1 {
                format!("{base_name}_lines")
            } else {
                format!("{base_name}_tris")
            };
            let mut mode_base = family;
            if lines == 1 {
                mode_base.input_assembly.topology = vk::PrimitiveTopology::LINE_LIST;
                mode_base.rasterization.polygon_mode = vk::PolygonMode::LINE;
            }

            for variant in 0..MAIN_RENDER_PASS_VARIANT_COUNT {
                let mut infos = mode_base;
                infos.graphics.render_pass = env.main_rp[variant];
                infos.blend_attachments[0] = fte_particle_blend(i);
                let pipeline_name = if variant == MAIN_RENDER_PASS_STANDARD {
                    c_string(&name)
                } else {
                    c_string(&format!("{name}_main_oit"))
                };
                *vg_mut!(ctx, fte_particle_pipelines[variant][mode]) =
                    create_graphics(ctx, &mut infos, env.basic_layout, &pipeline_name);
            }

            let mut infos = mode_base;
            infos.set_render_pass(env.main_rp[MAIN_RENDER_PASS_OIT], 1, 2);
            infos.set_fragment(m.get(Shader::basic_oit_frag));
            set_wboit_blend(&mut infos.blend_attachments);
            let pipeline_name = c_string(&format!("{name}_wboit"));
            *vg_mut!(ctx, fte_particle_wboit_pipelines[mode]) =
                create_graphics(ctx, &mut infos, env.basic_layout, &pipeline_name);

            for variant in MAIN_RENDER_PASS_OIT..=MAIN_RENDER_PASS_MBOIT {
                let mut infos = mode_base;
                infos.graphics.render_pass = env.main_rp[variant];
                infos.graphics.subpass = if variant == MAIN_RENDER_PASS_MBOIT {
                    3
                } else {
                    2
                };
                infos.blend_attachments[0] = fte_particle_blend(i);
                let pipeline_name = c_string(&format!("{name}_post_oit"));
                *vg_mut!(ctx, fte_particle_post_oit_pipelines[variant][mode]) =
                    create_graphics(ctx, &mut infos, env.basic_layout, &pipeline_name);
            }
        }
    }
}

/// `R_CreateSpritesPipelines`.
fn create_sprites<E: Engine>(
    ctx: &mut Ctx<'_, E>,
    env: &Env,
    m: &ShaderModules,
    base: &PipelineInfos,
) {
    let mut family = *base;
    family.set_fragment(m.get(Shader::basic_alphatest_frag));
    family.set_depth(true, true);
    family.add_dynamic_depth_bias();

    for variant in 0..MAIN_RENDER_PASS_VARIANT_COUNT {
        let mut infos = family;
        infos.graphics.render_pass = env.main_rp[variant];
        let name = if variant == MAIN_RENDER_PASS_STANDARD {
            c"sprite"
        } else {
            c"sprite_main_oit"
        };
        *vg_mut!(ctx, sprite_pipeline[variant]) =
            create_graphics(ctx, &mut infos, env.basic_layout, name);
    }

    let mut infos = family;
    infos.set_render_pass(env.main_rp[MAIN_RENDER_PASS_OIT], 1, 2);
    infos.set_fragment(m.get(Shader::basic_oit_frag));
    set_wboit_blend(&mut infos.blend_attachments);
    *vg_mut!(ctx, sprite_oit_pipeline) =
        create_graphics(ctx, &mut infos, env.basic_layout, c"sprite_oit");

    let mut infos = family;
    infos.set_render_pass(env.main_rp[MAIN_RENDER_PASS_MBOIT], 1, 2);
    infos.set_fragment(m.get(Shader::basic_mboit_moment_frag));
    set_mboit_moment_blend(&mut infos.blend_attachments);
    *vg_mut!(ctx, sprite_mboit_moment_pipeline) =
        create_graphics(ctx, &mut infos, env.basic_layout, c"sprite_mboit_moment");

    let mut infos = family;
    infos.set_render_pass(env.main_rp[MAIN_RENDER_PASS_MBOIT], 2, 1);
    infos.set_fragment(env.pick(
        m.get(Shader::basic_mboit_composite_frag),
        m.get(Shader::basic_mboit_composite_msaa_frag),
    ));
    set_mboit_composite_blend(&mut infos.blend_attachments);
    *vg_mut!(ctx, sprite_mboit_composite_pipeline) =
        create_graphics(ctx, &mut infos, env.basic_layout, c"sprite_mboit_composite");
}

/// `R_CreateSkyPipelines`.
fn create_sky<E: Engine>(ctx: &mut Ctx<'_, E>, env: &Env, m: &ShaderModules, base: &PipelineInfos) {
    let sky_layout = vg!(ctx, sky_pipeline_layout);
    for i in 0..2 {
        let mut family = *base;
        if i == 1 {
            family.set_vertex_input(&WORLD_INPUT);
        }
        family.set_depth(true, true);
        let indirect = if i == 1 { "_indirect" } else { "" };

        for variant in 0..MAIN_RENDER_PASS_VARIANT_COUNT {
            let rp = env.main_rp[variant];
            let suffix = Env::variant_suffix(variant);

            let mut infos = family;
            infos.graphics.render_pass = rp;
            infos.graphics.stage_count = 1;
            infos.set_fragment(vk::ShaderModule::null());
            infos.depth_stencil.stencil_test_enable = vk::TRUE;
            infos.depth_stencil.front = vk::StencilOpState {
                fail_op: vk::StencilOp::KEEP,
                pass_op: vk::StencilOp::REPLACE,
                depth_fail_op: vk::StencilOp::KEEP,
                compare_op: vk::CompareOp::ALWAYS,
                compare_mask: 0xFF,
                write_mask: 0xFF,
                reference: 1,
            };
            infos.blend_attachments[0].color_write_mask = vk::ColorComponentFlags::empty();
            let name = c_string(&format!("sky_stencil{indirect}{suffix}"));
            *vg_mut!(ctx, sky_stencil_pipeline[variant][i]) =
                create_graphics(ctx, &mut infos, sky_layout[0], &name);

            let mut infos = family;
            infos.graphics.render_pass = rp;
            infos.set_fragment(m.get(Shader::basic_notex_frag));
            let name = c_string(&format!("sky_color{indirect}{suffix}"));
            *vg_mut!(ctx, sky_color_pipeline[variant][i]) =
                create_graphics(ctx, &mut infos, sky_layout[0], &name);

            let mut infos = family;
            infos.graphics.render_pass = rp;
            infos.set_vertex(m.get(Shader::sky_cube_vert));
            infos.set_fragment(m.get(Shader::sky_cube_frag));
            let name = c_string(&format!("sky_cube{indirect}{suffix}"));
            *vg_mut!(ctx, sky_cube_pipeline[variant][i]) =
                create_graphics(ctx, &mut infos, sky_layout[0], &name);

            let mut infos = family;
            infos.graphics.render_pass = rp;
            infos.set_vertex(m.get(Shader::sky_layer_vert));
            infos.set_fragment(m.get(Shader::sky_layer_frag));
            let name = c_string(&format!("sky_layer{indirect}{suffix}"));
            *vg_mut!(ctx, sky_layer_pipeline[variant][i]) =
                create_graphics(ctx, &mut infos, sky_layout[1], &name);

            if i == 0 {
                let mut infos = family;
                infos.graphics.render_pass = rp;
                infos.set_depth(false, false);
                infos.depth_stencil.stencil_test_enable = vk::TRUE;
                infos.depth_stencil.front = vk::StencilOpState {
                    fail_op: vk::StencilOp::KEEP,
                    pass_op: vk::StencilOp::KEEP,
                    depth_fail_op: vk::StencilOp::KEEP,
                    compare_op: vk::CompareOp::EQUAL,
                    compare_mask: 0xFF,
                    write_mask: 0,
                    reference: 1,
                };
                infos.set_fragment(m.get(Shader::sky_box_frag));
                let name = c_string(&format!("sky_box{suffix}"));
                *vg_mut!(ctx, sky_box_pipeline[variant]) =
                    create_graphics(ctx, &mut infos, sky_layout[0], &name);
            }
        }
    }
}

/// `R_CreateShowTrisPipelines`.
fn create_showtris<E: Engine>(
    ctx: &mut Ctx<'_, E>,
    env: &Env,
    m: &ShaderModules,
    base: &PipelineInfos,
) {
    if !env.non_solid_fill {
        return;
    }
    let mut family = *base;
    family.rasterization.cull_mode = vk::CullModeFlags::NONE;
    family.rasterization.polygon_mode = vk::PolygonMode::LINE;
    family.set_vertex_input(&SHOWTRIS_INPUT);
    family.set_vertex(m.get(Shader::showtris_vert));
    family.set_fragment(m.get(Shader::showtris_frag));

    for variant in 0..MAIN_RENDER_PASS_VARIANT_COUNT {
        let rp = env.main_rp[variant];
        let suffix = Env::variant_suffix(variant);

        let mut infos = family;
        infos.graphics.render_pass = rp;
        let name = c_string(&format!("showtris{suffix}"));
        *vg_mut!(ctx, showtris_pipeline[variant]) =
            create_graphics(ctx, &mut infos, env.basic_layout, &name);

        let mut infos = family;
        infos.graphics.render_pass = rp;
        infos.depth_stencil.depth_test_enable = vk::TRUE;
        infos.set_depth_bias(500.0, 0.0);
        let name = c_string(&format!("showtris_depth_test{suffix}"));
        *vg_mut!(ctx, showtris_depth_test_pipeline[variant]) =
            create_graphics(ctx, &mut infos, env.basic_layout, &name);

        let mut infos = family;
        infos.graphics.render_pass = rp;
        infos.input_assembly.topology = vk::PrimitiveTopology::LINE_LIST;
        let name = c_string(&format!("showbboxes{suffix}"));
        *vg_mut!(ctx, showbboxes_pipeline[variant]) =
            create_graphics(ctx, &mut infos, env.basic_layout, &name);

        let mut infos = family;
        infos.graphics.render_pass = rp;
        infos.set_vertex(m.get(Shader::world_vert));
        infos.set_vertex_input(&WORLD_INPUT);
        let name = c_string(&format!("showtris_indirect{suffix}"));
        *vg_mut!(ctx, showtris_indirect_pipeline[variant]) =
            create_graphics(ctx, &mut infos, env.world_layout, &name);

        let mut infos = family;
        infos.graphics.render_pass = rp;
        infos.set_vertex(m.get(Shader::world_vert));
        infos.set_vertex_input(&WORLD_INPUT);
        infos.depth_stencil.depth_test_enable = vk::TRUE;
        infos.set_depth_bias(500.0, 0.0);
        let name = c_string(&format!("showtris_indirect_depth_test{suffix}"));
        *vg_mut!(ctx, showtris_indirect_depth_test_pipeline[variant]) =
            create_graphics(ctx, &mut infos, env.world_layout, &name);
    }
}

/// The five `world_frag` specialization constants, one `uint32_t` each.
static WORLD_SPEC_ENTRIES: [vk::SpecializationMapEntry; 5] = [
    vk::SpecializationMapEntry {
        constant_id: 0,
        offset: 0,
        size: 4,
    },
    vk::SpecializationMapEntry {
        constant_id: 1,
        offset: 4,
        size: 4,
    },
    vk::SpecializationMapEntry {
        constant_id: 2,
        offset: 8,
        size: 4,
    },
    vk::SpecializationMapEntry {
        constant_id: 3,
        offset: 12,
        size: 4,
    },
    vk::SpecializationMapEntry {
        constant_id: 4,
        offset: 16,
        size: 4,
    },
];

fn spec_info<'a>(
    entries: &'a [vk::SpecializationMapEntry],
    data: &'a [u32],
) -> vk::SpecializationInfo<'a> {
    vk::SpecializationInfo {
        map_entry_count: entries.len() as u32,
        p_map_entries: entries.as_ptr(),
        data_size: size_of_val(data),
        p_data: data.as_ptr().cast(),
        _marker: core::marker::PhantomData,
    }
}

/// Aim `shader_stages[1].pSpecializationInfo` at `spec`; `infos` (and every
/// copy of it) is then bound to `spec`'s lifetime.
fn set_fragment_spec<'a>(infos: &mut PipelineInfos<'a>, spec: &'a vk::SpecializationInfo<'a>) {
    infos.shader_stages[1].p_specialization_info = spec;
}

/// `R_CreateWorldPipelines`.
fn create_world<E: Engine>(
    ctx: &mut Ctx<'_, E>,
    env: &Env,
    m: &ShaderModules,
    base: &PipelineInfos,
) {
    let mut family = *base;
    family.set_depth(true, true);
    family.rasterization.depth_bias_enable = vk::TRUE;
    family.add_dynamic_depth_bias();
    family.set_vertex_input(&WORLD_INPUT);
    family.set_vertex(m.get(Shader::world_vert));

    for alpha_blend in 0..2u32 {
        for alpha_test in 0..2u32 {
            for fullbright in 0..2u32 {
                for quantize_lm in 0..2u32 {
                    let idx =
                        (fullbright + alpha_test * 2 + alpha_blend * 4 + quantize_lm * 8) as usize;
                    let data = [
                        fullbright,
                        alpha_test,
                        alpha_blend,
                        quantize_lm,
                        env.ten_bit as u32,
                    ];
                    let spec = spec_info(&WORLD_SPEC_ENTRIES, &data);
                    let blend = alpha_blend != 0;

                    for variant in 0..MAIN_RENDER_PASS_VARIANT_COUNT {
                        let mut infos = family;
                        infos.graphics.render_pass = env.main_rp[variant];
                        infos.set_fragment(m.get(Shader::world_frag));
                        set_fragment_spec(&mut infos, &spec);
                        infos.blend_attachments[0].blend_enable = blend as vk::Bool32;
                        infos.depth_stencil.depth_write_enable = (!blend) as vk::Bool32;
                        let name = if variant == MAIN_RENDER_PASS_STANDARD {
                            c_string(&format!("world {idx}"))
                        } else {
                            c_string(&format!("world_main_oit {idx}"))
                        };
                        *vg_mut!(ctx, world_pipelines[variant][idx]) =
                            create_graphics(ctx, &mut infos, env.world_layout, &name);
                    }

                    if blend {
                        let mut infos = family;
                        infos.set_render_pass(env.main_rp[MAIN_RENDER_PASS_OIT], 1, 2);
                        infos.set_fragment(m.get(Shader::world_oit_frag));
                        set_fragment_spec(&mut infos, &spec);
                        infos.depth_stencil.depth_write_enable = vk::FALSE;
                        set_wboit_blend(&mut infos.blend_attachments);
                        let name = c_string(&format!("world_wboit {idx}"));
                        *vg_mut!(ctx, world_wboit_pipelines[idx]) =
                            create_graphics(ctx, &mut infos, env.world_layout, &name);

                        let mut infos = family;
                        infos.set_render_pass(env.main_rp[MAIN_RENDER_PASS_MBOIT], 1, 2);
                        infos.set_fragment(m.get(Shader::world_mboit_moment_frag));
                        set_fragment_spec(&mut infos, &spec);
                        infos.depth_stencil.depth_write_enable = vk::FALSE;
                        set_mboit_moment_blend(&mut infos.blend_attachments);
                        let name = c_string(&format!("world_mboit_moment {idx}"));
                        *vg_mut!(ctx, world_mboit_moment_pipelines[idx]) =
                            create_graphics(ctx, &mut infos, env.world_layout, &name);

                        let mut infos = family;
                        infos.set_render_pass(env.main_rp[MAIN_RENDER_PASS_MBOIT], 2, 1);
                        infos.set_fragment(env.pick(
                            m.get(Shader::world_mboit_composite_frag),
                            m.get(Shader::world_mboit_composite_msaa_frag),
                        ));
                        set_fragment_spec(&mut infos, &spec);
                        infos.depth_stencil.depth_write_enable = vk::FALSE;
                        set_mboit_composite_blend(&mut infos.blend_attachments);
                        let name = c_string(&format!("world_mboit_composite {idx}"));
                        *vg_mut!(ctx, world_mboit_composite_pipelines[idx]) =
                            create_graphics(ctx, &mut infos, env.world_layout, &name);
                    }
                }
            }
        }
    }
}

/// The per-model-family inputs shared by `R_CreateAliasPipelines` and
/// `R_CreateMD5PipelineSet`.
struct ModelFamily {
    input: &'static VertexInput,
    vert: Shader,
    layout: VulkanPipelineLayout,
    /// `[alphatest, plain]` composite modules, `[single, msaa]` each.
    composite: [[Shader; 2]; 2],
    name: &'static str,
}

/// Which `vulkan_globals` arrays a model family writes.
#[derive(Clone, Copy)]
enum ModelSet {
    Alias,
    Md5,
    Md5x8,
}

/// Writes `pipeline` into the `set`/`variant`/`idx` slot of `vulkan_globals`
/// (one field-level store, see `VgPtr`).
fn set_model_slot<E: Engine>(
    ctx: &mut Ctx<'_, E>,
    set: ModelSet,
    variant: usize,
    idx: usize,
    pipeline: VulkanPipeline,
) {
    match set {
        ModelSet::Alias => *vg_mut!(ctx, alias_pipelines[variant][idx]) = pipeline,
        ModelSet::Md5 => *vg_mut!(ctx, md5_pipelines[variant][idx]) = pipeline,
        ModelSet::Md5x8 => *vg_mut!(ctx, md5_8_pipelines[variant][idx]) = pipeline,
    }
}

/// Writes the `[wboit, mboit_moment, mboit_composite]` pipelines of `set`.
fn set_model_oit_slots<E: Engine>(
    ctx: &mut Ctx<'_, E>,
    set: ModelSet,
    idx: usize,
    [wboit, moment, composite]: [VulkanPipeline; 3],
) {
    match set {
        ModelSet::Alias => {
            *vg_mut!(ctx, alias_wboit_pipelines[idx]) = wboit;
            *vg_mut!(ctx, alias_mboit_moment_pipelines[idx]) = moment;
            *vg_mut!(ctx, alias_mboit_composite_pipelines[idx]) = composite;
        }
        ModelSet::Md5 => {
            *vg_mut!(ctx, md5_wboit_pipelines[idx]) = wboit;
            *vg_mut!(ctx, md5_mboit_moment_pipelines[idx]) = moment;
            *vg_mut!(ctx, md5_mboit_composite_pipelines[idx]) = composite;
        }
        ModelSet::Md5x8 => {
            *vg_mut!(ctx, md5_8_wboit_pipelines[idx]) = wboit;
            *vg_mut!(ctx, md5_8_mboit_moment_pipelines[idx]) = moment;
            *vg_mut!(ctx, md5_8_mboit_composite_pipelines[idx]) = composite;
        }
    }
}

/// `R_CreateAliasPipelines` / `R_CreateMD5PipelineSet`: the shared body.
fn create_model_family<E: Engine>(
    ctx: &mut Ctx<'_, E>,
    env: &Env,
    m: &ShaderModules,
    base: &PipelineInfos,
    set: ModelSet,
    family_desc: &ModelFamily,
) {
    let mut family = *base;
    family.set_depth(true, true);
    family.set_vertex_input(family_desc.input);
    family.set_vertex(m.get(family_desc.vert));
    let layout = family_desc.layout;
    let name = family_desc.name;

    for idx in 0..4usize {
        let alpha_test = idx & 1 != 0;
        let alpha_blend = idx & 2 != 0;

        for variant in 0..MAIN_RENDER_PASS_VARIANT_COUNT {
            let mut infos = family;
            infos.graphics.render_pass = env.main_rp[variant];
            infos.set_fragment(m.get(if alpha_test {
                Shader::alias_alphatest_frag
            } else {
                Shader::alias_frag
            }));
            infos.blend_attachments[0].blend_enable = alpha_blend as vk::Bool32;
            infos.depth_stencil.depth_write_enable = (!alpha_blend) as vk::Bool32;
            let pipeline_name = if variant == MAIN_RENDER_PASS_STANDARD {
                c_string(&format!("{name} {idx}"))
            } else {
                c_string(&format!("{name}_main_oit {idx}"))
            };
            let pipeline = create_graphics(ctx, &mut infos, layout, &pipeline_name);
            set_model_slot(ctx, set, variant, idx, pipeline);
        }

        if alpha_blend {
            let mut infos = family;
            infos.set_render_pass(env.main_rp[MAIN_RENDER_PASS_OIT], 1, 2);
            infos.set_fragment(m.get(if alpha_test {
                Shader::alias_alphatest_oit_frag
            } else {
                Shader::alias_oit_frag
            }));
            infos.depth_stencil.depth_write_enable = vk::FALSE;
            set_wboit_blend(&mut infos.blend_attachments);
            let pipeline_name = c_string(&format!("{name}_wboit {idx}"));
            let wboit = create_graphics(ctx, &mut infos, layout, &pipeline_name);

            let mut infos = family;
            infos.set_render_pass(env.main_rp[MAIN_RENDER_PASS_MBOIT], 1, 2);
            infos.set_fragment(m.get(if alpha_test {
                Shader::alias_alphatest_mboit_moment_frag
            } else {
                Shader::alias_mboit_moment_frag
            }));
            infos.depth_stencil.depth_write_enable = vk::FALSE;
            set_mboit_moment_blend(&mut infos.blend_attachments);
            let pipeline_name = c_string(&format!("{name}_mboit_moment {idx}"));
            let moment = create_graphics(ctx, &mut infos, layout, &pipeline_name);

            let mut infos = family;
            infos.set_render_pass(env.main_rp[MAIN_RENDER_PASS_MBOIT], 2, 1);
            let [single, msaa] = family_desc.composite[if alpha_test { 0 } else { 1 }];
            infos.set_fragment(env.pick(m.get(single), m.get(msaa)));
            infos.depth_stencil.depth_write_enable = vk::FALSE;
            set_mboit_composite_blend(&mut infos.blend_attachments);
            let pipeline_name = c_string(&format!("{name}_mboit_composite {idx}"));
            let composite = create_graphics(ctx, &mut infos, layout, &pipeline_name);

            set_model_oit_slots(ctx, set, idx, [wboit, moment, composite]);
        }
    }

    if env.non_solid_fill {
        for idx in 4..=5usize {
            let depth_test = idx == 5;
            for variant in 0..MAIN_RENDER_PASS_VARIANT_COUNT {
                let mut infos = family;
                infos.graphics.render_pass = env.main_rp[variant];
                infos.rasterization.cull_mode = vk::CullModeFlags::NONE;
                infos.rasterization.polygon_mode = vk::PolygonMode::LINE;
                infos.set_depth(depth_test, false);
                infos.rasterization.depth_bias_enable = depth_test as vk::Bool32;
                infos.rasterization.depth_bias_constant_factor = 500.0;
                infos.rasterization.depth_bias_slope_factor = 0.0;
                infos.set_fragment(m.get(Shader::showtris_frag));
                let pipeline_name = if variant == MAIN_RENDER_PASS_STANDARD {
                    c_string(&format!("{name}_showtris {idx}"))
                } else {
                    c_string(&format!("{name}_showtris_main_oit {idx}"))
                };
                let pipeline = create_graphics(ctx, &mut infos, layout, &pipeline_name);
                set_model_slot(ctx, set, variant, idx, pipeline);
            }
        }
    }
}

/// `R_CreateAliasPipelines`.
fn create_alias<E: Engine>(
    ctx: &mut Ctx<'_, E>,
    env: &Env,
    m: &ShaderModules,
    base: &PipelineInfos,
) {
    let desc = ModelFamily {
        input: &ALIAS_INPUT,
        vert: Shader::alias_vert,
        layout: vg!(ctx, alias_pipelines[MAIN_RENDER_PASS_STANDARD][0].layout),
        composite: [
            [
                Shader::alias_alphatest_mboit_composite_frag,
                Shader::alias_alphatest_mboit_composite_msaa_frag,
            ],
            [
                Shader::alias_mboit_composite_frag,
                Shader::alias_mboit_composite_msaa_frag,
            ],
        ],
        name: "alias",
    };
    create_model_family(ctx, env, m, base, ModelSet::Alias, &desc);
}

/// `R_CreateMD5Pipelines`: both sets share `md5_pipelines[STANDARD][0].layout`.
fn create_md5<E: Engine>(ctx: &mut Ctx<'_, E>, env: &Env, m: &ShaderModules, base: &PipelineInfos) {
    let layout = vg!(ctx, md5_pipelines[MAIN_RENDER_PASS_STANDARD][0].layout);
    let composite = [
        [
            Shader::md5_alphatest_mboit_composite_frag,
            Shader::md5_alphatest_mboit_composite_msaa_frag,
        ],
        [
            Shader::md5_mboit_composite_frag,
            Shader::md5_mboit_composite_msaa_frag,
        ],
    ];
    let md5 = ModelFamily {
        input: &MD5_INPUT,
        vert: Shader::md5_vert,
        layout,
        composite,
        name: "md5",
    };
    create_model_family(ctx, env, m, base, ModelSet::Md5, &md5);
    let md5_8 = ModelFamily {
        input: &MD5_8_INPUT,
        vert: Shader::md5_8_vert,
        layout,
        composite,
        name: "md5_8",
    };
    create_model_family(ctx, env, m, base, ModelSet::Md5x8, &md5_8);
}

/// `R_CreatePostprocessPipelines`.
fn create_postprocess<E: Engine>(
    ctx: &mut Ctx<'_, E>,
    env: &Env,
    m: &ShaderModules,
    base: &PipelineInfos,
) {
    let mut family = *base;
    family.rasterization.cull_mode = vk::CullModeFlags::NONE;
    family.color_blend.attachment_count = 1;
    family.set_vertex_input(&EMPTY_INPUT);
    family.set_vertex(m.get(Shader::postprocess_vert));

    let mut infos = family;
    infos.multisample.rasterization_samples = vk::SampleCountFlags::TYPE_1;
    infos.set_depth(true, true);
    infos.set_fragment(m.get(Shader::postprocess_frag));
    infos.graphics.render_pass = env.gui_rp;
    infos.graphics.subpass = 1;
    let layout = vg!(ctx, postprocess_pipeline.layout);
    *vg_mut!(ctx, postprocess_pipeline) = create_graphics(ctx, &mut infos, layout, c"postprocess");

    let resolve_blend = blend(
        true,
        RGBA,
        vk::BlendFactor::SRC_ALPHA,
        vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
        vk::BlendFactor::ONE,
        vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
    );

    let mut infos = family;
    infos.blend_attachments[0] = resolve_blend;
    infos.set_fragment(env.pick(
        m.get(Shader::wboit_resolve_frag),
        m.get(Shader::wboit_resolve_msaa_frag),
    ));
    infos.graphics.render_pass = env.main_rp[MAIN_RENDER_PASS_OIT];
    infos.graphics.subpass = 2;
    let layout = vg!(ctx, wboit_resolve_pipeline.layout);
    *vg_mut!(ctx, wboit_resolve_pipeline) =
        create_graphics(ctx, &mut infos, layout, c"wboit_resolve");

    let mut infos = family;
    infos.blend_attachments[0] = resolve_blend;
    infos.set_fragment(env.pick(
        m.get(Shader::mboit_resolve_frag),
        m.get(Shader::mboit_resolve_msaa_frag),
    ));
    infos.graphics.render_pass = env.main_rp[MAIN_RENDER_PASS_MBOIT];
    infos.graphics.subpass = 3;
    let layout = vg!(ctx, mboit_resolve_pipeline.layout);
    *vg_mut!(ctx, mboit_resolve_pipeline) =
        create_graphics(ctx, &mut infos, layout, c"mboit_resolve");
}

/// `R_CreateScreenEffectsPipelines`.
fn create_screen_effects<E: Engine>(ctx: &mut Ctx<'_, E>, env: &Env, m: &ShaderModules) {
    let no_flags = vk::PipelineShaderStageCreateFlags::empty();
    let module = if env.ten_bit {
        Shader::screen_effects_10bit_comp
    } else {
        Shader::screen_effects_8bit_comp
    };
    let layout = vg!(ctx, screen_effects_pipeline.layout);
    *vg_mut!(ctx, screen_effects_pipeline) = create_compute(
        ctx,
        layout,
        m.get(module),
        no_flags,
        None,
        c"screen_effects",
    );

    let module = if env.ten_bit {
        Shader::screen_effects_10bit_scale_comp
    } else {
        Shader::screen_effects_8bit_scale_comp
    };
    let layout = vg!(ctx, screen_effects_scale_pipeline.layout);
    *vg_mut!(ctx, screen_effects_scale_pipeline) = create_compute(
        ctx,
        layout,
        m.get(module),
        no_flags,
        None,
        c"screen_effects_scale",
    );

    if env.screen_effects_sops {
        let module = if env.ten_bit {
            Shader::screen_effects_10bit_scale_sops_comp
        } else {
            Shader::screen_effects_8bit_scale_sops_comp
        };
        let flags = vk::PipelineShaderStageCreateFlags::ALLOW_VARYING_SUBGROUP_SIZE
            | vk::PipelineShaderStageCreateFlags::REQUIRE_FULL_SUBGROUPS;
        let layout = vg!(ctx, screen_effects_scale_sops_pipeline.layout);
        *vg_mut!(ctx, screen_effects_scale_sops_pipeline) = create_compute(
            ctx,
            layout,
            m.get(module),
            flags,
            None,
            c"screen_effects_scale_sops",
        );
    }
}

static LIGHTMAP_SPEC_ENTRIES: [vk::SpecializationMapEntry; 1] = [vk::SpecializationMapEntry {
    constant_id: 0,
    offset: 0,
    size: 4,
}];

/// `R_CreateUpdateLightmapPipelines`.
fn create_update_lightmap<E: Engine>(ctx: &mut Ctx<'_, E>, env: &Env, m: &ShaderModules) {
    let no_flags = vk::PipelineShaderStageCreateFlags::empty();
    let data = [env.ten_bit as u32];
    let spec = spec_info(&LIGHTMAP_SPEC_ENTRIES, &data);

    let module = if env.ten_bit {
        Shader::update_lightmap_10bit_comp
    } else {
        Shader::update_lightmap_8bit_comp
    };
    let layout = vg!(ctx, update_lightmap_pipeline.layout);
    *vg_mut!(ctx, update_lightmap_pipeline) = create_compute(
        ctx,
        layout,
        m.get(module),
        no_flags,
        Some(&spec),
        c"update_lightmap",
    );

    if env.ray_query {
        let module = if env.ten_bit {
            Shader::update_lightmap_10bit_rt_comp
        } else {
            Shader::update_lightmap_8bit_rt_comp
        };
        let layout = vg!(ctx, update_lightmap_rt_pipeline.layout);
        *vg_mut!(ctx, update_lightmap_rt_pipeline) = create_compute(
            ctx,
            layout,
            m.get(module),
            no_flags,
            Some(&spec),
            c"update_lightmap_rt",
        );
    }
}

/// `R_CreateIndirectComputePipelines`.
fn create_indirect_compute<E: Engine>(ctx: &mut Ctx<'_, E>, m: &ShaderModules) {
    let no_flags = vk::PipelineShaderStageCreateFlags::empty();
    let layout = vg!(ctx, indirect_draw_pipeline.layout);
    *vg_mut!(ctx, indirect_draw_pipeline) = create_compute(
        ctx,
        layout,
        m.get(Shader::indirect_comp),
        no_flags,
        None,
        c"indirect_draw",
    );
    let layout = vg!(ctx, indirect_clear_pipeline.layout);
    *vg_mut!(ctx, indirect_clear_pipeline) = create_compute(
        ctx,
        layout,
        m.get(Shader::indirect_clear_comp),
        no_flags,
        None,
        c"indirect_clear",
    );
}

/// `R_CreateRayDebugPipelines` (`_DEBUG` only).
fn create_ray_debug<E: Engine>(ctx: &mut Ctx<'_, E>, env: &Env, m: &ShaderModules) {
    if !cfg!(feature = "engine-debug") || !env.ray_query {
        return;
    }
    let layout = vg!(ctx, ray_debug_pipeline.layout);
    *vg_mut!(ctx, ray_debug_pipeline) = create_compute(
        ctx,
        layout,
        m.get(Shader::ray_debug_comp),
        vk::PipelineShaderStageCreateFlags::empty(),
        None,
        c"ray_debug_pipeline",
    );
}

/// `R_CreateAnimComputePipelines`.
fn create_anim_compute<E: Engine>(ctx: &mut Ctx<'_, E>, env: &Env, m: &ShaderModules) {
    if !env.ray_query {
        return;
    }
    let no_flags = vk::PipelineShaderStageCreateFlags::empty();
    let layout = vg!(ctx, mesh_interpolate_pipeline.layout);
    *vg_mut!(ctx, mesh_interpolate_pipeline) = create_compute(
        ctx,
        layout,
        m.get(Shader::mesh_interpolate_comp),
        no_flags,
        None,
        c"mesh_interpolate_pipeline",
    );
    let layout = vg!(ctx, skinning_pipeline.layout);
    *vg_mut!(ctx, skinning_pipeline) = create_compute(
        ctx,
        layout,
        m.get(Shader::skinning_comp),
        no_flags,
        None,
        c"skinning_pipeline",
    );
    let layout = vg!(ctx, skinning_8_pipeline.layout);
    *vg_mut!(ctx, skinning_8_pipeline) = create_compute(
        ctx,
        layout,
        m.get(Shader::skinning_8_comp),
        no_flags,
        None,
        c"skinning_8_pipeline",
    );
}

/// `R_CreatePipelines`.
pub fn create_pipelines<E: Engine>(ctx: &mut Ctx<'_, E>, modules: &mut ShaderModules) {
    ctx.engine.sys_printf("Creating pipelines\n");
    modules.create_all(ctx);
    let m = &*modules;
    let (env, base) = {
        // SAFETY: a shared view scoped to these two constructors, which make
        // no C callback; `R_CreatePipelines` runs on the main thread from
        // `GL_Init`/`vid_restart`, with no worker allocation in flight (a
        // `vid_restart` starts with `GL_WaitForDeviceIdle`).
        let vg = unsafe { &*ctx.vg.as_ptr() };
        (Env::new(vg), PipelineInfos::defaults(vg, m))
    };

    create_basic(ctx, &env, m, &base);
    create_warp(ctx, &env, m, &base);
    create_particles(ctx, &env, m, &base);
    create_fte_particles(ctx, &env, m, &base);
    create_sprites(ctx, &env, m, &base);
    create_sky(ctx, &env, m, &base);
    create_showtris(ctx, &env, m, &base);
    create_world(ctx, &env, m, &base);
    create_alias(ctx, &env, m, &base);
    create_md5(ctx, &env, m, &base);
    create_postprocess(ctx, &env, m, &base);
    create_screen_effects(ctx, &env, m);
    create_update_lightmap(ctx, &env, m);
    create_indirect_compute(ctx, m);
    create_ray_debug(ctx, &env, m);
    create_anim_compute(ctx, &env, m);

    modules.destroy_all(ctx);
}

// ---------------------------------------------------------------------------
// R_DestroyPipelines
// ---------------------------------------------------------------------------

/// `vkDestroyPipeline` + `handle = VK_NULL_HANDLE`. C also calls it on
/// handles that were never created (a no-op for `VK_NULL_HANDLE`); skipping
/// those is the same thing.
fn destroy_with(device: &ash::Device, pipeline: &mut VulkanPipeline) {
    if pipeline.handle != vk::Pipeline::null() {
        // SAFETY: `GL_WaitForDeviceIdle` preceded `R_DestroyPipelines`, so no
        // submission references the pipeline; the device is live.
        unsafe { device.destroy_pipeline(pipeline.handle, None) };
    }
    pipeline.handle = vk::Pipeline::null();
}

/// `R_DestroyPipelines`.
pub fn destroy_pipelines<E: Engine>(ctx: &mut Ctx<'_, E>) {
    if ctx.engine.renderhash() {
        ctx.engine.pipelines_destroyed();
    }
    let device = ctx.device;
    // SAFETY: scoped to this function, which makes no C callback past this
    // point (`destroy_with` only calls the device); `R_DestroyPipelines`
    // runs on the main thread after `GL_WaitForDeviceIdle`, with no task
    // worker alive to touch the struct.
    let vg = unsafe { &mut *ctx.vg.as_ptr() };
    let d = |p: &mut VulkanPipeline| destroy_with(device, p);

    for i in 0..RENDER_PASS_INDEX_COUNT {
        d(&mut vg.basic_alphatest_pipeline[i]);
        d(&mut vg.basic_blend_pipeline[i]);
        d(&mut vg.basic_notex_blend_pipeline[i]);
    }
    for i in 0..WORLD_PIPELINE_COUNT {
        for variant in 0..MAIN_RENDER_PASS_VARIANT_COUNT {
            d(&mut vg.world_pipelines[variant][i]);
        }
        d(&mut vg.world_wboit_pipelines[i]);
        d(&mut vg.world_mboit_moment_pipelines[i]);
        d(&mut vg.world_mboit_composite_pipelines[i]);
    }
    d(&mut vg.raster_tex_warp_pipeline);
    d(&mut vg.particle_pipeline);
    d(&mut vg.particle_oit_pipeline);
    for variant in 0..MAIN_RENDER_PASS_VARIANT_COUNT {
        d(&mut vg.particle_post_oit_pipeline[variant]);
    }
    d(&mut vg.particle_mboit_moment_pipeline);
    d(&mut vg.particle_mboit_composite_pipeline);
    let fte_modes = if vg.non_solid_fill {
        FTE_PARTICLE_PIPELINE_COUNT
    } else {
        FTE_PARTICLE_PIPELINE_COUNT / 2
    };
    for i in 0..fte_modes {
        for variant in 0..MAIN_RENDER_PASS_VARIANT_COUNT {
            d(&mut vg.fte_particle_pipelines[variant][i]);
        }
        d(&mut vg.fte_particle_wboit_pipelines[i]);
        for variant in 0..MAIN_RENDER_PASS_VARIANT_COUNT {
            d(&mut vg.fte_particle_post_oit_pipelines[variant][i]);
        }
    }
    for variant in 0..MAIN_RENDER_PASS_VARIANT_COUNT {
        d(&mut vg.sprite_pipeline[variant]);
    }
    d(&mut vg.sprite_oit_pipeline);
    d(&mut vg.sprite_mboit_moment_pipeline);
    d(&mut vg.sprite_mboit_composite_pipeline);
    for variant in 0..MAIN_RENDER_PASS_VARIANT_COUNT {
        for i in 0..2 {
            d(&mut vg.sky_stencil_pipeline[variant][i]);
            d(&mut vg.sky_color_pipeline[variant][i]);
            d(&mut vg.sky_cube_pipeline[variant][i]);
            d(&mut vg.sky_layer_pipeline[variant][i]);
        }
        d(&mut vg.sky_box_pipeline[variant]);
    }
    for i in 0..MODEL_PIPELINE_COUNT {
        for variant in 0..MAIN_RENDER_PASS_VARIANT_COUNT {
            d(&mut vg.alias_pipelines[variant][i]);
            d(&mut vg.md5_pipelines[variant][i]);
            d(&mut vg.md5_8_pipelines[variant][i]);
        }
        d(&mut vg.alias_wboit_pipelines[i]);
        d(&mut vg.alias_mboit_moment_pipelines[i]);
        d(&mut vg.alias_mboit_composite_pipelines[i]);
        d(&mut vg.md5_wboit_pipelines[i]);
        d(&mut vg.md5_mboit_moment_pipelines[i]);
        d(&mut vg.md5_mboit_composite_pipelines[i]);
        d(&mut vg.md5_8_wboit_pipelines[i]);
        d(&mut vg.md5_8_mboit_moment_pipelines[i]);
        d(&mut vg.md5_8_mboit_composite_pipelines[i]);
    }
    d(&mut vg.postprocess_pipeline);
    d(&mut vg.wboit_resolve_pipeline);
    d(&mut vg.mboit_resolve_pipeline);
    d(&mut vg.screen_effects_pipeline);
    d(&mut vg.screen_effects_scale_pipeline);
    d(&mut vg.screen_effects_scale_sops_pipeline);
    d(&mut vg.cs_tex_warp_pipeline);
    if vg.showtris_pipeline[MAIN_RENDER_PASS_STANDARD].handle != vk::Pipeline::null() {
        for variant in 0..MAIN_RENDER_PASS_VARIANT_COUNT {
            d(&mut vg.showtris_pipeline[variant]);
            d(&mut vg.showtris_indirect_pipeline[variant]);
            d(&mut vg.showtris_depth_test_pipeline[variant]);
            d(&mut vg.showtris_indirect_depth_test_pipeline[variant]);
            d(&mut vg.showbboxes_pipeline[variant]);
        }
    }
    d(&mut vg.update_lightmap_pipeline);
    d(&mut vg.update_lightmap_rt_pipeline);
    d(&mut vg.ray_debug_pipeline);
    d(&mut vg.mesh_interpolate_pipeline);
    d(&mut vg.skinning_pipeline);
    d(&mut vg.skinning_8_pipeline);
    d(&mut vg.indirect_draw_pipeline);
    d(&mut vg.indirect_clear_pipeline);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_tables_match_c_layouts() {
        assert_eq!(BASIC_INPUT.bindings[0].stride, 24);
        assert_eq!(WORLD_INPUT.bindings[0].stride, 28);
        assert_eq!(
            ALIAS_INPUT
                .bindings
                .iter()
                .map(|b| b.stride)
                .collect::<Vec<_>>(),
            [8, 12, 12]
        );
        assert_eq!(MD5_INPUT.bindings[0].stride, 88);
        assert_eq!(MD5_8_INPUT.bindings[0].stride, 144);
        assert_eq!(
            MD5_INPUT.attrs.iter().map(|a| a.offset).collect::<Vec<_>>(),
            [0, 12, 24, 32, 36, 40, 56, 72]
        );
        assert_eq!(
            MD5_8_INPUT
                .attrs
                .iter()
                .map(|a| a.offset)
                .collect::<Vec<_>>(),
            [0, 12, 24, 32, 36, 40, 44, 48, 64, 80, 96, 112, 128]
        );
        for (input, n) in [
            (&BASIC_INPUT, 3),
            (&WORLD_INPUT, 3),
            (&ALIAS_INPUT, 5),
            (&MD5_INPUT, 8),
            (&MD5_8_INPUT, 13),
            (&SHOWTRIS_INPUT, 1),
        ] {
            assert_eq!(input.attrs.len(), n);
            for (location, attr) in input.attrs.iter().enumerate() {
                assert_eq!(attr.location as usize, location);
            }
        }
        assert!(EMPTY_INPUT.attrs.is_empty() && EMPTY_INPUT.bindings.is_empty());
    }

    #[test]
    fn fte_blend_keeps_the_dst_alpha_quirk() {
        let premultiplied = fte_particle_blend(7);
        assert_eq!(premultiplied.src_color_blend_factor, vk::BlendFactor::ONE);
        assert_eq!(
            premultiplied.dst_color_blend_factor,
            vk::BlendFactor::ONE_MINUS_SRC_ALPHA
        );
        assert_eq!(premultiplied.dst_alpha_blend_factor, vk::BlendFactor::ONE);
        let subtract = fte_particle_blend(4);
        assert_eq!(
            subtract.dst_color_blend_factor,
            vk::BlendFactor::ONE_MINUS_SRC_COLOR
        );
        assert_eq!(subtract.dst_alpha_blend_factor, vk::BlendFactor::SRC_ALPHA);
        assert_eq!(subtract.color_write_mask, vk::ColorComponentFlags::RGBA);
    }

    #[test]
    fn wboit_blend_matches_c() {
        let mut states = [BlendState::default(); 3];
        set_wboit_blend(&mut states);
        assert_eq!(states[0].color_write_mask, vk::ColorComponentFlags::RGBA);
        assert_eq!(states[1].color_write_mask, vk::ColorComponentFlags::R);
        assert_eq!(
            states[1].dst_color_blend_factor,
            vk::BlendFactor::ONE_MINUS_SRC_COLOR
        );
        assert_eq!(
            states[1].dst_alpha_blend_factor,
            vk::BlendFactor::ONE_MINUS_SRC_ALPHA
        );
        assert_eq!(states[2].blend_enable, vk::FALSE);
        assert_eq!(states[2].color_write_mask, vk::ColorComponentFlags::empty());
        set_mboit_moment_blend(&mut states);
        assert_eq!(states[0].color_write_mask, vk::ColorComponentFlags::R);
        assert_eq!(states[1].color_write_mask, vk::ColorComponentFlags::RGBA);
    }
}
