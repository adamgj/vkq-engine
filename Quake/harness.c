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

// harness.c -- differential-verification harness (Rust migration Phase 0, ADR-019)

#include "quakedef.h"
#include "harness.h"

qboolean harness_active = false;
qboolean harness_fixed_dt = false;
qboolean no_rendering = false;

#define HARNESS_FRAME_DT   (1.0 / 72)
#define HARNESS_RAND_SEED  0x76715248 /* 'vqRH' */
#define HARNESS_MAX_CMDS   256
#define HARNESS_HASH_BASIS UINT64_C (0xcbf29ce484222325)
#define HARNESS_HASH_PRIME UINT64_C (0x100000001b3)

static FILE	   *harness_hashfile = NULL;
static FILE	   *harness_capturefile = NULL;
static uint64_t harness_hashchain = HARNESS_HASH_BASIS;
static int		harness_exitafter = 0;

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
		COM_CheckParm ("-netcapture"))
		harness_active = true;
	if (COM_CheckParm ("-demohash"))
		harness_fixed_dt = true;
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

	i = COM_CheckParm ("-netcapture");
	if (i && i < com_argc - 1)
	{
		harness_capturefile = Sys_fopen (com_argv[i + 1], "wb");
		if (!harness_capturefile)
			Sys_Error ("Harness: can't open -netcapture file %s", com_argv[i + 1]);
	}

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

void Harness_Shutdown (void)
{
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

	if (harness_exitafter && host_framecount >= harness_exitafter)
		Harness_Exit (2);
}

void Harness_DemoEnded (void)
{
	if (!harness_active)
		return;
	Harness_Exit (0);
}
