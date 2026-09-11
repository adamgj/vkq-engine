//! The staging buffers (`R_InitStagingBuffers` .. `R_StagingUploadBuffer`).
//!
//! The C keeps one `staging_mutex` locked from `R_StagingAllocate` until
//! `R_StagingBeginCopy` (the caller records into the staging command buffer
//! in between) and a `staging_cond` on `num_stagings_in_flight`. A
//! `std::sync::Mutex` guard cannot cross that FFI return, so the lock is
//! modelled explicitly: `held` is the logical `staging_mutex`, the `Mutex`
//! only guards the bookkeeping, and the `Condvar` carries both the "mutex
//! released" and the "copy finished" wakeups.

use std::sync::{Condvar, Mutex, MutexGuard};

use ash::vk;
use quake_types::render::{VulkanMemory, VulkanMemoryType};

use super::memory::{allocate_vulkan_memory, free_vulkan_memory};
use super::{q_align, Ctx, Engine};

pub const NUM_STAGING_BUFFERS: usize = 2;

/// A host pointer into the mapped staging memory. The memory is owned by
/// the Vulkan device, the pointer is only ever dereferenced by the C/Rust
/// uploader that holds the logical staging lock.
#[derive(Clone, Copy)]
struct HostPtr(*mut u8);
// SAFETY: the pointee is device-mapped memory that outlives the process's
// use of it; access is serialised by the logical staging lock.
unsafe impl Send for HostPtr {}
// SAFETY: as above -- no Rust reference to the mapping is ever formed.
unsafe impl Sync for HostPtr {}

/// `stagingbuffer_t`.
#[derive(Clone, Copy)]
struct StagingBuffer {
    buffer: vk::Buffer,
    command_buffer: vk::CommandBuffer,
    fence: vk::Fence,
    current_offset: i32,
    submitted: bool,
    data: HostPtr,
}

impl StagingBuffer {
    const NONE: StagingBuffer = StagingBuffer {
        buffer: vk::Buffer::null(),
        command_buffer: vk::CommandBuffer::null(),
        fence: vk::Fence::null(),
        current_offset: 0,
        submitted: false,
        data: HostPtr(core::ptr::null_mut()),
    };
}

struct Shared {
    /// The logical `staging_mutex`.
    held: bool,
    num_stagings_in_flight: u32,
    command_pool: vk::CommandPool,
    memory: VulkanMemory,
    buffers: [StagingBuffer; NUM_STAGING_BUFFERS],
    current: usize,
}

/// The `static`s of the staging section of `gl_rmisc.c`.
pub struct Staging {
    shared: Mutex<Shared>,
    cond: Condvar,
}

/// What `R_StagingAllocate` hands back.
#[derive(Clone, Copy, Debug)]
pub struct StagingAllocation {
    pub data: *mut u8,
    pub command_buffer: vk::CommandBuffer,
    pub buffer: vk::Buffer,
    pub buffer_offset: i32,
}

impl Default for Staging {
    fn default() -> Self {
        Self::new()
    }
}

impl Staging {
    pub const fn new() -> Self {
        Staging {
            shared: Mutex::new(Shared {
                held: false,
                num_stagings_in_flight: 0,
                command_pool: vk::CommandPool::null(),
                memory: VulkanMemory {
                    handle: vk::DeviceMemory::null(),
                    size: 0,
                    type_: VulkanMemoryType::None,
                },
                buffers: [StagingBuffer::NONE; NUM_STAGING_BUFFERS],
                current: 0,
            }),
            cond: Condvar::new(),
        }
    }

    fn guard(&self) -> MutexGuard<'_, Shared> {
        self.shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// `QMutex_Lock (staging_mutex)`: wait for the logical lock, take it, and
    /// return the bookkeeping guard.
    fn acquire(&self) -> MutexGuard<'_, Shared> {
        let mut g = self.guard();
        while g.held {
            g = self
                .cond
                .wait(g)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        g.held = true;
        g
    }

    /// `QMutex_Unlock (staging_mutex)`.
    fn release(&self, mut g: MutexGuard<'_, Shared>) {
        g.held = false;
        drop(g);
        self.cond.notify_all();
    }

    /// `while (num_stagings_in_flight > 0) QCond_Wait (staging_cond,
    /// staging_mutex)`, with the logical lock held on entry and exit.
    fn wait_no_copies<'g>(&'g self, mut g: MutexGuard<'g, Shared>) -> MutexGuard<'g, Shared> {
        while g.num_stagings_in_flight > 0 {
            g.held = false;
            self.cond.notify_all();
            g = self
                .cond
                .wait(g)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            while g.held {
                g = self
                    .cond
                    .wait(g)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            g.held = true;
        }
        g
    }

    /// `R_CreateStagingBuffers`.
    fn create_buffers<E: Engine>(&self, ctx: &mut Ctx<'_, E>, s: &mut Shared) {
        let size = ctx.vg.staging_buffer_size as u64;
        let info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC);
        for sb in &mut s.buffers {
            sb.current_offset = 0;
            sb.submitted = false;
            // SAFETY: `info` is complete and the device is live.
            sb.buffer = match unsafe { ctx.device.create_buffer(&info, None) } {
                Ok(buffer) => buffer,
                Err(err) => ctx.vk_fail("vkCreateBuffer", err),
            };
            ctx.name_object(sb.buffer, c"Staging Buffer");
        }

        // SAFETY: the buffer was just created.
        let requirements = unsafe {
            ctx.device
                .get_buffer_memory_requirements(s.buffers[0].buffer)
        };
        let aligned_size = q_align(size, requirements.alignment);
        let memory_type_index = ctx.memory_type_from_properties(
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE,
            vk::MemoryPropertyFlags::HOST_CACHED,
        );
        let alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(NUM_STAGING_BUFFERS as u64 * aligned_size)
            .memory_type_index(memory_type_index);
        allocate_vulkan_memory(
            ctx,
            &mut s.memory,
            &alloc,
            VulkanMemoryType::Host,
            Some(ctx.counters.misc),
        );
        ctx.name_object(s.memory.handle, c"Staging Buffers");

        for (i, sb) in s.buffers.iter().enumerate() {
            // SAFETY: `sb.buffer` is unbound and the slice lies inside the allocation.
            if let Err(err) = unsafe {
                ctx.device
                    .bind_buffer_memory(sb.buffer, s.memory.handle, i as u64 * aligned_size)
            } {
                ctx.vk_fail("vkBindBufferMemory", err);
            }
        }

        // SAFETY: the allocation is host-visible and not yet mapped.
        let base: *mut u8 = match unsafe {
            ctx.device.map_memory(
                s.memory.handle,
                0,
                NUM_STAGING_BUFFERS as u64 * aligned_size,
                vk::MemoryMapFlags::empty(),
            )
        } {
            Ok(ptr) => ptr.cast(),
            Err(err) => ctx.vk_fail("vkMapMemory", err),
        };
        for (i, sb) in s.buffers.iter_mut().enumerate() {
            sb.data = HostPtr(base.wrapping_add(i * aligned_size as usize));
        }
    }

    /// `R_DestroyStagingBuffers`.
    fn destroy_buffers<E: Engine>(&self, ctx: &mut Ctx<'_, E>, s: &mut Shared) {
        free_vulkan_memory(ctx, &mut s.memory, Some(ctx.counters.misc));
        for sb in &s.buffers {
            // SAFETY: every submission using the buffer has been waited for.
            unsafe { ctx.device.destroy_buffer(sb.buffer, None) };
        }
    }

    /// `R_InitStagingBuffers`.
    pub fn init<E: Engine>(&self, ctx: &mut Ctx<'_, E>) {
        ctx.engine.con_printf("Initializing staging\n");
        let mut g = self.guard();
        let s = &mut *g;
        self.create_buffers(ctx, s);

        let pool_info = vk::CommandPoolCreateInfo::default()
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
            .queue_family_index(ctx.vg.gfx_queue_family_index);
        // SAFETY: `pool_info` is complete.
        s.command_pool = match unsafe { ctx.device.create_command_pool(&pool_info, None) } {
            Ok(pool) => pool,
            Err(err) => ctx.vk_fail("vkCreateCommandPool", err),
        };

        let cb_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(s.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(NUM_STAGING_BUFFERS as u32);
        // SAFETY: the pool was just created on this device.
        let command_buffers = match unsafe { ctx.device.allocate_command_buffers(&cb_info) } {
            Ok(cbs) => cbs,
            Err(err) => ctx.vk_fail("vkAllocateCommandBuffers", err),
        };

        let fence_info = vk::FenceCreateInfo::default();
        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        for (sb, cb) in s.buffers.iter_mut().zip(command_buffers) {
            // SAFETY: `fence_info` is complete.
            sb.fence = match unsafe { ctx.device.create_fence(&fence_info, None) } {
                Ok(fence) => fence,
                Err(err) => ctx.vk_fail("vkCreateFence", err),
            };
            sb.command_buffer = cb;
            // SAFETY: `cb` is a fresh primary command buffer from `s.command_pool`.
            if let Err(err) = unsafe { ctx.device.begin_command_buffer(cb, &begin_info) } {
                ctx.vk_fail("vkBeginCommandBuffer", err);
            }
        }
    }

    /// `R_SubmitStagingBuffer`, with the logical lock held.
    fn submit_buffer<'g, E: Engine>(
        &'g self,
        ctx: &mut Ctx<'_, E>,
        mut g: MutexGuard<'g, Shared>,
        index: usize,
    ) -> MutexGuard<'g, Shared> {
        g = self.wait_no_copies(g);
        let s = &mut *g;
        let sb = s.buffers[index];

        let memory_barrier = vk::MemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE);
        // SAFETY: `sb.command_buffer` is in the recording state (begun by
        // `init`/`flush_command_buffer`) and belongs to this thread by the
        // logical lock.
        unsafe {
            ctx.device.cmd_pipeline_barrier(
                sb.command_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::DependencyFlags::empty(),
                &[memory_barrier],
                &[],
                &[],
            );
            if let Err(err) = ctx.device.end_command_buffer(sb.command_buffer) {
                ctx.vk_fail("vkEndCommandBuffer", err);
            }
        }

        let range = vk::MappedMemoryRange::default()
            .memory(s.memory.handle)
            .offset(0)
            .size(vk::WHOLE_SIZE);
        // SAFETY: `s.memory` is the mapped staging allocation.
        if let Err(err) = unsafe { ctx.device.flush_mapped_memory_ranges(&[range]) } {
            ctx.vk_fail("vkFlushMappedMemoryRanges", err);
        }

        let command_buffers = [sb.command_buffer];
        let submit_info = vk::SubmitInfo::default().command_buffers(&command_buffers);
        // SAFETY: the command buffer is ended, the fence is unsignalled
        // (reset by `flush_command_buffer`), and `vg.queue` is the graphics queue.
        if let Err(err) = unsafe {
            ctx.device
                .queue_submit(ctx.vg.queue, &[submit_info], sb.fence)
        } {
            ctx.vk_fail("vkQueueSubmit", err);
        }

        s.buffers[index].submitted = true;
        s.current = (s.current + 1) % NUM_STAGING_BUFFERS;
        g
    }

    /// `R_SubmitStagingBuffers`.
    pub fn submit<E: Engine>(&self, ctx: &mut Ctx<'_, E>) {
        let mut g = self.acquire();
        g = self.wait_no_copies(g);
        for i in 0..NUM_STAGING_BUFFERS {
            let sb = g.buffers[i];
            if !sb.submitted && sb.current_offset > 0 {
                g = self.submit_buffer(ctx, g, i);
            }
        }
        self.release(g);
    }

    /// `R_FlushStagingCommandBuffer`.
    fn flush_command_buffer<E: Engine>(ctx: &mut Ctx<'_, E>, sb: &mut StagingBuffer) {
        if !sb.submitted {
            return;
        }
        // SAFETY: `sb.fence` was signalled by the submission recorded in
        // `submit_buffer`; the command buffer may be re-begun once it has completed.
        unsafe {
            if let Err(err) = ctx.device.wait_for_fences(&[sb.fence], true, u64::MAX) {
                ctx.vk_fail("vkWaitForFences", err);
            }
            if let Err(err) = ctx.device.reset_fences(&[sb.fence]) {
                ctx.vk_fail("vkResetFences", err);
            }
        }
        sb.current_offset = 0;
        sb.submitted = false;
        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        // SAFETY: the command buffer has completed execution (fence waited above).
        if let Err(err) = unsafe {
            ctx.device
                .begin_command_buffer(sb.command_buffer, &begin_info)
        } {
            ctx.vk_fail("vkBeginCommandBuffer", err);
        }
    }

    /// `R_StagingAllocate`: returns with the logical staging lock held; the
    /// caller records into `command_buffer`, then calls [`Staging::begin_copy`].
    pub fn allocate<E: Engine>(
        &self,
        ctx: &mut Ctx<'_, E>,
        size: i32,
        alignment: i32,
    ) -> StagingAllocation {
        let mut g = self.acquire();
        g = self.wait_no_copies(g);
        ctx.vg.device_idle = false;

        if size > ctx.vg.staging_buffer_size {
            // `R_SubmitStagingBuffers` re-takes the lock in C; here the lock is
            // already ours, so submit in place.
            for i in 0..NUM_STAGING_BUFFERS {
                let sb = g.buffers[i];
                if !sb.submitted && sb.current_offset > 0 {
                    g = self.submit_buffer(ctx, g, i);
                }
            }
            for i in 0..NUM_STAGING_BUFFERS {
                Self::flush_command_buffer(ctx, &mut g.buffers[i]);
            }
            ctx.vg.staging_buffer_size = size;
            self.destroy_buffers(ctx, &mut g);
            self.create_buffers(ctx, &mut g);
        }

        assert!(
            alignment > 0 && (alignment & (alignment - 1)) == 0,
            "staging alignment must be a power of two"
        );
        let current = g.current;
        g.buffers[current].current_offset =
            q_align(g.buffers[current].current_offset as u64, alignment as u64) as i32;
        let sb = g.buffers[current];
        if (sb.current_offset + size) >= ctx.vg.staging_buffer_size && !sb.submitted {
            g = self.submit_buffer(ctx, g, current);
        }

        let current = g.current;
        Self::flush_command_buffer(ctx, &mut g.buffers[current]);
        let sb = &mut g.buffers[current];
        let allocation = StagingAllocation {
            data: sb.data.0.wrapping_add(sb.current_offset as usize),
            command_buffer: sb.command_buffer,
            buffer: sb.buffer,
            buffer_offset: sb.current_offset,
        };
        sb.current_offset += size;
        g.num_stagings_in_flight += 1;
        // The logical lock stays held (`g.held` remains true).
        drop(g);
        allocation
    }

    /// `R_StagingBeginCopy`: releases the logical lock taken by [`Staging::allocate`].
    pub fn begin_copy(&self) {
        let g = self.guard();
        debug_assert!(g.held);
        self.release(g);
    }

    /// `R_StagingEndCopy`.
    pub fn end_copy(&self) {
        let mut g = self.acquire();
        g.num_stagings_in_flight -= 1;
        self.release(g);
    }

    /// `R_StagingUploadBuffer`.
    pub fn upload_buffer<E: Engine>(&self, ctx: &mut Ctx<'_, E>, buffer: vk::Buffer, data: &[u8]) {
        let mut remaining = data.len();
        let mut copy_offset = 0usize;
        while remaining > 0 {
            let size = remaining.min(ctx.vg.staging_buffer_size as usize);
            let staging = self.allocate(ctx, size as i32, 1);
            let region = vk::BufferCopy::default()
                .src_offset(staging.buffer_offset as u64)
                .dst_offset(copy_offset as u64)
                .size(size as u64);
            // SAFETY: the staging command buffer is recording and the logical
            // lock is held until `begin_copy`.
            unsafe {
                ctx.device.cmd_copy_buffer(
                    staging.command_buffer,
                    staging.buffer,
                    buffer,
                    &[region],
                )
            };
            self.begin_copy();
            // SAFETY: `staging.data` points at `size` writable bytes of the
            // mapped staging buffer reserved for this allocation.
            unsafe {
                core::ptr::copy_nonoverlapping(data.as_ptr().add(copy_offset), staging.data, size)
            };
            self.end_copy();
            copy_offset += size;
            remaining -= size;
        }
    }
}
