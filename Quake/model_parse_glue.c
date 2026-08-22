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
// model_parse_glue.c -- Host_Error trampolines for the Rust brush/BSP loaders
// (Rust migration Phase 3). Compiled only with -Duse_rust_formats.
//
// PLAN.md 4.3: a Host_Error longjmp must never unwind a Rust frame, so the
// Rust exports return a status plus the message C would have raised, and
// these pure-C frames re-raise it. sv.modelname is passed in for the same
// reason -- the Rust side never reads a server global.

#include "quakedef.h"
#include "model_parse.h"
// quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t, so it
// needs steam.h first (host.c does the same)
#include "steam.h"
#include "quake_rs.h"

#define MOD_ERR_SIZE 256

void Mod_LoadLeafs (qmodel_t *mod, byte *mod_base, lump_t *l, int bsp2)
{
	char err[MOD_ERR_SIZE];

	if (!quake_rs_mod_load_leafs (mod, mod_base, l, bsp2, err))
		Host_Error ("%s", err);
}

void Mod_LoadClipnodes (qmodel_t *mod, byte *mod_base, lump_t *l, qboolean bsp2)
{
	char err[MOD_ERR_SIZE];

	if (!quake_rs_mod_load_clipnodes (mod, mod_base, l, bsp2, err))
		Host_Error ("%s", err);
}

void Mod_LoadMarksurfaces (qmodel_t *mod, byte *mod_base, lump_t *l, int bsp2)
{
	char err[MOD_ERR_SIZE];

	if (!quake_rs_mod_load_marksurfaces (mod, mod_base, l, bsp2, err))
		Host_Error ("%s", err);
}

void Mod_SetupSubmodels (qmodel_t *mod)
{
	char err[MOD_ERR_SIZE];

	if (!quake_rs_mod_setup_submodels (mod, sv.modelname, err))
		Host_Error ("%s", err);
}

void Mod_LoadLeafsExternal (qmodel_t *mod, FILE *f)
{
	char err[MOD_ERR_SIZE];

	if (!quake_rs_mod_load_leafs_external (mod, f, err))
		Host_Error ("%s", err);
}
