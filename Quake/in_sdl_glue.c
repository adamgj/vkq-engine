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
// in_sdl_glue.c -- the C frame around the Rust input port.
//
// Compiled instead of in_sdl.c/in_sdl2.c/in_sdl3.c under -Duse_rust_platform
// (Rust migration Phase 9 M3, ADR-017). Three jobs, mirroring chase_glue.c:
//
//  1. Own the thirteen cvars in_sdl.c defined (in_sdl.c:29-49). Storage stays
//     in C so Cvar_RegisterVariable keeps stable cvar_t addresses (ADR-007);
//     Rust reaches them through externs. in_debugkeys keeps external linkage
//     as before; the joy_* cvars were static and stay file-private here.
//  2. Guard the callees the SDL event pump reaches that are Host_Reraise
//     wrappers under -Duse_rust_host/-Duse_rust_cvar (ADR-009 rule 3):
//     Key_Event, Key_EventWithKeycode, Char_Event, CL_Disconnect, Sys_Quit,
//     the scr_conscale cvar callback and Cvar_RegisterVariable.
//  3. Re-raise from IN_Init, IN_SendKeyEvents and IN_Commands what the guards
//     caught. The remaining input.h entry points cannot raise and are
//     exported by quake-capi under their C names directly.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

/* ---------------------------------------------------------------------------
 * C-visible objects (in_sdl.c:29-49).
 */

cvar_t in_debugkeys = {"in_debugkeys", "0", CVAR_NONE};

// SDL Game Controller cvars
cvar_t joy_deadzone_look = {"joy_deadzone_look", "0.175", CVAR_ARCHIVE};
cvar_t joy_deadzone_move = {"joy_deadzone_move", "0.175", CVAR_ARCHIVE};
cvar_t joy_outer_threshold_look = {"joy_outer_threshold_look", "0.02", CVAR_ARCHIVE};
cvar_t joy_outer_threshold_move = {"joy_outer_threshold_move", "0.02", CVAR_ARCHIVE};
cvar_t joy_deadzone_trigger = {"joy_deadzone_trigger", "0.2", CVAR_ARCHIVE};
cvar_t joy_sensitivity_yaw = {"joy_sensitivity_yaw", "240", CVAR_ARCHIVE};
cvar_t joy_sensitivity_pitch = {"joy_sensitivity_pitch", "130", CVAR_ARCHIVE};
cvar_t joy_invert = {"joy_invert", "0", CVAR_ARCHIVE};
cvar_t joy_exponent = {"joy_exponent", "2", CVAR_ARCHIVE};
cvar_t joy_exponent_move = {"joy_exponent_move", "2", CVAR_ARCHIVE};
cvar_t joy_swapmovelook = {"joy_swapmovelook", "0", CVAR_ARCHIVE};
cvar_t joy_enable = {"joy_enable", "1", CVAR_ARCHIVE};

/* ---------------------------------------------------------------------------
 * Guarded callbacks (ADR-009 rule 3).
 */

typedef struct
{
	int		 key;
	qboolean down;
	int		 keycode;
} insdl_keyevent_t;

static void InSdl_InvokeKeyEvent (void *p)
{
	const insdl_keyevent_t *e = (const insdl_keyevent_t *)p;
	Key_Event (e->key, e->down);
}

int InSdl_Glue_KeyEvent (int key, qboolean down)
{
	insdl_keyevent_t e = {key, down, 0};
	return Host_Guard (InSdl_InvokeKeyEvent, &e);
}

static void InSdl_InvokeKeyEventWithKeycode (void *p)
{
	const insdl_keyevent_t *e = (const insdl_keyevent_t *)p;
	Key_EventWithKeycode (e->key, e->down, e->keycode);
}

int InSdl_Glue_KeyEventWithKeycode (int key, qboolean down, int keycode)
{
	insdl_keyevent_t e = {key, down, keycode};
	return Host_Guard (InSdl_InvokeKeyEventWithKeycode, &e);
}

static void InSdl_InvokeCharEvent (void *p)
{
	Char_Event (*(const int *)p);
}

int InSdl_Glue_CharEvent (int key)
{
	return Host_Guard (InSdl_InvokeCharEvent, &key);
}

static void InSdl_InvokeCLDisconnect (void *p)
{
	(void)p;
	CL_Disconnect ();
}

int InSdl_Glue_CLDisconnect (void)
{
	return Host_Guard (InSdl_InvokeCLDisconnect, NULL);
}

static void InSdl_InvokeSysQuit (void *p)
{
	(void)p;
	Sys_Quit ();
}

int InSdl_Glue_SysQuit (void)
{
	return Host_Guard (InSdl_InvokeSysQuit, NULL);
}

/* in_sdl3.c:135 / in_sdl2.c -- the window size change arm. */
static void InSdl_InvokeConscaleCallback (void *p)
{
	(void)p;
	Cvar_FindVar ("scr_conscale")->callback (NULL);
}

int InSdl_Glue_ConscaleCallback (void)
{
	return Host_Guard (InSdl_InvokeConscaleCallback, NULL);
}

/* in_sdl.c:211-223 -- one Cvar_RegisterVariable. */
static void InSdl_InvokeRegisterVariable (void *p)
{
	Cvar_RegisterVariable ((cvar_t *)p);
}

int InSdl_Glue_RegisterVariable (cvar_t *var)
{
	return Host_Guard (InSdl_InvokeRegisterVariable, var);
}

/* ---------------------------------------------------------------------------
 * Re-raising public entry points (ADR-009).
 */

/* in_sdl.c:195 */
void IN_Init (void)
{
	int r = quake_rs_in_init ();
	Host_Reraise (r);
}

/* in_sdl3.c:111 / in_sdl2.c IN_SendKeyEvents */
void IN_SendKeyEvents (void)
{
	int r = quake_rs_in_send_key_events ();
	Host_Reraise (r);
}

/* in_sdl.c:444 */
void IN_Commands (void)
{
	int r = quake_rs_in_commands ();
	Host_Reraise (r);
}
