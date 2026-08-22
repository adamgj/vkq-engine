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
// image_stb.c -- PNG/TGA/JPG decode via stb_image (stays C until Phase 3 M8)

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
