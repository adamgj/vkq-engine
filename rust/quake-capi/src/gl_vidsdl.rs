//! C ABI for the Vulkan half of `gl_vidsdl.c` (Rust migration Phase 8 M6,
//! ADR-015): instance, device, swap chain, render resources and the per-frame
//! begin/end rendering tasks live in `quake_render::vid`; this module owns
//! their state, implements the [`VidEngine`] seams over the C that remains
//! (`gl_vidsdl_glue.c` keeps the SDL window, modes, cvars and video menu) and
//! exports the entry points `glquake.h` declares.
//!
//! `use_rust_render` is orthogonal to `use_rust_tasks` and `use_rust_host`, so
//! the frame tasks go through the `Task_*` C ABI and the client/server state
//! the C file read directly (`cl.time`, `r_refdef.vieworg`, `sv.active`,
//! `cls.signon`) comes through `VID_Glue_*` accessors.

use core::ffi::{c_char, c_int, c_uint, c_void, CStr};
use core::mem::size_of;
use core::ptr;
use core::slice;
use std::ffi::CString;

use ash::vk::{self, Handle};
use quake_c_sys as c;
use quake_c_sys::render as g;
use quake_render::rmisc::{DynBuffers, Engine, Staging};
use quake_render::vid::frame::{self, EndRenderingParms};
use quake_render::vid::{self, instance, resources, VidEngine, VidState};
use quake_types::render::{CbContext, GlTexture};

use crate::gl_rmisc::{vulkan_globals, with_ctx, CEngine, DEVICE, DYN, STAGING};

/* ---- State ---- */

/// The file-scope statics of `gl_vidsdl.c`'s Vulkan half (instance, device
/// tables, swap chain, command pools/buffers, attachments, ...). Only the
/// main thread and the end-rendering task touch it, never concurrently:
/// every entry point below either runs on the main thread after
/// `GL_SynchronizeEndRenderingTask` or *is* the end-rendering task. The one
/// exception is `GL_BeginRendering(use_tasks = true)`, which (like
/// `gl_vidsdl.c:3473-3509`) overlaps the previous frame's end-rendering task
/// (`gl_screen.c` only orders that task before the *begin task*) and reads
/// `render_resources_created` through [`render_resources_created`] -- a raw
/// field read, never a `&mut VidState` -- and the task never writes that
/// field.
static mut VID: VidState = VidState::new();

/// `task_handle_t prev_end_rendering_task` (`gl_vidsdl.c:228`,
/// `glquake.h:701`). Rust-owned from M6; `gl_screen.c` reads and writes it
/// through this symbol (ADR-007 dual view).
#[no_mangle]
pub static mut prev_end_rendering_task: u64 = INVALID_TASK_HANDLE;

/// `INVALID_TASK_HANDLE` (`tasks.h`).
const INVALID_TASK_HANDLE: u64 = u64::MAX;
/// `TASK_TIMEOUT_INFINITE` (`tasks.h:82-88`): `SDL_MUTEX_MAXWAIT` under SDL2
/// and `(uint32_t)-1` under SDL3, both `0xFFFFFFFF`.
const TASK_TIMEOUT_INFINITE: u32 = u32::MAX;

fn vid_state() -> &'static mut VidState {
    // SAFETY: see the `VID` doc comment -- the callers serialise access.
    unsafe { &mut *ptr::addr_of_mut!(VID) }
}

/// `render_resources_created` without materialising a `&mut VidState`, for
/// the one entry point that may overlap the end-rendering task (see `VID`).
fn render_resources_created() -> bool {
    // SAFETY: a plain read of one field of the static; the only other thread
    // that can be inside `VID` (the end-rendering task) never writes it.
    unsafe { ptr::addr_of!(VID.render_resources_created).read() }
}

/* ---- Engine seams ---- */

impl VidEngine for CEngine {
    fn get_instance_proc_addr(&self) -> vk::PFN_vkGetInstanceProcAddr {
        // SAFETY: `VID_Glue_GetInstanceProcAddr` returns
        // `SDL_Vulkan_GetVkGetInstanceProcAddr()` after `SDL_Vulkan_LoadLibrary`
        // succeeded (`VID_Init` `Sys_Error`s otherwise), which is exactly a
        // `PFN_vkGetInstanceProcAddr`.
        unsafe {
            let raw = g::VID_Glue_GetInstanceProcAddr();
            if raw.is_null() {
                self.sys_error("SDL_Vulkan_GetVkGetInstanceProcAddr returned NULL");
            }
            core::mem::transmute::<*const c_void, vk::PFN_vkGetInstanceProcAddr>(raw)
        }
    }

    fn instance_extensions(&self) -> Vec<CString> {
        let mut count: c_uint = 0;
        // SAFETY: the glue returns SDL's `count`-long array of NUL-terminated
        // names (it `Sys_Error`s on failure), valid until the window is
        // destroyed.
        unsafe {
            let names = g::VID_Glue_InstanceExtensions(&mut count);
            slice::from_raw_parts(names, count as usize)
                .iter()
                .map(|&name| CStr::from_ptr(name).to_owned())
                .collect()
        }
    }

    fn create_surface(&self, instance: vk::Instance) -> vk::SurfaceKHR {
        // SAFETY: `instance` is the live `VkInstance`; the glue `Sys_Error`s
        // when SDL cannot create the surface.
        let raw = unsafe { g::VID_Glue_CreateSurface(instance.as_raw() as usize as *mut c_void) };
        vk::SurfaceKHR::from_raw(raw)
    }

    #[cfg(windows)]
    fn window_monitor(&self) -> *mut c_void {
        // SAFETY: `VID_Glue_WindowMonitor` has no preconditions beyond the
        // window existing, which `VID_Init` guarantees before `GL_InitDevice`.
        unsafe { g::VID_Glue_WindowMonitor() }
    }

    fn has_focus(&self) -> bool {
        // SAFETY: plain SDL window-flag query.
        unsafe { g::VID_Glue_HasFocus() }
    }

    fn fullscreen(&self) -> bool {
        // SAFETY: plain SDL window-flag query.
        unsafe { g::VID_Glue_GetFullscreen() }
    }

    fn device_parm(&self) -> Option<Option<i32>> {
        // SAFETY: `COM_InitArgv` has run long before `VID_Init`, so
        // `com_argc`/`com_argv` are populated and every entry below `com_argc`
        // is a NUL-terminated string (`tasks.rs` precedent).
        unsafe {
            let index = c::COM_CheckParm(c"-device".as_ptr());
            if index == 0 {
                return None;
            }
            let argc = ptr::addr_of!(c::com_argc).read();
            if index >= argc - 1 {
                return Some(None);
            }
            let argv = ptr::addr_of!(c::com_argv).read();
            Some(Some(c::view::atoi(*argv.add(index as usize + 1))))
        }
    }

    fn vid_size(&self) -> (u32, u32) {
        // SAFETY: `vid` is the C `viddef_t`; `VID_Init` filled `width`/`height`
        // before the Vulkan half runs and only the main thread writes them.
        let v = unsafe { &*ptr::addr_of!(c::cl_parse::vid) };
        (v.width as u32, v.height as u32)
    }

    fn set_restart_next_frame(&self) {
        // SAFETY: as above; `restart_next_frame` is a plain `qboolean`.
        unsafe { (*ptr::addr_of_mut!(c::cl_parse::vid)).restart_next_frame = true }
    }

    #[cfg(feature = "engine-debug")]
    fn debug_message_callback(&self) -> vk::PFN_vkDebugUtilsMessengerCallbackEXT {
        Some(debug_message_callback)
    }

    fn install_harness_hooks(&self) {
        // SAFETY: `Harness_RenderInstallHooks` has no preconditions.
        unsafe { g::Harness_RenderInstallHooks() }
    }

    fn double_time(&self) -> f64 {
        // SAFETY: `Sys_DoubleTime` has no preconditions.
        unsafe { c::Sys_DoubleTime() }
    }

    fn sky_need_stencil(&self) -> bool {
        // SAFETY: `Sky_NeedStencil` only reads cvars and the sky state.
        unsafe { g::Sky_NeedStencil() }
    }

    fn set_canvas(&self, cbx: &mut CbContext, canvas: c_int) {
        // SAFETY: `GL_SetCanvas` (`gl_draw.c`) takes a `cb_context_t *`;
        // `CbContext` is its ADR-011 mirror.
        unsafe { c::console::GL_SetCanvas(ptr::from_mut(cbx).cast(), canvas) }
    }

    fn viewport(
        &self,
        cbx: &mut CbContext,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        min_depth: f32,
        max_depth: f32,
    ) {
        // SAFETY: as for `set_canvas`.
        unsafe {
            g::GL_Viewport(
                ptr::from_mut(cbx).cast(),
                x,
                y,
                width,
                height,
                min_depth,
                max_depth,
            )
        }
    }

    fn collect_mesh_buffer_garbage(&self) {
        // SAFETY: `R_CollectMeshBufferGarbage` runs on the main thread with
        // the device idle, which is how the ported caller invokes it.
        unsafe { g::R_CollectMeshBufferGarbage() }
    }

    fn collect_tlas_garbage(&self) {
        // SAFETY: as above.
        unsafe { g::R_CollectTLASGarbage() }
    }

    fn texmgr_collect_garbage(&self) {
        crate::gl_texmgr::TexMgr_CollectGarbage();
    }

    fn frame_oit_mode(&self) -> c_int {
        // SAFETY: `frame_oit_mode` (`gl_rmisc_glue.c`) is a plain `int`-ranked
        // enum written only by `GL_BeginRendering` on the main thread.
        unsafe { ptr::addr_of!(g::frame_oit_mode).read() }
    }

    fn skip_render_resources(&self) -> bool {
        // SAFETY: the glue reads `sv.active`/`cls.signon` on the main thread.
        unsafe { g::VID_Glue_SkipRenderResources() }
    }

    fn vid_vsync(&self) -> f32 {
        // SAFETY: the cvar is defined in `gl_vidsdl_glue.c`; `.value` is a
        // plain float only the main thread writes.
        unsafe { (*ptr::addr_of!(g::vid_vsync)).value }
    }

    fn vid_maxframelatency(&self) -> f32 {
        // SAFETY: as above.
        unsafe { (*ptr::addr_of!(g::vid_maxframelatency)).value }
    }

    fn vid_fsaa(&self) -> f32 {
        // SAFETY: as above.
        unsafe { (*ptr::addr_of!(c::menu::vid_fsaa)).value }
    }

    fn vid_fsaamode(&self) -> f32 {
        // SAFETY: as above.
        unsafe { (*ptr::addr_of!(c::menu::vid_fsaamode)).value }
    }

    fn vid_gamma(&self) -> f32 {
        // SAFETY: as above.
        unsafe { (*ptr::addr_of!(c::menu::vid_gamma)).value }
    }

    fn vid_contrast(&self) -> f32 {
        // SAFETY: as above.
        unsafe { (*ptr::addr_of!(c::menu::vid_contrast)).value }
    }

    fn r_usesops(&self) -> f32 {
        // SAFETY: as above (`gl_rmisc_glue.c`).
        unsafe { (*ptr::addr_of!(g::r_usesops)).value }
    }

    fn add_gpu_wait_us(&self, us: u32) {
        // SAFETY: `rs_gpuwaitaccum_us` (`gl_rmain.c`) is a plain counter the
        // end-rendering task owns while it runs, as in C.
        unsafe { *ptr::addr_of_mut!(g::rs_gpuwaitaccum_us) += us }
    }

    fn set_gpu_time_us(&self, us: u32) {
        // SAFETY: as above for `rs_gputime_us`.
        unsafe { ptr::addr_of_mut!(g::rs_gputime_us).write(us) }
    }

    fn write_screenshot(&self, pixels: &[u8], width: u32, height: u32) {
        debug_assert_eq!(pixels.len(), width as usize * height as usize * 4);
        // SAFETY: `pixels` holds `width * height` RGBA quads for the call.
        unsafe { g::VID_Glue_WriteScreenshot(pixels.as_ptr(), width as c_int, height as c_int) }
    }

    fn bluenoise_image_view(&self) -> vk::ImageView {
        // SAFETY: `bluenoisetexture` is the `gltexture_t *` `gl_texmgr.rs`
        // publishes at `TexMgr_Init` (before any frame renders) and never
        // frees; `GlTexture` is its mirror.
        unsafe {
            let tex = ptr::addr_of!(g::bluenoisetexture)
                .read()
                .cast::<GlTexture>();
            if tex.is_null() {
                vk::ImageView::null()
            } else {
                (*tex).image_view
            }
        }
    }

    fn bmodel_tlas(&self) -> vk::AccelerationStructureKHR {
        // SAFETY: `bmodel_tlas` (`r_brush.c`) is a plain handle read.
        vk::AccelerationStructureKHR::from_raw(unsafe { ptr::addr_of!(g::bmodel_tlas).read() })
    }

    fn staging(&self) -> &Staging {
        &STAGING
    }

    fn dyn_buffers(&self) -> &DynBuffers {
        &DYN
    }

    fn frame_upload_buffers_memory(&self) -> vk::DeviceMemory {
        // SAFETY: `frame_upload_buffers_memory` (`gl_rmain.c`) is a plain
        // handle array.
        vk::DeviceMemory::from_raw(unsafe {
            ptr::addr_of!(g::frame_upload_buffers_memory).read()[0]
        })
    }

    fn synchronize_end_rendering_task(&self) {
        GL_SynchronizeEndRenderingTask();
    }
}

/// `DebugMessageCallback` (`gl_vidsdl.c:207`, `_DEBUG` only).
#[cfg(feature = "engine-debug")]
unsafe extern "system" fn debug_message_callback(
    _message_severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    _message_types: vk::DebugUtilsMessageTypeFlagsEXT,
    callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _user_data: *mut c_void,
) -> vk::Bool32 {
    // SAFETY: the validation layer passes a complete callback-data struct
    // whose `p_message` is a NUL-terminated string for the duration of the
    // call; `Sys_Printf` takes a format and a `%s` argument.
    unsafe { c::Sys_Printf(c"%s\n".as_ptr(), (*callback_data).p_message) };
    vk::FALSE
}

/* ---- Entry points (glquake.h) ---- */

/// `void GL_SetObjectName (uint64_t object, VkObjectType object_type, const
/// char *name)` (`gl_vidsdl.c:750`): a no-op outside `_DEBUG` builds and until
/// `VK_EXT_debug_utils` is loaded.
///
/// # Safety
/// `name` is NULL or a NUL-terminated string.
#[no_mangle]
#[allow(unused_variables)]
pub unsafe extern "C" fn GL_SetObjectName(object: u64, object_type: c_int, name: *const c_char) {
    #[cfg(feature = "engine-debug")]
    {
        let Some(set_name) = vid_state().procs.set_debug_utils_object_name else {
            return;
        };
        if name.is_null() {
            return;
        }
        // SAFETY: `name` is NUL-terminated per the contract; `set_name` was
        // loaded from the live device in `vulkan_globals.device`.
        unsafe {
            let mut info =
                vk::DebugUtilsObjectNameInfoEXT::default().object_name(CStr::from_ptr(name));
            info.object_type = vk::ObjectType::from_raw(object_type);
            info.object_handle = object;
            let _ = set_name((*ptr::addr_of!(vulkan_globals)).device, &info);
        }
    }
}

/// `void GL_UpdateDescriptorSets (void)`.
#[no_mangle]
pub extern "C" fn GL_UpdateDescriptorSets() {
    with_ctx(|ctx| resources::update_descriptor_sets(ctx, vid_state()));
}

/// `void GL_SynchronizeEndRenderingTask (void)` (`gl_vidsdl.c:3450`).
#[no_mangle]
pub extern "C" fn GL_SynchronizeEndRenderingTask() {
    // SAFETY: `prev_end_rendering_task` is only touched from the main thread
    // (here and in `gl_screen.c`); `Task_Join` accepts any handle.
    unsafe {
        let prev = ptr::addr_of_mut!(prev_end_rendering_task);
        if *prev != INVALID_TASK_HANDLE {
            c::tasks::Task_Join(*prev, TASK_TIMEOUT_INFINITE);
            *prev = INVALID_TASK_HANDLE;
        }
    }
}

/// `GL_FrameOITModeForCvarValue` (`gl_vidsdl.c:3464`).
fn frame_oit_mode_for_cvar_value(r_oit_value: c_int) -> c_int {
    if r_oit_value == 1 {
        vid::OIT_MODE_WBOIT
    } else if r_oit_value >= 2 {
        vid::OIT_MODE_MBOIT
    } else {
        vid::OIT_MODE_NONE
    }
}

/// `void GL_BeginRenderingTask (void *unused)` as a `task_func_t`.
unsafe extern "C" fn begin_rendering_task_trampoline(_unused: *mut c_void) {
    with_ctx(|ctx| frame::begin_rendering_task(ctx, vid_state()));
}

/// `qboolean GL_BeginRendering (qboolean use_tasks, task_handle_t
/// *begin_rendering_task, int *width, int *height)` (`gl_vidsdl.c:3473`).
///
/// # Safety
/// `width` and `height` are writable; `begin_rendering_task` is writable when
/// `use_tasks` is set.
#[no_mangle]
pub unsafe extern "C" fn GL_BeginRendering(
    use_tasks: bool,
    begin_rendering_task: *mut u64,
    width: *mut c_int,
    height: *mut c_int,
) -> bool {
    let engine = CEngine;
    if !use_tasks {
        GL_SynchronizeEndRenderingTask();
    }

    // SAFETY: `r_oit`/`frame_oit_mode`/`vid` are main-thread-only C globals
    // (see the seam methods above).
    unsafe {
        let requested_oit_mode =
            frame_oit_mode_for_cvar_value((*ptr::addr_of!(c::menu::r_oit)).value as c_int);
        let oit_mode_changed = requested_oit_mode != engine.frame_oit_mode();
        ptr::addr_of_mut!(g::frame_oit_mode).write(requested_oit_mode);

        let vid_ref = &mut *ptr::addr_of_mut!(c::cl_parse::vid);
        if vid_ref.restart_next_frame || (render_resources_created() && oit_mode_changed) {
            g::VID_Restart(false);
            vid_ref.restart_next_frame = false;
            // Reread: `VID_Restart` re-registers the cvar value.
            ptr::addr_of_mut!(g::frame_oit_mode).write(frame_oit_mode_for_cvar_value(
                (*ptr::addr_of!(c::menu::r_oit)).value as c_int,
            ));
        }
    }

    // `GL_CreateRenderResources` runs only while no end-rendering task can be
    // in flight (nothing was submitted since the resources were destroyed).
    if !render_resources_created() {
        GL_CreateRenderResources();
    }
    if !render_resources_created() {
        return false;
    }

    let (w, h) = engine.vid_size();
    // SAFETY: `width`/`height` are writable per the contract.
    unsafe {
        width.write(w as c_int);
        height.write(h as c_int);
    }

    if use_tasks {
        // SAFETY: the task system is initialised (`Tasks_Init` precedes
        // `VID_Init`); `begin_rendering_task` is writable per the contract.
        unsafe {
            let task = c::tasks::Task_Allocate();
            c::tasks::Task_AssignFunc(
                task,
                Some(begin_rendering_task_trampoline),
                ptr::null_mut(),
                0,
            );
            begin_rendering_task.write(task);
        }
    } else {
        with_ctx(|ctx| frame::begin_rendering_task(ctx, vid_state()));
    }
    true
}

/// `qboolean GL_AcquireNextSwapChainImage (void)`.
#[no_mangle]
pub extern "C" fn GL_AcquireNextSwapChainImage() -> bool {
    with_ctx(|ctx| frame::acquire_next_swap_chain_image(ctx, vid_state()))
}

/// `void GL_EndRenderingTask (gl_end_rendering_parms_t *parms)` as a
/// `task_func_t` over the 128-byte payload copy.
unsafe extern "C" fn end_rendering_task_trampoline(payload: *mut c_void) {
    // SAFETY: `Task_AssignFunc` copied a complete `EndRenderingParms` into the
    // task payload and passes that copy here.
    let parms = unsafe { payload.cast::<EndRenderingParms>().read_unaligned() };
    with_ctx(|ctx| frame::end_rendering_task(ctx, vid_state(), &parms));
}

const _: () = assert!(
    size_of::<EndRenderingParms>() <= 128,
    "task payload is 128 bytes"
);

/// `task_handle_t GL_EndRendering (qboolean use_tasks, qboolean
/// use_swapchain)` (`gl_vidsdl.c:4227`): samples the frame parameters on the
/// main thread and either runs or schedules the end-rendering task.
#[no_mangle]
pub extern "C" fn GL_EndRendering(use_tasks: bool, use_swapchain: bool) -> u64 {
    let engine = CEngine;
    // SAFETY: every global sampled here is a main-thread-only C/Rust global
    // (`vulkan_globals` is read through the exported symbol -- ADR-007 dual
    // view, see `with_ctx`).
    let parms = unsafe {
        let vg = &*ptr::addr_of!(vulkan_globals);
        let vm = &vg.view_matrix;
        let (vid_width, vid_height) = engine.vid_size();
        let mut origin = [0.0f32; 3];
        g::VID_Glue_ViewOrg(origin.as_mut_ptr());
        EndRenderingParms {
            swapchain: use_swapchain,
            use_oit: vid::use_oit(&engine),
            use_mboit: vid::use_mboit(&engine),
            render_warp: ptr::addr_of!(c::view::render_warp).read(),
            vid_palettize: (*ptr::addr_of!(c::menu::vid_palettize)).value != 0.0,
            polyblend: (*ptr::addr_of!(g::gl_polyblend)).value != 0.0,
            menu: ptr::addr_of!(c::menu::key_dest).read() == KEY_MENU,
            #[cfg(feature = "engine-debug")]
            ray_debug: (*ptr::addr_of!(g::r_raydebug)).value != 0.0
                && ptr::addr_of!(g::bmodel_tlas).read() != 0,
            #[cfg(not(feature = "engine-debug"))]
            ray_debug: false,
            screenshot: g::VID_Glue_TakeScreenshot(),
            render_scale: ptr::addr_of!(c::view::render_scale).read() as u32,
            vid_width,
            vid_height,
            time: (g::VID_Glue_ClTime() % (2.0 * core::f64::consts::PI)) as f32,
            color_clear_value: vg.color_clear_value,
            v_blend: ptr::addr_of!(c::view::v_blend).read(),
            origin,
            forward: [-vm[2], -vm[6], -vm[10]],
            right: [vm[0], vm[4], vm[8]],
            down: [-vm[1], -vm[5], -vm[9]],
        }
    };

    if use_tasks {
        // SAFETY: `Task_AssignFunc` copies `size_of::<EndRenderingParms>()`
        // bytes out of `parms` (≤ 128, asserted above) before returning.
        unsafe {
            let task = c::tasks::Task_Allocate();
            c::tasks::Task_AssignFunc(
                task,
                Some(end_rendering_task_trampoline),
                ptr::from_ref(&parms).cast_mut().cast(),
                size_of::<EndRenderingParms>(),
            );
            task
        }
    } else {
        with_ctx(|ctx| frame::end_rendering_task(ctx, vid_state(), &parms));
        INVALID_TASK_HANDLE
    }
}

/// `key_menu` (`keys.h` `keydest_t`).
const KEY_MENU: c_int = 3;

/// `void GL_WaitForDeviceIdle (void)` (`gl_vidsdl.c:4289`).
#[no_mangle]
pub extern "C" fn GL_WaitForDeviceIdle() {
    with_ctx(frame::wait_for_device_idle);
}

/* ---- Entry points (gl_vidsdl_glue.c) ---- */

/// `GL_InitInstance` (`gl_vidsdl.c:773`), the Vulkan half: the SDL window and
/// `SDL_Vulkan_LoadLibrary` are the glue's job before this runs.
#[no_mangle]
pub extern "C" fn GL_InitInstance() {
    // SAFETY: `VID_Init` runs on the main thread before any renderer state
    // exists; `vulkan_globals` is reached through the exported static (see
    // `with_ctx`'s ADR-007 note).
    let vg = unsafe { &mut *ptr::addr_of_mut!(vulkan_globals) };
    instance::init_instance(&CEngine, vg, vid_state());
}

/// `GL_InitDevice` (`gl_vidsdl.c:1029`); also seeds `gl_rmisc.rs`'s device
/// dispatch table with the `ash::Device` the port created.
#[no_mangle]
pub extern "C" fn GL_InitDevice() {
    // SAFETY: as for `GL_InitInstance`.
    let vg = unsafe { &mut *ptr::addr_of_mut!(vulkan_globals) };
    let device = instance::init_device(&CEngine, vg, vid_state());
    let _ = DEVICE.set(device);
}

/// `GL_InitCommandBuffers` (`gl_vidsdl.c:1464`).
#[no_mangle]
pub extern "C" fn GL_InitCommandBuffers() {
    with_ctx(|ctx| instance::init_command_buffers(ctx, vid_state()));
}

/// `GL_CreateRenderResources` (`gl_vidsdl.c:3126`).
#[no_mangle]
pub extern "C" fn GL_CreateRenderResources() {
    with_ctx(|ctx| resources::create_render_resources(ctx, vid_state()));
}

/// `GL_DestroyRenderResources` (`gl_vidsdl.c:3175`).
#[no_mangle]
pub extern "C" fn GL_DestroyRenderResources() {
    with_ctx(|ctx| resources::destroy_render_resources(ctx, vid_state()));
}

/// `void R_CreatePaletteOctreeBuffers (uint32_t *colors, int num_colors,
/// palette_octree_node_t *nodes, int num_nodes)` (`gl_vidsdl.c:4512`).
///
/// # Safety
/// `colors` points at `num_colors` `uint32_t`s and `nodes` at `num_nodes`
/// 32-byte `palette_octree_node_t`s.
#[no_mangle]
pub unsafe extern "C" fn R_CreatePaletteOctreeBuffers(
    colors: *const u32,
    num_colors: c_int,
    nodes: *const c_void,
    num_nodes: c_int,
) {
    /// `sizeof (palette_octree_node_t)` (`palette.h`).
    const PALETTE_OCTREE_NODE_SIZE: usize = 32;
    // SAFETY: the sizes follow from the contract; the byte views are read-only
    // for the call.
    let (colors, nodes) = unsafe {
        (
            slice::from_raw_parts(colors.cast::<u8>(), num_colors as usize * size_of::<u32>()),
            slice::from_raw_parts(
                nodes.cast::<u8>(),
                num_nodes as usize * PALETTE_OCTREE_NODE_SIZE,
            ),
        )
    };
    with_ctx(|ctx| {
        resources::create_palette_octree_buffers(ctx, vid_state(), &STAGING, colors, nodes)
    });
}
