//! `gl_texmgr.c` -- the texture manager (Rust migration Phase 8 M4,
//! ADR-015). This is the whole file except its process-level seams: the
//! `gltexture_t` arena and active/free lists (C readers keep dereferencing
//! the pointers, so the arena is the ADR-011 `GlTexture` mirror and the
//! `next` links are written exactly as the C does), the two-slot garbage
//! ring, the texture heap (`crate::heap`), the 8-bit loaders, the 32-bit
//! Vulkan image path, the palette builders and the blue-noise tile.
//!
//! Everything that touches the process -- Vulkan entry points,
//! `vulkan_globals`, cvars, the staging ring, `COM_FOpenFile`,
//! `Image_LoadImage`, `Con_Printf`, `Sys_Error` and the stb downsampler --
//! goes through [`TexMgrBackend`], so the same code runs under the engine
//! (`quake-capi`) and under the `quake-ctest` differential with a fake
//! backend that logs every call.
//!
//! Locking: the C serialises with a recursive `texmgr_mutex` and releases it
//! around the CPU work of `TexMgr_LoadImage32` (premultiply, downsample,
//! staging copy) so task workers can load textures in parallel. This crate
//! holds no lock; the entry points that the C runs lock/unlock/lock are
//! written against [`TexMgrLock`], so the caller decides the granularity:
//! the engine shim passes its `Mutex` (C granularity), everything else
//! passes [`Direct`] (already inside one critical section).
//!
//! Pixel data crosses as raw pointers, like the C: the loaders read (and,
//! as the C does, write -- the `shot1sid` patch, premultiply and in-place
//! downsampling) the caller's buffer, whose length is implied by the
//! texture's dimensions and format. Those are the crate's `unsafe` blocks
//! (ADR-004: `quake-render` is one of the crates that may carry them), each
//! documenting the C contract it relies on.

use core::cell::RefCell;
use core::ffi::{c_int, c_void, CStr};
use core::ptr;

use ash::vk;
use ash::vk::Handle;
use quake_types::render::{
    GlHeapStats, GlTexture, SrcFormat, VulkanMemoryType, TEXPREF_ALPHA, TEXPREF_ALPHAPIXELS,
    TEXPREF_CONCHARS, TEXPREF_FULLBRIGHT, TEXPREF_LINEAR, TEXPREF_MIPMAP, TEXPREF_NEAREST,
    TEXPREF_NOBRIGHT, TEXPREF_NOPICMIP, TEXPREF_OVERWRITE, TEXPREF_PERSIST, TEXPREF_PREMULTIPLY,
    TEXPREF_WARPIMAGE,
};

use crate::heap::{Allocation, DeviceMemoryBackend, Heap};

mod bluenoise;
pub use bluenoise::BLUENOISE_DATA;

/// `MAX_GLTEXTURES` (`gl_texmgr.h`)
pub const MAX_GLTEXTURES: usize = 16 * 4096;
/// `LIGHTMAP_BYTES` (`gl_texmgr.h`)
pub const LIGHTMAP_BYTES: usize = 4;
/// `WARPIMAGEMIPS` (`gl_texmgr.h`)
pub const WARPIMAGEMIPS: i32 = 5;
/// `MAX_MIPS` (`gl_texmgr.c`)
pub const MAX_MIPS: usize = 16;
/// `TEXTURE_HEAP_MEMORY_SIZE_MB` (`gl_texmgr.c`)
pub const TEXTURE_HEAP_MEMORY_SIZE_MB: u64 = 64;
/// `TEXTURE_HEAP_PAGE_SIZE` (`gl_texmgr.c`)
pub const TEXTURE_HEAP_PAGE_SIZE: u32 = 16384;
const TOP_RANGE: usize = 16;
const BOTTOM_RANGE: usize = 96;

/// The `vulkan_globals` members `gl_texmgr.c` reads, sampled once per
/// `TexMgr_*` entry point: `vid_restart`/`vid_anisotropic` recreate the
/// samplers, but only on the main thread between entry points.
#[derive(Clone, Copy, Debug, Default)]
pub struct Env {
    pub max_image_dimension_2d: u32,
    pub max_image_dimension_cube: u32,
    pub color_format: vk::Format,
    pub point_sampler_lod_bias: vk::Sampler,
    pub linear_sampler_lod_bias: vk::Sampler,
    pub point_aniso_sampler_lod_bias: vk::Sampler,
    pub linear_aniso_sampler_lod_bias: vk::Sampler,
    pub warp_render_pass: vk::RenderPass,
}

/// The cvar values `gl_texmgr.c` reads (`.value` of each).
#[derive(Clone, Copy, Debug, Default)]
pub struct Cvars {
    pub gl_fullbrights: f32,
    pub vid_filter: f32,
    pub vid_anisotropic: f32,
    pub gl_max_size: f32,
    pub gl_picmip: f32,
}

/// Which `vulkan_desc_set_layout_t` a descriptor set comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DescLayout {
    /// `vulkan_globals.single_texture_set_layout`
    SingleTexture,
    /// `vulkan_globals.single_texture_cs_write_set_layout`
    SingleTextureCsWrite,
}

/// One `vkUpdateDescriptorSets` call with a single `VkWriteDescriptorSet`
/// of one `VkDescriptorImageInfo` -- the only shape `gl_texmgr.c` issues.
#[derive(Clone, Copy, Debug)]
pub struct DescriptorWrite {
    pub set: vk::DescriptorSet,
    pub binding: u32,
    pub descriptor_type: vk::DescriptorType,
    pub sampler: vk::Sampler,
    pub image_view: vk::ImageView,
    pub image_layout: vk::ImageLayout,
}

/// What `R_StagingAllocate` hands back.
#[derive(Clone, Copy, Debug)]
pub struct Staging {
    pub memory: *mut u8,
    pub command_buffer: vk::CommandBuffer,
    pub buffer: vk::Buffer,
    pub offset: i32,
}

/// `COM_FOpenFile` + `Sys_fseek` + `fread` as `TexMgr_ReloadImage` and
/// `TexMgr_LoadPalette` use them.
#[derive(Clone, Debug)]
pub enum SourceRead {
    /// `COM_FOpenFile` found nothing.
    NotFound,
    /// The file opened but `fread` came up short.
    Short,
    Ok(Vec<u8>),
}

/// A successful `Image_LoadImage`: `data` is `Mem_Alloc`ed by the engine and
/// goes back through [`TexMgrBackend::free_loaded`].
#[derive(Clone, Copy, Debug)]
pub struct LoadedImage {
    pub data: *mut u8,
    pub width: i32,
    pub height: i32,
    pub format: c_int,
}

/// The process seams of `gl_texmgr.c`. `Result<_, i32>` carries the
/// `VkResult` the C prints in its `Sys_Error`.
pub trait TexMgrBackend {
    type HeapBackend: DeviceMemoryBackend;

    /// The backend for the texture heap (`GL_HeapCreate (..., "Texture Heap")`).
    fn new_heap_backend(&self) -> Self::HeapBackend;
    /// `&num_vulkan_tex_allocations`
    fn heap_counter(&self) -> <Self::HeapBackend as DeviceMemoryBackend>::Counter;

    fn env(&self) -> Env;
    fn cvars(&self) -> Cvars;
    /// `no_rendering`
    fn no_rendering(&self) -> bool;
    /// `in_update_screen`
    fn in_update_screen(&self) -> bool;

    fn create_image(&self, info: &vk::ImageCreateInfo<'_>) -> Result<vk::Image, i32>;
    fn destroy_image(&self, image: vk::Image);
    fn image_memory_requirements(&self, image: vk::Image) -> vk::MemoryRequirements;
    fn bind_image_memory(
        &self,
        image: vk::Image,
        memory: vk::DeviceMemory,
        offset: u64,
    ) -> Result<(), i32>;
    fn create_image_view(&self, info: &vk::ImageViewCreateInfo<'_>) -> Result<vk::ImageView, i32>;
    fn destroy_image_view(&self, view: vk::ImageView);
    fn create_framebuffer(
        &self,
        info: &vk::FramebufferCreateInfo<'_>,
    ) -> Result<vk::Framebuffer, i32>;
    fn destroy_framebuffer(&self, framebuffer: vk::Framebuffer);
    fn update_descriptor_set(&self, write: &DescriptorWrite);
    /// `R_AllocateDescriptorSet`
    fn allocate_descriptor_set(&self, layout: DescLayout) -> vk::DescriptorSet;
    /// `R_FreeDescriptorSet`
    fn free_descriptor_set(&self, set: vk::DescriptorSet, layout: DescLayout);
    /// `GL_SetObjectName`
    fn set_object_name(&self, object: u64, object_type: vk::ObjectType, name: &str);
    /// `GL_MemoryTypeFromProperties`
    fn memory_type_from_properties(
        &self,
        type_bits: u32,
        required: vk::MemoryPropertyFlags,
        preferred: vk::MemoryPropertyFlags,
    ) -> u32;
    /// `GL_WaitForDeviceIdle`
    fn wait_for_device_idle(&self);
    /// `R_StagingAllocate`
    fn staging_allocate(&self, size: i32, alignment: i32) -> Staging;
    /// `R_StagingBeginCopy`
    fn staging_begin_copy(&self);
    /// `R_StagingEndCopy`
    fn staging_end_copy(&self);
    /// `vkCmdPipelineBarrier` with one image barrier and nothing else.
    fn cmd_pipeline_barrier(
        &self,
        command_buffer: vk::CommandBuffer,
        src_stage: vk::PipelineStageFlags,
        dst_stage: vk::PipelineStageFlags,
        barrier: &vk::ImageMemoryBarrier<'_>,
    );
    fn cmd_copy_buffer_to_image(
        &self,
        command_buffer: vk::CommandBuffer,
        buffer: vk::Buffer,
        image: vk::Image,
        layout: vk::ImageLayout,
        regions: &[vk::BufferImageCopy],
    );
    /// `TexMgr_Downsample`: `stbir_resize_uint8` of the `in_width x
    /// in_height` RGBA8 image at `data` to `out_width x out_height`, written
    /// back in place. stb stays C (the engine's `TexMgr_Glue_Downsample`,
    /// the differential's `ctest_texmgr_downsample`).
    ///
    /// # Safety
    /// `data` holds `in_width * in_height` RGBA8 pixels and `1 <= out <= in`
    /// on both axes.
    unsafe fn downsample(
        &self,
        data: *mut u32,
        in_width: i32,
        in_height: i32,
        out_width: i32,
        out_height: i32,
    );
    /// `COM_FOpenFile (name)`, `Sys_fseek (offset, SEEK_CUR)` when `offset`
    /// is non-zero, `fread (size)`.
    fn read_source(&self, name: &CStr, offset: usize, size: usize) -> SourceRead;
    /// `Image_LoadImage (name, ..., path_id)`
    fn image_load(&self, name: &CStr, min_path_id: u32) -> Option<LoadedImage>;
    /// `Mem_Free` of an [`Image_LoadImage`](Self::image_load) buffer.
    ///
    /// # Safety
    /// `data` came from `image_load` and is freed once.
    unsafe fn free_loaded(&self, data: *mut u8);
    /// `owner->path_id` (`qmodel_t`); `owner` is non-null.
    ///
    /// # Safety
    /// `owner` is a live `qmodel_t`.
    unsafe fn owner_path_id(&self, owner: *mut c_void) -> u32;
    /// `Con_Printf ("%s", msg)`
    fn con_printf(&self, msg: &str);
    /// `Sys_Error ("%s", msg)`; never returns (ADR-009: it terminates, it
    /// does not longjmp through these frames).
    fn sys_error(&self, msg: &str) -> !;
}

/// The six `d_8to24table*` palettes, as references into wherever they live
/// (the engine's C arrays under `quake-capi`, a [`Palettes`] elsewhere).
#[derive(Clone, Copy)]
pub struct PaletteRefs<'a> {
    pub table: &'a [u32; 256],
    pub fbright: &'a [u32; 256],
    pub fbright_fence: &'a [u32; 256],
    pub nobright: &'a [u32; 256],
    pub nobright_fence: &'a [u32; 256],
    pub conchars: &'a [u32; 256],
}

/// `d_8to24table`, `_fbright`, `_fbright_fence`, `_nobright`,
/// `_nobright_fence`, `_conchars` (`gl_texmgr.c:63-68`), as
/// `TexMgr_LoadPalette` builds them from `gfx/palette.lmp`.
#[derive(Clone, Debug)]
pub struct Palettes {
    pub table: [u32; 256],
    pub fbright: [u32; 256],
    pub fbright_fence: [u32; 256],
    pub nobright: [u32; 256],
    pub nobright_fence: [u32; 256],
    pub conchars: [u32; 256],
}

impl Default for Palettes {
    fn default() -> Self {
        Self {
            table: [0; 256],
            fbright: [0; 256],
            fbright_fence: [0; 256],
            nobright: [0; 256],
            nobright_fence: [0; 256],
            conchars: [0; 256],
        }
    }
}

#[inline]
fn rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    u32::from_ne_bytes([r, g, b, a])
}

impl Palettes {
    /// `TexMgr_LoadPalette` after the file read: the six tables from the
    /// 768-byte lump.
    pub fn from_lump(pal: &[u8; 768]) -> Self {
        let mut p = Self::default();
        // standard palette, 255 is transparent
        for i in 0..256 {
            p.table[i] = rgba(pal[i * 3], pal[i * 3 + 1], pal[i * 3 + 2], 255);
        }
        p.table[255] = rgba(pal[765], pal[766], pal[767], 0);
        // fullbright palette, 0-223 are black (for additive blending)
        for i in 224..256 {
            p.fbright[i] = rgba(pal[i * 3], pal[i * 3 + 1], pal[i * 3 + 2], 255);
        }
        for i in 0..224 {
            p.fbright[i] = rgba(0, 0, 0, 255);
        }
        // nobright palette, 224-255 are black (for additive blending)
        for i in 0..256 {
            p.nobright[i] = rgba(pal[i * 3], pal[i * 3 + 1], pal[i * 3 + 2], 255);
        }
        for i in 224..256 {
            p.nobright[i] = rgba(0, 0, 0, 255);
        }
        // fence variants: index 255 fully transparent
        p.fbright_fence = p.fbright;
        p.fbright_fence[255] = 0;
        p.nobright_fence = p.nobright;
        p.nobright_fence[255] = 0;
        // conchars palette, 0 and 255 are transparent
        p.conchars = p.table;
        p.conchars[0] = u32::from_ne_bytes({
            let mut b = p.table[0].to_ne_bytes();
            b[3] = 0;
            b
        });
        p
    }

    pub fn refs(&self) -> PaletteRefs<'_> {
        PaletteRefs {
            table: &self.table,
            fbright: &self.fbright,
            fbright_fence: &self.fbright_fence,
            nobright: &self.nobright,
            nobright_fence: &self.nobright_fence,
            conchars: &self.conchars,
        }
    }
}

/// `TexMgr_LoadMiptexPalette`: a 24-bit `numcolors` palette to RGBA8 in
/// `out` (which holds at least `numcolors` entries) honouring the
/// fullbright/nobright/alpha flags. `gl_fullbrights` is the cvar value.
pub fn load_miptex_palette(
    input: &[u8],
    out: &mut [u32],
    numcolors: usize,
    flags: u32,
    gl_fullbrights: f32,
) {
    if numcolors == 0 {
        return;
    }
    let px = |i: usize| rgba(input[i * 3], input[i * 3 + 1], input[i * 3 + 2], 255);
    if flags & (TEXPREF_FULLBRIGHT | TEXPREF_NOBRIGHT) == 0 {
        for (i, o) in out.iter_mut().enumerate().take(numcolors) {
            *o = px(i);
        }
    } else {
        let mut numnobright = 224.min(numcolors);
        if flags & TEXPREF_NOBRIGHT != 0 {
            // nobright palette
            if gl_fullbrights == 0.0 {
                numnobright = numcolors;
            }
            for (i, o) in out.iter_mut().enumerate().take(numnobright) {
                *o = px(i);
            }
            // 224-255 are black (for additive blending)
            for o in out.iter_mut().take(numcolors).skip(numnobright) {
                *o = rgba(0, 0, 0, 255);
            }
        } else {
            // fullbright palette
            // 0-223 are black (for additive blending)
            for o in out.iter_mut().take(numnobright) {
                *o = rgba(0, 0, 0, 255);
            }
            for (i, o) in out.iter_mut().enumerate().take(numcolors).skip(numnobright) {
                *o = px(i);
            }
        }
    }
    // 255 is transparent (`out[-1] = 0`: the alpha byte of the last entry)
    if flags & TEXPREF_ALPHA != 0 {
        let mut b = out[numcolors - 1].to_ne_bytes();
        b[3] = 0;
        out[numcolors - 1] = u32::from_ne_bytes(b);
    }
}

/// `q_strlcpy` into a fixed C buffer: at most `dst.len() - 1` bytes, always
/// NUL-terminated.
fn strlcpy(dst: &mut [u8], src: &[u8]) {
    let n = src.len().min(dst.len() - 1);
    dst[..n].copy_from_slice(&src[..n]);
    dst[n] = 0;
}

fn cstr_bytes(buf: &[u8]) -> &[u8] {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    &buf[..end]
}

fn name_of(glt: &GlTexture) -> String {
    String::from_utf8_lossy(cstr_bytes(&glt.name)).into_owned()
}

/// `TexMgr_AlphaEdgeFix`: eliminate pink edges on sprites, etc. Operates in
/// place on 32-bit data.
pub fn alpha_edge_fix(data: &mut [u8], width: usize, height: usize) {
    let (mut n, mut c) = (0u32, [0u32; 3]);
    let mut dest = 0usize;
    for i in 0..height {
        let lastrow = width * 4 * if i == 0 { height - 1 } else { i - 1 };
        let thisrow = width * 4 * i;
        let nextrow = width * 4 * if i == height - 1 { 0 } else { i + 1 };
        for j in 0..width {
            let d = dest;
            dest += 4;
            if data[d + 3] != 0 {
                // not transparent
                continue;
            }
            let lastpix = 4 * if j == 0 { width - 1 } else { j - 1 };
            let thispix = 4 * j;
            let nextpix = 4 * if j == width - 1 { 0 } else { j + 1 };
            for b in [
                lastrow + lastpix,
                thisrow + lastpix,
                nextrow + lastpix,
                lastrow + thispix,
                nextrow + thispix,
                lastrow + nextpix,
                thisrow + nextpix,
                nextrow + nextpix,
            ] {
                if data[b + 3] != 0 {
                    c[0] += u32::from(data[b]);
                    c[1] += u32::from(data[b + 1]);
                    c[2] += u32::from(data[b + 2]);
                    n += 1;
                }
            }
            // average all non-transparent neighbors
            if let Some(nz) = core::num::NonZeroU32::new(n) {
                data[d] = (c[0] / nz) as u8;
                data[d + 1] = (c[1] / nz) as u8;
                data[d + 2] = (c[2] / nz) as u8;
                n = 0;
                c = [0; 3];
            }
        }
    }
}

/// `TexMgr_PreMultiply32`
pub fn premultiply32(data: &mut [u8]) {
    for px in data.chunks_exact_mut(4) {
        let a = i32::from(px[3]);
        px[0] = ((i32::from(px[0]) * a) >> 8) as u8;
        px[1] = ((i32::from(px[1]) * a) >> 8) as u8;
        px[2] = ((i32::from(px[2]) * a) >> 8) as u8;
    }
}

/// `TexMgr_DeriveNumMips`
pub fn derive_num_mips(mut width: i32, mut height: i32) -> i32 {
    let mut num_mips = 0;
    while width >= 1 && height >= 1 {
        width /= 2;
        height /= 2;
        num_mips += 1;
    }
    num_mips
}

/// `TexMgr_DeriveStagingSize`
pub fn derive_staging_size(mut width: i32, mut height: i32) -> i32 {
    let mut size = 0;
    while width >= 1 && height >= 1 {
        size += width * height * 4;
        width /= 2;
        height /= 2;
    }
    size
}

/// `texture_garbage_t`
#[derive(Clone, Copy)]
struct TextureGarbage {
    image: vk::Image,
    target_image_view: vk::ImageView,
    image_view: vk::ImageView,
    frame_buffer: vk::Framebuffer,
    descriptor_set: vk::DescriptorSet,
    storage_descriptor_set: vk::DescriptorSet,
    allocation: *mut c_void,
}

/// The per-texture state `TexMgr_LoadImage32` computes before it takes the
/// lock (the CPU half) and needs again after it releases it (the upload).
#[derive(Clone, Copy, Debug)]
pub struct Prep32 {
    pub mipwidth: i32,
    pub mipheight: i32,
    pub num_mips: i32,
    pub is_cube: bool,
    pub ten_bit: bool,
}

/// How an entry point written at C lock granularity reaches the manager:
/// each `with` is one `QMutex_Lock`/`QMutex_Unlock` pair of the C.
pub trait TexMgrLock<B: TexMgrBackend> {
    fn with<R>(&self, f: impl FnOnce(&mut TexMgr<B>) -> R) -> R;
}

/// A [`TexMgrLock`] over a manager the caller already holds exclusively
/// (inside one critical section, or single-threaded).
pub struct Direct<'a, B: TexMgrBackend>(RefCell<&'a mut TexMgr<B>>);

impl<'a, B: TexMgrBackend> Direct<'a, B> {
    pub fn new(texmgr: &'a mut TexMgr<B>) -> Self {
        Self(RefCell::new(texmgr))
    }
}

impl<B: TexMgrBackend> TexMgrLock<B> for Direct<'_, B> {
    fn with<R>(&self, f: impl FnOnce(&mut TexMgr<B>) -> R) -> R {
        let mut guard = self.0.borrow_mut();
        f(&mut **guard)
    }
}

/// The manager state: `active_gltextures`, `free_gltextures`,
/// `numgltextures`, `texmgr_heap`, the garbage ring.
pub struct TexMgr<B: TexMgrBackend> {
    /// The `MAX_GLTEXTURES` array `TexMgr_Init` allocates; every
    /// `gltexture_t *` C sees points into it. Held raw (`Box::into_raw`,
    /// freed in `drop`) so no `&mut` to the allocation is ever formed after
    /// the first texture pointer is derived from it: under Stacked and Tree
    /// Borrows a `Box` move or `&mut` through it would retag the allocation
    /// and invalidate every pointer C holds.
    arena: *mut [GlTexture],
    active: *mut GlTexture,
    free: *mut GlTexture,
    numgltextures: i32,
    heap: Option<Heap<B::HeapBackend>>,
    current_garbage_index: usize,
    garbage: [Vec<TextureGarbage>; 2],
}

// SAFETY: `arena` is a heap allocation owned by this value (freed once, in
// `drop`) and the other raw pointers point into it; the engine shim
// serialises access with its mutex exactly as the C does with
// `texmgr_mutex`.
unsafe impl<B: TexMgrBackend + Send> Send for TexMgr<B> where Heap<B::HeapBackend>: Send {}

impl<B: TexMgrBackend> Drop for TexMgr<B> {
    fn drop(&mut self) {
        self.release_arena();
    }
}

impl<B: TexMgrBackend> Default for TexMgr<B> {
    fn default() -> Self {
        Self::new()
    }
}

impl<B: TexMgrBackend> TexMgr<B> {
    /// The state before `TexMgr_Init`: no arena, no heap.
    pub fn new() -> Self {
        Self {
            arena: Self::empty_arena(),
            active: ptr::null_mut(),
            free: ptr::null_mut(),
            numgltextures: 0,
            heap: None,
            current_garbage_index: 0,
            garbage: [Vec::new(), Vec::new()],
        }
    }

    /// The "no arena" value: a zero-length slice that `release_arena` and
    /// `drop` leave alone.
    fn empty_arena() -> *mut [GlTexture] {
        ptr::slice_from_raw_parts_mut(ptr::NonNull::<GlTexture>::dangling().as_ptr(), 0)
    }

    /// Free the arena, if any. Every texture pointer derived from it (the
    /// list links, the pointers C holds) is dangling afterwards.
    fn release_arena(&mut self) {
        if self.arena.len() != 0 {
            // SAFETY: a non-empty `arena` came from `Box::into_raw` in
            // `init_list`, is released exactly once (here, then replaced),
            // and no reference into it exists: the manager only ever holds
            // raw pointers.
            unsafe { drop(Box::from_raw(self.arena)) };
            self.arena = Self::empty_arena();
        }
        self.active = ptr::null_mut();
        self.free = ptr::null_mut();
        self.numgltextures = 0;
    }

    /// `TexMgr_Init`'s list setup: allocate the arena and thread the free
    /// list through it. The links are written through the raw allocation,
    /// never through a `Box` or slice reference, so the pointers stay valid
    /// under Stacked and Tree Borrows (checked by the Miri smoke test).
    pub fn init_list(&mut self) {
        self.release_arena();
        let arena: *mut [GlTexture] =
            Box::into_raw(vec![GlTexture::ZEROED; MAX_GLTEXTURES].into_boxed_slice());
        let base = arena.cast::<GlTexture>();
        for i in 0..MAX_GLTEXTURES - 1 {
            // SAFETY: `i + 1 < MAX_GLTEXTURES`, inside the arena, and no
            // reference to the arena exists (`into_raw` consumed the `Box`).
            unsafe { (*base.add(i)).next = base.add(i + 1) };
        }
        // SAFETY: the last slot of the arena.
        unsafe { (*base.add(MAX_GLTEXTURES - 1)).next = ptr::null_mut() };
        self.arena = arena;
        self.free = base;
        self.active = ptr::null_mut();
        self.numgltextures = 0;
    }

    /// `numgltextures`
    pub fn num_textures(&self) -> i32 {
        self.numgltextures
    }

    /// `active_gltextures`
    pub fn active_head(&self) -> *mut GlTexture {
        self.active
    }

    /// The active list in order (`for (glt = active_gltextures; glt; glt = glt->next)`).
    pub fn active_textures(&self) -> impl Iterator<Item = *mut GlTexture> + '_ {
        let mut p = self.active;
        core::iter::from_fn(move || {
            if p.is_null() {
                None
            } else {
                let cur = p;
                // SAFETY: a non-null link is into the arena we own.
                p = unsafe { (*cur).next };
                Some(cur)
            }
        })
    }

    /// `TexMgr_FindTexture`
    pub fn find_texture(&self, owner: *mut c_void, name: Option<&CStr>) -> *mut GlTexture {
        let Some(name) = name else {
            return ptr::null_mut();
        };
        let name = name.to_bytes();
        for glt in self.active_textures() {
            // SAFETY: arena pointer.
            let t = unsafe { &*glt };
            if t.owner == owner && cstr_bytes(&t.name) == name {
                return glt;
            }
        }
        ptr::null_mut()
    }

    /// `TexMgr_NewTexture`: pop the free list, push the active list. Like the
    /// C, an exhausted free list is a null dereference (`MAX_GLTEXTURES`
    /// textures live at once).
    pub fn new_texture(&mut self) -> *mut GlTexture {
        let glt = self.free;
        assert!(!glt.is_null(), "TexMgr_NewTexture: out of textures");
        // SAFETY: arena pointer (non-null checked).
        unsafe {
            self.free = (*glt).next;
            (*glt).next = self.active;
        }
        self.active = glt;
        self.numgltextures += 1;
        glt
    }

    /// `TexMgr_FreeTexture`
    ///
    /// # Safety
    /// `kill` is null or a texture of this manager's arena.
    pub unsafe fn free_texture(&mut self, backend: &B, kill: *mut GlTexture) {
        if kill.is_null() {
            backend.con_printf("TexMgr_FreeTexture: NULL texture\n");
            return;
        }
        if self.active == kill {
            // SAFETY: arena pointer.
            unsafe {
                self.active = (*kill).next;
                (*kill).next = self.free;
            }
            self.free = kill;
            // SAFETY: per the contract.
            unsafe { self.delete_texture(backend, kill) };
            self.numgltextures -= 1;
            return;
        }
        let mut glt = self.active;
        while !glt.is_null() {
            // SAFETY: arena pointers.
            unsafe {
                if (*glt).next == kill {
                    (*glt).next = (*kill).next;
                    (*kill).next = self.free;
                    self.free = kill;
                    self.delete_texture(backend, kill);
                    self.numgltextures -= 1;
                    return;
                }
                glt = (*glt).next;
            }
        }
        backend.con_printf("TexMgr_FreeTexture: not found\n");
    }

    /// `TexMgr_FreeTextures`: compares each bit in `flags` to the one in
    /// `glt->flags` only if that bit is active in `mask`.
    pub fn free_textures(&mut self, backend: &B, flags: u32, mask: u32) {
        let mut glt = self.active;
        while !glt.is_null() {
            // SAFETY: arena pointer; `next` is read before the node moves lists.
            let (next, tflags) = unsafe { ((*glt).next, (*glt).flags) };
            if tflags & mask == flags & mask {
                // SAFETY: `glt` is on the active list.
                unsafe { self.free_texture(backend, glt) };
            }
            glt = next;
        }
    }

    /// `TexMgr_FreeTexturesForOwner`
    pub fn free_textures_for_owner(&mut self, backend: &B, owner: *mut c_void) {
        let mut glt = self.active;
        while !glt.is_null() {
            // SAFETY: as in `free_textures`.
            let (next, towner) = unsafe { ((*glt).next, (*glt).owner) };
            if towner == owner {
                // SAFETY: `glt` is on the active list.
                unsafe { self.free_texture(backend, glt) };
            }
            glt = next;
        }
    }

    /// `TexMgr_DeleteTextureObjects`
    pub fn delete_texture_objects(&mut self, backend: &B) {
        let list: Vec<_> = self.active_textures().collect();
        for glt in list {
            // SAFETY: `glt` is on the active list.
            unsafe { self.delete_texture(backend, glt) };
        }
    }

    /// `TexMgr_InitHeap`
    pub fn init_heap(&mut self, backend: &B) {
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .extent(vk::Extent3D {
                width: 1,
                height: 1,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(
                vk::ImageUsageFlags::COLOR_ATTACHMENT
                    | vk::ImageUsageFlags::SAMPLED
                    | vk::ImageUsageFlags::TRANSFER_SRC
                    | vk::ImageUsageFlags::TRANSFER_DST
                    | vk::ImageUsageFlags::STORAGE,
            )
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let dummy_image = match backend.create_image(&info) {
            Ok(image) => image,
            Err(err) => backend.sys_error(&format!("vkCreateImage failed with code {err}")),
        };
        let reqs = backend.image_memory_requirements(dummy_image);
        let memory_type_index = backend.memory_type_from_properties(
            reqs.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            vk::MemoryPropertyFlags::empty(),
        );
        let heap_memory_size = TEXTURE_HEAP_MEMORY_SIZE_MB * 1024 * 1024;
        self.heap = Some(Heap::new(
            backend.new_heap_backend(),
            heap_memory_size,
            TEXTURE_HEAP_PAGE_SIZE,
            memory_type_index,
            VulkanMemoryType::Device,
            false,
        ));
        backend.destroy_image(dummy_image);
    }

    fn heap(&mut self) -> &mut Heap<B::HeapBackend> {
        self.heap.as_mut().expect("TexMgr_InitHeap has not run")
    }

    /// `TexMgr_GetHeapStats`: `GL_HeapGetStats (texmgr_heap)`, null before
    /// `TexMgr_InitHeap` (where the C dereferences a null heap).
    ///
    /// # Safety
    /// `this` is a live manager.
    pub unsafe fn heap_stats_ptr(this: *mut Self) -> *mut GlHeapStats {
        // SAFETY: per the contract; the `&mut Option` is derived from `this`
        // and released before the stats pointer is handed out.
        match unsafe { &mut *ptr::addr_of_mut!((*this).heap) } {
            // SAFETY: the heap is live for as long as the manager is.
            Some(heap) => unsafe { Heap::stats_ptr(ptr::from_mut(heap)) },
            None => ptr::null_mut(),
        }
    }

    /// `TexMgr_LoadPalette`: the file read; the tables are
    /// [`Palettes::from_lump`].
    pub fn load_palette(backend: &B) -> Palettes {
        match backend.read_source(c"gfx/palette.lmp", 0, 768) {
            SourceRead::Ok(bytes) if bytes.len() == 768 => {
                let mut pal = [0u8; 768];
                pal.copy_from_slice(&bytes);
                Palettes::from_lump(&pal)
            }
            _ => backend.sys_error("Couldn't load gfx/palette.lmp"),
        }
    }

    /// `TexMgr_SetFilterModes`
    pub fn set_filter_modes(backend: &B, glt: &GlTexture) {
        let env = backend.env();
        let cvars = backend.cvars();
        let enable_anisotropy = cvars.vid_anisotropic != 0.0
            && (glt.flags & TEXPREF_NOPICMIP == 0 || glt.flags & TEXPREF_WARPIMAGE != 0);
        let point_sampler = if enable_anisotropy {
            env.point_aniso_sampler_lod_bias
        } else {
            env.point_sampler_lod_bias
        };
        let linear_sampler = if enable_anisotropy {
            env.linear_aniso_sampler_lod_bias
        } else {
            env.linear_sampler_lod_bias
        };
        let sampler = if glt.flags & TEXPREF_NEAREST != 0 {
            point_sampler
        } else if glt.flags & TEXPREF_LINEAR != 0 {
            linear_sampler
        } else if cvars.vid_filter == 1.0 {
            point_sampler
        } else {
            linear_sampler
        };
        backend.update_descriptor_set(&DescriptorWrite {
            set: glt.descriptor_set,
            binding: 0,
            descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
            sampler,
            image_view: glt.image_view,
            image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        });
    }

    /// `TexMgr_UpdateTextureDescriptorSets`
    pub fn update_texture_descriptor_sets(&self, backend: &B) {
        for glt in self.active_textures() {
            // SAFETY: arena pointer.
            Self::set_filter_modes(backend, unsafe { &*glt });
        }
    }

    /// `TexMgr_CollectGarbage`
    pub fn collect_garbage(&mut self, backend: &B) {
        self.current_garbage_index = (self.current_garbage_index + 1) % 2;
        let list = core::mem::take(&mut self.garbage[self.current_garbage_index]);
        for garbage in &list {
            if garbage.frame_buffer != vk::Framebuffer::null() {
                backend.destroy_framebuffer(garbage.frame_buffer);
            }
            if garbage.target_image_view != vk::ImageView::null() {
                backend.destroy_image_view(garbage.target_image_view);
            }
            backend.destroy_image_view(garbage.image_view);
            backend.destroy_image(garbage.image);
            backend.free_descriptor_set(garbage.descriptor_set, DescLayout::SingleTexture);
            if garbage.storage_descriptor_set != vk::DescriptorSet::null() {
                // COMPAT: gl_texmgr.c:1743 frees `garbage->descriptor_set`
                // (not `storage_descriptor_set`) against the cs-write layout;
                // kept verbatim so the descriptor free lists match the C
                // (Phase 8 task plan, M4 amendment).
                backend
                    .free_descriptor_set(garbage.descriptor_set, DescLayout::SingleTextureCsWrite);
            }
            self.free_allocation(backend, garbage.allocation);
        }
        // the C keeps the array and resets the count; the Vec keeps its capacity
        let mut list = list;
        list.clear();
        self.garbage[self.current_garbage_index] = list;
    }

    fn free_allocation(&mut self, backend: &B, allocation: *mut c_void) {
        let counter = backend.heap_counter();
        // SAFETY: `allocation` came from `Box::into_raw` in
        // `load_image32_create` and is reclaimed exactly once; the C
        // `GL_HeapFree` dereferences it the same way.
        let allocation = unsafe { *Box::from_raw(allocation.cast::<Allocation>()) };
        self.heap().free(allocation, counter);
    }

    /// `GL_DeleteTexture`
    ///
    /// # Safety
    /// `texture` is a texture of this manager's arena.
    pub unsafe fn delete_texture(&mut self, backend: &B, texture: *mut GlTexture) {
        // SAFETY: per the contract.
        let t = unsafe { &mut *texture };
        if t.image_view == vk::ImageView::null() {
            return;
        }
        if backend.in_update_screen() {
            self.garbage[self.current_garbage_index].push(TextureGarbage {
                image: t.image,
                target_image_view: t.target_image_view,
                image_view: t.image_view,
                frame_buffer: t.frame_buffer,
                descriptor_set: t.descriptor_set,
                storage_descriptor_set: t.storage_descriptor_set,
                allocation: t.allocation,
            });
        } else {
            backend.wait_for_device_idle();
            if t.frame_buffer != vk::Framebuffer::null() {
                backend.destroy_framebuffer(t.frame_buffer);
            }
            if t.target_image_view != vk::ImageView::null() {
                backend.destroy_image_view(t.target_image_view);
            }
            backend.destroy_image_view(t.image_view);
            backend.destroy_image(t.image);
            backend.free_descriptor_set(t.descriptor_set, DescLayout::SingleTexture);
            if t.storage_descriptor_set != vk::DescriptorSet::null() {
                backend.free_descriptor_set(
                    t.storage_descriptor_set,
                    DescLayout::SingleTextureCsWrite,
                );
            }
            let allocation = t.allocation;
            self.free_allocation(backend, allocation);
        }
        t.frame_buffer = vk::Framebuffer::null();
        t.target_image_view = vk::ImageView::null();
        t.image_view = vk::ImageView::null();
        t.image = vk::Image::null();
        t.allocation = ptr::null_mut();
    }

    /// `TexMgr_LoadImage32` before `QMutex_Lock`: premultiply, picmip /
    /// `gl_max_size` sizing, alpha detection, downsample + edge fix, mip count.
    ///
    /// # Safety
    /// `data` is null (warp images) or the texture's pixels: `width *
    /// height` RGBA8 for 2D formats, six such faces behind six pointers for
    /// `SRC_RGBA_CUBEMAP`. The pixels are 4-byte aligned (`unsigned *` in
    /// C) and are modified in place as the C does.
    pub unsafe fn load_image32_prepare(backend: &B, glt: &mut GlTexture, data: *mut u32) -> Prep32 {
        let env = backend.env();
        let cvars = backend.cvars();
        // For a cubemap `data` is the six-entry face pointer table, so the
        // C's `TEXPREF_PREMULTIPLY`/`TEXPREF_ALPHA` passes over `width *
        // height * 4` bytes would walk off it. No caller reaches that path
        // (`gl_sky.c` loads its cubemap with `TEXPREF_NONE`); it is skipped
        // rather than reproduced, since a slice past the table is UB here.
        let is_cube = glt.source_format == SrcFormat::RgbaCubemap as c_int;
        // do this before any rescaling
        if glt.flags & TEXPREF_PREMULTIPLY != 0 && !is_cube {
            let n = glt.width as usize * glt.height as usize * 4;
            // SAFETY: per the contract (a 2D RGBA8 buffer of that size).
            premultiply32(unsafe { core::slice::from_raw_parts_mut(data.cast::<u8>(), n) });
        }
        // mipmap down
        // COMPAT: C's `(int)gl_picmip.value` is undefined for out-of-range
        // floats and `width >> picmip` for `picmip >= 32`; Rust saturates the
        // cast and masks the shift count, which matches what x86-64/AArch64
        // builds of the C do in practice (ADR-010 per-platform parity).
        let picmip = if glt.flags & TEXPREF_NOPICMIP != 0 {
            0
        } else {
            (cvars.gl_picmip as i32).max(0)
        };
        let mut mipwidth = glt.width.wrapping_shr(picmip as u32).max(1) as i32;
        let mut mipheight = glt.height.wrapping_shr(picmip as u32).max(1) as i32;
        let mut maxsize = if is_cube {
            env.max_image_dimension_cube
        } else {
            env.max_image_dimension_2d
        } as i32;
        if glt.flags & TEXPREF_NOPICMIP == 0 && cvars.gl_max_size != 0.0 {
            maxsize = (cvars.gl_max_size as i32).max(1).min(maxsize);
        }
        if mipwidth > maxsize || mipheight > maxsize {
            if mipwidth >= mipheight {
                mipheight = ((mipheight * maxsize) / mipwidth).max(1);
                mipwidth = maxsize;
            } else {
                mipwidth = ((mipwidth * maxsize) / mipheight).max(1);
                mipheight = maxsize;
            }
        }
        // has alpha detection :
        if !data.is_null()
            && glt.source_format == SrcFormat::Rgba as c_int
            && glt.flags & TEXPREF_ALPHAPIXELS == 0
        {
            let num_pixels = glt.width as usize * glt.height as usize;
            // SAFETY: per the contract.
            let pixels = unsafe { core::slice::from_raw_parts(data.cast::<u8>(), num_pixels * 4) };
            if pixels.chunks_exact(4).any(|px| px[3] != 255) {
                glt.flags |= TEXPREF_ALPHA;
                glt.flags |= TEXPREF_ALPHAPIXELS;
            }
        }
        // downsizing according to gl_picmip / gl_max_size: (debug only)
        // don't attempt to downsize below 1x1
        if (glt.width as i32 != mipwidth || glt.height as i32 != mipheight)
            && mipwidth >= 1
            && mipheight >= 1
        {
            if is_cube {
                for i in 0..6 {
                    // SAFETY: per the contract, `data` is six face pointers.
                    let face = unsafe { *data.cast::<*mut u32>().add(i) };
                    // SAFETY: each face holds `width * height` pixels.
                    unsafe {
                        backend.downsample(
                            face,
                            glt.width as i32,
                            glt.height as i32,
                            mipwidth,
                            mipheight,
                        );
                    }
                }
            } else {
                // SAFETY: per the contract.
                unsafe {
                    backend.downsample(
                        data,
                        glt.width as i32,
                        glt.height as i32,
                        mipwidth,
                        mipheight,
                    );
                }
            }
            glt.width = mipwidth as u32;
            glt.height = mipheight as u32;
            if glt.flags & TEXPREF_ALPHA != 0 && !is_cube {
                let n = glt.width as usize * glt.height as usize * 4;
                // SAFETY: per the contract (a 2D RGBA8 buffer of that size;
                // the cubemap table is skipped above).
                alpha_edge_fix(
                    unsafe { core::slice::from_raw_parts_mut(data.cast::<u8>(), n) },
                    glt.width as usize,
                    glt.height as usize,
                );
            }
        }
        let num_mips = if glt.flags & TEXPREF_MIPMAP != 0 {
            derive_num_mips(glt.width as i32, glt.height as i32)
        } else {
            1
        };
        Prep32 {
            mipwidth,
            mipheight,
            num_mips,
            is_cube,
            ten_bit: false,
        }
    }

    /// `TexMgr_LoadImage32` between `QMutex_Lock` and `QMutex_Unlock`: the
    /// image, its memory, views, descriptor sets and (warp) framebuffer.
    pub fn load_image32_create(&mut self, backend: &B, glt: &mut GlTexture, prep: &mut Prep32) {
        let env = backend.env();
        let warp_image = glt.flags & TEXPREF_WARPIMAGE != 0;
        if warp_image {
            prep.num_mips = WARPIMAGEMIPS;
        }
        // Check for sanity. This should never be reached.
        if prep.num_mips as usize > MAX_MIPS {
            backend.sys_error(&format!("Texture has over {MAX_MIPS} mips"));
        }
        let lightmap = glt.source_format == SrcFormat::Lightmap as c_int;
        let surface_indices = glt.source_format == SrcFormat::SurfIndices as c_int;
        let ten_bit = lightmap && env.color_format == vk::Format::A2B10G10R10_UNORM_PACK32;
        prep.ten_bit = ten_bit;
        let is_cube = prep.is_cube;
        let num_mips = prep.num_mips as u32;
        let name = name_of(glt);

        let format = if surface_indices {
            vk::Format::R32_UINT
        } else if ten_bit {
            vk::Format::A2B10G10R10_UNORM_PACK32
        } else {
            vk::Format::R8G8B8A8_UNORM
        };

        let usage = if warp_image {
            vk::ImageUsageFlags::COLOR_ATTACHMENT
                | vk::ImageUsageFlags::SAMPLED
                | vk::ImageUsageFlags::TRANSFER_SRC
                | vk::ImageUsageFlags::TRANSFER_DST
                | vk::ImageUsageFlags::STORAGE
        } else if lightmap {
            vk::ImageUsageFlags::TRANSFER_DST
                | vk::ImageUsageFlags::SAMPLED
                | vk::ImageUsageFlags::STORAGE
        } else {
            vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED
        };
        let image_info = vk::ImageCreateInfo::default()
            .flags(if is_cube {
                vk::ImageCreateFlags::CUBE_COMPATIBLE
            } else {
                vk::ImageCreateFlags::empty()
            })
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D {
                width: glt.width,
                height: glt.height,
                depth: 1,
            })
            .mip_levels(num_mips)
            .array_layers(if is_cube { 6 } else { 1 })
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        glt.image = match backend.create_image(&image_info) {
            Ok(image) => image,
            Err(err) => backend.sys_error(&format!("vkCreateImage failed with code {err}")),
        };
        backend.set_object_name(
            glt.image.as_raw(),
            vk::ObjectType::IMAGE,
            &format!("{name} image"),
        );

        let reqs = backend.image_memory_requirements(glt.image);
        let counter = backend.heap_counter();
        // The C asserts are `assert ()`; see `GL_HeapAllocate` in quake-capi.
        assert!(reqs.size > 0);
        assert!(reqs.alignment > 0);
        let allocation = match self.heap().allocate(reqs.size, reqs.alignment, counter) {
            Some(allocation) => allocation,
            None => backend.sys_error("GL_HeapAllocate failed to allocate"),
        };
        let (memory, offset) = (allocation.memory(), allocation.offset());
        glt.allocation = Box::into_raw(Box::new(allocation)).cast();
        if let Err(err) = backend.bind_image_memory(glt.image, memory, offset) {
            backend.sys_error(&format!("vkBindImageMemory failed with code {err}"));
        }

        let mut view_info = vk::ImageViewCreateInfo::default()
            .image(glt.image)
            .view_type(if is_cube {
                vk::ImageViewType::CUBE
            } else {
                vk::ImageViewType::TYPE_2D
            })
            .format(format)
            .components(vk::ComponentMapping {
                r: vk::ComponentSwizzle::R,
                g: vk::ComponentSwizzle::G,
                b: vk::ComponentSwizzle::B,
                a: vk::ComponentSwizzle::A,
            })
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: num_mips,
                base_array_layer: 0,
                layer_count: if is_cube { 6 } else { 1 },
            });
        glt.image_view = match backend.create_image_view(&view_info) {
            Ok(view) => view,
            Err(err) => backend.sys_error(&format!("vkCreateImageView failed with code {err}")),
        };
        backend.set_object_name(
            glt.image_view.as_raw(),
            vk::ObjectType::IMAGE_VIEW,
            &format!("{name} image view"),
        );

        // Allocate and update descriptor for this texture
        glt.descriptor_set = backend.allocate_descriptor_set(DescLayout::SingleTexture);
        backend.set_object_name(
            glt.descriptor_set.as_raw(),
            vk::ObjectType::DESCRIPTOR_SET,
            &format!("{name} desc set"),
        );

        Self::set_filter_modes(backend, glt);

        if warp_image || lightmap {
            view_info.subresource_range.level_count = 1;
            glt.target_image_view = match backend.create_image_view(&view_info) {
                Ok(view) => view,
                Err(err) => backend.sys_error(&format!("vkCreateImageView failed with code {err}")),
            };
            backend.set_object_name(
                glt.target_image_view.as_raw(),
                vk::ObjectType::IMAGE_VIEW,
                &format!("{name} target image view"),
            );
        } else {
            glt.target_image_view = vk::ImageView::null();
        }

        // Don't upload data for warp image, will be updated by rendering
        if warp_image {
            let attachments = [glt.target_image_view];
            let fb_info = vk::FramebufferCreateInfo::default()
                .render_pass(env.warp_render_pass)
                .attachments(&attachments)
                .width(glt.width)
                .height(glt.height)
                .layers(1);
            glt.frame_buffer = match backend.create_framebuffer(&fb_info) {
                Ok(fb) => fb,
                Err(err) => {
                    backend.sys_error(&format!("vkCreateFramebuffer failed with code {err}"))
                }
            };
            backend.set_object_name(
                glt.frame_buffer.as_raw(),
                vk::ObjectType::FRAMEBUFFER,
                &format!("{name} framebuffer"),
            );
        }

        if warp_image {
            // Allocate and update descriptor for this texture
            glt.storage_descriptor_set =
                backend.allocate_descriptor_set(DescLayout::SingleTextureCsWrite);
            backend.set_object_name(
                glt.storage_descriptor_set.as_raw(),
                vk::ObjectType::DESCRIPTOR_SET,
                &format!("{name} storage desc set"),
            );
            backend.update_descriptor_set(&DescriptorWrite {
                set: glt.storage_descriptor_set,
                binding: 0,
                descriptor_type: vk::DescriptorType::STORAGE_IMAGE,
                sampler: vk::Sampler::null(),
                image_view: glt.target_image_view,
                image_layout: vk::ImageLayout::GENERAL,
            });
        } else {
            glt.storage_descriptor_set = vk::DescriptorSet::null();
        }
    }

    /// `TexMgr_LoadImage32` after `QMutex_Unlock`: the staging upload.
    ///
    /// # Safety
    /// As [`load_image32_prepare`](Self::load_image32_prepare), for the
    /// texture's current (post-downsample) dimensions; non-null unless the
    /// texture is a warp image.
    pub unsafe fn load_image32_upload(
        backend: &B,
        glt: &mut GlTexture,
        data: *mut u32,
        prep: &Prep32,
    ) {
        // Don't upload data for warp image, will be updated by rendering
        if glt.flags & TEXPREF_WARPIMAGE != 0 {
            return;
        }
        glt.frame_buffer = vk::Framebuffer::null();

        let Prep32 {
            mut mipwidth,
            mut mipheight,
            num_mips,
            is_cube,
            ten_bit,
        } = *prep;
        let layers = if is_cube { 6 } else { 1 };
        let mut regions = [vk::BufferImageCopy::default(); MAX_MIPS];

        let mut staging_size = if glt.flags & TEXPREF_MIPMAP != 0 {
            derive_staging_size(mipwidth, mipheight)
        } else {
            mipwidth * mipheight * 4
        };
        if is_cube {
            staging_size *= 6;
        }
        let staging = backend.staging_allocate(staging_size, 4);
        let staging_offset = staging.offset;

        let mut num_regions = 0usize;
        if glt.flags & TEXPREF_MIPMAP != 0 {
            let mut mip_offset = 0i32;
            mipwidth = glt.width as i32;
            mipheight = glt.height as i32;
            while mipwidth >= 1 && mipheight >= 1 {
                let r = &mut regions[num_regions];
                r.buffer_offset = (staging_offset + mip_offset) as u64;
                r.image_subresource.aspect_mask = vk::ImageAspectFlags::COLOR;
                r.image_subresource.layer_count = 1;
                r.image_subresource.mip_level = num_regions as u32;
                r.image_extent = vk::Extent3D {
                    width: mipwidth as u32,
                    height: mipheight as u32,
                    depth: 1,
                };
                mip_offset += mipwidth * mipheight * 4;
                num_regions += 1;
                mipwidth /= 2;
                mipheight /= 2;
            }
        } else if is_cube {
            for (i, r) in regions.iter_mut().enumerate().take(6) {
                r.buffer_offset = (staging_offset + (i as i32 * mipwidth * mipheight * 4)) as u64;
                r.image_subresource.aspect_mask = vk::ImageAspectFlags::COLOR;
                r.image_subresource.layer_count = 1;
                r.image_subresource.base_array_layer = i as u32;
                r.image_subresource.mip_level = 0;
                r.image_extent = vk::Extent3D {
                    width: mipwidth as u32,
                    height: mipheight as u32,
                    depth: 1,
                };
            }
        } else {
            let r = &mut regions[0];
            r.buffer_offset = staging_offset as u64;
            r.image_subresource.aspect_mask = vk::ImageAspectFlags::COLOR;
            r.image_subresource.layer_count = 1;
            r.image_subresource.mip_level = 0;
            r.image_extent = vk::Extent3D {
                width: mipwidth as u32,
                height: mipheight as u32,
                depth: 1,
            };
        }

        let mut barrier = vk::ImageMemoryBarrier::default()
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(glt.image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: num_mips as u32,
                base_array_layer: 0,
                layer_count: layers,
            })
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE);
        backend.cmd_pipeline_barrier(
            staging.command_buffer,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            &barrier,
        );

        // gl_texmgr.c:1305 passes `num_mips * 6` regions from a MAX_MIPS
        // array for a mipmapped cubemap, reading past it; no caller loads a
        // cubemap with TEXPREF_MIPMAP (gl_sky.c:499), so clamping is the
        // only in-bounds reading of the same call.
        let num_copy_regions = num_mips as usize * layers as usize;
        backend.cmd_copy_buffer_to_image(
            staging.command_buffer,
            staging.buffer,
            glt.image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &regions[..num_copy_regions.min(MAX_MIPS)],
        );

        barrier.src_access_mask = vk::AccessFlags::TRANSFER_WRITE;
        barrier.dst_access_mask = vk::AccessFlags::SHADER_READ;
        barrier.old_layout = vk::ImageLayout::TRANSFER_DST_OPTIMAL;
        barrier.new_layout = vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL;
        backend.cmd_pipeline_barrier(
            staging.command_buffer,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            &barrier,
        );

        backend.staging_begin_copy();
        let dst = staging.memory;
        if glt.flags & TEXPREF_MIPMAP != 0 {
            let mut mip_offset = 0i32;
            mipwidth = glt.width as i32;
            mipheight = glt.height as i32;
            while mipwidth >= 1 && mipheight >= 1 {
                let n = (mipwidth * mipheight * 4) as usize;
                // SAFETY: the staging block holds `staging_size` bytes (the
                // sum of every mip); `data` holds the current mip (contract).
                unsafe {
                    ptr::copy_nonoverlapping(data.cast::<u8>(), dst.add(mip_offset as usize), n)
                };
                mip_offset += mipwidth * mipheight * 4;
                if mipwidth > 1 && mipheight > 1 {
                    // SAFETY: per the contract; the C downsamples in place.
                    unsafe {
                        backend.downsample(data, mipwidth, mipheight, mipwidth / 2, mipheight / 2)
                    };
                }
                mipwidth /= 2;
                mipheight /= 2;
            }
        } else if is_cube {
            const REORDER: [usize; 6] = [3, 1, 4, 5, 0, 2];
            let face_size = (staging_size / 6) as usize;
            for (i, &src_face) in REORDER.iter().enumerate() {
                // SAFETY: per the contract, six face pointers of `face_size` bytes.
                unsafe {
                    let face = *data.cast::<*mut u32>().add(src_face);
                    ptr::copy_nonoverlapping(face.cast::<u8>(), dst.add(i * face_size), face_size);
                }
            }
        } else {
            let n = staging_size as usize;
            if !ten_bit {
                // SAFETY: per the contract.
                unsafe { ptr::copy_nonoverlapping(data.cast::<u8>(), dst, n) };
            } else {
                // SAFETY: per the contract; `n` is a multiple of 4.
                let src = unsafe { core::slice::from_raw_parts(data.cast::<u8>(), n) };
                for (i, p) in src.chunks_exact(4).enumerate() {
                    let packed = u32::from(p[0]) | u32::from(p[1]) << 10 | u32::from(p[2]) << 20;
                    // SAFETY: the staging block holds `n` bytes; unaligned
                    // like the C's `*(unsigned *)` store.
                    unsafe { dst.add(i * 4).cast::<u32>().write_unaligned(packed) };
                }
            }
        }
        backend.staging_end_copy();
    }

    /// `TexMgr_LoadImage8` after its `GL_DeleteTexture`: the `shot1sid`
    /// patch, false-alpha detection, palette choice and the 8-to-32
    /// conversion (+ edge fix). Returns the converted pixels for
    /// `load_image32_*`.
    ///
    /// # Safety
    /// `data` holds `width * height` indices and is written by the
    /// `shot1sid` patch as the C does.
    pub unsafe fn load_image8_convert(
        backend: &B,
        pal: &PaletteRefs<'_>,
        glt: &mut GlTexture,
        data: *mut u8,
        usepal: Option<&[u32]>,
    ) -> Vec<u32> {
        let cvars = backend.cvars();
        let pixels = glt.width as usize * glt.height as usize;
        // SAFETY: per the contract.
        let data = unsafe { core::slice::from_raw_parts_mut(data, pixels) };

        // HACK HACK HACK -- taken from tomazquake
        if name_of(glt).contains("shot1sid")
            && glt.width == 32
            && glt.height == 32
            && quake_util::crc::crc_block(&data[..1024]) == 65393
        {
            // This texture in b_shell1.bsp has some of the first 32 pixels painted white.
            // They are invisible in software, but look really ugly in GL. So we just copy
            // 32 pixels from the bottom to make it look nice.
            data.copy_within(32 * 31..32 * 32, 0);
        }

        // detect false alpha cases
        if glt.flags & TEXPREF_ALPHA != 0
            && glt.flags & TEXPREF_CONCHARS == 0
            && !data.contains(&255)
        {
            glt.flags -= TEXPREF_ALPHA;
        }

        // choose palette and padbyte
        let usepal: &[u32] = match usepal {
            Some(p) => p,
            None => {
                if glt.flags & TEXPREF_FULLBRIGHT != 0 {
                    if glt.flags & TEXPREF_ALPHA != 0 {
                        pal.fbright_fence
                    } else {
                        pal.fbright
                    }
                } else if glt.flags & TEXPREF_NOBRIGHT != 0 && cvars.gl_fullbrights != 0.0 {
                    if glt.flags & TEXPREF_ALPHA != 0 {
                        pal.nobright_fence
                    } else {
                        pal.nobright
                    }
                } else if glt.flags & TEXPREF_CONCHARS != 0 {
                    pal.conchars
                } else {
                    pal.table
                }
            }
        };

        // convert to 32bit (`TexMgr_8to32`)
        // COMPAT: a Valve miptex whose indices exceed its own palette count
        // reads past the palette in C (garbage colours, the map still loads);
        // that read is out of bounds here, so such an index degrades to
        // transparent black instead of aborting on mod content.
        let mut converted: Vec<u32> = data
            .iter()
            .map(|&i| usepal.get(usize::from(i)).copied().unwrap_or(0))
            .collect();

        // fix edges
        if glt.flags & TEXPREF_ALPHA != 0 {
            let bytes = converted.as_mut_ptr().cast::<u8>();
            // SAFETY: `converted` holds `pixels` u32s, viewed as bytes in place.
            alpha_edge_fix(
                unsafe { core::slice::from_raw_parts_mut(bytes, pixels * 4) },
                glt.width as usize,
                glt.height as usize,
            );
        }
        converted
    }

    /// `TexMgr_LoadImage8Valve`: the embedded palette after the mip data.
    ///
    /// # Safety
    /// `data` is a Valve miptex: `source_width * source_height` indices,
    /// followed at `source_width * source_height / 64 * 85` by a little-endian
    /// colour count and that many RGB triples.
    unsafe fn load_image8_valve_palette(backend: &B, glt: &GlTexture, data: *mut u8) -> Vec<u32> {
        let at = (glt.source_width as usize * glt.source_height as usize / 64) * 85;
        // SAFETY: per the contract.
        let (colors, input) = unsafe {
            let p = data.add(at);
            let colors = u16::from_le_bytes([*p, *p.add(1)]) as usize;
            (colors, core::slice::from_raw_parts(p.add(2), colors * 3))
        };
        let mut usepal = vec![0u32; colors];
        load_miptex_palette(
            input,
            &mut usepal,
            colors,
            glt.flags,
            backend.cvars().gl_fullbrights,
        );
        usepal
    }

    /// The `switch (glt->source_format)` upload shared by `TexMgr_LoadImage`
    /// and `TexMgr_ReloadImage`, at C lock granularity over `lock`.
    ///
    /// # Safety
    /// `data` matches `glt->source_format` and the texture's dimensions (see
    /// the `load_image*` contracts).
    unsafe fn upload_with<L: TexMgrLock<B>>(
        lock: &L,
        backend: &B,
        pal: &PaletteRefs<'_>,
        glt: *mut GlTexture,
        data: *mut u8,
    ) {
        // No `&mut GlTexture` is held across `lock.with`: `delete_texture`
        // writes the same texture through its own pointer, which would
        // invalidate an earlier reference under Stacked/Tree Borrows. The C
        // writes the fields unlocked too.
        // SAFETY: arena pointer (contract).
        let source_format = unsafe { (*glt).source_format };
        match SrcFormat::from_raw(source_format) {
            Some(SrcFormat::Indexed) => {
                // SAFETY: `glt` is an arena texture (contract).
                lock.with(|m| unsafe { m.delete_texture(backend, glt) });
                // SAFETY: arena pointer; nothing else references it now.
                let t = unsafe { &mut *glt };
                // SAFETY: per the contract.
                let mut converted =
                    unsafe { Self::load_image8_convert(backend, pal, t, data, None) };
                // SAFETY: `converted` is the texture's RGBA8 pixels.
                unsafe { Self::load_image32_with(lock, backend, glt, converted.as_mut_ptr()) };
            }
            Some(SrcFormat::IndexedPalette) => {
                // SAFETY: per the contract; the shared reference ends before
                // the delete below.
                let usepal = unsafe { Self::load_image8_valve_palette(backend, &*glt, data) };
                // SAFETY: `glt` is an arena texture (contract).
                lock.with(|m| unsafe { m.delete_texture(backend, glt) });
                // SAFETY: arena pointer; nothing else references it now.
                let t = unsafe { &mut *glt };
                // SAFETY: per the contract.
                let mut converted =
                    unsafe { Self::load_image8_convert(backend, pal, t, data, Some(&usepal)) };
                // SAFETY: as above.
                unsafe { Self::load_image32_with(lock, backend, glt, converted.as_mut_ptr()) };
            }
            Some(SrcFormat::Lightmap)
            | Some(SrcFormat::Rgba)
            | Some(SrcFormat::SurfIndices)
            | Some(SrcFormat::RgbaCubemap) => {
                // SAFETY: per the contract.
                unsafe { Self::load_image32_with(lock, backend, glt, data.cast()) };
            }
            None => {}
        }
    }

    /// `TexMgr_LoadImage32` at C lock granularity.
    ///
    /// # Safety
    /// See [`load_image32_prepare`](Self::load_image32_prepare).
    pub unsafe fn load_image32_with<L: TexMgrLock<B>>(
        lock: &L,
        backend: &B,
        glt: *mut GlTexture,
        data: *mut u32,
    ) {
        // SAFETY: `glt` is an arena texture (contract).
        lock.with(|m| unsafe { m.delete_texture(backend, glt) });
        // SAFETY: arena pointer, derived after the delete (which wrote the
        // texture through its own pointer); `load_image32_create` touches
        // the texture only through this reborrow, so it stays valid across
        // the `&mut TexMgr` (the arena is held raw, not as a `Box`).
        let t = unsafe { &mut *glt };
        // SAFETY: per the contract.
        let mut prep = unsafe { Self::load_image32_prepare(backend, t, data) };
        lock.with(|m| m.load_image32_create(backend, t, &mut prep));
        // SAFETY: per the contract.
        unsafe { Self::load_image32_upload(backend, t, data, &prep) };
    }

    /// `TexMgr_LoadImage` -- the one entry point for loading all textures --
    /// at C lock granularity.
    ///
    /// # Safety
    /// `data` matches `format` and `width x height`: indices for
    /// `SRC_INDEXED`/`SRC_INDEXED_PALETTE` (the latter a whole Valve miptex),
    /// RGBA8 for `SRC_RGBA`/`SRC_LIGHTMAP`, `R32` for `SRC_SURF_INDICES`, six
    /// face pointers for `SRC_RGBA_CUBEMAP`; null only for warp images. The
    /// 32-bit formats are 4-byte aligned (the C reads them as `unsigned *`).
    /// `owner` is null or a live `qmodel_t`.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn load_image_with<L: TexMgrLock<B>>(
        lock: &L,
        backend: &B,
        pal: &PaletteRefs<'_>,
        owner: *mut c_void,
        name: &CStr,
        width: i32,
        height: i32,
        format: c_int,
        data: *mut u8,
        source_file: &CStr,
        source_offset: usize,
        flags: u32,
    ) -> *mut GlTexture {
        if backend.no_rendering() {
            return ptr::null_mut();
        }
        // cache check
        let mut crc: u16 = 0;
        if flags & TEXPREF_OVERWRITE != 0 {
            let n = match SrcFormat::from_raw(format) {
                Some(SrcFormat::Indexed) | Some(SrcFormat::IndexedPalette) => {
                    Some(width as usize * height as usize)
                }
                Some(SrcFormat::Lightmap) => {
                    Some(width as usize * height as usize * LIGHTMAP_BYTES)
                }
                Some(SrcFormat::Rgba) => Some(width as usize * height as usize * 4),
                _ => None, /* not reachable but avoids compiler warnings */
            };
            if let Some(n) = n {
                // SAFETY: per the contract, `data` holds at least `n` bytes.
                crc = quake_util::crc::crc_block(unsafe { core::slice::from_raw_parts(data, n) });
            }
        }
        let glt = if flags & TEXPREF_OVERWRITE != 0 {
            let found = lock.with(|m| m.find_texture(owner, Some(name)));
            if found.is_null() {
                lock.with(|m| m.new_texture())
            } else {
                // SAFETY: arena pointer.
                if unsafe { (*found).source_crc } == crc {
                    return found;
                }
                found
            }
        } else {
            lock.with(|m| m.new_texture())
        };

        // copy data
        // SAFETY: arena pointer; the C writes these unlocked as well.
        let t = unsafe { &mut *glt };
        t.owner = owner;
        strlcpy(&mut t.name, name.to_bytes());
        t.path_id = if owner.is_null() {
            0
        } else {
            // SAFETY: per the contract.
            unsafe { backend.owner_path_id(owner) }
        };
        t.width = width as u32;
        t.height = height as u32;
        t.flags = flags;
        t.shirt = -1;
        t.pants = -1;
        strlcpy(&mut t.source_file, source_file.to_bytes());
        t.source_offset = source_offset;
        t.source_format = format;
        t.source_width = width as u32;
        t.source_height = height as u32;
        t.source_crc = crc;

        // upload it
        // SAFETY: per the contract.
        unsafe { Self::upload_with(lock, backend, pal, glt, data) };
        glt
    }

    /// [`load_image_with`](Self::load_image_with) inside one critical section.
    ///
    /// # Safety
    /// As `load_image_with`.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn load_image(
        &mut self,
        backend: &B,
        pal: &PaletteRefs<'_>,
        owner: *mut c_void,
        name: &CStr,
        width: i32,
        height: i32,
        format: c_int,
        data: *mut u8,
        source_file: &CStr,
        source_offset: usize,
        flags: u32,
    ) -> *mut GlTexture {
        // SAFETY: per the contract.
        unsafe {
            Self::load_image_with(
                &Direct::new(self),
                backend,
                pal,
                owner,
                name,
                width,
                height,
                format,
                data,
                source_file,
                source_offset,
                flags,
            )
        }
    }

    /// `TexMgr_ReloadImage` -- reloads a texture, and colormaps it if needed
    /// -- at C lock granularity.
    ///
    /// # Safety
    /// `glt` is an arena texture; a memory source (`source_offset` with no
    /// `source_file`) still points at its pixels.
    pub unsafe fn reload_image_with<L: TexMgrLock<B>>(
        lock: &L,
        backend: &B,
        pal: &PaletteRefs<'_>,
        glt: *mut GlTexture,
        shirt: i32,
        pants: i32,
    ) {
        // SAFETY: arena pointer.
        let t = unsafe { &mut *glt };
        let has_file = t.source_file[0] != 0;
        let name = name_of(t);
        let invalid =
            || backend.con_printf(&format!("TexMgr_ReloadImage: invalid source for {name}\n"));

        //
        // get source data
        //
        let mut owned: Option<Vec<u8>> = None;
        let mut loaded: Option<LoadedImage> = None;
        let data: *mut u8;
        if has_file && t.source_offset != 0 {
            // lump inside file
            let mut size = t.source_width as usize * t.source_height as usize;
            /* should be SRC_INDEXED, but no harm being paranoid:  */
            if t.source_format == SrcFormat::Rgba as c_int {
                size *= 4;
            } else if t.source_format == SrcFormat::Lightmap as c_int {
                size *= LIGHTMAP_BYTES;
            }
            let file = CStr::from_bytes_until_nul(&t.source_file).unwrap_or(c"");
            match backend.read_source(file, t.source_offset, size) {
                SourceRead::Ok(bytes) if bytes.len() == size => {
                    owned = Some(bytes);
                    data = owned.as_mut().map_or(ptr::null_mut(), |v| v.as_mut_ptr());
                }
                _ => {
                    invalid();
                    return;
                }
            }
        } else if has_file {
            let file = CStr::from_bytes_until_nul(&t.source_file).unwrap_or(c"");
            match backend.image_load(file, t.path_id) {
                Some(img) => {
                    // simple file
                    t.source_width = img.width as u32;
                    t.source_height = img.height as u32;
                    t.source_format = img.format;
                    loaded = Some(img);
                    data = img.data;
                }
                None => {
                    invalid();
                    return;
                }
            }
        } else if t.source_offset != 0 {
            data = t.source_offset as *mut u8; // image in memory
        } else {
            invalid();
            return;
        }
        if data.is_null() {
            invalid();
            return;
        }

        t.width = t.source_width;
        t.height = t.source_height;
        //
        // apply shirt and pants colors
        //
        // if shirt and pants are -1,-1, use existing shirt and pants colors
        // if existing shirt and pants colors are -1,-1, don't bother colormapping
        if shirt > -1 && pants > -1 {
            if t.source_format == SrcFormat::Indexed as c_int {
                t.shirt = shirt as i8;
                t.pants = pants as i8;
            } else {
                backend.con_printf(&format!(
                    "TexMgr_ReloadImage: can't colormap a non SRC_INDEXED texture: {name}\n"
                ));
            }
        }
        let mut translated: Option<Vec<u8>> = None;
        let mut data = data;
        if t.shirt > -1 && t.pants > -1 {
            // create new translation table
            let mut translation = [0u8; 256];
            for (i, tr) in translation.iter_mut().enumerate() {
                *tr = i as u8;
            }
            let shirt = i32::from(t.shirt) * 16;
            for i in 0..16 {
                translation[TOP_RANGE + i] = (if shirt < 128 {
                    shirt + i as i32
                } else {
                    shirt + 15 - i as i32
                }) as u8;
            }
            let pants = i32::from(t.pants) * 16;
            for i in 0..16 {
                translation[BOTTOM_RANGE + i] = (if pants < 128 {
                    pants + i as i32
                } else {
                    pants + 15 - i as i32
                }) as u8;
            }
            // translate texture
            let size = t.width as usize * t.height as usize;
            // SAFETY: per the contract, the source holds `width * height` indices.
            let src = unsafe { core::slice::from_raw_parts(data, size) };
            let mut out: Vec<u8> = src.iter().map(|&i| translation[usize::from(i)]).collect();
            data = out.as_mut_ptr();
            translated = Some(out);
        }
        //
        // upload it
        //
        // SAFETY: per the contract.
        unsafe { Self::upload_with(lock, backend, pal, glt, data) };

        drop(translated);
        drop(owned);
        if let Some(img) = loaded {
            // SAFETY: `img.data` came from `image_load`, freed once (the C `Mem_Free (allocated)`).
            unsafe { backend.free_loaded(img.data) };
        }
    }

    /// [`reload_image_with`](Self::reload_image_with) inside one critical section.
    ///
    /// # Safety
    /// As `reload_image_with`.
    pub unsafe fn reload_image(
        &mut self,
        backend: &B,
        pal: &PaletteRefs<'_>,
        glt: *mut GlTexture,
        shirt: i32,
        pants: i32,
    ) {
        // SAFETY: per the contract.
        unsafe { Self::reload_image_with(&Direct::new(self), backend, pal, glt, shirt, pants) }
    }

    /// `TexMgr_ReloadNobrightImages`: reloads all textures that were loaded
    /// with the nobright palette. Called when `gl_fullbrights` changes.
    pub fn reload_nobright_images(&mut self, backend: &B, pal: &PaletteRefs<'_>) {
        let list: Vec<_> = self.active_textures().collect();
        for glt in list {
            // SAFETY: arena pointer.
            if unsafe { (*glt).flags } & TEXPREF_NOBRIGHT != 0 {
                // SAFETY: a texture on the active list keeps its source valid.
                unsafe { self.reload_image(backend, pal, glt, -1, -1) };
            }
        }
    }

    /// `TexMgr_FreeTextures (0, TEXPREF_PERSIST)` -- the texture half of
    /// `TexMgr_NewGame` (the palette reload is the caller's, since the
    /// palettes live outside the manager).
    pub fn new_game_free(&mut self, backend: &B) {
        self.free_textures(backend, 0, TEXPREF_PERSIST);
    }
}

/// The six textures `TexMgr_Init` loads.
#[derive(Clone, Copy, Debug)]
pub struct BuiltinTextures {
    pub notexture: *mut GlTexture,
    pub nulltexture: *mut GlTexture,
    pub whitetexture: *mut GlTexture,
    pub greytexture: *mut GlTexture,
    pub greylightmap: *mut GlTexture,
    pub bluenoisetexture: *mut GlTexture,
}

/// black and pink checker
pub static mut NOTEXTURE_DATA: [u8; 16] = [
    159, 91, 83, 255, 0, 0, 0, 255, 0, 0, 0, 255, 159, 91, 83, 255,
];
/// black and blue checker
pub static mut NULLTEXTURE_DATA: [u8; 16] = [
    127, 191, 255, 255, 0, 0, 0, 255, 0, 0, 0, 255, 127, 191, 255, 255,
];
/// white
pub static mut WHITETEXTURE_DATA: [u8; 16] = [255; 16];
/// 50% grey
pub static mut GREYTEXTURE_DATA: [u8; 16] = [
    127, 127, 127, 255, 127, 127, 127, 255, 127, 127, 127, 255, 127, 127, 127, 255,
];

impl<B: TexMgrBackend> TexMgr<B> {
    /// The texture loads of `TexMgr_Init` (after the list, palette, cvar and
    /// command setup): the four 2x2 textures, `greylightmap` and the
    /// blue-noise tile. The static data arrays are the `source_offset` of
    /// the memory-sourced textures, as in the C (`bluenoise` records
    /// `greytexture_data`, a C quirk kept as-is). The C builds the RGBA
    /// blue-noise tile in a `TEMP_ALLOC` buffer whose alpha bytes it never
    /// writes (uninitialised stack on the alloca path); here they are zero.
    pub fn load_builtin_textures(&mut self, backend: &B, pal: &PaletteRefs<'_>) -> BuiltinTextures {
        const FLAGS: u32 = TEXPREF_NEAREST | TEXPREF_PERSIST | TEXPREF_NOPICMIP;
        let rgba = SrcFormat::Rgba as c_int;
        let notexture_data = ptr::addr_of_mut!(NOTEXTURE_DATA).cast::<u8>();
        let nulltexture_data = ptr::addr_of_mut!(NULLTEXTURE_DATA).cast::<u8>();
        let whitetexture_data = ptr::addr_of_mut!(WHITETEXTURE_DATA).cast::<u8>();
        let greytexture_data = ptr::addr_of_mut!(GREYTEXTURE_DATA).cast::<u8>();
        // SAFETY: the static arrays hold 2x2 RGBA8 pixels; nothing writes
        // them (no premultiply/downsample under these flags).
        let notexture = unsafe {
            self.load_image(
                backend,
                pal,
                ptr::null_mut(),
                c"notexture",
                2,
                2,
                rgba,
                notexture_data,
                c"",
                notexture_data as usize,
                FLAGS,
            )
        };
        // SAFETY: as above.
        let nulltexture = unsafe {
            self.load_image(
                backend,
                pal,
                ptr::null_mut(),
                c"nulltexture",
                2,
                2,
                rgba,
                nulltexture_data,
                c"",
                nulltexture_data as usize,
                FLAGS,
            )
        };
        // SAFETY: as above.
        let whitetexture = unsafe {
            self.load_image(
                backend,
                pal,
                ptr::null_mut(),
                c"whitetexture",
                2,
                2,
                rgba,
                whitetexture_data,
                c"",
                whitetexture_data as usize,
                FLAGS,
            )
        };
        // SAFETY: as above.
        let greytexture = unsafe {
            self.load_image(
                backend,
                pal,
                ptr::null_mut(),
                c"greytexture",
                2,
                2,
                rgba,
                greytexture_data,
                c"",
                greytexture_data as usize,
                FLAGS,
            )
        };
        // SAFETY: as above.
        let greylightmap = unsafe {
            self.load_image(
                backend,
                pal,
                ptr::null_mut(),
                c"greytexture",
                2,
                2,
                SrcFormat::Lightmap as c_int,
                greytexture_data,
                c"",
                greytexture_data as usize,
                FLAGS,
            )
        };
        // `u32` pixels (same bytes as the C's `byte[4]` fill): the RGBA
        // path reads them as `unsigned *`, so a `Vec<u8>` would only be
        // aligned by luck of the allocator
        let mut bluenoise_rgba: Vec<u32> = BLUENOISE_DATA
            .iter()
            .map(|&v| u32::from_ne_bytes([v, v, v, 0]))
            .collect();
        // SAFETY: `bluenoise_rgba` holds 64x64 RGBA8 pixels, 4-byte aligned.
        let bluenoisetexture = unsafe {
            self.load_image(
                backend,
                pal,
                ptr::null_mut(),
                c"bluenoise",
                64,
                64,
                rgba,
                bluenoise_rgba.as_mut_ptr().cast(),
                c"",
                greytexture_data as usize,
                FLAGS,
            )
        };
        BuiltinTextures {
            notexture,
            nulltexture,
            whitetexture,
            greytexture,
            greylightmap,
            bluenoisetexture,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mips_and_staging() {
        assert_eq!(derive_num_mips(256, 256), 9);
        assert_eq!(derive_num_mips(256, 64), 7);
        assert_eq!(derive_num_mips(1, 1), 1);
        assert_eq!(derive_staging_size(4, 4), (16 + 4 + 1) * 4);
    }

    #[test]
    fn premultiply_matches_c() {
        let mut px = [200, 100, 50, 128];
        premultiply32(&mut px);
        assert_eq!(px, [100, 50, 25, 128]);
    }

    #[test]
    fn palette_lump() {
        let mut lump = [0u8; 768];
        for (i, b) in lump.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let p = Palettes::from_lump(&lump);
        assert_eq!(p.table[255].to_ne_bytes()[3], 0);
        assert_eq!(p.table[0].to_ne_bytes(), [0, 1, 2, 255]);
        assert_eq!(p.fbright[0], rgba(0, 0, 0, 255));
        assert_eq!(p.nobright[255], rgba(0, 0, 0, 255));
        assert_eq!(p.fbright_fence[255], 0);
        assert_eq!(p.conchars[0].to_ne_bytes()[3], 0);
        assert_eq!(p.conchars[1], p.table[1]);
    }

    #[test]
    fn miptex_palette_alpha_and_ranges() {
        let input: Vec<u8> = (0..(256 * 3)).map(|i| (i % 255) as u8).collect();
        let mut out = [0u32; 256];
        load_miptex_palette(&input, &mut out, 256, TEXPREF_ALPHA | TEXPREF_NOBRIGHT, 1.0);
        assert_eq!(out[223].to_ne_bytes()[3], 255);
        assert_eq!(out[224], rgba(0, 0, 0, 255));
        assert_eq!(out[255], rgba(0, 0, 0, 0));
        let mut out = [0u32; 256];
        load_miptex_palette(&input, &mut out, 256, TEXPREF_FULLBRIGHT, 1.0);
        assert_eq!(out[0], rgba(0, 0, 0, 255));
        assert_eq!(out[224].to_ne_bytes()[..3], input[224 * 3..224 * 3 + 3]);
    }

    #[test]
    fn edge_fix_averages_neighbours() {
        // 2x1: left transparent, right opaque (wraps: every neighbour is the right pixel)
        let mut data = [0, 0, 0, 0, 100, 50, 10, 255];
        alpha_edge_fix(&mut data, 2, 1);
        assert_eq!(&data[..3], &[100, 50, 10]);
        assert_eq!(data[3], 0);
    }

    /// A backend that hands out counted handles and a real staging arena
    /// so the whole manager runs without Vulkan -- the Miri smoke test's
    /// backend (`cargo miri test -p quake-render`), which checks the raw
    /// arena/list pointers and the `&mut GlTexture` reborrows across
    /// `TexMgrLock::with` under Stacked/Tree Borrows.
    struct SmokeBackend {
        next_handle: core::cell::Cell<u64>,
        staging: RefCell<Vec<u8>>,
    }

    #[derive(Default)]
    struct SmokeHeap {
        next_handle: u64,
    }

    impl DeviceMemoryBackend for SmokeHeap {
        type Counter = ();
        fn allocate(
            &mut self,
            memory: &mut quake_types::render::VulkanMemory,
            size: u64,
            _memory_type_index: u32,
            memory_type: VulkanMemoryType,
            _device_address: bool,
            (): (),
        ) {
            self.next_handle += 1;
            memory.handle = vk::DeviceMemory::from_raw(self.next_handle);
            memory.size = size as usize;
            memory.type_ = memory_type;
        }
        fn free(&mut self, memory: &mut quake_types::render::VulkanMemory, (): ()) {
            *memory = quake_types::render::VulkanMemory::default();
        }
    }

    impl SmokeBackend {
        fn new() -> Self {
            Self {
                next_handle: core::cell::Cell::new(0),
                staging: RefCell::new(vec![0u8; 1 << 20]),
            }
        }
        fn handle(&self) -> u64 {
            self.next_handle.set(self.next_handle.get() + 1);
            self.next_handle.get()
        }
    }

    impl TexMgrBackend for SmokeBackend {
        type HeapBackend = SmokeHeap;
        fn new_heap_backend(&self) -> SmokeHeap {
            SmokeHeap::default()
        }
        fn heap_counter(&self) {}
        fn env(&self) -> Env {
            Env {
                max_image_dimension_2d: 4096,
                max_image_dimension_cube: 4096,
                color_format: vk::Format::B8G8R8A8_UNORM,
                point_sampler_lod_bias: vk::Sampler::from_raw(11),
                linear_sampler_lod_bias: vk::Sampler::from_raw(12),
                point_aniso_sampler_lod_bias: vk::Sampler::from_raw(13),
                linear_aniso_sampler_lod_bias: vk::Sampler::from_raw(14),
                warp_render_pass: vk::RenderPass::from_raw(15),
            }
        }
        fn cvars(&self) -> Cvars {
            Cvars {
                gl_fullbrights: 1.0,
                vid_filter: 0.0,
                vid_anisotropic: 0.0,
                gl_max_size: 0.0,
                gl_picmip: 0.0,
            }
        }
        fn no_rendering(&self) -> bool {
            false
        }
        fn in_update_screen(&self) -> bool {
            false
        }
        fn create_image(&self, _info: &vk::ImageCreateInfo<'_>) -> Result<vk::Image, i32> {
            Ok(vk::Image::from_raw(self.handle()))
        }
        fn destroy_image(&self, _image: vk::Image) {}
        fn image_memory_requirements(&self, _image: vk::Image) -> vk::MemoryRequirements {
            vk::MemoryRequirements {
                size: 65536,
                alignment: 256,
                memory_type_bits: 0xff,
            }
        }
        fn bind_image_memory(&self, _: vk::Image, _: vk::DeviceMemory, _: u64) -> Result<(), i32> {
            Ok(())
        }
        fn create_image_view(
            &self,
            _info: &vk::ImageViewCreateInfo<'_>,
        ) -> Result<vk::ImageView, i32> {
            Ok(vk::ImageView::from_raw(self.handle()))
        }
        fn destroy_image_view(&self, _view: vk::ImageView) {}
        fn create_framebuffer(
            &self,
            _info: &vk::FramebufferCreateInfo<'_>,
        ) -> Result<vk::Framebuffer, i32> {
            Ok(vk::Framebuffer::from_raw(self.handle()))
        }
        fn destroy_framebuffer(&self, _framebuffer: vk::Framebuffer) {}
        fn update_descriptor_set(&self, _write: &DescriptorWrite) {}
        fn allocate_descriptor_set(&self, _layout: DescLayout) -> vk::DescriptorSet {
            vk::DescriptorSet::from_raw(self.handle())
        }
        fn free_descriptor_set(&self, _set: vk::DescriptorSet, _layout: DescLayout) {}
        fn set_object_name(&self, _object: u64, _object_type: vk::ObjectType, _name: &str) {}
        fn memory_type_from_properties(
            &self,
            _type_bits: u32,
            _required: vk::MemoryPropertyFlags,
            _preferred: vk::MemoryPropertyFlags,
        ) -> u32 {
            0
        }
        fn wait_for_device_idle(&self) {}
        fn staging_allocate(&self, size: i32, _alignment: i32) -> Staging {
            let mut arena = self.staging.borrow_mut();
            assert!(size as usize <= arena.len());
            Staging {
                memory: arena.as_mut_ptr(),
                command_buffer: vk::CommandBuffer::from_raw(0x1000),
                buffer: vk::Buffer::from_raw(0x2000),
                offset: 0,
            }
        }
        fn staging_begin_copy(&self) {}
        fn staging_end_copy(&self) {}
        fn cmd_pipeline_barrier(
            &self,
            _: vk::CommandBuffer,
            _: vk::PipelineStageFlags,
            _: vk::PipelineStageFlags,
            _: &vk::ImageMemoryBarrier<'_>,
        ) {
        }
        fn cmd_copy_buffer_to_image(
            &self,
            _: vk::CommandBuffer,
            _: vk::Buffer,
            _: vk::Image,
            _: vk::ImageLayout,
            _: &[vk::BufferImageCopy],
        ) {
        }
        unsafe fn downsample(&self, data: *mut u32, iw: i32, ih: i32, ow: i32, oh: i32) {
            // nearest neighbour stands in for stb's filter: the smoke test
            // checks memory safety, not pixels
            let (iw, ih, ow, oh) = (iw as usize, ih as usize, ow as usize, oh as usize);
            // SAFETY: per the trait contract (`iw * ih` pixels at `data`).
            let src = unsafe { core::slice::from_raw_parts(data, iw * ih) };
            let out: Vec<u32> = (0..oh)
                .flat_map(|y| (0..ow).map(move |x| (x, y)))
                .map(|(x, y)| src[(y * ih / oh) * iw + x * iw / ow])
                .collect();
            // SAFETY: `ow * oh <= iw * ih` (contract); `out` is a separate
            // buffer, so `src` is no longer used.
            unsafe { core::ptr::copy_nonoverlapping(out.as_ptr(), data, out.len()) };
        }
        fn read_source(&self, _name: &CStr, _offset: usize, _size: usize) -> SourceRead {
            SourceRead::NotFound
        }
        fn image_load(&self, _name: &CStr, _min_path_id: u32) -> Option<LoadedImage> {
            None
        }
        unsafe fn free_loaded(&self, _data: *mut u8) {}
        unsafe fn owner_path_id(&self, _owner: *mut c_void) -> u32 {
            0
        }
        fn con_printf(&self, _msg: &str) {}
        fn sys_error(&self, msg: &str) -> ! {
            panic!("Sys_Error: {msg}");
        }
    }

    /// The manager's life cycle over the fake backend: arena setup, indexed,
    /// RGBA and Valve-palette uploads (including the overwrite cache hit and
    /// an index past a short palette), the reload of a texture in place,
    /// descriptor refresh, the garbage ring, and teardown. Meant for Miri.
    #[test]
    fn texmgr_smoke() {
        let backend = SmokeBackend::new();
        let mut lump = [0u8; 768];
        for (i, b) in lump.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let palettes = Palettes::from_lump(&lump);
        let pal = palettes.refs();
        let mut m = TexMgr::<SmokeBackend>::new();
        m.init_list();
        m.init_heap(&backend);
        assert_eq!(m.num_textures(), 0);

        let mut indexed: Vec<u8> = (0..64u8)
            .map(|i| if i % 5 == 0 { 255 } else { i })
            .collect();
        // `u32` pixels: `TexMgr_LoadImage32` takes `unsigned *`, so the
        // RGBA paths require 4-byte alignment (Miri checks it)
        let mut rgba: Vec<u32> = (0..4 * 4).map(|i| i * 0x0107_0b0d).collect();
        let mut rgba8: Vec<u32> = (0..8 * 8).map(|i| i * 0x0301_0507).collect();
        // an 8x8 Valve miptex with a four-colour palette and indices past it
        let mut valve = vec![0u8; 85 + 2 + 4 * 3];
        for (i, b) in valve[..64].iter_mut().enumerate() {
            *b = (i % 9) as u8;
        }
        valve[85] = 4;
        for (i, b) in valve[87..].iter_mut().enumerate() {
            *b = (i * 20) as u8;
        }

        let (first, second, third, fourth) = {
            let lock = Direct::new(&mut m);
            // SAFETY: `indexed` holds 8x8 indices; no owner.
            let first = unsafe {
                TexMgr::load_image_with(
                    &lock,
                    &backend,
                    &pal,
                    ptr::null_mut(),
                    c"smoke_indexed",
                    8,
                    8,
                    SrcFormat::Indexed as c_int,
                    indexed.as_mut_ptr(),
                    c"",
                    0,
                    TEXPREF_MIPMAP | TEXPREF_ALPHA | TEXPREF_OVERWRITE,
                )
            };
            // SAFETY: `rgba` holds 4x4 RGBA8 pixels.
            let second = unsafe {
                TexMgr::load_image_with(
                    &lock,
                    &backend,
                    &pal,
                    ptr::null_mut(),
                    c"smoke_rgba",
                    4,
                    4,
                    SrcFormat::Rgba as c_int,
                    rgba.as_mut_ptr().cast(),
                    c"",
                    0,
                    TEXPREF_MIPMAP | TEXPREF_ALPHA | TEXPREF_OVERWRITE | TEXPREF_PREMULTIPLY,
                )
            };
            // SAFETY: same data again -> the cache hit returns `second`.
            let third = unsafe {
                TexMgr::load_image_with(
                    &lock,
                    &backend,
                    &pal,
                    ptr::null_mut(),
                    c"smoke_rgba",
                    4,
                    4,
                    SrcFormat::Rgba as c_int,
                    rgba.as_mut_ptr().cast(),
                    c"",
                    0,
                    TEXPREF_MIPMAP | TEXPREF_ALPHA | TEXPREF_OVERWRITE | TEXPREF_PREMULTIPLY,
                )
            };
            // SAFETY: `valve` is a whole 8x8 Valve miptex.
            let fourth = unsafe {
                TexMgr::load_image_with(
                    &lock,
                    &backend,
                    &pal,
                    ptr::null_mut(),
                    c"smoke_valve",
                    8,
                    8,
                    SrcFormat::IndexedPalette as c_int,
                    valve.as_mut_ptr(),
                    c"",
                    0,
                    TEXPREF_MIPMAP,
                )
            };
            // SAFETY: `first` is live and 8x8, which `rgba8` covers;
            // overwrite it in place through the other lock granularity
            // (`TexMgr_LoadImage32` on an existing texture).
            unsafe { TexMgr::load_image32_with(&lock, &backend, first, rgba8.as_mut_ptr()) };
            (first, second, third, fourth)
        };
        assert!(!first.is_null() && !second.is_null() && !fourth.is_null());
        assert!(ptr::eq(second, third));
        assert_eq!(m.num_textures(), 3);
        assert_eq!(m.active_textures().count(), 3);
        assert!(ptr::eq(
            m.find_texture(ptr::null_mut(), Some(c"smoke_valve")),
            fourth
        ));
        // SAFETY: arena pointers, no live references.
        unsafe {
            assert_eq!((*first).width, 8);
            assert_eq!((*second).width, 4);
            assert_eq!((*fourth).source_format, SrcFormat::IndexedPalette as c_int);
        }

        m.update_texture_descriptor_sets(&backend);
        m.collect_garbage(&backend);
        m.collect_garbage(&backend);
        // SAFETY: `second` is on the active list.
        unsafe { m.free_texture(&backend, second) };
        assert_eq!(m.num_textures(), 2);
        m.free_textures(&backend, 0, 0);
        assert_eq!(m.num_textures(), 0);
        m.collect_garbage(&backend);
        m.collect_garbage(&backend);
        m.delete_texture_objects(&backend);
        m.init_list();
        assert_eq!(m.num_textures(), 0);
        drop(m);
        drop(TexMgr::<SmokeBackend>::new());
    }
}
