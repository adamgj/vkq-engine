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
// pr_cmds_sv_glue.c -- the C frame around the Rust server-coupled builtins
// (Rust migration Phase 7 M5, Groups A/B/C).
//
// Compiled under -Duse_rust_host, NOT -Duse_rust_progs. The Rust module that
// calls these (rust/quake-capi/src/progs_builtins_sv.rs) is gated on the
// `host` cargo feature because its bodies reach world.c / sv_move.c / sv_phys.c
// cores that only exist there; the glue has to be compiled under exactly the
// same condition or the link breaks in the -Duse_rust_progs-only config.
// pr_cmds_glue.c is the progs-gated sibling and stays untouched by M5 wave 1
// apart from its RUST_PF rows.
//
// Two jobs (ADR-009 rule 3):
//
//  1. Guard every seam these builtins reach that can Host_Error / PR_RunError:
//     SetMinMaxSize's "backwards mins/maxs", PF_setmodel's whole precache
//     lookup (G_STRING, the Con_Warning, SV_Precache_Model -> Mod_ForName and
//     the "no precache" PR_RunError), and the traceline/tracebox NAN warning
//     whose NUM_FOR_EDICT argument Host_Errors on a bad pointer. None of those
//     longjmps may unwind a Rust frame; each returns a Host_Guard status that
//     pr_cmds_glue.c's PRBI_Raise re-issues as PRBI_ERR_GUARD.
//  2. Keep the `server_t` reads in C. `sv.lastcheck` / `sv.lastchecktime` have
//     no ADR-011 mirror in Phase 7 (server_t is not in quake-types), so Rust
//     goes through accessors instead of the struct.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

/* ---------------------------------------------------------------------------
 * Guarded seams (ADR-009 rule 3).
 */

/* pr_cmds.c:239-241. SetMinMaxSize raises before it touches the edict, so the
   Rust caller only needs the raise, not the loop. */
static void PRBI_SvInvokeBackwardsMinsMaxs (void *p)
{
	(void)p;
	PR_RunError ("backwards mins/maxs");
}

int PRBI_SvGlue_RunErrorBackwardsMinsMaxs (void)
{
	return Host_Guard (PRBI_SvInvokeBackwardsMinsMaxs, NULL);
}

typedef struct
{
	int			 handle;
	const char **out_name;
	int			*out_index;
	void	   **out_model;
} prbi_setmodel_arg_t;

/* pr_cmds.c:346-370, kept whole in C on purpose. Three reasons:
   - G_STRING expands to PR_GetString, which Host_Errors on a bad handle;
   - SV_Precache_Model reaches Mod_ForName, which Host_Errors;
   - the scan's aliasing is load-bearing. `check` is NOT advanced past the
	 NULL terminator, and SV_Precache_Model writes sv.model_precache[i] into
	 that very slot, so `*check` afterwards yields the freshly precached name.
	 The loop also reads sv.model_precache[MAX_MODELS] when the table is full.
   Both behaviours are preserved by leaving the code where it is.
   sv.models[i] is read here rather than on the Rust side because `server_t`
   has no ADR-011 mirror; `i` is what pr_cmds.c round-trips through the float
   e->v.modelindex, which is lossless for every in-range index. */
static void PRBI_SvInvokeSetModelLookup (void *p)
{
	prbi_setmodel_arg_t *a = (prbi_setmodel_arg_t *)p;
	const char			*m;
	const char		   **check;
	int					 i;

	m = PR_GetString (a->handle);

	for (i = 0, check = sv.model_precache; *check; i++, check++)
	{
		if (!strcmp (*check, m))
			break;
	}

	if (!*check)
	{
		if (pr_checkextension.value)
		{
			Con_Warning ("PF_setmodel(\"%s\"): Model was not precached\n", m);
			i = SV_Precache_Model (m);
		}
		else
			PR_RunError ("no precache: %s", m);
	}

	*a->out_name = *check;
	*a->out_index = i;
	*a->out_model = (void *)sv.models[i];
}

int PRBI_SvGlue_SetModelLookup (int handle, const char **out_name, int *out_index, void **out_model)
{
	prbi_setmodel_arg_t arg;

	*out_name = NULL;
	*out_index = 0;
	*out_model = NULL;

	arg.handle = handle;
	arg.out_name = out_name;
	arg.out_index = out_index;
	arg.out_model = out_model;
	return Host_Guard (PRBI_SvInvokeSetModelLookup, &arg);
}

typedef struct
{
	float	*v1;
	float	*v2;
	edict_t *ent;
} prbi_nantrace_arg_t;

/* pr_cmds.c:755 and pr_ext.c:1851 -- byte-identical message text in both, the
   tracebox copy included ("traceline", not "tracebox"). NUM_FOR_EDICT
   Host_Errors on a bad pointer (pr_edict.c:1082) and is evaluated before
   Con_Warning runs, so the whole line is inside the guard. */
static void PRBI_SvInvokeWarnNanTrace (void *p)
{
	prbi_nantrace_arg_t *a = (prbi_nantrace_arg_t *)p;

	Con_Warning (
		"NAN in traceline:\nv1(%f %f %f) v2(%f %f %f)\nentity %d\n", a->v1[0], a->v1[1], a->v1[2], a->v2[0], a->v2[1], a->v2[2], NUM_FOR_EDICT (a->ent));
}

int PRBI_SvGlue_WarnNanTrace (float *v1, float *v2, edict_t *ent)
{
	prbi_nantrace_arg_t arg;

	arg.v1 = v1;
	arg.v2 = v2;
	arg.ent = ent;
	return Host_Guard (PRBI_SvInvokeWarnNanTrace, &arg);
}

/* ---------------------------------------------------------------------------
 * Non-raising server_t accessors (server.h:59-60). PF_checkclient's round-robin
 * cursor lives in `sv`, which both VMs share; it is deliberately NOT moved into
 * qcvm_t.
 */

int PRBI_SvGlue_SvLastCheck (void)
{
	return sv.lastcheck;
}

void PRBI_SvGlue_SetSvLastCheck (int value)
{
	sv.lastcheck = value;
}

double PRBI_SvGlue_SvLastCheckTime (void)
{
	return sv.lastchecktime;
}

void PRBI_SvGlue_SetSvLastCheckTime (double value)
{
	sv.lastchecktime = value;
}

#endif /* USE_RUST_HOST */
