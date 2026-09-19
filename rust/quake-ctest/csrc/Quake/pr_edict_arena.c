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
// pr_edict_arena.c -- the edict arena and the progs string table, split
// verbatim out of pr_edict.c (Rust migration Phase 6 M2, behaviour-neutral).
//
// These two blocks are the ADR-006 subject matter -- the FIFO free list whose
// ordering decides entity numbering, and the string table with its negative
// engine-string handles -- and they are what quake-progs::{arena,strings}
// mirror. Keeping them in their own translation unit gives the differential
// oracle a small stub surface and lets the later milestones flip this file
// whole rather than threading #ifdefs through 2300 lines of pr_edict.c.

#include "quakedef.h"

// The one and only Hook instance : no need to either lock or multiple instances,
// because ED_* functions are only called from the main thread.
static ED_AllocHook_func ED_ALLOC_HOOK = NULL;

ED_AllocHook_func ED_AllocSetHook (ED_AllocHook_func alloc_hook)
{
	ED_AllocHook_func previous = ED_ALLOC_HOOK;
	ED_ALLOC_HOOK = alloc_hook;

	return previous;
}

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
	// get head of FIFO, if not empty
	edict_t *e = (qcvm->free_list.size > 0) ? EDICT_NUM (qcvm->free_list.circular_buffer[qcvm->free_list.head_index]) : NULL;

	if (e && ((e->freetime < MAX_EDICT_FREETIME_ALWAYS_REUSE) || (qcvm->time - e->freetime) > MIN_EDICT_AGE_FOR_REUSE))
	{
		assert (e->free);
		memset (&e->v, 0, qcvm->progs->entityfields * 4);
		e->free = false;

		// pop HEAD
		qcvm->free_list.head_index = (qcvm->free_list.head_index + 1) % MAX_EDICTS;
		qcvm->free_list.size -= 1;

		if (ED_ALLOC_HOOK)
			ED_ALLOC_HOOK (e);

		return e;
	}

	if (qcvm->num_edicts == qcvm->max_edicts) // johnfitz -- use sv.max_edicts instead of MAX_EDICTS
		Host_Error ("ED_Alloc: no free edicts (max_edicts is %i)", qcvm->max_edicts);

	e = EDICT_NUM (qcvm->num_edicts++);

	// vso - 'new' free edicts are not necessarily clean after a load/fastload
	// so completly reset their state from scratch in this case
	// force clean slate to prevent problems
	memset (e, 0, qcvm->edict_size);
	e->free = false;

	e->baseline = nullentitystate;

#if defined(DEBUG) || defined(_DEBUG)
	// fill debug fields, they were overwriten above:
	e->qcvm_owner = qcvm;
	e->edict_ptr = e;
	e->edict_num = qcvm->num_edicts - 1;
#endif

	if (ED_ALLOC_HOOK)
		ED_ALLOC_HOOK (e);

	return e;
}

/*
=================
ED_AddToFreeList
=================
*/
static void ED_AddToFreeList (edict_t *ed)
{
#if defined(DEBUG) || defined(_DEBUG)
	if (qcvm->free_list.size >= MAX_EDICTS)
		Host_Error ("ED_AddToFreeList : is full (qcvm 0x%p)", qcvm);
	if (qcvm->free_list.size >= qcvm->max_edicts)
		Host_Error ("ED_AddToFreeList : has more than max_edicts >= %i (qcvm 0x%p)", qcvm->max_edicts, qcvm);
#endif
	size_t add_index = (qcvm->free_list.head_index + qcvm->free_list.size) % MAX_EDICTS;
	qcvm->free_list.circular_buffer[add_index] = NUM_FOR_EDICT (ed);
	qcvm->free_list.size += 1;
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
	if (ed->free)
	{
		// Assert that this isn't linked to any area
		assert (!ed->area.prev);
		return;
	}

	SV_UnlinkEdict (ed); // unlink from world bsp

	ed->free = true;
	ed->v.model = 0;
	ed->v.takedamage = 0;
	ed->v.modelindex = 0;
	ed->v.colormap = 0;
	ed->v.skin = 0;
	ed->v.frame = 0;
	VectorCopy (vec3_origin, ed->v.origin);
	VectorCopy (vec3_origin, ed->v.angles);
	ed->v.nextthink = -1;
	ed->v.solid = 0;
	ed->alpha = ENTALPHA_DEFAULT; // johnfitz -- reset alpha for next entity

	ed->freetime = qcvm->time;

	ED_AddToFreeList (ed);
}

/*
=================
ED_RemoveFromFreeList
=================
*/
void ED_RemoveFromFreeList (edict_t *ed)
{
	const int	 num_edict_found = NUM_FOR_EDICT (ed);
	const size_t head_index = qcvm->free_list.head_index;
	// find the index where ed is...
	for (size_t i = 0; i < qcvm->free_list.size; i++)
	{
		const size_t found_index = (head_index + i) % MAX_EDICTS;

		if (qcvm->free_list.circular_buffer[found_index] == num_edict_found)
		{
			// overwrite found_index with head data, advance head.
			qcvm->free_list.circular_buffer[found_index] = qcvm->free_list.circular_buffer[head_index];
			qcvm->free_list.head_index = (head_index + 1) % MAX_EDICTS;
			qcvm->free_list.size -= 1;
			break;
		}
	}
}

static int ED_freetime_compare_func (const void *first, const void *second)
{
	int firstInt = *(const int *)first;
	int secondInt = *(const int *)second;
	return (int)copysign (1.0, EDICT_NUM (firstInt)->freetime - EDICT_NUM (secondInt)->freetime);
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
	bool  has_errors = false;
	// 1. for each free edict i of the free list : check is it effectively free
	//  and mark it at free_list_edicts[i] = 1 (0 = default = not free)
	byte *free_list_edicts = (byte *)Mem_Alloc (MAX_EDICTS * sizeof (byte));

	size_t current_index = qcvm->free_list.head_index;

	for (size_t j = 0; j < qcvm->free_list.size; j++)
	{
		int edict_num = qcvm->free_list.circular_buffer[current_index];

		edict_t *e = EDICT_NUM (edict_num);

		// check : e should be free
		if (!e->free)
		{
			Con_Warning ("ED_CheckFreeList: edict %i is in free-list but is NOT free\n", edict_num);
			has_errors = true;
		}

		free_list_edicts[edict_num] = 1;

		current_index = (current_index + 1) % MAX_EDICTS;
	}

	// 2. inverted check: Enumerate edicts in qcvm, they should have the same state as free_list_edicts
	for (int i = 0; i < qcvm->num_edicts; i++)
	{
		edict_t *e = EDICT_NUM (i);

		if (e->free)
		{
			if (free_list_edicts[i] != 1)
			{
				Con_Warning ("ED_CheckFreeList: edict %i is free, but is NOT in free-list\n", i);
				has_errors = true;
			}
		}
		else
		{
			if (free_list_edicts[i] != 0)
			{
				Con_Warning ("ED_CheckFreeList: edict %i is NOT free, but is in free-list\n", i);
				has_errors = true;
			}
		}
	}

	if (has_errors)
	{
		ED_RebuildFreeList (false);
	}

	Mem_Free (free_list_edicts);
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
	int *free_edicts_table = (int *)Mem_Alloc (qcvm->num_edicts * sizeof (int));

	int nb_free_edicts = 0;

	// 1. Enumerate free edict nums and put it in free_edicts_table
	for (int i = 0; i < qcvm->num_edicts; i++)
	{
		if (EDICT_NUM (i)->free)
		{
			if (force_free_reuse)
				EDICT_NUM (i)->freetime = 0.0f;

			free_edicts_table[nb_free_edicts++] = i;
		}
	}

	if (!force_free_reuse)
	{
		// 2.2 Sort free_edicts_table by their corresponding edict freetime
		qsort (free_edicts_table, nb_free_edicts, sizeof (int), ED_freetime_compare_func);
	}

	// 3. Reset freelist and insert by free_edicts_table order
	memset (&(qcvm->free_list), 0x0, sizeof (freelist_t));

	for (int j = 0; j < nb_free_edicts; j++)
	{
		ED_AddToFreeList (EDICT_NUM (free_edicts_table[j]));
	}

	Mem_Free (free_edicts_table);
}

//===========================================================================

#define PR_STRING_ALLOCSLOTS 256

static void PR_AllocStringSlots (void)
{
	qcvm->maxknownstrings += PR_STRING_ALLOCSLOTS;
	Con_DPrintf2 ("PR_AllocStringSlots: realloc'ing for %d slots\n", qcvm->maxknownstrings);
	qcvm->knownstrings = (const char **)Mem_Realloc ((void *)qcvm->knownstrings, qcvm->maxknownstrings * sizeof (char *));
	qcvm->knownstringsowned = (qboolean *)Mem_Realloc ((void *)qcvm->knownstringsowned, qcvm->maxknownstrings * sizeof (qboolean));
}

const char *PR_GetString (int num)
{
	if (num >= 0 && num < qcvm->stringssize)
		return qcvm->strings + num;
	else if (num < 0 && num >= -qcvm->numknownstrings)
	{
		if (!qcvm->knownstrings[-1 - num])
		{
			Host_Error ("PR_GetString: attempt to get a non-existant string %d\n", num);
			return "";
		}
		return qcvm->knownstrings[-1 - num];
	}
	else
	{
		return qcvm->strings;
		Host_Error ("PR_GetString: invalid string offset %d\n", num);
		return "";
	}
}

void PR_ClearEngineString (int num)
{
	if (num < 0 && num >= -qcvm->numknownstrings)
	{
		num = -1 - num;
		if (qcvm->knownstringsowned[num])
		{
			SAFE_FREE (qcvm->knownstrings[num]);
			qcvm->knownstringsowned[num] = false;
		}
		else
			qcvm->knownstrings[num] = NULL;
		if (qcvm->freeknownstrings > num)
			qcvm->freeknownstrings = num;
	}
}

int PR_SetEngineString (const char *s)
{
	int i;

	if (!s)
		return 0;
#if 0 /* can't: sv.model_precache & sv.sound_precache points to pr_strings */
	if (s >= qcvm->strings && s <= qcvm->strings + qcvm->stringssize)
		Host_Error("PR_SetEngineString: \"%s\" in pr_strings area\n", s);
#else
	if (s >= qcvm->strings && s <= qcvm->strings + qcvm->stringssize - 2)
		return (int)(s - qcvm->strings);
#endif
	for (i = 0; i < qcvm->numknownstrings; i++)
	{
		if (qcvm->knownstrings[i] == s)
			return -1 - i;
	}
	// new unknown engine string
	// Con_DPrintf ("PR_SetEngineString: new engine string %p\n", s);
	for (i = qcvm->freeknownstrings;; i++)
	{
		if (i < qcvm->numknownstrings)
		{
			if (qcvm->knownstrings[i])
				continue;
		}
		else
		{
			if (i >= qcvm->maxknownstrings)
				PR_AllocStringSlots ();
			qcvm->numknownstrings++;
		}
		break;
	}
	qcvm->freeknownstrings = i + 1;
	qcvm->knownstrings[i] = s;
	qcvm->knownstringsowned[i] = false;
	return -1 - i;
}

int PR_AllocString (int size, char **ptr)
{
	int i;

	if (!size)
		return 0;

	for (i = qcvm->freeknownstrings;; i++)
	{
		if (i < qcvm->numknownstrings)
		{
			if (qcvm->knownstrings[i])
				continue;
		}
		else
		{
			if (i >= qcvm->maxknownstrings)
				PR_AllocStringSlots ();
			qcvm->numknownstrings++;
		}
		break;
	}
	qcvm->freeknownstrings = i + 1;
	qcvm->knownstrings[i] = (char *)Mem_Alloc (size);
	qcvm->knownstringsowned[i] = true;
	if (ptr)
		*ptr = (char *)qcvm->knownstrings[i];
	return -1 - i;
}

void PR_ClearEdictStrings ()
{
	for (int i = qcvm->progsstrings; i < qcvm->numknownstrings; ++i)
		if (qcvm->knownstringsowned[i])
		{
			SAFE_FREE (qcvm->knownstrings[i]);
			qcvm->knownstringsowned[i] = false;
		}

#ifndef _DEBUG
	// do not reuse slots in debug builds to help catch stale references
	qcvm->freeknownstrings = qcvm->progsstrings;
#endif
}
