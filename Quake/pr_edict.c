/*
Copyright (C) 1996-2001 Id Software, Inc.
Copyright (C) 2002-2009 John Fitzgibbons and others
Copyright (C) 2010-2014 QuakeSpasm developers

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
// sv_edict.c -- entity dictionary

#include "quakedef.h"
#if defined(USE_RUST_HOST) && defined(USE_RUST_PROGS)
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"
#endif

const int type_size[NUM_TYPE_SIZES] = {
	1, // ev_void
	1, // sizeof(string_t) / 4		// ev_string
	1, // ev_float
	3, // ev_vector
	1, // ev_entity
	1, // ev_field
	1, // sizeof(func_t) / 4		// ev_function
	1  // sizeof(void *) / 4		// ev_pointer
};

cvar_t nomonsters = {"nomonsters", "0", CVAR_NONE};
cvar_t gamecfg = {"gamecfg", "0", CVAR_NONE};
cvar_t scratch1 = {"scratch1", "0", CVAR_NONE};
cvar_t scratch2 = {"scratch2", "0", CVAR_NONE};
cvar_t scratch3 = {"scratch3", "0", CVAR_NONE};
cvar_t scratch4 = {"scratch4", "0", CVAR_NONE};
cvar_t savedgamecfg = {"savedgamecfg", "0", CVAR_ARCHIVE};
cvar_t saved1 = {"saved1", "0", CVAR_ARCHIVE};
cvar_t saved2 = {"saved2", "0", CVAR_ARCHIVE};
cvar_t saved3 = {"saved3", "0", CVAR_ARCHIVE};
cvar_t saved4 = {"saved4", "0", CVAR_ARCHIVE};

//===========================================================================

/*
============
ED_GlobalAtOfs
============
*/
static ddef_t *ED_GlobalAtOfs (int ofs)
{
	ddef_t *def;
	int		i;

	for (i = 0; i < qcvm->progs->numglobaldefs; i++)
	{
		def = &qcvm->globaldefs[i];
		if (def->ofs == ofs)
			return def;
	}
	return NULL;
}

/*
============
ED_FieldAtOfs
============
*/
ddef_t *ED_FieldAtOfs (int ofs)
{
	ddef_t *def;
	int		i;

	for (i = 1; i < qcvm->progs->numfielddefs; i++)
	{
		def = &qcvm->fielddefs[i];
		if (def->ofs == ofs)
			return def;
	}
	return NULL;
}

/*
============
ED_FindField
============
*/
ddef_t *ED_FindField (const char *name)
{
	ddef_t **def_ptr = HashMap_Lookup (ddef_t *, qcvm->fielddefs_map, &name);
	if (def_ptr)
		return *def_ptr;
	return NULL;
}

/*
 */
int ED_FindFieldOffset (const char *name)
{
	ddef_t *def = ED_FindField (name);
	if (!def)
		return -1;
	return def->ofs;
}

/*
============
ED_FindGlobal
============
*/
ddef_t *ED_FindGlobal (const char *name)
{
	ddef_t **def_ptr = HashMap_Lookup (ddef_t *, qcvm->globaldefs_map, &name);
	if (def_ptr)
		return *def_ptr;
	return NULL;
}

/*
============
ED_FindFunction
============
*/
dfunction_t *ED_FindFunction (const char *fn_name)
{
	dfunction_t **func_ptr = HashMap_Lookup (dfunction_t *, qcvm->function_map, &fn_name);
	if (func_ptr)
		return *func_ptr;
	return NULL;
}

/*
============
GetEdictFieldValue
============
*/
eval_t *GetEdictFieldValue (edict_t *ed, int fldofs)
{
	if (fldofs < 0)
		return NULL;

	return (eval_t *)((char *)&ed->v + fldofs * 4);
}

/*
============
GetEdictFieldValueByName
============
*/
eval_t *GetEdictFieldValueByName (edict_t *ed, const char *name)
{
	return GetEdictFieldValue (ed, ED_FindFieldOffset (name));
}

/*
============
PR_FloatFormat
============
*/
static const char *PR_FloatFormat (float f)
{
	return fabs (f - round (f)) < 0.05f ? "% 5.0f  " : "% 7.1f";
}

/*
============
PR_DoubleFormat
============
*/
static const char *PR_DoubleFormat (double d)
{
	return fabs (d - round (d)) < 0.05 ? "% 13.0lf  " : "% 15.1lf";
}

/*
============
PR_ValueString
(etype_t type, eval_t *val)

Returns a string describing *data in a type specific manner
=============
*/
static const char *PR_ValueString (int type, eval_t *val)
{
	static char	 line[512];
	char		 fmt[64];
	const char	*str;
	ddef_t		*def;
	dfunction_t *f;
	edict_t		*ed;

	type &= ~DEF_SAVEGLOBAL;

	switch (type)
	{
	case ev_string:
		q_snprintf (line, sizeof (line), "%s", PR_GetString (val->string));
		break;
	case ev_entity:
		ed = PROG_TO_EDICT (val->edict);
		str = PR_GetString (ed->v.classname);
		q_snprintf (line, sizeof (line), *str ? "entity %i (%s)" : "entity %i", NUM_FOR_EDICT (ed), PR_GetString (ed->v.classname));
		break;
	case ev_function:
		f = qcvm->functions + val->function;
		q_snprintf (line, sizeof (line), "%s()", PR_GetString (f->s_name));
		break;
	case ev_field:
		def = ED_FieldAtOfs (val->_int);
		q_snprintf (line, sizeof (line), ".%s", PR_GetString (def->s_name));
		break;
	case ev_void:
		q_snprintf (line, sizeof (line), "void");
		break;
	case ev_float:
		// Note: leading space, so that float fields are aligned with the first value in vector fields
		q_snprintf (fmt, sizeof (fmt), " %s", PR_FloatFormat (val->_float));
		q_snprintf (line, sizeof (line), fmt, val->_float);
		break;
	case ev_ext_double:
		// Note: leading space, so that double fields are aligned with the first value in vector fields
		q_snprintf (fmt, sizeof (fmt), " %s", PR_DoubleFormat (val->_double));
		q_snprintf (line, sizeof (line), fmt, val->_double);
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
	case ev_vector:
		q_snprintf (fmt, sizeof (fmt), "'%s %s %s'", PR_FloatFormat (val->vector[0]), PR_FloatFormat (val->vector[1]), PR_FloatFormat (val->vector[2]));
		q_snprintf (line, sizeof (line), fmt, val->vector[0], val->vector[1], val->vector[2]);
		break;
	case ev_pointer:
		q_snprintf (line, sizeof (line), "pointer");
		break;
	default:
		q_snprintf (line, sizeof (line), "bad type %i", type);
		break;
	}

	return line;
}

/*
============
PR_GlobalString

Returns a string with a description and the contents of a global,
padded to 20 field width
============
*/
const char *PR_GlobalString (int ofs)
{
	static char		 line[512];
	static const int lastchari = countof (line) - 2;
	const char		*s;
	int				 i;
	ddef_t			*def;
	void			*val;

	val = (void *)&qcvm->globals[ofs];
	def = ED_GlobalAtOfs (ofs);
	if (!def)
		q_snprintf (line, sizeof (line), "%i(?)", ofs);
	else
	{
		s = PR_ValueString (def->type, (eval_t *)val);
		q_snprintf (line, sizeof (line), "%i(%s)%s", ofs, PR_GetString (def->s_name), s);
	}

	i = strlen (line);
	for (; i < 20; i++)
		strcat (line, " ");

	if (i < lastchari)
		strcat (line, " ");
	else
		line[lastchari] = ' ';

	return line;
}

const char *PR_GlobalStringNoContents (int ofs)
{
	static char		 line[512];
	static const int lastchari = countof (line) - 2;
	int				 i;
	ddef_t			*def;

	def = ED_GlobalAtOfs (ofs);
	if (!def)
		q_snprintf (line, sizeof (line), "%i(?)", ofs);
	else
		q_snprintf (line, sizeof (line), "%i(%s)", ofs, PR_GetString (def->s_name));

	i = strlen (line);
	for (; i < 20; i++)
		strcat (line, " ");

	if (i < lastchari)
		strcat (line, " ");
	else
		line[lastchari] = ' ';

	return line;
}

/*
=============
ED_IsRelevantField

Returns true if the field should be printed by the edict command:
- not a _x/_y_z variable
- non-zero contents
=============
*/
static qboolean ED_IsRelevantField (edict_t *ed, ddef_t *d)
{
	const char *name;
	size_t		l;
	int		   *v;
	int			type;
	int			i;

	name = PR_GetString (d->s_name);
	l = strlen (name);
	if (l > 1 && name[l - 2] == '_')
		return false; // skip _x, _y, _z vars

	type = d->type & ~DEF_SAVEGLOBAL;
	if (type >= NUM_TYPE_SIZES)
		return false;

	// if the value is still all 0, skip the field
	v = (int *)((char *)&ed->v + d->ofs * 4);
	for (i = 0; i < type_size[type]; i++)
		if (v[i])
			return true;

	return false;
}

/*
=============
ED_AppendFlagString
=============
*/
static void ED_AppendFlagString (char *dst, size_t dstsize, const char *desc)
{
	if (*dst)
		q_strlcat (dst, " | ", dstsize);
	q_strlcat (dst, desc, dstsize);
}

/*
=============
ED_FieldValueString
=============
*/
static const char *ED_FieldValueString (edict_t *ed, ddef_t *d)
{
	static char str[1024];
	int			ofs = d->ofs * 4;
	eval_t	   *val = (eval_t *)((char *)&ed->v + ofs);

	// .movetype
	if (ofs == offsetof (entvars_t, movetype) && val->_float == (int)val->_float)
	{
		switch ((int)val->_float)
		{
#define MOVETYPE_CASE(x) \
	case x:              \
		return #x
			MOVETYPE_CASE (MOVETYPE_NONE);
			MOVETYPE_CASE (MOVETYPE_ANGLENOCLIP);
			MOVETYPE_CASE (MOVETYPE_ANGLECLIP);
			MOVETYPE_CASE (MOVETYPE_WALK);
			MOVETYPE_CASE (MOVETYPE_STEP);
			MOVETYPE_CASE (MOVETYPE_FLY);
			MOVETYPE_CASE (MOVETYPE_TOSS);
			MOVETYPE_CASE (MOVETYPE_PUSH);
			MOVETYPE_CASE (MOVETYPE_NOCLIP);
			MOVETYPE_CASE (MOVETYPE_FLYMISSILE);
			MOVETYPE_CASE (MOVETYPE_BOUNCE);
			MOVETYPE_CASE (MOVETYPE_GIB);
#undef MOVETYPE_CASE
		default:
			break;
		}
	}

	// .solid
	if (ofs == offsetof (entvars_t, solid) && val->_float == (int)val->_float)
	{
		switch ((int)val->_float)
		{
#define SOLID_CASE(x) \
	case x:           \
		return #x
			SOLID_CASE (SOLID_NOT);
			SOLID_CASE (SOLID_TRIGGER);
			SOLID_CASE (SOLID_BBOX);
			SOLID_CASE (SOLID_SLIDEBOX);
			SOLID_CASE (SOLID_BSP);
#undef SOLID_CASE
		default:
			break;
		}
	}

	// .deadflag
	if (ofs == offsetof (entvars_t, deadflag) && val->_float == (int)val->_float)
	{
		switch ((int)val->_float)
		{
#define DEAD_CASE(x) \
	case x:          \
		return #x
			DEAD_CASE (DEAD_NO);
			DEAD_CASE (DEAD_DYING);
			DEAD_CASE (DEAD_DEAD);
			DEAD_CASE (DEAD_RESPAWNABLE);
#undef DEAD_CASE
		default:
			break;
		}
	}

	// .takedamage
	if (ofs == offsetof (entvars_t, takedamage) && val->_float == (int)val->_float)
	{
		switch ((int)val->_float)
		{
#define TAKEDAMAGE_CASE(x) \
	case x:                \
		return #x
			TAKEDAMAGE_CASE (DAMAGE_NO);
			TAKEDAMAGE_CASE (DAMAGE_YES);
			TAKEDAMAGE_CASE (DAMAGE_AIM);
#undef TAKEDAMAGE_CASE
		default:
			break;
		}
	}

	// bitfield: .flags, .spawnflags, .effects
	if ((ofs == offsetof (entvars_t, flags) || ofs == offsetof (entvars_t, spawnflags) || ofs == offsetof (entvars_t, effects)) &&
		val->_float == (int)val->_float)
	{
		int bits = (int)val->_float;
		str[0] = '\0';

#define BIT_CASE(f)                                      \
	do                                                   \
	{                                                    \
		if (bits & (int)f)                               \
		{                                                \
			bits ^= (int)f;                              \
			ED_AppendFlagString (str, sizeof (str), #f); \
		}                                                \
	} while (0)

		if (ofs == offsetof (entvars_t, flags))
		{
			BIT_CASE (FL_FLY);
			BIT_CASE (FL_CONVEYOR);
			BIT_CASE (FL_CLIENT);
			BIT_CASE (FL_INWATER);
			BIT_CASE (FL_MONSTER);
			BIT_CASE (FL_GODMODE);
			BIT_CASE (FL_NOTARGET);
			BIT_CASE (FL_ITEM);
			BIT_CASE (FL_ONGROUND);
			BIT_CASE (FL_PARTIALGROUND);
			BIT_CASE (FL_WATERJUMP);
			BIT_CASE (FL_JUMPRELEASED);
		}
		else if (ofs == offsetof (entvars_t, spawnflags))
		{
			BIT_CASE (SPAWNFLAG_NOT_EASY);
			BIT_CASE (SPAWNFLAG_NOT_MEDIUM);
			BIT_CASE (SPAWNFLAG_NOT_HARD);
			BIT_CASE (SPAWNFLAG_NOT_DEATHMATCH);
		}
		else if (ofs == offsetof (entvars_t, effects))
		{
			BIT_CASE (EF_BRIGHTFIELD);
			BIT_CASE (EF_MUZZLEFLASH);
			BIT_CASE (EF_BRIGHTLIGHT);
			BIT_CASE (EF_DIMLIGHT);
		}

#undef BIT_CASE

		while (bits)
		{
			int lowest = bits & -bits;
			bits ^= lowest;
			ED_AppendFlagString (str, sizeof (str), va ("%d", lowest));
		}

		return str;
	}

	// .nextthink
	if (ofs == offsetof (entvars_t, nextthink) && val->_float)
	{
		return va (" %7.1f (%+.2f)", val->_float, val->_float - qcvm->time);
	}

	// generic field
	return PR_ValueString (d->type, val);
}

/*
=============
ED_Print

For debugging
=============
*/
void ED_Print (edict_t *ed)
{
	ddef_t *d;
	int		i, l;
	char	field[4096], buf[4096], *p;

	if (ed->free)
	{
		Con_SafePrintf ("EDICT %5i: FREE, age: %5.1f\n", NUM_FOR_EDICT (ed), qcvm->time - ed->freetime);
		return;
	}

	q_snprintf (buf, sizeof (buf), "\nEDICT %5i:\n", NUM_FOR_EDICT (ed)); // johnfitz -- was Con_Printf
	p = buf + strlen (buf);
	for (i = 1; i < qcvm->progs->numfielddefs; i++)
	{
		d = &qcvm->fielddefs[i];
		if (!ED_IsRelevantField (ed, d))
			continue;

		q_snprintf (field, sizeof (field), "%-14s %s\n", PR_GetString (d->s_name), ED_FieldValueString (ed, d)); // johnfitz -- was Con_Printf
		l = strlen (field);
		if (l + 1 > buf + sizeof (buf) - p)
		{
			Con_SafePrintf ("%s", buf);
			p = buf;
		}

		memcpy (p, field, l + 1);
		p += l;
	}

	Con_SafePrintf ("%s\n", buf);
}

void ED_PrintNum (int ent)
{
	ED_Print (EDICT_NUM (ent));
}

/*
=============
ED_PrintEdicts

For debugging, prints all the entities in the current server
=============
*/
void ED_PrintEdicts (void)
{
	if (!sv.active)
		return;

	int free_edicts_count = 0;
	int free_list_count = 0;

	Q_UNUSED (free_edicts_count);
	Q_UNUSED (free_list_count);

	PR_SwitchQCVM (&sv.qcvm);

	ED_CheckFreeList ();

	// display the non-free ones first
	for (int i = 0; i < qcvm->num_edicts; i++)
	{
		if (EDICT_NUM (i)->free)
		{
			free_edicts_count++;
		}
		else
		{
			ED_PrintNum (i);
		}
	}

	Con_Printf ("\nFree-list:\n");

	size_t current_index = qcvm->free_list.head_index;

	for (size_t j = 0; j < qcvm->free_list.size; j++)
	{
		edict_t *e = EDICT_NUM (qcvm->free_list.circular_buffer[current_index]);

		ED_Print (e);
		free_list_count++;

		current_index = (current_index + 1) % MAX_EDICTS;
	}

	assert (free_list_count == free_edicts_count);

	Con_Printf ("Total: %i entities\n", qcvm->num_edicts);

	PR_SwitchQCVM (NULL);
}

/*
=============
ED_PrintEdict_f

For debugging, prints a single edicy
=============
*/
static void ED_PrintEdict_f (void)
{
	int i;

	if (!sv.active)
		return;

	i = atoi (Cmd_Argv (1));
	PR_SwitchQCVM (&sv.qcvm);

	ED_CheckFreeList ();

	if (i < 0 || i >= qcvm->num_edicts)
		Con_Printf ("Bad edict number\n");
	else
	{
		if (Cmd_Argc () == 2 || svs.maxclients != 1) // edict N
			ED_PrintNum (i);
		else // edict N FLD ...
		{
			ddef_t *def = ED_FindField (Cmd_Argv (2));
			if (!def)
				Con_Printf ("Field %s not defined\n", Cmd_Argv (2));
			else if (Cmd_Argc () < 4)
				Con_Printf (
					"Edict %u.%s==%s\n", i, PR_GetString (def->s_name),
					PR_UglyValueString (def->type & ~DEF_SAVEGLOBAL, (eval_t *)((char *)&EDICT_NUM (i)->v + def->ofs * 4)));
			else
				ED_ParseEpair ((void *)&EDICT_NUM (i)->v, def, Cmd_Argv (3), false);
		}
	}
	PR_SwitchQCVM (NULL);
}

/*
=============
ED_Count

For debugging
=============
*/
static void ED_Count (void)
{
	edict_t *ent;
	int		 i, active, models, solid, step, push, none, noclip, free_edicts;

	if (!sv.active)
		return;

	PR_SwitchQCVM (&sv.qcvm);

	ED_CheckFreeList ();

	active = models = solid = step = push = none = noclip = free_edicts = 0;
	for (i = 0; i < qcvm->num_edicts; i++)
	{
		ent = EDICT_NUM (i);
		if (ent->free)
		{
			free_edicts++;
			continue;
		}

		active++;
		if (ent->v.solid)
			solid++;
		if (ent->v.model)
			models++;

		if (ent->v.movetype == MOVETYPE_STEP)
			step++;
		if (ent->v.movetype == MOVETYPE_PUSH)
			push++;
		if (ent->v.movetype == MOVETYPE_NONE)
			none++;
		if (ent->v.movetype == MOVETYPE_NOCLIP)
			noclip++;
	}

	Con_Printf ("num_edicts : %5i\n", qcvm->num_edicts);
	Con_Printf ("active     : %5i\n", active);
	Con_Printf ("free       : %5i\n", free_edicts);
	Con_Printf ("view       : %5i\n", models);
	Con_Printf ("touch      : %5i\n", solid);
	Con_Printf ("------------------\n");
	Con_Printf ("move step  : %5i\n", step);
	Con_Printf ("move push  : %5i\n", push);
	Con_Printf ("move none  : %5i\n", none);
	Con_Printf ("move noclip: %5i\n", noclip);
	PR_SwitchQCVM (NULL);
}

/*
==============================================================================

ARCHIVING GLOBALS

FIXME: need to tag constants, doesn't really work
==============================================================================
*/

#if defined(USE_RUST_HOST) && defined(USE_RUST_PROGS)
/* Phase 7 M5: the ED_Parse* dispatchers flip in place (the PF_changeyaw
   precedent -- ED_ParseEdict has direct callers in host_cmd.c and pr_ext.c,
   not just a table slot), and only when both switches are on: their Rust
   cores live behind quake-capi's progs-host feature, which Meson sets exactly
   for use_rust_progs and use_rust_host together. The CI build-rs-cprogs leg
   is -Duse_rust_progs=disabled with host still enabled, so gating on
   USE_RUST_HOST alone would leave that leg with an unresolved
   quake_rs_ed_parse_globals.

   Status codes below are shared with
   rust/quake-capi/src/progs_edict_dispatch.rs (keep in sync). These are the
   dispatchers' own set -- not pr_cmds_glue.c's PRBI_ERR_* -- because the two
   functions raise messages no builtin does. */
#define PREDD_OK					0
#define PREDD_ERR_EOF				1
#define PREDD_ERR_CLOSE_NO_DATA		2
#define PREDD_ERR_EPAIR_PARSE		3
#define PREDD_ERR_ENTITY_RANGE		4
#define PREDD_ERR_BAD_EDICT_NUM		5
#define PREDD_ERR_FREELIST_FULL		6
#define PREDD_ERR_FREELIST_OVER_MAX 7
#define PREDD_ERR_GUARD				8

/* ADR-009: the raise lives in a pure C frame, above the Rust core that only
   ever returns a status.

   `brace` is the prefix for the two brace errors and `func` the prefix for the
   parse error; they differ only in ED_ParseGlobals, which reports its brace
   errors as "ED_ParseEntity". That is an upstream copy-paste preserved
   bug-for-bug -- the message is user-visible in Host_Error output and in
   demo/console captures. Codes 4-7 pass straight through from the Rust value
   parser and reuse pr_edict_parse_glue.c:104-111's wording verbatim, since in
   the C build they are raised by ED_ParseEpair itself. */
static void PREdictDispatch_Raise (int status, int detail, const char *brace, const char *func)
{
	switch (status)
	{
	case PREDD_ERR_EOF:
		Host_Error ("%s: EOF without closing brace", brace);
	case PREDD_ERR_CLOSE_NO_DATA:
		Host_Error ("%s: closing brace without data", brace);
	case PREDD_ERR_EPAIR_PARSE:
		Host_Error ("%s: parse error", func);
	case PREDD_ERR_ENTITY_RANGE:
		Host_Error ("ED_ParseEpair: ev_entity %d too large (max_edicts is %i)", detail, qcvm->max_edicts);
	case PREDD_ERR_BAD_EDICT_NUM:
		Host_Error ("EDICT_NUM: bad edict_num %i", detail);
	case PREDD_ERR_FREELIST_FULL:
		Host_Error ("ED_AddToFreeList : is full (qcvm 0x%p)", qcvm);
	case PREDD_ERR_FREELIST_OVER_MAX:
		Host_Error ("ED_AddToFreeList : has more than max_edicts >= %i (qcvm 0x%p)", detail, qcvm);
	case PREDD_ERR_GUARD:
		Host_Reraise (detail);
		return;
	default:
		Host_Error ("%s: unknown status %i", func, status);
	}
}
#endif

/*
=============
ED_ParseGlobals
=============
*/
#if defined(USE_RUST_HOST) && defined(USE_RUST_PROGS)
const char *ED_ParseGlobals (const char *data)
{
	const char *out = data;
	int			detail = 0;
	int			status = quake_rs_ed_parse_globals (data, &out, &detail);

	if (status != PREDD_OK)
		PREdictDispatch_Raise (status, detail, "ED_ParseEntity", "ED_ParseGlobals");
	return out;
}
#else
const char *ED_ParseGlobals (const char *data)
{
	char	keyname[64];
	ddef_t *key;

	while (1)
	{
		// parse key
		data = COM_Parse (data);
		if (com_token[0] == '}')
			break;
		if (!data)
			Host_Error ("ED_ParseEntity: EOF without closing brace");

		q_strlcpy (keyname, com_token, sizeof (keyname));

		// parse value
		data = COM_Parse (data);
		if (!data)
			Host_Error ("ED_ParseEntity: EOF without closing brace");

		if (com_token[0] == '}')
			Host_Error ("ED_ParseEntity: closing brace without data");

		key = ED_FindGlobal (keyname);
		if (!key)
		{
			Con_Printf ("'%s' is not a global\n", keyname);
			continue;
		}

		if (!ED_ParseEpair ((void *)qcvm->globals, key, com_token, false))
			Host_Error ("ED_ParseGlobals: parse error");
	}
	return data;
}
#endif

//============================================================================

/*
====================
ED_ParseEdict

Parses an edict out of the given string, returning the new position
ed should be a properly initialized empty edict.
Used for initial level load and for savegames.
====================
*/
#if defined(USE_RUST_HOST) && defined(USE_RUST_PROGS)
const char *ED_ParseEdict (const char *data, edict_t *ent)
{
	const char *out = data;
	int			detail = 0;
	/* NUM_FOR_EDICT_NO_CHECK, matching PRParse_Glue_UnlinkEdict: `ent` is
	   always a pointer the engine itself just produced, and the checked form
	   would add a raise the C original does not have here. */
	int			status = quake_rs_ed_parse_edict (data, NUM_FOR_EDICT_NO_CHECK (ent), &out, &detail);

	if (status != PREDD_OK)
		PREdictDispatch_Raise (status, detail, "ED_ParseEdict", "ED_ParseEdict");
	return out;
}
#else
const char *ED_ParseEdict (const char *data, edict_t *ent)
{
	ddef_t	*key;
	char	 keyname[256];
	qboolean anglehack, init;
	int		 n;

	init = false;

	// clear it
	if (ent != qcvm->edicts) // hack, this way never clear edict 0 = world
		memset (&ent->v, 0, qcvm->progs->entityfields * 4);

	// go through all the dictionary pairs
	while (1)
	{
		// parse key
		data = COM_Parse (data);
		if (com_token[0] == '}')
			break;
		if (!data)
			Host_Error ("ED_ParseEdict: EOF without closing brace");

		// anglehack is to allow QuakeEd to write single scalar angles
		// and allow them to be turned into vectors. (FIXME...)
		if (!strcmp (com_token, "angle"))
		{
			strcpy (com_token, "angles");
			anglehack = true;
		}
		else
			anglehack = false;

		// FIXME: change light to _light to get rid of this hack
		if (!strcmp (com_token, "light"))
			strcpy (com_token, "light_lev"); // hack for single light def

		q_strlcpy (keyname, com_token, sizeof (keyname));

		// another hack to fix keynames with trailing spaces
		n = strlen (keyname);
		while (n && keyname[n - 1] == ' ')
		{
			keyname[n - 1] = 0;
			n--;
		}

		// parse value
		// HACK: we allow truncation when reading the wad field,
		// otherwise maps using lots of wads with absolute paths
		// could cause a parse error
		data = COM_ParseEx (data, !strcmp (keyname, "wad") ? CPE_ALLOWTRUNC : CPE_NOTRUNC);
		if (!data)
			Host_Error ("ED_ParseEdict: EOF without closing brace");

		if (com_token[0] == '}')
			Host_Error ("ED_ParseEdict: closing brace without data");

		init = true;

		// keynames with a leading underscore are used for utility comments,
		// and are immediately discarded by quake, except for some specific keywords...
		if (keyname[0] == '_')
		{
			// spike -- hacks to support func_illusionary with all sorts of mdls, and various particle effects
			if (qcvm == &sv.qcvm)
			{
				if (!strcmp (keyname, "_precache_model") && sv.state == ss_loading)
					SV_Precache_Model (PR_GetString (ED_NewString (com_token)));
				else if (!strcmp (keyname, "_precache_sound") && sv.state == ss_loading)
					SV_Precache_Sound (PR_GetString (ED_NewString (com_token)));
			}
			// spike
			continue;
		}

		// johnfitz -- hack to support .alpha even when progs.dat doesn't know about it
		if (!strcmp (keyname, "alpha"))
			ent->alpha = ENTALPHA_ENCODE (atof (com_token));
		// johnfitz

		key = ED_FindField (keyname);
		if (!key)
		{
#ifdef PSET_SCRIPT
			eval_t *val;
			if (!strcmp (keyname, "traileffect") && qcvm == &sv.qcvm && sv.state == ss_loading)
			{
				if ((val = GetEdictFieldValue (ent, qcvm->extfields.traileffectnum)))
					val->_float = PF_SV_ForceParticlePrecache (com_token);
			}
			else if (!strcmp (keyname, "emiteffect") && qcvm == &sv.qcvm && sv.state == ss_loading)
			{
				if ((val = GetEdictFieldValue (ent, qcvm->extfields.emiteffectnum)))
					val->_float = PF_SV_ForceParticlePrecache (com_token);
			}
			// johnfitz -- HACK -- suppress error becuase fog/sky/alpha fields might not be mentioned in defs.qc
			else
#endif
				if (strncmp (keyname, "sky", 3) && strcmp (keyname, "fog") && strcmp (keyname, "alpha"))
				Con_DPrintf ("\"%s\" is not a field\n", keyname); // johnfitz -- was Con_Printf
			continue;
		}

		if (anglehack)
		{
			char temp[32];
			strcpy (temp, com_token);
			q_snprintf (com_token, sizeof (temp), "0 %s 0", temp);
		}

		if (!ED_ParseEpair ((void *)&ent->v, key, com_token, qcvm != &sv.qcvm))
			Host_Error ("ED_ParseEdict: parse error");
	}

	if (!init)
		ED_Free (ent);

	return data;
}
#endif

/*
================
ED_LoadFromFile
Creates a server's entity / program execution context by
parsing textual entity definitions out of an ent file.

Used for both fresh maps and savegame loads.  A fresh map would also need
to call ED_CallSpawnFunctions () to let the objects initialize themselves.
================
*/
void ED_LoadFromFile (const char *data)
{
	dfunction_t *func;
	edict_t		*ent = NULL;
	int			 inhibit = 0;
	int			 usingspawnfunc = 0;

	pr_global_struct->time = qcvm->time;

	// parse ents
	while (1)
	{
		// parse the opening brace
		data = COM_Parse (data);
		if (!data)
			break;
		if (com_token[0] != '{')
			Host_Error ("ED_LoadFromFile: found %s when expecting {", com_token);

		if (!ent)
			ent = EDICT_NUM (0);
		else
			ent = ED_Alloc ();
		data = ED_ParseEdict (data, ent);

		// remove things from different skill levels or deathmatch
		if (deathmatch.value)
		{
			if (((int)ent->v.spawnflags & SPAWNFLAG_NOT_DEATHMATCH))
			{
				ED_Free (ent);
				inhibit++;
				continue;
			}
		}
		else if (
			(current_skill == 0 && ((int)ent->v.spawnflags & SPAWNFLAG_NOT_EASY)) || (current_skill == 1 && ((int)ent->v.spawnflags & SPAWNFLAG_NOT_MEDIUM)) ||
			(current_skill >= 2 && ((int)ent->v.spawnflags & SPAWNFLAG_NOT_HARD)))
		{
			ED_Free (ent);
			inhibit++;
			continue;
		}

		//
		// immediately call spawn function
		//
		if (!ent->v.classname)
		{
			Con_SafePrintf ("No classname for:\n"); // johnfitz -- was Con_Printf
			ED_Print (ent);
			ED_Free (ent);
			continue;
		}

		const char *classname = PR_GetString (ent->v.classname);

		if (sv.nomonsters && !strncmp (classname, "monster_", 8))
		{
			ED_Free (ent);
			inhibit++;
			continue;
		}

		// look for the spawn function
		//
		func = ED_FindFunction (va ("spawnfunc_%s", classname));
		if (func)
		{
			if (!usingspawnfunc++)
				Con_DPrintf2 ("Using DP_SV_SPAWNFUNC_PREFIX\n");
		}
		else
			func = ED_FindFunction (classname);

		if (!func)
		{
			if (!strcmp (classname, "misc_model"))
				PR_spawnfunc_misc_model (ent);
			else
			{
				Con_SafePrintf ("No spawn function for:\n"); // johnfitz -- was Con_Printf
				ED_Print (ent);
				ED_Free (ent);
			}
			continue;
		}

		pr_global_struct->self = EDICT_TO_PROG (ent);
		PR_ExecuteProgram (func - qcvm->functions);
	}

	Con_DPrintf ("%i entities inhibited\n", inhibit);
}

/*
===============
ED_Nomonsters_f
===============
*/
static void ED_Nomonsters_f (cvar_t *cvar)
{
	if (cvar->value)
		Con_Warning ("\"%s\" can break gameplay.\n", cvar->name);
}

/*
===============
PR_Init
===============
*/
void PR_Init (void)
{
	Cmd_AddCommand ("edict", ED_PrintEdict_f);
	Cmd_AddCommand ("edicts", ED_PrintEdicts);
	Cmd_AddCommand ("edictcount", ED_Count);
	Cmd_AddCommand ("profile", PR_Profile_f);
	Cmd_AddCommand ("pr_dumpplatform", PR_DumpPlatform_f);
	Cmd_AddCommand ("pr_dumpbuiltins", PR_DumpBuiltinTable_f);
	Cvar_RegisterVariable (&nomonsters);
	Cvar_SetCallback (&nomonsters, ED_Nomonsters_f);
	Cvar_RegisterVariable (&gamecfg);
	Cvar_RegisterVariable (&scratch1);
	Cvar_RegisterVariable (&scratch2);
	Cvar_RegisterVariable (&scratch3);
	Cvar_RegisterVariable (&scratch4);
	Cvar_RegisterVariable (&savedgamecfg);
	Cvar_RegisterVariable (&saved1);
	Cvar_RegisterVariable (&saved2);
	Cvar_RegisterVariable (&saved3);
	Cvar_RegisterVariable (&saved4);

	PR_InitExtensions ();
}

edict_t *EDICT_NUM (int n)
{
	if (n < 0 || n >= qcvm->max_edicts)
		Host_Error ("EDICT_NUM: bad edict_num %i", n);

	edict_t *found_edict = EDICT_NUM_NO_CHECK (n);

#if defined(DEBUG) || defined(_DEBUG)
	if (found_edict->edict_num != n)
		Host_Error ("EDICT_NUM(%i): inconsistent number vs. edict_num=%i", n, (int)found_edict->edict_num);

	if (found_edict->edict_ptr != found_edict)
		Host_Error ("EDICT_NUM(%i) inconsistent pointer", n);
#endif
	return found_edict;
}

int NUM_FOR_EDICT (edict_t *e)
{
#if defined(DEBUG) || defined(_DEBUG)
	if (e->qcvm_owner != qcvm)
		Host_Error ("NUM_FOR_EDICT inconsistent qcvm 0x%p, expected 0x%p", qcvm, e->qcvm_owner);

	if (e->edict_ptr != e)
		Host_Error ("NUM_FOR_EDICT inconsistent pointer");
#endif

	int b;

	b = (byte *)e - (byte *)qcvm->edicts;
	b = b / qcvm->edict_size;

	if (b < 0 || b >= qcvm->num_edicts)
		Host_Error ("NUM_FOR_EDICT: bad pointer");

#if defined(DEBUG) || defined(_DEBUG)
	if (e->edict_num != b)
		Host_Error ("NUM_FOR_EDICT: inconsistent number %i vs. e.edict_num %i", b, (int)e->edict_num);
#endif

	return b;
}

#if defined(DEBUG) || defined(_DEBUG)
edict_t *NEXT_EDICT (edict_t *e)
{
	int current_num = NUM_FOR_EDICT (e);
	EDICT_NUM (current_num);

	// the usage pattern is such that NEXT_EDICT can go beyond the last element
	// in for loops but this is normally fine because the returned value is not used.
	// here test for last element and return a NULL edict_t* if we go beyond the last element,
	// and we coredump if that element get used. This NULL checks is only for
	// Debug builds because it has a noticable performance impact on big edict-heavy levels.
	if (current_num == qcvm->num_edicts - 1)
		return NULL;

	edict_t *next_edict = (edict_t *)((byte *)e + qcvm->edict_size);
	int		 next_num = NUM_FOR_EDICT (next_edict);
	EDICT_NUM (next_num);
	if (next_num != current_num + 1)
		Host_Error ("NEXT_EDICT: inconsistent next edict %i (expected %i)", next_num, current_num + 1);

	return next_edict;
}

int EDICT_TO_PROG (edict_t *e)
{
	if (e->qcvm_owner != qcvm)
		Host_Error ("EDICT_TO_PROG inconsistent qcvm 0x%p, expected 0x%p", qcvm, e->qcvm_owner);

	if (e->edict_ptr != e)
		Host_Error ("EDICT_TO_PROG inconsistent pointer");

	int edict_num = NUM_FOR_EDICT (e);
	int found_prog = (int)((byte *)e - (byte *)qcvm->edicts);

	// It seems invalid to cast a edict to prog if it's free, because it is intended to be active.
	if (e->free)
		Host_Error ("EDICT_TO_PROG: edict %i is free (qcvm 0x%p)", edict_num, qcvm);

	return found_prog;
}

edict_t *PROG_TO_EDICT (int p)
{
	edict_t *found_edict = (edict_t *)((byte *)qcvm->edicts + p);
	NUM_FOR_EDICT (found_edict);

	return found_edict;
}

#endif
//===========================================================================
