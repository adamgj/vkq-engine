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
// world_glue.c -- the C frame around the Rust world/collision port.
//
// Compiled instead of world.c under -Duse_rust_host (Rust migration Phase 7
// M3). Four jobs:
//
//  1. Own the two C-visible cvar objects world.c defined (world.c:33-35).
//     sv_main.c registers them and other translation units read them, so the
//     storage stays here and Rust reads .value through an extern.
//  2. Guard everything world.c reached that can Host_Error (ADR-009 rule 3):
//     the SV_TouchLinks touch dispatch, the EDICT_NUM / NUM_FOR_EDICT bounds
//     checks and the SV_TouchLinks assert_always. None of those longjmps may
//     unwind the Rust frame that issued them.
//  3. Re-raise, from a pure C frame, what those guards caught. Every world.c
//     entry point whose body can reach Host_Error is a thin wrapper over a
//     quake_rs_* status core: SV_LinkEdict, SV_HullForEntity,
//     SV_ClipMoveToEntity, SV_Move, SV_TestEntityPosition and
//     SV_PointContentsAllBsps. Host_Reraise is called only from here.
//  4. Keep the cl / entity_t reads World_ClipToNetwork needs in C. `cl`,
//     `entity_t` and the PSET_SCRIPT-conditional members inside it have no
//     ADR-011 mirror until M7, so Rust goes through accessors instead.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

/* ---------------------------------------------------------------------------
 * C-visible cvar objects (world.c:33-35).
 */

// QSS has an optimzed path for SV_RecursiveHullCheck if FTE extensions are available
// but it differs from other engines and sometimes entities are falling through
// the world at level start because being misplaced.
// In such case, disable it (0) to get the same behaviour as QuakeSpasm.
cvar_t sv_fte_recursivehullckeck = {"sv_fte_recursivehullckeck", "1", CVAR_NONE};

cvar_t sv_fte_createareanode = {"sv_fte_createareanode", "1", CVAR_NONE};

/* ---------------------------------------------------------------------------
 * Guarded callbacks (ADR-009 rule 3).
 */

typedef struct
{
	edict_t *touch;
	edict_t *other;
	float	 time;
} world_touch_arg_t;

static void World_InvokeTouch (void *p)
{
	world_touch_arg_t *a = (world_touch_arg_t *)p;

	pr_global_struct->self = EDICT_TO_PROG (a->touch);
	pr_global_struct->other = EDICT_TO_PROG (a->other);
	pr_global_struct->time = a->time;
	PR_ExecuteProgram (a->touch->v.touch);
}

/* world.c:373-376. The second parameter is the entity being linked; it becomes
   pr_global_struct->other, not ->self (->self is `touch`). The frozen contract
   spells the parameter `self`, so the name is kept for the ABI both agents code
   against, but the assignment order below is world.c's. */
int World_Glue_CallTouch (edict_t *touch, edict_t *self, float time)
{
	world_touch_arg_t arg;

	arg.touch = touch;
	arg.other = self;
	arg.time = time;
	return Host_Guard (World_InvokeTouch, &arg);
}

typedef struct
{
	int		  num;
	edict_t **out;
} world_edictnum_arg_t;

static void World_InvokeEdictNum (void *p)
{
	world_edictnum_arg_t *a = (world_edictnum_arg_t *)p;
	*a->out = EDICT_NUM (a->num);
}

/* EDICT_NUM Host_Errors on a bad index (pr_edict.c:1059), so the Rust
   SV_TouchLinks loop cannot expand the macro itself. */
int World_Glue_EdictNum (int num, edict_t **out)
{
	world_edictnum_arg_t arg;

	arg.num = num;
	arg.out = out;
	*out = NULL;
	return Host_Guard (World_InvokeEdictNum, &arg);
}

typedef struct
{
	edict_t *ent;
	int		*out;
} world_numforedict_arg_t;

static void World_InvokeNumForEdict (void *p)
{
	world_numforedict_arg_t *a = (world_numforedict_arg_t *)p;
	*a->out = NUM_FOR_EDICT (a->ent);
}

/* NUM_FOR_EDICT Host_Errors on a bad pointer (pr_edict.c:1082). */
int World_Glue_NumForEdict (edict_t *ent, int *out)
{
	world_numforedict_arg_t arg;

	arg.ent = ent;
	arg.out = out;
	*out = 0;
	return Host_Guard (World_InvokeNumForEdict, &arg);
}

typedef struct
{
	const char *expr;
	const char *file;
	int			line;
} world_assert_arg_t;

static void World_InvokeAssertFailed (void *p)
{
	world_assert_arg_t *a = (world_assert_arg_t *)p;
	COM_Assert_Failed (a->expr, a->file, a->line);
}

/* assert_always's failure branch (quakedef.h:336) reaches Host_Error on the
   main thread. Rust passes the exact #e / __FILE__ / __LINE__ world.c would
   have produced so the message stays byte-identical. */
int World_Glue_AssertFailed (const char *expr, const char *file, int line)
{
	world_assert_arg_t arg;

	arg.expr = expr;
	arg.file = file;
	arg.line = line;
	return Host_Guard (World_InvokeAssertFailed, &arg);
}

/* world.c:144 and world.c:151. PR_GetString reaches Host_Error on a corrupt
   string_t (pr_edict_arena.c:315), so both warnings run under a guard; the
   format strings also carry %f, which ADR-005's Rust formatter refuses, so the
   two lines stay in C verbatim. */
static void World_InvokeWarnSolidBspNoPush (void *p)
{
	edict_t *ent = (edict_t *)p;
	Con_Warning ("SOLID_BSP without MOVETYPE_PUSH (%s at %f %f %f)\n", PR_GetString (ent->v.classname), ent->v.origin[0], ent->v.origin[1], ent->v.origin[2]);
}

int World_Glue_WarnSolidBspNoPush (edict_t *ent)
{
	return Host_Guard (World_InvokeWarnSolidBspNoPush, ent);
}

static void World_InvokeWarnSolidBspNonBspModel (void *p)
{
	edict_t *ent = (edict_t *)p;
	Con_Warning ("SOLID_BSP with a non bsp model (%s at %f %f %f)\n", PR_GetString (ent->v.classname), ent->v.origin[0], ent->v.origin[1], ent->v.origin[2]);
}

int World_Glue_WarnSolidBspNonBspModel (edict_t *ent)
{
	return Host_Guard (World_InvokeWarnSolidBspNonBspModel, ent);
}

/* ---------------------------------------------------------------------------
 * Re-raising public entry points. Each is the exact world.c signature; the
 * Rust body is a quake_rs_* status core and the jump is re-issued from here,
 * never from a Rust frame (ADR-009).
 */

void SV_LinkEdict (edict_t *ent, qboolean touch_triggers)
{
	int r = quake_rs_sv_link_edict (ent, touch_triggers);
	Host_Reraise (r);
}

/* world.c:131 -- file-private in world.c, but the differential harness drives
   it by name, so the plain symbol is exported here too. */
hull_t *SV_HullForEntity (edict_t *ent, vec3_t mins, vec3_t maxs, vec3_t offset)
{
	hull_t *hull = NULL;
	int		r = quake_rs_sv_hull_for_entity (ent, mins, maxs, offset, &hull);
	Host_Reraise (r);
	return hull;
}

/* world.c:924 */
trace_t SV_ClipMoveToEntity (edict_t *ent, vec3_t start, vec3_t mins, vec3_t maxs, vec3_t end, unsigned int hitcontents)
{
	trace_t t;
	int		r;

	memset (&t, 0, sizeof (t));
	r = quake_rs_sv_clip_move_to_entity (&t, ent, start, mins, maxs, end, hitcontents);
	Host_Reraise (r);
	return t;
}

/* world.c:1264 */
trace_t SV_Move (vec3_t start, vec3_t mins, vec3_t maxs, vec3_t end, int type, edict_t *passedict)
{
	trace_t t;
	int		r;

	memset (&t, 0, sizeof (t));
	r = quake_rs_sv_move (&t, start, mins, maxs, end, type, passedict);
	Host_Reraise (r);
	return t;
}

/* world.c:604 */
edict_t *SV_TestEntityPosition (edict_t *ent)
{
	edict_t *out = NULL;
	int		 r = quake_rs_sv_test_entity_position (ent, &out);
	Host_Reraise (r);
	return out;
}

/* world.c:588 / world.h:68 -- calls SV_Move, so it raises like the rest. */
int SV_PointContentsAllBsps (vec3_t p, edict_t *forent)
{
	int c = 0;
	int r = quake_rs_sv_point_contents_all_bsps (&c, p, forent);
	Host_Reraise (r);
	return c;
}

/* ---------------------------------------------------------------------------
 * Thin, non-raising shims.
 */

/* sv_phys.c owns the entity grid; it stays C through M3. */
void World_Glue_PushGridEntityLinked (edict_t *ent)
{
	SV_PushGridEntityLinked (ent);
}

/* world.c:1305 -- the CSQC branch of SV_Move. */
int World_Glue_QcvmIsClient (void)
{
	return qcvm == &cl.qcvm;
}

int World_Glue_ClNumEntities (void)
{
	return cl.num_entities;
}

entity_t *World_Glue_ClEntity (int i)
{
	if (i < 0 || i >= cl.num_entities)
		return NULL;
	return cl.entities + i;
}

/* Everything World_ClipToNetwork reads off an entity_t (world.c:1083-1200) in
   one call. entity_t has PSET_SCRIPT-conditional members and embeds
   lightcache_t / entlerp_t, so it gets no ADR-011 mirror before M7. qmodel_t is
   already mirrored, so the model pointer is handed back raw. */
void World_Glue_EntClipInfo (entity_t *e, unsigned int *solidsize, qmodel_t **model, vec3_t origin, vec3_t angles, int *skinnum)
{
	*solidsize = e->netstate.solidsize;
	*model = e->model;
	VectorCopy (e->origin, origin);
	VectorCopy (e->angles, angles);
	*skinnum = e->skinnum;
}

/* world.c:864 */
void World_Glue_DPrintBackupPast0 (void)
{
	Con_DPrintf ("backup past 0\n");
}

#endif /* USE_RUST_HOST */
