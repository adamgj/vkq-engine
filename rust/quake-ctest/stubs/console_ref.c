/* Phase 7 M10c oracle TU for Quake/console.c.
 *
 * WHY THIS FILE COMPOSES console.c INSTEAD OF build.rs LISTING IT IN C_SOURCES
 *
 * The prelude's c_ref_* renames are translation-unit-wide by construction: one
 * #define rewrites the definition in the oracle source AND every call in every
 * other oracle source. For console.c that is wrong for two reasons.
 *
 *   1. Con_Printf / Con_SafePrintf / Con_LinkPrintf / Con_Warning / ... are
 *      the harness's single most-used observation channel: stubs/stubs.c
 *      defines them as CON_STUB capture doubles and roughly a thousand tests
 *      assert on ctest_con_log_get(). A prelude rename would repoint every
 *      other oracle source at the real console.c, whose output depends on
 *      con_text, con_linewidth and the notify ring rather than on the format
 *      string.
 *
 *   2. stubs/stubs.c defined Con_AddToTabList, Con_LogCenterPrint and
 *      Con_Quakebar as *aborting* doubles, and tests/cl_main_differential.rs
 *      records that reaching one IS the observation.
 *
 * So the rename layer for console.c lives HERE, in console.c's own TU, where
 * it renames console.c's definitions and console.c's internal calls and
 * nothing else. The plain names below belong to the port --
 * quake-capi/src/console.rs for the twenty-one entry points and the ten
 * quake_rs_* cores, this file for the glue-owned data, the ADR-009
 * trampolines and the C-variadic entry points, exactly as Quake/console_glue.c
 * does in the engine build.
 *
 * TWO SYMBOLS ARE RENAMED FOR THE WHOLE TU, port half included:
 *
 *   SCR_UpdateScreen -- console.c:1282 (Con_Printf's tail) and console.c:2393
 *     (Con_NotifyBox) call it, and so does the plain Con_Printf below. It
 *     cannot be defined plain here: stubs/host_ref.c owns that name as an
 *     aborting double and stubs/keys_ref.c renames it the same way for the
 *     same reason. Both halves land on the counter at the bottom of this file.
 *
 *   Sys_Printf -- console.c:1256 echoes every Con_Printf through it. stubs.c
 *     defines Sys_Printf as a CON_STUB under the "[sys]" tag, so leaving it
 *     plain would put the ORACLE's console text into the capture log (once as
 *     "[sys]", on top of its own con_text) while the port's Con_Printf below
 *     emits "[con]". Silencing it here keeps the log exactly as the ~1100
 *     existing assertions have always seen it, and console text is compared
 *     through con_text, which is the real observable anyway.
 *
 * WHAT THE PORT'S Con_Printf DELIBERATELY OMITS: the Sys_Printf echo. The
 * historic stubs.c double logged "[con] <formatted>" and nothing else, so the
 * plain Con_Printf below emits exactly that through ctest_con_appendf, and a
 * nesting counter keeps Con_SafePrintf -> Con_Printf and Con_LinkPrintf ->
 * Con_SafePrintf -> Con_Printf emitting ONE line under the outermost entry
 * point's historic tag. Con_StripControlPrefixes is therefore not on that
 * path; it is compared directly through ctest_console_strip.
 *
 * Con_Warning / Con_DWarning / Con_DPrintf / Con_DPrintf2 are NOT defined
 * here. They are 100% C in Quake/console_glue.c (nothing about them was
 * ported), so there is nothing to compare, and stubs/stubs.c keeps its
 * capture doubles for them.
 *
 * COST, stated so it is not discovered later:
 * scripts/harness/check_ctest_symbols.sh reads C_SOURCES out of build.rs, so
 * it does not inspect this object; build.rs watches Quake/console.c
 * explicitly instead.
 */

/* ---- whole-TU renames (see the header comment) --------------------------- */
#define SCR_UpdateScreen ctest_console_SCR_UpdateScreen
#define Sys_Printf		 ctest_console_Sys_Printf

/* ---- console.c rename block ----------------------------------------------
 * Every file-scope symbol Quake/console.c defines with external linkage, plus
 * the six statics the fixture has to reach by name (a static definition would
 * otherwise occupy the plain name in this very TU and hide the port's export)
 * and the tab_t/tab_s spelling the plain half needs for its own tablist.
 */

/* data (console.c:39-77, :1546, :1555) */
#define con_linewidth		 c_ref_con_linewidth
#define con_cursorspeed		 c_ref_con_cursorspeed
#define con_buffersize		 c_ref_con_buffersize
#define con_forcedup		 c_ref_con_forcedup
#define con_totallines		 c_ref_con_totallines
#define con_backscroll		 c_ref_con_backscroll
#define con_current			 c_ref_con_current
#define con_x				 c_ref_con_x
#define con_text			 c_ref_con_text
#define con_notifytime		 c_ref_con_notifytime
#define con_logcenterprint	 c_ref_con_logcenterprint
#define con_notifycenter	 c_ref_con_notifycenter
#define con_notifyfade		 c_ref_con_notifyfade
#define con_notifyfadetime	 c_ref_con_notifyfadetime
#define con_maxcols			 c_ref_con_maxcols
#define con_lastcenterstring c_ref_con_lastcenterstring
#define con_redirect_flush	 c_ref_con_redirect_flush
#define con_redirect_buffer	 c_ref_con_redirect_buffer
#define con_times			 c_ref_con_times
#define con_vislines		 c_ref_con_vislines
#define con_debuglog		 c_ref_con_debuglog
#define con_initialized		 c_ref_con_initialized
#define con_mutex			 c_ref_con_mutex
#define key_tabpartial		 c_ref_key_tabpartial
#define tablist				 c_ref_tablist
#define tab_s				 c_ref_tab_s
#define tab_t				 c_ref_tab_t

/* functions with external linkage (console.c:403-2445) */
#define Con_SelectAll				 c_ref_Con_SelectAll
#define Con_Mousemove				 c_ref_Con_Mousemove
#define Con_ForceMouseMove			 c_ref_Con_ForceMouseMove
#define Con_UpdateMouseState		 c_ref_Con_UpdateMouseState
#define Con_Quakebar				 c_ref_Con_Quakebar
#define Con_ToggleConsole_f			 c_ref_Con_ToggleConsole_f
#define Con_CopySelectionToClipboard c_ref_Con_CopySelectionToClipboard
#define Con_ClearNotify				 c_ref_Con_ClearNotify
#define Con_CheckResize				 c_ref_Con_CheckResize
#define Con_Scroll					 c_ref_Con_Scroll
#define Con_Init					 c_ref_Con_Init
#define Con_DebugLog				 c_ref_Con_DebugLog
#define Con_Printf					 c_ref_Con_Printf
#define Con_DWarning				 c_ref_Con_DWarning
#define Con_Warning					 c_ref_Con_Warning
#define Con_DPrintf					 c_ref_Con_DPrintf
#define Con_DPrintf2				 c_ref_Con_DPrintf2
#define Con_LinkPrintf				 c_ref_Con_LinkPrintf
#define Con_SafePrintf				 c_ref_Con_SafePrintf
#define Con_CenterPrintf			 c_ref_Con_CenterPrintf
#define Con_LogCenterPrint			 c_ref_Con_LogCenterPrint
#define Con_IsRedirected			 c_ref_Con_IsRedirected
#define Con_Redirect				 c_ref_Con_Redirect
#define Con_AddToTabList			 c_ref_Con_AddToTabList
#define Con_Match					 c_ref_Con_Match
#define Con_TabComplete				 c_ref_Con_TabComplete
#define Con_DrawNotify				 c_ref_Con_DrawNotify
#define Con_DrawInput				 c_ref_Con_DrawInput
#define Con_DrawConsole				 c_ref_Con_DrawConsole
#define Con_NotifyBox				 c_ref_Con_NotifyBox
#define LOG_Init					 c_ref_LOG_Init
#define LOG_Close					 c_ref_LOG_Close

/* statics the fixture drives directly (console.c:781-1229) */
#define Con_Print				 c_ref_Con_Print
#define Con_Clear_f				 c_ref_Con_Clear_f
#define Con_Dump_f				 c_ref_Con_Dump_f
#define Con_MessageMode_f		 c_ref_Con_MessageMode_f
#define Con_MessageMode2_f		 c_ref_Con_MessageMode2_f
#define Con_StripControlPrefixes c_ref_Con_StripControlPrefixes

/* console.h was force-included by the prelude ahead of the block above, so
 * console.c's renamed definitions would have no visible prototype and its
 * forward calls (Con_ClearNotify from Con_CheckResize at console.c:1009,
 * Con_ForceMouseMove from Con_Scroll at console.c:1044, ...) would fall back
 * to implicit int. Re-declaring them here costs nothing: the macros above
 * rewrite each line, so the text is a verbatim copy of console.h plus the two
 * entry points console.c declares in no header at all. */
void		Con_CheckResize (void);
void		Con_Init (void);
void		Con_DrawConsole (cb_context_t *cbx, int lines, qboolean drawinput);
void		Con_Print (const char *txt);
void		Con_Printf (const char *fmt, ...) FUNC_PRINTF (1, 2);
void		Con_DWarning (const char *fmt, ...) FUNC_PRINTF (1, 2);
void		Con_Warning (const char *fmt, ...) FUNC_PRINTF (1, 2);
void		Con_DPrintf (const char *fmt, ...) FUNC_PRINTF (1, 2);
void		Con_DPrintf2 (const char *fmt, ...) FUNC_PRINTF (1, 2);
void		Con_SafePrintf (const char *fmt, ...) FUNC_PRINTF (1, 2);
void		Con_LinkPrintf (const char *addr, const char *fmt, ...) FUNC_PRINTF (2, 3);
void		Con_CenterPrintf (int linewidth, const char *fmt, ...) FUNC_PRINTF (2, 3);
void		Con_DrawNotify (cb_context_t *cbx);
void		Con_DrawInput (cb_context_t *cbx);
void		Con_ClearNotify (void);
void		Con_ToggleConsole_f (void);
void		Con_NotifyBox (const char *text);
void		Con_Scroll (int lines);
void		Con_SelectAll (void);
void		Con_Mousemove (int x, int y);
void		Con_ForceMouseMove (void);
void		Con_UpdateMouseState (void);
qboolean	Con_CopySelectionToClipboard (void);
const char *Con_Quakebar (int len);
void		Con_LogCenterPrint (const char *str);
void		Con_Redirect (void (*flush) (const char *text));
qboolean	Con_IsRedirected (void);
void		Con_DebugLog (const char *msg);
void		Con_AddToTabList (const char *name, const char *partial, const char *type);
void		Con_TabComplete (tabcomplete_t mode);
qboolean	Con_Match (const char *str, const char *partial);
void		LOG_Init (quakeparms_t *parms);
void		LOG_Close (void);

/* Renamed above, so screen.h's plain prototype does not cover it and
 * console.c:1282 would call it implicitly. Same for sys.h's Sys_Printf. */
void SCR_UpdateScreen (qboolean use_tasks);
void Sys_Printf (const char *fmt, ...) FUNC_PRINTF (1, 2);

#include <stdio.h>
#include <string.h>
#include <time.h>

#include "console.c"

/* =========================================================================
 * THE PLAIN HALF -- the ctest-link mirror of Quake/console_glue.c
 * ========================================================================= */

#undef con_linewidth
#undef con_cursorspeed
#undef con_buffersize
#undef con_forcedup
#undef con_totallines
#undef con_backscroll
#undef con_current
#undef con_x
#undef con_text
#undef con_notifytime
#undef con_logcenterprint
#undef con_notifycenter
#undef con_notifyfade
#undef con_notifyfadetime
#undef con_maxcols
#undef con_lastcenterstring
#undef con_redirect_flush
#undef con_redirect_buffer
#undef con_times
#undef con_vislines
#undef con_debuglog
#undef con_initialized
#undef con_mutex
#undef key_tabpartial
#undef tablist
#undef tab_s
#undef tab_t
#undef cl
#undef cls
#undef Con_SelectAll
#undef Con_Mousemove
#undef Con_ForceMouseMove
#undef Con_UpdateMouseState
#undef Con_Quakebar
#undef Con_ToggleConsole_f
#undef Con_CopySelectionToClipboard
#undef Con_ClearNotify
#undef Con_CheckResize
#undef Con_Scroll
#undef Con_Init
#undef Con_DebugLog
#undef Con_Printf
#undef Con_DWarning
#undef Con_Warning
#undef Con_DPrintf
#undef Con_DPrintf2
#undef Con_LinkPrintf
#undef Con_SafePrintf
#undef Con_CenterPrintf
#undef Con_LogCenterPrint
#undef Con_IsRedirected
#undef Con_Redirect
#undef Con_AddToTabList
#undef Con_Match
#undef Con_TabComplete
#undef Con_DrawNotify
#undef Con_DrawInput
#undef Con_DrawConsole
#undef Con_NotifyBox
#undef LOG_Init
#undef LOG_Close
#undef Con_Print
#undef Con_Clear_f
#undef Con_Dump_f
#undef Con_MessageMode_f
#undef Con_MessageMode2_f
#undef Con_StripControlPrefixes

extern client_state_t  cl;	/* quake-capi's cl_main port owns these two */
extern client_static_t cls;

/* ---------------------------------------------------------------------------
 * C-visible objects (console.c:39-77, :1546, :1555), initializers verbatim
 * from Quake/console_glue.c. con_forcedup and con_lastcenterstring used to be
 * defined in stubs/stubs.c and the six con_* geometry fields in
 * stubs/keys_ref.c; both sets moved here in the same change, so this is still
 * the single definition of each.
 */

int con_linewidth;

float con_cursorspeed = 4;

int con_buffersize;

qboolean con_forcedup;

int	  con_totallines;
int	  con_backscroll;
int	  con_current;
int	  con_x;
char *con_text = NULL;

cvar_t con_notifytime = {"con_notifytime", "3", CVAR_NONE};
cvar_t con_logcenterprint = {"con_logcenterprint", "1", CVAR_NONE};
cvar_t con_notifycenter = {"con_notifycenter", "0", CVAR_ARCHIVE};
cvar_t con_notifyfade = {"con_notifyfade", "0", CVAR_ARCHIVE};
cvar_t con_notifyfadetime = {"con_notifyfadetime", "0.5", CVAR_ARCHIVE};
cvar_t con_maxcols = {"con_maxcols", "0", CVAR_ARCHIVE};

char con_lastcenterstring[1024];
void (*con_redirect_flush) (const char *buffer);
char con_redirect_buffer[8192];

float con_times[NUM_CON_TIMES];

int con_vislines;

qboolean con_debuglog = false;

qboolean con_initialized;

qmutex_t *con_mutex;

char key_tabpartial[MAXCMDLINE];

/* console.c:1546-1555. tablist has external linkage in console.c and
 * Quake/console_glue.c keeps it, so the completion list is a direct
 * observable on both sides instead of being visible only through the text
 * Con_PrintTabList happens to emit. */
typedef struct tab_s
{
	const char	 *name;
	const char	 *type;
	struct tab_s *next;
	struct tab_s *prev;
	int			  count;
} tab_t;
tab_t *tablist;

/* ---------------------------------------------------------------------------
 * The port's status cores and the harness's raise machinery.
 */
extern int	Host_Guard (void (*fn) (void *), void *arg);
extern void Host_Reraise (int guard_result);

extern int		   quake_rs_con_toggle_console_f (void);
extern int		   quake_rs_con_tab_complete (int mode);
extern void		   quake_rs_con_print (const char *txt);
extern const char *quake_rs_con_strip_control_prefixes (const char *txt);
extern void		   quake_rs_con_link_print (const char *addr, const char *msg);
extern void		   quake_rs_con_center_print (int linewidth, const char *msg);
extern void		   quake_rs_con_draw_notify (cb_context_t *cbx);
extern void		   quake_rs_con_draw_input (cb_context_t *cbx);
extern void		   quake_rs_con_draw_console (cb_context_t *cbx, int lines, qboolean drawinput);
extern void		   quake_rs_log_init (const char *basedir, const char *session);
extern void		   quake_rs_con_scroll (int lines);
extern void		   quake_rs_con_select_all (void);
extern void		   quake_rs_con_force_mouse_move (void);
extern qboolean	   quake_rs_con_copy_selection_to_clipboard (void);

/* ---------------------------------------------------------------------------
 * ADR-009 trampolines, bodies verbatim from Quake/console_glue.c.
 */

/* console.c:763 */
static void Console_InvokeMenuMain (void *p)
{
	(void)p;
	M_Menu_Main_f ();
}

int Console_Glue_MenuMain (void)
{
	return Host_Guard (Console_InvokeMenuMain, NULL);
}

/* console.c:1858 */
typedef struct
{
	cvar_t	   *cvar;
	const char *partial;
} conglue_cvarcomp_t;

static void Console_InvokeCvarCompletion (void *p)
{
	conglue_cvarcomp_t *arg = (conglue_cvarcomp_t *)p;
	arg->cvar->completion (arg->cvar, arg->partial);
}

int Console_Glue_CvarCompletion (cvar_t *cvar, const char *partial)
{
	conglue_cvarcomp_t arg;
	arg.cvar = cvar;
	arg.partial = partial;
	return Host_Guard (Console_InvokeCvarCompletion, &arg);
}

/* console.c:1866 */
typedef struct
{
	xtabcommand_t completion;
	const char	 *partial;
} conglue_cmdcomp_t;

static void Console_InvokeCmdCompletion (void *p)
{
	conglue_cmdcomp_t *arg = (conglue_cmdcomp_t *)p;
	arg->completion (arg->partial);
}

int Console_Glue_CmdCompletion (xtabcommand_t completion, const char *partial)
{
	conglue_cmdcomp_t arg;
	arg.completion = completion;
	arg.partial = partial;
	return Host_Guard (Console_InvokeCmdCompletion, &arg);
}

/* ---------------------------------------------------------------------------
 * Non-guard shims, bodies verbatim from Quake/console_glue.c except that
 * _IONBF is not spelled in a harness that never opens a real log through the
 * port (the value differs between the MSVC CRT and glibc).
 */

/* console.c:834 */
void Console_Glue_SetClipboardText (const char *text)
{
	SDL_SetClipboardText (text);
}

/* quakever.h */
const char *Console_Glue_EngineNameAndVer (void)
{
	return ENGINE_NAME_AND_VER;
}

/* console.c:2440 */
void Console_Glue_LogSetUnbuffered (FILE *f)
{
	setvbuf (f, NULL, _IONBF, 0);
}

/* console.c:2437 */
void Console_Glue_LogOpenFailed (const char *name)
{
	fprintf (stderr, "Error: Unable to create log file %s\n", name);
}

/* ---------------------------------------------------------------------------
 * Quake/console_glue.c owns the plain names of six more entry points --
 * Con_ToggleConsole_f and Con_TabComplete (Host_Reraise wrappers over the
 * status cores) plus Con_SelectAll, Con_ForceMouseMove, Con_Scroll and
 * Con_CopySelectionToClipboard (plain wrappers). They are deliberately NOT
 * mirrored here: keys.c calls all six, and stubs/keys_ref.c still owns those
 * plain names with the link doubles tests/keys_differential.rs counts. The
 * fixture below reaches the port through the quake_rs_* names instead, which
 * is exactly what the glue wrappers do.
 */

/* ---------------------------------------------------------------------------
 * The C-variadic console entry points (console.c:1241-1461).
 *
 * ctest_console_depth exists only in this harness: Con_SafePrintf calls
 * Con_Printf (console.c:1450) and Con_LinkPrintf calls Con_SafePrintf
 * (console.c:1406), so without it one Con_SafePrintf("x") would append both
 * "[safe] x" and "[con] x" and break every historic assertion. Exactly one
 * line is appended per outermost public entry point, under the tag
 * stubs/stubs.c used before this file existed.
 */

extern void ctest_con_appendf (const char *tag, const char *fmt, ...);

static int	ctest_console_depth;
static char ctest_console_last_link[MAX_OSPATH];

static void ctest_console_emit (const char *tag, const char *msg)
{
	if (ctest_console_depth == 0)
		ctest_con_appendf (tag, "%s", msg);
}

/* tests/host_cmd_differential.rs reads this back to check the link target
 * Con_LinkPrintf was handed; it used to live next to stubs.c's Con_LinkPrintf
 * double. */
const char *ctest_get_last_link_addr (void)
{
	return ctest_console_last_link;
}

/* console.c:1241 */
void Con_Printf (const char *fmt, ...)
{
	va_list			argptr;
	char			msg[MAXPRINTMSG];
	static qboolean inupdate;

	va_start (argptr, fmt);
	q_vsnprintf (msg, sizeof (msg), fmt, argptr);
	va_end (argptr);

	if (con_redirect_flush)
		q_strlcat (con_redirect_buffer, msg, sizeof (con_redirect_buffer));
	/* the Sys_Printf echo is replaced by the capture log; see the header */
	ctest_console_emit ("[con]", msg);

	if (con_debuglog)
		Con_DebugLog (msg);

	if (!con_initialized)
		return;

	if (cls.state == ca_dedicated)
		return;

	quake_rs_con_print (msg);

	if (cls.signon != SIGNONS && !scr_disabled_for_loading && !Tasks_IsWorker ())
	{
		if (!inupdate)
		{
			inupdate = true;
			SCR_UpdateScreen (false);
			inupdate = false;
		}
	}
}

/* console.c:1383 */
void Con_LinkPrintf (const char *addr, const char *fmt, ...)
{
	va_list argptr;
	char	msg[MAXPRINTMSG];

	va_start (argptr, fmt);
	q_vsnprintf (msg, sizeof (msg), fmt, argptr);
	va_end (argptr);

	q_strlcpy (ctest_console_last_link, addr, sizeof (ctest_console_last_link));
	ctest_console_emit ("[link]", msg);

	ctest_console_depth++;
	quake_rs_con_link_print (addr, msg);
	ctest_console_depth--;
}

/* console.c:1435 */
void Con_SafePrintf (const char *fmt, ...)
{
	va_list argptr;
	char	msg[MAXPRINTMSG];
	int		temp;

	va_start (argptr, fmt);
	q_vsnprintf (msg, sizeof (msg), fmt, argptr);
	va_end (argptr);

	ctest_console_emit ("[safe]", msg);

	QMutex_Lock (con_mutex);
	temp = scr_disabled_for_loading;
	scr_disabled_for_loading = true;
	ctest_console_depth++;
	Con_Printf ("%s", msg);
	ctest_console_depth--;
	scr_disabled_for_loading = temp;
	QMutex_Unlock (con_mutex);
}

/* console.c:1456 */
void Con_CenterPrintf (int linewidth, const char *fmt, ...)
{
	va_list argptr;
	char	msg[MAXPRINTMSG];

	va_start (argptr, fmt);
	q_vsnprintf (msg, sizeof (msg), fmt, argptr);
	va_end (argptr);

	quake_rs_con_center_print (linewidth, msg);
}

/* ---------------------------------------------------------------------------
 * Plain forwards, bodies verbatim from Quake/console_glue.c.
 */

void Con_DrawNotify (cb_context_t *cbx)
{
	quake_rs_con_draw_notify (cbx);
}

void Con_DrawInput (cb_context_t *cbx)
{
	quake_rs_con_draw_input (cbx);
}

void Con_DrawConsole (cb_context_t *cbx, int lines, qboolean drawinput)
{
	quake_rs_con_draw_console (cbx, lines, drawinput);
}

/* console.c:2409. The -condebug check and the session timestamp stay in C in
 * the glue; ctest_console_log_init drives quake_rs_log_init directly with a
 * fixed session string so the two sides write byte-identical headers. */
void LOG_Init (quakeparms_t *parms)
{
	time_t inittime;
	char   session[24];

#if !defined(DEBUG) && !defined(_DEBUG)
	if (!COM_CheckParm ("-condebug"))
		return;
#endif

	inittime = time (NULL);
	strftime (session, sizeof (session), "%m/%d/%Y %H:%M:%S", localtime (&inittime));

	quake_rs_log_init (parms->basedir, session);
}

/* console.c:2375. Kept for link fidelity with Quake/console_glue.c; nothing in
 * this harness drives it (it spins on SCR_UpdateScreen until a key arrives). */
void Con_NotifyBox (const char *text)
{
	double t1, t2;
	int	   lastkey, lastchar;

	Con_Printf ("\n\n%s", Con_Quakebar (40));
	Con_Printf ("%s", text);
	Con_Printf ("Press a key.\n");
	Con_Printf ("%s", Con_Quakebar (40));

	IN_DeactivateForConsole ();
	key_dest = key_console;

	Key_BeginInputGrab ();
	do
	{
		t1 = Sys_DoubleTime ();
		SCR_UpdateScreen (false);
		Sys_SendKeyEvents ();
		Key_GetGrabbedInput (&lastkey, &lastchar);
		Sys_Sleep (16);
		t2 = Sys_DoubleTime ();
		realtime += t2 - t1;
	} while (lastkey == -1 && lastchar == -1);
	Key_EndInputGrab ();

	Con_Printf ("\n");
	IN_Activate ();
	key_dest = key_game;
	realtime = 0;
}

/* =========================================================================
 * LINK DOUBLES
 *
 * Everything below is reached by BOTH halves, so each one is a recorder
 * rather than an abort: the renderer entry points, the two SDL-facing hooks
 * and the three common.c helpers console.c uses that nothing else in this
 * link defines.
 * ========================================================================= */

typedef struct
{
	int	  updatescreen_calls;
	int	  menumain_calls;
	int	  explore_calls;
	int	  clipboard_calls;
	int	  setcursor_calls;
	int	  cursor;
	int	  sleep_calls;
	char  explore_path[MAX_OSPATH];
	char  clipboard[4096];
	float canvascolor[4];
} ctest_console_calls_t;

static ctest_console_calls_t ctest_console_calls;

/* Settable answers for the callees console.c branches on. */
static qboolean ctest_console_explore_result = true;
static int		ctest_console_mousex, ctest_console_mousey;

/* The draw stream. Con_DrawNotify / Con_DrawInput / Con_DrawConsole have no
 * state to compare afterwards -- the whole of their behaviour is the order and
 * arguments of the renderer calls they make -- so each one is appended here as
 * a line of text and the two sides' logs are compared as a whole. */
#define CTEST_CONSOLE_DRAWLOG_SIZE 262144
static char	  ctest_console_drawlog[CTEST_CONSOLE_DRAWLOG_SIZE];
static size_t ctest_console_drawlog_len;

static void ctest_console_draw_record (const char *fmt, ...)
{
	va_list argptr;
	char	line[512];

	va_start (argptr, fmt);
	q_vsnprintf (line, sizeof (line), fmt, argptr);
	va_end (argptr);

	q_strlcat (ctest_console_drawlog, line, sizeof (ctest_console_drawlog));
	ctest_console_drawlog_len = strlen (ctest_console_drawlog);
}

void ctest_console_clear_draw_log (void)
{
	ctest_console_drawlog[0] = '\0';
	ctest_console_drawlog_len = 0;
}

const char *ctest_console_draw_log (void)
{
	return ctest_console_drawlog;
}

/* draw.h:46 */
void Draw_Character (cb_context_t *cbx, float x, float y, int num)
{
	(void)cbx;
	ctest_console_draw_record ("char %.2f %.2f %d\n", x, y, num);
}

/* draw.h:54 */
void Draw_String (cb_context_t *cbx, float x, float y, const char *str)
{
	(void)cbx;
	ctest_console_draw_record ("str %.2f %.2f |%s|\n", x, y, str ? str : "(null)");
}

/* draw.h:52 */
void Draw_Fill (cb_context_t *cbx, float x, float y, float w, float h, int c, float alpha)
{
	(void)cbx;
	ctest_console_draw_record ("fill %.2f %.2f %.2f %.2f %d %.4f\n", x, y, w, h, c, alpha);
}

/* draw.h:50 */
/* draw.h:47 -- the insert/overwrite cursor pic at console.c:2251. The pic
 * pointer is only ever pic_ins or pic_ovr, so it is logged as which. */
void Draw_Pic (cb_context_t *cbx, float x, float y, qpic_t *pic, float alpha, qboolean alpha_blend)
{
	(void)cbx;
	ctest_console_draw_record ("pic %g %g %s %g %d", x, y, pic == pic_ins ? "ins" : (pic == pic_ovr ? "ovr" : "?"), alpha, alpha_blend ? 1 : 0);
}

void Draw_ConsoleBackground (cb_context_t *cbx)
{
	(void)cbx;
	ctest_console_draw_record ("conback\n");
}

/* draw.h:64 */
void GL_SetCanvas (cb_context_t *cbx, canvastype newcanvas)
{
	(void)cbx;
	ctest_console_draw_record ("canvas %d\n", (int)newcanvas);
}

/* draw.h:65 */
void GL_SetCanvasColor (float r, float g, float b, float a)
{
	ctest_console_calls.canvascolor[0] = r;
	ctest_console_calls.canvascolor[1] = g;
	ctest_console_calls.canvascolor[2] = b;
	ctest_console_calls.canvascolor[3] = a;
	ctest_console_draw_record ("canvascolor %.4f %.4f %.4f %.4f\n", r, g, b, a);
}

/* gl_draw.c: the two cursor pics Con_DrawInput (console.c:2216) alternates
 * between. NULL is what Draw_TryCachePic hands back for a missing pic, and
 * console.c only ever passes them straight to Draw_Pic. */
qpic_t *pic_ovr;
qpic_t *pic_ins;

/* gl_screen.c: how far the console has slid down; console.c:2371 reads it. */
float scr_con_current;

/* screen.h:37 (renamed for this TU; stubs/host_ref.c owns the plain name) */
void SCR_UpdateScreen (qboolean use_tasks)
{
	(void)use_tasks;
	ctest_console_calls.updatescreen_calls++;
}

/* sys.h:71 (renamed for this TU; stubs/stubs.c owns the plain name as the
 * "[sys]" capture double) */
void Sys_Printf (const char *fmt, ...)
{
	(void)fmt;
}

/* menu.h:74 -- console.c:763, always reached through Console_Glue_MenuMain */
void M_Menu_Main_f (void)
{
	ctest_console_calls.menumain_calls++;
}

/* sys.h:100 -- console.c:591 opens the hot link's target */
qboolean Sys_Explore (const char *path)
{
	ctest_console_calls.explore_calls++;
	q_strlcpy (ctest_console_calls.explore_path, path ? path : "", sizeof (ctest_console_calls.explore_path));
	return ctest_console_explore_result;
}

/* sys.h:159 -- Con_NotifyBox only */
void Sys_Sleep (unsigned long msecs)
{
	(void)msecs;
	ctest_console_calls.sleep_calls++;
}

/* input.h -- the mouse position Con_ForceMouseMove replays */
void IN_GetMousePos (int *x, int *y)
{
	*x = ctest_console_mousex;
	*y = ctest_console_mousey;
}

/* vid.h:100 */
void VID_SetMouseCursor (mousecursor_t cursor)
{
	ctest_console_calls.setcursor_calls++;
	ctest_console_calls.cursor = (int)cursor;
}

/* SDL3. stubs/stubs.c used to abort here; both halves reach it now, the port
 * through Console_Glue_SetClipboardText. */
int SDL_SetClipboardText (const char *text)
{
	ctest_console_calls.clipboard_calls++;
	q_strlcpy (ctest_console_calls.clipboard, text ? text : "", sizeof (ctest_console_calls.clipboard));
	return 0;
}

/* ---------------------------------------------------------------------------
 * common.c helpers console.c needs that nothing else in this link defines.
 * The four Vec_* are copied verbatim from Quake/common.c:118-176 because
 * con_links and Con_CopySelectionToClipboard's scratch buffer are grown
 * through the VEC_* macros, and the exact growth policy is visible through
 * VEC_SIZE. q_strnaturalcmp is verbatim from Quake/common.c:181.
 */

void Vec_Grow (void **pvec, size_t element_size, size_t count)
{
	vec_header_t header;
	if (*pvec)
		header = VEC_HEADER (*pvec);
	else
		header.size = header.capacity = 0;

	if (header.size + count > header.capacity)
	{
		void  *new_buffer;
		size_t total_size;

		header.capacity = header.size + count;
		header.capacity += header.capacity >> 1;
		if (header.capacity < 16)
			header.capacity = 16;
		total_size = sizeof (vec_header_t) + header.capacity * element_size;

		if (*pvec)
			new_buffer = Mem_Realloc (((vec_header_t *)*pvec) - 1, total_size);
		else
			new_buffer = Mem_Alloc (total_size);
		if (!new_buffer)
			Sys_Error ("Vec_Grow: failed to allocate %lu bytes\n", (unsigned long)total_size);

		*pvec = 1 + (vec_header_t *)new_buffer;
		VEC_HEADER (*pvec) = header;
	}
}

void Vec_Append (void **pvec, size_t element_size, const void *data, size_t count)
{
	if (!count)
		return;
	Vec_Grow (pvec, element_size, count);
	memcpy ((byte *)*pvec + VEC_HEADER (*pvec).size * element_size, data, count * element_size);
	VEC_HEADER (*pvec).size += count;
}

void Vec_Clear (void **pvec)
{
	if (*pvec)
		VEC_HEADER (*pvec).size = 0;
}

void Vec_Free (void **pvec)
{
	if (*pvec)
	{
		Mem_Free (&VEC_HEADER (*pvec));
		*pvec = NULL;
	}
}

int q_strnaturalcmp (const char *s1, const char *s2)
{
	qboolean neg1, neg2, sign1, sign2;

	if (s1 == s2)
		return 0;

	neg1 = *s1 == '-';
	neg2 = *s2 == '-';
	sign1 = neg1 || *s1 == '+';
	sign2 = neg2 || *s2 == '+';

	// early out if strings start with different signs followed by digits
	if (neg1 != neg2 && q_isdigit (s1[sign1]) && q_isdigit (s1[sign2]))
		return neg2 - neg1;

skip_prefix:
	while (*s1 && !q_isdigit (*s1) && q_toupper (*s1) == q_toupper (*s2))
	{
		s1++;
		s2++;
		continue;
	}

	if (q_isdigit (*s1) && q_isdigit (*s2))
	{
		const char *begin1 = s1++;
		const char *begin2 = s2++;
		int			diff, sign;

		while (*begin1 == '0')
			begin1++;
		while (*begin2 == '0')
			begin2++;
		while (q_isdigit (*s1))
			s1++;
		while (q_isdigit (*s2))
			s2++;

		sign = neg1 ? -1 : 1;

		diff = (s1 - begin1) - (s2 - begin2);
		if (diff)
			return diff * sign;

		while (begin1 != s1)
		{
			diff = *begin1++ - *begin2++;
			if (diff)
				return diff * sign;
		}

		// We only support negative numbers at the beginning of strings so that
		// "-2" is sorted before "-1", but "file-2345.ext" *after* "file-1234.ext".
		neg1 = neg2 = false;

		goto skip_prefix;
	}

	return q_toupper (*s1) - q_toupper (*s2);
}

/* common.c:2003. The real one walks the 256-entry qchar_to_unicode table and
 * UTF8_WriteCodePoint; the harness only needs it to be deterministic, total
 * and identical for both halves, so high-bit characters are masked to their
 * ASCII twin and everything else is copied through. Both Con_
 * CopySelectionToClipboard paths call it with the same qtext, so the clipboard
 * bytes stay a valid differential. */
size_t UTF8_FromQuake (char *dst, size_t maxbytes, const char *src)
{
	size_t n = 0;

	for (; *src; src++)
	{
		char c = (char)(*src & 0x7f);
		if (dst && n + 1 < maxbytes)
			dst[n] = c;
		n++;
	}
	if (dst && maxbytes)
		dst[n < maxbytes ? n : maxbytes - 1] = '\0';
	return n + 1;
}

/* =========================================================================
 * THE FIXTURE
 *
 * `side` is 1 for the C oracle (c_ref_*) and 0 for the Rust port (plain), the
 * same convention stubs/cl_demo_ref.c and stubs/keys_ref.c use. Every
 * accessor is per-side because the two halves own two disjoint object sets;
 * the shared objects (key_dest, vid, the call recorder and the draw log above)
 * have unsuffixed accessors and must be re-seeded before each side runs.
 * ========================================================================= */

extern int	  c_ref_con_linewidth;
extern int	  c_ref_con_buffersize;
extern int	  c_ref_con_totallines;
extern int	  c_ref_con_backscroll;
extern int	  c_ref_con_current;
extern int	  c_ref_con_x;
extern int	  c_ref_con_vislines;
extern char	 *c_ref_con_text;
extern float  c_ref_con_times[NUM_CON_TIMES];
extern char	  c_ref_con_lastcenterstring[1024];
extern char	  c_ref_con_redirect_buffer[8192];
extern qboolean c_ref_con_debuglog;
extern qboolean c_ref_con_initialized;
extern qboolean c_ref_con_forcedup;
extern char	  c_ref_key_tabpartial[MAXCMDLINE];
extern cvar_t c_ref_con_notifytime;
extern cvar_t c_ref_con_logcenterprint;
extern cvar_t c_ref_con_notifycenter;
extern cvar_t c_ref_con_notifyfade;
extern cvar_t c_ref_con_notifyfadetime;
extern cvar_t c_ref_con_maxcols;

typedef struct
{
	int		 *linewidth;
	int		 *buffersize;
	int		 *totallines;
	int		 *backscroll;
	int		 *current;
	int		 *x;
	int		 *vislines;
	char	**text;
	float	 *times;
	char	 *lastcenter;
	char	 *redirect;
	qboolean *debuglog;
	qboolean *initialized;
	qboolean *forcedup;
	char	 *tabpartial;
	cvar_t	 *notifytime;
	cvar_t	 *logcenterprint;
	cvar_t	 *notifycenter;
	cvar_t	 *notifyfade;
	cvar_t	 *notifyfadetime;
	cvar_t	 *maxcols;
} ctest_console_side_t;

static ctest_console_side_t ctest_console_side (int side)
{
	ctest_console_side_t s;
	if (side)
	{
		s.linewidth = &c_ref_con_linewidth;
		s.buffersize = &c_ref_con_buffersize;
		s.totallines = &c_ref_con_totallines;
		s.backscroll = &c_ref_con_backscroll;
		s.current = &c_ref_con_current;
		s.x = &c_ref_con_x;
		s.vislines = &c_ref_con_vislines;
		s.text = &c_ref_con_text;
		s.times = c_ref_con_times;
		s.lastcenter = c_ref_con_lastcenterstring;
		s.redirect = c_ref_con_redirect_buffer;
		s.debuglog = &c_ref_con_debuglog;
		s.initialized = &c_ref_con_initialized;
		s.forcedup = &c_ref_con_forcedup;
		s.tabpartial = c_ref_key_tabpartial;
		s.notifytime = &c_ref_con_notifytime;
		s.logcenterprint = &c_ref_con_logcenterprint;
		s.notifycenter = &c_ref_con_notifycenter;
		s.notifyfade = &c_ref_con_notifyfade;
		s.notifyfadetime = &c_ref_con_notifyfadetime;
		s.maxcols = &c_ref_con_maxcols;
	}
	else
	{
		s.linewidth = &con_linewidth;
		s.buffersize = &con_buffersize;
		s.totallines = &con_totallines;
		s.backscroll = &con_backscroll;
		s.current = &con_current;
		s.x = &con_x;
		s.vislines = &con_vislines;
		s.text = &con_text;
		s.times = con_times;
		s.lastcenter = con_lastcenterstring;
		s.redirect = con_redirect_buffer;
		s.debuglog = &con_debuglog;
		s.initialized = &con_initialized;
		s.forcedup = &con_forcedup;
		s.tabpartial = key_tabpartial;
		s.notifytime = &con_notifytime;
		s.logcenterprint = &con_logcenterprint;
		s.notifycenter = &con_notifycenter;
		s.notifyfade = &con_notifyfade;
		s.notifyfadetime = &con_notifyfadetime;
		s.maxcols = &con_maxcols;
	}
	return s;
}

/* ---- entry points, dispatched per side ---------------------------------- */

void ctest_console_print (int side, const char *txt)
{
	if (side)
		c_ref_Con_Print (txt);
	else
		quake_rs_con_print (txt);
}

void ctest_console_printf (int side, const char *txt)
{
	if (side)
		c_ref_Con_Printf ("%s", txt);
	else
		Con_Printf ("%s", txt);
}

void ctest_console_safeprintf (int side, const char *txt)
{
	if (side)
		c_ref_Con_SafePrintf ("%s", txt);
	else
		Con_SafePrintf ("%s", txt);
}

void ctest_console_linkprintf (int side, const char *addr, const char *txt)
{
	if (side)
		c_ref_Con_LinkPrintf (addr, "%s", txt);
	else
		Con_LinkPrintf (addr, "%s", txt);
}

void ctest_console_centerprintf (int side, int linewidth, const char *txt)
{
	if (side)
		c_ref_Con_CenterPrintf (linewidth, "%s", txt);
	else
		Con_CenterPrintf (linewidth, "%s", txt);
}

void ctest_console_strip (int side, const char *txt, char *out, int cap)
{
	const char *r = side ? c_ref_Con_StripControlPrefixes (txt) : quake_rs_con_strip_control_prefixes (txt);
	memset (out, 0, (size_t)cap);
	if (r)
		q_strlcpy (out, r, (size_t)cap);
}

void ctest_console_checkresize (int side)
{
	if (side)
		c_ref_Con_CheckResize ();
	else
		Con_CheckResize ();
}

void ctest_console_clear (int side)
{
	if (side)
		c_ref_Con_Clear_f ();
	else
		Con_Clear_f ();
}

void ctest_console_dump (int side)
{
	if (side)
		c_ref_Con_Dump_f ();
	else
		Con_Dump_f ();
}

void ctest_console_clearnotify (int side)
{
	if (side)
		c_ref_Con_ClearNotify ();
	else
		Con_ClearNotify ();
}

void ctest_console_scroll (int side, int lines)
{
	if (side)
		c_ref_Con_Scroll (lines);
	else
		quake_rs_con_scroll (lines);
}

void ctest_console_selectall (int side)
{
	if (side)
		c_ref_Con_SelectAll ();
	else
		quake_rs_con_select_all ();
}

void ctest_console_mousemove (int side, int x, int y)
{
	if (side)
		c_ref_Con_Mousemove (x, y);
	else
		Con_Mousemove (x, y);
}

void ctest_console_updatemousestate (int side)
{
	if (side)
		c_ref_Con_UpdateMouseState ();
	else
		Con_UpdateMouseState ();
}

qboolean ctest_console_copyselection (int side)
{
	return side ? c_ref_Con_CopySelectionToClipboard () : quake_rs_con_copy_selection_to_clipboard ();
}

void ctest_console_quakebar (int side, int len, char *out, int cap)
{
	const char *r = side ? c_ref_Con_Quakebar (len) : Con_Quakebar (len);
	memset (out, 0, (size_t)cap);
	if (r)
		q_strlcpy (out, r, (size_t)cap);
}

void ctest_console_logcenterprint (int side, const char *str)
{
	if (side)
		c_ref_Con_LogCenterPrint (str);
	else
		Con_LogCenterPrint (str);
}

qboolean ctest_console_match (int side, const char *str, const char *partial)
{
	return side ? c_ref_Con_Match (str, partial) : Con_Match (str, partial);
}

void ctest_console_addtotablist (int side, const char *name, const char *partial, const char *type)
{
	if (side)
		c_ref_Con_AddToTabList (name, partial, type);
	else
		Con_AddToTabList (name, partial, type);
}

void ctest_console_tabcomplete (int side, int mode)
{
	if (side)
		c_ref_Con_TabComplete ((tabcomplete_t)mode);
	else
		Host_Reraise (quake_rs_con_tab_complete (mode));
}

void ctest_console_toggleconsole (int side)
{
	if (side)
		c_ref_Con_ToggleConsole_f ();
	else
		Host_Reraise (quake_rs_con_toggle_console_f ());
}

void ctest_console_messagemode (int side, int team)
{
	if (side)
	{
		if (team)
			c_ref_Con_MessageMode2_f ();
		else
			c_ref_Con_MessageMode_f ();
	}
	else
	{
		if (team)
			Con_MessageMode2_f ();
		else
			Con_MessageMode_f ();
	}
}

void ctest_console_debuglog (int side, const char *msg)
{
	if (side)
		c_ref_Con_DebugLog (msg);
	else
		Con_DebugLog (msg);
}

/* The C oracle's LOG_Init (console.c:2408) is the whole function, -condebug
 * check and strftime included; the port's is Quake/console_glue.c's C half
 * plus quake_rs_log_init, which starts one line later and takes the session
 * string as a parameter. There is no way to make the two agree on the
 * timestamp, so the caller is handed both halves and
 * tests/console_differential.rs compares everything after the "LOG started
 * on: " line. The -condebug parm the oracle gates on comes from com_argv,
 * which stubs/stubs.c owns and the test seeds. */
void ctest_console_log_init (int side, const char *basedir, const char *session)
{
	if (side)
	{
		quakeparms_t parms;
		memset (&parms, 0, sizeof (parms));
		parms.basedir = (char *)basedir;
		c_ref_LOG_Init (&parms);
	}
	else
		quake_rs_log_init (basedir, session);
}

void ctest_console_log_close (int side)
{
	if (side)
		c_ref_LOG_Close ();
	else
		LOG_Close ();
}

void ctest_console_init (int side)
{
	if (side)
		c_ref_Con_Init ();
	else
		Con_Init ();
}

void ctest_console_drawnotify (int side)
{
	if (side)
		c_ref_Con_DrawNotify (NULL);
	else
		quake_rs_con_draw_notify (NULL);
}

void ctest_console_drawinput (int side)
{
	if (side)
		c_ref_Con_DrawInput (NULL);
	else
		quake_rs_con_draw_input (NULL);
}

void ctest_console_drawconsole (int side, int lines, qboolean drawinput)
{
	if (side)
		c_ref_Con_DrawConsole (NULL, lines, drawinput);
	else
		quake_rs_con_draw_console (NULL, lines, drawinput);
}

/* ---- the redirect pair --------------------------------------------------
 * Con_Redirect installs a flush callback; the two sides get two different
 * recorders so that a leak from one into the other is visible. */

static char ctest_console_redirect_out[2][8192];

static void ctest_console_flush_c (const char *text)
{
	q_strlcat (ctest_console_redirect_out[1], text, sizeof (ctest_console_redirect_out[1]));
}

static void ctest_console_flush_rust (const char *text)
{
	q_strlcat (ctest_console_redirect_out[0], text, sizeof (ctest_console_redirect_out[0]));
}

void ctest_console_redirect (int side, qboolean on)
{
	if (side)
		c_ref_Con_Redirect (on ? ctest_console_flush_c : NULL);
	else
		Con_Redirect (on ? ctest_console_flush_rust : NULL);
}

qboolean ctest_console_is_redirected (int side)
{
	return side ? c_ref_Con_IsRedirected () : Con_IsRedirected ();
}

const char *ctest_console_redirect_output (int side)
{
	return ctest_console_redirect_out[side ? 1 : 0];
}

void ctest_console_clear_redirect_output (void)
{
	ctest_console_redirect_out[0][0] = '\0';
	ctest_console_redirect_out[1][0] = '\0';
}

/* ---- tab list observation ------------------------------------------------ */

static tab_t *ctest_console_tablist (int side)
{
	return side ? c_ref_tablist : tablist;
}

int ctest_console_tablist_count (int side)
{
	tab_t *head = ctest_console_tablist (side);
	tab_t *t;
	int	   n;

	if (!head)
		return 0;
	n = 1;
	for (t = head->next; t && t != head; t = t->next)
		n++;
	return n;
}

/* Fills name/type with entry `idx` (walking `next` from the head) and returns
 * its count field, or -1 when the list is shorter than that. */
int ctest_console_tablist_entry (int side, int idx, char *name, int namecap, char *type, int typecap)
{
	tab_t *head = ctest_console_tablist (side);
	tab_t *t;
	int	   i;

	memset (name, 0, (size_t)namecap);
	memset (type, 0, (size_t)typecap);
	if (!head || idx < 0 || idx >= ctest_console_tablist_count (side))
		return -1;
	t = head;
	for (i = 0; i < idx; i++)
		t = t->next;
	if (t->name)
		q_strlcpy (name, t->name, (size_t)namecap);
	if (t->type)
		q_strlcpy (type, t->type, (size_t)typecap);
	return t->count;
}

/* Con_ClearTabList (console.c:1573) is static on both sides, so the fixture
 * repeats its body over whichever head it was handed. */
static void ctest_console_free_tablist (int side)
{
	tab_t *head = ctest_console_tablist (side);
	tab_t *t, *next;

	if (!head)
		return;
	head->prev->next = NULL;
	for (t = head; t; t = next)
	{
		next = t->next;
		Mem_Free (t);
	}
	if (side)
		c_ref_tablist = NULL;
	else
		tablist = NULL;
}

/* ---- geometry, seeding and snapshots ------------------------------------ */

/* Con_Init's geometry (console.c:1049-1075) without its cvar and command
 * registration, so a test can drive one side repeatedly inside a single test
 * binary. con_initialized is set because everything downstream of
 * Con_Printf's early return (console.c:1273) depends on it. */
void ctest_console_setup (int side, int buffersize, int linewidth)
{
	ctest_console_side_t s = ctest_console_side (side);

	if (*s.text)
		Mem_Free (*s.text);
	*s.buffersize = buffersize;
	*s.text = (char *)Mem_Alloc ((size_t)buffersize);
	memset (*s.text, ' ', (size_t)buffersize);
	*s.linewidth = linewidth;
	*s.totallines = buffersize / linewidth;
	*s.backscroll = 0;
	*s.current = *s.totallines - 1;
	*s.x = 0;
	*s.vislines = 0;
	memset (s.times, 0, sizeof (float) * NUM_CON_TIMES);
	*s.initialized = true;
}

/* Puts one side back to the state it had before Con_Init: no buffer, no
 * links, no selection, no hot link, no tab list, no redirect. The selection
 * and hot-link statics have no external handle on either side, so they are
 * cleared the only way a caller can -- Con_UpdateMouseState's key_dest !=
 * key_console branch (console.c:681-687), which is why key_dest is forced to
 * key_game first. */
void ctest_console_reset (int side)
{
	ctest_console_side_t s = ctest_console_side (side);
	keydest_t			 saved = key_dest;

	if (*s.text)
	{
		key_dest = key_game;
		if (side)
			c_ref_Con_UpdateMouseState ();
		else
			Con_UpdateMouseState ();
		if (side)
			c_ref_Con_Clear_f ();
		else
			Con_Clear_f ();
		Mem_Free (*s.text);
	}
	key_dest = saved;

	ctest_console_free_tablist (side);

	*s.text = NULL;
	*s.buffersize = 0;
	*s.linewidth = 0;
	*s.totallines = 0;
	*s.backscroll = 0;
	*s.current = 0;
	*s.x = 0;
	*s.vislines = 0;
	*s.forcedup = false;
	*s.debuglog = false;
	*s.initialized = false;
	memset (s.times, 0, sizeof (float) * NUM_CON_TIMES);
	memset (s.lastcenter, 0, 1024);
	memset (s.tabpartial, 0, MAXCMDLINE);
	s.redirect[0] = '\0';
	if (side)
		c_ref_Con_Redirect (NULL);
	else
		Con_Redirect (NULL);
}

void ctest_console_set_cvars (int side, const char *notifytime, const char *logcenterprint, const char *notifycenter, const char *notifyfade,
							  const char *notifyfadetime, const char *maxcols)
{
	ctest_console_side_t s = ctest_console_side (side);
	struct
	{
		cvar_t	   *cv;
		const char *val;
	} tab[6] = {
		{s.notifytime, notifytime},		  {s.logcenterprint, logcenterprint},	{s.notifycenter, notifycenter},
		{s.notifyfade, notifyfade},		  {s.notifyfadetime, notifyfadetime},	{s.maxcols, maxcols},
	};
	int i;

	for (i = 0; i < 6; i++)
	{
		tab[i].cv->value = (float)atof (tab[i].val);
		tab[i].cv->string = (char *)tab[i].val;
	}
}

void ctest_console_set_notify_time (int side, int index, float t)
{
	ctest_console_side_t s = ctest_console_side (side);
	if (index >= 0 && index < NUM_CON_TIMES)
		s.times[index] = t;
}

void ctest_console_set_forcedup (int side, qboolean v)
{
	ctest_console_side_t s = ctest_console_side (side);
	*s.forcedup = v;
}

void ctest_console_set_tabpartial (int side, const char *text)
{
	ctest_console_side_t s = ctest_console_side (side);
	memset (s.tabpartial, 0, MAXCMDLINE);
	q_strlcpy (s.tabpartial, text, MAXCMDLINE);
}

typedef struct
{
	int	  linewidth;
	int	  buffersize;
	int	  totallines;
	int	  backscroll;
	int	  current;
	int	  x;
	int	  vislines;
	int	  initialized;
	int	  debuglog;
	int	  forcedup;
	int	  redirected;
	int	  tablistlen;
	float times[NUM_CON_TIMES];
	char  lastcenter[1024];
	char  redirect[8192];
	char  tabpartial[MAXCMDLINE];
} ctest_console_state_t;

void ctest_console_snapshot (int side, ctest_console_state_t *out)
{
	ctest_console_side_t s = ctest_console_side (side);

	memset (out, 0, sizeof (*out));
	out->linewidth = *s.linewidth;
	out->buffersize = *s.buffersize;
	out->totallines = *s.totallines;
	out->backscroll = *s.backscroll;
	out->current = *s.current;
	out->x = *s.x;
	out->vislines = *s.vislines;
	out->initialized = *s.initialized ? 1 : 0;
	out->debuglog = *s.debuglog ? 1 : 0;
	out->forcedup = *s.forcedup ? 1 : 0;
	out->redirected = ctest_console_is_redirected (side) ? 1 : 0;
	out->tablistlen = ctest_console_tablist_count (side);
	memcpy (out->times, s.times, sizeof (out->times));
	memcpy (out->lastcenter, s.lastcenter, sizeof (out->lastcenter));
	memcpy (out->redirect, s.redirect, sizeof (out->redirect));
	memcpy (out->tabpartial, s.tabpartial, MAXCMDLINE);
}

/* The scrollback itself. Lines come back in ring order (0 is the oldest line
 * still in the buffer) so the two sides can be compared without either test
 * having to know where con_current happens to sit. */
int ctest_console_buffer_size (int side)
{
	ctest_console_side_t s = ctest_console_side (side);
	return *s.buffersize;
}

void ctest_console_get_line (int side, int line, char *out, int cap)
{
	ctest_console_side_t s = ctest_console_side (side);
	int					 w = *s.linewidth;
	int					 n;

	memset (out, 0, (size_t)cap);
	if (!*s.text || w <= 0 || *s.totallines <= 0)
		return;
	line %= *s.totallines;
	if (line < 0)
		line += *s.totallines;
	n = cap - 1 < w ? cap - 1 : w;
	memcpy (out, *s.text + line * w, (size_t)n);
}

/* ---- shared engine state both halves read ------------------------------
 * None of these belong to console.c, so there is one copy of each and a test
 * has to re-seed them before it runs the second side. */

extern int	 history_line; /* keys.c:81, declared nowhere public but at console.c:743 */

void ctest_console_set_vid (int conwidth, int conheight, int width, int height)
{
	vid.conwidth = conwidth;
	vid.conheight = conheight;
	vid.width = width;
	vid.height = height;
}

void ctest_console_set_gl (int w, int h, float concurrent)
{
	glwidth = w;
	glheight = h;
	scr_con_current = concurrent;
}

void ctest_console_set_keydest (int dest)
{
	key_dest = (keydest_t)dest;
}

int ctest_console_get_keydest (void)
{
	return (int)key_dest;
}

/* The console edit line lives in keys.c. console.c reads it in
 * Con_TabComplete and Con_DrawInput and writes it in Con_ToggleConsole_f and
 * Con_TabComplete, so it is both seed and observation. */
void ctest_console_set_editline (const char *text, int linepos, int insert)
{
	edit_line = 0;
	history_line = 0;
	memset (key_lines[0], 0, MAXCMDLINE);
	q_strlcpy (key_lines[0], text, MAXCMDLINE);
	key_linepos = linepos;
	key_insert = insert;
	memset (key_tabhint, 0, MAXCMDLINE);
}

void ctest_console_get_editline (char *out, int cap, int *linepos, int *hist)
{
	memset (out, 0, (size_t)cap);
	q_strlcpy (out, key_lines[edit_line], (size_t)cap);
	*linepos = key_linepos;
	*hist = history_line;
}

void ctest_console_get_tabhint (char *out, int cap)
{
	memset (out, 0, (size_t)cap);
	q_strlcpy (out, key_tabhint, (size_t)cap);
}

void ctest_console_set_chat_team (qboolean v)
{
	chat_team = v;
}

qboolean ctest_console_get_chat_team (void)
{
	return chat_team;
}

/* c_ref_prelude.h:1744/1783 renames cl and cls for every oracle TU, so the
 * two halves read different objects. Both are pure inputs here -- no Con_*
 * function writes either -- so the seeders keep the two copies in step
 * instead of taking a side argument. */
extern client_static_t c_ref_cls;
extern client_state_t  c_ref_cl;

void ctest_console_set_cls (int state, int signon, qboolean demoplayback, qboolean demoseeking)
{
	cls.state = c_ref_cls.state = (cactive_t)state;
	cls.signon = c_ref_cls.signon = signon;
	cls.demoplayback = c_ref_cls.demoplayback = demoplayback;
	cls.demoseeking = c_ref_cls.demoseeking = demoseeking;
}

void ctest_console_set_cl_gametype (int gametype)
{
	cl.gametype = c_ref_cl.gametype = gametype;
}

void ctest_console_set_time (double now, double rawframetime)
{
	realtime = now;
	host_rawframetime = rawframetime;
}

void ctest_console_set_scr_disabled (qboolean v)
{
	scr_disabled_for_loading = v;
}

/* Con_Dump_f (console.c:857) resolves its output path under com_gamedir, and
 * common_fs.c IS an oracle source, so the prelude renamed it TU-wide and the
 * two sides have separate copies; the same #undef dance stubs/keys_ref.c:1027
 * uses gets at the port's.  */
#undef com_gamedir
extern char com_gamedir[MAX_OSPATH];
extern char c_ref_com_gamedir[MAX_OSPATH];

void ctest_console_set_gamedir (int side, const char *dir)
{
	q_strlcpy (side ? c_ref_com_gamedir : com_gamedir, dir, MAX_OSPATH);
}

/* Con_Dump_f reads Cmd_Argc/Cmd_Argv, and cmd.c is an oracle source too, so
 * the argument vector is per-side as well. cmd.h was included by the prelude
 * with the rename block live, so the plain spelling has to be re-declared. */
#undef Cmd_TokenizeString
void Cmd_TokenizeString (const char *text);

void ctest_console_tokenize (int side, const char *text)
{
	if (side)
		c_ref_Cmd_TokenizeString (text);
	else
		Cmd_TokenizeString (text);
}

/* keydown (keys.c:78) has one definition in this link -- quake-capi's keys
 * port -- and both halves of Con_TabComplete read keydown[K_SHIFT], so it is
 * seeded once per scenario like key_dest is. */
void ctest_console_set_keydown (int key, qboolean down)
{
	if (key >= 0 && key < MAX_KEYS)
		keydown[key] = down;
}

/* ---- the shared call recorder ------------------------------------------- */

void ctest_console_reset_calls (void)
{
	memset (&ctest_console_calls, 0, sizeof (ctest_console_calls));
	ctest_console_explore_result = true;
	ctest_console_mousex = 0;
	ctest_console_mousey = 0;
	ctest_console_clear_draw_log ();
	ctest_console_clear_redirect_output ();
	ctest_console_last_link[0] = '\0';
}

void ctest_console_get_calls (ctest_console_calls_t *out)
{
	*out = ctest_console_calls;
}

void ctest_console_set_mouse (int x, int y)
{
	ctest_console_mousex = x;
	ctest_console_mousey = y;
}

void ctest_console_set_explore_result (qboolean v)
{
	ctest_console_explore_result = v;
}

const char *ctest_console_clipboard (void)
{
	return ctest_console_calls.clipboard;
}
