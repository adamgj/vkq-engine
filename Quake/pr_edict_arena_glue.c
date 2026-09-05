/*
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
// pr_edict_arena_glue.c -- the C frame around the Rust edict arena and progs
// string table.
//
// Compiled instead of pr_edict_arena.c under -Duse_rust_progs (Phase 7 M9d).
// Four things stay C here:
//
//   1. C-visible storage: ED_ALLOC_HOOK, the one file-static the flipped file
//      owned. It stays a C function pointer -- sv_phys.c installs one through
//      ED_AllocSetHook and ED_Alloc calls it -- so it never crosses the
//      boundary and the call ABI is unchanged.
//   2. The platform qsort and ED_freetime_compare_func, verbatim (ADR-010).
//      The comparator is (int)copysign (1.0, a - b), which is never 0, so it
//      is inconsistent and qsort's tie ordering is implementation-defined;
//      entity numbering is observable, so the same qsort and the same
//      comparator must keep deciding it.
//   3. The Host_Error trampolines (ADR-009 rule 3: no longjmp may cross a
//      Rust frame). The Rust cores return a status plus a detail int, and the
//      switch below issues the original message from this C frame. The
//      raise-capable entry points are ED_Alloc, ED_Free, ED_RemoveFromFreeList,
//      ED_CheckFreeList, ED_RebuildFreeList (all via EDICT_NUM/NUM_FOR_EDICT,
//      plus ED_AddToFreeList's two debug-only preconditions) and PR_GetString.
//   4. Everything else is a plain forward.
//
// PR_SetEngineString is deliberately NOT in that list: pr_edict_arena.c:351
// wraps its only Host_Error in `#if 0` (the comment there records why -- the
// precaches point into pr_strings), so the live path cannot raise and the
// wrapper needs no status. PR_ClearEngineString, PR_AllocString and
// PR_ClearEdictStrings cannot raise either.
//
// Con_Warning (ED_CheckFreeList's three diagnostics) and Con_DPrintf2
// (PR_AllocStringSlots) stay plain leaf calls made from the Rust side, per
// project policy.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

/* status codes shared with rust/quake-capi/src/pr_edict_arena.rs (keep in sync) */
#define PRARENA_OK						0
#define PRARENA_RAISE_NO_FREE_EDICTS	1
#define PRARENA_RAISE_EDICT_NUM			2
#define PRARENA_RAISE_NUM_FOR_EDICT		3
#define PRARENA_RAISE_FREELIST_FULL		4
#define PRARENA_RAISE_FREELIST_OVER_MAX 5
#define PRARENA_RAISE_STRING_MISSING	6

/* ---- C-visible storage ---- */

// The one and only Hook instance : no need to either lock or multiple instances,
// because ED_* functions are only called from the main thread.
static ED_AllocHook_func ED_ALLOC_HOOK = NULL;

ED_AllocHook_func ED_AllocSetHook (ED_AllocHook_func alloc_hook)
{
	ED_AllocHook_func previous = ED_ALLOC_HOOK;
	ED_ALLOC_HOOK = alloc_hook;

	return previous;
}

/* ---- the platform sort (ADR-010: its tie ordering is the contract) ---- */

static int ED_freetime_compare_func (const void *first, const void *second)
{
	int firstInt = *(const int *)first;
	int secondInt = *(const int *)second;
	return (int)copysign (1.0, EDICT_NUM_NO_CHECK (firstInt)->freetime - EDICT_NUM_NO_CHECK (secondInt)->freetime);
}

/* EDICT_NUM itself can raise -- its `n < 0 || n >= max_edicts` check is
   unconditional, not debug-only -- and a raise out of a qsort comparator would
   longjmp through libc, so the unchecked form is used here: the Rust caller
   range-checks every number against max_edicts before it can reach this. */
void PREdictArena_Glue_SortFreeEdicts (int *nums, size_t n)
{
	qsort (nums, n, sizeof (int), ED_freetime_compare_func);
}

/* ---- the exported entry points ---- */

/*
=================
ED_Alloc

Either finds a free edict, or allocates a new one.
Try to avoid reusing an entity that was recently freed, because it
can cause the client to think the entity morphed into something else
instead of being removed and recreated, which can cause interpolated
angles and bad trails.
=================
*/
edict_t *ED_Alloc (void)
{
	edict_t *e;
	int		 num = 0;
	int		 detail = 0;
	int		 status = quake_rs_ed_alloc (&num, &detail);

	switch (status)
	{
	case PRARENA_OK:
		break;
	case PRARENA_RAISE_NO_FREE_EDICTS:
		Host_Error ("ED_Alloc: no free edicts (max_edicts is %i)", detail);
	case PRARENA_RAISE_EDICT_NUM:
		Host_Error ("EDICT_NUM: bad edict_num %i", detail);
	default:
		Host_Error ("ED_Alloc: unknown status %i", status);
	}

	e = EDICT_NUM_NO_CHECK (num);

	if (ED_ALLOC_HOOK)
		ED_ALLOC_HOOK (e);

	return e;
}

/*
=================
ED_Free

Marks the edict as free
FIXME: walk all entities and NULL out references to this entity
=================
*/
void ED_Free (edict_t *ed)
{
	int detail = 0;
	int status = quake_rs_ed_free (ed, &detail);

	switch (status)
	{
	case PRARENA_OK:
		break;
	case PRARENA_RAISE_NUM_FOR_EDICT:
		Host_Error ("NUM_FOR_EDICT: bad pointer");
#if defined(DEBUG) || defined(_DEBUG)
	case PRARENA_RAISE_FREELIST_FULL:
		Host_Error ("ED_AddToFreeList : is full (qcvm 0x%p)", qcvm);
	case PRARENA_RAISE_FREELIST_OVER_MAX:
		Host_Error ("ED_AddToFreeList : has more than max_edicts >= %i (qcvm 0x%p)", detail, qcvm);
#endif
	default:
		Host_Error ("ED_Free: unknown status %i", status);
	}
}

/*
=================
ED_RemoveFromFreeList
=================
*/
void ED_RemoveFromFreeList (edict_t *ed)
{
	int status = quake_rs_ed_remove_from_free_list (ed);

	switch (status)
	{
	case PRARENA_OK:
		break;
	case PRARENA_RAISE_NUM_FOR_EDICT:
		Host_Error ("NUM_FOR_EDICT: bad pointer");
	default:
		Host_Error ("ED_RemoveFromFreeList: unknown status %i", status);
	}
}

/*
=================
ED_CheckFreeList
For debugging : Check that the list of free edicts in the free-list
is the same as the qcvm->edicts structure
=================
*/
void ED_CheckFreeList (void)
{
	int detail = 0;
	int status = quake_rs_ed_check_free_list (&detail);

	switch (status)
	{
	case PRARENA_OK:
		break;
	case PRARENA_RAISE_EDICT_NUM:
		Host_Error ("EDICT_NUM: bad edict_num %i", detail);
	default:
		Host_Error ("ED_CheckFreeList: unknown status %i", status);
	}
}

/*
=================
ED_RebuildFreeList
Rebuild the entire free list, ordering the free edicts
by the smallest freetime to maximize chance of reuse in ED_Alloc
=================
*/
void ED_RebuildFreeList (bool force_free_reuse)
{
	int detail = 0;
	int status = quake_rs_ed_rebuild_free_list (force_free_reuse, &detail);

	switch (status)
	{
	case PRARENA_OK:
		break;
	case PRARENA_RAISE_EDICT_NUM:
		Host_Error ("EDICT_NUM: bad edict_num %i", detail);
	default:
		Host_Error ("ED_RebuildFreeList: unknown status %i", status);
	}
}

//===========================================================================

const char *PR_GetString (int num)
{
	const char *s = NULL;
	int			status = quake_rs_pr_get_string (num, &s);

	switch (status)
	{
	case PRARENA_OK:
		return s;
	case PRARENA_RAISE_STRING_MISSING:
		Host_Error ("PR_GetString: attempt to get a non-existant string %d\n", num);
		return "";
	default:
		Host_Error ("PR_GetString: unknown status %i", status);
	}

	return "";
}

void PR_ClearEngineString (int num)
{
	quake_rs_pr_clear_engine_string (num);
}

int PR_SetEngineString (const char *s)
{
	return quake_rs_pr_set_engine_string (s);
}

int PR_AllocString (int size, char **ptr)
{
	return quake_rs_pr_alloc_string (size, ptr);
}

void PR_ClearEdictStrings ()
{
	quake_rs_pr_clear_edict_strings ();
}
