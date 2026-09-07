/* Phase 8 M3 oracle TU for Quake/gl_heap.c -- the device-memory
 * suballocator (ADR-015; tests/gl_heap_differential.rs).
 *
 * gl_heap.c is composed here rather than listed in build.rs's C_SOURCES for
 * the stubs/r_part_ref.c reason: its three seams -- R_AllocateVulkanMemory,
 * R_FreeVulkanMemory and GL_SetObjectName -- are already defined as plain
 * aborting doubles by stubs/r_part_fte_ref.c:1657-1668 and
 * stubs/r_part_ref.c:331, and the heap needs *working* fakes of them. The
 * renames below are therefore TU-local: the seven GL_Heap* entry points
 * become c_ref_GL_Heap*, and the seams become c_ref_heap_* fakes that hand
 * out sequential handles. Handles are deterministic on both sides, so the
 * differential can compare the memory a suballocation landed in (segment
 * identity and dedicated-ness) as well as its offset.
 *
 * _DEBUG is undefined for this TU: gl_heap.c's `test_gl_heap` command needs
 * TEMP_ALLOC_ZEROED/COM_Rand/Atomic_StoreUInt32 and is not what this oracle
 * is for (the differential drives the public API with proptest traces). */
#undef _DEBUG

#include "quakedef.h"

#define GL_HeapCreate			   c_ref_GL_HeapCreate
#define GL_HeapDestroy			   c_ref_GL_HeapDestroy
#define GL_HeapAllocate			   c_ref_GL_HeapAllocate
#define GL_HeapFree				   c_ref_GL_HeapFree
#define GL_HeapGetAllocationMemory c_ref_GL_HeapGetAllocationMemory
#define GL_HeapGetAllocationOffset c_ref_GL_HeapGetAllocationOffset
#define GL_HeapGetStats			   c_ref_GL_HeapGetStats
#define R_AllocateVulkanMemory	   c_ref_heap_R_AllocateVulkanMemory
#define R_FreeVulkanMemory		   c_ref_heap_R_FreeVulkanMemory
#define GL_SetObjectName		   c_ref_heap_GL_SetObjectName

uint64_t c_ref_heap_next_handle;
uint32_t c_ref_heap_num_live;
uint32_t c_ref_heap_num_device_address_allocs;
uint32_t c_ref_heap_num_named;

/* gl_rmisc.c:4601's shape: type recorded, size recorded, counter bumped when
 * given; the handle is the next value of a per-process sequence instead of a
 * VkDeviceMemory from a driver. */
void R_AllocateVulkanMemory (vulkan_memory_t *memory, VkMemoryAllocateInfo *memory_allocate_info, vulkan_memory_type_t type, atomic_uint32_t *num_allocations)
{
	memory->type = type;
	memory->size = (size_t)memory_allocate_info->allocationSize;
	memory->handle = (VkDeviceMemory)(uintptr_t)(++c_ref_heap_next_handle);
	c_ref_heap_num_live++;
	if (memory_allocate_info->pNext != NULL)
	{
		const VkMemoryAllocateFlagsInfo *flags = (const VkMemoryAllocateFlagsInfo *)memory_allocate_info->pNext;
		if (flags->sType == VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_FLAGS_INFO && (flags->flags & VK_MEMORY_ALLOCATE_DEVICE_ADDRESS_BIT))
			c_ref_heap_num_device_address_allocs++;
	}
	if (num_allocations)
		num_allocations->value++;
}

void R_FreeVulkanMemory (vulkan_memory_t *memory, atomic_uint32_t *num_allocations)
{
	memory->handle = VK_NULL_HANDLE;
	memory->size = 0;
	memory->type = VULKAN_MEMORY_TYPE_NONE;
	c_ref_heap_num_live--;
	if (num_allocations)
		num_allocations->value--;
}

void GL_SetObjectName (uint64_t object, VkObjectType object_type, const char *name)
{
	(void)object, (void)object_type, (void)name;
	c_ref_heap_num_named++;
}

#include "gl_heap.c"
