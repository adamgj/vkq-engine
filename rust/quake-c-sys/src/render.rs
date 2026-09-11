//! Hand-written externs for the renderer seams the Rust `gl_heap`,
//! `gl_texmgr` and `gl_rmisc` call (Rust migration Phase 8 M3/M4/M5,
//! ADR-015). `glquake.h`, `gl_heap.h` and `gl_texmgr.h` are not bindgen roots
//! (`bindings_wrapper.h`): all three pull `<vulkan/vulkan_core.h>` in, so the
//! C callees are declared here by hand. The pointer parameters are typed on
//! the Rust side by the caller (`quake-capi`'s `gl_heap.rs`/`gl_texmgr.rs`/
//! `gl_rmisc.rs`), where the ADR-011 mirrors and `ash::vk` types live; this
//! crate has no dependencies.
//! Vulkan handles cross as `u64` (non-dispatchable) or `*mut c_void`
//! (dispatchable), Vulkan enums as `c_int`, Vulkan structs as `*const c_void`
//! to the caller's `ash::vk` structs.

use core::ffi::{c_char, c_int, c_uint, c_void};

use crate::cvar_t;

extern "C" {
    /// `glquake.h:933` -- `void GL_SetObjectName (uint64_t object,
    /// VkObjectType object_type, const char *name)` (`gl_vidsdl.c`).
    /// `object_type` is the Vulkan enum (`int`).
    pub fn GL_SetObjectName(object: u64, object_type: c_int, name: *const c_char);

    /* Phase 8 M4: gl_texmgr.c's seams into the still-C renderer */

    /// `glquake.h:33` -- `void GL_WaitForDeviceIdle (void)` (`gl_vidsdl.c`).
    pub fn GL_WaitForDeviceIdle();
    /// `glquake.h:543` -- `extern qboolean in_update_screen;` (`gl_screen.c`).
    pub static mut in_update_screen: bool;
    /// `gl_rmain.c:88` -- `cvar_t gl_fullbrights`.
    pub static mut gl_fullbrights: cvar_t;
    /// `image.h:29` -- `byte *Image_LoadImage (const char *name, int *width,
    /// int *height, enum srcformat *fmt, unsigned int min_path_id)`.
    pub fn Image_LoadImage(
        name: *const c_char,
        width: *mut c_int,
        height: *mut c_int,
        fmt: *mut c_int,
        min_path_id: c_uint,
    ) -> *mut u8;

    /* Quake/gl_texmgr_glue.c -- the C-visible data of gl_texmgr.c and the
     * accessors into glquake.h types this crate cannot spell */

    /// `cvar_t gl_max_size`
    pub static mut gl_max_size: cvar_t;
    /// `cvar_t gl_picmip`
    pub static mut gl_picmip: cvar_t;
    pub static mut d_8to24table: [c_uint; 256];
    pub static mut d_8to24table_fbright: [c_uint; 256];
    pub static mut d_8to24table_fbright_fence: [c_uint; 256];
    pub static mut d_8to24table_nobright: [c_uint; 256];
    pub static mut d_8to24table_nobright_fence: [c_uint; 256];
    pub static mut d_8to24table_conchars: [c_uint; 256];
    /// `gltexture_t *notexture` etc.; `gltexture_t` is the caller's mirror.
    pub static mut notexture: *mut c_void;
    pub static mut nulltexture: *mut c_void;
    pub static mut whitetexture: *mut c_void;
    pub static mut greytexture: *mut c_void;
    pub static mut greylightmap: *mut c_void;
    pub static mut bluenoisetexture: *mut c_void;
    /// `Cvar_RegisterVariable` of `gl_max_size`/`gl_picmip` under
    /// `Host_Guard`; non-zero when a `Host_Error` was caught (ADR-009).
    pub fn TexMgr_Glue_RegisterVariables() -> c_int;
    /// `Cmd_AddCommand ("imagelist", TexMgr_Rust_Imagelist_f)` plus its
    /// completion under `Host_Guard`; non-zero on a caught `Host_Error`.
    pub fn TexMgr_Glue_RegisterCommands() -> c_int;
    /// `((qmodel_t *)owner)->path_id`
    pub fn TexMgr_Glue_OwnerPathId(owner: *const c_void) -> c_uint;
    /// `r_notexture_mip->gltexture = r_notexture_mip2->gltexture = tex`
    pub fn TexMgr_Glue_SetNotextureMips(tex: *mut c_void);
    /// `stbir_resize_uint8` of the RGBA8 image in place (the stb
    /// implementation stays C).
    pub fn TexMgr_Glue_Downsample(
        data: *mut u32,
        in_width: c_int,
        in_height: c_int,
        out_width: c_int,
        out_height: c_int,
    );

    /* Phase 8 M5: gl_rmisc.c's seams into the C that remains */

    /// `harness.h:105` -- `extern qboolean harness_renderhash;` (`harness.c`).
    pub static mut harness_renderhash: bool;
    /// `harness.h:110` -- `void Harness_RenderPipelineCreated (uint64_t
    /// handle, const char *name)` (`harness_render.c`).
    pub fn Harness_RenderPipelineCreated(handle: u64, name: *const c_char);
    /// `harness.h:111` -- `void Harness_RenderPipelinesDestroyed (void)`
    /// (`harness_render.c`).
    pub fn Harness_RenderPipelinesDestroyed();
    /// `Quake/gl_rmisc_glue.c` -- `cvar_t r_lodbias`, `cvar_t gl_lodbias`.
    pub static mut r_lodbias: cvar_t;
    pub static mut gl_lodbias: cvar_t;
    /// `r_brush.c:292` -- `vulkan_memory_t frame_upload_buffers_memory`,
    /// spelled as three words (handle, size, type + padding; the caller
    /// asserts the size against its `VulkanMemory` mirror).
    pub static mut frame_upload_buffers_memory: [u64; 3];
    /// The `bintoc` SPIR-V arrays (`Shaders/bintoc.c`): `const unsigned
    /// char <name>_spv[]` and `const int <name>_spv_size`, one pair per
    /// `DECLARE_SHADER_MODULE` line of `gl_rmisc.c`, in that order.
    pub static basic_vert_spv: [u8; 0];
    pub static basic_vert_spv_size: c_int;
    pub static basic_frag_spv: [u8; 0];
    pub static basic_frag_spv_size: c_int;
    pub static basic_oit_frag_spv: [u8; 0];
    pub static basic_oit_frag_spv_size: c_int;
    pub static basic_mboit_moment_frag_spv: [u8; 0];
    pub static basic_mboit_moment_frag_spv_size: c_int;
    pub static basic_mboit_composite_frag_spv: [u8; 0];
    pub static basic_mboit_composite_frag_spv_size: c_int;
    pub static basic_mboit_composite_msaa_frag_spv: [u8; 0];
    pub static basic_mboit_composite_msaa_frag_spv_size: c_int;
    pub static basic_alphatest_frag_spv: [u8; 0];
    pub static basic_alphatest_frag_spv_size: c_int;
    pub static basic_notex_frag_spv: [u8; 0];
    pub static basic_notex_frag_spv_size: c_int;
    pub static world_vert_spv: [u8; 0];
    pub static world_vert_spv_size: c_int;
    pub static world_frag_spv: [u8; 0];
    pub static world_frag_spv_size: c_int;
    pub static world_oit_frag_spv: [u8; 0];
    pub static world_oit_frag_spv_size: c_int;
    pub static world_mboit_moment_frag_spv: [u8; 0];
    pub static world_mboit_moment_frag_spv_size: c_int;
    pub static world_mboit_composite_frag_spv: [u8; 0];
    pub static world_mboit_composite_frag_spv_size: c_int;
    pub static world_mboit_composite_msaa_frag_spv: [u8; 0];
    pub static world_mboit_composite_msaa_frag_spv_size: c_int;
    pub static alias_vert_spv: [u8; 0];
    pub static alias_vert_spv_size: c_int;
    pub static alias_frag_spv: [u8; 0];
    pub static alias_frag_spv_size: c_int;
    pub static alias_alphatest_frag_spv: [u8; 0];
    pub static alias_alphatest_frag_spv_size: c_int;
    pub static alias_oit_frag_spv: [u8; 0];
    pub static alias_oit_frag_spv_size: c_int;
    pub static alias_alphatest_oit_frag_spv: [u8; 0];
    pub static alias_alphatest_oit_frag_spv_size: c_int;
    pub static alias_mboit_moment_frag_spv: [u8; 0];
    pub static alias_mboit_moment_frag_spv_size: c_int;
    pub static alias_mboit_composite_frag_spv: [u8; 0];
    pub static alias_mboit_composite_frag_spv_size: c_int;
    pub static alias_mboit_composite_msaa_frag_spv: [u8; 0];
    pub static alias_mboit_composite_msaa_frag_spv_size: c_int;
    pub static alias_alphatest_mboit_moment_frag_spv: [u8; 0];
    pub static alias_alphatest_mboit_moment_frag_spv_size: c_int;
    pub static alias_alphatest_mboit_composite_frag_spv: [u8; 0];
    pub static alias_alphatest_mboit_composite_frag_spv_size: c_int;
    pub static alias_alphatest_mboit_composite_msaa_frag_spv: [u8; 0];
    pub static alias_alphatest_mboit_composite_msaa_frag_spv_size: c_int;
    pub static md5_mboit_composite_frag_spv: [u8; 0];
    pub static md5_mboit_composite_frag_spv_size: c_int;
    pub static md5_mboit_composite_msaa_frag_spv: [u8; 0];
    pub static md5_mboit_composite_msaa_frag_spv_size: c_int;
    pub static md5_alphatest_mboit_composite_frag_spv: [u8; 0];
    pub static md5_alphatest_mboit_composite_frag_spv_size: c_int;
    pub static md5_alphatest_mboit_composite_msaa_frag_spv: [u8; 0];
    pub static md5_alphatest_mboit_composite_msaa_frag_spv_size: c_int;
    pub static md5_vert_spv: [u8; 0];
    pub static md5_vert_spv_size: c_int;
    pub static md5_8_vert_spv: [u8; 0];
    pub static md5_8_vert_spv_size: c_int;
    pub static sky_layer_vert_spv: [u8; 0];
    pub static sky_layer_vert_spv_size: c_int;
    pub static sky_layer_frag_spv: [u8; 0];
    pub static sky_layer_frag_spv_size: c_int;
    pub static sky_box_frag_spv: [u8; 0];
    pub static sky_box_frag_spv_size: c_int;
    pub static sky_cube_vert_spv: [u8; 0];
    pub static sky_cube_vert_spv_size: c_int;
    pub static sky_cube_frag_spv: [u8; 0];
    pub static sky_cube_frag_spv_size: c_int;
    pub static postprocess_vert_spv: [u8; 0];
    pub static postprocess_vert_spv_size: c_int;
    pub static postprocess_frag_spv: [u8; 0];
    pub static postprocess_frag_spv_size: c_int;
    pub static wboit_resolve_frag_spv: [u8; 0];
    pub static wboit_resolve_frag_spv_size: c_int;
    pub static wboit_resolve_msaa_frag_spv: [u8; 0];
    pub static wboit_resolve_msaa_frag_spv_size: c_int;
    pub static mboit_resolve_frag_spv: [u8; 0];
    pub static mboit_resolve_frag_spv_size: c_int;
    pub static mboit_resolve_msaa_frag_spv: [u8; 0];
    pub static mboit_resolve_msaa_frag_spv_size: c_int;
    pub static screen_effects_8bit_comp_spv: [u8; 0];
    pub static screen_effects_8bit_comp_spv_size: c_int;
    pub static screen_effects_8bit_scale_comp_spv: [u8; 0];
    pub static screen_effects_8bit_scale_comp_spv_size: c_int;
    pub static screen_effects_8bit_scale_sops_comp_spv: [u8; 0];
    pub static screen_effects_8bit_scale_sops_comp_spv_size: c_int;
    pub static screen_effects_10bit_comp_spv: [u8; 0];
    pub static screen_effects_10bit_comp_spv_size: c_int;
    pub static screen_effects_10bit_scale_comp_spv: [u8; 0];
    pub static screen_effects_10bit_scale_comp_spv_size: c_int;
    pub static screen_effects_10bit_scale_sops_comp_spv: [u8; 0];
    pub static screen_effects_10bit_scale_sops_comp_spv_size: c_int;
    pub static cs_tex_warp_comp_spv: [u8; 0];
    pub static cs_tex_warp_comp_spv_size: c_int;
    pub static indirect_comp_spv: [u8; 0];
    pub static indirect_comp_spv_size: c_int;
    pub static indirect_clear_comp_spv: [u8; 0];
    pub static indirect_clear_comp_spv_size: c_int;
    pub static showtris_vert_spv: [u8; 0];
    pub static showtris_vert_spv_size: c_int;
    pub static showtris_frag_spv: [u8; 0];
    pub static showtris_frag_spv_size: c_int;
    pub static update_lightmap_8bit_comp_spv: [u8; 0];
    pub static update_lightmap_8bit_comp_spv_size: c_int;
    pub static update_lightmap_10bit_comp_spv: [u8; 0];
    pub static update_lightmap_10bit_comp_spv_size: c_int;
    pub static update_lightmap_8bit_rt_comp_spv: [u8; 0];
    pub static update_lightmap_8bit_rt_comp_spv_size: c_int;
    pub static update_lightmap_10bit_rt_comp_spv: [u8; 0];
    pub static update_lightmap_10bit_rt_comp_spv_size: c_int;
    pub static ray_debug_comp_spv: [u8; 0];
    pub static ray_debug_comp_spv_size: c_int;
    pub static mesh_interpolate_comp_spv: [u8; 0];
    pub static mesh_interpolate_comp_spv_size: c_int;
    pub static skinning_comp_spv: [u8; 0];
    pub static skinning_comp_spv_size: c_int;
    pub static skinning_8_comp_spv: [u8; 0];
    pub static skinning_8_comp_spv_size: c_int;
}

// The Vulkan loader entry points gl_texmgr.c and gl_rmisc.c call directly; the engine
// links the loader (MoltenVK on macOS), so these resolve at link time like
// the C's own calls. `VKAPI_CALL` is `__stdcall` on 32-bit Windows, which is
// what `extern "system"` spells.
extern "system" {
    /// `PFN_vkGetDeviceProcAddr`: `quake-capi`'s `gl_rmisc.rs` loads its
    /// `ash::Device` table through it (Phase 8 M5).
    pub fn vkGetDeviceProcAddr(device: *mut c_void, name: *const c_char) -> *const c_void;
    pub fn vkCreateImage(
        device: *mut c_void,
        create_info: *const c_void,
        allocator: *const c_void,
        image: *mut u64,
    ) -> c_int;
    pub fn vkDestroyImage(device: *mut c_void, image: u64, allocator: *const c_void);
    pub fn vkGetImageMemoryRequirements(
        device: *mut c_void,
        image: u64,
        memory_requirements: *mut c_void,
    );
    pub fn vkBindImageMemory(
        device: *mut c_void,
        image: u64,
        memory: u64,
        memory_offset: u64,
    ) -> c_int;
    pub fn vkCreateImageView(
        device: *mut c_void,
        create_info: *const c_void,
        allocator: *const c_void,
        view: *mut u64,
    ) -> c_int;
    pub fn vkDestroyImageView(device: *mut c_void, view: u64, allocator: *const c_void);
    pub fn vkCreateFramebuffer(
        device: *mut c_void,
        create_info: *const c_void,
        allocator: *const c_void,
        framebuffer: *mut u64,
    ) -> c_int;
    pub fn vkDestroyFramebuffer(device: *mut c_void, framebuffer: u64, allocator: *const c_void);
    pub fn vkUpdateDescriptorSets(
        device: *mut c_void,
        descriptor_write_count: u32,
        descriptor_writes: *const c_void,
        descriptor_copy_count: u32,
        descriptor_copies: *const c_void,
    );
    pub fn vkCmdPipelineBarrier(
        command_buffer: *mut c_void,
        src_stage_mask: u32,
        dst_stage_mask: u32,
        dependency_flags: u32,
        memory_barrier_count: u32,
        memory_barriers: *const c_void,
        buffer_memory_barrier_count: u32,
        buffer_memory_barriers: *const c_void,
        image_memory_barrier_count: u32,
        image_memory_barriers: *const c_void,
    );
    pub fn vkCmdCopyBufferToImage(
        command_buffer: *mut c_void,
        src_buffer: u64,
        dst_image: u64,
        dst_image_layout: c_int,
        region_count: u32,
        regions: *const c_void,
    );
}
