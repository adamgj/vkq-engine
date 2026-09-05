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
// cl_input_glue.c -- the C frame around the Rust client input port.
//
// Compiled instead of cl_input.c under -Duse_rust_host (Rust migration Phase 7
// M7, T7.2), mirroring sv_user_glue.c:
//
//  1. Own the C-visible objects cl_input.c defined: the seventeen kbutton_t
//     input states and in_impulse (cl_input.c:53-58), and the nine movement
//     cvars (:323-335). in_sdl.c reads in_speed/in_strafe/in_mlook and four of
//     the cvars, menu.c and view.c read three more, and cl_main.c:1376-1390 is
//     what registers all nine -- so the storage stays here and Rust reaches it
//     through externs.
//  2. Guard what cl_input.c reached that can Host_Error / Host_EndGame
//     (ADR-009 rule 3): every MSG_Write* CL_SendMove makes (each reaches
//     SZ_GetSpace, which Host_Errors when the sizebuf disallows overflow) and
//     CL_Disconnect (transitively Host_ShutdownServer -> SV_DropClient ->
//     ClientDisconnect QC, plus its own MSG_WriteByte).
//  3. Re-raise, from a pure C frame, what those guards caught. CL_SendMove is
//     the only cl_input.c entry point whose body can raise; it is a thin
//     wrapper over quake_rs_cl_send_move and Host_Reraise is called only here.
//  4. Leave everything else plain. Cmd_Argv, Cmd_AddCommand2, Con_Printf,
//     anglemod, V_StartPitchDrift/V_StopPitchDrift and
//     NET_SendUnreliableMessage cannot longjmp (the net drivers' only failure
//     path is Sys_Error, which terminates rather than jumping), so the Rust
//     side calls them directly.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

/* ---------------------------------------------------------------------------
 * C-visible objects (cl_input.c:53-58, :323-335).
 */

kbutton_t in_mlook = {.state = 1}, in_klook;
kbutton_t in_left, in_right, in_forward, in_back;
kbutton_t in_lookup, in_lookdown, in_moveleft, in_moveright;
kbutton_t in_strafe, in_speed, in_use, in_jump, in_attack;
kbutton_t in_up, in_down;

int in_impulse;

cvar_t cl_upspeed = {"cl_upspeed", "200", CVAR_NONE};
cvar_t cl_forwardspeed = {"cl_forwardspeed", "200", CVAR_ARCHIVE};
cvar_t cl_backspeed = {"cl_backspeed", "200", CVAR_ARCHIVE};
cvar_t cl_sidespeed = {"cl_sidespeed", "350", CVAR_NONE};

cvar_t cl_movespeedkey = {"cl_movespeedkey", "2.0", CVAR_NONE};

cvar_t cl_yawspeed = {"cl_yawspeed", "140", CVAR_NONE};
cvar_t cl_pitchspeed = {"cl_pitchspeed", "150", CVAR_NONE};

cvar_t cl_anglespeedkey = {"cl_anglespeedkey", "1.5", CVAR_NONE};

cvar_t cl_alwaysrun = {"cl_alwaysrun", "1", CVAR_ARCHIVE}; // QuakeSpasm -- new always run

/* ---------------------------------------------------------------------------
 * Guarded callbacks (ADR-009 rule 3).
 */

/* Every MSG_Write* reaches SZ_GetSpace (net_msg.c:479), which Host_Errors when
   the sizebuf disallows overflow, so no such call may be made straight from
   Rust. CL_SendMove reads buf.cursize once in the middle (the `dump` rollback
   point at cl_input.c:497), so the Rust side buffers a run of writes and
   flushes it before that read; see the Writer in quake-capi/src/cl_input.rs.

   op->u carries the protocol flag word per op rather than being read from
   cl.protocolflags here, so the batch stays a pure replay of what the Rust
   side decided. */
typedef struct
{
	int		 kind;
	int		 i;
	float	 f;
	unsigned u;
} clinput_write_t;

typedef struct
{
	sizebuf_t			  *sb;
	const clinput_write_t *ops;
	int					   count;
} clinput_writebatch_arg_t;

static void ClInput_InvokeWriteBatch (void *p)
{
	clinput_writebatch_arg_t *a = (clinput_writebatch_arg_t *)p;
	int						  k;

	for (k = 0; k < a->count; k++)
	{
		const clinput_write_t *op = &a->ops[k];
		switch (op->kind)
		{
		case 0:
			MSG_WriteByte (a->sb, op->i);
			break;
		case 1:
			MSG_WriteShort (a->sb, op->i);
			break;
		case 2:
			MSG_WriteLong (a->sb, op->i);
			break;
		case 3:
			MSG_WriteFloat (a->sb, op->f);
			break;
		case 4:
			MSG_WriteAngle (a->sb, op->f, op->u);
			break;
		case 5:
			MSG_WriteAngle16 (a->sb, op->f, op->u);
			break;
		default:
			Sys_Error ("ClInput_Glue_WriteBatch: bad op %i", op->kind);
		}
	}
}

int ClInput_Glue_WriteBatch (sizebuf_t *sb, const clinput_write_t *ops, int count)
{
	clinput_writebatch_arg_t arg;
	arg.sb = sb;
	arg.ops = ops;
	arg.count = count;
	return Host_Guard (ClInput_InvokeWriteBatch, &arg);
}

/* cl_input.c:544 -- CL_Disconnect (), on a lost server connection. A raise site
   twice over: its own MSG_WriteByte (cl_main.c:391) and, when a local server is
   running, Host_ShutdownServer -> SV_DropClient -> ClientDisconnect QC. */
static void ClInput_InvokeDisconnect (void *p)
{
	(void)p;
	CL_Disconnect ();
}

int ClInput_Glue_Disconnect (void)
{
	return Host_Guard (ClInput_InvokeDisconnect, NULL);
}

/* ---------------------------------------------------------------------------
 * Re-raising public entry point. The Rust body is a quake_rs_* status core and
 * the jump is re-issued from here, never from a Rust frame (ADR-009).
 */

/* cl_input.c:476 */
void CL_SendMove (const usercmd_t *cmd)
{
	int r = quake_rs_cl_send_move (cmd);
	Host_Reraise (r);
}

#endif /* USE_RUST_HOST */
