/*
Copyright (C) 2026 vkQuake contributors

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

#ifndef __PR_TRACE_H
#define __PR_TRACE_H

#include "q_types.h" // FILE, via the libc includes

/* Per-instruction progs VM trace for the Rust migration's differential
   verification (ADR-019). Compiled in with -Dtrace=true (-DPR_TRACE) and
   active when -tracefile <path> is given. Independent of the qcvm->trace
   Con_Printf debugging path, which is unchanged.

   Versioned text format, one record per line:
	 PRTRACE 1          header
	 E <fnum>           function enter
	 L                  function leave
	 S <pc> <op> <a> <b> <c>              statement (before execution)
	 W <ofs> <n> <v..>  global write (store family), raw i32 values in hex
	 P <ofs> <n> <v..>  entity-field write (storep family), byte offset
	 B <ord> <argc> <parm hex triplets>   builtin call (before)
	 R <v0> <v1> <v2>   builtin return (OFS_RETURN after)

   The sink is an unlocked FILE *, and PR_ExecuteProgram can run on a task
   worker (CSQC drawing under draw_qcvm_mutex), so trace byte-stability holds
   only while progs execution is single-threaded -- true under -headless, not
   true for a windowed -tracefile run. Only headless traces are oracles. */

#ifdef PR_TRACE

void PR_TraceInit (void);	  /* checks -tracefile, opens the sink */
void PR_TraceShutdown (void); /* flush + close */

extern FILE *pr_tracefile;

void PR_TraceEnter (int fnum);
void PR_TraceLeave (void);
void PR_TraceStatement (int pc, int op, int a, int b, int c);
void PR_TraceGlobalWrite (int ofs, const int *values, int count);
void PR_TraceFieldWrite (int byteofs, const int *values, int count);
void PR_TraceBuiltin (int ordinal, int argc, const int *parms);
void PR_TraceBuiltinReturn (const int *ret);

/* do/while(0): a bare `if` would bind a following `else` to the macro's own
   conditional, and in the !PR_TRACE build below would swallow the next
   statement -- these hooks get sprinkled through the VM in later phases */
#define PR_TRACE_ENTER(fnum)      \
	do                            \
	{                             \
		if (pr_tracefile)         \
			PR_TraceEnter (fnum); \
	} while (0)
#define PR_TRACE_LEAVE()      \
	do                        \
	{                         \
		if (pr_tracefile)     \
			PR_TraceLeave (); \
	} while (0)
#define PR_TRACE_STATEMENT(pc, op, a, b, c)      \
	do                                           \
	{                                            \
		if (pr_tracefile)                        \
			PR_TraceStatement (pc, op, a, b, c); \
	} while (0)
#define PR_TRACE_GLOBAL_WRITE(ofs, values, count)     \
	do                                                \
	{                                                 \
		if (pr_tracefile)                             \
			PR_TraceGlobalWrite (ofs, values, count); \
	} while (0)
#define PR_TRACE_FIELD_WRITE(ofs, values, count)     \
	do                                               \
	{                                                \
		if (pr_tracefile)                            \
			PR_TraceFieldWrite (ofs, values, count); \
	} while (0)
#define PR_TRACE_BUILTIN(ordinal, argc, parms)      \
	do                                              \
	{                                               \
		if (pr_tracefile)                           \
			PR_TraceBuiltin (ordinal, argc, parms); \
	} while (0)
#define PR_TRACE_BUILTIN_RETURN(ret)     \
	do                                   \
	{                                    \
		if (pr_tracefile)                \
			PR_TraceBuiltinReturn (ret); \
	} while (0)

#else /* !PR_TRACE: hooks compile to nothing */

#define PR_TRACE_ENTER(fnum) \
	do                       \
	{                        \
	} while (0)
#define PR_TRACE_LEAVE() \
	do                   \
	{                    \
	} while (0)
#define PR_TRACE_STATEMENT(pc, op, a, b, c) \
	do                                      \
	{                                       \
	} while (0)
#define PR_TRACE_GLOBAL_WRITE(ofs, values, count) \
	do                                            \
	{                                             \
	} while (0)
#define PR_TRACE_FIELD_WRITE(ofs, values, count) \
	do                                           \
	{                                            \
	} while (0)
#define PR_TRACE_BUILTIN(ordinal, argc, parms) \
	do                                         \
	{                                          \
	} while (0)
#define PR_TRACE_BUILTIN_RETURN(ret) \
	do                               \
	{                                \
	} while (0)

#endif /* PR_TRACE */

#endif /* __PR_TRACE_H */
