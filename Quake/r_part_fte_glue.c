/*
Copyright (C) 1996-1997 Id Software, Inc.
Copyright (C) 2016      Spike
Copyright (C) 2026      vkqr-engine contributors

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
// r_part_fte_glue.c -- the C frame around the Rust FTE-particle port.
//
// Compiled instead of r_part_fte.c under -Duse_rust_host (Rust migration
// Phase 7 M10f-2, T10.5). Like r_part_glue.c this is a *split*, not just a
// frame: only r_part_fte.c's simulation half moved to Rust
// (rust/quake-capi/src/r_part_fte.rs). Six jobs:
//
//  1. Keep r_part_fte.c's rendering/emit half in C, verbatim and in its
//     original order (r_part_fte.c:502-536, :1146-1327, :5579-6253,
//     :6786-7393). It is Vulkan-typed throughout -- VkBuffer,
//     vulkan_memory_t, cb_context_t, R_BindPipeline -- or gltexture_t-typed
//     (P_LoadTexture), and the renderer belongs to Phase 8 per ROADMAP.md,
//     so porting it now would be out of roadmap order.
//
//  2. Keep the decal clipper (r_part_fte.c:3928-4308) in C as well. It is
//     not Vulkan-typed, but it walks mnode_t/msurface_t/mtexinfo_t and the
//     BSP node hierarchy directly, which Phase 7 does not own; the Rust side
//     builds the decalctx_t and reaches it through FtePart_Glue_ClipDecal.
//     This is a deviation from the M10f-2 contract's port region and is
//     called out in the milestone report.
//
//  3. Own the shared state that has a live C reader in either half
//     (ADR-007): the type array and run list, the three free lists,
//     particletime, the deferred queues, the flattened update array, the
//     per-type emit metadata, the kill list, pright/pup, the sine tables and
//     the sixteen cvars. Each loses its `static` so the Rust core can extern
//     it; the pool arrays themselves (particles/decals/beams/trailstates)
//     have no C reader left and stayed in Rust.
//
//  4. Guard the four callees that can themselves re-raise, so no C longjmp
//     crosses a Rust frame (ADR-009 rule 3): Cvar_RegisterVariable (sixteen
//     calls in PScript_InitParticles), CL_ClearTrailStates,
//     CL_RegisterParticles and CL_EntityNum.
//
//  5. Bridge the two halves in both directions. C -> Rust:
//     FtePart_Glue_LoadTexture, FtePart_Glue_ClipDecal,
//     FtePart_Glue_ModKnown, the two atomics helpers and the four stable
//     command/callback function pointers. Rust -> C: VectorNormalize2 and
//     PScript_QueueEffect were ported, but r_part_fte.c:5846 and :7071 (both
//     in the half that stays C) still call them, so they are re-exposed here
//     as thin forwards.
//
//  6. Give every public entry point in glquake.h:109-131 its plain C name,
//     turning the Rust status code back into a Host_Error on the C side of
//     the frame where the callee can raise.
//
// PSET_CLASSIC is never defined in this tree, so r_part_fte.c:3559-3568's
// `fallback`/`pe_classic` arm is dead code and was not ported.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

/* r_part_fte.c:31 -- outside PSET_SCRIPT in the original, read by pr_ext.c. */
cvar_t r_fteparticles = {"r_fteparticles", "1", CVAR_ARCHIVE};

#ifdef PSET_SCRIPT

/* ---------------------------------------------------------------------------
 * r_part_fte.c:34-86 -- the file's private macro vocabulary, verbatim.
 * VectorNormalize2 (:88) and VectorVectors (:109) moved to Rust; the former
 * comes back as a forward below because the C half still calls it, the latter
 * has no C caller left.
 */
#define USE_DECALS
#define Con_Printf Con_SafePrintf

#define frandom()  (COM_Rand () * (1.0f / (float)COM_RAND_MAX))
#define crandom()  (COM_Rand () * (2.0f / (float)COM_RAND_MAX) - 1.0f)
#define hrandom()  (COM_Rand () * (1.0f / (float)COM_RAND_MAX) - 0.5f)
#define particle_s fparticle_s
#define particle_t fparticle_t
typedef vec_t vec2_t[2];
#define FloatInterpolate(a, bness, b, c) ((c) = (a) + (b - a) * bness)
#define Vector2Copy(a, b) \
	do                    \
	{                     \
		(b)[0] = (a)[0];  \
		(b)[1] = (a)[1];  \
	} while (0)
#define Vector2Set(r, x, y) \
	do                      \
	{                       \
		(r)[0] = x;         \
		(r)[1] = y;         \
	} while (0)
#define VectorClear(a) ((a)[0] = (a)[1] = (a)[2] = 0)
#define VectorInterpolate(a, bness, b, c) \
	FloatInterpolate ((a)[0], bness, (b)[0], (c)[0]), FloatInterpolate ((a)[1], bness, (b)[1], (c)[1]), FloatInterpolate ((a)[2], bness, (b)[2], (c)[2])
#define VectorSet(r, x, y, z) \
	do                        \
	{                         \
		(r)[0] = x;           \
		(r)[1] = y;           \
		(r)[2] = z;           \
	} while (0)
#define Vector4Clear(a)				 ((a)[0] = (a)[1] = (a)[2] = (a)[3] = 0)
#define Vector4Scale(in, scale, out) ((out)[0] = (in)[0] * scale, (out)[1] = (in)[1] * scale, (out)[2] = (in)[2] * scale, (out)[3] = (in)[3] * scale)
#define FloatToColor(a, b)                              \
	do                                                  \
	{                                                   \
		(b) = (byte)(CLAMP (0.0f, (a), 1.0f) * 255.0f); \
	} while (0)
#define Vector3ToColor(a, b)                                  \
	do                                                        \
	{                                                         \
		(b)[0] = (byte)(CLAMP (0.0f, (a)[0], 1.0f) * 255.0f); \
		(b)[1] = (byte)(CLAMP (0.0f, (a)[1], 1.0f) * 255.0f); \
		(b)[2] = (byte)(CLAMP (0.0f, (a)[2], 1.0f) * 255.0f); \
	} while (0)
#define Vector4ToColor(a, b)                                  \
	do                                                        \
	{                                                         \
		(b)[0] = (byte)(CLAMP (0.0f, (a)[0], 1.0f) * 255.0f); \
		(b)[1] = (byte)(CLAMP (0.0f, (a)[1], 1.0f) * 255.0f); \
		(b)[2] = (byte)(CLAMP (0.0f, (a)[2], 1.0f) * 255.0f); \
		(b)[3] = (byte)(CLAMP (0.0f, (a)[3], 1.0f) * 255.0f); \
	} while (0)
typedef enum
{
	BM_BLEND /*SRC_ALPHA ONE_MINUS_SRC_ALPHA*/,
	BM_BLENDCOLOUR /*SRC_COLOR ONE_MINUS_SRC_COLOR*/,
	BM_ADDA /*SRC_ALPHA ONE*/,
	BM_ADDC /*GL_SRC_COLOR GL_ONE*/,
	BM_SUBTRACT /*SRC_ALPHA ONE_MINUS_SRC_COLOR*/,
	BM_INVMODA /*ZERO ONE_MINUS_SRC_ALPHA*/,
	BM_INVMODC /*ZERO ONE_MINUS_SRC_COLOR*/,
	BM_PREMUL /*ONE ONE_MINUS_SRC_ALPHA*/
} blendmode_t;
typedef struct trailstate_s
{
	struct trailstate_s **key;		// key to check if ts has been overwriten
	struct trailstate_s	 *assoc;	// assoc linked trail
	struct beamseg_s	 *lastbeam; // last beam pointer (flagged with BS_LASTSEG)
	union
	{
		float lastdist;	 // last distance used with particle effect
		float statetime; // time to emit effect again (used by spawntime field)
	} state1;
	union
	{
		float laststop; // last stopping point for particle effect
		float emittime; // used by r_effect emitters
	} state2;
} trailstate_t;
#define CON_WARNING "Warning: "
entity_t *CL_EntityNum (int num);
#define BEF_LINES 1

/* ---------------------------------------------------------------------------
 * r_part_fte.c:166-170. The sine tables lose their `static`: buildsintable
 * (:177) moved to Rust, but the C emit half still reads them through the
 * sin()/cos() macros below (:6068, :6160), so the storage stays here
 * (ADR-007).
 */

#define SINTABLE_ENTRIES 128
float psintable[SINTABLE_ENTRIES];
float pcostable[SINTABLE_ENTRIES];

/* r_part_fte.c:170 -- external linkage already; glquake.h:133 externs it and
 * InvalidateTraceLineCache() increments it from several other TUs. */
int r_trace_line_cache_counter;

#define sin(x) (psintable[(size_t)(int)((x) * ((SINTABLE_ENTRIES / 2) / M_PI)) % SINTABLE_ENTRIES])
#define cos(x) (pcostable[(size_t)(int)((x) * ((SINTABLE_ENTRIES / 2) / M_PI)) % SINTABLE_ENTRIES])
typedef struct particle_s
{
	struct particle_s *next;
	float			   die;

	// driver-usable fields
	vec3_t org;
	vec4_t rgba;
	float  scale;
	float  s1, t1, s2, t2;

	vec3_t oldorg; // to throttle traces
	vec3_t vel;	   // renderer uses for sparks
	float  angle;
	union
	{
		float		  nextemit;
		trailstate_t *trailstate;
	} state;
	// drivers never touch the following fields
	float rotationspeed;
} particle_t;

typedef struct clippeddecal_s
{
	struct clippeddecal_s *next;
	float				   die;

	int		  entity; //>0 is a lerpentity, <0 is a csqc ent. 0 is world. woot.
	qmodel_t *model;  // just for paranoia

	vec3_t vertex[3];
	vec2_t texcoords[3];
	float  valpha[3];

	vec4_t rgba;
} clippeddecal_t;

#define BS_LASTSEG 0x1 // no draw to next, no delete
#define BS_DEAD	   0x2 // segment is dead
#define BS_NODRAW  0x4 // only used for lerp switching

typedef struct beamseg_s
{
	struct beamseg_s *next; // next in beamseg list

	particle_t *p;
	int			flags; // flags for beamseg
	vec3_t		dir;

	float texture_s;
} beamseg_t;

typedef struct skytris_s
{
	struct skytris_s  *next;
	vec3_t			   org;
	vec3_t			   x;
	vec3_t			   y;
	float			   area;
	double			   nexttime;
	int				   ptype;
	struct msurface_s *face;
} skytris_t;

typedef struct skytriblock_s
{
	struct skytriblock_s *next;
	unsigned int		  count;
	skytris_t			  tris[1024];
} skytriblock_t;
// this is the required render state for each particle
// dynamic per-particle stuff isn't important. only static state.
typedef struct
{
	enum
	{
		PT_NORMAL,
		PT_SPARK,
		PT_SPARKFAN,
		PT_TEXTUREDSPARK,
		PT_BEAM,
		PT_CDECAL,
		PT_UDECAL,
		PT_INVISIBLE
	} type;

	blendmode_t	 blendmode;
	gltexture_t *texture;
	qboolean	 nearest;

	float scalefactor;
	float invscalefactor;
	float stretch;
	float minstretch; // limits the particle's length to a multiple of its width.
	int	  premul;	  // 0: direct rgba. 1: rgb*a,a (blend). 2: rgb*a,0 (add).
} plooks_t;

// these could be deltas or absolutes depending on ramping mode.
typedef struct
{
	vec3_t rgb;
	float  alpha;
	float  scale;
	float  rotation;
} ramp_t;
typedef struct
{
	char  name[MAX_QPATH];
	float vol;
	float atten;
	float delay;
	float pitch;
	float weight;
} partsounds_t;
// TODO: merge in alpha with rgb to gain benefit of vector opts
typedef struct part_type_s
{
	char name[MAX_QPATH];
	char config[MAX_QPATH];
	char texname[MAX_QPATH];

	int			  numsounds;
	partsounds_t *sounds;

	vec3_t rgb; // initial colour
	float  alpha;
	vec3_t rgbchange; // colour delta (per second)
	float  alphachange;
	vec3_t rgbrand; // random rgb colour to start with
	float  alpharand;
	int	   colorindex;			   // get colour from a palette
	int	   colorrand;			   // and add up to this amount
	float  rgbchangetime;		   // colour stops changing at this time
	vec3_t rgbrandsync;			   // like rgbrand, but a single random value instead of separate (can mix)
	float  scale;				   // initial scale
	float  scalerand;			   // with up to this much extra
	float  die, randdie;		   // how long it lasts (plus some rand)
	float  veladd, randomveladd;   // scale the incoming velocity by this much
	float  orgadd, randomorgadd;   // spawn the particle this far along its velocity direction
	float  spawnvel, spawnvelvert; // spawn the particle with a velocity based upon its spawn type (generally so it flies outwards)
	vec3_t orgbias;				   // static 3d world-coord bias
	vec3_t velbias;
	vec3_t orgwrand; // 3d world-coord randomisation without relation to spawn mode
	vec3_t velwrand; // 3d world-coord randomisation without relation to spawn mode
	float  viewspacefrac;
	float  flurry;
	int	   surfflagmatch; // this decal only spawns on these surfaces
	int	   surfflagmask;  // this decal only spawns on these surfaces

	float s1, t1, s2, t2; // texture coords
	float texsstride;	  // addition for s for each random slot.
	int	  randsmax;		  // max times the stride can be added

	plooks_t *slooks; // shared looks, so state switches don't apply between particles so much.
	plooks_t  looks;  //

	float spawntime;   // time limit for trails
	float spawnchance; // if < 0, particles might not spawn so many

	float rotationstartmin, rotationstartrand;
	float rotationmin, rotationrand;

	float scaledelta;
	float countextra;
	float count;
	float countrand;
	float countspacing;	 // for trails.
	float countoverflow; // for badly-designed effects, instead of depending on trail state.
	float rainfrequency; // surface emitter multiplier

	int	  assoc;
	int	  cliptype;
	int	  inwater;
	float clipcount;
	int	  emit;
	float emittime;
	float emitrand;
	float emitstart;

	float areaspread;
	float areaspreadvert;

	float spawnparam1;
	float spawnparam2;
	/*	float spawnparam3; */

	enum
	{
		SM_BOX,		   // box = even spread within the area
		SM_CIRCLE,	   // circle = around edge of a circle
		SM_BALL,	   // ball = filled sphere
		SM_SPIRAL,	   // spiral = spiral trail
		SM_TRACER,	   // tracer = tracer trail
		SM_TELEBOX,	   // telebox = q1-style telebox
		SM_LAVASPLASH, // lavasplash = q1-style lavasplash
		SM_UNICIRCLE,  // unicircle = uniform circle
		SM_FIELD,	   // field = synced field (brightfield, etc)
		SM_DISTBALL,   // uneven distributed ball
		SM_MESHSURFACE // distributed roughly evenly over the surface of the mesh
	} spawnmode;

	float  gravity;
	vec3_t friction;
	float  clipbounce;
	float  stainonimpact;

	vec3_t dl_rgb;
	float  dl_radius[2];
	float  dl_time;
	vec4_t dl_decay;
	float  dl_corona_intensity;
	float  dl_corona_scale;
	vec3_t dl_scales;
	// PT_NODLSHADOW
	int	   dl_cubemapnum;

	enum
	{
		RAMP_NONE,
		RAMP_DELTA,
		RAMP_NEAREST,
		RAMP_LERP
	} rampmode;
	int		rampindexes;
	ramp_t *ramp;

	int					loaded; // 0 if not loaded, 1 if automatically loaded, 2 if user loaded
	particle_t		   *particles;
	clippeddecal_t	   *clippeddecals;
	beamseg_t		   *beams;
	struct part_type_s *nexttorun;

	unsigned int flags;
#define PT_VELOCITY		  0x0001 // has velocity modifiers
#define PT_FRICTION		  0x0002 // has friction modifiers
#define PT_CHANGESCOLOUR  0x0004
#define PT_CITRACER		  0x0008 // Q1-style tracer behavior for colorindex
#define PT_INVFRAMETIME	  0x0010 // apply inverse frametime to count (causes emits to be per frame)
#define PT_AVERAGETRAIL	  0x0020 // average trail points from start to end, useful with t_lightning, etc
#define PT_NOSTATE		  0x0040 // don't use trailstate for this emitter (careful with assoc...)
#define PT_NOSPREADFIRST  0x0080 // don't randomize org/vel for first generated particle
#define PT_NOSPREADLAST	  0x0100 // don't randomize org/vel for last generated particle
#define PT_TROVERWATER	  0x0200 // don't spawn if underwater
#define PT_TRUNDERWATER	  0x0400 // don't spawn if overwater
#define PT_NODLSHADOW	  0x0800 // dlights from this effect don't cast shadows.
#define PT_WORLDSPACERAND 0x1000 // effect has orgwrand or velwrand properties
	unsigned int fluidmask;

	unsigned int state;
#define PS_INRUNLIST 0x1 // particle type is currently in execution list
} part_type_t;
extern cvar_t r_showtris;
extern cvar_t r_particles;

/* ---------------------------------------------------------------------------
 * Glue-owned storage (ADR-007). Each of these has a live C reader in the half
 * that stays C, so the storage stays here and
 * rust/quake-c-sys/src/r_part_fte.rs externs it. They all lose the `static`
 * they had in r_part_fte.c.
 */

/* r_part_fte.c:459 -- the FTE free list. It has to be renamed on the way out:
 * the classic particle system's own free_particles (r_part.c:38) is external
 * linkage in r_part_glue.c, and the two were only ever distinct because
 * r_part_fte.c's copy was static. Same trick the original file already uses
 * for particle_s/particle_t (:40-41). */
#define free_particles fte_free_particles
particle_t *free_particles;

/* r_part_fte.c:465, :469 -- the other two free-list heads. The pools they are
 * carved from (particles :460, beams :465, decals :469, trailstates :473) have
 * no C reader left and stayed in Rust. */
beamseg_t	   *free_beams;
clippeddecal_t *free_decals;

/* r_part_fte.c:500 -- advanced by PScript_UpdateParticleTypes (:7292). */
float particletime;

/* r_part_fte.c:760-762 -- the type array and the PS_INRUNLIST chain. */
int			 numparticletypes;
part_type_t *part_type;
part_type_t *part_run_list;

/* r_part_fte.c:482-496. cvar_t is a C ABI object that Cvar_RegisterVariable
 * links into the engine's hash chain and mutates thereafter, so the storage
 * stays C exactly as chase_glue.c and r_part_glue.c keep theirs;
 * r_decal_noperpendicular is additionally read by the decal callback below
 * (:3969). r_particledesc already had external linkage in the original --
 * pr_ext.c:4724 externs it. */
cvar_t r_bouncysparks = {"r_bouncysparks", "1"};
cvar_t r_part_rain = {"r_part_rain", "1"};
cvar_t r_decal_noperpendicular = {"r_decal_noperpendicular", "1"};
cvar_t r_particledesc = {"r_particledesc", "classic"};
cvar_t r_part_rain_quantity = {"r_part_rain_quantity", "1"};
cvar_t r_particle_tracelimit = {"r_particle_tracelimit", "16777216"};
cvar_t r_part_sparks = {"r_part_sparks", "1"};
cvar_t r_part_sparks_trifan = {"r_part_sparks_trifan", "1"};
cvar_t r_part_sparks_textured = {"r_part_sparks_textured", "1"};
cvar_t r_part_beams = {"r_part_beams", "1"};
cvar_t r_part_contentswitch = {"r_part_contentswitch", "1"};
cvar_t r_part_density = {"r_part_density", "1"};
cvar_t r_part_maxparticles = {"r_part_maxparticles", "65536"};
cvar_t r_part_maxdecals = {"r_part_maxdecals", "8192"};
cvar_t r_lightflicker = {"r_lightflicker", "1"};

typedef struct
{
	int firstidx;
	int firstvert;
	int numidx;
	int numvert;

	gltexture_t *texture;
	blendmode_t	 blendmode;
	int			 beflags;
	qboolean	 use_oit;
} scenetris_t;

static qboolean PScript_LooksUseWBOIT (const plooks_t *looks)
{
	// Ironwail's OIT path is alpha transparency only. FTE additive, inverse-modulate,
	// subtractive, and color-modulate modes need their original blend equations.
	return looks->blendmode == BM_BLEND || (looks->blendmode == BM_PREMUL && looks->premul != 2);
}

#define MAX_INDICES			 0xffff
#define INITIAL_NUM_VERTICES 100000
#define INITIAL_NUM_INDICES	 150000

static scenetris_t	  *cl_stris;
static unsigned int	   cl_numstris;
static unsigned int	   cl_maxstris;
static basicvertex_t  *cl_strisvert[2];
static basicvertex_t  *cl_curstrisvert;
static unsigned int	   cl_numstrisvert;
static unsigned int	   cl_maxstrisvert[2];
static unsigned short *cl_strisidx[2];
static unsigned short *cl_curstrisidx;
static unsigned int	   cl_numstrisidx;
static unsigned int	   cl_maxstrisidx[2];

static void P_LoadTexture (part_type_t *ptype, qboolean warn)
{
	if (*ptype->texname)
	{
		byte *data = NULL;
		char  filename[MAX_QPATH];
		int	  fwidth = 0, fheight = 0;
		char *texname = va ("%s%s%s", ptype->texname, ptype->looks.premul ? "_premul" : "", ptype->looks.nearest ? "_nearest" : "");

		ptype->looks.texture = TexMgr_FindTexture (NULL, texname);
		if (!ptype->looks.texture)
		{
			enum srcformat fmt = SRC_RGBA;
			if (!data)
			{
				q_snprintf (filename, sizeof (filename), "textures/%s", ptype->texname);
				data = Image_LoadImage (filename, &fwidth, &fheight, &fmt, 0);
			}
			if (!data)
			{
				q_snprintf (filename, sizeof (filename), "%s", ptype->texname);
				data = Image_LoadImage (filename, &fwidth, &fheight, &fmt, 0);
			}

			if (data)
			{
				ptype->looks.texture = TexMgr_LoadImage (
					NULL, texname, fwidth, fheight, fmt, data, filename, 0,
					(ptype->looks.premul ? TEXPREF_PREMULTIPLY : 0) | (ptype->looks.nearest ? TEXPREF_NEAREST : 0) | TEXPREF_NOPICMIP | TEXPREF_ALPHA);
			}
		}
	}
	else
		ptype->looks.texture = 0;

	if (!ptype->looks.texture)
	{
		// the specified texture isn't valid. make something up based upon the particle's type
		ptype->s1 = 0;
		ptype->t1 = 0;
		ptype->s2 = 1;
		ptype->t2 = 1;
		ptype->randsmax = 1;

#define PARTICLETEXTURESIZE 64
		if (ptype->looks.type == PT_SPARK)
		{
			static gltexture_t *thetex;
			if (!thetex)
			{
				static byte data[4 * 4 * 4];
				memset (data, 0xff, sizeof (data));
				thetex = TexMgr_LoadImage (
					NULL, "particles/white", 4, 4, SRC_RGBA, data, "", (src_offset_t)data, TEXPREF_PERSIST | TEXPREF_NOPICMIP | TEXPREF_ALPHA);
			}
			ptype->looks.texture = thetex;
		}
		else if (ptype->looks.type == PT_BEAM) // untextured beams get a single continuous blob
		{
			static gltexture_t *thetex;
			if (!thetex)
			{
				int			y, x;
				float		dy, d;
				static byte data[PARTICLETEXTURESIZE * PARTICLETEXTURESIZE * 4];
				memset (data, 0xff, sizeof (data));
				for (y = 0; y < PARTICLETEXTURESIZE; y++)
				{
					dy = (y - 0.5f * PARTICLETEXTURESIZE) / (PARTICLETEXTURESIZE * 0.5f - 1);
					d = 256 * (1 - (dy * dy));
					if (d < 0)
						d = 0;
					for (x = 0; x < PARTICLETEXTURESIZE; x++)
					{
						data[(y * PARTICLETEXTURESIZE + x) * 4 + 3] = (byte)d;
					}
				}
				thetex = TexMgr_LoadImage (
					NULL, "particles/beamtexture", PARTICLETEXTURESIZE, PARTICLETEXTURESIZE, SRC_RGBA, data, "", (src_offset_t)data,
					TEXPREF_PERSIST | TEXPREF_NOPICMIP | TEXPREF_ALPHA);
			}
			ptype->looks.texture = thetex;
		}
		else if (ptype->looks.type == PT_SPARKFAN) // untextured beams get a single continuous blob
		{
			static gltexture_t *thetex;
			if (!thetex)
			{
				int			y, x;
				float		dy, dx, d;
				static byte data[PARTICLETEXTURESIZE * PARTICLETEXTURESIZE * 4];
				for (y = 0; y < PARTICLETEXTURESIZE; y++)
				{
					dy = y / (PARTICLETEXTURESIZE * 0.5f - 1);
					for (x = 0; x < PARTICLETEXTURESIZE; x++)
					{
						dx = x / (PARTICLETEXTURESIZE * 0.5f - 1);
						d = 256 * (1 - (dx + dy));
						if (d < 0)
							d = 0;
						data[(y * PARTICLETEXTURESIZE + x) * 4 + 0] = (byte)d;
						data[(y * PARTICLETEXTURESIZE + x) * 4 + 1] = (byte)d;
						data[(y * PARTICLETEXTURESIZE + x) * 4 + 2] = (byte)d;
						data[(y * PARTICLETEXTURESIZE + x) * 4 + 3] = (byte)d / 2;
					}
				}
				thetex = TexMgr_LoadImage (
					NULL, "particles/ptritexture", PARTICLETEXTURESIZE, PARTICLETEXTURESIZE, SRC_RGBA, data, "", (src_offset_t)data,
					TEXPREF_PERSIST | TEXPREF_NOPICMIP | TEXPREF_ALPHA);
			}
			ptype->looks.texture = thetex;
		}
		else if (strstr (ptype->texname, "classicparticle"))
		{
			extern gltexture_t *particletexture1;
			ptype->looks.texture = particletexture1;
			ptype->s2 = 0.5;
			ptype->t2 = 0.5;
		}
		else if (strstr (ptype->texname, "glow") || strstr (ptype->texname, "ball") || ptype->looks.type == PT_TEXTUREDSPARK) // sparks and special names get a
																															  // nice circular texture.
		{
			static gltexture_t *thetex;
			if (!thetex)
			{
				int			y, x;
				float		dy, dx, d;
				static byte data[PARTICLETEXTURESIZE * PARTICLETEXTURESIZE * 4];
				memset (data, 0xff, sizeof (data));
				for (y = 0; y < PARTICLETEXTURESIZE; y++)
				{
					dy = (y - 0.5f * PARTICLETEXTURESIZE) / (PARTICLETEXTURESIZE * 0.5f - 1);
					for (x = 0; x < PARTICLETEXTURESIZE; x++)
					{
						dx = (x - 0.5f * PARTICLETEXTURESIZE) / (PARTICLETEXTURESIZE * 0.5f - 1);
						d = 255 * (1 - (dx * dx + dy * dy));
						if (d < 0)
							d = 0;
						data[(y * PARTICLETEXTURESIZE + x) * 4 + 3] = (byte)d;
					}
				}
				thetex = TexMgr_LoadImage (
					NULL, "particles/balltexture", PARTICLETEXTURESIZE, PARTICLETEXTURESIZE, SRC_RGBA, data, "", (src_offset_t)data,
					TEXPREF_PERSIST | TEXPREF_NOPICMIP | TEXPREF_ALPHA);
			}
			ptype->looks.texture = thetex;
		}
		else // anything else gets a fuzzy texture
		{
			static gltexture_t *thetex;
			if (!thetex)
			{
				int			y, x;
				static byte exptexture[16][16] = {
					{0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0}, {0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0},
					{0, 0, 0, 1, 1, 1, 1, 1, 3, 1, 1, 2, 1, 0, 0, 0}, {0, 0, 0, 1, 1, 1, 1, 4, 4, 4, 5, 4, 2, 1, 1, 0},
					{0, 0, 1, 1, 6, 5, 5, 8, 6, 8, 3, 6, 3, 2, 1, 0}, {0, 0, 1, 5, 6, 7, 5, 6, 8, 8, 8, 3, 3, 1, 0, 0},
					{0, 0, 0, 1, 6, 8, 9, 9, 9, 9, 4, 6, 3, 1, 0, 0}, {0, 0, 2, 1, 7, 7, 9, 9, 9, 9, 5, 3, 1, 0, 0, 0},
					{0, 0, 2, 4, 6, 8, 9, 9, 9, 9, 8, 6, 1, 0, 0, 0}, {0, 0, 2, 2, 3, 5, 6, 8, 9, 8, 8, 4, 4, 1, 0, 0},
					{0, 0, 1, 2, 4, 1, 8, 7, 8, 8, 6, 5, 4, 1, 0, 0}, {0, 1, 1, 1, 7, 8, 1, 6, 7, 5, 4, 7, 1, 0, 0, 0},
					{0, 1, 2, 1, 1, 5, 1, 3, 4, 3, 1, 1, 0, 0, 0, 0}, {0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0},
					{0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0}, {0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0},
				};
				static byte data[16 * 16 * 4];
				for (x = 0; x < 16; x++)
				{
					for (y = 0; y < 16; y++)
					{
						data[(y * 16 + x) * 4 + 0] = 255;
						data[(y * 16 + x) * 4 + 1] = 255;
						data[(y * 16 + x) * 4 + 2] = 255;
						data[(y * 16 + x) * 4 + 3] = exptexture[x][y] * 255 / 9.0;
					}
				}
				thetex = TexMgr_LoadImage (
					NULL, "particles/fuzzyparticle", 16, 16, SRC_RGBA, data, "", (src_offset_t)data, TEXPREF_PERSIST | TEXPREF_NOPICMIP | TEXPREF_ALPHA);
			}
			ptype->looks.texture = thetex;
		}
	}
}
#ifdef USE_DECALS
typedef struct
{
	part_type_t *ptype;
	int			 entity;
	qmodel_t	*model;
	vec3_t		 center;
	vec3_t		 normal;
	vec3_t		 tangent1;
	vec3_t		 tangent2;

	float scale0;
	float scale1;
	float scale2;

	float bias1;
	float bias2;
} decalctx_t;
static void PScript_AddDecals (void *vctx, vec3_t *points, size_t numtris)
{
	decalctx_t	   *ctx = vctx;
	part_type_t	   *ptype = ctx->ptype;
	clippeddecal_t *d;
	unsigned int	i;
	vec3_t			vec;
	byte		   *palrgba = (byte *)d_8to24table;
	while (numtris-- > 0)
	{
		if (!free_decals)
			break;

		d = free_decals;
		free_decals = d->next;
		d->next = ptype->clippeddecals;
		ptype->clippeddecals = d;

		for (i = 0; i < 3; i++)
		{
			VectorCopy (points[i], d->vertex[i]);
			VectorSubtract (d->vertex[i], ctx->center, vec);
			d->texcoords[i][0] = (DotProduct (vec, ctx->tangent1) * ctx->scale1) + ctx->bias1;
			d->texcoords[i][1] = (DotProduct (vec, ctx->tangent2) * ctx->scale2) + ctx->bias2;
			if (r_decal_noperpendicular.value)
			{
				// the decal code is already making sure the surfaces are mostly aligned, which should solve some issues.
				// this means we can make sure that there's NO fading at all, so no issues if the center of the effect is not actually aligned with any surface
				// (yay inprecision).
				d->valpha[i] = 1;
			}
			else
			{
				// fade the alpha depending on the distance from the center)
				// FIXME: should be fabsed by glsl so that linear interpolation works correctly
				d->valpha[i] = 1 - fabs ((DotProduct (vec, ctx->normal) * ctx->scale0));
			}
		}
		points += 3;

		d->entity = ctx->entity;
		d->model = ctx->model;
		d->die = ptype->randdie * frandom ();

		if (ptype->die)
			d->rgba[3] = ptype->alpha + d->die * ptype->alphachange;
		else
			d->rgba[3] = ptype->alpha;
		d->rgba[3] += ptype->alpharand * frandom ();

		if (ptype->colorindex >= 0)
		{
			int cidx;
			cidx = ptype->colorrand > 0 ? COM_Rand () % ptype->colorrand : 0;
			cidx = ptype->colorindex + cidx;
			if (cidx > 255)
				d->rgba[3] = d->rgba[3] / 2; // Hexen 2 style transparency
			cidx = (cidx & 0xff) * 4;
			d->rgba[0] = palrgba[cidx] * (1 / 255.0);
			d->rgba[1] = palrgba[cidx + 1] * (1 / 255.0);
			d->rgba[2] = palrgba[cidx + 2] * (1 / 255.0);
		}
		else
			VectorCopy (ptype->rgb, d->rgba);

		vec[2] = frandom ();
		vec[0] = vec[2] * ptype->rgbrandsync[0] + frandom () * (1 - ptype->rgbrandsync[0]);
		vec[1] = vec[2] * ptype->rgbrandsync[1] + frandom () * (1 - ptype->rgbrandsync[1]);
		vec[2] = vec[2] * ptype->rgbrandsync[2] + frandom () * (1 - ptype->rgbrandsync[2]);
		d->rgba[0] += vec[0] * ptype->rgbrand[0] + ptype->rgbchange[0] * d->die;
		d->rgba[1] += vec[1] * ptype->rgbrand[1] + ptype->rgbchange[1] * d->die;
		d->rgba[2] += vec[2] * ptype->rgbrand[2] + ptype->rgbchange[2] * d->die;

		d->die = particletime + ptype->die - d->die;

		if (ptype->looks.type != PT_CDECAL)
			d->die += 20;

		// maintain run list
		if (!(ptype->state & PS_INRUNLIST))
		{
			ptype->nexttorun = part_run_list;
			part_run_list = ptype;
			ptype->state |= PS_INRUNLIST;
		}
	}
}

typedef struct fragmentdecal_s fragmentdecal_t;
static void					   Mod_ClipDecal (
					   qmodel_t *mod, vec3_t center, vec3_t normal, vec3_t tangent1, vec3_t tangent2, float size, unsigned int surfflagmask, unsigned int surfflagmatch,
					   void (*callback) (void *ctx, vec3_t *points, size_t numpoints), void *ctx);

// clipped decals actually work by defining the area of the decal with some planes, and then chopping away the entirety of the world based upon those planes
// (hurrah for bsp to trivially reject most of it) the decal is then textured according to some texture projection.
#define MAXFRAGMENTVERTS (128 * 3)
struct fragmentdecal_s
{
	vec3_t center;

	vec3_t normal;
	vec3_t planenorm[6];
	float  planedist[6];
	int	   numplanes;

	vec_t radius;

	// will only appear on surfaces with the matching surfaceflag
	unsigned int surfflagmask;
	unsigned int surfflagmatch;

	void (*callback) (void *ctx, vec3_t *points, size_t numpoints);
	void *ctx;
};
static int Fragment_ClipPolyToPlane (vec3_t *inverts, vec3_t *outverts, int incount, float *plane, float planedist)
{
	float dotv[MAXFRAGMENTVERTS + 1];
	char  keep[MAXFRAGMENTVERTS + 1];
#define KEEP_KILL	0
#define KEEP_KEEP	1
#define KEEP_BORDER 2
	int	   i;
	int	   outcount = 0;
	int	   clippedcount = 0;
	float  d;
	float *p1, *p2;
	float *out;
#define FRAG_EPSILON (1.0 / 32) // 0.5

	for (i = 0; i < incount; i++)
	{
		dotv[i] = DotProduct (inverts[i], plane) - planedist;
		if (dotv[i] < -FRAG_EPSILON)
		{
			keep[i] = KEEP_KILL;
			clippedcount++;
		}
		else if (dotv[i] > FRAG_EPSILON)
			keep[i] = KEEP_KEEP;
		else
			keep[i] = KEEP_BORDER;
	}
	dotv[i] = dotv[0];
	keep[i] = keep[0];

	if (clippedcount == incount)
		return 0; // all were clipped
	if (clippedcount == 0)
	{ // none were clipped
		for (i = 0; i < incount; i++)
			VectorCopy (inverts[i], outverts[i]);
		return incount;
	}

	for (i = 0; i < incount; i++)
	{
		p1 = inverts[i];
		if (keep[i] == KEEP_BORDER)
		{
			out = outverts[outcount++];
			VectorCopy (p1, out);
			continue;
		}
		if (keep[i] == KEEP_KEEP)
		{
			out = outverts[outcount++];
			VectorCopy (p1, out);
		}
		if (keep[i + 1] == KEEP_BORDER || keep[i] == keep[i + 1])
			continue;
		p2 = inverts[(i + 1) % incount];
		d = dotv[i] - dotv[i + 1];
		if (d)
			d = dotv[i] / d;

		out = outverts[outcount++];
		VectorInterpolate (p1, d, p2, out);
	}
	return outcount;
}
static void Fragment_ClipPoly (fragmentdecal_t *dec, int numverts, vec3_t *inverts)
{
	// emit the triangle, and clip it's fragments.
	int	   p;
	vec3_t verts[2][MAXFRAGMENTVERTS];
	vec3_t decalfragmentverts[MAXFRAGMENTVERTS];
	int	   flip;
	vec3_t d1, d2, n;
	size_t numtris;

	if (numverts > MAXFRAGMENTVERTS)
		return;

	if (r_decal_noperpendicular.value)
	{
		VectorSubtract (inverts[1], inverts[0], d1);
		for (p = 2;; p++)
		{
			if (p >= numverts)
				return;
			VectorSubtract (inverts[p], inverts[0], d2);
			CrossProduct (d1, d2, n);
			if (DotProduct (n, n) > .1)
				break;
		}
		VectorNormalizeFast (n);
		if (DotProduct (n, dec->normal) < 0.1)
			return; // faces too far way from the normal
	}

	flip = 0;
	// clip to the first plane specially, so we don't have extra copys
	numverts = Fragment_ClipPolyToPlane (inverts, verts[flip], numverts, dec->planenorm[0], dec->planedist[0]);

	if (numverts < 3) // totally clipped.
		return;

	// clip the polygon to the 6 planes.
	for (p = 1; p < dec->numplanes; p++)
	{
		numverts = Fragment_ClipPolyToPlane (verts[flip], verts[flip ^ 1], numverts, dec->planenorm[p], dec->planedist[p]);
		flip ^= 1;

		if (numverts < 3) // totally clipped.
			return;
	}

	// decompose the resulting polygon into triangles.

	numtris = 0;
	while (numverts-- > 2)
	{
		if (numtris + 3 > MAXFRAGMENTVERTS)
		{
			dec->callback (dec->ctx, decalfragmentverts, numtris);
			numtris = 0;
			break;
		}

		VectorCopy (verts[flip][0], decalfragmentverts[numtris * 3 + 0]);
		VectorCopy (verts[flip][numverts - 1], decalfragmentverts[numtris * 3 + 1]);
		VectorCopy (verts[flip][numverts], decalfragmentverts[numtris * 3 + 2]);
		numtris++;
	}
	if (numtris)
		dec->callback (dec->ctx, decalfragmentverts, numtris);
}
// this could be inlined, but I'm lazy.
static void Q1BSP_Fragment_Surface (fragmentdecal_t *dec, msurface_t *surf)
{
	int		  i;
	vec3_t	  verts[MAXFRAGMENTVERTS];
	glpoly_t *poly;
	float	 *poly_vert;

	// water and sky should not get decals.
	if (surf->flags & (SURF_DRAWSKY | SURF_DRAWTURB))
		return;

	for (poly = surf->polys; poly; poly = poly->next)
	{
		if (poly->numverts > MAXFRAGMENTVERTS)
			continue;

		for (i = 0; i < poly->numverts; i++)
		{
			poly_vert = &poly->verts[0][0] + (i * VERTEXSIZE);
			VectorCopy (poly_vert, verts[i]);
		}
		Fragment_ClipPoly (dec, i, verts);
	}
}
static void Q1BSP_ClipDecalToNodes (qmodel_t *mod, fragmentdecal_t *dec, mnode_t *node)
{
	mplane_t	*splitplane;
	float		 dist;
	msurface_t	*surf;
	unsigned int i;

	if (node->contents < 0)
		return;

	splitplane = node->plane;
	dist = DotProduct (dec->center, splitplane->normal) - splitplane->dist;

	if (dist > dec->radius)
	{
		Q1BSP_ClipDecalToNodes (mod, dec, node->children[0]);
		return;
	}
	if (dist < -dec->radius)
	{
		Q1BSP_ClipDecalToNodes (mod, dec, node->children[1]);
		return;
	}

	// mark the polygons
	surf = mod->surfaces + node->firstsurface;
	if (r_decal_noperpendicular.value)
	{
		for (i = 0; i < node->numsurfaces; i++, surf++)
		{
			if (surf->flags & SURF_PLANEBACK)
			{
				if (-DotProduct (surf->plane->normal, dec->normal) > -0.5)
					continue;
			}
			else
			{
				if (DotProduct (surf->plane->normal, dec->normal) > -0.5)
					continue;
			}
			Q1BSP_Fragment_Surface (dec, surf);
		}
	}
	else
	{
		for (i = 0; i < node->numsurfaces; i++, surf++)
			Q1BSP_Fragment_Surface (dec, surf);
	}

	Q1BSP_ClipDecalToNodes (mod, dec, node->children[0]);
	Q1BSP_ClipDecalToNodes (mod, dec, node->children[1]);
}

static void Mod_ClipDecal (
	qmodel_t *mod, vec3_t center, vec3_t normal, vec3_t tangent1, vec3_t tangent2, float size, unsigned int surfflagmask, unsigned int surfflagmatch,
	void (*callback) (void *ctx, vec3_t *points, size_t numpoints), void *ctx)
{ // quad marks a full, independant quad
	int				p;
	float			r;
	fragmentdecal_t dec;

	VectorCopy (center, dec.center);
	VectorCopy (normal, dec.normal);
	dec.radius = 0;
	dec.callback = callback;
	dec.ctx = ctx;
	dec.surfflagmask = surfflagmask;
	dec.surfflagmatch = surfflagmatch;

	VectorCopy (tangent1, dec.planenorm[0]);
	VectorScale (tangent1, -1, dec.planenorm[1]);
	VectorCopy (tangent2, dec.planenorm[2]);
	VectorScale (tangent2, -1, dec.planenorm[3]);
	VectorCopy (dec.normal, dec.planenorm[4]);
	VectorScale (dec.normal, -1, dec.planenorm[5]);
	for (p = 0; p < 6; p++)
	{
		r = sqrt (DotProduct (dec.planenorm[p], dec.planenorm[p]));
		VectorScale (dec.planenorm[p], 1 / r, dec.planenorm[p]);
		r *= size / 2;
		if (r > dec.radius)
			dec.radius = r;
		dec.planedist[p] = -(r - DotProduct (dec.center, dec.planenorm[p]));
	}
	dec.numplanes = 6;

	if (mod && !mod->needload && mod->type == mod_brush)
		Q1BSP_ClipDecalToNodes (mod, &dec, mod->nodes + mod->hulls[0].firstclipnode);
}
#endif

/* ---------------------------------------------------------------------------
 * Rust -> C forwards. Both of these were ported, but the half that stays C
 * still calls them, so they come back as file-static thunks rather than as
 * duplicated logic. Neither can raise.
 */

/* r_part_fte.c:88 -- called by R_EmitLineSparkParticle (:5846). */
static vec_t VectorNormalize2 (const vec3_t v, vec3_t out)
{
	return quake_rs_ftepart_vector_normalize2 (v, out);
}

/* r_part_fte.c:6325 -- called by PScript_UpdateParticleTypes (:7071). */
static void PScript_QueueEffect (vec3_t org, vec3_t dir, float count, int type)
{
	quake_rs_ftepart_queue_effect (org, dir, count, type);
}

static int			   current_buffer_index = 0;
static VkBuffer		   vertex_buffers[2] = {VK_NULL_HANDLE, VK_NULL_HANDLE};
static vulkan_memory_t vertex_buffers_memory[2] = {{VK_NULL_HANDLE, 0, 0}, {VK_NULL_HANDLE, 0, 0}};
static VkBuffer		   index_buffers[2] = {VK_NULL_HANDLE, VK_NULL_HANDLE};
static vulkan_memory_t index_buffers_memory[2] = {{VK_NULL_HANDLE, 0, 0}, {VK_NULL_HANDLE, 0, 0}};

static void ReallocateVertexBuffer ()
{
	VkResult err;

	if (vertex_buffers[current_buffer_index] != VK_NULL_HANDLE)
		vkDestroyBuffer (vulkan_globals.device, vertex_buffers[current_buffer_index], NULL);

	vulkan_memory_t		 old_memory = vertex_buffers_memory[current_buffer_index];
	const basicvertex_t *old_cl_curstrisvert = cl_curstrisvert;
	const int			 old_maxstrisvert = cl_maxstrisvert[current_buffer_index];

	cl_maxstrisvert[current_buffer_index] = q_max (cl_maxstrisvert[current_buffer_index] * 2, INITIAL_NUM_VERTICES);
	const VkDeviceSize new_size = cl_maxstrisvert[current_buffer_index] * sizeof (basicvertex_t);
	Sys_Printf ("Reallocating FTE particle vertex buffer (%u KB)\n", (int)(new_size / 1024));

	ZEROED_STRUCT (VkBufferCreateInfo, buffer_create_info);
	buffer_create_info.sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO;
	buffer_create_info.size = new_size;
	buffer_create_info.usage = VK_BUFFER_USAGE_VERTEX_BUFFER_BIT;

	err = vkCreateBuffer (vulkan_globals.device, &buffer_create_info, NULL, &vertex_buffers[current_buffer_index]);
	if (err != VK_SUCCESS)
		Sys_Error ("vkCreateBuffer failed with code %i", (int)err);
	GL_SetObjectName ((uint64_t)vertex_buffers[current_buffer_index], VK_OBJECT_TYPE_BUFFER, "FTE Particle Vertex Buffer");

	VkMemoryRequirements memory_requirements;
	vkGetBufferMemoryRequirements (vulkan_globals.device, vertex_buffers[current_buffer_index], &memory_requirements);

	const int aligned_size = q_align (memory_requirements.size, memory_requirements.alignment);

	ZEROED_STRUCT (VkMemoryAllocateInfo, memory_allocate_info);
	memory_allocate_info.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;
	memory_allocate_info.allocationSize = aligned_size;
	memory_allocate_info.memoryTypeIndex =
		GL_MemoryTypeFromProperties (memory_requirements.memoryTypeBits, VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT, VK_MEMORY_PROPERTY_HOST_CACHED_BIT);

	R_AllocateVulkanMemory (&vertex_buffers_memory[current_buffer_index], &memory_allocate_info, VULKAN_MEMORY_TYPE_HOST, &num_vulkan_dynbuf_allocations);
	GL_SetObjectName ((uint64_t)vertex_buffers_memory[current_buffer_index].handle, VK_OBJECT_TYPE_DEVICE_MEMORY, "FTE Particle Vertex Buffer");

	err = vkBindBufferMemory (vulkan_globals.device, vertex_buffers[current_buffer_index], vertex_buffers_memory[current_buffer_index].handle, 0);
	if (err != VK_SUCCESS)
		Sys_Error ("vkBindBufferMemory failed with code %i", (int)err);

	err = vkMapMemory (vulkan_globals.device, vertex_buffers_memory[current_buffer_index].handle, 0, new_size, 0, (void **)&cl_curstrisvert);
	if (err != VK_SUCCESS)
		Sys_Error ("vkMapMemory failed with code %i", (int)err);
	cl_strisvert[current_buffer_index] = cl_curstrisvert;

	if (old_memory.handle != VK_NULL_HANDLE)
	{
		// Copy over data from old buffer
		memcpy (cl_curstrisvert, old_cl_curstrisvert, old_maxstrisvert * sizeof (basicvertex_t));

		vkUnmapMemory (vulkan_globals.device, old_memory.handle);
		R_FreeVulkanMemory (&old_memory, &num_vulkan_dynbuf_allocations);
	}
}

static void ReallocateIndexBuffer ()
{
	VkResult err;

	if (index_buffers[current_buffer_index] != VK_NULL_HANDLE)
		vkDestroyBuffer (vulkan_globals.device, index_buffers[current_buffer_index], NULL);

	vulkan_memory_t		  old_memory = index_buffers_memory[current_buffer_index];
	const unsigned short *old_cl_curstrisidx = cl_curstrisidx;
	const int			  old_maxstrisidx = cl_maxstrisidx[current_buffer_index];

	cl_maxstrisidx[current_buffer_index] = q_max (cl_maxstrisidx[current_buffer_index] * 2, INITIAL_NUM_INDICES);
	const VkDeviceSize new_size = cl_maxstrisidx[current_buffer_index] * sizeof (unsigned short);
	Sys_Printf ("Reallocating FTE particle index buffer (%u KB)\n", (int)(new_size / 1024));

	ZEROED_STRUCT (VkBufferCreateInfo, buffer_create_info);
	buffer_create_info.sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO;
	buffer_create_info.size = new_size;
	buffer_create_info.usage = VK_BUFFER_USAGE_INDEX_BUFFER_BIT;

	err = vkCreateBuffer (vulkan_globals.device, &buffer_create_info, NULL, &index_buffers[current_buffer_index]);
	if (err != VK_SUCCESS)
		Sys_Error ("vkCreateBuffer failed with code %i", (int)err);
	GL_SetObjectName ((uint64_t)index_buffers[current_buffer_index], VK_OBJECT_TYPE_BUFFER, "FTE Particle Index Buffer");

	VkMemoryRequirements memory_requirements;
	vkGetBufferMemoryRequirements (vulkan_globals.device, index_buffers[current_buffer_index], &memory_requirements);

	const int aligned_size = q_align (memory_requirements.size, memory_requirements.alignment);

	ZEROED_STRUCT (VkMemoryAllocateInfo, memory_allocate_info);
	memory_allocate_info.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;
	memory_allocate_info.allocationSize = aligned_size;
	memory_allocate_info.memoryTypeIndex =
		GL_MemoryTypeFromProperties (memory_requirements.memoryTypeBits, VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT, VK_MEMORY_PROPERTY_HOST_CACHED_BIT);

	R_AllocateVulkanMemory (&index_buffers_memory[current_buffer_index], &memory_allocate_info, VULKAN_MEMORY_TYPE_HOST, &num_vulkan_dynbuf_allocations);
	GL_SetObjectName ((uint64_t)index_buffers_memory[current_buffer_index].handle, VK_OBJECT_TYPE_DEVICE_MEMORY, "FTE Particle index Buffer");

	err = vkBindBufferMemory (vulkan_globals.device, index_buffers[current_buffer_index], index_buffers_memory[current_buffer_index].handle, 0);
	if (err != VK_SUCCESS)
		Sys_Error ("vkBindBufferMemory failed with code %i", (int)err);

	err = vkMapMemory (vulkan_globals.device, index_buffers_memory[current_buffer_index].handle, 0, new_size, 0, (void **)&cl_curstrisidx);
	if (err != VK_SUCCESS)
		Sys_Error ("vkMapMemory failed with code %i", (int)err);
	cl_strisidx[current_buffer_index] = cl_curstrisidx;

	if (old_memory.handle != VK_NULL_HANDLE)
	{
		// Copy over data from old buffer
		memcpy (cl_curstrisidx, old_cl_curstrisidx, old_maxstrisidx * sizeof (unsigned short));

		vkUnmapMemory (vulkan_globals.device, old_memory.handle);
		R_FreeVulkanMemory (&old_memory, &num_vulkan_dynbuf_allocations);
	}
}

/* r_part_fte.c:5701 -- the 1.5-scaled view axes. Written by
 * PScript_UpdateParticlesSetupTask on the Rust side, read by every sprite
 * emitter below, so the storage stays here (ADR-007). */
vec3_t pright, pup;

static void R_EmitFanSparkParticle (scenetris_t *t, particle_t *p, plooks_t *type, unsigned int vertofs, unsigned int idxofs)
{
	vec3_t v, cr, o2;
	float  scale;

	scale = (p->org[0] - r_origin[0]) * vpn[0] + (p->org[1] - r_origin[1]) * vpn[1] + (p->org[2] - r_origin[2]) * vpn[2];
	scale = (scale * p->scale) * (type->invscalefactor) + p->scale * (type->scalefactor * 250);
	if (scale < 20)
		scale = 0.05;
	else
		scale = 0.05 + scale * 0.0001;

	if (type->premul)
	{
		vec4_t rgba;
		float  a = p->rgba[3];
		if (a > 1)
			a = 1;
		a *= 255.0f;
		rgba[0] = p->rgba[0] * a;
		rgba[1] = p->rgba[1] * a;
		rgba[2] = p->rgba[2] * a;
		rgba[3] = (type->premul == 2) ? 0 : a;
		Vector4ToColor (rgba, cl_curstrisvert[vertofs + 0].color);
		Vector4ToColor (rgba, cl_curstrisvert[vertofs + 1].color);
		Vector4ToColor (rgba, cl_curstrisvert[vertofs + 2].color);
	}
	else
	{
		Vector4ToColor (p->rgba, cl_curstrisvert[vertofs + 0].color);
		Vector4ToColor (p->rgba, cl_curstrisvert[vertofs + 1].color);
		Vector4ToColor (p->rgba, cl_curstrisvert[vertofs + 2].color);
	}

	Vector2Set (cl_curstrisvert[vertofs + 0].texcoord, p->s1, p->t1);
	Vector2Set (cl_curstrisvert[vertofs + 1].texcoord, p->s1, p->t2);
	Vector2Set (cl_curstrisvert[vertofs + 2].texcoord, p->s2, p->t1);

	VectorMA (p->org, -scale, p->vel, o2);
	VectorSubtract (r_refdef.vieworg, o2, v);
	CrossProduct (v, p->vel, cr);
	VectorNormalize (cr);

	VectorCopy (p->org, cl_curstrisvert[vertofs + 0].position);
	VectorMA (o2, -p->scale, cr, cl_curstrisvert[vertofs + 1].position);
	VectorMA (o2, p->scale, cr, cl_curstrisvert[vertofs + 2].position);

	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 0;
	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 1;
	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 2;
}

static void R_AddFanSparkParticle (scenetris_t *t, particle_t *p, plooks_t *type)
{
	if (cl_numstrisvert + 3 > cl_maxstrisvert[current_buffer_index])
		ReallocateVertexBuffer ();
	if (cl_numstrisidx + 3 > cl_maxstrisidx[current_buffer_index])
		ReallocateIndexBuffer ();
	R_EmitFanSparkParticle (t, p, type, cl_numstrisvert, cl_numstrisidx);
	cl_numstrisvert += 3;
	cl_numstrisidx += 3;
	t->numvert += 3;
	t->numidx += 3;
}

static void R_EmitLineSparkParticle (scenetris_t *t, particle_t *p, plooks_t *type, unsigned int vertofs, unsigned int idxofs)
{

	if (type->premul)
	{
		vec4_t scaled_color;
		float  a = p->rgba[3];
		if (a > 1)
			a = 1;
		VectorScale (p->rgba, a, scaled_color);
		Vector3ToColor (scaled_color, cl_curstrisvert[vertofs + 0].color);
		FloatToColor ((type->premul == 2) ? 0 : a, cl_curstrisvert[vertofs + 0].color[3]);
		Vector4Clear (cl_curstrisvert[vertofs + 1].color);
	}
	else
	{
		Vector4ToColor (p->rgba, cl_curstrisvert[vertofs + 0].color);
		Vector3ToColor (p->rgba, cl_curstrisvert[vertofs + 1].color);
		cl_curstrisvert[vertofs + 1].color[3] = 0;
	}
	Vector2Set (cl_curstrisvert[vertofs + 0].texcoord, p->s1, p->t1);
	Vector2Set (cl_curstrisvert[vertofs + 1].texcoord, p->s2, p->t2);

	VectorCopy (p->org, cl_curstrisvert[vertofs + 0].position);
	VectorMA (p->org, -1.0 / 10, p->vel, cl_curstrisvert[vertofs + 1].position);

	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 0;
	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 1;
}

static void R_AddLineSparkParticle (scenetris_t *t, particle_t *p, plooks_t *type)
{
	if (cl_numstrisvert + 2 > cl_maxstrisvert[current_buffer_index])
		ReallocateVertexBuffer ();
	if (cl_numstrisidx + 2 > cl_maxstrisidx[current_buffer_index])
		ReallocateIndexBuffer ();
	R_EmitLineSparkParticle (t, p, type, cl_numstrisvert, cl_numstrisidx);
	cl_numstrisvert += 2;
	cl_numstrisidx += 2;
	t->numvert += 2;
	t->numidx += 2;
}

static void R_EmitTSparkParticle (scenetris_t *t, particle_t *p, plooks_t *type, unsigned int vertofs, unsigned int idxofs)
{
	vec3_t v, cr, o2;

	if (type->premul)
	{
		vec4_t rgba;
		float  a = p->rgba[3];
		if (a > 1)
			a = 1;
		rgba[0] = p->rgba[0] * a;
		rgba[1] = p->rgba[1] * a;
		rgba[2] = p->rgba[2] * a;
		rgba[3] = (type->premul == 2) ? 0 : a;
		Vector4ToColor (rgba, cl_curstrisvert[vertofs + 0].color);
		Vector4ToColor (rgba, cl_curstrisvert[vertofs + 1].color);
		Vector4ToColor (rgba, cl_curstrisvert[vertofs + 2].color);
		Vector4ToColor (rgba, cl_curstrisvert[vertofs + 3].color);
	}
	else
	{
		Vector4ToColor (p->rgba, cl_curstrisvert[vertofs + 0].color);
		Vector4ToColor (p->rgba, cl_curstrisvert[vertofs + 1].color);
		Vector4ToColor (p->rgba, cl_curstrisvert[vertofs + 2].color);
		Vector4ToColor (p->rgba, cl_curstrisvert[vertofs + 3].color);
	}

	Vector2Set (cl_curstrisvert[vertofs + 0].texcoord, p->s1, p->t1);
	Vector2Set (cl_curstrisvert[vertofs + 1].texcoord, p->s1, p->t2);
	Vector2Set (cl_curstrisvert[vertofs + 2].texcoord, p->s2, p->t2);
	Vector2Set (cl_curstrisvert[vertofs + 3].texcoord, p->s2, p->t1);

	{
		vec3_t movedir;
		float  halfscale = p->scale * 0.5;
		float  length = VectorNormalize2 (p->vel, movedir);
		if (type->stretch < 0)
			length = -type->stretch; // fixed lengths
		else if (type->stretch)
			length *= type->stretch; // velocity multiplier
		else
			Sys_Error ("type->stretch should be 0.05\n");
		//			length *= 0.05;				//fallback

		if (length < halfscale * type->minstretch)
			length = halfscale * type->minstretch;

		VectorMA (p->org, -length, movedir, o2);
		VectorSubtract (r_refdef.vieworg, o2, v);
		CrossProduct (v, p->vel, cr);
		VectorNormalize (cr);
		VectorMA (o2, -p->scale / 2, cr, cl_curstrisvert[vertofs + 0].position);
		VectorMA (o2, p->scale / 2, cr, cl_curstrisvert[vertofs + 1].position);

		VectorMA (p->org, length, movedir, o2);
	}

	VectorSubtract (r_refdef.vieworg, o2, v);
	CrossProduct (v, p->vel, cr);
	VectorNormalize (cr);

	VectorMA (o2, p->scale * 0.5, cr, cl_curstrisvert[vertofs + 2].position);
	VectorMA (o2, -p->scale * 0.5, cr, cl_curstrisvert[vertofs + 3].position);

	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 0;
	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 1;
	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 2;
	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 0;
	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 2;
	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 3;
}

static void R_AddTSparkParticle (scenetris_t *t, particle_t *p, plooks_t *type)
{
	if (cl_numstrisvert + 4 > cl_maxstrisvert[current_buffer_index])
		ReallocateVertexBuffer ();
	if (cl_numstrisidx + 6 > cl_maxstrisidx[current_buffer_index])
		ReallocateIndexBuffer ();
	R_EmitTSparkParticle (t, p, type, cl_numstrisvert, cl_numstrisidx);
	cl_numstrisvert += 4;
	cl_numstrisidx += 6;
	t->numvert += 4;
	t->numidx += 6;
}

static void R_DrawParticleBeam (scenetris_t *t, beamseg_t *b, plooks_t *type)
{
	vec3_t		v;
	vec3_t		cr;
	beamseg_t  *c;
	particle_t *p;
	particle_t *q;
	float		ts;

	c = b->next;

	q = c->p;
	if (!q)
		return;
	p = b->p;

	if (cl_numstrisvert + 4 > cl_maxstrisvert[current_buffer_index])
		ReallocateVertexBuffer ();

	VectorSubtract (r_refdef.vieworg, q->org, v);
	VectorNormalize (v);
	CrossProduct (c->dir, v, cr);
	VectorNormalize (cr);
	ts = c->texture_s * q->angle + particletime * q->rotationspeed;
	Vector4ToColor (q->rgba, cl_curstrisvert[cl_numstrisvert + 0].color);
	Vector4ToColor (q->rgba, cl_curstrisvert[cl_numstrisvert + 1].color);
	Vector2Set (cl_curstrisvert[cl_numstrisvert + 0].texcoord, ts, p->t1);
	Vector2Set (cl_curstrisvert[cl_numstrisvert + 1].texcoord, ts, p->t2);
	VectorMA (q->org, -q->scale, cr, cl_curstrisvert[cl_numstrisvert + 0].position);
	VectorMA (q->org, q->scale, cr, cl_curstrisvert[cl_numstrisvert + 1].position);

	VectorSubtract (r_refdef.vieworg, p->org, v);
	VectorNormalize (v);
	CrossProduct (b->dir, v, cr); // replace with old p->dir?
	VectorNormalize (cr);
	ts = b->texture_s * p->angle + particletime * p->rotationspeed;
	Vector4ToColor (p->rgba, cl_curstrisvert[cl_numstrisvert + 2].color);
	Vector4ToColor (p->rgba, cl_curstrisvert[cl_numstrisvert + 3].color);
	Vector2Set (cl_curstrisvert[cl_numstrisvert + 2].texcoord, ts, p->t2);
	Vector2Set (cl_curstrisvert[cl_numstrisvert + 3].texcoord, ts, p->t1);
	VectorMA (p->org, p->scale, cr, cl_curstrisvert[cl_numstrisvert + 2].position);
	VectorMA (p->org, -p->scale, cr, cl_curstrisvert[cl_numstrisvert + 3].position);

	t->numvert += 4;

	if (cl_numstrisidx + 6 > cl_maxstrisidx[current_buffer_index])
		ReallocateIndexBuffer ();

	cl_curstrisidx[cl_numstrisidx++] = (cl_numstrisvert - t->firstvert) + 0;
	cl_curstrisidx[cl_numstrisidx++] = (cl_numstrisvert - t->firstvert) + 1;
	cl_curstrisidx[cl_numstrisidx++] = (cl_numstrisvert - t->firstvert) + 2;
	cl_curstrisidx[cl_numstrisidx++] = (cl_numstrisvert - t->firstvert) + 0;
	cl_curstrisidx[cl_numstrisidx++] = (cl_numstrisvert - t->firstvert) + 2;
	cl_curstrisidx[cl_numstrisidx++] = (cl_numstrisvert - t->firstvert) + 3;
	cl_numstrisvert += 4;
	t->numidx += 4;
}

static void R_AddClippedDecal (scenetris_t *t, clippeddecal_t *d, plooks_t *type)
{
	if (cl_numstrisvert + 4 > cl_maxstrisvert[current_buffer_index])
		ReallocateVertexBuffer ();

	if (d->entity > 0)
	{
		entity_t *le = CL_EntityNum (d->entity);
		if (le->angles[0] || le->angles[1] || le->angles[2])
		{ // FIXME: deal with rotated entities.
			d->die = -1;
			return;
		}
		VectorAdd (d->vertex[0], le->origin, cl_curstrisvert[cl_numstrisvert + 0].position);
		VectorAdd (d->vertex[1], le->origin, cl_curstrisvert[cl_numstrisvert + 1].position);
		VectorAdd (d->vertex[2], le->origin, cl_curstrisvert[cl_numstrisvert + 2].position);
	}
	else
	{
		VectorCopy (d->vertex[0], cl_curstrisvert[cl_numstrisvert + 0].position);
		VectorCopy (d->vertex[1], cl_curstrisvert[cl_numstrisvert + 1].position);
		VectorCopy (d->vertex[2], cl_curstrisvert[cl_numstrisvert + 2].position);
	}

	if (type->premul)
	{
		vec4_t rgba;
		vec4_t scaled_color;
		float  a = d->rgba[3];
		if (a > 1)
			a = 1;
		rgba[0] = d->rgba[0] * a;
		rgba[1] = d->rgba[1] * a;
		rgba[2] = d->rgba[2] * a;
		rgba[3] = (type->premul == 2) ? 0 : a;
		Vector4Scale (rgba, d->valpha[0], scaled_color);
		Vector4ToColor (scaled_color, cl_curstrisvert[cl_numstrisvert + 0].color);
		Vector4Scale (rgba, d->valpha[1], scaled_color);
		Vector4ToColor (scaled_color, cl_curstrisvert[cl_numstrisvert + 1].color);
		Vector4Scale (rgba, d->valpha[2], scaled_color);
		Vector4ToColor (scaled_color, cl_curstrisvert[cl_numstrisvert + 2].color);
	}
	else
	{
		vec4_t rgba;
		rgba[0] = d->rgba[0];
		rgba[1] = d->rgba[1];
		rgba[2] = d->rgba[2];
		rgba[3] = d->rgba[3] * d->valpha[0];
		Vector4ToColor (rgba, cl_curstrisvert[cl_numstrisvert + 0].color);
		rgba[3] = d->rgba[3] * d->valpha[1];
		Vector4ToColor (rgba, cl_curstrisvert[cl_numstrisvert + 1].color);
		rgba[3] = d->rgba[3] * d->valpha[2];
		Vector4ToColor (rgba, cl_curstrisvert[cl_numstrisvert + 2].color);
	}

	Vector2Copy (d->texcoords[0], cl_curstrisvert[cl_numstrisvert + 0].texcoord);
	Vector2Copy (d->texcoords[1], cl_curstrisvert[cl_numstrisvert + 1].texcoord);
	Vector2Copy (d->texcoords[2], cl_curstrisvert[cl_numstrisvert + 2].texcoord);

	if (cl_numstrisidx + 3 > cl_maxstrisidx[current_buffer_index])
		ReallocateIndexBuffer ();

	cl_curstrisidx[cl_numstrisidx++] = (cl_numstrisvert - t->firstvert) + 0;
	cl_curstrisidx[cl_numstrisidx++] = (cl_numstrisvert - t->firstvert) + 1;
	cl_curstrisidx[cl_numstrisidx++] = (cl_numstrisvert - t->firstvert) + 2;

	cl_numstrisvert += 3;

	t->numvert += 3;
	t->numidx += 3;
}

static void R_EmitUnclippedDecal (scenetris_t *t, particle_t *p, plooks_t *type, unsigned int vertofs, unsigned int idxofs)
{
	float  x, y;
	vec3_t sdir, tdir;

	if (type->premul)
	{
		vec4_t rgba;
		float  a = p->rgba[3];
		if (a > 1)
			a = 1;
		rgba[0] = p->rgba[0] * a;
		rgba[1] = p->rgba[1] * a;
		rgba[2] = p->rgba[2] * a;
		rgba[3] = (type->premul == 2) ? 0 : a;
		Vector4ToColor (rgba, cl_curstrisvert[vertofs + 0].color);
		Vector4ToColor (rgba, cl_curstrisvert[vertofs + 1].color);
		Vector4ToColor (rgba, cl_curstrisvert[vertofs + 2].color);
		Vector4ToColor (rgba, cl_curstrisvert[vertofs + 3].color);
	}
	else
	{
		Vector4ToColor (p->rgba, cl_curstrisvert[vertofs + 0].color);
		Vector4ToColor (p->rgba, cl_curstrisvert[vertofs + 1].color);
		Vector4ToColor (p->rgba, cl_curstrisvert[vertofs + 2].color);
		Vector4ToColor (p->rgba, cl_curstrisvert[vertofs + 3].color);
	}

	Vector2Set (cl_curstrisvert[vertofs + 0].texcoord, p->s1, p->t1);
	Vector2Set (cl_curstrisvert[vertofs + 1].texcoord, p->s1, p->t2);
	Vector2Set (cl_curstrisvert[vertofs + 2].texcoord, p->s2, p->t2);
	Vector2Set (cl_curstrisvert[vertofs + 3].texcoord, p->s2, p->t1);

	//	if (p->vel[1] == 1)
	{
		VectorSet (sdir, 1, 0, 0);
		VectorSet (tdir, 0, 1, 0);
	}

	if (p->angle)
	{
		x = sin (p->angle) * p->scale;
		y = cos (p->angle) * p->scale;

		cl_curstrisvert[vertofs + 0].position[0] = p->org[0] - x * sdir[0] - y * tdir[0];
		cl_curstrisvert[vertofs + 0].position[1] = p->org[1] - x * sdir[1] - y * tdir[1];
		cl_curstrisvert[vertofs + 0].position[2] = p->org[2] - x * sdir[2] - y * tdir[2];
		cl_curstrisvert[vertofs + 1].position[0] = p->org[0] - y * sdir[0] + x * tdir[0];
		cl_curstrisvert[vertofs + 1].position[1] = p->org[1] - y * sdir[1] + x * tdir[1];
		cl_curstrisvert[vertofs + 1].position[2] = p->org[2] - y * sdir[2] + x * tdir[2];
		cl_curstrisvert[vertofs + 2].position[0] = p->org[0] + x * sdir[0] + y * tdir[0];
		cl_curstrisvert[vertofs + 2].position[1] = p->org[1] + x * sdir[1] + y * tdir[1];
		cl_curstrisvert[vertofs + 2].position[2] = p->org[2] + x * sdir[2] + y * tdir[2];
		cl_curstrisvert[vertofs + 3].position[0] = p->org[0] + y * sdir[0] - x * tdir[0];
		cl_curstrisvert[vertofs + 3].position[1] = p->org[1] + y * sdir[1] - x * tdir[1];
		cl_curstrisvert[vertofs + 3].position[2] = p->org[2] + y * sdir[2] - x * tdir[2];
	}
	else
	{
		VectorMA (p->org, -p->scale, tdir, cl_curstrisvert[vertofs + 0].position);
		VectorMA (p->org, -p->scale, sdir, cl_curstrisvert[vertofs + 1].position);
		VectorMA (p->org, p->scale, tdir, cl_curstrisvert[vertofs + 2].position);
		VectorMA (p->org, p->scale, sdir, cl_curstrisvert[vertofs + 3].position);
	}

	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 0;
	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 1;
	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 2;
	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 0;
	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 2;
	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 3;
}

static void R_AddUnclippedDecal (scenetris_t *t, particle_t *p, plooks_t *type)
{
	if (cl_numstrisvert + 4 > cl_maxstrisvert[current_buffer_index])
		ReallocateVertexBuffer ();
	if (cl_numstrisidx + 6 > cl_maxstrisidx[current_buffer_index])
		ReallocateIndexBuffer ();
	R_EmitUnclippedDecal (t, p, type, cl_numstrisvert, cl_numstrisidx);
	cl_numstrisvert += 4;
	cl_numstrisidx += 6;
	t->numvert += 4;
	t->numidx += 6;
}

static void R_EmitTexturedParticle (scenetris_t *t, particle_t *p, plooks_t *type, unsigned int vertofs, unsigned int idxofs)
{
	float scale, x, y;

	if (type->scalefactor == 1)
		scale = p->scale * 0.25;
	else
	{
		scale = (p->org[0] - r_origin[0]) * vpn[0] + (p->org[1] - r_origin[1]) * vpn[1] + (p->org[2] - r_origin[2]) * vpn[2];
		scale = (scale * p->scale) * (type->invscalefactor) + p->scale * (type->scalefactor * 250);
		if (scale < 20)
			scale = 0.25;
		else
			scale = 0.25 + scale * 0.001;
	}

	if (type->premul)
	{
		vec4_t rgba;
		float  a = p->rgba[3];
		if (a > 1)
			a = 1;
		rgba[0] = p->rgba[0] * a;
		rgba[1] = p->rgba[1] * a;
		rgba[2] = p->rgba[2] * a;
		rgba[3] = (type->premul == 2) ? 0 : a;
		Vector4ToColor (rgba, cl_curstrisvert[vertofs + 0].color);
		Vector4ToColor (rgba, cl_curstrisvert[vertofs + 1].color);
		Vector4ToColor (rgba, cl_curstrisvert[vertofs + 2].color);
		Vector4ToColor (rgba, cl_curstrisvert[vertofs + 3].color);
	}
	else
	{
		Vector4ToColor (p->rgba, cl_curstrisvert[vertofs + 0].color);
		Vector4ToColor (p->rgba, cl_curstrisvert[vertofs + 1].color);
		Vector4ToColor (p->rgba, cl_curstrisvert[vertofs + 2].color);
		Vector4ToColor (p->rgba, cl_curstrisvert[vertofs + 3].color);
	}

	Vector2Set (cl_curstrisvert[vertofs + 0].texcoord, p->s1, p->t1);
	Vector2Set (cl_curstrisvert[vertofs + 1].texcoord, p->s1, p->t2);
	Vector2Set (cl_curstrisvert[vertofs + 2].texcoord, p->s2, p->t2);
	Vector2Set (cl_curstrisvert[vertofs + 3].texcoord, p->s2, p->t1);

	if (p->angle)
	{
		x = sin (p->angle) * scale;
		y = cos (p->angle) * scale;

		cl_curstrisvert[vertofs + 0].position[0] = p->org[0] - x * pright[0] - y * pup[0];
		cl_curstrisvert[vertofs + 0].position[1] = p->org[1] - x * pright[1] - y * pup[1];
		cl_curstrisvert[vertofs + 0].position[2] = p->org[2] - x * pright[2] - y * pup[2];
		cl_curstrisvert[vertofs + 1].position[0] = p->org[0] - y * pright[0] + x * pup[0];
		cl_curstrisvert[vertofs + 1].position[1] = p->org[1] - y * pright[1] + x * pup[1];
		cl_curstrisvert[vertofs + 1].position[2] = p->org[2] - y * pright[2] + x * pup[2];
		cl_curstrisvert[vertofs + 2].position[0] = p->org[0] + x * pright[0] + y * pup[0];
		cl_curstrisvert[vertofs + 2].position[1] = p->org[1] + x * pright[1] + y * pup[1];
		cl_curstrisvert[vertofs + 2].position[2] = p->org[2] + x * pright[2] + y * pup[2];
		cl_curstrisvert[vertofs + 3].position[0] = p->org[0] + y * pright[0] - x * pup[0];
		cl_curstrisvert[vertofs + 3].position[1] = p->org[1] + y * pright[1] - x * pup[1];
		cl_curstrisvert[vertofs + 3].position[2] = p->org[2] + y * pright[2] - x * pup[2];
	}
	else
	{
		VectorMA (p->org, -scale, pup, cl_curstrisvert[vertofs + 0].position);
		VectorMA (p->org, -scale, pright, cl_curstrisvert[vertofs + 1].position);
		VectorMA (p->org, scale, pup, cl_curstrisvert[vertofs + 2].position);
		VectorMA (p->org, scale, pright, cl_curstrisvert[vertofs + 3].position);
	}

	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 0;
	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 1;
	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 2;
	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 0;
	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 2;
	cl_curstrisidx[idxofs++] = (vertofs - t->firstvert) + 3;
}

static void R_AddTexturedParticle (scenetris_t *t, particle_t *p, plooks_t *type)
{
	if (cl_numstrisvert + 4 > cl_maxstrisvert[current_buffer_index])
		ReallocateVertexBuffer ();
	if (cl_numstrisidx + 6 > cl_maxstrisidx[current_buffer_index])
		ReallocateIndexBuffer ();
	R_EmitTexturedParticle (t, p, type, cl_numstrisvert, cl_numstrisidx);
	cl_numstrisvert += 4;
	cl_numstrisidx += 6;
	t->numvert += 4;
	t->numidx += 6;
}

static void PScript_DrawParticleBatches (cb_context_t *cbx, qboolean draw_oit_batches, qboolean split_batches)
{
	unsigned int i, o;

	if (!cbx || !cl_numstris)
		return;

	R_BeginDebugUtilsLabel (cbx, draw_oit_batches ? "FTE Particles OIT" : "FTE Particles");

	for (o = 0; o < 3; o++)
	{
		static int blend_modes_order[] = {1, 1, 2, 2, 0, 0, 0, 2};
		for (i = 0; i < cl_numstris; i++)
		{
			scenetris_t *tris = &cl_stris[i];
			const int	 blend_mode = tris->blendmode;
			if (split_batches && tris->use_oit != draw_oit_batches)
				continue;
			if (blend_modes_order[blend_mode] != o)
				continue;
			const qboolean draw_lines = ((tris->beflags & BEF_LINES) != 0);
			if (!vulkan_globals.non_solid_fill && draw_lines)
				continue; // Can't draw lines
			if (tris->numidx == 0)
				continue;

			const int						 pipeline_index = blend_mode + (draw_lines ? 8 : 0);
			const main_render_pass_variant_t main_pass_variant = R_MainPassPipelineVariant (cbx->render_pass_index);
			const vulkan_pipeline_t			 pipeline = draw_oit_batches	? vulkan_globals.fte_particle_wboit_pipelines[pipeline_index]
														: cbx->subpass != 0 ? vulkan_globals.fte_particle_post_oit_pipelines[main_pass_variant][pipeline_index]
																			: vulkan_globals.fte_particle_pipelines[main_pass_variant][pipeline_index];
			R_BindPipeline (cbx, VK_PIPELINE_BIND_POINT_GRAPHICS, pipeline);
			R_PushConstants (cbx, VK_SHADER_STAGE_ALL_GRAPHICS, 0, 16 * sizeof (float), vulkan_globals.view_projection_matrix);
			Fog_DisableGFog (cbx);
			gltexture_t *tex = (tris->beflags & BEF_LINES) ? whitetexture : tris->texture;

			const int		   num_indices = tris->numidx;
			const VkDeviceSize vertex_buffer_offset = 0;
			vulkan_globals.vk_cmd_bind_index_buffer (cbx->cb, index_buffers[current_buffer_index], 0, VK_INDEX_TYPE_UINT16);
			vulkan_globals.vk_cmd_bind_vertex_buffers (cbx->cb, 0, 1, &vertex_buffers[current_buffer_index], &vertex_buffer_offset);
			vulkan_globals.vk_cmd_bind_descriptor_sets (cbx->cb, VK_PIPELINE_BIND_POINT_GRAPHICS, pipeline.layout.handle, 0, 1, &tex->descriptor_set, 0, NULL);
			vulkan_globals.vk_cmd_draw_indexed (cbx->cb, num_indices, 1, tris->firstidx, tris->firstvert, 0);
		}
	}
	R_EndDebugUtilsLabel (cbx);
}

// Deferred spawns and the flat update list decouple the per particle update from the
// per type linked lists: the update never mutates any list and never spawns into other
// types, so it only touches the particle itself and can eventually run in parallel
typedef struct deferred_effect_s
{
	vec3_t org;
	vec3_t dir;
	float  count;
	int	   type;
} deferred_effect_t;

typedef struct deferred_trail_s
{
	vec3_t		   start;
	vec3_t		   end;
	int			   type;
	trailstate_t **tsk;
} deferred_trail_t;

typedef struct deferred_decal_s
{
	part_type_t *type;
	int			 entity;
	vec3_t		 center;
	vec3_t		 normal;
	float		 scale;
} deferred_decal_t;

typedef struct deferred_dlight_s
{
	int	   key;
	vec3_t org;
	float  radius;
	float  die;
	float  decay;
	vec3_t rgb;
} deferred_dlight_t;

typedef struct particle_update_s
{
	particle_t	*p;
	part_type_t *type;
} particle_update_t;
// each task worker queues into its own arrays so the parallel update never contends
typedef struct deferred_queues_s
{
	deferred_effect_t *effects;
	int				   num_effects, max_effects;
	deferred_trail_t  *trails;
	int				   num_trails, max_trails;
	deferred_decal_t  *decals;
	int				   num_decals, max_decals;
	deferred_dlight_t *dlights;
	int				   num_dlights, max_dlights;
} deferred_queues_t;

/* r_part_fte.c:6318-6323. particle_trace_limit and particle_update_seed
 * (:6322-6323) are written and read only by the ported half and moved to
 * Rust; the rest keep C storage because PScript_UpdateParticleTypes (:7226)
 * and PScript_EmitParticlesTask (:7339) still read them. */
deferred_queues_t  deferred_queues[TASKS_MAX_WORKERS];
particle_update_t *particle_updates;
int				   num_particle_updates, max_particle_updates;
atomic_uint32_t	   particle_traces_used;

// per type draw metadata: every draw function emits a fixed number of vertices/indices
// per particle, so the serial layout pass only reserves ranges and the vertices are
// written in parallel by PScript_EmitParticlesTask with pure arithmetic addressing
typedef struct particle_emit_meta_s
{
	int start, count;  // contiguous segment of this type's particles in particle_updates
	int first_stri;	   // first of the consecutive batches reserved for this type
	int vpp, ipp, ppb; // vertices/indices per particle, particles per batch
	void (*emit_core) (scenetris_t *t, particle_t *p, plooks_t *type, unsigned int vertofs, unsigned int idxofs);
} particle_emit_meta_t;
/* r_part_fte.c:6428-6429 -- indexed by `type - part_type` from both halves
 * (:6979 in the C tail, and the Rust setup task). */
particle_emit_meta_t *type_emit_meta;
int					  num_type_emit_meta;

/* r_part_fte.c:6585-6589. p_doflurry (:6587) is written and read only by the
 * ported half and moved to Rust. */
#define PARTICLE_UPDATE_CHUNK_SIZE 1024

float		p_frametime;
particle_t *p_kill_list, *p_kill_first; // the kill list is to stop particles from being freed and reused whilst still in this frame
										// which is bad because beams need to find out when particles died. Reuse can do wierd things.
										// remember that they're not drawn instantly either.

static void PScript_UpdateParticleTypes (float pframetime)
{
	void (*bdraw) (scenetris_t *t, beamseg_t *p, plooks_t *type);
	void (*tdraw) (scenetris_t *t, particle_t *p, plooks_t *type);
	void (*emit_core) (scenetris_t *t, particle_t *p, plooks_t *type, unsigned int vertofs, unsigned int idxofs);
	int vpp, ipp;

	vec3_t			oldorg;
	vec3_t			stop;
	part_type_t	   *type, *lastvalidtype;
	particle_t	   *p;
	clippeddecal_t *d, *dkill;
	ramp_t		   *ramp;
	scenetris_t	   *scenetri;
	particle_t	   *kill_list = p_kill_list, *kill_first = p_kill_first;
	beamseg_t	   *b, *bkill;

	int rampind;
	int batchflags;

	for (type = part_run_list, lastvalidtype = NULL; type != NULL; type = type->nexttorun)
	{
		if (type->clippeddecals)
		{
			const qboolean use_oit = PScript_LooksUseWBOIT (type->slooks);
			if (cl_numstris && cl_stris[cl_numstris - 1].texture == type->looks.texture && cl_stris[cl_numstris - 1].blendmode == type->looks.blendmode &&
				cl_stris[cl_numstris - 1].beflags == 0 && cl_stris[cl_numstris - 1].use_oit == use_oit)
				scenetri = &cl_stris[cl_numstris - 1];
			else
			{
				if (cl_numstris == cl_maxstris)
				{
					cl_maxstris += 8;
					cl_stris = Mem_Realloc (cl_stris, sizeof (*cl_stris) * cl_maxstris);
				}
				scenetri = &cl_stris[cl_numstris++];
				scenetri->texture = type->looks.texture;
				scenetri->blendmode = type->looks.blendmode;
				scenetri->beflags = 0;
				scenetri->use_oit = use_oit;
				scenetri->firstidx = cl_numstrisidx;
				scenetri->firstvert = cl_numstrisvert;
				scenetri->numvert = 0;
				scenetri->numidx = 0;
			}

			for (;;)
			{
				dkill = type->clippeddecals;
				if (dkill && dkill->die < particletime)
				{
					type->clippeddecals = dkill->next;
					dkill->next = free_decals;
					free_decals = dkill;
					continue;
				}
				break;
			}
			for (d = type->clippeddecals; d; d = d->next)
			{
				for (;;)
				{
					dkill = d->next;
					if (dkill && dkill->die < particletime)
					{
						d->next = dkill->next;
						dkill->next = free_decals;
						free_decals = dkill;
						continue;
					}
					break;
				}

				if (d->die - particletime <= type->die)
				{
					switch (type->rampmode)
					{
					case RAMP_NEAREST:
						rampind = (int)(type->rampindexes * (type->die - (d->die - particletime)) / type->die);
						if (rampind >= type->rampindexes)
							rampind = type->rampindexes - 1;
						ramp = type->ramp + rampind;
						VectorCopy (ramp->rgb, d->rgba);
						d->rgba[3] = ramp->alpha;
						break;
					case RAMP_LERP:
					{
						float frac = (type->rampindexes * (type->die - (d->die - particletime)) / type->die);
						int	  s1, s2;
						s1 = frac;
						s2 = s1 + 1;
						if (s1 > type->rampindexes - 1)
							s1 = type->rampindexes - 1;
						if (s2 > type->rampindexes - 1)
							s2 = type->rampindexes - 1;
						frac -= s1;
						VectorInterpolate (type->ramp[s1].rgb, frac, type->ramp[s2].rgb, d->rgba);
						FloatInterpolate (type->ramp[s1].alpha, frac, type->ramp[s2].alpha, d->rgba[3]);
					}
					break;
					case RAMP_DELTA: // particle ramps
						ramp = type->ramp + (int)(type->rampindexes * (type->die - (d->die - particletime)) / type->die);
						VectorMA (d->rgba, pframetime, ramp->rgb, d->rgba);
						d->rgba[3] -= pframetime * ramp->alpha;
						break;
					case RAMP_NONE: // particle changes acording to it's preset properties.
						if (particletime < (d->die - type->die + type->rgbchangetime))
						{
							d->rgba[0] += pframetime * type->rgbchange[0];
							d->rgba[1] += pframetime * type->rgbchange[1];
							d->rgba[2] += pframetime * type->rgbchange[2];
						}
						d->rgba[3] += pframetime * type->alphachange;
					}
				}

				if (cl_numstrisvert - scenetri->firstvert >= MAX_INDICES - 6)
				{
					// generate a new mesh if the old one overflowed. yay smc...
					if (cl_numstris == cl_maxstris)
					{
						cl_maxstris += 8;
						cl_stris = Mem_Realloc (cl_stris, sizeof (*cl_stris) * cl_maxstris);
					}
					scenetri = &cl_stris[cl_numstris++];
					scenetri->texture = scenetri[-1].texture;
					scenetri->blendmode = scenetri[-1].blendmode;
					scenetri->beflags = scenetri[-1].beflags;
					scenetri->use_oit = scenetri[-1].use_oit;
					scenetri->firstidx = cl_numstrisidx;
					scenetri->firstvert = cl_numstrisvert;
					scenetri->numvert = 0;
					scenetri->numidx = 0;
				}
				R_AddClippedDecal (scenetri, d, type->slooks);
			}
		}

		bdraw = NULL;
		tdraw = NULL;
		emit_core = NULL;
		batchflags = 0;
		vpp = ipp = 0;

		// set drawing methods by type and cvars and hope branch
		// prediction takes care of the rest
		switch (type->looks.type)
		{
		default:
		case PT_INVISIBLE:
			break;
		case PT_BEAM:
			bdraw = R_DrawParticleBeam;
			break;
		case PT_CDECAL:
			break;
		case PT_UDECAL:
			tdraw = R_AddUnclippedDecal;
			emit_core = R_EmitUnclippedDecal;
			vpp = 4;
			ipp = 6;
			break;
		case PT_NORMAL:
			tdraw = R_AddTexturedParticle;
			emit_core = R_EmitTexturedParticle;
			vpp = 4;
			ipp = 6;
			break;
		case PT_SPARK:
			tdraw = R_AddLineSparkParticle;
			emit_core = R_EmitLineSparkParticle;
			vpp = 2;
			ipp = 2;
			batchflags = BEF_LINES;
			break;
		case PT_SPARKFAN:
			tdraw = R_AddFanSparkParticle;
			emit_core = R_EmitFanSparkParticle;
			vpp = 3;
			ipp = 3;
			break;
		case PT_TEXTUREDSPARK:
			tdraw = R_AddTSparkParticle;
			emit_core = R_EmitTSparkParticle;
			vpp = 4;
			ipp = 6;
			break;
		}

		// types with a fixed size draw function only get their batches and vertex ranges
		// reserved here, the vertices are written in parallel by PScript_EmitParticlesTask
		if (emit_core && type->die)
		{
			particle_emit_meta_t *meta = &type_emit_meta[type - part_type];
			if (meta->count)
			{
				const qboolean use_oit = PScript_LooksUseWBOIT (type->slooks);
				int			   remaining = meta->count;

				meta->vpp = vpp;
				meta->ipp = ipp;
				meta->ppb = (MAX_INDICES - 8) / vpp;
				meta->first_stri = (int)cl_numstris;
				meta->emit_core = emit_core;

				while (remaining > 0)
				{
					const int n = q_min (remaining, meta->ppb);
					if (cl_numstris == cl_maxstris)
					{
						cl_maxstris += 8;
						cl_stris = Mem_Realloc (cl_stris, sizeof (*cl_stris) * cl_maxstris);
					}
					scenetri = &cl_stris[cl_numstris++];
					scenetri->texture = type->looks.texture;
					scenetri->blendmode = type->looks.blendmode;
					scenetri->beflags = batchflags;
					scenetri->use_oit = use_oit;
					scenetri->firstidx = cl_numstrisidx;
					scenetri->firstvert = cl_numstrisvert;
					scenetri->numvert = n * vpp;
					scenetri->numidx = n * ipp;
					while (cl_numstrisvert + n * vpp > cl_maxstrisvert[current_buffer_index])
						ReallocateVertexBuffer ();
					while (cl_numstrisidx + n * ipp > cl_maxstrisidx[current_buffer_index])
						ReallocateIndexBuffer ();
					cl_numstrisvert += n * vpp;
					cl_numstrisidx += n * ipp;
					remaining -= n;
				}
			}
			goto endtype;
		}

		const qboolean use_oit = PScript_LooksUseWBOIT (type->slooks);
		if (cl_numstris && cl_stris[cl_numstris - 1].texture == type->looks.texture && cl_stris[cl_numstris - 1].blendmode == type->looks.blendmode &&
			cl_stris[cl_numstris - 1].beflags == batchflags && cl_stris[cl_numstris - 1].use_oit == use_oit)
			scenetri = &cl_stris[cl_numstris - 1];
		else
		{
			if (cl_numstris == cl_maxstris)
			{
				cl_maxstris += 8;
				cl_stris = Mem_Realloc (cl_stris, sizeof (*cl_stris) * cl_maxstris);
			}
			scenetri = &cl_stris[cl_numstris++];
			scenetri->texture = type->looks.texture;
			scenetri->blendmode = type->looks.blendmode;
			scenetri->beflags = batchflags;
			scenetri->use_oit = use_oit;
			scenetri->firstidx = cl_numstrisidx;
			scenetri->firstvert = cl_numstrisvert;
			scenetri->numvert = 0;
			scenetri->numidx = 0;
		}

		if (!type->die)
		{
			while ((p = type->particles))
			{
				if (scenetri && tdraw)
				{
					if (cl_numstrisvert - scenetri->firstvert >= MAX_INDICES - 6)
					{
						// generate a new mesh if the old one overflowed. yay smc...
						if (cl_numstris == cl_maxstris)
						{
							cl_maxstris += 8;
							cl_stris = Mem_Realloc (cl_stris, sizeof (*cl_stris) * cl_maxstris);
						}
						scenetri = &cl_stris[cl_numstris++];
						scenetri->texture = scenetri[-1].texture;
						scenetri->blendmode = scenetri[-1].blendmode;
						scenetri->beflags = scenetri[-1].beflags;
						scenetri->use_oit = scenetri[-1].use_oit;
						scenetri->firstidx = cl_numstrisidx;
						scenetri->firstvert = cl_numstrisvert;
						scenetri->numvert = 0;
						scenetri->numidx = 0;
					}
					tdraw (scenetri, p, type->slooks);
				}

				// make sure emitter runs at least once
				if (type->emit >= 0 && type->emitstart <= 0)
					PScript_QueueEffect (p->org, p->vel, 1, type->emit);

				type->particles = p->next;
				p->next = kill_list;
				kill_list = p;
				if (!kill_first) // branch here is probably faster than list traversal later
					kill_first = p;
			}

			if (type->beams)
			{
				b = type->beams;
			}

			while ((b = type->beams) && (b->flags & BS_DEAD))
			{
				type->beams = b->next;
				b->next = free_beams;
				free_beams = b;
			}

			while (b)
			{
				if (!(b->flags & BS_NODRAW))
				{
					// no BS_NODRAW implies b->next != NULL
					// BS_NODRAW should imply b->next == NULL or b->next->flags & BS_DEAD
					VectorCopy (b->next->p->org, stop);
					VectorCopy (b->p->org, oldorg);
					VectorSubtract (stop, oldorg, b->next->dir);
					VectorNormalize (b->next->dir);
					if (bdraw)
						bdraw (scenetri, b, type->slooks);
				}

				// clean up dead entries ahead of current
				for (;;)
				{
					bkill = b->next;
					if (bkill && (bkill->flags & BS_DEAD))
					{
						b->next = bkill->next;
						bkill->next = free_beams;
						free_beams = bkill;
						continue;
					}
					break;
				}

				b->flags |= BS_DEAD;
				b = b->next;
			}

			goto endtype;
		}

		// beams are dealt with here

		// kill early entries
		for (;;)
		{
			bkill = type->beams;
			if (bkill && (bkill->flags & BS_DEAD || bkill->p->die < particletime) && !(bkill->flags & BS_LASTSEG))
			{
				type->beams = bkill->next;
				bkill->next = free_beams;
				free_beams = bkill;
				continue;
			}
			break;
		}

		b = type->beams;
		if (b)
		{
			for (;;)
			{
				if (b->next)
				{
					// mark dead entries
					if (b->flags & (BS_LASTSEG | BS_DEAD | BS_NODRAW))
					{
						// kill some more dead entries
						for (;;)
						{
							bkill = b->next;
							if (bkill && (bkill->flags & BS_DEAD) && !(bkill->flags & BS_LASTSEG))
							{
								b->next = bkill->next;
								bkill->next = free_beams;
								free_beams = bkill;
								continue;
							}
							break;
						}

						if (!bkill) // have to check so we don't hit NULL->next
							continue;
					}
					else
					{
						if (!(b->next->flags & BS_DEAD))
						{
							VectorCopy (b->next->p->org, stop);
							VectorCopy (b->p->org, oldorg);
							VectorSubtract (stop, oldorg, b->next->dir);
							VectorNormalize (b->next->dir);
							if (bdraw)
							{
								VectorAdd (stop, oldorg, stop);
								VectorScale (stop, 0.5, stop);
							}
						}

						if (b->p->die < particletime)
							b->flags |= BS_DEAD;
					}
				}
				else
				{
					if (b->p->die < particletime) // end of the list check
						b->flags |= BS_DEAD;

					break;
				}

				if (b->p->die < particletime)
					b->flags |= BS_DEAD;

				b = b->next;
			}
		}

	endtype:

		// delete from run list if necessary
		if (!type->particles && !type->beams && !type->clippeddecals)
		{
			if (!lastvalidtype)
				part_run_list = type->nexttorun;
			else if (lastvalidtype->nexttorun == type)
				lastvalidtype->nexttorun = type->nexttorun;
			else
				lastvalidtype->nexttorun->nexttorun = type->nexttorun;
			type->state &= ~PS_INRUNLIST;
		}
		else
			lastvalidtype = type;
	}

	// run the spawns that were queued during the update. New particles get their first
	// update and draw next frame, which also gives every emitted effect the same timing
	// instead of depending on the run list order of its parent type
	for (int w = 0; w < TASKS_MAX_WORKERS; w++)
	{
		deferred_queues_t *queue = &deferred_queues[w];
		for (int fx = 0; fx < queue->num_trails; fx++)
			PScript_ParticleTrail (queue->trails[fx].start, queue->trails[fx].end, queue->trails[fx].type, pframetime, 0, NULL, queue->trails[fx].tsk);
		queue->num_trails = 0;
		for (int fx = 0; fx < queue->num_effects; fx++)
			PScript_RunParticleEffectState (queue->effects[fx].org, queue->effects[fx].dir, queue->effects[fx].count, queue->effects[fx].type, NULL);
		queue->num_effects = 0;
#ifdef USE_DECALS
		for (int fx = 0; fx < queue->num_decals; fx++)
		{
			deferred_decal_t *dd = &queue->decals[fx];
			part_type_t		 *dtype = dd->type;
			decalctx_t		  ctx;
			float			  m;
			vec3_t			  vec = {0.5, 0.5, 0.431};
			qmodel_t		 *model;

			ctx.entity = dd->entity;
			if (!ctx.entity)
			{
				model = cl.worldmodel;
				VectorCopy (dd->center, ctx.center);
			}
			else
			{ // this trace hit a door or something.
				entity_t *ent = CL_EntityNum (ctx.entity);
				model = ent->model;
				VectorSubtract (dd->center, ent->origin, ctx.center);
				// FIXME: rotate center+normal around entity.
			}

			VectorScale (dd->normal, -1, ctx.normal);
			VectorNormalize (ctx.normal);

			VectorNormalize (vec);
			CrossProduct (ctx.normal, vec, ctx.tangent1);
			RotatePointAroundVector (ctx.tangent2, ctx.normal, ctx.tangent1, frandom () * 360);
			CrossProduct (ctx.normal, ctx.tangent2, ctx.tangent1);

			VectorNormalize (ctx.tangent1);
			VectorNormalize (ctx.tangent2);

			ctx.ptype = dtype;
			ctx.scale1 = dtype->s2 - dtype->s1;
			ctx.bias1 = dtype->s1 + (ctx.scale1 * 0.5);
			ctx.scale2 = dtype->t2 - dtype->t1;
			ctx.bias2 = dtype->t1 + (ctx.scale2 * 0.5);
			m = dd->scale * (1.5 + frandom () * 0.5) * 0.5; // decals should be a little bigger, for some reason.
			ctx.scale0 = 2.0 / m;
			ctx.scale1 /= m;
			ctx.scale2 /= m;

			// inserts decals through a callback.
			Mod_ClipDecal (model, ctx.center, ctx.normal, ctx.tangent2, ctx.tangent1, m, dtype->surfflagmask, dtype->surfflagmatch, PScript_AddDecals, &ctx);
		}
		queue->num_decals = 0;
#endif
	}

	// lazy delete for particles is done here
	if (kill_list)
	{
		kill_first->next = free_particles;
		free_particles = kill_list;
	}

	particletime += pframetime;
}

/*
===============
PScript_DrawParticles
===============
*/
/*
===============
PScript_LayoutParticlesTask

Serial: batch creation and vertex range reservation for the fixed size particle
types, decals, beams and zero lifetime drains, then the deferred spawns
===============
*/
void PScript_LayoutParticlesTask (void *unused)
{
	current_buffer_index = (current_buffer_index + 1) % 2;
	cl_numstris = 0;
	cl_numstrisvert = 0;
	cl_numstrisidx = 0;
	cl_curstrisvert = cl_strisvert[current_buffer_index];
	cl_curstrisidx = cl_strisidx[current_buffer_index];

	if (!r_particles.value)
		return;

	PScript_UpdateParticleTypes (p_frametime);
}

/*
===============
PScript_EmitParticlesTask

Indexed over the worker count: fills the vertex ranges reserved by the layout.
Every slot is addressed arithmetically so the workers never contend
===============
*/
void PScript_EmitParticlesTask (int index, void *unused)
{
	const int stride = q_max (Tasks_NumWorkers (), 1) * PARTICLE_UPDATE_CHUNK_SIZE;
	for (int start = index * PARTICLE_UPDATE_CHUNK_SIZE; start < num_particle_updates; start += stride)
	{
		const int end = q_min (start + PARTICLE_UPDATE_CHUNK_SIZE, num_particle_updates);
		for (int i = start; i < end; i++)
		{
			particle_t				   *p = particle_updates[i].p;
			part_type_t				   *type = particle_updates[i].type;
			const particle_emit_meta_t *meta = &type_emit_meta[type - part_type];
			if (!meta->emit_core)
				continue;
			const int		   local = i - meta->start;
			scenetris_t		  *stri = &cl_stris[meta->first_stri + (local / meta->ppb)];
			const int		   batch_local = local % meta->ppb;
			const unsigned int vertofs = stri->firstvert + batch_local * meta->vpp;
			const unsigned int idxofs = stri->firstidx + batch_local * meta->ipp;
			if (p->die < particletime)
			{
				// died during the update: fill the reserved slot with degenerate primitives
				memset (&cl_curstrisvert[vertofs], 0, meta->vpp * sizeof (basicvertex_t));
				for (int k = 0; k < meta->ipp; k++)
					cl_curstrisidx[idxofs + k] = vertofs - stri->firstvert;
				continue;
			}
			meta->emit_core (stri, p, type->slooks, vertofs, idxofs);
		}
	}
}

void PScript_DrawParticles (cb_context_t *blend_cbx, cb_context_t *wboit_cbx)
{
	if (!r_particles.value)
		return;

	// simulated and emitted by the PScript_*ParticlesTask graph nodes, this only records the draws
	PScript_DrawParticleBatches (blend_cbx, false, wboit_cbx != NULL);
	PScript_DrawParticleBatches (wboit_cbx, true, true);
}

/*
===============
R_DrawParticles_ShowTris
===============
*/
void PScript_DrawParticles_ShowTris (cb_context_t *cbx)
{
	if (r_showtris.value == 1)
		R_BindPipeline (cbx, VK_PIPELINE_BIND_POINT_GRAPHICS, vulkan_globals.showtris_pipeline[R_MainPassPipelineVariant (cbx->render_pass_index)]);
	else
		R_BindPipeline (cbx, VK_PIPELINE_BIND_POINT_GRAPHICS, vulkan_globals.showtris_depth_test_pipeline[R_MainPassPipelineVariant (cbx->render_pass_index)]);

	for (unsigned int i = 0; i < cl_numstris; i++)
	{
		scenetris_t		  *tris = &cl_stris[i];
		const int		   num_indices = tris->numidx;
		const VkDeviceSize vertex_buffer_offset = 0;
		vulkan_globals.vk_cmd_bind_index_buffer (cbx->cb, index_buffers[current_buffer_index], 0, VK_INDEX_TYPE_UINT16);
		vulkan_globals.vk_cmd_bind_vertex_buffers (cbx->cb, 0, 1, &vertex_buffers[current_buffer_index], &vertex_buffer_offset);
		vulkan_globals.vk_cmd_draw_indexed (cbx->cb, num_indices, 1, tris->firstidx, tris->firstvert, 0);
	}
}

/* ---------------------------------------------------------------------------
 * ADR-009 rule 3: no C longjmp may unwind a Rust frame. Each of the four
 * callees below can itself re-raise, so the Rust core reaches them through a
 * Host_Guard and the status comes back as a return value.
 */

static void FtePart_InvokeRegisterVariable (void *p)
{
	Cvar_RegisterVariable ((cvar_t *)p);
}

int FtePart_Glue_RegisterVariable (cvar_t *var)
{
	return Host_Guard (FtePart_InvokeRegisterVariable, var);
}

static void FtePart_InvokeClearTrailStates (void *unused)
{
	(void)unused;
	CL_ClearTrailStates ();
}

int FtePart_Glue_ClearTrailStates (void)
{
	return Host_Guard (FtePart_InvokeClearTrailStates, NULL);
}

static void FtePart_InvokeRegisterParticles (void *unused)
{
	(void)unused;
	CL_RegisterParticles ();
}

int FtePart_Glue_RegisterParticles (void)
{
	return Host_Guard (FtePart_InvokeRegisterParticles, NULL);
}

typedef struct
{
	int		  num;
	entity_t *ent;
} fteparticle_entnum_t;

static void FtePart_InvokeEntityNum (void *p)
{
	fteparticle_entnum_t *a = (fteparticle_entnum_t *)p;
	a->ent = CL_EntityNum (a->num);
}

int FtePart_Glue_EntityNum (int num, void **out)
{
	fteparticle_entnum_t args = {num, NULL};
	int					 raised = Host_Guard (FtePart_InvokeEntityNum, &args);
	if (!raised)
		*out = args.ent;
	return raised;
}

/* ---------------------------------------------------------------------------
 * C -> Rust and Rust -> C shims for the half that stays C. None of these can
 * raise, so they are plain forwards.
 */

void FtePart_Glue_LoadTexture (part_type_t *ptype, qboolean warn)
{
	P_LoadTexture (ptype, warn);
}

void FtePart_Glue_ClipDecal (
	void *model, float *center, float *normal, float *tangent1, float *tangent2, float size, unsigned int surfflagmask, unsigned int surfflagmatch, void *ctx)
{
#ifdef USE_DECALS
	Mod_ClipDecal ((qmodel_t *)model, center, normal, tangent1, tangent2, size, surfflagmask, surfflagmatch, PScript_AddDecals, ctx);
#else
	(void)model, (void)center, (void)normal, (void)tangent1, (void)tangent2, (void)size, (void)surfflagmask, (void)surfflagmatch, (void)ctx;
#endif
}

void *FtePart_Glue_ModKnown (int i)
{
	extern qmodel_t mod_known[];
	return &mod_known[i];
}

unsigned int FtePart_Glue_AtomicIncrementU32 (void *atomic)
{
	return Atomic_IncrementUInt32 ((atomic_uint32_t *)atomic);
}

void FtePart_Glue_AtomicStoreU32 (void *atomic, unsigned int desired)
{
	Atomic_StoreUInt32 ((atomic_uint32_t *)atomic, desired);
}

/* The three console commands and the cvar callback keep a stable C function
 * pointer, so a raise out of a handler unwinds through this frame rather than
 * through the Rust one that Cmd_AddCommand/Cvar_SetCallback was called from
 * (ADR-009). */

void FtePart_Glue_PartRedirect_f (void)
{
	quake_rs_ftepart_part_redirect_f ();
}

void FtePart_Glue_PartInfo_f (void)
{
	quake_rs_ftepart_part_info_f ();
}

void FtePart_Glue_BeamInfo_f (void)
{
	quake_rs_ftepart_beam_info_f ();
}

void FtePart_Glue_ParticleDesc_Callback (cvar_t *var)
{
	Host_Reraise (quake_rs_ftepart_particle_desc_callback (var));
}

/* ---------------------------------------------------------------------------
 * glquake.h:109-131 -- the public entry points. Each turns the Rust status
 * code back into a Host_Error here, on the C side of the frame.
 * PScript_LayoutParticlesTask, PScript_EmitParticlesTask, PScript_DrawParticles
 * and PScript_DrawParticles_ShowTris are defined by the C half above.
 */

void PScript_InitParticles (void)
{
	Host_Reraise (quake_rs_ftepart_init_particles ());
}

void PScript_Shutdown (void)
{
	Host_Reraise (quake_rs_ftepart_shutdown ());
}

void PScript_ClearSurfaceParticles (qmodel_t *mod)
{
	quake_rs_ftepart_clear_surface_particles (mod);
}

void PScript_ClearParticles (qboolean load)
{
	Host_Reraise (quake_rs_ftepart_clear_particles (load));
}

void PScript_UpdateModelEffects (qmodel_t *mod)
{
	quake_rs_ftepart_update_model_effects (mod);
}

int PScript_FindParticleType (const char *fullname)
{
	return quake_rs_ftepart_find_particle_type (fullname);
}

void PScript_EmitSkyEffectTris (qmodel_t *mod, msurface_t *fa, int ptype)
{
	quake_rs_ftepart_emit_sky_effect_tris (mod, fa, ptype);
}

void PScript_DelinkTrailstate (trailstate_t **tsk)
{
	quake_rs_ftepart_delink_trailstate ((void **)tsk);
}

int PScript_RunParticleEffectState (vec3_t org, vec3_t dir, float count, int typenum, trailstate_t **tsk)
{
	int out = 0;
	Host_Reraise (quake_rs_ftepart_run_particle_effect_state (org, dir, count, typenum, (void **)tsk, &out));
	return out;
}

int PScript_ParticleTrail (vec3_t startpos, vec3_t end, int type, float timeinterval, int dlkey, vec3_t axis[3], trailstate_t **tsk)
{
	return quake_rs_ftepart_particle_trail (startpos, end, type, timeinterval, dlkey, (const float *)axis, (void **)tsk);
}

int PScript_RunParticleEffectTypeString (vec3_t org, vec3_t dir, float count, const char *name)
{
	int out = 0;
	Host_Reraise (quake_rs_ftepart_run_particle_effect_type_string (org, dir, count, name, &out));
	return out;
}

int PScript_EntParticleTrail (vec3_t oldorg, entity_t *ent, const char *name)
{
	int out = 0;
	Host_Reraise (quake_rs_ftepart_ent_particle_trail (oldorg, ent, name, &out));
	return out;
}

int PScript_RunParticleEffect (vec3_t org, vec3_t dir, int color, int count)
{
	int out = 0;
	Host_Reraise (quake_rs_ftepart_run_particle_effect (org, dir, color, count, &out));
	return out;
}

void PScript_RunParticleWeather (vec3_t minb, vec3_t maxb, vec3_t dir, float count, int colour, const char *efname)
{
	Host_Reraise (quake_rs_ftepart_run_particle_weather (minb, maxb, dir, count, colour, efname));
}

void PScript_FlushDlightsTask (void *unused)
{
	(void)unused;
	quake_rs_ftepart_flush_dlights_task ();
}

void PScript_UpdateParticlesSetupTask (void *unused)
{
	(void)unused;
	Host_Reraise (quake_rs_ftepart_update_particles_setup_task ());
}

void PScript_UpdateParticlesTask (int index, void *unused)
{
	(void)unused;
	quake_rs_ftepart_update_particles_task (index);
}

float CL_TraceLine (vec3_t start, vec3_t end, vec3_t impact, vec3_t normal, int *entnum)
{
	return quake_rs_ftepart_trace_line (start, end, impact, normal, entnum);
}

#endif /* PSET_SCRIPT */

#endif /* USE_RUST_HOST */
