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
// pr_edict_load_glue.c -- the C frame around the Rust progs loader.
//
// Compiled instead of pr_edict_load.c under -Duse_rust_progs (Phase 6 M6).
// Owns:
//   - PR_SwitchQCVM, the selector every other translation unit reaches the
//     ambient VM through (ADR-008). Phase 7 M9g moved the two pointers it
//     assigns -- qcvm and pr_global_struct -- into Rust
//     (rust/quake-capi/src/progs_load.rs), closing the ADR-007 row; this file
//     writes that storage but no longer defines it;
//   - COM_LoadFile, so com_filesize (THREAD_LOCAL) never has to be threaded
//     across the boundary;
//   - the engine lookups and va() the loader must call rather than
//     reimplement; and
//   - the five Host_Error raises (ADR-009).

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

/* status codes shared with rust/quake-capi/src/progs_load.rs (keep in sync) */
#define PRLOAD_OK						0
#define PRLOAD_FALSE					1 /* C's `return false`, message already printed */
#define PRLOAD_ERR_VERSION				2
#define PRLOAD_ERR_CRC					3
#define PRLOAD_ERR_STRINGS_PAST_END		4
#define PRLOAD_ERR_SAVEGLOBAL			5
#define PRLOAD_ERR_LUMP_RANGE			6
#define PRLOAD_ERR_ENTITYFIELDS			7
#define PRLOAD_ERR_TOO_SHORT			8
#define PRLOAD_ERR_UNTERMINATED_STRINGS 9

/* ---- the VM selection (the flip map keeps this side C) ----

   Phase 7 M9g: qcvm and pr_global_struct are Rust-owned storage, defined by
   rust/quake-capi/src/progs_load.rs under the `progs` feature (which tracks
   -Duse_rust_progs exactly, so pr_edict_load.c keeps its own copies for the
   oracle leg). progs.h:433-435 keeps both declarations, so every reader here
   and in the other 14 dereferencing files is unchanged. */

void PR_SwitchQCVM (qcvm_t *nvm)
{
	if (qcvm && nvm)
		Sys_Error ("PR_SwitchQCVM: A qcvm was already active");
	qcvm = nvm;
	if (qcvm)
		pr_global_struct = (globalvars_t *)qcvm->globals;
	else
		pr_global_struct = NULL;
}

void PRLoad_Glue_SwitchQCVM (qcvm_t *nvm)
{
	PR_SwitchQCVM (nvm);
}

/* PR_ClearProgs assigns qcvm directly rather than going through
   PR_SwitchQCVM, which is how it gets past the already-active Sys_Error.
   pr_global_struct is deliberately left alone, exactly as the C does. */
void PRLoad_Glue_DeselectQCVM (void)
{
	qcvm = NULL;
}

void PRLoad_Glue_SetPrGlobalStruct (float *globals)
{
	pr_global_struct = (globalvars_t *)globals;
}

/* ---- hash maps: the object ED_FindField and friends dereference ---- */

hash_map_t *PRLoad_Glue_MapCreate (void)
{
	return HashMap_Create (const char *, void *, &HashStr, &HashStrCmp);
}

void PRLoad_Glue_MapReserve (hash_map_t *map, int capacity)
{
	HashMap_Reserve (map, capacity);
}

/* The map stores the key *pointer*, not a copy, and dereferences it on
   lookup -- which is why PR_MergeEngineFieldDefs' va()-keyed vector
   components are unfindable once va's buffer ring wraps. Preserved. */
void PRLoad_Glue_MapInsert (hash_map_t *map, const char *key, const void *value)
{
	HashMap_InsertImpl (map, sizeof (const char *), sizeof (const void *), &key, &value);
}

void PRLoad_Glue_MapDestroy (hash_map_t *map)
{
	HashMap_Destroy (map);
}

/* ---- engine lookups and helpers still owned by C ---- */

void PRLoad_Glue_SetEmptyEngineString (void)
{
	PR_SetEngineString ("");
}

int PRLoad_Glue_FindFieldOfs (const char *name)
{
	return ED_FindFieldOffset (name);
}

/* PR_HasGlobal's whole test: the global exists, is an ev_float, and its
   value is readable through G_FLOAT. */
qboolean PRLoad_Glue_GlobalFloat (const char *name, float *out)
{
	ddef_t *g = ED_FindGlobal (name);
	if (!g || (g->type & ~DEF_SAVEGLOBAL) != ev_float)
		return false;
	*out = G_FLOAT (g->ofs);
	return true;
}

int PRLoad_Glue_FindFunction (const char *name)
{
	dfunction_t *f = ED_FindFunction (name);
	return f ? (int)(f - qcvm->functions) : -1;
}

const char *PRLoad_Glue_VaComponent (const char *name, int component)
{
	return va ("%s_%c", name, 'x' + component);
}

void PRLoad_Glue_ShutdownExtensions (void)
{
	PR_ShutdownExtensions ();
}

void PRLoad_Glue_EnableExtensions (ddef_t *globaldefs)
{
	PR_EnableExtensions (globaldefs);
}

qboolean PRLoad_Glue_IsServerVM (qcvm_t *vm)
{
	return vm == &sv.qcvm;
}

void PRLoad_Glue_SetEffectsMask (int mask)
{
	sv.effectsmask = mask;
}

/* ---- the exported entry points ---- */

void PR_ClearProgs (qcvm_t *vm)
{
	quake_rs_pr_clear_progs (vm);
}

qboolean PR_LoadProgs (const char *filename, qboolean fatal, unsigned int needcrc, const builtin_t *builtins, size_t numbuiltins)
{
	void  *data;
	size_t len;
	int	   detail = 0;
	int	   status;

	PR_ClearProgs (qcvm); // just in case.

	data = COM_LoadFile (filename, NULL);
	if (!data)
		return false;
	len = (size_t)com_filesize;

	status = quake_rs_pr_load_progs (qcvm, data, len, filename, fatal, (int)needcrc, builtins, numbuiltins, &detail);

	switch (status)
	{
	case PRLOAD_OK:
		return true;
	case PRLOAD_FALSE:
		return false;
	case PRLOAD_ERR_VERSION:
		Host_Error ("%s has wrong version number (%i should be %i)", filename, detail, PROG_VERSION);
	case PRLOAD_ERR_CRC:
		Host_Error ("%s system vars have been modified, progdefs.h is out of date", filename);
	case PRLOAD_ERR_STRINGS_PAST_END:
		Host_Error ("%s strings go past end of file\n", filename);
	case PRLOAD_ERR_SAVEGLOBAL:
		Host_Error ("PR_LoadProgs: pr_fielddefs[i].type & DEF_SAVEGLOBAL");
	case PRLOAD_ERR_LUMP_RANGE:
		Host_Error ("%s has a lump that runs past the end of the file", filename);
	case PRLOAD_ERR_UNTERMINATED_STRINGS:
		Host_Error ("%s has an unterminated string table", filename);
	case PRLOAD_ERR_TOO_SHORT:
		Host_Error ("%s is %i bytes, too short to hold a progs header", filename, detail);
	case PRLOAD_ERR_ENTITYFIELDS:
		Host_Error ("%s declares %i entity fields, which cannot be addressed", filename, detail);
	default:
		Host_Error ("PR_LoadProgs: unknown status %i", status);
	}
	return false;
}
