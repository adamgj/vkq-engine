//! `GL_MemoryTypeFromProperties`, `R_AllocateVulkanMemory`,
//! `R_FreeVulkanMemory`, `R_CreateBuffer(s)`, `R_FreeBuffer(s)`.

use core::ffi::{c_void, CStr};
use core::sync::atomic::{AtomicU32, Ordering::SeqCst};
use std::ffi::CString;

use ash::vk;
use quake_types::render::{VulkanMemory, VulkanMemoryType};

use super::{q_align, Ctx, Engine};

/// `GL_MemoryTypeFromProperties` without the `Sys_Error`: the first memory
/// type in `type_bits` carrying both masks, else the first carrying the
/// required mask.
pub fn memory_type_from_properties(
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    requirements_mask: vk::MemoryPropertyFlags,
    preferred_mask: vk::MemoryPropertyFlags,
) -> Option<u32> {
    let both = requirements_mask | preferred_mask;
    for mask in [both, requirements_mask] {
        let mut current_type_bits = type_bits;
        for i in 0..vk::MAX_MEMORY_TYPES as u32 {
            if (current_type_bits & 1) == 1
                && memory_properties.memory_types[i as usize].property_flags & mask == mask
            {
                return Some(i);
            }
            current_type_bits >>= 1;
        }
    }
    None
}

impl<E: Engine> Ctx<'_, E> {
    /// `GL_MemoryTypeFromProperties`.
    pub fn memory_type_from_properties(
        &self,
        type_bits: u32,
        requirements_mask: vk::MemoryPropertyFlags,
        preferred_mask: vk::MemoryPropertyFlags,
    ) -> u32 {
        match memory_type_from_properties(
            &self.vg.memory_properties,
            type_bits,
            requirements_mask,
            preferred_mask,
        ) {
            Some(i) => i,
            None => self.engine.sys_error("Could not find memory type"),
        }
    }
}

/// `R_AllocateVulkanMemory`.
pub fn allocate_vulkan_memory<E: Engine>(
    ctx: &Ctx<'_, E>,
    memory: &mut VulkanMemory,
    memory_allocate_info: &vk::MemoryAllocateInfo<'_>,
    memory_type: VulkanMemoryType,
    num_allocations: Option<&AtomicU32>,
) {
    memory.type_ = memory_type;
    if memory_type != VulkanMemoryType::None {
        // SAFETY: `memory_allocate_info` is a complete structure whose pNext
        // chain (if any) the caller built from live locals.
        memory.handle = match unsafe { ctx.device.allocate_memory(memory_allocate_info, None) } {
            Ok(handle) => handle,
            Err(err) => ctx.vk_fail("vkAllocateMemory", err),
        };
        if let Some(counter) = num_allocations {
            counter.fetch_add(1, SeqCst);
        }
    }
    memory.size = memory_allocate_info.allocation_size as usize;
    match memory_type {
        VulkanMemoryType::Device => {
            ctx.counters
                .total_device
                .fetch_add(memory_allocate_info.allocation_size, SeqCst);
        }
        VulkanMemoryType::Host => {
            ctx.counters
                .total_host
                .fetch_add(memory_allocate_info.allocation_size, SeqCst);
        }
        VulkanMemoryType::None => {}
    }
}

/// `R_FreeVulkanMemory`.
pub fn free_vulkan_memory<E: Engine>(
    ctx: &Ctx<'_, E>,
    memory: &mut VulkanMemory,
    num_allocations: Option<&AtomicU32>,
) {
    match memory.type_ {
        VulkanMemoryType::Device => {
            ctx.counters
                .total_device
                .fetch_sub(memory.size as u64, SeqCst);
        }
        VulkanMemoryType::Host => {
            ctx.counters
                .total_host
                .fetch_sub(memory.size as u64, SeqCst);
        }
        VulkanMemoryType::None => {}
    }
    if memory.type_ != VulkanMemoryType::None {
        // SAFETY: the handle came from `allocate_vulkan_memory` on this
        // device and every buffer bound to it has been destroyed by the caller.
        unsafe { ctx.device.free_memory(memory.handle, None) };
        if let Some(counter) = num_allocations {
            counter.fetch_sub(1, SeqCst);
        }
    }
    memory.handle = vk::DeviceMemory::null();
    memory.size = 0;
    memory.type_ = VulkanMemoryType::None;
}

fn create_named_buffer<E: Engine>(
    ctx: &Ctx<'_, E>,
    size: u64,
    usage: vk::BufferUsageFlags,
    name: &str,
) -> vk::Buffer {
    let info = vk::BufferCreateInfo::default().size(size).usage(usage);
    // SAFETY: `info` is complete and `ctx.device` is the live device.
    let buffer = match unsafe { ctx.device.create_buffer(&info, None) } {
        Ok(buffer) => buffer,
        Err(err) => ctx.vk_fail("vkCreateBuffer", err),
    };
    ctx.name_object(buffer, &c_string(name));
    buffer
}

/// `R_CreateBuffer`: one buffer with its own device-local allocation. Returns
/// the buffer and, when `device_address` was asked for and the device has
/// `vkGetBufferDeviceAddress`, its address.
#[allow(clippy::too_many_arguments)]
pub fn create_buffer<E: Engine>(
    ctx: &Ctx<'_, E>,
    memory: &mut VulkanMemory,
    size: u64,
    mut usage: vk::BufferUsageFlags,
    mem_requirements_mask: vk::MemoryPropertyFlags,
    mem_preferred_mask: vk::MemoryPropertyFlags,
    num_allocations: Option<&AtomicU32>,
    device_address: bool,
    name: &str,
) -> (vk::Buffer, Option<vk::DeviceAddress>) {
    let get_device_address = ctx.vg.vk_get_buffer_device_address.is_some() && device_address;
    if get_device_address {
        usage |= vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS_KHR;
    }
    let buffer = create_named_buffer(ctx, size, usage, &format!("{name} buffer"));

    // SAFETY: `buffer` was just created on this device.
    let requirements = unsafe { ctx.device.get_buffer_memory_requirements(buffer) };
    let mut flags_info =
        vk::MemoryAllocateFlagsInfo::default().flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
    let mut info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(ctx.memory_type_from_properties(
            requirements.memory_type_bits,
            mem_requirements_mask,
            mem_preferred_mask,
        ));
    if get_device_address {
        info = info.push_next(&mut flags_info);
    }
    allocate_vulkan_memory(
        ctx,
        memory,
        &info,
        VulkanMemoryType::Device,
        num_allocations,
    );
    ctx.name_object(memory.handle, &c_string(&format!("{name} memory")));

    // SAFETY: `buffer` is unbound and `memory` is a fresh allocation of at
    // least `requirements.size` bytes of a type `requirements` permits.
    if let Err(err) = unsafe { ctx.device.bind_buffer_memory(buffer, memory.handle, 0) } {
        ctx.vk_fail("vkBindBufferMemory", err);
    }
    let address = get_device_address.then(|| ctx.buffer_device_address(buffer));
    (buffer, address)
}

/// `R_FreeBuffer`.
pub fn free_buffer<E: Engine>(
    ctx: &Ctx<'_, E>,
    buffer: vk::Buffer,
    memory: &mut VulkanMemory,
    num_allocations: Option<&AtomicU32>,
) {
    if buffer != vk::Buffer::null() {
        // SAFETY: the buffer came from `create_buffer` and is no longer in use.
        unsafe { ctx.device.destroy_buffer(buffer, None) };
        free_vulkan_memory(ctx, memory, num_allocations);
    }
}

/// One entry of `R_CreateBuffers`' `buffer_create_info_t` array, minus the
/// out-pointers.
#[derive(Clone, Copy, Debug)]
pub struct BufferRequest<'a> {
    pub size: u64,
    pub alignment: u64,
    pub usage: vk::BufferUsageFlags,
    pub mapped: bool,
    pub address: bool,
    pub name: &'a str,
}

/// What `R_CreateBuffers` writes back for one request.
#[derive(Clone, Copy, Debug)]
pub struct BufferResult {
    pub buffer: vk::Buffer,
    /// The host pointer when the request was `mapped`.
    pub mapped: *mut c_void,
    /// The device address when the request asked for one and the device
    /// supports it.
    pub address: Option<vk::DeviceAddress>,
}

/// `R_CreateBuffers`: `requests.len()` buffers suballocated from one
/// allocation named `memory_name`. Returns the allocation size.
pub fn create_buffers<E: Engine>(
    ctx: &Ctx<'_, E>,
    requests: &[BufferRequest<'_>],
    memory: &mut VulkanMemory,
    mem_requirements_mask: vk::MemoryPropertyFlags,
    mem_preferred_mask: vk::MemoryPropertyFlags,
    num_allocations: Option<&AtomicU32>,
    memory_name: &CStr,
) -> (u64, Vec<BufferResult>) {
    let has_get_address = ctx.vg.vk_get_buffer_device_address.is_some();
    let mut get_device_address = false;
    let mut usage_union = vk::BufferUsageFlags::empty();
    let mut usages = Vec::with_capacity(requests.len());
    for request in requests {
        let mut usage = request.usage;
        if has_get_address && request.address {
            get_device_address = true;
            usage |= vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS_KHR;
        }
        usage_union |= usage;
        usages.push(usage);
    }

    let mut total_size = 0u64;
    let mut map_memory = false;
    let mut results = Vec::with_capacity(requests.len());
    let mut requirements = Vec::with_capacity(requests.len());
    for (request, usage) in requests.iter().zip(&usages) {
        let buffer = create_named_buffer(
            ctx,
            request.size,
            *usage,
            &format!("{} buffer", request.name),
        );
        // SAFETY: `buffer` was just created on this device.
        let reqs = unsafe { ctx.device.get_buffer_memory_requirements(buffer) };
        let alignment = reqs.alignment.max(request.alignment);
        total_size = q_align(total_size, alignment) + reqs.size;
        map_memory |= request.mapped;
        results.push(BufferResult {
            buffer,
            mapped: core::ptr::null_mut(),
            address: None,
        });
        requirements.push(reqs);
    }

    let dummy = vk::BufferCreateInfo::default()
        .size(total_size)
        .usage(usage_union);
    // SAFETY: `dummy` is complete; the buffer is destroyed right after its
    // memory requirements are read.
    let memory_type_bits = unsafe {
        let dummy_buffer = match ctx.device.create_buffer(&dummy, None) {
            Ok(buffer) => buffer,
            Err(err) => ctx.vk_fail("vkCreateBuffer", err),
        };
        let bits = ctx
            .device
            .get_buffer_memory_requirements(dummy_buffer)
            .memory_type_bits;
        ctx.device.destroy_buffer(dummy_buffer, None);
        bits
    };

    let mut flags_info =
        vk::MemoryAllocateFlagsInfo::default().flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
    let mut info = vk::MemoryAllocateInfo::default()
        .allocation_size(total_size)
        .memory_type_index(ctx.memory_type_from_properties(
            memory_type_bits,
            mem_requirements_mask,
            mem_preferred_mask,
        ));
    if get_device_address {
        info = info.push_next(&mut flags_info);
    }
    allocate_vulkan_memory(
        ctx,
        memory,
        &info,
        VulkanMemoryType::Device,
        num_allocations,
    );
    ctx.name_object(memory.handle, memory_name);

    let mut mapped_base: *mut u8 = core::ptr::null_mut();
    if map_memory {
        // SAFETY: `memory` is a fresh host-visible allocation of `total_size`
        // bytes (the caller's masks asked for HOST_VISIBLE when mapping).
        mapped_base = match unsafe {
            ctx.device
                .map_memory(memory.handle, 0, total_size, vk::MemoryMapFlags::empty())
        } {
            Ok(ptr) => ptr.cast(),
            Err(err) => ctx.vk_fail("vkMapMemory", err),
        };
    }

    let mut current_offset = 0u64;
    for ((request, reqs), result) in requests.iter().zip(&requirements).zip(&mut results) {
        let alignment = reqs.alignment.max(request.alignment);
        current_offset = q_align(current_offset, alignment);
        // SAFETY: `result.buffer` is unbound and `[current_offset,
        // current_offset + reqs.size)` lies inside the allocation.
        if let Err(err) = unsafe {
            ctx.device
                .bind_buffer_memory(result.buffer, memory.handle, current_offset)
        } {
            ctx.vk_fail("vkBindBufferMemory", err);
        }
        if request.mapped {
            result.mapped = mapped_base.wrapping_add(current_offset as usize).cast();
        }
        current_offset += reqs.size;
        if get_device_address && request.address {
            result.address = Some(ctx.buffer_device_address(result.buffer));
        }
    }
    (total_size, results)
}

/// `R_FreeBuffers`.
pub fn free_buffers<E: Engine>(
    ctx: &Ctx<'_, E>,
    buffers: &[vk::Buffer],
    memory: &mut VulkanMemory,
    num_allocations: Option<&AtomicU32>,
) {
    for &buffer in buffers {
        if buffer != vk::Buffer::null() {
            // SAFETY: the buffers came from `create_buffers` and are no longer in use.
            unsafe { ctx.device.destroy_buffer(buffer, None) };
        }
    }
    free_vulkan_memory(ctx, memory, num_allocations);
}

pub(crate) fn c_string(name: &str) -> CString {
    CString::new(name).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(flags: &[vk::MemoryPropertyFlags]) -> vk::PhysicalDeviceMemoryProperties {
        let mut props = vk::PhysicalDeviceMemoryProperties {
            memory_type_count: flags.len() as u32,
            ..Default::default()
        };
        for (slot, &f) in props.memory_types.iter_mut().zip(flags) {
            slot.property_flags = f;
        }
        props
    }

    #[test]
    fn memory_type_prefers_both_masks_then_required() {
        use vk::MemoryPropertyFlags as F;
        let p = props(&[
            F::DEVICE_LOCAL,
            F::HOST_VISIBLE | F::HOST_COHERENT,
            F::HOST_VISIBLE | F::HOST_CACHED,
        ]);
        assert_eq!(
            memory_type_from_properties(&p, 0b111, F::HOST_VISIBLE, F::HOST_CACHED),
            Some(2)
        );
        assert_eq!(
            memory_type_from_properties(&p, 0b011, F::HOST_VISIBLE, F::HOST_CACHED),
            Some(1)
        );
        assert_eq!(
            memory_type_from_properties(&p, 0b110, F::DEVICE_LOCAL, F::empty()),
            None
        );
        assert_eq!(
            memory_type_from_properties(&p, 0b001, F::DEVICE_LOCAL, F::empty()),
            Some(0)
        );
        assert_eq!(
            memory_type_from_properties(&p, 0, F::empty(), F::empty()),
            None
        );
    }
}
