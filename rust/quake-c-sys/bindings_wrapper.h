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
#include "steam.h"

/* console.h is not a bindgen-clean root (it needs quakeparms_t from
 * quakedef.h), so the console functions called from Rust are declared
 * directly; signatures must match console.h exactly. */
void Con_Printf (const char *fmt, ...) FUNC_PRINTF (1, 2);
void Con_Warning (const char *fmt, ...) FUNC_PRINTF (1, 2);
void Con_DPrintf (const char *fmt, ...) FUNC_PRINTF (1, 2);
void Con_DPrintf2 (const char *fmt, ...) FUNC_PRINTF (1, 2);

/* cmd.h is not a bindgen-clean root; these must match cmd.h exactly.
 * Cmd_AddCommand is a macro over Cmd_AddCommand2 (cmdname, func,
 * src_command, false) there; the Rust shims call Cmd_AddCommand2 the same
 * way. */
typedef void (*xcommand_t) (void);
typedef enum
{
	src_client,
	src_command,
	src_server
} cmd_source_t;
typedef struct cmd_function_s cmd_function_t;
cmd_function_t *Cmd_AddCommand2 (const char *cmd_name, xcommand_t function, cmd_source_t srctype, qboolean qcinterceptable);

/* Sys_SelectFolder is #ifdef USE_SDL3 in sys.h; the declaration is always
 * bound, the Rust shim only calls it under its sdl3 cargo feature (set by
 * Meson alongside the C define). Must match sys.h exactly. */
int Sys_SelectFolder (const char *title, const char *default_location, char *dst, size_t dstsize);

/* image.h is not a bindgen-clean root (enum srcformat lives in the
 * Vulkan-bearing gl_texmgr.h), so the M8 in-memory stb fallback decoder is
 * declared directly; must match image.h exactly. */
byte *Image_DecodeSTBMem (const byte *mem, int len, int *width, int *height, const char **failure_reason);

/* engine globals living in headers that are not bindgen-clean roots
 * (quakedef.h, harness.h); declarations must match those headers exactly. */
extern qboolean multiuser;	  /* quakedef.h */
extern qboolean isDedicated;  /* quakedef.h */
extern cvar_t	developer;	  /* quakedef.h */
extern qboolean harness_active; /* harness.h */
extern cvar_t	external_ents;	  /* gl_model.c */

/* embedded pak (generated embedded_pak.c) */
extern const unsigned char vkquake_pak[];
extern const int		   vkquake_pak_size;
extern const int		   vkquake_pak_decompressed_size;
