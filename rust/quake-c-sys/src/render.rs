//! Hand-written externs for the renderer seams the Rust `gl_heap` calls
//! (Rust migration Phase 8 M3, ADR-015). `glquake.h` and `gl_heap.h` are not
//! bindgen roots (`bindings_wrapper.h`): both pull `<vulkan/vulkan_core.h>`
//! in, so the three C callees are declared here by hand. The pointer
//! parameters are typed on the Rust side by the caller (`quake-capi`'s
//! `gl_heap.rs`), where the ADR-011 mirrors and `ash::vk` types live; this
//! crate has no dependencies.

use core::ffi::{c_char, c_int, c_void};

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
}
