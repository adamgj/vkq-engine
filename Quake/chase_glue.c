/*
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
// chase_glue.c -- the C frame around the Rust chase-camera port.
//
// Compiled instead of chase.c under -Duse_rust_host (Rust migration Phase 7
// M7, T7.2a). Three jobs, mirroring view_glue.c:
//
//  1. Own the four cvars chase.c defined (chase.c:26-29). view.c, cl_input.c,
//     gl_rmain.c and menu.c reach chase_active by plain name, so the storage
//     stays in C and Rust reaches it through externs.
//  2. Guard Chase_Init's four Cvar_RegisterVariable calls, which are
//     Host_Reraise wrappers under -Duse_rust_cvar (ADR-009 rule 3), and
//     re-raise from here what the guard caught.
//  3. Forward TraceLine, Chase_UpdateForClient and Chase_UpdateForDrawing to
//     their Rust cores. None of those can raise: they reach only
//     SV_RecursiveHullCheck (whose Sys_Error aborts rather than jumping) and
//     mathlib.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

/* ---------------------------------------------------------------------------
 * C-visible objects (chase.c:26-29).
 */

cvar_t chase_back = {"chase_back", "100", CVAR_NONE};
cvar_t chase_up = {"chase_up", "16", CVAR_NONE};
cvar_t chase_right = {"chase_right", "0", CVAR_NONE};
cvar_t chase_active = {"chase_active", "0", CVAR_NONE};

/* ---------------------------------------------------------------------------
 * Guarded callback (ADR-009 rule 3).
 */

/* chase.c:38-41 -- one Cvar_RegisterVariable. */
static void Chase_InvokeRegisterVariable (void *p)
{
	Cvar_RegisterVariable ((cvar_t *)p);
}

int Chase_Glue_RegisterVariable (cvar_t *var)
{
	return Host_Guard (Chase_InvokeRegisterVariable, var);
}

/* ---------------------------------------------------------------------------
 * Re-raising public entry point (ADR-009).
 */

/* chase.c:36 */
void Chase_Init (void)
{
	int r = quake_rs_chase_init ();
	Host_Reraise (r);
}

/* ---------------------------------------------------------------------------
 * Non-raising public entry points: plain forwards to the Rust cores.
 */

/* chase.c:51 */
void TraceLine (vec3_t start, vec3_t end, vec3_t impact)
{
	quake_rs_trace_line (start, end, impact);
}

/* chase.c:66 */
void Chase_UpdateForClient (void)
{
	quake_rs_chase_update_for_client ();
}

/* chase.c:84 */
void Chase_UpdateForDrawing (void)
{
	quake_rs_chase_update_for_drawing ();
}

#endif /* USE_RUST_HOST */
