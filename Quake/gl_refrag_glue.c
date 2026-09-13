/*
Copyright (C) 1996-2001 Id Software, Inc.
Copyright (C) 2002-2009 John Fitzgibbons and others
Copyright (C) 2007-2008 Kristian Duske
Copyright (C) 2010-2014 QuakeSpasm developers
Copyright (C) 2016 Axel Gneiting

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

// gl_refrag_glue.c -- the C that stays with gl_refrag.c under -Duse_rust_render
// (Rust migration Phase 8 M7). R_StoreEfrags reaches PScript_RunParticleEffectState
// and R_AllocateEntityBLAS, both of which can Host_Error, so those two calls run
// under Host_Guard here and the Rust loop returns the guard code for this TU to
// Host_Reraise once its frames have unwound (ADR-009: no longjmp through Rust).

#include "quakedef.h"

int RRefrag_StoreEfrags (efrag_t **ppefrag);

void R_StoreEfrags (efrag_t **ppefrag)
{
	Host_Reraise (RRefrag_StoreEfrags (ppefrag));
}

/* gl_refrag.c:264 -- R_AllocateEntityBLAS (pent) */
static void Refrag_InvokeAllocateEntityBLAS (void *p)
{
	R_AllocateEntityBLAS ((entity_t *)p);
}
int Refrag_Glue_AllocateEntityBLAS (void *ent)
{
	return Host_Guard (Refrag_InvokeAllocateEntityBLAS, ent);
}

/* gl_refrag.c:240, :258 -- PScript_RunParticleEffectState (...) */
typedef struct
{
	const float *org;
	const float *dir;
	float		 count;
	int			 typenum;
	void	   **tsk;
} refrag_pstate_arg_t;

static void Refrag_InvokeRunParticleEffectState (void *p)
{
	refrag_pstate_arg_t *a = (refrag_pstate_arg_t *)p;
	vec3_t				 o, d;
	VectorCopy (a->org, o);
	VectorCopy (a->dir, d);
	PScript_RunParticleEffectState (o, d, a->count, a->typenum, (struct trailstate_s **)a->tsk);
}
int Refrag_Glue_RunParticleEffectState (const float *org, const float *dir, float count, int typenum, void **tsk)
{
	refrag_pstate_arg_t arg;
	arg.org = org;
	arg.dir = dir;
	arg.count = count;
	arg.typenum = typenum;
	arg.tsk = tsk;
	return Host_Guard (Refrag_InvokeRunParticleEffectState, &arg);
}
