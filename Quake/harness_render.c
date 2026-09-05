/*
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

// harness_render.c -- -renderhash: per-frame chained hash of the renderer's
// cull decisions and draw-call structure (Rust migration Phase 8, ADR-015).
// Separate TU from harness.c so the headless harness stays Vulkan-free.

#include "quakedef.h"

qboolean harness_renderhash = false;

#define RENDERHASH_MAX_PIPELINES 512
#define RENDERHASH_HASH_BASIS	 0xcbf29ce484222325ull

typedef struct
{
	uint64_t handle;
	uint64_t name_hash;
} renderhash_pipeline_t;

typedef struct
{
	uint32_t kind; // 1 draw, 2 draw_indexed, 3 draw_indexed_indirect
	uint32_t canvas;
	uint64_t pipeline;
	uint32_t count;
	uint32_t instance_count;
	uint32_t first_instance;
	uint32_t pad;
} renderhash_draw_record_t;

// one chain per command-buffer context: the secondary contexts are recorded
// on worker threads in parallel, so a single chain would be order-dependent.
// The entity contexts are the exception: R_DrawEntitiesOnList hands entities
// to whichever task asks next (next_visedict), so both the order and the
// partition across the NUM_ENTITIES_CBX buffers vary run to run; their draws
// are folded commutatively instead.
#define RENDERHASH_NUM_CHAINS	  (PCBX_NUM + SCBX_NUM * 8)
#define RENDERHASH_ENTITIES_FIRST (PCBX_NUM + SCBX_ENTITIES * 8)
#define RENDERHASH_ENTITIES_LAST  (RENDERHASH_ENTITIES_FIRST + NUM_ENTITIES_CBX - 1)

static FILE					*renderhash_file;
static uint64_t				 renderhash_chains[RENDERHASH_NUM_CHAINS];
static atomic_uint64_t		 renderhash_cull_fold; // commutative: entities are culled from several tasks
static atomic_uint32_t		 renderhash_cull_count;
static atomic_uint64_t		 renderhash_ent_fold;
static atomic_uint32_t		 renderhash_ent_count;
static uint64_t				 renderhash_chain;
static int					 renderhash_frame;
static qboolean				 renderhash_detail; // -renderhashdetail: per-frame component lines for triage
static renderhash_pipeline_t renderhash_pipelines[RENDERHASH_MAX_PIPELINES];
static int					 renderhash_num_pipelines;

static PFN_vkCmdDraw				renderhash_orig_draw;
static PFN_vkCmdDrawIndexed			renderhash_orig_draw_indexed;
static PFN_vkCmdDrawIndexedIndirect renderhash_orig_draw_indexed_indirect;

void Harness_RenderInit (void)
{
	int i;

	i = COM_CheckParm ("-renderhash");
	if (i && i < com_argc - 1)
	{
		renderhash_file = Sys_fopen (com_argv[i + 1], "w");
		if (!renderhash_file)
			Sys_Error ("Harness: can't open -renderhash file %s", com_argv[i + 1]);
	}
	else if (harness_renderhash)
		Sys_Error ("Harness: -renderhash needs a file argument");
	renderhash_detail = COM_CheckParm ("-renderhashdetail") != 0;

	for (i = 0; i < RENDERHASH_NUM_CHAINS; i++)
		renderhash_chains[i] = RENDERHASH_HASH_BASIS;
	renderhash_chain = RENDERHASH_HASH_BASIS;
}

void Harness_RenderShutdown (void)
{
	if (renderhash_file)
	{
		fprintf (renderhash_file, "END %d %016" PRIx64 "\n", renderhash_frame, renderhash_chain);
		fclose (renderhash_file);
		renderhash_file = NULL;
	}
}

void Harness_RenderPipelineCreated (uint64_t handle, const char *name)
{
	int i;

	if (!harness_renderhash || renderhash_num_pipelines >= RENDERHASH_MAX_PIPELINES)
		return;
	// handles are reused across pipeline recreation (vid_restart): replace
	for (i = 0; i < renderhash_num_pipelines; i++)
		if (renderhash_pipelines[i].handle == handle)
			break;
	renderhash_pipelines[i].handle = handle;
	renderhash_pipelines[i].name_hash = Harness_Hash64 (RENDERHASH_HASH_BASIS, name, strlen (name));
	if (i == renderhash_num_pipelines)
		renderhash_num_pipelines++;
}

static uint64_t RenderHash_PipelineName (VkPipeline pipeline)
{
	int i;
	for (i = 0; i < renderhash_num_pipelines; i++)
		if (renderhash_pipelines[i].handle == (uint64_t)pipeline)
			return renderhash_pipelines[i].name_hash;
	return 0;
}

// map a command buffer back to its cb_context_t, whose index is stable
// across frames and builds (the VkCommandBuffer handle itself is not)
static int RenderHash_ContextIndex (VkCommandBuffer cb, cb_context_t **cbx_out)
{
	int i, s, m, index;

	for (i = 0; i < PCBX_NUM; i++)
		if (vulkan_globals.primary_cb_contexts[i].cb == cb)
		{
			*cbx_out = &vulkan_globals.primary_cb_contexts[i];
			return i;
		}
	index = PCBX_NUM;
	for (s = 0; s < SCBX_NUM; s++)
	{
		for (m = 0; m < SECONDARY_CB_MULTIPLICITY[s]; m++)
		{
			if (vulkan_globals.secondary_cb_contexts[s] && vulkan_globals.secondary_cb_contexts[s][m].cb == cb)
			{
				*cbx_out = &vulkan_globals.secondary_cb_contexts[s][m];
				return index + m;
			}
		}
		index += 8;
	}
	*cbx_out = NULL;
	return -1;
}

static void RenderHash_Record (VkCommandBuffer cb, uint32_t kind, uint32_t count, uint32_t instance_count, uint32_t first_instance)
{
	renderhash_draw_record_t rec;
	cb_context_t			*cbx;
	int						 index = RenderHash_ContextIndex (cb, &cbx);

	if (index < 0)
		return; // staging/upload command buffers never draw; ignore anything unknown
	memset (&rec, 0, sizeof (rec));
	rec.kind = kind;
	rec.canvas = (uint32_t)cbx->current_canvas;
	rec.pipeline = RenderHash_PipelineName (cbx->current_pipeline.handle);
	rec.count = count;
	rec.instance_count = instance_count;
	rec.first_instance = first_instance;
	if (index >= RENDERHASH_ENTITIES_FIRST && index <= RENDERHASH_ENTITIES_LAST)
	{
		Atomic_AddUInt64 (&renderhash_ent_fold, Harness_Hash64 (RENDERHASH_HASH_BASIS, &rec, sizeof (rec)));
		Atomic_IncrementUInt32 (&renderhash_ent_count);
	}
	else
		renderhash_chains[index] = Harness_Hash64 (renderhash_chains[index], &rec, sizeof (rec));
}

// Buffer offsets, first_index and vertex_offset are deliberately excluded:
// the dynamic vertex/index/uniform rings are carved by parallel tasks, so
// their positions vary run to run even when the draw structure is identical.
static VKAPI_ATTR void VKAPI_CALL
RenderHash_CmdDraw (VkCommandBuffer cb, uint32_t vertex_count, uint32_t instance_count, uint32_t first_vertex, uint32_t first_instance)
{
	RenderHash_Record (cb, 1, vertex_count, instance_count, first_instance);
	renderhash_orig_draw (cb, vertex_count, instance_count, first_vertex, first_instance);
}

static VKAPI_ATTR void VKAPI_CALL RenderHash_CmdDrawIndexed (
	VkCommandBuffer cb, uint32_t index_count, uint32_t instance_count, uint32_t first_index, int32_t vertex_offset, uint32_t first_instance)
{
	RenderHash_Record (cb, 2, index_count, instance_count, first_instance);
	renderhash_orig_draw_indexed (cb, index_count, instance_count, first_index, vertex_offset, first_instance);
}

static VKAPI_ATTR void VKAPI_CALL
RenderHash_CmdDrawIndexedIndirect (VkCommandBuffer cb, VkBuffer buffer, VkDeviceSize offset, uint32_t draw_count, uint32_t stride)
{
	RenderHash_Record (cb, 3, draw_count, stride, 0);
	renderhash_orig_draw_indexed_indirect (cb, buffer, offset, draw_count, stride);
}

void Harness_RenderInstallHooks (void)
{
	if (!harness_renderhash)
		return;
	if (vulkan_globals.vk_cmd_draw == RenderHash_CmdDraw)
		return; // GL_InitDevice ran again (vid_restart): already wrapped
	renderhash_orig_draw = vulkan_globals.vk_cmd_draw;
	renderhash_orig_draw_indexed = vulkan_globals.vk_cmd_draw_indexed;
	renderhash_orig_draw_indexed_indirect = vulkan_globals.vk_cmd_draw_indexed_indirect;
	vulkan_globals.vk_cmd_draw = RenderHash_CmdDraw;
	vulkan_globals.vk_cmd_draw_indexed = RenderHash_CmdDrawIndexed;
	vulkan_globals.vk_cmd_draw_indexed_indirect = RenderHash_CmdDrawIndexedIndirect;
}

void Harness_RenderCull (const struct entity_s *e, qboolean culled)
{
	struct
	{
		uint64_t name_hash;
		uint32_t origin[3];
		uint32_t angles[3];
		uint32_t scale;
		uint32_t culled;
	} rec;
	uint64_t h;

	if (!harness_renderhash)
		return;
	memset (&rec, 0, sizeof (rec));
	rec.name_hash = e->model ? Harness_Hash64 (RENDERHASH_HASH_BASIS, e->model->name, strlen (e->model->name)) : 0;
	memcpy (rec.origin, e->origin, sizeof (rec.origin));
	memcpy (rec.angles, e->angles, sizeof (rec.angles));
	rec.scale = e->netstate.scale;
	rec.culled = culled ? 1 : 0;
	h = Harness_Hash64 (RENDERHASH_HASH_BASIS, &rec, sizeof (rec));
	// entity culling runs from several tasks in parallel: fold commutatively
	Atomic_AddUInt64 (&renderhash_cull_fold, h);
	Atomic_IncrementUInt32 (&renderhash_cull_count);
}

// called from SCR_DrawDone: every draw task of the frame has retired and
// the next frame's marking has not started, so the accumulators are quiet
void Harness_RenderDrawDone (void)
{
	int		 i;
	uint64_t fold, ent_fold;
	uint32_t count, ent_count;

	if (!harness_renderhash || !renderhash_file)
		return;
	fold = Atomic_LoadUInt64 (&renderhash_cull_fold);
	count = Atomic_LoadUInt32 (&renderhash_cull_count);
	ent_fold = Atomic_LoadUInt64 (&renderhash_ent_fold);
	ent_count = Atomic_LoadUInt32 (&renderhash_ent_count);
	if (renderhash_detail)
	{
		fprintf (renderhash_file, "D %d", renderhash_frame);
		if (cl.worldmodel && cl.worldmodel->surfvis)
			fprintf (renderhash_file, " sv=%016" PRIx64, Harness_Hash64 (RENDERHASH_HASH_BASIS, cl.worldmodel->surfvis, (cl.worldmodel->numsurfaces + 31) / 8));
		for (i = 0; i < RENDERHASH_NUM_CHAINS; i++)
			if (renderhash_chains[i] != RENDERHASH_HASH_BASIS)
				fprintf (renderhash_file, " c%d=%016" PRIx64, i, renderhash_chains[i]);
		fprintf (renderhash_file, " ent=%016" PRIx64 " en=%u cull=%016" PRIx64 " n=%u\n", ent_fold, ent_count, fold, count);
	}
	if (cl.worldmodel && cl.worldmodel->surfvis)
		renderhash_chain = Harness_Hash64 (renderhash_chain, cl.worldmodel->surfvis, (cl.worldmodel->numsurfaces + 31) / 8);
	for (i = 0; i < RENDERHASH_NUM_CHAINS; i++)
	{
		renderhash_chain = Harness_Hash64 (renderhash_chain, &renderhash_chains[i], sizeof (renderhash_chains[i]));
		renderhash_chains[i] = RENDERHASH_HASH_BASIS;
	}
	renderhash_chain = Harness_Hash64 (renderhash_chain, &ent_fold, sizeof (ent_fold));
	renderhash_chain = Harness_Hash64 (renderhash_chain, &ent_count, sizeof (ent_count));
	renderhash_chain = Harness_Hash64 (renderhash_chain, &fold, sizeof (fold));
	renderhash_chain = Harness_Hash64 (renderhash_chain, &count, sizeof (count));
	Atomic_StoreUInt64 (&renderhash_ent_fold, 0);
	Atomic_StoreUInt32 (&renderhash_ent_count, 0);
	Atomic_StoreUInt64 (&renderhash_cull_fold, 0);
	Atomic_StoreUInt32 (&renderhash_cull_count, 0);
	fprintf (renderhash_file, "R %d %016" PRIx64 "\n", renderhash_frame, renderhash_chain);
	renderhash_frame++;
}
