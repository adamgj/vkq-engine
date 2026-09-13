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

// gl_rlight_glue.c -- the C that stays with gl_rlight.c under -Duse_rust_render
// (Rust migration Phase 8 M7). gl_rlight.c owns no cvar definitions other than
// r_entdlightscale, which gl_rmisc_glue.c registers; every function now lives in
// rust/quake-capi/src/gl_rlight.rs.

#include "quakedef.h"

cvar_t r_entdlightscale = {"r_entdlightscale", "1", CVAR_NONE};
