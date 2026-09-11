//! `gl_texmgr.c` -- the texture manager's C ABI over `quake_render::texmgr`
//! (Rust migration Phase 8 M4, ADR-015). Compiled into the staticlib under
//! the `render` feature and linked instead of `gl_texmgr.c` under
//! `-Duse_rust_render` (`Quake/gl_texmgr_glue.c` carries the C-visible data
//! and the glue accessors).
//!
//! Ownership: the texture list (`gltexture_t` arena, active/free chains,
//! `numgltextures`), the texture heap and the garbage ring are Rust-owned
//! behind one `Mutex` (the C's `texmgr_mutex`). `gltexture_t` itself stays a
//! C-layout struct (`quake_types::render::GlTexture`, ADR-011): the C
//! renderer reads `gltexture` fields through the pointers this module hands
//! back. The six `d_8to24table*` palettes and the six well-known texture
//! pointers (`notexture` ...) are C globals defined in the glue TU and
//! written from here, so every C reader keeps its plain extern (ADR-007
//! dual view).
//!
//! Lock granularity: each C `QMutex_Lock`/`QMutex_Unlock` pair is one
//! [`TexMgrLock::with`] call through [`EngineLock`], so
//! `TexMgr_LoadImage` releases the mutex between the cache check, the image
//! creation and the upload exactly where the C does (the world loader's
//! texture tasks load in parallel). Entry points the C locks once run under
//! one `lock()` guard.
//!
//! ADR-009: `TexMgr_Init` registers two cvars and one command, which can
//! `Host_Error`; the glue TU owns the `void TexMgr_Init (void)` symbol,
//! calls [`quake_rs_texmgr_init`] and `Host_Reraise`s its result after
//! this frame has returned. The registrations themselves go through
//! `Host_Guard` thunks in the glue. Everything else here can only
//! `Sys_Error` (which terminates).

use ash::vk::{self, Handle};
use core::ffi::{c_char, c_int, c_uint, c_void, CStr};
use core::ptr;
use std::ffi::CString;
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use quake_c_sys as c;
use quake_c_sys::render as g;
use quake_render::texmgr::{
    Cvars, DescLayout, DescriptorWrite, Env, LoadedImage, PaletteRefs, Palettes, SourceRead,
    Staging, TexMgr, TexMgrBackend, TexMgrLock,
};
use quake_types::render::{GlHeapStats, GlTexture, SrcFormat, VulkanDescSetLayout, TEXPREF_MIPMAP};

use crate::gl_heap::CBackend;

/// The `vulkan_globals` members `gl_texmgr.c` reads, snapshotted from the
/// Rust-owned `vulkan_globals` (`gl_rmisc.rs`, Phase 8 M5; until then the
/// C glue copied them out through `TexMgr_Glue_VulkanEnv`). `VkDevice` is
/// kept as the opaque pointer the `vk*` loader entry points take.
struct GlueEnv {
    device: *mut c_void,
    max_image_dimension_2d: u32,
    max_image_dimension_cube: u32,
    color_format: vk::Format,
    point_sampler_lod_bias: vk::Sampler,
    linear_sampler_lod_bias: vk::Sampler,
    point_aniso_sampler_lod_bias: vk::Sampler,
    linear_aniso_sampler_lod_bias: vk::Sampler,
    warp_render_pass: vk::RenderPass,
    single_texture_set_layout: *mut VulkanDescSetLayout,
    single_texture_cs_write_set_layout: *mut VulkanDescSetLayout,
}

impl GlueEnv {
    fn fetch() -> Self {
        let vg = ptr::addr_of_mut!(crate::gl_rmisc::vulkan_globals);
        // SAFETY: plain reads of scalar members of the exported static (ADR-007
        // dual view: the same reads the C `gl_texmgr.c` made in place), and
        // the addresses of two of its members, which stay valid for the
        // program's lifetime.
        unsafe {
            Self {
                device: (*vg).device.as_raw() as usize as *mut c_void,
                max_image_dimension_2d: (*vg).device_properties.limits.max_image_dimension2_d,
                max_image_dimension_cube: (*vg).device_properties.limits.max_image_dimension_cube,
                color_format: (*vg).color_format,
                point_sampler_lod_bias: (*vg).point_sampler_lod_bias,
                linear_sampler_lod_bias: (*vg).linear_sampler_lod_bias,
                point_aniso_sampler_lod_bias: (*vg).point_aniso_sampler_lod_bias,
                linear_aniso_sampler_lod_bias: (*vg).linear_aniso_sampler_lod_bias,
                warp_render_pass: (*vg).warp_render_pass,
                single_texture_set_layout: ptr::addr_of_mut!((*vg).single_texture_set_layout),
                single_texture_cs_write_set_layout: ptr::addr_of_mut!(
                    (*vg).single_texture_cs_write_set_layout
                ),
            }
        }
    }
}

/// The engine backend: the Vulkan loader for the `vk*` calls (with
/// `vulkan_globals.device` read through the glue), `gl_rmisc.c`/`gl_vidsdl.c`
/// for staging, descriptor sets and memory types, the file system and image
/// loader for sources, and the glue for everything `glquake.h`-shaped.
pub struct EngineBackend {
    /// One `vulkan_globals` snapshot per exported entry point. The C read
    /// `vulkan_globals` in place; the device and samplers only change
    /// between entry points (`vid_restart` on the main thread), so a fetch
    /// per `TexMgr_*` call keeps each `vk*` call at a field read.
    glue: GlueEnv,
}

// SAFETY: `TexMgr<EngineBackend>` sits behind the `TEXMGR` mutex, whose
// `Send` impl asks for `EngineBackend: Send`. The only field is the glue
// snapshot: an opaque `VkDevice` (the C reads `vulkan_globals.device` from
// any thread already) and plain handles, none dereferenced by Rust; each
// snapshot lives on the entry point's stack and never crosses a thread.
unsafe impl Send for EngineBackend {}

impl EngineBackend {
    fn snapshot() -> Self {
        Self {
            glue: GlueEnv::fetch(),
        }
    }

    fn device(&self) -> *mut c_void {
        self.glue.device
    }
}

fn cstring(s: &str) -> CString {
    CString::new(s.replace('\0', "?")).expect("no interior NUL")
}

impl TexMgrBackend for EngineBackend {
    type HeapBackend = CBackend;

    fn new_heap_backend(&self) -> CBackend {
        CBackend::new(c"Texture Heap".as_ptr())
    }

    fn heap_counter(&self) -> *mut c_void {
        ptr::addr_of!(crate::gl_rmisc::num_vulkan_tex_allocations)
            .cast_mut()
            .cast()
    }

    fn env(&self) -> Env {
        let e = &self.glue;
        Env {
            max_image_dimension_2d: e.max_image_dimension_2d,
            max_image_dimension_cube: e.max_image_dimension_cube,
            color_format: e.color_format,
            point_sampler_lod_bias: e.point_sampler_lod_bias,
            linear_sampler_lod_bias: e.linear_sampler_lod_bias,
            point_aniso_sampler_lod_bias: e.point_aniso_sampler_lod_bias,
            linear_aniso_sampler_lod_bias: e.linear_aniso_sampler_lod_bias,
            warp_render_pass: e.warp_render_pass,
        }
    }

    fn cvars(&self) -> Cvars {
        // SAFETY: the five cvars are engine globals, read like any C reader
        // of `.value` (main thread or under the texture loader's own
        // ordering, as in the C).
        unsafe {
            Cvars {
                gl_fullbrights: (*ptr::addr_of!(g::gl_fullbrights)).value,
                vid_filter: (*ptr::addr_of!(c::menu::vid_filter)).value,
                vid_anisotropic: (*ptr::addr_of!(c::menu::vid_anisotropic)).value,
                gl_max_size: (*ptr::addr_of!(g::gl_max_size)).value,
                gl_picmip: (*ptr::addr_of!(g::gl_picmip)).value,
            }
        }
    }

    fn no_rendering(&self) -> bool {
        // SAFETY: a plain engine global, set once at startup.
        unsafe { *ptr::addr_of!(c::host::no_rendering) }
    }

    fn in_update_screen(&self) -> bool {
        // SAFETY: a plain engine global written by the main thread only.
        unsafe { *ptr::addr_of!(g::in_update_screen) }
    }

    fn create_image(&self, info: &vk::ImageCreateInfo<'_>) -> Result<vk::Image, i32> {
        let mut image = 0u64;
        // SAFETY: `info` is a complete `VkImageCreateInfo` whose pNext chain
        // outlives the call; the device is the engine's.
        let r = unsafe {
            g::vkCreateImage(
                self.device(),
                ptr::from_ref(info).cast(),
                ptr::null(),
                &mut image,
            )
        };
        if r == 0 {
            Ok(vk::Image::from_raw(image))
        } else {
            Err(r)
        }
    }

    fn destroy_image(&self, image: vk::Image) {
        // SAFETY: `image` came from `create_image` and is destroyed once.
        unsafe { g::vkDestroyImage(self.device(), image.as_raw(), ptr::null()) }
    }

    fn image_memory_requirements(&self, image: vk::Image) -> vk::MemoryRequirements {
        let mut reqs = vk::MemoryRequirements::default();
        // SAFETY: `image` is live; `reqs` is a `VkMemoryRequirements`.
        unsafe {
            g::vkGetImageMemoryRequirements(
                self.device(),
                image.as_raw(),
                ptr::from_mut(&mut reqs).cast(),
            )
        };
        reqs
    }

    fn bind_image_memory(
        &self,
        image: vk::Image,
        memory: vk::DeviceMemory,
        offset: u64,
    ) -> Result<(), i32> {
        // SAFETY: `image` and `memory` are live handles of this device.
        let r =
            unsafe { g::vkBindImageMemory(self.device(), image.as_raw(), memory.as_raw(), offset) };
        if r == 0 {
            Ok(())
        } else {
            Err(r)
        }
    }

    fn create_image_view(&self, info: &vk::ImageViewCreateInfo<'_>) -> Result<vk::ImageView, i32> {
        let mut view = 0u64;
        // SAFETY: as in `create_image`.
        let r = unsafe {
            g::vkCreateImageView(
                self.device(),
                ptr::from_ref(info).cast(),
                ptr::null(),
                &mut view,
            )
        };
        if r == 0 {
            Ok(vk::ImageView::from_raw(view))
        } else {
            Err(r)
        }
    }

    fn destroy_image_view(&self, view: vk::ImageView) {
        // SAFETY: `view` came from `create_image_view` and is destroyed once.
        unsafe { g::vkDestroyImageView(self.device(), view.as_raw(), ptr::null()) }
    }

    fn create_framebuffer(
        &self,
        info: &vk::FramebufferCreateInfo<'_>,
    ) -> Result<vk::Framebuffer, i32> {
        let mut fb = 0u64;
        // SAFETY: as in `create_image`; the attachment array outlives the call.
        let r = unsafe {
            g::vkCreateFramebuffer(
                self.device(),
                ptr::from_ref(info).cast(),
                ptr::null(),
                &mut fb,
            )
        };
        if r == 0 {
            Ok(vk::Framebuffer::from_raw(fb))
        } else {
            Err(r)
        }
    }

    fn destroy_framebuffer(&self, framebuffer: vk::Framebuffer) {
        // SAFETY: `framebuffer` came from `create_framebuffer`, destroyed once.
        unsafe { g::vkDestroyFramebuffer(self.device(), framebuffer.as_raw(), ptr::null()) }
    }

    fn update_descriptor_set(&self, w: &DescriptorWrite) {
        let image_info = vk::DescriptorImageInfo {
            sampler: w.sampler,
            image_view: w.image_view,
            image_layout: w.image_layout,
        };
        let write = vk::WriteDescriptorSet::default()
            .dst_set(w.set)
            .dst_binding(w.binding)
            .dst_array_element(0)
            .descriptor_type(w.descriptor_type)
            .image_info(core::slice::from_ref(&image_info));
        // SAFETY: one complete `VkWriteDescriptorSet` whose image-info array
        // outlives the call, no copies.
        unsafe {
            g::vkUpdateDescriptorSets(
                self.device(),
                1,
                ptr::from_ref(&write).cast(),
                0,
                ptr::null(),
            )
        }
    }

    fn allocate_descriptor_set(&self, layout: DescLayout) -> vk::DescriptorSet {
        let e = &self.glue;
        let l = match layout {
            DescLayout::SingleTexture => e.single_texture_set_layout,
            DescLayout::SingleTextureCsWrite => e.single_texture_cs_write_set_layout,
        };
        // SAFETY: `l` points at one of the two `vulkan_globals` layouts.
        unsafe { crate::gl_rmisc::R_AllocateDescriptorSet(l) }
    }

    fn free_descriptor_set(&self, set: vk::DescriptorSet, layout: DescLayout) {
        let e = &self.glue;
        let l = match layout {
            DescLayout::SingleTexture => e.single_texture_set_layout,
            DescLayout::SingleTextureCsWrite => e.single_texture_cs_write_set_layout,
        };
        // SAFETY: as in `allocate_descriptor_set`; the set is freed once.
        unsafe { crate::gl_rmisc::R_FreeDescriptorSet(set, l) }
    }

    fn set_object_name(&self, object: u64, object_type: vk::ObjectType, name: &str) {
        let name = cstring(name);
        // SAFETY: `name` is NUL-terminated and outlives the call, which
        // copies it.
        unsafe { crate::gl_vidsdl::GL_SetObjectName(object, object_type.as_raw(), name.as_ptr()) }
    }

    fn memory_type_from_properties(
        &self,
        type_bits: u32,
        required: vk::MemoryPropertyFlags,
        preferred: vk::MemoryPropertyFlags,
    ) -> u32 {
        crate::gl_rmisc::GL_MemoryTypeFromProperties(
            type_bits,
            required.as_raw(),
            preferred.as_raw(),
        ) as u32
    }

    fn wait_for_device_idle(&self) {
        crate::gl_vidsdl::GL_WaitForDeviceIdle();
    }

    fn staging_allocate(&self, size: i32, alignment: i32) -> Staging {
        let mut cb = vk::CommandBuffer::null();
        let mut buffer = vk::Buffer::null();
        let mut offset = 0 as c_int;
        // SAFETY: the three out-params are live locals.
        let memory = unsafe {
            crate::gl_rmisc::R_StagingAllocate(size, alignment, &mut cb, &mut buffer, &mut offset)
        };
        Staging {
            memory,
            command_buffer: cb,
            buffer,
            offset,
        }
    }

    fn staging_begin_copy(&self) {
        crate::gl_rmisc::R_StagingBeginCopy()
    }

    fn staging_end_copy(&self) {
        crate::gl_rmisc::R_StagingEndCopy()
    }

    fn cmd_pipeline_barrier(
        &self,
        command_buffer: vk::CommandBuffer,
        src_stage: vk::PipelineStageFlags,
        dst_stage: vk::PipelineStageFlags,
        barrier: &vk::ImageMemoryBarrier<'_>,
    ) {
        // SAFETY: the staging command buffer is in the recording state
        // between `R_StagingAllocate` and `R_StagingEndCopy`; one complete
        // image barrier, no memory/buffer barriers.
        unsafe {
            g::vkCmdPipelineBarrier(
                command_buffer.as_raw() as usize as *mut c_void,
                src_stage.as_raw(),
                dst_stage.as_raw(),
                0,
                0,
                ptr::null(),
                0,
                ptr::null(),
                1,
                ptr::from_ref(barrier).cast(),
            )
        }
    }

    fn cmd_copy_buffer_to_image(
        &self,
        command_buffer: vk::CommandBuffer,
        buffer: vk::Buffer,
        image: vk::Image,
        layout: vk::ImageLayout,
        regions: &[vk::BufferImageCopy],
    ) {
        // SAFETY: as in `cmd_pipeline_barrier`; `regions` is a live slice
        // of `VkBufferImageCopy`.
        unsafe {
            g::vkCmdCopyBufferToImage(
                command_buffer.as_raw() as usize as *mut c_void,
                buffer.as_raw(),
                image.as_raw(),
                layout.as_raw(),
                regions.len() as u32,
                regions.as_ptr().cast(),
            )
        }
    }

    unsafe fn downsample(
        &self,
        data: *mut u32,
        in_width: i32,
        in_height: i32,
        out_width: i32,
        out_height: i32,
    ) {
        // SAFETY: per the trait contract.
        unsafe { g::TexMgr_Glue_Downsample(data, in_width, in_height, out_width, out_height) }
    }

    fn read_source(&self, name: &CStr, offset: usize, size: usize) -> SourceRead {
        let mut file: *mut c::FILE = ptr::null_mut();
        let mut path_id: c_uint = 0;
        // SAFETY: `name` is NUL-terminated; the out-params are live locals.
        unsafe { c::COM_FOpenFile(name.as_ptr(), &mut file, &mut path_id) };
        if file.is_null() {
            return SourceRead::NotFound;
        }
        let mut buf = vec![0u8; size];
        // SAFETY: `file` is open; `buf` holds `size` bytes; the file is
        // closed exactly once.
        let got = unsafe {
            if offset != 0 {
                c::Sys_fseek(file, offset as c::qfileofs_t, 1 /* SEEK_CUR */);
            }
            let got = c::stdio::fread(buf.as_mut_ptr().cast(), 1, size, file);
            c::stdio::fclose(file);
            got
        };
        if got == size {
            SourceRead::Ok(buf)
        } else {
            SourceRead::Short
        }
    }

    fn image_load(&self, name: &CStr, min_path_id: u32) -> Option<LoadedImage> {
        let mut width: c_int = 0;
        let mut height: c_int = 0;
        let mut format: c_int = 0;
        // SAFETY: `name` is NUL-terminated; the out-params are live locals.
        let data = unsafe {
            g::Image_LoadImage(
                name.as_ptr(),
                &mut width,
                &mut height,
                &mut format,
                min_path_id,
            )
        };
        if data.is_null() {
            None
        } else {
            Some(LoadedImage {
                data,
                width,
                height,
                format,
            })
        }
    }

    unsafe fn free_loaded(&self, data: *mut u8) {
        // SAFETY: per the trait contract, an `Image_LoadImage` buffer.
        unsafe { c::Mem_Free(data.cast()) }
    }

    unsafe fn owner_path_id(&self, owner: *mut c_void) -> u32 {
        // SAFETY: per the trait contract, a live `qmodel_t`.
        unsafe { g::TexMgr_Glue_OwnerPathId(owner) }
    }

    fn con_printf(&self, msg: &str) {
        let msg = cstring(msg);
        // SAFETY: `"%s"` with one NUL-terminated argument.
        unsafe { c::Con_Printf(c"%s".as_ptr(), msg.as_ptr()) }
    }

    fn sys_error(&self, msg: &str) -> ! {
        let msg = cstring(msg);
        // SAFETY: `"%s"` with one NUL-terminated argument; Sys_Error never
        // returns (ADR-009: it terminates, no longjmp).
        unsafe { c::Sys_Error(c"%s".as_ptr(), msg.as_ptr()) }
    }
}

/// `texmgr_mutex` and everything it guards.
static TEXMGR: LazyLock<Mutex<TexMgr<EngineBackend>>> = LazyLock::new(|| Mutex::new(TexMgr::new()));

fn lock() -> MutexGuard<'static, TexMgr<EngineBackend>> {
    // `panic = "abort"`: a poisoned lock cannot be observed, so the
    // fallback is unreachable in practice.
    TEXMGR.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One `QMutex_Lock`/`QMutex_Unlock` pair per `with`.
struct EngineLock;

impl TexMgrLock<EngineBackend> for EngineLock {
    fn with<R>(&self, f: impl FnOnce(&mut TexMgr<EngineBackend>) -> R) -> R {
        f(&mut lock())
    }
}

/// The six C palette arrays (`Quake/gl_texmgr_glue.c`) as the core's
/// references. The arrays are written only by [`store_palettes`] on the main
/// thread (`TexMgr_Init`, `TexMgr_NewGame`, `TexMgr_LoadPalette`) and read
/// everywhere, as in the C.
fn palettes() -> PaletteRefs<'static> {
    // SAFETY: the six arrays are `unsigned int [256]` C globals that live
    // for the whole process; `c_uint` is `u32`.
    unsafe {
        PaletteRefs {
            table: &*ptr::addr_of!(g::d_8to24table),
            fbright: &*ptr::addr_of!(g::d_8to24table_fbright),
            fbright_fence: &*ptr::addr_of!(g::d_8to24table_fbright_fence),
            nobright: &*ptr::addr_of!(g::d_8to24table_nobright),
            nobright_fence: &*ptr::addr_of!(g::d_8to24table_nobright_fence),
            conchars: &*ptr::addr_of!(g::d_8to24table_conchars),
        }
    }
}

fn store_palettes(p: &Palettes) {
    // SAFETY: as in `palettes`; main-thread writes of the C globals, exactly
    // what `TexMgr_LoadPalette` does in C.
    unsafe {
        ptr::write(ptr::addr_of_mut!(g::d_8to24table), p.table);
        ptr::write(ptr::addr_of_mut!(g::d_8to24table_fbright), p.fbright);
        ptr::write(
            ptr::addr_of_mut!(g::d_8to24table_fbright_fence),
            p.fbright_fence,
        );
        ptr::write(ptr::addr_of_mut!(g::d_8to24table_nobright), p.nobright);
        ptr::write(
            ptr::addr_of_mut!(g::d_8to24table_nobright_fence),
            p.nobright_fence,
        );
        ptr::write(ptr::addr_of_mut!(g::d_8to24table_conchars), p.conchars);
    }
}

/// `gl_texmgr.h` -- `void TexMgr_InitHeap (void)`
#[no_mangle]
pub extern "C" fn TexMgr_InitHeap() {
    lock().init_heap(&EngineBackend::snapshot());
}

/// `gl_texmgr.h` -- `gltexture_t *TexMgr_FindTexture (qmodel_t *owner, const
/// char *name)`. `name` may be null (the C compares it with `strcmp` only
/// when non-null).
///
/// # Safety
/// `name` is null or NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn TexMgr_FindTexture(
    owner: *mut c_void,
    name: *const c_char,
) -> *mut GlTexture {
    // SAFETY: per the contract.
    let name = unsafe { name.as_ref().map(|p| CStr::from_ptr(p)) };
    lock().find_texture(owner, name)
}

/// `gl_texmgr.h` -- `gltexture_t *TexMgr_NewTexture (void)`
#[no_mangle]
pub extern "C" fn TexMgr_NewTexture() -> *mut GlTexture {
    lock().new_texture()
}

/// `gl_texmgr.h` -- `void TexMgr_FreeTexture (gltexture_t *kill)`
///
/// # Safety
/// `kill` is null or a texture this manager handed out.
#[no_mangle]
pub unsafe extern "C" fn TexMgr_FreeTexture(kill: *mut GlTexture) {
    // SAFETY: per the contract.
    unsafe { lock().free_texture(&EngineBackend::snapshot(), kill) }
}

/// `gl_texmgr.h` -- `void TexMgr_FreeTextures (unsigned int flags, unsigned
/// int mask)`
#[no_mangle]
pub extern "C" fn TexMgr_FreeTextures(flags: c_uint, mask: c_uint) {
    lock().free_textures(&EngineBackend::snapshot(), flags, mask);
}

/// `gl_texmgr.h` -- `void TexMgr_FreeTexturesForOwner (qmodel_t *owner)`
#[no_mangle]
pub extern "C" fn TexMgr_FreeTexturesForOwner(owner: *mut c_void) {
    lock().free_textures_for_owner(&EngineBackend::snapshot(), owner);
}

/// `gl_texmgr.h` -- `void TexMgr_NewGame (void)`
#[no_mangle]
pub extern "C" fn TexMgr_NewGame() {
    lock().new_game_free(&EngineBackend::snapshot());
    TexMgr_LoadPalette();
}

/// `gl_texmgr.h` -- `void TexMgr_DeleteTextureObjects (void)`
#[no_mangle]
pub extern "C" fn TexMgr_DeleteTextureObjects() {
    lock().delete_texture_objects(&EngineBackend::snapshot());
}

/// `gl_texmgr.h` -- `void TexMgr_CollectGarbage (void)`
#[no_mangle]
pub extern "C" fn TexMgr_CollectGarbage() {
    lock().collect_garbage(&EngineBackend::snapshot());
}

/// `gl_texmgr.h` -- `void TexMgr_LoadPalette (void)`
#[no_mangle]
pub extern "C" fn TexMgr_LoadPalette() {
    let p = TexMgr::<EngineBackend>::load_palette(&EngineBackend::snapshot());
    store_palettes(&p);
}

/// `gl_texmgr.h` -- `gltexture_t *TexMgr_LoadImage (qmodel_t *owner, const
/// char *name, int width, int height, enum srcformat format, byte *data,
/// const char *source_file, src_offset_t source_offset, unsigned flags)`
///
/// # Safety
/// `name` and `source_file` are NUL-terminated; `data` holds the image
/// `format` describes at `width x height`, 4-byte aligned for the 32-bit
/// formats (null only for a `TEXPREF_WARPIMAGE` texture, which uploads
/// nothing); `owner` is null or a live `qmodel_t`.
#[no_mangle]
pub unsafe extern "C" fn TexMgr_LoadImage(
    owner: *mut c_void,
    name: *const c_char,
    width: c_int,
    height: c_int,
    format: c_int,
    data: *mut u8,
    source_file: *const c_char,
    source_offset: usize,
    flags: c_uint,
) -> *mut GlTexture {
    // SAFETY: per the contract.
    unsafe {
        TexMgr::load_image_with(
            &EngineLock,
            &EngineBackend::snapshot(),
            &palettes(),
            owner,
            CStr::from_ptr(name),
            width,
            height,
            format,
            data,
            CStr::from_ptr(source_file),
            source_offset,
            flags,
        )
    }
}

/// `gl_texmgr.h` -- `void TexMgr_ReloadImage (gltexture_t *glt, int shirt,
/// int pants)`
///
/// # Safety
/// `glt` is a texture this manager handed out.
#[no_mangle]
pub unsafe extern "C" fn TexMgr_ReloadImage(glt: *mut GlTexture, shirt: c_int, pants: c_int) {
    // SAFETY: per the contract.
    unsafe {
        TexMgr::reload_image_with(
            &EngineLock,
            &EngineBackend::snapshot(),
            &palettes(),
            glt,
            shirt,
            pants,
        )
    }
}

/// `gl_texmgr.h` -- `void TexMgr_ReloadNobrightImages (void)`
#[no_mangle]
pub extern "C" fn TexMgr_ReloadNobrightImages() {
    lock().reload_nobright_images(&EngineBackend::snapshot(), &palettes());
}

/// `gl_texmgr.h` -- `void TexMgr_UpdateTextureDescriptorSets (void)`
#[no_mangle]
pub extern "C" fn TexMgr_UpdateTextureDescriptorSets() {
    lock().update_texture_descriptor_sets(&EngineBackend::snapshot());
}

/// `gl_texmgr.h` -- `glheapstats_t *TexMgr_GetHeapStats (void)`. The stats
/// live inside the manager's heap, which never moves (a `static`), so the
/// pointer stays valid after the lock is released, as the C's is.
#[no_mangle]
pub extern "C" fn TexMgr_GetHeapStats() -> *mut GlHeapStats {
    let mut guard = lock();
    // SAFETY: the guard's target is the live static manager.
    unsafe { TexMgr::heap_stats_ptr(ptr::from_mut(&mut *guard)) }
}

/// `TexMgr_Init` minus the ADR-009 wrapper: the glue's `void TexMgr_Init
/// (void)` calls this and `Host_Reraise`s the result. Non-zero is a
/// `Host_Error` caught by one of the registration thunks, after which the
/// remaining steps are skipped (the C would have longjmp'd out at the same
/// point).
#[no_mangle]
pub extern "C" fn quake_rs_texmgr_init() -> c_int {
    {
        let mut m = lock();
        m.init_list();
        let p = TexMgr::<EngineBackend>::load_palette(&EngineBackend::snapshot());
        store_palettes(&p);
    }
    // SAFETY: Host_Guard thunks over the glue-owned cvars/command.
    let r = unsafe { g::TexMgr_Glue_RegisterVariables() };
    if r != 0 {
        return r;
    }
    // SAFETY: as above.
    let r = unsafe { g::TexMgr_Glue_RegisterCommands() };
    if r != 0 {
        return r;
    }
    let b = lock().load_builtin_textures(&EngineBackend::snapshot(), &palettes());
    // SAFETY: main-thread writes of the six glue-owned pointers, followed
    // by the glue's `r_notexture_mip` assignment; `notexture` is the arena
    // pointer the C would store.
    unsafe {
        ptr::write(ptr::addr_of_mut!(g::notexture), b.notexture.cast());
        ptr::write(ptr::addr_of_mut!(g::nulltexture), b.nulltexture.cast());
        ptr::write(ptr::addr_of_mut!(g::whitetexture), b.whitetexture.cast());
        ptr::write(ptr::addr_of_mut!(g::greytexture), b.greytexture.cast());
        ptr::write(ptr::addr_of_mut!(g::greylightmap), b.greylightmap.cast());
        ptr::write(
            ptr::addr_of_mut!(g::bluenoisetexture),
            b.bluenoisetexture.cast(),
        );
        g::TexMgr_Glue_SetNotextureMips(b.notexture.cast());
    }
    0
}

/// `TexMgr_Imagelist_f` -- `imagelist [filter]`, registered by the glue's
/// `TexMgr_Glue_RegisterCommands`.
#[no_mangle]
pub extern "C" fn TexMgr_Rust_Imagelist_f() {
    // SAFETY: `Cmd_Argc`/`Cmd_Argv` are valid inside a command handler;
    // `Cmd_Argv (1)` is NUL-terminated and lives for the handler.
    let filter: *const c_char = unsafe {
        if c::Cmd_Argc() >= 2 {
            c::Cmd_Argv(1)
        } else {
            ptr::null()
        }
    };
    let mut texels: f32 = 0.0;
    let mut count: c_int = 0;
    let mut displayed_name = [0 as c_char; 64];
    let numgltextures;
    {
        let m = lock();
        for glt in m.active_textures() {
            // SAFETY: arena pointer from the manager's own list.
            let t = unsafe { &*glt };
            let name = t.name.as_ptr().cast::<c_char>();
            // SAFETY: `name` and `filter` are NUL-terminated;
            // `displayed_name` holds `MAX_QPATH` bytes.
            let shown = unsafe {
                if !filter.is_null() {
                    if c::menu::q_strcasestr(name, filter).is_null() {
                        continue;
                    }
                    c::menu::COM_TintSubstring(
                        name,
                        filter,
                        displayed_name.as_mut_ptr(),
                        displayed_name.len(),
                    );
                    displayed_name.as_ptr()
                } else {
                    name
                }
            };
            if t.flags & TEXPREF_MIPMAP != 0 {
                texels += t.width.wrapping_mul(t.height) as f32 * 4.0 / 3.0;
            } else {
                texels += t.width.wrapping_mul(t.height) as f32;
            }
            // SAFETY: the format strings match their arguments (`%4i` <-
            // `int`, `%s` <- NUL-terminated).
            unsafe {
                if t.source_format == SrcFormat::RgbaCubemap as c_int {
                    c::Con_SafePrintf(c"   %4i CUBE  %s\n".as_ptr(), t.width as c_int, shown);
                    texels *= 6.0;
                } else {
                    c::Con_SafePrintf(
                        c"   %4i x%4i %s\n".as_ptr(),
                        t.width as c_int,
                        t.height as c_int,
                        shown,
                    );
                }
            }
            count += 1;
        }
        numgltextures = m.num_textures();
    }
    let bytes = texels * 4.0;
    // SAFETY: the format strings match their arguments (`%i` <- `int`,
    // `%s` <- NUL-terminated, `%.1lf`/`%1.1lf` <- `double`).
    unsafe {
        if !filter.is_null() {
            if texels < 100000.0 {
                c::Con_Printf(
                    c"%i/%i textures containing '%s': %.1lf pixels %1.1lf bytes\n".as_ptr(),
                    count,
                    numgltextures,
                    filter,
                    f64::from(texels),
                    f64::from(bytes),
                );
            } else {
                c::Con_Printf(
                    c"%i/%i textures containing '%s': %.1lf mpixels %1.1lf megabytes\n".as_ptr(),
                    count,
                    numgltextures,
                    filter,
                    f64::from(texels) * 1e-6,
                    f64::from(bytes / 1048576.0),
                );
            }
        } else if texels < 100000.0 {
            c::Con_Printf(
                c"%i textures %.1lf pixels %1.1lf bytes\n".as_ptr(),
                numgltextures,
                f64::from(texels),
                f64::from(bytes),
            );
        } else {
            c::Con_Printf(
                c"%i textures %.1lf mpixels %1.1lf megabytes\n".as_ptr(),
                numgltextures,
                f64::from(texels) * 1e-6,
                f64::from(bytes / 1048576.0),
            );
        }
    }
}

/// `TexMgr_Imagelist_Completion_f` -- tab completion for `imagelist`.
///
/// # Safety
/// `partial` is NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn TexMgr_Rust_Imagelist_Completion_f(partial: *const c_char) {
    let m = lock();
    for glt in m.active_textures() {
        // SAFETY: arena pointer; the name is NUL-terminated; `partial` per
        // the contract.
        unsafe {
            c::cl_main::Con_AddToTabList(
                (*glt).name.as_ptr().cast::<c_char>(),
                partial,
                ptr::null(),
            )
        }
    }
}
