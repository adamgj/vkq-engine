/* Minimal engine-function stubs for the reference C files compiled into the
 * differential test binaries.
 *
 * Phase 2 inversion: common_fs.c/steam.c are now compiled as c_ref_* and are
 * themselves under test, so this file no longer stubs the filesystem entry
 * points (COM_LoadFile, COM_FOpenFile, FS_*...). It stubs the engine seams
 * UNDER the filesystem instead — the Sys_File* handle layer, host_parms, the
 * store-discovery probes, the console — and both the c_ref C side and the
 * Rust shims (quake_rs with the `fs` feature) run through these exact same
 * stubs, so their observable behavior is comparable.
 */

#include <assert.h>
#include <errno.h>
#include <setjmp.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* MSVC declares struct _stat64 / _S_IFDIR here too, so both branches need it */
#include <sys/stat.h>
#include <sys/types.h>

#ifdef _WIN32
#include <direct.h>
#include <io.h>
#else
#include <dirent.h>
#endif

#include "q_ctype.h" /* q_isspace, for the q_strtrim copy below */
#include "steam.h" /* steamgame_t / quakeflavor_t for the Steam_Init stub */

/* ---------------------------------------------------------------------------
 * Sys_Error: setjmp/longjmp trap so tests can differentially compare WHICH
 * inputs fatal (and with what message) instead of aborting the test binary.
 * ctest_try() arms the trap around a callback; an unarmed Sys_Error aborts.
 */

static jmp_buf ctest_sys_error_jmp;
static int	   ctest_sys_error_armed;
static char	   ctest_sys_error_msg[2048];

void Sys_Error (const char *error, ...)
{
	va_list ap;
	va_start (ap, error);
	vsnprintf (ctest_sys_error_msg, sizeof (ctest_sys_error_msg), error, ap);
	va_end (ap);
	if (ctest_sys_error_armed)
	{
		ctest_sys_error_armed = 0;
		longjmp (ctest_sys_error_jmp, 1);
	}
	fprintf (stderr, "Sys_Error: %s\n", ctest_sys_error_msg);
	abort ();
}

/* Runs fn(arg) with the Sys_Error trap armed. Returns 1 if Sys_Error fired
 * (message in ctest_sys_error_message()), 0 if fn returned normally. */
int ctest_try (void (*fn) (void *), void *arg)
{
	if (setjmp (ctest_sys_error_jmp))
		return 1;
	ctest_sys_error_armed = 1;
	ctest_sys_error_msg[0] = 0;
	fn (arg);
	ctest_sys_error_armed = 0;
	return 0;
}

const char *ctest_sys_error_message (void)
{
	return ctest_sys_error_msg;
}

/* ---------------------------------------------------------------------------
 * Console / Sys_Printf capture: both sides print through these stubs, so the
 * exact line sequences (with a tag for which channel) are comparable.
 */

#define CTEST_CON_LOG_MAX 256
static char ctest_con_log[CTEST_CON_LOG_MAX][1024];
static int	ctest_con_log_count;

static void ctest_con_append (const char *tag, const char *fmt, va_list ap)
{
	if (ctest_con_log_count < CTEST_CON_LOG_MAX)
	{
		char  *dst = ctest_con_log[ctest_con_log_count];
		size_t taglen;
		snprintf (dst, 1024, "%s ", tag);
		taglen = strlen (dst);
		vsnprintf (dst + taglen, 1024 - taglen, fmt, ap);
	}
	ctest_con_log_count++;
}

void ctest_clear_con_log (void)
{
	ctest_con_log_count = 0;
}

int ctest_con_log_len (void)
{
	return ctest_con_log_count;
}

const char *ctest_con_log_get (int i)
{
	return ctest_con_log[i];
}

#define CON_STUB(name, tag)                  \
	void name (const char *fmt, ...)         \
	{                                        \
		va_list ap;                          \
		va_start (ap, fmt);                  \
		ctest_con_append (tag, fmt, ap);     \
		va_end (ap);                         \
	}

CON_STUB (Con_Printf, "[con]")
CON_STUB (Con_DPrintf, "[dcon]")
CON_STUB (Con_DPrintf2, "[dcon2]")
CON_STUB (Con_Warning, "[warn]")
CON_STUB (Sys_Printf, "[sys]")

/* ---------------------------------------------------------------------------
 * Memory: Mem_Alloc semantics per Quake/mem.h (zero-initialized)
 */

void *Mem_Alloc (const size_t size)
{
	return calloc (1, size ? size : 1);
}

/* poisoned rather than left uninitialized: the brush loaders leave a few
 * msurface_t fields untouched (Phase 3 M3), and a deterministic fill makes
 * the C and Rust sides agree on those bytes instead of comparing two
 * different heaps' garbage. Still distinct from Mem_Alloc's zeros. */
/* zero-filled on purpose: Mod_ParseFaces ORs into msurface_t::styles_bitmap
 * without initializing it first, so a garbage-filled Mem_AllocNonZero makes
 * the C oracle non-reproducible. Zeroing keeps both sides deterministic; a
 * field the Rust port fails to write still shows up as 0 against the C
 * value whenever that value is non-zero. */
void *Mem_AllocNonZero (const size_t size)
{
	void *p = malloc (size ? size : 1);
	if (p)
		memset (p, 0, size ? size : 1);
	return p;
}

void *Mem_Realloc (void *ptr, const size_t size)
{
	return realloc (ptr, size ? size : 1);
}

void Mem_Free (const void *ptr)
{
	free ((void *)ptr);
}

/* UTF8_WriteCodePoint copied verbatim from Quake/common.c (json.c needs it) */
size_t UTF8_WriteCodePoint (char *dst, size_t maxbytes, uint32_t codepoint)
{
	if (!maxbytes)
		return 0;

	if (codepoint < 0x80)
	{
		dst[0] = (char)codepoint;
		return 1;
	}

	if (codepoint < 0x800)
	{
		if (maxbytes < 2)
			return 0;
		dst[0] = 0xC0 | (codepoint >> 6);
		dst[1] = 0x80 | (codepoint & 63);
		return 2;
	}

	if (codepoint < 0x10000)
	{
		if (maxbytes < 3)
			return 0;
		dst[0] = 0xE0 | (codepoint >> 12);
		dst[1] = 0x80 | ((codepoint >> 6) & 63);
		dst[2] = 0x80 | (codepoint & 63);
		return 3;
	}

	if (codepoint < 0x110000)
	{
		if (maxbytes < 4)
			return 0;
		dst[0] = 0xF0 | (codepoint >> 18);
		dst[1] = 0x80 | ((codepoint >> 12) & 63);
		dst[2] = 0x80 | ((codepoint >> 6) & 63);
		dst[3] = 0x80 | (codepoint & 63);
		return 4;
	}

	return 0;
}

/* COM_SeedRand / COM_Rand copied verbatim from Quake/common.c so stress tests
 * that consume the RNG behave identically */
static uint32_t xorshiro_state[2];

void COM_SeedRand (uint64_t seed)
{
	uint64_t z = (seed + 0x9e3779b97f4a7c15);
	z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9;
	z = (z ^ (z >> 27)) * 0x94d049bb133111eb;
	uint64_t state = z ^ (z >> 31);
	xorshiro_state[0] = (uint32_t)state;
	xorshiro_state[1] = (uint32_t)(state >> 32);
}

static inline uint32_t rotl (const uint32_t x, int k)
{
	return (x << k) | (x >> (32 - k));
}

int32_t COM_Rand (void)
{
	const uint32_t s0 = xorshiro_state[0];
	uint32_t	   s1 = xorshiro_state[1];
	const uint32_t result = rotl (s0 * 0x9E3779BB, 5) * 5;
	s1 ^= s0;
	xorshiro_state[0] = rotl (s0, 26) ^ s1 ^ (s1 << 9);
	xorshiro_state[1] = rotl (s1, 13);
	return result;
}

/* ---------------------------------------------------------------------------
 * Shared thread-local fs state (common.c keeps these; both the c_ref fs and
 * the Rust shims funnel through the same variables / accessors)
 */

THREAD_LOCAL qfileofs_t com_filesize;
THREAD_LOCAL int		file_from_pak;

qfileofs_t COM_ThreadFileSize (void)
{
	return com_filesize;
}

int COM_ThreadFileFromPak (void)
{
	return file_from_pak;
}

void COM_SetThreadFileSize (qfilesize_t size)
{
	com_filesize = size;
}

void COM_SetThreadFileFromPak (int from_pak)
{
	file_from_pak = from_pak;
}

/* ---------------------------------------------------------------------------
 * stdio helpers (Sys_ftell / Sys_fseek / Sys_filelength / Sys_fopen)
 */

qfileofs_t Sys_ftell (FILE *file)
{
#ifdef _WIN32
	return _ftelli64 (file);
#else
	return ftello (file);
#endif
}

int Sys_fseek (FILE *file, qfileofs_t ofs, int origin)
{
#ifdef _WIN32
	return _fseeki64 (file, ofs, origin);
#else
	return fseeko (file, ofs, origin);
#endif
}

qfilesize_t Sys_filelength (FILE *f)
{
	qfileofs_t pos, end;

	pos = Sys_ftell (f);
	Sys_fseek (f, 0, SEEK_END);
	end = Sys_ftell (f);
	Sys_fseek (f, pos, SEEK_SET);

	return end;
}

FILE *Sys_fopen (const char *path, const char *mode)
{
	return fopen (path, mode);
}

void Sys_mkdir (const char *path)
{
#ifdef _WIN32
	_mkdir (path);
#else
	mkdir (path, 0755);
#endif
}

int Sys_FileType (const char *path)
{
#ifdef _WIN32
	struct _stat64 st;
	if (_stat64 (path, &st) != 0)
		return 0; /* FS_ENT_NONE */
	if (st.st_mode & _S_IFDIR)
		return 1 << 1; /* FS_ENT_DIRECTORY */
	return 1 << 0;	   /* FS_ENT_FILE */
#else
	struct stat st;
	if (stat (path, &st) != 0)
		return 0;
	if (S_ISDIR (st.st_mode))
		return 1 << 1;
	if (S_ISREG (st.st_mode))
		return 1 << 0;
	return 0;
#endif
}

/* ---------------------------------------------------------------------------
 * Sys_File* handle layer: semantics copied from Quake/sys_sdl.c (file- and
 * memory-backed handles, duplicate-by-reopen, EOF bookkeeping), minus the
 * mutex — the differential tests serialize all fs access.
 */

typedef struct ctest_handle_s
{
	bool		free;
	char	   *file_path;
	FILE	   *file;
	const byte *memory;
	qfileofs_t	pos;
	qfilesize_t size;
	bool		eof_condition;
} ctest_handle_t;

#define CTEST_MAX_HANDLES 256
static ctest_handle_t ctest_handles[CTEST_MAX_HANDLES];
static int			  ctest_handles_initialized;

static int allocHandle (void)
{
	int i;
	if (!ctest_handles_initialized)
	{
		for (i = 1; i < CTEST_MAX_HANDLES; i++)
			ctest_handles[i].free = true;
		ctest_handles_initialized = 1;
	}
	/* index 0 stays invalid by design, like the engine */
	for (i = 1; i < CTEST_MAX_HANDLES; i++)
	{
		if (ctest_handles[i].free)
		{
			memset (&ctest_handles[i], 0, sizeof (ctest_handle_t));
			return i;
		}
	}
	Sys_Error ("out of handles");
	return -1;
}

int ctest_open_handle_count (void)
{
	int i, n = 0;
	if (!ctest_handles_initialized)
		return 0;
	for (i = 1; i < CTEST_MAX_HANDLES; i++)
		if (!ctest_handles[i].free)
			n++;
	return n;
}

qfilesize_t Sys_FileOpenRead (const char *path, int *hndl)
{
	FILE	   *f;
	int			i;
	qfilesize_t retval;

	i = allocHandle ();
	f = Sys_fopen (path, "rb");

	if (!f)
	{
		ctest_handles[i].free = true;
		*hndl = -1;
		retval = -1;
	}
	else
	{
		ctest_handles[i].memory = NULL;
		ctest_handles[i].file_path = (char *)Mem_Alloc (strlen (path) + 1);
		memcpy (ctest_handles[i].file_path, path, strlen (path) + 1);
		ctest_handles[i].file = f;
		ctest_handles[i].pos = 0;
		ctest_handles[i].eof_condition = false;
		*hndl = i;
		retval = Sys_filelength (f);
		ctest_handles[i].size = retval;
	}

	return retval;
}

void Sys_MemFileOpenRead (const byte *memory, qfilesize_t size, int *hndl)
{
	int i = allocHandle ();

	ctest_handles[i].file = NULL;
	ctest_handles[i].file_path = NULL;
	ctest_handles[i].memory = memory;
	ctest_handles[i].size = size;
	ctest_handles[i].pos = 0;
	ctest_handles[i].eof_condition = false;
	*hndl = i;
}

int Sys_DuplicateHandle (int handle)
{
	FILE *new_file = NULL;

	if (ctest_handles[handle].file)
	{
		new_file = Sys_fopen (ctest_handles[handle].file_path, "rb");
		if (!new_file)
			return -1;
	}

	int new_handle = allocHandle ();
	ctest_handles[new_handle] = ctest_handles[handle];

	if (ctest_handles[handle].file)
	{
		ctest_handles[new_handle].file = new_file;
		ctest_handles[new_handle].file_path = (char *)Mem_Alloc (strlen (ctest_handles[handle].file_path) + 1);
		memcpy (ctest_handles[new_handle].file_path, ctest_handles[handle].file_path, strlen (ctest_handles[handle].file_path) + 1);
	}

	Sys_FileSeek (new_handle, ctest_handles[handle].pos);
	return new_handle;
}

int Sys_FileOpenWrite (const char *path)
{
	FILE *f;
	int	  i;

	i = allocHandle ();
	f = Sys_fopen (path, "wb");
	if (!f)
	{
		ctest_handles[i].free = true;
		return -1;
	}

	ctest_handles[i].file = f;
	ctest_handles[i].file_path = (char *)Mem_Alloc (strlen (path) + 1);
	memcpy (ctest_handles[i].file_path, path, strlen (path) + 1);
	ctest_handles[i].size = Sys_filelength (f);
	ctest_handles[i].pos = ctest_handles[i].size;
	ctest_handles[i].eof_condition = false;
	ctest_handles[i].memory = NULL;
	return i;
}

void Sys_FileClose (int handle)
{
	if (ctest_handles[handle].file)
	{
		fclose (ctest_handles[handle].file);
		Mem_Free (ctest_handles[handle].file_path);
	}
	ctest_handles[handle].free = true;
}

int Sys_FileSeek (int handle, qfileofs_t position)
{
	if (position >= 0)
	{
		if (ctest_handles[handle].file)
			Sys_fseek (ctest_handles[handle].file, position, SEEK_SET);
		ctest_handles[handle].pos = position;
		return 0;
	}
	return 1;
}

int Sys_FileRead (int handle, void *dest, int count)
{
	if (count <= 0)
		return 0;

	ctest_handles[handle].eof_condition =
		ctest_handles[handle].eof_condition || ((ctest_handles[handle].size - ctest_handles[handle].pos) <= 0) ? true : false;

	if (ctest_handles[handle].eof_condition)
		return 0;

	qfilesize_t computed_read_count = q_min ((qfilesize_t)count, ctest_handles[handle].size - ctest_handles[handle].pos);
	computed_read_count = q_max (0, computed_read_count);

	if (ctest_handles[handle].file)
	{
		qfilesize_t fread_count = fread (dest, 1, count, ctest_handles[handle].file);
		ctest_handles[handle].pos += fread_count;
		ctest_handles[handle].eof_condition = feof (ctest_handles[handle].file);
		return fread_count;
	}
	else
	{
		memcpy (dest, ctest_handles[handle].memory + ctest_handles[handle].pos, computed_read_count);
		ctest_handles[handle].pos += computed_read_count;
		ctest_handles[handle].eof_condition = (computed_read_count < count);
		return computed_read_count;
	}
}

qfileofs_t Sys_FilePos (int handle)
{
	return ctest_handles[handle].pos;
}

/* Sys_fgetc copied verbatim from Quake/sys_sdl.c (image_decode.c streams
 * PCX RLE data through it) */
int Sys_fgetc (int handle)
{
	if (ctest_handles[handle].eof_condition)
		return EOF;

	int next_byte_read = 0;

	if (Sys_FileRead (handle, (void *)&next_byte_read, 1) != 1)
	{
		assert (ctest_handles[handle].eof_condition);
		return EOF;
	}

	return next_byte_read;
}

int Sys_FileWrite (int handle, const void *data, int count)
{
	const int effective_nb_write = fwrite (data, 1, count, ctest_handles[handle].file);
	ctest_handles[handle].pos += effective_nb_write;
	ctest_handles[handle].size += effective_nb_write;
	return effective_nb_write;
}

/* ---------------------------------------------------------------------------
 * Directory enumeration (EGS *.item scan): deterministic sorted order so both
 * sides observe the same sequence regardless of readdir order.
 */

typedef struct ctest_findfile_s
{
	findfile_t base; /* must stay first: callers use findfile_t* */
	char	 **names;
	int		   count;
	int		   index;
	char	   dir[1024];
} ctest_findfile_t;

static int ctest_name_cmp (const void *a, const void *b)
{
	return strcmp (*(const char *const *)a, *(const char *const *)b);
}

/* Appends one directory entry, applying the extension filter. Shared by both
 * enumeration backends so the observable name set is identical. */
static void ctest_find_add (ctest_findfile_t *ff, const char *name, const char *ext, const char *suffix)
{
	size_t len = strlen (name);
	if (!strcmp (name, ".") || !strcmp (name, ".."))
		return;
	if (strcmp (ext, "*") != 0)
	{
		size_t slen = strlen (suffix);
		if (len < slen || strcmp (name + len - slen, suffix) != 0)
			return;
	}
	ff->names = (char **)Mem_Realloc (ff->names, sizeof (char *) * (ff->count + 1));
	ff->names[ff->count] = (char *)Mem_Alloc (len + 1);
	memcpy (ff->names[ff->count], name, len + 1);
	ff->count++;
}

findfile_t *Sys_FindFirst (const char *dir, const char *ext)
{
	ctest_findfile_t *ff;
	char			  suffix[64];

	if (!ext)
		ext = "*";
	else if (*ext == '.')
		++ext;
	snprintf (suffix, sizeof (suffix), ".%s", ext);

	ff = (ctest_findfile_t *)Mem_Alloc (sizeof (ctest_findfile_t));
	snprintf (ff->dir, sizeof (ff->dir), "%s", dir);
	ff->names = NULL;
	ff->count = 0;

#ifdef _WIN32
	{
		struct _finddata_t fd;
		char			   pattern[1024];
		intptr_t		   h;

		snprintf (pattern, sizeof (pattern), "%s/*", dir);
		h = _findfirst (pattern, &fd);
		if (h != -1)
		{
			do
			{
				ctest_find_add (ff, fd.name, ext, suffix);
			} while (_findnext (h, &fd) == 0);
			_findclose (h);
		}
	}
#else
	{
		DIR			  *d = opendir (dir);
		struct dirent *e;

		if (d)
		{
			while ((e = readdir (d)) != NULL)
				ctest_find_add (ff, e->d_name, ext, suffix);
			closedir (d);
		}
	}
#endif

	if (!ff->count)
	{
		Mem_Free (ff->names);
		Mem_Free (ff);
		return NULL;
	}
	qsort (ff->names, ff->count, sizeof (char *), ctest_name_cmp);
	ff->index = -1;
	return Sys_FindNext (&ff->base);
}

findfile_t *Sys_FindNext (findfile_t *find)
{
	ctest_findfile_t *ff = (ctest_findfile_t *)find;
	char			  full[2048];

	if (++ff->index >= ff->count)
	{
		Sys_FindClose (find);
		return NULL;
	}
	snprintf (ff->base.name, sizeof (ff->base.name), "%s", ff->names[ff->index]);
	snprintf (full, sizeof (full), "%s/%s", ff->dir, ff->names[ff->index]);
	ff->base.attribs = (Sys_FileType (full) == (1 << 1)) ? FA_DIRECTORY : 0;
	return find;
}

void Sys_FindClose (findfile_t *find)
{
	if (find)
	{
		ctest_findfile_t *ff = (ctest_findfile_t *)find;
		int				  i;
		for (i = 0; i < ff->count; i++)
			Mem_Free (ff->names[i]);
		Mem_Free (ff->names);
		Mem_Free (ff);
	}
}

/* ---------------------------------------------------------------------------
 * host_parms / userdir seam. Pointer identity matters: the fs compares
 * userdir != basedir as pointers, so ctest_set_host_dirs with a NULL userdir
 * aliases the two.
 */

static char			ctest_host_basedir[1024] = ".";
static char			ctest_host_userdir[1024];
static quakeparms_t ctest_parms = {ctest_host_basedir, ctest_host_basedir, 0, NULL, 0};
quakeparms_t	   *host_parms = &ctest_parms;

void ctest_set_host_dirs (const char *basedir, const char *userdir)
{
	snprintf (ctest_host_basedir, sizeof (ctest_host_basedir), "%s", basedir);
	ctest_parms.basedir = ctest_host_basedir;
	if (userdir)
	{
		snprintf (ctest_host_userdir, sizeof (ctest_host_userdir), "%s", userdir);
		ctest_parms.userdir = ctest_host_userdir;
	}
	else
	{
		ctest_parms.userdir = ctest_host_basedir;
	}
}

const char *COM_HostBasedir (void)
{
	return host_parms->basedir;
}

const char *COM_HostUserdir (void)
{
	return host_parms->userdir;
}

void COM_SetHostUserdir (const char *dir)
{
	host_parms->userdir = dir;
}

/* ---------------------------------------------------------------------------
 * Host globals / cvars shared by both sides
 */

qboolean isDedicated = false;
qboolean multiuser = false;
qboolean harness_active = true; /* hermetic: pref files land in com_gamedir */

cvar_t registered = {"registered", "1", CVAR_ROM, 1.0f, NULL, NULL, NULL, NULL};
cvar_t cmdline = {"cmdline", "", CVAR_ROM, 0.0f, NULL, NULL, NULL, NULL};
cvar_t developer = {"developer", "0", CVAR_NONE, 0.0f, NULL, NULL, NULL, NULL};

/* pref dir for the non-harness COM_FOpenPrefFile / COM_SetUserPrefDir path */
static char ctest_pref_path[1024];

void ctest_set_pref_path (const char *path)
{
	snprintf (ctest_pref_path, sizeof (ctest_pref_path), "%s", path ? path : "");
}

char *Sys_GetPrefPath (const char *org, const char *app)
{
	(void)org;
	(void)app;
	if (!ctest_pref_path[0])
		return NULL;
	{
		size_t len = strlen (ctest_pref_path) + 1;
		char  *ret = (char *)Mem_Alloc (len);
		memcpy (ret, ctest_pref_path, len);
		return ret;
	}
}

/* ---------------------------------------------------------------------------
 * Store discovery seams: test-controllable static paths, default not-found
 */

static char ctest_steam_dir[1024];
static char ctest_gog_dir[1024];
static char ctest_gog_enhanced_dir[1024];
static char ctest_egs_manifest_dir[1024];
static char ctest_nightdive_dir[1024];
static char ctest_egs_launcher_data[8192];
static int	ctest_egs_launcher_data_set;

void ctest_set_steam_dir (const char *path)
{
	snprintf (ctest_steam_dir, sizeof (ctest_steam_dir), "%s", path ? path : "");
}

void ctest_set_gog_dir (const char *path)
{
	snprintf (ctest_gog_dir, sizeof (ctest_gog_dir), "%s", path ? path : "");
}

void ctest_set_gog_enhanced_dir (const char *path)
{
	snprintf (ctest_gog_enhanced_dir, sizeof (ctest_gog_enhanced_dir), "%s", path ? path : "");
}

void ctest_set_egs_manifest_dir (const char *path)
{
	snprintf (ctest_egs_manifest_dir, sizeof (ctest_egs_manifest_dir), "%s", path ? path : "");
}

void ctest_set_nightdive_dir (const char *path)
{
	snprintf (ctest_nightdive_dir, sizeof (ctest_nightdive_dir), "%s", path ? path : "");
}

void ctest_set_egs_launcher_data (const char *json)
{
	if (json)
	{
		snprintf (ctest_egs_launcher_data, sizeof (ctest_egs_launcher_data), "%s", json);
		ctest_egs_launcher_data_set = 1;
	}
	else
	{
		ctest_egs_launcher_data[0] = 0;
		ctest_egs_launcher_data_set = 0;
	}
}

static qboolean ctest_get_dir (const char *src, char *path, size_t pathsize)
{
	if (!src[0])
		return false;
	snprintf (path, pathsize, "%s", src);
	return true;
}

qboolean Sys_GetSteamDir (char *path, size_t pathsize)
{
	return ctest_get_dir (ctest_steam_dir, path, pathsize);
}

qboolean Sys_GetGOGQuakeDir (char *path, size_t pathsize)
{
	return ctest_get_dir (ctest_gog_dir, path, pathsize);
}

qboolean Sys_GetGOGQuakeEnhancedDir (char *path, size_t pathsize)
{
	return ctest_get_dir (ctest_gog_enhanced_dir, path, pathsize);
}

qboolean Sys_GetEGSManifestDir (char *path, size_t pathsize)
{
	return ctest_get_dir (ctest_egs_manifest_dir, path, pathsize);
}

qboolean Sys_GetNightdiveUserDir (char *path, size_t pathsize, const char *steamlibrary)
{
	(void)steamlibrary;
	return ctest_get_dir (ctest_nightdive_dir, path, pathsize);
}

const char *Sys_GetEGSLauncherData (void)
{
	size_t len;
	char  *ret;
	if (!ctest_egs_launcher_data_set)
		return NULL;
	len = strlen (ctest_egs_launcher_data) + 1;
	ret = (char *)Mem_Alloc (len);
	memcpy (ret, ctest_egs_launcher_data, len);
	return ret;
}

/* Steam API runtime / flavor chooser (steam_api.c / sys_sdl.c seams) */
qboolean Steam_Init (const steamgame_t *game)
{
	Con_Printf ("ctest Steam_Init appid=%d\n", game->appid);
	return false;
}

quakeflavor_t ChooseQuakeFlavor (void)
{
	Con_Printf ("ctest ChooseQuakeFlavor\n");
	return QUAKE_FLAVOR_REMASTERED;
}

/* SDL3-only dialogs: unreachable in this build (USE_SDL3 undefined) */
void Sys_MessageBoxWarning (const char *title, const char *message)
{
	(void)title;
	(void)message;
	abort ();
}

void Sys_QuitNoShutdown (void)
{
	abort ();
}

/* ---------------------------------------------------------------------------
 * Cvar / Cmd registration capture
 */

void Cvar_RegisterVariable (cvar_t *variable)
{
	Con_Printf ("ctest Cvar_RegisterVariable %s\n", variable->name);
}

struct cmd_function_s *Cmd_AddCommand2 (const char *cmd_name, xcommand_t function, cmd_source_t srctype, qboolean qcinterceptable)
{
	(void)function;
	Con_Printf ("ctest Cmd_AddCommand %s src=%d qc=%d\n", cmd_name, (int)srctype, (int)qcinterceptable);
	return NULL;
}

/* Rust migration seams that stay in common.c in the real engine */
void COM_Game_f (void) {}

void COM_CheckRegistered (void)
{
	Con_Printf ("ctest COM_CheckRegistered\n");
}

/* ---------------------------------------------------------------------------
 * COM_FileGetExtension / COM_FileBase / COM_AddExtension / q_strdup copied
 * verbatim from Quake/common.c (pure helpers)
 */

const char *COM_FileGetExtension (const char *in)
{
	const char *src;
	size_t		len;

	len = strlen (in);
	if (len < 2)
		return "";

	src = in + len - 1;
	while (src != in && src[-1] != '.')
		src--;
	if (src == in || strchr (src, '/') != NULL || strchr (src, '\\') != NULL)
		return "";

	return src;
}

void COM_FileBase (const char *in, char *out, size_t outsize)
{
	const char *dot, *slash, *s;

	s = in;
	slash = in;
	dot = NULL;
	while (*s)
	{
		if (*s == '/' || *s == '\\')
			slash = s + 1;
		if (*s == '.')
			dot = s;
		s++;
	}
	if (dot == NULL)
		dot = s;

	if (dot - slash < 2)
		q_strlcpy (out, "?model?", outsize);
	else
	{
		size_t len = dot - slash;
		if (len >= outsize)
			len = outsize - 1;
		memcpy (out, slash, len);
		out[len] = '\0';
	}
}

void COM_AddExtension (char *path, const char *extension, size_t len)
{
	if (strcmp (COM_FileGetExtension (path), extension + 1) != 0)
		q_strlcat (path, extension, len);
}

char *q_strdup (const char *str)
{
	size_t len = strlen (str) + 1;
	char  *newstr = (char *)Mem_Alloc (len);
	memcpy (newstr, str, len);
	return newstr;
}

int q_strcasecmp (const char *s1, const char *s2)
{
	const char *p1 = s1;
	const char *p2 = s2;
	char		c1, c2;

	if (p1 == p2)
		return 0;

	do
	{
		c1 = *p1++;
		c2 = *p2++;
		if (c1 >= 'A' && c1 <= 'Z')
			c1 += 'a' - 'A';
		if (c2 >= 'A' && c2 <= 'Z')
			c2 += 'a' - 'A';
		if (c1 == '\0')
			break;
	} while (c1 == c2);

	return (int)(c1 - c2);
}

/* q_vsnprintf / q_snprintf copied verbatim from Quake/common.c */
int q_vsnprintf (char *str, size_t size, const char *format, va_list args)
{
	int ret;

	ret = vsnprintf (str, size, format, args);

	if (ret < 0)
		ret = (int)size;
	if (size == 0) /* no buffer */
		return ret;
	if ((size_t)ret >= size)
		str[size - 1] = '\0';

	return ret;
}

int q_snprintf (char *str, size_t size, const char *format, ...)
{
	int		ret;
	va_list argptr;

	va_start (argptr, format);
	ret = q_vsnprintf (str, size, format, argptr);
	va_end (argptr);

	return ret;
}

/* va: single static buffer is enough for the tests (each use is consumed
 * before the next call on both sides) */
char *va (const char *format, ...)
{
	static char va_buf[1024];
	va_list		argptr;
	va_start (argptr, format);
	vsnprintf (va_buf, sizeof (va_buf), format, argptr);
	va_end (argptr);
	return va_buf;
}

/* LittleLong is a function pointer in the engine (byte-order dispatch); all
 * targets are little-endian */
static int ctest_LongNoSwap (int l)
{
	return l;
}
int (*LittleLong) (int l) = ctest_LongNoSwap;
static short ctest_ShortNoSwap (short l)
{
	return l;
}
short (*LittleShort) (short l) = ctest_ShortNoSwap;

/* ---------------------------------------------------------------------------
 * Cvar_Set capture: both the C reference and the Rust shims funnel through
 * this stub, so tests snapshot and compare the exact call sequences
 */
#define CTEST_CVAR_LOG_MAX 128
static char ctest_cvar_log[CTEST_CVAR_LOG_MAX][2][256];
static int	ctest_cvar_log_count;

void Cvar_Set (const char *var_name, const char *value)
{
	if (ctest_cvar_log_count < CTEST_CVAR_LOG_MAX)
	{
		snprintf (ctest_cvar_log[ctest_cvar_log_count][0], 256, "%s", var_name);
		snprintf (ctest_cvar_log[ctest_cvar_log_count][1], 256, "%s", value);
	}
	ctest_cvar_log_count++;
}

void ctest_clear_cvar_log (void)
{
	ctest_cvar_log_count = 0;
}

int ctest_cvar_log_len (void)
{
	return ctest_cvar_log_count;
}

const char *ctest_cvar_log_get (int i, int which)
{
	return ctest_cvar_log[i][which];
}

/* ---------------------------------------------------------------------------
 * command line for COM_CheckParm / COM_CheckParmNext (verbatim from common.c)
 */
#define CTEST_MAX_ARGS 64
static char *ctest_argv[CTEST_MAX_ARGS];
int			 com_argc;
char	   **com_argv = ctest_argv;

void ctest_set_args (int argc, char **argv)
{
	com_argc = argc < CTEST_MAX_ARGS ? argc : CTEST_MAX_ARGS;
	for (int i = 0; i < com_argc; i++)
		ctest_argv[i] = argv[i];
}

int COM_CheckParmNext (int last, const char *parm)
{
	int i;

	for (i = last + 1; i < com_argc; i++)
	{
		if (!com_argv[i])
			continue; // NEXTSTEP sometimes clears appkit vars.
		if (!strcmp (parm, com_argv[i]))
			return i;
	}

	return 0;
}

int COM_CheckParm (const char *parm)
{
	return COM_CheckParmNext (0, parm);
}

/* ---------------------------------------------------------------------------
 * COM_Parse: copied verbatim from Quake/common.c (COM_ParseEx + COM_Parse +
 * the com_token TLS and its accessor) so token parsing matches the engine for
 * both sides of COM_Effectinfo_Enumerate.
 */

THREAD_LOCAL char com_token[COM_PARSE_MAX_TOKEN_SIZE];

const char *COM_ParseEx (const char *data, cpe_mode mode)
{
	int c;
	int len;

	len = 0;
	com_token[0] = 0;

	if (!data)
		return NULL;

// skip whitespace
skipwhite:
	while ((c = *data) <= ' ')
	{
		if (c == 0)
			return NULL; // end of file
		data++;
	}

	// skip // comments
	if (c == '/' && data[1] == '/')
	{
		while (*data && *data != '\n')
			data++;
		goto skipwhite;
	}

	// skip /*..*/ comments
	if (c == '/' && data[1] == '*')
	{
		data += 2;
		while (*data && !(*data == '*' && data[1] == '/'))
			data++;
		if (*data)
			data += 2;
		goto skipwhite;
	}

	// handle quoted strings specially
	if (c == '\"')
	{
		data++;
		while (1)
		{
			if ((c = *data) != 0)
				++data;
			if (c == '\"' || !c)
			{
				com_token[len] = 0;
				return data;
			}
			if (len < countof (com_token) - 1)
				com_token[len++] = c;
			else if (mode == CPE_NOTRUNC)
				return NULL;
		}
	}

	// parse single characters
	if (c == '{' || c == '}' || c == '(' || c == ')' || c == '\'' || c == ':')
	{
		if (len < countof (com_token) - 1)
			com_token[len++] = c;
		else if (mode == CPE_NOTRUNC)
			return NULL;
		com_token[len] = 0;
		return data + 1;
	}

	// parse a regular word
	do
	{
		if (len < countof (com_token) - 1)
			com_token[len++] = c;
		else if (mode == CPE_NOTRUNC)
			return NULL;
		data++;
		c = *data;
		/* commented out the check for ':' so that ip:port works */
		if (c == '{' || c == '}' || c == '(' || c == ')' || c == '\'' /* || c == ':' */)
			break;
	} while (c > 32);

	com_token[len] = 0;
	return data;
}

const char *COM_Parse (const char *data)
{
	return COM_ParseEx (data, CPE_NOTRUNC);
}

const char *COM_ThreadToken (void)
{
	return com_token;
}

/* ---------------------------------------------------------------------------
 * Phase 3 (model_parse.c) stubs: the seams UNDER the brush loaders. Both the
 * c_ref side and the Rust exports resolve to these same definitions, so the
 * observable behavior is comparable.
 */

static float ctest_FloatNoSwap (float f)
{
	return f;
}
float (*LittleFloat) (float f) = ctest_FloatNoSwap;

CON_STUB (Con_DWarning, "[dwarn]")

/* Host_Error: separate trap from Sys_Error so tests can differentially
 * compare WHICH error path fired. Unarmed Host_Error aborts. */
static jmp_buf ctest_host_error_jmp;
static int	   ctest_host_error_armed;
static char	   ctest_host_error_msg[2048];

FUNC_NORETURN void Host_Error (const char *error, ...)
{
	va_list ap;
	va_start (ap, error);
	vsnprintf (ctest_host_error_msg, sizeof (ctest_host_error_msg), error, ap);
	va_end (ap);
	if (ctest_host_error_armed)
	{
		ctest_host_error_armed = 0;
		longjmp (ctest_host_error_jmp, 1);
	}
	fprintf (stderr, "Host_Error: %s\n", ctest_host_error_msg);
	abort ();
}

/* Runs fn(arg) with the Host_Error trap armed. Returns 1 if Host_Error fired
 * (message in ctest_host_error_message()), 0 if fn returned normally. */
int ctest_try_host (void (*fn) (void *), void *arg)
{
	if (setjmp (ctest_host_error_jmp))
		return 1;
	ctest_host_error_armed = 1;
	ctest_host_error_msg[0] = 0;
	fn (arg);
	ctest_host_error_armed = 0;
	return 0;
}

const char *ctest_host_error_message (void)
{
	return ctest_host_error_msg;
}

/* sv.modelname is the only server state model_parse.c reads
 * (Mod_SetupSubmodels submodel-0 check) */
ctest_server_stub_t sv;

void ctest_set_sv_modelname (const char *name)
{
	snprintf (sv.modelname, sizeof (sv.modelname), "%s", name ? name : "");
}

cvar_t external_ents = {"external_ents", "1", CVAR_ARCHIVE};

void ctest_set_external_ents (float value)
{
	external_ents.value = value;
}

/* Mod_FindName: static pool keyed by name, so submodel chaining
 * (Mod_SetupSubmodels' "*1".."*n" clones) gets stable identical pointers on
 * both sides without the real model cache. */
/* sized for real maps: the M7 corpus gate loads shipped BSPs whose
 * submodel counts (one Mod_FindName clone each) far exceed the synthetic
 * fixtures' (id1 tops out around 256) */
#define CTEST_MOD_POOL_MAX 1024
static qmodel_t ctest_mod_pool[CTEST_MOD_POOL_MAX];
static int		ctest_mod_pool_count;

qmodel_t *Mod_FindName (const char *name)
{
	int i;
	for (i = 0; i < ctest_mod_pool_count; i++)
		if (!strcmp (ctest_mod_pool[i].name, name))
			return &ctest_mod_pool[i];
	assert (ctest_mod_pool_count < CTEST_MOD_POOL_MAX);
	memset (&ctest_mod_pool[ctest_mod_pool_count], 0, sizeof (qmodel_t));
	snprintf (ctest_mod_pool[ctest_mod_pool_count].name, sizeof (ctest_mod_pool[0].name), "%s", name);
	ctest_mod_pool[ctest_mod_pool_count].needload = true;
	return &ctest_mod_pool[ctest_mod_pool_count++];
}

void ctest_mod_pool_reset (void)
{
	ctest_mod_pool_count = 0;
}

qmodel_t *ctest_mod_pool_get (int i)
{
	return (i >= 0 && i < ctest_mod_pool_count) ? &ctest_mod_pool[i] : NULL;
}

int ctest_mod_pool_len (void)
{
	return ctest_mod_pool_count;
}

/* Mod_LoadWadTexture: wad fallback lookup lives in gl_model.c (GPU-adjacent,
 * stays C); the ctest fixtures don't mount texture wads, so a NULL return
 * exercises the missing-texture fallback identically on both sides. */
texture_t *Mod_LoadWadTexture (qmodel_t *mod, wad_t *wads, const char *name)
{
	(void)mod;
	(void)wads;
	(void)name;
	return NULL;
}

/* Phase 3 M4: the alias/sprite loaders' only side effects outside the
 * Mem_Alloc'ed structs are these two calls, so both sides share one recorder
 * and the differential compares the argument streams. */
#define CTEST_MODELSTUB_MAX 64

typedef struct
{
	char	 name[64];
	int		 width;
	int		 height;
	int		 format;
	int64_t	 data_ofs; /* byte offset from the base passed to the reset */
	char	 source_file[64];
	uint64_t source_offset;
	uint32_t flags;
} ctest_teximage_call_t;

typedef struct
{
	int32_t numskins;
	int64_t pskintype_ofs;
} ctest_allskins_call_t;

static const byte		   *ctest_modelstub_base;
static ctest_teximage_call_t ctest_teximage_log[CTEST_MODELSTUB_MAX];
static int32_t				 ctest_teximage_n;
static ctest_allskins_call_t ctest_allskins_log[CTEST_MODELSTUB_MAX];
static int32_t				 ctest_allskins_n;
static int					 ctest_allskins_advance;

void ctest_modelstub_reset (const byte *mod_base)
{
	ctest_modelstub_base = mod_base;
	ctest_teximage_n = 0;
	ctest_allskins_n = 0;
	ctest_allskins_advance = 0;
	memset (ctest_teximage_log, 0, sizeof (ctest_teximage_log));
	memset (ctest_allskins_log, 0, sizeof (ctest_allskins_log));
}

/* Phase 3 M7: the synthetic differential fixtures all use numskins == 0, but
 * the formats-corpus gate feeds real skinned .mdl files, whose parse depends
 * on Mod_LoadAllSkins returning the cursor advanced past the skin data. When
 * enabled, the stub replicates the frozen gl_model.c cursor walk (and its
 * numskins Sys_Error) without loading anything. Reset clears the flag. */
void ctest_allskins_set_advance (int on)
{
	ctest_allskins_advance = on;
}

int32_t ctest_teximage_count (void)
{
	return ctest_teximage_n;
}

const ctest_teximage_call_t *ctest_teximage_calls (void)
{
	return ctest_teximage_log;
}

int32_t ctest_allskins_count (void)
{
	return ctest_allskins_n;
}

const ctest_allskins_call_t *ctest_allskins_calls (void)
{
	return ctest_allskins_log;
}

static int64_t ctest_modelstub_ofs (const void *p)
{
	if (!p || !ctest_modelstub_base)
		return INT64_MIN;
	return (int64_t)((const byte *)p - ctest_modelstub_base);
}

void *Mod_LoadAllSkins (aliashdr_t *pheader, qmodel_t *mod, byte *mod_base, int numskins, byte *pskintype)
{
	(void)pheader;
	(void)mod;
	(void)mod_base;
	if (ctest_allskins_n < CTEST_MODELSTUB_MAX)
	{
		ctest_allskins_log[ctest_allskins_n].numskins = numskins;
		ctest_allskins_log[ctest_allskins_n].pskintype_ofs = ctest_modelstub_ofs (pskintype);
	}
	++ctest_allskins_n;
	if (ctest_allskins_advance)
	{
		/* the frozen gl_model.c cursor walk, minus the texture loads */
		if (numskins < 1 || numskins > 32 /* MAX_SKINS */)
			Sys_Error ("Mod_LoadAliasModel: Invalid # of skins: %d", numskins);
		int size = pheader->skinwidth * pheader->skinheight;
		for (int i = 0; i < numskins; i++)
		{
			int32_t type;
			memcpy (&type, pskintype, sizeof (type));
			if (type == 0 /* ALIAS_SKIN_SINGLE */)
			{
				pskintype += sizeof (int32_t) + size;
			}
			else
			{
				int32_t groupskins;
				memcpy (&groupskins, pskintype + sizeof (int32_t), sizeof (groupskins));
				pskintype += 2 * sizeof (int32_t) + groupskins * (int32_t)sizeof (int32_t) + groupskins * size;
			}
		}
	}
	return pskintype;
}

gltexture_t *TexMgr_LoadImage (
	qmodel_t *owner, const char *name, int width, int height, enum srcformat format, byte *data, const char *source_file, src_offset_t source_offset,
	unsigned flags)
{
	(void)owner;
	if (ctest_teximage_n < CTEST_MODELSTUB_MAX)
	{
		ctest_teximage_call_t *c = &ctest_teximage_log[ctest_teximage_n];
		q_strlcpy (c->name, name ? name : "", sizeof (c->name));
		c->width = width;
		c->height = height;
		c->format = (int)format;
		c->data_ofs = ctest_modelstub_ofs (data);
		q_strlcpy (c->source_file, source_file ? source_file : "", sizeof (c->source_file));
		c->source_offset = (uint64_t)source_offset;
		c->flags = (uint32_t)flags;
	}
	++ctest_teximage_n;
	return NULL;
}

/* Phase 3 M5: the MD3/MD5 loaders' side effects outside their Mem_Alloc'ed
 * aliashdr_t block are the skin-loading callbacks (which stay in gl_model.c)
 * and the mesh upload (gl_mesh.c). The upload is where the *parsed* payload
 * goes -- baked MD5 influences, generated normals, MD3 vertex/texcoord
 * streams -- so the recorder copies the buffers, not just the pointers; that
 * copy is the primary AC6 evidence.
 *
 * Buffer lengths are derived from the aliashdr_t the loader just filled, so
 * both sides record by the same rule. The tests assert the recorded lengths
 * against what the fixture implies, so a mis-derived length fails loudly
 * instead of silently comparing nothing. */
/* MD3's own MAX_SURFACES, so a format-legal MD3 can never outrun the log;
 * the Rust reader asserts rather than clamps (tests/support/mdx_record.rs) */
#define CTEST_UPLOAD_MAX   32
#define CTEST_UPLOAD_BYTES 65536

typedef struct
{
	int32_t numverts, numverts_vbo, numtris, numindexes;
	int32_t numposes, numframes, numjoints, poseverttype, numskins;
	int32_t has_next_surface, has_desc, has_joints;
	int32_t index_bytes, vertex_bytes, desc_bytes, joint_bytes;
	/* FNV-1a over each buffer, always computed: a real model's vertex block
	 * can outgrow the byte copy below, and the hash still compares. */
	uint64_t index_hash, vertex_hash, desc_hash, joint_hash;
	int32_t truncated;
	byte	data[CTEST_UPLOAD_BYTES]; /* indexes | vertexes | desc | joints */
} ctest_upload_call_t;

typedef struct
{
	char	name[64];
	int32_t surf_index;
	int64_t numsurfaces;
	int64_t numskins;
	int32_t kind; /* 0 = MD5 shader name, 1 = MD3 surface name */
} ctest_mdxskin_call_t;

static ctest_upload_call_t	ctest_upload_log[CTEST_UPLOAD_MAX];
static int32_t				ctest_upload_n;
static ctest_mdxskin_call_t ctest_mdxskin_log[CTEST_MODELSTUB_MAX];
static int32_t				ctest_mdxskin_n;
static int32_t				ctest_skindefs_n;
static int32_t				ctest_deletemesh_n;
static int32_t				ctest_mdxskin_result;

void ctest_mdxstub_reset (int32_t skins_result)
{
	ctest_upload_n = 0;
	ctest_mdxskin_n = 0;
	ctest_skindefs_n = 0;
	ctest_deletemesh_n = 0;
	ctest_mdxskin_result = skins_result;
	memset (ctest_upload_log, 0, sizeof (ctest_upload_log));
	memset (ctest_mdxskin_log, 0, sizeof (ctest_mdxskin_log));
}

int32_t ctest_upload_count (void)
{
	return ctest_upload_n;
}

const ctest_upload_call_t *ctest_upload_calls (void)
{
	return ctest_upload_log;
}

int32_t ctest_mdxskin_count (void)
{
	return ctest_mdxskin_n;
}

const ctest_mdxskin_call_t *ctest_mdxskin_calls (void)
{
	return ctest_mdxskin_log;
}

int32_t ctest_skindefs_count (void)
{
	return ctest_skindefs_n;
}

int32_t ctest_deletemesh_count (void)
{
	return ctest_deletemesh_n;
}

static size_t ctest_pose_vertex_size (int poseverttype)
{
	switch (poseverttype)
	{
	case PV_MD5:
		return sizeof (md5vert_t);
	case PV_MD5_8:
		return sizeof (md5vert8_t);
	case PV_QUAKE3:
		return sizeof (md3XyzNormal_t);
	default:
		return sizeof (trivertx_t);
	}
}

static uint64_t ctest_fnv1a (const void *src, size_t n)
{
	const byte *p = (const byte *)src;
	uint64_t	h = 1469598103934665603ull;
	size_t		i;
	for (i = 0; i < n; i++)
	{
		h ^= p[i];
		h *= 1099511628211ull;
	}
	return h;
}

static void ctest_upload_append (
	ctest_upload_call_t *c, size_t *used, const void *src, size_t n, int32_t *out_bytes, uint64_t *out_hash)
{
	*out_bytes = (int32_t)n;
	*out_hash = (src && n) ? ctest_fnv1a (src, n) : 0;
	if (!src || n == 0)
		return;
	if (*used + n > CTEST_UPLOAD_BYTES)
	{
		c->truncated = 1;
		return;
	}
	memcpy (c->data + *used, src, n);
	*used += n;
}

void GLMesh_UploadBuffers (qmodel_t *mod, aliashdr_t *hdr, unsigned short *indexes, byte *vertexes, aliasmesh_t *desc, jointpose_t *joints)
{
	(void)mod;
	if (ctest_upload_n < CTEST_UPLOAD_MAX)
	{
		ctest_upload_call_t *c = &ctest_upload_log[ctest_upload_n];
		size_t				 used = 0;
		size_t				 vsize = ctest_pose_vertex_size (hdr->poseverttype);
		/* MD3 hands over numframes poses of numverts; MD5 hands over the one
		 * skinned pose (numposes == 1 there too, but the vertex stream is not
		 * per-frame). */
		size_t				 vert_records = (hdr->poseverttype == PV_QUAKE3) ? (size_t)hdr->numframes * (size_t)hdr->numverts : (size_t)hdr->numverts;

		c->numverts = hdr->numverts;
		c->numverts_vbo = hdr->numverts_vbo;
		c->numtris = hdr->numtris;
		c->numindexes = hdr->numindexes;
		c->numposes = hdr->numposes;
		c->numframes = hdr->numframes;
		c->numjoints = hdr->numjoints;
		c->poseverttype = (int32_t)hdr->poseverttype;
		c->numskins = hdr->numskins;
		c->has_next_surface = hdr->nextsurface != NULL;
		c->has_desc = desc != NULL;
		c->has_joints = joints != NULL;

		ctest_upload_append (c, &used, indexes, (size_t)hdr->numindexes * sizeof (unsigned short), &c->index_bytes, &c->index_hash);
		ctest_upload_append (c, &used, vertexes, vert_records * vsize, &c->vertex_bytes, &c->vertex_hash);
		ctest_upload_append (
			c, &used, desc, desc ? (size_t)hdr->numverts * sizeof (aliasmesh_t) : 0, &c->desc_bytes, &c->desc_hash);
		ctest_upload_append (
			c, &used, joints, joints ? (size_t)hdr->numjoints * (size_t)hdr->numframes * sizeof (jointpose_t) : 0, &c->joint_bytes,
			&c->joint_hash);
	}
	++ctest_upload_n;
}

void GLMesh_DeleteMeshBuffers (aliashdr_t *mainhdr)
{
	(void)mainhdr;
	++ctest_deletemesh_n;
}

void Mod_LoadMD3SkinDefinitions (qmodel_t *mod, all_surfaces_def_t *surf_defs)
{
	(void)mod;
	(void)surf_defs;
	++ctest_skindefs_n;
}

static int ctest_record_mdxskin (const char *name, int surf_index, size_t numsurfaces, size_t numskins, int kind)
{
	if (ctest_mdxskin_n < CTEST_MODELSTUB_MAX)
	{
		ctest_mdxskin_call_t *c = &ctest_mdxskin_log[ctest_mdxskin_n];
		q_strlcpy (c->name, name ? name : "", sizeof (c->name));
		c->surf_index = surf_index;
		c->numsurfaces = (int64_t)numsurfaces;
		c->numskins = (int64_t)numskins;
		c->kind = kind;
	}
	++ctest_mdxskin_n;
	return ctest_mdxskin_result;
}

int Mod_LoadMD3SurfaceSkins (
	qmodel_t *mod, aliashdr_t *surf, all_surfaces_def_t *surfaces_def, const char *surface_name, int surface_index, size_t numsurfs, size_t numskins)
{
	(void)mod;
	(void)surf;
	(void)surfaces_def;
	return ctest_record_mdxskin (surface_name, surface_index, numsurfs, numskins, 1);
}

size_t Mod_LoadMD5SurfaceSkins (qmodel_t *mod, aliashdr_t *surf, int surf_index, size_t numsurfaces, const char *shader_name)
{
	(void)mod;
	(void)surf;
	return (size_t)ctest_record_mdxskin (shader_name, surf_index, numsurfaces, MAX_SKINS, 0);
}

/* q_strtrim lives in Quake/common.c, which is not in the c_ref build; verbatim
 * copy shared by both sides (the prelude does not rename it). The size_t
 * last_index underflow on an all-whitespace field is the C's, kept as-is. */
char *q_strtrim (char *str)
{
	// trim leading and ending whitespaces:
	char *str_start = (char *)str;

	// trim leading:
	while (q_isspace ((unsigned char)*str))
		str++;

	// trim ending :
	size_t last_index = strlen (str_start) - 1;

	while (last_index >= 0)
	{
		if (q_isspace (str_start[last_index]))
		{
			str_start[last_index] = '\0';
		}
		else
		{
			break;
		}
		last_index--;
	}

	return str;
}

/* COM_SkipPath / COM_StripExtension live in Quake/common.c, which is not in
 * the c_ref build; verbatim copies here are shared by both sides (the
 * prelude does not rename them). */
const char *COM_SkipPath (const char *pathname)
{
	const char *last;

	last = pathname;
	while (*pathname)
	{
		if (*pathname == '/')
			last = pathname + 1;
		pathname++;
	}
	return last;
}

void COM_StripExtension (const char *in, char *out, size_t outsize)
{
	int length;

	if (!*in)
	{
		*out = '\0';
		return;
	}
	if (in != out) /* copy when not in-place editing */
		q_strlcpy (out, in, outsize);
	length = (int)strlen (out) - 1;
	while (length > 0 && out[length] != '.')
	{
		--length;
		if (out[length] == '/' || out[length] == '\\')
			return; /* no extension */
	}
	if (length > 0)
		out[length] = '\0';
}

/* ---------------------------------------------------------------------------
 * Tail of model_parse.c that M3 does not port: the alias/sprite half still
 * references these, and the linker pulls the whole TU in. Only q_strncasecmp
 * is reachable from the brush range (Mod_TextureTypeFromName), so it is a
 * verbatim copy of common.c's; the rest are inert stand-ins.
 */

int q_strncasecmp (const char *s1, const char *s2, size_t n)
{
	const char *p1 = s1;
	const char *p2 = s2;
	char		c1, c2;

	if (p1 == p2 || n == 0)
		return 0;

	do
	{
		c1 = *p1++;
		c2 = *p2++;
		if (c1 >= 'A' && c1 <= 'Z')
			c1 += 'a' - 'A';
		if (c2 >= 'A' && c2 <= 'Z')
			c2 += 'a' - 'A';
		if (c1 == '\0' || c1 != c2)
			break;
	} while (--n > 0);

	return (int)(c1 - c2);
}

void PScript_UpdateModelEffects (qmodel_t *mod)
{
	(void)mod;
}

#ifndef THREAD_LOCAL
#ifdef _MSC_VER
#define THREAD_LOCAL __declspec (thread)
#else
#define THREAD_LOCAL _Thread_local
#endif
#endif

THREAD_LOCAL size_t thread_stack_alloc_size = 0;
size_t				max_thread_stack_alloc_size = 0;

cvar_t r_nolerp_list = {
	"r_nolerp_list",
	"progs/flame.mdl,progs/flame2.mdl,progs/braztall.mdl,progs/brazshrt.mdl,progs/longtrch.mdl,progs/flame_pyre.mdl,progs/v_saw.mdl,progs/"
	"v_xfist.mdl,progs/h2stuff/newfire.mdl",
	CVAR_NONE};

/* gl_model.c's two dummy texture slots: Mod_Init allocates r_notexture_mip /
 * r_notexture_mip2 and Mod_LoadTextures fills the last two entries of
 * mod->textures after Mod_ParseTextures returns. Both halves stay C, so the
 * differential driver replicates the two assignments for either side out of
 * one shared pair (identical contents keep the snapshots comparable). */
static texture_t ctest_notexture_mip;
static texture_t ctest_notexture_mip2;

void ctest_fill_dummy_textures (qmodel_t *mod)
{
	memset (&ctest_notexture_mip, 0, sizeof (ctest_notexture_mip));
	strcpy (ctest_notexture_mip.name, "notexture");
	ctest_notexture_mip.height = ctest_notexture_mip.width = 32;

	memset (&ctest_notexture_mip2, 0, sizeof (ctest_notexture_mip2));
	strcpy (ctest_notexture_mip2.name, "notexture2");
	ctest_notexture_mip2.height = ctest_notexture_mip2.width = 32;

	mod->textures[mod->numtextures - 2] = &ctest_notexture_mip;
	mod->textures[mod->numtextures - 1] = &ctest_notexture_mip2;
}
