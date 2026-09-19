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

#include "quakedef.h"
#include "q_stdinc.h"
#include "arch_def.h"
#include "net_sys.h"
#include "net_defs.h"

/* Rust migration Phase 5 M9: the ADR-009-safe core of this file lives in
   quake-capi (rust_net_*); the dispatch funnels (NET_Connect/GetMessage/
   GetServerMessage / Send / SendToAll / Poll) and NET_Init/Shutdown stay C
   frames here -- Host_Error-capable code runs beneath them and a longjmp
   must never unwind a Rust frame. See the task plan's M9 audit note. */
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#define PrintSlistHeader  rust_net_PrintSlistHeader
#define PrintSlist		  rust_net_PrintSlist
#define PrintSlistTrailer rust_net_PrintSlistTrailer
#define NET_Listen_f	  rust_net_Listen_f
#define MaxPlayers_f	  rust_net_MaxPlayers_f
#define NET_Port_f		  rust_net_Port_f

qsocket_t *net_activeSockets = NULL;
qsocket_t *net_freeSockets = NULL;
int		   net_numsockets = 0;

qboolean ipv4Available = false;
qboolean ipv6Available = false;

int net_hostport;
int DEFAULTnet_hostport = 26000;

char my_ipv4_address[NET_NAMELEN];
char my_ipv6_address[NET_NAMELEN];

qboolean listening = false;

qboolean		  slistInProgress = false;
qboolean		  slist_silent = false;
enum slistScope_e slist_scope = SLIST_LOOP;

int net_activeconnections = 0;

int messagesSent = 0;
int messagesReceived = 0;
int unreliableMessagesSent = 0;
int unreliableMessagesReceived = 0;

cvar_t net_messagetimeout = {"net_messagetimeout", "300", CVAR_NONE};
cvar_t net_connecttimeout = {"net_connecttimeout", "10", CVAR_NONE}; // this might be a little brief, but we don't have a way to protect against smurf attacks.
cvar_t hostname = {"hostname", "UNNAMED", CVAR_SERVERINFO};

// these two macros are to make the code more readable
#define sfunc net_drivers[sock->driver]
#define dfunc net_drivers[net_driverlevel]

int net_driverlevel;

double net_time;

double SetNetTime (void)
{
	return rust_net_SetNetTime ();
}

/*
===================
NET_NewQSocket

Called by drivers when a new communications endpoint is required
The sequence and buffer fields will be filled in properly
===================
*/
qsocket_t *NET_NewQSocket (void)
{
	return rust_net_NewQSocket ();
}

void NET_FreeQSocket (qsocket_t *sock)
{
	rust_net_FreeQSocket (sock);
}

int NET_QSocketGetSequenceIn (const qsocket_t *s)
{
	return rust_net_QSocketGetSequenceIn (s);
}
int NET_QSocketGetSequenceOut (const qsocket_t *s)
{
	return rust_net_QSocketGetSequenceOut (s);
}
double NET_QSocketGetTime (const qsocket_t *s)
{
	return rust_net_QSocketGetTime (s);
}
const char *NET_QSocketGetTrueAddressString (const qsocket_t *s)
{
	return rust_net_QSocketGetTrueAddressString (s);
}
const char *NET_QSocketGetMaskedAddressString (const qsocket_t *s)
{
	return rust_net_QSocketGetMaskedAddressString (s);
}
qboolean NET_QSocketGetProQuakeAngleHack (const qsocket_t *s)
{
	return rust_net_QSocketGetProQuakeAngleHack (s);
}
void NET_QSocketSetMSS (qsocket_t *s, int mss)
{
	rust_net_QSocketSetMSS (s, mss);
}

/* svs/sv accessor funnels for the Rust side (server.h is not
   bindgen-clean) */
qboolean NetMain_SVActive (void)
{
	return sv.active;
}
int NetMain_MaxClients (void)
{
	return svs.maxclients;
}
int NetMain_MaxClientsLimit (void)
{
	return svs.maxclientslimit;
}
void NetMain_SetMaxClients (int n)
{
	svs.maxclients = n;
}

qboolean NetMain_ClsDedicated (void)
{
	return cls.state == ca_dedicated;
}

/* host_client walks svs.clients in NET_SendToAll. The Rust core assigns it
   at exactly the points the C for-headers did, so the value it is left on
   after each loop is the one the C code left behind. */
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

/* net_drivers[]/net_landrivers[] are incomplete array types here (sized by
   their initializers in net_bsd.c/net_win.c), so the Rust side cannot
   declare a truthful array extern for them. Handing out the base pointer
   from C instead gives the Rust pointer arithmetic provenance over the real
   object (ADR-004: the SAFETY obligation is discharged, not asserted). */
net_driver_t *NetMain_Drivers (void)
{
	return net_drivers;
}
net_landriver_t *NetMain_LanDrivers (void)
{
	return net_landrivers;
}

void NET_Slist_f (void)
{
	rust_net_Slist_f ();
}

void NET_SlistSort (void)
{
	rust_net_SlistSort ();
}
const char *NET_SlistPrintServer (size_t idx)
{
	return rust_net_SlistPrintServer (idx);
}
const char *NET_SlistPrintServerName (size_t idx)
{
	return rust_net_SlistPrintServerName (idx);
}

/*
===================
NET_Connect
===================
*/

size_t		hostCacheCount = 0;
hostcache_t hostcache[HOSTCACHESIZE];

qsocket_t *NET_Connect (const char *host)
{
	qsocket_t *out = NULL;
	Host_Reraise (rust_net_Connect (host, &out));
	return out;
}

/* ADR-009: dfunc.Connect reaches Datagram_Connect, which can Host_Error.
   The guard therefore wraps the vtable call itself rather than the whole
   funnel; net_driverlevel is read ambiently, exactly as the C loop did. */
typedef struct
{
	const char *host;
	qsocket_t **out;
} netmain_connect_t;

static void NetMain_InvokeDriverConnect (void *p)
{
	netmain_connect_t *a = (netmain_connect_t *)p;
	*a->out = dfunc.Connect (a->host);
}

int NetMain_Glue_DriverConnect (const char *host, qsocket_t **out)
{
	netmain_connect_t a;
	a.host = host;
	a.out = out;
	*out = NULL;
	return Host_Guard (NetMain_InvokeDriverConnect, &a);
}

/*
===================
NET_CheckNewConnections
===================
*/
qsocket_t *NET_CheckNewConnections (void)
{
	return rust_net_CheckNewConnections ();
}

/*
===================
NET_Close
===================
*/
void NET_Close (qsocket_t *sock)
{
	rust_net_Close (sock);
}

/*
=================
NET_GetMessage

If there is a complete message, return it in net_message

returns 0 if no data is waiting
returns 1 if a message was received
returns -1 if connection is invalid
=================
*/
int NET_GetMessage (qsocket_t *sock)
{
	int out = 0;
	Host_Reraise (rust_net_GetMessage (sock, &out));
	return out;
}

/* ADR-009: sfunc.QGetMessage reaches Datagram_GetMessage, which can
   Host_Error. `sock` is a local so the sfunc macro resolves here. */
typedef struct
{
	qsocket_t *sock;
	int		  *out;
} netmain_getmessage_t;

static void NetMain_InvokeQGetMessage (void *p)
{
	netmain_getmessage_t *a = (netmain_getmessage_t *)p;
	qsocket_t			 *sock = a->sock;
	*a->out = sfunc.QGetMessage (sock);
}

int NetMain_Glue_QGetMessage (qsocket_t *sock, int *out)
{
	netmain_getmessage_t a;
	a.sock = sock;
	a.out = out;
	*out = 0;
	return Host_Guard (NetMain_InvokeQGetMessage, &a);
}

/*
=================
NET_GetServerMessage

If there is a complete message, return it in net_message

returns the qsocket that the message was meant to be for.
=================
*/
qsocket_t *NET_GetServerMessage (void)
{
	qsocket_t *out = NULL;
	Host_Reraise (rust_net_GetServerMessage (&out));
	return out;
}

/* ADR-009: QGetAnyMessage reaches Datagram_GetAnyMessage, which can
   Host_Error. net_driverlevel is read ambiently, as the C loop did. */
static void NetMain_InvokeQGetAnyMessage (void *p)
{
	*(qsocket_t **)p = net_drivers[net_driverlevel].QGetAnyMessage ();
}

int NetMain_Glue_QGetAnyMessage (qsocket_t **out)
{
	*out = NULL;
	return Host_Guard (NetMain_InvokeQGetAnyMessage, out);
}

/*
Spike: This function is for the menus+status command
Just queries each driver's public addresses (which often requires system-specific calls)
*/
int NET_ListAddresses (qhostaddr_t *addresses, int maxaddresses)
{
	return rust_net_ListAddresses (addresses, maxaddresses);
}

/*
==================
NET_SendMessage

Try to send a complete length+message unit over the reliable stream.
returns 0 if the message cannot be delivered reliably, but the connection
		is still considered valid
returns 1 if the message was sent properly
returns -1 if the connection died
==================
*/
int NET_SendMessage (qsocket_t *sock, sizebuf_t *data)
{
	return rust_net_SendMessage (sock, data);
}

int NET_SendUnreliableMessage (qsocket_t *sock, sizebuf_t *data)
{
	return rust_net_SendUnreliableMessage (sock, data);
}

/*
==================
NET_CanSendMessage

Returns true or false if the given qsocket can currently accept a
message to be transmitted.
==================
*/
qboolean NET_CanSendMessage (qsocket_t *sock)
{
	return rust_net_CanSendMessage (sock);
}

int NET_SendToAll (sizebuf_t *data, double blocktime)
{
	int out = 0;
	Host_Reraise (rust_net_SendToAll (data, blocktime, &out));
	return out;
}

//=============================================================================

/*
====================
NET_Init
====================
*/

void NET_Init (void)
{
	Host_Reraise (rust_net_Init ());
}

/* ADR-009: net_drivers[].Init reaches Datagram_Init, and the cvar/command
   registrations reach the Rust cvar registry. Both
   guards sit in pure C frames; net_driverlevel is read ambiently. */
static void NetMain_InvokeDriverInit (void *p)
{
	*(int *)p = net_drivers[net_driverlevel].Init ();
}

int NetMain_Glue_DriverInit (int *out)
{
	*out = -1;
	return Host_Guard (NetMain_InvokeDriverInit, out);
}

static void NetMain_InvokeRegisterNetVars (void *p)
{
	(void)p;
	Cvar_RegisterVariable (&net_messagetimeout);
	Cvar_RegisterVariable (&net_connecttimeout);
	Cvar_RegisterVariable (&hostname);

	Cmd_AddCommand ("slist", NET_Slist_f);
	Cmd_AddCommand ("listen", NET_Listen_f);
	Cmd_AddCommand ("maxplayers", MaxPlayers_f);
	Cmd_AddCommand ("port", NET_Port_f);
}

int NetMain_Glue_RegisterNetVars (void)
{
	return Host_Guard (NetMain_InvokeRegisterNetVars, NULL);
}

/*
====================
NET_Shutdown
====================
*/

void NET_Shutdown (void)
{
	rust_net_Shutdown ();
}

void NET_Poll (void)
{
	rust_net_Poll ();
}

void SchedulePollProcedure (PollProcedure *proc, double timeOffset)
{
	rust_net_SchedulePollProcedure (proc, timeOffset);
}
