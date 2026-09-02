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
// console_glue.c -- the C frame around the Rust console port.
//
// Compiled instead of console.c under -Duse_rust_host (Rust migration Phase 7
// M10c), mirroring keys_glue.c, cl_input_glue.c and view_glue.c:
//
//  1. Own the C-visible objects console.c defined: the scrollback buffer and
//     its geometry (con_text/con_linewidth/con_totallines/con_current/con_x/
//     con_backscroll/con_buffersize/con_vislines/con_forcedup), the six
//     console cvars, con_times[], con_lastcenterstring, the rcon redirect
//     pair, con_debuglog/con_initialized, con_cursorspeed, con_mutex and the
//     shared key_tabpartial. Every one of them had external linkage in the
//     original and has live C readers (keys.c:245, gl_screen.c, host.c,
//     sv_main.c, net_dgrm.c), so the storage stays here and Rust reaches it
//     through externs (ADR-007). console.c's file-statics move to Rust,
//     `tablist` and the `tab_t` it points at also stay here: they had
//     external linkage in the original, and keeping the list observable from
//     C is what lets the ctest oracle compare tab-completion ordering.
//  2. Keep every C-variadic entry point here. Con_Printf, Con_DWarning,
//     Con_Warning, Con_DPrintf, Con_DPrintf2, Con_LinkPrintf, Con_SafePrintf
//     and Con_CenterPrintf q_vsnprintf into a char[MAXPRINTMSG] exactly as
//     console.c did and hand the finished string to Rust. libc keeps doing
//     the formatting, so the emitted bytes are unchanged -- ADR-005's Rust
//     float formatter is deliberately NOT in this path.
//  3. Keep Con_Printf's screen-update tail in C, including its `static
//     qboolean inupdate`: SCR_UpdateScreen (false) reaches Mod_LoadModel
//     (mod, true) -> Host_Error at gl_model.c:531, and ADR-009 rule 3 forbids
//     a longjmp crossing a Rust frame. Con_NotifyBox stays entirely in C for
//     the same reason (it drives SCR_UpdateScreen in a loop).
//  4. Guard what the Rust side reaches that can Host_Error / Host_EndGame
//     (ADR-009 rule 3): M_Menu_Main_f (console.c:763) and the two tab
//     completion callbacks (console.c:1858, :1866), which are QC- and
//     mod-supplied and reach PR_GetString. The two entry points that can
//     reach a guard -- Con_ToggleConsole_f and Con_TabComplete -- are thin
//     wrappers over quake_rs_* status cores, and Host_Reraise is called only
//     here.
//  5. Leave everything else plain. S_LocalSound (snd_dma.c:1135 reaches only
//     Con_Printf and S_StartSound), Sys_Explore, SCR_EndLoadingPlaque
//     (gl_screen.c:993, two assignments), IN_Activate /
//     IN_DeactivateForConsole / IN_GetMousePos, VID_SetMouseCursor (pure
//     SDL), the Draw_*/GL_SetCanvas* renderer entry points, Cvar_* and Cmd_*
//     and the string helpers cannot longjmp, so the Rust side calls them
//     directly.
//  6. Own the plain names of four non-raising entry points -- Con_SelectAll,
//     Con_ForceMouseMove, Con_CopySelectionToClipboard and Con_Scroll. They
//     are exported from Rust as quake_rs_* instead, because keys.c calls all
//     four and quake-ctest/stubs/keys_ref.c has to keep counting those calls
//     with link doubles that own the plain names in that link.
//
// Con_DrawNotify, Con_DrawInput, Con_DrawConsole and LOG_Init keep plain C
// forwards rather than direct Rust exports because cb_context_t and
// quakeparms_t have no cbindgen spelling; none of them needs a guard.
// SDL_SetClipboardText and the ENGINE_NAME_AND_VER macro are shimmed here so
// that no Rust translation unit names an SDL symbol (check_headers.sh) or has
// to reproduce a build-time string macro.

#include "quakedef.h"
#include "arch_def.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

#include <time.h>

/* ---------------------------------------------------------------------------
 * C-visible objects (console.c:38-79, :1546).
 */

int con_linewidth;

float con_cursorspeed = 4;

int con_buffersize; // johnfitz -- user can now override default

qboolean con_forcedup; // because no entities to refresh

int	  con_totallines; // total lines in console scrollback
int	  con_backscroll; // lines up from bottom to display
int	  con_current;	  // where next message will be printed
int	  con_x;		  // offset in current line for next print
char *con_text = NULL;

cvar_t con_notifytime = {"con_notifytime", "3", CVAR_NONE};			// seconds
cvar_t con_logcenterprint = {"con_logcenterprint", "1", CVAR_NONE}; // johnfitz
cvar_t con_notifycenter = {"con_notifycenter", "0", CVAR_ARCHIVE};
cvar_t con_notifyfade = {"con_notifyfade", "0", CVAR_ARCHIVE};
cvar_t con_notifyfadetime = {"con_notifyfadetime", "0.5", CVAR_ARCHIVE};
cvar_t con_maxcols = {"con_maxcols", "0", CVAR_ARCHIVE};

char con_lastcenterstring[1024];				 // johnfitz
void (*con_redirect_flush) (const char *buffer); // call this to flush the redirection buffer (for rcon)
char con_redirect_buffer[8192];

#define NUM_CON_TIMES 4
float con_times[NUM_CON_TIMES]; // realtime time the line was generated
								// for transparent notify lines

int con_vislines;

qboolean con_debuglog = false;

qboolean con_initialized;

qmutex_t *con_mutex;

char key_tabpartial[MAXCMDLINE];

/* console.c:1547-1555. tablist had external linkage in the original, so the
 * list head and the node layout stay on this side of the seam; the port
 * reaches both through quake-c-sys (ADR-007, ADR-011). */
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
 * ADR-009 trampolines. Every raise-capable callee the Rust port reaches runs
 * inside Host_Guard here, never from a Rust frame.
 */

/* console.c:763 -- the menus run console commands, load maps and start demos,
   so M_Menu_Main_f is raise-capable from many directions. */
static void Console_InvokeMenuMain (void *p)
{
	(void)p;
	M_Menu_Main_f ();
}

int Console_Glue_MenuMain (void)
{
	return Host_Guard (Console_InvokeMenuMain, NULL);
}

/* console.c:1858 -- cvar->completion is supplied by mods/QC and reaches
   PR_GetString, so it can Host_Error. */
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

/* console.c:1866 -- same reasoning for a command's completion callback. */
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
 * Non-guard shims: SDL and two build-time strings.
 */

/* console.c:834 */
void Console_Glue_SetClipboardText (const char *text)
{
	SDL_SetClipboardText (text);
}

/* quakever.h:59 */
const char *Console_Glue_EngineNameAndVer (void)
{
	return ENGINE_NAME_AND_VER;
}

/* console.c:2440 -- _IONBF is 2 on glibc/musl/BSD but 4 on the MSVC CRT. */
void Console_Glue_LogSetUnbuffered (FILE *f)
{
	setvbuf (f, NULL, _IONBF, 0); // keep the log complete on crashes
}

/* console.c:2437 -- stderr is a macro on several libcs. */
void Console_Glue_LogOpenFailed (const char *name)
{
	fprintf (stderr, "Error: Unable to create log file %s\n", name);
}

/* ---------------------------------------------------------------------------
 * Re-raising public entry points. The Rust bodies are quake_rs_* status cores
 * and the jump is re-issued from here, never from a Rust frame (ADR-009).
 */

/* console.c:757 -- reaches M_Menu_Main_f. */
void Con_ToggleConsole_f (void)
{
	int r = quake_rs_con_toggle_console_f ();
	Host_Reraise (r);
}

/* console.c:1988 -- reaches the completion callbacks through BuildTabList. */
void Con_TabComplete (tabcomplete_t mode)
{
	int r = quake_rs_con_tab_complete ((int)mode);
	Host_Reraise (r);
}

/* ---------------------------------------------------------------------------
 * Plain wrappers over four non-raising quake_rs_* entry points. keys.c calls
 * all four, and quake-ctest/stubs/keys_ref.c has to keep counting those calls
 * with its own link doubles, so the port exports them under quake_rs_* names
 * and the plain names live here.
 */

/* console.c:403 */
void Con_SelectAll (void)
{
	quake_rs_con_select_all ();
}

/* console.c:665 */
void Con_ForceMouseMove (void)
{
	quake_rs_con_force_mouse_move ();
}

/* console.c:800 */
qboolean Con_CopySelectionToClipboard (void)
{
	return quake_rs_con_copy_selection_to_clipboard ();
}

/* console.c:1023 */
void Con_Scroll (int lines)
{
	quake_rs_con_scroll (lines);
}

/* ---------------------------------------------------------------------------
 * The C-variadic console entry points (console.c:1237-1461). Each formats
 * with libc exactly as the original did and hands the finished string to
 * Rust; nothing about the emitted bytes changes.
 */

#define MAXPRINTMSG 4096

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
	// also echo to debugging console
	Sys_Printf ("%s", quake_rs_con_strip_control_prefixes (msg));

	// log all messages to file
	if (con_debuglog)
		Con_DebugLog (msg);

	if (!con_initialized)
		return;

	if (cls.state == ca_dedicated)
		return; // no graphics mode

	// write it to the scrollable buffer
	quake_rs_con_print (msg);

	// update the screen if the console is displayed
	if (cls.signon != SIGNONS && !scr_disabled_for_loading && !Tasks_IsWorker ())
	{
		// protect against infinite loop if something in SCR_UpdateScreen calls
		// Con_Printd
		if (!inupdate)
		{
			inupdate = true;
			SCR_UpdateScreen (false);
			inupdate = false;
		}
	}
}

/* console.c:1306 */
void Con_DWarning (const char *fmt, ...)
{
	va_list argptr;
	char	msg[MAXPRINTMSG];

	if (developer.value >= 2)
	{ // don't confuse non-developers with techie stuff...
		// (this is limit exceeded warnings)

		va_start (argptr, fmt);
		q_vsnprintf (msg, sizeof (msg), fmt, argptr);
		va_end (argptr);

		Con_SafePrintf ("\x02Warning: ");
		Con_SafePrintf ("%s", msg);
	}
}

/* console.c:1328 */
void Con_Warning (const char *fmt, ...)
{
	va_list argptr;
	char	msg[MAXPRINTMSG];

	va_start (argptr, fmt);
	q_vsnprintf (msg, sizeof (msg), fmt, argptr);
	va_end (argptr);

	Con_SafePrintf ("\x02Warning: ");
	Con_SafePrintf ("%s", msg);
}

/* console.c:1346 */
void Con_DPrintf (const char *fmt, ...)
{
	va_list argptr;
	char	msg[MAXPRINTMSG];

	if (!developer.value)
		return; // don't confuse non-developers with techie stuff...

	va_start (argptr, fmt);
	q_vsnprintf (msg, sizeof (msg), fmt, argptr);
	va_end (argptr);

	Con_SafePrintf ("%s", msg); // johnfitz -- was Con_Printf
}

/* console.c:1367 */
void Con_DPrintf2 (const char *fmt, ...)
{
	va_list argptr;
	char	msg[MAXPRINTMSG];

	if (developer.value >= 2)
	{
		va_start (argptr, fmt);
		q_vsnprintf (msg, sizeof (msg), fmt, argptr);
		va_end (argptr);
		Con_Printf ("%s", msg);
	}
}

/* console.c:1383 -- the link bookkeeping (and the con_mutex window around it)
   lives in Rust; only the formatting stays here. */
void Con_LinkPrintf (const char *addr, const char *fmt, ...)
{
	va_list argptr;
	char	msg[MAXPRINTMSG];

	va_start (argptr, fmt);
	q_vsnprintf (msg, sizeof (msg), fmt, argptr);
	va_end (argptr);

	quake_rs_con_link_print (addr, msg);
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

	QMutex_Lock (con_mutex);
	temp = scr_disabled_for_loading;
	scr_disabled_for_loading = true;
	Con_Printf ("%s", msg);
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
 * Plain forwards. None needs a guard; each exists only because its parameter
 * type has no cbindgen spelling.
 */

/* console.c:2131 */
void Con_DrawNotify (cb_context_t *cbx)
{
	quake_rs_con_draw_notify (cbx);
}

/* console.c:2201 */
void Con_DrawInput (cb_context_t *cbx)
{
	quake_rs_con_draw_input (cbx);
}

/* console.c:2277 */
void Con_DrawConsole (cb_context_t *cbx, int lines, qboolean drawinput)
{
	quake_rs_con_draw_console (cbx, lines, drawinput);
}

/* console.c:2409 -- the -condebug check and the session timestamp stay here
   (localtime/strftime and the quakeparms_t layout); everything after them is
   Rust. */
void LOG_Init (quakeparms_t *parms)
{
	time_t inittime;
	char   session[24];

	// always activate the console log in Debug mode
#if !defined(DEBUG) && !defined(_DEBUG)
	if (!COM_CheckParm ("-condebug"))
		return;
#endif

	inittime = time (NULL);
	strftime (session, sizeof (session), "%m/%d/%Y %H:%M:%S", localtime (&inittime));

	quake_rs_log_init (parms->basedir, session);
}

/* ---------------------------------------------------------------------------
 * Con_NotifyBox stays entirely in C: it drives SCR_UpdateScreen in a loop
 * (console.c:2393), which reaches Host_Error (ADR-009 rule 3).
 */

/* console.c:2375 */
void Con_NotifyBox (const char *text)
{
	double t1, t2;
	int	   lastkey, lastchar;

	// during startup for sound / cd warnings
	Con_Printf ("\n\n%s", Con_Quakebar (40)); // johnfitz
	Con_Printf ("%s", text);
	Con_Printf ("Press a key.\n");
	Con_Printf ("%s", Con_Quakebar (40)); // johnfitz

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
		realtime += t2 - t1; // make the cursor blink
	} while (lastkey == -1 && lastchar == -1);
	Key_EndInputGrab ();

	Con_Printf ("\n");
	IN_Activate ();
	key_dest = key_game;
	realtime = 0; // put the cursor back to invisible
}

#endif /* USE_RUST_HOST */
