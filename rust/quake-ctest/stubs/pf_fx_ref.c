/* Phase 7 M5 wave 2 oracle TU: Group E, world-effect builtins (pr_cmds_sv_fx_glue.c).
 *
 * STATUS: 7 of 12 builtins have a real C oracle transcription + fixture:
 * PF_particle, PF_sound, PF_sv_precache_sound, PF_sv_precache_model,
 * PF_sv_finalefinished, PF_sv_CheckPlayerEXFlags, PF_sv_changelevel.
 *
 * The other 5 (PF_sv_ambientsound, PF_sv_lightstyle, PF_sv_makestatic,
 * PF_sv_setspawnparms, PF_sv_localsound) are left as abort stubs. Every one
 * of them needs a `svs`/`client_t`-equivalent mock: `sv.ambientsounds`,
 * `sv.lightstyles`+`svs.clients` broadcast, `sv.static_entities`+
 * `SV_BuildEntityState`, `svs.clients[i].spawn_parms`, or
 * `svs.clients`+`SV_LocalSound`. None of `svs`, `client_t` or
 * `server_static_t` exist anywhere in this ctest fixture (confirmed by grep
 * across stubs.c and c_ref_prelude.h: zero matches), and `SV_BuildEntityState`/
 * `Mod_ForName`/`SV_LocalSound` are only defined in sv_main.c/gl_model.c,
 * neither of which is in build.rs's C_SOURCES. Hand-building that mock is
 * disproportionate to the remaining wave-2 budget, so these 5 stay
 * ABORT stubs -- deliberately, not silently: see the header comment this
 * replaces (wave 1 shipped two "quiet default" oracle bugs and an M4 amendment
 * records a third, so a stub here still aborts loudly rather than returning a
 * plausible-looking value).
 *
 * HAZARD (wave 1): stubs.c #undefs several c_ref_prelude.h rename macros
 * (SV_Move, SV_LinkEdict, SV_HullForEntity, SV_ClipMoveToEntity,
 * SV_TestEntityPosition, SV_CheckBottom, SV_movestep). None of those are
 * touched here, but the same discipline applies: spell oracle call sites as
 * explicit c_ref_* names, and declare an explicit c_ref_* prototype for
 * anything whose header is not included, or MSVC invents an int-returning
 * one and silently corrupts qboolean results. PRBI_Glue_ChangelevelIssued's
 * qboolean (== bool) return is spelled explicitly for exactly this reason.
 *
 * DESIGN: for each of the 7 implemented builtins there are, deliberately,
 * TWO independent C transcriptions sharing one resettable fixture:
 *   - `PRBI_FxGlue_*` / `PRBI_Glue_ChangelevelIssued` -- what the REAL Rust
 *     module (rust/quake-capi/src/progs_builtins_sv_fx.rs, unmodified here)
 *     actually links against and calls. Faithful reimplementations of
 *     Quake/pr_cmds_sv_fx_glue.c's bodies (that file is not compiled into
 *     ctest), operating on this TU's private fixture state.
 *   - `ctest_fx_oracle_pf_*` -- independent, from-scratch transcriptions of
 *     the ORIGINAL, unsplit Quake/pr_cmds.c bodies (before the Rust port
 *     existed), reading globals directly via G_STRING/G_EDICT/etc, called
 *     only by this file's own dispatcher (ctest_fx_pf_run), never by the
 *     glue functions above.
 * A differential test resets the shared fixture, runs one side, inspects
 * outcome + fixture state, resets again, runs the other side, and compares.
 * Because the fixture is fully reset between the two runs of one comparison,
 * sharing one array/counter set for both transcriptions is safe (matches the
 * existing SV_StartSound/SV_Precache_Sound doubles below in stubs.c, which
 * are also single shared recorders, reset before use, not per-side
 * duplicated) -- it is not circular: neither transcription calls the other,
 * both are independent readings of the same production intent, and what's
 * being tested is exactly "does Rust's arg-extraction/write-order/split
 * preserve the original monolithic body's observable behaviour".
 *
 * KNOWN GAPS, documented rather than silently skipped:
 *   - SV_Precache_Sound's ctest double (stubs.c) always returns 1 (never
 *     "full"), so PF_sv_precache_sound's overflow raise is NOT reachable
 *     through it. PF_sv_precache_model's overflow IS reachable (this file's
 *     own private, small model-precache mock can be filled deliberately).
 *   - PR_GetString's Host_Error-raising branch (a cleared known string,
 *     negative offset) is not exercised: this fixture only ever interns
 *     ordinary positive-offset strings (see ctest_fx_intern below), it never
 *     populates qcvm->knownstrings. Every string handle this file hands to
 *     PR_GetString is always valid.
 *   - PF_sv_precache_model's/PF_sv_precache_sound's/PF_sv_precache_model's
 *     `sv.model_precache[i] = s; sv.models[i] = Mod_ForName(s, i==1);` and
 *     `MSG_WriteByte/Short/String(&sv.reliable_datagram, ...)` "notify
 *     existing clients" side effects have no fixture equivalent (no
 *     sv.models/sv.reliable_datagram mirror) and are intentionally omitted;
 *     only the control-flow-relevant half (does it raise, is the slot
 *     filled, does Con_Warning fire) is preserved.
 */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>

static void m5_wave2_unimplemented (const char *fn)
{
	fprintf (stderr, "quake-ctest: %s has no M5 wave-2 oracle yet\n", fn);
	abort ();
}

/* ---------------------------------------------------------------------------
 * Fixture primitives owned by stubs.c with no c_ref_prelude.h prototype
 * (only Host_Error is declared there) -- explicit local prototypes per this
 * file's own documented convention above.
 */
void		ctest_world_reset (int vm_kind, int num_edicts);
int			ctest_try_host (void (*fn) (void *), void *arg);
const char *ctest_host_error_message (void);
int			Host_Guard (void (*fn) (void *), void *arg);
void		ctest_clear_con_log (void);
int			ctest_con_log_len (void);
const char *ctest_con_log_get (int i);

/* SV_StartSound's real, reusable test double + recorder (stubs.c). */
void ctest_phys_sound_arm_raise (int on);
int	 ctest_phys_sound_len (void);
int	 ctest_phys_sound_get (int i, int *ent, int *channel, int *volume, float *attenuation, int *has_origin, const char **sample);
void ctest_phys_sound_clear (void);

/* SV_Precache_Sound's real, reusable test double + recorder (stubs.c). Always
 * succeeds (see KNOWN GAPS above). */
int			ctest_predd_get_particle_calls (void);
const char *ctest_predd_get_last_sound (void);
void		ctest_predd_reset_doubles (void);
int			SV_Precache_Sound (const char *s);

/* ---------------------------------------------------------------------------
 * Shared fixture: string pool, global read/write, entity handle helper.
 */

#define CTEST_FX_STRINGS_CAP 4096
static char ctest_fx_strings[CTEST_FX_STRINGS_CAP];
static int	ctest_fx_strings_len;

static void ctest_fx_strings_reset (void)
{
	memset (ctest_fx_strings, 0, sizeof (ctest_fx_strings));
	ctest_fx_strings_len = 1; /* offset 0 reserved as "" (matches PR_GetString(0) == "") */
}

int ctest_fx_intern (const char *s)
{
	size_t len = strlen (s);
	int	   ofs;

	if (ctest_fx_strings_len + (int)len + 1 > CTEST_FX_STRINGS_CAP)
	{
		fprintf (stderr, "ctest_fx_intern: string pool exhausted\n");
		abort ();
	}
	ofs = ctest_fx_strings_len;
	memcpy (ctest_fx_strings + ofs, s, len + 1);
	ctest_fx_strings_len += (int)len + 1;
	return ofs;
}

void ctest_fx_set_global_float (int ofs, float v)
{
	qcvm->globals[ofs] = v;
}

void ctest_fx_set_global_int (int ofs, int v)
{
	*(int *)&qcvm->globals[ofs] = v;
}

void ctest_fx_set_global_vector (int ofs, float x, float y, float z)
{
	qcvm->globals[ofs] = x;
	qcvm->globals[ofs + 1] = y;
	qcvm->globals[ofs + 2] = z;
}

float ctest_fx_get_global_float (int ofs)
{
	return qcvm->globals[ofs];
}

int ctest_fx_get_global_int (int ofs)
{
	return *(int *)&qcvm->globals[ofs];
}

/* Value to write into a G_EDICT-read global to reference edict `num`. */
int ctest_fx_edict_to_prog (int num)
{
	return EDICT_TO_PROG (EDICT_NUM (num));
}

/* ---------------------------------------------------------------------------
 * PF_particle (pr_cmds.c:614-625). No glue: SV_StartParticle never raises
 * and PF_particle uses no G_STRING/G_EDICTNUM, so quake_rs_pf_particle calls
 * it directly (pr_cmds_sv_fx_glue.c's header comment). Both this oracle and
 * the Rust builtin call the SAME recorder below -- not circular, since the
 * recorder only captures call arguments, it makes no decision the test could
 * be fooled by.
 */

typedef struct
{
	float org[3];
	float dir[3];
	int	  color;
	int	  count;
} ctest_fx_particle_rec_t;

static ctest_fx_particle_rec_t ctest_fx_particle_last;
static int					   ctest_fx_particle_call_count;

/* Real signature per sv_main.c: color/count are truncated to int at the call
 * site by every caller (PF_particle passes floats through an (int,int)
 * prototype) -- this is an existing, unrelated-to-Rust compat truncation,
 * not something this file introduces. */
void SV_StartParticle (float *org, float *dir, int color, int count)
{
	ctest_fx_particle_last.org[0] = org[0];
	ctest_fx_particle_last.org[1] = org[1];
	ctest_fx_particle_last.org[2] = org[2];
	ctest_fx_particle_last.dir[0] = dir[0];
	ctest_fx_particle_last.dir[1] = dir[1];
	ctest_fx_particle_last.dir[2] = dir[2];
	ctest_fx_particle_last.color = color;
	ctest_fx_particle_last.count = count;
	ctest_fx_particle_call_count++;
}

void ctest_fx_particle_clear (void)
{
	memset (&ctest_fx_particle_last, 0, sizeof (ctest_fx_particle_last));
	ctest_fx_particle_call_count = 0;
}

int ctest_fx_particle_calls (void)
{
	return ctest_fx_particle_call_count;
}

void ctest_fx_particle_get (float *org, float *dir, int *color, int *count)
{
	org[0] = ctest_fx_particle_last.org[0];
	org[1] = ctest_fx_particle_last.org[1];
	org[2] = ctest_fx_particle_last.org[2];
	dir[0] = ctest_fx_particle_last.dir[0];
	dir[1] = ctest_fx_particle_last.dir[1];
	dir[2] = ctest_fx_particle_last.dir[2];
	*color = ctest_fx_particle_last.color;
	*count = ctest_fx_particle_last.count;
}

static void ctest_fx_oracle_pf_particle (void)
{
	float *org, *dir;
	float  color;
	float  count;

	org = G_VECTOR (OFS_PARM0);
	dir = G_VECTOR (OFS_PARM1);
	color = G_FLOAT (OFS_PARM2);
	count = G_FLOAT (OFS_PARM3);
	SV_StartParticle (org, dir, color, count);
}

/* ---------------------------------------------------------------------------
 * PF_sound (pr_cmds.c:692-713 / pr_cmds_sv_fx_glue.c PRBI_FxGlue_Sound).
 */

typedef struct
{
	void *entity;
	int	  channel;
	int	  sample_handle;
	int	  volume;
	float attenuation;
} ctest_fx_sound_arg_t;

static void ctest_fx_invoke_sound (void *p)
{
	ctest_fx_sound_arg_t *a = (ctest_fx_sound_arg_t *)p;
	const char			 *sample = PR_GetString (a->sample_handle);

	if (!*sample)
	{
		PR_RunWarning ("PF_sound: empty string\n");
		return;
	}
	SV_StartSound ((edict_t *)a->entity, NULL, a->channel, sample, a->volume, a->attenuation);
}

int PRBI_FxGlue_Sound (void *entity, int channel, int sample_handle, int volume, float attenuation)
{
	ctest_fx_sound_arg_t arg;

	arg.entity = entity;
	arg.channel = channel;
	arg.sample_handle = sample_handle;
	arg.volume = volume;
	arg.attenuation = attenuation;
	return Host_Guard (ctest_fx_invoke_sound, &arg);
}

/* Independent oracle: reads globals directly, mirrors PF_sound's own body
 * rather than calling PRBI_FxGlue_Sound above. */
static void ctest_fx_oracle_pf_sound (void)
{
	const char *sample;
	int			channel;
	edict_t	   *entity;
	int			volume;
	float		attenuation;

	entity = G_EDICT (OFS_PARM0);
	channel = G_FLOAT (OFS_PARM1);
	sample = G_STRING (OFS_PARM2);
	volume = G_FLOAT (OFS_PARM3) * 255;
	attenuation = G_FLOAT (OFS_PARM4);

	if (!*sample)
	{
		PR_RunWarning ("PF_sound: empty string\n");
		return;
	}

	SV_StartSound (entity, NULL, channel, sample, volume, attenuation);
}

/* ---------------------------------------------------------------------------
 * PF_sv_precache_sound (pr_cmds.c:1185-1198 / PRBI_FxGlue_PrecacheSound).
 * "Bad string" is PR_CheckEmptyString's raise, duplicated here since that
 * function is `static` in pr_cmds.c. SV_Precache_Sound's "overflow" raise is
 * NOT reachable through this fixture's double (see KNOWN GAPS above).
 */

static void ctest_fx_invoke_precache_sound (void *p)
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
	return Host_Guard (ctest_fx_invoke_precache_sound, &handle);
}

static void ctest_fx_oracle_pf_precache_sound (void)
{
	const char *s;

	s = G_STRING (OFS_PARM0);
	G_INT (OFS_RETURN) = G_INT (OFS_PARM0);
	if (s[0] <= ' ')
		PR_RunError ("Bad string");
	if (!SV_Precache_Sound (s))
		PR_RunError ("PF_precache_sound: overflow");
}

/* ---------------------------------------------------------------------------
 * PF_sv_precache_model (pr_cmds.c:1225-1259 / PRBI_FxGlue_PrecacheModel).
 *
 * sv.model_precache has no ADR-011 mirror; this file's own small, private
 * mock array stands in for it (CTEST_FX_MODEL_SLOTS instead of the real
 * MAX_MODELS==8192, so the overflow path is reachable by a test without
 * inserting 8192 distinct names -- documented deviation, only the array
 * *size* differs, the scan/insert/warn/raise logic is transcribed exactly).
 * ss_loading stands in for `sv.state == ss_loading`.
 */

#define CTEST_FX_MODEL_SLOTS 8
static const char *ctest_fx_model_precache[CTEST_FX_MODEL_SLOTS];
static bool		   ctest_fx_ss_loading;

static void ctest_fx_model_precache_reset (void)
{
	memset (ctest_fx_model_precache, 0, sizeof (ctest_fx_model_precache));
	ctest_fx_ss_loading = true; /* matches ctest_world_reset's other ss_* defaults: loading until told otherwise */
}

void ctest_fx_set_ss_loading (bool loading)
{
	ctest_fx_ss_loading = loading;
}

int ctest_fx_model_slot_used (int i)
{
	return (i >= 0 && i < CTEST_FX_MODEL_SLOTS && ctest_fx_model_precache[i]) ? 1 : 0;
}

static void ctest_fx_invoke_precache_model (void *p)
{
	int		   *handle = (int *)p;
	const char *s = PR_GetString (*handle);
	int			i;

	if (s[0] <= ' ')
		PR_RunError ("Bad string");

	for (i = 0; i < CTEST_FX_MODEL_SLOTS; i++)
	{
		if (!ctest_fx_model_precache[i])
		{
			if (!ctest_fx_ss_loading)
				Con_Warning ("PF_precache_model(\"%s\"): Precache should only be done in spawn functions\n", s);
			ctest_fx_model_precache[i] = s;
			return;
		}
		if (!strcmp (ctest_fx_model_precache[i], s))
		{
			if (!ctest_fx_ss_loading && !pr_checkextension.value)
				Con_Warning ("PF_precache_model(\"%s\"): Precache should only be done in spawn functions\n", s);
			return;
		}
	}
	PR_RunError ("PF_precache_model: overflow");
}

int PRBI_FxGlue_PrecacheModel (int handle)
{
	return Host_Guard (ctest_fx_invoke_precache_model, &handle);
}

static void ctest_fx_oracle_pf_precache_model (void)
{
	const char *s;
	int			i;

	s = G_STRING (OFS_PARM0);
	G_INT (OFS_RETURN) = G_INT (OFS_PARM0);
	if (s[0] <= ' ')
		PR_RunError ("Bad string");

	for (i = 0; i < CTEST_FX_MODEL_SLOTS; i++)
	{
		if (!ctest_fx_model_precache[i])
		{
			if (!ctest_fx_ss_loading)
				Con_Warning ("PF_precache_model(\"%s\"): Precache should only be done in spawn functions\n", s);
			ctest_fx_model_precache[i] = s;
			return;
		}
		if (!strcmp (ctest_fx_model_precache[i], s))
		{
			if (!ctest_fx_ss_loading && !pr_checkextension.value)
				Con_Warning ("PF_precache_model(\"%s\"): Precache should only be done in spawn functions\n", s);
			return;
		}
	}
	PR_RunError ("PF_precache_model: overflow");
}

/* ---------------------------------------------------------------------------
 * PF_sv_finalefinished (pr_cmds.c:1845) / PF_sv_CheckPlayerEXFlags
 * (pr_cmds.c:1849). Trivially identical bodies, never raise.
 */

static void ctest_fx_oracle_pf_finalefinished (void)
{
	G_FLOAT (OFS_RETURN) = 0;
}

static void ctest_fx_oracle_pf_check_player_ex_flags (void)
{
	G_FLOAT (OFS_RETURN) = 0;
}

/* ---------------------------------------------------------------------------
 * PF_sv_changelevel (pr_cmds.c:1766-1777). Split across
 * PRBI_Glue_ChangelevelIssued (pr_cmds_glue.c:157, an *existing* wave-1
 * primitive, not owned by pr_cmds_sv_fx_glue.c) and PRBI_FxGlue_ChangeLevel.
 * svs.changelevel_issued has no ADR-011 mirror; ctest_fx_changelevel_issued
 * stands in for it. Cbuf_AddText has no fixture double (its real, renamed
 * c_ref_Cbuf_AddText is a shared global command-text ring buffer other
 * concurrently-running tests also touch), so a private recorder stands in
 * for it instead of calling any real Cbuf machinery.
 */

static bool ctest_fx_changelevel_issued;
static char ctest_fx_changelevel_cmd[256];
static int	ctest_fx_changelevel_call_count;

static void ctest_fx_changelevel_record (const char *level)
{
	snprintf (ctest_fx_changelevel_cmd, sizeof (ctest_fx_changelevel_cmd), "changelevel %s\n", level);
	ctest_fx_changelevel_call_count++;
}

void ctest_fx_changelevel_set_issued (bool v)
{
	ctest_fx_changelevel_issued = v;
}

bool ctest_fx_changelevel_get_issued (void)
{
	return ctest_fx_changelevel_issued;
}

int ctest_fx_changelevel_calls (void)
{
	return ctest_fx_changelevel_call_count;
}

const char *ctest_fx_changelevel_last (void)
{
	return ctest_fx_changelevel_cmd;
}

/* qboolean is bool (q_types.h:122) -- spelled explicitly so MSVC cannot
   invent an int-returning prototype (wave 1 finding, pr_cmds_glue.c:157's
   real signature is `qboolean PRBI_Glue_ChangelevelIssued (qboolean set)`). */
bool PRBI_Glue_ChangelevelIssued (bool set)
{
	bool was = ctest_fx_changelevel_issued;
	if (set)
		ctest_fx_changelevel_issued = true;
	return was;
}

static void ctest_fx_invoke_changelevel (void *p)
{
	int		   *handle = (int *)p;
	const char *s = PR_GetString (*handle);

	ctest_fx_changelevel_record (s);
}

int PRBI_FxGlue_ChangeLevel (int level_handle)
{
	return Host_Guard (ctest_fx_invoke_changelevel, &level_handle);
}

static void ctest_fx_oracle_pf_changelevel (void)
{
	const char *s;

	if (ctest_fx_changelevel_issued)
		return;
	ctest_fx_changelevel_issued = true;

	s = G_STRING (OFS_PARM0);
	ctest_fx_changelevel_record (s);
}

/* ---------------------------------------------------------------------------
 * Fixture reset + oracle dispatcher.
 */

void ctest_fx_reset (int num_edicts)
{
	ctest_world_reset (0, num_edicts < 1 ? 1 : num_edicts);
	ctest_fx_strings_reset ();
	qcvm->strings = ctest_fx_strings;
	qcvm->stringssize = CTEST_FX_STRINGS_CAP;

	ctest_fx_particle_clear ();
	ctest_phys_sound_clear ();
	ctest_phys_sound_arm_raise (0);
	ctest_predd_reset_doubles ();
	ctest_fx_model_precache_reset ();
	ctest_fx_changelevel_issued = false;
	ctest_fx_changelevel_cmd[0] = '\0';
	ctest_fx_changelevel_call_count = 0;
	ctest_clear_con_log ();
	/* pr_checkextension is a plain global cvar_t that ctest_world_reset
	   never touches (only ctest_world_set_cvars does); reset it here so a
	   test that arms it via ctest_world_set_cvars cannot leak state into a
	   later test sharing this process. */
	pr_checkextension.value = 0;
}

enum
{
	CTEST_FX_PF_PARTICLE = 0,
	CTEST_FX_PF_SOUND,
	CTEST_FX_PF_PRECACHE_SOUND,
	CTEST_FX_PF_PRECACHE_MODEL,
	CTEST_FX_PF_FINALEFINISHED,
	CTEST_FX_PF_CHECK_PLAYER_EX_FLAGS,
	CTEST_FX_PF_CHANGELEVEL
};

static int ctest_fx_pf_which;

static void ctest_fx_pf_dispatch (void *arg)
{
	(void)arg;
	switch (ctest_fx_pf_which)
	{
	case CTEST_FX_PF_PARTICLE:
		ctest_fx_oracle_pf_particle ();
		break;
	case CTEST_FX_PF_SOUND:
		ctest_fx_oracle_pf_sound ();
		break;
	case CTEST_FX_PF_PRECACHE_SOUND:
		ctest_fx_oracle_pf_precache_sound ();
		break;
	case CTEST_FX_PF_PRECACHE_MODEL:
		ctest_fx_oracle_pf_precache_model ();
		break;
	case CTEST_FX_PF_FINALEFINISHED:
		ctest_fx_oracle_pf_finalefinished ();
		break;
	case CTEST_FX_PF_CHECK_PLAYER_EX_FLAGS:
		ctest_fx_oracle_pf_check_player_ex_flags ();
		break;
	case CTEST_FX_PF_CHANGELEVEL:
		ctest_fx_oracle_pf_changelevel ();
		break;
	default:
		Sys_Error ("ctest_fx_pf_run: bad index %d", ctest_fx_pf_which);
	}
}

int ctest_fx_pf_run (int which)
{
	ctest_fx_pf_which = which;
	return ctest_try_host (ctest_fx_pf_dispatch, NULL);
}

/* ---------------------------------------------------------------------------
 * Left as abort stubs -- see the file header for why.
 */

int PRBI_FxGlue_AmbientSound (float *pos, int sample_handle, float vol, float attenuation)
{
	(void)pos;
	(void)sample_handle;
	(void)vol;
	(void)attenuation;
	m5_wave2_unimplemented (__func__);
	return 0;
}

int PRBI_FxGlue_LightStyle (int style, int val_handle)
{
	(void)style;
	(void)val_handle;
	m5_wave2_unimplemented (__func__);
	return 0;
}

int PRBI_FxGlue_MakeStatic (void *ent)
{
	(void)ent;
	m5_wave2_unimplemented (__func__);
	return 0;
}

int PRBI_FxGlue_SetSpawnParms (void *ent)
{
	(void)ent;
	m5_wave2_unimplemented (__func__);
	return 0;
}

int PRBI_FxGlue_LocalSound (void *ent, int sample_handle)
{
	(void)ent;
	(void)sample_handle;
	m5_wave2_unimplemented (__func__);
	return 0;
}
