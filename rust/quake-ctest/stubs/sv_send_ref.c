/* Phase 7 M6 oracle TU for Quake/sv_send.c (task T6.2, the gate for T6.3).
 *
 * Two jobs:
 *  1. own a self-contained server fixture -- a qcvm + edict arena, a synthetic
 *     worldmodel, a `client_t` array published as `svs.clients`, a strings
 *     blob, an `sv` populated with protocol/precache state, and a `sizebuf_t`
 *     the writers emit into;
 *  2. expose one `ctest_svsend_drive_*_c` entry per subject that runs exactly
 *     one C-side call, so `tests/sv_send_differential.rs` can capture the
 *     observable record after it.
 *
 * Every oracle call site is spelled as an explicit `c_ref_*` name (the M5 wave
 * found that relying on c_ref_prelude.h's rename macros lets a `#undef`
 * elsewhere silently redirect an oracle call while the test still passes).
 * Data globals (`sv`, `svs`, `sv_netsort`, ...) are written through their
 * plain names, which the prelude maps onto the same oracle storage.
 *
 * T6.3 adds the `_rs` twin of each `ctest_svsend_drive_*_c` below, calling the
 * plain-named entry point; the Rust test's `Side` enum then grows a `Rust`
 * variant and every existing test becomes a real differential test unchanged.
 *
 * DEVIATIONS from the engine, all forced by stubs.c (which this file may not
 * edit) -- each is called out in the T6.2 report:
 *  - `NET_SendUnreliableMessage` (stubs.c:6959) discards the datagram, so the
 *    bytes SVFTE_WriteStats/SVFTE_WriteEntitiesToClient emit through
 *    SV_SendClientDatagram are NOT observable. Only their side effects and the
 *    byte COUNT (dev_stats.packetsize) are gated here.
 *  - `NET_QSocketGetSequenceOut` (stubs.c:6934) returns 0 unconditionally, so
 *    the outgoing sequence that selects client->frames[] cannot be varied.
 *  - `NET_QSocketGetTrueAddressString` (stubs.c:6922) returns "ctest", never
 *    "LOCAL", so MSGFTE_WriteEntityUpdate's LERP_BANDAID block (sv_send.c:266)
 *    always strips UF_UNUSED2 and never writes state->lerp. The bit is still
 *    gated where MSGFTE_DeltaCalcBits records it in pendingentities_bits.
 *  - `Mod_LeafPVS` (stubs.c:5729) returns one buffer for every leaf, so
 *    SV_AddToFatPVS' cross-leaf OR is idempotent here. The fixture instead
 *    picks numleafs so that `fatbytes` is not a multiple of 4, which turns
 *    sv_send.c:1060's `i < fatbytes - 3` truncation into an exact byte gate.
 */

/* ------------------------------------------------------------------------- */
/* fixture storage                                                            */

#define CTEST_SVSEND_MSGMAX	 70000
#define CTEST_SVSEND_CLIENTS 2
#define CTEST_SVSEND_STATES	 4
#define CTEST_SVSEND_EVALS	 4
#define CTEST_SVSEND_STRSLOTS 12
#define CTEST_SVSEND_STRSTRIDE 64
#define CTEST_SVSEND_PVSMAX	 64

/* 9 visible leafs => fatbytes = (9 + 31) / 8 = 5, deliberately not a multiple
 * of 4 (see the Mod_LeafPVS deviation above). */
#define CTEST_SVSEND_NUMLEAFS 9
#define CTEST_SVSEND_FATBYTES ((CTEST_SVSEND_NUMLEAFS + 31) / 8)

static byte		 ctest_svsend_msgbuf[CTEST_SVSEND_MSGMAX];
static sizebuf_t ctest_svsend_msg;

static client_t	   ctest_svsend_clients[CTEST_SVSEND_CLIENTS];
static globalvars_t ctest_svsend_globals;
static int		   ctest_svsend_socket; /* dummy non-NULL qsocket */

static entity_state_t ctest_svsend_states[CTEST_SVSEND_STATES];
static eval_t		  ctest_svsend_evals[CTEST_SVSEND_EVALS];

static char ctest_svsend_strings[CTEST_SVSEND_STRSLOTS * CTEST_SVSEND_STRSTRIDE];

static int	 ctest_svsend_statsi[MAX_CL_STATS];
static float ctest_svsend_statsf[MAX_CL_STATS];
static const char *ctest_svsend_statss[MAX_CL_STATS];

static byte	 ctest_svsend_pvscopy[CTEST_SVSEND_PVSMAX];
static byte *ctest_svsend_fatptr;
static byte ctest_svsend_leafpvs[CTEST_SVSEND_PVSMAX];
static int	ctest_svsend_pvslen;

/* the synthetic worldmodel: node0 splits on x=0, node1 (its front child)
 * splits on y=0. leaf 0 is CONTENTS_SOLID and sits behind x=0, so an origin
 * far behind the first plane reaches only solid and leaves fatpvs_any false. */
static mplane_t ctest_svsend_planes[2];
static mnode_t	ctest_svsend_nodes[2];
static mleaf_t	ctest_svsend_leafs[3];
static qmodel_t ctest_svsend_worldmodel;

static int ctest_svsend_extbase;

/* extfield selectors, shared with the Rust side by index */
enum
{
	CTEST_XF_ALPHA = 0,
	CTEST_XF_SCALE,
	CTEST_XF_COLORMOD,
	CTEST_XF_TAG_ENTITY,
	CTEST_XF_TAG_INDEX,
	CTEST_XF_MODELFLAGS,
	CTEST_XF_TRAILEFFECTNUM,
	CTEST_XF_EMITEFFECTNUM,
	CTEST_XF_ITEMS2,
	CTEST_XF_VIEWZOOM,
	CTEST_XF_NODRAWTOCLIENT,
	CTEST_XF_DRAWONLYTOCLIENT,
	CTEST_XF_COUNT
};

/* float offsets, relative to &ent->v, of each extfield. colormod occupies
 * three, so the table is not simply base + index. */
static const int ctest_svsend_xfslot[CTEST_XF_COUNT] = {0, 1, 2, 5, 6, 7, 8, 9, 10, 11, 12, 13};

static int ctest_svsend_xfoffset (int which)
{
	return ctest_svsend_extbase + ctest_svsend_xfslot[which];
}

/* ------------------------------------------------------------------------- */
/* helpers                                                                    */

extern void *ctest_progs_reset_vm (int max_edicts, int entityfields);
extern void	 ctest_progs_set_time (double t);
extern void	 ctest_progs_set_strings (char *blob, int size, int progsstrings);
extern void	 ctest_pf_set_leaf_pvs (unsigned char *pvs);
extern void	 ctest_set_host_client (client_t *c);
extern void	 ctest_clear_con_log (void);

/* sv_send.c declares these two itself (sv_send.c:27-28); no header does. The
 * prelude's rename macros point them at the oracle's storage. */
extern cvar_t sv_netsort;
extern cvar_t sv_smoothplatformlerps;

/* sv_send.c entry points that server.h does not declare. Written plain so the
 * prelude renames them; the call sites below still spell c_ref_* explicitly. */
void	  SV_CalcStats (client_t *client, int *statsi, float *statsf, const char **statss);
void	  MSG_WriteStaticOrBaseLine (sizebuf_t *buf, int idx, entity_state_t *state, unsigned int protocol_pext2, unsigned int protocol, unsigned int protocolflags);
void	  SV_AddToFatPVS (vec3_t org, mnode_t *node, qmodel_t *worldmodel);
byte	 *SV_FatPVS (vec3_t org, qmodel_t *worldmodel);
qboolean  SV_VisibleToClient (edict_t *client, edict_t *test, qmodel_t *worldmodel);
void	  SV_WriteEntitiesToClient (client_t *client, sizebuf_t *msg, size_t overflowsize);
void	  SV_PresendClientDatagram (client_t *client);
qboolean  SV_SendClientDatagram (client_t *client);

static edict_t *ctest_svsend_ed (int num)
{
	return EDICT_NUM (num);
}

static float *ctest_svsend_evfield (int num, int off)
{
	return (float *)&ctest_svsend_ed (num)->v + off;
}

/* ------------------------------------------------------------------------- */
/* reset                                                                      */

void ctest_svsend_reset (int num_edicts, int maxedicts)
{
	int i;

	for (i = 0; i < CTEST_SVSEND_CLIENTS; i++)
	{
		int s;
		/* release anything a previous test's SVFTE_SetupFrames left behind */
		client_t *clx = &ctest_svsend_clients[i];
		for (s = 0; s < MAX_CL_STATS; s++)
		{
			if (clx->oldstats_s[s])
				Mem_Free (clx->oldstats_s[s]);
		}
		if (clx->previousentities)
			Mem_Free (clx->previousentities);
		if (clx->pendingentities_bits)
			Mem_Free (clx->pendingentities_bits);
		while (clx->numframes > 0)
		{
			clx->numframes--;
			if (clx->frames[clx->numframes].ents)
				Mem_Free (clx->frames[clx->numframes].ents);
		}
		if (clx->frames)
			Mem_Free (clx->frames);
		memset (clx, 0, sizeof (*clx));
	}

	/* entityfields must cover entvars_t plus the extension fields the fixture
	 * hands out, so the arena stride leaves room for both. */
	ctest_svsend_extbase = (int)(sizeof (entvars_t) / 4);
	ctest_progs_reset_vm (maxedicts, ctest_svsend_extbase + 16);
	qcvm->num_edicts = num_edicts;
	ctest_progs_set_time (0.0);

	memset (ctest_svsend_strings, 0, sizeof (ctest_svsend_strings));
	ctest_progs_set_strings (ctest_svsend_strings, (int)sizeof (ctest_svsend_strings), (int)sizeof (ctest_svsend_strings));

	qcvm->extfields.alpha = -1;
	qcvm->extfields.scale = -1;
	qcvm->extfields.colormod = -1;
	qcvm->extfields.tag_entity = -1;
	qcvm->extfields.tag_index = -1;
	qcvm->extfields.modelflags = -1;
	qcvm->extfields.traileffectnum = -1;
	qcvm->extfields.emiteffectnum = -1;
	qcvm->extfields.items2 = -1;
	qcvm->extfields.viewzoom = -1;
	qcvm->extfields.nodrawtoclient = -1;
	qcvm->extfields.drawonlytoclient = -1;

	/* worldmodel */
	memset (ctest_svsend_planes, 0, sizeof (ctest_svsend_planes));
	memset (ctest_svsend_nodes, 0, sizeof (ctest_svsend_nodes));
	memset (ctest_svsend_leafs, 0, sizeof (ctest_svsend_leafs));
	memset (&ctest_svsend_worldmodel, 0, sizeof (ctest_svsend_worldmodel));

	ctest_svsend_planes[0].normal[0] = 1.0f;
	ctest_svsend_planes[0].dist = 0.0f;
	ctest_svsend_planes[1].normal[1] = 1.0f;
	ctest_svsend_planes[1].dist = 0.0f;

	ctest_svsend_leafs[0].contents = CONTENTS_SOLID;
	ctest_svsend_leafs[1].contents = CONTENTS_EMPTY;
	ctest_svsend_leafs[2].contents = CONTENTS_WATER;

	ctest_svsend_nodes[0].contents = 0;
	ctest_svsend_nodes[0].plane = &ctest_svsend_planes[0];
	ctest_svsend_nodes[0].children[0] = &ctest_svsend_nodes[1];
	ctest_svsend_nodes[0].children[1] = (mnode_t *)&ctest_svsend_leafs[0];

	ctest_svsend_nodes[1].contents = 0;
	ctest_svsend_nodes[1].plane = &ctest_svsend_planes[1];
	ctest_svsend_nodes[1].children[0] = (mnode_t *)&ctest_svsend_leafs[1];
	ctest_svsend_nodes[1].children[1] = (mnode_t *)&ctest_svsend_leafs[2];

	ctest_svsend_worldmodel.nodes = ctest_svsend_nodes;
	ctest_svsend_worldmodel.numleafs = CTEST_SVSEND_NUMLEAFS;
	qcvm->worldmodel = &ctest_svsend_worldmodel;

	memset (ctest_svsend_leafpvs, 0, sizeof (ctest_svsend_leafpvs));
	ctest_pf_set_leaf_pvs (ctest_svsend_leafpvs);

	memset (&ctest_svsend_globals, 0, sizeof (ctest_svsend_globals));
	pr_global_struct = &ctest_svsend_globals;

	memset (ctest_svsend_states, 0, sizeof (ctest_svsend_states));
	memset (ctest_svsend_evals, 0, sizeof (ctest_svsend_evals));
	memset (ctest_svsend_statsi, 0, sizeof (ctest_svsend_statsi));
	memset (ctest_svsend_statsf, 0, sizeof (ctest_svsend_statsf));
	memset ((void *)ctest_svsend_statss, 0, sizeof (ctest_svsend_statss));
	memset (ctest_svsend_pvscopy, 0, sizeof (ctest_svsend_pvscopy));
	ctest_svsend_pvslen = 0;

	/* sv/svs: only the fields sv_send.c actually reads are populated. The
	 * datagram sizebufs have to point at their own backing arrays or
	 * SV_SendClientDatagram's SZ_Write of sv.datagram walks a NULL. */
	memset (&sv, 0, sizeof (sv));
	sv.active = true;
	sv.state = ss_active;
	sv.protocol = PROTOCOL_FITZQUAKE;
	sv.protocolflags = 0;
	sv.effectsmask = 0xffffffffu;
	sv.numcustomstats = 0;
	sv.datagram.data = sv.datagram_buf;
	sv.datagram.maxsize = sizeof (sv.datagram_buf);
	sv.datagram.cursize = 0;
	sv.reliable_datagram.data = sv.reliable_datagram_buf;
	sv.reliable_datagram.maxsize = sizeof (sv.reliable_datagram_buf);
	sv.signon.data = sv.signon_buf;
	sv.signon.maxsize = sizeof (sv.signon_buf);

	for (i = 0; i < CTEST_SVSEND_CLIENTS; i++)
	{
		client_t *clx = &ctest_svsend_clients[i];
		clx->active = true;
		clx->spawned = true;
		clx->netconnection = (struct qsocket_s *)&ctest_svsend_socket;
		clx->message.data = clx->msgbuf;
		clx->message.maxsize = sizeof (clx->msgbuf);
		clx->datagram.data = clx->datagram_buf;
		clx->datagram.maxsize = sizeof (clx->datagram_buf);
		clx->limit_entities = 8192;
		clx->limit_models = MAX_MODELS;
		clx->limit_sounds = 2048;
		clx->limit_unreliable = MAX_DATAGRAM;
		clx->limit_reliable = MAX_MSGLEN;
		clx->edict = (num_edicts > i + 1) ? ctest_svsend_ed (i + 1) : NULL;
	}
	svs.clients = ctest_svsend_clients;
	svs.maxclients = CTEST_SVSEND_CLIENTS;
	svs.maxclientslimit = CTEST_SVSEND_CLIENTS;
	ctest_set_host_client (&ctest_svsend_clients[0]);

	ctest_svsend_msg.data = ctest_svsend_msgbuf;
	ctest_svsend_msg.maxsize = CTEST_SVSEND_MSGMAX;
	ctest_svsend_msg.cursize = 0;
	ctest_svsend_msg.allowoverflow = false;
	ctest_svsend_msg.overflowed = false;
	memset (ctest_svsend_msgbuf, 0, sizeof (ctest_svsend_msgbuf));

	sv_netsort.value = 0.0f;
	sv_smoothplatformlerps.value = 0.0f;
	isDedicated = false;
	realtime = 0.0;
	memset (&dev_stats, 0, sizeof (dev_stats));
	memset (&dev_peakstats, 0, sizeof (dev_peakstats));
	memset (&dev_overflows, 0, sizeof (dev_overflows));
	ctest_clear_con_log ();
}

/* ------------------------------------------------------------------------- */
/* fixture setters                                                            */

void ctest_svsend_set_globals (float serverflags)
{
	ctest_svsend_globals.serverflags = serverflags;
}

void ctest_svsend_set_cvars (float netsort, float smoothlerps, int dedicated, double now, double overflowtime)
{
	sv_netsort.value = netsort;
	sv_smoothplatformlerps.value = smoothlerps;
	isDedicated = dedicated ? true : false;
	realtime = now;
	dev_overflows.packetsize = overflowtime;
}

void ctest_svsend_set_server (unsigned int protocol, unsigned int protocolflags, int effectsmask)
{
	sv.protocol = protocol;
	sv.protocolflags = protocolflags;
	sv.effectsmask = effectsmask;
}

void ctest_svsend_set_vm (int num_edicts, double time)
{
	qcvm->num_edicts = num_edicts;
	ctest_progs_set_time (time);
}

/* Enables the extension fields named by `mask` (bit CTEST_XF_*); the rest keep
 * offset -1, which is what makes GetEdictFieldValue return NULL. */
void ctest_svsend_set_extfields (unsigned int mask)
{
	qcvm->extfields.alpha = (mask & (1u << CTEST_XF_ALPHA)) ? ctest_svsend_xfoffset (CTEST_XF_ALPHA) : -1;
	qcvm->extfields.scale = (mask & (1u << CTEST_XF_SCALE)) ? ctest_svsend_xfoffset (CTEST_XF_SCALE) : -1;
	qcvm->extfields.colormod = (mask & (1u << CTEST_XF_COLORMOD)) ? ctest_svsend_xfoffset (CTEST_XF_COLORMOD) : -1;
	qcvm->extfields.tag_entity = (mask & (1u << CTEST_XF_TAG_ENTITY)) ? ctest_svsend_xfoffset (CTEST_XF_TAG_ENTITY) : -1;
	qcvm->extfields.tag_index = (mask & (1u << CTEST_XF_TAG_INDEX)) ? ctest_svsend_xfoffset (CTEST_XF_TAG_INDEX) : -1;
	qcvm->extfields.modelflags = (mask & (1u << CTEST_XF_MODELFLAGS)) ? ctest_svsend_xfoffset (CTEST_XF_MODELFLAGS) : -1;
	qcvm->extfields.traileffectnum = (mask & (1u << CTEST_XF_TRAILEFFECTNUM)) ? ctest_svsend_xfoffset (CTEST_XF_TRAILEFFECTNUM) : -1;
	qcvm->extfields.emiteffectnum = (mask & (1u << CTEST_XF_EMITEFFECTNUM)) ? ctest_svsend_xfoffset (CTEST_XF_EMITEFFECTNUM) : -1;
	qcvm->extfields.items2 = (mask & (1u << CTEST_XF_ITEMS2)) ? ctest_svsend_xfoffset (CTEST_XF_ITEMS2) : -1;
	qcvm->extfields.viewzoom = (mask & (1u << CTEST_XF_VIEWZOOM)) ? ctest_svsend_xfoffset (CTEST_XF_VIEWZOOM) : -1;
	qcvm->extfields.nodrawtoclient = (mask & (1u << CTEST_XF_NODRAWTOCLIENT)) ? ctest_svsend_xfoffset (CTEST_XF_NODRAWTOCLIENT) : -1;
	qcvm->extfields.drawonlytoclient = (mask & (1u << CTEST_XF_DRAWONLYTOCLIENT)) ? ctest_svsend_xfoffset (CTEST_XF_DRAWONLYTOCLIENT) : -1;
}

void ctest_svsend_set_ext (int num, int which, float x, float y, float z)
{
	float *p = ctest_svsend_evfield (num, ctest_svsend_xfoffset (which));
	p[0] = x;
	if (which == CTEST_XF_COLORMOD)
	{
		p[1] = y;
		p[2] = z;
	}
}

/* Writes an extension field as an edict reference (eval_t::edict is a prog
 * offset, not an index), which is what nodrawtoclient / drawonlytoclient /
 * tag_entity are compared against. */
void ctest_svsend_set_ext_edict (int num, int which, int targetnum)
{
	eval_t *p = (eval_t *)ctest_svsend_evfield (num, ctest_svsend_xfoffset (which));
	p->edict = (targetnum >= 0) ? (int)EDICT_TO_PROG (ctest_svsend_ed (targetnum)) : 0;
}

/* entvars scalar selectors */
enum
{
	CTEST_EV_MOVETYPE = 0,
	CTEST_EV_MODELINDEX,
	CTEST_EV_FRAME,
	CTEST_EV_COLORMAP,
	CTEST_EV_SKIN,
	CTEST_EV_EFFECTS,
	CTEST_EV_FLAGS,
	CTEST_EV_HEALTH,
	CTEST_EV_CURRENTAMMO,
	CTEST_EV_ARMORVALUE,
	CTEST_EV_WEAPONFRAME,
	CTEST_EV_AMMO_SHELLS,
	CTEST_EV_AMMO_NAILS,
	CTEST_EV_AMMO_ROCKETS,
	CTEST_EV_AMMO_CELLS,
	CTEST_EV_WEAPON,
	CTEST_EV_ITEMS,
	CTEST_EV_IDEALPITCH,
	CTEST_EV_NEXTTHINK,
	CTEST_EV_TOUCH,
	CTEST_EV_MODEL,
	CTEST_EV_WEAPONMODEL,
	CTEST_EV_FRAGS
};

void ctest_svsend_set_ev (int num, int field, float v)
{
	entvars_t *ev = &ctest_svsend_ed (num)->v;
	switch (field)
	{
	case CTEST_EV_MOVETYPE:
		ev->movetype = v;
		break;
	case CTEST_EV_MODELINDEX:
		ev->modelindex = v;
		break;
	case CTEST_EV_FRAME:
		ev->frame = v;
		break;
	case CTEST_EV_COLORMAP:
		ev->colormap = v;
		break;
	case CTEST_EV_SKIN:
		ev->skin = v;
		break;
	case CTEST_EV_EFFECTS:
		ev->effects = v;
		break;
	case CTEST_EV_FLAGS:
		ev->flags = v;
		break;
	case CTEST_EV_HEALTH:
		ev->health = v;
		break;
	case CTEST_EV_CURRENTAMMO:
		ev->currentammo = v;
		break;
	case CTEST_EV_ARMORVALUE:
		ev->armorvalue = v;
		break;
	case CTEST_EV_WEAPONFRAME:
		ev->weaponframe = v;
		break;
	case CTEST_EV_AMMO_SHELLS:
		ev->ammo_shells = v;
		break;
	case CTEST_EV_AMMO_NAILS:
		ev->ammo_nails = v;
		break;
	case CTEST_EV_AMMO_ROCKETS:
		ev->ammo_rockets = v;
		break;
	case CTEST_EV_AMMO_CELLS:
		ev->ammo_cells = v;
		break;
	case CTEST_EV_WEAPON:
		ev->weapon = v;
		break;
	case CTEST_EV_ITEMS:
		ev->items = v;
		break;
	case CTEST_EV_IDEALPITCH:
		ev->idealpitch = v;
		break;
	case CTEST_EV_NEXTTHINK:
		ev->nextthink = v;
		break;
	case CTEST_EV_TOUCH:
		ev->touch = (func_t)(int)v;
		break;
	case CTEST_EV_MODEL:
		ev->model = (string_t)(int)v;
		break;
	case CTEST_EV_WEAPONMODEL:
		ev->weaponmodel = (string_t)(int)v;
		break;
	case CTEST_EV_FRAGS:
		ev->frags = v;
		break;
	default:
		break;
	}
}

/* entvars vector selectors */
enum
{
	CTEST_EVV_ORIGIN = 0,
	CTEST_EVV_ANGLES,
	CTEST_EVV_VELOCITY,
	CTEST_EVV_VIEW_OFS,
	CTEST_EVV_PUNCHANGLE,
	CTEST_EVV_ABSMIN,
	CTEST_EVV_ABSMAX,
	CTEST_EVV_V_ANGLE
};

void ctest_svsend_set_evv (int num, int field, float x, float y, float z)
{
	entvars_t *ev = &ctest_svsend_ed (num)->v;
	float	  *p;
	switch (field)
	{
	case CTEST_EVV_ORIGIN:
		p = ev->origin;
		break;
	case CTEST_EVV_ANGLES:
		p = ev->angles;
		break;
	case CTEST_EVV_VELOCITY:
		p = ev->velocity;
		break;
	case CTEST_EVV_VIEW_OFS:
		p = ev->view_ofs;
		break;
	case CTEST_EVV_PUNCHANGLE:
		p = ev->punchangle;
		break;
	case CTEST_EVV_ABSMIN:
		p = ev->absmin;
		break;
	case CTEST_EVV_ABSMAX:
		p = ev->absmax;
		break;
	case CTEST_EVV_V_ANGLE:
		p = ev->v_angle;
		break;
	default:
		return;
	}
	p[0] = x;
	p[1] = y;
	p[2] = z;
}

void ctest_svsend_set_ed (int num, int alpha, int sendinterval, int sendinterval_default, float lastthink, float px, float py, float pz)
{
	edict_t *ed = ctest_svsend_ed (num);
	ed->alpha = (unsigned char)alpha;
	ed->sendinterval = sendinterval ? true : false;
	ed->sendinterval_default = sendinterval_default ? true : false;
	ed->lastthink = lastthink;
	ed->predthinkpos[0] = px;
	ed->predthinkpos[1] = py;
	ed->predthinkpos[2] = pz;
	ed->free = false;
}

void ctest_svsend_set_leafs (int num, const int *leafs, int count)
{
	edict_t *ed = ctest_svsend_ed (num);
	int		 i;
	if (count > MAX_ENT_LEAFS)
		count = MAX_ENT_LEAFS;
	for (i = 0; i < count; i++)
		ed->leafnums[i] = leafs[i];
	ed->num_leafs = (unsigned int)count;
}

/* ------------------------------------------------------------------------- */
/* entity_state_t <-> flat word array (27 words, see the Rust side's StateWords) */

#define CTEST_SVSEND_STATEWORDS 27

static void ctest_svsend_pack (const entity_state_t *s, int *w)
{
	memcpy (&w[0], &s->origin[0], 4);
	memcpy (&w[1], &s->origin[1], 4);
	memcpy (&w[2], &s->origin[2], 4);
	memcpy (&w[3], &s->angles[0], 4);
	memcpy (&w[4], &s->angles[1], 4);
	memcpy (&w[5], &s->angles[2], 4);
	w[6] = s->modelindex;
	w[7] = s->frame;
	w[8] = (int)s->effects;
	w[9] = s->colormap;
	w[10] = s->skin;
	w[11] = s->scale;
	w[12] = s->pmovetype;
	w[13] = s->traileffectnum;
	w[14] = s->emiteffectnum;
	w[15] = s->velocity[0];
	w[16] = s->velocity[1];
	w[17] = s->velocity[2];
	w[18] = s->eflags;
	w[19] = s->tagindex;
	w[20] = s->tagentity;
	w[21] = s->colormod[0];
	w[22] = s->colormod[1];
	w[23] = s->colormod[2];
	w[24] = s->alpha;
	w[25] = (int)s->solidsize;
#ifdef LERP_BANDAID
	w[26] = s->lerp;
#else
	w[26] = 0;
#endif
}

static void ctest_svsend_unpack (entity_state_t *s, const int *w)
{
	memset (s, 0, sizeof (*s));
	memcpy (&s->origin[0], &w[0], 4);
	memcpy (&s->origin[1], &w[1], 4);
	memcpy (&s->origin[2], &w[2], 4);
	memcpy (&s->angles[0], &w[3], 4);
	memcpy (&s->angles[1], &w[4], 4);
	memcpy (&s->angles[2], &w[5], 4);
	s->modelindex = (unsigned short)w[6];
	s->frame = (unsigned short)w[7];
	s->effects = (unsigned int)w[8];
	s->colormap = (unsigned char)w[9];
	s->skin = (unsigned char)w[10];
	s->scale = (unsigned char)w[11];
	s->pmovetype = (unsigned char)w[12];
	s->traileffectnum = (unsigned short)w[13];
	s->emiteffectnum = (unsigned short)w[14];
	s->velocity[0] = (short)w[15];
	s->velocity[1] = (short)w[16];
	s->velocity[2] = (short)w[17];
	s->eflags = (unsigned char)w[18];
	s->tagindex = (unsigned char)w[19];
	s->tagentity = (unsigned short)w[20];
	s->colormod[0] = (unsigned char)w[21];
	s->colormod[1] = (unsigned char)w[22];
	s->colormod[2] = (unsigned char)w[23];
	s->alpha = (unsigned char)w[24];
	s->solidsize = (unsigned int)w[25];
#ifdef LERP_BANDAID
	s->lerp = (unsigned short)w[26];
#endif
}

int ctest_svsend_statewords (void)
{
	return CTEST_SVSEND_STATEWORDS;
}

void ctest_svsend_set_state (int slot, const int *w)
{
	ctest_svsend_unpack (&ctest_svsend_states[slot], w);
}

void ctest_svsend_get_state (int slot, int *w)
{
	ctest_svsend_pack (&ctest_svsend_states[slot], w);
}

void ctest_svsend_set_baseline (int num, const int *w)
{
	ctest_svsend_unpack (&ctest_svsend_ed (num)->baseline, w);
}

/* ------------------------------------------------------------------------- */
/* strings / precaches                                                        */

int ctest_svsend_set_string (int slot, const char *s)
{
	int	  off = slot * CTEST_SVSEND_STRSTRIDE;
	char *dst = &ctest_svsend_strings[off];
	int	  i;
	for (i = 0; i < CTEST_SVSEND_STRSTRIDE - 1 && s[i]; i++)
		dst[i] = s[i];
	dst[i] = 0;
	return off;
}

void ctest_svsend_precache_model (int idx, int slot)
{
	sv.model_precache[idx] = (slot < 0) ? NULL : &ctest_svsend_strings[slot * CTEST_SVSEND_STRSTRIDE];
}

void ctest_svsend_set_leafpvs (const unsigned char *bytes, int count)
{
	int i;
	memset (ctest_svsend_leafpvs, 0, sizeof (ctest_svsend_leafpvs));
	if (count > CTEST_SVSEND_PVSMAX)
		count = CTEST_SVSEND_PVSMAX;
	for (i = 0; i < count; i++)
		ctest_svsend_leafpvs[i] = bytes[i];
}

/* ------------------------------------------------------------------------- */
/* client setters                                                             */

void ctest_svsend_set_client (int clx, int spawned, unsigned int pext2, unsigned int limit_entities, unsigned int limit_models, unsigned int limit_unreliable,
							  int edictnum)
{
	client_t *c = &ctest_svsend_clients[clx];
	c->spawned = spawned ? true : false;
	c->protocol_pext2 = pext2;
	c->limit_entities = limit_entities;
	c->limit_models = limit_models;
	c->limit_unreliable = limit_unreliable;
	c->edict = (edictnum >= 0) ? ctest_svsend_ed (edictnum) : NULL;
}

void ctest_svsend_set_lastmovemessage (int clx, int v)
{
	ctest_svsend_clients[clx].lastmovemessage = v;
}

void ctest_svsend_set_pending (int clx, unsigned int entnum, unsigned int bits)
{
	client_t *c = &ctest_svsend_clients[clx];
	if (entnum < c->numpendingentities)
		c->pendingentities_bits[entnum] = bits;
}

void ctest_svsend_set_lastack (int clx, int seq)
{
	ctest_svsend_clients[clx].lastacksequence = seq;
}

/* Directly seeds a client's tracked frame, so the SVFTE_Ack / SVFTE_DroppedFrame
 * resend paths can be driven without needing a settable outgoing sequence. */
void ctest_svsend_seed_frame (int clx, int slot, int sequence, float timestamp, const unsigned int *rsnum, const unsigned int *rsstr, const unsigned int *ents,
							  int numents)
{
	client_t			*c = &ctest_svsend_clients[clx];
	struct deltaframe_s *f;
	int					 i;
	if ((size_t)slot >= c->numframes)
		return;
	f = &c->frames[slot];
	f->sequence = sequence;
	f->timestamp = timestamp;
	for (i = 0; i < MAX_CL_STATS / 32; i++)
	{
		f->resendstatsnum[i] = rsnum[i];
		f->resendstatsstr[i] = rsstr[i];
	}
	if (numents > f->maxents)
	{
		f->maxents = numents;
		f->ents = Mem_Realloc (f->ents, sizeof (*f->ents) * f->maxents);
	}
	for (i = 0; i < numents; i++)
	{
		f->ents[i].num = ents[i * 2 + 0];
		f->ents[i].ebits = ents[i * 2 + 1];
		f->ents[i].csqcbits = 0;
	}
	f->numents = numents;
}

void ctest_svsend_set_resendstats (int clx, const unsigned int *rsnum, const unsigned int *rsstr)
{
	client_t *c = &ctest_svsend_clients[clx];
	int		  i;
	for (i = 0; i < MAX_CL_STATS / 32; i++)
	{
		c->resendstatsnum[i] = rsnum[i];
		c->resendstatsstr[i] = rsstr[i];
	}
}

void ctest_svsend_set_evalslot (int slot, unsigned int w0, unsigned int w1, unsigned int w2)
{
	unsigned int *p = (unsigned int *)&ctest_svsend_evals[slot];
	p[0] = w0;
	p[1] = w1;
	p[2] = w2;
}

void ctest_svsend_add_customstat (int idx, int type, int which_extfield, int eval_slot)
{
	size_t n = sv.numcustomstats;
	if (n >= MAX_CL_STATS * 2)
		return;
	sv.customstats[n].idx = idx;
	sv.customstats[n].type = type;
	sv.customstats[n].fld = (which_extfield >= 0) ? ctest_svsend_xfoffset (which_extfield) : -1;
	sv.customstats[n].ptr = (eval_slot >= 0) ? &ctest_svsend_evals[eval_slot] : NULL;
	sv.numcustomstats = n + 1;
}

/* ------------------------------------------------------------------------- */
/* message buffer                                                             */

void ctest_svsend_msg_reset (int maxsize)
{
	ctest_svsend_msg.data = ctest_svsend_msgbuf;
	ctest_svsend_msg.maxsize = (maxsize > 0 && maxsize < CTEST_SVSEND_MSGMAX) ? maxsize : CTEST_SVSEND_MSGMAX;
	ctest_svsend_msg.cursize = 0;
	ctest_svsend_msg.allowoverflow = false;
	ctest_svsend_msg.overflowed = false;
	memset (ctest_svsend_msgbuf, 0, sizeof (ctest_svsend_msgbuf));
}

int ctest_svsend_msg_copy (unsigned char *out, int max)
{
	int n = ctest_svsend_msg.cursize;
	if (n > max)
		n = max;
	memcpy (out, ctest_svsend_msgbuf, (size_t)n);
	return ctest_svsend_msg.cursize;
}

/* ------------------------------------------------------------------------- */
/* observation                                                                */

int ctest_svsend_pending_copy (int clx, unsigned int *out, int max)
{
	client_t *c = &ctest_svsend_clients[clx];
	int		  n = (int)c->numpendingentities;
	int		  i;
	if (n > max)
		n = max;
	for (i = 0; i < n; i++)
		out[i] = c->pendingentities_bits[i];
	return (int)c->numpendingentities;
}

int ctest_svsend_prev_count (int clx)
{
	return (int)ctest_svsend_clients[clx].numpreviousentities;
}

int ctest_svsend_prev_get (int clx, int i, int *w)
{
	client_t *c = &ctest_svsend_clients[clx];
	if ((size_t)i >= c->numpreviousentities)
		return -1;
	ctest_svsend_pack (&c->previousentities[i].state, w);
	return (int)c->previousentities[i].num;
}

int ctest_svsend_numframes (int clx)
{
	return (int)ctest_svsend_clients[clx].numframes;
}

void ctest_svsend_frame_get (int clx, int slot, int *seq, unsigned int *timestamp_bits, int *numents, unsigned int *rsnum, unsigned int *rsstr)
{
	client_t			*c = &ctest_svsend_clients[clx];
	struct deltaframe_s *f = &c->frames[slot];
	int					 i;
	*seq = f->sequence;
	memcpy (timestamp_bits, &f->timestamp, 4);
	*numents = f->numents;
	for (i = 0; i < MAX_CL_STATS / 32; i++)
	{
		rsnum[i] = f->resendstatsnum[i];
		rsstr[i] = f->resendstatsstr[i];
	}
}

int ctest_svsend_frame_ent (int clx, int slot, int i, unsigned int *num, unsigned int *ebits, unsigned int *csqcbits)
{
	struct deltaframe_s *f = &ctest_svsend_clients[clx].frames[slot];
	if (i >= f->numents)
		return 0;
	*num = f->ents[i].num;
	*ebits = f->ents[i].ebits;
	*csqcbits = f->ents[i].csqcbits;
	return 1;
}

void ctest_svsend_client_get (int clx, int *lastack, unsigned int *snapshotresume, unsigned int *numpending, unsigned int *rsnum, unsigned int *rsstr,
							  int *num_pings, unsigned int *pings)
{
	client_t *c = &ctest_svsend_clients[clx];
	int		  i;
	*lastack = c->lastacksequence;
	*snapshotresume = c->snapshotresume;
	*numpending = (unsigned int)c->numpendingentities;
	for (i = 0; i < MAX_CL_STATS / 32; i++)
	{
		rsnum[i] = c->resendstatsnum[i];
		rsstr[i] = c->resendstatsstr[i];
	}
	*num_pings = c->num_pings;
	for (i = 0; i < NUM_PING_TIMES; i++)
		memcpy (&pings[i], &c->ping_times[i], 4);
}

void ctest_svsend_oldstats_get (int clx, int idx, int *si, unsigned int *sf, const char **ss)
{
	client_t *c = &ctest_svsend_clients[clx];
	*si = c->oldstats_i[idx];
	memcpy (sf, &c->oldstats_f[idx], 4);
	*ss = c->oldstats_s[idx];
}

void ctest_svsend_stats_get (int idx, int *si, unsigned int *sf, const char **ss)
{
	*si = ctest_svsend_statsi[idx];
	memcpy (sf, &ctest_svsend_statsf[idx], 4);
	*ss = ctest_svsend_statss[idx];
}

int ctest_svsend_pvs_copy (unsigned char *out, int max)
{
	int n = ctest_svsend_pvslen;
	int i;
	if (n > max)
		n = max;
	for (i = 0; i < n; i++)
		out[i] = ctest_svsend_pvscopy[i];
	return ctest_svsend_pvslen;
}

void ctest_svsend_devstats_get (int *cur, int *peak)
{
	*cur = dev_stats.packetsize;
	*peak = dev_peakstats.packetsize;
}

int ctest_svsend_fatbytes (void)
{
	return CTEST_SVSEND_FATBYTES;
}

/* ------------------------------------------------------------------------- */
/* C-side drivers. One oracle call each; T6.3 adds the `_rs` twins.            */

void ctest_svsend_drive_baseline_c (int idx, int slot, unsigned int pext2, unsigned int protocol, unsigned int protocolflags)
{
	c_ref_MSG_WriteStaticOrBaseLine (&ctest_svsend_msg, idx, &ctest_svsend_states[slot], pext2, protocol, protocolflags);
}

void ctest_svsend_drive_buildstate_c (int ednum, int slot)
{
	c_ref_SV_BuildEntityState (ctest_svsend_ed (ednum), &ctest_svsend_states[slot]);
}

void ctest_svsend_drive_calcstats_c (int clx)
{
	c_ref_SV_CalcStats (&ctest_svsend_clients[clx], ctest_svsend_statsi, ctest_svsend_statsf, ctest_svsend_statss);
}

void ctest_svsend_drive_setupframes_c (int clx)
{
	c_ref_SVFTE_SetupFrames (&ctest_svsend_clients[clx]);
}

void ctest_svsend_drive_destroyframes_c (int clx)
{
	c_ref_SVFTE_DestroyFrames (&ctest_svsend_clients[clx]);
}

void ctest_svsend_drive_ack_c (int clx, int sequence)
{
	ctest_set_host_client (&ctest_svsend_clients[clx]);
	c_ref_SVFTE_Ack (&ctest_svsend_clients[clx], sequence);
}

int ctest_svsend_drive_fatpvs_c (float x, float y, float z, int want)
{
	vec3_t org;
	byte  *p;
	int	   i;
	org[0] = x;
	org[1] = y;
	org[2] = z;
	p = c_ref_SV_FatPVS (org, &ctest_svsend_worldmodel);
	ctest_svsend_fatptr = p;
	if (want > CTEST_SVSEND_PVSMAX)
		want = CTEST_SVSEND_PVSMAX;
	for (i = 0; i < want; i++)
		ctest_svsend_pvscopy[i] = p[i];
	ctest_svsend_pvslen = want;
	return want;
}

/* Must follow a drive_fatpvs call: SV_AddToFatPVS accumulates into the
 * fatpvs/fatbytes statics that SV_FatPVS sets up. */
int ctest_svsend_drive_addtofatpvs_c (float x, float y, float z, int nodeidx, int want)
{
	vec3_t org;
	int	   i;
	if (!ctest_svsend_fatptr)
		return -1;
	org[0] = x;
	org[1] = y;
	org[2] = z;
	c_ref_SV_AddToFatPVS (org, &ctest_svsend_nodes[nodeidx], &ctest_svsend_worldmodel);
	/* the accumulate lands in the fatpvs static, which is the buffer the last
	 * SV_FatPVS returned; a second SV_FatPVS would wipe it before we looked. */
	if (want > CTEST_SVSEND_PVSMAX)
		want = CTEST_SVSEND_PVSMAX;
	for (i = 0; i < want; i++)
		ctest_svsend_pvscopy[i] = ctest_svsend_fatptr[i];
	ctest_svsend_pvslen = want;
	return want;
}

int ctest_svsend_drive_visible_c (int clientnum, int testnum)
{
	return c_ref_SV_VisibleToClient (ctest_svsend_ed (clientnum), ctest_svsend_ed (testnum), &ctest_svsend_worldmodel) ? 1 : 0;
}

void ctest_svsend_drive_writeents_c (int clx, int overflowsize)
{
	ctest_set_host_client (&ctest_svsend_clients[clx]);
	c_ref_SV_WriteEntitiesToClient (&ctest_svsend_clients[clx], &ctest_svsend_msg, (size_t)overflowsize);
}

void ctest_svsend_drive_presend_c (int clx)
{
	ctest_set_host_client (&ctest_svsend_clients[clx]);
	c_ref_SV_PresendClientDatagram (&ctest_svsend_clients[clx]);
}

int ctest_svsend_drive_senddatagram_c (int clx)
{
	return c_ref_SV_SendClientDatagram (&ctest_svsend_clients[clx]) ? 1 : 0;
}

/* Plain-named forwarding trampoline for the Rust sv_user port (T6.4).
 *
 * Quake/sv_user.c:605 calls SVFTE_Ack, so the Rust port at
 * rust/quake-capi/src/sv_user.rs:841 declares it as a plain C extern -- which
 * is correct for the real Meson build, where Quake/sv_send.c (or, after T6.3,
 * Quake/sv_send_glue.c) supplies it. In the ctest link the only definition is
 * the prelude-renamed c_ref_SVFTE_Ack, so the plain name is unresolved and
 * every quake-ctest target fails to link.
 *
 * Both sides of every differential reach the same C implementation through
 * this, so it does not weaken a gate. Safe against the M5 wave's #undef
 * hazard specifically because this file's own call site (line 941) spells
 * c_ref_SVFTE_Ack explicitly rather than relying on the rename; the #undef is
 * also last in the file, so nothing above it is affected.
 */
#undef SVFTE_Ack
void SVFTE_Ack (client_t *client, int sequence)
{
	c_ref_SVFTE_Ack (client, sequence);
}

/* =========================================================================
 * T6.3: the Rust side of the differential.
 *
 * Everything below this line is compiled with the prelude's rename macros
 * still live, so every oracle symbol is spelled c_ref_* by hand (the rule
 * this file's header states) and every plain-named object the Rust port
 * reads is #undef'd first.
 *
 * Storage split. The prelude maps `sv`, `svs`, `sv_netsort` and
 * `sv_smoothplatformlerps` onto the oracle's copies (c_ref_*, defined by
 * Quake/sv_main.c and Quake/sv_send.c in C_SOURCES); the Rust port reads the
 * plain twins, which stubs/sv_main_ref.c defines. The fixture populates only
 * the oracle's copies, so ctest_svsend_mirror_to_rust () republishes them
 * into the plain twins immediately before each Rust drive. sizebuf data
 * pointers are copied VERBATIM rather than re-pointed at the twin's own
 * arrays: both sides must read and write one datagram buffer, or
 * SV_SendClientDatagram's SZ_Write of sv.datagram would see different bytes.
 *
 * Everything else sv_send.c touches is already shared: host_client, cls,
 * qcvm, pr_global_struct, nullentitystate, realtime, isDedicated, dev_stats,
 * dev_peakstats, dev_overflows, the client array (ctest_svsend_clients, which
 * both sides reach through svs.clients) and the edict arena are all
 * un-renamed single definitions.
 *
 * Callee selection. Each SvSend_Glue_* trampoline below calls the ORACLE
 * implementation of the helper (c_ref_MSG_Write*, c_ref_SZ_Write,
 * c_ref_SZ_Clear, c_ref_PR_GetString, c_ref_AngleVectors,
 * c_ref_SV_ModelIndex, c_ref_standard_quake, c_ref_SV_SetIdealPitch,
 * c_ref_sv_player), so the differential isolates sv_send.c itself rather than
 * also folding in the M5/M6 ports of its helpers. Two of those matter beyond
 * tidiness:
 *  - plain `standard_quake` in this link is quake-ctest's own Rust
 *    quake_rs::fs::standard_quake (rust/quake-ctest/src/fs.rs:337), a
 *    different object from the one the oracle reads at sv_send.c:1669;
 *  - plain `SV_SetIdealPitch` is T6.4's Rust port (sv_user_ref.c:209) reading
 *    plain sv_player, while the oracle reaches sv_user.c's C body reading
 *    c_ref_sv_player. Routing both sides to c_ref_ removes a cross-task
 *    coupling from this gate. The real -Duse_rust_host build keeps the
 *    Rust-to-Rust topology; that is Quake/sv_send_glue.c's business, not
 *    this file's.
 * SV_DropClient, Mod_LeafPVS, NET_*, Con_* and Sys_Error are not renamed, so
 * their plain names already resolve to the single shared stubs.c definitions.
 */

#undef sv
#undef svs
#undef sv_netsort
#undef sv_smoothplatformlerps

extern server_t		   sv;	/* plain twins, defined by stubs/sv_main_ref.c */
extern server_static_t svs;
extern cvar_t		   sv_netsort;
extern cvar_t		   sv_smoothplatformlerps;

extern server_t		   c_ref_sv; /* oracle copies, what the fixture populates */
extern server_static_t c_ref_svs;
extern cvar_t		   c_ref_sv_netsort;
extern cvar_t		   c_ref_sv_smoothplatformlerps;

static void ctest_svsend_mirror_to_rust (void)
{
	memcpy (&sv, &c_ref_sv, sizeof (sv));
	memcpy (&svs, &c_ref_svs, sizeof (svs));
	sv_netsort.value = c_ref_sv_netsort.value;
	sv_smoothplatformlerps.value = c_ref_sv_smoothplatformlerps.value;
}

/* --------------------------------------------------------------------------
 * SvSend_Glue_* -- the ADR-009 guards and non-raising shims the Rust port
 * declares in rust/quake-c-sys/src/sv_send.rs. Bodies mirror
 * Quake/sv_send_glue.c exactly, except that each engine helper is spelled
 * c_ref_* (see the callee-selection note above). Not defined anywhere else in
 * the harness: Quake/sv_send_glue.c is not in build.rs's C_SOURCES.
 */

/* quakedef.h:478-479. The prelude renamed quakedef.h's prototypes before they
 * reached this file, so the plain spellings below -- stubs.c's substitute
 * guard, which is what the seams here are specified against -- have no
 * declaration left. Same line, same reason, as chase_ref.c:38-39. */
extern int	Host_Guard (void (*fn) (void *), void *arg);
extern void Host_Reraise (int guard_result);

/* Layout-identical to svsend_write_t in Quake/sv_send_glue.c:76-83 and to
 * SvSendWriteOp in rust/quake-c-sys/src/sv_send.rs:186-194. */
typedef struct
{
	int			kind;
	int			i;
	float		f;
	unsigned	u;
	const void *p;
} ctest_svsend_write_t;

typedef struct
{
	sizebuf_t				   *sb;
	const ctest_svsend_write_t *ops;
	int							count;
} ctest_svsend_writebatch_arg_t;

static void ctest_svsend_invoke_writebatch (void *p)
{
	ctest_svsend_writebatch_arg_t *a = (ctest_svsend_writebatch_arg_t *)p;
	int							   k;

	for (k = 0; k < a->count; k++)
	{
		const ctest_svsend_write_t *op = &a->ops[k];
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
			c_ref_MSG_WriteFloat (a->sb, op->f);
			break;
		case 5:
			c_ref_MSG_WriteString (a->sb, (const char *)op->p);
			break;
		case 6:
			c_ref_MSG_WriteCoord (a->sb, op->f, op->u);
			break;
		case 7:
			c_ref_MSG_WriteAngle (a->sb, op->f, op->u);
			break;
		case 8:
			c_ref_MSG_WriteAngle16 (a->sb, op->f, op->u);
			break;
		case 9:
			c_ref_MSG_WriteEntity (a->sb, (unsigned int)op->i, op->u);
			break;
		case 10:
			c_ref_SZ_Write (a->sb, op->p, op->i);
			break;
		default:
			break;
		}
	}
}

int SvSend_Glue_WriteBatch (sizebuf_t *sb, const ctest_svsend_write_t *ops, int count)
{
	ctest_svsend_writebatch_arg_t arg;
	arg.sb = sb;
	arg.ops = ops;
	arg.count = count;
	return Host_Guard (ctest_svsend_invoke_writebatch, &arg);
}

typedef struct
{
	int			 handle;
	const char **out;
} ctest_svsend_getstring_arg_t;

static void ctest_svsend_invoke_getstring (void *p)
{
	ctest_svsend_getstring_arg_t *a = (ctest_svsend_getstring_arg_t *)p;
	*a->out = c_ref_PR_GetString (a->handle);
}

int SvSend_Glue_GetString (int handle, const char **out)
{
	ctest_svsend_getstring_arg_t arg;
	arg.handle = handle;
	arg.out = out;
	return Host_Guard (ctest_svsend_invoke_getstring, &arg);
}

static void ctest_svsend_invoke_setidealpitch (void *p)
{
	(void)p;
	c_ref_SV_SetIdealPitch ();
}

int SvSend_Glue_SetIdealPitch (void)
{
	return Host_Guard (ctest_svsend_invoke_setidealpitch, NULL);
}

static void ctest_svsend_invoke_dropclient (void *p)
{
	SV_DropClient (*(qboolean *)p);
}

int SvSend_Glue_DropClient (qboolean crash)
{
	return Host_Guard (ctest_svsend_invoke_dropclient, &crash);
}

static void ctest_svsend_invoke_executereconnect (void *p)
{
	(void)p;
	c_ref_Cmd_ExecuteString ("reconnect\n", src_command);
}

int SvSend_Glue_ExecuteReconnect (void)
{
	return Host_Guard (ctest_svsend_invoke_executereconnect, NULL);
}

void SvSend_Glue_SzClear (sizebuf_t *sb)
{
	c_ref_SZ_Clear (sb);
}

void SvSend_Glue_AngleVectors (const float *angles, float *forward, float *right, float *up)
{
	c_ref_AngleVectors ((float *)angles, forward, right, up);
}

int SvSend_Glue_StandardQuake (void)
{
	return c_ref_standard_quake ? 1 : 0;
}

int SvSend_Glue_ModelIndex (const char *name)
{
	return c_ref_SV_ModelIndex (name);
}

void SvSend_Glue_SetPlayer (edict_t *ent)
{
	c_ref_sv_player = ent;
}

/* sv_send.c:268-271. LERP_BANDAID is unconditional (protocol.h:33), and the
 * stub address string is never "LOCAL" (stubs.c:6922), so this always returns
 * 1 here -- the same value the oracle computes from the same two reads. */
int SvSend_Glue_StripLerp (void)
{
#ifdef LERP_BANDAID
	return (cls.demorecording || strcmp (NET_QSocketGetTrueAddressString (host_client->netconnection), "LOCAL")) ? 1 : 0;
#else
	return 0;
#endif
}

void SvSend_Glue_WarnPacket (int cursize)
{
	Con_DWarning ("%i byte packet exceeds standard limit of 1024.\n", cursize);
}

void SvSend_Glue_WarnPacketMax (int cursize, int maxsize)
{
	Con_DWarning ("%i byte packet exceeds standard limit of 1024 (max = %d).\n", cursize, maxsize);
}

void SvSend_Glue_WarnOverflow (void)
{
	Con_Printf ("Packet overflow!\n");
}

FUNC_NORETURN void SvSend_Glue_FatPvsAllocFailed (int capacity)
{
	Sys_Error ("SV_FatPVS: realloc() failed on %d bytes", capacity);
}

/* --------------------------------------------------------------------------
 * Rust-side drivers -- the `_rs` twin of each ctest_svsend_drive_*_c above.
 *
 * quake_rs_svsend_* are the #[no_mangle] exports of
 * rust/quake-capi/src/sv_send.rs, linked in because quake-ctest depends on
 * quake-capi with the "host" feature. Each returns a Host_Guard status that
 * must reach a pure C frame untouched (ADR-009), which is what Host_Reraise
 * is for; the bodies mirror Quake/sv_send_glue.c's own entry points.
 *
 * No plain-named wrapper (SVFTE_SetupFrames, SV_CreateBaseline, ...) is
 * defined here on purpose: every plain sv_send.c name this link needs is
 * already supplied -- SVFTE_Ack by the trampoline above, and nothing else,
 * since sv_main_ref.c's SvMain_Glue_* bodies and stubs.c:7123's
 * ctest_m6_linkproof all reach the oracle through the prelude rename or an
 * explicit c_ref_ spelling. Adding one would collide with the oracle.
 */

extern int quake_rs_svsend_calc_stats (void *client, void *statsi, void *statsf, void *statss);
extern int quake_rs_svsend_destroy_frames (void *client);
extern int quake_rs_svsend_setup_frames (void *client);
extern int quake_rs_svsend_ack (void *client, int sequence);
extern int quake_rs_svsend_build_entity_state (void *ent, void *state);
extern int quake_rs_svsend_write_static_or_baseline (void *buf, int idx, void *state, unsigned int protocol_pext2, unsigned int protocol, unsigned int protocolflags);
extern int quake_rs_svsend_add_to_fat_pvs (void *org, void *node, void *worldmodel);
extern int quake_rs_svsend_fat_pvs (void *org, void *worldmodel, void **out);
extern int quake_rs_svsend_visible_to_client (void *client, void *test, void *worldmodel, int *out);
extern int quake_rs_svsend_write_entities_to_client (void *client, void *msg, size_t overflowsize);
extern int quake_rs_svsend_presend_client_datagram (void *client);
extern int quake_rs_svsend_send_client_datagram (void *client, int *out);

void ctest_svsend_drive_baseline_rs (int idx, int slot, unsigned int pext2, unsigned int protocol, unsigned int protocolflags)
{
	ctest_svsend_mirror_to_rust ();
	Host_Reraise (quake_rs_svsend_write_static_or_baseline (&ctest_svsend_msg, idx, &ctest_svsend_states[slot], pext2, protocol, protocolflags));
}

void ctest_svsend_drive_buildstate_rs (int ednum, int slot)
{
	ctest_svsend_mirror_to_rust ();
	Host_Reraise (quake_rs_svsend_build_entity_state (ctest_svsend_ed (ednum), &ctest_svsend_states[slot]));
}

void ctest_svsend_drive_calcstats_rs (int clx)
{
	ctest_svsend_mirror_to_rust ();
	Host_Reraise (quake_rs_svsend_calc_stats (&ctest_svsend_clients[clx], ctest_svsend_statsi, ctest_svsend_statsf, (void *)ctest_svsend_statss));
}

void ctest_svsend_drive_setupframes_rs (int clx)
{
	ctest_svsend_mirror_to_rust ();
	Host_Reraise (quake_rs_svsend_setup_frames (&ctest_svsend_clients[clx]));
}

void ctest_svsend_drive_destroyframes_rs (int clx)
{
	ctest_svsend_mirror_to_rust ();
	Host_Reraise (quake_rs_svsend_destroy_frames (&ctest_svsend_clients[clx]));
}

void ctest_svsend_drive_ack_rs (int clx, int sequence)
{
	ctest_set_host_client (&ctest_svsend_clients[clx]);
	ctest_svsend_mirror_to_rust ();
	Host_Reraise (quake_rs_svsend_ack (&ctest_svsend_clients[clx], sequence));
}

int ctest_svsend_drive_fatpvs_rs (float x, float y, float z, int want)
{
	vec3_t org;
	byte  *p = NULL;
	int	   i;
	org[0] = x;
	org[1] = y;
	org[2] = z;
	ctest_svsend_mirror_to_rust ();
	/* plain forward, not a Host_Reraise site: SV_FatPVS' only failure path is
	 * the Sys_Error at sv_send.c:1096 (Quake/sv_send_glue.c:309-314). */
	quake_rs_svsend_fat_pvs (org, &ctest_svsend_worldmodel, (void **)&p);
	ctest_svsend_fatptr = p;
	if (want > CTEST_SVSEND_PVSMAX)
		want = CTEST_SVSEND_PVSMAX;
	for (i = 0; i < want; i++)
		ctest_svsend_pvscopy[i] = p[i];
	ctest_svsend_pvslen = want;
	return want;
}

int ctest_svsend_drive_addtofatpvs_rs (float x, float y, float z, int nodeidx, int want)
{
	vec3_t org;
	int	   i;
	if (!ctest_svsend_fatptr)
		return -1;
	org[0] = x;
	org[1] = y;
	org[2] = z;
	ctest_svsend_mirror_to_rust ();
	Host_Reraise (quake_rs_svsend_add_to_fat_pvs (org, &ctest_svsend_nodes[nodeidx], &ctest_svsend_worldmodel));
	if (want > CTEST_SVSEND_PVSMAX)
		want = CTEST_SVSEND_PVSMAX;
	for (i = 0; i < want; i++)
		ctest_svsend_pvscopy[i] = ctest_svsend_fatptr[i];
	ctest_svsend_pvslen = want;
	return want;
}

int ctest_svsend_drive_visible_rs (int clientnum, int testnum)
{
	int out = 0;
	ctest_svsend_mirror_to_rust ();
	Host_Reraise (quake_rs_svsend_visible_to_client (ctest_svsend_ed (clientnum), ctest_svsend_ed (testnum), &ctest_svsend_worldmodel, &out));
	return out ? 1 : 0;
}

void ctest_svsend_drive_writeents_rs (int clx, int overflowsize)
{
	ctest_set_host_client (&ctest_svsend_clients[clx]);
	ctest_svsend_mirror_to_rust ();
	Host_Reraise (quake_rs_svsend_write_entities_to_client (&ctest_svsend_clients[clx], &ctest_svsend_msg, (size_t)overflowsize));
}

void ctest_svsend_drive_presend_rs (int clx)
{
	ctest_set_host_client (&ctest_svsend_clients[clx]);
	ctest_svsend_mirror_to_rust ();
	Host_Reraise (quake_rs_svsend_presend_client_datagram (&ctest_svsend_clients[clx]));
}

int ctest_svsend_drive_senddatagram_rs (int clx)
{
	int out = 0;
	ctest_svsend_mirror_to_rust ();
	Host_Reraise (quake_rs_svsend_send_client_datagram (&ctest_svsend_clients[clx], &out));
	return out ? 1 : 0;
}
