//! `gl_heap.c` -- the C ABI of the device-memory suballocator (Rust
//! migration Phase 8 M3, ADR-015). The allocator is `quake_render::heap`;
//! this module adds what C sees: the seven `GL_Heap*` entry points of
//! `gl_heap.h`, the opaque `glheap_t`/`glheapallocation_t` pointers (a
//! boxed `Heap`/`Allocation`), and the memory backend that routes segment
//! and dedicated allocations to the engine's `R_AllocateVulkanMemory`,
//! `R_FreeVulkanMemory` and `GL_SetObjectName`.
//!
//! `GL_HeapAllocate`'s failure exit is `Sys_Error ("GL_HeapAllocate failed
//! to allocate")` (`gl_heap.c`), which terminates rather than longjmping, so
//! no Rust frame is unwound across (ADR-009).

use ash::vk::Handle;
use core::ffi::{c_char, c_int, c_void};
use core::ptr;
use quake_c_sys as c;
use quake_render::heap::{Allocation, DeviceMemoryBackend, Heap};
use quake_types::render::{GlHeapStats, VulkanMemory, VulkanMemoryType};

/// The engine backend: `R_AllocateVulkanMemory` + `GL_SetObjectName` with
/// the heap's own name, `R_FreeVulkanMemory`. `name` is the `const char *`
/// the C caller passed to `GL_HeapCreate` (a string literal at both call
/// sites, `gl_mesh.c:136` and `gl_texmgr.c:587`); the C stores the pointer
/// too.
pub struct CBackend {
    name: *const c_char,
}

// The counter is forwarded to C untouched, never dereferenced here; its
// validity is the `GL_Heap*` callers' contract (see their `# Safety`).
#[allow(clippy::not_unsafe_ptr_arg_deref)]
impl DeviceMemoryBackend for CBackend {
    /// `atomic_uint32_t *num_allocations`, passed straight through.
    type Counter = *mut c_void;

    fn allocate(
        &mut self,
        memory: &mut VulkanMemory,
        size: u64,
        memory_type_index: u32,
        memory_type: VulkanMemoryType,
        device_address: bool,
        counter: *mut c_void,
    ) {
        let mut flags_info = ash::vk::MemoryAllocateFlagsInfo::default()
            .flags(ash::vk::MemoryAllocateFlags::DEVICE_ADDRESS);
        let mut info = ash::vk::MemoryAllocateInfo::default()
            .allocation_size(size)
            .memory_type_index(memory_type_index);
        if device_address {
            info = info.push_next(&mut flags_info);
        }
        // SAFETY: `memory` is a live `vulkan_memory_t` mirror (ADR-011,
        // layout-asserted in quake-types), `info` is a complete
        // `VkMemoryAllocateInfo` whose pNext chain outlives the call, the
        // enum is passed by value and `counter` is whatever the C caller
        // handed `GL_HeapAllocate`/`GL_HeapCreate`, forwarded untouched
        // exactly as gl_heap.c forwards it.
        unsafe {
            c::render::R_AllocateVulkanMemory(
                ptr::from_mut(memory).cast(),
                ptr::from_mut(&mut info).cast(),
                memory_type as c_int,
                counter,
            );
        }
        if memory.handle != ash::vk::DeviceMemory::null() {
            // SAFETY: `name` is the caller's string literal (see the struct
            // doc) and the object type is `VK_OBJECT_TYPE_DEVICE_MEMORY`.
            unsafe {
                c::render::GL_SetObjectName(
                    memory.handle.as_raw(),
                    ash::vk::ObjectType::DEVICE_MEMORY.as_raw(),
                    self.name,
                );
            }
        }
    }

    fn free(&mut self, memory: &mut VulkanMemory, counter: *mut c_void) {
        // SAFETY: as in `allocate`; `memory` was filled by
        // `R_AllocateVulkanMemory` and is freed exactly once.
        unsafe { c::render::R_FreeVulkanMemory(ptr::from_mut(memory).cast(), counter) }
    }
}

/// `glheap_t`
pub type GlHeap = Heap<CBackend>;

/// `gl_heap.h` -- `GL_HeapCreate`. `memory_type` crosses as the C `int`
/// the enum is and is checked here: a value outside the enum is an engine
/// bug the C would carry silently, and the Rust enum must never hold one.
#[no_mangle]
pub extern "C" fn GL_HeapCreate(
    segment_size: u64,
    page_size: u32,
    memory_type_index: u32,
    memory_type: c_int,
    device_address: bool,
    heap_name: *const c_char,
) -> *mut GlHeap {
    let memory_type = match memory_type {
        0 => VulkanMemoryType::None,
        1 => VulkanMemoryType::Device,
        2 => VulkanMemoryType::Host,
        // SAFETY: Sys_Error never returns (ADR-009: it terminates, no longjmp).
        _ => unsafe { c::Sys_Error(c"GL_HeapCreate: bad vulkan_memory_type_t".as_ptr()) },
    };
    Box::into_raw(Box::new(Heap::new(
        CBackend { name: heap_name },
        segment_size,
        page_size,
        memory_type_index,
        memory_type,
        device_address,
    )))
}

/// `gl_heap.h` -- `GL_HeapDestroy`. Releases the segments; like the C, the
/// heap struct itself is never freed (its only caller is the `_DEBUG`
/// self-test, and the C leaks it too).
///
/// # Safety
/// `heap` came from `GL_HeapCreate` and every allocation has been freed.
#[no_mangle]
pub unsafe extern "C" fn GL_HeapDestroy(heap: *mut GlHeap, num_allocations: *mut c_void) {
    // SAFETY: per the contract above.
    unsafe { (*heap).destroy(num_allocations) }
}

/// `gl_heap.h` -- `GL_HeapAllocate`.
///
/// # Safety
/// `heap` came from `GL_HeapCreate`; `num_allocations` is null or a live
/// `atomic_uint32_t`.
#[no_mangle]
pub unsafe extern "C" fn GL_HeapAllocate(
    heap: *mut GlHeap,
    size: u64,
    alignment: u64,
    num_allocations: *mut c_void,
) -> *mut Allocation {
    // The C asserts are `assert ()` (compiled out under NDEBUG); these stay
    // on in every profile. No caller passes 0 (gl_mesh.c/gl_texmgr.c size
    // from Vulkan memory requirements), and the workspace's `panic = "abort"`
    // makes a trip a plain abort rather than an unwind across the ABI.
    assert!(size > 0);
    assert!(alignment > 0);
    // SAFETY: per the contract above.
    match unsafe { (*heap).allocate(size, alignment, num_allocations) } {
        Some(allocation) => Box::into_raw(Box::new(allocation)),
        // SAFETY: the C failure exit, verbatim; Sys_Error never returns.
        None => unsafe { c::Sys_Error(c"GL_HeapAllocate failed to allocate".as_ptr()) },
    }
}

/// `gl_heap.h` -- `GL_HeapFree`.
///
/// # Safety
/// `heap` came from `GL_HeapCreate`, `allocation` from `GL_HeapAllocate` on
/// that heap and has not been freed.
#[no_mangle]
pub unsafe extern "C" fn GL_HeapFree(
    heap: *mut GlHeap,
    allocation: *mut Allocation,
    num_allocations: *mut c_void,
) {
    // SAFETY: per the contract above; the box is reclaimed exactly once.
    unsafe { (*heap).free(*Box::from_raw(allocation), num_allocations) }
}

/// `gl_heap.h` -- `GL_HeapGetAllocationMemory`.
///
/// # Safety
/// `allocation` is live.
#[no_mangle]
pub unsafe extern "C" fn GL_HeapGetAllocationMemory(
    allocation: *mut Allocation,
) -> ash::vk::DeviceMemory {
    // SAFETY: per the contract above.
    unsafe { (*allocation).memory() }
}

/// `gl_heap.h` -- `GL_HeapGetAllocationOffset`.
///
/// # Safety
/// `allocation` is live.
#[no_mangle]
pub unsafe extern "C" fn GL_HeapGetAllocationOffset(allocation: *mut Allocation) -> u64 {
    // SAFETY: per the contract above.
    unsafe { (*allocation).offset() }
}

/// `gl_heap.h` -- `GL_HeapGetStats`. The pointer is into the boxed heap, as
/// stable as the C `&heap->stats`, and derived from the raw heap pointer
/// (not through a `&mut`) so writes through it by C are within its
/// provenance.
///
/// # Safety
/// `heap` came from `GL_HeapCreate`.
#[no_mangle]
pub unsafe extern "C" fn GL_HeapGetStats(heap: *mut GlHeap) -> *mut GlHeapStats {
    // SAFETY: per the contract above.
    unsafe { Heap::stats_ptr(heap) }
}
