/* pr_ext_ref.c -- the Quake/pr_ext.c differential oracle (Rust migration
 * Phase 7, M9f, T9f.0).
 *
 * WHY THIS FILE EXISTS (the composing-TU pattern, as stubs/host_ref.c)
 *
 * Almost every builtin in pr_ext.c is `static`, so adding Quake/pr_ext.c to
 * build.rs's C_SOURCES would compile it but leave nothing callable: the
 * differential could not reach PF_strzone, PF_stof, PF_substring and the rest
 * under any spelling. This file instead #includes pr_ext.c at the bottom of
 * its own translation unit, after a per-TU rename block, and exposes the
 * statics the tests need through thin non-static forwarders below the include.
 *
 * COST, spelled out so it is not rediscovered: scripts/harness/
 * check_ctest_symbols.sh reads C_SOURCES out of build.rs and sweeps those
 * files for symbols that escaped the prelude's rename. It never inspects a
 * composed object, so this file's rename list is NOT covered by that gate. It
 * was verified once by hand with `llvm-nm --defined-only` over the composed
 * object -- pr_ext.c defines exactly the 19 non-static file-scope symbols
 * renamed below and nothing else -- and that sweep must be repeated by hand if
 * pr_ext.c gains a non-static definition. Three of the nineteen
 * (PF_SV_ForceParticlePrecache, PR_AutoCvarChanged, pr_checkextension) already
 * have plain-named definitions elsewhere in this link and would be hard
 * duplicate-symbol errors without the rename; the other sixteen are renamed
 * for uniformity, so a future stub gaining any of those names cannot collide.
 *
 * WHAT IS *NOT* HERE
 *
 * No C is ported to Rust by this file and no builtin is flipped. T9f.0 is a
 * gate-first task (ADR-019): its deliverable is that the oracle links, plus
 * one differential proving the wiring. The M9f port wave consumes it.
 *
 * LINK DOUBLES
 *
 * pr_ext.c reaches engine symbols that no oracle TU in this link defines
 * (pr_cmds.c, gl_draw.c, gl_screen.c, sbar.c, r_part_fte.c, gl_rlight.c and
 * common.c are all absent). They are defined below in three flavours, each
 * marked at its definition:
 *
 *   TRANSCRIBED -- a faithful statement-for-statement copy of the real body
 *                  (the pf_msg_ref.c convention). Safe for a differential to
 *                  drive.
 *   RECORDER    -- inert data with a real default value, plus a fixture setter
 *                  where a test needs to steer it.
 *   UNREACHED   -- aborts. Every one of these is Phase 8 renderer/server code
 *                  that M9f does not port; a differential that reaches one has
 *                  a bug, and the abort says which symbol.
 *
 * ADR-009: nothing in this file longjmps across a Rust frame. The oracle
 * bodies are driven through ctest_cref_pr_ext_run below, which arms the
 * Host_Error trap inside a C frame with Host_Guard, exactly as pf_cl_ref.c's
 * ctest_cref_pf_cl_run does.
 */

/* ---------------------------------------------------------------------------
 * Per-TU rename block. Every non-static file-scope symbol Quake/pr_ext.c
 * defines, verified by llvm-nm over the composed object (see the header).
 * host_ref.c splits its block into data and functions; there is nothing to
 * split here -- all nineteen are functions.
 */
#define PF_CL_ForceParticlePrecache c_ref_PF_CL_ForceParticlePrecache
#define PF_CL_GetParticle			c_ref_PF_CL_GetParticle
#define PF_Fixme					c_ref_PF_Fixme
#define PF_SV_ForceParticlePrecache c_ref_PF_SV_ForceParticlePrecache
#define PR_AutoCvarChanged			c_ref_PR_AutoCvarChanged
#define PR_Can_Ent_Alpha			c_ref_PR_Can_Ent_Alpha
#define PR_Can_Ent_ColorMod			c_ref_PR_Can_Ent_ColorMod
#define PR_Can_Ent_Scale			c_ref_PR_Can_Ent_Scale
#define PR_Can_Particles			c_ref_PR_Can_Particles
#define PR_DumpBuiltinTable_f		c_ref_PR_DumpBuiltinTable_f
#define PR_DumpPlatform_f			c_ref_PR_DumpPlatform_f
#define PR_EnableExtensions			c_ref_PR_EnableExtensions
#define PR_FindExtFunction			c_ref_PR_FindExtFunction
#define PR_InitExtensions			c_ref_PR_InitExtensions
#define PR_MakeTempString			c_ref_PR_MakeTempString
#define PR_Markup_Begin				c_ref_PR_Markup_Begin
#define PR_Markup_Parse				c_ref_PR_Markup_Parse
#define PR_ShutdownExtensions		c_ref_PR_ShutdownExtensions
#define pr_checkextension			c_ref_pr_checkextension

#include "quakedef.h"
#include "q_ctype.h"

/* Prototypes the force-included c_ref_prelude.h chain already consumed under
 * the plain spelling, before the rename block above was in effect. Written
 * here in plain spelling so the macros rewrite them, which is host_ref.c:141-
 * 143's pattern for exactly this situation. Without these, pr_ext.c's own uses
 * resolve to an undeclared c_ref_* identifier. */
void	 PR_InitExtensions (void);
void	 PR_EnableExtensions (ddef_t *pr_globaldefs);
void	 PR_ShutdownExtensions (void);
func_t	 PR_FindExtFunction (const char *entryname);
void	 PR_DumpPlatform_f (void);
void	 PR_DumpBuiltinTable_f (void);
int		 PF_SV_ForceParticlePrecache (const char *s);
int		 PR_MakeTempString (const char *val);
void	 PF_Fixme (void);
void	 PR_AutoCvarChanged (cvar_t *var);
extern cvar_t pr_checkextension;

/* stubs.c's world fixture; see ctest_pr_ext_reset_fixture below. */
extern void ctest_world_reset (int vm_kind, int num_edicts);
extern void ctest_clear_con_log (void);
/* stubs.c's setjmp/longjmp Host_Error trap (pf_cl_ref.c:118's spelling). */
extern int	Host_Guard (void (*fn) (void *), void *arg);

FUNC_NORETURN static void ctest_pr_ext_unreached (const char *what)
{
	Sys_Error ("pr_ext_ref.c: %s is an UNREACHED link double (Phase 8 surface, not part of M9f)", what);
}

/* ---------------------------------------------------------------------------
 * TRANSCRIBED link doubles.
 */

/* pr_cmds.c:128-136, verbatim. The `byte` index against a 1024-entry ring is
 * the real quirk (only 256 of the buffers are ever handed out); it is copied
 * rather than corrected, since every temp-string builtin M9f ports observes
 * the reuse distance. */
static char pr_string_temp[STRINGTEMP_BUFFERS][STRINGTEMP_LENGTH];
static byte pr_string_tempindex = 0;

char *PR_GetTempString (void)
{
	return pr_string_temp[(STRINGTEMP_BUFFERS - 1) & ++pr_string_tempindex];
}

/* pr_cmds.c:146-151 PF_GetStringArg, used by PF_VarString's LOC_Format path. */
static const char *ctest_pr_ext_getstringarg (int idx, void *userdata)
{
	if (userdata)
		idx += *(int *)userdata;
	if (idx < 0 || idx >= qcvm->argc)
		return "";
	return LOC_GetString (G_STRING (OFS_PARM0 + idx * 3));
}

/* pr_cmds.c:155-195 PF_VarString. DEVIATION, following pf_msg_ref.c's own
 * documented precedent for the same function: the trailing `s > 255`
 * dev_overflows/realtime rate-limited Con_DWarning is not reproduced --
 * host.c's dev_overflows is renamed c_ref_dev_overflows by stubs/host_ref.c,
 * and the branch is diagnostic-only with no effect on the returned string. */
char *PF_VarString (int first)
{
	int			i;
	static char out[1024];
	const char *format;
	size_t		s;

	out[0] = 0;
	s = 0;

	if (first >= qcvm->argc)
		return out;

	format = LOC_GetString (G_STRING ((OFS_PARM0 + first * 3)));
	if (LOC_HasPlaceholders (format))
	{
		int offset = first + 1;
		s = LOC_Format (format, ctest_pr_ext_getstringarg, &offset, out, sizeof (out));
	}
	else
	{
		for (i = first; i < qcvm->argc; i++)
		{
			s = q_strlcat (out, LOC_GetString (G_STRING (OFS_PARM0 + i * 3)), sizeof (out));
			if (s >= sizeof (out))
			{
				Con_Warning ("PF_VarString: overflow (string truncated)\n");
				return out;
			}
		}
	}
	(void)s;
	return out;
}

/* common.c:723-757, verbatim (common.c is not an oracle file; only
 * common_fs.c is in C_SOURCES). Pure, so pr_ext.c:3762's wildcard cvar
 * filtering is drivable by a differential as-is. */
int wildcmp (const char *wild, const char *string)
{
	while (*string)
	{
		if (*wild == '*')
		{
			if (*string == '/' || *string == '\\')
			{
				wild++;
				continue;
			}
			if (wildcmp (wild + 1, string))
				return true;
			string++;
		}
		else if ((q_tolower (*wild) == q_tolower (*string)) || (*wild == '?'))
		{
			wild++;
			string++;
		}
		else
		{
			return false;
		}
	}

	while (*wild == '*')
	{
		wild++;
	}
	return !*wild;
}

/* ---------------------------------------------------------------------------
 * RECORDER link doubles: real defaults, plus setters where a test steers one.
 */

/* gl_screen.c:85 / pr_cmds.c:383 / r_part_fte.c:31,485. Cvar_RegisterVariable
 * is never run over these here, so `.value` is set explicitly to the value
 * parsing the default string would produce. */
cvar_t scr_sbarscale = {"scr_sbarscale", "1", CVAR_ARCHIVE, 1.0f};
cvar_t sv_gameplayfix_setmodelrealbox = {"sv_gameplayfix_setmodelrealbox", "1", 0, 1.0f};
cvar_t r_fteparticles = {"r_fteparticles", "1", CVAR_ARCHIVE, 1.0f};
cvar_t r_particledesc = {"r_particledesc", "classic", 0, 0.0f};

/* glquake.h:41. PR_GetVMScale (pr_ext.c:37-45) divides it, so a differential
 * over that helper must be able to set it; 640 is the engine's own startup
 * default. glheight is not defined here: pr_ext.c never names it. */
int glwidth = 640;

void ctest_pr_ext_set_glwidth (int w)
{
	glwidth = w;
}

void ctest_pr_ext_set_sbarscale (float v)
{
	scr_sbarscale.value = v;
}

/* gl_texmgr.c:63, gl_draw.c:36, gl_screen.c:139 and the COMPILE-ONLY
 * vulkanglobals_t from c_ref_prelude.h. All inert: the code that reads them
 * is Phase 8 (PF_cl_draw*). sbar.c:439-441's fragsort/scoreboardlines moved
 * to Quake/sbar_glue.c's oracle mirror in stubs/sbar_ref.c at M10d. */
unsigned int	d_8to24table[256];
qpic_t		   *pic_nul;
gltexture_t	   *char_texture;
qmutex_t	   *draw_qcvm_mutex;
vulkanglobals_t vulkan_globals;

/* ---------------------------------------------------------------------------
 * UNREACHED link doubles. Phase 8 renderer surface.
 */
byte *R_VertexAllocate (int size, VkBuffer *buffer, VkDeviceSize *buffer_offset)
{
	(void)size;
	(void)buffer;
	(void)buffer_offset;
	ctest_pr_ext_unreached ("R_VertexAllocate");
}

void R_BindPipeline (cb_context_t *cbx, VkPipelineBindPoint bind_point, vulkan_pipeline_t pipeline)
{
	(void)cbx;
	(void)bind_point;
	(void)pipeline;
	ctest_pr_ext_unreached ("R_BindPipeline");
}

void vkCmdSetScissor (VkCommandBuffer cb, uint32_t first, uint32_t count, const VkRect2D *scissors)
{
	(void)cb;
	(void)first;
	(void)count;
	(void)scissors;
	ctest_pr_ext_unreached ("vkCmdSetScissor");
}

int R_LightPoint (vec3_t p, float ofs, lightcache_t *cache, vec3_t *lightcolor)
{
	(void)p;
	(void)ofs;
	(void)cache;
	(void)lightcolor;
	ctest_pr_ext_unreached ("R_LightPoint");
}

qpic_t *Draw_PicFromWad2 (const char *name, unsigned int texflags, int picflags)
{
	(void)name;
	(void)texflags;
	(void)picflags;
	ctest_pr_ext_unreached ("Draw_PicFromWad2");
}

qpic_t *Draw_GetCachedPic (const char *path)
{
	(void)path;
	ctest_pr_ext_unreached ("Draw_GetCachedPic");
}

qpic_t *Draw_TryCachePic (const char *path, unsigned int texflags, int picflags)
{
	(void)path;
	(void)texflags;
	(void)picflags;
	ctest_pr_ext_unreached ("Draw_TryCachePic");
}

/* UNREACHED link doubles, pr_cmds.c server surface: builtin-table rows
 * pr_ext.c shares with pr_cmds.c, plus the two helpers its setmodel/message
 * builtins call. Porting any of these is M9e/Phase 8 work, not M9f's. */
sizebuf_t *WriteDest (void)
{
	ctest_pr_ext_unreached ("WriteDest");
}

void SetMinMaxSize (edict_t *e, float *minvec, float *maxvec, qboolean rotate)
{
	(void)e;
	(void)minvec;
	(void)maxvec;
	(void)rotate;
	ctest_pr_ext_unreached ("SetMinMaxSize");
}

#define CTEST_PR_EXT_UNREACHED_BUILTIN(name) \
	void name (void)                         \
	{                                        \
		ctest_pr_ext_unreached (#name);      \
	}

CTEST_PR_EXT_UNREACHED_BUILTIN (PF_bprint)
CTEST_PR_EXT_UNREACHED_BUILTIN (PF_sprint)
CTEST_PR_EXT_UNREACHED_BUILTIN (PF_centerprint)
CTEST_PR_EXT_UNREACHED_BUILTIN (PF_sv_CheckPlayerEXFlags)
CTEST_PR_EXT_UNREACHED_BUILTIN (PF_sv_finalefinished)
CTEST_PR_EXT_UNREACHED_BUILTIN (PF_sv_localsound)
CTEST_PR_EXT_UNREACHED_BUILTIN (PF_sv_walkpathtogoal)

/* ---------------------------------------------------------------------------
 * The oracle itself.
 */

#include "pr_ext.c"

/* ---------------------------------------------------------------------------
 * Fixture. A private copy per oracle TU (the pf_cl_ref.c / pf_fx_ref.c rule:
 * no cross-file coupling between peer stub TUs).
 */

#define CTEST_PR_EXT_STRINGS_CAP 2048
static char ctest_pr_ext_strings[CTEST_PR_EXT_STRINGS_CAP];
static int	ctest_pr_ext_strings_len;

int ctest_pr_ext_intern (const char *s)
{
	size_t len = strlen (s);
	int	   ofs;

	if (ctest_pr_ext_strings_len + (int)len + 1 > CTEST_PR_EXT_STRINGS_CAP)
		Sys_Error ("ctest_pr_ext_intern: string pool exhausted");
	ofs = ctest_pr_ext_strings_len;
	memcpy (ctest_pr_ext_strings + ofs, s, len + 1);
	ctest_pr_ext_strings_len += (int)len + 1;
	return ofs;
}

/* Resets the shared world/edict arena (vm_kind 0: the private generic VM, not
 * cl.qcvm/sv.qcvm) and repoints qcvm->strings at this file's own pool, so
 * offsets from ctest_pr_ext_intern resolve against the buffer they were
 * written into. ctest_world_reset memsets the whole qcvm, which clears
 * knownstrings/numknownstrings/maxknownstrings and knownzone/knownzonesize --
 * that is what makes a C run and a Rust run over the same script hand out the
 * same string handles and therefore be comparable. */
void ctest_pr_ext_reset_fixture (int num_edicts)
{
	ctest_world_reset (0, num_edicts);
	memset (ctest_pr_ext_strings, 0, sizeof (ctest_pr_ext_strings));
	ctest_pr_ext_strings_len = 1; /* offset 0 reserved as "" */
	qcvm->strings = ctest_pr_ext_strings;
	qcvm->stringssize = CTEST_PR_EXT_STRINGS_CAP;
	qcvm->argc = 0;
	ctest_clear_con_log ();
}

void ctest_pr_ext_set_argc (int argc)
{
	qcvm->argc = argc;
}

void ctest_pr_ext_set_global_int (int ofs, int v)
{
	*(int *)&qcvm->globals[ofs] = v;
}

int ctest_pr_ext_get_global_int (int ofs)
{
	return *(int *)&qcvm->globals[ofs];
}

/* The oracle's own PR_GetString (c_ref_PR_GetString, pr_edict_arena.c). Both
 * sides intern through it -- stubs.c:6002-6019 forwards the plain-named
 * PR_SetEngineString/PR_ClearEngineString the Rust port imports straight back
 * to the c_ref pair -- so one accessor reads back either side's result. */
const char *ctest_pr_ext_get_string (int handle)
{
	return PR_GetString (handle);
}

size_t ctest_pr_ext_knownzone_size (void)
{
	return qcvm->knownzonesize;
}

int ctest_pr_ext_knownzone_allocated (void)
{
	return qcvm->knownzone != NULL;
}

int ctest_pr_ext_knownzone_test (size_t id)
{
	if (id >= qcvm->knownzonesize || !qcvm->knownzone)
		return 0;
	return (qcvm->knownzone[id >> 3] & (1u << (id & 7))) != 0;
}

/* Dispatch indices; must match tests/pr_ext_differential.rs's `mod pf`. */
static int ctest_cref_pr_ext_which;

static void ctest_cref_pr_ext_dispatch (void *p)
{
	(void)p;
	switch (ctest_cref_pr_ext_which)
	{
	case 0:
		PF_strzone ();
		break;
	case 1:
		PF_strunzone ();
		break;
	case 2:
		PR_UnzoneAll ();
		break;
	default:
		Sys_Error ("ctest_cref_pr_ext_run: bad index %d", ctest_cref_pr_ext_which);
	}
}

/* Returns the Host_Guard status (0 = ok, matching quake-capi's
   guarded()/HOST_GUARD_OK convention). */
int ctest_cref_pr_ext_run (int which)
{
	ctest_cref_pr_ext_which = which;
	return Host_Guard (ctest_cref_pr_ext_dispatch, NULL);
}

/* ==== M9F GROUP A BEGIN ==== */
/* pr_ext.c sprintf group -- PF_sprintf / PF_sprintf_internal (:1110-1589),
 * ported in quake-capi/src/progs_builtins_sprintf.rs and driven by
 * tests/pr_ext_sprintf_differential.rs.
 *
 * Self-contained on purpose. The five M9f port groups are written in parallel
 * and every one of them adding a case to ctest_cref_pr_ext_dispatch above
 * would collide in the same three lines, so this group brings its own
 * Host_Guard runner over its own reserved index range (10-19). Fold it into
 * the shared switch at integration time if a single dispatcher is preferred.
 *
 * ADR-009: as ctest_cref_pr_ext_run, the oracle body runs inside Host_Guard,
 * so a Host_Error/PR_RunError unwinds in a C frame and never crosses Rust. */

/* PF_sprintf's arguments are QC floats, and the existing fixture only has an
 * int setter. */
void ctest_m9fa_set_global_float (int ofs, float v)
{
	qcvm->globals[ofs] = v;
}

static int ctest_m9fa_which;

static void ctest_m9fa_dispatch (void *p)
{
	(void)p;
	switch (ctest_m9fa_which)
	{
	case 10:
		PF_sprintf ();
		break;
	default:
		Sys_Error ("ctest_m9fa_run: bad index %d", ctest_m9fa_which);
	}
}

/* Returns the Host_Guard status (0 = ok), as ctest_cref_pr_ext_run. */
int ctest_m9fa_run (int which)
{
	ctest_m9fa_which = which;
	return Host_Guard (ctest_m9fa_dispatch, NULL);
}
/* ==== M9F GROUP A END ==== */

/* ==== M9F GROUP B BEGIN ==== */
/* Phase 7 M9f group B (strconv / infoadd / infoget / the qc tokenizer /
 * strftime / stov) -- the fixture and dispatcher for
 * tests/pr_ext_strext_differential.rs against
 * rust/quake-capi/src/progs_builtins_strext.rs.
 *
 * WHY A SECOND DISPATCHER RATHER THAN MORE CASES IN THE FIRST
 *
 * M9f ports five groups in parallel and every group needs oracle cases. If
 * each one edited ctest_cref_pr_ext_dispatch's switch they would all edit the
 * same few lines. A per-group runner keeps each group's additions in one
 * contiguous, conflict-free block. The `which` numbering is still globally
 * partitioned (group B owns 20-39), so the two schemes can be folded into one
 * switch later without renumbering anything.
 *
 * WHAT THE RAW ACCESSORS ARE FOR
 *
 * qctoken/qctoken_count are pr_ext.c file statics on this side and Rust
 * statics on the other, so no single accessor can read both. The differential
 * therefore observes tokenizer state symmetrically, through the ported
 * builtins themselves (PF_ArgC / PF_ArgV / PF_argv_start_index /
 * PF_argv_end_index). The ctest_pr_ext_strext_token_* accessors below read
 * this side's array directly and exist to validate that observation channel:
 * a test asserts the C builtins report what the C array actually holds, so a
 * shared misreading cannot hide a difference.
 *
 * THE TEMP-STRING RING IS SHARED, AND IS ITSELF AN OBSERVABLE
 *
 * PR_GetTempString above is the only definition in this link, so both sides
 * step the same pr_string_tempindex. How far it moves is directly comparable,
 * which is what pins PF_infoadd's quirk of taking a temp string before its
 * empty-key early return while PF_infoget takes one only on a hit.
 * ctest_pr_ext_strext_reset rewinds the index (and flushes this side's token
 * table) so each half of a comparison starts from the same place.
 */

void ctest_pr_ext_set_global_float (int ofs, float v)
{
	qcvm->globals[ofs] = v;
}

float ctest_pr_ext_get_global_float (int ofs)
{
	return qcvm->globals[ofs];
}

/* Flushes this side's token table and rewinds the shared temp-string ring.
   ctest_pr_ext_reset_fixture does neither: qctoken is pr_ext.c's own static
   and pr_string_tempindex is this file's, and both outlive a qcvm reset. */
void ctest_pr_ext_strext_reset (void)
{
	tokenize_flush ();
	pr_string_tempindex = 0;
}

int ctest_pr_ext_strext_tempindex (void)
{
	return pr_string_tempindex;
}

int ctest_pr_ext_strext_token_count (void)
{
	return (int)qctoken_count;
}

int ctest_pr_ext_strext_token_start (int i)
{
	if ((unsigned int)i >= qctoken_count)
		return -1;
	return (int)qctoken[i].start;
}

int ctest_pr_ext_strext_token_end (int i)
{
	if ((unsigned int)i >= qctoken_count)
		return -1;
	return (int)qctoken[i].end;
}

const char *ctest_pr_ext_strext_token_text (int i)
{
	if ((unsigned int)i >= qctoken_count)
		return NULL;
	return qctoken[i].token;
}

/* PR_GetString (pr_edict_arena.c:307-326) raises only for a *negative*
   string_t inside the knownstrings range whose slot is NULL; an out-of-range
   positive offset silently returns qcvm->strings, because the Host_Error in
   that arm sits behind a return and is dead. Arming a cleared slot is
   therefore the only way to reach the raise path from a builtin argument, and
   it has to happen after ctest_pr_ext_reset_fixture, which zeroes
   numknownstrings.

   The table below is static storage, so a builtin that got as far as
   PR_SetEngineString could hand it to Z_Realloc. Only use the returned handle
   on a builtin that reads it before interning anything -- every group B
   builtin resolves its arguments first, so all of them qualify. */
static const char *ctest_pr_ext_strext_knownstrings[4];

int ctest_pr_ext_strext_arm_bad_string (void)
{
	int i;
	for (i = 0; i < 4; i++)
		ctest_pr_ext_strext_knownstrings[i] = NULL;
	qcvm->knownstrings = ctest_pr_ext_strext_knownstrings;
	qcvm->numknownstrings = 4;
	qcvm->maxknownstrings = 4;
	return -1; /* knownstrings[0], which is NULL */
}

/* Dispatch indices 20-39 are group B's; they must match
   tests/pr_ext_strext_differential.rs's `mod pf`. Index 32 drives
   tokenize_flush, which is not a builtin -- PR_ShutdownExtensions calls it
   directly, which is why the Rust port has to own it (and that call site)
   along with the token table. */
static int ctest_cref_pr_ext_strext_which;

static void ctest_cref_pr_ext_strext_dispatch (void *p)
{
	(void)p;
	switch (ctest_cref_pr_ext_strext_which)
	{
	case 20:
		PF_strconv ();
		break;
	case 21:
		PF_infoadd ();
		break;
	case 22:
		PF_infoget ();
		break;
	case 23:
		PF_Tokenize ();
		break;
	case 24:
		PF_tokenize_console ();
		break;
	case 25:
		PF_tokenizebyseparator ();
		break;
	case 26:
		PF_ArgC ();
		break;
	case 27:
		PF_ArgV ();
		break;
	case 28:
		PF_argv_start_index ();
		break;
	case 29:
		PF_argv_end_index ();
		break;
	case 30:
		PF_strftime ();
		break;
	case 31:
		PF_stov ();
		break;
	case 32:
		tokenize_flush ();
		break;
	default:
		Sys_Error ("ctest_cref_pr_ext_strext_run: bad index %d", ctest_cref_pr_ext_strext_which);
	}
}

/* Returns the Host_Guard status (0 = ok), as ctest_cref_pr_ext_run does. */
int ctest_cref_pr_ext_strext_run (int which)
{
	ctest_cref_pr_ext_strext_which = which;
	return Host_Guard (ctest_cref_pr_ext_strext_dispatch, NULL);
}
/* ==== M9F GROUP B END ==== */

/* ==== M9F GROUP C BEGIN ====
 *
 * Phase 7 M9f group C: the FRIK_FILE + strbuf builtins
 * (Quake/pr_ext.c:3130-3773), differentially tested against
 * rust/quake-capi/src/progs_builtins_filebuf.rs.
 *
 * Both halves need a real gamedir: PF_fopen goes through COM_FOpenFile, so
 * the fixture points com_searchpaths at a scratch directory it creates. The
 * C oracle half below drives the prelude-renamed c_ref_* filesystem and cvar
 * registry; the Rust half further down (ctest_pr_ext_rs_*) #undefs those
 * spellings and drives quake-capi's own. See the comment at that block for
 * why there are two of each.
 */

static searchpath_t	 ctest_pr_ext_searchpath;
static searchpath_t *ctest_pr_ext_saved_searchpaths;
static float		 ctest_pr_ext_saved_registered;

void ctest_pr_ext_fs_setup (const char *dir)
{
	char sub[MAX_OSPATH];

	Sys_mkdir (dir);
	q_snprintf (sub, sizeof (sub), "%s/data", dir);
	Sys_mkdir (sub);

	q_strlcpy (com_gamedir, dir, sizeof (com_gamedir));

	memset (&ctest_pr_ext_searchpath, 0, sizeof (ctest_pr_ext_searchpath));
	ctest_pr_ext_searchpath.path_id = 1;
	q_strlcpy (ctest_pr_ext_searchpath.filename, dir, sizeof (ctest_pr_ext_searchpath.filename));

	ctest_pr_ext_saved_searchpaths = com_searchpaths;
	ctest_pr_ext_saved_registered = registered.value;
	com_searchpaths = &ctest_pr_ext_searchpath;
	registered.value = 1;
}

void ctest_pr_ext_fs_teardown (void)
{
	com_searchpaths = ctest_pr_ext_saved_searchpaths;
	registered.value = ctest_pr_ext_saved_registered;
	com_gamedir[0] = 0;
}

/* Cvar registry control for PF_buf_cvarlist. Cvar_FindVarAfter walks
 * cvar_vars, which the ctest binary shares with every other cvar test in it,
 * so the differential registers its own known set and compares against that
 * rather than against whatever else happens to be registered. */
void ctest_pr_ext_cvar_register (cvar_t *var)
{
	Cvar_RegisterVariable (var);
}

static int ctest_cref_pr_ext_m9f_c_which;

static void ctest_cref_pr_ext_dispatch_m9f_c (void *p)
{
	(void)p;
	switch (ctest_cref_pr_ext_m9f_c_which)
	{
	case 40:
		PF_fopen ();
		break;
	case 41:
		PF_fgets ();
		break;
	case 42:
		PF_fputs ();
		break;
	case 43:
		PF_fclose ();
		break;
	case 44:
		PF_fseek ();
		break;
	case 45:
		PF_whichpack ();
		break;
	case 46:
		PF_buf_create ();
		break;
	case 47:
		PF_buf_del ();
		break;
	case 48:
		PF_buf_getsize ();
		break;
	case 49:
		PF_buf_copy ();
		break;
	case 50:
		PF_buf_sort ();
		break;
	case 51:
		PF_buf_implode ();
		break;
	case 52:
		PF_bufstr_get ();
		break;
	case 53:
		PF_bufstr_set ();
		break;
	case 54:
		PF_bufstr_add ();
		break;
	case 55:
		PF_bufstr_free ();
		break;
	case 56:
		PF_buf_cvarlist ();
		break;
	case 57:
		PF_frikfile_shutdown ();
		break;
	case 58:
		PF_buf_shutdown ();
		break;
	default:
		Sys_Error ("ctest_cref_pr_ext_run_m9f_c: bad index %d", ctest_cref_pr_ext_m9f_c_which);
	}
}

/* Returns the Host_Guard status, same convention as ctest_cref_pr_ext_run. */
int ctest_cref_pr_ext_run_m9f_c (int which)
{
	ctest_cref_pr_ext_m9f_c_which = which;
	return Host_Guard (ctest_cref_pr_ext_dispatch_m9f_c, NULL);
}

/* ---------------------------------------------------------------------------
 * Plain-spelling twins, for the Rust half of each comparison.
 *
 * c_ref_prelude.h renames com_gamedir (:406), com_searchpaths (:410) and
 * Cvar_RegisterVariable (:321), so everything above reaches common_fs.c's and
 * cvar.c's objects. quake-capi defines its own #[no_mangle] com_gamedir
 * (fs.rs:56), com_searchpaths (fs.rs:69) and cvar registry (cvar.rs:37), and
 * the port under test reaches *those*. There are therefore two filesystems and
 * two cvar registries in this link, and each side of a comparison has to be
 * pointed at its own. This is host_glue_ref.c:656-671's pattern; the #undefs
 * are undone again below so the rest of the file is unaffected.
 *
 * `registered` is NOT renamed, so both sides share one cvar_t; it is saved per
 * side anyway so the two setups stay independent.
 */
#undef com_gamedir
#undef com_searchpaths
#undef Cvar_RegisterVariable

extern char			 com_gamedir[MAX_OSPATH];
extern searchpath_t *com_searchpaths;
/* quake-capi/src/cvar.rs:887 -- the raise-returning register export. */
extern void quake_rs_cvar_register_variable (cvar_t *variable, int *raised);

static searchpath_t	 ctest_pr_ext_rs_searchpath;
static searchpath_t *ctest_pr_ext_rs_saved_searchpaths;
static float		 ctest_pr_ext_rs_saved_registered;

void ctest_pr_ext_rs_fs_setup (const char *dir)
{
	char sub[MAX_OSPATH];

	Sys_mkdir (dir);
	q_snprintf (sub, sizeof (sub), "%s/data", dir);
	Sys_mkdir (sub);

	q_strlcpy (com_gamedir, dir, sizeof (com_gamedir));

	memset (&ctest_pr_ext_rs_searchpath, 0, sizeof (ctest_pr_ext_rs_searchpath));
	ctest_pr_ext_rs_searchpath.path_id = 1;
	q_strlcpy (ctest_pr_ext_rs_searchpath.filename, dir, sizeof (ctest_pr_ext_rs_searchpath.filename));

	ctest_pr_ext_rs_saved_searchpaths = com_searchpaths;
	ctest_pr_ext_rs_saved_registered = registered.value;
	com_searchpaths = &ctest_pr_ext_rs_searchpath;
	registered.value = 1;
}

void ctest_pr_ext_rs_fs_teardown (void)
{
	com_searchpaths = ctest_pr_ext_rs_saved_searchpaths;
	registered.value = ctest_pr_ext_rs_saved_registered;
	com_gamedir[0] = 0;
}

/* Returns the raise status, so the caller can assert it stayed 0. */
int ctest_pr_ext_rs_cvar_register (cvar_t *var)
{
	int raised = 0;
	quake_rs_cvar_register_variable (var, &raised);
	return raised;
}

#define com_gamedir			  c_ref_com_gamedir
#define com_searchpaths		  c_ref_com_searchpaths
#define Cvar_RegisterVariable c_ref_Cvar_RegisterVariable

/* ==== M9F GROUP C END ==== */

/* ==== M9F GROUP D BEGIN ====
 *
 * Phase 7 M9f group D: the sv/cl temp-entity builtins
 * (Quake/pr_ext.c:2647-3061), differentially tested against
 * rust/quake-capi/src/progs_builtins_te.rs. Self-contained: a private
 * fixture plus a second, independent dispatcher (ctest_cref_pr_ext_te_run,
 * indices 60-99) rather than extending ctest_cref_pr_ext_dispatch above, so
 * this block never has to interleave with the T9f.0 baseline it was added
 * next to.
 *
 * WHY THE STATIC PF_sv_te_* / PF_cl_te_* BODIES ARE CALLED DIRECTLY
 *
 * Every builtin this block drives is `static` in Quake/pr_ext.c, but that
 * file is #included above (not linked as a separate TU), so its statics are
 * ordinary file-scope names for the remainder of *this* TU -- callable
 * directly, with no oracle transcription needed and therefore no
 * transcription-fidelity risk (unlike pf_msg_ref.c's hand-copied bodies).
 * SV_Multicast / PF_multicast_internal (both static, pr_ext.c:4169,:4216) run
 * for real too, PVS fanout included, exercising the identical control flow
 * rust_sv_multicast_unreliable (progs_builtins_te.rs) reimplements.
 *
 * WHY sv/svs NEED THEIR OWN #undef HERE
 *
 * c_ref_prelude.h (force-included ahead of this whole TU, same as every
 * C_SOURCES file) renames `sv`/`svs` to `c_ref_sv`/`c_ref_svs` for the
 * duration of the translation unit. The sv_te_*-prefixed and SV_Multicast bodies above
 * were already compiled under that rename (real, pristine C oracle storage,
 * shared with Quake/sv_main.c which is also in C_SOURCES). Below this point
 * the macros are turned off so the plain names `sv`/`svs` reach the
 * Rust-owned storage instead (quake-capi/src/sv_main.rs, T6.6 -- the exact
 * mechanism documented in stubs/sv_main_ref.c's own header comment). Two
 * independent storages, one process: exactly what a differential needs.
 *
 * WHY NO WORLDMODEL/PVS FIXTURE IS BUILT HERE
 *
 * ctest_pr_ext_reset_fixture (already called by ctest_pr_ext_te_reset below)
 * runs ctest_world_reset(0, ...), which points the ambient qcvm->worldmodel
 * at stubs.c's shared synthetic brush model. Mod_PointInLeaf/Mod_LeafPVS
 * (stubs.c) are themselves settable test doubles, not renamed by the
 * prelude (they are not part of any oracle C_SOURCES file), so both the C
 * oracle's SV_Multicast and the Rust port's sv_multicast_pvs_u call the same
 * shared fake -- neutral infrastructure, not part of what this differential
 * compares. Every scenario below runs with svs.maxclients == 0 except the
 * particlerain/particlesnow ones (MULTICAST_ALL_U's fanout has no
 * broadcast-when-empty fallback, unlike the PHS_U path), so the PVS fanout
 * itself is not exercised; SV_Multicast still unconditionally computes it at
 * MULTICAST_PVS_U call sites (pr_ext.c:4242) even with zero clients, so the
 * shared fake must exist and be safe to call regardless -- it is.
 *
 * SCOPE: the 3 SV_StartParticle-based builtins plus the 16 sv_te_* network
 * writers (19 total) are covered here; all of them are byte-comparable via
 * sv.datagram (directly, or via the PHS_U/ALL_U multicast collapse -- see
 * progs_builtins_te.rs's module doc). The 14 cl_te_* builtins are pure
 * client-side rendering/audio side effects (dlights, particle systems,
 * S_StartSound) with no wire bytes to compare and are NOT dispatched here;
 * see the M9f group D report for that gap.
 */

#undef sv
#undef svs
extern server_t		sv;	 /* Rust: quake-capi/src/sv_main.rs (T6.6) */
extern server_static_t svs; /* Rust: quake-capi/src/sv_main.rs (T6.6) */
extern server_t		c_ref_sv;  /* oracle: Quake/sv_main.c under the prelude */
extern server_static_t c_ref_svs;

/* stubs.c:6126 -- shared across every stub TU, operates on the ambient qcvm
 * ctest_pr_ext_reset_fixture already publishes. */
extern int ctest_pf_edict_prog (int num);

#define CTEST_PR_EXT_TE_CLIENTS 2

static client_t ctest_pr_ext_te_clients_r[CTEST_PR_EXT_TE_CLIENTS];
static client_t ctest_pr_ext_te_clients_c[CTEST_PR_EXT_TE_CLIENTS];

static void ctest_pr_ext_te_init_sv (server_t *s)
{
	memset (s, 0, sizeof (*s));
	s->datagram.data = s->datagram_buf;
	s->datagram.maxsize = sizeof (s->datagram_buf);
	s->datagram.allowoverflow = true;
	s->multicast.data = s->multicast_buf;
	s->multicast.maxsize = sizeof (s->multicast_buf);
	s->multicast.allowoverflow = true;
	s->protocolflags = 0;
}

static void ctest_pr_ext_te_init_client (client_t *c)
{
	memset (c, 0, sizeof (*c));
	c->datagram.data = c->datagram_buf;
	c->datagram.maxsize = sizeof (c->datagram_buf);
	c->datagram.allowoverflow = true;
}

/* Resets the shared world/edict arena (via ctest_pr_ext_reset_fixture), then
 * both sv/svs copies and Group D's own private client array. `maxclients`
 * activates that many clients (indices 0..maxclients-1) with `pext2` set on
 * both sides -- needed only by particlerain/particlesnow's MULTICAST_ALL_U
 * fanout; every other scenario in this file passes maxclients == 0. */
void ctest_pr_ext_te_reset (int num_edicts, int maxclients, unsigned int pext2)
{
	int i;

	ctest_pr_ext_reset_fixture (num_edicts);

	ctest_pr_ext_te_init_sv (&sv);
	ctest_pr_ext_te_init_sv (&c_ref_sv);

	memset (&svs, 0, sizeof (svs));
	memset (&c_ref_svs, 0, sizeof (c_ref_svs));
	for (i = 0; i < CTEST_PR_EXT_TE_CLIENTS; i++)
	{
		ctest_pr_ext_te_init_client (&ctest_pr_ext_te_clients_r[i]);
		ctest_pr_ext_te_init_client (&ctest_pr_ext_te_clients_c[i]);
		ctest_pr_ext_te_clients_r[i].active = (i < maxclients);
		ctest_pr_ext_te_clients_c[i].active = (i < maxclients);
		ctest_pr_ext_te_clients_r[i].protocol_pext2 = pext2;
		ctest_pr_ext_te_clients_c[i].protocol_pext2 = pext2;
	}
	svs.maxclients = maxclients;
	svs.clients = ctest_pr_ext_te_clients_r;
	c_ref_svs.maxclients = maxclients;
	c_ref_svs.clients = ctest_pr_ext_te_clients_c;
}

/* `side`: 0 = C oracle (c_ref_sv/c_ref_svs), 1 = Rust (sv/svs) -- must match
 * tests/pr_ext_te_differential.rs's `Side` enum exactly. */
int ctest_pr_ext_te_datagram_len (int side)
{
	return (side == 0 ? &c_ref_sv : &sv)->datagram.cursize;
}

int ctest_pr_ext_te_datagram_byte (int side, int i)
{
	return (unsigned char) (side == 0 ? &c_ref_sv : &sv)->datagram_buf[i];
}

int ctest_pr_ext_te_client_datagram_len (int side, int idx0based)
{
	client_t *c = (side == 0 ? ctest_pr_ext_te_clients_c : ctest_pr_ext_te_clients_r) + idx0based;
	return c->datagram.cursize;
}

int ctest_pr_ext_te_client_datagram_byte (int side, int idx0based, int i)
{
	client_t *c = (side == 0 ? ctest_pr_ext_te_clients_c : ctest_pr_ext_te_clients_r) + idx0based;
	return (unsigned char) c->datagram_buf[i];
}

int ctest_pr_ext_te_edict_prog (int num)
{
	return ctest_pf_edict_prog (num);
}

/* Dispatch indices 60-99; must match tests/pr_ext_te_differential.rs's
 * `mod pf`. Independent of ctest_cref_pr_ext_which above. */
static int ctest_cref_pr_ext_te_which;

static void ctest_cref_pr_ext_te_dispatch (void *p)
{
	(void)p;
	switch (ctest_cref_pr_ext_te_which)
	{
	case 60:
		PF_sv_te_blooddp ();
		break;
	case 61:
		PF_sv_te_bloodqw ();
		break;
	case 62:
		PF_sv_te_lightningblood ();
		break;
	case 63:
		PF_sv_te_spike ();
		break;
	case 64:
		PF_sv_te_superspike ();
		break;
	case 65:
		PF_sv_te_gunshot ();
		break;
	case 66:
		PF_sv_te_explosion ();
		break;
	case 67:
		PF_sv_te_tarexplosion ();
		break;
	case 68:
		PF_sv_te_lightning1 ();
		break;
	case 69:
		PF_sv_te_lightning2 ();
		break;
	case 70:
		PF_sv_te_wizspike ();
		break;
	case 71:
		PF_sv_te_knightspike ();
		break;
	case 72:
		PF_sv_te_lightning3 ();
		break;
	case 73:
		PF_sv_te_lavasplash ();
		break;
	case 74:
		PF_sv_te_teleport ();
		break;
	case 75:
		PF_sv_te_beam ();
		break;
	case 76:
		PF_sv_te_explosion2 ();
		break;
	case 77:
		PF_sv_te_particlerain ();
		break;
	case 78:
		PF_sv_te_particlesnow ();
		break;
	default:
		Sys_Error ("ctest_cref_pr_ext_te_run: bad index %d", ctest_cref_pr_ext_te_which);
	}
}

/* Returns the Host_Guard status (0 = ok), same convention as
 * ctest_cref_pr_ext_run above. */
int ctest_cref_pr_ext_te_run (int which)
{
	ctest_cref_pr_ext_te_which = which;
	return Host_Guard (ctest_cref_pr_ext_te_dispatch, NULL);
}

/* ==== M9F GROUP D END ==== */

/* ==== M9F GROUP E BEGIN ====
 *
 * Phase 7 M9f group E: the six particle builtins (Quake/pr_ext.c:4720-4944),
 * differentially tested against
 * rust/quake-capi/src/progs_builtins_particles.rs. Self-contained the same way
 * groups A-D above are: a private fixture plus a private dispatcher
 * (ctest_m9fe_run, indices 100-106), so it never interleaves with them.
 *
 * WHY THE TWO PRExt_Glue_* TRAMPOLINES ARE DEFINED HERE
 *
 * Quake/pr_ext.c defines PRExt_Glue_SVMulticast (:4388) and
 * PRExt_Glue_EffectinfoEnumerate (:4715) inside #ifdef USE_RUST_HOST, and this
 * oracle TU never defines that macro -- so the #include above compiles the
 * #else arms and neither symbol reaches the ctest link. The Rust port imports
 * both, so they are supplied here, exactly as stubs/sv_send_ref.c:1169
 * supplies SvSend_Glue_WriteBatch for the same reason.
 *
 * They are not plain forwarders, and cannot be. In the real engine there is
 * ONE sv/svs storage and pr_ext.c's static SV_Multicast reads it directly. In
 * this TU there are two: the C oracle's c_ref_sv/c_ref_svs (SV_Multicast was
 * compiled under c_ref_prelude.h's rename) and the Rust-owned plain sv/svs
 * that the port writes into. A Rust-frame caller must see its own storage, so
 * each trampoline aliases the Rust sizebufs/client array into the c_ref
 * structs for the duration of the guarded call and copies the results back --
 * only the fields SV_Multicast, PF_multicast_internal and
 * PF_SV_ForceParticlePrecache actually touch -- which includes datagram and
 * reliable_datagram, because PF_multicast_internal's requireext2 == 0 arm
 * (pr_ext.c:4283) writes those instead of fanning out per client.
 * The sizebuf_t assignments carry the data pointer, so the fanout writes land
 * in the Rust buffers themselves rather than in a copy. A whole-struct copy
 * would be wrong: server_t embeds its buffers inline, so c_ref_sv's copies
 * would go stale and copying back would clobber the bytes under test.
 *
 * WHY cl NEEDS ITS OWN #undef HERE
 *
 * Group D's block above already turned sv/svs back into their plain
 * (Rust-owned) spellings and everything below inherits that. cl is the same
 * situation for the three PF_cl_* builtins -- c_ref_prelude.h:1744 renamed it
 * for the whole TU, so the oracle bodies read c_ref_cl (Quake/cl_main.c under
 * the prelude) while quake-capi's port reads the plain cl
 * (quake-capi/src/cl_main.rs, T7.4). Undone here so this fixture seeds both.
 *
 * WHY pr_checkextension NEEDS ONE TOO
 *
 * pr_ext.c:51 defines it, so this file's own rename block (:78) turned that
 * definition into c_ref_pr_checkextension; the Rust port reads the plain
 * cvar_t that stubs.c:3579 owns. Both are zero-initialised, but
 * PF_sv_particleeffectnum's warning arm is gated on .value, so the fixture
 * sets both from one parameter rather than relying on that.
 *
 * SCOPE
 *
 * All six builtins are dispatched (100-105), plus the warn-counter reset (106)
 * that PR_ShutdownExtensions reaches through PR_RSH_ResetParticleWarnCount.
 * r_particledesc (:223 above), host_frametime (stubs.c:2718) and the PScript_*
 * recorders (stubs.c:7538/:7551) are NOT prelude-renamed, so both sides share
 * them: they are inputs and instruments, not part of what is compared.
 */

#undef cl
#undef pr_checkextension

extern client_state_t cl;			 /* Rust: quake-capi/src/cl_main.rs (T7.4) */
extern client_state_t c_ref_cl;		 /* oracle: Quake/cl_main.c under the prelude */
extern cvar_t		  pr_checkextension; /* stubs.c:3579 -- the Rust side's copy */

extern void ctest_pscript_reset (void);

#define CTEST_M9FE_CLIENTS 2

static client_t ctest_m9fe_clients_r[CTEST_M9FE_CLIENTS];
static client_t ctest_m9fe_clients_c[CTEST_M9FE_CLIENTS];

/* One shared filler name for the overflow fixtures: non-NULL, and distinct
 * from every probe name the differential passes in. */
static const char ctest_m9fe_filler[] = "ctest_m9fe_filler";

/* ---------------------------------------------------------------------------
 * The two ADR-009 trampolines (see the header comment).
 */

typedef struct
{
	sizebuf_t	   multicast;
	sizebuf_t	   signon;
	sizebuf_t	   datagram;
	sizebuf_t	   reliable_datagram;
	server_state_t state;
	unsigned	   protocolflags;
	int			   maxclients;
	client_t	  *clients;
} ctest_m9fe_svview_t;

static ctest_m9fe_svview_t ctest_m9fe_swap_in (void)
{
	ctest_m9fe_svview_t saved;

	saved.multicast = c_ref_sv.multicast;
	saved.signon = c_ref_sv.signon;
	saved.datagram = c_ref_sv.datagram;
	saved.reliable_datagram = c_ref_sv.reliable_datagram;
	saved.state = c_ref_sv.state;
	saved.protocolflags = c_ref_sv.protocolflags;
	saved.maxclients = c_ref_svs.maxclients;
	saved.clients = c_ref_svs.clients;

	c_ref_sv.multicast = sv.multicast;
	c_ref_sv.signon = sv.signon;
	c_ref_sv.datagram = sv.datagram;
	c_ref_sv.reliable_datagram = sv.reliable_datagram;
	c_ref_sv.state = sv.state;
	c_ref_sv.protocolflags = sv.protocolflags;
	c_ref_svs.maxclients = svs.maxclients;
	c_ref_svs.clients = svs.clients;

	return saved;
}

static void ctest_m9fe_swap_out (const ctest_m9fe_svview_t *saved)
{
	sv.multicast = c_ref_sv.multicast;
	sv.signon = c_ref_sv.signon;
	sv.datagram = c_ref_sv.datagram;
	sv.reliable_datagram = c_ref_sv.reliable_datagram;

	c_ref_sv.multicast = saved->multicast;
	c_ref_sv.signon = saved->signon;
	c_ref_sv.datagram = saved->datagram;
	c_ref_sv.reliable_datagram = saved->reliable_datagram;
	c_ref_sv.state = saved->state;
	c_ref_sv.protocolflags = saved->protocolflags;
	c_ref_svs.maxclients = saved->maxclients;
	c_ref_svs.clients = saved->clients;
}

typedef struct
{
	int			 to;
	float		*org;
	int			 msg_entity;
	unsigned int requireext2;
} ctest_m9fe_mc_arg_t;

static void ctest_m9fe_invoke_multicast (void *p)
{
	ctest_m9fe_mc_arg_t *a = (ctest_m9fe_mc_arg_t *)p;
	SV_Multicast ((multicast_t)a->to, a->org, a->msg_entity, a->requireext2);
}

int PRExt_Glue_SVMulticast (int to, float *org, int msg_entity, unsigned int requireext2)
{
	ctest_m9fe_mc_arg_t a;
	ctest_m9fe_svview_t saved;
	int					status;

	a.to = to;
	a.org = org;
	a.msg_entity = msg_entity;
	a.requireext2 = requireext2;

	saved = ctest_m9fe_swap_in ();
	status = Host_Guard (ctest_m9fe_invoke_multicast, &a);
	ctest_m9fe_swap_out (&saved);
	return status;
}

static const char *ctest_m9fe_saved_precache[MAX_PARTICLETYPES];

static void ctest_m9fe_invoke_effectinfo (void *p)
{
	(void)p;
	COM_Effectinfo_Enumerate (PF_SV_ForceParticlePrecache);
}

int PRExt_Glue_EffectinfoEnumerate (void)
{
	ctest_m9fe_svview_t saved;
	int					status;

	saved = ctest_m9fe_swap_in ();
	memcpy (ctest_m9fe_saved_precache, c_ref_sv.particle_precache, sizeof (ctest_m9fe_saved_precache));
	memcpy (c_ref_sv.particle_precache, sv.particle_precache, sizeof (c_ref_sv.particle_precache));

	status = Host_Guard (ctest_m9fe_invoke_effectinfo, NULL);

	memcpy (sv.particle_precache, c_ref_sv.particle_precache, sizeof (sv.particle_precache));
	memcpy (c_ref_sv.particle_precache, ctest_m9fe_saved_precache, sizeof (ctest_m9fe_saved_precache));
	ctest_m9fe_swap_out (&saved);
	return status;
}

/* ---------------------------------------------------------------------------
 * Fixture.
 */

static void ctest_m9fe_init_sv (server_t *s)
{
	memset (s, 0, sizeof (*s));
	s->datagram.data = s->datagram_buf;
	s->datagram.maxsize = sizeof (s->datagram_buf);
	s->datagram.allowoverflow = true;
	s->multicast.data = s->multicast_buf;
	s->multicast.maxsize = sizeof (s->multicast_buf);
	s->multicast.allowoverflow = true;
	s->signon.data = s->signon_buf;
	s->signon.maxsize = sizeof (s->signon_buf);
	s->signon.allowoverflow = true;
	s->protocolflags = 0;
}

/* Unlike group D's client fixture this also arms message/msgbuf:
 * PF_sv_particleeffectnum broadcasts with MULTICAST_ALL_R, which fans out into
 * svs.clients[i].message rather than .datagram. */
static void ctest_m9fe_init_client (client_t *c)
{
	memset (c, 0, sizeof (*c));
	c->message.data = c->msgbuf;
	c->message.maxsize = sizeof (c->msgbuf);
	c->message.allowoverflow = true;
	c->datagram.data = c->datagram_buf;
	c->datagram.maxsize = sizeof (c->datagram_buf);
	c->datagram.allowoverflow = true;
}

/* Resets the shared world/edict arena (via ctest_pr_ext_reset_fixture), both
 * sv/svs copies, this group's own client array, both cl particle tables and
 * the warn counter. sv_state is a server_state_t (0 == ss_loading), and
 * checkextension seeds both pr_checkextension copies -- together those two are
 * what gate PF_sv_particleeffectnum's Con_Warning and multicast arm. */
void ctest_m9fe_reset (int num_edicts, int maxclients, unsigned int pext2, int sv_state, float checkextension)
{
	int i;

	ctest_pr_ext_reset_fixture (num_edicts);

	ctest_m9fe_init_sv (&sv);
	ctest_m9fe_init_sv (&c_ref_sv);
	sv.state = (server_state_t)sv_state;
	c_ref_sv.state = (server_state_t)sv_state;

	memset (&svs, 0, sizeof (svs));
	memset (&c_ref_svs, 0, sizeof (c_ref_svs));
	for (i = 0; i < CTEST_M9FE_CLIENTS; i++)
	{
		ctest_m9fe_init_client (&ctest_m9fe_clients_r[i]);
		ctest_m9fe_init_client (&ctest_m9fe_clients_c[i]);
		ctest_m9fe_clients_r[i].active = (i < maxclients);
		ctest_m9fe_clients_c[i].active = (i < maxclients);
		ctest_m9fe_clients_r[i].protocol_pext2 = pext2;
		ctest_m9fe_clients_c[i].protocol_pext2 = pext2;
	}
	svs.maxclients = maxclients;
	svs.clients = ctest_m9fe_clients_r;
	c_ref_svs.maxclients = maxclients;
	c_ref_svs.clients = ctest_m9fe_clients_c;

	memset (cl.particle_precache, 0, sizeof (cl.particle_precache));
	memset (cl.local_particle_precache, 0, sizeof (cl.local_particle_precache));
	memset (c_ref_cl.particle_precache, 0, sizeof (c_ref_cl.particle_precache));
	memset (c_ref_cl.local_particle_precache, 0, sizeof (c_ref_cl.local_particle_precache));

	pr_ext_warned_particleeffectnum = 0;
	pr_checkextension.value = checkextension;
	c_ref_pr_checkextension.value = checkextension;

	r_particledesc.string = "classic";
	host_frametime = 0.0;

	ctest_pscript_reset ();
}

/* r_particledesc is this file's own plain cvar_t (:223), shared by both sides;
 * s must outlive the call. */
void ctest_m9fe_set_particledesc (const char *s)
{
	r_particledesc.string = s;
}

void ctest_m9fe_set_host_frametime (double v)
{
	host_frametime = v;
}

/* Seeds identical inputs into both storages; name must outlive the call. */
void ctest_m9fe_set_sv_precache (int idx, const char *name)
{
	sv.particle_precache[idx] = name;
	c_ref_sv.particle_precache[idx] = name;
}

void ctest_m9fe_fill_sv_precache (void)
{
	int i;

	for (i = 1; i < MAX_PARTICLETYPES; i++)
		ctest_m9fe_set_sv_precache (i, ctest_m9fe_filler);
}

void ctest_m9fe_set_cl_precache (int idx, const char *name, int index)
{
	cl.particle_precache[idx].name = name;
	cl.particle_precache[idx].index = index;
	c_ref_cl.particle_precache[idx].name = name;
	c_ref_cl.particle_precache[idx].index = index;
}

void ctest_m9fe_set_cl_local_precache (int idx, const char *name, int index)
{
	cl.local_particle_precache[idx].name = name;
	cl.local_particle_precache[idx].index = index;
	c_ref_cl.local_particle_precache[idx].name = name;
	c_ref_cl.local_particle_precache[idx].index = index;
}

/* Fills both cl tables so PF_CL_ForceParticlePrecache falls through to its
 * return 0 -- the PF_cl_particleeffectnum raise. Filling the LOCAL table is
 * what keeps PScript_FindParticleType (a Sys_Error abort double, stubs.c:7517)
 * off the path: it is only called on the allocating branch. */
void ctest_m9fe_fill_cl_precache (void)
{
	int i;

	for (i = 1; i < MAX_PARTICLETYPES; i++)
	{
		ctest_m9fe_set_cl_precache (i, ctest_m9fe_filler, 0);
		ctest_m9fe_set_cl_local_precache (i, ctest_m9fe_filler, 0);
	}
}

/* side: 0 = C oracle (c_ref_sv/c_ref_cl), 1 = Rust (sv/cl) -- must match
 * tests/pr_ext_particles_differential.rs's Side enum exactly. */
const char *ctest_m9fe_sv_precache (int side, int idx)
{
	return (side == 0 ? &c_ref_sv : &sv)->particle_precache[idx];
}

const char *ctest_m9fe_cl_precache (int side, int idx)
{
	return (side == 0 ? &c_ref_cl : &cl)->particle_precache[idx].name;
}

const char *ctest_m9fe_cl_local_precache (int side, int idx)
{
	return (side == 0 ? &c_ref_cl : &cl)->local_particle_precache[idx].name;
}

int ctest_m9fe_cl_local_precache_index (int side, int idx)
{
	return (side == 0 ? &c_ref_cl : &cl)->local_particle_precache[idx].index;
}

int ctest_m9fe_datagram_len (int side)
{
	return (side == 0 ? &c_ref_sv : &sv)->datagram.cursize;
}

int ctest_m9fe_datagram_byte (int side, int i)
{
	return (unsigned char) (side == 0 ? &c_ref_sv : &sv)->datagram_buf[i];
}

int ctest_m9fe_multicast_len (int side)
{
	return (side == 0 ? &c_ref_sv : &sv)->multicast.cursize;
}

int ctest_m9fe_multicast_byte (int side, int i)
{
	return (unsigned char) (side == 0 ? &c_ref_sv : &sv)->multicast_buf[i];
}

int ctest_m9fe_client_message_len (int side, int idx0based)
{
	client_t *c = (side == 0 ? ctest_m9fe_clients_c : ctest_m9fe_clients_r) + idx0based;
	return c->message.cursize;
}

int ctest_m9fe_client_message_byte (int side, int idx0based, int i)
{
	client_t *c = (side == 0 ? ctest_m9fe_clients_c : ctest_m9fe_clients_r) + idx0based;
	return (unsigned char)c->msgbuf[i];
}

/* The UNRELIABLE half of the fan-out. With a non-zero requireext2 the
 * MULTICAST_PHS_U arm writes svs.clients[i].datagram, not .message
 * (pr_ext.c:4283) -- and MULTICAST_PVS_U lands there too in this fixture,
 * because stubs.c's Mod_LeafPVS returns NULL and takes the same branch. */
int ctest_m9fe_client_datagram_len (int side, int idx0based)
{
	client_t *c = (side == 0 ? ctest_m9fe_clients_c : ctest_m9fe_clients_r) + idx0based;
	return c->datagram.cursize;
}

int ctest_m9fe_client_datagram_byte (int side, int idx0based, int i)
{
	client_t *c = (side == 0 ? ctest_m9fe_clients_c : ctest_m9fe_clients_r) + idx0based;
	return (unsigned char)c->datagram_buf[i];
}

/* The composed TU's own pr_ext.c:52 static -- the C side's half of the
 * once-per-map warning budget. The Rust side's twin is reset through
 * quake_rs_pr_reset_particle_warn_count. */
int ctest_m9fe_warn_count (void)
{
	return pr_ext_warned_particleeffectnum;
}

/* ---------------------------------------------------------------------------
 * Dispatch indices 100-106; must match
 * tests/pr_ext_particles_differential.rs's pf module.
 */

static int ctest_m9fe_which;

static void ctest_m9fe_dispatch (void *p)
{
	(void)p;
	switch (ctest_m9fe_which)
	{
	case 100:
		PF_sv_particleeffectnum ();
		break;
	case 101:
		PF_sv_trailparticles ();
		break;
	case 102:
		PF_sv_pointparticles ();
		break;
	case 103:
		PF_cl_particleeffectnum ();
		break;
	case 104:
		PF_cl_trailparticles ();
		break;
	case 105:
		PF_cl_pointparticles ();
		break;
	case 106:
		/* PR_RSH_ResetParticleWarnCount's non-Rust expansion (pr_ext.c:335). */
		pr_ext_warned_particleeffectnum = 0;
		break;
	default:
		Sys_Error ("ctest_m9fe_run: bad index %d", ctest_m9fe_which);
	}
}

/* Returns the Host_Guard status (0 == ok), same convention as
 * ctest_cref_pr_ext_te_run above. */
int ctest_m9fe_run (int which)
{
	ctest_m9fe_which = which;
	return Host_Guard (ctest_m9fe_dispatch, NULL);
}

/* ==== M9F GROUP E END ==== */
