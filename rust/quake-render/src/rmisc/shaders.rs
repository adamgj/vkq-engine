//! The 61 shader modules (`R_CreateShaderModule`, `R_CreateShaderModules`,
//! `R_DestroyShaderModules`). The SPIR-V blobs themselves are the `bintoc`
//! arrays C links; [`Engine::shader_spv`] hands them over.

use core::ffi::CStr;

use ash::vk;

use super::{cstr, Ctx, Engine};

/// When `R_CreateShaderModules` creates a module.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cond {
    Always,
    /// `vulkan_globals.sample_count != VK_SAMPLE_COUNT_1_BIT`
    Msaa,
    /// `vulkan_globals.screen_effects_sops`
    Sops,
    /// `vulkan_globals.ray_query`
    RayQuery,
    /// `vulkan_globals.ray_query` under `_DEBUG` only
    DebugRayQuery,
}

macro_rules! shaders {
    ($($id:ident => $name:literal $cond:ident),* $(,)?) => {
        /// One `DECLARE_SHADER_MODULE` entry, in `R_CreateShaderModules` order.
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        #[repr(u8)]
        #[allow(non_camel_case_types)]
        pub enum Shader {
            $($id,)*
        }

        impl Shader {
            pub const ALL: [Shader; shaders!(@count $($id)*)] = [$(Shader::$id,)*];

            /// The `bintoc` symbol stem / `GL_SetObjectName` label.
            pub const fn name(self) -> &'static CStr {
                match self {
                    $(Shader::$id => cstr(concat!($name, "\0")),)*
                }
            }

            pub const fn cond(self) -> Cond {
                match self {
                    $(Shader::$id => Cond::$cond,)*
                }
            }
        }
    };
    (@count) => { 0usize };
    (@count $head:ident $($tail:ident)*) => { 1usize + shaders!(@count $($tail)*) };
}

shaders! {
    basic_vert => "basic_vert" Always,
    basic_frag => "basic_frag" Always,
    basic_oit_frag => "basic_oit_frag" Always,
    basic_mboit_moment_frag => "basic_mboit_moment_frag" Always,
    basic_mboit_composite_frag => "basic_mboit_composite_frag" Always,
    basic_mboit_composite_msaa_frag => "basic_mboit_composite_msaa_frag" Msaa,
    basic_alphatest_frag => "basic_alphatest_frag" Always,
    basic_notex_frag => "basic_notex_frag" Always,
    world_vert => "world_vert" Always,
    world_frag => "world_frag" Always,
    world_oit_frag => "world_oit_frag" Always,
    world_mboit_moment_frag => "world_mboit_moment_frag" Always,
    world_mboit_composite_frag => "world_mboit_composite_frag" Always,
    world_mboit_composite_msaa_frag => "world_mboit_composite_msaa_frag" Msaa,
    alias_vert => "alias_vert" Always,
    alias_frag => "alias_frag" Always,
    alias_alphatest_frag => "alias_alphatest_frag" Always,
    alias_oit_frag => "alias_oit_frag" Always,
    alias_alphatest_oit_frag => "alias_alphatest_oit_frag" Always,
    alias_mboit_moment_frag => "alias_mboit_moment_frag" Always,
    alias_mboit_composite_frag => "alias_mboit_composite_frag" Always,
    alias_mboit_composite_msaa_frag => "alias_mboit_composite_msaa_frag" Msaa,
    alias_alphatest_mboit_moment_frag => "alias_alphatest_mboit_moment_frag" Always,
    alias_alphatest_mboit_composite_frag => "alias_alphatest_mboit_composite_frag" Always,
    alias_alphatest_mboit_composite_msaa_frag => "alias_alphatest_mboit_composite_msaa_frag" Msaa,
    md5_mboit_composite_frag => "md5_mboit_composite_frag" Always,
    md5_mboit_composite_msaa_frag => "md5_mboit_composite_msaa_frag" Msaa,
    md5_alphatest_mboit_composite_frag => "md5_alphatest_mboit_composite_frag" Always,
    md5_alphatest_mboit_composite_msaa_frag => "md5_alphatest_mboit_composite_msaa_frag" Msaa,
    md5_vert => "md5_vert" Always,
    md5_8_vert => "md5_8_vert" Always,
    sky_layer_vert => "sky_layer_vert" Always,
    sky_layer_frag => "sky_layer_frag" Always,
    sky_box_frag => "sky_box_frag" Always,
    sky_cube_vert => "sky_cube_vert" Always,
    sky_cube_frag => "sky_cube_frag" Always,
    postprocess_vert => "postprocess_vert" Always,
    postprocess_frag => "postprocess_frag" Always,
    wboit_resolve_frag => "wboit_resolve_frag" Always,
    wboit_resolve_msaa_frag => "wboit_resolve_msaa_frag" Msaa,
    mboit_resolve_frag => "mboit_resolve_frag" Always,
    mboit_resolve_msaa_frag => "mboit_resolve_msaa_frag" Msaa,
    screen_effects_8bit_comp => "screen_effects_8bit_comp" Always,
    screen_effects_8bit_scale_comp => "screen_effects_8bit_scale_comp" Always,
    screen_effects_8bit_scale_sops_comp => "screen_effects_8bit_scale_sops_comp" Sops,
    screen_effects_10bit_comp => "screen_effects_10bit_comp" Always,
    screen_effects_10bit_scale_comp => "screen_effects_10bit_scale_comp" Always,
    screen_effects_10bit_scale_sops_comp => "screen_effects_10bit_scale_sops_comp" Sops,
    cs_tex_warp_comp => "cs_tex_warp_comp" Always,
    indirect_comp => "indirect_comp" Always,
    indirect_clear_comp => "indirect_clear_comp" Always,
    showtris_vert => "showtris_vert" Always,
    showtris_frag => "showtris_frag" Always,
    update_lightmap_8bit_comp => "update_lightmap_8bit_comp" Always,
    update_lightmap_10bit_comp => "update_lightmap_10bit_comp" Always,
    update_lightmap_8bit_rt_comp => "update_lightmap_8bit_rt_comp" RayQuery,
    update_lightmap_10bit_rt_comp => "update_lightmap_10bit_rt_comp" RayQuery,
    ray_debug_comp => "ray_debug_comp" DebugRayQuery,
    mesh_interpolate_comp => "mesh_interpolate_comp" RayQuery,
    skinning_comp => "skinning_comp" RayQuery,
    skinning_8_comp => "skinning_8_comp" RayQuery,
}

pub const SHADER_COUNT: usize = Shader::ALL.len();

/// The `*_module` statics.
pub struct ShaderModules {
    modules: [vk::ShaderModule; SHADER_COUNT],
}

impl Default for ShaderModules {
    fn default() -> Self {
        Self::new()
    }
}

impl ShaderModules {
    pub const fn new() -> Self {
        ShaderModules {
            modules: [vk::ShaderModule::null(); SHADER_COUNT],
        }
    }

    /// `name##_module`.
    pub fn get(&self, shader: Shader) -> vk::ShaderModule {
        self.modules[shader as usize]
    }

    /// `R_CreateShaderModule`.
    fn create<E: Engine>(ctx: &Ctx<'_, E>, shader: Shader) -> vk::ShaderModule {
        let bytes = ctx.engine.shader_spv(shader);
        // `bintoc` emits `unsigned char[]`, so the blob is not 4-aligned;
        // copy into words rather than casting. The C passes `codeSize`
        // verbatim and lets validation reject a non-multiple-of-4 blob;
        // `chunks_exact` would silently drop the tail instead.
        debug_assert_eq!(
            bytes.len() % 4,
            0,
            "{}: SPIR-V size is not a multiple of 4",
            shader.name().to_string_lossy()
        );
        let code: Vec<u32> = bytes
            .chunks_exact(4)
            .map(|w| u32::from_ne_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        let info = vk::ShaderModuleCreateInfo::default().code(&code);
        // SAFETY: `info` borrows the live `code` vector; the device is live.
        let module = match unsafe { ctx.device.create_shader_module(&info, None) } {
            Ok(module) => module,
            Err(err) => ctx.vk_fail("vkCreateShaderModule", err),
        };
        ctx.name_object(module, shader.name());
        module
    }

    /// `R_CreateShaderModules`.
    pub fn create_all<E: Engine>(&mut self, ctx: &Ctx<'_, E>) {
        let msaa = vg!(ctx, sample_count) != vk::SampleCountFlags::TYPE_1;
        let sops = vg!(ctx, screen_effects_sops);
        let ray_query = vg!(ctx, ray_query);
        for shader in Shader::ALL {
            let wanted = match shader.cond() {
                Cond::Always => true,
                Cond::Msaa => msaa,
                Cond::Sops => sops,
                Cond::RayQuery => ray_query,
                Cond::DebugRayQuery => cfg!(feature = "engine-debug") && ray_query,
            };
            self.modules[shader as usize] = if wanted {
                Self::create(ctx, shader)
            } else {
                vk::ShaderModule::null()
            };
        }
    }

    /// `R_DestroyShaderModules`.
    pub fn destroy_all<E: Engine>(&mut self, ctx: &Ctx<'_, E>) {
        for module in &mut self.modules {
            if *module != vk::ShaderModule::null() {
                // SAFETY: pipelines were destroyed first (`R_DestroyPipelines`
                // order), so nothing references the module any more.
                unsafe { ctx.device.destroy_shader_module(*module, None) };
            }
            *module = vk::ShaderModule::null();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixty_one_shaders_in_c_order() {
        assert_eq!(SHADER_COUNT, 61);
        assert_eq!(Shader::ALL[0], Shader::basic_vert);
        assert_eq!(Shader::ALL[60], Shader::skinning_8_comp);
        assert_eq!(Shader::ray_debug_comp.name(), c"ray_debug_comp");
        assert_eq!(Shader::ray_debug_comp.cond(), Cond::DebugRayQuery);
        assert_eq!(
            Shader::ALL
                .iter()
                .filter(|s| s.cond() == Cond::Msaa)
                .count(),
            8
        );
        assert_eq!(
            Shader::ALL
                .iter()
                .filter(|s| s.cond() == Cond::RayQuery)
                .count(),
            5
        );
        assert_eq!(
            Shader::ALL
                .iter()
                .filter(|s| s.cond() == Cond::Sops)
                .count(),
            2
        );
    }
}
