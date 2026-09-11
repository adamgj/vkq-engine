//! Renderer ABI mirrors (`Quake/glquake.h`, `Quake/gl_heap.h`) -- Rust
//! migration Phase 8 M3 (ADR-011, ADR-015). Under `-Duse_rust_render` the
//! Rust `gl_heap` fills `vulkan_memory_t` through the C
//! `R_AllocateVulkanMemory` and hands `glheapstats_t` back to C readers
//! (`gl_mesh.c`, `gl_texmgr.c`), so layout drift is silent memory
//! corruption. Verified by `quake-ctest/tests/render_abi.rs` (`glheapstats_t`
//! against the engine's `gl_heap.h`; the `glquake.h`/Vulkan types against
//! the prelude's hand copies) and, against the real `glquake.h`, by the
//! `COMPILE_TIME_ASSERT`s in `Quake/gl_heap_glue.c`, which every
//! `-Duse_rust_render` build compiles. The const asserts below pin the
//! same numbers on the Rust side.
//!
//! `VkDeviceMemory` is a pointer typedef where `vulkan_core.h`'s
//! `VK_USE_64_BIT_PTR_DEFINES` is 1 and a `uint64_t` elsewhere;
//! `ash::vk::DeviceMemory` is a `repr(transparent)` `u64` on both (task plan
//! D2). Only 64-bit targets have been checked; a 32-bit leg would take the
//! D2 fallback (a `u64` newtype) if its probe disagreed.

use core::ffi::c_int;

/// `vulkan_memory_type_t` (`glquake.h`): a C `enum`, so `c_int`-sized.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum VulkanMemoryType {
    #[default]
    None = 0,
    Device = 1,
    Host = 2,
}

/// `vulkan_memory_t` (`glquake.h`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct VulkanMemory {
    pub handle: ash::vk::DeviceMemory,
    pub size: usize,
    pub type_: VulkanMemoryType,
}

/// `glheapstats_t` (`gl_heap.h`).
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct GlHeapStats {
    pub num_segments: u32,
    pub num_allocations: u32,
    pub num_small_allocations: u32,
    pub num_block_allocations: u32,
    pub num_dedicated_allocations: u32,
    pub num_blocks_used: u32,
    pub num_blocks_free: u32,
    pub num_pages_allocated: u32,
    pub num_pages_free: u32,
    pub num_bytes_allocated: u64,
    pub num_bytes_free: u64,
    pub num_bytes_wasted: u64,
}

const _: () = {
    assert!(core::mem::size_of::<VulkanMemoryType>() == core::mem::size_of::<c_int>());
    assert!(core::mem::size_of::<ash::vk::DeviceMemory>() == 8);
    assert!(core::mem::size_of::<VulkanMemory>() == 8 + core::mem::size_of::<usize>() * 2);
    assert!(core::mem::offset_of!(VulkanMemory, handle) == 0);
    assert!(core::mem::offset_of!(VulkanMemory, size) == 8);
    assert!(core::mem::offset_of!(VulkanMemory, type_) == 8 + core::mem::size_of::<usize>());
    assert!(core::mem::size_of::<GlHeapStats>() == 64);
    assert!(core::mem::offset_of!(GlHeapStats, num_bytes_allocated) == 40);
};

// ---- Phase 8 M4: gl_texmgr.h ---------------------------------------------

/// `TEXPREF_*` (`gl_texmgr.h:33-50`, `textureflags_t`).
pub const TEXPREF_NONE: u32 = 0x0000;
pub const TEXPREF_MIPMAP: u32 = 0x0001;
pub const TEXPREF_LINEAR: u32 = 0x0002;
pub const TEXPREF_NEAREST: u32 = 0x0004;
pub const TEXPREF_ALPHA: u32 = 0x0008;
pub const TEXPREF_PAD: u32 = 0x0010;
pub const TEXPREF_PERSIST: u32 = 0x0020;
pub const TEXPREF_OVERWRITE: u32 = 0x0040;
pub const TEXPREF_NOPICMIP: u32 = 0x0080;
pub const TEXPREF_FULLBRIGHT: u32 = 0x0100;
pub const TEXPREF_NOBRIGHT: u32 = 0x0200;
pub const TEXPREF_CONCHARS: u32 = 0x0400;
pub const TEXPREF_WARPIMAGE: u32 = 0x0800;
pub const TEXPREF_PREMULTIPLY: u32 = 0x1000;
pub const TEXPREF_ALPHAPIXELS: u32 = 0x2000;

/// `enum srcformat` (`gl_texmgr.h:52-60`): a C `enum`, so `c_int`-sized. The
/// `gltexture_t` mirror stores it as the raw `c_int` (C can hand a value the
/// enum does not name; `SrcFormat::from_raw` maps those to `None`).
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SrcFormat {
    #[default]
    Indexed = 0,
    Lightmap = 1,
    Rgba = 2,
    SurfIndices = 3,
    RgbaCubemap = 4,
    IndexedPalette = 5,
}

impl SrcFormat {
    pub const fn from_raw(raw: c_int) -> Option<Self> {
        match raw {
            0 => Some(Self::Indexed),
            1 => Some(Self::Lightmap),
            2 => Some(Self::Rgba),
            3 => Some(Self::SurfIndices),
            4 => Some(Self::RgbaCubemap),
            5 => Some(Self::IndexedPalette),
            _ => None,
        }
    }
}

/// `gltexture_t` (`gl_texmgr.h:66-93`). Under `-Duse_rust_render` the Rust
/// texture manager owns the array and C readers (`gl_draw.c`, `r_brush.c`,
/// `gl_warp.c`, `gl_sky.c`, `gl_model.c`, ...) dereference the pointers it
/// hands out, so every offset matters. `owner` is a `qmodel_t *` and
/// `allocation` a `glheapallocation_t *` (the M3 boxed `Allocation`), both
/// opaque here; `flags` is the `textureflags_t` enum (`c_int`-sized, used as
/// `unsigned`); `source_offset` is `src_offset_t` (`uintptr_t`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GlTexture {
    pub next: *mut GlTexture,
    pub owner: *mut core::ffi::c_void,
    pub name: [u8; 64],
    pub path_id: u32,
    pub width: u32,
    pub height: u32,
    pub flags: u32,
    pub source_file: [u8; 64],
    pub source_offset: usize,
    pub source_format: c_int,
    pub source_width: u32,
    pub source_height: u32,
    pub source_crc: u16,
    pub shirt: i8,
    pub pants: i8,
    pub image: ash::vk::Image,
    pub image_view: ash::vk::ImageView,
    pub target_image_view: ash::vk::ImageView,
    pub allocation: *mut core::ffi::c_void,
    pub descriptor_set: ash::vk::DescriptorSet,
    pub frame_buffer: ash::vk::Framebuffer,
    pub storage_descriptor_set: ash::vk::DescriptorSet,
}

impl GlTexture {
    /// The all-zero slot `TexMgr_Init`'s `Mem_Alloc` hands out.
    pub const ZEROED: GlTexture = GlTexture {
        next: core::ptr::null_mut(),
        owner: core::ptr::null_mut(),
        name: [0; 64],
        path_id: 0,
        width: 0,
        height: 0,
        flags: 0,
        source_file: [0; 64],
        source_offset: 0,
        source_format: 0,
        source_width: 0,
        source_height: 0,
        source_crc: 0,
        shirt: 0,
        pants: 0,
        image: ash::vk::Image::null(),
        image_view: ash::vk::ImageView::null(),
        target_image_view: ash::vk::ImageView::null(),
        allocation: core::ptr::null_mut(),
        descriptor_set: ash::vk::DescriptorSet::null(),
        frame_buffer: ash::vk::Framebuffer::null(),
        storage_descriptor_set: ash::vk::DescriptorSet::null(),
    };
}

impl Default for GlTexture {
    fn default() -> Self {
        Self::ZEROED
    }
}

const _: () = {
    use core::mem::{offset_of, size_of};
    assert!(size_of::<SrcFormat>() == size_of::<c_int>());
    // 64-bit layout (gl_texmgr.h:66-93). A 32-bit target shrinks the
    // pointer-typed members and realigns the 64-bit handles; that layout is
    // unverified (the ctest probe measures the host, and every CI leg is
    // 64-bit), so `-Duse_rust_render` is 64-bit only until a 32-bit leg
    // measures it. The glue's COMPILE_TIME_ASSERTs are guarded the same way.
    assert!(size_of::<usize>() != 8 || size_of::<GlTexture>() == 240);
    assert!(size_of::<usize>() != 8 || offset_of!(GlTexture, name) == 16);
    assert!(size_of::<usize>() != 8 || offset_of!(GlTexture, path_id) == 80);
    assert!(size_of::<usize>() != 8 || offset_of!(GlTexture, flags) == 92);
    assert!(size_of::<usize>() != 8 || offset_of!(GlTexture, source_file) == 96);
    assert!(size_of::<usize>() != 8 || offset_of!(GlTexture, source_offset) == 160);
    assert!(size_of::<usize>() != 8 || offset_of!(GlTexture, source_crc) == 180);
    assert!(size_of::<usize>() != 8 || offset_of!(GlTexture, pants) == 183);
    assert!(size_of::<usize>() != 8 || offset_of!(GlTexture, image) == 184);
    assert!(size_of::<usize>() != 8 || offset_of!(GlTexture, storage_descriptor_set) == 232);
};

// ---- Phase 8 M5: gl_rmisc.c / glquake.h -----------------------------------
//
// Under `-Duse_rust_render` the Rust `gl_rmisc` owns `vulkan_globals` and the
// C renderer files that remain (`gl_vidsdl.c`, `gl_rmain.c`, `r_world.c`,
// ...) read and write it through the exported C-layout view (ADR-007), so
// every offset below is load-bearing. The numbers are the clang-cl x64 probe
// of the real `glquake.h` (task plan M5), re-checked by `render_abi.rs`
// against `abi_probe.c` and by the `COMPILE_TIME_ASSERT`s in
// `Quake/gl_rmisc_glue.c`. The Vulkan-defined members use `ash::vk` types:
// the handles are `repr(transparent)` `u64`/pointer newtypes, the enums
// `repr(transparent)` `i32`, the structs `repr(C)` mirrors of `vulkan_core.h`.
//
// The `_DEBUG` tail of `vulkanglobals_t` (two debug-utils label PFNs) is the
// `engine-debug` cargo feature, which the meson build enables together with
// `-D_DEBUG` (`meson.build`, the `engine_debug` feature list).

use ash::vk;

/// `PCBX_*` (`glquake.h:204-211`): the primary command-buffer contexts.
pub const PCBX_BUILD_ACCELERATION_STRUCTURES: usize = 0;
pub const PCBX_UPDATE_LIGHTMAPS: usize = 1;
pub const PCBX_UPDATE_WARP: usize = 2;
pub const PCBX_RENDER_PASSES: usize = 3;
pub const PCBX_NUM: usize = 4;
/// `SCBX_NUM` (`glquake.h:233`)
pub const SCBX_NUM: usize = 16;
/// `RENDER_PASS_INDEX_COUNT` (`glquake.h:250`)
pub const RENDER_PASS_INDEX_COUNT: usize = 7;
/// `MAIN_RENDER_PASS_VARIANT_COUNT` (`glquake.h:258`)
pub const MAIN_RENDER_PASS_VARIANT_COUNT: usize = 3;
/// `MAIN_RENDER_PASS_STENCIL_COUNT` (`glquake.h:265`)
pub const MAIN_RENDER_PASS_STENCIL_COUNT: usize = 2;
pub const WORLD_PIPELINE_COUNT: usize = 16;
pub const MODEL_PIPELINE_COUNT: usize = 6;
pub const FTE_PARTICLE_PIPELINE_COUNT: usize = 16;
pub const MAX_BATCH_SIZE: usize = 65536;
pub const NUM_COLOR_BUFFERS: usize = 2;
pub const INITIAL_STAGING_BUFFER_SIZE_KB: usize = 16384;
pub const FAN_INDEX_BUFFER_SIZE: usize = 126;
pub const MIN_NB_DESCRIPTORS_PER_TYPE: u32 = 32;
pub const MAX_SANITY_LIGHTMAPS: u32 = 256;
pub const MAX_GLTEXTURES: u32 = 16 * 4096;
/// `MAXLIGHTMAPS` (`bspfile.h:242`)
pub const MAXLIGHTMAPS: u32 = 4;
/// `MAX_UNIFORM_ALLOC` (`gl_rmisc.c`)
pub const MAX_UNIFORM_ALLOC: usize = 2048;

/// `oit_mode_t` (`glquake.h`): a C `enum`, stored as the raw `c_int`.
pub const OIT_MODE_NONE: c_int = 0;
pub const OIT_MODE_WBOIT: c_int = 1;
pub const OIT_MODE_MBOIT: c_int = 2;

/// `dynbuffer_t` (`glquake.h:67-73`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DynBuffer {
    pub buffer: vk::Buffer,
    pub current_offset: u32,
    pub data: *mut u8,
    pub device_address: vk::DeviceAddress,
}

impl DynBuffer {
    pub const ZEROED: DynBuffer = DynBuffer {
        buffer: vk::Buffer::null(),
        current_offset: 0,
        data: core::ptr::null_mut(),
        device_address: 0,
    };
}

impl Default for DynBuffer {
    fn default() -> Self {
        Self::ZEROED
    }
}

/// `vulkan_pipeline_layout_t` (`glquake.h`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VulkanPipelineLayout {
    pub handle: vk::PipelineLayout,
    pub push_constant_range: vk::PushConstantRange,
    pub mboit_input_attachment_set: c_int,
}

impl VulkanPipelineLayout {
    pub const ZEROED: VulkanPipelineLayout = VulkanPipelineLayout {
        handle: vk::PipelineLayout::null(),
        push_constant_range: vk::PushConstantRange {
            stage_flags: vk::ShaderStageFlags::empty(),
            offset: 0,
            size: 0,
        },
        mboit_input_attachment_set: 0,
    };
}

impl Default for VulkanPipelineLayout {
    fn default() -> Self {
        Self::ZEROED
    }
}

/// `vulkan_pipeline_t` (`glquake.h`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VulkanPipeline {
    pub handle: vk::Pipeline,
    pub layout: VulkanPipelineLayout,
}

impl VulkanPipeline {
    pub const ZEROED: VulkanPipeline = VulkanPipeline {
        handle: vk::Pipeline::null(),
        layout: VulkanPipelineLayout::ZEROED,
    };
}

impl Default for VulkanPipeline {
    fn default() -> Self {
        Self::ZEROED
    }
}

/// `vulkan_desc_set_layout_t` (`glquake.h`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct VulkanDescSetLayout {
    pub handle: vk::DescriptorSetLayout,
    pub num_combined_image_samplers: c_int,
    pub num_ubos: c_int,
    pub num_ubos_dynamic: c_int,
    pub num_storage_buffers: c_int,
    pub num_input_attachments: c_int,
    pub num_storage_images: c_int,
    pub num_sampled_images: c_int,
    pub num_acceleration_structures: c_int,
}

/// `buffer_create_info_t` (`glquake.h:893-902`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct BufferCreateInfo {
    pub buffer: *mut vk::Buffer,
    pub size: usize,
    pub alignment: usize,
    pub usage: vk::BufferUsageFlags,
    pub mapped: *mut *mut core::ffi::c_void,
    pub address: *mut vk::DeviceAddress,
    pub name: *const core::ffi::c_char,
}

/// `cb_context_t` (`glquake.h:339-349`). `current_canvas` is the
/// `canvastype` enum (`quakedef.h`), stored raw.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CbContext {
    pub cb: vk::CommandBuffer,
    pub current_canvas: c_int,
    pub render_pass: vk::RenderPass,
    pub render_pass_index: c_int,
    pub subpass: c_int,
    pub current_pipeline: VulkanPipeline,
    pub vbo_indices: [u32; MAX_BATCH_SIZE],
    pub num_vbo_indices: u32,
}

/// `vulkanglobals_t` (`glquake.h:351-532`), member for member. The function
/// pointers are `Option<PFN>` so a null slot is `None` (the C checks
/// `vk_get_buffer_device_address != NULL` and the two `_DEBUG` labels).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VulkanGlobals {
    pub device: vk::Device,
    pub device_idle: bool,
    pub validation: bool,
    pub debug_utils: bool,
    pub queue: vk::Queue,
    pub primary_cb_contexts: [CbContext; PCBX_NUM],
    pub secondary_cb_contexts: [*mut CbContext; SCBX_NUM],
    pub color_clear_value: vk::ClearValue,
    pub swap_chain_format: vk::Format,
    pub want_full_screen_exclusive: bool,
    pub swap_chain_full_screen_exclusive: bool,
    pub swap_chain_full_screen_acquired: bool,
    pub device_properties: vk::PhysicalDeviceProperties,
    pub device_features: vk::PhysicalDeviceFeatures,
    pub memory_properties: vk::PhysicalDeviceMemoryProperties,
    pub gfx_queue_family_index: u32,
    pub color_format: vk::Format,
    pub depth_format: vk::Format,
    pub sample_count: vk::SampleCountFlags,
    pub supersampling: bool,
    pub non_solid_fill: bool,
    pub multi_draw_indirect: bool,
    pub screen_effects_sops: bool,
    pub get_surface_capabilities_2: bool,
    pub get_physical_device_properties_2: bool,
    pub vulkan_1_1_available: bool,
    pub dedicated_allocation: bool,
    pub full_screen_exclusive: bool,
    pub ray_query: bool,
    pub present_wait: bool,
    pub color_buffers: [vk::Image; NUM_COLOR_BUFFERS],
    pub oit_accum_buffer: vk::Image,
    pub oit_reveal_buffer: vk::Image,
    pub mboit_b0_buffer: vk::Image,
    pub mboit_moments0_buffer: vk::Image,
    pub mboit_color_buffer: vk::Image,
    pub fan_index_buffer: vk::Buffer,
    pub staging_buffer_size: c_int,
    pub main_render_pass:
        [[vk::RenderPass; MAIN_RENDER_PASS_STENCIL_COUNT]; MAIN_RENDER_PASS_VARIANT_COUNT],
    pub warp_render_pass: vk::RenderPass,
    pub basic_alphatest_pipeline: [VulkanPipeline; RENDER_PASS_INDEX_COUNT],
    pub basic_blend_pipeline: [VulkanPipeline; RENDER_PASS_INDEX_COUNT],
    pub basic_notex_blend_pipeline: [VulkanPipeline; RENDER_PASS_INDEX_COUNT],
    pub basic_pipeline_layout: VulkanPipelineLayout,
    pub world_pipelines: [[VulkanPipeline; WORLD_PIPELINE_COUNT]; MAIN_RENDER_PASS_VARIANT_COUNT],
    pub world_wboit_pipelines: [VulkanPipeline; WORLD_PIPELINE_COUNT],
    pub world_mboit_moment_pipelines: [VulkanPipeline; WORLD_PIPELINE_COUNT],
    pub world_mboit_composite_pipelines: [VulkanPipeline; WORLD_PIPELINE_COUNT],
    pub world_pipeline_layout: VulkanPipelineLayout,
    pub raster_tex_warp_pipeline: VulkanPipeline,
    pub particle_pipeline: VulkanPipeline,
    pub particle_oit_pipeline: VulkanPipeline,
    pub particle_post_oit_pipeline: [VulkanPipeline; MAIN_RENDER_PASS_VARIANT_COUNT],
    pub particle_mboit_moment_pipeline: VulkanPipeline,
    pub particle_mboit_composite_pipeline: VulkanPipeline,
    pub sprite_pipeline: [VulkanPipeline; MAIN_RENDER_PASS_VARIANT_COUNT],
    pub sprite_oit_pipeline: VulkanPipeline,
    pub sprite_mboit_moment_pipeline: VulkanPipeline,
    pub sprite_mboit_composite_pipeline: VulkanPipeline,
    pub sky_pipeline_layout: [VulkanPipelineLayout; 2],
    pub sky_stencil_pipeline: [[VulkanPipeline; 2]; MAIN_RENDER_PASS_VARIANT_COUNT],
    pub sky_color_pipeline: [[VulkanPipeline; 2]; MAIN_RENDER_PASS_VARIANT_COUNT],
    pub sky_box_pipeline: [VulkanPipeline; MAIN_RENDER_PASS_VARIANT_COUNT],
    pub sky_cube_pipeline: [[VulkanPipeline; 2]; MAIN_RENDER_PASS_VARIANT_COUNT],
    pub sky_layer_pipeline: [[VulkanPipeline; 2]; MAIN_RENDER_PASS_VARIANT_COUNT],
    pub alias_pipelines: [[VulkanPipeline; MODEL_PIPELINE_COUNT]; MAIN_RENDER_PASS_VARIANT_COUNT],
    pub alias_wboit_pipelines: [VulkanPipeline; MODEL_PIPELINE_COUNT],
    pub alias_mboit_moment_pipelines: [VulkanPipeline; MODEL_PIPELINE_COUNT],
    pub alias_mboit_composite_pipelines: [VulkanPipeline; MODEL_PIPELINE_COUNT],
    pub md5_pipelines: [[VulkanPipeline; MODEL_PIPELINE_COUNT]; MAIN_RENDER_PASS_VARIANT_COUNT],
    pub md5_wboit_pipelines: [VulkanPipeline; MODEL_PIPELINE_COUNT],
    pub md5_mboit_moment_pipelines: [VulkanPipeline; MODEL_PIPELINE_COUNT],
    pub md5_mboit_composite_pipelines: [VulkanPipeline; MODEL_PIPELINE_COUNT],
    pub md5_8_pipelines: [[VulkanPipeline; MODEL_PIPELINE_COUNT]; MAIN_RENDER_PASS_VARIANT_COUNT],
    pub md5_8_wboit_pipelines: [VulkanPipeline; MODEL_PIPELINE_COUNT],
    pub md5_8_mboit_moment_pipelines: [VulkanPipeline; MODEL_PIPELINE_COUNT],
    pub md5_8_mboit_composite_pipelines: [VulkanPipeline; MODEL_PIPELINE_COUNT],
    pub postprocess_pipeline: VulkanPipeline,
    pub wboit_resolve_pipeline: VulkanPipeline,
    pub mboit_resolve_pipeline: VulkanPipeline,
    pub screen_effects_pipeline: VulkanPipeline,
    pub screen_effects_scale_pipeline: VulkanPipeline,
    pub screen_effects_scale_sops_pipeline: VulkanPipeline,
    pub cs_tex_warp_pipeline: VulkanPipeline,
    pub showtris_pipeline: [VulkanPipeline; MAIN_RENDER_PASS_VARIANT_COUNT],
    pub showtris_indirect_pipeline: [VulkanPipeline; MAIN_RENDER_PASS_VARIANT_COUNT],
    pub showtris_depth_test_pipeline: [VulkanPipeline; MAIN_RENDER_PASS_VARIANT_COUNT],
    pub showtris_indirect_depth_test_pipeline: [VulkanPipeline; MAIN_RENDER_PASS_VARIANT_COUNT],
    pub showbboxes_pipeline: [VulkanPipeline; MAIN_RENDER_PASS_VARIANT_COUNT],
    pub update_lightmap_pipeline: VulkanPipeline,
    pub update_lightmap_rt_pipeline: VulkanPipeline,
    pub indirect_draw_pipeline: VulkanPipeline,
    pub indirect_clear_pipeline: VulkanPipeline,
    pub ray_debug_pipeline: VulkanPipeline,
    pub mesh_interpolate_pipeline: VulkanPipeline,
    pub skinning_pipeline: VulkanPipeline,
    pub skinning_8_pipeline: VulkanPipeline,
    pub fte_particle_pipelines:
        [[VulkanPipeline; FTE_PARTICLE_PIPELINE_COUNT]; MAIN_RENDER_PASS_VARIANT_COUNT],
    pub fte_particle_wboit_pipelines: [VulkanPipeline; FTE_PARTICLE_PIPELINE_COUNT],
    pub fte_particle_post_oit_pipelines:
        [[VulkanPipeline; FTE_PARTICLE_PIPELINE_COUNT]; MAIN_RENDER_PASS_VARIANT_COUNT],
    pub descriptor_pool: vk::DescriptorPool,
    pub ubo_set_layout: VulkanDescSetLayout,
    pub single_texture_set_layout: VulkanDescSetLayout,
    pub input_attachment_set_layout: VulkanDescSetLayout,
    pub oit_input_attachment_set_layout: VulkanDescSetLayout,
    pub mboit_input_attachment_set_layout: VulkanDescSetLayout,
    pub mboit_input_attachment_descriptor_set: vk::DescriptorSet,
    pub screen_effects_desc_set: vk::DescriptorSet,
    pub screen_effects_set_layout: VulkanDescSetLayout,
    pub single_texture_cs_write_set_layout: VulkanDescSetLayout,
    pub lightmap_compute_set_layout: VulkanDescSetLayout,
    pub indirect_compute_desc_set: vk::DescriptorSet,
    pub indirect_compute_set_layout: VulkanDescSetLayout,
    pub bmodel_instances_desc_set: vk::DescriptorSet,
    pub bmodel_instances_set_layout: VulkanDescSetLayout,
    pub ray_query_push_set_layout: VulkanDescSetLayout,
    pub ray_debug_desc_set: vk::DescriptorSet,
    pub ray_debug_set_layout: VulkanDescSetLayout,
    pub joints_buffer_set_layout: VulkanDescSetLayout,
    pub point_sampler: vk::Sampler,
    pub linear_sampler: vk::Sampler,
    pub point_aniso_sampler: vk::Sampler,
    pub linear_aniso_sampler: vk::Sampler,
    pub point_sampler_lod_bias: vk::Sampler,
    pub linear_sampler_lod_bias: vk::Sampler,
    pub point_aniso_sampler_lod_bias: vk::Sampler,
    pub linear_aniso_sampler_lod_bias: vk::Sampler,
    pub projection_matrix: [f32; 16],
    pub view_matrix: [f32; 16],
    pub view_projection_matrix: [f32; 16],
    pub vk_cmd_bind_pipeline: Option<vk::PFN_vkCmdBindPipeline>,
    pub vk_cmd_push_constants: Option<vk::PFN_vkCmdPushConstants>,
    pub vk_cmd_bind_descriptor_sets: Option<vk::PFN_vkCmdBindDescriptorSets>,
    pub vk_cmd_bind_index_buffer: Option<vk::PFN_vkCmdBindIndexBuffer>,
    pub vk_cmd_bind_vertex_buffers: Option<vk::PFN_vkCmdBindVertexBuffers>,
    pub vk_cmd_draw: Option<vk::PFN_vkCmdDraw>,
    pub vk_cmd_draw_indexed: Option<vk::PFN_vkCmdDrawIndexed>,
    pub vk_cmd_draw_indexed_indirect: Option<vk::PFN_vkCmdDrawIndexedIndirect>,
    pub vk_cmd_pipeline_barrier: Option<vk::PFN_vkCmdPipelineBarrier>,
    pub vk_cmd_copy_buffer_to_image: Option<vk::PFN_vkCmdCopyBufferToImage>,
    pub vk_cmd_dispatch: Option<vk::PFN_vkCmdDispatch>,
    pub vk_cmd_push_descriptor_set: Option<vk::PFN_vkCmdPushDescriptorSetKHR>,
    pub vk_get_buffer_device_address: Option<vk::PFN_vkGetBufferDeviceAddress>,
    pub vk_get_acceleration_structure_build_sizes:
        Option<vk::PFN_vkGetAccelerationStructureBuildSizesKHR>,
    pub vk_create_acceleration_structure: Option<vk::PFN_vkCreateAccelerationStructureKHR>,
    pub vk_destroy_acceleration_structure: Option<vk::PFN_vkDestroyAccelerationStructureKHR>,
    pub vk_cmd_build_acceleration_structures: Option<vk::PFN_vkCmdBuildAccelerationStructuresKHR>,
    pub vk_get_acceleration_structure_device_address:
        Option<vk::PFN_vkGetAccelerationStructureDeviceAddressKHR>,
    pub physical_device_acceleration_structure_properties:
        vk::PhysicalDeviceAccelerationStructurePropertiesKHR<'static>,
    #[cfg(feature = "engine-debug")]
    pub vk_cmd_begin_debug_utils_label: Option<vk::PFN_vkCmdBeginDebugUtilsLabelEXT>,
    #[cfg(feature = "engine-debug")]
    pub vk_cmd_end_debug_utils_label: Option<vk::PFN_vkCmdEndDebugUtilsLabelEXT>,
}

macro_rules! vg_offset {
    ($field:ident, $expected:expr) => {
        assert!(
            core::mem::size_of::<usize>() != 8
                || core::mem::offset_of!(VulkanGlobals, $field) == $expected
        );
    };
}

const _: () = {
    use core::mem::{offset_of, size_of};
    // 64-bit clang-cl/MSVC/gcc layout (probe of the real glquake.h). Only
    // 64-bit targets are checked, as for `GlTexture` above.
    assert!(size_of::<usize>() != 8 || size_of::<DynBuffer>() == 32);
    assert!(offset_of!(DynBuffer, current_offset) == 8);
    assert!(size_of::<usize>() != 8 || offset_of!(DynBuffer, data) == 16);
    assert!(size_of::<usize>() != 8 || offset_of!(DynBuffer, device_address) == 24);
    assert!(size_of::<VulkanPipelineLayout>() == 24);
    assert!(offset_of!(VulkanPipelineLayout, push_constant_range) == 8);
    assert!(offset_of!(VulkanPipelineLayout, mboit_input_attachment_set) == 20);
    assert!(size_of::<VulkanPipeline>() == 32);
    assert!(offset_of!(VulkanPipeline, layout) == 8);
    assert!(size_of::<VulkanDescSetLayout>() == 40);
    assert!(offset_of!(VulkanDescSetLayout, num_combined_image_samplers) == 8);
    assert!(offset_of!(VulkanDescSetLayout, num_ubos) == 12);
    assert!(offset_of!(VulkanDescSetLayout, num_ubos_dynamic) == 16);
    assert!(offset_of!(VulkanDescSetLayout, num_storage_buffers) == 20);
    assert!(offset_of!(VulkanDescSetLayout, num_input_attachments) == 24);
    assert!(offset_of!(VulkanDescSetLayout, num_storage_images) == 28);
    assert!(offset_of!(VulkanDescSetLayout, num_sampled_images) == 32);
    assert!(offset_of!(VulkanDescSetLayout, num_acceleration_structures) == 36);
    assert!(size_of::<usize>() != 8 || size_of::<BufferCreateInfo>() == 56);
    assert!(size_of::<usize>() != 8 || offset_of!(BufferCreateInfo, size) == 8);
    assert!(size_of::<usize>() != 8 || offset_of!(BufferCreateInfo, alignment) == 16);
    assert!(size_of::<usize>() != 8 || offset_of!(BufferCreateInfo, usage) == 24);
    assert!(size_of::<usize>() != 8 || offset_of!(BufferCreateInfo, mapped) == 32);
    assert!(size_of::<usize>() != 8 || offset_of!(BufferCreateInfo, address) == 40);
    assert!(size_of::<usize>() != 8 || offset_of!(BufferCreateInfo, name) == 48);
    assert!(size_of::<usize>() != 8 || size_of::<CbContext>() == 262216);
    assert!(size_of::<usize>() != 8 || offset_of!(CbContext, current_canvas) == 8);
    assert!(size_of::<usize>() != 8 || offset_of!(CbContext, render_pass) == 16);
    assert!(size_of::<usize>() != 8 || offset_of!(CbContext, render_pass_index) == 24);
    assert!(size_of::<usize>() != 8 || offset_of!(CbContext, subpass) == 28);
    assert!(size_of::<usize>() != 8 || offset_of!(CbContext, current_pipeline) == 32);
    assert!(size_of::<usize>() != 8 || offset_of!(CbContext, vbo_indices) == 64);
    assert!(size_of::<usize>() != 8 || offset_of!(CbContext, num_vbo_indices) == 262208);
    assert!(size_of::<vk::PhysicalDeviceProperties>() == 824);
    assert!(size_of::<vk::PhysicalDeviceFeatures>() == 220);
    assert!(size_of::<vk::PhysicalDeviceMemoryProperties>() == 520);
    assert!(size_of::<vk::ClearValue>() == 16);
    assert!(
        size_of::<usize>() != 8
            || size_of::<vk::PhysicalDeviceAccelerationStructurePropertiesKHR>() == 64
    );
    assert!(size_of::<vk::PushConstantRange>() == 12);
    assert!(
        size_of::<usize>() != 8
            || size_of::<VulkanGlobals>()
                == if cfg!(feature = "engine-debug") {
                    1064968
                } else {
                    1064952
                }
    );
    vg_offset!(device, 0);
    vg_offset!(device_idle, 8);
    vg_offset!(validation, 9);
    vg_offset!(debug_utils, 10);
    vg_offset!(queue, 16);
    vg_offset!(primary_cb_contexts, 24);
    vg_offset!(secondary_cb_contexts, 1048888);
    vg_offset!(color_clear_value, 1049016);
    vg_offset!(swap_chain_format, 1049032);
    vg_offset!(want_full_screen_exclusive, 1049036);
    vg_offset!(swap_chain_full_screen_acquired, 1049038);
    vg_offset!(device_properties, 1049040);
    vg_offset!(device_features, 1049864);
    vg_offset!(memory_properties, 1050088);
    vg_offset!(gfx_queue_family_index, 1050608);
    vg_offset!(color_format, 1050612);
    vg_offset!(depth_format, 1050616);
    vg_offset!(sample_count, 1050620);
    vg_offset!(supersampling, 1050624);
    vg_offset!(present_wait, 1050634);
    vg_offset!(color_buffers, 1050640);
    vg_offset!(oit_accum_buffer, 1050656);
    vg_offset!(mboit_color_buffer, 1050688);
    vg_offset!(fan_index_buffer, 1050696);
    vg_offset!(staging_buffer_size, 1050704);
    vg_offset!(main_render_pass, 1050712);
    vg_offset!(warp_render_pass, 1050760);
    vg_offset!(basic_alphatest_pipeline, 1050768);
    vg_offset!(basic_blend_pipeline, 1050992);
    vg_offset!(basic_notex_blend_pipeline, 1051216);
    vg_offset!(basic_pipeline_layout, 1051440);
    vg_offset!(world_pipelines, 1051464);
    vg_offset!(world_wboit_pipelines, 1053000);
    vg_offset!(world_mboit_moment_pipelines, 1053512);
    vg_offset!(world_mboit_composite_pipelines, 1054024);
    vg_offset!(world_pipeline_layout, 1054536);
    vg_offset!(raster_tex_warp_pipeline, 1054560);
    vg_offset!(particle_pipeline, 1054592);
    vg_offset!(particle_oit_pipeline, 1054624);
    vg_offset!(particle_post_oit_pipeline, 1054656);
    vg_offset!(particle_mboit_moment_pipeline, 1054752);
    vg_offset!(particle_mboit_composite_pipeline, 1054784);
    vg_offset!(sprite_pipeline, 1054816);
    vg_offset!(sprite_oit_pipeline, 1054912);
    vg_offset!(sprite_mboit_moment_pipeline, 1054944);
    vg_offset!(sprite_mboit_composite_pipeline, 1054976);
    vg_offset!(sky_pipeline_layout, 1055008);
    vg_offset!(sky_stencil_pipeline, 1055056);
    vg_offset!(sky_color_pipeline, 1055248);
    vg_offset!(sky_box_pipeline, 1055440);
    vg_offset!(sky_cube_pipeline, 1055536);
    vg_offset!(sky_layer_pipeline, 1055728);
    vg_offset!(alias_pipelines, 1055920);
    vg_offset!(alias_wboit_pipelines, 1056496);
    vg_offset!(alias_mboit_moment_pipelines, 1056688);
    vg_offset!(alias_mboit_composite_pipelines, 1056880);
    vg_offset!(md5_pipelines, 1057072);
    vg_offset!(md5_wboit_pipelines, 1057648);
    vg_offset!(md5_mboit_moment_pipelines, 1057840);
    vg_offset!(md5_mboit_composite_pipelines, 1058032);
    vg_offset!(md5_8_pipelines, 1058224);
    vg_offset!(md5_8_wboit_pipelines, 1058800);
    vg_offset!(md5_8_mboit_moment_pipelines, 1058992);
    vg_offset!(md5_8_mboit_composite_pipelines, 1059184);
    vg_offset!(postprocess_pipeline, 1059376);
    vg_offset!(wboit_resolve_pipeline, 1059408);
    vg_offset!(mboit_resolve_pipeline, 1059440);
    vg_offset!(screen_effects_pipeline, 1059472);
    vg_offset!(screen_effects_scale_pipeline, 1059504);
    vg_offset!(screen_effects_scale_sops_pipeline, 1059536);
    vg_offset!(cs_tex_warp_pipeline, 1059568);
    vg_offset!(showtris_pipeline, 1059600);
    vg_offset!(showtris_indirect_pipeline, 1059696);
    vg_offset!(showtris_depth_test_pipeline, 1059792);
    vg_offset!(showtris_indirect_depth_test_pipeline, 1059888);
    vg_offset!(showbboxes_pipeline, 1059984);
    vg_offset!(update_lightmap_pipeline, 1060080);
    vg_offset!(update_lightmap_rt_pipeline, 1060112);
    vg_offset!(indirect_draw_pipeline, 1060144);
    vg_offset!(indirect_clear_pipeline, 1060176);
    vg_offset!(ray_debug_pipeline, 1060208);
    vg_offset!(mesh_interpolate_pipeline, 1060240);
    vg_offset!(skinning_pipeline, 1060272);
    vg_offset!(skinning_8_pipeline, 1060304);
    vg_offset!(fte_particle_pipelines, 1060336);
    vg_offset!(fte_particle_wboit_pipelines, 1061872);
    vg_offset!(fte_particle_post_oit_pipelines, 1062384);
    vg_offset!(descriptor_pool, 1063920);
    vg_offset!(ubo_set_layout, 1063928);
    vg_offset!(single_texture_set_layout, 1063968);
    vg_offset!(input_attachment_set_layout, 1064008);
    vg_offset!(oit_input_attachment_set_layout, 1064048);
    vg_offset!(mboit_input_attachment_set_layout, 1064088);
    vg_offset!(mboit_input_attachment_descriptor_set, 1064128);
    vg_offset!(screen_effects_desc_set, 1064136);
    vg_offset!(screen_effects_set_layout, 1064144);
    vg_offset!(single_texture_cs_write_set_layout, 1064184);
    vg_offset!(lightmap_compute_set_layout, 1064224);
    vg_offset!(indirect_compute_desc_set, 1064264);
    vg_offset!(indirect_compute_set_layout, 1064272);
    vg_offset!(bmodel_instances_desc_set, 1064312);
    vg_offset!(bmodel_instances_set_layout, 1064320);
    vg_offset!(ray_query_push_set_layout, 1064360);
    vg_offset!(ray_debug_desc_set, 1064400);
    vg_offset!(ray_debug_set_layout, 1064408);
    vg_offset!(joints_buffer_set_layout, 1064448);
    vg_offset!(point_sampler, 1064488);
    vg_offset!(linear_aniso_sampler_lod_bias, 1064544);
    vg_offset!(projection_matrix, 1064552);
    vg_offset!(view_matrix, 1064616);
    vg_offset!(view_projection_matrix, 1064680);
    vg_offset!(vk_cmd_bind_pipeline, 1064744);
    vg_offset!(vk_get_buffer_device_address, 1064840);
    vg_offset!(vk_get_acceleration_structure_build_sizes, 1064848);
    vg_offset!(vk_get_acceleration_structure_device_address, 1064880);
    vg_offset!(physical_device_acceleration_structure_properties, 1064888);
    #[cfg(feature = "engine-debug")]
    vg_offset!(vk_cmd_begin_debug_utils_label, 1064952);
    #[cfg(feature = "engine-debug")]
    vg_offset!(vk_cmd_end_debug_utils_label, 1064960);
};
