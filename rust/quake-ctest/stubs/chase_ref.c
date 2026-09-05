/* Phase 7 M7 oracle TU for Quake/chase.c (task T7.2a).
 *
 * Quake/chase.c is an oracle source (build.rs C_SOURCES) and the prelude
 * renames its four cvars and four entry points (c_ref_prelude.h:1464ff), so
 * the oracle half is c_ref_Chase_Init / c_ref_TraceLine /
 * c_ref_Chase_UpdateFor{Client,Drawing} over c_ref_chase_{back,up,right,active}.
 *
 * This file is the plain half, shaped exactly like Quake/chase_glue.c: the
 * four cvars (whose storage stays C-owned so menu.c, gl_rmain.c, cl_input.c
 * and view.c keep reaching them by name), the one ADR-009 Host_Guard
 * trampoline over Cvar_RegisterVariable, the re-raising Chase_Init, and three
 * plain forwards.
 *
 * The differential is genuinely two-sided at every layer:
 *   - Chase_Init: the oracle registers into Quake/cvar.c's list through
 *     c_ref_Cvar_RegisterVariable, the port into quake-capi's own list through
 *     the plain Cvar_RegisterVariable (stubs.c:1848). Two registries, two
 *     parsers, two cvar_t object sets. stubs.c's ctest_m7_linkproof already
 *     proves the oracle side really parses .value in this link.
 *   - TraceLine: the oracle reaches Quake/world.c's c_ref_SV_RecursiveHullCheck
 *     through c_ref_cl.worldmodel, the port reaches quake-capi's plain
 *     SV_RecursiveHullCheck through the plain cl.worldmodel. Both worldmodel
 *     pointers are set to stubs.c's one synthetic room, so the geometry is
 *     shared and only the traversal code differs.
 *   - Chase_UpdateForDrawing: r_refdef is a single shared object (gl_rmain.c is
 *     not an oracle source), so the fixture republishes its inputs before each
 *     side's run and snapshots the result in between.
 *
 * ADR-009: chase.c's only raise site is Cvar_RegisterVariable in Chase_Init,
 * which is a Host_Reraise wrapper under -Duse_rust_cvar. TraceLine reaches
 * only SV_RecursiveHullCheck (whose Sys_Error aborts rather than jumping) and
 * mathlib, and the two update functions reach only TraceLine and mathlib, so
 * none of the three needs a guard.
 */

#include <string.h>

extern int	Host_Guard (void (*fn) (void *), void *arg);
extern void Host_Reraise (int guard_result);

extern void	 ctest_world_reset (int vm_kind, int num_edicts);
extern void *ctest_world_model (void);

extern cvar_t c_ref_chase_back, c_ref_chase_up, c_ref_chase_right, c_ref_chase_active;

/* ---------------------------------------------------------------------------
 * C-visible objects (chase.c:26-29), initializers verbatim from
 * Quake/chase_glue.c.
 */
#undef chase_back
#undef chase_up
#undef chase_right
#undef chase_active

cvar_t chase_back = {"chase_back", "100", CVAR_NONE};
cvar_t chase_up = {"chase_up", "16", CVAR_NONE};
cvar_t chase_right = {"chase_right", "0", CVAR_NONE};
cvar_t chase_active = {"chase_active", "0", CVAR_NONE};

/* ---------------------------------------------------------------------------
 * Plain handles.
 */
#undef cl
#undef Cvar_RegisterVariable
#undef Cvar_FindVar
extern client_state_t cl;
extern void			  Cvar_RegisterVariable (cvar_t *variable);
extern cvar_t		 *Cvar_FindVar (const char *var_name);

extern void quake_rs_chase_update_for_client (void);
extern void quake_rs_chase_update_for_drawing (void);
extern void quake_rs_trace_line (vec3_t start, vec3_t end, vec3_t impact);
extern int	quake_rs_chase_init (void);

/* ---------------------------------------------------------------------------
 * Guarded callback (ADR-009 rule 3), same body as Quake/chase_glue.c.
 */
static void Chase_InvokeRegisterVariable (void *p)
{
	Cvar_RegisterVariable ((cvar_t *)p);
}

int Chase_Glue_RegisterVariable (cvar_t *var)
{
	return Host_Guard (Chase_InvokeRegisterVariable, var);
}

/* ---------------------------------------------------------------------------
 * Plain-named drivers.
 */
#undef Chase_Init
#undef TraceLine
#undef Chase_UpdateForClient
#undef Chase_UpdateForDrawing

void Chase_Init (void)
{
	Host_Reraise (quake_rs_chase_init ());
}

void TraceLine (vec3_t start, vec3_t end, vec3_t impact)
{
	quake_rs_trace_line (start, end, impact);
}

void Chase_UpdateForClient (void)
{
	quake_rs_chase_update_for_client ();
}

void Chase_UpdateForDrawing (void)
{
	quake_rs_chase_update_for_drawing ();
}

/* ===========================================================================
 * THE FIXTURE
 * =========================================================================== */

#define CTEST_CHASE_CVARS 4

static cvar_t *ctest_chase_cvar (int idx, int oracle)
{
	static cvar_t *const plain[CTEST_CHASE_CVARS] = {&chase_back, &chase_up, &chase_right, &chase_active};
	static cvar_t *const ref[CTEST_CHASE_CVARS] = {&c_ref_chase_back, &c_ref_chase_up, &c_ref_chase_right, &c_ref_chase_active};

	if (idx < 0 || idx >= CTEST_CHASE_CVARS)
		return NULL;
	return oracle ? ref[idx] : plain[idx];
}

int ctest_chase_cvar_count (void)
{
	return CTEST_CHASE_CVARS;
}

/* Republishes the synthetic room on BOTH cl copies: ctest_world_reset sets
 * only c_ref_cl (stubs.c re-#defines cl to c_ref_cl around its own
 * definition), so without this the port would trace against a NULL
 * worldmodel while the oracle traced against the room -- both sides would
 * "agree" only by crashing. */
void ctest_chase_reset (void)
{
	ctest_world_reset (0, 2);
	cl.worldmodel = (qmodel_t *)ctest_world_model ();
	c_ref_cl.worldmodel = (qmodel_t *)ctest_world_model ();
}

/* Seeds .value on both sides. Cvar_RegisterVariable does not run for these
 * outside the registration test, so without this every chase cvar would read
 * 0.0 on both halves and Chase_UpdateForDrawing would degenerate to a
 * zero-length trace that passes while measuring nothing. */
void ctest_chase_set_cvars (float back, float up, float right, float active)
{
	int	  i;
	float v[CTEST_CHASE_CVARS];

	v[0] = back;
	v[1] = up;
	v[2] = right;
	v[3] = active;
	for (i = 0; i < CTEST_CHASE_CVARS; i++)
	{
		ctest_chase_cvar (i, 0)->value = v[i];
		ctest_chase_cvar (i, 1)->value = v[i];
	}
}

/* Post-registration observation, one scalar per call so no struct layout has
 * to be mirrored in Rust. `found` proves the object entered that side's own
 * registry: Quake/cvar.c's list for the oracle, quake-capi's for the port. */
int ctest_chase_cvar_found (int idx, int oracle)
{
	const cvar_t *var = ctest_chase_cvar (idx, oracle);
	if (!var)
		return 0;
	return (oracle ? c_ref_Cvar_FindVar (var->name) : Cvar_FindVar (var->name)) == var;
}

float ctest_chase_cvar_value (int idx, int oracle)
{
	const cvar_t *var = ctest_chase_cvar (idx, oracle);
	return var ? var->value : 0.0f;
}

unsigned int ctest_chase_cvar_flags (int idx, int oracle)
{
	const cvar_t *var = ctest_chase_cvar (idx, oracle);
	return var ? var->flags : 0u;
}

const char *ctest_chase_cvar_name (int idx, int oracle)
{
	const cvar_t *var = ctest_chase_cvar (idx, oracle);
	return (var && var->name) ? var->name : "";
}

const char *ctest_chase_cvar_string (int idx, int oracle)
{
	const cvar_t *var = ctest_chase_cvar (idx, oracle);
	return (var && var->string) ? var->string : "";
}

/* Chase_UpdateForDrawing's other two inputs. cl.viewangles and cl.viewent are
 * inline in client_state_t, so unlike cl.worldmodel they really do exist twice
 * and both copies must be written. */
void ctest_chase_set_client (const float *viewangles, const float *viewent_origin)
{
	int i;
	for (i = 0; i < 3; i++)
	{
		cl.viewangles[i] = viewangles[i];
		cl.viewent.origin[i] = viewent_origin[i];
		c_ref_cl.viewangles[i] = viewangles[i];
		c_ref_cl.viewent.origin[i] = viewent_origin[i];
	}
}

/* r_refdef is one shared object, so the caller sets it, runs one side, reads
 * it back, sets it again and runs the other. */
void ctest_chase_set_refdef (const float *vieworg, const float *viewangles)
{
	int i;
	for (i = 0; i < 3; i++)
	{
		r_refdef.vieworg[i] = vieworg[i];
		r_refdef.viewangles[i] = viewangles[i];
	}
}

void ctest_chase_get_refdef (float *out6)
{
	int i;
	for (i = 0; i < 3; i++)
	{
		out6[i] = r_refdef.vieworg[i];
		out6[3 + i] = r_refdef.viewangles[i];
	}
}
