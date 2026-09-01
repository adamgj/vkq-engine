/* Phase 7 M7 oracle TU for Quake/view.c (task T7.2a).
 *
 * c_ref_prelude.h is force-included by build.rs and pulls in the real
 * Quake/client.h, Quake/render.h, Quake/view.h and Quake/cmd.h, so
 * client_state_t, entity_t, entlerp_t, refdef_t and cshift_t here are the
 * engine's own declarations. Quake/view.c is an oracle source (build.rs
 * C_SOURCES), so its 23 entry points are reachable as c_ref_<name> and its 30
 * cvars as c_ref_<name> (c_ref_prelude.h "view.c" block, :1628ff).
 *
 * This file is the PLAIN (Rust-reading) half of the link -- the same shape
 * Quake/view_glue.c gives the real engine:
 *
 *   - the C-visible objects view.c owns, MINUS cl_rollspeed/cl_rollangle,
 *     which stubs.c:6913-6962 already defines plain (together with a
 *     hand-transcribed plain V_CalcRoll) for the M6 sv_user.c wave.
 *     Redefining any of those three here would be LNK2005, so
 *     view_differential.rs drives quake_rs_v_calc_roll directly instead of
 *     going through a plain V_CalcRoll.
 *   - the four View_Glue_* Host_Guard trampolines (ADR-009), byte-for-byte
 *     the bodies Quake/view_glue.c uses.
 *   - the plain-named drivers over the quake_rs_* cores.
 *   - the fixture, which must seed BOTH sides of every input.
 *     Cvar_RegisterVariable does not run for these objects in this link
 *     (except in the one V_Init test), so a cvar left at its static
 *     initializer would read .value == 0 on both sides and every comparison
 *     downstream of it would pass while measuring nothing -- the trap T7.0
 *     hit with cl_rollangle/cl_rollspeed. ctest_view_set_cvars therefore
 *     writes one table into both halves.
 *
 * CROSS-WAVE DEPENDENCY (T7.2b, stubs/cl_input_ref.c): cl_forwardspeed and
 * lookspring are cl_input.c cvars whose plain copies the T7.2b peer owns.
 * They are declared and never defined here; until cl_input_ref.c replaces its
 * placeholder no ctest binary links. Same hand-off shape as sv_user_ref.c's
 * sv/svs dependency in M6.
 *
 * `needs_relink` is the mirror case: cl_main.c (an oracle source) defines it
 * and the prelude renames it (:1606), so only c_ref_needs_relink exists and
 * the plain twin is defined below. It moves to cl_main.c's glue in T7.4.
 *
 * ADR-009. view.c has one direct raise site, Sys_Error at view.c:922, which
 * aborts rather than jumping in the real engine, so the Rust port calls it
 * directly (the world.c / sv_phys.c / sv_send.c precedent). It has four
 * transitive ones: Cvar_RegisterVariable and Cmd_AddCommand in V_Init, and
 * CL_RelinkEntities and R_RenderView in V_RenderView. Only V_Init and
 * V_RenderView therefore get quake_rs_* status cores, and only their plain
 * wrappers below call Host_Reraise -- from a pure C frame, never a Rust one.
 */

#include <stddef.h>
#include <string.h>

extern int	Host_Guard (void (*fn) (void *), void *arg);
extern void Host_Reraise (int guard_result);

/* stubs.c's synthetic room (the shared M3 world fixture): a brush model with
 * real clipping hulls. chase.c's TraceLine needs cl.worldmodel to be one, and
 * ctest_world_reset publishes it only on the ORACLE copy of cl (stubs.c
 * re-#defines cl to c_ref_cl around its own definition, stubs.c:2656-2686), so
 * ctest_view_reset republishes it on the plain copy too. */
extern void	 ctest_world_reset (int vm_kind, int num_edicts);
extern void *ctest_world_model (void);

/* net_message staging, reused rather than duplicated: sv_user_ref.c already
 * owns one MAX_DATAGRAM buffer per side and writes identical bytes into both,
 * which is exactly what V_ParseDamage needs. */
extern void ctest_svuser_load_message (const unsigned char *data, int len);

/* ---------------------------------------------------------------------------
 * Oracle-side handles. The 30 cvars, v_punchangles/v_punchangles_times and
 * needs_relink have no header declaration at all (view.c and cl_main.c define
 * them and consumers re-declare them locally), so their c_ref_ twins are
 * spelled out. c_ref_cl / c_ref_cls / c_ref_v_blend and the seven view.h entry
 * points arrive through the prelude's own header includes.
 */
extern cvar_t c_ref_scr_ofsx, c_ref_scr_ofsy, c_ref_scr_ofsz;
extern cvar_t c_ref_cl_rollspeed, c_ref_cl_rollangle;
extern cvar_t c_ref_cl_bob, c_ref_cl_bobcycle, c_ref_cl_bobup;
extern cvar_t c_ref_v_kicktime, c_ref_v_kickroll, c_ref_v_kickpitch, c_ref_v_gunkick;
extern cvar_t c_ref_v_autopitch;
extern cvar_t c_ref_v_iyaw_cycle, c_ref_v_iroll_cycle, c_ref_v_ipitch_cycle;
extern cvar_t c_ref_v_iyaw_level, c_ref_v_iroll_level, c_ref_v_ipitch_level;
extern cvar_t c_ref_v_idlescale;
extern cvar_t c_ref_crosshair, c_ref_crosshair_def;
extern cvar_t c_ref_gl_cshiftpercent, c_ref_gl_cshiftpercent_contents, c_ref_gl_cshiftpercent_damage;
extern cvar_t c_ref_gl_cshiftpercent_bonus, c_ref_gl_cshiftpercent_powerup;
extern cvar_t c_ref_r_viewmodel_quake;
extern cvar_t c_ref_v_centermove, c_ref_v_centerspeed;

extern vec3_t	c_ref_v_punchangles[2];
extern double	c_ref_v_punchangles_times[2];
extern qboolean c_ref_needs_relink;

/* chase.c's chase_active, read by V_CalcRefdef. chase_ref.c owns the plain
 * copy; both halves are reachable from here. */
extern cvar_t c_ref_chase_active;

/* cl_input.c cvars V_DriftPitch reads (view.c:190, :232). */
extern cvar_t c_ref_cl_forwardspeed;
extern cvar_t c_ref_lookspring;

/* view.c entry points view.h does not declare; only the two the fixture calls
 * itself are needed here (view_differential.rs declares the rest). */
extern void c_ref_V_CalcRefdef (void);

/* The three command handlers V_Init installs, needed by ctest_view_cmd_handler
 * to tell the two sides' function pointers apart in cmd.c's one table. */
extern void c_ref_V_cshift_f (void);
extern void c_ref_V_BonusFlash_f (void);
extern void c_ref_V_StartPitchDrift (void);

/* ---------------------------------------------------------------------------
 * Plain (Rust-reading) storage this wave owns. Initializers are verbatim from
 * Quake/view_glue.c, which took them from view.c:35-71, :147-148, :258-262.
 * .value stays 0 on purpose: ctest_view_set_cvars seeds both sides from one
 * table so the halves cannot drift apart through a static initializer.
 */
#undef scr_ofsx
#undef scr_ofsy
#undef scr_ofsz
#undef cl_bob
#undef cl_bobcycle
#undef cl_bobup
#undef v_kicktime
#undef v_kickroll
#undef v_kickpitch
#undef v_gunkick
#undef v_autopitch
#undef v_iyaw_cycle
#undef v_iroll_cycle
#undef v_ipitch_cycle
#undef v_iyaw_level
#undef v_iroll_level
#undef v_ipitch_level
#undef v_idlescale
#undef crosshair
#undef crosshair_def
#undef gl_cshiftpercent
#undef gl_cshiftpercent_contents
#undef gl_cshiftpercent_damage
#undef gl_cshiftpercent_bonus
#undef gl_cshiftpercent_powerup
#undef r_viewmodel_quake
#undef v_centermove
#undef v_centerspeed
#undef v_punchangles
#undef v_punchangles_times
#undef v_blend
#undef cshift_water
#undef cshift_slime
#undef cshift_lava
#undef needs_relink

cvar_t scr_ofsx = {"scr_ofsx", "0", CVAR_NONE};
cvar_t scr_ofsy = {"scr_ofsy", "0", CVAR_NONE};
cvar_t scr_ofsz = {"scr_ofsz", "0", CVAR_NONE};

cvar_t cl_bob = {"cl_bob", "0.02", CVAR_ARCHIVE};
cvar_t cl_bobcycle = {"cl_bobcycle", "0.6", CVAR_NONE};
cvar_t cl_bobup = {"cl_bobup", "0.5", CVAR_NONE};

cvar_t v_kicktime = {"v_kicktime", "0.5", CVAR_NONE};
cvar_t v_kickroll = {"v_kickroll", "0.6", CVAR_NONE};
cvar_t v_kickpitch = {"v_kickpitch", "0.6", CVAR_NONE};
cvar_t v_gunkick = {"v_gunkick", "1", CVAR_ARCHIVE};

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
cvar_t gl_cshiftpercent_contents = {"gl_cshiftpercent_contents", "100", CVAR_NONE};
cvar_t gl_cshiftpercent_damage = {"gl_cshiftpercent_damage", "100", CVAR_NONE};
cvar_t gl_cshiftpercent_bonus = {"gl_cshiftpercent_bonus", "100", CVAR_NONE};
cvar_t gl_cshiftpercent_powerup = {"gl_cshiftpercent_powerup", "100", CVAR_NONE};

cvar_t r_viewmodel_quake = {"r_viewmodel_quake", "0", CVAR_ARCHIVE};

cvar_t v_centermove = {"v_centermove", "0.15", CVAR_NONE};
cvar_t v_centerspeed = {"v_centerspeed", "500", CVAR_NONE};

vec3_t v_punchangles[2];
double v_punchangles_times[2];

uint8_t v_blend[4];

/* view.c:258-260. Nothing outside view.c reads these, but the plain twins
 * exist so this TU's ABI matches Quake/view_glue.c exactly. The Rust port
 * carries its own const copies (quake-capi/src/view.rs), which the
 * set_contents_color test group pins against these. */
const cshift_t cshift_water = {{130, 80, 50}, 128};
const cshift_t cshift_slime = {{0, 25, 5}, 150};
const cshift_t cshift_lava = {{255, 80, 0}, 150};

/* cl_main.c:73 in the real engine; T7.4 hand-off. */
qboolean needs_relink;

/* ---------------------------------------------------------------------------
 * Plain handles this TU only reads or writes.
 */
#undef cl
#undef cls
#undef cl_rollspeed
#undef cl_rollangle
#undef cl_forwardspeed
#undef lookspring
#undef chase_active
extern client_state_t  cl;
extern client_static_t cls;
extern cvar_t		   cl_rollspeed, cl_rollangle;
extern cvar_t		   cl_forwardspeed, lookspring;
extern cvar_t		   chase_active;

/* Rust-routed Cvar_RegisterVariable (stubs.c:1848 -- it forwards to
 * quake_rs_cvar_register_variable and re-raises, exactly like the real
 * cvar_cmd_glue.c under -Duse_rust_cvar), plus quake-capi's plain lookup used
 * by the V_Init test. */
#undef Cvar_RegisterVariable
#undef Cvar_FindVar
extern void	   Cvar_RegisterVariable (cvar_t *variable);
extern cvar_t *Cvar_FindVar (const char *var_name);

/* ---------------------------------------------------------------------------
 * The Rust cores (rust/quake-capi/src/view.rs). build-rs/quake_rs.h is
 * generated by cbindgen at Meson build time and does not exist in the ctest
 * link, so every core is declared by hand -- the sv_main_ref.c/sv_user_ref.c
 * idiom.
 */
extern float quake_rs_v_calc_bob (void);
extern void	 quake_rs_v_start_pitch_drift (void);
extern void	 quake_rs_v_stop_pitch_drift (void);
extern void	 quake_rs_v_drift_pitch (void);
extern void	 quake_rs_v_reset_blend (void);
extern void	 quake_rs_v_parse_damage (void);
extern void	 quake_rs_v_cshift_f (void);
extern void	 quake_rs_v_bonus_flash_f (void);
extern void	 quake_rs_v_set_contents_color (int contents);
extern void	 quake_rs_v_calc_powerup_cshift (void);
extern void	 quake_rs_v_calc_blend (void);
extern float quake_rs_angledelta (float a);
extern void	 quake_rs_calc_gun_angle (void);
extern void	 quake_rs_v_bound_offsets (void);
extern void	 quake_rs_v_add_idle (void);
extern void	 quake_rs_v_calc_view_roll (void);
extern void	 quake_rs_v_calc_intermission_refdef (void);
extern void	 quake_rs_v_calc_refdef (void);
extern void	 quake_rs_v_restore_angles (void);
extern void	 quake_rs_v_setup_frame (void);
extern int	 quake_rs_v_render_view (qboolean use_tasks, task_handle_t begin_rendering_task, task_handle_t setup_frame_task, task_handle_t draw_done_task);
extern int	 quake_rs_v_init (void);

/* ---------------------------------------------------------------------------
 * Plain-named drivers, one for one with Quake/view_glue.c. Each name is
 * #undef'd first: the prelude's rename macros are still live in this TU and
 * would otherwise rewrite the definition to c_ref_*, colliding at link time
 * with the real oracle compiled from view.c.
 *
 * V_CalcRoll is deliberately absent -- stubs.c:6939 owns the plain copy.
 */
#undef V_CalcBob
#undef V_StartPitchDrift
#undef V_StopPitchDrift
#undef V_DriftPitch
#undef V_ResetBlend
#undef V_ParseDamage
#undef V_cshift_f
#undef V_BonusFlash_f
#undef V_SetContentsColor
#undef V_CalcPowerupCshift
#undef V_CalcBlend
#undef angledelta
#undef CalcGunAngle
#undef V_BoundOffsets
#undef V_AddIdle
#undef V_CalcViewRoll
#undef V_CalcIntermissionRefdef
#undef V_CalcRefdef
#undef V_RestoreAngles
#undef V_SetupFrame
#undef V_RenderView
#undef V_Init

float V_CalcBob (void)
{
	return quake_rs_v_calc_bob ();
}

void V_StartPitchDrift (void)
{
	quake_rs_v_start_pitch_drift ();
}

void V_StopPitchDrift (void)
{
	quake_rs_v_stop_pitch_drift ();
}

void V_DriftPitch (void)
{
	quake_rs_v_drift_pitch ();
}

void V_ResetBlend (void)
{
	quake_rs_v_reset_blend ();
}

void V_ParseDamage (void)
{
	quake_rs_v_parse_damage ();
}

void V_cshift_f (void)
{
	quake_rs_v_cshift_f ();
}

void V_BonusFlash_f (void)
{
	quake_rs_v_bonus_flash_f ();
}

void V_SetContentsColor (int contents)
{
	quake_rs_v_set_contents_color (contents);
}

void V_CalcPowerupCshift (void)
{
	quake_rs_v_calc_powerup_cshift ();
}

void V_CalcBlend (void)
{
	quake_rs_v_calc_blend ();
}

float angledelta (float a)
{
	return quake_rs_angledelta (a);
}

void CalcGunAngle (void)
{
	quake_rs_calc_gun_angle ();
}

void V_BoundOffsets (void)
{
	quake_rs_v_bound_offsets ();
}

void V_AddIdle (void)
{
	quake_rs_v_add_idle ();
}

void V_CalcViewRoll (void)
{
	quake_rs_v_calc_view_roll ();
}

void V_CalcIntermissionRefdef (void)
{
	quake_rs_v_calc_intermission_refdef ();
}

void V_CalcRefdef (void)
{
	quake_rs_v_calc_refdef ();
}

void V_RestoreAngles (void)
{
	quake_rs_v_restore_angles ();
}

void V_SetupFrame (void)
{
	quake_rs_v_setup_frame ();
}

void V_RenderView (qboolean use_tasks, task_handle_t begin_rendering_task, task_handle_t setup_frame_task, task_handle_t draw_done_task)
{
	Host_Reraise (quake_rs_v_render_view (use_tasks, begin_rendering_task, setup_frame_task, draw_done_task));
}

void V_Init (void)
{
	Host_Reraise (quake_rs_v_init ());
}

/* ---------------------------------------------------------------------------
 * View_Glue_* trampolines (ADR-009), the same bodies as Quake/view_glue.c.
 * Defined after the drivers above so View_InvokeAddCommands installs the plain
 * (Rust-routed) handlers, exactly as the real glue does.
 */
static void View_InvokeRegisterVariable (void *p)
{
	Cvar_RegisterVariable ((cvar_t *)p);
}

int View_Glue_RegisterVariable (cvar_t *var)
{
	return Host_Guard (View_InvokeRegisterVariable, var);
}

/* Cmd_AddCommand stays under the active rename here. cmd.c is an oracle
 * source and there is no plain Cmd_AddCommand2 in this link, so both sides
 * register into cmd.c's one command table -- an identity mapping of an
 * unported dependency, the arrangement stubs.c already uses for R_RenderView.
 * The command *names* and the installed function pointers still differ per
 * side, which is what the V_Init test asserts on. */
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

/* CL_RelinkEntities likewise stays renamed: cl_main.c IS an oracle source, and
 * this fixture predates T7.4's plain twin, so it drives the oracle directly. */
static void View_InvokeRelinkEntities (void *p)
{
	(void)p;
	CL_RelinkEntities ();
}

int View_Glue_RelinkEntities (void)
{
	return Host_Guard (View_InvokeRelinkEntities, NULL);
}

typedef struct
{
	qboolean	  use_tasks;
	task_handle_t begin_rendering_task;
	task_handle_t setup_frame_task;
	task_handle_t draw_done_task;
} view_renderview_args_t;

/* R_RenderView is never renamed (c_ref_prelude.h:1336): gl_rmain.c is not an
 * oracle source, so stubs.c:7245 provides the single shared stand-in, which
 * Sys_Errors. Both sides reach the same one. */
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

/* ===========================================================================
 * THE FIXTURE
 * =========================================================================== */

#define CTEST_VIEW_ENTS	  4
#define CTEST_VIEW_MODELS 4
#define CTEST_VIEW_CVARS  33

/* ONE shared entity array both cl copies point at, so the two runs start from
 * byte-identical entity state by construction rather than by convention.
 * cl.viewent is inline in client_state_t, so that one genuinely exists twice
 * and is seeded and snapshotted per side. */
static entity_t ctest_view_entities[CTEST_VIEW_ENTS];
static qmodel_t ctest_view_models[CTEST_VIEW_MODELS];

typedef struct
{
	double time, oldtime, mtime0, mtime1, laststop, host_frametime;
	float  velocity[3], viewangles[3], punchangle[3];
	float  ent_origin[3], ent_angles[3], ent_msg_angles[3];
	float  viewent_origin[3], viewent_angles[3];
	double viewent_frame_change_time, viewent_frame_duration, viewent_frame_finish_time;
	int	   viewent_frame, viewent_prev_frame, viewent_snap_frames, viewent_model_idx, viewent_colormap;
	int	   stat_health, stat_weapon, stat_weaponframe, stat_viewheight;
	float  statsf_idealpitch;
	int	   items, intermission, maxclients, viewentity;
	int	   onground, inwater, paused, nodrift, demoplayback, demoseeking;
	float  pitchvel, driftmove, idealpitch, faceanimtime;
	float  v_dmg_time, v_dmg_roll, v_dmg_pitch;
	int	   movemessages;
	float  movecmd_forwardmove;
	int	   cshifts_dest[NUM_CSHIFTS][3];
	float  cshifts_pct[NUM_CSHIFTS];
	int	   prev_dest[NUM_CSHIFTS][3];
	float  prev_pct[NUM_CSHIFTS];
	int	   empty_dest[3];
	float  empty_pct;
	float  punchangles[2][3];
	double punchangles_times[2];
	int	   blend[4];
	int	   noclip_anglehack, con_forcedup, needs_relink;
	float  scr_viewsize;
	float  vieworg[3], refdef_viewangles[3];
	int	   trace_line_cache_counter, render_scale, render_warp;
	int	   protocolflags;
} ctest_view_state_t;

typedef struct
{
	float  vieworg[3];
	float  viewangles[3];
	float  cl_viewangles[3];
	float  cl_punchangle[3];
	float  pitchvel, driftmove, idealpitch, faceanimtime;
	int	   nodrift;
	double laststop;
	float  v_dmg_time, v_dmg_roll, v_dmg_pitch;
	int	   cshifts_dest[NUM_CSHIFTS][3];
	float  cshifts_pct[NUM_CSHIFTS];
	int	   prev_dest[NUM_CSHIFTS][3];
	float  prev_pct[NUM_CSHIFTS];
	int	   empty_dest[3];
	float  empty_pct;
	float  viewent_origin[3];
	float  viewent_angles[3];
	int	   viewent_frame, viewent_prev_frame, viewent_snap_frames;
	double viewent_frame_change_time, viewent_frame_duration, viewent_frame_finish_time;
	int	   viewent_model_idx, viewent_colormap;
	float  ent_origin[3];
	float  ent_angles[3];
	int	   blend[4];
	float  punchangles[2][3];
	double punchangles_times[2];
	int	   trace_line_cache_counter, render_scale, render_warp;
} ctest_view_snap_t;

typedef struct
{
	int			 found;	   /* the side's registry resolves the name to THIS object */
	unsigned int flags;	   /* after CVAR_REGISTERED is set */
	float		 value;	   /* parsed out of the static initializer's string */
	char		 name[64]; /* the registered name */
	char		 string[64];
} ctest_view_cvar_info_t;

static void ctest_view_vcopy (float *dst, const float *src)
{
	dst[0] = src[0];
	dst[1] = src[1];
	dst[2] = src[2];
}

static cvar_t *ctest_view_cvar (int idx, int oracle)
{
	/* view.c declaration order (view.c:35-71, :147-148), then the two
	 * cl_input.c cvars V_DriftPitch reads, then chase_active (V_CalcRefdef). */
	static cvar_t *const plain[CTEST_VIEW_CVARS] = {
		&scr_ofsx,
		&scr_ofsy,
		&scr_ofsz,
		&cl_rollspeed,
		&cl_rollangle,
		&cl_bob,
		&cl_bobcycle,
		&cl_bobup,
		&v_kicktime,
		&v_kickroll,
		&v_kickpitch,
		&v_gunkick,
		&v_autopitch,
		&v_iyaw_cycle,
		&v_iroll_cycle,
		&v_ipitch_cycle,
		&v_iyaw_level,
		&v_iroll_level,
		&v_ipitch_level,
		&v_idlescale,
		&crosshair,
		&crosshair_def,
		&gl_cshiftpercent,
		&gl_cshiftpercent_contents,
		&gl_cshiftpercent_damage,
		&gl_cshiftpercent_bonus,
		&gl_cshiftpercent_powerup,
		&r_viewmodel_quake,
		&v_centermove,
		&v_centerspeed,
		&cl_forwardspeed,
		&lookspring,
		&chase_active,
	};
	static cvar_t *const ref[CTEST_VIEW_CVARS] = {
		&c_ref_scr_ofsx,
		&c_ref_scr_ofsy,
		&c_ref_scr_ofsz,
		&c_ref_cl_rollspeed,
		&c_ref_cl_rollangle,
		&c_ref_cl_bob,
		&c_ref_cl_bobcycle,
		&c_ref_cl_bobup,
		&c_ref_v_kicktime,
		&c_ref_v_kickroll,
		&c_ref_v_kickpitch,
		&c_ref_v_gunkick,
		&c_ref_v_autopitch,
		&c_ref_v_iyaw_cycle,
		&c_ref_v_iroll_cycle,
		&c_ref_v_ipitch_cycle,
		&c_ref_v_iyaw_level,
		&c_ref_v_iroll_level,
		&c_ref_v_ipitch_level,
		&c_ref_v_idlescale,
		&c_ref_crosshair,
		&c_ref_crosshair_def,
		&c_ref_gl_cshiftpercent,
		&c_ref_gl_cshiftpercent_contents,
		&c_ref_gl_cshiftpercent_damage,
		&c_ref_gl_cshiftpercent_bonus,
		&c_ref_gl_cshiftpercent_powerup,
		&c_ref_r_viewmodel_quake,
		&c_ref_v_centermove,
		&c_ref_v_centerspeed,
		&c_ref_cl_forwardspeed,
		&c_ref_lookspring,
		&c_ref_chase_active,
	};

	if (idx < 0 || idx >= CTEST_VIEW_CVARS)
		return NULL;
	return oracle ? ref[idx] : plain[idx];
}

int ctest_view_cvar_count (void)
{
	return CTEST_VIEW_CVARS;
}

/* Writes one table into BOTH sides. Index order is ctest_view_cvar's. */
void ctest_view_set_cvars (const float *v)
{
	int i;

	for (i = 0; i < CTEST_VIEW_CVARS; i++)
	{
		ctest_view_cvar (i, 0)->value = v[i];
		ctest_view_cvar (i, 1)->value = v[i];
	}
}

/* Post-registration observation for the V_Init test. `found` proves the object
 * really entered that side's registry -- the oracle's is Quake/cvar.c's list,
 * the plain side's is quake-capi's, two independent implementations. */
void ctest_view_cvar_info (int idx, int oracle, ctest_view_cvar_info_t *out)
{
	const cvar_t *var = ctest_view_cvar (idx, oracle);
	const cvar_t *hit;

	memset (out, 0, sizeof (*out));
	if (!var)
		return;

	hit = oracle ? c_ref_Cvar_FindVar (var->name) : Cvar_FindVar (var->name);
	out->found = (hit == var);
	out->flags = var->flags;
	out->value = var->value;
	if (var->name)
		q_strlcpy (out->name, var->name, sizeof (out->name));
	if (var->string)
		q_strlcpy (out->string, var->string, sizeof (out->string));
}

/* Which handler cmd.c's table holds for `name`: 0 unregistered, 1/2/3 the
 * plain (Rust-routed) V_cshift_f / V_BonusFlash_f / V_StartPitchDrift, 4/5/6
 * their c_ref_ twins, 7 something else. Lets view_differential.rs prove
 * quake_rs_v_init registered the right names AND the right functions. */
int ctest_view_cmd_handler (const char *name)
{
	const cmd_function_t *cmd = Cmd_FindCommand (name);

	if (!cmd)
		return 0;
	if (cmd->function == V_cshift_f)
		return 1;
	if (cmd->function == V_BonusFlash_f)
		return 2;
	if (cmd->function == V_StartPitchDrift)
		return 3;
	if (cmd->function == c_ref_V_cshift_f)
		return 4;
	if (cmd->function == c_ref_V_BonusFlash_f)
		return 5;
	if (cmd->function == c_ref_V_StartPitchDrift)
		return 6;
	return 7;
}

static void ctest_view_apply_side (client_state_t *c, const ctest_view_state_t *s)
{
	int i, j;

	c->entities = ctest_view_entities;
	c->num_entities = CTEST_VIEW_ENTS;
	c->max_edicts = CTEST_VIEW_ENTS;
	c->worldmodel = (qmodel_t *)ctest_world_model ();

	c->time = s->time;
	c->oldtime = s->oldtime;
	c->mtime[0] = s->mtime0;
	c->mtime[1] = s->mtime1;
	c->laststop = s->laststop;

	ctest_view_vcopy (c->velocity, s->velocity);
	ctest_view_vcopy (c->viewangles, s->viewangles);
	ctest_view_vcopy (c->punchangle, s->punchangle);

	memset (c->stats, 0, sizeof (c->stats));
	memset (c->statsf, 0, sizeof (c->statsf));
	c->stats[STAT_HEALTH] = s->stat_health;
	c->stats[STAT_WEAPON] = s->stat_weapon;
	c->stats[STAT_WEAPONFRAME] = s->stat_weaponframe;
	c->stats[STAT_VIEWHEIGHT] = s->stat_viewheight;
	c->statsf[STAT_IDEALPITCH] = s->statsf_idealpitch;

	c->items = s->items;
	c->intermission = s->intermission;
	c->maxclients = s->maxclients;
	c->viewentity = s->viewentity & (CTEST_VIEW_ENTS - 1);
	c->protocolflags = (unsigned)s->protocolflags;

	c->onground = s->onground ? true : false;
	c->inwater = s->inwater ? true : false;
	c->paused = s->paused ? true : false;
	c->nodrift = s->nodrift ? true : false;

	c->pitchvel = s->pitchvel;
	c->driftmove = s->driftmove;
	c->idealpitch = s->idealpitch;
	c->faceanimtime = s->faceanimtime;
	c->v_dmg_time = s->v_dmg_time;
	c->v_dmg_roll = s->v_dmg_roll;
	c->v_dmg_pitch = s->v_dmg_pitch;

	c->movemessages = s->movemessages;
	memset (c->movecmds, 0, sizeof (c->movecmds));
	c->movecmds[(s->movemessages - 1) & MOVECMDS_MASK].forwardmove = s->movecmd_forwardmove;

	for (i = 0; i < NUM_CSHIFTS; i++)
	{
		for (j = 0; j < 3; j++)
		{
			c->cshifts[i].destcolor[j] = s->cshifts_dest[i][j];
			c->prev_cshifts[i].destcolor[j] = s->prev_dest[i][j];
		}
		c->cshifts[i].percent = s->cshifts_pct[i];
		c->prev_cshifts[i].percent = s->prev_pct[i];
	}
	for (j = 0; j < 3; j++)
		c->cshift_empty.destcolor[j] = s->empty_dest[j];
	c->cshift_empty.percent = s->empty_pct;

	memset (c->model_precache, 0, sizeof (c->model_precache));
	for (i = 0; i < CTEST_VIEW_MODELS; i++)
		c->model_precache[i] = &ctest_view_models[i];

	memset (&c->viewent, 0, sizeof (c->viewent));
	ctest_view_vcopy (c->viewent.origin, s->viewent_origin);
	ctest_view_vcopy (c->viewent.angles, s->viewent_angles);
	c->viewent.frame = s->viewent_frame;
	c->viewent.lerp.prev_frame = s->viewent_prev_frame;
	c->viewent.lerp.snap_frames = s->viewent_snap_frames;
	c->viewent.lerp.frame_change_time = s->viewent_frame_change_time;
	c->viewent.lerp.frame_duration = s->viewent_frame_duration;
	c->viewent.lerp.frame_finish_time = s->viewent_frame_finish_time;
	c->viewent.model = (s->viewent_model_idx < 0) ? NULL : &ctest_view_models[s->viewent_model_idx & (CTEST_VIEW_MODELS - 1)];
	c->viewent.netstate.colormap = (byte)s->viewent_colormap;
}

/* Republishes every input both sides read. Called before EACH side's run, so
 * the shared entity array, the shared r_refdef and the shared render globals
 * are back at the fixture value whichever side ran last. */
void ctest_view_apply (const ctest_view_state_t *s)
{
	entity_t *ent;
	int		  i;

	memset (ctest_view_entities, 0, sizeof (ctest_view_entities));
	ent = &ctest_view_entities[s->viewentity & (CTEST_VIEW_ENTS - 1)];
	ctest_view_vcopy (ent->origin, s->ent_origin);
	ctest_view_vcopy (ent->angles, s->ent_angles);
	ctest_view_vcopy (ent->msg_angles[0], s->ent_msg_angles);

	ctest_view_apply_side (&cl, s);
	ctest_view_apply_side (&c_ref_cl, s);

	cls.demoplayback = s->demoplayback ? true : false;
	cls.demoseeking = s->demoseeking ? true : false;
	c_ref_cls.demoplayback = s->demoplayback ? true : false;
	c_ref_cls.demoseeking = s->demoseeking ? true : false;

	for (i = 0; i < 2; i++)
	{
		ctest_view_vcopy (v_punchangles[i], s->punchangles[i]);
		ctest_view_vcopy (c_ref_v_punchangles[i], s->punchangles[i]);
		v_punchangles_times[i] = s->punchangles_times[i];
		c_ref_v_punchangles_times[i] = s->punchangles_times[i];
	}
	for (i = 0; i < 4; i++)
	{
		v_blend[i] = (uint8_t)s->blend[i];
		c_ref_v_blend[i] = (uint8_t)s->blend[i];
	}

	/* Single shared symbols: host.c, console.c, gl_screen.c and gl_rmain.c are
	 * not oracle sources, so the prelude never renames these and stubs.c
	 * (:7218-7234) provides one copy each for both sides. */
	host_frametime = s->host_frametime;
	noclip_anglehack = s->noclip_anglehack ? true : false;
	con_forcedup = s->con_forcedup ? true : false;
	scr_viewsize.value = s->scr_viewsize;
	r_trace_line_cache_counter = s->trace_line_cache_counter;
	render_scale = s->render_scale;
	render_warp = s->render_warp ? true : false;

	needs_relink = s->needs_relink ? true : false;
	c_ref_needs_relink = s->needs_relink ? true : false;

	ctest_view_vcopy (r_refdef.vieworg, s->vieworg);
	ctest_view_vcopy (r_refdef.viewangles, s->refdef_viewangles);
}

static void ctest_view_snapshot_side (ctest_view_snap_t *out, const client_state_t *c, const uint8_t *blend, const vec3_t *punch, const double *ptimes)
{
	const entity_t *ent = &c->entities[c->viewentity & (CTEST_VIEW_ENTS - 1)];
	int				i, j;

	memset (out, 0, sizeof (*out));

	ctest_view_vcopy (out->vieworg, r_refdef.vieworg);
	ctest_view_vcopy (out->viewangles, r_refdef.viewangles);

	ctest_view_vcopy (out->cl_viewangles, c->viewangles);
	ctest_view_vcopy (out->cl_punchangle, c->punchangle);
	out->pitchvel = c->pitchvel;
	out->driftmove = c->driftmove;
	out->idealpitch = c->idealpitch;
	out->faceanimtime = c->faceanimtime;
	out->nodrift = c->nodrift ? 1 : 0;
	out->laststop = c->laststop;
	out->v_dmg_time = c->v_dmg_time;
	out->v_dmg_roll = c->v_dmg_roll;
	out->v_dmg_pitch = c->v_dmg_pitch;

	for (i = 0; i < NUM_CSHIFTS; i++)
	{
		for (j = 0; j < 3; j++)
		{
			out->cshifts_dest[i][j] = c->cshifts[i].destcolor[j];
			out->prev_dest[i][j] = c->prev_cshifts[i].destcolor[j];
		}
		out->cshifts_pct[i] = c->cshifts[i].percent;
		out->prev_pct[i] = c->prev_cshifts[i].percent;
	}
	for (j = 0; j < 3; j++)
		out->empty_dest[j] = c->cshift_empty.destcolor[j];
	out->empty_pct = c->cshift_empty.percent;

	ctest_view_vcopy (out->viewent_origin, c->viewent.origin);
	ctest_view_vcopy (out->viewent_angles, c->viewent.angles);
	out->viewent_frame = c->viewent.frame;
	out->viewent_prev_frame = c->viewent.lerp.prev_frame;
	out->viewent_snap_frames = c->viewent.lerp.snap_frames;
	out->viewent_frame_change_time = c->viewent.lerp.frame_change_time;
	out->viewent_frame_duration = c->viewent.lerp.frame_duration;
	out->viewent_frame_finish_time = c->viewent.lerp.frame_finish_time;
	out->viewent_model_idx = -1;
	for (i = 0; i < CTEST_VIEW_MODELS; i++)
		if (c->viewent.model == &ctest_view_models[i])
			out->viewent_model_idx = i;
	out->viewent_colormap = (int)c->viewent.netstate.colormap;

	ctest_view_vcopy (out->ent_origin, ent->origin);
	ctest_view_vcopy (out->ent_angles, ent->angles);

	for (i = 0; i < 4; i++)
		out->blend[i] = (int)blend[i];
	for (i = 0; i < 2; i++)
	{
		ctest_view_vcopy (out->punchangles[i], punch[i]);
		out->punchangles_times[i] = ptimes[i];
	}

	out->trace_line_cache_counter = r_trace_line_cache_counter;
	out->render_scale = render_scale;
	out->render_warp = render_warp ? 1 : 0;
}

/* oracle != 0 reads the c_ref_ half, 0 the plain half. */
void ctest_view_snapshot (ctest_view_snap_t *out, int oracle)
{
	if (oracle)
		ctest_view_snapshot_side (out, &c_ref_cl, c_ref_v_blend, (const vec3_t *)c_ref_v_punchangles, c_ref_v_punchangles_times);
	else
		ctest_view_snapshot_side (out, &cl, v_blend, (const vec3_t *)v_punchangles, v_punchangles_times);
}

/* ---------------------------------------------------------------------------
 * Function-local-static lockstep.
 *
 * V_CalcRefdef carries `static float oldz` and `static vec3_t punch`
 * (view.c:716-717); the Rust port carries the module statics REFDEF_OLDZ and
 * REFDEF_PUNCH. Neither can be written from outside, so both are driven to the
 * same known value by a deterministic prologue instead:
 *
 *   oldz  -- cl.onground == false takes V_CalcRefdef's `else` branch, whose
 *            only statement is `oldz = ent->origin[2]`.
 *   punch -- with v_gunkick == 2, v_punchangles[0] = {0,0,0},
 *            v_punchangles[1] = {1e9,1e9,1e9}, an interval of 0.1 and
 *            host_frametime 0.1, delta is -1e9 on every component, so
 *            punch[i] = q_max (punch[i] - 1e9, 0) == 0 whatever it held. A
 *            component already at 0 is skipped by the
 *            `punch[i] != v_punchangles[0][i]` guard and stays 0 either way,
 *            so the prologue is idempotent.
 *
 * CalcGunAngle's own oldyaw/oldpitch need no priming: view.c:562 computes
 * `yaw = angledelta (yaw - r_refdef.viewangles[YAW]) * 0.4` after yaw was set
 * from r_refdef.viewangles[YAW], so the argument is identically 0 (same for
 * pitch), and with host_frametime >= 0 the move clamp cannot pull either
 * static off 0. Tests must keep host_frametime >= 0 for that to hold.
 */
static const float ctest_view_default_cvars[CTEST_VIEW_CVARS] = {
	0.0f,	0.0f,	0.0f,	/* scr_ofs{x,y,z} */
	200.0f, 2.0f,			/* cl_rollspeed, cl_rollangle */
	0.02f,	0.6f,	0.5f,	/* cl_bob, cl_bobcycle, cl_bobup */
	0.5f,	0.6f,	0.6f,	/* v_kick{time,roll,pitch} */
	2.0f,					/* v_gunkick: the lerped-kick branch */
	0.0f,					/* v_autopitch */
	2.0f,	0.5f,	1.0f,	/* v_i{yaw,roll,pitch}_cycle */
	0.3f,	0.1f,	0.3f,	/* v_i{yaw,roll,pitch}_level */
	0.0f,					/* v_idlescale */
	1.0f,	0.0f,			/* crosshair, crosshair_def */
	100.0f, 100.0f, 100.0f, /* gl_cshiftpercent, _contents, _damage */
	100.0f, 100.0f,			/* gl_cshiftpercent_bonus, _powerup */
	0.0f,					/* r_viewmodel_quake */
	0.15f,	500.0f,			/* v_centermove, v_centerspeed */
	200.0f, 0.0f,			/* cl_forwardspeed, lookspring */
	0.0f,					/* chase_active: keep Chase_UpdateForDrawing out */
};

void ctest_view_default_cvar_table (float *out)
{
	memcpy (out, ctest_view_default_cvars, sizeof (ctest_view_default_cvars));
}

static void ctest_view_prime (float oldz_seed)
{
	ctest_view_state_t s;

	memset (&s, 0, sizeof (s));
	s.time = 1.0;
	s.oldtime = 1.0;
	s.host_frametime = 0.1;
	s.viewentity = 1;
	s.maxclients = 2; /* > 1 skips V_CalcRefdef's scr_ofs* cheat-protection */
	s.ent_origin[2] = oldz_seed;
	s.viewent_model_idx = -1;
	s.stat_health = 100;
	s.punchangles[1][0] = 1e9f;
	s.punchangles[1][1] = 1e9f;
	s.punchangles[1][2] = 1e9f;
	s.punchangles_times[0] = 0.1;
	s.punchangles_times[1] = 0.0;
	s.onground = 0; /* the `else oldz = ent->origin[2]` branch */

	ctest_view_set_cvars (ctest_view_default_cvars);

	ctest_view_apply (&s);
	c_ref_V_CalcRefdef ();
	ctest_view_apply (&s);
	V_CalcRefdef ();
}

/* Rebuilds the shared world/entity fixture, restores the default cvar table on
 * both sides and drives both V_CalcRefdef static sets to (oldz_seed, {0,0,0}).
 * Call before EVERY side of EVERY comparison so the two runs are entered from
 * identical hidden state. */
void ctest_view_reset (float oldz_seed)
{
	ctest_world_reset (0, 2);

	memset (ctest_view_models, 0, sizeof (ctest_view_models));

	cl.entities = ctest_view_entities;
	cl.worldmodel = (qmodel_t *)ctest_world_model ();
	c_ref_cl.entities = ctest_view_entities;
	c_ref_cl.worldmodel = (qmodel_t *)ctest_world_model ();

	ctest_view_prime (oldz_seed);
}

/* V_ParseDamage's readers consume different state on the two sides: the
 * oracle's c_ref_MSG_ReadByte / c_ref_MSG_ReadCoord walk c_ref_net_message and
 * c_ref_msg_readcount (Quake/net_msg.c), the Rust port's plain MSG_* walk the
 * plain twins sv_user_ref.c owns (quake-capi::net). Both buffers get the same
 * bytes and both cursors are rewound here. */
#undef msg_readcount
#undef msg_badread
extern int		msg_readcount;
extern qboolean msg_badread;

void ctest_view_load_message (const unsigned char *data, int len)
{
	ctest_svuser_load_message (data, len);
	msg_readcount = 0;
	msg_badread = false;
	c_ref_msg_readcount = 0;
	c_ref_msg_badread = false;
}

/* V_cshift_f reads Cmd_Argc/Cmd_Argv, likewise two implementations over two
 * argv stores (Quake/cmd.c for the oracle, quake-capi::cmd for the port). */
#undef Cmd_TokenizeString
extern void Cmd_TokenizeString (const char *text);

void ctest_view_tokenize (const char *text)
{
	Cmd_TokenizeString (text);
	c_ref_Cmd_TokenizeString (text);
}

/* ---------------------------------------------------------------------------
 * ABI probes for the #[repr(C)] mirrors in rust/quake-capi/src/view.rs.
 *
 * entity_t / entlerp_t / lightcache_t / refdef_t have no ADR-011 mirror in
 * quake-types: abi_probe.c pins only sizeof/alignof entity_t and treats its
 * interior as opaque. rust/quake-ctest/tests/host_abi.rs is shared with the
 * T7.2b wave, so the field-level assertions live in view_differential.rs and
 * read their expected values from here.
 */
int ctest_view_abi (int idx)
{
	switch (idx)
	{
	case 0:
		return (int)sizeof (entity_t);
	case 1:
		return (int)offsetof (entity_t, origin);
	case 2:
		return (int)offsetof (entity_t, angles);
	case 3:
		return (int)offsetof (entity_t, msg_angles);
	case 4:
		return (int)offsetof (entity_t, model);
	case 5:
		return (int)offsetof (entity_t, frame);
	case 6:
		return (int)offsetof (entity_t, lerp);
	case 7:
		return (int)offsetof (entity_t, netstate);
	case 8:
		return (int)sizeof (entlerp_t);
	case 9:
		return (int)offsetof (entlerp_t, prev_frame);
	case 10:
		return (int)offsetof (entlerp_t, frame_change_time);
	case 11:
		return (int)offsetof (entlerp_t, frame_duration);
	case 12:
		return (int)offsetof (entlerp_t, frame_finish_time);
	case 13:
		return (int)offsetof (entlerp_t, snap_frames);
	case 14:
		return (int)sizeof (refdef_t);
	case 15:
		return (int)offsetof (refdef_t, vieworg);
	case 16:
		return (int)offsetof (refdef_t, viewangles);
	case 17:
		return (int)sizeof (vrect_t);
	case 18:
		return (int)sizeof (lightcache_t);
	case 19:
		return (int)offsetof (entity_t, lightcache);
	case 20:
		return (int)offsetof (entity_state_t, colormap);
	case 21:
		return (int)sizeof (cshift_t);
	case 22:
		return (int)sizeof (ctest_view_state_t);
	case 23:
		return (int)sizeof (ctest_view_snap_t);
	case 24:
		return (int)sizeof (ctest_view_cvar_info_t);
	case 25:
		return (int)offsetof (entity_t, msg_origins);
	default:
		return -1;
	}
}
