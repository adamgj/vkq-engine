/* Phase 7 M5 wave 2 oracle TU: Group D, message builtins (pr_cmds_sv_msg_glue.c).
 *
 * This file has two jobs:
 *
 *  1. Implement the eleven PRBI_MsgGlue_* symbols quake-c-sys declares
 *     (rust/quake-c-sys/src/progs_builtins_sv_msg.rs) and
 *     rust/quake-capi/src/progs_builtins_sv_msg.rs calls. In the real build
 *     these come from Quake/pr_cmds_sv_msg_glue.c (compiled only under
 *     -Duse_rust_host); that file is not part of quake-ctest's C_SOURCES, so
 *     this is quake-ctest's stand-in -- every implementation below mirrors
 *     Quake/pr_cmds_sv_msg_glue.c statement for statement, substituting
 *     Host_Error for PR_RunError (pr_cmds_sv_msg_glue.c itself calls
 *     PR_RunError; this harness has no interpreter frame, matching wave 1's
 *     documented convention in stubs.c's PRBI_SvGlue_* oracle comment).
 *
 *  2. Provide an INDEPENDENT C oracle -- ctest_msgref_pf_*_run() below --
 *     transcribed straight from Quake/pr_cmds.c / Quake/pr_ext.c, that does
 *     NOT call the PRBI_MsgGlue_* functions above. Comparing the Rust path
 *     (quake_rs_pf_* -> PRBI_MsgGlue_*, job 1) against this oracle is the
 *     actual differential; if the oracle called job 1's functions, the
 *     comparison would be circular. MSG_Write, SZ_Write, LOC_*, PR_GetString,
 *     PR_SetEngineString are real functions compiled elsewhere in
 *     quake-ctest's C_SOURCES (net_msg.c, loc.c, pr_edict_arena.c), reached
 *     here through c_ref_prelude.h's rename macros, so only each PF_*
 *     builtin's own control flow is hand-transcribed -- the underlying wire
 *     primitives are the genuine originals on both sides of the diff.
 *
 * HAZARD (wave 1, still applies): stubs.c #undefs several c_ref_prelude.h
 * rename macros (SV_Move, SV_LinkEdict, SV_HullForEntity,
 * SV_ClipMoveToEntity, SV_TestEntityPosition, SV_CheckBottom, SV_movestep).
 * None of those names are used in this file, but as a matter of discipline
 * every oracle call site below is spelled with an explicit prototype for
 * anything whose header this file does not include (LOC_*, q_strlcat),
 * rather than trusting an implicit declaration -- wave 1 hit MSVC inventing
 * an int-returning prototype for a qboolean function, a silent
 * nondeterministic differential failure, not a link error.
 *
 * DEVIATIONS (documented in the M5 wave-2 Group D report):
 *  - client_t (c_ref_prelude.h:1153-1161) has no `.spawned` field (only
 *    `.active`/`.knowntoqc`, added by Phase 7 M2/M4). SV_BroadcastPrintf's
 *    real loop (host.c:552, `active && spawned`) is reproduced here checking
 *    `.active` alone. No test in this suite depends on the active/spawned
 *    distinction.
 *  - PF_VarString's `s > 255` dev_overflows/realtime rate-limited
 *    Con_DWarning diagnostic (pr_cmds.c:142-148) is not reproduced:
 *    dev_overflows and realtime are host.c-local statics with no
 *    c_ref_prelude.h slice. This is a cosmetic diagnostic only, never
 *    dispatch/control flow; no test constructs a >255-character result.
 *  - ctest_server_stub_t (c_ref_prelude.h:737-745) has no
 *    reliable_datagram/signon/multicast/datagram sizebuf_t fields (server.h
 *    is not included by the prelude). MSG_ALL/MSG_INIT/MSG_EXT_MULTICAST/
 *    MSG_EXT_ENTITY/MSG_BROADCAST resolve to private static sizebuf_t
 *    fixtures owned by this file instead of `sv.*`, mirroring
 *    pr_cmds_sv_msg_glue.c's own PRBI_MsgWriteDest exactly except for that
 *    substitution. MSG_ONE resolves to the real `svs.clients[n-1].message`,
 *    which the stub struct does carry.
 */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>

/* server.h:308-313. server.h itself is not part of the c_ref_prelude.h slice
   (c_ref_prelude.h:1141-1152's comment), so these are transcribed by hand,
   matching the prelude's existing convention for the movetype/solid
   enumerators it also transcribes (c_ref_prelude.h:877). */
#define MSG_BROADCAST	   0
#define MSG_ONE			   1
#define MSG_ALL			   2
#define MSG_INIT		   3
#define MSG_EXT_MULTICAST  4
#define MSG_EXT_ENTITY	   5

/* q_strlcat (strl_fn.h:8) and loc.c's LOC_* (loc.h) are real, compiled
   functions this file does not otherwise get a prototype for (their headers
   are not included here) -- declared explicitly so the c_ref_prelude.h
   rename macros (q_strlcat -> c_ref_q_strlcat, LOC_GetString ->
   c_ref_LOC_GetString, etc, already active from the force-included prelude)
   apply to a correct signature instead of an MSVC-invented implicit one. */
extern size_t	   q_strlcat (char *dst, const char *src, size_t size);
extern const char *LOC_GetString (const char *key);
extern qboolean	   LOC_HasPlaceholders (const char *s);
extern size_t	   LOC_Format (const char *format, const char *(*getarg_fn) (int, void *), void *userdata, char *out, size_t len);

/* Host_Guard (stubs.c) is not declared by any header the prelude includes --
   the real engine declares it via host.h/quakedef.h, neither of which this
   TU includes -- so it is declared explicitly here too. Host_Error is
   already declared by c_ref_prelude.h:549. */
extern int Host_Guard (void (*fn) (void *), void *arg);

/* ---------------------------------------------------------------------------
 * Fixture state: private client_t array bound to `svs.clients`, and private
 * sizebuf_t destinations for the four non-MSG_ONE WriteDest cases (see the
 * DEVIATIONS note above).
 */

#define CTEST_MSGREF_MAX_CLIENTS 4

static client_t ctest_msgref_clients[CTEST_MSGREF_MAX_CLIENTS];
static byte		 ctest_msgref_client_buf[CTEST_MSGREF_MAX_CLIENTS][MAX_MSGLEN];

static sizebuf_t ctest_msgref_reliable_datagram; /* MSG_ALL  -- sv.reliable_datagram */
static sizebuf_t ctest_msgref_signon;			  /* MSG_INIT -- sv.signon */
static sizebuf_t ctest_msgref_multicast;		  /* MSG_EXT_MULTICAST/_ENTITY -- sv.multicast */
static sizebuf_t ctest_msgref_datagram;		  /* MSG_BROADCAST -- sv.datagram */
static byte		  ctest_msgref_reliable_datagram_buf[MAX_MSGLEN];
static byte		  ctest_msgref_signon_buf[MAX_MSGLEN];
static byte		  ctest_msgref_multicast_buf[MAX_MSGLEN];
static byte		  ctest_msgref_datagram_buf[MAX_MSGLEN];

static void ctest_msgref_sz_init (sizebuf_t *sz, byte *buf, int size)
{
	memset (sz, 0, sizeof (*sz));
	sz->data = buf;
	sz->maxsize = size;
	sz->allowoverflow = true;
}

void ctest_msgref_reset (void)
{
	int i;
	for (i = 0; i < CTEST_MSGREF_MAX_CLIENTS; i++)
	{
		memset (&ctest_msgref_clients[i], 0, sizeof (client_t));
		ctest_msgref_sz_init (&ctest_msgref_clients[i].message, ctest_msgref_client_buf[i], MAX_MSGLEN);
	}
	ctest_msgref_sz_init (&ctest_msgref_reliable_datagram, ctest_msgref_reliable_datagram_buf, MAX_MSGLEN);
	ctest_msgref_sz_init (&ctest_msgref_signon, ctest_msgref_signon_buf, MAX_MSGLEN);
	ctest_msgref_sz_init (&ctest_msgref_multicast, ctest_msgref_multicast_buf, MAX_MSGLEN);
	ctest_msgref_sz_init (&ctest_msgref_datagram, ctest_msgref_datagram_buf, MAX_MSGLEN);

	svs.clients = ctest_msgref_clients;
	svs.maxclients = 0;
	host_client = NULL;
}

void ctest_msgref_set_maxclients (int n)
{
	svs.maxclients = n;
}

void ctest_msgref_set_client_active (int idx1based, int active)
{
	ctest_msgref_clients[idx1based - 1].active = active ? true : false;
}

void ctest_msgref_set_argc (int n)
{
	qcvm->argc = n;
}

/* pr_cmds.c:1640 WriteDest's MSG_ONE case reads pr_global_struct->msg_entity
   (a QC global, EDICT_TO_PROG-encoded), not an OFS_PARM slot. */
void ctest_msgref_set_msg_entity (int edict_prog_num)
{
	pr_global_struct->msg_entity = edict_prog_num;
}

/* PR_SetEngineString (progs.h) is real here -- pr_edict_arena.c is compiled
   into quake-ctest and c_ref_prelude.h renames it like PR_GetString. */
int ctest_msgref_intern_string (const char *s)
{
	return PR_SetEngineString (s);
}

/* Shared dest resolution, mirroring pr_cmds_sv_msg_glue.c's private
   PRBI_MsgWriteDest exactly except for the sv.*-vs-private-fixture
   substitution documented above. Used by both the glue functions (job 1) and
   the fixture accessors below; NOT used by the independent oracle (job 2),
   which re-derives it from pr_cmds.c's WriteDest() body instead. */
static sizebuf_t *PRBI_MsgWriteDest (int dest, int entnum)
{
	switch (dest)
	{
	case MSG_ONE:
		return &svs.clients[entnum - 1].message;
	case MSG_ALL:
		return &ctest_msgref_reliable_datagram;
	case MSG_INIT:
		return &ctest_msgref_signon;
	case MSG_EXT_MULTICAST:
	case MSG_EXT_ENTITY:
		return &ctest_msgref_multicast;
	default: /* MSG_BROADCAST */
		return &ctest_msgref_datagram;
	}
}

int ctest_msgref_client_len (int idx1based)
{
	return ctest_msgref_clients[idx1based - 1].message.cursize;
}

int ctest_msgref_client_byte (int idx1based, int off)
{
	return ctest_msgref_clients[idx1based - 1].message.data[off];
}

/* entnum is only consulted for dest == MSG_ONE; pass 0 for the other four. */
int ctest_msgref_dest_len (int dest, int entnum)
{
	return PRBI_MsgWriteDest (dest, entnum)->cursize;
}

int ctest_msgref_dest_byte (int dest, int entnum, int off)
{
	return PRBI_MsgWriteDest (dest, entnum)->data[off];
}

/* ---------------------------------------------------------------------------
 * Job 1: PRBI_MsgGlue_* -- mirrors Quake/pr_cmds_sv_msg_glue.c.
 */

/* ---- guarded seams (ADR-009 rule 3) ---- */

typedef struct
{
	int entnum;
} prbi_msg_client_check_arg_t;

/* pr_cmds.c:938. */
static void PRBI_MsgInvokeStuffcmdClientCheck (void *p)
{
	prbi_msg_client_check_arg_t *a = (prbi_msg_client_check_arg_t *)p;
	if (a->entnum < 1 || a->entnum > svs.maxclients)
		Host_Error ("Parm 0 not a client");
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

/* progs.h:174 G_STRING (o) = PR_GetString (*(string_t *)&qcvm->globals[o]). */
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

/* pr_cmds.c:102-109 PF_GetStringArg, used by PF_VarString's LOC_Format path. */
static const char *ctest_msgref_getstringarg (int idx, void *userdata)
{
	if (userdata)
		idx += *(int *)userdata;
	if (idx < 0 || idx >= qcvm->argc)
		return "";
	return LOC_GetString (G_STRING (OFS_PARM0 + idx * 3));
}

/* pr_cmds.c:111-151 PF_VarString, transcribed (real production glue calls
   PF_VarString directly since pr_cmds.c is compiled there; it is not part of
   quake-ctest's C_SOURCES). See the DEVIATIONS note at the top of this file
   for the dropped `s > 255` diagnostic branch. `out` must point at a
   writable 1024-byte buffer, matching PF_VarString's own `static char
   out[1024]`. */
static void ctest_msgref_varstring_body (int first, char *out)
{
	int			i;
	const char *format;
	size_t		s = 0;

	out[0] = 0;
	if (first >= qcvm->argc)
		return;

	format = LOC_GetString (G_STRING (OFS_PARM0 + first * 3));
	if (LOC_HasPlaceholders (format))
	{
		int offset = first + 1;
		s = LOC_Format (format, ctest_msgref_getstringarg, &offset, out, 1024);
	}
	else
	{
		for (i = first; i < qcvm->argc; i++)
		{
			s = q_strlcat (out, LOC_GetString (G_STRING (OFS_PARM0 + i * 3)), 1024);
			if (s >= 1024)
			{
				Con_Warning ("PF_VarString: overflow (string truncated)\n");
				return;
			}
		}
	}
	(void)s; /* the >255 dev_overflows diagnostic is intentionally not reproduced */
}

typedef struct
{
	int	 first;
	char out[1024];
} prbi_msg_varstring_arg_t;

static void PRBI_MsgInvokeVarString (void *p)
{
	prbi_msg_varstring_arg_t *a = (prbi_msg_varstring_arg_t *)p;
	ctest_msgref_varstring_body (a->first, a->out);
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

/* ---- leaves: none of these can Host_Error / PR_RunError ----
 * host.c is not in quake-ctest's C_SOURCES, so Host_ClientCommands
 * (host.c:569-580) and SV_BroadcastPrintf (host.c:542-561) are inlined here
 * instead of called; see the file-header DEVIATIONS note for
 * BroadcastPrintf's .spawned gap. */

/* pr_cmds.c:942-945 + host.c:569-580. */
void PRBI_MsgGlue_ClientCommandsPlain (int entnum, const char *str)
{
	client_t *old = host_client;
	host_client = &svs.clients[entnum - 1];
	MSG_WriteByte (&host_client->message, svc_stufftext);
	MSG_WriteString (&host_client->message, str);
	host_client = old;
}

/* pr_cmds.c:401 + host.c:542-561. */
void PRBI_MsgGlue_BroadcastPrintfPlain (const char *str)
{
	int i;
	for (i = 0; i < svs.maxclients; i++)
	{
		if (svs.clients[i].active)
		{
			MSG_WriteByte (&svs.clients[i].message, svc_print);
			MSG_WriteString (&svs.clients[i].message, str);
		}
	}
}

/* pr_cmds.c:430-431 / :460-461. kind 0 is PF_sprint's svc_print, kind 1 is
   PF_centerprint's svc_centerprint. */
void PRBI_MsgGlue_ClientMessageWrite (int entnum, int kind, const char *str)
{
	client_t *client = &svs.clients[entnum - 1];
	MSG_WriteChar (&client->message, kind ? svc_centerprint : svc_print);
	MSG_WriteString (&client->message, str);
}

/* ---- extended message writers (pr_ext.c) ---- */

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
   writer -- reproduced verbatim, including C's own implicit int-to-double
   conversion. "WriteUInt"'s table slot (pr_ext.c:5676) shares this same
   PF_WriteInt function pointer. */
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
   points at. */
void PRBI_MsgGlue_WriteString2 (int dest, int entnum, const char *string)
{
	SZ_Write (PRBI_MsgWriteDest (dest, entnum), string, (int)strlen (string));
}

/* ---------------------------------------------------------------------------
 * Job 2: independent C oracle. Transcribed from Quake/pr_cmds.c and
 * Quake/pr_ext.c; does NOT call any PRBI_MsgGlue_* function above. Each
 * entry point matches ctest_try_host()'s `void (*)(void *)` signature so the
 * differential test can run it under the Host_Error trap.
 */

/* pr_cmds.c:931-946 PF_stuffcmd. */
void ctest_msgref_pf_stuffcmd_run (void *unused)
{
	int			entnum;
	const char *str;
	client_t   *old;

	(void)unused;
	entnum = G_EDICTNUM (OFS_PARM0);
	if (entnum < 1 || entnum > svs.maxclients)
		Host_Error ("Parm 0 not a client");
	str = G_STRING (OFS_PARM1);

	old = host_client;
	host_client = &svs.clients[entnum - 1];
	MSG_WriteByte (&host_client->message, svc_stufftext);
	MSG_WriteString (&host_client->message, str);
	host_client = old;
}

/* pr_cmds.c:396-402 PF_bprint. */
void ctest_msgref_pf_bprint_run (void *unused)
{
	char s[1024];
	int	 i;

	(void)unused;
	ctest_msgref_varstring_body (0, s);
	for (i = 0; i < svs.maxclients; i++)
	{
		if (svs.clients[i].active)
		{
			MSG_WriteByte (&svs.clients[i].message, svc_print);
			MSG_WriteString (&svs.clients[i].message, s);
		}
	}
}

/* pr_cmds.c:413-432 PF_sprint. */
void ctest_msgref_pf_sprint_run (void *unused)
{
	char	  s[1024];
	client_t *client;
	int		  entnum;

	(void)unused;
	entnum = G_EDICTNUM (OFS_PARM0);
	ctest_msgref_varstring_body (1, s);

	if (entnum < 1 || entnum > svs.maxclients)
	{
		Con_Printf ("tried to sprint to a non-client\n");
		return;
	}

	client = &svs.clients[entnum - 1];
	MSG_WriteChar (&client->message, svc_print);
	MSG_WriteString (&client->message, s);
}

/* pr_cmds.c:443-462 PF_centerprint. COMPAT: the "tried to sprint..." warning
   text is copy-pasted from PF_sprint in the original source (pr_cmds.c:454)
   -- reproduced verbatim, not a transcription typo. */
void ctest_msgref_pf_centerprint_run (void *unused)
{
	char	  s[1024];
	client_t *client;
	int		  entnum;

	(void)unused;
	entnum = G_EDICTNUM (OFS_PARM0);
	ctest_msgref_varstring_body (1, s);

	if (entnum < 1 || entnum > svs.maxclients)
	{
		Con_Printf ("tried to sprint to a non-client\n");
		return;
	}

	client = &svs.clients[entnum - 1];
	MSG_WriteChar (&client->message, svc_centerprint);
	MSG_WriteString (&client->message, s);
}

/* pr_cmds.c:1627-1662 WriteDest, independently re-derived (not sharing
   PRBI_MsgWriteDest above, see the job-2 header comment). */
static sizebuf_t *ctest_msgref_oracle_writedest (void)
{
	int		 entnum;
	int		 dest;
	edict_t *ent;

	dest = (int)G_FLOAT (OFS_PARM0);
	switch (dest)
	{
	case MSG_BROADCAST:
		return &ctest_msgref_datagram;

	case MSG_ONE:
		ent = PROG_TO_EDICT (pr_global_struct->msg_entity);
		entnum = NUM_FOR_EDICT (ent);
		if (entnum < 1 || entnum > svs.maxclients)
			Host_Error ("WriteDest: not a client");
		return &svs.clients[entnum - 1].message;

	case MSG_ALL:
		return &ctest_msgref_reliable_datagram;

	case MSG_INIT:
		return &ctest_msgref_signon;

	case MSG_EXT_MULTICAST:
	case MSG_EXT_ENTITY:
		return &ctest_msgref_multicast;

	default:
		Host_Error ("WriteDest: bad destination");
		break;
	}

	return NULL;
}

/* pr_ext.c:2592-2595 PF_WriteFloat. */
void ctest_msgref_pf_writefloat_run (void *unused)
{
	(void)unused;
	MSG_WriteFloat (ctest_msgref_oracle_writedest (), G_FLOAT (OFS_PARM0));
}

/* pr_ext.c:2596-2599 PF_WriteDouble. */
void ctest_msgref_pf_writedouble_run (void *unused)
{
	(void)unused;
	MSG_WriteDouble (ctest_msgref_oracle_writedest (), G_DOUBLE (OFS_PARM0));
}

/* pr_ext.c:2600-2603 PF_WriteInt. COMPAT: see PRBI_MsgGlue_WriteIntAsDouble
   above -- reproduced verbatim, also covers "WriteUInt"'s shared table slot. */
void ctest_msgref_pf_writeint_run (void *unused)
{
	(void)unused;
	MSG_WriteDouble (ctest_msgref_oracle_writedest (), G_INT (OFS_PARM0));
}

/* pr_ext.c:2604-2607 PF_WriteInt64. */
void ctest_msgref_pf_writeint64_run (void *unused)
{
	(void)unused;
	MSG_WriteInt64 (ctest_msgref_oracle_writedest (), G_INT64 (OFS_PARM0));
}

/* pr_ext.c:2608-2611 PF_WriteUInt64. */
void ctest_msgref_pf_writeuint64_run (void *unused)
{
	(void)unused;
	MSG_WriteUInt64 (ctest_msgref_oracle_writedest (), G_UINT64 (OFS_PARM0));
}

/* pr_ext.c:2587-2591 PF_WriteString2. G_STRING is evaluated before
   WriteDest() in the original source (two separate statements), so a raise
   from either fires in that same order here. */
void ctest_msgref_pf_writestring2_run (void *unused)
{
	const char *string;

	(void)unused;
	string = G_STRING (OFS_PARM0);
	SZ_Write (ctest_msgref_oracle_writedest (), string, (int)strlen (string));
}
