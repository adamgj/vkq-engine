/* Phase 7 M8 T8.1: Quake/host.c as a differential-oracle translation unit.
 *
 * WHY THIS FILE COMPOSES host.c INSTEAD OF build.rs LISTING IT IN C_SOURCES
 *
 * Every other oracle source is renamed to c_ref_* by the force-included
 * c_ref_prelude.h, whose macros are translation-unit-wide by construction:
 * one #define rewrites the definition in the oracle source AND every call in
 * all 43 other oracle sources. That is exactly right for the sv_ and cl_
 * strata. It is wrong for host.c, because four of the symbols host.c owns are
 * the ADR-009 trap machinery this harness substitutes for: Host_Error
 * (stubs.c), Host_EndGame (stubs.c), Host_Guard and Host_Reraise (stubs.c,
 * with the harness's own CTEST_GUARD_ result set and a documented departure
 * from the real HOST_GUARD_ semantics). Nine other oracle sources contain 69
 * calls to Host_Error/Host_EndGame, and 19 test files assert on what the trap
 * recorded; a prelude-wide rename would silently repoint all of them at
 * host.c's real Host_Error, which walks a stack trace, disconnects the client,
 * shuts the server down and longjmps to a host_abortserver no test has set.
 *
 * So the rename layer for host.c lives HERE, in host.c's own translation unit,
 * where it renames host.c's definitions and host.c's internal calls and
 * nothing else. Every other TU keeps the harness trap under the plain names.
 * The engine's own build is untouched either way -- this is oracle plumbing.
 *
 * Composing a .c into another TU is an established shape in this build
 * (Quake/common_fs.c does it with miniz.c, Quake/image_stb.c with
 * stb_image.h), and it has a second benefit the separate-TU form cannot give:
 * host.c's file-statics stay visible to the fixture drivers at the bottom.
 *
 * COST, stated so it is not discovered later: scripts/harness/
 * check_ctest_symbols.sh reads C_SOURCES out of build.rs, so it does not
 * inspect this object. The rename list below is verified instead by an
 * llvm-nm --defined-only sweep recorded in the task handoff.
 */

/* host.c #includes these beyond quakedef.h (which the prelude neuters by
 * defining QUAKEDEFS_H). Each is turned off by defining its own include guard,
 * exactly as the prelude turns off q_stdinc.h with __QSTDINC_H:
 *  - tasks.h  (#include "q_stdinc.h" -> SDL.h; ADR-016 keeps tasks.c C until
 *              Phase 8). The prelude already supplies task_handle_t.
 *  - gl_heap.h (#include "q_render_types.h" -> Vulkan) -- only reached under
 *              _DEBUG, and only for GL_HeapTest_f's prototype.
 *  - bgmusic.h / steam.h are include-clean, so they are left alone. */
#define __TASKS_H
#define __HEAP__

/* tasks.h:29 and the two entry points host.c calls from the neutered header. */
#define INVALID_TASK_HANDLE UINT64_MAX

/* ---- host.c rename block ------------------------------------------------
 * Every non-static file-scope symbol Quake/host.c defines, EXCEPT the three
 * called out below. A miss here is a duplicate-definition link error against
 * stubs.c rather than a silent override, because stubs.c already owns
 * plain-named copies of most of them.
 *
 * NOT RENAMED, and this is forced rather than chosen: max_edicts (host.c:76),
 * deathmatch (host.c:88) and coop (host.c:89) are also the spellings of
 * struct members host.c reaches -- qcvm->max_edicts (host.c:888, :963, :964,
 * progs.h) and pr_global_struct->deathmatch / ->coop (host.c:990, :991,
 * progs.h). An object-like #define rewrites those member accesses too, and
 * progs.h is a real engine header this harness mirrors rather than owns, so
 * there is no place to add an alias. These three therefore stay plain here and
 * host_ref.o supplies them for the whole link; the fake copies stubs.c used to
 * define were deleted in the same change. That is a straight upgrade in
 * fidelity for the other TUs -- they now read the real host.c cvar_t objects,
 * with the real "32000"/"0"/"0" defaults -- but it does move each one's
 * runtime .value from stubs.c's pre-seeded number to 0 until something
 * registers or sets it, so any test that cared sets it explicitly.
 */

/* data */
#define host_parms			  c_ref_host_parms
#define host_initialized	  c_ref_host_initialized
#define host_frametime		  c_ref_host_frametime
#define host_rawframetime	  c_ref_host_rawframetime
#define realtime			  c_ref_realtime
#define oldrealtime			  c_ref_oldrealtime
#define host_framecount		  c_ref_host_framecount
#define minimum_memory		  c_ref_minimum_memory
#define host_client			  c_ref_host_client
#define host_abortserver	  c_ref_host_abortserver
#define screen_error		  c_ref_screen_error
#define host_colormap		  c_ref_host_colormap
#define host_netinterval	  c_ref_host_netinterval
#define host_framerate		  c_ref_host_framerate
#define host_speeds			  c_ref_host_speeds
#define sv_speeds			  c_ref_sv_speeds
#define host_maxfps			  c_ref_host_maxfps
#define host_phys_max_ticrate c_ref_host_phys_max_ticrate
#define host_timescale		  c_ref_host_timescale
#define cl_nocsqc			  c_ref_cl_nocsqc
#define sys_ticrate			  c_ref_sys_ticrate
#define serverprofile		  c_ref_serverprofile
#define fraglimit			  c_ref_fraglimit
#define timelimit			  c_ref_timelimit
#define teamplay			  c_ref_teamplay
#define samelevel			  c_ref_samelevel
#define noexit				  c_ref_noexit
#define skill				  c_ref_skill
#define pausable			  c_ref_pausable
#define autoload			  c_ref_autoload
#define autofastload		  c_ref_autofastload
#define developer			  c_ref_developer
#define temp1				  c_ref_temp1
#define devstats			  c_ref_devstats
#define campaign			  c_ref_campaign
#define horde				  c_ref_horde
#define sv_cheats			  c_ref_sv_cheats
#define dev_stats			  c_ref_dev_stats
#define dev_peakstats		  c_ref_dev_peakstats
#define dev_overflows		  c_ref_dev_overflows

/* functions */
#define Host_EndGame			c_ref_Host_EndGame
#define Host_Error				c_ref_Host_Error
#define Host_Guard				c_ref_Host_Guard
#define Host_Reraise			c_ref_Host_Reraise
#define Host_FindMaxClients		c_ref_Host_FindMaxClients
#define Host_Version_f			c_ref_Host_Version_f
#define Host_Callback_Notify	c_ref_Host_Callback_Notify
#define Host_InitLocal			c_ref_Host_InitLocal
#define Host_WriteConfiguration c_ref_Host_WriteConfiguration
#define SV_ClientPrintf			c_ref_SV_ClientPrintf
#define SV_BroadcastPrintf		c_ref_SV_BroadcastPrintf
#define Host_ClientCommands		c_ref_Host_ClientCommands
#define SV_DropClient			c_ref_SV_DropClient
#define Host_ShutdownServer		c_ref_Host_ShutdownServer
#define Host_ClearMemory		c_ref_Host_ClearMemory
#define Host_FilterTime			c_ref_Host_FilterTime
#define Host_GetConsoleCommands c_ref_Host_GetConsoleCommands
#define Host_ServerFrame		c_ref_Host_ServerFrame
#define Host_Frame				c_ref_Host_Frame
#define Host_Init				c_ref_Host_Init
#define Host_Shutdown			c_ref_Host_Shutdown

/* server.h:361-362 and server.h's Host_ShutdownServer declaration were already
 * seen by the preprocessor under their plain names (the prelude force-includes
 * them), so the renamed definitions below have no visible prototype and
 * host.c's earlier calls (host.c:198, :419) would fall back to implicit int.
 * Re-declare the two under the renamed spelling; the signatures are copied
 * from server.h:361-362 and host.c:661. */
void SV_ClientPrintf (const char *fmt, ...) FUNC_PRINTF (1, 2);
void SV_BroadcastPrintf (const char *fmt, ...) FUNC_PRINTF (1, 2);
void Host_ShutdownServer (qboolean crash);

/* ---- link doubles for the sub-systems host.c starts up ------------------
 * host.c is the engine's top-level wiring, so composing it drags in every
 * sub-system initialiser. None of these live in an oracle source and none is
 * declared by a header in this slice. They are defined here rather than in
 * stubs.c because host.c (and, later, host_cmd.c) is the only caller.
 *
 * Two rules from the earlier milestones apply. First, a double that a test
 * could actually reach must count or record, never silently return: the only
 * one Host_FilterTime (host.c:773) reaches is SDL_Delay, so that one is a
 * recorder. Second, a double on a path no test drives aborts loudly rather
 * than pretending to succeed, because a wrong answer there would be
 * indistinguishable from a real one. Everything below is only reachable from
 * Host_Init (host.c:1288), Host_Shutdown (host.c:1424) or Host_Frame's render
 * half, none of which this suite drives.
 */

/* Real SDL_Delay sleeps ms milliseconds and returns nothing; host.c:791 calls
 * it with 1 purely to yield when the frame is early. Sleeping is not
 * observable state, so the faithful harness substitute is to record that the
 * call happened and with what argument -- that is exactly the branch
 * Host_FilterTime's "(min_frame_time - delta) > 2/1000" test selects. */
static int			ctest_sdl_delay_calls = 0;
static unsigned int ctest_sdl_delay_last_ms = 0;

void SDL_Delay (unsigned int ms)
{
	ctest_sdl_delay_calls++;
	ctest_sdl_delay_last_ms = ms;
}

static void ctest_host_unreached (const char *who)
{
	Sys_Error ("host_ref.c: %s reached; it is a link double with no behaviour", who);
}

/* Recorders, not aborts, because a driven path reaches them. */

/* sys.h:156. The real Sys_ConsoleInput returns NULL when the console has
 * nothing pending, which is what Host_GetConsoleCommands (host.c:836) sees on
 * every frame that the user did not type. Returning NULL is therefore the
 * faithful answer for a harness with no console, not an evasion. */
static int ctest_sys_console_input_calls = 0;

const char *Sys_ConsoleInput (void)
{
	ctest_sys_console_input_calls++;
	return NULL;
}

/* keys.h:173. The real Key_WriteBindings (keys.c) emits one bind line per
 * bound key and nothing at all for an empty binding table; the harness never
 * binds a key, so writing nothing is what the real function would do here.
 * The call is still counted so Host_WriteConfiguration's ordering is
 * observable. */
static int ctest_key_write_bindings_calls = 0;

void Key_WriteBindings (FILE *f)
{
	(void)f;
	ctest_key_write_bindings_calls++;
}

/* sys.h:169-175. Host_Error (host.c:218) calls all three. Sys_IsInDebugger
 * answers true, which is a real runtime state and the one under which
 * host.c:231-251 -- a block whose only effect is Con_Printf output -- is
 * skipped; Sys_StackTrace and q_strsplit are consequently unreachable and
 * abort rather than fabricate a trace. */
static int ctest_sys_debug_break_calls = 0;

void Sys_DebugBreak (void)
{
	ctest_sys_debug_break_calls++;
}

bool Sys_IsInDebugger (void)
{
	return true;
}

const char *Sys_StackTrace (void)
{
	ctest_host_unreached ("Sys_StackTrace");
	return NULL;
}

char **q_strsplit (char *str, const char *sep_set, size_t *nb_substr)
{
	(void)str;
	(void)sep_set;
	(void)nb_substr;
	ctest_host_unreached ("q_strsplit");
	return NULL;
}

/* Aborting doubles: Host_Init (host.c:1288), Host_Shutdown (host.c:1424),
 * Host_ClearMemory (host.c:735) and _Host_Frame's render half only. */
void Mem_Init (void) { ctest_host_unreached ("Mem_Init"); }
void COM_Init (void) { ctest_host_unreached ("COM_Init"); }
void Mod_Init (void) { ctest_host_unreached ("Mod_Init"); }
void Mod_ClearAll (void) { ctest_host_unreached ("Mod_ClearAll"); }
void PR_Init (void) { ctest_host_unreached ("PR_Init"); }
void NET_Init (void) { ctest_host_unreached ("NET_Init"); }
void NET_Shutdown (void) { ctest_host_unreached ("NET_Shutdown"); }
void NET_Poll (void) { ctest_host_unreached ("NET_Poll"); }
void VID_Init (void) { ctest_host_unreached ("VID_Init"); }
void VID_Shutdown (void) { ctest_host_unreached ("VID_Shutdown"); }
void Con_Init (void) { ctest_host_unreached ("Con_Init"); }
void Con_UpdateMouseState (void) { ctest_host_unreached ("Con_UpdateMouseState"); }
void LOG_Init (quakeparms_t *parms) { (void)parms, ctest_host_unreached ("LOG_Init"); }
void LOG_Close (void) { ctest_host_unreached ("LOG_Close"); }
void SCR_Init (void) { ctest_host_unreached ("SCR_Init"); }
void SCR_UpdateScreen (qboolean use_tasks) { (void)use_tasks, ctest_host_unreached ("SCR_UpdateScreen"); }
void Key_Init (void) { ctest_host_unreached ("Key_Init"); }
void Sys_SendKeyEvents (void) { ctest_host_unreached ("Sys_SendKeyEvents"); }
void Key_UpdateForDest (void) { ctest_host_unreached ("Key_UpdateForDest"); }
void History_Shutdown (void) { ctest_host_unreached ("History_Shutdown"); }
void IN_Init (void) { ctest_host_unreached ("IN_Init"); }
void IN_Shutdown (void) { ctest_host_unreached ("IN_Shutdown"); }
void IN_Commands (void) { ctest_host_unreached ("IN_Commands"); }
void IN_UpdateInputMode (void) { ctest_host_unreached ("IN_UpdateInputMode"); }
int	 CDAudio_Init (void)
{
	ctest_host_unreached ("CDAudio_Init");
	return 0;
}
void CDAudio_Shutdown (void) { ctest_host_unreached ("CDAudio_Shutdown"); }
void CDAudio_Update (void) { ctest_host_unreached ("CDAudio_Update"); }
void Harness_Init (void) { ctest_host_unreached ("Harness_Init"); }
void Harness_Frame (void) { ctest_host_unreached ("Harness_Frame"); }
void Harness_Shutdown (void) { ctest_host_unreached ("Harness_Shutdown"); }
void Steam_Shutdown (void) { ctest_host_unreached ("Steam_Shutdown"); }
void Steam_SetStatus_Menu (void) { ctest_host_unreached ("Steam_SetStatus_Menu"); }
void Steam_SetStatus_SinglePlayer (const char *map) { (void)map, ctest_host_unreached ("Steam_SetStatus_SinglePlayer"); }
void Steam_SetStatus_Multiplayer (int players, int maxplayers, const char *map)
{
	(void)players, (void)maxplayers, (void)map;
	ctest_host_unreached ("Steam_SetStatus_Multiplayer");
}

/* Data host.c reads on the paths above. progs.h:440-441 (the csqc builtin
 * table, matching stubs.c:6931-6932's ssqc pair), render.h:201, console.h:32,
 * screen.h:49, harness.h:52 and glquake.h:543. */
const builtin_t pr_csqcbuiltins[1] = {NULL};
const int		pr_csqcnumbuiltins = 0;
vec3_t			r_origin, vup, vright;
qboolean		con_initialized = false;
qboolean		scr_disabled_for_loading = false;
qboolean		no_rendering = true;
qboolean		in_update_screen = false;

void Host_InitCommands (void) { ctest_host_unreached ("Host_InitCommands"); }
void Sky_ClearAll (void) { ctest_host_unreached ("Sky_ClearAll"); }
void M_UpdateMouse (void) { ctest_host_unreached ("M_UpdateMouse"); }
void CL_RunParticles (void) { ctest_host_unreached ("CL_RunParticles"); }
void Tasks_Init (void) { ctest_host_unreached ("Tasks_Init"); }
void M_Init (void) { ctest_host_unreached ("M_Init"); }
void M_CheckMods (void) { ctest_host_unreached ("M_CheckMods"); }
void ExtraMaps_Init (void) { ctest_host_unreached ("ExtraMaps_Init"); }
void ExtraMaps_ShutDown (void) { ctest_host_unreached ("ExtraMaps_ShutDown"); }
void Modlist_Init (void) { ctest_host_unreached ("Modlist_Init"); }
void DemoList_Init (void) { ctest_host_unreached ("DemoList_Init"); }
void SaveList_Init (void) { ctest_host_unreached ("SaveList_Init"); }
void TexMgr_Init (void) { ctest_host_unreached ("TexMgr_Init"); }
void Draw_Init (void) { ctest_host_unreached ("Draw_Init"); }
void R_Init (void) { ctest_host_unreached ("R_Init"); }
void Sbar_Init (void) { ctest_host_unreached ("Sbar_Init"); }
void R_InitParticles (void) { ctest_host_unreached ("R_InitParticles"); }
void PScript_InitParticles (void) { ctest_host_unreached ("PScript_InitParticles"); }

/* tasks.h:52. host.c:1188 asks whether the caller is a worker thread; the
 * harness is single-threaded, so the honest answer is false. */
qboolean Tasks_IsWorker (void)
{
	return false;
}

#include "host.c"

/* ---- fixture drivers ----------------------------------------------------
 * Named ctest_host_* and deliberately NOT renamed: they are harness entry
 * points, not engine symbols.
 */

void ctest_host_reset_time (double realtime_in, double oldrealtime_in)
{
	c_ref_realtime = realtime_in;
	c_ref_oldrealtime = oldrealtime_in;
	c_ref_host_frametime = 0.0;
	c_ref_host_rawframetime = 0.0;
}

void ctest_host_set_maxfps (float value)
{
	c_ref_host_maxfps.value = value;
}

void ctest_host_set_timescale (float value)
{
	c_ref_host_timescale.value = value;
}

void ctest_host_set_framerate (float value)
{
	c_ref_host_framerate.value = value;
}

void ctest_host_set_demo (int demoplayback, float demospeed, int timedemo)
{
	cls.demoplayback = demoplayback ? true : false;
	cls.demospeed = demospeed;
	cls.timedemo = timedemo ? true : false;
}

int ctest_host_filter_time (float time)
{
	return c_ref_Host_FilterTime (time) ? 1 : 0;
}

double ctest_host_get_realtime (void)
{
	return c_ref_realtime;
}

double ctest_host_get_oldrealtime (void)
{
	return c_ref_oldrealtime;
}

double ctest_host_get_frametime (void)
{
	return c_ref_host_frametime;
}

double ctest_host_get_rawframetime (void)
{
	return c_ref_host_rawframetime;
}

float ctest_host_get_netinterval (void)
{
	return c_ref_host_netinterval;
}

void ctest_host_set_netinterval (float value)
{
	c_ref_host_netinterval = value;
}

/* Max_Fps_f (host.c:131) and Phys_Ticrate_f (host.c:162) are the two cvar
 * callbacks that own host_netinterval; both are file-static, which is why the
 * drivers live inside this TU. */
void ctest_host_max_fps_f (float value)
{
	c_ref_host_maxfps.value = value;
	Max_Fps_f (&c_ref_host_maxfps);
}

void ctest_host_phys_ticrate_f (float value)
{
	c_ref_host_phys_max_ticrate.value = value;
	Phys_Ticrate_f (&c_ref_host_phys_max_ticrate);
}

void ctest_host_callback_notify (cvar_t *var)
{
	c_ref_Host_Callback_Notify (var);
}

void ctest_host_client_commands (const char *text)
{
	c_ref_Host_ClientCommands ("%s", text);
}

void ctest_host_sv_client_printf (const char *text)
{
	c_ref_SV_ClientPrintf ("%s", text);
}

void ctest_host_sv_broadcast_printf (const char *text)
{
	c_ref_SV_BroadcastPrintf ("%s", text);
}

void ctest_host_set_host_client (int index)
{
	c_ref_host_client = svs.clients + index;
}

void ctest_host_find_max_clients (void)
{
	c_ref_Host_FindMaxClients ();
}

void ctest_host_write_configuration (void)
{
	c_ref_Host_WriteConfiguration ();
}

void ctest_host_set_initialized (int value)
{
	c_ref_host_initialized = value ? true : false;
}

int ctest_host_sdl_delay_calls (void)
{
	return ctest_sdl_delay_calls;
}

unsigned int ctest_host_sdl_delay_last_ms (void)
{
	return ctest_sdl_delay_last_ms;
}

void ctest_host_sdl_delay_reset (void)
{
	ctest_sdl_delay_calls = 0;
	ctest_sdl_delay_last_ms = 0;
}

/* A self-contained server-client fixture, the same shape sv_send_ref.c uses:
 * a static client_t array with real sizebuf_t message buffers, published into
 * the renamed `svs` host.c reads. SV_ClientPrintf (host.c:522),
 * SV_BroadcastPrintf (host.c:542) and Host_ClientCommands (host.c:569) write
 * through MSG_WriteByte/MSG_WriteString into those buffers, so the bytes are
 * the observable. */
#define CTEST_HOST_CLIENTS 4
#define CTEST_HOST_MSGMAX  1024

static client_t ctest_host_clients[CTEST_HOST_CLIENTS];
static byte		ctest_host_msgbuf[CTEST_HOST_CLIENTS][CTEST_HOST_MSGMAX];

void ctest_host_reset_clients (int maxclients)
{
	int i;

	memset (ctest_host_clients, 0, sizeof (ctest_host_clients));
	memset (ctest_host_msgbuf, 0, sizeof (ctest_host_msgbuf));
	for (i = 0; i < CTEST_HOST_CLIENTS; i++)
	{
		ctest_host_clients[i].message.data = ctest_host_msgbuf[i];
		ctest_host_clients[i].message.maxsize = CTEST_HOST_MSGMAX;
		ctest_host_clients[i].active = true;
		ctest_host_clients[i].spawned = true;
		q_snprintf (ctest_host_clients[i].name, sizeof (ctest_host_clients[i].name), "player%i", i);
	}

	svs.clients = ctest_host_clients;
	svs.maxclients = maxclients;
	svs.maxclientslimit = CTEST_HOST_CLIENTS;
	c_ref_host_client = &ctest_host_clients[0];
}

void ctest_host_set_client_state (int index, int active, int spawned)
{
	ctest_host_clients[index].active = active ? true : false;
	ctest_host_clients[index].spawned = spawned ? true : false;
}

int ctest_host_client_msg_len (int index)
{
	return ctest_host_clients[index].message.cursize;
}

int ctest_host_client_msg_byte (int index, int offset)
{
	return ctest_host_msgbuf[index][offset];
}

void ctest_host_set_sv_active (int value)
{
	sv.active = value ? true : false;
}

int ctest_host_get_maxclients (void)
{
	return svs.maxclients;
}

/* host.c:492 guards on host_initialized, isDedicated and host_parms->errstate;
 * the fixture owns the quakeparms_t so the third is settable. */
static quakeparms_t ctest_host_parms;

void ctest_host_set_parms (int errstate)
{
	memset (&ctest_host_parms, 0, sizeof (ctest_host_parms));
	ctest_host_parms.errstate = errstate;
	c_ref_host_parms = &ctest_host_parms;
}

int ctest_host_key_write_bindings_calls (void)
{
	return ctest_key_write_bindings_calls;
}

/* Host_FindMaxClients (host.c:357) reads the command line through
 * COM_CheckParm, which the harness backs with stubs.c's com_argc/com_argv
 * (stubs.c:1242) and its ctest_set_args setter. Wrapping it here keeps the
 * char ** marshalling out of the test. */
extern void ctest_set_args (int argc, char **argv);

void ctest_host_set_cmdline (const char *a0, const char *a1, const char *a2)
{
	static char *argv[3];
	int			 argc = 0;

	if (a0)
		argv[argc++] = (char *)a0;
	if (a1)
		argv[argc++] = (char *)a1;
	if (a2)
		argv[argc++] = (char *)a2;
	ctest_set_args (argc, argv);
}

float ctest_host_get_deathmatch (void)
{
	return deathmatch.value;
}

int ctest_host_get_cls_state (void)
{
	return (int)cls.state;
}

void ctest_host_set_phys_max_ticrate (float value)
{
	c_ref_host_phys_max_ticrate.value = value;
}

float ctest_host_get_phys_max_ticrate (void)
{
	return c_ref_host_phys_max_ticrate.value;
}

/* stubs.c:797 leaves harness_active true, so COM_FOpenPrefFile
 * (common_fs.c:776) writes into com_gamedir; Host_WriteConfiguration
 * (host.c:487) therefore needs the fixture to own that directory. */
void ctest_host_set_gamedir (const char *dir)
{
	q_strlcpy (com_gamedir, dir, sizeof (com_gamedir));
}

/* Cvar_SetQuick (cvar.c) returns early unless CVAR_REGISTERED is set, so
 * Host_FindMaxClients' deathmatch write (host.c:395) is only observable once
 * the cvar has been through Cvar_RegisterVariable -- which is what
 * Host_InitLocal (host.c:445) does at startup. Idempotent: registering twice
 * is a Sys_Error in cvar.c. */
void ctest_host_register_deathmatch (void)
{
	static qboolean done = false;
	if (done)
		return;
	done = true;
	Cvar_RegisterVariable (&deathmatch);
}

const char *ctest_host_get_deathmatch_string (void)
{
	return deathmatch.string;
}
