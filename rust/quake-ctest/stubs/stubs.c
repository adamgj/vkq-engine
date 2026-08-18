/* Minimal engine-function stubs for the reference C files compiled into the
 * differential test binaries. */

#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

void Sys_Error (const char *error, ...)
{
	va_list ap;
	va_start (ap, error);
	vfprintf (stderr, error, ap);
	va_end (ap);
	abort ();
}

void Con_Printf (const char *fmt, ...)
{
	va_list ap;
	va_start (ap, fmt);
	vfprintf (stderr, fmt, ap);
	va_end (ap);
}

/* Mem_Alloc semantics per Quake/mem.h: zero-initialized */
void *Mem_Alloc (const size_t size)
{
	return calloc (1, size ? size : 1);
}

void *Mem_AllocNonZero (const size_t size)
{
	return malloc (size ? size : 1);
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

/* ---- wad.c dependencies ---- */

THREAD_LOCAL qfileofs_t com_filesize;
THREAD_LOCAL int		file_from_pak;
char					com_basedir[MAX_OSPATH];

qfileofs_t COM_ThreadFileSize (void)
{
	return com_filesize;
}

int COM_ThreadFileFromPak (void)
{
	return file_from_pak;
}

/* registered file directory: tests write real files here and both the C
 * reference and the Rust shims load them through the same stubs */
static char ctest_file_dir[1024];

void ctest_set_file_dir (const char *dir)
{
	snprintf (ctest_file_dir, sizeof (ctest_file_dir), "%s", dir);
}

static FILE *ctest_open (const char *path)
{
	char full[2048];
	snprintf (full, sizeof (full), "%s/%s", ctest_file_dir, path);
	return fopen (full, "rb");
}

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

byte *COM_LoadFile (const char *path, unsigned int *path_id)
{
	(void)path_id;
	FILE *f = ctest_open (path);
	if (!f)
		return NULL;
	Sys_fseek (f, 0, SEEK_END);
	qfileofs_t len = Sys_ftell (f);
	Sys_fseek (f, 0, SEEK_SET);
	byte *buf = (byte *)Mem_AllocNonZero (len + 1);
	buf[len] = 0;
	if (fread (buf, 1, len, f) != (size_t)len)
	{
		fclose (f);
		Mem_Free (buf);
		return NULL;
	}
	fclose (f);
	com_filesize = len;
	return buf;
}

qfilesize_t COM_FOpenFile (const char *filename, FILE **file, unsigned int *path_id)
{
	(void)path_id;
	FILE *f = ctest_open (filename);
	if (!f)
	{
		*file = NULL;
		com_filesize = -1;
		return -1;
	}
	Sys_fseek (f, 0, SEEK_END);
	qfileofs_t len = Sys_ftell (f);
	Sys_fseek (f, 0, SEEK_SET);
	*file = f;
	file_from_pak = 0;
	com_filesize = len;
	return len;
}

/* FS_fread / FS_fseek copied verbatim from Quake/common.c */
#include <errno.h>
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

	nmemb_read = bytes_read / size;
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

	if (offset > fh->length)
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

/* COM_FileGetExtension / COM_FileBase / COM_AddExtension / q_strdup copied
 * verbatim from Quake/common.c (pure helpers) */
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

void Con_Warning (const char *fmt, ...)
{
	(void)fmt;
}

void Con_DPrintf (const char *fmt, ...)
{
	(void)fmt;
}

void Con_DPrintf2 (const char *fmt, ...)
{
	(void)fmt;
}

/* LittleLong is a function pointer in the engine (byte-order dispatch); all
 * targets are little-endian */
static int ctest_LongNoSwap (int l)
{
	return l;
}
int (*LittleLong) (int l) = ctest_LongNoSwap;

/* ---- cfgfile.c dependencies ---- */

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

void FS_rewind (fshandle_t *fh)
{
	if (!fh)
		return;
	clearerr (fh->file);
	fseek (fh->file, fh->start, SEEK_SET);
	fh->pos = 0;
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

FILE *COM_FOpenPrefFile (const char *filename, const char *mode)
{
	char full[2048];
	snprintf (full, sizeof (full), "%s/%s", ctest_file_dir, filename);
	return fopen (full, mode);
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

/* va: single static buffer is enough for the tests */
char *va (const char *format, ...)
{
	static char va_buf[1024];
	va_list		argptr;
	va_start (argptr, format);
	vsnprintf (va_buf, sizeof (va_buf), format, argptr);
	va_end (argptr);
	return va_buf;
}

/* Cvar_Set capture: both the C reference and the Rust shims funnel through
 * this stub, so tests snapshot and compare the exact call sequences */
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

/* command line for COM_CheckParm */
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

int COM_CheckParm (const char *parm)
{
	int i;

	for (i = 1; i < com_argc; i++)
	{
		if (!com_argv[i])
			continue;
		if (!strcmp (parm, com_argv[i]))
			return i;
	}

	return 0;
}
