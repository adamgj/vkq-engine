/* Minimal engine-function stubs for the reference C files compiled into the
 * differential test binaries. */

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>

void Sys_Error (const char *error, ...)
{
	va_list ap;
	va_start (ap, error);
	vfprintf (stderr, error, ap);
	va_end (ap);
	abort ();
}
