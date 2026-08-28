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
// pr_edict_load.c -- the progs loader, split verbatim out of pr_edict.c
// (Rust migration Phase 6 M6, behaviour-neutral).
//
// PR_LoadProgs and its helpers decide everything downstream compatibility
// depends on: the CRC/hash taken before the in-place byteswap, the reverse
// hash-map build order that preserves linear-search first-match, the engine
// fielddef merge that fixes entityfields -> edict_size -> savegame layout,
// and the re-release builtin patching that runs after PR_EnableExtensions.
//
// qcvm and pr_global_struct are defined here because PR_SwitchQCVM is; under
// -Duse_rust_progs pr_edict_load_glue.c owns that storage instead.

#include "quakedef.h"

#ifndef PR_SwitchQCVM
qcvm_t		 *qcvm;
globalvars_t *pr_global_struct;
void		  PR_SwitchQCVM (qcvm_t *nvm)
{
	if (qcvm && nvm)
		Sys_Error ("PR_SwitchQCVM: A qcvm was already active");
	qcvm = nvm;
	if (qcvm)
		pr_global_struct = (globalvars_t *)qcvm->globals;
	else
		pr_global_struct = NULL;
}
#endif

void PR_ClearProgs (qcvm_t *vm)
{
	qcvm_t *oldvm = qcvm;
	if (!vm->progs)
		return; // wasn't loaded.
	qcvm = NULL;
	PR_SwitchQCVM (vm);
	PR_ShutdownExtensions ();

	if (qcvm->knownstrings)
	{
		for (int i = 0; i < qcvm->numknownstrings; ++i)
			if (qcvm->knownstringsowned[i])
				Mem_Free (qcvm->knownstrings[i]);
		Mem_Free ((void *)qcvm->knownstrings);
		Mem_Free (qcvm->knownstringsowned);
	}
	Mem_Free (qcvm->edicts); // ericw -- sv.edicts switched to use malloc()
	if (qcvm->fielddefs != (ddef_t *)((byte *)qcvm->progs + qcvm->progs->ofs_fielddefs))
		Mem_Free (qcvm->fielddefs);
	Mem_Free (qcvm->progs); // spike -- pr_progs switched to use malloc (so menuqc doesn't end up stuck on the early hunk nor wiped on every map change)
	HashMap_Destroy (qcvm->function_map);
	HashMap_Destroy (qcvm->fielddefs_map);
	HashMap_Destroy (qcvm->globaldefs_map);
	memset (qcvm, 0, sizeof (*qcvm));

	qcvm = NULL;
	PR_SwitchQCVM (oldvm);
}

// extension fields:
struct
{
	const char *fname;
	etype_t		type;
	int			newidx;
} extrafields[] = {
	// table of engine fields to add. we'll be using ED_FindFieldOffset for these later.
	// this is useful for fields that should be defined for mappers which are not defined by the mod.
	// future note: mutators will need to edit the mutator's globaldefs table too. remember to handle vectors and their 3 globals too.
	{"alpha", ev_float},		  // just because we can (though its already handled in a weird hacky way)
	{"scale", ev_float},		  // hurrah for being able to rescale entities.
	{"emiteffectnum", ev_float},  // constantly emitting particles, even without moving.
	{"traileffectnum", ev_float}, // custom effect for trails
								  //{"glow_size",		ev_float},	//deprecated particle trail rubbish
								  //{"glow_color",	ev_float},	//deprecated particle trail rubbish
	{"tag_entity", ev_float},	  // for setattachment to not bug out when omitted.
	{"tag_index", ev_float},	  // for setattachment to not bug out when omitted.
	{"modelflags", ev_float},	  // deprecated rubbish to fill the high 8 bits of effects.
								  //{"vw_index",		ev_float},	//modelindex2
								  //{"pflags",		ev_float},	//for rtlights
								  //{"drawflags",		ev_float},	//hexen2 compat
								  //{"abslight",		ev_float},	//hexen2 compat
	{"colormod", ev_vector},	  // lighting tints
								  //{"glowmod",		ev_vector},	//fullbright tints
								  //{"fatness",		ev_float},	//bloated rendering...
								  //{"gravitydir",	ev_vector},	//says which direction gravity should act for this ent...

};

// makes sure extension fields are actually registered so they can be used for mappers without qc changes. eg so scale can be used.
static void PR_MergeEngineFieldDefs (void)
{
	int			 maxofs = qcvm->progs->entityfields;
	int			 maxdefs = qcvm->progs->numfielddefs;
	unsigned int j, a;

	// figure out where stuff goes
	for (j = 0; j < countof (extrafields); j++)
	{
		extrafields[j].newidx = ED_FindFieldOffset (extrafields[j].fname);
		if (extrafields[j].newidx < 0)
		{
			extrafields[j].newidx = maxofs;
			maxdefs++;
			if (extrafields[j].type == ev_vector)
				maxdefs += 3;
			maxofs += type_size[extrafields[j].type];
		}
	}

	if (maxdefs != qcvm->progs->numfielddefs)
	{ // we now know how many entries we need to add...
		ddef_t *olddefs = qcvm->fielddefs;
		qcvm->fielddefs = Mem_Alloc (maxdefs * sizeof (*qcvm->fielddefs));
		memcpy (qcvm->fielddefs, olddefs, qcvm->progs->numfielddefs * sizeof (*qcvm->fielddefs));
		if (olddefs != (ddef_t *)((byte *)qcvm->progs + qcvm->progs->ofs_fielddefs))
			Mem_Free (olddefs);

		// allocate the extra defs
		for (j = 0; j < countof (extrafields); j++)
		{
			if (extrafields[j].newidx >= qcvm->progs->entityfields && extrafields[j].newidx < maxofs)
			{ // looks like its new. make sure ED_FindField can find it.
				qcvm->fielddefs[qcvm->progs->numfielddefs].ofs = extrafields[j].newidx;
				qcvm->fielddefs[qcvm->progs->numfielddefs].type = extrafields[j].type;
				qcvm->fielddefs[qcvm->progs->numfielddefs].s_name = ED_NewString (extrafields[j].fname);
				const ddef_t *def_ptr = &qcvm->fielddefs[qcvm->progs->numfielddefs];
				HashMap_Insert (qcvm->fielddefs_map, &extrafields[j].fname, &def_ptr);
				qcvm->progs->numfielddefs++;

				if (extrafields[j].type == ev_vector)
				{ // vectors are weird and annoying.
					for (a = 0; a < 3; a++)
					{
						qcvm->fielddefs[qcvm->progs->numfielddefs].ofs = extrafields[j].newidx + a;
						qcvm->fielddefs[qcvm->progs->numfielddefs].type = ev_float | DEF_SAVEGLOBAL;
						const char *fielddef_name = va ("%s_%c", extrafields[j].fname, 'x' + a);
						qcvm->fielddefs[qcvm->progs->numfielddefs].s_name = ED_NewString (fielddef_name);
						const ddef_t *def_ptr_v = &qcvm->fielddefs[qcvm->progs->numfielddefs];
						HashMap_Insert (qcvm->fielddefs_map, &fielddef_name, &def_ptr_v);
						qcvm->progs->numfielddefs++;
					}
				}
			}
		}
		qcvm->progs->entityfields = maxofs;
	}
}

/*
===============
PR_HasGlobal
===============
*/
static qboolean PR_HasGlobal (const char *name, float value)
{
	ddef_t *g = ED_FindGlobal (name);
	return g && (g->type & ~DEF_SAVEGLOBAL) == ev_float && G_FLOAT (g->ofs) == value;
}

/*
===============
PR_FindSupportedEffects

Disables Quake 2021 release effects flags when not present in progs.dat to avoid conflicts
(e.g. Arcane Dimensions uses bit 32 for its explosions, same as EF_QEX_PENTALIGHT)
===============
*/
static void PR_FindSupportedEffects (void)
{
	if (qcvm == &sv.qcvm)
	{
		qboolean isqex = PR_HasGlobal ("EF_QUADLIGHT", EF_QEX_QUADLIGHT) &&
						 (PR_HasGlobal ("EF_PENTLIGHT", EF_QEX_PENTALIGHT) || PR_HasGlobal ("EF_PENTALIGHT", EF_QEX_PENTALIGHT));
		sv.effectsmask = isqex ? -1 : -1 & ~(EF_QEX_QUADLIGHT | EF_QEX_PENTALIGHT | EF_QEX_CANDLELIGHT);
	}
}

/* for 2021 re-release */
typedef struct
{
	const char *name;
	int			first_statement;
	int			patch_statement;
} exbuiltin_t;

/*
===============
PR_PatchRereleaseBuiltins

for 2021 re-release
===============
*/
static const exbuiltin_t exbuiltins[] = {
	/* Update-1 adds the following builtins with new ids. Patch them to use old indices.
	 * (https://steamcommunity.com/games/2310/announcements/detail/2943653788150871156) */
	{"centerprint", -90, -73},
	{"bprint", -91, -23},
	{"sprint", -92, -24},
	{NULL, 0, 0} /* end-of-list. */
};

static void PR_PatchRereleaseBuiltins (void)
{
	const exbuiltin_t *ex = exbuiltins;
	dfunction_t		  *f;

	for (; ex->name != NULL; ++ex)
	{
		f = ED_FindFunction (ex->name);
		if (f && f->first_statement == ex->first_statement)
			f->first_statement = ex->patch_statement;
	}
}

/*
===============
PR_LoadProgs
===============
*/
qboolean PR_LoadProgs (const char *filename, qboolean fatal, unsigned int needcrc, const builtin_t *builtins, size_t numbuiltins)
{
	int i;

	PR_ClearProgs (qcvm); // just in case.

	qcvm->progs = (dprograms_t *)COM_LoadFile (filename, NULL);
	if (!qcvm->progs)
		return false;

	qcvm->progssize = com_filesize;
	CRC_Init (&qcvm->progscrc);
	for (i = 0; i < com_filesize; i++)
		CRC_ProcessByte (&qcvm->progscrc, ((byte *)qcvm->progs)[i]);
	qcvm->progshash = Com_BlockChecksum (qcvm->progs, com_filesize);

	// byte swap the header
	for (i = 0; i < (int)sizeof (*qcvm->progs) / 4; i++)
		((int *)qcvm->progs)[i] = LittleLong (((int *)qcvm->progs)[i]);

	if (qcvm->progs->version != PROG_VERSION)
	{
		if (fatal)
			Host_Error ("%s has wrong version number (%i should be %i)", filename, qcvm->progs->version, PROG_VERSION);
		else
		{
			Con_Printf ("%s ABI set not supported\n", filename);
			qcvm->progs = NULL;
			return false;
		}
	}
	if (qcvm->progs->crc != needcrc)
	{
		if (fatal)
			Host_Error ("%s system vars have been modified, progdefs.h is out of date", filename);
		else
		{
			switch (qcvm->progs->crc)
			{
			case 22390: // full csqc
				Con_Printf ("%s - full csqc is not supported\n", filename);
				break;
			case 52195: // dp csqc
				Con_Printf ("%s - obsolete csqc is not supported\n", filename);
				break;
			case 54730: // quakeworld
				Con_Printf ("%s - quakeworld gamecode is not supported\n", filename);
				break;
			case 26940: // prerelease
				Con_Printf ("%s - prerelease gamecode is not supported\n", filename);
				break;
			case 32401: // tenebrae
				Con_Printf ("%s - tenebrae gamecode is not supported\n", filename);
				break;
			case 38488: // hexen2 release
			case 26905: // hexen2 mission pack
			case 14046: // hexen2 demo
				Con_Printf ("%s - hexen2 gamecode is not supported\n", filename);
				break;
			// case 5927: //nq PROGHEADER_CRC as above. shouldn't happen, obviously.
			default:
				Con_Printf ("%s system vars are not supported\n", filename);
				break;
			}
			qcvm->progs = NULL;
			return false;
		}
	}
	Con_DPrintf ("%s occupies %uK.\n", filename, (unsigned)(com_filesize / 1024u));

	qcvm->functions = (dfunction_t *)((byte *)qcvm->progs + qcvm->progs->ofs_functions);
	qcvm->strings = (char *)qcvm->progs + qcvm->progs->ofs_strings;
	if (qcvm->progs->ofs_strings + qcvm->progs->numstrings >= com_filesize)
		Host_Error ("%s strings go past end of file\n", filename);

	qcvm->globaldefs = (ddef_t *)((byte *)qcvm->progs + qcvm->progs->ofs_globaldefs);
	qcvm->fielddefs = (ddef_t *)((byte *)qcvm->progs + qcvm->progs->ofs_fielddefs);
	qcvm->statements = (dstatement_t *)((byte *)qcvm->progs + qcvm->progs->ofs_statements);

	qcvm->globals = (float *)((byte *)qcvm->progs + qcvm->progs->ofs_globals);
	pr_global_struct = (globalvars_t *)qcvm->globals;

	qcvm->stringssize = qcvm->progs->numstrings;

	// byte swap the lumps
	for (i = 0; i < qcvm->progs->numstatements; i++)
	{
		qcvm->statements[i].op = LittleShort (qcvm->statements[i].op);
		qcvm->statements[i].a = LittleShort (qcvm->statements[i].a);
		qcvm->statements[i].b = LittleShort (qcvm->statements[i].b);
		qcvm->statements[i].c = LittleShort (qcvm->statements[i].c);
	}

	for (i = 0; i < qcvm->progs->numfunctions; i++)
	{
		qcvm->functions[i].first_statement = LittleLong (qcvm->functions[i].first_statement);
		qcvm->functions[i].parm_start = LittleLong (qcvm->functions[i].parm_start);
		qcvm->functions[i].s_name = LittleLong (qcvm->functions[i].s_name);
		qcvm->functions[i].s_file = LittleLong (qcvm->functions[i].s_file);
		qcvm->functions[i].numparms = LittleLong (qcvm->functions[i].numparms);
		qcvm->functions[i].locals = LittleLong (qcvm->functions[i].locals);
	}
	// Just to be sure: Reverse insert because there can be duplicates and we want
	// to match linear search with hash lookup (find first)
	qcvm->function_map = HashMap_Create (const char *, dfunction_t *, &HashStr, &HashStrCmp);
	HashMap_Reserve (qcvm->function_map, qcvm->progs->numfunctions);
	for (i = qcvm->progs->numfunctions - 1; i >= 0; --i)
	{
		const char		  *func_name = PR_GetString (qcvm->functions[i].s_name);
		const dfunction_t *func_ptr = &qcvm->functions[i];
		HashMap_Insert (qcvm->function_map, &func_name, &func_ptr);
	}

	for (i = 0; i < qcvm->progs->numglobaldefs; i++)
	{
		qcvm->globaldefs[i].type = LittleShort (qcvm->globaldefs[i].type);
		qcvm->globaldefs[i].ofs = LittleShort (qcvm->globaldefs[i].ofs);
		qcvm->globaldefs[i].s_name = LittleLong (qcvm->globaldefs[i].s_name);
	}
	qcvm->globaldefs_map = HashMap_Create (const char *, ddef_t *, &HashStr, &HashStrCmp);
	HashMap_Reserve (qcvm->globaldefs_map, qcvm->progs->numglobaldefs);
	for (i = qcvm->progs->numglobaldefs - 1; i >= 0; --i)
	{
		const char	 *globaldef_name = PR_GetString (qcvm->globaldefs[i].s_name);
		const ddef_t *def_ptr = &qcvm->globaldefs[i];
		HashMap_Insert (qcvm->globaldefs_map, &globaldef_name, &def_ptr);
	}

	for (i = 0; i < qcvm->progs->numfielddefs; i++)
	{
		qcvm->fielddefs[i].type = LittleShort (qcvm->fielddefs[i].type);
		if (qcvm->fielddefs[i].type & DEF_SAVEGLOBAL)
			Host_Error ("PR_LoadProgs: pr_fielddefs[i].type & DEF_SAVEGLOBAL");
		qcvm->fielddefs[i].ofs = LittleShort (qcvm->fielddefs[i].ofs);
		qcvm->fielddefs[i].s_name = LittleLong (qcvm->fielddefs[i].s_name);
	}
	qcvm->fielddefs_map = HashMap_Create (const char *, ddef_t *, &HashStr, &HashStrCmp);
	HashMap_Reserve (
		qcvm->fielddefs_map, qcvm->progs->numfielddefs + countof (extrafields) * 3); // assume size of vectors for all engine autofields, for margin.
	for (i = qcvm->progs->numfielddefs - 1; i >= 0; --i)
	{
		const char	 *fielddef_name = PR_GetString (qcvm->fielddefs[i].s_name);
		const ddef_t *def_ptr = &qcvm->fielddefs[i];
		HashMap_Insert (qcvm->fielddefs_map, &fielddef_name, &def_ptr);
	}

	for (i = 0; i < qcvm->progs->numglobals; i++)
		((int *)qcvm->globals)[i] = LittleLong (((int *)qcvm->globals)[i]);

	memcpy (qcvm->builtins, builtins, numbuiltins * sizeof (qcvm->builtins[0]));
	qcvm->numbuiltins = numbuiltins;

	// spike: detect extended fields from progs
	PR_MergeEngineFieldDefs ();
#define QCEXTFIELD(n, t) qcvm->extfields.n = ED_FindFieldOffset (#n);
	QCEXTFIELDS_ALL
	QCEXTFIELDS_GAME
	QCEXTFIELDS_SS
#undef QCEXTFIELD

	qcvm->edict_size = qcvm->progs->entityfields * 4 + sizeof (edict_t) - sizeof (entvars_t);
	// round off to next highest whole word address (esp for Alpha)
	// this ensures that pointers in the engine data area are always
	// properly aligned
	qcvm->edict_size += sizeof (void *) - 1;
	qcvm->edict_size &= ~(sizeof (void *) - 1);

	PR_SetEngineString ("");
	PR_EnableExtensions (qcvm->globaldefs);
	PR_PatchRereleaseBuiltins ();
	PR_FindSupportedEffects ();

	qcvm->progsstrings = qcvm->numknownstrings;
	return true;
}
