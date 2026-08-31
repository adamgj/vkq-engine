/*
Copyright (C) 1996-2001 Id Software, Inc.
Copyright (C) 2002-2009 John Fitzgibbons and others
Copyright (C) 2010-2014 QuakeSpasm developers
Copyright (C) 2016      Spike

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
// sv_main.c -- server main program

#include "quakedef.h"

server_t		sv;
server_static_t svs;

static char localmodels[MAX_MODELS][8]; // inline model names for precache

int sv_protocol = PROTOCOL_RMQ; // spike -- enough maps need this now that we can probably afford incompatibility with engines that still don't support 999
								// (vanilla was already broken) -- PROTOCOL_FITZQUAKE; //johnfitz
unsigned int sv_protocol_pext1 = PEXT1_SUPPORTED_SERVER; // spike
unsigned int sv_protocol_pext2 = PEXT2_SUPPORTED_SERVER; // spike

cvar_t sv_netsort = {"sv_netsort", "1", CVAR_NONE};
cvar_t sv_smoothplatformlerps = {"sv_smoothplatformlerps", "1", CVAR_NONE};

extern cvar_t nomonsters;

static void SV_Pext_f (void);

/*
===============
SV_Protocol_f
===============
*/
static void SV_Protocol_f (void)
{
	int			i;
	const char *s;
	int			prot, pext1, pext2;

	prot = sv_protocol;
	pext1 = sv_protocol_pext1;
	pext2 = sv_protocol_pext2;

	switch (Cmd_Argc ())
	{
	case 1:
		//"FTE+15" or "15", just to be explicit about it
		Con_Printf ("\"sv_protocol\" is \"%s%i\"\n", sv_protocol_pext2 ? "fte" : "", sv_protocol);
		break;
	case 2:
		s = Cmd_Argv (1);
		if (!q_strncasecmp (s, "FTE", 3))
		{
			s += 3;
			if (*s == '+' || *s == '-')
				s++;
			pext1 = PEXT1_SUPPORTED_SERVER;
			pext2 = PEXT2_SUPPORTED_SERVER;
		}
		else if (!q_strncasecmp (s, "+", 3))
		{
			s += 1;
			pext1 = PEXT1_SUPPORTED_SERVER;
			pext2 = PEXT2_SUPPORTED_SERVER;
		}
		else if (!q_strncasecmp (s, "Base", 4))
		{
			s += 4;
			if (*s == '+' || *s == '-')
				s++;
			pext1 = 0;
			pext2 = 0;
		}
		else if (*s == '-')
		{
			s++;
			pext1 = 0;
			pext2 = 0;
		}

		i = strtol (s, (char **)&s, 0);
		if (*s == '-')
		{
			pext1 = 0;
			pext2 = 0;
		}
		else if (*s == '+')
		{
			pext1 = PEXT1_SUPPORTED_SERVER;
			pext2 = PEXT2_SUPPORTED_SERVER;
		}

		if (i != PROTOCOL_NETQUAKE && i != PROTOCOL_FITZQUAKE && i != PROTOCOL_RMQ)
			Con_Printf (
				"sv_protocol must be %i or %i or %i.\nProtocol may be prefixed with FTE+ or Base- to enable/disable FTE extensions.\n", PROTOCOL_NETQUAKE,
				PROTOCOL_FITZQUAKE, PROTOCOL_RMQ);
		else
		{
			sv_protocol = i;
			sv_protocol_pext1 = pext1;
			sv_protocol_pext2 = pext2;
			if (sv.active)
			{
				if (prot == sv_protocol && pext1 == sv_protocol_pext1 && pext2 == sv_protocol_pext2)
					Con_Printf ("specified protocol already active.\n");
				else
					Con_Printf ("changes will not take effect until the next level load.\n");
			}
		}
		break;
	default:
		Con_SafePrintf ("usage: sv_protocol <protocol>\n");
		break;
	}
}

/*
===============
SV_Init
===============
*/
void SV_Init (void)
{
	int			  i;
	const char	 *p;
	extern cvar_t sv_maxvelocity;
	extern cvar_t sv_gravity;
	extern cvar_t sv_nostep;
	extern cvar_t sv_freezenonclients;
	extern cvar_t sv_gameplayfix_spawnbeforethinks;
	extern cvar_t sv_gameplayfix_bouncedownslopes;
	extern cvar_t sv_gameplayfix_elevators;
	extern cvar_t sv_fastpushmove;
	extern cvar_t sv_pushgrid;
	extern cvar_t sv_analyticphysics;
	extern cvar_t sv_friction;
	extern cvar_t sv_edgefriction;
	extern cvar_t sv_stopspeed;
	extern cvar_t sv_maxspeed;
	extern cvar_t sv_accelerate;
	extern cvar_t sv_idealpitchscale;
	extern cvar_t sv_aim;
	extern cvar_t sv_altnoclip; // johnfitz

	// FTE optimized world geometry checks
	extern cvar_t sv_fte_recursivehullckeck;
	extern cvar_t sv_fte_createareanode;

	Cvar_RegisterVariable (&sv_maxvelocity);
	Cvar_RegisterVariable (&sv_gravity);
	Cvar_RegisterVariable (&sv_friction);
	Cvar_SetCallback (&sv_gravity, Host_Callback_Notify);
	Cvar_SetCallback (&sv_friction, Host_Callback_Notify);
	Cvar_RegisterVariable (&sv_edgefriction);
	Cvar_RegisterVariable (&sv_stopspeed);
	Cvar_RegisterVariable (&sv_maxspeed);
	Cvar_SetCallback (&sv_maxspeed, Host_Callback_Notify);
	Cvar_RegisterVariable (&sv_accelerate);
	Cvar_RegisterVariable (&sv_idealpitchscale);
	Cvar_RegisterVariable (&sv_aim);
	Cvar_RegisterVariable (&sv_nostep);
	Cvar_RegisterVariable (&sv_freezenonclients);
	Cvar_RegisterVariable (&sv_gameplayfix_spawnbeforethinks);
	Cvar_RegisterVariable (&sv_gameplayfix_bouncedownslopes);
	Cvar_RegisterVariable (&sv_gameplayfix_elevators);
	Cvar_RegisterVariable (&sv_fastpushmove);
	Cvar_RegisterVariable (&sv_pushgrid);
	Cvar_RegisterVariable (&sv_analyticphysics);
	Cvar_RegisterVariable (&pr_checkextension);
	Cvar_RegisterVariable (&sv_altnoclip); // johnfitz
	Cvar_RegisterVariable (&sv_netsort);
	Cvar_RegisterVariable (&sv_smoothplatformlerps);

	Cvar_RegisterVariable (&sv_fte_recursivehullckeck);
	Cvar_RegisterVariable (&sv_fte_createareanode);

	Cmd_AddCommand ("pext", SV_Pext_f);
	Cmd_AddCommand ("sv_protocol", &SV_Protocol_f); // johnfitz

	for (i = 0; i < MAX_MODELS; i++)
		q_snprintf (localmodels[i], 8, "*%i", i);

	i = COM_CheckParm ("-protocol");
	if (i && i < com_argc - 1)
		sv_protocol = atoi (com_argv[i + 1]);
	switch (sv_protocol)
	{
	case PROTOCOL_NETQUAKE:
		p = "NetQuake";
		break;
	case PROTOCOL_FITZQUAKE:
		p = "FitzQuake";
		break;
	case PROTOCOL_RMQ:
		p = "RMQ";
		break;
	default:
		Sys_Error ("Bad protocol version request %i. Accepted values: %i, %i, %i.", sv_protocol, PROTOCOL_NETQUAKE, PROTOCOL_FITZQUAKE, PROTOCOL_RMQ);
		return; /* silence compiler */
	}
	Sys_Printf ("Server using protocol %i%s (%s%s)\n", sv_protocol, sv_protocol_pext2 ? "+" : "", sv_protocol_pext2 ? "FTE-" : "", p);
}

/*
=============================================================================

EVENT MESSAGES

=============================================================================
*/

/*
==================
SV_StartParticle

Make sure the event gets sent to all clients
==================
*/
void SV_StartParticle (vec3_t org, vec3_t dir, int color, int count)
{
	int i, v;

	if (sv.datagram.cursize > sv.datagram.maxsize - 18)
		return;
	MSG_WriteByte (&sv.datagram, svc_particle);
	MSG_WriteCoord (&sv.datagram, org[0], sv.protocolflags);
	MSG_WriteCoord (&sv.datagram, org[1], sv.protocolflags);
	MSG_WriteCoord (&sv.datagram, org[2], sv.protocolflags);
	for (i = 0; i < 3; i++)
	{
		v = dir[i] * 16;
		if (v > 127)
			v = 127;
		else if (v < -128)
			v = -128;
		MSG_WriteChar (&sv.datagram, v);
	}
	// count is sent through 1 byte, clamp it to 255
	if (count > 255.0f)
		count = 255.0f;
	MSG_WriteByte (&sv.datagram, count);

	MSG_WriteByte (&sv.datagram, color);
}

/*
==================
SV_StartSound

Each entity can have eight independant sound sources, like voice,
weapon, feet, etc.

Channel 0 is an auto-allocate channel, the others override anything
allready running on that entity/channel pair.
Volume is in 0-255.
Attenuation is in 0-4
An attenuation of 0 will play full volume everywhere in the level.
Larger attenuations will drop off.  (max 4 attenuation)

==================
*/
void SV_StartSound (edict_t *entity, float *origin, int channel, const char *sample, int volume, float attenuation)
{
	unsigned int sound_num, ent;
	int			 i, field_mask;
	int			 p;
	client_t	*client;

	if (volume < 0)
		Host_Error ("SV_StartSound: volume = %i", volume);
	else if (volume > 255)
	{
		volume = 255;
		Con_Printf ("SV_StartSound: volume = %i\n", volume);
	}

	if (attenuation < 0 || attenuation > 4)
		Host_Error ("SV_StartSound: attenuation = %f", attenuation);

	if (channel < 0 || channel > 255)
		Host_Error ("SV_StartSound: channel = %i", channel);
	else if (channel > 7)
		Con_DPrintf ("SV_StartSound: channel = %i\n", channel);

	// find precache number for sound
	for (sound_num = 1; sound_num < MAX_SOUNDS && sv.sound_precache[sound_num]; sound_num++)
	{
		if (!strcmp (sample, sv.sound_precache[sound_num]))
			break;
	}

	if (sound_num == MAX_SOUNDS || !sv.sound_precache[sound_num])
	{
		Con_Printf ("SV_StartSound: %s not precacheed\n", sample);
		return;
	}

	ent = NUM_FOR_EDICT (entity);

	field_mask = 0;
	if (volume != DEFAULT_SOUND_PACKET_VOLUME)
		field_mask |= SND_VOLUME;
	if (attenuation != DEFAULT_SOUND_PACKET_ATTENUATION)
		field_mask |= SND_ATTENUATION;

	// johnfitz -- PROTOCOL_FITZQUAKE
	if (ent >= 8192 || channel >= 8)
		field_mask |= SND_LARGEENTITY;
	if (sound_num >= 256)
		field_mask |= SND_LARGESOUND;
	// johnfitz

	for (p = 0; p < svs.maxclients; p++)
	{
		client = &svs.clients[p];
		if (!client->active || !client->spawned)
			continue;

		if (ent >= client->limit_entities)
			continue;
		if (sound_num >= client->limit_sounds)
			continue;
		// PROTOCOL_NETQUAKE do not support more than 256 sounds and/or 8192 entities.
		if ((field_mask & (SND_LARGEENTITY | SND_LARGESOUND)) && (sv.protocol == PROTOCOL_NETQUAKE))
			continue;

		if (client->datagram.cursize > client->datagram.maxsize - 22)
			continue;

		// directed messages go only to the entity the are targeted on
		MSG_WriteByte (&client->datagram, svc_sound);
		MSG_WriteByte (&client->datagram, field_mask);
		if (field_mask & SND_VOLUME)
			MSG_WriteByte (&client->datagram, volume);
		if (field_mask & SND_ATTENUATION)
			MSG_WriteByte (&client->datagram, attenuation * 64);

		// johnfitz -- PROTOCOL_FITZQUAKE
		if (field_mask & SND_LARGEENTITY)
		{
			if ((client->protocol_pext2 & PEXT2_REPLACEMENTDELTAS) && ent > 0x7fff)
			{
				MSG_WriteShort (&client->datagram, (ent >> 8) | 0x8000);
				MSG_WriteByte (&client->datagram, ent & 0xff);
			}
			else
				MSG_WriteShort (&client->datagram, ent);
			MSG_WriteByte (&client->datagram, channel);
		}
		else
			MSG_WriteShort (&client->datagram, (ent << 3) | channel);
		if (field_mask & SND_LARGESOUND)
			MSG_WriteShort (&client->datagram, sound_num);
		else
			MSG_WriteByte (&client->datagram, sound_num);
		// johnfitz

		for (i = 0; i < 3; i++)
		{
			if (origin)
				MSG_WriteCoord (&client->datagram, origin[i], sv.protocolflags);
			else
				MSG_WriteCoord (&client->datagram, entity->v.origin[i] + 0.5 * (entity->v.mins[i] + entity->v.maxs[i]), sv.protocolflags);
		}
	}
}

/*
==================
SV_LocalSound - for 2021 rerelease
==================
*/
void SV_LocalSound (client_t *client, const char *sample)
{
	int sound_num, field_mask;

	for (sound_num = 1; sound_num < MAX_SOUNDS && sv.sound_precache[sound_num]; sound_num++)
	{
		if (!strcmp (sample, sv.sound_precache[sound_num]))
			break;
	}
	if (sound_num == MAX_SOUNDS || !sv.sound_precache[sound_num])
	{
		Con_Printf ("SV_LocalSound: %s not precached\n", sample);
		return;
	}

	field_mask = 0;
	if (sound_num >= 256)
	{
		if (sv.protocol == PROTOCOL_NETQUAKE)
			return;
		field_mask = SND_LARGESOUND;
	}

	MSG_WriteByte (&client->message, svc_localsound);
	MSG_WriteByte (&client->message, field_mask);
	if (field_mask & SND_LARGESOUND)
		MSG_WriteShort (&client->message, sound_num);
	else
		MSG_WriteByte (&client->message, sound_num);
}

/*
==============================================================================

CLIENT SPAWNING

==============================================================================
*/

/*
================
SV_SendServerinfo

Sends the first message from the server to a connected client.
This will be sent on the initial connection and upon each server load.
================
*/
void SV_SendServerinfo (client_t *client)
{
	const char **s;
	char		 message[2048];
	unsigned int i; // johnfitz
	qboolean	 cantruncate;
	qboolean	 truncated = false;

	client->spawned = false; // need prespawn, spawn, etc

	// assume some safe defaults if we early out.
	client->limit_unreliable = 1024;
	client->limit_reliable = 8192;
	client->limit_entities = 0;
	client->limit_models = 0;
	client->limit_sounds = 0;

	if (!sv_protocol_pext2)
	{ // server disabled pext completely, don't bother trying.
		// make sure we try reenabling it again on the next map though.
		client->pextknown = false;
	}
	else if (!client->pextknown)
	{
		MSG_WriteByte (&client->message, svc_stufftext);
		MSG_WriteString (&client->message, "cmd pext\n");
		client->sendsignon = PRESPAWN_FLUSH;
		return;
	}
	client->protocol_pext2 &= sv_protocol_pext2;

	if (!(client->protocol_pext2 & PEXT2_REPLACEMENTDELTAS))
		client->protocol_pext2 &= ~PEXT2_PREDINFO; // stats can't be deltaed if there's no deltas, so just pretend its not supported on its own.

	// now we know their protocol, pick some real defaults that match the limits of the engine that most defines that protocol's limits.
	switch (client->protocol_pext2 ? PROTOCOL_FTE_PEXT2 : sv.protocol)
	{
	default: // eep
	case PROTOCOL_NETQUAKE:
		client->limit_unreliable = 1024;
		client->limit_reliable = 8192;
		if (sv_protocol_pext2 && NET_QSocketGetProQuakeAngleHack (client->netconnection))
			client->limit_entities = 2048; // proquake supports more so assume we can use that limit if angles are also available (but only if we're not
										   // being strict about protocols)
		else
			client->limit_entities = 600; // vanilla sucks.
		client->limit_models = 256;		  // single byte
		client->limit_sounds = 256;		  // single byte
		break;
	case PROTOCOL_FITZQUAKE: // fitzquake didn't get abused quite as much as later engines did.
		client->limit_unreliable = 32000;
		client->limit_reliable = 32000;
		client->limit_entities = 32000;
		client->limit_models = 2048;
		client->limit_sounds = 2048;
		break;
	case PROTOCOL_RMQ: // actually QS - a moving target, so use our server's limits.
		client->limit_unreliable = 32000;
		client->limit_reliable = 64000;
		client->limit_entities = 32000;
		client->limit_models = 2048;
		client->limit_sounds = 2048;
		break;
	case PROTOCOL_FTE_PEXT2:					   // not a real protocol in itself, used to indicate QSS's full limits. FTE will match or allow higher.
		client->limit_unreliable = NET_MAXMESSAGE; // some safe ethernet limit. these clients should accept pretty much anything, but any routers will not.
		client->limit_reliable = NET_MAXMESSAGE;   // adhere to fitzquake's limits if we're recording a demoquite large, ip allows 16 bits
		client->limit_entities = MAX_EDICTS;	   // we don't really know, 8k is probably a save guess but could be 32k, 65k, or even more...
		client->limit_models = MAX_MODELS;		   // not really sure, client's problem until >14bits
		client->limit_sounds = MAX_SOUNDS;		   // not really sure, client's problem until >14bits
		break;
	}

	if (!strcmp (NET_QSocketGetTrueAddressString (client->netconnection), "LOCAL"))
	{ // might as well super-size it. demo playback doesn't care. mostly only affects vanilla. we should trigger other warnings if this limit is exceeded so
	  // don't worry about testers.
		client->limit_unreliable = client->limit_reliable;
	}
	else
	{ // remote clients must not exceed ip MTUs.
		if (client->limit_unreliable > DATAGRAM_MTU)
			client->limit_unreliable = DATAGRAM_MTU;
	}
	if (client->limit_entities > 0x8000 && !(client->protocol_pext2 & PEXT2_REPLACEMENTDELTAS))
		client->limit_entities =
			0x8000; // pext2 changes the encoding of entities to support 23 bits instead of dpp7's 15bits or vanilla's 16bits, but our writeentity is lazy.
	if (client->limit_entities > (unsigned int)qcvm->max_edicts)
		client->limit_entities = (unsigned int)qcvm->max_edicts;

	// unfortunately we can't split this up, so if its oversized, we'll just let the client complain instead of always kicking them
	client->message.maxsize = sizeof (client->msgbuf);
	if (client->message.maxsize > (int)client->limit_reliable)
		client->message.maxsize = client->limit_reliable;
	if (client->datagram.maxsize > (int)client->limit_unreliable)
		client->datagram.maxsize = client->limit_unreliable;

	NET_QSocketSetMSS (client->netconnection, client->limit_unreliable);

	if (client->message.cursize)
	{ // try and flush the reliable NOW, in case the qc is evil
		if (NET_CanSendMessage (host_client->netconnection))
		{
			if (NET_SendMessage (host_client->netconnection, &host_client->message) != -1)
			{
				SZ_Clear (&host_client->message);
				host_client->last_message = realtime;
			}
		}
	}

	cantruncate = client->message.cursize == 0;
retry:
	MSG_WriteByte (&client->message, svc_print);
	//	q_snprintf (message, "%c\nFITZQUAKE %1.2f SERVER (%i CRC)\n", 2, FITZQUAKE_VERSION, pr_crc); //johnfitz -- include fitzquake version
	q_snprintf (
		message, sizeof (message), "%c\n" ENGINE_NAME_AND_VER " Server (%i CRC)\n", 2,
		qcvm->progscrc); // spike -- quakespasm has moved on, and has its own server capabilities now. Advertising = good, right?
	MSG_WriteString (&client->message, message);

	MSG_WriteByte (&client->message, svc_serverinfo);
	if (client->protocol_pext2)
	{ // pext stuff takes the form of modifiers to an underlaying protocol
		MSG_WriteLong (&client->message, PROTOCOL_FTE_PEXT2);
		MSG_WriteLong (&client->message, client->protocol_pext2); // active extensions that the client needs to look out for
	}
	MSG_WriteLong (&client->message, sv.protocol); // johnfitz -- sv.protocol instead of PROTOCOL_VERSION

	if (sv.protocol == PROTOCOL_RMQ)
	{
		// mh - now send protocol flags so that the client knows the protocol features to expect
		MSG_WriteLong (&client->message, sv.protocolflags);
	}

	if (client->protocol_pext2 & PEXT2_PREDINFO)
	{
		// if multiple gamedirs were used, we should list all the active ones eg: "id1;hipnotic;rogue;quoth;mod".
		// fixme: engine-specific forced gamedirs like id1/ or qw/ or fte/ are redundant, so don't bother listing them
		// we don't really track that stuff, so I'm just going to report the last one
		MSG_WriteString (&client->message, COM_GetGameNames (false));
	}

	MSG_WriteByte (&client->message, svs.maxclients);

	if (!coop.value && deathmatch.value)
		MSG_WriteByte (&client->message, GAME_DEATHMATCH);
	else
		MSG_WriteByte (&client->message, GAME_COOP);

	MSG_WriteString (&client->message, PR_GetString (qcvm->edicts->v.message));

	// johnfitz -- only send the first 256 model and sound precaches if protocol is 15
	for (i = 1, s = sv.model_precache + 1; *s && i < client->limit_models; s++, i++)
		MSG_WriteString (&client->message, *s);
	MSG_WriteByte (&client->message, 0);
	client->signon_models = i;

	// Spike: if we have svc_precache then use it for sounds. this reduces the stress on the serverinfo message size.
	if (host_client->protocol_pext2 && truncated)
		i = 1; // we tried, it didn't fit.
	else
		for (i = 1, s = sv.sound_precache + 1; *s && i < client->limit_sounds; s++, i++)
			MSG_WriteString (&client->message, *s);
	MSG_WriteByte (&client->message, 0);
	client->signon_sounds = i;
	// johnfitz

	// send music
	MSG_WriteByte (&client->message, svc_cdtrack);
	MSG_WriteByte (&client->message, qcvm->edicts->v.sounds);
	MSG_WriteByte (&client->message, qcvm->edicts->v.sounds);

	// set view
	MSG_WriteByte (&client->message, svc_setview);
	MSG_WriteShort (&client->message, NUM_FOR_EDICT (client->edict));

	MSG_WriteByte (&client->message, svc_signonnum);
	MSG_WriteByte (&client->message, 1);

	client->sendsignon = PRESPAWN_FLUSH;

	SVFTE_SetupFrames (client);

	if (client->message.overflowed && client->limit_models > 64 && cantruncate)
	{
		if (!host_client->protocol_pext2 || truncated)
		{ // first time around we can just drop sounds completely, filling them in later.
			// theoretically we can do the same with models too, but we don't entirely trust clients to handle lightmaps properly when its external bmodels.
			if (client->limit_models > client->limit_sounds || host_client->protocol_pext2)
				client->limit_models /= 2;
			else
				client->limit_sounds /= 2;
		}
		SZ_Clear (&client->message);
		truncated = true;
		goto retry;
	}

	// try and flush the reliable NOW, in case the qc is evil
	if (NET_CanSendMessage (client->netconnection))
	{
		if (NET_SendMessage (client->netconnection, &client->message) != -1)
		{
			SZ_Clear (&client->message);
			client->last_message = realtime;
			client->sendsignon = PRESPAWN_DONE;
		}
	}

	if (truncated)
		Con_Printf ("Protocol limitation (serverinfo) for %s\n", NET_QSocketGetTrueAddressString (client->netconnection));
}

void SV_Pext_f (void)
{
	// this only makes sense on the server. the clientside part only takes the form of 'cmd pext', for compat with clients that don't support this.
	if (cmd_source != src_client)
	{
		if (!cls.state)
		{
			Con_Printf ("Not connected\n");
			return;
		}
		Con_Printf ("Current Protocols:\n");
		if (cl.protocol_pext2 & PEXT2_REPLACEMENTDELTAS)
			Con_Printf ("  Replacement Entity Deltas\n");
		if (cl.protocol_pext2 & PEXT2_PREDINFO)
			Con_Printf ("  Replacement Stats ('predinfo')\n");
		if (cl.protocol == PROTOCOL_NETQUAKE)
			Con_Printf ("  vanilla(15)\n");
		else if (cl.protocol == PROTOCOL_FITZQUAKE)
			Con_Printf ("  fitzquake(666)\n");
		else if (cl.protocol == PROTOCOL_RMQ)
			Con_Printf ("  rmq(999)\n");
		else
			Con_Printf ("  unknown protocol(%i)\n", cl.protocol);
		return;
	}

	if (!host_client->pextknown && !host_client->spawned)
	{
		int i;
		int key;
		int value;
		for (i = 1; i < Cmd_Argc (); i += 2)
		{
			key = strtoul (Cmd_Argv (i), NULL, 0);
			value = strtoul (Cmd_Argv (i + 1), NULL, 0);

			if (key == PROTOCOL_FTE_PEXT2)
				host_client->protocol_pext2 = value & PEXT2_SUPPORTED_SERVER;
			// else some other extension that we don't know
		}

		host_client->pextknown = true;
		SV_SendServerinfo (host_client);
	}
}

/*
================
SV_ConnectClient

Initializes a client_t for a new net connection.  This will only be called
once for a player each game, not once for each level change.
================
*/
void SV_ConnectClient (int clientnum)
{
	edict_t			 *ent;
	client_t		 *client;
	int				  edictnum;
	struct qsocket_s *netconnection;
	int				  i;
	float			  spawn_parms[NUM_TOTAL_SPAWN_PARMS];

	client = svs.clients + clientnum;

	if (client->netconnection)
		Con_DPrintf ("Client %s connected\n", NET_QSocketGetTrueAddressString (client->netconnection));
	else
		Con_DPrintf ("Bot connected\n");

	edictnum = clientnum + 1;

	ent = EDICT_NUM (edictnum);

	// set up the client_t
	netconnection = client->netconnection;
	net_activeconnections++;

	if (sv.loadgame)
		memcpy (spawn_parms, client->spawn_parms, sizeof (spawn_parms));
	memset (client, 0, sizeof (*client));
	client->netconnection = netconnection;

	strcpy (client->name, "unconnected");
	client->active = true;
	client->spawned = false;
	client->edict = ent;
	client->message.data = client->msgbuf;
	client->message.maxsize = sizeof (client->msgbuf);
	client->message.allowoverflow = true; // we can catch it

	client->datagram.data = client->datagram_buf;
	client->datagram.maxsize = sizeof (client->datagram_buf);
	client->datagram.allowoverflow = true; // simply ignored on overflow

	client->pextknown = false;
	client->protocol_pext2 = 0;

	if (sv.loadgame)
		memcpy (client->spawn_parms, spawn_parms, sizeof (spawn_parms));
	else
	{
		// call the progs to get default spawn parms for the new client
		PR_ExecuteProgram (pr_global_struct->SetNewParms);
		for (i = 0; i < NUM_TOTAL_SPAWN_PARMS; i++)
			client->spawn_parms[i] = (&pr_global_struct->parm1)[i];
	}

	SV_SendServerinfo (client);
}

/*
===================
SV_CheckForNewClients

===================
*/
void SV_CheckForNewClients (void)
{
	struct qsocket_s *ret;
	int				  i;

	//
	// check for new connections
	//
	while (1)
	{
		ret = NET_CheckNewConnections ();
		if (!ret)
			break;

		//
		// init a new client structure
		//
		for (i = 0; i < svs.maxclients; i++)
			if (!svs.clients[i].active)
				break;
		if (i == svs.maxclients)
			Sys_Error ("Host_CheckForNewClients: no free clients");

		svs.clients[i].netconnection = ret;
		SV_ConnectClient (i);
	}
}

/*
===============================================================================

FRAME UPDATES

===============================================================================
*/

/*
==================
SV_ClearDatagram

==================
*/
void SV_ClearDatagram (void)
{
	SZ_Clear (&sv.datagram);
}

/*
==============================================================================

SERVER SPAWNING

==============================================================================
*/

/*
================
SV_ModelIndex

================
*/
int SV_ModelIndex (const char *name)
{
	int i;

	if (!name || !name[0])
		return 0;

	for (i = 0; i < MAX_MODELS && sv.model_precache[i]; i++)
		if (!strcmp (sv.model_precache[i], name))
			return i;
	if (i == MAX_MODELS || !sv.model_precache[i])
		Sys_Error ("SV_ModelIndex: model %s not precached", name);
	return i;
}

/*
================
SV_SaveSpawnparms

Grabs the current state of each client for saving across the
transition to another level
================
*/
void SV_SaveSpawnparms (void)
{
	int i, j;

	svs.serverflags = pr_global_struct->serverflags;

	for (i = 0, host_client = svs.clients; i < svs.maxclients; i++, host_client++)
	{
		if (!host_client->active)
			continue;

		// call the progs to get default spawn parms for the new client
		pr_global_struct->self = EDICT_TO_PROG (host_client->edict);
		PR_ExecuteProgram (pr_global_struct->SetChangeParms);
		for (j = 0; j < NUM_BASIC_SPAWN_PARMS; j++)
			host_client->spawn_parms[j] = (&pr_global_struct->parm1)[j];
		for (; j < NUM_TOTAL_SPAWN_PARMS; j++)
		{
			ddef_t *g = ED_FindGlobal (va ("parm%i", j + 1));
			host_client->spawn_parms[j] = g ? qcvm->globals[g->ofs] : 0;
		}
	}
}

// used for sv.qcvm.GetModel (so ssqc+csqc can share builtins)
qmodel_t *SV_ModelForIndex (int index)
{
	if (index < 0 || index >= MAX_MODELS)
		return NULL;
	return sv.models[index];
}

/*
================
SV_SpawnServer

This is called at the start of each level
================
*/
void SV_SpawnServer (const char *server)
{
	static char dummy[8] = {0, 0, 0, 0, 0, 0, 0, 0};
	edict_t	   *ent;
	int			i;
	qcvm_t	   *vm = qcvm;

	// let's not have any servers with no name
	if (hostname.string[0] == 0)
		Cvar_Set ("hostname", "UNNAMED");
	SCR_CenterPrintClear ();

	Con_DPrintf ("SpawnServer: %s\n", server);
	svs.changelevel_issued = false; // now safe to issue another

	PR_SwitchQCVM (NULL);

	//
	// tell all connected clients that we are going to a new level
	//
	if (sv.active)
		SV_SendReconnect ();

	//
	// make cvars consistant
	//
	if (coop.value)
		Cvar_Set ("deathmatch", "0");
	current_skill = (int)(skill.value + 0.5);
	if (current_skill < 0)
		current_skill = 0;
	if (current_skill > 3)
		current_skill = 3;

	Cvar_SetValue ("skill", (float)current_skill);

	//
	// set up the new server
	//
	// memset (&sv, 0, sizeof(sv));
	Host_ClearMemory ();

	q_strlcpy (sv.name, server, sizeof (sv.name));

	sv.protocol = sv_protocol; // johnfitz

	if (sv.protocol == PROTOCOL_RMQ)
	{
		// set up the protocol flags used by this server
		// (note - these could be cvar-ised so that server admins could choose the protocol features used by their servers)
		if (sv_protocol_pext2) // spike: I don't really want to step on anyone's toes, but floats have the exact same precision as qc does.
			sv.protocolflags = PRFL_FLOATCOORD | PRFL_SHORTANGLE;
		else // spike: purists might want to preserve the inprecision and just extend the range though. This matches vanilla QS. should compress a bit better
			 // too.
			sv.protocolflags = PRFL_INT32COORD | PRFL_SHORTANGLE;
	}
	else
		sv.protocolflags = 0;

	PR_SwitchQCVM (vm);
	// load progs to get entity field count
	PR_LoadProgs ("progs.dat", true, PROGHEADER_CRC, pr_ssqcbuiltins, pr_ssqcnumbuiltins);

	// allocate server memory
	/* Host_ClearMemory() called above already cleared the whole sv structure */
	qcvm->max_edicts = CLAMP (MIN_EDICTS, (int)max_edicts.value, MAX_EDICTS);  // johnfitz -- max_edicts cvar
	qcvm->edicts = (edict_t *)Mem_Alloc (qcvm->max_edicts * qcvm->edict_size); // ericw -- sv.edicts switched to use malloc()

#if defined(DEBUG) || defined(_DEBUG)
	for (int j = 0; j < qcvm->max_edicts; j++)
	{
		// set debug fiels for all max_edicts
		edict_t *e = EDICT_NUM_NO_CHECK (j);
		e->qcvm_owner = qcvm;
		e->edict_ptr = e;
		e->edict_num = j;
	}
#endif

	sv.datagram.maxsize = sizeof (sv.datagram_buf);
	sv.datagram.cursize = 0;
	sv.datagram.data = sv.datagram_buf;

	sv.multicast.maxsize = sizeof (sv.multicast_buf);
	sv.multicast.cursize = 0;
	sv.multicast.data = sv.multicast_buf;

	sv.reliable_datagram.maxsize = sizeof (sv.reliable_datagram_buf);
	sv.reliable_datagram.cursize = 0;
	sv.reliable_datagram.data = sv.reliable_datagram_buf;

	sv.signon.maxsize = sizeof (sv.signon_buf);
	sv.signon.cursize = 0;
	sv.signon.data = sv.signon_buf;

	// leave slots at start for clients only:
	qcvm->num_edicts = qcvm->reserved_edicts = svs.maxclients + 1;

	for (i = 0; i < svs.maxclients; i++)
	{
		// skip entity 0 = World, initialized below:
		ent = EDICT_NUM (i + 1);
		assert (!ent->free);
		svs.clients[i].edict = ent;
	}

	sv.state = ss_loading;
	sv.paused = false;
	sv.nomonsters = (nomonsters.value != 0.f);

	qcvm->time = 1.0;

	q_strlcpy (sv.name, server, sizeof (sv.name));
	q_snprintf (sv.modelname, sizeof (sv.modelname), "maps/%s.bsp", server);
	qcvm->worldmodel = Mod_ForName (sv.modelname, false);
	if (!qcvm->worldmodel || qcvm->worldmodel->type != mod_brush)
	{
		Con_Printf ("Couldn't spawn server %s\n", sv.modelname);
		sv.active = false;
		return;
	}
	sv.models[1] = qcvm->worldmodel;
	qcvm->GetModel = SV_ModelForIndex;

	//
	// clear world interaction links
	//
	SV_ClearWorld ();

	sv.sound_precache[0] = dummy;
	sv.model_precache[0] = dummy;
	sv.model_precache[1] = sv.modelname;
	if (qcvm->worldmodel->numsubmodels > MAX_MODELS)
	{
		Con_Printf ("too many inline models %s\n", sv.modelname);
		sv.active = false;
		return;
	}
	for (i = 1; i < qcvm->worldmodel->numsubmodels; i++)
	{
		sv.model_precache[1 + i] = localmodels[i];
		sv.models[i + 1] = Mod_ForName (localmodels[i], false);
	}

	//
	// load the rest of the entities:
	//

	// Initialize entity 0 = World
	ent = EDICT_NUM (0);
	memset (&ent->v, 0, qcvm->progs->entityfields * 4);
	ent->free = false;
	ent->v.model = PR_SetEngineString (qcvm->worldmodel->name);
	ent->v.modelindex = 1; // world model
	ent->v.solid = SOLID_BSP;
	ent->v.movetype = MOVETYPE_PUSH;

	if (coop.value)
		pr_global_struct->coop = coop.value;
	else
		pr_global_struct->deathmatch = deathmatch.value;

	pr_global_struct->mapname = PR_SetEngineString (sv.name);

	// serverflags are for cross level information (sigils)
	pr_global_struct->serverflags = svs.serverflags;

	ED_LoadFromFile (qcvm->worldmodel->entities);

	sv.active = true;

	SV_Precache_Model ("progs/player.mdl"); // Spike -- SV_CreateBaseline depends on this model.

	// all setup is completed, any further precache statements are errors
	sv.state = ss_active;

	// run two frames to allow everything to settle
	host_frametime = 0.1;
	SV_Physics ();
	SV_Physics ();

	// create a baseline for more efficient communications
	SV_CreateBaseline ();

	// johnfitz -- warn if signon buffer larger than standard server can handle
	if (sv.signon.cursize > 8000 - 2) // max size that will fit into 8000-sized client->message buffer with 2 extra bytes on the end
		Con_DWarning ("%i byte signon buffer exceeds standard limit of 7998 (max = %d).\n", sv.signon.cursize, sv.signon.maxsize);
	// johnfitz

	// send serverinfo to all connected clients
	for (i = 0, host_client = svs.clients; i < svs.maxclients; i++, host_client++)
	{
		host_client->knowntoqc = false;
		if (host_client->active)
			SV_SendServerinfo (host_client);
	}

	Con_DPrintf ("Server spawned.\n");
}
