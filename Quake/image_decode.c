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
// image_decode.c -- image decoders (Rust migration Phase 3: replaced by
// quake-image under -Duse_rust_image; this file is the differential oracle)

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

//==============================================================================
//
//  PCX
//
//==============================================================================

typedef struct
{
	char		   signature;
	char		   version;
	char		   encoding;
	char		   bits_per_pixel;
	unsigned short xmin, ymin, xmax, ymax;
	unsigned short hdpi, vdpi;
	byte		   colortable[48];
	char		   reserved;
	char		   color_planes;
	unsigned short bytes_per_line;
	unsigned short palette_type;
	char		   filler[58];
} pcxheader_t;

/*
============
Image_DecodePCX
============
*/
byte *Image_DecodePCX (int file_handle, int *width, int *height, const char *image_name)
{
	pcxheader_t pcx;
	int			x, y, w, h, readbyte, runlength;
	byte	   *p, *data;
	byte		palette[768];

	// save start of file since we might be inside a pak file
	const int start = Sys_FilePos (file_handle);

	// We may are in a pak file so that resource size is com_filesize
	const int file_size = com_filesize;

	if (Sys_FileRead (file_handle, &pcx, sizeof (pcx)) != sizeof (pcx))
		Sys_Error ("'%s' is not a valid PCX file", image_name);

	pcx.xmin = (unsigned short)LittleShort (pcx.xmin);
	pcx.ymin = (unsigned short)LittleShort (pcx.ymin);
	pcx.xmax = (unsigned short)LittleShort (pcx.xmax);
	pcx.ymax = (unsigned short)LittleShort (pcx.ymax);
	pcx.bytes_per_line = (unsigned short)LittleShort (pcx.bytes_per_line);

	if (pcx.signature != 0x0A)
		Sys_Error ("'%s' is not a valid PCX file", image_name);

	if (pcx.version != 5)
		Sys_Error ("'%s' is version %i, should be 5", image_name, pcx.version);

	if (pcx.encoding != 1 || pcx.bits_per_pixel != 8 || pcx.color_planes != 1)
		Sys_Error ("'%s' has wrong encoding or bit depth", image_name);

	w = pcx.xmax - pcx.xmin + 1;
	h = pcx.ymax - pcx.ymin + 1;

	data = (byte *)Mem_Alloc ((w * h + 1) * 4); //+1 to allow reading padding byte on last line

	// load palette
	Sys_FileSeek (file_handle, start + file_size - 768);

	if (Sys_FileRead (file_handle, palette, 768) != 768)
		Sys_Error ("'%s' is not a valid PCX file", image_name);

	// back to start of image data
	Sys_FileSeek (file_handle, start + sizeof (pcx));

	for (y = 0; y < h; y++)
	{
		p = data + y * w * 4;

		for (x = 0; x < (pcx.bytes_per_line);) // read the extra padding byte if necessary
		{
			readbyte = Sys_fgetc (file_handle);

			if (readbyte >= 0xC0)
			{
				runlength = readbyte & 0x3F;
				readbyte = Sys_fgetc (file_handle);
			}
			else
				runlength = 1;

			while (runlength--)
			{
				p[0] = palette[readbyte * 3];
				p[1] = palette[readbyte * 3 + 1];
				p[2] = palette[readbyte * 3 + 2];
				p[3] = 255;
				p += 4;
				x++;
			}
		}
	}

	COM_CloseFile (file_handle);

	*width = w;
	*height = h;
	return data;
}

//==============================================================================
//
//  QPIC (aka '.lmp')
//
//==============================================================================

typedef struct
{
	unsigned int width, height;
} lmpheader_t;

/*
============
Image_DecodeLMP
============
*/
byte *Image_DecodeLMP (int file_handle, int *width, int *height, const char *image_name)
{
	lmpheader_t qpic;
	size_t		pix;
	void	   *data;

	// We may are in a pak file so that resource size is com_filesize
	const int file_size = com_filesize;

	if (Sys_FileRead (file_handle, &qpic, sizeof (qpic)) != sizeof (qpic))
		Sys_Error ("'%s' is not a valid LMP file", image_name);

	qpic.width = LittleLong (qpic.width);
	qpic.height = LittleLong (qpic.height);

	pix = qpic.width * qpic.height;

	if (file_size != 8 + pix)
	{
		COM_CloseFile (file_handle);
		return NULL;
	}

	data = (byte *)Mem_Alloc (pix); //+1 to allow reading padding byte on last line

	if (Sys_FileRead (file_handle, data, pix) != pix)
		Sys_Error ("'%s' is not a valid LMP file", image_name);

	COM_CloseFile (file_handle);

	*width = qpic.width;
	*height = qpic.height;
	return data;
}
