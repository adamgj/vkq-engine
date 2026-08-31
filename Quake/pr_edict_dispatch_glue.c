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
// pr_edict_dispatch_glue.c -- the C frame around the Rust ED_ParseGlobals /
// ED_ParseEdict dispatchers (Rust migration Phase 7 M5 T5.2).
//
// Compiled under -Duse_rust_host AND -Duse_rust_progs together: the Rust
// dispatchers (rust/quake-capi/src/progs_edict_dispatch.rs) call
// quake_rs_ed_parse_epair / quake_rs_ed_new_string directly -- those are
// progs-gated cores, only compiled under -Duse_rust_progs -- and also read
// sv.state and call SV_Precache_Model/SV_Precache_Sound (host-gated), so
// neither flag alone keeps the link working; this file (and the pr_edict.c
// rewrite that calls into it) must only be selected when both are set.
//
// Three jobs:
//  1. ED_FindGlobal / ED_FindField hash lookups, out-param wrapped so
//     cbindgen never has to see ddef_t (mirrors PRSave_Glue_FieldAtOfs's
//     established style).
//  2. sv.state == ss_loading, the one server_t read the dispatchers need
//     that has no ADR-011 mirror in Phase 7.
//  3. Guard the one call the dispatchers reach that can Host_Error while a
//     Rust frame is on the stack (ADR-009 rule 3): SV_Precache_Model's
//     `Mod_ForName (s, i == 1)` path Host_Errors when `crash` (i.e. `i ==
//     1`, "this is the first precache slot") is true and the file is
//     missing (Quake/gl_model.c:531). SV_Precache_Sound and
//     PF_SV_ForceParticlePrecache need no guard here: on every call site the
//     Rust dispatcher reaches them from, sv.state == ss_loading (mirroring
//     Quake/pr_edict.c:860-887's own guards on these same three calls), and
//     both functions gate their only Host_Error-reachable statements
//     (MSG_Write*/SV_Multicast) behind `sv.state != ss_loading` -- dead code
//     on this path.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#if defined(USE_RUST_HOST) && defined(USE_RUST_PROGS)

/* ---- ED_FindGlobal / ED_FindField, out-param wrapped ---- */

qboolean PREdictDispatch_Glue_FindGlobal (const char *name, unsigned short *out_type, unsigned short *out_ofs, int *out_s_name)
{
	ddef_t *def = ED_FindGlobal (name);
	if (!def)
		return false;
	*out_type = def->type;
	*out_ofs = def->ofs;
	*out_s_name = def->s_name;
	return true;
}

qboolean PREdictDispatch_Glue_FindField (const char *name, unsigned short *out_type, unsigned short *out_ofs, int *out_s_name)
{
	ddef_t *def = ED_FindField (name);
	if (!def)
		return false;
	*out_type = def->type;
	*out_ofs = def->ofs;
	*out_s_name = def->s_name;
	return true;
}

/* ---- server_t read with no ADR-011 mirror ---- */

qboolean PREdictDispatch_Glue_ServerLoading (void)
{
	return sv.state == ss_loading;
}

/* ---- the one guarded seam (ADR-009 rule 3) ---- */

typedef struct
{
	const char *s;
	int		   *out;
} predd_precache_model_arg_t;

static void PREdictDispatch_InvokePrecacheModel (void *p)
{
	predd_precache_model_arg_t *a = (predd_precache_model_arg_t *)p;
	*a->out = SV_Precache_Model (a->s);
}

int PREdictDispatch_Glue_PrecacheModel (const char *s, int *out)
{
	predd_precache_model_arg_t arg;

	arg.s = s;
	arg.out = out;
	*out = 0;
	return Host_Guard (PREdictDispatch_InvokePrecacheModel, &arg);
}

#endif /* USE_RUST_HOST && USE_RUST_PROGS */
