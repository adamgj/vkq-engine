/*
Copyright (C) 1996-2001 Id Software, Inc.
Copyright (C) 2002-2009 John Fitzgibbons and others
Copyright (C) 2007-2008 Kristian Duske
Copyright (C) 2010-2014 QuakeSpasm developers
Copyright (C) 2016 Axel Gneiting

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
// r_brush_glue.c: the ray-tracing half of r_brush.c (AS scratch buffer, TLAS
// allocation/garbage and the bmodel BLAS/TLAS builds), kept in C under
// use_rust_render until Phase 8 M10 (plan D5). Lightmaps, indirect draws and
// the bmodel vertex buffer live in quake-capi r_brush.rs, which exports the
// three bmodel_* globals below.

#include "quakedef.h"

extern cvar_t r_rtshadows;

// r_brush.rs
extern VkBuffer		   bmodel_vertex_buffer;
extern uint32_t		   bmodel_numverts;
extern VkDeviceAddress bmodel_vertex_buffer_device_address;

#define TLAS_SIZE_MULTIPLE 1024

VkAccelerationStructureKHR bmodel_tlas = VK_NULL_HANDLE;
static VkBuffer			   bmodel_tlas_buffer;
static size_t			   bmodel_tlas_size;
static vulkan_memory_t	   bmodel_tlas_device_memory;
static uint32_t			   bmodel_tlas_max_instances = TLAS_SIZE_MULTIPLE;
static VkBuffer			   bmodel_indices_buffer;
static VkDeviceAddress	   bmodel_indices_device_address;
static vulkan_memory_t	   bmodel_as_device_memory;

#define TLAS_GARBAGE_FRAME_COUNT 2
static VkAccelerationStructureKHR tlas_garbage[TLAS_GARBAGE_FRAME_COUNT];
static int						  tlas_garbage_index;

#define MIN_SCRATCH_BUFFER_SIZE_MB 8

// Shared scratch buffer for all AS operations (bmodel BLAS build, TLAS build, animated BLAS updates)
dynbuffer_t			   as_scratch_buffer;
static vulkan_memory_t as_scratch_memory;
uint32_t			   as_scratch_buffer_size;

/*
===============
R_EnsureASScratchBufferSize
===============
*/
void R_EnsureASScratchBufferSize (uint32_t required_size)
{
	if (required_size <= as_scratch_buffer_size)
		return;

	if (as_scratch_buffer.buffer != VK_NULL_HANDLE)
		R_AddDynamicBufferGarbage (as_scratch_memory, &as_scratch_buffer, 1, NULL);
	as_scratch_buffer_size = Q_nextPow2 (q_max (required_size, MIN_SCRATCH_BUFFER_SIZE_MB * 1024 * 1024));

	Sys_Printf ("Reallocating dynamic AS scratch buffer (%u KB)\n", as_scratch_buffer_size / 1024);

	VkResult err;

	ZEROED_STRUCT (VkBufferCreateInfo, buffer_create_info);
	buffer_create_info.sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO;
	buffer_create_info.size = as_scratch_buffer_size;
	buffer_create_info.usage = VK_BUFFER_USAGE_STORAGE_BUFFER_BIT | VK_BUFFER_USAGE_ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_BIT_KHR |
							   VK_BUFFER_USAGE_SHADER_DEVICE_ADDRESS_BIT_KHR;

	err = vkCreateBuffer (vulkan_globals.device, &buffer_create_info, NULL, &as_scratch_buffer.buffer);
	if (err != VK_SUCCESS)
		Sys_Error ("vkCreateBuffer failed with code %i", (int)err);
	GL_SetObjectName ((uint64_t)as_scratch_buffer.buffer, VK_OBJECT_TYPE_BUFFER, "AS scratch buffer");

	VkMemoryRequirements memory_requirements;
	vkGetBufferMemoryRequirements (vulkan_globals.device, as_scratch_buffer.buffer, &memory_requirements);

	ZEROED_STRUCT (VkMemoryAllocateFlagsInfo, memory_allocate_flags_info);
	memory_allocate_flags_info.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_FLAGS_INFO_KHR;
	memory_allocate_flags_info.flags = VK_MEMORY_ALLOCATE_DEVICE_ADDRESS_BIT;

	ZEROED_STRUCT (VkMemoryAllocateInfo, memory_allocate_info);
	memory_allocate_info.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;
	memory_allocate_info.pNext = &memory_allocate_flags_info;
	memory_allocate_info.allocationSize = memory_requirements.size;
	memory_allocate_info.memoryTypeIndex = GL_MemoryTypeFromProperties (memory_requirements.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT, 0);

	R_AllocateVulkanMemory (&as_scratch_memory, &memory_allocate_info, VULKAN_MEMORY_TYPE_DEVICE, &num_vulkan_dynbuf_allocations);
	GL_SetObjectName ((uint64_t)as_scratch_memory.handle, VK_OBJECT_TYPE_DEVICE_MEMORY, "AS scratch buffer");

	err = vkBindBufferMemory (vulkan_globals.device, as_scratch_buffer.buffer, as_scratch_memory.handle, 0);
	if (err != VK_SUCCESS)
		Sys_Error ("vkBindBufferMemory failed with code %i", (int)err);

	ZEROED_STRUCT (VkBufferDeviceAddressInfoKHR, buffer_device_address_info);
	buffer_device_address_info.sType = VK_STRUCTURE_TYPE_BUFFER_DEVICE_ADDRESS_INFO_KHR;
	buffer_device_address_info.buffer = as_scratch_buffer.buffer;
	as_scratch_buffer.device_address = vulkan_globals.vk_get_buffer_device_address (vulkan_globals.device, &buffer_device_address_info);
	as_scratch_buffer.current_offset = 0;
}

/*
===============
R_FreeASScratchBuffer
===============
*/
void R_FreeASScratchBuffer (void)
{
	if (as_scratch_buffer.buffer != VK_NULL_HANDLE)
	{
		vkDestroyBuffer (vulkan_globals.device, as_scratch_buffer.buffer, NULL);
		memset (&as_scratch_buffer, 0, sizeof (as_scratch_buffer));
		R_FreeVulkanMemory (&as_scratch_memory, &num_vulkan_dynbuf_allocations);
	}
	as_scratch_buffer_size = 0;
}

/*
===============
R_CollectTLASGarbage
===============
*/
void R_CollectTLASGarbage (void)
{
	tlas_garbage_index = (tlas_garbage_index + 1) % TLAS_GARBAGE_FRAME_COUNT;
	if (tlas_garbage[tlas_garbage_index] != VK_NULL_HANDLE)
	{
		vulkan_globals.vk_destroy_acceleration_structure (vulkan_globals.device, tlas_garbage[tlas_garbage_index], NULL);
		tlas_garbage[tlas_garbage_index] = VK_NULL_HANDLE;
	}
}

/*
===============
R_AllocateTLAS
===============
*/
static void R_AllocateTLAS (void)
{
	VkResult err;

	ZEROED_STRUCT (VkBufferCreateInfo, buffer_create_info);
	buffer_create_info.sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO;
	buffer_create_info.size = bmodel_tlas_size;
	buffer_create_info.usage = VK_BUFFER_USAGE_ACCELERATION_STRUCTURE_STORAGE_BIT_KHR | VK_BUFFER_USAGE_SHADER_DEVICE_ADDRESS_BIT;

	err = vkCreateBuffer (vulkan_globals.device, &buffer_create_info, NULL, &bmodel_tlas_buffer);
	if (err != VK_SUCCESS)
		Sys_Error ("vkCreateBuffer failed with code %i", (int)err);
	GL_SetObjectName ((uint64_t)bmodel_tlas_buffer, VK_OBJECT_TYPE_BUFFER, "BModel TLAS");

	VkMemoryRequirements memory_requirements;
	vkGetBufferMemoryRequirements (vulkan_globals.device, bmodel_tlas_buffer, &memory_requirements);

	ZEROED_STRUCT (VkMemoryAllocateFlagsInfo, memory_allocate_flags_info);
	memory_allocate_flags_info.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_FLAGS_INFO_KHR;
	memory_allocate_flags_info.flags = VK_MEMORY_ALLOCATE_DEVICE_ADDRESS_BIT;

	ZEROED_STRUCT (VkMemoryAllocateInfo, memory_allocate_info);
	memory_allocate_info.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;
	memory_allocate_info.pNext = &memory_allocate_flags_info;
	memory_allocate_info.allocationSize = memory_requirements.size;
	memory_allocate_info.memoryTypeIndex = GL_MemoryTypeFromProperties (memory_requirements.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT, 0);

	R_AllocateVulkanMemory (&bmodel_tlas_device_memory, &memory_allocate_info, VULKAN_MEMORY_TYPE_DEVICE, &num_vulkan_bmodel_allocations);
	GL_SetObjectName ((uint64_t)bmodel_tlas_device_memory.handle, VK_OBJECT_TYPE_DEVICE_MEMORY, "BModel TLAS");

	err = vkBindBufferMemory (vulkan_globals.device, bmodel_tlas_buffer, bmodel_tlas_device_memory.handle, 0);
	if (err != VK_SUCCESS)
		Sys_Error ("vkBindBufferMemory failed with code %i", (int)err);

	ZEROED_STRUCT (VkAccelerationStructureCreateInfoKHR, acceleration_structure_create_info);
	acceleration_structure_create_info.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_CREATE_INFO_KHR;
	acceleration_structure_create_info.buffer = bmodel_tlas_buffer;
	acceleration_structure_create_info.size = bmodel_tlas_size;
	acceleration_structure_create_info.type = VK_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL_KHR;
	err = vulkan_globals.vk_create_acceleration_structure (vulkan_globals.device, &acceleration_structure_create_info, NULL, &bmodel_tlas);
	if (err != VK_SUCCESS)
		Sys_Error ("vkCreateAccelerationStructure failed with code %i", (int)err);
}

/*
==================
GL_DeleteBModelAccelerationStructures
==================
*/
void GL_DeleteBModelAccelerationStructures (void)
{
	if (bmodel_tlas == VK_NULL_HANDLE)
		return;

	GL_WaitForDeviceIdle ();
	TEMP_ALLOC (VkBuffer, buffers, 1 + MAX_MODELS);
	int num_buffers = 0;
	buffers[num_buffers++] = bmodel_indices_buffer;
	for (int i = 0; i < MAX_MODELS; ++i)
	{
		qmodel_t *m = cl.model_precache[i];
		if (!m)
			continue;
		if (m->blas != VK_NULL_HANDLE)
		{
			vulkan_globals.vk_destroy_acceleration_structure (vulkan_globals.device, cl.model_precache[i]->blas, NULL);
			buffers[num_buffers++] = m->buffer;
			m->blas = VK_NULL_HANDLE;
			m->buffer = VK_NULL_HANDLE;
			m->address = 0;
		}
		assert (m->buffer == VK_NULL_HANDLE);
		assert (m->address == 0);
	}
	R_FreeBuffers (num_buffers, buffers, &bmodel_as_device_memory, &num_vulkan_bmodel_allocations);

	vulkan_globals.vk_destroy_acceleration_structure (vulkan_globals.device, bmodel_tlas, NULL);
	vkDestroyBuffer (vulkan_globals.device, bmodel_tlas_buffer, NULL);
	R_FreeVulkanMemory (&bmodel_tlas_device_memory, &num_vulkan_bmodel_allocations);

	bmodel_tlas = VK_NULL_HANDLE;
	bmodel_tlas_buffer = VK_NULL_HANDLE;
	bmodel_tlas_size = 0;
	bmodel_indices_buffer = VK_NULL_HANDLE;
	bmodel_indices_device_address = 0;
	TEMP_FREE (buffers);
}

/*
==================
GL_BuildBModelAccelerationStructures
==================
*/
void GL_BuildBModelAccelerationStructures (void)
{
	VkResult err;

	if (!vulkan_globals.ray_query || !r_rtshadows.value || (bmodel_tlas != VK_NULL_HANDLE))
		return;

	// count all tris in all models
	uint32_t total_num_triangles = 0;
	TEMP_ALLOC_ZEROED (uint32_t, blas_num_tris, MAX_MODELS);
	TEMP_ALLOC_ZEROED (VkAccelerationStructureGeometryKHR, blas_geometries, MAX_MODELS);
	TEMP_ALLOC_ZEROED (VkAccelerationStructureBuildGeometryInfoKHR, blas_geometry_infos, MAX_MODELS);
	TEMP_ALLOC_ZEROED (VkAccelerationStructureBuildSizesInfoKHR, blas_sizes_infos, MAX_MODELS);
	TEMP_ALLOC_ZEROED (qmodel_t *, blas_models, MAX_MODELS);
	TEMP_ALLOC_ZEROED (buffer_create_info_t, buffer_create_infos, 1 + MAX_MODELS);

	size_t scratch_buffer_size = 0;
	int	   num_blas = 0;
	for (int j = 1; j < MAX_MODELS; j++)
	{
		qmodel_t *m = cl.model_precache[j];
		if (!m || m->type != mod_brush)
			continue;
		if (m->flags & MF_HOLEY)
			continue;

		for (int i = m->firstmodelsurface; i < m->firstmodelsurface + m->nummodelsurfaces; i++)
		{
			msurface_t *s = &m->surfaces[i];
			if ((s->flags & ~SURF_PLANEBACK) != 0)
				continue;
			total_num_triangles += m->surfaces[i].numedges - 2;
			blas_num_tris[num_blas] += m->surfaces[i].numedges - 2;
		}
		if (blas_num_tris[num_blas] == 0)
			continue;

		blas_models[num_blas] = m;

		VkAccelerationStructureGeometryKHR *blas_geometry = &blas_geometries[num_blas];
		blas_geometry->sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_KHR;
		blas_geometry->geometryType = VK_GEOMETRY_TYPE_TRIANGLES_KHR;
		blas_geometry->geometry.triangles.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_TRIANGLES_DATA_KHR;
		blas_geometry->geometry.triangles.vertexFormat = VK_FORMAT_R32G32B32_SFLOAT;
		blas_geometry->geometry.triangles.vertexStride = 28;
		blas_geometry->geometry.triangles.maxVertex = bmodel_numverts;
		blas_geometry->geometry.triangles.indexType = VK_INDEX_TYPE_UINT32;

		VkAccelerationStructureBuildGeometryInfoKHR *blas_geometry_info = &blas_geometry_infos[num_blas];
		blas_geometry_info->sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_BUILD_GEOMETRY_INFO_KHR;
		blas_geometry_info->type = VK_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL_KHR;
		blas_geometry_info->flags = VK_BUILD_ACCELERATION_STRUCTURE_PREFER_FAST_TRACE_BIT_KHR;
		blas_geometry_info->mode = VK_BUILD_ACCELERATION_STRUCTURE_MODE_BUILD_KHR;
		blas_geometry_info->geometryCount = 1;
		blas_geometry_info->pGeometries = blas_geometry;

		VkAccelerationStructureBuildSizesInfoKHR *blas_build_sizes_info = &blas_sizes_infos[num_blas];
		blas_build_sizes_info->sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_BUILD_SIZES_INFO_KHR;
		vulkan_globals.vk_get_acceleration_structure_build_sizes (
			vulkan_globals.device, VK_ACCELERATION_STRUCTURE_BUILD_TYPE_DEVICE_KHR, blas_geometry_info, &blas_num_tris[num_blas], blas_build_sizes_info);

		scratch_buffer_size = q_max (scratch_buffer_size, blas_build_sizes_info->buildScratchSize);
		++num_blas;
	}

	if (num_blas == 0)
	{
		TEMP_FREE (blas_num_tris);
		TEMP_FREE (blas_geometries);
		TEMP_FREE (blas_geometry_infos);
		TEMP_FREE (blas_sizes_infos);
		TEMP_FREE (blas_models);
		TEMP_FREE (buffer_create_infos);
		return;
	}

	// Query TLAS sizes for initial instance count
	{
		ZEROED_STRUCT (VkAccelerationStructureGeometryKHR, tlas_geometry);
		tlas_geometry.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_KHR;
		tlas_geometry.geometryType = VK_GEOMETRY_TYPE_INSTANCES_KHR;
		tlas_geometry.geometry.instances.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_INSTANCES_DATA_KHR;

		ZEROED_STRUCT (VkAccelerationStructureBuildGeometryInfoKHR, tlas_geometry_info);
		tlas_geometry_info.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_BUILD_GEOMETRY_INFO_KHR;
		tlas_geometry_info.type = VK_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL_KHR;
		tlas_geometry_info.flags = VK_BUILD_ACCELERATION_STRUCTURE_PREFER_FAST_TRACE_BIT_KHR;
		tlas_geometry_info.mode = VK_BUILD_ACCELERATION_STRUCTURE_MODE_BUILD_KHR;
		tlas_geometry_info.geometryCount = 1;
		tlas_geometry_info.pGeometries = &tlas_geometry;

		ZEROED_STRUCT (VkAccelerationStructureBuildSizesInfoKHR, tlas_build_sizes_info);
		tlas_build_sizes_info.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_BUILD_SIZES_INFO_KHR;
		vulkan_globals.vk_get_acceleration_structure_build_sizes (
			vulkan_globals.device, VK_ACCELERATION_STRUCTURE_BUILD_TYPE_DEVICE_KHR, &tlas_geometry_info, &bmodel_tlas_max_instances, &tlas_build_sizes_info);

		scratch_buffer_size = q_max (scratch_buffer_size, tlas_build_sizes_info.buildScratchSize);
		bmodel_tlas_size = tlas_build_sizes_info.accelerationStructureSize;
	}

	const size_t indices_size = total_num_triangles * 3 * sizeof (uint32_t);

	buffer_create_infos[0].buffer = &bmodel_indices_buffer;
	buffer_create_infos[0].size = indices_size;
	buffer_create_infos[0].usage = VK_BUFFER_USAGE_ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_BIT_KHR | VK_BUFFER_USAGE_TRANSFER_DST_BIT;
	buffer_create_infos[0].address = &bmodel_indices_device_address;
	buffer_create_infos[0].name = "BModel indices";

	for (int i = 0; i < num_blas; ++i)
	{
		buffer_create_info_t *create_info = &buffer_create_infos[1 + i];
		create_info->buffer = &blas_models[i]->buffer;
		create_info->size = blas_sizes_infos[i].accelerationStructureSize;
		create_info->usage = VK_BUFFER_USAGE_ACCELERATION_STRUCTURE_STORAGE_BIT_KHR;
		create_info->address = &blas_models[i]->address;
		create_info->name = "BModel BLAS";
	}

	const size_t total_as_device_size = R_CreateBuffers (
		1 + num_blas, buffer_create_infos, &bmodel_as_device_memory, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT, 0, &num_vulkan_bmodel_allocations, "BModel AS");

	Sys_Printf ("Allocating acceleration structure data (%u KB)\n", (int)(total_as_device_size / 1024ull));

	R_AllocateTLAS ();

	R_EnsureASScratchBufferSize (scratch_buffer_size);

	VkBuffer		staging_buffer;
	VkCommandBuffer command_buffer;
	int				staging_offset;
	unsigned char  *staging_memory = R_StagingAllocate (indices_size, 1, &command_buffer, &staging_buffer, &staging_offset);

	{
		ZEROED_STRUCT (VkBufferCopy, region);
		region.srcOffset = staging_offset;
		region.dstOffset = 0;
		region.size = indices_size;
		vkCmdCopyBuffer (command_buffer, staging_buffer, bmodel_indices_buffer, 1, &region);

		ZEROED_STRUCT (VkMemoryBarrier, memory_barrier);
		memory_barrier.sType = VK_STRUCTURE_TYPE_MEMORY_BARRIER;
		memory_barrier.srcAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT;
		memory_barrier.dstAccessMask = VK_ACCESS_SHADER_READ_BIT;
		vulkan_globals.vk_cmd_pipeline_barrier (
			command_buffer, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR, 0, 1, &memory_barrier, 0, NULL, 0, NULL);
	}

	size_t scratch_offset = 0;
	size_t indices_offsets = 0;
	for (int i = 0; i < num_blas; ++i)
	{
		scratch_offset =
			q_align (scratch_offset, vulkan_globals.physical_device_acceleration_structure_properties.minAccelerationStructureScratchOffsetAlignment);

		if ((scratch_offset + blas_sizes_infos[i].buildScratchSize) > scratch_buffer_size)
		{
			ZEROED_STRUCT (VkMemoryBarrier, memory_barrier);
			memory_barrier.sType = VK_STRUCTURE_TYPE_MEMORY_BARRIER;
			memory_barrier.srcAccessMask = VK_ACCESS_ACCELERATION_STRUCTURE_READ_BIT_KHR | VK_ACCESS_ACCELERATION_STRUCTURE_WRITE_BIT_KHR;
			memory_barrier.dstAccessMask = VK_ACCESS_ACCELERATION_STRUCTURE_READ_BIT_KHR | VK_ACCESS_ACCELERATION_STRUCTURE_WRITE_BIT_KHR;
			vulkan_globals.vk_cmd_pipeline_barrier (
				command_buffer, VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR, VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR, 0, 1,
				&memory_barrier, 0, NULL, 0, NULL);
			scratch_offset = 0;
		}

		ZEROED_STRUCT (VkAccelerationStructureCreateInfoKHR, acceleration_structure_create_info);
		acceleration_structure_create_info.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_CREATE_INFO_KHR;
		acceleration_structure_create_info.buffer = blas_models[i]->buffer;
		acceleration_structure_create_info.size = blas_sizes_infos[i].accelerationStructureSize;
		acceleration_structure_create_info.type = VK_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL_KHR;
		err = vulkan_globals.vk_create_acceleration_structure (vulkan_globals.device, &acceleration_structure_create_info, NULL, &blas_models[i]->blas);
		if (err != VK_SUCCESS)
			Sys_Error ("vkCreateAccelerationStructure failed with code %i", (int)err);

		ZEROED_STRUCT (VkAccelerationStructureBuildRangeInfoKHR, build_range_info);
		build_range_info.primitiveCount = blas_num_tris[i];
		VkAccelerationStructureBuildGeometryInfoKHR *blas_geometry_info = &blas_geometry_infos[i];
		blas_geometry_info->dstAccelerationStructure = blas_models[i]->blas;
		blas_geometry_info->scratchData.deviceAddress = as_scratch_buffer.device_address + scratch_offset;
		VkAccelerationStructureGeometryKHR *blas_geometry = &blas_geometries[i];
		blas_geometry->geometry.triangles.vertexData.deviceAddress = bmodel_vertex_buffer_device_address;
		blas_geometry->geometry.triangles.indexData.deviceAddress = bmodel_indices_device_address + indices_offsets;
		const VkAccelerationStructureBuildRangeInfoKHR *build_range_info_ptr = &build_range_info;
		vulkan_globals.vk_cmd_build_acceleration_structures (command_buffer, 1, blas_geometry_info, &build_range_info_ptr);

		scratch_offset += blas_sizes_infos[i].buildScratchSize;
		indices_offsets += blas_num_tris[i] * 3 * sizeof (uint32_t);
	}

	{
		ZEROED_STRUCT (VkMemoryBarrier, memory_barrier);
		memory_barrier.sType = VK_STRUCTURE_TYPE_MEMORY_BARRIER;
		memory_barrier.srcAccessMask = VK_ACCESS_ACCELERATION_STRUCTURE_WRITE_BIT_KHR;
		memory_barrier.dstAccessMask = VK_ACCESS_ACCELERATION_STRUCTURE_READ_BIT_KHR;
		vulkan_globals.vk_cmd_pipeline_barrier (
			command_buffer, VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR, VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR, 0, 1,
			&memory_barrier, 0, NULL, 0, NULL);
	}

	TEMP_FREE (blas_num_tris);
	TEMP_FREE (blas_geometries);
	TEMP_FREE (blas_geometry_infos);
	TEMP_FREE (blas_sizes_infos);
	TEMP_FREE (blas_models);
	TEMP_FREE (buffer_create_infos);

	uint32_t *indices = (uint32_t *)staging_memory;
	uint32_t  current_index = 0;
	R_StagingBeginCopy ();

	for (int j = 1; j < MAX_MODELS; j++)
	{
		qmodel_t *m = cl.model_precache[j];
		if (!m || m->type != mod_brush)
			continue;
		if (m->flags & MF_HOLEY)
			continue;

		for (int i = m->firstmodelsurface; i < m->firstmodelsurface + m->nummodelsurfaces; i++)
		{
			msurface_t *s = &m->surfaces[i];
			if ((s->flags & ~SURF_PLANEBACK) != 0)
				continue;

			for (int k = 2; k < s->numedges; ++k)
			{
				indices[current_index++] = s->vbo_firstvert;
				indices[current_index++] = s->vbo_firstvert + k - 1;
				indices[current_index++] = s->vbo_firstvert + k;
			}
		}
	}
	R_StagingEndCopy ();
}

/*
=============
R_BuildTopLevelAccelerationStructure
=============
*/
void R_BuildTopLevelAccelerationStructure (void *unused)
{
	if (bmodel_tlas == VK_NULL_HANDLE)
		return;

	cb_context_t *cbx = &vulkan_globals.primary_cb_contexts[PCBX_BUILD_ACCELERATION_STRUCTURES];

	// Update animated entity BLASes first
	R_UpdateAnimatedBLASes (cbx);

	R_BeginDebugUtilsLabel (cbx, "Build TLAS");

	int num_instances = 0;
	for (int i = 0; i < cl.num_entities + cl.num_statics; ++i)
	{
		entity_t *e = (i < cl.num_entities) ? &cl.entities[i] : cl.static_entities[i - cl.num_entities];
		if (!e->model || e->model->needload)
			continue;
		if ((e->alpha != ENTALPHA_DEFAULT) && (ENTALPHA_DECODE (e->alpha) < 1.0f))
			continue;

		// Brush models use model BLAS, alias models use entity BLAS
		if (e->model->type == mod_brush && e->model->blas != VK_NULL_HANDLE)
			++num_instances;
		else if (
			e->model->type == mod_alias && e->blas_data && e->blas_data->blas != VK_NULL_HANDLE && !e->blas_data->needs_initial_build &&
			e->blas_data->model == e->model)
			++num_instances;
	}

	VkDeviceAddress						instances_device_address;
	VkAccelerationStructureInstanceKHR *instances = (VkAccelerationStructureInstanceKHR *)R_StorageAllocate (
		num_instances * sizeof (VkAccelerationStructureInstanceKHR), NULL, NULL, &instances_device_address);

	num_instances = 0;
	for (int i = 0; i < cl.num_entities + cl.num_statics; ++i)
	{
		entity_t *e = (i < cl.num_entities) ? &cl.entities[i] : cl.static_entities[i - cl.num_entities];
		if (!e->model || e->model->needload)
			continue;
		if ((e->alpha != ENTALPHA_DEFAULT) && (ENTALPHA_DECODE (e->alpha) < 1.0f))
			continue;

		VkDeviceAddress address = 0;
		qboolean		is_alias = false;

		if (e->model->type == mod_brush && e->model->blas != VK_NULL_HANDLE)
		{
			address = e->model->address;
		}
		else if (
			e->model->type == mod_alias && e->blas_data && e->blas_data->blas != VK_NULL_HANDLE && !e->blas_data->needs_initial_build &&
			e->blas_data->model == e->model)
		{
			address = e->blas_data->address;
			is_alias = true;
		}
		else
		{
			continue;
		}

		vec3_t lerped_origin, lerped_angles;
		if (is_alias)
			R_GetEntityLerpedTransform (e, lerped_origin, lerped_angles);
		else
		{
			VectorCopy (e->origin, lerped_origin);
			VectorCopy (e->angles, lerped_angles);
		}
		lerped_angles[0] = -lerped_angles[0]; // quake bug

		float model_matrix[16];
		IdentityMatrix (model_matrix);
		if (e->model != cl.worldmodel)
			R_RotateForEntity (model_matrix, lerped_origin, lerped_angles, e->netstate.scale);

		// For alias models, apply scale_origin translation and scale
		if (is_alias)
		{
			aliashdr_t *hdr = (aliashdr_t *)Mod_Extradata (e->model);
			if (hdr)
			{
				float translation_matrix[16];
				TranslationMatrix (translation_matrix, hdr->scale_origin[0], hdr->scale_origin[1], hdr->scale_origin[2]);
				MatrixMultiply (model_matrix, translation_matrix);

				float scale_matrix[16];
				ScaleMatrix (scale_matrix, hdr->scale[0], hdr->scale[1], hdr->scale[2]);
				MatrixMultiply (model_matrix, scale_matrix);
			}
		}

		VkAccelerationStructureInstanceKHR *instance = &instances[num_instances];
		instance->transform.matrix[0][0] = model_matrix[0];
		instance->transform.matrix[0][1] = model_matrix[4];
		instance->transform.matrix[0][2] = model_matrix[8];
		instance->transform.matrix[0][3] = model_matrix[12];
		instance->transform.matrix[1][0] = model_matrix[1];
		instance->transform.matrix[1][1] = model_matrix[5];
		instance->transform.matrix[1][2] = model_matrix[9];
		instance->transform.matrix[1][3] = model_matrix[13];
		instance->transform.matrix[2][0] = model_matrix[2];
		instance->transform.matrix[2][1] = model_matrix[6];
		instance->transform.matrix[2][2] = model_matrix[10];
		instance->transform.matrix[2][3] = model_matrix[14];
		instance->instanceCustomIndex = 0;
		instance->mask = 0xFF;
		instance->instanceShaderBindingTableRecordOffset = 0;
		instance->flags = VK_GEOMETRY_INSTANCE_FORCE_OPAQUE_BIT_KHR | VK_GEOMETRY_INSTANCE_TRIANGLE_FACING_CULL_DISABLE_BIT_KHR;
		instance->accelerationStructureReference = address;

		++num_instances;
	}

	ZEROED_STRUCT (VkAccelerationStructureGeometryKHR, tlas_geometry);
	tlas_geometry.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_KHR;
	tlas_geometry.geometryType = VK_GEOMETRY_TYPE_INSTANCES_KHR;
	tlas_geometry.geometry.instances.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_INSTANCES_DATA_KHR;
	tlas_geometry.geometry.instances.data.deviceAddress = instances_device_address;

	ZEROED_STRUCT (VkAccelerationStructureBuildGeometryInfoKHR, tlas_geometry_info);
	tlas_geometry_info.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_BUILD_GEOMETRY_INFO_KHR;
	tlas_geometry_info.type = VK_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL_KHR;
	tlas_geometry_info.flags = VK_BUILD_ACCELERATION_STRUCTURE_PREFER_FAST_TRACE_BIT_KHR;
	tlas_geometry_info.mode = VK_BUILD_ACCELERATION_STRUCTURE_MODE_BUILD_KHR;
	tlas_geometry_info.geometryCount = 1;
	tlas_geometry_info.pGeometries = &tlas_geometry;

	// Resize TLAS if instance count exceeds current capacity
	if ((uint32_t)num_instances > bmodel_tlas_max_instances)
	{
		tlas_garbage[tlas_garbage_index] = bmodel_tlas;
		dynbuffer_t tlas_dynbuf;
		memset (&tlas_dynbuf, 0, sizeof (tlas_dynbuf));
		tlas_dynbuf.buffer = bmodel_tlas_buffer;
		R_AddDynamicBufferGarbage (bmodel_tlas_device_memory, &tlas_dynbuf, 1, NULL);

		bmodel_tlas_max_instances = ((num_instances / TLAS_SIZE_MULTIPLE) + 1) * TLAS_SIZE_MULTIPLE;

		ZEROED_STRUCT (VkAccelerationStructureBuildSizesInfoKHR, new_tlas_sizes);
		new_tlas_sizes.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_BUILD_SIZES_INFO_KHR;
		vulkan_globals.vk_get_acceleration_structure_build_sizes (
			vulkan_globals.device, VK_ACCELERATION_STRUCTURE_BUILD_TYPE_DEVICE_KHR, &tlas_geometry_info, &bmodel_tlas_max_instances, &new_tlas_sizes);
		bmodel_tlas_size = new_tlas_sizes.accelerationStructureSize;

		Sys_Printf ("Reallocating TLAS for %u instances (%u KB)\n", bmodel_tlas_max_instances, (uint32_t)(bmodel_tlas_size / 1024));
		R_AllocateTLAS ();
	}

	const uint32_t							 tlas_num_instances = num_instances;
	VkAccelerationStructureBuildSizesInfoKHR tlas_sizes;
	tlas_sizes.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_BUILD_SIZES_INFO_KHR;
	tlas_sizes.pNext = NULL;
	vulkan_globals.vk_get_acceleration_structure_build_sizes (
		vulkan_globals.device, VK_ACCELERATION_STRUCTURE_BUILD_TYPE_DEVICE_KHR, &tlas_geometry_info, &tlas_num_instances, &tlas_sizes);

	R_EnsureASScratchBufferSize (tlas_sizes.buildScratchSize);

	tlas_geometry_info.dstAccelerationStructure = bmodel_tlas;
	tlas_geometry_info.scratchData.deviceAddress = as_scratch_buffer.device_address;

	ZEROED_STRUCT (VkAccelerationStructureBuildRangeInfoKHR, build_range_info);
	build_range_info.primitiveCount = num_instances;
	const VkAccelerationStructureBuildRangeInfoKHR *build_range_info_ptr = &build_range_info;
	vulkan_globals.vk_cmd_build_acceleration_structures (cbx->cb, 1, &tlas_geometry_info, &build_range_info_ptr);

	ZEROED_STRUCT (VkMemoryBarrier, memory_barrier);
	memory_barrier.sType = VK_STRUCTURE_TYPE_MEMORY_BARRIER;
	memory_barrier.srcAccessMask = VK_ACCESS_ACCELERATION_STRUCTURE_WRITE_BIT_KHR;
	memory_barrier.dstAccessMask = VK_ACCESS_ACCELERATION_STRUCTURE_READ_BIT_KHR;
	vulkan_globals.vk_cmd_pipeline_barrier (
		cbx->cb, VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR, VK_PIPELINE_STAGE_ALL_COMMANDS_BIT, 0, 1, &memory_barrier, 0, NULL, 0, NULL);

	R_EndDebugUtilsLabel (cbx);
}
