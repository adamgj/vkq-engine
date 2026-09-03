/* Phase 7 M10d oracle TU for Quake/sbar.c.
 *
 * WHY THIS FILE COMPOSES sbar.c INSTEAD OF build.rs LISTING IT IN C_SOURCES
 *
 * The prelude's c_ref_* renames are translation-unit-wide by construction:
 * one #define rewrites the definition in the oracle source AND every call in
 * every other oracle source. For sbar.c that is wrong twice over.
 *
 *   1. stubs/host_ref.c already defines Sbar_Init as an aborting link double
 *      (host_ref.c:339) because host.c:1354 calls it, and
 *      stubs/host_glue_ref.c wraps that same name in HOST_GUARD_VOID. The
 *      plain Sbar_ names in this link therefore belong to the port, exactly
 *      as Quake/sbar_glue.c owns them in the engine build, and the oracle
 *      has to answer to something else.
 *
 *   2. sbar.c's ~150 qpic_t * file statics and its hudtype autodetect state
 *      are precisely what the two sides must NOT share: each half has to load
 *      its own pic table, so that a divergence in Sbar_LoadPics shows up as a
 *      divergence in every later draw.
 *
 * So the rename layer for sbar.c lives HERE, in sbar.c's own TU, where it
 * renames sbar.c's definitions and sbar.c's internal calls and nothing else.
 *
 * ONE SYMBOL IS UN-RENAMED FOR THE WHOLE TU, oracle half included:
 *
 *   PR_MakeTempString -- pr_ext.c defines it, but stubs/pr_ext_ref.c renames
 *     it TU-locally (pr_ext_ref.c:74), so again no plain definition exists in
 *     this link. The double below records its argument into the draw log,
 *     which is how the Cmd_Argv (0) handoff at sbar.c:81 is compared.
 *
 * W_GetLumpName is NOT one of them. quake-capi's wad port exports the plain
 * name, so a double here would collide; each half keeps its own lookup (the
 * oracle's c_ref_W_GetLumpName from wad.c, the port's Rust one) and the
 * fixture seeds both wad directories from one table instead --
 * ctest_sbar_seed_hud () is what actually steers hudtype.
 *
 * WHAT IS SHARED AND WHAT IS PER SIDE
 *
 * Per side, because the prelude renamed it and the two halves own disjoint
 * copies: everything sbar.c defines, cl / cls, and the command argument
 * vector behind Cmd_Argv -- hence ctest_sbar_tokenize ().
 *
 * Shared, and therefore re-seeded by the test before each side runs: the draw
 * doubles and the draw log in stubs/draw_ref.c, realtime, host_frametime,
 * vid, glwidth, glheight, scr_con_current, scr_viewsize, scr_sbarscale,
 * scr_sbaralpha, scr_style, skill, teamplay, key_dest, the ambient qcvm /
 * pr_global_struct pair, and PR_ExecuteProgram itself -- both halves reach
 * the one real pr_exec.c oracle, the port through Host_Glue_PR_ExecuteProgram
 * (stubs/host_glue_ref.c:352), so the QC that runs is identical by
 * construction and only sbar.c's marshalling is under comparison.
 *
 * ADR-009. sbar.c's only longjmp-capable callee is PR_ExecuteProgram, at
 * sbar.c:82, :864, :870 and :1590. The five entry points that reach it --
 * Sbar_CSQCCommand, Sbar_ShowScores, Sbar_DontShowScores, Sbar_Draw and
 * Sbar_IntermissionOverlay -- are dispatched through ctest_try_host below, so
 * a raise is a comparable result instead of an escape past a Rust frame. On
 * that path C skips its PR_SwitchQCVM (NULL) and leaves the qcvm switched;
 * ctest_sbar_qcvm_active () makes that visible so the port can be held to it.
 *
 * COST, stated so it is not discovered later:
 * scripts/harness/check_ctest_symbols.sh reads C_SOURCES out of build.rs, so
 * it does not inspect this object; build.rs watches Quake/sbar.c explicitly
 * instead. A missed rename below shows up only as a duplicate-symbol link
 * error, so the block is kept in step with sbar.c by hand.
 */

#include "quakedef.h"

/* ---- sbar.c rename block -------------------------------------------------
 * Every file-scope symbol Quake/sbar.c defines. The statics do not collide,
 * but they are renamed with the rest so the block can be audited against one
 * grep of sbar.c instead of against two lists.
 */

/* data (sbar.c:28-66, :439, :441) */
#define sb_nums				 c_ref_sb_nums
#define sb_colon			 c_ref_sb_colon
#define sb_slash			 c_ref_sb_slash
#define sb_ibar				 c_ref_sb_ibar
#define sb_sbar				 c_ref_sb_sbar
#define sb_scorebar			 c_ref_sb_scorebar
#define sb_weapons			 c_ref_sb_weapons
#define sb_ammo				 c_ref_sb_ammo
#define sb_sigil			 c_ref_sb_sigil
#define sb_armor			 c_ref_sb_armor
#define sb_items			 c_ref_sb_items
#define sb_faces			 c_ref_sb_faces
#define sb_face_invis		 c_ref_sb_face_invis
#define sb_face_quad		 c_ref_sb_face_quad
#define sb_face_invuln		 c_ref_sb_face_invuln
#define sb_face_invis_invuln c_ref_sb_face_invis_invuln
#define sb_showscores		 c_ref_sb_showscores
#define sb_lines			 c_ref_sb_lines
#define rsb_invbar			 c_ref_rsb_invbar
#define rsb_weapons			 c_ref_rsb_weapons
#define rsb_items			 c_ref_rsb_items
#define rsb_ammo			 c_ref_rsb_ammo
#define rsb_teambord		 c_ref_rsb_teambord
#define hsb_weapons			 c_ref_hsb_weapons
#define hipweapons			 c_ref_hipweapons
#define hsb_items			 c_ref_hsb_items
#define hudtype				 c_ref_hudtype
#define fragsort			 c_ref_fragsort
#define scoreboardlines		 c_ref_scoreboardlines

/* functions with external linkage (sbar.c:75-1632) */
#define Sbar_CSQCCommand			c_ref_Sbar_CSQCCommand
#define Sbar_ShowScores				c_ref_Sbar_ShowScores
#define Sbar_LoadPics				c_ref_Sbar_LoadPics
#define Sbar_Init					c_ref_Sbar_Init
#define Sbar_DrawPic				c_ref_Sbar_DrawPic
#define Sbar_DrawPicAlpha			c_ref_Sbar_DrawPicAlpha
#define Sbar_DrawCharacter			c_ref_Sbar_DrawCharacter
#define Sbar_DrawString				c_ref_Sbar_DrawString
#define Sbar_DrawScrollString		c_ref_Sbar_DrawScrollString
#define Sbar_itoa					c_ref_Sbar_itoa
#define Sbar_DrawNum				c_ref_Sbar_DrawNum
#define Sbar_DrawSmallAmmoCounter	c_ref_Sbar_DrawSmallAmmoCounter
#define Sbar_SortFrags				c_ref_Sbar_SortFrags
#define Sbar_ColorForMap			c_ref_Sbar_ColorForMap
#define Sbar_SoloScoreboard			c_ref_Sbar_SoloScoreboard
#define Sbar_DrawScoreboard			c_ref_Sbar_DrawScoreboard
#define Sbar_InventoryBarPic		c_ref_Sbar_InventoryBarPic
#define Sbar_CalculateFlashOn		c_ref_Sbar_CalculateFlashOn
#define Sbar_DrawInventory			c_ref_Sbar_DrawInventory
#define Sbar_DrawFrags				c_ref_Sbar_DrawFrags
#define Sbar_DrawFace				c_ref_Sbar_DrawFace
#define Sbar_Draw					c_ref_Sbar_Draw
#define Sbar_IntermissionNumber		c_ref_Sbar_IntermissionNumber
#define Sbar_IntermissionPicForChar c_ref_Sbar_IntermissionPicForChar
#define Sbar_IntermissionTextWidth	c_ref_Sbar_IntermissionTextWidth
#define Sbar_IntermissionText		c_ref_Sbar_IntermissionText
#define Sbar_DeathmatchOverlay		c_ref_Sbar_DeathmatchOverlay
#define Sbar_MiniDeathmatchOverlay	c_ref_Sbar_MiniDeathmatchOverlay
#define Sbar_IntermissionOverlay	c_ref_Sbar_IntermissionOverlay
#define Sbar_FinaleOverlay			c_ref_Sbar_FinaleOverlay

/* statics (sbar.c:111-997). Sbar_DontShowScores is the one the fixture calls;
 * the other four are renamed only for the audit rule stated above. */
#define Sbar_DontShowScores	 c_ref_Sbar_DontShowScores
#define Sbar_CheckPicFromWad c_ref_Sbar_CheckPicFromWad
#define Sbar_DrawCSCQ		 c_ref_Sbar_DrawCSCQ
#define Sbar_DrawClassic	 c_ref_Sbar_DrawClassic
#define Sbar_DrawModern		 c_ref_Sbar_DrawModern

/* sbar.h is not force-included by the prelude, so sbar.c's definitions would
 * have no visible prototype. Re-declaring them here costs nothing -- the
 * macros above rewrite each line -- and keeps the oracle build warning-clean.
 */
qboolean Sbar_CSQCCommand (void);
void	 Sbar_ShowScores (void);
void	 Sbar_LoadPics (void);
void	 Sbar_Init (void);
void	 Sbar_DrawPic (cb_context_t *cbx, int x, int y, qpic_t *pic);
void	 Sbar_DrawPicAlpha (cb_context_t *cbx, int x, int y, qpic_t *pic, float alpha);
void	 Sbar_DrawCharacter (cb_context_t *cbx, int x, int y, int num);
void	 Sbar_DrawString (cb_context_t *cbx, int x, int y, const char *str);
void	 Sbar_DrawScrollString (cb_context_t *cbx, int x, int y, int width, const char *str);
int		 Sbar_itoa (int num, char *buf);
void	 Sbar_DrawNum (cb_context_t *cbx, int x, int y, int num, int digits, int color);
void	 Sbar_DrawSmallAmmoCounter (cb_context_t *cbx, int x, int y, int val);
void	 Sbar_SortFrags (void);
int		 Sbar_ColorForMap (int m);
void	 Sbar_SoloScoreboard (cb_context_t *cbx);
void	 Sbar_DrawScoreboard (cb_context_t *cbx);
qpic_t	*Sbar_InventoryBarPic (void);
int		 Sbar_CalculateFlashOn (int val);
void	 Sbar_DrawInventory (cb_context_t *cbx);
void	 Sbar_DrawFrags (cb_context_t *cbx);
void	 Sbar_DrawFace (cb_context_t *cbx, int x, int y, qboolean classic_style);
void	 Sbar_Draw (cb_context_t *cbx);
void	 Sbar_IntermissionNumber (cb_context_t *cbx, int x, int y, int num, int digits, int color);
qpic_t	*Sbar_IntermissionPicForChar (char c, int color);
int		 Sbar_IntermissionTextWidth (const char *str, int color);
void	 Sbar_IntermissionText (cb_context_t *cbx, int x, int y, const char *str, int color);
void	 Sbar_DeathmatchOverlay (cb_context_t *cbx);
void	 Sbar_MiniDeathmatchOverlay (cb_context_t *cbx);
void	 Sbar_IntermissionOverlay (cb_context_t *cbx);
void	 Sbar_FinaleOverlay (cb_context_t *cbx);

/* c_ref_prelude.h:403-404 renames common.c's `hipnotic` / `rogue` globals,
 * but sbar.c:66-67 defines object-like macros of the same two names. Dropping
 * the prelude's pair first keeps that from being a macro redefinition; sbar.c
 * never reads the globals. */
#undef hipnotic
#undef rogue

/* The double this file supplies for both halves (see the header). */
int PR_MakeTempString (const char *val);

#include "sbar.c"

/* =========================================================================
 * THE PLAIN HALF -- the ctest-link mirror of Quake/sbar_glue.c
 * ========================================================================= */

#undef sb_nums
#undef sb_colon
#undef sb_slash
#undef sb_ibar
#undef sb_sbar
#undef sb_scorebar
#undef sb_weapons
#undef sb_ammo
#undef sb_sigil
#undef sb_armor
#undef sb_items
#undef sb_faces
#undef sb_face_invis
#undef sb_face_quad
#undef sb_face_invuln
#undef sb_face_invis_invuln
#undef sb_showscores
#undef sb_lines
#undef rsb_invbar
#undef rsb_weapons
#undef rsb_items
#undef rsb_ammo
#undef rsb_teambord
#undef hsb_weapons
#undef hipweapons
#undef hsb_items
#undef hudtype
#undef fragsort
#undef scoreboardlines
#undef Sbar_CSQCCommand
#undef Sbar_ShowScores
#undef Sbar_LoadPics
#undef Sbar_Init
#undef Sbar_DrawPic
#undef Sbar_DrawPicAlpha
#undef Sbar_DrawCharacter
#undef Sbar_DrawString
#undef Sbar_DrawScrollString
#undef Sbar_itoa
#undef Sbar_DrawNum
#undef Sbar_DrawSmallAmmoCounter
#undef Sbar_SortFrags
#undef Sbar_ColorForMap
#undef Sbar_SoloScoreboard
#undef Sbar_DrawScoreboard
#undef Sbar_InventoryBarPic
#undef Sbar_CalculateFlashOn
#undef Sbar_DrawInventory
#undef Sbar_DrawFrags
#undef Sbar_DrawFace
#undef Sbar_Draw
#undef Sbar_IntermissionNumber
#undef Sbar_IntermissionPicForChar
#undef Sbar_IntermissionTextWidth
#undef Sbar_IntermissionText
#undef Sbar_DeathmatchOverlay
#undef Sbar_MiniDeathmatchOverlay
#undef Sbar_IntermissionOverlay
#undef Sbar_FinaleOverlay
#undef Sbar_DontShowScores
#undef Sbar_CheckPicFromWad
#undef Sbar_DrawCSCQ
#undef Sbar_DrawClassic
#undef Sbar_DrawModern
#undef cl
#undef cls

/* sbar.c:67-68 leaves these two behind, and the fixture spells hudtype out. */
#undef hipnotic
#undef rogue
#undef STAT_MINUS

extern client_state_t cl; /* quake-capi's cl_main port owns it (ADR-007) */

/* ---------------------------------------------------------------------------
 * C-visible objects (sbar.c:47, :49, :439, :441), initializers verbatim from
 * Quake/sbar_glue.c.
 */

qboolean sb_showscores;

int sb_lines; // scan lines to draw

int fragsort[MAX_SCOREBOARD];
int scoreboardlines;

/* gl_screen.c:86 and :94. The renderer is not in this link and nothing else
 * defines either cvar, so the two the port reads are defined here with
 * gl_screen.c's own defaults; both sides read this one pair. */
cvar_t scr_sbaralpha = {"scr_sbaralpha", "0.75", CVAR_ARCHIVE, 0.75f};
cvar_t scr_style = {"scr_style", "0", CVAR_ARCHIVE, 0.0f};

/* ---------------------------------------------------------------------------
 * The port's status cores and the harness's raise machinery.
 */
extern int	Host_Guard (void (*fn) (void *), void *arg);
extern void Host_Reraise (int guard_result);
extern int	ctest_try_host (void (*fn) (void *), void *arg);

extern int	 quake_rs_sbar_csqc_command (bool *out);
extern int	 quake_rs_sbar_show_scores (void);
extern int	 quake_rs_sbar_dont_show_scores (void);
extern int	 quake_rs_sbar_draw (void *cbx);
extern int	 quake_rs_sbar_intermission_overlay (void *cbx);
extern void	 quake_rs_sbar_load_pics (void);
extern void	 quake_rs_sbar_init (void);
extern void	 quake_rs_sbar_draw_pic (void *cbx, int x, int y, void *pic);
extern void	 quake_rs_sbar_draw_pic_alpha (void *cbx, int x, int y, void *pic, float alpha);
extern void	 quake_rs_sbar_draw_character (void *cbx, int x, int y, int num);
extern void	 quake_rs_sbar_draw_string (void *cbx, int x, int y, const char *str);
extern void	 quake_rs_sbar_draw_scroll_string (void *cbx, int x, int y, int width, const char *str);
extern int	 quake_rs_sbar_itoa (int num, char *buf);
extern void	 quake_rs_sbar_draw_num (void *cbx, int x, int y, int num, int digits, int color);
extern void	 quake_rs_sbar_draw_small_ammo_counter (void *cbx, int x, int y, int val);
extern void	 quake_rs_sbar_sort_frags (void);
extern int	 quake_rs_sbar_color_for_map (int m);
extern void	 quake_rs_sbar_solo_scoreboard (void *cbx);
extern void	 quake_rs_sbar_draw_scoreboard (void *cbx);
extern void *quake_rs_sbar_inventory_bar_pic (void);
extern int	 quake_rs_sbar_calculate_flash_on (int val);
extern void	 quake_rs_sbar_draw_inventory (void *cbx);
extern void	 quake_rs_sbar_draw_frags (void *cbx);
extern void	 quake_rs_sbar_draw_face (void *cbx, int x, int y, qboolean classic_style);
extern void	 quake_rs_sbar_intermission_number (void *cbx, int x, int y, int num, int digits, int color);
extern void *quake_rs_sbar_intermission_pic_for_char (char c, int color);
extern int	 quake_rs_sbar_intermission_text_width (const char *str, int color);
extern void	 quake_rs_sbar_intermission_text (void *cbx, int x, int y, const char *str, int color);
extern void	 quake_rs_sbar_deathmatch_overlay (void *cbx);
extern void	 quake_rs_sbar_mini_deathmatch_overlay (void *cbx);
extern void	 quake_rs_sbar_finale_overlay (void *cbx);

/* Quake/sbar_glue.c:94 and :101. These two are the only plain Sbar_ names the
 * port itself links against (quake-c-sys/src/sbar.rs declares exactly them):
 * Sbar_Init registers them as +showscores / -showscores, and both reach QC
 * through Sbar_CSQCCommand, so both re-raise. Sbar_DontShowScores is static
 * in sbar.c, so the glue exports it under a Sbar_Glue_ name. */
void Sbar_ShowScores (void)
{
	int r = quake_rs_sbar_show_scores ();
	Host_Reraise (r);
}

void Sbar_Glue_DontShowScores (void)
{
	int r = quake_rs_sbar_dont_show_scores ();
	Host_Reraise (r);
}

/* =========================================================================
 * SHARED DOUBLES THIS FILE OWNS
 * ========================================================================= */

extern void		   ctest_draw_record (const char *fmt, ...);
extern qpic_t	  *ctest_draw_pic (const char *name);
extern const char *ctest_draw_pic_name (const qpic_t *pic);

/* THE WAD DIRECTORY BOTH SIDES LOOK UP IN
 *
 * Sbar_CheckPicFromWad (sbar.c:117) asks W_GetLumpName whether a lump exists,
 * and that answer is the whole of hudtype autodetection. The two halves reach
 * two different implementations -- wad.c's c_ref_W_GetLumpName for the oracle,
 * quake-capi's Rust export for the port -- so rather than fake either one, the
 * seeder below builds one lumpinfo_t table and publishes it through both
 * wad_lumps/wad_numlumps pairs. Both lookups then run for real against
 * identical data.
 *
 * The names are stored the way W_CleanupName (wad.c:43) would leave them:
 * lowercased, truncated to 16 bytes, zero-filled. Every name below is already
 * lowercase and at most 15 characters, so a zeroed field plus a memcpy is
 * exactly that. wad_base has to be non-NULL because both implementations
 * return `wad_base + filepos` and a NULL result reads as "missing". */
#define CTEST_SBAR_MAX_LUMPS 128

#undef wad_numlumps
#undef wad_lumps
#undef wad_base

extern int		   wad_numlumps;
extern lumpinfo_t *wad_lumps;
extern byte		  *wad_base;
extern int		   c_ref_wad_numlumps;
extern lumpinfo_t *c_ref_wad_lumps;
extern byte		  *c_ref_wad_base;

static lumpinfo_t ctest_sbar_lumpdir[CTEST_SBAR_MAX_LUMPS];
static int		  ctest_sbar_numlumps;
static byte		  ctest_sbar_wadbase[64];

static void ctest_sbar_publish_lumps (void)
{
	wad_lumps = c_ref_wad_lumps = ctest_sbar_lumpdir;
	wad_numlumps = c_ref_wad_numlumps = ctest_sbar_numlumps;
	wad_base = c_ref_wad_base = ctest_sbar_wadbase;
}

void ctest_sbar_clear_lumps (void)
{
	memset (ctest_sbar_lumpdir, 0, sizeof (ctest_sbar_lumpdir));
	ctest_sbar_numlumps = 0;
	ctest_sbar_publish_lumps ();
}

void ctest_sbar_add_lump (const char *name)
{
	size_t len = strlen (name);

	if (ctest_sbar_numlumps == CTEST_SBAR_MAX_LUMPS)
		Sys_Error ("ctest_sbar_add_lump: out of slots");
	if (len > sizeof (ctest_sbar_lumpdir[0].name))
		Sys_Error ("ctest_sbar_add_lump: name too long");
	memcpy (ctest_sbar_lumpdir[ctest_sbar_numlumps].name, name, len);
	ctest_sbar_lumpdir[ctest_sbar_numlumps].size = 32;
	ctest_sbar_lumpdir[ctest_sbar_numlumps].disksize = 32;
	ctest_sbar_numlumps++;
	ctest_sbar_publish_lumps ();
}

/* The three lump sets Sbar_LoadPics probes for, so a test can ask for a
 * hudtype instead of spelling out 37 or 13 names. mode 3 is the partial
 * hipnotic set: it drops the last probe, which is what makes hudtype fall
 * back to 0 halfway through the block (sbar.c:224-253). mode 4 drops the
 * first probe instead, so hudtype is reset before the rest of the block runs
 * and every later probe reaches the hudtype == 0 early-out with its lump
 * still present -- the only arrangement in which that early-out is
 * observable at all (sbar.c:118). */
void ctest_sbar_seed_hud (int mode)
{
	static const char *const hip[] = {
		"inv_laser",	   "inv_mjolnir",	  "inv_gren_prox",	 "inv_prox_gren",	"inv_prox",		   "inv2_laser",	  "inv2_mjolnir",	 "inv2_gren_prox",
		"inv2_prox_gren",  "inv2_prox",		  "inva1_laser",	 "inva1_mjolnir",	"inva1_gren_prox", "inva1_prox_gren", "inva1_prox",		 "inva2_laser",
		"inva2_mjolnir",   "inva2_gren_prox", "inva2_prox_gren", "inva2_prox",		"inva3_laser",	   "inva3_mjolnir",	  "inva3_gren_prox", "inva3_prox_gren",
		"inva3_prox",	   "inva4_laser",	  "inva4_mjolnir",	 "inva4_gren_prox", "inva4_prox_gren", "inva4_prox",	  "inva5_laser",	 "inva5_mjolnir",
		"inva5_gren_prox", "inva5_prox_gren", "inva5_prox",		 "sb_wsuit",		"sb_eshld"};
	static const char *const rog[] = {"r_invbar1", "r_invbar2", "r_lava",	  "r_superlava", "r_gren",		"r_multirock", "r_plasma",
									  "r_shield1", "r_agrav1",	"r_teambord", "r_ammolava",	 "r_ammomulti", "r_ammoplasma"};
	size_t					 i;

	ctest_sbar_clear_lumps ();
	if (mode == 1 || mode == 3 || mode == 4)
		for (i = (size_t)(mode == 4 ? 1 : 0); i < sizeof (hip) / sizeof (hip[0]) - (size_t)(mode == 3 ? 1 : 0); i++)
			ctest_sbar_add_lump (hip[i]);
	else if (mode == 2)
		for (i = 0; i < sizeof (rog) / sizeof (rog[0]); i++)
			ctest_sbar_add_lump (rog[i]);
}

/* progs.h:207. pr_ext_ref.c:74 renames the real definition, so the plain name
 * this link needs is defined here. A constant is returned rather than a
 * counter: sbar.c:81 stores it into G_INT (OFS_PARM0), so a per-call value
 * would differ between the side that ran first and the side that ran second
 * and turn a shared counter into a false divergence. The argument is what
 * matters, so it goes into the draw log. */
#define CTEST_SBAR_TEMPSTRING (-24301)

int PR_MakeTempString (const char *val)
{
	ctest_draw_record ("tempstring |%s|\n", val ? val : "(null)");
	return CTEST_SBAR_TEMPSTRING;
}

/* =========================================================================
 * THE FIXTURE
 *
 * `side` is 1 for the C oracle (c_ref_*) and 0 for the Rust port (plain), the
 * convention stubs/console_ref.c and stubs/keys_ref.c use. Anything sbar.c
 * owns is per-side; the screen/time/cvar state below is shared and has to be
 * re-seeded before each side runs.
 * ========================================================================= */

extern qboolean c_ref_sb_showscores;
extern int		c_ref_sb_lines;
extern int		c_ref_fragsort[MAX_SCOREBOARD];
extern int		c_ref_scoreboardlines;

extern qboolean c_ref_Sbar_CSQCCommand (void);
extern void		c_ref_Sbar_ShowScores (void);
extern void		c_ref_Sbar_DontShowScores (void);
extern void		c_ref_Sbar_LoadPics (void);
extern void		c_ref_Sbar_Init (void);
extern void		c_ref_Sbar_DrawPic (cb_context_t *cbx, int x, int y, qpic_t *pic);
extern void		c_ref_Sbar_DrawPicAlpha (cb_context_t *cbx, int x, int y, qpic_t *pic, float alpha);
extern void		c_ref_Sbar_DrawCharacter (cb_context_t *cbx, int x, int y, int num);
extern void		c_ref_Sbar_DrawString (cb_context_t *cbx, int x, int y, const char *str);
extern void		c_ref_Sbar_DrawScrollString (cb_context_t *cbx, int x, int y, int width, const char *str);
extern int		c_ref_Sbar_itoa (int num, char *buf);
extern void		c_ref_Sbar_DrawNum (cb_context_t *cbx, int x, int y, int num, int digits, int color);
extern void		c_ref_Sbar_DrawSmallAmmoCounter (cb_context_t *cbx, int x, int y, int val);
extern void		c_ref_Sbar_SortFrags (void);
extern int		c_ref_Sbar_ColorForMap (int m);
extern void		c_ref_Sbar_SoloScoreboard (cb_context_t *cbx);
extern void		c_ref_Sbar_DrawScoreboard (cb_context_t *cbx);
extern qpic_t  *c_ref_Sbar_InventoryBarPic (void);
extern int		c_ref_Sbar_CalculateFlashOn (int val);
extern void		c_ref_Sbar_DrawInventory (cb_context_t *cbx);
extern void		c_ref_Sbar_DrawFrags (cb_context_t *cbx);
extern void		c_ref_Sbar_DrawFace (cb_context_t *cbx, int x, int y, qboolean classic_style);
extern void		c_ref_Sbar_Draw (cb_context_t *cbx);
extern void		c_ref_Sbar_IntermissionNumber (cb_context_t *cbx, int x, int y, int num, int digits, int color);
extern qpic_t  *c_ref_Sbar_IntermissionPicForChar (char c, int color);
extern int		c_ref_Sbar_IntermissionTextWidth (const char *str, int color);
extern void		c_ref_Sbar_IntermissionText (cb_context_t *cbx, int x, int y, const char *str, int color);
extern void		c_ref_Sbar_DeathmatchOverlay (cb_context_t *cbx);
extern void		c_ref_Sbar_MiniDeathmatchOverlay (cb_context_t *cbx);
extern void		c_ref_Sbar_IntermissionOverlay (cb_context_t *cbx);
extern void		c_ref_Sbar_FinaleOverlay (cb_context_t *cbx);

extern client_state_t c_ref_cl;

static client_state_t *ctest_sbar_cl (int side)
{
	return side ? &c_ref_cl : &cl;
}

/* cl.scores is a pointer into a [cl.maxclients] array the client normally
 * allocates on connect (client.h:257); each half gets its own. */
static scoreboard_t ctest_sbar_scores[2][MAX_SCOREBOARD];

/* ---- shared screen / time / cvar state ---------------------------------- */

void ctest_sbar_set_screen (int vid_w, int vid_h, int glw, int glh, float con_current, float viewsize)
{
	vid.width = vid_w;
	vid.height = vid_h;
	vid.conwidth = vid_w;
	vid.conheight = vid_h;
	glwidth = glw;
	glheight = glh;
	scr_con_current = con_current;
	scr_viewsize.value = viewsize;
}

void ctest_sbar_set_cvars (float style, float sbarscale, float sbaralpha, float skill_value, float teamplay_value)
{
	scr_style.value = style;
	scr_sbarscale.value = sbarscale;
	scr_sbaralpha.value = sbaralpha;
	skill.value = skill_value;
	teamplay.value = teamplay_value;
}

void ctest_sbar_set_time (double now, double frametime)
{
	realtime = now;
	host_frametime = frametime;
}

void ctest_sbar_set_key_dest (int dest)
{
	key_dest = (keydest_t)dest;
}

void ctest_sbar_set_draw_disc (const char *name)
{
	draw_disc = name ? ctest_draw_pic (name) : NULL;
}

/* Cmd_Argv is per-side for the same reason console.c's is: cmd.c is an oracle
 * source, so the oracle reads c_ref_ tokenizer state while the port reads
 * quake-capi's (stubs/console_ref.c:1675 records the same finding). */
#undef Cmd_TokenizeString
void Cmd_TokenizeString (const char *text);

void ctest_sbar_tokenize (int side, const char *text)
{
	if (side)
		c_ref_Cmd_TokenizeString (text);
	else
		Cmd_TokenizeString (text);
}

/* ---- per-side client state ---------------------------------------------- */

void ctest_sbar_set_client (
	int side, double time, double oldtime, int intermission, int completed_time, float faceanimtime, int items, int viewentity, int maxclients, int gametype,
	const char *mapname, const char *levelname)
{
	client_state_t *c = ctest_sbar_cl (side);

	c->time = time;
	c->oldtime = oldtime;
	c->intermission = intermission;
	c->completed_time = completed_time;
	c->faceanimtime = faceanimtime;
	c->items = items;
	c->viewentity = viewentity;
	c->maxclients = maxclients;
	c->gametype = gametype;
	q_strlcpy (c->mapname, mapname ? mapname : "", sizeof (c->mapname));
	q_strlcpy (c->levelname, levelname ? levelname : "", sizeof (c->levelname));
	c->scores = ctest_sbar_scores[side ? 1 : 0];
}

void ctest_sbar_set_stat (int side, int stat, int value)
{
	if (stat >= 0 && stat < MAX_CL_STATS)
		ctest_sbar_cl (side)->stats[stat] = value;
}

int ctest_sbar_get_stat (int side, int stat)
{
	if (stat < 0 || stat >= MAX_CL_STATS)
		return 0;
	return ctest_sbar_cl (side)->stats[stat];
}

void ctest_sbar_set_item_gettime (int side, int index, float t)
{
	if (index >= 0 && index < 32)
		ctest_sbar_cl (side)->item_gettime[index] = t;
}

/* Sbar_CalculateFlashOn writes cl.item_gettime back when the stamp is in the
 * future (sbar.c:561), and that write is its only observable effect there. */
float ctest_sbar_get_item_gettime (int side, int index)
{
	if (index < 0 || index >= 32)
		return 0.0f;
	return ctest_sbar_cl (side)->item_gettime[index];
}

void ctest_sbar_clear_scores (int side)
{
	memset (ctest_sbar_scores[side ? 1 : 0], 0, sizeof (ctest_sbar_scores[0]));
}

void ctest_sbar_set_score (int side, int index, const char *name, int frags, int colors, float entertime, int ping)
{
	scoreboard_t *s;

	if (index < 0 || index >= MAX_SCOREBOARD)
		return;
	s = &ctest_sbar_scores[side ? 1 : 0][index];
	q_strlcpy (s->name, name ? name : "", sizeof (s->name));
	s->frags = frags;
	s->colors = colors;
	s->entertime = entertime;
	s->ping = ping;
}

/* ---- per-side sbar objects (glue-owned in the engine build) -------------- */

void ctest_sbar_set_sb_showscores (int side, qboolean v)
{
	if (side)
		c_ref_sb_showscores = v;
	else
		sb_showscores = v;
}

qboolean ctest_sbar_get_sb_showscores (int side)
{
	return side ? c_ref_sb_showscores : sb_showscores;
}

void ctest_sbar_set_sb_lines (int side, int v)
{
	if (side)
		c_ref_sb_lines = v;
	else
		sb_lines = v;
}

int ctest_sbar_get_sb_lines (int side)
{
	return side ? c_ref_sb_lines : sb_lines;
}

int ctest_sbar_get_scoreboardlines (int side)
{
	return side ? c_ref_scoreboardlines : scoreboardlines;
}

int ctest_sbar_get_fragsort (int side, int index)
{
	if (index < 0 || index >= MAX_SCOREBOARD)
		return -1;
	return side ? c_ref_fragsort[index] : fragsort[index];
}

/* =========================================================================
 * THE CSQC FIXTURE
 *
 * Both halves run the one real pr_exec.c oracle -- the port through
 * Host_Glue_PR_ExecuteProgram (stubs/host_glue_ref.c:352) -- so each side only
 * needs its own identically-shaped qcvm_t hanging off cl.qcvm (client.h:282).
 *
 * PR_ExecuteProgram cannot dispatch a builtin at top level (pr_exec.c:325
 * says so and then crashes), so the entry points are real functions whose one
 * statement is an OP_CALL0 at a global holding the builtin's function index.
 * ctest_progs_synth_vm publishes builtin 1 as a marker that writes
 * argc * 100 + xstatement to OFS_RETURN and echoes OFS_PARM0 to OFS_RETURN+1,
 * and builtin 2 as one that calls Host_Error (stubs.c:3325-3337) -- which is
 * how a raising QC function is spelled here.
 * ========================================================================= */

#define CTEST_SBAR_NUMGLOBALS	256
#define CTEST_SBAR_G_MARKER		100 /* holds function index 3, the marker */
#define CTEST_SBAR_G_RAISER		101 /* holds function index 4, the raiser */
#define CTEST_SBAR_G_CLTIME		110
#define CTEST_SBAR_G_CLFRAME	111
#define CTEST_SBAR_G_INTERM		112
#define CTEST_SBAR_G_INTERMTIME 113
#define CTEST_SBAR_G_LOCALENT	114

extern void *ctest_progs_synth_vm (
	int which, int max_edicts, int entityfields, int numglobals, const void *stmts, int nstmts, const void *funcs, int nfuncs, const char *strings,
	int stringssize);
extern int ctest_progs_builtin_calls;

/* mode 0 = not installed, 1 = the marker entry, 2 = the raising entry. */
static func_t ctest_sbar_entry (int mode)
{
	return mode == 1 ? (func_t)1 : (mode == 2 ? (func_t)2 : (func_t)0);
}

void ctest_sbar_install_csqc (int side, int consolecommand, int drawhud, int drawscores)
{
	dstatement_t	  st[5];
	dfunction_t		  fn[5];
	static const char strings[2] = {0, 0};
	client_state_t	 *c = ctest_sbar_cl (side);
	qcvm_t			 *vm;

	memset (st, 0, sizeof (st));
	memset (fn, 0, sizeof (fn));

	st[0].op = OP_DONE;
	st[1].op = OP_CALL0;
	st[1].a = CTEST_SBAR_G_MARKER;
	st[2].op = OP_DONE;
	st[3].op = OP_CALL0;
	st[3].a = CTEST_SBAR_G_RAISER;
	st[4].op = OP_DONE;

	fn[1].first_statement = 1; /* entry: calls the marker builtin */
	fn[2].first_statement = 3; /* entry: calls the raising builtin */
	fn[3].first_statement = -1;
	fn[4].first_statement = -2;

	vm = (qcvm_t *)ctest_progs_synth_vm (side ? 1 : 0, 8, 32, CTEST_SBAR_NUMGLOBALS, st, 5, fn, 5, strings, 2);
	((int *)vm->globals)[CTEST_SBAR_G_MARKER] = 3;
	((int *)vm->globals)[CTEST_SBAR_G_RAISER] = 4;

	memcpy (&c->qcvm, vm, sizeof (qcvm_t));
	c->qcvm.extfuncs.CSQC_ConsoleCommand = ctest_sbar_entry (consolecommand);
	c->qcvm.extfuncs.CSQC_DrawHud = ctest_sbar_entry (drawhud);
	c->qcvm.extfuncs.CSQC_DrawScores = ctest_sbar_entry (drawscores);
	c->qcvm.extglobals.cltime = &c->qcvm.globals[CTEST_SBAR_G_CLTIME];
	c->qcvm.extglobals.clframetime = &c->qcvm.globals[CTEST_SBAR_G_CLFRAME];
	c->qcvm.extglobals.intermission = &c->qcvm.globals[CTEST_SBAR_G_INTERM];
	c->qcvm.extglobals.intermission_time = &c->qcvm.globals[CTEST_SBAR_G_INTERMTIME];
	c->qcvm.extglobals.player_localentnum = &c->qcvm.globals[CTEST_SBAR_G_LOCALENT];
}

void ctest_sbar_clear_csqc (int side)
{
	memset (&ctest_sbar_cl (side)->qcvm, 0, sizeof (qcvm_t));
}

void ctest_sbar_set_qc_global_int (int side, int ofs, int v)
{
	client_state_t *c = ctest_sbar_cl (side);
	if (c->qcvm.globals && ofs >= 0 && ofs < CTEST_SBAR_NUMGLOBALS)
		((int *)c->qcvm.globals)[ofs] = v;
}

void ctest_sbar_set_qc_global_float (int side, int ofs, float v)
{
	client_state_t *c = ctest_sbar_cl (side);
	if (c->qcvm.globals && ofs >= 0 && ofs < CTEST_SBAR_NUMGLOBALS)
		c->qcvm.globals[ofs] = v;
}

/* Read back as raw bits: OFS_PARM0 often holds an int (a string index) that
 * would be a NaN if it were compared as a float, and NaN != NaN. */
int ctest_sbar_get_qc_global_int (int side, int ofs)
{
	client_state_t *c = ctest_sbar_cl (side);
	if (!c->qcvm.globals || ofs < 0 || ofs >= CTEST_SBAR_NUMGLOBALS)
		return 0;
	return ((int *)c->qcvm.globals)[ofs];
}

int ctest_sbar_builtin_calls (void)
{
	return ctest_progs_builtin_calls;
}

void ctest_sbar_reset_builtin_calls (void)
{
	ctest_progs_builtin_calls = 0;
}

/* COMPAT: sbar.c skips its PR_SwitchQCVM (NULL) when PR_ExecuteProgram raises
 * (sbar.c:84, :874, :1591), so the ambient qcvm is left switched. That is the
 * bug-for-bug behaviour the port has to reproduce, so it is observed rather
 * than cleaned up; stubs.c:3177 Sys_Errors on a double switch, so a test that
 * saw a raise clears it explicitly before the next scenario. */
qboolean ctest_sbar_qcvm_active (void)
{
	return qcvm != NULL;
}

void ctest_sbar_clear_qcvm (void)
{
	PR_SwitchQCVM (NULL);
}

/* =========================================================================
 * ENTRY POINTS, DISPATCHED PER SIDE
 *
 * The five that can reach QC run under ctest_try_host so a raise is a
 * comparable result instead of an escape past a Rust frame (ADR-009); they
 * return 1 when Host_Error fired. The Rust arm of each spells out the same
 * Host_Reraise (quake_rs_*()) wrapper Quake/sbar_glue.c compiles into the
 * engine build.
 * ========================================================================= */

typedef struct
{
	int		 side;
	qboolean ret;
} ctest_sbar_call_t;

static void ctest_sbar_thunk_csqccommand (void *p)
{
	ctest_sbar_call_t *c = (ctest_sbar_call_t *)p;
	if (c->side)
		c->ret = c_ref_Sbar_CSQCCommand ();
	else
	{
		bool r = false;
		int	 g = quake_rs_sbar_csqc_command (&r);
		Host_Reraise (g);
		c->ret = r;
	}
}

int ctest_sbar_csqc_command (int side, int *ret)
{
	ctest_sbar_call_t c;
	int				  raised;

	c.side = side;
	c.ret = false;
	raised = ctest_try_host (ctest_sbar_thunk_csqccommand, &c);
	*ret = c.ret;
	return raised;
}

static void ctest_sbar_thunk_showscores (void *p)
{
	ctest_sbar_call_t *c = (ctest_sbar_call_t *)p;
	if (c->side)
		c_ref_Sbar_ShowScores ();
	else
		Sbar_ShowScores ();
}

int ctest_sbar_show_scores (int side)
{
	ctest_sbar_call_t c;
	c.side = side;
	c.ret = false;
	return ctest_try_host (ctest_sbar_thunk_showscores, &c);
}

static void ctest_sbar_thunk_dontshowscores (void *p)
{
	ctest_sbar_call_t *c = (ctest_sbar_call_t *)p;
	if (c->side)
		c_ref_Sbar_DontShowScores ();
	else
		Sbar_Glue_DontShowScores ();
}

int ctest_sbar_dont_show_scores (int side)
{
	ctest_sbar_call_t c;
	c.side = side;
	c.ret = false;
	return ctest_try_host (ctest_sbar_thunk_dontshowscores, &c);
}

static void ctest_sbar_thunk_draw (void *p)
{
	ctest_sbar_call_t *c = (ctest_sbar_call_t *)p;
	if (c->side)
		c_ref_Sbar_Draw (NULL);
	else
		Host_Reraise (quake_rs_sbar_draw (NULL));
}

int ctest_sbar_draw (int side)
{
	ctest_sbar_call_t c;
	c.side = side;
	c.ret = false;
	return ctest_try_host (ctest_sbar_thunk_draw, &c);
}

static void ctest_sbar_thunk_intermissionoverlay (void *p)
{
	ctest_sbar_call_t *c = (ctest_sbar_call_t *)p;
	if (c->side)
		c_ref_Sbar_IntermissionOverlay (NULL);
	else
		Host_Reraise (quake_rs_sbar_intermission_overlay (NULL));
}

int ctest_sbar_intermission_overlay (int side)
{
	ctest_sbar_call_t c;
	c.side = side;
	c.ret = false;
	return ctest_try_host (ctest_sbar_thunk_intermissionoverlay, &c);
}

/* ---- the rest, which cannot raise --------------------------------------- */

void ctest_sbar_load_pics (int side)
{
	if (side)
		c_ref_Sbar_LoadPics ();
	else
		quake_rs_sbar_load_pics ();
}

void ctest_sbar_init (int side)
{
	if (side)
		c_ref_Sbar_Init ();
	else
		quake_rs_sbar_init ();
}

/* Sbar_Init's two Cmd_AddCommand calls land in two different registries (the
 * oracle's cmd.c and quake-capi's), so the registration is compared by
 * behaviour: run the command through the matching registry and look at
 * sb_showscores. Same per-side split as ctest_sbar_tokenize above. */
#undef Cmd_ExecuteString
qboolean Cmd_ExecuteString (const char *text, cmd_source_t src);

typedef struct
{
	int			side;
	const char *text;
} ctest_sbar_cmd_t;

static void ctest_sbar_thunk_exec_cmd (void *p)
{
	ctest_sbar_cmd_t *c = (ctest_sbar_cmd_t *)p;
	if (c->side)
		c_ref_Cmd_ExecuteString (c->text, src_command);
	else
		Cmd_ExecuteString (c->text, src_command);
}

int ctest_sbar_exec_cmd (int side, const char *text)
{
	ctest_sbar_cmd_t c;
	c.side = side;
	c.text = text;
	return ctest_try_host (ctest_sbar_thunk_exec_cmd, &c);
}

void ctest_sbar_draw_pic (int side, int x, int y, const char *pic)
{
	if (side)
		c_ref_Sbar_DrawPic (NULL, x, y, ctest_draw_pic (pic));
	else
		quake_rs_sbar_draw_pic (NULL, x, y, ctest_draw_pic (pic));
}

void ctest_sbar_draw_pic_alpha (int side, int x, int y, const char *pic, float alpha)
{
	if (side)
		c_ref_Sbar_DrawPicAlpha (NULL, x, y, ctest_draw_pic (pic), alpha);
	else
		quake_rs_sbar_draw_pic_alpha (NULL, x, y, ctest_draw_pic (pic), alpha);
}

void ctest_sbar_draw_character (int side, int x, int y, int num)
{
	if (side)
		c_ref_Sbar_DrawCharacter (NULL, x, y, num);
	else
		quake_rs_sbar_draw_character (NULL, x, y, num);
}

void ctest_sbar_draw_string (int side, int x, int y, const char *str)
{
	if (side)
		c_ref_Sbar_DrawString (NULL, x, y, str);
	else
		quake_rs_sbar_draw_string (NULL, x, y, str);
}

void ctest_sbar_draw_scroll_string (int side, int x, int y, int width, const char *str)
{
	if (side)
		c_ref_Sbar_DrawScrollString (NULL, x, y, width, str);
	else
		quake_rs_sbar_draw_scroll_string (NULL, x, y, width, str);
}

int ctest_sbar_itoa (int side, int num, char *buf)
{
	return side ? c_ref_Sbar_itoa (num, buf) : quake_rs_sbar_itoa (num, buf);
}

void ctest_sbar_draw_num (int side, int x, int y, int num, int digits, int color)
{
	if (side)
		c_ref_Sbar_DrawNum (NULL, x, y, num, digits, color);
	else
		quake_rs_sbar_draw_num (NULL, x, y, num, digits, color);
}

void ctest_sbar_draw_small_ammo_counter (int side, int x, int y, int val)
{
	if (side)
		c_ref_Sbar_DrawSmallAmmoCounter (NULL, x, y, val);
	else
		quake_rs_sbar_draw_small_ammo_counter (NULL, x, y, val);
}

void ctest_sbar_sort_frags (int side)
{
	if (side)
		c_ref_Sbar_SortFrags ();
	else
		quake_rs_sbar_sort_frags ();
}

int ctest_sbar_color_for_map (int side, int m)
{
	return side ? c_ref_Sbar_ColorForMap (m) : quake_rs_sbar_color_for_map (m);
}

void ctest_sbar_solo_scoreboard (int side)
{
	if (side)
		c_ref_Sbar_SoloScoreboard (NULL);
	else
		quake_rs_sbar_solo_scoreboard (NULL);
}

void ctest_sbar_draw_scoreboard (int side)
{
	if (side)
		c_ref_Sbar_DrawScoreboard (NULL);
	else
		quake_rs_sbar_draw_scoreboard (NULL);
}

/* The pic identity is what matters, and the registry interns by name, so the
 * name is what crosses back to the test (an address would differ per run). */
const char *ctest_sbar_inventory_bar_pic (int side)
{
	qpic_t	   *p = side ? c_ref_Sbar_InventoryBarPic () : (qpic_t *)quake_rs_sbar_inventory_bar_pic ();
	const char *name = ctest_draw_pic_name (p);
	return name ? name : (p ? "?" : "(null)");
}

int ctest_sbar_calculate_flash_on (int side, int val)
{
	return side ? c_ref_Sbar_CalculateFlashOn (val) : quake_rs_sbar_calculate_flash_on (val);
}

void ctest_sbar_draw_inventory (int side)
{
	if (side)
		c_ref_Sbar_DrawInventory (NULL);
	else
		quake_rs_sbar_draw_inventory (NULL);
}

void ctest_sbar_draw_frags (int side)
{
	if (side)
		c_ref_Sbar_DrawFrags (NULL);
	else
		quake_rs_sbar_draw_frags (NULL);
}

void ctest_sbar_draw_face (int side, int x, int y, qboolean classic_style)
{
	if (side)
		c_ref_Sbar_DrawFace (NULL, x, y, classic_style);
	else
		quake_rs_sbar_draw_face (NULL, x, y, classic_style);
}

void ctest_sbar_intermission_number (int side, int x, int y, int num, int digits, int color)
{
	if (side)
		c_ref_Sbar_IntermissionNumber (NULL, x, y, num, digits, color);
	else
		quake_rs_sbar_intermission_number (NULL, x, y, num, digits, color);
}

const char *ctest_sbar_intermission_pic_for_char (int side, char ch, int color)
{
	qpic_t	   *p = side ? c_ref_Sbar_IntermissionPicForChar (ch, color) : (qpic_t *)quake_rs_sbar_intermission_pic_for_char (ch, color);
	const char *name = ctest_draw_pic_name (p);
	return name ? name : (p ? "?" : "(null)");
}

int ctest_sbar_intermission_text_width (int side, const char *str, int color)
{
	return side ? c_ref_Sbar_IntermissionTextWidth (str, color) : quake_rs_sbar_intermission_text_width (str, color);
}

void ctest_sbar_intermission_text (int side, int x, int y, const char *str, int color)
{
	if (side)
		c_ref_Sbar_IntermissionText (NULL, x, y, str, color);
	else
		quake_rs_sbar_intermission_text (NULL, x, y, str, color);
}

void ctest_sbar_deathmatch_overlay (int side)
{
	if (side)
		c_ref_Sbar_DeathmatchOverlay (NULL);
	else
		quake_rs_sbar_deathmatch_overlay (NULL);
}

void ctest_sbar_mini_deathmatch_overlay (int side)
{
	if (side)
		c_ref_Sbar_MiniDeathmatchOverlay (NULL);
	else
		quake_rs_sbar_mini_deathmatch_overlay (NULL);
}

void ctest_sbar_finale_overlay (int side)
{
	if (side)
		c_ref_Sbar_FinaleOverlay (NULL);
	else
		quake_rs_sbar_finale_overlay (NULL);
}
