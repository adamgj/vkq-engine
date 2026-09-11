//! Descriptor set layouts, the descriptor pool and the counted
//! allocate/free pair (`R_CreateDescriptorSetLayouts`,
//! `R_CreateDescriptorPool`, `R_AllocateDescriptorSet`,
//! `R_FreeDescriptorSet`).

use core::ffi::CStr;
use core::sync::atomic::Ordering::SeqCst;

use ash::vk;
use quake_types::render::{
    VulkanDescSetLayout, MAXLIGHTMAPS, MAX_GLTEXTURES, MAX_SANITY_LIGHTMAPS,
    MIN_NB_DESCRIPTORS_PER_TYPE,
};

use super::{Ctx, Engine};

fn binding(
    binding: u32,
    count: u32,
    ty: vk::DescriptorType,
    stages: vk::ShaderStageFlags,
) -> vk::DescriptorSetLayoutBinding<'static> {
    vk::DescriptorSetLayoutBinding::default()
        .binding(binding)
        .descriptor_count(count)
        .descriptor_type(ty)
        .stage_flags(stages)
}

/// One `vkCreateDescriptorSetLayout` block of `R_CreateDescriptorSetLayouts`:
/// the layout is zeroed, `counts` filled in, the handle created and named.
fn create_layout<E: Engine>(
    ctx: &Ctx<'_, E>,
    layout: &mut VulkanDescSetLayout,
    bindings: &[vk::DescriptorSetLayoutBinding<'_>],
    flags: vk::DescriptorSetLayoutCreateFlags,
    name: &CStr,
    counts: impl FnOnce(&mut VulkanDescSetLayout),
) {
    let info = vk::DescriptorSetLayoutCreateInfo::default()
        .flags(flags)
        .bindings(bindings);
    *layout = VulkanDescSetLayout::default();
    counts(layout);
    // SAFETY: `info` borrows the live `bindings` slice; the device is live.
    layout.handle = match unsafe { ctx.device.create_descriptor_set_layout(&info, None) } {
        Ok(handle) => handle,
        Err(err) => ctx.vk_fail("vkCreateDescriptorSetLayout", err),
    };
    ctx.name_object(layout.handle, name);
}

/// `R_CreateDescriptorSetLayouts`.
pub fn create_descriptor_set_layouts<E: Engine>(ctx: &mut Ctx<'_, E>) {
    ctx.engine.sys_printf("Creating descriptor set layouts\n");
    use vk::DescriptorType as T;
    use vk::ShaderStageFlags as S;
    let none = vk::DescriptorSetLayoutCreateFlags::empty();

    let mut layout = VulkanDescSetLayout::default();

    create_layout(
        ctx,
        &mut layout,
        &[binding(
            0,
            1,
            T::COMBINED_IMAGE_SAMPLER,
            S::FRAGMENT | S::COMPUTE,
        )],
        none,
        c"single texture",
        |l| {
            l.num_combined_image_samplers = 1;
        },
    );
    *vg_mut!(ctx, single_texture_set_layout) = layout;

    create_layout(
        ctx,
        &mut layout,
        &[binding(0, 1, T::UNIFORM_BUFFER_DYNAMIC, S::ALL_GRAPHICS)],
        none,
        c"single dynamic UBO",
        |l| {
            l.num_ubos_dynamic = 1;
        },
    );
    *vg_mut!(ctx, ubo_set_layout) = layout;

    create_layout(
        ctx,
        &mut layout,
        &[binding(0, 1, T::STORAGE_BUFFER, S::ALL_GRAPHICS)],
        none,
        c"joints buffer",
        |l| {
            l.num_storage_buffers = 1;
        },
    );
    *vg_mut!(ctx, joints_buffer_set_layout) = layout;

    create_layout(
        ctx,
        &mut layout,
        &[binding(0, 1, T::INPUT_ATTACHMENT, S::FRAGMENT)],
        none,
        c"input attachment",
        |l| {
            l.num_input_attachments = 1;
        },
    );
    *vg_mut!(ctx, input_attachment_set_layout) = layout;

    let oit: Vec<_> = (0..2)
        .map(|i| binding(i, 1, T::INPUT_ATTACHMENT, S::FRAGMENT))
        .collect();
    create_layout(ctx, &mut layout, &oit, none, c"oit input attachment", |l| {
        l.num_input_attachments = 2;
    });
    *vg_mut!(ctx, oit_input_attachment_set_layout) = layout;

    let mboit: Vec<_> = (0..3)
        .map(|i| binding(i, 1, T::INPUT_ATTACHMENT, S::FRAGMENT))
        .collect();
    create_layout(
        ctx,
        &mut layout,
        &mboit,
        none,
        c"mboit input attachment",
        |l| {
            l.num_input_attachments = 3;
        },
    );
    *vg_mut!(ctx, mboit_input_attachment_set_layout) = layout;

    let screen_effects = [
        binding(0, 1, T::COMBINED_IMAGE_SAMPLER, S::COMPUTE),
        binding(1, 1, T::COMBINED_IMAGE_SAMPLER, S::COMPUTE),
        binding(2, 1, T::STORAGE_IMAGE, S::COMPUTE),
        binding(3, 1, T::UNIFORM_TEXEL_BUFFER, S::COMPUTE),
        binding(4, 1, T::UNIFORM_BUFFER, S::COMPUTE),
    ];
    create_layout(
        ctx,
        &mut layout,
        &screen_effects,
        none,
        c"screen effects",
        |l| {
            l.num_combined_image_samplers = 2;
            l.num_storage_images = 1;
        },
    );
    *vg_mut!(ctx, screen_effects_set_layout) = layout;

    create_layout(
        ctx,
        &mut layout,
        &[binding(0, 1, T::STORAGE_IMAGE, S::COMPUTE)],
        none,
        c"single storage image",
        |l| {
            l.num_storage_images = 1;
        },
    );
    *vg_mut!(ctx, single_texture_cs_write_set_layout) = layout;

    let lightmap_compute = [
        binding(0, 1, T::STORAGE_IMAGE, S::COMPUTE),
        binding(1, 1, T::SAMPLED_IMAGE, S::COMPUTE),
        binding(2, MAXLIGHTMAPS * 3 / 4, T::SAMPLED_IMAGE, S::COMPUTE),
        binding(3, 1, T::STORAGE_BUFFER, S::COMPUTE),
        binding(4, 1, T::STORAGE_BUFFER, S::COMPUTE),
        binding(5, 1, T::UNIFORM_BUFFER_DYNAMIC, S::COMPUTE),
        binding(6, 1, T::UNIFORM_BUFFER_DYNAMIC, S::COMPUTE),
        binding(7, 1, T::STORAGE_BUFFER, S::COMPUTE),
        binding(8, 1, T::STORAGE_BUFFER, S::COMPUTE),
        binding(9, 1, T::STORAGE_BUFFER, S::COMPUTE),
    ];
    create_layout(
        ctx,
        &mut layout,
        &lightmap_compute,
        none,
        c"lightmap compute",
        |l| {
            l.num_storage_images = 1;
            l.num_sampled_images = (1 + MAXLIGHTMAPS * 3 / 4) as _;
            l.num_storage_buffers = 5;
            l.num_ubos_dynamic = 2;
        },
    );
    *vg_mut!(ctx, lightmap_compute_set_layout) = layout;

    let indirect_compute: Vec<_> = (0..6)
        .map(|i| binding(i, 1, T::STORAGE_BUFFER, S::COMPUTE))
        .collect();
    create_layout(
        ctx,
        &mut layout,
        &indirect_compute,
        none,
        c"indirect compute",
        |l| {
            l.num_storage_buffers = 6;
        },
    );
    *vg_mut!(ctx, indirect_compute_set_layout) = layout;

    let bmodel_instances: Vec<_> = (0..2)
        .map(|i| binding(i, 1, T::STORAGE_BUFFER, S::VERTEX))
        .collect();
    create_layout(
        ctx,
        &mut layout,
        &bmodel_instances,
        none,
        c"bmodel instances",
        |l| {
            l.num_storage_buffers = 2;
        },
    );
    *vg_mut!(ctx, bmodel_instances_set_layout) = layout;

    if vg!(ctx, ray_query) {
        create_layout(
            ctx,
            &mut layout,
            &[binding(0, 1, T::ACCELERATION_STRUCTURE_KHR, S::COMPUTE)],
            vk::DescriptorSetLayoutCreateFlags::PUSH_DESCRIPTOR_KHR,
            c"ray query push",
            |_| {},
        );
        *vg_mut!(ctx, ray_query_push_set_layout) = layout;
    }

    #[cfg(feature = "engine-debug")]
    if vg!(ctx, ray_query) {
        create_layout(
            ctx,
            &mut layout,
            &[binding(0, 1, T::STORAGE_IMAGE, S::COMPUTE)],
            none,
            c"ray debug",
            |l| {
                l.num_storage_images = 1;
            },
        );
        *vg_mut!(ctx, ray_debug_set_layout) = layout;
    }
}

/// The eight `VkDescriptorPoolSize`s of `R_CreateDescriptorPool`.
pub fn pool_sizes() -> [vk::DescriptorPoolSize; 8] {
    use vk::DescriptorType as T;
    let min = MIN_NB_DESCRIPTORS_PER_TYPE;
    let lightmaps = MAX_SANITY_LIGHTMAPS;
    let textures = MAX_GLTEXTURES;
    let size = |ty, count| {
        vk::DescriptorPoolSize::default()
            .ty(ty)
            .descriptor_count(count)
    };
    [
        size(
            T::COMBINED_IMAGE_SAMPLER,
            min + (lightmaps * 2) + (textures + 1),
        ),
        size(T::STORAGE_IMAGE, min + textures + lightmaps),
        size(T::UNIFORM_TEXEL_BUFFER, min),
        size(T::UNIFORM_BUFFER, min),
        size(T::STORAGE_BUFFER, min + lightmaps * 2),
        size(T::UNIFORM_BUFFER_DYNAMIC, min + (lightmaps * 2)),
        size(T::INPUT_ATTACHMENT, min),
        size(
            T::SAMPLED_IMAGE,
            min + (1 + MAXLIGHTMAPS * 3 / 4) * lightmaps,
        ),
    ]
}

/// `maxSets` of `R_CreateDescriptorPool`.
pub const MAX_SETS: u32 = MAX_GLTEXTURES + MAX_SANITY_LIGHTMAPS + 128;

/// `R_CreateDescriptorPool`. The result is unchecked, as in C.
pub fn create_descriptor_pool<E: Engine>(ctx: &mut Ctx<'_, E>) {
    let sizes = pool_sizes();
    let info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(MAX_SETS)
        .pool_sizes(&sizes)
        .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET);
    // SAFETY: `info` borrows the live `sizes` array; the device is live.
    *vg_mut!(ctx, descriptor_pool) =
        unsafe { ctx.device.create_descriptor_pool(&info, None) }.unwrap_or_default();
}

/// `R_AllocateDescriptorSet`. The allocation result is unchecked, as in C;
/// a failure yields a null handle here.
pub fn allocate_descriptor_set<E: Engine>(
    ctx: &Ctx<'_, E>,
    layout: &VulkanDescSetLayout,
) -> vk::DescriptorSet {
    let layouts = [layout.handle];
    let info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(vg!(ctx, descriptor_pool))
        .set_layouts(&layouts);
    // SAFETY: `info` borrows the live `layouts` array; pool and device are live.
    let handle = unsafe { ctx.device.allocate_descriptor_sets(&info) }
        .ok()
        .and_then(|sets| sets.first().copied())
        .unwrap_or_default();
    let c = ctx.counters;
    c.combined_image_samplers
        .fetch_add(layout.num_combined_image_samplers as u32, SeqCst);
    c.ubos_dynamic
        .fetch_add(layout.num_ubos_dynamic as u32, SeqCst);
    c.ubos.fetch_add(layout.num_ubos as u32, SeqCst);
    c.storage_buffers
        .fetch_add(layout.num_storage_buffers as u32, SeqCst);
    c.input_attachments
        .fetch_add(layout.num_input_attachments as u32, SeqCst);
    c.storage_images
        .fetch_add(layout.num_storage_images as u32, SeqCst);
    c.sampled_images
        .fetch_add(layout.num_sampled_images as u32, SeqCst);
    c.acceleration_structures
        .fetch_add(layout.num_acceleration_structures as u32, SeqCst);
    handle
}

/// `R_FreeDescriptorSet`.
pub fn free_descriptor_set<E: Engine>(
    ctx: &Ctx<'_, E>,
    desc_set: vk::DescriptorSet,
    layout: &VulkanDescSetLayout,
) {
    let pool = vg!(ctx, descriptor_pool);
    // SAFETY: `desc_set` came from `pool` (FREE_DESCRIPTOR_SET) and the
    // caller guarantees no submission still references it.
    let _ = unsafe { ctx.device.free_descriptor_sets(pool, &[desc_set]) };
    let c = ctx.counters;
    c.combined_image_samplers
        .fetch_sub(layout.num_combined_image_samplers as u32, SeqCst);
    c.ubos_dynamic
        .fetch_sub(layout.num_ubos_dynamic as u32, SeqCst);
    c.ubos.fetch_sub(layout.num_ubos as u32, SeqCst);
    c.storage_buffers
        .fetch_sub(layout.num_storage_buffers as u32, SeqCst);
    c.input_attachments
        .fetch_sub(layout.num_input_attachments as u32, SeqCst);
    c.storage_images
        .fetch_sub(layout.num_storage_images as u32, SeqCst);
    c.sampled_images
        .fetch_sub(layout.num_sampled_images as u32, SeqCst);
    c.acceleration_structures
        .fetch_sub(layout.num_acceleration_structures as u32, SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_sizes_match_c_constants() {
        let sizes = pool_sizes();
        assert_eq!(sizes[0].descriptor_count, 32 + 512 + 65537);
        assert_eq!(sizes[1].descriptor_count, 32 + 65536 + 256);
        assert_eq!(sizes[2].descriptor_count, 32);
        assert_eq!(sizes[3].descriptor_count, 32);
        assert_eq!(sizes[4].descriptor_count, 32 + 512);
        assert_eq!(sizes[5].descriptor_count, 32 + 512);
        assert_eq!(sizes[6].descriptor_count, 32);
        assert_eq!(sizes[7].descriptor_count, 32 + (1 + 3) * 256);
        assert_eq!(MAX_SETS, 65536 + 256 + 128);
    }
}
