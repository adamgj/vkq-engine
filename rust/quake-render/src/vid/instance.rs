//! `GL_InitInstance`, `GL_InitDevice` and `GL_InitCommandBuffers`
//! (`gl_vidsdl.c`, Phase 8 M6).
//!
//! Extension entry points and `vkGetPhysicalDeviceProperties2`/`Features2`
//! are loaded by hand through `vkGetInstanceProcAddr`/`vkGetDeviceProcAddr`
//! like the C `GET_*_PROC_ADDR` macros (fatal when missing), so the two
//! builds resolve the same functions; core commands go through ash's tables.

use core::ffi::{c_char, CStr};
use core::ptr;

use ash::vk;
use quake_types::render::{CbContext, VulkanGlobals, PCBX_NUM};

use super::{
    device_proc, fail, instance_proc, PhysicalDevicePresentId2FeaturesKHR,
    PhysicalDevicePresentWait2FeaturesKHR, Procs, VidEngine, VidState, DOUBLE_BUFFERED,
    KHR_PRESENT_ID_2_EXTENSION_NAME, KHR_PRESENT_WAIT_2_EXTENSION_NAME,
    REQUIRED_COLOR_BUFFER_FEATURES, SECONDARY_CB_MULTIPLICITY,
    STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_ID_2_FEATURES_KHR,
    STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_WAIT_2_FEATURES_KHR,
};
use crate::rmisc::Ctx;

fn lossy(s: Result<&CStr, core::ffi::FromBytesUntilNulError>) -> String {
    s.map(|c| c.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn extension_present(props: &[vk::ExtensionProperties], name: &CStr) -> bool {
    props
        .iter()
        .any(|p| p.extension_name_as_c_str().is_ok_and(|n| n == name))
}

/// `GL_InitInstance`.
pub fn init_instance<E: VidEngine>(engine: &E, vg: &mut VulkanGlobals, vid: &mut VidState) {
    vg.debug_utils = false;

    let sdl_extensions = engine.instance_extensions();
    let mut instance_extensions: Vec<*const c_char> =
        sdl_extensions.iter().map(|s| s.as_ptr()).collect();

    let get_instance_proc_addr = engine.get_instance_proc_addr();
    // SAFETY: `get_instance_proc_addr` is the loader entry point SDL
    // resolved for this process and stays valid for its lifetime.
    let entry = unsafe {
        ash::Entry::from_static_fn(ash::StaticFn {
            get_instance_proc_addr,
        })
    };

    vg.get_surface_capabilities_2 = false;
    vg.get_physical_device_properties_2 = false;
    // SAFETY: no Vulkan state is required for the global command.
    let extension_props =
        unsafe { entry.enumerate_instance_extension_properties(None) }.unwrap_or_default();
    if extension_present(&extension_props, ash::khr::get_surface_capabilities2::NAME) {
        vg.get_surface_capabilities_2 = true;
    }
    if extension_present(
        &extension_props,
        ash::khr::get_physical_device_properties2::NAME,
    ) {
        vg.get_physical_device_properties_2 = true;
    }
    if cfg!(feature = "engine-debug")
        && extension_present(&extension_props, ash::ext::debug_utils::NAME)
    {
        vg.debug_utils = true;
    }
    drop(extension_props);

    vg.vulkan_1_1_available = false;
    vid.procs = Procs::default();
    vid.procs.get_instance_proc_addr = Some(get_instance_proc_addr);
    let enumerate_instance_version: vk::PFN_vkEnumerateInstanceVersion = instance_proc(
        engine,
        get_instance_proc_addr,
        vk::Instance::null(),
        c"vkEnumerateInstanceVersion",
    );
    {
        let mut api_version = 0u32;
        // SAFETY: the loader entry point writes one `u32`; the result is
        // deliberately unchecked like the C.
        let _ = unsafe { enumerate_instance_version(&mut api_version) };
        if api_version >= vk::make_api_version(0, 1, 1, 0) {
            engine.con_printf("Using Vulkan 1.1\n");
            vg.vulkan_1_1_available = true;
        }
    }

    let application_info = vk::ApplicationInfo::default()
        .application_name(c"vkqr-engine")
        .application_version(1)
        .engine_name(c"vkqr-engine")
        .engine_version(1)
        .api_version(if vg.vulkan_1_1_available {
            vk::make_api_version(0, 1, 1, 0)
        } else {
            vk::make_api_version(0, 1, 0, 0)
        });

    if vg.get_surface_capabilities_2 {
        instance_extensions.push(ash::khr::get_surface_capabilities2::NAME.as_ptr());
    }
    if vg.get_physical_device_properties_2 {
        instance_extensions.push(ash::khr::get_physical_device_properties2::NAME.as_ptr());
    }
    #[cfg_attr(not(feature = "engine-debug"), allow(unused_mut))]
    let mut layer_names: Vec<*const c_char> = Vec::new();
    #[cfg(feature = "engine-debug")]
    {
        if vg.debug_utils {
            instance_extensions.push(ash::ext::debug_utils::NAME.as_ptr());
        }
        if vg.validation {
            engine.con_printf("Using VK_LAYER_KHRONOS_validation\n");
            layer_names.push(c"VK_LAYER_KHRONOS_validation".as_ptr());
        }
    }

    let instance_create_info = vk::InstanceCreateInfo::default()
        .application_info(&application_info)
        .enabled_extension_names(&instance_extensions)
        .enabled_layer_names(&layer_names);
    // SAFETY: `instance_create_info` borrows live, NUL-terminated names.
    let instance = match unsafe { entry.create_instance(&instance_create_info, None) } {
        Ok(instance) => instance,
        Err(err) => fail(engine, "Couldn't create Vulkan instance", err),
    };
    let handle = instance.handle();

    vid.surface = engine.create_surface(handle);

    let procs = &mut vid.procs;
    procs.get_device_proc_addr = Some(instance_proc(
        engine,
        get_instance_proc_addr,
        handle,
        c"vkGetDeviceProcAddr",
    ));
    procs.get_physical_device_surface_support = Some(instance_proc(
        engine,
        get_instance_proc_addr,
        handle,
        c"vkGetPhysicalDeviceSurfaceSupportKHR",
    ));
    procs.get_physical_device_surface_capabilities = Some(instance_proc(
        engine,
        get_instance_proc_addr,
        handle,
        c"vkGetPhysicalDeviceSurfaceCapabilitiesKHR",
    ));
    procs.get_physical_device_surface_formats = Some(instance_proc(
        engine,
        get_instance_proc_addr,
        handle,
        c"vkGetPhysicalDeviceSurfaceFormatsKHR",
    ));
    procs.get_physical_device_surface_present_modes = Some(instance_proc(
        engine,
        get_instance_proc_addr,
        handle,
        c"vkGetPhysicalDeviceSurfacePresentModesKHR",
    ));
    procs.get_swapchain_images = Some(instance_proc(
        engine,
        get_instance_proc_addr,
        handle,
        c"vkGetSwapchainImagesKHR",
    ));
    if vg.get_physical_device_properties_2 {
        procs.get_physical_device_properties2 = Some(instance_proc(
            engine,
            get_instance_proc_addr,
            handle,
            c"vkGetPhysicalDeviceProperties2",
        ));
        procs.get_physical_device_features2 = Some(instance_proc(
            engine,
            get_instance_proc_addr,
            handle,
            c"vkGetPhysicalDeviceFeatures2",
        ));
    }
    if vg.get_surface_capabilities_2 {
        procs.get_physical_device_surface_capabilities2 = Some(instance_proc(
            engine,
            get_instance_proc_addr,
            handle,
            c"vkGetPhysicalDeviceSurfaceCapabilities2KHR",
        ));
    }

    engine.con_printf("Instance extensions:\n");
    for &name in &instance_extensions {
        // SAFETY: every entry is a NUL-terminated string live for this
        // function (SDL's copies or the `NAME` constants).
        let name = unsafe { CStr::from_ptr(name) };
        engine.con_printf(&format!(" {}\n", name.to_string_lossy()));
    }
    engine.con_printf("\n");

    #[cfg(feature = "engine-debug")]
    if vg.validation {
        engine.con_printf("Creating debug report callback\n");
        let create: vk::PFN_vkCreateDebugUtilsMessengerEXT = instance_proc(
            engine,
            get_instance_proc_addr,
            handle,
            c"vkCreateDebugUtilsMessengerEXT",
        );
        let info = vk::DebugUtilsMessengerCreateInfoEXT::default()
            .message_severity(
                vk::DebugUtilsMessageSeverityFlagsEXT::ERROR
                    | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING,
            )
            .message_type(
                vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                    | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION,
            )
            .pfn_user_callback(engine.debug_message_callback());
        // SAFETY: `info` is complete; `create` was loaded from this instance.
        let err = unsafe { create(handle, &info, ptr::null(), &mut vid.debug_utils_messenger) };
        if err != vk::Result::SUCCESS {
            fail(engine, "Could not create debug report callback", err);
        }
    }

    vid.entry = Some(entry);
    vid.instance = Some(instance);
}

/// `GetDeviceVendorFromDriverProperties` over `VkDriverId` values.
fn vendor_from_driver_id(driver_id: vk::DriverId) -> Option<&'static str> {
    match driver_id.as_raw() {
        1..=3 => Some("AMD"),
        4 => Some("NVIDIA"),
        5 | 6 => Some("Intel"),
        7 => Some("ImgTec"),
        8 | 18 => Some("Qualcomm"),
        9 | 20 => Some("ARM"),
        10 | 11 => Some("Google"),
        12 => Some("Broadcom"),
        19 => Some("Raspberry Pi"),
        13 | 22 => Some("MESA"),
        14 => Some("MoltenVK"),
        21 => Some("Samsung"),
        _ => None,
    }
}

/// `GetDeviceVendorFromDeviceProperties`.
fn vendor_from_vendor_id(vendor_id: u32) -> Option<&'static str> {
    match vendor_id {
        0x8086 => Some("Intel"),
        0x10DE => Some("NVIDIA"),
        0x1002 => Some("AMD"),
        0x1010 => Some("ImgTec"),
        0x13B5 => Some("ARM"),
        0x5143 => Some("Qualcomm"),
        _ => None,
    }
}

/// `GL_InitDevice`. Returns the ash device for the caller to keep; the raw
/// handle is stored in `vg.device`.
pub fn init_device<E: VidEngine>(
    engine: &E,
    vg: &mut VulkanGlobals,
    vid: &mut VidState,
) -> ash::Device {
    let instance = vid
        .instance
        .as_ref()
        .expect("GL_InitInstance runs before GL_InitDevice");
    let get_device_proc_addr = vid
        .procs
        .get_device_proc_addr
        .expect("GL_InitInstance loads vkGetDeviceProcAddr");
    let get_instance_proc_addr = vid
        .procs
        .get_instance_proc_addr
        .expect("GL_InitInstance stores vkGetInstanceProcAddr");

    // SAFETY: the instance is live.
    let physical_devices = match unsafe { instance.enumerate_physical_devices() } {
        Ok(devices) if !devices.is_empty() => devices,
        Ok(_) => fail(
            engine,
            "Couldn't find any Vulkan devices",
            vk::Result::SUCCESS,
        ),
        Err(err) => fail(engine, "Couldn't find any Vulkan devices", err),
    };
    let physical_device_count = physical_devices.len() as i32;

    let mut device_index = 0i32;
    let device_parm = engine.device_parm();
    if let Some(Some(device_num)) = device_parm {
        device_index = (device_num - 1).clamp(0, physical_device_count - 1);
    }
    if device_parm.is_none() {
        for (i, &pd) in physical_devices.iter().enumerate() {
            // SAFETY: `pd` came from `enumerate_physical_devices`.
            let props = unsafe { instance.get_physical_device_properties(pd) };
            if props.device_type == vk::PhysicalDeviceType::DISCRETE_GPU {
                device_index = i as i32;
                break;
            }
        }
    }
    let physical_device = physical_devices[device_index as usize];
    vid.physical_device = physical_device;

    let mut found_swapchain_extension = false;
    vg.dedicated_allocation = false;
    vg.full_screen_exclusive = false;
    vg.swap_chain_full_screen_acquired = false;
    vg.screen_effects_sops = false;
    vg.ray_query = false;
    let mut push_descriptor = false;
    let mut subgroup_size_control = false;

    // SAFETY: `physical_device` is one the instance enumerated.
    unsafe {
        vg.memory_properties = instance.get_physical_device_memory_properties(physical_device);
        vg.device_properties = instance.get_physical_device_properties(physical_device);
    }

    let mut driver_properties_available = false;
    let mut present_id = false;
    let mut present_wait = false;

    // SAFETY: as above.
    let device_extensions =
        unsafe { instance.enumerate_device_extension_properties(physical_device) }
            .unwrap_or_default();
    for ext in &device_extensions {
        let Ok(name) = ext.extension_name_as_c_str() else {
            continue;
        };
        if name == ash::khr::swapchain::NAME {
            found_swapchain_extension = true;
        }
        if name == ash::khr::dedicated_allocation::NAME {
            vg.dedicated_allocation = true;
        }
        if vg.get_physical_device_properties_2 && name == ash::khr::driver_properties::NAME {
            driver_properties_available = true;
        }
        if name == ash::ext::subgroup_size_control::NAME {
            subgroup_size_control = true;
        }
        #[cfg(windows)]
        if name == ash::ext::full_screen_exclusive::NAME {
            vg.full_screen_exclusive = true;
        }
        if name == ash::khr::push_descriptor::NAME {
            push_descriptor = true;
        }
        if name == ash::khr::ray_query::NAME {
            vg.ray_query = true;
        }
        if name == KHR_PRESENT_ID_2_EXTENSION_NAME {
            present_id = true;
        }
        if name == KHR_PRESENT_WAIT_2_EXTENSION_NAME {
            present_wait = true;
        }
    }
    drop(device_extensions);

    let mut vendor = None;
    let mut driver_properties = vk::PhysicalDeviceDriverProperties::default();
    if driver_properties_available {
        let get_properties2 = vid
            .procs
            .get_physical_device_properties2
            .expect("loaded with VK_KHR_get_physical_device_properties2");
        let mut properties2 =
            vk::PhysicalDeviceProperties2::default().push_next(&mut driver_properties);
        // SAFETY: `properties2` chains a live local; the entry point was
        // loaded from this instance.
        unsafe { get_properties2(physical_device, &mut properties2) };
        vendor = vendor_from_driver_id(driver_properties.driver_id);
    }
    if vendor.is_none() {
        vendor = vendor_from_vendor_id(vg.device_properties.vendor_id);
    }
    match vendor {
        Some(vendor) => engine.con_printf(&format!("Vendor: {vendor}\n")),
        None => engine.con_printf(&format!(
            "Vendor: Unknown (0x{:x})\n",
            vg.device_properties.vendor_id
        )),
    }
    engine.con_printf(&format!(
        "Device: {}\n",
        lossy(vg.device_properties.device_name_as_c_str())
    ));
    if driver_properties_available {
        engine.con_printf(&format!(
            "Driver: {} {}\n",
            lossy(driver_properties.driver_name_as_c_str()),
            lossy(driver_properties.driver_info_as_c_str())
        ));
    }

    if !found_swapchain_extension {
        engine.sys_error(&format!(
            "Couldn't find {} extension",
            ash::khr::swapchain::NAME.to_string_lossy()
        ));
    }

    // SAFETY: as above.
    let queue_family_properties =
        unsafe { instance.get_physical_device_queue_family_properties(physical_device) };
    if queue_family_properties.is_empty() {
        engine.sys_error("Couldn't find any Vulkan queues");
    }
    let get_surface_support = vid
        .procs
        .get_physical_device_surface_support
        .expect("GL_InitInstance loads vkGetPhysicalDeviceSurfaceSupportKHR");
    let queue_supports_present: Vec<vk::Bool32> = (0..queue_family_properties.len() as u32)
        .map(|i| {
            let mut supported = vk::FALSE;
            // SAFETY: the surface belongs to this instance; the result is
            // unchecked like the C.
            let _ = unsafe { get_surface_support(physical_device, i, vid.surface, &mut supported) };
            supported
        })
        .collect();
    let mut found_graphics_queue = false;
    for (i, props) in queue_family_properties.iter().enumerate() {
        if props.queue_flags.contains(vk::QueueFlags::GRAPHICS)
            && queue_supports_present[i] != vk::FALSE
        {
            found_graphics_queue = true;
            vg.gfx_queue_family_index = i as u32;
            break;
        }
    }
    drop(queue_supports_present);
    drop(queue_family_properties);
    if !found_graphics_queue {
        engine.sys_error("Couldn't find graphics queue");
    }

    let queue_priorities = [0.0f32];
    let queue_create_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(vg.gfx_queue_family_index)
        .queue_priorities(&queue_priorities);

    let mut physical_device_subgroup_properties = vk::PhysicalDeviceSubgroupProperties::default();
    let mut physical_device_subgroup_size_control_properties =
        vk::PhysicalDeviceSubgroupSizeControlProperties::default();
    let mut subgroup_size_control_features =
        vk::PhysicalDeviceSubgroupSizeControlFeatures::default();
    let mut buffer_device_address_features =
        vk::PhysicalDeviceBufferDeviceAddressFeatures::default();
    let mut acceleration_structure_features =
        vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default();
    let mut ray_query_features = vk::PhysicalDeviceRayQueryFeaturesKHR::default();
    let mut present_id_features = PhysicalDevicePresentId2FeaturesKHR {
        s_type: STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_ID_2_FEATURES_KHR,
        p_next: ptr::null_mut(),
        present_id2: vk::FALSE,
    };
    let mut present_wait_features = PhysicalDevicePresentWait2FeaturesKHR {
        s_type: STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_WAIT_2_FEATURES_KHR,
        p_next: ptr::null_mut(),
        present_wait2: vk::FALSE,
    };
    vg.physical_device_acceleration_structure_properties =
        vk::PhysicalDeviceAccelerationStructurePropertiesKHR::default();

    if vg.vulkan_1_1_available {
        let get_properties2 = vid
            .procs
            .get_physical_device_properties2
            .unwrap_or_else(|| engine.sys_error("vkGetPhysicalDeviceProperties2 is not loaded"));
        let get_features2 = vid
            .procs
            .get_physical_device_features2
            .unwrap_or_else(|| engine.sys_error("vkGetPhysicalDeviceFeatures2 is not loaded"));

        let mut properties2 = vk::PhysicalDeviceProperties2::default();
        if subgroup_size_control {
            properties2 = properties2
                .push_next(&mut physical_device_subgroup_size_control_properties)
                .push_next(&mut physical_device_subgroup_properties);
        }
        if vg.ray_query {
            properties2 =
                properties2.push_next(&mut vg.physical_device_acceleration_structure_properties);
        }
        // SAFETY: the chain only holds live locals and the `vg` field; the
        // entry point was loaded from this instance.
        unsafe { get_properties2(physical_device, &mut properties2) };
        vg.physical_device_acceleration_structure_properties.p_next = ptr::null_mut();

        let mut features2 = vk::PhysicalDeviceFeatures2::default();
        if subgroup_size_control {
            features2 = features2.push_next(&mut subgroup_size_control_features);
        }
        if vg.ray_query {
            features2 = features2
                .push_next(&mut buffer_device_address_features)
                .push_next(&mut acceleration_structure_features)
                .push_next(&mut ray_query_features);
        }
        if present_id && present_wait {
            features2 = features2
                .push_next(&mut present_id_features)
                .push_next(&mut present_wait_features);
        }
        // SAFETY: as above.
        unsafe { get_features2(physical_device, &mut features2) };
        vg.device_features = features2.features;
    } else {
        // SAFETY: as above.
        vg.device_features = unsafe { instance.get_physical_device_features(physical_device) };
    }
    // The query chained these; `vkCreateDevice` gets a fresh chain below.
    subgroup_size_control_features.p_next = ptr::null_mut();
    buffer_device_address_features.p_next = ptr::null_mut();
    acceleration_structure_features.p_next = ptr::null_mut();
    ray_query_features.p_next = ptr::null_mut();
    present_id_features.p_next = ptr::null_mut();
    present_wait_features.p_next = ptr::null_mut();

    // MoltenVK lies about this
    if cfg!(target_vendor = "apple") {
        vg.device_features.sample_rate_shading = vk::FALSE;
    }

    vg.screen_effects_sops = vg.vulkan_1_1_available
        && subgroup_size_control
        && subgroup_size_control_features.subgroup_size_control != vk::FALSE
        && subgroup_size_control_features.compute_full_subgroups != vk::FALSE
        && physical_device_subgroup_properties
            .supported_stages
            .contains(vk::ShaderStageFlags::COMPUTE)
        && physical_device_subgroup_properties
            .supported_operations
            .contains(vk::SubgroupFeatureFlags::SHUFFLE)
        && physical_device_subgroup_size_control_properties.min_subgroup_size >= 4
        && physical_device_subgroup_size_control_properties.max_subgroup_size <= 64;
    if vg.screen_effects_sops {
        engine.con_printf("Using subgroup operations\n");
    }

    vg.ray_query = vg.ray_query
        && push_descriptor
        && acceleration_structure_features.acceleration_structure != vk::FALSE
        && ray_query_features.ray_query != vk::FALSE
        && buffer_device_address_features.buffer_device_address != vk::FALSE;
    if vg.ray_query {
        engine.con_printf("Using ray queries\n");
    }

    vg.present_wait = vg.vulkan_1_1_available
        && vg.get_surface_capabilities_2
        && present_id
        && present_wait
        && present_id_features.present_id2 != vk::FALSE
        && present_wait_features.present_wait2 != vk::FALSE;
    if vg.present_wait {
        engine.con_printf("Using present wait\n");
    }

    let mut enabled_extensions: Vec<&CStr> = vec![ash::khr::swapchain::NAME];
    if vg.dedicated_allocation {
        enabled_extensions.push(ash::khr::get_memory_requirements2::NAME);
        enabled_extensions.push(ash::khr::dedicated_allocation::NAME);
    }
    if vg.screen_effects_sops {
        enabled_extensions.push(ash::ext::subgroup_size_control::NAME);
    }
    #[cfg(windows)]
    if vg.full_screen_exclusive {
        enabled_extensions.push(ash::ext::full_screen_exclusive::NAME);
    }
    if vg.present_wait {
        enabled_extensions.push(KHR_PRESENT_ID_2_EXTENSION_NAME);
        enabled_extensions.push(KHR_PRESENT_WAIT_2_EXTENSION_NAME);
    }
    if vg.ray_query {
        // COMPAT: the C list names VK_KHR_acceleration_structure twice.
        enabled_extensions.push(ash::khr::acceleration_structure::NAME);
        enabled_extensions.push(ash::khr::push_descriptor::NAME);
        enabled_extensions.push(ash::ext::descriptor_indexing::NAME);
        enabled_extensions.push(ash::khr::buffer_device_address::NAME);
        enabled_extensions.push(ash::khr::deferred_host_operations::NAME);
        enabled_extensions.push(ash::khr::shader_float_controls::NAME);
        enabled_extensions.push(ash::khr::spirv_1_4::NAME);
        enabled_extensions.push(ash::khr::acceleration_structure::NAME);
        enabled_extensions.push(ash::khr::ray_query::NAME);
    }
    let enabled_extension_ptrs: Vec<*const c_char> =
        enabled_extensions.iter().map(|s| s.as_ptr()).collect();

    let extended_format_support = vg.device_features.shader_storage_image_extended_formats;
    let mut device_features = vk::PhysicalDeviceFeatures::default();
    device_features.shader_storage_image_extended_formats = extended_format_support;
    device_features.independent_blend = vg.device_features.independent_blend;
    device_features.sampler_anisotropy = vg.device_features.sampler_anisotropy;
    device_features.sample_rate_shading = vg.device_features.sample_rate_shading;
    device_features.fill_mode_non_solid = vg.device_features.fill_mode_non_solid;
    device_features.multi_draw_indirect = vg.device_features.multi_draw_indirect;
    vg.non_solid_fill = device_features.fill_mode_non_solid == vk::TRUE;
    vg.multi_draw_indirect = device_features.multi_draw_indirect == vk::TRUE;

    let queue_create_infos = [queue_create_info];
    let mut device_create_info = vk::DeviceCreateInfo::default()
        .queue_create_infos(&queue_create_infos)
        .enabled_extension_names(&enabled_extension_ptrs)
        .enabled_features(&device_features);
    if vg.screen_effects_sops {
        device_create_info = device_create_info.push_next(&mut subgroup_size_control_features);
    }
    if vg.ray_query {
        device_create_info = device_create_info
            .push_next(&mut buffer_device_address_features)
            .push_next(&mut acceleration_structure_features)
            .push_next(&mut ray_query_features);
    }
    if vg.present_wait {
        device_create_info = device_create_info
            .push_next(&mut present_id_features)
            .push_next(&mut present_wait_features);
    }
    // SAFETY: every pointer in `device_create_info` is to a live local.
    let device = match unsafe { instance.create_device(physical_device, &device_create_info, None) }
    {
        Ok(device) => device,
        Err(err) => fail(engine, "Couldn't create Vulkan device", err),
    };
    let dev = device.handle();
    vg.device = dev;

    let procs = &mut vid.procs;
    procs.create_swapchain = Some(device_proc(
        engine,
        get_device_proc_addr,
        dev,
        c"vkCreateSwapchainKHR",
    ));
    procs.destroy_swapchain = Some(device_proc(
        engine,
        get_device_proc_addr,
        dev,
        c"vkDestroySwapchainKHR",
    ));
    procs.get_swapchain_images = Some(device_proc(
        engine,
        get_device_proc_addr,
        dev,
        c"vkGetSwapchainImagesKHR",
    ));
    procs.acquire_next_image = Some(device_proc(
        engine,
        get_device_proc_addr,
        dev,
        c"vkAcquireNextImageKHR",
    ));
    procs.queue_present = Some(device_proc(
        engine,
        get_device_proc_addr,
        dev,
        c"vkQueuePresentKHR",
    ));

    engine.con_printf("Device extensions:\n");
    for name in &enabled_extensions {
        engine.con_printf(&format!(" {}\n", name.to_string_lossy()));
    }

    #[cfg(windows)]
    if vg.full_screen_exclusive {
        procs.acquire_full_screen_exclusive_mode = Some(device_proc(
            engine,
            get_device_proc_addr,
            dev,
            c"vkAcquireFullScreenExclusiveModeEXT",
        ));
        procs.release_full_screen_exclusive_mode = Some(device_proc(
            engine,
            get_device_proc_addr,
            dev,
            c"vkReleaseFullScreenExclusiveModeEXT",
        ));
    }
    if vg.present_wait {
        procs.wait_for_present2 = Some(device_proc(
            engine,
            get_device_proc_addr,
            dev,
            c"vkWaitForPresent2KHR",
        ));
    }
    if vg.ray_query {
        vg.vk_get_buffer_device_address = Some(device_proc(
            engine,
            get_device_proc_addr,
            dev,
            c"vkGetBufferDeviceAddressKHR",
        ));
        vg.vk_get_acceleration_structure_build_sizes = Some(device_proc(
            engine,
            get_device_proc_addr,
            dev,
            c"vkGetAccelerationStructureBuildSizesKHR",
        ));
        vg.vk_create_acceleration_structure = Some(device_proc(
            engine,
            get_device_proc_addr,
            dev,
            c"vkCreateAccelerationStructureKHR",
        ));
        vg.vk_destroy_acceleration_structure = Some(device_proc(
            engine,
            get_device_proc_addr,
            dev,
            c"vkDestroyAccelerationStructureKHR",
        ));
        vg.vk_cmd_build_acceleration_structures = Some(device_proc(
            engine,
            get_device_proc_addr,
            dev,
            c"vkCmdBuildAccelerationStructuresKHR",
        ));
        vg.vk_cmd_push_descriptor_set = Some(device_proc(
            engine,
            get_device_proc_addr,
            dev,
            c"vkCmdPushDescriptorSetKHR",
        ));
        vg.vk_get_acceleration_structure_device_address = Some(device_proc(
            engine,
            get_device_proc_addr,
            dev,
            c"vkGetAccelerationStructureDeviceAddressKHR",
        ));
    }
    #[cfg(feature = "engine-debug")]
    if vg.debug_utils {
        let handle = instance.handle();
        procs.set_debug_utils_object_name = Some(instance_proc(
            engine,
            get_instance_proc_addr,
            handle,
            c"vkSetDebugUtilsObjectNameEXT",
        ));
        vg.vk_cmd_begin_debug_utils_label = Some(instance_proc(
            engine,
            get_instance_proc_addr,
            handle,
            c"vkCmdBeginDebugUtilsLabelEXT",
        ));
        vg.vk_cmd_end_debug_utils_label = Some(instance_proc(
            engine,
            get_instance_proc_addr,
            handle,
            c"vkCmdEndDebugUtilsLabelEXT",
        ));
    }
    #[cfg(not(feature = "engine-debug"))]
    let _ = get_instance_proc_addr;

    // SAFETY: the queue family index was validated against the device.
    vg.queue = unsafe { device.get_device_queue(vg.gfx_queue_family_index, 0) };

    // SAFETY (all format queries below): `physical_device` is live.
    vg.color_format = vk::Format::R8G8B8A8_UNORM;
    if extended_format_support == vk::TRUE {
        // SAFETY: as above.
        let format_properties = unsafe {
            instance.get_physical_device_format_properties(
                physical_device,
                vk::Format::A2B10G10R10_UNORM_PACK32,
            )
        };
        let a2_b10_g10_r10_support = format_properties
            .optimal_tiling_features
            .contains(REQUIRED_COLOR_BUFFER_FEATURES);
        if a2_b10_g10_r10_support {
            engine.con_printf("Using A2B10G10R10 color buffer format\n");
            vg.color_format = vk::Format::A2B10G10R10_UNORM_PACK32;
        }
    }

    // SAFETY: as above.
    let format_properties = unsafe {
        instance
            .get_physical_device_format_properties(physical_device, vk::Format::D24_UNORM_S8_UINT)
    };
    let x8_d24_support = format_properties
        .optimal_tiling_features
        .contains(vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT);
    // SAFETY: as above.
    let format_properties = unsafe {
        instance
            .get_physical_device_format_properties(physical_device, vk::Format::D32_SFLOAT_S8_UINT)
    };
    let d32_support = format_properties
        .optimal_tiling_features
        .contains(vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT);

    vg.depth_format = vk::Format::UNDEFINED;
    if d32_support {
        engine.con_printf("Using D32_S8 depth buffer format\n");
        vg.depth_format = vk::Format::D32_SFLOAT_S8_UINT;
    } else if x8_d24_support {
        engine.con_printf("Using D24_S8 depth buffer format\n");
        vg.depth_format = vk::Format::D24_UNORM_S8_UINT;
    } else {
        engine.sys_error(
            "Cannot find VK_FORMAT_D24_UNORM_S8_UINT or VK_FORMAT_D32_SFLOAT_S8_UINT depth buffer format",
        );
    }

    engine.con_printf("\n");

    vg.vk_cmd_bind_pipeline = Some(device_proc(
        engine,
        get_device_proc_addr,
        dev,
        c"vkCmdBindPipeline",
    ));
    vg.vk_cmd_push_constants = Some(device_proc(
        engine,
        get_device_proc_addr,
        dev,
        c"vkCmdPushConstants",
    ));
    vg.vk_cmd_bind_descriptor_sets = Some(device_proc(
        engine,
        get_device_proc_addr,
        dev,
        c"vkCmdBindDescriptorSets",
    ));
    vg.vk_cmd_bind_index_buffer = Some(device_proc(
        engine,
        get_device_proc_addr,
        dev,
        c"vkCmdBindIndexBuffer",
    ));
    vg.vk_cmd_bind_vertex_buffers = Some(device_proc(
        engine,
        get_device_proc_addr,
        dev,
        c"vkCmdBindVertexBuffers",
    ));
    vg.vk_cmd_draw = Some(device_proc(engine, get_device_proc_addr, dev, c"vkCmdDraw"));
    vg.vk_cmd_draw_indexed = Some(device_proc(
        engine,
        get_device_proc_addr,
        dev,
        c"vkCmdDrawIndexed",
    ));
    vg.vk_cmd_draw_indexed_indirect = Some(device_proc(
        engine,
        get_device_proc_addr,
        dev,
        c"vkCmdDrawIndexedIndirect",
    ));
    vg.vk_cmd_pipeline_barrier = Some(device_proc(
        engine,
        get_device_proc_addr,
        dev,
        c"vkCmdPipelineBarrier",
    ));
    vg.vk_cmd_copy_buffer_to_image = Some(device_proc(
        engine,
        get_device_proc_addr,
        dev,
        c"vkCmdCopyBufferToImage",
    ));
    vg.vk_cmd_dispatch = Some(device_proc(
        engine,
        get_device_proc_addr,
        dev,
        c"vkCmdDispatch",
    ));

    if engine.renderhash() {
        engine.install_harness_hooks();
    }

    device
}

/// `Mem_Alloc (multiplicity * sizeof (cb_context_t))`: a zeroed, leaked
/// array the C side reads through `vulkan_globals.secondary_cb_contexts`.
fn alloc_cb_contexts(multiplicity: usize) -> *mut CbContext {
    let layout = std::alloc::Layout::array::<CbContext>(multiplicity)
        .expect("SECONDARY_CB_MULTIPLICITY entries are small");
    // SAFETY: `layout` is non-zero sized; `CbContext` is a `repr(C)` POD of
    // handles, integers and one pointer, for which all-zero bytes are valid.
    let contexts = unsafe { std::alloc::alloc_zeroed(layout) }.cast::<CbContext>();
    if contexts.is_null() {
        std::alloc::handle_alloc_error(layout);
    }
    contexts
}

/// `GL_InitCommandBuffers`.
pub fn init_command_buffers<E: VidEngine>(ctx: &mut Ctx<'_, E>, vid: &mut VidState) {
    ctx.engine.con_printf("Creating command buffers\n");

    let info = vk::CommandPoolCreateInfo::default()
        .flags(vk::CommandPoolCreateFlags::TRANSIENT)
        .queue_family_index(ctx.vg.gfx_queue_family_index);
    // SAFETY (all calls below): the device is live and the create infos
    // are complete locals.
    // SAFETY: as above.
    vid.transient_command_pool = match unsafe { ctx.device.create_command_pool(&info, None) } {
        Ok(pool) => pool,
        Err(err) => ctx.vk_fail("vkCreateCommandPool", err),
    };

    let info = vk::CommandPoolCreateInfo::default()
        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
        .queue_family_index(ctx.vg.gfx_queue_family_index);

    for pcbx_index in 0..PCBX_NUM {
        // SAFETY: as above.
        let pool = match unsafe { ctx.device.create_command_pool(&info, None) } {
            Ok(pool) => pool,
            Err(err) => ctx.vk_fail("vkCreateCommandPool", err),
        };
        vid.primary_command_pools[pcbx_index] = pool;
        let alloc = vk::CommandBufferAllocateInfo::default()
            .command_pool(pool)
            .command_buffer_count(DOUBLE_BUFFERED as u32);
        // SAFETY: as above.
        let buffers = match unsafe { ctx.device.allocate_command_buffers(&alloc) } {
            Ok(buffers) => buffers,
            Err(err) => ctx.vk_fail("vkAllocateCommandBuffers", err),
        };
        for (i, &cb) in buffers.iter().enumerate().take(DOUBLE_BUFFERED) {
            vid.primary_command_buffers[pcbx_index][i] = cb;
            let name =
                crate::rmisc::memory::c_string(&format!("PCBX index: {pcbx_index} cb_index: {i}"));
            ctx.name_object(cb, &name);
        }
    }

    for (scbx_index, multiplicity) in SECONDARY_CB_MULTIPLICITY.iter().copied().enumerate() {
        ctx.vg.secondary_cb_contexts[scbx_index] = alloc_cb_contexts(multiplicity);
        for i in 0..multiplicity {
            // SAFETY: as above.
            let pool = match unsafe { ctx.device.create_command_pool(&info, None) } {
                Ok(pool) => pool,
                Err(err) => ctx.vk_fail("vkCreateCommandPool", err),
            };
            vid.secondary_command_pools[scbx_index][i] = pool;
            let alloc = vk::CommandBufferAllocateInfo::default()
                .command_pool(pool)
                .command_buffer_count(DOUBLE_BUFFERED as u32)
                .level(vk::CommandBufferLevel::SECONDARY);
            // SAFETY: as above.
            let buffers = match unsafe { ctx.device.allocate_command_buffers(&alloc) } {
                Ok(buffers) => buffers,
                Err(err) => ctx.vk_fail("vkAllocateCommandBuffers", err),
            };
            for (j, &cb) in buffers.iter().enumerate().take(DOUBLE_BUFFERED) {
                vid.secondary_command_buffers[scbx_index][j][i] = cb;
                let name = crate::rmisc::memory::c_string(&format!(
                    "SCBX index: {scbx_index} sub_index: {i} cb_index: {j}"
                ));
                ctx.name_object(cb, &name);
            }
        }
    }

    let fence_info = vk::FenceCreateInfo::default();
    for fence in vid.command_buffer_fences.iter_mut() {
        // SAFETY: as above.
        *fence = match unsafe { ctx.device.create_fence(&fence_info, None) } {
            Ok(fence) => fence,
            Err(err) => ctx.vk_fail("vkCreateFence", err),
        };
    }

    let limits = &ctx.vg.device_properties.limits;
    if vid.timestamp_query_pool == vk::QueryPool::null()
        && limits.timestamp_compute_and_graphics != vk::FALSE
        && limits.timestamp_period > 0.0
    {
        let info = vk::QueryPoolCreateInfo::default()
            .query_type(vk::QueryType::TIMESTAMP)
            .query_count(2 * DOUBLE_BUFFERED as u32);
        // SAFETY: as above.
        vid.timestamp_query_pool = match unsafe { ctx.device.create_query_pool(&info, None) } {
            Ok(pool) => pool,
            Err(err) => ctx.vk_fail("vkCreateQueryPool", err),
        };
    }
}
