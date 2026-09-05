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
// keys_glue.c -- the C frame around the Rust key/console-input port.
//
// Compiled instead of keys.c under -Duse_rust_host (Rust migration Phase 7
// M10b), mirroring cl_input_glue.c and view_glue.c:
//
//  1. Own the C-visible objects keys.c defined: the console edit line
//     (key_lines/key_tabhint/key_linepos/key_insert/key_blinktime/edit_line/
//     history_line), key_dest, keybindings[], consolekeys[], menubound[],
//     keydown[], chat_team and the keynames[] table. Every one of them had
//     external linkage in the original, and console.c:37/:690/:743/:2056 and
//     menu.c:114 resolve keydown[] and history_line by local re-declaration,
//     so the storage stays here and Rust reaches it through externs (ADR-007).
//     keys.c's six file-statics (chat_buffer, chat_bufferlen, key_inputgrab,
//     Key_Console's `current`, Key_KeynumToString's `tinystr` and
//     Key_UpdateForDest's `forced`) were not C-visible and move to Rust.
//  2. Guard what keys.c reached that can Host_Error / Host_EndGame (ADR-009
//     rule 3): SCR_UpdateScreen, the console's Con_TabComplete / Con_Scroll /
//     Con_ForceMouseMove / Con_SelectAll / Con_CopySelectionToClipboard /
//     Con_ToggleConsole_f (each redraws or reloads through paths that reach
//     Host_Error), the menu's M_Keydown / M_Charinput / M_ToggleMenu_f (which
//     run console commands and load maps), and VID_Toggle.
//  3. Re-raise, from a pure C frame, what those guards caught. The eight
//     entry points that can reach a guard -- Key_Console, Char_Console,
//     Key_Event, Key_EventWithKeycode, Char_Event, Key_ClearStates,
//     Key_BeginInputGrab and Key_EndInputGrab -- are thin wrappers over
//     quake_rs_* status cores, and Host_Reraise is called only here.
//  4. Leave everything else plain. Cbuf_AddText (quake-capi cmd.rs:146 keeps
//     it off SZ_Write's Host_Error path), Cmd_AddCommand2, Cmd_Argc/Cmd_Argv,
//     Con_Printf/Con_SafePrintf, PL_GetClipboardData, M_TextEntry,
//     M_WaitingForKeyBinding, IN_UpdateInputMode/IN_Activate/
//     IN_DeactivateForConsole, q_strdup/q_strcasecmp/q_strlcat, the stdio
//     history helpers and Sys_Error (which terminates rather than jumping)
//     cannot longjmp, so the Rust side calls them directly.
//
// Key_WriteBindings keeps a plain C forward rather than a direct Rust export
// because its FILE * parameter has no cbindgen spelling; it needs no guard.

#include "quakedef.h"
#include "arch_def.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

/* ---------------------------------------------------------------------------
 * C-visible objects (keys.c:31-46, :54-142, :539).
 */

char key_lines[CMDLINES][MAXCMDLINE];
char key_tabhint[MAXCMDLINE];

int	   key_linepos;
int	   key_insert = 1; // johnfitz -- insert key toggle (for editing)
double key_blinktime;  // johnfitz -- fudge cursor blinking to make it easier to spot in certain cases

int edit_line = 0;
int history_line = 0;

keydest_t key_dest;

char	*keybindings[MAX_KEYS];
qboolean consolekeys[MAX_KEYS]; // if true, can't be rebound while in console
qboolean menubound[MAX_KEYS];	// if true, can't be rebound while in menu
qboolean keydown[MAX_KEYS];

qboolean chat_team = false;

typedef struct
{
	const char *name;
	int			keynum;
} keyname_t;

keyname_t keynames[] = {
	{"TAB", K_TAB},
	{"ENTER", K_ENTER},
	{"ESCAPE", K_ESCAPE},
	{"SPACE", K_SPACE},
	{"BACKSPACE", K_BACKSPACE},
	{"UPARROW", K_UPARROW},
	{"DOWNARROW", K_DOWNARROW},
	{"LEFTARROW", K_LEFTARROW},
	{"RIGHTARROW", K_RIGHTARROW},

	{"ALT", K_ALT},
	{"CTRL", K_CTRL},
	{"SHIFT", K_SHIFT},

	//	{"KP_NUMLOCK", K_KP_NUMLOCK},
	{"KP_SLASH", K_KP_SLASH},
	{"KP_STAR", K_KP_STAR},
	{"KP_MINUS", K_KP_MINUS},
	{"KP_HOME", K_KP_HOME},
	{"KP_UPARROW", K_KP_UPARROW},
	{"KP_PGUP", K_KP_PGUP},
	{"KP_PLUS", K_KP_PLUS},
	{"KP_LEFTARROW", K_KP_LEFTARROW},
	{"KP_5", K_KP_5},
	{"KP_RIGHTARROW", K_KP_RIGHTARROW},
	{"KP_END", K_KP_END},
	{"KP_DOWNARROW", K_KP_DOWNARROW},
	{"KP_PGDN", K_KP_PGDN},
	{"KP_ENTER", K_KP_ENTER},
	{"KP_INS", K_KP_INS},
	{"KP_DEL", K_KP_DEL},

	{"F1", K_F1},
	{"F2", K_F2},
	{"F3", K_F3},
	{"F4", K_F4},
	{"F5", K_F5},
	{"F6", K_F6},
	{"F7", K_F7},
	{"F8", K_F8},
	{"F9", K_F9},
	{"F10", K_F10},
	{"F11", K_F11},
	{"F12", K_F12},

	{"INS", K_INS},
	{"DEL", K_DEL},
	{"PGDN", K_PGDN},
	{"PGUP", K_PGUP},
	{"HOME", K_HOME},
	{"END", K_END},

	{"COMMAND", K_COMMAND},

	{"MOUSE1", K_MOUSE1},
	{"MOUSE2", K_MOUSE2},
	{"MOUSE3", K_MOUSE3},
	{"MOUSE4", K_MOUSE4},
	{"MOUSE5", K_MOUSE5},

	{"PAUSE", K_PAUSE},

	{"MWHEELUP", K_MWHEELUP},
	{"MWHEELDOWN", K_MWHEELDOWN},

	{"SEMICOLON", ';'}, // because a raw semicolon seperates commands

	{"BACKQUOTE", '`'}, // because a raw backquote may toggle the console
	{"TILDE", '~'},		// because a raw tilde may toggle the console

	{"LTHUMB", K_LTHUMB},
	{"RTHUMB", K_RTHUMB},
	{"LSHOULDER", K_LSHOULDER},
	{"RSHOULDER", K_RSHOULDER},
	{"ABUTTON", K_ABUTTON},
	{"BBUTTON", K_BBUTTON},
	{"XBUTTON", K_XBUTTON},
	{"YBUTTON", K_YBUTTON},
	{"LTRIGGER", K_LTRIGGER},
	{"RTRIGGER", K_RTRIGGER},
	{"MISC1", K_MISC1},
	{"PADDLE1", K_PADDLE1},
	{"PADDLE2", K_PADDLE2},
	{"PADDLE3", K_PADDLE3},
	{"PADDLE4", K_PADDLE4},
	{"TOUCHPAD", K_TOUCHPAD},

	{NULL, 0}};

/* ---------------------------------------------------------------------------
 * Guarded callbacks (ADR-009 rule 3).
 */

/* keys.c:285 -- forces a redraw so a slow command shows progress.
   SCR_UpdateScreen reaches Mod_LoadModel (gl_model.c:531), which Host_Errors
   on a missing model. */
static void Keys_InvokeUpdateScreen (void *p)
{
	(void)p;
	SCR_UpdateScreen (false);
}

int Keys_Glue_UpdateScreen (void)
{
	return Host_Guard (Keys_InvokeUpdateScreen, NULL);
}

/* keys.c:289 and the sixteen TABCOMPLETE_AUTOHINT sites -- Con_TabComplete
   walks the command/cvar/map lists and prints through Con_Printf. */
static void Keys_InvokeTabComplete (void *p)
{
	Con_TabComplete (*(tabcomplete_t *)p);
}

int Keys_Glue_TabComplete (int mode)
{
	tabcomplete_t m = (tabcomplete_t)mode;
	return Host_Guard (Keys_InvokeTabComplete, &m);
}

/* keys.c:352, :357 */
static void Keys_InvokeScroll (void *p)
{
	Con_Scroll (*(int *)p);
}

int Keys_Glue_Scroll (int lines)
{
	return Host_Guard (Keys_InvokeScroll, &lines);
}

/* keys.c:339, :349 */
static void Keys_InvokeForceMouseMove (void *p)
{
	(void)p;
	Con_ForceMouseMove ();
}

int Keys_Glue_ForceMouseMove (void)
{
	return Host_Guard (Keys_InvokeForceMouseMove, NULL);
}

/* keys.c:451 */
static void Keys_InvokeSelectAll (void *p)
{
	(void)p;
	Con_SelectAll ();
}

int Keys_Glue_SelectAll (void)
{
	return Host_Guard (Keys_InvokeSelectAll, NULL);
}

/* keys.c:437, :461 -- the result is only meaningful when the guard says the
   call completed, so the Rust side checks the status before reading *out. */
static void Keys_InvokeCopySelectionToClipboard (void *p)
{
	*(qboolean *)p = Con_CopySelectionToClipboard ();
}

int Keys_Glue_CopySelectionToClipboard (qboolean *out)
{
	return Host_Guard (Keys_InvokeCopySelectionToClipboard, out);
}

/* keys.c:1077 */
static void Keys_InvokeToggleConsole (void *p)
{
	(void)p;
	Con_ToggleConsole_f ();
}

int Keys_Glue_ToggleConsole (void)
{
	return Host_Guard (Keys_InvokeToggleConsole, NULL);
}

/* keys.c:1087, :1182 -- the menus run console commands, load maps and start
   demos, so M_Keydown is raise-capable from many directions. */
static void Keys_InvokeMenuKeydown (void *p)
{
	M_Keydown (*(int *)p);
}

int Keys_Glue_MenuKeydown (int key)
{
	return Host_Guard (Keys_InvokeMenuKeydown, &key);
}

/* keys.c:1226 */
static void Keys_InvokeMenuCharinput (void *p)
{
	M_Charinput (*(int *)p);
}

int Keys_Glue_MenuCharinput (int key)
{
	return Host_Guard (Keys_InvokeMenuCharinput, &key);
}

/* keys.c:1091, :1145 */
static void Keys_InvokeToggleMenu (void *p)
{
	(void)p;
	M_ToggleMenu_f ();
}

int Keys_Glue_ToggleMenu (void)
{
	return Host_Guard (Keys_InvokeToggleMenu, NULL);
}

/* keys.c:1040 -- VID_Toggle restarts the swapchain and Host_Errors on failure. */
static void Keys_InvokeVidToggle (void *p)
{
	(void)p;
	VID_Toggle ();
}

int Keys_Glue_VidToggle (void)
{
	return Host_Guard (Keys_InvokeVidToggle, NULL);
}

/* ---------------------------------------------------------------------------
 * Re-raising public entry points. The Rust bodies are quake_rs_* status cores
 * and the jump is re-issued from here, never from a Rust frame (ADR-009).
 */

/* keys.c:258 -- external linkage in the original but declared in no header,
   like Char_Console below; the plain name stays a real C function so the
   linkage matches. */
void Key_Console (int key)
{
	int r = quake_rs_key_console (key);
	Host_Reraise (r);
}

/* keys.c:499 */
void Char_Console (int key)
{
	int r = quake_rs_char_console (key);
	Host_Reraise (r);
}

/* keys.c:1017 */
void Key_Event (int key, qboolean down)
{
	int r = quake_rs_key_event (key, down);
	Host_Reraise (r);
}

/* keys.c:1033 */
void Key_EventWithKeycode (int key, qboolean down, int keycode)
{
	int r = quake_rs_key_event_with_keycode (key, down, keycode);
	Host_Reraise (r);
}

/* keys.c:1215 */
void Char_Event (int key)
{
	int r = quake_rs_char_event (key);
	Host_Reraise (r);
}

/* keys.c:1284 */
void Key_ClearStates (void)
{
	int r = quake_rs_key_clear_states ();
	Host_Reraise (r);
}

/* keys.c:967 */
void Key_BeginInputGrab (void)
{
	int r = quake_rs_key_begin_input_grab ();
	Host_Reraise (r);
}

/* keys.c:981 */
void Key_EndInputGrab (void)
{
	int r = quake_rs_key_end_input_grab ();
	Host_Reraise (r);
}

/* ---------------------------------------------------------------------------
 * Plain forward. Key_WriteBindings needs no guard -- fprintf and the Rust
 * Key_KeynumToString are its only callees -- but its FILE * parameter has no
 * cbindgen spelling, so the public name is defined here (keys.c:782).
 */
void Key_WriteBindings (FILE *f)
{
	quake_rs_key_write_bindings (f);
}

#endif /* USE_RUST_HOST */
