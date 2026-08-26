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

// net_dgrm_glue.c -- C trampolines for the Rust dgrm reliable layer
// (Rust migration Phase 5 M7). Compiled only with -Duse_rust_net, replacing
// net_dgrm_rel.c; the Datagram_* names stay so the net_bsd.c/net_win.c
// vtables and net_dgrm.c's orchestration half are untouched.
//
// ADR-009: raises stay in these C frames. The send paths are validated here
// (the DEBUG Sys_Errors, plus the release oversize guard where the C
// original memcpy'd blindly -- unreachable, engine sizebufs are allocated
// at <= NET_MAXMESSAGE); Datagram_GetMessage re-raises the Rust
// hostile-length status as the exact SZ_GetSpace Host_Error.

#include "quakedef.h"
#include "q_stdinc.h"
#include "arch_def.h"
#include "net_sys.h"
#include "net_defs.h"
#include "net_dgrm.h"
#include "net_dgrm_int.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

/* keep in sync with rust/quake-capi/src/net_dgrm.rs */
#define RUST_DGRM_NET_MESSAGE_OVERFLOW (-2)

/* the shared statics stay C-owned so net_dgrm.c's GetAnyMessage /
   NET_Stats_f and the Rust shims (quake-c-sys) see one copy */
dgrm_packet_t packetBuffer;

/* statistic counters */
int packetsSent = 0;
int packetsReSent = 0;
int packetsReceived = 0;
int receivedDuplicateCount = 0;
int shortPacketCount = 0;
int droppedDatagrams;

int Datagram_SendMessage (qsocket_t *sock, sizebuf_t *data)
{
#ifdef DEBUG
	if (data->cursize == 0)
		Sys_Error ("Datagram_SendMessage: zero length message");

	if (data->cursize > NET_MAXMESSAGE)
		Sys_Error ("Datagram_SendMessage: message too big: %u", data->cursize);

	if (sock->canSend == false)
		Sys_Error ("SendMessage: called with canSend == false");
#endif
	/* COMPAT: release C memcpy'd an oversize message into sendMessage (UB);
	   the port refuses, so fatal out here instead (unreachable) */
	if (data->cursize > NET_MAXMESSAGE)
		Sys_Error ("Datagram_SendMessage: message too big: %u", data->cursize);
	return rust_dgrm_SendMessage (sock, data);
}

int SendMessageNext (qsocket_t *sock)
{
	return rust_dgrm_SendMessageNext (sock);
}

int ReSendMessage (qsocket_t *sock)
{
	return rust_dgrm_ReSendMessage (sock);
}

qboolean Datagram_CanSendMessage (qsocket_t *sock)
{
	return rust_dgrm_CanSendMessage (sock);
}

qboolean Datagram_CanSendUnreliableMessage (qsocket_t *sock)
{
	return rust_dgrm_CanSendUnreliableMessage (sock);
}

int Datagram_SendUnreliableMessage (qsocket_t *sock, sizebuf_t *data)
{
#ifdef DEBUG
	if (data->cursize == 0)
		Sys_Error ("Datagram_SendUnreliableMessage: zero length message");

	if (data->cursize > MAX_DATAGRAM)
		Sys_Error ("Datagram_SendUnreliableMessage: message too big: %u", data->cursize);
#endif
	/* COMPAT: see Datagram_SendMessage */
	if (data->cursize > MAX_DATAGRAM)
		Sys_Error ("Datagram_SendUnreliableMessage: message too big: %u", data->cursize);
	return rust_dgrm_SendUnreliableMessage (sock, data);
}

qboolean Datagram_ProcessPacket (unsigned int length, qsocket_t *sock)
{
	return rust_dgrm_ProcessPacket (length, sock);
}

int Datagram_GetMessage (qsocket_t *sock)
{
	int ret = rust_dgrm_GetMessage (sock);
	if (ret == RUST_DGRM_NET_MESSAGE_OVERFLOW)
		Host_Error ("SZ_GetSpace: overflow without allowoverflow set");
	return ret;
}
