//! `R_InitSamplers`: the four fixed samplers (created once) and the four
//! lod-biased samplers (recreated whenever `r_lodbias`/`gl_lodbias`/`r_scale`
//! change).

use core::ffi::CStr;

use ash::vk;

use super::{Ctx, Engine};

/// The `mipLodBias` `R_InitSamplers` derives from the cvars and the
/// supersampling sample count.
pub fn lod_bias(
    r_lodbias: f32,
    gl_lodbias: f32,
    r_scale: f32,
    supersampling: bool,
    sample_count: vk::SampleCountFlags,
) -> f32 {
    let mut lod_bias = 0.0f32;
    if r_lodbias != 0.0 {
        if supersampling {
            lod_bias -= match sample_count {
                vk::SampleCountFlags::TYPE_2 => 0.5,
                vk::SampleCountFlags::TYPE_4 => 1.0,
                vk::SampleCountFlags::TYPE_8 => 1.5,
                vk::SampleCountFlags::TYPE_16 => 2.0,
                _ => 0.0,
            };
        }
        if r_scale >= 8.0 {
            lod_bias += 3.0;
        } else if r_scale >= 4.0 {
            lod_bias += 2.0;
        } else if r_scale >= 2.0 {
            lod_bias += 1.0;
        }
    }
    lod_bias + gl_lodbias
}

/// Creates the point / point_aniso / linear / linear_aniso quartet with the
/// given `mipLodBias` and name suffix.
fn create_quartet<E: Engine>(
    ctx: &Ctx<'_, E>,
    mip_lod_bias: f32,
    names: [&CStr; 4],
) -> [vk::Sampler; 4] {
    let max_anisotropy = vg!(ctx, device_properties.limits.max_sampler_anisotropy);
    let base = vk::SamplerCreateInfo::default()
        .mag_filter(vk::Filter::NEAREST)
        .min_filter(vk::Filter::NEAREST)
        .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
        .address_mode_u(vk::SamplerAddressMode::REPEAT)
        .address_mode_v(vk::SamplerAddressMode::REPEAT)
        .address_mode_w(vk::SamplerAddressMode::REPEAT)
        .mip_lod_bias(mip_lod_bias)
        .max_anisotropy(1.0)
        .min_lod(0.0)
        .max_lod(f32::MAX);
    let infos = [
        base,
        base.anisotropy_enable(true).max_anisotropy(max_anisotropy),
        base.mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR),
        base.mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .anisotropy_enable(true)
            .max_anisotropy(max_anisotropy),
    ];
    let mut samplers = [vk::Sampler::null(); 4];
    for ((sampler, info), name) in samplers.iter_mut().zip(&infos).zip(names) {
        // SAFETY: `info` is complete and the device is live.
        *sampler = match unsafe { ctx.device.create_sampler(info, None) } {
            Ok(sampler) => sampler,
            Err(err) => ctx.vk_fail("vkCreateSampler", err),
        };
        ctx.name_object(*sampler, name);
    }
    samplers
}

/// `R_InitSamplers`.
pub fn init_samplers<E: Engine>(ctx: &mut Ctx<'_, E>) {
    ctx.engine.wait_for_device_idle();
    ctx.engine.sys_printf("Initializing samplers\n");

    if vg!(ctx, point_sampler) == vk::Sampler::null() {
        let [point, point_aniso, linear, linear_aniso] = create_quartet(
            ctx,
            0.0,
            [c"point", c"point_aniso", c"linear", c"linear_aniso"],
        );
        *vg_mut!(ctx, point_sampler) = point;
        *vg_mut!(ctx, point_aniso_sampler) = point_aniso;
        *vg_mut!(ctx, linear_sampler) = linear;
        *vg_mut!(ctx, linear_aniso_sampler) = linear_aniso;
    }

    if vg!(ctx, point_sampler_lod_bias) != vk::Sampler::null() {
        for sampler in [
            vg!(ctx, point_sampler_lod_bias),
            vg!(ctx, point_aniso_sampler_lod_bias),
            vg!(ctx, linear_sampler_lod_bias),
            vg!(ctx, linear_aniso_sampler_lod_bias),
        ] {
            // SAFETY: the device was idled above, so no submission uses the sampler.
            unsafe { ctx.device.destroy_sampler(sampler, None) };
        }
    }

    let bias = lod_bias(
        ctx.engine.r_lodbias(),
        ctx.engine.gl_lodbias(),
        ctx.engine.r_scale(),
        vg!(ctx, supersampling),
        vg!(ctx, sample_count),
    );
    ctx.engine
        .sys_printf(&format!("Texture lod bias: {bias:.6}\n"));
    let [point, point_aniso, linear, linear_aniso] = create_quartet(
        ctx,
        bias,
        [
            c"point_lod_bias",
            c"point_aniso_lod_bias",
            c"linear_lod_bias",
            c"linear_aniso_lod_bias",
        ],
    );
    *vg_mut!(ctx, point_sampler_lod_bias) = point;
    *vg_mut!(ctx, point_aniso_sampler_lod_bias) = point_aniso;
    *vg_mut!(ctx, linear_sampler_lod_bias) = linear;
    *vg_mut!(ctx, linear_aniso_sampler_lod_bias) = linear_aniso;

    ctx.engine.update_texture_descriptor_sets();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lod_bias_matches_c_switch() {
        assert_eq!(
            lod_bias(0.0, 0.0, 8.0, true, vk::SampleCountFlags::TYPE_16),
            0.0
        );
        assert_eq!(
            lod_bias(0.0, 0.25, 8.0, true, vk::SampleCountFlags::TYPE_16),
            0.25
        );
        assert_eq!(
            lod_bias(1.0, 0.0, 1.0, true, vk::SampleCountFlags::TYPE_4),
            -1.0
        );
        assert_eq!(
            lod_bias(1.0, 0.0, 1.0, false, vk::SampleCountFlags::TYPE_4),
            0.0
        );
        assert_eq!(
            lod_bias(1.0, 0.0, 2.0, true, vk::SampleCountFlags::TYPE_8),
            -0.5
        );
        assert_eq!(
            lod_bias(1.0, 0.5, 4.0, true, vk::SampleCountFlags::TYPE_2),
            2.0
        );
        assert_eq!(
            lod_bias(1.0, 0.0, 8.0, true, vk::SampleCountFlags::TYPE_1),
            3.0
        );
    }
}
