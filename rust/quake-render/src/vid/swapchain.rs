//! `GL_CreateSwapChain` (Phase 8 M6): surface capability/format/present-mode
//! selection, the optional full-screen-exclusive (Windows) and present-wait2
//! probes, swap chain creation with the exclusive-mode retry, and the swap
//! chain image views and semaphores.  Destruction lives in
//! `resources::destroy_render_resources`, as it does in C.

use ash::vk;

use super::{fail, VidEngine, VidState, DOUBLE_BUFFERED, MAX_SWAP_CHAIN_IMAGES};
use super::{
    SurfaceCapabilitiesPresentId2KHR, SurfaceCapabilitiesPresentWait2KHR,
    STRUCTURE_TYPE_SURFACE_CAPABILITIES_PRESENT_ID_2_KHR,
    STRUCTURE_TYPE_SURFACE_CAPABILITIES_PRESENT_WAIT_2_KHR, SWAPCHAIN_CREATE_PRESENT_ID_2_BIT_KHR,
    SWAPCHAIN_CREATE_PRESENT_WAIT_2_BIT_KHR,
};
use crate::rmisc::Ctx;

/// `GL_CreateSwapChain`; `false` when the surface's current extent disagrees
/// with `vid.width`/`vid.height` (the caller retries next frame).
pub fn create_swap_chain<E: VidEngine>(ctx: &mut Ctx<'_, E>, vid: &mut VidState) -> bool {
    let engine = ctx.engine;
    let (vid_width, vid_height) = engine.vid_size();

    #[cfg(windows)]
    let mut use_exclusive_full_screen = false;
    #[cfg(windows)]
    let mut full_screen_exclusive_win32_info =
        vk::SurfaceFullScreenExclusiveWin32InfoEXT::default();
    #[cfg(windows)]
    let mut full_screen_exclusive_info = vk::SurfaceFullScreenExclusiveInfoEXT::default();

    let surface_capabilities: vk::SurfaceCapabilitiesKHR;

    #[cfg(windows)]
    let try_use_exclusive_full_screen = ctx.vg.full_screen_exclusive
        && ctx.vg.want_full_screen_exclusive
        && engine.has_focus()
        && engine.fullscreen();
    #[cfg(not(windows))]
    let try_use_exclusive_full_screen = false;

    if try_use_exclusive_full_screen {
        #[cfg(windows)]
        {
            full_screen_exclusive_win32_info.hmonitor = engine.window_monitor() as vk::HMONITOR;
            full_screen_exclusive_info.full_screen_exclusive =
                vk::FullScreenExclusiveEXT::APPLICATION_CONTROLLED;
            full_screen_exclusive_info.p_next = (&mut full_screen_exclusive_win32_info
                as *mut vk::SurfaceFullScreenExclusiveWin32InfoEXT)
                .cast();

            let mut surface_info_2 =
                vk::PhysicalDeviceSurfaceInfo2KHR::default().surface(vid.surface);
            surface_info_2.p_next = (&full_screen_exclusive_info
                as *const vk::SurfaceFullScreenExclusiveInfoEXT)
                .cast();

            let mut surface_capabilities_full_screen_exclusive =
                vk::SurfaceCapabilitiesFullScreenExclusiveEXT::default();
            let mut surface_capabilities_2 = vk::SurfaceCapabilities2KHR::default()
                .push_next(&mut surface_capabilities_full_screen_exclusive);

            let get_capabilities2 = vid.procs.get_physical_device_surface_capabilities2.expect(
                "VK_EXT_full_screen_exclusive implies vkGetPhysicalDeviceSurfaceCapabilities2KHR",
            );
            // SAFETY: the query structs are complete locals; the physical
            // device and surface are the ones `init_instance`/`init_device`
            // chose.
            let err = unsafe {
                get_capabilities2(
                    vid.physical_device,
                    &surface_info_2,
                    &mut surface_capabilities_2,
                )
            };
            if err != vk::Result::SUCCESS {
                fail(engine, "Couldn't get surface capabilities", err);
            }
            surface_capabilities = surface_capabilities_2.surface_capabilities;
            use_exclusive_full_screen = surface_capabilities_full_screen_exclusive
                .full_screen_exclusive_supported
                != vk::FALSE;
        }
        #[cfg(not(windows))]
        {
            unreachable!();
        }
    } else {
        let get_capabilities = vid
            .procs
            .get_physical_device_surface_capabilities
            .expect("GL_InitInstance loads vkGetPhysicalDeviceSurfaceCapabilitiesKHR");
        let mut capabilities = vk::SurfaceCapabilitiesKHR::default();
        // SAFETY: as above.
        let err = unsafe { get_capabilities(vid.physical_device, vid.surface, &mut capabilities) };
        if err != vk::Result::SUCCESS {
            fail(engine, "Couldn't get surface capabilities", err);
        }
        surface_capabilities = capabilities;
    }

    let current = surface_capabilities.current_extent;
    if (current.width != 0xFFFF_FFFF || current.height != 0xFFFF_FFFF)
        && (current.width != vid_width || current.height != vid_height)
    {
        return false;
    }

    vid.swapchain_present_wait = false;
    if ctx.vg.present_wait {
        let mut present_id_2_capabilities = SurfaceCapabilitiesPresentId2KHR {
            s_type: STRUCTURE_TYPE_SURFACE_CAPABILITIES_PRESENT_ID_2_KHR,
            p_next: core::ptr::null_mut(),
            present_id2_supported: vk::FALSE,
        };
        let mut present_wait_2_capabilities = SurfaceCapabilitiesPresentWait2KHR {
            s_type: STRUCTURE_TYPE_SURFACE_CAPABILITIES_PRESENT_WAIT_2_KHR,
            p_next: core::ptr::null_mut(),
            present_wait2_supported: vk::FALSE,
        };
        let present_wait_surface_info =
            vk::PhysicalDeviceSurfaceInfo2KHR::default().surface(vid.surface);
        let mut surface_capabilities_2 = vk::SurfaceCapabilities2KHR::default()
            .push_next(&mut present_id_2_capabilities)
            .push_next(&mut present_wait_2_capabilities);

        let get_capabilities2 = vid
            .procs
            .get_physical_device_surface_capabilities2
            .expect("present_wait implies vkGetPhysicalDeviceSurfaceCapabilities2KHR");
        // SAFETY: as above; the hand-declared capability structs are
        // `repr(C)` with the `VkBaseOutStructure` prefix.
        let err = unsafe {
            get_capabilities2(
                vid.physical_device,
                &present_wait_surface_info,
                &mut surface_capabilities_2,
            )
        };
        if err == vk::Result::SUCCESS {
            vid.swapchain_present_wait = present_id_2_capabilities.present_id2_supported
                != vk::FALSE
                && present_wait_2_capabilities.present_wait2_supported != vk::FALSE;
        }
    }

    let get_formats = vid
        .procs
        .get_physical_device_surface_formats
        .expect("GL_InitInstance loads vkGetPhysicalDeviceSurfaceFormatsKHR");
    let mut format_count: u32 = 0;
    // SAFETY: count query with a null array.
    let err = unsafe {
        get_formats(
            vid.physical_device,
            vid.surface,
            &mut format_count,
            core::ptr::null_mut(),
        )
    };
    if err != vk::Result::SUCCESS {
        fail(engine, "Couldn't get surface formats", err);
    }
    let mut surface_formats = vec![vk::SurfaceFormatKHR::default(); format_count as usize];
    // SAFETY: `surface_formats` holds `format_count` entries.
    let err = unsafe {
        get_formats(
            vid.physical_device,
            vid.surface,
            &mut format_count,
            surface_formats.as_mut_ptr(),
        )
    };
    if err != vk::Result::SUCCESS {
        fail(engine, "fpGetPhysicalDeviceSurfaceFormatsKHR failed", err);
    }

    let mut swap_chain_format = vk::Format::B8G8R8A8_UNORM;
    let mut swap_chain_color_space = vk::ColorSpaceKHR::SRGB_NONLINEAR;
    if surface_formats[0].format != vk::Format::UNDEFINED || format_count > 1 {
        let found_wanted_format = surface_formats[..format_count as usize]
            .iter()
            .any(|f| f.format == swap_chain_format && f.color_space == swap_chain_color_space);
        if !found_wanted_format {
            swap_chain_format = surface_formats[0].format;
            swap_chain_color_space = surface_formats[0].color_space;
        }
    }

    let get_present_modes = vid
        .procs
        .get_physical_device_surface_present_modes
        .expect("GL_InitInstance loads vkGetPhysicalDeviceSurfacePresentModesKHR");
    let mut present_mode_count: u32 = 0;
    // SAFETY: count query with a null array.
    let err = unsafe {
        get_present_modes(
            vid.physical_device,
            vid.surface,
            &mut present_mode_count,
            core::ptr::null_mut(),
        )
    };
    if err != vk::Result::SUCCESS {
        fail(
            engine,
            "fpGetPhysicalDeviceSurfacePresentModesKHR failed",
            err,
        );
    }
    let mut present_modes = vec![vk::PresentModeKHR::default(); present_mode_count as usize];
    // SAFETY: `present_modes` holds `present_mode_count` entries.
    let err = unsafe {
        get_present_modes(
            vid.physical_device,
            vid.surface,
            &mut present_mode_count,
            present_modes.as_mut_ptr(),
        )
    };
    if err != vk::Result::SUCCESS {
        fail(
            engine,
            "fpGetPhysicalDeviceSurfacePresentModesKHR failed",
            err,
        );
    }

    let mut present_mode = vk::PresentModeKHR::FIFO;
    if engine.vid_vsync() == 0.0 {
        let mut found_immediate = false;
        let mut found_mailbox = false;
        for mode in &present_modes[..present_mode_count as usize] {
            if *mode == vk::PresentModeKHR::IMMEDIATE {
                found_immediate = true;
            }
            if *mode == vk::PresentModeKHR::MAILBOX {
                found_mailbox = true;
            }
        }
        if found_mailbox {
            present_mode = vk::PresentModeKHR::MAILBOX;
        }
        if found_immediate {
            present_mode = vk::PresentModeKHR::IMMEDIATE;
        }
    }
    drop(present_modes);

    match present_mode {
        vk::PresentModeKHR::FIFO => engine.sys_printf("Using FIFO present mode\n"),
        vk::PresentModeKHR::MAILBOX => engine.sys_printf("Using MAILBOX present mode\n"),
        vk::PresentModeKHR::IMMEDIATE => engine.sys_printf("Using IMMEDIATE present mode\n"),
        _ => {}
    }

    let wanted_images: u32 = if engine.vid_vsync() >= 2.0 { 3 } else { 2 };
    let mut swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
        .surface(vid.surface)
        .min_image_count(wanted_images.max(surface_capabilities.min_image_count))
        .image_format(swap_chain_format)
        .image_color_space(swap_chain_color_space)
        .image_extent(vk::Extent2D {
            width: vid_width,
            height: vid_height,
        })
        .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC)
        .pre_transform(vk::SurfaceTransformFlagsKHR::IDENTITY)
        .image_array_layers(1)
        .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
        .present_mode(present_mode)
        .clipped(true)
        .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE);
    if !surface_capabilities
        .supported_composite_alpha
        .contains(vk::CompositeAlphaFlagsKHR::OPAQUE)
    {
        swapchain_create_info =
            swapchain_create_info.composite_alpha(vk::CompositeAlphaFlagsKHR::INHERIT);
    }
    if vid.swapchain_present_wait {
        swapchain_create_info.flags |=
            SWAPCHAIN_CREATE_PRESENT_ID_2_BIT_KHR | SWAPCHAIN_CREATE_PRESENT_WAIT_2_BIT_KHR;
    }

    ctx.vg.swap_chain_full_screen_exclusive = false;
    ctx.vg.swap_chain_full_screen_acquired = false;
    #[cfg(windows)]
    if use_exclusive_full_screen {
        swapchain_create_info.p_next =
            (&full_screen_exclusive_info as *const vk::SurfaceFullScreenExclusiveInfoEXT).cast();
        ctx.vg.swap_chain_full_screen_exclusive = true;
    }

    ctx.vg.swap_chain_format = swap_chain_format;
    drop(surface_formats);

    debug_assert!(vid.swapchain == vk::SwapchainKHR::null());
    let create_swapchain = vid
        .procs
        .create_swapchain
        .expect("GL_InitDevice loads vkCreateSwapchainKHR");
    // SAFETY: `swapchain_create_info` and its (optional) exclusive-mode chain
    // are locals that outlive the call.
    let mut err = unsafe {
        create_swapchain(
            ctx.vg.device,
            &swapchain_create_info,
            core::ptr::null(),
            &mut vid.swapchain,
        )
    };
    if err != vk::Result::SUCCESS {
        #[cfg(windows)]
        if use_exclusive_full_screen {
            swapchain_create_info.p_next = core::ptr::null();
            ctx.vg.swap_chain_full_screen_exclusive = false;
            // SAFETY: as above, without the exclusive-mode chain.
            err = unsafe {
                create_swapchain(
                    ctx.vg.device,
                    &swapchain_create_info,
                    core::ptr::null(),
                    &mut vid.swapchain,
                )
            };
        }
        if err != vk::Result::SUCCESS {
            fail(engine, "Couldn't create swap chain", err);
        }
    }

    vid.num_images_acquired = 0;
    vid.current_present_id = 0; // present ids are scoped to the swapchain

    for i in 0..vid.num_swap_chain_images {
        debug_assert!(vid.swapchain_images[i] == vk::Image::null());
    }

    let get_swapchain_images = vid
        .procs
        .get_swapchain_images
        .expect("GL_InitDevice loads vkGetSwapchainImagesKHR");
    let mut num_swap_chain_images = vid.num_swap_chain_images as u32;
    // SAFETY: count query with a null array.
    let err = unsafe {
        get_swapchain_images(
            ctx.vg.device,
            vid.swapchain,
            &mut num_swap_chain_images,
            core::ptr::null_mut(),
        )
    };
    if err != vk::Result::SUCCESS || num_swap_chain_images as usize > MAX_SWAP_CHAIN_IMAGES {
        fail(engine, "Couldn't get swap chain images", err);
    }
    // SAFETY: `swapchain_images` holds `MAX_SWAP_CHAIN_IMAGES` entries and the
    // count was just bounded by it; C ignores this result.
    let _ = unsafe {
        get_swapchain_images(
            ctx.vg.device,
            vid.swapchain,
            &mut num_swap_chain_images,
            vid.swapchain_images.as_mut_ptr(),
        )
    };
    vid.num_swap_chain_images = num_swap_chain_images as usize;

    let mut image_view_create_info = vk::ImageViewCreateInfo::default()
        .format(ctx.vg.swap_chain_format)
        .components(vk::ComponentMapping {
            r: vk::ComponentSwizzle::R,
            g: vk::ComponentSwizzle::G,
            b: vk::ComponentSwizzle::B,
            a: vk::ComponentSwizzle::A,
        })
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        })
        .view_type(vk::ImageViewType::TYPE_2D);

    let semaphore_create_info = vk::SemaphoreCreateInfo::default();

    for i in 0..vid.num_swap_chain_images {
        ctx.name_object(vid.swapchain_images[i], c"Swap Chain");
        debug_assert!(vid.swapchain_images_views[i] == vk::ImageView::null());
        image_view_create_info.image = vid.swapchain_images[i];
        // SAFETY: `image_view_create_info` is complete over a swap chain image.
        vid.swapchain_images_views[i] =
            match unsafe { ctx.device.create_image_view(&image_view_create_info, None) } {
                Ok(view) => view,
                Err(err) => ctx.vk_fail("vkCreateImageView", err),
            };
        ctx.name_object(vid.swapchain_images_views[i], c"Swap Chain View");
    }

    for i in 0..DOUBLE_BUFFERED {
        debug_assert!(vid.image_aquired_semaphores[i] == vk::Semaphore::null());
        // SAFETY: a default semaphore create info.
        vid.image_aquired_semaphores[i] =
            match unsafe { ctx.device.create_semaphore(&semaphore_create_info, None) } {
                Ok(semaphore) => semaphore,
                Err(err) => ctx.vk_fail("vkCreateSemaphore", err),
            };
    }

    for i in 0..vid.num_swap_chain_images {
        debug_assert!(vid.draw_complete_semaphores[i] == vk::Semaphore::null());
        // SAFETY: as above.
        vid.draw_complete_semaphores[i] =
            match unsafe { ctx.device.create_semaphore(&semaphore_create_info, None) } {
                Ok(semaphore) => semaphore,
                Err(err) => ctx.vk_fail("vkCreateSemaphore", err),
            };
    }

    true
}
