/* Phase 7 M10b oracle TU for Quake/keys.c.
 *
 * WHY THIS FILE COMPOSES keys.c INSTEAD OF build.rs LISTING IT IN C_SOURCES
 *
 * The prelude's c_ref_* renames are translation-unit-wide by construction:
 * one #define rewrites the definition in the oracle source AND every call in
 * every other oracle source. For keys.c that is wrong, because four of the
 * symbols keys.c owns already have hand-written link doubles that existing
 * tests assert on:
 *
 *   stubs/host_ref.c:201 defines a counting Key_WriteBindings, and
 *   tests/host_differential.rs:1197 asserts Host_WriteConfiguration writes
 *   exactly "vid_restart\n+mlook\n" while calling it once. Repointing host.c
 *   at the real keys.c would make that text depend on cfg_unbindall and on
 *   whatever the binding table happens to hold.
 *
 *   stubs/host_ref.c:257-264 defines aborting doubles for Key_Init,
 *   Key_UpdateForDest, History_Shutdown and IN_UpdateInputMode. Those are
 *   deliberate: Host_Init and Host_Shutdown are not driven by that suite, and
 *   an abort is how it says so.
 *
 * So the rename layer for keys.c lives HERE, in keys.c's own translation
 * unit, where it renames keys.c's definitions and keys.c's internal calls and
 * nothing else. host_ref.c keeps its four doubles under TU-local names of its
 * own (added in the same change), and the plain names below belong to the
 * port -- quake-capi/src/keys.rs for the twenty-seven entry points, this file
 * for the glue-owned data and the ADR-009 trampolines, exactly as
 * Quake/keys_glue.c does in the engine build.
 *
 * ONE SYMBOL IS DELIBERATELY NOT RENAMED: key_dest. Nineteen files read it,
 * including the cl_demo.c, cl_main.c, snd_mix.c, sv_user.c, view.c, host.c
 * and host_cmd.c oracles plus stubs.c:2717 and stubs/sv_user_ref.c; renaming
 * it here would split them across two objects for no gain, because keys.c is
 * the only writer either way. It stays one shared object, which means a test
 * that cares about it must set it before each side runs and read it straight
 * after -- the same rule stubs/chase_ref.c states for r_refdef.
 *
 * The same holds for the console/menu/input/video callees near the bottom of
 * this file: console.c, menu.c, in_sdl.c and gl_vidsdl.c are not oracle
 * sources, so both sides call the SAME recorder. Tests reset the recorder
 * immediately before each side's invocation and read it immediately after.
 *
 * COST, stated so it is not discovered later: scripts/harness/
 * check_ctest_symbols.sh reads C_SOURCES out of build.rs, so it does not
 * inspect this object; build.rs watches Quake/keys.c explicitly instead.
 */

/* ---- keys.c rename block ------------------------------------------------
 * Every file-scope symbol Quake/keys.c defines with external linkage, plus
 * the file-local keyname_t typedef (the plain half needs the same tag for
 * its own keynames[] copy, and C forbids redefining a typedef name).
 * key_dest is the one deliberate omission; see the header comment.
 */
#define keyname_t c_ref_keyname_t

/* SCR_UpdateScreen is renamed for the WHOLE of this TU -- both keys.c's own
 * call at keys.c:285 and the plain half's Keys_Glue_UpdateScreen trampoline --
 * so the two sides land on the recorder at the bottom of this file. It cannot
 * simply be defined plain here: stubs/host_ref.c:256 already owns that name as
 * an aborting double, and stubs/net_dgrm_orch_glue_ref.c:40 records that it
 * deliberately relies on that abort. */
#define SCR_UpdateScreen ctest_keys_SCR_UpdateScreen

/* data (keys.c:31-46, :54, :539) */

#define key_lines     c_ref_key_lines
#define key_tabhint   c_ref_key_tabhint
#define key_linepos   c_ref_key_linepos
#define key_insert    c_ref_key_insert
#define key_blinktime c_ref_key_blinktime
#define edit_line     c_ref_edit_line
#define history_line  c_ref_history_line
#define keybindings   c_ref_keybindings
#define consolekeys   c_ref_consolekeys
#define menubound     c_ref_menubound
#define keydown       c_ref_keydown
#define chat_team     c_ref_chat_team
#define keynames      c_ref_keynames

/* functions (keys.c:258-1301) */
#define Key_Console          c_ref_Key_Console
#define Char_Console         c_ref_Char_Console
#define Key_GetChatBuffer    c_ref_Key_GetChatBuffer
#define Key_GetChatMsgLen    c_ref_Key_GetChatMsgLen
#define Key_EndChat          c_ref_Key_EndChat
#define Key_Message          c_ref_Key_Message
#define Char_Message         c_ref_Char_Message
#define Key_StringToKeynum   c_ref_Key_StringToKeynum
#define Key_KeynumToString   c_ref_Key_KeynumToString
#define Key_SetBinding       c_ref_Key_SetBinding
#define Key_Unbind_f         c_ref_Key_Unbind_f
#define Key_Unbindall_f      c_ref_Key_Unbindall_f
#define Key_Bindlist_f       c_ref_Key_Bindlist_f
#define Key_Bind_f           c_ref_Key_Bind_f
#define Key_WriteBindings    c_ref_Key_WriteBindings
#define History_Init         c_ref_History_Init
#define History_Shutdown     c_ref_History_Shutdown
#define Key_Init             c_ref_Key_Init
#define Key_BeginInputGrab   c_ref_Key_BeginInputGrab
#define Key_EndInputGrab     c_ref_Key_EndInputGrab
#define Key_GetGrabbedInput  c_ref_Key_GetGrabbedInput
#define Key_Event            c_ref_Key_Event
#define Key_EventWithKeycode c_ref_Key_EventWithKeycode
#define Char_Event           c_ref_Char_Event
#define Key_TextEntry        c_ref_Key_TextEntry
#define Key_ClearStates      c_ref_Key_ClearStates
#define Key_UpdateForDest    c_ref_Key_UpdateForDest

/* keys.h:158-179 was force-included by the prelude ahead of the block above,
 * so keys.c's renamed definitions would have no visible prototype and its
 * forward calls (Key_ClearStates from Key_BeginInputGrab at keys.c:974,
 * Key_Console from Key_EventWithKeycode at keys.c:1129, ...) would fall back
 * to implicit int. Re-declaring them here costs nothing: the macros above
 * rewrite each line, so the text is a verbatim copy of keys.h:158-179 plus
 * the five entry points keys.c declares in no header at all. */
void		Key_Init (void);
void		Key_ClearStates (void);
void		Key_UpdateForDest (void);
void		Key_BeginInputGrab (void);
void		Key_EndInputGrab (void);
void		Key_GetGrabbedInput (int *lastkey, int *lastchar);
void		Key_Event (int key, qboolean down);
void		Key_EventWithKeycode (int key, qboolean down, int keycode);
void		Char_Event (int key);
qboolean	Key_TextEntry (void);
void		Key_SetBinding (int keynum, const char *binding);
const char *Key_KeynumToString (int keynum);
void		Key_WriteBindings (FILE *f);
void		Key_EndChat (void);
const char *Key_GetChatBuffer (void);
int			Key_GetChatMsgLen (void);
void		History_Init (void);
void		History_Shutdown (void);

/* keys.c:258, :499, :555, :582, :602 -- external linkage, no header. */
void Key_Console (int key);
void Char_Console (int key);
void Key_Message (int key);
void Char_Message (int key);
int	 Key_StringToKeynum (const char *str);

/* Renamed above, so screen.h:37's plain prototype does not cover it and
 * keys.c:285 would call it implicitly. */
void SCR_UpdateScreen (qboolean use_tasks);

/* keys.c:678-735 -- the four console commands Key_Init registers. */
void Key_Unbind_f (void);
void Key_Unbindall_f (void);
void Key_Bindlist_f (void);
void Key_Bind_f (void);

#include <stdio.h>
#include <string.h>

/* Quake/keys.c:41 defines key_dest, and stubs.c defines it too: it is the one
 * symbol this file deliberately leaves unrenamed (see the header comment), so
 * the two halves share one object. Under -fno-common -- the GCC 10+ / clang
 * 15+ default -- that is two strong definitions and every ELF link fails; MSVC
 * merges them silently, which is why only CI ever saw it.
 *
 * stubs.o has to be the owner rather than this file. Ten other oracle objects
 * carry an undefined key_dest (console_ref.o, menu_ref.o, sbar_ref.o,
 * host_ref.o, host_cmd_ref.o, cl_main.o, cl_demo.o, snd_mix.o, sv_user.o,
 * sv_user_ref.o), so if keys_ref.o were the only definition the linker would
 * extract it into EVERY test binary -- dragging in the plain-named recorders
 * below (Con_TabComplete and the Keys_Glue_* trampolines), which are selected
 * ahead of quake-capi's exports and silently replace the port under test.
 * That is a SIGSEGV in console_differential, not a link error.
 *
 * So: stubs.c keeps the strong definition and keys.c's copy is weak here.
 * MSVC has no #pragma weak and does not need one -- it already merges the two
 * tentative definitions. */
#if !defined(_MSC_VER)
#pragma weak key_dest
#endif

#include "keys.c"

/* =========================================================================
 * THE PLAIN HALF -- the ctest-link mirror of Quake/keys_glue.c
 * ========================================================================= */

#undef key_lines
#undef key_tabhint
#undef key_linepos
#undef key_insert
#undef key_blinktime
#undef edit_line
#undef history_line
#undef keybindings
#undef consolekeys
#undef menubound
#undef keydown
#undef chat_team
#undef keynames
#undef keyname_t
#undef cls
#undef Key_Console
#undef Char_Console
#undef Key_GetChatBuffer
#undef Key_GetChatMsgLen
#undef Key_EndChat
#undef Key_Message
#undef Char_Message
#undef Key_StringToKeynum
#undef Key_KeynumToString
#undef Key_SetBinding
#undef Key_Unbind_f
#undef Key_Unbindall_f
#undef Key_Bindlist_f
#undef Key_Bind_f
#undef Key_WriteBindings
#undef History_Init
#undef History_Shutdown
#undef Key_Init
#undef Key_BeginInputGrab
#undef Key_EndInputGrab
#undef Key_GetGrabbedInput
#undef Key_Event
#undef Key_EventWithKeycode
#undef Char_Event
#undef Key_TextEntry
#undef Key_ClearStates
#undef Key_UpdateForDest

/* ---------------------------------------------------------------------------
 * C-visible objects (keys.c:31-46, :54-142, :539), initializers verbatim from
 * Quake/keys_glue.c. key_dest is NOT here: stubs.c owns the one shared
 * definition, and Quake/keys.c's own copy is weakened just above the #include
 * below so the two do not collide.
 */

char key_lines[CMDLINES][MAXCMDLINE];
char key_tabhint[MAXCMDLINE];

int	   key_linepos;
int	   key_insert = 1; // johnfitz -- insert key toggle (for editing)
double key_blinktime;  // johnfitz -- fudge cursor blinking to make it easier to spot in certain cases

int edit_line = 0;
int history_line = 0;

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
 * The port's status cores and the harness's raise machinery.
 */
extern int	Host_Guard (void (*fn) (void *), void *arg);
extern void Host_Reraise (int guard_result);

extern int	quake_rs_key_console (int key);
extern int	quake_rs_char_console (int key);
extern int	quake_rs_key_event (int key, qboolean down);
extern int	quake_rs_key_event_with_keycode (int key, qboolean down, int keycode);
extern int	quake_rs_char_event (int key);
extern int	quake_rs_key_clear_states (void);
extern int	quake_rs_key_begin_input_grab (void);
extern int	quake_rs_key_end_input_grab (void);
extern void quake_rs_key_write_bindings (FILE *f);

/* ---------------------------------------------------------------------------
 * Guarded callbacks (ADR-009 rule 3), bodies verbatim from Quake/keys_glue.c.
 */

/* keys.c:285 */
static void Keys_InvokeUpdateScreen (void *p)
{
	(void)p;
	SCR_UpdateScreen (false);
}

int Keys_Glue_UpdateScreen (void)
{
	return Host_Guard (Keys_InvokeUpdateScreen, NULL);
}

/* keys.c:289 and the sixteen TABCOMPLETE_AUTOHINT sites */
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

/* keys.c:437, :461 */
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

/* keys.c:1087, :1182 */
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

/* keys.c:1040 */
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
 * Re-raising public entry points (keys.c:258, :499, :967, :981, :1017, :1033,
 * :1215, :1284), bodies verbatim from Quake/keys_glue.c.
 */

void Key_Console (int key)
{
	int r = quake_rs_key_console (key);
	Host_Reraise (r);
}

void Char_Console (int key)
{
	int r = quake_rs_char_console (key);
	Host_Reraise (r);
}

void Key_Event (int key, qboolean down)
{
	int r = quake_rs_key_event (key, down);
	Host_Reraise (r);
}

void Key_EventWithKeycode (int key, qboolean down, int keycode)
{
	int r = quake_rs_key_event_with_keycode (key, down, keycode);
	Host_Reraise (r);
}

void Char_Event (int key)
{
	int r = quake_rs_char_event (key);
	Host_Reraise (r);
}

void Key_ClearStates (void)
{
	int r = quake_rs_key_clear_states ();
	Host_Reraise (r);
}

void Key_BeginInputGrab (void)
{
	int r = quake_rs_key_begin_input_grab ();
	Host_Reraise (r);
}

void Key_EndInputGrab (void)
{
	int r = quake_rs_key_end_input_grab ();
	Host_Reraise (r);
}

/* keys.c:782. stubs/host_ref.c owns the counter so that
 * tests/host_differential.rs's write_configuration_guard_ladder_matches_the_port
 * sees the same tally from host.c's TU-local double and from the port's path
 * through Host_Glue_KeyWriteBindings (stubs/host_glue_ref.c:347), which lands
 * here. */
extern int ctest_key_write_bindings_calls;

void Key_WriteBindings (FILE *f)
{
	ctest_key_write_bindings_calls++;
	quake_rs_key_write_bindings (f);
}

/* =========================================================================
 * LINK DOUBLES for the console / menu / input / video callees
 *
 * None of console.c, menu.c, in_sdl.c or gl_vidsdl.c is an oracle source, so
 * every one of these is a single shared object that BOTH sides call. They
 * record rather than abort, because keys.c reaches all of them on paths this
 * suite drives; a silently-succeeding double would make an empty-bodied port
 * indistinguishable from the real one.
 * ========================================================================= */

typedef struct
{
	int updatescreen_calls;
	int tabcomplete_calls;
	int tabcomplete_mode;
	int scroll_calls;
	int scroll_lines;
	int forcemousemove_calls;
	int selectall_calls;
	int copyselection_calls;
	int toggleconsole_calls;
	int menukeydown_calls;
	int menukeydown_key;
	int menucharinput_calls;
	int menucharinput_key;
	int togglemenu_calls;
	int vidtoggle_calls;
	int updateinputmode_calls;
	int activate_calls;
	int deactivateforconsole_calls;
	int clipboard_calls;
} ctest_keys_calls_t;

static ctest_keys_calls_t ctest_keys_calls;

/* Settable answers for the callees keys.c branches on. */
static qboolean ctest_keys_copyselection_result = false;
static qboolean ctest_keys_m_text_entry = false;
static qboolean ctest_keys_m_waiting_for_key_binding = false;
static char		ctest_keys_clipboard[1024] = "";

void SCR_UpdateScreen (qboolean use_tasks)
{
	(void)use_tasks;
	ctest_keys_calls.updatescreen_calls++;
}

void Con_TabComplete (tabcomplete_t mode)
{
	ctest_keys_calls.tabcomplete_calls++;
	ctest_keys_calls.tabcomplete_mode = (int)mode;
}

void Con_Scroll (int lines)
{
	ctest_keys_calls.scroll_calls++;
	ctest_keys_calls.scroll_lines = lines;
}

void Con_ForceMouseMove (void)
{
	ctest_keys_calls.forcemousemove_calls++;
}

void Con_SelectAll (void)
{
	ctest_keys_calls.selectall_calls++;
}

qboolean Con_CopySelectionToClipboard (void)
{
	ctest_keys_calls.copyselection_calls++;
	return ctest_keys_copyselection_result;
}

void Con_ToggleConsole_f (void)
{
	ctest_keys_calls.toggleconsole_calls++;
}

void M_Keydown (int key)
{
	ctest_keys_calls.menukeydown_calls++;
	ctest_keys_calls.menukeydown_key = key;
}

void M_Charinput (int key)
{
	ctest_keys_calls.menucharinput_calls++;
	ctest_keys_calls.menucharinput_key = key;
}

void M_ToggleMenu_f (void)
{
	ctest_keys_calls.togglemenu_calls++;
}

qboolean M_TextEntry (void)
{
	return ctest_keys_m_text_entry;
}

qboolean M_WaitingForKeyBinding (void)
{
	return ctest_keys_m_waiting_for_key_binding;
}

void VID_Toggle (void)
{
	ctest_keys_calls.vidtoggle_calls++;
}

void IN_DeactivateForConsole (void)
{
	ctest_keys_calls.deactivateforconsole_calls++;
}

/* stubs/host_ref.c:264 and stubs/stubs.c:8196 used to define these two as
 * aborting doubles for host.c's TU; keys.c drives both, so the plain names
 * moved here and host_ref.c keeps its abort under a TU-local name. */
void IN_UpdateInputMode (void)
{
	ctest_keys_calls.updateinputmode_calls++;
}

void IN_Activate (void)
{
	ctest_keys_calls.activate_calls++;
}

/* platform.h:34. The real one hands back a Mem_Alloc'd copy of the system
 * clipboard that PasteToConsole (keys.c:152) Mem_Frees; NULL when empty. */
char *PL_GetClipboardData (void)
{
	size_t len;
	char  *out;

	ctest_keys_calls.clipboard_calls++;
	if (!ctest_keys_clipboard[0])
		return NULL;
	len = strlen (ctest_keys_clipboard);
	out = (char *)Mem_Alloc (len + 1);
	memcpy (out, ctest_keys_clipboard, len + 1);
	return out;
}

/* glheight (gl_vidsdl.c) and m_is_quitting (menu.c:114) have no definition
 * anywhere else in this link, so they are defined here. The con_* block that
 * used to sit alongside them moved to stubs/console_ref.c at Phase 7 M10c --
 * console.c is an oracle TU of its own now and owns those objects, so keys.c
 * and the Rust keys port both read the real console state. */
int		 glheight = 0;
qboolean m_is_quitting = false;

/* =========================================================================
 * THE FIXTURE
 *
 * `side` is 1 for the C oracle (c_ref_*) and 0 for the Rust port (plain), the
 * same convention stubs/cl_demo_ref.c uses. Every accessor is per-side
 * because the two halves own two disjoint object sets; the shared objects
 * (key_dest, the con_* block, the call recorder above) have unsuffixed
 * accessors and must be re-seeded before each side runs.
 * ========================================================================= */

extern char		c_ref_key_lines[CMDLINES][MAXCMDLINE];
extern char		c_ref_key_tabhint[MAXCMDLINE];
extern int		c_ref_key_linepos;
extern int		c_ref_key_insert;
extern double	c_ref_key_blinktime;
extern int		c_ref_edit_line;
extern int		c_ref_history_line;
extern char	   *c_ref_keybindings[MAX_KEYS];
extern qboolean c_ref_consolekeys[MAX_KEYS];
extern qboolean c_ref_menubound[MAX_KEYS];
extern qboolean c_ref_keydown[MAX_KEYS];
extern qboolean c_ref_chat_team;

extern const char *c_ref_Key_GetChatBuffer (void);
extern int		   c_ref_Key_GetChatMsgLen (void);
extern qboolean	   c_ref_Key_TextEntry (void);
extern void		   c_ref_Key_GetGrabbedInput (int *lastkey, int *lastchar);
extern void		   c_ref_Key_EndChat (void);

extern client_static_t cls;		  /* quake-capi's cl_main port owns this */
extern client_static_t c_ref_cls; /* Quake/cl_main.c's oracle copy */

typedef struct
{
	char (*lines)[MAXCMDLINE];
	char	 *tabhint;
	int		 *linepos;
	int		 *insert;
	double	 *blinktime;
	int		 *eline;
	int		 *hline;
	char	**bindings;
	qboolean *consolekeys;
	qboolean *menubound;
	qboolean *keydown;
	qboolean *chatteam;
} ctest_keys_side_t;

static ctest_keys_side_t ctest_keys_side (int side)
{
	ctest_keys_side_t s;
	if (side)
	{
		s.lines = c_ref_key_lines;
		s.tabhint = c_ref_key_tabhint;
		s.linepos = &c_ref_key_linepos;
		s.insert = &c_ref_key_insert;
		s.blinktime = &c_ref_key_blinktime;
		s.eline = &c_ref_edit_line;
		s.hline = &c_ref_history_line;
		s.bindings = c_ref_keybindings;
		s.consolekeys = c_ref_consolekeys;
		s.menubound = c_ref_menubound;
		s.keydown = c_ref_keydown;
		s.chatteam = &c_ref_chat_team;
	}
	else
	{
		s.lines = key_lines;
		s.tabhint = key_tabhint;
		s.linepos = &key_linepos;
		s.insert = &key_insert;
		s.blinktime = &key_blinktime;
		s.eline = &edit_line;
		s.hline = &history_line;
		s.bindings = keybindings;
		s.consolekeys = consolekeys;
		s.menubound = menubound;
		s.keydown = keydown;
		s.chatteam = &chat_team;
	}
	return s;
}

typedef struct
{
	char   line[MAXCMDLINE]; /* key_lines[edit_line] */
	char   tabhint[MAXCMDLINE];
	char   chat[MAXCMDLINE];
	int	   lpos;
	int	   ins;
	int	   eline;
	int	   hline;
	int	   dest;
	int	   team;
	int	   clen;
	int	   textentry;
	int	   grabkey;
	int	   grabchar;
	double blink;
} ctest_keys_state_t;

/* Frees and clears everything keys.c owns on one side. The two per-function
 * statics keys.c keeps -- Key_Console's `current` (keys.c:260) and
 * Key_UpdateForDest's `forced` (keys.c:1303) -- have no external handle on
 * either side, so a test that cares about them drives both sides through the
 * same sequence rather than resetting them. */
void ctest_keys_reset (int side)
{
	ctest_keys_side_t s = ctest_keys_side (side);
	int				  i;

	for (i = 0; i < MAX_KEYS; i++)
	{
		if (s.bindings[i])
			Mem_Free (s.bindings[i]);
		s.bindings[i] = NULL;
		s.consolekeys[i] = false;
		s.menubound[i] = false;
		s.keydown[i] = false;
	}
	memset (s.lines, 0, (size_t)CMDLINES * MAXCMDLINE);
	memset (s.tabhint, 0, MAXCMDLINE);
	*s.linepos = 0;
	*s.insert = 1;
	*s.blinktime = 0.0;
	*s.eline = 0;
	*s.hline = 0;
	*s.chatteam = false;
	if (side)
		c_ref_Key_EndChat ();
	else
		Key_EndChat ();

	key_dest = key_game;
	memset (key_tabpartial, 0, MAXCMDLINE);
}

void ctest_keys_set_line (int side, int line, const char *text)
{
	ctest_keys_side_t s = ctest_keys_side (side);
	memset (s.lines[line], 0, MAXCMDLINE);
	strncpy (s.lines[line], text, MAXCMDLINE - 1);
}

void ctest_keys_set_edit (int side, int eline, int hline, int linepos, int insert)
{
	ctest_keys_side_t s = ctest_keys_side (side);
	*s.eline = eline;
	*s.hline = hline;
	*s.linepos = linepos;
	*s.insert = insert;
}

void ctest_keys_snapshot (int side, ctest_keys_state_t *out)
{
	ctest_keys_side_t s = ctest_keys_side (side);
	const char		 *chat;
	int				  li = *s.eline;

	memset (out, 0, sizeof (*out));
	if (li >= 0 && li < CMDLINES)
		memcpy (out->line, s.lines[li], MAXCMDLINE);
	memcpy (out->tabhint, s.tabhint, MAXCMDLINE);
	out->lpos = *s.linepos;
	out->ins = *s.insert;
	out->eline = *s.eline;
	out->hline = *s.hline;
	out->dest = (int)key_dest;
	out->team = *s.chatteam ? 1 : 0;
	out->blink = *s.blinktime;

	chat = side ? c_ref_Key_GetChatBuffer () : Key_GetChatBuffer ();
	out->clen = side ? c_ref_Key_GetChatMsgLen () : Key_GetChatMsgLen ();
	if (chat)
	{
		memcpy (out->chat, chat, MAXCMDLINE - 1);
		out->chat[MAXCMDLINE - 1] = 0;
	}
	out->textentry = (side ? c_ref_Key_TextEntry () : Key_TextEntry ()) ? 1 : 0;
	if (side)
		c_ref_Key_GetGrabbedInput (&out->grabkey, &out->grabchar);
	else
		Key_GetGrabbedInput (&out->grabkey, &out->grabchar);
}

void ctest_keys_get_line (int side, int line, char *out, int cap)
{
	ctest_keys_side_t s = ctest_keys_side (side);
	int				  n = cap < MAXCMDLINE ? cap : MAXCMDLINE;

	memset (out, 0, (size_t)cap);
	memcpy (out, s.lines[line], (size_t)n);
	out[cap - 1] = 0;
}

/* keynames[] order is observable through Key_StringToKeynum (first match on
 * a case-insensitive name) and Key_KeynumToString (first match on a keynum),
 * so the tables are walked position by position rather than compared as sets.
 */
int ctest_keys_keyname_count (int side)
{
	int n = 0;
	if (side)
		while (c_ref_keynames[n].name)
			n++;
	else
		while (keynames[n].name)
			n++;
	return n;
}

const char *ctest_keys_keyname (int side, int i)
{
	return side ? c_ref_keynames[i].name : keynames[i].name;
}

int ctest_keys_keyname_num (int side, int i)
{
	return side ? c_ref_keynames[i].keynum : keynames[i].keynum;
}

const char *ctest_keys_binding (int side, int keynum)
{
	return ctest_keys_side (side).bindings[keynum];
}

/* bit 0: keydown, bit 1: consolekeys, bit 2: menubound */
int ctest_keys_flags (int side, int keynum)
{
	ctest_keys_side_t s = ctest_keys_side (side);
	return (s.keydown[keynum] ? 1 : 0) | (s.consolekeys[keynum] ? 2 : 0) | (s.menubound[keynum] ? 4 : 0);
}

void ctest_keys_set_keydown (int side, int keynum, int value)
{
	ctest_keys_side (side).keydown[keynum] = value ? true : false;
}

/* consolekeys[] and menubound[] are filled only by Key_Init (keys.c:900-950),
 * which also registers four commands; a test that needs one entry set without
 * re-registering anything sets it here. */
void ctest_keys_set_consolekey (int side, int keynum, int value)
{
	ctest_keys_side (side).consolekeys[keynum] = value ? true : false;
}

void ctest_keys_set_menubound (int side, int keynum, int value)
{
	ctest_keys_side (side).menubound[keynum] = value ? true : false;
}

void ctest_keys_set_cls_state (int side, int state)
{
	if (side)
		c_ref_cls.state = (cactive_t)state;
	else
		cls.state = (cactive_t)state;
}

void ctest_keys_set_demo (int side, int playback, int paused, float speed)
{
	client_static_t *c = side ? &c_ref_cls : &cls;

	c->demoplayback = playback ? true : false;
	c->demopaused = paused ? true : false;
	c->demospeed = speed;
}

float ctest_keys_demospeed (int side)
{
	return side ? c_ref_cls.demospeed : cls.demospeed;
}

int ctest_keys_demopaused (int side)
{
	return (side ? c_ref_cls.demopaused : cls.demopaused) ? 1 : 0;
}

/* ---- shared objects: re-seed before each side runs ---- */

void ctest_keys_set_dest (int dest)
{
	key_dest = (keydest_t)dest;
}

int ctest_keys_get_dest (void)
{
	return (int)key_dest;
}

void ctest_keys_set_con (char *text, int current, int linewidth, int vislines, int totallines, int backscroll, int forcedup, int height)
{
	con_text = text;
	con_current = current;
	con_linewidth = linewidth;
	con_vislines = vislines;
	con_totallines = totallines;
	con_backscroll = backscroll;
	con_forcedup = forcedup ? true : false;
	glheight = height;
}

int ctest_keys_con_backscroll (void)
{
	return con_backscroll;
}

void ctest_keys_probe_reset (void)
{
	memset (&ctest_keys_calls, 0, sizeof (ctest_keys_calls));
}

void ctest_keys_probe_get (ctest_keys_calls_t *out)
{
	*out = ctest_keys_calls;
}

void ctest_keys_set_menu (int text_entry, int waiting_for_key_binding, int is_quitting)
{
	ctest_keys_m_text_entry = text_entry ? true : false;
	ctest_keys_m_waiting_for_key_binding = waiting_for_key_binding ? true : false;
	m_is_quitting = is_quitting ? true : false;
}

void ctest_keys_set_copyselection_result (int value)
{
	ctest_keys_copyselection_result = value ? true : false;
}

/* History_OpenFile (keys.c:796) goes through COM_FOpenPrefFile while
 * harness_active (stubs.c:797), which resolves under com_gamedir. common_fs.c
 * IS an oracle source, so the prelude renamed com_gamedir TU-wide and the two
 * sides have separate copies; the same #undef dance stubs/host_cmd_glue_ref.c:788
 * uses gets at the port's. */
#undef com_gamedir
extern char com_gamedir[MAX_OSPATH];
extern char c_ref_com_gamedir[MAX_OSPATH];

void ctest_keys_set_gamedir (int side, const char *dir)
{
	q_strlcpy (side ? c_ref_com_gamedir : com_gamedir, dir, MAX_OSPATH);
}

/* Key_WriteBindings (keys.c:786) branches on cfg_unbindall.value. The prelude
 * renames it TU-wide (c_ref_prelude.h:1743) because cl_main.c defines it, so
 * the oracle reads c_ref_cfg_unbindall while the port reads the plain one
 * stubs/cl_main_ref.c:165 owns. */
#undef cfg_unbindall
extern cvar_t cfg_unbindall;
extern cvar_t c_ref_cfg_unbindall;

void ctest_keys_set_cfg_unbindall (int side, float value)
{
	if (side)
		c_ref_cfg_unbindall.value = value;
	else
		cfg_unbindall.value = value;
}

void ctest_keys_set_clipboard (const char *text)
{
	memset (ctest_keys_clipboard, 0, sizeof (ctest_keys_clipboard));
	if (text)
		strncpy (ctest_keys_clipboard, text, sizeof (ctest_keys_clipboard) - 1);
}
