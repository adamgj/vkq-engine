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
// pr_cmds_glue.c -- the C frame around the ported QuakeC builtins.
//
// Compiled alongside pr_cmds.c under -Duse_rust_progs (Phase 6 M7). The flip
// is per-slot: pr_cmds.c's tables name rust_pf_* through the PF_RS macro, and
// every builtin absent from this file is still the C original.
//
// The wrappers exist for three reasons: the builtin_t signature is void(void)
// while a builtin that can raise has to return a status; the one PR_RunError
// must issue from a C frame after the Rust frame has gone (ADR-009); and the
// engine seams a ported builtin calls are gathered in one place where the
// "no seam that can Host_Error" rule can be checked by eye.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

/* status codes shared with rust/quake-capi/src/progs_builtins.rs */
#define PRBI_OK					 0
#define PRBI_ERR_FIND_BAD_STRING 1
#define PRBI_ERR_NO_STRING		 2

/* ---- engine seams. Every one of these is a leaf, or reaches only
   Sys_Error/Con_* -- none can Host_Error, which is the rule that decides
   which builtins may be ported at all while the interpreter's Host_Guard sits
   outside the dispatch (ADR-009). ---- */

double PRBI_Glue_Ceil (double v)
{
	return ceil (v);
}

void PRBI_Glue_AngleVectors (const float *angles)
{
	AngleVectors ((float *)angles, pr_global_struct->v_forward, pr_global_struct->v_right, pr_global_struct->v_up);
}

/* PR_GetTempString steps the process-global ring; q_snprintf's truncation at
   STRINGTEMP_LENGTH is reproduced here so the ring is stepped exactly once
   per call, as it is in C. */
int PRBI_Glue_StoreTempString (const char *bytes, int len)
{
	char *s = PR_GetTempString ();
	int	  n = (len < STRINGTEMP_LENGTH - 1) ? len : STRINGTEMP_LENGTH - 1;
	if (n < 0)
		n = 0;
	memcpy (s, bytes, (size_t)n);
	s[n] = 0;
	return PR_SetEngineString (s);
}

const char *PRBI_Glue_VarString (int first)
{
	return PF_VarString (first);
}

float PRBI_Glue_CvarValue (const char *name)
{
	return Cvar_VariableValue (name);
}

qboolean PRBI_Glue_ChangelevelIssued (qboolean set)
{
	qboolean was = svs.changelevel_issued;
	if (set)
		svs.changelevel_issued = true;
	return was;
}

/* ---- the builtin_t wrappers named by pr_cmds.c's tables ---- */

/* Every raise happens here, in a C frame, after the Rust builtin has returned
   (ADR-009). PR_GetString's non-existent-string Host_Error is C's own message,
   reproduced verbatim: the port reports it rather than raising inside the
   string table. */
FUNC_NORETURN static void PRBI_Raise (int status, int detail, const char *name)
{
	switch (status)
	{
	case PRBI_ERR_FIND_BAD_STRING:
		PR_RunError ("PF_Find: bad search string");
	case PRBI_ERR_NO_STRING:
		Host_Error ("PR_GetString: attempt to get a non-existant string %d\n", detail);
	default:
		PR_RunError ("PF_%s: unknown status %i", name, status);
	}
}

#define RUST_PF(name)                              \
	void rust_pf_##name (void)                     \
	{                                              \
		int detail = 0;                            \
		int status = quake_rs_pf_##name (&detail); \
		if (status != PRBI_OK)                     \
			PRBI_Raise (status, detail, #name);    \
	}

RUST_PF (normalize)
RUST_PF (vlen)
RUST_PF (vectoyaw)
RUST_PF (vectoangles)
RUST_PF (makevectors)
RUST_PF (random)
RUST_PF (fabs)
RUST_PF (floor)
RUST_PF (ceil)
RUST_PF (rint)
RUST_PF (ftos)
RUST_PF (vtos)
RUST_PF (cvar)
RUST_PF (cvar_set)
RUST_PF (localcmd)
RUST_PF (nextent)
RUST_PF (traceon)
RUST_PF (traceoff)
RUST_PF (precache_file)
RUST_PF (dprint)
RUST_PF (coredump)
RUST_PF (Find)
