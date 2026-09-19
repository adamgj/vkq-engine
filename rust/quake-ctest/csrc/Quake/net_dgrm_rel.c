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

// net_dgrm_rel.c -- the reliable/unreliable datagram wire layer, moved
// verbatim from net_dgrm.c (Rust migration Phase 5 M6; only the file
// statics became externally visible via net_dgrm_int.h so the two halves
// keep sharing them)

#include "quakedef.h"
#include "q_stdinc.h"
#include "arch_def.h"
#include "net_sys.h"
#include "net_defs.h"
#include "net_dgrm.h"
#include "net_dgrm_int.h"

/* statistic counters */
int packetsSent = 0;
int packetsReSent = 0;
int packetsReceived = 0;
int receivedDuplicateCount = 0;
int shortPacketCount = 0;
int droppedDatagrams;

dgrm_packet_t packetBuffer;

int Datagram_SendMessage (qsocket_t *sock, sizebuf_t *data)
{
	unsigned int packetLen;
	unsigned int dataLen;
	unsigned int eom;

#ifdef DEBUG
	if (data->cursize == 0)
		Sys_Error ("Datagram_SendMessage: zero length message");

	if (data->cursize > NET_MAXMESSAGE)
		Sys_Error ("Datagram_SendMessage: message too big: %u", data->cursize);

	if (sock->canSend == false)
		Sys_Error ("SendMessage: called with canSend == false");
#endif

	memcpy (sock->sendMessage, data->data, data->cursize);
	sock->sendMessageLength = data->cursize;

	sock->max_datagram = sock->pending_max_datagram; // this can apply only at the start of a reliable, to avoid issues with acks if its resized later.

	if (data->cursize <= sock->max_datagram)
	{
		dataLen = data->cursize;
		eom = NETFLAG_EOM;
	}
	else
	{
		dataLen = sock->max_datagram;
		eom = 0;
	}
	packetLen = NET_HEADERSIZE + dataLen;

	packetBuffer.length = BigLong (packetLen | (NETFLAG_DATA | eom));
	packetBuffer.sequence = BigLong (sock->sendSequence++);
	memcpy (packetBuffer.data, sock->sendMessage, dataLen);

	sock->canSend = false;

	if (sfunc.Write (sock->socket, (byte *)&packetBuffer, packetLen, &sock->addr) == -1)
		return -1;

	sock->lastSendTime = net_time;
	packetsSent++;
	return 1;
}

int SendMessageNext (qsocket_t *sock)
{
	unsigned int packetLen;
	unsigned int dataLen;
	unsigned int eom;

	if (sock->sendMessageLength <= sock->max_datagram)
	{
		dataLen = sock->sendMessageLength;
		eom = NETFLAG_EOM;
	}
	else
	{
		dataLen = sock->max_datagram;
		eom = 0;
	}
	packetLen = NET_HEADERSIZE + dataLen;

	packetBuffer.length = BigLong (packetLen | (NETFLAG_DATA | eom));
	packetBuffer.sequence = BigLong (sock->sendSequence++);
	memcpy (packetBuffer.data, sock->sendMessage, dataLen);

	sock->sendNext = false;

	if (sfunc.Write (sock->socket, (byte *)&packetBuffer, packetLen, &sock->addr) == -1)
		return -1;

	sock->lastSendTime = net_time;
	packetsSent++;
	return 1;
}

int ReSendMessage (qsocket_t *sock)
{
	unsigned int packetLen;
	unsigned int dataLen;
	unsigned int eom;

	if (sock->sendMessageLength <= sock->max_datagram)
	{
		dataLen = sock->sendMessageLength;
		eom = NETFLAG_EOM;
	}
	else
	{
		dataLen = sock->max_datagram;
		eom = 0;
	}
	packetLen = NET_HEADERSIZE + dataLen;

	packetBuffer.length = BigLong (packetLen | (NETFLAG_DATA | eom));
	packetBuffer.sequence = BigLong (sock->sendSequence - 1);
	memcpy (packetBuffer.data, sock->sendMessage, dataLen);

	if (sfunc.Write (sock->socket, (byte *)&packetBuffer, packetLen, &sock->addr) == -1)
		return -1;

	sock->lastSendTime = net_time;
	packetsReSent++;
	return 1;
}

qboolean Datagram_CanSendMessage (qsocket_t *sock)
{
	if (sock->sendNext)
		SendMessageNext (sock);

	return sock->canSend;
}

qboolean Datagram_CanSendUnreliableMessage (qsocket_t *sock)
{
	return true;
}

int Datagram_SendUnreliableMessage (qsocket_t *sock, sizebuf_t *data)
{
	int packetLen;

#ifdef DEBUG
	if (data->cursize == 0)
		Sys_Error ("Datagram_SendUnreliableMessage: zero length message");

	if (data->cursize > MAX_DATAGRAM)
		Sys_Error ("Datagram_SendUnreliableMessage: message too big: %u", data->cursize);
#endif

	packetLen = NET_HEADERSIZE + data->cursize;

	packetBuffer.length = BigLong (packetLen | NETFLAG_UNRELIABLE);
	packetBuffer.sequence = BigLong (sock->unreliableSendSequence++);
	memcpy (packetBuffer.data, data->data, data->cursize);

	if (sfunc.Write (sock->socket, (byte *)&packetBuffer, packetLen, &sock->addr) == -1)
		return -1;

	packetsSent++;
	return 1;
}

qboolean Datagram_ProcessPacket (unsigned int length, qsocket_t *sock)
{
	unsigned int flags;
	unsigned int sequence;
	unsigned int count;

	if (length < NET_HEADERSIZE)
	{
		shortPacketCount++;
		return false;
	}

	length = BigLong (packetBuffer.length);
	flags = length & (~NETFLAG_LENGTH_MASK);
	length &= NETFLAG_LENGTH_MASK;

	if (flags & NETFLAG_CTL)
		return false; // should only be for OOB packets.

	sequence = BigLong (packetBuffer.sequence);
	packetsReceived++;

	if (flags & NETFLAG_UNRELIABLE)
	{
		if (sequence < sock->unreliableReceiveSequence)
		{
			Con_DPrintf ("Got a stale datagram\n");
			return false;
		}
		if (sequence != sock->unreliableReceiveSequence)
		{
			count = sequence - sock->unreliableReceiveSequence;
			droppedDatagrams += count;
			Con_DPrintf ("Dropped %u datagram(s)\n", count);
		}
		sock->unreliableReceiveSequence = sequence + 1;

		length -= NET_HEADERSIZE;

		if (length > (unsigned int)net_message.maxsize)
		{ // is this even possible? maybe it will be in the future! either way, no sys_errors please.
			Con_Printf ("Over-sized unreliable\n");
			return true;
		}
		SZ_Clear (&net_message);
		SZ_Write (&net_message, packetBuffer.data, length);

		unreliableMessagesReceived++;
		return true; // parse the unreliable
	}

	if (flags & NETFLAG_ACK)
	{
		if (sequence != (sock->sendSequence - 1))
		{
			Con_DPrintf ("Stale ACK received\n");
			return false;
		}
		if (sequence == sock->ackSequence)
		{
			sock->ackSequence++;
			if (sock->ackSequence != sock->sendSequence)
				Con_DPrintf ("ack sequencing error\n");
		}
		else
		{
			Con_DPrintf ("Duplicate ACK received\n");
			return false;
		}
		sock->sendMessageLength -= sock->max_datagram;
		if (sock->sendMessageLength > 0)
		{
			memmove (sock->sendMessage, sock->sendMessage + sock->max_datagram, sock->sendMessageLength);
			sock->sendNext = true;
		}
		else
		{
			sock->sendMessageLength = 0;
			sock->canSend = true;
		}
		return false;
	}

	if (flags & NETFLAG_DATA)
	{
		packetBuffer.length = BigLong (NET_HEADERSIZE | NETFLAG_ACK);
		packetBuffer.sequence = BigLong (sequence);
		sfunc.Write (sock->socket, (byte *)&packetBuffer, NET_HEADERSIZE, &sock->addr);

		if (sequence != sock->receiveSequence)
		{
			receivedDuplicateCount++;
			return false;
		}
		sock->receiveSequence++;

		length -= NET_HEADERSIZE;

		if (flags & NETFLAG_EOM)
		{
			if (sock->receiveMessageLength + length > (unsigned int)net_message.maxsize)
			{
				Con_Printf ("Over-sized reliable\n");
				return true;
			}
			SZ_Clear (&net_message);
			SZ_Write (&net_message, sock->receiveMessage, sock->receiveMessageLength);
			SZ_Write (&net_message, packetBuffer.data, length);
			sock->receiveMessageLength = 0;

			messagesReceived++;
			return true; // parse this reliable!
		}

		if (sock->receiveMessageLength + length > sizeof (sock->receiveMessage))
		{
			Con_Printf ("Over-sized reliable\n");
			return true;
		}
		memcpy (sock->receiveMessage + sock->receiveMessageLength, packetBuffer.data, length);
		sock->receiveMessageLength += length;
		return false; // still watiting for the eom
	}
	// unknown flags
	Con_DPrintf ("Unknown packet flags\n");
	return false;
}

int Datagram_GetMessage (qsocket_t *sock)
{
	unsigned int	 length;
	unsigned int	 flags;
	int				 ret = 0;
	struct qsockaddr readaddr;
	unsigned int	 sequence;
	unsigned int	 count;

	if (!sock->canSend)
		if ((net_time - sock->lastSendTime) > 1.0)
			ReSendMessage (sock);

	while (1)
	{
		length = (unsigned int)sfunc.Read (sock->socket, (byte *)&packetBuffer, NET_DATAGRAMSIZE, &readaddr);

		//	if ((rand() & 255) > 220)
		//		continue;

		if (length == 0)
			break;

		if (length == (unsigned int)-1)
		{
			Con_Printf ("Read error\n");
			return -1;
		}

		if (sfunc.AddrCompare (&readaddr, &sock->addr) != 0)
		{
			Con_Printf ("Stray/Forged packet received\n");
			Con_Printf ("Expected: %s\n", sfunc.AddrToString (&sock->addr, false));
			Con_Printf ("Received: %s\n", sfunc.AddrToString (&readaddr, false));
			continue;
		}

		if (length < NET_HEADERSIZE)
		{
			shortPacketCount++;
			continue;
		}

		length = BigLong (packetBuffer.length);
		flags = length & (~NETFLAG_LENGTH_MASK);
		length &= NETFLAG_LENGTH_MASK;

		if (flags & NETFLAG_CTL)
			continue;

		sequence = BigLong (packetBuffer.sequence);
		packetsReceived++;

		if (flags & NETFLAG_UNRELIABLE)
		{
			if (sequence < sock->unreliableReceiveSequence)
			{
				Con_DPrintf ("Got a stale datagram\n");
				ret = 0;
				break;
			}
			if (sequence != sock->unreliableReceiveSequence)
			{
				count = sequence - sock->unreliableReceiveSequence;
				droppedDatagrams += count;
				Con_DPrintf ("Dropped %u datagram(s)\n", count);
			}
			sock->unreliableReceiveSequence = sequence + 1;

			length -= NET_HEADERSIZE;

			SZ_Clear (&net_message);
			SZ_Write (&net_message, packetBuffer.data, length);

			ret = 2;
			break;
		}

		if (flags & NETFLAG_ACK)
		{
			if (sequence != (sock->sendSequence - 1))
			{
				Con_DPrintf ("Stale ACK received\n");
				continue;
			}
			if (sequence == sock->ackSequence)
			{
				sock->ackSequence++;
				if (sock->ackSequence != sock->sendSequence)
					Con_DPrintf ("ack sequencing error\n");
			}
			else
			{
				Con_DPrintf ("Duplicate ACK received\n");
				continue;
			}
			sock->sendMessageLength -= sock->max_datagram;
			if (sock->sendMessageLength > 0)
			{
				memmove (sock->sendMessage, sock->sendMessage + sock->max_datagram, sock->sendMessageLength);
				sock->sendNext = true;
			}
			else
			{
				sock->sendMessageLength = 0;
				sock->canSend = true;
			}
			continue;
		}

		if (flags & NETFLAG_DATA)
		{
			packetBuffer.length = BigLong (NET_HEADERSIZE | NETFLAG_ACK);
			packetBuffer.sequence = BigLong (sequence);
			sfunc.Write (sock->socket, (byte *)&packetBuffer, NET_HEADERSIZE, &readaddr);

			if (sequence != sock->receiveSequence)
			{
				receivedDuplicateCount++;
				continue;
			}
			sock->receiveSequence++;

			length -= NET_HEADERSIZE;

			if (flags & NETFLAG_EOM)
			{
				if (sock->receiveMessageLength + length > (unsigned int)net_message.maxsize)
				{
					Con_Printf ("Over-sized reliable\n");
					return -1;
				}
				SZ_Clear (&net_message);
				SZ_Write (&net_message, sock->receiveMessage, sock->receiveMessageLength);
				SZ_Write (&net_message, packetBuffer.data, length);
				sock->receiveMessageLength = 0;

				ret = 1;
				break;
			}

			if (sock->receiveMessageLength + length > sizeof (sock->receiveMessage))
			{
				Con_Printf ("Over-sized reliable\n");
				return -1;
			}
			memcpy (sock->receiveMessage + sock->receiveMessageLength, packetBuffer.data, length);
			sock->receiveMessageLength += length;
			continue;
		}
	}

	if (sock->sendNext)
		SendMessageNext (sock);

	return ret;
}
