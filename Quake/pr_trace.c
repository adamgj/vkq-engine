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

// pr_trace.c -- progs VM trace sink (ADR-019); see pr_trace.h for the format

#include "quakedef.h"
#include "pr_trace.h"

#ifdef PR_TRACE

FILE *pr_tracefile = NULL;

void PR_TraceInit (void)
{
	int i = COM_CheckParm ("-tracefile");
	if (!i || i >= com_argc - 1)
		return;
	pr_tracefile = Sys_fopen (com_argv[i + 1], "w");
	if (!pr_tracefile)
		Sys_Error ("PR_TraceInit: can't open %s", com_argv[i + 1]);
	fprintf (pr_tracefile, "PRTRACE 1\n");
}

void PR_TraceShutdown (void)
{
	if (pr_tracefile)
	{
		fclose (pr_tracefile);
		pr_tracefile = NULL;
	}
}

void PR_TraceEnter (int fnum)
{
	fprintf (pr_tracefile, "E %d\n", fnum);
}

void PR_TraceLeave (void)
{
	fprintf (pr_tracefile, "L\n");
}

void PR_TraceStatement (int pc, int op, int a, int b, int c)
{
	fprintf (pr_tracefile, "S %d %d %d %d %d\n", pc, op, a, b, c);
}

static void PR_TraceValues (const char *tag, int ofs, const int *values, int count)
{
	int i;
	fprintf (pr_tracefile, "%s %d %d", tag, ofs, count);
	for (i = 0; i < count; i++)
		fprintf (pr_tracefile, " %x", (unsigned)values[i]);
	fputc ('\n', pr_tracefile);
}

void PR_TraceGlobalWrite (int ofs, const int *values, int count)
{
	PR_TraceValues ("W", ofs, values, count);
}

void PR_TraceFieldWrite (int byteofs, const int *values, int count)
{
	PR_TraceValues ("P", byteofs, values, count);
}

void PR_TraceBuiltin (int ordinal, int argc, const int *parms)
{
	int i;
	fprintf (pr_tracefile, "B %d %d", ordinal, argc);
	for (i = 0; i < argc * 3; i++)
		fprintf (pr_tracefile, " %x", (unsigned)parms[i]);
	fputc ('\n', pr_tracefile);
}

void PR_TraceBuiltinReturn (const int *ret)
{
	fprintf (pr_tracefile, "R %x %x %x\n", (unsigned)ret[0], (unsigned)ret[1], (unsigned)ret[2]);
}

#endif /* PR_TRACE */
