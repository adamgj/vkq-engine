//! ABI cross-check: the `quake_types::render` mirrors vs the C probe
//! (Phase 8 M3, ADR-011). Under `-Duse_rust_render` the Rust heap hands
//! `glheapstats_t` back through `GL_HeapGetStats` and fills
//! `vulkan_memory_t` for the C `R_AllocateVulkanMemory` seam, so mirror
//! drift is silent memory corruption rather than a link error.
//!
//! What the probe measures: `glheapstats_t` is the engine's own `gl_heap.h`;
//! `vulkan_memory_t`, `vulkan_memory_type_t` and `VkDeviceMemory` are the
//! prelude's hand copies of `glquake.h:178-190` and `vulkan_core.h`
//! (`c_ref_prelude.h`), because the real headers pull in the Vulkan SDK.
//! Those rows therefore check mirror-vs-copy; the check against the real
//! `glquake.h` is the `COMPILE_TIME_ASSERT`s in `Quake/gl_heap_glue.c`,
//! which the `use_rust_render` build compiles with the SDK header in scope.
//!
//! Phase 8 M4 adds `gltexture_t` (the Rust texture manager's list node,
//! read by every C renderer file through the C view) and its `srcformat` /
//! `textureflags_t` constants, again against the prelude's copy of
//! `gl_texmgr.h` with the engine-header check in `Quake/gl_texmgr_glue.c`.
//!
//! Phase 8 M5 adds the structs the Rust `gl_rmisc.c` hands across the seam
//! by pointer (`dynbuffer_t`, `vulkan_desc_set_layout_t`,
//! `buffer_create_info_t`) and the two pipeline structs, against the
//! prelude's copies of `glquake.h`. `vulkanglobals_t` is not probed here --
//! its prelude copy is a compile-only cut-down (the real shape needs most of
//! `vulkan_core.h`) -- so its only check is the `COMPILE_TIME_ASSERT` block
//! in `Quake/gl_rmisc_glue.c`, which the `use_rust_render` build compiles
//! against the real header on every CI leg.
//!
//! Phase 8 M9 adds `cb_context_t` (the Rust frame graph owns the
//! `secondary_cb_contexts` array; the prelude copy is now verbatim) and the
//! M8 mirrors the C glue reads by layout (`lerpdata_t`, `struct lightmap_s`
//! and its member structs), with the engine-header check in
//! `Quake/gl_rmain_glue.c`.
//!
//! Name-keyed like the Phase 3/4 probes so this consumer and the C table can't
//! drift by index; an unknown key returns usize::MAX and fails the assert.

use core::mem::{offset_of, size_of};

use quake_ctest as _;
use quake_types::render::{
    BufferCreateInfo, CbContext, DynBuffer, GlHeapStats, GlMaxUsed, GlRect, GlTexture, LerpData,
    Lightmap, LmComputeWorkgroupBounds, SrcFormat, VulkanDescSetLayout, VulkanMemory,
    VulkanMemoryType, VulkanPipeline, VulkanPipelineLayout, LMBLOCK_HEIGHT, LMBLOCK_WIDTH,
    LM_CULL_BLOCK_H, LM_CULL_BLOCK_W, MAX_BATCH_SIZE, MAX_LIGHTSTYLES, TASKS_MAX_WORKERS,
    TEXPREF_ALPHA, TEXPREF_ALPHAPIXELS, TEXPREF_CONCHARS, TEXPREF_FULLBRIGHT, TEXPREF_LINEAR,
    TEXPREF_MIPMAP, TEXPREF_NEAREST, TEXPREF_NOBRIGHT, TEXPREF_NOPICMIP, TEXPREF_OVERWRITE,
    TEXPREF_PAD, TEXPREF_PERSIST, TEXPREF_PREMULTIPLY, TEXPREF_WARPIMAGE,
};

extern "C" {
    fn ctest_abi_render_lookup(key: *const core::ffi::c_char) -> usize;
}

fn c_abi(key: &str) -> usize {
    let cstr = std::ffi::CString::new(key).unwrap();
    // SAFETY: the probe only strcmp's the key against a compile-time table.
    let v = unsafe { ctest_abi_render_lookup(cstr.as_ptr()) };
    assert_ne!(v, usize::MAX, "key {key:?} missing from the C probe table");
    v
}

macro_rules! check_size {
    ($rust:ty, $ctag:literal) => {
        assert_eq!(
            size_of::<$rust>(),
            c_abi(concat!("sizeof.", $ctag)),
            concat!("sizeof ", $ctag)
        );
    };
}

macro_rules! check_offsets {
    ($rust:ty, $ctag:literal, [$($field:ident),+ $(,)?]) => {
        $(
            assert_eq!(
                offset_of!($rust, $field),
                c_abi(concat!($ctag, ".", stringify!($field))),
                concat!($ctag, ".", stringify!($field))
            );
        )+
    };
}

#[test]
fn render_mirrors_match_engine_headers() {
    check_size!(GlHeapStats, "glheapstats_t");
    check_offsets!(
        GlHeapStats,
        "glheapstats_t",
        [
            num_segments,
            num_allocations,
            num_small_allocations,
            num_block_allocations,
            num_dedicated_allocations,
            num_blocks_used,
            num_blocks_free,
            num_pages_allocated,
            num_pages_free,
            num_bytes_allocated,
            num_bytes_free,
            num_bytes_wasted
        ]
    );

    check_size!(VulkanMemory, "vulkan_memory_t");
    check_offsets!(VulkanMemory, "vulkan_memory_t", [handle, size]);
    // `type` is a keyword, so the mirror spells it `type_`
    assert_eq!(
        offset_of!(VulkanMemory, type_),
        c_abi("vulkan_memory_t.type"),
        "vulkan_memory_t.type"
    );

    check_size!(VulkanMemoryType, "vulkan_memory_type_t");
    // D2: ash's DeviceMemory is a repr(transparent) u64 on every target; the
    // prelude's VkDeviceMemory follows vulkan_core.h's
    // VK_USE_64_BIT_PTR_DEFINES rule (pointer on 64-bit, uint64_t
    // otherwise). Only 64-bit targets have run this so far.
    assert_eq!(
        size_of::<ash::vk::DeviceMemory>(),
        c_abi("sizeof.VkDeviceMemory")
    );
}

#[test]
fn render_consts_match_engine_headers() {
    assert_eq!(
        VulkanMemoryType::None as usize,
        c_abi("const.VULKAN_MEMORY_TYPE_NONE")
    );
    assert_eq!(
        VulkanMemoryType::Device as usize,
        c_abi("const.VULKAN_MEMORY_TYPE_DEVICE")
    );
    assert_eq!(
        VulkanMemoryType::Host as usize,
        c_abi("const.VULKAN_MEMORY_TYPE_HOST")
    );
}

#[test]
fn texture_mirror_matches_engine_headers() {
    check_size!(GlTexture, "gltexture_t");
    check_offsets!(
        GlTexture,
        "gltexture_t",
        [
            next,
            owner,
            name,
            path_id,
            width,
            height,
            flags,
            source_file,
            source_offset,
            source_format,
            source_width,
            source_height,
            source_crc,
            shirt,
            pants,
            image,
            image_view,
            target_image_view,
            allocation,
            descriptor_set,
            frame_buffer,
            storage_descriptor_set,
        ]
    );
    assert_eq!(size_of::<SrcFormat>(), c_abi("sizeof.enum srcformat"));
}

#[test]
fn texture_consts_match_engine_headers() {
    for (name, value) in [
        ("SRC_INDEXED", SrcFormat::Indexed as usize),
        ("SRC_LIGHTMAP", SrcFormat::Lightmap as usize),
        ("SRC_RGBA", SrcFormat::Rgba as usize),
        ("SRC_SURF_INDICES", SrcFormat::SurfIndices as usize),
        ("SRC_RGBA_CUBEMAP", SrcFormat::RgbaCubemap as usize),
        ("SRC_INDEXED_PALETTE", SrcFormat::IndexedPalette as usize),
        ("TEXPREF_MIPMAP", TEXPREF_MIPMAP as usize),
        ("TEXPREF_LINEAR", TEXPREF_LINEAR as usize),
        ("TEXPREF_NEAREST", TEXPREF_NEAREST as usize),
        ("TEXPREF_ALPHA", TEXPREF_ALPHA as usize),
        ("TEXPREF_PAD", TEXPREF_PAD as usize),
        ("TEXPREF_PERSIST", TEXPREF_PERSIST as usize),
        ("TEXPREF_OVERWRITE", TEXPREF_OVERWRITE as usize),
        ("TEXPREF_NOPICMIP", TEXPREF_NOPICMIP as usize),
        ("TEXPREF_FULLBRIGHT", TEXPREF_FULLBRIGHT as usize),
        ("TEXPREF_NOBRIGHT", TEXPREF_NOBRIGHT as usize),
        ("TEXPREF_CONCHARS", TEXPREF_CONCHARS as usize),
        ("TEXPREF_WARPIMAGE", TEXPREF_WARPIMAGE as usize),
        ("TEXPREF_PREMULTIPLY", TEXPREF_PREMULTIPLY as usize),
        ("TEXPREF_ALPHAPIXELS", TEXPREF_ALPHAPIXELS as usize),
    ] {
        assert_eq!(value, c_abi(&format!("const.{name}")), "{name}");
    }
}

#[test]
fn rmisc_mirrors_match_engine_headers() {
    check_size!(DynBuffer, "dynbuffer_t");
    check_offsets!(
        DynBuffer,
        "dynbuffer_t",
        [buffer, current_offset, data, device_address]
    );
    check_size!(VulkanPipelineLayout, "vulkan_pipeline_layout_t");
    check_offsets!(
        VulkanPipelineLayout,
        "vulkan_pipeline_layout_t",
        [handle, push_constant_range, mboit_input_attachment_set]
    );
    check_size!(VulkanPipeline, "vulkan_pipeline_t");
    check_offsets!(VulkanPipeline, "vulkan_pipeline_t", [handle, layout]);
    check_size!(VulkanDescSetLayout, "vulkan_desc_set_layout_t");
    check_offsets!(
        VulkanDescSetLayout,
        "vulkan_desc_set_layout_t",
        [
            handle,
            num_combined_image_samplers,
            num_ubos,
            num_ubos_dynamic,
            num_storage_buffers,
            num_input_attachments,
            num_storage_images,
            num_sampled_images,
            num_acceleration_structures,
        ]
    );
    check_size!(BufferCreateInfo, "buffer_create_info_t");
    check_offsets!(
        BufferCreateInfo,
        "buffer_create_info_t",
        [buffer, size, alignment, usage, mapped, address, name]
    );
    assert_eq!(
        size_of::<ash::vk::PushConstantRange>(),
        c_abi("sizeof.VkPushConstantRange")
    );
    assert_eq!(
        size_of::<ash::vk::DescriptorSetLayout>(),
        c_abi("sizeof.VkDescriptorSetLayout")
    );
}

#[test]
fn frame_graph_mirrors_match_engine_headers() {
    check_size!(CbContext, "cb_context_t");
    check_offsets!(
        CbContext,
        "cb_context_t",
        [
            cb,
            current_canvas,
            render_pass,
            render_pass_index,
            subpass,
            current_pipeline,
            vbo_indices,
            num_vbo_indices,
        ]
    );
    assert_eq!(MAX_BATCH_SIZE, c_abi("const.MAX_BATCH_SIZE"));
    check_size!(LerpData, "lerpdata_t");
    check_offsets!(
        LerpData,
        "lerpdata_t",
        [pose1, pose2, blend, origin, angles]
    );
    check_size!(LmComputeWorkgroupBounds, "lm_compute_workgroup_bounds_t");
    check_offsets!(
        LmComputeWorkgroupBounds,
        "lm_compute_workgroup_bounds_t",
        [mins, maxs, submodel]
    );
    check_size!(GlRect, "glRect_t");
    check_offsets!(GlRect, "glRect_t", [l, t, w, h]);
    check_size!(GlMaxUsed, "glMaxUsed_t");
    check_offsets!(GlMaxUsed, "glMaxUsed_t", [w, h]);
    check_size!(Lightmap, "struct lightmap_s");
    check_offsets!(
        Lightmap,
        "struct lightmap_s",
        [
            texture,
            surface_indices_texture,
            lightstyle_textures,
            descriptor_set,
            modified,
            workgroup_bounds_buffer,
            rectchange,
            lightstyle_rectused,
            global_bounds,
            active_dlights,
            block_has_submodels,
            num_used_lightstyles,
            used_lightstyles,
            cached_light,
            cached_framecount,
            data,
            lightstyle_data,
            surface_indices,
            workgroup_bounds,
        ]
    );
    assert_eq!(LMBLOCK_WIDTH, c_abi("const.LMBLOCK_WIDTH"));
    assert_eq!(LMBLOCK_HEIGHT, c_abi("const.LMBLOCK_HEIGHT"));
    assert_eq!(LM_CULL_BLOCK_W, c_abi("const.LM_CULL_BLOCK_W"));
    assert_eq!(LM_CULL_BLOCK_H, c_abi("const.LM_CULL_BLOCK_H"));
    assert_eq!(TASKS_MAX_WORKERS, c_abi("const.TASKS_MAX_WORKERS"));
    assert_eq!(MAX_LIGHTSTYLES, c_abi("const.MAX_LIGHTSTYLES"));
}
