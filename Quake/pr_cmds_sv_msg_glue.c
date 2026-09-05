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
// pr_cmds_sv_msg_glue.c -- the C frame around the Rust message builtins
// (Rust migration Phase 7 M5, wave 2 Group D).
//
// Compiled under -Duse_rust_host, NOT -Duse_rust_progs, exactly like
// pr_cmds_sv_glue.c (wave 1's sibling glue file for Groups A/B/C): the Rust
// module that calls these (rust/quake-capi/src/progs_builtins_sv_msg.rs) is
// gated on the `host` cargo feature.
//
// Two jobs (ADR-009 rule 3):
//
//  1. Guard every seam these builtins reach that can Host_Error / PR_RunError:
//     PF_stuffcmd's "Parm 0 not a client" (pr_cmds.c:938), G_STRING's
//     PR_GetString (pr_edict_arena.c:315), and PF_VarString as a whole (its
//     own G_STRING resolution per variadic argument, plus its two
//     Con_Warning/Con_DWarning overflow diagnostics -- following the guarded
//     Con_Warning precedent pr_cmds_sv_glue.c and world_glue.c already
//     establish for a warning whose arguments can themselves raise). None of
//     those longjmps may unwind a Rust frame.
//  2. Provide the leaf server_t/client_t writes Rust cannot do directly:
//     server_t/client_t have no ADR-011 mirror, and Host_ClientCommands /
//     SV_BroadcastPrintf are C variadic functions Rust cannot call through
//     FFI, so each gets a plain (non-variadic, non-raising) wrapper here.
//
// WriteDest() (pr_cmds.c:1627) is deliberately NOT wrapped whole: the M5
// contract asks Group D to resolve MSG_ONE's "not a client" and the
// bad-destination default case as PRBI_ERR_WRITEDEST_NOT_CLIENT /
// PRBI_ERR_WRITEDEST_BAD_DEST (pr_cmds_glue.c's shared PRBI_Raise already
// carries both arms), so the dispatch is reimplemented on the Rust side using
// the same guarded World_Glue_NumForEdict() world_glue.c already exports, and
// this file only re-derives the destination sizebuf_t* from the (dest,
// entnum) pair the Rust side has already validated -- exactly mirroring
// pr_cmds_glue.c's own private PRBI_WriteDest() helper for the base Write*
// builtins.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

/* ---------------------------------------------------------------------------
 * Guarded seams (ADR-009 rule 3).
 */

typedef struct
{
	int entnum;
} prbi_msg_client_check_arg_t;

/* pr_cmds.c:938. PF_stuffcmd raises before touching host_client, so the Rust
   caller only needs the raise. */
static void PRBI_MsgInvokeStuffcmdClientCheck (void *p)
{
	prbi_msg_client_check_arg_t *a = (prbi_msg_client_check_arg_t *)p;
	if (a->entnum < 1 || a->entnum > svs.maxclients)
		PR_RunError ("Parm 0 not a client");
}

int PRBI_MsgGlue_StuffcmdClientCheck (int entnum)
{
	prbi_msg_client_check_arg_t arg;
	arg.entnum = entnum;
	return Host_Guard (PRBI_MsgInvokeStuffcmdClientCheck, &arg);
}

typedef struct
{
	int			ofs;
	const char *out;
} prbi_msg_getstring_arg_t;

/* progs.h:174 G_STRING (o) = PR_GetString (*(string_t *)&qcvm->globals[o]).
   `out` aliases the engine's string arena, exactly like G_STRING's own
   result -- not a temp-string-ring buffer, so no copy is needed. */
static void PRBI_MsgInvokeGetString (void *p)
{
	prbi_msg_getstring_arg_t *a = (prbi_msg_getstring_arg_t *)p;
	a->out = G_STRING (a->ofs);
}

int PRBI_MsgGlue_GetString (int ofs, const char **out)
{
	prbi_msg_getstring_arg_t arg;
	arg.ofs = ofs;
	arg.out = NULL;
	int guard = Host_Guard (PRBI_MsgInvokeGetString, &arg);
	if (!guard)
		*out = arg.out;
	return guard;
}

typedef struct
{
	int	 first;
	char out[1024];
} prbi_msg_varstring_arg_t;

/* pr_cmds.c:111. PF_VarString's LOC_GetString/LOC_Format layer is out of M5's
   scope to reimplement in Rust (T5 ports builtins, not the localisation
   engine), so the whole call runs here, guarded: every G_STRING it resolves
   can Host_Error, and its overflow/length Con_Warning|Con_DWarning calls
   carry only %s/%i/%d (no %f), but still must not run with a live Rust frame
   below them. `out` is `PF_VarString`'s own `static char out[1024]`, copied
   before the guard returns since the caller reuses that static buffer on its
   next call. */
static void PRBI_MsgInvokeVarString (void *p)
{
	prbi_msg_varstring_arg_t *a = (prbi_msg_varstring_arg_t *)p;
	const char				 *s = PF_VarString (a->first);
	memcpy (a->out, s, sizeof (a->out));
}

int PRBI_MsgGlue_VarString (int first, char *out)
{
	prbi_msg_varstring_arg_t arg;
	arg.first = first;
	arg.out[0] = 0;
	int guard = Host_Guard (PRBI_MsgInvokeVarString, &arg);
	if (!guard)
		memcpy (out, arg.out, sizeof (arg.out));
	return guard;
}

/* ---------------------------------------------------------------------------
 * Leaves: none of these can Host_Error / PR_RunError.
 */

/* pr_cmds.c:942-945. host_client is saved/restored around the call exactly as
   PF_stuffcmd does; Host_ClientCommands is variadic, so this plain wrapper is
   what Rust's FFI can call. entnum is 1-based and already range-checked by
   the caller (PRBI_MsgGlue_StuffcmdClientCheck ran first). */
void PRBI_MsgGlue_ClientCommandsPlain (int entnum, const char *str)
{
	client_t *old = host_client;
	host_client = &svs.clients[entnum - 1];
	Host_ClientCommands ("%s", str);
	host_client = old;
}

/* pr_cmds.c:401. SV_BroadcastPrintf is variadic; this is the plain wrapper. */
void PRBI_MsgGlue_BroadcastPrintfPlain (const char *str)
{
	SV_BroadcastPrintf ("%s", str);
}

/* pr_cmds.c:430-431 / :460-461. kind 0 is PF_sprint's svc_print, kind 1 is
   PF_centerprint's svc_centerprint. entnum is 1-based and already
   range-checked by the caller (the "tried to sprint/centerprint to a
   non-client" warning is a soft, non-raising return on the Rust side, so this
   is only ever called once entnum is known good). */
void PRBI_MsgGlue_ClientMessageWrite (int entnum, int kind, const char *str)
{
	client_t *client = &svs.clients[entnum - 1];
	MSG_WriteChar (&client->message, kind ? svc_centerprint : svc_print);
	MSG_WriteString (&client->message, str);
}

/* ---------------------------------------------------------------------------
 * Extended message writers (pr_ext.c). WriteDest()'s dispatch is reimplemented
 * on the Rust side (see the file header); this reproduces only its private
 * sizebuf_t selection (pr_cmds_glue.c's own PRBI_WriteDest, duplicated here
 * since that one is static to pr_cmds_glue.c). dest/entnum are already
 * validated by the caller, so MSG_ONE indexing is safe.
 */

static sizebuf_t *PRBI_MsgWriteDest (int dest, int entnum)
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
	default: /* MSG_BROADCAST */
		return &sv.datagram;
	}
}

/* pr_ext.c:2594. */
void PRBI_MsgGlue_WriteFloat (int dest, int entnum, float f)
{
	MSG_WriteFloat (PRBI_MsgWriteDest (dest, entnum), f);
}

/* pr_ext.c:2598. */
void PRBI_MsgGlue_WriteDouble (int dest, int entnum, double f)
{
	MSG_WriteDouble (PRBI_MsgWriteDest (dest, entnum), f);
}

/* pr_ext.c:2602. COMPAT: PF_WriteInt calls MSG_WriteDouble, not an int
   writer -- `v` is `int` here so C's own implicit int-to-double conversion
   reproduces that bug exactly, the same as the original `MSG_WriteDouble
   (WriteDest (), G_INT (OFS_PARM0))`. */
void PRBI_MsgGlue_WriteIntAsDouble (int dest, int entnum, int v)
{
	MSG_WriteDouble (PRBI_MsgWriteDest (dest, entnum), v);
}

/* pr_ext.c:2606. */
void PRBI_MsgGlue_WriteInt64 (int dest, int entnum, long long v)
{
	MSG_WriteInt64 (PRBI_MsgWriteDest (dest, entnum), v);
}

/* pr_ext.c:2610. */
void PRBI_MsgGlue_WriteUInt64 (int dest, int entnum, unsigned long long v)
{
	MSG_WriteUInt64 (PRBI_MsgWriteDest (dest, entnum), v);
}

/* pr_ext.c:2590. strlen runs here so it sees exactly the bytes `string`
   points at, matching `SZ_Write (WriteDest (), string, strlen (string))`. */
void PRBI_MsgGlue_WriteString2 (int dest, int entnum, const char *string)
{
	SZ_Write (PRBI_MsgWriteDest (dest, entnum), string, (int)strlen (string));
}

#endif /* USE_RUST_HOST */
