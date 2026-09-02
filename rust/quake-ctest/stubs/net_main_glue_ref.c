/* net_main_glue_ref.c -- ctest-link mirror of the Quake/net_main.c seams the
 * Rust dispatch funnels import (Rust migration Phase 7 M9c, T9.1).
 *
 * Quake/net_main.c is not in build.rs's C_SOURCES, so there is no c_ref
 * oracle for it and no differential compares the two sides. The ctest
 * binaries do enable quake-capi's `net` feature, though, so quake-capi's
 * net_main module is in this link and every symbol it imports has to resolve
 * here or the test binary does not link (task-plan lesson (ff): `cargo clippy
 * --all-targets` type-checks but never links).
 *
 * Scope: exactly the thirteen net_main.c-owned symbols the link probe
 * reported unresolved -- the four USE_RUST_NET accessor funnels and the five
 * ADR-009 Host_Guard trampolines, plus the two local definitions they need.
 * The plain data globals (slistInProgress, slist_silent, net_numsockets) and
 * the harness.c net-replay seam live on the Rust side in
 * quake-ctest/src/net_stubs.rs, next to the rest of net_main.c's ambient
 * globals.
 *
 * Callee spelling follows the host_cmd_glue_ref.c rule: every callee is
 * spelled the way Quake/net_main.c spells it under USE_RUST_NET and no #undef
 * appears anywhere in this file, so c_ref_prelude.h's per-TU renames rewrite
 * both sides identically. `svs` binds to c_ref_svs (prelude:1145), `cls` to
 * c_ref_cls (prelude:1619), `Cvar_RegisterVariable` to
 * c_ref_Cvar_RegisterVariable (prelude:321) and `Cmd_AddCommand` (a cmd.h
 * macro) to c_ref_Cmd_AddCommand2 (prelude:356), i.e. into cvar.c's and
 * cmd.c's one oracle table each -- the identity mapping of an unported
 * dependency that view_ref.c:417 and net_dgrm_orch_glue_ref.c already rely
 * on. host_client (stubs.c:808), hostname (stubs.c:2933),
 * net_messagetimeout / net_connecttimeout (stubs.c:3065-3066) and
 * net_driverlevel (net_stubs.rs) are not renamed and resolve under their
 * plain names.
 *
 * qsocket_t, net_driver_t, cvar_t, client_t and ca_dedicated all arrive
 * through the force-included prelude, so this file needs no #includes of its
 * own beyond the raise machinery below.
 */

/* stubs.c's raise machinery. The real engine declares these in quakedef.h,
 * which c_ref_prelude.h does not pull in (host_cmd_glue_ref.c:80-81 and
 * net_dgrm_orch_glue_ref.c:51-52 do the same). */
extern int	Host_Guard (void (*fn) (void *), void *arg);
extern void Host_Reraise (int guard_result);

/* quake-capi/src/net_main.rs. net_main.c gets these from the generated
 * quake_rs.h, which this link has no counterpart for; only the ones this file
 * actually names are declared. Under USE_RUST_NET net_main.c reaches the last
 * three through its own #defines (net_main.c:41-43), so registering them here
 * under their rust_net_ spelling is the same registration. */
extern void rust_net_Slist_f (void);
extern void rust_net_Listen_f (void);
extern void rust_net_MaxPlayers_f (void);
extern void rust_net_Port_f (void);

/* net_drivers[] is defined by its initializer in net_bsd.c / net_win.c, and
 * neither is an oracle source or otherwise in this link, so the trampolines
 * below would not resolve without a definition here. This deliberately is NOT
 * the array stubs.c's NetMain_Drivers() hands out (a file static there): the
 * two are unobservably distinct, because nothing in this link ever drives a
 * path that reads both and there is no net_main.c differential to go vacuous.
 * The extent matches net_stubs.rs's `net_numdrivers = 2`. */
net_driver_t net_drivers[2];

/* net_main.c:87-88 spells its two vtable calls `sfunc`/`dfunc`. Those names
 * are already taken here: the force-included prelude pulls in net_dgrm_int.h,
 * whose `sfunc` is net_landrivers[sock->landriver], and no #undef is allowed
 * in a c_ref mirror. The two calls below are therefore written out as the
 * net_main.c macros expand -- net_drivers[sock->driver] and
 * net_drivers[net_driverlevel] -- which is the same code the engine compiles.
 *
 * net_messagetimeout / net_connecttimeout are defined in net_main.c in the
 * engine (stubs.c:3065-3066 here) and declared by no header, so the
 * registration below needs its own declarations. `hostname` has one in
 * net.h:41. */
extern cvar_t net_messagetimeout;
extern cvar_t net_connecttimeout;

/* ---------------------------------------------------------------------------
 * 1. The svs/cls/host_client accessor funnels (net_main.c under
 * USE_RUST_NET): server.h and client.h are not bindgen-clean, so the Rust
 * side reaches this state through C.
 */

qboolean NetMain_ClsDedicated (void)
{
	return cls.state == ca_dedicated;
}

void NetMain_SetHostClient (int idx)
{
	host_client = svs.clients + idx;
}

qboolean NetMain_HostClientActive (void)
{
	return host_client->active;
}

qsocket_t *NetMain_HostClientConnection (void)
{
	return host_client->netconnection;
}

/* ---------------------------------------------------------------------------
 * 2. Host_Guard trampolines (ADR-009), mirroring net_main.c's own bodies
 * exactly. Each one wraps a single raise-capable vtable call rather than a
 * whole funnel, so the C frame the longjmp needs sits directly beneath the
 * callee and no jump crosses a Rust frame. The three level-indexed ones read
 * net_driverlevel ambiently, exactly as the C for-headers did.
 */

typedef struct
{
	qsocket_t *sock;
	int		  *out;
} netmain_getmessage_t;

static void NetMain_InvokeQGetMessage (void *p)
{
	netmain_getmessage_t *a = (netmain_getmessage_t *)p;
	qsocket_t			 *sock = a->sock;
	*a->out = net_drivers[sock->driver].QGetMessage (sock);
}

int NetMain_Glue_QGetMessage (qsocket_t *sock, int *out)
{
	netmain_getmessage_t a;
	a.sock = sock;
	a.out = out;
	*out = 0;
	return Host_Guard (NetMain_InvokeQGetMessage, &a);
}

static void NetMain_InvokeQGetAnyMessage (void *p)
{
	*(qsocket_t **)p = net_drivers[net_driverlevel].QGetAnyMessage ();
}

int NetMain_Glue_QGetAnyMessage (qsocket_t **out)
{
	*out = NULL;
	return Host_Guard (NetMain_InvokeQGetAnyMessage, out);
}

typedef struct
{
	const char *host;
	qsocket_t **out;
} netmain_connect_t;

static void NetMain_InvokeDriverConnect (void *p)
{
	netmain_connect_t *a = (netmain_connect_t *)p;
	*a->out = net_drivers[net_driverlevel].Connect (a->host);
}

int NetMain_Glue_DriverConnect (const char *host, qsocket_t **out)
{
	netmain_connect_t a;
	a.host = host;
	a.out = out;
	*out = NULL;
	return Host_Guard (NetMain_InvokeDriverConnect, &a);
}

static void NetMain_InvokeDriverInit (void *p)
{
	*(int *)p = net_drivers[net_driverlevel].Init ();
}

int NetMain_Glue_DriverInit (int *out)
{
	*out = -1;
	return Host_Guard (NetMain_InvokeDriverInit, out);
}

/* net_main.c's USE_RUST_NET wrapper for NET_Slist_f. Nothing else in this
 * link defines or names it, and net.h:112 already declares it, so it keeps
 * its external linkage. Registering the wrapper rather than rust_net_Slist_f
 * is what the engine does. */
void NET_Slist_f (void)
{
	rust_net_Slist_f ();
}

static void NetMain_InvokeRegisterNetVars (void *p)
{
	(void)p;
	Cvar_RegisterVariable (&net_messagetimeout);
	Cvar_RegisterVariable (&net_connecttimeout);
	Cvar_RegisterVariable (&hostname);

	Cmd_AddCommand ("slist", NET_Slist_f);
	Cmd_AddCommand ("listen", rust_net_Listen_f);
	Cmd_AddCommand ("maxplayers", rust_net_MaxPlayers_f);
	Cmd_AddCommand ("port", rust_net_Port_f);
}

int NetMain_Glue_RegisterNetVars (void)
{
	return Host_Guard (NetMain_InvokeRegisterNetVars, NULL);
}
