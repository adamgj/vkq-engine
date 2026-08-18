/* Test-only snprintf oracle for the ADR-005 formatter conformance suite.
 *
 * Typed non-variadic wrappers so the Rust side never constructs a C vararg
 * call itself; compiled by the platform compiler against the platform CRT,
 * which is exactly the formatting engine the C build's savegame/config
 * writers use. */

#include <stdio.h>
#include <stddef.h>

int ctest_snprintf_f (char *buf, size_t n, const char *fmt, double v)
{
	return snprintf (buf, n, fmt, v);
}

int ctest_snprintf_i32 (char *buf, size_t n, const char *fmt, int v)
{
	return snprintf (buf, n, fmt, v);
}

int ctest_snprintf_u32 (char *buf, size_t n, const char *fmt, unsigned v)
{
	return snprintf (buf, n, fmt, v);
}

int ctest_snprintf_i64 (char *buf, size_t n, const char *fmt, long long v)
{
	return snprintf (buf, n, fmt, v);
}

int ctest_snprintf_u64 (char *buf, size_t n, const char *fmt, unsigned long long v)
{
	return snprintf (buf, n, fmt, v);
}

int ctest_snprintf_str (char *buf, size_t n, const char *fmt, const char *v)
{
	return snprintf (buf, n, fmt, v);
}
