/*
Copyright (C) 1996-2001 Id Software, Inc.
Copyright (C) 2002-2009 John Fitzgibbons and others
Copyright (C) 2007-2008 Kristian Duske
Copyright (C) 2010-2014 QuakeSpasm developers
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
// gl_texmgr_glue.c -- the C remainder of gl_texmgr.c under -Duse_rust_render
//
// Compiled instead of gl_texmgr.c when the Rust texture manager (quake-render
// via quake-capi, Rust migration Phase 8 M4, ADR-015) provides the TexMgr_*
// ABI. This TU keeps what must stay C-visible or C-implemented:
//   - the data every C reader takes through a plain extern (the six
//     d_8to24table* palettes, the six well-known texture pointers) and the
//     two cvars, which Rust writes through hand externs (ADR-007 dual view);
//   - the ADR-009 registration thunks (Cvar_RegisterVariable / Cmd_AddCommand
//     can Host_Error) and TexMgr_Init itself, which re-raises after the Rust
//     frame has returned;
//   - the stb_image_resize implementation (TexMgr_Downsample), which stays C
//     for bit-identical resampling;
//   - accessors into vulkan_globals / qmodel_t / texture_t, which the Rust
//     side cannot spell until M5 (glquake.h is not a bindgen root).
#include "quakedef.h"
#include "gl_heap.h"

#if defined(__GNUC__)
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wunused-function"
#elif defined(_MSC_VER)
#pragma warning(push)
#pragma warning(disable : 4505)
#endif

#define STB_IMAGE_RESIZE_IMPLEMENTATION
#define STB_IMAGE_RESIZE_STATIC
// STB_IMAGERESIZE config:
// plug our Mem_Alloc in stb_image_resize:
// use comma operator to evaluate c, to avoid "unused parameter" warnings
#define STBIR_MALLOC(sz, c) ((void)(c), Mem_Alloc (sz))
#define STBIR_FREE(p, c)	((void)(c), Mem_Free (p))
#include "stb_image_resize.h"

#if defined(__GNUC__)
#pragma GCC diagnostic pop
#elif defined(_MSC_VER)
#pragma warning(pop)
#endif

// The gltexture_t layout the Rust mirror (rust/quake-types/src/render.rs)
// assumes, checked against the real gl_texmgr.h/vulkan_core.h (ADR-011);
// the quake-ctest probe measures the same header. Guarded on a 64-bit
// target like the Rust asserts: the 32-bit layout (64-bit handles realign)
// is unverified on both sides, so -Duse_rust_render is 64-bit only.
#define TEXMGR_LAYOUT_64(expr) (sizeof (void *) != 8 || (expr))
COMPILE_TIME_ASSERT (gltexture_size, TEXMGR_LAYOUT_64 (sizeof (gltexture_t) == 6 * sizeof (void *) + 192));
COMPILE_TIME_ASSERT (gltexture_name, TEXMGR_LAYOUT_64 (offsetof (gltexture_t, name) == 2 * sizeof (void *)));
COMPILE_TIME_ASSERT (gltexture_path_id, TEXMGR_LAYOUT_64 (offsetof (gltexture_t, path_id) == 2 * sizeof (void *) + 64));
COMPILE_TIME_ASSERT (gltexture_flags, TEXMGR_LAYOUT_64 (offsetof (gltexture_t, flags) == 2 * sizeof (void *) + 76));
COMPILE_TIME_ASSERT (gltexture_source_file, TEXMGR_LAYOUT_64 (offsetof (gltexture_t, source_file) == 2 * sizeof (void *) + 80));
COMPILE_TIME_ASSERT (gltexture_source_offset, TEXMGR_LAYOUT_64 (offsetof (gltexture_t, source_offset) == 2 * sizeof (void *) + 144));
COMPILE_TIME_ASSERT (gltexture_source_crc, TEXMGR_LAYOUT_64 (offsetof (gltexture_t, source_crc) == 3 * sizeof (void *) + 156));
COMPILE_TIME_ASSERT (gltexture_pants, TEXMGR_LAYOUT_64 (offsetof (gltexture_t, pants) == 3 * sizeof (void *) + 159));
COMPILE_TIME_ASSERT (gltexture_image, TEXMGR_LAYOUT_64 (offsetof (gltexture_t, image) == 3 * sizeof (void *) + 160));
COMPILE_TIME_ASSERT (gltexture_storage_set, TEXMGR_LAYOUT_64 (offsetof (gltexture_t, storage_descriptor_set) == 4 * sizeof (void *) + 200));
COMPILE_TIME_ASSERT (srcformat_indexed, SRC_INDEXED == 0);
COMPILE_TIME_ASSERT (srcformat_lightmap, SRC_LIGHTMAP == 1);
COMPILE_TIME_ASSERT (srcformat_rgba, SRC_RGBA == 2);
COMPILE_TIME_ASSERT (srcformat_surf_indices, SRC_SURF_INDICES == 3);
COMPILE_TIME_ASSERT (srcformat_rgba_cubemap, SRC_RGBA_CUBEMAP == 4);
COMPILE_TIME_ASSERT (srcformat_indexed_palette, SRC_INDEXED_PALETTE == 5);
COMPILE_TIME_ASSERT (src_offset_size, sizeof (src_offset_t) == sizeof (size_t));

cvar_t gl_max_size = {"gl_max_size", "0", CVAR_NONE};
cvar_t gl_picmip = {"gl_picmip", "0", CVAR_NONE};

gltexture_t *notexture, *nulltexture, *whitetexture, *greytexture, *greylightmap, *bluenoisetexture;

unsigned int d_8to24table[256];
unsigned int d_8to24table_fbright[256];
unsigned int d_8to24table_fbright_fence[256];
unsigned int d_8to24table_nobright[256];
unsigned int d_8to24table_nobright_fence[256];
unsigned int d_8to24table_conchars[256];

extern texture_t *r_notexture_mip, *r_notexture_mip2;

// rust/quake-capi/src/gl_texmgr.rs
int	 quake_rs_texmgr_init (void);
void TexMgr_Rust_Imagelist_f (void);
void TexMgr_Rust_Imagelist_Completion_f (const char *partial);

// Mirrored by GlueEnv in rust/quake-capi/src/gl_texmgr.rs
typedef struct texmgr_glue_env_s
{
	VkDevice	 device;
	uint32_t	 max_image_dimension_2d;
	uint32_t	 max_image_dimension_cube;
	VkFormat	 color_format;
	VkSampler	 point_sampler_lod_bias;
	VkSampler	 linear_sampler_lod_bias;
	VkSampler	 point_aniso_sampler_lod_bias;
	VkSampler	 linear_aniso_sampler_lod_bias;
	VkRenderPass warp_render_pass;
	void		*single_texture_set_layout;
	void		*single_texture_cs_write_set_layout;
} texmgr_glue_env_t;

COMPILE_TIME_ASSERT (texmgr_glue_env_size, TEXMGR_LAYOUT_64 (sizeof (texmgr_glue_env_t) == 3 * sizeof (void *) + 56));
COMPILE_TIME_ASSERT (texmgr_glue_env_format, sizeof (VkFormat) == sizeof (int));

/*
================
TexMgr_Glue_VulkanEnv
================
*/
void TexMgr_Glue_VulkanEnv (texmgr_glue_env_t *out)
{
	out->device = vulkan_globals.device;
	out->max_image_dimension_2d = vulkan_globals.device_properties.limits.maxImageDimension2D;
	out->max_image_dimension_cube = vulkan_globals.device_properties.limits.maxImageDimensionCube;
	out->color_format = vulkan_globals.color_format;
	out->point_sampler_lod_bias = vulkan_globals.point_sampler_lod_bias;
	out->linear_sampler_lod_bias = vulkan_globals.linear_sampler_lod_bias;
	out->point_aniso_sampler_lod_bias = vulkan_globals.point_aniso_sampler_lod_bias;
	out->linear_aniso_sampler_lod_bias = vulkan_globals.linear_aniso_sampler_lod_bias;
	out->warp_render_pass = vulkan_globals.warp_render_pass;
	out->single_texture_set_layout = &vulkan_globals.single_texture_set_layout;
	out->single_texture_cs_write_set_layout = &vulkan_globals.single_texture_cs_write_set_layout;
}

/*
================
TexMgr_Glue_OwnerPathId
================
*/
unsigned int TexMgr_Glue_OwnerPathId (const qmodel_t *owner)
{
	return owner->path_id;
}

/*
================
TexMgr_Glue_SetNotextureMips
================
*/
void TexMgr_Glue_SetNotextureMips (gltexture_t *tex)
{
	r_notexture_mip->gltexture = r_notexture_mip2->gltexture = tex;
}

/*
================
TexMgr_Glue_Downsample

TexMgr_Downsample of gl_texmgr.c: the stb_image_resize implementation stays
C so the mixed build resamples bit-identically to the C oracle.
================
*/
void TexMgr_Glue_Downsample (unsigned *data, int in_width, int in_height, int out_width, int out_height)
{
	const int out_size_bytes = out_width * out_height * 4;

	assert ((out_width >= 1) && (out_width <= in_width));
	assert ((out_height >= 1) && (out_height <= in_height));

	TEMP_ALLOC (byte, image_resize_buffer, out_size_bytes);
	stbir_resize_uint8 ((byte *)data, in_width, in_height, 0, image_resize_buffer, out_width, out_height, 0, 4);
	memcpy (data, image_resize_buffer, out_size_bytes);
	TEMP_FREE (image_resize_buffer);
}

/*
================
TexMgr_Glue_RegisterVariables

ADR-009: Cvar_RegisterVariable can Host_Error, so it runs under Host_Guard
and the status travels back through the Rust frame to TexMgr_Init.
================
*/
static void TexMgr_InvokeRegisterVariables (void *unused)
{
	(void)unused;
	Cvar_RegisterVariable (&gl_max_size);
	Cvar_RegisterVariable (&gl_picmip);
}

int TexMgr_Glue_RegisterVariables (void)
{
	return Host_Guard (TexMgr_InvokeRegisterVariables, NULL);
}

/*
================
TexMgr_Glue_RegisterCommands
================
*/
static void TexMgr_InvokeRegisterCommands (void *unused)
{
	cmd_function_t *cmd;
	(void)unused;
	cmd = Cmd_AddCommand ("imagelist", TexMgr_Rust_Imagelist_f);
	if (cmd)
		cmd->completion = TexMgr_Rust_Imagelist_Completion_f;
}

int TexMgr_Glue_RegisterCommands (void)
{
	return Host_Guard (TexMgr_InvokeRegisterCommands, NULL);
}

/*
================
TexMgr_Init

The Rust core does everything but the re-raise (ADR-009).
================
*/
void TexMgr_Init (void)
{
	Host_Reraise (quake_rs_texmgr_init ());
}
