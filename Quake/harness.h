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
#ifndef QUAKE_HARNESS_H
#define QUAKE_HARNESS_H

#include "q_types.h" // qboolean, byte, uint64_t

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

/* -sndhash <file> (Phase 4 PCM-hash gate): write a per-frame chained hash of
   the software mixer's output. Initializes sound even under -headless,
   replacing the SDL DMA backend with a deterministic fixed-format clock;
   forces fixed timestep. True when -sndhash is active: S_Startup /
   GetSoundtime / S_Shutdown route to the Harness_SNDDMA_* clock below
   instead of the real SDL audio backend. */
extern qboolean harness_sndhash;

/* true when -demohash is active: the main loop feeds a fixed timestep so
   state is wall-clock independent. Live-network runs (-netcapture without
   -demohash) keep real pacing, since a remote peer runs on wall time. */
extern qboolean harness_fixed_dt;

/* true when running without a renderer: dedicated server or -headless client */
extern qboolean no_rendering;

void Harness_CheckArgs (void); /* early, right after COM_InitArgv */
void Harness_Init (void);	   /* end of Host_Init */
void Harness_Frame (void);	   /* end of _Host_Frame, before host_framecount++ */
void Harness_DemoEnded (void); /* demo playback finished: flush hashes and exit 0 */
void Harness_Shutdown (void);  /* finalize the hash+capture files; safe to call twice */

/* -netcapture <file>: message-level capture at the NET_* funnels. Note this is
   whole logical messages, not packets: the dgrm driver fragments reliable
   messages into several UDP datagrams below this layer and reassembles on
   receive, so packet boundaries are not visible here.
   Framed records: [u8 direction 0=recv,1=send][u8 driver]
				   [u8 kind 0=unknown,1=reliable,2=unreliable][u32le len][payload]
   kind 0 is emitted by the server-side NET_GetServerMessage funnel, which has
   no reliability information available at that point. */
void Harness_NetCapture (int direction, int driver, int kind, const byte *data, int len);

/* -netreplay <capture> (Phase 5 M8, ADR-019 gate 4 "captured-session
   replay"): deterministic client-side replay of a -netcapture recv stream.
   NET_Connect hands out a pseudo-socket, each host frame delivers at most
   one captured recv record into net_message, and the send/can-send funnels
   absorb the client's output. Forces the fixed timestep, so with -demohash
   the replayed session's state-hash chain (and any demo recorded during
   it) is byte-comparable across builds. The instrument is identical C in
   every configuration; the surface under test is everything beneath the
   funnels -- the MSG/SZ readers, cl_parse, and the demo writer. */
extern qboolean harness_netreplay;

struct qsocket_s *Harness_NetReplayConnect (void);
qboolean		  Harness_NetReplayOwns (struct qsocket_s *sock);
int				  Harness_NetReplayGetMessage (void);

/* cumulative count of MSG_Read* underruns (msg_badread events). Not a
   zero-gate: the dgrm connect path deliberately probes optional ProQuake
   fields until badread reports end-of-message. Printed at Harness_Shutdown
   ("Harness: msgbadread=<n>") so interop scripts can compare C and Rust
   builds cell-for-cell (Phase 5). */
extern unsigned int harness_badread_count;

/* fixed timestep fed to Host_Frame when harness_active */
double Harness_FrameTime (void);

/* deterministic DMA backend for -sndhash: fixed 44100 Hz / 16-bit / stereo,
   sample position derived from host_framecount so headless runs need no audio
   device and two runs of the same script produce identical mixer output */
qboolean Harness_SNDDMA_Init (void *dma); /* dma_t*; void to keep this header q_sound.h-free */
int		 Harness_SNDDMA_GetDMAPos (void);
void	 Harness_SNDDMA_Shutdown (void);

/* fold one S_PaintChannels block into the -sndhash chain: the painted
   region of the mix buffer plus the DMA buffer after transfer */
void Harness_SndPaint (int painted, int end, const void *paintbuf, const volatile unsigned char *dmabuf, int dmabytes);

uint64_t Harness_Hash64 (uint64_t h, const void *data, size_t len);

/* fold the classic particle simulator's active list into the client hash.
   Defined in r_part.c, which owns the statics; a no-op before
   R_InitParticles (dedicated servers never call it). */
uint64_t Harness_HashParticles (uint64_t h);

#endif /* QUAKE_HARNESS_H */
