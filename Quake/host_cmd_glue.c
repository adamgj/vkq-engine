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
// host_cmd_glue.c -- the C frame around the Rust host_cmd.c port (T8.3).
//
// Compiled instead of host_cmd.c under -Duse_rust_host, the same Pattern A
// whole-file swap host.c took at T8.2. Four jobs, in file order:
//
// 1. Own the C-visible objects host_cmd.c defined. All seven keep C storage
//    here, so no ADR-007 row opens or closes at T8.3 (see
//    rust/quake-c-sys/src/host_cmd.rs's module doc for the accounting).
// 2. Guard everything the port calls that can Host_Error / Host_EndGame
//    (ADR-009 rule 3), one named seam per callee. atomics.h's accessors are
//    static inline with compiler-specific barriers, so the two atomic objects
//    and the map-parsing thread are reached through seams too rather than
//    re-derived with Rust orderings (ADR-016: no Rust assumptions about the
//    C worker thread).
// 3. Re-raise from a pure C frame. The port returns a guard status, never a
//    longjmp; HostCmd_Raise turns it back into the original Host_Error.
// 4. Leave everything else plain. The command handlers of host_cmd.c become
//    plain C thunks here rather than Rust function pointers, so
//    Host_InitCommands stays a verbatim copy of the original.
//
// Host_Guard / Host_Reraise / HOST_GUARD_* are declared in quakedef.h:475-479
// and defined in host_glue.c.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t

#include "quake_rs.h"

#ifdef USE_RUST_HOST

// ---------------------------------------------------------------------------
// 1. C-visible storage.
//
// host_cmd.c's non-static data, verbatim and in declaration order. Every one
// of them is read or written by C outside this file (server.h's current_skill,
// quakedef.h's four list heads plus extralevels_sorted, view.c's
// noclip_anglehack), so the storage stays C and the port reaches it through
// quake-c-sys externs.

int current_skill; // host_cmd.c:41

filelist_item_t	 *extralevels;		  // host_cmd.c:193
filelist_item_t **extralevels_sorted; // host_cmd.c:194
filelist_item_t	 *modlist;			  // host_cmd.c:567
filelist_item_t	 *demolist;			  // host_cmd.c:815
filelist_item_t	 *savelist;			  // host_cmd.c:837

qboolean noclip_anglehack; // host_cmd.c:1065

/* host_cmd.c:931 -- print_fn ("version: " ENGINE_NAME_AND_VER "\n"). Kept as
 * C data rather than a Rust literal because ENGINE_NAME_AND_VER
 * (quakever.h:59-61) expands a build-date macro that has no fixed Rust
 * spelling. Used verbatim by the port: it carries no format specifiers. */
const char *const HostCmd_EngineVersionLine = "version: " ENGINE_NAME_AND_VER "\n";

/* host_cmd.c:1510. File-local there, so it is redefined here for the
 * HOSTCMD_RAISE_SAVEGAME_VERSION re-issue below. */
#define SAVEGAME_VERSION 5

/* gl_model.c:2583. External linkage but declared in no header, exactly as in
 * host_cmd.c:43; Host_InitCommands registers it as "mcache". */
void Mod_Print (void);

// ---------------------------------------------------------------------------
// 2a. The map-description parsing thread and its two atomics
// (host_cmd.c:197-198, :377, :398-406, :460-461, :473).

static qthread_t	  *extralevels_parsing_thread;
static atomic_uint32_t extralevels_cancel_parsing;

unsigned int HostCmd_Glue_AtomicLoadU32 (void *atomic)
{
	return Atomic_LoadUInt32 ((atomic_uint32_t *)atomic);
}

void HostCmd_Glue_AtomicStoreU32 (void *atomic, unsigned int desired)
{
	Atomic_StoreUInt32 ((atomic_uint32_t *)atomic, desired);
}

void *HostCmd_Glue_AtomicLoadPtr (void *atomic)
{
	return Atomic_LoadPtr ((atomic_ptr_t *)atomic);
}

void HostCmd_Glue_AtomicStorePtr (void *atomic, void *desired)
{
	Atomic_StorePtr ((atomic_ptr_t *)atomic, desired);
}

/* host_cmd.c:461. Only the QThread_Create half: the port issues the
 * cancel-flag clear at :460 through HostCmd_Glue_SetCancelParsing, keeping
 * the two writes in the original order. */
void HostCmd_Glue_StartParsingThread (qthread_func_t func)
{
	extralevels_parsing_thread = QThread_Create (func, "Map parser", NULL);
}

/* host_cmd.c:398-406 -- ExtraMaps_WaitForParsingThread, verbatim. */
void HostCmd_Glue_WaitForParsingThread (void)
{
	if (extralevels_parsing_thread)
	{
		QThread_Wait (extralevels_parsing_thread);
		extralevels_parsing_thread = NULL;
		Atomic_StoreUInt32 (&extralevels_cancel_parsing, 0);
	}
}

void HostCmd_Glue_SetCancelParsing (unsigned int value)
{
	Atomic_StoreUInt32 (&extralevels_cancel_parsing, value);
}

unsigned int HostCmd_Glue_GetCancelParsing (void)
{
	return Atomic_LoadUInt32 (&extralevels_cancel_parsing);
}

// ---------------------------------------------------------------------------
// 2b. Guarded seams (ADR-009 rule 3). Each is enumerated by name rather than
// reached through a generic function-pointer guard: naming them keeps the
// Pattern A symbol audit meaningful and lets the compiler type-check every
// call. The macros collapse only the boilerplate, not the enumeration.

typedef struct
{
	const char	 *s;
	const char	 *t;
	void		 *p;
	void		 *q;
	void		**outp;
	const char	**outs;
	int			 *outi;
	unsigned int *outu;
	int			  i;
	unsigned int  u;
	float		  f;
	qboolean	  b;
} hostcmd_arg_t;

#define HOSTCMD_GUARD_VOID(name, call)                  \
	static void HostCmd_Invoke##name (void *p)          \
	{                                                   \
		(void)p;                                        \
		call;                                           \
	}                                                   \
	int HostCmd_Glue_##name (void)                      \
	{                                                   \
		return Host_Guard (HostCmd_Invoke##name, NULL); \
	}

#define HOSTCMD_GUARD_STR(name, call)                   \
	static void HostCmd_Invoke##name (void *p)          \
	{                                                   \
		hostcmd_arg_t *a = (hostcmd_arg_t *)p;          \
		call;                                           \
	}                                                   \
	int HostCmd_Glue_##name (const char *s)             \
	{                                                   \
		hostcmd_arg_t arg = {0};                        \
		arg.s = s;                                      \
		return Host_Guard (HostCmd_Invoke##name, &arg); \
	}

#define HOSTCMD_GUARD_PTR(name, call)                   \
	static void HostCmd_Invoke##name (void *p)          \
	{                                                   \
		hostcmd_arg_t *a = (hostcmd_arg_t *)p;          \
		call;                                           \
	}                                                   \
	int HostCmd_Glue_##name (void *ptr)                 \
	{                                                   \
		hostcmd_arg_t arg = {0};                        \
		arg.p = ptr;                                    \
		return Host_Guard (HostCmd_Invoke##name, &arg); \
	}

#define HOSTCMD_GUARD_BOOL(name, call)                  \
	static void HostCmd_Invoke##name (void *p)          \
	{                                                   \
		hostcmd_arg_t *a = (hostcmd_arg_t *)p;          \
		call;                                           \
	}                                                   \
	int HostCmd_Glue_##name (qboolean flag)             \
	{                                                   \
		hostcmd_arg_t arg = {0};                        \
		arg.b = flag;                                   \
		return Host_Guard (HostCmd_Invoke##name, &arg); \
	}

/* Defined further down as the C-visible entry points the port drives; the
 * guards below wrap those wrappers, so a Host_Reraise issued from inside one
 * of them is caught here and returned as a status like any other raise. */
void		SaveList_Rebuild (void);
static void Host_Reconnect_f (void);

// -- chunk A (host_cmd.c:24-898) --------------------------------------------

/* host_cmd.c:56 -- M_Menu_Quit_f (), which reaches Cbuf_AddText. */
HOSTCMD_GUARD_VOID (M_Menu_Quit_f, M_Menu_Quit_f ())
/* host_cmd.c:59. */
HOSTCMD_GUARD_VOID (CL_Disconnect, CL_Disconnect ())
/* host_cmd.c:62 -- Sys_Quit () runs Host_Shutdown before exit (0). */
HOSTCMD_GUARD_VOID (Sys_Quit, Sys_Quit ())

/* host_cmd.c:60 -- Host_ShutdownServer (false). qboolean crosses as the int
 * the HOST_GUARD_INT shape uses (host_glue.c:531 precedent); the seam narrows
 * it back. */
static void HostCmd_InvokeHost_ShutdownServer (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	Host_ShutdownServer (a->i != 0);
}

int HostCmd_Glue_Host_ShutdownServer (int crash)
{
	hostcmd_arg_t arg = {0};

	arg.i = crash;
	return Host_Guard (HostCmd_InvokeHost_ShutdownServer, &arg);
}

/* host_cmd.c:795 -- COM_LoadFile ("mapdb.json", &path_id). Both results leave
 * through out-parameters so the int return stays the guard status
 * (host_glue.c:683 precedent). */
static void HostCmd_InvokeComLoadFile (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	*a->outp = COM_LoadFile (a->s, a->outu);
}

int HostCmd_Glue_ComLoadFile (const char *path, unsigned int *path_id, void **out)
{
	hostcmd_arg_t arg = {0};

	arg.s = path;
	arg.outu = path_id;
	arg.outp = out;
	*out = NULL;
	return Host_Guard (HostCmd_InvokeComLoadFile, &arg);
}

// -- chunk B (host_cmd.c:899-1509) ------------------------------------------

/* host_cmd.c:1341 -- Cmd_ExecuteString ("connect local", src_command) in
 * Host_Map_f. Cmd_ExecuteString cannot itself longjmp, but the handler it
 * dispatches to transitively can. The qboolean result is discarded, matching
 * host_cmd.c's own ignored return value. */
static void HostCmd_InvokeCmdExecuteString (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	Cmd_ExecuteString (a->s, (cmd_source_t)a->u);
}

int HostCmd_Glue_CmdExecuteString (const char *text, unsigned int src)
{
	hostcmd_arg_t arg = {0};

	arg.s = text;
	arg.u = src;
	return Host_Guard (HostCmd_InvokeCmdExecuteString, &arg);
}

/* host_cmd.c:1497 (Host_Connect_f) and :2148 (Host_Reconnect_f). */
HOSTCMD_GUARD_STR (CLEstablishConnection, CL_EstablishConnection (a->s))

/* host_cmd.c:1413 -- Host_Error ("cannot find map %s", level). */
HOSTCMD_GUARD_STR (ErrorCannotFindMap, Host_Error ("cannot find map %s", a->s))
/* host_cmd.c:1426 -- Host_Error ("cannot run map %s", level). */
HOSTCMD_GUARD_STR (ErrorCannotRunMap, Host_Error ("cannot run map %s", a->s))
/* host_cmd.c:1458 -- Host_Error ("cannot restart map %s", mapname). */
HOSTCMD_GUARD_STR (ErrorCannotRestartMap, Host_Error ("cannot restart map %s", a->s))

// -- chunk C (host_cmd.c:1510-2156) -----------------------------------------

/* EDICT_NUM (host_cmd.c:1652, :1779, :2075, :2112). The macro Host_Errors on
 * an out-of-range index (pr_edict_arena.c). */
static void HostCmd_InvokeEdictNum (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	*a->outp = EDICT_NUM (a->i);
}

int HostCmd_Glue_EdictNum (int n, void **out)
{
	hostcmd_arg_t arg = {0};

	arg.i = n;
	arg.outp = out;
	*out = NULL;
	return Host_Guard (HostCmd_InvokeEdictNum, &arg);
}

/* ED_WriteGlobals (f) -- host_cmd.c:1649. */
HOSTCMD_GUARD_PTR (EDWriteGlobals, ED_WriteGlobals ((FILE *)a->p))
/* ED_CheckFreeList () -- host_cmd.c:1703. */
HOSTCMD_GUARD_VOID (EDCheckFreeList, ED_CheckFreeList ())
/* SaveList_Rebuild () -- host_cmd.c:1708; the wrapper defined below. */
HOSTCMD_GUARD_VOID (SaveListRebuild, SaveList_Rebuild ())
/* ED_Free (EDICT_NUM (i)) -- host_cmd.c:2112. */
HOSTCMD_GUARD_PTR (EDFree, ED_Free ((edict_t *)a->p))
/* ED_RebuildFreeList (true) -- host_cmd.c:2119. */
HOSTCMD_GUARD_BOOL (EDRebuildFreeList, ED_RebuildFreeList (a->b))
/* CL_Disconnect_f () -- host_cmd.c:1914, :3123. */
HOSTCMD_GUARD_VOID (CLDisconnect_f, CL_Disconnect_f ())
/* CL_Stop_f () -- host_cmd.c:1916. */
HOSTCMD_GUARD_VOID (CLStop_f, CL_Stop_f ())
/* SV_SpawnServer (mapname) -- host_cmd.c:1921. */
HOSTCMD_GUARD_STR (SVSpawnServer, SV_SpawnServer (a->s))
/* CL_Resume_Record (fastload) -- host_cmd.c:1941. */
HOSTCMD_GUARD_BOOL (CLResumeRecord, CL_Resume_Record (a->b))
/* Sky_LoadSkyBox (com_token) -- host_cmd.c:2049; guarded conservatively, it
 * reaches the image loaders. */
HOSTCMD_GUARD_STR (SkyLoadSkyBox, Sky_LoadSkyBox (a->s))
/* Host_Reconnect_f () -- host_cmd.c:2149; the thunk defined below. */
HOSTCMD_GUARD_VOID (HostReconnect_f, Host_Reconnect_f ())

/* ED_Write (f, ed) -- host_cmd.c:1652. */
static void HostCmd_InvokeEDWrite (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	ED_Write ((FILE *)a->p, (edict_t *)a->q);
}

int HostCmd_Glue_EDWrite (void *f, void *ed)
{
	hostcmd_arg_t arg = {0};

	arg.p = f;
	arg.q = ed;
	return Host_Guard (HostCmd_InvokeEDWrite, &arg);
}

/* One buffered MSG_Write* call. Mirrored in Rust as HostCmdWriteOp
 * (rust/quake-c-sys/src/host_cmd.rs); the kind values are the switch below. */
typedef struct
{
	int			 kind;
	int			 i;
	float		 f;
	unsigned int u;
	const void	*p;
} hostcmd_write_t;

typedef struct
{
	sizebuf_t			  *sb;
	const hostcmd_write_t *ops;
	int					   count;
} hostcmd_writebatch_arg_t;

/* Replays a run of MSG_Write* calls against one sizebuf_t inside a single
 * Host_Guard frame (host_cmd.c:1723-1789). Every writer reaches SZ_GetSpace,
 * which Host_Errors on overflow (net_msg.c:488). */
static void HostCmd_InvokeWriteBatch (void *p)
{
	hostcmd_writebatch_arg_t *a = (hostcmd_writebatch_arg_t *)p;
	int						  k;

	for (k = 0; k < a->count; k++)
	{
		const hostcmd_write_t *op = &a->ops[k];
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
			MSG_WriteString (a->sb, (const char *)op->p);
			break;
		case 5:
			MSG_WriteAngle (a->sb, op->f, op->u);
			break;
		default:
			Sys_Error ("HostCmd_InvokeWriteBatch: unknown op %i", op->kind);
			break;
		}
	}
}

int HostCmd_Glue_WriteBatch (void *sb, const hostcmd_write_t *ops, int count)
{
	hostcmd_writebatch_arg_t arg;

	arg.sb = (sizebuf_t *)sb;
	arg.ops = ops;
	arg.count = count;
	return Host_Guard (HostCmd_InvokeWriteBatch, &arg);
}

/* SV_WriteClientdataToMessage (c, &c->message) -- host_cmd.c:1789. */
static void HostCmd_InvokeSVWriteClientdataToMessage (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	SV_WriteClientdataToMessage ((client_t *)a->p, (sizebuf_t *)a->q);
}

int HostCmd_Glue_SVWriteClientdataToMessage (void *client, void *sb)
{
	hostcmd_arg_t arg = {0};

	arg.p = client;
	arg.q = sb;
	return Host_Guard (HostCmd_InvokeSVWriteClientdataToMessage, &arg);
}

/* Cvar_SetValue ("skill", ...) -- host_cmd.c:1890; a cvar callback can
 * Host_Error. */
static void HostCmd_InvokeCvarSetValue (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	Cvar_SetValue (a->s, a->f);
}

int HostCmd_Glue_CvarSetValue (const char *name, float value)
{
	hostcmd_arg_t arg = {0};

	arg.s = name;
	arg.f = value;
	return Host_Guard (HostCmd_InvokeCvarSetValue, &arg);
}

/* Cvar_SetValueQuick (&nomonsters, 0.f) -- host_cmd.c:1832. */
static void HostCmd_InvokeCvarSetValueQuick (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	Cvar_SetValueQuick ((cvar_t *)a->p, a->f);
}

int HostCmd_Glue_CvarSetValueQuick (cvar_t *var, float value)
{
	hostcmd_arg_t arg = {0};

	arg.p = var;
	arg.f = value;
	return Host_Guard (HostCmd_InvokeCvarSetValueQuick, &arg);
}

/* Mod_ForName (name, crash) -- host_cmd.c:1991, :2907. crash=true Host_Errors
 * from gl_model.c:531; the model comes back in *out. */
static void HostCmd_InvokeModForName (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	*a->outp = Mod_ForName (a->s, a->b);
}

int HostCmd_Glue_ModForName (const char *name, qboolean crash, void **out)
{
	hostcmd_arg_t arg = {0};

	arg.s = name;
	arg.b = crash;
	arg.outp = out;
	*out = NULL;
	return Host_Guard (HostCmd_InvokeModForName, &arg);
}

/* ED_ParseGlobals (data) -- host_cmd.c:2071; the advanced cursor in *out. */
static void HostCmd_InvokeEDParseGlobals (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	*a->outs = ED_ParseGlobals (a->s);
}

int HostCmd_Glue_EDParseGlobals (const char *data, const char **out)
{
	hostcmd_arg_t arg = {0};

	arg.s = data;
	arg.outs = out;
	*out = NULL;
	return Host_Guard (HostCmd_InvokeEDParseGlobals, &arg);
}

/* ED_ParseEdict (data, ent) -- host_cmd.c:2097; cursor in *out. */
static void HostCmd_InvokeEDParseEdict (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	*a->outs = ED_ParseEdict (a->s, (edict_t *)a->p);
}

int HostCmd_Glue_EDParseEdict (const char *data, void *ent, const char **out)
{
	hostcmd_arg_t arg = {0};

	arg.s = data;
	arg.p = ent;
	arg.outs = out;
	*out = NULL;
	return Host_Guard (HostCmd_InvokeEDParseEdict, &arg);
}

/* SV_LinkEdict (ent, false) -- host_cmd.c:2101. */
static void HostCmd_InvokeSVLinkEdict (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	SV_LinkEdict ((edict_t *)a->p, a->b);
}

int HostCmd_Glue_SVLinkEdict (void *ent, qboolean touch_triggers)
{
	hostcmd_arg_t arg = {0};

	arg.p = ent;
	arg.b = touch_triggers;
	return Host_Guard (HostCmd_InvokeSVLinkEdict, &arg);
}

// -- chunk D (host_cmd.c:2158-2649) -----------------------------------------

/* host_cmd.c:2185, :2372-2373 -- Cvar_Set (name, value), which can reach
 * Host_Error through Cvar_SetQuick -> Cvar_CallCallback. */
static void HostCmd_InvokeCvarSet (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	Cvar_Set (a->s, a->t);
}

int HostCmd_Glue_CvarSet (const char *name, const char *value)
{
	hostcmd_arg_t arg = {0};

	arg.s = name;
	arg.t = value;
	return Host_Guard (HostCmd_InvokeCvarSet, &arg);
}

/* Cmd_ForwardToServer () -- host_cmd.c:922, :984, :1029, :1075, :1131, :1188,
 * :2186, :2210, :2299, :2377, :2665, :3234. Reaches MSG_WriteByte/SZ_Print ->
 * SZ_GetSpace, which is Host_Error-capable on overflow. */
HOSTCMD_GUARD_VOID (CmdForwardToServer, Cmd_ForwardToServer ())

/* host_cmd.c:2438, :2440 -- PR_GetString (sv_player->v.netname), which
 * Host_Errors on an invalid string index (pr_edict_arena.c:307-325). */
static void HostCmd_InvokePRGetString (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	*a->outs = PR_GetString (a->i);
}

int HostCmd_Glue_PRGetString (int num, const char **out)
{
	hostcmd_arg_t arg = {0};

	arg.i = num;
	arg.outs = out;
	*out = NULL;
	return Host_Guard (HostCmd_InvokePRGetString, &arg);
}

/* host_cmd.c:2521-2527 -- the extended (17-64) spawn-parm write loop's
 * qcvm->globals[g->ofs] = host_client->spawn_parms[i], after ED_FindGlobal has
 * located g. Write-direction mirror of SvMain_Glue_SpawnParmGlobal
 * (sv_main_glue.c:429-435): ED_FindGlobal and a bounded global-array store
 * have no error path, so this is unguarded and void like its counterpart. */
void HostCmd_Glue_SetSpawnParmGlobal (int index, float value)
{
	ddef_t *g = ED_FindGlobal (va ("parm%i", index));
	if (g)
		qcvm->globals[g->ofs] = value;
}

// -- chunk E (host_cmd.c:2650-3298) -----------------------------------------

/* host_cmd.c:2971 (Host_Viewmodel_f): e->v.modelindex = SV_Precache_Model
 * (m->name). Unlike SvMain_Glue_PrecacheModel (whose only caller at
 * sv_main.c:1048 discards the index), this call site needs the slot. */
static void HostCmd_InvokePrecacheModel (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	*a->outi = SV_Precache_Model (a->s);
}

int HostCmd_Glue_PrecacheModel (const char *name, int *out)
{
	hostcmd_arg_t arg = {0};

	arg.s = name;
	arg.outi = out;
	*out = 0;
	return Host_Guard (HostCmd_InvokePrecacheModel, &arg);
}

/* host_cmd.c:3007 (PrintFrameName): Mod_Extradata (m) ->
 * Mod_Extradata_CheckSkin -> Mod_LoadModel (mod, true), which reloads from
 * disk when the cache slot was dropped and can Host_Error. */
static void HostCmd_InvokeModExtradata (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	*a->outp = Mod_Extradata ((qmodel_t *)a->p);
}

int HostCmd_Glue_ModExtradata (void *model, void **out)
{
	hostcmd_arg_t arg = {0};

	arg.p = model;
	arg.outp = out;
	*out = NULL;
	return Host_Guard (HostCmd_InvokeModExtradata, &arg);
}

/* host_cmd.c:3109, :3126 (Host_Startdemos_f, Host_Demos_f). */
HOSTCMD_GUARD_VOID (CLNextDemo, CL_NextDemo ())

/* host_cmd.c:3189, :3222 (Host_Serverinfo_f, Host_Setinfo_f). */
static void HostCmd_InvokeSVUpdateInfo (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	SV_UpdateInfo (a->i, a->s, a->t);
}

int HostCmd_Glue_SVUpdateInfo (int edict, const char *keyname, const char *value)
{
	hostcmd_arg_t arg = {0};

	arg.i = edict;
	arg.s = keyname;
	arg.t = value;
	return Host_Guard (HostCmd_InvokeSVUpdateInfo, &arg);
}

/* host_cmd.c:3163 (Info_ClientPrint_Callback): SV_ClientPrintf ("%20s: %s\n",
 * key, val). The Rust callback is handed to Info_Enumerate as a plain
 * void (*) (void *, const char *, const char *) and cannot return a status, so
 * it writes this seam's result through Info_Enumerate's own cbctx pointer,
 * into a Raise that Host_Serverinfo_f / Host_Setinfo_f own on their stack and
 * drain right after the enumeration returns. */
static void HostCmd_InvokeSVClientPrintfKV (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	SV_ClientPrintf ("%20s: %s\n", a->s, a->t);
}

int HostCmd_Glue_SVClientPrintfKV (const char *key, const char *val)
{
	hostcmd_arg_t arg = {0};

	arg.s = key;
	arg.t = val;
	return Host_Guard (HostCmd_InvokeSVClientPrintfKV, &arg);
}

/* host_cmd.c:3203 (Host_Setinfo_f): SV_ClientPrintf ("Your Serverside User
 * Info:\n") -- a fixed literal with no format arguments, kept as its own
 * hardcoded seam rather than routed through a generic "%s" wrapper so the
 * format string byte-for-byte matches the original call site. */
HOSTCMD_GUARD_VOID (SVClientPrintfUserInfoHeader, SV_ClientPrintf ("Your Serverside User Info:\n"))

// ---------------------------------------------------------------------------
// 3. Re-raising from a pure C frame (ADR-009 rule 4).
//
// Two Host_Error sites of host_cmd.c are issued by the port itself rather than
// by a guarded callee, so they cross as negative raise codes and are re-issued
// here. The other three (cannot find/run/restart map) fire inside their own
// HostCmd_Glue_ErrorCannot* guard and arrive as ordinary statuses.

#define HOSTCMD_RAISE_SAVEGAME_VERSION	(-101)
#define HOSTCMD_RAISE_FIRST_TOKEN_BRACE (-102)

static void HostCmd_Raise (int r)
{
	switch (r)
	{
	case HOSTCMD_RAISE_SAVEGAME_VERSION:
		/* host_cmd.c:1879 */
		Host_Error ("Savegame is version %i, not %i", quake_rs_hostcmd_raise_detail (), SAVEGAME_VERSION);
		break;
	case HOSTCMD_RAISE_FIRST_TOKEN_BRACE:
		/* host_cmd.c:2066 */
		Host_Error ("First token isn't a brace");
		break;
	default:
		Host_Reraise (r);
		break;
	}
}

// ---------------------------------------------------------------------------
// 4a. The C-visible entry points of host_cmd.c (quakedef.h:454-497).

maptype_t ExtraMaps_GetType (const filelist_item_t *item)
{
	return (maptype_t)quake_rs_hostcmd_extra_maps_get_type (item);
}

const char *ExtraMaps_GetMessage (const filelist_item_t *item)
{
	return quake_rs_hostcmd_extra_maps_get_message (item);
}

qboolean ExtraMaps_IsStart (maptype_t type)
{
	return quake_rs_hostcmd_extra_maps_is_start ((unsigned int)type);
}

void ExtraMaps_Init (void)
{
	quake_rs_hostcmd_extra_maps_init ();
}

void ExtraMaps_Clear (void)
{
	quake_rs_hostcmd_extra_maps_clear ();
}

void ExtraMaps_ShutDown (void)
{
	quake_rs_hostcmd_extra_maps_shut_down ();
}

void ExtraMaps_NewGame (void)
{
	quake_rs_hostcmd_extra_maps_new_game ();
}

const char *Modlist_GetFullName (const filelist_item_t *item)
{
	return quake_rs_hostcmd_modlist_get_full_name (item);
}

void Modlist_Init (void)
{
	HostCmd_Raise (quake_rs_hostcmd_modlist_init ());
}

void DemoList_Rebuild (void)
{
	quake_rs_hostcmd_demo_list_rebuild ();
}

void DemoList_Init (void)
{
	quake_rs_hostcmd_demo_list_init ();
}

void SaveList_Rebuild (void)
{
	quake_rs_hostcmd_save_list_rebuild ();
}

void SaveList_Init (void)
{
	quake_rs_hostcmd_save_list_init ();
}

void Host_Quit_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_host_quit_f ());
}

/* Host_Resetdemos (host_cmd.c:3155) needs no wrapper: it cannot raise, so the
 * port exports it directly under its C name for common.c:1763. */

// ---------------------------------------------------------------------------
// 4b. The command handlers. Every one of them is file-static in host_cmd.c and
// referenced only by Host_InitCommands, so they stay static here; each is a
// thin thunk that turns the port's status back into a raise.

static void Host_Maps_f (void)
{
	quake_rs_hostcmd_maps_f ();
}

static void Host_Mods_f (void)
{
	quake_rs_hostcmd_mods_f ();
}

static void Host_Mapname_f (void)
{
	quake_rs_hostcmd_mapname_f ();
}

static void Host_Randmap_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_randmap_f ());
}

static void Host_Serverinfo_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_serverinfo_f ());
}

static void Host_Setinfo_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_setinfo_f ());
}

static void Host_User_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_user_f ());
}

static void Host_Status_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_status_f ());
}

static void Host_God_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_god_f ());
}

static void Host_Notarget_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_notarget_f ());
}

static void Host_Fly_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_fly_f ());
}

static void Host_Map_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_map_f ());
}

static void Host_Restart_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_restart_f ());
}

static void Host_Changelevel_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_changelevel_f ());
}

static void Host_Connect_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_connect_f ());
}

static void Host_Reconnect_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_reconnect_f ());
}

static void Host_Name_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_host_name_f ());
}

static void Host_Noclip_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_noclip_f ());
}

static void Host_SetPos_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_setpos_f ());
}

static void Host_Say_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_host_say_f ());
}

static void Host_Say_Team_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_host_say_team_f ());
}

static void Host_Tell_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_host_tell_f ());
}

static void Host_Color_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_host_color_f ());
}

static void Host_Kill_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_host_kill_f ());
}

static void Host_Pause_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_host_pause_f ());
}

static void Host_Spawn_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_host_spawn_f ());
}

static void Host_Begin_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_host_begin_f ());
}

static void Host_PreSpawn_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_host_prespawn_f ());
}

static void Host_Kick_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_host_kick_f ());
}

static void Host_Ping_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_ping_f ());
}

static void Host_Loadgame_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_loadgame_f ());
}

static void Host_Savegame_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_savegame_f ());
}

static void Host_Give_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_give_f ());
}

static void Host_Startdemos_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_startdemos_f ());
}

static void Host_Demos_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_demos_f ());
}

static void Host_Stopdemo_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_stopdemo_f ());
}

static void Host_Viewmodel_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_viewmodel_f ());
}

static void Host_Viewframe_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_viewframe_f ());
}

static void Host_Viewnext_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_viewnext_f ());
}

static void Host_Viewprev_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_viewprev_f ());
}

// ---------------------------------------------------------------------------
// 4c. Registration. Verbatim from host_cmd.c:3299-3351, including comments and
// order -- Cmd_AddCommand order is observable through "cmdlist" and completion.

/*
==================
Host_InitCommands
==================
*/
void Host_InitCommands (void)
{
	Cmd_AddCommand ("maps", Host_Maps_f);		// johnfitz
	Cmd_AddCommand ("mods", Host_Mods_f);		// johnfitz
	Cmd_AddCommand ("games", Host_Mods_f);		// as an alias to "mods" -- S.A. / QuakeSpasm
	Cmd_AddCommand ("mapname", Host_Mapname_f); // johnfitz
	Cmd_AddCommand ("randmap", Host_Randmap_f); // ericw

	Cmd_AddCommand_ClientCommand ("serverinfo", Host_Serverinfo_f); // spike
	Cmd_AddCommand_ClientCommand ("setinfo", Host_Setinfo_f);		// spike
	Cmd_AddCommand ("user", Host_User_f);							// spike

	Cmd_AddCommand ("status", Host_Status_f);
	Cmd_AddCommand ("quit", Host_Quit_f);
	Cmd_AddCommand ("god", Host_God_f);
	Cmd_AddCommand ("notarget", Host_Notarget_f);
	Cmd_AddCommand ("fly", Host_Fly_f);
	Cmd_AddCommand ("map", Host_Map_f);
	Cmd_AddCommand ("restart", Host_Restart_f);
	Cmd_AddCommand ("changelevel", Host_Changelevel_f);
	Cmd_AddCommand ("connect", Host_Connect_f);
	Cmd_AddCommand ("reconnect", Host_Reconnect_f);
	Cmd_AddCommand ("name", Host_Name_f);
	Cmd_AddCommand ("noclip", Host_Noclip_f);
	Cmd_AddCommand ("setpos", Host_SetPos_f); // QuakeSpasm

	Cmd_AddCommand ("say", Host_Say_f);
	Cmd_AddCommand ("say_team", Host_Say_Team_f);
	Cmd_AddCommand ("tell", Host_Tell_f);
	Cmd_AddCommand ("color", Host_Color_f);
	Cmd_AddCommand ("kill", Host_Kill_f);
	Cmd_AddCommand ("pause", Host_Pause_f);
	Cmd_AddCommand ("spawn", Host_Spawn_f);
	Cmd_AddCommand ("begin", Host_Begin_f);
	Cmd_AddCommand ("prespawn", Host_PreSpawn_f);
	Cmd_AddCommand ("kick", Host_Kick_f);
	Cmd_AddCommand ("ping", Host_Ping_f);
	Cmd_AddCommand ("load", Host_Loadgame_f);
	Cmd_AddCommand ("fastload", Host_Loadgame_f);
	Cmd_AddCommand ("save", Host_Savegame_f);
	Cmd_AddCommand ("give", Host_Give_f);

	Cmd_AddCommand ("startdemos", Host_Startdemos_f);
	Cmd_AddCommand ("demos", Host_Demos_f);
	Cmd_AddCommand ("stopdemo", Host_Stopdemo_f);

	Cmd_AddCommand ("viewmodel", Host_Viewmodel_f);
	Cmd_AddCommand ("viewframe", Host_Viewframe_f);
	Cmd_AddCommand ("viewnext", Host_Viewnext_f);
	Cmd_AddCommand ("viewprev", Host_Viewprev_f);

	Cmd_AddCommand ("mcache", Mod_Print);
}

#endif /* USE_RUST_HOST */
