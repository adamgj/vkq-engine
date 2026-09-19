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

#ifndef __NET_DGRM_INT_H
#define __NET_DGRM_INT_H

/* Internal seam between net_dgrm.c (orchestration: connect handshake,
 * control packets, hostcache, heartbeats, stats command) and net_dgrm_rel.c
 * (the reliable/unreliable datagram wire layer). Split at Rust migration
 * Phase 5 M6 so the wire layer compiles standalone as the quake-ctest
 * differential oracle and swaps whole-file under -Duse_rust_net at M7. */

// this macro is to make the code more readable
#define sfunc net_landrivers[sock->landriver]

typedef struct
{
	unsigned int length;
	unsigned int sequence;
	byte		 data[MAX_DATAGRAM];
} dgrm_packet_t;

extern dgrm_packet_t packetBuffer;

/* statistic counters (printed by net_dgrm.c's NET_Stats_f) */
extern int packetsSent;
extern int packetsReSent;
extern int packetsReceived;
extern int receivedDuplicateCount;
extern int shortPacketCount;
extern int droppedDatagrams;

int		 SendMessageNext (qsocket_t *sock);
int		 ReSendMessage (qsocket_t *sock);
qboolean Datagram_ProcessPacket (unsigned int length, qsocket_t *sock);

#endif /* __NET_DGRM_INT_H */
