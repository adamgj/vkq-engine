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
// image.c -- image loading

#include "quakedef.h"

// STB_IMAGE_WRITE config:
#define STB_IMAGE_WRITE_IMPLEMENTATION
#define STB_IMAGE_WRITE_STATIC
#define STBI_WRITE_NO_STDIO		// all file output goes through Sys_fopen
// plug our Mem_Alloc in stb_image_write:
#define STBIW_MALLOC(sz)		Mem_Alloc (sz)
#define STBIW_REALLOC(p, newsz) Mem_Realloc (p, newsz)
#define STBIW_FREE(p)			Mem_Free (p)
#include "stb_image_write.h"

// LODEPNG config:
#define LODEPNG_NO_COMPILE_ALLOCATORS
#define LODEPNG_NO_COMPILE_DECODER
#define LODEPNG_NO_COMPILE_CPP
#define LODEPNG_NO_COMPILE_ANCILLARY_CHUNKS
#define LODEPNG_NO_COMPILE_ERROR_TEXT
#include "lodepng.h"
#include "lodepng.c"

void *lodepng_malloc (size_t size)
{
	return Mem_Alloc (size);
}

void *lodepng_realloc (void *ptr, size_t new_size)
{
	return Mem_Realloc (ptr, new_size);
}

void lodepng_free (void *ptr)
{
	Mem_Free (ptr);
}

/*
============
Image_LoadImage
of an image filename 'name' (with no extension)
either returns a pointer to Mem_Alloc allocated RGBA data
or returns NULL if not loaded, either because not found OR if name
is ignored because from a gamedir with lower priority than min_path_id.
Use min_path_id = 0 if gamedir priority is N/A.
Search order:  png tga jpg pcx lmp
Note : makes a thread-safe copy of 'name' so we can use va() as inuput.
============
*/
// image formats supported, ordered by priority
typedef enum
{
	STB_IMAGE_LOADER,
	PCX_LOADER,
	LMP_LOADER
} image_loader_t;

static struct
{
	const char	  *file_extension;
	image_loader_t loader;
} supported_image_formats[] = {{"png", STB_IMAGE_LOADER}, {"tga", STB_IMAGE_LOADER}, {"jpg", STB_IMAGE_LOADER}, {"pcx", PCX_LOADER}, {"lmp", LMP_LOADER}};

const int num_supported_image_formats = countof (supported_image_formats);

byte *Image_LoadImage (const char *name, int *width, int *height, enum srcformat *fmt, unsigned int min_path_id)
{
	// 1. Search 'name' image by supported_image_formats, keeping only the best, as:
	// a) The highest path_id wins,
	// b) For equivalent path_id, Highest supported_image_formats[] priority wins. (smallest index)
	int			 best_path_id = -1;
	unsigned int path_id = 0;
	int			 best_image_kind_index = -1;
	char		 loadfilename[MAX_OSPATH];
	int			 file_handle = -1;

	for (int image_kind_index = 0; image_kind_index < num_supported_image_formats; image_kind_index++)
	{
		q_snprintf (loadfilename, sizeof (loadfilename), "%s.%s", name, supported_image_formats[image_kind_index].file_extension);

		if (COM_FileExists (loadfilename, &path_id))
		{
			if ((int)path_id > best_path_id)
			{
				best_path_id = (int)path_id;
				best_image_kind_index = image_kind_index;
			}
		}
	}

	// at that point, best_image_kind_index points on the highest path_id image,
	// or in case of path_id equality, the best format in terms of priority.
	// min_path_id is the final barrier of entry:
	// if no file was found, this is also used to bail out. (best_path_id  = -1 < min_path_id ( = 0))
	if (best_path_id < (int)min_path_id)
		return NULL;

	// 2. Load image matching supported_image_formats[best_image_kind_index].file_extension
	q_snprintf (loadfilename, sizeof (loadfilename), "%s.%s", name, supported_image_formats[best_image_kind_index].file_extension);

	COM_OpenFile (loadfilename, &file_handle, NULL);

	assert (file_handle >= 0);

	if (supported_image_formats[best_image_kind_index].loader == STB_IMAGE_LOADER)
	{
		*fmt = SRC_RGBA;
		return Image_DecodeSTB (file_handle, width, height, loadfilename);
	}
	else if (supported_image_formats[best_image_kind_index].loader == PCX_LOADER)
	{
		*fmt = SRC_RGBA;
		return Image_DecodePCX (file_handle, width, height, loadfilename);
	}
	else if (supported_image_formats[best_image_kind_index].loader == LMP_LOADER)
	{
		*fmt = SRC_INDEXED;
		return Image_DecodeLMP (file_handle, width, height, loadfilename);
	}

	return NULL;
}

//==============================================================================
//
//  TGA
//
//==============================================================================

#define TARGAHEADERSIZE 18 /* size on disk */

/*
============
Image_WriteTGA -- writes RGB or RGBA data to a TGA file

returns true if successful
============
*/
qboolean Image_WriteTGA (const char *name, byte *data, int width, int height, int bpp, qboolean upsidedown)
{
	int	 handle, i, size, temp, bytes;
	char pathname[MAX_OSPATH];
	byte header[TARGAHEADERSIZE];

	q_snprintf (pathname, sizeof (pathname), "%s/%s", com_gamedir, name);
	handle = Sys_FileOpenWrite (pathname);
	if (handle == -1)
		return false;

	memset (header, 0, TARGAHEADERSIZE);
	header[2] = 2; // uncompressed type
	header[12] = width & 255;
	header[13] = width >> 8;
	header[14] = height & 255;
	header[15] = height >> 8;
	header[16] = bpp; // pixel size
	if (upsidedown)
		header[17] = 0x20; // upside-down attribute

	// swap red and blue bytes
	bytes = bpp / 8;
	size = width * height * bytes;
	for (i = 0; i < size; i += bytes)
	{
		temp = data[i];
		data[i] = data[i + 2];
		data[i + 2] = temp;
	}

	Sys_FileWrite (handle, header, TARGAHEADERSIZE);
	Sys_FileWrite (handle, data, size);
	Sys_FileClose (handle);

	return true;
}

//==============================================================================
//
//  STB_IMAGE_WRITE
//
//==============================================================================

static byte *CopyFlipped (const byte *data, int width, int height, int bpp)
{
	int	  y, rowsize;
	byte *flipped;

	rowsize = width * (bpp / 8);
	flipped = (byte *)Mem_Alloc (height * rowsize);
	if (!flipped)
		return NULL;

	for (y = 0; y < height; y++)
	{
		memcpy (&flipped[y * rowsize], &data[(height - 1 - y) * rowsize], rowsize);
	}
	return flipped;
}

// stbi_write_func that writes to a FILE opened with Sys_fopen, so non-ASCII
// paths work on Windows (stb's own file output goes through ANSI fopen)
static void Image_WriteToFileFunc (void *context, void *data, int size)
{
	fwrite (data, 1, size, (FILE *)context);
}

/*
============
Image_WriteJPG -- writes using stb_image_write

returns true if successful
============
*/
qboolean Image_WriteJPG (const char *name, byte *data, int width, int height, int bpp, int quality, qboolean upsidedown)
{
	unsigned error;
	char	 pathname[MAX_OSPATH];
	byte	*flipped;
	int		 bytes_per_pixel;

	if (!(bpp == 32 || bpp == 24))
		Sys_Error ("bpp not 24 or 32");

	bytes_per_pixel = bpp / 8;

	q_snprintf (pathname, sizeof (pathname), "%s/%s", com_gamedir, name);

	if (!upsidedown)
	{
		flipped = CopyFlipped (data, width, height, bpp);
		if (!flipped)
			return false;
	}
	else
		flipped = data;

	FILE *f = Sys_fopen (pathname, "wb");
	if (f)
	{
		error = stbi_write_jpg_to_func (Image_WriteToFileFunc, f, width, height, bytes_per_pixel, flipped, quality);
		fclose (f);
	}
	else
		error = 0;
	if (!upsidedown)
		Mem_Free (flipped);

	return (error != 0);
}

qboolean Image_WritePNG (const char *name, byte *data, int width, int height, int bpp, qboolean upsidedown)
{
	unsigned	   error;
	char		   pathname[MAX_OSPATH];
	byte		  *flipped;
	unsigned char *filters;
	unsigned char *png;
	size_t		   pngsize;
	LodePNGState   state;

	if (!(bpp == 32 || bpp == 24))
		Sys_Error ("bpp not 24 or 32");

	q_snprintf (pathname, sizeof (pathname), "%s/%s", com_gamedir, name);

	flipped = (!upsidedown) ? CopyFlipped (data, width, height, bpp) : data;
	filters = (unsigned char *)Mem_Alloc (height);
	if (!filters || !flipped)
	{
		if (!upsidedown)
			Mem_Free (flipped);
		Mem_Free (filters);
		return false;
	}

	// set some options for faster compression
	lodepng_state_init (&state);
	state.encoder.zlibsettings.use_lz77 = 0;
	state.encoder.auto_convert = 0;
	state.encoder.filter_strategy = LFS_PREDEFINED;
	memset (filters, 1, height); // use filter 1; see https://www.w3.org/TR/PNG-Filters.html
	state.encoder.predefined_filters = filters;

	if (bpp == 24)
	{
		state.info_raw.colortype = LCT_RGB;
		state.info_png.color.colortype = LCT_RGB;
	}
	else
	{
		state.info_raw.colortype = LCT_RGBA;
		state.info_png.color.colortype = LCT_RGBA;
	}

	error = lodepng_encode (&png, &pngsize, flipped, width, height, &state);
	if (error == 0)
	{
		FILE *f = Sys_fopen (pathname, "wb");
		if (f)
		{
			fwrite (png, 1, pngsize, f);
			fclose (f);
		}
		else
			error = 1;
	}
#ifdef LODEPNG_COMPILE_ERROR_TEXT
	else
		Con_Printf ("WritePNG: %s\n", lodepng_error_text (error));
#endif

	lodepng_state_cleanup (&state);
	lodepng_free (png); /* png was allocated by lodepng */
	Mem_Free (filters);
	if (!upsidedown)
		Mem_Free (flipped);

	return (error == 0);
}
