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

// r_sprite_glue.c -- the C that stays with r_sprite.c under -Duse_rust_render
// (Rust migration Phase 8 M7). Mod_Extradata can reach Host_Error through
// Mod_LoadModel, so the entry points resolve the sprite header here (ADR-009:
// no longjmp through Rust frames) and hand it to quake-capi's r_sprite.rs.

#include "quakedef.h"

void RSprite_DrawSpriteModel (cb_context_t *cbx, entity_t *e, msprite_t *psprite);
void RSprite_DrawSpriteModel_ShowTris (cb_context_t *cbx, entity_t *e, msprite_t *psprite);

void R_DrawSpriteModel (cb_context_t *cbx, entity_t *e)
{
	RSprite_DrawSpriteModel (cbx, e, (msprite_t *)Mod_Extradata (e->model));
}

void R_DrawSpriteModel_ShowTris (cb_context_t *cbx, entity_t *e)
{
	RSprite_DrawSpriteModel_ShowTris (cbx, e, (msprite_t *)Mod_Extradata (e->model));
}
