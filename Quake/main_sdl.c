/*
Copyright (C) 1996-2001 Id Software, Inc.
Copyright (C) 2002-2005 John Fitzgibbons and others
Copyright (C) 2007-2008 Kristian Duske
Copyright (C) 2010-2014 QuakeSpasm developers

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

#include "quakedef.h"
#include <stdio.h>

#ifdef USE_SDL3
// provides the WinMain shim; must be included in the file that defines main ()
#include <SDL3/SDL_main.h>
#endif

static void Sys_AtExit (void)
{
	SDL_Quit ();
}

static void Sys_InitSDL (void)
{
#ifdef USE_SDL3
	int version = SDL_GetVersion ();
	int major = SDL_VERSIONNUM_MAJOR (version);
	int minor = SDL_VERSIONNUM_MINOR (version);
	int patch = SDL_VERSIONNUM_MICRO (version);
#else
	SDL_version version;
	SDL_GetVersion (&version);
	int major = version.major;
	int minor = version.minor;
	int patch = version.patch;
#endif

	Sys_Printf ("Using SDL version %i.%i.%i\n", major, minor, patch);

#ifdef USE_SDL3
	const bool initialized = SDL_Init (0);
#else
	const bool initialized = SDL_Init (0) >= 0;
#endif

	if (!initialized)
	{
		Sys_Error ("Couldn't init SDL: %s", SDL_GetError ());
	}

#ifdef _DEBUG
#ifdef USE_SDL3
	SDL_SetLogPriorities (SDL_LOG_PRIORITY_DEBUG);
#else
	SDL_LogSetAllPriority (SDL_LOG_PRIORITY_DEBUG);
#endif
#endif

	atexit (Sys_AtExit);
}

static quakeparms_t parms;

int main (int argc, char *argv[])
{
	double time, oldtime, newtime;

	host_parms = &parms;
	parms.basedir = ".";

	parms.argc = argc;
	parms.argv = argv;

	parms.errstate = 0;

	COM_InitArgv (parms.argc, parms.argv);

	isDedicated = (COM_CheckParm ("-dedicated") != 0);
	Harness_CheckArgs ();

	Sys_InitSDL ();

	Sys_Init ();

#ifdef USE_SDL3
	Sys_Printf ("Detected %d CPUs.\n", SDL_GetNumLogicalCPUCores ());
#else
	Sys_Printf ("Detected %d CPUs.\n", SDL_GetCPUCount ());
#endif
	Sys_Printf ("Initializing %s\n", ENGINE_NAME_AND_VER);
#if defined(__clang_version__)
	Sys_Printf ("Built with Clang " __clang_version__ "\n");
#elif defined(__GNUC__)
	Sys_Printf ("Built with GCC %u.%u.%u\n", __GNUC__, __GNUC_MINOR__, __GNUC_PATCHLEVEL__);
#elif defined(_MSC_FULL_VER)
	Sys_Printf ("Built with Microsoft C %u\n", _MSC_FULL_VER);
#else
	Sys_Printf ("Built with unknown compiler\n");
#endif

	Sys_Printf ("Host_Init\n");
	Host_Init ();

	oldtime = Sys_DoubleTime ();
	if (isDedicated)
	{
		while (1)
		{
			newtime = Sys_DoubleTime ();
			time = newtime - oldtime;

			while (time < sys_ticrate.value)
			{
				SDL_Delay (1);
				newtime = Sys_DoubleTime ();
				time = newtime - oldtime;
			}

			/* fixed timestep: state must not depend on wall-clock time */
			if (harness_fixed_dt)
				time = Harness_FrameTime ();

			Host_Frame (time);
			oldtime = newtime;
		}
	}
	else
		while (1)
		{
			if (!no_rendering)
			{
				/* If we have no input focus at all, sleep a bit */
				if ((!listening && !VID_HasMouseOrInputFocus ()) || cl.paused)
				{
					SDL_Delay (16);
				}
				/* If we're minimised, sleep a bit more */
				if (!listening && VID_IsMinimized ())
					SDL_Delay (32);
			}

			/* A harness client with no fixed timestep (-headless alone, e.g.
			   interop_matrix.py's live network client) is not a single-process
			   demo/sound/netreplay hash subject -- it is one half of a genuine
			   two-process network session. Left unthrottled it races through
			   -exitafter frames as fast as the CPU allows, flooding the
			   dedicated server's unreliable channel with a burst of clc_move
			   datagrams far faster than the server (paced to sys_ticrate)
			   drains its socket -- OS receive-buffer drop timing is then
			   genuinely nondeterministic across separate launches, which
			   showed up as a real (not fault-injected) per-frame state-hash
			   divergence around frame 128 in two independently-launched,
			   otherwise-identical C/C soak sessions. Pacing this client to
			   the same real-time cadence as the dedicated server keeps the
			   two processes' packet exchange bounded and reproducible.

			   The predicate is deliberately wider than that one script: it
			   also paces the other non-fixed-dt harness modes (builtin_diff's
			   20-frame run, capture_session's 1200-frame capture, which is
			   itself half of a two-process session). The cost is
			   frames * sys_ticrate of wall clock for runs that may have no
			   peer; the alternative -- an opt-in flag -- would leave any
			   future networked harness mode silently unpaced, which is the
			   failure this fix exists to remove. Note the floor is
			   sys_ticrate, nominally the *dedicated server* tic rate
			   (host.c:79): a scenario that sets it changes this client's
			   cadence too. */
			newtime = Sys_DoubleTime ();
			time = newtime - oldtime;
			if (harness_active && !harness_fixed_dt)
			{
				while (time < sys_ticrate.value)
				{
					SDL_Delay (1);
					newtime = Sys_DoubleTime ();
					time = newtime - oldtime;
				}
			}

			/* fixed timestep: state must not depend on wall-clock time */
			if (harness_fixed_dt)
				time = Harness_FrameTime ();

			Host_Frame (time);

			oldtime = newtime;
		}

	return 0;
}
