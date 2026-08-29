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
// cvar_cmd_glue.c -- the C frame around the Rust cvar/command registries.
//
// Compiled instead of cvar.c and cmd.c under -Duse_rust_cvar (Rust migration
// Phase 7 M2). Four jobs:
//
//  1. Own the C-visible data symbols the rest of the engine links against.
//     cvar.c's cvar_vars was static and moves into Rust; everything else
//     (cmd_text, cmd_source, cmd_functions, cmd_alias, cl_nopext,
//     cmd_warncmd) is read directly by other translation units -- console.c
//     walks the two registries for tab completion -- so the storage stays
//     here and Rust manipulates it through externs.
//  2. Guard every handler and callback the registries dispatch (ADR-009
//     rule 3). A command function or a cvar callback can Host_Error, and
//     that longjmp must not unwind the Rust frame that dispatched it.
//  3. Re-raise, from a pure C frame, what those guards caught: the
//     raise-capable public entry points are defined here as thin wrappers
//     over quake_rs_* status cores.
//  4. Keep the serverinfo/userinfo replication in C. It reads svs/cls and
//     client_t, which have no ADR-011 mirrors yet; M6/M7 port it once those
//     land (task plan T1.5).

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_CVAR

/* ---------------------------------------------------------------------------
 * C-visible registry data (cmd.c:27-29, 42, 70, 504, 508).
 */

cvar_t cl_nopext = {"cl_nopext", "0", CVAR_NONE};	 // Spike -- prevent autodetection of protocol extensions, so that servers fall back to only their base
													 // protocol (without needing to reconfigure the server. Requires reconnect.
cvar_t cmd_warncmd = {"cl_warncmd", "1", CVAR_NONE}; // Spike -- prevent autodetection of protocol extensions, so that servers fall back to only their base
													 // protocol (without needing to reconfigure the server. Requires reconnect.

#define MAX_ALIAS_NAME 32

/* cmd.c keeps this private and console.c hand-copies it; do the same rather
   than widening cmd.h's contract for the port. */
typedef struct cmdalias_s
{
	struct cmdalias_s *next;
	char			   name[MAX_ALIAS_NAME];
	char			  *value;
} cmdalias_t;

sizebuf_t		cmd_text;
cmd_source_t	cmd_source;
cmd_function_t *cmd_functions; // possible commands to execute
cmdalias_t	   *cmd_alias;

/* ---------------------------------------------------------------------------
 * ADR-009 rule 3: guarded dispatch of everything the registries can call.
 */

typedef struct
{
	xcommand_t fn;
} cvarcmd_xcommand_arg_t;

static void CvarCmd_InvokeXCommand (void *p)
{
	((cvarcmd_xcommand_arg_t *)p)->fn ();
}

int CvarCmd_Glue_CallXCommand (xcommand_t fn)
{
	cvarcmd_xcommand_arg_t arg;
	arg.fn = fn;
	return Host_Guard (CvarCmd_InvokeXCommand, &arg);
}

typedef struct
{
	cvarcallback_t cb;
	cvar_t		  *var;
} cvarcmd_callback_arg_t;

static void CvarCmd_InvokeCvarCallback (void *p)
{
	cvarcmd_callback_arg_t *a = (cvarcmd_callback_arg_t *)p;
	a->cb (a->var);
}

int CvarCmd_Glue_CallCvarCallback (cvarcallback_t cb, cvar_t *var)
{
	cvarcmd_callback_arg_t arg;
	arg.cb = cb;
	arg.var = var;
	return Host_Guard (CvarCmd_InvokeCvarCallback, &arg);
}

static void CvarCmd_InvokeAutoCvarChanged (void *p)
{
	PR_AutoCvarChanged ((cvar_t *)p);
}

int CvarCmd_Glue_AutoCvarChanged (cvar_t *var)
{
	return Host_Guard (CvarCmd_InvokeAutoCvarChanged, var);
}

/* ---------------------------------------------------------------------------
 * Guarded sizebuf writes. SZ_GetSpace Host_Errors on overflow, so the two
 * cbuf/forward paths that can actually reach a full buffer go through here.
 */

typedef struct
{
	sizebuf_t  *buf;
	const void *data;
	int			length;
} cvarcmd_szwrite_arg_t;

static void CvarCmd_InvokeSzWrite (void *p)
{
	cvarcmd_szwrite_arg_t *a = (cvarcmd_szwrite_arg_t *)p;
	SZ_Write (a->buf, a->data, a->length);
}

int CvarCmd_Glue_SzWrite (sizebuf_t *buf, const void *data, int length)
{
	cvarcmd_szwrite_arg_t arg;
	arg.buf = buf;
	arg.data = data;
	arg.length = length;
	return Host_Guard (CvarCmd_InvokeSzWrite, &arg);
}

/* ---------------------------------------------------------------------------
 * Accessors for engine state the port must not mirror yet (ADR-011).
 */

const char *CvarCmd_Glue_HostClientName (void)
{
	return host_client ? host_client->name : "";
}

qboolean CvarCmd_Glue_ClsConnected (void)
{
	return cls.state == ca_connected;
}

qboolean CvarCmd_Glue_ClsDemoPlayback (void)
{
	return cls.demoplayback;
}

static void CvarCmd_InvokeForwardBegin (void *p)
{
	(void)p;
	MSG_WriteByte (&cls.message, clc_stringcmd);
}

int CvarCmd_Glue_ForwardBegin (void)
{
	return Host_Guard (CvarCmd_InvokeForwardBegin, NULL);
}

static void CvarCmd_InvokeForwardPrint (void *p)
{
	SZ_Print (&cls.message, (const char *)p);
}

int CvarCmd_Glue_ForwardPrint (const char *s)
{
	return Host_Guard (CvarCmd_InvokeForwardPrint, (void *)s);
}

/* protocol.h numbers, handed over rather than re-spelled in Rust */
void CvarCmd_Glue_Protocols (int *rmq, int *fitzquake, int *netquake)
{
	*rmq = PROTOCOL_RMQ;
	*fitzquake = PROTOCOL_FITZQUAKE;
	*netquake = PROTOCOL_NETQUAKE;
}

void CvarCmd_Glue_PextNumbers (unsigned int *pext1, unsigned int *pext1_client, unsigned int *pext2, unsigned int *pext2_client)
{
	*pext1 = PROTOCOL_FTE_PEXT1;
	*pext1_client = PEXT1_SUPPORTED_CLIENT;
	*pext2 = PROTOCOL_FTE_PEXT2;
	*pext2_client = PEXT2_SUPPORTED_CLIENT;
}

/* ---------------------------------------------------------------------------
 * CVAR_SERVERINFO / CVAR_USERINFO replication (cvar.c:508-538).
 *
 * Kept in C for M2: it reads svs.serverinfo/svs.clients/svs.maxclients,
 * cls.userinfo/cls.message/cls.state and client_t.active/.message, none of
 * which have ADR-011 mirrors yet. M6/M7 port it once those land (T1.5).
 * The MSG_ writes can Host_Error, so both blocks are guarded and the caught
 * jump travels back through Rust as a status.
 */

static void CvarCmd_InvokeServerinfoChanged (void *p)
{
	cvar_t *var = (cvar_t *)p;

	// replicate the cvar change into the serverinfo string and let clients know.
	Info_SetKey (svs.serverinfo, sizeof (svs.serverinfo), var->name, var->string);

	for (client_t *current_client = svs.clients; current_client < svs.clients + svs.maxclients; current_client++)
	{
		if (current_client->active)
		{
			MSG_WriteByte (&current_client->message, svc_stufftext);
			MSG_WriteString (&current_client->message, va ("%s \"%s\" \"%s\"\n", "//svi", var->name, var->string));
		}
	}
}

int CvarCmd_Glue_ServerinfoChanged (cvar_t *var)
{
	return Host_Guard (CvarCmd_InvokeServerinfoChanged, var);
}

static void CvarCmd_InvokeUserinfoChanged (void *p)
{
	cvar_t *var = (cvar_t *)p;

	// replicate the cvar change into the userinfo.
	Info_SetKey (cls.userinfo, sizeof (cls.userinfo), var->name, var->string);

	// let the server know.
	if (cls.state == ca_connected)
	{
		MSG_WriteByte (&cls.message, clc_stringcmd);
		if (var == &cl_name) // some hacks for legacy settings.
			MSG_WriteString (&cls.message, va ("name \"%s\"\n", var->string));
		else if (var == &cl_topcolor || var == &cl_bottomcolor)
			MSG_WriteString (&cls.message, va ("color \"%s\" \"%s\"\n", cl_topcolor.string, cl_bottomcolor.string));
		else
			MSG_WriteString (&cls.message, va ("setinfo \"%s\" \"%s\"\n", var->name, var->string));
	}
}

int CvarCmd_Glue_UserinfoChanged (cvar_t *var)
{
	return Host_Guard (CvarCmd_InvokeUserinfoChanged, var);
}

/* ---------------------------------------------------------------------------
 * The raise-capable public ABI. Each wrapper calls a Rust status core and
 * re-issues the caught jump from this pure C frame (ADR-009 rule 2).
 */

void Cbuf_Execute (void)
{
	Host_Reraise (quake_rs_cbuf_execute ());
}

/* Cbuf_InsertText and Cmd_ForwardToServer are listed as direct Rust exports
   in the M2 contract, but both reach SZ_GetSpace, whose overflow path is a
   Host_Error. They are wrapped here instead so the jump is re-issued from C
   (ADR-009); the C ABI is unchanged, so Cmd_ForwardToServer still registers
   as the "cmd" xcommand_t. */
void Cbuf_InsertText (const char *text)
{
	int raised = 0;
	quake_rs_cbuf_insert_text (text, &raised);
	Host_Reraise (raised);
}

void Cmd_ForwardToServer (void)
{
	Host_Reraise (quake_rs_cmd_forward_to_server ());
}

qboolean Cmd_ExecuteString (const char *text, cmd_source_t src)
{
	int		 raised = 0;
	qboolean result = quake_rs_cmd_execute_string (text, src, &raised);
	Host_Reraise (raised);
	return result;
}

void Cvar_RegisterVariable (cvar_t *variable)
{
	int raised = 0;
	quake_rs_cvar_register_variable (variable, &raised);
	Host_Reraise (raised);
}

void Cvar_SetQuick (cvar_t *var, const char *value)
{
	int raised = 0;
	quake_rs_cvar_set_quick (var, value, &raised);
	Host_Reraise (raised);
}

void Cvar_SetValueQuick (cvar_t *var, const float value)
{
	int raised = 0;
	quake_rs_cvar_set_value_quick (var, value, &raised);
	Host_Reraise (raised);
}

void Cvar_Set (const char *var_name, const char *value)
{
	int raised = 0;
	quake_rs_cvar_set (var_name, value, &raised);
	Host_Reraise (raised);
}

void Cvar_SetValue (const char *var_name, const float value)
{
	int raised = 0;
	quake_rs_cvar_set_value (var_name, value, &raised);
	Host_Reraise (raised);
}

void Cvar_SetROM (const char *var_name, const char *value)
{
	int raised = 0;
	quake_rs_cvar_set_rom (var_name, value, &raised);
	Host_Reraise (raised);
}

void Cvar_SetValueROM (const char *var_name, const float value)
{
	int raised = 0;
	quake_rs_cvar_set_value_rom (var_name, value, &raised);
	Host_Reraise (raised);
}

cvar_t *Cvar_Create (const char *name, const char *value)
{
	int		raised = 0;
	cvar_t *result = quake_rs_cvar_create (name, value, &raised);
	Host_Reraise (raised);
	return result;
}

qboolean Cvar_Command (void)
{
	int		 raised = 0;
	qboolean result = quake_rs_cvar_command (&raised);
	Host_Reraise (raised);
	return result;
}

/* cvar.c declares this without a header (no external callers today); the
   symbol is kept so the C and Rust builds expose the same surface. */
void Cvar_Reset (const char *name);
void Cvar_Reset (const char *name)
{
	int raised = 0;
	quake_rs_cvar_reset (name, &raised);
	Host_Reraise (raised);
}

#endif /* USE_RUST_CVAR */
