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
// pr_cmds_sv_fx_glue.c -- the C frame around the Rust world-effect builtins
// (Rust migration Phase 7 M5, Group E).
//
// Compiled under -Duse_rust_host, NOT -Duse_rust_progs (same reasoning as
// pr_cmds_sv_glue.c, which this file mirrors): the Rust module that calls
// these (rust/quake-capi/src/progs_builtins_sv_fx.rs) is gated on the `host`
// cargo feature, and the glue has to be compiled under exactly the same
// condition or the link breaks in the -Duse_rust_progs-only config.
//
// Every non-trivial builtin here is kept whole in C (ADR-009 rule 3), not
// split at the string/edict boundary, because `server_t sv` / `server_static_t
// svs` have no ADR-011 mirror in Phase 7 -- the same situation
// pr_cmds_sv_glue.c documents for `sv.lastcheck` and resolves the same way
// pr_cmds_sv_glue.c's PRBI_SvGlue_SetModelLookup does for `sv.model_precache`:
// the whole precache scan (and here, the whole ambientsound/lightstyle/
// makestatic/setspawnparms/localsound body) stays in one guarded C frame
// rather than being partially ported field-by-field.
//
// PF_particle (Quake/pr_cmds.c:614-625) needs no glue: SV_StartParticle
// (sv_main.c:1231) never raises and PF_particle uses no G_STRING/G_EDICTNUM,
// so quake_rs_pf_particle calls it directly.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

/* ---------------------------------------------------------------------------
 * PF_sound (pr_cmds.c:692-713).
 */

typedef struct
{
	void *entity;
	int	  channel;
	int	  sample_handle;
	int	  volume;
	float attenuation;
} prbi_fx_sound_arg_t;

/* G_STRING's PR_GetString can Host_Error on a cleared known-string;
   SV_StartSound Host_Errors on a bad volume/attenuation/channel or (via its
   internal NUM_FOR_EDICT) a bad entity. PR_RunWarning on an empty sample is
   not itself a raise, but it walks qcvm->statements (PR_PrintStatement) and
   the call stack (PR_StackTrace), which have no ADR-011 mirror, so it stays
   in this same C frame rather than being deferred through SvConsole. */
static void PRBI_FxInvokeSound (void *p)
{
	prbi_fx_sound_arg_t *a = (prbi_fx_sound_arg_t *)p;
	const char			*sample = PR_GetString (a->sample_handle);

	if (!*sample)
	{
		PR_RunWarning ("PF_sound: empty string\n");
		return;
	}
	SV_StartSound ((edict_t *)a->entity, NULL, a->channel, sample, a->volume, a->attenuation);
}

int PRBI_FxGlue_Sound (void *entity, int channel, int sample_handle, int volume, float attenuation)
{
	prbi_fx_sound_arg_t arg;

	arg.entity = entity;
	arg.channel = channel;
	arg.sample_handle = sample_handle;
	arg.volume = volume;
	arg.attenuation = attenuation;
	return Host_Guard (PRBI_FxInvokeSound, &arg);
}

/* ---------------------------------------------------------------------------
 * PF_sv_ambientsound (pr_cmds.c:633-675).
 */

typedef struct
{
	float *pos;
	int	   sample_handle;
	float  vol;
	float  attenuation;
} prbi_fx_ambientsound_arg_t;

/* sv.sound_precache has no ADR-011 mirror, so the precache scan and the "no
   precache" Con_Printf stay here; the sv.ambientsounds growth's
   Mem_Realloc failure PR_RunErrors. */
static void PRBI_FxInvokeAmbientSound (void *p)
{
	prbi_fx_ambientsound_arg_t *a = (prbi_fx_ambientsound_arg_t *)p;
	const char				   *samp = PR_GetString (a->sample_handle);
	const char				  **check;
	int							soundnum;
	struct ambientsound_s	   *st;

	for (soundnum = 0, check = sv.sound_precache; *check; check++, soundnum++)
	{
		if (!strcmp (*check, samp))
			break;
	}

	if (!*check)
	{
		Con_Printf ("no precache: %s\n", samp);
		return;
	}

	if (sv.num_ambients == sv.max_ambients)
	{
		int					   nm = sv.max_ambients + 128;
		struct ambientsound_s *n = (nm * sizeof (*n) < sv.max_ambients * sizeof (*n)) ? NULL : Mem_Realloc (sv.ambientsounds, nm * sizeof (*n));
		if (!n)
			PR_RunError ("PF_ambientsound: out of memory"); // shouldn't really happen.
		sv.ambientsounds = n;
		memset (sv.ambientsounds + sv.max_ambients, 0, (nm - sv.max_ambients) * sizeof (*n));
		sv.max_ambients = nm;
	}
	st = &sv.ambientsounds[sv.num_ambients++];
	VectorCopy (a->pos, st->origin);
	st->soundindex = soundnum;
	st->volume = a->vol;
	st->attenuation = a->attenuation;
}

int PRBI_FxGlue_AmbientSound (float *pos, int sample_handle, float vol, float attenuation)
{
	prbi_fx_ambientsound_arg_t arg;

	arg.pos = pos;
	arg.sample_handle = sample_handle;
	arg.vol = vol;
	arg.attenuation = attenuation;
	return Host_Guard (PRBI_FxInvokeAmbientSound, &arg);
}

/* ---------------------------------------------------------------------------
 * PF_sv_lightstyle (pr_cmds.c:1364-1405).
 */

typedef struct
{
	int style;
	int val_handle;
} prbi_fx_lightstyle_arg_t;

/* sv.lightstyles and svs.clients have no ADR-011 mirror, so the bounds
   check, the array write and the per-client broadcast loop stay here.
   Never actually raises on its own; guarded anyway because the G_STRING
   fetch can. */
static void PRBI_FxInvokeLightStyle (void *p)
{
	prbi_fx_lightstyle_arg_t *a = (prbi_fx_lightstyle_arg_t *)p;
	const char				 *val = PR_GetString (a->val_handle);
	client_t				 *client;
	int						  j;

	// bounds check to avoid clobbering sv struct
	if (a->style < 0 || a->style >= MAX_LIGHTSTYLES)
	{
		Con_DWarning ("PF_lightstyle: invalid style %d\n", a->style);
		return;
	}

	// change the string in sv
	sv.lightstyles[a->style] = val;

	// send message to all clients on this server
	if (sv.state != ss_active)
		return;

	for (j = 0, client = svs.clients; j < svs.maxclients; j++, client++)
	{
		if (client->active || client->spawned)
		{
			if (a->style > 0xff)
			{
				MSG_WriteByte (&client->message, svc_stufftext);
				MSG_WriteString (&client->message, va ("//ls %i \"%s\"\n", a->style, val));
			}
			else
			{
				MSG_WriteChar (&client->message, svc_lightstyle);
				MSG_WriteChar (&client->message, a->style);
				MSG_WriteString (&client->message, val);
			}
		}
	}
}

int PRBI_FxGlue_LightStyle (int style, int val_handle)
{
	prbi_fx_lightstyle_arg_t arg;

	arg.style = style;
	arg.val_handle = val_handle;
	return Host_Guard (PRBI_FxInvokeLightStyle, &arg);
}

/* ---------------------------------------------------------------------------
 * PF_sv_makestatic (pr_cmds.c:1708-1734).
 */

/* sv.static_entities has no ADR-011 mirror. The sv.static_entities growth's
   Mem_Realloc failure and ED_Free can both Host_Error. */
static void PRBI_FxInvokeMakeStatic (void *p)
{
	edict_t		   *ent = (edict_t *)p;
	entity_state_t *st;

	if (sv.num_statics == sv.max_statics)
	{
		int				nm = sv.max_statics + 128;
		entity_state_t *n = (nm * sizeof (*n) < sv.max_statics * sizeof (*n)) ? NULL : Mem_Realloc (sv.static_entities, nm * sizeof (*n));
		if (!n)
			PR_RunError ("PF_makestatic: out of memory"); // shouldn't really happen.
		sv.static_entities = n;
		memset (sv.static_entities + sv.max_statics, 0, (nm - sv.max_statics) * sizeof (*n));
		sv.max_statics = nm;
	}
	st = &sv.static_entities[sv.num_statics];
	SV_BuildEntityState (ent, st);
	if (st->alpha == ENTALPHA_ZERO)
		; // no point
	else
		sv.num_statics++;

	// throw the entity away now
	ED_Free (ent);
}

int PRBI_FxGlue_MakeStatic (void *ent)
{
	return Host_Guard (PRBI_FxInvokeMakeStatic, ent);
}

/* ---------------------------------------------------------------------------
 * PF_sv_setspawnparms (pr_cmds.c:1743-1759).
 */

/* NUM_FOR_EDICT Host_Errors on a bad edict; svs.clients / pr_global_struct
   have no ADR-011 mirror, so the copy loop stays here too. */
static void PRBI_FxInvokeSetSpawnParms (void *p)
{
	edict_t	 *ent = (edict_t *)p;
	int		  i = NUM_FOR_EDICT (ent);
	client_t *client;

	if (i < 1 || i > svs.maxclients)
		PR_RunError ("Entity is not a client");

	// copy spawn parms out of the client_t
	client = svs.clients + (i - 1);

	for (i = 0; i < NUM_BASIC_SPAWN_PARMS; i++)
		(&pr_global_struct->parm1)[i] = client->spawn_parms[i];
}

int PRBI_FxGlue_SetSpawnParms (void *ent)
{
	return Host_Guard (PRBI_FxInvokeSetSpawnParms, ent);
}

/* ---------------------------------------------------------------------------
 * PF_sv_changelevel (pr_cmds.c:1766-1777).
 */

/* Only the G_STRING fetch and the Cbuf_AddText half: PR_GetString can
   Host_Error on a cleared known-string. The svs.changelevel_issued
   check-and-set is done in Rust via the existing
   PRBI_Glue_ChangelevelIssued, which cannot raise. */
static void PRBI_FxInvokeChangeLevel (void *p)
{
	int		   *handle = (int *)p;
	const char *s = PR_GetString (*handle);

	Cbuf_AddText (va ("changelevel %s\n", s));
}

int PRBI_FxGlue_ChangeLevel (int level_handle)
{
	return Host_Guard (PRBI_FxInvokeChangeLevel, &level_handle);
}

/* ---------------------------------------------------------------------------
 * PF_sv_precache_sound (pr_cmds.c:1188-1198).
 */

/* PR_CheckEmptyString is `static` in pr_cmds.c (pr_cmds.c:1148-1152), so its
   "Bad string" PR_RunError is duplicated here rather than exposed;
   SV_Precache_Sound's "overflow" case also PR_RunErrors. */
static void PRBI_FxInvokePrecacheSound (void *p)
{
	int		   *handle = (int *)p;
	const char *s = PR_GetString (*handle);

	if (s[0] <= ' ')
		PR_RunError ("Bad string");
	if (!SV_Precache_Sound (s))
		PR_RunError ("PF_precache_sound: overflow");
}

int PRBI_FxGlue_PrecacheSound (int handle)
{
	return Host_Guard (PRBI_FxInvokePrecacheSound, &handle);
}

/* ---------------------------------------------------------------------------
 * PF_sv_precache_model (pr_cmds.c:1225-1259).
 */

/* Kept whole in C with its own scan rather than calling SV_Precache_Model:
   PF_precache_model warns unconditionally on the "not yet precached" path
   and conditionally (only when !pr_checkextension.value) on the "already
   precached" path, which SV_Precache_Model does not reproduce. */
static void PRBI_FxInvokePrecacheModel (void *p)
{
	int		   *handle = (int *)p;
	const char *s = PR_GetString (*handle);
	int			i;

	if (s[0] <= ' ')
		PR_RunError ("Bad string");

	for (i = 0; i < MAX_MODELS; i++)
	{
		if (!sv.model_precache[i])
		{
			if (sv.state != ss_loading)
			{
				Con_Warning ("PF_precache_model(\"%s\"): Precache should only be done in spawn functions\n", s);
				// let existing clients know about it
				MSG_WriteByte (&sv.reliable_datagram, svcdp_precache);
				MSG_WriteShort (&sv.reliable_datagram, i | 0x8000);
				MSG_WriteString (&sv.reliable_datagram, s);
			}

			sv.model_precache[i] = s;
			sv.models[i] = Mod_ForName (s, i == 1);
			return;
		}
		if (!strcmp (sv.model_precache[i], s))
		{
			if (sv.state != ss_loading && !pr_checkextension.value)
				Con_Warning ("PF_precache_model(\"%s\"): Precache should only be done in spawn functions\n", s);
			return;
		}
	}
	PR_RunError ("PF_precache_model: overflow");
}

int PRBI_FxGlue_PrecacheModel (int handle)
{
	return Host_Guard (PRBI_FxInvokePrecacheModel, &handle);
}

/* ---------------------------------------------------------------------------
 * PF_sv_localsound (pr_cmds.c:1857-1870).
 */

typedef struct
{
	void *ent;
	int	  sample_handle;
} prbi_fx_localsound_arg_t;

/* NUM_FOR_EDICT Host_Errors on a bad edict; svs.clients has no ADR-011
   mirror, so the range check and SV_LocalSound call stay here too. */
static void PRBI_FxInvokeLocalSound (void *p)
{
	prbi_fx_localsound_arg_t *a = (prbi_fx_localsound_arg_t *)p;
	int						  entnum = NUM_FOR_EDICT ((edict_t *)a->ent);
	const char				 *sample = PR_GetString (a->sample_handle);

	if (entnum < 1 || entnum > svs.maxclients)
	{
		Con_Printf ("tried to localsound to a non-client\n");
		return;
	}
	SV_LocalSound (&svs.clients[entnum - 1], sample);
}

int PRBI_FxGlue_LocalSound (void *ent, int sample_handle)
{
	prbi_fx_localsound_arg_t arg;

	arg.ent = ent;
	arg.sample_handle = sample_handle;
	return Host_Guard (PRBI_FxInvokeLocalSound, &arg);
}

#endif /* USE_RUST_HOST */
