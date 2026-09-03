/*
Copyright (C) 1996-2001 Id Software, Inc.
Copyright (C) 2002-2009 John Fitzgibbons and others
Copyright (C) 2007-2008 Kristian Duske
Copyright (C) 2010-2014 QuakeSpasm developers
Copyright (C) 2016 Axel Gneiting
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
// r_part_glue.c -- the C frame around the Rust classic-particle port.
//
// Compiled instead of r_part.c under -Duse_rust_host (Rust migration Phase 7
// M10f-1, T10.5). Unlike the other Pattern A swaps this file is a *split*, not
// just a frame: only r_part.c's simulation half moved to Rust
// (rust/quake-capi/src/r_part.rs). Five jobs:
//
//  1. Keep r_part.c's rendering half in C, verbatim and in its original order
//     (r_part.c:54-221 and r_part.c:951-1106). It is Vulkan-typed throughout
//     -- cb_context_t, VkBuffer, R_VertexAllocate, the particle pipelines --
//     and the renderer belongs to Phase 8 per ROADMAP.md, so porting it now
//     would be out of roadmap order.
//
//  2. Own the particle pool (r_part.c:38, :42). ADR-007: R_DrawParticlesFaces
//     walks active_particles and R_InitParticleIndexBuffer sizes itself from
//     r_numparticles, so a live C reader survives the port and the storage
//     stays here, exactly as sbar_glue.c keeps fragsort/scoreboardlines. The
//     four objects lose their `static` so the Rust core can extern them; no
//     other translation unit defines those names (r_part_fte.c's same-named
//     globals are static, r_part_fte.c:459-461).
//
//  3. Own the cvars and texture state the rendering half reads: r_particles,
//     r_quadparticles, particletexture..particletexture4, texturescalefactor
//     and particle_index_buffer. r_quadparticles also loses its `static` for
//     the same reason as the pool.
//
//  4. Guard R_InitParticles' two Cvar_RegisterVariable calls, which are
//     Host_Reraise wrappers under -Duse_rust_cvar (ADR-009 rule 3), and
//     re-raise from R_InitParticles what the guard caught. This is the
//     module's only raise site; every other entry point is a plain forward.
//
//  5. Bridge the two halves: RPart_Glue_InitRender runs R_InitParticles'
//     rendering tail (r_part.c:250-254) and RPart_Glue_SetParticleTexture_f
//     gives the file-static cvar callback external linkage so the Rust core
//     can hand it to Cvar_SetCallback.
//
// ramp1/ramp2/ramp3 (r_part.c:34-36) and avelocities/beamlength
// (r_part.c:268-269) are read only by the simulation, so they moved to Rust
// and are not duplicated here.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

/* ---------------------------------------------------------------------------
 * C-visible objects (r_part.c:38-52). The pool and r_quadparticles are no
 * longer static: rust/quake-c-sys/src/r_part.rs externs them.
 */

particle_t *active_particles, *free_particles, *particles;

// beware: different from the r_part_fte.c r_numparticles one, this is for classic particles,
// set by "-particles" command line.
int r_numparticles;

gltexture_t *particletexture, *particletexture1, *particletexture2, *particletexture3, *particletexture4; // johnfitz
static float texturescalefactor; // johnfitz -- compensate for apparent size of different particle textures

cvar_t r_particles = {"r_particles", "1", CVAR_ARCHIVE};		 // johnfitz
cvar_t r_quadparticles = {"r_quadparticles", "1", CVAR_ARCHIVE}; // johnfitz

extern cvar_t r_showtris;

static VkBuffer particle_index_buffer;

/*
===============
R_ParticleTextureLookup -- johnfitz -- generate nice antialiased 32x32 circle for particles
===============
*/
int R_ParticleTextureLookup (int x, int y, int sharpness)
{
	int r; // distance from point x,y to circle origin, squared
	int a; // alpha value to return

	x -= 16;
	y -= 16;
	r = x * x + y * y;
	r = r > 255 ? 255 : r;
	a = sharpness * (255 - r);
	a = q_min (a, 255);
	return a;
}

/*
===============
R_InitParticleTextures -- johnfitz -- rewritten
===============
*/
void R_InitParticleTextures (void)
{
	int			x, y;
	static byte particle1_data[64 * 64 * 4];
	static byte particle2_data[2 * 2 * 4];
	static byte particle3_data[64 * 64 * 4];
	byte	   *dst;

	// particle texture 1 -- circle
	dst = particle1_data;
	for (x = 0; x < 64; x++)
		for (y = 0; y < 64; y++)
		{
			*dst++ = 255;
			*dst++ = 255;
			*dst++ = 255;
			*dst++ = R_ParticleTextureLookup (x, y, 8);
		}
	particletexture1 = TexMgr_LoadImage (
		NULL, "particle1", 64, 64, SRC_RGBA, particle1_data, "", (src_offset_t)particle1_data, TEXPREF_PERSIST | TEXPREF_ALPHA | TEXPREF_LINEAR);

	// particle texture 2 -- square
	dst = particle2_data;
	for (x = 0; x < 2; x++)
		for (y = 0; y < 2; y++)
		{
			*dst++ = 255;
			*dst++ = 255;
			*dst++ = 255;
			*dst++ = x || y ? 0 : 255;
		}
	particletexture2 = TexMgr_LoadImage (
		NULL, "particle2", 2, 2, SRC_RGBA, particle2_data, "", (src_offset_t)particle2_data, TEXPREF_PERSIST | TEXPREF_ALPHA | TEXPREF_NEAREST);

	// particle texture 3 -- blob
	dst = particle3_data;
	for (x = 0; x < 64; x++)
		for (y = 0; y < 64; y++)
		{
			*dst++ = 255;
			*dst++ = 255;
			*dst++ = 255;
			*dst++ = R_ParticleTextureLookup (x, y, 2);
		}
	particletexture3 = TexMgr_LoadImage (
		NULL, "particle3", 64, 64, SRC_RGBA, particle3_data, "", (src_offset_t)particle3_data, TEXPREF_PERSIST | TEXPREF_ALPHA | TEXPREF_LINEAR);

	// set default
	particletexture = particletexture1;
	texturescalefactor = 1.27;
}

/*
===============
R_SetParticleTexture_f -- johnfitz
===============
*/
static void R_SetParticleTexture_f (cvar_t *var)
{
	switch ((int)(r_particles.value))
	{
	case 1:
		particletexture = particletexture1;
		texturescalefactor = 1.27;
		break;
	case 2:
		particletexture = particletexture2;
		texturescalefactor = 1.0;
		break;
		//	case 3:
		//		particletexture = particletexture3;
		//		texturescalefactor = 1.5;
		//		break;
	}
}

/*
===============
R_InitParticleIndexBuffer
===============
*/
void R_InitParticleIndexBuffer (void)
{
	uint32_t particle_index_buffer_size = r_numparticles * sizeof (uint16_t) * 6; // 6 indices per particle quad

	VkResult err;

	ZEROED_STRUCT (VkBufferCreateInfo, buffer_create_info);
	buffer_create_info.sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO;
	buffer_create_info.size = particle_index_buffer_size;
	buffer_create_info.usage = VK_BUFFER_USAGE_INDEX_BUFFER_BIT | VK_BUFFER_USAGE_TRANSFER_DST_BIT;

	err = vkCreateBuffer (vulkan_globals.device, &buffer_create_info, NULL, &particle_index_buffer);
	if (err != VK_SUCCESS)
		Sys_Error ("vkCreateBuffer failed with code %i", (int)err);

	GL_SetObjectName ((uint64_t)particle_index_buffer, VK_OBJECT_TYPE_BUFFER, "Particle index buffer");

	VkMemoryRequirements memory_requirements;
	vkGetBufferMemoryRequirements (vulkan_globals.device, particle_index_buffer, &memory_requirements);

	const int aligned_size = q_align (memory_requirements.size, memory_requirements.alignment);

	ZEROED_STRUCT (VkMemoryAllocateInfo, memory_allocate_info);
	memory_allocate_info.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;
	memory_allocate_info.allocationSize = aligned_size;
	memory_allocate_info.memoryTypeIndex = GL_MemoryTypeFromProperties (memory_requirements.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT, 0);

	Atomic_IncrementUInt32 (&num_vulkan_dynbuf_allocations);
	VkDeviceMemory particle_index_buffer_memory;
	Atomic_AddUInt64 (&total_device_vulkan_allocation_size, memory_requirements.size);
	err = vkAllocateMemory (vulkan_globals.device, &memory_allocate_info, NULL, &particle_index_buffer_memory);
	if (err != VK_SUCCESS)
		Sys_Error ("vkAllocateMemory failed with code %i", (int)err);

	GL_SetObjectName ((uint64_t)particle_index_buffer_memory, VK_OBJECT_TYPE_DEVICE_MEMORY, "Particle index buffer");

	err = vkBindBufferMemory (vulkan_globals.device, particle_index_buffer, particle_index_buffer_memory, 0);
	if (err != VK_SUCCESS)
		Sys_Error ("vkBindBufferMemory failed with code %i", (int)err);

	VkBuffer		staging_buffer;
	VkCommandBuffer cb_context;
	int				staging_offset;
	uint16_t	   *staging_indices = (uint16_t *)R_StagingAllocate (particle_index_buffer_size, 1, &cb_context, &staging_buffer, &staging_offset);

	VkBufferCopy region;
	region.srcOffset = staging_offset;
	region.dstOffset = 0;
	region.size = particle_index_buffer_size;
	vkCmdCopyBuffer (cb_context, staging_buffer, particle_index_buffer, 1, &region);

	R_StagingBeginCopy ();
	for (int i = 0; i < r_numparticles; ++i)
	{
		staging_indices[i * 6 + 0] = i * 4 + 0;
		staging_indices[i * 6 + 1] = i * 4 + 1;
		staging_indices[i * 6 + 2] = i * 4 + 2;
		staging_indices[i * 6 + 3] = i * 4 + 0;
		staging_indices[i * 6 + 4] = i * 4 + 2;
		staging_indices[i * 6 + 5] = i * 4 + 3;
	}
	R_StagingEndCopy ();
}

/* ---------------------------------------------------------------------------
 * Bridge from the Rust core back into the rendering half above.
 */

/* r_part.c:250-254 -- R_InitParticles' rendering tail. */
void RPart_Glue_InitRender (void)
{
	if (!no_rendering)
	{
		R_InitParticleTextures (); // johnfitz
		R_InitParticleIndexBuffer ();
	}
}

/* r_part.c:248 -- external linkage for the file-static cvar callback. */
void RPart_Glue_SetParticleTexture_f (cvar_t *var)
{
	R_SetParticleTexture_f (var);
}

/* ---------------------------------------------------------------------------
 * Guarded callback (ADR-009 rule 3).
 */

/* r_part.c:247, :249 -- one Cvar_RegisterVariable. */
static void RPart_InvokeRegisterVariable (void *p)
{
	Cvar_RegisterVariable ((cvar_t *)p);
}

int RPart_Glue_RegisterVariable (cvar_t *var)
{
	return Host_Guard (RPart_InvokeRegisterVariable, var);
}

/* r_part.c:376 -- fscanf. Not a trampoline. A direct fscanf extern on the Rust
   side compiled and linked under cargo test but failed the meson/clang-cl
   engine link with LNK2019: unresolved external symbol fscanf, in build-rs,
   build-rs-cprogs and build-rs-trace (M10f-1 integration). The mechanism was
   not established: the pre-existing Rust fscanf externs used by menu.rs and
   cl_demo.rs resolve in the same binary, so it is not that fscanf lacks an
   importable symbol. Keeping the call here removes the dependency and keeps
   libc's exact scanner, which is the compat surface -- the pointfile is plain
   text and reimplementing float parsing in Rust would be a new divergence,
   not a port. */
int RPart_Glue_ScanPoint (FILE *f, vec3_t org)
{
	return fscanf (f, "%f %f %f\n", &org[0], &org[1], &org[2]);
}

/* ---------------------------------------------------------------------------
 * Re-raising public entry point (ADR-009).
 */

/* r_part.c:227 */
void R_InitParticles (void)
{
	int r = quake_rs_rpart_init_particles ();
	Host_Reraise (r);
}

/* ---------------------------------------------------------------------------
 * Non-raising public entry points: plain forwards to the Rust cores.
 */

/* r_part.c:274 -- the core takes the one field it reads; cbindgen cannot spell
 * entity_t. */
void R_EntityParticles (entity_t *ent)
{
	quake_rs_rpart_entity_particles (ent->origin);
}

/* r_part.c:332 */
void R_ClearParticles (void)
{
	quake_rs_rpart_clear_particles ();
}

/* r_part.c:349 */
void R_ReadPointFile_f (void)
{
	quake_rs_rpart_read_point_file_f ();
}

/* r_part.c:409 */
void R_ParseParticleEffect (void)
{
	quake_rs_rpart_parse_particle_effect ();
}

/* r_part.c:433 */
void R_ParticleExplosion (vec3_t org)
{
	quake_rs_rpart_particle_explosion (org);
}

/* r_part.c:476 */
void R_ParticleExplosion2 (vec3_t org, int colorStart, int colorLength)
{
	quake_rs_rpart_particle_explosion2 (org, colorStart, colorLength);
}

/* r_part.c:509 */
void R_BlobExplosion (vec3_t org)
{
	quake_rs_rpart_blob_explosion (org);
}

/* r_part.c:553 */
void R_RunParticleEffect (vec3_t org, vec3_t dir, int color, int count)
{
	quake_rs_rpart_run_particle_effect (org, dir, color, count);
}

/* r_part.c:610 */
void R_LavaSplash (vec3_t org)
{
	quake_rs_rpart_lava_splash (org);
}

/* r_part.c:651 */
void R_TeleportSplash (vec3_t org)
{
	quake_rs_rpart_teleport_splash (org);
}

/* r_part.c:694 -- start is advanced in place. */
void R_RocketTrail (vec3_t start, vec3_t end, int type)
{
	quake_rs_rpart_rocket_trail (start, end, type);
}

/* r_part.c:806 */
void CL_RunParticles (void)
{
	quake_rs_rpart_run_particles ();
}

/* r_part.c:927 */
uint64_t Harness_HashParticles (uint64_t h)
{
	return quake_rs_rpart_hash_particles (h);
}

/*
===============
R_DrawParticlesFaces
===============
*/
static void R_DrawParticlesFaces (cb_context_t *cbx)
{
	particle_t	 *p;
	float		  scale, texcoord_scale;
	vec3_t		  up, right, up_right, p_up, p_right, p_up_right;
	extern cvar_t r_particles; // johnfitz

	if (!r_particles.value)
		return;

	if (!active_particles)
		return;

	if (r_quadparticles.value)
	{
		VectorScale (vup, 0.75, up);
		VectorScale (vright, 0.75, right);
		texcoord_scale = 0.5f;
	}
	else
	{
		VectorScale (vup, 1.5, up);
		VectorScale (vright, 1.5, right);
		texcoord_scale = 1.0f;
	}

	for (int i = 0; i < 3; ++i)
		up_right[i] = up[i] + right[i];

	int num_particles = 0;
	for (p = active_particles; p; p = p->next)
		num_particles += 1;
	Atomic_AddUInt32 (&rs_particles, num_particles);

	VkBuffer	   vertex_buffer;
	VkDeviceSize   vertex_buffer_offset;
	basicvertex_t *vertices;
	if (r_quadparticles.value)
		vertices = (basicvertex_t *)R_VertexAllocate (num_particles * 4 * sizeof (basicvertex_t), &vertex_buffer, &vertex_buffer_offset);
	else
		vertices = (basicvertex_t *)R_VertexAllocate (num_particles * 3 * sizeof (basicvertex_t), &vertex_buffer, &vertex_buffer_offset);

	int current_vertex = 0;
	for (p = active_particles; p; p = p->next)
	{
		// hack a scale up to keep particles from disapearing
		scale = (p->org[0] - r_origin[0]) * vpn[0] + (p->org[1] - r_origin[1]) * vpn[1] + (p->org[2] - r_origin[2]) * vpn[2];
		if (scale < 20)
			scale = 1 + 0.08; // johnfitz -- added .08 to be consistent
		else
			scale = 1 + scale * 0.004;

		scale *= texturescalefactor; // johnfitz -- compensate for apparent size of different particle textures

		byte *c = (byte *)&d_8to24table[(int)p->color];

		vertices[current_vertex].position[0] = p->org[0];
		vertices[current_vertex].position[1] = p->org[1];
		vertices[current_vertex].position[2] = p->org[2];
		vertices[current_vertex].texcoord[0] = 0.0f;
		vertices[current_vertex].texcoord[1] = 0.0f;
		vertices[current_vertex].color[0] = c[0];
		vertices[current_vertex].color[1] = c[1];
		vertices[current_vertex].color[2] = c[2];
		vertices[current_vertex].color[3] = 255;
		current_vertex++;

		VectorMA (p->org, scale, up, p_up);
		vertices[current_vertex].position[0] = p_up[0];
		vertices[current_vertex].position[1] = p_up[1];
		vertices[current_vertex].position[2] = p_up[2];
		vertices[current_vertex].texcoord[0] = texcoord_scale;
		vertices[current_vertex].texcoord[1] = 0.0f;
		vertices[current_vertex].color[0] = c[0];
		vertices[current_vertex].color[1] = c[1];
		vertices[current_vertex].color[2] = c[2];
		vertices[current_vertex].color[3] = 255;
		current_vertex++;

		if (r_quadparticles.value)
		{
			VectorMA (p->org, scale, up_right, p_up_right);
			vertices[current_vertex].position[0] = p_up_right[0];
			vertices[current_vertex].position[1] = p_up_right[1];
			vertices[current_vertex].position[2] = p_up_right[2];
			vertices[current_vertex].texcoord[0] = texcoord_scale;
			vertices[current_vertex].texcoord[1] = texcoord_scale;
			vertices[current_vertex].color[0] = c[0];
			vertices[current_vertex].color[1] = c[1];
			vertices[current_vertex].color[2] = c[2];
			vertices[current_vertex].color[3] = 255;
			current_vertex++;
		}

		VectorMA (p->org, scale, right, p_right);
		vertices[current_vertex].position[0] = p_right[0];
		vertices[current_vertex].position[1] = p_right[1];
		vertices[current_vertex].position[2] = p_right[2];
		vertices[current_vertex].texcoord[0] = 0.0f;
		vertices[current_vertex].texcoord[1] = texcoord_scale;
		vertices[current_vertex].color[0] = c[0];
		vertices[current_vertex].color[1] = c[1];
		vertices[current_vertex].color[2] = c[2];
		vertices[current_vertex].color[3] = 255;
		current_vertex++;
	}

	vulkan_globals.vk_cmd_bind_vertex_buffers (cbx->cb, 0, 1, &vertex_buffer, &vertex_buffer_offset);
	if (r_quadparticles.value)
	{
		vulkan_globals.vk_cmd_bind_index_buffer (cbx->cb, particle_index_buffer, 0, VK_INDEX_TYPE_UINT16);
		vulkan_globals.vk_cmd_draw_indexed (cbx->cb, num_particles * 6, 1, 0, 0, 0);
	}
	else
		vulkan_globals.vk_cmd_draw (cbx->cb, num_particles * 3, 1, 0, 0);
}

/*
===============
R_DrawParticles -- johnfitz -- moved all non-drawing code to CL_RunParticles
===============
*/
void R_DrawParticles (cb_context_t *cbx)
{
	R_BeginDebugUtilsLabel (cbx, "Particles");
	R_BindPipeline (
		cbx, VK_PIPELINE_BIND_POINT_GRAPHICS,
		R_PipelineForRenderPass (
			cbx->render_pass_index, vulkan_globals.particle_pipeline, vulkan_globals.particle_oit_pipeline, vulkan_globals.particle_mboit_moment_pipeline,
			vulkan_globals.particle_mboit_composite_pipeline));
	vulkan_globals.vk_cmd_bind_descriptor_sets (
		cbx->cb, VK_PIPELINE_BIND_POINT_GRAPHICS, vulkan_globals.basic_pipeline_layout.handle, 0, 1, &particletexture->descriptor_set, 0, NULL);

	R_DrawParticlesFaces (cbx);
	R_EndDebugUtilsLabel (cbx);
}

/*
===============
R_DrawParticles_ShowTris -- johnfitz
===============
*/
void R_DrawParticles_ShowTris (cb_context_t *cbx)
{
	if (r_showtris.value == 1)
		R_BindPipeline (cbx, VK_PIPELINE_BIND_POINT_GRAPHICS, vulkan_globals.showtris_pipeline[R_MainPassPipelineVariant (cbx->render_pass_index)]);
	else
		R_BindPipeline (cbx, VK_PIPELINE_BIND_POINT_GRAPHICS, vulkan_globals.showtris_depth_test_pipeline[R_MainPassPipelineVariant (cbx->render_pass_index)]);

	R_DrawParticlesFaces (cbx);
}

#endif /* USE_RUST_HOST */
