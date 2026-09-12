/*
Copyright (C) 1996-2001 Id Software, Inc.
Copyright (C) 2002-2009 John Fitzgibbons and others
Copyright (C) 2007-2008 Kristian Duske
Copyright (C) 2010-2014 QuakeSpasm developers
Copyright (C) 2016 Axel Gneiting

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

// gl_screen_glue.c -- the C that stays with gl_screen.c under -Duse_rust_render
// (Rust migration Phase 8 M7): the cvar and global definitions the rest of the
// engine reads by name, SCR_Init and the key-binding commands it registers,
// SCR_DrawGUI (a setjmp target -- ADR-009 keeps longjmp out of Rust frames),
// the loading plaque and SCR_ModalMessage (both re-enter SCR_UpdateScreen),
// and SCR_UpdateScreen with its task graph (Phase 8 M9). The 2D overlay
// drawing, the refdef/fov math and the cvar callbacks live in
// rust/quake-capi/src/gl_screen.rs and keep their C names.

#include "quakedef.h"
#include "cfgfile.h"
#include <setjmp.h>

int glwidth, glheight;

float scr_con_current;
float scr_conlines; // lines of console to display

// johnfitz -- new cvars
cvar_t scr_menuscale = {"scr_menuscale", "1", CVAR_ARCHIVE};
cvar_t scr_sbarscale = {"scr_sbarscale", "1", CVAR_ARCHIVE};
cvar_t scr_sbaralpha = {"scr_sbaralpha", "0.75", CVAR_ARCHIVE};
cvar_t scr_conwidth = {"scr_conwidth", "0", CVAR_ARCHIVE};
cvar_t scr_conscale = {"scr_conscale", "1", CVAR_ARCHIVE};
cvar_t scr_crosshairscale = {"scr_crosshairscale", "1", CVAR_ARCHIVE};
cvar_t scr_showfps = {"scr_showfps", "0", CVAR_ARCHIVE};
cvar_t scr_clock = {"scr_clock", "0", CVAR_NONE};
cvar_t scr_autoclock = {"scr_autoclock", "1", CVAR_ARCHIVE};
cvar_t scr_usekfont = {"scr_usekfont", "0", CVAR_NONE}; // 2021 re-release
cvar_t scr_style = {"scr_style", "0", CVAR_ARCHIVE};

cvar_t scr_viewsize = {"viewsize", "100", CVAR_ARCHIVE};
cvar_t scr_viewsize_allow_shrinking = {"viewsize_allow_shrinking", "0", CVAR_ARCHIVE};
cvar_t scr_fov = {"fov", "90", CVAR_ARCHIVE}; // 10 - 170
cvar_t scr_fov_adapt = {"fov_adapt", "1", CVAR_ARCHIVE};
cvar_t scr_zoomfov = {"zoom_fov", "30", CVAR_ARCHIVE}; // 10 - 170
cvar_t scr_zoomspeed = {"zoom_speed", "8", CVAR_ARCHIVE};
cvar_t scr_conspeed = {"scr_conspeed", "500", CVAR_ARCHIVE};
cvar_t scr_conanim = {"scr_conanim", "0", CVAR_ARCHIVE};
cvar_t scr_centertime = {"scr_centertime", "2", CVAR_NONE};
cvar_t scr_showturtle = {"showturtle", "0", CVAR_NONE};
cvar_t scr_showpause = {"showpause", "1", CVAR_NONE};
cvar_t scr_printspeed = {"scr_printspeed", "8", CVAR_NONE};

cvar_t cl_gun_fovscale = {"cl_gun_fovscale", "1", CVAR_ARCHIVE}; // Qrack

// All scaling is done relative to resolution with scr_relativescale
cvar_t scr_relativescale = {"scr_relativescale", "2", CVAR_ARCHIVE};
cvar_t scr_relmenuscale = {"scr_relmenuscale", "1", CVAR_ARCHIVE};
cvar_t scr_relsbarscale = {"scr_relsbarscale", "1", CVAR_ARCHIVE};
cvar_t scr_relcrosshairscale = {"scr_relcrosshairscale", "1", CVAR_ARCHIVE};
cvar_t scr_relconscale = {"scr_relconscale", "1", CVAR_ARCHIVE};

extern cvar_t crosshair;
extern cvar_t crosshair_def;
extern cvar_t r_tasks;
extern cvar_t r_gpulightmapupdate;
extern cvar_t r_showbboxes;

qboolean scr_initialized; // ready to draw

qpic_t *scr_net;
qpic_t *scr_turtle;

int clearconsole;

vrect_t scr_vrect;

qboolean scr_disabled_for_loading;
qboolean scr_drawloading;
float	 scr_disabled_time;

qboolean	   in_update_screen;
extern jmp_buf screen_error;
qmutex_t	  *draw_qcvm_mutex;

void SCR_ScreenShot_f (void);

/*
===============================================================================

CENTER PRINTING

===============================================================================
*/

// rust/quake-capi/src/gl_screen.rs (Phase 8 M7)
extern float scr_centertime_off;
void SCR_CheckDrawCenterString (cb_context_t *cbx);
void SCR_CalcRefdef (void);
void SCR_Conwidth_f (cvar_t *var);
void SCR_DrawFPS (cb_context_t *cbx);
void SCR_DrawSpeeds (cb_context_t *cbx);
void SCR_DrawClock (cb_context_t *cbx);
void SCR_DrawDevStats (cb_context_t *cbx);
void SCR_DrawTurtle (cb_context_t *cbx);
void SCR_DrawNet (cb_context_t *cbx);
void SCR_DrawPause (cb_context_t *cbx);
void SCR_DrawLoading (cb_context_t *cbx);
void SCR_DrawCrosshair (cb_context_t *cbx);
void SCR_SetUpToDrawConsole (void);
void SCR_DrawConsole (cb_context_t *cbx);
void SCR_DrawNotifyString (cb_context_t *cbx);
void SCR_TileClear (cb_context_t *cbx);

// M_GetCrosshairDef returns crosshair_t by value, which Rust cannot spell
// portably; the Rust SCR_DrawCrosshair reads it through this out-pointer.
void SCR_Glue_GetCrosshairDef (float crosshair_def_value, crosshair_t *out)
{
	*out = M_GetCrosshairDef (crosshair_def_value);
}

//=============================================================================

//=============================================================================

/*
====================
SCR_ToggleZoom_f
====================
*/
static void SCR_ToggleZoom_f (void)
{
	if (cl.zoomdir)
		cl.zoomdir = -cl.zoomdir;
	else
		cl.zoomdir = cl.zoom > 0.5f ? -1.f : 1.f;
}

/*
====================
SCR_ZoomDown_f
====================
*/
static void SCR_ZoomDown_f (void)
{
	cl.zoomdir = 1.f;
}

/*
====================
SCR_ZoomUp_f
====================
*/
static void SCR_ZoomUp_f (void)
{
	cl.zoomdir = -1.f;
}


/*
=================
SCR_SizeUp_f

Keybinding command
=================
*/
static void SCR_SizeUp_f (void)
{
	Cvar_SetValueQuick (&scr_viewsize, scr_viewsize.value + 10);
}

/*
=================
SCR_SizeDown_f

Keybinding command
=================
*/
static void SCR_SizeDown_f (void)
{
	float new_value = scr_viewsize.value - 10;
	if (!scr_viewsize_allow_shrinking.value)
		new_value = q_max (new_value, 100);
	Cvar_SetValueQuick (&scr_viewsize, new_value);
}

static void SCR_Callback_refdef (cvar_t *var)
{
	vid.recalc_refdef = 1;
}

/*
==================
SCR_UpdateRelativeScale_f
==================
*/
static void SCR_UpdateRelativeScale_f (cvar_t *var)
{
	SCR_UpdateRelativeScale ();
}

/*
==================
SCR_Init
==================
*/
void SCR_Init (void)
{
	// johnfitz -- new cvars
	Cvar_RegisterVariable (&scr_menuscale);
	Cvar_RegisterVariable (&scr_sbarscale);
	Cvar_SetCallback (&scr_sbaralpha, SCR_Callback_refdef);
	Cvar_RegisterVariable (&scr_sbaralpha);
	Cvar_SetCallback (&scr_conwidth, &SCR_Conwidth_f);
	Cvar_SetCallback (&scr_conscale, &SCR_Conwidth_f);
	Cvar_RegisterVariable (&scr_conwidth);
	Cvar_RegisterVariable (&scr_conscale);
	Cvar_RegisterVariable (&scr_crosshairscale);
	Cvar_RegisterVariable (&scr_showfps);
	Cvar_RegisterVariable (&scr_clock);
	Cvar_RegisterVariable (&scr_autoclock);
	// johnfitz
	Cvar_RegisterVariable (&scr_usekfont); // 2021 re-release
	Cvar_SetCallback (&scr_fov, SCR_Callback_refdef);
	Cvar_SetCallback (&scr_fov_adapt, SCR_Callback_refdef);
	Cvar_SetCallback (&scr_zoomfov, SCR_Callback_refdef);
	Cvar_SetCallback (&scr_viewsize, SCR_Callback_refdef);
	Cvar_SetCallback (&scr_style, SCR_Callback_refdef);
	Cvar_RegisterVariable (&scr_fov);
	Cvar_RegisterVariable (&scr_fov_adapt);
	Cvar_RegisterVariable (&scr_zoomfov);
	Cvar_RegisterVariable (&scr_zoomspeed);
	Cvar_RegisterVariable (&scr_viewsize);
	Cvar_RegisterVariable (&scr_viewsize_allow_shrinking);
	Cvar_RegisterVariable (&scr_conspeed);
	Cvar_RegisterVariable (&scr_conanim);
	Cvar_RegisterVariable (&scr_showturtle);
	Cvar_RegisterVariable (&scr_showpause);
	Cvar_RegisterVariable (&scr_centertime);
	Cvar_RegisterVariable (&scr_printspeed);
	Cvar_RegisterVariable (&scr_style);
	Cvar_RegisterVariable (&cl_gun_fovscale);

	Cvar_RegisterVariable (&scr_relativescale);
	Cvar_RegisterVariable (&scr_relmenuscale);
	Cvar_RegisterVariable (&scr_relsbarscale);
	Cvar_RegisterVariable (&scr_relcrosshairscale);
	Cvar_RegisterVariable (&scr_relconscale);
	Cvar_SetCallback (&scr_relativescale, &SCR_UpdateRelativeScale_f);
	Cvar_SetCallback (&scr_relmenuscale, &SCR_UpdateRelativeScale_f);
	Cvar_SetCallback (&scr_relsbarscale, &SCR_UpdateRelativeScale_f);
	Cvar_SetCallback (&scr_relcrosshairscale, &SCR_UpdateRelativeScale_f);
	Cvar_SetCallback (&scr_relconscale, &SCR_UpdateRelativeScale_f);
	SCR_UpdateRelativeScale ();

	if (CFG_OpenConfig (CONFIG_NAME) == 0)
	{
		const char *early_read[] = {"scr_relativescale"};
		CFG_ReadCvars (early_read, 1);
		CFG_CloseConfig ();
	}

	Cmd_AddCommand ("screenshot", SCR_ScreenShot_f);
	Cmd_AddCommand ("sizeup", SCR_SizeUp_f);
	Cmd_AddCommand ("sizedown", SCR_SizeDown_f);

	Cmd_AddCommand ("togglezoom", SCR_ToggleZoom_f);
	Cmd_AddCommand ("+zoom", SCR_ZoomDown_f);
	Cmd_AddCommand ("-zoom", SCR_ZoomUp_f);

	SCR_LoadPics (); // johnfitz

	draw_qcvm_mutex = QMutex_Create ();

	scr_initialized = true;
}

//=============================================================================

//=============================================================================

/*
===============
SCR_BeginLoadingPlaque

================
*/
void SCR_BeginLoadingPlaque (void)
{
	S_StopAllSounds (true, false);

	if (cls.state != ca_connected)
		return;
	if (cls.signon != SIGNONS)
		return;

	// redraw with no console and the loading plaque
	Con_ClearNotify ();
	SCR_CenterPrintClear ();
	scr_con_current = 0;

	scr_drawloading = true;
	SCR_UpdateScreen (false);
	scr_drawloading = false;

	scr_disabled_for_loading = true;
	scr_disabled_time = realtime;
}

/*
===============
SCR_EndLoadingPlaque

================
*/
void SCR_EndLoadingPlaque (void)
{
	scr_disabled_for_loading = false;
	Con_ClearNotify ();
}

//=============================================================================

const char *scr_notifystring;
qboolean	scr_drawdialog;

/*
==================
SCR_ModalMessage

Displays a text string in the center of the screen and waits for a Y or N
keypress.
==================
*/
int SCR_ModalMessage (const char *text, float timeout) // johnfitz -- timeout
{
	double time1, time2; // johnfitz -- timeout
	int	   lastkey, lastchar;

	if (cls.state == ca_dedicated)
		return true;

	scr_notifystring = text;

	// draw a fresh screen
	scr_drawdialog = true;
	SCR_UpdateScreen (false);
	scr_drawdialog = false;

	S_ClearBuffer (); // so dma doesn't loop current sound

	time1 = Sys_DoubleTime () + timeout; // johnfitz -- timeout
	time2 = 0.0f;						 // johnfitz -- timeout

	Key_BeginInputGrab ();
	do
	{
		Sys_SendKeyEvents ();
		Key_GetGrabbedInput (&lastkey, &lastchar);
		Sys_Sleep (16);
		if (timeout)
			time2 = Sys_DoubleTime (); // johnfitz -- zero timeout means wait forever.
	} while (lastchar != 'y' && lastchar != 'Y' && lastchar != 'n' && lastchar != 'N' && lastkey != K_ESCAPE && lastkey != K_ABUTTON && lastkey != K_BBUTTON &&
			 lastkey != K_MOUSE2 && time2 <= time1);
	Key_EndInputGrab ();

	//	SCR_UpdateScreen (); //johnfitz -- commented out

	// johnfitz -- timeout
	if (time2 > time1)
		return false;
	// johnfitz

	return (lastchar == 'y' || lastchar == 'Y' || lastkey == K_ABUTTON);
}

/*
==================
SCR_DrawGUI
==================
*/
static void SCR_DrawGUI (void *unused)
{
	cb_context_t *cbx = vulkan_globals.secondary_cb_contexts[SCBX_GUI];

	GL_SetCanvas (cbx, CANVAS_DEFAULT);
	R_BindPipeline (cbx, VK_PIPELINE_BIND_POINT_GRAPHICS, vulkan_globals.basic_blend_pipeline[cbx->render_pass_index]);

	// FIXME: only call this when needed
	R_BeginDebugUtilsLabel (cbx, "2D");
	SCR_TileClear (cbx);

	const qboolean cscqhud = (scr_style.value < 1.0f) && cl.qcvm.extfuncs.CSQC_DrawHud;

	if (cscqhud && setjmp (screen_error))
		PR_ClearProgs (&cl.qcvm);

	QMutex_Lock (draw_qcvm_mutex);

	if (scr_drawdialog) // new game confirm
	{
		if (con_forcedup)
			Draw_ConsoleBackground (cbx);
		else
			Sbar_Draw (cbx);
		Draw_FadeScreen (cbx);
		SCR_DrawNotifyString (cbx);
	}
	else if (scr_drawloading) // loading
	{
		SCR_DrawLoading (cbx);
		Sbar_Draw (cbx);
	}
	else if (cl.intermission == 1 && key_dest == key_game) // end of level
	{
		Sbar_IntermissionOverlay (cbx);
	}
	else if (cl.intermission == 2 && key_dest == key_game) // end of episode
	{
		Sbar_FinaleOverlay (cbx);
		SCR_CheckDrawCenterString (cbx);
	}
	else
	{
		SCR_DrawCrosshair (cbx); // johnfitz
		SCR_DrawNet (cbx);
		SCR_DrawTurtle (cbx);
		SCR_DrawPause (cbx);
		SCR_CheckDrawCenterString (cbx);
		Sbar_Draw (cbx);
		SCR_DrawDevStats (cbx); // johnfitz
		SCR_DrawFPS (cbx);		// johnfitz
		SCR_DrawSpeeds (cbx);
		SCR_DrawClock (cbx); // johnfitz
		SCR_DrawConsole (cbx);
		M_Draw (cbx);
	}

	QMutex_Unlock (draw_qcvm_mutex);
	R_EndDebugUtilsLabel (cbx);
}

/*
==================
SCR_SetupFrame
==================
*/
static void SCR_SetupFrame (void *unused)
{
	SCR_SetUpToDrawConsole ();
	V_SetupFrame ();
}

/*
==================
SCR_DrawDone
==================
*/
static void SCR_DrawDone (void *unused)
{
	if (scr_speeds.value)
		rs_cputime_us = (uint32_t)((Sys_DoubleTime () - rs_frame_starttime) * 1000000.0);
	// end_rendering depends on draw_done, so this can't lose a wait from the current frame
	rs_gpuwaittime_us = rs_gpuwaitaccum_us;
	rs_gpuwaitaccum_us = 0;
	if (harness_renderhash)
		Harness_RenderDrawDone ();
	r_framecount++;
}

/*
==================
SCR_UpdateScreen

This is called every frame, and can also be called explicitly to flush
text to the screen.

WARNING: be very careful calling this from elsewhere, because the refresh
needs almost the entire 256k of stack space!
==================
*/
void SCR_UpdateScreen (qboolean use_tasks)
{
	if (!scr_initialized || !con_initialized || in_update_screen)
		return; // not initialized yet

	if (Tasks_IsWorker ())
		return; // not safe

	in_update_screen = true;
	use_tasks = use_tasks && (Tasks_NumWorkers () > 1) && r_tasks.value && r_gpulightmapupdate.value;

	if (scr_disabled_for_loading)
	{
		if (realtime - scr_disabled_time > 60)
		{
			scr_disabled_for_loading = false;
			Con_Printf ("load failed.\n");
		}
		else
		{
			in_update_screen = false;
			return;
		}
	}

	if (vid.recalc_refdef)
		SCR_CalcRefdef ();

	// decide on the height of the console
	con_forcedup = !cl.worldmodel || cls.signon != SIGNONS;

	task_handle_t begin_rendering_task = INVALID_TASK_HANDLE;
	if (!GL_BeginRendering (use_tasks, &begin_rendering_task, &glwidth, &glheight))
	{
		in_update_screen = false;
		return;
	}

	if (use_tasks)
	{
		if (prev_end_rendering_task != INVALID_TASK_HANDLE)
		{
			Task_AddDependency (prev_end_rendering_task, begin_rendering_task);
			prev_end_rendering_task = INVALID_TASK_HANDLE;
		}

		task_handle_t draw_done_task = Task_AllocateAndAssignFunc (SCR_DrawDone, NULL, 0);
		task_handle_t setup_frame_task = Task_AllocateAndAssignFunc (SCR_SetupFrame, NULL, 0);
		V_RenderView (use_tasks, begin_rendering_task, setup_frame_task, draw_done_task);
		task_handle_t draw_gui_task = Task_AllocateAndAssignFunc (SCR_DrawGUI, NULL, 0);
		task_handle_t end_rendering_task = GL_EndRendering (use_tasks, true);

		Task_AddDependency (begin_rendering_task, draw_gui_task);
		Task_AddDependency (setup_frame_task, draw_gui_task);
		Task_AddDependency (draw_gui_task, draw_done_task);
		Task_AddDependency (draw_done_task, end_rendering_task);

		task_handle_t tasks[] = {begin_rendering_task, setup_frame_task, draw_done_task, draw_gui_task, end_rendering_task};
		Tasks_Submit (sizeof (tasks) / sizeof (task_handle_t), tasks);

		while (!Task_Join (draw_done_task, 10))
			S_ExtraUpdate ();
		prev_end_rendering_task = end_rendering_task;
	}
	else
	{
		GL_SynchronizeEndRenderingTask ();
		SCR_SetupFrame (NULL);
		V_RenderView (use_tasks, INVALID_TASK_HANDLE, INVALID_TASK_HANDLE, INVALID_TASK_HANDLE);
		S_ExtraUpdate ();
		SCR_DrawGUI (NULL);
		SCR_DrawDone (NULL);
		GL_EndRendering (false, true);
	}

	in_update_screen = false;
}
