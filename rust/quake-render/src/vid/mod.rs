//! The Vulkan half of `gl_vidsdl.c` (Phase 8 M6): instance/device setup,
//! swap chain and presentation, render passes and frame buffers, the
//! per-frame command-buffer orchestration and the device-idle wait.
//!
//! The SDL half (window, video modes, `vid_*` cvars, video menu, cursors,
//! focus, screenshot file writing) stays C in `Quake/gl_vidsdl_glue.c`; the
//! engine reaches it through [`VidEngine`]. Everything here is generic over
//! that trait and owns its Vulkan state in [`VidState`]; `quake-capi`
//! provides the storage and the C ABI.
//!
//! ash 0.38 predates `VK_KHR_present_id2`/`VK_KHR_present_wait2`, so those
//! few structures and the one entry point are declared by hand below with
//! the raw enumerant values from `vulkan_core.h` (plan RA8).

use core::ffi::{c_int, c_void, CStr};
use std::ffi::CString;

use ash::vk;
use quake_types::render::{CbContext, VulkanGlobals, VulkanMemory, SCBX_NUM};

use crate::rmisc::VgPtr;

/// [`vg!`](crate::rmisc::vg) over a bare [`VgPtr`], for `init_instance` and
/// `init_device`, which run before a `Ctx` (an `ash::Device`) exists.
macro_rules! vgp {
    ($vg:expr, $($field:tt)+) => {
        // SAFETY: as for `rmisc::vg!`: the pointer is valid for its lifetime
        // and the place expression reads only this field.
        unsafe { (*$vg.as_ptr()).$($field)+ }
    };
}

/// [`vg_mut!`](crate::rmisc::vg_mut) over a bare [`VgPtr`].
macro_rules! vgp_mut {
    ($vg:expr, $($field:tt)+) => {
        // SAFETY: as for `rmisc::vg_mut!`: one field, one statement; the
        // init functions write on the main thread before any worker runs.
        unsafe { &mut (*$vg.as_ptr()).$($field)+ }
    };
}

use crate::rmisc::{dynbuf::DynBuffers, staging::Staging, Engine};

pub mod frame;
pub mod instance;
pub mod resources;
pub mod swapchain;

pub use frame::{
    acquire_next_swap_chain_image, begin_rendering_task, end_rendering_task, wait_for_device_idle,
    EndRenderingParms,
};
pub use instance::{init_command_buffers, init_device, init_instance};
pub use resources::{
    create_palette_octree_buffers, create_render_resources, destroy_render_resources,
    update_descriptor_sets,
};

// `glquake.h` secondary command-buffer indices.
pub const SCBX_WORLD: c_int = 0;
pub const SCBX_ENTITIES: c_int = 1;
pub const SCBX_SKY: c_int = 2;
pub const SCBX_VIEW_MODEL: c_int = 3;
pub const SCBX_FTE_PARTICLES_BLEND: c_int = 4;
pub const SCBX_ALPHA_ENTITIES_ACROSS_WATER: c_int = 5;
pub const SCBX_WATER: c_int = 6;
pub const SCBX_ALPHA_ENTITIES: c_int = 7;
pub const SCBX_PARTICLES: c_int = 8;
pub const SCBX_MBOIT_COMPOSITE_ALPHA_ENTITIES_ACROSS_WATER: c_int = 9;
pub const SCBX_MBOIT_COMPOSITE_WATER: c_int = 10;
pub const SCBX_MBOIT_COMPOSITE_ALPHA_ENTITIES: c_int = 11;
pub const SCBX_MBOIT_COMPOSITE_PARTICLES: c_int = 12;
pub const SCBX_OIT_RESOLVE: c_int = 13;
pub const SCBX_GUI: c_int = 14;
pub const SCBX_POST_PROCESS: c_int = 15;
pub const SCBX_MAIN_OPAQUE_PASS_LAST: c_int = SCBX_FTE_PARTICLES_BLEND;
pub const SCBX_MAIN_PASS_LAST: c_int = SCBX_PARTICLES;
pub const SCBX_MBOIT_COMPOSITE_PASS_FIRST: c_int = SCBX_MBOIT_COMPOSITE_ALPHA_ENTITIES_ACROSS_WATER;
pub const SCBX_MBOIT_COMPOSITE_PASS_LAST: c_int = SCBX_MBOIT_COMPOSITE_PARTICLES;

// `render_pass_index_t`.
pub const RENDER_PASS_INDEX_MAIN: c_int = 0;
pub const RENDER_PASS_INDEX_UI: c_int = 1;
pub const RENDER_PASS_INDEX_MAIN_OIT: c_int = 2;
pub const RENDER_PASS_INDEX_MAIN_MBOIT: c_int = 3;
pub const RENDER_PASS_INDEX_WBOIT: c_int = 4;
pub const RENDER_PASS_INDEX_MBOIT_MOMENTS: c_int = 5;
pub const RENDER_PASS_INDEX_MBOIT_COMPOSITE: c_int = 6;

// `main_render_pass_variant_t` / `main_render_pass_stencil_t`.
pub const MAIN_RENDER_PASS_STANDARD: usize = 0;
pub const MAIN_RENDER_PASS_OIT: usize = 1;
pub const MAIN_RENDER_PASS_MBOIT: usize = 2;
pub const MAIN_RENDER_PASS_STENCIL_CLEAR: usize = 0;
pub const MAIN_RENDER_PASS_NO_STENCIL: usize = 1;

/// `SECONDARY_CB_MULTIPLICITY` (`NUM_WORLD_CBX`/`NUM_ENTITIES_CBX` = 6).
pub const SECONDARY_CB_MULTIPLICITY: [usize; SCBX_NUM] =
    [6, 6, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1];
pub const MAX_SECONDARY_CB_MULTIPLICITY: usize = 6;
pub const DOUBLE_BUFFERED: usize = 2;
pub const MAX_SWAP_CHAIN_IMAGES: usize = 8;
pub const CANVAS_NONE: c_int = 0;
pub const CANVAS_INVALID: c_int = -1;
pub const OIT_MODE_NONE: c_int = 0;
pub const OIT_MODE_WBOIT: c_int = 1;
pub const OIT_MODE_MBOIT: c_int = 2;

/// `REQUIRED_COLOR_BUFFER_FEATURES`.
pub const REQUIRED_COLOR_BUFFER_FEATURES: vk::FormatFeatureFlags = vk::FormatFeatureFlags::from_raw(
    vk::FormatFeatureFlags::COLOR_ATTACHMENT.as_raw()
        | vk::FormatFeatureFlags::COLOR_ATTACHMENT_BLEND.as_raw()
        | vk::FormatFeatureFlags::SAMPLED_IMAGE.as_raw()
        | vk::FormatFeatureFlags::STORAGE_IMAGE.as_raw()
        | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR.as_raw(),
);

// ---------------------------------------------------------------------------
// VK_KHR_present_id2 / VK_KHR_present_wait2 (absent from ash 0.38).

pub const STRUCTURE_TYPE_SURFACE_CAPABILITIES_PRESENT_ID_2_KHR: vk::StructureType =
    vk::StructureType::from_raw(1000479000);
pub const STRUCTURE_TYPE_PRESENT_ID_2_KHR: vk::StructureType =
    vk::StructureType::from_raw(1000479001);
pub const STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_ID_2_FEATURES_KHR: vk::StructureType =
    vk::StructureType::from_raw(1000479002);
pub const STRUCTURE_TYPE_SURFACE_CAPABILITIES_PRESENT_WAIT_2_KHR: vk::StructureType =
    vk::StructureType::from_raw(1000480000);
pub const STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_WAIT_2_FEATURES_KHR: vk::StructureType =
    vk::StructureType::from_raw(1000480001);
pub const STRUCTURE_TYPE_PRESENT_WAIT_2_INFO_KHR: vk::StructureType =
    vk::StructureType::from_raw(1000480002);
pub const SWAPCHAIN_CREATE_PRESENT_ID_2_BIT_KHR: vk::SwapchainCreateFlagsKHR =
    vk::SwapchainCreateFlagsKHR::from_raw(0x40);
pub const SWAPCHAIN_CREATE_PRESENT_WAIT_2_BIT_KHR: vk::SwapchainCreateFlagsKHR =
    vk::SwapchainCreateFlagsKHR::from_raw(0x80);
pub const KHR_PRESENT_ID_2_EXTENSION_NAME: &CStr = c"VK_KHR_present_id2";
pub const KHR_PRESENT_WAIT_2_EXTENSION_NAME: &CStr = c"VK_KHR_present_wait2";

/// `VkSurfaceCapabilitiesPresentId2KHR`.
#[repr(C)]
pub struct SurfaceCapabilitiesPresentId2KHR {
    pub s_type: vk::StructureType,
    pub p_next: *mut c_void,
    pub present_id2_supported: vk::Bool32,
}

/// `VkSurfaceCapabilitiesPresentWait2KHR`.
#[repr(C)]
pub struct SurfaceCapabilitiesPresentWait2KHR {
    pub s_type: vk::StructureType,
    pub p_next: *mut c_void,
    pub present_wait2_supported: vk::Bool32,
}

/// `VkPhysicalDevicePresentId2FeaturesKHR`.
#[repr(C)]
pub struct PhysicalDevicePresentId2FeaturesKHR {
    pub s_type: vk::StructureType,
    pub p_next: *mut c_void,
    pub present_id2: vk::Bool32,
}

/// `VkPhysicalDevicePresentWait2FeaturesKHR`.
#[repr(C)]
pub struct PhysicalDevicePresentWait2FeaturesKHR {
    pub s_type: vk::StructureType,
    pub p_next: *mut c_void,
    pub present_wait2: vk::Bool32,
}

/// `VkPresentWait2InfoKHR`.
#[repr(C)]
pub struct PresentWait2InfoKHR {
    pub s_type: vk::StructureType,
    pub p_next: *const c_void,
    pub present_id: u64,
    pub timeout: u64,
}

/// `VkPresentId2KHR`.
#[repr(C)]
pub struct PresentId2KHR {
    pub s_type: vk::StructureType,
    pub p_next: *const c_void,
    pub swapchain_count: u32,
    pub p_present_ids: *const u64,
}

// SAFETY: each struct is the `repr(C)` mirror of the Vulkan struct named in
// its doc comment, with `s_type`/`p_next` first, and may legally extend the
// chains it is marked for (VK_KHR_present_id2 / VK_KHR_present_wait2).
unsafe impl vk::ExtendsSurfaceCapabilities2KHR for SurfaceCapabilitiesPresentId2KHR {}
// SAFETY: as above.
unsafe impl vk::ExtendsSurfaceCapabilities2KHR for SurfaceCapabilitiesPresentWait2KHR {}
// SAFETY: as above.
unsafe impl vk::ExtendsPhysicalDeviceFeatures2 for PhysicalDevicePresentId2FeaturesKHR {}
// SAFETY: as above.
unsafe impl vk::ExtendsDeviceCreateInfo for PhysicalDevicePresentId2FeaturesKHR {}
// SAFETY: as above.
unsafe impl vk::ExtendsPhysicalDeviceFeatures2 for PhysicalDevicePresentWait2FeaturesKHR {}
// SAFETY: as above.
unsafe impl vk::ExtendsDeviceCreateInfo for PhysicalDevicePresentWait2FeaturesKHR {}
// SAFETY: as above.
unsafe impl vk::ExtendsPresentInfoKHR for PresentId2KHR {}

/// `PFN_vkWaitForPresent2KHR`.
#[allow(non_camel_case_types)]
pub type PFN_vkWaitForPresent2KHR = unsafe extern "system" fn(
    vk::Device,
    vk::SwapchainKHR,
    *const PresentWait2InfoKHR,
) -> vk::Result;

// ---------------------------------------------------------------------------

/// What the Vulkan side needs from the SDL/engine half of `gl_vidsdl.c` and
/// the rest of the engine, beyond [`Engine`].
pub trait VidEngine: Engine {
    /// `SDL_Vulkan_GetVkGetInstanceProcAddr`.
    fn get_instance_proc_addr(&self) -> vk::PFN_vkGetInstanceProcAddr;
    /// `SDL_Vulkan_GetInstanceExtensions` (fatal on failure, like C).
    fn instance_extensions(&self) -> Vec<CString>;
    /// `SDL_Vulkan_CreateSurface` (fatal on failure, like C).
    fn create_surface(&self, instance: vk::Instance) -> vk::SurfaceKHR;
    /// `MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY)` for the SDL window.
    #[cfg(windows)]
    fn window_monitor(&self) -> *mut c_void;
    /// The `has_focus` static of `gl_vidsdl.c`.
    fn has_focus(&self) -> bool;
    /// `VID_GetFullscreen`.
    fn fullscreen(&self) -> bool;
    /// `COM_CheckParm ("-device")`: `None` when absent, `Some(None)` when it
    /// is the last argument (no value), else `Some(Some(atoi(next)))`.
    fn device_parm(&self) -> Option<Option<i32>>;
    /// `vid.width`, `vid.height`.
    fn vid_size(&self) -> (u32, u32);
    /// `vid.restart_next_frame = true`.
    fn set_restart_next_frame(&self);
    /// `DebugMessageCallback`.
    #[cfg(feature = "engine-debug")]
    fn debug_message_callback(&self) -> vk::PFN_vkDebugUtilsMessengerCallbackEXT;
    /// `Harness_RenderInstallHooks`.
    fn install_harness_hooks(&self);
    /// `Sys_DoubleTime`.
    fn double_time(&self) -> f64;
    /// `Sky_NeedStencil`.
    fn sky_need_stencil(&self) -> bool;
    /// `GL_SetCanvas`.
    fn set_canvas(&self, cbx: &mut CbContext, canvas: c_int);
    /// `GL_Viewport`.
    #[allow(clippy::too_many_arguments)]
    fn viewport(
        &self,
        cbx: &mut CbContext,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        min_depth: f32,
        max_depth: f32,
    );
    /// `R_CollectMeshBufferGarbage`.
    fn collect_mesh_buffer_garbage(&self);
    /// `R_CollectTLASGarbage`.
    fn collect_tlas_garbage(&self);
    /// `TexMgr_CollectGarbage`.
    fn texmgr_collect_garbage(&self);
    /// `frame_oit_mode`.
    fn frame_oit_mode(&self) -> c_int;
    /// `sv.active && cls.signon < 1` (`GL_CreateRenderResources` early out).
    fn skip_render_resources(&self) -> bool;
    fn vid_vsync(&self) -> f32;
    fn vid_maxframelatency(&self) -> f32;
    fn vid_fsaa(&self) -> f32;
    fn vid_fsaamode(&self) -> f32;
    fn vid_gamma(&self) -> f32;
    fn vid_contrast(&self) -> f32;
    fn r_usesops(&self) -> f32;
    /// `rs_gpuwaitaccum_us += us`.
    fn add_gpu_wait_us(&self, us: u32);
    /// `rs_gputime_us = us`.
    fn set_gpu_time_us(&self, us: u32);
    /// The file/Steam/console half of `WriteScreenshot` over RGBA pixels.
    fn write_screenshot(&self, pixels: &[u8], width: u32, height: u32);
    /// `bluenoisetexture->image_view`.
    fn bluenoise_image_view(&self) -> vk::ImageView;
    /// `bmodel_tlas`.
    fn bmodel_tlas(&self) -> vk::AccelerationStructureKHR;
    /// The `gl_rmisc.c` staging buffers (`R_SubmitStagingBuffers`).
    fn staging(&self) -> &Staging;
    /// The `gl_rmisc.c` dynamic buffers (`R_SwapDynamicBuffers` and friends).
    fn dyn_buffers(&self) -> &DynBuffers;
    /// `frame_upload_buffers_memory[0].handle` (`r_brush.c`).
    fn frame_upload_buffers_memory(&self) -> vk::DeviceMemory;
    /// `GL_SynchronizeEndRenderingTask`: joins the previous frame's
    /// `GL_EndRenderingTask`, if any.
    fn synchronize_end_rendering_task(&self);
}

/// `R_UseWBOIT` for the frame's OIT mode.
pub fn use_wboit<E: VidEngine>(engine: &E) -> bool {
    engine.frame_oit_mode() == OIT_MODE_WBOIT
}

/// `R_UseMBOIT`.
pub fn use_mboit<E: VidEngine>(engine: &E) -> bool {
    engine.frame_oit_mode() == OIT_MODE_MBOIT
}

/// `R_UseOIT`.
pub fn use_oit<E: VidEngine>(engine: &E) -> bool {
    engine.frame_oit_mode() != OIT_MODE_NONE
}

/// The `fp*` function pointers `gl_vidsdl.c` loads by hand.
#[derive(Default)]
pub struct Procs {
    pub get_instance_proc_addr: Option<vk::PFN_vkGetInstanceProcAddr>,
    pub get_device_proc_addr: Option<vk::PFN_vkGetDeviceProcAddr>,
    pub get_physical_device_surface_support: Option<vk::PFN_vkGetPhysicalDeviceSurfaceSupportKHR>,
    pub get_physical_device_surface_capabilities:
        Option<vk::PFN_vkGetPhysicalDeviceSurfaceCapabilitiesKHR>,
    pub get_physical_device_surface_capabilities2:
        Option<vk::PFN_vkGetPhysicalDeviceSurfaceCapabilities2KHR>,
    pub get_physical_device_surface_formats: Option<vk::PFN_vkGetPhysicalDeviceSurfaceFormatsKHR>,
    pub get_physical_device_surface_present_modes:
        Option<vk::PFN_vkGetPhysicalDeviceSurfacePresentModesKHR>,
    pub get_physical_device_properties2: Option<vk::PFN_vkGetPhysicalDeviceProperties2>,
    pub get_physical_device_features2: Option<vk::PFN_vkGetPhysicalDeviceFeatures2>,
    pub create_swapchain: Option<vk::PFN_vkCreateSwapchainKHR>,
    pub destroy_swapchain: Option<vk::PFN_vkDestroySwapchainKHR>,
    pub get_swapchain_images: Option<vk::PFN_vkGetSwapchainImagesKHR>,
    pub acquire_next_image: Option<vk::PFN_vkAcquireNextImageKHR>,
    pub queue_present: Option<vk::PFN_vkQueuePresentKHR>,
    pub wait_for_present2: Option<PFN_vkWaitForPresent2KHR>,
    #[cfg(windows)]
    pub acquire_full_screen_exclusive_mode: Option<vk::PFN_vkAcquireFullScreenExclusiveModeEXT>,
    #[cfg(windows)]
    pub release_full_screen_exclusive_mode: Option<vk::PFN_vkReleaseFullScreenExclusiveModeEXT>,
    #[cfg(feature = "engine-debug")]
    pub set_debug_utils_object_name: Option<vk::PFN_vkSetDebugUtilsObjectNameEXT>,
}

/// A zeroed `vulkan_memory_t` (const-constructible).
const NULL_MEMORY: VulkanMemory = VulkanMemory {
    handle: vk::DeviceMemory::null(),
    size: 0,
    type_: quake_types::render::VulkanMemoryType::None,
};

/// An image with its dedicated memory and view (`depth_buffer`, the OIT
/// buffers, `msaa_color_buffer`).
#[derive(Default)]
pub struct ImageSet {
    pub image: vk::Image,
    pub memory: VulkanMemory,
    pub view: vk::ImageView,
}

/// The file-scope statics of `gl_vidsdl.c` that back the Vulkan half.
#[derive(Default)]
pub struct VidState {
    pub entry: Option<ash::Entry>,
    pub instance: Option<ash::Instance>,
    pub physical_device: vk::PhysicalDevice,
    pub surface: vk::SurfaceKHR,
    pub swapchain: vk::SwapchainKHR,
    pub procs: Procs,
    #[cfg(feature = "engine-debug")]
    pub debug_utils_messenger: vk::DebugUtilsMessengerEXT,

    pub num_swap_chain_images: usize,
    pub render_resources_created: bool,
    pub current_cb_index: usize,
    pub primary_command_pools: [vk::CommandPool; quake_types::render::PCBX_NUM],
    pub secondary_command_pools: [[vk::CommandPool; MAX_SECONDARY_CB_MULTIPLICITY]; SCBX_NUM],
    pub transient_command_pool: vk::CommandPool,
    pub primary_command_buffers:
        [[vk::CommandBuffer; DOUBLE_BUFFERED]; quake_types::render::PCBX_NUM],
    pub secondary_command_buffers:
        [[[vk::CommandBuffer; MAX_SECONDARY_CB_MULTIPLICITY]; DOUBLE_BUFFERED]; SCBX_NUM],
    pub command_buffer_fences: [vk::Fence; DOUBLE_BUFFERED],
    pub frame_submitted: [bool; DOUBLE_BUFFERED],
    pub timestamp_query_pool: vk::QueryPool,
    pub timestamps_written: [bool; DOUBLE_BUFFERED],
    pub main_framebuffers: [vk::Framebuffer; quake_types::render::NUM_COLOR_BUFFERS],
    pub image_aquired_semaphores: [vk::Semaphore; DOUBLE_BUFFERED],
    pub draw_complete_semaphores: [vk::Semaphore; MAX_SWAP_CHAIN_IMAGES],
    pub ui_framebuffers: [vk::Framebuffer; MAX_SWAP_CHAIN_IMAGES],
    pub swapchain_images: [vk::Image; MAX_SWAP_CHAIN_IMAGES],
    pub swapchain_images_views: [vk::ImageView; MAX_SWAP_CHAIN_IMAGES],
    pub depth_buffer: ImageSet,
    pub color_buffers_memory: [VulkanMemory; quake_types::render::NUM_COLOR_BUFFERS],
    pub color_buffers_view: [vk::ImageView; quake_types::render::NUM_COLOR_BUFFERS],
    pub oit_accum_buffer_memory: VulkanMemory,
    pub oit_accum_buffer_view: vk::ImageView,
    pub oit_reveal_buffer_memory: VulkanMemory,
    pub oit_reveal_buffer_view: vk::ImageView,
    pub mboit_b0_buffer_memory: VulkanMemory,
    pub mboit_b0_buffer_view: vk::ImageView,
    pub mboit_moments0_buffer_memory: VulkanMemory,
    pub mboit_moments0_buffer_view: vk::ImageView,
    pub mboit_color_buffer_memory: VulkanMemory,
    pub mboit_color_buffer_view: vk::ImageView,
    pub msaa_color_buffer: ImageSet,
    pub postprocess_descriptor_set: vk::DescriptorSet,
    pub wboit_resolve_descriptor_set: vk::DescriptorSet,
    pub palette_colors_buffer: vk::Buffer,
    pub palette_buffer_view: vk::BufferView,
    pub palette_octree_buffer: vk::Buffer,
    pub current_swapchain_buffer: u32,
    pub num_images_acquired: usize,
    pub current_present_id: u64,
    pub swapchain_present_wait: bool,
}

impl VidState {
    pub const fn new() -> Self {
        Self {
            entry: None,
            instance: None,
            physical_device: vk::PhysicalDevice::null(),
            surface: vk::SurfaceKHR::null(),
            swapchain: vk::SwapchainKHR::null(),
            procs: Procs {
                get_instance_proc_addr: None,
                get_device_proc_addr: None,
                get_physical_device_surface_support: None,
                get_physical_device_surface_capabilities: None,
                get_physical_device_surface_capabilities2: None,
                get_physical_device_surface_formats: None,
                get_physical_device_surface_present_modes: None,
                get_physical_device_properties2: None,
                get_physical_device_features2: None,
                create_swapchain: None,
                destroy_swapchain: None,
                get_swapchain_images: None,
                acquire_next_image: None,
                queue_present: None,
                wait_for_present2: None,
                #[cfg(windows)]
                acquire_full_screen_exclusive_mode: None,
                #[cfg(windows)]
                release_full_screen_exclusive_mode: None,
                #[cfg(feature = "engine-debug")]
                set_debug_utils_object_name: None,
            },
            #[cfg(feature = "engine-debug")]
            debug_utils_messenger: vk::DebugUtilsMessengerEXT::null(),
            num_swap_chain_images: 0,
            render_resources_created: false,
            current_cb_index: 0,
            primary_command_pools: [vk::CommandPool::null(); quake_types::render::PCBX_NUM],
            secondary_command_pools: [[vk::CommandPool::null(); MAX_SECONDARY_CB_MULTIPLICITY];
                SCBX_NUM],
            transient_command_pool: vk::CommandPool::null(),
            primary_command_buffers: [[vk::CommandBuffer::null(); DOUBLE_BUFFERED];
                quake_types::render::PCBX_NUM],
            secondary_command_buffers: [[[vk::CommandBuffer::null(); MAX_SECONDARY_CB_MULTIPLICITY];
                DOUBLE_BUFFERED]; SCBX_NUM],
            command_buffer_fences: [vk::Fence::null(); DOUBLE_BUFFERED],
            frame_submitted: [false; DOUBLE_BUFFERED],
            timestamp_query_pool: vk::QueryPool::null(),
            timestamps_written: [false; DOUBLE_BUFFERED],
            main_framebuffers: [vk::Framebuffer::null(); quake_types::render::NUM_COLOR_BUFFERS],
            image_aquired_semaphores: [vk::Semaphore::null(); DOUBLE_BUFFERED],
            draw_complete_semaphores: [vk::Semaphore::null(); MAX_SWAP_CHAIN_IMAGES],
            ui_framebuffers: [vk::Framebuffer::null(); MAX_SWAP_CHAIN_IMAGES],
            swapchain_images: [vk::Image::null(); MAX_SWAP_CHAIN_IMAGES],
            swapchain_images_views: [vk::ImageView::null(); MAX_SWAP_CHAIN_IMAGES],
            depth_buffer: ImageSet::NULL,
            color_buffers_memory: [NULL_MEMORY; quake_types::render::NUM_COLOR_BUFFERS],
            color_buffers_view: [vk::ImageView::null(); quake_types::render::NUM_COLOR_BUFFERS],
            oit_accum_buffer_memory: NULL_MEMORY,
            oit_accum_buffer_view: vk::ImageView::null(),
            oit_reveal_buffer_memory: NULL_MEMORY,
            oit_reveal_buffer_view: vk::ImageView::null(),
            mboit_b0_buffer_memory: NULL_MEMORY,
            mboit_b0_buffer_view: vk::ImageView::null(),
            mboit_moments0_buffer_memory: NULL_MEMORY,
            mboit_moments0_buffer_view: vk::ImageView::null(),
            mboit_color_buffer_memory: NULL_MEMORY,
            mboit_color_buffer_view: vk::ImageView::null(),
            msaa_color_buffer: ImageSet::NULL,
            postprocess_descriptor_set: vk::DescriptorSet::null(),
            wboit_resolve_descriptor_set: vk::DescriptorSet::null(),
            palette_colors_buffer: vk::Buffer::null(),
            palette_buffer_view: vk::BufferView::null(),
            palette_octree_buffer: vk::Buffer::null(),
            current_swapchain_buffer: 0,
            num_images_acquired: 0,
            current_present_id: 0,
            swapchain_present_wait: false,
        }
    }

    /// The `ash::Instance` created by [`init_instance`].
    pub fn instance(&self) -> &ash::Instance {
        self.instance.as_ref().expect("GL_InitInstance has run")
    }
}

impl ImageSet {
    pub const NULL: Self = Self {
        image: vk::Image::null(),
        memory: NULL_MEMORY,
        view: vk::ImageView::null(),
    };
}

/// Loads `name` through `vkGetInstanceProcAddr`, failing like the
/// `GET_INSTANCE_PROC_ADDR`/`GET_GLOBAL_INSTANCE_PROC_ADDR` macros.
pub(crate) fn instance_proc<E: Engine, T: Copy>(
    engine: &E,
    get_instance_proc_addr: vk::PFN_vkGetInstanceProcAddr,
    instance: vk::Instance,
    name: &CStr,
) -> T {
    const {
        assert!(core::mem::size_of::<T>() == core::mem::size_of::<vk::PFN_vkVoidFunction>());
    }
    // SAFETY: `name` is NUL-terminated and the loader entry point accepts a
    // null instance for the global commands.
    let f = unsafe { get_instance_proc_addr(instance, name.as_ptr()) };
    match f {
        // SAFETY: `T` is a function-pointer type of the same size as
        // `PFN_vkVoidFunction`; the loader returned the entry point for
        // `name`, whose prototype the caller picked `T` to match.
        Some(f) => unsafe { core::mem::transmute_copy::<unsafe extern "system" fn(), T>(&f) },
        None => engine.sys_error(&format!(
            "vkGetInstanceProcAddr failed to find {}",
            name.to_str().unwrap_or("?")
        )),
    }
}

/// Loads `name` through `vkGetDeviceProcAddr`, failing like the
/// `GET_DEVICE_PROC_ADDR`/`GET_GLOBAL_DEVICE_PROC_ADDR` macros.
pub(crate) fn device_proc<E: Engine, T: Copy>(
    engine: &E,
    get_device_proc_addr: vk::PFN_vkGetDeviceProcAddr,
    device: vk::Device,
    name: &CStr,
) -> T {
    const {
        assert!(core::mem::size_of::<T>() == core::mem::size_of::<vk::PFN_vkVoidFunction>());
    }
    // SAFETY: `name` is NUL-terminated; `device` is a live device.
    let f = unsafe { get_device_proc_addr(device, name.as_ptr()) };
    match f {
        // SAFETY: as in `instance_proc`.
        Some(f) => unsafe { core::mem::transmute_copy::<unsafe extern "system" fn(), T>(&f) },
        None => engine.sys_error(&format!(
            "vkGetDeviceProcAddr failed to find {}",
            name.to_str().unwrap_or("?")
        )),
    }
}

/// `(scbx, multiplicity)` for the secondary command-buffer slots `first..=last`.
pub(crate) fn scbx_slots(first: c_int, last: c_int) -> impl Iterator<Item = (usize, usize)> {
    SECONDARY_CB_MULTIPLICITY
        .iter()
        .copied()
        .enumerate()
        .take(last as usize + 1)
        .skip(first as usize)
}

/// The `i`-th `cb_context_t` of secondary command-buffer slot `scbx`, for
/// the init/teardown paths (`create_render_resources`,
/// `destroy_render_resources`): main thread, no render task in flight, so
/// nothing else touches the context for the returned lifetime.
pub(crate) fn scbx_mut<'a>(vg: VgPtr<'a>, scbx: usize, i: usize) -> &'a mut CbContext {
    // SAFETY: `init_command_buffers` allocates `secondary_cb_contexts[scbx]`
    // with `SECONDARY_CB_MULTIPLICITY[scbx]` entries that are never freed
    // (see `scbx_ptr`); the callers' phase guarantees exclusivity.
    unsafe { &mut *scbx_ptr(vg.as_ptr(), scbx, i) }
}

/// A raw pointer to the `i`-th `cb_context_t` of secondary slot `scbx`, for
/// the per-frame path, which has to read `vulkan_globals` while a secondary
/// context is live (the callers document why no other reference exists).
pub(crate) fn scbx_ptr(vg: *const VulkanGlobals, scbx: usize, i: usize) -> *mut CbContext {
    debug_assert!(i < SECONDARY_CB_MULTIPLICITY[scbx]);
    // SAFETY: `vg` is a `VgPtr` (valid, aligned); the field read forms no
    // reference to the struct, and the offset is in bounds of the
    // allocation `init_command_buffers` made for `secondary_cb_contexts[scbx]`.
    unsafe { (*vg).secondary_cb_contexts[scbx].add(i) }
}

/// `Sys_Error ("<what> with code %i", err)`.
pub(crate) fn fail<E: Engine>(engine: &E, what: &str, err: vk::Result) -> ! {
    engine.sys_error(&format!("{what} with code {}", err.as_raw()))
}
