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
// net_msg_glue.c -- Host_Error/Sys_Error trampolines for the Rust MSG/SZ
// wire layer (Rust migration Phase 5 M3). Compiled only with -Duse_rust_net,
// replacing net_msg.c.
//
// ADR-009: a Host_Error longjmp must never unwind a Rust frame, so the Rust
// writer exports return a status and these pure-C frames re-raise it with
// the exact message the C original used. The reader-side entry points and
// SZ_Clear carry no error paths and are exported from Rust under their C
// names directly. The reader globals stay C-owned here (ADR-007 net row);
// net_message itself stays in net_main.c.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

int		 msg_readcount;
qboolean msg_badread;

/* status codes shared with rust/quake-capi/src/net.rs (keep in sync) */
#define SZ_OK			   0
#define SZ_ERR_OVERFLOW	   1
#define SZ_ERR_OVERSIZE	   2
#define SZ_ERR_RANGE_CHAR  3
#define SZ_ERR_RANGE_BYTE  4
#define SZ_ERR_RANGE_SHORT 5

static void NetMsg_Raise (int status, int value, int length)
{
	/* ADR-009/aliasing: the Rust exports must not call Con_Printf (it is not
	   a leaf) while they hold borrows of the sizebuf, so the allowed-overflow
	   diagnostics are accumulated Rust-side and printed here, in a C frame,
	   before any error is raised -- same output order as the C original. */
	unsigned int overflows = quake_rs_sz_take_overflow_events ();
	while (overflows--)
		Con_Printf ("SZ_GetSpace: overflow\n");

	switch (status)
	{
	case SZ_OK:
		return;
	case SZ_ERR_OVERFLOW:
		Host_Error ("SZ_GetSpace: overflow without allowoverflow set"); // ericw -- made Host_Error to be less annoying
		break;
	case SZ_ERR_OVERSIZE:
		Sys_Error ("SZ_GetSpace: %i is > full buffer size", length);
		break;
	case SZ_ERR_RANGE_CHAR:
		Host_Error ("MSG_WriteChar: range error = %i not in -128..127", value);
		break;
	case SZ_ERR_RANGE_BYTE:
		Host_Error ("MSG_WriteByte: range error = %i not in 0..255", value);
		break;
	case SZ_ERR_RANGE_SHORT:
		Host_Error ("MSG_WriteShort: range error = %i not in -32768..65535", value);
		break;
	default:
		Sys_Error ("NetMsg_Raise: unknown status %i", status);
	}
}

void MSG_WriteChar (sizebuf_t *sb, int c)
{
	NetMsg_Raise (quake_rs_msg_write_char (sb, c), c, 1);
}

void MSG_WriteByte (sizebuf_t *sb, int c)
{
	NetMsg_Raise (quake_rs_msg_write_byte (sb, c), c, 1);
}

void MSG_WriteShort (sizebuf_t *sb, int c)
{
	NetMsg_Raise (quake_rs_msg_write_short (sb, c), c, 2);
}

void MSG_WriteLong (sizebuf_t *sb, int c)
{
	NetMsg_Raise (quake_rs_msg_write_long (sb, c), c, 4);
}

void MSG_WriteUInt64 (sizebuf_t *sb, unsigned long long c)
{
	NetMsg_Raise (quake_rs_msg_write_uint64 (sb, c), 0, 9);
}

void MSG_WriteInt64 (sizebuf_t *sb, long long c)
{
	NetMsg_Raise (quake_rs_msg_write_int64 (sb, c), 0, 10);
}

void MSG_WriteFloat (sizebuf_t *sb, float f)
{
	NetMsg_Raise (quake_rs_msg_write_float (sb, f), 0, 4);
}

void MSG_WriteDouble (sizebuf_t *sb, double f)
{
	NetMsg_Raise (quake_rs_msg_write_double (sb, f), 0, 8);
}

void MSG_WriteString (sizebuf_t *sb, const char *s)
{
	NetMsg_Raise (quake_rs_msg_write_string (sb, s), 0, s ? (int)strlen (s) + 1 : 1);
}

void MSG_WriteStringUnterminated (sizebuf_t *sb, const char *s)
{
	NetMsg_Raise (quake_rs_msg_write_string_unterminated (sb, s), 0, (int)strlen (s));
}

void MSG_WriteCoord (sizebuf_t *sb, float f, unsigned int flags)
{
	NetMsg_Raise (quake_rs_msg_write_coord (sb, f, flags), 0, 4);
}

void MSG_WriteAngle (sizebuf_t *sb, float f, unsigned int flags)
{
	NetMsg_Raise (quake_rs_msg_write_angle (sb, f, flags), 0, 4);
}

void MSG_WriteAngle16 (sizebuf_t *sb, float f, unsigned int flags)
{
	NetMsg_Raise (quake_rs_msg_write_angle16 (sb, f, flags), 0, 4);
}

void MSG_WriteEntity (sizebuf_t *sb, unsigned int entnum, unsigned int pext2)
{
	NetMsg_Raise (quake_rs_msg_write_entity (sb, entnum, pext2), 0, 3);
}

void SZ_Alloc (sizebuf_t *buf, int startsize)
{
	/* allocation policy stays engine-side (Mem_Alloc), verbatim from
	   net_msg.c */
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

void *SZ_GetSpace (sizebuf_t *buf, int length)
{
	int offset = 0;
	NetMsg_Raise (quake_rs_sz_get_space (buf, length, &offset), 0, length);
	return buf->data + offset;
}

void SZ_Write (sizebuf_t *buf, const void *data, int length)
{
	NetMsg_Raise (quake_rs_sz_write (buf, data, length), 0, length);
}

void SZ_Print (sizebuf_t *buf, const char *data)
{
	NetMsg_Raise (quake_rs_sz_print (buf, data), 0, (int)strlen (data) + 1);
}
