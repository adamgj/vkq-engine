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
//! Name-keyed like the Phase 3/4 probes so this consumer and the C table can't
//! drift by index; an unknown key returns usize::MAX and fails the assert.

use core::mem::{offset_of, size_of};

use quake_ctest as _;
use quake_types::render::{
    GlHeapStats, GlTexture, SrcFormat, VulkanMemory, VulkanMemoryType, TEXPREF_ALPHA,
    TEXPREF_ALPHAPIXELS, TEXPREF_CONCHARS, TEXPREF_FULLBRIGHT, TEXPREF_LINEAR, TEXPREF_MIPMAP,
    TEXPREF_NEAREST, TEXPREF_NOBRIGHT, TEXPREF_NOPICMIP, TEXPREF_OVERWRITE, TEXPREF_PAD,
    TEXPREF_PERSIST, TEXPREF_PREMULTIPLY, TEXPREF_WARPIMAGE,
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
