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
#include "q_thread.h"
#include "q_sound.h"
#include "harness.h"

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

/* Phase 4 sound: cvars declared in snd_dma.c/snd_glue.c rather than a
 * header, and engine globals from quakedef.h (not a bindgen-clean root);
 * declarations must match the defining files exactly. */
extern cvar_t snd_waterfx;		/* snd_dma.c / snd_glue.c */
extern cvar_t snd_pauselooping;
extern cvar_t precache;
extern dma_t  sn;				/* snd_dma.c / snd_glue.c storage for S_Startup */
extern double host_frametime;	/* quakedef.h */
extern int	  host_framecount;	/* quakedef.h */

/* cmd.h / console.h are not bindgen-clean roots; must match those headers */
int			Cmd_Argc (void);
const char *Cmd_Argv (int arg);
void		Con_SafePrintf (const char *fmt, ...) FUNC_PRINTF (1, 2);

/* Phase 4 M7: the codec framework types and the C decoder wrappers' vtable
 * statics. snd_codec.h/snd_codeci.h are bindgen-clean (fshandle_t comes
 * from common.h above). The *_codec statics are declared in per-codec
 * headers behind USE_CODEC_* defines; declared here unconditionally -- the
 * Rust registry references each only under its codec-* cargo feature. */
#include "snd_codec.h"
#include "snd_codeci.h"
extern snd_codec_t flac_codec;
extern snd_codec_t mp3_codec;
extern snd_codec_t vorbis_codec;
extern snd_codec_t opus_codec;
/* common.h helpers the codec framework uses */
const char *COM_FileGetExtension (const char *in);

/* snd_glue.c accessors (compiled only under -Duse_rust_snd): the
 * client/server state snd_dma.c read directly. The real return types are
 * pointers to qmodel_t and mleaf_t (gl_model.h, not bindgen-clean); only
 * the address is passed through, and the Rust side views mleaf_t via its
 * quake-types mirror. */
qboolean SND_Glue_ClientConnected (void);
int		 SND_Glue_ViewEntity (void);
void	*SND_Glue_Worldmodel (void);
void	*SND_Glue_PointInLeaf (float *p);
qboolean SND_Glue_PauseLoops (void);

/* Phase 5 net: net.h/net_defs.h are not bindgen-clean roots (net_sys.h pulls
 * the system socket headers), so the wire-layer globals and the qsocket-pool
 * entry points are declared directly; must match net.h/net_defs.h exactly.
 * qsocket_t stays opaque here -- the Rust side views it through the
 * hand-written quake-types::net mirror (ADR-011). */
extern sizebuf_t net_message;	 /* net.h */
extern double	 net_time;		 /* net.h */
extern int		 net_driverlevel; /* net_defs.h */
struct qsocket_s *NET_NewQSocket (void);
void			  NET_FreeQSocket (struct qsocket_s *sock);
double			  SetNetTime (void);

/* Phase 5 M7: the dgrm shared statics (net_dgrm_glue.c under USE_RUST_NET,
 * net_dgrm_rel.c otherwise) and the net_main message counters the reliable
 * layer increments; must match net_dgrm_int.h / net_defs.h exactly */
typedef struct
{
	unsigned int  length;
	unsigned int  sequence;
	unsigned char data[64000]; /* MAX_DATAGRAM */
} dgrm_packet_t;
extern dgrm_packet_t packetBuffer;
extern int			 packetsSent;
extern int			 packetsReSent;
extern int			 packetsReceived;
extern int			 receivedDuplicateCount;
extern int			 shortPacketCount;
extern int			 droppedDatagrams;
extern int			 messagesReceived;
extern int			 unreliableMessagesReceived;

/* Phase 5 M7b: the UDP landriver shims' engine globals (net.h) */
extern int		net_hostport;
extern char		my_ipv4_address[64]; /* NET_NAMELEN */
extern char		my_ipv6_address[64];
extern qboolean ipv4Available;
extern qboolean ipv6Available;
double			Sys_DoubleTime (void);

/* Phase 5 M9: the net_main.c core the Rust port reads/writes (net.h /
 * net_defs.h; qsocket_t stays opaque -- ADR-011 mirror) plus the
 * svs/sv accessor funnels net_main.c defines under USE_RUST_NET */
extern struct qsocket_s *net_activeSockets;
extern struct qsocket_s *net_freeSockets;
extern int				 net_activeconnections;
extern int				 DEFAULTnet_hostport;
extern qboolean			 listening;
extern size_t			 hostCacheCount;
qboolean				 NetMain_SVActive (void);
int						 NetMain_MaxClients (void);
int						 NetMain_MaxClientsLimit (void);
void					 NetMain_SetMaxClients (int n);
void					 Cbuf_AddText (const char *text);

/* embedded pak (generated embedded_pak.c) */
extern const unsigned char vkquake_pak[];
extern const int		   vkquake_pak_size;
extern const int		   vkquake_pak_decompressed_size;
