/*
Copyright (C) 2022 A. Drexler

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

// steam_api.c -- Steam API runtime (achievements, rich presence),
// split out of steam.c. Loads the flat (C) steam_api library shipped
// with the rerelease dynamically at runtime. Stays C until Phase 9
// (platform layer) of the Rust migration; the discovery half of
// steam.c is ported in Phase 2.

#include "quakedef.h"
#include "steam.h"

/*
==============================================================================
STEAM API (from Ironwail)

Achievements and rich presence, using the flat (C) API of the steam_api
library shipped with the rerelease, loaded dynamically at runtime
==============================================================================
*/

#ifdef _WIN32
#define STEAMAPI __cdecl
#else
#define STEAMAPI
#endif

// clang-format off
#define STEAMAPI_FUNCTIONS(x)\
	x(int,		SteamAPI_IsSteamRunning, (void))\
	x(int,		SteamAPI_Init, (void))\
	x(void,		SteamAPI_Shutdown, (void))\
	x(int,		SteamAPI_GetHSteamUser, (void))\
	x(int,		SteamAPI_GetHSteamPipe, (void))\
	x(void*,	SteamInternal_CreateInterface, (const char *which))\
	x(void*,	SteamAPI_ISteamClient_GetISteamFriends, (void *client, int huser, int hpipe, const char *version))\
	x(void*,	SteamAPI_ISteamClient_GetISteamUserStats, (void *client, int huser, int hpipe, const char *version))\
	x(void*,	SteamAPI_ISteamClient_GetISteamScreenshots, (void *client, int huser, int hpipe, const char *version))\
	x(void,		SteamAPI_ISteamFriends_ClearRichPresence, (void *client))\
	x(int,		SteamAPI_ISteamFriends_SetRichPresence, (void *friends, const char *key, const char *val))\
	x(int,		SteamAPI_ISteamUserStats_SetAchievement, (void *userstats, const char *name))\
	x(int,		SteamAPI_ISteamUserStats_StoreStats, (void *userstats))\
	x(int,		SteamAPI_ISteamScreenshots_HookScreenshots, (void *screenshots, int hook))\
	x(uint32_t,	SteamAPI_ISteamScreenshots_WriteScreenshot, (void *screenshots, const void *data, uint32_t size, int width, int height))
// clang-format on

#define STEAMAPI_CLIENT_VERSION		 "SteamClient015"
#define STEAMAPI_FRIENDS_VERSION	 "SteamFriends015"
#define STEAMAPI_USERSTATS_VERSION	 "STEAMUSERSTATS_INTERFACE_VERSION012"
#define STEAMAPI_SCREENSHOTS_VERSION "STEAMSCREENSHOTS_INTERFACE_VERSION003"

#define STEAMAPI_DECLARE_FUNCTION(ret, name, args) static ret (STEAMAPI *name##_Func) args;
STEAMAPI_FUNCTIONS (STEAMAPI_DECLARE_FUNCTION)
#undef STEAMAPI_DECLARE_FUNCTION

typedef struct
{
	void	  **address;
	const char *name;
} apifunc_t;

static const apifunc_t steamapi_funcs[] = {
#define STEAMAPI_FUNC_DEF(ret, name, args) {(void **)&name##_Func, #name},
	STEAMAPI_FUNCTIONS (STEAMAPI_FUNC_DEF)
#undef STEAMAPI_FUNC_DEF
};

static struct
{
	void	*library;
	int		 hsteamuser;
	int		 hsteampipe;
	void	*client;
	void	*friends;
	void	*userstats;
	void	*screenshots;
	qboolean needs_shutdown;
} steamapi;

/*
========================
Steam_ClearFunctions
========================
*/
static void Steam_ClearFunctions (void)
{
	size_t i;
	for (i = 0; i < countof (steamapi_funcs); i++)
		*steamapi_funcs[i].address = NULL;
}

/*
========================
Steam_InitFunctions
========================
*/
static qboolean Steam_InitFunctions (void)
{
	size_t i, missing;

	for (i = missing = 0; i < countof (steamapi_funcs); i++)
	{
		const apifunc_t *func = &steamapi_funcs[i];
		*func->address = (void *)SDL_LoadFunction (steamapi.library, func->name);
		if (!*func->address)
		{
			Sys_Printf ("ERROR: missing Steam API function \"%s\"\n", func->name);
			missing++;
		}
	}

	if (missing)
	{
		Steam_ClearFunctions ();
		return false;
	}

	return true;
}

/*
========================
Steam_LoadLibrary
========================
*/
static qboolean Steam_LoadLibrary (const steamgame_t *game)
{
	char dllpath[MAX_OSPATH];
	if (!Sys_GetSteamAPILibraryPath (dllpath, sizeof (dllpath), game))
		return false;
	steamapi.library = SDL_LoadObject (dllpath);
	return steamapi.library != NULL;
}

/*
========================
Steam_Init
========================
*/
qboolean Steam_Init (const steamgame_t *game)
{
	char appid[32];

	if (COM_CheckParm ("-nosteamapi"))
	{
		Sys_Printf ("Steam API support disabled on the command line\n");
		return false;
	}

	if (!game->appid)
		return false;

	q_snprintf (appid, sizeof (appid), "%d", game->appid);
#ifdef _WIN32
	if (_putenv_s ("SteamAppId", appid) != 0)
#else
	if (setenv ("SteamAppId", appid, 1) != 0)
#endif
	{
		Sys_Printf ("ERROR: Couldn't set SteamAppId environment variable\n");
		return false;
	}

	if (!Steam_LoadLibrary (game))
	{
		Sys_Printf ("Couldn't load Steam API library\n");
		return false;
	}
	Sys_Printf ("Loaded Steam API library\n");

	if (!Steam_InitFunctions ())
	{
		Steam_Shutdown ();
		return false;
	}

	if (!SteamAPI_IsSteamRunning_Func ())
	{
		Sys_Printf ("Steam not running\n");

		SDL_ShowSimpleMessageBox (
			SDL_MESSAGEBOX_INFORMATION, "Steam not running",
			"Steam must be running in order to update achievements,\n"
			"track total time played, and show in-game status to your friends.\n"
			"\n"
			"If this functionality is important to you, please start Steam\n"
			"before continuing.\n",
			NULL);

		if (SteamAPI_IsSteamRunning_Func ())
			Sys_Printf ("Steam is now running, continuing\n");
		else
			Sys_Printf ("Steam is still not running\n");
	}

	if (!SteamAPI_Init_Func ())
	{
		Sys_Printf ("Couldn't initialize Steam API\n");
		Steam_Shutdown ();
		return false;
	}
	steamapi.needs_shutdown = true;

	steamapi.hsteamuser = SteamAPI_GetHSteamUser_Func ();
	steamapi.hsteampipe = SteamAPI_GetHSteamPipe_Func ();
	steamapi.client = SteamInternal_CreateInterface_Func (STEAMAPI_CLIENT_VERSION);
	if (!steamapi.client)
	{
		Sys_Printf ("ERROR: Couldn't create Steam client interface\n");
		Steam_Shutdown ();
		return false;
	}

	steamapi.friends = SteamAPI_ISteamClient_GetISteamFriends_Func (steamapi.client, steamapi.hsteamuser, steamapi.hsteampipe, STEAMAPI_FRIENDS_VERSION);
	if (!steamapi.friends)
	{
		Sys_Printf ("Couldn't initialize SteamFriends interface\n");
		Steam_Shutdown ();
		return false;
	}

	steamapi.userstats = SteamAPI_ISteamClient_GetISteamUserStats_Func (steamapi.client, steamapi.hsteamuser, steamapi.hsteampipe, STEAMAPI_USERSTATS_VERSION);
	if (!steamapi.userstats)
	{
		Sys_Printf ("Couldn't initialize SteamUserStats interface\n");
		Steam_Shutdown ();
		return false;
	}

	steamapi.screenshots =
		SteamAPI_ISteamClient_GetISteamScreenshots_Func (steamapi.client, steamapi.hsteamuser, steamapi.hsteampipe, STEAMAPI_SCREENSHOTS_VERSION);
	if (!steamapi.screenshots)
		Sys_Printf ("Couldn't initialize SteamScreenshots interface\n");
	else
		SteamAPI_ISteamScreenshots_HookScreenshots_Func (steamapi.screenshots, true);

	Sys_Printf ("Steam API initialized\n");

	Steam_ClearStatus ();

	return true;
}

/*
========================
Steam_Shutdown
========================
*/
void Steam_Shutdown (void)
{
	if (!steamapi.library)
		return;
	if (steamapi.needs_shutdown)
		SteamAPI_Shutdown_Func ();
	Steam_ClearFunctions ();
	SDL_UnloadObject (steamapi.library);
	memset (&steamapi, 0, sizeof (steamapi));
	Sys_Printf ("Unloaded Steam API\n");
}

/*
========================
Steam_SetAchievement
========================
*/
qboolean Steam_SetAchievement (const char *name)
{
	if (!steamapi.userstats || !SteamAPI_ISteamUserStats_SetAchievement_Func (steamapi.userstats, name))
		return false;
	SteamAPI_ISteamUserStats_StoreStats_Func (steamapi.userstats);
	return true;
}

/*
========================
Steam_ClearStatus
========================
*/
void Steam_ClearStatus (void)
{
	if (steamapi.friends)
		SteamAPI_ISteamFriends_ClearRichPresence_Func (steamapi.friends);
}

/*
========================
Steam_SetStatus_Menu
========================
*/
void Steam_SetStatus_Menu (void)
{
	if (steamapi.friends)
		SteamAPI_ISteamFriends_SetRichPresence_Func (steamapi.friends, "steam_display", "#mainmenu");
}

/*
========================
Steam_SetStatus_SinglePlayer
========================
*/
void Steam_SetStatus_SinglePlayer (const char *map)
{
	if (steamapi.friends)
	{
		SteamAPI_ISteamFriends_SetRichPresence_Func (steamapi.friends, "map", map);
		SteamAPI_ISteamFriends_SetRichPresence_Func (steamapi.friends, "steam_display", "#singleplayer");
	}
}

/*
========================
Steam_SaveScreenshot

Adds a screenshot to the Steam library; expects top-down RGBA data
========================
*/
qboolean Steam_SaveScreenshot (const void *rgba, int width, int height)
{
	const byte *src = (const byte *)rgba;
	byte	   *rgb;
	int			i, numpixels;
	qboolean	result;

	if (!steamapi.screenshots)
		return false;

	// pack to the RGB format expected by WriteScreenshot
	numpixels = width * height;
	rgb = (byte *)Mem_Alloc (numpixels * 3);
	for (i = 0; i < numpixels; i++)
	{
		rgb[i * 3 + 0] = src[i * 4 + 0];
		rgb[i * 3 + 1] = src[i * 4 + 1];
		rgb[i * 3 + 2] = src[i * 4 + 2];
	}

	result = SteamAPI_ISteamScreenshots_WriteScreenshot_Func (steamapi.screenshots, rgb, (uint32_t)numpixels * 3, width, height) != 0;
	Mem_Free (rgb);

	return result;
}

/*
========================
Steam_SetStatus_Multiplayer
========================
*/
void Steam_SetStatus_Multiplayer (int players, int maxplayers, const char *map)
{
	if (steamapi.friends)
	{
		char value[32];
		q_snprintf (value, sizeof (value), "%d", players);
		SteamAPI_ISteamFriends_SetRichPresence_Func (steamapi.friends, "steam_player_group_size", value);
		q_snprintf (value, sizeof (value), "%d", maxplayers);
		SteamAPI_ISteamFriends_SetRichPresence_Func (steamapi.friends, "max_group_size", value);
		SteamAPI_ISteamFriends_SetRichPresence_Func (steamapi.friends, "map", map);
		SteamAPI_ISteamFriends_SetRichPresence_Func (steamapi.friends, "steam_display", "#multiplayer");
	}
}
