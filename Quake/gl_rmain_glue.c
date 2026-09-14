/*
Copyright (C) 1996-2001 Id Software, Inc.
Copyright (C) 2002-2009 John Fitzgibbons and others
Copyright (C) 2010-2014 QuakeSpasm developers

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

// gl_rmain_glue.c -- the C that stays with gl_rmain.c under -Duse_rust_render
// (Rust migration Phase 8 M9). The cvar/global definitions the rest of the
// engine reads by name stay here (frustum, r_refdef, the rs_* counters, the
// view vectors, d_lightstylevalue, the cheat-safe flags, ...), together with
// R_PrintStats (%5.3g output, ADR-005), the r_showbboxes edict walk
// (PR_GetString/NUM_FOR_EDICT under draw_qcvm_mutex) and the Host_Error-
// capable model draws: R_DrawAliasModel/R_DrawSpriteModel resolve model
// headers through Mod_Extradata, which can Host_Error, so on the main thread
// they run under Host_Guard and quake-capi's gl_rmain.rs returns the code
// from RRmain_RenderView for R_RenderView to Host_Reraise (ADR-009). On a
// task worker there is no jmp_buf to guard -- the C original longjmp'd a
// main-thread jmp_buf from the worker (plan I2) -- so the thunks call the
// draw entry directly there.

#include "quakedef.h"
#include "tasks.h"
#include "atomics.h"

// The quake_types::render mirrors the Rust frame graph and the M8 renderer
// hand across the seam (CbContext, LerpData, Lightmap and its members) carry
// the same numbers as `const` asserts and the ctest ABI probe checks them
// against c_ref_prelude.h's copies; this is the check against the real
// glquake.h. 64-bit layout only, as gl_rmisc_glue.c.
#define RMAIN_LAYOUT_64(expr) (sizeof (void *) != 8 || (expr))
COMPILE_TIME_ASSERT (cb_context_size, RMAIN_LAYOUT_64 (sizeof (cb_context_t) == 262216));
COMPILE_TIME_ASSERT (cb_context_current_canvas, RMAIN_LAYOUT_64 (offsetof (cb_context_t, current_canvas) == 8));
COMPILE_TIME_ASSERT (cb_context_render_pass, RMAIN_LAYOUT_64 (offsetof (cb_context_t, render_pass) == 16));
COMPILE_TIME_ASSERT (cb_context_render_pass_index, RMAIN_LAYOUT_64 (offsetof (cb_context_t, render_pass_index) == 24));
COMPILE_TIME_ASSERT (cb_context_subpass, RMAIN_LAYOUT_64 (offsetof (cb_context_t, subpass) == 28));
COMPILE_TIME_ASSERT (cb_context_current_pipeline, RMAIN_LAYOUT_64 (offsetof (cb_context_t, current_pipeline) == 32));
COMPILE_TIME_ASSERT (cb_context_vbo_indices, RMAIN_LAYOUT_64 (offsetof (cb_context_t, vbo_indices) == 64));
COMPILE_TIME_ASSERT (cb_context_num_vbo_indices, RMAIN_LAYOUT_64 (offsetof (cb_context_t, num_vbo_indices) == 262208));
COMPILE_TIME_ASSERT (lerpdata_size, sizeof (lerpdata_t) == 32);
COMPILE_TIME_ASSERT (lerpdata_blend, offsetof (lerpdata_t, blend) == 4);
COMPILE_TIME_ASSERT (lerpdata_origin, offsetof (lerpdata_t, origin) == 8);
COMPILE_TIME_ASSERT (lerpdata_angles, offsetof (lerpdata_t, angles) == 20);
COMPILE_TIME_ASSERT (lm_workgroup_bounds_submodel, offsetof (lm_compute_workgroup_bounds_t, submodel) == 24);
COMPILE_TIME_ASSERT (glrect_size, sizeof (glRect_t) == 8);
COMPILE_TIME_ASSERT (glmaxused_size, sizeof (glMaxUsed_t) == 4);
COMPILE_TIME_ASSERT (lightmap_size, RMAIN_LAYOUT_64 (sizeof (struct lightmap_s) == 3568));
COMPILE_TIME_ASSERT (lightmap_descriptor_set, RMAIN_LAYOUT_64 (offsetof (struct lightmap_s, descriptor_set) == 40));
COMPILE_TIME_ASSERT (lightmap_modified, RMAIN_LAYOUT_64 (offsetof (struct lightmap_s, modified) == 48));
COMPILE_TIME_ASSERT (lightmap_workgroup_bounds_buffer, RMAIN_LAYOUT_64 (offsetof (struct lightmap_s, workgroup_bounds_buffer) == 176));
COMPILE_TIME_ASSERT (lightmap_rectchange, RMAIN_LAYOUT_64 (offsetof (struct lightmap_s, rectchange) == 184));
COMPILE_TIME_ASSERT (lightmap_lightstyle_rectused, RMAIN_LAYOUT_64 (offsetof (struct lightmap_s, lightstyle_rectused) == 192));
COMPILE_TIME_ASSERT (lightmap_global_bounds, RMAIN_LAYOUT_64 (offsetof (struct lightmap_s, global_bounds) == 208));
COMPILE_TIME_ASSERT (lightmap_active_dlights, RMAIN_LAYOUT_64 (offsetof (struct lightmap_s, active_dlights) == 1104));
COMPILE_TIME_ASSERT (lightmap_used_lightstyles, RMAIN_LAYOUT_64 (offsetof (struct lightmap_s, used_lightstyles) == 1200));
COMPILE_TIME_ASSERT (lightmap_cached_light, RMAIN_LAYOUT_64 (offsetof (struct lightmap_s, cached_light) == 3248));
COMPILE_TIME_ASSERT (lightmap_cached_framecount, RMAIN_LAYOUT_64 (offsetof (struct lightmap_s, cached_framecount) == 3504));
COMPILE_TIME_ASSERT (lightmap_data, RMAIN_LAYOUT_64 (offsetof (struct lightmap_s, data) == 3512));
COMPILE_TIME_ASSERT (lightmap_workgroup_bounds, RMAIN_LAYOUT_64 (offsetof (struct lightmap_s, workgroup_bounds) == 3560));

int r_visframecount; // bumped when going to a new PVS
int r_framecount;	 // used for dlight push checking

mplane_t frustum[4];

qboolean render_warp;
int		 render_scale;

// johnfitz -- rendering statistics
atomic_uint32_t rs_brushpolys, rs_aliaspolys, rs_skypolys, rs_particles, rs_fogpolys;
atomic_uint32_t rs_dynamiclightmaps, rs_brushpasses, rs_aliaspasses;
uint32_t		rs_cputime_us, rs_gputime_us;
uint32_t		rs_gpuwaittime_us, rs_gpuwaitaccum_us;
double			rs_frame_starttime;
char			rs_display_lines[3][40];
int				rs_display_numlines;

//
// view origin
//
vec3_t vup;
vec3_t vpn;
vec3_t vright;
vec3_t r_origin;

float r_fovx, r_fovy; // johnfitz -- rendering fov may be different becuase of r_waterwarp

//
// screen size info
//
refdef_t r_refdef;

mleaf_t *r_viewleaf, *r_oldviewleaf;

int d_lightstylevalue[MAX_LIGHTSTYLES]; // 8.8 fraction of base light value

cvar_t r_drawentities = {"r_drawentities", "1", CVAR_NONE};
cvar_t r_drawviewmodel = {"r_drawviewmodel", "1", CVAR_NONE};
cvar_t scr_speeds = {"scr_speeds", "0", CVAR_NONE};
cvar_t r_pos = {"r_pos", "0", CVAR_NONE};
cvar_t r_fullbright = {"r_fullbright", "0", CVAR_NONE};
cvar_t r_lightmap = {"r_lightmap", "0", CVAR_NONE};
cvar_t r_wateralpha = {"r_wateralpha", "1", CVAR_ARCHIVE};
cvar_t r_oit = {"r_oit", "1", CVAR_ARCHIVE};
cvar_t r_dynamic = {"r_dynamic", "1", CVAR_ARCHIVE};
cvar_t r_novis = {"r_novis", "0", CVAR_ARCHIVE};
#if defined(USE_SIMD)
cvar_t r_simd = {"r_simd", "1", CVAR_ARCHIVE};
#endif
cvar_t r_alphasort = {"r_alphasort", "1", CVAR_ARCHIVE};

cvar_t gl_finish = {"gl_finish", "0", CVAR_NONE};
cvar_t gl_polyblend = {"gl_polyblend", "1", CVAR_NONE};
cvar_t gl_nocolors = {"gl_nocolors", "0", CVAR_NONE};

// johnfitz -- new cvars
cvar_t r_clearcolor = {"r_clearcolor", "2", CVAR_ARCHIVE};
cvar_t r_fastclear = {"r_fastclear", "1", CVAR_ARCHIVE};
cvar_t r_flatlightstyles = {"r_flatlightstyles", "0", CVAR_NONE};
cvar_t r_lerplightstyles = {"r_lerplightstyles", "1", CVAR_ARCHIVE}; // 0=off; 1=skip abrupt transitions; 2=always lerp
cvar_t gl_fullbrights = {"gl_fullbrights", "1", CVAR_ARCHIVE};
cvar_t gl_farclip = {"gl_farclip", "16384", CVAR_ARCHIVE};
cvar_t r_oldskyleaf = {"r_oldskyleaf", "0", CVAR_NONE};
cvar_t r_drawworld = {"r_drawworld", "1", CVAR_NONE};
cvar_t r_showtris = {"r_showtris", "0", CVAR_NONE};
cvar_t r_showbboxes = {"r_showbboxes", "0", CVAR_NONE};
cvar_t r_lerpmodels = {"r_lerpmodels", "1", CVAR_ARCHIVE};
cvar_t r_lerpmove = {"r_lerpmove", "1", CVAR_ARCHIVE};
cvar_t r_lerpturn = {"r_lerpturn", "1", CVAR_ARCHIVE};
cvar_t r_nolerp_list = {
	"r_nolerp_list",
	"progs/flame.mdl,progs/flame2.mdl,progs/braztall.mdl,progs/brazshrt.mdl,progs/longtrch.mdl,progs/flame_pyre.mdl,progs/v_saw.mdl,progs/"
	"v_xfist.mdl,progs/h2stuff/newfire.mdl",
	CVAR_NONE};

extern cvar_t r_vfog;
// johnfitz

cvar_t gl_zfix = {"gl_zfix", "1", CVAR_ARCHIVE}; // QuakeSpasm z-fighting fix

cvar_t r_lavaalpha = {"r_lavaalpha", "0", CVAR_NONE};
cvar_t r_telealpha = {"r_telealpha", "0", CVAR_NONE};
cvar_t r_slimealpha = {"r_slimealpha", "0", CVAR_NONE};

float map_wateralpha, map_lavaalpha, map_telealpha, map_slimealpha;
float map_fallbackalpha;

qboolean r_drawworld_cheatsafe, r_fullbright_cheatsafe, r_lightmap_cheatsafe; // johnfitz

cvar_t r_scale = {"r_scale", "1", CVAR_ARCHIVE};

cvar_t r_gpulightmapupdate = {"r_gpulightmapupdate", "1", CVAR_NONE};
cvar_t r_rtshadows = {"r_rtshadows", "2", CVAR_ARCHIVE};

cvar_t r_tasks = {"r_tasks", "1", CVAR_NONE};

cvar_t			r_indirect = {"r_indirect", "1", CVAR_NONE};
extern qboolean indirect_ready;

/*
================
R_EmitWirePoint -- johnfitz -- draws a wireframe cross shape for point entities
================
*/
void R_EmitWirePoint (cb_context_t *cbx, vec3_t origin)
{
	VkBuffer	   vertex_buffer;
	VkDeviceSize   vertex_buffer_offset;
	basicvertex_t *vertices = (basicvertex_t *)R_VertexAllocate (6 * sizeof (basicvertex_t), &vertex_buffer, &vertex_buffer_offset);
	int			   size = 8;

	vertices[0].position[0] = origin[0] - size;
	vertices[0].position[1] = origin[1];
	vertices[0].position[2] = origin[2];
	vertices[1].position[0] = origin[0] + size;
	vertices[1].position[1] = origin[1];
	vertices[1].position[2] = origin[2];
	vertices[2].position[0] = origin[0];
	vertices[2].position[1] = origin[1] - size;
	vertices[2].position[2] = origin[2];
	vertices[3].position[0] = origin[0];
	vertices[3].position[1] = origin[1] + size;
	vertices[3].position[2] = origin[2];
	vertices[4].position[0] = origin[0];
	vertices[4].position[1] = origin[1];
	vertices[4].position[2] = origin[2] - size;
	vertices[5].position[0] = origin[0];
	vertices[5].position[1] = origin[1];
	vertices[5].position[2] = origin[2] + size;

	vulkan_globals.vk_cmd_bind_vertex_buffers (cbx->cb, 0, 1, &vertex_buffer, &vertex_buffer_offset);
	vulkan_globals.vk_cmd_draw (cbx->cb, 6, 1, 0, 0);
}

/*
================
R_EmitWireBox -- johnfitz -- draws one axis aligned bounding box
================
*/
void R_EmitWireBox (cb_context_t *cbx, vec3_t mins, vec3_t maxs, VkBuffer box_index_buffer, VkDeviceSize box_index_buffer_offset)
{
	VkBuffer	   vertex_buffer;
	VkDeviceSize   vertex_buffer_offset;
	basicvertex_t *vertices = (basicvertex_t *)R_VertexAllocate (8 * sizeof (basicvertex_t), &vertex_buffer, &vertex_buffer_offset);

	for (int i = 0; i < 8; ++i)
	{
		vertices[i].position[0] = ((i % 2) < 1) ? mins[0] : maxs[0];
		vertices[i].position[1] = ((i % 4) < 2) ? mins[1] : maxs[1];
		vertices[i].position[2] = ((i % 8) < 4) ? mins[2] : maxs[2];
	}

	vulkan_globals.vk_cmd_bind_index_buffer (cbx->cb, box_index_buffer, box_index_buffer_offset, VK_INDEX_TYPE_UINT16);
	vulkan_globals.vk_cmd_bind_vertex_buffers (cbx->cb, 0, 1, &vertex_buffer, &vertex_buffer_offset);
	vulkan_globals.vk_cmd_draw_indexed (cbx->cb, 24, 1, 0, 0, 0);
}

static uint16_t box_indices[24] = {0, 1, 2, 3, 4, 5, 6, 7, 0, 4, 1, 5, 2, 6, 3, 7, 0, 2, 1, 3, 4, 6, 5, 7};

/*
================
R_ShowBoundingBoxesFilter

r_showbboxes_filter "artifact,=trigger_secret"
================
*/
char	*r_showbboxes_filter_strings = NULL;
qboolean r_showbboxes_filter_byindex = false;

static qboolean R_ShowBoundingBoxesFilter (edict_t *ed)
{
	char		entnum[16] = "";
	const char *classname = NULL;
	const char *filter_p = r_showbboxes_filter_strings;

	if (!r_showbboxes_filter_strings || !r_showbboxes_filter_strings[0])
		return true;

	if (r_showbboxes_filter_byindex)
		q_snprintf (entnum, sizeof (entnum), "%d", NUM_FOR_EDICT (ed));

	if (ed->v.classname)
		classname = PR_GetString (ed->v.classname);

	for (filter_p = r_showbboxes_filter_strings; *filter_p; filter_p += strlen (filter_p) + 1)
	{
		if (*filter_p == '#')
		{
			if (!strcmp (entnum, filter_p + 1))
				return true;
			continue;
		}

		if (!classname)
			continue;

		if (*filter_p == '=')
		{
			if (!strcmp (classname, filter_p + 1))
				return true;
			continue;
		}

		if (strstr (classname, filter_p) != NULL)
			return true;
	}

	return false;
}

/*
================
R_ShowBoundingBoxes -- johnfitz

draw bounding boxes -- the server-side boxes, not the renderer cullboxes
================
*/
static void R_ShowBoundingBoxes (cb_context_t *cbx)
{
	extern edict_t *sv_player;
	vec3_t			mins, maxs, center;
	edict_t		   *ed;
	int				i, pass;

	if (!r_showbboxes.value || cl.maxclients > 1 || !r_drawentities.value || !sv.active)
		return;

	R_BeginDebugUtilsLabel (cbx, "show bboxes");
	if (vulkan_globals.non_solid_fill)
		R_BindPipeline (cbx, VK_PIPELINE_BIND_POINT_GRAPHICS, vulkan_globals.showbboxes_pipeline[R_MainPassPipelineVariant (cbx->render_pass_index)]);

	VkBuffer	 box_index_buffer;
	VkDeviceSize box_index_buffer_offset;
	uint16_t	*indices = (uint16_t *)R_IndexAllocate (24 * sizeof (uint16_t), &box_index_buffer, &box_index_buffer_offset);
	memcpy (indices, box_indices, 24 * sizeof (uint16_t));

	QMutex_Lock (draw_qcvm_mutex);
	PR_SwitchQCVM (&sv.qcvm);
	for (pass = 0; pass < 2; pass++) // two passes (0 = lines, 1 = text) to avoid switching pipelines for every edict and so that the text is on top
	{
		if (pass == 0 && !vulkan_globals.non_solid_fill)
			continue;
		if (pass == 1 && r_showbboxes.value < 0)
			continue;
		for (i = 1, ed = NEXT_EDICT (qcvm->edicts); i < qcvm->num_edicts; i++, ed = NEXT_EDICT (ed))
		{
			if (ed == sv_player || ed->free)
				continue; // don't draw player's own bbox or freed edicts

			if (!R_ShowBoundingBoxesFilter (ed))
				continue;

			if (ed->v.mins[0] == ed->v.maxs[0] && ed->v.mins[1] == ed->v.maxs[1] && ed->v.mins[2] == ed->v.maxs[2])
			{
				// point entity
				if (pass == 0)
				{
					R_EmitWirePoint (cbx, ed->v.origin);
				}
				else
				{
					VectorCopy (ed->v.origin, center);
					center[2] += 16; // show a bit above
				}
			}
			else
			{
				// box entity
				VectorAdd (ed->v.mins, ed->v.origin, mins);
				VectorAdd (ed->v.maxs, ed->v.origin, maxs);
				if (pass == 0)
				{
					R_EmitWireBox (cbx, mins, maxs, box_index_buffer, box_index_buffer_offset);
				}
				else
				{
					VectorAdd (mins, maxs, center);
					for (int j = 0; j < 3; j++)
						center[j] /= 2;
				}
			}

			if (pass == 1)
			{
				char text[16];
				q_snprintf (text, sizeof (text), "%i", i);
				for (char *c = text; *c; c++)
					*c |= 0x80; // the lines are already white so gold is more legible
				Draw_String_3D (cbx, center, 8, text);
			}
		}
	}
	PR_SwitchQCVM (NULL);
	QMutex_Unlock (draw_qcvm_mutex);

	R_EndDebugUtilsLabel (cbx);
}

/*
================
R_PrintStats
================
*/
void R_PrintStats (void)
{
	// johnfitz -- modified scr_speeds output
	double		 lms = r_gpulightmapupdate.value
						   ? (double)Atomic_LoadUInt32 (&rs_dynamiclightmaps) / (LMBLOCK_HEIGHT / LM_CULL_BLOCK_H * LMBLOCK_WIDTH / LM_CULL_BLOCK_W)
						   : Atomic_LoadUInt32 (&rs_dynamiclightmaps);
	const double cpu_ms = (double)rs_cputime_us / 1000.0;
	const double gpu_ms = (double)rs_gputime_us / 1000.0;
	const double gpu_wait_ms = (double)rs_gpuwaittime_us / 1000.0;
	rs_display_numlines = 0;
	if (r_pos.value)
		Con_Printf (
			"x %i y %i z %i (pitch %i yaw %i roll %i)\n", (int)cl.entities[cl.viewentity].origin[0], (int)cl.entities[cl.viewentity].origin[1],
			(int)cl.entities[cl.viewentity].origin[2], (int)cl.viewangles[PITCH], (int)cl.viewangles[YAW], (int)cl.viewangles[ROLL]);
	else if (scr_speeds.value)
	{
		q_snprintf (rs_display_lines[0], sizeof (rs_display_lines[0]), "cpu%6.2f gpu%6.2f wait%6.2f ms", cpu_ms, gpu_ms, gpu_wait_ms);
		if (scr_speeds.value == 2)
		{
			q_snprintf (
				rs_display_lines[1], sizeof (rs_display_lines[1]), "%4u/%u wpoly %4u/%u epoly", rs_brushpolys, rs_brushpasses, rs_aliaspolys, rs_aliaspasses);
			q_snprintf (rs_display_lines[2], sizeof (rs_display_lines[2]), "%5.3g lmap %4u skypoly", lms, rs_skypolys);
			rs_display_numlines = 3;
		}
		else
		{
			q_snprintf (rs_display_lines[1], sizeof (rs_display_lines[1]), "%4u wpoly %4u epoly %5.3g lmap", rs_brushpolys, rs_aliaspolys, lms);
			rs_display_numlines = 2;
		}
	}
	// johnfitz
}

int RRmain_RenderView (qboolean use_tasks, task_handle_t begin_rendering_task, task_handle_t setup_frame_task, task_handle_t draw_done_task);

void R_RenderView (qboolean use_tasks, task_handle_t begin_rendering_task, task_handle_t setup_frame_task, task_handle_t draw_done_task)
{
	Host_Reraise (RRmain_RenderView (use_tasks, begin_rendering_task, setup_frame_task, draw_done_task));
}

/* gl_rmain.c -- R_DrawEntitiesOnList / R_DrawViewModel / R_ShowTris model draws */
typedef struct
{
	cb_context_t *cbx;
	entity_t	 *e;
	int			 *aliaspolys;
} rmain_draw_arg_t;

static void Rmain_InvokeDrawAliasModel (void *p)
{
	rmain_draw_arg_t *a = (rmain_draw_arg_t *)p;
	R_DrawAliasModel (a->cbx, a->e, a->aliaspolys);
}
int RRmain_Glue_DrawAliasModel (cb_context_t *cbx, entity_t *e, int *aliaspolys)
{
	rmain_draw_arg_t arg = {cbx, e, aliaspolys};
	if (Tasks_IsWorker ())
	{
		R_DrawAliasModel (cbx, e, aliaspolys);
		return HOST_GUARD_OK;
	}
	return Host_Guard (Rmain_InvokeDrawAliasModel, &arg);
}

static void Rmain_InvokeDrawAliasModel_ShowTris (void *p)
{
	rmain_draw_arg_t *a = (rmain_draw_arg_t *)p;
	R_DrawAliasModel_ShowTris (a->cbx, a->e);
}
int RRmain_Glue_DrawAliasModel_ShowTris (cb_context_t *cbx, entity_t *e)
{
	rmain_draw_arg_t arg = {cbx, e, NULL};
	if (Tasks_IsWorker ())
	{
		R_DrawAliasModel_ShowTris (cbx, e);
		return HOST_GUARD_OK;
	}
	return Host_Guard (Rmain_InvokeDrawAliasModel_ShowTris, &arg);
}

static void Rmain_InvokeDrawSpriteModel (void *p)
{
	rmain_draw_arg_t *a = (rmain_draw_arg_t *)p;
	R_DrawSpriteModel (a->cbx, a->e);
}
int RRmain_Glue_DrawSpriteModel (cb_context_t *cbx, entity_t *e)
{
	rmain_draw_arg_t arg = {cbx, e, NULL};
	if (Tasks_IsWorker ())
	{
		R_DrawSpriteModel (cbx, e);
		return HOST_GUARD_OK;
	}
	return Host_Guard (Rmain_InvokeDrawSpriteModel, &arg);
}

static void Rmain_InvokeDrawSpriteModel_ShowTris (void *p)
{
	rmain_draw_arg_t *a = (rmain_draw_arg_t *)p;
	R_DrawSpriteModel_ShowTris (a->cbx, a->e);
}
int RRmain_Glue_DrawSpriteModel_ShowTris (cb_context_t *cbx, entity_t *e)
{
	rmain_draw_arg_t arg = {cbx, e, NULL};
	if (Tasks_IsWorker ())
	{
		R_DrawSpriteModel_ShowTris (cbx, e);
		return HOST_GUARD_OK;
	}
	return Host_Guard (Rmain_InvokeDrawSpriteModel_ShowTris, &arg);
}

/* gl_rmain.c:1077 -- R_ShowBoundingBoxes (cbx) from R_DrawViewModelTask */
static void Rmain_InvokeShowBoundingBoxes (void *p)
{
	R_ShowBoundingBoxes ((cb_context_t *)p);
}
int RRmain_Glue_ShowBoundingBoxes (cb_context_t *cbx)
{
	if (Tasks_IsWorker ())
	{
		R_ShowBoundingBoxes (cbx);
		return HOST_GUARD_OK;
	}
	return Host_Guard (Rmain_InvokeShowBoundingBoxes, cbx);
}

/* gl_rmain.c:1318 -- the serial-frame PScript_UpdateParticlesSetupTask (NULL).
 * Which side owns it follows use_rust_host, not use_rust_render: the
 * r_part_fte_glue.c entry Host_Reraises a Rust status core, the r_part_fte.c
 * one Host_Errors directly, so the Rust frame goes through the guard here
 * rather than naming either (ADR-009). Main thread only. */
int RRmain_Glue_UpdateParticlesSetup (void)
{
	return Host_Guard (PScript_UpdateParticlesSetupTask, NULL);
}
