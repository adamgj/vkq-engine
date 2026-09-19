/*
Copyright (C) 1996-2001 Id Software, Inc.
Copyright (C) 2002-2009 John Fitzgibbons and others
Copyright (C) 2007-2008 Kristian Duske
Copyright (C) 2010-2014 QuakeSpasm developers
Copyright (C) 2016 Axel Gneiting
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
// r_part_glue.c -- the C frame around the Rust classic-particle port.
//
// Compiled instead of r_part.c under -Duse_rust_host (Rust migration Phase 7
// M10f-1, T10.5). Unlike the other Pattern A swaps this file is a *split*, not
// just a frame: only r_part.c's simulation half moved to Rust
// (rust/quake-capi/src/r_part.rs). Five jobs:
//
//  1. Keep r_part.c's rendering half in C, verbatim and in its original order
//     (r_part.c:54-221 and r_part.c:951-1106). It is Vulkan-typed throughout
//     -- cb_context_t, VkBuffer, R_VertexAllocate, the particle pipelines --
//     and the renderer belongs to Phase 8 per ROADMAP.md, so porting it now
//     would be out of roadmap order. (Phase 8 M8 ported it to
//     rust/quake-capi/src/r_part_render.rs; the C copy is now fenced under
//     #ifndef USE_RUST_RENDER for the build-rs-crender leg.)
//
//  2. Own the particle pool (r_part.c:38, :42). ADR-007: R_DrawParticlesFaces
//     walks active_particles and R_InitParticleIndexBuffer sizes itself from
//     r_numparticles, so a live C reader survives the port and the storage
//     stays here, exactly as sbar_glue.c keeps fragsort/scoreboardlines. The
//     four objects lose their `static` so the Rust core can extern them; no
//     other translation unit defines those names (r_part_fte.c's same-named
//     globals are static, r_part_fte.c:459-461).
//
//  3. Own the cvars and texture state the rendering half reads: r_particles,
//     r_quadparticles, particletexture..particletexture4, texturescalefactor
//     and particle_index_buffer. r_quadparticles also loses its `static` for
//     the same reason as the pool.
//
//  4. Guard R_InitParticles' two Cvar_RegisterVariable calls, which are
//     Host_Reraise wrappers under -Duse_rust_cvar (ADR-009 rule 3), and
//     re-raise from R_InitParticles what the guard caught. This is the
//     module's only raise site; every other entry point is a plain forward.
//
//  5. Bridge the two halves: RPart_Glue_InitRender runs R_InitParticles'
//     rendering tail (r_part.c:250-254) and RPart_Glue_SetParticleTexture_f
//     gives the file-static cvar callback external linkage so the Rust core
//     can hand it to Cvar_SetCallback.
//
// ramp1/ramp2/ramp3 (r_part.c:34-36) and avelocities/beamlength
// (r_part.c:268-269) are read only by the simulation, so they moved to Rust
// and are not duplicated here.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

/* ---------------------------------------------------------------------------
 * C-visible objects (r_part.c:38-52). The pool and r_quadparticles are no
 * longer static: rust/quake-c-sys/src/r_part.rs externs them.
 */

particle_t *active_particles, *free_particles, *particles;

// beware: different from the r_part_fte.c r_numparticles one, this is for classic particles,
// set by "-particles" command line.
int r_numparticles;

cvar_t r_particles = {"r_particles", "1", CVAR_ARCHIVE};		 // johnfitz
cvar_t r_quadparticles = {"r_quadparticles", "1", CVAR_ARCHIVE}; // johnfitz

/* Phase 8 M8: under -Duse_rust_render the rendering half below -- the
 * particle textures, texturescalefactor, the index buffer and the draw
 * entry points -- lives in rust/quake-capi/src/r_part_render.rs, which
 * exports the same C names. The C copy stays for build-rs-crender. */

/* ---------------------------------------------------------------------------
 * Guarded callback (ADR-009 rule 3).
 */

/* r_part.c:247, :249 -- one Cvar_RegisterVariable. */
static void RPart_InvokeRegisterVariable (void *p)
{
	Cvar_RegisterVariable ((cvar_t *)p);
}

int RPart_Glue_RegisterVariable (cvar_t *var)
{
	return Host_Guard (RPart_InvokeRegisterVariable, var);
}

/* r_part.c:376 -- fscanf. Not a trampoline. A direct fscanf extern on the Rust
   side compiled and linked under cargo test but failed the meson/clang-cl
   engine link with LNK2019: unresolved external symbol fscanf, in build-rs,
   build-rs-cprogs and build-rs-trace (M10f-1 integration). The mechanism was
   not established: the pre-existing Rust fscanf externs used by menu.rs and
   cl_demo.rs resolve in the same binary, so it is not that fscanf lacks an
   importable symbol. Keeping the call here removes the dependency and keeps
   libc's exact scanner, which is the compat surface -- the pointfile is plain
   text and reimplementing float parsing in Rust would be a new divergence,
   not a port. */
int RPart_Glue_ScanPoint (FILE *f, vec3_t org)
{
	return fscanf (f, "%f %f %f\n", &org[0], &org[1], &org[2]);
}

/* ---------------------------------------------------------------------------
 * Re-raising public entry point (ADR-009).
 */

/* r_part.c:227 */
void R_InitParticles (void)
{
	int r = quake_rs_rpart_init_particles ();
	Host_Reraise (r);
}

/* ---------------------------------------------------------------------------
 * Non-raising public entry points: plain forwards to the Rust cores.
 */

/* r_part.c:274 -- the core takes the one field it reads; cbindgen cannot spell
 * entity_t. */
void R_EntityParticles (entity_t *ent)
{
	quake_rs_rpart_entity_particles (ent->origin);
}

/* r_part.c:332 */
void R_ClearParticles (void)
{
	quake_rs_rpart_clear_particles ();
}

/* r_part.c:349 */
void R_ReadPointFile_f (void)
{
	quake_rs_rpart_read_point_file_f ();
}

/* r_part.c:409 */
void R_ParseParticleEffect (void)
{
	quake_rs_rpart_parse_particle_effect ();
}

/* r_part.c:433 */
void R_ParticleExplosion (vec3_t org)
{
	quake_rs_rpart_particle_explosion (org);
}

/* r_part.c:476 */
void R_ParticleExplosion2 (vec3_t org, int colorStart, int colorLength)
{
	quake_rs_rpart_particle_explosion2 (org, colorStart, colorLength);
}

/* r_part.c:509 */
void R_BlobExplosion (vec3_t org)
{
	quake_rs_rpart_blob_explosion (org);
}

/* r_part.c:553 */
void R_RunParticleEffect (vec3_t org, vec3_t dir, int color, int count)
{
	quake_rs_rpart_run_particle_effect (org, dir, color, count);
}

/* r_part.c:610 */
void R_LavaSplash (vec3_t org)
{
	quake_rs_rpart_lava_splash (org);
}

/* r_part.c:651 */
void R_TeleportSplash (vec3_t org)
{
	quake_rs_rpart_teleport_splash (org);
}

/* r_part.c:694 -- start is advanced in place. */
void R_RocketTrail (vec3_t start, vec3_t end, int type)
{
	quake_rs_rpart_rocket_trail (start, end, type);
}

/* r_part.c:806 */
void CL_RunParticles (void)
{
	quake_rs_rpart_run_particles ();
}

/* r_part.c:927 */
uint64_t Harness_HashParticles (uint64_t h)
{
	return quake_rs_rpart_hash_particles (h);
}
