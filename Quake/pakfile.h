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

#ifndef __PAKFILE_H
#define __PAKFILE_H

// pakfile.h -- on-disk PAK archive format (Rust migration core header)

#include "q_types.h"

//
// on-disk pakfile
//
typedef struct
{
	char name[56];
	int	 filepos, filelen;
} dpackfile_t;

typedef struct
{
	char id[4];
	int	 dirofs;
	int	 dirlen;
} dpackheader_t;

#define MAX_FILES_IN_PACK 2048

#endif /* __PAKFILE_H */
