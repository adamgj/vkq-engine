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
// pr_exec_glue.c -- the C frame around the Rust progs interpreter.
//
// Compiled instead of pr_exec.c under -Duse_rust_progs (Rust migration
// Phase 6 M3). Three jobs:
//
//  1. Keep the diagnostics in C. PR_PrintStatement/PR_StackTrace/PR_Profile_f
//     and the pr_opnames table are console code with no compatibility risk,
//     and PR_RunError/PR_RunWarning stay exported here because pr_cmds.c and
//     pr_ext.c call them.
//  2. Raise from a C frame (ADR-009). quake_rs_pr_execute_program returns a
//     status; this file turns it back into the exact PR_RunError/Host_Error
//     the C interpreter would have produced.
//  3. Guard the builtin dispatch. A C builtin can Host_Error, and that
//     longjmp must not cross the Rust interpreter frame, so the call goes
//     through Host_Guard and the caught jump is re-issued from here once Rust
//     has returned normally.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

/* status codes shared with rust/quake-capi/src/progs_exec.rs (keep in sync) */
#define PREXEC_OK					0
#define PREXEC_ERR_STACK_OVERFLOW	1
#define PREXEC_ERR_LOCALS_OVERFLOW	2
#define PREXEC_ERR_LOCALS_UNDERFLOW 3
#define PREXEC_ERR_RUNAWAY			4
#define PREXEC_ERR_NULL_FUNCTION	5
#define PREXEC_ERR_WORLD_ASSIGN		6
#define PREXEC_ERR_BAD_OPCODE		7
#define PREXEC_ERR_FIELD_RANGE		8
#define PREXEC_ERR_BAD_FUNC_INDEX	9
#define PREXEC_ERR_NO_STRING		10
#define PREXEC_ERR_STACK_UNDERFLOW	11
#define PREXEC_GUARD_ABORTSERVER	12
#define PREXEC_GUARD_SCREEN_ERROR	13

// clang-format off
static const char *const pr_opnames[] = {"DONE",

										 "MUL_F",	 "MUL_V",	 "MUL_FV",	 "MUL_VF",

										 "DIV",

										 "ADD_F",	 "ADD_V",

										 "SUB_F",	 "SUB_V",

										 "EQ_F",	 "EQ_V",	 "EQ_S",	 "EQ_E",	   "EQ_FNC",

										 "NE_F",	 "NE_V",	 "NE_S",	 "NE_E",	   "NE_FNC",

										 "LE",		 "GE",		 "LT",		 "GT",

										 "INDIRECT", "INDIRECT", "INDIRECT", "INDIRECT",   "INDIRECT",	 "INDIRECT",

										 "ADDRESS",

										 "STORE_F",	 "STORE_V",	 "STORE_S",	 "STORE_ENT",  "STORE_FLD",	 "STORE_FNC",

										 "STOREP_F", "STOREP_V", "STOREP_S", "STOREP_ENT", "STOREP_FLD", "STOREP_FNC",

										 "RETURN",

										 "NOT_F",	 "NOT_V",	 "NOT_S",	 "NOT_ENT",	   "NOT_FNC",

										 "IF",		 "IFNOT",

										 "CALL0",	 "CALL1",	 "CALL2",	 "CALL3",	   "CALL4",		 "CALL5",	   "CALL6", "CALL7", "CALL8",

										 "STATE",

										 "GOTO",

										 "AND",		 "OR",

										 "BITAND",	 "BITOR"};
// clang-format on

const char *PR_GlobalString (int ofs);
const char *PR_GlobalStringNoContents (int ofs);

//=============================================================================

/*
=================
PR_PrintStatement
=================
*/
static void PR_PrintStatement (dstatement_t *s)
{
	int i;

	if ((unsigned int)s->op < countof (pr_opnames))
	{
		Con_Printf ("%s ", pr_opnames[s->op]);
		i = strlen (pr_opnames[s->op]);
		for (; i < 10; i++)
			Con_Printf (" ");
	}

	if (s->op == OP_IF || s->op == OP_IFNOT)
		Con_Printf ("%sbranch %i", PR_GlobalString (s->a), s->b);
	else if (s->op == OP_GOTO)
	{
		Con_Printf ("branch %i", s->a);
	}
	else if ((unsigned int)(s->op - OP_STORE_F) < 6)
	{
		Con_Printf ("%s", PR_GlobalString (s->a));
		Con_Printf ("%s", PR_GlobalStringNoContents (s->b));
	}
	else
	{
		if (s->a)
			Con_Printf ("%s", PR_GlobalString (s->a));
		if (s->b)
			Con_Printf ("%s", PR_GlobalString (s->b));
		if (s->c)
			Con_Printf ("%s", PR_GlobalStringNoContents (s->c));
	}
	Con_Printf ("\n");
}

/*
============
PR_StackTrace
============
*/
static void PR_StackTrace (void)
{
	int			 i;
	dfunction_t *f;

	if (qcvm->depth == 0)
	{
		Con_Printf ("<NO STACK>\n");
		return;
	}

	qcvm->stack[qcvm->depth].f = qcvm->xfunction;
	for (i = qcvm->depth; i >= 0; i--)
	{
		f = qcvm->stack[i].f;
		if (!f)
		{
			Con_Printf ("<NO FUNCTION>\n");
		}
		else
		{
			Con_Printf ("%12s : %s\n", PR_GetString (f->s_file), PR_GetString (f->s_name));
		}
	}
}

/*
============
PR_Profile_f

============
*/
void PR_Profile_f (void)
{
	int			 i, num;
	int			 pmax;
	dfunction_t *f, *best;

	if (!sv.active)
		return;

	PR_SwitchQCVM (&sv.qcvm);

	num = 0;
	do
	{
		pmax = 0;
		best = NULL;
		for (i = 0; i < qcvm->progs->numfunctions; i++)
		{
			f = &qcvm->functions[i];
			if (f->profile > pmax)
			{
				pmax = f->profile;
				best = f;
			}
		}
		if (best)
		{
			if (num < 10)
				Con_Printf ("%7i %s\n", best->profile, PR_GetString (best->s_name));
			num++;
			best->profile = 0;
		}
	} while (best);

	PR_SwitchQCVM (NULL);
}

/*
============
PR_RunError

Aborts the currently executing function
============
*/
void PR_RunError (const char *error, ...)
{
	va_list argptr;
	char	string[1024];

	va_start (argptr, error);
	q_vsnprintf (string, sizeof (string), error, argptr);
	va_end (argptr);

	PR_PrintStatement (qcvm->statements + qcvm->xstatement);
	PR_StackTrace ();

	Con_Printf ("%s\n", string);

	qcvm->depth = 0; // dump the stack so host_error can shutdown functions

	Host_Error ("Program error");
}

void PR_RunWarning (const char *error, ...)
{
	va_list argptr;
	char	string[1024];

	va_start (argptr, error);
	q_vsnprintf (string, sizeof (string), error, argptr);
	va_end (argptr);

	PR_PrintStatement (qcvm->statements + qcvm->xstatement);
	PR_StackTrace ();

	Con_Warning ("%s\n", string);
}

/* ---------------------------------------------------------------------------
 * Callbacks the Rust interpreter makes back into the engine.
 */

void PRExec_Glue_PrintStatement (int pc)
{
	PR_PrintStatement (qcvm->statements + pc);
}

int PRExec_Glue_SvActive (void)
{
	return sv.state == ss_active;
}

int PRExec_Glue_Strcmp (const char *a, const char *b)
{
	return strcmp (a, b);
}

typedef struct
{
	int index;
} prexec_builtin_arg_t;

static void PRExec_InvokeBuiltin (void *arg)
{
	qcvm->builtins[((prexec_builtin_arg_t *)arg)->index]();
}

/* ADR-009 rule 3: a builtin can Host_Error, and that longjmp must not unwind
   the Rust interpreter frame above us. Host_Guard redirects it here; the
   status travels back through Rust as a normal return and pr_exec_glue's
   PR_ExecuteProgram re-issues the jump from a C frame. */
int PRExec_Glue_CallBuiltin (int index)
{
	prexec_builtin_arg_t arg;
	arg.index = index;
	return Host_Guard (PRExec_InvokeBuiltin, &arg);
}

/* The -tracefile sink (pr_trace.h). The PR_TRACE_* macros compile to nothing
   without -Dtrace=true, so the Rust side never needs to know which it is. */
int PRExec_Glue_TraceEnabled (void)
{
#ifdef PR_TRACE
	return pr_tracefile != NULL;
#else
	return 0;
#endif
}

void PRExec_Glue_TraceEnter (int fnum)
{
	PR_TRACE_ENTER (fnum);
}

void PRExec_Glue_TraceLeave (void)
{
	PR_TRACE_LEAVE ();
}

void PRExec_Glue_TraceStatement (int pc, int op, int a, int b, int c)
{
	PR_TRACE_STATEMENT (pc, op, a, b, c);
}

void PRExec_Glue_TraceGlobalWrite (int ofs, const int *values, int count)
{
	PR_TRACE_GLOBAL_WRITE (ofs, values, count);
}

void PRExec_Glue_TraceFieldWrite (int ofs, const int *values, int count)
{
	PR_TRACE_FIELD_WRITE (ofs, values, count);
}

void PRExec_Glue_TraceBuiltin (int ordinal, int argc, const int *parms)
{
	PR_TRACE_BUILTIN (ordinal, argc, parms);
}

void PRExec_Glue_TraceBuiltinReturn (const int *ret)
{
	PR_TRACE_BUILTIN_RETURN (ret);
}

/*
====================
PR_ExecuteProgram

The interpretation loop lives in quake-progs; this frame owns the entry
validation (it wants ED_Print, which is still pr_edict.c) and every error
raise.
====================
*/
void PR_ExecuteProgram (func_t fnum)
{
	int status;
	int detail = 0;

	if (!fnum || fnum >= (func_t)qcvm->progs->numfunctions)
	{
		if (pr_global_struct->self)
			ED_Print (PROG_TO_EDICT (pr_global_struct->self));
		Host_Error ("PR_ExecuteProgram: NULL function");
	}

	status = quake_rs_pr_execute_program (fnum, &detail);
	if (status == PREXEC_OK)
		return;

	switch (status)
	{
	case PREXEC_ERR_STACK_OVERFLOW:
		PR_RunError ("stack overflow");
	case PREXEC_ERR_LOCALS_OVERFLOW:
		PR_RunError ("PR_ExecuteProgram: locals stack overflow");
	case PREXEC_ERR_LOCALS_UNDERFLOW:
		PR_RunError ("PR_ExecuteProgram: locals stack underflow");
	case PREXEC_ERR_RUNAWAY:
		PR_RunError ("runaway loop error");
	case PREXEC_ERR_NULL_FUNCTION:
		PR_RunError ("NULL function");
	case PREXEC_ERR_WORLD_ASSIGN:
		PR_RunError ("assignment to world entity");
	case PREXEC_ERR_BAD_OPCODE:
		PR_RunError ("Bad opcode %i", detail);

	/* accepted divergences: C performs these accesses unchecked, which is an
	   out-of-bounds read or write on a malformed progs. See the ADR-006 note
	   in rust/quake-progs/src/arena.rs. */
	case PREXEC_ERR_FIELD_RANGE:
		PR_RunError ("bad field offset %i", detail);
	case PREXEC_ERR_BAD_FUNC_INDEX:
		PR_RunError ("bad function index %i", detail);
	case PREXEC_ERR_NO_STRING:
		Host_Error ("PR_GetString: attempt to get a non-existant string %d\n", detail);

	case PREXEC_ERR_STACK_UNDERFLOW:
		Host_Error ("prog stack underflow");

	/* a builtin raised; Rust has unwound, so re-issue the jump from here */
	case PREXEC_GUARD_ABORTSERVER:
		Host_Reraise (HOST_GUARD_ABORTSERVER);
		break;
	case PREXEC_GUARD_SCREEN_ERROR:
		Host_Reraise (HOST_GUARD_SCREEN_ERROR);
		break;

	default:
		Host_Error ("PR_ExecuteProgram: unknown status %i", status);
	}
}
