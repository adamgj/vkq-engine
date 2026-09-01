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
// sv_user_glue.c -- the C frame around the Rust server user-movement port.
//
// Compiled instead of sv_user.c under -Duse_rust_host (Rust migration Phase 7
// M6, T6.4). Four jobs, mirroring sv_phys_glue.c:
//
//  1. Own the C-visible objects sv_user.c defined: sv_player (sv_user.c:26)
//     and the five cvars (:29, :43, :44, :198, :199). sv_main.c reads
//     sv_player and reaches the cvars via block-scope `extern cvar_t`, so the
//     storage stays here and Rust reaches them through externs.
//  2. Guard everything sv_user.c reached that can Host_Error / Host_EndGame
//     (ADR-009 rule 3): the clc_stringcmd case body (both its QC-dispatch and
//     Cmd_ExecuteString branches) and SV_DropClient (a confirmed transitive
//     raise site via ClientDisconnect QC).
//  3. Re-raise, from a pure C frame, what those guards caught. The three
//     sv_user.c entry points whose bodies transitively reach Host_Error are
//     thin wrappers over quake_rs_* status cores: SV_SetIdealPitch,
//     SV_ClientThink and SV_RunClients. Host_Reraise is called only from
//     here.
//  4. Keep the MSG_Read*/key_dest/V_CalcRoll/SVFTE_Ack/net accessors reached
//     as ordinary C externs; none of those can raise, so no guard is needed
//     for them.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

/* ---------------------------------------------------------------------------
 * C-visible objects (sv_user.c:26, :29, :43, :44, :198, :199).
 */

edict_t *sv_player;

extern cvar_t sv_friction;
cvar_t		  sv_edgefriction = {"edgefriction", "2", CVAR_NONE}; // COMPAT: name string is "edgefriction", not "sv_edgefriction"
extern cvar_t sv_stopspeed;

cvar_t sv_idealpitchscale = {"sv_idealpitchscale", "0.8", CVAR_NONE};
cvar_t sv_altnoclip = {"sv_altnoclip", "1", CVAR_ARCHIVE}; // johnfitz

cvar_t sv_maxspeed = {"sv_maxspeed", "320", CVAR_NOTIFY | CVAR_SERVERINFO};
cvar_t sv_accelerate = {"sv_accelerate", "10", CVAR_NONE};

/* ---------------------------------------------------------------------------
 * Guarded callbacks (ADR-009 rule 3).
 */

/* sv_user.c:577-592 -- the whole clc_stringcmd case body, minus the
   MSG_ReadString call the Rust side already made (it cannot raise). Both the
   QC-dispatch branch (PR_ExecuteProgram) and the Cmd_ExecuteString branch
   stay C-to-C inside this one guarded frame. */
static void SvUser_InvokeStringCmd (void *p)
{
	const char *s = (const char *)p;

	if (q_strncasecmp (s, "spawn", 5) && q_strncasecmp (s, "begin", 5) && q_strncasecmp (s, "prespawn", 8) && qcvm->extfuncs.SV_ParseClientCommand)
	{ // the spawn/begin/prespawn are because of numerous mods that disobey the rules.
		// at a minimum, we must be able to join the server, so that we can see any sprints/bprints (because dprint sucks, yes there's proper ways
		// to deal with this, but moders don't always know them).
		client_t *ohc = host_client;
		G_INT (OFS_PARM0) = PR_SetEngineString (s);
		pr_global_struct->time = qcvm->time;
		pr_global_struct->self = EDICT_TO_PROG (host_client->edict);
		PR_ExecuteProgram (qcvm->extfuncs.SV_ParseClientCommand);
		host_client = ohc;
	}
	else
		Cmd_ExecuteString (s, src_client);
}

int SvUser_Glue_StringCmd (const char *s)
{
	return Host_Guard (SvUser_InvokeStringCmd, (void *)s);
}

/* sv_user.c:653 -- SV_DropClient (crash), called from SV_RunClients. A
   confirmed transitive ADR-009 raise site via host.c:590's PR_ExecuteProgram
   (pr_global_struct->ClientDisconnect). */
static void SvUser_InvokeDropClient (void *p)
{
	qboolean crash = *(qboolean *)p;
	SV_DropClient (crash);
}

int SvUser_Glue_DropClient (qboolean crash)
{
	return Host_Guard (SvUser_InvokeDropClient, &crash);
}

/* ---------------------------------------------------------------------------
 * Re-raising public entry points. Each is the exact sv_user.c signature; the
 * Rust body is a quake_rs_* status core and the jump is re-issued from here,
 * never from a Rust frame (ADR-009).
 */

/* sv_user.c:52 */
void SV_SetIdealPitch (void)
{
	int r = quake_rs_sv_set_ideal_pitch ();
	Host_Reraise (r);
}

/* sv_user.c:417 */
void SV_ClientThink (void)
{
	int r = quake_rs_sv_client_think ();
	Host_Reraise (r);
}

/* sv_user.c:618 */
void SV_RunClients (void)
{
	int r = quake_rs_sv_run_clients ();
	Host_Reraise (r);
}

#endif /* USE_RUST_HOST */
