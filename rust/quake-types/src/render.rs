//! Renderer ABI mirrors (`Quake/glquake.h`, `Quake/gl_heap.h`) -- Rust
//! migration Phase 8 M3 (ADR-011, ADR-015). Under `-Duse_rust_render` the
//! Rust `gl_heap` fills `vulkan_memory_t` through the C
//! `R_AllocateVulkanMemory` and hands `glheapstats_t` back to C readers
//! (`gl_mesh.c`, `gl_texmgr.c`), so layout drift is silent memory
//! corruption. Verified per-platform by `quake-ctest/tests/render_abi.rs`
//! against the engine's own headers.
//!
//! `VkDeviceMemory` is a pointer typedef on 64-bit C and a `uint64_t` on
//! 32-bit; `ash::vk::DeviceMemory` is a `repr(transparent)` `u64` on both,
//! which the ABI probe's `sizeof` row confirms (task plan D2).

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
    assert!(core::mem::size_of::<GlHeapStats>() == 64);
    assert!(core::mem::offset_of!(GlHeapStats, num_bytes_allocated) == 40);
};
