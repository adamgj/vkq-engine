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
// models.c -- model loading and caching

// models are the only shared resource between a client and server running
// on the same machine.

#include "quakedef.h"
#include "model_parse.h"

static void		 Mod_LoadBrushModel (qmodel_t *mod, const char *loadname, void *buffer);
static void		 Mod_LoadAliasModel (qmodel_t *mod, void *buffer);
static qmodel_t *Mod_LoadModel (qmodel_t *mod, qboolean crash);
static void		 Mod_FreeModelMemory (qmodel_t *mod);

cvar_t external_ents = {"external_ents", "1", CVAR_ARCHIVE};
cvar_t external_vis = {"external_vis", "1", CVAR_ARCHIVE};

// wad_external_textures = 1 enable loading of external WAD textures, 0 to forbid it for debug purposes.
cvar_t wad_external_textures = {"wad_external_textures", "1", CVAR_NONE};

// mdl_external_textures = 1 enable loading of external MDL textures, 0 to forbid it for debug purposes.
cvar_t mdl_external_textures = {"mdl_external_textures", "1", CVAR_NONE};

// r_allow_replacement_md5models = 1 allow loading of MD5 replacement models if available, 0 to forbid it for debug purposes.
cvar_t r_allow_replacement_md5models = {"r_allow_replacement_md5models", "1", CVAR_NONE};

// r_allow_replacement_md3models = 1 allow loading of MD3 replacement models if available, 0 to forbid it for debug purposes.
cvar_t r_allow_replacement_md3models = {"r_allow_replacement_md3models", "1", CVAR_NONE};

cvar_t r_enhancedmodels = {"r_enhancedmodels", "1", CVAR_ARCHIVE}; // controlled in Menu with Models: enhanced (1) / classic (0)

qmodel_t mod_known[MAX_MODELS];
int		 mod_numknown;

texture_t *r_notexture_mip;	 // johnfitz -- moved here from r_main.c
texture_t *r_notexture_mip2; // johnfitz -- used for non-lightmapped surfs with a missing texture

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
Mod_RefreshSkins_f
===============
*/
void Mod_RefreshSkins_f (cvar_t *var)
{
	for (int i = 0; i < cl.maxclients; ++i)
		R_TranslateNewPlayerSkin (i);
}

/*
===============
Mod_EnhancedModels_f
===============
*/
static void Mod_EnhancedModels_f (cvar_t *var)
{
	int		  i;
	qmodel_t *mod;

	R_FreeAllEntityBLASes ();

	for (i = 0, mod = mod_known; i < mod_numknown; i++, mod++)
	{
		if (mod->type != mod_alias)
			continue;

		for (int j = 0; j < PV_SIZE; ++j)
			GLMesh_DeleteMeshBuffers ((aliashdr_t *)mod->extradata[j]);
		Mod_FreeModelMemory (mod);
		mod->needload = true;
	}

	for (i = 0, mod = mod_known; i < mod_numknown; i++, mod++)
	{
		if (mod->type == mod_alias)
			Mod_LoadModel (mod, false);
	}

	Mod_RefreshSkins_f (var);
	R_RebuildAllEfrags ();
	InvalidateTraceLineCache ();
}

/*
===============
Mod_Init
===============
*/
void Mod_Init (void)
{
	Cvar_RegisterVariable (&external_vis);
	Cvar_RegisterVariable (&external_ents);
	Cvar_RegisterVariable (&wad_external_textures);
	Cvar_RegisterVariable (&mdl_external_textures);
	Cvar_RegisterVariable (&r_allow_replacement_md5models);
	Cvar_RegisterVariable (&r_allow_replacement_md3models);
	Cvar_RegisterVariable (&r_enhancedmodels);
	Cvar_SetCallback (&r_enhancedmodels, Mod_EnhancedModels_f);

	// johnfitz -- create notexture miptex
	r_notexture_mip = (texture_t *)Mem_Alloc (sizeof (texture_t));
	strcpy (r_notexture_mip->name, "notexture");
	r_notexture_mip->height = r_notexture_mip->width = 32;

	r_notexture_mip2 = (texture_t *)Mem_Alloc (sizeof (texture_t));
	strcpy (r_notexture_mip2->name, "notexture2");
	r_notexture_mip2->height = r_notexture_mip2->width = 32;
	// johnfitz
}

const char *MODEL_TYPE_STR (poseverttype_t kind)
{
	switch (kind)
	{
	case PV_QUAKE1:
		return "MDL";
		break;
	case PV_MD5:
	case PV_MD5_8:
		return "MD5";
		break;
	case PV_QUAKE3:
		return "MD3";
		break;
	default:
		return "(invalid)";
	}
}
/*
===============
Mod_Extradata_CheckSkin

Caches the data if needed
===============
*/
void *Mod_Extradata_CheckSkin (qmodel_t *mod, int skinnum)
{
	Mod_LoadModel (mod, true);

	if (mod->type != mod_alias)
		return mod->extradata[PV_QUAKE1];

	poseverttype_t valid_models_with_prio[PV_SIZE] = {0};
	int			   id_models_with_prio_size = 0;

	// 1. fill valid_models_with_prio in the order (MD3, MD5, MDL) selecting non-null extradata
	// there are probably smarter things to do but let's not over-engeneer this and trust the compiler instead
	for (size_t i = 0; i < PV_SIZE; i++)
	{
		if (mod->extradata[i] && (i == PV_QUAKE3))
		{
			valid_models_with_prio[id_models_with_prio_size++] = PV_QUAKE3;
			break;
		}
	}
	for (size_t i = 0; i < PV_SIZE; i++)
	{
		if (mod->extradata[i] && (i == PV_MD5))
		{
			valid_models_with_prio[id_models_with_prio_size++] = PV_MD5;
			break;
		}
	}
	for (size_t i = 0; i < PV_SIZE; i++)
	{
		if (mod->extradata[i] && (i == PV_QUAKE1))
		{
			valid_models_with_prio[id_models_with_prio_size++] = PV_QUAKE1;
			break;
		}
	}

	// by construction of Mod_LoadModel we only have 2 models at most
	assert (id_models_with_prio_size <= 2);

	byte *mdx_extradata = mod->extradata[valid_models_with_prio[0]];

	// 2. Only one model, return it whatever its kind.
	if (id_models_with_prio_size == 1)
		return mdx_extradata;

	// 3. Apply the dynamic rule MDL vs. MDX now:
	if (r_enhancedmodels.value && skinnum < ((aliashdr_t *)mdx_extradata)->numskins)
		return mdx_extradata;
	//
	return mod->extradata[PV_QUAKE1];
}

/*
===============
Mod_Extradata

Caches the data if needed
===============
*/
void *Mod_Extradata (qmodel_t *mod)
{
	return Mod_Extradata_CheckSkin (mod, 0);
}

/*
===============
Mod_PointInLeaf
===============
*/
mleaf_t *Mod_PointInLeaf (float *p, qmodel_t *model)
{
	mnode_t	 *node;
	float	  d;
	mplane_t *plane;

	if (!model || !model->nodes)
		Sys_Error ("Mod_PointInLeaf: bad model");

	node = model->nodes;
	while (1)
	{
		if (node->contents < 0)
			return (mleaf_t *)node;
		plane = node->plane;
		d = DotProduct (p, plane->normal) - plane->dist;
		if (d > 0)
			node = node->children[0];
		else
			node = node->children[1];
	}

	return NULL; // never reached
}

static byte *mod_novis;
static int	 mod_novis_capacity;

/*
===================
Mod_LeafPVS
===================
*/
byte *Mod_LeafPVS (mleaf_t *leaf, qmodel_t *model)
{
	if (leaf == model->leafs)
		return Mod_NoVisPVS (model);
	return Mod_DecompressVis (leaf->compressed_vis, model);
}

/*
===================
Mod_NoVisPVS
===================
*/
byte *Mod_NoVisPVS (qmodel_t *model)
{
	int pvsbytes;

	pvsbytes = (model->numleafs + 31) / 8;
	if (mod_novis == NULL || pvsbytes > mod_novis_capacity)
	{
		mod_novis_capacity = pvsbytes;
		mod_novis = (byte *)Mem_Realloc (mod_novis, mod_novis_capacity);
		if (!mod_novis)
			Sys_Error ("Mod_NoVisPVS: realloc() failed on %d bytes", mod_novis_capacity);
	}
	memset (mod_novis, 0xff, mod_novis_capacity);
	return mod_novis;
}

/*
===================
Mod_FreeSpriteMemory
===================
*/
static void Mod_FreeSpriteMemory (msprite_t *psprite)
{
	for (int i = 0; i < psprite->numframes; ++i)
	{
		if (psprite->frames[i].type == SPR_SINGLE)
		{
			SAFE_FREE (psprite->frames[i].frameptr);
		}
		else
		{
			mspritegroup_t *group = (mspritegroup_t *)psprite->frames[i].frameptr;
			for (int j = 0; j < group->numframes; ++j)
			{
				SAFE_FREE (group->frames[i]);
			}
			SAFE_FREE (psprite->frames[i].frameptr);
		}
	}
	psprite->numframes = 0;
}

/*
===================
Mod_FreeModelMemory
===================
*/
static void Mod_FreeModelMemory (qmodel_t *mod)
{
	if (mod->name[0] != '*')
	{
		if ((mod->type == mod_sprite) && (mod->extradata[PV_QUAKE1]))
			Mod_FreeSpriteMemory ((msprite_t *)mod->extradata[PV_QUAKE1]);
		// Last two ones are dummy textures
		for (int i = 0; i < mod->numtextures - 2; ++i)
			SAFE_FREE (mod->textures[i]);
		for (int i = 0; i < mod->numsurfaces; ++i)
			SAFE_FREE (mod->surfaces[i].polys);
		SAFE_FREE (mod->hulls[0].clipnodes);
		SAFE_FREE (mod->submodels);
		mod->numsubmodels = 0;
		SAFE_FREE (mod->planes);
		mod->numplanes = 0;
		SAFE_FREE (mod->leafs);
		mod->numleafs = 0;
		SAFE_FREE (mod->vertexes);
		mod->numvertexes = 0;
		SAFE_FREE (mod->edges);
		mod->numedges = 0;
		SAFE_FREE (mod->nodes);
		mod->numnodes = 0;
		SAFE_FREE (mod->texinfo);
		mod->numtexinfo = 0;
		SAFE_FREE (mod->surfaces);
		mod->numsurfaces = 0;
		SAFE_FREE (mod->surfedges);
		mod->numsurfedges = 0;
		SAFE_FREE (mod->clipnodes);
		mod->numclipnodes = 0;
		SAFE_FREE (mod->marksurfaces);
		mod->nummarksurfaces = 0;
		SAFE_FREE (mod->soa_leafbounds);
		SAFE_FREE (mod->surfvis);
		SAFE_FREE (mod->soa_surfplanes);
		SAFE_FREE (mod->textures);
		mod->numtextures = 0;
		SAFE_FREE (mod->visdata);
		SAFE_FREE (mod->lightdata);
		SAFE_FREE (mod->entities);
		for (int i = 0; i < PV_SIZE; ++i)
			SAFE_FREE (mod->extradata[i]);
		SAFE_FREE (mod->water_surfs);
		mod->used_water_surfs = 0;
		mod->water_surfs_specials = 0;
	}
	else
		SAFE_FREE (mod->textures);

	if (!no_rendering)
		TexMgr_FreeTexturesForOwner (mod);
}

/*
===================
Mod_ClearAll
===================
*/
void Mod_ClearAll (void)
{
	int		  i;
	qmodel_t *mod;
	GL_DeleteBModelAccelerationStructures ();

	for (i = 0, mod = mod_known; i < mod_numknown; i++, mod++)
	{
		if (mod->type != mod_alias)
		{
			mod->needload = true;
			Mod_FreeModelMemory (mod); // johnfitz
		}
	}

	InvalidateTraceLineCache ();
}

/*
===================
Mod_ResetAll
===================
*/
void Mod_ResetAll (void)
{
	int		  i;
	qmodel_t *mod;

	// ericw -- free alias model VBOs
	GLMesh_DeleteAllMeshBuffers ();
	GL_DeleteBModelAccelerationStructures ();

	for (i = 0, mod = mod_known; i < mod_numknown; i++, mod++)
	{
		if (!mod->needload) // otherwise Mod_ClearAll() did it already
			Mod_FreeModelMemory (mod);

		memset (mod, 0, sizeof (qmodel_t));
	}
	mod_numknown = 0;

	InvalidateTraceLineCache ();
}

/*
==================
Mod_FindName

==================
*/
qmodel_t *Mod_FindName (const char *name)
{
	int		  i;
	qmodel_t *mod;

	if (!name[0])
		Sys_Error ("Mod_FindName: NULL name"); // johnfitz -- was "Mod_ForName"

	//
	// search the currently loaded models
	//
	for (i = 0, mod = mod_known; i < mod_numknown; i++, mod++)
		if (!strcmp (mod->name, name))
			break;

	if (i == mod_numknown)
	{
		if (mod_numknown == MAX_MODELS)
			Sys_Error ("mod_numknown == MAX_MODELS");
		q_strlcpy (mod->name, name, MAX_QPATH);
		mod->needload = true;
		mod_numknown++;
		InvalidateTraceLineCache ();
	}

	return mod;
}

/*
==================
Mod_TouchModel

==================
*/
void Mod_TouchModel (const char *name)
{
	Mod_FindName (name);
}

/*
==================
Mod_LoadModel

Loads a model into the cache
==================
*/
static qmodel_t *Mod_LoadModel (qmodel_t *mod, qboolean crash)
{
	int mod_type;

	if (!mod->needload)
		return mod;

	InvalidateTraceLineCache ();

	if (mod->type == mod_alias)
	{
		for (int i = 0; i < PV_SIZE; ++i)
		{
			GLMesh_DeleteMeshBuffers ((aliashdr_t *)mod->extradata[i]);
		}
	}

	// load the model file, together with replacement overrides for .mdl, if they are available.
	// 0 is an invalid path_id, starts at 1 for existing files:
	unsigned int md5_enhanced_path_id = 0;
	unsigned int md3_enhanced_path_id = 0;

	byte *buf = NULL;

	char md3_name[MAX_QPATH], md5_name[MAX_QPATH];

	// 1. Load the original model buffer:
	buf = COM_LoadFile (mod->name, &mod->path_id);

	if (!buf)
	{
		if (crash)
			Host_Error ("Mod_LoadModel: %s not found", mod->name); // johnfitz -- was "Mod_NumForName"
		return NULL;
	}

	const bool mod_is_mdl = (strcmp (COM_FileGetExtension (mod->name), "mdl") == 0);
	const bool load_enhanced_model = mod_is_mdl && r_enhancedmodels.value;

	// 2. Find MDL "enhanced" complementary models, if any:
	if (load_enhanced_model && r_allow_replacement_md3models.value)
	{
		// newname is the .mdl model with extension changed to .md3:
		COM_StripExtension (mod->name, md3_name, sizeof (md3_name));
		COM_AddExtension (md3_name, ".md3", sizeof (md3_name));

		// Search for the file but do not load it:
		//   look for it in the filesystem or pack files
		if (!COM_FileExists (md3_name, &md3_enhanced_path_id))
			md3_enhanced_path_id = 0; // file not found

		// this is a replacement only if its priority is >= MDL one, else discard it
		if (md3_enhanced_path_id < mod->path_id)
		{
			md3_enhanced_path_id = 0;
		}
	}

	if (load_enhanced_model && r_allow_replacement_md5models.value)
	{
		// newname is the .mdl model with extension changed to .md5mesh:
		COM_StripExtension (mod->name, md5_name, sizeof (md5_name));
		COM_AddExtension (md5_name, ".md5mesh", sizeof (md5_name));

		// Search for the file but do not load it:
		//   look for it in the filesystem or pack files
		if (!COM_FileExists (md5_name, &md5_enhanced_path_id))
			md5_enhanced_path_id = 0; // file not found

		// this is a replacement only if its priority is >= MDL one, else discard it
		if (md5_enhanced_path_id < mod->path_id)
		{
			md5_enhanced_path_id = 0;
		}
	}

	// 3. If there are multiple replacement models (MD3 + MD5) only keep the one with the highest prio
	//  in case of equality, MD3 wins.
	if (md3_enhanced_path_id && md5_enhanced_path_id)
	{
		if (md5_enhanced_path_id > md3_enhanced_path_id)
			md3_enhanced_path_id = 0;
		else
			md5_enhanced_path_id = 0;
	}

	// 4. Load the (unique) selected complementary model :
	if (md3_enhanced_path_id)
	{
		byte		*md3_buf = COM_LoadFile (md3_name, &md3_enhanced_path_id);
		// To assure that the external resources associated with MD3
		// are properly filtered/loaded, we need to set mod->path_id = md3_enhanced_path_id temporarilly
		unsigned int original_path_id = mod->path_id;
		mod->path_id = md3_enhanced_path_id;
		Mod_LoadMD3Model (mod, md3_buf);
		mod->path_id = original_path_id;
		Mem_Free (md3_buf);
	}
	else if (md5_enhanced_path_id)
	{
		byte		*md5_buf = COM_LoadFile (md5_name, &md5_enhanced_path_id);
		// To assure that the external resources associated with MD5
		// are properly filtered/loaded, we need to set mod->path_id = md5_enhanced_path_id temporarilly
		unsigned int original_path_id = mod->path_id;
		mod->path_id = md5_enhanced_path_id;
		Mod_LoadMD5MeshModel (mod, md5_buf);
		mod->path_id = original_path_id;
		Mem_Free (md5_buf);
	}

	// 5. Finally, Load the original model, calling the appropriate loader:
	mod->needload = false;

	mod_type = (buf[0] | (buf[1] << 8) | (buf[2] << 16) | (buf[3] << 24));
	switch (mod_type)
	{
	case IDPOLYHEADER:
		Mod_LoadAliasModel (mod, buf);
		break;

	case IDSPRITEHEADER:
		Mod_LoadSpriteModel (mod, buf);
		break;

	//
	case IDMD5HEADER:
	{
		// by construction this is a "native" MD5 model, NOT a .mdl replacement so md5_enhanced_path_id = 0 here
		assert (md5_enhanced_path_id == 0);
		if (!Mod_LoadMD5MeshModel (mod, (const void *)buf))
			Sys_Error ("Mod_LoadModel: failed to load %s", mod->name);
	}
	break;

	//
	case IDMD3HEADER:
	{
		// by construction this is a "native" MD3 model, NOT a .mdl replacement so md3_enhanced_path_id = 0 here
		assert (md3_enhanced_path_id == 0);
		Mod_LoadMD3Model (mod, (const void *)buf);
	}
	break;

	default:
	{
		char loadname[MAX_QPATH];
		COM_FileBase (mod->name, loadname, sizeof (loadname));
		Mod_LoadBrushModel (mod, loadname, buf);
	}
	break;
	}

	Mem_Free (buf);
	return mod;
}

/*
==================
Mod_ForName

Loads in a model for the given name
==================
*/
qmodel_t *Mod_ForName (const char *name, qboolean crash)
{
	qmodel_t *mod;

	mod = Mod_FindName (name);

	return Mod_LoadModel (mod, crash);
}

/*
===============================================================================

					BRUSHMODEL LOADING

===============================================================================
*/

/*
=============
Mod_LoadWadFiles

load all of the wads listed in the worldspawn "wad" field
=============
*/
static wad_t *Mod_LoadWadFiles (qmodel_t *mod)
{
	char		key[128], value[4096];
	const char *data;

	if (!wad_external_textures.value)
		return NULL;

	// disregard if this isn't the world model
	if (strcmp (mod->name, sv.modelname))
		return NULL;

	data = COM_Parse (mod->entities);
	if (!data)
		return NULL; // error
	if (com_token[0] != '{')
		return NULL; // error
	while (1)
	{
		data = COM_Parse (data);
		if (!data)
			return NULL; // error
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
			return NULL; // error
		q_strlcpy (value, com_token, sizeof (value));

		if (!strcmp ("wad", key))
		{
			return W_LoadWadList (value);
		}
	}
	return NULL;
}

/*
=================
Mod_LoadWadTexture

look for an external texture in any of the loaded map wads
=================
*/
texture_t *Mod_LoadWadTexture (qmodel_t *mod, wad_t *wads, const char *name)
{
	int			   i, pixels;
	lumpinfo_t	  *info;
	wad_t		  *wad;
	miptex_t	   mt;
	texture_t	  *tx;
	qboolean	   pal;
	unsigned short colors;

	// look for the lump in any of the loaded wads
	info = W_GetLumpinfoList (wads, name, &wad);

	// ensure we're dealing with a miptex
	if (!info || (info->type != TYP_MIPTEX && (wad->id != WADID_VALVE || info->type != TYP_MIPTEX_PALETTE)))
	{
		Con_Warning ("Missing external texture '%s' in wads, using BSP\n", name);
		return NULL;
	}

	// override the texture from the bsp file
	FS_fseek (&wad->fh, info->filepos, SEEK_SET);
	FS_fread (&mt, 1, sizeof (miptex_t), &wad->fh);

	mt.width = LittleLong (mt.width);
	mt.height = LittleLong (mt.height);
	for (i = 0; i < MIPLEVELS; i++)
		mt.offsets[i] = LittleLong (mt.offsets[i]);

	if (mt.width == 0 || mt.height == 0)
	{
		Con_Warning ("Zero sized texture %s in %s!\n", mt.name, wad->name);
		return NULL;
	}

	pal = wad->id == WADID_VALVE && info->type == TYP_MIPTEX_PALETTE;

	pixels = mt.width * mt.height / 64 * 85;
	// valve textures have a color palette immediately following the pixels
	if (pal)
	{
		if ((pixels + 2) <= info->size)
		{
			// the palette is basically garunteed to be 256 colors but,
			// we might as well use the value since it *does* exist
			FS_fseek (&wad->fh, info->filepos + pixels, SEEK_SET);
			FS_fread (&colors, 1, 2, &wad->fh);
			colors = LittleShort (colors);
			// add space for the color palette
			pixels += colors * 3;
		}
		// add space for the color count
		pixels += 2;
	}
	tx = (texture_t *)Mem_Alloc (sizeof (texture_t) + pixels);

	memcpy (tx->name, mt.name, sizeof (tx->name));
	tx->width = mt.width;
	tx->height = mt.height;
	for (i = 0; i < MIPLEVELS; i++)
		tx->offsets[i] = mt.offsets[i] + sizeof (texture_t) - sizeof (miptex_t);
	// the pixels immediately follow the structures

	// check for pixels extending past the end of the lump
	if (pixels > info->size)
	{
		Con_DPrintf ("Texture %s extends past end of lump\n", mt.name);
		pixels = info->size;
	}
	tx->source_file[0] = 0;
	tx->source_offset = (src_offset_t)(tx + 1);

	Atomic_StoreUInt32 (&tx->update_warp, false); // johnfitz
	tx->warpimage = NULL;						  // johnfitz
	tx->fullbright = NULL;						  // johnfitz
	tx->shift = 0;								  // Q64 only
	tx->palette = pal;

	FS_fseek (&wad->fh, info->filepos + sizeof (miptex_t), SEEK_SET);
	FS_fread (tx + 1, 1, pixels, &wad->fh);

	return tx;
}

/*
=================
Mod_CheckFullbrights -- johnfitz
=================
*/
qboolean Mod_CheckFullbrights (byte *pixels, int count)
{
	int i;
	for (i = 0; i < count; i++)
		if (*pixels++ > 223)
			return true;
	return false;
}

/*
=================
Mod_CheckFullbrightsValve
=================
*/
static qboolean Mod_CheckFullbrightsValve (char *name, byte *pixels, int count)
{
	if (name[0] == '~' || (name[2] == '~' && name[0] == '+'))
		return Mod_CheckFullbrights (pixels, count);
	return false;
}

/*
=================
Mod_CheckAnimTextureArrayQ64

Quake64 bsp
Check if we have any missing textures in the array
=================
*/
qboolean Mod_CheckAnimTextureArrayQ64 (texture_t *anims[], int numTex)
{
	int i;

	for (i = 0; i < numTex; i++)
	{
		if (!anims[i])
			return false;
	}
	return true;
}

/*
=================
Mod_LoadTextureTask
=================
*/
static void Mod_LoadTextureTask (int i, qmodel_t **ppmod)
{
	qmodel_t  *mod = *ppmod;
	texture_t *tx = mod->textures[i];
	if (!tx)
		return;

	int	  pixels = tx->width * tx->height / 64 * 85;
	char  texturename[64];
	int	  fwidth, fheight;
	char  filename[MAX_OSPATH], mapname[MAX_OSPATH];
	byte *data = NULL;
	bool  fbright;

	// Only filter out external textures for static models, not the level:
	const unsigned int effective_min_path_id = (mod->is_worldmodel ? 0 : mod->path_id);

#ifdef BSP29_VALVE
	if (mod->bspversion != BSPVERSION_VALVE && !q_strncasecmp (tx->name, "sky", 3))
#else
	if (!q_strncasecmp (tx->name, "sky", 3)) // sky texture //also note -- was strncmp, changed to match qbsp
#endif
	{
		if (mod->bspversion == BSPVERSION_QUAKE64)
			Sky_LoadTextureQ64 (mod, tx, i);
		else
			Sky_LoadTexture (mod, tx, i);
	}
	else if (tx->name[0] == '*' || tx->name[0] == '!') // warping texture
	{
		// external textures -- first look in "textures/mapname/" then look in "textures/"
		COM_StripExtension (mod->name + 5, mapname, sizeof (mapname));
		q_snprintf (filename, sizeof (filename), "textures/%s/#%s", mapname, tx->name + 1); // this also replaces the '*' with a '#'
		enum srcformat fmt = SRC_RGBA;
		data = Image_LoadImage (filename, &fwidth, &fheight, &fmt, effective_min_path_id);
		if (!data)
		{
			q_snprintf (filename, sizeof (filename), "textures/#%s", tx->name + 1);
			data = Image_LoadImage (filename, &fwidth, &fheight, &fmt, effective_min_path_id);
		}

		// now load whatever we found
		if (data) // load external image
		{
			q_strlcpy (texturename, filename, sizeof (texturename));
			tx->gltexture = TexMgr_LoadImage (mod, texturename, fwidth, fheight, fmt, data, filename, 0, TEXPREF_NONE);
		}
		else // use the texture from the bsp file
		{
			q_snprintf (texturename, sizeof (texturename), "%s:%s", mod->name, tx->name);
			fmt = SRC_INDEXED;
			if (tx->palette)
				fmt = SRC_INDEXED_PALETTE;
			tx->gltexture = TexMgr_LoadImage (mod, texturename, tx->width, tx->height, fmt, (byte *)(tx + 1), tx->source_file, tx->source_offset, TEXPREF_NONE);
		}

		// now create the warpimage, using dummy data from the hunk to create the initial image
		q_snprintf (texturename, sizeof (texturename), "%s_warp", texturename);
		tx->warpimage = TexMgr_LoadImage (mod, texturename, WARPIMAGESIZE, WARPIMAGESIZE, SRC_RGBA, NULL, "", 0, TEXPREF_NOPICMIP | TEXPREF_WARPIMAGE);
		Atomic_StoreUInt32 (&tx->update_warp, true);
	}
	else // regular texture
	{
		// ericw -- fence textures
		int extraflags;

		extraflags = 0;
		if (tx->name[0] == '{')
			extraflags |= TEXPREF_ALPHA;
		// ericw

		// external textures -- first look in "textures/mapname/" then look in "textures/"
		COM_StripExtension (mod->name + 5, mapname, sizeof (mapname));
		q_snprintf (filename, sizeof (filename), "textures/%s/%s", mapname, tx->name);
		enum srcformat fmt = SRC_RGBA;
		data = Image_LoadImage (filename, &fwidth, &fheight, &fmt, effective_min_path_id);
		if (!data)
		{
			q_snprintf (filename, sizeof (filename), "textures/%s", tx->name);
			data = Image_LoadImage (filename, &fwidth, &fheight, &fmt, effective_min_path_id);
		}

		// now load whatever we found
		if (data) // load external image
		{
			char filename2[MAX_OSPATH];

			tx->gltexture = TexMgr_LoadImage (mod, filename, fwidth, fheight, fmt, data, filename, 0, TEXPREF_MIPMAP | extraflags);
			Mem_Free (data);

			// now try to load glow/luma image from the same place
			q_snprintf (filename2, sizeof (filename2), "%s_glow", filename);
			data = Image_LoadImage (filename2, &fwidth, &fheight, &fmt, effective_min_path_id);
			if (!data)
			{
				q_snprintf (filename2, sizeof (filename2), "%s_luma", filename);
				data = Image_LoadImage (filename2, &fwidth, &fheight, &fmt, effective_min_path_id);
			}

			if (data)
				tx->fullbright = TexMgr_LoadImage (mod, filename2, fwidth, fheight, fmt, data, filename2, 0, TEXPREF_MIPMAP | extraflags);
		}
		else // use the texture from the bsp file
		{
			q_snprintf (texturename, sizeof (texturename), "%s:%s", mod->name, tx->name);
			if (tx->palette)
			{
				fmt = SRC_INDEXED_PALETTE;
				fbright = Mod_CheckFullbrightsValve (tx->name, (byte *)(tx + 1), pixels);
			}
			else
			{
				fmt = SRC_INDEXED;
				fbright = Mod_CheckFullbrights ((byte *)(tx + 1), pixels);
			}
			if (fbright)
			{
				tx->gltexture = TexMgr_LoadImage (
					mod, texturename, tx->width, tx->height, fmt, (byte *)(tx + 1), tx->source_file, tx->source_offset,
					TEXPREF_MIPMAP | TEXPREF_NOBRIGHT | extraflags);
				q_snprintf (texturename, sizeof (texturename), "%s:%s_glow", mod->name, tx->name);
				tx->fullbright = TexMgr_LoadImage (
					mod, texturename, tx->width, tx->height, fmt, (byte *)(tx + 1), tx->source_file, tx->source_offset,
					TEXPREF_MIPMAP | TEXPREF_FULLBRIGHT | extraflags);
			}
			else
			{
				tx->gltexture = TexMgr_LoadImage (
					mod, texturename, tx->width, tx->height, fmt, (byte *)(tx + 1), tx->source_file, tx->source_offset, TEXPREF_MIPMAP | extraflags);
			}
		}
	}
	Mem_Free (data);
}

/*
=================
Mod_LoadTextures
=================
*/
static void Mod_LoadTextures (qmodel_t *mod, byte *mod_base, lump_t *l)
{
	int		   i, j, num, maxanim, altmax;
	texture_t *tx, *tx2;
	texture_t *anims[10];
	texture_t *altanims[10];
	int		   nummiptex;
	wad_t	  *wads;

	// load any wads this map may need to load external textures from
	wads = Mod_LoadWadFiles (mod);

	Mod_ParseTextures (mod, mod_base, l, wads);
	nummiptex = mod->numtextures - 2;

	// we no longer need the wads after this point
	W_FreeWadList (wads);

	if (!no_rendering)
	{
		if (!Tasks_IsWorker () && (nummiptex > 1))
		{
			task_handle_t task = Task_AllocateAssignIndexedFuncAndSubmit ((task_indexed_func_t)Mod_LoadTextureTask, nummiptex, &mod, sizeof (mod));
			Task_Join (task, TASK_TIMEOUT_INFINITE);
		}
		else
		{
			for (i = 0; i < nummiptex; i++)
				Mod_LoadTextureTask (i, &mod);
		}
	}

	// johnfitz -- last 2 slots in array should be filled with dummy textures
	mod->textures[mod->numtextures - 2] = r_notexture_mip;	// for lightmapped surfs
	mod->textures[mod->numtextures - 1] = r_notexture_mip2; // for SURF_DRAWTILED surfs

	//
	// sequence the animations
	//
	for (i = 0; i < nummiptex; i++)
	{
		tx = mod->textures[i];
		if (!tx || tx->name[0] != '+')
			continue;
		if (tx->anim_next)
			continue; // allready sequenced

		// find the number of frames in the animation
		memset (anims, 0, sizeof (anims));
		memset (altanims, 0, sizeof (altanims));

		maxanim = tx->name[1];
		altmax = 0;
		if (maxanim >= 'a' && maxanim <= 'z')
			maxanim -= 'a' - 'A';
		if (maxanim >= '0' && maxanim <= '9')
		{
			maxanim -= '0';
			altmax = 0;
			anims[maxanim] = tx;
			maxanim++;
		}
		else if (maxanim >= 'A' && maxanim <= 'J')
		{
			altmax = maxanim - 'A';
			maxanim = 0;
			altanims[altmax] = tx;
			altmax++;
		}
		else
			Sys_Error ("Bad animating texture %s", tx->name);

		for (j = i + 1; j < nummiptex; j++)
		{
			tx2 = mod->textures[j];
			if (!tx2 || tx2->name[0] != '+')
				continue;
			if (strcmp (tx2->name + 2, tx->name + 2))
				continue;

			num = tx2->name[1];
			if (num >= 'a' && num <= 'z')
				num -= 'a' - 'A';
			if (num >= '0' && num <= '9')
			{
				num -= '0';
				anims[num] = tx2;
				if (num + 1 > maxanim)
					maxanim = num + 1;
			}
			else if (num >= 'A' && num <= 'J')
			{
				num = num - 'A';
				altanims[num] = tx2;
				if (num + 1 > altmax)
					altmax = num + 1;
			}
			else
				Sys_Error ("Bad animating texture %s", tx->name);
		}

		if (mod->bspversion == BSPVERSION_QUAKE64 && !Mod_CheckAnimTextureArrayQ64 (anims, maxanim))
			continue; // Just pretend this is a normal texture

#define ANIM_CYCLE 2
		// link them all together
		for (j = 0; j < maxanim; j++)
		{
			tx2 = anims[j];
			if (!tx2)
				Sys_Error ("Missing frame %i of %s", j, tx->name);
			tx2->anim_total = maxanim * ANIM_CYCLE;
			tx2->anim_min = j * ANIM_CYCLE;
			tx2->anim_max = (j + 1) * ANIM_CYCLE;
			tx2->anim_next = anims[(j + 1) % maxanim];
			if (altmax)
				tx2->alternate_anims = altanims[0];
		}
		for (j = 0; j < altmax; j++)
		{
			tx2 = altanims[j];
			if (!tx2)
				Sys_Error ("Missing frame %i of %s", j, tx->name);
			tx2->anim_total = altmax * ANIM_CYCLE;
			tx2->anim_min = j * ANIM_CYCLE;
			tx2->anim_max = (j + 1) * ANIM_CYCLE;
			tx2->anim_next = altanims[(j + 1) % altmax];
			if (maxanim)
				tx2->alternate_anims = anims[0];
		}
	}
}

/*
================
Mod_PolyForUnlitSurface -- johnfitz -- creates polys for unlightmapped surfaces (sky and water)

TODO: merge this into BuildSurfaceDisplayList?
================
*/
static void Mod_PolyForUnlitSurface (qmodel_t *mod, msurface_t *fa)
{
	const int numverts = fa->numedges;
	int		  i, lindex;
	float	 *vec;
	glpoly_t *poly;
	float	  texscale;

	if (fa->flags & (SURF_DRAWTURB | SURF_DRAWSKY))
		texscale = (1.0 / 128.0); // warp animation repeats every 128
	else
		texscale = (1.0 / 32.0); // to match r_notexture_mip

	// create the poly
	poly = (glpoly_t *)Mem_Alloc (sizeof (glpoly_t) + (numverts - 4) * VERTEXSIZE * sizeof (float));
	poly->next = NULL;
	fa->polys = poly;
	poly->numverts = numverts;
	for (i = 0; i < numverts; i++)
	{
		lindex = mod->surfedges[fa->firstedge + i];
		vec = (lindex > 0) ? mod->vertexes[mod->edges[lindex].v[0]].position : mod->vertexes[mod->edges[-lindex].v[1]].position;

		VectorCopy (vec, poly->verts[i]);
		poly->verts[i][3] = DotProduct (vec, fa->texinfo->vecs[0]) * texscale;
		poly->verts[i][4] = DotProduct (vec, fa->texinfo->vecs[1]) * texscale;
	}
}

/*
================
Mod_CalcSurfaceExtents
================
*/
static void Mod_CalcSurfaceExtentsTask (int surfnum, qmodel_t **mod_ptr)
{
	qmodel_t *mod = *mod_ptr;
	CalcSurfaceExtents (mod, &mod->surfaces[surfnum]);
}

/*
=================
Mod_LoadFaces
=================
*/
static void Mod_LoadFaces (qmodel_t *mod, byte *mod_base, lump_t *l, qboolean bsp2)
{
	int i, count;

	Mod_ParseFaces (mod, mod_base, l, bsp2);
	count = mod->numsurfaces;

	// SURF_DRAWTILED surfs get their polys built here instead of during parsing (Rust migration Phase 3 seam)
	for (i = 0; i < count; i++)
	{
		msurface_t *out = &mod->surfaces[i];
		if (out->flags & SURF_DRAWTILED)
			Mod_PolyForUnlitSurface (mod, out);
	}

	if (!no_rendering)
	{
		if (!Tasks_IsWorker () && (count > 1))
		{
			task_handle_t task = Task_AllocateAssignIndexedFuncAndSubmit ((task_indexed_func_t)Mod_CalcSurfaceExtentsTask, count, &mod, sizeof (qmodel_t *));
			Task_Join (task, TASK_TIMEOUT_INFINITE);
		}
		else
		{
			for (i = 0; i < count; i++)
				Mod_CalcSurfaceExtentsTask (i, &mod);
		}
	}
}

/*
=================
Mod_CheckWaterVis
=================
*/
static void Mod_CheckWaterVis (qmodel_t *mod)
{
	mleaf_t	   *leaf, *other;
	msurface_t *surf;
	int			i, j, k;
	int			numclusters = mod->submodels[0].visleafs;
	int			contentfound = 0;
	int			contenttransparent = 0;
	int			contenttype;
	unsigned	hascontents = 0;

	if (r_novis.value)
	{ // all can be
		mod->contentstransparent = (SURF_DRAWWATER | SURF_DRAWTELE | SURF_DRAWSLIME | SURF_DRAWLAVA);
		return;
	}

	// pvs is 1-based. leaf 0 sees all (the solid leaf).
	// leaf 0 has no pvs, and does not appear in other leafs either, so watch out for the biases.
	for (i = 0, leaf = mod->leafs + 1; i < numclusters - 1; i++, leaf++)
	{
		byte *vis;
		if (leaf->contents < 0) // err... wtf?
			hascontents = 0;
		if (leaf->contents == CONTENTS_WATER)
		{
			if ((contenttransparent & (SURF_DRAWWATER | SURF_DRAWTELE)) == (SURF_DRAWWATER | SURF_DRAWTELE))
				continue;
			// this check is somewhat risky, but we should be able to get away with it.
			for (contenttype = 0, j = 0; j < leaf->nummarksurfaces; j++)
			{
				surf = &mod->surfaces[leaf->firstmarksurface[j]];
				if (surf->flags & (SURF_DRAWWATER | SURF_DRAWTELE))
				{
					contenttype = surf->flags & (SURF_DRAWWATER | SURF_DRAWTELE);
					break;
				}
			}
			// its possible that this leaf has absolutely no surfaces in it, turb or otherwise.
			if (contenttype == 0)
				continue;
		}
		else if (leaf->contents == CONTENTS_SLIME)
			contenttype = SURF_DRAWSLIME;
		else if (leaf->contents == CONTENTS_LAVA)
			contenttype = SURF_DRAWLAVA;
		// fixme: tele
		else
			continue;
		if (contenttransparent & contenttype)
		{
		nextleaf:
			continue; // found one of this type already
		}
		contentfound |= contenttype;
		vis = Mod_DecompressVis (leaf->compressed_vis, mod);
		for (j = 0; j < (numclusters + 7) / 8; j++)
		{
			if (vis[j])
			{
				for (k = 0; k < 8; k++)
				{
					if (vis[j] & (1u << k))
					{
						other = &mod->leafs[(j << 3) + k + 1];
						if (leaf->contents != other->contents)
						{
							//							Con_Printf("%p:%i sees %p:%i\n", leaf, leaf->contents, other, other->contents);
							contenttransparent |= contenttype;
							goto nextleaf;
						}
					}
				}
			}
		}
	}

	if (!contenttransparent)
	{ // no water leaf saw a non-water leaf
		// but only warn when there's actually water somewhere there...
		if (hascontents & ((1 << -CONTENTS_WATER) | (1 << -CONTENTS_SLIME) | (1 << -CONTENTS_LAVA)))
			Con_DPrintf ("%s is not watervised\n", mod->name);
	}
	else
	{
		Con_DPrintf2 ("%s is vised for transparent", mod->name);
		if (contenttransparent & SURF_DRAWWATER)
			Con_DPrintf2 (" water");
		if (contenttransparent & SURF_DRAWTELE)
			Con_DPrintf2 (" tele");
		if (contenttransparent & SURF_DRAWLAVA)
			Con_DPrintf2 (" lava");
		if (contenttransparent & SURF_DRAWSLIME)
			Con_DPrintf2 (" slime");
		Con_DPrintf2 ("\n");
	}
	// any types that we didn't find are assumed to be transparent.
	// this allows submodels to work okay (eg: ad uses func_illusionary teleporters for some reason).
	mod->contentstransparent = contenttransparent | (~contentfound & (SURF_DRAWWATER | SURF_DRAWTELE | SURF_DRAWSLIME | SURF_DRAWLAVA));
}

/*
=================
Mod_BoundsFromClipNode -- johnfitz

update the model's clipmins and clipmaxs based on each node's plane.

This works because of the way brushes are expanded in hull generation.
Each brush will include all six axial planes, which bound that brush.
Therefore, the bounding box of the hull can be constructed entirely
from axial planes found in the clipnodes for that hull.
=================
*/
#if 0  /* disabled for now -- see in Mod_SetupSubmodels()  */
static void Mod_BoundsFromClipNode (qmodel_t *mod, int hull, int nodenum)
{
	mplane_t    *plane;
	mclipnode_t *node;

	if (nodenum < 0)
		return; // hit a leafnode

	node = &mod->clipnodes[nodenum];
	plane = mod->hulls[hull].planes + node->planenum;
	switch (plane->type)
	{

	case PLANE_X:
		if (plane->signbits == 1)
			mod->clipmins[0] = q_min (mod->clipmins[0], -plane->dist - mod->hulls[hull].clip_mins[0]);
		else
			mod->clipmaxs[0] = q_max (mod->clipmaxs[0], plane->dist - mod->hulls[hull].clip_maxs[0]);
		break;
	case PLANE_Y:
		if (plane->signbits == 2)
			mod->clipmins[1] = q_min (mod->clipmins[1], -plane->dist - mod->hulls[hull].clip_mins[1]);
		else
			mod->clipmaxs[1] = q_max (mod->clipmaxs[1], plane->dist - mod->hulls[hull].clip_maxs[1]);
		break;
	case PLANE_Z:
		if (plane->signbits == 4)
			mod->clipmins[2] = q_min (mod->clipmins[2], -plane->dist - mod->hulls[hull].clip_mins[2]);
		else
			mod->clipmaxs[2] = q_max (mod->clipmaxs[2], plane->dist - mod->hulls[hull].clip_maxs[2]);
		break;
	default:
		// skip nonaxial planes; don't need them
		break;
	}

	Mod_BoundsFromClipNode (mod, hull, node->children[0]);
	Mod_BoundsFromClipNode (mod, hull, node->children[1]);
}
#endif /* #if 0 */

/*
=================
Mod_LoadBrushModel
=================
*/
static void Mod_LoadBrushModel (qmodel_t *mod, const char *loadname, void *buffer)
{
	int		   i;
	int		   bsp2;
	dheader_t *header;

	mod->type = mod_brush;
	mod->is_worldmodel = (sv.modelname[0] && !q_strcasecmp (loadname, sv.name));

	header = (dheader_t *)buffer;

	mod->bspversion = LittleLong (header->version);

	switch (mod->bspversion)
	{
	case BSPVERSION:
		bsp2 = false;
		break;
#ifdef BSP29_VALVE
	case BSPVERSION_VALVE:
		bsp2 = false;
		break;
#endif
	case BSP2VERSION_2PSB:
		bsp2 = 1; // first iteration
		break;
	case BSP2VERSION_BSP2:
		bsp2 = 2; // sanitised revision
		break;
	case BSPVERSION_QUAKE64:
		bsp2 = false;
		break;
	default:
		Sys_Error ("Mod_LoadBrushModel: %s has unsupported version number (%i)", mod->name, mod->bspversion);
		break;
	}

	// swap all the lumps
	byte *mod_base = (byte *)header;

	for (i = 0; i < (int)sizeof (dheader_t) / 4; i++)
		((int *)header)[i] = LittleLong (((int *)header)[i]);

	// load into heap
	Mod_LoadVertexes (mod, mod_base, &header->lumps[LUMP_VERTEXES]);
	Mod_LoadEdges (mod, mod_base, &header->lumps[LUMP_EDGES], bsp2);
	Mod_LoadSurfedges (mod, mod_base, &header->lumps[LUMP_SURFEDGES]);
	Mod_LoadEntities (mod, mod_base, &header->lumps[LUMP_ENTITIES]);
	Mod_LoadTextures (mod, mod_base, &header->lumps[LUMP_TEXTURES]);
	Mod_LoadLighting (mod, mod_base, &header->lumps[LUMP_LIGHTING]);
	Mod_LoadPlanes (mod, mod_base, &header->lumps[LUMP_PLANES]);
	Mod_LoadTexinfo (mod, mod_base, &header->lumps[LUMP_TEXINFO]);
	Mod_LoadFaces (mod, mod_base, &header->lumps[LUMP_FACES], bsp2);
	Mod_LoadMarksurfaces (mod, mod_base, &header->lumps[LUMP_MARKSURFACES], bsp2);

	if (mod->bspversion == BSPVERSION && external_vis.value && mod->is_worldmodel)
	{
		FILE *fvis;
		Con_DPrintf ("trying to open external vis file\n");
		fvis = Mod_FindVisibilityExternal (mod, loadname);
		if (fvis)
		{
			mod->leafs = NULL;
			mod->numleafs = 0;
			Con_DPrintf ("found valid external .vis file for map\n");
			mod->visdata = Mod_LoadVisibilityExternal (fvis);
			if (mod->visdata)
			{
				Mod_LoadLeafsExternal (mod, fvis);
			}
			fclose (fvis);
			if (mod->visdata && mod->leafs && mod->numleafs)
			{
				goto visdone;
			}
			Con_DPrintf ("External VIS data failed, using standard vis.\n");
		}
	}

	Mod_LoadVisibility (mod, mod_base, &header->lumps[LUMP_VISIBILITY]);
	Mod_LoadLeafs (mod, mod_base, &header->lumps[LUMP_LEAFS], bsp2);
visdone:
	Mod_LoadNodes (mod, mod_base, &header->lumps[LUMP_NODES], bsp2);
	Mod_LoadClipnodes (mod, mod_base, &header->lumps[LUMP_CLIPNODES], bsp2);
	Mod_LoadSubmodels (mod, mod_base, &header->lumps[LUMP_MODELS]);

	Mod_MakeHull0 (mod);

	mod->numframes = 2; // regular and alternate animation

	Mod_CheckWaterVis (mod);
	Mod_SetupSubmodels (mod);
}

/*
=================
Mod_SanitizeMapDescription

Cleans up map descriptions:
- removes colors
- replaces newlines with spaces
- replaces consecutive spaces with single one
- removes leading/trailing spaces

Returns dst string length (excluding NUL terminator)
=================
*/
size_t Mod_SanitizeMapDescription (char *dst, size_t dstsize, const char *src)
{
	int srcpos, dstpos;

	if (!dstsize)
		return 0;

	for (srcpos = dstpos = 0; src[srcpos] && (size_t)dstpos + 1 < dstsize; srcpos++)
	{
		char c = src[srcpos] & 0x7f; // remove color
		if (c == '\n' || c == '\r')	 // replace newlines with spaces
			c = ' ';
		else if (c == '\\' && src[srcpos + 1] == 'n') // replace '\\' followed by 'n' with space
		{
			c = ' ';
			srcpos++;
		}
		// remove leading spaces, replace consecutive spaces with single one
		if (c != ' ' || (dstpos > 0 && dst[dstpos - 1] != c))
			dst[dstpos++] = c;
	}
	// remove trailing space, if any
	if (dstpos > 0 && dst[dstpos - 1] == ' ')
		--dstpos;

	dst[dstpos] = '\0';
	return dstpos;
}

/*
=================
Mod_LoadMapDescription

Parses the entity lump in the given map to find its worldspawn message
Writes at most maxchars bytes to dest, including the NUL terminator
Returns true if map is playable, false otherwise
=================
*/
qboolean Mod_LoadMapDescription (char *desc, size_t maxchars, const char *map)
{
	char		buf[4 * 1024];
	char		path[MAX_QPATH];
	const char *data;
	FILE	   *f;
	lump_t	   *entlump;
	dheader_t	header;
	int			i;
	qfileofs_t	filesize;
	qboolean	ret = false;

	if (!maxchars)
		return false;
	*desc = '\0';

	if ((size_t)q_snprintf (path, sizeof (path), "maps/%s.bsp", map) >= sizeof (path))
		return false;

	filesize = COM_FOpenFile (path, &f, NULL);
	if (filesize <= (qfileofs_t)sizeof (header))
	{
		if (filesize != -1)
			fclose (f);
		return false;
	}

	if (fread (&header, sizeof (header), 1, f) != 1)
	{
		fclose (f);
		return false;
	}

	header.version = LittleLong (header.version);

	switch (header.version)
	{
	case BSPVERSION:
	case BSP2VERSION_2PSB:
	case BSP2VERSION_BSP2:
	case BSPVERSION_QUAKE64:
		break;
	default:
		fclose (f);
		return false;
	}

	for (i = 1; i < (int)(sizeof (header) / sizeof (int)); i++)
		((int *)&header)[i] = LittleLong (((int *)&header)[i]);

	entlump = &header.lumps[LUMP_ENTITIES];
	if (entlump->filelen < 0 || entlump->filelen >= filesize || entlump->fileofs < 0 || entlump->fileofs + entlump->filelen > filesize)
	{
		fclose (f);
		return false;
	}

	// if the entity lump is large enough we assume the map is playable
	// and only try to parse the first entity (worldspawn) for the map title
	if (entlump->filelen >= (int)sizeof (buf))
	{
		ret = true;
		entlump->filelen = sizeof (buf) - 1;
	}

	Sys_fseek (f, (qfileofs_t)entlump->fileofs - sizeof (header), SEEK_CUR);
	i = fread (buf, 1, entlump->filelen, f);
	fclose (f);

	if (i <= 0)
		return false;
	buf[i] = '\0';

	for (i = 0, data = buf; data; i++)
	{
		data = COM_Parse (data);
		if (!data || com_token[0] != '{')
			return ret;

		while (1)
		{
			qboolean is_message;
			qboolean is_classname;

			// parse key
			data = COM_Parse (data);
			if (!data)
				return ret;
			if (com_token[0] == '}')
				break;

			is_message = i == 0 && !strcmp (com_token, "message");
			is_classname = i != 0 && !strcmp (com_token, "classname");

			// parse value
			data = COM_ParseEx (data, CPE_ALLOWTRUNC);
			if (!data)
				return ret;

			if (is_message)
			{
				Mod_SanitizeMapDescription (desc, maxchars, com_token);
				if (ret)
					return true;
			}
			else if (is_classname)
			{
#define CLASSNAME_STARTS_WITH(str) (!strncmp (com_token, str, strlen (str)))
#define CLASSNAME_IS(str)		   (!strcmp (com_token, str))

				if (CLASSNAME_STARTS_WITH ("info_player_") || CLASSNAME_STARTS_WITH ("ammo_") || CLASSNAME_STARTS_WITH ("weapon_") ||
					CLASSNAME_STARTS_WITH ("monster_") || CLASSNAME_IS ("trigger_changelevel"))
				{
					return true;
				}

#undef CLASSNAME_IS
#undef CLASSNAME_STARTS_WITH
			}
		}
	}

	return ret;
}

//=========================================================

/*
=================
Mod_FloodFillSkin

Fill background pixels so mipmapping doesn't have haloes - Ed
=================
*/

typedef struct
{
	short x, y;
} floodfill_t;

// must be a power of 2
#define FLOODFILL_FIFO_SIZE 0x1000
#define FLOODFILL_FIFO_MASK (FLOODFILL_FIFO_SIZE - 1)

#define FLOODFILL_STEP(off, dx, dy)                           \
	do                                                        \
	{                                                         \
		if (pos[off] == fillcolor)                            \
		{                                                     \
			pos[off] = 255;                                   \
			fifo[inpt].x = x + (dx), fifo[inpt].y = y + (dy); \
			inpt = (inpt + 1) & FLOODFILL_FIFO_MASK;          \
		}                                                     \
		else if (pos[off] != 255)                             \
			fdc = pos[off];                                   \
	} while (0)

static void Mod_FloodFillSkin (byte *skin, int skinwidth, int skinheight)
{
	byte fillcolor = *skin; // assume this is the pixel to fill
	int	 inpt = 0, outpt = 0;
	int	 filledcolor = -1;
	int	 i;

	TEMP_ALLOC (floodfill_t, fifo, FLOODFILL_FIFO_SIZE);

	if (filledcolor == -1)
	{
		filledcolor = 0;
		// attempt to find opaque black
		for (i = 0; i < 256; ++i)
			if (d_8to24table[i] == (255 << 0)) // alpha 1.0
			{
				filledcolor = i;
				break;
			}
	}

	// can't fill to filled color or to transparent color (used as visited marker)
	if ((fillcolor == filledcolor) || (fillcolor == 255))
	{
		// printf( "not filling skin from %d to %d\n", fillcolor, filledcolor );
		return;
	}

	fifo[inpt].x = 0, fifo[inpt].y = 0;
	inpt = (inpt + 1) & FLOODFILL_FIFO_MASK;

	while (outpt != inpt)
	{
		int	  x = fifo[outpt].x, y = fifo[outpt].y;
		int	  fdc = filledcolor;
		byte *pos = &skin[x + skinwidth * y];

		outpt = (outpt + 1) & FLOODFILL_FIFO_MASK;

		if (x > 0)
			FLOODFILL_STEP (-1, -1, 0);
		if (x < skinwidth - 1)
			FLOODFILL_STEP (1, 1, 0);
		if (y > 0)
			FLOODFILL_STEP (-skinwidth, 0, -1);
		if (y < skinheight - 1)
			FLOODFILL_STEP (skinwidth, 0, 1);
		skin[x + skinwidth * y] = fdc;
	}

	TEMP_FREE (fifo);
}

static gltexture_t *Mod_LoadFullbrightTexture (qmodel_t *mod, aliashdr_t *surf, const char *texname)
{
	// make a safe copy of texname to manage va() trensient usage
	char texname_copy[MAX_QPATH];
	q_strlcpy (texname_copy, texname, MAX_QPATH);

	// try to find matching glow texture :
	unsigned int fb_width = 0;
	unsigned int fb_height = 0;

	// unsupported format by default
	enum srcformat fb_fmt = SRC_INDEXED;

	void *fb_data = Image_LoadImage (texname_copy, (int *)&fb_width, (int *)&fb_height, &fb_fmt, mod->path_id);

	// fb texture found:
	if (fb_data)
	{
		if (fb_fmt != SRC_RGBA)
		{
			Con_Warning ("%s fbrights not RGBA, skipped.\n", texname_copy);
			Mem_Free (fb_data);
			return NULL;
		}

		// Normalize pixels for additive blending as in INDEXED: fullbright pixels have alpha > 0 => force alpha = 255 anyway.
		// otherwhise for transparent pixels (alpha = 0) => force alapha = 255 AND force color = black.
		for (size_t pixel_index = 0; pixel_index < (size_t)fb_width * (size_t)fb_height; pixel_index++)
		{
			uint32_t *rgba_pixel = (uint32_t *)fb_data + pixel_index;
			byte	 *rgba_component = (byte *)rgba_pixel;

			// not transparent pixels are the fulbright ones, otherwise they are the mask.
			if (rgba_component[3] == 0)
			{
				// transparent pixels / mask are forced to black.
				rgba_component[0] = 0;
				rgba_component[1] = 0;
				rgba_component[2] = 0;
			}

			// always force alpha = 255 for all pixels
			rgba_component[3] = 255;
		}

		gltexture_t *loaded_texture =
			TexMgr_LoadImage (mod, texname_copy, fb_width, fb_height, SRC_RGBA, (byte *)fb_data, texname_copy, 0, TEXPREF_ALPHA | TEXPREF_MIPMAP);
		Mem_Free (fb_data);

		return loaded_texture;
	}

	return NULL;
}

/*
===============
Mod_LoadSkinTask
===============
*/
typedef struct load_skin_task_args_s
{
	aliashdr_t *pheader;
	qmodel_t   *mod;
	byte	   *mod_base;
	byte	  **ppskintypes;
} load_skin_task_args_t;

static void Mod_LoadSkinTask (int i, load_skin_task_args_t *args)
{
	int			 j, k, size, groupskins;
	char		 name[MAX_QPATH];
	byte		*skin, *texels;
	byte		*pskintype = args->ppskintypes[i];
	byte		*pinskingroup;
	byte		*pinskinintervals;
	char		 fbr_mask_name[MAX_QPATH]; // johnfitz -- added for fullbright support
	src_offset_t offset;				   // johnfitz
	unsigned int texflags = TEXPREF_PAD;
	qmodel_t	*mod = args->mod;
	byte		*mod_base = args->mod_base;
	aliashdr_t	*pheader = args->pheader;

	size = pheader->skinwidth * pheader->skinheight;

	if (mod->flags & MF_HOLEY)
		texflags |= TEXPREF_ALPHA;

	if (ReadLongUnaligned (pskintype + offsetof (daliasskintype_t, type)) == ALIAS_SKIN_SINGLE)
	{
		skin = pskintype + sizeof (daliasskintype_t);
		Mod_FloodFillSkin (skin, pheader->skinwidth, pheader->skinheight);

		// save 8 bit texels for the player model to remap
		texels = (byte *)Mem_Alloc (size);
		pheader->texels[i] = texels;
		memcpy (texels, skin, size);

		pheader->gltextures[i][0] = NULL;
		pheader->fbtextures[i][0] = NULL;

		// try to load external textures first, if enabled.
		if (mdl_external_textures.value > 0.0f)
		{
			unsigned int   fwidth = 0;
			unsigned int   fheight = 0;
			void		  *data = NULL;
			// unsupported format by default
			enum srcformat fmt = SRC_INDEXED;

			if (!data)
				data = Image_LoadImage (va ("%s_%i", mod->name, i), (int *)&fwidth, (int *)&fheight, &fmt, mod->path_id);

			if (!data)
				data = Image_LoadImage (va ("progs/%s_%i", mod->name, i), (int *)&fwidth, (int *)&fheight, &fmt, mod->path_id);

			if (!data)
				data = Image_LoadImage (va ("textures/%s_%i", mod->name, i), (int *)&fwidth, (int *)&fheight, &fmt, mod->path_id);

			if (data)
			{
				if (fmt == SRC_RGBA)
				{
					pheader->gltextures[i][0] = TexMgr_LoadImage (
						mod, va ("%s_%i", mod->name, i), fwidth, fheight, fmt, data, va ("%s_%i", mod->name, i), 0, TEXPREF_ALPHA | TEXPREF_MIPMAP);

#define TRY_LOAD_FULLBRIGHTS(tex_name)                                                      \
	do                                                                                      \
	{                                                                                       \
		if (!pheader->fbtextures[i][0])                                                     \
			pheader->fbtextures[i][0] = Mod_LoadFullbrightTexture (mod, pheader, tex_name); \
	} while (0);

					// try to load the external fullbright texture, if any.
					assert (pheader->fbtextures[i][0] == NULL);

					TRY_LOAD_FULLBRIGHTS (va ("%s_%i_glow", mod->name, i));
					TRY_LOAD_FULLBRIGHTS (va ("%s_%i_luma", mod->name, i));
					TRY_LOAD_FULLBRIGHTS (va ("progs/%s_%i_glow", mod->name, i));
					TRY_LOAD_FULLBRIGHTS (va ("progs/%s_%i_luma", mod->name, i));
					TRY_LOAD_FULLBRIGHTS (va ("textures/%s_%i_glow", mod->name, i));
					TRY_LOAD_FULLBRIGHTS (va ("textures/%s_%i_luma", mod->name, i));
				}
				else
				{
					Con_Warning ("%s skin not RGBA, skipped.\n", va ("%s_%i", mod->name, i));
				}
			}

			Mem_Free (data);
		} // end if mdl_external_textures

		if (!pheader->gltextures[i][0])
		{
			// johnfitz -- rewritten
			q_snprintf (name, sizeof (name), "%s:frame%i", mod->name, i);
			offset = (src_offset_t)(skin) - (src_offset_t)mod_base;
			if (Mod_CheckFullbrights (skin, size))
			{
				pheader->gltextures[i][0] = TexMgr_LoadImage (
					mod, name, pheader->skinwidth, pheader->skinheight, SRC_INDEXED, skin, mod->name, offset, texflags | TEXPREF_MIPMAP | TEXPREF_NOBRIGHT);
				q_snprintf (fbr_mask_name, sizeof (fbr_mask_name), "%s:frame%i_glow", mod->name, i);
				pheader->fbtextures[i][0] = TexMgr_LoadImage (
					mod, fbr_mask_name, pheader->skinwidth, pheader->skinheight, SRC_INDEXED, skin, mod->name, offset,
					texflags | TEXPREF_MIPMAP | TEXPREF_FULLBRIGHT);
			}
			else
			{
				pheader->gltextures[i][0] =
					TexMgr_LoadImage (mod, name, pheader->skinwidth, pheader->skinheight, SRC_INDEXED, skin, mod->name, offset, texflags | TEXPREF_MIPMAP);
				pheader->fbtextures[i][0] = NULL;
			}
		}

		pheader->gltextures[i][3] = pheader->gltextures[i][2] = pheader->gltextures[i][1] = pheader->gltextures[i][0];
		pheader->fbtextures[i][3] = pheader->fbtextures[i][2] = pheader->fbtextures[i][1] = pheader->fbtextures[i][0];
		// johnfitz
	}
	else
	{
		// animating skin group.  yuck.
		pinskingroup = pskintype + sizeof (daliasskintype_t);
		groupskins = ReadLongUnaligned (pinskingroup + offsetof (daliasskingroup_t, numskins));
		pinskinintervals = pinskingroup + sizeof (daliasskingroup_t);
		skin = pinskinintervals + (groupskins * sizeof (daliasskininterval_t));

		for (j = 0; j < groupskins; j++)
		{
			Mod_FloodFillSkin (skin, pheader->skinwidth, pheader->skinheight);
			if (j == 0)
			{
				texels = (byte *)Mem_Alloc (size);
				pheader->texels[i] = texels;
				memcpy (texels, skin, size);
			}

			// johnfitz -- rewritten
			q_snprintf (name, sizeof (name), "%s:frame%i_%i", mod->name, i, j);
			offset = (src_offset_t)(skin) - (src_offset_t)mod_base; // johnfitz
			if (Mod_CheckFullbrights (skin, size))
			{
				pheader->gltextures[i][j & 3] = TexMgr_LoadImage (
					mod, name, pheader->skinwidth, pheader->skinheight, SRC_INDEXED, skin, mod->name, offset, texflags | TEXPREF_MIPMAP | TEXPREF_NOBRIGHT);
				q_snprintf (fbr_mask_name, sizeof (fbr_mask_name), "%s:frame%i_%i_glow", mod->name, i, j);
				pheader->fbtextures[i][j & 3] = TexMgr_LoadImage (
					mod, fbr_mask_name, pheader->skinwidth, pheader->skinheight, SRC_INDEXED, skin, mod->name, offset,
					texflags | TEXPREF_MIPMAP | TEXPREF_FULLBRIGHT);
			}
			else
			{
				pheader->gltextures[i][j & 3] =
					TexMgr_LoadImage (mod, name, pheader->skinwidth, pheader->skinheight, SRC_INDEXED, skin, mod->name, offset, texflags | TEXPREF_MIPMAP);
				pheader->fbtextures[i][j & 3] = NULL;
			}
			// johnfitz

			skin += size;
		}
		k = j;
		for (/**/; j < 4; j++)
			pheader->gltextures[i][j & 3] = pheader->gltextures[i][j - k];
	}
#undef TRY_LOAD_FULLBRIGHTS
}

/*
===============
Mod_LoadAllSkins
===============
*/
void *Mod_LoadAllSkins (aliashdr_t *pheader, qmodel_t *mod, byte *mod_base, int numskins, byte *pskintype)
{
	assert (pheader->poseverttype == PV_QUAKE1);

	if (numskins < 1 || numskins > MAX_SKINS)
		Sys_Error ("Mod_LoadAliasModel: Invalid # of skins: %d", numskins);

	TEMP_ALLOC (byte *, ppskintypes, numskins);
	int size = pheader->skinwidth * pheader->skinheight;
	for (int i = 0; i < numskins; i++)
	{
		ppskintypes[i] = pskintype;
		if (ReadLongUnaligned (pskintype + offsetof (daliasskintype_t, type)) == ALIAS_SKIN_SINGLE)
		{
			pskintype += sizeof (daliasskintype_t) + size;
		}
		else
		{
			// animating skin group.  yuck.
			byte *pinskingroup = pskintype + sizeof (daliasskintype_t);
			int	  groupskins = ReadLongUnaligned (pinskingroup + offsetof (daliasskingroup_t, numskins));
			byte *pinskinintervals = pinskingroup + sizeof (daliasskingroup_t);
			byte *skin = pinskinintervals + (groupskins * sizeof (daliasskininterval_t));
			pskintype = skin + (groupskins * size);
		}
	}

	load_skin_task_args_t args = {
		.pheader = pheader,
		.mod = mod,
		.mod_base = mod_base,
		.ppskintypes = ppskintypes,
	};
	if (!Tasks_IsWorker () && (numskins > 1))
	{
		task_handle_t task = Task_AllocateAssignIndexedFuncAndSubmit ((task_indexed_func_t)Mod_LoadSkinTask, numskins, &args, sizeof (args));
		Task_Join (task, TASK_TIMEOUT_INFINITE);
	}
	else
	{
		for (int i = 0; i < numskins; i++)
		{
			Mod_LoadSkinTask (i, &args);
		}
	}

	TEMP_FREE (ppskintypes);
	return (void *)pskintype;
}

//=========================================================================

/*
=================
Mod_LoadAliasModel
=================
*/
static void Mod_LoadAliasModel (qmodel_t *mod, void *buffer)
{
	aliashdr_t *pheader = Mod_ParseAliasModel (mod, buffer);

	//
	// build the draw lists
	//
	GL_MakeAliasModelDisplayLists (mod, pheader);

	//
	// move the complete, relocatable alias model to the cache
	//
	mod->extradata[PV_QUAKE1] = (byte *)pheader;
}

//=============================================================================

/*
===============
Mod_LoadMDXSkinTask
===============
*/
typedef struct load_skin_MDX_task_args_s
{
	qmodel_t   *mod;
	aliashdr_t *surf;
	skin_def_t *skins; // skins * framegroups table
} load_skin_MDX_task_args_t;

static void Mod_LoadMDXSkinTask (int i, load_skin_MDX_task_args_t *args)
{
	qmodel_t   *mod = args->mod;
	aliashdr_t *surf = args->surf;
	skin_def_t *skins = args->skins;

	assert (surf->poseverttype != PV_QUAKE1);

	const int skin_index = i / MAX_FRAMEGROUPS;
	const int f = i - skin_index * MAX_FRAMEGROUPS;

	const char *basic_texname = skins[skin_index].framegroups[f].c_str;

	if (!basic_texname)
		return;

#define TRY_LOAD_FULLBRIGHTS(tex_name)                                                         \
	do                                                                                         \
	{                                                                                          \
		if (!surf->fbtextures[skin_index][f])                                                  \
		{                                                                                      \
			surf->fbtextures[skin_index][f] = Mod_LoadFullbrightTexture (mod, surf, tex_name); \
		}                                                                                      \
	} while (0);

	unsigned int fwidth, fheight;

	void *data;
	char  texname[MAX_QPATH] = {0};
	q_snprintf (texname, sizeof (texname), "%s", basic_texname);

	enum srcformat fmt = SRC_RGBA;

	data = Image_LoadImage (texname, (int *)&fwidth, (int *)&fheight, &fmt, mod->path_id);

	if (!data)
	{
		q_snprintf (texname, sizeof (texname), "progs/%s", basic_texname);
		data = Image_LoadImage (texname, (int *)&fwidth, (int *)&fheight, &fmt, mod->path_id);
	}
	if (!data)
	{
		q_snprintf (texname, sizeof (texname), "textures/%s", basic_texname);
		data = Image_LoadImage (texname, (int *)&fwidth, (int *)&fheight, &fmt, mod->path_id);
	}

	if (data) // load external image
	{
		surf->gltextures[skin_index][f] =
			TexMgr_LoadImage (mod, texname, fwidth, fheight, fmt, data, texname, 0, TEXPREF_ALPHA | TEXPREF_NOBRIGHT | TEXPREF_MIPMAP);

		// no fullbrights by default.
		assert (surf->fbtextures[skin_index][f] == NULL);

		// initialize skinsizes:
		if (i == 0)
		{
			surf->skinwidth = surf->gltextures[0][0] ? surf->gltextures[0][0]->width : 1;
			surf->skinheight = surf->gltextures[0][0] ? surf->gltextures[0][0]->height : 1;
		}

		if (fmt == SRC_INDEXED)
		{
			if (f == 0)
			{
				size_t size = fwidth * fheight;
				byte  *texels = (byte *)Mem_Alloc (size);
				surf->texels[surf->numskins] = texels;
				memcpy (texels, data, size);
			}
			// 8bit base texture. use it for fullbrights.
			for (size_t j = 0; j < fwidth * fheight; j++)
			{
				if (((byte *)data)[j] > 223)
				{
					surf->fbtextures[skin_index][f] = TexMgr_LoadImage (
						mod, va ("%s_luma", basic_texname), fwidth, fheight, SRC_INDEXED, data, texname, 0,
						TEXPREF_ALPHA | TEXPREF_MIPMAP | TEXPREF_FULLBRIGHT);
					break;
				}
			}
		}
		else
		{
			// we found a 32bit base texture, try to fetch the fullbrights counterparts
			// Same as skins, try first the same location as the model, then 'progs/', then 'textures/' if not found.
			assert (surf->fbtextures[skin_index][f] == NULL);

			TRY_LOAD_FULLBRIGHTS (va ("%s_glow", basic_texname));
			TRY_LOAD_FULLBRIGHTS (va ("%s_luma", basic_texname));
			TRY_LOAD_FULLBRIGHTS (va ("progs/%s_glow", basic_texname));
			TRY_LOAD_FULLBRIGHTS (va ("progs/%s_luma", basic_texname));
			TRY_LOAD_FULLBRIGHTS (va ("textures/%s_glow", basic_texname));
			TRY_LOAD_FULLBRIGHTS (va ("textures/%s_luma", basic_texname));
		}

		Mem_Free (data);
	}

#undef TRY_LOAD_FULLBRIGHTS
}
/*
=====================
Mod_LoadMDXSkinsByIndex : generic method to load skins for MD3/MD5 exploring standard search paths,
parametrized by skin and framegroup index and skin_texture_pattern_fn.
returns the number of successfully loaded (i.e. up to numskins) skins for surf.
=====================
*/
typedef void (*skin_base_name_fn) (
	qmodel_t *mod, aliashdr_t *surf, all_surfaces_def_t *surf_defs, int surf_index, size_t numsurfaces, int skin_index, int framegroup_index,
	const char *basename, char output_name[MAX_QPATH]);

#define SKIN_PATTERN_FUNC_DEF(signature)                                                                                                          \
	static void signature (                                                                                                                       \
		qmodel_t *mod, aliashdr_t *surf, all_surfaces_def_t *surf_defs, int surf_index, size_t numsurfaces, int skin_index, int framegroup_index, \
		const char *basename, char output_name[MAX_QPATH])

static size_t Mod_LoadMDXSkinsByIndex (
	qmodel_t *mod, aliashdr_t *surf, all_surfaces_def_t *surf_defs, int surf_index, size_t numsurfaces, size_t numskins, const char *basename,
	skin_base_name_fn skin_pattern_func)
{
	// for each skin:
	size_t nb_loaded_skins = 0;

	TEMP_ALLOC_ZEROED (skin_def_t, skin_table, numskins);

	int effective_num_skins = 0;

	// Populate basic texture names:
	for (int skin_index = 0; skin_index < numskins; skin_index++)
	{
		for (int f = 0; f < countof (surf->gltextures[0]); f++)
		{
			// Generate basic skin name:
			skin_pattern_func (mod, surf, surf_defs, surf_index, numsurfaces, skin_index, f, basename, skin_table[skin_index].framegroups[f].c_str);

			if (strlen (skin_table[skin_index].framegroups[f].c_str) == 0)
			{
				// No more framegroup
				break;
			}
			skin_table[skin_index].numframegroups++;
		}
		if (skin_table[skin_index].numframegroups)
		{
			effective_num_skins++;
		}
		else
			break;
	}

	// load textures: (concurrently)
	load_skin_MDX_task_args_t args = {.mod = mod, .surf = surf, .skins = skin_table};

	if (!Tasks_IsWorker () && effective_num_skins > 1)
	{
		task_handle_t task =
			Task_AllocateAssignIndexedFuncAndSubmit ((task_indexed_func_t)Mod_LoadMDXSkinTask, effective_num_skins * MAX_FRAMEGROUPS, &args, sizeof (args));
		Task_Join (task, TASK_TIMEOUT_INFINITE);
	}
	else
	{
		for (int i = 0; i < effective_num_skins * MAX_FRAMEGROUPS; i++)
		{
			Mod_LoadMDXSkinTask (i, &args);
		}
	}

	// normalize skin definitions:
	// fills out animation array based on existing data (from Ironwail Mod_LoadMD3_PopulateAnimation)
	for (int skin_index = 0; skin_index < effective_num_skins; skin_index++)
	{
		int frame_count = 0;
		for (int framegroup_index = 0; framegroup_index < skin_table[skin_index].numframegroups; framegroup_index++)
		{
			if (surf->gltextures[skin_index][framegroup_index])
				frame_count++;
		}

		if (frame_count)
			nb_loaded_skins++;

		switch (frame_count)
		{
		case 1:
			surf->gltextures[skin_index][1] = surf->gltextures[skin_index][0];
			surf->fbtextures[skin_index][1] = surf->fbtextures[skin_index][0];
			surf->gltextures[skin_index][2] = surf->gltextures[skin_index][0];
			surf->fbtextures[skin_index][2] = surf->fbtextures[skin_index][0];
			surf->gltextures[skin_index][3] = surf->gltextures[skin_index][0];
			surf->fbtextures[skin_index][3] = surf->fbtextures[skin_index][0];
			break;
		case 2:
			surf->gltextures[skin_index][2] = surf->gltextures[skin_index][0];
			surf->fbtextures[skin_index][2] = surf->fbtextures[skin_index][0];
			surf->gltextures[skin_index][3] = surf->gltextures[skin_index][1];
			surf->fbtextures[skin_index][3] = surf->fbtextures[skin_index][1];
			break;
		case 3:
			surf->gltextures[skin_index][3] = surf->gltextures[skin_index][0];
			surf->fbtextures[skin_index][3] = surf->fbtextures[skin_index][0];
			break;
		default: // either full or impossible situation, either way nothing to do really
			break;
		}
	}

	TEMP_FREE (skin_table);

	return nb_loaded_skins;
}

/*
=====================
Mod_LoadMD5MeshModel
=====================
*/
SKIN_PATTERN_FUNC_DEF (MD5_Skin_Name)
{
	q_snprintf (output_name, MAX_QPATH, "%s_%02u_%02u", basename, skin_index, framegroup_index);
}

/*
=====================
Mod_LoadMD5SurfaceSkins -- the MD5 side of Mod_LoadMDXSkinsByIndex, wrapped so the
skin_base_name_fn table stays private to gl_model.c (Rust migration Phase 3 seam)
=====================
*/
size_t Mod_LoadMD5SurfaceSkins (qmodel_t *mod, aliashdr_t *surf, int surf_index, size_t numsurfaces, const char *shader_name)
{
	return Mod_LoadMDXSkinsByIndex (mod, surf, NULL, surf_index, numsurfaces, MAX_SKINS, shader_name, MD5_Skin_Name);
}

/*
=====================
Mod_AppendMD3SkinFile:
if expected_skin_index >= 0 assume file_contents is for skin index expected_skin_index,
else we assume it is for a .skin file containing all surfaces definitions for all skins.
=====================
*/
static void Mod_AppendMD3SkinFile (char *file_contents, int expected_skin_index, all_surfaces_def_t *surf_defs)
{
	// Read .skin file contents, appending to surf_defs
	//  a line is made of:
	//  surface_name,skin0, skin1,  skin2, ...

	// split by lines:
	size_t nb_lines = 0;
	char **lines = q_strsplit (file_contents, "\n\r", &nb_lines);

	// parse line by line
	for (size_t line_index = 0; line_index < nb_lines; line_index++)
	{
		// This line must be split by its ',' and stripping whitespaces of the resulting sub-tokens
		size_t nb_fields = 0;
		char **fields = q_strsplit (q_strtrim (lines[line_index]), ",", &nb_fields);

		// there should be at least a "surface name, skin"
		// butno more than "surface name, skin0, skin1, ..." upto MAX_SKINS
		if ((nb_fields < 2) || (nb_fields > MAX_SKINS + 1))
		{
			Mem_Free (fields);
			continue;
		}

		// token 0 is the surface name
		char *surface_name = q_strtrim (fields[0]);

		// find the surface index in surf_defs matching surface_name:
		int found_surface_index = -1;

		for (size_t surface_index = 0; surface_index < surf_defs->numsurfaces; surface_index++)
		{
			if (!strcmp (surf_defs->surfaces[surface_index].surfname.c_str, surface_name))
			{
				found_surface_index = surface_index;
			}
		}

		// if not found, this is a new surface, add it:
		if (found_surface_index < 0)
		{
			if (surf_defs->numsurfaces >= MAX_SURFACES)
			{
				// too many surfaces, skip
				Mem_Free (fields);
				continue;
			}

			q_strlcpy (surf_defs->surfaces[surf_defs->numsurfaces].surfname.c_str, surface_name, MAX_QPATH);
			found_surface_index = surf_defs->numsurfaces;
			surf_defs->numsurfaces++;
		}

		//  Different line formats:
		// 1) New format    : if expected_skin_index < 0 a line lists all skins for a given framegroup in one go.
		// 2) Legacy format : else if expected_skin_index >= 0 we only expect 1 skin name per-line, whose index
		// is expected_skin_index.
		for (int field_index = 1; field_index < nb_fields; field_index++)
		{
			int skin_index = ((expected_skin_index < 0) ? field_index - 1 : expected_skin_index);

			char *skin_name = q_strtrim (fields[field_index]);

			int current_nb_framegroups = surf_defs->surfaces[found_surface_index].skins[skin_index].numframegroups;

			// we are incrementing the frame group count for each skin:
			if (current_nb_framegroups < MAX_FRAMEGROUPS)
			{
				q_strlcpy (surf_defs->surfaces[found_surface_index].skins[skin_index].framegroups[current_nb_framegroups].c_str, skin_name, MAX_QPATH);
				surf_defs->surfaces[found_surface_index].skins[skin_index].numframegroups++;
			}
		}
		// update skin counts:
		int current_num_skins = surf_defs->surfaces[found_surface_index].numskins;

		current_num_skins = q_max (current_num_skins, ((expected_skin_index < 0) ? nb_fields - 1 : expected_skin_index + 1));

		surf_defs->surfaces[found_surface_index].numskins = current_num_skins;

		Mem_Free (fields);
	} // for each line

	Mem_Free (lines);
}

/*
=====================
Mod_LoadMD3SkinDefinitions
=====================
*/
void Mod_LoadMD3SkinDefinitions (qmodel_t *mod, all_surfaces_def_t *surf_defs)
{
	memset ((void *)surf_defs, 0x0, sizeof (*surf_defs));

	char basename[MAX_QPATH];
	COM_StripExtension (mod->name, basename, sizeof (basename));

	bool loading_complete = false;

	// file_number = -1 is special case = no numbering suffix
	// else the numbering is based in skin indices
	for (int file_number = -1; !loading_complete && file_number < MAX_SKINS; file_number++)
	{
		// version with .md3 suffix has priority over the non-suffix one.
		for (int has_md3_suffix = 1; has_md3_suffix >= 0; has_md3_suffix--)
		{
			char skinfile_name[MAX_QPATH];
			// build a .skin file name:
			q_snprintf (
				skinfile_name, MAX_QPATH, "%s%s%s%s", basename, (has_md3_suffix ? ".md3" : ""), ((file_number >= 0) ? va ("_%d", file_number) : ""), ".skin");

			unsigned int opened_file_path_id = 0;
			// Load the file as binary blob, to be parsed in Mod_AppendMD3SkinFile.
			char		*file_contents = (char *)COM_LoadFile (skinfile_name, &opened_file_path_id);

			if (file_contents)
			{
				if (opened_file_path_id >= mod->path_id)
				{
					// Read contents:
					Mod_AppendMD3SkinFile (file_contents, file_number, surf_defs);
					Mem_Free (file_contents);

					// if file_number = -1 i.e. no numbering suffix variant was loaded successfully,
					// we assumes it contains all surface definitions, so skip the numbered ones entirely.
					if (file_number == -1)
						loading_complete = true;

					// if a .md3 suffix variant was found, skip the non-md3 one.
					if (has_md3_suffix)
						break;
				}
				else
				{
					Con_DPrintf ("MD3 skfile: ignored %s from a gamedir with lower priority\n", skinfile_name);
					Mem_Free (file_contents);

					// no more skins, stop searching for more files
					if ((file_number >= 0) && (has_md3_suffix == 0))
					{
						loading_complete = true;
						break;
					}
				}
			}
			else
			{ // no more skins, stop searching for more files
				if ((file_number >= 0) && (has_md3_suffix == 0))
				{
					loading_complete = true;
					break;
				}
			}
		}
	}

	// List contents if developer >= 1
	if (developer.value >= 1)
	{
		if (!surf_defs->numsurfaces)
		{
			Con_DPrintf ("MD3 skfile: %s, no surfaces found.\n", mod->name);
		}
		else
		{
			for (size_t surf_index = 0; surf_index < surf_defs->numsurfaces; surf_index++)
				for (size_t skin_index = 0; skin_index < surf_defs->surfaces[surf_index].numskins; skin_index++)
					for (size_t framegrp_index = 0; framegrp_index < surf_defs->surfaces[surf_index].skins[skin_index].numframegroups; framegrp_index++)
						Con_DPrintf (
							"MD3 skfile:%s|surf %s(%d)|%d-%d|%s\n", mod->name, surf_defs->surfaces[surf_index].surfname.c_str, (int)surf_index, (int)skin_index,
							(int)framegrp_index, surf_defs->surfaces[surf_index].skins[skin_index].framegroups[framegrp_index].c_str);
		}
	}
}

//
SKIN_PATTERN_FUNC_DEF (MD3_Skinfile)
{
	// BEWARE : surf_index here designates the mesh index in the MD3 file
	// which has nothing to do with the index of the surface in surf_defs.
	// So we have to search for the right surface by name using 'basename' which is in this case
	// is the MD3 surface name to look for.
	int skinfile_surf_index = -1;

	for (size_t i = 0; i < surf_defs->numsurfaces; i++)
	{
		if (!strcmp (basename, surf_defs->surfaces[i].surfname.c_str))
		{
			skinfile_surf_index = i;
			break;
		}
	}

	if (skinfile_surf_index == -1)
		return;

	if (skin_index >= surf_defs->surfaces[skinfile_surf_index].numskins)
		return;

	if (framegroup_index >= surf_defs->surfaces[skinfile_surf_index].skins[skin_index].numframegroups)
		return;

	// strip extension:
	char skin_name[MAX_QPATH];
	q_strlcpy (skin_name, surf_defs->surfaces[skinfile_surf_index].skins[skin_index].framegroups[framegroup_index].c_str, MAX_QPATH);
	COM_StripExtension (skin_name, output_name, MAX_QPATH);
}
// skin name : surfacename.ext (1 skin, 1 framgroup)
SKIN_PATTERN_FUNC_DEF (MD3_Surf_Name_Legacy_Single)
{
	q_snprintf (output_name, MAX_QPATH, "%s", basename);
}

// skin name : surfacename_X.ext (0..X-1 skin, 1 framgroup)
SKIN_PATTERN_FUNC_DEF (MD3_Surf_Name_Legacy_One_Framegroup)
{
	q_snprintf (output_name, MAX_QPATH, "%s_%d", basename, skin_index);
}

// skin name : surfacename_X_Y.ext (0..X-1 skin, 0..Y-1 framgroup)
SKIN_PATTERN_FUNC_DEF (MD3_Surf_Name_Legacy)
{
	q_snprintf (output_name, MAX_QPATH, "%s_%d_%d", basename, skin_index, framegroup_index);
}

// skin name : model_name.md3_S_X_Y.ext (0..S-1 surfaces, 0..X-1 skin, 0..Y-1 framgroup) using Legacy conventions (%d), using the model name as prefix
SKIN_PATTERN_FUNC_DEF (MD3_Model_Name_Legacy_Full)
{
	char newname[MAX_QPATH];
	COM_StripExtension (basename, newname, sizeof (newname));
	COM_AddExtension (newname, ".md3", sizeof (newname));
	q_snprintf (output_name, MAX_QPATH, "%s_%d_%d_%d", newname, surf_index, skin_index, framegroup_index);
}

/*
=====================
Mod_LoadMD3SurfaceSkins
=====================
*/
int Mod_LoadMD3SurfaceSkins (
	qmodel_t *mod, aliashdr_t *surf, all_surfaces_def_t *surfaces_def, const char *surface_name, int surface_index, size_t numsurfs, size_t numskins)
{
	// 1. Try to load first from existing surfaces_def built from .skin files:
	int surf_numskins = (int)Mod_LoadMDXSkinsByIndex (mod, surf, surfaces_def, surface_index, numsurfs, numskins, surface_name, MD3_Skinfile);

	// 2. Try to load the "legacy" MD3 namings from the existing Quake 3 ecosystem :
	// skin name : surfacename_X_Y.ext (0..X-1 skins, 0..Y-1 framgroups)
	if (!surf_numskins)
		surf_numskins = (int)Mod_LoadMDXSkinsByIndex (mod, surf, NULL, surface_index, numsurfs, numskins, surface_name, MD3_Surf_Name_Legacy);

	// skin name : surfacename_X.ext (0..X-1 skins, 1 framgroup)
	if (!surf_numskins)
		surf_numskins = (int)Mod_LoadMDXSkinsByIndex (mod, surf, NULL, surface_index, numsurfs, numskins, surface_name, MD3_Surf_Name_Legacy_One_Framegroup);

	//  skin name : surfacename.ext (1 skin, 1 framgroup)
	if (!surf_numskins)
		surf_numskins = (int)Mod_LoadMDXSkinsByIndex (mod, surf, NULL, surface_index, numsurfs, 1, surface_name, MD3_Surf_Name_Legacy_Single);

	// 3. Model-based names:
	//   skin name : model_name.md3_S_X_Y.ext (0..S-1 surfaces, 0..X-1 skin, 0..Y-1 framgroup) using Legacy conventions (%d), using the model name as prefix
	if (!surf_numskins)
		surf_numskins = (int)Mod_LoadMDXSkinsByIndex (mod, surf, NULL, surface_index, numsurfs, numskins, mod->name, MD3_Model_Name_Legacy_Full);

	// 4. MD5-like naming conventions:
	// skin name : surfacename_X_Y.ext (0..X-1 skins, 0..Y-1 framgroups) with %s_%02u_%02u pattern
	if (!surf_numskins)
		surf_numskins = (int)Mod_LoadMDXSkinsByIndex (mod, surf, NULL, surface_index, numsurfs, numskins, surface_name, MD5_Skin_Name);

	return surf_numskins;
}

#undef SKIN_PATTERN_FUNC_DEF

//=============================================================================

/*
================
Mod_Print
================
*/
void Mod_Print (void)
{
	int		  i;
	qmodel_t *mod;

	Con_SafePrintf ("Cached models:\n"); // johnfitz -- safeprint instead of print
	for (i = 0, mod = mod_known; i < mod_numknown; i++, mod++)
	{
		Con_SafePrintf (
			"MDL:%s || MD5:%s || MD3:%s  -  %s\n", (mod->extradata[PV_QUAKE1]) ? "YES" : " no", (mod->extradata[PV_MD5]) ? "YES" : " no",
			(mod->extradata[PV_QUAKE3]) ? "YES" : " no", mod->name); // johnfitz -- safeprint instead of print
	}
	Con_Printf ("%i models\n", mod_numknown); // johnfitz -- print the total too
}
