/* Phase 7 M7 oracle fixture TU for Quake/cl_demo.c (T7.4).
 *
 * Same three roles as cl_main_ref.c, with one structural difference worth
 * stating up front: cl_demo.c owns NO file-scope object with external
 * linkage (the shared `name` buffer at cl_demo.c:37 and the alt record buffer
 * at :584 are both static, and became Rust statics), so this file defines no
 * storage twin at all. What it owns is the 13 ClDemo_Glue_* trampolines, the
 * plain entry points, and the demo-file fixtures.
 *
 * The link probe (`cargo test -p quake-ctest --no-run`) reported exactly 18
 * unresolved externals for this half: the 13 trampolines plus CL_Record_f,
 * CL_Stop_f, CL_PlayDemo_f, CL_TimeDemo_f and CL_Seek_f. CL_StopPlayback,
 * CL_GetMessage and CL_Resume_Record were NOT unresolved -- nothing plain in
 * the link references them -- but they are defined here anyway so the drivers
 * can enter through the same re-raising frame Quake/cl_demo_glue.c installs
 * rather than reaching around it to the core.
 *
 * CALLEE SELECTION, by the same rule cl_main_ref.c states:
 *   plain twin taken (#undef'd):   net_message (sv_user_ref.c:80),
 *                                  CL_Disconnect (cl_input_ref.c),
 *                                  Cmd_ExecuteString (stubs.c),
 *                                  S_StopAllSounds, V_ResetBlend (view_ref.c)
 *   oracle/shared copy (renamed,   MSG_WriteStaticOrBaseLine, S_LoadSound,
 *   no plain twin exists):         COM_FOpenFile
 *   single shared stub (unrenamed) NET_GetMessage, BGM_Stop, DemoList_Rebuild,
 *                                  Fog_NewMap, Fog_GetFogCommand, Sky_NewMap,
 *                                  Sky_GetSkyCommand, R_ClearParticles,
 *                                  PScript_ClearParticles, SCR_CenterPrintClear
 *
 * MSG_Write* again has neither shape (no plain MSG_WriteByte exists in this
 * link), so ClDemo_Glue_WriteBatch drives quake-capi's quake_rs_msg_write_*
 * status cores against the PLAIN net_message and Host_Reraise's a non-zero
 * status from inside the guard -- the same thing Quake/net_msg_glue.c does
 * for the real build.
 *
 * COVERAGE CEILING, stated so nothing here reads as more than it is:
 *   - CL_Record_f and CL_PlayDemo_f past their argument checks reach
 *     COM_FOpenFile / Sys_fopen and then the signon-replay writers; both
 *     sides stop at the same shared stub, so what is compared is the
 *     arguments and the state mutated before it, not the file that would have
 *     been produced.
 *   - CL_StopPlayback's tail reaches stubs.c:7595 Harness_DemoEnded, which
 *     Sys_Errors. That is a real comparison (both sides must arrive there
 *     with identical cls state and the identical guard message) but it does
 *     end the call, so nothing after Harness_DemoEnded is exercised. Nothing
 *     follows it in cl_demo.c:59-76.
 *   - USE_RUST_NET is OFF for the oracle C but quake-capi is linked with its
 *     `net` feature ON, so the demo record-header path compares the Rust
 *     16-byte atomic reader against C's field-at-a-time reader. cl_demo.c:150
 *     documents these as deliberately divergent on a demo truncated 4-15
 *     bytes into a record header. The fixtures below therefore feed only
 *     well-formed records and a cleanly-truncated (zero-byte) tail; the
 *     partial-header case is a KNOWN, DOCUMENTED gap and is not tested.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

extern int	Host_Guard (void (*fn) (void *), void *arg);
extern void Host_Reraise (int guard_result);

/* quake-capi/src/cl_demo.rs */
extern void quake_rs_cl_stop_playback (void);
extern int	quake_rs_cl_seek_f (void);
extern int	quake_rs_cl_get_message (int *out);
extern int	quake_rs_cl_stop_f (void);
extern int	quake_rs_cl_record_f (void);
extern int	quake_rs_cl_resume_record (qboolean recordsignons);
extern int	quake_rs_cl_play_demo_f (void);
extern int	quake_rs_cl_time_demo_f (void);

/* quake-capi/src/net.rs */
extern int quake_rs_msg_write_byte (sizebuf_t *sb, int v);
extern int quake_rs_msg_write_short (sizebuf_t *sb, int v);
extern int quake_rs_msg_write_long (sizebuf_t *sb, int v);
extern int quake_rs_msg_write_float (sizebuf_t *sb, float v);
extern int quake_rs_msg_write_string (sizebuf_t *sb, const char *s);
extern int quake_rs_msg_write_coord (sizebuf_t *sb, float v, unsigned int flags);

/* Shared stubs with no declaration reaching this TU. scr_clock_off is a
   header-less external -- cl_demo.c:213 declares it by hand, as does
   quake-c-sys/src/cl_demo.rs. It is NOT renamed by the prelude, so both
   sides read the one object. */
extern void	 BGM_Stop (void);
extern float scr_clock_off;

/* Renamed callees whose plain twin this file must reach. */
#undef net_message
#undef CL_Disconnect
#undef Cmd_ExecuteString
#undef S_StopAllSounds
#undef V_ResetBlend
#undef cmd_source
extern cmd_source_t cmd_source;
extern cmd_source_t c_ref_cmd_source;

/* cl and cls exist twice from T7.4 on: quake-capi owns the plain pair and
   cl_main.c owns c_ref_cl / c_ref_cls (ADR-007). Without these two #undefs
   every bare `cls` below would expand to c_ref_cls and each seeder would
   write the oracle copy twice, leaving the port looking at a demofile-less,
   demoplayback-false client -- the whole suite would then compare
   CL_GetDemoMessage against NET_GetMessage. */
#undef cl
#undef cls
extern client_state_t  cl;
extern client_static_t cls;
extern sizebuf_t net_message;
extern void		 CL_Disconnect (void);
extern qboolean Cmd_ExecuteString (const char *text, cmd_source_t src);
extern void		 S_StopAllSounds (qboolean clear, qboolean stopmusic);
extern void		 V_ResetBlend (void);

/* Plain entry points defined below. client.h declares them, but under the
   renamed spelling, so each needs a fresh prototype after its #undef. */
#undef CL_StopPlayback
#undef CL_GetMessage
#undef CL_Resume_Record
#undef CL_Seek_f
#undef CL_Stop_f
#undef CL_Record_f
#undef CL_PlayDemo_f
#undef CL_TimeDemo_f
void CL_StopPlayback (void);
int	 CL_GetMessage (void);
void CL_Resume_Record (qboolean recordsignons);
void CL_Seek_f (void);
void CL_Stop_f (void);
void CL_Record_f (void);
void CL_PlayDemo_f (void);
void CL_TimeDemo_f (void);

/* ==========================================================================
 * 1. The 13 trampolines, mirroring Quake/cl_demo_glue.c body for body.
 */

typedef struct
{
	int			kind;
	int			i;
	float		f;
	unsigned	u;
	const void *p;
} cldemo_write_t;

typedef struct
{
	const cldemo_write_t *ops;
	int					  count;
} cldemo_writebatch_arg_t;

static void ClDemo_InvokeWriteBatch (void *p)
{
	cldemo_writebatch_arg_t *a = (cldemo_writebatch_arg_t *)p;
	int						 k;

	for (k = 0; k < a->count; k++)
	{
		const cldemo_write_t *op = &a->ops[k];
		switch (op->kind)
		{
		case 0:
			Host_Reraise (quake_rs_msg_write_byte (&net_message, op->i));
			break;
		case 1:
			Host_Reraise (quake_rs_msg_write_short (&net_message, op->i));
			break;
		case 2:
			Host_Reraise (quake_rs_msg_write_long (&net_message, op->i));
			break;
		case 3:
			Host_Reraise (quake_rs_msg_write_float (&net_message, op->f));
			break;
		case 4:
			Host_Reraise (quake_rs_msg_write_string (&net_message, (const char *)op->p));
			break;
		case 5:
			Host_Reraise (quake_rs_msg_write_coord (&net_message, op->f, op->u));
			break;
		default:
			Sys_Error ("ClDemo_InvokeWriteBatch: unknown op %i", op->kind);
			break;
		}
	}
}

int ClDemo_Glue_WriteBatch (const cldemo_write_t *ops, int count)
{
	cldemo_writebatch_arg_t arg;
	arg.ops = ops;
	arg.count = count;
	return Host_Guard (ClDemo_InvokeWriteBatch, &arg);
}

typedef struct
{
	int			 idx;
	void		*state;
	unsigned int pext2;
	unsigned int protocol;
	unsigned int protocolflags;
} cldemo_baseline_arg_t;

static void ClDemo_InvokeStaticOrBaseLine (void *p)
{
	cldemo_baseline_arg_t *a = (cldemo_baseline_arg_t *)p;
	MSG_WriteStaticOrBaseLine (&net_message, a->idx, (struct entity_state_s *)a->state, a->pext2, a->protocol, a->protocolflags);
}

int ClDemo_Glue_WriteStaticOrBaseLine (int idx, void *state, unsigned int pext2, unsigned int protocol, unsigned int protocolflags)
{
	cldemo_baseline_arg_t arg;
	arg.idx = idx;
	arg.state = state;
	arg.pext2 = pext2;
	arg.protocol = protocol;
	arg.protocolflags = protocolflags;
	return Host_Guard (ClDemo_InvokeStaticOrBaseLine, &arg);
}

typedef struct
{
	struct qsocket_s *sock;
	int				 *out;
} cldemo_netget_arg_t;

static void ClDemo_InvokeNetGetMessage (void *p)
{
	cldemo_netget_arg_t *a = (cldemo_netget_arg_t *)p;
	*a->out = NET_GetMessage (a->sock);
}

int ClDemo_Glue_NetGetMessage (struct qsocket_s *sock, int *out)
{
	cldemo_netget_arg_t arg;
	arg.sock = sock;
	arg.out = out;
	*out = 0;
	return Host_Guard (ClDemo_InvokeNetGetMessage, &arg);
}

typedef struct
{
	const char *text;
	int			src;
} cldemo_exec_arg_t;

static void ClDemo_InvokeCmdExecuteString (void *p)
{
	cldemo_exec_arg_t *a = (cldemo_exec_arg_t *)p;
	Cmd_ExecuteString (a->text, (cmd_source_t)a->src);
}

int ClDemo_Glue_CmdExecuteString (const char *text, int src)
{
	cldemo_exec_arg_t arg;
	arg.text = text;
	arg.src = src;
	return Host_Guard (ClDemo_InvokeCmdExecuteString, &arg);
}

static void ClDemo_InvokeDisconnect (void *p)
{
	(void)p;
	CL_Disconnect ();
}

int ClDemo_Glue_Disconnect (void)
{
	return Host_Guard (ClDemo_InvokeDisconnect, NULL);
}

static void ClDemo_InvokeSeekEffects (void *p)
{
	(void)p;
	V_ResetBlend ();
	Fog_NewMap ();
	Sky_NewMap ();
	R_ClearParticles ();
#ifdef PSET_SCRIPT
	PScript_ClearParticles (false);
#endif
	SCR_CenterPrintClear ();
}

int ClDemo_Glue_SeekEffects (void)
{
	return Host_Guard (ClDemo_InvokeSeekEffects, NULL);
}

static void ClDemo_InvokeBgmStop (void *p)
{
	(void)p;
	BGM_Stop ();
}

int ClDemo_Glue_BgmStop (void)
{
	return Host_Guard (ClDemo_InvokeBgmStop, NULL);
}

static void ClDemo_InvokeStopAllSounds (void *p)
{
	(void)p;
	S_StopAllSounds (true, true);
}

int ClDemo_Glue_StopAllSounds (void)
{
	return Host_Guard (ClDemo_InvokeStopAllSounds, NULL);
}

static void ClDemo_InvokeDemoListRebuild (void *p)
{
	(void)p;
	DemoList_Rebuild ();
}

int ClDemo_Glue_DemoListRebuild (void)
{
	return Host_Guard (ClDemo_InvokeDemoListRebuild, NULL);
}

typedef struct
{
	void  *sfx;
	void **out;
} cldemo_loadsound_arg_t;

static void ClDemo_InvokeLoadSound (void *p)
{
	cldemo_loadsound_arg_t *a = (cldemo_loadsound_arg_t *)p;
	*a->out = S_LoadSound ((sfx_t *)a->sfx);
}

int ClDemo_Glue_LoadSound (void *sfx, void **out)
{
	cldemo_loadsound_arg_t arg;
	arg.sfx = sfx;
	arg.out = out;
	*out = NULL;
	return Host_Guard (ClDemo_InvokeLoadSound, &arg);
}

static void ClDemo_InvokeFogGetFogCommand (void *p)
{
	*(const char **)p = Fog_GetFogCommand (false);
}

int ClDemo_Glue_FogGetFogCommand (const char **out)
{
	*out = NULL;
	return Host_Guard (ClDemo_InvokeFogGetFogCommand, out);
}

static void ClDemo_InvokeSkyGetSkyCommand (void *p)
{
	*(const char **)p = Sky_GetSkyCommand (false);
}

int ClDemo_Glue_SkyGetSkyCommand (const char **out)
{
	*out = NULL;
	return Host_Guard (ClDemo_InvokeSkyGetSkyCommand, out);
}

typedef struct
{
	const char *name;
	FILE	  **file;
} cldemo_fopen_arg_t;

static void ClDemo_InvokeComFOpenFile (void *p)
{
	cldemo_fopen_arg_t *a = (cldemo_fopen_arg_t *)p;
	COM_FOpenFile (a->name, a->file, NULL);
}

int ClDemo_Glue_ComFOpenFile (const char *name, FILE **file)
{
	cldemo_fopen_arg_t arg;
	arg.name = name;
	arg.file = file;
	return Host_Guard (ClDemo_InvokeComFOpenFile, &arg);
}

/* ==========================================================================
 * 2. Re-raising plain entry points (Quake/cl_demo_glue.c:351-405).
 */

void CL_StopPlayback (void)
{
	quake_rs_cl_stop_playback ();
}

void CL_Seek_f (void)
{
	Host_Reraise (quake_rs_cl_seek_f ());
}

int CL_GetMessage (void)
{
	int out = 0;
	int r = quake_rs_cl_get_message (&out);
	Host_Reraise (r);
	return out;
}

void CL_Stop_f (void)
{
	Host_Reraise (quake_rs_cl_stop_f ());
}

void CL_Record_f (void)
{
	Host_Reraise (quake_rs_cl_record_f ());
}

void CL_Resume_Record (qboolean recordsignons)
{
	Host_Reraise (quake_rs_cl_resume_record (recordsignons));
}

void CL_PlayDemo_f (void)
{
	Host_Reraise (quake_rs_cl_play_demo_f ());
}

void CL_TimeDemo_f (void)
{
	Host_Reraise (quake_rs_cl_time_demo_f ());
}

/* ==========================================================================
 * 3. Fixtures.
 *
 * cl/cls are seeded and read back through cl_main_ref.c's helpers (same test
 * binary, same two objects), so only what is specific to cl_demo lives here:
 * the two net_message buffers and the two demo files.
 */

extern client_state_t  c_ref_cl;
extern client_static_t c_ref_cls;

static byte ctest_cldemo_msgbuf[MAX_MSGLEN];
static byte ctest_cldemo_oracle_msgbuf[MAX_MSGLEN];

void ctest_cldemo_attach_message (void)
{
	memset (ctest_cldemo_msgbuf, 0, sizeof (ctest_cldemo_msgbuf));
	memset (ctest_cldemo_oracle_msgbuf, 0, sizeof (ctest_cldemo_oracle_msgbuf));

	net_message.data = ctest_cldemo_msgbuf;
	net_message.maxsize = MAX_MSGLEN;
	net_message.cursize = 0;
	net_message.allowoverflow = false;
	net_message.overflowed = false;

	c_ref_net_message.data = ctest_cldemo_oracle_msgbuf;
	c_ref_net_message.maxsize = MAX_MSGLEN;
	c_ref_net_message.cursize = 0;
	c_ref_net_message.allowoverflow = false;
	c_ref_net_message.overflowed = false;
}

int ctest_cldemo_message_size (int side)
{
	return side ? c_ref_net_message.cursize : net_message.cursize;
}

const unsigned char *ctest_cldemo_message_data (int side)
{
	return side ? ctest_cldemo_oracle_msgbuf : ctest_cldemo_msgbuf;
}

/* The two sides need INDEPENDENT stdio handles, so two real files are used
 * rather than one file opened twice: cl_demo.c both reads and (for the record
 * paths) writes through cls.demofile, and a shared handle would let one
 * side's file position leak into the other's read. tmpfile() is avoided
 * deliberately -- on Windows the CRT places it in the volume root and it
 * fails without elevation. */
static FILE *ctest_cldemo_files[2];
static char	 ctest_cldemo_paths[2][512];

static void ctest_cldemo_close (int side)
{
	if (ctest_cldemo_files[side])
	{
		fclose (ctest_cldemo_files[side]);
		ctest_cldemo_files[side] = NULL;
	}
	if (ctest_cldemo_paths[side][0])
	{
		remove (ctest_cldemo_paths[side]);
		ctest_cldemo_paths[side][0] = '\0';
	}
}

static FILE *ctest_cldemo_open (int side, const unsigned char *data, int len)
{
	const char *dir = getenv ("TEMP");
	FILE	   *f;

	ctest_cldemo_close (side);
	if (!dir || !*dir)
		dir = getenv ("TMPDIR");
	if (!dir || !*dir)
		dir = ".";

	q_snprintf (ctest_cldemo_paths[side], sizeof (ctest_cldemo_paths[side]), "%s/ctest_cldemo_%d.dem", dir, side);
	f = fopen (ctest_cldemo_paths[side], "wb+");
	if (!f)
		Sys_Error ("ctest_cldemo_open: cannot create %s", ctest_cldemo_paths[side]);

	if (len > 0)
		fwrite (data, 1, (size_t)len, f);
	fflush (f);
	rewind (f);
	ctest_cldemo_files[side] = f;
	return f;
}

/* Gives BOTH sides their own handle on identical bytes and marks the client
 * as playing back. Returns 0; the caller reads state through cl_main_ref.c's
 * cls image. */
void ctest_cldemo_attach_demo (const unsigned char *data, int len)
{
	cls.demofile = ctest_cldemo_open (0, data, len);
	c_ref_cls.demofile = ctest_cldemo_open (1, data, len);
	cls.demoplayback = c_ref_cls.demoplayback = true;
}

/* Detaches without closing through the port, for tests that must not run
 * CL_StopPlayback's fclose twice. */
void ctest_cldemo_release_demo (void)
{
	cls.demofile = NULL;
	c_ref_cls.demofile = NULL;
	ctest_cldemo_files[0] = NULL; /* the port may already have fclose'd it */
	ctest_cldemo_files[1] = NULL;
	ctest_cldemo_close (0);
	ctest_cldemo_close (1);
}

/* Byte offset the two handles currently sit at, so a read that consumed the
 * same number of bytes on both sides is provable rather than assumed. */
long ctest_cldemo_file_pos (int side)
{
	FILE *f = side ? c_ref_cls.demofile : cls.demofile;
	return f ? ftell (f) : -1L;
}

/* cmd_source has TWO copies in this link -- stubs.c owns the plain one the
   port reads and cmd.c owns c_ref_cmd_source -- so both must be set.
   Writing only one makes every cmd_source guard in cl_demo.c compare a
   different value on each side. */
void ctest_cldemo_set_cmd_source (int src)
{
	cmd_source = (cmd_source_t)src;
	c_ref_cmd_source = (cmd_source_t)src;
}

void ctest_cldemo_set_demo_state (int demoplayback, int demorecording, int demopaused, int demoseeking, int signon)
{
	int i;
	for (i = 0; i < 2; i++)
	{
		client_static_t *s = i ? &c_ref_cls : &cls;
		s->demoplayback = demoplayback ? true : false;
		s->demorecording = demorecording ? true : false;
		s->demopaused = demopaused ? true : false;
		s->demoseeking = demoseeking ? true : false;
		s->signon = signon;
	}
}

void ctest_cldemo_set_seek_fields (float demospeed, int prespawn_end, float seektime)
{
	int i;
	for (i = 0; i < 2; i++)
	{
		client_static_t *s = i ? &c_ref_cls : &cls;
		s->demospeed = demospeed;
		s->demo_prespawn_end = prespawn_end;
		s->seektime = seektime;
	}
}

/* Read-back for the CL_GetDemoMessage signon-2 stamp. The cls image already
   covers it, but a stamp of 0 over a field that was already 0 is invisible,
   so the test needs to see the value itself. */
qfileofs_t ctest_cldemo_get_prespawn_end (int side)
{
	return (side ? &c_ref_cls : &cls)->demo_prespawn_end;
}

float ctest_cldemo_get_scr_clock_off (void)
{
	return scr_clock_off;
}

void ctest_cldemo_set_scr_clock_off (float v)
{
	scr_clock_off = v;
}

void ctest_cldemo_get_mviewangles (int side, float *out)
{
	client_state_t *c = side ? &c_ref_cl : &cl;
	VectorCopy (c->mviewangles[0], out + 0);
	VectorCopy (c->mviewangles[1], out + 3);
}

void ctest_cldemo_set_viewangles (float x, float y, float z)
{
	int i;
	for (i = 0; i < 2; i++)
	{
		client_state_t *c = i ? &c_ref_cl : &cl;
		c->viewangles[0] = x;
		c->viewangles[1] = y;
		c->viewangles[2] = z;
		memset (c->mviewangles, 0, sizeof (c->mviewangles));
	}
}

/* ==========================================================================
 * 4. Drivers. Every one enters through Host_Guard, so the setjmp that catches
 * an armed Sys_Error/Host_Error sits in a pure C frame outside the Rust call.
 */

typedef struct
{
	int side;
	int out;
} cldemo_drv_t;

static void ctest_cldemo_invoke_stop_playback (void *p)
{
	if (((cldemo_drv_t *)p)->side)
		c_ref_CL_StopPlayback ();
	else
		CL_StopPlayback ();
}

int ctest_cldemo_stop_playback (int side)
{
	cldemo_drv_t a;
	a.side = side;
	a.out = 0;
	return Host_Guard (ctest_cldemo_invoke_stop_playback, &a);
}

static void ctest_cldemo_invoke_get_message (void *p)
{
	cldemo_drv_t *a = (cldemo_drv_t *)p;
	a->out = a->side ? c_ref_CL_GetMessage () : CL_GetMessage ();
}

int ctest_cldemo_get_message (int side, int *out)
{
	cldemo_drv_t a;
	int			 r;
	a.side = side;
	a.out = -1;
	r = Host_Guard (ctest_cldemo_invoke_get_message, &a);
	*out = a.out;
	return r;
}

static void ctest_cldemo_invoke_seek (void *p)
{
	if (((cldemo_drv_t *)p)->side)
		c_ref_CL_Seek_f ();
	else
		CL_Seek_f ();
}

int ctest_cldemo_seek (int side)
{
	cldemo_drv_t a;
	a.side = side;
	a.out = 0;
	return Host_Guard (ctest_cldemo_invoke_seek, &a);
}

static void ctest_cldemo_invoke_stop (void *p)
{
	if (((cldemo_drv_t *)p)->side)
		c_ref_CL_Stop_f ();
	else
		CL_Stop_f ();
}

int ctest_cldemo_stop (int side)
{
	cldemo_drv_t a;
	a.side = side;
	a.out = 0;
	return Host_Guard (ctest_cldemo_invoke_stop, &a);
}

static void ctest_cldemo_invoke_record (void *p)
{
	if (((cldemo_drv_t *)p)->side)
		c_ref_CL_Record_f ();
	else
		CL_Record_f ();
}

int ctest_cldemo_record (int side)
{
	cldemo_drv_t a;
	a.side = side;
	a.out = 0;
	return Host_Guard (ctest_cldemo_invoke_record, &a);
}

static void ctest_cldemo_invoke_play (void *p)
{
	if (((cldemo_drv_t *)p)->side)
		c_ref_CL_PlayDemo_f ();
	else
		CL_PlayDemo_f ();
}

int ctest_cldemo_play (int side)
{
	cldemo_drv_t a;
	a.side = side;
	a.out = 0;
	return Host_Guard (ctest_cldemo_invoke_play, &a);
}

static void ctest_cldemo_invoke_timedemo (void *p)
{
	if (((cldemo_drv_t *)p)->side)
		c_ref_CL_TimeDemo_f ();
	else
		CL_TimeDemo_f ();
}

int ctest_cldemo_timedemo (int side)
{
	cldemo_drv_t a;
	a.side = side;
	a.out = 0;
	return Host_Guard (ctest_cldemo_invoke_timedemo, &a);
}

static void ctest_cldemo_invoke_resume_record (void *p)
{
	cldemo_drv_t *a = (cldemo_drv_t *)p;
	if (a->side)
		c_ref_CL_Resume_Record (a->out ? true : false);
	else
		CL_Resume_Record (a->out ? true : false);
}

int ctest_cldemo_resume_record (int side, int recordsignons)
{
	cldemo_drv_t a;
	a.side = side;
	a.out = recordsignons;
	return Host_Guard (ctest_cldemo_invoke_resume_record, &a);
}

/* ==========================================================================
 * 5. Reset. Runs cl_main_ref.c's reset first (it owns cl/cls), then puts the
 * demo-specific fields into a defined, non-degenerate state.
 */

extern void ctest_clmain_reset (void);

void ctest_cldemo_reset (void)
{
	ctest_cldemo_release_demo ();
	ctest_clmain_reset ();
	ctest_cldemo_attach_message ();
	ctest_cldemo_set_cmd_source ((int)src_command);
	ctest_cldemo_set_demo_state (0, 0, 0, 0, SIGNONS);
	ctest_cldemo_set_seek_fields (0.0f, 0, 0.0f);
	ctest_cldemo_set_viewangles (0.0f, 0.0f, 0.0f);
	scr_clock_off = 0.0f;
	host_framecount = 0;
	cls.timedemo = c_ref_cls.timedemo = false;
	cls.td_lastframe = c_ref_cls.td_lastframe = 0;
	cls.td_startframe = c_ref_cls.td_startframe = 0;
	cls.td_starttime = c_ref_cls.td_starttime = 0.0f;
	cls.demonum = c_ref_cls.demonum = -1;
}

/* Builds one well-formed demo record -- 4-byte little-endian length, three
 * little-endian float viewangles, then `len` payload bytes -- into `out`,
 * returning the total size. The tests use it rather than hand-writing the
 * header so a record is well-formed by construction; the deliberately
 * truncated cases are built by passing a shorter length to
 * ctest_cldemo_attach_demo. */
int ctest_cldemo_build_record (unsigned char *out, const unsigned char *payload, int len, float a0, float a1, float a2)
{
	int	  l = LittleLong (len);
	float f;

	memcpy (out + 0, &l, 4);
	f = LittleFloat (a0);
	memcpy (out + 4, &f, 4);
	f = LittleFloat (a1);
	memcpy (out + 8, &f, 4);
	f = LittleFloat (a2);
	memcpy (out + 12, &f, 4);
	if (len > 0)
		memcpy (out + 16, payload, (size_t)len);
	return 16 + len;
}
