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

/* gl_texmgr.c:63, gl_draw.c:36, gl_screen.c:139, sbar.c:439-441 and the
 * COMPILE-ONLY vulkanglobals_t from c_ref_prelude.h. All inert: the code that
 * reads them is Phase 8 (PF_cl_draw*, PF_sb_*). */
unsigned int	d_8to24table[256];
qpic_t		   *pic_nul;
gltexture_t	   *char_texture;
qmutex_t	   *draw_qcvm_mutex;
int				fragsort[MAX_SCOREBOARD];
int				scoreboardlines;
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

void Draw_SubPic (cb_context_t *cbx, float x, float y, float w, float h, qpic_t *pic, float s1, float t1, float s2, float t2, float *rgb, float alpha)
{
	(void)cbx;
	(void)x;
	(void)y;
	(void)w;
	(void)h;
	(void)pic;
	(void)s1;
	(void)t1;
	(void)s2;
	(void)t2;
	(void)rgb;
	(void)alpha;
	ctest_pr_ext_unreached ("Draw_SubPic");
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
