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
// sv_phys_glue.c -- the C frame around the Rust server physics port.
//
// Compiled instead of sv_phys.c under -Duse_rust_host (Rust migration Phase 7
// M4). Four jobs, mirroring world_glue.c:
//
//  1. Own the C-visible objects sv_phys.c defined: the twelve physics cvars
//     (sv_phys.c:44-56, :705), the sv_analyticphysics_frame latch (:707) and
//     the sv_speeds_* counters (:345-346). sv_main.c registers the cvars and
//     host.c reads the counters, so the storage stays here and Rust reaches
//     them through externs.
//  2. Guard everything sv_phys.c reached that can Host_Error / Host_EndGame
//     (ADR-009 rule 3): every PR_ExecuteProgram dispatch, the two NaN warnings
//     (PR_GetString), the two "bad movetype" Host_EndGame sites, SV_StartSound
//     and the un-embed Con_DPrintf2 (its NUM_FOR_EDICT arguments raise).
//  3. Re-raise, from a pure C frame, what those guards caught. The four
//     sv_phys.c entry points whose bodies reach Host_Error are thin wrappers
//     over quake_rs_* status cores: SV_CheckAllEnts, SV_CheckVelocity,
//     SV_CheckWaterTransition and SV_Physics. Host_Reraise is called only
//     from here.
//  4. Keep the sv / svs / sv_player reads in C. These accessors predate M6,
//     which moved sv/svs storage into Rust and gave server_t and
//     server_static_t ADR-011 mirrors; reaching those mirrors directly from
//     sv_phys is M8's business, so the accessors stay for now.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

/* ---------------------------------------------------------------------------
 * C-visible cvar objects and globals (sv_phys.c:44-56, :345-346, :705-707).
 */

cvar_t sv_friction = {"sv_friction", "4", CVAR_NOTIFY | CVAR_SERVERINFO};
cvar_t sv_stopspeed = {"sv_stopspeed", "100", CVAR_NONE};
cvar_t sv_gravity = {"sv_gravity", "800", CVAR_NOTIFY | CVAR_SERVERINFO};
cvar_t sv_maxvelocity = {"sv_maxvelocity", "2000", CVAR_NONE};
cvar_t sv_nostep = {"sv_nostep", "0", CVAR_NONE};
cvar_t sv_freezenonclients = {"sv_freezenonclients", "0", CVAR_NONE};
cvar_t sv_gameplayfix_spawnbeforethinks = {"sv_gameplayfix_spawnbeforethinks", "0", CVAR_NONE};
cvar_t sv_gameplayfix_bouncedownslopes = {"sv_gameplayfix_bouncedownslopes", "1", CVAR_NONE}; // fixes grenades making horrible noises on slopes.
cvar_t sv_fastpushmove = {"sv_fastpushmove", "1", CVAR_NONE};								  // 0=old SV_PushMove processing; 1= faster SV_PushMove, (default)
cvar_t sv_pushgrid = {"sv_pushgrid", "1", CVAR_NONE};				// cull SV_PushMove candidates with a spatial hash, needs sv_fastpushmove
cvar_t sv_analyticphysics = {"sv_analyticphysics", "1", CVAR_NONE}; // gravity/friction integration matches 72Hz physics at any tick rate

// 0=off; 1=legacy DIST_EPSILON nudge, clients only; 2=legacy nudge, all entities; 3=robust pusher contact (default)
cvar_t sv_gameplayfix_elevators = {"sv_gameplayfix_elevators", "3", CVAR_NONE};

qboolean sv_analyticphysics_frame = true; // sv_analyticphysics latched per SV_Physics, QC can flip the cvar mid-tick

double sv_speeds_think_ms, sv_speeds_pusher_ms, sv_speeds_build_ms;
int	   sv_speeds_thinks, sv_speeds_pushers, sv_speeds_pushables, sv_speeds_grid_entries;

/* ---------------------------------------------------------------------------
 * Guarded callbacks (ADR-009 rule 3).
 */

/* sv_phys.c:318 and :323. PR_GetString reaches Host_Error on a corrupt
   string_t (pr_edict_arena.c:315), so both warnings run under a guard. */
static void SvPhys_InvokeWarnNanVelocity (void *p)
{
	edict_t *ent = (edict_t *)p;
	Con_DPrintf ("Got a NaN velocity on %s\n", PR_GetString (ent->v.classname));
}

int SvPhys_Glue_WarnNanVelocity (edict_t *ent)
{
	return Host_Guard (SvPhys_InvokeWarnNanVelocity, ent);
}

static void SvPhys_InvokeWarnNanOrigin (void *p)
{
	edict_t *ent = (edict_t *)p;
	Con_DPrintf ("Got a NaN origin on %s\n", PR_GetString (ent->v.classname));
}

int SvPhys_Glue_WarnNanOrigin (edict_t *ent)
{
	return Host_Guard (SvPhys_InvokeWarnNanOrigin, ent);
}

typedef struct
{
	edict_t *ent;
	float	 time;
} svphys_think_arg_t;

/* Both think dispatches: sv_phys.c:369-372 (SV_RunThink, time = the clamped
   think time) and sv_phys.c:1610-1613 (SV_Physics_Pusher, time = qcvm->time).
   They differ only in the stamped time, so the caller supplies it. */
static void SvPhys_InvokeCallThink (void *p)
{
	svphys_think_arg_t *a = (svphys_think_arg_t *)p;

	pr_global_struct->time = a->time;
	pr_global_struct->self = EDICT_TO_PROG (a->ent);
	pr_global_struct->other = EDICT_TO_PROG (qcvm->edicts);
	PR_ExecuteProgram (a->ent->v.think);
}

int SvPhys_Glue_CallThink (edict_t *ent, float time)
{
	svphys_think_arg_t arg;

	arg.ent = ent;
	arg.time = time;
	return Host_Guard (SvPhys_InvokeCallThink, &arg);
}

typedef struct
{
	edict_t *self;
	edict_t *other;
} svphys_pair_arg_t;

/* sv_phys.c:424-426 and :434-436 -- both SV_Impact dispatches. SV_Impact sets
   pr_global_struct->time once, before the first dispatch, so it is
   deliberately not touched here: QC may change it between the two calls. */
static void SvPhys_InvokeImpactTouch (void *p)
{
	svphys_pair_arg_t *a = (svphys_pair_arg_t *)p;

	pr_global_struct->self = EDICT_TO_PROG (a->self);
	pr_global_struct->other = EDICT_TO_PROG (a->other);
	PR_ExecuteProgram (a->self->v.touch);
}

int SvPhys_Glue_ImpactTouch (edict_t *self, edict_t *other)
{
	svphys_pair_arg_t arg;

	arg.self = self;
	arg.other = other;
	return Host_Guard (SvPhys_InvokeImpactTouch, &arg);
}

/* sv_phys.c:1254 -- NUM_FOR_EDICT Host_Errors on a bad pointer
   (pr_edict.c:1082), so the whole line runs under a guard, not just the
   print. */
static void SvPhys_InvokeDPrintUnembedded (void *p)
{
	svphys_pair_arg_t *a = (svphys_pair_arg_t *)p;
	Con_DPrintf2 ("SV_PushEntityTo: un-embedded entity %i from pusher %i\n", NUM_FOR_EDICT (a->self), NUM_FOR_EDICT (a->other));
}

int SvPhys_Glue_DPrintUnembedded (edict_t *ent, edict_t *ground)
{
	svphys_pair_arg_t arg;

	arg.self = ent;
	arg.other = ground;
	return Host_Guard (SvPhys_InvokeDPrintUnembedded, &arg);
}

/* sv_phys.c:1559-1561 -- SV_PushMove's blocked dispatch. */
static void SvPhys_InvokeCallBlocked (void *p)
{
	svphys_pair_arg_t *a = (svphys_pair_arg_t *)p;

	pr_global_struct->self = EDICT_TO_PROG (a->self);
	pr_global_struct->other = EDICT_TO_PROG (a->other);
	PR_ExecuteProgram (a->self->v.blocked);
}

int SvPhys_Glue_CallBlocked (edict_t *pusher, edict_t *obstacle)
{
	svphys_pair_arg_t arg;

	arg.self = pusher;
	arg.other = obstacle;
	return Host_Guard (SvPhys_InvokeCallBlocked, &arg);
}

/* sv_phys.c:2007-2009. Neither player think sets ->other. */
static void SvPhys_InvokeCallPlayerPreThink (void *p)
{
	svphys_think_arg_t *a = (svphys_think_arg_t *)p;

	pr_global_struct->time = a->time;
	pr_global_struct->self = EDICT_TO_PROG (a->ent);
	PR_ExecuteProgram (pr_global_struct->PlayerPreThink);
}

int SvPhys_Glue_CallPlayerPreThink (edict_t *ent, float time)
{
	svphys_think_arg_t arg;

	arg.ent = ent;
	arg.time = time;
	return Host_Guard (SvPhys_InvokeCallPlayerPreThink, &arg);
}

/* sv_phys.c:2065-2067 */
static void SvPhys_InvokeCallPlayerPostThink (void *p)
{
	svphys_think_arg_t *a = (svphys_think_arg_t *)p;

	pr_global_struct->time = a->time;
	pr_global_struct->self = EDICT_TO_PROG (a->ent);
	PR_ExecuteProgram (pr_global_struct->PlayerPostThink);
}

int SvPhys_Glue_CallPlayerPostThink (edict_t *ent, float time)
{
	svphys_think_arg_t arg;

	arg.ent = ent;
	arg.time = time;
	return Host_Guard (SvPhys_InvokeCallPlayerPostThink, &arg);
}

/* sv_phys.c:2334-2339 -- the StartFrame dispatch. The pr_global_struct
   ->StartFrame test itself stays on the Rust side; only the dispatch is
   guarded. */
static void SvPhys_InvokeCallStartFrame (void *p)
{
	pr_global_struct->self = EDICT_TO_PROG (qcvm->edicts);
	pr_global_struct->other = EDICT_TO_PROG (qcvm->edicts);
	pr_global_struct->time = *(float *)p;
	PR_ExecuteProgram (pr_global_struct->StartFrame);
}

int SvPhys_Glue_CallStartFrame (float time)
{
	return Host_Guard (SvPhys_InvokeCallStartFrame, &time);
}

typedef struct
{
	edict_t	   *ent;
	int			channel;
	const char *sample;
	int			volume;
	float		attenuation;
} svphys_sound_arg_t;

/* sv_phys.c:2139, :2148 (SV_CheckWaterTransition) and :2270
   (SV_Physics_Toss). SV_StartSound Host_Errors three ways (sv_main.c:285,
   :293, :296 -- renumbered by T6.1's split). All three call sites pass
   origin NULL. */
static void SvPhys_InvokeStartSound (void *p)
{
	svphys_sound_arg_t *a = (svphys_sound_arg_t *)p;
	SV_StartSound (a->ent, NULL, a->channel, a->sample, a->volume, a->attenuation);
}

int SvPhys_Glue_StartSound (edict_t *ent, int channel, const char *sample, int volume, float attenuation)
{
	svphys_sound_arg_t arg;

	arg.ent = ent;
	arg.channel = channel;
	arg.sample = sample;
	arg.volume = volume;
	arg.attenuation = attenuation;
	return Host_Guard (SvPhys_InvokeStartSound, &arg);
}

/* sv_phys.c:2055 and :2429. Host_EndGame is FUNC_NORETURN and longjmps, so
   both messages stay in C; they are separate helpers because the format
   strings differ. */
static void SvPhys_InvokeEndGameBadClientMovetype (void *p)
{
	Host_EndGame ("SV_Physics_client: bad movetype %i", *(int *)p);
}

int SvPhys_Glue_EndGameBadClientMovetype (int movetype)
{
	return Host_Guard (SvPhys_InvokeEndGameBadClientMovetype, &movetype);
}

static void SvPhys_InvokeEndGameBadMovetype (void *p)
{
	Host_EndGame ("SV_Physics: bad movetype %i", *(int *)p);
}

int SvPhys_Glue_EndGameBadMovetype (int movetype)
{
	return Host_Guard (SvPhys_InvokeEndGameBadMovetype, &movetype);
}

/* ---------------------------------------------------------------------------
 * Re-raising public entry points. Each is the exact sv_phys.c signature; the
 * Rust body is a quake_rs_* status core and the jump is re-issued from here,
 * never from a Rust frame (ADR-009).
 */

/* sv_phys.c:283 */
void SV_CheckAllEnts (void)
{
	int r = quake_rs_sv_check_all_ents ();
	Host_Reraise (r);
}

/* sv_phys.c:307 */
void SV_CheckVelocity (edict_t *ent)
{
	int r = quake_rs_sv_check_velocity (ent);
	Host_Reraise (r);
}

/* sv_phys.c:2122 */
void SV_CheckWaterTransition (edict_t *ent)
{
	int r = quake_rs_sv_check_water_transition (ent);
	Host_Reraise (r);
}

/* sv_phys.c:2298 */
void SV_Physics (void)
{
	int r = quake_rs_sv_physics ();
	Host_Reraise (r);
}

/* ---------------------------------------------------------------------------
 * Thin, non-raising shims.
 */

/* sv_phys.c:298 */
void SvPhys_Glue_PrintInvalidPosition (void)
{
	Con_Printf ("entity in invalid position\n");
}

/* sv_phys.c:1655 and :1669 */
void SvPhys_Glue_DPrintUnstuck (void)
{
	Con_DPrintf ("Unstuck.\n");
}

/* sv_phys.c:1676 */
void SvPhys_Glue_DPrintPlayerStuck (void)
{
	Con_DPrintf ("player is stuck.\n");
}

/* The `qcvm == &sv.qcvm` test sv_phys.c makes in a dozen places (:357, :740,
   :852, :915, :984, :1029, :1598, :2313, :2349, :2363, :2390, :2414, :2453).
   server_t has no ADR-011 mirror in Phase 7. */
int SvPhys_Glue_QcvmIsServer (void)
{
	return qcvm == &sv.qcvm;
}

/* sv_phys.c:1541, :2350, :2414 */
int SvPhys_Glue_MaxClients (void)
{
	return svs.maxclients;
}

/* sv_phys.c:1996 -- `num` is the entity number, so the client slot is
   num - 1. */
int SvPhys_Glue_ClientActive (int num)
{
	return svs.clients[num - 1].active;
}

/* sv_phys.c:1999 */
int SvPhys_Glue_ClientKnownToQc (int num)
{
	return svs.clients[num - 1].knowntoqc;
}

/* sv_phys.c:1893 -- SV_WalkMove reads the waterjump flag off the client the
   host is currently running, not off `ent`. */
edict_t *SvPhys_Glue_SvPlayer (void)
{
	return sv_player;
}

#endif /* USE_RUST_HOST */
