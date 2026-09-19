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
// pr_edict_save.c -- the savegame writer, split verbatim out of pr_edict.c
// (Rust migration Phase 6 M4, behaviour-neutral).
//
// These three functions are the subject of ADR-019's gate 2: save_diff.py
// byte-compares the .sav two builds produce, so their output is the
// compatibility contract. Splitting them into their own translation unit gives
// the differential oracle a small stub surface and lets the milestone flip
// them with the established meson file swap.
//
// PR_UglyValueString stays an exported symbol either way: progs.h publishes it
// for pr_ext.c.

#include "quakedef.h"

/*
============
PR_UglyValueString
(etype_t type, eval_t *val)

Returns a string describing *data in a type specific manner
Easier to parse than PR_ValueString
=============
*/
const char *PR_UglyValueString (int type, eval_t *val)
{
	static char	 line[1024];
	ddef_t		*def;
	dfunction_t *f;

	type &= ~DEF_SAVEGLOBAL;

	switch (type)
	{
	case ev_string:
		q_snprintf (line, sizeof (line), "%s", PR_GetString (val->string));
		break;
	case ev_entity:
		q_snprintf (line, sizeof (line), "%i", NUM_FOR_EDICT (PROG_TO_EDICT (val->edict)));
		break;
	case ev_function:
		f = qcvm->functions + val->function;
		q_snprintf (line, sizeof (line), "%s", PR_GetString (f->s_name));
		break;
	case ev_field:
		def = ED_FieldAtOfs (val->_int);
		q_snprintf (line, sizeof (line), "%s", PR_GetString (def->s_name));
		break;
	case ev_void:
		q_snprintf (line, sizeof (line), "void");
		break;
	case ev_float:
		q_snprintf (line, sizeof (line), "%f", val->_float);
		break;
	case ev_ext_integer:
		q_snprintf (line, sizeof (line), "%i", val->_int);
		break;
	case ev_ext_uint32:
		sprintf (line, "%u", val->_uint32);
		break;
	case ev_ext_sint64:
		sprintf (line, "%" PRIi64, val->_sint64);
		break;
	case ev_ext_uint64:
		sprintf (line, "%" PRIu64, val->_uint64);
		break;
	case ev_ext_double:
		q_snprintf (line, sizeof (line), "%f", val->_double);
		break;
	case ev_vector:
		q_snprintf (line, sizeof (line), "%f %f %f", val->vector[0], val->vector[1], val->vector[2]);
		break;
	default:
		q_snprintf (line, sizeof (line), "bad type %i", type);
		break;
	}

	return line;
}

/*
=============
ED_Write

For savegames
=============
*/
void ED_Write (FILE *f, edict_t *ed)
{
	ddef_t	   *d;
	int		   *v;
	int			i, j;
	const char *name;
	int			type;

	if (ed->free)
	{
		fprintf (f, "{\n}\n");
		return;
	}

	fprintf (f, "{\n");

	for (i = 1; i < qcvm->progs->numfielddefs; i++)
	{
		d = &qcvm->fielddefs[i];
		type = d->type;
		// exclude tagged DEF_SAVEGLOBAL, which are saved by the dedicated ED_WriteGlobals()
		if (type & DEF_SAVEGLOBAL)
			continue;

		if (type >= NUM_TYPE_SIZES)
			continue;

		name = PR_GetString (d->s_name);
		j = strlen (name);
		if (j > 1 && name[j - 2] == '_')
			continue; // skip _x, _y, _z vars

		v = (int *)((char *)&ed->v + d->ofs * 4);

		// if the value is still all 0, skip the field
		assert (type < NUM_TYPE_SIZES && ((type == ev_vector && type_size[type] == 3) || (type != ev_vector && type_size[type] == 1)));
		if (type != ev_vector && !v[0])
			continue;
		if (type == ev_vector && !v[0] && !v[1] && !v[2])
			continue;

		fprintf (f, "\"%s\" \"%s\"\n", name, PR_UglyValueString (d->type, (eval_t *)v));
	}

	// johnfitz -- save entity alpha manually when progs.dat doesn't know about alpha
	if (qcvm->extfields.alpha < 0 && ed->alpha != ENTALPHA_DEFAULT)
		fprintf (f, "\"alpha\" \"%f\"\n", ENTALPHA_TOSAVE (ed->alpha));
	// johnfitz

	fprintf (f, "}\n");
}

/*
=============
ED_WriteGlobals
=============
*/
void ED_WriteGlobals (FILE *f)
{
	ddef_t	   *def;
	int			i;
	const char *name;
	int			type;

	fprintf (f, "{\n");
	for (i = 0; i < qcvm->progs->numglobaldefs; i++)
	{
		def = &qcvm->globaldefs[i];
		type = def->type;
		if (!(def->type & DEF_SAVEGLOBAL))
			continue;
		type &= ~DEF_SAVEGLOBAL;

		if (type != ev_string && type != ev_float && type != ev_ext_double && type != ev_ext_integer && type != ev_ext_uint32 && type != ev_ext_sint64 &&
			type != ev_ext_uint64 && type != ev_entity)
			continue;

		name = PR_GetString (def->s_name);
		fprintf (f, "\"%s\" ", name);
		fprintf (f, "\"%s\"\n", PR_UglyValueString (type, (eval_t *)&qcvm->globals[def->ofs]));
	}
	fprintf (f, "}\n");
}
