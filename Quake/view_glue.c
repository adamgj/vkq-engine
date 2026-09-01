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
// view_glue.c -- the C frame around the Rust player-view port.
//
// Compiled instead of view.c under -Duse_rust_host (Rust migration Phase 7 M7,
// T7.2a). Four jobs, mirroring sv_user_glue.c:
//
//  1. Own the C-visible objects view.c defined: the 30 cvars (view.c:35-71,
//     :147-148), v_punchangles / v_punchangles_times (:75-76), v_blend (:262)
//     and the three const cshift_t tables (:258-260). menu.c, gl_screen.c,
//     gl_draw.c, cl_parse.c and sbar.c reach these by plain name, so the
//     storage stays in C and Rust reaches it through externs;
//     Cvar_RegisterVariable also keeps receiving stable cvar_t addresses.
//  2. Guard everything view.c reached that can Host_Error / Host_EndGame
//     (ADR-009 rule 3): Cvar_RegisterVariable and Cmd_AddCommand in V_Init
//     (both are Host_Reraise wrappers under -Duse_rust_cvar), plus
//     CL_RelinkEntities and R_RenderView in V_RenderView.
//  3. Re-raise, from a pure C frame, what those guards caught. The two view.c
//     entry points whose bodies transitively reach a raise are thin wrappers
//     over quake_rs_* status cores: V_RenderView and V_Init. Host_Reraise is
//     called only from here.
//  4. Forward the remaining view.c entry points straight to their Rust cores.
//     None of those can raise: the one Sys_Error (view.c:922) aborts rather
//     than jumping, so the Rust side calls it directly -- the world.c,
//     sv_phys.c and sv_send.c precedent.

#include "quakedef.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

/* ---------------------------------------------------------------------------
 * C-visible objects (view.c:35-71, :75-76, :147-148, :258-260, :262).
 */

cvar_t scr_ofsx = {"scr_ofsx", "0", CVAR_NONE};
cvar_t scr_ofsy = {"scr_ofsy", "0", CVAR_NONE};
cvar_t scr_ofsz = {"scr_ofsz", "0", CVAR_NONE};

cvar_t cl_rollspeed = {"cl_rollspeed", "200", CVAR_NONE};
cvar_t cl_rollangle = {"cl_rollangle", "2.0", CVAR_ARCHIVE};

cvar_t cl_bob = {"cl_bob", "0.02", CVAR_ARCHIVE};
cvar_t cl_bobcycle = {"cl_bobcycle", "0.6", CVAR_NONE};
cvar_t cl_bobup = {"cl_bobup", "0.5", CVAR_NONE};

cvar_t v_kicktime = {"v_kicktime", "0.5", CVAR_NONE};
cvar_t v_kickroll = {"v_kickroll", "0.6", CVAR_NONE};
cvar_t v_kickpitch = {"v_kickpitch", "0.6", CVAR_NONE};
cvar_t v_gunkick = {"v_gunkick", "1", CVAR_ARCHIVE}; // johnfitz

cvar_t v_autopitch = {"v_autopitch", "0", CVAR_ARCHIVE};

cvar_t v_iyaw_cycle = {"v_iyaw_cycle", "2", CVAR_NONE};
cvar_t v_iroll_cycle = {"v_iroll_cycle", "0.5", CVAR_NONE};
cvar_t v_ipitch_cycle = {"v_ipitch_cycle", "1", CVAR_NONE};
cvar_t v_iyaw_level = {"v_iyaw_level", "0.3", CVAR_NONE};
cvar_t v_iroll_level = {"v_iroll_level", "0.1", CVAR_NONE};
cvar_t v_ipitch_level = {"v_ipitch_level", "0.3", CVAR_NONE};

cvar_t v_idlescale = {"v_idlescale", "0", CVAR_NONE};

cvar_t crosshair = {"crosshair", "1", CVAR_ARCHIVE};
cvar_t crosshair_def = {"crosshair_def", "0", CVAR_ARCHIVE};

cvar_t gl_cshiftpercent = {"gl_cshiftpercent", "100", CVAR_NONE};
cvar_t gl_cshiftpercent_contents = {"gl_cshiftpercent_contents", "100", CVAR_NONE}; // QuakeSpasm
cvar_t gl_cshiftpercent_damage = {"gl_cshiftpercent_damage", "100", CVAR_NONE};		// QuakeSpasm
cvar_t gl_cshiftpercent_bonus = {"gl_cshiftpercent_bonus", "100", CVAR_NONE};		// QuakeSpasm
cvar_t gl_cshiftpercent_powerup = {"gl_cshiftpercent_powerup", "100", CVAR_NONE};	// QuakeSpasm

cvar_t r_viewmodel_quake = {"r_viewmodel_quake", "0", CVAR_ARCHIVE};

cvar_t v_centermove = {"v_centermove", "0.15", CVAR_NONE};
cvar_t v_centerspeed = {"v_centerspeed", "500", CVAR_NONE};

vec3_t v_punchangles[2];	   // johnfitz -- copied from cl.punchangle.  0 is current, 1 is previous value. never the same unless map just loaded
double v_punchangles_times[2]; // spike -- times, to avoid assumptions...

uint8_t v_blend[4]; // rgba 0 - 255

const cshift_t cshift_water = {{130, 80, 50}, 128};
const cshift_t cshift_slime = {{0, 25, 5}, 150};
const cshift_t cshift_lava = {{255, 80, 0}, 150};

/* ---------------------------------------------------------------------------
 * Guarded callbacks (ADR-009 rule 3).
 */

/* view.c:945-975 -- one Cvar_RegisterVariable. Under -Duse_rust_cvar the plain
   name is itself a Host_Reraise wrapper (cvar_cmd_glue.c), so a Rust frame
   must never call it directly. */
static void View_InvokeRegisterVariable (void *p)
{
	Cvar_RegisterVariable ((cvar_t *)p);
}

int View_Glue_RegisterVariable (cvar_t *var)
{
	return Host_Guard (View_InvokeRegisterVariable, var);
}

/* view.c:356 / :371 -- external linkage in the C original but declared in no
   header, so the plain-named wrappers below need forward declarations here. */
void V_cshift_f (void);
void V_BonusFlash_f (void);

/* view.c:942-944 -- the three Cmd_AddCommand calls, kept in one guarded frame
   because they are adjacent and unconditional. The handlers are the
   plain-named ABI wrappers defined below, so the command table keeps holding
   stable C function pointers. */
static void View_InvokeAddCommands (void *p)
{
	(void)p;
	Cmd_AddCommand ("v_cshift", V_cshift_f);
	Cmd_AddCommand ("bf", V_BonusFlash_f);
	Cmd_AddCommand ("centerview", V_StartPitchDrift);
}

int View_Glue_AddCommands (void)
{
	return Host_Guard (View_InvokeAddCommands, NULL);
}

/* view.c:924 -- CL_RelinkEntities, which reaches Mem_Realloc, R_AddEfrags and
   the PScript_* particle system, all of which can Host_Error. */
static void View_InvokeRelinkEntities (void *p)
{
	(void)p;
	CL_RelinkEntities ();
}

int View_Glue_RelinkEntities (void)
{
	return Host_Guard (View_InvokeRelinkEntities, NULL);
}

/* view.c:927 -- R_RenderView, i.e. the whole renderer. */
typedef struct
{
	qboolean	  use_tasks;
	task_handle_t begin_rendering_task;
	task_handle_t setup_frame_task;
	task_handle_t draw_done_task;
} view_renderview_args_t;

static void View_InvokeRenderView (void *p)
{
	view_renderview_args_t *a = (view_renderview_args_t *)p;
	R_RenderView (a->use_tasks, a->begin_rendering_task, a->setup_frame_task, a->draw_done_task);
}

int View_Glue_RenderView (qboolean use_tasks, task_handle_t begin_rendering_task, task_handle_t setup_frame_task, task_handle_t draw_done_task)
{
	view_renderview_args_t args;
	args.use_tasks = use_tasks;
	args.begin_rendering_task = begin_rendering_task;
	args.setup_frame_task = setup_frame_task;
	args.draw_done_task = draw_done_task;
	return Host_Guard (View_InvokeRenderView, &args);
}

/* ---------------------------------------------------------------------------
 * Re-raising public entry points (ADR-009). Each is the exact view.c
 * signature; the Rust body is a quake_rs_* status core and the jump is
 * re-issued from here, never from a Rust frame.
 */

/* view.c:908 */
void V_RenderView (qboolean use_tasks, task_handle_t begin_rendering_task, task_handle_t setup_frame_task, task_handle_t draw_done_task)
{
	int r = quake_rs_v_render_view (use_tasks, begin_rendering_task, setup_frame_task, draw_done_task);
	Host_Reraise (r);
}

/* view.c:940 */
void V_Init (void)
{
	int r = quake_rs_v_init ();
	Host_Reraise (r);
}

/* ---------------------------------------------------------------------------
 * Non-raising public entry points: plain forwards to the Rust cores.
 */

/* view.c:87 */
float V_CalcRoll (vec3_t angles, vec3_t velocity)
{
	return quake_rs_v_calc_roll (angles, velocity);
}

/* view.c:117 */
float V_CalcBob (void)
{
	return quake_rs_v_calc_bob ();
}

/* view.c:150 */
void V_StartPitchDrift (void)
{
	quake_rs_v_start_pitch_drift ();
}

/* view.c:166 */
void V_StopPitchDrift (void)
{
	quake_rs_v_stop_pitch_drift ();
}

/* view.c:185 */
void V_DriftPitch (void)
{
	quake_rs_v_drift_pitch ();
}

/* view.c:271 */
void V_ResetBlend (void)
{
	quake_rs_v_reset_blend ();
}

/* view.c:281 */
void V_ParseDamage (void)
{
	quake_rs_v_parse_damage ();
}

/* view.c:351 */
void V_cshift_f (void)
{
	quake_rs_v_cshift_f ();
}

/* view.c:366 */
void V_BonusFlash_f (void)
{
	quake_rs_v_bonus_flash_f ();
}

/* view.c:381 */
void V_SetContentsColor (int contents)
{
	quake_rs_v_set_contents_color (contents);
}

/* view.c:411 */
void V_CalcPowerupCshift (void)
{
	quake_rs_v_calc_powerup_cshift ();
}

/* view.c:448 */
void V_CalcBlend (void)
{
	quake_rs_v_calc_blend ();
}

/* view.c:538 */
float angledelta (float a)
{
	return quake_rs_angledelta (a);
}

/* view.c:551 */
void CalcGunAngle (void)
{
	quake_rs_calc_gun_angle ();
}

/* view.c:606 */
void V_BoundOffsets (void)
{
	quake_rs_v_bound_offsets ();
}

/* view.c:634 */
void V_AddIdle (void)
{
	quake_rs_v_add_idle ();
}

/* view.c:648 */
void V_CalcViewRoll (void)
{
	quake_rs_v_calc_view_roll ();
}

/* view.c:677 */
void V_CalcIntermissionRefdef (void)
{
	quake_rs_v_calc_intermission_refdef ();
}

/* view.c:710 */
void V_CalcRefdef (void)
{
	quake_rs_v_calc_refdef ();
}

/* view.c:877 */
void V_RestoreAngles (void)
{
	quake_rs_v_restore_angles ();
}

/* view.c:888 */
void V_SetupFrame (void)
{
	quake_rs_v_setup_frame ();
}

#endif /* USE_RUST_HOST */
