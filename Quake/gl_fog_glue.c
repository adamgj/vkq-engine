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

// gl_fog_glue.c -- the C that stays with gl_fog.c under -Duse_rust_render
// (Rust migration Phase 8 M7). The fog state, fades, worldspawn/server-message
// parsing and the push-constant setup are quake-capi's gl_fog.rs, which
// exports the fog_* globals and fade_done with their C names. What remains
// here is what formats with %g/%f (ADR-005 gap) and the command registration.
// Every function body is verbatim gl_fog.c.

#include "quakedef.h"

#define DEFAULT_DENSITY 0.0
#define DEFAULT_GRAY	0.3

extern float fog_density;
extern float fog_red;
extern float fog_green;
extern float fog_blue;
extern float fade_done;

/*
=============
Fog_FogCommand_f

handle the 'fog' console command
=============
*/
void Fog_FogCommand_f (void)
{
	switch (Cmd_Argc ())
	{
	default:
	case 1:
		Con_Printf ("usage:\n");
		Con_Printf ("   fog <density>\n");
		Con_Printf ("   fog <red> <green> <blue>\n");
		Con_Printf ("   fog <density> <red> <green> <blue>\n");
		Con_Printf ("current values:\n");
		Con_Printf ("   \"density\" is \"%f\"\n", fog_density);
		Con_Printf ("   \"red\" is \"%f\"\n", fog_red);
		Con_Printf ("   \"green\" is \"%f\"\n", fog_green);
		Con_Printf ("   \"blue\" is \"%f\"\n", fog_blue);
		break;
	case 2:
		Fog_Update (q_max (0.0, atof (Cmd_Argv (1))), fog_red, fog_green, fog_blue, 0.0);
		break;
	case 3: // TEST
		Fog_Update (q_max (0.0, atof (Cmd_Argv (1))), fog_red, fog_green, fog_blue, atof (Cmd_Argv (2)));
		break;
	case 4:
		Fog_Update (fog_density, CLAMP (0.0, atof (Cmd_Argv (1)), 1.0), CLAMP (0.0, atof (Cmd_Argv (2)), 1.0), CLAMP (0.0, atof (Cmd_Argv (3)), 1.0), 0.0);
		break;
	case 5:
		Fog_Update (
			q_max (0.0, atof (Cmd_Argv (1))), CLAMP (0.0, atof (Cmd_Argv (2)), 1.0), CLAMP (0.0, atof (Cmd_Argv (3)), 1.0),
			CLAMP (0.0, atof (Cmd_Argv (4)), 1.0), 0.0);
		break;
	case 6: // TEST
		Fog_Update (
			q_max (0.0, atof (Cmd_Argv (1))), CLAMP (0.0, atof (Cmd_Argv (2)), 1.0), CLAMP (0.0, atof (Cmd_Argv (3)), 1.0),
			CLAMP (0.0, atof (Cmd_Argv (4)), 1.0), atof (Cmd_Argv (5)));
		break;
	}
}

/*
=============
Fog_GetFogCommand

so fog is preserved when starting a demo recording or in savegames
=============
*/
const char *Fog_GetFogCommand (qboolean always)
{
	if (fade_done || always)
		return va ("\nfog %g %g %g %g\n", fog_density, fog_red, fog_green, fog_blue);
	return NULL;
}

//==============================================================================
//
//  VOLUMETRIC FOG
//
//==============================================================================

cvar_t r_vfog = {"r_vfog", "1", CVAR_NONE};

//==============================================================================
//
//  INIT
//
//==============================================================================

/*
=============
Fog_Init

called when quake initializes
=============
*/
void Fog_Init (void)
{
	Cmd_AddCommand ("fog", Fog_FogCommand_f);

	// Cvar_RegisterVariable (&r_vfog);

	// set up global fog
	fog_density = DEFAULT_DENSITY;
	fog_red = DEFAULT_GRAY;
	fog_green = DEFAULT_GRAY;
	fog_blue = DEFAULT_GRAY;
}
