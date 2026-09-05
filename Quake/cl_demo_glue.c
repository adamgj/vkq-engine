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
// cl_demo_glue.c -- the C frame around the Rust demo record/playback port.
//
// Compiled instead of cl_demo.c under -Duse_rust_host (Rust migration Phase 7
// M7, T7.4), mirroring cl_input_glue.c and cl_parse_glue.c:
//
//  1. Own the C-visible objects cl_demo.c defined -- of which there are none.
//     Every file-scope object in cl_demo.c had internal linkage (the shared
//     `name` buffer at cl_demo.c:37, the alt record buffer at :584), so those
//     became Rust-owned statics. All eight non-static functions are declared
//     in client.h:404-412, so this file has no header-less external to
//     re-declare locally.
//  2. Guard everything cl_demo.c reached that can Host_Error / Host_EndGame
//     (ADR-009 rule 3): every MSG_Write* run (batched -- see
//     ClDemo_Glue_WriteBatch, since each write reaches SZ_GetSpace, which
//     Host_Errors at net_msg.c:488), MSG_WriteStaticOrBaseLine,
//     Cmd_ExecuteString (:681, which runs an arbitrary command -- `map` in
//     practice), CL_Disconnect (:767) and NET_GetMessage (:302, which
//     net_main.c:29-34 documents as sitting above Host_Error-capable driver
//     code). The remaining trampolines (the seek effect group, BGM_Stop,
//     S_StopAllSounds, S_LoadSound, Fog_GetFogCommand, Sky_GetSkyCommand,
//     DemoList_Rebuild, COM_FOpenFile) wrap callees whose transitive reach was
//     not proven raise-free; guarding them is conservative and costs only the
//     setjmp.
//  3. Re-raise, from a pure C frame, what those guards caught. cl_demo.c has
//     no Host_Error / Host_EndGame of its own (its only failure exit is
//     Sys_Error at :175/:196, which terminates rather than jumping), so the
//     status a quake_rs_cl_demo_* core returns is always the Host_Guard result
//     itself and Host_Reraise is called only here.
//  4. Leave everything else plain. The stdio on cls.demofile, Con_Printf,
//     Con_SafePrintf, Con_LinkPrintf, Cmd_Argc/Cmd_Argv, va, q_strlcpy,
//     q_snprintf, COM_AddExtension, COM_SkipPath, Sys_fopen/fseek/ftell,
//     SZ_Clear and Harness_DemoEnded cannot longjmp, so the Rust side calls
//     them directly.

#include "quakedef.h"
#include "sys.h"

#include "bgmusic.h"

#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

/* ---------------------------------------------------------------------------
 * Batched, guarded net_message writers.
 *
 * Every MSG_Write* reaches SZ_GetSpace (net_msg.c:481), which Host_Errors on
 * overflow, so no Rust frame may sit under one. A run of writes is buffered on
 * the Rust side and replayed here inside a single Host_Guard; the emitted byte
 * stream is identical for any batch size because the ops replay in insertion
 * order. cl_demo.c always writes to net_message (with net_message.data
 * temporarily swapped by CL_Record_Signons, cl_demo.c:581-599), so the target
 * sizebuf is implicit.
 */

typedef struct
{
	int			kind;
	int			i;
	float		f;
	unsigned	u;
	const void *p;
} cldemo_write_t;

typedef struct
{
	const cldemo_write_t *ops;
	int					  count;
} cldemo_writebatch_arg_t;

static void ClDemo_InvokeWriteBatch (void *p)
{
	cldemo_writebatch_arg_t *a = (cldemo_writebatch_arg_t *)p;
	int						 k;

	for (k = 0; k < a->count; k++)
	{
		const cldemo_write_t *op = &a->ops[k];
		switch (op->kind)
		{
		case 0:
			MSG_WriteByte (&net_message, op->i);
			break;
		case 1:
			MSG_WriteShort (&net_message, op->i);
			break;
		case 2:
			MSG_WriteLong (&net_message, op->i);
			break;
		case 3:
			MSG_WriteFloat (&net_message, op->f);
			break;
		case 4:
			MSG_WriteString (&net_message, (const char *)op->p);
			break;
		case 5:
			MSG_WriteCoord (&net_message, op->f, op->u);
			break;
		default:
			Sys_Error ("ClDemo_InvokeWriteBatch: unknown op %i", op->kind);
			break;
		}
	}
}

int ClDemo_Glue_WriteBatch (const cldemo_write_t *ops, int count)
{
	cldemo_writebatch_arg_t arg;
	arg.ops = ops;
	arg.count = count;
	return Host_Guard (ClDemo_InvokeWriteBatch, &arg);
}

/* cl_demo.c:391, :400 */
typedef struct
{
	int			 idx;
	void		*state;
	unsigned int pext2;
	unsigned int protocol;
	unsigned int protocolflags;
} cldemo_baseline_arg_t;

static void ClDemo_InvokeStaticOrBaseLine (void *p)
{
	cldemo_baseline_arg_t *a = (cldemo_baseline_arg_t *)p;
	MSG_WriteStaticOrBaseLine (&net_message, a->idx, (struct entity_state_s *)a->state, a->pext2, a->protocol, a->protocolflags);
}

int ClDemo_Glue_WriteStaticOrBaseLine (int idx, void *state, unsigned int pext2, unsigned int protocol, unsigned int protocolflags)
{
	cldemo_baseline_arg_t arg;
	arg.idx = idx;
	arg.state = state;
	arg.pext2 = pext2;
	arg.protocol = protocol;
	arg.protocolflags = protocolflags;
	return Host_Guard (ClDemo_InvokeStaticOrBaseLine, &arg);
}

/* ---------------------------------------------------------------------------
 * The remaining guarded callees.
 */

/* cl_demo.c:302 */
typedef struct
{
	struct qsocket_s *sock;
	int				 *out;
} cldemo_netget_arg_t;

static void ClDemo_InvokeNetGetMessage (void *p)
{
	cldemo_netget_arg_t *a = (cldemo_netget_arg_t *)p;
	*a->out = NET_GetMessage (a->sock);
}

int ClDemo_Glue_NetGetMessage (struct qsocket_s *sock, int *out)
{
	cldemo_netget_arg_t arg;
	arg.sock = sock;
	arg.out = out;
	*out = 0;
	return Host_Guard (ClDemo_InvokeNetGetMessage, &arg);
}

/* cl_demo.c:681 */
typedef struct
{
	const char *text;
	int			src;
} cldemo_exec_arg_t;

static void ClDemo_InvokeCmdExecuteString (void *p)
{
	cldemo_exec_arg_t *a = (cldemo_exec_arg_t *)p;
	Cmd_ExecuteString (a->text, (cmd_source_t)a->src);
}

int ClDemo_Glue_CmdExecuteString (const char *text, int src)
{
	cldemo_exec_arg_t arg;
	arg.text = text;
	arg.src = src;
	return Host_Guard (ClDemo_InvokeCmdExecuteString, &arg);
}

/* cl_demo.c:767 */
static void ClDemo_InvokeDisconnect (void *p)
{
	(void)p;
	CL_Disconnect ();
}

int ClDemo_Glue_Disconnect (void)
{
	return Host_Guard (ClDemo_InvokeDisconnect, NULL);
}

/* cl_demo.c:260-267 -- the seek effect reset, kept as one trampoline because
 * the calls are unconditional and adjacent and nothing between them is
 * observable from Rust. */
static void ClDemo_InvokeSeekEffects (void *p)
{
	(void)p;
	V_ResetBlend ();
	Fog_NewMap ();
	Sky_NewMap ();
	R_ClearParticles ();
#ifdef PSET_SCRIPT
	PScript_ClearParticles (false);
#endif
	SCR_CenterPrintClear ();
}

int ClDemo_Glue_SeekEffects (void)
{
	return Host_Guard (ClDemo_InvokeSeekEffects, NULL);
}

/* cl_demo.c:271 */
static void ClDemo_InvokeBgmStop (void *p)
{
	(void)p;
	BGM_Stop ();
}

int ClDemo_Glue_BgmStop (void)
{
	return Host_Guard (ClDemo_InvokeBgmStop, NULL);
}

/* cl_demo.c:278 */
static void ClDemo_InvokeStopAllSounds (void *p)
{
	(void)p;
	S_StopAllSounds (true, true);
}

int ClDemo_Glue_StopAllSounds (void)
{
	return Host_Guard (ClDemo_InvokeStopAllSounds, NULL);
}

/* cl_demo.c:350 */
static void ClDemo_InvokeDemoListRebuild (void *p)
{
	(void)p;
	DemoList_Rebuild ();
}

int ClDemo_Glue_DemoListRebuild (void)
{
	return Host_Guard (ClDemo_InvokeDemoListRebuild, NULL);
}

/* cl_demo.c:426 */
typedef struct
{
	void  *sfx;
	void **out;
} cldemo_loadsound_arg_t;

static void ClDemo_InvokeLoadSound (void *p)
{
	cldemo_loadsound_arg_t *a = (cldemo_loadsound_arg_t *)p;
	*a->out = S_LoadSound ((sfx_t *)a->sfx);
}

int ClDemo_Glue_LoadSound (void *sfx, void **out)
{
	cldemo_loadsound_arg_t arg;
	arg.sfx = sfx;
	arg.out = out;
	*out = NULL;
	return Host_Guard (ClDemo_InvokeLoadSound, &arg);
}

/* cl_demo.c:515 */
static void ClDemo_InvokeFogGetFogCommand (void *p)
{
	*(const char **)p = Fog_GetFogCommand (false);
}

int ClDemo_Glue_FogGetFogCommand (const char **out)
{
	*out = NULL;
	return Host_Guard (ClDemo_InvokeFogGetFogCommand, out);
}

/* cl_demo.c:522 */
static void ClDemo_InvokeSkyGetSkyCommand (void *p)
{
	*(const char **)p = Sky_GetSkyCommand (false);
}

int ClDemo_Glue_SkyGetSkyCommand (const char **out)
{
	*out = NULL;
	return Host_Guard (ClDemo_InvokeSkyGetSkyCommand, out);
}

/* cl_demo.c:775 */
typedef struct
{
	const char *name;
	FILE	  **file;
} cldemo_fopen_arg_t;

static void ClDemo_InvokeComFOpenFile (void *p)
{
	cldemo_fopen_arg_t *a = (cldemo_fopen_arg_t *)p;
	COM_FOpenFile (a->name, a->file, NULL);
}

int ClDemo_Glue_ComFOpenFile (const char *name, FILE **file)
{
	cldemo_fopen_arg_t arg;
	arg.name = name;
	arg.file = file;
	return Host_Guard (ClDemo_InvokeComFOpenFile, &arg);
}

/* ---------------------------------------------------------------------------
 * Re-raising public entry points (client.h:404-412). Each Rust body is a
 * quake_rs_* status core and the jump is re-issued from here, never from a
 * Rust frame (ADR-009).
 */

/* cl_demo.c:59 -- cannot raise: fclose, CL_FinishTimeDemo (Con_Printf only)
 * and Harness_DemoEnded (which exits rather than jumping). */
void CL_StopPlayback (void)
{
	quake_rs_cl_stop_playback ();
}

/* cl_demo.c:214 */
void CL_Seek_f (void)
{
	int r = quake_rs_cl_seek_f ();
	Host_Reraise (r);
}

/* cl_demo.c:293 */
int CL_GetMessage (void)
{
	int out = 0;
	int r = quake_rs_cl_get_message (&out);
	Host_Reraise (r);
	return out;
}

/* cl_demo.c:327 */
void CL_Stop_f (void)
{
	int r = quake_rs_cl_stop_f ();
	Host_Reraise (r);
}

/* cl_demo.c:609 */
void CL_Record_f (void)
{
	int r = quake_rs_cl_record_f ();
	Host_Reraise (r);
}

/* cl_demo.c:726 */
void CL_Resume_Record (qboolean recordsignons)
{
	int r = quake_rs_cl_resume_record (recordsignons);
	Host_Reraise (r);
}

/* cl_demo.c:753 */
void CL_PlayDemo_f (void)
{
	int r = quake_rs_cl_play_demo_f ();
	Host_Reraise (r);
}

/* cl_demo.c:850 */
void CL_TimeDemo_f (void)
{
	int r = quake_rs_cl_time_demo_f ();
	Host_Reraise (r);
}

#endif /* USE_RUST_HOST */
