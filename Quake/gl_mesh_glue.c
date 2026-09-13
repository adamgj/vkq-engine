/*
Copyright (C) 1996-2001 Id Software, Inc.
Copyright (C) 2002-2009 John Fitzgibbons and others
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
// gl_mesh_glue.c: the ray-tracing half of gl_mesh.c (BLAS allocation and
// animated BLAS rebuilds), kept in C under use_rust_render until Phase 8 M10
// (plan D5). The display-list/mesh-buffer half lives in quake-capi gl_mesh.rs.

#include "quakedef.h"
#include "gl_heap.h"

extern cvar_t r_rtshadows;

// gl_mesh.rs
extern glheap_t *mesh_buffer_heap;
void			 GLMesh_Glue_AddBLASGarbage (VkAccelerationStructureKHR blas, VkBuffer buffer, glheapallocation_t *allocation);

/*
================
R_AllocateEntityBLAS

Allocate acceleration structure for an animated entity model.
Handles MDL (PV_QUAKE1), MD3 (PV_QUAKE3), and MD5 (PV_MD5) models.
================
*/
void R_AllocateEntityBLAS (entity_t *e)
{
	if (!vulkan_globals.ray_query || r_rtshadows.value <= 0)
		return;
	if (!e->model || e->model->type != mod_alias)
		return;
	if (e->model->flags & EF_ROCKET)
		return;

	aliashdr_t *hdr = (aliashdr_t *)Mod_Extradata (e->model);
	if (!hdr)
		return;

	// TODO: handle multi-surface models (nextsurface chain)
	const uint32_t num_triangles = hdr->numtris;
	if (num_triangles == 0)
		return;

	// Check if the entity switched models; enhanced model reloads free all entity BLASes explicitly.
	if (e->blas_data && (e->blas_data->model != e->model))
		R_FreeEntityBLAS (e);

	if (e->blas_data)
		return;

	// Allocate BLAS data struct (after validation to avoid alloc/free cycles)
	e->blas_data = Mem_Alloc (sizeof (entity_blas_t));
	memset (e->blas_data, 0, sizeof (entity_blas_t));
	e->blas_data->needs_initial_build = true;

	// Set up geometry info for size query
	// Vertex positions will be computed into scratch memory as vec3 floats
	ZEROED_STRUCT (VkAccelerationStructureGeometryKHR, blas_geometry);
	blas_geometry.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_KHR;
	blas_geometry.geometryType = VK_GEOMETRY_TYPE_TRIANGLES_KHR;
	blas_geometry.geometry.triangles.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_TRIANGLES_DATA_KHR;
	blas_geometry.geometry.triangles.vertexFormat = VK_FORMAT_R32G32B32_SFLOAT;
	blas_geometry.geometry.triangles.vertexStride = sizeof (float) * 3;
	blas_geometry.geometry.triangles.maxVertex = hdr->numverts_vbo;
	blas_geometry.geometry.triangles.indexType = VK_INDEX_TYPE_UINT16;

	ZEROED_STRUCT (VkAccelerationStructureBuildGeometryInfoKHR, blas_geometry_info);
	blas_geometry_info.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_BUILD_GEOMETRY_INFO_KHR;
	blas_geometry_info.type = VK_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL_KHR;
	blas_geometry_info.flags = VK_BUILD_ACCELERATION_STRUCTURE_PREFER_FAST_BUILD_BIT_KHR | VK_BUILD_ACCELERATION_STRUCTURE_ALLOW_UPDATE_BIT_KHR;
	blas_geometry_info.mode = VK_BUILD_ACCELERATION_STRUCTURE_MODE_BUILD_KHR;
	blas_geometry_info.geometryCount = 1;
	blas_geometry_info.pGeometries = &blas_geometry;

	// Query acceleration structure size
	ZEROED_STRUCT (VkAccelerationStructureBuildSizesInfoKHR, blas_sizes_info);
	blas_sizes_info.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_BUILD_SIZES_INFO_KHR;
	vulkan_globals.vk_get_acceleration_structure_build_sizes (
		vulkan_globals.device, VK_ACCELERATION_STRUCTURE_BUILD_TYPE_DEVICE_KHR, &blas_geometry_info, &num_triangles, &blas_sizes_info);

	// Create buffer for BLAS
	ZEROED_STRUCT (VkBufferCreateInfo, buffer_create_info);
	buffer_create_info.sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO;
	buffer_create_info.size = blas_sizes_info.accelerationStructureSize;
	buffer_create_info.usage = VK_BUFFER_USAGE_ACCELERATION_STRUCTURE_STORAGE_BIT_KHR | VK_BUFFER_USAGE_SHADER_DEVICE_ADDRESS_BIT;

	VkResult err = vkCreateBuffer (vulkan_globals.device, &buffer_create_info, NULL, &e->blas_data->buffer);
	if (err != VK_SUCCESS)
		Sys_Error ("vkCreateBuffer failed for entity BLAS with code %i", (int)err);

	// Allocate from mesh heap
	VkMemoryRequirements memory_requirements;
	vkGetBufferMemoryRequirements (vulkan_globals.device, e->blas_data->buffer, &memory_requirements);

	e->blas_data->allocation = GL_HeapAllocate (mesh_buffer_heap, memory_requirements.size, memory_requirements.alignment, &num_vulkan_mesh_allocations);
	err = vkBindBufferMemory (
		vulkan_globals.device, e->blas_data->buffer, GL_HeapGetAllocationMemory (e->blas_data->allocation),
		GL_HeapGetAllocationOffset (e->blas_data->allocation));
	if (err != VK_SUCCESS)
		Sys_Error ("vkBindBufferMemory failed for entity BLAS with code %i", (int)err);

	// Create acceleration structure
	ZEROED_STRUCT (VkAccelerationStructureCreateInfoKHR, acceleration_structure_create_info);
	acceleration_structure_create_info.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_CREATE_INFO_KHR;
	acceleration_structure_create_info.buffer = e->blas_data->buffer;
	acceleration_structure_create_info.size = blas_sizes_info.accelerationStructureSize;
	acceleration_structure_create_info.type = VK_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL_KHR;

	err = vulkan_globals.vk_create_acceleration_structure (vulkan_globals.device, &acceleration_structure_create_info, NULL, &e->blas_data->blas);
	if (err != VK_SUCCESS)
		Sys_Error ("vkCreateAccelerationStructure failed for entity BLAS with code %i", (int)err);

	// Get device address
	ZEROED_STRUCT (VkAccelerationStructureDeviceAddressInfoKHR, address_info);
	address_info.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_DEVICE_ADDRESS_INFO_KHR;
	address_info.accelerationStructure = e->blas_data->blas;
	e->blas_data->address = vulkan_globals.vk_get_acceleration_structure_device_address (vulkan_globals.device, &address_info);

	// Store scratch sizes for per-frame rebuilds/updates
	e->blas_data->build_scratch_size = blas_sizes_info.buildScratchSize;
	e->blas_data->update_scratch_size = blas_sizes_info.updateScratchSize;

	// Track which model this BLAS was allocated for
	e->blas_data->model = e->model;
}

/*
================
R_FreeEntityBLAS

Free acceleration structure for an entity
================
*/
void R_FreeEntityBLAS (entity_t *e)
{
	if (!e || !e->blas_data)
		return;

	// Add to garbage collection - resources will be freed after GPU is done with them
	if (e->blas_data->blas != VK_NULL_HANDLE)
		GLMesh_Glue_AddBLASGarbage (e->blas_data->blas, e->blas_data->buffer, e->blas_data->allocation);

	Mem_Free (e->blas_data);
	e->blas_data = NULL;
}

/*
================
R_FreeAllEntityBLASes

Free all entity BLASes. Called when RT shadows are disabled.
================
*/
void R_FreeAllEntityBLASes (void)
{
	if (!cl.entities)
		return;

	for (int i = 0; i < cl.num_entities; i++)
		R_FreeEntityBLAS (&cl.entities[i]);

	for (int i = 0; i < cl.num_statics; i++)
		R_FreeEntityBLAS (cl.static_entities[i]);
}

/*
================
R_UpdateAnimatedBLASes

Update all entity BLASes with animated vertex data.
This dispatches compute shaders to interpolate/skin vertices into the
scratch buffer, then builds/updates the BLASes.
================
*/
#define MAX_PENDING_BLAS_BUILDS 256

static VkAccelerationStructureGeometryKHR			   pending_geometries[MAX_PENDING_BLAS_BUILDS];
static VkAccelerationStructureBuildGeometryInfoKHR	   pending_build_infos[MAX_PENDING_BLAS_BUILDS];
static VkAccelerationStructureBuildRangeInfoKHR		   pending_range_infos[MAX_PENDING_BLAS_BUILDS];
static const VkAccelerationStructureBuildRangeInfoKHR *pending_range_info_ptrs[MAX_PENDING_BLAS_BUILDS];

/*
================
R_FlushPendingBLASBuilds

Flushes pending BLAS builds: inserts compute->AS barrier, builds all pending AS, then inserts appropriate barrier for next phase.
================
*/
static void R_FlushPendingBLASBuilds (cb_context_t *cbx, int num_pending, qboolean more_entities)
{
	if (num_pending == 0)
		return;

	// Barrier: compute writes -> AS reads
	ZEROED_STRUCT (VkMemoryBarrier, compute_barrier);
	compute_barrier.sType = VK_STRUCTURE_TYPE_MEMORY_BARRIER;
	compute_barrier.srcAccessMask = VK_ACCESS_SHADER_WRITE_BIT;
	compute_barrier.dstAccessMask = VK_ACCESS_ACCELERATION_STRUCTURE_READ_BIT_KHR;
	vulkan_globals.vk_cmd_pipeline_barrier (
		cbx->cb, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR, 0, 1, &compute_barrier, 0, NULL, 0, NULL);

	// Single batched AS build call
	vulkan_globals.vk_cmd_build_acceleration_structures (cbx->cb, num_pending, pending_build_infos, pending_range_info_ptrs);

	// Barrier for next phase
	ZEROED_STRUCT (VkMemoryBarrier, as_barrier);
	as_barrier.sType = VK_STRUCTURE_TYPE_MEMORY_BARRIER;
	as_barrier.srcAccessMask = VK_ACCESS_ACCELERATION_STRUCTURE_WRITE_BIT_KHR;

	if (more_entities)
	{
		// More batches coming: need AS_READ for TLAS + SHADER_WRITE for next compute batch
		as_barrier.dstAccessMask = VK_ACCESS_ACCELERATION_STRUCTURE_READ_BIT_KHR | VK_ACCESS_SHADER_WRITE_BIT;
		vulkan_globals.vk_cmd_pipeline_barrier (
			cbx->cb, VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR,
			VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR | VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, 0, 1, &as_barrier, 0, NULL, 0, NULL);
	}
	else
	{
		// Final batch: only need AS_READ for TLAS build
		as_barrier.dstAccessMask = VK_ACCESS_ACCELERATION_STRUCTURE_READ_BIT_KHR;
		vulkan_globals.vk_cmd_pipeline_barrier (
			cbx->cb, VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR, VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR, 0, 1, &as_barrier, 0, NULL,
			0, NULL);
	}
}

void R_UpdateAnimatedBLASes (cb_context_t *cbx)
{
	if (!vulkan_globals.ray_query)
		return;
	if (as_scratch_buffer.buffer == VK_NULL_HANDLE)
		return;

	const VkDeviceSize scratch_alignment = vulkan_globals.physical_device_acceleration_structure_properties.minAccelerationStructureScratchOffsetAlignment;
	// 16 bytes because of device address default buffer_reference_align
	const VkDeviceSize buffer_alignment = q_max (vulkan_globals.device_properties.limits.minStorageBufferOffsetAlignment, 16);
	const int		   total_entities = cl.num_entities + cl.num_statics;

	// Pre-pass: find max scratch size needed across all entities and resize if necessary
	{
		VkDeviceSize max_scratch_needed = 0;
		for (int i = 0; i < total_entities; ++i)
		{
			entity_t *e = (i < cl.num_entities) ? &cl.entities[i] : cl.static_entities[i - cl.num_entities];
			if (!e->model || e->model->needload || e->model->type != mod_alias || !e->blas_data || e->blas_data->blas == VK_NULL_HANDLE)
				continue;
			if ((e->alpha != ENTALPHA_DEFAULT) && (ENTALPHA_DECODE (e->alpha) < 1.0f))
				continue;
			aliashdr_t *hdr = (aliashdr_t *)Mod_Extradata (e->model);
			if (!hdr || hdr->numverts_vbo == 0)
				continue;
			if (e->blas_data->model != e->model)
				continue;

			const VkDeviceSize vertex_size = hdr->numverts_vbo * sizeof (float) * 3;
			const VkDeviceSize as_scratch_size = e->blas_data->needs_initial_build ? e->blas_data->build_scratch_size : e->blas_data->update_scratch_size;
			const VkDeviceSize total_needed = q_align (vertex_size, scratch_alignment) + as_scratch_size;
			max_scratch_needed = q_max (max_scratch_needed, total_needed);
		}

		R_EnsureASScratchBufferSize (max_scratch_needed);
	}

	const VkDeviceSize scratch_buffer_size = as_scratch_buffer_size;
	VkDeviceSize	   scratch_offset = 0;
	int				   num_pending = 0;
	int				   entity_index = 0;

	R_BeginDebugUtilsLabel (cbx, "Update Animated BLAS");

	while (entity_index < total_entities)
	{
		// Phase 1: Compute - dispatch shaders and collect build info
		while (entity_index < total_entities && num_pending < MAX_PENDING_BLAS_BUILDS)
		{
			entity_t *e = (entity_index < cl.num_entities) ? &cl.entities[entity_index] : cl.static_entities[entity_index - cl.num_entities];
			++entity_index;

			if (!e->model || e->model->needload || e->model->type != mod_alias || !e->blas_data || e->blas_data->blas == VK_NULL_HANDLE)
				continue;

			// Skip transparent entities (same as TLAS)
			if ((e->alpha != ENTALPHA_DEFAULT) && (ENTALPHA_DECODE (e->alpha) < 1.0f))
				continue;

			aliashdr_t *hdr = (aliashdr_t *)Mod_Extradata (e->model);
			if (!hdr || hdr->numverts_vbo == 0)
				continue;

			// Skip if BLAS was allocated for a different model/geometry (model changed but entity not visible yet)
			if (e->blas_data->model != e->model)
				continue;

			// Get lerp data for vertex interpolation
			lerpdata_t lerpdata;
			R_SetupAliasFrame (e, hdr, &lerpdata);
			int	  pose1 = lerpdata.pose1;
			int	  pose2 = lerpdata.pose2;
			float blend = lerpdata.blend;

			// Always use refit after first build. We trace few rays and full updates are expensive.
			qboolean use_update = !e->blas_data->needs_initial_build;

			const VkDeviceSize vertex_size = hdr->numverts_vbo * sizeof (float) * 3;
			const VkDeviceSize as_scratch_size = use_update ? e->blas_data->update_scratch_size : e->blas_data->build_scratch_size;

			// Check if we have space; if not, flush current batch and reset
			const VkDeviceSize vertex_offset = q_align (scratch_offset, buffer_alignment);
			const VkDeviceSize as_scratch_offset = q_align (vertex_offset + vertex_size, scratch_alignment);
			const VkDeviceSize total_needed = as_scratch_offset - scratch_offset + as_scratch_size;
			if (scratch_offset + total_needed > scratch_buffer_size)
			{
				// Need to flush - back up entity_index to retry this entity after flush
				--entity_index;
				break;
			}

			e->blas_data->needs_initial_build = false;

			VkDeviceAddress vertex_output_address = as_scratch_buffer.device_address + vertex_offset;
			VkDeviceAddress scratch_address = as_scratch_buffer.device_address + as_scratch_offset;

			// Dispatch compute shader with push constants containing buffer addresses
			if (hdr->poseverttype == PV_MD5 || hdr->poseverttype == PV_MD5_8)
			{
				// MD5 skinning
				skinning_push_constants_t pc = {
					.input_address = hdr->vertex_buffer_address,
					.joints_address = hdr->joints_buffer_address,
					.output_address = vertex_output_address,
					.joints_offset0 = pose1 * hdr->numjoints,
					.joints_offset1 = pose2 * hdr->numjoints,
					.output_offset = 0, // output starts at output_address
					.num_verts = hdr->numverts_vbo,
					.blend_factor = blend,
				};
				R_BindPipeline (
					cbx, VK_PIPELINE_BIND_POINT_COMPUTE,
					(hdr->poseverttype == PV_MD5_8) ? vulkan_globals.skinning_8_pipeline : vulkan_globals.skinning_pipeline);
				R_PushConstants (cbx, VK_SHADER_STAGE_COMPUTE_BIT, 0, sizeof (pc), &pc);
			}
			else
			{
				// MDL/MD3 interpolation
				mesh_interpolate_push_constants_t pc = {
					.input_address = hdr->vertex_buffer_address,
					.output_address = vertex_output_address,
					.pose1_offset = pose1 * hdr->numverts_vbo,
					.pose2_offset = pose2 * hdr->numverts_vbo,
					.output_offset = 0, // output starts at output_address
					.num_verts = hdr->numverts_vbo,
					.blend_factor = blend,
					.flags = (hdr->poseverttype == PV_QUAKE3) ? 0x4 : 0,
				};
				R_BindPipeline (cbx, VK_PIPELINE_BIND_POINT_COMPUTE, vulkan_globals.mesh_interpolate_pipeline);
				R_PushConstants (cbx, VK_SHADER_STAGE_COMPUTE_BIT, 0, sizeof (pc), &pc);
			}

			uint32_t num_groups = (hdr->numverts_vbo + 63) / 64;
			vulkan_globals.vk_cmd_dispatch (cbx->cb, num_groups, 1, 1);

			// Store build info for later
			VkAccelerationStructureGeometryKHR *geom = &pending_geometries[num_pending];
			memset (geom, 0, sizeof (*geom));
			geom->sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_KHR;
			geom->geometryType = VK_GEOMETRY_TYPE_TRIANGLES_KHR;
			geom->geometry.triangles.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_TRIANGLES_DATA_KHR;
			geom->geometry.triangles.vertexFormat = VK_FORMAT_R32G32B32_SFLOAT;
			geom->geometry.triangles.vertexData.deviceAddress = vertex_output_address;
			geom->geometry.triangles.vertexStride = sizeof (float) * 3;
			geom->geometry.triangles.maxVertex = hdr->numverts_vbo;
			geom->geometry.triangles.indexType = VK_INDEX_TYPE_UINT16;
			geom->geometry.triangles.indexData.deviceAddress = hdr->index_buffer_address;

			VkAccelerationStructureBuildGeometryInfoKHR *build = &pending_build_infos[num_pending];
			memset (build, 0, sizeof (*build));
			build->sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_BUILD_GEOMETRY_INFO_KHR;
			build->type = VK_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL_KHR;
			build->flags = VK_BUILD_ACCELERATION_STRUCTURE_PREFER_FAST_BUILD_BIT_KHR | VK_BUILD_ACCELERATION_STRUCTURE_ALLOW_UPDATE_BIT_KHR;
			if (use_update)
			{
				build->mode = VK_BUILD_ACCELERATION_STRUCTURE_MODE_UPDATE_KHR;
				build->srcAccelerationStructure = e->blas_data->blas; // Required for UPDATE mode
			}
			else
			{
				build->mode = VK_BUILD_ACCELERATION_STRUCTURE_MODE_BUILD_KHR;
			}
			build->dstAccelerationStructure = e->blas_data->blas;
			build->geometryCount = 1;
			build->pGeometries = geom;
			build->scratchData.deviceAddress = scratch_address;

			VkAccelerationStructureBuildRangeInfoKHR *range = &pending_range_infos[num_pending];
			memset (range, 0, sizeof (*range));
			range->primitiveCount = hdr->numtris;
			pending_range_info_ptrs[num_pending] = range;

			++num_pending;

			scratch_offset += total_needed;
		}

		// Phase 2: Build - flush pending builds
		if (num_pending > 0)
		{
			qboolean more_entities = (entity_index < total_entities);
			R_FlushPendingBLASBuilds (cbx, num_pending, more_entities);
			num_pending = 0;
			scratch_offset = 0;
		}
	}

	R_EndDebugUtilsLabel (cbx);
}
