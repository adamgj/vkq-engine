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

// common.c -- misc functions used in client and server

#include "quakedef.h"
#include "sys.h"

#include "q_ctype.h"
#include "filenames.h"
#include <errno.h>

static char *largv[MAX_NUM_ARGVS + 1];
static char	 argvdummy[] = " ";

int safemode;

cvar_t registered = {"registered", "1", CVAR_ROM};				 /* set to correct value in COM_CheckRegistered() */
cvar_t cmdline = {"cmdline", "", CVAR_ROM /*|CVAR_SERVERINFO*/}; /* sending cmdline upon CCREQ_RULE_INFO is evil */

qboolean multiuser;

THREAD_LOCAL char com_token[COM_PARSE_MAX_TOKEN_SIZE];
int				  com_argc;
char			**com_argv;

static char com_cmdline[CMDLINE_LENGTH];

/*

All of Quake's data access is through a hierchal file system, but the contents
of the file system can be transparently merged from several sources.

The "base directory" is the path to the directory holding the quake.exe and all
game directories.  The sys_* files pass this to host_init in quakeparms_t->basedir.
This can be overridden with the "-basedir" command line parm to allow code
debugging in a different directory.  The base directory is only used during
filesystem initialization.

The "game directory" is the first tree on the search path and directory that all
generated files (savegames, screenshots, demos, config files) will be saved to.
This can be overridden with the "-game" command line parameter.  The game
directory can never be changed while quake is executing.  This is a precacution
against having a malicious server instruct clients to write files over areas they
shouldn't.

The "cache directory" is only used during development to save network bandwidth,
especially over ISDN / T1 lines.  If there is a cache directory specified, when
a file is found by the normal search path, it will be mirrored into the cache
directory, then opened there.

FIXME:
The file "parms.txt" will be read out of the game directory and appended to the
current command line arguments to allow different games to initialize startup
parms differently.  This could be used to add a "-sspeed 22050" for the high
quality sound edition.  Because they are added at the end, they will not
override an explicit setting on the original command line.

*/

//============================================================================

// ClearLink is used for new headnodes
void ClearLink (link_t *l)
{
	l->prev = l->next = l;
}

void RemoveLink (link_t *l)
{
	l->next->prev = l->prev;
	l->prev->next = l->next;
}

void InsertLinkBefore (link_t *l, link_t *before)
{
	l->next = before;
	l->prev = before->prev;
	l->prev->next = l;
	l->next->prev = l;
}

void InsertLinkAfter (link_t *l, link_t *after)
{
	l->next = after->next;
	l->prev = after;
	l->prev->next = l;
	l->next->prev = l;
}

/*
============================================================================

							DYNAMIC VECTORS

============================================================================
*/

void Vec_Grow (void **pvec, size_t element_size, size_t count)
{
	vec_header_t header;
	if (*pvec)
		header = VEC_HEADER (*pvec);
	else
		header.size = header.capacity = 0;

	if (header.size + count > header.capacity)
	{
		void  *new_buffer;
		size_t total_size;

		header.capacity = header.size + count;
		header.capacity += header.capacity >> 1;
		if (header.capacity < 16)
			header.capacity = 16;
		total_size = sizeof (vec_header_t) + header.capacity * element_size;

		if (*pvec)
			new_buffer = Mem_Realloc (((vec_header_t *)*pvec) - 1, total_size);
		else
			new_buffer = Mem_Alloc (total_size);
		if (!new_buffer)
			Sys_Error ("Vec_Grow: failed to allocate %lu bytes\n", (unsigned long)total_size);

		*pvec = 1 + (vec_header_t *)new_buffer;
		VEC_HEADER (*pvec) = header;
	}
}

void Vec_Append (void **pvec, size_t element_size, const void *data, size_t count)
{
	if (!count)
		return;
	Vec_Grow (pvec, element_size, count);
	memcpy ((byte *)*pvec + VEC_HEADER (*pvec).size * element_size, data, count * element_size);
	VEC_HEADER (*pvec).size += count;
}

void Vec_Clear (void **pvec)
{
	if (*pvec)
		VEC_HEADER (*pvec).size = 0;
}

void Vec_Free (void **pvec)
{
	if (*pvec)
	{
		Mem_Free (&VEC_HEADER (*pvec));
		*pvec = NULL;
	}
}

/*
============================================================================

					LIBRARY REPLACEMENT FUNCTIONS

============================================================================
*/

int q_strnaturalcmp (const char *s1, const char *s2)
{
	qboolean neg1, neg2, sign1, sign2;

	if (s1 == s2)
		return 0;

	neg1 = *s1 == '-';
	neg2 = *s2 == '-';
	sign1 = neg1 || *s1 == '+';
	sign2 = neg2 || *s2 == '+';

	// early out if strings start with different signs followed by digits
	if (neg1 != neg2 && q_isdigit (s1[sign1]) && q_isdigit (s1[sign2]))
		return neg2 - neg1;

skip_prefix:
	while (*s1 && !q_isdigit (*s1) && q_toupper (*s1) == q_toupper (*s2))
	{
		s1++;
		s2++;
		continue;
	}

	if (q_isdigit (*s1) && q_isdigit (*s2))
	{
		const char *begin1 = s1++;
		const char *begin2 = s2++;
		int			diff, sign;

		while (*begin1 == '0')
			begin1++;
		while (*begin2 == '0')
			begin2++;

		while (q_isdigit (*s1))
			s1++;
		while (q_isdigit (*s2))
			s2++;

		sign = neg1 ? -1 : 1;

		diff = (s1 - begin1) - (s2 - begin2);
		if (diff)
			return diff * sign;

		while (begin1 != s1)
		{
			diff = *begin1++ - *begin2++;
			if (diff)
				return diff * sign;
		}

		// We only support negative numbers at the beginning of strings so that
		// "-2" is sorted before "-1", but "file-2345.ext" *after* "file-1234.ext".
		neg1 = neg2 = false;

		goto skip_prefix;
	}

	return q_toupper (*s1) - q_toupper (*s2);
}

int q_strcasecmp (const char *s1, const char *s2)
{
	const char *p1 = s1;
	const char *p2 = s2;
	char		c1, c2;

	if (p1 == p2)
		return 0;

	do
	{
		c1 = q_tolower (*p1++);
		c2 = q_tolower (*p2++);
		if (c1 == '\0')
			break;
	} while (c1 == c2);

	return (int)(c1 - c2);
}

int q_strncasecmp (const char *s1, const char *s2, size_t n)
{
	const char *p1 = s1;
	const char *p2 = s2;
	char		c1, c2;

	if (p1 == p2 || n == 0)
		return 0;

	do
	{
		c1 = q_tolower (*p1++);
		c2 = q_tolower (*p2++);
		if (c1 == '\0' || c1 != c2)
			break;
	} while (--n > 0);

	return (int)(c1 - c2);
}

char *q_strcasestr (const char *haystack, const char *needle)
{
	const size_t len = strlen (needle);

	if (!len)
		return (char *)haystack;

	while (*haystack)
	{
		if (!q_strncasecmp (haystack, needle, len))
			return (char *)haystack;

		++haystack;
	}

	return NULL;
}

/*
================
COM_TintSubstring
================
*/
char *COM_TintSubstring (const char *in, const char *substr, char *out, size_t outsize)
{
	int	  l;
	char *m = out;
	q_strlcpy (out, in, outsize);
	if (*substr)
	{
		while ((m = q_strcasestr (m, substr)))
		{
			for (l = 0; substr[l]; l++)
				if (m[l] > ' ')
					m[l] |= 0x80;
			m += l;
		}
	}
	return out;
}

char *q_strlwr (char *str)
{
	char *c;
	c = str;
	while (*c)
	{
		*c = q_tolower (*c);
		c++;
	}
	return str;
}

char *q_strupr (char *str)
{
	char *c;
	c = str;
	while (*c)
	{
		*c = q_toupper (*c);
		c++;
	}
	return str;
}

/*
==================
UTF8_WriteCodePoint

Writes the UTF-8 encoding of the given code point.
Returns the number of bytes written (up to 4),
or 0 on error (overflow or invalid code point)
==================
*/
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

// clang-format off
static const uint32_t qchar_to_unicode[256] =
{/*     0       1       2       3       4       5       6       7       8       9       10      11      12      13      14      15
      ----------------------------------------------------------------------------------------------------------------------------------
  0 */  0x00B7, 0,      0,      0,      0,      0x00B7, 0,      0,      0,      0,      '\n',   0x25A0, ' ',    0x25B6, 0x00B7, 0x00B7, /*
  1 */  0x301A, 0x301B, '0',    '1',    '2',    '3',    '4',    '5',    '6',    '7',    '8',    '9',    0x00B7, '-',    '-',    '-',    /*
  2 */  ' ',    '!',    '"',    '#',    '$',    '%',    '&',    '\'',   '(',    ')',    '*',    '+',    ',',    '-',    '.',    '/',    /*
  3 */  '0',    '1',    '2',    '3',    '4',    '5',    '6',    '7',    '8',    '9',    ':',    ';',    '<',    '=',    '>',    '?',    /*
  4 */  '@',    'A',    'B',    'C',    'D',    'E',    'F',    'G',    'H',    'I',    'J',    'K',    'L',    'M',    'N',    'O',    /*
  5 */  'P',    'Q',    'R',    'S',    'T',    'U',    'V',    'W',    'X',    'Y',    'Z',    '[',    '\\',   ']',    '^',    '_',    /*
  6 */  '`',    'a',    'b',    'c',    'd',    'e',    'f',    'g',    'h',    'i',    'j',    'k',    'l',    'm',    'n',    'o',    /*
  7 */  'p',    'q',    'r',    's',    't',    'u',    'v',    'w',    'x',    'y',    'z',    '{',    '|',    '}',    '~',    0x2190, /*

  8 */  '-',    '-',    '-',    '-',    0,      0x2022, 0,      0,      0,      0,      '\n',   0x25A0, ' ',    0x25B6, 0x2022, 0x2022, /*
  9 */  0x301A, 0x301B, '0',    '1',    '2',    '3',    '4',    '5',    '6',    '7',    '8',    '9',    0x2022, '-',    '-',    '-',    /*
 10 */  ' ',    '!',    '"',    '#',    '$',    '%',    '&',    '\'',   '(',    ')',    '*',    '+',    ',',    '-',    '.',    '/',    /*
 11 */  '0',    '1',    '2',    '3',    '4',    '5',    '6',    '7',    '8',    '9',    ':',    ';',    '<',    '=',    '>',    '?',    /*
 12 */  '@',    'A',    'B',    'C',    'D',    'E',    'F',    'G',    'H',    'I',    'J',    'K',    'L',    'M',    'N',    'O',    /*
 13 */  'P',    'Q',    'R',    'S',    'T',    'U',    'V',    'W',    'X',    'Y',    'Z',    '[',    '\\',   ']',    '^',    '_',    /*
 14 */  '`',    'a',    'b',    'c',    'd',    'e',    'f',    'g',    'h',    'i',    'j',    'k',    'l',    'm',    'n',    'o',    /*
 15 */  'p',    'q',    'r',    's',    't',    'u',    'v',    'w',    'x',    'y',    'z',    '{',    '|',    '}',    '~',    0x2190, /*
      ----------------------------------------------------------------------------------------------------------------------------------
*/};
// clang-format on

/*
==================
UTF8_CodePointLength

Returns the number of bytes needed to encode the codepoint
using UTF-8 (max 4), or 0 for an invalid code point
==================
*/
size_t UTF8_CodePointLength (uint32_t codepoint)
{
	if (codepoint < 0x80)
		return 1;

	if (codepoint < 0x800)
		return 2;

	if (codepoint < 0x10000)
		return 3;

	if (codepoint < 0x110000)
		return 4;

	return 0;
}

/*
==================
UTF8_FromQuake

Converts a string from Quake encoding to UTF-8

Returns the number of written characters (including the NUL terminator)
if a valid output buffer is provided (dst is non-NULL, maxbytes > 0),
or the total amount of space necessary to encode the entire src string
if dst is NULL and maxbytes is 0.
==================
*/
size_t UTF8_FromQuake (char *dst, size_t maxbytes, const char *src)
{
	size_t i, j, written;

	if (!maxbytes)
	{
		if (dst)
			return 0; // error
		for (i = 0, j = 0; src[i]; i++)
		{
			uint32_t codepoint = qchar_to_unicode[(unsigned char)src[i]];
			if (codepoint)
				j += UTF8_CodePointLength (codepoint);
		}
		return j + 1; // include terminator
	}

	--maxbytes;

	for (i = 0, j = 0; j < maxbytes && src[i]; i++)
	{
		uint32_t codepoint = qchar_to_unicode[(unsigned char)src[i]];
		if (!codepoint)
			continue;
		written = UTF8_WriteCodePoint (dst + j, maxbytes - j, codepoint);
		if (!written)
			break;
		j += written;
	}

	dst[j++] = '\0';

	return j;
}

char *q_strtrim (char *str)
{
	// trim leading and ending whitespaces:
	char *str_start = (char *)str;

	// trim leading:
	while (q_isspace ((unsigned char)*str))
		str++;

	// trim ending :
	size_t last_index = strlen (str_start) - 1;

	while (last_index >= 0)
	{
		if (q_isspace (str_start[last_index]))
		{
			str_start[last_index] = '\0';
		}
		else
		{
			break;
		}
		last_index--;
	}

	return str;
}

static bool is_in_char_set (char single_char, const char *char_set)
{
	const size_t char_set_size = strlen (char_set);

	for (size_t char_index = 0; char_index < char_set_size; char_index++)
	{
		if (char_set[char_index] == single_char)
			return true;
	}

	return false;
}

char **q_strsplit (char *str, const char *sep_set, size_t *nb_substr)
{
	size_t nb_sub_strings_max_size = 8;
	// if the nb_substr is NULL, we are just interested in splitting str-on place by '\0' ,
	// and not in returning the token starts at all.
	char **sub_strings = (nb_substr ? Mem_Alloc (nb_sub_strings_max_size * sizeof (char *)) : NULL);
	int	   nb_sub_strings = 0;

	size_t start_str_index = 0;

	// special case, gobble the leading sep characters:
	while (is_in_char_set (str[start_str_index], sep_set))
	{
		str[start_str_index] = 0;
		start_str_index++;
	}
	// the real start of the string is here
	char *str_start = &str[start_str_index];

	// always return a valid memory although  nb_sub_strings = 0
	// so that the caller is not burdened with NULL checks.
	//  TODO: or more explicit if it would ?
	assert (nb_sub_strings == 0);
	if (!str_start)
		return sub_strings;

	const size_t initial_str_size = strlen (str_start);

	for (size_t char_index = 0; char_index < initial_str_size; char_index++)
	{
		// find the next sep
		if (is_in_char_set (str_start[char_index], sep_set))
		{
			// goble consecutive seps, if any
			while (is_in_char_set (str_start[char_index], sep_set))
			{
				// split the original string
				str_start[char_index] = '\0';
				char_index++;
			}
			//
			if (sub_strings && char_index <= initial_str_size)
			{
				// make room
				if (nb_sub_strings >= nb_sub_strings_max_size)
				{
					nb_sub_strings_max_size = nb_sub_strings_max_size * 2;
					sub_strings = Mem_Realloc (sub_strings, nb_sub_strings_max_size * sizeof (char *));
				}
				// we found the first split, meaning the string before this split is indeed the first sub-string
				if (nb_sub_strings == 0)
					sub_strings[nb_sub_strings++] = &str_start[0];

				if (char_index < initial_str_size)
					sub_strings[nb_sub_strings++] = &str_start[char_index];
			}
		}
	}

	// no split, return the original string stripped from its leadings seps
	if (sub_strings && nb_sub_strings == 0)
		sub_strings[nb_sub_strings++] = &str_start[0];

	if (nb_substr)
		*nb_substr = nb_sub_strings;

	return sub_strings;
}

char *q_strdup (const char *str)
{
	size_t len = strlen (str) + 1;
	char  *newstr = (char *)Mem_Alloc (len);
	memcpy (newstr, str, len);
	return newstr;
}

int q_vsnprintf (char *str, size_t size, const char *format, va_list args)
{
	int ret;

	ret = vsnprintf (str, size, format, args);

	if (ret < 0)
		ret = (int)size;
	if (size == 0) /* no buffer */
		return ret;
	if ((size_t)ret >= size)
		str[size - 1] = '\0';

	return ret;
}

int q_snprintf (char *str, size_t size, const char *format, ...)
{
	int		ret;
	va_list argptr;

	va_start (argptr, format);
	ret = q_vsnprintf (str, size, format, argptr);
	va_end (argptr);

	return ret;
}

char *q_vstrcatf (char *input_str, const char *format, va_list args)
{
#define MIN_SIZE_POW 8
#define MIN_SIZE	 (1 << MIN_SIZE_POW)

	char *output_str = NULL;

	if (input_str == NULL)
		input_str = (char *)Mem_Alloc (MIN_SIZE);

	// We allways construct the dynamically allocated buffer a way we can
	// get its current size from the current input_str_size: allocated size is the next power of 2 strictly.
	const size_t input_str_size = strlen (input_str);
	const size_t input_str_allocated_size = (input_str_size + 1 < MIN_SIZE) ? MIN_SIZE : Q_nextPow2_Strict (input_str_size + 1);

	// we can push remaining_size more characters (including null)
	size_t remaining_size = input_str_allocated_size - (input_str_size + 1);

	//  First try : attempt to sprintf and append into the current input_str
	va_list argptr_first;
	va_copy (argptr_first, args);

	// Note : we need C99 conformant vsnprintf, returning the number of chars that are written, or would have been written
	// This is OK for MSVC 2015+ and all other compilers out there
	int expected_append_size = vsnprintf ((char *)(input_str + input_str_size), remaining_size, format, argptr_first);
	va_end (argptr_first);

	// Something wrong happened, return the original, unmodified.
	if (expected_append_size < 0)
	{
		output_str = (char *)input_str;
		output_str[input_str_size] = '\0';
		return output_str;
	}
	// Fits into the remaining room
	if (expected_append_size < remaining_size)
	{
		output_str = (char *)input_str;
		// the C99 conformant vsnprintf should already do this, but anyway
		output_str[input_str_size + expected_append_size] = '\0';
		return output_str;
	}

	// Second try : do not fit, so reallocate to the next power of 2 for the final size
	const size_t output_str_size = input_str_size + expected_append_size;
	size_t		 output_str_allocated_size = Q_nextPow2_Strict (output_str_size + 1);

	va_list argptr_second;
	va_copy (argptr_second, args);

	output_str = (char *)Mem_Realloc ((void *)input_str, output_str_allocated_size);
	output_str[input_str_size] = '\0';

	remaining_size = output_str_allocated_size - (input_str_size + 1);

	if (expected_append_size == vsnprintf ((char *)(output_str + input_str_size), remaining_size, format, argptr_second))
	{
		output_str[input_str_size + expected_append_size] = '\0';
	}

	va_end (argptr_second);

	return output_str;

#undef MIN_SIZE_POW
#undef MIN_SIZE
}

char *q_strcatf (char *input_str, const char *format, ...)
{
	va_list argptr;
	va_start (argptr, format);
	char *output_buffer = q_vstrcatf (input_str, format, argptr);
	va_end (argptr);

	return output_buffer;
}

int wildcmp (const char *wild, const char *string)
{ // case-insensitive string compare with wildcards. returns true for a match.
	while (*string)
	{
		if (*wild == '*')
		{
			if (*string == '/' || *string == '\\')
			{
				//* terminates if we get a match on the char following it, or if its a \ or / char
				wild++;
				continue;
			}
			if (wildcmp (wild + 1, string))
				return true;
			string++;
		}
		else if ((q_tolower (*wild) == q_tolower (*string)) || (*wild == '?'))
		{
			// this char matches
			wild++;
			string++;
		}
		else
		{
			// failure
			return false;
		}
	}

	while (*wild == '*')
	{
		wild++;
	}
	return !*wild;
}

void Info_RemoveKey (char *info, const char *key)
{ // only shrinks, so no need for max size.
	size_t keylen = strlen (key);

	while (*info)
	{
		char *l = info;
		if (*info++ != '\\')
			break; // error / end-of-string

		if (!strncmp (info, key, keylen) && info[keylen] == '\\')
		{
			// skip the key name
			info += keylen + 1;
			// this is the old value for the key. skip over it
			while (*info && *info != '\\')
				info++;

			// okay, we found it. strip it out now.
			memmove (l, info, strlen (info) + 1);
			return;
		}
		else
		{
			// skip the key
			while (*info && *info != '\\')
				info++;

			// validate that its a value now
			if (*info++ != '\\')
				break; // error
			// skip the value
			while (*info && *info != '\\')
				info++;
		}
	}
}
void Info_SetKey (char *info, size_t infosize, const char *key, const char *val)
{
	size_t keylen = strlen (key);
	size_t vallen = strlen (val);

	Info_RemoveKey (info, key);

	if (vallen)
	{
		char *o = info + strlen (info);
		char *e = info + infosize - 1;

		if (!*key || strchr (key, '\\') || strchr (val, '\\'))
			Con_Warning ("Info_SetKey(%s): invalid key/value\n", key);
		else if (o + 2 + keylen + vallen >= e)
			Con_Warning ("Info_SetKey(%s): length exceeds max\n", key);
		else
		{
			*o++ = '\\';
			memcpy (o, key, keylen);
			o += keylen;
			*o++ = '\\';
			memcpy (o, val, vallen);
			o += vallen;

			*o = 0;
		}
	}
}
const char *Info_GetKey (const char *info, const char *key, char *out, size_t outsize)
{
	const char *r = out;
	size_t		keylen = strlen (key);

	outsize--;

	while (*info)
	{
		if (*info++ != '\\')
			break; // error / end-of-string

		if (!strncmp (info, key, keylen) && info[keylen] == '\\')
		{
			// skip the key name
			info += keylen + 1;
			// this is the value for the key. copy it out
			while (*info && *info != '\\' && outsize-- > 0)
				*out++ = *info++;
			break;
		}
		else
		{
			// skip the key
			while (*info && *info != '\\')
				info++;

			// validate that its a value now
			if (*info++ != '\\')
				break; // error
			// skip the value
			while (*info && *info != '\\')
				info++;
		}
	}
	*out = 0;
	return r;
}

void Info_Enumerate (const char *info, void (*cb) (void *ctx, const char *key, const char *value), void *cbctx)
{
	char   key[1024];
	char   val[1024];
	size_t kl, vl;
	while (*info)
	{
		kl = vl = 0;
		if (*info++ != '\\')
			break; // error / end-of-string

		// skip the key
		while (*info && *info != '\\')
		{
			if (kl < sizeof (key) - 1)
				key[kl++] = *info;
			info++;
		}

		// validate that its a value now
		if (*info++ != '\\')
			break; // error
		// skip the value
		while (*info && *info != '\\')
		{
			if (vl < sizeof (val) - 1)
				val[vl++] = *info;
			info++;
		}

		key[kl] = 0;
		val[vl] = 0;
		cb (cbctx, key, val);
	}
}
static void Info_Print_Callback (void *ctx, const char *key, const char *val)
{
	Con_Printf ("%20s: %s\n", key, val);
}
void Info_Print (const char *info)
{
	Info_Enumerate (info, Info_Print_Callback, NULL);
}
/*
============================================================================

					BYTE ORDER FUNCTIONS

============================================================================
*/

short ShortSwap (short l)
{
	byte b1, b2;

	b1 = l & 255;
	b2 = (l >> 8) & 255;

	return ((unsigned short)b1 << 8) + b2;
}

short ShortNoSwap (short l)
{
	return l;
}

int LongSwap (int l)
{
	byte b1, b2, b3, b4;

	b1 = l & 255;
	b2 = (l >> 8) & 255;
	b3 = (l >> 16) & 255;
	b4 = (l >> 24) & 255;

	return ((unsigned int)b1 << 24) + ((unsigned int)b2 << 16) + ((unsigned int)b3 << 8) + b4;
}

int LongNoSwap (int l)
{
	return l;
}

float FloatSwap (float f)
{
	union
	{
		float f;
		byte  b[4];
	} dat1, dat2;

	dat1.f = f;
	dat2.b[0] = dat1.b[3];
	dat2.b[1] = dat1.b[2];
	dat2.b[2] = dat1.b[1];
	dat2.b[3] = dat1.b[0];
	return dat2.f;
}

float FloatNoSwap (float f)
{
	return f;
}

short (*BigShort) (short l) = ShortSwap;
short (*LittleShort) (short l) = ShortNoSwap;

int (*BigLong) (int l) = LongSwap;
int (*LittleLong) (int l) = LongNoSwap;

float (*BigFloat) (float l) = FloatSwap;
float (*LittleFloat) (float l) = FloatNoSwap;

/*
==============================================================================

			MESSAGE IO FUNCTIONS

Handles byte ordering and avoids alignment errors
==============================================================================
*/

//
// writing functions
//

void MSG_WriteChar (sizebuf_t *sb, int c)
{
	byte *buf;

#if defined(DEBUG) || defined(_DEBUG)
	if (c < -128 || c > 127)
		Host_Error ("MSG_WriteChar: range error = %i not in -128..127", c);
#endif

	buf = (byte *)SZ_GetSpace (sb, 1);
	buf[0] = c;
}

void MSG_WriteByte (sizebuf_t *sb, int c)
{
	byte *buf;

#if defined(DEBUG) || defined(_DEBUG)
	if (c < 0 || c > 255)
		Host_Error ("MSG_WriteByte: range error = %i not in 0..255", c);
#endif

	buf = (byte *)SZ_GetSpace (sb, 1);
	buf[0] = c;
}

void MSG_WriteShort (sizebuf_t *sb, int c)
{
	byte *buf;

#if defined(DEBUG) || defined(_DEBUG)
	// it is apparently used to encode signed OR unsigned shorts...
	if (c < INT16_MIN || c > UINT16_MAX)
		Host_Error ("MSG_WriteShort: range error = %i not in -32768..65535", c);
#endif

	buf = (byte *)SZ_GetSpace (sb, 2);
	buf[0] = c & 0xff;
	buf[1] = c >> 8;
}

void MSG_WriteLong (sizebuf_t *sb, int c)
{
	byte *buf;

	buf = (byte *)SZ_GetSpace (sb, 4);
	buf[0] = c & 0xff;
	buf[1] = (c >> 8) & 0xff;
	buf[2] = (c >> 16) & 0xff;
	buf[3] = c >> 24;
}

void MSG_WriteUInt64 (sizebuf_t *sb, unsigned long long c)
{ // 0* 10*,*, 110*,*,* etc, up to 0xff followed by 8 continuation bytes
	byte			  *buf;
	int				   b = 0;
	unsigned long long l = 128;
	while (c > l - 1u)
	{ // count the extra bytes we need
		b++;
		l <<= 7; // each byte we add gains 8 bits, but we spend one on length.
	}
	buf = (byte *)SZ_GetSpace (sb, 1 + b);
	*buf++ = 0xffu << (8 - b) | (c >> (b * 8));
	while (b-- > 0)
		*buf++ = (c >> (b * 8)) & 0xff;
}
void MSG_WriteInt64 (sizebuf_t *sb, long long c)
{ // move the sign bit into the low bit and avoid sign extension for more efficient length coding.
	if (c < 0)
		MSG_WriteUInt64 (sb, ((unsigned long long)(-1 - c) << 1) | 1);
	else
		MSG_WriteUInt64 (sb, c << 1);
}

void MSG_WriteFloat (sizebuf_t *sb, float f)
{
	union
	{
		float f;
		int	  l;
	} dat;

	dat.f = f;
	dat.l = LittleLong (dat.l);

	SZ_Write (sb, &dat.l, 4);
}

void MSG_WriteDouble (sizebuf_t *sb, double f)
{
	union
	{
		double	f;
		int64_t l;
	} dat;
	byte *o = SZ_GetSpace (sb, sizeof (f));
	dat.f = f;

	o[0] = dat.l >> 0;
	o[1] = dat.l >> 8;
	o[2] = dat.l >> 16;
	o[3] = dat.l >> 24;
	o[4] = dat.l >> 32;
	o[5] = dat.l >> 40;
	o[6] = dat.l >> 48;
	o[7] = dat.l >> 56;
}

void MSG_WriteString (sizebuf_t *sb, const char *s)
{
	if (!s)
		SZ_Write (sb, "", 1);
	else
		SZ_Write (sb, s, strlen (s) + 1);
}
void MSG_WriteStringUnterminated (sizebuf_t *sb, const char *s)
{
	SZ_Write (sb, s, strlen (s));
}

// johnfitz -- original behavior, 13.3 fixed point coords, max range +-4096
void MSG_WriteCoord16 (sizebuf_t *sb, float f)
{
	MSG_WriteShort (sb, Q_rint (f * 8));
}

// johnfitz -- 16.8 fixed point coords, max range +-32768
void MSG_WriteCoord24 (sizebuf_t *sb, float f)
{
	MSG_WriteShort (sb, f);
	MSG_WriteByte (sb, (int)(f * 255) % 255);
}

// johnfitz -- 32-bit float coords
void MSG_WriteCoord32f (sizebuf_t *sb, float f)
{
	MSG_WriteFloat (sb, f);
}

void MSG_WriteCoord (sizebuf_t *sb, float f, unsigned int flags)
{
	if (flags & PRFL_FLOATCOORD)
		MSG_WriteFloat (sb, f);
	else if (flags & PRFL_INT32COORD)
		MSG_WriteLong (sb, Q_rint (f * 16));
	else if (flags & PRFL_24BITCOORD)
		MSG_WriteCoord24 (sb, f);
	else
		MSG_WriteCoord16 (sb, f);
}

void MSG_WriteAngle (sizebuf_t *sb, float f, unsigned int flags)
{
	if (flags & PRFL_FLOATANGLE)
		MSG_WriteFloat (sb, f);
	else if (flags & PRFL_SHORTANGLE)
		MSG_WriteShort (sb, Q_rint (f * 65536.0 / 360.0) & 65535);
	else
		MSG_WriteByte (sb, Q_rint (f * 256.0 / 360.0) & 255); // johnfitz -- use Q_rint instead of (int)	}
}

// johnfitz -- for PROTOCOL_FITZQUAKE
void MSG_WriteAngle16 (sizebuf_t *sb, float f, unsigned int flags)
{
	if (flags & PRFL_FLOATANGLE)
		MSG_WriteFloat (sb, f);
	else
		MSG_WriteShort (sb, Q_rint (f * 65536.0 / 360.0) & 65535);
}
// johnfitz

// spike -- for PEXT2_REPLACEMENTDELTAS
void MSG_WriteEntity (sizebuf_t *sb, unsigned int entnum, unsigned int pext2)
{
	// high short, low byte
	if (entnum > 0x7fff && (pext2 & PEXT2_REPLACEMENTDELTAS))
	{
		MSG_WriteShort (sb, 0x8000 | (entnum >> 8));
		MSG_WriteByte (sb, entnum & 0xff);
	}
	else
		MSG_WriteShort (sb, entnum);
}

//
// reading functions
//
int		 msg_readcount;
qboolean msg_badread;

void MSG_BeginReading (void)
{
	msg_readcount = 0;
	msg_badread = false;
}

// returns -1 and sets msg_badread if no more characters are available
int MSG_ReadChar (void)
{
	int c;

	if (msg_readcount + 1 > net_message.cursize)
	{
		msg_badread = true;
		return -1;
	}

	c = (signed char)net_message.data[msg_readcount];
	msg_readcount++;

	return c;
}

int MSG_ReadByte (void)
{
	int c;

	if (msg_readcount + 1 > net_message.cursize)
	{
		msg_badread = true;
		return -1;
	}

	c = (unsigned char)net_message.data[msg_readcount];
	msg_readcount++;

	return c;
}

int MSG_ReadShort (void)
{
	int c;

	if (msg_readcount + 2 > net_message.cursize)
	{
		msg_badread = true;
		return -1;
	}

	c = (short)(net_message.data[msg_readcount] + (net_message.data[msg_readcount + 1] << 8));

	msg_readcount += 2;

	return c;
}

int MSG_ReadLong (void)
{
	uint32_t c;

	if (msg_readcount + 4 > net_message.cursize)
	{
		msg_badread = true;
		return -1;
	}

	c = (uint32_t)net_message.data[msg_readcount] + ((uint32_t)(net_message.data[msg_readcount + 1]) << 8) +
		((uint32_t)(net_message.data[msg_readcount + 2]) << 16) + ((uint32_t)(net_message.data[msg_readcount + 3]) << 24);

	msg_readcount += 4;

	return c;
}

unsigned long long MSG_ReadUInt64 (void)
{ // 0* 10*,*, 110*,*,* etc, up to 0xff followed by 8 continuation bytes
	byte			   l = 0x80, v, b = 0;
	unsigned long long r;
	v = MSG_ReadByte ();
	for (; v & l; l >>= 1)
	{
		v -= l;
		b++;
	}
	r = v << (b * 8);
	while (b-- > 0)
		r |= MSG_ReadByte () << (b * 8);
	return r;
}
long long MSG_ReadInt64 (void)
{ // we do some fancy bit recoding for more efficient length coding.
	unsigned long long c = MSG_ReadUInt64 ();
	if (c & 1)
		return -1 - (long long)(c >> 1);
	else
		return (long long)(c >> 1);
}

float MSG_ReadFloat (void)
{
	union
	{
		byte  b[4];
		float f;
		int	  l;
	} dat;

	dat.b[0] = net_message.data[msg_readcount];
	dat.b[1] = net_message.data[msg_readcount + 1];
	dat.b[2] = net_message.data[msg_readcount + 2];
	dat.b[3] = net_message.data[msg_readcount + 3];
	msg_readcount += 4;

	dat.l = LittleLong (dat.l);

	return dat.f;
}
float MSG_ReadDouble (void)
{
	union
	{
		double	 f;
		uint64_t l;
	} dat;

	dat.l = ((uint64_t)net_message.data[msg_readcount] << 0) | ((uint64_t)net_message.data[msg_readcount + 1] << 8) |
			((uint64_t)net_message.data[msg_readcount + 2] << 16) | ((uint64_t)net_message.data[msg_readcount + 3] << 24) |
			((uint64_t)net_message.data[msg_readcount + 4] << 32) | ((uint64_t)net_message.data[msg_readcount + 5] << 40) |
			((uint64_t)net_message.data[msg_readcount + 6] << 48) | ((uint64_t)net_message.data[msg_readcount + 7] << 56);
	msg_readcount += 8;

	return dat.f;
}

const char *MSG_ReadString (void)
{
	static char string[2048];
	int			c;
	size_t		l;

	l = 0;
	do
	{
		c = MSG_ReadByte ();
		if (c == -1 || c == 0)
			break;
		string[l] = c;
		l++;
	} while (l < sizeof (string) - 1);

	string[l] = 0;

	return string;
}

// johnfitz -- original behavior, 13.3 fixed point coords, max range +-4096
float MSG_ReadCoord16 (void)
{
	return MSG_ReadShort () * (1.0 / 8);
}

// johnfitz -- 16.8 fixed point coords, max range +-32768
float MSG_ReadCoord24 (void)
{
	return MSG_ReadShort () + MSG_ReadByte () * (1.0 / 255);
}

// johnfitz -- 32-bit float coords
float MSG_ReadCoord32f (void)
{
	return MSG_ReadFloat ();
}

float MSG_ReadCoord (unsigned int flags)
{
	if (flags & PRFL_FLOATCOORD)
		return MSG_ReadFloat ();
	else if (flags & PRFL_INT32COORD)
		return MSG_ReadLong () * (1.0 / 16.0);
	else if (flags & PRFL_24BITCOORD)
		return MSG_ReadCoord24 ();
	else
		return MSG_ReadCoord16 ();
}

float MSG_ReadAngle (unsigned int flags)
{
	if (flags & PRFL_FLOATANGLE)
		return MSG_ReadFloat ();
	else if (flags & PRFL_SHORTANGLE)
		return MSG_ReadShort () * (360.0 / 65536);
	else
		return MSG_ReadChar () * (360.0 / 256);
}

// johnfitz -- for PROTOCOL_FITZQUAKE
float MSG_ReadAngle16 (unsigned int flags)
{
	if (flags & PRFL_FLOATANGLE)
		return MSG_ReadFloat (); // make sure
	else
		return MSG_ReadShort () * (360.0 / 65536);
}
// johnfitz

unsigned int MSG_ReadEntity (unsigned int pext2)
{
	unsigned int e = (unsigned short)MSG_ReadShort ();
	if (pext2 & PEXT2_REPLACEMENTDELTAS)
	{
		if (e & 0x8000)
		{
			e = (e & 0x7fff) << 8;
			e |= MSG_ReadByte ();
		}
	}
	return e;
}

//===========================================================================

void SZ_Alloc (sizebuf_t *buf, int startsize)
{
	if (startsize < 256)
		startsize = 256;
	buf->data = (byte *)Mem_Alloc (startsize);
	buf->maxsize = startsize;
	buf->cursize = 0;
}

void SZ_Free (sizebuf_t *buf)
{
	Mem_Free (buf->data);
	buf->data = NULL;
	buf->maxsize = 0;
	buf->cursize = 0;
}

void SZ_Clear (sizebuf_t *buf)
{
	buf->cursize = 0;
	buf->overflowed = false;
}

void *SZ_GetSpace (sizebuf_t *buf, int length)
{
	void *data;

	if (buf->cursize + length > buf->maxsize)
	{
		if (!buf->allowoverflow)
			Host_Error ("SZ_GetSpace: overflow without allowoverflow set"); // ericw -- made Host_Error to be less annoying

		if (length > buf->maxsize)
			Sys_Error ("SZ_GetSpace: %i is > full buffer size", length);

		Con_Printf ("SZ_GetSpace: overflow\n");
		SZ_Clear (buf);
		buf->overflowed = true;
	}

	data = buf->data + buf->cursize;
	buf->cursize += length;

	return data;
}

void SZ_Write (sizebuf_t *buf, const void *data, int length)
{
	memcpy (SZ_GetSpace (buf, length), data, length);
}

void SZ_Print (sizebuf_t *buf, const char *data)
{
	int len = strlen (data) + 1;

	if (buf->data[buf->cursize - 1])
	{ /* no trailing 0 */
		memcpy ((byte *)SZ_GetSpace (buf, len), data, len);
	}
	else
	{ /* write over trailing 0 */
		memcpy ((byte *)SZ_GetSpace (buf, len - 1) - 1, data, len);
	}
}

//============================================================================

/*
============
COM_SkipPath
============
*/
const char *COM_SkipPath (const char *pathname)
{
	const char *last;

	last = pathname;
	while (*pathname)
	{
		if (*pathname == '/')
			last = pathname + 1;
		pathname++;
	}
	return last;
}

/*
============
COM_StripExtension
============
*/
void COM_StripExtension (const char *in, char *out, size_t outsize)
{
	int length;

	if (!*in)
	{
		*out = '\0';
		return;
	}
	if (in != out) /* copy when not in-place editing */
		q_strlcpy (out, in, outsize);
	length = (int)strlen (out) - 1;
	while (length > 0 && out[length] != '.')
	{
		--length;
		if (out[length] == '/' || out[length] == '\\')
			return; /* no extension */
	}
	if (length > 0)
		out[length] = '\0';
}

/*
============
COM_FileGetExtension - doesn't return NULL
============
*/
const char *COM_FileGetExtension (const char *in)
{
	const char *src;
	size_t		len;

	len = strlen (in);
	if (len < 2) /* nothing meaningful */
		return "";

	src = in + len - 1;
	while (src != in && src[-1] != '.')
		src--;
	if (src == in || strchr (src, '/') != NULL || strchr (src, '\\') != NULL)
		return ""; /* no extension, or parent directory has a dot */

	return src;
}

/*
============
COM_ExtractExtension
============
*/
void COM_ExtractExtension (const char *in, char *out, size_t outsize)
{
	const char *ext = COM_FileGetExtension (in);
	if (!*ext)
		*out = '\0';
	else
		q_strlcpy (out, ext, outsize);
}

/*
============
COM_FileBase
take 'somedir/otherdir/filename.ext',
write only 'filename' to the output
============
*/
void COM_FileBase (const char *in, char *out, size_t outsize)
{
	const char *dot, *slash, *s;

	s = in;
	slash = in;
	dot = NULL;
	while (*s)
	{
		if (*s == '/' || *s == '\\')
			slash = s + 1;
		if (*s == '.')
			dot = s;
		s++;
	}
	if (dot == NULL)
		dot = s;

	if (dot - slash < 2)
		q_strlcpy (out, "?model?", outsize);
	else
	{
		size_t len = dot - slash;
		if (len >= outsize)
			len = outsize - 1;
		memcpy (out, slash, len);
		out[len] = '\0';
	}
}

/*
==================
COM_DefaultExtension
if path doesn't have a .EXT, append extension
(extension should include the leading ".")
==================
*/
#if 0 /* can be dangerous */
void COM_DefaultExtension (char *path, const char *extension, size_t len)
{
	char	*src;

	if (!*path) return;
	src = path + strlen(path) - 1;

	while (*src != '/' && *src != '\\' && src != path)
	{
		if (*src == '.')
			return; // it has an extension
		src--;
	}

	q_strlcat(path, extension, len);
}
#endif

/*
==================
COM_AddExtension
if path extension doesn't match .EXT, append it
(extension should include the leading ".")
==================
*/
void COM_AddExtension (char *path, const char *extension, size_t len)
{
	if (strcmp (COM_FileGetExtension (path), extension + 1) != 0)
		q_strlcat (path, extension, len);
}

/*
==============
COM_ParseEx

Parse a token out of a string

The mode argument controls how overflow is handled:
- CPE_NOTRUNC:		return NULL (abort parsing)
- CPE_ALLOWTRUNC:	truncate com_token (ignore the extra characters in this token)
==============
*/
const char *COM_ParseEx (const char *data, cpe_mode mode)
{
	int c;
	int len;

	len = 0;
	com_token[0] = 0;

	if (!data)
		return NULL;

// skip whitespace
skipwhite:
	while ((c = *data) <= ' ')
	{
		if (c == 0)
			return NULL; // end of file
		data++;
	}

	// skip // comments
	if (c == '/' && data[1] == '/')
	{
		while (*data && *data != '\n')
			data++;
		goto skipwhite;
	}

	// skip /*..*/ comments
	if (c == '/' && data[1] == '*')
	{
		data += 2;
		while (*data && !(*data == '*' && data[1] == '/'))
			data++;
		if (*data)
			data += 2;
		goto skipwhite;
	}

	// handle quoted strings specially
	if (c == '\"')
	{
		data++;
		while (1)
		{
			if ((c = *data) != 0)
				++data;
			if (c == '\"' || !c)
			{
				com_token[len] = 0;
				return data;
			}
			if (len < countof (com_token) - 1)
				com_token[len++] = c;
			else if (mode == CPE_NOTRUNC)
				return NULL;
		}
	}

	// parse single characters
	if (c == '{' || c == '}' || c == '(' || c == ')' || c == '\'' || c == ':')
	{
		if (len < countof (com_token) - 1)
			com_token[len++] = c;
		else if (mode == CPE_NOTRUNC)
			return NULL;
		com_token[len] = 0;
		return data + 1;
	}

	// parse a regular word
	do
	{
		if (len < countof (com_token) - 1)
			com_token[len++] = c;
		else if (mode == CPE_NOTRUNC)
			return NULL;
		data++;
		c = *data;
		/* commented out the check for ':' so that ip:port works */
		if (c == '{' || c == '}' || c == '(' || c == ')' || c == '\'' /* || c == ':' */)
			break;
	} while (c > 32);

	com_token[len] = 0;
	return data;
}

/*
==============
COM_Parse

Parse a token out of a string

Return NULL in case of overflow
==============
*/
const char *COM_Parse (const char *data)
{
	return COM_ParseEx (data, CPE_NOTRUNC);
}

// savegame parse helpers (Phase 7 material; kept out of the Phase 2
// filesystem port: they wrap sscanf/com_token, not searchpath state)
const char *COM_ParseIntNewline (const char *buffer, int *value)
{
	int consumed = 0;
	sscanf (buffer, "%i\n%n", value, &consumed);
	return buffer + consumed;
}

const char *COM_ParseFloatNewline (const char *buffer, float *value)
{
	int consumed = 0;
	sscanf (buffer, "%f\n%n", value, &consumed);
	return buffer + consumed;
}

const char *COM_ParseStringNewline (const char *buffer)
{
	int consumed = 0;
	com_token[0] = '\0';
	sscanf (buffer, "%1023s\n%n", com_token, &consumed);
	return buffer + consumed;
}

size_t COM_SanitizeDescriptionString (char *dst, size_t dstsize, const char *src, bool remove_color)
{
	int srcpos, dstpos;

	if (!dstsize)
		return 0;

	for (srcpos = dstpos = 0; src[srcpos] && (size_t)dstpos + 1 < dstsize; srcpos++)
	{
		char c = src[srcpos] & (remove_color ? 0x7f : 0xFF); // remove_color

		// When reducing to plain ASCII, also strip control chars: colored glyphs can mask down to
		// scanf whitespace (e.g. 0x8b -> \v), which would split the savegame comment line on load
		if (remove_color && !q_isprint (c))
			c = ' ';
		else if (c == '\n' || c == '\r') // replace newlines with spaces
			c = ' ';
		else if (c == '\\' && src[srcpos + 1] == 'n') // replace '\\' followed by 'n' with space
		{
			c = ' ';
			srcpos++;
		}
		// remove leading spaces, replace consecutive spaces with single one
		if (c != ' ' || (dstpos > 0 && dst[dstpos - 1] != c))
			dst[dstpos++] = c;
	}
	// remove trailing space, if any
	if (dstpos > 0 && dst[dstpos - 1] == ' ')
		--dstpos;

	dst[dstpos] = '\0';
	return dstpos;
}

/*
================
COM_CheckParm

Returns the position (1 to argc-1) in the program's argument list
where the given parameter apears, or 0 if not present
================
*/
int COM_CheckParmNext (int last, const char *parm)
{
	int i;

	for (i = last + 1; i < com_argc; i++)
	{
		if (!com_argv[i])
			continue; // NEXTSTEP sometimes clears appkit vars.
		if (!strcmp (parm, com_argv[i]))
			return i;
	}

	return 0;
}
int COM_CheckParm (const char *parm)
{
	return COM_CheckParmNext (0, parm);
}

/*
================
COM_CheckRegistered

Looks for the pop.txt file and verifies it.
Sets the "registered" cvar.
Immediately exits out if an alternate game was attempted to be started without
being registered.
================
*/
void COM_CheckRegistered (void)
{
	int h;
	int i;

	COM_OpenFile ("gfx/pop.lmp", &h, NULL);

	if (h == -1)
	{
		Cvar_SetROM ("registered", "0");
		Con_Printf ("Playing shareware version.\n");
		if (com_modified)
			Sys_Error (
				"You must have the registered version to use modified games.\n\n"
				"Basedir is: %s\n\n"
				"Check that this has an " GAMENAME " subdirectory containing pak0.pak and pak1.pak, "
				"or use the -basedir command-line option to specify another directory.",
				com_basedir);
		return;
	}

	COM_CloseFile (h);

	for (i = 0; com_cmdline[i]; i++)
	{
		if (com_cmdline[i] != ' ')
			break;
	}

	Cvar_SetROM ("cmdline", &com_cmdline[i]);
	Cvar_SetROM ("registered", "1");
	Con_Printf ("Playing registered version.\n");
}

/*
================
COM_InitArgv
================
*/
void COM_InitArgv (int argc, char **argv)
{
	int i, j, n;

	// reconstitute the command line for the cmdline externally visible cvar
	n = 0;

	for (j = 0; (j < MAX_NUM_ARGVS) && (j < argc); j++)
	{
		i = 0;

		while ((n < (CMDLINE_LENGTH - 1)) && argv[j][i])
		{
			com_cmdline[n++] = argv[j][i++];
		}

		if (n < (CMDLINE_LENGTH - 1))
			com_cmdline[n++] = ' ';
		else
			break;
	}

	if (n > 0 && com_cmdline[n - 1] == ' ')
		com_cmdline[n - 1] = 0; // johnfitz -- kill the trailing space

	Con_Printf ("Command line: %s\n", com_cmdline);

	for (com_argc = 0; (com_argc < MAX_NUM_ARGVS) && (com_argc < argc); com_argc++)
	{
		largv[com_argc] = argv[com_argc];
		if (!strcmp ("-safe", argv[com_argc]))
			safemode = 1;
	}

	largv[com_argc] = argvdummy;
	com_argv = largv;

	if (COM_CheckParm ("-rogue"))
	{
		rogue = true;
		standard_quake = false;
	}

	if (COM_CheckParm ("-hipnotic") || COM_CheckParm ("-quoth")) // johnfitz -- "-quoth" support
	{
		hipnotic = true;
		standard_quake = false;
	}
}

entity_state_t nullentitystate;
static void	   COM_SetupNullState (void)
{
	// the null state has some specific default values
	//	nullentitystate.drawflags = /*SCALE_ORIGIN_ORIGIN*/96;
	nullentitystate.colormod[0] = 32;
	nullentitystate.colormod[1] = 32;
	nullentitystate.colormod[2] = 32;
	//	nullentitystate.glowmod[0] = 32;
	//	nullentitystate.glowmod[1] = 32;
	//	nullentitystate.glowmod[2] = 32;
	nullentitystate.colormap = 0;
	nullentitystate.alpha = ENTALPHA_DEFAULT; // fte has 255 by default, with 0 for invisible. fitz uses 1 for invisible, 0 default, and 255=full alpha
	nullentitystate.scale = ENTSCALE_DEFAULT;
	//	nullentitystate.solidsize = 0;//ES_SOLID_BSP;
	nullentitystate.solidsize = ES_SOLID_NOT;
}

/*
================
COM_WordLength
================
*/
int COM_WordLength (const char *text)
{
	const char *start = text;
	while (*text && !q_isspace (*text))
		text++;
	return text - start;
}

/*
================
COM_AdvanceLineWrapped

Advances text by as much as possible until the maxchars limit is hit,
avoiding splitting words if possible.

Returns the length of the consumed text, excluding a potential trailing space or newline.
================
*/
int COM_AdvanceLineWrapped (const char **text, int maxchars)
{
	const char *str = *text;
	int			i;

	for (i = 0; i < maxchars && str[i]; /**/)
	{
		if (str[i] == '\n')
		{
			*text += i + 1;
			return i;
		}

		// new word
		if (!q_isspace (str[i]) && (i == 0 || q_isspace (str[i - 1])))
		{
			int len = COM_WordLength (str + i);
			// split word if longer than given limit
			if (len > maxchars)
			{
				*text += maxchars;
				return maxchars;
			}
			// not enough space left? push word to next line
			if (i + len > maxchars)
			{
				*text += i;
				return i;
			}
			// word fits, continue
			i += len;
		}
		else
			i++;
	}

	// avoid starting next line with a space
	*text += i + (q_isspace (str[i]) ? 1 : 0);

	return i;
}

/*
================
COM_WordWrap

Copies src to dst by word-wrapping lines longer than maxcols, preserving existing linefeeds.
If maxcols <= 0 no wrapping is performed (plain string copy).
dst is always NUL terminated if dstsize > 0.
================
*/
void COM_WordWrap (char *dst, const char *src, size_t dstsize, int maxcols)
{
	size_t ofs;

	if (maxcols <= 0)
	{
		q_strlcpy (dst, src, dstsize);
		return;
	}

	if (!dstsize)
		return;
	// reserve space for terminating NUL
	--dstsize;

	ofs = 0;
	while (*src)
	{
		const char *start = src;
		size_t		len = (size_t)COM_AdvanceLineWrapped (&src, maxcols);
		size_t		remaining = dstsize - ofs;
		len = q_min (len, remaining);
		memcpy (dst + ofs, start, len);
		ofs += len;
		if (ofs + 1 < dstsize && *src)
			dst[ofs++] = '\n';
	}

	dst[ofs++] = '\0';
}

/*
================
COM_Init
================
*/
void COM_Init (void)
{
	uint32_t uint_value = 0x12345678;
	uint8_t	 bytes[4];
	memcpy (bytes, &uint_value, sizeof (uint32_t));

	/*    U N I X */

	/*
	BE_ORDER:  12 34 56 78
		   U  N  I  X

	LE_ORDER:  78 56 34 12
		   X  I  N  U

	PDP_ORDER: 34 12 78 56
		   N  U  X  I
	*/
	if (bytes[0] != 0x78 || bytes[1] != 0x56 || bytes[2] != 0x34 || bytes[3] != 0x12)
		Sys_Error ("Unsupported endianism. Only little endian is supported");

	if (COM_CheckParm ("-validation"))
		vulkan_globals.validation = true;

	if (COM_CheckParm ("-multiuser"))
		multiuser = true;

	COM_SetupNullState ();
}

/*
============
va

does a varargs printf into a temp buffer. Cycles between
VA_NUM_BUFFS different static buffers.
FIXME: make this buffer size safe someday
============
*/
#define VA_NUM_BUFFS 8
#if (MAX_OSPATH >= 1024)
#define VA_BUFFERLEN MAX_OSPATH
#else
#define VA_BUFFERLEN 1024
#endif

static char *get_va_buffer (void)
{
	static THREAD_LOCAL char va_buffers[VA_NUM_BUFFS][VA_BUFFERLEN];
	static THREAD_LOCAL int	 buffer_idx = 0;
	buffer_idx = (buffer_idx + 1) & (VA_NUM_BUFFS - 1);
	return va_buffers[buffer_idx];
}

char *va (const char *format, ...)
{
	va_list argptr;
	char   *va_buf;

	va_buf = get_va_buffer ();
	va_start (argptr, format);
	q_vsnprintf (va_buf, VA_BUFFERLEN, format, argptr);
	va_end (argptr);

	return va_buf;
}

THREAD_LOCAL qfilesize_t com_filesize;

THREAD_LOCAL int file_from_pak; // ZOID: global indicating that file came from a pak

// Rust migration seam: thread-local globals are not reachable through bindgen
qfileofs_t COM_ThreadFileSize (void)
{
	return com_filesize;
}

int COM_ThreadFileFromPak (void)
{
	return file_from_pak;
}

void COM_SetThreadFileSize (qfilesize_t size)
{
	com_filesize = size;
}

void COM_SetThreadFileFromPak (int from_pak)
{
	file_from_pak = from_pak;
}

const char *COM_ThreadToken (void)
{
	return com_token;
}

// Rust migration seam: quakeparms_t lives outside the bindgen-clean headers
const char *COM_HostBasedir (void)
{
	return host_parms->basedir;
}

const char *COM_HostUserdir (void)
{
	return host_parms->userdir;
}

void COM_SetHostUserdir (const char *dir)
{
	host_parms->userdir = dir;
}

//==============================================================================
// johnfitz -- dynamic gamedir stuff -- modified by QuakeSpasm team.
//==============================================================================
void COM_Game_f (void)
{
	if (Cmd_Argc () > 1)
	{
		int	 i, pri;
		char paths[1024];

		if (!registered.value) // disable shareware quake
		{
			Con_Printf ("You must have the registered version to use modified games\n");
			return;
		}

		*paths = 0;
		q_strlcat (paths, GAMENAME, sizeof (paths));
		for (pri = 0; pri <= 1; pri++)
		{
			for (i = 1; i < Cmd_Argc (); i++)
			{
				const char *p = Cmd_Argv (i);
				if (!*p)
					p = GAMENAME;
				if (pri == 0)
				{
					if (*p != '-')
						continue;
					p++;
				}
				else if (*p == '-')
					continue;

				if (COM_ModForbiddenChars (p))
				{
					Con_Printf ("gamedir should be a single directory name, not a path\n");
					return;
				}

				if (!q_strcasecmp (p, GAMENAME))
					continue; // don't add id1, its not interesting enough.

				if (*paths)
					q_strlcat (paths, ";", sizeof (paths));
				q_strlcat (paths, p, sizeof (paths));
			}
		}

		if (!q_strcasecmp (paths, COM_GetGameNames (true)))
		{
			Con_Printf ("\"game\" is already \"%s\"\n", COM_GetGameNames (true));
			return;
		}

		com_modified = true;

		// Kill the server
		CL_Disconnect ();
		cls.demonum = -1;
		Host_ShutdownServer (true);

		SCR_CenterPrintClear ();

		// Write config file
		Host_WriteConfiguration ();

		// stop parsing map files before changing file system search paths
		ExtraMaps_Clear ();
		LOC_Shutdown ();

		COM_ResetGameDirectories (paths);

		// clear out and reload appropriate data; the renderer reload is
		// guarded by the same flag that gated its init in Host_Init, so a
		// headless (-headless harness) client can switch games too
		Mod_ResetAll ();
		Sky_ClearAll ();
		if (!no_rendering)
		{
			TexMgr_NewGame ();
			Draw_NewGame ();
			R_NewGame ();
		}
		if (!isDedicated)
			M_NewGame ();
		ExtraMaps_NewGame ();
		Host_Resetdemos ();
		DemoList_Rebuild ();
		SaveList_Rebuild ();
		M_CheckMods ();
		S_ClearAll ();

		Con_Printf ("\"game\" changed to \"%s\"\n", COM_GetGameNames (true));

		LOC_Init ();
		VID_Lock ();
		Cbuf_AddText ("exec quake.rc\n");
		Cbuf_AddText ("vid_unlock\n");
	}
	else // Diplay the current gamedir
		Con_Printf ("\"game\" is \"%s\"\n", COM_GetGameNames (true));
}
/*
================
Initial state picked randomly, can't be 0.
================
*/
static uint32_t xorshiro_state[2] = {0xcdb38550, 0x720a8392};

/*
=================
COM_SeedRand
=================
*/
void COM_SeedRand (uint64_t seed)
{
	// SplitMix64
	uint64_t z = (seed + 0x9e3779b97f4a7c15);
	z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9;
	z = (z ^ (z >> 27)) * 0x94d049bb133111eb;
	uint64_t state = z ^ (z >> 31);
	xorshiro_state[0] = (uint32_t)state;
	xorshiro_state[1] = (uint32_t)(state >> 32);
}

/*
=================
COM_Rand
=================
*/
static inline uint32_t rotl (const uint32_t x, int k)
{
	return (x << k) | (x >> (32 - k));
}

/*
=================
COM_RandState

Exposes the RNG state for the verification harness state hash.
=================
*/
void COM_RandState (uint32_t state[2])
{
	state[0] = xorshiro_state[0];
	state[1] = xorshiro_state[1];
}

int32_t COM_Rand ()
{
	// Xorshiro64**
	const uint32_t s0 = xorshiro_state[0];
	uint32_t	   s1 = xorshiro_state[1];
	const uint32_t result = rotl (s0 * 0x9E3779BB, 5) * 5;
	s1 ^= s0;
	xorshiro_state[0] = rotl (s0, 26) ^ s1 ^ (s1 << 9);
	xorshiro_state[1] = rotl (s1, 13);

	return (int32_t)(result & COM_RAND_MAX);
}

void COM_Assert_Failed (const char *expr, const char *file_path, int line)
{
	// only keep the simple file name, strip the directory part
	// we only want the short file name, not the full path:
	const char *last_sep = FIND_LAST_DIRSEP (file_path);

	const char *filename = (last_sep ? last_sep + 1 : file_path);

	if (Tasks_IsWorker ())
	{
		Sys_DebugBreak ();

		if (!Sys_IsInDebugger ())
		{
			const char *captured_stack_trace = Sys_StackTrace ();

			char *msg = q_strcatf (NULL, "%s:%d Assertion: '%s' failed\nSTACK TRACE:\n%s", filename, line, expr, captured_stack_trace);

			Mem_Free (captured_stack_trace);

			Sys_Printf ("%s\n", msg);
#if defined(_WIN32)
			// Only the Win32 MessageBox can safely be called from any thread.
			PL_ErrorDialog (msg);
#endif
			Mem_Free (msg);
		}
		abort ();
	}
	else // We are in the main thread, console is accessible, do Host_Error and we can recover.
		Host_Error ("%s:%d Assertion: '%s' failed", filename, line, expr);
}
