/* Phase 7 M8 T8.1: Quake/host_cmd.c as a differential-oracle translation unit.
 *
 * Same construction as stubs/host_ref.c, and for the same reason -- read that
 * file's header comment first; it explains why the c_ref_ rename layer for the
 * host stratum lives inside the translation unit instead of in
 * c_ref_prelude.h, and what that costs in check_ctest_symbols.sh coverage.
 *
 * The subjects this TU exists for are Host_SavegameComment (host_cmd.c:1519),
 * Host_Savegame_f (host_cmd.c:1554) and Host_Loadgame_f (host_cmd.c:1797).
 * All three are file-static, so composing host_cmd.c here is not a
 * convenience: it is the only way to drive them at all.
 */

/* host_cmd.c #includes tasks.h transitively through quakedef.h, which the
 * prelude neuters; see host_ref.c for the guard-define idiom. gl_heap.h is
 * turned off the same way. */
#define __TASKS_H
#define __HEAP__
#define INVALID_TASK_HANDLE UINT64_MAX

/* ---- host_cmd.c rename block --------------------------------------------
 * Every non-static file-scope symbol Quake/host_cmd.c defines. host_cmd.c:43
 * also forward-declares Mod_Print, which gl_model.c owns; it is a declaration,
 * not a definition, so it is not renamed here.
 */

/* data */
#define current_skill	   c_ref_current_skill
#define extralevels		   c_ref_extralevels
#define extralevels_sorted c_ref_extralevels_sorted
#define modlist			   c_ref_modlist
#define demolist		   c_ref_demolist
#define savelist		   c_ref_savelist
#define noclip_anglehack   c_ref_noclip_anglehack

/* functions */
#define Host_Quit_f			 c_ref_Host_Quit_f
#define ExtraMaps_GetType	 c_ref_ExtraMaps_GetType
#define ExtraMaps_GetMessage c_ref_ExtraMaps_GetMessage
#define ExtraMaps_IsStart	 c_ref_ExtraMaps_IsStart
#define ExtraMaps_Init		 c_ref_ExtraMaps_Init
#define ExtraMaps_Clear		 c_ref_ExtraMaps_Clear
#define ExtraMaps_ShutDown	 c_ref_ExtraMaps_ShutDown
#define ExtraMaps_NewGame	 c_ref_ExtraMaps_NewGame
#define Modlist_GetFullName	 c_ref_Modlist_GetFullName
#define Modlist_Init		 c_ref_Modlist_Init
#define DemoList_Rebuild	 c_ref_DemoList_Rebuild
#define DemoList_Init		 c_ref_DemoList_Init
#define SaveList_Rebuild	 c_ref_SaveList_Rebuild
#define SaveList_Init		 c_ref_SaveList_Init
#define Host_Resetdemos		 c_ref_Host_Resetdemos
#define Host_InitCommands	 c_ref_Host_InitCommands

/* The prelude force-includes quakedef.h's declarations for all of these under
 * their plain names, so the renamed definitions below would have no visible
 * prototype and host_cmd.c's earlier internal calls would fall back to
 * implicit int (host_cmd.c:828, :850 hit exactly that). Re-declare them under
 * the renamed spelling; signatures copied from quakedef.h:454-461 and
 * quakedef.h:482-497. */
maptype_t	ExtraMaps_GetType (const filelist_item_t *item);
qboolean	ExtraMaps_IsStart (maptype_t type);
const char *ExtraMaps_GetMessage (const filelist_item_t *item);
const char *Modlist_GetFullName (const filelist_item_t *item);
void		Host_Quit_f (void);
void		Host_Resetdemos (void);
void		ExtraMaps_Init (void);
void		Modlist_Init (void);
void		DemoList_Init (void);
void		SaveList_Init (void);
void		ExtraMaps_NewGame (void);
void		ExtraMaps_Clear (void);
void		ExtraMaps_ShutDown (void);
void		DemoList_Rebuild (void);
void		SaveList_Rebuild (void);
void		Host_InitCommands (void);

/* ---- host.c cross-TU rename block ---------------------------------------
 * host_cmd.c reads six file-scope symbols that Quake/host.c defines, and
 * host_ref.c has renamed every one of those definitions to c_ref_*. Left
 * plain here they would either bind to an unrelated stubs.c double
 * (host_client, stubs.c:808) or not resolve at all (SV_ClientPrintf,
 * SV_BroadcastPrintf, pausable, autoload and autofastload have no plain
 * definition anywhere in the ctest link). Renaming them here reunites the two
 * halves of the host stratum on the one object the real engine has.
 *
 * Deliberately NOT renamed, so that they keep binding to their stubs.c
 * doubles: Host_Error (stubs.c:1398 is the armable trap the tests observe),
 * SV_DropClient (stubs.c:6943), Host_ShutdownServer (stubs.c:7737), skill
 * (stubs.c:6923) and teamplay (stubs.c:5786).
 */
#define host_client		   c_ref_host_client
#define pausable		   c_ref_pausable
#define autoload		   c_ref_autoload
#define autofastload	   c_ref_autofastload
#define SV_ClientPrintf	   c_ref_SV_ClientPrintf
#define SV_BroadcastPrintf c_ref_SV_BroadcastPrintf

/* server.h was force-included under the plain spellings before the renames
 * above took effect, so the renamed names have no declaration yet. Re-declare
 * them; copied from server.h:329 and server.h:361-362. pausable, autoload and
 * autofastload need no such line because host_cmd.c:34-37 declares them
 * itself, after the rename. */
extern client_t *host_client;
void			 SV_ClientPrintf (const char *fmt, ...) FUNC_PRINTF (1, 2);
void			 SV_BroadcastPrintf (const char *fmt, ...) FUNC_PRINTF (1, 2);

#include "host_cmd.c"

/* ---- fixture drivers ----------------------------------------------------
 * Named ctest_hostcmd_* and deliberately NOT renamed: harness entry points.
 */

void ctest_hostcmd_savegame_comment (char *out)
{
	Host_SavegameComment (out);
}

void ctest_hostcmd_savegame_f (void)
{
	Host_Savegame_f ();
}

void ctest_hostcmd_loadgame_f (void)
{
	Host_Loadgame_f ();
}

int ctest_hostcmd_get_current_skill (void)
{
	return c_ref_current_skill;
}

void ctest_hostcmd_set_current_skill (int value)
{
	c_ref_current_skill = value;
}


/* ---- savegame fixture ----------------------------------------------------
 * Host_Savegame_f (host_cmd.c:1554) walks a whole single-player server: an
 * active sv carrying a qcvm, one live client whose edict has positive health,
 * the lightstyle and precache tables, and the extended comment block.
 * Everything below is the smallest such server that still reaches every
 * fprintf in the writer, so the golden file the test pins covers the whole
 * format.
 *
 * The qcvm is assembled in place inside sv.qcvm rather than borrowed from
 * stubs.c's ctest_progs_* fixtures, because the writer hardcodes
 * PR_SwitchQCVM (&sv.qcvm) (host_cmd.c:1626).
 */

#define CTEST_SAVE_EDICTS  2
#define CTEST_SAVE_EDSIZE  ((int)sizeof (edict_t) + 256)

/* ddef_t::s_name and ::ofs are offsets into these two blobs; ofs counts
 * 4-byte slots, the unit progs.h uses. */
static char		   ctest_save_strings[] = "\0gsave\0health\0";
static ddef_t	   ctest_save_globaldefs[2];
static ddef_t	   ctest_save_fielddefs[2];
static dprograms_t ctest_save_progs;
/* sized well past globalvars_t: PR_SwitchQCVM republishes this block as
 * pr_global_struct (stubs.c:3094). */
static float	   ctest_save_globals[1024];
static byte		  *ctest_save_edicts;
static client_t	   ctest_save_client;

static edict_t *ctest_save_edict (int n)
{
	return (edict_t *)(ctest_save_edicts + (size_t)n * (size_t)CTEST_SAVE_EDSIZE);
}

void ctest_hostcmd_setup_savegame (const char *gamedir, const char *levelname, const char *mapname, int monsters, int totalmonsters, int skill_value, float qctime)
{
	qcvm_t *vm = &sv.qcvm;
	int		i;

	if (!ctest_save_edicts)
		ctest_save_edicts = (byte *)malloc ((size_t)CTEST_SAVE_EDICTS * (size_t)CTEST_SAVE_EDSIZE);

	memset (&sv, 0, sizeof (sv));
	memset (&cl, 0, sizeof (cl));
	memset (&ctest_save_client, 0, sizeof (ctest_save_client));
	memset (ctest_save_edicts, 0, (size_t)CTEST_SAVE_EDICTS * (size_t)CTEST_SAVE_EDSIZE);
	memset (ctest_save_globals, 0, sizeof (ctest_save_globals));

	q_strlcpy (com_gamedir, gamedir, MAX_OSPATH);
	q_strlcpy (cl.levelname, levelname, sizeof (cl.levelname));
	cl.stats[STAT_MONSTERS] = monsters;
	cl.stats[STAT_TOTALMONSTERS] = totalmonsters;
	cl.intermission = 0;

	sv.active = true;
	sv.nomonsters = false;
	q_strlcpy (sv.name, mapname, sizeof (sv.name));
	sv.lightstyles[0] = "a";
	sv.lightstyles[2] = "mmnn";
	sv.model_precache[1] = "maps/ctest.bsp";
	sv.model_precache[2] = "progs/player.mdl";
	sv.sound_precache[1] = "weapons/r_exp3.wav";
	sv.particle_precache[1] = "tr_rocket";

	/* ED_WriteGlobals (pr_edict_save.c:162) emits only defs carrying
	 * DEF_SAVEGLOBAL, so the second def is deliberately without it. */
	ctest_save_globaldefs[0].type = ev_float | DEF_SAVEGLOBAL;
	ctest_save_globaldefs[0].ofs = 30;
	ctest_save_globaldefs[0].s_name = 1; /* "gsave" */
	ctest_save_globaldefs[1].type = ev_float;
	ctest_save_globaldefs[1].ofs = 31;
	ctest_save_globaldefs[1].s_name = 1;
	ctest_save_globals[30] = 3.5f;
	ctest_save_globals[31] = 9.0f;

	/* ED_Write starts its field walk at 1 (pr_edict_save.c:120), so the one
	 * live field has to be the second def. */
	ctest_save_fielddefs[0].type = ev_void;
	ctest_save_fielddefs[0].ofs = 0;
	ctest_save_fielddefs[0].s_name = 0;
	ctest_save_fielddefs[1].type = ev_float;
	ctest_save_fielddefs[1].ofs = (unsigned short)(offsetof (entvars_t, health) / 4);
	ctest_save_fielddefs[1].s_name = 7; /* "health" */

	memset (&ctest_save_progs, 0, sizeof (ctest_save_progs));
	ctest_save_progs.numglobaldefs = 2;
	ctest_save_progs.numfielddefs = 2;
	ctest_save_progs.entityfields = (int)(sizeof (entvars_t) / 4);

	memset (vm, 0, sizeof (*vm));
	vm->progs = &ctest_save_progs;
	vm->globaldefs = ctest_save_globaldefs;
	vm->fielddefs = ctest_save_fielddefs;
	vm->globals = ctest_save_globals;
	vm->strings = ctest_save_strings;
	vm->stringssize = (int)sizeof (ctest_save_strings);
	vm->edicts = (edict_t *)ctest_save_edicts;
	vm->edict_size = CTEST_SAVE_EDSIZE;
	vm->num_edicts = CTEST_SAVE_EDICTS;
	vm->max_edicts = CTEST_SAVE_EDICTS;
	vm->time = qctime;
	/* a non-negative alpha field offset suppresses the synthesised alpha line
	 * at pr_edict_save.c:150, which only fires for progs without one. */
	vm->extfields.alpha = 0;

	for (i = 0; i < CTEST_SAVE_EDICTS; i++)
		ctest_save_edict (i)->free = false;
	ctest_save_edict (1)->v.health = 42.0f;

	ctest_save_client.active = true;
	ctest_save_client.spawned = true;
	ctest_save_client.edict = ctest_save_edict (1);
	for (i = 0; i < NUM_BASIC_SPAWN_PARMS; i++)
		ctest_save_client.spawn_parms[i] = (float)i;
	/* the extended block only prints non-zero high spawn parms
	 * (host_cmd.c:1685), so seed exactly one of them. */
	ctest_save_client.spawn_parms[NUM_BASIC_SPAWN_PARMS] = 7.5f;

	svs.clients = &ctest_save_client;
	svs.maxclients = 1;
	svs.maxclientslimit = 1;
	svs.serverflags = 3;
	host_client = &ctest_save_client;

	current_skill = skill_value;
	cmd_source = src_command;
}

void ctest_hostcmd_set_intermission (int value)
{
	cl.intermission = value;
}

void ctest_hostcmd_set_nomonsters (int value)
{
	sv.nomonsters = value ? true : false;
}

void ctest_hostcmd_set_sv_active (int value)
{
	sv.active = value ? true : false;
}

void ctest_hostcmd_set_maxclients (int value)
{
	svs.maxclients = value;
}

void ctest_hostcmd_set_player_health (float value)
{
	ctest_save_edict (1)->v.health = value;
}

void ctest_hostcmd_set_cmd_source (int value)
{
	cmd_source = (cmd_source_t)value;
}

void ctest_hostcmd_tokenize (const char *text)
{
	Cmd_TokenizeString (text);
}

const char *ctest_hostcmd_get_lastsave (void)
{
	return sv.lastsave;
}

/* Every early return in the writer happens before PR_SwitchQCVM, and the
 * success path switches back out again (host_cmd.c:1706); this exists so a
 * test that fails mid-write cannot poison the next one through the
 * "already active" guard in stubs.c's PR_SwitchQCVM (stubs.c:3092). */
void ctest_hostcmd_clear_qcvm (void)
{
	if (qcvm)
		PR_SwitchQCVM (NULL);
}
