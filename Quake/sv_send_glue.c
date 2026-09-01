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
// sv_send_glue.c -- the C frame around the Rust FTE delta writer port.
//
// Compiled instead of sv_send.c under -Duse_rust_host (Rust migration Phase 7
// M6, T6.3). Four jobs, mirroring sv_main_glue.c:
//
//  1. Own the C-visible objects sv_send.c defined -- of which there are none.
//     sv_send.c's only file-scope declarations are the two extern cvar_t
//     references at sv_send.c:27-28 (sv_netsort, sv_smoothplatformlerps);
//     both objects are defined by sv_main_glue.c:67-68. Every other file-scope
//     object in sv_send.c had internal linkage (the snapshot buffers at :420,
//     the fat-PVS state at :1038, the net sort arrays at :1132), so those
//     became Rust-owned statics instead.
//  2. Guard everything sv_send.c reached that can Host_Error / Host_EndGame
//     (ADR-009 rule 3): every MSG_Write* and SZ_Write run (batched -- see
//     SvSend_Glue_WriteBatch), PR_GetString, SV_DropClient (transitively, via
//     host.c:590's PR_ExecuteProgram of ClientDisconnect), Cmd_ExecuteString,
//     and -- because under -Duse_rust_host the plain name is itself a
//     Host_Reraise wrapper -- SV_SetIdealPitch.
//  3. Re-raise, from a pure C frame, what those guards caught. The ten
//     sv_send.c entry points another translation unit references are thin
//     wrappers over quake_rs_svsend_* status cores; Host_Reraise is called
//     only from here. The other sixteen non-static sv_send.c functions had no
//     caller outside sv_send.c (checked across Quake/*.c and Quake/*.h), so
//     they are private to the Rust module and get no C symbol.
//  4. Keep the compile-time-conditional and renamed fragments in C: the
//     LERP_BANDAID strip test (which reads the client-side cls from inside the
//     server writer), the three Con_* format strings, the Sys_Error format
//     string, and the small accessors -- SZ_Clear, AngleVectors,
//     standard_quake, SV_ModelIndex, sv_player -- whose names the ctest oracle
//     prelude rewrites, so that both sides of the differential reach one
//     implementation.
//
// Nothing here guards the Sys_Error at sv_send.c:1096: Sys_Error terminates
// rather than longjmping, so SvSend_Glue_FatPvsAllocFailed is a plain noreturn
// shim and not a Host_Guard site.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

/* ---------------------------------------------------------------------------
 * Guarded callbacks (ADR-009 rule 3).
 */

/* Every MSG_Write* and SZ_Write reaches SZ_GetSpace (net_msg.c:479), which
   Host_Errors when the sizebuf disallows overflow. sv_send.c reads msg->cursize
   between writes (the rollback points at :795 and :1443, the packet budgets at
   :1806 and :1953), so the Rust side buffers a run of writes and flushes it
   before every such read; see the Writer in quake-capi/src/sv_send.rs.

   op->u carries the protocol flag word per op rather than being read from
   sv.protocolflags here (as SvMain_Glue_WriteBatch does), because
   MSGFTE_WriteEntityUpdate takes protocolflags and pext2 as parameters
   (sv_send.c:239) and MSG_WriteStaticOrBaseLine is called with a
   caller-supplied pair (sv_send.c:963). */
typedef struct
{
	int			kind;
	int			i;
	float		f;
	unsigned	u;
	const void *p;
} svsend_write_t;

typedef struct
{
	sizebuf_t			 *sb;
	const svsend_write_t *ops;
	int					  count;
} svsend_writebatch_arg_t;

static void SvSend_InvokeWriteBatch (void *p)
{
	svsend_writebatch_arg_t *a = (svsend_writebatch_arg_t *)p;
	int						 k;

	for (k = 0; k < a->count; k++)
	{
		const svsend_write_t *op = &a->ops[k];
		switch (op->kind)
		{
		case 0:
			MSG_WriteByte (a->sb, op->i);
			break;
		case 1:
			MSG_WriteChar (a->sb, op->i);
			break;
		case 2:
			MSG_WriteShort (a->sb, op->i);
			break;
		case 3:
			MSG_WriteLong (a->sb, op->i);
			break;
		case 4:
			MSG_WriteFloat (a->sb, op->f);
			break;
		case 5:
			MSG_WriteString (a->sb, (const char *)op->p);
			break;
		case 6:
			MSG_WriteCoord (a->sb, op->f, op->u);
			break;
		case 7:
			MSG_WriteAngle (a->sb, op->f, op->u);
			break;
		case 8:
			MSG_WriteAngle16 (a->sb, op->f, op->u);
			break;
		case 9:
			MSG_WriteEntity (a->sb, (unsigned int)op->i, op->u);
			break;
		case 10:
			SZ_Write (a->sb, op->p, op->i);
			break;
		default:
			break;
		}
	}
}

int SvSend_Glue_WriteBatch (sizebuf_t *sb, const svsend_write_t *ops, int count)
{
	svsend_writebatch_arg_t arg;
	arg.sb = sb;
	arg.ops = ops;
	arg.count = count;
	return Host_Guard (SvSend_InvokeWriteBatch, &arg);
}

/* PR_GetString (sv_send.c:66, :132, :890, :1215, :1551, :2244) -- Host_Errors
   on a handle outside the string table. */
typedef struct
{
	int			 handle;
	const char **out;
} svsend_getstring_arg_t;

static void SvSend_InvokeGetString (void *p)
{
	svsend_getstring_arg_t *a = (svsend_getstring_arg_t *)p;
	*a->out = PR_GetString (a->handle);
}

int SvSend_Glue_GetString (int handle, const char **out)
{
	svsend_getstring_arg_t arg;
	arg.handle = handle;
	arg.out = out;
	return Host_Guard (SvSend_InvokeGetString, &arg);
}

/* sv_send.c:1538 -- SV_SetIdealPitch (), called from SV_WriteDamageToMessage.
   Under -Duse_rust_host the plain name is sv_user_glue.c's Host_Reraise
   wrapper over the T6.4 port, so it must not be called from a Rust frame. */
static void SvSend_InvokeSetIdealPitch (void *p)
{
	(void)p;
	SV_SetIdealPitch ();
}

int SvSend_Glue_SetIdealPitch (void)
{
	return Host_Guard (SvSend_InvokeSetIdealPitch, NULL);
}

/* sv_send.c:1869, :1934, :2171, :2184, :2188 -- SV_DropClient (false). A
   confirmed transitive raise site through host.c:590's PR_ExecuteProgram
   (pr_global_struct->ClientDisconnect). */
static void SvSend_InvokeDropClient (void *p)
{
	SV_DropClient (*(qboolean *)p);
}

int SvSend_Glue_DropClient (qboolean crash)
{
	return Host_Guard (SvSend_InvokeDropClient, &crash);
}

/* sv_send.c:2276 -- Cmd_ExecuteString ("reconnect\n", src_command). The command
   string and src_command stay in C; under -Duse_rust_cvar the plain name is
   itself a Host_Reraise wrapper, so the guard is required twice over. */
static void SvSend_InvokeExecuteReconnect (void *p)
{
	(void)p;
	Cmd_ExecuteString ("reconnect\n", src_command);
}

int SvSend_Glue_ExecuteReconnect (void)
{
	return Host_Guard (SvSend_InvokeExecuteReconnect, NULL);
}

/* ---------------------------------------------------------------------------
 * Non-raising shims (job 4).
 */

void SvSend_Glue_SzClear (sizebuf_t *sb)
{
	SZ_Clear (sb);
}

void SvSend_Glue_AngleVectors (const float *angles, float *forward, float *right, float *up)
{
	AngleVectors ((float *)angles, forward, right, up);
}

int SvSend_Glue_StandardQuake (void)
{
	return standard_quake ? 1 : 0;
}

int SvSend_Glue_ModelIndex (const char *name)
{
	return SV_ModelIndex (name);
}

void SvSend_Glue_SetPlayer (edict_t *ent)
{
	sv_player = ent;
}

/* sv_send.c:268-271 -- the LERP_BANDAID strip test. Kept in C because it reads
   the client-side cls from inside the server writer and sits under a
   compile-time condition. */
int SvSend_Glue_StripLerp (void)
{
#ifdef LERP_BANDAID
	return (cls.demorecording || strcmp (NET_QSocketGetTrueAddressString (host_client->netconnection), "LOCAL")) ? 1 : 0;
#else
	return 0;
#endif
}

void SvSend_Glue_WarnPacket (int cursize)
{
	Con_DWarning ("%i byte packet exceeds standard limit of 1024.\n", cursize);
}

void SvSend_Glue_WarnPacketMax (int cursize, int maxsize)
{
	Con_DWarning ("%i byte packet exceeds standard limit of 1024 (max = %d).\n", cursize, maxsize);
}

void SvSend_Glue_WarnOverflow (void)
{
	Con_Printf ("Packet overflow!\n");
}

FUNC_NORETURN void SvSend_Glue_FatPvsAllocFailed (int capacity)
{
	Sys_Error ("SV_FatPVS: realloc() failed on %d bytes", capacity);
}

/* ---------------------------------------------------------------------------
 * Re-raising entry points (ADR-009 rule 3). Only the sv_send.c functions
 * another translation unit references get a C symbol; the rest are private to
 * quake-capi/src/sv_send.rs.
 */

void SVFTE_DestroyFrames (client_t *client)
{
	Host_Reraise (quake_rs_svsend_destroy_frames (client));
}

void SVFTE_SetupFrames (client_t *client)
{
	Host_Reraise (quake_rs_svsend_setup_frames (client));
}

void SVFTE_Ack (client_t *client, int sequence)
{
	Host_Reraise (quake_rs_svsend_ack (client, sequence));
}

void SV_BuildEntityState (edict_t *ent, entity_state_t *state)
{
	Host_Reraise (quake_rs_svsend_build_entity_state (ent, state));
}

void MSG_WriteStaticOrBaseLine (
	sizebuf_t *buf, int idx, struct entity_state_s *state, unsigned int protocol_pext2, unsigned int protocol, unsigned int protocolflags)
{
	Host_Reraise (quake_rs_svsend_write_static_or_baseline (buf, idx, state, protocol_pext2, protocol, protocolflags));
}

/* sv_send.c:1089. Cannot raise -- the only failure path is the Sys_Error at
   :1096 -- so this is a plain forward, not a Host_Reraise site. It keeps a C
   symbol because r_world.c:41 declares and calls it. */
byte *SV_FatPVS (vec3_t org, qmodel_t *worldmodel)
{
	byte *out = NULL;
	quake_rs_svsend_fat_pvs (org, worldmodel, (void **)&out);
	return out;
}

void SV_WriteClientdataToMessage (client_t *client, sizebuf_t *msg)
{
	Host_Reraise (quake_rs_svsend_write_clientdata_to_message (client, msg));
}

void SV_SendClientMessages (void)
{
	Host_Reraise (quake_rs_svsend_send_client_messages ());
}

void SV_CreateBaseline (void)
{
	Host_Reraise (quake_rs_svsend_create_baseline ());
}

void SV_SendReconnect (void)
{
	Host_Reraise (quake_rs_svsend_send_reconnect ());
}

#endif // USE_RUST_HOST
