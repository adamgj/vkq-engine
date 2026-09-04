/* Phase 7 M6 oracle TU for Quake/sv_main.c (task T6.5).
 *
 * Three jobs:
 *  1. own the C-visible objects Quake/sv_main_glue.c defines in the shipping
 *     build -- `sv`, `svs`, `sv_protocol`, `sv_protocol_pext1`,
 *     `sv_protocol_pext2`, `sv_netsort` and `sv_smoothplatformlerps` -- plus
 *     the five sv_user.c cvars Quake/sv_user_glue.c owns there, because
 *     stubs/sv_user_ref.c deliberately leaves those bare (T6.4);
 *  2. provide the ctest bodies of every `SvMain_Glue_*` trampoline, so the
 *     Rust port links here exactly as it links against sv_main_glue.c;
 *  3. expose one `ctest_svmain_drive_*_c` / `_rs` pair per subject plus flat
 *     snapshot accessors, so tests/sv_main_differential.rs never needs an ABI
 *     mirror of client_t.
 *
 * Every oracle call site is spelled as an explicit `c_ref_*` name (the M5 wave
 * found that relying on c_ref_prelude.h's rename macros lets a `#undef`
 * elsewhere silently redirect an oracle call while the test still passes).
 *
 * DEVIATIONS from Quake/sv_main_glue.c, all forced by stubs.c (which this file
 * may not edit) -- each is called out in the T6.5 report:
 *  - `SvMain_Glue_LoadProgs` passes NULL/0 for the builtin table instead of
 *    `pr_ssqcbuiltins`/`pr_ssqcnumbuiltins`. stubs.c's PR_LoadProgs (6892)
 *    Sys_Errors before looking at them, and Sys_Error longjmps in ctest, so
 *    SV_SpawnServer is not drivable past that point either way.
 *  - `SvMain_Glue_PrecacheModel` is a no-op returning CTEST_GUARD_OK.
 *    SV_Precache_Model lives in Quake/pr_cmds.c, which is neither an oracle
 *    source nor stubbed, so calling it would be an unresolved external.
 *    Unreachable for the same SV_SpawnServer reason.
 *  - `NET_CanSendMessage` (stubs.c:6946) returns true and `NET_SendMessage`
 *    (6952) returns 1, so SV_SendServerinfo always SZ_Clears the reliable it
 *    just built. `cursize` is therefore 0 on both sides and the serverinfo
 *    BYTES are compared as the residue left in `client->msgbuf`, which the
 *    fixture zeroes before each drive.
 *  - `NET_CheckNewConnections` (stubs.c:6912) returns NULL, so
 *    SV_CheckForNewClients only exercises its immediate loop exit.
 *  - `NET_QSocketGetTrueAddressString` (stubs.c:6922) returns "ctest", never
 *    "LOCAL", so SV_SendServerinfo's limit_unreliable super-size arm
 *    (sv_main.c:507) is unreachable; the DATAGRAM_MTU clamp is always taken.
 *  - `NET_QSocketGetProQuakeAngleHack` (stubs.c:6928) returns false, so the
 *    2048-entity ProQuake arm (sv_main.c:478) is unreachable.
 */

/* -------------------------------------------------------------------------
 * C-visible storage. In the shipping build these live in
 * Quake/sv_main_glue.c and Quake/sv_user_glue.c; here they are the Rust
 * side's storage, and the prelude's rename keeps the oracle's own copies
 * (c_ref_sv, c_ref_svs, ...) compiled out of Quake/sv_main.c itself.
 *
 * sv and svs are the exception: T6.6 moved that storage into Rust
 * (quake-capi's sv_main.rs, closing the ADR-007 sv/svs row), so they are
 * DECLARED here and defined there. Defining them here as well would put two
 * definitions of the same symbol in one link, and which one won would depend
 * on how rustc happened to split its codegen units -- the same
 * configuration-dependent resolution that let `precache` reach CI and break
 * only the Windows job (see scripts/harness/check_ctest_symbols.sh). The
 * mirror below therefore writes c_ref_sv into the Rust-owned storage, which
 * is also what the shipping build does.
 *
 * DUPLICATE-SYMBOL HAZARD: if Quake/sv_main_glue.c or Quake/sv_user_glue.c is
 * ever added to build.rs' C_SOURCES, the definitions below must be deleted in
 * one go -- every name still defined here would otherwise be defined twice.
 */

#undef sv
#undef svs
#undef sv_protocol
#undef sv_protocol_pext1
#undef sv_protocol_pext2
#undef sv_netsort
#undef sv_smoothplatformlerps
#undef sv_edgefriction
#undef sv_idealpitchscale
#undef sv_altnoclip
#undef sv_maxspeed
#undef sv_accelerate

extern server_t		sv;   /* defined by quake-capi sv_main.rs (T6.6) */
extern server_static_t svs; /* defined by quake-capi sv_main.rs (T6.6) */

int			 sv_protocol = PROTOCOL_RMQ;
unsigned int sv_protocol_pext1 = PEXT1_SUPPORTED_SERVER;
unsigned int sv_protocol_pext2 = PEXT2_SUPPORTED_SERVER;

cvar_t sv_netsort = {"sv_netsort", "1", CVAR_NONE};
cvar_t sv_smoothplatformlerps = {"sv_smoothplatformlerps", "1", CVAR_NONE};

/* The five sv_user.c cvars (sv_user.c:29/43/44/198/199) that SV_Init
 * registers are DEFINED by stubs/sv_user_ref.c, which owns the sv_user glue
 * storage; SV_Init only needs to reach them. Declared, never defined here. */
extern cvar_t sv_edgefriction;
extern cvar_t sv_idealpitchscale;
extern cvar_t sv_altnoclip;
extern cvar_t sv_maxspeed;
extern cvar_t sv_accelerate;

/* the oracle's own copies, reached explicitly from here on */
extern server_t		   c_ref_sv;
extern server_static_t c_ref_svs;
extern int			   c_ref_sv_protocol;
extern unsigned int	   c_ref_sv_protocol_pext1;
extern unsigned int	   c_ref_sv_protocol_pext2;
extern int			   c_ref_net_activeconnections;

/* -------------------------------------------------------------------------
 * Phase 7 M7 (T7.0): `cl`/`cls` split in two the same way `sv`/`svs` did at
 * T6.5. cl_main.c is an oracle source now, so client.h's declarations reach
 * c_ref_cl / c_ref_cls through the prelude rename, and stubs.c keeps the plain
 * (Rust-read) copies -- see the DUPLICATE-SYMBOL HAZARD block there.
 * SV_Pext_f's console arm reads all three fields seeded in ctest_svmain_reset
 * below, from c_ref_cl/c_ref_cls on the oracle side and from these plain ones
 * on the Rust side (quake-capi/src/sv_main.rs:105,107).
 */
#undef cl
#undef cls
extern client_state_t  cl;
extern client_static_t cls;

/* -------------------------------------------------------------------------
 * Plain (== Rust) cvar/cmd entry points. The prelude renamed these onto the
 * oracle; the glue trampolines below must reach the Rust ones, exactly as
 * sv_main_glue.c does under -Duse_rust_cvar.
 */

#undef Cvar_RegisterVariable
#undef Cvar_SetCallback
#undef Cvar_Set
#undef Cvar_SetValue
#undef Cmd_AddCommand2
#undef Cmd_TokenizeString
#undef cmd_source

/* The three sv_main.c entry points the Rust port exports under their PLAIN
 * names (no status, no raise). The prelude renames those names onto the
 * oracle, so without these #undefs the "Rust" side of each driver below would
 * silently call c_ref_* and every one of their differentials would be
 * vacuous. Found by a mutation check, task T6.5. */
#undef SV_ClearDatagram
#undef SV_ModelIndex
#undef SV_ModelForIndex
extern void		 SV_ClearDatagram (void);
extern int		 SV_ModelIndex (const char *name);
extern qmodel_t *SV_ModelForIndex (int index);

extern void			   Cvar_RegisterVariable (cvar_t *variable);
extern void			   Cvar_SetCallback (cvar_t *var, cvarcallback_t func);
extern void			   Cvar_Set (const char *var_name, const char *value);
extern void			   Cvar_SetValue (const char *var_name, const float value);
extern cmd_function_t *Cmd_AddCommand2 (const char *cmd_name, xcommand_t function, cmd_source_t srctype, qboolean qcinterceptable);
extern void			   Cmd_TokenizeString (const char *text);
extern cmd_source_t	   cmd_source;

/* stubs.c:? -- the ctest setjmp raise topology (ADR-009). */
extern int	Host_Guard (void (*fn) (void *), void *arg);
extern void Host_Reraise (int status);

/* The 23 cvars SV_Init registers. Most exist twice in this link: the oracle
 * sources define the c_ref_* copy, and stubs.c (or this file) defines the
 * plain twin the Rust port reads through quake-c-sys. `sv_aim` and
 * `pr_checkextension` are deliberately NOT renamed by the prelude, so those
 * two are a single shared object registered by both sides in turn. */
#undef sv_maxvelocity
#undef sv_gravity
#undef sv_friction
#undef sv_stopspeed
#undef sv_nostep
#undef sv_freezenonclients
#undef sv_gameplayfix_spawnbeforethinks
#undef sv_gameplayfix_bouncedownslopes
#undef sv_gameplayfix_elevators
#undef sv_fastpushmove
#undef sv_pushgrid
#undef sv_analyticphysics
#undef sv_fte_recursivehullckeck
#undef sv_fte_createareanode

extern cvar_t sv_maxvelocity;
extern cvar_t sv_gravity;
extern cvar_t sv_friction;
extern cvar_t sv_stopspeed;
extern cvar_t sv_nostep;
extern cvar_t sv_freezenonclients;
extern cvar_t sv_gameplayfix_spawnbeforethinks;
extern cvar_t sv_gameplayfix_bouncedownslopes;
extern cvar_t sv_gameplayfix_elevators;
extern cvar_t sv_fastpushmove;
extern cvar_t sv_pushgrid;
extern cvar_t sv_analyticphysics;
extern cvar_t sv_fte_recursivehullckeck;
extern cvar_t sv_fte_createareanode;
extern cvar_t sv_aim;
extern cvar_t pr_checkextension;

extern cvar_t c_ref_sv_maxvelocity;
extern cvar_t c_ref_sv_gravity;
extern cvar_t c_ref_sv_friction;
extern cvar_t c_ref_sv_edgefriction;
extern cvar_t c_ref_sv_stopspeed;
extern cvar_t c_ref_sv_maxspeed;
extern cvar_t c_ref_sv_accelerate;
extern cvar_t c_ref_sv_idealpitchscale;
extern cvar_t c_ref_sv_nostep;
extern cvar_t c_ref_sv_freezenonclients;
extern cvar_t c_ref_sv_gameplayfix_spawnbeforethinks;
extern cvar_t c_ref_sv_gameplayfix_bouncedownslopes;
extern cvar_t c_ref_sv_gameplayfix_elevators;
extern cvar_t c_ref_sv_fastpushmove;
extern cvar_t c_ref_sv_pushgrid;
extern cvar_t c_ref_sv_analyticphysics;
extern cvar_t c_ref_sv_altnoclip;
extern cvar_t c_ref_sv_netsort;
extern cvar_t c_ref_sv_smoothplatformlerps;
extern cvar_t c_ref_sv_fte_recursivehullckeck;
extern cvar_t c_ref_sv_fte_createareanode;

/* the Rust cores (quake-capi/src/sv_main.rs) */
extern int	quake_rs_sv_init (void);
extern void quake_rs_sv_protocol_f (void);
extern int	quake_rs_sv_pext_f (void);
extern int	quake_rs_sv_start_particle (vec3_t org, vec3_t dir, int color, int count);
extern int	quake_rs_sv_start_sound (edict_t *entity, float *origin, int channel, const char *sample, int volume, float attenuation);
extern int	quake_rs_sv_local_sound (client_t *client, const char *sample);
extern int	quake_rs_sv_send_serverinfo (client_t *client);
extern int	quake_rs_sv_connect_client (int clientnum);
extern int	quake_rs_sv_check_for_new_clients (void);
extern int	quake_rs_sv_save_spawnparms (void);
extern int	SV_ModelIndex (const char *name);

/* oracle entry points (Quake/sv_main.c compiled with the prelude) */
extern void		 c_ref_SV_Init (void);
extern void		 c_ref_SV_StartParticle (vec3_t org, vec3_t dir, int color, int count);
extern void		 c_ref_SV_StartSound (edict_t *entity, float *origin, int channel, const char *sample, int volume, float attenuation);
extern void		 c_ref_SV_LocalSound (client_t *client, const char *sample);
extern void		 c_ref_SV_SendServerinfo (client_t *client);
extern void		 c_ref_SV_ConnectClient (int clientnum);
extern void		 c_ref_SV_CheckForNewClients (void);
extern void		 c_ref_SV_SaveSpawnparms (void);
extern void		 c_ref_SV_ClearDatagram (void);
extern int		 c_ref_SV_ModelIndex (const char *name);
extern qmodel_t *c_ref_SV_ModelForIndex (int index);

/* shared engine primitives, spelled explicitly */
extern qboolean	   c_ref_Cmd_ExecuteString (const char *text, cmd_source_t src);
extern void		   c_ref_Cmd_TokenizeString (const char *text);
extern const char *c_ref_PR_GetString (int num);
extern void		   c_ref_PR_ExecuteProgram (func_t fnum);
/* NOT an oracle rename: pr_edict.c is not a C_SOURCES oracle file, so
 * stubs.c:6611 owns the single plain-named ED_FindGlobal both sides use. */
extern ddef_t	  *ED_FindGlobal (const char *name);
extern void		   c_ref_SVFTE_SetupFrames (client_t *client);
extern void		   c_ref_SV_SendReconnect (void);
extern void		   c_ref_SV_CreateBaseline (void);
extern void		   c_ref_MSG_WriteByte (sizebuf_t *sb, int c);
extern void		   c_ref_MSG_WriteChar (sizebuf_t *sb, int c);
extern void		   c_ref_MSG_WriteShort (sizebuf_t *sb, int c);
extern void		   c_ref_MSG_WriteLong (sizebuf_t *sb, int c);
extern void		   c_ref_MSG_WriteString (sizebuf_t *sb, const char *s);
extern void		   c_ref_MSG_WriteCoord (sizebuf_t *sb, float f, unsigned int flags);

/* stubs.c fixture helpers */
extern void *ctest_progs_reset_vm (int max_edicts, int entityfields);
extern void	 ctest_progs_set_strings (char *blob, int size, int progsstrings);
extern int	 ctest_progs_edict_size (void);
extern void *ctest_progs_edicts (void);
extern void	 ctest_set_host_client (client_t *c);
extern void	 ctest_clear_con_log (void);

/* -------------------------------------------------------------------------
 * SvMain_Glue_* -- the ctest bodies of Quake/sv_main_glue.c's trampolines.
 */

typedef struct
{
	int	  i;
	float f;
} svmain_ierr_t;

static void SvMain_InvokeErrorVolume (void *p)
{
	Host_Error ("SV_StartSound: volume = %i", ((svmain_ierr_t *)p)->i);
}

int SvMain_Glue_ErrorVolume (int volume)
{
	svmain_ierr_t a;
	a.i = volume;
	a.f = 0;
	return Host_Guard (SvMain_InvokeErrorVolume, &a);
}

static void SvMain_InvokeErrorAttenuation (void *p)
{
	/* COMPAT: ADR-005 -- the only floating-point conversion specifier in
	   Quake/sv_main.c, kept in C so the Rust formatter never sees it. */
	Host_Error ("SV_StartSound: attenuation = %f", ((svmain_ierr_t *)p)->f);
}

int SvMain_Glue_ErrorAttenuation (float attenuation)
{
	svmain_ierr_t a;
	a.i = 0;
	a.f = attenuation;
	return Host_Guard (SvMain_InvokeErrorAttenuation, &a);
}

static void SvMain_InvokeErrorChannel (void *p)
{
	Host_Error ("SV_StartSound: channel = %i", ((svmain_ierr_t *)p)->i);
}

int SvMain_Glue_ErrorChannel (int channel)
{
	svmain_ierr_t a;
	a.i = channel;
	a.f = 0;
	return Host_Guard (SvMain_InvokeErrorChannel, &a);
}

typedef struct
{
	int			 handle;
	const char **out;
} svmain_getstring_t;

static void SvMain_InvokeGetString (void *p)
{
	svmain_getstring_t *a = (svmain_getstring_t *)p;
	*a->out = c_ref_PR_GetString (a->handle);
}

int SvMain_Glue_GetString (int handle, const char **out)
{
	svmain_getstring_t a;
	a.handle = handle;
	a.out = out;
	return Host_Guard (SvMain_InvokeGetString, &a);
}

static void SvMain_InvokeSetupFrames (void *p)
{
	c_ref_SVFTE_SetupFrames ((client_t *)p);
}

int SvMain_Glue_SetupFrames (client_t *client)
{
	return Host_Guard (SvMain_InvokeSetupFrames, client);
}

static void SvMain_InvokeSendReconnect (void *p)
{
	(void)p;
	c_ref_SV_SendReconnect ();
}

int SvMain_Glue_SendReconnect (void)
{
	return Host_Guard (SvMain_InvokeSendReconnect, NULL);
}

static void SvMain_InvokeCreateBaseline (void *p)
{
	(void)p;
	c_ref_SV_CreateBaseline ();
}

int SvMain_Glue_CreateBaseline (void)
{
	return Host_Guard (SvMain_InvokeCreateBaseline, NULL);
}

static void SvMain_InvokeClearMemory (void *p)
{
	(void)p;
	Host_ClearMemory ();
}

int SvMain_Glue_ClearMemory (void)
{
	return Host_Guard (SvMain_InvokeClearMemory, NULL);
}

/* DEVIATION: NULL/0 builtin table, see the header. */
static void SvMain_InvokeLoadProgs (void *p)
{
	(void)p;
	PR_LoadProgs ("progs.dat", true, PROGHEADER_CRC, NULL, 0);
}

int SvMain_Glue_LoadProgs (void)
{
	return Host_Guard (SvMain_InvokeLoadProgs, NULL);
}

typedef struct
{
	const char *name;
	void	  **out;
} svmain_modforname_t;

static void SvMain_InvokeModForName (void *p)
{
	svmain_modforname_t *a = (svmain_modforname_t *)p;
	*a->out = Mod_ForName (a->name, false);
}

int SvMain_Glue_ModForName (const char *name, void **out)
{
	svmain_modforname_t a;
	a.name = name;
	a.out = out;
	return Host_Guard (SvMain_InvokeModForName, &a);
}

static void SvMain_InvokeLoadFromFile (void *p)
{
	ED_LoadFromFile ((const char *)p);
}

int SvMain_Glue_LoadFromFile (const char *data)
{
	return Host_Guard (SvMain_InvokeLoadFromFile, (void *)(intptr_t)data);
}

/* DEVIATION: no-op, see the header. */
int SvMain_Glue_PrecacheModel (const char *name)
{
	(void)name;
	return 0;
}

static void SvMain_InvokeSetNewParms (void *p)
{
	(void)p;
	c_ref_PR_ExecuteProgram (pr_global_struct->SetNewParms);
}

int SvMain_Glue_CallSetNewParms (void)
{
	return Host_Guard (SvMain_InvokeSetNewParms, NULL);
}

static void SvMain_InvokeSetChangeParms (void *p)
{
	pr_global_struct->self = EDICT_TO_PROG ((edict_t *)p);
	c_ref_PR_ExecuteProgram (pr_global_struct->SetChangeParms);
}

int SvMain_Glue_CallSetChangeParms (edict_t *ent)
{
	return Host_Guard (SvMain_InvokeSetChangeParms, ent);
}

static void SvMain_InvokeRegisterVariable (void *p)
{
	Cvar_RegisterVariable ((cvar_t *)p);
}

int SvMain_Glue_RegisterVariable (cvar_t *var)
{
	return Host_Guard (SvMain_InvokeRegisterVariable, var);
}

void SvMain_Glue_SetNotifyCallback (cvar_t *var)
{
	Cvar_SetCallback (var, Host_Callback_Notify);
}

static void SvMain_PextCommand (void)
{
	Host_Reraise (quake_rs_sv_pext_f ());
}

static void SvMain_ProtocolCommand (void)
{
	quake_rs_sv_protocol_f ();
}

static void SvMain_InvokeAddCommands (void *p)
{
	(void)p;
	Cmd_AddCommand2 ("pext", SvMain_PextCommand, src_command, false);
	Cmd_AddCommand2 ("sv_protocol", SvMain_ProtocolCommand, src_command, false);
}

int SvMain_Glue_AddCommands (void)
{
	return Host_Guard (SvMain_InvokeAddCommands, NULL);
}

typedef struct
{
	const char *name;
	const char *value;
	float		fvalue;
} svmain_cvarset_t;

static void SvMain_InvokeCvarSet (void *p)
{
	svmain_cvarset_t *a = (svmain_cvarset_t *)p;
	Cvar_Set (a->name, a->value);
}

int SvMain_Glue_CvarSet (const char *name, const char *value)
{
	svmain_cvarset_t a;
	a.name = name;
	a.value = value;
	a.fvalue = 0;
	return Host_Guard (SvMain_InvokeCvarSet, &a);
}

static void SvMain_InvokeCvarSetValue (void *p)
{
	svmain_cvarset_t *a = (svmain_cvarset_t *)p;
	Cvar_SetValue (a->name, a->fvalue);
}

int SvMain_Glue_CvarSetValue (const char *name, float value)
{
	svmain_cvarset_t a;
	a.name = name;
	a.value = NULL;
	a.fvalue = value;
	return Host_Guard (SvMain_InvokeCvarSetValue, &a);
}

typedef struct
{
	int			kind;
	int			i;
	float		f;
	const char *s;
} svmain_write_t;

typedef struct
{
	sizebuf_t			 *sb;
	const svmain_write_t *ops;
	int					  count;
} svmain_writebatch_arg_t;

static void SvMain_InvokeWriteBatch (void *p)
{
	svmain_writebatch_arg_t *a = (svmain_writebatch_arg_t *)p;
	int						 k;

	for (k = 0; k < a->count; k++)
	{
		const svmain_write_t *op = &a->ops[k];
		switch (op->kind)
		{
		case 0:
			c_ref_MSG_WriteByte (a->sb, op->i);
			break;
		case 1:
			c_ref_MSG_WriteChar (a->sb, op->i);
			break;
		case 2:
			c_ref_MSG_WriteShort (a->sb, op->i);
			break;
		case 3:
			c_ref_MSG_WriteLong (a->sb, op->i);
			break;
		case 4:
			c_ref_MSG_WriteString (a->sb, op->s);
			break;
		case 5:
			c_ref_MSG_WriteCoord (a->sb, op->f, sv.protocolflags);
			break;
		default:
			break;
		}
	}
}

int SvMain_Glue_WriteBatch (sizebuf_t *sb, const svmain_write_t *ops, int count)
{
	svmain_writebatch_arg_t arg;

	arg.sb = sb;
	arg.ops = ops;
	arg.count = count;
	return Host_Guard (SvMain_InvokeWriteBatch, &arg);
}

void SvMain_Glue_ServerinfoPrint (char *out, size_t size, int crc)
{
	q_snprintf (out, size, "%c\n" ENGINE_NAME_AND_VER " Server (%i CRC)\n", 2, crc);
}

void SvMain_Glue_SpawnParmGlobal (int index, float *out)
{
	ddef_t *g = ED_FindGlobal (va ("parm%i", index));
	*out = g ? qcvm->globals[g->ofs] : 0;
}

void SvMain_Glue_InitDebugEdicts (void)
{
#if defined(DEBUG) || defined(_DEBUG)
	for (int j = 0; j < qcvm->max_edicts; j++)
	{
		edict_t *e = EDICT_NUM_NO_CHECK (j);
		e->qcvm_owner = qcvm;
		e->edict_ptr = e;
		e->edict_num = j;
	}
#endif
}

void SvMain_Glue_AssertEdictNotFree (edict_t *ent)
{
	assert (!ent->free);
	(void)ent;
}

/* =========================================================================
 * Fixture
 */

#define CTEST_SVMAIN_CLIENTS   4
#define CTEST_SVMAIN_STRSLOTS  8
#define CTEST_SVMAIN_STRSTRIDE 64
#define CTEST_SVMAIN_EDICTS	   16

#define CTEST_SIDE_C  0
#define CTEST_SIDE_RS 1

static client_t		ctest_svmain_clients_c[CTEST_SVMAIN_CLIENTS];
static client_t		ctest_svmain_clients_r[CTEST_SVMAIN_CLIENTS];
static globalvars_t ctest_svmain_globals;
static char			ctest_svmain_strings[CTEST_SVMAIN_STRSLOTS * CTEST_SVMAIN_STRSTRIDE];
static int			ctest_svmain_socket;

/* precache tables: index 0 is the map model, matching SV_SpawnServer */
static const char *const ctest_svmain_models[] = {"maps/ctest.bsp", "progs/player.mdl", "progs/eyes.mdl", "*1", NULL};
static const char *const ctest_svmain_sounds[] = {NULL, "weapons/r_exp3.wav", "items/damage.wav", "player/udeath.wav", NULL};

static client_t *ctest_svmain_side_clients (int side)
{
	return side == CTEST_SIDE_C ? ctest_svmain_clients_c : ctest_svmain_clients_r;
}

static server_t *ctest_svmain_side_sv (int side)
{
	return side == CTEST_SIDE_C ? &c_ref_sv : &sv;
}

static server_static_t *ctest_svmain_side_svs (int side)
{
	return side == CTEST_SIDE_C ? &c_ref_svs : &svs;
}

static void ctest_svmain_init_client (client_t *c, int i)
{
	memset (c, 0, sizeof (*c));
	c->active = true;
	c->spawned = true;
	c->netconnection = (struct qsocket_s *)&ctest_svmain_socket;
	c->message.data = c->msgbuf;
	c->message.maxsize = sizeof (c->msgbuf);
	c->message.allowoverflow = true;
	c->datagram.data = c->datagram_buf;
	c->datagram.maxsize = sizeof (c->datagram_buf);
	c->datagram.allowoverflow = true;
	c->limit_entities = 32000;
	c->limit_unreliable = 1400;
	c->limit_reliable = 64000;
	c->limit_models = 2048;
	c->limit_sounds = 2048;
	q_snprintf (c->name, sizeof (c->name), "player%i", i);
}

static void ctest_svmain_init_sv (server_t *s, int protocol, unsigned int protocolflags, int loadgame)
{
	int i;

	memset (s, 0, sizeof (*s));
	s->active = true;
	s->state = ss_active;
	s->loadgame = loadgame ? true : false;
	s->protocol = (unsigned)protocol;
	s->protocolflags = protocolflags;
	q_strlcpy (s->name, "ctest", sizeof (s->name));
	q_strlcpy (s->modelname, "maps/ctest.bsp", sizeof (s->modelname));

	for (i = 0; ctest_svmain_models[i]; i++)
		s->model_precache[i] = ctest_svmain_models[i];
	for (i = 0; i < 4; i++)
		s->sound_precache[i] = ctest_svmain_sounds[i];

	s->datagram.data = s->datagram_buf;
	s->datagram.maxsize = sizeof (s->datagram_buf);
	s->datagram.allowoverflow = true;
	s->reliable_datagram.data = s->reliable_datagram_buf;
	s->reliable_datagram.maxsize = sizeof (s->reliable_datagram_buf);
	s->signon.data = s->signon_buf;
	s->signon.maxsize = sizeof (s->signon_buf);
	s->multicast.data = s->multicast_buf;
	s->multicast.maxsize = sizeof (s->multicast_buf);
}

/* Builds the shared qcvm (both sides read the ambient `qcvm`, which is not
 * renamed by the prelude) and resets both sides' sv/svs to identical state. */
void ctest_svmain_reset (int protocol, unsigned int protocolflags, int maxclients, int loadgame)
{
	int i;

	ctest_progs_reset_vm (CTEST_SVMAIN_EDICTS, 64);
	memset (ctest_svmain_strings, 0, sizeof (ctest_svmain_strings));
	q_strlcpy (ctest_svmain_strings + CTEST_SVMAIN_STRSTRIDE, "ctest level", CTEST_SVMAIN_STRSTRIDE);
	ctest_progs_set_strings (ctest_svmain_strings, (int)sizeof (ctest_svmain_strings), CTEST_SVMAIN_STRSLOTS * CTEST_SVMAIN_STRSTRIDE);

	memset (&ctest_svmain_globals, 0, sizeof (ctest_svmain_globals));
	pr_global_struct = &ctest_svmain_globals;

	qcvm->num_edicts = CTEST_SVMAIN_EDICTS;
	qcvm->progscrc = 12345;
	qcvm->edicts->v.message = CTEST_SVMAIN_STRSTRIDE;
	qcvm->edicts->v.sounds = 3;

	ctest_svmain_init_sv (&sv, protocol, protocolflags, loadgame);
	ctest_svmain_init_sv (&c_ref_sv, protocol, protocolflags, loadgame);

	for (i = 0; i < CTEST_SVMAIN_CLIENTS; i++)
	{
		ctest_svmain_init_client (&ctest_svmain_clients_c[i], i);
		ctest_svmain_init_client (&ctest_svmain_clients_r[i], i);
		ctest_svmain_clients_c[i].edict = EDICT_NUM_NO_CHECK (i + 1);
		ctest_svmain_clients_r[i].edict = EDICT_NUM_NO_CHECK (i + 1);
	}

	memset (&svs, 0, sizeof (svs));
	memset (&c_ref_svs, 0, sizeof (c_ref_svs));
	svs.maxclients = maxclients;
	svs.maxclientslimit = CTEST_SVMAIN_CLIENTS;
	svs.clients = ctest_svmain_clients_r;
	c_ref_svs.maxclients = maxclients;
	c_ref_svs.maxclientslimit = CTEST_SVMAIN_CLIENTS;
	c_ref_svs.clients = ctest_svmain_clients_c;

	sv_protocol = protocol;
	c_ref_sv_protocol = protocol;
	sv_protocol_pext1 = PEXT1_SUPPORTED_SERVER;
	c_ref_sv_protocol_pext1 = PEXT1_SUPPORTED_SERVER;
	sv_protocol_pext2 = PEXT2_SUPPORTED_SERVER;
	c_ref_sv_protocol_pext2 = PEXT2_SUPPORTED_SERVER;

	c_ref_net_activeconnections = 0;

	/* Both copies of the SV_Pext_f console-arm inputs -- see the Phase 7 M7 note
	 * at the top of this file. cl_main.c defines `cls` with no initializer
	 * (state == ca_dedicated) while stubs.c's plain copy starts at
	 * ca_disconnected, so leaving either unseeded makes the two sides read two
	 * different fixtures: that asymmetry, not a port bug, is what broke
	 * pext_f_from_the_console_prints_the_client_side_report the moment
	 * cl_main.c joined C_SOURCES. ca_disconnected is the value the test has
	 * always exercised (the "Current Protocols:" branch). */
	cls.state = ca_disconnected;
	c_ref_cls.state = ca_disconnected;
	cl.protocol = 0;
	c_ref_cl.protocol = 0;
	cl.protocol_pext2 = 0;
	c_ref_cl.protocol_pext2 = 0;

	ctest_set_host_client (NULL);
	ctest_clear_con_log ();
}

void ctest_svmain_set_pext (unsigned int pext1, unsigned int pext2)
{
	sv_protocol_pext1 = pext1;
	c_ref_sv_protocol_pext1 = pext1;
	sv_protocol_pext2 = pext2;
	c_ref_sv_protocol_pext2 = pext2;
}

void ctest_svmain_set_client_pext (int i, unsigned int pext2, int pextknown)
{
	ctest_svmain_clients_c[i].protocol_pext2 = pext2;
	ctest_svmain_clients_r[i].protocol_pext2 = pext2;
	ctest_svmain_clients_c[i].pextknown = pextknown ? true : false;
	ctest_svmain_clients_r[i].pextknown = pextknown ? true : false;
}

void ctest_svmain_set_client_flags (int i, int active, int spawned)
{
	ctest_svmain_clients_c[i].active = active ? true : false;
	ctest_svmain_clients_r[i].active = active ? true : false;
	ctest_svmain_clients_c[i].spawned = spawned ? true : false;
	ctest_svmain_clients_r[i].spawned = spawned ? true : false;
}

void ctest_svmain_set_client_limits (int i, unsigned int entities, unsigned int sounds)
{
	ctest_svmain_clients_c[i].limit_entities = entities;
	ctest_svmain_clients_r[i].limit_entities = entities;
	ctest_svmain_clients_c[i].limit_sounds = sounds;
	ctest_svmain_clients_r[i].limit_sounds = sounds;
}

void ctest_svmain_set_spawn_parm (int i, int j, float v)
{
	ctest_svmain_clients_c[i].spawn_parms[j] = v;
	ctest_svmain_clients_r[i].spawn_parms[j] = v;
}

/* globalvars_t only carries parm1..parm16 (progdefs.q1:21-36). The extended
 * spawn parms are separate QC globals that SV_SaveSpawnparms resolves with
 * ED_FindGlobal (sv_main.c:863-867), which is why the engine splits its own
 * loop at NUM_BASIC_SPAWN_PARMS. Writing NUM_TOTAL_SPAWN_PARMS floats from
 * &parm1 ran 33 slots over the rest of the struct -- the trace block and all
 * ten func_t entry points, SetChangeParms included -- and then 15 slots past
 * the end of ctest_svmain_globals itself. */
void ctest_svmain_set_global_parms (float base)
{
	int i;
	for (i = 0; i < NUM_BASIC_SPAWN_PARMS; i++)
		(&pr_global_struct->parm1)[i] = base + (float)i;
}

void ctest_svmain_set_edict_origin (int num, float x, float y, float z, float mins, float maxs)
{
	edict_t *e = EDICT_NUM_NO_CHECK (num);
	e->v.origin[0] = x;
	e->v.origin[1] = y;
	e->v.origin[2] = z;
	e->v.mins[0] = e->v.mins[1] = e->v.mins[2] = mins;
	e->v.maxs[0] = e->v.maxs[1] = e->v.maxs[2] = maxs;
}

static void ctest_svmain_clear_bufs (int i)
{
	memset (ctest_svmain_clients_c[i].msgbuf, 0, sizeof (ctest_svmain_clients_c[i].msgbuf));
	memset (ctest_svmain_clients_r[i].msgbuf, 0, sizeof (ctest_svmain_clients_r[i].msgbuf));
	memset (ctest_svmain_clients_c[i].datagram_buf, 0, sizeof (ctest_svmain_clients_c[i].datagram_buf));
	memset (ctest_svmain_clients_r[i].datagram_buf, 0, sizeof (ctest_svmain_clients_r[i].datagram_buf));
	ctest_svmain_clients_c[i].message.cursize = 0;
	ctest_svmain_clients_r[i].message.cursize = 0;
	ctest_svmain_clients_c[i].message.overflowed = false;
	ctest_svmain_clients_r[i].message.overflowed = false;
	ctest_svmain_clients_c[i].datagram.cursize = 0;
	ctest_svmain_clients_r[i].datagram.cursize = 0;
	ctest_svmain_clients_c[i].datagram.overflowed = false;
	ctest_svmain_clients_r[i].datagram.overflowed = false;
}

void ctest_svmain_clear_all_bufs (void)
{
	int i;
	for (i = 0; i < CTEST_SVMAIN_CLIENTS; i++)
		ctest_svmain_clear_bufs (i);
	memset (sv.datagram_buf, 0, sizeof (sv.datagram_buf));
	memset (c_ref_sv.datagram_buf, 0, sizeof (c_ref_sv.datagram_buf));
	sv.datagram.cursize = 0;
	c_ref_sv.datagram.cursize = 0;
	sv.datagram.overflowed = false;
	c_ref_sv.datagram.overflowed = false;
}

/* =========================================================================
 * Observation
 */

typedef struct
{
	int			 active;
	int			 spawned;
	int			 sendsignon;
	int			 pextknown;
	unsigned int protocol_pext2;
	unsigned int limit_entities;
	unsigned int limit_unreliable;
	unsigned int limit_reliable;
	unsigned int limit_models;
	unsigned int limit_sounds;
	unsigned int signon_models;
	unsigned int signon_sounds;
	int			 message_cursize;
	int			 message_maxsize;
	int			 message_overflowed;
	int			 datagram_cursize;
	int			 datagram_maxsize;
	int			 datagram_overflowed;
	double		 last_message;
	int			 edictnum;
	char		 name[32];
	float		 spawn_parms[NUM_TOTAL_SPAWN_PARMS];
} ctest_svmain_client_snap_t;

void ctest_svmain_snap_client (int side, int i, ctest_svmain_client_snap_t *out)
{
	const client_t *c = &ctest_svmain_side_clients (side)[i];
	int				j;

	memset (out, 0, sizeof (*out));
	out->active = c->active ? 1 : 0;
	out->spawned = c->spawned ? 1 : 0;
	out->sendsignon = (int)c->sendsignon;
	out->pextknown = c->pextknown ? 1 : 0;
	out->protocol_pext2 = c->protocol_pext2;
	out->limit_entities = c->limit_entities;
	out->limit_unreliable = c->limit_unreliable;
	out->limit_reliable = c->limit_reliable;
	out->limit_models = c->limit_models;
	out->limit_sounds = c->limit_sounds;
	out->signon_models = c->signon_models;
	out->signon_sounds = c->signon_sounds;
	out->message_cursize = c->message.cursize;
	out->message_maxsize = c->message.maxsize;
	out->message_overflowed = c->message.overflowed ? 1 : 0;
	out->datagram_cursize = c->datagram.cursize;
	out->datagram_maxsize = c->datagram.maxsize;
	out->datagram_overflowed = c->datagram.overflowed ? 1 : 0;
	out->last_message = c->last_message;
	out->edictnum = c->edict ? (int)((byte *)c->edict - (byte *)qcvm->edicts) / qcvm->edict_size : -1;
	memcpy (out->name, c->name, sizeof (out->name));
	for (j = 0; j < NUM_TOTAL_SPAWN_PARMS; j++)
		out->spawn_parms[j] = c->spawn_parms[j];
}

int ctest_svmain_client_snap_size (void)
{
	return (int)sizeof (ctest_svmain_client_snap_t);
}

/* first differing byte index, or -1 when identical */
static int ctest_svmain_memdiff (const void *a, const void *b, size_t n)
{
	const byte *p = (const byte *)a;
	const byte *q = (const byte *)b;
	size_t		k;
	for (k = 0; k < n; k++)
		if (p[k] != q[k])
			return (int)k;
	return -1;
}

int ctest_svmain_msgbuf_diff (int i)
{
	return ctest_svmain_memdiff (ctest_svmain_clients_c[i].msgbuf, ctest_svmain_clients_r[i].msgbuf, sizeof (ctest_svmain_clients_c[i].msgbuf));
}

int ctest_svmain_client_datagram_diff (int i)
{
	return ctest_svmain_memdiff (
		ctest_svmain_clients_c[i].datagram_buf, ctest_svmain_clients_r[i].datagram_buf, sizeof (ctest_svmain_clients_c[i].datagram_buf));
}

int ctest_svmain_sv_datagram_diff (void)
{
	return ctest_svmain_memdiff (c_ref_sv.datagram_buf, sv.datagram_buf, sizeof (sv.datagram_buf));
}

int ctest_svmain_sv_datagram_size (int side)
{
	return ctest_svmain_side_sv (side)->datagram.cursize;
}

int ctest_svmain_sv_datagram_byte (int side, int i)
{
	return ctest_svmain_side_sv (side)->datagram_buf[i];
}

int ctest_svmain_client_msg_byte (int side, int i, int k)
{
	return ctest_svmain_side_clients (side)[i].msgbuf[k];
}

int ctest_svmain_client_datagram_byte (int side, int i, int k)
{
	return ctest_svmain_side_clients (side)[i].datagram_buf[k];
}

int ctest_svmain_protocol (int side)
{
	return side == CTEST_SIDE_C ? c_ref_sv_protocol : sv_protocol;
}

unsigned int ctest_svmain_protocol_pext1 (int side)
{
	return side == CTEST_SIDE_C ? c_ref_sv_protocol_pext1 : sv_protocol_pext1;
}

unsigned int ctest_svmain_protocol_pext2 (int side)
{
	return side == CTEST_SIDE_C ? c_ref_sv_protocol_pext2 : sv_protocol_pext2;
}

int ctest_svmain_serverflags (int side)
{
	return ctest_svmain_side_svs (side)->serverflags;
}

/* =========================================================================
 * Drivers. Every one returns the Host_Guard status of the side it drove, so a
 * raise is a comparable observable rather than a crash.
 */

typedef struct
{
	float		org[3];
	float		dir[3];
	int			color;
	int			count;
	int			channel;
	int			volume;
	float		attenuation;
	const char *sample;
	int			entnum;
	int			useorigin;
	int			clientnum;
} ctest_svmain_call_t;

static ctest_svmain_call_t ctest_svmain_call;

static void ctest_svmain_invoke_startparticle (void *p)
{
	ctest_svmain_call_t *a = (ctest_svmain_call_t *)p;
	c_ref_SV_StartParticle (a->org, a->dir, a->color, a->count);
}

int ctest_svmain_drive_startparticle (int side, const float *org, const float *dir, int color, int count)
{
	ctest_svmain_call_t *a = &ctest_svmain_call;
	int					 i;

	for (i = 0; i < 3; i++)
	{
		a->org[i] = org[i];
		a->dir[i] = dir[i];
	}
	a->color = color;
	a->count = count;

	if (side == CTEST_SIDE_C)
		return Host_Guard (ctest_svmain_invoke_startparticle, a);
	return quake_rs_sv_start_particle (a->org, a->dir, a->color, a->count);
}

static void ctest_svmain_invoke_startsound (void *p)
{
	ctest_svmain_call_t *a = (ctest_svmain_call_t *)p;
	edict_t				*ent = EDICT_NUM_NO_CHECK (a->entnum);
	c_ref_SV_StartSound (ent, a->useorigin ? a->org : NULL, a->channel, a->sample, a->volume, a->attenuation);
}

int ctest_svmain_drive_startsound (int side, int entnum, const float *origin, int channel, const char *sample, int volume, float attenuation)
{
	ctest_svmain_call_t *a = &ctest_svmain_call;
	int					 i;

	a->entnum = entnum;
	a->useorigin = origin != NULL;
	for (i = 0; i < 3; i++)
		a->org[i] = origin ? origin[i] : 0.0f;
	a->channel = channel;
	a->sample = sample;
	a->volume = volume;
	a->attenuation = attenuation;

	if (side == CTEST_SIDE_C)
		return Host_Guard (ctest_svmain_invoke_startsound, a);
	return quake_rs_sv_start_sound (EDICT_NUM_NO_CHECK (entnum), a->useorigin ? a->org : NULL, channel, sample, volume, attenuation);
}

static void ctest_svmain_invoke_localsound (void *p)
{
	ctest_svmain_call_t *a = (ctest_svmain_call_t *)p;
	c_ref_SV_LocalSound (&ctest_svmain_clients_c[a->clientnum], a->sample);
}

int ctest_svmain_drive_localsound (int side, int clientnum, const char *sample)
{
	ctest_svmain_call_t *a = &ctest_svmain_call;

	a->clientnum = clientnum;
	a->sample = sample;

	if (side == CTEST_SIDE_C)
		return Host_Guard (ctest_svmain_invoke_localsound, a);
	return quake_rs_sv_local_sound (&ctest_svmain_clients_r[clientnum], sample);
}

static void ctest_svmain_invoke_serverinfo (void *p)
{
	ctest_svmain_call_t *a = (ctest_svmain_call_t *)p;
	c_ref_SV_SendServerinfo (&ctest_svmain_clients_c[a->clientnum]);
}

int ctest_svmain_drive_serverinfo (int side, int clientnum)
{
	ctest_svmain_call_t *a = &ctest_svmain_call;

	a->clientnum = clientnum;

	if (side == CTEST_SIDE_C)
	{
		ctest_set_host_client (&ctest_svmain_clients_c[clientnum]);
		return Host_Guard (ctest_svmain_invoke_serverinfo, a);
	}
	ctest_set_host_client (&ctest_svmain_clients_r[clientnum]);
	return quake_rs_sv_send_serverinfo (&ctest_svmain_clients_r[clientnum]);
}

static void ctest_svmain_invoke_connectclient (void *p)
{
	ctest_svmain_call_t *a = (ctest_svmain_call_t *)p;
	c_ref_SV_ConnectClient (a->clientnum);
}

int ctest_svmain_drive_connectclient (int side, int clientnum)
{
	ctest_svmain_call_t *a = &ctest_svmain_call;

	a->clientnum = clientnum;

	if (side == CTEST_SIDE_C)
	{
		ctest_set_host_client (&ctest_svmain_clients_c[clientnum]);
		return Host_Guard (ctest_svmain_invoke_connectclient, a);
	}
	ctest_set_host_client (&ctest_svmain_clients_r[clientnum]);
	return quake_rs_sv_connect_client (clientnum);
}

static void ctest_svmain_invoke_checknewclients (void *p)
{
	(void)p;
	c_ref_SV_CheckForNewClients ();
}

int ctest_svmain_drive_checknewclients (int side)
{
	if (side == CTEST_SIDE_C)
		return Host_Guard (ctest_svmain_invoke_checknewclients, NULL);
	return quake_rs_sv_check_for_new_clients ();
}

static void ctest_svmain_invoke_savespawnparms (void *p)
{
	(void)p;
	c_ref_SV_SaveSpawnparms ();
}

int ctest_svmain_drive_savespawnparms (int side)
{
	if (side == CTEST_SIDE_C)
		return Host_Guard (ctest_svmain_invoke_savespawnparms, NULL);
	return quake_rs_sv_save_spawnparms ();
}

void ctest_svmain_drive_cleardatagram (int side)
{
	if (side == CTEST_SIDE_C)
		c_ref_SV_ClearDatagram ();
	else
		SV_ClearDatagram ();
}

int ctest_svmain_drive_modelindex (int side, const char *name)
{
	if (side == CTEST_SIDE_C)
		return c_ref_SV_ModelIndex (name);
	return SV_ModelIndex (name);
}

void ctest_svmain_set_model_slot (int index, int marker)
{
	sv.models[index] = (qmodel_t *)(intptr_t)marker;
	c_ref_sv.models[index] = (qmodel_t *)(intptr_t)marker;
}

intptr_t ctest_svmain_modelforindex_raw (int side, int index)
{
	qmodel_t *m = side == CTEST_SIDE_C ? c_ref_SV_ModelForIndex (index) : SV_ModelForIndex (index);
	return (intptr_t)m;
}

/* -------------------------------------------------------------------------
 * The two commands sv_main.c registers are `static`, so the oracle side can
 * only be reached through the command table c_ref_SV_Init filled in. The Rust
 * side calls its core directly after tokenizing with the plain (Rust)
 * tokenizer.
 */

static void ctest_svmain_invoke_cmdexec (void *p)
{
	c_ref_Cmd_ExecuteString ((const char *)p, src_command);
}

int ctest_svmain_drive_protocol_f (int side, const char *cmdtext)
{
	if (side == CTEST_SIDE_C)
		return Host_Guard (ctest_svmain_invoke_cmdexec, (void *)(intptr_t)cmdtext);

	Cmd_TokenizeString (cmdtext);
	cmd_source = src_command;
	quake_rs_sv_protocol_f ();
	return 0;
}

typedef struct
{
	const char	*cmdtext;
	cmd_source_t src;
} ctest_svmain_pext_arg_t;

static void ctest_svmain_invoke_pext (void *p)
{
	ctest_svmain_pext_arg_t *a = (ctest_svmain_pext_arg_t *)p;
	c_ref_Cmd_ExecuteString (a->cmdtext, a->src);
}

int ctest_svmain_drive_pext_f (int side, const char *cmdtext, int from_client, int clientnum)
{
	ctest_svmain_pext_arg_t a;

	a.cmdtext = cmdtext;
	a.src = from_client ? src_client : src_command;

	if (side == CTEST_SIDE_C)
	{
		ctest_set_host_client (&ctest_svmain_clients_c[clientnum]);
		return Host_Guard (ctest_svmain_invoke_pext, &a);
	}

	ctest_set_host_client (&ctest_svmain_clients_r[clientnum]);
	Cmd_TokenizeString (cmdtext);
	cmd_source = a.src;
	return quake_rs_sv_pext_f ();
}

/* =========================================================================
 * SV_Init. The two sides register DIFFERENT cvar_t objects -- the oracle
 * registers the prelude-renamed c_ref_* copies its own translation units
 * define, the Rust port registers the plain twins stubs.c and this file own
 * -- so the table below is two columns. cvar_t has a single `next` field, so
 * neither registry may be left holding a struct when the other runs: the C
 * side runs first, is snapshotted, and every struct in both columns is
 * restored before the Rust side runs. That makes the pair one-shot per test
 * binary.
 */

/* The 23 registrations of sv_main.c:164-190, in source order. Each row is the
 * pair of cvar_t objects the two sides actually register: `rs` is the plain
 * twin the Rust port reads through quake-c-sys, `c` is the oracle's own copy.
 * sv_aim and pr_checkextension are not renamed by the prelude, so both
 * columns point at the one shared object there. */
typedef struct
{
	cvar_t *c;
	cvar_t *rs;
} ctest_svmain_cvar_pair_t;

static const ctest_svmain_cvar_pair_t ctest_svmain_init_cvars[] = {
	{&c_ref_sv_maxvelocity, &sv_maxvelocity},
	{&c_ref_sv_gravity, &sv_gravity},
	{&c_ref_sv_friction, &sv_friction},
	{&c_ref_sv_edgefriction, &sv_edgefriction},
	{&c_ref_sv_stopspeed, &sv_stopspeed},
	{&c_ref_sv_maxspeed, &sv_maxspeed},
	{&c_ref_sv_accelerate, &sv_accelerate},
	{&c_ref_sv_idealpitchscale, &sv_idealpitchscale},
	{&sv_aim, &sv_aim},
	{&c_ref_sv_nostep, &sv_nostep},
	{&c_ref_sv_freezenonclients, &sv_freezenonclients},
	{&c_ref_sv_gameplayfix_spawnbeforethinks, &sv_gameplayfix_spawnbeforethinks},
	{&c_ref_sv_gameplayfix_bouncedownslopes, &sv_gameplayfix_bouncedownslopes},
	{&c_ref_sv_gameplayfix_elevators, &sv_gameplayfix_elevators},
	{&c_ref_sv_fastpushmove, &sv_fastpushmove},
	{&c_ref_sv_pushgrid, &sv_pushgrid},
	{&c_ref_sv_analyticphysics, &sv_analyticphysics},
	{&pr_checkextension, &pr_checkextension},
	{&c_ref_sv_altnoclip, &sv_altnoclip},
	{&c_ref_sv_netsort, &sv_netsort},
	{&c_ref_sv_smoothplatformlerps, &sv_smoothplatformlerps},
	{&c_ref_sv_fte_recursivehullckeck, &sv_fte_recursivehullckeck},
	{&c_ref_sv_fte_createareanode, &sv_fte_createareanode},
};

#define CTEST_SVMAIN_INIT_CVARS ((int)(sizeof (ctest_svmain_init_cvars) / sizeof (ctest_svmain_init_cvars[0])))

typedef struct
{
	char		 name[64];
	char		 string[64];
	float		 value;
	unsigned int flags;
} ctest_svmain_cvar_snap_t;

static cvar_t					ctest_svmain_backup_c[CTEST_SVMAIN_INIT_CVARS];
static cvar_t					ctest_svmain_backup_rs[CTEST_SVMAIN_INIT_CVARS];
static ctest_svmain_cvar_snap_t ctest_svmain_cvar_snap[2][CTEST_SVMAIN_INIT_CVARS];
static char						ctest_svmain_serverinfo_snap[2][SERVER_INFO_STRING_SIZE];
static int						ctest_svmain_init_status[2];
static int						ctest_svmain_init_protocol_snap[2];
static unsigned int				ctest_svmain_init_pext_snap[2][2];
static int						ctest_svmain_init_done;

static cvar_t *ctest_svmain_side_cvar (int side, int i)
{
	return side == CTEST_SIDE_C ? ctest_svmain_init_cvars[i].c : ctest_svmain_init_cvars[i].rs;
}

/* stubs.c zeroes the plain twins of pr_checkextension (3441) and the two
 * sv_fte_* cvars (5372-5373), so they reach SV_Init with a NULL name and a
 * NULL string. Cvar_RegisterVariable's duplicate check would dereference
 * both. Seed them with world.c:33/35 and pr_ext.c's real initialisers so the
 * two sides start from the same values. */
static void ctest_svmain_seed_zeroed_cvars (void)
{
	if (!pr_checkextension.name)
	{
		pr_checkextension.name = "pr_checkextension";
		pr_checkextension.string = "1";
		pr_checkextension.flags = CVAR_NONE;
	}
	if (!sv_fte_recursivehullckeck.name)
	{
		sv_fte_recursivehullckeck.name = "sv_fte_recursivehullckeck";
		sv_fte_recursivehullckeck.string = "1";
		sv_fte_recursivehullckeck.flags = CVAR_NONE;
	}
	if (!sv_fte_createareanode.name)
	{
		sv_fte_createareanode.name = "sv_fte_createareanode";
		sv_fte_createareanode.string = "1";
		sv_fte_createareanode.flags = CVAR_NONE;
	}
}

static void ctest_svmain_snap_cvars (int side)
{
	int i;
	for (i = 0; i < CTEST_SVMAIN_INIT_CVARS; i++)
	{
		const cvar_t *v = ctest_svmain_side_cvar (side, i);
		q_strlcpy (ctest_svmain_cvar_snap[side][i].name, v->name ? v->name : "", sizeof (ctest_svmain_cvar_snap[side][i].name));
		q_strlcpy (ctest_svmain_cvar_snap[side][i].string, v->string ? v->string : "", sizeof (ctest_svmain_cvar_snap[side][i].string));
		ctest_svmain_cvar_snap[side][i].value = v->value;
		ctest_svmain_cvar_snap[side][i].flags = v->flags;
	}
	memcpy (ctest_svmain_serverinfo_snap[side], c_ref_svs.serverinfo, SERVER_INFO_STRING_SIZE);
	ctest_svmain_init_protocol_snap[side] = side == CTEST_SIDE_C ? c_ref_sv_protocol : sv_protocol;
	ctest_svmain_init_pext_snap[side][0] = side == CTEST_SIDE_C ? c_ref_sv_protocol_pext1 : sv_protocol_pext1;
	ctest_svmain_init_pext_snap[side][1] = side == CTEST_SIDE_C ? c_ref_sv_protocol_pext2 : sv_protocol_pext2;
}

static void ctest_svmain_invoke_init_c (void *p)
{
	(void)p;
	c_ref_SV_Init ();
}

/* Runs both sides of SV_Init exactly once per test binary and records the
 * observables. Returns 0.
 *
 * The C side runs first and is snapshotted before every cvar_t is restored,
 * because cvar_t has a single `next` field: leaving the oracle's chain in
 * place would make the Rust registry and the C registry fight over it. That
 * one-shot restore is why this is not a per-test fixture. */
int ctest_svmain_run_init_pair (void)
{
	int i;

	if (ctest_svmain_init_done)
		return 0;
	ctest_svmain_init_done = 1;

	ctest_progs_reset_vm (CTEST_SVMAIN_EDICTS, 64);
	ctest_svmain_seed_zeroed_cvars ();

	for (i = 0; i < CTEST_SVMAIN_INIT_CVARS; i++)
	{
		ctest_svmain_backup_c[i] = *ctest_svmain_init_cvars[i].c;
		ctest_svmain_backup_rs[i] = *ctest_svmain_init_cvars[i].rs;
	}

	/* svs.maxclients == 0 keeps Cvar_SetQuick's CVAR_SERVERINFO stufftext
	   loop empty on both sides; svs.serverinfo itself carries the
	   registration ORDER of the three CVAR_SERVERINFO cvars, which the
	   alphabetical cvar_vars chain cannot show. Both sides write it: the
	   oracle through cvar.c, the Rust registry through stubs.c's
	   CvarCmd_Glue_ServerinfoChanged, which also spells `svs` (== c_ref_svs
	   under the prelude). */
	memset (&c_ref_svs, 0, sizeof (c_ref_svs));
	memset (&svs, 0, sizeof (svs));
	ctest_clear_con_log ();

	ctest_svmain_init_status[CTEST_SIDE_C] = Host_Guard (ctest_svmain_invoke_init_c, NULL);
	ctest_svmain_snap_cvars (CTEST_SIDE_C);

	for (i = 0; i < CTEST_SVMAIN_INIT_CVARS; i++)
	{
		*ctest_svmain_init_cvars[i].c = ctest_svmain_backup_c[i];
		*ctest_svmain_init_cvars[i].rs = ctest_svmain_backup_rs[i];
	}
	ctest_svmain_seed_zeroed_cvars ();
	memset (&c_ref_svs, 0, sizeof (c_ref_svs));
	ctest_clear_con_log ();

	ctest_svmain_init_status[CTEST_SIDE_RS] = quake_rs_sv_init ();
	ctest_svmain_snap_cvars (CTEST_SIDE_RS);

	return 0;
}

int ctest_svmain_init_cvar_count (void)
{
	return CTEST_SVMAIN_INIT_CVARS;
}

const char *ctest_svmain_init_cvar_name (int side, int i)
{
	return ctest_svmain_cvar_snap[side][i].name;
}

const char *ctest_svmain_init_cvar_string (int side, int i)
{
	return ctest_svmain_cvar_snap[side][i].string;
}

float ctest_svmain_init_cvar_value (int side, int i)
{
	return ctest_svmain_cvar_snap[side][i].value;
}

unsigned int ctest_svmain_init_cvar_flags (int side, int i)
{
	return ctest_svmain_cvar_snap[side][i].flags;
}

const char *ctest_svmain_init_serverinfo (int side)
{
	return ctest_svmain_serverinfo_snap[side];
}

int ctest_svmain_init_status_get (int side)
{
	return ctest_svmain_init_status[side];
}

int ctest_svmain_init_protocol_get (int side)
{
	return ctest_svmain_init_protocol_snap[side];
}

unsigned int ctest_svmain_init_pext_get (int side, int which)
{
	return ctest_svmain_init_pext_snap[side][which];
}
