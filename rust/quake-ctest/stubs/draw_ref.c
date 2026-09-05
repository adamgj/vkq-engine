/* Shared 2D-renderer recorder for the Phase 7 client-UI oracles.
 *
 * WHY THIS FILE EXISTS
 *
 * Quake/console.c and Quake/sbar.c have no state to compare after a draw
 * call: the whole of their drawing behaviour IS the order and the arguments
 * of the Draw_ / GL_ calls they make. Both oracles therefore observe through
 * one append-only text log, and both need definitions of the same handful of
 * renderer entry points -- which two translation units cannot each provide.
 *
 * Until M10d the log and the Draw_Character / Draw_String / Draw_Fill /
 * Draw_Pic / GL_SetCanvas doubles lived inside stubs/console_ref.c. M10d
 * promoted them here, unchanged, so stubs/sbar_ref.c can share them. The two
 * console-only doubles (Draw_ConsoleBackground, GL_SetCanvasColor) stayed in
 * stubs/console_ref.c, because GL_SetCanvasColor also feeds a console-local
 * call counter; they append through ctest_draw_record below.
 *
 * THE LOG FORMAT IS FROZEN. tests/console_differential.rs compares whole logs
 * literally, so every line below must keep emitting exactly the bytes it
 * emitted before the move -- including Draw_Pic's missing trailing newline,
 * which is a wart rather than a decision, but a wart the console goldens have
 * baked in. sbar.c issues hundreds of Draw_Pic calls per frame, so
 * ctest_draw_set_pic_newline () lets a fixture opt into a terminator; it
 * defaults off, which is console.c's historic behaviour.
 *
 * PICS. gl_draw.c hands out interned qpic_t pointers and sbar.c stores ~150
 * of them in file statics, then draws by pointer. The two sides run the same
 * loader against the same doubles, so the registry below interns by name and
 * hands both sides the same pointer for the same name; Draw_Pic then logs the
 * NAME rather than an address, which is both stable across runs and readable
 * when a comparison fails. Width and height are derived from the name by a
 * fixed hash so that the pic-geometry arithmetic in Sbar_IntermissionText and
 * Sbar_DeathmatchOverlay has something non-uniform to get wrong.
 *
 * console.c's two cursor pics predate the registry: pic_ins and pic_ovr are
 * never assigned by anything in this link, so both are NULL, NULL is never
 * registrable, and the historic pic_ins / pic_ovr fallback below keeps
 * console's one Draw_Pic line byte-identical.
 */

#include "quakedef.h"

/* =========================================================================
 * THE DRAW LOG
 * ========================================================================= */

#define CTEST_DRAW_LOG_SIZE 262144
static char	  ctest_draw_logbuf[CTEST_DRAW_LOG_SIZE];
static size_t ctest_draw_logbuf_len;

void ctest_draw_record (const char *fmt, ...)
{
	va_list argptr;
	char	line[512];

	va_start (argptr, fmt);
	q_vsnprintf (line, sizeof (line), fmt, argptr);
	va_end (argptr);

	q_strlcat (ctest_draw_logbuf, line, sizeof (ctest_draw_logbuf));
	ctest_draw_logbuf_len = strlen (ctest_draw_logbuf);
}

void ctest_draw_clear_log (void)
{
	ctest_draw_logbuf[0] = '\0';
	ctest_draw_logbuf_len = 0;
}

const char *ctest_draw_log (void)
{
	return ctest_draw_logbuf;
}

/* The names tests/console_differential.rs has always bound. */
void ctest_console_clear_draw_log (void)
{
	ctest_draw_clear_log ();
}

const char *ctest_console_draw_log (void)
{
	return ctest_draw_log ();
}

/* Off = console.c's historic Draw_Pic line, which has no terminator. */
static qboolean ctest_draw_pic_newline;

void ctest_draw_set_pic_newline (qboolean on)
{
	ctest_draw_pic_newline = on;
}

/* =========================================================================
 * THE PIC REGISTRY
 * ========================================================================= */

#define CTEST_DRAW_MAX_PICS 384

typedef struct
{
	qpic_t pic;
	char   name[64];
} ctest_draw_pic_t;

static ctest_draw_pic_t ctest_draw_pics[CTEST_DRAW_MAX_PICS];
static int				ctest_draw_numpics;

static qpic_t *ctest_draw_intern (const char *name)
{
	const char	*s;
	unsigned int h;
	int			 i;

	if (!name)
		return NULL;

	for (i = 0; i < ctest_draw_numpics; i++)
		if (!strcmp (ctest_draw_pics[i].name, name))
			return &ctest_draw_pics[i].pic;

	if (ctest_draw_numpics == CTEST_DRAW_MAX_PICS)
		Sys_Error ("ctest_draw_intern: out of pic slots");

	h = 2166136261u;
	for (s = name; *s; s++)
		h = (h ^ (unsigned char)*s) * 16777619u;

	i = ctest_draw_numpics++;
	q_strlcpy (ctest_draw_pics[i].name, name, sizeof (ctest_draw_pics[i].name));
	ctest_draw_pics[i].pic.width = 8 + (int)(h % 33u);
	ctest_draw_pics[i].pic.height = 8 + (int)((h >> 8) % 17u);
	return &ctest_draw_pics[i].pic;
}

/* NULL for a pointer the registry never handed out. */
const char *ctest_draw_pic_name (const qpic_t *pic)
{
	int i;

	for (i = 0; i < ctest_draw_numpics; i++)
		if (pic == &ctest_draw_pics[i].pic)
			return ctest_draw_pics[i].name;
	return NULL;
}

qpic_t *ctest_draw_pic (const char *name)
{
	return ctest_draw_intern (name);
}

/* gl_draw.c:33 -- the loading disc, drawn by sbar.c:922 and sbar.c:1018.
 * gl_draw.c:483 sets it from the WAD; the fixtures set it directly. */
qpic_t *draw_disc;

/* gl_draw.c: the two cursor pics Con_DrawInput (console.c:2216) alternates
 * between. NULL is what Draw_TryCachePic hands back for a missing pic, and
 * console.c only ever passes them straight to Draw_Pic. */
qpic_t *pic_ovr;
qpic_t *pic_ins;

/* =========================================================================
 * LINK DOUBLES -- the 2D API both oracles reach
 * ========================================================================= */

/* draw.h:46 */
void Draw_Character (cb_context_t *cbx, float x, float y, int num)
{
	(void)cbx;
	ctest_draw_record ("char %.2f %.2f %d\n", x, y, num);
}

/* draw.h:54 */
void Draw_String (cb_context_t *cbx, float x, float y, const char *str)
{
	(void)cbx;
	ctest_draw_record ("str %.2f %.2f |%s|\n", x, y, str ? str : "(null)");
}

/* draw.h:52 */
void Draw_Fill (cb_context_t *cbx, float x, float y, float w, float h, int c, float alpha)
{
	(void)cbx;
	ctest_draw_record ("fill %.2f %.2f %.2f %.2f %d %.4f\n", x, y, w, h, c, alpha);
}

/* draw.h:47 -- the insert/overwrite cursor pic at console.c:2251, and every
 * status-bar pic. See the pic note at the top of this file for why the
 * registry lookup comes first and the pic_ins/pic_ovr test second. */
void Draw_Pic (cb_context_t *cbx, float x, float y, qpic_t *pic, float alpha, qboolean alpha_blend)
{
	const char *name = ctest_draw_pic_name (pic);

	(void)cbx;
	if (!name)
		name = (pic == pic_ins) ? "ins" : ((pic == pic_ovr) ? "ovr" : "?");
	ctest_draw_record ("pic %g %g %s %g %d", x, y, name, alpha, alpha_blend ? 1 : 0);
	if (ctest_draw_pic_newline)
		ctest_draw_record ("\n");
}

/* draw.h:64 */
void GL_SetCanvas (cb_context_t *cbx, canvastype newcanvas)
{
	(void)cbx;
	ctest_draw_record ("canvas %d\n", (int)newcanvas);
}

/* draw.h:44 -- gl_draw.c returns pic_nul for a missing lump; sbar.c only
 * calls this for lumps it has already found (Sbar_CheckPicFromWad) or for
 * lumps the base WAD always has, so the double always succeeds and hudtype
 * is steered purely through the wad tables the fixture seeds. */
qpic_t *Draw_PicFromWad (const char *name)
{
	ctest_draw_record ("picfromwad |%s|\n", name ? name : "(null)");
	return ctest_draw_intern (name);
}

/* draw.h:48 */
qpic_t *Draw_CachePic (const char *path)
{
	ctest_draw_record ("cachepic |%s|\n", path ? path : "(null)");
	return ctest_draw_intern (path);
}

/* draw.h:50 -- menu.c:638 (Get_Menu2) is the only caller in this link. The
 * engine hands back NULL when the lump is absent, and whether gfx/mainmenu2.lmp
 * is present is what decides between a four-item and a five-item main menu, so
 * the fixture has to be able to answer "missing" as well as "present".
 * (stubs/pr_ext_ref.c owned an aborting double until M10e) */
static qboolean ctest_draw_trycache_missing;

void ctest_draw_set_trycache_missing (qboolean missing)
{
	ctest_draw_trycache_missing = missing;
}

qpic_t *Draw_TryCachePic (const char *path, unsigned int texflags, int picflags)
{
	qboolean missing = ctest_draw_trycache_missing;

	ctest_draw_record ("trycachepic |%s| %u %d -> %d\n", path ? path : "(null)", texflags, picflags, missing ? 0 : 1);
	return missing ? NULL : ctest_draw_intern (path);
}

/* draw.h:51 */
void Draw_TileClear (cb_context_t *cbx, int x, int y, int w, int h)
{
	(void)cbx;
	ctest_draw_record ("tileclear %d %d %d %d\n", x, y, w, h);
}

/* draw.h:53 (stubs/pr_ext_ref.c owned an aborting double until M10d) */
void Draw_SubPic (cb_context_t *cbx, float x, float y, float w, float h, qpic_t *pic, float s1, float t1, float s2, float t2, float *rgb, float alpha)
{
	const char *name = ctest_draw_pic_name (pic);

	(void)cbx;
	ctest_draw_record ("subpic %g %g %g %g %s %g %g %g %g %s %g\n", x, y, w, h, name ? name : "?", s1, t1, s2, t2, rgb ? "rgb" : "null", alpha);
}

/* menu.h:78 -- sbar.c:1461 draws scoreboard names through the menu font */
void M_Print (cb_context_t *cbx, int cx, int cy, const char *str)
{
	(void)cbx;
	ctest_draw_record ("mprint %d %d |%s|\n", cx, cy, str ? str : "(null)");
}

/* menu.h:82 -- sbar.c:1405 */
void M_DrawPic (cb_context_t *cbx, int x, int y, qpic_t *pic)
{
	const char *name = ctest_draw_pic_name (pic);

	(void)cbx;
	ctest_draw_record ("mdrawpic %d %d %s\n", x, y, name ? name : "?");
}

/* draw.h:49 -- menu.c:294 (M_DrawTransPicTranslate), the colour-translated
 * player model on the Setup menu. Added in M10e. */
void Draw_TransPicTranslate (cb_context_t *cbx, float x, float y, qpic_t *pic, int top, int bottom)
{
	const char *name = ctest_draw_pic_name (pic);

	(void)cbx;
	ctest_draw_record ("transpictranslate %g %g %s %d %d\n", x, y, name ? name : "?", top, bottom);
}

/* draw.h:53 -- menu.c:4683, the dim behind a menu drawn over the world.
 * Added in M10e. */
void Draw_FadeScreen (cb_context_t *cbx)
{
	(void)cbx;
	ctest_draw_record ("fadescreen\n");
}
