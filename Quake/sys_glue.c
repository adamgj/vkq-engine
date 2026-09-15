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
// sys_glue.c -- the C frame around the Rust sys.h layer.
//
// Compiled instead of sys_sdl.c + sys_sdl_win.c/sys_sdl_unix.c under
// -Duse_rust_platform (Rust migration Phase 9 M5, ADR-017). Four jobs:
//
//  1. Keep the two variadic entry points, Sys_Error and Sys_Printf, in C:
//     each formats with q_vstrcatf and hands the finished text to the Rust
//     core (task plan I5). Sys_Error's core owns and frees the buffer.
//  2. Guard Host_Shutdown, a Host_Reraise wrapper under -Duse_rust_host
//     (ADR-009 rule 3), for the Rust Sys_Quit / unix Sys_Error paths.
//  3. Re-raise from Sys_Error, Sys_Quit and Sys_SendKeyEvents what the
//     guards caught. The remaining sys.h entry points cannot raise and are
//     exported by quake-capi under their C names directly.
//  4. Carry the MSVC comctl32 v6 manifest pragma sys_sdl.c held.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_PLATFORM

#if defined(_WIN32) && defined(_MSC_VER)
// comctl32 v6 activation context: with this in the manifest, SDL_ShowMessageBox
// takes its native TaskDialogIndirect path instead of the hand-built dialog fallback
#pragma comment( \
	linker,      \
	"\"/manifestdependency:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'\"")
#endif

/* ---------------------------------------------------------------------------
 * Guarded callback (ADR-009 rule 3).
 */

static void SysGlue_InvokeHostShutdown (void *p)
{
	(void)p;
	Host_Shutdown ();
}

int SysGlue_HostShutdown (void)
{
	return Host_Guard (SysGlue_InvokeHostShutdown, NULL);
}

/* ---------------------------------------------------------------------------
 * Variadic entry points (I5) and re-raising frames (ADR-009).
 */

/* sys_sdl_win.c:654 / sys_sdl_unix.c Sys_Error -- the core exits the process
   and returns only with a guard result to re-issue. */
void Sys_Error (const char *error, ...)
{
	va_list argptr;

	va_start (argptr, error);
	char *text = q_vstrcatf (NULL, error, argptr);
	va_end (argptr);

	Host_Reraise (quake_rs_sys_error (text));
	exit (1);
}

/* sys_sdl_win.c Sys_Printf / sys_sdl_unix.c:607 */
void Sys_Printf (const char *fmt, ...)
{
	va_list argptr;

	va_start (argptr, fmt);
	char *text = q_vstrcatf (NULL, fmt, argptr);
	va_end (argptr);

	quake_rs_sys_printf (text);
	Mem_Free (text);
}

/* sys_sdl_win.c Sys_Quit / sys_sdl_unix.c:616 */
void Sys_Quit (void)
{
	Host_Reraise (quake_rs_sys_quit ());
	exit (0);
}

/* sys_sdl.c Sys_SendKeyEvents */
void Sys_SendKeyEvents (void)
{
	Host_Reraise (quake_rs_sys_send_key_events ());
}

#endif /* USE_RUST_PLATFORM */
