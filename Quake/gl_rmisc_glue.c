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

// gl_rmisc_glue.c -- the C that stays with gl_rmisc.c under -Duse_rust_render
// (Rust migration Phase 8 M5). Everything Vulkan -- memory, buffers, staging,
// dynamic buffers, descriptors, samplers, shader modules and pipelines -- is
// quake-capi's gl_rmisc.rs; vulkan_globals and the allocation counters are
// Rust-owned and exported with their C names (ADR-007). What remains here is
// what can Host_Error (cvar/command registration and callbacks), R_Init and
// the map/game/skin/timerefresh/memstats functions that only orchestrate
// other still-C files. Every function body is verbatim gl_rmisc.c.

// r_misc.c

#include "quakedef.h"
#include "gl_heap.h"
#include <float.h>

cvar_t r_lodbias = {"r_lodbias", "1", CVAR_ARCHIVE};
cvar_t gl_lodbias = {"gl_lodbias", "0", CVAR_ARCHIVE};

// johnfitz -- new cvars
extern cvar_t r_clearcolor;
extern cvar_t r_fastclear;
extern cvar_t r_flatlightstyles;
extern cvar_t r_lerplightstyles;
extern cvar_t r_entdlightscale;
extern cvar_t gl_fullbrights;
extern cvar_t gl_farclip;
extern cvar_t r_waterquality;
extern cvar_t r_waterwarp;
extern cvar_t r_waterwarpcompute;
extern cvar_t r_oldskyleaf;
extern cvar_t r_drawworld;
extern cvar_t r_showtris;
extern cvar_t r_showbboxes;
extern cvar_t r_lerpmodels;
extern cvar_t r_lerpmove;
extern cvar_t r_lerpturn;
extern cvar_t r_nolerp_list;
extern cvar_t r_oit;
// johnfitz
extern cvar_t gl_zfix; // QuakeSpasm z-fighting fix
extern cvar_t r_alphasort;

extern cvar_t r_gpulightmapupdate;
extern cvar_t r_rtshadows;
extern cvar_t r_indirect;
extern cvar_t r_tasks;
extern cvar_t r_parallelmark;
extern cvar_t r_usesops;

#if defined(USE_SIMD)
extern cvar_t r_simd;
#endif
extern gltexture_t *playertextures[MAX_SCOREBOARD]; // johnfitz

extern atomic_uint32_t num_vulkan_ubos;
extern atomic_uint32_t num_vulkan_storage_buffers;
extern atomic_uint32_t num_vulkan_sampled_images;
extern atomic_uint32_t num_acceleration_structures;

// The quake_types::render mirrors (VulkanGlobals, CbContext, DynBuffer, ...)
// carry the same numbers as `const` asserts; this is the check against the
// real glquake.h, which the ctest ABI probe cannot see (it probes the
// c_ref_prelude.h copies). 64-bit layout only, as gl_texmgr_glue.c.
#define RMISC_LAYOUT_64(expr) (sizeof (void *) != 8 || (expr))
#ifdef _DEBUG
#define RMISC_VULKANGLOBALS_SIZE 1064968
#else
#define RMISC_VULKANGLOBALS_SIZE 1064952
#endif
COMPILE_TIME_ASSERT (dynbuffer_size, RMISC_LAYOUT_64 (sizeof (dynbuffer_t) == 32));
COMPILE_TIME_ASSERT (dynbuffer_current_offset, offsetof (dynbuffer_t, current_offset) == 8);
COMPILE_TIME_ASSERT (dynbuffer_data, RMISC_LAYOUT_64 (offsetof (dynbuffer_t, data) == 16));
COMPILE_TIME_ASSERT (dynbuffer_device_address, RMISC_LAYOUT_64 (offsetof (dynbuffer_t, device_address) == 24));
COMPILE_TIME_ASSERT (vulkan_pipeline_layout_size, sizeof (vulkan_pipeline_layout_t) == 24);
COMPILE_TIME_ASSERT (vulkan_pipeline_layout_push_constant_range, offsetof (vulkan_pipeline_layout_t, push_constant_range) == 8);
COMPILE_TIME_ASSERT (vulkan_pipeline_layout_mboit_set, offsetof (vulkan_pipeline_layout_t, mboit_input_attachment_set) == 20);
COMPILE_TIME_ASSERT (vulkan_pipeline_size, sizeof (vulkan_pipeline_t) == 32);
COMPILE_TIME_ASSERT (vulkan_pipeline_layout_off, offsetof (vulkan_pipeline_t, layout) == 8);
COMPILE_TIME_ASSERT (vulkan_desc_set_layout_size, sizeof (vulkan_desc_set_layout_t) == 40);
COMPILE_TIME_ASSERT (vulkan_desc_set_layout_cis, offsetof (vulkan_desc_set_layout_t, num_combined_image_samplers) == 8);
COMPILE_TIME_ASSERT (vulkan_desc_set_layout_ubos, offsetof (vulkan_desc_set_layout_t, num_ubos) == 12);
COMPILE_TIME_ASSERT (vulkan_desc_set_layout_ubos_dynamic, offsetof (vulkan_desc_set_layout_t, num_ubos_dynamic) == 16);
COMPILE_TIME_ASSERT (vulkan_desc_set_layout_storage_buffers, offsetof (vulkan_desc_set_layout_t, num_storage_buffers) == 20);
COMPILE_TIME_ASSERT (vulkan_desc_set_layout_input_attachments, offsetof (vulkan_desc_set_layout_t, num_input_attachments) == 24);
COMPILE_TIME_ASSERT (vulkan_desc_set_layout_storage_images, offsetof (vulkan_desc_set_layout_t, num_storage_images) == 28);
COMPILE_TIME_ASSERT (vulkan_desc_set_layout_sampled_images, offsetof (vulkan_desc_set_layout_t, num_sampled_images) == 32);
COMPILE_TIME_ASSERT (vulkan_desc_set_layout_as, offsetof (vulkan_desc_set_layout_t, num_acceleration_structures) == 36);
COMPILE_TIME_ASSERT (buffer_create_info_size, RMISC_LAYOUT_64 (sizeof (buffer_create_info_t) == 56));
COMPILE_TIME_ASSERT (buffer_create_info_size_off, RMISC_LAYOUT_64 (offsetof (buffer_create_info_t, size) == 8));
COMPILE_TIME_ASSERT (buffer_create_info_alignment, RMISC_LAYOUT_64 (offsetof (buffer_create_info_t, alignment) == 16));
COMPILE_TIME_ASSERT (buffer_create_info_usage, RMISC_LAYOUT_64 (offsetof (buffer_create_info_t, usage) == 24));
COMPILE_TIME_ASSERT (buffer_create_info_mapped, RMISC_LAYOUT_64 (offsetof (buffer_create_info_t, mapped) == 32));
COMPILE_TIME_ASSERT (buffer_create_info_address, RMISC_LAYOUT_64 (offsetof (buffer_create_info_t, address) == 40));
COMPILE_TIME_ASSERT (buffer_create_info_name, RMISC_LAYOUT_64 (offsetof (buffer_create_info_t, name) == 48));
COMPILE_TIME_ASSERT (cb_context_size, RMISC_LAYOUT_64 (sizeof (cb_context_t) == 262216));
COMPILE_TIME_ASSERT (cb_context_current_canvas, RMISC_LAYOUT_64 (offsetof (cb_context_t, current_canvas) == 8));
COMPILE_TIME_ASSERT (cb_context_render_pass, RMISC_LAYOUT_64 (offsetof (cb_context_t, render_pass) == 16));
COMPILE_TIME_ASSERT (cb_context_render_pass_index, RMISC_LAYOUT_64 (offsetof (cb_context_t, render_pass_index) == 24));
COMPILE_TIME_ASSERT (cb_context_subpass, RMISC_LAYOUT_64 (offsetof (cb_context_t, subpass) == 28));
COMPILE_TIME_ASSERT (cb_context_current_pipeline, RMISC_LAYOUT_64 (offsetof (cb_context_t, current_pipeline) == 32));
COMPILE_TIME_ASSERT (cb_context_vbo_indices, RMISC_LAYOUT_64 (offsetof (cb_context_t, vbo_indices) == 64));
COMPILE_TIME_ASSERT (cb_context_num_vbo_indices, RMISC_LAYOUT_64 (offsetof (cb_context_t, num_vbo_indices) == 262208));
COMPILE_TIME_ASSERT (vulkanglobals_size, RMISC_LAYOUT_64 (sizeof (vulkanglobals_t) == RMISC_VULKANGLOBALS_SIZE));
COMPILE_TIME_ASSERT (vulkanglobals_device, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, device) == 0));
COMPILE_TIME_ASSERT (vulkanglobals_device_idle, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, device_idle) == 8));
COMPILE_TIME_ASSERT (vulkanglobals_validation, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, validation) == 9));
COMPILE_TIME_ASSERT (vulkanglobals_debug_utils, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, debug_utils) == 10));
COMPILE_TIME_ASSERT (vulkanglobals_queue, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, queue) == 16));
COMPILE_TIME_ASSERT (vulkanglobals_primary_cb_contexts, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, primary_cb_contexts) == 24));
COMPILE_TIME_ASSERT (vulkanglobals_secondary_cb_contexts, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, secondary_cb_contexts) == 1048888));
COMPILE_TIME_ASSERT (vulkanglobals_color_clear_value, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, color_clear_value) == 1049016));
COMPILE_TIME_ASSERT (vulkanglobals_swap_chain_format, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, swap_chain_format) == 1049032));
COMPILE_TIME_ASSERT (vulkanglobals_want_full_screen_exclusive, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, want_full_screen_exclusive) == 1049036));
COMPILE_TIME_ASSERT (vulkanglobals_swap_chain_full_screen_acquired, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, swap_chain_full_screen_acquired) == 1049038));
COMPILE_TIME_ASSERT (vulkanglobals_device_properties, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, device_properties) == 1049040));
COMPILE_TIME_ASSERT (vulkanglobals_device_features, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, device_features) == 1049864));
COMPILE_TIME_ASSERT (vulkanglobals_memory_properties, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, memory_properties) == 1050088));
COMPILE_TIME_ASSERT (vulkanglobals_gfx_queue_family_index, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, gfx_queue_family_index) == 1050608));
COMPILE_TIME_ASSERT (vulkanglobals_color_format, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, color_format) == 1050612));
COMPILE_TIME_ASSERT (vulkanglobals_depth_format, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, depth_format) == 1050616));
COMPILE_TIME_ASSERT (vulkanglobals_sample_count, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, sample_count) == 1050620));
COMPILE_TIME_ASSERT (vulkanglobals_supersampling, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, supersampling) == 1050624));
COMPILE_TIME_ASSERT (vulkanglobals_present_wait, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, present_wait) == 1050634));
COMPILE_TIME_ASSERT (vulkanglobals_color_buffers, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, color_buffers) == 1050640));
COMPILE_TIME_ASSERT (vulkanglobals_oit_accum_buffer, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, oit_accum_buffer) == 1050656));
COMPILE_TIME_ASSERT (vulkanglobals_mboit_color_buffer, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, mboit_color_buffer) == 1050688));
COMPILE_TIME_ASSERT (vulkanglobals_fan_index_buffer, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, fan_index_buffer) == 1050696));
COMPILE_TIME_ASSERT (vulkanglobals_staging_buffer_size, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, staging_buffer_size) == 1050704));
COMPILE_TIME_ASSERT (vulkanglobals_main_render_pass, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, main_render_pass) == 1050712));
COMPILE_TIME_ASSERT (vulkanglobals_warp_render_pass, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, warp_render_pass) == 1050760));
COMPILE_TIME_ASSERT (vulkanglobals_basic_alphatest_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, basic_alphatest_pipeline) == 1050768));
COMPILE_TIME_ASSERT (vulkanglobals_basic_blend_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, basic_blend_pipeline) == 1050992));
COMPILE_TIME_ASSERT (vulkanglobals_basic_notex_blend_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, basic_notex_blend_pipeline) == 1051216));
COMPILE_TIME_ASSERT (vulkanglobals_basic_pipeline_layout, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, basic_pipeline_layout) == 1051440));
COMPILE_TIME_ASSERT (vulkanglobals_world_pipelines, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, world_pipelines) == 1051464));
COMPILE_TIME_ASSERT (vulkanglobals_world_wboit_pipelines, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, world_wboit_pipelines) == 1053000));
COMPILE_TIME_ASSERT (vulkanglobals_world_mboit_moment_pipelines, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, world_mboit_moment_pipelines) == 1053512));
COMPILE_TIME_ASSERT (vulkanglobals_world_mboit_composite_pipelines, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, world_mboit_composite_pipelines) == 1054024));
COMPILE_TIME_ASSERT (vulkanglobals_world_pipeline_layout, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, world_pipeline_layout) == 1054536));
COMPILE_TIME_ASSERT (vulkanglobals_raster_tex_warp_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, raster_tex_warp_pipeline) == 1054560));
COMPILE_TIME_ASSERT (vulkanglobals_particle_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, particle_pipeline) == 1054592));
COMPILE_TIME_ASSERT (vulkanglobals_particle_oit_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, particle_oit_pipeline) == 1054624));
COMPILE_TIME_ASSERT (vulkanglobals_particle_post_oit_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, particle_post_oit_pipeline) == 1054656));
COMPILE_TIME_ASSERT (vulkanglobals_particle_mboit_moment_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, particle_mboit_moment_pipeline) == 1054752));
COMPILE_TIME_ASSERT (
	vulkanglobals_particle_mboit_composite_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, particle_mboit_composite_pipeline) == 1054784));
COMPILE_TIME_ASSERT (vulkanglobals_sprite_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, sprite_pipeline) == 1054816));
COMPILE_TIME_ASSERT (vulkanglobals_sprite_oit_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, sprite_oit_pipeline) == 1054912));
COMPILE_TIME_ASSERT (vulkanglobals_sprite_mboit_moment_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, sprite_mboit_moment_pipeline) == 1054944));
COMPILE_TIME_ASSERT (vulkanglobals_sprite_mboit_composite_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, sprite_mboit_composite_pipeline) == 1054976));
COMPILE_TIME_ASSERT (vulkanglobals_sky_pipeline_layout, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, sky_pipeline_layout) == 1055008));
COMPILE_TIME_ASSERT (vulkanglobals_sky_stencil_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, sky_stencil_pipeline) == 1055056));
COMPILE_TIME_ASSERT (vulkanglobals_sky_color_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, sky_color_pipeline) == 1055248));
COMPILE_TIME_ASSERT (vulkanglobals_sky_box_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, sky_box_pipeline) == 1055440));
COMPILE_TIME_ASSERT (vulkanglobals_sky_cube_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, sky_cube_pipeline) == 1055536));
COMPILE_TIME_ASSERT (vulkanglobals_sky_layer_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, sky_layer_pipeline) == 1055728));
COMPILE_TIME_ASSERT (vulkanglobals_alias_pipelines, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, alias_pipelines) == 1055920));
COMPILE_TIME_ASSERT (vulkanglobals_alias_wboit_pipelines, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, alias_wboit_pipelines) == 1056496));
COMPILE_TIME_ASSERT (vulkanglobals_alias_mboit_moment_pipelines, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, alias_mboit_moment_pipelines) == 1056688));
COMPILE_TIME_ASSERT (vulkanglobals_alias_mboit_composite_pipelines, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, alias_mboit_composite_pipelines) == 1056880));
COMPILE_TIME_ASSERT (vulkanglobals_md5_pipelines, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, md5_pipelines) == 1057072));
COMPILE_TIME_ASSERT (vulkanglobals_md5_wboit_pipelines, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, md5_wboit_pipelines) == 1057648));
COMPILE_TIME_ASSERT (vulkanglobals_md5_mboit_moment_pipelines, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, md5_mboit_moment_pipelines) == 1057840));
COMPILE_TIME_ASSERT (vulkanglobals_md5_mboit_composite_pipelines, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, md5_mboit_composite_pipelines) == 1058032));
COMPILE_TIME_ASSERT (vulkanglobals_md5_8_pipelines, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, md5_8_pipelines) == 1058224));
COMPILE_TIME_ASSERT (vulkanglobals_md5_8_wboit_pipelines, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, md5_8_wboit_pipelines) == 1058800));
COMPILE_TIME_ASSERT (vulkanglobals_md5_8_mboit_moment_pipelines, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, md5_8_mboit_moment_pipelines) == 1058992));
COMPILE_TIME_ASSERT (vulkanglobals_md5_8_mboit_composite_pipelines, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, md5_8_mboit_composite_pipelines) == 1059184));
COMPILE_TIME_ASSERT (vulkanglobals_postprocess_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, postprocess_pipeline) == 1059376));
COMPILE_TIME_ASSERT (vulkanglobals_wboit_resolve_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, wboit_resolve_pipeline) == 1059408));
COMPILE_TIME_ASSERT (vulkanglobals_mboit_resolve_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, mboit_resolve_pipeline) == 1059440));
COMPILE_TIME_ASSERT (vulkanglobals_screen_effects_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, screen_effects_pipeline) == 1059472));
COMPILE_TIME_ASSERT (vulkanglobals_screen_effects_scale_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, screen_effects_scale_pipeline) == 1059504));
COMPILE_TIME_ASSERT (
	vulkanglobals_screen_effects_scale_sops_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, screen_effects_scale_sops_pipeline) == 1059536));
COMPILE_TIME_ASSERT (vulkanglobals_cs_tex_warp_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, cs_tex_warp_pipeline) == 1059568));
COMPILE_TIME_ASSERT (vulkanglobals_showtris_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, showtris_pipeline) == 1059600));
COMPILE_TIME_ASSERT (vulkanglobals_showtris_indirect_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, showtris_indirect_pipeline) == 1059696));
COMPILE_TIME_ASSERT (vulkanglobals_showtris_depth_test_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, showtris_depth_test_pipeline) == 1059792));
COMPILE_TIME_ASSERT (
	vulkanglobals_showtris_indirect_depth_test_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, showtris_indirect_depth_test_pipeline) == 1059888));
COMPILE_TIME_ASSERT (vulkanglobals_showbboxes_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, showbboxes_pipeline) == 1059984));
COMPILE_TIME_ASSERT (vulkanglobals_update_lightmap_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, update_lightmap_pipeline) == 1060080));
COMPILE_TIME_ASSERT (vulkanglobals_update_lightmap_rt_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, update_lightmap_rt_pipeline) == 1060112));
COMPILE_TIME_ASSERT (vulkanglobals_indirect_draw_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, indirect_draw_pipeline) == 1060144));
COMPILE_TIME_ASSERT (vulkanglobals_indirect_clear_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, indirect_clear_pipeline) == 1060176));
COMPILE_TIME_ASSERT (vulkanglobals_ray_debug_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, ray_debug_pipeline) == 1060208));
COMPILE_TIME_ASSERT (vulkanglobals_mesh_interpolate_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, mesh_interpolate_pipeline) == 1060240));
COMPILE_TIME_ASSERT (vulkanglobals_skinning_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, skinning_pipeline) == 1060272));
COMPILE_TIME_ASSERT (vulkanglobals_skinning_8_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, skinning_8_pipeline) == 1060304));
COMPILE_TIME_ASSERT (vulkanglobals_fte_particle_pipelines, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, fte_particle_pipelines) == 1060336));
COMPILE_TIME_ASSERT (vulkanglobals_fte_particle_wboit_pipelines, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, fte_particle_wboit_pipelines) == 1061872));
COMPILE_TIME_ASSERT (vulkanglobals_fte_particle_post_oit_pipelines, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, fte_particle_post_oit_pipelines) == 1062384));
COMPILE_TIME_ASSERT (vulkanglobals_descriptor_pool, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, descriptor_pool) == 1063920));
COMPILE_TIME_ASSERT (vulkanglobals_ubo_set_layout, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, ubo_set_layout) == 1063928));
COMPILE_TIME_ASSERT (vulkanglobals_single_texture_set_layout, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, single_texture_set_layout) == 1063968));
COMPILE_TIME_ASSERT (vulkanglobals_input_attachment_set_layout, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, input_attachment_set_layout) == 1064008));
COMPILE_TIME_ASSERT (vulkanglobals_oit_input_attachment_set_layout, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, oit_input_attachment_set_layout) == 1064048));
COMPILE_TIME_ASSERT (
	vulkanglobals_mboit_input_attachment_set_layout, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, mboit_input_attachment_set_layout) == 1064088));
COMPILE_TIME_ASSERT (
	vulkanglobals_mboit_input_attachment_descriptor_set, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, mboit_input_attachment_descriptor_set) == 1064128));
COMPILE_TIME_ASSERT (vulkanglobals_screen_effects_desc_set, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, screen_effects_desc_set) == 1064136));
COMPILE_TIME_ASSERT (vulkanglobals_screen_effects_set_layout, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, screen_effects_set_layout) == 1064144));
COMPILE_TIME_ASSERT (
	vulkanglobals_single_texture_cs_write_set_layout, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, single_texture_cs_write_set_layout) == 1064184));
COMPILE_TIME_ASSERT (vulkanglobals_lightmap_compute_set_layout, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, lightmap_compute_set_layout) == 1064224));
COMPILE_TIME_ASSERT (vulkanglobals_indirect_compute_desc_set, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, indirect_compute_desc_set) == 1064264));
COMPILE_TIME_ASSERT (vulkanglobals_indirect_compute_set_layout, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, indirect_compute_set_layout) == 1064272));
COMPILE_TIME_ASSERT (vulkanglobals_bmodel_instances_desc_set, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, bmodel_instances_desc_set) == 1064312));
COMPILE_TIME_ASSERT (vulkanglobals_bmodel_instances_set_layout, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, bmodel_instances_set_layout) == 1064320));
COMPILE_TIME_ASSERT (vulkanglobals_ray_query_push_set_layout, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, ray_query_push_set_layout) == 1064360));
COMPILE_TIME_ASSERT (vulkanglobals_ray_debug_desc_set, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, ray_debug_desc_set) == 1064400));
COMPILE_TIME_ASSERT (vulkanglobals_ray_debug_set_layout, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, ray_debug_set_layout) == 1064408));
COMPILE_TIME_ASSERT (vulkanglobals_joints_buffer_set_layout, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, joints_buffer_set_layout) == 1064448));
COMPILE_TIME_ASSERT (vulkanglobals_point_sampler, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, point_sampler) == 1064488));
COMPILE_TIME_ASSERT (vulkanglobals_linear_aniso_sampler_lod_bias, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, linear_aniso_sampler_lod_bias) == 1064544));
COMPILE_TIME_ASSERT (vulkanglobals_projection_matrix, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, projection_matrix) == 1064552));
COMPILE_TIME_ASSERT (vulkanglobals_view_matrix, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, view_matrix) == 1064616));
COMPILE_TIME_ASSERT (vulkanglobals_view_projection_matrix, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, view_projection_matrix) == 1064680));
COMPILE_TIME_ASSERT (vulkanglobals_vk_cmd_bind_pipeline, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, vk_cmd_bind_pipeline) == 1064744));
COMPILE_TIME_ASSERT (vulkanglobals_vk_get_buffer_device_address, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, vk_get_buffer_device_address) == 1064840));
COMPILE_TIME_ASSERT (
	vulkanglobals_vk_get_acceleration_structure_build_sizes,
	RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, vk_get_acceleration_structure_build_sizes) == 1064848));
COMPILE_TIME_ASSERT (
	vulkanglobals_vk_get_acceleration_structure_device_address,
	RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, vk_get_acceleration_structure_device_address) == 1064880));
COMPILE_TIME_ASSERT (
	vulkanglobals_physical_device_acceleration_structure_properties,
	RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, physical_device_acceleration_structure_properties) == 1064888));
#ifdef _DEBUG
COMPILE_TIME_ASSERT (vulkanglobals_vk_cmd_begin_debug_utils_label, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, vk_cmd_begin_debug_utils_label) == 1064952));
COMPILE_TIME_ASSERT (vulkanglobals_vk_cmd_end_debug_utils_label, RMISC_LAYOUT_64 (offsetof (vulkanglobals_t, vk_cmd_end_debug_utils_label) == 1064960));
#endif

void R_VulkanMemStats_f (void);

qboolean   use_simd;
oit_mode_t frame_oit_mode;

qboolean R_UseAlphaSort (void)
{
	return r_alphasort.value && !R_UseOIT ();
}

qboolean R_UseIndirectTransparentWater (void)
{
	return R_UseOIT ();
}

/*
====================
R_ShowbboxesFilter_f
====================
*/
static void R_ShowbboxesFilter_f (void)
{
	extern char	   *r_showbboxes_filter_strings;
	extern qboolean r_showbboxes_filter_byindex;

	if (Cmd_Argc () >= 2)
	{
		// reset
		SAFE_FREE (r_showbboxes_filter_strings);
		r_showbboxes_filter_byindex = false;

		// Concat all arguments adding a ' ' (space) separator
		for (int i = 1; i < Cmd_Argc (); i++)
		{
			const char *arg = Cmd_Argv (i);
			if (!*arg)
				continue;

			r_showbboxes_filter_strings = q_strcatf (r_showbboxes_filter_strings, "%s ", arg);

			r_showbboxes_filter_byindex |= (arg[0] == '#');
		}

		// Re-split r_showbboxes_filter_strings by ' ' (space) on-place, effectively splitting it by '\0'
		if (r_showbboxes_filter_strings)
			q_strsplit (r_showbboxes_filter_strings, " ", NULL);
	}
	else
	{
		const char *p = r_showbboxes_filter_strings;

		Con_SafePrintf ("\"r_showbboxes_filter\" is");
		if (!p)
			Con_SafePrintf (" \"\"");
		else
			do
			{
				Con_SafePrintf (" \"%s\"", p);
				p += strlen (p) + 1;
			} while (*p);
		Con_SafePrintf ("\n");
	}
}

/*
====================
R_ShowbboxesFilter_Completion_f -- tab completion for r_showbboxes_filter
====================
*/
static void R_ShowbboxesFilter_Completion_f (const char *partial)
{
	extern edict_t *sv_player;
	edict_t		   *ed;
	int				i;

	if (!sv.active)
		return;

	QMutex_Lock (draw_qcvm_mutex);
	PR_SwitchQCVM (&sv.qcvm);

	for (i = 1, ed = NEXT_EDICT (qcvm->edicts); i < qcvm->num_edicts; i++, ed = NEXT_EDICT (ed))
	{
		const char *name;
		if (ed == sv_player || ed->free || !ed->v.classname)
			continue;
		name = PR_GetString (ed->v.classname);
		if (*name)
			Con_AddToTabList (name, partial, "#");
	}

	PR_SwitchQCVM (NULL);
	QMutex_Unlock (draw_qcvm_mutex);
}

/*
====================
R_ShowbboxesFilterClear_f
====================
*/
static void R_ShowbboxesFilterClear_f (void)
{
	extern char	   *r_showbboxes_filter_strings;
	extern qboolean r_showbboxes_filter_byindex;

	QMutex_Lock (draw_qcvm_mutex);

	SAFE_FREE (r_showbboxes_filter_strings);
	r_showbboxes_filter_byindex = false;

	QMutex_Unlock (draw_qcvm_mutex);
}

/*
====================
GL_Fullbrights_f -- johnfitz
====================
*/
static void GL_Fullbrights_f (cvar_t *var)
{
	TexMgr_ReloadNobrightImages ();
}

/*
====================
SetClearColor
====================
*/
static void SetClearColor ()
{
	byte *rgb;
	int	  s;

	if (r_fastclear.value != 0.0f)
	{
		// Set to black so fast clear works properly on modern GPUs
		vulkan_globals.color_clear_value.color.float32[0] = 0.0f;
		vulkan_globals.color_clear_value.color.float32[1] = 0.0f;
		vulkan_globals.color_clear_value.color.float32[2] = 0.0f;
		vulkan_globals.color_clear_value.color.float32[3] = 0.0f;
	}
	else
	{
		s = (int)r_clearcolor.value & 0xFF;
		rgb = (byte *)(d_8to24table + s);
		vulkan_globals.color_clear_value.color.float32[0] = rgb[0] / 255.0f;
		vulkan_globals.color_clear_value.color.float32[1] = rgb[1] / 255.0f;
		vulkan_globals.color_clear_value.color.float32[2] = rgb[2] / 255.0f;
		vulkan_globals.color_clear_value.color.float32[3] = 0.0f;
	}
}

/*
====================
R_SetClearColor_f -- johnfitz
====================
*/
static void R_SetClearColor_f (cvar_t *var)
{
	if (r_fastclear.value != 0.0f)
		Con_Warning ("Black clear color forced by r_fastclear\n");

	SetClearColor ();
}

/*
====================
R_SetFastClear_f -- johnfitz
====================
*/
static void R_SetFastClear_f (cvar_t *var)
{
	SetClearColor ();
}

/*
===============
R_Model_ExtraFlags_List_f -- johnfitz -- called when r_nolerp_list cvar changes
===============
*/
static void R_Model_ExtraFlags_List_f (cvar_t *var)
{
	int i;
	for (i = 0; i < MAX_MODELS; i++)
		Mod_SetExtraFlags (cl.model_precache[i]);
}

/*
====================
R_SetWateralpha_f -- ericw
====================
*/
static void R_SetWateralpha_f (cvar_t *var)
{
	if (cls.signon == SIGNONS && cl.worldmodel && !(cl.worldmodel->contentstransparent & SURF_DRAWWATER) && var->value < 1)
		Con_Warning ("Map does not appear to be water-vised\n");
	map_wateralpha = var->value;
	map_fallbackalpha = var->value;
}

#if defined(USE_SIMD)
/*
====================
R_SIMD_f
====================
*/
static void R_SIMD_f (cvar_t *var)
{
#if defined(USE_SSE2)
	use_simd = SDL_HasSSE () && SDL_HasSSE2 () && (var->value != 0.0f);
#elif defined(USE_NEON)
	// We only enable USE_NEON on AArch64 which always has support for it
	use_simd = var->value != 0.0f;
#else
#error not implemented
#endif
}
#endif

/*
====================
R_SetLavaalpha_f -- ericw
====================
*/
static void R_SetLavaalpha_f (cvar_t *var)
{
	if (cls.signon == SIGNONS && cl.worldmodel && !(cl.worldmodel->contentstransparent & SURF_DRAWLAVA) && var->value && var->value < 1)
		Con_Warning ("Map does not appear to be lava-vised\n");
	map_lavaalpha = var->value;
}

/*
====================
R_SetTelealpha_f -- ericw
====================
*/
static void R_SetTelealpha_f (cvar_t *var)
{
	if (cls.signon == SIGNONS && cl.worldmodel && !(cl.worldmodel->contentstransparent & SURF_DRAWTELE) && var->value && var->value < 1)
		Con_Warning ("Map does not appear to be tele-vised\n");
	map_telealpha = var->value;
}

/*
====================
R_SetSlimealpha_f -- ericw
====================
*/
static void R_SetSlimealpha_f (cvar_t *var)
{
	if (cls.signon == SIGNONS && cl.worldmodel && !(cl.worldmodel->contentstransparent & SURF_DRAWSLIME) && var->value && var->value < 1)
		Con_Warning ("Map does not appear to be slime-vised\n");
	map_slimealpha = var->value;
}

/*
====================
R_SetRTShadows_f
====================
*/
static void R_SetRTShadows_f (cvar_t *var)
{
	if (var->value > 0)
	{
		GL_BuildBModelAccelerationStructures ();
	}
	else
	{
		GL_DeleteBModelAccelerationStructures ();
		R_FreeAllEntityBLASes ();
		R_FreeASScratchBuffer ();
	}
	GL_UpdateLightmapDescriptorSets ();
}

/*
====================
GL_WaterAlphaForSurface -- ericw

Returns the map level alpha for the surface's liquid type. Only valid in
contexts where the entity alpha is known to be ENTALPHA_DEFAULT; entities
with an explicit alpha need GL_WaterAlphaForEntityTextureType instead.
====================
*/
float GL_WaterAlphaForSurface (msurface_t *fa)
{
	if (fa->flags & SURF_DRAWLAVA)
		return map_lavaalpha > 0 ? map_lavaalpha : map_fallbackalpha;
	else if (fa->flags & SURF_DRAWTELE)
		return map_telealpha > 0 ? map_telealpha : map_fallbackalpha;
	else if (fa->flags & SURF_DRAWSLIME)
		return map_slimealpha > 0 ? map_slimealpha : map_fallbackalpha;
	else
		return map_wateralpha;
}

float GL_WaterAlphaForTextureType (textype_t type)
{
	switch (type)
	{
	case TEXTYPE_LAVA:
		return map_lavaalpha > 0 ? map_lavaalpha : map_fallbackalpha;
	case TEXTYPE_SLIME:
		return map_slimealpha > 0 ? map_slimealpha : map_fallbackalpha;
	case TEXTYPE_TELE:
		return map_telealpha > 0 ? map_telealpha : map_fallbackalpha;
	case TEXTYPE_WATER:
		return map_wateralpha;
	default:
		return 1.0f;
	}
}

/*
===================
R_ScaleChanged_f
===================
*/
static void R_ScaleChanged_f (cvar_t *var)
{
	R_InitSamplers ();
}

/*
===============
R_Init
===============
*/
void R_Init (void)
{
	cmd_function_t *cmd;

	Cmd_AddCommand ("timerefresh", R_TimeRefresh_f);
	Cmd_AddCommand ("pointfile", R_ReadPointFile_f);

	cmd = Cmd_AddCommand ("r_showbboxes_filter", R_ShowbboxesFilter_f);
	if (cmd)
		cmd->completion = R_ShowbboxesFilter_Completion_f;

	Cmd_AddCommand ("r_showbboxes_filter_clear", R_ShowbboxesFilterClear_f);

	Cmd_AddCommand ("vkmemstats", R_VulkanMemStats_f);

	Cvar_RegisterVariable (&r_fullbright);
	Cvar_RegisterVariable (&r_lightmap);
	Cvar_RegisterVariable (&r_drawentities);
	Cvar_RegisterVariable (&r_drawviewmodel);
	Cvar_RegisterVariable (&r_wateralpha);
	Cvar_RegisterVariable (&r_oit);
	Cvar_SetCallback (&r_wateralpha, R_SetWateralpha_f);
	Cvar_RegisterVariable (&r_dynamic);
	Cvar_RegisterVariable (&r_novis);
#if defined(USE_SIMD)
	Cvar_RegisterVariable (&r_simd);
	Cvar_SetCallback (&r_simd, R_SIMD_f);
	R_SIMD_f (&r_simd);
#endif
	Cvar_RegisterVariable (&r_alphasort);
	Cvar_RegisterVariable (&scr_speeds);
	Cvar_RegisterVariable (&r_pos);
	Cvar_RegisterVariable (&gl_polyblend);
	Cvar_RegisterVariable (&gl_nocolors);
	Cvar_SetCallback (&gl_nocolors, Mod_RefreshSkins_f);

	// johnfitz -- new cvars
	Cvar_RegisterVariable (&r_clearcolor);
	Cvar_SetCallback (&r_clearcolor, R_SetClearColor_f);
	Cvar_RegisterVariable (&r_fastclear);
	Cvar_SetCallback (&r_fastclear, R_SetFastClear_f);
	Cvar_RegisterVariable (&r_waterquality);
	Cvar_RegisterVariable (&r_waterwarp);
	Cvar_RegisterVariable (&r_waterwarpcompute);
	Cvar_RegisterVariable (&r_flatlightstyles);
	Cvar_RegisterVariable (&r_lerplightstyles);
	Cvar_RegisterVariable (&r_entdlightscale);
	Cvar_RegisterVariable (&r_oldskyleaf);
	Cvar_RegisterVariable (&r_drawworld);
	Cvar_RegisterVariable (&r_showtris);
	Cvar_RegisterVariable (&r_showbboxes);
	Cvar_RegisterVariable (&gl_farclip);
	Cvar_RegisterVariable (&gl_fullbrights);
	Cvar_SetCallback (&gl_fullbrights, GL_Fullbrights_f);
	Cvar_RegisterVariable (&r_lerpmodels);
	Cvar_RegisterVariable (&r_lerpmove);
	Cvar_RegisterVariable (&r_lerpturn);
	Cvar_RegisterVariable (&r_nolerp_list);
	Cvar_SetCallback (&r_nolerp_list, R_Model_ExtraFlags_List_f);
	// johnfitz

	Cvar_RegisterVariable (&gl_zfix); // QuakeSpasm z-fighting fix
	Cvar_RegisterVariable (&r_lavaalpha);
	Cvar_RegisterVariable (&r_telealpha);
	Cvar_RegisterVariable (&r_slimealpha);
	Cvar_RegisterVariable (&r_scale);
	Cvar_RegisterVariable (&r_lodbias);
	Cvar_RegisterVariable (&gl_lodbias);
	Cvar_SetCallback (&r_scale, R_ScaleChanged_f);
	Cvar_SetCallback (&r_lodbias, R_ScaleChanged_f);
	Cvar_SetCallback (&gl_lodbias, R_ScaleChanged_f);
	Cvar_SetCallback (&r_lavaalpha, R_SetLavaalpha_f);
	Cvar_SetCallback (&r_telealpha, R_SetTelealpha_f);
	Cvar_SetCallback (&r_slimealpha, R_SetSlimealpha_f);

	Cvar_RegisterVariable (&r_gpulightmapupdate);
	Cvar_RegisterVariable (&r_rtshadows);
	Cvar_SetCallback (&r_rtshadows, R_SetRTShadows_f);
	Cvar_RegisterVariable (&r_indirect);
	Cvar_RegisterVariable (&r_tasks);
	Cvar_RegisterVariable (&r_parallelmark);
	Cvar_RegisterVariable (&r_usesops);

	R_InitParticles ();
	SetClearColor (); // johnfitz

	Sky_Init (); // johnfitz
	Fog_Init (); // johnfitz

	R_AllocateLightmapComputeBuffers ();
}

/*
===============
R_TranslatePlayerSkin -- johnfitz -- rewritten.  also, only handles new colors, not new skins
===============
*/
void R_TranslatePlayerSkin (int playernum)
{
	int top, bottom;

	top = (cl.scores[playernum].colors & 0xf0) >> 4;
	bottom = cl.scores[playernum].colors & 15;

	if (!gl_nocolors.value)
		if (playertextures[playernum])
			TexMgr_ReloadImage (playertextures[playernum], top, bottom);
}

/*
===============
R_TranslateNewPlayerSkin -- johnfitz -- split off of TranslatePlayerSkin -- this is called when
the skin or model actually changes, instead of just new colors
added bug fix from bengt jardup
===============
*/
void R_TranslateNewPlayerSkin (int playernum)
{
	char		name[64];
	byte	   *pixels;
	aliashdr_t *paliashdr;
	int			skinnum;

	if (no_rendering)
		return;

	// get correct texture pixels
	entity_t *currententity = &cl.entities[1 + playernum];

	if (!currententity->model || currententity->model->type != mod_alias)
		return;

	paliashdr = (aliashdr_t *)Mod_Extradata (currententity->model);
	skinnum = currententity->skinnum;

	// TODO: move these tests to the place where skinnum gets received from the server
	if (skinnum < 0 || skinnum >= paliashdr->numskins)
	{
		Con_DPrintf ("(%d): Invalid player skin #%d\n", playernum, skinnum);
		skinnum = 0;
	}

	pixels = (byte *)paliashdr->texels[skinnum];
	if (!pixels)
	{
		static qboolean warned = false;
		if (!warned)
		{
			warned = true;
			Con_Warning ("can't recolor non-indexed player skin\n");
		}
		playertextures[playernum] = NULL;
		return;
	}

	// upload new image
	q_snprintf (name, sizeof (name), "player_%i", playernum);
	playertextures[playernum] = TexMgr_LoadImage (
		currententity->model, name, paliashdr->skinwidth, paliashdr->skinheight, SRC_INDEXED, pixels, paliashdr->gltextures[skinnum][0]->source_file,
		paliashdr->gltextures[skinnum][0]->source_offset, TEXPREF_PAD | TEXPREF_OVERWRITE);

	// now recolor it
	R_TranslatePlayerSkin (playernum);
}

/*
===============
R_NewGame -- johnfitz -- handle a game switch
===============
*/
void R_NewGame (void)
{
	int i;

	// clear playertexture pointers (the textures themselves were freed by texmgr_newgame)
	for (i = 0; i < MAX_SCOREBOARD; i++)
		playertextures[i] = NULL;
}

/*
=============
R_ParseWorldspawn

called at map load
=============
*/
static void R_ParseWorldspawn (void)
{
	char		key[128], value[4096];
	const char *data;

	map_fallbackalpha = r_wateralpha.value;
	map_wateralpha = (cl.worldmodel->contentstransparent & SURF_DRAWWATER) ? r_wateralpha.value : 1;
	map_lavaalpha = (cl.worldmodel->contentstransparent & SURF_DRAWLAVA) ? r_lavaalpha.value : 1;
	map_telealpha = (cl.worldmodel->contentstransparent & SURF_DRAWTELE) ? r_telealpha.value : 1;
	map_slimealpha = (cl.worldmodel->contentstransparent & SURF_DRAWSLIME) ? r_slimealpha.value : 1;

	data = COM_Parse (cl.worldmodel->entities);
	if (!data)
		return; // error
	if (com_token[0] != '{')
		return; // error
	while (1)
	{
		data = COM_Parse (data);
		if (!data)
			return; // error
		if (com_token[0] == '}')
			break; // end of worldspawn
		if (com_token[0] == '_')
			q_strlcpy (key, com_token + 1, sizeof (key));
		else
			q_strlcpy (key, com_token, sizeof (key));
		while (key[0] && key[strlen (key) - 1] == ' ') // remove trailing spaces
			key[strlen (key) - 1] = 0;
		data = COM_ParseEx (data, CPE_ALLOWTRUNC);
		if (!data)
			return; // error
		q_strlcpy (value, com_token, sizeof (value));

		if (!strcmp ("wateralpha", key))
			map_wateralpha = atof (value);

		if (!strcmp ("lavaalpha", key))
			map_lavaalpha = atof (value);

		if (!strcmp ("telealpha", key))
			map_telealpha = atof (value);

		if (!strcmp ("slimealpha", key))
			map_slimealpha = atof (value);
	}
}

/*
===============
R_NewMap
===============
*/
void R_NewMap (void)
{
	int i;

	for (i = 0; i < MAX_LIGHTSTYLES; i++)
		d_lightstylevalue[i] = 264; // normal light value

	// clear out efrags in case the level hasn't been reloaded
	// FIXME: is this one short?
	for (i = 0; i < cl.worldmodel->numleafs; i++)
		cl.worldmodel->leafs[i].efrags = NULL;

	r_viewleaf = NULL;
	R_ClearParticles ();
#ifdef PSET_SCRIPT
	PScript_ClearParticles (true);
#endif

	if (no_rendering)
		return;

	GL_DeleteBModelVertexBuffer ();

	GL_BuildLightmaps ();
	GL_BuildBModelVertexBuffer ();
	GL_BuildBModelAccelerationStructures ();
	GL_PrepareSIMDAndParallelData ();
	GL_SetupIndirectDraws ();
	GL_SetupLightmapCompute ();
	GL_UpdateLightmapDescriptorSets ();
	// ericw -- no longer load alias models into a VBO here, it's done in Mod_LoadAliasModel

	r_framecount = 0;	 // johnfitz -- paranoid?
	r_visframecount = 0; // johnfitz -- paranoid?

	Sky_NewMap ();			 // johnfitz -- skybox in worldspawn
	Fog_NewMap ();			 // johnfitz -- global fog in worldspawn
	R_ParseWorldspawn ();	 // ericw -- wateralpha, lavaalpha, telealpha, slimealpha in worldspawn
	R_ParseEntityDlights (); // 2021 rerelease shadow casting light entities

	GL_UpdateDescriptorSets ();
}

/*
====================
R_TimeRefresh_f

For program optimization
====================
*/
void R_TimeRefresh_f (void)
{
	int	  i;
	float start, stop, time;

	if (cls.state != ca_connected)
	{
		Con_Printf ("Not connected to a server\n");
		return;
	}

	GL_SynchronizeEndRenderingTask ();

	start = Sys_DoubleTime ();
	for (i = 0; i < 128; i++)
	{
		GL_BeginRendering (false, NULL, &glwidth, &glheight);
		r_refdef.viewangles[1] = i / 128.0 * 360.0;
		R_RenderView (false, INVALID_TASK_HANDLE, INVALID_TASK_HANDLE, INVALID_TASK_HANDLE);
		GL_EndRendering (false, false);
	}

	// glFinish ();
	stop = Sys_DoubleTime ();
	time = stop - start;
	Con_Printf ("%f seconds (%f fps)\n", time, 128 / time);
}

/*
====================
R_PrintHeapStats
====================
*/
static void R_PrintHeapStats (const char *name, glheapstats_t *stats)
{
	Con_Printf (
		" %s:\n"
		"  segments: %" SDL_PRIu32 "\n"
		"  allocations: %" SDL_PRIu32 "\n"
		"  small allocations: %" SDL_PRIu32 "\n"
		"  block allocations: %" SDL_PRIu32 "\n"
		"  dedicated allocs: %" SDL_PRIu32 "\n"
		"  blocks used: %" SDL_PRIu32 "\n"
		"  blocks free: %" SDL_PRIu32 "\n"
		"  pages allocated: %" SDL_PRIu32 "\n"
		"  pages free: %" SDL_PRIu32 "\n"
		"  bytes allocated: %" SDL_PRIu64 "\n"
		"  bytes free: %" SDL_PRIu64 "\n"
		"  bytes wasted: %" SDL_PRIu64 " (%.3g%%)\n",
		name, stats->num_segments, stats->num_allocations, stats->num_small_allocations, stats->num_block_allocations, stats->num_dedicated_allocations,
		stats->num_blocks_used, stats->num_blocks_free, stats->num_pages_allocated, stats->num_pages_free, stats->num_bytes_allocated, stats->num_bytes_free,
		stats->num_bytes_wasted, ((double)stats->num_bytes_wasted / (double)stats->num_bytes_allocated) * 100.0f);
}

/*
====================
R_VulkanMemStats_f
====================
*/
void R_VulkanMemStats_f (void)
{
	const uint32_t num_tex_allocations = Atomic_LoadUInt32 (&num_vulkan_tex_allocations);
	const uint32_t num_bmodel_allocations = Atomic_LoadUInt32 (&num_vulkan_bmodel_allocations);
	const uint32_t num_mesh_allocations = Atomic_LoadUInt32 (&num_vulkan_mesh_allocations);
	const uint32_t num_misc_allocations = Atomic_LoadUInt32 (&num_vulkan_misc_allocations);
	const uint32_t num_dynbuf_allocations = Atomic_LoadUInt32 (&num_vulkan_dynbuf_allocations);

	Con_Printf (
		"Vulkan allocations: %" SDL_PRIu32 "\n",
		num_tex_allocations + num_bmodel_allocations + num_mesh_allocations + num_misc_allocations + num_dynbuf_allocations);
	Con_Printf (" Tex:    %" SDL_PRIu32 "\n", num_tex_allocations);
	Con_Printf (" BModel: %" SDL_PRIu32 "\n", num_bmodel_allocations);
	Con_Printf (" Mesh:   %" SDL_PRIu32 "\n", num_mesh_allocations);
	Con_Printf (" Misc:   %" SDL_PRIu32 "\n", num_misc_allocations);
	Con_Printf (" DynBuf: %" SDL_PRIu32 "\n", num_dynbuf_allocations);

	Con_Printf ("Heaps:\n");
	R_PrintHeapStats ("Tex", TexMgr_GetHeapStats ());
	R_PrintHeapStats ("Mesh", R_GetMeshHeapStats ());

	Con_Printf ("Descriptors:\n");
	Con_Printf (" Combined image samplers: %" SDL_PRIu32 "\n", Atomic_LoadUInt32 (&num_vulkan_combined_image_samplers));
	Con_Printf (" Dynamic UBOs: %" SDL_PRIu32 "\n", Atomic_LoadUInt32 (&num_vulkan_ubos_dynamic));
	Con_Printf (" UBOs: %" SDL_PRIu32 "\n", Atomic_LoadUInt32 (&num_vulkan_ubos));
	Con_Printf (" Storage buffers: %" SDL_PRIu32 "\n", Atomic_LoadUInt32 (&num_vulkan_ubos_dynamic));
	Con_Printf (" Input attachments: %" SDL_PRIu32 "\n", Atomic_LoadUInt32 (&num_vulkan_storage_buffers));
	Con_Printf (" Storage images: %" SDL_PRIu32 "\n", Atomic_LoadUInt32 (&num_vulkan_storage_images));
	Con_Printf (" Sampled images: %" SDL_PRIu32 "\n", Atomic_LoadUInt32 (&num_vulkan_sampled_images));
	Con_Printf (" Acceleration structures: %" SDL_PRIu32 "\n", Atomic_LoadUInt32 (&num_acceleration_structures));
	Con_Printf ("Device %" SDL_PRIu64 " MiB total\n", Atomic_LoadUInt64 (&total_device_vulkan_allocation_size) / 1024 / 1024);
	Con_Printf ("Host %" SDL_PRIu64 " MiB total\n", Atomic_LoadUInt64 (&total_host_vulkan_allocation_size) / 1024 / 1024);
}
