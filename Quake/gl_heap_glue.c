/*
Copyright (C) 2022 Axel Gneiting
Copyright (C) 2026 vkqr-engine contributors

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.

See the GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with this program; if not, write to the Free Software
Foundation, Inc., 59 Temple Place - Suite 330, Boston, MA  02111-1307, USA.

*/
// gl_heap_glue.c -- the C remainder of gl_heap.c under -Duse_rust_render
//
// Compiled instead of gl_heap.c when the Rust suballocator (quake-render via
// quake-capi, Rust migration Phase 8 M3, ADR-015) provides the GL_Heap* ABI.
// Only the _DEBUG `test_gl_heap` command stays here: host.c and host_glue.c
// both register GL_HeapTest_f. The heap is opaque across the ABI, so the two
// consistency checks keep only the parts of the originals that go through
// GL_HeapGetStats; the structural checks live in the Rust unit tests and the
// gl_heap differential (quake-ctest).
#include "quakedef.h"
#include "gl_heap.h"

// The layout the Rust mirrors (rust/quake-types/src/render.rs) assume,
// checked here against the real glquake.h/gl_heap.h/vulkan_core.h; the
// quake-ctest probe sees hand copies of the glquake.h types (ADR-011).
COMPILE_TIME_ASSERT (vk_device_memory, sizeof (VkDeviceMemory) == 8);
COMPILE_TIME_ASSERT (vulkan_memory_type, sizeof (vulkan_memory_type_t) == sizeof (int));
COMPILE_TIME_ASSERT (vulkan_memory_type_none, VULKAN_MEMORY_TYPE_NONE == 0);
COMPILE_TIME_ASSERT (vulkan_memory_type_device, VULKAN_MEMORY_TYPE_DEVICE == 1);
COMPILE_TIME_ASSERT (vulkan_memory_type_host, VULKAN_MEMORY_TYPE_HOST == 2);
COMPILE_TIME_ASSERT (vulkan_memory_size, sizeof (vulkan_memory_t) == 8 + 2 * sizeof (size_t));
COMPILE_TIME_ASSERT (vulkan_memory_handle, offsetof (vulkan_memory_t, handle) == 0);
COMPILE_TIME_ASSERT (vulkan_memory_size_off, offsetof (vulkan_memory_t, size) == 8);
COMPILE_TIME_ASSERT (vulkan_memory_type_off, offsetof (vulkan_memory_t, type) == 8 + sizeof (size_t));
COMPILE_TIME_ASSERT (glheapstats_size, sizeof (glheapstats_t) == 64);
COMPILE_TIME_ASSERT (glheapstats_bytes_allocated, offsetof (glheapstats_t, num_bytes_allocated) == 40);

#ifdef _DEBUG
/*
=================
HEAP_TEST_ASSERT
=================
*/
#define HEAP_TEST_ASSERT(cond, what) \
	if (!(cond))                     \
	{                                \
		Con_Printf ("%s\n", what);   \
		abort ();                    \
	}

/*
=================
TestHeapCleanState
=================
*/
static void TestHeapCleanState (glheap_t *heap)
{
	glheapstats_t *stats = GL_HeapGetStats (heap);
	HEAP_TEST_ASSERT (stats->num_allocations == 0, "Invalid num_allocations counter");
	HEAP_TEST_ASSERT (stats->num_small_allocations == 0, "Invalid num_small_allocations counter");
	HEAP_TEST_ASSERT (stats->num_block_allocations == 0, "Invalid num_block_allocations counter");
	HEAP_TEST_ASSERT (stats->num_dedicated_allocations == 0, "Invalid num_dedicated_allocations counter");
	HEAP_TEST_ASSERT (stats->num_blocks_free == stats->num_segments, "Invalid num_blocks_free counter");
	HEAP_TEST_ASSERT (stats->num_pages_allocated == 0, "num_pages_allocated needs to be 0");
	HEAP_TEST_ASSERT (stats->num_bytes_allocated == 0, "num_bytes_allocated needs to be 0");
}

/*
=================
TestHeapConsistency
=================
*/
static void TestHeapConsistency (glheap_t *heap)
{
	glheapstats_t *stats = GL_HeapGetStats (heap);
	HEAP_TEST_ASSERT (
		stats->num_allocations == (stats->num_small_allocations + stats->num_block_allocations + stats->num_dedicated_allocations), "Invalid alloc counter");
}

/*
=================
GL_HeapTest_f
=================
*/
void GL_HeapTest_f (void)
{
	const VkDeviceSize TEST_HEAP_SIZE = 1ull * 1024ull * 1024ull;
	const VkDeviceSize TEST_HEAP_PAGE_SIZE = 4096;
	const int		   NUM_ITERATIONS = 10000;
	const int		   NUM_ALLOCS_PER_ITERATION = 500;
	const int		   MAX_ALLOC_SIZE = 64ull * 1024ull;
	const int		   MAX_ALIGNMENT_POW2 = 14;

	atomic_uint32_t num_allocations;
	Atomic_StoreUInt32 (&num_allocations, 0);
	glheap_t *test_heap = GL_HeapCreate (TEST_HEAP_SIZE, TEST_HEAP_PAGE_SIZE, 0, VULKAN_MEMORY_TYPE_NONE, false, "Test Heap");
	TestHeapCleanState (test_heap);
	COM_SeedRand (0);
	TEMP_ALLOC_ZEROED (glheapallocation_t *, allocations, NUM_ALLOCS_PER_ITERATION);
	for (int j = 0; j < NUM_ITERATIONS; ++j)
	{
		const int STRIDE = 3;
		for (int k = 0; k <= STRIDE; ++k)
		{
			if (k < STRIDE)
			{
				for (int i = k; i < NUM_ALLOCS_PER_ITERATION; i += STRIDE)
				{
					// Exponential distribution (more small allocations)
					const double	   exponential_dist_size = powf ((double)COM_Rand () / (double)COM_RAND_MAX, 5.0);
					const VkDeviceSize size = (VkDeviceSize)((double)(MAX_ALLOC_SIZE - 1) * exponential_dist_size) + 1;
					const double	   exponential_dist_alignment = powf ((double)COM_Rand () / (double)COM_RAND_MAX, 10.0);
					const VkDeviceSize alignment = 1ull << (uint32_t)(exponential_dist_alignment * (double)MAX_ALIGNMENT_POW2);
					HEAP_TEST_ASSERT (allocations[i] == NULL, "allocation is not NULL");

					glheapallocation_t *allocation = GL_HeapAllocate (test_heap, size, alignment, &num_allocations);
					const VkDeviceSize	offset = GL_HeapGetAllocationOffset (allocation);
					HEAP_TEST_ASSERT ((offset % alignment) == 0, "wrong alignment");
					allocations[i] = allocation;
					TestHeapConsistency (test_heap);
				}
			}
			if (k > 0)
			{
				for (int i = k - 1; i < NUM_ALLOCS_PER_ITERATION; i += STRIDE)
				{
					HEAP_TEST_ASSERT (allocations[i] != NULL, "allocation is NULL");
					GL_HeapFree (test_heap, allocations[i], &num_allocations);
					allocations[i] = NULL;
					TestHeapConsistency (test_heap);
				}
			}
		}
		for (int i = 0; i < NUM_ALLOCS_PER_ITERATION; ++i)
			HEAP_TEST_ASSERT (allocations[i] == NULL, "allocation is not NULL");
		TestHeapCleanState (test_heap);
	}
	TEMP_FREE (allocations);
	{
		glheapallocation_t *large_alloc = GL_HeapAllocate (test_heap, TEST_HEAP_SIZE * 2, 1, &num_allocations);
		TestHeapConsistency (test_heap);
		GL_HeapFree (test_heap, large_alloc, &num_allocations);
		TestHeapConsistency (test_heap);
		TestHeapCleanState (test_heap);
	}
	GL_HeapDestroy (test_heap, &num_allocations);
}
#endif
