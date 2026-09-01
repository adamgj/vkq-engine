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
// cl_tent_glue.c -- the C frame around the Rust client temp-entity port.
//
// Compiled instead of cl_tent.c under -Duse_rust_host (Rust migration Phase 7
// M7, T7.2), mirroring sv_user_glue.c:
//
//  1. Own the C-visible objects cl_tent.c defined: num_temp_entities,
//     cl_temp_entities[] and cl_beams[] (cl_tent.c:26-28). cl_main.c,
//     cl_demo.c and host_cmd.c all still read them, so the storage stays here
//     and Rust reaches it through externs. The seven cl_sfx_* handles were
//     file-static and move to Rust.
//  2. Guard the one thing cl_tent.c reached that can Host_Error: Mod_ForName
//     (gl_model.c:531) behind the four TE_LIGHTNING*/TE_BEAM cases.
//  3. Re-raise, from a pure C frame, what that guard caught. CL_ParseTEnt is
//     the only cl_tent.c entry point that can raise.
//  4. Reach into entity_t for Rust, which sees it as an opaque 456-byte blob
//     (ADR-011; entity_t carries PSET_SCRIPT-conditional members and an
//     entity_state_t, so it is deliberately not mirrored) -- same shape as
//     world_glue.c's World_Glue_EntClipInfo.
//
// Nothing here guards the Sys_Error at cl_tent.c:314: Sys_Error terminates
// rather than longjmping, so ClTent_Glue_BadTEntType is a plain noreturn shim
// and not a Host_Guard site. Likewise S_PrecacheSound/S_StartSound,
// CL_AllocDlight, CL_TraceLine, the PScript_*/R_* particle entry points, va,
// COM_Rand/COM_SeedRand and the MSG_Read* family contain no Host_Error or
// Host_EndGame on any reachable path, so Rust calls them directly.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

/* ---------------------------------------------------------------------------
 * C-visible objects (cl_tent.c:26-28).
 */

int		 num_temp_entities;
entity_t cl_temp_entities[MAX_TEMP_ENTITIES];
beam_t	 cl_beams[MAX_BEAMS];

/* ---------------------------------------------------------------------------
 * Guarded callback (ADR-009 rule 3).
 */

typedef struct
{
	const char *name;
	qmodel_t  **out;
} cltent_modforname_arg_t;

/* cl_tent.c:236, :240, :244, :249 -- Mod_ForName (..., true), whose crash=true
   path Host_Errors from gl_model.c:531 when the model is missing. */
static void ClTent_InvokeModForName (void *p)
{
	cltent_modforname_arg_t *a = (cltent_modforname_arg_t *)p;
	*a->out = Mod_ForName (a->name, true);
}

int ClTent_Glue_ModForName (const char *name, qmodel_t **out)
{
	cltent_modforname_arg_t arg;
	arg.name = name;
	arg.out = out;
	*out = NULL;
	return Host_Guard (ClTent_InvokeModForName, &arg);
}

/* ---------------------------------------------------------------------------
 * Plain shims. None of these can raise.
 */

/* cl_tent.c:314. Sys_Error terminates; this is not a guard site. */
FUNC_NORETURN void ClTent_Glue_BadTEntType (void)
{
	Sys_Error ("CL_ParseTEnt: bad type");
}

/* cl_tent.c:275 -- keeps the va() format string on the C side so the ADR-005
   integer formatting is not re-implemented. */
const char *ClTent_Glue_Explosion2Name (int colorStart, int colorLength)
{
	return va ("TE_EXPLOSION2_%i_%i", colorStart, colorLength);
}

/* cl_tent.c:332 and :337 -- the two entity_t writes CL_NewTempEntity makes that
   Rust cannot express against an opaque blob. They stay two shims because C
   does them either side of the three counter updates. */
void ClTent_Glue_ClearTempEntity (entity_t *ent)
{
	memset (ent, 0, sizeof (*ent));
}

void ClTent_Glue_SetTempEntityNetstate (entity_t *ent)
{
	ent->netstate = nullentitystate;
}

/* cl_tent.c:404-408 -- the five entity_t writes CL_UpdateTEnts makes per
   lightning segment, in source order. */
void ClTent_Glue_SetBeamEntity (entity_t *ent, const float *org, qmodel_t *model, float pitch, float yaw, float roll)
{
	VectorCopy (org, ent->origin);
	ent->model = model;
	ent->angles[0] = pitch;
	ent->angles[1] = yaw;
	ent->angles[2] = roll;
}

/* cl_tent.c:370 -- cl.entities[cl.viewentity].origin. */
void ClTent_Glue_GetEntityOrigin (const entity_t *ent, float *out)
{
	VectorCopy (ent->origin, out);
}

/* ---------------------------------------------------------------------------
 * Re-raising public entry point (ADR-009).
 */

/* cl_tent.c:133 */
void CL_ParseTEnt (void)
{
	int r = quake_rs_cl_parse_tent ();
	Host_Reraise (r);
}

#endif /* USE_RUST_HOST */
