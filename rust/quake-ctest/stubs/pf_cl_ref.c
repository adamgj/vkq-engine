/* Phase 7 M5 wave 2 oracle TU: Group F, PF_cl_* client builtins (pr_cmds_cl_glue.c).
 *
 * STATUS: real oracle. `Quake/pr_cmds.c` and `Quake/pr_ext.c` are deliberately
 * NOT in build.rs's C_SOURCES (adding them is far larger than M5), so there is
 * no c_ref_PF_cl_* symbol to call. The four `ctest_cref_pf_cl_*` functions
 * below are hand transcriptions, statement for statement, of `Quake/pr_cmds.c`
 * (:1779 PF_cl_sound, :1806 PF_cl_ambientsound, :1872 PF_cl_precache_sound,
 * :1931 PF_cl_particle), driven through `ctest_cref_pf_cl_run` exactly like
 * wave 1's `ctest_cref_pf_run` in stubs.c. `PF_cl_makestatic` (pr_cmds.c:1884)
 * is NOT transcribed here -- see the "guard-plumbing probe, not an oracle"
 * note above `PRBI_ClGlue_MakeStatic` below for why, and the Group F report
 * for the full justification.
 *
 * What makes the four transcriptions an oracle rather than a second copy of
 * the port: every primitive they call is the renamed C original --
 * NUM_FOR_EDICT, VectorAdd, VectorMA, G_STRING (PR_GetString), G_EDICT --
 * exactly as `include/c_ref_prelude.h` renames them, so the double-promotion
 * site (`VectorMA`'s internal `a[i] + scale * b[i]`) is evaluated by the C
 * compiler, not by a Rust transcription of it (ADR-010). The one guarded seam
 * these four bodies reach (`PF_cl_precache_sound`'s `PR_CheckEmptyString`)
 * calls the REAL, compiled `PR_RunError` (`c_ref_PR_RunError`, Quake/pr_exec.c
 * is in build.rs's C_SOURCES) directly, matching `Quake/pr_cmds_cl_glue.c`'s
 * real `PRBI_ClInvokeCheckEmptyString` exactly (see that function's oracle
 * stand-in, `ctest_cl_invoke_checkemptystring` below, and
 * `ctest_cref_pf_cl_precache_sound`'s own comment). An earlier revision of
 * this file substituted a direct `Host_Error ("Bad string")` here on the
 * theory that "this harness has no interpreter frame for PR_RunError to
 * unwind" -- that theory was wrong (PR_PrintStatement/PR_StackTrace tolerate
 * this fixture's minimal VM fine, matching Group E's identical
 * `pf_fx_ref.c` precedent) and the substitution was a real bug, caught by
 * `progs_builtins_cl_differential.rs`'s precache_sound tests observing
 * "Bad string" instead of the real "Program error" on the Host_Error
 * channel.
 *
 * HAZARD (wave 1, reconfirmed here): stubs.c #undefs several c_ref_prelude.h
 * rename macros (SV_Move, SV_LinkEdict, SV_HullForEntity, SV_ClipMoveToEntity,
 * SV_TestEntityPosition, SV_CheckBottom, SV_movestep). Never rely on the
 * prelude renames here -- spell oracle call sites as explicit c_ref_* names,
 * and declare an explicit c_ref_* prototype for anything whose header is not
 * included, or MSVC invents an int-returning one and silently corrupts
 * qboolean results.
 *
 * CORRECTION to the previous (abort-stub) revision of this file: its header
 * comment claimed "qboolean is bool (q_types.h:122); PScript_* return it."
 * That is wrong for these two functions -- `Quake/glquake.h:125,127`
 * declares `int PScript_RunParticleEffectTypeString (...)` and
 * `int PScript_RunParticleEffect (...)`, not `qboolean`/`bool`. The
 * quake-c-sys extern block (`rust/quake-c-sys/src/progs_builtins_cl.rs`)
 * already declares both as `-> c_int`, so this was a stub-comment inaccuracy
 * with no port-side consequence; the recorder doubles below return plain
 * `int` to match the real header exactly. `PSET_SCRIPT` (which gates the
 * real-function declarations vs. a `true`-returning macro fallback in
 * glquake.h) is defined unconditionally in `Quake/quakedef.h:38`, so
 * production always compiles the real, linkable functions.
 *
 * SOUND-ENGINE LINKAGE FACT (corrects an earlier, wrong revision of this
 * file): `Quake/snd_dma.c` IS in build.rs's C_SOURCES, and its
 * `S_StartSound` / `S_StaticSound` / `S_PrecacheSound` ARE renamed to
 * `c_ref_*` by the prelude like every other symbol it defines
 * (c_ref_prelude.h:813-823). A previous revision of this file wrongly
 * assumed the *plain* names were unclaimed and `#undef`'d + redefined all
 * three as recording doubles -- but `rust/quake-capi/src/snd_dma.rs` already
 * `#[no_mangle] pub extern "C" fn`-exports the SAME three plain names (the
 * Rust port of `Quake/snd_dma.c` is already flipped, unconditionally linked
 * into every ctest binary), so that redefinition was a hard LNK2005
 * duplicate-symbol error, not a working double -- it happened to be caught
 * before landing because it broke the *whole workspace* build, not just this
 * suite.
 *
 * Fix: call the real oracle, `c_ref_S_StartSound` / `c_ref_S_StaticSound` /
 * `c_ref_S_PrecacheSound`, directly. No local prototype is written for any of
 * the three: `c_ref_prelude.h`'s own include chain (progs.h/protocol.h)
 * transitively drags in `Quake/q_sound.h`, whose prototypes get the same
 * rename applied, so the real, correctly-typed (`sfx_t *`, not `void *`)
 * declaration is already visible -- exactly the hazard-avoidance rule this
 * file's own header already states: spell renamed call sites explicitly,
 * never rely on the plain name, but do not shadow a transitively-visible real
 * prototype with a hand-written one either (a hand-written `void *` version
 * here previously produced MSVC C2371).
 *
 * SCOPE NOTE this creates: neither `c_ref_S_StartSound`/`c_ref_S_StaticSound`
 * (oracle) nor the Rust port's calls into the real `S_StartSound`/
 * `S_StaticSound` (quake-capi) touch any state this harness can read back --
 * both are genuinely safe to call unconditionally because `sound_started`
 * defaults false in this harness (no `S_Init`/DMA bring-up in Group F's
 * scope), so both real implementations take their documented early-return
 * no-op path (confirmed by reading `Quake/snd_dma.c`'s `S_PrecacheSound` /
 * `S_StartSound` and their line-for-line Rust port in
 * `rust/quake-capi/src/snd_dma.rs:406-568`). That makes the call chain safe
 * to exercise for real (crash/raise parity), but it means this suite CANNOT
 * differentially verify `PF_cl_sound`'s / `PF_cl_ambientsound`'s computed
 * call arguments (entnum sign flip, origin midpoint via VectorAdd+VectorMA,
 * vol*255 truncation) against each other -- there is no observable seam left
 * once the plain names are the real, shared-behavior mixer. Standing up a
 * full dual-sided channel-state fixture (one that separately snapshots the
 * C-oracle's `c_ref_snd_channels[]` and the Rust port's private channel
 * table after a real `sound_started=1` DMA bring-up) is a materially larger
 * infra task than Group F's scope and is left undone; the tests below cover
 * what remains observable: `PF_cl_sound`'s `NUM_FOR_EDICT` raise path (which
 * fires before any sound call), and every bit of `PF_cl_precache_sound`
 * (whose `G_INT (OFS_RETURN)` echo and raise paths do not depend on what
 * `S_PrecacheSound` does internally).
 */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>

/* quakedef.h declares this, but c_ref_prelude.h pre-empts quakedef.h itself
 * (its own include-guard trick, prelude:16-18) so this TU never sees it.
 * stubs.c's real Host_Guard/Host_Error setjmp/longjmp trap defines it; this
 * is just its prototype, exactly the way stubs.c's own PRBI_SvGlue_* helpers
 * assume it (they are compiled in the same TU that defines it, so they never
 * needed to). */
extern int		   Host_Guard (void (*fn) (void *), void *arg);
extern const char *ctest_host_error_message (void);

/* stubs.c's real Con_Printf capture buffer -- needed to observe
 * PR_RunError's Con_Printf ("Bad string") side effect (see
 * ctest_cref_pf_cl_precache_sound below), reused for the same reason
 * pf_fx_ref.c (Group E) reuses it. */
extern void		   ctest_clear_con_log (void);
extern int		   ctest_con_log_len (void);
extern const char *ctest_con_log_get (int i);

/* `ctest_world_reset` (stubs.c) publishes a synthetic qcvm on `qcvm` for
 * vm_kind 0 (a private generic VM, unlike vm_kind 1/2 which reuse cl.qcvm /
 * sv.qcvm) -- exactly wave 1's `world_differential.rs` idiom, reused here for
 * the same reason `pf_fx_ref.c` (Group E) reuses it: an ambient qcvm with a
 * real edict arena and string blob is all G_EDICT / NUM_FOR_EDICT / G_STRING
 * need, and building a second one from scratch would just duplicate it. */
extern void ctest_world_reset (int vm_kind, int num_edicts);

/* ---------------------------------------------------------------------------
 * Shared fixture: string pool, global read/write, entity handle helper. Each
 * Group's oracle TU owns a private copy of this (pf_fx_ref.c has its own
 * ctest_fx_* set) rather than sharing one across groups -- no cross-file
 * coupling between peer M5 groups' TUs.
 */

#define CTEST_CL_STRINGS_CAP 1024
static char ctest_cl_strings[CTEST_CL_STRINGS_CAP];
static int	ctest_cl_strings_len;

static void ctest_cl_strings_reset (void)
{
	memset (ctest_cl_strings, 0, sizeof (ctest_cl_strings));
	ctest_cl_strings_len = 1; /* offset 0 reserved as "" (PR_GetString (0) == "") */
}

int ctest_cl_intern (const char *s)
{
	size_t len = strlen (s);
	int	   ofs;

	if (ctest_cl_strings_len + (int)len + 1 > CTEST_CL_STRINGS_CAP)
	{
		fprintf (stderr, "ctest_cl_intern: string pool exhausted\n");
		abort ();
	}
	ofs = ctest_cl_strings_len;
	memcpy (ctest_cl_strings + ofs, s, len + 1);
	ctest_cl_strings_len += (int)len + 1;
	return ofs;
}

void ctest_cl_set_global_float (int ofs, float v)
{
	qcvm->globals[ofs] = v;
}

void ctest_cl_set_global_int (int ofs, int v)
{
	*(int *)&qcvm->globals[ofs] = v;
}

void ctest_cl_set_global_vector (int ofs, float x, float y, float z)
{
	qcvm->globals[ofs] = x;
	qcvm->globals[ofs + 1] = y;
	qcvm->globals[ofs + 2] = z;
}

float ctest_cl_get_global_float (int ofs)
{
	return qcvm->globals[ofs];
}

int ctest_cl_get_global_int (int ofs)
{
	return *(int *)&qcvm->globals[ofs];
}

/* Value to write into a G_EDICT-read global to reference edict `num`. */
int ctest_cl_edict_to_prog (int num)
{
	return EDICT_TO_PROG (EDICT_NUM (num));
}

/* The raw edict_t* for edict `num`, for comparing against what
 * ctest_cl_makestatic_last_ent_get() records (PF_cl_makestatic's PRBI_ClGlue_
 * MakeStatic probe has no oracle dispatch index -- see pf_cl_ref.c's header
 * -- so this is how a test confirms the Rust port handed the probe the same
 * pointer G_EDICT would have computed, without a second oracle transcription). */
void *ctest_cl_edict_ptr (int num)
{
	return (void *)EDICT_NUM (num);
}

/* Sets edict `num`'s v.mins/v.maxs/v.origin (the three vec3_t fields
 * PF_cl_sound reads directly off the edict, not through a global). */
void ctest_cl_edict_set_physics (int num, const float *mins, const float *maxs, const float *origin)
{
	edict_t *e = EDICT_NUM (num);
	memcpy (e->v.mins, mins, sizeof (vec3_t));
	memcpy (e->v.maxs, maxs, sizeof (vec3_t));
	memcpy (e->v.origin, origin, sizeof (vec3_t));
}

/* Resets this file's own fixture (string pool, particle/makestatic
 * recorders) and the shared world/edict arena (vm_kind 0: a private generic
 * VM, not cl.qcvm/sv.qcvm). Forward-declared here; defined after the
 * recorder/probe reset helpers below, which this calls into. */
void ctest_cl_reset (void);
void ctest_cl_makestatic_reset (void);

void ctest_cl_reset_fixture (int num_edicts)
{
	ctest_world_reset (0, num_edicts);
	ctest_cl_strings_reset ();
	/* ctest_world_reset points qcvm->strings at its own small default
	 * buffer; repoint it at this file's own pool so offsets computed by
	 * ctest_cl_intern() resolve against the buffer they were written into
	 * (matching pf_fx_ref.c's ctest_fx_reset -- missing this was a real bug
	 * caught before any test exercised it). */
	qcvm->strings = ctest_cl_strings;
	qcvm->stringssize = CTEST_CL_STRINGS_CAP;
	ctest_cl_reset ();
	ctest_cl_makestatic_reset ();
	ctest_clear_con_log ();
}

/* ---------------------------------------------------------------------------
 * S_StartSound / S_StaticSound / S_PrecacheSound: the REAL c_ref oracle
 * (Quake/snd_dma.c, renamed by the prelude -- see the "SOUND-ENGINE LINKAGE
 * FACT" note above). No local prototype needed: c_ref_prelude.h's own chain
 * (progs.h/protocol.h) transitively drags in Quake/q_sound.h, and its
 * `S_StartSound`/`S_StaticSound`/`S_PrecacheSound` prototypes get the same
 * `#define ... c_ref_*` rename applied -- confirmed by MSVC itself pointing
 * a redeclaration-conflict error (a hand-written prototype here, since
 * removed, disagreed with `sfx_t *` vs `void *`) at `Quake/q_sound.h:107`.
 * Writing our own would only risk exactly the MSVC implicit-int-style
 * mismatch this file's header warns against, so this relies on the real
 * declaration instead of shadowing it.
 */

/* ---------------------------------------------------------------------------
 * PScript_RunParticleEffectTypeString / PScript_RunParticleEffect /
 * R_RunParticleEffect recording doubles. `Quake/r_part_fte.c` / `r_part.c`
 * are not in build.rs's C_SOURCES and c_ref_prelude.h never mentions any of
 * the three names, so there is no rename to work around here -- these are
 * plain-named from the start, same rationale as the sound recorders above
 * (Group F's scope is the argument marshaling PF_cl_particle does, not the
 * particle system's own behavior).
 */

typedef struct
{
	int	  called;
	float org[3];
	float dir[3];
	float count;
	char  name[64];
} ctest_cl_pscript_typestring_t;
static ctest_cl_pscript_typestring_t ctest_cl_pscript_typestring_log;
static int							 ctest_cl_pscript_typestring_ret = 1;

int PScript_RunParticleEffectTypeString (float *org, float *dir, float count, const char *name)
{
	ctest_cl_pscript_typestring_log.called++;
	if (org)
	{
		ctest_cl_pscript_typestring_log.org[0] = org[0];
		ctest_cl_pscript_typestring_log.org[1] = org[1];
		ctest_cl_pscript_typestring_log.org[2] = org[2];
	}
	if (dir)
	{
		ctest_cl_pscript_typestring_log.dir[0] = dir[0];
		ctest_cl_pscript_typestring_log.dir[1] = dir[1];
		ctest_cl_pscript_typestring_log.dir[2] = dir[2];
	}
	ctest_cl_pscript_typestring_log.count = count;
	if (name)
	{
		size_t n = strlen (name);
		if (n >= sizeof (ctest_cl_pscript_typestring_log.name))
			n = sizeof (ctest_cl_pscript_typestring_log.name) - 1;
		memcpy (ctest_cl_pscript_typestring_log.name, name, n);
		ctest_cl_pscript_typestring_log.name[n] = '\0';
	}
	return ctest_cl_pscript_typestring_ret;
}

int ctest_cl_pscript_typestring_called (void)
{
	return ctest_cl_pscript_typestring_log.called;
}
float ctest_cl_pscript_typestring_count (void)
{
	return ctest_cl_pscript_typestring_log.count;
}
const char *ctest_cl_pscript_typestring_name (void)
{
	return ctest_cl_pscript_typestring_log.name;
}
void ctest_cl_pscript_typestring_set_return (int ret)
{
	ctest_cl_pscript_typestring_ret = ret;
}

typedef struct
{
	int	  called;
	float org[3];
	float dir[3];
	int	  color;
	int	  count;
} ctest_cl_pscript_effect_t;
static ctest_cl_pscript_effect_t ctest_cl_pscript_effect_log;
static int						 ctest_cl_pscript_effect_ret = 1;

int PScript_RunParticleEffect (float *org, float *dir, int color, int count)
{
	ctest_cl_pscript_effect_log.called++;
	if (org)
	{
		ctest_cl_pscript_effect_log.org[0] = org[0];
		ctest_cl_pscript_effect_log.org[1] = org[1];
		ctest_cl_pscript_effect_log.org[2] = org[2];
	}
	if (dir)
	{
		ctest_cl_pscript_effect_log.dir[0] = dir[0];
		ctest_cl_pscript_effect_log.dir[1] = dir[1];
		ctest_cl_pscript_effect_log.dir[2] = dir[2];
	}
	ctest_cl_pscript_effect_log.color = color;
	ctest_cl_pscript_effect_log.count = count;
	return ctest_cl_pscript_effect_ret;
}

int ctest_cl_pscript_effect_called (void)
{
	return ctest_cl_pscript_effect_log.called;
}
int ctest_cl_pscript_effect_color (void)
{
	return ctest_cl_pscript_effect_log.color;
}
int ctest_cl_pscript_effect_count (void)
{
	return ctest_cl_pscript_effect_log.count;
}
void ctest_cl_pscript_effect_set_return (int ret)
{
	ctest_cl_pscript_effect_ret = ret;
}

typedef struct
{
	int	  called;
	float org[3];
	float dir[3];
	int	  color;
	int	  count;
} ctest_cl_runparticleeffect_t;
static ctest_cl_runparticleeffect_t ctest_cl_runparticleeffect_log;

void R_RunParticleEffect (float *org, float *dir, int color, int count)
{
	ctest_cl_runparticleeffect_log.called++;
	if (org)
	{
		ctest_cl_runparticleeffect_log.org[0] = org[0];
		ctest_cl_runparticleeffect_log.org[1] = org[1];
		ctest_cl_runparticleeffect_log.org[2] = org[2];
	}
	if (dir)
	{
		ctest_cl_runparticleeffect_log.dir[0] = dir[0];
		ctest_cl_runparticleeffect_log.dir[1] = dir[1];
		ctest_cl_runparticleeffect_log.dir[2] = dir[2];
	}
	ctest_cl_runparticleeffect_log.color = color;
	ctest_cl_runparticleeffect_log.count = count;
}

int ctest_cl_runparticleeffect_called (void)
{
	return ctest_cl_runparticleeffect_log.called;
}
int ctest_cl_runparticleeffect_color (void)
{
	return ctest_cl_runparticleeffect_log.color;
}
int ctest_cl_runparticleeffect_count (void)
{
	return ctest_cl_runparticleeffect_log.count;
}

/* Resets every recorder log above and every configurable return value back
 * to its default (matching "cache hit" / "particle effect ran"), so each
 * test starts from a clean slate without depending on call order. */
void ctest_cl_reset (void)
{
	memset (&ctest_cl_pscript_typestring_log, 0, sizeof (ctest_cl_pscript_typestring_log));
	ctest_cl_pscript_typestring_ret = 1;
	memset (&ctest_cl_pscript_effect_log, 0, sizeof (ctest_cl_pscript_effect_log));
	ctest_cl_pscript_effect_ret = 1;
	memset (&ctest_cl_runparticleeffect_log, 0, sizeof (ctest_cl_runparticleeffect_log));
}

/* ---------------------------------------------------------------------------
 * PRBI_ClGlue_GetString / PRBI_ClGlue_CheckEmptyString: real oracle
 * transcriptions of Quake/pr_cmds_cl_glue.c's bodies (:67-81, :84-94).
 * `Quake/pr_cmds_cl_glue.c` is gated `#ifdef USE_RUST_HOST` and is not in
 * build.rs's C_SOURCES, so (like wave 1's PRBI_SvGlue_* in stubs.c) there is
 * no c_ref_PRBI_ClGlue_* to call -- these ARE the only definitions of these
 * two symbols in this binary.
 */

typedef struct
{
	int			 handle;
	const char **out;
} ctest_cl_getstring_arg_t;

static void ctest_cl_invoke_getstring (void *p)
{
	ctest_cl_getstring_arg_t *a = (ctest_cl_getstring_arg_t *)p;
	*a->out = PR_GetString (a->handle);
}

int PRBI_ClGlue_GetString (int handle, const char **out)
{
	ctest_cl_getstring_arg_t a;
	*out = NULL;
	a.handle = handle;
	a.out = out;
	return Host_Guard (ctest_cl_invoke_getstring, &a);
}

static void ctest_cl_invoke_checkemptystring (void *p)
{
	const char *s = (const char *)p;
	/* Matches Quake/pr_cmds_cl_glue.c's real PRBI_ClInvokeCheckEmptyString
	 * exactly: PR_RunError ("Bad string"), NOT a direct Host_Error. An
	 * earlier revision of this stand-in called Host_Error ("Bad string")
	 * directly on the theory that "this harness has no interpreter frame" --
	 * that theory is wrong: c_ref_PR_RunError (pr_exec.c, real, compiled,
	 * renamed by the prelude -- see ctest_cref_pf_cl_precache_sound's comment
	 * below) tolerates this fixture's minimal VM fine, and calling it
	 * directly is what makes this stand-in produce Host_Error("Program
	 * error") plus a "Bad string" console line, matching the shipping glue's
	 * real behavior instead of diverging from it. Caught by
	 * progs_builtins_cl_differential.rs's precache_sound_empty_string_raises_
	 * program_error_and_matches_oracle test observing "Bad string" on this
	 * side vs the oracle's "Program error". */
	if (s[0] <= ' ')
		c_ref_PR_RunError ("Bad string");
}

int PRBI_ClGlue_CheckEmptyString (const char *s)
{
	return Host_Guard (ctest_cl_invoke_checkemptystring, (void *)s);
}

/* PRBI_ClGlue_MakeStatic is a GUARD-PLUMBING PROBE, not a full oracle
 * transcription of pr_cmds.c:1884's PF_cl_makestatic / pr_cmds_cl_glue.c's
 * PRBI_ClInvokeMakeStatic. The real body needs cl.static_entities /
 * cl.num_statics / cl.max_static_entities / cl.model_precache -- this
 * harness's `ctest_cl_t` stand-in (c_ref_prelude.h) carries only `paused`,
 * `viewentity`, `worldmodel`, `num_entities`, `entities` and `qcvm`, none of
 * the static-entity fields -- plus SV_BuildEntityState and R_AddEfrags,
 * neither of which is compiled into this binary (sv_ents.c / r_efrag.c are
 * not in build.rs's C_SOURCES). Extending the shared ctest_cl_t fixture to
 * carry a full static-entity array is out of Group F's scope.
 *
 * What is in scope: quake_rs_pf_cl_makestatic's OWN logic is exactly
 * "G_EDICT (OFS_PARM0), then propagate PRBI_ClGlue_MakeStatic's Host_Guard
 * status" (progs_builtins_cl.rs:274-282) -- ADR-007 keeps the entire
 * behavioral body in C, so there is no Rust-side arithmetic left to
 * differentially verify beyond that plumbing. This probe is a controllable
 * double: it records every call (so a test can assert the right edict
 * pointer reached it) and, when armed via ctest_cl_makestatic_set_fail (1),
 * raises "Too many static entities" exactly like the real Mem_Realloc /
 * Mem_Alloc failure branch (pr_cmds_cl_glue.c:113-114) -- so both of
 * quake_rs_pf_cl_makestatic's paths (guard success, PRBI_ERR_GUARD
 * propagation with the same message) are exercised without a full
 * client-state fixture. This is NOT a substitute for a real oracle: nothing
 * here checks SV_BuildEntityState's baseline copy, the static-entity array
 * growth, or R_AddEfrags. */
static int	 ctest_cl_makestatic_fail;
static int	 ctest_cl_makestatic_calls;
static void *ctest_cl_makestatic_last_ent;

void ctest_cl_makestatic_set_fail (int fail)
{
	ctest_cl_makestatic_fail = fail != 0;
}
int ctest_cl_makestatic_calls_get (void)
{
	return ctest_cl_makestatic_calls;
}
void *ctest_cl_makestatic_last_ent_get (void)
{
	return ctest_cl_makestatic_last_ent;
}
void ctest_cl_makestatic_reset (void)
{
	ctest_cl_makestatic_fail = 0;
	ctest_cl_makestatic_calls = 0;
	ctest_cl_makestatic_last_ent = NULL;
}

static void ctest_cl_invoke_makestatic (void *p)
{
	ctest_cl_makestatic_calls++;
	ctest_cl_makestatic_last_ent = p;
	if (ctest_cl_makestatic_fail)
		Host_Error ("Too many static entities");
}

int PRBI_ClGlue_MakeStatic (void *ent)
{
	return Host_Guard (ctest_cl_invoke_makestatic, ent);
}

/* ---------------------------------------------------------------------------
 * The C oracle: statement-for-statement transcriptions of pr_cmds.c:1779,
 * :1806, :1872, :1931, driven through ctest_cref_pf_cl_run exactly like wave
 * 1's ctest_cref_pf_run (stubs.c). Host_Guard is the raise trap in both
 * cases, so a Host_Error longjmp out of one of these bodies never unwinds a
 * Rust frame (ADR-009) -- same rule, same trap function.
 */

static void ctest_cref_pf_cl_sound (void)
{
	const char *sample;
	int			channel;
	edict_t	   *entity;
	float		volume;
	float		attenuation;
	int			entnum;
	vec3_t		origin;

	entity = G_EDICT (OFS_PARM0);
	channel = (int)G_FLOAT (OFS_PARM1);
	sample = G_STRING (OFS_PARM2);
	volume = G_FLOAT (OFS_PARM3);
	attenuation = G_FLOAT (OFS_PARM4);

	entnum = NUM_FOR_EDICT (entity);
	entnum *= -1;

	VectorAdd (entity->v.mins, entity->v.maxs, origin);
	VectorMA (entity->v.origin, 0.5, origin, origin);

	c_ref_S_StartSound (entnum, channel, c_ref_S_PrecacheSound (sample), origin, volume, attenuation);
}

static void ctest_cref_pf_cl_ambientsound (void)
{
	const char *samp;
	float	   *pos;
	float		vol, attenuation;

	pos = G_VECTOR (OFS_PARM0);
	samp = G_STRING (OFS_PARM1);
	vol = G_FLOAT (OFS_PARM2) * 255;
	attenuation = G_FLOAT (OFS_PARM3);

	c_ref_S_StaticSound (c_ref_S_PrecacheSound (samp), pos, (int)vol, attenuation);
}

static void ctest_cref_pf_cl_precache_sound (void)
{
	const char *s;

	s = G_STRING (OFS_PARM0);
	G_INT (OFS_RETURN) = G_INT (OFS_PARM0);

	/* pr_cmds.c:1878 calls PR_CheckEmptyString (s), a `static` pr_cmds.c
	 * helper (pr_cmds.c:1148-1152) that itself calls PR_RunError ("Bad
	 * string"). PR_RunError (pr_exec.c:190-207) is real, compiled C
	 * (Quake/pr_exec.c is in build.rs's C_SOURCES, renamed c_ref_PR_RunError
	 * by the prelude) -- it prints the formatted "Bad string" text to the
	 * console via Con_Printf, then always hands Host_Error the LITERAL
	 * "Program error", never the formatted text. An earlier revision of this
	 * transcription called Host_Error ("Bad string") directly, which is
	 * wrong: it skips PR_RunError's console side effect and reports the
	 * wrong message on the Host_Error channel. Fixed to match pf_fx_ref.c's
	 * identical PF_sv_precache_sound/PF_sv_precache_model precedent (see its
	 * ctest_fx_oracle_pf_precache_sound). No explicit c_ref_PR_RunError
	 * prototype is needed: Quake/progs.h declares it and is transitively
	 * included via c_ref_prelude.h's own chain (same rationale as
	 * c_ref_S_StartSound above), and the prelude's plain-name #define makes
	 * this call site resolve to the same renamed symbol either way -- written
	 * as the explicit c_ref_ name per this file's own hazard-avoidance
	 * convention. */
	if (s[0] <= ' ')
		c_ref_PR_RunError ("Bad string");

	/* precache sounds are optional in quake's sound system. NULL is a valid
	   response so don't check (pr_cmds.c:1880-1881). */
	c_ref_S_PrecacheSound (s);
}

static void ctest_cref_pf_cl_particle (void)
{
	float *org = G_VECTOR (OFS_PARM0);
	float *dir = G_VECTOR (OFS_PARM1);
	float  color = G_FLOAT (OFS_PARM2);
	float  count = G_FLOAT (OFS_PARM3);

	if (count == 255)
	{
		if (!PScript_RunParticleEffectTypeString (org, dir, 1, "te_explosion"))
			count = 0;
		else
			count = 1024;
	}
	else
	{
		if (!PScript_RunParticleEffect (org, dir, (int)color, (int)count))
			count = 0;
	}
	R_RunParticleEffect (org, dir, (int)color, (int)count);
}

static int ctest_cref_pf_cl_which;

static void ctest_cref_pf_cl_dispatch (void *p)
{
	(void)p;
	switch (ctest_cref_pf_cl_which)
	{
	case 0:
		ctest_cref_pf_cl_sound ();
		break;
	case 1:
		ctest_cref_pf_cl_ambientsound ();
		break;
	case 2:
		ctest_cref_pf_cl_precache_sound ();
		break;
	case 3:
		ctest_cref_pf_cl_particle ();
		break;
	default:
		Sys_Error ("ctest_cref_pf_cl_run: bad index %d", ctest_cref_pf_cl_which);
	}
}

/* which: 0 = PF_cl_sound, 1 = PF_cl_ambientsound, 2 = PF_cl_precache_sound,
 * 3 = PF_cl_particle. Returns the Host_Guard status (0 = ok, matching
 * quake-capi's guarded()/HOST_GUARD_OK convention). */
int ctest_cref_pf_cl_run (int which)
{
	ctest_cref_pf_cl_which = which;
	return Host_Guard (ctest_cref_pf_cl_dispatch, NULL);
}
