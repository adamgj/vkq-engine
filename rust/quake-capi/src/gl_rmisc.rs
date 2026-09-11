//! C ABI for `Quake/gl_rmisc.c` (Rust migration Phase 8 M5, ADR-007,
//! ADR-011, ADR-015). The port itself lives in `quake_render::rmisc`; this
//! module owns the C-visible data (`vulkan_globals` and the allocation
//! counters, ADR-007 dual view), exports the `glquake.h` prototypes with
//! byte-identical signatures, and implements the [`Engine`] seams over the C
//! that remains (`Sys_Error`, `GL_SetObjectName`, the `bintoc` SPIR-V arrays,
//! the harness hooks, ...).
//!
//! What stays C is in `Quake/gl_rmisc_glue.c`: the cvar registrations and
//! callbacks, `R_Init`, `R_NewMap`/`R_NewGame`, the player-skin translation,
//! `R_TimeRefresh_f` and the `vkmemstats` printer -- everything that can
//! `Host_Error` (ADR-009) or only orchestrates other still-C files.

#![allow(non_upper_case_globals)]

use core::ffi::{c_char, c_int, c_void, CStr};
use core::mem::size_of;
use core::ptr;
use core::slice;
use core::sync::atomic::{AtomicU32, AtomicU64};
use std::ffi::CString;
use std::sync::OnceLock;

use ash::vk::{self, Handle};

use quake_c_sys as c;
use quake_c_sys::render as g;
use quake_render::rmisc::shaders::ShaderModules;
use quake_render::rmisc::{
    self, BufferRequest, Counters, Ctx, DynBuffers, Engine, Shader, Staging, VgPtr,
};
use quake_types::render::{
    BufferCreateInfo, DynBuffer, VulkanDescSetLayout, VulkanGlobals, VulkanMemory, VulkanMemoryType,
};

/* ---- C-visible data (ADR-007 dual view) ---- */

/// `vulkanglobals_t vulkan_globals;` (`gl_rmisc.c:44`). Rust-owned from M5
/// on; the still-C renderer files read and write it through this symbol
/// exactly as before, and the Rust side goes through [`with_ctx`].
// SAFETY: the all-zero bit pattern is a valid `VulkanGlobals` (null handles,
// `false` flags, `None` function pointers, zero integers), which is also what
// the C definition's BSS placement produced.
#[no_mangle]
pub static mut vulkan_globals: VulkanGlobals = unsafe { core::mem::zeroed() };

/// The `atomic_uint32_t`/`atomic_uint64_t` allocation counters
/// (`gl_rmisc.c:46-62`, `quakedef.h:509-523`). `AtomicU32`/`AtomicU64` have
/// the same size and alignment as the C11 `_Atomic uint32_t`/`uint64_t`
/// (and the MSVC `volatile` arm of `atomics.h`).
#[no_mangle]
pub static num_vulkan_tex_allocations: AtomicU32 = AtomicU32::new(0);
#[no_mangle]
pub static num_vulkan_bmodel_allocations: AtomicU32 = AtomicU32::new(0);
#[no_mangle]
pub static num_vulkan_mesh_allocations: AtomicU32 = AtomicU32::new(0);
#[no_mangle]
pub static num_vulkan_misc_allocations: AtomicU32 = AtomicU32::new(0);
#[no_mangle]
pub static num_vulkan_dynbuf_allocations: AtomicU32 = AtomicU32::new(0);
#[no_mangle]
pub static num_vulkan_combined_image_samplers: AtomicU32 = AtomicU32::new(0);
#[no_mangle]
pub static num_vulkan_ubos_dynamic: AtomicU32 = AtomicU32::new(0);
#[no_mangle]
pub static num_vulkan_ubos: AtomicU32 = AtomicU32::new(0);
#[no_mangle]
pub static num_vulkan_storage_buffers: AtomicU32 = AtomicU32::new(0);
#[no_mangle]
pub static num_vulkan_input_attachments: AtomicU32 = AtomicU32::new(0);
#[no_mangle]
pub static num_vulkan_storage_images: AtomicU32 = AtomicU32::new(0);
#[no_mangle]
pub static num_vulkan_sampled_images: AtomicU32 = AtomicU32::new(0);
#[no_mangle]
pub static num_acceleration_structures: AtomicU32 = AtomicU32::new(0);
#[no_mangle]
pub static total_device_vulkan_allocation_size: AtomicU64 = AtomicU64::new(0);
#[no_mangle]
pub static total_host_vulkan_allocation_size: AtomicU64 = AtomicU64::new(0);

/// The staging ring (`gl_rmisc.c:506-836`) and the dynamic vertex/index/
/// uniform/storage rings (`gl_rmisc.c:837-1292`), formerly file statics.
static STAGING: Staging = Staging::new();
static DYN: DynBuffers = DynBuffers::new();

/// The `ash::Device` dispatch table, loaded through `vkGetDeviceProcAddr`
/// from the `VkDevice` `gl_vidsdl.c` stores in `vulkan_globals.device`
/// (created once, never destroyed -- there is no `vkDestroyDevice` path).
static DEVICE: OnceLock<ash::Device> = OnceLock::new();

fn device() -> &'static ash::Device {
    // SAFETY: `vulkan_globals.device` is a plain handle read; C only writes it
    // during `GL_CreateDevice`, before any of these entry points run.
    let handle = unsafe { (*ptr::addr_of!(vulkan_globals)).device };
    let device = DEVICE.get_or_init(|| {
        if handle == vk::Device::null() {
            CEngine.sys_error("gl_rmisc: vulkan_globals.device is not created yet");
        }
        // SAFETY: `handle` is the live `VkDevice`; `vkGetDeviceProcAddr` is the
        // loader's exported entry point and the names ash asks for are NUL
        // terminated `&CStr`s.
        unsafe {
            ash::Device::load_with(
                |name| {
                    g::vkGetDeviceProcAddr(handle.as_raw() as usize as *mut c_void, name.as_ptr())
                },
                handle,
            )
        }
    });
    debug_assert_eq!(
        device.handle(),
        handle,
        "vulkan_globals.device changed after the table was loaded"
    );
    device
}

/* ---- Engine seams ---- */

struct CEngine;

fn cstring(s: &str) -> CString {
    CString::new(s.replace('\0', "?")).expect("no interior NUL")
}

impl Engine for CEngine {
    fn sys_error(&self, msg: &str) -> ! {
        let msg = cstring(msg);
        // SAFETY: `Sys_Error` is a diverging variadic C function; `"%s"` and
        // `msg` are NUL-terminated C strings (ADR-009: terminates, no longjmp).
        unsafe { c::Sys_Error(c"%s".as_ptr(), msg.as_ptr()) }
    }

    fn sys_printf(&self, msg: &str) {
        let msg = cstring(msg);
        // SAFETY: `"%s"` and `msg` are NUL-terminated C strings.
        unsafe { c::Sys_Printf(c"%s".as_ptr(), msg.as_ptr()) }
    }

    fn con_printf(&self, msg: &str) {
        let msg = cstring(msg);
        // SAFETY: `"%s"` and `msg` are NUL-terminated C strings.
        unsafe { c::Con_Printf(c"%s".as_ptr(), msg.as_ptr()) }
    }

    fn set_object_name(&self, handle: u64, object_type: vk::ObjectType, name: &CStr) {
        // SAFETY: `GL_SetObjectName` (`gl_vidsdl.c`) reads `vulkan_globals.
        // debug_utils`/`device` and the NUL-terminated `name`.
        unsafe { g::GL_SetObjectName(handle, object_type.as_raw(), name.as_ptr()) }
    }

    fn pipeline_created(&self, handle: u64, name: &CStr) {
        // SAFETY: `Harness_RenderPipelineCreated` takes a handle and a
        // NUL-terminated name; only called when `harness_renderhash` is set.
        unsafe { g::Harness_RenderPipelineCreated(handle, name.as_ptr()) }
    }

    fn pipelines_destroyed(&self) {
        // SAFETY: `Harness_RenderPipelinesDestroyed` has no preconditions.
        unsafe { g::Harness_RenderPipelinesDestroyed() }
    }

    fn wait_for_device_idle(&self) {
        // SAFETY: `GL_WaitForDeviceIdle` (`gl_vidsdl.c`) has no preconditions.
        unsafe { g::GL_WaitForDeviceIdle() }
    }

    fn update_texture_descriptor_sets(&self) {
        crate::gl_texmgr::TexMgr_UpdateTextureDescriptorSets();
    }

    fn r_lodbias(&self) -> f32 {
        // SAFETY: `r_lodbias` is a `cvar_t` defined in `gl_rmisc_glue.c`; only
        // `.value` is read, which the main thread alone writes.
        unsafe { (*ptr::addr_of!(g::r_lodbias)).value }
    }

    fn gl_lodbias(&self) -> f32 {
        // SAFETY: as `r_lodbias`.
        unsafe { (*ptr::addr_of!(g::gl_lodbias)).value }
    }

    fn r_scale(&self) -> f32 {
        // SAFETY: `r_scale` is a `cvar_t` defined in `gl_screen.c`; as above.
        unsafe { (*ptr::addr_of!(c::menu::r_scale)).value }
    }

    fn shader_spv(&self, shader: Shader) -> &[u8] {
        // SAFETY: the `bintoc` symbols are `const` data with static storage
        // duration; reading `<name>_spv_size` and taking `<name>_spv`'s
        // address never races with anything.
        let (data, len): (*const u8, c_int) = unsafe {
            match shader {
                Shader::basic_vert => (
                    ptr::addr_of!(g::basic_vert_spv).cast::<u8>(),
                    g::basic_vert_spv_size,
                ),
                Shader::basic_frag => (
                    ptr::addr_of!(g::basic_frag_spv).cast::<u8>(),
                    g::basic_frag_spv_size,
                ),
                Shader::basic_oit_frag => (
                    ptr::addr_of!(g::basic_oit_frag_spv).cast::<u8>(),
                    g::basic_oit_frag_spv_size,
                ),
                Shader::basic_mboit_moment_frag => (
                    ptr::addr_of!(g::basic_mboit_moment_frag_spv).cast::<u8>(),
                    g::basic_mboit_moment_frag_spv_size,
                ),
                Shader::basic_mboit_composite_frag => (
                    ptr::addr_of!(g::basic_mboit_composite_frag_spv).cast::<u8>(),
                    g::basic_mboit_composite_frag_spv_size,
                ),
                Shader::basic_mboit_composite_msaa_frag => (
                    ptr::addr_of!(g::basic_mboit_composite_msaa_frag_spv).cast::<u8>(),
                    g::basic_mboit_composite_msaa_frag_spv_size,
                ),
                Shader::basic_alphatest_frag => (
                    ptr::addr_of!(g::basic_alphatest_frag_spv).cast::<u8>(),
                    g::basic_alphatest_frag_spv_size,
                ),
                Shader::basic_notex_frag => (
                    ptr::addr_of!(g::basic_notex_frag_spv).cast::<u8>(),
                    g::basic_notex_frag_spv_size,
                ),
                Shader::world_vert => (
                    ptr::addr_of!(g::world_vert_spv).cast::<u8>(),
                    g::world_vert_spv_size,
                ),
                Shader::world_frag => (
                    ptr::addr_of!(g::world_frag_spv).cast::<u8>(),
                    g::world_frag_spv_size,
                ),
                Shader::world_oit_frag => (
                    ptr::addr_of!(g::world_oit_frag_spv).cast::<u8>(),
                    g::world_oit_frag_spv_size,
                ),
                Shader::world_mboit_moment_frag => (
                    ptr::addr_of!(g::world_mboit_moment_frag_spv).cast::<u8>(),
                    g::world_mboit_moment_frag_spv_size,
                ),
                Shader::world_mboit_composite_frag => (
                    ptr::addr_of!(g::world_mboit_composite_frag_spv).cast::<u8>(),
                    g::world_mboit_composite_frag_spv_size,
                ),
                Shader::world_mboit_composite_msaa_frag => (
                    ptr::addr_of!(g::world_mboit_composite_msaa_frag_spv).cast::<u8>(),
                    g::world_mboit_composite_msaa_frag_spv_size,
                ),
                Shader::alias_vert => (
                    ptr::addr_of!(g::alias_vert_spv).cast::<u8>(),
                    g::alias_vert_spv_size,
                ),
                Shader::alias_frag => (
                    ptr::addr_of!(g::alias_frag_spv).cast::<u8>(),
                    g::alias_frag_spv_size,
                ),
                Shader::alias_alphatest_frag => (
                    ptr::addr_of!(g::alias_alphatest_frag_spv).cast::<u8>(),
                    g::alias_alphatest_frag_spv_size,
                ),
                Shader::alias_oit_frag => (
                    ptr::addr_of!(g::alias_oit_frag_spv).cast::<u8>(),
                    g::alias_oit_frag_spv_size,
                ),
                Shader::alias_alphatest_oit_frag => (
                    ptr::addr_of!(g::alias_alphatest_oit_frag_spv).cast::<u8>(),
                    g::alias_alphatest_oit_frag_spv_size,
                ),
                Shader::alias_mboit_moment_frag => (
                    ptr::addr_of!(g::alias_mboit_moment_frag_spv).cast::<u8>(),
                    g::alias_mboit_moment_frag_spv_size,
                ),
                Shader::alias_mboit_composite_frag => (
                    ptr::addr_of!(g::alias_mboit_composite_frag_spv).cast::<u8>(),
                    g::alias_mboit_composite_frag_spv_size,
                ),
                Shader::alias_mboit_composite_msaa_frag => (
                    ptr::addr_of!(g::alias_mboit_composite_msaa_frag_spv).cast::<u8>(),
                    g::alias_mboit_composite_msaa_frag_spv_size,
                ),
                Shader::alias_alphatest_mboit_moment_frag => (
                    ptr::addr_of!(g::alias_alphatest_mboit_moment_frag_spv).cast::<u8>(),
                    g::alias_alphatest_mboit_moment_frag_spv_size,
                ),
                Shader::alias_alphatest_mboit_composite_frag => (
                    ptr::addr_of!(g::alias_alphatest_mboit_composite_frag_spv).cast::<u8>(),
                    g::alias_alphatest_mboit_composite_frag_spv_size,
                ),
                Shader::alias_alphatest_mboit_composite_msaa_frag => (
                    ptr::addr_of!(g::alias_alphatest_mboit_composite_msaa_frag_spv).cast::<u8>(),
                    g::alias_alphatest_mboit_composite_msaa_frag_spv_size,
                ),
                Shader::md5_mboit_composite_frag => (
                    ptr::addr_of!(g::md5_mboit_composite_frag_spv).cast::<u8>(),
                    g::md5_mboit_composite_frag_spv_size,
                ),
                Shader::md5_mboit_composite_msaa_frag => (
                    ptr::addr_of!(g::md5_mboit_composite_msaa_frag_spv).cast::<u8>(),
                    g::md5_mboit_composite_msaa_frag_spv_size,
                ),
                Shader::md5_alphatest_mboit_composite_frag => (
                    ptr::addr_of!(g::md5_alphatest_mboit_composite_frag_spv).cast::<u8>(),
                    g::md5_alphatest_mboit_composite_frag_spv_size,
                ),
                Shader::md5_alphatest_mboit_composite_msaa_frag => (
                    ptr::addr_of!(g::md5_alphatest_mboit_composite_msaa_frag_spv).cast::<u8>(),
                    g::md5_alphatest_mboit_composite_msaa_frag_spv_size,
                ),
                Shader::md5_vert => (
                    ptr::addr_of!(g::md5_vert_spv).cast::<u8>(),
                    g::md5_vert_spv_size,
                ),
                Shader::md5_8_vert => (
                    ptr::addr_of!(g::md5_8_vert_spv).cast::<u8>(),
                    g::md5_8_vert_spv_size,
                ),
                Shader::sky_layer_vert => (
                    ptr::addr_of!(g::sky_layer_vert_spv).cast::<u8>(),
                    g::sky_layer_vert_spv_size,
                ),
                Shader::sky_layer_frag => (
                    ptr::addr_of!(g::sky_layer_frag_spv).cast::<u8>(),
                    g::sky_layer_frag_spv_size,
                ),
                Shader::sky_box_frag => (
                    ptr::addr_of!(g::sky_box_frag_spv).cast::<u8>(),
                    g::sky_box_frag_spv_size,
                ),
                Shader::sky_cube_vert => (
                    ptr::addr_of!(g::sky_cube_vert_spv).cast::<u8>(),
                    g::sky_cube_vert_spv_size,
                ),
                Shader::sky_cube_frag => (
                    ptr::addr_of!(g::sky_cube_frag_spv).cast::<u8>(),
                    g::sky_cube_frag_spv_size,
                ),
                Shader::postprocess_vert => (
                    ptr::addr_of!(g::postprocess_vert_spv).cast::<u8>(),
                    g::postprocess_vert_spv_size,
                ),
                Shader::postprocess_frag => (
                    ptr::addr_of!(g::postprocess_frag_spv).cast::<u8>(),
                    g::postprocess_frag_spv_size,
                ),
                Shader::wboit_resolve_frag => (
                    ptr::addr_of!(g::wboit_resolve_frag_spv).cast::<u8>(),
                    g::wboit_resolve_frag_spv_size,
                ),
                Shader::wboit_resolve_msaa_frag => (
                    ptr::addr_of!(g::wboit_resolve_msaa_frag_spv).cast::<u8>(),
                    g::wboit_resolve_msaa_frag_spv_size,
                ),
                Shader::mboit_resolve_frag => (
                    ptr::addr_of!(g::mboit_resolve_frag_spv).cast::<u8>(),
                    g::mboit_resolve_frag_spv_size,
                ),
                Shader::mboit_resolve_msaa_frag => (
                    ptr::addr_of!(g::mboit_resolve_msaa_frag_spv).cast::<u8>(),
                    g::mboit_resolve_msaa_frag_spv_size,
                ),
                Shader::screen_effects_8bit_comp => (
                    ptr::addr_of!(g::screen_effects_8bit_comp_spv).cast::<u8>(),
                    g::screen_effects_8bit_comp_spv_size,
                ),
                Shader::screen_effects_8bit_scale_comp => (
                    ptr::addr_of!(g::screen_effects_8bit_scale_comp_spv).cast::<u8>(),
                    g::screen_effects_8bit_scale_comp_spv_size,
                ),
                Shader::screen_effects_8bit_scale_sops_comp => (
                    ptr::addr_of!(g::screen_effects_8bit_scale_sops_comp_spv).cast::<u8>(),
                    g::screen_effects_8bit_scale_sops_comp_spv_size,
                ),
                Shader::screen_effects_10bit_comp => (
                    ptr::addr_of!(g::screen_effects_10bit_comp_spv).cast::<u8>(),
                    g::screen_effects_10bit_comp_spv_size,
                ),
                Shader::screen_effects_10bit_scale_comp => (
                    ptr::addr_of!(g::screen_effects_10bit_scale_comp_spv).cast::<u8>(),
                    g::screen_effects_10bit_scale_comp_spv_size,
                ),
                Shader::screen_effects_10bit_scale_sops_comp => (
                    ptr::addr_of!(g::screen_effects_10bit_scale_sops_comp_spv).cast::<u8>(),
                    g::screen_effects_10bit_scale_sops_comp_spv_size,
                ),
                Shader::cs_tex_warp_comp => (
                    ptr::addr_of!(g::cs_tex_warp_comp_spv).cast::<u8>(),
                    g::cs_tex_warp_comp_spv_size,
                ),
                Shader::indirect_comp => (
                    ptr::addr_of!(g::indirect_comp_spv).cast::<u8>(),
                    g::indirect_comp_spv_size,
                ),
                Shader::indirect_clear_comp => (
                    ptr::addr_of!(g::indirect_clear_comp_spv).cast::<u8>(),
                    g::indirect_clear_comp_spv_size,
                ),
                Shader::showtris_vert => (
                    ptr::addr_of!(g::showtris_vert_spv).cast::<u8>(),
                    g::showtris_vert_spv_size,
                ),
                Shader::showtris_frag => (
                    ptr::addr_of!(g::showtris_frag_spv).cast::<u8>(),
                    g::showtris_frag_spv_size,
                ),
                Shader::update_lightmap_8bit_comp => (
                    ptr::addr_of!(g::update_lightmap_8bit_comp_spv).cast::<u8>(),
                    g::update_lightmap_8bit_comp_spv_size,
                ),
                Shader::update_lightmap_10bit_comp => (
                    ptr::addr_of!(g::update_lightmap_10bit_comp_spv).cast::<u8>(),
                    g::update_lightmap_10bit_comp_spv_size,
                ),
                Shader::update_lightmap_8bit_rt_comp => (
                    ptr::addr_of!(g::update_lightmap_8bit_rt_comp_spv).cast::<u8>(),
                    g::update_lightmap_8bit_rt_comp_spv_size,
                ),
                Shader::update_lightmap_10bit_rt_comp => (
                    ptr::addr_of!(g::update_lightmap_10bit_rt_comp_spv).cast::<u8>(),
                    g::update_lightmap_10bit_rt_comp_spv_size,
                ),
                Shader::ray_debug_comp => (
                    ptr::addr_of!(g::ray_debug_comp_spv).cast::<u8>(),
                    g::ray_debug_comp_spv_size,
                ),
                Shader::mesh_interpolate_comp => (
                    ptr::addr_of!(g::mesh_interpolate_comp_spv).cast::<u8>(),
                    g::mesh_interpolate_comp_spv_size,
                ),
                Shader::skinning_comp => (
                    ptr::addr_of!(g::skinning_comp_spv).cast::<u8>(),
                    g::skinning_comp_spv_size,
                ),
                Shader::skinning_8_comp => (
                    ptr::addr_of!(g::skinning_8_comp_spv).cast::<u8>(),
                    g::skinning_8_comp_spv_size,
                ),
            }
        };
        // SAFETY: `<name>_spv` is a `bintoc` `const unsigned char[]` of exactly
        // `<name>_spv_size` bytes with static storage duration.
        unsafe { slice::from_raw_parts(data, len as usize) }
    }

    fn renderhash(&self) -> bool {
        // SAFETY: `harness_renderhash` is a `qboolean` set once at startup.
        unsafe { *ptr::addr_of!(g::harness_renderhash) }
    }
}

/// Builds the [`Ctx`] over the C-visible globals and runs `f` with it.
///
/// ADR-007 dual view: while `f` runs, `vulkan_globals` is reachable both
/// through `ctx.vg` and through the exported symbol, and that is not
/// single-threaded: the staging, memory and ring allocators run on task
/// workers (`gl_model.c` miptex loads, `r_alias.c` uniform allocations) while
/// the main thread touches the same object, and the C callbacks the port
/// makes (`GL_WaitForDeviceIdle` writes `device_idle` and re-enters
/// `R_SubmitStagingBuffers`, `GL_SetObjectName` reads `device`/`debug_utils`,
/// `TexMgr_UpdateTextureDescriptorSets` reads the samplers) touch it through
/// the symbol. So no `&`/`&mut VulkanGlobals` is formed here: `ctx.vg` is a
/// raw-pointer [`VgPtr`] and the port reads and writes single fields through
/// it (ADR-004; `device_idle` atomically). Recorded in the ADR-007 table row.
fn with_ctx<R>(f: impl FnOnce(&mut Ctx<'_, CEngine>) -> R) -> R {
    let engine = CEngine;
    // SAFETY: the exported static is live and aligned for the whole process;
    // the `VgPtr` access discipline is what the port's modules follow.
    let vg = unsafe { VgPtr::from_raw(ptr::addr_of_mut!(vulkan_globals)) };
    let mut ctx = Ctx {
        engine: &engine,
        device: device(),
        vg,
        counters: Counters {
            misc: &num_vulkan_misc_allocations,
            dynbuf: &num_vulkan_dynbuf_allocations,
            combined_image_samplers: &num_vulkan_combined_image_samplers,
            ubos_dynamic: &num_vulkan_ubos_dynamic,
            ubos: &num_vulkan_ubos,
            storage_buffers: &num_vulkan_storage_buffers,
            input_attachments: &num_vulkan_input_attachments,
            storage_images: &num_vulkan_storage_images,
            sampled_images: &num_vulkan_sampled_images,
            acceleration_structures: &num_acceleration_structures,
            total_device: &total_device_vulkan_allocation_size,
            total_host: &total_host_vulkan_allocation_size,
        },
    };
    f(&mut ctx)
}

/// The `atomic_uint32_t *num_allocations` out-parameter (nullable).
///
/// # Safety
/// `p` is null or points to a live `atomic_uint32_t`.
unsafe fn counter<'a>(p: *mut c_void) -> Option<&'a AtomicU32> {
    // SAFETY: per the contract; `AtomicU32` is layout-compatible with the C
    // atomic and all its accesses are atomic.
    unsafe { p.cast::<AtomicU32>().as_ref() }
}

/// A `const char *name` that C always passes non-null.
///
/// # Safety
/// `p` points to a NUL-terminated string.
unsafe fn name_str<'a>(p: *const c_char) -> &'a str {
    // SAFETY: per the contract.
    unsafe { CStr::from_ptr(p) }.to_str().unwrap_or("?")
}

/* ---- glquake.h:826-833 ---- */

#[no_mangle]
pub extern "C" fn GL_MemoryTypeFromProperties(
    type_bits: u32,
    requirements_mask: u32,
    preferred_mask: u32,
) -> c_int {
    with_ctx(|ctx| {
        ctx.memory_type_from_properties(
            type_bits,
            vk::MemoryPropertyFlags::from_raw(requirements_mask),
            vk::MemoryPropertyFlags::from_raw(preferred_mask),
        ) as c_int
    })
}

#[no_mangle]
pub extern "C" fn R_CreateDescriptorPool() {
    with_ctx(rmisc::create_descriptor_pool)
}

#[no_mangle]
pub extern "C" fn R_CreateDescriptorSetLayouts() {
    with_ctx(rmisc::create_descriptor_set_layouts)
}

#[no_mangle]
pub extern "C" fn R_InitSamplers() {
    with_ctx(rmisc::init_samplers)
}

#[no_mangle]
pub extern "C" fn R_CreatePipelineLayouts() {
    with_ctx(rmisc::create_pipeline_layouts)
}

/// `R_CreatePipelines`: creates the shader modules, every pipeline family,
/// then destroys the modules again -- so the `*_module` statics never
/// outlive the call and live on the stack here.
#[no_mangle]
pub extern "C" fn R_CreatePipelines() {
    let mut modules = ShaderModules::new();
    with_ctx(|ctx| rmisc::create_pipelines(ctx, &mut modules))
}

#[no_mangle]
pub extern "C" fn R_DestroyPipelines() {
    with_ctx(rmisc::destroy_pipelines)
}

/* ---- glquake.h:885-901 (memory and buffers) ---- */

/// # Safety
/// `memory` and `memory_allocate_info` are valid; `num_allocations` is null
/// or a live `atomic_uint32_t`.
#[no_mangle]
pub unsafe extern "C" fn R_AllocateVulkanMemory(
    memory: *mut VulkanMemory,
    memory_allocate_info: *const vk::MemoryAllocateInfo<'_>,
    memory_type: VulkanMemoryType,
    num_allocations: *mut c_void,
) {
    // SAFETY: per the contract.
    let (memory, info, counter) = unsafe {
        (
            &mut *memory,
            &*memory_allocate_info,
            counter(num_allocations),
        )
    };
    with_ctx(|ctx| rmisc::allocate_vulkan_memory(ctx, memory, info, memory_type, counter))
}

/// # Safety
/// As [`R_AllocateVulkanMemory`].
#[no_mangle]
pub unsafe extern "C" fn R_FreeVulkanMemory(
    memory: *mut VulkanMemory,
    num_allocations: *mut c_void,
) {
    // SAFETY: per the contract.
    let (memory, counter) = unsafe { (&mut *memory, counter(num_allocations)) };
    with_ctx(|ctx| rmisc::free_vulkan_memory(ctx, memory, counter))
}

/// # Safety
/// `buffer`, `memory` and `name` are valid; `device_address` is null or a
/// valid out-pointer; `num_allocations` as above.
#[no_mangle]
pub unsafe extern "C" fn R_CreateBuffer(
    buffer: *mut vk::Buffer,
    memory: *mut VulkanMemory,
    size: usize,
    usage: vk::BufferUsageFlags,
    mem_requirements_mask: u32,
    mem_preferred_mask: u32,
    num_allocations: *mut c_void,
    device_address: *mut vk::DeviceAddress,
    name: *const c_char,
) {
    // SAFETY: per the contract.
    let (memory, counter, name) =
        unsafe { (&mut *memory, counter(num_allocations), name_str(name)) };
    let (handle, address) = with_ctx(|ctx| {
        rmisc::create_buffer(
            ctx,
            memory,
            size as u64,
            usage,
            vk::MemoryPropertyFlags::from_raw(mem_requirements_mask),
            vk::MemoryPropertyFlags::from_raw(mem_preferred_mask),
            counter,
            !device_address.is_null(),
            name,
        )
    });
    // SAFETY: `buffer` is a valid out-pointer; `device_address` is non-null
    // whenever `address` is `Some` (it is only requested through it).
    unsafe {
        *buffer = handle;
        if let Some(address) = address {
            *device_address = address;
        }
    }
}

/// # Safety
/// As [`R_AllocateVulkanMemory`].
#[no_mangle]
pub unsafe extern "C" fn R_FreeBuffer(
    buffer: vk::Buffer,
    memory: *mut VulkanMemory,
    num_allocations: *mut c_void,
) {
    // SAFETY: per the contract.
    let (memory, counter) = unsafe { (&mut *memory, counter(num_allocations)) };
    with_ctx(|ctx| rmisc::free_buffer(ctx, buffer, memory, counter))
}

/// # Safety
/// `create_infos` points to `num_buffers` valid `buffer_create_info_t`
/// entries whose `buffer` out-pointers are valid and whose `mapped`/`address`
/// out-pointers are null or valid; `memory` and `memory_name` are valid.
#[no_mangle]
pub unsafe extern "C" fn R_CreateBuffers(
    num_buffers: c_int,
    create_infos: *mut BufferCreateInfo,
    memory: *mut VulkanMemory,
    mem_requirements_mask: u32,
    mem_preferred_mask: u32,
    num_allocations: *mut c_void,
    memory_name: *const c_char,
) -> usize {
    // SAFETY: per the contract.
    let (infos, memory, counter, memory_name) = unsafe {
        (
            slice::from_raw_parts_mut(create_infos, num_buffers.max(0) as usize),
            &mut *memory,
            counter(num_allocations),
            CStr::from_ptr(memory_name),
        )
    };
    let (size, results) = with_ctx(|ctx| {
        // C ORs `VK_BUFFER_USAGE_SHADER_DEVICE_ADDRESS_BIT_KHR` into the
        // caller's `create_infos[i].usage` in place; keep that visible.
        let has_get_address = ctx.has_buffer_device_address();
        for ci in infos.iter_mut() {
            if has_get_address && !ci.address.is_null() {
                ci.usage |= vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS_KHR;
            }
        }
        let requests: Vec<BufferRequest<'_>> = infos
            .iter()
            .map(|ci| BufferRequest {
                size: ci.size as u64,
                alignment: ci.alignment as u64,
                usage: ci.usage,
                mapped: !ci.mapped.is_null(),
                address: !ci.address.is_null(),
                // SAFETY: `name` is a NUL-terminated string (contract).
                name: unsafe { name_str(ci.name) },
            })
            .collect();
        rmisc::create_buffers(
            ctx,
            &requests,
            memory,
            vk::MemoryPropertyFlags::from_raw(mem_requirements_mask),
            vk::MemoryPropertyFlags::from_raw(mem_preferred_mask),
            counter,
            memory_name,
        )
    });
    for (ci, result) in infos.iter().zip(&results) {
        // SAFETY: the out-pointers are valid per the contract; `mapped` and
        // `address` are written only when the caller asked (non-null).
        unsafe {
            *ci.buffer = result.buffer;
            if !ci.mapped.is_null() {
                *ci.mapped = result.mapped;
            }
            if let Some(address) = result.address {
                *ci.address = address;
            }
        }
    }
    size as usize
}

/// # Safety
/// `buffers` points to `num_buffers` handles; `memory`/`num_allocations` as
/// [`R_AllocateVulkanMemory`].
#[no_mangle]
pub unsafe extern "C" fn R_FreeBuffers(
    num_buffers: c_int,
    buffers: *mut vk::Buffer,
    memory: *mut VulkanMemory,
    num_allocations: *mut c_void,
) {
    // SAFETY: per the contract.
    let (buffers, memory, counter) = unsafe {
        (
            slice::from_raw_parts(buffers, num_buffers.max(0) as usize),
            &mut *memory,
            counter(num_allocations),
        )
    };
    with_ctx(|ctx| rmisc::free_buffers(ctx, buffers, memory, counter))
}

/* ---- glquake.h:902-903 (descriptor sets) ---- */

/// # Safety
/// `layout` points to a live `vulkan_desc_set_layout_t`.
#[no_mangle]
pub unsafe extern "C" fn R_AllocateDescriptorSet(
    layout: *mut VulkanDescSetLayout,
) -> vk::DescriptorSet {
    // SAFETY: per the contract; the layout is only read.
    let layout = unsafe { &*layout };
    with_ctx(|ctx| rmisc::allocate_descriptor_set(ctx, layout))
}

/// # Safety
/// As [`R_AllocateDescriptorSet`].
#[no_mangle]
pub unsafe extern "C" fn R_FreeDescriptorSet(
    desc_set: vk::DescriptorSet,
    layout: *mut VulkanDescSetLayout,
) {
    // SAFETY: per the contract; the layout is only read.
    let layout = unsafe { &*layout };
    with_ctx(|ctx| rmisc::free_descriptor_set(ctx, desc_set, layout))
}

/* ---- glquake.h:905-910 (staging) ---- */

#[no_mangle]
pub extern "C" fn R_InitStagingBuffers() {
    with_ctx(|ctx| STAGING.init(ctx))
}

#[no_mangle]
pub extern "C" fn R_SubmitStagingBuffers() {
    with_ctx(|ctx| STAGING.submit(ctx))
}

/// Writes `value` through an optional out-pointer: the C
/// `R_DynBufferAllocate` null-checks every out-parameter, and
/// `r_brush.c` passes `NULL` for the buffer and offset of the TLAS
/// instance buffer.
///
/// # Safety
/// `out` is null or valid for writes.
unsafe fn write_opt<T>(out: *mut T, value: T) {
    if !out.is_null() {
        // SAFETY: non-null per the contract.
        unsafe { *out = value };
    }
}

/// # Safety
/// `cb_context`, `buffer` and `buffer_offset` are null or valid
/// out-pointers.
#[no_mangle]
pub unsafe extern "C" fn R_StagingAllocate(
    size: c_int,
    alignment: c_int,
    cb_context: *mut vk::CommandBuffer,
    buffer: *mut vk::Buffer,
    buffer_offset: *mut c_int,
) -> *mut u8 {
    let allocation = with_ctx(|ctx| STAGING.allocate(ctx, size, alignment));
    // SAFETY: per the contract.
    unsafe {
        write_opt(cb_context, allocation.command_buffer);
        write_opt(buffer, allocation.buffer);
        write_opt(buffer_offset, allocation.buffer_offset);
    }
    allocation.data
}

#[no_mangle]
pub extern "C" fn R_StagingBeginCopy() {
    STAGING.begin_copy()
}

#[no_mangle]
pub extern "C" fn R_StagingEndCopy() {
    STAGING.end_copy()
}

/// # Safety
/// `data` points to `size` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn R_StagingUploadBuffer(buffer: vk::Buffer, size: usize, data: *const u8) {
    // SAFETY: per the contract.
    let data = unsafe { slice::from_raw_parts(data, size) };
    with_ctx(|ctx| STAGING.upload_buffer(ctx, buffer, data))
}

/* ---- glquake.h:912-929 (dynamic buffers) ---- */

#[no_mangle]
pub extern "C" fn R_InitGPUBuffers() {
    with_ctx(|ctx| DYN.init_gpu_buffers(ctx, &STAGING))
}

/// # Safety
/// `buffers` points to `num_buffers` `dynbuffer_t`s; `descriptor_sets` is
/// null or points to two descriptor sets (`gl_rmisc.c:1141`).
#[no_mangle]
pub unsafe extern "C" fn R_AddDynamicBufferGarbage(
    memory: VulkanMemory,
    buffers: *mut DynBuffer,
    num_buffers: c_int,
    descriptor_sets: *mut vk::DescriptorSet,
) {
    // SAFETY: per the contract.
    let buffers: Vec<vk::Buffer> =
        unsafe { slice::from_raw_parts(buffers, num_buffers.max(0) as usize) }
            .iter()
            .map(|b| b.buffer)
            .collect();
    let sets = if descriptor_sets.is_null() {
        None
    } else {
        // SAFETY: non-null means exactly two sets (contract).
        Some(unsafe { slice::from_raw_parts(descriptor_sets, 2) })
    };
    DYN.add_garbage(memory, &buffers, sets)
}

#[no_mangle]
pub extern "C" fn R_SwapDynamicBuffers() {
    DYN.swap()
}

const _: () = assert!(size_of::<VulkanMemory>() == size_of::<[u64; 3]>());

#[no_mangle]
pub extern "C" fn R_FlushDynamicBuffers() {
    // SAFETY: `frame_upload_buffers_memory` (`r_brush.c`) is a
    // `vulkan_memory_t` whose first word is the `VkDeviceMemory` handle
    // (size asserted above); a plain read on the main thread.
    let frame_upload = unsafe { (*ptr::addr_of!(g::frame_upload_buffers_memory))[0] };
    with_ctx(|ctx| DYN.flush(ctx, vk::DeviceMemory::from_raw(frame_upload)))
}

#[no_mangle]
pub extern "C" fn R_CollectDynamicBufferGarbage() {
    with_ctx(|ctx| DYN.collect_garbage(ctx))
}

/// # Safety
/// `buffer` and `buffer_offset` are null or valid out-pointers.
#[no_mangle]
pub unsafe extern "C" fn R_VertexAllocate(
    size: c_int,
    buffer: *mut vk::Buffer,
    buffer_offset: *mut vk::DeviceSize,
) -> *mut u8 {
    let a = with_ctx(|ctx| DYN.vertex_allocate(ctx, size as u32));
    // SAFETY: per the contract.
    unsafe {
        write_opt(buffer, a.buffer);
        write_opt(buffer_offset, a.buffer_offset);
    }
    a.data
}

/// # Safety
/// As [`R_VertexAllocate`].
#[no_mangle]
pub unsafe extern "C" fn R_IndexAllocate(
    size: c_int,
    buffer: *mut vk::Buffer,
    buffer_offset: *mut vk::DeviceSize,
) -> *mut u8 {
    let a = with_ctx(|ctx| DYN.index_allocate(ctx, size as u32));
    // SAFETY: per the contract.
    unsafe {
        write_opt(buffer, a.buffer);
        write_opt(buffer_offset, a.buffer_offset);
    }
    a.data
}

/// # Safety
/// `buffer`, `buffer_offset` and `descriptor_set` are null or valid
/// out-pointers.
#[no_mangle]
pub unsafe extern "C" fn R_UniformAllocate(
    size: c_int,
    buffer: *mut vk::Buffer,
    buffer_offset: *mut u32,
    descriptor_set: *mut vk::DescriptorSet,
) -> *mut u8 {
    let a = with_ctx(|ctx| DYN.uniform_allocate(ctx, size as u32));
    // SAFETY: per the contract.
    unsafe {
        write_opt(buffer, a.buffer);
        write_opt(buffer_offset, a.buffer_offset as u32);
        write_opt(descriptor_set, a.descriptor_set);
    }
    a.data
}

/// # Safety
/// `buffer`, `buffer_offset` and `device_address` are null or valid
/// out-pointers.
#[no_mangle]
pub unsafe extern "C" fn R_StorageAllocate(
    size: c_int,
    buffer: *mut vk::Buffer,
    buffer_offset: *mut vk::DeviceSize,
    device_address: *mut vk::DeviceAddress,
) -> *mut u8 {
    let a = with_ctx(|ctx| DYN.storage_allocate(ctx, size as u32));
    // SAFETY: per the contract.
    unsafe {
        write_opt(buffer, a.buffer);
        write_opt(buffer_offset, a.buffer_offset);
        write_opt(device_address, a.device_address);
    }
    a.data
}
