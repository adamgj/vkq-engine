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

extern cvar_t snd_pauselooping; // defined below

// the S_PaintChannels pause_loops predicate (snd_mix.c line: looping sounds
// keep silent while the game is effectively paused)
qboolean SND_Glue_PauseLoops (void)
{
	return snd_pauselooping.value && (cl.paused || (sv.active && svs.maxclients == 1 && key_dest != key_game));
}

/* ---------------------------------------------------------------------------
 * Phase 4 M6b: compat-surface storage from snd_dma.c. These stay C-defined so
 * every existing C reader keeps working unchanged: menu.c reads
 * sfxvolume/bgmvolume cvar storage directly, cl_demo.c iterates
 * snd_channels[]/total_channels, gl_screen/host read the timing globals.
 * The logic lives in quake-capi's snd_dma.rs.
 */

channel_t snd_channels[MAX_CHANNELS];
int		  total_channels;

dma_t			sn;
volatile dma_t *shm = NULL;

vec3_t listener_origin;
vec3_t listener_forward;
vec3_t listener_right;
vec3_t listener_up;

int soundtime;	 // sample PAIRS
int paintedtime; // sample PAIRS

int					  s_rawend;
portable_samplepair_t s_rawsamples[MAX_RAW_SAMPLES];

qmutex_t *snd_mutex;

cvar_t bgmvolume = {"bgmvolume", "1", CVAR_ARCHIVE};
cvar_t sfxvolume = {"volume", "0.7", CVAR_ARCHIVE};

cvar_t precache = {"precache", "1", CVAR_NONE};
cvar_t loadas8bit = {"loadas8bit", "0", CVAR_NONE};

cvar_t sndspeed = {"sndspeed", "11025", CVAR_NONE};
cvar_t snd_mixspeed = {"snd_mixspeed", "44100", CVAR_NONE};

cvar_t snd_waterfx = {"snd_waterfx", "1", CVAR_ARCHIVE};

cvar_t snd_pauselooping = {"snd_pauselooping", "1", CVAR_ARCHIVE};

#if defined(_WIN32)
#define SND_FILTERQUALITY_DEFAULT "5"
#else
#define SND_FILTERQUALITY_DEFAULT "1"
#endif

cvar_t snd_filterquality = {"snd_filterquality", SND_FILTERQUALITY_DEFAULT, CVAR_NONE};

cvar_t nosound = {"nosound", "0", CVAR_NONE};
cvar_t ambient_level = {"ambient_level", "0.3", CVAR_NONE};
cvar_t ambient_fade = {"ambient_fade", "100", CVAR_NONE};
cvar_t snd_noextraupdate = {"snd_noextraupdate", "0", CVAR_NONE};
cvar_t snd_show = {"snd_show", "0", CVAR_NONE};
cvar_t _snd_mixahead = {"_snd_mixahead", "0.1", CVAR_ARCHIVE};

// the client/server state snd_dma.c read directly (quakedef.h/client.h are
// not bindgen-clean, so Rust reaches it through these accessors)
qboolean SND_Glue_ClientConnected (void)
{
	return cls.state == ca_connected && cls.signon == SIGNONS;
}

int SND_Glue_ViewEntity (void)
{
	return cl.viewentity;
}

// NULL when there is no usable worldmodel (matches snd_dma.c's
// `!cl.worldmodel || cl.worldmodel->needload` early-out)
qmodel_t *SND_Glue_Worldmodel (void)
{
	return (cl.worldmodel && !cl.worldmodel->needload) ? cl.worldmodel : NULL;
}

mleaf_t *SND_Glue_PointInLeaf (float *p)
{
	return Mod_PointInLeaf (p, cl.worldmodel);
}
