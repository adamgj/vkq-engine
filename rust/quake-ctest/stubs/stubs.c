/* Minimal engine-function stubs for the reference C files compiled into the
 * differential test binaries. */

#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
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

void Con_Printf (const char *fmt, ...)
{
	va_list ap;
	va_start (ap, fmt);
	vfprintf (stderr, fmt, ap);
	va_end (ap);
}

/* Mem_Alloc semantics per Quake/mem.h: zero-initialized */
void *Mem_Alloc (const size_t size)
{
	return calloc (1, size ? size : 1);
}

void *Mem_AllocNonZero (const size_t size)
{
	return malloc (size ? size : 1);
}

void *Mem_Realloc (void *ptr, const size_t size)
{
	return realloc (ptr, size ? size : 1);
}

void Mem_Free (const void *ptr)
{
	free ((void *)ptr);
}

/* UTF8_WriteCodePoint copied verbatim from Quake/common.c (json.c needs it) */
size_t UTF8_WriteCodePoint (char *dst, size_t maxbytes, uint32_t codepoint)
{
	if (!maxbytes)
		return 0;

	if (codepoint < 0x80)
	{
		dst[0] = (char)codepoint;
		return 1;
	}

	if (codepoint < 0x800)
	{
		if (maxbytes < 2)
			return 0;
		dst[0] = 0xC0 | (codepoint >> 6);
		dst[1] = 0x80 | (codepoint & 63);
		return 2;
	}

	if (codepoint < 0x10000)
	{
		if (maxbytes < 3)
			return 0;
		dst[0] = 0xE0 | (codepoint >> 12);
		dst[1] = 0x80 | ((codepoint >> 6) & 63);
		dst[2] = 0x80 | (codepoint & 63);
		return 3;
	}

	if (codepoint < 0x110000)
	{
		if (maxbytes < 4)
			return 0;
		dst[0] = 0xF0 | (codepoint >> 18);
		dst[1] = 0x80 | ((codepoint >> 12) & 63);
		dst[2] = 0x80 | ((codepoint >> 6) & 63);
		dst[3] = 0x80 | (codepoint & 63);
		return 4;
	}

	return 0;
}

/* COM_SeedRand / COM_Rand copied verbatim from Quake/common.c so stress tests
 * that consume the RNG behave identically */
static uint32_t xorshiro_state[2];

void COM_SeedRand (uint64_t seed)
{
	uint64_t z = (seed + 0x9e3779b97f4a7c15);
	z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9;
	z = (z ^ (z >> 27)) * 0x94d049bb133111eb;
	uint64_t state = z ^ (z >> 31);
	xorshiro_state[0] = (uint32_t)state;
	xorshiro_state[1] = (uint32_t)(state >> 32);
}

static inline uint32_t rotl (const uint32_t x, int k)
{
	return (x << k) | (x >> (32 - k));
}

int32_t COM_Rand (void)
{
	const uint32_t s0 = xorshiro_state[0];
	uint32_t	   s1 = xorshiro_state[1];
	const uint32_t result = rotl (s0 * 0x9E3779BB, 5) * 5;
	s1 ^= s0;
	xorshiro_state[0] = rotl (s0, 26) ^ s1 ^ (s1 << 9);
	xorshiro_state[1] = rotl (s1, 13);
	return result;
}
