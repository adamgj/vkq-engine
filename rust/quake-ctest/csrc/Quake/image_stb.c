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
// image_stb.c -- PNG/TGA/JPG decode via stb_image (Rust migration Phase 3 M8 seam).
// Under -Duse_rust_image (USE_RUST_IMAGE) the Rust shim provides Image_DecodeSTB
// and this file keeps only Image_DecodeSTBMem, the in-memory stb decoder the
// Rust side uses as the fallback/oracle for formats not (yet) decoded by crates.

#include "quakedef.h"

#ifdef _MSC_VER
// Disable warning C4505: Unused functions
#pragma warning(push)
#pragma warning(disable : 4505)
#endif

#ifdef __GNUC__
// Suppress unused function warnings on GCC/clang
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wunused-function"
#endif

// STB_IMAGE config:
#define STB_IMAGE_IMPLEMENTATION
#define STB_IMAGE_STATIC
#define STBI_NO_BMP
#define STBI_NO_PSD
#define STBI_NO_GIF
#define STBI_NO_HDR
#define STBI_NO_PIC
#define STBI_NO_PNM
#define STBI_NO_LINEAR
#define STBI_NO_STDIO
// plug our Mem_Alloc in stb_image:
#define STBI_MALLOC(sz)		   Mem_Alloc (sz)
#define STBI_REALLOC(p, newsz) Mem_Realloc (p, newsz)
#define STBI_FREE(p)		   Mem_Free (p)
#include "stb_image.h"

#ifdef __GNUC__
// Restore unused function warnings on GCC/clang
#pragma GCC diagnostic pop
#endif

#ifdef _MSC_VER
#pragma warning(pop)
#endif

/*
============
Image_DecodeSTBMem -- stb decode over an in-memory image; always compiled.

The Rust Image_DecodeSTB bulk-reads the resource and routes formats it does
not decode itself through this helper; on failure the stb reason is returned
through *failure_reason (a static/thread-local string, valid until the next
stb call on this thread).

Decodes through stbi_load_from_callbacks over a memory cursor mirroring the
Sys_File* handle semantics (sys_sdl.c) byte for byte -- NOT
stbi_load_from_memory, whose context copies nothing on a short read where
the callback context keeps the partial bytes (observable on truncated raw
TGA tails). The cursor treats the resource slice as the whole file; for a
resource inside a pak the streaming decoder could read past the resource
into neighboring pak bytes, which the slice cannot reproduce -- same
out-of-resource class the PCX shim documents (Phase 3 M2 amendment log).
============
*/
typedef struct stbmemfile_s
{
	const byte *mem;
	int			len;
	qfileofs_t	pos;
	bool		eof_condition;
} stbmemfile_t;

// Sys_FileRead (sys_sdl.c) over the resource slice
static int stbmem_read_cb (void *user, char *data, int size)
{
	stbmemfile_t *f = (stbmemfile_t *)user;

	if (size <= 0)
		return 0;

	f->eof_condition = f->eof_condition || (((qfileofs_t)f->len - f->pos) <= 0) ? true : false;

	if (f->eof_condition)
		return 0;

	qfilesize_t computed_read_count = q_min ((qfilesize_t)size, (qfilesize_t)f->len - f->pos);
	computed_read_count = q_max (0, computed_read_count);

	memcpy (data, f->mem + f->pos, computed_read_count);
	f->pos += computed_read_count;
	f->eof_condition = (computed_read_count < size);
	return (int)computed_read_count;
}

// Sys_FileSeek: going beyond the end is fine; eof_condition is NOT cleared
static void stbmem_seek (stbmemfile_t *f, qfileofs_t position)
{
	if (position >= 0)
		f->pos = position;
}

// Sys_fgetc over the cursor
static int stbmem_fgetc (stbmemfile_t *f)
{
	if (f->eof_condition)
		return EOF;

	char next_byte_read = 0;

	if (stbmem_read_cb (f, &next_byte_read, 1) != 1)
		return EOF;

	return (unsigned char)next_byte_read;
}

// mirrors stbi_skip_cb below: seek, then the fgetc probe + re-seek
static void stbmem_skip_cb (void *user, int n)
{
	stbmemfile_t *f = (stbmemfile_t *)user;

	qfileofs_t current_pos = f->pos;

	qfileofs_t new_pos = current_pos + n;

	stbmem_seek (f, new_pos);

	if (stbmem_fgetc (f) != EOF)
	{
		stbmem_seek (f, new_pos);
	}
}

static int stbmem_eof_cb (void *user)
{
	return (int)((stbmemfile_t *)user)->eof_condition;
}

byte *Image_DecodeSTBMem (const byte *mem, int len, int *width, int *height, const char **failure_reason)
{
	stbmemfile_t	  memfile = {.mem = mem, .len = len, .pos = 0, .eof_condition = false};
	stbi_io_callbacks mem_cb = {.read = stbmem_read_cb, .eof = stbmem_eof_cb, .skip = stbmem_skip_cb};

	byte *data = stbi_load_from_callbacks (&mem_cb, &memfile, width, height, NULL, 4);

	if (!data && failure_reason)
		*failure_reason = stbi_failure_reason ();

	return data;
}

#ifndef USE_RUST_IMAGE

static int stbi_read_cb (void *user, char *data, int size)
{
	int *file_handle = (int *)user;

	return Sys_FileRead (*file_handle, (void *)data, size);
}

static void stbi_skip_cb (void *user, int n)
{
	int *file_handle = (int *)user;

	qfileofs_t current_pos = Sys_FilePos (*file_handle);

	qfileofs_t new_pos = current_pos + n;

	// mimic default stbi__stdio_skip() :
	Sys_FileSeek (*file_handle, new_pos);

	if (Sys_fgetc (*file_handle) != EOF)
	{
		Sys_FileSeek (*file_handle, new_pos);
	}
}

static int stbi_eof_cb (void *user)
{
	int *file_handle = (int *)user;

	return (int)Sys_feof (*file_handle);
}

/*
============
Image_DecodeSTB -- PNG/TGA/JPG via stb_image
============
*/
byte *Image_DecodeSTB (int file_handle, int *width, int *height, const char *image_name)
{
	stbi_io_callbacks sys_file_cb = {.read = stbi_read_cb, .eof = stbi_eof_cb, .skip = stbi_skip_cb};

	// data is managed by our Mem_Alloc routines, nothing more to do.
	byte *data = stbi_load_from_callbacks (&sys_file_cb, (void *)&file_handle, width, height, NULL, 4);

	if (!data)
		Con_Warning ("couldn't load %s (%s)\n", image_name, stbi_failure_reason ());

	COM_CloseFile (file_handle);
	return data;
}

#endif /* !USE_RUST_IMAGE */
