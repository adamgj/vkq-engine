/*
Copyright (C) 2026 vkqr-engine contributors

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
// sbar_glue.c -- the C frame around the Rust status-bar port.
//
// Compiled instead of sbar.c under -Duse_rust_host (Rust migration Phase 7
// M10d), mirroring console_glue.c, keys_glue.c and view_glue.c:
//
//  1. Own the four C-visible objects sbar.c defined: sb_showscores
//     (sbar.c:47), sb_lines (:49), fragsort[] (:439) and scoreboardlines
//     (:441). All four had external linkage and all four have live readers
//     outside sbar.c -- gl_screen.c:397-410 writes and reads sb_lines,
//     gl_screen.c:682 reads sb_showscores, and pr_ext.c:5344-5347 reads
//     fragsort and scoreboardlines -- so the storage stays here and Rust
//     reaches it through externs (ADR-007). Everything else sbar.c defined
//     at file scope is static with no reader outside the file (the ~150
//     qpic_t * handles, hipweapons[] and hudtype), so it moves to Rust.
//  2. Own every plain Sbar_* name. Each one is a wrapper over a quake_rs_*
//     core, for two reasons: most of these prototypes mention cb_context_t *
//     or qpic_t *, neither of which has a cbindgen spelling, and
//     quake-ctest/stubs/host_ref.c already defines Sbar_Init as a link
//     double in the oracle link, so no Rust translation unit may export a
//     plain Sbar_ name.
//  3. Re-raise for the five entry points that can reach QC (ADR-009 rule 3).
//     sbar.c's only longjmp-capable callee is PR_ExecuteProgram, at
//     sbar.c:82 (Sbar_CSQCCommand), :864 and :870 (Sbar_DrawCSCQ) and :1590
//     (Sbar_IntermissionOverlay). The Rust side calls it through the
//     existing Host_Glue_PR_ExecuteProgram trampoline (host_glue.c:532), so
//     no new Host_Guard is needed here; the five reachable entry points --
//     Sbar_CSQCCommand, Sbar_ShowScores, Sbar_Glue_DontShowScores, Sbar_Draw
//     and Sbar_IntermissionOverlay -- are thin wrappers over quake_rs_*
//     status cores and Host_Reraise is called only from this file.
//  4. Leave everything else plain. The Draw_*/M_*/GL_SetCanvas renderer
//     entry points, W_GetLumpName, COM_SanitizeDescriptionString,
//     PR_MakeTempString, PR_SwitchQCVM, Cmd_AddCommand and q_snprintf/va
//     cannot longjmp, so the Rust side calls them directly.
//
// Accepted, pre-existing exposure: Draw_PicFromWad2 warns through
// Con_Warning/Con_DPrintf, and Con_Printf's screen-update tail can reach
// Mod_LoadModel -> Host_Error (gl_model.c:531). That is the standing project
// exposure every client-stratum port inherits; it is not guarded here.
//
// Sbar_DontShowScores was static in sbar.c. It is exported as
// Sbar_Glue_DontShowScores so the Rust Sbar_Init can register it with
// Cmd_AddCommand while the raise still leaves from a C frame.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

/* ---------------------------------------------------------------------------
 * C-visible objects (sbar.c:47, :49, :439, :441).
 */

qboolean sb_showscores;

int sb_lines; // scan lines to draw

int fragsort[MAX_SCOREBOARD];
int scoreboardlines;

/* ---------------------------------------------------------------------------
 * Re-raising entry points. The Rust bodies are quake_rs_* status cores and the
 * jump is re-issued from here, never from a Rust frame (ADR-009).
 */

/* sbar.c:75 -- runs CSQC_ConsoleCommand. The result and the guard status are
   split (ADR-009 rule 2): a raise must not be mistaken for a return value. */
qboolean Sbar_CSQCCommand (void)
{
	bool ret = false;
	int	 r = quake_rs_sbar_csqc_command (&ret);
	Host_Reraise (r);
	return ret;
}

/* sbar.c:96 -- reaches Sbar_CSQCCommand. */
void Sbar_ShowScores (void)
{
	int r = quake_rs_sbar_show_scores ();
	Host_Reraise (r);
}

/* sbar.c:110 -- static in sbar.c; see the header note. */
void Sbar_Glue_DontShowScores (void)
{
	int r = quake_rs_sbar_dont_show_scores ();
	Host_Reraise (r);
}

/* sbar.c:1262 -- reaches CSQC_DrawHud/CSQC_DrawScores through Sbar_DrawCSCQ. */
void Sbar_Draw (cb_context_t *cbx)
{
	int r = quake_rs_sbar_draw (cbx);
	Host_Reraise (r);
}

/* sbar.c:1559 -- reaches CSQC_DrawScores. */
void Sbar_IntermissionOverlay (cb_context_t *cbx)
{
	int r = quake_rs_sbar_intermission_overlay (cbx);
	Host_Reraise (r);
}

/* ---------------------------------------------------------------------------
 * Plain forwards. None of these can raise; they live here only because the
 * plain Sbar_ names must not be exported from Rust (see note 2 above).
 */

/* sbar.c:138 */
void Sbar_LoadPics (void)
{
	quake_rs_sbar_load_pics ();
}

/* sbar.c:283 */
void Sbar_Init (void)
{
	quake_rs_sbar_init ();
}

/* sbar.c:297 */
void Sbar_DrawPic (cb_context_t *cbx, int x, int y, qpic_t *pic)
{
	quake_rs_sbar_draw_pic (cbx, x, y, pic);
}

/* sbar.c:307 */
void Sbar_DrawPicAlpha (cb_context_t *cbx, int x, int y, qpic_t *pic, float alpha)
{
	quake_rs_sbar_draw_pic_alpha (cbx, x, y, pic, alpha);
}

/* sbar.c:317 */
void Sbar_DrawCharacter (cb_context_t *cbx, int x, int y, int num)
{
	quake_rs_sbar_draw_character (cbx, x, y, num);
}

/* sbar.c:327 */
void Sbar_DrawString (cb_context_t *cbx, int x, int y, const char *str)
{
	quake_rs_sbar_draw_string (cbx, x, y, str);
}

/* sbar.c:337 */
void Sbar_DrawScrollString (cb_context_t *cbx, int x, int y, int width, const char *str)
{
	quake_rs_sbar_draw_scroll_string (cbx, x, y, width, str);
}

/* sbar.c:356 */
int Sbar_itoa (int num, char *buf)
{
	return quake_rs_sbar_itoa (num, buf);
}

/* sbar.c:389 */
void Sbar_DrawNum (cb_context_t *cbx, int x, int y, int num, int digits, int color)
{
	quake_rs_sbar_draw_num (cbx, x, y, num, digits, color);
}

/* sbar.c:422 */
void Sbar_DrawSmallAmmoCounter (cb_context_t *cbx, int x, int y, int val)
{
	quake_rs_sbar_draw_small_ammo_counter (cbx, x, y, val);
}

/* sbar.c:446 */
void Sbar_SortFrags (void)
{
	quake_rs_sbar_sort_frags ();
}

/* sbar.c:475 */
int Sbar_ColorForMap (int m)
{
	return quake_rs_sbar_color_for_map (m);
}

/* sbar.c:485 */
void Sbar_SoloScoreboard (cb_context_t *cbx)
{
	quake_rs_sbar_solo_scoreboard (cbx);
}

/* sbar.c:533 */
void Sbar_DrawScoreboard (cb_context_t *cbx)
{
	quake_rs_sbar_draw_scoreboard (cbx);
}

/* sbar.c:546 */
qpic_t *Sbar_InventoryBarPic (void)
{
	return quake_rs_sbar_inventory_bar_pic ();
}

/* sbar.c:558 */
int Sbar_CalculateFlashOn (int val)
{
	return quake_rs_sbar_calculate_flash_on (val);
}

/* sbar.c:581 */
void Sbar_DrawInventory (cb_context_t *cbx)
{
	quake_rs_sbar_draw_inventory (cbx);
}

/* sbar.c:698 */
void Sbar_DrawFrags (cb_context_t *cbx)
{
	quake_rs_sbar_draw_frags (cbx);
}

/* sbar.c:740 */
void Sbar_DrawFace (cb_context_t *cbx, int x, int y, qboolean classic_style)
{
	quake_rs_sbar_draw_face (cbx, x, y, classic_style);
}

/* sbar.c:1309 */
void Sbar_IntermissionNumber (cb_context_t *cbx, int x, int y, int num, int digits, int color)
{
	quake_rs_sbar_intermission_number (cbx, x, y, num, digits, color);
}

/* sbar.c:1340 */
qpic_t *Sbar_IntermissionPicForChar (char c, int color)
{
	return quake_rs_sbar_intermission_pic_for_char (c, color);
}

/* sbar.c:1356 */
int Sbar_IntermissionTextWidth (const char *str, int color)
{
	return quake_rs_sbar_intermission_text_width (str, color);
}

/* sbar.c:1373 */
void Sbar_IntermissionText (cb_context_t *cbx, int x, int y, const char *str, int color)
{
	quake_rs_sbar_intermission_text (cbx, x, y, str, color);
}

/* sbar.c:1390 */
void Sbar_DeathmatchOverlay (cb_context_t *cbx)
{
	quake_rs_sbar_deathmatch_overlay (cbx);
}

/* sbar.c:1471 */
void Sbar_MiniDeathmatchOverlay (cb_context_t *cbx)
{
	quake_rs_sbar_mini_deathmatch_overlay (cbx);
}

/* sbar.c:1629 */
void Sbar_FinaleOverlay (cb_context_t *cbx)
{
	quake_rs_sbar_finale_overlay (cbx);
}

#endif /* USE_RUST_HOST */
