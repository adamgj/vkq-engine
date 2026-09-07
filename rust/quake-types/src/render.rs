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
