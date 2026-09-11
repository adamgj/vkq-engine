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
// gl_vidsdl_glue.c -- the C that stays with gl_vidsdl.c under -Duse_rust_render
// (Rust migration Phase 8 M6): the SDL window/mode/cvar/menu half, the
// screenshot command, and the VID_Glue_* accessors the Rust Vulkan half
// (quake-capi gl_vidsdl.rs over quake-render::vid) calls back into.
// Every function body is verbatim gl_vidsdl.c.

#include "quakedef.h"
#define NO_SDL_VULKAN_TYPEDEFS
#include "cfgfile.h"
#include "bgmusic.h"
#include "resource.h"
#include "palette.h"
#include "menu.h"
#include "steam.h"

#ifdef USE_SDL3
#include <SDL3/SDL_vulkan.h>
#else
#if defined(SDL_FRAMEWORK) || defined(NO_SDL_CONFIG)
#include <SDL2/SDL_vulkan.h>
#else
#include "SDL_vulkan.h"
#endif
#endif

#ifdef _WIN32
#include <windows.h>
#include <vulkan/vulkan_win32.h>
#endif

#include <float.h>
#include <time.h>

#define MAX_MODE_LIST  600 // johnfitz -- was 30
#define MAX_BPPS_LIST  5
#define MAX_RATES_LIST 20
#define MAXWIDTH	   10000
#define MAXHEIGHT	   10000

#define DEFAULT_REFRESHRATE 60

typedef struct
{
	int	  width;
	int	  height;
	float refreshrate;
} vmode_t;

static vmode_t *modelist = NULL;
static int		nummodes;

static qboolean vid_initialized = false;
static qboolean has_focus = true;

static SDL_Window *draw_context;

static qboolean vid_locked = false; // johnfitz
static qboolean vid_changed = false;

static void VID_Menu_RebuildModeList (void); // johnfitz
static void VID_Restart_f (void);

static void ClearAllStates (void);

viddef_t		vid; // global video state
modestate_t		modestate = MS_UNINIT;
extern qboolean scr_initialized;

//====================================

// johnfitz -- new cvars
static cvar_t vid_fullscreen = {"vid_fullscreen", "0", CVAR_ARCHIVE}; // QuakeSpasm, was "1"
static cvar_t vid_width = {"vid_width", "1280", CVAR_ARCHIVE};		  // QuakeSpasm, was 640
static cvar_t vid_height = {"vid_height", "720", CVAR_ARCHIVE};		  // QuakeSpasm, was 480
static cvar_t vid_refreshrate = {"vid_refreshrate", "60", CVAR_ARCHIVE};
cvar_t		  vid_vsync = {"vid_vsync", "0", CVAR_ARCHIVE};
cvar_t		  vid_maxframelatency = {"vid_maxframelatency", "2", CVAR_ARCHIVE};		// max frames queued for display under vsync, 0 = uncapped
static cvar_t vid_desktopfullscreen = {"vid_desktopfullscreen", "0", CVAR_ARCHIVE}; // QuakeSpasm
static cvar_t vid_borderless = {"vid_borderless", "0", CVAR_ARCHIVE};				// QuakeSpasm
cvar_t		  vid_palettize = {"vid_palettize", "0", CVAR_ARCHIVE};
cvar_t		  vid_filter = {"vid_filter", "0", CVAR_ARCHIVE};
cvar_t		  vid_anisotropic = {"vid_anisotropic", "0", CVAR_ARCHIVE};
cvar_t		  vid_fsaa = {"vid_fsaa", "0", CVAR_ARCHIVE};
cvar_t		  vid_fsaamode = {"vid_fsaamode", "0", CVAR_ARCHIVE};
cvar_t		  vid_gamma = {"gamma", "0.9", CVAR_ARCHIVE};		// johnfitz -- moved here from view.c
cvar_t		  vid_contrast = {"contrast", "1.4", CVAR_ARCHIVE}; // QuakeSpasm, MarkV
cvar_t		  r_usesops = {"r_usesops", "1", CVAR_ARCHIVE};		// johnfitz
#if defined(_DEBUG)
cvar_t r_raydebug = {"r_raydebug", "0", 0};
#endif

// Screenshots
// set by SCR_ScreenShot_f and consumed by GL_EndRendering, both on the main
// thread, so the request always applies to the next submitted frame (the
// asynchronous end-rendering task of the previous frame must not see it)
static qboolean take_screenshot = false;
static char		screenshot_ext[4];
char			screenshot_imagename[MAX_OSPATH]; // johnfitz -- was [80]
int				screenshot_quality;

// the Vulkan half lives in Rust (quake-capi gl_vidsdl.rs); glquake.h
// declares the rest of it
void GL_InitInstance (void);
void GL_InitDevice (void);
void GL_InitCommandBuffers (void);
void GL_CreateRenderResources (void);
void GL_DestroyRenderResources (void);
void R_CreatePaletteOctreeBuffers (uint32_t *colors, int num_colors, palette_octree_node_t *nodes, int num_nodes);

/*
================
VID_Gamma_Init -- call on init
================
*/
static void VID_Gamma_Init (void)
{
	Cvar_RegisterVariable (&vid_gamma);
	Cvar_RegisterVariable (&vid_contrast);
}

/*
======================
VID_GetCurrentWidth
======================
*/
static int VID_GetCurrentWidth (void)
{
	int w = 0, h = 0;
#ifdef USE_SDL3
	SDL_GetWindowSizeInPixels (draw_context, &w, &h);
#else
	SDL_Vulkan_GetDrawableSize (draw_context, &w, &h);
#endif
	return w;
}

/*
=======================
VID_GetCurrentHeight
=======================
*/
static int VID_GetCurrentHeight (void)
{
	int w = 0, h = 0;
#ifdef USE_SDL3
	SDL_GetWindowSizeInPixels (draw_context, &w, &h);
#else
	SDL_Vulkan_GetDrawableSize (draw_context, &w, &h);
#endif
	return h;
}

/*
====================
VID_GetCurrentRefreshRate
====================
*/
static float VID_GetCurrentRefreshRate (void)
{
#ifdef USE_SDL3
	SDL_DisplayID		   current_display;
	const SDL_DisplayMode *mode;

	current_display = SDL_GetDisplayForWindow (draw_context);
	if (current_display == 0)
		current_display = SDL_GetPrimaryDisplay ();

	mode = SDL_GetCurrentDisplayMode (current_display);
	if (!mode)
		return DEFAULT_REFRESHRATE;

	return mode->refresh_rate;
#else
	int				current_display;
	SDL_DisplayMode mode;

	current_display = SDL_GetWindowDisplayIndex (draw_context);
	if (current_display < 0)
		current_display = 0;

	if (SDL_GetCurrentDisplayMode (current_display, &mode) != 0)
		return DEFAULT_REFRESHRATE;

	return mode.refresh_rate;
#endif
}

/*
====================
VID_GetCurrentBPP
====================
*/
static int VID_GetCurrentBPP (void)
{
	const Uint32 pixelFormat = SDL_GetWindowPixelFormat (draw_context);
	return SDL_BITSPERPIXEL (pixelFormat);
}

/*
====================
VID_GetFullscreen

returns true if we are in regular fullscreen or "desktop fullscren"
====================
*/
static qboolean VID_GetFullscreen (void)
{
	return (SDL_GetWindowFlags (draw_context) & SDL_WINDOW_FULLSCREEN) != 0;
}

/*
====================
VID_GetDesktopFullscreen

returns true if we are specifically in "desktop fullscreen" mode
====================
*/
static qboolean VID_GetDesktopFullscreen (void)
{
#ifdef USE_SDL3
	// In SDL3, check if fullscreen mode is NULL (desktop fullscreen) or has a mode (exclusive fullscreen)
	return SDL_GetWindowFullscreenMode (draw_context) == NULL && (SDL_GetWindowFlags (draw_context) & SDL_WINDOW_FULLSCREEN);
#else
	return (SDL_GetWindowFlags (draw_context) & SDL_WINDOW_FULLSCREEN_DESKTOP) != 0;
#endif
}

/*
====================
VID_GetWindow

used by pl_win.c
====================
*/
void *VID_GetWindow (void)
{
	return draw_context;
}

/*
====================
VID_HasMouseOrInputFocus
====================
*/
qboolean VID_HasMouseOrInputFocus (void)
{
	return (SDL_GetWindowFlags (draw_context) & (SDL_WINDOW_MOUSE_FOCUS | SDL_WINDOW_INPUT_FOCUS)) != 0;
}

/*
====================
VID_IsMinimized
====================
*/
qboolean VID_IsMinimized (void)
{
#ifdef USE_SDL3
	return (SDL_GetWindowFlags (draw_context) & SDL_WINDOW_MINIMIZED) != 0;
#else
	return !(SDL_GetWindowFlags (draw_context) & SDL_WINDOW_SHOWN);
#endif
}

/*
================
VID_SDL_GetDisplayMode

Returns a pointer to a SDL_DisplayMode structure with the requested size.
Returns NULL if the size is not available at all.

SDL3: searches the display the window is on (the primary display before the
window exists) and picks the available mode with the closest refresh rate.
SDL2: requires an exact refresh rate match on the primary display.

This is passed to SDL_SetWindowFullscreenMode to specify a pixel format
with the requested bpp. If we didn't care about bpp we could just pass NULL.
================
*/
static const SDL_DisplayMode *VID_SDL_GetDisplayMode (int width, int height, float refreshrate)
{
#ifdef USE_SDL3
	static SDL_DisplayMode result;
	qboolean			   found = false;
	float				   best_dist = FLT_MAX;
	int					   i;

	SDL_DisplayID display = draw_context ? SDL_GetDisplayForWindow (draw_context) : 0;
	if (display == 0)
		display = SDL_GetPrimaryDisplay ();

	int				  count = 0;
	SDL_DisplayMode **modes = (SDL_DisplayMode **)SDL_GetFullscreenDisplayModes (display, &count);
	if (!modes)
		return NULL;

	for (i = 0; i < count; i++)
	{
		const SDL_DisplayMode *mode = modes[i];
		if (mode->w != width || mode->h != height || SDL_BITSPERPIXEL (mode->format) < 24)
			continue;

		const float dist = fabsf (mode->refresh_rate - refreshrate);
		if (dist < best_dist)
		{
			best_dist = dist;
			// copy before SDL_free: the mode structs live inside the same
			// allocation as the returned pointer array
			result = *mode;
			found = true;
		}
	}
	SDL_free (modes);
	return found ? &result : NULL;
#else
	static SDL_DisplayMode mode;
	const int			   sdlmodes = SDL_GetNumDisplayModes (0);
	int					   i;

	for (i = 0; i < sdlmodes; i++)
	{
		if (SDL_GetDisplayMode (0, i, &mode) != 0)
			continue;

		if (mode.w == width && mode.h == height && SDL_BITSPERPIXEL (mode.format) >= 24 && mode.refresh_rate == refreshrate)
		{
			return &mode;
		}
	}
	return NULL;
#endif
}

/*
================
VID_ValidMode
================
*/
static qboolean VID_ValidMode (int width, int height, float refreshrate, qboolean fullscreen)
{
	// ignore width / height / bpp if vid_desktopfullscreen is enabled
	if (fullscreen && vid_desktopfullscreen.value)
		return true;

	if (width < 320)
		return false;

	if (height < 200)
		return false;

	if (fullscreen && VID_SDL_GetDisplayMode (width, height, refreshrate) == NULL)
		return false;

	return true;
}

/*
================
VID_SetMode
================
*/
static qboolean VID_SetMode (int width, int height, float refreshrate, qboolean fullscreen)
{
	int	   temp;
	Uint32 flags;
	char   caption[50];
	int	   previous_display;

	// so Con_Printfs don't mess us up by forcing vid and snd updates
	temp = scr_disabled_for_loading;
	scr_disabled_for_loading = true;

	CDAudio_Pause ();
	BGM_Pause ();

	q_snprintf (caption, sizeof (caption), ENGINE_NAME_AND_VER);

	/* Create the window if needed, hidden */
	if (!draw_context)
	{
		flags = SDL_WINDOW_HIDDEN | SDL_WINDOW_VULKAN;

#ifdef USE_SDL3
		flags |= SDL_WINDOW_HIGH_PIXEL_DENSITY;
#endif

		if (vid_borderless.value)
			flags |= SDL_WINDOW_BORDERLESS;
		else if (!fullscreen)
			flags |= SDL_WINDOW_RESIZABLE;

#ifdef USE_SDL3
		draw_context = SDL_CreateWindow (caption, width, height, flags);
#else
		draw_context = SDL_CreateWindow (caption, SDL_WINDOWPOS_UNDEFINED, SDL_WINDOWPOS_UNDEFINED, width, height, flags);
#endif
		if (!draw_context)
			Sys_Error ("Couldn't create window: %s", SDL_GetError ());

#ifdef USE_SDL3
		previous_display = 0;
#else
		previous_display = -1;
#endif
	}
	else
	{
#ifdef USE_SDL3
		previous_display = SDL_GetDisplayForWindow (draw_context);
#else
		previous_display = SDL_GetWindowDisplayIndex (draw_context);
#endif
	}

	/* Ensure the window is not fullscreen */
	if (VID_GetFullscreen ())
	{
		qboolean ok;
#ifdef USE_SDL3
		ok = SDL_SetWindowFullscreen (draw_context, false);
#else
		ok = SDL_SetWindowFullscreen (draw_context, 0) == 0;
#endif
		if (!ok)
			Sys_Error ("Couldn't set fullscreen state mode: %s", SDL_GetError ());
	}

	/* Set window size and display mode */
	SDL_SetWindowSize (draw_context, width, height);
	if (previous_display >= 0)
		SDL_SetWindowPosition (draw_context, SDL_WINDOWPOS_CENTERED_DISPLAY (previous_display), SDL_WINDOWPOS_CENTERED_DISPLAY (previous_display));
	else
		SDL_SetWindowPosition (draw_context, SDL_WINDOWPOS_CENTERED, SDL_WINDOWPOS_CENTERED);

#ifdef USE_SDL3
	// Set fullscreen mode: NULL for desktop fullscreen, specific mode for exclusive fullscreen
	if (vid_desktopfullscreen.value)
		SDL_SetWindowFullscreenMode (draw_context, NULL);
	else
		SDL_SetWindowFullscreenMode (draw_context, VID_SDL_GetDisplayMode (width, height, refreshrate));
	SDL_SetWindowBordered (draw_context, vid_borderless.value ? false : true);
#else
	SDL_SetWindowDisplayMode (draw_context, VID_SDL_GetDisplayMode (width, height, refreshrate));
	SDL_SetWindowBordered (draw_context, vid_borderless.value ? SDL_FALSE : SDL_TRUE);
#endif

	/* Make window fullscreen if needed, and show the window */

	if (fullscreen)
	{
#ifdef USE_SDL3
		if (!SDL_SetWindowFullscreen (draw_context, true))
			Sys_Error ("Couldn't set fullscreen state mode: %s", SDL_GetError ());
#else
		Uint32 fullscreen_flag = vid_desktopfullscreen.value ? SDL_WINDOW_FULLSCREEN_DESKTOP : SDL_WINDOW_FULLSCREEN;
		if (SDL_SetWindowFullscreen (draw_context, fullscreen_flag) != 0)
			Sys_Error ("Couldn't set fullscreen state mode: %s", SDL_GetError ());
#endif
	}

	SDL_ShowWindow (draw_context);
	SDL_RaiseWindow (draw_context);

#ifdef USE_SDL3
	// window size, position and fullscreen changes are asynchronous requests
	// on some platforms (X11, Wayland); wait until they are actually applied
	// so the sizes queried below are correct
	SDL_SyncWindow (draw_context);
#endif

	vid.width = VID_GetCurrentWidth ();
	vid.height = VID_GetCurrentHeight ();
	vid.conwidth = vid.width & 0xFFFFFFF8;
	vid.conheight = vid.conwidth * vid.height / vid.width;

	modestate = VID_GetFullscreen () ? MS_FULLSCREEN : MS_WINDOWED;

	CDAudio_Resume ();
	BGM_Resume ();
	scr_disabled_for_loading = temp;

	// fix the leftover Alt from any Alt-Tab or the like that switched us away
	ClearAllStates ();

	vid.recalc_refdef = 1;

	// no pending changes
	vid_changed = false;

	SCR_UpdateRelativeScale ();

	return true;
}

/*
===================
VID_Changed_f -- kristian -- notify us that a value has changed that requires a vid_restart
===================
*/
static void VID_Changed_f (cvar_t *var)
{
	vid_changed = true;
}

/*
===================
VID_FilterChanged_f
===================
*/
static void VID_FilterChanged_f (cvar_t *var)
{
	R_InitSamplers ();
}

/*
===================
VID_FSAAChanged_f
===================
*/
static void VID_FSAAChanged_f (cvar_t *var)
{
	VID_Restart (false);
}

/*
===================
VID_VsyncChanged_f -- vsync only needs the swapchain recreated, apply it immediately
===================
*/
static void VID_VsyncChanged_f (cvar_t *var)
{
	VID_Restart (false);
}

/*
================
VID_Test -- johnfitz -- like vid_restart, but asks for confirmation after switching modes
================
*/
static void VID_Test (void)
{
	int	  old_width, old_height, old_fullscreen;
	float old_refreshrate;

	if (vid_locked || !vid_changed)
		return;
	//
	// now try the switch
	//
	old_width = VID_GetCurrentWidth ();
	old_height = VID_GetCurrentHeight ();
	old_refreshrate = VID_GetCurrentRefreshRate ();
	old_fullscreen = VID_GetFullscreen () ? (vulkan_globals.swap_chain_full_screen_exclusive ? 2 : 1) : 0;
	VID_Restart (true);

	// pop up confirmation dialoge
	if (!SCR_ModalMessage ("Would you like to keep this\nvideo mode? (y/n)\n", 5.0f))
	{
		// revert cvars and mode
		Cvar_SetValueQuick (&vid_width, old_width);
		Cvar_SetValueQuick (&vid_height, old_height);
		Cvar_SetValueQuick (&vid_refreshrate, old_refreshrate);
		Cvar_SetValueQuick (&vid_fullscreen, old_fullscreen);
		VID_Restart (true);
	}
}

/*
================
VID_Unlock -- johnfitz
================
*/
static void VID_Unlock (void)
{
	vid_locked = false;
	VID_SyncCvars ();
}

/*
================
VID_Lock -- ericw

Subsequent changes to vid_* mode settings, and vid_restart commands, will
be ignored until the "vid_unlock" command is run.

Used when changing gamedirs so the current settings override what was saved
in the config.cfg.
================
*/
void VID_Lock (void)
{
	vid_locked = true;
}

/*
=================
VID_SetMouseCursor
=================
*/
static SDL_Cursor *cursor_default;
static SDL_Cursor *cursor_hand;
static SDL_Cursor *cursor_ibeam;

static void VID_CreateCursors (void)
{
#ifdef USE_SDL3
	cursor_default = SDL_CreateSystemCursor (SDL_SYSTEM_CURSOR_DEFAULT);
	cursor_hand = SDL_CreateSystemCursor (SDL_SYSTEM_CURSOR_POINTER);
	cursor_ibeam = SDL_CreateSystemCursor (SDL_SYSTEM_CURSOR_TEXT);
#else
	cursor_default = SDL_CreateSystemCursor (SDL_SYSTEM_CURSOR_ARROW);
	cursor_hand = SDL_CreateSystemCursor (SDL_SYSTEM_CURSOR_HAND);
	cursor_ibeam = SDL_CreateSystemCursor (SDL_SYSTEM_CURSOR_IBEAM);
#endif
}

static void VID_DestroyCursors (void)
{
#ifdef USE_SDL3
	SDL_DestroyCursor (cursor_default);
	SDL_DestroyCursor (cursor_hand);
	SDL_DestroyCursor (cursor_ibeam);
#else
	SDL_FreeCursor (cursor_default);
	SDL_FreeCursor (cursor_hand);
	SDL_FreeCursor (cursor_ibeam);
#endif
	cursor_default = NULL;
	cursor_hand = NULL;
	cursor_ibeam = NULL;
}

void VID_SetMouseCursor (mousecursor_t cursor)
{
	static mousecursor_t current_cursor = MOUSECURSOR_DEFAULT;

	if (cursor == current_cursor)
		return;
	current_cursor = cursor;

	switch (cursor)
	{
	case MOUSECURSOR_HAND:
		SDL_SetCursor (cursor_hand);
		break;

	case MOUSECURSOR_IBEAM:
		SDL_SetCursor (cursor_ibeam);
		break;

	case MOUSECURSOR_DEFAULT:
	default:
		SDL_SetCursor (cursor_default);
		break;
	}
}

/*
=================
VID_Shutdown
=================
*/
void VID_Shutdown (void)
{
	if (vid_initialized)
	{
		assert (draw_context != NULL);
		VID_DestroyCursors ();
		SDL_DestroyWindow (draw_context);
		draw_context = NULL;
		SDL_QuitSubSystem (SDL_INIT_VIDEO);
		PL_VID_Shutdown ();
	}
}

/*
===================================================================

MAIN WINDOW

===================================================================
*/

/*
================
ClearAllStates
================
*/
static void ClearAllStates (void)
{
	Key_ClearStates ();
	IN_ClearStates ();
}

//==========================================================================
//
//  COMMANDS
//
//==========================================================================

/*
=================
VID_DescribeCurrentMode_f
=================
*/
static void VID_DescribeCurrentMode_f (void)
{
	if (draw_context)
		Con_Printf (
			"%dx%dx%d %gHz %s\n", VID_GetCurrentWidth (), VID_GetCurrentHeight (), VID_GetCurrentBPP (), VID_GetCurrentRefreshRate (),
			VID_GetFullscreen () ? "fullscreen" : "windowed");
}

/*
=================
VID_DescribeModes_f -- johnfitz -- changed formatting, and added refresh rates after each mode.
=================
*/
static void VID_DescribeModes_f (void)
{
	int i;
	int lastwidth, lastheight, count;

	lastwidth = lastheight = count = 0;

	for (i = 0; i < nummodes; i++)
	{
		if (lastwidth != modelist[i].width || lastheight != modelist[i].height)
		{
			if (count > 0)
				Con_SafePrintf ("\n");
			Con_SafePrintf ("   %4i x %4i : %g", modelist[i].width, modelist[i].height, modelist[i].refreshrate);
			lastwidth = modelist[i].width;
			lastheight = modelist[i].height;
			count++;
		}
	}
	Con_Printf ("\n%i modes\n", count);
}

//==========================================================================
//
//  INIT
//
//==========================================================================

/*
=================
VID_InitModelist
=================
*/
static void VID_InitModelist (void)
{
#ifdef USE_SDL3
	SDL_DisplayID	  display = SDL_GetPrimaryDisplay ();
	int				  count = 0;
	SDL_DisplayMode **modes = (SDL_DisplayMode **)SDL_GetFullscreenDisplayModes (display, &count);
	int				  i;

	if (!modes)
	{
		nummodes = 0;
		return;
	}

	modelist = Mem_Realloc (modelist, sizeof (vmode_t) * count);
	nummodes = 0;

	for (i = 0; i < count; i++)
	{
		const SDL_DisplayMode *mode = modes[i];
		modelist[nummodes].width = mode->w;
		modelist[nummodes].height = mode->h;
		modelist[nummodes].refreshrate = mode->refresh_rate;
		nummodes++;
	}

	SDL_free (modes);
#else
	const int sdlmodes = SDL_GetNumDisplayModes (0);
	int		  i;

	modelist = Mem_Realloc (modelist, sizeof (vmode_t) * sdlmodes);
	nummodes = 0;
	for (i = 0; i < sdlmodes; i++)
	{
		SDL_DisplayMode mode;

		if (SDL_GetDisplayMode (0, i, &mode) == 0)
		{
			modelist[nummodes].width = mode.w;
			modelist[nummodes].height = mode.h;
			modelist[nummodes].refreshrate = mode.refresh_rate;
			nummodes++;
		}
	}
#endif
}

/*
===================
VID_Init
===================
*/
void VID_Init (void)
{
	static char vid_center[] = "SDL_VIDEO_CENTERED=center";
	int			p, width, height;
	float		refreshrate;
	int			display_width, display_height;
	float		display_refreshrate;
	qboolean	fullscreen;
	const char *read_vars[] = {"vid_fullscreen",		"vid_width",	"vid_height", "vid_refreshrate", "vid_vsync",
							   "vid_desktopfullscreen", "vid_fsaamode", "vid_fsaa",	  "vid_borderless"};
#define num_readvars countof (read_vars)

	Cvar_RegisterVariable (&vid_fullscreen);  // johnfitz
	Cvar_RegisterVariable (&vid_width);		  // johnfitz
	Cvar_RegisterVariable (&vid_height);	  // johnfitz
	Cvar_RegisterVariable (&vid_refreshrate); // johnfitz
	Cvar_RegisterVariable (&vid_vsync);		  // johnfitz
	Cvar_RegisterVariable (&vid_maxframelatency);
	Cvar_RegisterVariable (&vid_filter);
	Cvar_RegisterVariable (&vid_anisotropic);
	Cvar_RegisterVariable (&vid_fsaamode);
	Cvar_RegisterVariable (&vid_fsaa);
	Cvar_RegisterVariable (&vid_desktopfullscreen); // QuakeSpasm
	Cvar_RegisterVariable (&vid_borderless);		// QuakeSpasm
	Cvar_RegisterVariable (&vid_palettize);
#if defined(_DEBUG)
	Cvar_RegisterVariable (&r_raydebug);
#endif
	Cvar_SetCallback (&vid_fullscreen, VID_Changed_f);
	Cvar_SetCallback (&vid_width, VID_Changed_f);
	Cvar_SetCallback (&vid_height, VID_Changed_f);
	Cvar_SetCallback (&vid_refreshrate, VID_Changed_f);
	Cvar_SetCallback (&vid_filter, VID_FilterChanged_f);
	Cvar_SetCallback (&vid_anisotropic, VID_FilterChanged_f);
	Cvar_SetCallback (&vid_fsaamode, VID_FSAAChanged_f);
	Cvar_SetCallback (&vid_fsaa, VID_FSAAChanged_f);
	Cvar_SetCallback (&vid_vsync, VID_VsyncChanged_f);
	Cvar_SetCallback (&vid_desktopfullscreen, VID_Changed_f);
	Cvar_SetCallback (&vid_borderless, VID_Changed_f);

	Cmd_AddCommand ("vid_unlock", VID_Unlock);	   // johnfitz
	Cmd_AddCommand ("vid_restart", VID_Restart_f); // johnfitz
	Cmd_AddCommand ("vid_test", VID_Test);		   // johnfitz
	Cmd_AddCommand ("vid_describecurrentmode", VID_DescribeCurrentMode_f);
	Cmd_AddCommand ("vid_describemodes", VID_DescribeModes_f);

#ifdef _DEBUG
	Cmd_AddCommand ("create_palette_octree", CreatePaletteOctree_f);
#endif

	putenv (vid_center); /* SDL_putenv is problematic in versions <= 1.2.9 */

#ifdef USE_SDL3
	if (!SDL_InitSubSystem (SDL_INIT_VIDEO))
		Sys_Error ("Couldn't init SDL video: %s", SDL_GetError ());

	{
		SDL_DisplayID		   display = SDL_GetPrimaryDisplay ();
		const SDL_DisplayMode *mode = SDL_GetDesktopDisplayMode (display);
		if (!mode)
			Sys_Error ("Could not get desktop display mode: %s\n", SDL_GetError ());

		display_width = mode->w;
		display_height = mode->h;
		display_refreshrate = mode->refresh_rate;
	}
#else
	if (SDL_InitSubSystem (SDL_INIT_VIDEO) < 0)
		Sys_Error ("Couldn't init SDL video: %s", SDL_GetError ());

	{
		SDL_DisplayMode mode;
		if (SDL_GetDesktopDisplayMode (0, &mode) != 0)
			Sys_Error ("Could not get desktop display mode: %s\n", SDL_GetError ());

		display_width = mode.w;
		display_height = mode.h;
		display_refreshrate = mode.refresh_rate;
	}
#endif

	Sys_Printf ("SDL Video Driver: %s\n", SDL_GetCurrentVideoDriver ());

	VID_CreateCursors ();

	if (CFG_OpenConfig (CONFIG_NAME) == 0)
	{
		CFG_ReadCvars (read_vars, num_readvars);
		CFG_CloseConfig ();
	}
	CFG_ReadCvarOverrides (read_vars, num_readvars);

	VID_InitModelist ();

	width = (int)vid_width.value;
	height = (int)vid_height.value;
	refreshrate = vid_refreshrate.value;
	fullscreen = (int)vid_fullscreen.value;
	vulkan_globals.want_full_screen_exclusive = vid_fullscreen.value >= 2;

	if (COM_CheckParm ("-current"))
	{
		width = display_width;
		height = display_height;
		refreshrate = display_refreshrate;
		fullscreen = true;
	}
	else
	{
		p = COM_CheckParm ("-width");
		if (p && p < com_argc - 1)
		{
			width = atoi (com_argv[p + 1]);

			if (!COM_CheckParm ("-height"))
				height = width * 3 / 4;
		}

		p = COM_CheckParm ("-height");
		if (p && p < com_argc - 1)
		{
			height = atoi (com_argv[p + 1]);

			if (!COM_CheckParm ("-width"))
				width = height * 4 / 3;
		}

		p = COM_CheckParm ("-refreshrate");
		if (p && p < com_argc - 1)
			refreshrate = (float)atof (com_argv[p + 1]);

		if (COM_CheckParm ("-window") || COM_CheckParm ("-w"))
			fullscreen = false;
		else if (COM_CheckParm ("-fullscreen") || COM_CheckParm ("-f"))
			fullscreen = true;
	}

	if (!VID_ValidMode (width, height, refreshrate, fullscreen))
	{
		width = (int)vid_width.value;
		height = (int)vid_height.value;
		refreshrate = vid_refreshrate.value;
		fullscreen = (int)vid_fullscreen.value;
	}

	if (!VID_ValidMode (width, height, refreshrate, fullscreen))
	{
		width = 640;
		height = 480;
		refreshrate = display_refreshrate;
		fullscreen = false;
	}

	vid_initialized = true;

	vid.colormap = host_colormap;
	vid.fullbright = 256 - LittleLong (*((int *)vid.colormap + 2048));

	VID_SetMode (width, height, refreshrate, fullscreen);

	// set window icon
	PL_SetWindowIcon ();

	Con_Printf ("\nVulkan Initialization\n");
	SDL_Vulkan_LoadLibrary (NULL);
	GL_InitInstance ();
	GL_InitDevice ();
	GL_InitCommandBuffers ();
	vulkan_globals.staging_buffer_size = INITIAL_STAGING_BUFFER_SIZE_KB * 1024;
	R_InitStagingBuffers ();
	R_CreateDescriptorSetLayouts ();
	R_CreateDescriptorPool ();
	R_InitGPUBuffers ();
	R_InitMeshHeap ();
	TexMgr_InitHeap ();
	R_InitSamplers ();
	R_CreatePipelineLayouts ();
	R_CreatePaletteOctreeBuffers (palette_octree_colors, NUM_PALETTE_OCTREE_COLORS, palette_octree_nodes, NUM_PALETTE_OCTREE_NODES);
	// GL_CreateRenderResources ();

	// johnfitz -- removed code creating "glquake" subdirectory

	VID_Gamma_Init ();			 // johnfitz
	VID_Menu_RebuildModeList (); // johnfitz

	// QuakeSpasm: current vid settings should override config file settings.
	// so we have to lock the vid mode from now until after all config files are read.
	vid_locked = true;
}

/*
===================
VID_Restart
===================
*/
void VID_Restart (qboolean set_mode)
{
	if (!vid_initialized)
		return;

	GL_SynchronizeEndRenderingTask ();

	int		 width, height;
	float	 refreshrate;
	qboolean fullscreen;

	width = (int)vid_width.value;
	height = (int)vid_height.value;
	refreshrate = vid_refreshrate.value;
	fullscreen = vid_fullscreen.value ? true : false;
	vulkan_globals.want_full_screen_exclusive = vid_fullscreen.value >= 2;

	//
	// validate new mode
	//
	if (set_mode && !VID_ValidMode (width, height, refreshrate, fullscreen))
	{
		Con_Printf ("%dx%d %gHz %s is not a valid mode\n", width, height, refreshrate, fullscreen ? "fullscreen" : "windowed");
		return;
	}

	scr_initialized = false;

	GL_WaitForDeviceIdle ();
	GL_DestroyRenderResources ();

	//
	// set new mode
	//
	if (set_mode)
		VID_SetMode (width, height, refreshrate, fullscreen);

	GL_CreateRenderResources ();

	// conwidth and conheight need to be recalculated
	vid.conwidth = (scr_conwidth.value > 0) ? (int)scr_conwidth.value : (scr_conscale.value > 0) ? (int)(vid.width / scr_conscale.value) : vid.width;
	vid.conwidth = CLAMP (320, vid.conwidth, vid.width);
	vid.conwidth &= 0xFFFFFFF8;
	vid.conheight = vid.conwidth * vid.height / vid.width;
	//
	// keep cvars in line with actual mode
	//
	if (set_mode)
		VID_SyncCvars ();

	//
	// update mouse grab
	//
	if (key_dest == key_console || key_dest == key_menu)
	{
		if (modestate == MS_WINDOWED)
			IN_Deactivate (true);
		else if (modestate == MS_FULLSCREEN && key_dest != key_menu)
			IN_HideCursor ();
	}

	R_InitSamplers ();

	SCR_UpdateRelativeScale ();

	scr_initialized = true;
}

/*
===================
VID_Restart_f -- johnfitz -- change video modes on the fly
===================
*/
static void VID_Restart_f (void)
{
	if (vid_locked || !vid_changed)
		return;
	VID_Restart (true);
}

/*
===================
VID_Toggle
new proc by S.A., called by alt-return key binding.
===================
*/
void VID_Toggle (void)
{
	qboolean toggleWorked;
	Uint32	 flags = 0;

	S_ClearBuffer ();

	if (!VID_GetFullscreen ())
	{
#ifdef USE_SDL3
		// Set fullscreen mode before enabling fullscreen
		if (vid_desktopfullscreen.value)
			SDL_SetWindowFullscreenMode (draw_context, NULL);
		else
			SDL_SetWindowFullscreenMode (draw_context, VID_SDL_GetDisplayMode (vid.width, vid.height, vid_refreshrate.value));
		flags = SDL_WINDOW_FULLSCREEN;
#else
		flags = vid_desktopfullscreen.value ? SDL_WINDOW_FULLSCREEN_DESKTOP : SDL_WINDOW_FULLSCREEN;
#endif
	}

#ifdef USE_SDL3
	toggleWorked = SDL_SetWindowFullscreen (draw_context, flags != 0);
#else
	toggleWorked = (SDL_SetWindowFullscreen (draw_context, flags) == 0);
#endif
	if (toggleWorked)
	{
		modestate = VID_GetFullscreen () ? MS_FULLSCREEN : MS_WINDOWED;

		VID_SyncCvars ();

		// update mouse grab
		if (key_dest == key_console || key_dest == key_menu)
		{
			if (modestate == MS_WINDOWED)
				IN_Deactivate (true);
			else if (modestate == MS_FULLSCREEN && key_dest != key_menu)
				IN_HideCursor ();
		}
	}
}

/*
================
VID_SyncCvars -- johnfitz -- set vid cvars to match current video mode
================
*/
void VID_SyncCvars (void)
{
	if (draw_context)
	{
		if (!VID_GetDesktopFullscreen ())
		{
			Cvar_SetValueQuick (&vid_width, VID_GetCurrentWidth ());
			Cvar_SetValueQuick (&vid_height, VID_GetCurrentHeight ());
		}
		Cvar_SetValueQuick (&vid_refreshrate, VID_GetCurrentRefreshRate ());
		Cvar_SetQuick (&vid_fullscreen, VID_GetFullscreen () ? (vulkan_globals.want_full_screen_exclusive ? "2" : "1") : "0");
		// don't sync vid_desktopfullscreen, it's a user preference that
		// should persist even if we are in windowed mode.
	}

	vid_changed = false;
}

//==========================================================================
//
//  NEW VIDEO MENU -- johnfitz
//
//==========================================================================

enum
{
	VID_OPT_MODE,
	VID_OPT_REFRESHRATE,
	VID_OPT_FULLSCREEN,
	VID_OPT_VSYNC,
	VID_OPT_PADDING,
	VID_OPT_TEST,
	VID_OPT_APPLY,
	VIDEO_OPTIONS_ITEMS
};

static int video_options_cursor = 0;

typedef struct
{
	int width, height;
} vid_menu_mode;

// TODO: replace these fixed-length arrays with hunk_allocated buffers
static vid_menu_mode vid_menu_modes[MAX_MODE_LIST];
static int			 vid_menu_nummodes = 0;

static float vid_menu_rates[MAX_RATES_LIST];
static int	 vid_menu_numrates = 0;

// common window sizes offered in addition to the display modes when windowed
static const vid_menu_mode vid_menu_windowed_modes[] = {
	{640, 480},	  {800, 600},	{1024, 768},  {1280, 720},	{1280, 800},  {1366, 768},	{1440, 900},  {1600, 900},	{1600, 1200}, {1680, 1050},
	{1920, 1080}, {1920, 1200}, {2560, 1080}, {2560, 1440}, {2560, 1600}, {3440, 1440}, {3840, 1600}, {3840, 2160}, {5120, 1440}, {5120, 2880},
};

/*
================
VID_Menu_AddMode
================
*/
static void VID_Menu_AddMode (int w, int h)
{
	int i;

	if (vid_menu_nummodes >= MAX_MODE_LIST)
		return;

	for (i = 0; i < vid_menu_nummodes; i++)
	{
		if (vid_menu_modes[i].width == w && vid_menu_modes[i].height == h)
			return;
	}

	vid_menu_modes[vid_menu_nummodes].width = w;
	vid_menu_modes[vid_menu_nummodes].height = h;
	vid_menu_nummodes++;
}

/*
================
VID_Menu_CompareModes
================
*/
static int VID_Menu_CompareModes (const void *a, const void *b)
{
	const vid_menu_mode *ma = (const vid_menu_mode *)a;
	const vid_menu_mode *mb = (const vid_menu_mode *)b;

	if (ma->width != mb->width)
		return mb->width - ma->width;
	return mb->height - ma->height;
}

/*
================
VID_Menu_RebuildModeList

regenerates mode list based on current vid_fullscreen. fullscreen offers the
display modes, windowed additionally offers common window sizes that fit on
the desktop since windows are not limited to display modes
================
*/
static void VID_Menu_RebuildModeList (void)
{
	int i;

	vid_menu_nummodes = 0;

	for (i = 0; i < nummodes; i++)
		VID_Menu_AddMode (modelist[i].width, modelist[i].height);

	if (!vid_fullscreen.value)
	{
		int desktop_width = 0, desktop_height = 0;
#ifdef USE_SDL3
		const SDL_DisplayMode *mode = SDL_GetDesktopDisplayMode (SDL_GetPrimaryDisplay ());
		if (mode)
		{
			desktop_width = mode->w;
			desktop_height = mode->h;
		}
#else
		SDL_DisplayMode mode;
		if (SDL_GetDesktopDisplayMode (0, &mode) == 0)
		{
			desktop_width = mode.w;
			desktop_height = mode.h;
		}
#endif
		for (i = 0; i < (int)countof (vid_menu_windowed_modes); i++)
		{
			if (vid_menu_windowed_modes[i].width <= desktop_width && vid_menu_windowed_modes[i].height <= desktop_height)
				VID_Menu_AddMode (vid_menu_windowed_modes[i].width, vid_menu_windowed_modes[i].height);
		}
	}

	qsort (vid_menu_modes, vid_menu_nummodes, sizeof (vid_menu_modes[0]), VID_Menu_CompareModes);
}

/*
================
VID_Menu_RebuildRateList

regenerates rate list based on current vid_width, vid_height
================
*/
static void VID_Menu_RebuildRateList (void)
{
	int	  i, j;
	float r;

	vid_menu_numrates = 0;

	for (i = 0; i < nummodes; i++)
	{
		// rate list is limited to rates available with current width/height
		if (modelist[i].width != vid_width.value || modelist[i].height != vid_height.value)
			continue;

		r = modelist[i].refreshrate;

		for (j = 0; j < vid_menu_numrates; j++)
		{
			if (vid_menu_rates[j] == r)
				break;
		}

		if (j == vid_menu_numrates)
		{
			vid_menu_rates[j] = r;
			vid_menu_numrates++;
		}
	}

	// if there are no valid fullscreen refreshrates for this width/height, just pick one
	if (vid_menu_numrates == 0)
	{
		Cvar_SetValue ("vid_refreshrate", modelist[0].refreshrate);
		return;
	}

	// if vid_refreshrate is not in the new list, change vid_refreshrate
	for (i = 0; i < vid_menu_numrates; i++)
		if (vid_menu_rates[i] == vid_refreshrate.value)
			break;

	if (i == vid_menu_numrates)
		Cvar_SetValue ("vid_refreshrate", vid_menu_rates[0]);
}

/*
================
VID_Menu_ChooseNextMode

chooses next resolution in order, then updates vid_width and
vid_height cvars, then updates refreshrate lists
================
*/
static void VID_Menu_ChooseNextMode (int dir)
{
	int i;

	if (vid_menu_nummodes)
	{
		for (i = 0; i < vid_menu_nummodes; i++)
		{
			if (vid_menu_modes[i].width == vid_width.value && vid_menu_modes[i].height == vid_height.value)
				break;
		}

		if (i == vid_menu_nummodes) // can't find it in list, so it must be a custom windowed res
		{
			i = 0;
		}
		else
		{
			i += dir;
			if (i >= vid_menu_nummodes)
				i = 0;
			else if (i < 0)
				i = vid_menu_nummodes - 1;
		}

		Cvar_SetValueQuick (&vid_width, (float)vid_menu_modes[i].width);
		Cvar_SetValueQuick (&vid_height, (float)vid_menu_modes[i].height);
		VID_Menu_RebuildRateList ();
	}
}

/*
================
VID_Menu_ChooseNextRate

chooses next refresh rate in order, then updates vid_refreshrate cvar
================
*/
static void VID_Menu_ChooseNextRate (int dir)
{
	int i;

	for (i = 0; i < vid_menu_numrates; i++)
	{
		if (vid_menu_rates[i] == vid_refreshrate.value)
			break;
	}

	if (i == vid_menu_numrates) // can't find it in list
	{
		i = 0;
	}
	else
	{
		i += dir;
		if (i >= vid_menu_numrates)
			i = 0;
		else if (i < 0)
			i = vid_menu_numrates - 1;
	}

	Cvar_SetValue ("vid_refreshrate", vid_menu_rates[i]);
}

/*
================
VID_Menu_ChooseNextFullScreenMode
================
*/
static void VID_Menu_ChooseNextFullScreenMode (int dir)
{
	int i, best, bestdist, dist;

	if (vulkan_globals.full_screen_exclusive)
		Cvar_SetValueQuick (&vid_fullscreen, (float)(((int)vid_fullscreen.value + 3 + dir) % 3));
	else
		Cvar_SetValueQuick (&vid_fullscreen, (float)(((int)vid_fullscreen.value + 2 + dir) % 2));

	VID_Menu_RebuildModeList ();

	// if the current width/height is not in the new list, snap to the closest mode
	for (i = 0; i < vid_menu_nummodes; i++)
	{
		if (vid_menu_modes[i].width == vid_width.value && vid_menu_modes[i].height == vid_height.value)
			break;
	}

	if (i == vid_menu_nummodes && vid_menu_nummodes > 0)
	{
		best = 0;
		bestdist = INT_MAX;
		for (i = 0; i < vid_menu_nummodes; i++)
		{
			dist = abs (vid_menu_modes[i].width - (int)vid_width.value) + abs (vid_menu_modes[i].height - (int)vid_height.value);
			if (dist < bestdist)
			{
				bestdist = dist;
				best = i;
			}
		}
		Cvar_SetValueQuick (&vid_width, (float)vid_menu_modes[best].width);
		Cvar_SetValueQuick (&vid_height, (float)vid_menu_modes[best].height);
		VID_Menu_RebuildRateList ();
	}
}

/*
================
VID_Menu_ChooseNextVSyncMode
================
*/
static void VID_Menu_ChooseNextVSyncMode (int dir)
{
	Cvar_SetValueQuick (&vid_vsync, (float)(((int)vid_vsync.value + 3 + dir) % 3));
}

/*
================
M_Video_Key
================
*/
void M_Video_Key (int key)
{
	switch (key)
	{
	case K_MOUSE2:
	case K_ESCAPE:
	case K_BBUTTON:
		VID_SyncCvars (); // sync cvars before leaving menu. FIXME: there are other ways to leave menu
		S_LocalSound ("misc/menu1.wav");
		M_Menu_Options_f ();
		break;

	case K_UPARROW:
		S_LocalSound ("misc/menu1.wav");
		--video_options_cursor;
		if (video_options_cursor == VID_OPT_PADDING)
			--video_options_cursor;
		if (video_options_cursor < 0)
			video_options_cursor = VIDEO_OPTIONS_ITEMS - 1;
		break;

	case K_DOWNARROW:
		S_LocalSound ("misc/menu1.wav");
		++video_options_cursor;
		if (video_options_cursor == VID_OPT_PADDING)
			++video_options_cursor;
		if (video_options_cursor >= VIDEO_OPTIONS_ITEMS)
			video_options_cursor = 0;
		break;

	case K_LEFTARROW:
		S_LocalSound ("misc/menu3.wav");
		switch (video_options_cursor)
		{
		case VID_OPT_MODE:
			VID_Menu_ChooseNextMode (1);
			break;
		case VID_OPT_REFRESHRATE:
			VID_Menu_ChooseNextRate (1);
			break;
		case VID_OPT_FULLSCREEN:
			VID_Menu_ChooseNextFullScreenMode (-1);
			break;
		case VID_OPT_VSYNC:
			VID_Menu_ChooseNextVSyncMode (-1);
			break;
		default:
			break;
		}
		break;

	case K_RIGHTARROW:
		S_LocalSound ("misc/menu3.wav");
		switch (video_options_cursor)
		{
		case VID_OPT_MODE:
			VID_Menu_ChooseNextMode (-1);
			break;
		case VID_OPT_REFRESHRATE:
			VID_Menu_ChooseNextRate (-1);
			break;
		case VID_OPT_FULLSCREEN:
			VID_Menu_ChooseNextFullScreenMode (1);
			break;
		case VID_OPT_VSYNC:
			VID_Menu_ChooseNextVSyncMode (1);
			break;
		default:
			break;
		}
		break;

	case K_MOUSE1:
	case K_ENTER:
	case K_KP_ENTER:
	case K_ABUTTON:
		m_entersound = true;
		switch (video_options_cursor)
		{
		case VID_OPT_MODE:
			VID_Menu_ChooseNextMode (-1);
			break;
		case VID_OPT_REFRESHRATE:
			VID_Menu_ChooseNextRate (-1);
			break;
		case VID_OPT_FULLSCREEN:
			VID_Menu_ChooseNextFullScreenMode (1);
			break;
		case VID_OPT_VSYNC:
			VID_Menu_ChooseNextVSyncMode (1);
			break;
		case VID_OPT_TEST:
			Cbuf_AddText ("vid_test\n");
			break;
		case VID_OPT_APPLY:
			Cbuf_AddText ("vid_restart\n");
			break;
		default:
			break;
		}
		break;

	default:
		break;
	}
}

/*
================
M_Video_Draw
================
*/
void M_Video_Draw (cb_context_t *cbx)
{
	qpic_t *p;
	int		y = 4;

	// plaque
	p = Draw_CachePic ("gfx/qplaque.lmp");
	M_DrawTransPic (cbx, 16, y, p);

	// p = Draw_CachePic ("gfx/vidmodes.lmp");
	p = Draw_CachePic ("gfx/p_option.lmp");
	M_DrawPic (cbx, (320 - p->width) / 2, y, p);

	y += 36;

	// options
	for (int i = 0; i < VIDEO_OPTIONS_ITEMS; i++)
	{
		switch (i)
		{
		case VID_OPT_MODE:
			M_Print (cbx, MENU_LABEL_X, y, "Video mode");
			M_Print (cbx, MENU_VALUE_X, y, va ("%ix%i", (int)vid_width.value, (int)vid_height.value));
			break;
		case VID_OPT_REFRESHRATE:
			M_Print (cbx, MENU_LABEL_X, y, "Refresh rate");
			M_Print (cbx, MENU_VALUE_X, y, va ("%g", vid_refreshrate.value));
			break;
		case VID_OPT_FULLSCREEN:
			M_Print (cbx, MENU_LABEL_X, y, "Fullscreen");
			M_Print (cbx, MENU_VALUE_X, y, ((int)vid_fullscreen.value == 0) ? "off" : (((int)vid_fullscreen.value == 1) ? "on" : "exclusive"));
			break;
		case VID_OPT_VSYNC:
			M_Print (cbx, MENU_LABEL_X, y, "Vertical sync");
			M_Print (cbx, MENU_VALUE_X, y, ((int)vid_vsync.value == 0) ? "off" : (((int)vid_vsync.value == 1) ? "on" : "triple buffer"));
			break;
		case VID_OPT_TEST:
			M_Print (cbx, MENU_LABEL_X, y, "Test changes");
			break;
		case VID_OPT_APPLY:
			M_Print (cbx, MENU_LABEL_X, y, "Apply changes");
			break;
		}

		M_Mouse_UpdateCursor (&video_options_cursor, 12, 400, y, 8, i);
		if (video_options_cursor == VID_OPT_PADDING)
			video_options_cursor = VID_OPT_VSYNC;
		if (video_options_cursor == i)
			Draw_Character (cbx, MENU_CURSOR_X, y, 12 + ((int)(realtime * 4) & 1));

		y += 8;
	}
}

/*
================
M_Menu_Video_f
================
*/
void M_Menu_Video_f (void)
{
	M_MenuChanged ();
	IN_Deactivate (modestate == MS_WINDOWED);
	key_dest = key_menu;
	m_state = m_video;
	m_entersound = true;

	// set all the cvars to match the current mode when entering the menu
	VID_SyncCvars ();

	// set up mode and rate lists based on current cvars
	VID_Menu_RebuildModeList ();
	VID_Menu_RebuildRateList ();
}

/*
==============================================================================

SCREEN SHOTS

==============================================================================
*/

static void SCR_ScreenShot_Usage (void)
{
	Con_Printf ("usage: screenshot <format> <quality>\n");
	Con_Printf ("   format must be \"png\" or \"tga\" or \"jpg\"\n");
	Con_Printf ("   quality must be 1-100\n");
	return;
}
/*
==================
SCR_GetScreenshotMapTitle
==================
*/

static void SCR_GetScreenshotMapTitle (char *buf, size_t maxchars)
{
	char   clean[countof (cl.levelname)] = {0};
	size_t i, j;

	for (i = j = 0; i + 1 < countof (cl.levelname) && cl.levelname[i]; i++)
	{
		char c = cl.levelname[i] & 0x7f;
		switch (c)
		{
		case '/':
		case '|':
		case ':':
		case ' ':
		case '\n':
		case '*':
		case '<':
		case '>':
		case '-':
		case '?':
		case '!':
		case '"':
		case '\t':
		case '\\':
			c = '_';
			break;
		default:
			break;
		}
		// remove leading spaces, replace consecutive spaces with a single one
		if (c != ' ' || (j > 0 && clean[j - 1] != c))
			clean[j++] = c;
	}
	clean[j++] = '\0';

	q_strlcpy (buf, clean, maxchars);
}

/*
==================
SCR_ScreenShot_f -- johnfitz
==================
*/
void SCR_ScreenShot_f (void)
{
	if ((vulkan_globals.swap_chain_format != VK_FORMAT_B8G8R8A8_UNORM) && (vulkan_globals.swap_chain_format != VK_FORMAT_B8G8R8A8_SRGB) &&
		(vulkan_globals.swap_chain_format != VK_FORMAT_R8G8B8A8_UNORM) && (vulkan_globals.swap_chain_format != VK_FORMAT_R8G8B8A8_SRGB))
	{
		Con_Printf ("SCR_ScreenShot_f: Unsupported surface format\n");
		return;
	}

	memcpy (screenshot_ext, "png", sizeof (screenshot_ext));

	if (Cmd_Argc () >= 2)
	{
		const char *requested_ext = Cmd_Argv (1);

		if (!q_strcasecmp ("png", requested_ext) || !q_strcasecmp ("tga", requested_ext) || !q_strcasecmp ("jpg", requested_ext))
			memcpy (screenshot_ext, requested_ext, sizeof (screenshot_ext));
		else
		{
			SCR_ScreenShot_Usage ();
			return;
		}
	}

	// read quality as the 3rd param (only used for JPG)
	screenshot_quality = 90;
	if (Cmd_Argc () >= 3)
		screenshot_quality = atoi (Cmd_Argv (2));
	if (screenshot_quality < 1 || screenshot_quality > 100)
	{
		SCR_ScreenShot_Usage ();
		return;
	}

	if ((vulkan_globals.swap_chain_format != VK_FORMAT_B8G8R8A8_UNORM) && (vulkan_globals.swap_chain_format != VK_FORMAT_B8G8R8A8_SRGB) &&
		(vulkan_globals.swap_chain_format != VK_FORMAT_R8G8B8A8_UNORM) && (vulkan_globals.swap_chain_format != VK_FORMAT_R8G8B8A8_SRGB))
	{
		Con_Printf ("SCR_ScreenShot_f: Unsupported surface format\n");
		return;
	}

	// find a file name to save it to
	int i;

	// retreive the current date
	time_t now;
	time (&now);
	struct tm *lt = localtime (&now);

	// extract map title:
	char map_title[128];
	SCR_GetScreenshotMapTitle (map_title, sizeof (map_title));

	// not all maps have a valid map title:
	const bool have_map_title = (strlen (map_title) > 0);

	for (i = 0; i < 100; i++)
	{
		q_snprintf (
			screenshot_imagename, sizeof (screenshot_imagename), "%s-%s%s-%04d%02d%02d-%02d%02d%02d-%02i.%s", SCREENSHOT_PREFIX,
			(have_map_title ? va ("%s-", map_title) : ""), cl.mapname, lt->tm_year + 1900, lt->tm_mon + 1, lt->tm_mday, lt->tm_hour, lt->tm_min, lt->tm_sec, i,
			screenshot_ext); // "vkQuake%04scbx_index.tga"

		char checkname[MAX_OSPATH];
		q_snprintf (checkname, sizeof (checkname), "%s/%s", com_gamedir, screenshot_imagename);
		if (Sys_FileType (checkname) == FS_ENT_NONE)
			break; // file doesn't exist
	}
	if (i == 100)
	{
		Con_Printf ("SCR_ScreenShot_f: Couldn't find an unused filename\n");
		return;
	}

	take_screenshot = true;
}

void VID_FocusGained (void)
{
	has_focus = true;
	if (vulkan_globals.want_full_screen_exclusive)
	{
		vid.restart_next_frame = true;
	}
}

void VID_FocusLost (void)
{
	has_focus = false;
	if (vulkan_globals.want_full_screen_exclusive)
	{
		vid.restart_next_frame = true;
	}
}

//==============================================================================
//
//	VID_Glue_* -- the SDL/engine state the Rust Vulkan half reads
//
//==============================================================================

void *VID_Glue_GetInstanceProcAddr (void)
{
	return (void *)SDL_Vulkan_GetVkGetInstanceProcAddr ();
}

const char *const *VID_Glue_InstanceExtensions (unsigned int *count)
{
#ifdef USE_SDL3
	const char *const *sdl_extensions = SDL_Vulkan_GetInstanceExtensions (count);
	if (!sdl_extensions)
		Sys_Error ("SDL_Vulkan_GetInstanceExtensions failed: %s", SDL_GetError ());
	return sdl_extensions;
#else
	static const char **sdl_extensions;
	if (!SDL_Vulkan_GetInstanceExtensions (draw_context, count, NULL))
		Sys_Error ("SDL_Vulkan_GetInstanceExtensions failed: %s", SDL_GetError ());
	if (sdl_extensions)
		Mem_Free (sdl_extensions);
	sdl_extensions = Mem_Alloc (sizeof (const char *) * (*count + 1));
	if (!SDL_Vulkan_GetInstanceExtensions (draw_context, count, sdl_extensions))
		Sys_Error ("SDL_Vulkan_GetInstanceExtensions failed: %s", SDL_GetError ());
	return sdl_extensions;
#endif
}

uint64_t VID_Glue_CreateSurface (void *instance)
{
	VkSurfaceKHR vulkan_surface;
#ifdef USE_SDL3
	if (!SDL_Vulkan_CreateSurface (draw_context, (VkInstance)instance, NULL, &vulkan_surface))
		Sys_Error ("Couldn't create Vulkan surface");
#else
	if (!SDL_Vulkan_CreateSurface (draw_context, (VkInstance)instance, &vulkan_surface))
		Sys_Error ("Couldn't create Vulkan surface");
#endif
	return (uint64_t)vulkan_surface;
}

#ifdef _WIN32
void *VID_Glue_WindowMonitor (void)
{
	HWND	 hwnd = (HWND)SDL_GetPointerProperty (SDL_GetWindowProperties (draw_context), SDL_PROP_WINDOW_WIN32_HWND_POINTER, NULL);
	HMONITOR monitor = MonitorFromWindow (hwnd, MONITOR_DEFAULTTOPRIMARY);
	return (void *)monitor;
}
#endif

qboolean VID_Glue_HasFocus (void)
{
	return has_focus;
}

qboolean VID_Glue_GetFullscreen (void)
{
	return VID_GetFullscreen ();
}

qboolean VID_Glue_TakeScreenshot (void)
{
	const qboolean take = take_screenshot;
	take_screenshot = false;
	return take;
}

void VID_Glue_WriteScreenshot (const byte *pixels, int width, int height)
{
	byte *buffer_ptr = (byte *)pixels;
	// with the Steam API active, screenshots go to the Steam library instead (from Ironwail)
	if (Steam_SaveScreenshot (buffer_ptr, width, height))
		Con_Printf ("Wrote screenshot to the Steam library\n");
	else
	{
		qboolean ok;
		if (!q_strncasecmp (screenshot_ext, "png", sizeof (screenshot_ext)))
			ok = Image_WritePNG (screenshot_imagename, buffer_ptr, width, height, 32, true);
		else if (!q_strncasecmp (screenshot_ext, "tga", sizeof (screenshot_ext)))
			ok = Image_WriteTGA (screenshot_imagename, buffer_ptr, width, height, 32, true);
		else if (!q_strncasecmp (screenshot_ext, "jpg", sizeof (screenshot_ext)))
			ok = Image_WriteJPG (screenshot_imagename, buffer_ptr, width, height, 32, screenshot_quality, true);
		else
			ok = false;
		if (ok)
		{
			Con_SafePrintf ("Wrote ");
			Con_LinkPrintf (va ("%s/%s", com_gamedir, screenshot_imagename), "%s", screenshot_imagename);
			Con_SafePrintf ("\n");
		}
		else
			Con_Printf ("SCR_ScreenShot_f: Couldn't create %s\n", screenshot_imagename);
	}
}

double VID_Glue_ClTime (void)
{
	return cl.time;
}

void VID_Glue_ViewOrg (float *out)
{
	VectorCopy (r_refdef.vieworg, out);
}

qboolean VID_Glue_SkipRenderResources (void)
{
	return sv.active && cls.signon < 1; // server has loaded the map but client hasn't called R_NewMap yet - wait until next frame
}
