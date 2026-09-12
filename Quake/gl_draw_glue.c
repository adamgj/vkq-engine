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

// gl_draw_glue.c -- the C that stays with gl_draw.c under -Duse_rust_render
// (Rust migration Phase 8 M7): the scr_conalpha cvar definition, which the
// console/screen C reads by name. Everything else, including Draw_Init (which
// registers it), lives in rust/quake-capi/src/gl_draw.rs.

#include "quakedef.h"

cvar_t scr_conalpha = {"scr_conalpha", "0.5", CVAR_ARCHIVE}; // johnfitz
