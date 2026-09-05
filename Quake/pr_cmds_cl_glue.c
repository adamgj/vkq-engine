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
// pr_cmds_cl_glue.c -- the C frame around the Rust client-coupled builtins
// (Rust migration Phase 7 M5, Group F: PF_cl_sound, PF_cl_ambientsound,
// PF_cl_precache_sound, PF_cl_makestatic, PF_cl_particle).
//
// Compiled under -Duse_rust_host, NOT -Duse_rust_progs, matching
// pr_cmds_sv_glue.c's rationale: the Rust module that calls these
// (rust/quake-capi/src/progs_builtins_cl.rs) is gated on the `host` cargo
// feature, so the glue has to be compiled under exactly the same condition or
// the link breaks in a -Duse_rust_progs-only configuration.
//
// Three guarded seams (ADR-009 rule 3):
//  1. PR_GetString Host_Errors on an out-of-range string handle
//     (pr_edict_arena.c).
//  2. PR_CheckEmptyString's PR_RunError ("Bad string") (pr_cmds.c), used by
//     PF_cl_precache_sound.
//  3. PF_cl_makestatic's Mem_Realloc / Mem_Alloc failure Host_Error
//     ("Too many static entities") -- the whole body is kept in C (ADR-007):
//     entity_t and cl.static_entities have no ADR-011 mirror in Phase 7.
// None of those longjmps may unwind a Rust frame; each returns a Host_Guard
// status that pr_cmds_glue.c's PRBI_Raise re-issues as PRBI_ERR_GUARD.
//
// S_PrecacheSound, S_StartSound, S_StaticSound, PScript_RunParticleEffect,
// PScript_RunParticleEffectTypeString and R_RunParticleEffect are called
// directly from Rust (no glue needed here): none of them reach Host_Error,
// only S_FindName's pathological-path Sys_Error, which is fatal and not
// caught by Host_Guard.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

/* ---------------------------------------------------------------------------
 * Guarded seams (ADR-009 rule 3).
 */

typedef struct
{
	int			 handle;
	const char **out;
} prbi_cl_getstring_arg_t;

/* pr_edict_arena.c PR_GetString -- Host_Errors on an out-of-range negative
   string index. (Preserved-bug note: PR_GetString's final `else` branch has
   an unreachable Host_Error after an unconditional `return qcvm->strings;`,
   so in practice this call can only ever raise from the documented in-range
   checks earlier in the function, never that branch.) */
static void PRBI_ClInvokeGetString (void *p)
{
	prbi_cl_getstring_arg_t *a = (prbi_cl_getstring_arg_t *)p;
	*a->out = PR_GetString (a->handle);
}

int PRBI_ClGlue_GetString (int handle, const char **out)
{
	prbi_cl_getstring_arg_t arg;

	*out = NULL;
	arg.handle = handle;
	arg.out = out;
	return Host_Guard (PRBI_ClInvokeGetString, &arg);
}

/* pr_cmds.c PR_CheckEmptyString ("Bad string"), used by PF_cl_precache_sound. */
static void PRBI_ClInvokeCheckEmptyString (void *p)
{
	const char *s = (const char *)p;
	if (s[0] <= ' ')
		PR_RunError ("Bad string");
}

int PRBI_ClGlue_CheckEmptyString (const char *s)
{
	return Host_Guard (PRBI_ClInvokeCheckEmptyString, (void *)s);
}

/* pr_cmds.c PF_cl_makestatic, kept whole in C (ADR-007): entity_t and
   cl.static_entities have no ADR-011 mirror in Phase 7. Only the
   Mem_Realloc / Mem_Alloc failure branch raises (Host_Error
   ("Too many static entities")); SV_BuildEntityState, R_AddEfrags and
   ED_Free never raise. */
static void PRBI_ClInvokeMakeStatic (void *p)
{
	edict_t	 *ent = (edict_t *)p;
	entity_t *stat;
	int		  i;

	i = cl.num_statics;
	if (i >= cl.max_static_entities)
	{
		int		   ec = 64;
		entity_t **newstatics = Mem_Realloc (cl.static_entities, sizeof (*newstatics) * (cl.max_static_entities + ec));
		entity_t  *newents = Mem_Alloc (sizeof (*newents) * ec);
		if (!newstatics || !newents)
			Host_Error ("Too many static entities");
		cl.static_entities = newstatics;
		while (ec--)
			cl.static_entities[cl.max_static_entities++] = newents++;
	}

	stat = cl.static_entities[i];
	cl.num_statics++;

	SV_BuildEntityState (ent, &stat->baseline);

	// copy it to the current state
	stat->netstate = stat->baseline;
	stat->eflags = stat->netstate.eflags; // spike -- annoying and probably not used anyway, but w/e

	stat->trailstate = NULL;
	stat->emitstate = NULL;
	stat->model = cl.model_precache[stat->baseline.modelindex];
	stat->frame = stat->baseline.frame;
	stat->lerp.prev_frame = stat->frame; // johnfitz -- lerping

	stat->skinnum = stat->baseline.skin;
	stat->effects = stat->baseline.effects;
	stat->alpha = stat->baseline.alpha; // johnfitz -- alpha

	VectorCopy (ent->baseline.origin, stat->origin);
	VectorCopy (ent->baseline.angles, stat->angles);
	if (stat->model)
		R_AddEfrags (stat);

	// throw the entity away now
	ED_Free (ent);
}

int PRBI_ClGlue_MakeStatic (void *ent)
{
	return Host_Guard (PRBI_ClInvokeMakeStatic, ent);
}

#endif // USE_RUST_HOST
