/* Phase 7 M6 oracle TU for Quake/sv_user.c (T6.4).
 *
 * c_ref_prelude.h is force-included (build.rs) and already includes the real
 * Quake/server.h and Quake/client.h, so sv, svs, client_t, cl and cls are the
 * engine's own declarations here. Quake/sv_user.c is an oracle source, so
 * every one of its 13 entry points is reachable as c_ref_<name> (see the
 * rename block in c_ref_prelude.h's "sv_user.c" section).
 *
 * Only SV_SetIdealPitch, SV_ClientThink and SV_RunClients are declared in
 * server.h and called from another file (sv_send.c, host.c) -- confirmed by
 * grepping Quake/*.c for the other ten names, which are used only inside
 * sv_user.c itself. On the Rust side only those same three are `quake_rs_*`
 * exports (`rust/quake-capi/src/sv_user.rs`); the other ten are private
 * functions reached only by driving these three, exactly like
 * `rust/quake-ctest/tests/sv_phys_differential.rs` drives every internal
 * sv_phys.c helper through `SV_Physics`/individually-exported raise sites.
 * The tests in sv_user_differential.rs therefore reach every branch of all
 * 13 original functions by shaping fixture state and calling only these
 * three plain-named entry points (plus SV_ReadClientMove/SV_ReadClientMessage
 * indirectly through SV_RunClients's receive loop and directly is not
 * possible since neither is exported on the Rust side -- see the test file's
 * own doc comment for the exact coverage list).
 *
 * ADR-009: sv_user.c's own raise sites are all transitive (SV_Move, the
 * clc_stringcmd QC/Cmd_ExecuteString dispatch, and SV_DropClient). The three
 * plain-named wrappers below re-raise from a pure C frame, exactly mirroring
 * Quake/sv_user_glue.c (which is not compiled here -- Meson-only).
 *
 * CROSS-WAVE DEPENDENCY: sv/svs (server_t/server_static_t, Quake/sv_main.c)
 * are declared `extern` below rather than defined -- per M6 peer
 * coordination, `rust/quake-ctest/stubs/sv_main_ref.c` (T6.5) owns the one
 * plain (Rust-reading) copy of each, exactly like sv_friction/sv_stopspeed
 * are stub-owned for the M4 wave. Until that file replaces its current
 * placeholder, this TU compiles but the differential test binary does not
 * link (undefined external symbols: sv, svs).
 *
 * CORRECTION (found by build verification after the cvars were first left
 * `extern` here too): the five sv_user.c cvars are NOT part of that hand-off.
 * They are defined in `Quake/sv_user_glue.c` (this wave's own glue file),
 * which is a Meson-only TU never compiled into the ctest harness -- so
 * leaving them merely `extern`-declared here left C2065/link-time undefined
 * symbols with no plain definition anywhere in the link. sv_player has the
 * same shape (`sv, svs and sv_player move with their files`,
 * c_ref_prelude.h's M6 rename-block comment) and was already defined here
 * correctly; the five cvars are now defined here too, with the exact
 * initializers `Quake/sv_user_glue.c` uses.
 */

#include <string.h>

/* Host_Guard/Host_Reraise (stubs.c) and ctest_phys_reset (stubs.c) are not
 * declared by any header the prelude includes -- the real engine declares
 * Host_Guard/Host_Reraise via host.h/quakedef.h and ctest_phys_reset does not
 * exist in the real engine at all, so all three are declared explicitly
 * here too, exactly like pf_msg_ref.c/pf_cl_ref.c already do for Host_Guard.
 * Confirmed via an isolated /c compile of this TU alone: without these,
 * MSVC accepts the calls anyway (C4013 "assuming extern returning int"),
 * but ctest_phys_reset actually returns void, so the assumed-int fallback
 * is a real (if silent) return-type mismatch worth avoiding. */
extern int	Host_Guard (void (*fn) (void *), void *arg);
extern void Host_Reraise (int guard_result);
extern void ctest_phys_reset (int num_edicts, int maxclients, double frametime, double vmtime, int physics_mode);
extern void ctest_phys_set_cvars (const float *v);

/* --------------------------------------------------------------------------
 * Plain (Rust-reading) storage this wave owns: sv_player (sv_user.c:26) and
 * the net_message/msg_readcount/msg_badread trio SV_ReadClientMove and
 * SV_ReadClientMessage consume through quake-capi::net's plain MSG_Read*
 * exports (Quake/net_main.c / Quake/net_msg_glue.c own these in the real
 * engine; neither file is compiled here, so the harness owns them -- same
 * shape as stubs.c's own "net_message itself lives in net_main.c (not
 * compiled here)" c_ref copy, just the unrenamed twin of it).
 */
#undef sv_player
edict_t *sv_player;

#undef net_message
#undef msg_readcount
#undef msg_badread
sizebuf_t net_message;
int		  msg_readcount;
qboolean  msg_badread;

/* --------------------------------------------------------------------------
 * T6.5-owned plain storage (Quake/sv_main.c wave): sv/svs only. Declared,
 * not defined -- see the CROSS-WAVE DEPENDENCY note above.
 *
 * c_ref_sv/c_ref_svs are already `extern`-declared here: the M6 rename block
 * (c_ref_prelude.h) makes server.h's own `extern server_t sv;`/`extern
 * server_static_t svs;` expand under those names when server.h is force-
 * included below this point.
 *
 * The five cvars have no such header, so their c_ref_* twins are declared
 * explicitly, and (see CORRECTION above) this file also DEFINES the plain
 * copies -- they are this wave's own (Quake/sv_user_glue.c's), not T6.5's.
 * Every oracle-side use in this file spells the c_ref_* name literally, and
 * the plain names below are #undef'd once and never re-defined, so a
 * textual rename mistake elsewhere cannot silently redirect an oracle call.
 */
extern cvar_t c_ref_sv_idealpitchscale;
extern cvar_t c_ref_sv_altnoclip;
extern cvar_t c_ref_sv_maxspeed;
extern cvar_t c_ref_sv_accelerate;
extern cvar_t c_ref_sv_edgefriction;

#undef sv
#undef svs
extern server_t		   sv;
extern server_static_t svs;
/* sv/svs stay undef'd (bare name == plain/T6.5 copy) for the rest of this
 * file; oracle access always spells c_ref_* by hand. */

/* --------------------------------------------------------------------------
 * Phase 7 M7 (T7.0): cl_main.c and view.c became oracle sources, so `cls`,
 * `cl_rollangle` and `cl_rollspeed` now exist twice in the link -- the oracle's
 * c_ref_* copies (cl_main.c / view.c) and the plain copies the Rust port reads
 * (stubs.c). See the DUPLICATE-SYMBOL HAZARD block in stubs.c.
 *
 * Both matter to this fixture:
 *   - c_ref_SV_ReadClientMove reads cls.netcon (sv_user.c's ProQuake angle
 *     hack); quake-capi/src/sv_user.rs:73 reads the plain `cls` for the same
 *     branch. Seeding only one side would make the two disagree for a reason
 *     that has nothing to do with SV_ReadClientMove.
 *   - c_ref_SV_ClientThink reaches c_ref_V_CalcRoll (real view.c), which scales
 *     by c_ref_cl_rollangle.value / c_ref_cl_rollspeed.value. view.c defines
 *     that pair with no explicit `.value` (Cvar_RegisterVariable fills it in,
 *     and never runs in this link), so without the seeding below the oracle's
 *     roll would be a flat 0 while the Rust side's stubs.c copy returned the
 *     real value -- and, worse, a `0` result is exactly the degenerate answer a
 *     bit-exact comparison would accept if the Rust side ever regressed the
 *     same way. The plain copies are set to the same literals so the shared
 *     input stays symmetric by construction rather than by static-initializer
 *     coincidence in stubs.c.
 * c_ref_cls is already declared: client.h is force-included through the prelude
 * with the rename in effect. The two cvars have no header at all.
 */
extern cvar_t c_ref_cl_rollangle;
extern cvar_t c_ref_cl_rollspeed;

#undef cls
#undef cl_rollangle
#undef cl_rollspeed
extern client_static_t cls;
extern cvar_t		   cl_rollangle;
extern cvar_t		   cl_rollspeed;

#undef sv_idealpitchscale
#undef sv_altnoclip
#undef sv_maxspeed
#undef sv_accelerate
#undef sv_edgefriction
/* Plain definitions, this wave's own -- exact initializers copied from
 * Quake/sv_user_glue.c (sv_user.c:29, :43, :44, :198, :199). COMPAT: the
 * "edgefriction" cvar's name string is "edgefriction", not "sv_edgefriction"
 * (upstream naming quirk, preserved bug-for-bug). */
cvar_t sv_idealpitchscale = {"sv_idealpitchscale", "0.8", CVAR_NONE};
cvar_t sv_altnoclip = {"sv_altnoclip", "1", CVAR_ARCHIVE};
cvar_t sv_maxspeed = {"sv_maxspeed", "320", CVAR_NOTIFY | CVAR_SERVERINFO};
cvar_t sv_accelerate = {"sv_accelerate", "10", CVAR_NONE};
cvar_t sv_edgefriction = {"edgefriction", "2", CVAR_NONE}; // COMPAT: name string is "edgefriction", not "sv_edgefriction"

/* --------------------------------------------------------------------------
 * SvUser_Glue_* trampolines (ADR-009), mirroring Quake/sv_user_glue.c's own
 * bodies exactly (SvUser_InvokeStringCmd / SvUser_InvokeDropClient). Neither
 * is defined anywhere else in the ctest harness (confirmed: no match for
 * "SvUser_Glue_" outside this file), so the Rust port's
 * quake_c_sys::sv_user::SvUser_Glue_StringCmd/SvUser_Glue_DropClient externs
 * resolve here.
 */
/* Cmd_ExecuteString: undef'd + extern-declared rather than left under the
 * active c_ref_ rename. Quake/cvar_cmd_glue.c (the real engine's plain
 * wrapper, Host_Reraise(quake_rs_cmd_execute_string(...))) is not compiled
 * into this harness, but stubs.c already defines the identical plain
 * wrapper for the M2 wave's own tests (stubs.c:1832-1838, `#undef
 * Cmd_ExecuteString` then the same body) -- reused here rather than
 * duplicated, since a second plain definition would be a link error. Using
 * the oracle's c_ref_Cmd_ExecuteString here instead would be wrong: the real
 * Quake/sv_user_glue.c calls plain Cmd_ExecuteString, which -Duse_rust_cvar
 * (required alongside -Duse_rust_host, meson.build:398-399) always resolves
 * to the Rust-routed cvar_cmd_glue.c wrapper, never to Quake/cmd.c. */
#undef Cmd_ExecuteString
extern qboolean Cmd_ExecuteString (const char *text, cmd_source_t src);

static void SvUser_InvokeStringCmd (void *p)
{
	const char *s = (const char *)p;
	if (q_strncasecmp (s, "spawn", 5) && q_strncasecmp (s, "begin", 5) && q_strncasecmp (s, "prespawn", 8) && qcvm->extfuncs.SV_ParseClientCommand)
	{
		client_t *ohc = host_client;
		G_INT (OFS_PARM0) = PR_SetEngineString (s);
		pr_global_struct->time = qcvm->time;
		pr_global_struct->self = EDICT_TO_PROG (host_client->edict);
		PR_ExecuteProgram (qcvm->extfuncs.SV_ParseClientCommand);
		host_client = ohc;
	}
	else
		Cmd_ExecuteString (s, src_client);
}

int SvUser_Glue_StringCmd (const char *s)
{
	return Host_Guard (SvUser_InvokeStringCmd, (void *)s);
}

/* SV_DropClient is not an oracle source in this harness (stubs.c's own
 * placeholder unconditionally Sys_Errors when invoked -- confirmed by
 * reading stubs.c), so this guard's "success" path can never be observed
 * here; only its raise-propagation (CTEST_GUARD_SYS_ERROR) can. */
static void SvUser_InvokeDropClient (void *p)
{
	qboolean crash = *(qboolean *)p;
	SV_DropClient (crash);
}

int SvUser_Glue_DropClient (qboolean crash)
{
	return Host_Guard (SvUser_InvokeDropClient, &crash);
}

/* --------------------------------------------------------------------------
 * Plain-named Rust-side drivers, mirroring Quake/sv_user_glue.c exactly.
 * quake_rs_sv_set_ideal_pitch/quake_rs_sv_client_think/quake_rs_sv_run_clients
 * are the #[no_mangle] exports from rust/quake-capi/src/sv_user.rs, linked
 * in because quake-ctest's Cargo.toml depends on quake-capi with the "host"
 * feature.
 *
 * The prelude's rename macros are still live in this translation unit and
 * would rewrite these definitions to c_ref_*, colliding with the real oracle
 * compiled from sv_user.c (LNK2005), so each name is #undef'd first --
 * same idiom stubs.c uses for its own plain-named definitions (e.g.
 * SV_LinkEdict, around stubs.c:5400).
 */
#undef SV_SetIdealPitch
#undef SV_ClientThink
#undef SV_RunClients

extern int quake_rs_sv_set_ideal_pitch (void);
extern int quake_rs_sv_client_think (void);
extern int quake_rs_sv_run_clients (void);

void SV_SetIdealPitch (void)
{
	Host_Reraise (quake_rs_sv_set_ideal_pitch ());
}

void SV_ClientThink (void)
{
	Host_Reraise (quake_rs_sv_client_think ());
}

void SV_RunClients (void)
{
	Host_Reraise (quake_rs_sv_run_clients ());
}

/* --------------------------------------------------------------------------
 * The fixture.
 *
 * Edict/world/qcvm/areanode state is the SAME shared M3 "synthetic room"
 * fixture sv_phys_differential.rs and world_differential.rs use
 * (ctest_phys_reset/ctest_world_*), reused wholesale rather than duplicated:
 * a brush model with three real clipping hulls, a solid pillar, water/lava
 * boxes, floor at z=-168. ctest_phys_reset also republishes the oracle's own
 * c_ref_sv/c_ref_svs/c_ref_sv_player/host_frametime/qcvm->time, which is
 * everything the ORACLE run of any sv_user.c entry point needs.
 *
 * For the RUST run, this file additionally publishes the plain twins:
 * plain sv_player (this file), plain sv/svs (T6.5, once landed) and a
 * SEPARATE plain client array (ctest_svuser_clients below) -- svs.clients
 * cannot alias stubs.c's private ctest_phys_clients array (file-scope
 * static, not reachable from here), so the plain client_t state used by
 * SV_RunClients/SV_ReadClientMessage/SV_ReadClientMove is this file's own,
 * populated in parallel with (and to the same values as) whatever the
 * per-test oracle setup used.
 */

#define CTEST_SVUSER_MAX_CLIENTS 4
static client_t ctest_svuser_clients[CTEST_SVUSER_MAX_CLIENTS];

/* Resets the shared M3 room + edict arena (via ctest_phys_reset, physics
 * mode left at -1/unused) and both sides' sv/svs/sv_player/client array.
 * num_edicts must be >= 2 so EDICT_NUM(1) is a real, non-world edict; the
 * player edict is always EDICT_NUM(1). */
void ctest_svuser_reset (int num_edicts, int maxclients, double frametime, double vmtime)
{
	int i;

	if (num_edicts < 2)
		num_edicts = 2;
	if (maxclients < 0)
		maxclients = 0;
	if (maxclients > CTEST_SVUSER_MAX_CLIENTS)
		maxclients = CTEST_SVUSER_MAX_CLIENTS;

	ctest_phys_reset (num_edicts, maxclients, frametime, vmtime, -1); /* sets c_ref_sv/c_ref_svs/c_ref_sv_player */

	/* BUG FOUND AND FIXED THIS SESSION: ctest_phys_reset never calls
	 * ctest_phys_set_cvars, so sv_friction/sv_stopspeed (globals owned by the
	 * M4 wave, stub-defined at file scope with only their .string initialized
	 * -- see the CORRECTION note above this function) are left at .value==0
	 * from static init, not their real "4"/"100" defaults. SV_UserFriction's
	 * analytic-physics branch (sv_user.c:149) divides by log(r0) where
	 * r0 = 1 - friction*tau; with friction==0, r0==1.0 exactly, so
	 * log(stopspeed/ns)/log(r0) is a division by zero and every downstream
	 * velocity component goes NaN on both sides alike (masked, not caught, by
	 * bit-exact NaN-payload comparisons in the differential tests -- real
	 * mutation coverage of anything downstream of friction, e.g.
	 * SV_Accelerate's clamp, is impossible while velocities are NaN). Every
	 * other physics cvar sv_phys.c/sv_move.c reads is equally unset for the
	 * same reason, so the whole DEFAULTS block from
	 * sv_phys_differential.rs::Cvars::defaults() is applied here, once, for
	 * every sv_user.c test. */
	{
		static const float ctest_svuser_phys_cvars[13] = {
			4.0f,	 /* sv_friction */
			100.0f,	 /* sv_stopspeed */
			800.0f,	 /* sv_gravity */
			2000.0f, /* sv_maxvelocity */
			0.0f,	 /* sv_nostep */
			0.0f,	 /* sv_freezenonclients */
			0.0f,	 /* sv_gameplayfix_spawnbeforethinks */
			1.0f,	 /* sv_gameplayfix_bouncedownslopes */
			3.0f,	 /* sv_gameplayfix_elevators */
			1.0f,	 /* sv_fastpushmove */
			1.0f,	 /* sv_pushgrid */
			1.0f,	 /* sv_analyticphysics */
			1.0f,	 /* sv_speeds */
		};
		ctest_phys_set_cvars (ctest_svuser_phys_cvars);
	}

	sv_player = EDICT_NUM (1); /* plain twin; same shared arena */

	/* BUG FOUND AND FIXED THIS SESSION: ctest_phys_reset's own c_ref_svs.clients
	 * points at stubs.c's PRIVATE ctest_phys_clients array (its .edict/.spawned/
	 * .netconnection all zero -- only .active/.knowntoqc are set there, for
	 * sv_phys.c/sv_move.c's needs, not sv_user.c's). Left alone, c_ref_SV_RunClients
	 * would read host_client->edict == NULL and crash on sv_player->v.movetype.
	 * Both c_ref_svs.clients and the plain svs.clients below are therefore
	 * repointed at this file's OWN ctest_svuser_clients array -- one shared
	 * client_t array (client_t itself is not renamed) read/written by whichever
	 * side is currently driven, reset fresh before each side's run. */
	memset (ctest_svuser_clients, 0, sizeof (ctest_svuser_clients));
	for (i = 0; i < maxclients; i++)
	{
		ctest_svuser_clients[i].active = true;
		ctest_svuser_clients[i].spawned = true;
		ctest_svuser_clients[i].edict = EDICT_NUM (1);
		ctest_svuser_clients[i].netconnection = NULL;
	}

	c_ref_sv.active = true;
	c_ref_sv.paused = false;
	c_ref_sv.state = ss_active;
	c_ref_sv.protocol = PROTOCOL_FITZQUAKE;
	c_ref_sv.protocolflags = 0;
	c_ref_svs.maxclients = maxclients;
	c_ref_svs.clients = ctest_svuser_clients;

	/* sv/svs below are the plain (T6.5) copies -- undef'd file-wide above. */
	sv.active = true;
	sv.paused = false;
	sv.state = ss_active;
	sv.protocol = PROTOCOL_FITZQUAKE;
	sv.protocolflags = 0;
	svs.maxclients = maxclients;
	svs.clients = ctest_svuser_clients;

	host_client = maxclients > 0 ? &ctest_svuser_clients[0] : NULL;

	/* Both copies -- see the Phase 7 M7 note at the top of this file. */
	cls.netcon = NULL;
	c_ref_cls.netcon = NULL;
	cl_rollangle.value = 2.0f;
	cl_rollspeed.value = 200.0f;
	c_ref_cl_rollangle.value = 2.0f;
	c_ref_cl_rollspeed.value = 200.0f;

	key_dest = key_game;
}

/* Points one plain client's edict at a specific arena slot and sets its
 * active/spawned/netconnection state -- for tests that want more than one
 * distinct player (SV_RunClients' per-client dispatch). netconnection is an
 * opaque non-NULL sentinel (never dereferenced: NET_GetServerMessage always
 * returns NULL in this harness, and SV_ReadClientMessage/SV_ReadClientMove
 * are driven directly, not through the receive loop). */
static int ctest_svuser_netcon_sentinel;

void ctest_svuser_set_client (int slot, int edict_num, int active, int spawned, int has_netconnection)
{
	if (slot < 0 || slot >= CTEST_SVUSER_MAX_CLIENTS)
		return;
	ctest_svuser_clients[slot].edict = EDICT_NUM (edict_num);
	ctest_svuser_clients[slot].active = active ? true : false;
	ctest_svuser_clients[slot].spawned = spawned ? true : false;
	ctest_svuser_clients[slot].netconnection = has_netconnection ? (struct qsocket_s *)&ctest_svuser_netcon_sentinel : NULL;
}

void ctest_svuser_set_host_client (int slot)
{
	if (slot < 0 || slot >= CTEST_SVUSER_MAX_CLIENTS)
	{
		host_client = NULL;
		return;
	}
	host_client = &ctest_svuser_clients[slot];
}

/* SV_RunClients' "always pause in single player if in console or menus" gate
 * (sv_user.c:669) reads sv.paused and key_dest -- neither has a setter yet.
 * Both storages (oracle c_ref_sv.paused and plain sv.paused) are set so the
 * same call shapes fixture state identically for whichever side is about to
 * run; key_dest is a single shared (unrenamed) symbol, set once. */
void ctest_svuser_set_sv_paused (int paused)
{
	c_ref_sv.paused = paused ? true : false;
	sv.paused = paused ? true : false;
}

void ctest_svuser_set_key_dest (int kd)
{
	key_dest = kd;
}

/* scalars[8]: movetype, flags, waterlevel, watertype, health, teleport_time,
 *             idealpitch, fixangle
 * vectors[21]: origin[3], velocity[3], angles[3], v_angle[3], punchangle[3],
 *              view_ofs[3], movedir[3]
 * Field list and order match every field SV_ClientThink/SV_WaterMove/
 * SV_WaterJump/SV_AirMove/SV_UserFriction/SV_SetIdealPitch read or write on
 * sv_player->v (Quake/sv_user.c, re-read in full this session). */
void ctest_svuser_set_player (int num, const float *scalars, const float *vectors)
{
	edict_t *ed = EDICT_NUM (num);

	ed->v.movetype = scalars[0];
	ed->v.flags = scalars[1];
	ed->v.waterlevel = scalars[2];
	ed->v.watertype = scalars[3];
	ed->v.health = scalars[4];
	ed->v.teleport_time = scalars[5];
	ed->v.idealpitch = scalars[6];
	ed->v.fixangle = scalars[7];

	VectorCopy (vectors + 0, ed->v.origin);
	VectorCopy (vectors + 3, ed->v.velocity);
	VectorCopy (vectors + 6, ed->v.angles);
	VectorCopy (vectors + 9, ed->v.v_angle);
	VectorCopy (vectors + 12, ed->v.punchangle);
	VectorCopy (vectors + 15, ed->v.view_ofs);
	VectorCopy (vectors + 18, ed->v.movedir);
}

void ctest_svuser_set_cmd (int slot, float forwardmove, float sidemove, float upmove)
{
	if (slot < 0 || slot >= CTEST_SVUSER_MAX_CLIENTS)
		return;
	memset (&ctest_svuser_clients[slot].cmd, 0, sizeof (ctest_svuser_clients[slot].cmd));
	ctest_svuser_clients[slot].cmd.forwardmove = forwardmove;
	ctest_svuser_clients[slot].cmd.sidemove = sidemove;
	ctest_svuser_clients[slot].cmd.upmove = upmove;
}

/* Sets each cvar's .value on both storages: the oracle's c_ref_ copy
 * (spelled explicitly) and T6.5's plain copy (the bare name, undef'd file-
 * wide above). < 0 leaves a cvar untouched. */
void ctest_svuser_set_cvars (float altnoclip, float maxspeed, float accelerate, float idealpitchscale, float edgefriction)
{
	if (altnoclip >= 0)
	{
		c_ref_sv_altnoclip.value = altnoclip;
		sv_altnoclip.value = altnoclip;
	}
	if (maxspeed >= 0)
	{
		c_ref_sv_maxspeed.value = maxspeed;
		sv_maxspeed.value = maxspeed;
	}
	if (accelerate >= 0)
	{
		c_ref_sv_accelerate.value = accelerate;
		sv_accelerate.value = accelerate;
	}
	if (idealpitchscale >= 0)
	{
		c_ref_sv_idealpitchscale.value = idealpitchscale;
		sv_idealpitchscale.value = idealpitchscale;
	}
	if (edgefriction >= 0)
	{
		c_ref_sv_edgefriction.value = edgefriction;
		sv_edgefriction.value = edgefriction;
	}
}

/* --------------------------------------------------------------------------
 * net_message plumbing for SV_ReadClientMove/SV_ReadClientMessage (reached
 * through SV_RunClients -- see the module doc's coverage-strategy note).
 * Writes the SAME byte sequence into both net_message buffers: the oracle's
 * (c_ref_net_message, stub-owned by stubs.c under the renamed name) and this
 * file's plain twin, which quake-capi::net's MSG_Read* (linked in because
 * quake-ctest depends on quake-capi with the "net" feature) reads through
 * quake_c_sys::net_message.
 */
void ctest_svuser_load_message (const unsigned char *data, int len)
{
	static unsigned char c_buf[MAX_DATAGRAM];
	static unsigned char rust_buf[MAX_DATAGRAM];

	if (len < 0)
		len = 0;
	if (len > MAX_DATAGRAM)
		len = MAX_DATAGRAM;

	/* c_ref_net_message is already extern-declared (c_ref_prelude.h's own
	 * `extern sizebuf_t net_message;`, renamed by the still-active macro at
	 * the point the prelude declares it -- this file's own #undef net_message
	 * above only affects text after it, so that identifier is unaffected). */
	memcpy (c_buf, data, (size_t)len);
	c_ref_net_message.data = c_buf;
	c_ref_net_message.maxsize = MAX_DATAGRAM;
	c_ref_net_message.cursize = len;
	c_ref_net_message.allowoverflow = false;
	c_ref_net_message.overflowed = false;

	memcpy (rust_buf, data, (size_t)len);
	net_message.data = rust_buf; /* plain copy, this file's own */
	net_message.maxsize = MAX_DATAGRAM;
	net_message.cursize = len;
	net_message.allowoverflow = false;
	net_message.overflowed = false;
}

/* --------------------------------------------------------------------------
 * Read-back for the differential comparison. Same scalar/vector layout as
 * ctest_svuser_set_player, plus idealpitch's own COMPAT-relevant angles[]
 * (ROLL/PITCH/YAW) already included in the vectors[6..9) slot.
 */
void ctest_svuser_get_player (int num, float *scalars, float *vectors)
{
	edict_t *ed = EDICT_NUM (num);

	scalars[0] = ed->v.movetype;
	scalars[1] = ed->v.flags;
	scalars[2] = ed->v.waterlevel;
	scalars[3] = ed->v.watertype;
	scalars[4] = ed->v.health;
	scalars[5] = ed->v.teleport_time;
	scalars[6] = ed->v.idealpitch;
	scalars[7] = ed->v.fixangle;

	VectorCopy (ed->v.origin, vectors + 0);
	VectorCopy (ed->v.velocity, vectors + 3);
	VectorCopy (ed->v.angles, vectors + 6);
	VectorCopy (ed->v.v_angle, vectors + 9);
	VectorCopy (ed->v.punchangle, vectors + 12);
	VectorCopy (ed->v.view_ofs, vectors + 15);
	VectorCopy (ed->v.movedir, vectors + 18);
}

/* button0/button2/impulse (SV_ReadClientMove writes) plus v_angle, so a
 * (currently unreachable -- see the module doc's coverage-strategy note)
 * SV_ReadClientMove-through-SV_RunClients scenario could still be added
 * later without another round of fixture surgery. */
void ctest_svuser_get_player_buttons (int num, float *button0, float *button2, float *impulse)
{
	edict_t *ed = EDICT_NUM (num);
	*button0 = ed->v.button0;
	*button2 = ed->v.button2;
	*impulse = ed->v.impulse;
}

/* Reads back a client slot's cmd (SV_RunClients clears it when !spawned,
 * and republishes cmd.viewangles from v_angle when !netconnection). */
void ctest_svuser_get_cmd (int slot, float *forwardmove, float *sidemove, float *upmove, float *viewangles)
{
	if (slot < 0 || slot >= CTEST_SVUSER_MAX_CLIENTS)
	{
		*forwardmove = *sidemove = *upmove = 0;
		/* VectorClear is not a repo-wide macro (only r_part_fte.c defines its
		 * own local copy, not visible here), so this is spelled out. */
		viewangles[0] = viewangles[1] = viewangles[2] = 0;
		return;
	}
	*forwardmove = ctest_svuser_clients[slot].cmd.forwardmove;
	*sidemove = ctest_svuser_clients[slot].cmd.sidemove;
	*upmove = ctest_svuser_clients[slot].cmd.upmove;
	VectorCopy (ctest_svuser_clients[slot].cmd.viewangles, viewangles);
}

/* host_client's slot index within ctest_svuser_clients, or -1. SV_RunClients
 * leaves host_client pointed at the last-iterated client (svs.clients +
 * svs.maxclients - 1) when it returns, since its per-frame loop does not
 * restore the caller's host_client -- exactly like Quake/sv_user.c:647-671
 * itself; verifying this pointer walk matches is as much a part of the
 * differential contract as the edict state SV_ClientThink writes. */
int ctest_svuser_host_client_slot (void)
{
	int i;
	if (!host_client)
		return -1;
	for (i = 0; i < CTEST_SVUSER_MAX_CLIENTS; i++)
	{
		if (&ctest_svuser_clients[i] == host_client)
			return i;
	}
	return -1;
}
