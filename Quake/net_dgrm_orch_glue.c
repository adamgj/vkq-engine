/*
Copyright (C) 1996-2001 Id Software, Inc.
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

// net_dgrm_orch_glue.c -- C side of the Rust net_dgrm.c orchestration half
// (Rust migration Phase 7 M9b, T9.2). Compiled instead of net_dgrm.c under
// -Duse_rust_net (Pattern A whole-file swap); the Datagram_* names stay so the
// net_bsd.c / net_win.c driver vtables are untouched.
//
// This file is the orchestration half only. The Phase 5 net_dgrm_glue.c is a
// separate translation unit and still owns packetBuffer, the six packet
// counters and the eight reliable-wire slots -- none of them may be redefined
// here.
//
// Four jobs, the sv_user_glue.c shape:
//   1. own the C-visible objects net_dgrm.c defined that did not move to Rust
//      (the five cvar_t initializers, whose static data is the live data --
//      none of them is ever registered);
//   2. Host_Guard everything reachable that can Host_Error / Host_EndGame:
//      Cmd_ExecuteString, SV_DropClient, SV_ConnectClient, SCR_UpdateScreen
//      and the two Cmd_AddCommand seams (which are themselves Host_Reraise
//      wrappers under -Duse_rust_cvar);
//   3. Host_Reraise, from a pure C frame, what those guards caught. The
//      driver-table slots have no status channel of their own, so the vtable
//      wrapper is the raise frame: it takes an out-parameter, re-raises, and
//      only then returns;
//   4. leave the non-raising engine calls as ordinary externs.
//
// ADR-009 rule 3: no longjmp crosses a Rust frame. Every guarded callee runs
// inside a C trampoline that holds its arguments in a struct.
//
// COMPAT: on a caught raise the Rust cores return the status verbatim and run
// no cleanup -- no NET_FreeQSocket, no Close_Socket, no Con_Redirect (NULL),
// and m_return_onerror is left set. That leaks a qsocket and a system socket
// on every raise out of the connect path, and leaves the rcon redirect
// installed with an unflushed buffer. It is exactly what the C longjmp does;
// do not "fix" it.

#include "quakedef.h"
#include "q_stdinc.h"
#include "arch_def.h"
#include "net_sys.h"
#include "net_defs.h"
#include "net_dgrm.h"
#include "net_dgrm_int.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t

#include "quake_rs.h"

/* -- DEDUP (shared, whole-file) ------------------------------------------- */

/* Mirrors RUST_DGRM_NET_MESSAGE_OVERFLOW in net_dgrm_glue.c:44. A Rust
 * net_dgrm_orch entry point returns this instead of a Host_Guard status when
 * a reply write hit SZ_GetSpace with allowoverflow clear, which C raises from
 * inside SZ_GetSpace itself. Kept out of the 0/1/2 status space on purpose. */
#define RUST_DGRM_ORCH_NET_MESSAGE_OVERFLOW (-2)

/* Turn a Rust return value back into the raise C would have performed.
 * Host_Reraise (host.c:339) is a no-op for HOST_GUARD_OK. */
static void NetDgrmOrch_Reraise (int status)
{
	if (status == RUST_DGRM_ORCH_NET_MESSAGE_OVERFLOW)
		Host_Error ("SZ_GetSpace: overflow without allowoverflow set");
	Host_Reraise (status);
}

/* ---------------------------------------------------------------------------
 * 1. C-visible storage (net_dgrm.c:43-55).
 *
 * The five cvar_t objects stay C: cvar_t is a C ABI type with an engine-owned
 * `next` chain, and the initializers must stay byte-identical (the flags field
 * is omitted throughout, i.e. CVAR_NONE by zero-init -- preserved verbatim).
 *
 * NOTE for the merge: agents B and D also need these; exactly one copy lands
 * in the file. The comment block above them in net_dgrm.c:38-42 should come
 * across with them.
 */

/* cvars controlling dpmaster support:
   our servers might as well claim to be 'FTE-Quake' servers. this means FTE can see us, we can see FTE (when its pretending to be nq).
   we additionally look for 'DarkPlaces-Quake' servers too, because we can, but most of those servers will be using dpp7 and will (safely) not respond to our
   ccreq_server_info requests. we are not visible to DarkPlaces users - dp does not support fitz666 so that's not a viable option, at least by default, feel
   free to switch the order if you also change sv_protocol back to 15. */
cvar_t sv_reportheartbeats = {"sv_reportheartbeats", "0"};
cvar_t sv_public = {"sv_public", NULL};
cvar_t com_protocolname = {"com_protocolname", "FTE-Quake DarkPlaces-Quake"};
cvar_t net_masters[] = {
	{"net_master1", ""},
	{"net_master2", ""},
	{"net_master3", ""},
	{"net_master4", ""},
	{"net_masterextra1", "master.frag-net.com:27950"},
	{"net_masterextra2", "dpmaster.deathmask.net:27950"},
	{"net_masterextra3", "dpmaster.tchr.no:27950"},
	{NULL}};
cvar_t rcon_password = {"rcon_password", ""};

/* COMPAT: none of these five is ever passed to Cvar_RegisterVariable -- not
   in Datagram_Init and not anywhere else in the tree. Their `.value` is
   therefore permanently 0.0f and `sv_public.string` is permanently NULL.
   See m9b_a_notes.md; the M9b contract's claim that Datagram_Init registers
   them (and that the registration order is a config-dump gate) is wrong.
   Datagram_Init has three Cmd_AddCommand calls and zero
   Cvar_RegisterVariable calls. */

/* net_dgrm.c:231 -- registered by NetDgrmOrch_InvokeAddNetStatsCommand below,
   defined further down in this file. */
static void NET_Stats_f (void);

/* net_dgrm.c:744 -- the "\\ver\\" ENGINE_NAME_AND_VER literal, exported as data so
   the version field the Rust infoResponse builder appends is formed by the same
   preprocessor concatenation the C used (mirrors HostCmd_EngineVersionLine). */
const char *const NetDgrmOrch_GetInfoVerField = "\\ver\\" ENGINE_NAME_AND_VER;

/* net_dgrm.c:578 -- Cmd_AddCommand ("net_stats", NET_Stats_f). Under
   -Duse_rust_cvar the plain Cmd_AddCommand name is itself a Host_Reraise
   wrapper (precedent: SvMain_Glue_AddCommands, declared at
   rust/quake-c-sys/src/sv_main.rs:180-225), so Rust must reach it through a
   guard. */
static void NetDgrmOrch_InvokeAddNetStatsCommand (void *p)
{
	(void)p;
	Cmd_AddCommand ("net_stats", NET_Stats_f);
}

int NetDgrmOrch_Glue_AddNetStatsCommand (void)
{
	return Host_Guard (NetDgrmOrch_InvokeAddNetStatsCommand, NULL);
}

/* net_dgrm.c:598-599 -- the two test commands, registered in this order and
   only after the landriver init loop found at least one driver. One guard for
   the pair keeps the ordering atomic under a raise. */
static void NetDgrmOrch_InvokeAddTestCommands (void *p)
{
	(void)p;
	Cmd_AddCommand ("test", quake_rs_dgrm_test_f);
	Cmd_AddCommand ("test2", quake_rs_dgrm_test2_f);
}

int NetDgrmOrch_Glue_AddTestCommands (void)
{
	return Host_Guard (NetDgrmOrch_InvokeAddTestCommands, NULL);
}

/* net_dgrm.c:207-217 -- the timeout kick. SV_DropClient is a confirmed
   transitive raise site (host.c:590 PR_ExecuteProgram of
   pr_global_struct->ClientDisconnect), so the whole scan is guarded.

   It stays C for a second reason: svs and host_client live in
   quake-capi::sv_main, behind the `host` feature, while net_dgrm_orch.rs is
   behind `net`. The build-rs-chost CI leg builds use_rust_net ON with
   use_rust_host OFF, so Rust must not reach them.

   close_first selects the NET_Close-before-drop variant agent B needs at
   net_dgrm.c:1059; agent A's call site (:213) passes false. If agent B's site
   turns out to need a different shape, split this into two helpers rather
   than adding a second flag. */
typedef struct
{
	qsocket_t *sock;
	qboolean   close_first;
} netdgrmorch_dropclient_t;

static void NetDgrmOrch_InvokeDropClient (void *p)
{
	netdgrmorch_dropclient_t *a = (netdgrmorch_dropclient_t *)p;
	int						  i;

	for (i = 0; i < svs.maxclients; i++)
	{
		if (svs.clients[i].netconnection == a->sock)
		{
			/* net_dgrm.c:1057 closes early, to avoid svc_disconnects
			   confusing things. It sits inside the match, so a socket that
			   belongs to no client is not closed -- :213 has no close at all. */
			if (a->close_first)
				NET_Close (a->sock);
			host_client = &svs.clients[i];
			SV_DropClient (false);
			break;
		}
	}
}

int NetDgrmOrch_Glue_DropClient (qsocket_t *sock, qboolean close_first)
{
	netdgrmorch_dropclient_t a;
	a.sock = sock;
	a.close_first = close_first;
	return Host_Guard (NetDgrmOrch_InvokeDropClient, &a);
}

/* -- Host_Guard trampolines (ADR-009) --------------------------------------
 * Each Invoke* runs in a pure C frame, so a longjmp out of the engine call
 * never crosses a Rust frame. The wrapper returns Host_Guard's status
 * (HOST_GUARD_OK / HOST_GUARD_ABORTSERVER / HOST_GUARD_SCREEN_ERROR) for the
 * Rust caller to propagate up to a vtable wrapper, which re-raises. */

/* net_dgrm.c:936 -- Cmd_ExecuteString (MSG_ReadString (), src_command).
 * src_command is supplied here so the port never names cmd_source_t. The
 * qboolean result is discarded, as in the original. */
static void NetDgrmOrch_InvokeExecuteString (void *p)
{
	Cmd_ExecuteString (*(const char **)p, src_command);
}

int NetDgrmOrch_Glue_ExecuteString (const char *text)
{
	return Host_Guard (NetDgrmOrch_InvokeExecuteString, &text);
}

/* net_dgrm.c:1121 -- SV_ConnectClient (plnum). Deliberately NOT a direct
 * call to quake_rs_sv_connect_client: the build-rs-chost CI leg builds
 * -Duse_rust_net=true with -Duse_rust_host=false, where SV_ConnectClient is
 * still the C definition and the Rust export does not exist. */
static void NetDgrmOrch_InvokeConnectClient (void *p)
{
	SV_ConnectClient (*(int *)p);
}

int NetDgrmOrch_Glue_ConnectClient (int clientnum)
{
	return Host_Guard (NetDgrmOrch_InvokeConnectClient, &clientnum);
}

/* net_dgrm.c:1589, :1632, :1693 -- SCR_UpdateScreen (false), the three
 * redraws _Datagram_Connect performs (:1589 while waiting for a reply,
 * :1632 once one arrives and :1693 after "still trying..."), which is what
 * keeps the screen alive while the handshake is in flight.
 *
 * Shaped like sv_user_glue.c's SvUser_Glue_GetServerMessage: a static
 * ..._Invoke... (void *) plus an int-returning entry point that returns
 * Host_Guard (Invoke, arg).
 */

static void NetDgrmOrch_InvokeUpdateScreen (void *p)
{
	(void)p;
	SCR_UpdateScreen (false);
}

int NetDgrmOrch_Glue_UpdateScreen (void)
{
	return Host_Guard (NetDgrmOrch_InvokeUpdateScreen, NULL);
}

/* net_masters[m].string (net_dgrm.c:1291-1293). The C loop terminates on the
 * `{NULL}` sentinel's NULL string; the extra index guard is defensive only --
 * countof(net_masters) - 1 is the sentinel, so the NULL is always returned
 * first and behaviour is unchanged. */
const char *NetDgrmOrch_Glue_MasterString (size_t m)
{
	if (m >= countof (net_masters))
		return NULL;
	return net_masters[m].string;
}

/* com_protocolname.string (net_dgrm.c:1298) */
const char *NetDgrmOrch_Glue_ProtocolName (void)
{
	return com_protocolname.string;
}

/* net_dgrm.c:1301-1304, kept in C so the two format strings, the four
 * `%c` 255 bytes and va's VA_BUFFERLEN truncation are byte-identical.
 * va does not raise (q_vsnprintf into a rotating static buffer). */
const char *NetDgrmOrch_Glue_MasterQuery (int ipv6, const char *token, unsigned int protover)
{
	if (ipv6)
		return va ("%c%c%c%cgetserversExt %s %u empty full ipv6" /*\x0A\n"*/, 255, 255, 255, 255, token, protover);
	return va ("%c%c%c%cgetservers %s %u empty full" /*\x0A\n"*/, 255, 255, 255, 255, token, protover);
}

/* The platform's AF_INET / AF_INET6. quake_net::udp's copies are #[cfg(unix)]
 * and so are unreachable from the Rust net_dgrm_orch on the Windows leg;
 * net_sys.h has already pulled in the right socket headers here. */
int NetDgrmOrch_Glue_AfInet (void)
{
	return AF_INET;
}

int NetDgrmOrch_Glue_AfInet6 (void)
{
	return AF_INET6;
}

/* ---------------------------------------------------------------------------
 * 3. Entry points.
 */

/* net_dgrm.c:231 -- NET_Stats_f. Registered above from this same file, so it
   stays static exactly as the C had it. Nothing beneath the Rust body raises
   (Con_Printf, Cmd_Argc/Cmd_Argv, q_strcasecmp and the qsocket pool walk),
   hence no status channel. */
static void NET_Stats_f (void)
{
	quake_rs_dgrm_net_stats_f ();
}

/* net_dgrm.c:567 -- Datagram_Init. Raise frame for the two Cmd_AddCommand
   guards above. */
int Datagram_Init (void)
{
	int out = -1;
	Host_Reraise (quake_rs_dgrm_init (&out));
	return out;
}

/* net_dgrm.c:138 -- Datagram_GetAnyMessage. The vtable slot is
   `qsocket_t *(*QGetAnyMessage) (void)` with no status channel, so this
   wrapper is the raise frame (contract, "vtable wrapper shape"). It re-raises
   both the SV_DropClient guard above and agent B's
   _Datagram_ServerControlPacket guard (net_dgrm.c:189 -> SV_ConnectClient). */
qsocket_t *Datagram_GetAnyMessage (void)
{
	qsocket_t *out = NULL;
	Host_Reraise (quake_rs_dgrm_get_any_message (&out));
	return out;
}

/* net_dgrm.c:604 -- Datagram_Shutdown. No raise beneath it: the landriver
   Shutdown slots reach Sys_Error at worst, which aborts. */
void Datagram_Shutdown (void)
{
	quake_rs_dgrm_shutdown ();
}

/* net_dgrm.c:623 -- Datagram_Close. No raise beneath it. */
void Datagram_Close (qsocket_t *sock)
{
	quake_rs_dgrm_close (sock);
}

/* net_dgrm.c:634 -- Datagram_Listen. The Sys_Error at :663 aborts rather than
   longjmps, so this is a plain pass-through with no Host_Reraise. */
void Datagram_Listen (qboolean state)
{
	quake_rs_dgrm_listen (state);
}

/* net_dgrm.c:671 -- Datagram_Rcon_Flush, the Con_Redirect callback agent B
   installs at :936. In no header; the signature and calling convention must
   stay exactly what the redirect machinery expects, i.e.
   `void (*) (const char *)`. No raise beneath it: the packet is built in a
   Rust SizeBuf, so none of the C MSG_Write / SZ_ entry points (all of which
   Host_Error) is reached, and the landriver Write slot only Sys_Errors. */
void Datagram_Rcon_Flush (const char *text)
{
	quake_rs_dgrm_rcon_flush (text);
}

/* -- vtable wrapper: net_dgrm.c:1124 --------------------------------------
 * The one entry point in this agent's range that the net_driver_t vtable
 * names. _Datagram_ServerControlPacket is static in C and stays private to
 * the Rust module; its status is propagated by Datagram_GetAnyMessage
 * (agent A), whose own wrapper re-raises. */

qsocket_t *Datagram_CheckNewConnections (void)
{
	qsocket_t *out = NULL;
	NetDgrmOrch_Reraise (quake_rs_dgrm_check_new_connections (&out));
	return out;
}

/* --- vtable wrapper (net_win.c:61-63 / net_bsd.c:61-63 install this) ----- */

qboolean Datagram_SearchForHosts (qboolean xmit)
{
	/* No raise frame: nothing under this slot can reach Host_Error. */
	return quake_rs_dgrm_search_for_hosts (xmit);
}

/* ---------------------------------------------------------------------------
 * net_drivers[] vtable wrappers owned by C2 (net_win.c:61-63, net_bsd.c:61-63
 * install these symbols; the slot signatures carry no status channel, so the
 * C wrapper is the raise frame).
 */

/* net_dgrm.c:1819. The three SCR_UpdateScreen guards inside
   _Datagram_Connect propagate their status up through
   quake_rs_dgrm_connect, and this is the frame that re-issues the jump.
   `out` is left NULL on every raise path, so the `return out` below is only
   reached when Host_Reraise did nothing. */
qsocket_t *Datagram_Connect (const char *host)
{
	qsocket_t *out = NULL;
	Host_Reraise (quake_rs_dgrm_connect (host, &out));
	return out;
}

/* net_dgrm.c:1848. No raise is reachable below this one -- the only
   non-NULL QueryAddresses slots are rust_udp4_GetAddresses /
   rust_udp6_GetAddresses (net_bsd.c:80, :102; net_win.c likewise) and the
   loop driver defines Loop_QueryAddresses as NULL (net_loop.h:28) -- so the
   Rust core returns the result directly and there is nothing to re-raise. */
int Datagram_QueryAddresses (qhostaddr_t *addresses, int maxaddresses)
{
	return quake_rs_dgrm_query_addresses (addresses, maxaddresses);
}

/* net_dgrm.c:305-565 -- Test_Poll / Test_f / Test2_Poll / Test2_f need no glue
   of their own. They left no C-visible storage behind (all nine statics are
   Rust module statics now), they contain no raise site, and they are console
   command handlers rather than net_drivers[] slots, so Datagram_Init registers
   quake_rs_dgrm_test_f / quake_rs_dgrm_test2_f directly through the guard
   above. Their Sys_Error at :344 aborts rather than longjmps. */
