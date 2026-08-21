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

#ifndef MODEL_PARSE_H
#define MODEL_PARSE_H

// model_parse.h -- model/BSP parsing seam between gl_model.c and
// model_parse.c (Rust migration Phase 3). Requires quakedef.h.

// PVS decompression (also used by gl_model.c Mod_CheckWaterVis)
byte *Mod_DecompressVis (byte *in, qmodel_t *model);

// brush model lump parsing
void Mod_ParseTextures (qmodel_t *mod, byte *mod_base, lump_t *l, wad_t *wads);
void Mod_LoadLighting (qmodel_t *mod, byte *mod_base, lump_t *l);
void Mod_LoadVisibility (qmodel_t *mod, byte *mod_base, lump_t *l);
void Mod_LoadEntities (qmodel_t *mod, byte *mod_base, lump_t *l);
void Mod_LoadVertexes (qmodel_t *mod, byte *mod_base, lump_t *l);
void Mod_LoadEdges (qmodel_t *mod, byte *mod_base, lump_t *l, int bsp2);
void Mod_LoadTexinfo (qmodel_t *mod, byte *mod_base, lump_t *l);
void CalcSurfaceExtents (qmodel_t *mod, msurface_t *s);
void Mod_ParseFaces (qmodel_t *mod, byte *mod_base, lump_t *l, qboolean bsp2);
void Mod_LoadNodes (qmodel_t *mod, byte *mod_base, lump_t *l, int bsp2);
void Mod_LoadLeafs (qmodel_t *mod, byte *mod_base, lump_t *l, int bsp2);
void Mod_LoadClipnodes (qmodel_t *mod, byte *mod_base, lump_t *l, qboolean bsp2);
void Mod_MakeHull0 (qmodel_t *mod);
void Mod_LoadMarksurfaces (qmodel_t *mod, byte *mod_base, lump_t *l, int bsp2);
void Mod_LoadSurfedges (qmodel_t *mod, byte *mod_base, lump_t *l);
void Mod_LoadPlanes (qmodel_t *mod, byte *mod_base, lump_t *l);
void Mod_LoadSubmodels (qmodel_t *mod, byte *mod_base, lump_t *l);
void Mod_SetupSubmodels (qmodel_t *mod);

// external .vis support
FILE *Mod_FindVisibilityExternal (qmodel_t *mod, const char *loadname);
byte *Mod_LoadVisibilityExternal (FILE *f);
void  Mod_LoadLeafsExternal (qmodel_t *mod, FILE *f);

// alias model parsing
aliashdr_t *Mod_ParseAliasModel (qmodel_t *mod, void *buffer);
void		Mod_CalcAliasBounds (qmodel_t *mod, aliashdr_t *a, int numvertexes, byte *vertexes);

// sprite model parsing
void Mod_LoadSpriteModel (qmodel_t *mod, void *buffer);

#endif /* MODEL_PARSE_H */
