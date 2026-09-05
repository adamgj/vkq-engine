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
// host_glue.c -- the C frame around the Rust host orchestration port.
//
// Compiled instead of host.c under -Duse_rust_host (Rust migration Phase 7
// M8, T8.2), following the four-point Pattern A shape cl_main_glue.c and
// sv_main_glue.c established:
//
//  1. Own the C-visible objects host.c defined -- all of them. Unlike M6/M7
//     this flip closes no ADR-007 dual-view row, so nothing moves to Rust
//     storage. The measured reason: fourteen of these globals (host_frametime,
//     realtime, host_framecount, host_initialized, host_parms, host_client,
//     developer, skill, deathmatch, coop, teamplay, max_edicts, dev_stats/
//     dev_peakstats, dev_overflows) are already named in `extern "C"` blocks
//     across nine already-ported Rust modules, so re-homing them in Rust would
//     duplicate definitions -- the M5 defect MSVC merges silently and Linux
//     hard-errors.
//
//     Eight have external linkage but no header declaration, so this file must
//     define them under those exact spellings or those translation units fail
//     to link: screen_error (gl_screen.c), host_netinterval (cl_main.c and
//     cl_main.rs), sv_speeds (sv_phys.c and sv_phys.rs), host_maxfps (menu.c),
//     host_timescale (gl_screen.c), pausable (host_cmd.c), autoload and
//     autofastload (host_cmd.c, menu.c). The header-less-external defect class
//     is why the Pattern A checklist enumerates every non-static file-scope
//     symbol before a flip.
//
//     pr_engine (host.c:98) loses its `static` here: Host_InitLocal, the only
//     code that takes its address, moved to Rust (the T7.4 linkage rule).
//
//  2. Guard everything host.c reached that can Host_Error / Host_EndGame
//     (ADR-009 rule 3): the MSG_Write* runs into a client's message, cls.message
//     and Host_ShutdownServer's stack-local sizebuf (batched, as in
//     cl_main_glue.c); the NET_* funnels; PR_ExecuteProgram / PR_LoadProgs /
//     PR_ClearProgs / PR_SetEngineString and EDICT_NUM / EDICT_TO_PROG; the
//     SV_* frame entry points; SV_ClearWorld; SVFTE_DestroyFrames; CL_Disconnect,
//     CL_ReadFromServer, CL_SendCmd and CL_FreeState; Cbuf_AddText /
//     Cbuf_InsertText / Cbuf_Execute; Cvar_SetQuick; SCR_UpdateScreen;
//     Mod_ClearAll; and the one-shot subsystem init/shutdown calls.
//
//  3. Re-raise, from a pure C frame, what those guards caught. host.c has no
//     Host_Error or Host_EndGame call site of its own -- its only failure exits
//     are five Sys_Error calls (:201, :225, :271, :381, :1333), which terminate
//     rather than jumping -- so unlike cl_main_glue.c there is no HOST_RAISE_*
//     code set. Every quake_rs_host_* core returns a Host_Guard status verbatim
//     and the wrappers below pass it straight to Host_Reraise.
//
//  4. Leave everything else plain. Con_Printf / Con_DPrintf / Con_Warning /
//     Con_DWarning (per the convention cl_demo_glue.c:47-51 records), the Mem_*
//     allocator and Sys_Error (which terminate rather than jumping),
//     PR_SwitchQCVM (pr_edict_load.c:36 -- a pointer swap whose only failure
//     exit is Sys_Error), Cvar_RegisterVariable, Cvar_SetCallback,
//     Cmd_AddCommand, COM_CheckParm, COM_Rand, Sys_DoubleTime, Sys_Printf,
//     Sys_ConsoleInput, Sys_SendKeyEvents, SDL_Delay, SZ_Clear, Info_GetKey,
//     q_vsnprintf/q_snprintf/q_strlcpy and the Steam_SetStatus_* shims cannot
//     longjmp, so the Rust side calls them directly.
//
// Host_Error (:218), Host_EndGame (:185), Host_Guard (:302) and Host_Reraise
// (:339) are reproduced verbatim below: they are the raise machinery itself and
// stay C until Phase 9, per the roadmap and ADR-009. So does the setjmp shell
// of _Host_Frame -- see Host_Glue_FrameInner at the bottom of this file.

#include <setjmp.h>

#include "quakedef.h"
#include "sys.h"

#include "bgmusic.h"
#include "steam.h"
#include "tasks.h"
#ifdef _DEBUG
#include "gl_heap.h"
#endif
#include "quake_rs.h"

/*
 * 1. Storage. Every non-static file-scope object host.c defined, reproduced
 * verbatim; pr_engine additionally loses its `static` (see point 1 above).
 */

quakeparms_t *host_parms;

qboolean host_initialized; // true if into command execution

double host_frametime;
double host_rawframetime; // unscaled and unbounded
double realtime;		  // without any filtering or bounding
double oldrealtime;		  // last frame run

int host_framecount;

int minimum_memory;

client_t *host_client; // current client

jmp_buf host_abortserver;
jmp_buf screen_error;

byte  *host_colormap;
float  host_netinterval = 1.0 / HOST_NETITERVAL_FREQ;
cvar_t host_framerate = {"host_framerate", "0", CVAR_NONE}; // set for slow motion
cvar_t host_speeds = {"host_speeds", "0", CVAR_NONE};		// set for running times
cvar_t sv_speeds = {"sv_speeds", "0", CVAR_NONE};			// print per-tick server cost, split by section
cvar_t host_maxfps = {"host_maxfps", "200", CVAR_ARCHIVE};	// johnfitz

cvar_t host_phys_max_ticrate = {"host_phys_max_ticrate", "0", CVAR_NONE}; // vso = [0 = disabled; MAX_PHYSICS_FREQ]

cvar_t host_timescale = {"host_timescale", "0", CVAR_NONE}; // johnfitz
cvar_t max_edicts = {"max_edicts", "32000", CVAR_NONE};		// vso -- changed from 8192 to 32000 = MAX_EDICTS, because there is no performance impact to do so
cvar_t cl_nocsqc = {"cl_nocsqc", "0", CVAR_NONE};			// spike -- blocks the loading of any csqc modules

cvar_t sys_ticrate = {"sys_ticrate", "0.025", CVAR_NONE}; // dedicated server
cvar_t serverprofile = {"serverprofile", "0", CVAR_NONE};

cvar_t fraglimit = {"fraglimit", "0", CVAR_NOTIFY | CVAR_SERVERINFO};
cvar_t timelimit = {"timelimit", "0", CVAR_NOTIFY | CVAR_SERVERINFO};
cvar_t teamplay = {"teamplay", "0", CVAR_NOTIFY | CVAR_SERVERINFO};
cvar_t samelevel = {"samelevel", "0", CVAR_NONE};
cvar_t noexit = {"noexit", "0", CVAR_NOTIFY | CVAR_SERVERINFO};
cvar_t skill = {"skill", "1", CVAR_NONE};			// 0 - 3
cvar_t deathmatch = {"deathmatch", "0", CVAR_NONE}; // 0, 1, or 2
cvar_t coop = {"coop", "0", CVAR_NONE};				// 0 or 1

cvar_t pausable = {"pausable", "1", CVAR_NONE};

cvar_t autoload = {"autoload", "1", CVAR_ARCHIVE};
cvar_t autofastload = {"autofastload", "0", CVAR_ARCHIVE};

cvar_t developer = {"developer", "0", CVAR_NONE};

cvar_t pr_engine = {"pr_engine", ENGINE_NAME_AND_VER, CVAR_NONE};
cvar_t temp1 = {"temp1", "0", CVAR_NONE};

cvar_t devstats = {"devstats", "0", CVAR_NONE}; // johnfitz -- track developer statistics that vary every frame

cvar_t campaign = {"campaign", "0", CVAR_NONE};	  // for the 2021 rerelease
cvar_t horde = {"horde", "0", CVAR_NONE};		  // for the 2021 rerelease
cvar_t sv_cheats = {"sv_cheats", "0", CVAR_NONE}; // for the 2021 rerelease

devstats_t		dev_stats, dev_peakstats;
overflowtimes_t dev_overflows; // this stores the last time overflow messages were displayed, not the last time overflows occured

/*
 * Preprocessor-only values host.c interpolated directly. They have no
 * translation into Rust -- __TIME__/__DATE__ are stamped by the compiler, and
 * VERSION/QUAKESPASM_VER_STRING/ENGINE_NAME_AND_VER live in quakever.h, which
 * is not bindgen-visible -- so the glue owns them as data (point 1) and the
 * port reads them. host.c:401-413 and host.c:1377-1383.
 */
const double	  host_glue_version = VERSION;
const char *const host_glue_quakespasm_ver = QUAKESPASM_VER_STRING;
const char *const host_glue_engine_ver = ENGINE_NAME_AND_VER;
const char *const host_glue_build_time = __TIME__;
const char *const host_glue_build_date = __DATE__;
#if defined(DEBUG) || defined(_DEBUG)
const char *const host_glue_build_suffix = "(DEBUG build)";
#else
const char *const host_glue_build_suffix = "";
#endif

/*
 * host.c:1277-1284 -- Tests_Init's registration table. The three commands only
 * exist under _DEBUG, so the table (not the loop) is what is conditional; the
 * port walks it in order and the release build sees a count of zero.
 */
#ifdef _DEBUG
const char *const host_glue_test_names[] = {"test_hash_map", "test_gl_heap", "test_tasks"};
xcommand_t		  host_glue_test_funcs[] = {TestHashMap_f, GL_HeapTest_f, TestTasks_f};
const int		  host_glue_num_tests = 3;
#else
const char *const host_glue_test_names[1] = {NULL};
xcommand_t		  host_glue_test_funcs[1] = {NULL};
const int		  host_glue_num_tests = 0;
#endif

/*
 * 2. The raise machinery, verbatim from host.c. ADR-009: these define the
 * longjmp targets, so they stay C until Phase 9 removes the setjmp shell.
 */

/*
================
Host_EndGame
================
*/
void Host_EndGame (const char *message, ...)
{
	va_list argptr;
	char	string[1024];

	va_start (argptr, message);
	q_vsnprintf (string, sizeof (string), message, argptr);
	va_end (argptr);
	Con_DPrintf ("Host_EndGame: %s\n", string);

	PR_SwitchQCVM (NULL);

	if (sv.active)
		Host_ShutdownServer (false);

	if (cls.state == ca_dedicated)
		Sys_Error ("Host_EndGame: %s\n", string); // dedicated servers exit

	if (cls.demonum != -1 && !cls.timedemo)
		CL_NextDemo ();
	else
		CL_Disconnect ();

	longjmp (host_abortserver, 1);
}

/*
================
Host_Error

This shuts down both the client and server
================
*/
void Host_Error (const char *error, ...)
{
	va_list			argptr;
	char			string[1024];
	static qboolean inerror = false;

	if (inerror)
		Sys_Error ("Host_Error: recursively entered");
	inerror = true;

	va_start (argptr, error);
	q_vsnprintf (string, sizeof (string), error, argptr);
	va_end (argptr);

	Sys_DebugBreak ();

	if (!Sys_IsInDebugger ())
	{
		const char *captured_stack_trace = Sys_StackTrace ();

		Con_Printf ("================ STACK TRACE ================\n");

		// captured_stack_trace is long, split it into lines and Con_Printf each one
		size_t nb_lines = 0;
		char **stack_lines = q_strsplit ((char *)captured_stack_trace, "\r\n", &nb_lines);

		for (size_t line_index = 0; line_index < nb_lines; line_index++)
		{
			Con_Printf ("%s\n", stack_lines[line_index]);
		}

		Mem_Free (captured_stack_trace);
		Mem_Free (stack_lines);

		Con_Printf ("=============================================\n");
	}

	PR_SwitchQCVM (NULL);

	SCR_EndLoadingPlaque (); // reenable screen updates

	Con_Printf ("Host_Error: %s\n", string);

	if (cl.qcvm.extfuncs.CSQC_DrawHud && in_update_screen)
	{
		inerror = false;
		longjmp (screen_error, 1);
	}

	if (sv.active)
		Host_ShutdownServer (false);

	if (cls.state == ca_dedicated)
		Sys_Error ("Host_Error: %s\n", string); // dedicated servers exit

	CL_Disconnect ();
	cls.demonum = -1;
	cl.intermission = 0; // johnfitz -- for errors during intermissions (changelevel with no map found, etc.)

	inerror = false;

	longjmp (host_abortserver, 1);
}

/*
================
Host_Guard

ADR-009 rule 3: a longjmp must never unwind a Rust frame. Rust code that has
to call back into C which can Host_Error (the progs interpreter dispatching a
C builtin, Phase 6 M3) routes that call through here.

The guard installs its own longjmp targets, so Host_Error/Host_EndGame land
in this C frame instead of jumping past the Rust frames above it. It does NOT
re-run Host_Error -- that already did its work (shutting down the server,
disconnecting, printing) before it jumped, and doing it twice would be
observable. The caller instead re-issues the *same* jump from a pure C frame
once Rust has unwound normally, via Host_Reraise.

Nesting works one level at a time: an inner guard restores the outer's
buffers on the way out, so each re-issued jump is caught by the next guard
out until the outermost reaches host.c's own setjmp.
================
*/
int Host_Guard (void (*fn) (void *), void *arg)
{
	jmp_buf		 saved_abortserver;
	jmp_buf		 saved_screen_error;
	volatile int result;

	memcpy (saved_abortserver, host_abortserver, sizeof (jmp_buf));
	memcpy (saved_screen_error, screen_error, sizeof (jmp_buf));

	if (setjmp (host_abortserver))
	{
		result = HOST_GUARD_ABORTSERVER;
	}
	else if (setjmp (screen_error))
	{
		result = HOST_GUARD_SCREEN_ERROR;
	}
	else
	{
		result = HOST_GUARD_OK;
		fn (arg);
	}

	memcpy (host_abortserver, saved_abortserver, sizeof (jmp_buf));
	memcpy (screen_error, saved_screen_error, sizeof (jmp_buf));
	return result;
}

/*
================
Host_Reraise

Re-issues the jump a Host_Guard caught, from a pure C frame. Never returns
for a real guard result; HOST_GUARD_OK is a no-op so call sites can pass the
guard's return value through unconditionally.
================
*/
void Host_Reraise (int guard_result)
{
	switch (guard_result)
	{
	case HOST_GUARD_ABORTSERVER:
		longjmp (host_abortserver, 1);
	case HOST_GUARD_SCREEN_ERROR:
		longjmp (screen_error, 1);
	default:
		break;
	}
}
/*
 * 3. Guarded seams (ADR-009 rule 3). Each is enumerated by name rather than
 * reached through a generic function-pointer guard: naming them keeps the
 * Pattern A symbol audit meaningful and lets the compiler type-check every
 * call. The macros collapse only the boilerplate, not the enumeration.
 */

#define HOST_GUARD_VOID(name)                         \
	static void Host_Invoke_##name (void *p)          \
	{                                                 \
		(void)p;                                      \
		name ();                                      \
	}                                                 \
	int Host_Glue_##name (void)                       \
	{                                                 \
		return Host_Guard (Host_Invoke_##name, NULL); \
	}

#define HOST_GUARD_PTR(name, type)                 \
	static void Host_Invoke_##name (void *p)       \
	{                                              \
		name ((type)p);                            \
	}                                              \
	int Host_Glue_##name (void *a)                 \
	{                                              \
		return Host_Guard (Host_Invoke_##name, a); \
	}

#define HOST_GUARD_INT(name, type)                  \
	static void Host_Invoke_##name (void *p)        \
	{                                               \
		name ((type) * (int *)p);                   \
	}                                               \
	int Host_Glue_##name (int a)                    \
	{                                               \
		return Host_Guard (Host_Invoke_##name, &a); \
	}

/* host.c:746-751, :1005 -- world/model/sky/sound teardown and the CSQC world. */
HOST_GUARD_VOID (Mod_ClearAll)
HOST_GUARD_VOID (Sky_ClearAll)
HOST_GUARD_VOID (S_ClearAll)
HOST_GUARD_VOID (SV_ClearWorld)

/* host.c:858-898 -- the server frame entry points, all of which reach progs. */
HOST_GUARD_VOID (SV_ClearDatagram)
HOST_GUARD_VOID (SV_CheckForNewClients)
HOST_GUARD_VOID (SV_RunClients)
HOST_GUARD_VOID (SV_Physics)
HOST_GUARD_VOID (SV_SendClientMessages)

/* host.c:1110-1231 -- the _Host_Frame body's own calls. */
HOST_GUARD_VOID (Key_UpdateForDest)
HOST_GUARD_VOID (IN_UpdateInputMode)
HOST_GUARD_VOID (IN_Commands)
HOST_GUARD_VOID (Con_UpdateMouseState)
HOST_GUARD_VOID (Cbuf_Execute)
HOST_GUARD_VOID (NET_Poll)
HOST_GUARD_VOID (CL_AccumulateCmd)
HOST_GUARD_VOID (M_UpdateMouse)
HOST_GUARD_VOID (CL_SendCmd)
HOST_GUARD_VOID (CL_ReadFromServer)
HOST_GUARD_VOID (CL_RunParticles)
HOST_GUARD_VOID (CL_DecayLights)
HOST_GUARD_VOID (BGM_Update)
HOST_GUARD_VOID (CDAudio_Update)
HOST_GUARD_VOID (Harness_Frame)

/* host.c:676, :757 -- client teardown reached from the server shutdown path. */
HOST_GUARD_VOID (CL_Disconnect)
HOST_GUARD_VOID (CL_FreeState)

/* host.c:431 -- host_cmd.c's command registration. */
HOST_GUARD_VOID (Host_InitCommands)

/* host.c:1296-1404 -- Host_Init's one-shot subsystem bring-up. Guarding these
 * buys correct Rust unwinding only: a raise here longjmps into a host_abortserver
 * that _Host_Frame has not yet setjmp'd, which is pre-existing UB in the C build
 * (Host_Error diverts to Sys_Error first in the dedicated case). The guards do
 * not make that path recoverable and are not meant to. */
HOST_GUARD_VOID (Mem_Init)
HOST_GUARD_VOID (Tasks_Init)
HOST_GUARD_VOID (Cbuf_Init)
HOST_GUARD_VOID (Cmd_Init)
HOST_GUARD_VOID (Cvar_Init)
HOST_GUARD_VOID (COM_Init)
HOST_GUARD_VOID (COM_InitFilesystem)
HOST_GUARD_VOID (W_LoadWadFile)
HOST_GUARD_VOID (Key_Init)
HOST_GUARD_VOID (Con_Init)
HOST_GUARD_VOID (PR_Init)
HOST_GUARD_VOID (Mod_Init)
HOST_GUARD_VOID (NET_Init)
HOST_GUARD_VOID (SV_Init)
HOST_GUARD_VOID (V_Init)
HOST_GUARD_VOID (Chase_Init)
HOST_GUARD_VOID (M_Init)
HOST_GUARD_VOID (ExtraMaps_Init)
HOST_GUARD_VOID (M_CheckMods)
HOST_GUARD_VOID (Modlist_Init)
HOST_GUARD_VOID (DemoList_Init)
HOST_GUARD_VOID (SaveList_Init)
HOST_GUARD_VOID (VID_Init)
HOST_GUARD_VOID (IN_Init)
HOST_GUARD_VOID (TexMgr_Init)
HOST_GUARD_VOID (Draw_Init)
HOST_GUARD_VOID (SCR_Init)
HOST_GUARD_VOID (R_Init)
HOST_GUARD_VOID (S_Init)
HOST_GUARD_VOID (CDAudio_Init)
HOST_GUARD_VOID (BGM_Init)
HOST_GUARD_VOID (Sbar_Init)
HOST_GUARD_VOID (R_InitParticles)
HOST_GUARD_VOID (LOC_Init)
HOST_GUARD_VOID (Harness_Init)
HOST_GUARD_VOID (COM_WriteSelectedBaseDir)
HOST_GUARD_VOID (CL_Init)
/* The two optional subsystems keep an unconditional Host_Glue_* ABI: the Rust
 * port must link the same symbol set in every configuration, so when the C
 * macro is off the seam is a no-op that returns HOST_GUARD_OK and the call
 * disappears exactly as the #ifdef made it disappear in host.c. */
#ifdef PSET_SCRIPT
HOST_GUARD_VOID (PScript_InitParticles)
#else
int Host_Glue_PScript_InitParticles (void)
{
	return HOST_GUARD_OK;
}
#endif
#ifdef PR_TRACE
HOST_GUARD_VOID (PR_TraceInit)
HOST_GUARD_VOID (PR_TraceShutdown)
#else
int Host_Glue_PR_TraceInit (void)
{
	return HOST_GUARD_OK;
}

int Host_Glue_PR_TraceShutdown (void)
{
	return HOST_GUARD_OK;
}
#endif

/* host.c:1427-1455 -- Host_Shutdown's teardown. */
HOST_GUARD_VOID (Harness_Shutdown)
HOST_GUARD_VOID (NET_Shutdown)
HOST_GUARD_VOID (History_Shutdown)
HOST_GUARD_VOID (ExtraMaps_ShutDown)
HOST_GUARD_VOID (BGM_Shutdown)
HOST_GUARD_VOID (CDAudio_Shutdown)
HOST_GUARD_VOID (S_Shutdown)
HOST_GUARD_VOID (IN_Shutdown)
HOST_GUARD_VOID (VID_Shutdown)
HOST_GUARD_VOID (Steam_Shutdown)
HOST_GUARD_VOID (LOG_Close)
HOST_GUARD_VOID (LOC_Shutdown)

/* Pointer- and int-operand seams. */
HOST_GUARD_PTR (LOG_Init, quakeparms_t *)
HOST_GUARD_PTR (PR_ClearProgs, qcvm_t *)
HOST_GUARD_PTR (Key_WriteBindings, FILE *)
HOST_GUARD_PTR (Cvar_WriteVariables, FILE *)
HOST_GUARD_PTR (SVFTE_DestroyFrames, client_t *)
HOST_GUARD_PTR (NET_Close, struct qsocket_s *)
HOST_GUARD_INT (SCR_UpdateScreen, qboolean)
HOST_GUARD_INT (PR_ExecuteProgram, func_t)

/* Value-returning seams. The result is written through an out-parameter so the
 * int return stays the Host_Guard status; on a caught raise the out-parameter is
 * left untouched and the Rust core returns before observing it. */

typedef struct
{
	const char	*s;
	const char	*t;
	void		*p;
	void		*q;
	double		 d;
	int			 i;
	unsigned int u;
	size_t		 n;
	int			*out_i;
	void	   **out_p;
} host_arg_t;

static void Host_InvokeCvarSetQuick (void *p)
{
	host_arg_t *a = (host_arg_t *)p;
	Cvar_SetQuick ((cvar_t *)a->p, a->s);
}
int Host_Glue_CvarSetQuick (void *var, const char *value)
{
	host_arg_t a = {0};
	a.p = var;
	a.s = value;
	return Host_Guard (Host_InvokeCvarSetQuick, &a);
}

static void Host_InvokeCbufAddText (void *p)
{
	Cbuf_AddText (((host_arg_t *)p)->s);
}
int Host_Glue_CbufAddText (const char *text)
{
	host_arg_t a = {0};
	a.s = text;
	return Host_Guard (Host_InvokeCbufAddText, &a);
}

static void Host_InvokeCbufInsertText (void *p)
{
	Cbuf_InsertText (((host_arg_t *)p)->s);
}
int Host_Glue_CbufInsertText (const char *text)
{
	host_arg_t a = {0};
	a.s = text;
	return Host_Guard (Host_InvokeCbufInsertText, &a);
}

/* host.c:959-961 -- PR_LoadProgs always runs with fatal = false here. */
static void Host_InvokeLoadProgs (void *p)
{
	host_arg_t *a = (host_arg_t *)p;
	*a->out_i = PR_LoadProgs (a->s, false, a->u, (const builtin_t *)a->p, a->n);
}
int Host_Glue_PRLoadProgs (const char *filename, unsigned int needcrc, const void *builtins, size_t numbuiltins, int *out)
{
	host_arg_t a = {0};
	a.s = filename;
	a.u = needcrc;
	a.p = (void *)builtins;
	a.n = numbuiltins;
	a.out_i = out;
	return Host_Guard (Host_InvokeLoadProgs, &a);
}

static void Host_InvokeSetEngineString (void *p)
{
	host_arg_t *a = (host_arg_t *)p;
	*a->out_i = PR_SetEngineString (a->s);
}
int Host_Glue_PRSetEngineString (const char *s, int *out)
{
	host_arg_t a = {0};
	a.s = s;
	a.out_i = out;
	return Host_Guard (Host_InvokeSetEngineString, &a);
}

static void Host_InvokeEdictNum (void *p)
{
	host_arg_t *a = (host_arg_t *)p;
	*a->out_p = EDICT_NUM (a->i);
}
int Host_Glue_EdictNum (int n, void **out)
{
	host_arg_t a = {0};
	a.i = n;
	a.out_p = out;
	return Host_Guard (Host_InvokeEdictNum, &a);
}

static void Host_InvokeEdictToProg (void *p)
{
	host_arg_t *a = (host_arg_t *)p;
	*a->out_i = EDICT_TO_PROG ((edict_t *)a->p);
}
int Host_Glue_EdictToProg (void *e, int *out)
{
	host_arg_t a = {0};
	a.p = e;
	a.out_i = out;
	return Host_Guard (Host_InvokeEdictToProg, &a);
}

static void Host_InvokeNetCanSendMessage (void *p)
{
	host_arg_t *a = (host_arg_t *)p;
	*a->out_i = NET_CanSendMessage ((struct qsocket_s *)a->p);
}
int Host_Glue_NetCanSendMessage (void *sock, int *out)
{
	host_arg_t a = {0};
	a.p = sock;
	a.out_i = out;
	return Host_Guard (Host_InvokeNetCanSendMessage, &a);
}

static void Host_InvokeNetSendMessage (void *p)
{
	host_arg_t *a = (host_arg_t *)p;
	*a->out_i = NET_SendMessage ((struct qsocket_s *)a->p, (sizebuf_t *)a->q);
}
int Host_Glue_NetSendMessage (void *sock, void *data, int *out)
{
	host_arg_t a = {0};
	a.p = sock;
	a.q = data;
	a.out_i = out;
	return Host_Guard (Host_InvokeNetSendMessage, &a);
}

static void Host_InvokeNetGetMessage (void *p)
{
	host_arg_t *a = (host_arg_t *)p;
	*a->out_i = NET_GetMessage ((struct qsocket_s *)a->p);
}
int Host_Glue_NetGetMessage (void *sock, int *out)
{
	host_arg_t a = {0};
	a.p = sock;
	a.out_i = out;
	return Host_Guard (Host_InvokeNetGetMessage, &a);
}

static void Host_InvokeComLoadFile (void *p)
{
	host_arg_t *a = (host_arg_t *)p;
	*a->out_p = COM_LoadFile (a->s, NULL);
}
int Host_Glue_ComLoadFile (const char *path, void **out)
{
	host_arg_t a = {0};
	a.s = path;
	a.out_p = out;
	return Host_Guard (Host_InvokeComLoadFile, &a);
}

/* host.c:1212, :1216 -- S_Update takes four vec3_t. */
typedef struct
{
	const float *o, *f, *r, *u;
} host_supdate_arg_t;

static void Host_InvokeSUpdate (void *p)
{
	host_supdate_arg_t *a = (host_supdate_arg_t *)p;
	S_Update ((float *)a->o, (float *)a->f, (float *)a->r, (float *)a->u);
}
int Host_Glue_SUpdate (const float *origin, const float *forward, const float *right, const float *up)
{
	host_supdate_arg_t a;
	a.o = origin;
	a.f = forward;
	a.r = right;
	a.u = up;
	return Host_Guard (Host_InvokeSUpdate, &a);
}

/* Batched, guarded sizebuf writers, as in cl_main_glue.c:155-201. Every
 * MSG_Write* reaches SZ_GetSpace, which Host_Errors on overflow, so no Rust
 * frame may sit under one. host.c writes into four different targets
 * (host_client->message, svs.clients[i].message, client->message, cls.message),
 * so unlike cl_demo_glue.c's implicit net_message the batch takes the sizebuf
 * explicitly. Ops replay in insertion order, so the byte stream is identical
 * for any batch size. */

typedef struct
{
	int			kind;
	int			i;
	const void *p;
} host_write_t;

typedef struct
{
	sizebuf_t		   *sb;
	const host_write_t *ops;
	int					count;
} host_writebatch_arg_t;

static void Host_InvokeWriteBatch (void *p)
{
	host_writebatch_arg_t *a = (host_writebatch_arg_t *)p;
	int					   k;

	for (k = 0; k < a->count; k++)
	{
		const host_write_t *op = &a->ops[k];
		switch (op->kind)
		{
		case 0:
			MSG_WriteByte (a->sb, op->i);
			break;
		case 1:
			MSG_WriteShort (a->sb, op->i);
			break;
		case 2:
			MSG_WriteString (a->sb, (const char *)op->p);
			break;
		default:
			Sys_Error ("Host_InvokeWriteBatch: unknown op %i", op->kind);
			break;
		}
	}
}

int Host_Glue_WriteBatch (void *sb, const host_write_t *ops, int count)
{
	host_writebatch_arg_t arg;
	arg.sb = (sizebuf_t *)sb;
	arg.ops = ops;
	arg.count = count;
	return Host_Guard (Host_InvokeWriteBatch, &arg);
}

/* host.c:702-710 -- the disconnect broadcast writes into a stack-local
 * sizebuf_t backed by a four-byte array. Keeping the whole step in C preserves
 * that storage exactly rather than reconstructing a sizebuf on the Rust side. */
static void Host_InvokeBroadcastDisconnect (void *p)
{
	host_arg_t *a = (host_arg_t *)p;
	sizebuf_t	buf;
	byte		message[4];

	buf.data = message;
	buf.maxsize = 4;
	buf.cursize = 0;
	MSG_WriteByte (&buf, svc_disconnect);
	*a->out_i = NET_SendToAll (&buf, 5.0);
}
int Host_Glue_BroadcastDisconnect (int *out_count)
{
	host_arg_t a = {0};
	a.out_i = out_count;
	return Host_Guard (Host_InvokeBroadcastDisconnect, &a);
}

/*
 * 4. Entry points. Each calls the Rust core and re-issues, from this pure C
 * frame, whatever the core reported. host.c raises nothing of its own, so the
 * status is always a Host_Guard result and Host_Reraise handles it directly.
 */

void Host_FindMaxClients (void)
{
	Host_Reraise (quake_rs_host_find_max_clients ());
}

void Host_Version_f (void)
{
	Host_Reraise (quake_rs_host_version_f ());
}

void Host_Callback_Notify (cvar_t *var)
{
	Host_Reraise (quake_rs_host_callback_notify (var));
}

void Host_InitLocal (void)
{
	Host_Reraise (quake_rs_host_init_local ());
}

void Host_WriteConfiguration (void)
{
	Host_Reraise (quake_rs_host_write_configuration ());
}

/* host.c:522, :542, :569 -- the three variadic senders. The va_list formatting
 * stays here, exactly as host.c did it: the message text is produced by the C
 * library before any Rust frame exists, so ADR-005's formatter is not involved
 * and the 1024-byte truncation behaviour is preserved bit for bit. */

void SV_ClientPrintf (const char *fmt, ...)
{
	va_list argptr;
	char	string[1024];

	va_start (argptr, fmt);
	q_vsnprintf (string, sizeof (string), fmt, argptr);
	va_end (argptr);

	Host_Reraise (quake_rs_host_sv_client_printf (string));
}

void SV_BroadcastPrintf (const char *fmt, ...)
{
	va_list argptr;
	char	string[1024];

	va_start (argptr, fmt);
	q_vsnprintf (string, sizeof (string), fmt, argptr);
	va_end (argptr);

	Host_Reraise (quake_rs_host_sv_broadcast_printf (string));
}

void Host_ClientCommands (const char *fmt, ...)
{
	va_list argptr;
	char	string[1024];

	va_start (argptr, fmt);
	q_vsnprintf (string, sizeof (string), fmt, argptr);
	va_end (argptr);

	Host_Reraise (quake_rs_host_client_commands (string));
}

void SV_DropClient (qboolean crash)
{
	Host_Reraise (quake_rs_host_sv_drop_client (crash));
}

void Host_ShutdownServer (qboolean crash)
{
	Host_Reraise (quake_rs_host_shutdown_server (crash));
}

void Host_ClearMemory (void)
{
	Host_Reraise (quake_rs_host_clear_memory ());
}

/* host.c:773 -- provably raise-free (SDL_Delay and CLAMP only), so the core
 * returns the qboolean itself rather than a guard status. */
qboolean Host_FilterTime (float time)
{
	return quake_rs_host_filter_time (time);
}

void Host_GetConsoleCommands (void)
{
	Host_Reraise (quake_rs_host_get_console_commands ());
}

void Host_ServerFrame (void)
{
	Host_Reraise (quake_rs_host_server_frame ());
}

/*
 * host.c:1085-1094 -- the setjmp shell of _Host_Frame, the outermost longjmp
 * target in the engine. ADR-009 rule 3 forbids a longjmp crossing a Rust frame,
 * so this setjmp cannot move into Rust and stays here until Phase 9.
 *
 * No executable statement preceded the setjmp in host.c (only the static
 * accumulators, which are now Rust file statics), so the Rust core begins at
 * host.c:1096's COM_Rand and the control flow is identical: a guard inside the
 * core catches the raise, Rust unwinds normally, and Host_Reraise re-issues the
 * same jump from this pure C frame onto this frame's own setjmp, taking the
 * early return exactly as the C build did.
 */
void Host_Glue_FrameInner (double time)
{
	if (setjmp (host_abortserver))
		return; // something bad happened, or the server disconnected

	Host_Reraise (quake_rs_host_frame_core (time));
}

void Host_Frame (double time)
{
	Host_Reraise (quake_rs_host_frame (time));
}

void Host_Init (void)
{
	Host_Reraise (quake_rs_host_init ());
}

void Host_Shutdown (void)
{
	Host_Reraise (quake_rs_host_shutdown ());
}
