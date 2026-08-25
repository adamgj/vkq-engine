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

// snd_glue.c -- C-side glue for the Rust sound engine (Phase 4, ADR-011).
// Compiled only under -Duse_rust_snd; gives the Rust shims access to engine
// state whose headers are not bindgen-clean (cl/sv/svs/key_dest live in
// quakedef.h/client.h).

#include "quakedef.h"

extern cvar_t snd_pauselooping;

// the S_PaintChannels pause_loops predicate (snd_mix.c line: looping sounds
// keep silent while the game is effectively paused)
qboolean SND_Glue_PauseLoops (void)
{
	return snd_pauselooping.value && (cl.paused || (sv.active && svs.maxclients == 1 && key_dest != key_game));
}
