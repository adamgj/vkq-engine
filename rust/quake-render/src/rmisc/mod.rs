//! `gl_rmisc.c` (Phase 8 M5): device memory, staging and dynamic ring
//! buffers, descriptor set layouts/pool, samplers, shader modules, pipeline
//! layouts and the pipeline families.
//!
//! Everything here is a method over a [`Ctx`]: the engine callbacks, the
//! ash device, the Rust-owned `vulkan_globals` (ADR-007 dual view, the C
//! layout is exported by `quake-capi`) and the allocation counters that C
//! still reads. The C-only remainder of the file (cvar callbacks, `R_Init`,
//! player skins, worldspawn parsing, `vkmemstats`) stays in
//! `Quake/gl_rmisc_glue.c`.

use core::ffi::CStr;
use core::sync::atomic::{AtomicU32, AtomicU64};

use ash::vk;
use quake_types::render::VulkanGlobals;

pub mod descriptors;
pub mod dynbuf;
pub mod memory;
pub mod pipelines;
pub mod samplers;
pub mod shaders;
pub mod staging;

pub use descriptors::{
    allocate_descriptor_set, create_descriptor_pool, create_descriptor_set_layouts,
    free_descriptor_set,
};
pub use dynbuf::DynBuffers;
pub use memory::{
    allocate_vulkan_memory, create_buffer, create_buffers, free_buffer, free_buffers,
    free_vulkan_memory, memory_type_from_properties, BufferRequest, BufferResult,
};
pub use pipelines::{create_pipeline_layouts, create_pipelines, destroy_pipelines};
pub use samplers::init_samplers;
pub use shaders::Shader;
pub use staging::Staging;

/// The engine services `gl_rmisc.c` reaches through `Sys_*`/`Con_*`, the
/// texmgr, the harness and the `r_lodbias`/`gl_lodbias`/`r_scale` cvars.
pub trait Engine {
    /// `Sys_Error`: terminates, never returns (ADR-009).
    fn sys_error(&self, msg: &str) -> !;
    /// `Sys_Printf`.
    fn sys_printf(&self, msg: &str);
    /// `Con_Printf`.
    fn con_printf(&self, msg: &str);
    /// `GL_SetObjectName` (a no-op outside `_DEBUG`).
    fn set_object_name(&self, handle: u64, object_type: vk::ObjectType, name: &CStr);
    /// `Harness_RenderPipelineCreated` when `-renderhash` is active.
    fn pipeline_created(&self, handle: u64, name: &CStr);
    /// `Harness_RenderPipelinesDestroyed` when `-renderhash` is active.
    fn pipelines_destroyed(&self);
    /// `GL_WaitForDeviceIdle`.
    fn wait_for_device_idle(&self);
    /// `TexMgr_UpdateTextureDescriptorSets`.
    fn update_texture_descriptor_sets(&self);
    /// `r_lodbias.value`.
    fn r_lodbias(&self) -> f32;
    /// `gl_lodbias.value`.
    fn gl_lodbias(&self) -> f32;
    /// `r_scale.value`.
    fn r_scale(&self) -> f32;
    /// The SPIR-V blob `bintoc` linked for `shader`.
    fn shader_spv(&self, shader: Shader) -> &[u8];
    /// `harness_renderhash`: whether [`Engine::pipeline_created`] is wanted.
    fn renderhash(&self) -> bool;
}

/// The `atomic_uint32_t`/`atomic_uint64_t` allocation counters that stay
/// C-visible symbols (exported by `quake-capi`).
#[derive(Clone, Copy)]
pub struct Counters<'a> {
    pub misc: &'a AtomicU32,
    pub dynbuf: &'a AtomicU32,
    pub combined_image_samplers: &'a AtomicU32,
    pub ubos_dynamic: &'a AtomicU32,
    pub ubos: &'a AtomicU32,
    pub storage_buffers: &'a AtomicU32,
    pub input_attachments: &'a AtomicU32,
    pub storage_images: &'a AtomicU32,
    pub sampled_images: &'a AtomicU32,
    pub acceleration_structures: &'a AtomicU32,
    pub total_device: &'a AtomicU64,
    pub total_host: &'a AtomicU64,
}

/// Everything a `gl_rmisc.c` function reaches for.
pub struct Ctx<'a, E: Engine> {
    pub engine: &'a E,
    pub device: &'a ash::Device,
    pub vg: &'a mut VulkanGlobals,
    pub counters: Counters<'a>,
}

impl<E: Engine> Ctx<'_, E> {
    /// `Sys_Error ("<what> failed with code %i", err)`.
    pub fn vk_fail(&self, what: &str, err: vk::Result) -> ! {
        self.engine
            .sys_error(&format!("{what} failed with code {}", err.as_raw()))
    }

    /// `GL_SetObjectName` over an ash handle.
    pub fn name_object<H: vk::Handle>(&self, handle: H, name: &CStr) {
        self.engine.set_object_name(handle.as_raw(), H::TYPE, name);
    }

    /// `vulkan_globals.vk_get_buffer_device_address (device, &info)`: the
    /// engine's loaded entry point, not ash's, so the two builds resolve the
    /// same function.
    pub fn buffer_device_address(&self, buffer: vk::Buffer) -> vk::DeviceAddress {
        let Some(get) = self.vg.vk_get_buffer_device_address else {
            self.engine
                .sys_error("vkGetBufferDeviceAddress is not loaded");
        };
        let info = vk::BufferDeviceAddressInfo::default().buffer(buffer);
        // SAFETY: `get` is the entry point `gl_vidsdl.c` loaded from the
        // device `vg.device` names; `info` is a complete, live structure.
        unsafe { get(self.vg.device, &info) }
    }
}

/// `q_align`: round `value` up to a multiple of the power-of-two `alignment`.
#[inline]
pub fn q_align(value: u64, alignment: u64) -> u64 {
    debug_assert!(alignment.is_power_of_two());
    (value + alignment - 1) & !(alignment - 1)
}

/// `Q_nextPow2`: the smallest power of two not below `val` (`val` itself
/// when it is already one; 1 for 0 and 1).
#[inline]
pub fn q_next_pow2(val: u32) -> u32 {
    if val > 1 {
        1u32 << (32 - (val - 1).leading_zeros())
    } else {
        1
    }
}

/// Turn a `&'static str` literal into a `&'static CStr` at compile time.
pub(crate) const fn cstr(s: &'static str) -> &'static CStr {
    match CStr::from_bytes_with_nul(s.as_bytes()) {
        Ok(c) => c,
        Err(_) => panic!("shader name is not NUL-terminated"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align_and_pow2_match_mathlib() {
        assert_eq!(q_align(0, 256), 0);
        assert_eq!(q_align(1, 256), 256);
        assert_eq!(q_align(256, 256), 256);
        assert_eq!(q_align(257, 256), 512);
        assert_eq!(q_align(7, 1), 7);
        assert_eq!(q_next_pow2(0), 1);
        assert_eq!(q_next_pow2(1), 1);
        assert_eq!(q_next_pow2(2), 2);
        assert_eq!(q_next_pow2(3), 4);
        assert_eq!(q_next_pow2(1025), 2048);
        assert_eq!(q_next_pow2(1 << 20), 1 << 20);
    }
}
