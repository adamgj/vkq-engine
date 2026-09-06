//! ABI cross-check: the `quake_types::render` mirrors vs what the engine's own
//! `gl_heap.h`/`glquake.h` say on this platform (Phase 8 M3, ADR-011). Under
//! `-Duse_rust_render` the Rust heap hands `glheapstats_t` back through
//! `GL_HeapGetStats` and fills `vulkan_memory_t` for the C
//! `R_AllocateVulkanMemory` seam, so mirror drift is silent memory
//! corruption rather than a link error.
//!
//! Name-keyed like the Phase 3/4 probes so this consumer and the C table can't
//! drift by index; an unknown key returns usize::MAX and fails the assert.

use core::mem::{offset_of, size_of};

use quake_ctest as _;
use quake_types::render::{GlHeapStats, VulkanMemory, VulkanMemoryType};

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
    // C handle is a pointer on 64-bit and a uint64_t on 32-bit
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
