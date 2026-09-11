//! The dynamic vertex/index/uniform/storage ring buffers, the fan index
//! buffer and the per-frame garbage (`R_InitGPUBuffers` ..
//! `R_CollectDynamicBufferGarbage`, `R_*Allocate`).

use core::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::{Mutex, MutexGuard};

use ash::vk;
use quake_types::render::{
    VulkanMemory, VulkanMemoryType, FAN_INDEX_BUFFER_SIZE, MAX_UNIFORM_ALLOC,
};

use super::descriptors::{allocate_descriptor_set, free_descriptor_set};
use super::memory::{allocate_vulkan_memory, free_vulkan_memory};
use super::staging::Staging;
use super::{q_align, q_next_pow2, Ctx, Engine};

pub const INITIAL_DYNAMIC_VERTEX_BUFFER_SIZE_KB: u32 = 256;
pub const INITIAL_DYNAMIC_INDEX_BUFFER_SIZE_KB: u32 = 1024;
pub const INITIAL_DYNAMIC_UNIFORM_BUFFER_SIZE_KB: u32 = 256;
pub const NUM_DYNAMIC_BUFFERS: usize = 2;
pub const GARBAGE_FRAME_COUNT: usize = 3;

/// See `staging::HostPtr`.
#[derive(Clone, Copy)]
struct HostPtr(*mut u8);
// SAFETY: device-mapped memory owned by Vulkan; the pointer is only handed
// out under the ring's mutex and never dereferenced here.
unsafe impl Send for HostPtr {}
// SAFETY: as above.
unsafe impl Sync for HostPtr {}

/// `dynbuffer_t`.
#[derive(Clone, Copy)]
struct DynBuf {
    buffer: vk::Buffer,
    current_offset: u32,
    data: HostPtr,
    device_address: vk::DeviceAddress,
}

impl DynBuf {
    const NONE: DynBuf = DynBuf {
        buffer: vk::Buffer::null(),
        current_offset: 0,
        data: HostPtr(core::ptr::null_mut()),
        device_address: 0,
    };
}

/// Which of the four rings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Vertex,
    Index,
    Uniform,
    Storage,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Vertex => "vertex buffer",
            Kind::Index => "index buffer",
            Kind::Uniform => "uniform buffer",
            Kind::Storage => "storage buffer",
        }
    }
}

/// One ring: the `dyn_*_buffers`, `dyn_*_buffer_memory`,
/// `current_dyn_*_buffer_size` statics (plus `ubo_descriptor_sets` for the
/// uniform ring).
struct Ring {
    kind: Kind,
    buffers: [DynBuf; NUM_DYNAMIC_BUFFERS],
    memory: VulkanMemory,
    current_size: u32,
    descriptor_sets: [vk::DescriptorSet; NUM_DYNAMIC_BUFFERS],
}

impl Ring {
    const fn new(kind: Kind, current_size: u32) -> Self {
        Ring {
            kind,
            buffers: [DynBuf::NONE; NUM_DYNAMIC_BUFFERS],
            memory: VulkanMemory {
                handle: vk::DeviceMemory::null(),
                size: 0,
                type_: VulkanMemoryType::None,
            },
            current_size,
            descriptor_sets: [vk::DescriptorSet::null(); NUM_DYNAMIC_BUFFERS],
        }
    }
}

#[derive(Default)]
struct GarbageFrame {
    memory: Vec<VulkanMemory>,
    buffers: Vec<vk::Buffer>,
    descriptor_sets: Vec<vk::DescriptorSet>,
}

struct Garbage {
    current: usize,
    frames: [GarbageFrame; GARBAGE_FRAME_COUNT],
}

/// The dynamic-buffer statics of `gl_rmisc.c`.
pub struct DynBuffers {
    vertex: Mutex<Ring>,
    index: Mutex<Ring>,
    uniform: Mutex<Ring>,
    storage: Mutex<Ring>,
    /// `current_dyn_buffer_index`: written by `swap`, read by the allocators
    /// (a plain `int` in C, read without the ring mutexes).
    current_index: AtomicUsize,
    garbage: Mutex<Garbage>,
}

/// What `R_DynBufferAllocate` writes back.
#[derive(Clone, Copy, Debug)]
pub struct DynAllocation {
    pub data: *mut u8,
    pub buffer: vk::Buffer,
    pub buffer_offset: vk::DeviceSize,
    pub device_address: vk::DeviceAddress,
    pub descriptor_set: vk::DescriptorSet,
}

fn lock(ring: &Mutex<Ring>) -> MutexGuard<'_, Ring> {
    ring.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Default for DynBuffers {
    fn default() -> Self {
        Self::new()
    }
}

impl DynBuffers {
    pub const fn new() -> Self {
        DynBuffers {
            vertex: Mutex::new(Ring::new(
                Kind::Vertex,
                INITIAL_DYNAMIC_VERTEX_BUFFER_SIZE_KB * 1024,
            )),
            index: Mutex::new(Ring::new(
                Kind::Index,
                INITIAL_DYNAMIC_INDEX_BUFFER_SIZE_KB * 1024,
            )),
            uniform: Mutex::new(Ring::new(
                Kind::Uniform,
                INITIAL_DYNAMIC_UNIFORM_BUFFER_SIZE_KB * 1024,
            )),
            storage: Mutex::new(Ring::new(Kind::Storage, 0)),
            current_index: AtomicUsize::new(0),
            garbage: Mutex::new(Garbage {
                current: 0,
                frames: [GarbageFrame::EMPTY; GARBAGE_FRAME_COUNT],
            }),
        }
    }

    fn ring(&self, kind: Kind) -> &Mutex<Ring> {
        match kind {
            Kind::Vertex => &self.vertex,
            Kind::Index => &self.index,
            Kind::Uniform => &self.uniform,
            Kind::Storage => &self.storage,
        }
    }

    /// `R_InitDynamicBuffers` plus the per-kind wrapper
    /// (`R_InitDynamicVertexBuffers` .. `R_InitDynamicStorageBuffers`).
    fn init_ring<E: Engine>(ctx: &mut Ctx<'_, E>, ring: &mut Ring) {
        let name = ring.kind.name();
        ctx.engine.sys_printf(&format!(
            "Reallocating dynamic {name}s ({} KB)\n",
            ring.current_size / 1024
        ));

        let (mut usage_flags, get_device_address) = match ring.kind {
            Kind::Vertex => (vk::BufferUsageFlags::VERTEX_BUFFER, false),
            Kind::Index => (vk::BufferUsageFlags::INDEX_BUFFER, false),
            Kind::Uniform => (vk::BufferUsageFlags::UNIFORM_BUFFER, false),
            Kind::Storage => {
                let mut usage = vk::BufferUsageFlags::STORAGE_BUFFER;
                let gda = ctx.vg.ray_query;
                if gda {
                    usage |= vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR;
                }
                (usage, gda)
            }
        };
        if get_device_address {
            usage_flags |= vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;
        }

        let cname = super::memory::c_string(name);
        let info = vk::BufferCreateInfo::default()
            .size(ring.current_size as u64)
            .usage(usage_flags);
        for db in &mut ring.buffers {
            db.current_offset = 0;
            // SAFETY: `info` is complete and the device is live.
            db.buffer = match unsafe { ctx.device.create_buffer(&info, None) } {
                Ok(buffer) => buffer,
                Err(err) => ctx.vk_fail("vkCreateBuffer", err),
            };
            ctx.name_object(db.buffer, &cname);
        }

        // SAFETY: the buffer was just created.
        let requirements = unsafe {
            ctx.device
                .get_buffer_memory_requirements(ring.buffers[0].buffer)
        };
        let aligned_size = q_align(ring.current_size as u64, requirements.alignment);

        let mut flags_info =
            vk::MemoryAllocateFlagsInfo::default().flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
        let mut alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(NUM_DYNAMIC_BUFFERS as u64 * aligned_size)
            .memory_type_index(ctx.memory_type_from_properties(
                requirements.memory_type_bits,
                vk::MemoryPropertyFlags::HOST_VISIBLE,
                vk::MemoryPropertyFlags::HOST_CACHED,
            ));
        if get_device_address {
            alloc = alloc.push_next(&mut flags_info);
        }
        allocate_vulkan_memory(
            ctx,
            &mut ring.memory,
            &alloc,
            VulkanMemoryType::Host,
            Some(ctx.counters.dynbuf),
        );
        ctx.name_object(ring.memory.handle, &cname);

        for (i, db) in ring.buffers.iter().enumerate() {
            // SAFETY: `db.buffer` is unbound and the slice lies inside the allocation.
            if let Err(err) = unsafe {
                ctx.device.bind_buffer_memory(
                    db.buffer,
                    ring.memory.handle,
                    i as u64 * aligned_size,
                )
            } {
                ctx.vk_fail("vkBindBufferMemory", err);
            }
        }

        // SAFETY: the allocation is host-visible and not yet mapped.
        let base: *mut u8 = match unsafe {
            ctx.device.map_memory(
                ring.memory.handle,
                0,
                NUM_DYNAMIC_BUFFERS as u64 * aligned_size,
                vk::MemoryMapFlags::empty(),
            )
        } {
            Ok(ptr) => ptr.cast(),
            Err(err) => ctx.vk_fail("vkMapMemory", err),
        };
        for (i, db) in ring.buffers.iter_mut().enumerate() {
            db.data = HostPtr(base.wrapping_add(i * aligned_size as usize));
            if get_device_address {
                db.device_address = ctx.buffer_device_address(db.buffer);
            }
        }

        if ring.kind == Kind::Uniform {
            let layout = ctx.vg.ubo_set_layout;
            for (db, set) in ring.buffers.iter().zip(&mut ring.descriptor_sets) {
                *set = allocate_descriptor_set(ctx, &layout);
                let buffer_info = [vk::DescriptorBufferInfo::default()
                    .buffer(db.buffer)
                    .offset(0)
                    .range(MAX_UNIFORM_ALLOC as u64)];
                let write = vk::WriteDescriptorSet::default()
                    .dst_set(*set)
                    .dst_binding(0)
                    .dst_array_element(0)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                    .buffer_info(&buffer_info);
                // SAFETY: `write` points at the live `buffer_info` local and
                // `*set` is a freshly allocated set of `ubo_set_layout`.
                unsafe { ctx.device.update_descriptor_sets(&[write], &[]) };
            }
        }
    }

    /// `R_InitFanIndexBuffer`.
    fn init_fan_index_buffer<E: Engine>(ctx: &mut Ctx<'_, E>, staging: &Staging) {
        let buffer_size = core::mem::size_of::<u16>() * FAN_INDEX_BUFFER_SIZE;
        let info = vk::BufferCreateInfo::default()
            .size(buffer_size as u64)
            .usage(vk::BufferUsageFlags::INDEX_BUFFER | vk::BufferUsageFlags::TRANSFER_DST);
        // SAFETY: `info` is complete and the device is live.
        ctx.vg.fan_index_buffer = match unsafe { ctx.device.create_buffer(&info, None) } {
            Ok(buffer) => buffer,
            Err(err) => ctx.vk_fail("vkCreateBuffer", err),
        };
        ctx.name_object(ctx.vg.fan_index_buffer, c"Quad Index Buffer");

        // SAFETY: the buffer was just created.
        let requirements = unsafe {
            ctx.device
                .get_buffer_memory_requirements(ctx.vg.fan_index_buffer)
        };
        let alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(ctx.memory_type_from_properties(
                requirements.memory_type_bits,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
                vk::MemoryPropertyFlags::empty(),
            ));
        ctx.counters.dynbuf.fetch_add(1, SeqCst);
        ctx.counters
            .total_device
            .fetch_add(requirements.size, SeqCst);
        // SAFETY: `alloc` is complete. The allocation is intentionally
        // leaked, as in C (the fan index buffer lives for the process).
        let memory = match unsafe { ctx.device.allocate_memory(&alloc, None) } {
            Ok(memory) => memory,
            Err(err) => ctx.vk_fail("vkAllocateMemory", err),
        };
        // SAFETY: the buffer is unbound and `memory` is a fresh allocation of `requirements.size`.
        if let Err(err) = unsafe {
            ctx.device
                .bind_buffer_memory(ctx.vg.fan_index_buffer, memory, 0)
        } {
            ctx.vk_fail("vkBindBufferMemory", err);
        }

        let allocation = staging.allocate(ctx, buffer_size as i32, 1);
        let region = vk::BufferCopy::default()
            .src_offset(allocation.buffer_offset as u64)
            .dst_offset(0)
            .size(buffer_size as u64);
        // SAFETY: the staging command buffer is recording under the logical
        // staging lock held since `allocate`.
        unsafe {
            ctx.device.cmd_copy_buffer(
                allocation.command_buffer,
                allocation.buffer,
                ctx.vg.fan_index_buffer,
                &[region],
            )
        };
        staging.begin_copy();
        let indices = fan_indices();
        // SAFETY: `allocation.data` points at `buffer_size` writable bytes of
        // the mapped staging buffer reserved for this allocation.
        unsafe {
            core::ptr::copy_nonoverlapping(
                indices.as_ptr().cast::<u8>(),
                allocation.data,
                buffer_size,
            )
        };
        staging.end_copy();
    }

    /// `R_InitGPUBuffers`.
    pub fn init_gpu_buffers<E: Engine>(&self, ctx: &mut Ctx<'_, E>, staging: &Staging) {
        Self::init_ring(ctx, &mut lock(&self.vertex));
        Self::init_ring(ctx, &mut lock(&self.index));
        Self::init_ring(ctx, &mut lock(&self.uniform));
        Self::init_fan_index_buffer(ctx, staging);
    }

    /// `R_SwapDynamicBuffers`.
    pub fn swap(&self) {
        let index = (self.current_index.load(SeqCst) + 1) % NUM_DYNAMIC_BUFFERS;
        self.current_index.store(index, SeqCst);
        for ring in [&self.vertex, &self.index, &self.uniform, &self.storage] {
            lock(ring).buffers[index].current_offset = 0;
        }
    }

    /// `R_FlushDynamicBuffers`. `frame_upload_memory` is
    /// `frame_upload_buffers_memory.handle` (`r_brush.c`).
    pub fn flush<E: Engine>(&self, ctx: &Ctx<'_, E>, frame_upload_memory: vk::DeviceMemory) {
        let mut ranges = Vec::with_capacity(5);
        let range = |memory: vk::DeviceMemory| {
            vk::MappedMemoryRange::default()
                .memory(memory)
                .offset(0)
                .size(vk::WHOLE_SIZE)
        };
        ranges.push(range(lock(&self.vertex).memory.handle));
        ranges.push(range(lock(&self.index).memory.handle));
        ranges.push(range(lock(&self.uniform).memory.handle));
        ranges.push(range(frame_upload_memory));
        let storage = lock(&self.storage).memory.handle;
        if storage != vk::DeviceMemory::null() {
            ranges.push(range(storage));
        }
        // SAFETY: every range names a live mapped allocation.
        if let Err(err) = unsafe { ctx.device.flush_mapped_memory_ranges(&ranges) } {
            ctx.vk_fail("vkFlushMappedMemoryRanges", err);
        }
    }

    /// `R_AddDynamicBufferGarbage`: retires `memory`, `buffers` and (for a
    /// uniform ring) its two descriptor sets `GARBAGE_FRAME_COUNT` frames
    /// from now. Also the C-visible entry point (`r_brush.c` retires its
    /// acceleration-structure scratch buffers through it).
    pub fn add_garbage(
        &self,
        memory: VulkanMemory,
        buffers: &[vk::Buffer],
        descriptor_sets: Option<&[vk::DescriptorSet]>,
    ) {
        let mut garbage = self
            .garbage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let current = garbage.current;
        let frame = &mut garbage.frames[current];
        frame.memory.push(memory);
        frame.buffers.extend_from_slice(buffers);
        if let Some(sets) = descriptor_sets {
            frame.descriptor_sets.extend_from_slice(sets);
        }
    }

    fn add_ring_garbage(&self, ring: &Ring) {
        let buffers: Vec<vk::Buffer> = ring.buffers.iter().map(|db| db.buffer).collect();
        let sets = (ring.kind == Kind::Uniform).then_some(&ring.descriptor_sets[..]);
        self.add_garbage(ring.memory, &buffers, sets);
    }

    /// `R_CollectDynamicBufferGarbage`.
    pub fn collect_garbage<E: Engine>(&self, ctx: &mut Ctx<'_, E>) {
        let mut garbage = self
            .garbage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        garbage.current = (garbage.current + 1) % GARBAGE_FRAME_COUNT;
        let collect = (garbage.current + 1) % GARBAGE_FRAME_COUNT;
        let frame = core::mem::take(&mut garbage.frames[collect]);
        drop(garbage);

        let layout = ctx.vg.ubo_set_layout;
        for set in frame.descriptor_sets {
            free_descriptor_set(ctx, set, &layout);
        }
        for buffer in frame.buffers {
            // SAFETY: the buffer was retired `GARBAGE_FRAME_COUNT` frames ago
            // and no submission references it any more.
            unsafe { ctx.device.destroy_buffer(buffer, None) };
        }
        for mut memory in frame.memory {
            free_vulkan_memory(ctx, &mut memory, Some(ctx.counters.dynbuf));
        }
    }

    /// `R_DynBufferAllocate`.
    fn allocate<E: Engine>(
        &self,
        ctx: &mut Ctx<'_, E>,
        kind: Kind,
        size: u32,
        alignment: u32,
        min_tail_size: u32,
    ) -> DynAllocation {
        let mut ring = lock(self.ring(kind));
        let index = self.current_index.load(SeqCst);
        let aligned_size = q_align(size as u64, alignment as u64) as u32;
        if ring.buffers[index].current_offset + size.max(min_tail_size) > ring.current_size {
            self.add_ring_garbage(&ring);
            ring.current_size = grown_size(ring.current_size, size);
            Self::init_ring(ctx, &mut ring);
        }
        let db = &mut ring.buffers[index];
        let allocation = DynAllocation {
            data: db.data.0.wrapping_add(db.current_offset as usize),
            buffer: db.buffer,
            buffer_offset: db.current_offset as vk::DeviceSize,
            device_address: db.device_address + db.current_offset as vk::DeviceAddress,
            descriptor_set: ring.descriptor_sets[index],
        };
        ring.buffers[index].current_offset += aligned_size;
        allocation
    }

    /// `R_VertexAllocate`: aligned to `sizeof (float)`.
    pub fn vertex_allocate<E: Engine>(&self, ctx: &mut Ctx<'_, E>, size: u32) -> DynAllocation {
        self.allocate(ctx, Kind::Vertex, size, 4, 0)
    }

    /// `R_IndexAllocate`: aligned to 4 for mixed 16/32-bit indices.
    pub fn index_allocate<E: Engine>(&self, ctx: &mut Ctx<'_, E>, size: u32) -> DynAllocation {
        self.allocate(ctx, Kind::Index, size, 4, 0)
    }

    /// `R_UniformAllocate`.
    pub fn uniform_allocate<E: Engine>(&self, ctx: &mut Ctx<'_, E>, size: u32) -> DynAllocation {
        if size as usize > MAX_UNIFORM_ALLOC {
            ctx.engine.sys_error("Increase MAX_UNIFORM_ALLOC");
        }
        let alignment = ctx
            .vg
            .device_properties
            .limits
            .min_uniform_buffer_offset_alignment as u32;
        self.allocate(
            ctx,
            Kind::Uniform,
            size,
            alignment,
            MAX_UNIFORM_ALLOC as u32,
        )
    }

    /// `R_StorageAllocate`.
    pub fn storage_allocate<E: Engine>(&self, ctx: &mut Ctx<'_, E>, size: u32) -> DynAllocation {
        let alignment = ctx
            .vg
            .device_properties
            .limits
            .min_storage_buffer_offset_alignment as u32;
        self.allocate(ctx, Kind::Storage, size, alignment, 0)
    }
}

impl GarbageFrame {
    const EMPTY: GarbageFrame = GarbageFrame {
        memory: Vec::new(),
        buffers: Vec::new(),
        descriptor_sets: Vec::new(),
    };
}

/// `*current_size = q_max (*current_size * 2, Q_nextPow2 (size))`.
pub fn grown_size(current_size: u32, size: u32) -> u32 {
    (current_size * 2).max(q_next_pow2(size))
}

/// The fan index pattern uploaded by `R_InitFanIndexBuffer`: `[0, 1 + i, 2 + i]`
/// for every triangle.
pub fn fan_indices() -> [u16; FAN_INDEX_BUFFER_SIZE] {
    let mut indices = [0u16; FAN_INDEX_BUFFER_SIZE];
    for (i, tri) in indices.chunks_exact_mut(3).enumerate() {
        tri[0] = 0;
        tri[1] = 1 + i as u16;
        tri[2] = 2 + i as u16;
    }
    indices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grow_doubles_or_jumps_to_next_pow2() {
        assert_eq!(grown_size(256 * 1024, 100), 512 * 1024);
        assert_eq!(grown_size(0, 100), 128);
        assert_eq!(grown_size(0, 128), 128);
        assert_eq!(grown_size(1024, 5000), 8192);
    }

    #[test]
    fn fan_indices_match_c_loop() {
        let indices = fan_indices();
        assert_eq!(&indices[..6], &[0, 1, 2, 0, 2, 3]);
        assert_eq!(indices[FAN_INDEX_BUFFER_SIZE - 3..], [0, 42, 43]);
    }
}
