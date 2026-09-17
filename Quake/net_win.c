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

#include "net_dgrm.h"
#include "net_loop.h"
#ifdef USE_RUST_NET
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"
#endif

net_driver_t net_drivers[] = {
#ifdef USE_RUST_NET
	/* Rust migration Phase 5 M5: the loopback driver slots point at the Rust
	   implementation; Loop_SearchForHosts (hostcache/slist plumbing) stays C
	   until M9. Loop must stay driver 0 (IS_LOOP_DRIVER). */
	{.name = "Loopback",
	 .initialized = false,
	 .Init = rust_loop_Init,
	 .Listen = rust_loop_Listen,
	 .QueryAddresses = Loop_QueryAddresses,
	 .SearchForHosts = Loop_SearchForHosts,
	 .Connect = rust_loop_Connect,
	 .CheckNewConnections = rust_loop_CheckNewConnections,
	 .QGetAnyMessage = rust_loop_GetAnyMessage,
	 .QGetMessage = rust_loop_GetMessage,
	 .QSendMessage = rust_loop_SendMessage,
	 .SendUnreliableMessage = rust_loop_SendUnreliableMessage,
	 .CanSendMessage = rust_loop_CanSendMessage,
	 .CanSendUnreliableMessage = rust_loop_CanSendUnreliableMessage,
	 .Close = rust_loop_Close,
	 .Shutdown = rust_loop_Shutdown},
#else
	{"Loopback", false, Loop_Init, Loop_Listen, Loop_QueryAddresses, Loop_SearchForHosts, Loop_Connect, Loop_CheckNewConnections, Loop_GetAnyMessage,
	 Loop_GetMessage, Loop_SendMessage, Loop_SendUnreliableMessage, Loop_CanSendMessage, Loop_CanSendUnreliableMessage, Loop_Close, Loop_Shutdown},
#endif

	{"Datagram", false, Datagram_Init, Datagram_Listen, Datagram_QueryAddresses, Datagram_SearchForHosts, Datagram_Connect, Datagram_CheckNewConnections,
	 Datagram_GetAnyMessage, Datagram_GetMessage, Datagram_SendMessage, Datagram_SendUnreliableMessage, Datagram_CanSendMessage,
	 Datagram_CanSendUnreliableMessage, Datagram_Close, Datagram_Shutdown}};

const int net_numdrivers = countof (net_drivers);

#ifdef USE_RUST_NET
/* Rust migration Phase 9 M2: both Winsock landrivers point at the Rust
   implementation (quake-capi net_wins over quake-net::udp::sys::windows).
   Designated initializers so same-signature slots cannot swap silently.
   The IPv6 entry keeps the C table's IPPROTO_IPV6 guard: the Windows SDK
   defines IPPROTO_IPV6 as an enumerator, not a macro, so the MSVC/clang-cl
   oracle ships without the IPv6 landriver and the Rust table must too. */
net_landriver_t net_landrivers[] = {
	{.name = "Winsock TCPIP",
	 .initialized = false,
	 .controlSock = 0,
	 .Init = rust_udp4_Init,
	 .Shutdown = rust_udp4_Shutdown,
	 .Listen = rust_udp4_Listen,
	 .QueryAddresses = rust_udp4_GetAddresses,
	 .Open_Socket = rust_udp4_OpenSocket,
	 .Close_Socket = rust_udp_CloseSocket,
	 .Connect = rust_udp_Connect,
	 .CheckNewConnections = rust_udp4_CheckNewConnections,
	 .Read = rust_udp_Read,
	 .Write = rust_udp_Write,
	 .Broadcast = rust_udp4_Broadcast,
	 .AddrToString = rust_udp_AddrToString,
	 .StringToAddr = rust_udp4_StringToAddr,
	 .GetSocketAddr = rust_udp_GetSocketAddr,
	 .GetNameFromAddr = rust_udp4_GetNameFromAddr,
	 .GetAddrFromName = rust_udp4_GetAddrFromName,
	 .AddrCompare = rust_udp_AddrCompare,
	 .GetSocketPort = rust_udp_GetSocketPort,
	 .SetSocketPort = rust_udp_SetSocketPort},
#ifdef IPPROTO_IPV6
	{.name = "Winsock IPv6",
	 .initialized = false,
	 .controlSock = 0,
	 .Init = rust_udp6_Init,
	 .Shutdown = rust_udp6_Shutdown,
	 .Listen = rust_udp6_Listen,
	 .QueryAddresses = rust_udp6_GetAddresses,
	 .Open_Socket = rust_udp6_OpenSocket,
	 .Close_Socket = rust_udp_CloseSocket,
	 .Connect = rust_udp_Connect,
	 .CheckNewConnections = rust_udp6_CheckNewConnections,
	 .Read = rust_udp_Read,
	 .Write = rust_udp_Write,
	 .Broadcast = rust_udp6_Broadcast,
	 .AddrToString = rust_udp_AddrToString,
	 .StringToAddr = rust_udp6_StringToAddr,
	 .GetSocketAddr = rust_udp_GetSocketAddr,
	 .GetNameFromAddr = rust_udp6_GetNameFromAddr,
	 .GetAddrFromName = rust_udp6_GetAddrFromName,
	 .AddrCompare = rust_udp_AddrCompare,
	 .GetSocketPort = rust_udp_GetSocketPort,
	 .SetSocketPort = rust_udp_SetSocketPort},
#endif
};
#else
#include "net_wins.h"

net_landriver_t net_landrivers[] = {
	{"Winsock TCPIP",
	 false,
	 0,
	 WINIPv4_Init,
	 WINIPv4_Shutdown,
	 WINIPv4_Listen,
	 WINIPv4_GetAddresses,
	 WINIPv4_OpenSocket,
	 WINS_CloseSocket,
	 WINS_Connect,
	 WINIPv4_CheckNewConnections,
	 WINS_Read,
	 WINS_Write,
	 WINIPv4_Broadcast,
	 WINS_AddrToString,
	 WINIPv4_StringToAddr,
	 WINS_GetSocketAddr,
	 WINIPv4_GetNameFromAddr,
	 WINIPv4_GetAddrFromName,
	 WINS_AddrCompare,
	 WINS_GetSocketPort,
	 WINS_SetSocketPort},
#ifdef IPPROTO_IPV6
	{"Winsock IPv6",
	 false,
	 0,
	 WINIPv6_Init,
	 WINIPv6_Shutdown,
	 WINIPv6_Listen,
	 WINIPv6_GetAddresses,
	 WINIPv6_OpenSocket,
	 WINS_CloseSocket,
	 WINS_Connect,
	 WINIPv6_CheckNewConnections,
	 WINS_Read,
	 WINS_Write,
	 WINIPv6_Broadcast,
	 WINS_AddrToString,
	 WINIPv6_StringToAddr,
	 WINS_GetSocketAddr,
	 WINIPv6_GetNameFromAddr,
	 WINIPv6_GetAddrFromName,
	 WINS_AddrCompare,
	 WINS_GetSocketPort,
	 WINS_SetSocketPort},
#endif
};
#endif

const int net_numlandrivers = countof (net_landrivers);
