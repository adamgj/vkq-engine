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
// pr_edict_parse_glue.c -- the C frame around the Rust value parser.
//
// Compiled instead of pr_edict_parse.c under -Duse_rust_progs (Phase 6 M5).
// Owns the platform libc conversions the Rust side must not reimplement
// (ADR-010), the engine lookups that are still C, and the one raise (ADR-009).

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

/* status codes shared with rust/quake-capi/src/progs_parse.rs (keep in sync) */
#define PRPARSE_OK				  0
#define PRPARSE_FALSE			  1 /* C's `return false`, not an error */
#define PRPARSE_ERR_ENTITY_RANGE  2
#define PRPARSE_ERR_BAD_EDICT_NUM 3
#define PRPARSE_ERR_FREELIST_FULL 4

/* ---- the platform conversions (ADR-010: their rounding is the contract) ---- */

double PRParse_Glue_Atof (const char *s)
{
	return atof (s);
}

int PRParse_Glue_Atoi (const char *s)
{
	return atoi (s);
}

long long PRParse_Glue_Strtoll (const char *s)
{
	return strtoll (s, NULL, 0);
}

unsigned long long PRParse_Glue_Strtoull (const char *s)
{
	return strtoull (s, NULL, 0);
}

/* ---- engine lookups still owned by pr_edict.c ---- */

int PRParse_Glue_FindFieldOfs (const char *name)
{
	ddef_t *def = ED_FindField (name);
	return def ? (int)def->ofs : -1;
}

int PRParse_Glue_FindFunction (const char *name)
{
	dfunction_t *f = ED_FindFunction (name);
	return f ? (int)(f - qcvm->functions) : -1;
}

/* Audited for ADR-009 (Phase 6 M5 review): SV_UnlinkEdict (world.c) tests
   ent->area.prev, calls RemoveLink (common.c) and nulls two pointers. Neither
   can Host_Error, so this callback needs no Host_Guard.

   EDICT_NUM itself *can* raise -- its `n < 0 || n >= max_edicts` check is
   unconditional, not debug-only -- so the unchecked form is used here: the
   Rust caller has already bounds-checked `edict_num` against max_edicts and
   raised for a negative one before reaching this point. */
void PRParse_Glue_UnlinkEdict (int edict_num)
{
	SV_UnlinkEdict (EDICT_NUM_NO_CHECK (edict_num));
}

/* ---- the exported entry points ---- */

string_t ED_NewString (const char *string)
{
	return quake_rs_ed_new_string (string);
}

qboolean ED_ParseEpair (void *base, ddef_t *key, const char *s, qboolean zoned)
{
	int detail = 0;
	int status = quake_rs_ed_parse_epair (base, key->type, key->ofs, key->s_name, s, zoned, &detail);

	switch (status)
	{
	case PRPARSE_OK:
		return true;
	case PRPARSE_FALSE:
		return false;
	case PRPARSE_ERR_ENTITY_RANGE:
		Host_Error ("ED_ParseEpair: ev_entity %d too large (max_edicts is %i)", detail, qcvm->max_edicts);
	case PRPARSE_ERR_BAD_EDICT_NUM:
		Host_Error ("EDICT_NUM: bad edict_num %i", detail);
	case PRPARSE_ERR_FREELIST_FULL:
		Host_Error ("ED_AddToFreeList : has more than max_edicts >= %i (qcvm 0x%p)", detail, qcvm);
	default:
		Host_Error ("ED_ParseEpair: unknown status %i", status);
	}
	return false;
}
