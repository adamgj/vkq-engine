//! Hand-written externs for the renderer seams the Rust `gl_heap` and
//! `gl_texmgr` call (Rust migration Phase 8 M3/M4, ADR-015). `glquake.h`,
//! `gl_heap.h` and `gl_texmgr.h` are not bindgen roots (`bindings_wrapper.h`):
//! all three pull `<vulkan/vulkan_core.h>` in, so the C callees are declared
//! here by hand. The pointer parameters are typed on the Rust side by the
//! caller (`quake-capi`'s `gl_heap.rs`/`gl_texmgr.rs`), where the ADR-011
//! mirrors and `ash::vk` types live; this crate has no dependencies.
//! Vulkan handles cross as `u64` (non-dispatchable) or `*mut c_void`
//! (dispatchable), Vulkan enums as `c_int`, Vulkan structs as `*const c_void`
//! to the caller's `ash::vk` structs.

use core::ffi::{c_char, c_int, c_uint, c_void};

use crate::cvar_t;

extern "C" {
    /// `glquake.h:885` -- `void R_AllocateVulkanMemory (vulkan_memory_t
    /// *memory, VkMemoryAllocateInfo *memory_allocate_info,
    /// vulkan_memory_type_t type, atomic_uint32_t *num_allocations)`
    /// (`gl_rmisc.c`). `memory_type` is the C enum (`int`).
    pub fn R_AllocateVulkanMemory(
        memory: *mut c_void,
        memory_allocate_info: *mut c_void,
        memory_type: c_int,
        num_allocations: *mut c_void,
    );
    /// `glquake.h:886` -- `void R_FreeVulkanMemory (vulkan_memory_t *memory,
    /// atomic_uint32_t *num_allocations)`.
    pub fn R_FreeVulkanMemory(memory: *mut c_void, num_allocations: *mut c_void);
    /// `glquake.h:933` -- `void GL_SetObjectName (uint64_t object,
    /// VkObjectType object_type, const char *name)` (`gl_vidsdl.c`).
    /// `object_type` is the Vulkan enum (`int`).
    pub fn GL_SetObjectName(object: u64, object_type: c_int, name: *const c_char);

    /* Phase 8 M4: gl_texmgr.c's seams into the still-C renderer */

    /// `glquake.h:33` -- `void GL_WaitForDeviceIdle (void)` (`gl_vidsdl.c`).
    pub fn GL_WaitForDeviceIdle();
    /// `glquake.h:826` -- `int GL_MemoryTypeFromProperties (uint32_t
    /// type_bits, VkFlags requirements_mask, VkFlags preferred_mask)`.
    pub fn GL_MemoryTypeFromProperties(
        type_bits: u32,
        requirements_mask: u32,
        preferred_mask: u32,
    ) -> c_int;
    /// `glquake.h:908` -- `VkDescriptorSet R_AllocateDescriptorSet
    /// (vulkan_desc_set_layout_t *layout)` (`gl_rmisc.c`).
    pub fn R_AllocateDescriptorSet(layout: *mut c_void) -> u64;
    /// `glquake.h:909` -- `void R_FreeDescriptorSet (VkDescriptorSet
    /// desc_set, vulkan_desc_set_layout_t *layout)`.
    pub fn R_FreeDescriptorSet(desc_set: u64, layout: *mut c_void);
    /// `glquake.h:913` -- `byte *R_StagingAllocate (int size, int alignment,
    /// VkCommandBuffer *cb_context, VkBuffer *buffer, int *buffer_offset)`.
    pub fn R_StagingAllocate(
        size: c_int,
        alignment: c_int,
        cb_context: *mut *mut c_void,
        buffer: *mut u64,
        buffer_offset: *mut c_int,
    ) -> *mut u8;
    /// `glquake.h:914`
    pub fn R_StagingBeginCopy();
    /// `glquake.h:915`
    pub fn R_StagingEndCopy();
    /// `glquake.h:543` -- `extern qboolean in_update_screen;` (`gl_screen.c`).
    pub static mut in_update_screen: bool;
    /// `gl_rmain.c:88` -- `cvar_t gl_fullbrights`.
    pub static mut gl_fullbrights: cvar_t;
    /// `quakedef.h:509` -- `extern atomic_uint32_t num_vulkan_tex_allocations;`
    /// (`gl_rmisc.c`), only ever passed through to the heap counter.
    pub static mut num_vulkan_tex_allocations: u32;
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
    /// Copies the `vulkan_globals` members gl_texmgr.c reads into the
    /// caller's `texmgr_glue_env_t` mirror (layout asserted on both sides).
    pub fn TexMgr_Glue_VulkanEnv(out: *mut c_void);
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
}

// The Vulkan loader entry points gl_texmgr.c calls directly; the engine
// links the loader (MoltenVK on macOS), so these resolve at link time like
// the C's own calls. `VKAPI_CALL` is `__stdcall` on 32-bit Windows, which is
// what `extern "system"` spells.
extern "system" {
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
