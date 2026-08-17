/*
Copyright (C) 2026 vkQuake contributors

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
#ifndef QUAKE_HARNESS_H
#define QUAKE_HARNESS_H

/* Differential-verification harness (Rust migration Phase 0, PLAN.md section 7 / ADR-019).

   -headless           run the client without video/audio/input (implies no_rendering)
   -demohash <file>    write a per-frame chained state hash to <file>; forces
                       deterministic fixed-timestep pacing and a fixed RNG seed
   -exitafter <n>      hard frame cap; exit code 2 when reached (runaway guard)
   -harnesscmds <file> inject console commands at fixed frame numbers; each line
                       is "<framecount> <command...>"

   Goldens are generated and compared only between headless runs on the same
   platform: the headless RNG stream intentionally differs from a windowed run. */

extern qboolean harness_active;

/* true when running without a renderer: dedicated server or -headless client */
extern qboolean no_rendering;

void Harness_CheckArgs (void); /* early, right after COM_InitArgv */
void Harness_Init (void);	   /* end of Host_Init */
void Harness_Frame (void);	   /* end of _Host_Frame, before host_framecount++ */
void Harness_DemoEnded (void); /* demo playback finished: flush hashes and exit 0 */
void Harness_Shutdown (void);  /* finalize the hash file; safe to call twice */

/* fixed timestep fed to Host_Frame when harness_active */
double Harness_FrameTime (void);

uint64_t Harness_Hash64 (uint64_t h, const void *data, size_t len);

#endif /* QUAKE_HARNESS_H */
