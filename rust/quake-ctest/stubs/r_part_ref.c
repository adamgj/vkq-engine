/* Phase 7 M10f-1 (T10.5) oracle TU for Quake/r_part.c -- the classic particle
 * simulator.
 *
 * WHY THIS FILE COMPOSES r_part.c INSTEAD OF build.rs LISTING IT IN C_SOURCES
 *
 * The prelude's c_ref_* renames are translation-unit-wide by construction: one
 * #define rewrites the definition in the oracle source AND every call site in
 * every other oracle source. For r_part.c that is wrong twice over.
 *
 *   1. Every plain name r_part.c defines is already taken in this link, by
 *      doubles that existing differentials assert on: stubs.c:7548-7601 aborts
 *      on the eight R_*Explosion/Splash/Trail/Clear/Parse/Entity entry points,
 *      stubs/pf_cl_ref.c:393 records R_RunParticleEffect's arguments,
 *      stubs/host_ref.c:336 and :349 abort on CL_RunParticles and
 *      R_InitParticles, and stubs/host_glue_ref.c:252/:298 wrap those last two
 *      in HOST_GUARD_VOID. Those doubles are what cl_tent_differential,
 *      cl_main_differential, cl_demo_differential and host_differential
 *      compare against, so this file must not define a single plain R_*,
 *      CL_RunParticles or Harness_HashParticles name. The port is reached
 *      through its quake_rs_rpart_* cores instead, which is exactly the seam
 *      Quake/r_part_glue.c sits on in the engine build.
 *
 *   2. The pool (r_part.c:38-42) is precisely what the two sides must NOT
 *      share: each half needs its own particle array and its own free list, so
 *      that a divergence in one spawn shows up in every later frame.
 *
 * So the rename layer for r_part.c lives HERE, in r_part.c's own TU, where it
 * renames r_part.c's definitions and r_part.c's internal calls and nothing
 * else.
 *
 * TWO RENAMES ARE NOT SPELLED c_ref_<name>:
 *
 *   r_particles / r_quadparticles -> c_ref_rpart_r_particles /
 *     c_ref_rpart_r_quadparticles. stubs/menu_ref.c:725-726 already defines
 *     BOTH c_ref_r_particles and the plain r_particles (menu.c reads the cvar,
 *     so the menu oracle needed a pair), and menu_ref.c:873 asserts on them.
 *     Renaming r_part.c's definition to c_ref_r_particles would be a duplicate
 *     symbol; the plain r_particles the port reads is menu_ref.c's, which is
 *     correct -- one plain cvar object per link, as in the engine build.
 *     r_quadparticles has no such twin, so the plain one is defined below.
 *
 * WHAT IS SHARED AND WHAT IS PER SIDE
 *
 * Per side, because the prelude renamed it and the two halves own disjoint
 * copies: everything r_part.c defines (pool, free list, ramps, avelocities,
 * R_RocketTrail's function-local tracercount), cl / cls, sv_gravity, the
 * MSG_Read* cursor, vec3_origin and the VectorNormalize/VectorScale pair.
 *
 * Shared, and therefore re-seeded or cleared by the test between the two
 * sides: COM_Rand (stubs.c:233-262), the Con_Printf capture log
 * (stubs/console_ref.c:483 -> stubs.c's ctest_con_log), com_argc/com_argv,
 * Mem_Alloc, no_rendering (stubs/host_ref.c:330, true), r_avertexnormals and
 * Harness_Hash64 -- the last two defined below, because nothing else in this
 * link defines either and both are pure functions of their input.
 *
 * ADR-009. r_part.c's raise surface is four Con_Printf calls (plain and
 * unguarded, per the standing project decision), three MSG_Read* calls (which
 * only set msg_badread), one COM_FOpenFile, and -- the one that matters --
 * two Cvar_RegisterVariable calls at r_part.c:247 and :249. Under
 * -Duse_rust_cvar the plain Cvar_RegisterVariable is itself a Host_Reraise
 * wrapper (Quake/cvar_cmd_glue.c), so it can longjmp past a Rust frame.
 * Quake/r_part_glue.c therefore carries one trampoline,
 * RPart_Glue_RegisterVariable; this file mirrors it, and ctest_rpart_init
 * enters both sides through Host_Guard so a raise is a comparable result.
 * Every other entry point is called plainly on both sides.
 *
 * COST, stated so it is not discovered later:
 * scripts/harness/check_ctest_symbols.sh reads C_SOURCES out of build.rs, so
 * it does not inspect this object; build.rs watches Quake/r_part.c explicitly
 * instead. A missed rename below shows up only as a duplicate-symbol link
 * error, so the block is kept in step with r_part.c by hand.
 *
 * NOT OBSERVABLE HERE. r_part.c's rendering half (r_part.c:54-221, :951-1106)
 * is compiled -- a #include cannot take half a file -- but never runs:
 * no_rendering is true, so R_InitParticles skips R_InitParticleTextures and
 * R_InitParticleIndexBuffer, and nothing calls R_DrawParticles*. The Vulkan
 * surface it names is declared COMPILE-ONLY in the prelude's "Phase 7 M10f-1"
 * block. Consequently particletexture/texturescalefactor and the
 * R_SetParticleTexture_f cvar callback are compared only as far as the seam:
 * ctest_rpart_glue_texcb_calls counts the callback the port hands to
 * Cvar_SetCallback, and ctest_rpart_glue_initrender_calls counts the render
 * tail the port hands back to its C frame. That is the whole of the rendering
 * half's observability, and it is a Phase 8 gap, not a Phase 7 one.
 */

#include "quakedef.h"

/* ---- r_part.c rename block ----------------------------------------------
 * Every file-scope symbol Quake/r_part.c defines. The statics do not collide,
 * but they are renamed with the rest so the block can be audited against one
 * grep of r_part.c instead of against two lists.
 */

/* data (r_part.c:34-52, :266-272) */
#define ramp1				   c_ref_ramp1
#define ramp2				   c_ref_ramp2
#define ramp3				   c_ref_ramp3
#define active_particles	   c_ref_active_particles
#define free_particles		   c_ref_free_particles
#define particles			   c_ref_particles
#define r_numparticles		   c_ref_r_numparticles
#define particletexture		   c_ref_particletexture
#define particletexture1	   c_ref_particletexture1
#define particletexture2	   c_ref_particletexture2
#define particletexture3	   c_ref_particletexture3
#define particletexture4	   c_ref_particletexture4
#define texturescalefactor	   c_ref_texturescalefactor
#define r_particles			   c_ref_rpart_r_particles	   /* see the header */
#define r_quadparticles		   c_ref_rpart_r_quadparticles /* see the header */
#define particle_index_buffer  c_ref_particle_index_buffer
#define avelocities			   c_ref_avelocities
#define beamlength			   c_ref_beamlength
#define avelocity			   c_ref_avelocity
#define partstep			   c_ref_partstep
#define timescale			   c_ref_timescale

/* functions with external linkage (r_part.c:59-1106) */
#define R_ParticleTextureLookup	 c_ref_R_ParticleTextureLookup
#define R_InitParticleTextures	 c_ref_R_InitParticleTextures
#define R_InitParticleIndexBuffer c_ref_R_InitParticleIndexBuffer
#define R_InitParticles			 c_ref_R_InitParticles
#define R_EntityParticles		 c_ref_R_EntityParticles
#define R_ClearParticles		 c_ref_R_ClearParticles
#define R_ReadPointFile_f		 c_ref_R_ReadPointFile_f
#define R_ParseParticleEffect	 c_ref_R_ParseParticleEffect
#define R_ParticleExplosion		 c_ref_R_ParticleExplosion
#define R_ParticleExplosion2	 c_ref_R_ParticleExplosion2
#define R_BlobExplosion			 c_ref_R_BlobExplosion
#define R_RunParticleEffect		 c_ref_R_RunParticleEffect
#define R_LavaSplash			 c_ref_R_LavaSplash
#define R_TeleportSplash		 c_ref_R_TeleportSplash
#define R_RocketTrail			 c_ref_R_RocketTrail
#define CL_RunParticles			 c_ref_CL_RunParticles
#define Harness_HashParticles	 c_ref_Harness_HashParticles
#define R_DrawParticles			 c_ref_R_DrawParticles
#define R_DrawParticles_ShowTris c_ref_R_DrawParticles_ShowTris

/* statics (r_part.c:135, :956) */
#define R_SetParticleTexture_f c_ref_R_SetParticleTexture_f
#define R_DrawParticlesFaces   c_ref_R_DrawParticlesFaces

/* The prelude declares the plain spellings of eleven of these (its glquake.h
 * client slice), so after the renames above r_part.c's definitions would have
 * no visible prototype. Re-declaring them here costs nothing -- the macros
 * rewrite each line -- and keeps the oracle build warning-clean. */
int		 R_ParticleTextureLookup (int x, int y, int sharpness);
void	 R_InitParticleTextures (void);
void	 R_InitParticleIndexBuffer (void);
void	 R_InitParticles (void);
void	 R_EntityParticles (entity_t *ent);
void	 R_ClearParticles (void);
void	 R_ReadPointFile_f (void);
void	 R_ParseParticleEffect (void);
void	 R_ParticleExplosion (vec3_t org);
void	 R_ParticleExplosion2 (vec3_t org, int colorStart, int colorLength);
void	 R_BlobExplosion (vec3_t org);
void	 R_RunParticleEffect (vec3_t org, vec3_t dir, int color, int count);
void	 R_LavaSplash (vec3_t org);
void	 R_TeleportSplash (vec3_t org);
void	 R_RocketTrail (vec3_t start, vec3_t end, int type);
void	 CL_RunParticles (void);
uint64_t Harness_HashParticles (uint64_t h);
void	 R_DrawParticles (cb_context_t *cbx);
void	 R_DrawParticles_ShowTris (cb_context_t *cbx);

#include "r_part.c"

/* =========================================================================
 * THE PLAIN HALF -- the ctest-link mirror of Quake/r_part_glue.c
 * ========================================================================= */

#undef ramp1
#undef ramp2
#undef ramp3
#undef active_particles
#undef free_particles
#undef particles
#undef r_numparticles
#undef particletexture
#undef particletexture1
#undef particletexture2
#undef particletexture3
#undef particletexture4
#undef texturescalefactor
#undef r_particles
#undef r_quadparticles
#undef particle_index_buffer
#undef avelocities
#undef beamlength
#undef avelocity
#undef partstep
#undef timescale
#undef R_ParticleTextureLookup
#undef R_InitParticleTextures
#undef R_InitParticleIndexBuffer
#undef R_InitParticles
#undef R_EntityParticles
#undef R_ClearParticles
#undef R_ReadPointFile_f
#undef R_ParseParticleEffect
#undef R_ParticleExplosion
#undef R_ParticleExplosion2
#undef R_BlobExplosion
#undef R_RunParticleEffect
#undef R_LavaSplash
#undef R_TeleportSplash
#undef R_RocketTrail
#undef CL_RunParticles
#undef Harness_HashParticles
#undef R_DrawParticles
#undef R_DrawParticles_ShowTris
#undef R_SetParticleTexture_f
#undef R_DrawParticlesFaces
#undef cl
#undef cls
#undef sv_gravity
#undef Cvar_RegisterVariable

/* Storage Quake/r_part_glue.c owns in the engine build (ADR-007: the
 * rendering half stayed C, so a live C reader survives the port). The port
 * externs exactly these four through rust/quake-c-sys/src/r_part.rs. */
particle_t *active_particles, *free_particles, *particles;
int			r_numparticles;

/* r_part.c:48, initializer verbatim. The r_particles twin is menu_ref.c:726. */
cvar_t r_quadparticles = {"r_quadparticles", "1", CVAR_ARCHIVE};

/* gl_rmain.c:92, initializer verbatim. r_part.c:50 externs it and the renderer
 * is not in this link, so one shared object serves both halves; only the
 * rendering half reads it, and the rendering half never runs. */
cvar_t r_showtris = {"r_showtris", "0", CVAR_NONE};

extern cvar_t r_particles;			/* stubs/menu_ref.c:726 */
extern client_state_t  cl;			/* quake-capi/src/cl_main.rs owns it */
extern client_static_t cls;			/* likewise */
extern cvar_t		   sv_gravity;	/* quake-capi/src/sv_main.rs owns it */
extern void			   Cvar_RegisterVariable (cvar_t *variable);

extern int	Host_Guard (void (*fn) (void *), void *arg);
extern void Host_Reraise (int guard_result);

/* r_alias.c:38 builds this table from anorms.h; the renderer is not in this
 * link and r_part.c:267 is its only reader here, so one shared copy serves
 * both halves. It is constant data, so sharing it cannot mask a divergence. */
float r_avertexnormals[162][3] = {
#include "anorms.h"
};

/* Quake/harness.c:89-98, verbatim. harness.c is not an oracle source and no
 * stub defines this; both halves reach this one copy, which is what makes
 * ctest_rpart_hash a real cross-side comparison of the pool rather than of
 * two hash functions. */
#define HARNESS_HASH_PRIME UINT64_C (0x100000001b3) /* harness.c:65 */

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

/* ---------------------------------------------------------------------------
 * Link doubles for the rendering half's Vulkan surface. r_part.c:159-221's
 * R_InitParticleIndexBuffer names all thirteen; no_rendering is true here
 * (stubs/host_ref.c:330) so R_InitParticles never calls it, and nothing else
 * in this link calls it either. Each function aborts rather than returning a
 * plausible value, so "never runs" is checked rather than assumed.
 */

static void ctest_rpart_no_renderer (const char *what)
{
	Sys_Error ("ctest: %s reached -- r_part.c's rendering half is not linked here", what);
}

atomic_uint32_t rs_particles;
atomic_uint32_t num_vulkan_dynbuf_allocations;
atomic_uint64_t total_device_vulkan_allocation_size;

VkResult vkCreateBuffer (VkDevice device, const VkBufferCreateInfo *create_info, const void *allocator, VkBuffer *buffer)
{
	(void)device;
	(void)create_info;
	(void)allocator;
	(void)buffer;
	ctest_rpart_no_renderer ("vkCreateBuffer");
	return VK_SUCCESS;
}

void vkGetBufferMemoryRequirements (VkDevice device, VkBuffer buffer, VkMemoryRequirements *requirements)
{
	(void)device;
	(void)buffer;
	(void)requirements;
	ctest_rpart_no_renderer ("vkGetBufferMemoryRequirements");
}

VkResult vkAllocateMemory (VkDevice device, const VkMemoryAllocateInfo *allocate_info, const void *allocator, VkDeviceMemory *memory)
{
	(void)device;
	(void)allocate_info;
	(void)allocator;
	(void)memory;
	ctest_rpart_no_renderer ("vkAllocateMemory");
	return VK_SUCCESS;
}

VkResult vkBindBufferMemory (VkDevice device, VkBuffer buffer, VkDeviceMemory memory, VkDeviceSize offset)
{
	(void)device;
	(void)buffer;
	(void)memory;
	(void)offset;
	ctest_rpart_no_renderer ("vkBindBufferMemory");
	return VK_SUCCESS;
}

void vkCmdCopyBuffer (VkCommandBuffer cb, VkBuffer src, VkBuffer dst, uint32_t region_count, const VkBufferCopy *regions)
{
	(void)cb;
	(void)src;
	(void)dst;
	(void)region_count;
	(void)regions;
	ctest_rpart_no_renderer ("vkCmdCopyBuffer");
}

void GL_SetObjectName (uint64_t object, VkObjectType object_type, const char *name)
{
	(void)object;
	(void)object_type;
	(void)name;
	ctest_rpart_no_renderer ("GL_SetObjectName");
}

int GL_MemoryTypeFromProperties (uint32_t type_bits, VkFlags requirements_mask, VkFlags preferred_mask)
{
	(void)type_bits;
	(void)requirements_mask;
	(void)preferred_mask;
	ctest_rpart_no_renderer ("GL_MemoryTypeFromProperties");
	return 0;
}

byte *R_StagingAllocate (int size, int alignment, VkCommandBuffer *cb_context, VkBuffer *buffer, int *buffer_offset)
{
	(void)size;
	(void)alignment;
	(void)cb_context;
	(void)buffer;
	(void)buffer_offset;
	ctest_rpart_no_renderer ("R_StagingAllocate");
	return NULL;
}

void R_StagingBeginCopy (void)
{
	ctest_rpart_no_renderer ("R_StagingBeginCopy");
}

void R_StagingEndCopy (void)
{
	ctest_rpart_no_renderer ("R_StagingEndCopy");
}

/* ---------------------------------------------------------------------------
 * The glue seam: the three RPart_Glue_* entry points the Rust core calls back
 * into. Quake/r_part_glue.c implements them against the rendering half; here
 * they are recorders, because the rendering half never runs (see the header).
 */

static int	   ctest_rpart_initrender_calls;
static int	   ctest_rpart_texcb_calls;
static cvar_t *ctest_rpart_texcb_var;

void RPart_Glue_InitRender (void)
{
	ctest_rpart_initrender_calls++;
	if (!no_rendering)
		Sys_Error ("ctest: RPart_Glue_InitRender reached the renderer (no_rendering is false)");
}

void RPart_Glue_SetParticleTexture_f (cvar_t *var)
{
	ctest_rpart_texcb_calls++;
	ctest_rpart_texcb_var = var;
}

static void ctest_rpart_invoke_register (void *p)
{
	Cvar_RegisterVariable ((cvar_t *)p);
}

int RPart_Glue_RegisterVariable (cvar_t *var)
{
	return Host_Guard (ctest_rpart_invoke_register, var);
}

/* Mirror of Quake/r_part_glue.c's fscanf shim. */
int RPart_Glue_ScanPoint (FILE *f, vec3_t org)
{
	return fscanf (f, "%f %f %f\n", &org[0], &org[1], &org[2]);
}

/* =========================================================================
 * THE FIXTURE
 *
 * `side` is 1 for the C oracle (c_ref_*) and 0 for the Rust port (plain), the
 * convention stubs/console_ref.c, stubs/keys_ref.c and stubs/sbar_ref.c use.
 * ========================================================================= */

extern int		   c_ref_r_numparticles;
extern particle_t *c_ref_particles, *c_ref_active_particles, *c_ref_free_particles;
extern cvar_t	   c_ref_rpart_r_particles, c_ref_rpart_r_quadparticles;
extern client_state_t  c_ref_cl;
extern client_static_t c_ref_cls;
extern cvar_t		   c_ref_sv_gravity;

/* The port's cores. cbindgen cannot spell vec3_t / entity_t *, so the engine
 * build reaches these through Quake/r_part_glue.c's plain wrappers; this link
 * has no plain wrappers to offer (see the header), so the drivers below call
 * the cores directly, exactly as r_part_glue.c does. */
extern int		quake_rs_rpart_init_particles (void);
extern void		quake_rs_rpart_entity_particles (const float *origin);
extern void		quake_rs_rpart_clear_particles (void);
extern void		quake_rs_rpart_read_point_file_f (void);
extern void		quake_rs_rpart_parse_particle_effect (void);
extern void		quake_rs_rpart_particle_explosion (float *org);
extern void		quake_rs_rpart_particle_explosion2 (float *org, int colorStart, int colorLength);
extern void		quake_rs_rpart_blob_explosion (float *org);
extern void		quake_rs_rpart_run_particle_effect (float *org, float *dir, int color, int count);
extern void		quake_rs_rpart_lava_splash (float *org);
extern void		quake_rs_rpart_teleport_splash (float *org);
extern void		quake_rs_rpart_rocket_trail (float *start, float *end, int type);
extern void		quake_rs_rpart_run_particles (void);
extern uint64_t quake_rs_rpart_hash_particles (uint64_t h);

/* ---- pool ---------------------------------------------------------------
 * Two fixed arrays rather than Mem_Alloc, so a test can resize the pool as
 * often as it likes without growing the harness heap. ctest_rpart_alloc is
 * the mirror of R_InitParticles' allocation step only; the tests still drive
 * R_ClearParticles themselves, because the free-list layout it builds is part
 * of what is compared.
 */

#define CTEST_RPART_MAX_PARTICLES 2048

static particle_t ctest_rpart_pool[CTEST_RPART_MAX_PARTICLES];
static particle_t ctest_rpart_oracle_pool[CTEST_RPART_MAX_PARTICLES];

void ctest_rpart_alloc (int n)
{
	if (n < 1 || n > CTEST_RPART_MAX_PARTICLES)
		Sys_Error ("ctest_rpart_alloc: %i out of range", n);

	memset (ctest_rpart_pool, 0, sizeof (ctest_rpart_pool));
	memset (ctest_rpart_oracle_pool, 0, sizeof (ctest_rpart_oracle_pool));

	particles = ctest_rpart_pool;
	r_numparticles = n;
	active_particles = free_particles = NULL;

	c_ref_particles = ctest_rpart_oracle_pool;
	c_ref_r_numparticles = n;
	c_ref_active_particles = c_ref_free_particles = NULL;
}

/* Drops both pools entirely, so Harness_HashParticles' `if (!particles)`
 * early-out is reachable. */
void ctest_rpart_drop_pool (void)
{
	particles = c_ref_particles = NULL;
	r_numparticles = c_ref_r_numparticles = 0;
	active_particles = free_particles = NULL;
	c_ref_active_particles = c_ref_free_particles = NULL;
}

int ctest_rpart_numparticles (int side)
{
	return side ? c_ref_r_numparticles : r_numparticles;
}

typedef struct
{
	int	  index;
	float org[3];
	float vel[3];
	float color;
	float ramp;
	float die;
	int	  type;
} ctest_rpart_rec_t;

/* The Rust side mirrors ctest_rpart_rec_t with #[repr(C)]; this is the ABI
 * check that keeps the two spellings honest. */
int ctest_rpart_rec_size (void)
{
	return (int)sizeof (ctest_rpart_rec_t);
}

static int ctest_rpart_walk (particle_t *head, particle_t *base, ctest_rpart_rec_t *out, int max)
{
	int n = 0;

	for (; head; head = head->next)
	{
		if (n < max && out)
		{
			out[n].index = (int)(head - base);
			memcpy (out[n].org, head->org, sizeof (out[n].org));
			memcpy (out[n].vel, head->vel, sizeof (out[n].vel));
			out[n].color = head->color;
			out[n].ramp = head->ramp;
			out[n].die = head->die;
			out[n].type = (int)head->type;
		}
		n++;
		if (n > CTEST_RPART_MAX_PARTICLES)
			Sys_Error ("ctest_rpart_walk: cycle in the particle list");
	}
	return n;
}

/* The active list, in traversal order -- the same order Harness_HashParticles
 * hashes and R_DrawParticlesFaces draws. Returns the full length even when it
 * exceeds `max`, so a truncated read is visible to the caller. */
int ctest_rpart_active (int side, ctest_rpart_rec_t *out, int max)
{
	return side ? ctest_rpart_walk (c_ref_active_particles, c_ref_particles, out, max)
				: ctest_rpart_walk (active_particles, particles, out, max);
}

/* The free list, as pool indices, in traversal order. This is the half of the
 * allocator state the active list cannot show: recycling order is LIFO, so
 * the sequence here is what decides which slot the next spawn takes. */
int ctest_rpart_free (int side, int *out, int max)
{
	particle_t *p = side ? c_ref_free_particles : free_particles;
	particle_t *base = side ? c_ref_particles : particles;
	int			n = 0;

	for (; p; p = p->next)
	{
		if (n < max && out)
			out[n] = (int)(p - base);
		n++;
		if (n > CTEST_RPART_MAX_PARTICLES)
			Sys_Error ("ctest_rpart_free: cycle in the free list");
	}
	return n;
}

uint64_t ctest_rpart_hash (int side, uint64_t h)
{
	return side ? c_ref_Harness_HashParticles (h) : quake_rs_rpart_hash_particles (h);
}

/* ---- shared client / cvar state ----------------------------------------
 * cl, cls and sv_gravity exist twice in this link (oracle copy and port
 * copy), so every seeder writes both in one call; a test that wrote only one
 * would compare two different simulations.
 */

void ctest_rpart_seed_client (double time, double oldtime, int state, const char *mapname, unsigned protocolflags)
{
	cl.time = c_ref_cl.time = time;
	cl.oldtime = c_ref_cl.oldtime = oldtime;
	cl.protocolflags = c_ref_cl.protocolflags = protocolflags;
	cls.state = c_ref_cls.state = (cactive_t)state;

	memset (cl.mapname, 0, sizeof (cl.mapname));
	memset (c_ref_cl.mapname, 0, sizeof (c_ref_cl.mapname));
	if (mapname)
	{
		q_strlcpy (cl.mapname, mapname, sizeof (cl.mapname));
		q_strlcpy (c_ref_cl.mapname, mapname, sizeof (c_ref_cl.mapname));
	}
}

void ctest_rpart_set_gravity (float value)
{
	sv_gravity.value = c_ref_sv_gravity.value = value;
}

/* ---- the two cvars -------------------------------------------------------
 * `which` is 0 for r_particles and 1 for r_quadparticles. The flags word is
 * what makes R_InitParticles' Cvar_SetCallback observable: cvar.c:735-741 only
 * stores the pointer and sets CVAR_CALLBACK, and the pointer itself is not
 * comparable across sides (the oracle stores r_part.c's file-static, the port
 * stores RPart_Glue_SetParticleTexture_f).
 */

static cvar_t *ctest_rpart_cvar (int side, int which)
{
	if (side)
		return which ? &c_ref_rpart_r_quadparticles : &c_ref_rpart_r_particles;
	return which ? &r_quadparticles : &r_particles;
}

unsigned ctest_rpart_cvar_flags (int side, int which)
{
	return ctest_rpart_cvar (side, which)->flags;
}

float ctest_rpart_cvar_value (int side, int which)
{
	return ctest_rpart_cvar (side, which)->value;
}

/* 1 once R_InitParticles' Mem_Alloc has run for this side. */
int ctest_rpart_pool_allocated (int side)
{
	return (side ? c_ref_particles : particles) != NULL;
}

/* ---- glue-seam counters ------------------------------------------------- */

void ctest_rpart_reset_glue_counters (void)
{
	ctest_rpart_initrender_calls = 0;
	ctest_rpart_texcb_calls = 0;
	ctest_rpart_texcb_var = NULL;
}

int ctest_rpart_initrender_count (void)
{
	return ctest_rpart_initrender_calls;
}

int ctest_rpart_texcb_count (void)
{
	return ctest_rpart_texcb_calls;
}

/* 1 when the port handed Cvar_SetCallback the same cvar object the port
 * registered, which is the only part of r_part.c:248 this link can check. */
int ctest_rpart_texcb_was_r_particles (void)
{
	return ctest_rpart_texcb_var == &r_particles;
}

/* ---- drivers ------------------------------------------------------------
 * R_InitParticles is the module's only raise site (see ADR-009 in the header),
 * so it is entered through Host_Guard on both sides and returns the
 * CTEST_GUARD_* status; the message is readable through stubs.c's
 * ctest_host_error_message () / ctest_sys_error_message (). The rest are plain
 * calls, because r_part.c's remaining callees cannot longjmp.
 */

static void ctest_rpart_invoke_init (void *p)
{
	if (*(int *)p)
		c_ref_R_InitParticles ();
	else
		Host_Reraise (quake_rs_rpart_init_particles ());
}

int ctest_rpart_init (int side)
{
	int s = side;
	return Host_Guard (ctest_rpart_invoke_init, &s);
}

void ctest_rpart_entity_particles (int side, float *origin)
{
	if (side)
	{
		entity_t ent;
		memset (&ent, 0, sizeof (ent));
		VectorCopy (origin, ent.origin);
		c_ref_R_EntityParticles (&ent);
	}
	else
		quake_rs_rpart_entity_particles (origin);
}

void ctest_rpart_clear_particles (int side)
{
	if (side)
		c_ref_R_ClearParticles ();
	else
		quake_rs_rpart_clear_particles ();
}

void ctest_rpart_read_point_file (int side)
{
	if (side)
		c_ref_R_ReadPointFile_f ();
	else
		quake_rs_rpart_read_point_file_f ();
}

void ctest_rpart_parse_particle_effect (int side)
{
	if (side)
		c_ref_R_ParseParticleEffect ();
	else
		quake_rs_rpart_parse_particle_effect ();
}

void ctest_rpart_particle_explosion (int side, float *org)
{
	if (side)
		c_ref_R_ParticleExplosion (org);
	else
		quake_rs_rpart_particle_explosion (org);
}

void ctest_rpart_particle_explosion2 (int side, float *org, int colorStart, int colorLength)
{
	if (side)
		c_ref_R_ParticleExplosion2 (org, colorStart, colorLength);
	else
		quake_rs_rpart_particle_explosion2 (org, colorStart, colorLength);
}

void ctest_rpart_blob_explosion (int side, float *org)
{
	if (side)
		c_ref_R_BlobExplosion (org);
	else
		quake_rs_rpart_blob_explosion (org);
}

void ctest_rpart_run_particle_effect (int side, float *org, float *dir, int color, int count)
{
	if (side)
		c_ref_R_RunParticleEffect (org, dir, color, count);
	else
		quake_rs_rpart_run_particle_effect (org, dir, color, count);
}

void ctest_rpart_lava_splash (int side, float *org)
{
	if (side)
		c_ref_R_LavaSplash (org);
	else
		quake_rs_rpart_lava_splash (org);
}

void ctest_rpart_teleport_splash (int side, float *org)
{
	if (side)
		c_ref_R_TeleportSplash (org);
	else
		quake_rs_rpart_teleport_splash (org);
}

/* r_part.c:695 advances `start` in place, so the caller's buffer is an output
 * as much as an input; the test compares it after the call. */
void ctest_rpart_rocket_trail (int side, float *start, float *end, int type)
{
	if (side)
		c_ref_R_RocketTrail (start, end, type);
	else
		quake_rs_rpart_rocket_trail (start, end, type);
}

void ctest_rpart_run_particles (int side)
{
	if (side)
		c_ref_CL_RunParticles ();
	else
		quake_rs_rpart_run_particles ();
}
