//! Differential for `quake_render::texmgr` vs the C `gl_texmgr.c` oracle
//! (Rust migration Phase 8 M4, ADR-015).
//!
//! Both sides drive the texture manager against the same fake seams: a
//! sequential-handle Vulkan (`stubs/gl_texmgr_ref.c` for C, `FakeBackend`
//! here), a bump-arena staging buffer whose contents are FNV-hashed at every
//! `R_StagingEndCopy`, a fake file system and image loader, and the M3 fake
//! device-memory backend under the texture heap. Every seam call is recorded
//! as a text line in exactly the C stub's format, so after each operation the
//! two traces are compared verbatim -- image/view/framebuffer/descriptor
//! parameters, barrier and copy regions, heap offsets, the pixel pipeline's
//! output bytes (palette, alpha fix, premultiply, mip chain, cube faces,
//! 10-bit packing) and the console messages -- followed by a walk of both
//! active lists comparing every `gltexture_t` field but the allocation
//! pointer. Proptest generates the operation sequences.
//!
//! The C oracle's process-wide state (the list, the heap, the palettes, the
//! fake file system) is initialised once and reset per case through
//! `TexMgr_NewGame` + garbage collection, so the cases run under `C_LOCK`.

use ash::vk;
use ash::vk::Handle;
use core::cell::{Cell, RefCell};
use core::ffi::{c_char, c_int, c_uint, c_void, CStr};
use core::ptr;
use proptest::prelude::*;
use quake_ctest as _;
use quake_render::heap::DeviceMemoryBackend;
use quake_render::texmgr::{
    Cvars, DescLayout, DescriptorWrite, Env, LoadedImage, Palettes, SourceRead, Staging, TexMgr,
    TexMgrBackend,
};
use quake_types::model_mem::QModel;
use quake_types::render::{
    GlTexture, VulkanMemory, VulkanMemoryType, TEXPREF_ALPHA, TEXPREF_MIPMAP, TEXPREF_PERSIST,
    TEXPREF_PREMULTIPLY,
};
use std::collections::HashMap;

extern "C" {
    fn c_ref_TexMgr_InitHeap();
    fn c_ref_TexMgr_Init();
    fn c_ref_TexMgr_LoadImage(
        owner: *mut c_void,
        name: *const c_char,
        width: c_int,
        height: c_int,
        format: c_int,
        data: *mut u8,
        source_file: *const c_char,
        source_offset: usize,
        flags: c_uint,
    ) -> *mut GlTexture;
    fn c_ref_TexMgr_ReloadImage(glt: *mut GlTexture, shirt: c_int, pants: c_int);
    fn c_ref_TexMgr_FreeTexture(kill: *mut GlTexture);
    fn c_ref_TexMgr_FreeTextures(flags: c_uint, mask: c_uint);
    fn c_ref_TexMgr_FreeTexturesForOwner(owner: *mut c_void);
    fn c_ref_TexMgr_DeleteTextureObjects();
    fn c_ref_TexMgr_CollectGarbage();
    fn c_ref_TexMgr_NewGame();
    fn c_ref_TexMgr_ReloadNobrightImages();
    fn c_ref_TexMgr_UpdateTextureDescriptorSets();
    fn c_ref_TexMgr_FindTexture(owner: *mut c_void, name: *const c_char) -> *mut GlTexture;

    fn c_ref_texmgr_trace() -> *const c_char;
    fn c_ref_texmgr_trace_reset();
    fn c_ref_texmgr_next_handle() -> u64;
    fn c_ref_texmgr_files_reset();
    fn c_ref_texmgr_add_file(name: *const c_char, data: *const u8, size: usize);
    fn c_ref_texmgr_add_image(
        name: *const c_char,
        width: c_int,
        height: c_int,
        format: c_int,
        data: *const u8,
        size: usize,
    );
    fn c_ref_texmgr_set_cvars(
        fullbrights: f32,
        filter: f32,
        anisotropic: f32,
        max_size: f32,
        picmip: f32,
    );
    fn c_ref_texmgr_set_env(
        max_2d: u32,
        max_cube: u32,
        color_format: c_int,
        point: u64,
        linear: u64,
        point_aniso: u64,
        linear_aniso: u64,
        warp_render_pass: u64,
    );
    fn c_ref_texmgr_set_in_update_screen(value: bool);
    fn c_ref_texmgr_num_textures() -> c_int;
    fn c_ref_texmgr_active_head() -> *mut GlTexture;
    fn ctest_texmgr_downsample(
        data: *mut u32,
        in_width: c_int,
        in_height: c_int,
        out_width: c_int,
        out_height: c_int,
    );

    static mut c_ref_heap_next_handle: u64;
    static mut c_ref_texmgr_notexture: *mut GlTexture;
    static mut c_ref_texmgr_d_8to24table: [u32; 256];
    static mut c_ref_texmgr_d_8to24table_fbright: [u32; 256];
    static mut c_ref_texmgr_d_8to24table_fbright_fence: [u32; 256];
    static mut c_ref_texmgr_d_8to24table_nobright: [u32; 256];
    static mut c_ref_texmgr_d_8to24table_nobright_fence: [u32; 256];
    static mut c_ref_texmgr_d_8to24table_conchars: [u32; 256];
}

const SRC_INDEXED: c_int = 0;
const SRC_LIGHTMAP: c_int = 1;
const SRC_RGBA: c_int = 2;
const SRC_SURF_INDICES: c_int = 3;
const SRC_RGBA_CUBEMAP: c_int = 4;
const SRC_INDEXED_PALETTE: c_int = 5;

const STAGING_ARENA_SIZE: usize = 8 * 1024 * 1024;

/// The M3 fake device-memory backend (`gl_heap_ref.c`'s twin).
#[derive(Default)]
struct FakeHeapBackend {
    next_handle: u64,
    live: u32,
}

impl DeviceMemoryBackend for FakeHeapBackend {
    type Counter = ();
    fn allocate(
        &mut self,
        memory: &mut VulkanMemory,
        size: u64,
        _memory_type_index: u32,
        memory_type: VulkanMemoryType,
        _device_address: bool,
        (): (),
    ) {
        self.next_handle += 1;
        self.live += 1;
        memory.handle = vk::DeviceMemory::from_raw(self.next_handle);
        memory.size = size as usize;
        memory.type_ = memory_type;
    }
    fn free(&mut self, memory: &mut VulkanMemory, (): ()) {
        self.live -= 1;
        *memory = VulkanMemory::default();
    }
}

struct FakeFile {
    data: Vec<u8>,
}

struct FakeImageFile {
    width: i32,
    height: i32,
    format: c_int,
    data: Vec<u8>,
}

/// The Rust twin of `gl_texmgr_ref.c`'s fake seams.
struct FakeBackend {
    trace: RefCell<Vec<String>>,
    next_handle: Cell<u64>,
    image_sizes: RefCell<HashMap<u64, u64>>,
    arena: *mut u8,
    arena_offset: Cell<i32>,
    staging_last: Cell<(i32, i32)>,
    env: Cell<Env>,
    cvars: Cell<Cvars>,
    in_update_screen: Cell<bool>,
    files: RefCell<HashMap<Vec<u8>, FakeFile>>,
    images: RefCell<HashMap<Vec<u8>, FakeImageFile>>,
    loaded: RefCell<HashMap<usize, usize>>,
}

impl FakeBackend {
    fn new() -> Self {
        // SAFETY: a valid non-zero layout; the arena lives for the process
        // like the C stub's static one.
        let arena = unsafe {
            std::alloc::alloc_zeroed(
                std::alloc::Layout::from_size_align(STAGING_ARENA_SIZE, 16).unwrap(),
            )
        };
        assert!(!arena.is_null());
        Self {
            trace: RefCell::new(Vec::new()),
            next_handle: Cell::new(0),
            image_sizes: RefCell::new(HashMap::new()),
            arena,
            arena_offset: Cell::new(0),
            staging_last: Cell::new((0, 0)),
            env: Cell::new(Env::default()),
            cvars: Cell::new(Cvars::default()),
            in_update_screen: Cell::new(false),
            files: RefCell::new(HashMap::new()),
            images: RefCell::new(HashMap::new()),
            loaded: RefCell::new(HashMap::new()),
        }
    }

    fn t(&self, line: String) {
        self.trace.borrow_mut().push(line);
    }

    fn take_trace(&self) -> String {
        let mut lines = core::mem::take(&mut *self.trace.borrow_mut());
        if lines.is_empty() {
            return String::new();
        }
        lines.push(String::new());
        lines.join("\n")
    }

    fn handle(&self) -> u64 {
        let h = self.next_handle.get() + 1;
        self.next_handle.set(h);
        h
    }

    fn add_file(&self, name: &CStr, data: &[u8]) {
        self.files.borrow_mut().insert(
            name.to_bytes().to_vec(),
            FakeFile {
                data: data.to_vec(),
            },
        );
    }

    fn add_image(&self, name: &CStr, width: i32, height: i32, format: c_int, data: &[u8]) {
        self.images.borrow_mut().insert(
            name.to_bytes().to_vec(),
            FakeImageFile {
                width,
                height,
                format,
                data: data.to_vec(),
            },
        );
    }

    fn files_reset(&self) {
        self.files.borrow_mut().clear();
        self.images.borrow_mut().clear();
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
    }
    h
}

fn range(r: &vk::ImageSubresourceRange) -> String {
    format!(
        "aspect={} mip={}/{} layer={}/{}",
        r.aspect_mask.as_raw(),
        r.base_mip_level,
        r.level_count,
        r.base_array_layer,
        r.layer_count
    )
}

impl TexMgrBackend for FakeBackend {
    type HeapBackend = FakeHeapBackend;

    fn new_heap_backend(&self) -> FakeHeapBackend {
        FakeHeapBackend::default()
    }
    fn heap_counter(&self) {}
    fn env(&self) -> Env {
        self.env.get()
    }
    fn cvars(&self) -> Cvars {
        self.cvars.get()
    }
    fn no_rendering(&self) -> bool {
        false
    }
    fn in_update_screen(&self) -> bool {
        self.in_update_screen.get()
    }

    fn create_image(&self, info: &vk::ImageCreateInfo<'_>) -> Result<vk::Image, i32> {
        let handle = self.handle();
        let mut bytes = u64::from(info.extent.width)
            * u64::from(info.extent.height)
            * u64::from(info.extent.depth)
            * 4
            * u64::from(info.array_layers);
        if info.mip_levels > 1 {
            bytes += bytes / 2;
        }
        bytes = (bytes + 255) & !255;
        self.image_sizes.borrow_mut().insert(handle, bytes);
        self.t(format!(
            "CreateImage flags={} type={} fmt={} {}x{}x{} mips={} layers={} samples={} tiling={} usage={} sharing={} qfi={} layout={} -> {}",
            info.flags.as_raw(),
            info.image_type.as_raw(),
            info.format.as_raw(),
            info.extent.width,
            info.extent.height,
            info.extent.depth,
            info.mip_levels,
            info.array_layers,
            info.samples.as_raw(),
            info.tiling.as_raw(),
            info.usage.as_raw(),
            info.sharing_mode.as_raw(),
            info.queue_family_index_count,
            info.initial_layout.as_raw(),
            handle
        ));
        Ok(vk::Image::from_raw(handle))
    }
    fn destroy_image(&self, image: vk::Image) {
        self.t(format!("DestroyImage {}", image.as_raw()));
    }
    fn image_memory_requirements(&self, image: vk::Image) -> vk::MemoryRequirements {
        let size = self
            .image_sizes
            .borrow()
            .get(&image.as_raw())
            .copied()
            .unwrap_or(0);
        self.t(format!(
            "ImageMemoryRequirements {} -> size={} align=256 bits=255",
            image.as_raw(),
            size
        ));
        vk::MemoryRequirements {
            size,
            alignment: 256,
            memory_type_bits: 0xff,
        }
    }
    fn bind_image_memory(
        &self,
        image: vk::Image,
        memory: vk::DeviceMemory,
        offset: u64,
    ) -> Result<(), i32> {
        self.t(format!(
            "BindImageMemory {} mem={} off={}",
            image.as_raw(),
            memory.as_raw(),
            offset
        ));
        Ok(())
    }
    fn create_image_view(&self, info: &vk::ImageViewCreateInfo<'_>) -> Result<vk::ImageView, i32> {
        let handle = self.handle();
        self.t(format!(
            "CreateImageView flags={} image={} type={} fmt={} swz={},{},{},{} {} -> {}",
            info.flags.as_raw(),
            info.image.as_raw(),
            info.view_type.as_raw(),
            info.format.as_raw(),
            info.components.r.as_raw(),
            info.components.g.as_raw(),
            info.components.b.as_raw(),
            info.components.a.as_raw(),
            range(&info.subresource_range),
            handle
        ));
        Ok(vk::ImageView::from_raw(handle))
    }
    fn destroy_image_view(&self, view: vk::ImageView) {
        self.t(format!("DestroyImageView {}", view.as_raw()));
    }
    fn create_framebuffer(
        &self,
        info: &vk::FramebufferCreateInfo<'_>,
    ) -> Result<vk::Framebuffer, i32> {
        let handle = self.handle();
        let first = if info.attachment_count > 0 && !info.p_attachments.is_null() {
            // SAFETY: the caller's attachment array has `attachment_count` entries.
            unsafe { (*info.p_attachments).as_raw() }
        } else {
            0
        };
        self.t(format!(
            "CreateFramebuffer flags={} rp={} att={}[{}] {}x{} layers={} -> {}",
            info.flags.as_raw(),
            info.render_pass.as_raw(),
            info.attachment_count,
            first,
            info.width,
            info.height,
            info.layers,
            handle
        ));
        Ok(vk::Framebuffer::from_raw(handle))
    }
    fn destroy_framebuffer(&self, framebuffer: vk::Framebuffer) {
        self.t(format!("DestroyFramebuffer {}", framebuffer.as_raw()));
    }
    fn update_descriptor_set(&self, write: &DescriptorWrite) {
        self.t(format!(
            "UpdateDescriptorSet set={} binding={} elem=0 count=1 type={} sampler={} view={} layout={} copies=0",
            write.set.as_raw(),
            write.binding,
            write.descriptor_type.as_raw(),
            write.sampler.as_raw(),
            write.image_view.as_raw(),
            write.image_layout.as_raw()
        ));
    }
    fn allocate_descriptor_set(&self, layout: DescLayout) -> vk::DescriptorSet {
        let handle = self.handle();
        let which = match layout {
            DescLayout::SingleTexture => "single",
            DescLayout::SingleTextureCsWrite => "cs_write",
        };
        self.t(format!("AllocateDescriptorSet {which} -> {handle}"));
        vk::DescriptorSet::from_raw(handle)
    }
    fn free_descriptor_set(&self, set: vk::DescriptorSet, layout: DescLayout) {
        let which = match layout {
            DescLayout::SingleTexture => "single",
            DescLayout::SingleTextureCsWrite => "cs_write",
        };
        self.t(format!("FreeDescriptorSet {} {which}", set.as_raw()));
    }
    fn set_object_name(&self, object: u64, object_type: vk::ObjectType, name: &str) {
        self.t(format!(
            "SetObjectName {object} type={} '{name}'",
            object_type.as_raw()
        ));
    }
    fn memory_type_from_properties(
        &self,
        type_bits: u32,
        required: vk::MemoryPropertyFlags,
        preferred: vk::MemoryPropertyFlags,
    ) -> u32 {
        self.t(format!(
            "MemoryType bits={type_bits} req={} pref={} -> 3",
            required.as_raw(),
            preferred.as_raw()
        ));
        3
    }
    fn wait_for_device_idle(&self) {
        self.t("WaitForDeviceIdle".to_string());
    }
    fn staging_allocate(&self, size: i32, alignment: i32) -> Staging {
        let mut off = (self.arena_offset.get() + alignment - 1) / alignment * alignment;
        if off as usize + size as usize > STAGING_ARENA_SIZE {
            off = 0;
        }
        self.arena_offset.set(off + size);
        self.staging_last.set((off, size));
        self.t(format!(
            "StagingAllocate size={size} align={alignment} -> off={off}"
        ));
        Staging {
            // SAFETY: `off + size` is inside the arena.
            memory: unsafe { self.arena.add(off as usize) },
            command_buffer: vk::CommandBuffer::from_raw(0x1000),
            buffer: vk::Buffer::from_raw(0x2000),
            offset: off,
        }
    }
    fn staging_begin_copy(&self) {
        self.t("StagingBeginCopy".to_string());
    }
    fn staging_end_copy(&self) {
        let (off, size) = self.staging_last.get();
        // SAFETY: the last allocation's range is inside the arena and the
        // manager has finished writing it.
        let bytes =
            unsafe { core::slice::from_raw_parts(self.arena.add(off as usize), size as usize) };
        self.t(format!("StagingEndCopy hash={:016x}", fnv1a(bytes)));
    }
    fn cmd_pipeline_barrier(
        &self,
        cb: vk::CommandBuffer,
        src: vk::PipelineStageFlags,
        dst: vk::PipelineStageFlags,
        barrier: &vk::ImageMemoryBarrier<'_>,
    ) {
        self.t(format!(
            "Barrier cb={} src={} dst={} dep=0 counts=0/0/1",
            cb.as_raw(),
            src.as_raw(),
            dst.as_raw()
        ));
        self.t(format!(
            "  image access={}->{} layout={}->{} qf={}->{} image={} {}",
            barrier.src_access_mask.as_raw(),
            barrier.dst_access_mask.as_raw(),
            barrier.old_layout.as_raw(),
            barrier.new_layout.as_raw(),
            barrier.src_queue_family_index,
            barrier.dst_queue_family_index,
            barrier.image.as_raw(),
            range(&barrier.subresource_range)
        ));
    }
    fn cmd_copy_buffer_to_image(
        &self,
        cb: vk::CommandBuffer,
        buffer: vk::Buffer,
        image: vk::Image,
        layout: vk::ImageLayout,
        regions: &[vk::BufferImageCopy],
    ) {
        self.t(format!(
            "CopyBufferToImage cb={} buf={} image={} layout={} regions={}",
            cb.as_raw(),
            buffer.as_raw(),
            image.as_raw(),
            layout.as_raw(),
            regions.len()
        ));
        for r in regions {
            self.t(format!(
                "  region off={} rowlen={} imgh={} aspect={} mip={} layer={}/{} offset={},{},{} extent={}x{}x{}",
                r.buffer_offset,
                r.buffer_row_length,
                r.buffer_image_height,
                r.image_subresource.aspect_mask.as_raw(),
                r.image_subresource.mip_level,
                r.image_subresource.base_array_layer,
                r.image_subresource.layer_count,
                r.image_offset.x,
                r.image_offset.y,
                r.image_offset.z,
                r.image_extent.width,
                r.image_extent.height,
                r.image_extent.depth
            ));
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
        // SAFETY: per the trait contract (`data` holds `in_width * in_height`
        // RGBA8 pixels); the C helper is the oracle's own resampler.
        unsafe { ctest_texmgr_downsample(data, in_width, in_height, out_width, out_height) }
    }
    fn read_source(&self, name: &CStr, offset: usize, size: usize) -> SourceRead {
        let files = self.files.borrow();
        let Some(file) = files.get(name.to_bytes()) else {
            self.t(format!("FOpen '{}' -> none", name.to_string_lossy()));
            return SourceRead::NotFound;
        };
        self.t(format!("FOpen '{}' -> found", name.to_string_lossy()));
        let mut pos = 0usize;
        if offset != 0 {
            pos += offset;
            self.t(format!("Seek {offset}"));
        }
        let avail = file.data.len().saturating_sub(pos);
        let got = size.min(avail);
        self.t(format!("Read {size} -> {got}"));
        if got != size {
            // the C `goto invalid`s past the fclose
            return SourceRead::Short;
        }
        self.t("Close".to_string());
        SourceRead::Ok(file.data[pos..pos + got].to_vec())
    }
    fn image_load(&self, name: &CStr, min_path_id: u32) -> Option<LoadedImage> {
        let images = self.images.borrow();
        let Some(img) = images.get(name.to_bytes()) else {
            self.t(format!(
                "ImageLoad '{}' path={min_path_id} -> none",
                name.to_string_lossy()
            ));
            return None;
        };
        let len = img.data.len().max(1);
        let mut buf = vec![0u8; len].into_boxed_slice();
        buf[..img.data.len()].copy_from_slice(&img.data);
        let data = Box::into_raw(buf).cast::<u8>();
        self.loaded.borrow_mut().insert(data as usize, len);
        self.t(format!(
            "ImageLoad '{}' path={min_path_id} -> {}x{} fmt={}",
            name.to_string_lossy(),
            img.width,
            img.height,
            img.format
        ));
        Some(LoadedImage {
            data,
            width: img.width,
            height: img.height,
            format: img.format,
        })
    }
    unsafe fn free_loaded(&self, data: *mut u8) {
        let len = self
            .loaded
            .borrow_mut()
            .remove(&(data as usize))
            .expect("free_loaded of an unknown buffer");
        // SAFETY: `data` came from `Box::into_raw` of a `len`-byte slice in
        // `image_load` and is freed once (the map entry is gone).
        unsafe { drop(Box::from_raw(ptr::slice_from_raw_parts_mut(data, len))) };
    }
    unsafe fn owner_path_id(&self, owner: *mut c_void) -> u32 {
        // SAFETY: the differential only passes its own `QModel` buffers.
        unsafe { (*owner.cast::<QModel>()).path_id }
    }
    fn con_printf(&self, msg: &str) {
        self.t(format!("Con: {msg}"));
    }
    fn sys_error(&self, msg: &str) -> ! {
        panic!("Sys_Error: {msg}");
    }
}

// ---------------------------------------------------------------------------

static C_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const PALETTE_FILE: &CStr = c"gfx/palette.lmp";
const BLOB_FILE: &CStr = c"fake/blob.lmp";
const IMAGE_FILES: [&CStr; 2] = [c"fake/img0", c"fake/img1"];
const MISSING_IMAGE: &CStr = c"fake/missing";
const NAMES: [&CStr; 6] = [c"tex0", c"tex1", c"tex2", c"tex3", c"greytexture", c"a/b_c"];

fn palette_lump() -> [u8; 768] {
    let mut pal = [0u8; 768];
    for (i, b) in pal.iter_mut().enumerate() {
        *b = ((i * 7) ^ (i >> 3)) as u8;
    }
    pal
}

/// Owner models: two `qmodel_t` buffers with distinct `path_id`s, shared by
/// both sides (they are only ever read through `->path_id`).
struct Owners {
    models: Box<[QModel; 2]>,
}

impl Owners {
    fn new() -> Self {
        // SAFETY: `QModel` is a plain `repr(C)` aggregate for which all-zero
        // bytes are a valid value.
        let mut models: Box<[QModel; 2]> = unsafe { Box::new(core::mem::zeroed()) };
        models[0].path_id = 7;
        models[1].path_id = 9;
        Self { models }
    }
    fn ptr(&mut self, which: u8) -> *mut c_void {
        match which {
            0 => ptr::null_mut(),
            1 => ptr::addr_of_mut!(self.models[0]).cast(),
            _ => ptr::addr_of_mut!(self.models[1]).cast(),
        }
    }
    fn tag(&self, owner: *mut c_void) -> u8 {
        if owner.is_null() {
            0
        } else if owner == ptr::addr_of!(self.models[0]).cast_mut().cast() {
            1
        } else if owner == ptr::addr_of!(self.models[1]).cast_mut().cast() {
            2
        } else {
            u8::MAX
        }
    }
}

struct Sides {
    backend: FakeBackend,
    rust: TexMgr<FakeBackend>,
    palettes: Palettes,
    owners: Owners,
    /// Every pixel buffer handed to either side stays alive for the case,
    /// since memory-sourced textures reload from it.
    keep: Vec<Vec<u8>>,
}

// SAFETY: `Sides` holds raw pointers (the staging arena, the owner models and
// the C-side texture list) that are only ever touched by the thread holding
// `C_LOCK`; the static mutex below merely parks the value between cases.
unsafe impl Send for Sides {}

fn cstr_field(bytes: &[u8]) -> Vec<u8> {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    bytes[..end].to_vec()
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    name: Vec<u8>,
    owner: u8,
    path_id: u32,
    width: u32,
    height: u32,
    flags: u32,
    source_file: Vec<u8>,
    source_offset: Option<usize>,
    source_format: c_int,
    source_width: u32,
    source_height: u32,
    source_crc: u16,
    shirt: i8,
    pants: i8,
    image: u64,
    image_view: u64,
    target_image_view: u64,
    descriptor_set: u64,
    frame_buffer: u64,
    storage_descriptor_set: u64,
}

fn snapshot(owners: &Owners, glt: *mut GlTexture) -> Snapshot {
    // SAFETY: an active-list texture of either arena.
    let t = unsafe { &*glt };
    let source_file = cstr_field(&t.source_file);
    Snapshot {
        name: cstr_field(&t.name),
        owner: owners.tag(t.owner),
        path_id: t.path_id,
        width: t.width,
        height: t.height,
        flags: t.flags,
        source_offset: if source_file.is_empty() {
            None
        } else {
            Some(t.source_offset)
        },
        source_file,
        source_format: t.source_format,
        source_width: t.source_width,
        source_height: t.source_height,
        source_crc: t.source_crc,
        shirt: t.shirt,
        pants: t.pants,
        image: t.image.as_raw(),
        image_view: t.image_view.as_raw(),
        target_image_view: t.target_image_view.as_raw(),
        descriptor_set: t.descriptor_set.as_raw(),
        frame_buffer: t.frame_buffer.as_raw(),
        storage_descriptor_set: t.storage_descriptor_set.as_raw(),
    }
}

fn c_active_list() -> Vec<*mut GlTexture> {
    let mut out = Vec::new();
    // SAFETY: under C_LOCK; the list is the oracle's arena.
    let mut glt = unsafe { c_ref_texmgr_active_head() };
    while !glt.is_null() {
        out.push(glt);
        // SAFETY: arena pointer.
        glt = unsafe { (*glt).next };
    }
    out
}

fn c_trace() -> String {
    // SAFETY: under C_LOCK; the stub returns a NUL-terminated buffer.
    let s = unsafe { CStr::from_ptr(c_ref_texmgr_trace()) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: as above.
    unsafe { c_ref_texmgr_trace_reset() };
    s
}

fn c_palettes() -> Palettes {
    // SAFETY: under C_LOCK; the tables are the oracle's globals.
    unsafe {
        Palettes {
            table: *ptr::addr_of!(c_ref_texmgr_d_8to24table),
            fbright: *ptr::addr_of!(c_ref_texmgr_d_8to24table_fbright),
            fbright_fence: *ptr::addr_of!(c_ref_texmgr_d_8to24table_fbright_fence),
            nobright: *ptr::addr_of!(c_ref_texmgr_d_8to24table_nobright),
            nobright_fence: *ptr::addr_of!(c_ref_texmgr_d_8to24table_nobright_fence),
            conchars: *ptr::addr_of!(c_ref_texmgr_d_8to24table_conchars),
        }
    }
}

const ENV_SAMPLERS: [u64; 4] = [0x101, 0x102, 0x103, 0x104];
const ENV_WARP_RP: u64 = 0x200;

fn env_of(max_2d: u32, max_cube: u32, color_format: i32) -> Env {
    Env {
        max_image_dimension_2d: max_2d,
        max_image_dimension_cube: max_cube,
        color_format: vk::Format::from_raw(color_format),
        point_sampler_lod_bias: vk::Sampler::from_raw(ENV_SAMPLERS[0]),
        linear_sampler_lod_bias: vk::Sampler::from_raw(ENV_SAMPLERS[1]),
        point_aniso_sampler_lod_bias: vk::Sampler::from_raw(ENV_SAMPLERS[2]),
        linear_aniso_sampler_lod_bias: vk::Sampler::from_raw(ENV_SAMPLERS[3]),
        warp_render_pass: vk::RenderPass::from_raw(ENV_WARP_RP),
    }
}

impl Sides {
    fn set_env(&self, max_2d: u32, max_cube: u32, color_format: i32) {
        self.backend.env.set(env_of(max_2d, max_cube, color_format));
        // SAFETY: under C_LOCK.
        unsafe {
            c_ref_texmgr_set_env(
                max_2d,
                max_cube,
                color_format,
                ENV_SAMPLERS[0],
                ENV_SAMPLERS[1],
                ENV_SAMPLERS[2],
                ENV_SAMPLERS[3],
                ENV_WARP_RP,
            )
        };
    }

    fn set_cvars(&self, c: Cvars) {
        self.backend.cvars.set(c);
        // SAFETY: under C_LOCK.
        unsafe {
            c_ref_texmgr_set_cvars(
                c.gl_fullbrights,
                c.vid_filter,
                c.vid_anisotropic,
                c.gl_max_size,
                c.gl_picmip,
            )
        };
    }

    fn add_file(&self, name: &CStr, data: &[u8]) {
        self.backend.add_file(name, data);
        // SAFETY: under C_LOCK; the stub copies the bytes.
        unsafe { c_ref_texmgr_add_file(name.as_ptr(), data.as_ptr(), data.len()) };
    }

    fn add_image(&self, name: &CStr, width: i32, height: i32, format: c_int, data: &[u8]) {
        self.backend.add_image(name, width, height, format, data);
        // SAFETY: as above.
        unsafe {
            c_ref_texmgr_add_image(
                name.as_ptr(),
                width,
                height,
                format,
                data.as_ptr(),
                data.len(),
            )
        };
    }

    fn files_reset(&self) {
        self.backend.files_reset();
        // SAFETY: under C_LOCK.
        unsafe { c_ref_texmgr_files_reset() };
    }

    /// Both traces, then both active lists field by field.
    fn check(&self, what: &str) -> Result<(), TestCaseError> {
        let c = c_trace();
        let r = self.backend.take_trace();
        if c != r {
            let (cl, rl): (Vec<&str>, Vec<&str>) = (c.lines().collect(), r.lines().collect());
            let first = cl
                .iter()
                .zip(rl.iter())
                .position(|(a, b)| a != b)
                .unwrap_or(cl.len().min(rl.len()));
            let lo = first.saturating_sub(3);
            return Err(TestCaseError::fail(format!(
                "trace mismatch after {what} at line {first} (C {} lines, Rust {} lines)\n--- C ---\n{}\n--- Rust ---\n{}",
                cl.len(),
                rl.len(),
                cl[lo..(first + 4).min(cl.len())].join("\n"),
                rl[lo..(first + 4).min(rl.len())].join("\n")
            )));
        }
        let cl = c_active_list();
        let rl: Vec<_> = self.rust.active_textures().collect();
        prop_assert_eq!(cl.len(), rl.len(), "active list length after {}", what);
        // SAFETY: under C_LOCK.
        let c_num = unsafe { c_ref_texmgr_num_textures() };
        prop_assert_eq!(c_num, self.rust.num_textures());
        for (i, (c, r)) in cl.iter().zip(rl.iter()).enumerate() {
            let (cs, rs) = (snapshot(&self.owners, *c), snapshot(&self.owners, *r));
            prop_assert_eq!(cs, rs, "texture {} after {}", i, what);
        }
        // SAFETY: under C_LOCK.
        let c_handle = unsafe { c_ref_texmgr_next_handle() };
        prop_assert_eq!(c_handle, self.backend.next_handle.get());
        Ok(())
    }

    fn pairs(&self) -> Vec<(*mut GlTexture, *mut GlTexture)> {
        c_active_list()
            .into_iter()
            .zip(self.rust.active_textures())
            .collect()
    }
}

/// One-time init of both sides; the C oracle is process-global.
fn init() -> Sides {
    // SAFETY: under C_LOCK (the caller holds it); nothing else touches the
    // oracle's heap counter.
    unsafe { c_ref_heap_next_handle = 0 };
    let backend = FakeBackend::new();
    let mut sides = Sides {
        backend,
        rust: TexMgr::new(),
        palettes: Palettes::default(),
        owners: Owners::new(),
        keep: Vec::new(),
    };
    sides.set_env(4096, 4096, 37);
    sides.set_cvars(Cvars {
        gl_fullbrights: 1.0,
        vid_filter: 1.0,
        vid_anisotropic: 0.0,
        gl_max_size: 0.0,
        gl_picmip: 0.0,
    });
    sides.add_file(PALETTE_FILE, &palette_lump());
    // SAFETY: under C_LOCK; the stub's globals are set up above.
    unsafe {
        c_ref_TexMgr_InitHeap();
        c_ref_TexMgr_Init();
    }
    sides.rust.init_heap(&sides.backend);
    sides.rust.init_list();
    sides.palettes = TexMgr::<FakeBackend>::load_palette(&sides.backend);
    let refs = sides.palettes.refs();
    let builtins = sides.rust.load_builtin_textures(&sides.backend, &refs);
    assert!(!builtins.notexture.is_null());
    // SAFETY: under C_LOCK.
    assert!(unsafe { !c_ref_texmgr_notexture.is_null() });
    assert_eq!(c_palettes().table, sides.palettes.table);
    assert_eq!(c_palettes().conchars, sides.palettes.conchars);
    sides.check("init").expect("init traces differ");
    sides
}

fn sides() -> &'static std::sync::Mutex<Option<Sides>> {
    static SIDES: std::sync::Mutex<Option<Sides>> = std::sync::Mutex::new(None);
    &SIDES
}

// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Source {
    Memory,
    /// `source_file` + non-zero `source_offset` into the fake blob.
    Lump {
        offset: u16,
    },
    /// `source_file` naming a fake image (0/1) or a missing one (2).
    Image {
        which: u8,
    },
}

#[derive(Clone, Debug)]
enum Op {
    Load {
        owner: u8,
        name: u8,
        width: i32,
        height: i32,
        format: c_int,
        flags: u32,
        source: Source,
        seed: u64,
    },
    Reload {
        index: usize,
        shirt: i32,
        pants: i32,
    },
    Free {
        index: usize,
    },
    FreeNull,
    FreeFlags {
        flags: u32,
        mask: u32,
    },
    FreeOwner {
        owner: u8,
    },
    Collect,
    DeleteObjects,
    NewGame,
    ReloadNobright,
    SetCvars(Cvars),
    SetEnv {
        max_2d: u32,
        max_cube: u32,
        color_format: i32,
    },
    UpdateDescriptorSets,
    InUpdateScreen(bool),
    Find {
        owner: u8,
        name: u8,
    },
}

fn dims(format: c_int) -> impl Strategy<Value = (i32, i32)> {
    if format == SRC_INDEXED_PALETTE {
        // the miptex chain layout needs `w*h/64*85` to cover four mips
        (
            prop_oneof![Just(8), Just(16), Just(24), Just(32)],
            prop_oneof![Just(8), Just(16), Just(32)],
        )
            .boxed()
    } else {
        (1i32..=70, 1i32..=70).boxed()
    }
}

fn op_strategy() -> impl Strategy<Value = Op> {
    let format = prop_oneof![
        3 => Just(SRC_INDEXED),
        1 => Just(SRC_LIGHTMAP),
        4 => Just(SRC_RGBA),
        1 => Just(SRC_SURF_INDICES),
        1 => Just(SRC_RGBA_CUBEMAP),
        2 => Just(SRC_INDEXED_PALETTE),
    ];
    let source = prop_oneof![
        5 => Just(Source::Memory),
        2 => (1u16..=60000).prop_map(|offset| Source::Lump { offset }),
        2 => (0u8..3).prop_map(|which| Source::Image { which }),
    ];
    let cvars = (
        prop_oneof![Just(0.0f32), Just(1.0)],
        prop_oneof![Just(0.0f32), Just(1.0), Just(2.0), Just(3.0)],
        prop_oneof![Just(0.0f32), Just(1.0)],
        prop_oneof![
            Just(0.0f32),
            Just(8.0),
            Just(16.0),
            Just(64.0),
            Just(1024.0)
        ],
        prop_oneof![Just(0.0f32), Just(1.0), Just(2.0)],
    )
        .prop_map(|(f, vf, va, ms, pm)| Cvars {
            gl_fullbrights: f,
            vid_filter: vf,
            vid_anisotropic: va,
            gl_max_size: ms,
            gl_picmip: pm,
        });
    prop_oneof![
        12 => (0u8..3, 0u8..6, format, 0u32..0x4000, source, any::<u64>()).prop_flat_map(
            |(owner, name, format, flags, source, seed)| {
                dims(format).prop_map(move |(width, height)| Op::Load {
                    owner,
                    name,
                    width,
                    height,
                    format,
                    flags,
                    source,
                    seed,
                })
            }
        ),
        4 => (0usize..64, -1i32..=15, -1i32..=15)
            .prop_map(|(index, shirt, pants)| Op::Reload { index, shirt, pants }),
        3 => (0usize..64).prop_map(|index| Op::Free { index }),
        1 => Just(Op::FreeNull),
        1 => (0u32..0x4000, 0u32..0x4000).prop_map(|(flags, mask)| Op::FreeFlags { flags, mask }),
        1 => (0u8..3).prop_map(|owner| Op::FreeOwner { owner }),
        3 => Just(Op::Collect),
        1 => Just(Op::DeleteObjects),
        1 => Just(Op::NewGame),
        1 => Just(Op::ReloadNobright),
        2 => cvars.prop_map(Op::SetCvars),
        1 => (
            prop_oneof![Just(4096u32), Just(32), Just(16)],
            prop_oneof![Just(4096u32), Just(32)],
            prop_oneof![Just(37i32), Just(64)]
        )
            .prop_map(|(max_2d, max_cube, color_format)| Op::SetEnv { max_2d, max_cube, color_format }),
        1 => Just(Op::UpdateDescriptorSets),
        1 => any::<bool>().prop_map(Op::InUpdateScreen),
        1 => (0u8..3, 0u8..6).prop_map(|(owner, name)| Op::Find { owner, name }),
    ]
}

/// xorshift bytes, so both sides see identical pixels for a seed.
fn pixels(seed: u64, len: usize) -> Vec<u8> {
    let mut s = seed | 1;
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        out.extend_from_slice(&s.to_le_bytes());
    }
    out.truncate(len);
    out
}

/// The bytes a `TexMgr_LoadImage` of this shape reads.
fn source_bytes(format: c_int, width: i32, height: i32, seed: u64) -> Vec<u8> {
    let n = (width * height) as usize;
    match format {
        SRC_INDEXED => pixels(seed, n),
        SRC_LIGHTMAP | SRC_RGBA | SRC_SURF_INDICES => pixels(seed, n * 4),
        SRC_RGBA_CUBEMAP => pixels(seed, n * 4 * 6),
        _ => {
            // miptex: four mip levels of indices, u16 colour count, palette
            let colors = 1 + (seed % 256) as usize;
            let mut v = pixels(seed, n / 64 * 85);
            for b in &mut v {
                *b = (*b as usize % colors) as u8;
            }
            v.extend_from_slice(&(colors as u16).to_le_bytes());
            v.extend(pixels(seed ^ 0x5555, colors * 3));
            v
        }
    }
}

/// Reloads read `source_width * source_height * (4 | 1)` bytes from a lump,
/// so lump sources are limited to the formats whose upload reads no more.
fn lump_ok(format: c_int) -> bool {
    matches!(format, SRC_INDEXED | SRC_LIGHTMAP | SRC_RGBA)
}

fn blob() -> Vec<u8> {
    pixels(0xb10b, 48 * 1024)
}

fn image_files() -> [(i32, i32, c_int, Vec<u8>); 2] {
    [
        (12, 9, SRC_RGBA, pixels(0x1a, 12 * 9 * 4)),
        (16, 16, SRC_INDEXED, pixels(0x1b, 16 * 16)),
    ]
}

fn reset_case(s: &mut Sides) -> Result<(), TestCaseError> {
    s.backend.in_update_screen.set(false);
    // SAFETY: under C_LOCK.
    unsafe { c_ref_texmgr_set_in_update_screen(false) };
    s.files_reset();
    s.add_file(PALETTE_FILE, &palette_lump());
    s.add_file(BLOB_FILE, &blob());
    for (name, (w, h, f, data)) in IMAGE_FILES.iter().zip(image_files()) {
        s.add_image(name, w, h, f, &data);
    }
    s.set_env(4096, 4096, 37);
    s.set_cvars(Cvars {
        gl_fullbrights: 1.0,
        vid_filter: 1.0,
        vid_anisotropic: 0.0,
        gl_max_size: 0.0,
        gl_picmip: 0.0,
    });
    // SAFETY: under C_LOCK.
    unsafe {
        c_ref_TexMgr_NewGame();
        c_ref_TexMgr_CollectGarbage();
        c_ref_TexMgr_CollectGarbage();
    }
    s.rust.new_game_free(&s.backend);
    s.palettes = TexMgr::<FakeBackend>::load_palette(&s.backend);
    s.rust.collect_garbage(&s.backend);
    s.rust.collect_garbage(&s.backend);
    s.keep.clear();
    s.check("reset")
}

fn run_op(s: &mut Sides, op: &Op) -> Result<(), TestCaseError> {
    if std::env::var_os("TEXMGR_TRACE_OPS").is_some() {
        eprintln!("{op:?}");
    }
    let refs = s.palettes.refs();
    match *op {
        Op::Load {
            owner,
            name,
            width,
            height,
            format,
            flags,
            source,
            seed,
        } => {
            let owner = s.owners.ptr(owner);
            let name = NAMES[name as usize];
            // A cubemap's `data` is six face pointers (gl_sky.c:499), so a
            // lump or image source (a flat byte array) would be read as
            // pointers on both sides; gl_sky.c only ever loads it with
            // TEXPREF_NONE, and the C treats the pointer table as pixels for
            // PREMULTIPLY and the ALPHA edge fix (writing past it) and reads
            // past its 16-entry region array for MIPMAP.
            let is_cube = format == SRC_RGBA_CUBEMAP;
            let flags = if is_cube {
                flags & !(TEXPREF_MIPMAP | TEXPREF_ALPHA | TEXPREF_PREMULTIPLY)
            } else {
                flags
            };
            let source = match source {
                Source::Lump { .. } if !lump_ok(format) => Source::Memory,
                Source::Image { .. } if is_cube => Source::Memory,
                other => other,
            };
            // `keep` is emptied between cases while a PERSIST texture
            // survives the reset's `NewGame`, so a memory source must not
            // outlive its bytes (an OVERWRITE hit rewrites a builtin's flags
            // too); file sources reload from the per-case fake filesystem.
            let flags = if matches!(source, Source::Memory) {
                flags & !TEXPREF_PERSIST
            } else {
                flags
            };
            let (source_file, source_offset, base): (&CStr, usize, Vec<u8>) = match source {
                Source::Memory => (c"", 0, source_bytes(format, width, height, seed)),
                Source::Lump { offset } => {
                    let b = blob();
                    let n = source_bytes(format, width, height, seed).len();
                    let off = offset as usize;
                    let data = if off + n <= b.len() {
                        b[off..off + n].to_vec()
                    } else {
                        source_bytes(format, width, height, seed)
                    };
                    (BLOB_FILE, off, data)
                }
                Source::Image { which } => {
                    let file = if which < 2 {
                        IMAGE_FILES[which as usize]
                    } else {
                        MISSING_IMAGE
                    };
                    (file, 0, source_bytes(format, width, height, seed))
                }
            };
            // one private copy per side: the upload may rewrite the pixels
            // in place and a memory source reloads from its own copy
            s.keep.push(base.clone());
            s.keep.push(base);
            let n = s.keep.len();
            let mut c_data = s.keep[n - 2].as_mut_ptr();
            let mut r_data = s.keep[n - 1].as_mut_ptr();
            if is_cube {
                let face = (width * height * 4) as usize;
                let table = |buf: *mut u8| -> Vec<u8> {
                    (0..6)
                        .flat_map(|i| (buf as usize + i * face).to_ne_bytes())
                        .collect()
                };
                s.keep.push(table(c_data));
                s.keep.push(table(r_data));
                let n = s.keep.len();
                c_data = s.keep[n - 2].as_mut_ptr();
                r_data = s.keep[n - 1].as_mut_ptr();
            }
            let (c_off, r_off) = if source_file.is_empty() {
                (c_data as usize, r_data as usize)
            } else {
                (source_offset, source_offset)
            };
            // SAFETY: under C_LOCK; the buffers outlive the case.
            let c = unsafe {
                c_ref_TexMgr_LoadImage(
                    owner,
                    name.as_ptr(),
                    width,
                    height,
                    format,
                    c_data,
                    source_file.as_ptr(),
                    c_off,
                    flags,
                )
            };
            // SAFETY: as above.
            let r = unsafe {
                s.rust.load_image(
                    &s.backend,
                    &refs,
                    owner,
                    name,
                    width,
                    height,
                    format,
                    r_data,
                    source_file,
                    r_off,
                    flags,
                )
            };
            prop_assert_eq!(c.is_null(), r.is_null(), "load result nullness");
        }
        Op::Reload {
            index,
            shirt,
            pants,
        } => {
            let pairs = s.pairs();
            if pairs.is_empty() {
                return Ok(());
            }
            let (c, r) = pairs[index % pairs.len()];
            // the C `bluenoise` records a 16-byte array as a 64x64 memory
            // source: reloading it reads out of bounds on both sides
            // SAFETY: arena pointer.
            if cstr_field(unsafe { &(*c).name }) == b"bluenoise" {
                return Ok(());
            }
            // SAFETY: under C_LOCK; both textures are live and their sources
            // are kept alive by the case.
            unsafe {
                c_ref_TexMgr_ReloadImage(c, shirt, pants);
                s.rust.reload_image(&s.backend, &refs, r, shirt, pants);
            }
        }
        Op::Free { index } => {
            let pairs = s.pairs();
            if pairs.is_empty() {
                return Ok(());
            }
            let (c, r) = pairs[index % pairs.len()];
            // SAFETY: under C_LOCK; both are on their active lists.
            unsafe {
                c_ref_TexMgr_FreeTexture(c);
                s.rust.free_texture(&s.backend, r);
            }
        }
        Op::FreeNull => {
            // SAFETY: under C_LOCK; NULL is the documented no-op input.
            unsafe {
                c_ref_TexMgr_FreeTexture(ptr::null_mut());
                s.rust.free_texture(&s.backend, ptr::null_mut());
            }
        }
        Op::FreeFlags { flags, mask } => {
            // keep the builtins for the rest of the case
            let mask = mask | TEXPREF_PERSIST;
            let flags = flags & !TEXPREF_PERSIST;
            // SAFETY: under C_LOCK.
            unsafe { c_ref_TexMgr_FreeTextures(flags, mask) };
            s.rust.free_textures(&s.backend, flags, mask);
        }
        Op::FreeOwner { owner } => {
            let owner = s.owners.ptr(owner);
            if owner.is_null() {
                // the builtins are NULL-owned; freeing them would leave the
                // C `notexture` dangling for the rest of the process
                return Ok(());
            }
            // SAFETY: under C_LOCK.
            unsafe { c_ref_TexMgr_FreeTexturesForOwner(owner) };
            s.rust.free_textures_for_owner(&s.backend, owner);
        }
        Op::Collect => {
            // SAFETY: under C_LOCK.
            unsafe { c_ref_TexMgr_CollectGarbage() };
            s.rust.collect_garbage(&s.backend);
        }
        Op::DeleteObjects => {
            // SAFETY: under C_LOCK.
            unsafe { c_ref_TexMgr_DeleteTextureObjects() };
            s.rust.delete_texture_objects(&s.backend);
        }
        Op::NewGame => {
            // SAFETY: under C_LOCK.
            unsafe { c_ref_TexMgr_NewGame() };
            s.rust.new_game_free(&s.backend);
            s.palettes = TexMgr::<FakeBackend>::load_palette(&s.backend);
            prop_assert_eq!(c_palettes().nobright, s.palettes.nobright);
        }
        Op::ReloadNobright => {
            // SAFETY: under C_LOCK.
            unsafe { c_ref_TexMgr_ReloadNobrightImages() };
            s.rust.reload_nobright_images(&s.backend, &refs);
        }
        Op::SetCvars(c) => s.set_cvars(c),
        Op::SetEnv {
            max_2d,
            max_cube,
            color_format,
        } => s.set_env(max_2d, max_cube, color_format),
        Op::UpdateDescriptorSets => {
            // SAFETY: under C_LOCK.
            unsafe { c_ref_TexMgr_UpdateTextureDescriptorSets() };
            s.rust.update_texture_descriptor_sets(&s.backend);
        }
        Op::InUpdateScreen(v) => {
            s.backend.in_update_screen.set(v);
            // SAFETY: under C_LOCK.
            unsafe { c_ref_texmgr_set_in_update_screen(v) };
        }
        Op::Find { owner, name } => {
            let owner = s.owners.ptr(owner);
            let name = NAMES[name as usize];
            // SAFETY: under C_LOCK.
            let c = unsafe { c_ref_TexMgr_FindTexture(owner, name.as_ptr()) };
            let r = s.rust.find_texture(owner, Some(name));
            prop_assert_eq!(c.is_null(), r.is_null(), "find nullness");
            if !c.is_null() {
                prop_assert_eq!(snapshot(&s.owners, c), snapshot(&s.owners, r));
            }
            prop_assert!(s.rust.find_texture(owner, None).is_null());
        }
    }
    s.check(&format!("{op:?}"))
}

fn with_sides(
    f: impl FnOnce(&mut Sides) -> Result<(), TestCaseError>,
) -> Result<(), TestCaseError> {
    let _guard = C_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut slot = sides().lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_none() {
        *slot = Some(init());
    }
    let s = slot.as_mut().expect("initialised");
    if std::env::var_os("TEXMGR_TRACE_OPS").is_some() {
        eprintln!("--- case ---");
    }
    reset_case(s)?;
    let result = f(s);
    if let (Err(e), Some(_)) = (&result, std::env::var_os("TEXMGR_TRACE_OPS")) {
        eprintln!("case failed: {e}");
    }
    result
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: if cfg!(debug_assertions) { 24 } else { 200 },
        max_shrink_iters: 200,
        ..ProptestConfig::default()
    })]

    #[test]
    fn texmgr_matches_c(ops in prop::collection::vec(op_strategy(), 1..40)) {
        with_sides(|s| {
            for op in &ops {
                run_op(s, op)?;
            }
            Ok(())
        })?;
    }
}

/// The scripted path every map load takes: mip-mapped world textures, a
/// fullbright/nobright pair, a warp image, lightmaps, a cube map, overwrite
/// hits and misses, reload with colours, garbage collection across
/// `in_update_screen`, and a `NewGame`.
#[test]
fn texmgr_scripted() {
    let a = |o| Op::Load {
        owner: o,
        name: 0,
        width: 64,
        height: 32,
        format: SRC_INDEXED,
        flags: 0x1 | 0x8 | 0x100,
        source: Source::Memory,
        seed: 1,
    };
    let ops = vec![
        a(1),
        Op::Load {
            owner: 1,
            name: 1,
            width: 64,
            height: 32,
            format: SRC_INDEXED,
            flags: 0x1 | 0x200,
            source: Source::Lump { offset: 100 },
            seed: 1,
        },
        Op::Load {
            owner: 0,
            name: 2,
            width: 64,
            height: 64,
            format: SRC_RGBA,
            flags: 0x800 | 0x80,
            source: Source::Memory,
            seed: 2,
        },
        Op::Load {
            owner: 2,
            name: 3,
            width: 16,
            height: 16,
            format: SRC_LIGHTMAP,
            flags: 0x2 | 0x80,
            source: Source::Memory,
            seed: 3,
        },
        Op::Load {
            owner: 2,
            name: 5,
            width: 32,
            height: 32,
            format: SRC_RGBA_CUBEMAP,
            flags: 0x2,
            source: Source::Memory,
            seed: 4,
        },
        Op::Load {
            owner: 1,
            name: 0,
            width: 64,
            height: 32,
            format: SRC_INDEXED,
            flags: 0x1 | 0x8 | 0x100 | 0x40,
            source: Source::Memory,
            seed: 1,
        },
        Op::Load {
            owner: 1,
            name: 0,
            width: 64,
            height: 32,
            format: SRC_INDEXED,
            flags: 0x1 | 0x8 | 0x100 | 0x40,
            source: Source::Memory,
            seed: 99,
        },
        Op::Load {
            owner: 0,
            name: 4,
            width: 24,
            height: 16,
            format: SRC_INDEXED_PALETTE,
            flags: 0x1 | 0x100 | 0x1000,
            source: Source::Memory,
            seed: 5,
        },
        Op::Load {
            owner: 0,
            name: 3,
            width: 8,
            height: 8,
            format: SRC_RGBA,
            flags: 0x2,
            source: Source::Image { which: 0 },
            seed: 6,
        },
        Op::Load {
            owner: 0,
            name: 2,
            width: 8,
            height: 8,
            format: SRC_INDEXED,
            flags: 0x2,
            source: Source::Image { which: 2 },
            seed: 6,
        },
        Op::Reload {
            index: 0,
            shirt: 3,
            pants: 11,
        },
        Op::Reload {
            index: 1,
            shirt: 12,
            pants: 1,
        },
        Op::Reload {
            index: 2,
            shirt: -1,
            pants: -1,
        },
        Op::Find { owner: 1, name: 0 },
        Op::Find { owner: 2, name: 0 },
        Op::SetCvars(Cvars {
            gl_fullbrights: 0.0,
            vid_filter: 3.0,
            vid_anisotropic: 1.0,
            gl_max_size: 16.0,
            gl_picmip: 1.0,
        }),
        Op::ReloadNobright,
        Op::UpdateDescriptorSets,
        Op::InUpdateScreen(true),
        Op::Free { index: 0 },
        Op::Collect,
        Op::InUpdateScreen(false),
        Op::FreeOwner { owner: 2 },
        Op::Collect,
        Op::Collect,
        Op::DeleteObjects,
        Op::NewGame,
        Op::Collect,
        Op::Collect,
    ];
    with_sides(|s| {
        for op in &ops {
            run_op(s, op)?;
        }
        Ok(())
    })
    .unwrap();
}
