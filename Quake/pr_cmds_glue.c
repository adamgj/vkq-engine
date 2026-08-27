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
#define PRBI_OK						  0
#define PRBI_ERR_FIND_BAD_STRING	  1
#define PRBI_ERR_NO_STRING			  2
/* a guarded seam raised: detail is Host_Guard's result, re-issued below */
#define PRBI_ERR_GUARD				  3
#define PRBI_ERR_PROGRAM_ERROR		  4
#define PRBI_ERR_WRITEDEST_NOT_CLIENT 5
#define PRBI_ERR_WRITEDEST_BAD_DEST	  6
#define PRBI_ERR_BAD_EDICT_POINTER	  7

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

/* ---- guarded seams (ADR-009 rule 3) ----
   ED_Alloc, ED_Free, ED_Print and ED_PrintNum can all Host_Error. The
   interpreter's Host_Guard wraps the builtin *dispatch*, so a raise from
   inside a ported builtin would longjmp over its Rust frame. Each of these
   therefore carries its own guard: the jump is caught here, travels back
   through Rust as a plain status, and PRBI_Raise re-issues it below once the
   Rust frame has returned. This is what makes batch 2 portable at all. */

typedef struct
{
	int num;
} prbi_edict_arg_t;

static void PRBI_DoEdAlloc (void *p)
{
	((prbi_edict_arg_t *)p)->num = NUM_FOR_EDICT_NO_CHECK (ED_Alloc ());
}

int PRBI_Glue_EdAlloc (int *num)
{
	prbi_edict_arg_t arg;
	int				 guard;

	arg.num = 0;
	guard = Host_Guard (PRBI_DoEdAlloc, &arg);
	if (!guard)
		*num = arg.num;
	return guard;
}

static void PRBI_DoEdFree (void *p)
{
	ED_Free (EDICT_NUM_NO_CHECK (((prbi_edict_arg_t *)p)->num));
}

int PRBI_Glue_EdFree (int num)
{
	prbi_edict_arg_t arg;
	arg.num = num;
	return Host_Guard (PRBI_DoEdFree, &arg);
}

/* PF_error/PF_objerror print a banner and then dump the entity. ED_Print
   writes to the console too, so the two halves have to run in one C frame:
   deferring the banner the way the other diagnostics are deferred would put
   the entity dump above the "======SERVER ERROR" line. */
typedef struct
{
	const char *banner;
	int			num;
} prbi_banner_arg_t;

static void PRBI_DoEdPrintWithBanner (void *p)
{
	prbi_banner_arg_t *a = (prbi_banner_arg_t *)p;
	Con_Printf ("%s", a->banner);
	ED_Print (EDICT_NUM_NO_CHECK (a->num));
}

int PRBI_Glue_EdPrintWithBanner (const char *banner, int num)
{
	prbi_banner_arg_t arg;
	arg.banner = banner;
	arg.num = num;
	return Host_Guard (PRBI_DoEdPrintWithBanner, &arg);
}

static void PRBI_DoEdPrintNum (void *p)
{
	ED_PrintNum (((prbi_edict_arg_t *)p)->num);
}

int PRBI_Glue_EdPrintNum (int num)
{
	prbi_edict_arg_t arg;
	arg.num = num;
	return Host_Guard (PRBI_DoEdPrintNum, &arg);
}

/* ---- message writing ---- */

int PRBI_Glue_MaxClients (void)
{
	return svs.maxclients;
}

/* The sizebuf_t WriteDest chose, rebuilt from the destination code and (for
   MSG_ONE) the client's edict number the Rust side already range-checked.
   MSG_EXT_ENTITY reuses MSG_EXT_MULTICAST's buffer, as it does in C. */
static sizebuf_t *PRBI_WriteDest (int dest, int entnum)
{
	switch (dest)
	{
	case MSG_ONE:
		return &svs.clients[entnum - 1].message;
	case MSG_ALL:
		return &sv.reliable_datagram;
	case MSG_INIT:
		return &sv.signon;
	case MSG_EXT_MULTICAST:
	case MSG_EXT_ENTITY:
		return &sv.multicast;
	default:
		return &sv.datagram;
	}
}

void PRBI_Glue_MsgWrite (int dest, int entnum, int kind, int i, float f, const char *bytes)
{
	extern unsigned int sv_protocol_pext2; /* as PF_sv_WriteEntity declares it */
	sizebuf_t		   *sb = PRBI_WriteDest (dest, entnum);

	switch (kind)
	{
	case 0:
		MSG_WriteByte (sb, i);
		break;
	case 1:
		MSG_WriteChar (sb, i);
		break;
	case 2:
		MSG_WriteShort (sb, i);
		break;
	case 3:
		MSG_WriteLong (sb, i);
		break;
	case 4:
		MSG_WriteAngle (sb, f, sv.protocolflags);
		break;
	case 5:
		MSG_WriteCoord (sb, f, sv.protocolflags);
		break;
	case 6:
		MSG_WriteString (sb, LOC_GetString (bytes));
		break;
	default:
		MSG_WriteEntity (sb, i, sv_protocol_pext2);
		break;
	}
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
	case PRBI_ERR_GUARD:
		/* a guarded seam's Host_Error/Host_EndGame, re-issued now that the
		   Rust frame has returned normally (ADR-009 rule 3). */
		Host_Reraise (detail);
		Sys_Error ("PRBI_Raise: Host_Reraise returned");
	case PRBI_ERR_PROGRAM_ERROR:
		Host_Error ("Program error");
	case PRBI_ERR_WRITEDEST_NOT_CLIENT:
		PR_RunError ("WriteDest: not a client");
	case PRBI_ERR_WRITEDEST_BAD_DEST:
		PR_RunError ("WriteDest: bad destination");
	case PRBI_ERR_BAD_EDICT_POINTER:
		Host_Error ("NUM_FOR_EDICT: bad pointer");
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
RUST_PF (Spawn)
RUST_PF (Remove)
RUST_PF (eprint)
RUST_PF (error)
RUST_PF (objerror)
RUST_PF (sv_WriteByte)
RUST_PF (sv_WriteChar)
RUST_PF (sv_WriteShort)
RUST_PF (sv_WriteLong)
RUST_PF (sv_WriteAngle)
RUST_PF (sv_WriteCoord)
RUST_PF (sv_WriteString)
RUST_PF (sv_WriteEntity)
