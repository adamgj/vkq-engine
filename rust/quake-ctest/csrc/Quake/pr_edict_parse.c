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
// pr_edict_parse.c -- the savegame/entity-text value parser, split verbatim
// out of pr_edict.c (Rust migration Phase 6 M5, behaviour-neutral).
//
// ED_ParseEpair is where the read side's compatibility risk lives: every
// numeric conversion is a platform libc call whose rounding the savegame
// byte-diff gate depends on. The *key* dispatchers -- ED_ParseEdict and
// ED_ParseGlobals -- deliberately stay in pr_edict.c: their COM_Parse loops,
// _precache_model/_precache_sound hacks, PSET_SCRIPT traileffect branches and
// sv.state tests are server code Phase 7 owns.
//
// ED_NewString is exported rather than static because ED_ParseEdict and
// PR_MergeEngineFieldDefs both still call it.

#include "quakedef.h"

/*
=============
ED_NewString
=============
*/
string_t ED_NewString (const char *string)
{
	char	*new_p;
	int		 i, l;
	string_t num;

	l = strlen (string) + 1;
	num = PR_AllocString (l, &new_p);

	for (i = 0; i < l; i++)
	{
		if (string[i] == '\\' && i < l - 1)
		{
			i++;
			if (string[i] == 'n')
				*new_p++ = '\n';
			else
				*new_p++ = '\\';
		}
		else
			*new_p++ = string[i];
	}

	return num;
}

static void ED_RezoneString (string_t *ref, const char *str)
{
	char  *buf;
	size_t len = strlen (str) + 1;
	size_t id;

	if (*ref)
	{ // if the reference is already a zoned string then free it first.
		id = -1 - *ref;
		if (id < qcvm->knownzonesize && (qcvm->knownzone[id >> 3] & (1u << (id & 7))))
		{ // okay, it was zoned.
			qcvm->knownzone[id >> 3] &= ~(1u << (id & 7));
			buf = (char *)PR_GetString (*ref);
			PR_ClearEngineString (*ref);
			Mem_Free (buf);
		}
		//		else
		//			Con_Warning("ED_RezoneString: string wasn't strzoned\n");	//warnings would trigger from the default cvar value that autocvars are
		// initialised with
	}

	buf = Mem_Alloc (len);
	memcpy (buf, str, len);
	id = -1 - (*ref = PR_SetEngineString (buf));
	// make sure its flagged as zoned so we can clean up properly after.
	if (id >= qcvm->knownzonesize)
	{
		int old_size = (qcvm->knownzonesize + 7) >> 3;
		qcvm->knownzonesize = (id + 32) & ~7;
		int new_size = (qcvm->knownzonesize + 7) >> 3;
		qcvm->knownzone = Mem_Realloc (qcvm->knownzone, new_size);
		memset (qcvm->knownzone + old_size, 0, new_size - old_size);
	}
	qcvm->knownzone[id >> 3] |= 1u << (id & 7);
}

/*
=============
ED_ParseEval

Can parse either fields or globals
returns false if error
=============
*/
qboolean ED_ParseEpair (void *base, ddef_t *key, const char *s, qboolean zoned)
{
	int			 i;
	char		 string[128];
	ddef_t		*def;
	char		*v, *w;
	char		*end;
	void		*d;
	dfunction_t *func;

	d = (void *)((int *)base + key->ofs);

	switch (key->type & ~DEF_SAVEGLOBAL)
	{
	case ev_string:
		if (zoned) // zoned version allows us to change the strings more freely
			ED_RezoneString ((string_t *)d, s);
		else
			*(string_t *)d = ED_NewString (s);
		break;

	case ev_float:
		*(float *)d = atof (s);
		break;
	case ev_ext_double:
		*(qcdouble_t *)d = atof (s);
		break;
	case ev_ext_integer:
		*(int32_t *)d = atoi (s);
		break;
	case ev_ext_uint32:
		*(uint32_t *)d = atoi (s);
		break;
	case ev_ext_sint64:
		*(qcsint64_t *)d = strtoll (s, NULL, 0); // if longlong is 128bit then no real harm done for 64bit quantities...
		break;
	case ev_ext_uint64:
		*(qcuint64_t *)d = strtoull (s, NULL, 0);
		break;

	case ev_vector:
		q_strlcpy (string, s, sizeof (string));
		end = (char *)string + strlen (string);
		v = string;
		w = string;

		for (i = 0; i < 3 && (w <= end); i++) // ericw -- added (w <= end) check
		{
			// set v to the next space (or 0 byte), and change that char to a 0 byte
			while (*v && *v != ' ')
				v++;
			*v = 0;
			((float *)d)[i] = atof (w);
			w = v = v + 1;
		}
		// ericw -- fill remaining elements to 0 in case we hit the end of string
		// before reading 3 floats.
		if (i < 3)
		{
			Con_DWarning ("Avoided reading garbage for \"%s\" \"%s\"\n", PR_GetString (key->s_name), s);
			for (; i < 3; i++)
				((float *)d)[i] = 0.0f;
		}
		break;

	case ev_entity:
	{
		if (!strncmp (s, "entity ", 7)) // Spike: putentityfieldstring/etc should be able to cope with etos's weirdness.
			s += 7;
		const int loaded_ent_num = atoi (s);

		if (loaded_ent_num >= qcvm->max_edicts)
			Host_Error ("ED_ParseEpair: ev_entity %d too large (max_edicts is %i)", loaded_ent_num, qcvm->max_edicts);

		// loaded_ent_num can be beyond qcvm->num_edicts at loading, take care of adjusting
		// preperly.
		const int previous_num_edicts = qcvm->num_edicts;

		// adjust first we need it for consistenecy checks in EDICT_NUM / ED_Free..etc.
		qcvm->num_edicts = q_max (previous_num_edicts, loaded_ent_num + 1);

		// properly initialize the free edicts in previous_num_edicts..loaded_ent_num - 1 range:
		for (int j = previous_num_edicts; j < loaded_ent_num; j++)
		{
			edict_t *new_edict = EDICT_NUM (j);

			// proceed to the same init as new edicts in ED_Alloc: wipe all out, then deallocate it
			// right away
			memset (new_edict, 0, qcvm->edict_size);
#if defined(DEBUG) || defined(_DEBUG)
			// fill debug fields, they were overwriten above:
			new_edict->qcvm_owner = qcvm;
			new_edict->edict_ptr = new_edict;
			new_edict->edict_num = j;
#endif
			assert (!new_edict->free);

			ED_Free (new_edict);
		}

		edict_t *found_edict = EDICT_NUM (loaded_ent_num);

		// mark loaded_ent_num as allocated :
		if (found_edict->free)
		{
			ED_RemoveFromFreeList (found_edict);
			found_edict->free = false;
		}

		*(int *)d = EDICT_TO_PROG (found_edict);
	}
	break;

	case ev_field:
		def = ED_FindField (s);
		if (!def)
		{
			// johnfitz -- HACK -- suppress error becuase fog/sky fields might not be mentioned in defs.qc
			if (strncmp (s, "sky", 3) && strcmp (s, "fog"))
				Con_DPrintf ("Can't find field %s\n", s);
			return false;
		}
		*(int *)d = G_INT (def->ofs);
		break;

	case ev_function:
		func = ED_FindFunction (s);
		if (!func)
		{
			Con_Printf ("Can't find function %s\n", s);
			return false;
		}
		*(func_t *)d = func - qcvm->functions;
		break;

	default:
		break;
	}
	return true;
}
