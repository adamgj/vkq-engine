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
// menu_glue.c -- the C frame around the Rust menu port.
//
// Compiled instead of menu.c under -Duse_rust_host (Rust migration Phase 7
// M10e), mirroring console_glue.c, keys_glue.c, view_glue.c and sbar_glue.c:
//
//  1. Own the eight C-visible objects menu.c defined: vid_menucmdfn and
//     vid_menukeyfn (menu.c:26-27), m_state (:29), m_entersound (:85),
//     m_is_quitting (:89), m_return_state (:91), m_return_onerror (:92) and
//     m_return_reason[32] (:93). Each is declared in a public header and/or
//     has a live reader outside menu.c -- vid.h:72-73 declares the two
//     function pointers, gl_vidsdl.c:5392-5393 writes m_state and
//     m_entersound, net_dgrm.c:1813 reads m_return_state and writes m_state,
//     keys.c:1259 reads m_is_quitting, and net_dgrm.c:61-62 declares
//     m_return_onerror/m_return_reason and :1669-1814 reads and writes them
//     -- so the storage stays here and Rust reaches it through externs
//     (ADR-007). Every other file-scope object in menu.c had external
//     linkage only by accident (no other translation unit names any of
//     them), so all of those moved to Rust.
//  2. Own every plain M_* name. Each one is a wrapper over a quake_rs_menu_*
//     core, for two reasons: most of these prototypes mention cb_context_t *,
//     qpic_t * or a by-value crosshair_t, none of which has a cbindgen
//     spelling, and quake-ctest/stubs/*.c already define several M_* names as
//     link doubles in the oracle link, so no Rust translation unit may export
//     a plain M_ name.
//  3. Re-raise for the four entry points that can reach a longjmp (ADR-009
//     rule 3): M_ToggleMenu_f, M_UpdateMouse, M_Draw and M_Keydown. menu.c
//     names none of Host_Error/Host_EndGame/Sys_Error itself; the raise
//     surface is entirely indirect, through Con_ToggleConsole_f, CL_NextDemo,
//     SCR_ModalMessage, SCR_BeginLoadingPlaque, every cvar write, NET_Poll
//     and the three video-menu entry points in gl_vidsdl.c. Each of those
//     runs under Host_Guard here, so Host_Reraise is called only from this
//     file and no jump crosses a Rust frame.
//
//     M_Menu_Main_f, M_Menu_Quit_f, M_Charinput and M_TextEntry are plain
//     forwards: the pre-existing Host_Guard wrappers on those four
//     (console_glue.c:145, host_cmd_glue.c:218, keys_glue.c:47 and :288)
//     stay where they are and simply never fire, because the Rust cores turn
//     out to be provably non-raising.
//  4. Shim the three things a Rust translation unit must not name:
//     SDL_GetMouseState (check_headers.sh keeps the core headers SDL-free,
//     and the SDL2 int * / SDL3 float * split lives behind one C signature
//     here), the three vulkan_globals device queries (the core headers stay
//     Vulkan-free too), and the ENGINE_NAME_AND_VER build-time string macro.
//  5. Keep M_Init and M_Menu_Credits_f in C. M_Init is nothing but a list of
//     Cmd_AddCommand registrations, every one of which has to name a C
//     function pointer, and it is already entered through the pre-existing
//     Host_Glue_M_Init guard (host_glue.c:462). M_Menu_Credits_f was a static
//     empty function referenced only from that list, so it stays one here.
//
// Accepted, pre-existing exposure: Con_SafePrintf (menu.c:3038) and
// Con_Printf's screen-update tail can reach Mod_LoadModel -> Host_Error
// (gl_model.c:531). That is the standing project exposure every client-stratum
// port inherits; it is not guarded here.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

/* ---------------------------------------------------------------------------
 * C-visible objects (menu.c:26-27, :29, :85, :89, :91-93).
 */

void (*vid_menucmdfn) (void); // johnfitz
void (*vid_menukeyfn) (int key);

enum m_state_e m_state;

qboolean m_entersound; // play after drawing a frame, so caching
					   // won't disrupt the sound

qboolean m_is_quitting = false; // prevents SDL_StartTextInput during quit

enum m_state_e m_return_state;
qboolean	   m_return_onerror;
char		   m_return_reason[32];

/* ---------------------------------------------------------------------------
 * ADR-009 trampolines. Every raise-capable callee the Rust port reaches runs
 * inside Host_Guard here, never from a Rust frame.
 */

typedef struct
{
	const char	 *s;
	const char	 *t;
	float		  f;
	int			  i;
	int			 *outi;
	cvar_t		 *var;
	cb_context_t *cbx;
} menu_arg_t;

/* menu.c:485 -- under -Duse_rust_host the plain Con_ToggleConsole_f is itself
   a Host_Reraise wrapper (console_glue.c:234), so calling it from a Rust frame
   would re-issue the jump across that frame. */
static void Menu_InvokeToggleConsole (void *p)
{
	(void)p;
	Con_ToggleConsole_f ();
}

int Menu_Glue_ToggleConsole (void)
{
	return Host_Guard (Menu_InvokeToggleConsole, NULL);
}

/* menu.c:676 -- CL_NextDemo drives the demo loop and reaches Host_EndGame. */
static void Menu_InvokeNextDemo (void *p)
{
	(void)p;
	CL_NextDemo ();
}

int Menu_Glue_NextDemo (void)
{
	return Host_Guard (Menu_InvokeNextDemo, NULL);
}

/* menu.c:793, :2274 -- SCR_ModalMessage spins SCR_UpdateScreen (false), which
   reaches Mod_LoadModel (mod, true) -> Host_Error (gl_model.c:531). The result
   and the guard status are split (ADR-009 rule 2): a raise must not be
   mistaken for a return value. */
static void Menu_InvokeModalMessage (void *p)
{
	menu_arg_t *a = (menu_arg_t *)p;
	*a->outi = SCR_ModalMessage (a->s, a->f);
}

int Menu_Glue_ModalMessage (const char *text, float timeout, int *out)
{
	menu_arg_t arg = {0};

	arg.s = text;
	arg.f = timeout;
	arg.outi = out;
	return Host_Guard (Menu_InvokeModalMessage, &arg);
}

/* menu.c:952, :4387 -- same SCR_UpdateScreen path as SCR_ModalMessage. */
static void Menu_InvokeBeginLoadingPlaque (void *p)
{
	(void)p;
	SCR_BeginLoadingPlaque ();
}

int Menu_Glue_BeginLoadingPlaque (void)
{
	return Host_Guard (Menu_InvokeBeginLoadingPlaque, NULL);
}

/* menu.c:1203 and the options menus -- every cvar write can reach Host_Error
   through Cvar_SetQuick -> Cvar_CallCallback (cvar.c:507), and under
   -Duse_rust_cvar the plain names are themselves Host_Reraise wrappers. */
static void Menu_InvokeCvarSet (void *p)
{
	menu_arg_t *a = (menu_arg_t *)p;
	Cvar_Set (a->s, a->t);
}

int Menu_Glue_CvarSet (const char *name, const char *value)
{
	menu_arg_t arg = {0};

	arg.s = name;
	arg.t = value;
	return Host_Guard (Menu_InvokeCvarSet, &arg);
}

static void Menu_InvokeCvarSetValue (void *p)
{
	menu_arg_t *a = (menu_arg_t *)p;
	Cvar_SetValue (a->s, a->f);
}

int Menu_Glue_CvarSetValue (const char *name, float value)
{
	menu_arg_t arg = {0};

	arg.s = name;
	arg.f = value;
	return Host_Guard (Menu_InvokeCvarSetValue, &arg);
}

static void Menu_InvokeCvarSetValueQuick (void *p)
{
	menu_arg_t *a = (menu_arg_t *)p;
	Cvar_SetValueQuick (a->var, a->f);
}

int Menu_Glue_CvarSetValueQuick (cvar_t *var, float value)
{
	menu_arg_t arg = {0};

	arg.var = var;
	arg.f = value;
	return Host_Guard (Menu_InvokeCvarSetValueQuick, &arg);
}

/* menu.c:2284, :4736, :4860 -- the video menu lives in gl_vidsdl.c (:5387,
   :5328, :5216), not in menu.c, and all three entry points run VID_SyncCvars,
   which writes cvars and so reaches Host_Error through Cvar_CallCallback. */
static void Menu_InvokeMenuVideo (void *p)
{
	(void)p;
	M_Menu_Video_f ();
}

int Menu_Glue_MenuVideo (void)
{
	return Host_Guard (Menu_InvokeMenuVideo, NULL);
}

static void Menu_InvokeVideoDraw (void *p)
{
	menu_arg_t *a = (menu_arg_t *)p;
	M_Video_Draw (a->cbx);
}

int Menu_Glue_VideoDraw (cb_context_t *cbx)
{
	menu_arg_t arg = {0};

	arg.cbx = cbx;
	return Host_Guard (Menu_InvokeVideoDraw, &arg);
}

static void Menu_InvokeVideoKey (void *p)
{
	menu_arg_t *a = (menu_arg_t *)p;
	M_Video_Key (a->i);
}

int Menu_Glue_VideoKey (int key)
{
	menu_arg_t arg = {0};

	arg.i = key;
	return Host_Guard (Menu_InvokeVideoKey, &arg);
}

/* ---------------------------------------------------------------------------
 * Non-guard shims: things a Rust translation unit must not name.
 */

/* menu.c:4628/:4632 -- the SDL2/SDL3 signature split stays behind this one C
   prototype; the Rust side always sees the SDL3 float form. */
void Menu_Glue_GetMouseState (float *x, float *y)
{
#ifdef USE_SDL3
	SDL_GetMouseState (x, y);
#else
	int xi = 0;
	int yi = 0;
	SDL_GetMouseState (&xi, &yi);
	*x = (float)xi;
	*y = (float)yi;
#endif
}

/* menu.c:1738, :1925, :2062 */
qboolean Menu_Glue_RayQuery (void)
{
	return vulkan_globals.ray_query;
}

/* menu.c:1898, :2030 */
qboolean Menu_Glue_SampleRateShading (void)
{
	return vulkan_globals.device_features.sampleRateShading;
}

/* menu.c:2038 */
float Menu_Glue_MaxSamplerAnisotropy (void)
{
	return vulkan_globals.device_properties.limits.maxSamplerAnisotropy;
}

/* menu.c:3654 -- quakever.h:59-61 */
const char *Menu_Glue_EngineNameAndVer (void)
{
	return ENGINE_NAME_AND_VER;
}

/* ---------------------------------------------------------------------------
 * Re-raising entry points. The Rust bodies are quake_rs_menu_* status cores
 * and the jump is re-issued from here, never from a Rust frame (ADR-009).
 */

/* menu.c:466 -- reaches Con_ToggleConsole_f. */
void M_ToggleMenu_f (void)
{
	int r = quake_rs_menu_toggle_menu_f ();
	Host_Reraise (r);
}

/* menu.c:4623 -- re-enters M_Keydown (K_MOUSE1) and the three
 *_AdjustSliders helpers while a slider or scrollbar is being dragged. */
void M_UpdateMouse (void)
{
	int r = quake_rs_menu_update_mouse ();
	Host_Reraise (r);
}

/* menu.c:4670 -- reaches M_Quit_Draw, M_Search_Draw (NET_Poll) and
   M_Video_Draw. */
void M_Draw (cb_context_t *cbx)
{
	int r = quake_rs_menu_draw (cbx);
	Host_Reraise (r);
}

/* menu.c:4803 -- reaches most of the menu key handlers, which write cvars,
   start maps and run console commands. */
void M_Keydown (int key)
{
	int r = quake_rs_menu_keydown (key);
	Host_Reraise (r);
}

/* ---------------------------------------------------------------------------
 * Plain forwards. None of these can raise; they live here only because the
 * plain M_ names must not be exported from Rust (see note 2 above).
 */

/* menu.c:165 */
crosshair_t M_GetCrosshairDef (float crosshair_def_value)
{
	crosshair_t out = {0};
	quake_rs_menu_get_crosshair_def (crosshair_def_value, &out);
	return out;
}

/* menu.c:175 */
float M_GetScale ()
{
	return quake_rs_menu_get_scale ();
}

/* menu.c:218 */
void M_Print (cb_context_t *cbx, int cx, int cy, const char *str)
{
	quake_rs_menu_print (cbx, cx, cy, str);
}

/* menu.c:272 */
void M_DrawTransPic (cb_context_t *cbx, int x, int y, qpic_t *pic)
{
	quake_rs_menu_draw_trans_pic (cbx, x, y, pic);
}

/* menu.c:282 */
void M_DrawPic (cb_context_t *cbx, int x, int y, qpic_t *pic)
{
	quake_rs_menu_draw_pic (cbx, x, y, pic);
}

/* menu.c:362 */
void M_MenuChanged ()
{
	quake_rs_menu_menu_changed ();
}

/* menu.c:509 */
qboolean M_HandleScrollBarKeys (const int key, int *cursor, int *first_drawn, const int num_total, const int max_on_screen)
{
	return quake_rs_menu_handle_scroll_bar_keys (key, cursor, first_drawn, num_total, max_on_screen);
}

/* menu.c:610 */
void M_Mouse_UpdateCursor (int *cursor, int left, int right, int top, int item_height, int index)
{
	quake_rs_menu_mouse_update_cursor (cursor, left, right, top, item_height, index);
}

/* menu.c:621 -- provably non-raising; console_glue.c:145 keeps guarding it. */
void M_Menu_Main_f (void)
{
	quake_rs_menu_menu_main_f ();
}

/* menu.c:2217 */
void M_Menu_Options_f (void)
{
	quake_rs_menu_menu_options_f ();
}

/* menu.c:3559 -- provably non-raising; host_cmd_glue.c:218 keeps guarding
   it. */
void M_Menu_Quit_f (void)
{
	quake_rs_menu_menu_quit_f ();
}

/* menu.c:4580 */
void M_CheckMods (void)
{
	quake_rs_menu_check_mods ();
}

/* menu.c:4616 */
void M_NewGame (void)
{
	quake_rs_menu_new_game ();
}

/* menu.c:4900 -- provably non-raising; keys_glue.c keeps guarding it. */
void M_Charinput (int key)
{
	quake_rs_menu_charinput (key);
}

/* menu.c:4921 -- provably non-raising; keys_glue.c:47 keeps guarding it. */
qboolean M_TextEntry (void)
{
	return quake_rs_menu_text_entry ();
}

/* menu.c:4938 */
qboolean M_WaitingForKeyBinding (void)
{
	return quake_rs_menu_waiting_for_key_binding ();
}

/* ---------------------------------------------------------------------------
 * Command entry points. These were static in menu.c; M_Init has to take their
 * address, so each stays a static C function over its Rust core.
 */

/* menu.c:733 */
static void M_Menu_SinglePlayer_f (void)
{
	quake_rs_menu_menu_singleplayer_f ();
}

/* menu.c:871 */
static void M_Menu_Load_f (void)
{
	quake_rs_menu_menu_load_f ();
}

/* menu.c:881 */
static void M_Menu_Save_f (void)
{
	quake_rs_menu_menu_save_f ();
}

/* menu.c:3020 */
static void M_Menu_Maps_Cmd_f (void)
{
	quake_rs_menu_menu_maps_cmd_f ();
}

/* menu.c:1020 */
static void M_Menu_MultiPlayer_f (void)
{
	quake_rs_menu_menu_multiplayer_f ();
}

/* menu.c:1109 */
static void M_Menu_Setup_f (void)
{
	quake_rs_menu_menu_setup_f ();
}

/* menu.c:2361 -- declared static at menu.c:41, so the definition there has
   internal linkage too (C11 6.2.2p7). */
static void M_Menu_Keys_f (void)
{
	quake_rs_menu_menu_keys_f ();
}

/* menu.c:2531 */
static void M_Menu_Help_f (void)
{
	quake_rs_menu_menu_help_f ();
}

/* menu.c:4592 -- used by the 2021 re-release */
static void M_Menu_Credits_f (void) {}

/* ---------------------------------------------------------------------------
 * menu.c:4597 -- M_Init stays C: every registration has to name a C function
 * pointer, and host_glue.c:462 already guards the call.
 */

void M_Init (void)
{
	Cmd_AddCommand ("togglemenu", M_ToggleMenu_f);

	Cmd_AddCommand ("menu_main", M_Menu_Main_f);
	Cmd_AddCommand ("menu_singleplayer", M_Menu_SinglePlayer_f);
	Cmd_AddCommand ("menu_load", M_Menu_Load_f);
	Cmd_AddCommand ("menu_save", M_Menu_Save_f);
	Cmd_AddCommand ("menu_maps", M_Menu_Maps_Cmd_f);
	Cmd_AddCommand ("menu_multiplayer", M_Menu_MultiPlayer_f);
	Cmd_AddCommand ("menu_setup", M_Menu_Setup_f);
	Cmd_AddCommand ("menu_options", M_Menu_Options_f);
	Cmd_AddCommand ("menu_keys", M_Menu_Keys_f);
	Cmd_AddCommand ("menu_video", M_Menu_Video_f);
	Cmd_AddCommand ("help", M_Menu_Help_f);
	Cmd_AddCommand ("menu_quit", M_Menu_Quit_f);
	Cmd_AddCommand ("menu_credits", M_Menu_Credits_f); // needed by the 2021 re-release
}

#endif /* USE_RUST_HOST */
