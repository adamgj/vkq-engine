/*
Copyright (C) 1996-2001 Id Software, Inc.
Copyright (C) 2002-2009 John Fitzgibbons and others
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
// net_msg.c -- message wire IO (MSG_Read*/MSG_Write*/SZ_*), split from
// common.c in Rust migration Phase 5 M2 (behavior-neutral: the section moved
// verbatim) so the file can be compiled standalone as the quake-ctest
// differential reference and gated whole under -Duse_rust_net at M3.

#include "quakedef.h"

/*
==============================================================================

			MESSAGE IO FUNCTIONS

Handles byte ordering and avoids alignment errors
==============================================================================
*/

//
// writing functions
//

void MSG_WriteChar (sizebuf_t *sb, int c)
{
	byte *buf;

#if defined(DEBUG) || defined(_DEBUG)
	if (c < -128 || c > 127)
		Host_Error ("MSG_WriteChar: range error = %i not in -128..127", c);
#endif

	buf = (byte *)SZ_GetSpace (sb, 1);
	buf[0] = c;
}

void MSG_WriteByte (sizebuf_t *sb, int c)
{
	byte *buf;

#if defined(DEBUG) || defined(_DEBUG)
	if (c < 0 || c > 255)
		Host_Error ("MSG_WriteByte: range error = %i not in 0..255", c);
#endif

	buf = (byte *)SZ_GetSpace (sb, 1);
	buf[0] = c;
}

void MSG_WriteShort (sizebuf_t *sb, int c)
{
	byte *buf;

#if defined(DEBUG) || defined(_DEBUG)
	// it is apparently used to encode signed OR unsigned shorts...
	if (c < INT16_MIN || c > UINT16_MAX)
		Host_Error ("MSG_WriteShort: range error = %i not in -32768..65535", c);
#endif

	buf = (byte *)SZ_GetSpace (sb, 2);
	buf[0] = c & 0xff;
	buf[1] = c >> 8;
}

void MSG_WriteLong (sizebuf_t *sb, int c)
{
	byte *buf;

	buf = (byte *)SZ_GetSpace (sb, 4);
	buf[0] = c & 0xff;
	buf[1] = (c >> 8) & 0xff;
	buf[2] = (c >> 16) & 0xff;
	buf[3] = c >> 24;
}

void MSG_WriteUInt64 (sizebuf_t *sb, unsigned long long c)
{ // 0* 10*,*, 110*,*,* etc, up to 0xff followed by 8 continuation bytes
	byte			  *buf;
	int				   b = 0;
	unsigned long long l = 128;
	while (c > l - 1u)
	{ // count the extra bytes we need
		b++;
		l <<= 7; // each byte we add gains 8 bits, but we spend one on length.
	}
	buf = (byte *)SZ_GetSpace (sb, 1 + b);
	*buf++ = 0xffu << (8 - b) | (c >> (b * 8));
	while (b-- > 0)
		*buf++ = (c >> (b * 8)) & 0xff;
}
void MSG_WriteInt64 (sizebuf_t *sb, long long c)
{ // move the sign bit into the low bit and avoid sign extension for more efficient length coding.
	if (c < 0)
		MSG_WriteUInt64 (sb, ((unsigned long long)(-1 - c) << 1) | 1);
	else
		MSG_WriteUInt64 (sb, c << 1);
}

void MSG_WriteFloat (sizebuf_t *sb, float f)
{
	union
	{
		float f;
		int	  l;
	} dat;

	dat.f = f;
	dat.l = LittleLong (dat.l);

	SZ_Write (sb, &dat.l, 4);
}

void MSG_WriteDouble (sizebuf_t *sb, double f)
{
	union
	{
		double	f;
		int64_t l;
	} dat;
	byte *o = SZ_GetSpace (sb, sizeof (f));
	dat.f = f;

	o[0] = dat.l >> 0;
	o[1] = dat.l >> 8;
	o[2] = dat.l >> 16;
	o[3] = dat.l >> 24;
	o[4] = dat.l >> 32;
	o[5] = dat.l >> 40;
	o[6] = dat.l >> 48;
	o[7] = dat.l >> 56;
}

void MSG_WriteString (sizebuf_t *sb, const char *s)
{
	if (!s)
		SZ_Write (sb, "", 1);
	else
		SZ_Write (sb, s, strlen (s) + 1);
}
void MSG_WriteStringUnterminated (sizebuf_t *sb, const char *s)
{
	SZ_Write (sb, s, strlen (s));
}

// johnfitz -- original behavior, 13.3 fixed point coords, max range +-4096
void MSG_WriteCoord16 (sizebuf_t *sb, float f)
{
	MSG_WriteShort (sb, Q_rint (f * 8));
}

// johnfitz -- 16.8 fixed point coords, max range +-32768
void MSG_WriteCoord24 (sizebuf_t *sb, float f)
{
	MSG_WriteShort (sb, f);
	MSG_WriteByte (sb, (int)(f * 255) % 255);
}

// johnfitz -- 32-bit float coords
void MSG_WriteCoord32f (sizebuf_t *sb, float f)
{
	MSG_WriteFloat (sb, f);
}

void MSG_WriteCoord (sizebuf_t *sb, float f, unsigned int flags)
{
	if (flags & PRFL_FLOATCOORD)
		MSG_WriteFloat (sb, f);
	else if (flags & PRFL_INT32COORD)
		MSG_WriteLong (sb, Q_rint (f * 16));
	else if (flags & PRFL_24BITCOORD)
		MSG_WriteCoord24 (sb, f);
	else
		MSG_WriteCoord16 (sb, f);
}

void MSG_WriteAngle (sizebuf_t *sb, float f, unsigned int flags)
{
	if (flags & PRFL_FLOATANGLE)
		MSG_WriteFloat (sb, f);
	else if (flags & PRFL_SHORTANGLE)
		MSG_WriteShort (sb, Q_rint (f * 65536.0 / 360.0) & 65535);
	else
		MSG_WriteByte (sb, Q_rint (f * 256.0 / 360.0) & 255); // johnfitz -- use Q_rint instead of (int)	}
}

// johnfitz -- for PROTOCOL_FITZQUAKE
void MSG_WriteAngle16 (sizebuf_t *sb, float f, unsigned int flags)
{
	if (flags & PRFL_FLOATANGLE)
		MSG_WriteFloat (sb, f);
	else
		MSG_WriteShort (sb, Q_rint (f * 65536.0 / 360.0) & 65535);
}
// johnfitz

// spike -- for PEXT2_REPLACEMENTDELTAS
void MSG_WriteEntity (sizebuf_t *sb, unsigned int entnum, unsigned int pext2)
{
	// high short, low byte
	if (entnum > 0x7fff && (pext2 & PEXT2_REPLACEMENTDELTAS))
	{
		MSG_WriteShort (sb, 0x8000 | (entnum >> 8));
		MSG_WriteByte (sb, entnum & 0xff);
	}
	else
		MSG_WriteShort (sb, entnum);
}

//
// reading functions
//
int		 msg_readcount;
qboolean msg_badread;

void MSG_BeginReading (void)
{
	msg_readcount = 0;
	msg_badread = false;
}

// returns -1 and sets msg_badread if no more characters are available
int MSG_ReadChar (void)
{
	int c;

	if (msg_readcount + 1 > net_message.cursize)
	{
		msg_badread = true;
		harness_badread_count++;
		return -1;
	}

	c = (signed char)net_message.data[msg_readcount];
	msg_readcount++;

	return c;
}

int MSG_ReadByte (void)
{
	int c;

	if (msg_readcount + 1 > net_message.cursize)
	{
		msg_badread = true;
		harness_badread_count++;
		return -1;
	}

	c = (unsigned char)net_message.data[msg_readcount];
	msg_readcount++;

	return c;
}

int MSG_ReadShort (void)
{
	int c;

	if (msg_readcount + 2 > net_message.cursize)
	{
		msg_badread = true;
		harness_badread_count++;
		return -1;
	}

	c = (short)(net_message.data[msg_readcount] + (net_message.data[msg_readcount + 1] << 8));

	msg_readcount += 2;

	return c;
}

int MSG_ReadLong (void)
{
	uint32_t c;

	if (msg_readcount + 4 > net_message.cursize)
	{
		msg_badread = true;
		harness_badread_count++;
		return -1;
	}

	c = (uint32_t)net_message.data[msg_readcount] + ((uint32_t)(net_message.data[msg_readcount + 1]) << 8) +
		((uint32_t)(net_message.data[msg_readcount + 2]) << 16) + ((uint32_t)(net_message.data[msg_readcount + 3]) << 24);

	msg_readcount += 4;

	return c;
}

unsigned long long MSG_ReadUInt64 (void)
{ // 0* 10*,*, 110*,*,* etc, up to 0xff followed by 8 continuation bytes
	byte			   l = 0x80, v, b = 0;
	unsigned long long r;
	v = MSG_ReadByte ();
	for (; v & l; l >>= 1)
	{
		v -= l;
		b++;
	}
	r = v << (b * 8);
	while (b-- > 0)
		r |= MSG_ReadByte () << (b * 8);
	return r;
}
long long MSG_ReadInt64 (void)
{ // we do some fancy bit recoding for more efficient length coding.
	unsigned long long c = MSG_ReadUInt64 ();
	if (c & 1)
		return -1 - (long long)(c >> 1);
	else
		return (long long)(c >> 1);
}

float MSG_ReadFloat (void)
{
	union
	{
		byte  b[4];
		float f;
		int	  l;
	} dat;

	dat.b[0] = net_message.data[msg_readcount];
	dat.b[1] = net_message.data[msg_readcount + 1];
	dat.b[2] = net_message.data[msg_readcount + 2];
	dat.b[3] = net_message.data[msg_readcount + 3];
	msg_readcount += 4;

	dat.l = LittleLong (dat.l);

	return dat.f;
}
float MSG_ReadDouble (void)
{
	union
	{
		double	 f;
		uint64_t l;
	} dat;

	dat.l = ((uint64_t)net_message.data[msg_readcount] << 0) | ((uint64_t)net_message.data[msg_readcount + 1] << 8) |
			((uint64_t)net_message.data[msg_readcount + 2] << 16) | ((uint64_t)net_message.data[msg_readcount + 3] << 24) |
			((uint64_t)net_message.data[msg_readcount + 4] << 32) | ((uint64_t)net_message.data[msg_readcount + 5] << 40) |
			((uint64_t)net_message.data[msg_readcount + 6] << 48) | ((uint64_t)net_message.data[msg_readcount + 7] << 56);
	msg_readcount += 8;

	return dat.f;
}

const char *MSG_ReadString (void)
{
	static char string[2048];
	int			c;
	size_t		l;

	l = 0;
	do
	{
		c = MSG_ReadByte ();
		if (c == -1 || c == 0)
			break;
		string[l] = c;
		l++;
	} while (l < sizeof (string) - 1);

	string[l] = 0;

	return string;
}

// johnfitz -- original behavior, 13.3 fixed point coords, max range +-4096
float MSG_ReadCoord16 (void)
{
	return MSG_ReadShort () * (1.0 / 8);
}

// johnfitz -- 16.8 fixed point coords, max range +-32768
float MSG_ReadCoord24 (void)
{
	return MSG_ReadShort () + MSG_ReadByte () * (1.0 / 255);
}

// johnfitz -- 32-bit float coords
float MSG_ReadCoord32f (void)
{
	return MSG_ReadFloat ();
}

float MSG_ReadCoord (unsigned int flags)
{
	if (flags & PRFL_FLOATCOORD)
		return MSG_ReadFloat ();
	else if (flags & PRFL_INT32COORD)
		return MSG_ReadLong () * (1.0 / 16.0);
	else if (flags & PRFL_24BITCOORD)
		return MSG_ReadCoord24 ();
	else
		return MSG_ReadCoord16 ();
}

float MSG_ReadAngle (unsigned int flags)
{
	if (flags & PRFL_FLOATANGLE)
		return MSG_ReadFloat ();
	else if (flags & PRFL_SHORTANGLE)
		return MSG_ReadShort () * (360.0 / 65536);
	else
		return MSG_ReadChar () * (360.0 / 256);
}

// johnfitz -- for PROTOCOL_FITZQUAKE
float MSG_ReadAngle16 (unsigned int flags)
{
	if (flags & PRFL_FLOATANGLE)
		return MSG_ReadFloat (); // make sure
	else
		return MSG_ReadShort () * (360.0 / 65536);
}
// johnfitz

unsigned int MSG_ReadEntity (unsigned int pext2)
{
	unsigned int e = (unsigned short)MSG_ReadShort ();
	if (pext2 & PEXT2_REPLACEMENTDELTAS)
	{
		if (e & 0x8000)
		{
			e = (e & 0x7fff) << 8;
			e |= MSG_ReadByte ();
		}
	}
	return e;
}

//===========================================================================

void SZ_Alloc (sizebuf_t *buf, int startsize)
{
	if (startsize < 256)
		startsize = 256;
	buf->data = (byte *)Mem_Alloc (startsize);
	buf->maxsize = startsize;
	buf->cursize = 0;
}

void SZ_Free (sizebuf_t *buf)
{
	Mem_Free (buf->data);
	buf->data = NULL;
	buf->maxsize = 0;
	buf->cursize = 0;
}

void SZ_Clear (sizebuf_t *buf)
{
	buf->cursize = 0;
	buf->overflowed = false;
}

void *SZ_GetSpace (sizebuf_t *buf, int length)
{
	void *data;

	if (buf->cursize + length > buf->maxsize)
	{
		if (!buf->allowoverflow)
			Host_Error ("SZ_GetSpace: overflow without allowoverflow set"); // ericw -- made Host_Error to be less annoying

		if (length > buf->maxsize)
			Sys_Error ("SZ_GetSpace: %i is > full buffer size", length);

		Con_Printf ("SZ_GetSpace: overflow\n");
		SZ_Clear (buf);
		buf->overflowed = true;
	}

	data = buf->data + buf->cursize;
	buf->cursize += length;

	return data;
}

void SZ_Write (sizebuf_t *buf, const void *data, int length)
{
	memcpy (SZ_GetSpace (buf, length), data, length);
}

void SZ_Print (sizebuf_t *buf, const char *data)
{
	int len = strlen (data) + 1;

	if (buf->data[buf->cursize - 1])
	{ /* no trailing 0 */
		memcpy ((byte *)SZ_GetSpace (buf, len), data, len);
	}
	else
	{ /* write over trailing 0 */
		memcpy ((byte *)SZ_GetSpace (buf, len - 1) - 1, data, len);
	}
}
