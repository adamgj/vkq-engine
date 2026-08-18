/* Input header for scripts/gen_c_bindings.sh (bindgen CLI).
 *
 * Only the symbols allowlisted in that script are bound; the committed output
 * is rust/quake-c-sys/src/generated.rs, and CI regenerates + diffs it
 * (rust.yml bindgen-smoke job). All headers here are Phase 0
 * "bindgen-clean" roots or depend only on q_types.h. */

#include "q_types.h"
#include "mem.h"
#include "common.h"
#include "sys.h"
#include "cvar.h"

/* console.h is not a bindgen-clean root (it needs quakeparms_t from
 * quakedef.h), so the console functions called from Rust are declared
 * directly; signatures must match console.h exactly. */
void Con_Printf (const char *fmt, ...) FUNC_PRINTF (1, 2);
void Con_Warning (const char *fmt, ...) FUNC_PRINTF (1, 2);
void Con_DPrintf (const char *fmt, ...) FUNC_PRINTF (1, 2);
void Con_DPrintf2 (const char *fmt, ...) FUNC_PRINTF (1, 2);
