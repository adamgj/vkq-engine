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

// harness.c -- differential-verification harness (Rust migration Phase 0, ADR-019)

#include "quakedef.h"
#include "arch_def.h"
#include "net_sys.h"
#include "net_defs.h"
#include "harness.h"

qboolean harness_active = false;
qboolean harness_fixed_dt = false;
qboolean no_rendering = false;
qboolean harness_sndhash = false;

/* fixed simulation rate; the -sndhash DMA clock derives from the same
   constant so the mixer and the simulation can never drift apart */
#define HARNESS_FRAME_RATE 72
#define HARNESS_FRAME_DT   (1.0 / HARNESS_FRAME_RATE)
#define HARNESS_RAND_SEED  0x76715248 /* 'vqRH' */
#define HARNESS_MAX_CMDS   256
#define HARNESS_HASH_BASIS UINT64_C (0xcbf29ce484222325)
#define HARNESS_HASH_PRIME UINT64_C (0x100000001b3)

static FILE		   *harness_hashfile = NULL;
static FILE		   *harness_capturefile = NULL;
qboolean			harness_netreplay = false;
static FILE		   *harness_replayfile = NULL;
static qsocket_t   *harness_replaysock = NULL;
static int			harness_replay_lastframe = -1;
static unsigned int harness_replay_delivered = 0;
static FILE		   *harness_sndfile = NULL;
static uint64_t		harness_hashchain = HARNESS_HASH_BASIS;
static uint64_t		harness_sndchain = HARNESS_HASH_BASIS;
static int			harness_exitafter = 0;

typedef struct
{
	int	  frame;
	char *command;
} harness_cmd_t;

static harness_cmd_t harness_cmds[HARNESS_MAX_CMDS];
static int			 harness_numcmds = 0;
static int			 harness_nextcmd = 0;

uint64_t Harness_Hash64 (uint64_t h, const void *data, size_t len)
{
	const byte *p = (const byte *)data;
	while (len--)
	{
		h ^= *p++;
		h *= HARNESS_HASH_PRIME;
	}
	return h;
}

double Harness_FrameTime (void)
{
	return HARNESS_FRAME_DT;
}

void Harness_CheckArgs (void)
{
	if (COM_CheckParm ("-headless"))
		no_rendering = true;
	if (COM_CheckParm ("-headless") || COM_CheckParm ("-demohash") || COM_CheckParm ("-exitafter") || COM_CheckParm ("-harnesscmds") ||
		COM_CheckParm ("-netcapture") || COM_CheckParm ("-sndhash") || COM_CheckParm ("-netreplay"))
		harness_active = true;
	if (COM_CheckParm ("-demohash") || COM_CheckParm ("-sndhash") || COM_CheckParm ("-netreplay"))
		harness_fixed_dt = true;
	if (COM_CheckParm ("-netreplay"))
		harness_netreplay = true;
	if (COM_CheckParm ("-sndhash"))
		harness_sndhash = true;
	if (isDedicated)
		no_rendering = true;
}

static void Harness_LoadCmds (const char *path)
{
	FILE *f = Sys_fopen (path, "r");
	char  line[1024];
	int	  i;

	if (!f)
		Sys_Error ("Harness: can't open -harnesscmds file %s", path);
	while (fgets (line, sizeof (line), f))
	{
		char *cmd = NULL;
		long  frame = strtol (line, &cmd, 10);
		if (cmd == line || !cmd)
			continue; /* no leading frame number: comment/blank line */
		while (*cmd == ' ' || *cmd == '\t')
			cmd++;
		size_t len = strlen (cmd);
		while (len && (cmd[len - 1] == '\n' || cmd[len - 1] == '\r'))
			cmd[--len] = 0;
		if (!len)
			continue;
		if (harness_numcmds == HARNESS_MAX_CMDS)
			Sys_Error ("Harness: more than %d commands in %s", HARNESS_MAX_CMDS, path);
		harness_cmds[harness_numcmds].frame = (int)frame;
		harness_cmds[harness_numcmds].command = q_strdup (cmd);
		harness_numcmds++;
	}
	fclose (f);

	/* stable insertion sort: commands sharing a frame must keep file order.
	   qsort is not stable and its ordering of equal keys varies by libc, which
	   would make the same script produce different goldens per platform. */
	for (i = 1; i < harness_numcmds; i++)
	{
		harness_cmd_t key = harness_cmds[i];
		int			  j = i - 1;
		while (j >= 0 && harness_cmds[j].frame > key.frame)
		{
			harness_cmds[j + 1] = harness_cmds[j];
			j--;
		}
		harness_cmds[j + 1] = key;
	}
}

void Harness_Init (void)
{
	int i;

	if (!harness_active)
		return;

	COM_SeedRand (HARNESS_RAND_SEED);
	cls.demonum = -1; /* no attract-loop chaining after a demo ends */

	i = COM_CheckParm ("-demohash");
	if (i && i < com_argc - 1)
	{
		harness_hashfile = Sys_fopen (com_argv[i + 1], "w");
		if (!harness_hashfile)
			Sys_Error ("Harness: can't open -demohash file %s", com_argv[i + 1]);
	}

	i = COM_CheckParm ("-sndhash");
	if (i && i < com_argc - 1)
	{
		harness_sndfile = Sys_fopen (com_argv[i + 1], "w");
		if (!harness_sndfile)
			Sys_Error ("Harness: can't open -sndhash file %s", com_argv[i + 1]);
	}

	i = COM_CheckParm ("-netcapture");
	if (i && i < com_argc - 1)
	{
		harness_capturefile = Sys_fopen (com_argv[i + 1], "wb");
		if (!harness_capturefile)
			Sys_Error ("Harness: can't open -netcapture file %s", com_argv[i + 1]);
	}

	i = COM_CheckParm ("-netreplay");
	if (i && i < com_argc - 1)
	{
		harness_replayfile = Sys_fopen (com_argv[i + 1], "rb");
		if (!harness_replayfile)
			Sys_Error ("Harness: can't open -netreplay file %s", com_argv[i + 1]);
	}
	else if (harness_netreplay)
		Sys_Error ("Harness: -netreplay needs a capture file argument");

	i = COM_CheckParm ("-exitafter");
	if (i && i < com_argc - 1)
		harness_exitafter = atoi (com_argv[i + 1]);

	i = COM_CheckParm ("-harnesscmds");
	if (i && i < com_argc - 1)
		Harness_LoadCmds (com_argv[i + 1]);
}

static uint64_t Harness_HashServer (uint64_t h)
{
	qcvm_t *vm = &sv.qcvm;
	int		i;

	if (!sv.active || !vm->progs)
		return h;

	for (i = 0; i < vm->num_edicts; i++)
	{
		edict_t *ed = (edict_t *)((byte *)vm->edicts + i * vm->edict_size);
		/* never hash the raw edict block: the leading fields hold pointers,
		   area links and a debug-only header whose layout differs per build.
		   Everything below is layout-stable, simulation-derived state that a
		   port could get wrong silently -- baseline drives delta encoding,
		   the lerp fields are LERP_BANDAID/sv_smoothplatformlerps state, and
		   num_leafs/leafnums are SV_FindTouchedLeafs' world-linkage output. */
		h = Harness_Hash64 (h, &ed->free, sizeof (ed->free));
		h = Harness_Hash64 (h, &ed->freetime, sizeof (ed->freetime));
		h = Harness_Hash64 (h, &ed->alpha, sizeof (ed->alpha));
		h = Harness_Hash64 (h, &ed->baseline, sizeof (ed->baseline));
		h = Harness_Hash64 (h, &ed->sendinterval, sizeof (ed->sendinterval));
		h = Harness_Hash64 (h, &ed->sendinterval_default, sizeof (ed->sendinterval_default));
		h = Harness_Hash64 (h, &ed->oldframe, sizeof (ed->oldframe));
		h = Harness_Hash64 (h, &ed->oldthinktime, sizeof (ed->oldthinktime));
		h = Harness_Hash64 (h, ed->predthinkpos, sizeof (ed->predthinkpos));
		h = Harness_Hash64 (h, &ed->lastthink, sizeof (ed->lastthink));
		/* only the populated leafnums: entries past num_leafs are stale
		   leftovers no observer can see, and demanding a port reproduce them
		   would fail the gate on a difference that cannot matter */
		h = Harness_Hash64 (h, &ed->num_leafs, sizeof (ed->num_leafs));
		if (ed->num_leafs <= MAX_ENT_LEAFS)
			h = Harness_Hash64 (h, ed->leafnums, ed->num_leafs * sizeof (ed->leafnums[0]));
		h = Harness_Hash64 (h, &ed->v, vm->edict_size - offsetof (edict_t, v));
	}
	h = Harness_Hash64 (h, vm->globals, vm->progs->numglobals * 4);
	h = Harness_Hash64 (h, &vm->time, sizeof (vm->time));
	h = Harness_Hash64 (h, &vm->num_edicts, sizeof (vm->num_edicts));
	return h;
}

static uint64_t Harness_HashClient (uint64_t h)
{
	int i;

	h = Harness_Hash64 (h, &cl.time, sizeof (cl.time));
	h = Harness_Hash64 (h, &cl.oldtime, sizeof (cl.oldtime));
	h = Harness_Hash64 (h, cl.mtime, sizeof (cl.mtime));
	h = Harness_Hash64 (h, cl.viewangles, sizeof (cl.viewangles));
	h = Harness_Hash64 (h, cl.mviewangles, sizeof (cl.mviewangles));
	h = Harness_Hash64 (h, cl.velocity, sizeof (cl.velocity));
	h = Harness_Hash64 (h, cl.punchangle, sizeof (cl.punchangle));
	h = Harness_Hash64 (h, &cl.idealpitch, sizeof (cl.idealpitch));
	h = Harness_Hash64 (h, &cl.viewheight, sizeof (cl.viewheight));
	h = Harness_Hash64 (h, &cl.onground, sizeof (cl.onground));
	h = Harness_Hash64 (h, &cl.inwater, sizeof (cl.inwater));
	h = Harness_Hash64 (h, &cl.paused, sizeof (cl.paused));
	h = Harness_Hash64 (h, &cl.intermission, sizeof (cl.intermission));
	h = Harness_Hash64 (h, &cl.items, sizeof (cl.items));
	h = Harness_Hash64 (h, cl.stats, sizeof (cl.stats));
	h = Harness_Hash64 (h, cl.statsf, sizeof (cl.statsf));

	for (i = 0; i < cl.num_entities; i++)
	{
		entity_t *ent = &cl.entities[i];
		h = Harness_Hash64 (h, ent->origin, sizeof (ent->origin));
		h = Harness_Hash64 (h, ent->angles, sizeof (ent->angles));
		h = Harness_Hash64 (h, ent->msg_origins, sizeof (ent->msg_origins));
		h = Harness_Hash64 (h, ent->msg_angles, sizeof (ent->msg_angles));
		h = Harness_Hash64 (h, &ent->frame, sizeof (ent->frame));
		h = Harness_Hash64 (h, &ent->effects, sizeof (ent->effects));
		h = Harness_Hash64 (h, &ent->skinnum, sizeof (ent->skinnum));
		h = Harness_Hash64 (h, &ent->alpha, sizeof (ent->alpha));
	}
	return h;
}

/* -sndhash deterministic DMA backend: fixed 44100 Hz / 16-bit / stereo with a
   sample clock derived from host_framecount, so headless runs need no audio
   device and the mixer's input timing is bit-identical across runs. The
   format matches the SDL backends' common case; the buffer size is the SDL
   path's Q_nextPow2((2*44100)/10). */

#define HARNESS_SND_SPEED	 44100
#define HARNESS_SND_CHANNELS 2
#define HARNESS_SND_SAMPLES	 16384 /* interleaved samples in the ring buffer */

qboolean Harness_SNDDMA_Init (void *dma_)
{
	dma_t *dma = (dma_t *)dma_;

	memset (dma, 0, sizeof (dma_t));
	dma->samplebits = 16;
	dma->signed8 = 0;
	dma->speed = HARNESS_SND_SPEED;
	dma->channels = HARNESS_SND_CHANNELS;
	dma->samples = HARNESS_SND_SAMPLES;
	dma->samplepos = 0;
	dma->submission_chunk = 1;
	dma->buffer = (unsigned char *)Mem_Alloc (HARNESS_SND_SAMPLES * 2);
	shm = dma;
	Con_Printf ("Harness: deterministic sound clock (%d Hz, %d bit, %d ch)\n", dma->speed, dma->samplebits, dma->channels);
	return true;
}

int Harness_SNDDMA_GetDMAPos (void)
{
	/* monotone mono-sample clock: floor(framecount * speed * dt) frames of
	   audio have "played"; samplepos counts interleaved samples mod the ring */
	uint64_t frames = (uint64_t)host_framecount * HARNESS_SND_SPEED / HARNESS_FRAME_RATE;
	int		 samplepos = (int)((frames * HARNESS_SND_CHANNELS) % HARNESS_SND_SAMPLES);
	shm->samplepos = samplepos;
	return samplepos;
}

void Harness_SNDDMA_Shutdown (void)
{
	if (shm)
	{
		if (shm->buffer)
			Mem_Free (shm->buffer);
		shm->buffer = NULL;
		shm = NULL;
	}
}

void Harness_SndPaint (int painted, int end, const void *paintbuf, const volatile unsigned char *dmabuf, int dmabytes)
{
	uint64_t h;

	if (!harness_sndfile)
		return;
	h = harness_sndchain;
	h = Harness_Hash64 (h, &painted, sizeof (painted));
	h = Harness_Hash64 (h, &end, sizeof (end));
	h = Harness_Hash64 (h, paintbuf, (size_t)(end - painted) * 8); /* portable_samplepair_t is two ints */
	h = Harness_Hash64 (h, (const void *)dmabuf, (size_t)dmabytes);
	harness_sndchain = h;
}

unsigned int harness_badread_count = 0;

void Harness_Shutdown (void)
{
	static qboolean badread_printed = false;
	if (harness_active && !badread_printed)
	{
		badread_printed = true;
		Sys_Printf ("Harness: msgbadread=%u\n", harness_badread_count);
		if (harness_netreplay)
			Sys_Printf ("Harness: netreplay=%u\n", harness_replay_delivered);
	}
	if (harness_sndfile)
	{
		fprintf (harness_sndfile, "END %d %016" PRIx64 "\n", host_framecount, harness_sndchain);
		fclose (harness_sndfile);
		harness_sndfile = NULL;
	}
	if (harness_hashfile)
	{
		fprintf (harness_hashfile, "END %d %016" PRIx64 "\n", host_framecount, harness_hashchain);
		fclose (harness_hashfile);
		harness_hashfile = NULL;
	}
	if (harness_capturefile)
	{
		fclose (harness_capturefile);
		harness_capturefile = NULL;
	}
}

void Harness_NetCapture (int direction, int driver, int kind, const byte *data, int len)
{
	byte header[7];

	if (!harness_capturefile || len < 0)
		return;
	header[0] = (byte)direction;
	header[1] = (byte)driver;
	header[2] = (byte)kind;
	header[3] = (byte)(len & 0xff);
	header[4] = (byte)((len >> 8) & 0xff);
	header[5] = (byte)((len >> 16) & 0xff);
	header[6] = (byte)((len >> 24) & 0xff);
	fwrite (header, 1, sizeof (header), harness_capturefile);
	fwrite (data, 1, len, harness_capturefile);
}

struct qsocket_s *Harness_NetReplayConnect (void)
{
	qsocket_t *sock = NET_NewQSocket ();
	if (!sock)
		Sys_Error ("Harness: -netreplay could not allocate a qsocket");
	/* loop driver id: IS_LOOP_DRIVER skips timeouts and live counters, and
	   its Close tolerates a socket it never connected */
	sock->driver = 0;
	strcpy (sock->trueaddress, "netreplay");
	strcpy (sock->maskedaddress, "netreplay");
	harness_replaysock = sock;
	return sock;
}

qboolean Harness_NetReplayOwns (struct qsocket_s *sock)
{
	return harness_replaysock != NULL && sock == harness_replaysock;
}

/* delivers at most one captured recv record per host frame into
   net_message; returns the record's kind (1 reliable / 2 unreliable), or 0
   when none is due (send records and the server funnel's kind-0 records are
   skipped; EOF parks the connection with no traffic). */
int Harness_NetReplayGetMessage (void)
{
	byte hdr[7];

	if (host_framecount == harness_replay_lastframe)
		return 0;

	for (;;)
	{
		unsigned int len;
		int			 direction, kind;

		if (fread (hdr, 1, 7, harness_replayfile) != 7)
			return 0; /* EOF: idle out the rest of the session */
		direction = hdr[0];
		kind = hdr[2];
		len = hdr[3] | (hdr[4] << 8) | (hdr[5] << 16) | ((unsigned int)hdr[6] << 24);

		if (direction != 0 || kind == 0)
		{
			if (Sys_fseek (harness_replayfile, len, SEEK_CUR) != 0)
				return 0;
			continue;
		}
		if (len > (unsigned int)net_message.maxsize)
			Sys_Error ("Harness: -netreplay record larger than net_message (%u)", len);
		if (fread (net_message.data, 1, len, harness_replayfile) != len)
			Sys_Error ("Harness: -netreplay capture truncated mid-record");
		net_message.cursize = (int)len;
		harness_replay_lastframe = host_framecount;
		harness_replay_delivered++;
		return kind;
	}
}

static void Harness_Exit (int code)
{
	Harness_Shutdown ();
#ifdef PR_TRACE
	PR_TraceShutdown (); /* don't rely on exit() flushing the trace sink */
#endif
	Sys_Printf ("Harness: exiting at frame %d with code %d\n", host_framecount, code);
	if (code == 0)
		Sys_Quit ();
	exit (code);
}

void Harness_Frame (void)
{
	if (!harness_active)
		return;

	while (harness_nextcmd < harness_numcmds && harness_cmds[harness_nextcmd].frame <= host_framecount)
	{
		Cbuf_AddText (harness_cmds[harness_nextcmd].command);
		Cbuf_AddText ("\n");
		harness_nextcmd++;
	}

	if (harness_hashfile)
	{
		uint32_t rngstate[2];
		uint64_t h = harness_hashchain;

		h = Harness_HashServer (h);
		h = Harness_HashClient (h);
		COM_RandState (rngstate);
		h = Harness_Hash64 (h, rngstate, sizeof (rngstate));

		harness_hashchain = h;
		fprintf (harness_hashfile, "F %d %016" PRIx64 "\n", host_framecount, h);
	}

	if (harness_sndfile)
		fprintf (harness_sndfile, "S %d %016" PRIx64 "\n", host_framecount, harness_sndchain);

	if (harness_exitafter && host_framecount >= harness_exitafter)
		Harness_Exit (2);
}

void Harness_DemoEnded (void)
{
	if (!harness_active)
		return;
	Harness_Exit (0);
}
