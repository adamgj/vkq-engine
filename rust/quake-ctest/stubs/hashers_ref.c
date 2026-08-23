/* Non-inline wrappers around hash_map.h's static-inline hashers so the
 * differential tests can call the reference implementations. The force-
 * included prelude supplies q_types.h; hash_map.h's HashMap_* declarations
 * are renamed by the same prelude, which is fine for these wrappers. */

#include <string.h>
/* hash_map.h has no include guard and the prelude now includes it (Phase 3 M5
 * needs it for MD5_ComputeNormals), so including it again here would redefine
 * the static inlines. */

uint32_t c_ref_HashInt32 (const void *const val)
{
	return HashInt32 (val);
}

uint32_t c_ref_HashInt64 (const void *const val)
{
	return HashInt64 (val);
}

uint32_t c_ref_HashFloat (const void *const val)
{
	return HashFloat (val);
}

uint32_t c_ref_HashCombine (uint32_t a, uint32_t b)
{
	return HashCombine (a, b);
}

uint32_t c_ref_HashVec2 (const void *const val)
{
	return HashVec2 (val);
}

uint32_t c_ref_HashVec3 (const void *const val)
{
	return HashVec3 (val);
}

uint32_t c_ref_HashStr (const void *const val)
{
	return HashStr (val);
}

qboolean c_ref_HashStrCmp (const void *const a, const void *const b)
{
	return HashStrCmp (a, b);
}
