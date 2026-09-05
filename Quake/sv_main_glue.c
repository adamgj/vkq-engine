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
// sv_main_glue.c -- the C frame around the Rust server connection port.
//
// Compiled instead of sv_main.c under -Duse_rust_host (Rust migration Phase 7
// M6). Four jobs, mirroring sv_phys_glue.c:
//
//  1. Own the C-visible objects sv_main.c defined: sv_protocol (sv_main.c:32),
//     sv_protocol_pext1/pext2 (:34-35) and the two cvars sv_netsort (:37) and
//     sv_smoothplatformlerps (:38). localmodels (:30) had internal linkage,
//     so it became a Rust-owned array instead. server_t sv (:27) and
//     server_static_t svs (:28) are NOT here: T6.6 moved that storage to
//     quake-capi's sv_main.rs, closing the ADR-007 sv/svs dual view.
//     server.h's extern declarations are unchanged and the 35 C translation
//     units that read sv./svs. resolve to the Rust definitions.
//  2. Guard everything sv_main.c reached that can Host_Error / Host_EndGame
//     (ADR-009 rule 3): the three SV_StartSound argument checks, every
//     MSG_Write* run (batched -- see SvMain_Glue_WriteBatch), PR_GetString,
//     the two PR_ExecuteProgram dispatches, SVFTE_SetupFrames,
//     SV_SendReconnect, SV_CreateBaseline, Host_ClearMemory, PR_LoadProgs,
//     Mod_ForName, ED_LoadFromFile, SV_Precache_Model, and -- because under
//     -Duse_rust_cvar the plain names are themselves Host_Reraise wrappers --
//     Cvar_RegisterVariable, Cvar_Set, Cvar_SetValue and Cmd_AddCommand.
//  3. Re-raise, from a pure C frame, what those guards caught. Every sv_main.c
//     entry point whose body can raise is a thin wrapper over a quake_rs_*
//     status core; Host_Reraise is called only from here. SV_ClearDatagram,
//     SV_ModelIndex and SV_ModelForIndex cannot raise (the only error path in
//     SV_ModelIndex is Sys_Error, which terminates rather than longjmping), so
//     Rust exports those three under their own names.
//  4. Keep the compile-time-conditional and macro-heavy fragments in C: the
//     ENGINE_NAME_AND_VER serverinfo banner, PROGHEADER_CRC and the ssqc
//     builtin table, the DEBUG-only per-edict field loop, the NDEBUG-gated
//     assert, and the variadic va() lookup in front of ED_FindGlobal.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

/* ---------------------------------------------------------------------------
 * C-visible storage (sv_main.c:32-38; sv/svs are Rust-owned, see above).
 */

int sv_protocol = PROTOCOL_RMQ; // spike -- enough maps need this now that we can probably afford incompatibility with engines that still don't support 999
								// (vanilla was already broken) -- PROTOCOL_FITZQUAKE; //johnfitz
unsigned int sv_protocol_pext1 = PEXT1_SUPPORTED_SERVER; // spike
unsigned int sv_protocol_pext2 = PEXT2_SUPPORTED_SERVER; // spike

cvar_t sv_netsort = {"sv_netsort", "1", CVAR_NONE};
cvar_t sv_smoothplatformlerps = {"sv_smoothplatformlerps", "1", CVAR_NONE};

/* ---------------------------------------------------------------------------
 * Guarded callbacks (ADR-009 rule 3).
 */

/* sv_main.c:285, :293, :296 -- the three SV_StartSound argument checks. The
   %f in the attenuation message is the only floating-point conversion
   specifier anywhere in sv_main.c, and it stays in C (ADR-005). */
static void SvMain_InvokeErrorVolume (void *p)
{
	Host_Error ("SV_StartSound: volume = %i", *(int *)p);
}

int SvMain_Glue_ErrorVolume (int volume)
{
	return Host_Guard (SvMain_InvokeErrorVolume, &volume);
}

static void SvMain_InvokeErrorAttenuation (void *p)
{
	Host_Error ("SV_StartSound: attenuation = %f", *(float *)p);
}

int SvMain_Glue_ErrorAttenuation (float attenuation)
{
	return Host_Guard (SvMain_InvokeErrorAttenuation, &attenuation);
}

static void SvMain_InvokeErrorChannel (void *p)
{
	Host_Error ("SV_StartSound: channel = %i", *(int *)p);
}

int SvMain_Glue_ErrorChannel (int channel)
{
	return Host_Guard (SvMain_InvokeErrorChannel, &channel);
}

typedef struct
{
	int			 handle;
	const char **out;
} svmain_getstring_arg_t;

/* sv_main.c:581. PR_GetString reaches Host_Error on a corrupt string_t
   (pr_edict_arena.c:315). */
static void SvMain_InvokeGetString (void *p)
{
	svmain_getstring_arg_t *a = (svmain_getstring_arg_t *)p;
	*a->out = PR_GetString (a->handle);
}

int SvMain_Glue_GetString (int handle, const char **out)
{
	svmain_getstring_arg_t arg;

	arg.handle = handle;
	arg.out = out;
	*out = NULL;
	return Host_Guard (SvMain_InvokeGetString, &arg);
}

/* sv_main.c:614. */
static void SvMain_InvokeSetupFrames (void *p)
{
	SVFTE_SetupFrames ((client_t *)p);
}

int SvMain_Glue_SetupFrames (client_t *client)
{
	return Host_Guard (SvMain_InvokeSetupFrames, client);
}

/* sv_main.c:900. */
static void SvMain_InvokeSendReconnect (void *p)
{
	(void)p;
	SV_SendReconnect ();
}

int SvMain_Glue_SendReconnect (void)
{
	return Host_Guard (SvMain_InvokeSendReconnect, NULL);
}

/* sv_main.c:1064. */
static void SvMain_InvokeCreateBaseline (void *p)
{
	(void)p;
	SV_CreateBaseline ();
}

int SvMain_Glue_CreateBaseline (void)
{
	return Host_Guard (SvMain_InvokeCreateBaseline, NULL);
}

/* sv_main.c:927. */
static void SvMain_InvokeClearMemory (void *p)
{
	(void)p;
	Host_ClearMemory ();
}

int SvMain_Glue_ClearMemory (void)
{
	return Host_Guard (SvMain_InvokeClearMemory, NULL);
}

/* sv_main.c:946. The four constant arguments stay here so the ssqc builtin
   table never crosses the FFI boundary. */
static void SvMain_InvokeLoadProgs (void *p)
{
	(void)p;
	PR_LoadProgs ("progs.dat", true, PROGHEADER_CRC, pr_ssqcbuiltins, pr_ssqcnumbuiltins);
}

int SvMain_Glue_LoadProgs (void)
{
	return Host_Guard (SvMain_InvokeLoadProgs, NULL);
}

typedef struct
{
	const char *name;
	void	  **out;
} svmain_modforname_arg_t;

/* sv_main.c:996 and :1029. */
static void SvMain_InvokeModForName (void *p)
{
	svmain_modforname_arg_t *a = (svmain_modforname_arg_t *)p;
	*a->out = Mod_ForName (a->name, false);
}

int SvMain_Glue_ModForName (const char *name, void **out)
{
	svmain_modforname_arg_t arg;

	arg.name = name;
	arg.out = out;
	*out = NULL;
	return Host_Guard (SvMain_InvokeModForName, &arg);
}

/* sv_main.c:1047. */
static void SvMain_InvokeLoadFromFile (void *p)
{
	ED_LoadFromFile ((const char *)p);
}

int SvMain_Glue_LoadFromFile (const char *data)
{
	return Host_Guard (SvMain_InvokeLoadFromFile, (void *)(intptr_t)data);
}

/* sv_main.c:1051. */
static void SvMain_InvokePrecacheModel (void *p)
{
	SV_Precache_Model ((const char *)p);
}

int SvMain_Glue_PrecacheModel (const char *name)
{
	return Host_Guard (SvMain_InvokePrecacheModel, (void *)(intptr_t)name);
}

/* sv_main.c:748. */
static void SvMain_InvokeSetNewParms (void *p)
{
	(void)p;
	PR_ExecuteProgram (pr_global_struct->SetNewParms);
}

int SvMain_Glue_CallSetNewParms (void)
{
	return Host_Guard (SvMain_InvokeSetNewParms, NULL);
}

/* sv_main.c:858-859. The pr_global_struct->self store is part of the same
   two-statement idiom, so it stays inside the guarded frame. */
static void SvMain_InvokeSetChangeParms (void *p)
{
	pr_global_struct->self = EDICT_TO_PROG ((edict_t *)p);
	PR_ExecuteProgram (pr_global_struct->SetChangeParms);
}

int SvMain_Glue_CallSetChangeParms (edict_t *ent)
{
	return Host_Guard (SvMain_InvokeSetChangeParms, ent);
}

/* ---------------------------------------------------------------------------
 * cvar / command registration. Under -Duse_rust_cvar the plain
 * Cvar_RegisterVariable, Cvar_Set, Cvar_SetValue and Cmd_AddCommand names are
 * themselves Host_Reraise wrappers, so a Rust frame must never call them
 * directly.
 */

static void SvMain_InvokeRegisterVariable (void *p)
{
	Cvar_RegisterVariable ((cvar_t *)p);
}

int SvMain_Glue_RegisterVariable (cvar_t *var)
{
	return Host_Guard (SvMain_InvokeRegisterVariable, var);
}

/* sv_main.c:167-168 and :173. Pure field stores; kept here only so the
   Host_Callback_Notify function pointer never crosses the FFI boundary. */
void SvMain_Glue_SetNotifyCallback (cvar_t *var)
{
	Cvar_SetCallback (var, Host_Callback_Notify);
}

/* The two xcommand_t entry points sv_main.c registered. Both were static in
   C; the Rust cores keep their bodies, these keep their linkage. */
static void SvMain_PextCommand (void)
{
	Host_Reraise (quake_rs_sv_pext_f ());
}

static void SvMain_ProtocolCommand (void)
{
	quake_rs_sv_protocol_f ();
}

/* sv_main.c:192-193, in that order. */
static void SvMain_InvokeAddCommands (void *p)
{
	(void)p;
	Cmd_AddCommand ("pext", SvMain_PextCommand);
	Cmd_AddCommand ("sv_protocol", &SvMain_ProtocolCommand); // johnfitz
}

int SvMain_Glue_AddCommands (void)
{
	return Host_Guard (SvMain_InvokeAddCommands, NULL);
}

typedef struct
{
	const char *name;
	const char *value;
} svmain_cvarset_arg_t;

/* sv_main.c:894 and :906. */
static void SvMain_InvokeCvarSet (void *p)
{
	svmain_cvarset_arg_t *a = (svmain_cvarset_arg_t *)p;
	Cvar_Set (a->name, a->value);
}

int SvMain_Glue_CvarSet (const char *name, const char *value)
{
	svmain_cvarset_arg_t arg;

	arg.name = name;
	arg.value = value;
	return Host_Guard (SvMain_InvokeCvarSet, &arg);
}

typedef struct
{
	const char *name;
	float		value;
} svmain_cvarsetvalue_arg_t;

/* sv_main.c:918. */
static void SvMain_InvokeCvarSetValue (void *p)
{
	svmain_cvarsetvalue_arg_t *a = (svmain_cvarsetvalue_arg_t *)p;
	Cvar_SetValue (a->name, a->value);
}

int SvMain_Glue_CvarSetValue (const char *name, float value)
{
	svmain_cvarsetvalue_arg_t arg;

	arg.name = name;
	arg.value = value;
	return Host_Guard (SvMain_InvokeCvarSetValue, &arg);
}

/* ---------------------------------------------------------------------------
 * Guarded message writing. Every MSG_Write* reaches SZ_GetSpace, which
 * Host_Errors when the sizebuf disallows overflow (net_msg.c:493), so no such
 * call may be made straight from Rust. A whole run executes inside one
 * Host_Guard frame: a raise abandons the rest of the run exactly as the C
 * longjmp abandoned the rest of the function, so the byte stream is identical.
 */

typedef struct
{
	int			kind;
	int			i;
	float		f;
	const char *s;
} svmain_write_t;

typedef struct
{
	sizebuf_t			 *sb;
	const svmain_write_t *ops;
	int					  count;
} svmain_writebatch_arg_t;

static void SvMain_InvokeWriteBatch (void *p)
{
	svmain_writebatch_arg_t *a = (svmain_writebatch_arg_t *)p;
	int						 k;

	for (k = 0; k < a->count; k++)
	{
		const svmain_write_t *op = &a->ops[k];
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
			MSG_WriteString (a->sb, op->s);
			break;
		case 5:
			MSG_WriteCoord (a->sb, op->f, sv.protocolflags);
			break;
		default:
			break;
		}
	}
}

int SvMain_Glue_WriteBatch (sizebuf_t *sb, const svmain_write_t *ops, int count)
{
	svmain_writebatch_arg_t arg;

	arg.sb = sb;
	arg.ops = ops;
	arg.count = count;
	return Host_Guard (SvMain_InvokeWriteBatch, &arg);
}

/* ---------------------------------------------------------------------------
 * Non-raising shims for macro-heavy or compile-time-conditional fragments.
 */

/* sv_main.c:547-549. ENGINE_NAME_AND_VER stays where it is authoritative. */
void SvMain_Glue_ServerinfoPrint (char *out, size_t size, int crc)
{
	q_snprintf (out, size, "%c\n" ENGINE_NAME_AND_VER " Server (%i CRC)\n", 2, crc);
}

/* sv_main.c:864-865. va() is variadic and ED_FindGlobal cannot raise. */
void SvMain_Glue_SpawnParmGlobal (int index, float *out)
{
	ddef_t *g = ED_FindGlobal (va ("parm%i", index));
	*out = g ? qcvm->globals[g->ofs] : 0;
}

/* sv_main.c:954-963 -- the whole #if defined(DEBUG) || defined(_DEBUG) block,
   kept here so the compile-time condition stays where it is authoritative. */
void SvMain_Glue_InitDebugEdicts (void)
{
#if defined(DEBUG) || defined(_DEBUG)
	for (int j = 0; j < qcvm->max_edicts; j++)
	{
		// set debug fiels for all max_edicts
		edict_t *e = EDICT_NUM_NO_CHECK (j);
		e->qcvm_owner = qcvm;
		e->edict_ptr = e;
		e->edict_num = j;
	}
#endif
}

/* sv_main.c:987 -- assert is NDEBUG-gated, so it stays in C too. */
void SvMain_Glue_AssertEdictNotFree (edict_t *ent)
{
	assert (!ent->free);
	(void)ent;
}

/* ---------------------------------------------------------------------------
 * Entry points: re-raise, from a pure C frame, whatever the Rust core caught.
 * Host_Reraise is a no-op on HOST_GUARD_OK, so no test is needed.
 */

void SV_Init (void)
{
	Host_Reraise (quake_rs_sv_init ());
}

void SV_StartParticle (vec3_t org, vec3_t dir, int color, int count)
{
	Host_Reraise (quake_rs_sv_start_particle (org, dir, color, count));
}

void SV_StartSound (edict_t *entity, float *origin, int channel, const char *sample, int volume, float attenuation)
{
	Host_Reraise (quake_rs_sv_start_sound (entity, origin, channel, sample, volume, attenuation));
}

void SV_LocalSound (client_t *client, const char *sample)
{
	Host_Reraise (quake_rs_sv_local_sound (client, sample));
}

void SV_SendServerinfo (client_t *client)
{
	Host_Reraise (quake_rs_sv_send_serverinfo (client));
}

/* T6.5 deliverable: statusized so M9 can flip _Datagram_ServerControlPacket
   without a longjmp crossing a Rust frame. The status set is exactly
   Host_Guard's -- HOST_GUARD_OK (0), HOST_GUARD_ABORTSERVER (1) and
   HOST_GUARD_SCREEN_ERROR (2) -- propagated verbatim from EDICT_NUM,
   PR_ExecuteProgram (SetNewParms) and the tail call into SV_SendServerinfo. */
void SV_ConnectClient (int clientnum)
{
	Host_Reraise (quake_rs_sv_connect_client (clientnum));
}

void SV_CheckForNewClients (void)
{
	Host_Reraise (quake_rs_sv_check_for_new_clients ());
}

void SV_SaveSpawnparms (void)
{
	Host_Reraise (quake_rs_sv_save_spawnparms ());
}

void SV_SpawnServer (const char *server)
{
	Host_Reraise (quake_rs_sv_spawn_server (server));
}

#endif // USE_RUST_HOST
