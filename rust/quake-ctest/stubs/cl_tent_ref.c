/* Phase 7 M7 oracle fixture TU for Quake/cl_tent.c (T7.2b).
 *
 * c_ref_prelude.h is force-included (build.rs) and already includes the real
 * Quake/client.h, so entity_t, beam_t, dlight_t and client_state_t are the
 * engine's own declarations here. Quake/cl_tent.c and Quake/cl_main.c are both
 * oracle sources, so their entry points are reachable as c_ref_<name>.
 *
 * Same three roles cl_input_ref.c and sv_user_ref.c play for their waves:
 *
 *  1. Define the PLAIN (Rust-reading) twins of everything Quake/cl_tent_glue.c
 *     owns -- num_temp_entities, cl_temp_entities[] and cl_beams[] -- plus the
 *     four cl_main.c objects cl_tent.c reaches that have no plain twin
 *     anywhere: cl_visedicts, cl_numvisedicts, cl_maxvisedicts and
 *     CL_AllocDlight. cl_tent_glue.c is gated `#ifdef USE_RUST_HOST` and is
 *     not in build.rs's C_SOURCES, and cl_main.c is an oracle source whose
 *     every symbol is renamed, so without this file none of the seven has a
 *     definition under its plain name (verified: a `cargo test --no-run`
 *     before this file was written reported all seven as unresolved
 *     externals).
 *  2. Re-implement the one ADR-009 trampoline (ClTent_Glue_ModForName), the
 *     six plain ClTent_Glue_* shims and the re-raising CL_ParseTEnt, mirroring
 *     Quake/cl_tent_glue.c's bodies exactly.
 *  3. Provide the fixture seeders and read-backs. Nothing in this link ever
 *     runs CL_Init or CL_NewMap, so cl.entities, cl_visedicts and
 *     cl_maxvisedicts are NULL/0 from static init on BOTH sides -- the "both
 *     sides degenerate identically" shape a bit-exact differential silently
 *     accepts. An unseeded cl_maxvisedicts of 0 in particular makes
 *     CL_NewTempEntity return NULL on its first line on both sides, and the
 *     whole port then looks correct while doing nothing. Every seeder below
 *     therefore writes the c_ref_* copy and the plain copy in the same call,
 *     and ctest_cltent_reset publishes real, non-degenerate values.
 *
 * Callee selection (the rule sv_send_ref.c:1051 records): the ClTent_Glue_*
 * bodies below call the SAME unrenamed helpers the real glue file calls --
 * Mod_ForName, va, nullentitystate -- because all three are single shared
 * stub/engine symbols in this link, not oracle/port pairs. Where a pair does
 * exist (MSG_BeginReading, whose plain name is quake-capi/src/net.rs's export)
 * the fixture drives the matching side explicitly.
 *
 * HARNESS-ONLY RAISE HAZARD, stated deliberately: in the shipping engine
 * Sys_Error terminates, so ClTent_Glue_BadTEntType is a plain noreturn shim
 * and CL_ParseTEnt's only ADR-009 frame is the Mod_ForName guard. In THIS
 * harness stubs.c:48-61 makes Sys_Error longjmp when armed, so driving the
 * bad-type arm -- or any abort stub -- through the Rust port longjmps across
 * Rust frames. That is safe only because every driver below enters through
 * Host_Guard, whose setjmp sits in a pure C frame outside the Rust call. It is
 * a property of the harness, not of the port.
 *
 * cl_temp_entities/cl_beams/cl_visedicts are compared as byte images by the
 * suite (ctest_cltent_get_*), which is only sound because every pointer they
 * can hold is shared between the two sides: model handles are this file's own
 * sentinels, and netstate is copied from the single shared nullentitystate.
 */

#include <string.h>

/* Host_Guard/Host_Reraise live in stubs.c and are not declared by any header
 * the prelude pulls in (the real engine declares them via host.h), same as
 * sv_user_ref.c/cl_input_ref.c/pf_cl_ref.c. */
extern int	Host_Guard (void (*fn) (void *), void *arg);
extern void Host_Reraise (int guard_result);

/* quake-capi/src/cl_tent.rs's status core. cl_tent_glue.c gets this prototype
 * from the generated quake_rs.h, which this link has no counterpart for. */
extern int quake_rs_cl_parse_tent (void);

/* --------------------------------------------------------------------------
 * Plain (Rust-reading) storage this wave owns.
 *
 * The prelude's rename macros are live in this TU and would rewrite every
 * definition below to c_ref_*, colliding with the real oracle objects compiled
 * from cl_tent.c/cl_main.c (LNK2005), so each name is #undef'd first. Once
 * #undef'd the bare name means the PLAIN copy for the rest of the file;
 * oracle access always spells c_ref_* by hand. The #undef only affects text
 * after it, so the prelude's own (already renamed) declarations of these same
 * names stay valid -- that is what puts c_ref_cl_beams et al. in scope without
 * a hand-written extern.
 */

/* cl_tent.c:25-27 -- the three objects cl_tent_glue.c takes over. cl_main.c,
   cl_demo.c and host_cmd.c still read them in the shipping build. */
#undef num_temp_entities
#undef cl_temp_entities
#undef cl_beams
int		 num_temp_entities;
entity_t cl_temp_entities[MAX_TEMP_ENTITIES];
beam_t	 cl_beams[MAX_BEAMS];

/* cl_main.c:62-66 -- the visedict list. cl_main.c is a T7.4 file, so these
   keep their C storage in the shipping build; here they need a plain twin
   because the Rust port reads them through quake_c_sys::cl_tent. In the engine
   cl_visedicts points at a Mem_Realloc'd array sized by CL_NewMap; this file
   backs it with a fixed array instead, larger than any bound the suite uses. */
#undef cl_visedicts
#undef cl_numvisedicts
#undef cl_maxvisedicts
#define CTEST_CLTENT_MAX_VISEDICTS 512
entity_t **cl_visedicts;
int		   cl_numvisedicts;
int		   cl_maxvisedicts;

static entity_t *ctest_cltent_visedict_storage[CTEST_CLTENT_MAX_VISEDICTS];
static entity_t *ctest_cltent_oracle_visedict_storage[CTEST_CLTENT_MAX_VISEDICTS];

/* Neither of these two is declared in any engine header (both are file-scope
   objects other TUs reach only through cl_tent.c itself), so the prelude has
   no renamed declaration to inherit and they are spelled out by hand. */
extern int		 c_ref_num_temp_entities;
extern entity_t *c_ref_CL_NewTempEntity (void);

/* stubs.c owns the plain `cl` (its DUPLICATE-SYMBOL HAZARD block, :2657-2700);
   cl_main.c defines c_ref_cl. Both are read here, so the rename must be off
   and both spellings written out. */
#undef cl
extern client_state_t cl;

/* sv_user_ref.c owns the plain net_message/msg_readcount/msg_badread trio and
   ctest_svuser_load_message, which seeds BOTH sides' buffers; this suite
   reuses it rather than defining a second one -- every stub object links into
   every test binary, so a symbol may be defined only once across all of
   them. */
#undef msg_readcount
#undef msg_badread
extern int		msg_readcount;
extern qboolean msg_badread;

/* Entry points the drivers below call on the plain side. All four are
   #[no_mangle] exports of quake-capi/src/cl_tent.rs except CL_ParseTEnt, which
   this file defines. */
#undef CL_AllocDlight
#undef CL_InitTEnts
#undef CL_NewTempEntity
#undef CL_ParseTEnt
#undef CL_UpdateTEnts
#undef MSG_BeginReading

/* Their only declarations came from headers the prelude had already renamed,
   so after the #undef the plain spellings have none. For the void-returning
   ones that is merely untidy; for CL_NewTempEntity it is a correctness bug --
   an implicit declaration returns int, truncating the returned entity_t* and
   turning every index derived from it into garbage. */
entity_t *CL_NewTempEntity (void);
void	  CL_InitTEnts (void);
void	  CL_UpdateTEnts (void);
void	  MSG_BeginReading (void);

/* --------------------------------------------------------------------------
 * CL_AllocDlight (cl_main.c:361-403), hand-transcribed.
 *
 * cl_main.c is an oracle source, so only c_ref_CL_AllocDlight exists; the Rust
 * port calls the plain name. The body is copied line for line, including the
 * quirk that the final fallback slot does NOT clear kex_intensity while the
 * other two exits do. Transcribing that faithfully is the point: a tidied
 * version would diverge the moment a test drives the fallback.
 *
 * The table is private to this file rather than a plain `cl_dlights`: nothing
 * in the Rust port names cl_dlights, so a plain twin would be an unreferenced
 * global, and a private name cannot be caught by the prelude's rename.
 */
static dlight_t ctest_cltent_dlights[MAX_DLIGHTS];

dlight_t *CL_AllocDlight (int key)
{
	int		  i;
	dlight_t *dl;

	// first look for an exact key match
	if (key)
	{
		dl = ctest_cltent_dlights;
		for (i = 0; i < MAX_DLIGHTS; i++, dl++)
		{
			if (dl->key == key)
			{
				memset (dl, 0, sizeof (*dl));
				dl->key = key;
				dl->color[0] = dl->color[1] = dl->color[2] = 1;
				dl->cone_cos = -2.0f;
				dl->kex_intensity = 0.0f;
				return dl;
			}
		}
	}

	// then look for anything else
	dl = ctest_cltent_dlights;
	for (i = 0; i < MAX_DLIGHTS; i++, dl++)
	{
		if (dl->die < cl.time)
		{
			memset (dl, 0, sizeof (*dl));
			dl->key = key;
			dl->color[0] = dl->color[1] = dl->color[2] = 1;
			dl->cone_cos = -2.0f;
			dl->kex_intensity = 0.0f;
			return dl;
		}
	}

	dl = &ctest_cltent_dlights[0];
	memset (dl, 0, sizeof (*dl));
	dl->key = key;
	dl->color[0] = dl->color[1] = dl->color[2] = 1;
	dl->cone_cos = -2.0f;
	return dl;
}

/* --------------------------------------------------------------------------
 * ADR-009 trampoline, mirroring Quake/cl_tent_glue.c:63-84 exactly.
 *
 * Mod_ForName is a single unrenamed stubs.c symbol here (stubs.c:6965) and it
 * always Sys_Errors, so this guard reliably returns CTEST_GUARD_SYS_ERROR in
 * the harness. That is what makes the four TE_LIGHTNING1/2/3 and TE_BEAM cases
 * drivable at all: the differential over them compares WHERE each side stops
 * -- before or after the MSG_ReadEntity/MSG_ReadCoord calls CL_ParseBeam would
 * make -- and with which status, not what a real model load would produce.
 */

typedef struct
{
	const char *name;
	qmodel_t  **out;
} cltent_modforname_arg_t;

static void ClTent_InvokeModForName (void *p)
{
	cltent_modforname_arg_t *a = (cltent_modforname_arg_t *)p;
	*a->out = Mod_ForName (a->name, true);
}

int ClTent_Glue_ModForName (const char *name, qmodel_t **out)
{
	cltent_modforname_arg_t arg;
	arg.name = name;
	arg.out = out;
	*out = NULL;
	return Host_Guard (ClTent_InvokeModForName, &arg);
}

/* --------------------------------------------------------------------------
 * Plain shims, mirroring Quake/cl_tent_glue.c:90-131. None can raise except
 * BadTEntType, whose Sys_Error terminates in the shipping engine.
 */

FUNC_NORETURN void ClTent_Glue_BadTEntType (void)
{
	Sys_Error ("CL_ParseTEnt: bad type");
}

const char *ClTent_Glue_Explosion2Name (int colorStart, int colorLength)
{
	return va ("TE_EXPLOSION2_%i_%i", colorStart, colorLength);
}

void ClTent_Glue_ClearTempEntity (entity_t *ent)
{
	memset (ent, 0, sizeof (*ent));
}

void ClTent_Glue_SetTempEntityNetstate (entity_t *ent)
{
	ent->netstate = nullentitystate;
}

void ClTent_Glue_SetBeamEntity (entity_t *ent, const float *org, qmodel_t *model, float pitch, float yaw, float roll)
{
	VectorCopy (org, ent->origin);
	ent->model = model;
	ent->angles[0] = pitch;
	ent->angles[1] = yaw;
	ent->angles[2] = roll;
}

void ClTent_Glue_GetEntityOrigin (const entity_t *ent, float *out)
{
	VectorCopy (ent->origin, out);
}

/* --------------------------------------------------------------------------
 * Re-raising public entry point (ADR-009), mirroring cl_tent_glue.c:138-142.
 * This is the plain CL_ParseTEnt for the whole link.
 */

void CL_ParseTEnt (void)
{
	int r = quake_rs_cl_parse_tent ();
	Host_Reraise (r);
}

/* --------------------------------------------------------------------------
 * Fixture: model-handle sentinels.
 *
 * beam_t::model and entity_t::model are only compared against NULL and copied,
 * never dereferenced, by anything this suite drives. Both sides get the SAME
 * sentinel address, so the byte-image comparisons of cl_beams[] and
 * cl_temp_entities[] stay meaningful.
 */
#define CTEST_CLTENT_MODELS 4
static void *ctest_cltent_model_slots[CTEST_CLTENT_MODELS];

void *ctest_cltent_model (int idx)
{
	if (idx < 0 || idx >= CTEST_CLTENT_MODELS)
		return NULL;
	return &ctest_cltent_model_slots[idx];
}

/* --------------------------------------------------------------------------
 * Fixture: cl.entities. CL_UpdateTEnts reads cl.entities[cl.viewentity].origin
 * when a beam belongs to the view entity and skips the read entirely when
 * cl.entities is NULL, so both arms need driving. The two sides get separate
 * arrays seeded identically rather than one shared array, so an accidental
 * write by either side shows up as a difference instead of propagating.
 */
#define CTEST_CLTENT_ENTITIES 8
static entity_t ctest_cltent_entities[CTEST_CLTENT_ENTITIES];
static entity_t ctest_cltent_oracle_entities[CTEST_CLTENT_ENTITIES];

void ctest_cltent_attach_entities (int attach)
{
	if (attach)
	{
		cl.entities = ctest_cltent_entities;
		c_ref_cl.entities = ctest_cltent_oracle_entities;
	}
	else
	{
		cl.entities = NULL;
		c_ref_cl.entities = NULL;
	}
}

void ctest_cltent_set_entity_origin (int idx, const float *org)
{
	if (idx < 0 || idx >= CTEST_CLTENT_ENTITIES)
		return;
	VectorCopy (org, ctest_cltent_entities[idx].origin);
	VectorCopy (org, ctest_cltent_oracle_entities[idx].origin);
}

void ctest_cltent_get_entity_origin (int side, int idx, float *out)
{
	if (idx < 0 || idx >= CTEST_CLTENT_ENTITIES)
		return;
	VectorCopy ((side ? ctest_cltent_oracle_entities : ctest_cltent_entities)[idx].origin, out);
}

/* --------------------------------------------------------------------------
 * Fixture: the `cl` fields cl_tent.c reads, published to both sides.
 */

void ctest_cltent_set_time (double time)
{
	cl.time = c_ref_cl.time = time;
}

void ctest_cltent_set_paused (int paused)
{
	cl.paused = c_ref_cl.paused = paused ? true : false;
}

void ctest_cltent_set_viewentity (int viewentity)
{
	cl.viewentity = c_ref_cl.viewentity = viewentity;
}

void ctest_cltent_set_protocol (unsigned int protocolflags, unsigned int pext2)
{
	cl.protocolflags = c_ref_cl.protocolflags = protocolflags;
	cl.protocol_pext2 = c_ref_cl.protocol_pext2 = pext2;
}

/* CL_UpdateBeam's overflow warning is rate-limited off the single shared
   dev_overflows/realtime pair (glquake.h:616-623), so two runs of one
   differential would otherwise interfere: the first sets dev_overflows.beams
   and the second is silently rate-limited into taking the other arm. Tests
   reset it between sides and compare what each side wrote. */
void ctest_cltent_set_overflow_state (double beams, double now)
{
	dev_overflows.beams = beams;
	realtime = now;
}

double ctest_cltent_get_overflow_beams (void)
{
	return dev_overflows.beams;
}

/* --------------------------------------------------------------------------
 * Fixture: visedict list and temp-entity counter.
 */

void ctest_cltent_set_visedicts (int maxvisedicts, int numvisedicts)
{
	if (maxvisedicts < 0)
		maxvisedicts = 0;
	if (maxvisedicts > CTEST_CLTENT_MAX_VISEDICTS)
		maxvisedicts = CTEST_CLTENT_MAX_VISEDICTS;

	memset (ctest_cltent_visedict_storage, 0, sizeof (ctest_cltent_visedict_storage));
	memset (ctest_cltent_oracle_visedict_storage, 0, sizeof (ctest_cltent_oracle_visedict_storage));

	cl_visedicts = ctest_cltent_visedict_storage;
	c_ref_cl_visedicts = ctest_cltent_oracle_visedict_storage;
	cl_maxvisedicts = c_ref_cl_maxvisedicts = maxvisedicts;
	cl_numvisedicts = c_ref_cl_numvisedicts = numvisedicts;
}

int ctest_cltent_get_numvisedicts (int side)
{
	return side ? c_ref_cl_numvisedicts : cl_numvisedicts;
}

/* Index of visedict slot `i` within that side's cl_temp_entities[]: -1 for
   NULL, -2 for a pointer outside the temp-entity array, -3 for an out-of-range
   request. Pointer VALUES differ between the sides by construction (two
   separate arrays), so the comparison has to be positional. */
int ctest_cltent_visedict_index (int side, int i)
{
	entity_t *ent;
	entity_t *base;

	if (i < 0 || i >= CTEST_CLTENT_MAX_VISEDICTS)
		return -3;

	ent = side ? c_ref_cl_visedicts[i] : cl_visedicts[i];
	base = side ? c_ref_cl_temp_entities : cl_temp_entities;
	if (!ent)
		return -1;
	if (ent < base || ent >= base + MAX_TEMP_ENTITIES)
		return -2;
	return (int)(ent - base);
}

void ctest_cltent_set_num_temp_entities (int n)
{
	num_temp_entities = c_ref_num_temp_entities = n;
}

int ctest_cltent_get_num_temp_entities (int side)
{
	return side ? c_ref_num_temp_entities : num_temp_entities;
}

/* --------------------------------------------------------------------------
 * Fixture: beams. Seeded directly rather than through CL_UpdateBeam, because
 * CL_UpdateBeam unconditionally calls the CL_TraceLine abort stub -- its
 * PSET_SCRIPT block is always compiled, quakedef.h:38 defines PSET_SCRIPT
 * unconditionally -- which would end every run before the beam table is
 * reached.
 */

void ctest_cltent_set_beam (int side, int idx, int entity, void *model, float endtime, const float *start, const float *end)
{
	beam_t *b;

	if (idx < 0 || idx >= MAX_BEAMS)
		return;
	b = side ? &c_ref_cl_beams[idx] : &cl_beams[idx];
	b->entity = entity;
	b->model = (qmodel_t *)model;
	b->endtime = endtime;
	VectorCopy (start, b->start);
	VectorCopy (end, b->end);
	b->trailname = NULL;
	b->trailstate = NULL;
}

/* --------------------------------------------------------------------------
 * Byte-image read-back. entity_t carries PSET_SCRIPT-conditional members and
 * an embedded entity_state_t, so field-by-field getters would silently miss
 * whatever they forgot to list; the tests memcmp whole objects instead.
 */

int ctest_cltent_entity_size (void)
{
	return (int)sizeof (entity_t);
}

int ctest_cltent_beam_size (void)
{
	return (int)sizeof (beam_t);
}

int ctest_cltent_dlight_size (void)
{
	return (int)sizeof (dlight_t);
}

void ctest_cltent_get_temp_entity (int side, int idx, void *out)
{
	if (idx < 0 || idx >= MAX_TEMP_ENTITIES)
		return;
	memcpy (out, side ? &c_ref_cl_temp_entities[idx] : &cl_temp_entities[idx], sizeof (entity_t));
}

void ctest_cltent_get_beam (int side, int idx, void *out)
{
	if (idx < 0 || idx >= MAX_BEAMS)
		return;
	memcpy (out, side ? &c_ref_cl_beams[idx] : &cl_beams[idx], sizeof (beam_t));
}

void ctest_cltent_get_dlight (int side, int idx, void *out)
{
	if (idx < 0 || idx >= MAX_DLIGHTS)
		return;
	memcpy (out, side ? &c_ref_cl_dlights[idx] : &ctest_cltent_dlights[idx], sizeof (dlight_t));
}

/* --------------------------------------------------------------------------
 * Fixture: message reading. sv_user_ref.c's ctest_svuser_load_message seeds
 * both net_message buffers; this only rewinds the matching side's read cursor
 * and reads it back, because MSG_BeginReading/msg_readcount are an
 * oracle/port pair.
 */

void ctest_cltent_begin_reading (int side)
{
	if (side)
		c_ref_MSG_BeginReading ();
	else
		MSG_BeginReading ();
}

int ctest_cltent_get_readcount (int side)
{
	return side ? c_ref_msg_readcount : msg_readcount;
}

int ctest_cltent_get_badread (int side)
{
	return (side ? c_ref_msg_badread : msg_badread) ? 1 : 0;
}

/* --------------------------------------------------------------------------
 * Drivers. Every entry point is entered through Host_Guard, so the setjmp that
 * catches an armed Sys_Error/Host_Error always sits in a pure C frame outside
 * the Rust call (see the HARNESS-ONLY RAISE HAZARD note at the top of this
 * file). The return value is the CTEST_GUARD_* status: 0 ok, 1 Host_Error,
 * 2 Sys_Error; the message is readable through stubs.c's
 * ctest_host_error_message()/ctest_sys_error_message().
 */

static void ctest_cltent_invoke_parse (void *p)
{
	if (*(int *)p)
		c_ref_CL_ParseTEnt ();
	else
		CL_ParseTEnt ();
}

int ctest_cltent_parse_tent (int side)
{
	int s = side;
	return Host_Guard (ctest_cltent_invoke_parse, &s);
}

static void ctest_cltent_invoke_update (void *p)
{
	if (*(int *)p)
		c_ref_CL_UpdateTEnts ();
	else
		CL_UpdateTEnts ();
}

int ctest_cltent_update_tents (int side)
{
	int s = side;
	return Host_Guard (ctest_cltent_invoke_update, &s);
}

static void ctest_cltent_invoke_init (void *p)
{
	if (*(int *)p)
		c_ref_CL_InitTEnts ();
	else
		CL_InitTEnts ();
}

int ctest_cltent_init_tents (int side)
{
	int s = side;
	return Host_Guard (ctest_cltent_invoke_init, &s);
}

typedef struct
{
	int		  side;
	entity_t *out;
} cltent_new_arg_t;

static void ctest_cltent_invoke_new (void *p)
{
	cltent_new_arg_t *a = (cltent_new_arg_t *)p;
	a->out = a->side ? c_ref_CL_NewTempEntity () : CL_NewTempEntity ();
}

/* Returns the temp-entity index CL_NewTempEntity handed back, or -1 for NULL.
   The raw pointer is useless across sides (two arrays); the index is exactly
   what lines the two runs up. */
int ctest_cltent_new_temp_entity (int side)
{
	cltent_new_arg_t a;
	entity_t		*base;

	a.side = side;
	a.out = NULL;
	ctest_cltent_invoke_new (&a);

	if (!a.out)
		return -1;
	base = side ? c_ref_cl_temp_entities : cl_temp_entities;
	return (int)(a.out - base);
}

/* --------------------------------------------------------------------------
 * Whole-fixture reset. Publishes a non-degenerate starting state into BOTH
 * copies of everything: an empty beam table, an empty temp-entity table, a
 * live visedict list with room in it, cleared dlights, an attached entity
 * array, and the cl fields that keep the interesting arms reachable
 * (cl.time non-zero so CL_AllocDlight's `dl->die < cl.time` scan picks slot 0,
 * protocolflags 0 so MSG_ReadCoord takes its 16-bit path).
 */
void ctest_cltent_reset (void)
{
	int i;

	memset (cl_beams, 0, sizeof (cl_beams));
	memset (c_ref_cl_beams, 0, sizeof (c_ref_cl_beams));
	memset (cl_temp_entities, 0, sizeof (cl_temp_entities));
	memset (c_ref_cl_temp_entities, 0, sizeof (c_ref_cl_temp_entities));
	memset (ctest_cltent_dlights, 0, sizeof (ctest_cltent_dlights));
	memset (c_ref_cl_dlights, 0, sizeof (c_ref_cl_dlights));
	memset (ctest_cltent_entities, 0, sizeof (ctest_cltent_entities));
	memset (ctest_cltent_oracle_entities, 0, sizeof (ctest_cltent_oracle_entities));

	for (i = 0; i < CTEST_CLTENT_MODELS; i++)
		ctest_cltent_model_slots[i] = 0;

	ctest_cltent_set_num_temp_entities (0);
	ctest_cltent_set_visedicts (256, 0);
	ctest_cltent_attach_entities (1);

	ctest_cltent_set_time (1.0);
	ctest_cltent_set_paused (0);
	ctest_cltent_set_viewentity (1);
	ctest_cltent_set_protocol (0, 0);
	ctest_cltent_set_overflow_state (0.0, 0.0);
}
