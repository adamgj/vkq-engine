/*
Copyright (C) 1996-1997 Id Software, Inc.
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

#include "q_stdinc.h"
#include "arch_def.h"
#include "net_sys.h"
#include "quakedef.h"
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

#include "net_udp.h"

net_landriver_t net_landrivers[] = {
#ifdef USE_RUST_NET
	/* Rust migration Phase 5 M7b: both UDP landrivers point at the Rust
	   implementation (quake-capi net_udp over quake-net::udp). Designated
	   initializers so same-signature slots cannot swap silently. */
	{.name = "UDP",
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
	 .GetNameFromAddr = rust_udp_GetNameFromAddr,
	 .GetAddrFromName = rust_udp4_GetAddrFromName,
	 .AddrCompare = rust_udp_AddrCompare,
	 .GetSocketPort = rust_udp_GetSocketPort,
	 .SetSocketPort = rust_udp_SetSocketPort},
	{.name = "UDP6",
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
	 .GetNameFromAddr = rust_udp_GetNameFromAddr,
	 .GetAddrFromName = rust_udp6_GetAddrFromName,
	 .AddrCompare = rust_udp_AddrCompare,
	 .GetSocketPort = rust_udp_GetSocketPort,
	 .SetSocketPort = rust_udp_SetSocketPort}};
#else
	{"UDP",
	 false,
	 0,
	 UDP4_Init,
	 UDP4_Shutdown,
	 UDP4_Listen,
	 UDP4_GetAddresses,
	 UDP4_OpenSocket,
	 UDP_CloseSocket,
	 UDP_Connect,
	 UDP4_CheckNewConnections,
	 UDP_Read,
	 UDP_Write,
	 UDP4_Broadcast,
	 UDP_AddrToString,
	 UDP4_StringToAddr,
	 UDP_GetSocketAddr,
	 UDP_GetNameFromAddr,
	 UDP4_GetAddrFromName,
	 UDP_AddrCompare,
	 UDP_GetSocketPort,
	 UDP_SetSocketPort},
	{"UDP6",
	 false,
	 0,
	 UDP6_Init,
	 UDP6_Shutdown,
	 UDP6_Listen,
	 UDP6_GetAddresses,
	 UDP6_OpenSocket,
	 UDP_CloseSocket,
	 UDP_Connect,
	 UDP6_CheckNewConnections,
	 UDP_Read,
	 UDP_Write,
	 UDP6_Broadcast,
	 UDP_AddrToString,
	 UDP6_StringToAddr,
	 UDP_GetSocketAddr,
	 UDP_GetNameFromAddr,
	 UDP6_GetAddrFromName,
	 UDP_AddrCompare,
	 UDP_GetSocketPort,
	 UDP_SetSocketPort}};
#endif

const int net_numlandrivers = (sizeof (net_landrivers) / sizeof (net_landrivers[0]));
