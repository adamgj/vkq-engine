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
