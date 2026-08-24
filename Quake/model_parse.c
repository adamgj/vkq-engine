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
// model_parse.c -- model/BSP parsing (Rust migration Phase 3: replaced by
// quake-formats under -Duse_rust_formats; this file is the differential oracle)

#include "quakedef.h"
#include "model_parse.h"

extern cvar_t external_ents;

#ifndef USE_RUST_FORMATS
/*
===============
ReadShortUnaligned
===============
*/
static short ReadShortUnaligned (byte *ptr)
{
	short temp;
	memcpy (&temp, ptr, sizeof (short));
	return LittleShort (temp);
}

/*
===============
ReadLongUnaligned
===============
*/
static int ReadLongUnaligned (byte *ptr)
{
	int temp;
	memcpy (&temp, ptr, sizeof (int));
	return LittleLong (temp);
}

/*
===============
ReadFloatUnaligned
===============
*/
static float ReadFloatUnaligned (byte *ptr)
{
	float temp;
	memcpy (&temp, ptr, sizeof (float));
	return LittleFloat (temp);
}

static byte *mod_decompressed;
static int	 mod_decompressed_capacity;

/*
===================
Mod_DecompressVis
===================
*/
byte *Mod_DecompressVis (byte *in, qmodel_t *model)
{
	int	  c;
	byte *out;
	byte *outend;
	int	  row;

	row = (model->numleafs + 31) / 8;
	if (mod_decompressed == NULL || row > mod_decompressed_capacity)
	{
		mod_decompressed_capacity = row;
		mod_decompressed = (byte *)Mem_Realloc (mod_decompressed, mod_decompressed_capacity);
		if (!mod_decompressed)
			Sys_Error ("Mod_DecompressVis: realloc() failed on %d bytes", mod_decompressed_capacity);
	}
	out = mod_decompressed;
	outend = mod_decompressed + row;

	if (!in)
	{ // no vis info, so make all visible
		while (row)
		{
			*out++ = 0xff;
			row--;
		}
		return mod_decompressed;
	}

	do
	{
		if (*in)
		{
			*out++ = *in++;
			continue;
		}

		c = in[1];
		in += 2;
		if (c > row - (out - mod_decompressed))
			c = row -
				(out -
				 mod_decompressed); // now that we're dynamically allocating pvs buffers, we have to be more careful to avoid heap overflows with buggy maps.
		while (c)
		{
			if (out == outend)
			{
				if (!model->viswarn)
				{
					model->viswarn = true;
					Con_Warning ("Mod_DecompressVis: output overrun on model \"%s\"\n", model->name);
				}
				return mod_decompressed;
			}
			*out++ = 0;
			c--;
		}
	} while (out - mod_decompressed < row);

	return mod_decompressed;
}

/*
================
Mod_TextureTypeFromName
================
*/
static textype_t Mod_TextureTypeFromName (const char *texname)
{
	if (texname[0] == '*' || texname[0] == '!')
	{
		if (!strncmp (texname + 1, "lava", 4))
			return TEXTYPE_LAVA;
		if (!strncmp (texname + 1, "slime", 5))
			return TEXTYPE_SLIME;
		if (!strncmp (texname + 1, "tele", 4))
			return TEXTYPE_TELE;
		return TEXTYPE_WATER;
	}

	if (texname[0] == '{')
		return TEXTYPE_CUTOUT;

	if (!q_strncasecmp (texname, "sky", 3))
		return TEXTYPE_SKY;

	return TEXTYPE_DEFAULT;
}

/*
=================
Mod_ParseTextures
=================
*/
void Mod_ParseTextures (qmodel_t *mod, byte *mod_base, lump_t *l, wad_t *wads)
{
	int		   i, j, pixels;
	miptex_t   mt;
	texture_t *tx;
	byte	  *m;
	byte	  *pixels_p;
	int		   nummiptex;
	int		   dataofs;
#ifdef BSP29_VALVE
	qboolean	   pal;
	unsigned short colors;
#endif

	// johnfitz -- don't return early if no textures; still need to create dummy texture
	if (!l->filelen)
	{
		Con_Printf ("Mod_LoadTextures: no textures in bsp file\n");
		nummiptex = 0;
		m = NULL; // avoid bogus compiler warning
	}
	else
	{
		m = mod_base + l->fileofs;
		nummiptex = ReadLongUnaligned (m + offsetof (dmiptexlump_t, nummiptex));
	}
	// johnfitz

	mod->numtextures = nummiptex + 2; // johnfitz -- need 2 dummy texture chains for missing textures
	mod->textures = (texture_t **)Mem_Alloc (mod->numtextures * sizeof (*mod->textures));

#ifdef BSP29_VALVE
	pal = mod->bspversion == BSPVERSION_VALVE;
#endif

	for (i = 0; i < nummiptex; i++)
	{
		dataofs = ReadLongUnaligned (m + offsetof (dmiptexlump_t, dataofs[i]));
		if (dataofs == -1)
			continue;
		memcpy (&mt, m + dataofs, sizeof (miptex_t));
		mt.width = LittleLong (mt.width);
		mt.height = LittleLong (mt.height);
		for (j = 0; j < MIPLEVELS; j++)
			mt.offsets[j] = LittleLong (mt.offsets[j]);

		if (mt.width == 0 || mt.height == 0)
		{
			Con_Warning ("Zero sized texture %s in %s!\n", mt.name, mod->name);
			continue;
		}

		// an offset of zero indicates an external texture
		if (mt.offsets[0] == 0)
		{
			mod->textures[i] = Mod_LoadWadTexture (mod, wads, mt.name);
			// Mod_LoadWadTexture trust the .wad name in bsp, but its loading may
			//  fail anyway, so try with regular internal .bsp texture loading as fallback:
			if (mod->textures[i])
			{
				// external texture loading success, skip the regular internal .bsp texture loading below:
				continue;
			}
		}

		pixels = mt.width * mt.height / 64 * 85;
		pixels_p = m + dataofs + sizeof (miptex_t);
#ifdef BSP29_VALVE
		// valve textures have a color palette immediately following the pixels
		if (pal)
		{
			if ((pixels_p + pixels + 2) <= (mod_base + l->fileofs + l->filelen))
			{
				// the palette is basically garunteed to be 256 colors but,
				// we might as well use the value since it *does* exist
				memcpy (&colors, pixels_p + pixels, 2);
				colors = LittleShort (colors);
				// add space for the color palette
				pixels += colors * 3;
			}
			// add space for the color count
			pixels += 2;
		}
#endif
		tx = (texture_t *)Mem_Alloc (sizeof (texture_t) + pixels);
		mod->textures[i] = tx;

		memcpy (tx->name, mt.name, sizeof (tx->name));
		tx->width = mt.width;
		tx->height = mt.height;
		tx->type = Mod_TextureTypeFromName (tx->name);
		for (j = 0; j < MIPLEVELS; j++)
			tx->offsets[j] = mt.offsets[j] + sizeof (texture_t) - sizeof (miptex_t);
		// the pixels immediately follow the structures

		// ericw -- check for pixels extending past the end of the lump.
		// appears in the wild; e.g. jam2_tronyn.bsp (func_mapjam2),
		// kellbase1.bsp (quoth), and can lead to a segfault if we read past
		// the end of the .bsp file buffer
		if ((pixels_p + pixels) > (mod_base + l->fileofs + l->filelen))
		{
			Con_DPrintf ("Texture %s extends past end of lump\n", mt.name);
			pixels = q_max (0, (mod_base + l->fileofs + l->filelen) - pixels_p);
		}
		q_strlcpy (tx->source_file, mod->name, sizeof (tx->source_file));
		tx->source_offset = (src_offset_t)pixels_p - (src_offset_t)mod_base;

		Atomic_StoreUInt32 (&tx->update_warp, false); // johnfitz
		tx->warpimage = NULL;						  // johnfitz
		tx->fullbright = NULL;						  // johnfitz
		tx->shift = 0;								  // Q64 only
#ifdef BSP29_VALVE
		tx->palette = pal;
#else
		tx->palette = false;
#endif

		if (mod->bspversion != BSPVERSION_QUAKE64)
		{
			memcpy (tx + 1, pixels_p, pixels);
		}
		else
		{ // Q64 bsp
			tx->shift = ReadLongUnaligned (m + dataofs + offsetof (miptex64_t, shift));
			memcpy (tx + 1, m + dataofs + sizeof (miptex64_t), pixels);
		}
	}
}

/*
=================
Mod_LoadLighting -- johnfitz -- replaced with lit support code via lordhavoc
=================
*/
void Mod_LoadLighting (qmodel_t *mod, byte *mod_base, lump_t *l)
{
	int			 i;
	byte		*in, *out, *data;
	byte		 d, q64_b0, q64_b1;
	char		 litfilename[MAX_OSPATH];
	unsigned int path_id;

	mod->lightdata = NULL;
	// LordHavoc: check for a .lit file
	q_strlcpy (litfilename, mod->name, sizeof (litfilename));
	COM_StripExtension (litfilename, litfilename, sizeof (litfilename));
	q_strlcat (litfilename, ".lit", sizeof (litfilename));
	data = (byte *)COM_LoadFile (litfilename, &path_id);
	if (data)
	{
		// use lit file only from the same gamedir as the map
		// itself or from a searchpath with higher priority.
		if (path_id < mod->path_id)
		{
			Con_DPrintf ("ignored %s from a gamedir with lower priority\n", litfilename);
		}
		else if (data[0] == 'Q' && data[1] == 'L' && data[2] == 'I' && data[3] == 'T')
		{
			i = ReadLongUnaligned (data + sizeof (int));
			if (i == 1)
			{
				if (8 + l->filelen * 3 == com_filesize)
				{
					Con_DPrintf2 ("%s loaded\n", litfilename);
					mod->lightdata = (byte *)Mem_AllocNonZero (l->filelen * 3);
					memcpy (mod->lightdata, data + 8, l->filelen * 3);
					Mem_Free (data);
					return;
				}
				Con_Printf ("Outdated .lit file (%s should be %u bytes, not %lld)\n", litfilename, 8 + l->filelen * 3, com_filesize);
			}
			else
			{
				Con_Printf ("Unknown .lit file version (%d)\n", i);
			}
		}
		else
		{
			Con_Printf ("Corrupt .lit file (old version?), ignoring\n");
		}

		Mem_Free (data);
	}
	// LordHavoc: no .lit found, expand the white lighting data to color
	if (!l->filelen)
		return;

	// Quake64 bsp lighmap data
	if (mod->bspversion == BSPVERSION_QUAKE64)
	{
		// RGB lightmap samples are packed in 16bits.
		// RRRRR GGGGG BBBBBB

		mod->lightdata = (byte *)Mem_Alloc ((l->filelen / 2) * 3);
		in = mod_base + l->fileofs;
		out = mod->lightdata;

		for (i = 0; i < (l->filelen / 2); i++)
		{
			q64_b0 = *in++;
			q64_b1 = *in++;

			*out++ = q64_b0 & 0xf8;									  /* 0b11111000 */
			*out++ = ((q64_b0 & 0x07) << 5) + ((q64_b1 & 0xc0) >> 5); /* 0b00000111, 0b11000000 */
			*out++ = (q64_b1 & 0x3f) << 2;							  /* 0b00111111 */
		}
		return;
	}

#ifdef BSP29_VALVE
	if (mod->bspversion == BSPVERSION_VALVE)
	{
		// lightmap samples are already stored as rgb
		mod->lightdata = (byte *)Mem_Alloc (l->filelen);
		memcpy (mod->lightdata, mod_base + l->fileofs, l->filelen);
		return;
	}
#endif

	mod->lightdata = (byte *)Mem_Alloc (l->filelen * 3);
	in = mod->lightdata + l->filelen * 2; // place the file at the end, so it will not be overwritten until the very last write
	out = mod->lightdata;
	memcpy (in, mod_base + l->fileofs, l->filelen);
	for (i = 0; i < l->filelen; i++)
	{
		d = *in++;
		*out++ = d;
		*out++ = d;
		*out++ = d;
	}
}

/*
=================
Mod_LoadVisibility
=================
*/
void Mod_LoadVisibility (qmodel_t *mod, byte *mod_base, lump_t *l)
{
	mod->viswarn = false;
	if (!l->filelen)
	{
		mod->visdata = NULL;
		return;
	}
	mod->visdata = (byte *)Mem_Alloc (l->filelen);
	memcpy (mod->visdata, mod_base + l->fileofs, l->filelen);
}

/*
=================
Mod_LoadEntities
=================
*/
void Mod_LoadEntities (qmodel_t *mod, byte *mod_base, lump_t *l)
{
	char		 basemapname[MAX_QPATH];
	char		 entfilename[MAX_QPATH];
	char		*ents = NULL;
	unsigned int path_id;
	unsigned int crc = 0;
	qboolean	 versioned = true;

	if (!external_ents.value)
		goto _load_embedded;

	if (l->filelen > 0)
	{
		crc = CRC_Block (mod_base + l->fileofs, l->filelen - 1);
	}

	q_strlcpy (basemapname, mod->name, sizeof (basemapname));
	COM_StripExtension (basemapname, basemapname, sizeof (basemapname));

	q_snprintf (entfilename, sizeof (entfilename), "%s@%04x.ent", basemapname, crc);
	Con_DPrintf2 ("trying to load %s\n", entfilename);
	ents = (char *)COM_LoadFile (entfilename, &path_id);

	if (!ents)
	{
		q_snprintf (entfilename, sizeof (entfilename), "%s.ent", basemapname);
		Con_DPrintf2 ("trying to load %s\n", entfilename);
		ents = (char *)COM_LoadFile (entfilename, &path_id);
		versioned = false;
	}

	if (ents)
	{
		// use ent file only from the same gamedir as the map
		// itself or from a searchpath with higher priority
		// unless we got a CRC match
		if (versioned == false && path_id < mod->path_id)
		{
			Con_DPrintf ("ignored %s from a gamedir with lower priority\n", entfilename);
		}
		else
		{
			mod->entities = ents;
			Con_DPrintf ("Loaded external entity file %s\n", entfilename);
			return;
		}
	}

_load_embedded:
	if (!l->filelen)
	{
		Mem_Free (mod->entities);
		mod->entities = NULL;
		return;
	}
	// The BSP entity lump is a text-based lump intended to be read
	// by COM_Parse(), which expects a valid null-terminated string.
	// However l->filelen is the character (byte) length of the lump
	// not including the null-character, which, as it happens, can be absent from the BSP
	// in some cases.
	// To properly terminate the text-blob and prevent buffer overflows in COM_Parse, over-allocate + 1 byte
	// using Mem_Alloc (which 0-initialize)
	// The external .ent files are safe because COM_LoadFile also overallocate
	// with a 0-byte at the end by default.
	mod->entities = (char *)Mem_Alloc (l->filelen + 1);
	memcpy (mod->entities, mod_base + l->fileofs, l->filelen);
	Mem_Free (ents);
}

/*
=================
Mod_LoadVertexes
=================
*/
void Mod_LoadVertexes (qmodel_t *mod, byte *mod_base, lump_t *l)
{
	byte	  *in;
	mvertex_t *out;
	int		   i, count;

	in = mod_base + l->fileofs;
	if (l->filelen % sizeof (dvertex_t))
		Sys_Error ("MOD_LoadBmodel: funny lump size in %s", mod->name);
	count = l->filelen / sizeof (dvertex_t);
	out = (mvertex_t *)Mem_Alloc (count * sizeof (*out));

	mod->vertexes = out;
	mod->numvertexes = count;

	for (i = 0; i < count; i++, in += sizeof (dvertex_t), out++)
	{
		out->position[0] = ReadFloatUnaligned (in + offsetof (dvertex_t, point[0]));
		out->position[1] = ReadFloatUnaligned (in + offsetof (dvertex_t, point[1]));
		out->position[2] = ReadFloatUnaligned (in + offsetof (dvertex_t, point[2]));
	}
}

/*
=================
Mod_LoadEdges
=================
*/
void Mod_LoadEdges (qmodel_t *mod, byte *mod_base, lump_t *l, int bsp2)
{
	medge_t *out;
	int		 i, count;

	if (bsp2)
	{
		byte *in = mod_base + l->fileofs;

		if (l->filelen % sizeof (dledge_t))
			Sys_Error ("MOD_LoadBmodel: funny lump size in %s", mod->name);

		count = l->filelen / sizeof (dledge_t);
		out = (medge_t *)Mem_Alloc ((count + 1) * sizeof (*out));

		mod->edges = out;
		mod->numedges = count;

		for (i = 0; i < count; i++, in += sizeof (dledge_t), out++)
		{
			out->v[0] = ReadLongUnaligned (in + offsetof (dledge_t, v[0]));
			out->v[1] = ReadLongUnaligned (in + offsetof (dledge_t, v[1]));
		}
	}
	else
	{
		byte *in = mod_base + l->fileofs;

		if (l->filelen % sizeof (dsedge_t))
			Sys_Error ("MOD_LoadBmodel: funny lump size in %s", mod->name);

		count = l->filelen / sizeof (dsedge_t);
		out = (medge_t *)Mem_Alloc ((count + 1) * sizeof (*out));

		mod->edges = out;
		mod->numedges = count;

		for (i = 0; i < count; i++, in += sizeof (dsedge_t), out++)
		{
			out->v[0] = (unsigned short)ReadShortUnaligned (in + offsetof (dsedge_t, v[0]));
			out->v[1] = (unsigned short)ReadShortUnaligned (in + offsetof (dsedge_t, v[1]));
		}
	}
}

/*
=================
Mod_LoadTexinfo
=================
*/
void Mod_LoadTexinfo (qmodel_t *mod, byte *mod_base, lump_t *l)
{
	byte	   *in;
	mtexinfo_t *out;
	int			i, j, count, miptex;
	int			missing = 0; // johnfitz

	in = mod_base + l->fileofs;
	if (l->filelen % sizeof (texinfo_t))
		Sys_Error ("MOD_LoadBmodel: funny lump size in %s", mod->name);
	count = l->filelen / sizeof (texinfo_t);
	out = (mtexinfo_t *)Mem_Alloc (count * sizeof (*out));

	mod->texinfo = out;
	mod->numtexinfo = count;

	for (i = 0; i < count; i++, in += sizeof (texinfo_t), out++)
	{
		for (j = 0; j < 4; j++)
		{
			out->vecs[0][j] = ReadFloatUnaligned (in + offsetof (texinfo_t, vecs[0][j]));
			out->vecs[1][j] = ReadFloatUnaligned (in + offsetof (texinfo_t, vecs[1][j]));
		}

		miptex = ReadLongUnaligned (in + offsetof (texinfo_t, miptex));
		out->flags = ReadLongUnaligned (in + offsetof (texinfo_t, flags));

		// johnfitz -- rewrote this section
		if (miptex >= mod->numtextures - 1 || !mod->textures[miptex])
		{
			if (out->flags & TEX_SPECIAL)
				out->texture = mod->textures[mod->numtextures - 1];
			else
				out->texture = mod->textures[mod->numtextures - 2];
			out->flags |= TEX_MISSING;
			missing++;
			out->tex_idx = -1;
		}
		else
		{
			out->texture = mod->textures[miptex];
			out->tex_idx = miptex;
		}
		// johnfitz
	}

	// johnfitz: report missing textures
	if (missing && mod->numtextures > 1)
		Con_Printf ("Mod_LoadTexinfo: %d texture(s) missing from BSP file\n", missing);
	// johnfitz
}

/*
================
CalcSurfaceExtents

Fills in s->texturemins[] and s->extents[]
================
*/
void CalcSurfaceExtents (qmodel_t *mod, msurface_t *s)
{
	float		mins[2], maxs[2], val;
	int			i, j, e;
	mvertex_t  *v;
	mtexinfo_t *tex;
	int			bmins[2], bmaxs[2];

	mins[0] = mins[1] = FLT_MAX;
	maxs[0] = maxs[1] = -FLT_MAX;

	tex = s->texinfo;

	const double tex_vecs[2][4] = {
		{tex->vecs[0][0], tex->vecs[0][1], tex->vecs[0][2], tex->vecs[0][3]},
		{tex->vecs[1][0], tex->vecs[1][1], tex->vecs[1][2], tex->vecs[1][3]},
	};

	for (i = 0; i < s->numedges; i++)
	{
		e = mod->surfedges[s->firstedge + i];
		if (e >= 0)
			v = &mod->vertexes[mod->edges[e].v[0]];
		else
			v = &mod->vertexes[mod->edges[-e].v[1]];

		for (j = 0; j < 2; j++)
		{
			/* The following calculation is sensitive to floating-point
			 * precision.  It needs to produce the same result that the
			 * light compiler does, because R_BuildLightMap uses surf->
			 * extents to know the width/height of a surface's lightmap,
			 * and incorrect rounding here manifests itself as patches
			 * of "corrupted" looking lightmaps.
			 * Most light compilers are win32 executables, so they use
			 * x87 floating point.  This means the multiplies and adds
			 * are done at 80-bit precision, and the result is rounded
			 * down to 32-bits and stored in val.
			 * Adding the casts to double seems to be good enough to fix
			 * lighting glitches when Quakespasm is compiled as x86_64
			 * and using SSE2 floating-point.  A potential trouble spot
			 * is the hallway at the beginning of mfxsp17.  -- ericw
			 */
			val = ((double)v->position[0] * tex_vecs[j][0]) + ((double)v->position[1] * tex_vecs[j][1]) + ((double)v->position[2] * tex_vecs[j][2]) +
				  tex_vecs[j][3];

			mins[j] = q_min (mins[j], val);
			maxs[j] = q_max (maxs[j], val);
		}
	}

	for (i = 0; i < 2; i++)
	{
		bmins[i] = floor (mins[i] / 16);
		bmaxs[i] = ceil (maxs[i] / 16);

		s->texturemins[i] = bmins[i] * 16;
		s->extents[i] = (bmaxs[i] - bmins[i]) * 16;

		if (!(tex->flags & TEX_SPECIAL) && s->extents[i] > 2000) // johnfitz -- was 512 in glquake, 256 in winquake
			Sys_Error ("Bad surface extents");
	}
}

/*
=================
Mod_ParseFaces
=================
*/
void Mod_ParseFaces (qmodel_t *mod, byte *mod_base, lump_t *l, qboolean bsp2)
{
	byte	   *ins;
	byte	   *inl;
	msurface_t *out;
	int			i, count, surfnum, lofs;
	int			planenum, side, texinfon;

	if (bsp2)
	{
		ins = NULL;
		inl = mod_base + l->fileofs;
		if (l->filelen % sizeof (dlface_t))
			Sys_Error ("MOD_LoadBmodel: funny lump size in %s", mod->name);
		count = l->filelen / sizeof (dlface_t);
	}
	else
	{
		ins = mod_base + l->fileofs;
		inl = NULL;
		if (l->filelen % sizeof (dsface_t))
			Sys_Error ("MOD_LoadBmodel: funny lump size in %s", mod->name);
		count = l->filelen / sizeof (dsface_t);
	}
	out = (msurface_t *)Mem_AllocNonZero (count * sizeof (*out));

	// johnfitz -- warn mappers about exceeding old limits
	if (count > 32767 && !bsp2)
		Con_DWarning ("%i faces exceeds standard limit of 32767.\n", count);
	// johnfitz

	mod->surfaces = out;
	mod->numsurfaces = count;

	for (surfnum = 0; surfnum < count; surfnum++, out++)
	{
		out->styles_bitmap = 0; // OR-ed into below; the allocation is non-zeroing
		if (bsp2)
		{
			out->firstedge = ReadLongUnaligned (inl + offsetof (dlface_t, firstedge));
			out->numedges = ReadLongUnaligned (inl + offsetof (dlface_t, numedges));
			planenum = ReadLongUnaligned (inl + offsetof (dlface_t, planenum));
			side = ReadLongUnaligned (inl + offsetof (dlface_t, side));
			texinfon = ReadLongUnaligned (inl + offsetof (dlface_t, texinfo));
			for (i = 0; i < MAXLIGHTMAPS; i++)
			{
				out->styles[i] = *(inl + offsetof (dlface_t, styles[i]));
				if (out->styles[i] >= MAX_LIGHTSTYLES && out->styles[i] != 255)
				{
					Con_Warning ("Invalid lightstyle %d\n", out->styles[i]);
					out->styles[i] = 0;
				}
				byte j = out->styles[i];
				if (j < 255)
					out->styles_bitmap |= 1 << (j < 16 ? j : j % 16 + 16);
			}
			lofs = ReadLongUnaligned (inl + offsetof (dlface_t, lightofs));
			inl += sizeof (dlface_t);
		}
		else
		{
			out->firstedge = ReadLongUnaligned (ins + offsetof (dsface_t, firstedge));
			out->numedges = ReadShortUnaligned (ins + offsetof (dsface_t, numedges));
			planenum = ReadShortUnaligned (ins + offsetof (dsface_t, planenum));
			side = ReadShortUnaligned (ins + offsetof (dsface_t, side));
			texinfon = ReadShortUnaligned (ins + offsetof (dsface_t, texinfo));
			for (i = 0; i < MAXLIGHTMAPS; i++)
			{
				out->styles[i] = *(ins + offsetof (dsface_t, styles[i]));
				if (out->styles[i] >= MAX_LIGHTSTYLES && out->styles[i] != 255)
				{
					Con_Warning ("Invalid lightstyle %d\n", out->styles[i]);
					out->styles[i] = 0;
				}
				byte j = out->styles[i];
				if (j < 255)
					out->styles_bitmap |= 1 << (j < 16 ? j : j % 16 + 16);
			}
			lofs = ReadLongUnaligned (ins + offsetof (dsface_t, lightofs));
			ins += sizeof (dsface_t);
		}

		if (!out->styles_bitmap)
			out->styles_bitmap = 1;

		out->flags = 0;
		out->polys = NULL;

		if (side)
			out->flags |= SURF_PLANEBACK;

		out->plane = mod->planes + planenum;

		out->texinfo = mod->texinfo + texinfon;

		// lighting info
		if (mod->bspversion == BSPVERSION_QUAKE64)
			lofs /= 2; // Q64 samples are 16bits instead 8 in normal Quake

		if (lofs == -1)
			out->samples = NULL;
#ifdef BSP29_VALVE
		else if (mod->bspversion == BSPVERSION_VALVE)
			out->samples = mod->lightdata + lofs; // accounts for RGB light data
#endif
		else
			out->samples = mod->lightdata + (lofs * 3); // johnfitz -- lit support via lordhavoc (was "+ i")

		// johnfitz -- this section rewritten
		out->lightmaptexturenum = -1;
		if (out->texinfo->texture->type == TEXTYPE_SKY) // sky surface //also note -- was strncmp, changed to match qbsp
		{
			out->flags |= (SURF_DRAWSKY | SURF_DRAWTILED);
		}
		else if (TEXTYPE_ISLIQUID (out->texinfo->texture->type)) // warp surface
		{
			out->flags |= SURF_DRAWTURB;

			if (out->texinfo->flags & TEX_SPECIAL)
				out->flags |= SURF_DRAWTILED; // unlit water

			// detect special liquid types
			if (out->texinfo->texture->type == TEXTYPE_LAVA)
				out->flags |= SURF_DRAWLAVA;
			else if (out->texinfo->texture->type == TEXTYPE_SLIME)
				out->flags |= SURF_DRAWSLIME;
			else if (out->texinfo->texture->type == TEXTYPE_TELE)
				out->flags |= SURF_DRAWTELE;
			else
				out->flags |= SURF_DRAWWATER;
		}
		else if (out->texinfo->texture->type == TEXTYPE_CUTOUT) // ericw -- fence textures
		{
			out->flags |= SURF_DRAWFENCE;
		}
		else if (out->texinfo->flags & TEX_MISSING) // texture is missing from bsp
		{
			out->flags |= SURF_NOTEXTURE;
			qboolean missing_samples = !out->samples && out->styles[0] != 255;
			qboolean unlit_texture = out->texinfo->flags & TEX_SPECIAL;

			if (!unlit_texture && missing_samples)
			{
				// unlit surf in a lit texture (mod->numtextures - 2: r_notexture_mip instead of r_notexture_mip2)
				Con_Warning ("Mod_LoadFaces: TEX_MISSING without TEX_SPECIAL missing lightmap samples");
				out->lightmaptexturenum = 0; // set a lightmaptexturenum to at least avoid a crash
			}

			if (unlit_texture || missing_samples) // not lightmapped
			{
				out->flags |= SURF_DRAWTILED;
			}
		}
		// johnfitz
	}
}

/*
=================
Mod_LoadNodes
=================
*/
static void Mod_LoadNodes_S (qmodel_t *mod, byte *mod_base, lump_t *l)
{
	int		 i, j, count, p;
	byte	*in;
	mnode_t *out;

	in = mod_base + l->fileofs;
	if (l->filelen % sizeof (dsnode_t))
		Sys_Error ("MOD_LoadBmodel: funny lump size in %s", mod->name);
	count = l->filelen / sizeof (dsnode_t);
	out = (mnode_t *)Mem_Alloc (count * sizeof (*out));

	// johnfitz -- warn mappers about exceeding old limits
	if (count > 32767)
		Con_DWarning ("%i nodes exceeds standard limit of 32767.\n", count);
	// johnfitz

	mod->nodes = out;
	mod->numnodes = count;

	for (i = 0; i < count; i++, in += sizeof (dsnode_t), out++)
	{
		for (j = 0; j < 3; j++)
		{
			out->minmaxs[j] = ReadShortUnaligned (in + offsetof (dsnode_t, mins[j]));
			out->minmaxs[3 + j] = ReadShortUnaligned (in + offsetof (dsnode_t, maxs[j]));
		}

		p = ReadLongUnaligned (in + offsetof (dsnode_t, planenum));
		out->plane = mod->planes + p;

		out->firstsurface = (unsigned short)ReadShortUnaligned (in + offsetof (dsnode_t, firstface)); // johnfitz -- explicit cast as unsigned short
		out->numsurfaces = (unsigned short)ReadShortUnaligned (in + offsetof (dsnode_t, numfaces));	  // johnfitz -- explicit cast as unsigned short

		for (j = 0; j < 2; j++)
		{
			// johnfitz -- hack to handle nodes > 32k, adapted from darkplaces
			p = (unsigned short)ReadShortUnaligned (in + offsetof (dsnode_t, children[j]));
			if (p < count)
				out->children[j] = mod->nodes + p;
			else
			{
				p = 65535 - p; // note this uses 65535 intentionally, -1 is leaf 0
				if (p < mod->numleafs)
					out->children[j] = (mnode_t *)(mod->leafs + p);
				else
				{
					Con_Printf ("Mod_LoadNodes: invalid leaf index %i (file has only %i leafs)\n", p, mod->numleafs);
					out->children[j] = (mnode_t *)(mod->leafs); // map it to the solid leaf
				}
			}
			// johnfitz
		}
	}
}

static void Mod_LoadNodes_L1 (qmodel_t *mod, byte *mod_base, lump_t *l)
{
	int		 i, j, count, p;
	byte	*in;
	mnode_t *out;

	in = mod_base + l->fileofs;
	if (l->filelen % sizeof (dl1node_t))
		Sys_Error ("Mod_LoadNodes: funny lump size in %s", mod->name);

	count = l->filelen / sizeof (dl1node_t);
	out = (mnode_t *)Mem_Alloc (count * sizeof (*out));

	mod->nodes = out;
	mod->numnodes = count;

	for (i = 0; i < count; i++, in += sizeof (dl1node_t), out++)
	{
		for (j = 0; j < 3; j++)
		{
			out->minmaxs[j] = ReadShortUnaligned (in + offsetof (dl1node_t, mins[j]));
			out->minmaxs[3 + j] = ReadShortUnaligned (in + offsetof (dl1node_t, maxs[j]));
		}

		p = ReadLongUnaligned (in + offsetof (dl1node_t, planenum));
		out->plane = mod->planes + p;

		out->firstsurface = ReadLongUnaligned (in + offsetof (dl1node_t, firstface)); // johnfitz -- explicit cast as unsigned short
		out->numsurfaces = ReadLongUnaligned (in + offsetof (dl1node_t, numfaces));	  // johnfitz -- explicit cast as unsigned short

		for (j = 0; j < 2; j++)
		{
			// johnfitz -- hack to handle nodes > 32k, adapted from darkplaces
			p = ReadLongUnaligned (in + offsetof (dl1node_t, children[j]));
			if (p >= 0 && p < count)
				out->children[j] = mod->nodes + p;
			else
			{
				p = 0xffffffff - p; // note this uses 65535 intentionally, -1 is leaf 0
				if (p >= 0 && p < mod->numleafs)
					out->children[j] = (mnode_t *)(mod->leafs + p);
				else
				{
					Con_Printf ("Mod_LoadNodes: invalid leaf index %i (file has only %i leafs)\n", p, mod->numleafs);
					out->children[j] = (mnode_t *)(mod->leafs); // map it to the solid leaf
				}
			}
			// johnfitz
		}
	}
}

static void Mod_LoadNodes_L2 (qmodel_t *mod, byte *mod_base, lump_t *l)
{
	int		 i, j, count, p;
	byte	*in;
	mnode_t *out;

	in = mod_base + l->fileofs;
	if (l->filelen % sizeof (dl2node_t))
		Sys_Error ("Mod_LoadNodes: funny lump size in %s", mod->name);

	count = l->filelen / sizeof (dl2node_t);
	out = (mnode_t *)Mem_Alloc (count * sizeof (*out));

	mod->nodes = out;
	mod->numnodes = count;

	for (i = 0; i < count; i++, in += sizeof (dl2node_t), out++)
	{
		for (j = 0; j < 3; j++)
		{
			out->minmaxs[j] = ReadFloatUnaligned (in + offsetof (dl2node_t, mins[j]));
			out->minmaxs[3 + j] = ReadFloatUnaligned (in + offsetof (dl2node_t, maxs[j]));
		}

		p = ReadLongUnaligned (in + offsetof (dl2node_t, planenum));
		out->plane = mod->planes + p;

		out->firstsurface = ReadLongUnaligned (in + offsetof (dl2node_t, firstface)); // johnfitz -- explicit cast as unsigned short
		out->numsurfaces = ReadLongUnaligned (in + offsetof (dl2node_t, numfaces));	  // johnfitz -- explicit cast as unsigned short

		for (j = 0; j < 2; j++)
		{
			// johnfitz -- hack to handle nodes > 32k, adapted from darkplaces
			p = ReadLongUnaligned (in + offsetof (dl2node_t, children[j]));
			if (p > 0 && p < count)
				out->children[j] = mod->nodes + p;
			else
			{
				p = 0xffffffff - p; // note this uses 65535 intentionally, -1 is leaf 0
				if (p >= 0 && p < mod->numleafs)
					out->children[j] = (mnode_t *)(mod->leafs + p);
				else
				{
					Con_Printf ("Mod_LoadNodes: invalid leaf index %i (file has only %i leafs)\n", p, mod->numleafs);
					out->children[j] = (mnode_t *)(mod->leafs); // map it to the solid leaf
				}
			}
			// johnfitz
		}
	}
}

void Mod_LoadNodes (qmodel_t *mod, byte *mod_base, lump_t *l, int bsp2)
{
	if (bsp2 == 2)
		Mod_LoadNodes_L2 (mod, mod_base, l);
	else if (bsp2)
		Mod_LoadNodes_L1 (mod, mod_base, l);
	else
		Mod_LoadNodes_S (mod, mod_base, l);
}

static void Mod_ProcessLeafs_S (qmodel_t *mod, byte *in, int filelen)
{
	mleaf_t *out;
	int		 i, j, count, p;

	if (filelen % sizeof (dsleaf_t))
		Sys_Error ("Mod_ProcessLeafs: funny lump size in %s", mod->name);
	count = filelen / sizeof (dsleaf_t);
	out = (mleaf_t *)Mem_Alloc (count * sizeof (*out));

	// johnfitz
	if (count > 32767)
		Host_Error ("Mod_LoadLeafs: %i leafs exceeds limit of 32767.", count);
	// johnfitz

	mod->leafs = out;
	mod->numleafs = count;

	for (i = 0; i < count; i++, in += sizeof (dsleaf_t), out++)
	{
		for (j = 0; j < 3; j++)
		{
			out->minmaxs[j] = ReadShortUnaligned (in + offsetof (dsleaf_t, mins[j]));
			out->minmaxs[3 + j] = ReadShortUnaligned (in + offsetof (dsleaf_t, maxs[j]));
		}

		p = ReadLongUnaligned (in + offsetof (dsleaf_t, contents));
		out->contents = p;

		out->firstmarksurface =
			mod->marksurfaces + (unsigned short)ReadShortUnaligned (in + offsetof (dsleaf_t, firstmarksurface)); // johnfitz -- unsigned short
		out->nummarksurfaces = (unsigned short)ReadShortUnaligned (in + offsetof (dsleaf_t, nummarksurfaces));	 // johnfitz -- unsigned short

		p = ReadLongUnaligned (in + offsetof (dsleaf_t, visofs));
		if (p == -1)
			out->compressed_vis = NULL;
		else
			out->compressed_vis = (mod->visdata != NULL) ? (mod->visdata + p) : NULL;
		out->efrags = NULL;

		for (j = 0; j < 4; j++)
			out->ambient_sound_level[j] = *(in + offsetof (dsleaf_t, ambient_level[j]));

		// johnfitz -- removed code to mark surfaces as SURF_UNDERWATER
	}
}

static void Mod_ProcessLeafs_L1 (qmodel_t *mod, byte *in, int filelen)
{
	mleaf_t *out;
	int		 i, j, count, p;

	if (filelen % sizeof (dl1leaf_t))
		Sys_Error ("Mod_ProcessLeafs: funny lump size in %s", mod->name);

	count = filelen / sizeof (dl1leaf_t);

	out = (mleaf_t *)Mem_Alloc (count * sizeof (*out));

	mod->leafs = out;
	mod->numleafs = count;

	for (i = 0; i < count; i++, in += sizeof (dl1leaf_t), out++)
	{
		for (j = 0; j < 3; j++)
		{
			out->minmaxs[j] = ReadShortUnaligned (in + offsetof (dl1leaf_t, mins[j]));
			out->minmaxs[3 + j] = ReadShortUnaligned (in + offsetof (dl1leaf_t, maxs[j]));
		}

		p = ReadLongUnaligned (in + offsetof (dl1leaf_t, contents));
		out->contents = p;

		out->firstmarksurface = mod->marksurfaces + ReadLongUnaligned (in + offsetof (dl1leaf_t, firstmarksurface)); // johnfitz -- unsigned short
		out->nummarksurfaces = ReadLongUnaligned (in + offsetof (dl1leaf_t, nummarksurfaces));						 // johnfitz -- unsigned short

		p = ReadLongUnaligned (in + offsetof (dl1leaf_t, visofs));
		if (p == -1)
			out->compressed_vis = NULL;
		else
			out->compressed_vis = mod->visdata + p;
		out->efrags = NULL;

		for (j = 0; j < 4; j++)
			out->ambient_sound_level[j] = *(in + offsetof (dl1leaf_t, ambient_level[j]));

		// johnfitz -- removed code to mark surfaces as SURF_UNDERWATER
	}
}

static void Mod_ProcessLeafs_L2 (qmodel_t *mod, byte *in, int filelen)
{
	mleaf_t *out;
	int		 i, j, count, p;

	if (filelen % sizeof (dl2leaf_t))
		Sys_Error ("Mod_ProcessLeafs: funny lump size in %s", mod->name);

	count = filelen / sizeof (dl2leaf_t);

	out = (mleaf_t *)Mem_Alloc (count * sizeof (*out));

	mod->leafs = out;
	mod->numleafs = count;

	for (i = 0; i < count; i++, in += sizeof (dl2leaf_t), out++)
	{
		for (j = 0; j < 3; j++)
		{
			out->minmaxs[j] = ReadFloatUnaligned (in + offsetof (dl2leaf_t, mins[j]));
			out->minmaxs[3 + j] = ReadFloatUnaligned (in + offsetof (dl2leaf_t, maxs[j]));
		}

		p = ReadLongUnaligned (in + offsetof (dl2leaf_t, contents));
		out->contents = p;

		out->firstmarksurface = mod->marksurfaces + ReadLongUnaligned (in + offsetof (dl2leaf_t, firstmarksurface)); // johnfitz -- unsigned short
		out->nummarksurfaces = ReadLongUnaligned (in + offsetof (dl2leaf_t, nummarksurfaces));						 // johnfitz -- unsigned short

		p = ReadLongUnaligned (in + offsetof (dl2leaf_t, visofs));
		if (p == -1)
			out->compressed_vis = NULL;
		else
			out->compressed_vis = mod->visdata + p;
		out->efrags = NULL;

		for (j = 0; j < 4; j++)
			out->ambient_sound_level[j] = *(in + offsetof (dl2leaf_t, ambient_level[j]));

		// johnfitz -- removed code to mark surfaces as SURF_UNDERWATER
	}
}

/*
=================
Mod_LoadLeafs
=================
*/
void Mod_LoadLeafs (qmodel_t *mod, byte *mod_base, lump_t *l, int bsp2)
{
	void *in = (void *)(mod_base + l->fileofs);

	if (bsp2 == 2)
		Mod_ProcessLeafs_L2 (mod, in, l->filelen);
	else if (bsp2)
		Mod_ProcessLeafs_L1 (mod, in, l->filelen);
	else
		Mod_ProcessLeafs_S (mod, in, l->filelen);
}

/*
=================
Mod_LoadClipnodes
=================
*/
void Mod_LoadClipnodes (qmodel_t *mod, byte *mod_base, lump_t *l, qboolean bsp2)
{
	byte *ins;
	byte *inl;

	mclipnode_t *out; // johnfitz -- was dclipnode_t
	int			 i, count;
	hull_t		*hull;

	if (bsp2)
	{
		ins = NULL;
		inl = mod_base + l->fileofs;
		if (l->filelen % sizeof (dlclipnode_t))
			Sys_Error ("Mod_LoadClipnodes: funny lump size in %s", mod->name);

		count = l->filelen / sizeof (dlclipnode_t);
	}
	else
	{
		ins = mod_base + l->fileofs;
		inl = NULL;
		if (l->filelen % sizeof (dsclipnode_t))
			Sys_Error ("Mod_LoadClipnodes: funny lump size in %s", mod->name);

		count = l->filelen / sizeof (dsclipnode_t);
	}
	out = (mclipnode_t *)Mem_Alloc (count * sizeof (*out));

	// johnfitz -- warn about exceeding old limits
	if (count > 32767 && !bsp2)
		Con_DWarning ("%i clipnodes exceeds standard limit of 32767.\n", count);
	// johnfitz

	mod->clipnodes = out;
	mod->numclipnodes = count;

	hull = &mod->hulls[1];
	hull->clipnodes = out;
	hull->firstclipnode = 0;
	hull->lastclipnode = count - 1;
	hull->planes = mod->planes;
	hull->clip_mins[0] = -16;
	hull->clip_mins[1] = -16;
	hull->clip_mins[2] = -24;
	hull->clip_maxs[0] = 16;
	hull->clip_maxs[1] = 16;
	hull->clip_maxs[2] = 32;

	hull = &mod->hulls[2];
	hull->clipnodes = out;
	hull->firstclipnode = 0;
	hull->lastclipnode = count - 1;
	hull->planes = mod->planes;
	hull->clip_mins[0] = -32;
	hull->clip_mins[1] = -32;
	hull->clip_mins[2] = -24;
	hull->clip_maxs[0] = 32;
	hull->clip_maxs[1] = 32;
	hull->clip_maxs[2] = 64;

	if (bsp2)
	{
		for (i = 0; i < count; i++, out++, inl += sizeof (dlclipnode_t))
		{
			out->planenum = ReadLongUnaligned (inl + offsetof (dlclipnode_t, planenum));

			// johnfitz -- bounds check
			if (out->planenum < 0 || out->planenum >= mod->numplanes)
				Host_Error ("Mod_LoadClipnodes: planenum out of bounds");
			// johnfitz

			out->children[0] = ReadLongUnaligned (inl + offsetof (dlclipnode_t, children[0]));
			out->children[1] = ReadLongUnaligned (inl + offsetof (dlclipnode_t, children[1]));
			// Spike: FIXME: bounds check
		}
	}
	else
	{
		for (i = 0; i < count; i++, out++, ins += sizeof (dsclipnode_t))
		{
			out->planenum = ReadLongUnaligned (ins + offsetof (dsclipnode_t, planenum));

			// johnfitz -- bounds check
			if (out->planenum < 0 || out->planenum >= mod->numplanes)
				Host_Error ("Mod_LoadClipnodes: planenum out of bounds");
			// johnfitz

			// johnfitz -- support clipnodes > 32k
			out->children[0] = (unsigned short)ReadShortUnaligned (ins + offsetof (dsclipnode_t, children[0]));
			out->children[1] = (unsigned short)ReadShortUnaligned (ins + offsetof (dsclipnode_t, children[1]));

			if (out->children[0] >= count)
				out->children[0] -= 65536;
			if (out->children[1] >= count)
				out->children[1] -= 65536;
			// johnfitz
		}
	}
}

/*
=================
Mod_MakeHull0

Duplicate the drawing hull structure as a clipping hull
=================
*/
void Mod_MakeHull0 (qmodel_t *mod)
{
	mnode_t		*in, *child;
	mclipnode_t *out; // johnfitz -- was dclipnode_t
	int			 i, j, count;
	hull_t		*hull;

	hull = &mod->hulls[0];

	in = mod->nodes;
	count = mod->numnodes;
	out = (mclipnode_t *)Mem_Alloc (count * sizeof (*out));

	hull->clipnodes = out;
	hull->firstclipnode = 0;
	hull->lastclipnode = count - 1;
	hull->planes = mod->planes;

	for (i = 0; i < count; i++, out++, in++)
	{
		out->planenum = in->plane - mod->planes;
		for (j = 0; j < 2; j++)
		{
			child = in->children[j];
			if (child->contents < 0)
				out->children[j] = child->contents;
			else
				out->children[j] = child - mod->nodes;
		}
	}
}

/*
=================
Mod_LoadMarksurfaces
=================
*/
void Mod_LoadMarksurfaces (qmodel_t *mod, byte *mod_base, lump_t *l, int bsp2)
{
	int	 i, j, count;
	int *out;
	if (bsp2)
	{
		byte *in = mod_base + l->fileofs;

		if (l->filelen % sizeof (unsigned int))
			Host_Error ("Mod_LoadMarksurfaces: funny lump size in %s", mod->name);

		count = l->filelen / sizeof (unsigned int);
		out = (int *)Mem_Alloc (count * sizeof (*out));

		mod->marksurfaces = out;
		mod->nummarksurfaces = count;

		for (i = 0; i < count; i++)
		{
			j = ReadLongUnaligned (in + (i * sizeof (int)));
			if (j >= mod->numsurfaces)
				Host_Error ("Mod_LoadMarksurfaces: bad surface number");
			out[i] = j;
		}
	}
	else
	{
		byte *in = mod_base + l->fileofs;

		if (l->filelen % sizeof (short))
			Host_Error ("Mod_LoadMarksurfaces: funny lump size in %s", mod->name);

		count = l->filelen / sizeof (short);
		out = (int *)Mem_Alloc (count * sizeof (*out));

		mod->marksurfaces = out;
		mod->nummarksurfaces = count;

		// johnfitz -- warn mappers about exceeding old limits
		if (count > 32767)
			Con_DWarning ("%i marksurfaces exceeds standard limit of 32767.\n", count);
		// johnfitz

		for (i = 0; i < count; i++)
		{
			j = (unsigned short)ReadShortUnaligned (in + (i * sizeof (short))); // johnfitz -- explicit cast as unsigned short
			if (j >= mod->numsurfaces)
				Sys_Error ("Mod_LoadMarksurfaces: bad surface number");
			out[i] = j;
		}
	}
}

/*
=================
Mod_LoadSurfedges
=================
*/
void Mod_LoadSurfedges (qmodel_t *mod, byte *mod_base, lump_t *l)
{
	int	  i, count;
	byte *in;
	int	 *out;

	in = mod_base + l->fileofs;
	if (l->filelen % sizeof (int))
		Sys_Error ("MOD_LoadBmodel: funny lump size in %s", mod->name);
	count = l->filelen / sizeof (int);
	out = (int *)Mem_Alloc (count * sizeof (int));

	mod->surfedges = out;
	mod->numsurfedges = count;

	for (i = 0; i < count; i++)
	{
		out[i] = ReadLongUnaligned (in + (i * sizeof (int)));
	}
}

/*
=================
Mod_LoadPlanes
=================
*/
void Mod_LoadPlanes (qmodel_t *mod, byte *mod_base, lump_t *l)
{
	int		  i, j;
	mplane_t *out;
	byte	 *in;
	int		  count;
	int		  bits;

	in = mod_base + l->fileofs;
	if (l->filelen % sizeof (dplane_t))
		Sys_Error ("MOD_LoadBmodel: funny lump size in %s", mod->name);
	count = l->filelen / sizeof (dplane_t);
	out = (mplane_t *)Mem_Alloc (count * 2 * sizeof (*out));

	mod->planes = out;
	mod->numplanes = count;

	for (i = 0; i < count; i++, in += sizeof (dplane_t), out++)
	{
		bits = 0;
		for (j = 0; j < 3; j++)
		{
			out->normal[j] = ReadFloatUnaligned (in + offsetof (dplane_t, normal[j]));
			if (out->normal[j] < 0)
				bits |= 1 << j;
		}

		out->dist = ReadFloatUnaligned (in + offsetof (dplane_t, dist));
		out->type = ReadLongUnaligned (in + offsetof (dplane_t, type));
		out->signbits = bits;
	}
}

/*
=================
RadiusFromBounds
=================
*/
float RadiusFromBounds (vec3_t mins, vec3_t maxs)
{
	int	   i;
	vec3_t corner;

	for (i = 0; i < 3; i++)
	{
		corner[i] = fabs (mins[i]) > fabs (maxs[i]) ? fabs (mins[i]) : fabs (maxs[i]);
	}

	return VectorLength (corner);
}

/*
=================
Mod_LoadSubmodels
=================
*/
void Mod_LoadSubmodels (qmodel_t *mod, byte *mod_base, lump_t *l)
{
	byte	 *in;
	dmodel_t *out;
	int		  i, j, count;

	in = mod_base + l->fileofs;
	if (l->filelen % sizeof (dmodel_t))
		Sys_Error ("MOD_LoadBmodel: funny lump size in %s", mod->name);
	count = l->filelen / sizeof (dmodel_t);
	out = (dmodel_t *)Mem_Alloc (count * sizeof (*out));

	mod->submodels = out;
	mod->numsubmodels = count;

	for (i = 0; i < count; i++, in += sizeof (dmodel_t), out++)
	{
		for (j = 0; j < 3; j++)
		{ // spread the mins / maxs by a pixel
			out->mins[j] = ReadFloatUnaligned (in + offsetof (dmodel_t, mins[j])) - 1;
			out->maxs[j] = ReadFloatUnaligned (in + offsetof (dmodel_t, maxs[j])) + 1;
			out->origin[j] = ReadFloatUnaligned (in + offsetof (dmodel_t, origin[j]));
		}
		for (j = 0; j < MAX_MAP_HULLS; j++)
		{
			out->headnode[j] = ReadLongUnaligned (in + offsetof (dmodel_t, headnode[j]));
		}
		out->visleafs = ReadLongUnaligned (in + offsetof (dmodel_t, visleafs));
		out->firstface = ReadLongUnaligned (in + offsetof (dmodel_t, firstface));
		out->numfaces = ReadLongUnaligned (in + offsetof (dmodel_t, numfaces));
	}

	// johnfitz -- check world visleafs -- adapted from bjp
	out = mod->submodels;

	if (out->visleafs > 8192)
		Con_DWarning ("%i visleafs exceeds standard limit of 8192.\n", out->visleafs);
	// johnfitz
}

/* EXTERNAL VIS FILE SUPPORT:
 */
typedef struct vispatch_s
{
	char mapname[32];
	int	 filelen; // length of data after header (VIS+Leafs)
} vispatch_t;
#define VISPATCH_HEADER_LEN 36

FILE *Mod_FindVisibilityExternal (qmodel_t *mod, const char *loadname)
{
	vispatch_t	 header;
	char		 visfilename[MAX_QPATH];
	const char	*shortname;
	unsigned int path_id;
	FILE		*f;
	long		 pos;
	size_t		 r;

	q_snprintf (visfilename, sizeof (visfilename), "maps/%s.vis", loadname);
	if (COM_FOpenFile (visfilename, &f, &path_id) < 0)
	{
		Con_DPrintf ("%s not found, trying ", visfilename);
		q_snprintf (visfilename, sizeof (visfilename), "%s.vis", COM_SkipPath (com_gamedir));
		Con_DPrintf ("%s\n", visfilename);
		if (COM_FOpenFile (visfilename, &f, &path_id) < 0)
		{
			Con_DPrintf ("external vis not found\n");
			return NULL;
		}
	}
	if (path_id < mod->path_id)
	{
		fclose (f);
		Con_DPrintf ("ignored %s from a gamedir with lower priority\n", visfilename);
		return NULL;
	}

	Con_DPrintf ("Found external VIS %s\n", visfilename);

	shortname = COM_SkipPath (mod->name);
	pos = 0;
	while ((r = fread (&header, 1, VISPATCH_HEADER_LEN, f)) == VISPATCH_HEADER_LEN)
	{
		header.filelen = LittleLong (header.filelen);
		if (header.filelen <= 0)
		{ /* bad entry -- don't trust the rest. */
			fclose (f);
			return NULL;
		}
		if (!q_strcasecmp (header.mapname, shortname))
			break;
		pos += header.filelen + VISPATCH_HEADER_LEN;
		Sys_fseek (f, pos, SEEK_SET);
	}
	if (r != VISPATCH_HEADER_LEN)
	{
		fclose (f);
		Con_DPrintf ("%s not found in %s\n", shortname, visfilename);
		return NULL;
	}

	return f;
}

byte *Mod_LoadVisibilityExternal (FILE *f)
{
	int	  filelen;
	byte *visdata;

	filelen = 0;
	if (fread (&filelen, 1, 4, f) != 4)
		return NULL;
	filelen = LittleLong (filelen);
	if (filelen <= 0)
		return NULL;
	Con_DPrintf ("...%d bytes visibility data\n", filelen);
	visdata = (byte *)Mem_Alloc (filelen);
	if (fread (visdata, filelen, 1, f) != 1)
		return NULL;
	return visdata;
}

void Mod_LoadLeafsExternal (qmodel_t *mod, FILE *f)
{
	int	  filelen;
	void *in;

	filelen = 0;
	if (fread (&filelen, 1, 4, f) != 4)
		Sys_Error ("Invalid leaf");
	filelen = LittleLong (filelen);
	if (filelen <= 0)
		return;
	Con_DPrintf ("...%d bytes leaf data\n", filelen);
	in = Mem_Alloc (filelen);
	if (fread (in, filelen, 1, f) != 1)
		return;
	Mod_ProcessLeafs_S (mod, (byte *)in, filelen);
}

/*
================
Mod_CalcSpecialsAndTextures
================
*/
static int Mod_TextureIndexForSurface (qmodel_t *model, msurface_t *surf)
{
	texture_t *texture = surf->texinfo->texture;

	if (surf->texinfo->tex_idx >= 0 && surf->texinfo->tex_idx < model->numtextures && model->textures[surf->texinfo->tex_idx] == texture)
		return surf->texinfo->tex_idx;

	for (int i = 0; i < model->numtextures; i++)
	{
		if (model->textures[i] == texture)
			return i;
	}

	return -1;
}

static void Mod_CalcSpecialsAndTextures (qmodel_t *model)
{
	qboolean is_submodel = model->name[0] == '*';

	model->used_specials = 0;

	TEMP_ALLOC_ZEROED (byte, used_tex, model->numtextures);

	for (int i = 0; i < model->nummodelsurfaces; i++)
	{
		msurface_t *psurf = &model->surfaces[model->firstmodelsurface] + i;
		model->used_specials |= (SURF_DRAWSKY | SURF_DRAWTURB | SURF_DRAWWATER | SURF_DRAWLAVA | SURF_DRAWSLIME | SURF_DRAWTELE) & psurf->flags;

		if (is_submodel && psurf->texinfo->tex_idx >= 0)
		{
			if (psurf->texinfo->tex_idx < model->numtextures)
			{
				used_tex[psurf->texinfo->tex_idx] = true;
			}
			else
			{
				TEMP_FREE (used_tex);
				// Can we incounter invalid indices tex_idx >= model->numtextures
				Host_Error ("Mod_CalcSpecialsAndTextures: %s invalid tex_idx %i", model->name, (int)psurf->texinfo->tex_idx);
			}
		}
	}

	if (is_submodel)
	{
		int total = 0, placed = 0;
		for (int i = 0; i < model->numtextures; i++)
			if (used_tex[i])
				++total;

		texture_t **orig_textures = model->textures;
		model->textures = (texture_t **)Mem_AllocNonZero (total * sizeof (*model->textures));
		model->numtextures = total;

		for (int i = 0; placed < total; i++)
		{
			if (used_tex[i])
				model->textures[placed++] = orig_textures[i];
		}
	}

	memset (used_tex, 0, temp_alloc_used_tex_size);
	TEMP_ALLOC_ZEROED (int, tex_counts, TEXTYPE_COUNT);
	TEMP_ALLOC_ZEROED (int, tex_offsets, TEXTYPE_COUNT);

	for (int i = 0; i < model->nummodelsurfaces; i++)
	{
		msurface_t *psurf = &model->surfaces[model->firstmodelsurface] + i;
		const int	tex_index = Mod_TextureIndexForSurface (model, psurf);
		if (tex_index >= 0)
			used_tex[tex_index] = true;
	}

	for (int i = 0; i < model->numtextures; i++)
	{
		texture_t *texture = model->textures[i];
		if (texture && used_tex[i])
			++tex_counts[texture->type];
	}

	int total = 0;
	for (int i = 0; i < TEXTYPE_COUNT; i++)
	{
		model->texofs[i] = tex_offsets[i] = total;
		total += tex_counts[i];
	}
	model->texofs[TEXTYPE_COUNT] = total;

	model->usedtextures = total ? (int *)Mem_Alloc (total * sizeof (*model->usedtextures)) : NULL;
	for (int i = 0; i < model->numtextures; i++)
	{
		texture_t *texture = model->textures[i];
		if (texture && used_tex[i])
			model->usedtextures[tex_offsets[texture->type]++] = i;
	}

	TEMP_FREE (tex_offsets);
	TEMP_FREE (tex_counts);
	TEMP_FREE (used_tex);
}

/*
=================
Mod_SetupSubmodels
set up the submodels (FIXME: this is confusing)
=================
*/
void Mod_SetupSubmodels (qmodel_t *mod)
{
	texture_t **const orig_textures = mod->textures;
	int const		  orig_numtextures = mod->numtextures;

	int		  i, j;
	float	  radius;
	dmodel_t *bm;

	// johnfitz -- okay, so that i stop getting confused every time i look at this loop, here's how it works:
	// we're looping through the submodels starting at 0.  Submodel 0 is the main model, so we don't have to
	// worry about clobbering data the first time through, since it's the same data.  At the end of the loop,
	// we create a new copy of the data to use the next time through.
	for (i = 0; i < mod->numsubmodels; i++)
	{
		bm = &mod->submodels[i];

		mod->hulls[0].firstclipnode = bm->headnode[0];
		for (j = 1; j < MAX_MAP_HULLS; j++)
		{
			mod->hulls[j].firstclipnode = bm->headnode[j];
			mod->hulls[j].lastclipnode = mod->numclipnodes - 1;
		}

		mod->firstmodelsurface = bm->firstface;
		mod->nummodelsurfaces = bm->numfaces;

		VectorCopy (bm->maxs, mod->maxs);
		VectorCopy (bm->mins, mod->mins);

		// johnfitz -- calculate rotate bounds and yaw bounds
		radius = RadiusFromBounds (mod->mins, mod->maxs);
		mod->rmaxs[0] = mod->rmaxs[1] = mod->rmaxs[2] = mod->ymaxs[0] = mod->ymaxs[1] = mod->ymaxs[2] = radius;
		mod->rmins[0] = mod->rmins[1] = mod->rmins[2] = mod->ymins[0] = mod->ymins[1] = mod->ymins[2] = -radius;
		// johnfitz

		// johnfitz -- correct physics cullboxes so that outlying clip brushes on doors and stuff are handled right
		if (i > 0 || strcmp (mod->name, sv.modelname) != 0) // skip submodel 0 of sv.worldmodel, which is the actual world
		{
			// start with the hull0 bounds
			VectorCopy (mod->maxs, mod->clipmaxs);
			VectorCopy (mod->mins, mod->clipmins);

			// process hull1 (we don't need to process hull2 becuase there's
			// no such thing as a brush that appears in hull2 but not hull1)
			// Mod_BoundsFromClipNode (mod, 1, mod->hulls[1].firstclipnode); // (disabled for now becuase it fucks up on rotating models)
		}
		// johnfitz

		mod->numleafs = bm->visleafs;

		mod->textures = orig_textures;
		mod->numtextures = orig_numtextures;
		Mod_CalcSpecialsAndTextures (mod);

		if (i < mod->numsubmodels - 1)
		{ // duplicate the basic information
			char name[12];

			q_snprintf (name, sizeof (name), "*%i", i + 1);
			qmodel_t *submodel = Mod_FindName (name);
			*submodel = *mod;
			strcpy (submodel->name, name);
#ifdef PSET_SCRIPT
			// Need to NULL this otherwise we double delete in PScript_ClearSurfaceParticles
			submodel->skytrimem = NULL;
#endif
			mod = submodel;
		}
	}
}
#endif // !USE_RUST_FORMATS

/*
==============================================================================

ALIAS MODELS

==============================================================================
*/

stvert_t stverts[MAXALIASVERTS];

mtriangle_t *triangles = NULL;
// grow counter for the shared `triangles` allocation; stays C-owned under
// USE_RUST_FORMATS so the Rust loader reallocs against the same count
size_t		 triangles_size = 0;

// a pose is a single set of vertexes.  a frame may be
// an animating sequence of poses
trivertx_t *poseverts[MAXALIASFRAMES];
#ifndef USE_RUST_FORMATS
static int posenum;
#endif // !USE_RUST_FORMATS

#ifndef USE_RUST_FORMATS
/*
=================
Mod_LoadAliasFrame
=================
*/
void *Mod_LoadAliasFrame (void *pin, aliashdr_t *pheader, const int index)
{
	maliasframedesc_t *frame = &pheader->frames[index];
	trivertx_t		  *pinframe;
	int				   i;
	daliasframe_t	  *pdaliasframe;

	if (posenum >= MAXALIASFRAMES)
		Sys_Error ("posenum >= MAXALIASFRAMES");

	pdaliasframe = (daliasframe_t *)pin;

	q_strlcpy (frame->name, pdaliasframe->name, sizeof (frame->name));
	frame->firstpose = posenum;
	frame->numposes = 1;

	for (i = 0; i < 3; i++)
	{
		// these are byte values, so we don't have to worry about
		// endianness
		frame->bboxmin.v[i] = pdaliasframe->bboxmin.v[i];
		frame->bboxmax.v[i] = pdaliasframe->bboxmax.v[i];
	}

	pinframe = (trivertx_t *)(pdaliasframe + 1);

	poseverts[posenum] = pinframe;
	posenum++;

	pinframe += pheader->numverts;

	return (void *)pinframe;
}

/*
=================
Mod_LoadAliasGroup
=================
*/
void *Mod_LoadAliasGroup (void *pin, aliashdr_t *pheader, const int index)
{
	assert (pheader->poseverttype == PV_QUAKE1);

	maliasframedesc_t *frame = &pheader->frames[index];
	daliasgroup_t	  *pingroup;
	int				   i, numframes;
	daliasinterval_t  *pin_intervals;
	void			  *ptemp;

	pingroup = (daliasgroup_t *)pin;

	numframes = LittleLong (pingroup->numframes);

	frame->firstpose = posenum;
	frame->numposes = numframes;

	for (i = 0; i < 3; i++)
	{
		// these are byte values, so we don't have to worry about endianness
		frame->bboxmin.v[i] = pingroup->bboxmin.v[i];
		frame->bboxmax.v[i] = pingroup->bboxmax.v[i];
	}

	pin_intervals = (daliasinterval_t *)(pingroup + 1);

	frame->interval = LittleFloat (pin_intervals->interval);

	pin_intervals += numframes;

	ptemp = (void *)pin_intervals;

	for (i = 0; i < numframes; i++)
	{
		if (posenum >= MAXALIASFRAMES)
			Sys_Error ("posenum >= MAXALIASFRAMES");

		poseverts[posenum] = (trivertx_t *)((daliasframe_t *)ptemp + 1);
		posenum++;

		ptemp = (trivertx_t *)((daliasframe_t *)ptemp + 1) + pheader->numverts;
	}

	return ptemp;
}

/*
=================
Mod_CalcAliasBounds -- johnfitz -- calculate bounds of alias model for nonrotated, yawrotated, and fullrotated cases
=================
*/
void Mod_CalcAliasBounds (qmodel_t *mod, aliashdr_t *a, int numvertexes, byte *vertexes)
{
	float  dist, yawradius, radius;
	vec3_t v;

	// clear out all data
	for (int i = 0; i < 3; i++)
	{
		mod->mins[i] = mod->ymins[i] = mod->rmins[i] = FLT_MAX;
		mod->maxs[i] = mod->ymaxs[i] = mod->rmaxs[i] = -FLT_MAX;
		radius = yawradius = 0;
	}

	switch (a->poseverttype)
	{
	case PV_QUAKE1:
	{
		// process verts
		for (int i = 0; i < a->numposes; i++)
		{
			for (int j = 0; j < a->numverts; j++)
			{
				for (int k = 0; k < 3; k++)
					v[k] = poseverts[i][j].v[k] * a->scale[k] + a->scale_origin[k];

				for (int k = 0; k < 3; k++)
				{
					mod->mins[k] = q_min (mod->mins[k], v[k]);
					mod->maxs[k] = q_max (mod->maxs[k], v[k]);
				}
				dist = v[0] * v[0] + v[1] * v[1];
				if (yawradius < dist)
					yawradius = dist;
				dist += v[2] * v[2];
				if (radius < dist)
					radius = dist;
			}
		}
	}
	break;
	case PV_MD5:
	{
		// process verts : (vertexes;numvertexes) is all the vertices from all the poses/frames
		// for all the surfaces of a model.
		md5vert_t *pv = (md5vert_t *)vertexes;
		for (int j = 0; j < numvertexes; j++)
		{
			for (int k = 0; k < 3; k++)
				v[k] = pv[j].xyz[k];

			for (int k = 0; k < 3; k++)
			{
				mod->mins[k] = q_min (mod->mins[k], v[k]);
				mod->maxs[k] = q_max (mod->maxs[k], v[k]);
			}
			dist = v[0] * v[0] + v[1] * v[1];
			if (yawradius < dist)
				yawradius = dist;
			dist += v[2] * v[2];
			if (radius < dist)
				radius = dist;
		}
	}
	break;
	case PV_MD5_8:
	{
		// process verts : (vertexes;numvertexes) is all the vertices from all the poses/frames
		// for all the surfaces of a model.
		md5vert8_t *pv = (md5vert8_t *)vertexes;
		for (int j = 0; j < numvertexes; j++)
		{
			for (int k = 0; k < 3; k++)
				v[k] = pv[j].xyz[k];

			for (int k = 0; k < 3; k++)
			{
				mod->mins[k] = q_min (mod->mins[k], v[k]);
				mod->maxs[k] = q_max (mod->maxs[k], v[k]);
			}
			dist = v[0] * v[0] + v[1] * v[1];
			if (yawradius < dist)
				yawradius = dist;
			dist += v[2] * v[2];
			if (radius < dist)
				radius = dist;
		}
	}
	break;
	case PV_QUAKE3:
	{
		// process verts : (vertexes;numvertexes) is all the vertices from all the poses/frames
		// for all the surfaces of a model.
		md3XyzNormal_t *pv = (md3XyzNormal_t *)vertexes;
		for (int j = 0; j < numvertexes; j++)
		{
			for (int k = 0; k < 3; k++)
				v[k] = pv[j].xyz[k] * MD3_XYZ_SCALE;

			for (int k = 0; k < 3; k++)
			{
				mod->mins[k] = q_min (mod->mins[k], v[k]);
				mod->maxs[k] = q_max (mod->maxs[k], v[k]);
			}
			dist = v[0] * v[0] + v[1] * v[1];
			if (yawradius < dist)
				yawradius = dist;
			dist += v[2] * v[2];
			if (radius < dist)
				radius = dist;
		}
	}
	break;
	default:
		assert (false);
	}

	// rbounds will be used when entity has nonzero pitch or roll
	radius = sqrtf (radius);
	mod->rmins[0] = mod->rmins[1] = mod->rmins[2] = -radius;
	mod->rmaxs[0] = mod->rmaxs[1] = mod->rmaxs[2] = radius;

	// ybounds will be used when entity has nonzero yaw
	yawradius = sqrtf (yawradius);
	mod->ymins[0] = mod->ymins[1] = -yawradius;
	mod->ymaxs[0] = mod->ymaxs[1] = yawradius;
	mod->ymins[2] = mod->mins[2];
	mod->ymaxs[2] = mod->maxs[2];
}
#endif // !USE_RUST_FORMATS

static qboolean nameInList (const char *list, const char *name)
{
	const char *s;
	char		tmp[MAX_QPATH];
	int			i;

	s = list;

	while (*s)
	{
		// make a copy until the next comma or end of string
		i = 0;
		while (*s && *s != ',')
		{
			if (i < MAX_QPATH - 1)
				tmp[i++] = *s;
			s++;
		}
		tmp[i] = '\0';
		// compare it to the model name
		if (!strcmp (name, tmp))
		{
			return true;
		}
		// search forwards to the next comma or end of string
		while (*s && *s == ',')
			s++;
	}
	return false;
}

/*
=================
Mod_SetExtraFlags -- johnfitz -- set up extra flags that aren't in the mdl
=================
*/
void Mod_SetExtraFlags (qmodel_t *mod)
{
	extern cvar_t r_nolerp_list;

	if (!mod)
		return;

	mod->flags &= (0xFF | MF_HOLEY); // only preserve first byte, plus MF_HOLEY

	if (mod->type == mod_alias)
	{
		// nolerp flag
		if (nameInList (r_nolerp_list.string, mod->name))
			mod->flags |= MOD_NOLERP;

		// fullbright hack (TODO: make this a cvar list)
		if (!strcmp (mod->name, "progs/flame2.mdl") || !strcmp (mod->name, "progs/flame.mdl") || !strcmp (mod->name, "progs/boss.mdl"))
			mod->flags |= MOD_FBRIGHTHACK;
	}

#ifdef PSET_SCRIPT
	PScript_UpdateModelEffects (mod);
#endif
}

#ifndef USE_RUST_FORMATS
static void check_tris_size (size_t numtris)
{
	// 1. assure that numtris < trinagles_size, else realloc
	if (numtris > triangles_size)
	{
		size_t new_trinagles_size = q_max (triangles_size * 2, numtris);

		triangles = Mem_Realloc (triangles, new_trinagles_size * sizeof (mtriangle_t));

		triangles_size = new_trinagles_size;
	}
}

/*
=================
Mod_ParseAliasModel
=================
*/
aliashdr_t *Mod_ParseAliasModel (qmodel_t *mod, void *buffer)
{
	int	  i, j;
	byte *pinstverts;
	byte *pintriangles;
	int	  version, numframes;
	int	  size;
	byte *pframetype;
	byte *pskintype;
	byte *mod_base = (byte *)buffer; // johnfitz

	version = ReadLongUnaligned (mod_base + offsetof (mdl_t, version));
	if (version != ALIAS_VERSION)
		Sys_Error ("%s has wrong version number (%i should be %i)", mod->name, version, ALIAS_VERSION);

	//
	// allocate space for a working header, plus all the data except the frames,
	// skin and group info
	//
	size = sizeof (aliashdr_t) + (ReadLongUnaligned (mod_base + offsetof (mdl_t, numframes)) - 1) * sizeof (maliasframedesc_t);
	aliashdr_t *pheader = (aliashdr_t *)Mem_Alloc (size);
	pheader->poseverttype = PV_QUAKE1;

	mod->flags = ReadLongUnaligned (mod_base + offsetof (mdl_t, flags));

	//
	// endian-adjust and copy the data, starting with the alias model header
	//
	pheader->boundingradius = ReadLongUnaligned (mod_base + offsetof (mdl_t, boundingradius));
	pheader->numskins = ReadLongUnaligned (mod_base + offsetof (mdl_t, numskins));
	pheader->skinwidth = ReadLongUnaligned (mod_base + offsetof (mdl_t, skinwidth));
	pheader->skinheight = ReadLongUnaligned (mod_base + offsetof (mdl_t, skinheight));

	if (pheader->skinheight > MAX_LBM_HEIGHT)
		Con_DWarning ("model %s has a skin taller than %d", mod->name, MAX_LBM_HEIGHT);

	pheader->numverts = ReadLongUnaligned (mod_base + offsetof (mdl_t, numverts));

	if (pheader->numverts <= 0)
		Sys_Error ("model %s has no vertices", mod->name);

	if (pheader->numverts > MAXALIASVERTS)
		Sys_Error ("model %s has too many vertices (%d; max = %d)", mod->name, pheader->numverts, MAXALIASVERTS);

	if (pheader->numverts > MAXALIASVERTS_QS)
		Con_DWarning ("model %s vertex count of %d exceeds QS limit of %d\n", mod->name, pheader->numverts, MAXALIASVERTS_QS);

	pheader->numtris = ReadLongUnaligned (mod_base + offsetof (mdl_t, numtris));

	if (pheader->numtris <= 0)
		Sys_Error ("model %s has no triangles", mod->name);

	if (pheader->numtris > MAXALIASTRIS_QS)
		Con_DWarning ("model %s triangle count of %d exceeds QS limit of %d\n", mod->name, pheader->numtris, MAXALIASTRIS_QS);

	check_tris_size (pheader->numtris);

	pheader->numframes = ReadLongUnaligned (mod_base + offsetof (mdl_t, numframes));
	numframes = pheader->numframes;
	if (numframes < 1)
		Sys_Error ("Mod_LoadAliasModel: Invalid # of frames: %d", numframes);

	pheader->size = ReadFloatUnaligned (mod_base + offsetof (mdl_t, size)) * ALIAS_BASE_SIZE_RATIO;
	mod->synctype = (synctype_t)ReadLongUnaligned (mod_base + offsetof (mdl_t, synctype));
	mod->numframes = pheader->numframes;

	for (i = 0; i < 3; i++)
	{
		pheader->scale[i] = ReadFloatUnaligned (mod_base + offsetof (mdl_t, scale[i]));
		pheader->scale_origin[i] = ReadFloatUnaligned (mod_base + offsetof (mdl_t, scale_origin[i]));
		pheader->eyeposition[i] = ReadFloatUnaligned (mod_base + offsetof (mdl_t, eyeposition[i]));
	}

	//
	// load the skins
	//
	pskintype = mod_base + sizeof (mdl_t);
	pskintype = Mod_LoadAllSkins (pheader, mod, mod_base, pheader->numskins, pskintype);

	//
	// load base s and t vertices
	//
	pinstverts = pskintype;

	for (i = 0; i < pheader->numverts; i++)
	{
		stverts[i].onseam = ReadLongUnaligned (pinstverts + offsetof (stvert_t, onseam));
		stverts[i].s = ReadLongUnaligned (pinstverts + offsetof (stvert_t, s));
		stverts[i].t = ReadLongUnaligned (pinstverts + offsetof (stvert_t, t));
		pinstverts += sizeof (stvert_t);
	}

	//
	// load triangle lists
	//
	pintriangles = pinstverts;

	for (i = 0; i < pheader->numtris; i++)
	{
		triangles[i].facesfront = ReadLongUnaligned (pintriangles + offsetof (dtriangle_t, facesfront));

		for (j = 0; j < 3; j++)
		{
			triangles[i].vertindex[j] = ReadLongUnaligned (pintriangles + offsetof (dtriangle_t, vertindex[j]));
		}
		pintriangles += sizeof (dtriangle_t);
	}

	//
	// load the frames
	//
	posenum = 0;
	pframetype = pintriangles;

	for (i = 0; i < numframes; i++)
	{
		aliasframetype_t frametype;
		frametype = (aliasframetype_t)ReadLongUnaligned (pframetype + offsetof (daliasframetype_t, type));
		if (frametype == ALIAS_SINGLE)
			pframetype = Mod_LoadAliasFrame (pframetype + sizeof (daliasframetype_t), pheader, i);
		else
			pframetype = Mod_LoadAliasGroup (pframetype + sizeof (daliasframetype_t), pheader, i);
	}

	pheader->numposes = posenum;

	mod->type = mod_alias;

	Mod_SetExtraFlags (mod); // johnfitz

	Mod_CalcAliasBounds (mod, pheader, 0, NULL); // johnfitz

	return pheader;
}

/*
=================
Mod_LoadSpriteFrame
=================
*/
static void *Mod_LoadSpriteFrame (qmodel_t *mod, byte *mod_base, void *pin, mspriteframe_t **ppframe, int framenum)
{
	dspriteframe_t *pinframe;
	mspriteframe_t *pspriteframe;
	int				width, height, size, origin[2];
	char			name[64];
	src_offset_t	offset; // johnfitz

	pinframe = (dspriteframe_t *)pin;

	width = LittleLong (pinframe->width);
	height = LittleLong (pinframe->height);
	size = width * height;

	pspriteframe = (mspriteframe_t *)Mem_Alloc (sizeof (mspriteframe_t));
	*ppframe = pspriteframe;

	pspriteframe->width = width;
	pspriteframe->height = height;
	origin[0] = LittleLong (pinframe->origin[0]);
	origin[1] = LittleLong (pinframe->origin[1]);

	pspriteframe->up = origin[1];
	pspriteframe->down = origin[1] - height;
	pspriteframe->left = origin[0];
	pspriteframe->right = width + origin[0];

	pspriteframe->smax = 1;
	pspriteframe->tmax = 1;

	q_snprintf (name, sizeof (name), "%s:frame%i", mod->name, framenum);
	offset = (src_offset_t)(pinframe + 1) - (src_offset_t)mod_base; // johnfitz
	pspriteframe->gltexture = TexMgr_LoadImage (
		mod, name, width, height, SRC_INDEXED, (byte *)(pinframe + 1), mod->name, offset,
		TEXPREF_PAD | TEXPREF_ALPHA | TEXPREF_NOPICMIP); // johnfitz -- TexMgr

	return (void *)((byte *)pinframe + sizeof (dspriteframe_t) + size);
}

/*
=================
Mod_LoadSpriteGroup
=================
*/
static void *Mod_LoadSpriteGroup (qmodel_t *mod, byte *mod_base, void *pin, mspriteframe_t **ppframe, int framenum, spriteframetype_t type)
{
	dspritegroup_t	  *pingroup;
	mspritegroup_t	  *pspritegroup;
	int				   i, numframes;
	dspriteinterval_t *pin_intervals;
	float			  *poutintervals;
	void			  *ptemp;

	pingroup = (dspritegroup_t *)pin;

	numframes = LittleLong (pingroup->numframes);
	if (type == SPR_ANGLED && numframes != 8)
		Sys_Error ("Mod_LoadSpriteGroup: Bad # of frames: %d", numframes);

	pspritegroup = (mspritegroup_t *)Mem_Alloc (sizeof (mspritegroup_t) + (numframes - 1) * sizeof (pspritegroup->frames[0]));

	pspritegroup->numframes = numframes;

	*ppframe = (mspriteframe_t *)pspritegroup;

	pin_intervals = (dspriteinterval_t *)(pingroup + 1);

	poutintervals = (float *)Mem_Alloc (numframes * sizeof (float));

	pspritegroup->intervals = poutintervals;

	for (i = 0; i < numframes; i++)
	{
		*poutintervals = LittleFloat (pin_intervals->interval);
		if (*poutintervals <= 0.0)
			Sys_Error ("Mod_LoadSpriteGroup: interval<=0");

		poutintervals++;
		pin_intervals++;
	}

	ptemp = (void *)pin_intervals;

	for (i = 0; i < numframes; i++)
	{
		ptemp = Mod_LoadSpriteFrame (mod, mod_base, ptemp, &pspritegroup->frames[i], framenum * 100 + i);
	}

	return ptemp;
}

/*
=================
Mod_LoadSpriteModel
=================
*/
void Mod_LoadSpriteModel (qmodel_t *mod, void *buffer)
{
	int					i;
	int					version;
	dsprite_t		   *pin;
	msprite_t		   *psprite;
	int					numframes;
	int					size;
	dspriteframetype_t *pframetype;

	pin = (dsprite_t *)buffer;
	byte *mod_base = (byte *)buffer; // johnfitz

	version = LittleLong (pin->version);
	if (version != SPRITE_VERSION)
		Sys_Error (
			"%s has wrong version number "
			"(%i should be %i)",
			mod->name, version, SPRITE_VERSION);

	numframes = LittleLong (pin->numframes);
	if (numframes < 1)
		Sys_Error ("Mod_LoadSpriteModel: Invalid # of frames: %d", numframes);

	size = sizeof (msprite_t) + (numframes - 1) * sizeof (psprite->frames);

	psprite = (msprite_t *)Mem_Alloc (size);

	mod->extradata[PV_QUAKE1] = (byte *)psprite;

	psprite->type = LittleLong (pin->type);
	psprite->maxwidth = LittleLong (pin->width);
	psprite->maxheight = LittleLong (pin->height);
	mod->synctype = (synctype_t)LittleLong (pin->synctype);
	psprite->numframes = numframes;

	mod->mins[0] = mod->mins[1] = -psprite->maxwidth / 2;
	mod->maxs[0] = mod->maxs[1] = psprite->maxwidth / 2;
	mod->mins[2] = -psprite->maxheight / 2;
	mod->maxs[2] = psprite->maxheight / 2;

	//
	// load the frames
	//
	mod->numframes = numframes;

	pframetype = (dspriteframetype_t *)(pin + 1);

	for (i = 0; i < numframes; i++)
	{
		spriteframetype_t frametype;

		frametype = (spriteframetype_t)LittleLong (pframetype->type);
		psprite->frames[i].type = frametype;

		if (frametype == SPR_SINGLE)
		{
			pframetype = (dspriteframetype_t *)Mod_LoadSpriteFrame (mod, mod_base, pframetype + 1, &psprite->frames[i].frameptr, i);
		}
		else
		{
			pframetype = (dspriteframetype_t *)Mod_LoadSpriteGroup (mod, mod_base, pframetype + 1, &psprite->frames[i].frameptr, i, frametype);
		}
	}

	mod->type = mod_sprite;
}
#endif // !USE_RUST_FORMATS

/*
==============================================================================

MD5 / MD3 MODELS

==============================================================================
*/

#ifndef USE_RUST_FORMATS
/*
=================================================================
MD5 Models, for compat with the rerelease and NOT doom3.
=================================================================
md5mesh:
MD5Version 10
commandline ""
numJoints N
numMeshes N
joints {
	"name" ParentIdx ( Pos_X Y Z ) ( Quat_X Y Z )
}
mesh {
	shader "name"	//file-relative path, with _%02d_%02d postfixed for skin/framegroup support. unlike doom3.
	numverts N
	vert # ( S T ) FirstWeight count
	numtris N
	tri # A B C
	numweights N
	weight # JointIdx Scale ( X Y Z )
}

md5anim:
MD5Version 10
commandline ""
numFrames N
numJoints N
frameRate FPS
numAnimatedComponents N	//joints*6ish
hierachy {
	"name" ParentIdx Flags DataStart
}
bounds {
	( X Y Z ) ( X Y Z )
}
baseframe {
	( pos_X Y Z ) ( quad_X Y Z )
}
frame # {
	RAW ...
}

We'll unpack the animation to separate framegroups (one-pose per, for consistency with most q1 models).
*/

/*
================
MD5_ParseCheck
================
*/
static qboolean MD5_ParseCheck (const char *s, const void **buffer)
{
	if (strcmp (com_token, s))
		return false;
	*buffer = COM_Parse (*buffer);
	return true;
}

/*
================
MD5_ParseUInt
================
*/
static size_t MD5_ParseUInt (const void **buffer)
{
	size_t i = strtoull (com_token, NULL, 0);
	*buffer = COM_Parse (*buffer);
	return i;
}

/*
================
MD5_ParseSInt
================
*/
static long MD5_ParseSInt (const void **buffer)
{
	long i = strtol (com_token, NULL, 0);
	*buffer = COM_Parse (*buffer);
	return i;
}

/*
================
MD5_ParseFloat
================
*/
static double MD5_ParseFloat (const void **buffer)
{
	double i = strtod (com_token, NULL);
	*buffer = COM_Parse (*buffer);
	return i;
}

static size_t MD5_CountAnimatedComponents (unsigned int flags)
{
	size_t count = 0;
	for (unsigned int bit = 1; bit <= 32; bit <<= 1)
		if (flags & bit)
			count++;
	return count;
}

#define MD5ERROR(...)              \
	do                             \
	{                              \
		Con_Warning (__VA_ARGS__); \
		goto error;                \
	} while (0)

#define MD5EXPECT(s)                                                                                     \
	do                                                                                                   \
	{                                                                                                    \
		if (strcmp (com_token, s))                                                                       \
			MD5ERROR ("Mod_LoadMD5MeshModel(%s): expected \"%s\", found \"%s\"\n", fname, s, com_token); \
		buffer = COM_Parse (buffer);                                                                     \
	} while (0)
#define MD5UINT()	MD5_ParseUInt (&buffer)
#define MD5SINT()	MD5_ParseSInt (&buffer)
#define MD5FLOAT()	MD5_ParseFloat (&buffer)
#define MD5CHECK(s) MD5_ParseCheck (s, &buffer)
#define MD5IGNORE() buffer = COM_Parse (buffer)

/*
================
md5vertinfo_s
================
*/
typedef struct md5vertinfo_s
{
	size_t		 firstweight;
	unsigned int count;
	float		 st[2];
} md5vertinfo_t;

/*
================
md5weightinfo_s
================
*/
typedef struct md5weightinfo_s
{
	size_t joint_index;
	vec4_t pos;
} md5weightinfo_t;

/*
================
jointinfo_s
================
*/
typedef struct jointinfo_s
{
	ssize_t		parent; //-1 for a root joint
	ssize_t		poseparent;
	char		name[32];
	jointpose_t inverse;
} jointinfo_t;

typedef struct md5animjoint_s
{
	unsigned int flags, offset;
	ssize_t		 parent;
	ssize_t		 mesh_index;
	vec3_t		 basepos;
	vec4_t		 basequat;
} md5animjoint_t;

static qboolean MD5_ResolveMappedMeshParent (
	jointinfo_t *joints, size_t numjoints, const ssize_t *mesh_to_anim, ssize_t *mapped_mesh_parent, byte *mapped_mesh_parent_state, size_t mesh_index)
{
	ssize_t parent;

	if (mapped_mesh_parent_state[mesh_index] == 2)
		return true;
	if (mapped_mesh_parent_state[mesh_index] == 1)
		return false;

	mapped_mesh_parent_state[mesh_index] = 1;
	parent = joints[mesh_index].parent;
	if (parent < 0)
		mapped_mesh_parent[mesh_index] = -1;
	else if ((size_t)parent >= numjoints)
		return false;
	else if (mesh_to_anim[parent] >= 0)
		mapped_mesh_parent[mesh_index] = parent;
	else
	{
		if (!MD5_ResolveMappedMeshParent (joints, numjoints, mesh_to_anim, mapped_mesh_parent, mapped_mesh_parent_state, (size_t)parent))
			return false;
		mapped_mesh_parent[mesh_index] = mapped_mesh_parent[parent];
	}

	mapped_mesh_parent_state[mesh_index] = 2;
	return true;
}

/*
================
Matrix3x4_RM_Transform4
================
*/
static void Matrix3x4_RM_Transform4 (const float *matrix, const float *vector, float *product)
{
	product[0] = matrix[0] * vector[0] + matrix[1] * vector[1] + matrix[2] * vector[2] + matrix[3] * vector[3];
	product[1] = matrix[4] * vector[0] + matrix[5] * vector[1] + matrix[6] * vector[2] + matrix[7] * vector[3];
	product[2] = matrix[8] * vector[0] + matrix[9] * vector[1] + matrix[10] * vector[2] + matrix[11] * vector[3];
}

/*
================
GenMatrixPosQuat4Scale
================
*/
static void GenMatrixPosQuat4Scale (const vec3_t pos, const vec4_t quat, const vec3_t scale, float result[12])
{
	const float x2 = quat[0] + quat[0];
	const float y2 = quat[1] + quat[1];
	const float z2 = quat[2] + quat[2];

	const float xx = quat[0] * x2;
	const float xy = quat[0] * y2;
	const float xz = quat[0] * z2;
	const float yy = quat[1] * y2;
	const float yz = quat[1] * z2;
	const float zz = quat[2] * z2;
	const float xw = quat[3] * x2;
	const float yw = quat[3] * y2;
	const float zw = quat[3] * z2;

	result[0 * 4 + 0] = scale[0] * (1.0f - (yy + zz));
	result[1 * 4 + 0] = scale[0] * (xy + zw);
	result[2 * 4 + 0] = scale[0] * (xz - yw);

	result[0 * 4 + 1] = scale[1] * (xy - zw);
	result[1 * 4 + 1] = scale[1] * (1.0f - (xx + zz));
	result[2 * 4 + 1] = scale[1] * (yz + xw);

	result[0 * 4 + 2] = scale[2] * (xz + yw);
	result[1 * 4 + 2] = scale[2] * (yz - xw);
	result[2 * 4 + 2] = scale[2] * (1.0f - (xx + yy));

	result[0 * 4 + 3] = pos[0];
	result[1 * 4 + 3] = pos[1];
	result[2 * 4 + 3] = pos[2];
}

/*
================
Matrix3x4_Invert_Simple
================
*/
static void Matrix3x4_Invert_Simple (const float *in1, float *out)
{
	// we only support uniform scaling, so assume the first row is enough
	// (note the lack of sqrt here, because we're trying to undo the scaling,
	// this means multiplying by the inverse scale twice - squaring it, which
	// makes the sqrt a waste of time)
	const double scale = 1.0 / ((double)in1[0] * (double)in1[0] + (double)in1[1] * (double)in1[1] + (double)in1[2] * (double)in1[2]);

	// invert the rotation by transposing and multiplying by the squared
	// reciprocal of the input matrix scale as described above
	double temp[12];
	temp[0] = in1[0] * scale;
	temp[1] = in1[4] * scale;
	temp[2] = in1[8] * scale;
	temp[4] = in1[1] * scale;
	temp[5] = in1[5] * scale;
	temp[6] = in1[9] * scale;
	temp[8] = in1[2] * scale;
	temp[9] = in1[6] * scale;
	temp[10] = in1[10] * scale;

	// invert the translate
	temp[3] = -(in1[3] * temp[0] + in1[7] * temp[1] + in1[11] * temp[2]);
	temp[7] = -(in1[3] * temp[4] + in1[7] * temp[5] + in1[11] * temp[6]);
	temp[11] = -(in1[3] * temp[8] + in1[7] * temp[9] + in1[11] * temp[10]);

	for (int i = 0; i < 12; ++i)
		out[i] = (float)temp[i];
}

/*
================
MD5_BakeInfluences
================
*/
static qboolean MD5_BakeInfluences (
	const char *fname, jointpose_t *outposes, byte *vertexes, poseverttype_t poseverttype, struct md5vertinfo_s *vinfo, struct md5weightinfo_s *weight,
	size_t numverts, size_t numweights)
{
	struct md5weightinfo_s *w;
	vec3_t					pos;
	float					scale;
	unsigned int			maxinfluences = 0;
	float					scaleimprecision = 1;
	const int				stored_influences = (poseverttype == PV_MD5_8) ? NUM_JOINT_INFLUENCES_8_WEIGHT : NUM_JOINT_INFLUENCES_4_WEIGHT;
	const size_t			vertex_size = (poseverttype == PV_MD5_8) ? sizeof (md5vert8_t) : sizeof (md5vert_t);

	for (size_t v = 0; v < numverts; v++, vinfo++)
	{
		byte	   *vertex = vertexes + v * vertex_size;
		md5vert_t  *vert = (md5vert_t *)vertex;
		md5vert8_t *vert8 = (md5vert8_t *)vertex;
		float		weights[NUM_JOINT_INFLUENCES_8_WEIGHT] = {0.0f};
		size_t		joint_indices[NUM_JOINT_INFLUENCES_8_WEIGHT] = {0};
		vec4_t		joint_positions[NUM_JOINT_INFLUENCES_8_WEIGHT] = {{0.0f}};

		memset (vertex, 0, vertex_size);
		vert->st[0] = vinfo->st[0];
		vert->st[1] = vinfo->st[1];

		if (vinfo->firstweight + vinfo->count > numweights)
		{
			Con_Warning ("%s: weight index out of bounds\n", fname);
			return false;
		}
		if (maxinfluences < vinfo->count)
			maxinfluences = vinfo->count;
		w = weight + vinfo->firstweight;
		for (unsigned int i = 0; i < vinfo->count; i++, w++)
		{
			Matrix3x4_RM_Transform4 (outposes[w->joint_index].mat, w->pos, pos);
			VectorAdd (vert->xyz, pos, vert->xyz);

			if (i < NUM_JOINT_INFLUENCES_4_WEIGHT)
			{
				weights[i] = w->pos[3];
				joint_indices[i] = w->joint_index;
				Vector4Copy (w->pos, joint_positions[i]);
			}
			else if (i < NUM_JOINT_INFLUENCES_8_WEIGHT)
			{
				weights[i] = w->pos[3];
				joint_indices[i] = w->joint_index;
				Vector4Copy (w->pos, joint_positions[i]);
			}
			else
			{
				// obnoxious code to find the lowest of the current possible joint indexes.
				float  lowval = weights[0];
				size_t lowidx = 0;
				for (size_t k = 1; k < NUM_JOINT_INFLUENCES_8_WEIGHT; ++k)
				{
					if (weights[k] < lowval)
					{
						lowval = weights[k];
						lowidx = k;
					}
				}
				if (weights[lowidx] < w->pos[3])
				{ // found a lower/unset weight, replace it.
					weights[lowidx] = w->pos[3];
					joint_indices[lowidx] = w->joint_index;
					Vector4Copy (w->pos, joint_positions[lowidx]);
				}
			}
		}

		// normalize in case we dropped some weights.
		scale = 0;
		for (int k = 0; k < stored_influences; ++k)
			scale += weights[k];
		if (scale > 0)
		{
			if (scaleimprecision < scale)
				scaleimprecision = scale;
			scale = 1 / scale;
			for (int k = 0; k < stored_influences; ++k)
			{
				weights[k] *= scale;
				for (int c = 0; c < 4; ++c)
					joint_positions[k][c] *= scale;
			}
		}
		else // something bad...
		{
			weights[0] = 1;
			for (int k = 1; k < stored_influences; ++k)
				weights[k] = 0;
			joint_positions[0][3] = 1;
		}

		for (int j = 0; j < stored_influences; ++j)
		{
			if (poseverttype == PV_MD5_8)
			{
				vert8->joint_weights[j] = (byte)(CLAMP (0.0f, weights[j], 1.0f) * 255.0f);
				vert8->joint_indices[j] = joint_indices[j];
				vert8->joint_position_x[j] = joint_positions[j][0];
				vert8->joint_position_y[j] = joint_positions[j][1];
				vert8->joint_position_z[j] = joint_positions[j][2];
			}
			else
			{
				vert->joint_weights[j] = (byte)(CLAMP (0.0f, weights[j], 1.0f) * 255.0f);
				vert->joint_indices[j] = joint_indices[j];
				vert->joint_position_x[j] = joint_positions[j][0];
				vert->joint_position_y[j] = joint_positions[j][1];
				vert->joint_position_z[j] = joint_positions[j][2];
			}
		}
	}
	if (maxinfluences > NUM_JOINT_INFLUENCES_8_WEIGHT)
		Con_DWarning ("%s uses up to %u influences per vertex (weakest: %g)\n", fname, maxinfluences, scaleimprecision);
	return true;
}

/*
================
MD5_ComputeNormals
================
*/
static void MD5_ComputeNormals (byte *vertexes, size_t vertex_size, size_t numverts, unsigned short *indexes, size_t numindexes)
{
	hash_map_t *pos_to_normal_map = HashMap_Create (vec3_t, vec3_t, &HashVec3, NULL);
	HashMap_Reserve (pos_to_normal_map, numverts);

	for (size_t v = 0; v < numverts; v++)
	{
		md5vert_t *vert = (md5vert_t *)(vertexes + v * vertex_size);
		vert->norm[0] = vert->norm[1] = vert->norm[2] = 0;
	}
	for (size_t t = 0; t < numindexes; t += 3)
	{
		md5vert_t *verts[3] = {
			(md5vert_t *)(vertexes + indexes[t + 0] * vertex_size), (md5vert_t *)(vertexes + indexes[t + 1] * vertex_size),
			(md5vert_t *)(vertexes + indexes[t + 2] * vertex_size)};

		vec3_t d1, d2;
		VectorSubtract (verts[2]->xyz, verts[0]->xyz, d1);
		VectorSubtract (verts[1]->xyz, verts[0]->xyz, d2);
		VectorNormalize (d1);
		VectorNormalize (d2);

		vec3_t norm;
		CrossProduct (d1, d2, norm);
		VectorNormalize (norm);

		const float angle = acos (DotProduct (d1, d2));
		VectorScale (norm, angle, norm);

		vec3_t *found_normal;
		for (int i = 0; i < 3; ++i)
		{
			if ((found_normal = HashMap_Lookup (vec3_t, pos_to_normal_map, &verts[i]->xyz)))
				VectorAdd (norm, *found_normal, *found_normal);
			else
				HashMap_Insert (pos_to_normal_map, &verts[i]->xyz, &norm);
		}
	}

	const uint32_t map_size = HashMap_Size (pos_to_normal_map);
	for (uint32_t i = 0; i < map_size; ++i)
	{
		vec3_t *norm = HashMap_GetValue (vec3_t, pos_to_normal_map, i);
		VectorNormalize (*norm);
	}

	for (size_t v = 0; v < numverts; v++)
	{
		md5vert_t *vert = (md5vert_t *)(vertexes + v * vertex_size);
		vec3_t	  *norm = HashMap_Lookup (vec3_t, pos_to_normal_map, &vert->xyz);
		if (norm)
			VectorCopy (*norm, vert->norm);
	}

	HashMap_Destroy (pos_to_normal_map);
}

/*
================
MD5_HackyModelFlags
================
*/
static unsigned int MD5_HackyModelFlags (const char *name)
{
	unsigned int ret = 0;
	char		 oldmodel[MAX_QPATH];
	mdl_t		*f;
	COM_StripExtension (name, oldmodel, sizeof (oldmodel));
	COM_AddExtension (oldmodel, ".mdl", sizeof (oldmodel));

	f = (mdl_t *)COM_LoadFile (oldmodel, NULL);
	if (f)
	{
		if (com_filesize >= sizeof (*f) && LittleLong (f->ident) == IDPOLYHEADER && LittleLong (f->version) == ALIAS_VERSION)
			ret = f->flags;
		Mem_Free (f);
	}
	return ret;
}

/*
================
md5animctx_s
================
*/
typedef struct md5animctx_s
{
	void		*animfile;
	const void	*buffer;
	char		 fname[MAX_QPATH];
	size_t		 numposes;
	size_t		 numjoints;
	jointpose_t *posedata;
} md5animctx_t;

/*
================
MD5Anim_Begin
This is split into two because aliashdr_t has silly trailing framegroup info.
================
*/
static qboolean MD5Anim_Begin (md5animctx_t *ctx, const char *fname)
{
	// Load an md5anim into it, if we can.
	COM_StripExtension (fname, ctx->fname, sizeof (ctx->fname));
	COM_AddExtension (ctx->fname, ".md5anim", sizeof (ctx->fname));
	fname = ctx->fname;
	ctx->animfile = COM_LoadFile (fname, NULL);
	ctx->numposes = 0;

	if (ctx->animfile)
	{
		const void *buffer = COM_Parse (ctx->animfile);
		MD5EXPECT ("MD5Version");
		MD5EXPECT ("10");
		if (MD5CHECK ("commandline"))
			buffer = COM_Parse (buffer);
		MD5EXPECT ("numFrames");
		ctx->numposes = MD5UINT ();
		MD5EXPECT ("numJoints");
		ctx->numjoints = MD5UINT ();
		MD5EXPECT ("frameRate"); // irrelevant here

		if (ctx->numposes <= 0)
			MD5ERROR ("%s has no poses\n", fname);

		ctx->buffer = buffer;
	}
	return true;

error:
	SAFE_FREE (ctx->animfile);
	ctx->buffer = NULL;
	ctx->numposes = 0;
	ctx->numjoints = 0;
	return false;
}

/*
================
MD5Anim_Load
================
*/
static qboolean MD5Anim_Load (md5animctx_t *ctx, jointinfo_t *joints, jointpose_t *joint_poses, size_t numjoints)
{
	const char	*fname = ctx->fname;
	size_t		 rawcount;
	float		*r;
	jointpose_t *outposes = NULL;
	const void	*buffer = COM_Parse (ctx->buffer);
	size_t		 animjoints = ctx->numjoints;
	size_t		 j;
	TEMP_ALLOC_DECL (md5animjoint_t, ab);
	TEMP_ALLOC_DECL (ssize_t, mesh_to_anim);
	TEMP_ALLOC_DECL (ssize_t, mapped_mesh_parent);
	TEMP_ALLOC_DECL (byte, mapped_mesh_parent_state);
	TEMP_ALLOC_DECL (jointpose_t, bindposes);
	TEMP_ALLOC_DECL (float, raw);

	if (!buffer)
	{
		SAFE_FREE (ctx->animfile);
		return true;
	}

	MD5EXPECT ("numAnimatedComponents");
	rawcount = MD5UINT ();

	TEMP_ALLOC_ASSIGN_ZEROED (raw, rawcount + 6);
	TEMP_ALLOC_ASSIGN_ZEROED (ab, animjoints);
	TEMP_ALLOC_ASSIGN (mesh_to_anim, numjoints);
	TEMP_ALLOC_ASSIGN (mapped_mesh_parent, numjoints);
	TEMP_ALLOC_ASSIGN_ZEROED (mapped_mesh_parent_state, numjoints);
	TEMP_ALLOC_ASSIGN (bindposes, numjoints);

	for (j = 0; j < numjoints; j++)
	{
		mesh_to_anim[j] = -1;
		joints[j].poseparent = joints[j].parent;
	}

	for (j = 0; j < numjoints; j++)
	{
		if (joints[j].parent < 0)
			memcpy (bindposes[j].mat, joint_poses[j].mat, sizeof (bindposes[j].mat));
		else
			R_ConcatTransforms ((void *)joints[joints[j].parent].inverse.mat, (void *)joint_poses[j].mat, (void *)bindposes[j].mat);
	}

	ctx->posedata = outposes = Mem_Alloc (sizeof (*outposes) * numjoints * ctx->numposes);
	for (j = 0; j < ctx->numposes; j++)
		memcpy (outposes + j * numjoints, bindposes, sizeof (*bindposes) * numjoints);

	MD5EXPECT ("hierarchy");
	MD5EXPECT ("{");
	for (j = 0; j < animjoints; j++)
	{
		char		anim_name[sizeof (joints[0].name)];
		ssize_t		mesh_index = -1;
		const char *name = com_token;

		q_strlcpy (anim_name, name, sizeof (anim_name));
		buffer = COM_Parse (buffer);
		ab[j].parent = MD5SINT ();
		if (ab[j].parent < -1 || ab[j].parent >= (ssize_t)j)
			MD5ERROR ("%s: joint has bad parent order\n", fname);
		// new info
		ab[j].flags = MD5UINT ();
		if (ab[j].flags & ~63)
			MD5ERROR ("%s: joint has unsupported flags\n", fname);
		ab[j].offset = MD5UINT ();
		if (ab[j].offset + MD5_CountAnimatedComponents (ab[j].flags) > rawcount)
			MD5ERROR ("%s: joint has bad offset\n", fname);
		ab[j].mesh_index = -1;

		for (size_t k = 0; k < numjoints; k++)
		{
			if (!strcmp (joints[k].name, anim_name))
			{
				mesh_index = (ssize_t)k;
				break;
			}
		}

		if (mesh_index < 0)
			continue;
		if (mesh_to_anim[mesh_index] >= 0)
			MD5ERROR ("%s: duplicate joint \"%s\"\n", fname, anim_name);

		ab[j].mesh_index = mesh_index;
		mesh_to_anim[mesh_index] = (ssize_t)j;
	}
	MD5EXPECT ("}");

	for (j = 0; j < numjoints; j++)
	{
		if (!MD5_ResolveMappedMeshParent (joints, numjoints, mesh_to_anim, mapped_mesh_parent, mapped_mesh_parent_state, j))
			MD5ERROR ("%s: joint \"%s\" has bad parent chain\n", fname, joints[j].name);
	}

	for (j = 0; j < animjoints; j++)
	{
		ssize_t mesh_index = ab[j].mesh_index;
		ssize_t mesh_parent, anim_parent;

		if (mesh_index < 0)
			continue;

		mesh_parent = mapped_mesh_parent[mesh_index];
		if (ab[j].parent >= 0)
		{
			anim_parent = ab[ab[j].parent].mesh_index;
			if (anim_parent < 0)
				MD5ERROR ("%s: joint \"%s\" has unmapped parent\n", fname, joints[mesh_index].name);
		}
		else
			anim_parent = -1;
		if (mesh_parent != anim_parent)
			MD5ERROR ("%s: joint \"%s\" has wrong parent\n", fname, joints[mesh_index].name);
		joints[mesh_index].poseparent = anim_parent;
	}

	MD5EXPECT ("bounds");
	MD5EXPECT ("{");
	while (MD5CHECK ("("))
	{
		MD5IGNORE ();
		MD5IGNORE ();
		MD5IGNORE ();
		MD5EXPECT (")");

		MD5EXPECT ("(");
		MD5IGNORE ();
		MD5IGNORE ();
		MD5IGNORE ();
		MD5EXPECT (")");
	}
	MD5EXPECT ("}");

	MD5EXPECT ("baseframe");
	MD5EXPECT ("{");
	for (j = 0; j < animjoints; j++)
	{
		MD5EXPECT ("(");
		ab[j].basepos[0] = MD5FLOAT ();
		ab[j].basepos[1] = MD5FLOAT ();
		ab[j].basepos[2] = MD5FLOAT ();
		MD5EXPECT (")");

		MD5EXPECT ("(");
		ab[j].basequat[0] = MD5FLOAT ();
		ab[j].basequat[1] = MD5FLOAT ();
		ab[j].basequat[2] = MD5FLOAT ();
		ab[j].basequat[3] = 1 - DotProduct (ab[j].basequat, ab[j].basequat);
		if (ab[j].basequat[3] < 0)
			ab[j].basequat[3] = 0;
		ab[j].basequat[3] = -sqrtf (ab[j].basequat[3]);
		MD5EXPECT (")");
	}
	MD5EXPECT ("}");

	while (MD5CHECK ("frame"))
	{
		size_t idx = MD5UINT ();
		if (idx >= ctx->numposes)
			MD5ERROR ("%s: invalid pose index\n", fname);
		MD5EXPECT ("{");
		for (j = 0; j < rawcount; j++)
			raw[j] = MD5FLOAT ();
		MD5EXPECT ("}");

		// okay, we have our raw info, unpack the actual joint info.
		for (j = 0; j < animjoints; j++)
		{
			vec3_t		  pos;
			static vec3_t scale = {1, 1, 1};
			vec4_t		  quat;
			ssize_t		  mesh_index = ab[j].mesh_index;

			if (mesh_index < 0)
				continue;
			VectorCopy (ab[j].basepos, pos);
			Vector4Copy (ab[j].basequat, quat);
			r = raw + ab[j].offset;
			if (ab[j].flags & 1)
				pos[0] = *r++;
			if (ab[j].flags & 2)
				pos[1] = *r++;
			if (ab[j].flags & 4)
				pos[2] = *r++;

			if (ab[j].flags & 8)
				quat[0] = *r++;
			if (ab[j].flags & 16)
				quat[1] = *r++;
			if (ab[j].flags & 32)
				quat[2] = *r++;

			quat[3] = 1 - DotProduct (quat, quat);
			if (quat[3] < 0)
				quat[3] = 0; // we have no imagination.
			quat[3] = -sqrtf (quat[3]);

			GenMatrixPosQuat4Scale (pos, quat, scale, outposes[idx * numjoints + mesh_index].mat);
		}
	}
	TEMP_FREE (raw);
	TEMP_FREE (ab);
	TEMP_FREE (mesh_to_anim);
	TEMP_FREE (mapped_mesh_parent);
	TEMP_FREE (mapped_mesh_parent_state);
	TEMP_FREE (bindposes);
	SAFE_FREE (ctx->animfile);
	ctx->numjoints = numjoints;
	return true;

error:
	TEMP_FREE (raw);
	TEMP_FREE (ab);
	TEMP_FREE (mesh_to_anim);
	TEMP_FREE (mapped_mesh_parent);
	TEMP_FREE (mapped_mesh_parent_state);
	TEMP_FREE (bindposes);
	SAFE_FREE (ctx->posedata);
	SAFE_FREE (ctx->animfile);
	return false;
}

static qboolean Mod_LoadMD5MeshModelData (qmodel_t *mod, const void *buffer, size_t numjoints, size_t nummeshes)
{
	const char *fname = mod->name;

	aliashdr_t *outhdr = NULL, *surf;
	size_t		hdrsize = 0;

	md5animctx_t anim = {NULL};
	float		 dist, radius = 0, yawradius = 0;
	TEMP_ALLOC_DECL (md5vertinfo_t, vinfo);
	TEMP_ALLOC_DECL (byte, poutvertexes);
	TEMP_ALLOC_DECL (unsigned short, poutindexes);
	TEMP_ALLOC_DECL (md5weightinfo_t, weight);

	if (!MD5Anim_Begin (&anim, fname))
		return false;
	buffer = COM_Parse (buffer);

	hdrsize = sizeof (*outhdr) - sizeof (outhdr->frames);
	hdrsize += sizeof (outhdr->frames) * anim.numposes;

	// alloc all aliashdr_t and their chained nextsurface, a.k.a nummeshes, in one array
	// all aliashdr_t are zero-initialized by Mem_Alloc
	outhdr = (aliashdr_t *)Mem_Alloc (hdrsize * nummeshes);

	TEMP_ALLOC_ZEROED (jointinfo_t, joint_infos, numjoints);
	TEMP_ALLOC_ZEROED (jointpose_t, joint_poses, numjoints);
	TEMP_ALLOC_ZEROED (jointpose_t, skinning_joints, numjoints * anim.numposes);
	TEMP_ALLOC_ZEROED (jointpose_t, concat_joints, numjoints);

	for (size_t j = 0; j < 3; j++)
	{
		mod->mins[j] = mod->ymins[j] = mod->rmins[j] = FLT_MAX;
		mod->maxs[j] = mod->ymaxs[j] = mod->rmaxs[j] = -FLT_MAX;
	}

	// 1. Load joints
	MD5EXPECT ("{");
	for (size_t j = 0; j < numjoints; j++)
	{
		vec3_t		  pos;
		static vec3_t scale = {1, 1, 1};
		vec4_t		  quat;
		q_strlcpy (joint_infos[j].name, com_token, sizeof (joint_infos[j].name));
		buffer = COM_Parse (buffer);
		joint_infos[j].parent = MD5SINT ();
		if (joint_infos[j].parent < -1 || joint_infos[j].parent >= (ssize_t)numjoints)
			MD5ERROR ("%s: joint index out of bounds\n", fname);
		joint_infos[j].poseparent = joint_infos[j].parent;
		MD5EXPECT ("(");
		pos[0] = MD5FLOAT ();
		pos[1] = MD5FLOAT ();
		pos[2] = MD5FLOAT ();
		MD5EXPECT (")");
		MD5EXPECT ("(");
		quat[0] = MD5FLOAT ();
		quat[1] = MD5FLOAT ();
		quat[2] = MD5FLOAT ();
		quat[3] = 1 - DotProduct (quat, quat);
		if (quat[3] < 0)
			quat[3] = 0; // we have no imagination.
		quat[3] = -sqrtf (quat[3]);
		MD5EXPECT (")");

		GenMatrixPosQuat4Scale (pos, quat, scale, joint_poses[j].mat);
		Matrix3x4_Invert_Simple (joint_poses[j].mat, joint_infos[j].inverse.mat); // absolute, so we can just invert now.
	}

	if (strcmp (com_token, "}"))
		MD5ERROR ("Mod_LoadMD5MeshModel(%s): expected \"%s\", found \"%s\"\n", fname, "}", com_token);
	if (!MD5Anim_Load (&anim, joint_infos, joint_poses, numjoints))
		goto error;
	buffer = COM_Parse (buffer);

	// 2. Compute absolute animation joints:
	for (size_t pose_index = 0; pose_index < anim.numposes; ++pose_index)
	{
		const jointpose_t *in_pose = anim.posedata + (pose_index * anim.numjoints);
		jointpose_t		  *out_pose = skinning_joints + (pose_index * anim.numjoints);
		for (size_t joint_index = 0; joint_index < anim.numjoints; ++joint_index)
		{
			ssize_t poseparent = joint_infos[joint_index].poseparent;

			if (poseparent >= (ssize_t)joint_index)
				MD5ERROR ("%s: joint has bad pose parent order\n", fname);
			// concat it onto the parent (relative->abs)
			if (poseparent < 0)
				memcpy (concat_joints[joint_index].mat, in_pose[joint_index].mat, sizeof (jointpose_t));
			else
				R_ConcatTransforms ((void *)concat_joints[poseparent].mat, (void *)in_pose[joint_index].mat, (void *)concat_joints[joint_index].mat);
			memcpy (out_pose[joint_index].mat, concat_joints[joint_index].mat, sizeof (jointpose_t));
		}
	}
	SAFE_FREE (anim.posedata);

	// 3. each mesh has its own aliashdr_t : load vertices, triangles, textures...etc. and upload to GPU each surface:

	for (int m = 0; m < nummeshes; m++)
	{
		MD5EXPECT ("mesh");
		MD5EXPECT ("{");

		// go to the  surf, a.k.a mesh, chaining the next nextsurface
		surf = (aliashdr_t *)((byte *)outhdr + m * hdrsize);
		if (m + 1 < nummeshes)
			surf->nextsurface = (aliashdr_t *)((byte *)outhdr + (m + 1) * hdrsize);
		else
			surf->nextsurface = NULL;

		for (size_t j = 0; j < 3; j++)
		{
			surf->scale_origin[j] = 0;
			surf->scale[j] = 1.0;
		}

		surf->numjoints = numjoints;

		if (anim.numposes)
		{
			for (size_t j = 0; j < anim.numposes; j++)
			{
				surf->frames[j].firstpose = j;
				surf->frames[j].numposes = 1;
				surf->frames[j].interval = 0.1;
			}
			surf->numframes = anim.numposes;
		}

		//"shader" is the texture of the surf
		MD5EXPECT ("shader");
		char shader_name[MAX_QPATH];
		q_strlcpy (shader_name, (const char *)com_token, sizeof (shader_name));

		// MD5 have only 1 surface pose, meaning 1 vertex-like "pose" (not to ne mixed with md5animctx_t anim poses !)
		//  because it uses skeletal animation instead of displaying/interpolating different frames/poses of vertices
		surf->numposes = 1;

		buffer = COM_Parse (buffer);
		MD5EXPECT ("numverts");
		surf->numverts_vbo = surf->numverts = MD5UINT ();

		TEMP_ALLOC_ASSIGN_ZEROED (vinfo, surf->numverts);
		unsigned int max_mesh_influences = 0;

		while (MD5CHECK ("vert"))
		{
			size_t idx = MD5UINT ();
			if (idx >= (size_t)surf->numverts)
				MD5ERROR ("%s: vertex index out of bounds\n", fname);
			MD5EXPECT ("(");
			vinfo[idx].st[0] = MD5FLOAT ();
			vinfo[idx].st[1] = MD5FLOAT ();
			MD5EXPECT (")");
			vinfo[idx].firstweight = MD5UINT ();
			vinfo[idx].count = MD5UINT ();
			max_mesh_influences = q_max (max_mesh_influences, vinfo[idx].count);
		}

		surf->poseverttype = (max_mesh_influences > NUM_JOINT_INFLUENCES_4_WEIGHT) ? PV_MD5_8 : PV_MD5;
		const size_t md5_vertex_size = (surf->poseverttype == PV_MD5_8) ? sizeof (md5vert8_t) : sizeof (md5vert_t);
		TEMP_ALLOC_ASSIGN_ZEROED (poutvertexes, surf->numverts * md5_vertex_size);

		// MD5 violation: the skin is a single material. adding prefixes/postfixes here is the wrong thing to do.
		// but we do so anyway, because rerelease compat.
		surf->numskins = (int)Mod_LoadMD5SurfaceSkins (mod, surf, m, nummeshes, shader_name);

		if (surf->numskins == 0)
			Con_Warning ("MD5: %s, no skins found for surf '%s' (%d)\n", fname, shader_name, m);

		MD5EXPECT ("numtris");
		surf->numtris = MD5UINT ();
		surf->numindexes = surf->numtris * 3;
		TEMP_ALLOC_ASSIGN_ZEROED (poutindexes, surf->numindexes);

		while (MD5CHECK ("tri"))
		{
			size_t idx = MD5UINT ();
			if (idx >= (size_t)surf->numtris)
				MD5ERROR ("%s: triangle index out of bounds\n", fname);
			idx *= 3;
			for (size_t j = 0; j < 3; j++)
			{
				size_t t = MD5UINT ();
				if (t >= (size_t)surf->numverts)
					MD5ERROR ("%s: vertex index out of bounds\n", fname);
				poutindexes[idx + j] = t;
			}
		}

		// md5 is a gpu-unfriendly interchange format. :(
		MD5EXPECT ("numweights");
		size_t numweights = MD5UINT ();
		TEMP_ALLOC_ASSIGN_ZEROED (weight, numweights);

		while (MD5CHECK ("weight"))
		{
			size_t idx = MD5UINT ();
			if (idx >= numweights)
				MD5ERROR ("%s: weight index out of bounds\n", fname);

			weight[idx].joint_index = MD5UINT ();
			if (weight[idx].joint_index >= numjoints)
				MD5ERROR ("%s: joint index out of bounds\n", fname);
			weight[idx].pos[3] = MD5FLOAT ();
			MD5EXPECT ("(");
			weight[idx].pos[0] = MD5FLOAT () * weight[idx].pos[3];
			weight[idx].pos[1] = MD5FLOAT () * weight[idx].pos[3];
			weight[idx].pos[2] = MD5FLOAT () * weight[idx].pos[3];
			MD5EXPECT (")");
		}

		MD5EXPECT ("}");

		// so make it gpu-friendly.
		if (!MD5_BakeInfluences (fname, joint_poses, poutvertexes, surf->poseverttype, vinfo, weight, surf->numverts, numweights))
			goto error;
		// and now make up the normals that the format lacks. we'll still probably have issues from seams, but then so did qme, so at least its faithful...
		// :P
		MD5_ComputeNormals (poutvertexes, md5_vertex_size, surf->numverts, poutindexes, surf->numindexes);

		for (size_t j = 0; j < (size_t)surf->numverts; j++)
		{
			md5vert_t *poutvertex = (md5vert_t *)(poutvertexes + j * md5_vertex_size);
			for (size_t k = 0; k < 3; k++)
			{
				mod->mins[k] = q_min (mod->mins[k], poutvertex->xyz[k]);
				mod->maxs[k] = q_max (mod->maxs[k], poutvertex->xyz[k]);
			}
			dist = poutvertex->xyz[0] * poutvertex->xyz[0] + poutvertex->xyz[1] * poutvertex->xyz[1];
			if (yawradius < dist)
				yawradius = dist;
			dist += poutvertex->xyz[2] * poutvertex->xyz[2];
			if (radius < dist)
				radius = dist;
		}

		TEMP_FREE (weight);
		TEMP_FREE (vinfo);

		// Upload to GPU that surface/mesh m:
		GLMesh_UploadBuffers (mod, surf, poutindexes, (byte *)poutvertexes, NULL, skinning_joints);

		TEMP_FREE (poutvertexes);
		TEMP_FREE (poutindexes);

	} // end foreach mesh

	// the MD5 format does not have its own modelflags, yet we still need to know about trails and rotating etc
	mod->flags = MD5_HackyModelFlags (mod->name);

	mod->synctype = ST_FRAMETIME; // keep MD5 animations synced to when .frame is changed. framegroups are otherwise not very useful.
	mod->type = mod_alias;
	mod->extradata[PV_MD5] = (byte *)outhdr;

	radius = sqrtf (radius);
	mod->rmins[0] = mod->rmins[1] = mod->rmins[2] = -radius;
	mod->rmaxs[0] = mod->rmaxs[1] = mod->rmaxs[2] = radius;

	yawradius = sqrtf (yawradius);
	mod->ymins[0] = mod->ymins[1] = -yawradius;
	mod->ymaxs[0] = mod->ymaxs[1] = yawradius;
	mod->ymins[2] = mod->mins[2];
	mod->ymaxs[2] = mod->maxs[2];

	TEMP_FREE (concat_joints);
	TEMP_FREE (skinning_joints);

	TEMP_FREE (joint_poses);
	TEMP_FREE (joint_infos);

	return true;

error:
	// Recoverable replacement-model failures fall back to the MDL, so release
	// any partial MD5 state that Sys_Error used to abandon by terminating.
	TEMP_FREE (weight);
	TEMP_FREE (vinfo);
	TEMP_FREE (poutvertexes);
	TEMP_FREE (poutindexes);
	SAFE_FREE (anim.posedata);
	if (outhdr)
	{
		for (size_t surface_index = 0; surface_index < nummeshes; surface_index++)
		{
			aliashdr_t *hdr = (aliashdr_t *)((byte *)outhdr + surface_index * hdrsize);
			for (int skin_index = 0; skin_index < MAX_SKINS; skin_index++)
				SAFE_FREE (hdr->texels[skin_index]);
		}
		GLMesh_DeleteMeshBuffers (outhdr);
		Mem_Free (outhdr);
	}
	TEMP_FREE (concat_joints);
	TEMP_FREE (skinning_joints);
	TEMP_FREE (joint_poses);
	TEMP_FREE (joint_infos);
	return false;
}

qboolean Mod_LoadMD5MeshModel (qmodel_t *mod, const void *buffer)
{
	const char *fname = mod->name;
	size_t		numjoints;
	size_t		nummeshes;

	buffer = COM_Parse (buffer);

	MD5EXPECT ("MD5Version");
	MD5EXPECT (MD5_VERSION);
	if (MD5CHECK ("commandline"))
		buffer = COM_Parse (buffer);
	MD5EXPECT ("numJoints");
	numjoints = MD5UINT ();
	MD5EXPECT ("numMeshes");
	nummeshes = MD5UINT ();

	if (numjoints <= 0)
		MD5ERROR ("%s has no joints\n", mod->name);
	if (nummeshes <= 0)
		MD5ERROR ("%s has no meshes\n", mod->name);

	if (strcmp (com_token, "joints"))
		MD5ERROR ("Mod_LoadMD5MeshModel(%s): expected \"%s\", found \"%s\"\n", fname, "joints", com_token);

	return Mod_LoadMD5MeshModelData (mod, buffer, numjoints, nummeshes);

error:
	return false;
}

/*
=====================
Mod_LoadMD3Model
=====================
*/
void Mod_LoadMD3Model (qmodel_t *mod, const void *buffer)
{
	aliashdr_t	   *outhdr, *surf;
	md3Header_t	   *pinheader;
	md3Frame_t	   *pinframes;
	md3Triangle_t  *pintriangle;
	md3XyzNormal_t *pinvertexes;
	md3St_t		   *pinst;
	size_t			hdrsize;
	int				numsurfs;
	int				numframes;

	pinheader = (md3Header_t *)buffer;

	int version = LittleLong (pinheader->version);
	if (version != MD3_VERSION)
		Sys_Error ("MD3: %s has wrong version number (%d should be %d)", mod->name, version, MD3_VERSION);

	numsurfs = LittleLong (pinheader->numSurfaces);
	numframes = LittleLong (pinheader->numFrames);

	if (numframes > MAXALIASFRAMES)
		Sys_Error ("MD3: %s has too many frames (%i vs %i)", mod->name, numframes, MAXALIASFRAMES);

	if (!numsurfs)
		Sys_Error ("MD3: %s has no surfaces", mod->name);

	if (numsurfs > MAX_SURFACES)
		Sys_Error ("MD3: %s has too many surfaces : %d (max %d)", mod->name, numsurfs, MAX_SURFACES);

	// Collect the skin definitions from .skin files, if any;
	all_surfaces_def_t *surf_def = Mem_Alloc (sizeof (all_surfaces_def_t));

	Mod_LoadMD3SkinDefinitions (mod, surf_def);

	pinframes = (md3Frame_t *)((byte *)buffer + LittleLong (pinheader->ofsFrames));

	hdrsize = sizeof (*outhdr) - sizeof (outhdr->frames);
	hdrsize += sizeof (outhdr->frames) * numframes;

	// alloc all aliashdr_t and their chained nextsurface, a.k.a numsurfs, in one array
	outhdr = (aliashdr_t *)Mem_Alloc (hdrsize * numsurfs);

	// total_numverts and total_vertexes accumulate all vertices of the surface,
	// just to be able to Mod_CalcAliasBounds at the end.
	size_t			total_numverts = 0;
	md3XyzNormal_t *total_vertexes = NULL;

	// for each of the surfaces :
	md3Surface_t *pinsurface = (md3Surface_t *)((byte *)buffer + LittleLong (pinheader->ofsSurfaces));
	for (int m = 0; m < numsurfs; m++)
	{
		if (LittleLong (pinsurface->ident) != IDMD3HEADER)
			Sys_Error ("MD3: %s corrupt surface ident", mod->name);
		if (LittleLong (pinsurface->numFrames) != numframes)
			Sys_Error ("MD3: %s mismatched framecounts", mod->name);

		// go to the surf, chaining the next nextsurface
		surf = (aliashdr_t *)((byte *)outhdr + m * hdrsize);
		if (m + 1 < numsurfs)
			surf->nextsurface = (aliashdr_t *)((byte *)outhdr + (m + 1) * hdrsize);
		else
			surf->nextsurface = NULL;

		surf->poseverttype = PV_QUAKE3;

		// the number of vertices per-frame:
		surf->numverts_vbo = surf->numverts = LittleLong (pinsurface->numVerts);

		// All the vertices for this surface, concat of the vertices of each of the numframes, one frame after another:
		pinvertexes = (md3XyzNormal_t *)((byte *)pinsurface + LittleLong (pinsurface->ofsXyzNormals));

		md3XyzNormal_t *poutvertexes = (md3XyzNormal_t *)Mem_Alloc (numframes * surf->numverts * sizeof (*poutvertexes));
		// keep track of the original poutvertexes, because we are going to pointer arithmetic below...
		md3XyzNormal_t *poutvertexes_start = poutvertexes;

		// Load skins for that surface m:
		surf->numskins = Mod_LoadMD3SurfaceSkins (mod, surf, surf_def, q_strtrim (pinsurface->name), m, numsurfs, MAX_SKINS);

		if (surf->numskins == 0)
			Con_Warning ("MD3: %s, no skins found for surf '%s' (%d)\n", mod->name, pinsurface->name, m);

		// for each frame:
		// only 1 pose for MD3, it have frames instead
		surf->numposes = 1;

		for (int ival = 0; ival < numframes; ival++)
		{
			surf->frames[ival].firstpose = ival;
			surf->frames[ival].numposes = 1;
			surf->frames[ival].interval = 0.1;

			q_strlcpy (surf->frames[ival].name, pinframes->name, sizeof (surf->frames[ival].name));

			for (int j = 0; j < 3; j++)
			{ // fixme...
				surf->frames[ival].bboxmin.v[j] = 0;
				surf->frames[ival].bboxmax.v[j] = 255;
			}

			for (int j = 0; j < surf->numverts; j++)
				poutvertexes[j] = pinvertexes[j];

			poutvertexes += surf->numverts;
			pinvertexes += surf->numverts;
		}
		surf->numframes = numframes;

		surf->numtris = LittleLong (pinsurface->numTriangles);
		surf->numindexes = surf->numtris * 3;

		pintriangle = (md3Triangle_t *)((byte *)pinsurface + LittleLong (pinsurface->ofsTriangles));

		unsigned short *poutindexes = (unsigned short *)Mem_Alloc (sizeof (*poutindexes) * surf->numindexes);
		// keep track of the original poutindexes, because we are going to pointer arithmetic below...
		unsigned short *poutindexes_start = poutindexes;

		for (int ival = 0; ival < surf->numtris; ival++, pintriangle++, poutindexes += 3)
		{
			for (int j = 0; j < 3; j++)
				poutindexes[j] = LittleLong (pintriangle->indexes[j]);
		}

		for (int j = 0; j < 3; j++)
		{
			surf->scale_origin[j] = 0;
			surf->scale[j] = MD3_XYZ_SCALE;
		}

		// TODO: What to do with the shaders ?
		// int numshaders = pinsurface->numShaders;
		// md3Shader_t	   * pinshader = (md3Shader_t *)((byte *)pinsurface + LittleLong (pinsurface->ofsShaders));

		// and figure out the texture coords properly, now we know the actual sizes.
		pinst = (md3St_t *)((byte *)pinsurface + LittleLong (pinsurface->ofsSt));

		aliasmesh_t *poutst = (aliasmesh_t *)Mem_Alloc (sizeof (*poutst) * surf->numverts);

		for (int j = 0; j < surf->numverts; j++)
		{
			poutst[j].vertindex = j; // how is this useful?
			poutst[j].st[0] = pinst[j].s;
			poutst[j].st[1] = pinst[j].t;
		}

		// Upload to GPU that surface/mesh m:
		GLMesh_UploadBuffers (mod, surf, poutindexes_start, (byte *)poutvertexes_start, poutst, NULL);

		// concat surface vertices to total_vertexes
		total_vertexes = (md3XyzNormal_t *)Mem_Realloc (total_vertexes, sizeof (*poutvertexes) * (total_numverts + surf->numverts));
		memcpy ((void *)(total_vertexes + total_numverts), (const void *)poutvertexes_start, sizeof (*poutvertexes) * surf->numverts);
		total_numverts += surf->numverts;

		Mem_Free (poutst);
		Mem_Free (poutvertexes_start);
		Mem_Free (poutindexes_start);

		// go to the next surface:
		pinsurface = (md3Surface_t *)((byte *)pinsurface + LittleLong (pinsurface->ofsEnd));

	} // end for surface

	// small violation of the spec, but it seems like noone else uses it.
	mod->flags = LittleLong (pinheader->flags);

	mod->type = mod_alias;
	mod->extradata[PV_QUAKE3] = (byte *)outhdr;

	// calc alias bounds of the whole surfaces model :
	Mod_CalcAliasBounds (mod, outhdr, total_numverts, (byte *)total_vertexes); // johnfitz

	Mem_Free (total_vertexes);
	Mem_Free (surf_def);
}
#endif // !USE_RUST_FORMATS
