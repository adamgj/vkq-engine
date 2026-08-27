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
// pr_edict_save_glue.c -- the C frame around the Rust savegame writer.
//
// Compiled instead of pr_edict_save.c under -Duse_rust_progs (Phase 6 M4).
// The Rust side builds the exact bytes C would have fprintf'd; this file owns
// the FILE * writes, PR_UglyValueString's static return buffer, and the one
// error path (ADR-009).

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

/* status codes shared with rust/quake-capi/src/progs_save.rs (keep in sync) */
#define PRSAVE_OK			 0
#define PRSAVE_ERR_NO_STRING 1
#define PRSAVE_ERR_BAD_EDICT 2

/* ED_FieldAtOfs returns a ddef_t *, which is not a bindgen-clean type; hand
   the Rust side the three fields it needs instead. */
int PRSave_Glue_FieldAtOfs (int ofs, int *type, int *field_ofs, int *s_name)
{
	ddef_t *d = ED_FieldAtOfs (ofs);
	if (!d)
		return 0;
	*type = d->type;
	*field_ofs = d->ofs;
	*s_name = d->s_name;
	return 1;
}

static void PRSave_Raise (int status, int detail)
{
	if (status == PRSAVE_ERR_NO_STRING)
		Host_Error ("PR_GetString: attempt to get a non-existant string %d\n", detail);
	if (status == PRSAVE_ERR_BAD_EDICT)
		Host_Error ("NUM_FOR_EDICT: bad pointer");
	Host_Error ("progs savegame writer: unknown status %i", status);
}

const char *PR_UglyValueString (int type, eval_t *val)
{
	static char line[1024];
	int			detail = 0;
	int			status = quake_rs_pr_ugly_value_string (type, (const int *)val, line, sizeof (line), &detail);
	if (status != PRSAVE_OK)
		PRSave_Raise (status, detail);
	return line;
}

void ED_Write (FILE *f, edict_t *ed)
{
	const unsigned char *bytes = NULL;
	size_t				 len = 0;
	int					 detail = 0;
	int					 status = quake_rs_ed_write (NUM_FOR_EDICT_NO_CHECK (ed), &bytes, &len, &detail);
	if (status != PRSAVE_OK)
		PRSave_Raise (status, detail);
	if (len)
		fwrite (bytes, 1, len, f);
}

void ED_WriteGlobals (FILE *f)
{
	const unsigned char *bytes = NULL;
	size_t				 len = 0;
	int					 detail = 0;
	int					 status = quake_rs_ed_write_globals (&bytes, &len, &detail);
	if (status != PRSAVE_OK)
		PRSave_Raise (status, detail);
	if (len)
		fwrite (bytes, 1, len, f);
}
