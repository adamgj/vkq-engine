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

// gl_sky_glue.c -- the C that stays with gl_sky.c under -Duse_rust_render
// (Rust migration Phase 8 M7): the sky cvars, the console commands, the
// skywind config file I/O, the %g formatting of Sky_GetSkyCommand (ADR-005
// gap) and Sky_Init. Everything else, including the skybox record and skyfog,
// lives in rust/quake-capi/src/gl_sky.rs and is viewed here through externs.

#include "quakedef.h"

extern cvar_t gl_farclip;
cvar_t		  r_fastsky = {"r_fastsky", "0", CVAR_NONE};
cvar_t		  r_sky_quality = {"r_sky_quality", "12", CVAR_NONE};
cvar_t		  r_skyalpha = {"r_skyalpha", "1", CVAR_NONE};
cvar_t		  r_skyfog = {"r_skyfog", "0.5", CVAR_NONE};
cvar_t		  r_skywind = {"r_skywind", "1", CVAR_ARCHIVE};
#define SKYWIND_CFG "wind.cfg"

typedef struct skybox_s
{
	char		 name[1024]; // name of current skybox, or "" if no skybox
	char		 name_worldspawn[1024];
	gltexture_t *textures[6];
	gltexture_t *cubemap;
	float		 wind_dist;
	float		 wind_yaw;
	float		 wind_pitch;
	float		 wind_period;
} skybox_t;
extern skybox_t skybox; // gl_sky.rs
extern float	skyfog;	 // gl_sky.rs
void Skywind_Load_f (void);

static bool Skywind_is_enabled (void)
{
	return skybox.name[0] && (r_skywind.value > 0.0f) &&
		   ((skybox.wind_dist != 0.0f) || (skybox.wind_period != 0.0f) || (skybox.wind_pitch != 0.0f) || (skybox.wind_yaw != 0.0f));
}

/*
=================
Skywind_Clear
=================
*/
static void Skywind_Clear (void)
{
	// reset
	skybox.wind_dist = skybox.wind_period = skybox.wind_pitch = skybox.wind_yaw = 0.0f;
}

/*
=================
Skywind_Load_f
=================
*/
void Skywind_Load_f (void)
{
	char		relname[MAX_QPATH];
	char	   *buf;
	const char *data;

	Skywind_Clear ();

	if (!skybox.name[0])
	{
		Con_Printf ("No skybox loaded\n");
		return;
	}

	q_snprintf (relname, sizeof (relname), "gfx/env/%s" SKYWIND_CFG, skybox.name);
	buf = (char *)COM_LoadFile (relname, NULL);
	if (!buf)
	{
		Con_DPrintf ("Sky wind config not found '%s'.\n", relname);
		return;
	}

	data = COM_Parse (buf);
	if (!data)
		goto done;

	if (strcmp (com_token, "skywind") != 0)
	{
		Con_Printf ("Skywind_Load_f: first token must be 'skywind'.\n");
		goto done;
	}

	if ((data = COM_Parse (data)) != NULL)
		skybox.wind_dist = CLAMP (-2.0, atof (com_token), 2.0);

	if ((data = COM_Parse (data)) != NULL)
		skybox.wind_yaw = fmod (atof (com_token), 360.0);

	if ((data = COM_Parse (data)) != NULL)
		skybox.wind_period = atof (com_token);

	if ((data = COM_Parse (data)) != NULL)
		skybox.wind_pitch = fmod (atof (com_token) + 90.0, 180.0) - 90.0;

done:
	Mem_Free (buf);
}

/*
=================
Skywind_Save_f
=================
*/
static void Skywind_Save_f (void)
{
	char  relname[MAX_QPATH];
	char  path[MAX_OSPATH];
	FILE *f;

	if (!skybox.name[0])
	{
		Con_Printf ("No skybox loaded\n");
		return;
	}

	q_snprintf (relname, sizeof (relname), "gfx/env/%s" SKYWIND_CFG, skybox.name);
	q_snprintf (path, sizeof (path), "%s/%s", com_gamedir, relname);
	f = Sys_fopen (path, "wt");
	if (!f)
	{
		Con_Printf ("Couldn't write '%s'.\n", relname);
		return;
	}

	fprintf (
		f,
		"// distance yaw period pitch\n"
		"skywind %g %g %g %g\n",
		skybox.wind_dist, skybox.wind_yaw, skybox.wind_period, skybox.wind_pitch);

	fclose (f);

	Con_SafePrintf ("Wrote ");
	Con_LinkPrintf (path, "%s", relname);
	Con_SafePrintf ("\n");
}

/*
=================
Skywind_LookDir_f
=================
*/
static void Skywind_LookDir_f (void)
{
	if (cls.state != ca_connected)
		return;

	if (!skybox.name[0])
	{
		Con_Printf ("No skybox loaded\n");
		return;
	}

	// invert view direction so that clouds move towards the player, not away from them
	skybox.wind_yaw = fmod (cl.viewangles[YAW] + 180.0, 360.0);
	skybox.wind_pitch = -cl.viewangles[PITCH];

	// first argument, if present, overrides the loop duration (default: 30 seconds)
	if (Cmd_Argc () >= 2)
		skybox.wind_period = atof (Cmd_Argv (1));
	else if (!skybox.wind_period)
		skybox.wind_period = 30.f;

	// second argument, if present, overrides the amplitude of the movement (default: 1.0)
	if (Cmd_Argc () >= 3)
		skybox.wind_dist = CLAMP (-2.0, atof (Cmd_Argv (2)), 2.0);
	else if (!skybox.wind_dist)
		skybox.wind_dist = 1.f;
}

/*
=================
Skywind_Rotate_f
=================
*/
static void Skywind_Rotate_f (void)
{
	if (cls.state != ca_connected)
		return;

	if (!skybox.name[0])
	{
		Con_Printf ("No skybox loaded\n");
		return;
	}

	if (Cmd_Argc () < 2)
	{
		Con_Printf (
			"usage:\n"
			"   %s <yawdelta> [pitchdelta]\n",
			Cmd_Argv (0));
		return;
	}

	skybox.wind_yaw = fmod (skybox.wind_yaw + atof (Cmd_Argv (1)), 360.0);
	if (Cmd_Argc () >= 3)
		skybox.wind_pitch = fmod (skybox.wind_pitch + atof (Cmd_Argv (2)) + 90.0, 180.0) - 90.0;
}

/*
=================
Skywind_f
=================
*/
static void Skywind_f (void)
{
	if (cls.state != ca_connected)
		return;

	if (!skybox.name[0])
	{
		Con_Printf ("No skybox loaded\n");
		return;
	}

	if (Cmd_Argc () < 2)
	{
		Con_Printf (
			"usage:\n"
			"   %s [distance] [yaw] [period] [pitch]\n"
			"current values:\n"
			"   \"distance\" is \"%g\"\n"
			"   \"yaw\"      is \"%g\"\n"
			"   \"period\"   is \"%g\"\n"
			"   \"pitch\"    is \"%g\"\n",
			Cmd_Argv (0), skybox.wind_dist, skybox.wind_yaw, skybox.wind_period, skybox.wind_pitch);
		return;
	}

	skybox.wind_dist = CLAMP (-2.0, atof (Cmd_Argv (1)), 2.0);
	if (Cmd_Argc () >= 3)
		skybox.wind_yaw = fmod (atof (Cmd_Argv (2)), 360.0);
	if (Cmd_Argc () >= 4)
		skybox.wind_period = atof (Cmd_Argv (3));
	if (Cmd_Argc () >= 5)
		skybox.wind_pitch = fmod (atof (Cmd_Argv (4)) + 90.0, 180.0) - 90.0;
}
/*
=================
Sky_GetSkyCommand

To preserve dynamic skies in demos and savegames
=================
*/
const char *Sky_GetSkyCommand (qboolean always)
{
	qboolean need_sky = always || strcmp (skybox.name, skybox.name_worldspawn);
	qboolean need_skyfog = always; // no safe way to record skyfog in demos; r_skyfog is user pref

	if (need_sky || need_skyfog)
	{
		char sky[128];
		char fog[128];
		q_strlcpy (sky, va ("sky \"%s\"", skybox.name), sizeof (sky));
		q_strlcpy (fog, va ("skyfog %g", skyfog), sizeof (fog));
		return va ("\n%s%s%s\n", need_sky ? sky : "", need_sky && need_skyfog ? "\n" : "", need_skyfog ? fog : "");
	}

	return NULL;
}

/*
=================
Sky_SkyCommand_f
=================
*/
void Sky_SkyCommand_f (void)
{
	switch (Cmd_Argc ())
	{
	case 1:
	{
		const char *wind_params_str = (Skywind_is_enabled ()) ? va (", wind dist %.3f yaw %.3f period %.3f pitch %.3f", skybox.wind_dist, skybox.wind_yaw,
																	skybox.wind_period, skybox.wind_pitch)
															  : "";
		Con_Printf ("sky is \"%s\"%s\n", skybox.name, wind_params_str);
	}
	break;
	case 2:
		Sky_LoadSkyBox (Cmd_Argv (1));
		break;
	default:
		Con_Printf ("usage: sky <skyname>\n");
	}
}

/*
====================
R_SetSkyfog_f -- ericw
====================
*/
static void R_SetSkyfog_f (cvar_t *var)
{
	// clear any skyfog setting from worldspawn
	skyfog = var->value;
}

/*
=============
Sky_Init
=============
*/
void Sky_Init (void)
{
	Cvar_RegisterVariable (&r_fastsky);
	Cvar_RegisterVariable (&r_sky_quality);
	Cvar_RegisterVariable (&r_skyalpha);
	Cvar_RegisterVariable (&r_skyfog);
	Cvar_SetCallback (&r_skyfog, R_SetSkyfog_f);
	Cvar_RegisterVariable (&r_skywind);

	Cmd_AddCommand ("sky", Sky_SkyCommand_f);
	Cmd_AddCommand ("skywind", Skywind_f);
	Cmd_AddCommand ("skywind_save", Skywind_Save_f);
	Cmd_AddCommand ("skywind_load", Skywind_Load_f);
	Cmd_AddCommand ("skywind_lookdir", Skywind_LookDir_f);
	Cmd_AddCommand ("skywind_rotate", Skywind_Rotate_f);

}
