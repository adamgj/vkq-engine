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
#include <math.h>
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

/* zero-filled rather than left uninitialized: the brush loaders leave a few
 * msurface_t fields untouched (Phase 3 M3), so a deterministic fill makes the
 * C and Rust sides agree on those bytes instead of comparing two different
 * heaps' garbage. A field the Rust port fails to write still shows up as 0
 * against the C value whenever that value is non-zero.
 * The one field that *required* the fill to be zero rather than any other
 * constant -- msurface_t::styles_bitmap, OR-ed into without initialization --
 * is initialized by Mod_ParseFaces itself since Phase 3 M6 (RA12). */
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
 *
 * One exception (Phase 3 M6): threaded_parse.rs opens, decodes and closes
 * concurrently. Only the `free` flag crosses threads, so it is the only field
 * made atomic, and allocHandle claims a slot with a compare-and-swap rather
 * than a scan-then-take pair — which is the same job the real engine's
 * Sys_FileOpenRead mutex does. Every other field in a slot is touched solely
 * by the thread that owns it, between the claim and the close.
 *
 * The one-time table init below is deliberately *not* synchronized: callers
 * must perform at least one open before spawning threads (threaded_parse.rs
 * does its serial C decodes first), exactly as the engine initializes its
 * handle table on the main thread before any task worker runs.
 */

/* The handle-table free flag is the one stub field that genuinely crosses
 * threads, so it gets real atomics rather than the prelude's stand-in
 * Atomic_StoreUInt32 (which is a plain volatile store -- the prelude cannot
 * include the engine's atomics.h, because that drags in q_stdinc.h -> SDL.h).
 * MSVC is split out because it does not ship <stdatomic.h> reliably under
 * /std:c11; _InterlockedExchange/_InterlockedCompareExchange are intrinsics
 * with full-fence semantics, which is stronger than needed here. */
#ifdef _MSC_VER
#include <intrin.h>
typedef volatile long ctest_flag_t;
#define CTEST_FLAG_STORE(p, v) ((void)_InterlockedExchange ((p), (v)))
#define CTEST_FLAG_LOAD(p)	   (_InterlockedCompareExchange ((p), 0, 0))
/* claim: 1 (free) -> 0 (in use), atomically */
#define CTEST_FLAG_CLAIM(p) (_InterlockedCompareExchange ((p), 0, 1) == 1)
#else
#include <stdatomic.h>
typedef _Atomic unsigned int ctest_flag_t;
#define CTEST_FLAG_STORE(p, v) atomic_store ((p), (v))
#define CTEST_FLAG_LOAD(p)	   atomic_load ((p))
static inline bool ctest_flag_claim (ctest_flag_t *p)
{
	unsigned int expected = 1;
	return atomic_compare_exchange_strong (p, &expected, 0u);
}
#define CTEST_FLAG_CLAIM(p) ctest_flag_claim (p)
#endif

typedef struct ctest_handle_s
{
	ctest_flag_t free; /* written by the closing thread, scanned by allocHandle */
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
			CTEST_FLAG_STORE (&ctest_handles[i].free, 1);
		ctest_handles_initialized = 1;
	}
	/* index 0 stays invalid by design, like the engine */
	for (i = 1; i < CTEST_MAX_HANDLES; i++)
	{
		/* claim atomically: a plain "is it free? then take it" pair would let
		 * two threads pick the same slot, which is why the real engine's
		 * Sys_FileOpenRead holds a mutex here. Doing it with a CAS instead
		 * lets threaded_parse.rs run its opens fully concurrently. */
		if (CTEST_FLAG_CLAIM (&ctest_handles[i].free))
		{
			/* the slot is now owned by this thread alone */
			ctest_handles[i].file_path = NULL;
			ctest_handles[i].file = NULL;
			ctest_handles[i].memory = NULL;
			ctest_handles[i].pos = 0;
			ctest_handles[i].size = 0;
			ctest_handles[i].eof_condition = false;
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
		if (!CTEST_FLAG_LOAD (&ctest_handles[i].free))
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
		CTEST_FLAG_STORE (&ctest_handles[i].free, 1);
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
		CTEST_FLAG_STORE (&ctest_handles[i].free, 1);
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
	CTEST_FLAG_STORE (&ctest_handles[handle].free, 1);
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

/* Sys_feof copied verbatim from Quake/sys_sdl.c (image_stb.c's stb eof
 * callback reads it) */
bool Sys_feof (int handle)
{
	return ctest_handles[handle].eof_condition;
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

/* Phase 7 M2: true once command execution is "live". Gates
 * Cvar_RegisterVariable's dynamic-vs-static name-copy strategy and
 * Cvar_SetQuick's default_string update-in-place behavior; tests toggle it
 * with ctest_set_host_initialized to exercise both paths. */
qboolean host_initialized = false;
void	 ctest_set_host_initialized (qboolean v) { host_initialized = v; }

/* Phase 7 M2: the client the current src_client command is attributed to;
 * only Cmd_ExecuteString's src_client warning path reads it. */
client_t *host_client = NULL;
void	  ctest_set_host_client (client_t *c) { host_client = c; }

/* Phase 7 M2: differential tests exercising Cmd_ExecuteString's src_client
 * path need a `host_client` whose ->name is safe to print (cmd.c's
 * Cmd_ExecuteString dereferences it unconditionally for the "tried to"
 * warning); leaked so the pointer stays valid for the life of the process. */
client_t *ctest_make_client_with_name (const char *name)
{
	client_t *c = (client_t *)calloc (1, sizeof (client_t));
	q_strlcpy (c->name, name, sizeof (c->name));
	return c;
}

cvar_t registered = {"registered", "1", CVAR_ROM, 1.0f, NULL, NULL, NULL, NULL};
cvar_t cmdline = {"cmdline", "", CVAR_ROM, 0.0f, NULL, NULL, NULL, NULL};
cvar_t developer = {"developer", "0", CVAR_NONE, 0.0f, NULL, NULL, NULL, NULL};
/* Phase 7 M2: cvar.c's CVAR_USERINFO replication special-cases these three
 * by address (Cvar_SetQuick's "name"/"color" legacy setinfo hacks). Phase 7 M7
 * (T7.0) made cl_main.c an oracle source, so it defines cl_name/cl_topcolor/
 * cl_bottomcolor (renamed c_ref_*) itself and the stand-ins that used to live
 * here would be duplicate definitions. The `#define cl_name c_ref_cl_name`
 * family in c_ref_prelude.h repoints the address comparisons in
 * ctest_invoke_userinfo_changed below at cl_main.c's objects -- which is what
 * the gate wants, because cvar.c (the oracle side of the same comparison) has
 * always compared against whatever `cl_name` resolved to in its own TU.
 * The real names differ from the old stand-ins: cl_main.c:33-34 register
 * "topcolor"/"bottomcolor", not "_cl_topcolor"/"_cl_bottomcolor".
 */

/* Phase 7 M2: records the argv of whichever side's Cmd_ExecuteString/direct
 * call last ran a probe command, so cvar_cmd_differential.rs can compare the
 * two sides' tokenization without either side needing to expose cmd_argv
 * directly. ctest_probe_xcommand is registered as a c_ref-side xcommand (its
 * Cmd_Argc/Cmd_Argv calls resolve through the renames above to the c_ref
 * registry); the Rust side has no equivalent C source to compile an xcommand
 * from, so its test-side callback fills the same buffer via
 * ctest_probe_set_arg/ctest_probe_set_argc after calling the Rust Cmd_Argc/
 * Cmd_Argv exports itself. */
#define CTEST_PROBE_MAX_ARGS 16
static int	ctest_probe_argc_v;
static char ctest_probe_argv_v[CTEST_PROBE_MAX_ARGS][256];
void		ctest_probe_xcommand (void)
{
	int i, n = Cmd_Argc ();
	if (n > CTEST_PROBE_MAX_ARGS)
		n = CTEST_PROBE_MAX_ARGS;
	ctest_probe_argc_v = n;
	for (i = 0; i < n; i++)
		snprintf (ctest_probe_argv_v[i], sizeof (ctest_probe_argv_v[i]), "%s", Cmd_Argv (i));
}
void ctest_probe_clear (void) { ctest_probe_argc_v = 0; }
int	 ctest_probe_argc (void) { return ctest_probe_argc_v; }
const char *ctest_probe_argv (int i) { return (i >= 0 && i < ctest_probe_argc_v) ? ctest_probe_argv_v[i] : ""; }
void		ctest_probe_set_argc (int n) { ctest_probe_argc_v = (n > CTEST_PROBE_MAX_ARGS) ? CTEST_PROBE_MAX_ARGS : n; }
void		ctest_probe_set_arg (int i, const char *value)
{
	if (i >= 0 && i < CTEST_PROBE_MAX_ARGS)
		snprintf (ctest_probe_argv_v[i], sizeof (ctest_probe_argv_v[i]), "%s", value);
}

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
 *
 * Phase 7 M2: Cvar_RegisterVariable / Cmd_AddCommand2 (and, further below,
 * Cvar_Set / Cvar_SetQuick / Cvar_SetCallback / Cmd_Argc / Cmd_Argv) used to
 * be placeholder logging stubs here so both the c_ref oracle's C_SOURCES
 * files and the Rust shims funneled through one capture point. Now that
 * cvar.c/cmd.c compile into this binary as c_ref_Cvar_RegisterVariable /
 * c_ref_Cmd_AddCommand2 / etc (c_ref_prelude.h), and quake-capi's `cvar`
 * feature provides the real plain-named Rust exports (quake-capi/src/cvar.rs,
 * cmd.rs) that quake-c-sys's other FFI callers (fs.rs, snd_dma.rs, ...) link
 * against, a stub definition under any of these plain names would collide at
 * link time with the c_ref_* rename of the real definition. The registration
 * event log (ctest_cvar_log / ctest_clear_cvar_log / ctest_cvar_log_len /
 * ctest_cvar_log_get, below) is left in place -- cfgfile_differential.rs and
 * fs_gamedir_differential.rs still call it, it now just always observes an
 * empty log on both sides, since neither reaches a capture stub any more. */

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

/* q_strcasestr copied verbatim from Quake/common.c: the cmd.c oracle's
 * Cmd_TintSubstring calls the real (renamed) q_strcasestr, and quake-capi's
 * cmd module needs the plain name too (common.c isn't in C_SOURCES).
 * q_strncasecmp (which this calls) is already defined further below as part
 * of the model_parse.c tail stand-ins -- reuse that definition rather than
 * redefining it here. */
char *q_strcasestr (const char *haystack, const char *needle)
{
	const size_t len = strlen (needle);

	if (!len)
		return (char *)haystack;

	while (*haystack)
	{
		if (!q_strncasecmp (haystack, needle, len))
			return (char *)haystack;

		++haystack;
	}

	return NULL;
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
 * Cvar_Set capture log accessors. See the Phase 7 M2 comment above
 * Cvar_RegisterVariable's old definition: the old plain-named logging stubs
 * are gone (both sides now reach the real c_ref_Cvar_Set / quake_rs
 * Cvar_Set), so the log is refilled through a change callback on cvars a
 * test registers into BOTH registries via ctest_register_logged_cvar_pair
 * (defined below the raise-capable wrappers). fs_gamedir_differential.rs
 * still observes an empty log on both sides by design.
 */
#define CTEST_CVAR_LOG_MAX 128
static char ctest_cvar_log[CTEST_CVAR_LOG_MAX][2][256];
static int	ctest_cvar_log_count;

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

/* ---------------------------------------------------------------------------
 * Phase 7 M2: cvar.c/cmd.c externals quake-capi's `cvar` feature needs.
 *
 * Host.c's Host_Guard/Host_Reraise and Quake/cvar_cmd_glue.c (the C frame
 * around the Rust cvar/cmd registries, ADR-009 rule 3) are not compiled into
 * this binary -- host.c isn't in C_SOURCES, and cvar_cmd_glue.c is only
 * built by Meson under -Duse_rust_cvar. quake-capi's cvar/cmd modules still
 * declare all of it as extern (rust/quake-c-sys/src/lib.rs, module
 * cvar_cmd), so enabling the `cvar` feature (this crate's Cargo.toml) makes
 * every test binary in this crate need these symbols to link, not just
 * cvar_cmd_differential.rs.
 *
 * Host_Guard below reuses the Sys_Error/Host_Error traps already defined
 * above (ctest_sys_error_jmp/armed/msg, ctest_host_error_jmp/armed/msg): it
 * saves/installs its own setjmp point in those same statics, the way the
 * real Host_Guard saves/installs host_abortserver/screen_error, so a
 * Host_Error or Sys_Error raised inside fn() is caught here instead of
 * unwinding past the Rust frame that called it. The two-case CTEST_GUARD_*
 * result isn't the real HOST_GUARD_OK/ABORTSERVER/SCREEN_ERROR (this harness
 * has no console-abort/fatal-screen distinction, only the two traps above),
 * so it uses its own names rather than claiming identical semantics.
 * Host_Reraise re-issues the catch by calling Host_Error/Sys_Error again,
 * landing in the next guard out or, at the outermost level, in a test's own
 * ctest_try_host/ctest_try.
 *
 * The plain Cvar_, Cbuf_ and Cmd_ wrapper names below are exactly
 * cvar_cmd_glue.c's raise-capable public ABI. They are also cvar.c/cmd.c's
 * own names (c_ref_prelude.h renames those to c_ref_*), so each one is
 * #undef'd first: the force-included prelude's rename macros are still
 * active in this translation unit and would otherwise rewrite the
 * definition itself to c_ref_Cvar_Set etc, colliding with the real,
 * unrelated c_ref_Cvar_Set this same binary links from cvar.c.
 */

#define CTEST_GUARD_OK		   0
#define CTEST_GUARD_HOST_ERROR 1
#define CTEST_GUARD_SYS_ERROR  2

int Host_Guard (void (*fn) (void *), void *arg)
{
	jmp_buf saved_host_jmp, saved_sys_jmp;
	int		saved_host_armed = ctest_host_error_armed;
	int		saved_sys_armed = ctest_sys_error_armed;
	int		result;

	memcpy (saved_host_jmp, ctest_host_error_jmp, sizeof (jmp_buf));
	memcpy (saved_sys_jmp, ctest_sys_error_jmp, sizeof (jmp_buf));

	if (setjmp (ctest_host_error_jmp))
	{
		result = CTEST_GUARD_HOST_ERROR;
	}
	else if (setjmp (ctest_sys_error_jmp))
	{
		result = CTEST_GUARD_SYS_ERROR;
	}
	else
	{
		ctest_host_error_armed = 1;
		ctest_sys_error_armed = 1;
		fn (arg);
		result = CTEST_GUARD_OK;
	}

	memcpy (ctest_host_error_jmp, saved_host_jmp, sizeof (jmp_buf));
	memcpy (ctest_sys_error_jmp, saved_sys_jmp, sizeof (jmp_buf));
	ctest_host_error_armed = saved_host_armed;
	ctest_sys_error_armed = saved_sys_armed;
	return result;
}

void Host_Reraise (int guard_result)
{
	switch (guard_result)
	{
	case CTEST_GUARD_HOST_ERROR:
		Host_Error ("%s", ctest_host_error_msg);
		break;
	case CTEST_GUARD_SYS_ERROR:
		Sys_Error ("%s", ctest_sys_error_msg);
		break;
	default:
		break;
	}
}

/* Quake/common.c's Info_SetKey/Info_RemoveKey (common.c:759-824), copied
 * verbatim: cvar.c's CVAR_SERVERINFO/CVAR_USERINFO replication below and the
 * Rust cvar module both call Info_SetKey directly, and common.c isn't in
 * C_SOURCES. */
static void ctest_Info_RemoveKey (char *info, const char *key)
{
	size_t keylen = strlen (key);

	while (*info)
	{
		char *l = info;
		if (*info++ != '\\')
			break;
		if (!strncmp (info, key, keylen) && info[keylen] == '\\')
		{
			info += keylen + 1;
			while (*info && *info != '\\')
				info++;
			memmove (l, info, strlen (info) + 1);
			return;
		}
		while (*info && *info != '\\')
			info++;
		if (*info++ != '\\')
			break;
		while (*info && *info != '\\')
			info++;
	}
}

void Info_SetKey (char *info, size_t infosize, const char *key, const char *val)
{
	size_t keylen = strlen (key);
	size_t vallen = strlen (val);

	ctest_Info_RemoveKey (info, key);

	if (vallen)
	{
		char *o = info + strlen (info);
		char *e = info + infosize - 1;

		if (!*key || strchr (key, '\\') || strchr (val, '\\'))
			Con_Warning ("Info_SetKey(%s): invalid key/value\n", key);
		else if (o + 2 + keylen + vallen >= e)
			Con_Warning ("Info_SetKey(%s): length exceeds max\n", key);
		else
		{
			*o++ = '\\';
			memcpy (o, key, keylen);
			o += keylen;
			*o++ = '\\';
			memcpy (o, val, vallen);
			o += vallen;
			*o = 0;
		}
	}
}

/* pr_ext.c's autocvar-changed notifier: no QC VM is loaded in these test
 * binaries (Phase 6's progs harness is a separate set of suites), so this is
 * a no-op -- CVAR_AUTOCVAR is never set on any fixture cvar here. */
void PR_AutoCvarChanged (cvar_t *var)
{
	(void)var;
}

/* ---------------------------------------------------------------------------
 * C-visible registry data (cvar_cmd_glue.c). console.c would walk these for
 * tab completion in the real engine; nothing in this binary does.
 */

#undef cl_nopext
#undef cmd_warncmd
cvar_t cl_nopext = {"cl_nopext", "0", CVAR_NONE};
cvar_t cmd_warncmd = {"cl_warncmd", "1", CVAR_NONE};

#define MAX_ALIAS_NAME 32
typedef struct cmdalias_s
{
	struct cmdalias_s *next;
	char			   name[MAX_ALIAS_NAME];
	char			  *value;
} cmdalias_t;

#undef cmd_text
#undef cmd_source
#undef cmd_functions
#undef cmd_alias
sizebuf_t		 cmd_text;
cmd_source_t	 cmd_source;
cmd_function_t	*cmd_functions;
cmdalias_t		*cmd_alias;

/* ---------------------------------------------------------------------------
 * CvarCmd_Glue_*: guarded dispatch + the accessors/replication blocks
 * cvar_cmd_glue.c keeps in C (svs/cls have no ADR-011 mirror yet).
 */

typedef struct
{
	xcommand_t fn;
} ctest_xcommand_arg_t;

static void ctest_invoke_xcommand (void *p)
{
	((ctest_xcommand_arg_t *)p)->fn ();
}

int CvarCmd_Glue_CallXCommand (xcommand_t fn)
{
	ctest_xcommand_arg_t arg;
	arg.fn = fn;
	return Host_Guard (ctest_invoke_xcommand, &arg);
}

typedef struct
{
	cvarcallback_t cb;
	cvar_t		  *var;
} ctest_cvarcallback_arg_t;

static void ctest_invoke_cvar_callback (void *p)
{
	ctest_cvarcallback_arg_t *a = (ctest_cvarcallback_arg_t *)p;
	a->cb (a->var);
}

int CvarCmd_Glue_CallCvarCallback (cvarcallback_t cb, cvar_t *var)
{
	ctest_cvarcallback_arg_t arg;
	arg.cb = cb;
	arg.var = var;
	return Host_Guard (ctest_invoke_cvar_callback, &arg);
}

static void ctest_invoke_autocvar_changed (void *p)
{
	PR_AutoCvarChanged ((cvar_t *)p);
}

int CvarCmd_Glue_AutoCvarChanged (cvar_t *var)
{
	return Host_Guard (ctest_invoke_autocvar_changed, var);
}

typedef struct
{
	sizebuf_t  *buf;
	const void *data;
	int			length;
} ctest_szwrite_arg_t;

static void ctest_invoke_szwrite (void *p)
{
	ctest_szwrite_arg_t *a = (ctest_szwrite_arg_t *)p;
	SZ_Write (a->buf, a->data, a->length);
}

int CvarCmd_Glue_SzWrite (sizebuf_t *buf, const void *data, int length)
{
	ctest_szwrite_arg_t arg;
	arg.buf = buf;
	arg.data = data;
	arg.length = length;
	return Host_Guard (ctest_invoke_szwrite, &arg);
}

const char *CvarCmd_Glue_HostClientName (void)
{
	return host_client ? host_client->name : "";
}

qboolean CvarCmd_Glue_ClsConnected (void)
{
	return cls.state == ca_connected;
}

qboolean CvarCmd_Glue_ClsDemoPlayback (void)
{
	return cls.demoplayback;
}

static void ctest_invoke_forward_begin (void *p)
{
	(void)p;
	MSG_WriteByte (&cls.message, clc_stringcmd);
}

int CvarCmd_Glue_ForwardBegin (void)
{
	return Host_Guard (ctest_invoke_forward_begin, NULL);
}

static void ctest_invoke_forward_print (void *p)
{
	SZ_Print (&cls.message, (const char *)p);
}

int CvarCmd_Glue_ForwardPrint (const char *s)
{
	return Host_Guard (ctest_invoke_forward_print, (void *)s);
}

/* protocol.h numbers, handed over rather than re-spelled here */
void CvarCmd_Glue_Protocols (int *rmq, int *fitzquake, int *netquake)
{
	*rmq = PROTOCOL_RMQ;
	*fitzquake = PROTOCOL_FITZQUAKE;
	*netquake = PROTOCOL_NETQUAKE;
}

void CvarCmd_Glue_PextNumbers (unsigned int *pext1, unsigned int *pext1_client, unsigned int *pext2, unsigned int *pext2_client)
{
	*pext1 = PROTOCOL_FTE_PEXT1;
	*pext1_client = PEXT1_SUPPORTED_CLIENT;
	*pext2 = PROTOCOL_FTE_PEXT2;
	*pext2_client = PEXT2_SUPPORTED_CLIENT;
}

static void ctest_invoke_serverinfo_changed (void *p)
{
	cvar_t *var = (cvar_t *)p;

	Info_SetKey (svs.serverinfo, sizeof (svs.serverinfo), var->name, var->string);

	for (client_t *current_client = svs.clients; current_client < svs.clients + svs.maxclients; current_client++)
	{
		if (current_client->active)
		{
			MSG_WriteByte (&current_client->message, svc_stufftext);
			MSG_WriteString (&current_client->message, va ("%s \"%s\" \"%s\"\n", "//svi", var->name, var->string));
		}
	}
}

int CvarCmd_Glue_ServerinfoChanged (cvar_t *var)
{
	return Host_Guard (ctest_invoke_serverinfo_changed, var);
}

static void ctest_invoke_userinfo_changed (void *p)
{
	cvar_t *var = (cvar_t *)p;

	Info_SetKey (cls.userinfo, sizeof (cls.userinfo), var->name, var->string);

	if (cls.state == ca_connected)
	{
		MSG_WriteByte (&cls.message, clc_stringcmd);
		if (var == &cl_name)
			MSG_WriteString (&cls.message, va ("name \"%s\"\n", var->string));
		else if (var == &cl_topcolor || var == &cl_bottomcolor)
			MSG_WriteString (&cls.message, va ("color \"%s\" \"%s\"\n", cl_topcolor.string, cl_bottomcolor.string));
		else
			MSG_WriteString (&cls.message, va ("setinfo \"%s\" \"%s\"\n", var->name, var->string));
	}
}

int CvarCmd_Glue_UserinfoChanged (cvar_t *var)
{
	return Host_Guard (ctest_invoke_userinfo_changed, var);
}

/* ---------------------------------------------------------------------------
 * The raise-capable public ABI: thin wrappers over the Rust quake_rs_*
 * status cores, re-issuing the caught jump from this pure C frame. Mirrors
 * cvar_cmd_glue.c exactly (its only reason to not be reused as-is is that it
 * is conditionally compiled by Meson, not part of any C_SOURCES entry here).
 */

/* quake-capi's cmd module (Cbuf_Init/Cbuf_AddText, direct exports) calls
 * these plain names directly, not through a quake_rs_* status core -- their
 * own overflow path (SZ_GetSpace) isn't reachable from Cbuf_Init/Cbuf_AddText
 * the way it is from Cbuf_InsertText, so no guard/reraise is needed here,
 * just a passthrough to net_msg.c's real (renamed) implementations.
 * SZ_Clear is deliberately NOT wrapped here: quake-capi's net module (the
 * "net" feature, already enabled in this crate's Cargo.toml) exports its own
 * real Rust SZ_Clear (rust/quake-capi/src/net.rs), so a plain-named C
 * definition of it here would be a duplicate-symbol link error. */
#undef SZ_Alloc
#undef SZ_Write
void c_ref_SZ_Alloc (sizebuf_t *buf, int startsize);
void c_ref_SZ_Write (sizebuf_t *buf, const void *data, int length);
void SZ_Alloc (sizebuf_t *buf, int startsize) { c_ref_SZ_Alloc (buf, startsize); }
void SZ_Write (sizebuf_t *buf, const void *data, int length) { c_ref_SZ_Write (buf, data, length); }

extern int		quake_rs_cbuf_execute (void);
extern void		quake_rs_cbuf_insert_text (const char *text, int *raised);
extern qboolean quake_rs_cmd_execute_string (const char *text, cmd_source_t src, int *raised);
extern int		quake_rs_cmd_forward_to_server (void);
extern void		quake_rs_cvar_register_variable (cvar_t *variable, int *raised);
extern void		quake_rs_cvar_set_quick (cvar_t *var, const char *value, int *raised);
extern void		quake_rs_cvar_set_value_quick (cvar_t *var, float value, int *raised);
extern void		quake_rs_cvar_set (const char *name, const char *value, int *raised);
extern void		quake_rs_cvar_set_value (const char *name, float value, int *raised);
extern void		quake_rs_cvar_set_rom (const char *name, const char *value, int *raised);
extern void		quake_rs_cvar_set_value_rom (const char *name, float value, int *raised);
extern cvar_t  *quake_rs_cvar_create (const char *name, const char *value, int *raised);
extern qboolean quake_rs_cvar_command (int *raised);
extern void		quake_rs_cvar_reset (const char *name, int *raised);

#undef Cbuf_Execute
void Cbuf_Execute (void)
{
	Host_Reraise (quake_rs_cbuf_execute ());
}

#undef Cbuf_InsertText
void Cbuf_InsertText (const char *text)
{
	int raised = 0;
	quake_rs_cbuf_insert_text (text, &raised);
	Host_Reraise (raised);
}

#undef Cmd_ForwardToServer
void Cmd_ForwardToServer (void)
{
	Host_Reraise (quake_rs_cmd_forward_to_server ());
}

#undef Cmd_ExecuteString
qboolean Cmd_ExecuteString (const char *text, cmd_source_t src)
{
	int		 raised = 0;
	qboolean result = quake_rs_cmd_execute_string (text, src, &raised);
	Host_Reraise (raised);
	return result;
}

#undef Cvar_RegisterVariable
void Cvar_RegisterVariable (cvar_t *variable)
{
	int raised = 0;
	quake_rs_cvar_register_variable (variable, &raised);
	Host_Reraise (raised);
}

#undef Cvar_SetQuick
void Cvar_SetQuick (cvar_t *var, const char *value)
{
	int raised = 0;
	quake_rs_cvar_set_quick (var, value, &raised);
	Host_Reraise (raised);
}

#undef Cvar_SetValueQuick
void Cvar_SetValueQuick (cvar_t *var, const float value)
{
	int raised = 0;
	quake_rs_cvar_set_value_quick (var, value, &raised);
	Host_Reraise (raised);
}

#undef Cvar_Set
void Cvar_Set (const char *var_name, const char *value)
{
	int raised = 0;
	quake_rs_cvar_set (var_name, value, &raised);
	Host_Reraise (raised);
}

#undef Cvar_SetValue
void Cvar_SetValue (const char *var_name, const float value)
{
	int raised = 0;
	quake_rs_cvar_set_value (var_name, value, &raised);
	Host_Reraise (raised);
}

#undef Cvar_SetROM
void Cvar_SetROM (const char *var_name, const char *value)
{
	int raised = 0;
	quake_rs_cvar_set_rom (var_name, value, &raised);
	Host_Reraise (raised);
}

#undef Cvar_SetValueROM
void Cvar_SetValueROM (const char *var_name, const float value)
{
	int raised = 0;
	quake_rs_cvar_set_value_rom (var_name, value, &raised);
	Host_Reraise (raised);
}

#undef Cvar_Create
cvar_t *Cvar_Create (const char *name, const char *value)
{
	int		raised = 0;
	cvar_t *result = quake_rs_cvar_create (name, value, &raised);
	Host_Reraise (raised);
	return result;
}

#undef Cvar_Command
qboolean Cvar_Command (void)
{
	int		 raised = 0;
	qboolean result = quake_rs_cvar_command (&raised);
	Host_Reraise (raised);
	return result;
}

#undef Cvar_Reset
void Cvar_Reset (const char *name)
{
	int raised = 0;
	quake_rs_cvar_reset (name, &raised);
	Host_Reraise (raised);
}

/* cfgfile_differential's capture seam: register one cvar under the same name
 * into the c_ref registry and the Rust registry, with a change callback that
 * appends (name, new value) to ctest_cvar_log. Cvar_SetQuick's no-change
 * early return means only real value changes log — identically on both
 * sides, which is the differential. The cvar_t storage and name leak by
 * design (registry entries must outlive the test). */
static void ctest_log_cvar_change (cvar_t *var)
{
	if (ctest_cvar_log_count >= CTEST_CVAR_LOG_MAX)
		return;
	q_strlcpy (ctest_cvar_log[ctest_cvar_log_count][0], var->name, sizeof (ctest_cvar_log[0][0]));
	q_strlcpy (ctest_cvar_log[ctest_cvar_log_count][1], var->string, sizeof (ctest_cvar_log[0][1]));
	ctest_cvar_log_count++;
}

#undef Cvar_SetCallback
extern void Cvar_SetCallback (cvar_t *var, cvarcallback_t func);

#define CTEST_LOGGED_CVAR_MAX 16
static cvar_t *ctest_logged_c[CTEST_LOGGED_CVAR_MAX];
static cvar_t *ctest_logged_r[CTEST_LOGGED_CVAR_MAX];
static int	   ctest_logged_count;

void ctest_register_logged_cvar_pair (const char *name)
{
	cvar_t *c_var = calloc (1, sizeof (cvar_t));
	cvar_t *r_var = calloc (1, sizeof (cvar_t));
	c_var->name = q_strdup (name);
	c_var->string = "";
	r_var->name = c_var->name;
	r_var->string = "";
	c_ref_Cvar_RegisterVariable (c_var);
	c_ref_Cvar_SetCallback (c_var, ctest_log_cvar_change);
	Cvar_RegisterVariable (r_var);
	Cvar_SetCallback (r_var, ctest_log_cvar_change);
	if (ctest_logged_count < CTEST_LOGGED_CVAR_MAX)
	{
		ctest_logged_c[ctest_logged_count] = c_var;
		ctest_logged_r[ctest_logged_count] = r_var;
		ctest_logged_count++;
	}
}

/* Drive every logged cvar to a sentinel no fixture uses, for ONE side
 * (0 = c_ref, 1 = Rust). Without this, a config pass that re-sets a cvar to
 * the value it already holds hits Cvar_SetQuick's no-change early return and
 * logs nothing, so cfgfile_differential's second CFG_ReadCvars pass (the one
 * that exercises FS_rewind state) would be invisible to the differential.
 * Resetting one side at a time keeps each side's log symmetric. */
void ctest_reset_logged_cvars (int side)
{
	int i;
	for (i = 0; i < ctest_logged_count; i++)
	{
		if (side == 0)
			c_ref_Cvar_SetQuick (ctest_logged_c[i], "sentinel");
		else
			Cvar_SetQuick (ctest_logged_r[i], "sentinel");
	}
}

/* Phase 7 M6: `sv` (and `svs`, and sv_user.c's `sv_player`) are defined by
 * the oracle sources now -- sv_main.c:27-28, sv_user.c:26 -- and renamed
 * c_ref_* by c_ref_prelude.h, so every reference in this file reaches that
 * one storage. Only the setters stay here. sv.modelname is what
 * model_parse.c reads (Mod_SetupSubmodels submodel-0 check). */

void ctest_set_sv_modelname (const char *name)
{
	snprintf (sv.modelname, sizeof (sv.modelname), "%s", name ? name : "");
}

cvar_t external_ents = {"external_ents", "1", CVAR_ARCHIVE};

void ctest_set_external_ents (float value)
{
	external_ents.value = value;
}

/* bgmusic.c/cd_null.c seams the Phase 4 bgmusic shim imports. bgmusic.c is
 * not compiled as a c_ref oracle, so there is no renamed counterpart to
 * collide with; CDAudio_Play reports "no CD" exactly like cd_null.c. */
cvar_t bgm_extmusic = {"bgm_extmusic", "1", CVAR_ARCHIVE};

int CDAudio_Play (byte track, qboolean looping)
{
	(void)track;
	(void)looping;
	return -1;
}

/* Mod_FindName: static pool keyed by name, so submodel chaining
 * (Mod_SetupSubmodels' "*1".."*n" clones) gets stable identical pointers on
 * both sides without the real model cache. */
/* sized for real maps: the M7 corpus gate loads shipped BSPs, and each
 * submodel costs one Mod_FindName clone — real id1 maps run to ~256 where
 * the synthetic fixtures used a handful */
#define CTEST_MOD_POOL_MAX 1024
static qmodel_t ctest_mod_pool[CTEST_MOD_POOL_MAX];
static int		ctest_mod_pool_count;

qmodel_t *Mod_FindName (const char *name)
{
	int i;
	for (i = 0; i < ctest_mod_pool_count; i++)
		if (!strcmp (ctest_mod_pool[i].name, name))
			return &ctest_mod_pool[i];
	/* Sys_Error, not assert: under the drivers' armed trap an oversized map
	 * fails through the normal reject path with the asset named in the
	 * manifest, instead of aborting the whole corpus run */
	if (ctest_mod_pool_count >= CTEST_MOD_POOL_MAX)
		Sys_Error ("ctest Mod_FindName pool exhausted (%d) at %s", ctest_mod_pool_count, name);
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

/* the engine's ReadLongUnaligned (gl_model.c): memcpy + LittleLong */
static int ctest_read_long_unaligned (byte *ptr)
{
	int temp;
	memcpy (&temp, ptr, sizeof (int));
	return LittleLong (temp);
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
		/* the frozen gl_model.c:1969-2001 cursor walk, minus the texture
		 * loads, using the same ReadLongUnaligned (memcpy + LittleLong) and
		 * struct-size arithmetic as the original */
		assert (pheader->poseverttype == PV_QUAKE1);
		if (numskins < 1 || numskins > 32 /* MAX_SKINS */)
			Sys_Error ("Mod_LoadAliasModel: Invalid # of skins: %d", numskins);
		int size = pheader->skinwidth * pheader->skinheight;
		for (int i = 0; i < numskins; i++)
		{
			if (ctest_read_long_unaligned (pskintype + offsetof (daliasskintype_t, type)) == ALIAS_SKIN_SINGLE)
			{
				pskintype += sizeof (daliasskintype_t) + size;
			}
			else
			{
				byte *pinskingroup = pskintype + sizeof (daliasskintype_t);
				int	  groupskins = ctest_read_long_unaligned (pinskingroup + offsetof (daliasskingroup_t, numskins));
				/* the engine's own behavior on a negative count is UB (the
				 * cursor walks backwards out of the image); trap it so the
				 * gate reports a clean reject instead of both sides reading
				 * out of bounds identically */
				if (groupskins < 0)
					Sys_Error ("ctest Mod_LoadAllSkins: negative skin group count %d", groupskins);
				byte *pinskinintervals = pinskingroup + sizeof (daliasskingroup_t);
				byte *skin = pinskinintervals + (groupskins * sizeof (daliasskininterval_t));
				pskintype = skin + (groupskins * size);
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

/* Phase 7 M7 (T7.3): the body stays a no-op -- gl_pscript.c is not an oracle
 * source and the harness has no particle system -- but this is the only call
 * CL_RegisterParticles's second loop makes, so with a bare no-op the loop is
 * completely unobservable and a differential over it passes while executing
 * nothing. Counting the calls and remembering the last model keeps the loop
 * bound and the iteration order visible. Both sides share this one definition,
 * so the counter is reset between the two runs. */
static int	ctest_pscript_model_effects = 0;
static char ctest_pscript_last_model[64];

void PScript_UpdateModelEffects (qmodel_t *mod)
{
	ctest_pscript_model_effects++;
	if (mod)
		q_strlcpy (ctest_pscript_last_model, mod->name, sizeof (ctest_pscript_last_model));
	else
		ctest_pscript_last_model[0] = '\0';
}

void ctest_pscript_model_effects_reset (void)
{
	ctest_pscript_model_effects = 0;
	ctest_pscript_last_model[0] = '\0';
}

int ctest_pscript_model_effects_count (void)
{
	return ctest_pscript_model_effects;
}

const char *ctest_pscript_last_model_name (void)
{
	return ctest_pscript_last_model;
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

/* ---------------------------------------------------------------------------
 * Phase 4 sound: stub-owned globals shared by c_ref snd_mem.c and (from M3)
 * the Rust shims, plus a driver for the resampler differential.
 */

static dma_t ctest_dma;

/* the suites are single-threaded; locking is a no-op */
qmutex_t *QMutex_Create (void)
{
	return NULL;
}
void QMutex_Destroy (qmutex_t *mutex)
{
	(void)mutex;
}
void QMutex_Lock (qmutex_t *mutex)
{
	(void)mutex;
}
void QMutex_Unlock (qmutex_t *mutex)
{
	(void)mutex;
}

void ctest_snd_setup (int shm_speed, float loadas8bit_value)
{
	memset (&ctest_dma, 0, sizeof (ctest_dma));
	ctest_dma.speed = shm_speed;
	ctest_dma.samplebits = 16;
	ctest_dma.channels = 2;
	shm = &ctest_dma;
	loadas8bit.value = loadas8bit_value;
}

/* Runs c_ref ResampleSfx exactly as S_LoadSound would: the cache header is
 * pre-filled from the wav info, out_len is S_LoadSound's alloc size. Copies
 * the rewritten header into meta_out[5] (length, loopstart, speed, width,
 * stereo) and the resampled PCM into out. */
void ctest_resample_ref (int length, int loopstart, int inrate, int inwidth, int stereo, const byte *data, byte *out, int out_len, int meta_out[5])
{
	sfx_t		sfx;
	sfxcache_t *sc = (sfxcache_t *)Mem_Alloc (out_len + sizeof (sfxcache_t));

	memset (&sfx, 0, sizeof (sfx));
	sc->length = length;
	sc->loopstart = loopstart;
	sc->speed = inrate;
	sc->width = inwidth;
	sc->stereo = stereo;
	sfx.cache = sc;

	ResampleSfx (&sfx, inrate, inwidth, (byte *)data);

	meta_out[0] = sc->length;
	meta_out[1] = sc->loopstart;
	meta_out[2] = sc->speed;
	meta_out[3] = sc->width;
	meta_out[4] = sc->stereo;
	memcpy (out, sc->data, out_len);
	free (sc);
}

/* ---------------------------------------------------------------------------
 * Phase 4 M4: mixer oracle state (snd_dma.c is not compiled here, so its
 * globals are stub-owned), pause-state slice, and the block-hash recorder
 * the c_ref mixer reports through the Harness_SndPaint seam.
 */

/* snd_channels/total_channels/paintedtime/... and the sound cvars are
 * defined by the c_ref snd_dma.c (renamed c_ref_* by the prelude); tests
 * reach them via those names. */

/* ---------------------------------------------------------------------------
 * DUPLICATE-SYMBOL HAZARD -- `cl` and `cls`
 *
 * Phase 7 M7 (T7.0) made cl_main.c an oracle source. cl_main.c defines `cl` and
 * `cls`, which c_ref_prelude.h renames to `c_ref_cl` / `c_ref_cls`, so the two
 * definitions below are NOT duplicates of them -- they are the Rust-side
 * storage, and the `#undef` is what keeps them un-renamed. Removing either
 * `#undef` produces a second definition of `c_ref_cl` / `c_ref_cls` in the same
 * link (link.exe LNK2005; ld64 and rust-lld would instead silently drop the
 * archive member and take the oracle's, which is worse -- the differential
 * would then compare the oracle against itself).
 *
 * Every *reader* in the stubs stays renamed on purpose: the readers here are
 * ctest glue that must observe the same objects the oracle sources observe
 * (world.c walks `cl.entities`, cvar.c writes `cls.message`, sv_send.c reads
 * `cls.demorecording`). Only the two definitions are exempted.
 *
 * The Rust side reaches the un-renamed objects through plain externs:
 * quake-capi/src/sv_main.rs:105,107 (`SV_Pext_f`), sv_send.rs:125 and
 * sv_user.rs:73 (`cls.netcon`). Any fixture that seeds a field those paths
 * read must seed BOTH copies -- see ctest_svuser_reset in sv_user_ref.c, and
 * the identical rule M6 established for `sv`/`svs`.
 *
 * T7.4 closes the ADR-007 row for `cl`/`cls` by moving this storage into
 * quake-capi exactly as T6.6 did for `sv`/`svs`. When it lands, these two
 * definitions become `extern` declarations (`extern client_state_t cl;`) and
 * the `#undef`s stay; nothing else in this file has to change, because
 * everything else already goes through the renamed names.
 * ------------------------------------------------------------------------ */
#undef cl
extern client_state_t cl; /* T7.4: quake-capi/src/cl_main.rs owns the storage */
#define cl c_ref_cl
keydest_t	   key_dest = key_game;
double		   host_frametime = 0.0;

qboolean harness_sndhash = false;

static uint64_t ctest_snd_block_hash = UINT64_C (0xcbf29ce484222325);
static int		ctest_snd_block_count = 0;

static uint64_t ctest_snd_hash64 (uint64_t h, const void *data, size_t len)
{
	const byte *pd = (const byte *)data;
	while (len--)
	{
		h ^= *pd++;
		h *= UINT64_C (0x100000001b3);
	}
	return h;
}

void Harness_SndPaint (int painted, int end, const void *paintbuf, const volatile unsigned char *dmabuf, int dmabytes)
{
	uint64_t h = ctest_snd_block_hash;
	h = ctest_snd_hash64 (h, &painted, sizeof (painted));
	h = ctest_snd_hash64 (h, &end, sizeof (end));
	h = ctest_snd_hash64 (h, paintbuf, (size_t)(end - painted) * 8);
	h = ctest_snd_hash64 (h, (const void *)dmabuf, (size_t)dmabytes);
	ctest_snd_block_hash = h;
	ctest_snd_block_count++;
}

void ctest_snd_block_reset (void)
{
	ctest_snd_block_hash = UINT64_C (0xcbf29ce484222325);
	ctest_snd_block_count = 0;
	harness_sndhash = true; /* route the c_ref mixer through the hook */
}

uint64_t ctest_snd_block_get (int *count)
{
	*count = ctest_snd_block_count;
	return ctest_snd_block_hash;
}

/* full DMA description for the mixer differential; buffer is caller-owned */
void ctest_snd_setup_dma (int speed, int samplebits, int channels, int signed8, int samples, unsigned char *buffer)
{
	memset (&ctest_dma, 0, sizeof (ctest_dma));
	ctest_dma.speed = speed;
	ctest_dma.samplebits = samplebits;
	ctest_dma.channels = channels;
	ctest_dma.signed8 = signed8;
	ctest_dma.samples = samples;
	ctest_dma.buffer = buffer;
	shm = &ctest_dma;
}

void ctest_snd_set_pause_state (int cl_paused, int sv_active, int maxclients, int keydest, double frametime)
{
	cl.paused = cl_paused;
	sv.active = sv_active;
	svs.maxclients = maxclients;
	key_dest = (keydest_t)keydest;
	host_frametime = frametime;
}

void ctest_snd_set_cvars (float sfxvol, float sndspeed_v, float filterquality, float waterfx, float pauselooping)
{
	sfxvolume.value = sfxvol;
	sndspeed.value = sndspeed_v;
	snd_filterquality.value = filterquality;
	snd_waterfx.value = waterfx;
	snd_pauselooping.value = pauselooping;
}

/* Phase 4 M6: the snd_dma.c oracle's remaining seams. Phase 7 M7: the Rust-side
 * client_static_t; see the DUPLICATE-SYMBOL HAZARD block above `cl`. */
#undef cls
extern client_static_t cls; /* T7.4: quake-capi/src/cl_main.rs owns the storage */
#define cls c_ref_cls

static mleaf_t *ctest_point_leaf = NULL;
void ctest_set_point_leaf (mleaf_t *leaf)
{
	ctest_point_leaf = leaf;
}
mleaf_t *Mod_PointInLeaf (float *p, qmodel_t *model)
{
	(void)p;
	(void)model;
	return ctest_point_leaf;
}

/* Phase 7 M2: the old Cmd_Argc/Cmd_Argv placeholders (always argc==0) were
 * removed -- they would collide at link time with the c_ref_* rename of the
 * real cmd.c definitions (c_ref_prelude.h). Both sides now reach real
 * tokenizer state: c_ref_Cmd_Argc/c_ref_Cmd_Argv (cmd.c) for the oracle, and
 * quake_rs::cmd::Cmd_Argc/Cmd_Argv (quake-capi's `cvar` feature) for callers
 * that link the Rust shim. */

CON_STUB (Con_SafePrintf, "[safe]")

qboolean SNDDMA_Init (dma_t *dma)
{
	(void)dma;
	return false;
}
int SNDDMA_GetDMAPos (void)
{
	return 0;
}
void SNDDMA_Shutdown (void) {}
void SNDDMA_LockBuffer (void) {}
void SNDDMA_Submit (void) {}
void SNDDMA_BlockSound (void) {}
void SNDDMA_UnblockSound (void) {}
qboolean Harness_SNDDMA_Init (void *dma)
{
	(void)dma;
	return false;
}
int Harness_SNDDMA_GetDMAPos (void)
{
	return 0;
}
void Harness_SNDDMA_Shutdown (void) {}

void ctest_snd_set_listener (const float *origin, const float *right)
{
	memcpy ((void *)listener_origin, origin, sizeof (vec3_t));
	memcpy ((void *)listener_right, right, sizeof (vec3_t));
}

void ctest_set_cl_viewentity (int viewentity)
{
	cl.viewentity = viewentity;
}

/* ---------------------------------------------------------------------------
 * Phase 4 M7: the c_ref codec framework's remaining seams (q_strcasecmp /
 * q_snprintf already stubbed above). The dummy c_ref mp3 codec mirrors the
 * Rust-side dummy in src/snd_stubs.rs exactly.
 */

static qboolean ctest_dummy_codec_init (void)
{
	return true;
}
static void ctest_dummy_codec_shutdown (void) {}
static qboolean ctest_dummy_codec_open (snd_stream_t *stream)
{
	(void)stream;
	return false;
}
static int ctest_dummy_codec_read (snd_stream_t *stream, int bytes, void *buffer)
{
	(void)stream;
	(void)bytes;
	(void)buffer;
	return 0;
}
static int ctest_dummy_codec_rewind (snd_stream_t *stream)
{
	(void)stream;
	return -1;
}
static void ctest_dummy_codec_close (snd_stream_t *stream)
{
	(void)stream;
}

snd_codec_t mp3_codec = {
	CODECTYPE_MP3,
	true,
	"mp3",
	ctest_dummy_codec_init,
	ctest_dummy_codec_shutdown,
	ctest_dummy_codec_open,
	ctest_dummy_codec_read,
	ctest_dummy_codec_rewind,
	NULL,
	ctest_dummy_codec_close,
	NULL};

/* open a stand-alone fshandle_t over a plain OS file (start 0, full length) */
int ctest_open_fshandle (const char *path, fshandle_t *fh)
{
	FILE *f = Sys_fopen (path, "rb");
	if (!f)
		return -1;
	memset (fh, 0, sizeof (*fh));
	fh->file = f;
	fh->start = 0;
	fh->pos = 0;
	fh->length = Sys_filelength (f);
	fh->pak = false;
	return 0;
}

/* ---------------------------------------------------------------------------
 * Phase 5 (net_msg.c): the c_ref reader state. net_main.c owns the real
 * net_message; here the storage is stub-owned and tests aim .data/.cursize
 * at their fixtures (the names expand to c_ref_* through the prelude).
 */
sizebuf_t	 net_message;
unsigned int harness_badread_count;

/* ---------------------------------------------------------------------------
 * Phase 5 M5 (net_loop.c): the net_main.c globals the loopback oracle
 * references (names expand to c_ref_* through the prelude), plus a tiny
 * qsocket pool standing in for NET_NewQSocket. ctest_qsocket_reset_c lets
 * tests restart a scenario.
 */
int			net_driverlevel;
int			net_activeconnections;
size_t		hostCacheCount;
hostcache_t hostcache[HOSTCACHESIZE];
cvar_t		hostname = {"hostname", "UNNAMED", CVAR_NONE};

#define CTEST_QSOCKET_POOL 4
static qsocket_t ctest_qsocket_pool[CTEST_QSOCKET_POOL];
static int		 ctest_qsocket_used;

qsocket_t *NET_NewQSocket (void)
{
	if (ctest_qsocket_used >= CTEST_QSOCKET_POOL)
		return NULL;
	memset (&ctest_qsocket_pool[ctest_qsocket_used], 0, sizeof (qsocket_t));
	return &ctest_qsocket_pool[ctest_qsocket_used++];
}

void NET_FreeQSocket (qsocket_t *sock)
{
	(void)sock;
}

void ctest_qsocket_reset_c (void)
{
	ctest_qsocket_used = 0;
	memset (ctest_qsocket_pool, 0, sizeof (ctest_qsocket_pool));
}

/*
 * Phase 5 M6 (net_dgrm_rel.c): byte-order dispatch plus the ambient net
 * globals the reliable layer references (all names expand through the
 * c_ref renames). Every CI host is little-endian, so BigLong swaps --
 * matching COM_Init's runtime dispatch in the engine.
 */
static int ctest_LongSwap (int l)
{
	byte b1 = l & 255, b2 = (l >> 8) & 255, b3 = (l >> 16) & 255, b4 = (l >> 24) & 255;
	return ((int)b1 << 24) + ((int)b2 << 16) + ((int)b3 << 8) + b4;
}
int (*BigLong) (int l) = ctest_LongSwap;
double			net_time;
int				messagesReceived;
int				unreliableMessagesReceived;
net_landriver_t net_landrivers[3];

/* Phase 5 M7b (net_udp.c oracle + the Rust UDP shims): ambient globals and
 * a fixed clock (expand through the c_ref renames where renamed) */
int		 net_hostport = 26000;
char	 my_ipv4_address[64];
char	 my_ipv6_address[64];
qboolean ipv4Available;
qboolean ipv6Available;
double	 Sys_DoubleTime (void)
{
	return 0.0;
}

/* Phase 5 M9: the net_main.c accessor funnels + Cbuf the Rust core
 * references (linux links every rlib object; never invoked by tests) */
qboolean NetMain_SVActive (void)
{
	return false;
}
int NetMain_MaxClients (void)
{
	return 4; /* matches the test qsocket pools */
}
int NetMain_MaxClientsLimit (void)
{
	return 4;
}
void NetMain_SetMaxClients (int n)
{
	(void)n;
}
/* Phase 7 M2: Cbuf_AddText/Cbuf_Init/Cbuf_AddTextLen/Cbuf_Waited are direct
 * quake-capi `#[no_mangle]` exports now (cmd.rs) -- the c_ref_prelude.h
 * rename table sends the real cvar.c/cmd.c oracle's own calls to these names
 * to c_ref_Cbuf_AddText etc, so a plain-named placeholder here would collide
 * with that renamed definition once the "cvar" feature links cmd.c. */

/* Phase 5 M10: base pointers of the driver vtable arrays. The engine
 * defines these in net_main.c; here they hand back the stub arrays so the
 * capi shims link on platforms whose linker resolves every rlib object
 * (Linux) rather than only the reachable ones (macOS). */
net_driver_t *NetMain_Drivers (void)
{
	static net_driver_t ctest_net_drivers[2];
	return ctest_net_drivers;
}
net_landriver_t *NetMain_LanDrivers (void)
{
	return net_landrivers;
}

/* Phase 5 M10: the platform libc's own atoi, so quake_net::cnum::c_atoi is
 * pinned against it rather than against a reading of the standard (the
 * (int)strtol truncation point moves with sizeof(long): LP64 vs LLP64) */
int ctest_atoi (const char *s)
{
	return atoi (s);
}

void ctest_dgrm_reset_c (void)
{
	memset (&packetBuffer, 0, sizeof (packetBuffer));
	packetsSent = packetsReSent = packetsReceived = 0;
	receivedDuplicateCount = shortPacketCount = droppedDatagrams = 0;
	net_time = 0;
	messagesReceived = unreliableMessagesReceived = 0;
}

/* ---------------------------------------------------------------------------
 * Phase 5 M4 (review follow-up): fscanf oracle for the demo forcetrack
 * header parse. This is the exact C idiom CL_PlayDemo_f used --
 * fscanf ("%i") then an explicit fgetc == '\n' -- run over a memory buffer
 * via tmpfile so quake_net::demo::parse_forcetrack can be differentially
 * tested against the platform libc instead of the implementer's reading.
 */
int ctest_demo_forcetrack_oracle (const char *bytes, int len, int *track, int *consumed)
{
	FILE *f = tmpfile ();
	int	  ok;
	if (!f)
		return -1;
	if (len > 0 && fwrite (bytes, 1, (size_t)len, f) != (size_t)len)
	{
		fclose (f);
		return -1;
	}
	rewind (f);
	ok = !(fscanf (f, "%i", track) != 1 || fgetc (f) != '\n');
	if (ok)
		*consumed = (int)ftell (f);
	fclose (f);
	return ok;
}

/* ---------------------------------------------------------------------------
 * Phase 6 M2: the progs-VM fixture the pr_edict_arena.c oracle runs against.
 *
 * `qcvm` (renamed c_ref_qcvm by the prelude) is the ambient VM every function
 * in that file dereferences, and EDICT_NUM/NUM_FOR_EDICT stay behind in
 * pr_edict.c, so they are reproduced here verbatim -- including EDICT_NUM's
 * Host_Error bounds check, which the differential suites exercise through the
 * ctest_try_host trap.
 */
qcvm_t		  *qcvm;
globalvars_t  *pr_global_struct;
entity_state_t nullentitystate;

/* pr_edict.c seams the pr_exec.c oracle needs (Phase 6 M3) */
int ctest_progs_ed_print_count;

void ED_Print (edict_t *ed)
{
	(void)ed;
	ctest_progs_ed_print_count++;
}

void PR_SwitchQCVM (qcvm_t *nvm)
{
	if (nvm && qcvm)
		Sys_Error ("PR_SwitchQCVM: A qcvm was already active");
	qcvm = nvm;
	pr_global_struct = nvm ? (globalvars_t *)nvm->globals : NULL;
}

const char *PR_GlobalString (int ofs)
{
	(void)ofs;
	return "<global>";
}

const char *PR_GlobalStringNoContents (int ofs)
{
	(void)ofs;
	return "<global>";
}

static qcvm_t	   ctest_progs_vm_storage;
static dprograms_t ctest_progs_header;

edict_t *EDICT_NUM (int n)
{
	if (n < 0 || n >= qcvm->max_edicts)
		Host_Error ("EDICT_NUM: bad edict_num %i", n);
	return EDICT_NUM_NO_CHECK (n);
}

int NUM_FOR_EDICT (edict_t *e)
{
	int b = (byte *)e - (byte *)qcvm->edicts;
	b = b / qcvm->edict_size;

	if (b < 0 || b >= qcvm->num_edicts)
		Host_Error ("NUM_FOR_EDICT: bad pointer");
	return b;
}

/* common.c's COM_SetupNullState: the null baseline is not all-zero */
static void ctest_progs_setup_nullstate (void)
{
	memset (&nullentitystate, 0, sizeof (nullentitystate));
	nullentitystate.colormod[0] = 32;
	nullentitystate.colormod[1] = 32;
	nullentitystate.colormod[2] = 32;
	nullentitystate.colormap = 0;
	nullentitystate.alpha = ENTALPHA_DEFAULT;
	nullentitystate.scale = ENTSCALE_DEFAULT;
	nullentitystate.solidsize = ES_SOLID_NOT;
}

/* Builds a VM whose edict_size is computed exactly as PR_LoadProgs does, so
 * the Rust arena's stride and the oracle's agree by construction. */
void *ctest_progs_reset_vm (int max_edicts, int entityfields)
{
	qcvm_t *vm = &ctest_progs_vm_storage;

	if (vm->edicts)
		Mem_Free (vm->edicts);
	memset (vm, 0, sizeof (*vm));
	memset (&ctest_progs_header, 0, sizeof (ctest_progs_header));
	ctest_progs_setup_nullstate ();

	ctest_progs_header.entityfields = entityfields;
	vm->progs = &ctest_progs_header;
	vm->edict_size = entityfields * 4 + sizeof (edict_t) - sizeof (entvars_t);
	vm->edict_size += sizeof (void *) - 1;
	vm->edict_size &= ~(sizeof (void *) - 1);
	vm->max_edicts = max_edicts;
	vm->num_edicts = 0;
	vm->edicts = (edict_t *)Mem_Alloc ((size_t)max_edicts * vm->edict_size);

	qcvm = vm;
	return vm;
}

size_t ctest_progs_edict_size (void)
{
	return (size_t)ctest_progs_vm_storage.edict_size;
}

void *ctest_progs_edicts (void)
{
	return ctest_progs_vm_storage.edicts;
}

void ctest_progs_set_time (double t)
{
	ctest_progs_vm_storage.time = t;
}

/* string-table fixture: PR_SetEngineString compares against the strings blob,
 * so the tests need one with a known size */
void ctest_progs_set_strings (char *blob, int size, int progsstrings)
{
	ctest_progs_vm_storage.strings = blob;
	ctest_progs_vm_storage.stringssize = size;
	ctest_progs_vm_storage.progsstrings = progsstrings;
}

/* ED_RebuildFreeList sorts with qsort() and a comparator that never returns 0
 * (copysign(1.0, d)), so tie ordering is whatever this platform's qsort does
 * with an inconsistent comparator. The Rust port therefore does not implement
 * the sort -- it takes it as a parameter, and the differential suites hand it
 * this helper: the same libc qsort, the same comparator, over a freetime
 * table supplied by the caller's arena. */
static const float *ctest_sort_freetimes;

static int ctest_sort_freetime_cmp (const void *first, const void *second)
{
	int firstInt = *(const int *)first;
	int secondInt = *(const int *)second;
	return (int)copysign (1.0, ctest_sort_freetimes[firstInt] - ctest_sort_freetimes[secondInt]);
}

void ctest_progs_sort_by_freetime (int *nums, size_t n, const float *freetimes)
{
	ctest_sort_freetimes = freetimes;
	qsort (nums, n, sizeof (int), ctest_sort_freetime_cmp);
	ctest_sort_freetimes = NULL;
}

/* ---------------------------------------------------------------------------
 * Phase 6 M3: synthetic progs images for the interpreter differential.
 *
 * Two identically-initialised VMs are built: A is published as the ambient
 * `qcvm` for the c_ref_PR_ExecuteProgram oracle, B is handed to the Rust
 * interpreter through quake_progs::arena::VmRaw. Building both here means the
 * two agree on edict_size and lump placement by construction, and the
 * comparison afterwards is between two allocations of identical shape.
 */
#define CTEST_PROGS_NVMS 2

static qcvm_t	   ctest_synth_vm[CTEST_PROGS_NVMS];
static dprograms_t ctest_synth_hdr[CTEST_PROGS_NVMS];

int ctest_progs_builtin_calls;
int ctest_progs_last_builtin_vm;

/* A builtin both sides invoke. It writes a value derived from the ambient
 * VM's own state, so a mix-up between the two fixtures would show up. */
static void PF_Fixme_stub (void)
{
	ctest_progs_builtin_calls++;
}

static void ctest_progs_builtin_marker (void)
{
	ctest_progs_builtin_calls++;
	G_FLOAT (OFS_RETURN) = (float)(qcvm->argc * 100 + qcvm->xstatement);
	G_FLOAT (OFS_RETURN + 1) = G_FLOAT (OFS_PARM0);
	G_FLOAT (OFS_RETURN + 2) = 0.0f;
}

static void ctest_progs_builtin_error (void)
{
	ctest_progs_builtin_calls++;
	Host_Error ("ctest builtin raised");
}

void *ctest_progs_synth_vm (
	int which, int max_edicts, int entityfields, int numglobals, const void *stmts, int nstmts, const void *funcs, int nfuncs, const char *strings,
	int stringssize)
{
	qcvm_t	   *vm = &ctest_synth_vm[which];
	dprograms_t *hdr = &ctest_synth_hdr[which];

	if (vm->edicts)
		Mem_Free (vm->edicts);
	if (vm->statements)
		Mem_Free (vm->statements);
	if (vm->functions)
		Mem_Free (vm->functions);
	if (vm->globals)
		Mem_Free (vm->globals);
	if (vm->strings)
		Mem_Free (vm->strings);
	memset (vm, 0, sizeof (*vm));
	memset (hdr, 0, sizeof (*hdr));

	hdr->entityfields = entityfields;
	hdr->numfunctions = nfuncs;
	hdr->numstatements = nstmts;
	hdr->numglobals = numglobals;
	vm->progs = hdr;

	vm->statements = (dstatement_t *)Mem_Alloc ((size_t)nstmts * sizeof (dstatement_t));
	memcpy (vm->statements, stmts, (size_t)nstmts * sizeof (dstatement_t));
	vm->functions = (dfunction_t *)Mem_Alloc ((size_t)nfuncs * sizeof (dfunction_t));
	memcpy (vm->functions, funcs, (size_t)nfuncs * sizeof (dfunction_t));
	vm->globals = (float *)Mem_Alloc ((size_t)numglobals * sizeof (float));
	vm->strings = (char *)Mem_Alloc ((size_t)stringssize);
	memcpy (vm->strings, strings, (size_t)stringssize);
	vm->stringssize = stringssize;

	vm->edict_size = entityfields * 4 + sizeof (edict_t) - sizeof (entvars_t);
	vm->edict_size += sizeof (void *) - 1;
	vm->edict_size &= ~(sizeof (void *) - 1);
	vm->max_edicts = max_edicts;
	vm->num_edicts = max_edicts; /* all live: the tests address them directly */
	vm->edicts = (edict_t *)Mem_Alloc ((size_t)max_edicts * vm->edict_size);

	vm->builtins[0] = PF_Fixme_stub;
	vm->builtins[1] = ctest_progs_builtin_marker;
	vm->builtins[2] = ctest_progs_builtin_error;
	vm->numbuiltins = 3;

	return vm;
}

void *ctest_progs_vm (int which)
{
	return &ctest_synth_vm[which];
}

/* publish fixture A as the ambient VM for the oracle */
void ctest_progs_select_vm (int which)
{
	qcvm = &ctest_synth_vm[which];
	pr_global_struct = (globalvars_t *)qcvm->globals;
}

void ctest_progs_synth_free (void)
{
	int i;
	for (i = 0; i < CTEST_PROGS_NVMS; i++)
	{
		qcvm_t *vm = &ctest_synth_vm[i];
		SAFE_FREE (vm->edicts);
		SAFE_FREE (vm->statements);
		SAFE_FREE (vm->functions);
		SAFE_FREE (vm->globals);
		SAFE_FREE (vm->strings);
		SAFE_FREE (vm->fielddefs);
		SAFE_FREE (vm->globaldefs);
	}
	qcvm = NULL;
	pr_global_struct = NULL;
}

/* Invoke a builtin against a chosen fixture, the way the engine's guarded
 * dispatch does: the ambient VM is the one the builtin reads. */
void ctest_progs_call_builtin (int which, int index)
{
	qcvm_t		 *saved = qcvm;
	globalvars_t *saved_g = pr_global_struct;
	qcvm = &ctest_synth_vm[which];
	pr_global_struct = (globalvars_t *)qcvm->globals;
	qcvm->builtins[index] ();
	qcvm = saved;
	pr_global_struct = saved_g;
}

void ctest_progs_set_sv_state (int active)
{
	sv.state = active ? ss_active : ss_loading;
}

/* ---------------------------------------------------------------------------
 * Phase 6 M4: seams the pr_edict_save.c oracle needs. type_size and
 * ED_FieldAtOfs stay in pr_edict.c; the tests install a fielddef table so
 * ED_FieldAtOfs has something to search.
 */
const int type_size[NUM_TYPE_SIZES] = {1, 1, 1, 3, 1, 1, 1, 1};

ddef_t *ED_FieldAtOfs (int ofs)
{
	int i;
	for (i = 0; i < qcvm->progs->numfielddefs; i++)
	{
		ddef_t *def = &qcvm->fielddefs[i];
		if (def->ofs == ofs)
			return def;
	}
	return NULL;
}

/* Runs a c_ref writer into a temp file and returns its bytes. */
int ctest_progs_capture_ed_write (int edict_num, unsigned char *out, int out_max)
{
	FILE *f = tmpfile ();
	long  n;
	if (!f)
		return -1;
	c_ref_ED_Write (f, EDICT_NUM (edict_num));
	fflush (f);
	n = ftell (f);
	rewind (f);
	if (n > out_max)
		n = out_max;
	n = (long)fread (out, 1, (size_t)n, f);
	fclose (f);
	return (int)n;
}

int ctest_progs_capture_ed_write_globals (unsigned char *out, int out_max)
{
	FILE *f = tmpfile ();
	long  n;
	if (!f)
		return -1;
	c_ref_ED_WriteGlobals (f);
	fflush (f);
	n = ftell (f);
	rewind (f);
	if (n > out_max)
		n = out_max;
	n = (long)fread (out, 1, (size_t)n, f);
	fclose (f);
	return (int)n;
}

/* Installs a synthetic def table on a fixture so the writers have something
 * to iterate. Fielddefs/globaldefs are copied; the caller keeps ownership of
 * nothing. */
void ctest_progs_set_defs (int which, const void *fielddefs, int numfielddefs, const void *globaldefs, int numglobaldefs, int extfields_alpha)
{
	qcvm_t *vm = (qcvm_t *)ctest_progs_vm (which);

	if (vm->fielddefs)
		Mem_Free (vm->fielddefs);
	if (vm->globaldefs)
		Mem_Free (vm->globaldefs);

	vm->fielddefs = (ddef_t *)Mem_Alloc ((size_t)numfielddefs * sizeof (ddef_t));
	memcpy (vm->fielddefs, fielddefs, (size_t)numfielddefs * sizeof (ddef_t));
	vm->progs->numfielddefs = numfielddefs;

	vm->globaldefs = (ddef_t *)Mem_Alloc ((size_t)numglobaldefs * sizeof (ddef_t));
	memcpy (vm->globaldefs, globaldefs, (size_t)numglobaldefs * sizeof (ddef_t));
	vm->progs->numglobaldefs = numglobaldefs;

	vm->extfields.alpha = extfields_alpha;
}

/* ---------------------------------------------------------------------------
 * Phase 6 M5: the lookups pr_edict_parse.c's oracle needs. The real ones are
 * hash-map queries in pr_edict.c; over a synthetic fixture a linear search is
 * equivalent, and the Rust side is driven through the same tables.
 */
ddef_t *ED_FindField (const char *name)
{
	int i;
	for (i = 0; i < qcvm->progs->numfielddefs; i++)
	{
		ddef_t *def = &qcvm->fielddefs[i];
		if (!strcmp (PR_GetString (def->s_name), name))
			return def;
	}
	return NULL;
}

dfunction_t *ED_FindFunction (const char *fn_name)
{
	int i;
	for (i = 0; i < qcvm->progs->numfunctions; i++)
	{
		if (!strcmp (PR_GetString (qcvm->functions[i].s_name), fn_name))
			return &qcvm->functions[i];
	}
	return NULL;
}


/* ---------------------------------------------------------------------------
 * Phase 7 M3: the world.c fixture.
 *
 * world.c needs a whole server to be interesting: an ambient qcvm with an
 * edict arena and an areanode tree, a brush model with three clipping hulls
 * and a BSP node/leaf tree, a progs image whose touch functions actually
 * execute, and the sv_phys.c/client-VM seams it calls out through.
 *
 * Everything below is shared by both sides of the differential: the c_ref
 * world.c reaches it under the plain names, and the Rust port reaches the
 * same objects through world_glue.c's accessors -- whose stub bodies are
 * also here, because world_glue.c is a Meson-only file and is not in
 * build.rs's C_SOURCES. Nothing here references a Rust symbol, so the oracle
 * links and self-checks on its own; the two places where a test has to pick
 * an implementation (the re-entrant link/unlink a touch handler performs) go
 * through function pointers the test installs.
 */

/* quakedef.h's assert_always backing (common.c:1838). The engine prints and
 * dies; here it routes into the Sys_Error trap so a test can assert WHICH
 * assertion fired. The abort() is unreachable -- Sys_Error either longjmps
 * out or aborts itself -- and only satisfies the FUNC_NORETURN contract. */
void COM_Assert_Failed (const char *expr, const char *file_path, int line)
{
	Sys_Error ("Assertion failed: %s (%s:%d)", expr, file_path, line);
	abort ();
}

/* common.c:82-100, verbatim: world.c's areanode chains are these links, and
 * common.c is not in C_SOURCES. */
void ClearLink (link_t *l)
{
	l->prev = l->next = l;
}

void RemoveLink (link_t *l)
{
	l->next->prev = l->prev;
	l->prev->next = l->next;
}

void InsertLinkBefore (link_t *l, link_t *before)
{
	l->next = before;
	l->prev = before->prev;
	l->prev->next = l;
	l->next->prev = l;
}

/* pr_ext.c's extension gate. Shared un-renamed: world.c branches on it in
 * five places and the Rust port reads the same object through quake-c-sys,
 * so there is deliberately only one copy. */
cvar_t pr_checkextension;

/* world_glue.c owns these two for the Rust side; world.c defines its own
 * pair, renamed c_ref_*. ctest_world_set_cvars writes both so they cannot
 * drift apart mid-test.
 *
 * Declared, not defined: world.c:33/35 owns the c_ref_* pair, and the
 * prelude renames this file's `sv_fte_*` to `c_ref_sv_fte_*` as well, so a
 * tentative definition here would be a second strong definition of world.c's
 * under -fno-common (GCC 10+, and rust-lld). MSVC merges the two silently;
 * GNU/lld rejects them as duplicate symbols. */
extern cvar_t sv_fte_recursivehullckeck;
extern cvar_t sv_fte_createareanode;

/* --- the synthetic brush model -------------------------------------------
 *
 * A hollow room with a solid pillar and three liquid volumes, built as a
 * nested if/else chain of axis-aligned boxes. Each box contributes six
 * clipnodes -- for axis a, one plane at maxs[a] (front child = outside) and
 * one at mins[a] (back child = outside) -- with every "outside" edge pointing
 * at the next box's root and the last box falling through to CONTENTS_SOLID.
 * That is a well-formed clipnode tree (every node is a plane split with two
 * subtrees), so both hull-check implementations and SV_HullPointContents walk
 * it exactly as they would a real BSP.
 *
 * The three hulls are built from the same boxes expanded the way qbsp expands
 * them (solid volumes grow by -clip_maxs..-clip_mins, the open room shrinks
 * by the inverse), so hulls 0/1/2 really do differ and SV_HullForEntity's
 * size-based selection is observable in trace results.
 */

#define CTEST_WORLD_BOXES	  5
#define CTEST_WORLD_CLIPNODES (CTEST_WORLD_BOXES * 6)

typedef struct
{
	vec3_t mins, maxs;
	int	   contents;
	int	   open; /* shrink instead of grow: this box is a void, not a brush */
} ctest_world_box_t;

static const ctest_world_box_t ctest_world_boxes[CTEST_WORLD_BOXES] = {
	{{-256, -256, -256}, {-64, -64, -64}, CONTENTS_WATER, 0},
	{{-256, 128, -256}, {-128, 256, -128}, CONTENTS_LAVA, 0},
	{{128, -256, -256}, {256, -128, -128}, CONTENTS_CURRENT_0, 0},
	{{32, 32, -256}, {96, 96, 256}, CONTENTS_SOLID, 0},
	{{-448, -448, -192}, {448, 448, 192}, CONTENTS_EMPTY, 1},
};

static const vec3_t ctest_world_hullmins[MAX_MAP_HULLS] = {
	{0, 0, 0},
	{-16, -16, -24},
	{-32, -32, -24},
	{0, 0, 0},
};
static const vec3_t ctest_world_hullmaxs[MAX_MAP_HULLS] = {
	{0, 0, 0},
	{16, 16, 32},
	{32, 32, 64},
	{0, 0, 0},
};

static mclipnode_t ctest_world_clipnodes[MAX_MAP_HULLS][CTEST_WORLD_CLIPNODES];
static mplane_t	   ctest_world_planes[MAX_MAP_HULLS][CTEST_WORLD_CLIPNODES];
static qmodel_t	   ctest_world_bmodel;
static qmodel_t	   ctest_world_alias_model;
static mnode_t	   ctest_world_nodes[3];
static mleaf_t	   ctest_world_leafs[5];
static mplane_t	   ctest_world_nodeplanes[2];

static void ctest_world_build_hull (int h)
{
	mclipnode_t *nodes = ctest_world_clipnodes[h];
	mplane_t	*planes = ctest_world_planes[h];
	int			 i, a;

	memset (nodes, 0, sizeof (ctest_world_clipnodes[h]));
	memset (planes, 0, sizeof (ctest_world_planes[h]));

	for (i = 0; i < CTEST_WORLD_BOXES; i++)
	{
		const ctest_world_box_t *box = &ctest_world_boxes[i];
		int						 base = i * 6;
		int						 out = (i + 1 < CTEST_WORLD_BOXES) ? (i + 1) * 6 : CONTENTS_SOLID;
		vec3_t					 emins, emaxs;

		for (a = 0; a < 3; a++)
		{
			if (box->open)
			{
				emins[a] = box->mins[a] - ctest_world_hullmins[h][a];
				emaxs[a] = box->maxs[a] - ctest_world_hullmaxs[h][a];
			}
			else
			{
				emins[a] = box->mins[a] - ctest_world_hullmaxs[h][a];
				emaxs[a] = box->maxs[a] - ctest_world_hullmins[h][a];
			}
		}

		for (a = 0; a < 3; a++)
		{
			int hi = base + a * 2;
			int lo = hi + 1;

			planes[hi].normal[a] = 1;
			planes[hi].dist = emaxs[a];
			planes[hi].type = (byte)a;
			planes[lo].normal[a] = 1;
			planes[lo].dist = emins[a];
			planes[lo].type = (byte)a;

			nodes[hi].planenum = hi;
			nodes[hi].children[0] = out; /* at or past maxs: outside the box */
			nodes[hi].children[1] = lo;

			nodes[lo].planenum = lo;
			nodes[lo].children[0] = (a == 2) ? box->contents : (lo + 1);
			nodes[lo].children[1] = out; /* below mins: outside the box */
		}
	}
}

static void ctest_world_build_model (void)
{
	int h;

	memset (&ctest_world_bmodel, 0, sizeof (ctest_world_bmodel));
	memset (&ctest_world_alias_model, 0, sizeof (ctest_world_alias_model));
	memset (ctest_world_nodes, 0, sizeof (ctest_world_nodes));
	memset (ctest_world_leafs, 0, sizeof (ctest_world_leafs));
	memset (ctest_world_nodeplanes, 0, sizeof (ctest_world_nodeplanes));

	q_strlcpy (ctest_world_bmodel.name, "*ctest_world", sizeof (ctest_world_bmodel.name));
	ctest_world_bmodel.type = mod_brush;
	ctest_world_bmodel.is_worldmodel = true;
	/* the areanode tree is built from these: 4096 wide keeps size[axis] >= 500
	 * through depth 8, so sv_fte_createareanode=1 really does build a deeper
	 * tree (511 nodes) than the vanilla depth-4 one (31) */
	ctest_world_bmodel.mins[0] = ctest_world_bmodel.mins[1] = -2048;
	ctest_world_bmodel.mins[2] = -1024;
	ctest_world_bmodel.maxs[0] = ctest_world_bmodel.maxs[1] = 2048;
	ctest_world_bmodel.maxs[2] = 1024;

	for (h = 0; h < MAX_MAP_HULLS; h++)
	{
		hull_t *hull = &ctest_world_bmodel.hulls[h];
		ctest_world_build_hull (h);
		hull->clipnodes = ctest_world_clipnodes[h];
		hull->planes = ctest_world_planes[h];
		hull->firstclipnode = 0;
		hull->lastclipnode = CTEST_WORLD_CLIPNODES - 1;
		VectorCopy (ctest_world_hullmins[h], hull->clip_mins);
		VectorCopy (ctest_world_hullmaxs[h], hull->clip_maxs);
	}

	/* the node/leaf tree SV_FindTouchedLeafs walks: two splits, four leafs,
	 * plus leafs[0] standing in for the solid leaf a real BSP keeps at index
	 * 0 (SV_FindTouchedLeafs numbers leafs from -1). */
	ctest_world_nodeplanes[0].normal[0] = 1;
	ctest_world_nodeplanes[0].dist = 0;
	ctest_world_nodeplanes[0].type = 0;
	ctest_world_nodeplanes[1].normal[1] = 1;
	ctest_world_nodeplanes[1].dist = 0;
	ctest_world_nodeplanes[1].type = 1;

	ctest_world_leafs[0].contents = CONTENTS_SOLID;
	ctest_world_leafs[1].contents = CONTENTS_EMPTY;
	ctest_world_leafs[2].contents = CONTENTS_EMPTY;
	ctest_world_leafs[3].contents = CONTENTS_EMPTY;
	ctest_world_leafs[4].contents = CONTENTS_WATER;

	ctest_world_nodes[0].contents = 0;
	ctest_world_nodes[0].plane = &ctest_world_nodeplanes[0];
	ctest_world_nodes[0].children[0] = &ctest_world_nodes[1];
	ctest_world_nodes[0].children[1] = &ctest_world_nodes[2];
	ctest_world_nodes[1].contents = 0;
	ctest_world_nodes[1].plane = &ctest_world_nodeplanes[1];
	ctest_world_nodes[1].children[0] = (mnode_t *)&ctest_world_leafs[1];
	ctest_world_nodes[1].children[1] = (mnode_t *)&ctest_world_leafs[2];
	ctest_world_nodes[2].contents = 0;
	ctest_world_nodes[2].plane = &ctest_world_nodeplanes[1];
	ctest_world_nodes[2].children[0] = (mnode_t *)&ctest_world_leafs[3];
	ctest_world_nodes[2].children[1] = (mnode_t *)&ctest_world_leafs[4];

	ctest_world_bmodel.nodes = ctest_world_nodes;
	ctest_world_bmodel.numnodes = 3;
	ctest_world_bmodel.leafs = ctest_world_leafs;
	ctest_world_bmodel.numleafs = 4;
	ctest_world_bmodel.clipnodes = ctest_world_clipnodes[0];
	ctest_world_bmodel.numclipnodes = CTEST_WORLD_CLIPNODES;
	ctest_world_bmodel.planes = ctest_world_planes[0];
	ctest_world_bmodel.numplanes = CTEST_WORLD_CLIPNODES;

	/* modelindex 2: a non-brush model, so SV_HullForEntity's
	 * SOLID_BSP-with-a-non-bsp-model warning path is reachable */
	q_strlcpy (ctest_world_alias_model.name, "progs/ctest.mdl", sizeof (ctest_world_alias_model.name));
	ctest_world_alias_model.type = mod_alias;
}

static qmodel_t *ctest_world_getmodel (int modelindex)
{
	if (modelindex == 1)
		return &ctest_world_bmodel;
	if (modelindex == 2)
		return &ctest_world_alias_model;
	return NULL;
}

/* --- the progs image whose touch functions run ---------------------------
 *
 * pr_exec.c cannot dispatch a builtin at top level (world.c:365 hands
 * PR_ExecuteProgram a func_t, and PR_ExecuteProgram's own comment says a
 * builtin there would crash), so each touch function is two real statements:
 * OP_CALL0 into a logging builtin, then OP_DONE. Both sides run this same
 * c_ref interpreter, so the log isolates world.c's own ordering logic.
 */

#define CTEST_WORLD_TOUCH_KINDS 6

/* spare arena slots above num_edicts, for CTEST_WORLD_TOUCH_SPAWN */
#define CTEST_WORLD_SPAWN_HEADROOM 8

/* touch kinds; the test's own enum mirrors these */
#define CTEST_WORLD_TOUCH_LOG	   0 /* record (self, other, time) only */
#define CTEST_WORLD_TOUCH_RELINK   1 /* record, then relink ctest_world_relink_target */
#define CTEST_WORLD_TOUCH_FREE	   2 /* record, then free ctest_world_free_target */
#define CTEST_WORLD_TOUCH_FREESELF 3 /* record, then free `other` (the linking edict) */
#define CTEST_WORLD_TOUCH_SPAWN	   4 /* record, then ED_Alloc a live pushable edict */
#define CTEST_WORLD_TOUCH_SETSELF  5 /* record, then rewrite self's movetype/nextthink */

#define CTEST_WORLD_TOUCH_LOG_MAX 256

typedef struct
{
	int	  self;
	int	  other;
	float time;
	int	  kind;
} ctest_world_touch_rec_t;

static ctest_world_touch_rec_t ctest_world_touch_log[CTEST_WORLD_TOUCH_LOG_MAX];
static int					   ctest_world_touch_log_count;

static int ctest_world_relink_target = -1;
static int ctest_world_free_target = -1;

static void (*ctest_world_link_fn) (edict_t *ent, qboolean touch_triggers);
static void (*ctest_world_unlink_fn) (edict_t *ent);

static void ctest_world_default_link (edict_t *ent, qboolean touch_triggers)
{
	SV_LinkEdict (ent, touch_triggers); /* renamed c_ref_SV_LinkEdict */
}

static void ctest_world_default_unlink (edict_t *ent)
{
	SV_UnlinkEdict (ent); /* renamed c_ref_SV_UnlinkEdict */
}

/* Installs the implementation a re-entrant touch handler reaches. NULL
 * restores the c_ref pair, which is what the oracle self-check uses. */
void ctest_world_set_link_fns (void (*link) (edict_t *, qboolean), void (*unlink) (edict_t *))
{
	ctest_world_link_fn = link ? link : ctest_world_default_link;
	ctest_world_unlink_fn = unlink ? unlink : ctest_world_default_unlink;
}

static void ctest_world_record_touch (int kind)
{
	if (ctest_world_touch_log_count < CTEST_WORLD_TOUCH_LOG_MAX)
	{
		ctest_world_touch_rec_t *r = &ctest_world_touch_log[ctest_world_touch_log_count];
		r->self = pr_global_struct->self ? NUM_FOR_EDICT (PROG_TO_EDICT (pr_global_struct->self)) : 0;
		r->other = pr_global_struct->other ? NUM_FOR_EDICT (PROG_TO_EDICT (pr_global_struct->other)) : 0;
		r->time = pr_global_struct->time;
		r->kind = kind;
	}
	ctest_world_touch_log_count++;
}

static void ctest_world_free_edict (int num)
{
	edict_t *ed;
	if (num < 0 || num >= qcvm->num_edicts)
		return;
	ed = EDICT_NUM (num);
	if (ed->free)
		return;
	ctest_world_unlink_fn (ed);
	ed->v.model = 0;
	ed->v.takedamage = 0;
	ed->v.modelindex = 0;
	ed->v.colormap = 0;
	ed->v.skin = 0;
	ed->v.frame = 0;
	VectorCopy (vec3_origin, ed->v.origin);
	VectorCopy (vec3_origin, ed->v.angles);
	ed->v.nextthink = -1;
	ed->v.solid = 0;
	ed->freetime = (float)qcvm->time;
	ed->free = true;
}

static void ctest_world_builtin_log (void)
{
	ctest_world_record_touch (CTEST_WORLD_TOUCH_LOG);
}

static void ctest_world_builtin_relink (void)
{
	ctest_world_record_touch (CTEST_WORLD_TOUCH_RELINK);
	if (ctest_world_relink_target >= 0 && ctest_world_relink_target < qcvm->num_edicts)
	{
		edict_t *ed = EDICT_NUM (ctest_world_relink_target);
		if (!ed->free)
		{
			ed->v.origin[0] += 8;
			ctest_world_link_fn (ed, false);
		}
	}
}

static void ctest_world_builtin_free (void)
{
	ctest_world_record_touch (CTEST_WORLD_TOUCH_FREE);
	ctest_world_free_edict (ctest_world_free_target);
}

static void ctest_world_builtin_freeself (void)
{
	ctest_world_record_touch (CTEST_WORLD_TOUCH_FREESELF);
	if (pr_global_struct->other)
		ctest_world_free_edict (NUM_FOR_EDICT (PROG_TO_EDICT (pr_global_struct->other)));
}

/* --- the mid-tick allocator, and why it exists ---------------------------
 *
 * sv_phys.c:2397 installs SV_Physics_Alloc_Hook for the whole of SV_Physics'
 * entity loop, so an edict allocated by QC *during* a tick is appended to
 * pushable_ent_cache and can still be pushed by a pusher processed later in
 * the same loop (sv_phys.c:254 splices the cache tail onto every grid
 * query). Nothing else in the fixture ever calls ED_Alloc, so without this
 * builtin the hook is dead code on both sides and a Rust port that never
 * installed it would look identical to one that did.
 *
 * The allocation is budgeted because the builtin runs from whatever QC entry
 * point the test points at it, which may fire more than once per tick. */
static int	  ctest_world_spawn_budget;
static int	  ctest_world_spawn_count;
static int	  ctest_world_spawned_num = -1;
static float  ctest_world_spawn_origin[3];
static int	  ctest_world_spawn_ground = -1;

static void ctest_world_builtin_spawn (void)
{
	edict_t *ed;

	ctest_world_record_touch (CTEST_WORLD_TOUCH_SPAWN);
	if (ctest_world_spawn_count >= ctest_world_spawn_budget)
		return;
	ctest_world_spawn_count++;

	ed = ED_Alloc (); /* renamed c_ref_ED_Alloc -- fires the installed hook */
	ctest_world_spawned_num = NUM_FOR_EDICT (ed);

	ed->v.classname = 1;
	ed->v.movetype = MOVETYPE_STEP; /* SV_IsPushable accepts it, sv_phys.c:101 */
	ed->v.solid = SOLID_SLIDEBOX;
	ed->v.modelindex = 2;
	VectorCopy (ctest_world_spawn_origin, ed->v.origin);
	ed->v.mins[0] = ed->v.mins[1] = -16;
	ed->v.mins[2] = -24;
	ed->v.maxs[0] = ed->v.maxs[1] = 16;
	ed->v.maxs[2] = 32;
	VectorSubtract (ed->v.maxs, ed->v.mins, ed->v.size);
	/* riding the given pusher: SV_EntityRidingPusher (sv_phys.c:1153) wants
	 * FL_ONGROUND and groundentity, which is what puts the new edict on the
	 * pusher's candidate list for real rather than merely in the cache */
	if (ctest_world_spawn_ground >= 0 && ctest_world_spawn_ground < qcvm->num_edicts)
	{
		ed->v.flags = (float)((int)ed->v.flags | FL_ONGROUND);
		ed->v.groundentity = EDICT_TO_PROG (EDICT_NUM (ctest_world_spawn_ground));
	}
	ctest_world_link_fn (ed, false);
}

/* budget <= 0 disables the allocation but keeps the touch record */
void ctest_world_set_spawn (int budget, const float *origin, int ground)
{
	ctest_world_spawn_budget = budget;
	ctest_world_spawn_count = 0;
	ctest_world_spawned_num = -1;
	ctest_world_spawn_ground = ground;
	ctest_world_spawn_origin[0] = origin[0];
	ctest_world_spawn_origin[1] = origin[1];
	ctest_world_spawn_origin[2] = origin[2];
}

/* the edict number the last ctest_world_builtin_spawn handed out, or -1 */
int ctest_world_spawned_edict (void)
{
	return ctest_world_spawned_num;
}

int ctest_world_spawn_calls (void)
{
	return ctest_world_spawn_count;
}

/* --- QC that rewrites its own entity mid-dispatch -------------------------
 *
 * SV_Physics dispatches on ent->v.movetype and then, after the handler has
 * run whatever QC the tick reached, re-reads ent->v.movetype for the
 * sendinterval test (sv_phys.c:2444). A think that changes its own movetype
 * is therefore observable in ent->sendinterval without changing which
 * physics handler ran, and nothing else in the fixture can produce that. */
static int	 ctest_world_setself_armed;
static float ctest_world_setself_movetype;
static float ctest_world_setself_think_delay;

static void ctest_world_builtin_setself (void)
{
	ctest_world_record_touch (CTEST_WORLD_TOUCH_SETSELF);
	if (!ctest_world_setself_armed || !pr_global_struct->self)
		return;
	{
		edict_t *ed = PROG_TO_EDICT (pr_global_struct->self);
		if (ed->free)
			return;
		ed->v.movetype = ctest_world_setself_movetype;
		ed->v.nextthink = (float)qcvm->time + ctest_world_setself_think_delay;
	}
}

/* armed == 0 keeps the touch record but leaves the entity alone, so a test
 * can hold the QC dispatch log fixed and vary only the write */
void ctest_world_set_selfpoke (int armed, float movetype, float think_delay)
{
	ctest_world_setself_armed = armed;
	ctest_world_setself_movetype = movetype;
	ctest_world_setself_think_delay = think_delay;
}

static qcvm_t		ctest_world_sv_vm;
static dprograms_t	ctest_world_progs_hdr;
static dstatement_t ctest_world_statements[CTEST_WORLD_TOUCH_KINDS * 2];
static dfunction_t	ctest_world_functions[1 + CTEST_WORLD_TOUCH_KINDS * 2];
static char			ctest_world_strings[64];
static int			ctest_world_numglobals;
static int			ctest_world_builtin_ofs;

/* the func_t to store in an edict's v.touch for a given touch kind */
int ctest_world_touch_func (int kind)
{
	if (kind < 0 || kind >= CTEST_WORLD_TOUCH_KINDS)
		return 0;
	return 1 + kind;
}

int ctest_world_touch_log_len (void)
{
	return ctest_world_touch_log_count;
}

int ctest_world_touch_log_get (int i, int *self, int *other, float *time, int *kind)
{
	if (i < 0 || i >= ctest_world_touch_log_count || i >= CTEST_WORLD_TOUCH_LOG_MAX)
		return 0;
	*self = ctest_world_touch_log[i].self;
	*other = ctest_world_touch_log[i].other;
	*time = ctest_world_touch_log[i].time;
	*kind = ctest_world_touch_log[i].kind;
	return 1;
}

void ctest_world_touch_log_clear (void)
{
	ctest_world_touch_log_count = 0;
	memset (ctest_world_touch_log, 0, sizeof (ctest_world_touch_log));
}

void ctest_world_set_relink_target (int num)
{
	ctest_world_relink_target = num;
}

void ctest_world_set_free_target (int num)
{
	ctest_world_free_target = num;
}

/* --- sv_phys.c / client-VM seams -----------------------------------------
 *
 * Phase 7 M4: SV_PushGridEntityLinked used to be a stub-owned logger here,
 * because sv_phys.c was not one of build.rs's C_SOURCES and world.c:495 had
 * nothing else to call. It is now, so the real definition (sv_phys.c:200)
 * links under c_ref_SV_PushGridEntityLinked and the plain name belongs to
 * the Rust port. There is consequently no interceptable seam left on the C
 * side, and the M3 pushgrid log had to go with it -- see the M4 report.
 * World_Glue_PushGridEntityLinked now forwards to the Rust export and lives
 * at the bottom of this file, where the rename macro can be #undef'd. */

typedef struct
{
	edict_t *touch;
	edict_t *other;
	float	 time;
} ctest_world_calltouch_args_t;

static void ctest_world_invoke_touch (void *p)
{
	ctest_world_calltouch_args_t *a = (ctest_world_calltouch_args_t *)p;

	pr_global_struct->self = EDICT_TO_PROG (a->touch);
	pr_global_struct->other = EDICT_TO_PROG (a->other);
	pr_global_struct->time = a->time;
	PR_ExecuteProgram (a->touch->v.touch); /* renamed c_ref_PR_ExecuteProgram */
}

/* world_glue.c's guarded touch dispatch (the contract's World_Glue_CallTouch).
 * Parameter order follows world.c:362-365: the first edict is the toucher --
 * it becomes pr_global_struct->self and its v.touch is the function that runs
 * -- and the second is the edict being linked, which becomes
 * pr_global_struct->other. Only the dispatch is here; the list, the bbox
 * re-tests and the self/other save/restore stay on the caller's side, in
 * Rust. Returns 0, or the Host_Guard result for the caller to re-raise. */
int World_Glue_CallTouch (edict_t *touch, edict_t *self, float time)
{
	ctest_world_calltouch_args_t args;
	args.touch = touch;
	args.other = self;
	args.time = time;
	return Host_Guard (ctest_world_invoke_touch, &args);
}

int World_Glue_QcvmIsClient (void)
{
	return qcvm == &cl.qcvm;
}

int World_Glue_ClNumEntities (void)
{
	return cl.num_entities;
}

entity_t *World_Glue_ClEntity (int i)
{
	if (i < 0 || i >= cl.num_entities || !cl.entities)
		return NULL;
	return &cl.entities[i];
}

/* --- fixture setup / inspection ------------------------------------------ */

#define CTEST_WORLD_MAX_CL_ENTITIES 16
static entity_t ctest_world_cl_entities[CTEST_WORLD_MAX_CL_ENTITIES];

/* defined at the very bottom of this file, where the prelude's rename macros
 * can be #undef'd without affecting anything else */
void ctest_world_set_plain_cvars (float recursivehullcheck, float createareanode);

void ctest_world_set_cvars (float recursivehullcheck, float createareanode, float checkextension)
{
	c_ref_sv_fte_recursivehullckeck.value = recursivehullcheck;
	c_ref_sv_fte_createareanode.value = createareanode;
	ctest_world_set_plain_cvars (recursivehullcheck, createareanode);
	pr_checkextension.value = checkextension;
}

/* Rebuilds the whole fixture: model, progs image, arena, client entities and
 * every log. `vm_kind` picks which VM is published as the ambient one:
 *   0 -- a standalone server-shaped VM (the M3 default)
 *   1 -- cl.qcvm, which turns on SV_Move's World_ClipToNetwork branch
 *   2 -- sv.qcvm (Phase 7 M4): sv_phys.c gates its whole server-only half on
 *        `qcvm == &sv.qcvm` (pusher support frame, sv_speeds timing, client
 *        physics, sv_freezenonclients), so the physics differential has to
 *        run on the real sv.qcvm instance rather than a look-alike. */
void ctest_world_reset (int vm_kind, int num_edicts)
{
	qcvm_t *vm = (vm_kind == 1) ? &cl.qcvm : (vm_kind == 2) ? &sv.qcvm : &ctest_world_sv_vm;
	int		i;

	ctest_world_build_model ();

	if (vm->edicts)
		Mem_Free (vm->edicts);
	if (vm->globals)
		Mem_Free (vm->globals);
	memset (vm, 0, sizeof (*vm));
	memset (&ctest_world_progs_hdr, 0, sizeof (ctest_world_progs_hdr));
	memset (ctest_world_statements, 0, sizeof (ctest_world_statements));
	memset (ctest_world_functions, 0, sizeof (ctest_world_functions));
	memset (ctest_world_strings, 0, sizeof (ctest_world_strings));
	memcpy (ctest_world_strings + 1, "ctest_ent", 10);

	ctest_world_numglobals = (int)(sizeof (globalvars_t) / 4) + 8;
	ctest_world_builtin_ofs = (int)(sizeof (globalvars_t) / 4);

	for (i = 0; i < CTEST_WORLD_TOUCH_KINDS; i++)
	{
		ctest_world_statements[i * 2 + 0].op = OP_CALL0;
		ctest_world_statements[i * 2 + 0].a = (unsigned short)(ctest_world_builtin_ofs + i);
		ctest_world_statements[i * 2 + 1].op = OP_DONE;

		ctest_world_functions[1 + i].first_statement = i * 2;
		ctest_world_functions[1 + i].parm_start = 0;
		ctest_world_functions[1 + i].locals = 0;
		ctest_world_functions[1 + i].numparms = 0;
		ctest_world_functions[1 + i].s_name = 1;

		/* the builtin entries OP_CALL0 targets */
		ctest_world_functions[1 + CTEST_WORLD_TOUCH_KINDS + i].first_statement = -(i + 1);
		ctest_world_functions[1 + CTEST_WORLD_TOUCH_KINDS + i].s_name = 1;
	}

	ctest_world_progs_hdr.entityfields = (int)(sizeof (entvars_t) / 4);
	ctest_world_progs_hdr.numfunctions = 1 + CTEST_WORLD_TOUCH_KINDS * 2;
	ctest_world_progs_hdr.numstatements = CTEST_WORLD_TOUCH_KINDS * 2;
	ctest_world_progs_hdr.numglobals = ctest_world_numglobals;

	vm->progs = &ctest_world_progs_hdr;
	vm->statements = ctest_world_statements;
	vm->functions = ctest_world_functions;
	vm->strings = ctest_world_strings;
	vm->stringssize = (int)sizeof (ctest_world_strings);
	vm->globals = (float *)Mem_Alloc ((size_t)ctest_world_numglobals * sizeof (float));

	vm->builtins[0] = ctest_world_builtin_log;
	vm->builtins[1] = ctest_world_builtin_log;
	vm->builtins[2] = ctest_world_builtin_relink;
	vm->builtins[3] = ctest_world_builtin_free;
	vm->builtins[4] = ctest_world_builtin_freeself;
	vm->builtins[5] = ctest_world_builtin_spawn;
	vm->builtins[6] = ctest_world_builtin_setself;
	vm->numbuiltins = 1 + CTEST_WORLD_TOUCH_KINDS;

	vm->edict_size = ctest_world_progs_hdr.entityfields * 4 + (int)sizeof (edict_t) - (int)sizeof (entvars_t);
	vm->edict_size += (int)sizeof (void *) - 1;
	vm->edict_size &= ~((int)sizeof (void *) - 1);
	/* Headroom above num_edicts: ED_Alloc's grow path Host_Errors the moment
	 * num_edicts reaches max_edicts, so CTEST_WORLD_TOUCH_SPAWN would raise
	 * instead of allocating on an exactly-sized arena. num_edicts itself is
	 * unchanged, so every other test sees the same live entity count it did
	 * before, and the spare slots stay zeroed until something claims one. */
	vm->max_edicts = num_edicts + CTEST_WORLD_SPAWN_HEADROOM;
	vm->num_edicts = num_edicts;
	vm->edicts = (edict_t *)Mem_Alloc ((size_t)vm->max_edicts * vm->edict_size);
	vm->time = 4.5;
	vm->worldmodel = &ctest_world_bmodel;
	vm->GetModel = ctest_world_getmodel;

	qcvm = vm;
	pr_global_struct = (globalvars_t *)vm->globals;

	for (i = 0; i < CTEST_WORLD_TOUCH_KINDS; i++)
		((int *)vm->globals)[ctest_world_builtin_ofs + i] = 1 + CTEST_WORLD_TOUCH_KINDS + i;

	/* the world edict: SOLID_BSP on the brush model, exactly as sv_main.c
	 * leaves it, so SV_Move's clip-to-world step has something to trace */
	{
		edict_t *world = EDICT_NUM (0);
		world->v.solid = SOLID_BSP;
		world->v.movetype = MOVETYPE_PUSH;
		world->v.modelindex = 1;
		world->v.classname = 1;
	}
	for (i = 1; i < num_edicts; i++)
		EDICT_NUM (i)->v.classname = 1;

	memset (ctest_world_cl_entities, 0, sizeof (ctest_world_cl_entities));
	cl.entities = ctest_world_cl_entities;
	cl.num_entities = 0;
	cl.worldmodel = &ctest_world_bmodel;

	ctest_world_touch_log_clear ();
	ctest_world_relink_target = -1;
	ctest_world_free_target = -1;
	{
		static const float zero[3] = {0, 0, 0};
		ctest_world_set_spawn (0, zero, -1);
	}
	ctest_world_set_selfpoke (0, 0, 0);
	ctest_world_set_link_fns (NULL, NULL);
	ctest_world_set_cvars (1.0f, 1.0f, 1.0f);
	ctest_clear_con_log ();
}

void *ctest_world_qcvm (void)
{
	return qcvm;
}

void *ctest_world_edict (int num)
{
	return EDICT_NUM (num);
}

void *ctest_world_hull (int hullnum)
{
	return &ctest_world_bmodel.hulls[hullnum];
}

void *ctest_world_model (void)
{
	return &ctest_world_bmodel;
}

/* Configures one edict. touch_kind < 0 leaves v.touch at 0, which is how a
 * non-trigger entity is spelled. */
void ctest_world_edict_set (
	int num, float solid, float movetype, float modelindex, const float *origin, const float *mins, const float *maxs, const float *angles, float flags,
	int touch_kind, float skin, int owner, int is_free)
{
	edict_t *ed = EDICT_NUM (num);

	ed->v.solid = solid;
	ed->v.movetype = movetype;
	ed->v.modelindex = modelindex;
	VectorCopy (origin, ed->v.origin);
	VectorCopy (mins, ed->v.mins);
	VectorCopy (maxs, ed->v.maxs);
	VectorSubtract (maxs, mins, ed->v.size);
	VectorCopy (angles, ed->v.angles);
	ed->v.flags = flags;
	ed->v.touch = touch_kind >= 0 ? ctest_world_touch_func (touch_kind) : 0;
	ed->v.skin = skin;
	ed->v.owner = owner;
	ed->free = is_free ? true : false;
}

void ctest_world_edict_absbox (int num, float *out6)
{
	edict_t *ed = EDICT_NUM (num);
	out6[0] = ed->v.absmin[0];
	out6[1] = ed->v.absmin[1];
	out6[2] = ed->v.absmin[2];
	out6[3] = ed->v.absmax[0];
	out6[4] = ed->v.absmax[1];
	out6[5] = ed->v.absmax[2];
}

int ctest_world_edict_leafs (int num, int *out, int max)
{
	edict_t *ed = EDICT_NUM (num);
	int		 n = (int)ed->num_leafs;
	int		 i;
	for (i = 0; i < n && i < max; i++)
		out[i] = ed->leafnums[i];
	return n;
}

int ctest_world_edict_is_free (int num)
{
	return EDICT_NUM (num)->free ? 1 : 0;
}

/* Five ints per areanode, in index order: index, axis, dist (bit pattern),
 * child0 index, child1 index (-1 for a leaf). Returns the number of ints. */
int ctest_world_snapshot_areanodes (int *out, int max)
{
	int n = qcvm->numareanodes;
	int i, w = 0;

	for (i = 0; i < n; i++)
	{
		areanode_t *node = &qcvm->areanodes[i];
		float		dist = node->dist;
		int			bits;
		memcpy (&bits, &dist, sizeof (bits));
		if (w + 5 > max)
			break;
		out[w++] = i;
		out[w++] = node->axis;
		out[w++] = bits;
		out[w++] = node->children[0] ? (int)(node->children[0] - qcvm->areanodes) : -1;
		out[w++] = node->children[1] ? (int)(node->children[1] - qcvm->areanodes) : -1;
	}
	return w;
}

/* Four ints per linked edict, in chain order: areanode index, list (0 =
 * trigger_edicts, 1 = solid_edicts), position in that chain, edict number.
 * Chain order is the observable thing -- membership alone would not catch an
 * InsertLinkBefore that inserted on the wrong side. */
int ctest_world_snapshot_links (int *out, int max)
{
	int i, list, w = 0;

	for (i = 0; i < qcvm->numareanodes; i++)
	{
		areanode_t *node = &qcvm->areanodes[i];
		for (list = 0; list < 2; list++)
		{
			link_t *head = list ? &node->solid_edicts : &node->trigger_edicts;
			link_t *l;
			int		pos = 0;
			if (!head->next)
				continue; /* never cleared: an unbuilt tree */
			for (l = head->next; l != head; l = l->next)
			{
				if (w + 4 > max)
					return w;
				out[w++] = i;
				out[w++] = list;
				out[w++] = pos++;
				out[w++] = NUM_FOR_EDICT (EDICT_FROM_AREA (l));
				if (pos > qcvm->num_edicts)
					return w; /* corrupt chain: stop rather than spin */
			}
		}
	}
	return w;
}

/* Serializes a hull_t so the two implementations' private box hulls can be
 * compared without either side's pointer being meaningful. The ints are exact
 * in float here (clipnode indices and contents are tiny). */
int ctest_world_snapshot_hull (const void *hullp, float *out, int max)
{
	const hull_t *hull = (const hull_t *)hullp;
	int			  i, w = 0;

	if (w + 8 > max)
		return -1;
	out[w++] = (float)hull->firstclipnode;
	out[w++] = (float)hull->lastclipnode;
	out[w++] = hull->clip_mins[0];
	out[w++] = hull->clip_mins[1];
	out[w++] = hull->clip_mins[2];
	out[w++] = hull->clip_maxs[0];
	out[w++] = hull->clip_maxs[1];
	out[w++] = hull->clip_maxs[2];

	for (i = hull->firstclipnode; i <= hull->lastclipnode; i++)
	{
		const mclipnode_t *node = &hull->clipnodes[i];
		const mplane_t	  *plane = &hull->planes[node->planenum];
		if (w + 9 > max)
			return -1;
		out[w++] = (float)node->planenum;
		out[w++] = (float)node->children[0];
		out[w++] = (float)node->children[1];
		out[w++] = plane->normal[0];
		out[w++] = plane->normal[1];
		out[w++] = plane->normal[2];
		out[w++] = plane->dist;
		out[w++] = (float)plane->type;
		out[w++] = (float)plane->signbits;
	}
	return w;
}

/* --- the client VM's entity list (World_ClipToNetwork) -------------------- */

void ctest_world_cl_set_num_entities (int n)
{
	if (n < 0)
		n = 0;
	if (n > CTEST_WORLD_MAX_CL_ENTITIES)
		n = CTEST_WORLD_MAX_CL_ENTITIES;
	cl.num_entities = n;
}

void ctest_world_cl_set_entity (int i, int modelindex, unsigned int solidsize, const float *origin, const float *angles, int skinnum)
{
	entity_t *e;
	if (i < 0 || i >= CTEST_WORLD_MAX_CL_ENTITIES)
		return;
	e = &ctest_world_cl_entities[i];
	memset (e, 0, sizeof (*e));
	e->model = ctest_world_getmodel (modelindex);
	e->netstate.solidsize = solidsize;
	VectorCopy (origin, e->origin);
	VectorCopy (angles, e->angles);
	e->skinnum = skinnum;
}

/* trace_t's exact layout, so the test's #[repr(C)] mirror is checked against
 * the compiler's own view instead of being assumed (ADR-007/011 in spirit;
 * trace_t has no quake-types mirror, the mirror lives in the test file). */
int ctest_world_trace_layout (int *out, int max)
{
	if (max < 11)
		return 0;
	out[0] = (int)sizeof (trace_t);
	out[1] = (int)offsetof (trace_t, allsolid);
	out[2] = (int)offsetof (trace_t, startsolid);
	out[3] = (int)offsetof (trace_t, inopen);
	out[4] = (int)offsetof (trace_t, inwater);
	out[5] = (int)offsetof (trace_t, fraction);
	out[6] = (int)offsetof (trace_t, endpos);
	out[7] = (int)offsetof (trace_t, plane);
	out[8] = (int)offsetof (trace_t, plane.dist);
	out[9] = (int)offsetof (trace_t, ent);
	out[10] = (int)offsetof (trace_t, contents);
	return 11;
}

/* The rest of world_glue.c's surface (it is a Meson-only TU, so the harness
 * has to supply the same bodies for the Rust side to call). Kept
 * line-for-line equivalent to Quake/world_glue.c. */

typedef struct
{
	int		  num;
	edict_t **out;
} ctest_world_edictnum_arg_t;

static void ctest_world_invoke_edictnum (void *p)
{
	ctest_world_edictnum_arg_t *a = (ctest_world_edictnum_arg_t *)p;
	*a->out = EDICT_NUM (a->num);
}

int World_Glue_EdictNum (int num, edict_t **out)
{
	ctest_world_edictnum_arg_t arg;
	arg.num = num;
	arg.out = out;
	*out = NULL;
	return Host_Guard (ctest_world_invoke_edictnum, &arg);
}

typedef struct
{
	edict_t *ent;
	int		*out;
} ctest_world_numfor_arg_t;

static void ctest_world_invoke_numfor (void *p)
{
	ctest_world_numfor_arg_t *a = (ctest_world_numfor_arg_t *)p;
	*a->out = NUM_FOR_EDICT (a->ent);
}

int World_Glue_NumForEdict (edict_t *ent, int *out)
{
	ctest_world_numfor_arg_t arg;
	arg.ent = ent;
	arg.out = out;
	*out = 0;
	return Host_Guard (ctest_world_invoke_numfor, &arg);
}

typedef struct
{
	const char *expr;
	const char *file;
	int			line;
} ctest_world_assert_arg_t;

static void ctest_world_invoke_assert (void *p)
{
	ctest_world_assert_arg_t *a = (ctest_world_assert_arg_t *)p;
	COM_Assert_Failed (a->expr, a->file, a->line);
}

int World_Glue_AssertFailed (const char *expr, const char *file_path, int line)
{
	ctest_world_assert_arg_t arg;
	arg.expr = expr;
	arg.file = file_path;
	arg.line = line;
	return Host_Guard (ctest_world_invoke_assert, &arg);
}

void World_Glue_EntClipInfo (entity_t *e, unsigned int *solidsize, qmodel_t **model, vec3_t origin, vec3_t angles, int *skinnum)
{
	*solidsize = e->netstate.solidsize;
	*model = e->model;
	VectorCopy (e->origin, origin);
	VectorCopy (e->angles, angles);
	*skinnum = e->skinnum;
}

/* Both warn sites format PR_GetString (ent->v.classname), and PR_GetString
 * reaches Host_Error (Quake/pr_edict_arena.c:315) when the string_t is
 * negative and its knownstrings slot is NULL. That makes them raise-capable,
 * so per ADR-009 they are Host_Guard'd here and hand a status back for the
 * Rust caller to propagate; world_glue.c does the same in the real build. */
static void World_InvokeWarnSolidBspNoPush (void *p)
{
	edict_t *ent = (edict_t *)p;
	Con_Warning ("SOLID_BSP without MOVETYPE_PUSH (%s at %f %f %f)\n", PR_GetString (ent->v.classname), ent->v.origin[0], ent->v.origin[1], ent->v.origin[2]);
}

int World_Glue_WarnSolidBspNoPush (edict_t *ent)
{
	return Host_Guard (World_InvokeWarnSolidBspNoPush, ent);
}

static void World_InvokeWarnSolidBspNonBspModel (void *p)
{
	edict_t *ent = (edict_t *)p;
	Con_Warning ("SOLID_BSP with a non bsp model (%s at %f %f %f)\n", PR_GetString (ent->v.classname), ent->v.origin[0], ent->v.origin[1], ent->v.origin[2]);
}

int World_Glue_WarnSolidBspNonBspModel (edict_t *ent)
{
	return Host_Guard (World_InvokeWarnSolidBspNonBspModel, ent);
}

void World_Glue_DPrintBackupPast0 (void)
{
	Con_DPrintf ("backup past 0\n");
}

/* Pointer -> index, so a trace_t's `ent` and an areanode_t* can be compared
 * across two implementations that each hand back their own pointers. */
int ctest_world_edict_index (const void *p)
{
	const byte *base = (const byte *)qcvm->edicts;
	const byte *e = (const byte *)p;
	if (!p)
		return -1;
	if (e < base || e >= base + (size_t)qcvm->num_edicts * qcvm->edict_size)
		return -2;
	return (int)((e - base) / qcvm->edict_size);
}

int ctest_world_areanode_index (const void *p)
{
	const areanode_t *n = (const areanode_t *)p;
	if (!p)
		return -1;
	if (n < qcvm->areanodes || n >= qcvm->areanodes + AREA_NODES)
		return -2;
	return (int)(n - qcvm->areanodes);
}

int ctest_world_numareanodes (void)
{
	return qcvm->numareanodes;
}

/* Drives SV_CreateAreaNode directly (it appends to qcvm->areanodes, so the
 * counter has to be reset by hand -- SV_ClearWorld is the only caller in the
 * engine and does it there). */
void ctest_world_reset_areanodes (void)
{
	memset (qcvm->areanodes, 0, sizeof (qcvm->areanodes));
	qcvm->numareanodes = 0;
}

void ctest_world_world_bounds (float *out6)
{
	out6[0] = ctest_world_bmodel.mins[0];
	out6[1] = ctest_world_bmodel.mins[1];
	out6[2] = ctest_world_bmodel.mins[2];
	out6[3] = ctest_world_bmodel.maxs[0];
	out6[4] = ctest_world_bmodel.maxs[1];
	out6[5] = ctest_world_bmodel.maxs[2];
}

void *ctest_world_nodes_root (void)
{
	return ctest_world_bmodel.nodes;
}

void ctest_world_hull_arrays (const void *hullp, void **clipnodes, void **planes)
{
	const hull_t *hull = (const hull_t *)hullp;
	*clipnodes = (void *)hull->clipnodes;
	*planes = (void *)hull->planes;
}

int ctest_world_rhtctx_size (void)
{
	return (int)sizeof (struct rhtctx_s);
}

/* --- forcing PR_GetString to raise ----------------------------------------
 * PR_GetString (Quake/pr_edict_arena.c:307-326) calls Host_Error only for a
 * negative string_t inside the knownstrings range whose slot is NULL; every
 * other input returns without raising. ctest_world_reset leaves
 * numknownstrings at 0, so the fixture has to install a table before the
 * SV_HullForEntity warn sites can raise at all. */
static const char *ctest_world_knownstrings[4];

void ctest_world_arm_bad_classname (int num)
{
	int i;
	for (i = 0; i < 4; i++)
		ctest_world_knownstrings[i] = NULL;
	qcvm->knownstrings = ctest_world_knownstrings;
	qcvm->numknownstrings = 4;
	qcvm->maxknownstrings = 4;
	EDICT_NUM (num)->v.classname = -1; /* knownstrings[0], which is NULL */
}

/* ===========================================================================
 * Phase 7 M4: the sv_move.c / sv_phys.c differential fixture
 *
 * Layered on top of the M3 world fixture: ctest_phys_reset republishes the
 * same synthetic room and progs image on sv.qcvm (vm_kind 2), because
 * sv_phys.c gates its entire server-only half on `qcvm == &sv.qcvm`.
 * Everything below is shared by both sides of a differential unless the name
 * says otherwise (`_c` reads the c_ref_* storage, `_plain` the Rust one).
 * ===========================================================================
 */

/* --- engine seams the two new translation units link against --------------
 * None of these live in a build.rs C_SOURCES file, so the harness owns a
 * single shared definition and both sides call it. */

/* host.c:70 -- deliberately NOT renamed by the prelude: sv_phys.c's timing
 * blocks and host.c's report read one and the same cvar in the engine. */
cvar_t sv_speeds;

/* Quake/pr_edict.c, copied verbatim. SV_EntGravity (sv_phys.c:665) is the
 * only caller here; the fixture's progs image has no fielddefs, so
 * ED_FindField returns NULL, the offset is -1 and SV_EntGravity falls back
 * to 1.0 on both sides. */
int ED_FindFieldOffset (const char *name)
{
	ddef_t *def = ED_FindField (name); /* renamed c_ref_ED_FindField */
	if (!def)
		return -1;
	return def->ofs;
}

eval_t *GetEdictFieldValue (edict_t *ed, int fldofs)
{
	if (fldofs < 0)
		return NULL;
	return (eval_t *)((char *)&ed->v + fldofs * 4);
}

/* Quake/pr_cmds.c's PF_changeyaw, copied verbatim. sv_move.c:242 calls this
 * plain C symbol directly (it declares its own prototype at sv_move.c:235),
 * and pr_cmds.c is not in C_SOURCES, so the harness owns it. It reaches no
 * PR_ExecuteProgram and cannot raise, which is why the Rust port calls it
 * without a Host_Guard. */
void PF_changeyaw (void);
void PF_changeyaw (void)
{
	edict_t *ent;
	float	 ideal, current, move, speed;

	ent = PROG_TO_EDICT (pr_global_struct->self);
	current = anglemod (ent->v.angles[1]);
	ideal = ent->v.ideal_yaw;
	speed = ent->v.yaw_speed;

	if (current == ideal)
		return;
	move = ideal - current;
	if (ideal > current)
	{
		if (move >= 180)
			move = move - 360;
	}
	else
	{
		if (move <= -180)
			move = move + 360;
	}
	if (move > 0)
	{
		if (move > speed)
			move = speed;
	}
	else
	{
		if (move < -speed)
			move = -speed;
	}

	ent->v.angles[1] = anglemod (current + move);
}

/* host.c:185. Routed through the existing Host_Error trap so ctest_try_host
 * catches it; the "Host_EndGame: " prefix keeps the two raise paths
 * distinguishable in ctest_host_error_message(). */
static int ctest_phys_endgame_count;

FUNC_NORETURN void Host_EndGame (const char *message, ...)
{
	va_list ap;
	char	buf[1024];

	va_start (ap, message);
	vsnprintf (buf, sizeof (buf), message, ap);
	va_end (ap);

	ctest_phys_endgame_count++;
	Host_Error ("Host_EndGame: %s", buf);
}

int ctest_phys_endgame_calls (void)
{
	return ctest_phys_endgame_count;
}

/* Phase 7 M6: SV_StartSound (sv_main.c:277) used to be a stub-owned recorder
 * shared by the c_ref oracle and the Rust glue. sv_main.c is an oracle source
 * now, so both sides reach the real function (c_ref_SV_StartSound) and the
 * recorder is gone. Its observable moved to the console log, which both sides
 * already compare: the real function Con_Printf's
 * "SV_StartSound: <sample> not precacheed" for every call the fixture makes,
 * because the fixture never populates sv.sound_precache.
 *
 * FINDING (reported with T6.0): the M4 comment here claimed the real
 * SV_StartSound Host_Errors on an unprecached sample. It does not --
 * sv_main.c:307-311 Con_Printf's and returns. Its three Host_Error arms are
 * volume < 0, attenuation outside [0,4] and channel < 0, none of which
 * SV_CheckWaterTransition or SV_Physics_Step can reach (they pass literal
 * 255 / 1 / 0). SvPhys_Glue_StartSound stays guarded anyway, because the
 * Rust port may pass QC-derived arguments later. */

/* The pf_fx_ref.c oracle group is the one place where a stub-owned recorder is
 * still the right seam: that file hand-copies PF_particle/PF_sound rather than
 * compiling pr_cmds_sv_fx.c, so BOTH its oracle side and the Rust builtin can
 * be pointed at the same plain-named recorder with a local #undef (see the
 * #undef block at the top of pf_fx_ref.c). sv_phys.c cannot do that -- it IS
 * an oracle source, so its call renames to c_ref_SV_StartSound. */
#undef SV_StartSound

#define CTEST_PHYS_SOUND_MAX 64

typedef struct
{
	int	  ent;
	int	  channel;
	int	  volume;
	float attenuation;
	int	  has_origin;
	char  sample[64];
} ctest_phys_sound_rec_t;

static ctest_phys_sound_rec_t ctest_phys_sounds[CTEST_PHYS_SOUND_MAX];
static int					  ctest_phys_sound_count;
static int					  ctest_phys_sound_raises;

void SV_StartSound (edict_t *entity, float *origin, int channel, const char *sample, int volume, float attenuation)
{
	if (ctest_phys_sound_count < CTEST_PHYS_SOUND_MAX)
	{
		ctest_phys_sound_rec_t *r = &ctest_phys_sounds[ctest_phys_sound_count];
		r->ent = NUM_FOR_EDICT (entity);
		r->channel = channel;
		r->volume = volume;
		r->attenuation = attenuation;
		r->has_origin = origin ? 1 : 0;
		q_strlcpy (r->sample, sample ? sample : "", sizeof (r->sample));
	}
	ctest_phys_sound_count++;

	if (ctest_phys_sound_raises)
		Host_Error ("SV_StartSound: %s not precached", sample ? sample : "");
}

void ctest_phys_sound_arm_raise (int on)
{
	ctest_phys_sound_raises = on ? 1 : 0;
}

int ctest_phys_sound_len (void)
{
	return ctest_phys_sound_count;
}

int ctest_phys_sound_get (int i, int *ent, int *channel, int *volume, float *attenuation, int *has_origin, const char **sample)
{
	if (i < 0 || i >= ctest_phys_sound_count || i >= CTEST_PHYS_SOUND_MAX)
		return 0;
	*ent = ctest_phys_sounds[i].ent;
	*channel = ctest_phys_sounds[i].channel;
	*volume = ctest_phys_sounds[i].volume;
	*attenuation = ctest_phys_sounds[i].attenuation;
	*has_origin = ctest_phys_sounds[i].has_origin;
	*sample = ctest_phys_sounds[i].sample;
	return 1;
}

void ctest_phys_sound_clear (void)
{
	ctest_phys_sound_count = 0;
	memset (ctest_phys_sounds, 0, sizeof (ctest_phys_sounds));
}

/* Back to the oracle's name for the rest of this file: SvPhys_Glue_StartSound
 * below must reach the REAL implementation, because the C side of the sv_phys
 * differential (sv_phys.c, an oracle source) reaches it too. */
#define SV_StartSound c_ref_SV_StartSound

/* --- the sv_phys.c glue helpers -------------------------------------------
 * Quake/sv_phys_glue.c is not one of build.rs's C_SOURCES (it only compiles
 * under Meson's -Duse_rust_host), so the harness owns the same trampolines
 * under the same names. Every guarded one returns a Host_Guard status the
 * Rust caller propagates; ADR-009 keeps the longjmp inside C frames.
 *
 * DUPLICATE-SYMBOL HAZARD: if sv_phys_glue.c is ever added to C_SOURCES,
 * every SvPhys_Glue_* below AND the plain cvar/counter block at the bottom
 * of this file must be removed together. */

typedef struct
{
	edict_t	   *self;
	edict_t	   *other;
	float		time;
	int			channel;
	int			volume;
	float		attenuation;
	const char *sample;
} ctest_phys_glue_args_t;

static void ctest_phys_invoke_think (void *p)
{
	ctest_phys_glue_args_t *a = (ctest_phys_glue_args_t *)p;

	pr_global_struct->time = a->time;
	pr_global_struct->self = EDICT_TO_PROG (a->self);
	pr_global_struct->other = EDICT_TO_PROG (qcvm->edicts);
	PR_ExecuteProgram (a->self->v.think);
}

/* sv_phys.c:368-372 (SV_RunThink) and sv_phys.c:1609-1613 (SV_Physics_Pusher).
 * Both set time/self/other identically and dispatch ent->v.think; the
 * surrounding bookkeeping stays in Rust. */
int SvPhys_Glue_CallThink (edict_t *ent, float time)
{
	ctest_phys_glue_args_t a;
	memset (&a, 0, sizeof (a));
	a.self = ent;
	a.time = time;
	return Host_Guard (ctest_phys_invoke_think, &a);
}

static void ctest_phys_invoke_blocked (void *p)
{
	ctest_phys_glue_args_t *a = (ctest_phys_glue_args_t *)p;

	pr_global_struct->self = EDICT_TO_PROG (a->self);
	pr_global_struct->other = EDICT_TO_PROG (a->other);
	PR_ExecuteProgram (a->self->v.blocked);
}

/* sv_phys.c:1557-1562. Note C does NOT set pr_global_struct->time here. */
int SvPhys_Glue_CallBlocked (edict_t *pusher, edict_t *obstacle)
{
	ctest_phys_glue_args_t a;
	memset (&a, 0, sizeof (a));
	a.self = pusher;
	a.other = obstacle;
	return Host_Guard (ctest_phys_invoke_blocked, &a);
}

static void ctest_phys_invoke_prethink (void *p)
{
	ctest_phys_glue_args_t *a = (ctest_phys_glue_args_t *)p;

	pr_global_struct->time = a->time;
	pr_global_struct->self = EDICT_TO_PROG (a->self);
	PR_ExecuteProgram (pr_global_struct->PlayerPreThink);
}

/* sv_phys.c:2007-2009 */
int SvPhys_Glue_CallPlayerPreThink (edict_t *ent, float time)
{
	ctest_phys_glue_args_t a;
	memset (&a, 0, sizeof (a));
	a.self = ent;
	a.time = time;
	return Host_Guard (ctest_phys_invoke_prethink, &a);
}

static void ctest_phys_invoke_postthink (void *p)
{
	ctest_phys_glue_args_t *a = (ctest_phys_glue_args_t *)p;

	pr_global_struct->time = a->time;
	pr_global_struct->self = EDICT_TO_PROG (a->self);
	PR_ExecuteProgram (pr_global_struct->PlayerPostThink);
}

/* sv_phys.c:2065-2067 */
int SvPhys_Glue_CallPlayerPostThink (edict_t *ent, float time)
{
	ctest_phys_glue_args_t a;
	memset (&a, 0, sizeof (a));
	a.self = ent;
	a.time = time;
	return Host_Guard (ctest_phys_invoke_postthink, &a);
}

static void ctest_phys_invoke_startframe (void *p)
{
	ctest_phys_glue_args_t *a = (ctest_phys_glue_args_t *)p;

	pr_global_struct->self = EDICT_TO_PROG (qcvm->edicts);
	pr_global_struct->other = EDICT_TO_PROG (qcvm->edicts);
	pr_global_struct->time = a->time;
	PR_ExecuteProgram (pr_global_struct->StartFrame);
}

/* sv_phys.c:2333-2339 */
int SvPhys_Glue_CallStartFrame (float time)
{
	ctest_phys_glue_args_t a;
	memset (&a, 0, sizeof (a));
	a.time = time;
	return Host_Guard (ctest_phys_invoke_startframe, &a);
}

/* sv_phys.c:318 / :323. PR_GetString (pr_edict_arena.c:315) raises, so the
 * whole Con_DPrintf stays in C with the format string verbatim. */
static void ctest_phys_invoke_nan_velocity (void *p)
{
	edict_t *ent = (edict_t *)p;
	Con_DPrintf ("Got a NaN velocity on %s\n", PR_GetString (ent->v.classname));
}

int SvPhys_Glue_WarnNanVelocity (edict_t *ent)
{
	return Host_Guard (ctest_phys_invoke_nan_velocity, ent);
}

static void ctest_phys_invoke_nan_origin (void *p)
{
	edict_t *ent = (edict_t *)p;
	Con_DPrintf ("Got a NaN origin on %s\n", PR_GetString (ent->v.classname));
}

int SvPhys_Glue_WarnNanOrigin (edict_t *ent)
{
	return Host_Guard (ctest_phys_invoke_nan_origin, ent);
}

/* sv_phys.c:2055 and :2429 -- distinct strings, so two helpers. */
static void ctest_phys_invoke_endgame_client (void *p)
{
	Host_EndGame ("SV_Physics_client: bad movetype %i", *(int *)p);
}

int SvPhys_Glue_EndGameBadClientMovetype (int movetype)
{
	int m = movetype;
	return Host_Guard (ctest_phys_invoke_endgame_client, &m);
}

static void ctest_phys_invoke_endgame (void *p)
{
	Host_EndGame ("SV_Physics: bad movetype %i", *(int *)p);
}

int SvPhys_Glue_EndGameBadMovetype (int movetype)
{
	int m = movetype;
	return Host_Guard (ctest_phys_invoke_endgame, &m);
}

/* sv_phys.c:2139,:2148 (SV_CheckWaterTransition) and :2270
 * (SV_Physics_Step). SV_StartSound Host_Errors on an unprecached sample. */
static void ctest_phys_invoke_startsound (void *p)
{
	ctest_phys_glue_args_t *a = (ctest_phys_glue_args_t *)p;
	SV_StartSound (a->self, NULL, a->channel, a->sample, a->volume, a->attenuation);
}

int SvPhys_Glue_StartSound (edict_t *ent, int channel, const char *sample, int volume, float attenuation)
{
	ctest_phys_glue_args_t a;
	memset (&a, 0, sizeof (a));
	a.self = ent;
	a.channel = channel;
	a.sample = sample;
	a.volume = volume;
	a.attenuation = attenuation;
	return Host_Guard (ctest_phys_invoke_startsound, &a);
}

/* sv_phys.c:297. Unguarded: M3 established that plain Con_Printf does not
 * raise; only a PR_GetString argument makes one of these raise-capable. */
void SvPhys_Glue_PrintInvalidPosition (void)
{
	Con_Printf ("entity in invalid position\n");
}

/* sv_phys.c:413-443 (SV_Impact). NOT World_Glue_CallTouch: that helper stamps
 * pr_global_struct->time on every call, while SV_Impact stamps it once at
 * :419 and deliberately does not restamp between the two dispatches, so QC
 * that writes the `time` global inside e1's touch is observable inside e2's.
 * The single store stays on the caller's side; this only sets self/other and
 * dispatches self->v.touch. */
static void ctest_phys_invoke_impact_touch (void *p)
{
	ctest_phys_glue_args_t *a = (ctest_phys_glue_args_t *)p;

	pr_global_struct->self = EDICT_TO_PROG (a->self);
	pr_global_struct->other = EDICT_TO_PROG (a->other);
	PR_ExecuteProgram (a->self->v.touch);
}

int SvPhys_Glue_ImpactTouch (edict_t *self, edict_t *other)
{
	ctest_phys_glue_args_t a;
	memset (&a, 0, sizeof (a));
	a.self = self;
	a.other = other;
	return Host_Guard (ctest_phys_invoke_impact_touch, &a);
}

/* sv_phys.c:1254. Guarded because NUM_FOR_EDICT raises on an out-of-arena
 * edict; the format string stays in C because Rust cannot call variadic C. */
static void ctest_phys_invoke_dprint_unembedded (void *p)
{
	ctest_phys_glue_args_t *a = (ctest_phys_glue_args_t *)p;
	Con_DPrintf2 ("SV_PushEntityTo: un-embedded entity %i from pusher %i\n", NUM_FOR_EDICT (a->self), NUM_FOR_EDICT (a->other));
}

int SvPhys_Glue_DPrintUnembedded (edict_t *ent, edict_t *ground)
{
	ctest_phys_glue_args_t a;
	memset (&a, 0, sizeof (a));
	a.self = ent;
	a.other = ground;
	return Host_Guard (ctest_phys_invoke_dprint_unembedded, &a);
}

/* sv_phys.c:1655, :1669 and :1676. Constant strings, so unguarded. */
void SvPhys_Glue_DPrintUnstuck (void)
{
	Con_DPrintf ("Unstuck.\n");
}

void SvPhys_Glue_DPrintPlayerStuck (void)
{
	Con_DPrintf ("player is stuck.\n");
}

/* --- non-raising server-state accessors ----------------------------------
 * sv_phys.c gates its whole server-only half on `qcvm == &sv.qcvm` and reads
 * svs/sv_player directly. Those are C aggregates with no ADR-011 mirror in
 * Phase 7, so the port reaches them through these accessors. */
int SvPhys_Glue_QcvmIsServer (void)
{
	return qcvm == &sv.qcvm;
}

int SvPhys_Glue_MaxClients (void)
{
	return svs.maxclients;
}

/* `num` is the 1-based edict number of the client, as sv_phys.c:1996 and
 * :1999 use it (svs.clients[num - 1]). Out-of-range answers false rather
 * than reading past the array; C never asks outside 1..maxclients. */
int SvPhys_Glue_ClientActive (int num)
{
	if (!svs.clients || num < 1 || num > svs.maxclients)
		return 0;
	return svs.clients[num - 1].active ? 1 : 0;
}

int SvPhys_Glue_ClientKnownToQc (int num)
{
	if (!svs.clients || num < 1 || num > svs.maxclients)
		return 0;
	return svs.clients[num - 1].knowntoqc ? 1 : 0;
}

edict_t *SvPhys_Glue_SvPlayer (void)
{
	return sv_player;
}

/* --- the fixture ---------------------------------------------------------- */

#define CTEST_PHYS_MAX_CLIENTS 8
#define CTEST_PHYS_RAND_SEED   0x5EED4A17ull

static client_t ctest_phys_clients[CTEST_PHYS_MAX_CLIENTS];
static float	ctest_phys_mode_value;

/* defined at the bottom of this file, past the #undef's */
void ctest_phys_set_plain_cvars (const float *v);
void ctest_phys_zero_speeds_plain (void);

static void ctest_phys_zero_speeds_c (void)
{
	sv_speeds_think_ms = 0; /* every name in this function is renamed c_ref_* */
	sv_speeds_pusher_ms = 0;
	sv_speeds_build_ms = 0;
	sv_speeds_thinks = 0;
	sv_speeds_pushers = 0;
	sv_speeds_pushables = 0;
	sv_speeds_grid_entries = 0;
}

/* v[13]: friction, stopspeed, gravity, maxvelocity, nostep, freezenonclients,
 * spawnbeforethinks, bouncedownslopes, elevators, fastpushmove, pushgrid,
 * analyticphysics, speeds. Only .value is read by sv_phys.c/sv_move.c. */
void ctest_phys_set_cvars (const float *v)
{
	sv_friction.value = v[0]; /* every cvar here is renamed c_ref_* */
	sv_stopspeed.value = v[1];
	sv_gravity.value = v[2];
	sv_maxvelocity.value = v[3];
	sv_nostep.value = v[4];
	sv_freezenonclients.value = v[5];
	sv_gameplayfix_spawnbeforethinks.value = v[6];
	sv_gameplayfix_bouncedownslopes.value = v[7];
	sv_gameplayfix_elevators.value = v[8];
	sv_fastpushmove.value = v[9];
	sv_pushgrid.value = v[10];
	sv_analyticphysics.value = v[11];
	ctest_phys_set_plain_cvars (v);
	sv_speeds.value = v[12]; /* shared with host.c, not renamed */
}

/* Rebuilds the world fixture on sv.qcvm and clears every physics-side log.
 * `physics_mode` < 0 leaves qcvm->extglobals.physics_mode NULL, which is how
 * SV_Physics' `(qcvm == &cl.qcvm) ? 0 : 2` default is reached. */
void ctest_phys_reset (int num_edicts, int maxclients, double frametime, double vmtime, int physics_mode)
{
	int i;

	ctest_world_reset (2, num_edicts);
	sv.active = true;
	sv.state = ss_active;

	if (maxclients < 0)
		maxclients = 0;
	if (maxclients > CTEST_PHYS_MAX_CLIENTS)
		maxclients = CTEST_PHYS_MAX_CLIENTS;
	memset (ctest_phys_clients, 0, sizeof (ctest_phys_clients));
	for (i = 0; i < maxclients; i++)
	{
		ctest_phys_clients[i].active = true;
		ctest_phys_clients[i].knowntoqc = true;
	}
	svs.maxclients = maxclients;
	svs.clients = ctest_phys_clients;

	host_frametime = frametime;
	qcvm->time = vmtime;
	sv_player = EDICT_NUM (num_edicts > 1 ? 1 : 0);

	if (physics_mode < 0)
		qcvm->extglobals.physics_mode = NULL;
	else
	{
		ctest_phys_mode_value = (float)physics_mode;
		qcvm->extglobals.physics_mode = &ctest_phys_mode_value;
	}

	ctest_phys_endgame_count = 0;
	ctest_phys_sound_raises = 0;
	ctest_phys_sound_clear ();
	ctest_phys_zero_speeds_c ();
	ctest_phys_zero_speeds_plain ();
	COM_SeedRand (CTEST_PHYS_RAND_SEED);
	ctest_clear_con_log ();
}

/* Forces SV_BeginPusherSupportFrame's "the arena moved" branch (sv_phys.c:743
 * -- `sv_pusher_support_edicts != qcvm->edicts`). Publishing a freshly
 * allocated arena WITHOUT freeing the old one guarantees a different pointer;
 * reusing ctest_world_reset would not, because the allocator is free to hand
 * the same block back. The old arena is deliberately leaked: a test run makes
 * a handful of these calls.
 *
 * The new arena is zeroed, so every areanode chain now dangles. The caller
 * must re-apply its edict specs and re-run SV_ClearWorld/SV_LinkEdict before
 * touching the world again. */
void ctest_phys_swap_arena (int num_edicts)
{
	size_t sz;

	if (num_edicts < 1)
		num_edicts = 1;
	sz = (size_t)num_edicts * qcvm->edict_size;
	qcvm->edicts = (edict_t *)Mem_Alloc (sz);
	memset (qcvm->edicts, 0, sz);
	qcvm->max_edicts = num_edicts;
	qcvm->num_edicts = num_edicts;
	sv_player = EDICT_NUM (num_edicts > 1 ? 1 : 0);
}

void ctest_phys_set_client (int slot, int active, int knowntoqc)
{
	if (slot < 0 || slot >= CTEST_PHYS_MAX_CLIENTS)
		return;
	ctest_phys_clients[slot].active = active ? true : false;
	ctest_phys_clients[slot].knowntoqc = knowntoqc ? true : false;
}

/* Points StartFrame/PlayerPreThink/PlayerPostThink at the M3 logging touch
 * functions; kind < 0 leaves the global at 0 (no dispatch). */
void ctest_phys_set_prog_funcs (int startframe_kind, int prethink_kind, int postthink_kind, float force_retouch)
{
	pr_global_struct->StartFrame = startframe_kind >= 0 ? ctest_world_touch_func (startframe_kind) : 0;
	pr_global_struct->PlayerPreThink = prethink_kind >= 0 ? ctest_world_touch_func (prethink_kind) : 0;
	pr_global_struct->PlayerPostThink = postthink_kind >= 0 ? ctest_world_touch_func (postthink_kind) : 0;
	pr_global_struct->force_retouch = force_retouch;
}

/* scalars[16]: movetype, solid, modelindex, flags, waterlevel, watertype,
 *              groundentity (edict number, < 0 = none), nextthink, ltime,
 *              frame, owner (edict number, < 0 = none), yaw_speed,
 *              ideal_yaw, health, takedamage, skin
 * vectors[18]: origin[3], mins[3], maxs[3], velocity[3], angles[3],
 *              avelocity[3]
 * The three *_kind arguments select an M3 logging touch function (< 0 = 0). */
void ctest_phys_edict_set (int num, const float *scalars, const float *vectors, int think_kind, int touch_kind, int blocked_kind, int is_free)
{
	edict_t *ed = EDICT_NUM (num);

	ed->v.movetype = scalars[0];
	ed->v.solid = scalars[1];
	ed->v.modelindex = scalars[2];
	ed->v.flags = scalars[3];
	ed->v.waterlevel = scalars[4];
	ed->v.watertype = scalars[5];
	ed->v.groundentity = scalars[6] >= 0 ? (float)EDICT_TO_PROG (EDICT_NUM ((int)scalars[6])) : 0;
	ed->v.nextthink = scalars[7];
	ed->v.ltime = scalars[8];
	ed->v.frame = scalars[9];
	ed->v.owner = scalars[10] >= 0 ? (float)EDICT_TO_PROG (EDICT_NUM ((int)scalars[10])) : 0;
	ed->v.yaw_speed = scalars[11];
	ed->v.ideal_yaw = scalars[12];
	ed->v.health = scalars[13];
	ed->v.takedamage = scalars[14];
	ed->v.skin = scalars[15];

	VectorCopy (vectors + 0, ed->v.origin);
	VectorCopy (vectors + 3, ed->v.mins);
	VectorCopy (vectors + 6, ed->v.maxs);
	VectorSubtract (ed->v.maxs, ed->v.mins, ed->v.size);
	VectorCopy (vectors + 9, ed->v.velocity);
	VectorCopy (vectors + 12, ed->v.angles);
	VectorCopy (vectors + 15, ed->v.avelocity);

	ed->v.think = think_kind >= 0 ? ctest_world_touch_func (think_kind) : 0;
	ed->v.touch = touch_kind >= 0 ? ctest_world_touch_func (touch_kind) : 0;
	ed->v.blocked = blocked_kind >= 0 ? ctest_world_touch_func (blocked_kind) : 0;
	ed->free = is_free ? true : false;
}

/* sv_move.c reads three entvars the setter above does not cover, and none of
 * the five entry points writes them, so they get a separate setter rather
 * than widening the 16-float scalar block. `enemy` and `goalentity` are edict
 * numbers (< 0 = the world edict, which is what sv_move.c:408 and :129 test
 * against); `flags` is or'd, not replaced, so a test can add FL_ONGROUND to a
 * population entry without restating it. */
void ctest_phys_edict_set_refs (int num, int enemy, int goalentity, int extra_flags)
{
	edict_t *ed = EDICT_NUM (num);

	ed->v.enemy = (float)EDICT_TO_PROG (EDICT_NUM (enemy >= 0 ? enemy : 0));
	ed->v.goalentity = (float)EDICT_TO_PROG (EDICT_NUM (goalentity >= 0 ? goalentity : 0));
	ed->v.flags = (float)((int)ed->v.flags | extra_flags);
}

/* SV_MoveToGoal (sv_move.c:392-416) is a QC builtin body: it takes its actor
 * from pr_global_struct->self, its distance from OFS_PARM0 and reports
 * through OFS_RETURN. */
void ctest_phys_set_self (int num)
{
	pr_global_struct->self = EDICT_TO_PROG (EDICT_NUM (num));
}

void ctest_phys_set_parm0 (float v)
{
	G_FLOAT (OFS_PARM0) = v;
	G_FLOAT (OFS_RETURN) = 0;
}

void ctest_phys_globals (float *out3)
{
	out3[0] = G_FLOAT (OFS_RETURN);
	out3[1] = G_FLOAT (OFS_PARM0);
	out3[2] = (float)pr_global_struct->self;
}

/* Raw entvars_t write for what the setter above does not cover -- a NaN
 * velocity component in particular. `float_ofs` is a float offset into
 * entvars_t; the test spells it with the mirror's field offsets. */
void ctest_phys_edict_poke_bits (int num, int float_ofs, unsigned bits)
{
	memcpy (((float *)&EDICT_NUM (num)->v) + float_ofs, &bits, sizeof (bits));
}

/* The float offset of one entvars_t field, so a test can pin the constants it
 * pokes with against the C compiler's own layout instead of hardcoding them.
 * Returns -1 for a name this helper does not know. */
int ctest_phys_entvars_offset (const char *name)
{
	static const struct
	{
		const char *name;
		size_t		byte_ofs;
	} fields[] = {
		{"modelindex", offsetof (entvars_t, modelindex)},
		{"absmin", offsetof (entvars_t, absmin)},
		{"absmax", offsetof (entvars_t, absmax)},
		{"ltime", offsetof (entvars_t, ltime)},
		{"movetype", offsetof (entvars_t, movetype)},
		{"solid", offsetof (entvars_t, solid)},
		{"origin", offsetof (entvars_t, origin)},
		{"oldorigin", offsetof (entvars_t, oldorigin)},
		{"velocity", offsetof (entvars_t, velocity)},
		{"angles", offsetof (entvars_t, angles)},
		{"mins", offsetof (entvars_t, mins)},
		{"maxs", offsetof (entvars_t, maxs)},
		{"flags", offsetof (entvars_t, flags)},
		{"waterlevel", offsetof (entvars_t, waterlevel)},
		{"watertype", offsetof (entvars_t, watertype)},
		{"nextthink", offsetof (entvars_t, nextthink)},
		{"ideal_yaw", offsetof (entvars_t, ideal_yaw)},
		{"yaw_speed", offsetof (entvars_t, yaw_speed)},
	};
	size_t i;

	for (i = 0; i < sizeof (fields) / sizeof (fields[0]); i++)
		if (!strcmp (fields[i].name, name))
			return (int)(fields[i].byte_ofs / sizeof (float));
	return -1;
}

#define CTEST_PHYS_EDICT_WORDS 50

static void ctest_phys_put_f (unsigned *out, int *n, float v)
{
	memcpy (out + *n, &v, sizeof (v));
	(*n)++;
}

static void ctest_phys_put_i (unsigned *out, int *n, int v)
{
	out[*n] = (unsigned)v;
	(*n)++;
}

static void ctest_phys_put_v (unsigned *out, int *n, const float *v)
{
	ctest_phys_put_f (out, n, v[0]);
	ctest_phys_put_f (out, n, v[1]);
	ctest_phys_put_f (out, n, v[2]);
}

/* Every observable field of one edict, floats as exact bit patterns. */
int ctest_phys_edict_snapshot (int num, unsigned *out, int max)
{
	edict_t *ed;
	int		 n = 0;

	if (max < CTEST_PHYS_EDICT_WORDS)
		return 0;
	ed = EDICT_NUM (num);

	ctest_phys_put_i (out, &n, ed->free ? 1 : 0);
	ctest_phys_put_i (out, &n, (int)ed->num_leafs);
	ctest_phys_put_v (out, &n, ed->v.origin);
	ctest_phys_put_v (out, &n, ed->v.velocity);
	ctest_phys_put_v (out, &n, ed->v.angles);
	ctest_phys_put_v (out, &n, ed->v.avelocity);
	ctest_phys_put_v (out, &n, ed->v.mins);
	ctest_phys_put_v (out, &n, ed->v.maxs);
	ctest_phys_put_v (out, &n, ed->v.size);
	ctest_phys_put_v (out, &n, ed->v.absmin);
	ctest_phys_put_v (out, &n, ed->v.absmax);
	ctest_phys_put_f (out, &n, ed->v.movetype);
	ctest_phys_put_f (out, &n, ed->v.ideal_yaw); /* SV_StepDirection / SV_NewChaseDir write this */
	ctest_phys_put_f (out, &n, ed->v.yaw_speed);
	ctest_phys_put_f (out, &n, ed->v.solid);
	ctest_phys_put_f (out, &n, ed->v.flags);
	ctest_phys_put_f (out, &n, ed->v.waterlevel);
	ctest_phys_put_f (out, &n, ed->v.watertype);
	/* groundentity is a byte offset into the arena; report the edict number
	 * so the comparison survives a different arena base address */
	ctest_phys_put_i (out, &n, ed->v.groundentity ? NUM_FOR_EDICT (PROG_TO_EDICT ((int)ed->v.groundentity)) : -1);
	ctest_phys_put_f (out, &n, ed->v.nextthink);
	ctest_phys_put_f (out, &n, ed->v.ltime);
	ctest_phys_put_f (out, &n, ed->v.frame);
	ctest_phys_put_f (out, &n, ed->oldframe);
	ctest_phys_put_f (out, &n, ed->oldthinktime);
	ctest_phys_put_f (out, &n, ed->lastthink);
	ctest_phys_put_v (out, &n, ed->predthinkpos);
	ctest_phys_put_i (out, &n, ed->sendinterval ? 1 : 0);
	ctest_phys_put_i (out, &n, ed->sendinterval_default ? 1 : 0);
	ctest_phys_put_f (out, &n, ed->freetime);
	ctest_phys_put_i (out, &n, (int)ed->v.think);

	return n;
}

double ctest_phys_vm_time (void)
{
	return qcvm->time;
}

int ctest_phys_num_edicts (void)
{
	return qcvm->num_edicts;
}

float ctest_phys_force_retouch (void)
{
	return pr_global_struct->force_retouch;
}

/* The sv_speeds report. `_c` reads the c_ref_* storage sv_phys.c writes,
 * `_plain` (bottom of this file) the copies the Rust port writes. */
void ctest_phys_speeds_c (double *ms3, int *counts4, int *analytic_frame)
{
	ms3[0] = sv_speeds_think_ms; /* every name here is renamed c_ref_* */
	ms3[1] = sv_speeds_pusher_ms;
	ms3[2] = sv_speeds_build_ms;
	counts4[0] = sv_speeds_thinks;
	counts4[1] = sv_speeds_pushers;
	counts4[2] = sv_speeds_pushables;
	counts4[3] = sv_speeds_grid_entries;
	*analytic_frame = sv_analyticphysics_frame ? 1 : 0;
}

/* --- the plain-named world cvars ------------------------------------------
 * The Rust port reads the engine's own `sv_fte_*` cvars, which the shipping
 * build defines in world_glue.c -- a Meson-only translation unit that is not
 * one of build.rs's C_SOURCES. The harness therefore owns them, and has to
 * step around the prelude's rename macros to spell them plainly.
 *
 * DUPLICATE-SYMBOL HAZARD: if world_glue.c is ever added to build.rs's
 * C_SOURCES, these two cvars AND all five plain SV_* wrappers at the bottom
 * of this file must be removed together -- world_glue.c defines every one of
 * them, and keeping either half would break the link. */
#undef sv_fte_recursivehullckeck
#undef sv_fte_createareanode

cvar_t sv_fte_recursivehullckeck;
cvar_t sv_fte_createareanode;

void ctest_world_set_plain_cvars (float recursivehullcheck, float createareanode)
{
	sv_fte_recursivehullckeck.value = recursivehullcheck;
	sv_fte_createareanode.value = createareanode;
}

/* --- the raise-capable world.c public ABI ---------------------------------
 * ADR-009: six world.c entry points reach Host_Error -- SV_LinkEdict through
 * SV_TouchLinks -> PR_ExecuteProgram, and five more through PR_GetString
 * inside the two SV_HullForEntity Con_Warning sites plus, for SV_Move,
 * assert_always/COM_Assert_Failed at its tail -- so the Rust port exports
 * them as quake_rs_* status cores and the shipping build wraps them in
 * Quake/world_glue.c. world_glue.c is not one of build.rs's C_SOURCES, so
 * the harness owns the wrappers -- same topology, same names. The tests
 * drive the plain symbols only, so no longjmp unwinds a Rust frame here any
 * more than it does in the engine.
 *
 * DUPLICATE-SYMBOL HAZARD: if world_glue.c is ever added to build.rs's
 * C_SOURCES, all six wrappers below AND the two plain sv_fte_* cvars above
 * must be removed together -- world_glue.c defines every one of them, and
 * keeping either half would break the link.
 *
 * The prelude's rename macros are still live in this translation unit and
 * would rewrite these definitions to c_ref_*, colliding with the real oracle
 * compiled from world.c, so each name is #undef'd first. */
#undef SV_LinkEdict
#undef SV_UnlinkEdict
#undef SV_HullForEntity
#undef SV_ClipMoveToEntity
#undef SV_Move
#undef SV_TestEntityPosition
#undef SV_PointContentsAllBsps

extern int quake_rs_sv_link_edict (edict_t *ent, qboolean touch_triggers);
extern void SV_UnlinkEdict (edict_t *ent); /* the Rust export; world.h's declaration was renamed by the prelude */
extern int quake_rs_sv_hull_for_entity (edict_t *ent, vec3_t mins, vec3_t maxs, vec3_t offset, hull_t **out);
extern int quake_rs_sv_clip_move_to_entity (trace_t *out, edict_t *ent, vec3_t start, vec3_t mins, vec3_t maxs, vec3_t end, unsigned int hitcontents);
extern int quake_rs_sv_move (trace_t *out, vec3_t start, vec3_t mins, vec3_t maxs, vec3_t end, int type, edict_t *passedict);
extern int quake_rs_sv_test_entity_position (edict_t *ent, edict_t **out);
extern int quake_rs_sv_point_contents_all_bsps (int *out, vec3_t p, edict_t *forent);

void SV_LinkEdict (edict_t *ent, qboolean touch_triggers)
{
	Host_Reraise (quake_rs_sv_link_edict (ent, touch_triggers));
}

hull_t *SV_HullForEntity (edict_t *ent, vec3_t mins, vec3_t maxs, vec3_t offset)
{
	hull_t *out = NULL;
	int		r = quake_rs_sv_hull_for_entity (ent, mins, maxs, offset, &out);
	if (r)
		Host_Reraise (r);
	return out;
}

trace_t SV_ClipMoveToEntity (edict_t *ent, vec3_t start, vec3_t mins, vec3_t maxs, vec3_t end, unsigned int hitcontents)
{
	trace_t out;
	int		r;
	memset (&out, 0, sizeof (out));
	r = quake_rs_sv_clip_move_to_entity (&out, ent, start, mins, maxs, end, hitcontents);
	if (r)
		Host_Reraise (r);
	return out;
}

trace_t SV_Move (vec3_t start, vec3_t mins, vec3_t maxs, vec3_t end, int type, edict_t *passedict)
{
	trace_t out;
	int		r;
	memset (&out, 0, sizeof (out));
	r = quake_rs_sv_move (&out, start, mins, maxs, end, type, passedict);
	if (r)
		Host_Reraise (r);
	return out;
}

edict_t *SV_TestEntityPosition (edict_t *ent)
{
	edict_t *out = NULL;
	int		 r = quake_rs_sv_test_entity_position (ent, &out);
	if (r)
		Host_Reraise (r);
	return out;
}

/* SV_PointContentsAllBsps (world.c:588) calls SV_Move, so it inherits the
 * whole raise-capable pipeline underneath it. `*out` stays untouched on the
 * raising path because world.c:590 reads trace.contents only after SV_Move
 * returns -- the longjmp writes nothing either. */
int SV_PointContentsAllBsps (vec3_t p, edict_t *forent)
{
	int contents = 0;
	int r = quake_rs_sv_point_contents_all_bsps (&contents, p, forent);
	if (r)
		Host_Reraise (r);
	return contents;
}

/* Points the fixture's re-entrant link/unlink hook at the Rust pair. Doing
 * this from C rather than handing ctest_world_set_link_fns a Rust function
 * pointer keeps the ADR-009 rule intact on the raising path: a touch handler
 * that relinks reaches the wrapper above directly, so the longjmp out of
 * Host_Reraise unwinds C frames only. */
void ctest_world_set_rust_link_fns (void)
{
	ctest_world_set_link_fns (SV_LinkEdict, SV_UnlinkEdict);
}

/* --- the plain-named sv_phys.c data (Phase 7 M4) --------------------------
 * The Rust port reads and writes the engine's own cvars and sv_speeds
 * counters, which the shipping build defines in Quake/sv_phys_glue.c -- a
 * Meson-only translation unit that is not one of build.rs's C_SOURCES. The
 * harness therefore owns them, and has to step around the prelude's rename
 * macros to spell them plainly. Initialisers copied from sv_phys.c:44-54,
 * :56, :345-346 and :705.
 *
 * DUPLICATE-SYMBOL HAZARD: if sv_phys_glue.c is ever added to build.rs's
 * C_SOURCES, everything in this block AND every SvPhys_Glue_* helper AND the
 * nine plain SV_* wrappers below must be removed together. */
#undef sv_friction
#undef sv_stopspeed
#undef sv_gravity
#undef sv_maxvelocity
#undef sv_nostep
#undef sv_freezenonclients
#undef sv_gameplayfix_spawnbeforethinks
#undef sv_gameplayfix_bouncedownslopes
#undef sv_gameplayfix_elevators
#undef sv_fastpushmove
#undef sv_pushgrid
#undef sv_analyticphysics
#undef sv_analyticphysics_frame
#undef sv_speeds_think_ms
#undef sv_speeds_pusher_ms
#undef sv_speeds_build_ms
#undef sv_speeds_thinks
#undef sv_speeds_pushers
#undef sv_speeds_pushables
#undef sv_speeds_grid_entries

cvar_t sv_friction = {"sv_friction", "4", CVAR_NOTIFY | CVAR_SERVERINFO};
cvar_t sv_stopspeed = {"sv_stopspeed", "100", CVAR_NONE};
cvar_t sv_gravity = {"sv_gravity", "800", CVAR_NOTIFY | CVAR_SERVERINFO};
cvar_t sv_maxvelocity = {"sv_maxvelocity", "2000", CVAR_NONE};
cvar_t sv_nostep = {"sv_nostep", "0", CVAR_NONE};
cvar_t sv_freezenonclients = {"sv_freezenonclients", "0", CVAR_NONE};
cvar_t sv_gameplayfix_spawnbeforethinks = {"sv_gameplayfix_spawnbeforethinks", "0", CVAR_NONE};
cvar_t sv_gameplayfix_bouncedownslopes = {"sv_gameplayfix_bouncedownslopes", "1", CVAR_NONE};
cvar_t sv_fastpushmove = {"sv_fastpushmove", "1", CVAR_NONE};
cvar_t sv_pushgrid = {"sv_pushgrid", "1", CVAR_NONE};
cvar_t sv_analyticphysics = {"sv_analyticphysics", "1", CVAR_NONE};
/* 0=off; 1=legacy DIST_EPSILON nudge, clients only; 2=legacy nudge, all entities; 3=robust pusher contact (default) */
cvar_t sv_gameplayfix_elevators = {"sv_gameplayfix_elevators", "3", CVAR_NONE};

qboolean sv_analyticphysics_frame = true;

double sv_speeds_think_ms, sv_speeds_pusher_ms, sv_speeds_build_ms;
int	   sv_speeds_thinks, sv_speeds_pushers, sv_speeds_pushables, sv_speeds_grid_entries;

void ctest_phys_set_plain_cvars (const float *v)
{
	sv_friction.value = v[0];
	sv_stopspeed.value = v[1];
	sv_gravity.value = v[2];
	sv_maxvelocity.value = v[3];
	sv_nostep.value = v[4];
	sv_freezenonclients.value = v[5];
	sv_gameplayfix_spawnbeforethinks.value = v[6];
	sv_gameplayfix_bouncedownslopes.value = v[7];
	sv_gameplayfix_elevators.value = v[8];
	sv_fastpushmove.value = v[9];
	sv_pushgrid.value = v[10];
	sv_analyticphysics.value = v[11];
}

void ctest_phys_zero_speeds_plain (void)
{
	sv_speeds_think_ms = 0;
	sv_speeds_pusher_ms = 0;
	sv_speeds_build_ms = 0;
	sv_speeds_thinks = 0;
	sv_speeds_pushers = 0;
	sv_speeds_pushables = 0;
	sv_speeds_grid_entries = 0;
}

void ctest_phys_speeds_plain (double *ms3, int *counts4, int *analytic_frame)
{
	ms3[0] = sv_speeds_think_ms;
	ms3[1] = sv_speeds_pusher_ms;
	ms3[2] = sv_speeds_build_ms;
	counts4[0] = sv_speeds_thinks;
	counts4[1] = sv_speeds_pushers;
	counts4[2] = sv_speeds_pushables;
	counts4[3] = sv_speeds_grid_entries;
	*analytic_frame = sv_analyticphysics_frame ? 1 : 0;
}

/* --- the raise-capable sv_move.c / sv_phys.c public ABI (Phase 7 M4) ------
 * ADR-009: nine entry points reach Host_Error, so the Rust port exports them
 * as quake_rs_* status cores and the shipping build wraps them in
 * Quake/sv_move_glue.c / Quake/sv_phys_glue.c. Neither glue file is one of
 * build.rs's C_SOURCES, so the harness owns the wrappers -- same topology,
 * same names. The tests drive the plain symbols only, so no longjmp unwinds
 * a Rust frame here any more than it does in the engine.
 *
 * SV_FixCheckBottom, SV_CloseEnough and SV_PushGridEntityLinked cannot raise
 * and are exported plainly by the Rust port, so they get no wrapper.
 *
 * The prelude's rename macros are still live in this translation unit and
 * would rewrite these definitions to c_ref_*, colliding with the real
 * oracles compiled from sv_move.c/sv_phys.c, so each name is #undef'd. */
#undef SV_CheckBottom
#undef SV_movestep
#undef SV_StepDirection
#undef SV_NewChaseDir
#undef SV_MoveToGoal
#undef SV_CheckAllEnts
#undef SV_CheckVelocity
#undef SV_CheckWaterTransition
#undef SV_Physics
#undef SV_PushGridEntityLinked

extern int quake_rs_sv_check_bottom (edict_t *ent, qboolean *out);
extern int quake_rs_sv_movestep (edict_t *ent, vec3_t move, qboolean relink, qboolean *out);
extern int quake_rs_sv_step_direction (edict_t *ent, float yaw, float dist, qboolean *out);
extern int quake_rs_sv_new_chase_dir (edict_t *actor, edict_t *enemy, float dist);
extern int quake_rs_sv_move_to_goal (void);
extern int quake_rs_sv_check_all_ents (void);
extern int quake_rs_sv_check_velocity (edict_t *ent);
extern int quake_rs_sv_check_water_transition (edict_t *ent);
extern int quake_rs_sv_physics (void);

qboolean SV_CheckBottom (edict_t *ent)
{
	qboolean out = false;
	int		 r = quake_rs_sv_check_bottom (ent, &out);
	Host_Reraise (r);
	return out;
}

qboolean SV_movestep (edict_t *ent, vec3_t move, qboolean relink)
{
	qboolean out = false;
	int		 r = quake_rs_sv_movestep (ent, move, relink, &out);
	Host_Reraise (r);
	return out;
}

qboolean SV_StepDirection (edict_t *ent, float yaw, float dist)
{
	qboolean out = false;
	int		 r = quake_rs_sv_step_direction (ent, yaw, dist, &out);
	Host_Reraise (r);
	return out;
}

void SV_NewChaseDir (edict_t *actor, edict_t *enemy, float dist)
{
	Host_Reraise (quake_rs_sv_new_chase_dir (actor, enemy, dist));
}

void SV_MoveToGoal (void)
{
	Host_Reraise (quake_rs_sv_move_to_goal ());
}

void SV_CheckAllEnts (void)
{
	Host_Reraise (quake_rs_sv_check_all_ents ());
}

void SV_CheckVelocity (edict_t *ent)
{
	Host_Reraise (quake_rs_sv_check_velocity (ent));
}

void SV_CheckWaterTransition (edict_t *ent)
{
	Host_Reraise (quake_rs_sv_check_water_transition (ent));
}

void SV_Physics (void)
{
	Host_Reraise (quake_rs_sv_physics ());
}

/* world_glue.c's thin wrapper, for the Rust side. Through M3 this forwarded
 * to a stub-owned logger, because sv_phys.c was not in C_SOURCES; it now
 * forwards to the Rust port's own plain export, exactly as world_glue.c
 * does in the engine. */
extern void SV_PushGridEntityLinked (edict_t *ent);

void World_Glue_PushGridEntityLinked (edict_t *ent)
{
	SV_PushGridEntityLinked (ent);
}

/* --- the edict allocator's one hook slot (Phase 7 M4) --------------------
 *
 * ADR-006 keeps the edict arena in C, so pr_edict_arena.c is one of build.rs's
 * C_SOURCES and both ED_Alloc and its hook setter link as c_ref_*. There is
 * therefore exactly ONE allocator in this process and exactly ONE hook slot,
 * and c_ref_ED_Alloc is what the fixture's CTEST_WORLD_TOUCH_SPAWN builtin
 * calls on both sides of every differential.
 *
 * The Rust port of sv_phys.c installs SV_Physics_Alloc_Hook for the duration
 * of a fast-pushmove tick by calling the plain ED_AllocSetHook, which lands
 * here. Forwarding to c_ref_ED_AllocSetHook -- rather than keeping a private
 * slot -- is what keeps that coverage real: a store-and-return-previous stub
 * would link and go green while leaving the Rust side's pushable_ent_cache
 * blind to mid-tick allocations that the C side's cache picks up. */
static ED_AllocHook_func ctest_ed_alloc_set_hook (ED_AllocHook_func hook)
{
	return ED_AllocSetHook (hook); /* renamed c_ref_ED_AllocSetHook */
}

#undef ED_AllocSetHook

ED_AllocHook_func ED_AllocSetHook (ED_AllocHook_func hook)
{
	return ctest_ed_alloc_set_hook (hook);
}

/* --- Phase 7 M5 wave 1: the pr_cmds_sv_glue.c seams -----------------------
 *
 * `Quake/pr_cmds_sv_glue.c` is not in build.rs's C_SOURCES (pr_cmds.c and
 * pr_ext.c are not either -- there is no c_ref_PF_* oracle, by contract), so
 * the guarded C frame the Rust builtins call through is reproduced here.
 * Every helper keeps the same shape as the engine's: an argument struct plus
 * Host_Guard, so a raise never unwinds a Rust frame (ADR-009).
 *
 * Two deliberate simplifications, both documented in the M5 report:
 *  - the engine's "backwards mins/maxs" and "no precache" arms call
 *    PR_RunError, which walks the progs stack; this harness raises
 *    Host_Error with the identical message instead, so the tests can assert
 *    the message through ctest_host_error_message() without needing a live
 *    interpreter frame.
 *  - the precache lookup has no SV_Precache_Model fallback: the fixture's
 *    table is preloaded by the test.
 */

cvar_t sv_aim = {"sv_aim", "1", CVAR_NONE, 1.0f, NULL, NULL, NULL, NULL};
cvar_t teamplay = {"teamplay", "0", CVAR_NOTIFY | CVAR_SERVERINFO, 0.0f, NULL, NULL, NULL, NULL};

static unsigned char *ctest_pf_leaf_pvs = NULL;

void ctest_pf_set_leaf_pvs (unsigned char *pvs)
{
	ctest_pf_leaf_pvs = pvs;
}

unsigned char *Mod_LeafPVS (mleaf_t *leaf, qmodel_t *model)
{
	(void)leaf;
	(void)model;
	return ctest_pf_leaf_pvs;
}

#define CTEST_PF_MAX_MODELS 8
static const char *ctest_pf_precache_names[CTEST_PF_MAX_MODELS + 1];
static void		  *ctest_pf_precache_models[CTEST_PF_MAX_MODELS + 1];
static int		   ctest_pf_precache_count;

void ctest_pf_reset (void)
{
	memset (ctest_pf_precache_names, 0, sizeof (ctest_pf_precache_names));
	memset (ctest_pf_precache_models, 0, sizeof (ctest_pf_precache_models));
	ctest_pf_precache_count = 0;
	ctest_pf_leaf_pvs = NULL;
	ctest_set_point_leaf (NULL);
}

void ctest_pf_add_precache (const char *name, void *model)
{
	if (ctest_pf_precache_count >= CTEST_PF_MAX_MODELS)
		return;
	ctest_pf_precache_names[ctest_pf_precache_count] = name;
	ctest_pf_precache_models[ctest_pf_precache_count] = model;
	ctest_pf_precache_count++;
}

typedef struct
{
	int			 handle;
	const char **out_name;
	int			*out_index;
	void	   **out_model;
} ctest_pf_setmodel_args_t;

static void ctest_pf_invoke_setmodel (void *p)
{
	ctest_pf_setmodel_args_t *a = (ctest_pf_setmodel_args_t *)p;
	const char				 *m = PR_GetString (a->handle); /* c_ref_PR_GetString */
	const char				**check = ctest_pf_precache_names;
	int						  i;

	for (i = 0; *check; i++, check++)
		if (!strcmp (*check, m))
			break;

	if (!*check)
		Host_Error ("no precache: %s", m);

	*a->out_name = *check;
	*a->out_index = i;
	*a->out_model = ctest_pf_precache_models[i];
}

int PRBI_SvGlue_SetModelLookup (int handle, const char **out_name, int *out_index, void **out_model)
{
	ctest_pf_setmodel_args_t a;
	a.handle = handle;
	a.out_name = out_name;
	a.out_index = out_index;
	a.out_model = out_model;
	return Host_Guard (ctest_pf_invoke_setmodel, &a);
}

static void ctest_pf_invoke_backwards (void *p)
{
	(void)p;
	Host_Error ("backwards mins/maxs");
}

int PRBI_SvGlue_RunErrorBackwardsMinsMaxs (void)
{
	return Host_Guard (ctest_pf_invoke_backwards, NULL);
}

typedef struct
{
	float	*v1;
	float	*v2;
	edict_t *ent;
} ctest_pf_nan_args_t;

static void ctest_pf_invoke_warnnan (void *p)
{
	ctest_pf_nan_args_t *a = (ctest_pf_nan_args_t *)p;
	Con_Warning (
		"NAN in traceline:\nv1(%f %f %f) v2(%f %f %f)\nentity %d\n", a->v1[0], a->v1[1], a->v1[2], a->v2[0], a->v2[1], a->v2[2], NUM_FOR_EDICT (a->ent));
}

int PRBI_SvGlue_WarnNanTrace (float *v1, float *v2, edict_t *ent)
{
	ctest_pf_nan_args_t a;
	a.v1 = v1;
	a.v2 = v2;
	a.ent = ent;
	return Host_Guard (ctest_pf_invoke_warnnan, &a);
}

/* server.h's sv.lastcheck/sv.lastchecktime (server.h:59-60). Written when the
 * prelude's `sv` was a stand-in struct that did not carry them, so the
 * round-robin cursor lives in two statics with the same lifetime. Phase 7 M6:
 * `sv` is the real server_t now, so this pair CAN be collapsed onto
 * sv.lastcheck/sv.lastchecktime -- left alone here only because doing it would
 * change what the PF_checkclient tests observe, which is M6 port work, not
 * T6.0's. */
static int	  ctest_pf_lastcheck;
static double ctest_pf_lastchecktime;

int PRBI_SvGlue_SvLastCheck (void)
{
	return ctest_pf_lastcheck;
}

void PRBI_SvGlue_SetSvLastCheck (int value)
{
	ctest_pf_lastcheck = value;
}

double PRBI_SvGlue_SvLastCheckTime (void)
{
	return ctest_pf_lastchecktime;
}

void PRBI_SvGlue_SetSvLastCheckTime (double value)
{
	ctest_pf_lastchecktime = value;
}

static int ctest_pf_set_engine_string (const char *s)
{
	return PR_SetEngineString (s); /* c_ref_PR_SetEngineString */
}

#undef PR_SetEngineString

int PR_SetEngineString (const char *s)
{
	return ctest_pf_set_engine_string (s);
}

/* --- Phase 7 M5 wave 1: generic peek/poke for the builtin differentials ---
 *
 * The builtins read their arguments from, and write their results to, the
 * progs globals block, and they touch entvars fields ctest_phys_edict_set
 * does not cover (view_ofs, takedamage, team, chain, model). Rather than
 * grow a bespoke setter per builtin, these expose the two blocks by float
 * offset, with the offsets themselves resolved from the C compiler's own
 * layout so the Rust side never hardcodes them. */

int ctest_pf_globals_offset (const char *name)
{
	static const struct
	{
		const char *name;
		size_t		byte_ofs;
	} fields[] = {
		{"self", offsetof (globalvars_t, self)},
		{"other", offsetof (globalvars_t, other)},
		{"time", offsetof (globalvars_t, time)},
		{"v_forward", offsetof (globalvars_t, v_forward)},
		{"trace_allsolid", offsetof (globalvars_t, trace_allsolid)},
		{"trace_startsolid", offsetof (globalvars_t, trace_startsolid)},
		{"trace_fraction", offsetof (globalvars_t, trace_fraction)},
		{"trace_endpos", offsetof (globalvars_t, trace_endpos)},
		{"trace_plane_normal", offsetof (globalvars_t, trace_plane_normal)},
		{"trace_plane_dist", offsetof (globalvars_t, trace_plane_dist)},
		{"trace_ent", offsetof (globalvars_t, trace_ent)},
		{"trace_inopen", offsetof (globalvars_t, trace_inopen)},
		{"trace_inwater", offsetof (globalvars_t, trace_inwater)},
	};
	size_t i;

	for (i = 0; i < sizeof (fields) / sizeof (fields[0]); i++)
		if (!strcmp (fields[i].name, name))
			return (int)(fields[i].byte_ofs / sizeof (float));
	return -1;
}

int ctest_pf_entvars_offset (const char *name)
{
	static const struct
	{
		const char *name;
		size_t		byte_ofs;
	} fields[] = {
		{"model", offsetof (entvars_t, model)},
		{"modelindex", offsetof (entvars_t, modelindex)},
		{"size", offsetof (entvars_t, size)},
		{"absmin", offsetof (entvars_t, absmin)},
		{"absmax", offsetof (entvars_t, absmax)},
		{"view_ofs", offsetof (entvars_t, view_ofs)},
		{"takedamage", offsetof (entvars_t, takedamage)},
		{"team", offsetof (entvars_t, team)},
		{"health", offsetof (entvars_t, health)},
		{"chain", offsetof (entvars_t, chain)},
		{"groundentity", offsetof (entvars_t, groundentity)},
		{"flags", offsetof (entvars_t, flags)},
		{"origin", offsetof (entvars_t, origin)},
		{"mins", offsetof (entvars_t, mins)},
		{"maxs", offsetof (entvars_t, maxs)},
		{"solid", offsetof (entvars_t, solid)},
	};
	size_t i;

	for (i = 0; i < sizeof (fields) / sizeof (fields[0]); i++)
		if (!strcmp (fields[i].name, name))
			return (int)(fields[i].byte_ofs / sizeof (float));
	return -1;
}

unsigned ctest_pf_edict_bits (int num, int float_ofs)
{
	unsigned bits;
	memcpy (&bits, ((float *)&EDICT_NUM (num)->v) + float_ofs, sizeof (bits));
	return bits;
}

unsigned ctest_pf_global_bits (int float_ofs)
{
	unsigned bits;
	memcpy (&bits, qcvm->globals + float_ofs, sizeof (bits));
	return bits;
}

void ctest_pf_set_global_bits (int float_ofs, unsigned bits)
{
	memcpy (qcvm->globals + float_ofs, &bits, sizeof (bits));
}

int ctest_pf_num_globals (void)
{
	return qcvm->progs->numglobals;
}

/* Every global as an exact bit pattern: one call captures OFS_RETURN and the
 * whole trace_* block at once. */
int ctest_pf_snapshot_globals (unsigned *out, int max)
{
	int n = qcvm->progs->numglobals;
	if (n > max)
		n = max;
	memcpy (out, qcvm->globals, (size_t)n * sizeof (unsigned));
	return n;
}

int ctest_pf_edict_prog (int num)
{
	return EDICT_TO_PROG (EDICT_NUM (num));
}

int ctest_pf_prog_to_num (int prog)
{
	return NUM_FOR_EDICT (PROG_TO_EDICT (prog));
}

/* PF_setmodel reads qmodel_t's bounding boxes through quake-types' QModel
 * mirror; the fixture model is built by C, so these setters/readers let the
 * differential drive and check both sides of that mirror. */
void ctest_pf_set_model_boxes (void *model, int type, const float *mins, const float *maxs, const float *clipmins, const float *clipmaxs)
{
	qmodel_t *m = (qmodel_t *)model;
	m->type = (modtype_t)type;
	VectorCopy (mins, m->mins);
	VectorCopy (maxs, m->maxs);
	VectorCopy (clipmins, m->clipmins);
	VectorCopy (clipmaxs, m->clipmaxs);
}

/* --- Phase 7 M5 wave 1: the C oracle -------------------------------------
 *
 * pr_cmds.c and pr_ext.c are deliberately NOT in build.rs's C_SOURCES (the
 * M5 contract forbids adding them -- that is a far larger change), so there
 * is no c_ref_PF_* to diff against. These are hand transcriptions of the
 * bodies wave 1 ports, statement for statement out of Quake/pr_cmds.c
 * (:227, :237, :321, :340, :740, :804, :881, :1017, :1288, :1330, :1432,
 * :1446, :1494, :1853) and Quake/pr_ext.c (:1833, :5369).
 *
 * What makes them an oracle rather than a second copy of the port: every
 * primitive they call is the renamed C original -- SV_Move, SV_LinkEdict,
 * SV_movestep, SV_CheckBottom, SV_PointContents, VectorNormalize,
 * PR_GetString -- and the float arithmetic is evaluated by the C compiler,
 * so the double-promotion sites (`* 0.5`, `yaw * M_PI * 2 / 360`,
 * `cos (yaw) * dist`) are the C compiler's, not Rust's.
 *
 * Transcription notes:
 *  - PR_RunError becomes Host_Error with the identical message, matching the
 *    PRBI_SvGlue_* stubs above (this harness has no interpreter frame).
 *  - PF_setmodel's not-precached arm is Host_Error too: no SV_Precache_Model.
 *  - checkpvs/checkpvs_capacity are this file's own statics, mirroring the
 *    pr_cmds.c process globals; sv.lastcheck/sv.lastchecktime are shared with
 *    the Rust side through ctest_pf_lastcheck/ctest_pf_lastchecktime, exactly
 *    as both VMs share `sv` in the engine.
 */

#ifndef DAMAGE_AIM
#define DAMAGE_AIM 2
#endif

void ctest_pf_set_developer (float v)
{
	developer.value = v;
}

void ctest_pf_set_sv_aim (float v)
{
	sv_aim.value = v;
}

void ctest_pf_set_teamplay (float v)
{
	teamplay.value = v;
}

void ctest_pf_reset_checkclient (void)
{
	ctest_pf_lastcheck = 0;
	ctest_pf_lastchecktime = 0.0;
}

/* Mod_PointInLeaf is a settable stub; the fixture's brush model owns five
 * mleaf_t. `idx` 0 is the solid leaf, so l = (leaf - leafs) - 1 == -1 there,
 * which is PF_checkclient's negative-index arm. */
void ctest_pf_set_point_leaf_index (int idx)
{
	if (idx < 0)
		ctest_set_point_leaf (NULL);
	else
		ctest_set_point_leaf (qcvm->worldmodel->leafs + idx);
}

static int ctest_pf_is_nan (float x)
{
	unsigned u;
	memcpy (&u, &x, sizeof (u));
	return (u & (255u << 23)) == (255u << 23);
}

/* stubs.c includes world.h (which declares SV_Move/SV_LinkEdict/
 * SV_PointContents) but not server.h, so the two sv_move.c oracles the
 * transcriptions below call have no prototype in scope here. Without these
 * two lines MSVC falls back to "extern returning int" and the qboolean (=
 * bool) return is read as a full garbage register -- which is a silent,
 * non-deterministic differential failure, not a link error. */
qboolean c_ref_SV_CheckBottom (edict_t *ent);
qboolean c_ref_SV_movestep (edict_t *ent, vec3_t move, qboolean relink);

static void ctest_cref_SetMinMaxSize (edict_t *e, float *minvec, float *maxvec)
{
	int i;

	for (i = 0; i < 3; i++)
		if (minvec[i] > maxvec[i])
			Host_Error ("backwards mins/maxs"); /* PR_RunError in the engine */

	VectorCopy (minvec, e->v.mins);
	VectorCopy (maxvec, e->v.maxs);
	VectorSubtract (maxvec, minvec, e->v.size);

	c_ref_SV_LinkEdict (e, false);
}

void ctest_cref_pf_setorigin (void)
{
	edict_t *e;
	float	*org;

	e = G_EDICT (OFS_PARM0);
	org = G_VECTOR (OFS_PARM1);
	VectorCopy (org, e->v.origin);
	c_ref_SV_LinkEdict (e, false);
}

void ctest_cref_pf_setsize (void)
{
	edict_t *e;
	float	*minvec, *maxvec;

	e = G_EDICT (OFS_PARM0);
	minvec = G_VECTOR (OFS_PARM1);
	maxvec = G_VECTOR (OFS_PARM2);
	ctest_cref_SetMinMaxSize (e, minvec, maxvec);
}

void ctest_cref_pf_setmodel (void)
{
	int			 i;
	const char	*m, **check;
	qmodel_t	*mod;
	edict_t		 *e;

	e = G_EDICT (OFS_PARM0);
	m = PR_GetString (G_INT (OFS_PARM1));

	for (i = 0, check = ctest_pf_precache_names; *check; i++, check++)
	{
		if (!strcmp (*check, m))
			break;
	}

	if (!*check)
		Host_Error ("no precache: %s", m);

	e->v.model = PR_SetEngineString (*check);
	e->v.modelindex = i;

	mod = (qmodel_t *)ctest_pf_precache_models[(int)e->v.modelindex];

	if (mod)
	{
		if (mod->type == mod_brush)
			ctest_cref_SetMinMaxSize (e, mod->clipmins, mod->clipmaxs);
		else
			ctest_cref_SetMinMaxSize (e, mod->mins, mod->maxs);
	}
	else
		ctest_cref_SetMinMaxSize (e, vec3_origin, vec3_origin);
}

static void ctest_cref_trace_common (float *v1, float *mins, float *maxs, float *v2, int nomonsters, edict_t *ent)
{
	trace_t trace;

	if (developer.value)
	{
		if (ctest_pf_is_nan (v1[0]) || ctest_pf_is_nan (v1[1]) || ctest_pf_is_nan (v1[2]) || ctest_pf_is_nan (v2[0]) || ctest_pf_is_nan (v2[1]) ||
			ctest_pf_is_nan (v2[2]))
		{
			Con_Warning ("NAN in traceline:\nv1(%f %f %f) v2(%f %f %f)\nentity %d\n", v1[0], v1[1], v1[2], v2[0], v2[1], v2[2], NUM_FOR_EDICT (ent));
		}
	}

	if (ctest_pf_is_nan (v1[0]) || ctest_pf_is_nan (v1[1]) || ctest_pf_is_nan (v1[2]))
		v1[0] = v1[1] = v1[2] = 0;
	if (ctest_pf_is_nan (v2[0]) || ctest_pf_is_nan (v2[1]) || ctest_pf_is_nan (v2[2]))
		v2[0] = v2[1] = v2[2] = 0;

	trace = c_ref_SV_Move (v1, mins, maxs, v2, nomonsters, ent);

	pr_global_struct->trace_allsolid = trace.allsolid;
	pr_global_struct->trace_startsolid = trace.startsolid;
	pr_global_struct->trace_fraction = trace.fraction;
	pr_global_struct->trace_inwater = trace.inwater;
	pr_global_struct->trace_inopen = trace.inopen;
	VectorCopy (trace.endpos, pr_global_struct->trace_endpos);
	VectorCopy (trace.plane.normal, pr_global_struct->trace_plane_normal);
	pr_global_struct->trace_plane_dist = trace.plane.dist;

	if (trace.ent)
		pr_global_struct->trace_ent = EDICT_TO_PROG (trace.ent);
	else
		pr_global_struct->trace_ent = EDICT_TO_PROG (qcvm->edicts);
}

void ctest_cref_pf_traceline (void)
{
	ctest_cref_trace_common (G_VECTOR (OFS_PARM0), vec3_origin, vec3_origin, G_VECTOR (OFS_PARM1), (int)G_FLOAT (OFS_PARM2), G_EDICT (OFS_PARM3));
}

void ctest_cref_pf_tracebox (void)
{
	ctest_cref_trace_common (
		G_VECTOR (OFS_PARM0), G_VECTOR (OFS_PARM1), G_VECTOR (OFS_PARM2), G_VECTOR (OFS_PARM3), (int)G_FLOAT (OFS_PARM4), G_EDICT (OFS_PARM5));
}

void ctest_cref_pf_findradius (void)
{
	edict_t *ent, *chain;
	float	 rad;
	float	*org;
	int		 i;

	chain = (edict_t *)qcvm->edicts;

	org = G_VECTOR (OFS_PARM0);
	rad = G_FLOAT (OFS_PARM1);
	rad *= rad;

	ent = NEXT_EDICT (qcvm->edicts);
	for (i = 1; i < qcvm->num_edicts; i++, ent = NEXT_EDICT (ent))
	{
		float d, lensq;
		if (ent->free)
			continue;
		if (ent->v.solid == SOLID_NOT)
			continue;

		d = org[0] - (ent->v.origin[0] + (ent->v.mins[0] + ent->v.maxs[0]) * 0.5);
		lensq = d * d;
		if (lensq > rad)
			continue;
		d = org[1] - (ent->v.origin[1] + (ent->v.mins[1] + ent->v.maxs[1]) * 0.5);
		lensq += d * d;
		if (lensq > rad)
			continue;
		d = org[2] - (ent->v.origin[2] + (ent->v.mins[2] + ent->v.maxs[2]) * 0.5);
		lensq += d * d;
		if (lensq > rad)
			continue;

		ent->v.chain = EDICT_TO_PROG (chain);
		chain = ent;
	}

	RETURN_EDICT (chain);
}

void ctest_cref_pf_walkmove (void)
{
	edict_t		*ent;
	float		 yaw, dist;
	vec3_t		 move;
	dfunction_t *oldf;
	int			 oldself;

	ent = PROG_TO_EDICT (pr_global_struct->self);
	yaw = G_FLOAT (OFS_PARM0);
	dist = G_FLOAT (OFS_PARM1);

	if (!((int)ent->v.flags & (FL_ONGROUND | FL_FLY | FL_SWIM)))
	{
		G_FLOAT (OFS_RETURN) = 0;
		return;
	}

	yaw = yaw * M_PI * 2 / 360;

	move[0] = cos (yaw) * dist;
	move[1] = sin (yaw) * dist;
	move[2] = 0;

	/* save program state, because SV_movestep may call other progs */
	oldf = qcvm->xfunction;
	oldself = pr_global_struct->self;

	G_FLOAT (OFS_RETURN) = c_ref_SV_movestep (ent, move, true);

	/* restore program state */
	qcvm->xfunction = oldf;
	pr_global_struct->self = oldself;
}

void ctest_cref_pf_droptofloor (void)
{
	edict_t *ent;
	vec3_t	 end;
	trace_t	 trace;

	ent = PROG_TO_EDICT (pr_global_struct->self);

	VectorCopy (ent->v.origin, end);
	end[2] -= 256;

	trace = c_ref_SV_Move (ent->v.origin, ent->v.mins, ent->v.maxs, end, false, ent);

	if (trace.fraction == 1 || trace.allsolid)
		G_FLOAT (OFS_RETURN) = 0;
	else
	{
		VectorCopy (trace.endpos, ent->v.origin);
		c_ref_SV_LinkEdict (ent, false);
		ent->v.flags = (int)ent->v.flags | FL_ONGROUND;
		if (trace.ent)
			ent->v.groundentity = EDICT_TO_PROG (trace.ent);
		G_FLOAT (OFS_RETURN) = 1;
	}
}

void ctest_cref_pf_checkbottom (void)
{
	edict_t *ent;

	ent = G_EDICT (OFS_PARM0);

	G_FLOAT (OFS_RETURN) = c_ref_SV_CheckBottom (ent);
}

void ctest_cref_pf_pointcontents (void)
{
	float *v;

	v = G_VECTOR (OFS_PARM0);

	G_FLOAT (OFS_RETURN) = c_ref_SV_PointContents (v);
}

void ctest_cref_pf_aim (void)
{
	edict_t *ent, *check, *bestent;
	vec3_t	 start, dir, end, bestdir;
	int		 i, j;
	trace_t	 tr;
	float	 dist, bestdist;
	float	 speed;

	ent = G_EDICT (OFS_PARM0);
	speed = G_FLOAT (OFS_PARM1);
	(void)speed;

	VectorCopy (ent->v.origin, start);
	start[2] += 20;

	/* try sending a trace straight */
	VectorCopy (pr_global_struct->v_forward, dir);
	VectorMA (start, 2048, dir, end);
	tr = c_ref_SV_Move (start, vec3_origin, vec3_origin, end, false, ent);
	if (tr.ent && tr.ent->v.takedamage == DAMAGE_AIM && (!teamplay.value || ent->v.team <= 0 || ent->v.team != tr.ent->v.team))
	{
		VectorCopy (pr_global_struct->v_forward, G_VECTOR (OFS_RETURN));
		return;
	}

	/* try all possible entities */
	VectorCopy (dir, bestdir);
	bestdist = sv_aim.value;
	bestent = NULL;

	check = NEXT_EDICT (qcvm->edicts);
	for (i = 1; i < qcvm->num_edicts; i++, check = NEXT_EDICT (check))
	{
		if (check->v.takedamage != DAMAGE_AIM)
			continue;
		if (check == ent)
			continue;
		if (teamplay.value && ent->v.team > 0 && ent->v.team == check->v.team)
			continue; /* don't aim at teammate */
		for (j = 0; j < 3; j++)
			end[j] = check->v.origin[j] + 0.5 * (check->v.mins[j] + check->v.maxs[j]);
		VectorSubtract (end, start, dir);
		VectorNormalize (dir);
		dist = DotProduct (dir, pr_global_struct->v_forward);
		if (dist < bestdist)
			continue; /* to far to turn */
		tr = c_ref_SV_Move (start, vec3_origin, vec3_origin, end, false, ent);
		if (tr.ent == check)
		{ /* can shoot at this one */
			bestdist = dist;
			bestent = check;
		}
	}

	if (bestent)
	{
		VectorSubtract (bestent->v.origin, ent->v.origin, dir);
		dist = DotProduct (dir, pr_global_struct->v_forward);
		VectorScale (pr_global_struct->v_forward, dist, end);
		end[2] = dir[2];
		VectorNormalize (end);
		VectorCopy (end, G_VECTOR (OFS_RETURN));
	}
	else
	{
		VectorCopy (bestdir, G_VECTOR (OFS_RETURN));
	}
}

void ctest_cref_pf_walkpathtogoal (void)
{
	G_FLOAT (OFS_RETURN) = 0; /* PATH_ERROR */
}

static unsigned char *ctest_cref_checkpvs;
static int			  ctest_cref_checkpvs_capacity;

static int ctest_cref_pf_newcheckclient (int check)
{
	int			   i;
	unsigned char *pvs;
	edict_t		  *ent;
	mleaf_t		  *leaf;
	vec3_t		   org;
	int			   pvsbytes;

	if (check < 1)
		check = 1;
	if (check > svs.maxclients)
		check = svs.maxclients;

	if (check == svs.maxclients)
		i = 1;
	else
		i = check + 1;

	for (;; i++)
	{
		if (i == svs.maxclients + 1)
			i = 1;

		ent = EDICT_NUM (i);

		if (i == check)
			break;

		if (ent->free)
			continue;
		if (ent->v.health <= 0)
			continue;
		if ((int)ent->v.flags & FL_NOTARGET)
			continue;

		break;
	}

	VectorAdd (ent->v.origin, ent->v.view_ofs, org);
	leaf = Mod_PointInLeaf (org, qcvm->worldmodel);
	pvs = Mod_LeafPVS (leaf, qcvm->worldmodel);

	pvsbytes = (qcvm->worldmodel->numleafs + 31) >> 3;
	if (ctest_cref_checkpvs == NULL || pvsbytes > ctest_cref_checkpvs_capacity)
	{
		ctest_cref_checkpvs_capacity = pvsbytes;
		ctest_cref_checkpvs = (unsigned char *)Mem_Realloc (ctest_cref_checkpvs, ctest_cref_checkpvs_capacity);
		if (!ctest_cref_checkpvs)
			Sys_Error ("PF_newcheckclient: realloc() failed on %d bytes", ctest_cref_checkpvs_capacity);
	}
	memcpy (ctest_cref_checkpvs, pvs, pvsbytes);

	return i;
}

void ctest_cref_pf_checkclient (void)
{
	edict_t *ent, *self;
	mleaf_t *leaf;
	int		 l;
	vec3_t	 view;

	if (qcvm->time - ctest_pf_lastchecktime >= 0.1)
	{
		ctest_pf_lastcheck = ctest_cref_pf_newcheckclient (ctest_pf_lastcheck);
		ctest_pf_lastchecktime = qcvm->time;
	}

	ent = EDICT_NUM (ctest_pf_lastcheck);
	if (ent->free || ent->v.health <= 0)
	{
		RETURN_EDICT (qcvm->edicts);
		return;
	}

	self = PROG_TO_EDICT (pr_global_struct->self);
	VectorAdd (self->v.origin, self->v.view_ofs, view);
	leaf = Mod_PointInLeaf (view, qcvm->worldmodel);
	l = (int)(leaf - qcvm->worldmodel->leafs) - 1;
	if ((l < 0) || !(ctest_cref_checkpvs[l >> 3] & (1 << (l & 7))))
	{
		RETURN_EDICT (qcvm->edicts);
		return;
	}

	RETURN_EDICT (ent);
}

void ctest_cref_pf_checkpvs (void)
{
	float		  *org = G_VECTOR (OFS_PARM0);
	edict_t		  *ed = G_EDICT (OFS_PARM1);
	mleaf_t		  *leaf = Mod_PointInLeaf (org, qcvm->worldmodel);
	unsigned char *pvs = Mod_LeafPVS (leaf, qcvm->worldmodel);
	unsigned int   i;

	for (i = 0; i < ed->num_leafs; i++)
	{
		if (pvs[ed->leafnums[i] >> 3] & (1 << (ed->leafnums[i] & 7)))
		{
			G_FLOAT (OFS_RETURN) = true;
			return;
		}
	}

	G_FLOAT (OFS_RETURN) = false;
}

/* Two scratch qmodel_t for PF_setmodel's precache table. They are only ever
 * reached through ctest_pf_set_model_boxes, so nothing else in the model is
 * populated -- setmodel reads type/mins/maxs/clipmins/clipmaxs and nothing
 * else. */
#define CTEST_PF_SCRATCH_MODELS 2
static qmodel_t ctest_pf_scratch_models[CTEST_PF_SCRATCH_MODELS];

void *ctest_pf_model (int idx)
{
	if (idx < 0 || idx >= CTEST_PF_SCRATCH_MODELS)
		return NULL;
	memset (&ctest_pf_scratch_models[idx], 0, sizeof (ctest_pf_scratch_models[idx]));
	return &ctest_pf_scratch_models[idx];
}

/* The oracle side's raise trap. Kept in C so that a Host_Error longjmp out of
 * a ctest_cref_pf_* body never unwinds a Rust frame -- the same rule ADR-009
 * imposes on the port (there the trap is Host_Guard inside PRBI_SvGlue_*). */
static int ctest_cref_pf_which;

static void ctest_cref_pf_dispatch (void *p)
{
	(void)p;
	switch (ctest_cref_pf_which)
	{
	case 0:
		ctest_cref_pf_setorigin ();
		break;
	case 1:
		ctest_cref_pf_setsize ();
		break;
	case 2:
		ctest_cref_pf_setmodel ();
		break;
	case 3:
		ctest_cref_pf_traceline ();
		break;
	case 4:
		ctest_cref_pf_tracebox ();
		break;
	case 5:
		ctest_cref_pf_findradius ();
		break;
	case 6:
		ctest_cref_pf_walkmove ();
		break;
	case 7:
		ctest_cref_pf_droptofloor ();
		break;
	case 8:
		ctest_cref_pf_checkbottom ();
		break;
	case 9:
		ctest_cref_pf_pointcontents ();
		break;
	case 10:
		ctest_cref_pf_aim ();
		break;
	case 11:
		ctest_cref_pf_walkpathtogoal ();
		break;
	case 12:
		ctest_cref_pf_checkclient ();
		break;
	case 13:
		ctest_cref_pf_checkpvs ();
		break;
	default:
		Sys_Error ("ctest_cref_pf_run: bad index %d", ctest_cref_pf_which);
	}
}

int ctest_cref_pf_run (int which)
{
	ctest_cref_pf_which = which;
	return ctest_try_host (ctest_cref_pf_dispatch, NULL);
}

void ctest_pf_set_edict_bits (int num, int float_ofs, unsigned bits)
{
	memcpy (((float *)&EDICT_NUM (num)->v) + float_ofs, &bits, sizeof (bits));
}

/* ---------------------------------------------------------------------------
 * Phase 7 M5 T5.2: ED_ParseGlobals / ED_ParseEdict dispatcher test doubles.
 *
 * pr_edict.c, pr_edict_dispatch_glue.c, pr_edict_load_glue.c and
 * pr_edict_parse_glue.c are not in C_SOURCES (rust/quake-ctest/build.rs is
 * hard-shared -- not edited by this agent), so the handful of symbols the new
 * Rust dispatcher (quake-capi::progs_edict_dispatch) reaches that no earlier
 * phase's tests needed are stubbed here instead, mirroring how ED_FindField /
 * ED_FieldAtOfs above already stand in for pr_edict.c's hash-map versions.
 *
 * ED_FindGlobal/ED_FindField/PREdictDispatch_Glue_FindGlobal/FindField mirror
 * their real bodies exactly (pr_edict.c:94-120, pr_edict_dispatch_glue.c
 * :56-76) -- a linear search is behaviourally equivalent to the hash-map
 * lookup for a synthetic def table. PRLoad_Glue_IsServerVM and
 * PRParse_Glue_UnlinkEdict mirror pr_edict_load_glue.c:150-153 and
 * pr_edict_parse_glue.c:81-83 (the latter via SV_UnlinkEdict, which is
 * already the Rust world.c export on this branch). SV_Precache_Model/
 * SV_Precache_Sound/PF_SV_ForceParticlePrecache are simplified test doubles,
 * not full ports of pr_cmds.c's precache tables: SV_Precache_Model is
 * controllable to exercise both the Host_Guard-OK and Host_Guard-raise paths
 * (Quake/gl_model.c:531's Mod_ForName crash), the other two are leaf
 * recorders (matching the module doc comment's justification for calling
 * them unguarded).
 */

ddef_t *ED_FindGlobal (const char *name)
{
	int i;
	for (i = 0; i < qcvm->progs->numglobaldefs; i++)
	{
		ddef_t *def = &qcvm->globaldefs[i];
		if (!strcmp (PR_GetString (def->s_name), name))
			return def;
	}
	return NULL;
}

qboolean PREdictDispatch_Glue_FindGlobal (const char *name, unsigned short *out_type, unsigned short *out_ofs, int *out_s_name)
{
	ddef_t *def = ED_FindGlobal (name);
	if (!def)
		return false;
	*out_type = def->type;
	*out_ofs = def->ofs;
	*out_s_name = def->s_name;
	return true;
}

qboolean PREdictDispatch_Glue_FindField (const char *name, unsigned short *out_type, unsigned short *out_ofs, int *out_s_name)
{
	ddef_t *def = ED_FindField (name);
	if (!def)
		return false;
	*out_type = def->type;
	*out_ofs = def->ofs;
	*out_s_name = def->s_name;
	return true;
}

qboolean PREdictDispatch_Glue_ServerLoading (void)
{
	return sv.state == ss_loading;
}

qboolean PRLoad_Glue_IsServerVM (qcvm_t *vm)
{
	return vm == &sv.qcvm;
}

void PRParse_Glue_UnlinkEdict (int edict_num)
{
	SV_UnlinkEdict (EDICT_NUM_NO_CHECK (edict_num));
}

static qboolean ctest_predd_model_should_error;
static char	 ctest_predd_last_model[256];
static char	 ctest_predd_last_sound[256];
static char	 ctest_predd_last_particle[256];
static int	 ctest_predd_particle_calls;

/* When armed, SV_Precache_Model Host_Errors instead of returning -- the only
 * seam PREdictDispatch_Glue_PrecacheModel's Host_Guard exists to catch. */
void ctest_predd_set_model_error (qboolean should_error)
{
	ctest_predd_model_should_error = should_error;
}

int SV_Precache_Model (const char *s)
{
	q_strlcpy (ctest_predd_last_model, s ? s : "", sizeof (ctest_predd_last_model));
	if (ctest_predd_model_should_error)
		Host_Error ("ctest SV_Precache_Model: model not found: %s", s ? s : "(null)");
	return 1;
}

/* Host_Guard-wrapped SV_Precache_Model, mirroring pr_edict_dispatch_glue.c
 * :87-107 (arg struct + static invoke fn + Host_Guard call) exactly, over
 * the test double above instead of the real function. */
typedef struct
{
	const char *s;
	int		   *out;
} ctest_predd_precache_model_arg_t;

static void ctest_predd_invoke_precache_model (void *p)
{
	ctest_predd_precache_model_arg_t *a = (ctest_predd_precache_model_arg_t *)p;
	*a->out = SV_Precache_Model (a->s);
}

int PREdictDispatch_Glue_PrecacheModel (const char *s, int *out)
{
	ctest_predd_precache_model_arg_t arg;

	arg.s = s;
	arg.out = out;
	*out = 0;
	return Host_Guard (ctest_predd_invoke_precache_model, &arg);
}

int SV_Precache_Sound (const char *s)
{
	q_strlcpy (ctest_predd_last_sound, s ? s : "", sizeof (ctest_predd_last_sound));
	return 1;
}

int PF_SV_ForceParticlePrecache (const char *s)
{
	ctest_predd_particle_calls++;
	q_strlcpy (ctest_predd_last_particle, s ? s : "", sizeof (ctest_predd_last_particle));
	return ctest_predd_particle_calls;
}

const char *ctest_predd_get_last_model (void) { return ctest_predd_last_model; }
const char *ctest_predd_get_last_sound (void) { return ctest_predd_last_sound; }
const char *ctest_predd_get_last_particle (void) { return ctest_predd_last_particle; }
int			ctest_predd_get_particle_calls (void) { return ctest_predd_particle_calls; }

void ctest_predd_reset_doubles (void)
{
	ctest_predd_model_should_error = false;
	ctest_predd_last_model[0] = '\0';
	ctest_predd_last_sound[0] = '\0';
	ctest_predd_last_particle[0] = '\0';
	ctest_predd_particle_calls = 0;
}

/* Installs a synthetic def table directly on &sv.qcvm -- PRLoad_Glue_IsServerVM
 * gates the dispatcher's `_precache_model`/`_precache_sound`/`traileffect`/
 * `emiteffect` branches on `qcvm == &sv.qcvm` pointer identity (as does
 * sv_phys.c's own server-only half, per ctest_world_reset's vm_kind==2
 * comment above), so those branches can only be exercised on the real
 * instance, not a look-alike. Call after ctest_world_reset(2, ...), which
 * builds the rest of the fixture (edicts/globals/strings) but does not touch
 * fielddefs/globaldefs/extfields. Mirrors ctest_progs_set_defs's ownership
 * contract: the caller keeps ownership of nothing, old tables (if any) are
 * freed first. */
void ctest_predd_set_defs (
	const void *fielddefs, int numfielddefs, const void *globaldefs, int numglobaldefs, int extfields_alpha, int extfields_traileffectnum,
	int extfields_emiteffectnum)
{
	qcvm_t *vm = &sv.qcvm;

	if (vm->fielddefs)
		Mem_Free (vm->fielddefs);
	if (vm->globaldefs)
		Mem_Free (vm->globaldefs);

	vm->fielddefs = (ddef_t *)Mem_Alloc ((size_t)numfielddefs * sizeof (ddef_t));
	memcpy (vm->fielddefs, fielddefs, (size_t)numfielddefs * sizeof (ddef_t));
	vm->progs->numfielddefs = numfielddefs;

	vm->globaldefs = (ddef_t *)Mem_Alloc ((size_t)numglobaldefs * sizeof (ddef_t));
	memcpy (vm->globaldefs, globaldefs, (size_t)numglobaldefs * sizeof (ddef_t));
	vm->progs->numglobaldefs = numglobaldefs;

	vm->extfields.alpha = extfields_alpha;
	vm->extfields.traileffectnum = extfields_traileffectnum;
	vm->extfields.emiteffectnum = extfields_emiteffectnum;
}

/* ctest_world_reset points &sv.qcvm's strings at the static ctest_world_strings[64]
 * buffer (not a Mem_Alloc'd one), so ctest_predd_set_defs's fielddef/globaldef
 * names need somewhere bigger to live than that fixed 64-byte array leaves free.
 * Swaps in a heap-allocated strings blob the same way ctest_progs_synth_vm does
 * for the plain fixtures; the caller keeps ownership of nothing. Safe to call
 * more than once per test (the previous heap buffer, if any, is freed first --
 * the initial static buffer from ctest_world_reset is never freed). */
static char *ctest_predd_owned_strings;

void ctest_predd_set_strings (const void *data, int size)
{
	qcvm_t *vm = &sv.qcvm;

	if (ctest_predd_owned_strings)
		Mem_Free (ctest_predd_owned_strings);

	ctest_predd_owned_strings = (char *)Mem_Alloc ((size_t)size);
	memcpy (ctest_predd_owned_strings, data, (size_t)size);
	vm->strings = ctest_predd_owned_strings;
	vm->stringssize = size;
}

/* quake_rs_ed_parse_globals/quake_rs_ed_parse_edict (the dispatchers under
 * test) are called directly from the Rust test file via its own `extern "C"`
 * block, matching progs_parse_differential.rs's precedent for
 * quake_rs_ed_parse_epair -- no C-side wrapper needed. */

/* The pr_edict_parse.c platform conversions and engine lookups. These mirror
 * pr_edict_parse_glue.c:39-70 exactly -- ADR-010 makes the platform's own
 * atof/strtoll rounding the contract, so the oracle must call the same libc
 * entry points the shipping glue does, not a reimplementation. */

double PRParse_Glue_Atof (const char *s)
{
	return atof (s);
}

int PRParse_Glue_Atoi (const char *s)
{
	return atoi (s);
}

long long PRParse_Glue_Strtoll (const char *s)
{
	return strtoll (s, NULL, 0);
}

unsigned long long PRParse_Glue_Strtoull (const char *s)
{
	return strtoull (s, NULL, 0);
}

int PRParse_Glue_FindFieldOfs (const char *name)
{
	ddef_t *def = ED_FindField (name);
	return def ? (int)def->ofs : -1;
}

int PRParse_Glue_FindFunction (const char *name)
{
	dfunction_t *f = ED_FindFunction (name);
	return f ? (int)(f - qcvm->functions) : -1;
}

/* ---------------------------------------------------------------------------
 * Phase 7 M6: the engine symbols Quake/sv_main.c, Quake/sv_send.c and
 * Quake/sv_user.c reference but no oracle source defines. Everything here
 * belongs to a file that is still outside C_SOURCES (host.c, net.c/net_main.c,
 * gl_model.c, pr_edict.c, view.c, gl_screen.c), so the names stay un-renamed.
 *
 * These are link-satisfying doubles, not behaviour: the M6 differential suites
 * drive the ported functions along paths that do not reach them, and any suite
 * that needs one is expected to replace it with a recorder the way the M4/M5
 * groups did. Sys_Error rather than a silent return wherever a wrong answer
 * would be indistinguishable from a real one.
 */

double realtime = 0.0;
int	   current_skill = 0;

cvar_t max_edicts = {"max_edicts", "8192", CVAR_NONE, 8192.0f, NULL, NULL, NULL, NULL};
cvar_t skill = {"skill", "1", CVAR_NONE, 1.0f, NULL, NULL, NULL, NULL};
cvar_t deathmatch = {"deathmatch", "0", CVAR_NONE, 0.0f, NULL, NULL, NULL, NULL};
cvar_t coop = {"coop", "0", CVAR_NONE, 0.0f, NULL, NULL, NULL, NULL};
cvar_t nomonsters = {"nomonsters", "0", CVAR_NONE, 0.0f, NULL, NULL, NULL, NULL};
cvar_t r_lerpmove = {"r_lerpmove", "1", CVAR_ARCHIVE, 1.0f, NULL, NULL, NULL, NULL};
cvar_t devstats = {"devstats", "0", CVAR_NONE, 0.0f, NULL, NULL, NULL, NULL};

devstats_t		dev_stats, dev_peakstats;
overflowtimes_t dev_overflows;

const builtin_t pr_ssqcbuiltins[1] = {NULL};
const int		pr_ssqcnumbuiltins = 0;

void Host_Callback_Notify (cvar_t *var)
{
	(void)var;
}

void SCR_CenterPrintClear (void) {}

void Host_ClearMemory (void) {}

void SV_DropClient (qboolean crash)
{
	(void)crash;
	Sys_Error ("ctest: SV_DropClient reached (host.c is not an oracle source)");
}

/* Quake/sv_user.c:444 calls V_CalcRoll inside SV_ClientThink -- view.c:84 says
 * so itself ("Used by view and sv_user"). The T6.0 abort stub therefore killed
 * every sv_user differential the moment it reached SV_ClientThink.
 *
 * T7.2 ported view.c, so this is no longer a hand transcription: it forwards
 * to the Rust core, which is exactly what the shipped -Duse_rust_host build
 * does. Phase 7 M7 (T7.0) made view.c
 * an oracle source, so this is no longer the only V_CalcRoll in the link: the
 * oracle SV_ClientThink now calls c_ref_V_CalcRoll (real view.c) while the Rust
 * SV_ClientThink still calls this one through quake-c-sys/src/sv_user.rs. The
 * `#undef`s below are what keep this copy un-renamed; without them it would be
 * a second definition of c_ref_V_CalcRoll / c_ref_cl_rollspeed /
 * c_ref_cl_rollangle. This copy goes away when view.c itself is ported.
 *
 * A degenerate stub is the hazard here: cl_rollangle.value scales the result,
 * and cvar_t.value is populated by Cvar_RegisterVariable, which never runs in
 * the ctest link. Defining the cvars without an explicit .value would silently
 * return 0 for every input and flatten the ROLL path that sv_user.c:444 feeds
 * into the client's angle state. view.c declares its pair WITHOUT an explicit
 * .value (the engine fills it in at registration), so the c_ref copies are
 * seeded by ctest_svuser_reset (sv_user_ref.c) instead -- otherwise the two
 * sides would differ for a reason that has nothing to do with SV_ClientThink.
 */
#undef cl_rollspeed
#undef cl_rollangle
#undef V_CalcRoll
cvar_t cl_rollspeed = {"cl_rollspeed", "200", CVAR_NONE, 200.0f, NULL, NULL, NULL, NULL};
cvar_t cl_rollangle = {"cl_rollangle", "2.0", CVAR_ARCHIVE, 2.0f, NULL, NULL, NULL, NULL};

extern float quake_rs_v_calc_roll (float *angles, float *velocity);

float V_CalcRoll (vec3_t angles, vec3_t velocity)
{
	return quake_rs_v_calc_roll (angles, velocity);
}
#define cl_rollspeed c_ref_cl_rollspeed
#define cl_rollangle c_ref_cl_rollangle
#define V_CalcRoll	 c_ref_V_CalcRoll

qmodel_t *Mod_ForName (const char *name, qboolean crash)
{
	(void)name;
	(void)crash;
	Sys_Error ("ctest: Mod_ForName reached (gl_model.c is not an oracle source)");
	return NULL;
}

qboolean PR_LoadProgs (const char *filename, qboolean fatal, unsigned int needcrc, const builtin_t *builtins, size_t numbuiltins)
{
	(void)filename;
	(void)fatal;
	(void)needcrc;
	(void)builtins;
	(void)numbuiltins;
	Sys_Error ("ctest: PR_LoadProgs reached (pr_edict.c is not an oracle source)");
	return false;
}

void ED_LoadFromFile (const char *data)
{
	(void)data;
	Sys_Error ("ctest: ED_LoadFromFile reached (pr_edict.c is not an oracle source)");
}

/* net.h:53-91. net_main.c is not an oracle source. The "no connection" /
 * "nothing to send" answers are the only ones a fixture with no qsocket can
 * honestly give; the two that hand back a buffer Sys_Error instead. */
struct qsocket_s *NET_CheckNewConnections (void)
{
	return NULL;
}

struct qsocket_s *NET_GetServerMessage (void)
{
	return NULL;
}

const char *NET_QSocketGetTrueAddressString (const struct qsocket_s *sock)
{
	(void)sock;
	return "ctest";
}

qboolean NET_QSocketGetProQuakeAngleHack (const struct qsocket_s *sock)
{
	(void)sock;
	return false;
}

int NET_QSocketGetSequenceOut (const struct qsocket_s *sock)
{
	(void)sock;
	return 0;
}

void NET_QSocketSetMSS (struct qsocket_s *s, int mss)
{
	(void)s;
	(void)mss;
}

qboolean NET_CanSendMessage (struct qsocket_s *sock)
{
	(void)sock;
	return true;
}

/* Datagram recorder (Rust migration Phase 7, M6).
 *
 * sv_send.c hands every finished client datagram to NET_SendMessage /
 * NET_SendUnreliableMessage. While these stubs just returned 1, the sv_send
 * differentials could only compare how many bytes a side *claimed* to send --
 * so a port emitting the right number of wrong bytes passed. The recorder
 * keeps the full send log (order, per-call size, reliability, payload) for the
 * tests to byte-compare.
 *
 * Both sides of a differential write through this same recorder by design;
 * ctest_net_send_reset() between the two runs is the test's responsibility.
 * These are not c_ref_* oracle symbols -- stubs.c is not in build.rs's
 * C_SOURCES, so check_ctest_symbols.sh does not police this file.
 *
 * Truncation is reported rather than silently absorbed: two sides that both
 * overflow the log would otherwise compare equal on a prefix.
 */
#define CTEST_NET_LOG_CAP	(1 << 20)
#define CTEST_NET_CALLS_CAP 1024

static byte ctest_net_log[CTEST_NET_LOG_CAP];
static int	ctest_net_log_len;
static int	ctest_net_call_len[CTEST_NET_CALLS_CAP];
static byte ctest_net_call_rel[CTEST_NET_CALLS_CAP];
static int	ctest_net_calls;
static int	ctest_net_truncated;

static void ctest_net_record (sizebuf_t *data, int reliable)
{
	int len = data ? data->cursize : 0;

	if (ctest_net_calls < CTEST_NET_CALLS_CAP)
	{
		ctest_net_call_len[ctest_net_calls] = len;
		ctest_net_call_rel[ctest_net_calls] = (byte)reliable;
	}
	else
		ctest_net_truncated = 1;
	ctest_net_calls++;

	if (len <= 0)
		return;
	if (ctest_net_log_len + len <= CTEST_NET_LOG_CAP)
	{
		memcpy (ctest_net_log + ctest_net_log_len, data->data, (size_t)len);
		ctest_net_log_len += len;
	}
	else
		ctest_net_truncated = 1;
}

void ctest_net_send_reset (void)
{
	ctest_net_log_len = 0;
	ctest_net_calls = 0;
	ctest_net_truncated = 0;
}

int ctest_net_send_calls (void)
{
	return ctest_net_calls;
}

const unsigned char *ctest_net_send_bytes (int *len)
{
	if (len)
		*len = ctest_net_log_len;
	return ctest_net_log;
}

/* -1 for an index past what was retained; the caller must treat that as a
 * mismatch rather than as "no data". */
int ctest_net_send_call_len (int i)
{
	if (i < 0 || i >= ctest_net_calls || i >= CTEST_NET_CALLS_CAP)
		return -1;
	return ctest_net_call_len[i];
}

int ctest_net_send_call_reliable (int i)
{
	if (i < 0 || i >= ctest_net_calls || i >= CTEST_NET_CALLS_CAP)
		return -1;
	return ctest_net_call_rel[i];
}

int ctest_net_send_truncated (void)
{
	return ctest_net_truncated;
}

int NET_SendMessage (struct qsocket_s *sock, sizebuf_t *data)
{
	(void)sock;
	ctest_net_record (data, 1);
	return 1;
}

int NET_SendUnreliableMessage (struct qsocket_s *sock, sizebuf_t *data)
{
	(void)sock;
	ctest_net_record (data, 0);
	return 1;
}

int NET_SendToAll (sizebuf_t *data, double blocktime)
{
	(void)data;
	(void)blocktime;
	return 0;
}

/* ---------------------------------------------------------------------------
 * Phase 7 M6 link proof.
 *
 * MSVC pulls an archive member only when something references one of its
 * symbols, so a clean `cargo build` says nothing about whether sv_user.o,
 * sv_send.o and sv_main.o can actually be linked into a test binary. This
 * calls one real entry point from each of the three, chosen so that each one
 * returns on its first branch and touches no fixture state:
 *
 *   sv_main.c:824 SV_ModelIndex   -- `!name || !name[0]` returns 0.
 *   sv_send.c:2206 SV_CreateBaseline -- loop bound is qcvm->num_edicts, 0 here.
 *   sv_user.c:417 SV_ClientThink  -- `movetype == MOVETYPE_NONE` returns.
 *
 * Returns a bitmask so the Rust side can tell which arms ran; a wrong
 * SV_ModelIndex answer clears bit 0 rather than being swallowed.
 */
int ctest_m6_linkproof (void)
{
	static qcvm_t  linkproof_vm;
	static edict_t linkproof_ent;
	qcvm_t		  *saved_vm = qcvm;
	edict_t		  *saved_player = sv_player;
	int			   result = 0;

	if (SV_ModelIndex (NULL) == 0 && SV_ModelIndex ("") == 0)
		result |= 1;

	memset (&linkproof_vm, 0, sizeof (linkproof_vm));
	qcvm = &linkproof_vm;
	SV_CreateBaseline ();
	qcvm = saved_vm;
	result |= 2;

	memset (&linkproof_ent, 0, sizeof (linkproof_ent));
	linkproof_ent.v.movetype = MOVETYPE_NONE;
	sv_player = &linkproof_ent;
	SV_ClientThink ();
	sv_player = saved_player;
	result |= 4;

	return result;
}

/* ===========================================================================
 * Phase 7 M7 (T7.0): the client stratum's outward seam.
 *
 * chase.c, cl_demo.c, cl_input.c, cl_main.c, cl_parse.c, cl_tent.c and view.c
 * became oracle sources. Everything below is a symbol THEY reference that no
 * oracle source and no existing stub defines -- the renderer, the particle
 * script, the sky/fog layer, the CD player, the loading plaque, the net
 * transport, and a handful of engine globals owned by files that are still not
 * compiled here. The list is exactly link.exe's LNK2019/LNK2001 set for the
 * seven new objects, not a guess.
 *
 * None of these names is renamed: the files that define them for real
 * (gl_rmain.c, r_part.c, gl_screen.c, console.c, net_main.c, ...) are not
 * oracle sources, so there is one plain copy and both sides of every
 * differential reach it -- the same arrangement M4/M6 use for sv_friction and
 * friends.
 *
 * The functions abort rather than returning a plausible value. A silent no-op
 * here is the vacuous-gate shape this milestone has already been bitten by
 * twice: a differential that reaches one of these would compare two identical
 * nothings and pass. A later M7 task that needs one of these paths must
 * replace the abort with a fixture that both sides observe, not delete it.
 * ======================================================================== */

/* --- engine globals -------------------------------------------------------
 * cvar_t .value is normally filled in by Cvar_RegisterVariable, which never
 * runs for these in the ctest link, so each one carries its real default
 * explicitly -- a zeroed .value would flatten the branch that reads it on BOTH
 * sides at once (r_lerpmodels/r_lerpturn gate CL_RelinkEntities' lerp path;
 * scr_viewsize gates view.c's gun-offset math). */
refdef_t r_refdef;
viddef_t vid;
vec3_t	 vpn;

int		 host_framecount;
float	 host_netinterval; /* host.c; cl_main.c:71 declares it float, not cvar_t */
float	 scr_clock_off;	   /* gl_screen.c; cl_demo.c:213 declares it float */
qboolean noclip_anglehack;
qboolean con_forcedup;
char	 con_lastcenterstring[1024]; /* console.c:63 */
int		 r_trace_line_cache_counter;
int		 render_scale;
qboolean render_warp;

cvar_t r_lerpmodels = {"r_lerpmodels", "1", CVAR_ARCHIVE, 1.0f, NULL, NULL, NULL, NULL};
cvar_t r_lerpturn = {"r_lerpturn", "1", CVAR_ARCHIVE, 1.0f, NULL, NULL, NULL, NULL};
cvar_t scr_viewsize = {"viewsize", "100", CVAR_ARCHIVE, 100.0f, NULL, NULL, NULL, NULL};

/* gl_model.c:52-53. cl_parse.c:1673-1674 declares mod_known as an incomplete
 * array and only ever walks [0, mod_numknown), so the extent here is a harness
 * choice, not gl_model.c's MAX_MODELS (8192 qmodel_t is ~megabytes of BSS for
 * an array no ctest path populates). mod_numknown stays 0; a later task that
 * wants CL_RegisterParticles to iterate must raise BOTH together. */
qmodel_t mod_known[4];
int		 mod_numknown = 0;

/* --- renderer (gl_rmain.c / r_part.c / gl_sky.c / gl_fog.c) --------------- */
void R_RenderView (qboolean use_tasks, task_handle_t begin_rendering_task, task_handle_t setup_frame_task, task_handle_t draw_done_task)
{
	(void)use_tasks;
	(void)begin_rendering_task;
	(void)setup_frame_task;
	(void)draw_done_task;
	Sys_Error ("ctest: R_RenderView reached (gl_rmain.c is not an oracle source)");
}

void R_NewMap (void)
{
	Sys_Error ("ctest: R_NewMap reached (gl_rmain.c is not an oracle source)");
}

void R_CheckEfrags (void)
{
	Sys_Error ("ctest: R_CheckEfrags reached (r_efrag.c is not an oracle source)");
}

void R_AddEfrags (entity_t *ent)
{
	(void)ent;
	Sys_Error ("ctest: R_AddEfrags reached (r_efrag.c is not an oracle source)");
}

/* T7.4: cl_main.c:935 calls this at the tail of CL_RelinkEntities, so the
 * SCR_UpdateZoom counting stub above put it back on the live path. The real
 * gl_rlight.c:421 body early-returns unless vulkan_globals.ray_query and
 * r_rtshadows >= 2 and r_gpulightmapupdate are all set -- none of which the
 * ctest link ever sets -- so counting here is behaviourally faithful, not a
 * shortcut. Counting rather than no-oping keeps the seam observable, per the
 * R_FreeEntityBLAS precedent. */
static int ctest_entity_dlight_updates = 0;

void R_UpdateEntityDlights (void)
{
	ctest_entity_dlight_updates++;
}

void ctest_entity_dlights_reset (void)
{
	ctest_entity_dlight_updates = 0;
}

int ctest_entity_dlights_count (void)
{
	return ctest_entity_dlight_updates;
}

void R_TranslatePlayerSkin (int playernum)
{
	(void)playernum;
	Sys_Error ("ctest: R_TranslatePlayerSkin reached (gl_rmisc.c is not an oracle source)");
}

void R_TranslateNewPlayerSkin (int playernum)
{
	(void)playernum;
	Sys_Error ("ctest: R_TranslateNewPlayerSkin reached (gl_rmisc.c is not an oracle source)");
}

void R_AllocateEntityBLAS (entity_t *e)
{
	(void)e;
	Sys_Error ("ctest: R_AllocateEntityBLAS reached (gl_rmisc.c is not an oracle source)");
}

/* Phase 7 M7 (T7.3): this was an abort stub, which made every model change in
 * CL_ParseUpdate / CL_ParseBaseline unreachable -- the two largest functions
 * in cl_parse.c could not be compared past their first model swap. gl_rmisc.c
 * is not an oracle source, so BOTH sides call this one definition; counting
 * the calls keeps the seam observable (a port that skipped the free would
 * show a different count) without inventing GPU state the harness has none
 * of. Nothing else in the link reaches it. */
static int ctest_blas_frees = 0;

void R_FreeEntityBLAS (entity_t *e)
{
	(void)e;
	ctest_blas_frees++;
}

void ctest_blas_free_reset (void)
{
	ctest_blas_frees = 0;
}

int ctest_blas_free_count (void)
{
	return ctest_blas_frees;
}

void R_ClearParticles (void)
{
	Sys_Error ("ctest: R_ClearParticles reached (r_part.c is not an oracle source)");
}

void R_ParseParticleEffect (void)
{
	Sys_Error ("ctest: R_ParseParticleEffect reached (r_part.c is not an oracle source)");
}

void R_EntityParticles (entity_t *ent)
{
	(void)ent;
	Sys_Error ("ctest: R_EntityParticles reached (r_part.c is not an oracle source)");
}

void R_BlobExplosion (vec3_t org)
{
	(void)org;
	Sys_Error ("ctest: R_BlobExplosion reached (r_part.c is not an oracle source)");
}

void R_ParticleExplosion (vec3_t org)
{
	(void)org;
	Sys_Error ("ctest: R_ParticleExplosion reached (r_part.c is not an oracle source)");
}

void R_ParticleExplosion2 (vec3_t org, int colorStart, int colorLength)
{
	(void)org;
	(void)colorStart;
	(void)colorLength;
	Sys_Error ("ctest: R_ParticleExplosion2 reached (r_part.c is not an oracle source)");
}

void R_LavaSplash (vec3_t org)
{
	(void)org;
	Sys_Error ("ctest: R_LavaSplash reached (r_part.c is not an oracle source)");
}

void R_TeleportSplash (vec3_t org)
{
	(void)org;
	Sys_Error ("ctest: R_TeleportSplash reached (r_part.c is not an oracle source)");
}

void R_RocketTrail (vec3_t start, vec3_t end, int type)
{
	(void)start;
	(void)end;
	(void)type;
	Sys_Error ("ctest: R_RocketTrail reached (r_part.c is not an oracle source)");
}

/* --- PScript (gl_pscript.c, the PSET_SCRIPT arm of glquake.h) ------------- */
void PScript_Shutdown (void)
{
	Sys_Error ("ctest: PScript_Shutdown reached (gl_pscript.c is not an oracle source)");
}

void PScript_ClearParticles (qboolean load)
{
	(void)load;
	Sys_Error ("ctest: PScript_ClearParticles reached (gl_pscript.c is not an oracle source)");
}

void PScript_DelinkTrailstate (struct trailstate_s **tsk)
{
	(void)tsk;
	Sys_Error ("ctest: PScript_DelinkTrailstate reached (gl_pscript.c is not an oracle source)");
}

int PScript_FindParticleType (const char *fullname)
{
	(void)fullname;
	Sys_Error ("ctest: PScript_FindParticleType reached (gl_pscript.c is not an oracle source)");
	return -1;
}

/* T7.4: cl_main.c:857 and :893 call these from inside the CL_RelinkEntities
 * entity loop, which the SCR_UpdateZoom counting stub put back on the live
 * path. As abort stubs they made every entity with a model unreachable past
 * the lerp, and they are the only place CL_RelinkEntities' clamped frametime
 * escapes to anything observable -- so recording rather than aborting is what
 * makes cl_main.c:670 testable at all. gl_pscript.c is not an oracle source,
 * so BOTH sides call these definitions; the ctest_pscript_* accessors below
 * are reset per side by the fixtures. Recording, not no-oping, per the
 * R_FreeEntityBLAS precedent. */
static int	 ctest_pscript_trail_calls = 0;
static float ctest_pscript_last_timeinterval = 0.0f;
static int	 ctest_pscript_state_calls = 0;
static float ctest_pscript_last_count = 0.0f;

int PScript_ParticleTrail (vec3_t startpos, vec3_t end, int type, float timeinterval, int dlkey, vec3_t axis[3], struct trailstate_s **tsk)
{
	(void)startpos;
	(void)end;
	(void)type;
	(void)dlkey;
	(void)axis;
	(void)tsk;
	ctest_pscript_trail_calls++;
	ctest_pscript_last_timeinterval = timeinterval;
	return 1;
}

int PScript_RunParticleEffectState (vec3_t org, vec3_t dir, float count, int typenum, struct trailstate_s **tsk)
{
	(void)org;
	(void)dir;
	(void)typenum;
	(void)tsk;
	ctest_pscript_state_calls++;
	ctest_pscript_last_count = count;
	return 1;
}

void ctest_pscript_reset (void)
{
	ctest_pscript_trail_calls = 0;
	ctest_pscript_last_timeinterval = 0.0f;
	ctest_pscript_state_calls = 0;
	ctest_pscript_last_count = 0.0f;
}

int ctest_pscript_trail_count (void)
{
	return ctest_pscript_trail_calls;
}

float ctest_pscript_last_timeinterval_value (void)
{
	return ctest_pscript_last_timeinterval;
}

int ctest_pscript_state_count (void)
{
	return ctest_pscript_state_calls;
}

void PScript_RunParticleWeather (vec3_t minb, vec3_t maxb, vec3_t dir, float count, int colour, const char *efname)
{
	(void)minb;
	(void)maxb;
	(void)dir;
	(void)count;
	(void)colour;
	(void)efname;
	Sys_Error ("ctest: PScript_RunParticleWeather reached (gl_pscript.c is not an oracle source)");
}

int PScript_EntParticleTrail (vec3_t oldorg, entity_t *ent, const char *name)
{
	(void)oldorg;
	(void)ent;
	(void)name;
	Sys_Error ("ctest: PScript_EntParticleTrail reached (gl_pscript.c is not an oracle source)");
	return 1;
}

/* --- sky / fog (gl_sky.c, gl_fog.c) -------------------------------------- */
void Fog_ParseServerMessage (void)
{
	Sys_Error ("ctest: Fog_ParseServerMessage reached (gl_fog.c is not an oracle source)");
}

const char *Fog_GetFogCommand (qboolean always)
{
	(void)always;
	Sys_Error ("ctest: Fog_GetFogCommand reached (gl_fog.c is not an oracle source)");
	return NULL;
}

void Fog_NewMap (void)
{
	Sys_Error ("ctest: Fog_NewMap reached (gl_fog.c is not an oracle source)");
}

void Sky_NewMap (void)
{
	Sys_Error ("ctest: Sky_NewMap reached (gl_sky.c is not an oracle source)");
}

void Sky_LoadSkyBox (const char *name)
{
	(void)name;
	Sys_Error ("ctest: Sky_LoadSkyBox reached (gl_sky.c is not an oracle source)");
}

const char *Sky_GetSkyCommand (qboolean always)
{
	(void)always;
	Sys_Error ("ctest: Sky_GetSkyCommand reached (gl_sky.c is not an oracle source)");
	return NULL;
}

/* --- console / screen / keys / input / cd (console.c, gl_screen.c, keys.c,
 * in_sdl.c, cd_sdl.c) ------------------------------------------------------ */
void Con_AddToTabList (const char *name, const char *partial, const char *type)
{
	(void)name;
	(void)partial;
	(void)type;
	Sys_Error ("ctest: Con_AddToTabList reached (console.c is not an oracle source)");
}

void Con_LinkPrintf (const char *addr, const char *fmt, ...)
{
	(void)addr;
	(void)fmt;
	Sys_Error ("ctest: Con_LinkPrintf reached (console.c is not an oracle source)");
}

void Con_LogCenterPrint (const char *str)
{
	(void)str;
	Sys_Error ("ctest: Con_LogCenterPrint reached (console.c is not an oracle source)");
}

const char *Con_Quakebar (int len)
{
	(void)len;
	Sys_Error ("ctest: Con_Quakebar reached (console.c is not an oracle source)");
	return NULL;
}

void SCR_CenterPrint (const char *str)
{
	(void)str;
	Sys_Error ("ctest: SCR_CenterPrint reached (gl_screen.c is not an oracle source)");
}

void SCR_BeginLoadingPlaque (void)
{
	Sys_Error ("ctest: SCR_BeginLoadingPlaque reached (gl_screen.c is not an oracle source)");
}

void SCR_EndLoadingPlaque (void)
{
	Sys_Error ("ctest: SCR_EndLoadingPlaque reached (gl_screen.c is not an oracle source)");
}

/* T7.4: cl_main.c:681 calls this from the middle of CL_RelinkEntities, between
 * the velocity interpolation and the demo angle interpolation, bobjrotate and
 * the whole entity loop. As a Sys_Error stub it made every one of those
 * unreachable in the ctest link, which silently hid the teleport threshold and
 * the frametime clamp from mutation testing. gl_screen.c is not an oracle
 * source, so BOTH sides call this one definition; counting rather than
 * no-oping keeps the seam observable, per the R_FreeEntityBLAS precedent
 * above. */
static int ctest_scr_zoom_updates = 0;

void SCR_UpdateZoom (void)
{
	ctest_scr_zoom_updates++;
}

void ctest_scr_zoom_reset (void)
{
	ctest_scr_zoom_updates = 0;
}

int ctest_scr_zoom_count (void)
{
	return ctest_scr_zoom_updates;
}

void Key_ClearStates (void)
{
	Sys_Error ("ctest: Key_ClearStates reached (keys.c is not an oracle source)");
}

void Key_EndChat (void)
{
	Sys_Error ("ctest: Key_EndChat reached (keys.c is not an oracle source)");
}

void IN_ClearStates (void)
{
	Sys_Error ("ctest: IN_ClearStates reached (in_sdl.c is not an oracle source)");
}

void IN_Move (usercmd_t *cmd)
{
	(void)cmd;
	Sys_Error ("ctest: IN_Move reached (in_sdl.c is not an oracle source)");
}

void CDAudio_Pause (void)
{
	Sys_Error ("ctest: CDAudio_Pause reached (cd_sdl.c is not an oracle source)");
}

void CDAudio_Resume (void)
{
	Sys_Error ("ctest: CDAudio_Resume reached (cd_sdl.c is not an oracle source)");
}

void CDAudio_Stop (void)
{
	Sys_Error ("ctest: CDAudio_Stop reached (cd_sdl.c is not an oracle source)");
}

/* --- host / world / net / progs / platform ------------------------------- */
void Host_ShutdownServer (qboolean crash)
{
	(void)crash;
	Sys_Error ("ctest: Host_ShutdownServer reached (host.c is not an oracle source)");
}

void DemoList_Rebuild (void)
{
	Sys_Error ("ctest: DemoList_Rebuild reached (host_cmd.c's demo list is not an oracle source)");
}

void Harness_DemoEnded (void)
{
	Sys_Error ("ctest: Harness_DemoEnded reached (harness.c is not an oracle source)");
}

float CL_TraceLine (vec3_t start, vec3_t end, vec3_t impact, vec3_t normal, int *ent)
{
	(void)start;
	(void)end;
	(void)impact;
	(void)normal;
	(void)ent;
	Sys_Error ("ctest: CL_TraceLine reached (world.c's client trace is not an oracle source)");
	return 0.0f;
}

void Mod_TouchModel (const char *name)
{
	(void)name;
	Sys_Error ("ctest: Mod_TouchModel reached (gl_model.c is not an oracle source)");
}

void PR_ClearProgs (qcvm_t *vm)
{
	(void)vm;
	Sys_Error ("ctest: PR_ClearProgs reached (pr_edict.c's loader is not an oracle source)");
}

const char *Info_GetKey (const char *info, const char *key, char *out, size_t outsize)
{
	(void)info;
	(void)key;
	(void)out;
	(void)outsize;
	Sys_Error ("ctest: Info_GetKey reached (common.c's info layer is not an oracle source)");
	return NULL;
}

void Info_Enumerate (const char *info, void (*cb) (void *ctx, const char *key, const char *value), void *cbctx)
{
	(void)info;
	(void)cb;
	(void)cbctx;
	Sys_Error ("ctest: Info_Enumerate reached (common.c's info layer is not an oracle source)");
}

struct qsocket_s *NET_Connect (const char *host)
{
	(void)host;
	Sys_Error ("ctest: NET_Connect reached (net_main.c is not an oracle source)");
	return NULL;
}

void NET_Close (struct qsocket_s *sock)
{
	(void)sock;
	Sys_Error ("ctest: NET_Close reached (net_main.c is not an oracle source)");
}

int NET_GetMessage (struct qsocket_s *sock)
{
	(void)sock;
	Sys_Error ("ctest: NET_GetMessage reached (net_main.c is not an oracle source)");
	return -1;
}

int NET_QSocketGetSequenceIn (const struct qsocket_s *sock)
{
	(void)sock;
	Sys_Error ("ctest: NET_QSocketGetSequenceIn reached (net_main.c is not an oracle source)");
	return 0;
}

qboolean Steam_SetAchievement (const char *name)
{
	(void)name;
	Sys_Error ("ctest: Steam_SetAchievement reached (steam.c is not an oracle source)");
	return false;
}

int SDL_SetClipboardText (const char *text)
{
	(void)text;
	Sys_Error ("ctest: SDL_SetClipboardText reached (SDL is not linked into the ctest harness)");
	return -1;
}

/* ---------------------------------------------------------------------------
 * Phase 7 M7 (T7.0) link proof -- see rust/quake-ctest/tests/
 * cl_stratum_linkproof.rs and the M6 twin above. One real entry point per new
 * oracle translation unit, each OR-ing a distinct bit, so a green
 * `cargo build --tests` is not mistaken for evidence that the seven objects
 * actually made it into the binary.
 * ------------------------------------------------------------------------ */
/* These five have no header in the engine (chase.c, cl_tent.c and view.c
 * declare them locally where they are used), so the c_ref_* twins are
 * declared by hand -- the rename macros are in effect, so the bare names
 * below expand to the oracle's. */
extern cvar_t chase_back;
extern cvar_t chase_up;
extern int	  num_temp_entities;
extern cvar_t cl_rollangle;
extern cvar_t cl_rollspeed;

/* client.h declares Chase_Init/CL_Stop_f/CL_AllocDlight/CL_UpdateTEnts but
 * not these two (cl_parse.c and cl_input.c use them file-locally). Without
 * the declarations MSVC assumes int-returning (C4013) and the proof reads a
 * garbage eax instead of the real pointer/float return -- which is exactly
 * how the CL_EntityNum arm first failed. */
extern entity_t *CL_EntityNum (int num);
extern float	 CL_KeyState (kbutton_t *key);

int ctest_m7_linkproof (void)
{
	static entity_t	 linkproof_cl_entities[2];
	static kbutton_t linkproof_button;
	vec3_t			 linkproof_angles = {0.0f, 0.0f, 0.0f};
	vec3_t			 linkproof_velocity = {0.0f, 200.0f, 0.0f};
	entity_t		*saved_entities = cl.entities;
	int				 saved_num_entities = cl.num_entities;
	int				 saved_max_edicts = cl.max_edicts;
	int				 result = 0;

	/* chase.c */
	Chase_Init ();
	if (chase_back.value == 100.0f && chase_up.value == 16.0f)
		result |= 1;

	/* cl_demo.c -- cls.demorecording is false, so this is the "Not recording a
	 * demo." early return; nothing observable, same as the M6 proof's
	 * SV_CreateBaseline arm. */
	CL_Stop_f ();
	result |= 2;

	/* cl_input.c */
	memset (&linkproof_button, 0, sizeof (linkproof_button));
	linkproof_button.state = 1; /* down, no impulses -> held the entire frame */
	if (CL_KeyState (&linkproof_button) == 1.0f)
		result |= 4;

	/* cl_main.c */
	if (CL_AllocDlight (7)->key == 7)
		result |= 8;

	/* cl_parse.c */
	memset (linkproof_cl_entities, 0, sizeof (linkproof_cl_entities));
	cl.entities = linkproof_cl_entities;
	cl.num_entities = 0;
	cl.max_edicts = 2;
	if (CL_EntityNum (1) == &linkproof_cl_entities[1] && cl.num_entities == 2)
		result |= 16;
	cl.entities = saved_entities;
	cl.num_entities = saved_num_entities;
	cl.max_edicts = saved_max_edicts;

	/* cl_tent.c -- every beam has a NULL model, so the only observable effect
	 * is the unconditional num_temp_entities reset at the top. */
	num_temp_entities = 99;
	CL_UpdateTEnts ();
	if (num_temp_entities == 0)
		result |= 32;

	/* view.c -- the c_ref cvars have no registered .value in this link (see the
	 * V_CalcRoll note above), so seed them here too. */
	cl_rollangle.value = 2.0f;
	cl_rollspeed.value = 200.0f;
	if (V_CalcRoll (linkproof_angles, linkproof_velocity) == -2.0f)
		result |= 64;

	return result;
}
