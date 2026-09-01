/* Phase 7 M7 oracle fixture TU for Quake/cl_input.c (T7.2b).
 *
 * c_ref_prelude.h is force-included (build.rs) and already includes the real
 * Quake/client.h, so kbutton_t, client_state_t and usercmd_t are the engine's
 * own declarations here. Quake/cl_input.c is an oracle source, so every one of
 * its entry points is reachable as c_ref_<name> (c_ref_prelude.h's "cl_input.c"
 * rename block).
 *
 * This file plays the same three roles sv_user_ref.c plays for the M6 wave:
 *
 *  1. Define the PLAIN (Rust-reading) twins of every object
 *     Quake/cl_input_glue.c owns -- the seventeen kbutton_t states, in_impulse
 *     and the nine movement cvars. cl_input_glue.c is gated `#ifdef
 *     USE_RUST_HOST` and is not in build.rs's C_SOURCES, so without these
 *     there is no plain definition anywhere in the link (verified: a
 *     `cargo test --no-run` before this file was written reported all
 *     twenty-seven as unresolved externals).
 *  2. Re-implement the two ADR-009 trampolines (ClInput_Glue_WriteBatch,
 *     ClInput_Glue_Disconnect) and the plain re-raising CL_SendMove, mirroring
 *     Quake/cl_input_glue.c's bodies exactly.
 *  3. Provide the fixture seeders. Cvar_RegisterVariable never runs in this
 *     link, so every cvar's .value is 0 from static init on BOTH sides --
 *     exactly the "both sides degenerate identically" shape a bit-exact
 *     differential accepts. ctest_clinput_reset therefore publishes the real
 *     defaults into both storages explicitly, and every setter writes the
 *     c_ref_* copy and the plain copy in the same call.
 *
 * Callee selection (same rule sv_send_ref.c:1051 records): ClInput_Glue_
 * WriteBatch below replays through the ORACLE encoder (c_ref_MSG_Write*), so
 * this differential isolates cl_input.c's own decisions -- which ops, in which
 * order, with which arguments -- rather than also folding in the M5 net_msg
 * port, which net_msg_differential.rs already gates. The real -Duse_rust_host
 * build keeps the Rust-to-Rust topology; that is cl_input_glue.c's business.
 *
 * CROSS-WAVE DEPENDENCY (T7.2a, view.c/chase.c): plain V_StartPitchDrift,
 * V_StopPitchDrift and v_centerspeed are owned by stubs/view_ref.c, per the
 * peer split agreed for this milestone. They are declared here, not defined.
 * Until view_ref.c replaces its placeholder this TU compiles but no test
 * binary links. cl_maxpitch/cl_minpitch/lookspring are cl_main.c cvars with no
 * plain twin anywhere (cl_main.c is a T7.4 file); by the same agreement this
 * file owns them, since cl_input.c is their only reader in the ported set.
 */

#include <string.h>

/* Host_Guard/Host_Reraise live in stubs.c and are not declared by any header
 * the prelude pulls in (the real engine declares them via host.h), same as
 * sv_user_ref.c/pf_cl_ref.c. */
extern int	Host_Guard (void (*fn) (void *), void *arg);
extern void Host_Reraise (int guard_result);

/* --------------------------------------------------------------------------
 * Plain (Rust-reading) storage this wave owns.
 *
 * The prelude's rename macros are live in this TU and would rewrite each
 * definition below to c_ref_*, colliding with the real oracle compiled from
 * cl_input.c (LNK2005), so every name is #undef'd first. Once #undef'd the
 * bare name means the PLAIN copy for the rest of the file; oracle access
 * always spells c_ref_* by hand.
 *
 * Initializers copied verbatim from Quake/cl_input_glue.c:54-74, which in turn
 * copies cl_input.c:53-58 and :323-335. in_mlook's `.state = 1` is load-bearing
 * (IN_MLookUp's lookspring branch reads it), so the designated initializer is
 * kept rather than flattened.
 */
#undef in_mlook
#undef in_klook
#undef in_left
#undef in_right
#undef in_forward
#undef in_back
#undef in_lookup
#undef in_lookdown
#undef in_moveleft
#undef in_moveright
#undef in_strafe
#undef in_speed
#undef in_use
#undef in_jump
#undef in_attack
#undef in_up
#undef in_down
#undef in_impulse

kbutton_t in_mlook = {.state = 1}, in_klook;
kbutton_t in_left, in_right, in_forward, in_back;
kbutton_t in_lookup, in_lookdown, in_moveleft, in_moveright;
kbutton_t in_strafe, in_speed, in_use, in_jump, in_attack;
kbutton_t in_up, in_down;

int in_impulse;

#undef cl_upspeed
#undef cl_forwardspeed
#undef cl_backspeed
#undef cl_sidespeed
#undef cl_movespeedkey
#undef cl_yawspeed
#undef cl_pitchspeed
#undef cl_anglespeedkey
#undef cl_alwaysrun

cvar_t cl_upspeed = {"cl_upspeed", "200", CVAR_NONE};
cvar_t cl_forwardspeed = {"cl_forwardspeed", "200", CVAR_ARCHIVE};
cvar_t cl_backspeed = {"cl_backspeed", "200", CVAR_ARCHIVE};
cvar_t cl_sidespeed = {"cl_sidespeed", "350", CVAR_NONE};

cvar_t cl_movespeedkey = {"cl_movespeedkey", "2.0", CVAR_NONE};

cvar_t cl_yawspeed = {"cl_yawspeed", "140", CVAR_NONE};
cvar_t cl_pitchspeed = {"cl_pitchspeed", "150", CVAR_NONE};

cvar_t cl_anglespeedkey = {"cl_anglespeedkey", "1.5", CVAR_NONE};

cvar_t cl_alwaysrun = {"cl_alwaysrun", "1", CVAR_ARCHIVE};

/* cl_main.c:41, :50, :51 -- read by cl_input.c (the pitch clamps and
 * IN_MLookUp's lookspring branch) and by nothing else in the ported set. */
#undef cl_maxpitch
#undef cl_minpitch
#undef lookspring

cvar_t cl_maxpitch = {"cl_maxpitch", "90", CVAR_ARCHIVE};
cvar_t cl_minpitch = {"cl_minpitch", "-90", CVAR_ARCHIVE};
cvar_t lookspring = {"lookspring", "0", CVAR_NONE};

/* --------------------------------------------------------------------------
 * Oracle-side declarations the engine headers do not provide.
 *
 * client.h declares only in_mlook/in_klook/in_strafe/in_speed and the nine
 * cvars, so those c_ref_* twins are already in scope. The other thirteen
 * kbuttons, in_impulse and the two pitch-clamp cvars have no header at all.
 */
extern kbutton_t c_ref_in_left, c_ref_in_right, c_ref_in_forward, c_ref_in_back;
extern kbutton_t c_ref_in_lookup, c_ref_in_lookdown, c_ref_in_moveleft, c_ref_in_moveright;
extern kbutton_t c_ref_in_use, c_ref_in_jump, c_ref_in_attack;
extern kbutton_t c_ref_in_up, c_ref_in_down;
extern int		 c_ref_in_impulse;
extern cvar_t	 c_ref_cl_maxpitch;
extern cvar_t	 c_ref_cl_minpitch;

/* T7.2a-owned plain twins -- declared, not defined; see the CROSS-WAVE
 * DEPENDENCY note at the top. */
#undef v_centerspeed
extern cvar_t c_ref_v_centerspeed;
extern cvar_t v_centerspeed;

/* --------------------------------------------------------------------------
 * cl / cls. Both exist twice: the oracle's c_ref_ copies (cl_main.c) and the
 * plain copies the Rust port reads (stubs.c, see its DUPLICATE-SYMBOL HAZARD
 * block). Every seeder below writes both.
 */
#undef cl
#undef cls
extern client_state_t  cl;
extern client_static_t cls;

/* --------------------------------------------------------------------------
 * CL_Disconnect. cl_main.c is an oracle source, so c_ref_CL_Disconnect is the
 * real body; there is no plain twin anywhere in this link (T7.4 owns that).
 * The recording double below is what ClInput_Glue_Disconnect drives on the
 * Rust side.
 *
 * KNOWN ASYMMETRY, and a branch this harness cannot exercise: stubs.c's
 * NET_SendUnreliableMessage recorder always returns 1, never -1, so
 * cl_input.c:557's "lost server connection" arm is unreachable on BOTH sides
 * and neither CL_Disconnect ever runs. The two sides being different bodies
 * therefore cannot show up as a differential -- and equally cannot be proven
 * equivalent here. Reported as a stubs.c request (a settable return for the
 * NET send doubles) rather than papered over.
 */
#undef CL_Disconnect
static int ctest_clinput_disconnects;

void CL_Disconnect (void)
{
	ctest_clinput_disconnects++;
}

int ctest_clinput_disconnect_calls (void)
{
	return ctest_clinput_disconnects;
}

/* --------------------------------------------------------------------------
 * ClInput_Glue_* -- ADR-009 trampolines, bodies mirroring
 * Quake/cl_input_glue.c:88-158 exactly, except that the encoder is spelled
 * c_ref_* (callee-selection note above). Neither is defined anywhere else in
 * the harness: cl_input_glue.c is not in build.rs's C_SOURCES.
 */

/* Layout-identical to clinput_write_t in Quake/cl_input_glue.c:89-95 and to
 * ClInputWriteOp in rust/quake-c-sys/src/cl_input.rs:38-45. */
typedef struct
{
	int		 kind;
	int		 i;
	float	 f;
	unsigned u;
} ctest_clinput_write_t;

typedef struct
{
	sizebuf_t					*sb;
	const ctest_clinput_write_t *ops;
	int							 count;
} ctest_clinput_writebatch_arg_t;

static void ctest_clinput_invoke_writebatch (void *p)
{
	ctest_clinput_writebatch_arg_t *a = (ctest_clinput_writebatch_arg_t *)p;
	int								k;

	for (k = 0; k < a->count; k++)
	{
		const ctest_clinput_write_t *op = &a->ops[k];
		switch (op->kind)
		{
		case 0:
			c_ref_MSG_WriteByte (a->sb, op->i);
			break;
		case 1:
			c_ref_MSG_WriteShort (a->sb, op->i);
			break;
		case 2:
			c_ref_MSG_WriteLong (a->sb, op->i);
			break;
		case 3:
			c_ref_MSG_WriteFloat (a->sb, op->f);
			break;
		case 4:
			c_ref_MSG_WriteAngle (a->sb, op->f, op->u);
			break;
		case 5:
			c_ref_MSG_WriteAngle16 (a->sb, op->f, op->u);
			break;
		default:
			Sys_Error ("ClInput_Glue_WriteBatch: bad op %i", op->kind);
		}
	}
}

int ClInput_Glue_WriteBatch (sizebuf_t *sb, const ctest_clinput_write_t *ops, int count)
{
	ctest_clinput_writebatch_arg_t arg;
	arg.sb = sb;
	arg.ops = ops;
	arg.count = count;
	return Host_Guard (ctest_clinput_invoke_writebatch, &arg);
}

static void ctest_clinput_invoke_disconnect (void *p)
{
	(void)p;
	CL_Disconnect ();
}

int ClInput_Glue_Disconnect (void)
{
	return Host_Guard (ctest_clinput_invoke_disconnect, NULL);
}

/* --------------------------------------------------------------------------
 * Plain-named Rust-side driver, mirroring Quake/cl_input_glue.c:165-169.
 * quake_rs_cl_send_move is the #[no_mangle] export from
 * rust/quake-capi/src/cl_input.rs; the other cl_input.c entry points are
 * plain-named Rust exports already and need no wrapper here.
 */
#undef CL_SendMove
extern int quake_rs_cl_send_move (const usercmd_t *cmd);

void CL_SendMove (const usercmd_t *cmd)
{
	Host_Reraise (quake_rs_cl_send_move (cmd));
}

/* --------------------------------------------------------------------------
 * The fixture.
 */

/* Button index -> the two storages. Order is the one the Rust test mirrors;
 * it is arbitrary but must not change without updating cl_input_differential.rs. */
static kbutton_t *ctest_clinput_buttons[2][17] = {
	{&in_mlook, &in_klook, &in_left, &in_right, &in_forward, &in_back, &in_lookup, &in_lookdown, &in_moveleft, &in_moveright, &in_strafe, &in_speed, &in_use,
	 &in_jump, &in_attack, &in_up, &in_down},
	{&c_ref_in_mlook, &c_ref_in_klook, &c_ref_in_left, &c_ref_in_right, &c_ref_in_forward, &c_ref_in_back, &c_ref_in_lookup, &c_ref_in_lookdown,
	 &c_ref_in_moveleft, &c_ref_in_moveright, &c_ref_in_strafe, &c_ref_in_speed, &c_ref_in_use, &c_ref_in_jump, &c_ref_in_attack, &c_ref_in_up,
	 &c_ref_in_down}};

/* side 0 == plain (what the Rust port reads), side 1 == oracle (c_ref_*). */
void *ctest_clinput_button_addr (int side, int idx)
{
	if (side < 0 || side > 1 || idx < 0 || idx >= 17)
		return NULL;
	return ctest_clinput_buttons[side][idx];
}

void ctest_clinput_set_button (int idx, int down0, int down1, int state)
{
	int side;
	if (idx < 0 || idx >= 17)
		return;
	for (side = 0; side < 2; side++)
	{
		ctest_clinput_buttons[side][idx]->down[0] = down0;
		ctest_clinput_buttons[side][idx]->down[1] = down1;
		ctest_clinput_buttons[side][idx]->state = state;
	}
}

/* out[3] = {down[0], down[1], state}. */
void ctest_clinput_get_button (int side, int idx, int *out)
{
	if (side < 0 || side > 1 || idx < 0 || idx >= 17)
	{
		out[0] = out[1] = out[2] = -1;
		return;
	}
	out[0] = ctest_clinput_buttons[side][idx]->down[0];
	out[1] = ctest_clinput_buttons[side][idx]->down[1];
	out[2] = ctest_clinput_buttons[side][idx]->state;
}

void ctest_clinput_set_impulse (int v)
{
	in_impulse = v;
	c_ref_in_impulse = v;
}

int ctest_clinput_get_impulse (int side)
{
	return side ? c_ref_in_impulse : in_impulse;
}

/* v[13]: upspeed, forwardspeed, backspeed, sidespeed, movespeedkey, yawspeed,
 * pitchspeed, anglespeedkey, alwaysrun, maxpitch, minpitch, lookspring,
 * v_centerspeed. Every entry is written to both storages unconditionally --
 * a sentinel-guarded "skip" would let a test silently inherit the previous
 * test's value on one side only. */
void ctest_clinput_set_cvars (const float *v)
{
	cl_upspeed.value = c_ref_cl_upspeed.value = v[0];
	cl_forwardspeed.value = c_ref_cl_forwardspeed.value = v[1];
	cl_backspeed.value = c_ref_cl_backspeed.value = v[2];
	cl_sidespeed.value = c_ref_cl_sidespeed.value = v[3];
	cl_movespeedkey.value = c_ref_cl_movespeedkey.value = v[4];
	cl_yawspeed.value = c_ref_cl_yawspeed.value = v[5];
	cl_pitchspeed.value = c_ref_cl_pitchspeed.value = v[6];
	cl_anglespeedkey.value = c_ref_cl_anglespeedkey.value = v[7];
	cl_alwaysrun.value = c_ref_cl_alwaysrun.value = v[8];
	cl_maxpitch.value = c_ref_cl_maxpitch.value = v[9];
	cl_minpitch.value = c_ref_cl_minpitch.value = v[10];
	lookspring.value = c_ref_lookspring.value = v[11];
	v_centerspeed.value = c_ref_v_centerspeed.value = v[12];
}

/* The engine defaults, i.e. what Cvar_RegisterVariable would have produced
 * from the initializers above. Kept here rather than in the Rust test so the
 * two storages cannot drift apart in a per-test literal. */
static const float ctest_clinput_cvar_defaults[13] = {
	200.0f, /* cl_upspeed */
	200.0f, /* cl_forwardspeed */
	200.0f, /* cl_backspeed */
	350.0f, /* cl_sidespeed */
	2.0f,	/* cl_movespeedkey */
	140.0f, /* cl_yawspeed */
	150.0f, /* cl_pitchspeed */
	1.5f,	/* cl_anglespeedkey */
	1.0f,	/* cl_alwaysrun */
	90.0f,	/* cl_maxpitch */
	-90.0f, /* cl_minpitch */
	0.0f,	/* lookspring */
	500.0f, /* v_centerspeed */
};

/* Zeroes both copies of every kbutton_t (restoring in_mlook's .state == 1
 * static initializer), in_impulse, and every cl/cls field cl_input.c reads or
 * writes; then republishes the cvar defaults into both storages.
 *
 * cls.state is set to ca_connected explicitly: cactive_t starts at
 * ca_dedicated, so a memset would leave an "uninitialised" cls looking like a
 * dedicated server on both sides at once. */
void ctest_clinput_reset (void)
{
	int i;

	for (i = 0; i < 17; i++)
		ctest_clinput_set_button (i, 0, 0, 0);
	ctest_clinput_set_button (0, 0, 0, 1); /* in_mlook */

	ctest_clinput_set_impulse (0);
	ctest_clinput_set_cvars (ctest_clinput_cvar_defaults);

	memset (cl.viewangles, 0, sizeof (cl.viewangles));
	memset (c_ref_cl.viewangles, 0, sizeof (c_ref_cl.viewangles));
	cl.mtime[0] = c_ref_cl.mtime[0] = 0.0;
	cl.mtime[1] = c_ref_cl.mtime[1] = 0.0;
	cl.fixangle_time = c_ref_cl.fixangle_time = -1.0;
	cl.time = c_ref_cl.time = 0.0;
	cl.pitchvel = c_ref_cl.pitchvel = 0.0f;
	cl.nodrift = c_ref_cl.nodrift = false;
	cl.driftmove = c_ref_cl.driftmove = 0.0f;
	cl.laststop = c_ref_cl.laststop = 0.0;
	cl.movemessages = c_ref_cl.movemessages = 0;
	cl.protocol = c_ref_cl.protocol = PROTOCOL_FITZQUAKE;
	cl.protocolflags = c_ref_cl.protocolflags = 0;
	cl.protocol_pext2 = c_ref_cl.protocol_pext2 = 0;
	cl.ackframes_count = c_ref_cl.ackframes_count = 0;
	memset (cl.ackframes, 0, sizeof (cl.ackframes));
	memset (c_ref_cl.ackframes, 0, sizeof (c_ref_cl.ackframes));
	memset (cl.movecmds, 0, sizeof (cl.movecmds));
	memset (c_ref_cl.movecmds, 0, sizeof (c_ref_cl.movecmds));

	cls.signon = c_ref_cls.signon = SIGNONS;
	cls.demoplayback = c_ref_cls.demoplayback = false;
	cls.netcon = c_ref_cls.netcon = NULL;
	cls.state = c_ref_cls.state = ca_connected;

	host_frametime = 0.0;
	ctest_clinput_disconnects = 0;
}

void ctest_clinput_set_angles (const float *a)
{
	VectorCopy (a, cl.viewangles);
	VectorCopy (a, c_ref_cl.viewangles);
}

void ctest_clinput_get_angles (int side, float *out)
{
	VectorCopy (side ? c_ref_cl.viewangles : cl.viewangles, out);
}

/* mtime/fixangle_time drive CL_AngleLocked; time and the three drift fields
 * are what V_StopPitchDrift writes, so they are part of the comparison too. */
void ctest_clinput_set_times (double time, double mtime0, double mtime1, double fixangle_time, double frametime)
{
	cl.time = c_ref_cl.time = time;
	cl.mtime[0] = c_ref_cl.mtime[0] = mtime0;
	cl.mtime[1] = c_ref_cl.mtime[1] = mtime1;
	cl.fixangle_time = c_ref_cl.fixangle_time = fixangle_time;
	host_frametime = frametime;
}

/* out[4] = {pitchvel, nodrift, driftmove, laststop}. */
void ctest_clinput_get_drift (int side, double *out)
{
	const client_state_t *c = side ? &c_ref_cl : &cl;
	out[0] = (double)c->pitchvel;
	out[1] = (double)(c->nodrift ? 1 : 0);
	out[2] = (double)c->driftmove;
	out[3] = c->laststop;
}

static int ctest_clinput_netcon_sentinel;

void ctest_clinput_set_proto (unsigned protocol, unsigned pext2, unsigned protoflags, int signon, int demoplayback, int has_netcon)
{
	cl.protocol = c_ref_cl.protocol = protocol;
	cl.protocol_pext2 = c_ref_cl.protocol_pext2 = pext2;
	cl.protocolflags = c_ref_cl.protocolflags = protoflags;
	cls.signon = c_ref_cls.signon = signon;
	cls.demoplayback = c_ref_cls.demoplayback = demoplayback ? true : false;
	cls.netcon = c_ref_cls.netcon = has_netcon ? (struct qsocket_s *)&ctest_clinput_netcon_sentinel : NULL;
}

void ctest_clinput_set_ackframes (const int *frames, int count)
{
	int i;
	if (count < 0)
		count = 0;
	if (count > 8)
		count = 8;
	for (i = 0; i < count; i++)
		cl.ackframes[i] = c_ref_cl.ackframes[i] = frames[i];
	cl.ackframes_count = c_ref_cl.ackframes_count = (unsigned int)count;
}

int ctest_clinput_get_ackframes_count (int side)
{
	return (int)(side ? c_ref_cl.ackframes_count : cl.ackframes_count);
}

void ctest_clinput_set_movemessages (int n)
{
	cl.movemessages = c_ref_cl.movemessages = n;
}

int ctest_clinput_get_movemessages (int side)
{
	return side ? c_ref_cl.movemessages : cl.movemessages;
}

int ctest_clinput_usercmd_size (void)
{
	return (int)sizeof (usercmd_t);
}

/* Builds a usercmd_t in the caller's buffer. CL_SendMove reads servertime,
 * forwardmove/sidemove/upmove, buttons, impulse and weapon, and copies the
 * whole struct into cl.movecmds[], so the fields it does NOT read are seeded
 * too -- an accidental partial copy in the port would otherwise be invisible.
 * The bytes are zeroed first for the same reason. */
void ctest_clinput_make_cmd (
	void *out, float servertime, float seconds, const float *viewangles, float forwardmove, float sidemove, float upmove, unsigned int buttons,
	unsigned int impulse, unsigned int sequence, int weapon)
{
	usercmd_t *cmd = (usercmd_t *)out;

	memset (cmd, 0, sizeof (*cmd));
	cmd->servertime = servertime;
	cmd->seconds = seconds;
	VectorCopy (viewangles, cmd->viewangles);
	cmd->forwardmove = forwardmove;
	cmd->sidemove = sidemove;
	cmd->upmove = upmove;
	cmd->forwardmove_accumulator = 1.25f;
	cmd->sidemove_accumulator = -2.5f;
	cmd->upmove_accumulator = 0.125f;
	cmd->buttons = buttons;
	cmd->impulse = impulse;
	cmd->sequence = sequence;
	cmd->weapon = weapon;
}

/* Copies one movecmds[] ringbuffer slot out whole, so the test compares the
 * full usercmd_t byte image rather than a hand-picked field list. */
void ctest_clinput_get_movecmd (int side, int idx, void *out)
{
	const client_state_t *c = side ? &c_ref_cl : &cl;
	if (idx < 0 || idx >= 64)
	{
		memset (out, 0xff, sizeof (usercmd_t));
		return;
	}
	memcpy (out, &c->movecmds[idx], sizeof (usercmd_t));
}

/* --------------------------------------------------------------------------
 * Command-layer plumbing.
 *
 * KeyDown/KeyUp/IN_Impulse read Cmd_Argv(1), and CL_InitInput registers 34
 * commands. cmd.c is an oracle source, so there are two independent tokenizer
 * states and two independent command tables: the oracle's (c_ref_*) and the
 * Rust one quake-capi's `cvar` feature links in. Both must be driven.
 */
#undef Cmd_TokenizeString
#undef Cmd_Exists
extern void	   Cmd_TokenizeString (const char *text);
extern qboolean Cmd_Exists (const char *cmd_name);

void ctest_clinput_tokenize (const char *text)
{
	c_ref_Cmd_TokenizeString (text);
	Cmd_TokenizeString (text);
}

int ctest_clinput_cmd_exists (int side, const char *name)
{
	return (side ? c_ref_Cmd_Exists (name) : Cmd_Exists (name)) ? 1 : 0;
}
