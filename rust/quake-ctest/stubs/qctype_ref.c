/* Non-inline wrappers around q_ctype.h's static-inline functions so the
 * differential tests can call the reference implementations. */

#include "q_ctype.h"

#define WRAP(name)                       \
	int c_ref_##name (int c)             \
	{                                    \
		return name (c);                 \
	}

WRAP (q_isascii)
WRAP (q_islower)
WRAP (q_isupper)
WRAP (q_isalpha)
WRAP (q_isdigit)
WRAP (q_isxdigit)
WRAP (q_isalnum)
WRAP (q_isblank)
WRAP (q_isspace)
WRAP (q_isgraph)
WRAP (q_isprint)
WRAP (q_toascii)
WRAP (q_tolower)
WRAP (q_toupper)
