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

// common_fs.c -- Quake filesystem: search paths, pak files, gamedir
// handling, localization / .kpf reading. Split out of common.c for the
// Rust migration (ROADMAP Phase 2): this file is excluded from the build
// when -Duse_rust_fs is enabled and quake-fs provides these symbols.

#include "quakedef.h"
#include "sys.h"

#include "q_ctype.h"
#include "filenames.h"
#include "steam.h"
#include <errno.h>

// Plug our allocators into miniz:
#define MZ_MALLOC(x)	 Mem_Alloc (x)
#define MZ_FREE(x)		 Mem_Free (x)
#define MZ_REALLOC(p, x) Mem_Realloc (p, x)

// include miniz stb-syle, directly in this compilation unit.
// (supported by miniz)
#include "miniz.c"

qboolean com_modified; // set true if using non-id files

static void COM_Path_f (void);

// if a packfile directory differs from this, it is assumed to be hacked
#define PAK0_COUNT		339	  /* id1/pak0.pak - v1.0x */
#define PAK0_CRC_V100	13900 /* id1/pak0.pak - v1.00 */
#define PAK0_CRC_V101	62751 /* id1/pak0.pak - v1.01 */
#define PAK0_CRC_V106	32981 /* id1/pak0.pak - v1.06 */
#define PAK0_CRC		(PAK0_CRC_V106)
#define PAK0_COUNT_V091 308	  /* id1/pak0.pak - v0.91/0.92, not supported */
#define PAK0_CRC_V091	28804 /* id1/pak0.pak - v0.91/0.92, not supported */

qboolean standard_quake = true, rogue, hipnotic;

extern const unsigned char vkquake_pak[];
extern const int		   vkquake_pak_size;
extern const int		   vkquake_pak_decompressed_size;

/*
=============================================================================

QUAKE FILESYSTEM

=============================================================================
*/

#include "pakfile.h"

char com_gamenames[1024]; // eg: "hipnotic;quoth;warp" ... no id1
char com_gamedir[MAX_OSPATH];
char com_basedir[MAX_OSPATH];
char com_basedirs[MAX_BASEDIRS][MAX_OSPATH]; // all content roots in mount order: extras (e.g. the Nightdive
											 // add-on dir), the main basedir, the userdir (write target) last
int	 com_numbasedirs;

/*
=================
COM_AddBaseDir

Registers a content root; game directories are looked up in all roots,
with later-added roots taking precedence over earlier ones
=================
*/
void COM_AddBaseDir (const char *dir)
{
	int i;
	for (i = 0; i < com_numbasedirs; i++)
		if (!q_strcasecmp (com_basedirs[i], dir))
			return;
	if (com_numbasedirs == MAX_BASEDIRS)
		Sys_Error ("COM_AddBaseDir: too many base directories");
	q_strlcpy (com_basedirs[com_numbasedirs++], dir, sizeof (com_basedirs[0]));
}

searchpath_t *com_searchpaths;
searchpath_t *com_base_searchpaths;

/*
============
COM_Path_f
============
*/
static void COM_Path_f (void)
{
	searchpath_t *s;

	Con_Printf ("Current search path:\n");
	for (s = com_searchpaths; s; s = s->next)
	{
		if (s->pack)
		{
			Con_Printf ("%s (%i files)\n", s->pack->filename, s->pack->numfiles);
		}
		else
			Con_Printf ("%s\n", s->filename);
	}
}

/*
============
COM_WriteFile

The filename will be prefixed by the current game directory
============
*/
void COM_WriteFile (const char *filename, const void *data, int len)
{
	int	 handle;
	char name[MAX_OSPATH];

	q_snprintf (name, sizeof (name), "%s/%s", com_gamedir, filename);

	handle = Sys_FileOpenWrite (name);
	if (handle == -1)
	{
		Sys_Printf ("COM_WriteFile: failed on %s\n", name);
		return;
	}

	Sys_Printf ("COM_WriteFile: %s\n", name);
	Sys_FileWrite (handle, data, len);
	Sys_FileClose (handle);
}

/*
================
COM_filelength
================
*/
static qfilesize_t COM_filelength (FILE *f)
{
	return Sys_filelength (f);
}

/*
===========
COM_FindFile

Finds the file in the search path.
Sets com_filesize and one of handle or file
If neither of file or handle is set, this
can be used for detecting a file's presence.
===========
*/
static qfilesize_t COM_FindFile (const char *filename, int *handle, FILE **file, unsigned int *path_id)
{
	searchpath_t *search;
	char		  netpath[MAX_OSPATH];
	pack_t		 *pak;
	int			  i;
	qboolean	  is_config = !q_strcasecmp (filename, "config.cfg"), found = false;

	if (file && handle)
		Sys_Error ("COM_FindFile: both handle and file set");

	file_from_pak = 0;

	//
	// search through the path, one element at a time
	//
	for (search = com_searchpaths; search; search = search->next)
	{
		if (search->pack) /* look through all the pak file elements */
		{
			pak = search->pack;
			for (i = 0; i < pak->numfiles; i++)
			{
				if (strcmp (pak->files[i].name, filename) != 0)
					continue;
				// found it!
				com_filesize = pak->files[i].filelen;
				file_from_pak = 1;
				if (path_id)
					*path_id = search->path_id;
				if (handle)
				{
					// We can have concurrent reads to the pack (either as file or memory-based)
					// So we MUST duplicate the pak handle to allow independent reads and seeks.
					int new_handle = Sys_DuplicateHandle (pak->handle);
					if (new_handle < 0)
						Sys_Error ("COM_FindFile: couldn't reopen %s", pak->filename);
					Sys_FileSeek (new_handle, pak->files[i].filepos);
					*handle = new_handle;
					return com_filesize;
				}
				else if (file)
				{ /* open a new file on the pakfile */
					*file = Sys_fopen (pak->filename, "rb");
					if (*file)
						Sys_fseek (*file, pak->files[i].filepos, SEEK_SET);
					return com_filesize;
				}
				else /* for COM_FileExists() */
				{
					return com_filesize;
				}
			}
		}
		else /* check a file in the directory tree */
		{
			if (!registered.value)
			{ /* if not a registered version, don't ever go beyond base */
				if (strchr (filename, '/') || strchr (filename, '\\'))
					continue;
			}

			if (is_config)
			{
				q_snprintf (netpath, sizeof (netpath), "%s/" CONFIG_NAME, search->filename);
				if (Sys_FileType (netpath) & FS_ENT_FILE)
					found = true;
			}

			if (!found)
			{
				q_snprintf (netpath, sizeof (netpath), "%s/%s", search->filename, filename);
				if (!(Sys_FileType (netpath) & FS_ENT_FILE))
					continue;
			}

			if (path_id)
				*path_id = search->path_id;
			if (handle)
			{
				com_filesize = Sys_FileOpenRead (netpath, &i);
				*handle = i;
				return com_filesize;
			}
			else if (file)
			{
				*file = Sys_fopen (netpath, "rb");
				com_filesize = (*file == NULL) ? -1 : COM_filelength (*file);
				return com_filesize;
			}
			else
			{
				return 0; /* dummy valid value for COM_FileExists() */
			}
		}
	}

	if (developer.value > 1)
	{
		Con_DPrintf ("FindFile: can't find %s\n", filename);
	}

	if (handle)
		*handle = -1;
	if (file)
		*file = NULL;
	com_filesize = -1;
	return com_filesize;
}

/*
===========
COM_FileExists

Returns whether the file is found in the quake filesystem.
===========
*/
qboolean COM_FileExists (const char *filename, unsigned int *path_id)
{
	qfilesize_t ret = COM_FindFile (filename, NULL, NULL, path_id);
	return (ret == -1) ? false : true;
}

/*
===========
COM_OpenFile

filename never has a leading slash, but may contain directory walks
returns a handle and a length
it may actually be inside a pak file
===========
*/
qfilesize_t COM_OpenFile (const char *filename, int *handle, unsigned int *path_id)
{
	return COM_FindFile (filename, handle, NULL, path_id);
}

/*
===========
COM_FOpenFile

If the requested file is inside a packfile, a new FILE * will be opened
into the file.
===========
*/
qfilesize_t COM_FOpenFile (const char *filename, FILE **file, unsigned int *path_id)
{
	return COM_FindFile (filename, NULL, file, path_id);
}

/*
============
COM_CloseFile

If it is a pak file handle, don't really close it
============
*/
void COM_CloseFile (int h)
{
	if (h < 0)
		return;

	searchpath_t *s;

	for (s = com_searchpaths; s; s = s->next)
		if (s->pack && s->pack->handle == h)
			return;

	Sys_FileClose (h);
}

/*
============
COM_LoadFile

Filename are reletive to the quake directory.
Allways appends a 0 byte.
============
*/
byte *COM_LoadFile (const char *path, unsigned int *path_id)
{
	int			h;
	byte	   *buf;
	qfilesize_t len;

	buf = NULL; // quiet compiler warning

	// look for it in the filesystem or pack files
	len = COM_OpenFile (path, &h, path_id);
	if (h == -1)
		return NULL;

	buf = (byte *)Mem_AllocNonZero (len + 1);

	if (!buf)
		Sys_Error ("COM_LoadFile: not enough space for %s", path);

	((byte *)buf)[len] = 0;

	Sys_FileRead (h, buf, len);
	COM_CloseFile (h);

	return buf;
}

byte *COM_LoadMallocFile_TextMode_OSPath (const char *path, long *len_out)
{
	FILE	   *f;
	byte	   *data;
	qfilesize_t len, actuallen;

	// ericw -- this is used by Host_Loadgame_f. Translate CRLF to LF on load games,
	// othewise multiline messages have a garbage character at the end of each line.
	// TODO: could handle in a way that allows loading CRLF savegames on mac/linux
	// without the junk characters appearing.
	f = Sys_fopen (path, "rt");
	if (f == NULL)
		return NULL;

	len = COM_filelength (f);
	if (len < 0)
	{
		fclose (f);
		return NULL;
	}

	data = (byte *)Mem_AllocNonZero (len + 1);
	if (data == NULL)
	{
		fclose (f);
		return NULL;
	}

	// (actuallen < len) if CRLF to LF translation was performed
	actuallen = fread (data, 1, len, f);
	if (ferror (f))
	{
		fclose (f);
		Mem_Free (data);
		return NULL;
	}
	data[actuallen] = '\0';

	if (len_out != NULL)
		*len_out = actuallen;
	fclose (f);
	return data;
}

const char *COM_ParseIntNewline (const char *buffer, int *value)
{
	int consumed = 0;
	sscanf (buffer, "%i\n%n", value, &consumed);
	return buffer + consumed;
}

const char *COM_ParseFloatNewline (const char *buffer, float *value)
{
	int consumed = 0;
	sscanf (buffer, "%f\n%n", value, &consumed);
	return buffer + consumed;
}

const char *COM_ParseStringNewline (const char *buffer)
{
	int consumed = 0;
	com_token[0] = '\0';
	sscanf (buffer, "%1023s\n%n", com_token, &consumed);
	return buffer + consumed;
}

size_t COM_SanitizeDescriptionString (char *dst, size_t dstsize, const char *src, bool remove_color)
{
	int srcpos, dstpos;

	if (!dstsize)
		return 0;

	for (srcpos = dstpos = 0; src[srcpos] && (size_t)dstpos + 1 < dstsize; srcpos++)
	{
		char c = src[srcpos] & (remove_color ? 0x7f : 0xFF); // remove_color

		// When reducing to plain ASCII, also strip control chars: colored glyphs can mask down to
		// scanf whitespace (e.g. 0x8b -> \v), which would split the savegame comment line on load
		if (remove_color && !q_isprint (c))
			c = ' ';
		else if (c == '\n' || c == '\r') // replace newlines with spaces
			c = ' ';
		else if (c == '\\' && src[srcpos + 1] == 'n') // replace '\\' followed by 'n' with space
		{
			c = ' ';
			srcpos++;
		}
		// remove leading spaces, replace consecutive spaces with single one
		if (c != ' ' || (dstpos > 0 && dst[dstpos - 1] != c))
			dst[dstpos++] = c;
	}
	// remove trailing space, if any
	if (dstpos > 0 && dst[dstpos - 1] == ' ')
		--dstpos;

	dst[dstpos] = '\0';
	return dstpos;
}

/*
=================
COM_LoadPackFile -- johnfitz -- modified based on topaz's tutorial

Takes an explicit (not game tree related) path to a pak file.

Loads the header and directory, adding the files at the beginning
of the list so they override previous pack files.
=================
*/
static pack_t *COM_LoadPackFile (const char *packfile, int packhandle)
{
	dpackheader_t  header;
	int			   i;
	packfile_t	  *newfiles;
	int			   numpackfiles;
	pack_t		  *pack;
	unsigned short crc;

	// use global as temporary to prevent stack consumption,
	// fine because this is only called from the main loop.
	static dpackfile_t info[MAX_FILES_IN_PACK];

	Sys_FileRead (packhandle, (void *)&header, sizeof (header));
	if (header.id[0] != 'P' || header.id[1] != 'A' || header.id[2] != 'C' || header.id[3] != 'K')
		Sys_Error ("%s is not a packfile", packfile);

	header.dirofs = LittleLong (header.dirofs);
	header.dirlen = LittleLong (header.dirlen);

	numpackfiles = header.dirlen / sizeof (dpackfile_t);

	if (header.dirlen < 0 || header.dirofs < 0)
	{
		Sys_Error ("Invalid packfile %s (dirlen: %i, dirofs: %i)", packfile, header.dirlen, header.dirofs);
	}
	if (!numpackfiles)
	{
		Sys_Printf ("WARNING: %s has no files, ignored\n", packfile);
		Sys_FileClose (packhandle);
		return NULL;
	}
	if (numpackfiles > MAX_FILES_IN_PACK)
		Sys_Error ("%s has %i files", packfile, numpackfiles);

	if (numpackfiles != PAK0_COUNT)
		com_modified = true; // not the original file

	newfiles = (packfile_t *)Mem_Alloc (numpackfiles * sizeof (packfile_t));

	Sys_FileSeek (packhandle, header.dirofs);
	Sys_FileRead (packhandle, (void *)info, header.dirlen);

	// crc the directory to check for modifications
	CRC_Init (&crc);
	for (i = 0; i < header.dirlen; i++)
		CRC_ProcessByte (&crc, ((byte *)info)[i]);
	if (crc != PAK0_CRC_V106 && crc != PAK0_CRC_V101 && crc != PAK0_CRC_V100)
		com_modified = true;

	// parse the directory
	for (i = 0; i < numpackfiles; i++)
	{
		q_strlcpy (newfiles[i].name, info[i].name, sizeof (newfiles[i].name));
		newfiles[i].filepos = LittleLong (info[i].filepos);
		newfiles[i].filelen = LittleLong (info[i].filelen);
	}

	pack = (pack_t *)Mem_Alloc (sizeof (pack_t));
	q_strlcpy (pack->filename, packfile, sizeof (pack->filename));
	pack->handle = packhandle;
	pack->numfiles = numpackfiles;
	pack->files = newfiles;

	// Sys_Printf ("Added packfile %s (%i files)\n", packfile, numpackfiles);
	return pack;
}

const char *COM_GetGameNames (qboolean full)
{
	if (full)
	{
		if (*com_gamenames)
			return va ("%s;%s", GAMENAME, com_gamenames);
		else
			return GAMENAME;
	}
	return com_gamenames;
	//	return COM_SkipPath(com_gamedir);
}

// if either contain id1 then that gets ignored
qboolean COM_GameDirMatches (const char *tdirs)
{
	int			gnl = strlen (GAMENAME);
	const char *odirs = COM_GetGameNames (false);

	// ignore any core paths.
	if (!strncmp (tdirs, GAMENAME, gnl) && (tdirs[gnl] == ';' || !tdirs[gnl]))
	{
		tdirs += gnl;
		if (*tdirs == ';')
			tdirs++;
	}
	if (!strncmp (odirs, GAMENAME, gnl) && (odirs[gnl] == ';' || !odirs[gnl]))
	{
		odirs += gnl;
		if (*odirs == ';')
			odirs++;
	}
	// skip any qw in there from quakeworld (remote servers should really be skipping this, unless its maybe the only one in the path).
	if (!strncmp (tdirs, "qw;", 3) || !strcmp (tdirs, "qw"))
	{
		tdirs += 2;
		if (*tdirs == ';')
			tdirs++;
	}
	if (!strncmp (odirs, "qw;", 3) || !strcmp (odirs, "qw")) // need to cope with ourselves setting it that way too, just in case.
	{
		odirs += 2;
		if (*odirs == ';')
			odirs++;
	}

	// okay, now check it properly
	if (!strcmp (odirs, tdirs))
		return true;
	return false;
}

/*
=================
COM_AddGameDirectory -- johnfitz -- modified based on topaz's tutorial
=================
*/
static void COM_AddGameDirectoryRoot (const char *base, const char *dir, unsigned int path_id, qboolean add_embedded)
{
	int			  i, packhandle;
	searchpath_t *search;
	pack_t		 *pak;
	char		  pakfile[MAX_OSPATH];
	static byte	 *vkquake_pak_extracted;

	q_strlcpy (com_gamedir, va ("%s/%s", base, dir), sizeof (com_gamedir));

	// add the directory to the search path
	search = (searchpath_t *)Mem_Alloc (sizeof (searchpath_t));
	search->path_id = path_id;
	q_strlcpy (search->filename, com_gamedir, sizeof (search->filename));
	q_strlcpy (search->dir, dir, sizeof (search->dir));
	search->next = com_searchpaths;
	com_searchpaths = search;

	// add any pak files in the format pak0.pak pak1.pak, ...
	for (i = 0;; i++)
	{
		q_snprintf (pakfile, sizeof (pakfile), "%s/pak%i.pak", com_gamedir, i);
		if (Sys_FileOpenRead (pakfile, &packhandle) == -1)
			break;
		pak = COM_LoadPackFile (pakfile, packhandle);
		if (pak)
		{
			search = (searchpath_t *)Mem_Alloc (sizeof (searchpath_t));
			search->path_id = path_id;
			search->pack = pak;
			q_strlcpy (search->dir, dir, sizeof (search->dir));
			search->next = com_searchpaths;
			com_searchpaths = search;
		}

		if ((i == 0) && (path_id == 1) && add_embedded)
		{
			size_t vkquake_pak_size_compressed = vkquake_pak_size, vkquake_pak_size_extracted = vkquake_pak_decompressed_size;
			if (!vkquake_pak_extracted)
			{
				tinfl_decompressor inflator;
				tinfl_init (&inflator);
				vkquake_pak_extracted = Mem_Alloc (vkquake_pak_size_extracted);

				tinfl_status decomp_status = tinfl_decompress (
					&inflator, vkquake_pak, &vkquake_pak_size_compressed, vkquake_pak_extracted, vkquake_pak_extracted, &vkquake_pak_size_extracted,
					TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF);

				if (TINFL_STATUS_DONE != decomp_status)
					Sys_Error ("Error extracting embedded pack");
			}
			qboolean pak0_modified = com_modified;
			Sys_MemFileOpenRead (vkquake_pak_extracted, vkquake_pak_size_extracted, &packhandle);
			pak = COM_LoadPackFile ("vkquake.pak", packhandle);
			search = (searchpath_t *)Mem_Alloc (sizeof (searchpath_t));
			search->path_id = path_id;
			search->pack = pak;
			q_strlcpy (search->dir, dir, sizeof (search->dir));
			search->next = com_searchpaths;
			com_searchpaths = search;
			com_modified = pak0_modified;
		}

		if (!pak)
			break;
	}
}

static void COM_AddGameDirectory (const char *dir)
{
	int			 i;
	unsigned int path_id;
	char		 path[MAX_OSPATH];

	if (*com_gamenames)
		q_strlcat (com_gamenames, ";", sizeof (com_gamenames));
	q_strlcat (com_gamenames, dir, sizeof (com_gamenames));

	// quakespasm enables mission pack flags automatically,
	// so e.g. -game rogue works without breaking the hud
	if (!q_strcasecmp (dir, "rogue"))
	{
		rogue = true;
		standard_quake = false;
	}
	if (!q_strcasecmp (dir, "hipnotic") || !q_strcasecmp (dir, "quoth"))
	{
		hipnotic = true;
		standard_quake = false;
	}

	// assign a path_id to this game directory; all roots share it
	if (com_searchpaths)
		path_id = com_searchpaths->path_id << 1;
	else
		path_id = 1U;

	// mount all roots in order: the extras sit below the main basedir (so it
	// takes precedence on conflicts), the userdir on top as the write target
	for (i = 0; i < com_numbasedirs; i++)
	{
		qboolean is_main = !q_strcasecmp (com_basedirs[i], com_basedir);
		qboolean is_user = (host_parms->userdir != host_parms->basedir) && !q_strcasecmp (com_basedirs[i], host_parms->userdir);

		q_snprintf (path, sizeof (path), "%s/%s", com_basedirs[i], dir);
		if (is_user)
			Sys_mkdir (path);
		else if (!is_main && Sys_FileType (path) != FS_ENT_DIRECTORY)
			continue;
		COM_AddGameDirectoryRoot (com_basedirs[i], dir, path_id, is_main);
	}
}

void COM_ResetGameDirectories (const char *newdirs)
{
	char		 *newgamedirs = q_strdup (newdirs);
	char		 *newpath, *path;
	searchpath_t *search;
	// Kill the extra game if it is loaded
	while (com_searchpaths != com_base_searchpaths)
	{
		if (com_searchpaths->pack)
		{
			Sys_FileClose (com_searchpaths->pack->handle);
			Mem_Free (com_searchpaths->pack->files);
			Mem_Free (com_searchpaths->pack);
		}
		search = com_searchpaths->next;
		Mem_Free (com_searchpaths);
		com_searchpaths = search;
	}
	hipnotic = false;
	rogue = false;
	standard_quake = true;
	// wipe the list of mod gamedirs
	*com_gamenames = 0;
	// reset this too
	q_strlcpy (com_gamedir, va ("%s/%s", com_basedirs[com_numbasedirs - 1], GAMENAME), sizeof (com_gamedir));

	for (newpath = newgamedirs; newpath && *newpath;)
	{
		char *e = strchr (newpath, ';');
		if (e)
			*e++ = 0;

		if (!q_strcasecmp (GAMENAME, newpath))
			path = NULL;
		else
		{
			for (path = newgamedirs; path < newpath; path += strlen (path) + 1)
			{
				if (!q_strcasecmp (path, newpath))
					break;
			}
		}

		if (path == newpath) // not already loaded
			COM_AddGameDirectory (newpath);
		newpath = e;
	}
	Mem_Free (newgamedirs);
}

qboolean COM_ModForbiddenChars (const char *p)
{
	return !*p || !strcmp (p, ".") || strstr (p, "..") || strstr (p, "/") || strstr (p, "\\") || strstr (p, ":") || strstr (p, "\"") || strstr (p, ";");
}

/*
=================
COM_IsValidFlavorDir

Returns true if the directory contains usable game data for the given
flavor: classic id1/pak0.pak or the rerelease QuakeEX.kpf (-1 accepts
either)
=================
*/
static qboolean COM_IsValidFlavorDir (const char *dir, int flavor)
{
	char path[MAX_OSPATH];

	if (flavor != QUAKE_FLAVOR_REMASTERED && (size_t)q_snprintf (path, sizeof (path), "%s/" GAMENAME "/pak0.pak", dir) < sizeof (path) &&
		Sys_FileType (path) == FS_ENT_FILE)
		return true;
	if (flavor != QUAKE_FLAVOR_ORIGINAL && (size_t)q_snprintf (path, sizeof (path), "%s/QuakeEX.kpf", dir) < sizeof (path) &&
		Sys_FileType (path) == FS_ENT_FILE)
		return true;

	return false;
}

/*
=================
COM_RequestedQuakeFlavor

Quake version requested on the command line, -1 if none
=================
*/
static int COM_RequestedQuakeFlavor (void)
{
	if (COM_CheckParm ("-prefremaster") || COM_CheckParm ("-remaster") || COM_CheckParm ("-remastered"))
		return QUAKE_FLAVOR_REMASTERED;
	if (COM_CheckParm ("-preforiginal") || COM_CheckParm ("-original"))
		return QUAKE_FLAVOR_ORIGINAL;
	return -1;
}

/*
=================
COM_FOpenPrefFile

Opens a file in the per-user preferences directory
(%APPDATA%\vkQuake on Windows)
=================
*/
FILE *COM_FOpenPrefFile (const char *filename, const char *mode)
{
	char *pref_path;
	FILE *f;

	// harness runs must be hermetic: per-user state (configs, history,
	// remembered basedirs) is redirected into the disposable gamedir
	if (harness_active)
		return Sys_fopen (va ("%s/%s", com_gamedir, filename), mode);

	pref_path = Sys_GetPrefPath ("", "vkQuake");
	f = Sys_fopen (va ("%s/%s", pref_path, filename), mode);
	Mem_Free (pref_path);
	return f;
}

/*
=================
COM_SetUserPrefDir

Makes the per-user preferences directory the userdir, i.e. the write
target for saves, configs, screenshots etc. and the top-priority
content root. No-op if a real userdir is already set up.
=================
*/
static void COM_SetUserPrefDir (void)
{
	static char userprefdir[MAX_OSPATH];
	char	   *pref_path;
	size_t		len;

	if (host_parms->userdir != host_parms->basedir)
		return;
	pref_path = Sys_GetPrefPath ("", "vkQuake");
	if (!pref_path)
		return;

	len = q_strlcpy (userprefdir, pref_path, sizeof (userprefdir));
	Mem_Free (pref_path);
	len = q_min (len, sizeof (userprefdir) - 1);
	while (len > 0 && IS_DIR_SEPARATOR (userprefdir[len - 1]))
		userprefdir[--len] = '\0';

	host_parms->userdir = userprefdir;
	Sys_Printf ("Writing user files to %s\n", userprefdir);
}

#ifdef USE_SDL3
/*
=================
COM_LoadSelectedBaseDirs

Game folders the user picked in the folder dialog, kept in basedirs.txt
in the pref dir. A new pick is only written back once the engine is
fully initialized (COM_WriteSelectedBaseDir) so a folder with broken
data can't get remembered.
=================
*/
static char		com_storedbasedirs[2][MAX_OSPATH]; // indexed by quakeflavor_t
static qboolean com_pendingbasedirwrite;

static void COM_LoadSelectedBaseDirs (void)
{
	char  line[MAX_OSPATH + 16];
	FILE *f = COM_FOpenPrefFile ("basedirs.txt", "r");

	if (!f)
		return;

	while (fgets (line, sizeof (line), f))
	{
		char *path = strchr (line, ' ');
		if (!path)
			continue;
		*path++ = '\0';
		path[strcspn (path, "\r\n")] = '\0';
		if (!strcmp (line, "classic"))
			q_strlcpy (com_storedbasedirs[QUAKE_FLAVOR_ORIGINAL], path, MAX_OSPATH);
		else if (!strcmp (line, "remastered"))
			q_strlcpy (com_storedbasedirs[QUAKE_FLAVOR_REMASTERED], path, MAX_OSPATH);
	}

	fclose (f);
}

/*
=================
COM_SelectBaseDir

Asks the user for a game folder until it contains data for the wanted
flavor (-1 accepts either), starting at the folder remembered from a
previous run. Exits cleanly when the user cancels the dialog; returns
false when no dialog could be shown so the caller falls through to
the regular missing-data error
=================
*/
static qboolean COM_SelectBaseDir (int flavor, char *dst, size_t dstsize)
{
	const char *title, *complaint, *default_location;
	int			result;

	switch (flavor)
	{
	case QUAKE_FLAVOR_ORIGINAL:
		title = "Select your classic Quake folder";
		complaint = "The selected folder does not contain " GAMENAME "/pak0.pak.";
		default_location = com_storedbasedirs[QUAKE_FLAVOR_ORIGINAL];
		break;
	case QUAKE_FLAVOR_REMASTERED:
		title = "Select your remastered Quake folder";
		complaint = "The selected folder does not contain QuakeEX.kpf.";
		default_location = com_storedbasedirs[QUAKE_FLAVOR_REMASTERED];
		break;
	default:
		title = "Select your Quake folder";
		complaint = "The selected folder does not contain Quake game data (" GAMENAME "/pak0.pak or QuakeEX.kpf).";
		default_location =
			com_storedbasedirs[QUAKE_FLAVOR_REMASTERED][0] ? com_storedbasedirs[QUAKE_FLAVOR_REMASTERED] : com_storedbasedirs[QUAKE_FLAVOR_ORIGINAL];
		break;
	}

	while ((result = Sys_SelectFolder (title, default_location, dst, dstsize)) > 0)
	{
		if (COM_IsValidFlavorDir (dst, flavor))
			return true;
		Sys_MessageBoxWarning ("vkqr-engine", complaint);
	}

	if (result == 0) // cancelled
		Sys_QuitNoShutdown ();

	return false; // no dialog could be shown
}

static void COM_SetPendingBaseDir (int flavor, const char *dir)
{
	q_strlcpy (com_storedbasedirs[flavor], dir, MAX_OSPATH);
	com_pendingbasedirwrite = true;
}
#endif

/*
=================
COM_WriteSelectedBaseDir

Remembers the folder picked in the dialog; called once the engine is
fully initialized as proof the folder contains working game data
=================
*/
void COM_WriteSelectedBaseDir (void)
{
#ifdef USE_SDL3
	FILE *f;

	if (!com_pendingbasedirwrite)
		return;

	f = COM_FOpenPrefFile ("basedirs.txt", "w");
	if (!f)
		return;

	if (com_storedbasedirs[QUAKE_FLAVOR_ORIGINAL][0])
		fprintf (f, "classic %s\n", com_storedbasedirs[QUAKE_FLAVOR_ORIGINAL]);
	if (com_storedbasedirs[QUAKE_FLAVOR_REMASTERED][0])
		fprintf (f, "remastered %s\n", com_storedbasedirs[QUAKE_FLAVOR_REMASTERED]);

	fclose (f);
	com_pendingbasedirwrite = false;
#endif
}

/*
=================
COM_MountNightdiveUserDir

The official rerelease client downloads add-ons into its user dir
(e.g. Saved Games/Nightdive Studios/Quake); mount it as an extra
content root so they show up in the mods menu (like Ironwail does)
=================
*/
static char com_nightdivedir[MAX_OSPATH];

static void COM_MountNightdiveUserDir (void)
{
	if (!com_nightdivedir[0] || COM_CheckParm ("-nonightdive"))
		return;
	if (Sys_FileType (com_nightdivedir) != FS_ENT_DIRECTORY)
		return;

	COM_AddBaseDir (com_nightdivedir);
	Sys_Printf ("Mounted Nightdive add-on dir %s\n", com_nightdivedir);
}

/*
=================
COM_FindStoreBaseDir

Locates a Steam/GOG/Epic Games Store install of Quake and points
com_basedir at it (based on the Ironwail startup flow). Used when
the working directory has no game data and no -basedir was given.
Asks the user for the folder when the requested version isn't found,
starting at the previously picked folder.
=================
*/
static qboolean COM_FindStoreBaseDir (void)
{
	steamgame_t	  steamquake;
	char		  original[MAX_OSPATH] = {0};
	char		  remastered[MAX_OSPATH] = {0};
	quakeflavor_t flavor;
	int			  requested;
	qboolean	  force_steam = COM_CheckParm ("-steam") != 0;
	qboolean	  force_gog = COM_CheckParm ("-gog") != 0;
	qboolean	  force_egs = (COM_CheckParm ("-egs") || COM_CheckParm ("-epic")) != 0;
	qboolean	  forced = force_steam || force_gog || force_egs;

	if ((!forced || force_steam) && !COM_CheckParm ("-nosteam"))
	{
		if (Steam_FindGame (&steamquake, QUAKE_STEAM_APPID) && Steam_ResolvePath (original, sizeof (original), &steamquake))
		{
			if ((size_t)q_snprintf (remastered, sizeof (remastered), "%s/rerelease", original) >= sizeof (remastered))
				remastered[0] = '\0';
			else if (!Sys_GetNightdiveUserDir (com_nightdivedir, sizeof (com_nightdivedir), steamquake.library))
				com_nightdivedir[0] = '\0';
		}
	}

	if ((!forced || force_gog) && !COM_CheckParm ("-nogog"))
	{
		if (!original[0] && !Sys_GetGOGQuakeDir (original, sizeof (original)))
			original[0] = '\0';
		if (!remastered[0])
		{
			if (Sys_GetGOGQuakeEnhancedDir (remastered, sizeof (remastered)))
			{
				if (!com_nightdivedir[0] && !Sys_GetNightdiveUserDir (com_nightdivedir, sizeof (com_nightdivedir), NULL))
					com_nightdivedir[0] = '\0';
			}
			else
				remastered[0] = '\0';
		}
	}

	if ((!forced || force_egs) && !COM_CheckParm ("-noegs") && !COM_CheckParm ("-noepic"))
	{
		if (!remastered[0])
		{
			if (EGS_FindGame (remastered, sizeof (remastered), QUAKE_EGS_NAMESPACE, QUAKE_EGS_ITEM_ID, QUAKE_EGS_APP_NAME))
			{
				if (!com_nightdivedir[0] && !Sys_GetNightdiveUserDir (com_nightdivedir, sizeof (com_nightdivedir), NULL))
					com_nightdivedir[0] = '\0';
			}
			else
				remastered[0] = '\0';
		}
	}

	if (original[0] && !COM_IsValidFlavorDir (original, QUAKE_FLAVOR_ORIGINAL))
		original[0] = '\0';
	if (remastered[0] && !COM_IsValidFlavorDir (remastered, QUAKE_FLAVOR_REMASTERED))
		remastered[0] = com_nightdivedir[0] = '\0';

	requested = COM_RequestedQuakeFlavor ();

	if (!forced && !isDedicated)
	{
#ifdef USE_SDL3
		COM_LoadSelectedBaseDirs ();

		// use the folder picked in a previous run unless the user wants a new one
		if (!COM_CheckParm ("-select-basedir"))
		{
			if (!original[0] && com_storedbasedirs[QUAKE_FLAVOR_ORIGINAL][0] &&
				COM_IsValidFlavorDir (com_storedbasedirs[QUAKE_FLAVOR_ORIGINAL], QUAKE_FLAVOR_ORIGINAL))
				q_strlcpy (original, com_storedbasedirs[QUAKE_FLAVOR_ORIGINAL], sizeof (original));
			if (!remastered[0] && com_storedbasedirs[QUAKE_FLAVOR_REMASTERED][0] &&
				COM_IsValidFlavorDir (com_storedbasedirs[QUAKE_FLAVOR_REMASTERED], QUAKE_FLAVOR_REMASTERED))
				q_strlcpy (remastered, com_storedbasedirs[QUAKE_FLAVOR_REMASTERED], sizeof (remastered));
		}

		// still missing: ask for the folder, remember it only once it's usable
		if (requested == QUAKE_FLAVOR_ORIGINAL && !original[0])
		{
			if (COM_SelectBaseDir (QUAKE_FLAVOR_ORIGINAL, original, sizeof (original)))
				COM_SetPendingBaseDir (QUAKE_FLAVOR_ORIGINAL, original);
		}
		else if (requested == QUAKE_FLAVOR_REMASTERED && !remastered[0])
		{
			if (COM_SelectBaseDir (QUAKE_FLAVOR_REMASTERED, remastered, sizeof (remastered)))
				COM_SetPendingBaseDir (QUAKE_FLAVOR_REMASTERED, remastered);
		}
		else if (requested < 0 && !original[0] && !remastered[0])
		{
			char selected[MAX_OSPATH];
			if (COM_SelectBaseDir (-1, selected, sizeof (selected)))
			{
				if (COM_IsValidFlavorDir (selected, QUAKE_FLAVOR_REMASTERED))
				{
					q_strlcpy (remastered, selected, sizeof (remastered));
					COM_SetPendingBaseDir (QUAKE_FLAVOR_REMASTERED, selected);
				}
				else
				{
					q_strlcpy (original, selected, sizeof (original));
					COM_SetPendingBaseDir (QUAKE_FLAVOR_ORIGINAL, selected);
				}
			}
		}
#else
		// no folder picker without the SDL3 dialog API
		if (requested == QUAKE_FLAVOR_ORIGINAL && !original[0])
			Sys_Error ("Couldn't find the classic Quake folder. Use -basedir to specify it.");
		else if (requested == QUAKE_FLAVOR_REMASTERED && !remastered[0])
			Sys_Error ("Couldn't find the remastered Quake folder. Use -basedir to specify it.");
#endif
	}

	if (!original[0] && !remastered[0])
	{
		if (force_steam)
			Sys_Error ("Couldn't find Steam Quake");
		if (force_gog)
			Sys_Error ("Couldn't find GOG Quake");
		if (force_egs)
			Sys_Error ("Couldn't find Epic Games Store Quake");
		return false; // fall through to the regular missing-data error
	}

	if (requested == QUAKE_FLAVOR_REMASTERED && remastered[0])
		flavor = QUAKE_FLAVOR_REMASTERED;
	else if (requested == QUAKE_FLAVOR_ORIGINAL && original[0])
		flavor = QUAKE_FLAVOR_ORIGINAL;
	else if (original[0] && remastered[0])
		flavor = ChooseQuakeFlavor ();
	else
		flavor = remastered[0] ? QUAKE_FLAVOR_REMASTERED : QUAKE_FLAVOR_ORIGINAL;

	q_strlcpy (com_basedir, (flavor == QUAKE_FLAVOR_REMASTERED) ? remastered : original, sizeof (com_basedir));
	Sys_Printf ("Using Quake data from %s\n", com_basedir);

	if (flavor == QUAKE_FLAVOR_REMASTERED)
		COM_MountNightdiveUserDir ();
	else
		com_nightdivedir[0] = '\0';

	return true;
}

/*
=================
COM_IsPathPrefix

Compares path components case-insensitively, treating / and \ as equal
=================
*/
static qboolean COM_IsPathPrefix (const char *prefix, const char *path)
{
	size_t i, len = strlen (prefix);

	for (i = 0; i < len; i++)
	{
		char a = (prefix[i] == '\\') ? '/' : prefix[i];
		char b = (path[i] == '\\') ? '/' : path[i];
		if (q_tolower (a) != q_tolower (b))
			return false;
	}

	return path[len] == '\0' || path[len] == '/' || path[len] == '\\';
}

/*
=================
COM_InitSteamAPI

Enables Steam achievements and rich presence when the game data
comes from the Steam install (from Ironwail)
=================
*/
static void COM_InitSteamAPI (void)
{
	steamgame_t steamquake;
	char		steampath[MAX_OSPATH];

	if (COM_CheckParm ("-nosteam"))
		return;
	if (!Steam_FindGame (&steamquake, QUAKE_STEAM_APPID) || !Steam_ResolvePath (steampath, sizeof (steampath), &steamquake))
		return;
	if (!COM_IsPathPrefix (steampath, com_basedir))
		return;

	Steam_Init (&steamquake);
}

/*
=================
COM_InitFilesystem
=================
*/
void COM_InitFilesystem (void) // johnfitz -- modified based on topaz's tutorial
{
	int			i, j;
	const char *p;

	Cvar_RegisterVariable (&registered);
	Cvar_RegisterVariable (&cmdline);
	Cmd_AddCommand ("path", COM_Path_f);
	Cmd_AddCommand ("game", COM_Game_f); // johnfitz

	i = COM_CheckParm ("-basedir");
	if (i && i < com_argc - 1)
		q_strlcpy (com_basedir, com_argv[i + 1], sizeof (com_basedir));
	else
		q_strlcpy (com_basedir, host_parms->basedir, sizeof (com_basedir));

	j = strlen (com_basedir);
	if (j < 1)
		Sys_Error ("Bad argument to -basedir");
	if ((com_basedir[j - 1] == '\\') || (com_basedir[j - 1] == '/'))
		com_basedir[j - 1] = 0;

	// no explicit -basedir: run store detection if the working directory has no
	// game data for the requested version (any version if none was requested),
	// or if a store was named explicitly on the command line
	qboolean store_install = false;
	if (!i && (!COM_IsValidFlavorDir (com_basedir, COM_RequestedQuakeFlavor ()) || COM_CheckParm ("-steam") || COM_CheckParm ("-gog") ||
			   COM_CheckParm ("-egs") || COM_CheckParm ("-epic")))
		store_install = COM_FindStoreBaseDir ();

	// keep all writes out of the game dirs: always for store installs (the
	// user didn't opt into writing there, and they might not even be
	// writable), otherwise only when -multiuser asks for it
	if (store_install || multiuser)
		COM_SetUserPrefDir ();

	// achievements/rich presence if the game data comes from the Steam install,
	// no matter whether it was found by detection, -basedir or the working directory
	COM_InitSteamAPI ();

	// register the remaining content roots: the main basedir above the extras
	// added so far, the userdir on top of everything as the write target
	COM_AddBaseDir (com_basedir);
	if (host_parms->userdir != host_parms->basedir)
		COM_AddBaseDir (host_parms->userdir);

	i = COM_CheckParmNext (i, "-basegame");
	if (i)
	{ //-basegame:
		// a) replaces all hardcoded dirs (read: alternative to id1)
		// b) isn't flushed on normal gamedir switches (like id1).
		com_modified = true; // shouldn't be relevant when not using id content... but we don't really know.
		for (;; i = COM_CheckParmNext (i, "-basegame"))
		{
			if (!i || i >= com_argc - 1)
				break;

			p = com_argv[i + 1];
			if (COM_ModForbiddenChars (p))
				Sys_Error ("gamedir should be a single directory name, not a path\n");
			if (p != NULL)
				COM_AddGameDirectory (p);
		}
	}
	else
	{
		// start up with GAMENAME by default (id1)
		COM_AddGameDirectory (GAMENAME);
	}

	/* this is the end of our base searchpath:
	 * any set gamedirs, such as those from -game command line
	 * arguments or by the 'game' console command will be freed
	 * up to here upon a new game command. */
	com_base_searchpaths = com_searchpaths;
	COM_ResetGameDirectories ("");

	// add mission pack requests (only one should be specified)
	if (COM_CheckParm ("-rogue"))
		COM_AddGameDirectory ("rogue");
	if (COM_CheckParm ("-hipnotic"))
		COM_AddGameDirectory ("hipnotic");
	if (COM_CheckParm ("-quoth"))
		COM_AddGameDirectory ("quoth");

	for (i = 0;;)
	{
		i = COM_CheckParmNext (i, "-game");
		if (!i || i >= com_argc - 1)
			break;

		p = com_argv[i + 1];
		if (COM_ModForbiddenChars (p))
			Sys_Error ("gamedir should be a single directory name, not a path\n");
		com_modified = true;
		if (p != NULL)
			COM_AddGameDirectory (p);
	}

	COM_CheckRegistered ();
}

/* The following FS_*() stdio replacements are necessary if one is
 * to perform non-sequential reads on files reopened on pak files
 * because we need the bookkeeping about file start/end positions.
 * Allocating and filling in the fshandle_t structure is the users'
 * responsibility when the file is initially opened. */

size_t FS_fread (void *ptr, size_t size, size_t nmemb, fshandle_t *fh)
{
	qfilesize_t byte_size;
	qfilesize_t bytes_read;
	qfilesize_t nmemb_read;

	if (!fh)
	{
		errno = EBADF;
		return 0;
	}
	if (!ptr)
	{
		errno = EFAULT;
		return 0;
	}
	if (!size || !nmemb)
	{ /* no error, just zero bytes wanted */
		errno = 0;
		return 0;
	}

	byte_size = nmemb * size;
	if (byte_size > fh->length - fh->pos) /* just read to end */
		byte_size = fh->length - fh->pos;
	bytes_read = fread (ptr, 1, byte_size, fh->file);
	fh->pos += bytes_read;

	/* fread() must return the number of elements read,
	 * not the total number of bytes. */
	nmemb_read = bytes_read / size;
	/* even if the last member is only read partially
	 * it is counted as a whole in the return value. */
	if (bytes_read % size)
		nmemb_read++;

	return nmemb_read;
}

int FS_fseek (fshandle_t *fh, qfileofs_t offset, int whence)
{
	int ret;

	if (!fh)
	{
		errno = EBADF;
		return -1;
	}

	/* the relative file position shouldn't be smaller
	 * than zero or bigger than the filesize. */
	switch (whence)
	{
	case SEEK_SET:
		break;
	case SEEK_CUR:
		offset += fh->pos;
		break;
	case SEEK_END:
		offset = fh->length + offset;
		break;
	default:
		errno = EINVAL;
		return -1;
	}

	if (offset < 0)
	{
		errno = EINVAL;
		return -1;
	}

	if (offset > fh->length) /* just seek to end */
		offset = fh->length;

	ret = Sys_fseek (fh->file, fh->start + offset, SEEK_SET);
	if (ret < 0)
		return ret;

	fh->pos = offset;
	return 0;
}

int FS_fclose (fshandle_t *fh)
{
	if (!fh)
	{
		errno = EBADF;
		return -1;
	}
	return fclose (fh->file);
}

qfileofs_t FS_ftell (fshandle_t *fh)
{
	if (!fh)
	{
		errno = EBADF;
		return -1;
	}
	return fh->pos;
}

void FS_rewind (fshandle_t *fh)
{
	if (!fh)
		return;
	clearerr (fh->file);
	Sys_fseek (fh->file, fh->start, SEEK_SET);
	fh->pos = 0;
}

int FS_feof (fshandle_t *fh)
{
	if (!fh)
	{
		errno = EBADF;
		return -1;
	}
	if (fh->pos >= fh->length)
		return -1;
	return 0;
}

int FS_ferror (fshandle_t *fh)
{
	if (!fh)
	{
		errno = EBADF;
		return -1;
	}
	return ferror (fh->file);
}

int FS_fgetc (fshandle_t *fh)
{
	if (!fh)
	{
		errno = EBADF;
		return EOF;
	}
	if (fh->pos >= fh->length)
		return EOF;
	fh->pos += 1;
	return fgetc (fh->file);
}

char *FS_fgets (char *s, int size, fshandle_t *fh)
{
	char *ret;

	if (FS_feof (fh))
		return NULL;

	if (size > (fh->length - fh->pos) + 1)
		size = (fh->length - fh->pos) + 1;

	ret = fgets (s, size, fh->file);
	fh->pos = Sys_ftell (fh->file) - fh->start;

	return ret;
}

qfilesize_t FS_filelength (fshandle_t *fh)
{
	if (!fh)
	{
		errno = EBADF;
		return -1;
	}
	return fh->length;
}

#ifdef PSET_SCRIPT
// for compat with dpp7 protocols, and mods that cba to precache things.
void COM_Effectinfo_Enumerate (int (*cb) (const char *pname))
{
	int				   i;
	const char		  *f, *e;
	char			  *buf;
	static const char *dpnames[] = {"TE_GUNSHOT",		"TE_GUNSHOTQUAD",
									"TE_SPIKE",			"TE_SPIKEQUAD",
									"TE_SUPERSPIKE",	"TE_SUPERSPIKEQUAD",
									"TE_WIZSPIKE",		"TE_KNIGHTSPIKE",
									"TE_EXPLOSION",		"TE_EXPLOSIONQUAD",
									"TE_TAREXPLOSION",	"TE_TELEPORT",
									"TE_LAVASPLASH",	"TE_SMALLFLASH",
									"TE_FLAMEJET",		"EF_FLAME",
									"TE_BLOOD",			"TE_SPARK",
									"TE_PLASMABURN",	"TE_TEI_G3",
									"TE_TEI_SMOKE",		"TE_TEI_BIGEXPLOSION",
									"TE_TEI_PLASMAHIT", "EF_STARDUST",
									"TR_ROCKET",		"TR_GRENADE",
									"TR_BLOOD",			"TR_WIZSPIKE",
									"TR_SLIGHTBLOOD",	"TR_KNIGHTSPIKE",
									"TR_VORESPIKE",		"TR_NEHAHRASMOKE",
									"TR_NEXUIZPLASMA",	"TR_GLOWTRAIL",
									"SVC_PARTICLE",		NULL};

	buf = (char *)COM_LoadFile ("effectinfo.txt", NULL);
	if (!buf)
		return;

	for (i = 0; dpnames[i]; i++)
		cb (dpnames[i]);

	for (f = buf; f; f = e)
	{
		e = COM_Parse (f);
		if (!strcmp (com_token, "effect"))
		{
			e = COM_Parse (e);
			cb (com_token);
		}
		while (e && *e && *e != '\n')
			e++;
	}
	Mem_Free (buf);
}
#endif

/*
============================================================================
								LOCALIZATION
============================================================================
*/
typedef struct
{
	char *key;
	char *value;
} locentry_t;

typedef struct
{
	int			numentries;
	int			maxnumentries;
	int			numindices;
	unsigned   *indices;
	locentry_t *entries;
	char	   *text;
} localization_t;

static localization_t localization;

/*
================
COM_HashString
Computes the FNV-1a hash of string str
================
*/
unsigned COM_HashString (const char *str)
{
	unsigned hash = 0x811c9dc5u;
	while (*str)
	{
		hash ^= *str++;
		hash *= 0x01000193u;
	}
	return hash;
}

/*
================
COM_HashBlock
Computes the FNV-1a hash of a memory block
================
*/
unsigned COM_HashBlock (const void *data, size_t size)
{
	const byte *ptr = (const byte *)data;
	unsigned	hash = 0x811c9dc5u;
	while (size--)
	{
		hash ^= *ptr++;
		hash *= 0x01000193u;
	}
	return hash;
}

static size_t mz_zip_file_read_func (void *opaque, mz_uint64 ofs, void *buf, size_t n)
{
	int handle = (int)(intptr_t)opaque;
	int nread;
	if (Sys_FileSeek (handle, (qfileofs_t)ofs) != 0)
		return 0;
	nread = Sys_FileRead (handle, buf, (int)n);
	return (nread < 0) ? 0 : (size_t)nread;
}

/*
================
LOC_LoadFile
================
*/
void LOC_LoadFile (const char *file)
{
	char  path[1024];
	int	  i, lineno;
	char *cursor;

	int			   handle = -1;
	qfilesize_t	   sz;
	mz_zip_archive archive;
	size_t		   size = 0;

	// clear existing data
	if (localization.text)
	{
		Mem_Free (localization.text);
		localization.text = NULL;
	}
	localization.numentries = 0;
	localization.numindices = 0;

	if (!file || !*file)
		return;

	Con_Printf ("\nLanguage initialization\n");

	localization.text = (char *)COM_LoadFile (file, NULL);
	if (localization.text)
		goto loaded;

	memset (&archive, 0, sizeof (archive));
	q_snprintf (path, sizeof (path), "%s/%s", com_basedir, file);
	sz = Sys_FileOpenRead (path, &handle);
#if defined(DO_USERDIRS)
	if (handle < 0)
	{
		q_snprintf (path, sizeof (path), "%s/%s", host_parms->userdir, file);
		sz = Sys_FileOpenRead (path, &handle);
	}
#endif
	if (handle < 0)
	{
		q_snprintf (path, sizeof (path), "%s/QuakeEX.kpf", com_basedir);
		sz = Sys_FileOpenRead (path, &handle);
#if defined(DO_USERDIRS)
		if (handle < 0)
		{
			q_snprintf (path, sizeof (path), "%s/QuakeEX.kpf", host_parms->userdir);
			sz = Sys_FileOpenRead (path, &handle);
		}
#endif
		if (handle < 0)
			goto fail;
		if (sz <= 0)
			goto fail;
		archive.m_pRead = mz_zip_file_read_func;
		archive.m_pIO_opaque = (void *)(intptr_t)handle;
		if (!mz_zip_reader_init (&archive, sz, 0))
			goto fail;
		localization.text = (char *)mz_zip_reader_extract_file_to_heap (&archive, file, &size, 0);
		if (!localization.text)
			goto fail;
		mz_zip_reader_end (&archive);
		Sys_FileClose (handle);
		handle = -1;
		localization.text = (char *)Mem_Realloc (localization.text, size + 1);
		localization.text[size] = 0;
	}
	else
	{
		if (sz <= 0)
			goto fail;
		localization.text = (char *)Mem_Alloc (sz + 1);
		if (!localization.text)
		{
		fail:
			mz_zip_reader_end (&archive);
			if (handle >= 0)
				Sys_FileClose (handle);
			Con_Printf ("Couldn't load '%s'\nfrom '%s'\n", file, com_basedir);
			return;
		}
		Sys_FileRead (handle, localization.text, (int)sz);
		Sys_FileClose (handle);
		handle = -1;
	}
loaded:
	cursor = localization.text;

	// skip BOM
	if ((unsigned char)(cursor[0]) == 0xEF && (unsigned char)(cursor[1]) == 0xBB && (unsigned char)(cursor[2]) == 0xBF)
		cursor += 3;

	lineno = 0;
	while (*cursor)
	{
		char *line, *equals;

		lineno++;

		// skip leading whitespace
		while (q_isblank (*cursor))
			++cursor;

		line = cursor;
		equals = NULL;
		// find line end and first equals sign, if any
		while (*cursor && *cursor != '\n')
		{
			if (*cursor == '=' && !equals)
				equals = cursor;
			cursor++;
		}

		if (line[0] == '/')
		{
			if (line[1] != '/')
				Con_DPrintf ("LOC_LoadFile: malformed comment on line %d\n", lineno);
		}
		else if (equals)
		{
			char	   *key_end = equals;
			qboolean	leading_quote;
			qboolean	trailing_quote;
			locentry_t *entry;
			char	   *value_src;
			char	   *value_dst;
			char	   *value;

			// trim whitespace before equals sign
			while (key_end != line && q_isspace (key_end[-1]))
				key_end--;
			*key_end = 0;

			value = equals + 1;
			// skip whitespace after equals sign
			while (value != cursor && q_isspace (*value))
				value++;

			leading_quote = (*value == '\"');
			trailing_quote = false;
			value += leading_quote;

			// transform escape sequences in-place
			value_src = value;
			value_dst = value;
			while (value_src != cursor)
			{
				if (*value_src == '\\' && value_src + 1 != cursor)
				{
					char c = value_src[1];
					value_src += 2;
					switch (c)
					{
					case 'n':
						*value_dst++ = '\n';
						break;
					case 't':
						*value_dst++ = '\t';
						break;
					case 'v':
						*value_dst++ = '\v';
						break;
					case 'b':
						*value_dst++ = '\b';
						break;
					case 'f':
						*value_dst++ = '\f';
						break;

					case '"':
					case '\'':
						*value_dst++ = c;
						break;

					default:
						Con_Printf ("LOC_LoadFile: unrecognized escape sequence \\%c on line %d\n", c, lineno);
						*value_dst++ = c;
						break;
					}
					continue;
				}

				if (*value_src == '\"')
				{
					trailing_quote = true;
					*value_dst = 0;
					break;
				}

				*value_dst++ = *value_src++;
			}

			// if not a quoted string, trim trailing whitespace
			if (!trailing_quote)
			{
				while (value_dst != value && q_isblank (value_dst[-1]))
				{
					*value_dst = 0;
					value_dst--;
				}
			}

			if (localization.numentries == localization.maxnumentries)
			{
				// grow by 50%
				localization.maxnumentries += localization.maxnumentries >> 1;
				localization.maxnumentries = q_max (localization.maxnumentries, 32);
				localization.entries = (locentry_t *)Mem_Realloc (localization.entries, sizeof (*localization.entries) * localization.maxnumentries);
			}

			entry = &localization.entries[localization.numentries++];
			entry->key = line;
			entry->value = value;
		}

		if (*cursor)
			*cursor++ = 0; // terminate line and advance to next
	}

	// hash all entries

	localization.numindices = localization.numentries * 2; // 50% load factor
	if (localization.numindices == 0)
	{
		Con_Printf ("No localized strings in file '%s'\n", file);
		return;
	}

	localization.indices = (unsigned *)Mem_Realloc (localization.indices, localization.numindices * sizeof (*localization.indices));
	memset (localization.indices, 0, localization.numindices * sizeof (*localization.indices));

	for (i = 0; i < localization.numentries; i++)
	{
		locentry_t *entry = &localization.entries[i];
		unsigned	pos = COM_HashString (entry->key) % localization.numindices, end = pos;

		for (;;)
		{
			if (!localization.indices[pos])
			{
				localization.indices[pos] = i + 1;
				break;
			}

			++pos;
			if (pos == localization.numindices)
				pos = 0;

			if (pos == end)
				Sys_Error ("LOC_LoadFile failed");
		}
	}

	Con_Printf ("Loaded %d strings from '%s'\n", localization.numentries, file);
}

/*
================
LOC_Init
================
*/
void LOC_Init (void)
{
	LOC_LoadFile ("localization/loc_english.txt");
}

/*
================
LOC_Shutdown
================
*/
void LOC_Shutdown (void)
{
	Mem_Free (localization.indices);
	Mem_Free (localization.entries);
	Mem_Free (localization.text);
	memset (&localization, 0, sizeof (localization));
}

/*
================
LOC_GetRawString

Returns localized string if available, or NULL otherwise
================
*/
const char *LOC_GetRawString (const char *key)
{
	unsigned pos, end;

	if (!localization.numindices || !key || !*key || *key != '$')
		return NULL;
	key++;

	pos = COM_HashString (key) % localization.numindices;
	end = pos;

	do
	{
		unsigned	idx = localization.indices[pos];
		locentry_t *entry;
		if (!idx)
			return NULL;

		entry = &localization.entries[idx - 1];
		if (!strcmp (entry->key, key))
			return entry->value;

		++pos;
		if (pos == localization.numindices)
			pos = 0;
	} while (pos != end);

	return NULL;
}

/*
================
LOC_GetString

Returns localized string if available, or input string otherwise
================
*/
const char *LOC_GetString (const char *key)
{
	const char *value = LOC_GetRawString (key);
	return value ? value : key;
}

/*
================
LOC_ParseArg

Returns argument index (>= 0) and advances the string if it starts with a placeholder ({} or {N}),
otherwise returns a negative value and leaves the pointer unchanged
================
*/
static int LOC_ParseArg (const char **pstr)
{
	int			arg;
	const char *str = *pstr;

	// opening brace
	if (*str != '{')
		return -1;
	++str;

	// optional index, defaulting to 0
	arg = 0;
	while (q_isdigit (*str))
		arg = arg * 10 + *str++ - '0';

	// closing brace
	if (*str != '}')
		return -1;
	*pstr = ++str;

	return arg;
}

/*
================
LOC_HasPlaceholders
================
*/
qboolean LOC_HasPlaceholders (const char *str)
{
	if (!localization.numindices)
		return false;
	while (*str)
	{
		if (LOC_ParseArg (&str) >= 0)
			return true;
		str++;
	}
	return false;
}

/*
================
LOC_Format

Replaces placeholders (of the form {} or {N}) with the corresponding arguments

Returns number of written chars, excluding the NUL terminator
If len > 0, output is always NUL-terminated
================
*/
size_t LOC_Format (const char *format, const char *(*getarg_fn) (int idx, void *userdata), void *userdata, char *out, size_t len)
{
	size_t written = 0;
	int	   numargs = 0;

	if (!len)
	{
		Con_DPrintf ("LOC_Format: no output space\n");
		return 0;
	}
	--len; // reserve space for the terminator

	while (*format && written < len)
	{
		const char *insert;
		size_t		space_left;
		size_t		insert_len;
		int			argindex = LOC_ParseArg (&format);

		if (argindex < 0)
		{
			out[written++] = *format++;
			continue;
		}

		insert = getarg_fn (argindex, userdata);
		space_left = len - written;
		insert_len = strlen (insert);

		if (insert_len > space_left)
		{
			Con_DPrintf ("LOC_Format: overflow at argument #%d\n", numargs);
			insert_len = space_left;
		}

		memcpy (out + written, insert, insert_len);
		written += insert_len;
	}

	if (*format)
		Con_DPrintf ("LOC_Format: overflow\n");

	out[written] = 0;

	return written;
}
