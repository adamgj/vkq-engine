/* Phase 7 M10f-2 (T10.5) oracle TU for Quake/r_part_fte.c -- the FTE particle
 * script system's simulation half.
 *
 * WHY THIS FILE COMPOSES r_part_fte.c INSTEAD OF build.rs LISTING IT
 *
 * Same reason stubs/r_part_ref.c composes r_part.c, and then some. The
 * prelude's c_ref_* renames are translation-unit-wide by construction, and
 * r_part_fte.c's public names are already taken in this link by doubles that
 * existing differentials assert on:
 *
 *   stubs/stubs.c:7605-7714 aborts on PScript_Shutdown,
 *   PScript_ClearParticles, PScript_DelinkTrailstate,
 *   PScript_FindParticleType, PScript_RunParticleWeather and
 *   PScript_EntParticleTrail, and records PScript_ParticleTrail /
 *   PScript_RunParticleEffectState (:7643, :7656) -- cl_main_differential and
 *   cl_parse_differential compare exactly those recorders.
 *   stubs/stubs.c:2657 records PScript_UpdateModelEffects.
 *   stubs/stubs.c:7951 aborts on CL_TraceLine, which stubs/cl_tent_ref.c:447
 *   relies on for CL_UpdateBeam.
 *   stubs/host_ref.c:350 aborts on PScript_InitParticles and
 *   stubs/host_glue_ref.c:308 wraps it in HOST_GUARD_VOID.
 *   stubs/pf_cl_ref.c:281 and :346 record PScript_RunParticleEffectTypeString
 *   and PScript_RunParticleEffect.
 *
 * So this file must not define a single plain PScript_*, CL_TraceLine,
 * CL_PointContentsMask, VectorVectors or VectorNormalize2 name. The C oracle
 * is reached through c_ref_fte_* and the port through its
 * quake_rs_ftepart_* cores, which is exactly the seam Quake/r_part_fte_glue.c
 * sits on in the engine build.
 *
 * WHY THE PREFIX IS c_ref_fte_ AND NOT c_ref_
 *
 * stubs/r_part_ref.c already defines c_ref_free_particles, c_ref_particles
 * and c_ref_r_numparticles for the classic system (r_part.c:38-42). A bare
 * c_ref_ prefix on r_part_fte.c's free_particles / particles would be a
 * duplicate symbol, so every rename below carries the fte_ infix.
 *
 * THREE NAMES ARE DELIBERATELY NOT RENAMED
 *
 * particles, beams and decals are file-scope statics in r_part_fte.c AND
 * member spellings of part_type_t (:641-644). A #define would rewrite the
 * struct members too, so the pools keep their plain spellings; nothing else
 * in this link defines them, and they are r_part_fte.c-internal either way.
 * (stubs/r_part_ref.c's `particles` is renamed inside its own TU, so the two
 * never meet.)
 *
 * WHAT IS SHARED AND WHAT IS PER SIDE
 *
 * Per side: everything r_part_fte.c defines (the four pools, the free lists,
 * the type array, the run list, the sin/cos tables, the trace-line cache),
 * cl / cls / cl_dlights, CL_AllocDlight's table, CL_EntityNum, the cvar
 * registry and the filesystem -- the last two through the pr_ext_ref.c:675
 * pattern, one searchpath object per side.
 *
 * Shared, and therefore reset by the test between sides: COM_Rand
 * (stubs.c:233-262), the Con_Printf capture log (stubs/console_ref.c:483),
 * Mem_Alloc, the command registry (Cmd_AddCommand is NOT prelude-renamed, so
 * the second side to run PScript_InitParticles gets cmd.c's "already defined"
 * warning -- the drivers below therefore do not offer the init console log as
 * comparable data), and the TexMgr_LoadImage recorder (stubs.c:2224).
 *
 * TWO SEAMS ARE COUNTED RATHER THAN EXECUTED
 *
 * r_part_fte.c:3315 and :3532 call CL_ClearTrailStates, and :7292's type
 * update calls CL_RegisterParticles. cl_main.c:76's body calls
 * PScript_DelinkTrailstate for MAX_BEAMS beams unconditionally, and
 * cl_parse.c's CL_RegisterParticles calls PScript_FindParticleType -- both of
 * which are Sys_Error abort doubles here (stubs.c:7616, :7622) that other
 * differentials depend on. Executing either seam would abort both sides, so
 * both are intercepted symmetrically: the C oracle through the rename block
 * below, the port through FtePart_Glue_ClearTrailStates /
 * FtePart_Glue_RegisterParticles, and each is a per-side counter. Nothing in
 * this fixture seeds a trailstate into cl.entities, so the skipped work is
 * empty on both sides; that it is skipped at all is a Phase 8 gap, recorded
 * here rather than hidden.
 *
 * P_LoadTexture IS SHARED ON PURPOSE
 *
 * r_part_fte.c:1146 is a pure function of the part_type_t handed to it plus
 * the texture manager, and the engine build reaches it from Rust through
 * FtePart_Glue_LoadTexture (r_part_fte_glue.c). stubs.c:2224's
 * TexMgr_LoadImage records and returns NULL, so both sides end with
 * looks.texture == NULL and identical s1/t1/s2/t2/randsmax. The glue mirror
 * below therefore forwards to c_ref_fte_P_LoadTexture with the port's own
 * ptype -- the same body on both sides, which is what makes looks comparable
 * at all.
 *
 * NOT OBSERVABLE HERE. r_part_fte.c's rendering half (the decal clipper
 * :3928-4307, the Vulkan buffers :5480-5700, the draw/emit tail :5900-6290,
 * :7226-7396) is compiled -- a #include cannot take half a file -- but never
 * runs: nothing calls PScript_DrawParticles*, PScript_LayoutParticlesTask or
 * PScript_EmitParticlesTask, and the Vulkan surface it names is declared
 * COMPILE-ONLY in the prelude's "Phase 7 M10f-2" block and given aborting
 * doubles below. That is a Phase 8 gap, not a Phase 7 one.
 *
 * COST, stated so it is not discovered later:
 * scripts/harness/check_ctest_symbols.sh reads C_SOURCES out of build.rs, so
 * it does not inspect this object; build.rs has to watch Quake/r_part_fte.c
 * explicitly. A missed rename below shows up only as a duplicate-symbol link
 * error, so the block is kept in step with r_part_fte.c by hand.
 */

#include "quakedef.h"

/* ---- r_part_fte.c rename block ------------------------------------------
 * Every file-scope symbol Quake/r_part_fte.c defines, derived from
 * `llvm-nm --defined-only` on the composed object, minus particles / beams /
 * decals (see the header).
 */

/* functions (r_part_fte.c:200-7396) */
#define CL_PointContentsMask                 c_ref_fte_CL_PointContentsMask
#define CL_PrepareTraceLineEntities          c_ref_fte_CL_PrepareTraceLineEntities
#define CL_TraceLine                         c_ref_fte_CL_TraceLine
#define CheckAssosiation                     c_ref_fte_CheckAssosiation
#define FinishParticleType                   c_ref_fte_FinishParticleType
#define Fragment_ClipPoly                    c_ref_fte_Fragment_ClipPoly
#define Fragment_ClipPolyToPlane             c_ref_fte_Fragment_ClipPolyToPlane
#define Mod_ClipDecal                        c_ref_fte_Mod_ClipDecal
#define PScript_AddDecals                    c_ref_fte_PScript_AddDecals
#define PScript_AssociateEffect_f            c_ref_fte_PScript_AssociateEffect_f
#define PScript_ClearAllSurfaceParticles     c_ref_fte_PScript_ClearAllSurfaceParticles
#define PScript_ClearParticles               c_ref_fte_PScript_ClearParticles
#define PScript_ClearSurfaceParticles        c_ref_fte_PScript_ClearSurfaceParticles
#define PScript_DelinkTrailstate             c_ref_fte_PScript_DelinkTrailstate
#define PScript_DrawParticleBatches          c_ref_fte_PScript_DrawParticleBatches
#define PScript_DrawParticles                c_ref_fte_PScript_DrawParticles
#define PScript_DrawParticles_ShowTris       c_ref_fte_PScript_DrawParticles_ShowTris
#define PScript_EffectSpawned                c_ref_fte_PScript_EffectSpawned
#define PScript_EmitParticlesTask            c_ref_fte_PScript_EmitParticlesTask
#define PScript_EmitSkyEffectTris            c_ref_fte_PScript_EmitSkyEffectTris
#define PScript_EntParticleTrail             c_ref_fte_PScript_EntParticleTrail
#define PScript_FindParticleType             c_ref_fte_PScript_FindParticleType
#define PScript_FlushDlightsTask             c_ref_fte_PScript_FlushDlightsTask
#define PScript_InitParticles                c_ref_fte_PScript_InitParticles
#define PScript_LayoutParticlesTask          c_ref_fte_PScript_LayoutParticlesTask
#define PScript_LooksUseWBOIT                c_ref_fte_PScript_LooksUseWBOIT
#define PScript_ParseParticleEffectFile      c_ref_fte_PScript_ParseParticleEffectFile
#define PScript_ParticleTrail                c_ref_fte_PScript_ParticleTrail
#define PScript_ParticleTrailSpawn           c_ref_fte_PScript_ParticleTrailSpawn
#define PScript_QueueDecal                   c_ref_fte_PScript_QueueDecal
#define PScript_QueueDlight                  c_ref_fte_PScript_QueueDlight
#define PScript_QueueEffect                  c_ref_fte_PScript_QueueEffect
#define PScript_QueueTrail                   c_ref_fte_PScript_QueueTrail
#define PScript_ReadLine                     c_ref_fte_PScript_ReadLine
#define PScript_RecalculateSkyTris           c_ref_fte_PScript_RecalculateSkyTris
#define PScript_RetintEffect                 c_ref_fte_PScript_RetintEffect
#define PScript_RunParticleEffect            c_ref_fte_PScript_RunParticleEffect
#define PScript_RunParticleEffectState       c_ref_fte_PScript_RunParticleEffectState
#define PScript_RunParticleEffectTypeString  c_ref_fte_PScript_RunParticleEffectTypeString
#define PScript_RunParticleWeather           c_ref_fte_PScript_RunParticleWeather
#define PScript_Shutdown                     c_ref_fte_PScript_Shutdown
#define PScript_Startup                      c_ref_fte_PScript_Startup
#define PScript_UpdateModelEffects           c_ref_fte_PScript_UpdateModelEffects
#define PScript_UpdateParticle               c_ref_fte_PScript_UpdateParticle
#define PScript_UpdateParticleTypes          c_ref_fte_PScript_UpdateParticleTypes
#define PScript_UpdateParticlesSetupTask     c_ref_fte_PScript_UpdateParticlesSetupTask
#define PScript_UpdateParticlesTask          c_ref_fte_PScript_UpdateParticlesTask
#define P_AddRainParticles                   c_ref_fte_P_AddRainParticles
#define P_AllocateParticleType               c_ref_fte_P_AllocateParticleType
#define P_BeamInfo_f                         c_ref_fte_P_BeamInfo_f
#define P_CleanTrailstate                    c_ref_fte_P_CleanTrailstate
#define P_GetParticleType                    c_ref_fte_P_GetParticleType
#define P_LoadParticleSet                    c_ref_fte_P_LoadParticleSet
#define P_LoadTexture                        c_ref_fte_P_LoadTexture
#define P_NewTrailstate                      c_ref_fte_P_NewTrailstate
#define P_PartInfo_f                         c_ref_fte_P_PartInfo_f
#define P_PartRedirect_f                     c_ref_fte_P_PartRedirect_f
#define P_ResetToDefaults                    c_ref_fte_P_ResetToDefaults
#define P_UpdateRand                         c_ref_fte_P_UpdateRand
#define Q1BSP_ClipDecalToNodes               c_ref_fte_Q1BSP_ClipDecalToNodes
#define Q1BSP_Fragment_Surface               c_ref_fte_Q1BSP_Fragment_Surface
#define Q1BSP_RecursiveHullCheck             c_ref_fte_Q1BSP_RecursiveHullCheck
#define R_AddClippedDecal                    c_ref_fte_R_AddClippedDecal
#define R_AddFanSparkParticle                c_ref_fte_R_AddFanSparkParticle
#define R_AddLineSparkParticle               c_ref_fte_R_AddLineSparkParticle
#define R_AddTSparkParticle                  c_ref_fte_R_AddTSparkParticle
#define R_AddTexturedParticle                c_ref_fte_R_AddTexturedParticle
#define R_AddUnclippedDecal                  c_ref_fte_R_AddUnclippedDecal
#define R_DrawParticleBeam                   c_ref_fte_R_DrawParticleBeam
#define R_EmitFanSparkParticle               c_ref_fte_R_EmitFanSparkParticle
#define R_EmitLineSparkParticle              c_ref_fte_R_EmitLineSparkParticle
#define R_EmitTSparkParticle                 c_ref_fte_R_EmitTSparkParticle
#define R_EmitTexturedParticle               c_ref_fte_R_EmitTexturedParticle
#define R_EmitUnclippedDecal                 c_ref_fte_R_EmitUnclippedDecal
#define R_Part_SkyTri                        c_ref_fte_R_Part_SkyTri
#define R_ParticleDesc_Callback              c_ref_fte_R_ParticleDesc_Callback
#define R_Particles_KillAllEffects           c_ref_fte_R_Particles_KillAllEffects
#define ReallocateIndexBuffer                c_ref_fte_ReallocateIndexBuffer
#define ReallocateVertexBuffer               c_ref_fte_ReallocateVertexBuffer
#define VectorNormalize2                     c_ref_fte_VectorNormalize2
#define VectorVectors                        c_ref_fte_VectorVectors
#define buildsintable                        c_ref_fte_buildsintable

/* file-scope data (r_part_fte.c:160-6600) */
#define associatedeffect                c_ref_fte_associatedeffect
#define avelocities                     c_ref_fte_avelocities
#define cl_curstrisidx                  c_ref_fte_cl_curstrisidx
#define cl_curstrisvert                 c_ref_fte_cl_curstrisvert
#define cl_maxstris                     c_ref_fte_cl_maxstris
#define cl_maxstrisidx                  c_ref_fte_cl_maxstrisidx
#define cl_maxstrisvert                 c_ref_fte_cl_maxstrisvert
#define cl_numstris                     c_ref_fte_cl_numstris
#define cl_numstrisidx                  c_ref_fte_cl_numstrisidx
#define cl_numstrisvert                 c_ref_fte_cl_numstrisvert
#define cl_stris                        c_ref_fte_cl_stris
#define cl_strisidx                     c_ref_fte_cl_strisidx
#define cl_strisvert                    c_ref_fte_cl_strisvert
#define current_buffer_index            c_ref_fte_current_buffer_index
#define deferred_queues                 c_ref_fte_deferred_queues
#define free_beams                      c_ref_fte_free_beams
#define free_decals                     c_ref_fte_free_decals
#define free_particles                  c_ref_fte_free_particles
#define index_buffers                   c_ref_fte_index_buffers
#define index_buffers_memory            c_ref_fte_index_buffers_memory
#define legacynames                     c_ref_fte_legacynames
#define loadedconfigs                   c_ref_fte_loadedconfigs
#define max_particle_updates            c_ref_fte_max_particle_updates
#define max_trace_line_ents             c_ref_fte_max_trace_line_ents
#define num_particle_updates            c_ref_fte_num_particle_updates
#define num_trace_line_ents             c_ref_fte_num_trace_line_ents
#define num_type_emit_meta              c_ref_fte_num_type_emit_meta
#define numparticletypes                c_ref_fte_numparticletypes
#define p_doflurry                      c_ref_fte_p_doflurry
#define p_frametime                     c_ref_fte_p_frametime
#define p_kill_first                    c_ref_fte_p_kill_first
#define p_kill_list                     c_ref_fte_p_kill_list
#define part_run_list                   c_ref_fte_part_run_list
#define part_type                       c_ref_fte_part_type
#define partaliaslist                   c_ref_fte_partaliaslist
#define particle_trace_limit            c_ref_fte_particle_trace_limit
#define particle_traces_used            c_ref_fte_particle_traces_used
#define particle_update_seed            c_ref_fte_particle_update_seed
#define particle_updates                c_ref_fte_particle_updates
#define particletime                    c_ref_fte_particletime
#define pcostable                       c_ref_fte_pcostable
#define pe_default                      c_ref_fte_pe_default
#define pe_defaulttrail                 c_ref_fte_pe_defaulttrail
#define pe_size2                        c_ref_fte_pe_size2
#define pe_size3                        c_ref_fte_pe_size3
#define pright                          c_ref_fte_pright
#define psintable                       c_ref_fte_psintable
#define pup                             c_ref_fte_pup
#define r_bouncysparks                  c_ref_fte_r_bouncysparks
#define r_decal_noperpendicular         c_ref_fte_r_decal_noperpendicular
#define r_decalrecycle                  c_ref_fte_r_decalrecycle
#define r_fteparticles                  c_ref_fte_r_fteparticles
#define r_lightflicker                  c_ref_fte_r_lightflicker
#define r_numbeams                      c_ref_fte_r_numbeams
#define r_numdecals                     c_ref_fte_r_numdecals
#define r_numparticles                  c_ref_fte_r_numparticles
#define r_numtrailstates                c_ref_fte_r_numtrailstates
#define r_part_beams                    c_ref_fte_r_part_beams
#define r_part_contentswitch            c_ref_fte_r_part_contentswitch
#define r_part_density                  c_ref_fte_r_part_density
#define r_part_maxdecals                c_ref_fte_r_part_maxdecals
#define r_part_maxparticles             c_ref_fte_r_part_maxparticles
#define r_part_rain                     c_ref_fte_r_part_rain
#define r_part_rain_quantity            c_ref_fte_r_part_rain_quantity
#define r_part_sparks                   c_ref_fte_r_part_sparks
#define r_part_sparks_textured          c_ref_fte_r_part_sparks_textured
#define r_part_sparks_trifan            c_ref_fte_r_part_sparks_trifan
#define r_particle_tracelimit           c_ref_fte_r_particle_tracelimit
#define r_particledesc                  c_ref_fte_r_particledesc
#define r_particlerecycle               c_ref_fte_r_particlerecycle
#define r_plooksdirty                   c_ref_fte_r_plooksdirty
#define r_trace_line_cache_counter      c_ref_fte_r_trace_line_cache_counter
#define trace_line_bounds               c_ref_fte_trace_line_bounds
#define trace_line_cache_valid_count    c_ref_fte_trace_line_cache_valid_count
#define trace_line_ents                 c_ref_fte_trace_line_ents
#define trace_line_prepared_framecount  c_ref_fte_trace_line_prepared_framecount
#define trailstates                     c_ref_fte_trailstates
#define ts_cycle                        c_ref_fte_ts_cycle
#define type_emit_meta                  c_ref_fte_type_emit_meta
#define vertex_buffers                  c_ref_fte_vertex_buffers
#define vertex_buffers_memory           c_ref_fte_vertex_buffers_memory

/* ---- the two intercepted seams ------------------------------------------
 * See "TWO SEAMS ARE COUNTED RATHER THAN EXECUTED" in the header. The prelude
 * already renamed both to c_ref_*; that spelling is cl_main.c's / cl_parse.c's
 * real body, which aborts here, so the rename is redirected at this TU's own
 * recorders instead.
 */
#undef CL_ClearTrailStates
#undef CL_RegisterParticles
#define CL_ClearTrailStates  c_ref_fte_CL_ClearTrailStates
#define CL_RegisterParticles c_ref_fte_CL_RegisterParticles

void c_ref_fte_CL_ClearTrailStates (void);
void c_ref_fte_CL_RegisterParticles (void);

/* The prelude declares the plain spellings of r_part_fte.c's external
 * entry points (its glquake.h client slice), so after the renames above the
 * definitions would have no visible prototype. Re-declaring them here costs
 * nothing -- the macros rewrite each line -- and keeps the oracle build
 * warning-clean. */
void	  PScript_InitParticles (void);
void	  PScript_Shutdown (void);
void	  PScript_ClearSurfaceParticles (qmodel_t *mod);
void	  PScript_ClearParticles (qboolean load);
void	  PScript_UpdateModelEffects (qmodel_t *mod);
int		  PScript_FindParticleType (const char *fullname);
void	  PScript_DelinkTrailstate (struct trailstate_s **tsk);
int		  PScript_RunParticleEffectState (vec3_t org, vec3_t dir, float count, int typenum, struct trailstate_s **tsk);
int		  PScript_ParticleTrail (vec3_t startpos, vec3_t end, int type, float timeinterval, int dlkey, vec3_t axis[3], struct trailstate_s **tsk);
int		  PScript_RunParticleEffectTypeString (vec3_t org, vec3_t dir, float count, const char *name);
int		  PScript_EntParticleTrail (vec3_t oldorg, entity_t *ent, const char *name);
int		  PScript_RunParticleEffect (vec3_t org, vec3_t dir, int color, int count);
void	  PScript_RunParticleWeather (vec3_t minb, vec3_t maxb, vec3_t dir, float count, int colour, const char *efname);
void	  PScript_EmitSkyEffectTris (qmodel_t *mod, msurface_t *fa, int ptype);
void	  PScript_RecalculateSkyTris (void);
qboolean  PScript_Startup (void);
void	  PScript_ParseParticleEffectFile (const char *config, qboolean part_parseweak, char *context, size_t filesize);
char	 *PScript_ReadLine (char *buffer, size_t buffersize, const char *filedata, size_t filesize, size_t *offset);
void	  PScript_FlushDlightsTask (void *unused);
void	  PScript_UpdateParticlesSetupTask (void *unused);
void	  PScript_UpdateParticlesTask (int index, void *unused);
void	  PScript_LayoutParticlesTask (void *unused);
void	  PScript_EmitParticlesTask (int index, void *unused);
void	  PScript_DrawParticles (cb_context_t *blend_cbx, cb_context_t *wboit_cbx);
void	  PScript_DrawParticles_ShowTris (cb_context_t *cbx);
float	  CL_TraceLine (vec3_t start, vec3_t end, vec3_t impact, vec3_t normal, int *entnum);
static void CL_PrepareTraceLineEntities (void); /* static at r_part_fte.c:574; a non-static
												   declaration makes the rename above illegal */
void	  VectorVectors (const vec3_t forward, vec3_t right, vec3_t up);
vec_t	  VectorNormalize2 (const vec3_t v, vec3_t out);

#include "r_part_fte.c"

/* =========================================================================
 * THE PLAIN HALF -- the ctest-link mirror of Quake/r_part_fte_glue.c
 * ========================================================================= */

#undef CL_PointContentsMask
#undef CL_PrepareTraceLineEntities
#undef CL_TraceLine
#undef CheckAssosiation
#undef FinishParticleType
#undef Fragment_ClipPoly
#undef Fragment_ClipPolyToPlane
#undef Mod_ClipDecal
#undef PScript_AddDecals
#undef PScript_AssociateEffect_f
#undef PScript_ClearAllSurfaceParticles
#undef PScript_ClearParticles
#undef PScript_ClearSurfaceParticles
#undef PScript_DelinkTrailstate
#undef PScript_DrawParticleBatches
#undef PScript_DrawParticles
#undef PScript_DrawParticles_ShowTris
#undef PScript_EffectSpawned
#undef PScript_EmitParticlesTask
#undef PScript_EmitSkyEffectTris
#undef PScript_EntParticleTrail
#undef PScript_FindParticleType
#undef PScript_FlushDlightsTask
#undef PScript_InitParticles
#undef PScript_LayoutParticlesTask
#undef PScript_LooksUseWBOIT
#undef PScript_ParseParticleEffectFile
#undef PScript_ParticleTrail
#undef PScript_ParticleTrailSpawn
#undef PScript_QueueDecal
#undef PScript_QueueDlight
#undef PScript_QueueEffect
#undef PScript_QueueTrail
#undef PScript_ReadLine
#undef PScript_RecalculateSkyTris
#undef PScript_RetintEffect
#undef PScript_RunParticleEffect
#undef PScript_RunParticleEffectState
#undef PScript_RunParticleEffectTypeString
#undef PScript_RunParticleWeather
#undef PScript_Shutdown
#undef PScript_Startup
#undef PScript_UpdateModelEffects
#undef PScript_UpdateParticle
#undef PScript_UpdateParticleTypes
#undef PScript_UpdateParticlesSetupTask
#undef PScript_UpdateParticlesTask
#undef P_AddRainParticles
#undef P_AllocateParticleType
#undef P_BeamInfo_f
#undef P_CleanTrailstate
#undef P_GetParticleType
#undef P_LoadParticleSet
#undef P_LoadTexture
#undef P_NewTrailstate
#undef P_PartInfo_f
#undef P_PartRedirect_f
#undef P_ResetToDefaults
#undef P_UpdateRand
#undef Q1BSP_ClipDecalToNodes
#undef Q1BSP_Fragment_Surface
#undef Q1BSP_RecursiveHullCheck
#undef R_AddClippedDecal
#undef R_AddFanSparkParticle
#undef R_AddLineSparkParticle
#undef R_AddTSparkParticle
#undef R_AddTexturedParticle
#undef R_AddUnclippedDecal
#undef R_DrawParticleBeam
#undef R_EmitFanSparkParticle
#undef R_EmitLineSparkParticle
#undef R_EmitTSparkParticle
#undef R_EmitTexturedParticle
#undef R_EmitUnclippedDecal
#undef R_Part_SkyTri
#undef R_ParticleDesc_Callback
#undef R_Particles_KillAllEffects
#undef ReallocateIndexBuffer
#undef ReallocateVertexBuffer
#undef VectorNormalize2
#undef VectorVectors
#undef associatedeffect
#undef avelocities
#undef buildsintable
#undef cl_curstrisidx
#undef cl_curstrisvert
#undef cl_maxstris
#undef cl_maxstrisidx
#undef cl_maxstrisvert
#undef cl_numstris
#undef cl_numstrisidx
#undef cl_numstrisvert
#undef cl_stris
#undef cl_strisidx
#undef cl_strisvert
#undef current_buffer_index
#undef deferred_queues
#undef free_beams
#undef free_decals
#undef free_particles
#undef index_buffers
#undef index_buffers_memory
#undef legacynames
#undef loadedconfigs
#undef max_particle_updates
#undef max_trace_line_ents
#undef num_particle_updates
#undef num_trace_line_ents
#undef num_type_emit_meta
#undef numparticletypes
#undef p_doflurry
#undef p_frametime
#undef p_kill_first
#undef p_kill_list
#undef part_run_list
#undef part_type
#undef partaliaslist
#undef particle_trace_limit
#undef particle_traces_used
#undef particle_update_seed
#undef particle_updates
#undef particletime
#undef pcostable
#undef pe_default
#undef pe_defaulttrail
#undef pe_size2
#undef pe_size3
#undef pright
#undef psintable
#undef pup
#undef r_bouncysparks
#undef r_decal_noperpendicular
#undef r_decalrecycle
#undef r_fteparticles
#undef r_lightflicker
#undef r_numbeams
#undef r_numdecals
#undef r_numparticles
#undef r_numtrailstates
#undef r_part_beams
#undef r_part_contentswitch
#undef r_part_density
#undef r_part_maxdecals
#undef r_part_maxparticles
#undef r_part_rain
#undef r_part_rain_quantity
#undef r_part_sparks
#undef r_part_sparks_textured
#undef r_part_sparks_trifan
#undef r_particle_tracelimit
#undef r_particledesc
#undef r_particlerecycle
#undef r_plooksdirty
#undef r_trace_line_cache_counter
#undef trace_line_bounds
#undef trace_line_cache_valid_count
#undef trace_line_ents
#undef trace_line_prepared_framecount
#undef trailstates
#undef ts_cycle
#undef type_emit_meta
#undef vertex_buffers
#undef vertex_buffers_memory
#undef CL_ClearTrailStates
#undef CL_RegisterParticles
#undef cl
#undef cls
#undef cl_dlights
#undef Cvar_RegisterVariable
#undef COM_LoadFile
#undef com_gamedir
#undef com_searchpaths
/* r_part_fte.c:186-187 replaced sin/cos with table lookups for the whole file;
 * the fixture below must not inherit that. */
#undef sin
#undef cos

/* ---- storage Quake/r_part_fte_glue.c owns in the engine build (ADR-007) ---
 * rust/quake-c-sys/src/r_part_fte.rs externs exactly these. r_fteparticles
 * and r_particledesc are NOT here: stubs/pr_ext_ref.c:222-223 already defines
 * both plain cvar_t objects and pr_ext_differential asserts on them, so the
 * port reaches those, one plain object per link as in the engine build.
 */

float psintable[SINTABLE_ENTRIES];
float pcostable[SINTABLE_ENTRIES];

/* r_part_fte.c:459. Renamed on the way out for the same reason
 * r_part_fte_glue.c:456 renames it: r_part.c's classic free_particles is
 * external linkage and the two were only ever distinct because this one was
 * static. */
particle_t	   *fte_free_particles;
beamseg_t	   *free_beams;
clippeddecal_t *free_decals;

float particletime;

int			 numparticletypes;
part_type_t *part_type;
part_type_t *part_run_list;

vec3_t pright, pup;

deferred_queues_t  deferred_queues[TASKS_MAX_WORKERS];
particle_update_t *particle_updates;
int				   num_particle_updates, max_particle_updates;
atomic_uint32_t	   particle_traces_used;

particle_emit_meta_t *type_emit_meta;
int					  num_type_emit_meta;

float		p_frametime;
particle_t *p_kill_list, *p_kill_first;

/* r_part_fte.c:482-496, initializers verbatim. cvar_t is a C ABI object the
 * registry links into a hash chain and mutates thereafter, so the storage
 * stays C exactly as r_part_fte_glue.c:479-493 keeps it. */
cvar_t r_bouncysparks = {"r_bouncysparks", "1"};
cvar_t r_part_rain = {"r_part_rain", "1"};
cvar_t r_decal_noperpendicular = {"r_decal_noperpendicular", "1"};
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

extern cvar_t r_fteparticles;  /* stubs/pr_ext_ref.c:222 */
extern cvar_t r_particledesc;  /* stubs/pr_ext_ref.c:223 */
extern cvar_t r_particles;	   /* stubs/menu_ref.c:726 */
extern cvar_t r_showtris;	   /* stubs/r_part_ref.c's plain half */
extern int	  r_trace_line_cache_counter; /* stubs/stubs.c:7439 */

extern client_state_t	cl;	 /* quake-capi/src/cl_main.rs owns it */
extern client_static_t	cls; /* likewise */
extern dlight_t		   *cl_dlights;
extern char				com_gamedir[MAX_OSPATH];
extern searchpath_t	   *com_searchpaths;
extern byte			   *COM_LoadFile (const char *path, unsigned int *path_id);
extern void				Cvar_RegisterVariable (cvar_t *variable);
extern entity_t		   *CL_EntityNum (int num);

extern int	Host_Guard (void (*fn) (void *), void *arg);
extern void Host_Reraise (int guard_result);

/* stubs.c:7228 sizes this at 4 and leaves mod_numknown 0; both sides read the
 * one object, which is correct -- model state is input to this module, never
 * output. */
extern qmodel_t mod_known[];

/* ---------------------------------------------------------------------------
 * Link doubles the composed r_part_fte.c needs. None of these is defined
 * anywhere else in this link (checked by grep over stubs/ and quake-capi), so
 * defining them here cannot displace an existing double.
 */

/* The task graph is not in this link. One worker, index 0, so the deferred
 * queues and the chunked update both take their single-threaded path. */
int Tasks_NumWorkers (void)
{
	return 1;
}

int Tasks_GetWorkerIndex (void)
{
	return 0;
}

/* gl_texmgr.c. stubs.c:2224's TexMgr_LoadImage is the recorder both sides
 * reach; these two only ever have to say "no such texture" for
 * r_part_fte.c:1146 to take its generated-texture path. */
gltexture_t *TexMgr_FindTexture (qmodel_t *owner, const char *name)
{
	(void)owner;
	(void)name;
	return NULL;
}

byte *Image_LoadImage (const char *name, int *width, int *height, enum srcformat *fmt, unsigned int flags)
{
	(void)name;
	(void)flags;
	*width = 0;
	*height = 0;
	*fmt = SRC_RGBA;
	return NULL;
}

/* r_part_fte.c:1258 (classicparticle) and the rendering half's white default.
 * Nothing else in this link defines either; both stay NULL, so both sides see
 * the same value. */
gltexture_t *particletexture1;
gltexture_t *whitetexture;

/* ---------------------------------------------------------------------------
 * The two intercepted seams. Both of these are cl_main.c/cl_parse.c entry
 * points whose stubs.c doubles abort (stubs.c:7616 and :7622 are Sys_Error
 * recorders), so leaving them alone would kill BOTH sides at PScript_Startup.
 * Intercepting them symmetrically -- the oracle through the #define above,
 * the port through FtePart_Glue_* below -- turns each into a per-side call
 * counter. Neither one feeds anything the differential reads back, so counting
 * is all the parity this stratum needs; executing them is a Phase 8 concern.
 */

static int ftepart_cleartrails_calls[2];
static int ftepart_registerparticles_calls[2];
static int ftepart_clipdecal_calls[2];

void c_ref_fte_CL_ClearTrailStates (void)
{
	ftepart_cleartrails_calls[1]++;
}

void c_ref_fte_CL_RegisterParticles (void)
{
	ftepart_registerparticles_calls[1]++;
}

/* ---------------------------------------------------------------------------
 * Quake/r_part_fte_glue.c:2490-2601 mirrored. The four that can raise keep
 * their Host_Guard trampoline (ADR-009 rule 3: no longjmp may cross a Rust
 * frame), because stubs.c:1489's Host_Guard is the real setjmp one.
 */

/* rust/quake-capi/src/r_part_fte.rs -- the port's 24 exports. Spelled here
 * rather than pulled from a generated header for the same reason
 * stubs/chase_ref.c:70 spells its four: the ctest link has no cbindgen step.
 */
extern float		quake_rs_ftepart_trace_line (float *start, float *end, float *impact, float *normal, int *entnum);
extern float		quake_rs_ftepart_vector_normalize2 (const float *v, float *out);
extern int			quake_rs_ftepart_init_particles (void);
extern int			quake_rs_ftepart_shutdown (void);
extern void			quake_rs_ftepart_clear_surface_particles (void *md);
extern int			quake_rs_ftepart_clear_particles (qboolean load);
extern void			quake_rs_ftepart_update_model_effects (void *md);
extern int			quake_rs_ftepart_find_particle_type (const char *fullname);
extern int			quake_rs_ftepart_particle_desc_callback (cvar_t *var);
extern void			quake_rs_ftepart_part_redirect_f (void);
extern void			quake_rs_ftepart_part_info_f (void);
extern void			quake_rs_ftepart_beam_info_f (void);
extern void			quake_rs_ftepart_emit_sky_effect_tris (void *md, void *fa, int ptype);
extern void			quake_rs_ftepart_delink_trailstate (void **tsk);
extern int			quake_rs_ftepart_run_particle_effect_state (const float *org, const float *dir, float count, int typenum, void **tsk, int *out);
extern int			quake_rs_ftepart_particle_trail (const float *startpos, const float *end, int type, float timeinterval, int dlkey, const float *axis, void **tsk);
extern int			quake_rs_ftepart_run_particle_effect_type_string (const float *org, const float *dir, float count, const char *name, int *out);
extern int			quake_rs_ftepart_ent_particle_trail (const float *oldorg, void *ent, const char *name, int *out);
extern int			quake_rs_ftepart_run_particle_effect (const float *org, const float *dir, int color, int count, int *out);
extern int			quake_rs_ftepart_run_particle_weather (const float *minb, const float *maxb, const float *dir, float count, int colour, const char *efname);
extern void			quake_rs_ftepart_queue_effect (const float *org, const float *dir, float count, int type);
extern void			quake_rs_ftepart_flush_dlights_task (void);
extern int			quake_rs_ftepart_update_particles_setup_task (void);
extern void			quake_rs_ftepart_update_particles_task (int index);

static void FtePart_InvokeRegisterVariable (void *p)
{
	Cvar_RegisterVariable ((cvar_t *)p);
}

int FtePart_Glue_RegisterVariable (cvar_t *var)
{
	return Host_Guard (FtePart_InvokeRegisterVariable, var);
}

int FtePart_Glue_ClearTrailStates (void)
{
	ftepart_cleartrails_calls[0]++;
	return 0;
}

int FtePart_Glue_RegisterParticles (void)
{
	ftepart_registerparticles_calls[0]++;
	return 0;
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

/* Deliberately shared: P_LoadTexture is static inside the composed TU, so the
 * port reaches the oracle's own body with the port's own ptype. stubs.c:2224's
 * TexMgr_LoadImage is a recorder returning NULL rather than an abort, so the
 * body runs to completion and leaves looks.texture NULL on both sides, which
 * is what makes ptype->looks (and s1/t1/s2/t2/randsmax) comparable at all.
 * Re-implementing it here would be the asymmetry, not this. */
void FtePart_Glue_LoadTexture (part_type_t *ptype, qboolean warn)
{
	c_ref_fte_P_LoadTexture (ptype, warn);
}

/* Counted, not executed. Unlike P_LoadTexture this one cannot be shared: the
 * oracle's Mod_ClipDecal reaches PScript_AddDecals, which allocates out of
 * c_ref_fte_free_decals, so forwarding would have the port consume the
 * oracle's decal pool. The path is unreachable in this link anyway -- it needs
 * a mounted world model with a Q1BSP node tree -- and ctest_ftepart_read_ints
 * asserts the counter stays 0 on both sides, so a future link that does mount
 * one fails here loudly instead of drifting. */
void FtePart_Glue_ClipDecal (
	void *model, float *center, float *normal, float *tangent1, float *tangent2, float size, unsigned int surfflagmask, unsigned int surfflagmatch, void *ctx)
{
	(void)model, (void)center, (void)normal, (void)tangent1, (void)tangent2, (void)size, (void)surfflagmask, (void)surfflagmatch, (void)ctx;
	ftepart_clipdecal_calls[0]++;
}

void *FtePart_Glue_ModKnown (int i)
{
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

/* ===========================================================================
 * ctest fixture. side 1 drives the C oracle (c_ref_fte_*), side 0 the port
 * (quake_rs_ftepart_*), matching stubs/r_part_ref.c's convention.
 *
 * Everything the oracle touches that can Host_Error goes through Host_Guard
 * (stubs.c:1489), so a raise on either side comes back as a status code and
 * the two are comparable; the port's exports already return one (ADR-009).
 */

/* c_ref_prelude.h:406/:410/:418 rename the filesystem TU-wide and the #undef
 * block above restored the plain spellings for this half, so the two sides
 * have two filesystems and each has to be pointed at its own scratch dir.
 * This is stubs/pr_ext_ref.c:675-731's pattern. */
extern char			 c_ref_com_gamedir[MAX_OSPATH];
extern searchpath_t *c_ref_com_searchpaths;

static searchpath_t	 ftepart_searchpath[2];
static searchpath_t *ftepart_saved_searchpaths[2];
static float		 ftepart_saved_registered[2];

void ctest_ftepart_fs_setup (int side, const char *dir)
{
	char sub[MAX_OSPATH];

	Sys_mkdir (dir);
	q_snprintf (sub, sizeof (sub), "%s/particles", dir);
	Sys_mkdir (sub);

	memset (&ftepart_searchpath[side], 0, sizeof (ftepart_searchpath[side]));
	ftepart_searchpath[side].path_id = 1;
	q_strlcpy (ftepart_searchpath[side].filename, dir, sizeof (ftepart_searchpath[side].filename));

	ftepart_saved_registered[side] = registered.value;
	registered.value = 1;

	if (side)
	{
		q_strlcpy (c_ref_com_gamedir, dir, sizeof (c_ref_com_gamedir));
		ftepart_saved_searchpaths[side] = c_ref_com_searchpaths;
		c_ref_com_searchpaths = &ftepart_searchpath[side];
	}
	else
	{
		q_strlcpy (com_gamedir, dir, sizeof (com_gamedir));
		ftepart_saved_searchpaths[side] = com_searchpaths;
		com_searchpaths = &ftepart_searchpath[side];
	}
}

void ctest_ftepart_fs_teardown (int side)
{
	registered.value = ftepart_saved_registered[side];
	if (side)
	{
		c_ref_com_searchpaths = ftepart_saved_searchpaths[side];
		c_ref_com_gamedir[0] = 0;
	}
	else
	{
		com_searchpaths = ftepart_saved_searchpaths[side];
		com_gamedir[0] = 0;
	}
}

/* cl is renamed TU-wide (c_ref_prelude.h:1975) and quake-capi owns the plain
 * one, so the module's only clock has to be wound on both. */
void ctest_ftepart_set_time (double t)
{
	c_ref_cl.time = t;
	cl.time = t;
}

/* ---- oracle dispatch --------------------------------------------------- */

typedef struct
{
	int			which;
	int			ia, ib;
	float		fa;
	vec3_t		v1, v2, v3;
	const char *name;
	int			iout;
} ftepart_args_t;

static ftepart_args_t ftepart_args;

/* Two trailstate slots so a trail test can hand each side its own head
 * without the pointer values ever being compared. */
static trailstate_t *ftepart_trailstate[2];

static void ftepart_cref_dispatch (void *unused)
{
	ftepart_args_t *a = &ftepart_args;
	(void)unused;
	switch (a->which)
	{
	case 0:
		c_ref_fte_PScript_InitParticles ();
		break;
	case 1:
		c_ref_fte_PScript_Shutdown ();
		break;
	case 2:
		c_ref_fte_PScript_ClearParticles (a->ia ? true : false);
		break;
	case 3:
		a->iout = c_ref_fte_PScript_FindParticleType (a->name);
		break;
	case 4:
		c_ref_fte_R_ParticleDesc_Callback (&c_ref_fte_r_particledesc);
		break;
	case 5:
		a->iout = c_ref_fte_PScript_RunParticleEffect (a->v1, a->v2, a->ia, a->ib);
		break;
	case 6:
		a->iout = c_ref_fte_PScript_RunParticleEffectState (a->v1, a->v2, a->fa, a->ia, &ftepart_trailstate[1]);
		break;
	case 7:
		a->iout = c_ref_fte_PScript_RunParticleEffectTypeString (a->v1, a->v2, a->fa, a->name);
		break;
	case 8:
		a->iout = c_ref_fte_PScript_ParticleTrail (a->v1, a->v2, a->ia, 0, a->ib, NULL, &ftepart_trailstate[1]);
		break;
	case 9:
		c_ref_fte_PScript_RunParticleWeather (a->v1, a->v2, a->v3, a->fa, a->ia, a->name);
		break;
	case 10:
		c_ref_fte_PScript_UpdateParticlesSetupTask (NULL);
		break;
	case 11:
		c_ref_fte_PScript_UpdateParticlesTask (a->ia, NULL);
		break;
	case 12:
		c_ref_fte_PScript_FlushDlightsTask (NULL);
		break;
	case 13:
		c_ref_fte_PScript_DelinkTrailstate (&ftepart_trailstate[1]);
		break;
	case 14:
		c_ref_fte_PScript_QueueEffect (a->v1, a->v2, a->fa, a->ia);
		break;
	case 15:
		c_ref_fte_P_PartInfo_f ();
		break;
	case 16:
		c_ref_fte_P_BeamInfo_f ();
		break;
	case 17:
		c_ref_fte_P_PartRedirect_f ();
		break;
	default:
		Sys_Error ("ftepart_cref_dispatch: bad index %d", a->which);
	}
}

static int ftepart_cref_run (int which)
{
	ftepart_args.which = which;
	ftepart_args.iout = 0;
	return Host_Guard (ftepart_cref_dispatch, NULL);
}

/* ---- side-dispatched drivers ------------------------------------------- */

int ctest_ftepart_init (int side)
{
	if (side)
		return ftepart_cref_run (0);
	return quake_rs_ftepart_init_particles ();
}

int ctest_ftepart_shutdown (int side)
{
	if (side)
		return ftepart_cref_run (1);
	return quake_rs_ftepart_shutdown ();
}

int ctest_ftepart_clear (int side, int load)
{
	if (side)
	{
		ftepart_args.ia = load;
		return ftepart_cref_run (2);
	}
	return quake_rs_ftepart_clear_particles (load ? true : false);
}

int ctest_ftepart_find_type (int side, const char *name, int *out)
{
	if (side)
	{
		int raised;
		ftepart_args.name = name;
		raised = ftepart_cref_run (3);
		*out = ftepart_args.iout;
		return raised;
	}
	*out = quake_rs_ftepart_find_particle_type (name);
	return 0;
}

/* Sets the side's own r_particledesc string, then runs the callback the way
 * Cvar_Set would. The two cvar_t objects are distinct: the oracle's is the
 * renamed c_ref_fte_r_particledesc that r_part_fte.c defines, the port's is
 * stubs/pr_ext_ref.c:223's plain one that quake-capi registers. */
int ctest_ftepart_set_desc (int side, const char *value)
{
	cvar_t *var = side ? &c_ref_fte_r_particledesc : &r_particledesc;
	char   *saved = var->string;
	int		raised;

	var->string = (char *)value;
	if (side)
		raised = ftepart_cref_run (4);
	else
		raised = quake_rs_ftepart_particle_desc_callback (var);
	/* The port's r_particledesc is stubs/pr_ext_ref.c:223's object, shared with
	 * pr_ext_particles_differential; put its registered string back so this gate
	 * cannot leak into that one. */
	var->string = saved;
	return raised;
}

int ctest_ftepart_run_effect (int side, const float *org, const float *dir, int color, int count, int *out)
{
	if (side)
	{
		int raised;
		VectorCopy (org, ftepart_args.v1);
		VectorCopy (dir, ftepart_args.v2);
		ftepart_args.ia = color;
		ftepart_args.ib = count;
		raised = ftepart_cref_run (5);
		*out = ftepart_args.iout;
		return raised;
	}
	return quake_rs_ftepart_run_particle_effect (org, dir, color, count, out);
}

int ctest_ftepart_run_effect_state (int side, const float *org, const float *dir, float count, int type, int *out)
{
	if (side)
	{
		int raised;
		VectorCopy (org, ftepart_args.v1);
		VectorCopy (dir, ftepart_args.v2);
		ftepart_args.fa = count;
		ftepart_args.ia = type;
		raised = ftepart_cref_run (6);
		*out = ftepart_args.iout;
		return raised;
	}
	return quake_rs_ftepart_run_particle_effect_state (org, dir, count, type, (void **)&ftepart_trailstate[0], out);
}

int ctest_ftepart_run_effect_string (int side, const float *org, const float *dir, float count, const char *name, int *out)
{
	if (side)
	{
		int raised;
		VectorCopy (org, ftepart_args.v1);
		VectorCopy (dir, ftepart_args.v2);
		ftepart_args.fa = count;
		ftepart_args.name = name;
		raised = ftepart_cref_run (7);
		*out = ftepart_args.iout;
		return raised;
	}
	return quake_rs_ftepart_run_particle_effect_type_string (org, dir, count, name, out);
}

int ctest_ftepart_trail (int side, const float *start, const float *end, int type, int dlkey, int *out)
{
	if (side)
	{
		int raised;
		VectorCopy (start, ftepart_args.v1);
		VectorCopy (end, ftepart_args.v2);
		ftepart_args.ia = type;
		ftepart_args.ib = dlkey;
		raised = ftepart_cref_run (8);
		*out = ftepart_args.iout;
		return raised;
	}
	*out = quake_rs_ftepart_particle_trail (start, end, type, 0, dlkey, NULL, (void **)&ftepart_trailstate[0]);
	return 0;
}

int ctest_ftepart_weather (int side, const float *minb, const float *maxb, const float *dir, float count, int colour, const char *efname)
{
	if (side)
	{
		VectorCopy (minb, ftepart_args.v1);
		VectorCopy (maxb, ftepart_args.v2);
		VectorCopy (dir, ftepart_args.v3);
		ftepart_args.fa = count;
		ftepart_args.ia = colour;
		ftepart_args.name = efname;
		return ftepart_cref_run (9);
	}
	return quake_rs_ftepart_run_particle_weather (minb, maxb, dir, count, colour, efname);
}

void ctest_ftepart_queue_effect (int side, const float *org, const float *dir, float count, int type)
{
	if (side)
	{
		VectorCopy (org, ftepart_args.v1);
		VectorCopy (dir, ftepart_args.v2);
		ftepart_args.fa = count;
		ftepart_args.ia = type;
		(void)ftepart_cref_run (14);
		return;
	}
	quake_rs_ftepart_queue_effect (org, dir, count, type);
}

int ctest_ftepart_delink_trailstate (int side)
{
	if (side)
		return ftepart_cref_run (13);
	quake_rs_ftepart_delink_trailstate ((void **)&ftepart_trailstate[0]);
	return 0;
}

/* One frame of the update: the serial setup, the single worker's slice, then
 * the dlight flush the next frame's graph would run. Tasks_NumWorkers is 1
 * here, so index 0 covers every chunk. */
int ctest_ftepart_update (int side)
{
	if (side)
	{
		int raised = ftepart_cref_run (10);
		if (raised)
			return raised;
		ftepart_args.ia = 0;
		raised = ftepart_cref_run (11);
		if (raised)
			return raised;
		return ftepart_cref_run (12);
	}
	else
	{
		int raised = quake_rs_ftepart_update_particles_setup_task ();
		if (raised)
			return raised;
		quake_rs_ftepart_update_particles_task (0);
		quake_rs_ftepart_flush_dlights_task ();
		return 0;
	}
}

/* 0 = r_partinfo, 1 = r_beaminfo, 2 = r_partredirect. Only the raise status
 * and the state they leave behind are compared; the console text is not, see
 * the Cmd_AddCommand note in the header. */
int ctest_ftepart_run_cmd (int side, int which)
{
	if (side)
		return ftepart_cref_run (15 + which);
	switch (which)
	{
	case 0:
		quake_rs_ftepart_part_info_f ();
		break;
	case 1:
		quake_rs_ftepart_beam_info_f ();
		break;
	default:
		quake_rs_ftepart_part_redirect_f ();
		break;
	}
	return 0;
}

float ctest_ftepart_normalize2 (int side, const float *v, float *out)
{
	vec3_t in;
	VectorCopy (v, in);
	if (side)
		return c_ref_fte_VectorNormalize2 (in, out);
	return quake_rs_ftepart_vector_normalize2 (in, out);
}

float ctest_ftepart_trace_line (int side, const float *start, const float *end, float *impact, float *normal, int *entnum)
{
	vec3_t s, e;
	VectorCopy (start, s);
	VectorCopy (end, e);
	if (side)
		return c_ref_fte_CL_TraceLine (s, e, impact, normal, entnum);
	return quake_rs_ftepart_trace_line (s, e, impact, normal, entnum);
}

/* ---- state readers ------------------------------------------------------
 *
 * Every reader takes the side and reaches that side's own storage. The C
 * oracle's is the renamed c_ref_fte_* set the composed r_part_fte.c defines;
 * the port's is the plain set defined at the top of this file, which is what
 * rust/quake-c-sys/src/r_part_fte.rs externs.
 */

#define FTEPART_LIST_CAP 262144 /* > r_part_maxparticles, so a cycle cannot hang the test */

static int ftepart_plen (const particle_t *p)
{
	int n = 0;
	while (p && n < FTEPART_LIST_CAP)
	{
		p = p->next;
		n++;
	}
	return n;
}

static int ftepart_blen (const beamseg_t *b)
{
	int n = 0;
	while (b && n < FTEPART_LIST_CAP)
	{
		b = b->next;
		n++;
	}
	return n;
}

static int ftepart_dlen (const clippeddecal_t *d)
{
	int n = 0;
	while (d && n < FTEPART_LIST_CAP)
	{
		d = d->next;
		n++;
	}
	return n;
}

static part_type_t *ftepart_types (int side)
{
	return side ? c_ref_fte_part_type : part_type;
}

static int ftepart_numtypes (int side)
{
	return side ? c_ref_fte_numparticletypes : numparticletypes;
}

static part_type_t *ftepart_runlist (int side)
{
	return side ? c_ref_fte_part_run_list : part_run_list;
}

static int ftepart_index_of (int side, const part_type_t *t)
{
	part_type_t *base = ftepart_types (side);
	if (!t || !base)
		return -1;
	if (t < base || t >= base + ftepart_numtypes (side))
		return -2;
	return (int)(t - base);
}

/* Hashes one part_type_t by value with every pointer member replaced by a
 * side-independent derivation, so the digest describes the parsed effect and
 * not where two allocators happened to put it. Padding is included and is
 * deterministic: r_part_fte.c:960 memsets each entry on creation, so the two
 * sides start from identical bytes. */
static void ftepart_scrub_type (int side, int i, part_type_t *dst)
{
	part_type_t *base = ftepart_types (side);
	part_type_t	 t = base[i];

	/* slooks is reduced to a set/unset flag rather than to the index of the
	 * type whose looks it aliases, because that index is not a property of the
	 * port. r_part_fte.c:6631 assigns slooks as an interior pointer into
	 * part_type, and P_AllocateParticleType (r_part_fte.c:958-975) rebases
	 * part_run_list and every nexttorun after its Mem_Realloc but *not*
	 * slooks -- so once the table grows again, every slooks is left pointing
	 * into the old block. Whether that stale pointer still compares equal to
	 * &base[k].looks then depends only on whether realloc happened to grow in
	 * place, which is a property of that side's heap history and differs
	 * between the two independently-allocated tables in this link. The
	 * dedup decision itself stays observable: looks is hashed by value below,
	 * so two types that share a plooks_t still hash alike. */
	t.slooks = (plooks_t *)(intptr_t)(t.slooks ? 1 : 0);
	t.nexttorun = (part_type_t *)(intptr_t)ftepart_index_of (side, t.nexttorun);

	t.looks.texture = (gltexture_t *)(intptr_t)(t.looks.texture ? 1 : 0);
	t.sounds = (partsounds_t *)(intptr_t)(t.sounds ? 1 : 0);
	t.ramp = (ramp_t *)(intptr_t)(t.ramp ? 1 : 0);
	t.particles = (particle_t *)(intptr_t)ftepart_plen (t.particles);
	t.beams = (beamseg_t *)(intptr_t)ftepart_blen (t.beams);
	t.clippeddecals = (clippeddecal_t *)(intptr_t)ftepart_dlen (t.clippeddecals);

	*dst = t;
}

/* Both trailing arrays are grown with Mem_Realloc, which does not clear the
 * new tail (stubs/stubs.c:178, and Quake/mem.c:120 in the engine build), so
 * they are hashed member by member rather than as raw storage: hashing the
 * padding would be comparing two heaps' garbage. Two fields are deliberately
 * out of the digest for exactly that reason:
 *   - ramp_t::rotation, which nothing in r_part_fte.c ever writes or reads;
 *   - the bytes of partsounds_t::name past its NUL.
 * Everything the parser can actually set is covered. */
static uint64_t ftepart_hash_arrays (uint64_t h, const part_type_t *t)
{
	int i;

	h = Harness_Hash64 (h, &t->numsounds, sizeof (t->numsounds));
	for (i = 0; t->sounds && i < t->numsounds; i++)
	{
		const partsounds_t *s = &t->sounds[i];
		h = Harness_Hash64 (h, s->name, strlen (s->name) + 1);
		h = Harness_Hash64 (h, &s->vol, sizeof (s->vol));
		h = Harness_Hash64 (h, &s->atten, sizeof (s->atten));
		h = Harness_Hash64 (h, &s->delay, sizeof (s->delay));
		h = Harness_Hash64 (h, &s->pitch, sizeof (s->pitch));
		h = Harness_Hash64 (h, &s->weight, sizeof (s->weight));
	}

	h = Harness_Hash64 (h, &t->rampindexes, sizeof (t->rampindexes));
	for (i = 0; t->ramp && i < t->rampindexes; i++)
	{
		const ramp_t *r = &t->ramp[i];
		h = Harness_Hash64 (h, r->rgb, sizeof (r->rgb));
		h = Harness_Hash64 (h, &r->alpha, sizeof (r->alpha));
		h = Harness_Hash64 (h, &r->scale, sizeof (r->scale));
	}
	return h;
}

static uint64_t ftepart_hash_one_type (int side, uint64_t h, int i)
{
	part_type_t *base = ftepart_types (side);
	part_type_t	 t;

	h = ftepart_hash_arrays (h, &base[i]);
	ftepart_scrub_type (side, i, &t);
	return Harness_Hash64 (h, &t, sizeof (t));
}

uint64_t ctest_ftepart_hash_types (int side, uint64_t h)
{
	int n = ftepart_numtypes (side);
	int i;
	h = Harness_Hash64 (h, &n, sizeof (n));
	for (i = 0; i < n; i++)
		h = ftepart_hash_one_type (side, h, i);
	return h;
}

/* The run list as a sequence of type indices, so the order the update visits
 * types in is compared without comparing pointers. */
uint64_t ctest_ftepart_hash_runlist (int side, uint64_t h)
{
	part_type_t *t = ftepart_runlist (side);
	int			 guard = 0;
	while (t && guard++ < FTEPART_LIST_CAP)
	{
		int i = ftepart_index_of (side, t);
		h = Harness_Hash64 (h, &i, sizeof (i));
		t = t->nexttorun;
	}
	return h;
}

/* Every live particle of every type, in list order. `next` is skipped (it is
 * an allocator address) and so is the state union, whose active member depends
 * on the owning type; the trailstate arm is reduced to a null/non-null flag
 * and the nextemit arm hashed as the float it is. */
static uint64_t ftepart_hash_particle_list (uint64_t h, const part_type_t *type)
{
	const particle_t *p = type->particles;
	int				  guard = 0;
	const size_t	  head = offsetof (particle_t, die);
	const size_t	  tail = offsetof (particle_t, state);

	while (p && guard++ < FTEPART_LIST_CAP)
	{
		int flag;
		h = Harness_Hash64 (h, (const byte *)p + head, tail - head);
		if (type->emittime < 0)
		{
			flag = p->state.trailstate ? 1 : 0;
			h = Harness_Hash64 (h, &flag, sizeof (flag));
		}
		else
			h = Harness_Hash64 (h, &p->state.nextemit, sizeof (p->state.nextemit));
		h = Harness_Hash64 (h, &p->rotationspeed, sizeof (p->rotationspeed));
		p = p->next;
	}
	return h;
}

uint64_t ctest_ftepart_hash_particles (int side, uint64_t h)
{
	part_type_t *base = ftepart_types (side);
	int			 n = ftepart_numtypes (side);
	int			 i;
	for (i = 0; i < n; i++)
		h = ftepart_hash_particle_list (h, &base[i]);
	return h;
}

/* Fixed-width integer state. `out` must have room for CTEST_FTEPART_NUM_INTS.
 *  0 numparticletypes                8 deferred decals queued
 *  1 run list length                 9 deferred dlights queued
 *  2 free particle list length      10 num_type_emit_meta
 *  3 free beam list length          11 CL_ClearTrailStates calls
 *  4 free decal list length         12 CL_RegisterParticles calls
 *  5 num_particle_updates           13 Mod_ClipDecal calls (asserted 0)
 *  6 deferred effects queued        14 particle_traces_used
 *  7 deferred trails queued         15 live particles over all types
 */
void ctest_ftepart_read_ints (int side, int *out)
{
	deferred_queues_t *q = side ? c_ref_fte_deferred_queues : deferred_queues;
	part_type_t		  *base = ftepart_types (side);
	int				   n = ftepart_numtypes (side);
	int				   i;

	memset (out, 0, sizeof (int) * 16);

	out[0] = n;
	out[1] = 0;
	{
		part_type_t *t = ftepart_runlist (side);
		int			 guard = 0;
		while (t && guard++ < FTEPART_LIST_CAP)
		{
			out[1]++;
			t = t->nexttorun;
		}
	}
	out[2] = ftepart_plen (side ? c_ref_fte_free_particles : fte_free_particles);
	out[3] = ftepart_blen (side ? c_ref_fte_free_beams : free_beams);
	out[4] = ftepart_dlen (side ? c_ref_fte_free_decals : free_decals);
	out[5] = side ? c_ref_fte_num_particle_updates : num_particle_updates;

	for (i = 0; i < TASKS_MAX_WORKERS; i++)
	{
		out[6] += q[i].num_effects;
		out[7] += q[i].num_trails;
		out[8] += q[i].num_decals;
		out[9] += q[i].num_dlights;
	}

	out[10] = side ? c_ref_fte_num_type_emit_meta : num_type_emit_meta;
	out[11] = ftepart_cleartrails_calls[side];
	out[12] = ftepart_registerparticles_calls[side];
	out[13] = ftepart_clipdecal_calls[side];
	out[14] = (int)Atomic_LoadUInt32 (side ? &c_ref_fte_particle_traces_used : &particle_traces_used);

	for (i = 0; i < n; i++)
		out[15] += ftepart_plen (base[i].particles);
}

/* 0 particletime, 1 p_frametime. `out` must have room for 2. */
void ctest_ftepart_read_floats (int side, float *out)
{
	out[0] = side ? c_ref_fte_particletime : particletime;
	out[1] = side ? c_ref_fte_p_frametime : p_frametime;
}

/* Hashes the contents of every worker's deferred queues, not just their
 * lengths. The simulation half only ever *produces* into these arrays --
 * PScript_UpdateParticleTypes (r_part_fte.c:7227-7232) is what replays them,
 * and it belongs to the render half -- so without this the payload each
 * spawner writes is invisible to the gate and only the count is compared.
 *
 * Two members are reduced rather than hashed raw: deferred_trail_t::tsk is a
 * caller-owned trailstate_t ** (an address, and in this harness a stack
 * address), and deferred_decal_t::type is an interior pointer into that
 * side's own part_type array, so it is hashed as its index. */
uint64_t ctest_ftepart_hash_queues (int side, uint64_t h)
{
	deferred_queues_t *q = side ? c_ref_fte_deferred_queues : deferred_queues;
	int				   w;

	for (w = 0; w < TASKS_MAX_WORKERS; w++)
	{
		int i;

		h = Harness_Hash64 (h, &q[w].num_effects, sizeof (q[w].num_effects));
		for (i = 0; i < q[w].num_effects; i++)
		{
			deferred_effect_t e = q[w].effects[i];
			h = Harness_Hash64 (h, &e, sizeof (e));
		}

		h = Harness_Hash64 (h, &q[w].num_trails, sizeof (q[w].num_trails));
		for (i = 0; i < q[w].num_trails; i++)
		{
			deferred_trail_t t = q[w].trails[i];
			t.tsk = (trailstate_t **)(intptr_t)(t.tsk ? 1 : 0);
			h = Harness_Hash64 (h, &t, sizeof (t));
		}

		h = Harness_Hash64 (h, &q[w].num_decals, sizeof (q[w].num_decals));
		for (i = 0; i < q[w].num_decals; i++)
		{
			deferred_decal_t d = q[w].decals[i];
			d.type = (part_type_t *)(intptr_t)ftepart_index_of (side, d.type);
			h = Harness_Hash64 (h, &d, sizeof (d));
		}

		h = Harness_Hash64 (h, &q[w].num_dlights, sizeof (q[w].num_dlights));
		for (i = 0; i < q[w].num_dlights; i++)
		{
			deferred_dlight_t d = q[w].dlights[i];
			h = Harness_Hash64 (h, &d, sizeof (d));
		}
	}
	return h;
}

/* Hashes slooks as the index of the type whose looks it aliases.
 *
 * ftepart_hash_one_type reduces slooks to a set/unset flag instead, because
 * P_AllocateParticleType (r_part_fte.c:958-975) rebases part_run_list and
 * every nexttorun after its Mem_Realloc but *not* slooks: once the table
 * grows, every slooks left over from an earlier frame points into the freed
 * block, and whether it still compares equal to &base[k].looks depends only
 * on whether that side's realloc happened to grow in place.
 *
 * The dedup pass itself (r_part_fte.c:6621-6636) rebuilds every slooks from
 * scratch, so immediately after an update -- with no allocation in between --
 * the indices are exact on both sides and can be compared. Call this only
 * there; anywhere else it measures the allocator. */
uint64_t ctest_ftepart_hash_slooks (int side, uint64_t h)
{
	part_type_t *base = ftepart_types (side);
	int			 n = ftepart_numtypes (side);
	int			 i;

	for (i = 0; i < n; i++)
	{
		int j = -1;
		if (base[i].slooks)
		{
			int k;
			for (k = 0; k < n; k++)
				if (base[i].slooks == &base[k].looks)
				{
					j = k;
					break;
				}
		}
		h = Harness_Hash64 (h, &j, sizeof (j));
	}
	return h;
}

/* Closes the frame the way PScript_UpdateParticleTypes does, restricted to
 * the two things it does that the simulation half depends on.
 *
 * PScript_UpdateParticleTypes (r_part_fte.c:6786) is the render half's
 * per-type walk and is not one of the entry points this seam exposes, but two
 * of its statements are the *only* writers of state the simulation half then
 * reads back on the next frame:
 *
 *   r_part_fte.c:6800 + 7286-7290 -- the kill list the setup task built at
 *     :6726 and :6744 is spliced onto the head of free_particles. Nothing
 *     else ever returns an unlinked particle to the pool, so without this a
 *     harness that only drives the seam leaks the pool one frame at a time.
 *   r_part_fte.c:7292 -- `particletime += pframetime`. particletime is
 *     otherwise written only by PScript_ClearParticles (:3516), so it stays
 *     frozen, `kill->die < particletime` (:6721) never becomes true for a
 *     particle with an ordinary lifetime, and neither the ramp walk (:6478,
 *     :6488, :6503) nor the unlink path is reachable from a test.
 *
 * The render half copies p_kill_list into locals at :6800 and appends its own
 * per-type kills before the splice; with no render walk in this link, using
 * the globals directly is the same list. Call this exactly once per
 * ctest_ftepart_update: a second call with no intervening setup task (which
 * clears the list at :6617) would splice the same particles twice. */
void ctest_ftepart_finish_frame (int side)
{
	if (side)
	{
		if (c_ref_fte_p_kill_list)
		{
			c_ref_fte_p_kill_first->next = c_ref_fte_free_particles;
			c_ref_fte_free_particles = c_ref_fte_p_kill_list;
		}
		c_ref_fte_particletime += c_ref_fte_p_frametime;
	}
	else
	{
		if (p_kill_list)
		{
			p_kill_first->next = fte_free_particles;
			fte_free_particles = p_kill_list;
		}
		particletime += p_frametime;
	}
}

/* Resets the three seam counters so a test can measure one phase in
 * isolation. */
void ctest_ftepart_reset_counters (void)
{
	memset (ftepart_cleartrails_calls, 0, sizeof (ftepart_cleartrails_calls));
	memset (ftepart_registerparticles_calls, 0, sizeof (ftepart_registerparticles_calls));
	memset (ftepart_clipdecal_calls, 0, sizeof (ftepart_clipdecal_calls));
}

/* ---- cvar control -------------------------------------------------------
 *
 * PScript_Startup (r_part_fte.c:3361) sizes the three pools from
 * r_part_maxparticles / r_part_maxdecals once and never again, so the sizes
 * have to be set before the first ctest_ftepart_clear(side, 1). The defaults
 * (65536 / 8192) would work but make every free-list walk in
 * ctest_ftepart_read_ints six figures long, so the tests dial them down.
 *
 * r_fteparticles is per side (the oracle's is the renamed cvar_t
 * r_part_fte.c defines, the port's is stubs/pr_ext_ref.c:222's plain one);
 * r_particles is not renamed anywhere, so both sides read the one object and
 * setting it once is correct.
 */
void ctest_ftepart_set_limits (int side, float maxparticles, float maxdecals)
{
	if (side)
	{
		c_ref_fte_r_part_maxparticles.value = maxparticles;
		c_ref_fte_r_part_maxdecals.value = maxdecals;
	}
	else
	{
		r_part_maxparticles.value = maxparticles;
		r_part_maxdecals.value = maxdecals;
	}
}

void ctest_ftepart_set_enabled (int side, float fteparticles, float density)
{
	if (side)
	{
		c_ref_fte_r_fteparticles.value = fteparticles;
		c_ref_fte_r_part_density.value = density;
	}
	else
	{
		r_fteparticles.value = fteparticles;
		r_part_density.value = density;
	}
	r_particles.value = 1;
}

/* ---------------------------------------------------------------------------
 * The rendering half's Vulkan surface. r_part_fte.c:5547-6250 comes along
 * because a #include cannot take half a file; nothing in this link calls it
 * (PScript_DrawParticles and PScript_DrawParticleBatches have no caller, and
 * PScript_LayoutParticlesTask / PScript_EmitParticlesTask are only reachable
 * from a task graph this link does not build). These six are declared by
 * c_ref_prelude.h:2405-2410 and defined nowhere else, so they are defined
 * here and abort: reaching one means the composition changed and the gate
 * should say so instead of quietly rendering into an uninitialised device.
 * This is stubs/r_part_ref.c:264's pattern for the same problem.
 */

void R_AllocateVulkanMemory (vulkan_memory_t *memory, VkMemoryAllocateInfo *memory_allocate_info, vulkan_memory_type_t type, atomic_uint32_t *num_allocations)
{
	(void)memory, (void)memory_allocate_info, (void)type, (void)num_allocations;
	Sys_Error ("R_AllocateVulkanMemory: the r_part_fte.c rendering half is not driven in ctest");
}

void R_FreeVulkanMemory (vulkan_memory_t *memory, atomic_uint32_t *num_allocations)
{
	(void)memory, (void)num_allocations;
	Sys_Error ("R_FreeVulkanMemory: the r_part_fte.c rendering half is not driven in ctest");
}

void Fog_DisableGFog (cb_context_t *cbx)
{
	(void)cbx;
	Sys_Error ("Fog_DisableGFog: the r_part_fte.c rendering half is not driven in ctest");
}

void vkDestroyBuffer (VkDevice device, VkBuffer buffer, const void *allocator)
{
	(void)device, (void)buffer, (void)allocator;
	Sys_Error ("vkDestroyBuffer: the r_part_fte.c rendering half is not driven in ctest");
}

VkResult vkMapMemory (VkDevice device, VkDeviceMemory memory, VkDeviceSize offset, VkDeviceSize size, VkFlags flags, void **data)
{
	(void)device, (void)memory, (void)offset, (void)size, (void)flags, (void)data;
	Sys_Error ("vkMapMemory: the r_part_fte.c rendering half is not driven in ctest");
	return 0;
}

void vkUnmapMemory (VkDevice device, VkDeviceMemory memory)
{
	(void)device, (void)memory;
	Sys_Error ("vkUnmapMemory: the r_part_fte.c rendering half is not driven in ctest");
}
