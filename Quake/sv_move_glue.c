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
// sv_move_glue.c -- the C frame around the Rust monster-movement port.
//
// Compiled instead of sv_move.c under -Duse_rust_host (Rust migration Phase 7
// M4). Every sv_move.c entry point that can reach Host_Error, directly or
// transitively (through SV_Move/SV_LinkEdict/each other), is a thin wrapper
// over a quake_rs_* status core; Host_Reraise is called only from here
// (ADR-009). None of this file's five wrappers needs its own Host_Guard call
// site: every raising path inside them already funnels through world.c's
// SV_Move / SV_LinkEdict cores or world_glue.c's World_Glue_AssertFailed, all
// of which quake-capi's sv_move module calls directly as ordinary same-crate
// Rust functions and threads the resulting status back up to here.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

/* sv_move.c:39 */
qboolean SV_CheckBottom (edict_t *ent)
{
	qboolean out = false;
	int		 r = quake_rs_sv_check_bottom (ent, &out);
	Host_Reraise (r);
	return out;
}

/* sv_move.c:111 */
qboolean SV_movestep (edict_t *ent, vec3_t move, qboolean relink)
{
	qboolean out = false;
	int		 r = quake_rs_sv_movestep (ent, move, relink, &out);
	Host_Reraise (r);
	return out;
}

/* sv_move.c:236 */
qboolean SV_StepDirection (edict_t *ent, float yaw, float dist)
{
	qboolean out = false;
	int		 r = quake_rs_sv_step_direction (ent, yaw, dist, &out);
	Host_Reraise (r);
	return out;
}

/* sv_move.c:286 */
void SV_NewChaseDir (edict_t *actor, edict_t *enemy, float dist)
{
	int r = quake_rs_sv_new_chase_dir (actor, enemy, dist);
	Host_Reraise (r);
}

/* sv_move.c:392 -- QuakeC builtin calling convention; no parameters or
   return value to marshal beyond what the ambient qcvm already carries. */
void SV_MoveToGoal (void)
{
	int r = quake_rs_sv_move_to_goal ();
	Host_Reraise (r);
}

#endif /* USE_RUST_HOST */
