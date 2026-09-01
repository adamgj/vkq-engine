/* Phase 7 M7 oracle fixture TU for Quake/cl_parse.c (T7.3).
 *
 * c_ref_prelude.h is force-included (build.rs) and already includes the real
 * Quake/client.h, so client_state_t, client_static_t, entity_t, lightstyle_t
 * and scoreboard_t are the engine's own declarations here. Quake/cl_parse.c
 * and Quake/cl_main.c are both oracle sources, so their entry points are
 * reachable as c_ref_<name>.
 *
 * The three roles cl_tent_ref.c / cl_input_ref.c / sv_user_ref.c play for
 * their waves:
 *
 *  1. Define the PLAIN (Rust-reading) twins of everything Quake/cl_parse_glue.c
 *     owns -- svc_strings[128] -- plus the two cl_main.c objects the Rust port
 *     reads that have no plain twin anywhere: cl_lightstyle[] and the
 *     cl_shownet cvar. cl_parse_glue.c is gated `#ifdef USE_RUST_HOST` and is
 *     not in build.rs's C_SOURCES, and cl_main.c is an oracle source whose
 *     every symbol is renamed, so without this file none of the three has a
 *     definition under its plain name. The authoritative list came from a
 *     link probe: `cargo test -p quake-ctest --no-run` reported exactly 24
 *     unresolved externals -- those three objects and the 21 ClParse_Glue_*
 *     trampolines below.
 *  2. Re-implement ClParse_Raise, the 21 ADR-009 trampolines and the five
 *     re-raising entry points, mirroring Quake/cl_parse_glue.c's bodies
 *     exactly.
 *  3. Provide the fixture seeders and read-backs. Nothing in this link ever
 *     runs CL_Init or CL_ParseServerInfo to completion, so cl.entities,
 *     cl.scores, cl.static_entities and the precache tables are NULL/0 from
 *     static init on BOTH sides -- the "both sides degenerate identically"
 *     shape a bit-exact differential silently accepts, and with cl.entities
 *     NULL every CL_EntityNum would take the same early Mem_Alloc path and
 *     the whole port would look correct while doing nothing. Every seeder
 *     below therefore writes the c_ref_* copy and the plain copy in the same
 *     call, and ctest_clparse_reset publishes real, non-degenerate values.
 *
 * Callee selection (the rule sv_send_ref.c:1051 records): the ClParse_Glue_*
 * bodies below call the SAME unrenamed helpers the real glue file calls
 * wherever that helper is a single shared stubs.c symbol in this link, which
 * covers 18 of the 21 -- SCR_BeginLoadingPlaque, Key_ClearStates, R_NewMap,
 * R_CheckEfrags, Mod_ForName, R_AddEfrags, R_TranslatePlayerSkin,
 * R_TranslateNewPlayerSkin, PScript_*, Sky_LoadSkyBox, COM_Effectinfo_Enumerate
 * and CL_ParseTEnt (cl_tent_ref.c's plain definition). Those 18 are literally
 * the same function object the oracle's own c_ref_CL_ParseServerMessage calls,
 * so the two sides cannot diverge inside them.
 *
 * The remaining three -- CL_SignonReply, CL_ClearState and
 * CL_GenerateRandomParticlePrecache -- live in cl_main.c and exist only as
 * c_ref_*. They mutate `cl` / `cls`, and the two sides own different copies of
 * those (ADR-007: the cl/cls row closed in T7.4, so the port reads quake-capi's
 * Rust-owned pair -- stubs.c only externs it -- while the oracle reads
 * cl_main.c's c_ref_ pair). Calling the
 * oracle's copy from the port's trampoline would write the oracle's state from
 * the Rust run and make every comparison order-dependent, so each gets a
 * hand-transcribed plain twin below, exactly as cl_tent_ref.c did for
 * CL_AllocDlight. Their transcriptions are line-for-line, including the calls
 * that reach abort stubs.
 *
 * HARNESS-ONLY RAISE HAZARD, stated deliberately: stubs.c:48-61 makes
 * Sys_Error longjmp when armed, so driving any abort stub through the Rust
 * port longjmps across Rust frames. That is safe only because every driver
 * below enters through Host_Guard, whose setjmp sits in a pure C frame outside
 * the Rust call. It is a property of the harness, not of the port. It is also
 * what makes most of this suite work: an abort stub reached from the oracle
 * (direct call) and from the port (through a ClParse_Glue_* guard, then
 * CLPARSE_RAISE_GUARD, then ClParse_Raise's Host_Reraise) must land on the
 * outer guard with the SAME status and the SAME message -- which is precisely
 * the ADR-009 round trip under test.
 *
 * WHAT THE ABORT STUBS COST, stated so it is not mistaken for coverage:
 * CL_ParseServerInfo (cl_parse.c:922) calls CL_ClearState at :945, whose
 * CL_FreeState reaches stubs.c's PR_ClearProgs abort stub. Both sides stop
 * there, identically, so everything CL_ParseServerInfo does after :945 --
 * the protocol/pext negotiation, the gamedir switch, the model and sound
 * precache loops -- is NOT differentially covered by this suite. Raising
 * that ceiling means turning shared abort stubs into no-ops, which would
 * delete the "reached a module that is not an oracle source" tripwire for
 * every other suite in the harness; it is deliberately not done here.
 */

#include <string.h>

/* Host_Guard/Host_Reraise live in stubs.c and are not declared by any header
 * the prelude pulls in (the real engine declares them via host.h), same as
 * cl_tent_ref.c / sv_user_ref.c / cl_input_ref.c. */
extern int	Host_Guard (void (*fn) (void *), void *arg);
extern void Host_Reraise (int guard_result);

/* quake-capi/src/cl_parse.rs's five status cores. cl_parse_glue.c gets these
 * prototypes from the generated quake_rs.h, which this link has no counterpart
 * for. */
extern int quake_rs_cl_entity_num (int num, void **out);
extern int quake_rs_cl_parse_local_sound (int *detail);
extern int quake_rs_cl_new_translation (int slot, int *detail);
extern int quake_rs_cl_register_particles (int *detail);
extern int quake_rs_cl_parse_server_message (int *a, int *b, const char **s);

/* --------------------------------------------------------------------------
 * Plain (Rust-reading) storage this wave owns.
 *
 * The prelude's rename macros are live in this TU and would rewrite every
 * definition below to c_ref_*, colliding with the real oracle objects compiled
 * from cl_parse.c / cl_main.c (LNK2005), so each name is #undef'd first. Once
 * #undef'd the bare name means the PLAIN copy for the rest of the file;
 * oracle access always spells c_ref_* by hand. The #undef only affects text
 * after it, so the prelude's own (already renamed) declarations of these same
 * names stay valid -- that is what puts c_ref_cl_lightstyle et al. in scope
 * without a hand-written extern.
 */

/* cl_parse.c:30-88, moved into cl_parse_glue.c:56-116 by the port. Copied
   verbatim from the glue: the Illegible-server-message raise formats
   svc_strings[lastcmd], and the Rust core hands ClParse_Raise a pointer into
   this table, so the two copies must agree string for string. */
#undef svc_strings
const char *svc_strings[128] = {
	"svc_bad", "svc_nop", "svc_disconnect", "svc_updatestat",
	"svc_version",	 // [long] server version
	"svc_setview",	 // [short] entity number
	"svc_sound",	 // <see code>
	"svc_time",		 // [float] server time
	"svc_print",	 // [string] null terminated string
	"svc_stufftext", // [string] stuffed into client's console buffer
					 // the string should be \n terminated
	"svc_setangle",	 // [vec3] set the view angle to this absolute value

	"svc_serverinfo",	// [long] version
						// [string] signon string
						// [string]..[0]model cache [string]...[0]sounds cache
						// [string]..[0]item cache
	"svc_lightstyle",	// [byte] [string]
	"svc_updatename",	// [byte] [string]
	"svc_updatefrags",	// [byte] [short]
	"svc_clientdata",	// <shortbits + data>
	"svc_stopsound",	// <see code>
	"svc_updatecolors", // [byte] [byte]
	"svc_particle",		// [vec3] <variable>
	"svc_damage",		// [byte] impact [byte] blood [vec3] from

	"svc_spawnstatic",
	/*"OBSOLETE svc_spawnbinary"*/ "21 svc_spawnstatic_fte", "svc_spawnbaseline",

	"svc_temp_entity", // <variable>
	"svc_setpause", "svc_signonnum", "svc_centerprint", "svc_killedmonster", "svc_foundsecret", "svc_spawnstaticsound", "svc_intermission",
	"svc_finale",  // [string] music [string] text
	"svc_cdtrack", // [byte] track [byte] looptrack
	"svc_sellscreen", "svc_cutscene",
	// johnfitz -- new server messages
	"svc_showpic_dp",			  // 35
	"svc_hidepic_dp",			  // 36
	"svc_skybox_fitz",			  // 37					// [string] skyname
	"38",						  // 38
	"39",						  // 39
	"svc_bf_fitz",				  // 40						// no data
	"svc_fog_fitz",				  // 41					// [byte] density [byte] red [byte] green [byte] blue [float] time
	"svc_spawnbaseline2_fitz",	  // 42			// support for large modelindex, large framenum, alpha, using flags
	"svc_spawnstatic2_fitz",	  // 43			// support for large modelindex, large framenum, alpha, using flags
	"svc_spawnstaticsound2_fitz", //	44		// [coord3] [short] samp [byte] vol [byte] aten
								  // johnfitz

	// 2021 RE-RELEASE:
	"svc_setviews",		  // 45
	"svc_updateping",	  // 46
	"svc_updatesocial",	  // 47
	"svc_updateplinfo",	  // 48
	"svc_rawprint",		  // 49
	"svc_servervars",	  // 50
	"svc_seq",			  // 51
	"svc_achievement",	  // 52
	"svc_chat",			  // 53
	"svc_levelcompleted", // 54
	"svc_backtolobby",	  // 55
	"svc_localsound"	  // 56
};

/* cl_main.c:59 and cl_main.c:36. Both are T7.4 objects, so they keep their C
   storage in the shipping build; here the Rust port reads them through
   quake_c_sys::cl_parse, which needs the plain names. */
#undef cl_lightstyle
lightstyle_t cl_lightstyle[MAX_LIGHTSTYLES];

#undef cl_shownet
cvar_t cl_shownet = {"cl_shownet", "0", CVAR_NONE};

/* stubs.c owns the plain `cl` / `cls` (its DUPLICATE-SYMBOL HAZARD block,
   :2654-2765); cl_main.c defines c_ref_cl / c_ref_cls. Both sides are read
   here, so the rename must be off and both spellings written out. */
#undef cl
#undef cls
extern client_state_t  cl;
extern client_static_t cls;

/* sv_user_ref.c owns the plain net_message/msg_readcount/msg_badread trio and
   ctest_svuser_load_message, which seeds BOTH sides' buffers; this suite
   reuses it rather than defining a second one -- every stub object links into
   every test binary, so a symbol may be defined only once across all of
   them. */
#undef msg_readcount
#undef msg_badread
extern int		msg_readcount;
extern qboolean msg_badread;

/* Entry points this file defines, and the cl_main.c twins below. All are
   renamed by the prelude because cl_parse.c / cl_main.c define them. */
#undef CL_EntityNum
#undef CL_ParseLocalSound
#undef CL_NewTranslation
#undef CL_RegisterParticles
#undef CL_ParseServerMessage
#undef CL_ClearState
#undef CL_FreeState
#undef CL_SignonReply
#undef CL_GenerateRandomParticlePrecache

/* net_msg.c is an oracle source, so MSG_* and SZ_Clear are renamed here.
   quake-capi exports the readers and SZ_Clear under their exact C names, but
   the writers export as quake_rs_msg_write_* status cores that
   Quake/net_msg_glue.c wraps (ADR-009: SZ_GetSpace can Host_Error), so this
   link has no plain MSG_WriteByte at all and CL_SignonReply's twin calls the
   cores directly. The two bytes it writes cannot overflow an 8KB
   cls.message, and a non-zero status is raised loudly rather than dropped. */
#undef SZ_Clear
void	   SZ_Clear (sizebuf_t *buf);
/* CL_ParseServerMessage rewinds net_message itself; the four cores driven
   directly do not, so the suite needs an explicit per-side rewind. */
#undef MSG_BeginReading
void	   MSG_BeginReading (void);
void	   c_ref_MSG_BeginReading (void);
extern int quake_rs_msg_write_byte (sizebuf_t *sb, int v);
extern int quake_rs_msg_write_string (sizebuf_t *sb, const char *s);

static void ctest_clparse_write_byte (sizebuf_t *sb, int c)
{
	int r = quake_rs_msg_write_byte (sb, c);
	if (r)
		Sys_Error ("ctest: quake_rs_msg_write_byte status %i", r);
}

static void ctest_clparse_write_string (sizebuf_t *sb, const char *s)
{
	int r = quake_rs_msg_write_string (sb, s);
	if (r)
		Sys_Error ("ctest: quake_rs_msg_write_string status %i", r);
}

/* Their only declarations came from headers the prelude had already renamed,
   so after the #undef the plain spellings have none. */
entity_t *CL_EntityNum (int num);
/* Declared in no header -- without this prototype the call at
   ctest_clparse_invoke_entity_num truncates the returned pointer to int. */
entity_t *c_ref_CL_EntityNum (int num);
void	  CL_ParseLocalSound (void);
void	  c_ref_CL_ParseLocalSound (void);
void	  CL_NewTranslation (int slot);
void	  CL_RegisterParticles (void);
void	  CL_ParseServerMessage (void);
void	  CL_ClearState (void);
void	  CL_FreeState (void);
void	  CL_SignonReply (void);
int		  CL_GenerateRandomParticlePrecache (const char *pname);

/* --------------------------------------------------------------------------
 * cl_main.c twins, hand-transcribed.
 */

/* cl_main.c:96-120. Reaches stubs.c's PR_ClearProgs abort stub on its third
   statement; everything after it is transcribed anyway so that the twin stays
   line-for-line with the original if that stub ever becomes a no-op. */
void CL_FreeState (void)
{
	int i;
	for (i = 0; i < MAX_CL_STATS; i++)
		Mem_Free (cl.statss[i]);
	PR_ClearProgs (&cl.qcvm);
	// Free entity BLASes before freeing entities
	if (cl.entities)
	{
		for (i = 0; i < cl.max_edicts; i++)
			R_FreeEntityBLAS (&cl.entities[i]);
	}
	Mem_Free (cl.entities);
	for (i = 0; i < cl.num_statics; i++)
		R_FreeEntityBLAS (cl.static_entities[i]);
	for (i = 0; i < cl.num_statics; i += 64)
		Mem_Free (cl.static_entities[i]);
	Mem_Free (cl.static_entities);
	Mem_Free (cl.scores);
	for (i = 0; i < MAX_PARTICLETYPES; ++i)
		Mem_Free (cl.particle_precache[i].name);
	for (i = 0; i < cl.num_efragallocs; ++i)
		Mem_Free (cl.efrag_allocs[i]);
	Mem_Free (cl.efrag_allocs);
	memset (&cl, 0, sizeof (cl));
}

/* cl_main.c:129-155. `sv` is Rust-owned since T6.6, so sv.active/sv.loadgame
   are the single shared objects here and the rename does not apply to them;
   cl_dlights / cl_lightstyle / cl_temp_entities / cl_beams are the plain
   copies (cl_tent_ref.c owns the last two, this file owns cl_lightstyle, and
   cl_dlights has no plain twin at all -- cl_tent_ref.c keeps its table
   file-private -- so that one memset is the single line of the original this
   twin cannot reproduce, and it is unreachable behind PR_ClearProgs anyway). */
void CL_ClearState (void)
{
	if (!sv.active)
		Host_ClearMemory ();

	// wipe the entire cl structure
	CL_FreeState ();

	SZ_Clear (&cls.message);

	// clear other arrays
	memset (cl_lightstyle, 0, sizeof (cl_lightstyle));

	// johnfitz -- cl_entities is now dynamically allocated
	cl.max_edicts = CLAMP (MIN_EDICTS, (int)max_edicts.value, MAX_EDICTS);
	cl.entities = (entity_t *)Mem_Alloc (cl.max_edicts * sizeof (entity_t));
	// johnfitz

	cl.viewent.netstate = nullentitystate;
	PScript_Shutdown ();
}

/* cl_main.c:257-292. Cases 1 and 2 read cl_name / cl_topcolor / cl_bottomcolor
   and call cl_main.c's file-static CL_SendInitialUserinfo, none of which has a
   plain twin in this link; a transcription that silently skipped them would be
   a divergence no comparison could see, so they abort loudly instead and the
   suite drives only signon 3, 4 and the default arm. Case 4's
   SCR_EndLoadingPlaque is a shared abort stub, so that arm stops on both sides
   with the same message. */
void CL_SignonReply (void)
{
	Con_DPrintf ("CL_SignonReply: %i\n", cls.signon);

	switch (cls.signon)
	{
	case 1:
	case 2:
		Sys_Error ("ctest: CL_SignonReply case %i needs cl_main.c's cvars", cls.signon);
		break;

	case 3:
		ctest_clparse_write_byte (&cls.message, clc_stringcmd);
		ctest_clparse_write_string (&cls.message, "begin");
		break;

	case 4:
		SCR_EndLoadingPlaque (); // allow normal screen updates
		break;
	}
}

/* cl_main.c:939-955. Small enough to transcribe exactly; PScript_FindParticleType
   is a shared abort stub, so the first previously-unseen name stops both sides
   identically. */
int CL_GenerateRandomParticlePrecache (const char *pname)
{ // for dpp7 compat
	size_t i;
	pname = va ("%s", pname);
	for (i = 1; i < MAX_PARTICLETYPES; i++)
	{
		if (!cl.particle_precache[i].name)
		{
			cl.particle_precache[i].name = q_strdup (pname);
			cl.particle_precache[i].index = PScript_FindParticleType (cl.particle_precache[i].name);
			return i;
		}
		if (!strcmp (cl.particle_precache[i].name, pname))
			return i;
	}
	return 0;
}

/* --------------------------------------------------------------------------
 * ADR-009 status codes, mirroring Quake/cl_parse_glue.c:123-156 verbatim.
 */

#define CLPARSE_OK					 0
#define CLPARSE_RAISE_GUARD			 1
#define CLPARSE_ERR_ENTITYNUM		 2
#define CLPARSE_ERR_SOUNDNUM		 3
#define CLPARSE_ERR_SOUNDENT		 4
#define CLPARSE_ERR_LOCALSOUND		 5
#define CLPARSE_ERR_PEXT1			 6
#define CLPARSE_ERR_PEXT2			 7
#define CLPARSE_ERR_VERSION			 8
#define CLPARSE_ERR_MAXCLIENTS		 9
#define CLPARSE_ERR_TOOMANYMODELS	 10
#define CLPARSE_ERR_TOOMANYSOUNDS	 11
#define CLPARSE_ERR_MODELNOTFOUND	 12
#define CLPARSE_ERR_BADMODNUM		 13
#define CLPARSE_ERR_TOOMANYSTATICS	 14
#define CLPARSE_ERR_BADMESSAGE		 15
#define CLPARSE_ERR_ILLEGIBLE		 16
#define CLPARSE_ERR_UPDATENAME		 17
#define CLPARSE_ERR_UPDATEFRAGS		 18
#define CLPARSE_ERR_UPDATECOLORS	 19
#define CLPARSE_ERR_SIGNON			 20
#define CLPARSE_ERR_DPPRECACHE		 21
#define CLPARSE_ERR_UPDATESTATBYTE	 22
#define CLPARSE_ERR_UPDATESTATSTRING 23
#define CLPARSE_ERR_UPDATESTATFLOAT	 24
#define CLPARSE_ERR_SPAWNSTATIC2	 25
#define CLPARSE_ERR_SPAWNBASELINE2	 26
#define CLPARSE_ERR_UPDATEENTITIES	 27
#define CLPARSE_ERR_CGAMEPACKET		 28
#define CLPARSE_ERR_CSQC_MISSING	 29
#define CLPARSE_ERR_VOICECHAT		 30
#define CLPARSE_END_DELTAINFO		 31
#define CLPARSE_END_UF_UNUSED1		 32
#define CLPARSE_END_DISCONNECTED	 33

FUNC_NORETURN static void ClParse_Raise (int status, int a, int b, const char *s)
{
	switch (status)
	{
	case CLPARSE_RAISE_GUARD:
		Host_Reraise (a);
		Sys_Error ("ClParse_Raise: Host_Reraise returned");
	case CLPARSE_ERR_ENTITYNUM:
		Host_Error ("CL_EntityNum: %i is an invalid number", a);
	case CLPARSE_ERR_SOUNDNUM:
		Host_Error ("CL_ParseStartSoundPacket: %i > MAX_SOUNDS", a);
	case CLPARSE_ERR_SOUNDENT:
		Host_Error ("CL_ParseStartSoundPacket: ent = %i", a);
	case CLPARSE_ERR_LOCALSOUND:
		Host_Error ("CL_ParseLocalSound: %i > MAX_SOUNDS", a);
	case CLPARSE_ERR_PEXT1:
		Host_Error ("Server returned FTE1 protocol extensions that are not supported (%#x)", (unsigned int)a);
	case CLPARSE_ERR_PEXT2:
		Host_Error ("Server returned FTE2 protocol extensions that are not supported (%#x)", (unsigned int)a);
	case CLPARSE_ERR_VERSION:
		Host_Error ("Server returned version %i, not %i or %i or %i", a, PROTOCOL_NETQUAKE, PROTOCOL_FITZQUAKE, PROTOCOL_RMQ);
	case CLPARSE_ERR_MAXCLIENTS:
		Host_Error ("Bad maxclients (%u) from server", (unsigned int)a);
	case CLPARSE_ERR_TOOMANYMODELS:
		Host_Error ("Server sent too many model precaches");
	case CLPARSE_ERR_TOOMANYSOUNDS:
		Host_Error ("Server sent too many sound precaches");
	case CLPARSE_ERR_MODELNOTFOUND:
		Host_Error ("Model %s not found", s);
	case CLPARSE_ERR_BADMODNUM:
		Host_Error ("CL_ParseModel: bad modnum");
	case CLPARSE_ERR_TOOMANYSTATICS:
		Host_Error ("Too many static entities");
	case CLPARSE_ERR_BADMESSAGE:
		Host_Error ("CL_ParseServerMessage: Bad server message");
	case CLPARSE_ERR_ILLEGIBLE:
		Host_Error ("Illegible server message %d, previous was %s", a, s);
	case CLPARSE_ERR_UPDATENAME:
		Host_Error ("CL_ParseServerMessage: svc_updatename > MAX_SCOREBOARD");
	case CLPARSE_ERR_UPDATEFRAGS:
		Host_Error ("CL_ParseServerMessage: svc_updatefrags > MAX_SCOREBOARD");
	case CLPARSE_ERR_UPDATECOLORS:
		Host_Error ("CL_ParseServerMessage: svc_updatecolors > MAX_SCOREBOARD");
	case CLPARSE_ERR_SIGNON:
		Host_Error ("Received signon %i when at %i", a, b);
	case CLPARSE_ERR_DPPRECACHE:
		Host_Error ("Received svcdp_precache but extension not active");
	case CLPARSE_ERR_UPDATESTATBYTE:
		Host_Error ("Received svcdp_updatestatbyte but extension not active");
	case CLPARSE_ERR_UPDATESTATSTRING:
		Host_Error ("Received svcfte_updatestatstring but extension not active");
	case CLPARSE_ERR_UPDATESTATFLOAT:
		Host_Error ("Received svcfte_updatestatfloat but extension not active");
	case CLPARSE_ERR_SPAWNSTATIC2:
		Host_Error ("Received svcfte_spawnstatic2 but extension not active");
	case CLPARSE_ERR_SPAWNBASELINE2:
		Host_Error ("Received svcfte_spawnbaseline2 but extension not active");
	case CLPARSE_ERR_UPDATEENTITIES:
		Host_Error ("Received svcfte_updateentities but extension not active");
	case CLPARSE_ERR_CGAMEPACKET:
		Host_Error ("Received svcfte_cgamepacket but extension not active");
	case CLPARSE_ERR_CSQC_MISSING:
		Host_Error ("CSQC_Parse_Event: Missing or incompatible CSQC\n");
	case CLPARSE_ERR_VOICECHAT:
		Host_Error ("Received svcfte_voicechat but extension not active");
	case CLPARSE_END_DELTAINFO:
		Host_EndGame ("unsupported entity delta info\n");
	case CLPARSE_END_UF_UNUSED1:
		Host_EndGame ("UF_UNUSED1 bit\n");
	case CLPARSE_END_DISCONNECTED:
		Host_EndGame ("Server disconnected\n");
	default:
		Sys_Error ("ClParse_Raise: unknown status %i", status);
	}
}

/* --------------------------------------------------------------------------
 * Guarded callbacks, mirroring Quake/cl_parse_glue.c:236-575 exactly.
 */

static void ClParse_InvokeSignonReply (void *p)
{
	(void)p;
	CL_SignonReply ();
}

int ClParse_Glue_SignonReply (void)
{
	return Host_Guard (ClParse_InvokeSignonReply, NULL);
}

static void ClParse_InvokeClearState (void *p)
{
	(void)p;
	CL_ClearState ();
}

int ClParse_Glue_ClearState (void)
{
	return Host_Guard (ClParse_InvokeClearState, NULL);
}

static void ClParse_InvokeKeyClearStates (void *p)
{
	(void)p;
	Key_ClearStates ();
}

int ClParse_Glue_KeyClearStates (void)
{
	return Host_Guard (ClParse_InvokeKeyClearStates, NULL);
}

static void ClParse_InvokeBeginLoadingPlaque (void *p)
{
	(void)p;
	SCR_BeginLoadingPlaque ();
}

int ClParse_Glue_BeginLoadingPlaque (void)
{
	return Host_Guard (ClParse_InvokeBeginLoadingPlaque, NULL);
}

static void ClParse_InvokeNewMap (void *p)
{
	(void)p;
	R_NewMap ();
}

int ClParse_Glue_NewMap (void)
{
	return Host_Guard (ClParse_InvokeNewMap, NULL);
}

static void ClParse_InvokeCheckEfrags (void *p)
{
	(void)p;
	R_CheckEfrags ();
}

int ClParse_Glue_CheckEfrags (void)
{
	return Host_Guard (ClParse_InvokeCheckEfrags, NULL);
}

/* cl_tent_ref.c:284 defines the plain CL_ParseTEnt for this link -- the same
   Host_Reraise wrapper cl_tent_glue.c ships -- so this trampoline drives the
   Rust cl_tent port, while the oracle's c_ref_CL_ParseServerMessage calls
   c_ref_CL_ParseTEnt. That pair is the subject of cl_tent_differential.rs; the
   only opcode that reaches it here (svc_temp_entity) is driven with payloads
   whose two implementations that suite already proved equal. */
/* cl_tent.c is an oracle source, so the prelude would rewrite this call site
   to c_ref_CL_ParseTEnt -- the C oracle, reading the oracle's own
   c_ref_msg_readcount -- and the Rust arm of svc_temp_entity would silently
   test nothing. The rename must be off for the plain definition to win. */
#undef CL_ParseTEnt
void CL_ParseTEnt (void);

static void ClParse_InvokeParseTEnt (void *p)
{
	(void)p;
	CL_ParseTEnt ();
}

int ClParse_Glue_ParseTEnt (void)
{
	return Host_Guard (ClParse_InvokeParseTEnt, NULL);
}

static void ClParse_InvokeEffectinfoEnumerate (void *p)
{
	(void)p;
	PScript_FindParticleType ("effectinfo."); // make sure this is implicitly loaded.
	COM_Effectinfo_Enumerate (CL_GenerateRandomParticlePrecache);
}

int ClParse_Glue_EffectinfoEnumerate (void)
{
	return Host_Guard (ClParse_InvokeEffectinfoEnumerate, NULL);
}

static void ClParse_InvokeCsqcParseEvent (void *p)
{
	(void)p;
	PR_SwitchQCVM (&cl.qcvm);
	PR_ExecuteProgram (cl.qcvm.extfuncs.CSQC_Parse_Event);
	PR_SwitchQCVM (NULL);
}

int ClParse_Glue_CsqcParseEvent (void)
{
	return Host_Guard (ClParse_InvokeCsqcParseEvent, NULL);
}

static void ClParse_InvokeDebugNewEntity (void *p)
{
	unsigned int entnum = *(unsigned int *)p;
	qcvm_t		*old = qcvm;
	qcvm = NULL;
	PR_SwitchQCVM (&sv.qcvm);
	Con_DPrintf ("New entity %i(%s / %s) without reset\n", entnum, PR_GetString (EDICT_NUM (entnum)->v.classname), PR_GetString (EDICT_NUM (entnum)->v.model));
	PR_SwitchQCVM (old);
}

int ClParse_Glue_DebugNewEntity (unsigned int entnum)
{
	unsigned int n = entnum;
	return Host_Guard (ClParse_InvokeDebugNewEntity, &n);
}

static void ClParse_InvokeTranslateNewPlayerSkin (void *p)
{
	R_TranslateNewPlayerSkin (*(int *)p);
}

int ClParse_Glue_TranslateNewPlayerSkin (int playernum)
{
	int n = playernum;
	return Host_Guard (ClParse_InvokeTranslateNewPlayerSkin, &n);
}

static void ClParse_InvokeTranslatePlayerSkin (void *p)
{
	R_TranslatePlayerSkin (*(int *)p);
}

int ClParse_Glue_TranslatePlayerSkin (int playernum)
{
	int n = playernum;
	return Host_Guard (ClParse_InvokeTranslatePlayerSkin, &n);
}

static void ClParse_InvokeAddEfrags (void *p)
{
	R_AddEfrags ((entity_t *)p);
}

int ClParse_Glue_AddEfrags (void *ent)
{
	return Host_Guard (ClParse_InvokeAddEfrags, ent);
}

typedef struct
{
	const char *name;
	void	  **out;
} clparse_modforname_args_t;

static void ClParse_InvokeModForName (void *p)
{
	clparse_modforname_args_t *a = (clparse_modforname_args_t *)p;
	*a->out = Mod_ForName (a->name, false);
}

int ClParse_Glue_ModForName (const char *name, void **out)
{
	clparse_modforname_args_t args;
	args.name = name;
	args.out = out;
	*out = NULL;
	return Host_Guard (ClParse_InvokeModForName, &args);
}

static void ClParse_InvokeCbufAddText (void *p)
{
	Cbuf_AddText ((const char *)p);
}

int ClParse_Glue_CbufAddText (const char *text)
{
	return Host_Guard (ClParse_InvokeCbufAddText, (void *)(uintptr_t)text);
}

typedef struct
{
	const char *text;
	int			src;
	int		   *out;
} clparse_cmdexec_args_t;

static void ClParse_InvokeCmdExecuteString (void *p)
{
	clparse_cmdexec_args_t *a = (clparse_cmdexec_args_t *)p;
	*a->out = Cmd_ExecuteString (a->text, (cmd_source_t)a->src);
}

int ClParse_Glue_CmdExecuteString (const char *text, int src, int *out)
{
	clparse_cmdexec_args_t args;
	args.text = text;
	args.src = src;
	args.out = out;
	*out = 0;
	return Host_Guard (ClParse_InvokeCmdExecuteString, &args);
}

typedef struct
{
	const char *name;
	int		   *out;
} clparse_findptype_args_t;

static void ClParse_InvokeFindParticleType (void *p)
{
	clparse_findptype_args_t *a = (clparse_findptype_args_t *)p;
	*a->out = PScript_FindParticleType (a->name);
}

int ClParse_Glue_FindParticleType (const char *name, int *out)
{
	clparse_findptype_args_t args;
	args.name = name;
	args.out = out;
	*out = 0;
	return Host_Guard (ClParse_InvokeFindParticleType, &args);
}

static void ClParse_InvokeUpdateModelEffects (void *p)
{
	PScript_UpdateModelEffects ((qmodel_t *)p);
}

int ClParse_Glue_UpdateModelEffects (void *mod)
{
	return Host_Guard (ClParse_InvokeUpdateModelEffects, mod);
}

typedef struct
{
	const float *start;
	const float *end;
	int			 type;
	float		 timeinterval;
	int			 dlkey;
	void	   **tsk;
} clparse_ptrail_args_t;

static void ClParse_InvokeParticleTrail (void *p)
{
	clparse_ptrail_args_t *a = (clparse_ptrail_args_t *)p;
	PScript_ParticleTrail (
		(float *)(uintptr_t)a->start, (float *)(uintptr_t)a->end, a->type, a->timeinterval, a->dlkey, NULL, (struct trailstate_s **)a->tsk);
}

int ClParse_Glue_ParticleTrail (const float *start, const float *end, int type, float timeinterval, int dlkey, void **tsk)
{
	clparse_ptrail_args_t args;
	args.start = start;
	args.end = end;
	args.type = type;
	args.timeinterval = timeinterval;
	args.dlkey = dlkey;
	args.tsk = tsk;
	return Host_Guard (ClParse_InvokeParticleTrail, &args);
}

typedef struct
{
	const float *org;
	const float *dir;
	float		 count;
	int			 typenum;
	void	   **tsk;
} clparse_prunstate_args_t;

static void ClParse_InvokeRunParticleEffectState (void *p)
{
	clparse_prunstate_args_t *a = (clparse_prunstate_args_t *)p;
	PScript_RunParticleEffectState ((float *)(uintptr_t)a->org, (float *)(uintptr_t)a->dir, a->count, a->typenum, (struct trailstate_s **)a->tsk);
}

int ClParse_Glue_RunParticleEffectState (const float *org, const float *dir, float count, int typenum, void **tsk)
{
	clparse_prunstate_args_t args;
	args.org = org;
	args.dir = dir;
	args.count = count;
	args.typenum = typenum;
	args.tsk = tsk;
	return Host_Guard (ClParse_InvokeRunParticleEffectState, &args);
}

static void ClParse_InvokeLoadSkyBox (void *p)
{
	Sky_LoadSkyBox ((const char *)p);
}

int ClParse_Glue_LoadSkyBox (const char *name)
{
	return Host_Guard (ClParse_InvokeLoadSkyBox, (void *)(uintptr_t)name);
}

/* --------------------------------------------------------------------------
 * Re-raising public entry points, mirroring cl_parse_glue.c:580-637.
 */

entity_t *CL_EntityNum (int num)
{
	void *ent = NULL;
	int	  r = quake_rs_cl_entity_num (num, &ent);
	if (r != CLPARSE_OK)
		ClParse_Raise (r, num, 0, NULL);
	return (entity_t *)ent;
}

void CL_ParseLocalSound (void)
{
	int detail = 0;
	int r = quake_rs_cl_parse_local_sound (&detail);
	if (r != CLPARSE_OK)
		ClParse_Raise (r, detail, 0, NULL);
}

void CL_NewTranslation (int slot)
{
	int detail = 0;
	int r = quake_rs_cl_new_translation (slot, &detail);
	if (r != CLPARSE_OK)
		ClParse_Raise (r, detail, 0, NULL);
}

void CL_RegisterParticles (void)
{
	int detail = 0;
	int r = quake_rs_cl_register_particles (&detail);
	if (r != CLPARSE_OK)
		ClParse_Raise (r, detail, 0, NULL);
}

void CL_ParseServerMessage (void)
{
	int			a = 0, b = 0;
	const char *s = NULL;
	int			r = quake_rs_cl_parse_server_message (&a, &b, &s);
	if (r != CLPARSE_OK)
		ClParse_Raise (r, a, b, s);
}

/* ==========================================================================
 * Fixture.
 * ======================================================================== */

/* Shared model/sound handles. cl.model_precache[] and cl.sound_precache[] hold
   pointers the parser copies into entity_t::model and compares against NULL,
   and CL_ParseUpdate dereferences ->flags / ->type / ->numframes, so the
   handles must be real objects -- and the SAME objects on both sides, or the
   byte-image comparison of entity_t would fail on the pointer alone. */
#define CTEST_CLPARSE_MODELS 6
/* Every sound slot is seeded, not just a handful: CL_ParseLocalSound reads
   cl.sound_precache[sound_num]->name with no NULL check after its only guard
   (sound_num >= MAX_SOUNDS), so any unseeded slot the wire can name is an
   immediate crash rather than a comparable observation. */
#define CTEST_CLPARSE_SOUNDS MAX_SOUNDS
static qmodel_t ctest_clparse_models[CTEST_CLPARSE_MODELS];
static sfx_t	ctest_clparse_sounds[CTEST_CLPARSE_SOUNDS];

/* Two separate entity/score/static arrays, seeded identically, so an
   accidental write by one side shows up as a difference instead of
   propagating into the other side's view. */
#define CTEST_CLPARSE_ENTITIES 64
#define CTEST_CLPARSE_STATICS  64
static entity_t	   ctest_clparse_ents[CTEST_CLPARSE_ENTITIES];
static entity_t	   ctest_clparse_oracle_ents[CTEST_CLPARSE_ENTITIES];
static entity_t	  *ctest_clparse_statics[CTEST_CLPARSE_STATICS];
static entity_t	  *ctest_clparse_oracle_statics[CTEST_CLPARSE_STATICS];
static entity_t	   ctest_clparse_static_storage[CTEST_CLPARSE_STATICS];
static entity_t	   ctest_clparse_oracle_static_storage[CTEST_CLPARSE_STATICS];
static scoreboard_t ctest_clparse_scores[MAX_SCOREBOARD];
static scoreboard_t ctest_clparse_oracle_scores[MAX_SCOREBOARD];

static byte ctest_clparse_msgbuf[8192];
static byte ctest_clparse_oracle_msgbuf[8192];

static client_state_t *ctest_clparse_cl (int side)
{
	return side ? &c_ref_cl : &cl;
}

static client_static_t *ctest_clparse_cls (int side)
{
	return side ? &c_ref_cls : &cls;
}

/* --------------------------------------------------------------------------
 * Seeders. Every one writes BOTH sides in the same call.
 */

void ctest_clparse_set_shownet (float v)
{
	cl_shownet.value = c_ref_cl_shownet.value = v;
}

void ctest_clparse_set_protocol (int protocol, unsigned int flags, unsigned int pext1, unsigned int pext2)
{
	int i;
	for (i = 0; i < 2; i++)
	{
		client_state_t *c = ctest_clparse_cl (i);
		c->protocol = (unsigned int)protocol;
		c->protocolflags = flags;
		c->protocol_pext1 = pext1;
		c->protocol_pext2 = pext2;
	}
}

void ctest_clparse_set_time (double time, double oldtime, double mtime0, double mtime1)
{
	int i;
	for (i = 0; i < 2; i++)
	{
		client_state_t *c = ctest_clparse_cl (i);
		c->time = time;
		c->oldtime = oldtime;
		c->mtime[0] = mtime0;
		c->mtime[1] = mtime1;
	}
}

void ctest_clparse_set_counts (int maxclients, int viewentity, int num_entities, int num_statics)
{
	int i;
	for (i = 0; i < 2; i++)
	{
		client_state_t *c = ctest_clparse_cl (i);
		c->maxclients = maxclients;
		c->viewentity = viewentity;
		c->num_entities = num_entities;
		c->num_statics = num_statics;
	}
}

void ctest_clparse_set_conn (int state, int signon, int demoplayback, int demorecording)
{
	int i;
	for (i = 0; i < 2; i++)
	{
		client_static_t *s = ctest_clparse_cls (i);
		s->state = (cactive_t)state;
		s->signon = signon;
		s->demoplayback = demoplayback ? true : false;
		s->demorecording = demorecording ? true : false;
	}
}

void ctest_clparse_set_intermission (int intermission)
{
	int i;
	for (i = 0; i < 2; i++)
		ctest_clparse_cl (i)->intermission = intermission;
}

/* Publishes `count` shared handles into both precache tables (slot 0 stays
   NULL, exactly as the engine leaves it until CL_ParseServerInfo runs). */
void ctest_clparse_seed_precaches (int nummodels, int numsounds)
{
	int i, j;

	if (nummodels > CTEST_CLPARSE_MODELS)
		nummodels = CTEST_CLPARSE_MODELS;
	if (numsounds > CTEST_CLPARSE_SOUNDS)
		numsounds = CTEST_CLPARSE_SOUNDS;

	for (j = 0; j < 2; j++)
	{
		client_state_t *c = ctest_clparse_cl (j);
		memset (c->model_precache, 0, sizeof (c->model_precache));
		memset (c->sound_precache, 0, sizeof (c->sound_precache));
		for (i = 0; i < nummodels; i++)
			c->model_precache[i] = &ctest_clparse_models[i];
		for (i = 0; i < numsounds; i++)
			c->sound_precache[i] = &ctest_clparse_sounds[i];
		c->worldmodel = nummodels > 1 ? &ctest_clparse_models[1] : NULL;
	}
}

void ctest_clparse_set_model_info (int idx, int numframes, int flags, int type, int synctype)
{
	if (idx < 0 || idx >= CTEST_CLPARSE_MODELS)
		return;
	ctest_clparse_models[idx].numframes = numframes;
	ctest_clparse_models[idx].flags = flags;
	ctest_clparse_models[idx].type = (modtype_t)type;
	ctest_clparse_models[idx].synctype = (synctype_t)synctype;
}

/* Publishes a live entity array, score table and static-entity list into both
   copies of `cl`. Without this every CL_EntityNum would hit the
   cl.entities == NULL arm and the suite would compare two identical no-ops. */
void ctest_clparse_attach_arrays (int max_edicts)
{
	int i;

	if (max_edicts < 0)
		max_edicts = 0;
	if (max_edicts > CTEST_CLPARSE_ENTITIES)
		max_edicts = CTEST_CLPARSE_ENTITIES;

	memset (ctest_clparse_ents, 0, sizeof (ctest_clparse_ents));
	memset (ctest_clparse_oracle_ents, 0, sizeof (ctest_clparse_oracle_ents));
	memset (ctest_clparse_scores, 0, sizeof (ctest_clparse_scores));
	memset (ctest_clparse_oracle_scores, 0, sizeof (ctest_clparse_oracle_scores));
	memset (ctest_clparse_static_storage, 0, sizeof (ctest_clparse_static_storage));
	memset (ctest_clparse_oracle_static_storage, 0, sizeof (ctest_clparse_oracle_static_storage));

	for (i = 0; i < CTEST_CLPARSE_STATICS; i++)
	{
		ctest_clparse_statics[i] = &ctest_clparse_static_storage[i];
		ctest_clparse_oracle_statics[i] = &ctest_clparse_oracle_static_storage[i];
	}

	cl.entities = ctest_clparse_ents;
	c_ref_cl.entities = ctest_clparse_oracle_ents;
	cl.scores = ctest_clparse_scores;
	c_ref_cl.scores = ctest_clparse_oracle_scores;
	cl.static_entities = ctest_clparse_statics;
	c_ref_cl.static_entities = ctest_clparse_oracle_statics;
	cl.max_edicts = c_ref_cl.max_edicts = max_edicts;
	cl.max_static_entities = c_ref_cl.max_static_entities = CTEST_CLPARSE_STATICS;
}

/* sizebuf_t writing target for CL_SignonReply's `begin`. Separate storage per
   side; the suite compares cursize and the bytes. */
void ctest_clparse_attach_message (void)
{
	cls.message.data = ctest_clparse_msgbuf;
	cls.message.maxsize = (int)sizeof (ctest_clparse_msgbuf);
	cls.message.cursize = 0;
	cls.message.allowoverflow = false;
	cls.message.overflowed = false;

	c_ref_cls.message.data = ctest_clparse_oracle_msgbuf;
	c_ref_cls.message.maxsize = (int)sizeof (ctest_clparse_oracle_msgbuf);
	c_ref_cls.message.cursize = 0;
	c_ref_cls.message.allowoverflow = false;
	c_ref_cls.message.overflowed = false;
}

/* --------------------------------------------------------------------------
 * Read-backs.
 *
 * client_state_t is compared as a normalized byte image rather than through
 * field getters, which would silently miss whatever they forgot to list. The
 * normalization zeroes exactly the members whose VALUES are allowed to differ
 * between the sides -- the pointers into the two separate fixture arrays, the
 * heap strings, and the embedded qcvm -- and nothing else. Every zeroed member
 * has its own comparison below.
 */

static void ctest_clparse_normalize (client_state_t *c)
{
	int i;
	for (i = 0; i < MAX_CL_STATS; i++)
		c->statss[i] = NULL;
	c->entities = NULL;
	c->static_entities = NULL;
	c->scores = NULL;
	c->efrag_allocs = NULL;
	c->free_efrags = NULL;
	for (i = 0; i < MAX_PARTICLETYPES; i++)
	{
		c->particle_precache[i].name = NULL;
		c->local_particle_precache[i].name = NULL;
	}
	memset (&c->qcvm, 0, sizeof (c->qcvm));
}

int ctest_clparse_cl_image_size (void)
{
	return (int)sizeof (client_state_t);
}

void ctest_clparse_get_cl_image (int side, void *out)
{
	memcpy (out, ctest_clparse_cl (side), sizeof (client_state_t));
	ctest_clparse_normalize ((client_state_t *)out);
}

int ctest_clparse_cls_image_size (void)
{
	return (int)sizeof (client_static_t);
}

void ctest_clparse_get_cls_image (int side, void *out)
{
	client_static_t *s = (client_static_t *)out;
	memcpy (out, ctest_clparse_cls (side), sizeof (client_static_t));
	s->message.data = NULL;
	s->demofile = NULL;
	s->netcon = NULL;
}

int ctest_clparse_entity_size (void)
{
	return (int)sizeof (entity_t);
}

void ctest_clparse_get_entity (int side, int idx, void *out)
{
	if (idx < 0 || idx >= CTEST_CLPARSE_ENTITIES)
		return;
	memcpy (out, side ? &ctest_clparse_oracle_ents[idx] : &ctest_clparse_ents[idx], sizeof (entity_t));
}

void ctest_clparse_get_static_entity (int side, int idx, void *out)
{
	if (idx < 0 || idx >= CTEST_CLPARSE_STATICS)
		return;
	memcpy (out, side ? &ctest_clparse_oracle_static_storage[idx] : &ctest_clparse_static_storage[idx], sizeof (entity_t));
}

void ctest_clparse_get_viewent (int side, void *out)
{
	memcpy (out, &ctest_clparse_cl (side)->viewent, sizeof (entity_t));
}

int ctest_clparse_score_size (void)
{
	return (int)sizeof (scoreboard_t);
}

void ctest_clparse_get_score (int side, int idx, void *out)
{
	if (idx < 0 || idx >= MAX_SCOREBOARD)
		return;
	memcpy (out, side ? &ctest_clparse_oracle_scores[idx] : &ctest_clparse_scores[idx], sizeof (scoreboard_t));
}

int ctest_clparse_lightstyle_size (void)
{
	return (int)sizeof (lightstyle_t);
}

void ctest_clparse_get_lightstyle (int side, int idx, void *out)
{
	if (idx < 0 || idx >= MAX_LIGHTSTYLES)
		return;
	memcpy (out, side ? &c_ref_cl_lightstyle[idx] : &cl_lightstyle[idx], sizeof (lightstyle_t));
}

/* cl.statss[] holds q_strdup'd strings; the pointers differ by construction,
   so the suite compares the text. Returns NULL for an empty slot. */
const char *ctest_clparse_get_statstring (int side, int idx)
{
	if (idx < 0 || idx >= MAX_CL_STATS)
		return NULL;
	return ctest_clparse_cl (side)->statss[idx];
}

const char *ctest_clparse_get_particle_name (int side, int idx)
{
	if (idx < 0 || idx >= MAX_PARTICLETYPES)
		return NULL;
	return ctest_clparse_cl (side)->particle_precache[idx].name;
}

int ctest_clparse_get_particle_index (int side, int idx)
{
	if (idx < 0 || idx >= MAX_PARTICLETYPES)
		return 0;
	return ctest_clparse_cl (side)->particle_precache[idx].index;
}

int ctest_clparse_get_message_size (int side)
{
	return ctest_clparse_cls (side)->message.cursize;
}

const unsigned char *ctest_clparse_get_message_data (int side)
{
	return side ? ctest_clparse_oracle_msgbuf : ctest_clparse_msgbuf;
}

int ctest_clparse_get_readcount (int side)
{
	return side ? c_ref_msg_readcount : msg_readcount;
}

int ctest_clparse_get_badread (int side)
{
	return (side ? c_ref_msg_badread : msg_badread) ? 1 : 0;
}

/* --------------------------------------------------------------------------
 * Drivers. Every entry point is entered through Host_Guard, so the setjmp that
 * catches an armed Sys_Error/Host_Error always sits in a pure C frame outside
 * the Rust call. The return value is the CTEST_GUARD_* status: 0 ok,
 * 1 Host_Error, 2 Sys_Error; the message is readable through stubs.c's
 * ctest_host_error_message() / ctest_sys_error_message().
 */

static void ctest_clparse_invoke_parse (void *p)
{
	if (*(int *)p)
		c_ref_CL_ParseServerMessage ();
	else
		CL_ParseServerMessage ();
}

int ctest_clparse_parse_server_message (int side)
{
	int s = side;
	return Host_Guard (ctest_clparse_invoke_parse, &s);
}

typedef struct
{
	int side;
	int num;
	int out;
} clparse_entnum_arg_t;

static void ctest_clparse_invoke_entity_num (void *p)
{
	clparse_entnum_arg_t *a = (clparse_entnum_arg_t *)p;
	entity_t			 *ent = a->side ? c_ref_CL_EntityNum (a->num) : CL_EntityNum (a->num);
	entity_t			 *base = a->side ? ctest_clparse_oracle_ents : ctest_clparse_ents;
	if (!ent)
		a->out = -1;
	else if (ent < base || ent >= base + CTEST_CLPARSE_ENTITIES)
		a->out = -2;
	else
		a->out = (int)(ent - base);
}

/* Returns the guard status; *outidx receives the entity index the call handed
   back (-1 NULL, -2 outside the fixture array), which is what lines the two
   sides up -- the raw pointers differ by construction. */
int ctest_clparse_entity_num (int side, int num, int *outidx)
{
	clparse_entnum_arg_t a;
	int					 r;
	a.side = side;
	a.num = num;
	a.out = -3;
	r = Host_Guard (ctest_clparse_invoke_entity_num, &a);
	*outidx = a.out;
	return r;
}

static void ctest_clparse_invoke_local_sound (void *p)
{
	if (*(int *)p)
		c_ref_CL_ParseLocalSound ();
	else
		CL_ParseLocalSound ();
}

void ctest_clparse_begin_reading (int side)
{
	if (side)
		c_ref_MSG_BeginReading ();
	else
		MSG_BeginReading ();
}

int ctest_clparse_parse_local_sound (int side)
{
	int s = side;
	return Host_Guard (ctest_clparse_invoke_local_sound, &s);
}

typedef struct
{
	int side;
	int slot;
} clparse_newtrans_arg_t;

static void ctest_clparse_invoke_new_translation (void *p)
{
	clparse_newtrans_arg_t *a = (clparse_newtrans_arg_t *)p;
	if (a->side)
		c_ref_CL_NewTranslation (a->slot);
	else
		CL_NewTranslation (a->slot);
}

int ctest_clparse_new_translation (int side, int slot)
{
	clparse_newtrans_arg_t a;
	a.side = side;
	a.slot = slot;
	return Host_Guard (ctest_clparse_invoke_new_translation, &a);
}

static void ctest_clparse_invoke_register_particles (void *p)
{
	if (*(int *)p)
		c_ref_CL_RegisterParticles ();
	else
		CL_RegisterParticles ();
}

int ctest_clparse_register_particles (int side)
{
	int s = side;
	return Host_Guard (ctest_clparse_invoke_register_particles, &s);
}

/* --------------------------------------------------------------------------
 * Whole-fixture reset. Publishes a non-degenerate starting state into BOTH
 * copies of everything: live entity/score/static arrays with room in them, a
 * populated model and sound precache, a writable cls.message, a connected
 * client at signon 2 (so svc_signonnum 3 is the reachable transition), a
 * non-zero cl.time so the lerp arms are live, and protocol PROTOCOL_FITZQUAKE
 * with no protocolflags (MSG_ReadCoord takes its 16-bit path).
 *
 * cl.statss[] and cl.particle_precache[].name hold q_strdup'd storage that the
 * parser replaces in place, so both are freed here rather than merely zeroed;
 * leaking them across ~1500 fuzz iterations would be the harness's own bug.
 */
void ctest_clparse_reset (void)
{
	int i, j;

	for (j = 0; j < 2; j++)
	{
		client_state_t *c = ctest_clparse_cl (j);
		for (i = 0; i < MAX_CL_STATS; i++)
		{
			Mem_Free (c->statss[i]);
			c->statss[i] = NULL;
		}
		for (i = 0; i < MAX_PARTICLETYPES; i++)
		{
			Mem_Free (c->particle_precache[i].name);
			c->particle_precache[i].name = NULL;
			c->particle_precache[i].index = 0;
			c->local_particle_precache[i].name = NULL;
			c->local_particle_precache[i].index = 0;
		}
	}

	/* qcvm is normalized out of every comparison and nothing in this link
	   loads client progs, so wiping the rest of the struct is safe. */
	memset (&cl.movemessages, 0, offsetof (client_state_t, qcvm) - offsetof (client_state_t, movemessages));
	memset (&c_ref_cl.movemessages, 0, offsetof (client_state_t, qcvm) - offsetof (client_state_t, movemessages));
	cl.zoom = c_ref_cl.zoom = 0.0f;
	cl.zoomdir = c_ref_cl.zoomdir = 0.0f;
	memset (cl.serverinfo, 0, sizeof (cl.serverinfo));
	memset (c_ref_cl.serverinfo, 0, sizeof (c_ref_cl.serverinfo));

	memset (cl_lightstyle, 0, sizeof (cl_lightstyle));
	memset (c_ref_cl_lightstyle, 0, sizeof (c_ref_cl_lightstyle));

	memset (ctest_clparse_models, 0, sizeof (ctest_clparse_models));
	memset (ctest_clparse_sounds, 0, sizeof (ctest_clparse_sounds));
	for (i = 0; i < CTEST_CLPARSE_MODELS; i++)
	{
		ctest_clparse_models[i].numframes = 4;
		ctest_clparse_models[i].flags = 0;
		ctest_clparse_models[i].type = mod_alias;
		ctest_clparse_models[i].synctype = ST_SYNC;
		q_snprintf (ctest_clparse_models[i].name, sizeof (ctest_clparse_models[i].name), "progs/ctest%i.mdl", i);
	}

	ctest_clparse_attach_arrays (CTEST_CLPARSE_ENTITIES);
	ctest_clparse_attach_message ();
	ctest_clparse_seed_precaches (CTEST_CLPARSE_MODELS, CTEST_CLPARSE_SOUNDS);
	ctest_clparse_set_protocol (PROTOCOL_FITZQUAKE, 0, 0, 0);
	ctest_clparse_set_time (1.5, 1.4, 1.5, 1.4);
	ctest_clparse_set_counts (4, 1, 8, 0);
	ctest_clparse_set_conn ((int)ca_connected, 2, 0, 0);
	ctest_clparse_set_intermission (0);
	ctest_clparse_set_shownet (0.0f);
}

/* --------------------------------------------------------------------------
 * Late additions: shared globals the parser touches that are neither `cl` nor
 * `cls`.
 *
 * gl_vidsdl.c and gl_model.c are not oracle sources, so `vid` and
 * `mod_known`/`mod_numknown` are single stubs.c objects both sides write. That
 * makes vid.recalc_refdef comparable only if it is cleared between the two
 * runs, which is what the setter is for.
 */
extern qmodel_t mod_known[];
extern int		mod_numknown;

void ctest_clparse_set_vid_recalc (int v)
{
	vid.recalc_refdef = v ? true : false;
}

int ctest_clparse_get_vid_recalc (void)
{
	return vid.recalc_refdef ? 1 : 0;
}

/* stubs.c:7228 sizes mod_known at 4 and leaves mod_numknown 0, which makes
   CL_RegisterParticles's second loop iterate zero times on BOTH sides -- a
   comparison that passes without executing anything. Raising the count (the
   two must move together, as stubs.c:7226 says) makes the loop reach
   PScript_UpdateModelEffects, which stubs.c counts and whose last model name
   it records, so the loop bound and the iteration order become observable. */
void ctest_clparse_set_mod_numknown (int n)
{
	int i;
	if (n < 0)
		n = 0;
	if (n > 4)
		n = 4;
	for (i = 0; i < n; i++)
		q_snprintf (mod_known[i].name, sizeof (mod_known[i].name), "progs/known%i.mdl", i);
	mod_numknown = n;
}

/* cl.particle_precache[].name is normally filled by svcdp_precache or
   CL_GenerateRandomParticlePrecache, both of which reach an abort stub before
   they can store anything, so CL_RegisterParticles's first loop would only
   ever see NULLs. Seeding a name directly is what lets the non-NULL arm run.
   Storage is q_strdup'd on both sides because ctest_clparse_reset frees it
   with Mem_Free, exactly as CL_FreeState does. */
void ctest_clparse_set_particle_name (int idx, const char *name)
{
	int j;
	if (idx < 0 || idx >= MAX_PARTICLETYPES)
		return;
	for (j = 0; j < 2; j++)
	{
		client_state_t *c = ctest_clparse_cl (j);
		Mem_Free (c->particle_precache[idx].name);
		c->particle_precache[idx].name = name ? q_strdup (name) : NULL;
		c->particle_precache[idx].index = 0;
	}
}

/* cl.statss[] is the other q_strdup'd table; svcfte_updatestatstring writes it
   and CL_ParseStatString frees the previous value, so a test that wants to see
   the free-and-replace path needs a value already there. */
void ctest_clparse_set_statstring (int idx, const char *value)
{
	int j;
	if (idx < 0 || idx >= MAX_CL_STATS)
		return;
	for (j = 0; j < 2; j++)
	{
		client_state_t *c = ctest_clparse_cl (j);
		Mem_Free (c->statss[idx]);
		c->statss[idx] = value ? q_strdup (value) : NULL;
	}
}

void ctest_clparse_set_stat (int idx, int value)
{
	int j;
	if (idx < 0 || idx >= MAX_CL_STATS)
		return;
	for (j = 0; j < 2; j++)
	{
		client_state_t *c = ctest_clparse_cl (j);
		c->stats[idx] = value;
		c->statsf[idx] = (float)value;
	}
}

void ctest_clparse_set_items (int items)
{
	cl.items = c_ref_cl.items = items;
}

int ctest_clparse_get_stat (int side, int idx)
{
	if (idx < 0 || idx >= MAX_CL_STATS)
		return 0;
	return ctest_clparse_cl (side)->stats[idx];
}
