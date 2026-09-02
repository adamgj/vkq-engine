/* net_dgrm_orch_glue_ref.c -- ctest-link mirror of Quake/net_dgrm_orch_glue.c
 * (Rust migration Phase 7 M9b, T9.2).
 *
 * Quake/net_dgrm_orch_glue.c is a Meson-only translation unit: it is compiled
 * into the shipping binary under -Duse_rust_net and into nothing else. The
 * ctest binaries enable quake-capi's `net` feature, so quake-capi's
 * net_dgrm_orch module is in this link and every symbol it imports from the
 * glue has to resolve here or the test binary does not link (task-plan lesson
 * (ff): `cargo clippy --all-targets` type-checks but never links, so a
 * Pattern A flip can be green on all six Meson configs and still be broken).
 *
 * Scope: this file defines exactly the eighteen glue-owned symbols the link
 * probe reported unresolved -- the five cvar_t objects, the version-field
 * literal, the eleven NetDgrmOrch_Glue_* seams and Datagram_Rcon_Flush. It
 * deliberately does NOT transcribe the other net_drivers[] wrappers
 * (Datagram_Init, Datagram_GetAnyMessage, Datagram_CheckNewConnections,
 * Datagram_Connect, ...) or NetDgrmOrch_Reraise: nothing in this link names
 * them, and net_bsd.c / net_win.c are not oracle sources.
 *
 * There is no c_ref oracle for net_dgrm.c -- it is absent from build.rs's
 * C_SOURCES (T9.0 was scoped out of the plan), so no differential compares
 * the two sides and the guards' success paths are unobservable here, exactly
 * as with sv_user_ref.c's SvUser_Glue_DropClient.
 *
 * Callee spelling follows the host_cmd_glue_ref.c rule: every callee is
 * spelled the way Quake/net_dgrm_orch_glue.c spells it and no #undef appears
 * anywhere in this file, so c_ref_prelude.h's per-TU renames rewrite both
 * sides identically. Two consequences worth stating rather than discovering:
 *   - `svs` binds to c_ref_svs (prelude:1145) and `SV_ConnectClient` to
 *     c_ref_SV_ConnectClient (prelude:1156, defined by the sv_main.c oracle);
 *     the latter has no plain definition in this link at all, so the rename
 *     is the only spelling that links.
 *   - `Cmd_ExecuteString` binds to c_ref_Cmd_ExecuteString (prelude:362) and
 *     `Cmd_AddCommand` expands to c_ref_Cmd_AddCommand2, i.e. both register
 *     into cmd.c's one oracle table. That is the identity mapping of an
 *     unported dependency view_ref.c:417 already relies on; sv_user_ref.c's
 *     #undef exception exists to keep a *differential* from going vacuous and
 *     has no counterpart here.
 * host_client, SV_DropClient (stubs.c:6943), NET_Close (stubs.c:7814),
 * SCR_UpdateScreen (host_ref.c:256) and va (stubs.c:1186) are not renamed and
 * resolve under their plain names.
 *
 * AF_INET / AF_INET6, ENGINE_NAME_AND_VER, countof, src_command and qsocket_t
 * all arrive through the force-included prelude (net_sys.h:1073,
 * quakever.h:1217, common.h:865, cmd.h:373, net_defs.h:1075), so this file
 * needs no #includes of its own beyond the raise machinery below.
 */

/* stubs.c's raise machinery. The real engine declares these in host.h, which
 * c_ref_prelude.h does not pull in (host_cmd_glue_ref.c:80-81 does the same). */
extern int	Host_Guard (void (*fn) (void *), void *arg);
extern void Host_Reraise (int guard_result);

/* quake-capi/src/net_dgrm_orch.rs. net_dgrm_orch_glue.c gets these from the
 * generated quake_rs.h, which this link has no counterpart for; only the ones
 * this file actually names are declared. */
extern void quake_rs_dgrm_net_stats_f (void);
extern void quake_rs_dgrm_test_f (void);
extern void quake_rs_dgrm_test2_f (void);
extern void quake_rs_dgrm_rcon_flush (const char *text);

/* ---------------------------------------------------------------------------
 * 1. C-visible storage (Quake/net_dgrm_orch_glue.c:95-129, verbatim).
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

/* net_dgrm.c:744 -- the "\\ver\\" ENGINE_NAME_AND_VER literal, exported as data so
   the version field the Rust infoResponse builder appends is formed by the same
   preprocessor concatenation the C used (mirrors HostCmd_EngineVersionLine). */
const char *const NetDgrmOrch_GetInfoVerField = "\\ver\\" ENGINE_NAME_AND_VER;

/* ---------------------------------------------------------------------------
 * 2. Host_Guard trampolines (ADR-009), mirroring net_dgrm_orch_glue.c's own
 * bodies exactly.
 */

/* net_dgrm.c:231 -- NET_Stats_f, static there and here; registered by
   NetDgrmOrch_InvokeAddNetStatsCommand below. */
static void NET_Stats_f (void)
{
	quake_rs_dgrm_net_stats_f ();
}

static void NetDgrmOrch_InvokeAddNetStatsCommand (void *p)
{
	(void)p;
	Cmd_AddCommand ("net_stats", NET_Stats_f);
}

int NetDgrmOrch_Glue_AddNetStatsCommand (void)
{
	return Host_Guard (NetDgrmOrch_InvokeAddNetStatsCommand, NULL);
}

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

static void NetDgrmOrch_InvokeExecuteString (void *p)
{
	Cmd_ExecuteString (*(const char **)p, src_command);
}

int NetDgrmOrch_Glue_ExecuteString (const char *text)
{
	return Host_Guard (NetDgrmOrch_InvokeExecuteString, &text);
}

static void NetDgrmOrch_InvokeConnectClient (void *p)
{
	SV_ConnectClient (*(int *)p);
}

int NetDgrmOrch_Glue_ConnectClient (int clientnum)
{
	return Host_Guard (NetDgrmOrch_InvokeConnectClient, &clientnum);
}

static void NetDgrmOrch_InvokeUpdateScreen (void *p)
{
	(void)p;
	SCR_UpdateScreen (false);
}

int NetDgrmOrch_Glue_UpdateScreen (void)
{
	return Host_Guard (NetDgrmOrch_InvokeUpdateScreen, NULL);
}

/* ---------------------------------------------------------------------------
 * 3. Pure formatting seams (no raise reachable beneath any of them).
 */

const char *NetDgrmOrch_Glue_MasterString (size_t m)
{
	if (m >= countof (net_masters))
		return NULL;
	return net_masters[m].string;
}

const char *NetDgrmOrch_Glue_ProtocolName (void)
{
	return com_protocolname.string;
}

const char *NetDgrmOrch_Glue_MasterQuery (int ipv6, const char *token, unsigned int protover)
{
	if (ipv6)
		return va ("%c%c%c%cgetserversExt %s %u empty full ipv6" /*\x0A\n"*/, 255, 255, 255, 255, token, protover);
	return va ("%c%c%c%cgetservers %s %u empty full" /*\x0A\n"*/, 255, 255, 255, 255, token, protover);
}

int NetDgrmOrch_Glue_AfInet (void)
{
	return AF_INET;
}

int NetDgrmOrch_Glue_AfInet6 (void)
{
	return AF_INET6;
}

/* ---------------------------------------------------------------------------
 * 4. The one entry point outside the vtable that this link names.
 *
 * Datagram_Rcon_Flush is the Con_Redirect callback net_dgrm.c:936 installs;
 * quake-c-sys declares it, so it must exist here. Its `void (*) (const char *)`
 * shape is what the redirect machinery expects and must not drift.
 */
void Datagram_Rcon_Flush (const char *text)
{
	quake_rs_dgrm_rcon_flush (text);
}
