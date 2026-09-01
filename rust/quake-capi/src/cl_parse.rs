//! `Quake/cl_parse.c` -- the server-message parser (Rust migration Phase 7 M7,
//! T7.3). Pattern A whole-file swap: `Quake/cl_parse_glue.c` is the C frame,
//! this module is the body, and the pair replaces `cl_parse.c` under
//! `-Duse_rust_host`.
//!
//! ## ADR-009 (raise topology)
//!
//! `cl_parse.c` has 34 live raise sites (31 `Host_Error`, 3 `Host_EndGame`).
//! None of them may `longjmp` through a Rust frame, so this module never calls
//! a re-raising C wrapper: every raise becomes one of the `CLPARSE_*` status
//! codes below, returned up through plain `-> Raise` returns until
//! `ClParse_Raise` in the glue re-issues the original error from a pure C
//! frame. `Host_Reraise` is not declared anywhere under `rust/`.
//!
//! Every callee that can itself raise is reached through a `ClParse_Glue_*`
//! `Host_Guard` trampoline (21 of them). A non-zero trampoline result means "a
//! jump was caught": [`guard!`] stashes it in `Detail::a` and returns
//! [`CLPARSE_RAISE_GUARD`], so the Rust frames unwind by ordinary returns and
//! the glue re-issues the jump. Both `Sys_Error` sites (`cl_parse.c:1530`,
//! `:1928`) abort rather than jump, so they are called directly -- the
//! `world.rs` / `sv_phys.rs` / `sv_send.rs` precedent.
//!
//! ## ADR-007 (dual views)
//!
//! `cl` / `cls` stay C-owned for T7.3 (that row closes in T7.4 with
//! `cl_main.c`), as do `net_message` / `msg_readcount` / `msg_badread` (M9),
//! `cl_lightstyle`, `vid`, `dev_stats` / `dev_peakstats` / `dev_overflows`,
//! `con_lastcenterstring`, `v_punchangles*`, `noclip_anglehack` and
//! `r_trace_line_cache_counter`. `sv` is already Rust-owned (T6.6), so
//! `sv.active` / `sv.loadgame` read [`crate::sv_main::sv`] directly.
//!
//! ## ADR-010 (determinism)
//!
//! One libm site: `fabs` at `cl_parse.c:500`, routed through
//! `quake_c_sys::libm`. Every implicit C promotion in a float expression is
//! reproduced explicitly and marked `// COMPAT: ADR-010`.
//!
//! ## ADR-005 (float formatter)
//!
//! The format strings ported here use only `%3i %4i %i %d %u %x %#x %c %s`.
//! No `%g` / `%e` reaches the Rust formatter from this module.
//!
//! ## Function-local statics
//!
//! `CL_ParseServerInfo` (`cl_parse.c:931-934`) keeps four large buffers in
//! function-local `static` storage, and `cl_parse.c:91` has one file-scope
//! `static qboolean`. They become the five module-level `static mut`s below.
//! [`MODEL_PRECACHE`] specifically MUST have static storage: the
//! `CLPARSE_ERR_MODELNOTFOUND` raise hands the glue a `const char *` that
//! `ClParse_Raise` dereferences *after* this core has returned, so the pointer
//! has to outlive the return -- a local buffer would dangle.

use core::ffi::{c_char, c_float, c_int, c_uint, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::cl_input::V_StopPitchDrift;
use quake_c_sys::cl_parse as g;
use quake_c_sys::cl_tent::{MSG_ReadCoord, MSG_ReadEntity};
use quake_c_sys::libm;
use quake_c_sys::progs_builtins_cl::{S_PrecacheSound, S_StartSound, S_StaticSound};
use quake_c_sys::sv_main::COM_GetGameNames;
use quake_c_sys::sv_send::strlen;
use quake_c_sys::sv_user::{
    MSG_BeginReading, MSG_ReadAngle, MSG_ReadAngle16, MSG_ReadByte, MSG_ReadChar, MSG_ReadFloat,
    MSG_ReadLong, MSG_ReadShort, MSG_ReadString,
};
use quake_c_sys::view as gv;
use quake_math::mathlib as m;
use quake_types::host::{ClientState, ClientStatic, EntityOpaque, ScoreBoard};
use quake_types::model_mem::QModel;
use quake_types::progs::EntityState;
use quake_types::sound::Sfx;

use crate::view::{cl, cls, cvar_value, Entity};

/// A status for `ClParse_Raise`: 0 means "no raise".
type Raise = c_int;

/// Propagate a non-`CLPARSE_OK` status to the caller, abandoning the rest of
/// the body exactly where C's `longjmp` would have left it.
macro_rules! raise {
    ($e:expr) => {{
        let r: Raise = $e;
        if r != CLPARSE_OK {
            return r;
        }
    }};
}

/// Run a `Host_Guard` trampoline. A non-zero guard status means the callee
/// raised; hand it back as `CLPARSE_RAISE_GUARD` so the glue can re-issue it.
macro_rules! guard {
    ($d:expr, $e:expr) => {{
        let s: c_int = $e;
        if s != 0 {
            $d.a = s;
            return CLPARSE_RAISE_GUARD;
        }
    }};
}

// ---------------------------------------------------------------------------
// ADR-009 status codes. Mirrored verbatim from Quake/cl_parse_glue.c:123-156
// and rust/quake-ctest/stubs/cl_parse_ref.c; the three copies must stay in
// step.

const CLPARSE_OK: c_int = 0;
const CLPARSE_RAISE_GUARD: c_int = 1;
const CLPARSE_ERR_ENTITYNUM: c_int = 2;
const CLPARSE_ERR_SOUNDNUM: c_int = 3;
const CLPARSE_ERR_SOUNDENT: c_int = 4;
const CLPARSE_ERR_LOCALSOUND: c_int = 5;
const CLPARSE_ERR_PEXT1: c_int = 6;
const CLPARSE_ERR_PEXT2: c_int = 7;
const CLPARSE_ERR_VERSION: c_int = 8;
const CLPARSE_ERR_MAXCLIENTS: c_int = 9;
const CLPARSE_ERR_TOOMANYMODELS: c_int = 10;
const CLPARSE_ERR_TOOMANYSOUNDS: c_int = 11;
const CLPARSE_ERR_MODELNOTFOUND: c_int = 12;
const CLPARSE_ERR_BADMODNUM: c_int = 13;
const CLPARSE_ERR_TOOMANYSTATICS: c_int = 14;
const CLPARSE_ERR_BADMESSAGE: c_int = 15;
const CLPARSE_ERR_ILLEGIBLE: c_int = 16;
const CLPARSE_ERR_UPDATENAME: c_int = 17;
const CLPARSE_ERR_UPDATEFRAGS: c_int = 18;
const CLPARSE_ERR_UPDATECOLORS: c_int = 19;
const CLPARSE_ERR_SIGNON: c_int = 20;
const CLPARSE_ERR_DPPRECACHE: c_int = 21;
const CLPARSE_ERR_UPDATESTATBYTE: c_int = 22;
const CLPARSE_ERR_UPDATESTATSTRING: c_int = 23;
const CLPARSE_ERR_UPDATESTATFLOAT: c_int = 24;
const CLPARSE_ERR_SPAWNSTATIC2: c_int = 25;
const CLPARSE_ERR_SPAWNBASELINE2: c_int = 26;
const CLPARSE_ERR_UPDATEENTITIES: c_int = 27;
const CLPARSE_ERR_CGAMEPACKET: c_int = 28;
const CLPARSE_ERR_CSQC_MISSING: c_int = 29;
const CLPARSE_ERR_VOICECHAT: c_int = 30;
const CLPARSE_END_DELTAINFO: c_int = 31;
const CLPARSE_END_UF_UNUSED1: c_int = 32;
const CLPARSE_END_DISCONNECTED: c_int = 33;

/// The out-parameters `ClParse_Raise` formats into the original error text.
struct Detail {
    a: c_int,
    b: c_int,
    s: *const c_char,
}

impl Detail {
    const fn new() -> Self {
        Detail {
            a: 0,
            b: 0,
            s: ptr::null(),
        }
    }
}

// ---------------------------------------------------------------------------
// Quake/quakedef.h, Quake/protocol.h, Quake/client.h, Quake/common.h.

const MAX_LIGHTSTYLES: c_int = 64;
const MAX_MODELS: c_int = 8192;
const MAX_SOUNDS: c_int = 2048;
const MAX_PARTICLETYPES: c_int = 2048;
const MAX_STYLESTRING: usize = 64;
const MAX_SCOREBOARD: c_int = 16;
const MAX_SCOREBOARDNAME: usize = 32;
const MAX_QPATH: usize = 64;
const MAX_CL_STATS: c_int = 256;
const CLIENT_USER_INFO_STRING_SIZE: usize = 8192;
const SIGNONS: c_int = 4;
/// `Quake/common.h:332`.
const COM_RAND_MAX: c_int = 0x00FF_FFFF;

const STAT_HEALTH: usize = 0;
const STAT_WEAPON: usize = 2;
const STAT_AMMO: usize = 3;
const STAT_ARMOR: usize = 4;
const STAT_WEAPONFRAME: usize = 5;
const STAT_SHELLS: usize = 6;
const STAT_NAILS: usize = 7;
const STAT_ROCKETS: usize = 8;
const STAT_CELLS: usize = 9;
const STAT_ACTIVEWEAPON: usize = 10;
const STAT_SECRETS: usize = 13;
const STAT_MONSTERS: usize = 14;
const STAT_ITEMS: usize = 15;
const STAT_VIEWHEIGHT: usize = 16;
const STAT_VIEWZOOM: c_int = 21;
const STAT_IDEALPITCH: usize = 25;
const STAT_PUNCHANGLE_X: usize = 26;
const STAT_PUNCHANGLE_Y: usize = 27;
const STAT_PUNCHANGLE_Z: usize = 28;

const PROTOCOL_NETQUAKE: c_uint = 15;
const PROTOCOL_FITZQUAKE: c_uint = 666;
const PROTOCOL_RMQ: c_uint = 999;
/// `('F' << 0) + ('T' << 8) + ('E' << 16) + ('X' << 24)`.
const PROTOCOL_FTE_PEXT1: c_int = 0x5845_5446;
/// `('F' << 0) + ('T' << 8) + ('E' << 16) + ('2' << 24)`.
const PROTOCOL_FTE_PEXT2: c_int = 0x3245_5446;

const PRFL_SHORTANGLE: c_uint = 1 << 1;
const PRFL_FLOATANGLE: c_uint = 1 << 2;
const PRFL_24BITCOORD: c_uint = 1 << 3;
const PRFL_FLOATCOORD: c_uint = 1 << 4;
const PRFL_EDICTSCALE: c_uint = 1 << 5;
const PRFL_INT32COORD: c_uint = 1 << 7;

const PEXT1_CSQC: c_uint = 0x4000_0000;
const PEXT1_SUPPORTED_CLIENT: c_uint = PEXT1_CSQC;
const PEXT1_ACCEPTED_CLIENT: c_uint = PEXT1_SUPPORTED_CLIENT;

const PEXT2_PRYDONCURSOR: c_uint = 0x0000_0001;
const PEXT2_VOICECHAT: c_uint = 0x0000_0002;
const PEXT2_REPLACEMENTDELTAS: c_uint = 0x0000_0008;
const PEXT2_PREDINFO: c_uint = 0x0000_0020;
const PEXT2_SUPPORTED_CLIENT: c_uint = PEXT2_REPLACEMENTDELTAS | PEXT2_PREDINFO;
const PEXT2_ACCEPTED_CLIENT: c_uint = PEXT2_SUPPORTED_CLIENT | PEXT2_PRYDONCURSOR | PEXT2_VOICECHAT;

const U_MOREBITS: c_int = 1 << 0;
const U_ORIGIN1: c_int = 1 << 1;
const U_ORIGIN2: c_int = 1 << 2;
const U_ORIGIN3: c_int = 1 << 3;
const U_ANGLE2: c_int = 1 << 4;
const U_STEP: c_int = 1 << 5;
const U_FRAME: c_int = 1 << 6;
const U_SIGNAL: c_int = 1 << 7;
const U_ANGLE1: c_int = 1 << 8;
const U_ANGLE3: c_int = 1 << 9;
const U_MODEL: c_int = 1 << 10;
const U_COLORMAP: c_int = 1 << 11;
const U_SKIN: c_int = 1 << 12;
const U_EFFECTS: c_int = 1 << 13;
const U_LONGENTITY: c_int = 1 << 14;
const U_EXTEND1: c_int = 1 << 15;
/// Nehahra's reuse of `U_EXTEND1` under `PROTOCOL_NETQUAKE`.
const U_TRANS: c_int = 1 << 15;
const U_ALPHA: c_int = 1 << 16;
const U_FRAME2: c_int = 1 << 17;
const U_MODEL2: c_int = 1 << 18;
const U_LERPFINISH: c_int = 1 << 19;
const U_SCALE: c_int = 1 << 20;
const U_EXTEND2: c_int = 1 << 23;

const SU_VIEWHEIGHT: c_int = 1 << 0;
const SU_IDEALPITCH: c_int = 1 << 1;
const SU_PUNCH1: c_int = 1 << 2;
const SU_VELOCITY1: c_int = 1 << 5;
const SU_ITEMS: c_int = 1 << 9;
const SU_ONGROUND: c_int = 1 << 10;
const SU_INWATER: c_int = 1 << 11;
const SU_WEAPONFRAME: c_int = 1 << 12;
const SU_ARMOR: c_int = 1 << 13;
const SU_WEAPON: c_int = 1 << 14;
const SU_EXTEND1: c_int = 1 << 15;
const SU_WEAPON2: c_int = 1 << 16;
const SU_ARMOR2: c_int = 1 << 17;
const SU_AMMO2: c_int = 1 << 18;
const SU_SHELLS2: c_int = 1 << 19;
const SU_NAILS2: c_int = 1 << 20;
const SU_ROCKETS2: c_int = 1 << 21;
const SU_CELLS2: c_int = 1 << 22;
const SU_EXTEND2: c_int = 1 << 23;
const SU_WEAPONFRAME2: c_int = 1 << 24;
const SU_WEAPONALPHA: c_int = 1 << 25;

const SND_VOLUME: c_int = 1 << 0;
const SND_ATTENUATION: c_int = 1 << 1;
const DEFAULT_SOUND_PACKET_VOLUME: c_int = 255;
const DEFAULT_SOUND_PACKET_ATTENUATION: c_float = 1.0;
const SND_LARGEENTITY: c_int = 1 << 3;
const SND_LARGESOUND: c_int = 1 << 4;
const SND_FTE_MOREFLAGS: c_int = 1 << 2;
const SND_DP_PITCH: c_int = 1 << 5;
const SND_FTE_TIMEOFS: c_int = 1 << 6;
const SND_FTE_PITCHADJ: c_int = 1 << 7;
const SND_FTE_VELOCITY: c_int = 1 << 8;

const B_LARGEMODEL: c_int = 1 << 0;
const B_LARGEFRAME: c_int = 1 << 1;
const B_ALPHA: c_int = 1 << 2;
const B_SCALE: c_int = 1 << 3;

const ENTALPHA_DEFAULT: c_int = 0;
const ENTSCALE_DEFAULT: c_int = 16;
const DEFAULT_VIEWHEIGHT: c_int = 22;

const UF_FRAME: c_uint = 1 << 0;
const UF_ORIGINXY: c_uint = 1 << 1;
const UF_ORIGINZ: c_uint = 1 << 2;
const UF_ANGLESXZ: c_uint = 1 << 3;
const UF_ANGLESY: c_uint = 1 << 4;
const UF_EFFECTS: c_uint = 1 << 5;
const UF_PREDINFO: c_uint = 1 << 6;
const UF_EXTEND1: c_uint = 1 << 7;
const UF_RESET: c_uint = 1 << 8;
const UF_16BIT: c_uint = 1 << 9;
const UF_MODEL: c_uint = 1 << 10;
const UF_SKIN: c_uint = 1 << 11;
const UF_COLORMAP: c_uint = 1 << 12;
const UF_SOLID: c_uint = 1 << 13;
const UF_FLAGS: c_uint = 1 << 14;
const UF_EXTEND2: c_uint = 1 << 15;
const UF_ALPHA: c_uint = 1 << 16;
const UF_SCALE: c_uint = 1 << 17;
const UF_BONEDATA: c_uint = 1 << 18;
const UF_DRAWFLAGS: c_uint = 1 << 19;
const UF_TAGINFO: c_uint = 1 << 20;
const UF_LIGHT: c_uint = 1 << 21;
const UF_TRAILEFFECT: c_uint = 1 << 22;
const UF_EXTEND3: c_uint = 1 << 23;
const UF_COLORMOD: c_uint = 1 << 24;
const UF_GLOW: c_uint = 1 << 25;
const UF_FATNESS: c_uint = 1 << 26;
const UF_MODELINDEX2: c_uint = 1 << 27;
const UF_GRAVITYDIR: c_uint = 1 << 28;
const UF_EFFECTS2: c_uint = 1 << 29;
const UF_UNUSED2: c_uint = 1 << 30;
const UF_UNUSED1: c_uint = 1 << 31;

const UFP_FORWARD: c_uint = 1 << 0;
const UFP_SIDE: c_uint = 1 << 1;
const UFP_UP: c_uint = 1 << 2;
const UFP_MOVETYPE: c_uint = 1 << 3;
const UFP_VELOCITYXY: c_uint = 1 << 4;
const UFP_VELOCITYZ: c_uint = 1 << 5;
const UFP_MSEC: c_uint = 1 << 6;
const UFP_WEAPONFRAME_OLD: c_uint = 1 << 7;
const UFP_VIEWANGLE: c_uint = 1 << 7;

const ES_SOLID_BSP: c_int = 31;
const EFLAGS_STEP: u8 = 1;
const EFLAGS_ONGROUND: u8 = 128;

/// `Quake/modelgen.h:46-48` `synctype_t`.
const ST_RAND: i32 = 1;
const ST_FRAMETIME: i32 = 2;

/// `Quake/cmd.h:80-85` `cmd_source_t`.
const SRC_COMMAND: c_int = 1;
const SRC_SERVER: c_int = 2;

/// `Quake/cl_parse_glue.c:57` -- `countof (svc_strings)`, i.e.
/// `cl_parse.c:89`'s `NUM_SVC_STRINGS`.
const NUM_SVC_STRINGS: c_int = 128;

// svc opcodes (Quake/protocol.h:105-...).
const SVC_NOP: c_int = 1;
const SVC_DISCONNECT: c_int = 2;
const SVC_UPDATESTAT: c_int = 3;
const SVC_VERSION: c_int = 4;
const SVC_SETVIEW: c_int = 5;
const SVC_SOUND: c_int = 6;
const SVC_TIME: c_int = 7;
const SVC_PRINT: c_int = 8;
const SVC_STUFFTEXT: c_int = 9;
const SVC_SETANGLE: c_int = 10;
const SVC_SERVERINFO: c_int = 11;
const SVC_LIGHTSTYLE: c_int = 12;
const SVC_UPDATENAME: c_int = 13;
const SVC_UPDATEFRAGS: c_int = 14;
const SVC_CLIENTDATA: c_int = 15;
const SVC_STOPSOUND: c_int = 16;
const SVC_UPDATECOLORS: c_int = 17;
const SVC_PARTICLE: c_int = 18;
const SVC_DAMAGE: c_int = 19;
const SVC_SPAWNSTATIC: c_int = 20;
const SVCFTE_SPAWNSTATIC2: c_int = 21;
const SVC_SPAWNBASELINE: c_int = 22;
const SVC_TEMP_ENTITY: c_int = 23;
const SVC_SETPAUSE: c_int = 24;
const SVC_SIGNONNUM: c_int = 25;
const SVC_CENTERPRINT: c_int = 26;
const SVC_KILLEDMONSTER: c_int = 27;
const SVC_FOUNDSECRET: c_int = 28;
const SVC_SPAWNSTATICSOUND: c_int = 29;
const SVC_INTERMISSION: c_int = 30;
const SVC_FINALE: c_int = 31;
const SVC_CDTRACK: c_int = 32;
const SVC_SELLSCREEN: c_int = 33;
const SVC_CUTSCENE: c_int = 34;
const SVC_SKYBOX: c_int = 37;
const SVC_BF: c_int = 40;
const SVC_FOG: c_int = 41;
const SVC_SPAWNBASELINE2: c_int = 42;
const SVC_SPAWNSTATIC2: c_int = 43;
const SVC_SPAWNSTATICSOUND2: c_int = 44;
const SVCDP_UPDATESTATBYTE: c_int = 51;
const SVC_ACHIEVEMENT: c_int = 52;
const SVCDP_PRECACHE: c_int = 54;
const SVC_LOCALSOUND: c_int = 56;
const SVCDP_TRAILPARTICLES: c_int = 60;
const SVCDP_POINTPARTICLES: c_int = 61;
const SVCDP_POINTPARTICLES1: c_int = 62;
const SVCFTE_SPAWNBASELINE2: c_int = 66;
const SVCFTE_UPDATESTATSTRING: c_int = 78;
const SVCFTE_UPDATESTATFLOAT: c_int = 79;
const SVCFTE_CGAMEPACKET: c_int = 83;
const SVCFTE_VOICECHAT: c_int = 84;
const SVCFTE_SETANGLEDELTA: c_int = 85;
const SVCFTE_UPDATEENTITIES: c_int = 86;

// ---------------------------------------------------------------------------
// C-owned objects with no typed declaration elsewhere.

/// `host.c` `dev_overflows` -- four `double`s. Mirrored the same way
/// `cl_tent.rs` does; only ever zeroed from here.
#[repr(C)]
struct OverflowTimes {
    packetsize: f64,
    efrags: f64,
    beams: f64,
    varstring: f64,
}

extern "C" {
    /// `common.c:1435`. C-owned until M9.
    static nullentitystate: EntityState;
    /// `host.c`. C-owned until Phase 8.
    static mut dev_overflows: OverflowTimes;
}

// ---------------------------------------------------------------------------
// cl_parse.c's file-scope and function-local statics.

/// `cl_parse.c:91`.
static mut WARN_ABOUT_NEHAHRA_PROTOCOL: bool = false;
/// `cl_parse.c:931`.
static mut GAMEDIR: [c_char; 1024] = [0; 1024];
/// `cl_parse.c:932`.
static mut PROTNAME: [c_char; 64] = [0; 64];
/// `cl_parse.c:933`. Static storage is load-bearing, not an optimisation: the
/// `CLPARSE_ERR_MODELNOTFOUND` raise returns `&MODEL_PRECACHE[i][0]` to the
/// glue, which formats it after this frame has gone.
static mut MODEL_PRECACHE: [[c_char; MAX_QPATH]; MAX_MODELS as usize] =
    [[0; MAX_QPATH]; MAX_MODELS as usize];
/// `cl_parse.c:934`.
static mut SOUND_PRECACHE: [[c_char; MAX_QPATH]; MAX_SOUNDS as usize] =
    [[0; MAX_QPATH]; MAX_SOUNDS as usize];

// ---------------------------------------------------------------------------
// helpers

#[inline]
fn cl_p() -> *mut ClientState {
    ptr::addr_of_mut!(cl)
}

#[inline]
fn cls_p() -> *mut ClientStatic {
    ptr::addr_of_mut!(cls)
}

/// `&cl.entities[i]`, striding by the authoritative opaque `entity_t` size.
///
/// # Safety
/// `cl.entities` must be allocated and `i` in range, exactly as in C.
#[inline]
unsafe fn cl_entity(i: c_int) -> *mut Entity {
    // SAFETY: `cl.entities` is the C-owned entity array; the caller has
    // already grown it through `cl_entity_num` where C does.
    unsafe {
        ptr::addr_of!((*cl_p()).entities)
            .read()
            .offset(i as isize)
            .cast::<Entity>()
    }
}

/// `&cl.viewent`.
#[inline]
unsafe fn cl_viewent() -> *mut Entity {
    // SAFETY: an inline member of the C-owned `cl`.
    unsafe { ptr::addr_of_mut!((*cl_p()).viewent).cast::<Entity>() }
}

/// `cl.model_precache[i]` without a bounds check.
///
/// # Safety
/// COMPAT: several call sites index this with an unvalidated `unsigned short`
/// straight off the wire (`cl_parse.c:583`, `:1312`, `:1560`); C reads out of
/// bounds there and so must this port.
#[inline]
unsafe fn cl_model_precache(i: isize) -> *mut QModel {
    // SAFETY: reproduces the C indexing exactly, including its bugs.
    unsafe {
        ptr::addr_of!((*cl_p()).model_precache)
            .cast::<*mut QModel>()
            .offset(i)
            .read()
    }
}

/// `cl.sound_precache[i]` without a bounds check (`cl_parse.c:1601`).
#[inline]
unsafe fn cl_sound_precache(i: isize) -> *mut Sfx {
    // SAFETY: reproduces the C indexing exactly, including its bugs.
    unsafe {
        ptr::addr_of!((*cl_p()).sound_precache)
            .cast::<*mut Sfx>()
            .offset(i)
            .read()
    }
}

/// `&cl.particle_precache[i]`, unchecked (`cl_parse.c:1717` compares a signed
/// `efnum` only against the upper bound).
#[inline]
unsafe fn cl_particle_precache(i: isize) -> *mut quake_types::host::ParticlePrecacheEntry {
    // SAFETY: reproduces the C indexing exactly, including its bugs.
    unsafe {
        ptr::addr_of_mut!((*cl_p()).particle_precache)
            .cast::<quake_types::host::ParticlePrecacheEntry>()
            .wrapping_offset(i)
    }
}

/// `&model_precache[i][0]` (`cl_parse.c:933`).
#[inline]
unsafe fn model_precache_name(i: usize) -> *mut c_char {
    // SAFETY: `MODEL_PRECACHE` is process-lifetime storage; `i < MAX_MODELS`
    // is checked at every call site exactly as C does.
    unsafe {
        ptr::addr_of_mut!(MODEL_PRECACHE)
            .cast::<c_char>()
            .add(i * MAX_QPATH)
    }
}

/// `&sound_precache[i][0]` (`cl_parse.c:934`).
#[inline]
unsafe fn sound_precache_name(i: usize) -> *mut c_char {
    // SAFETY: as `model_precache_name`.
    unsafe {
        ptr::addr_of_mut!(SOUND_PRECACHE)
            .cast::<c_char>()
            .add(i * MAX_QPATH)
    }
}

/// `&cl_lightstyle[i]` (`Quake/client.h`), unchecked below zero exactly as
/// `cl_parse.c:1927` is.
#[inline]
unsafe fn cl_lightstyle(i: isize) -> *mut g::lightstyle_t {
    // SAFETY: reproduces the C indexing exactly, including its bugs.
    unsafe {
        ptr::addr_of_mut!(g::cl_lightstyle)
            .cast::<g::lightstyle_t>()
            .offset(i)
    }
}

/// `&cl.scores[i]`, unchecked below zero exactly as `cl_parse.c:1959` is.
#[inline]
unsafe fn cl_score(i: isize) -> *mut ScoreBoard {
    // SAFETY: reproduces the C indexing exactly, including its bugs.
    unsafe { ptr::addr_of!((*cl_p()).scores).read().offset(i) }
}

#[inline]
unsafe fn cl_shownet_value() -> c_float {
    // SAFETY: `cl_shownet` is a live engine cvar.
    unsafe { cvar_value(ptr::addr_of!(g::cl_shownet)) }
}

#[inline]
unsafe fn msg_readcount() -> c_int {
    // SAFETY: a plain `int` engine global.
    unsafe { ptr::addr_of!(c::msg_readcount).read() }
}

#[inline]
unsafe fn msg_badread() -> bool {
    // SAFETY: a plain `qboolean` engine global.
    unsafe { ptr::addr_of!(c::msg_badread).read() }
}

#[inline]
unsafe fn protocolflags() -> c_uint {
    // SAFETY: an inline member of the C-owned `cl`.
    unsafe { (*cl_p()).protocolflags }
}

/// `svc_strings[i]` (`cl_parse_glue.c:57`).
#[inline]
unsafe fn svc_string(i: usize) -> *const c_char {
    // SAFETY: `i < NUM_SVC_STRINGS` at both call sites.
    unsafe {
        ptr::addr_of!(g::svc_strings)
            .cast::<*const c_char>()
            .add(i)
            .read()
    }
}

/// `Quake/glquake.h:134` `InvalidateTraceLineCache()`.
#[inline]
unsafe fn invalidate_trace_line_cache() {
    // COMPAT: ADR-010 -- signed overflow is UB in C and wraps here.
    // SAFETY: a plain `int` engine global.
    unsafe {
        let p = ptr::addr_of_mut!(gv::r_trace_line_cache_counter);
        p.write(p.read().wrapping_add(1));
    }
}

/// `Quake/q_minmax.h:69` `Q_rint` applied to a `float`.
///
/// COMPAT: ADR-010 -- `(x) + 0.5` promotes to `double` in C.
#[inline]
fn q_rint_f(x: c_float) -> c_int {
    if x > 0.0 {
        (x as f64 + 0.5) as c_int
    } else {
        (x as f64 - 0.5) as c_int
    }
}

/// `Quake/q_minmax.h:20` `clamp_f`. `CLAMP (1, a * 254.0f + 1, 255)` dispatches
/// on `int + float + int`, i.e. `float`.
#[inline]
fn clamp_f(minval: c_float, val: c_float, maxval: c_float) -> c_float {
    if val < minval {
        minval
    } else if val > maxval {
        maxval
    } else {
        val
    }
}

/// `Quake/protocol.h:222` `ENTALPHA_ENCODE`.
#[inline]
fn entalpha_encode(a: c_float) -> c_int {
    if a == 0.0 {
        ENTALPHA_DEFAULT
    } else {
        // COMPAT: ADR-010 -- `a * 254.0f + 1` is evaluated in float.
        q_rint_f(clamp_f(1.0, a * 254.0f32 + 1.0, 255.0))
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:105

unsafe fn cl_entity_num(num: c_int, out: *mut *mut Entity, d: &mut Detail) -> Raise {
    // SAFETY: `cl` is C-owned with process lifetime; the growth loop mirrors
    // C's, so every write stays inside the allocated entity array.
    unsafe {
        // johnfitz -- check minimum number too
        if num < 0 {
            d.a = num;
            return CLPARSE_ERR_ENTITYNUM;
        }

        if num >= (*cl_p()).num_entities {
            if num >= (*cl_p()).max_edicts {
                d.a = num;
                return CLPARSE_ERR_ENTITYNUM;
            }
            while (*cl_p()).num_entities <= num {
                let n = (*cl_p()).num_entities;
                (*cl_entity(n)).baseline = ptr::addr_of!(nullentitystate).read();
                (*cl_p()).num_entities = n + 1;
            }
        }

        *out = cl_entity(num);
        CLPARSE_OK
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:126 -- the `sizebuf_t *` parameter is unused; MSG_ReadShort reads
// the global `net_message` in both the C original and here.

unsafe fn msg_read_size16() -> c_int {
    // SAFETY: plain reads off the C-owned message buffer.
    unsafe {
        let ssolid = MSG_ReadShort() as u16;
        if ssolid as c_int == ES_SOLID_BSP {
            ssolid as c_int
        } else {
            // `((x) - 32 + 32768) << 16` overflows a signed int in C; Rust's
            // `<<` discards the high bits the same way.
            let mut solid = ((((ssolid >> 7) & 0x1F8) as c_int) - 32 + 32768) << 16;
            solid |= ((ssolid & 0x1F) as c_int) << 3;
            solid |= ((ssolid & 0x3E0) as c_int) << 6;
            solid
        }
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:139

unsafe fn clfte_read_delta(
    entnum: c_uint,
    news: *mut EntityState,
    olds: *const EntityState,
    baseline: *const EntityState,
    out_bits: &mut c_uint,
    d: &mut Detail,
) -> Raise {
    // SAFETY: `news`/`olds`/`baseline` point at live `entity_state_t` storage
    // owned by the caller; all reads come from the C-owned message buffer.
    unsafe {
        let mut predbits: c_uint = 0;
        let mut bits: c_uint;

        bits = MSG_ReadByte() as c_uint;
        if bits & UF_EXTEND1 != 0 {
            bits |= (MSG_ReadByte() as c_uint) << 8;
        }
        if bits & UF_EXTEND2 != 0 {
            bits |= (MSG_ReadByte() as c_uint) << 16;
        }
        if bits & UF_EXTEND3 != 0 {
            bits |= (MSG_ReadByte() as c_uint) << 24;
        }

        if cl_shownet_value() >= 3.0 {
            c::Con_SafePrintf(
                c"%3i:     Update %4i 0x%x\n".as_ptr(),
                msg_readcount(),
                entnum,
                bits,
            );
        }

        if bits & UF_RESET != 0 {
            news.write(baseline.read());
        } else if olds.is_null() {
            // reset got lost, probably the data will be filled in later
            if ptr::addr_of!(crate::sv_main::sv.active).read() {
                guard!(d, g::ClParse_Glue_DebugNewEntity(entnum));
            } else {
                c::Con_DPrintf(c"New entity %i without reset\n".as_ptr(), entnum);
            }
            news.write(baseline.read());
        } else {
            news.write(olds.read());
        }

        if bits & UF_FRAME != 0 {
            if bits & UF_16BIT != 0 {
                (*news).frame = MSG_ReadShort() as u16;
            } else {
                (*news).frame = MSG_ReadByte() as u16;
            }
        }

        if bits & UF_ORIGINXY != 0 {
            (*news).origin[0] = MSG_ReadCoord(protocolflags());
            (*news).origin[1] = MSG_ReadCoord(protocolflags());
        }
        if bits & UF_ORIGINZ != 0 {
            (*news).origin[2] = MSG_ReadCoord(protocolflags());
        }

        if (bits & UF_PREDINFO) != 0 && ((*cl_p()).protocol_pext2 & PEXT2_PREDINFO) == 0 {
            // predicted stuff gets more precise angles
            if bits & UF_ANGLESXZ != 0 {
                (*news).angles[0] = MSG_ReadAngle16(protocolflags());
                (*news).angles[2] = MSG_ReadAngle16(protocolflags());
            }
            if bits & UF_ANGLESY != 0 {
                (*news).angles[1] = MSG_ReadAngle16(protocolflags());
            }
        } else {
            if bits & UF_ANGLESXZ != 0 {
                (*news).angles[0] = MSG_ReadAngle(protocolflags());
                (*news).angles[2] = MSG_ReadAngle(protocolflags());
            }
            if bits & UF_ANGLESY != 0 {
                (*news).angles[1] = MSG_ReadAngle(protocolflags());
            }
        }

        if (bits & (UF_EFFECTS | UF_EFFECTS2)) == (UF_EFFECTS | UF_EFFECTS2) {
            (*news).effects = MSG_ReadLong() as c_uint;
        } else if bits & UF_EFFECTS2 != 0 {
            (*news).effects = (MSG_ReadShort() as u16) as c_uint;
        } else if bits & UF_EFFECTS != 0 {
            (*news).effects = MSG_ReadByte() as c_uint;
        }

        (*news).velocity[0] = 0;
        (*news).velocity[1] = 0;
        (*news).velocity[2] = 0;
        if bits & UF_PREDINFO != 0 {
            predbits = MSG_ReadByte() as c_uint;

            if predbits & UFP_FORWARD != 0 {
                MSG_ReadShort();
            }
            if predbits & UFP_SIDE != 0 {
                MSG_ReadShort();
            }
            if predbits & UFP_UP != 0 {
                MSG_ReadShort();
            }
            if predbits & UFP_MOVETYPE != 0 {
                (*news).pmovetype = MSG_ReadByte() as u8;
            }
            if predbits & UFP_VELOCITYXY != 0 {
                (*news).velocity[0] = MSG_ReadShort() as i16;
                (*news).velocity[1] = MSG_ReadShort() as i16;
            } else {
                (*news).velocity[0] = 0;
                (*news).velocity[1] = 0;
            }
            if predbits & UFP_VELOCITYZ != 0 {
                (*news).velocity[2] = MSG_ReadShort() as i16;
            } else {
                (*news).velocity[2] = 0;
            }
            if predbits & UFP_MSEC != 0 {
                MSG_ReadByte();
            }

            if (*cl_p()).protocol_pext2 & PEXT2_PREDINFO != 0 {
                if predbits & UFP_VIEWANGLE != 0 {
                    if bits & UF_ANGLESXZ != 0 {
                        MSG_ReadShort();
                        MSG_ReadShort();
                    }
                    if bits & UF_ANGLESY != 0 {
                        MSG_ReadShort();
                    }
                }
            } else if predbits & UFP_WEAPONFRAME_OLD != 0 {
                let wframe = MSG_ReadByte();
                if wframe & 0x80 != 0 {
                    let _ = (wframe & 127) | (MSG_ReadByte() << 7);
                }
            }
        }

        // cl_parse.c:294-306 is an `if` with a wholly commented-out body; it
        // has no side effects, so `predbits` is only read below.
        let _ = predbits;

        if bits & UF_MODEL != 0 {
            if bits & UF_16BIT != 0 {
                (*news).modelindex = MSG_ReadShort() as u16;
            } else {
                (*news).modelindex = MSG_ReadByte() as u16;
            }
        }
        if bits & UF_SKIN != 0 {
            if bits & UF_16BIT != 0 {
                (*news).skin = MSG_ReadShort() as u8;
            } else {
                (*news).skin = MSG_ReadByte() as u8;
            }
        }
        if bits & UF_COLORMAP != 0 {
            (*news).colormap = MSG_ReadByte() as u8;
        }

        if bits & UF_SOLID != 0 {
            msg_read_size16();
        }

        if bits & UF_FLAGS != 0 {
            (*news).eflags = MSG_ReadByte() as u8;
        }

        if bits & UF_ALPHA != 0 {
            (*news).alpha = ((MSG_ReadByte() + 1) & 0xff) as u8;
        }
        if bits & UF_SCALE != 0 {
            (*news).scale = MSG_ReadByte() as u8;
        }
        if bits & UF_BONEDATA != 0 {
            let fl = MSG_ReadByte() as u8;
            if fl & 0x80 != 0 {
                let bonecount = MSG_ReadByte();
                let mut i = 0;
                while i < bonecount * 7 {
                    MSG_ReadShort();
                    i += 1;
                }
            }
            if fl & 0x40 != 0 {
                MSG_ReadByte();
                MSG_ReadShort();
            }
            if fl & 0x3f != 0 {
                return CLPARSE_END_DELTAINFO;
            }
        }

        if bits & UF_DRAWFLAGS != 0 {
            let drawflags = MSG_ReadByte();
            if (drawflags & 7) == 7 {
                MSG_ReadByte();
            }
        }
        if bits & UF_TAGINFO != 0 {
            (*news).tagentity = MSG_ReadEntity((*cl_p()).protocol_pext2) as u16;
            (*news).tagindex = MSG_ReadByte() as u8;
        }
        if bits & UF_LIGHT != 0 {
            MSG_ReadShort();
            MSG_ReadShort();
            MSG_ReadShort();
            MSG_ReadShort();
            MSG_ReadByte();
            MSG_ReadByte();
        }
        if bits & UF_TRAILEFFECT != 0 {
            let v = MSG_ReadShort() as u16;
            (*news).emiteffectnum = 0;
            (*news).traileffectnum = v & 0x3fff;
            if v & 0x8000 != 0 {
                (*news).emiteffectnum = (MSG_ReadShort() & 0x3fff) as u16;
            }
            if (*news).traileffectnum as c_int >= MAX_PARTICLETYPES {
                (*news).traileffectnum = 0;
            }
            if (*news).emiteffectnum as c_int >= MAX_PARTICLETYPES {
                (*news).emiteffectnum = 0;
            }
        }

        if bits & UF_COLORMOD != 0 {
            (*news).colormod[0] = MSG_ReadByte() as u8;
            (*news).colormod[1] = MSG_ReadByte() as u8;
            (*news).colormod[2] = MSG_ReadByte() as u8;
        }
        if bits & UF_GLOW != 0 {
            MSG_ReadByte();
            MSG_ReadByte();
            MSG_ReadByte();
            MSG_ReadByte();
            MSG_ReadByte();
        }
        if bits & UF_FATNESS != 0 {
            MSG_ReadByte();
        }
        if bits & UF_MODELINDEX2 != 0 {
            if bits & UF_16BIT != 0 {
                MSG_ReadShort();
            } else {
                MSG_ReadByte();
            }
        }
        if bits & UF_GRAVITYDIR != 0 {
            MSG_ReadByte();
            MSG_ReadByte();
        }
        if bits & UF_UNUSED2 != 0 {
            // LERP_BANDAID is defined unconditionally (protocol.h:33), so the
            // Host_EndGame in the #else is dead.
            (*news).lerp = MSG_ReadShort() as u16;
        }
        if bits & UF_UNUSED1 != 0 {
            return CLPARSE_END_UF_UNUSED1;
        }

        *out_bits = bits;
        CLPARSE_OK
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:448

unsafe fn clfte_parse_baseline(es: *mut EntityState, d: &mut Detail) -> Raise {
    // SAFETY: `es` points at a live `entity_state_t`; `nullentitystate` is a
    // C-owned global.
    unsafe {
        let mut bits: c_uint = 0;
        clfte_read_delta(
            0,
            es,
            ptr::addr_of!(nullentitystate),
            ptr::addr_of!(nullentitystate),
            &mut bits,
            d,
        )
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:465

unsafe fn cl_entity_lerp_updated(
    ent: *mut Entity,
    oldframe: c_int,
    forcelink: bool,
    snap_anim: bool,
) {
    // SAFETY: `ent` points at a live `entity_t`; `cl` is C-owned.
    unsafe {
        let duration = if (*ent).lerp.frame_finish_time > (*cl_p()).mtime[0] {
            (*ent).lerp.frame_finish_time - (*cl_p()).mtime[0]
        } else {
            0.1
        };

        if (*ent).frame != oldframe || forcelink || snap_anim {
            if forcelink || snap_anim {
                (*ent).lerp.prev_frame = (*ent).frame;
                (*ent).lerp.frame_change_time = 0.0;
                if forcelink {
                    (*ent).lerp.snap_frames = 0;
                }
            } else if (*ent).lerp.snap_frames > 0 {
                (*ent).lerp.snap_frames -= 1;
                (*ent).lerp.prev_frame = (*ent).frame;
                (*ent).lerp.frame_change_time = (*cl_p()).mtime[0];
            } else {
                (*ent).lerp.prev_frame = oldframe;
                (*ent).lerp.frame_change_time = (*cl_p()).mtime[0];
            }
            (*ent).lerp.frame_duration = duration;
        }

        if !m::vector_compare(&(*ent).msg_origins[0], &(*ent).msg_origins[1])
            || !m::vector_compare(&(*ent).msg_angles[0], &(*ent).msg_angles[1])
        {
            let mut teleport = false;
            for j in 0..3 {
                // COMPAT: ADR-010 -- the subtraction happens in float, then
                // promotes to double for fabs.
                if libm::fabs(((*ent).msg_origins[0][j] - (*ent).msg_origins[1][j]) as f64) > 100.0
                {
                    teleport = true; // johnfitz -- don't lerp teleports
                }
            }

            if forcelink || teleport {
                (*ent).lerp.prev_origin = (*ent).msg_origins[0];
                (*ent).lerp.prev_angles = (*ent).msg_angles[0];
                (*ent).lerp.move_change_time = 0.0;
            } else {
                (*ent).lerp.prev_origin = (*ent).msg_origins[1];
                (*ent).lerp.prev_angles = (*ent).msg_angles[1];
                (*ent).lerp.move_change_time = (*cl_p()).mtime[0];
            }
            (*ent).lerp.move_duration = duration;
        }
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:520

unsafe fn cl_entities_deltaed(d: &mut Detail) -> Raise {
    // SAFETY: `cl` is C-owned; every entity pointer comes from
    // `cl_entity_num`, which grows the array exactly where C does.
    unsafe {
        let mut newnum: c_int = 1;
        while newnum < (*cl_p()).num_entities {
            let mut ent: *mut Entity = ptr::null_mut();
            raise!(cl_entity_num(newnum, &mut ent, d));
            if (*ent).update_type == 0 {
                newnum += 1;
                continue; // not interested in this one
            }

            let oldframe = (*ent).frame;
            let mut snap_anim = false;
            let mut forcelink;

            if (*ent).msgtime == (*cl_p()).mtime[0] {
                forcelink = false; // update got fragmented, don't dirty anything.
            } else {
                forcelink = (*ent).msgtime != (*cl_p()).mtime[1];

                // johnfitz -- lerping
                if (*ent).msgtime + 0.2 < (*cl_p()).mtime[0] {
                    snap_anim = true;
                }

                (*ent).msgtime = (*cl_p()).mtime[0];

                // shift the known values for interpolation
                (*ent).msg_origins[1] = (*ent).msg_origins[0];
                (*ent).msg_angles[1] = (*ent).msg_angles[0];

                (*ent).msg_origins[0] = (*ent).netstate.origin;
                (*ent).msg_angles[0] = (*ent).netstate.angles;
            }
            let skin = (*ent).netstate.skin as c_int;
            if skin != (*ent).skinnum {
                (*ent).skinnum = skin;
                if newnum > 0 && newnum <= (*cl_p()).maxclients {
                    guard!(d, g::ClParse_Glue_TranslateNewPlayerSkin(newnum - 1));
                }
            }
            (*ent).effects = (*ent).netstate.effects as c_int;

            // johnfitz -- lerping for movetype_step entities
            (*ent).lerp.movestep = ((*ent).netstate.eflags & EFLAGS_STEP) != 0;
            if (*ent).lerp.movestep {
                (*ent).forcelink = true;
            }

            (*ent).alpha = (*ent).netstate.alpha;
            (*ent).lerp.frame_finish_time = 0.0;
            if (*ent).netstate.lerp > 0 {
                // COMPAT: ADR-010 -- `(lerp - 1)` is int, `/ 1000.f` is a
                // float divide, and the sum is taken in double.
                (*ent).lerp.frame_finish_time = (*ent).msgtime
                    + (((*ent).netstate.lerp as c_int - 1) as c_float / 1000.0f32) as f64;
            }

            let model = cl_model_precache((*ent).netstate.modelindex as isize);
            if model != (*ent).model {
                g::R_FreeEntityBLAS(ent.cast());
                (*ent).model = model;
                invalidate_trace_line_cache();

                // automatic animation (torches, etc) can be either all
                // together or randomized
                if !model.is_null() {
                    if (*model).synctype == ST_FRAMETIME {
                        // COMPAT: ADR-010 -- negate in double, store as float.
                        (*ent).syncbase = (-(*cl_p()).time) as c_float;
                    } else if (*model).synctype == ST_RAND {
                        // COMPAT: ADR-010 -- `(float)COM_Rand () / COM_RAND_MAX`
                        // is a float divide by an exactly representable 2^24-1.
                        (*ent).syncbase = c::COM_Rand() as c_float / COM_RAND_MAX as c_float;
                    } else {
                        (*ent).syncbase = 0.0;
                    }
                } else {
                    forcelink = true; // hack to make null model players work
                }
                if newnum > 0 && newnum <= (*cl_p()).maxclients {
                    guard!(d, g::ClParse_Glue_TranslateNewPlayerSkin(newnum - 1));
                }

                snap_anim = true; // johnfitz -- don't lerp across model changes
            } else if !model.is_null()
                && (*model).synctype == ST_FRAMETIME
                && (*ent).frame != (*ent).netstate.frame as c_int
            {
                // COMPAT: ADR-010 -- see above.
                (*ent).syncbase = (-(*cl_p()).time) as c_float;
            }
            (*ent).frame = (*ent).netstate.frame as c_int;

            cl_entity_lerp_updated(ent, oldframe, forcelink, snap_anim);

            if forcelink {
                // didn't have an update last message
                (*ent).msg_origins[1] = (*ent).msg_origins[0];
                (*ent).origin = (*ent).msg_origins[0];
                (*ent).msg_angles[1] = (*ent).msg_angles[0];
                (*ent).angles = (*ent).msg_angles[0];
                (*ent).forcelink = true;
            }

            newnum += 1;
        }
        CLPARSE_OK
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:624

unsafe fn clfte_parse_entities_update(d: &mut Detail) -> Raise {
    // SAFETY: `cl`/`cls` are C-owned; every entity pointer comes from
    // `cl_entity_num`.
    unsafe {
        // so the server can know when we got it
        if !(*cls_p()).netcon.is_null() && (*cl_p()).ackframes_count < 8 {
            let n = (*cl_p()).ackframes_count as usize;
            (*cl_p()).ackframes[n] = g::NET_QSocketGetSequenceIn((*cls_p()).netcon.cast());
            (*cl_p()).ackframes_count = (*cl_p()).ackframes_count.wrapping_add(1);
        }

        if (*cl_p()).protocol_pext2 & PEXT2_PREDINFO != 0 {
            // an ack from our input sequences; strictly ascending-or-equal
            let mut seq = ((((*cl_p()).movemessages as c_uint) & 0xffff_0000)
                | ((MSG_ReadShort() as u16) as c_uint)) as c_int;
            if seq > (*cl_p()).movemessages {
                seq -= 0x10000;
            }
            (*cl_p()).ackedmovemessages = seq;
        }

        let newtime = MSG_ReadFloat();
        if newtime as f64 != (*cl_p()).mtime[0] {
            (*cl_p()).mtime[1] = (*cl_p()).mtime[0];
            (*cl_p()).mtime[0] = newtime as f64;
        }

        loop {
            let mut newnum: c_int = ((MSG_ReadShort() as i16) as u16) as c_int;
            let removeflag = (newnum & 0x8000) != 0;
            if newnum & 0x4000 != 0 {
                newnum = (newnum & 0x3fff) | (MSG_ReadByte() << 14);
            } else {
                newnum &= !0x8000;
            }

            if (newnum == 0 && !removeflag) || msg_badread() {
                break;
            }

            let mut ent: *mut Entity = ptr::null_mut();
            raise!(cl_entity_num(newnum, &mut ent, d));

            if removeflag {
                if cl_shownet_value() >= 3.0 {
                    c::Con_SafePrintf(c"%3i:     Remove %i\n".as_ptr(), msg_readcount(), newnum);
                }

                if newnum == 0 {
                    // removal of world -- forget all entities, a full reset
                    if cl_shownet_value() >= 3.0 {
                        c::Con_SafePrintf(c"%3i:     Reset all\n".as_ptr(), msg_readcount());
                    }
                    newnum = 1;
                    while newnum < (*cl_p()).num_entities {
                        let mut e: *mut Entity = ptr::null_mut();
                        raise!(cl_entity_num(newnum, &mut e, d));
                        (*e).netstate.pmovetype = 0;
                        raise!(cl_entity_num(newnum, &mut e, d));
                        (*e).model = ptr::null_mut();
                        raise!(cl_entity_num(newnum, &mut e, d));
                        (*e).update_type = 0;
                        newnum += 1;
                    }
                    (*cl_p()).requestresend = false; // we got it.
                    continue;
                }
                (*ent).update_type = 0; // no longer valid
                (*ent).model = ptr::null_mut();
                invalidate_trace_line_cache();
                continue;
            } else if (*ent).update_type != 0 {
                // simple update
                let mut bits: c_uint = 0;
                raise!(clfte_read_delta(
                    newnum as c_uint,
                    ptr::addr_of_mut!((*ent).netstate),
                    ptr::addr_of!((*ent).netstate),
                    ptr::addr_of!((*ent).baseline),
                    &mut bits,
                    d,
                ));
                if (*ent).msgtime == (*cl_p()).mtime[0] {
                    (*ent).msgtime = (*cl_p()).mtime[1];
                }
            } else {
                // we had no previous copy of this entity...
                (*ent).update_type = 1;
                let mut bits: c_uint = 0;
                raise!(clfte_read_delta(
                    newnum as c_uint,
                    ptr::addr_of_mut!((*ent).netstate),
                    ptr::null(),
                    ptr::addr_of!((*ent).baseline),
                    &mut bits,
                    d,
                ));
                (*ent).msgtime = 0.0;
            }
        }

        raise!(cl_entities_deltaed(d));

        if (*cl_p()).protocol_pext2 & PEXT2_PREDINFO != 0 {
            // stats should normally be sent before the entity data.
            (*cl_p()).mvelocity[1] = (*cl_p()).mvelocity[0];
            let mut ent: *mut Entity = ptr::null_mut();
            raise!(cl_entity_num((*cl_p()).viewentity, &mut ent, d));
            // COMPAT: ADR-010 -- `short * (1 / 8.0)` promotes to double.
            (*cl_p()).mvelocity[0][0] =
                ((*ent).netstate.velocity[0] as f64 * (1.0 / 8.0)) as c_float;
            (*cl_p()).mvelocity[0][1] =
                ((*ent).netstate.velocity[1] as f64 * (1.0 / 8.0)) as c_float;
            (*cl_p()).mvelocity[0][2] =
                ((*ent).netstate.velocity[2] as f64 * (1.0 / 8.0)) as c_float;
            (*cl_p()).onground = ((*ent).netstate.eflags & EFLAGS_ONGROUND) != 0;

            if cvar_value(ptr::addr_of!(gv::v_gunkick)) == 1.0 {
                // truncate away any extra precision, like vanilla/qs would.
                (*cl_p()).punchangle[0] = (*cl_p()).stats[STAT_PUNCHANGLE_X] as c_float;
                (*cl_p()).punchangle[1] = (*cl_p()).stats[STAT_PUNCHANGLE_Y] as c_float;
                (*cl_p()).punchangle[2] = (*cl_p()).stats[STAT_PUNCHANGLE_Z] as c_float;
            } else {
                // woo, more precision
                (*cl_p()).punchangle[0] = (*cl_p()).statsf[STAT_PUNCHANGLE_X];
                (*cl_p()).punchangle[1] = (*cl_p()).statsf[STAT_PUNCHANGLE_Y];
                (*cl_p()).punchangle[2] = (*cl_p()).statsf[STAT_PUNCHANGLE_Z];
            }
            let pa = ptr::addr_of_mut!(gv::v_punchangles).cast::<[c_float; 3]>();
            let pt = ptr::addr_of_mut!(gv::v_punchangles_times).cast::<f64>();
            if (*pa.add(0))[0] != (*cl_p()).punchangle[0]
                || (*pa.add(0))[1] != (*cl_p()).punchangle[1]
                || (*pa.add(0))[2] != (*cl_p()).punchangle[2]
            {
                *pt.add(1) = *pt.add(0);
                *pt.add(0) = newtime as f64;

                *pa.add(1) = *pa.add(0);
                *pa.add(0) = (*cl_p()).punchangle;
            }
        }

        if !(*cl_p()).requestresend && (*cls_p()).signon == SIGNONS - 1 {
            // first update is the final signon stage
            (*cls_p()).signon = SIGNONS;
            guard!(d, g::ClParse_Glue_SignonReply());
        }

        CLPARSE_OK
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:753

unsafe fn cl_parse_start_sound_packet(d: &mut Detail) -> Raise {
    // SAFETY: `cl` is C-owned; `pos` is a local whose address does not escape
    // `S_StartSound`.
    unsafe {
        let mut pos: [c_float; 3] = [0.0; 3];
        let channel: c_int;
        let ent: c_int;

        let mut field_mask = MSG_ReadByte();
        if field_mask & SND_FTE_MOREFLAGS != 0 {
            field_mask |= MSG_ReadByte() << 8;
        }

        let volume = if field_mask & SND_VOLUME != 0 {
            MSG_ReadByte()
        } else {
            DEFAULT_SOUND_PACKET_VOLUME
        };

        // COMPAT: ADR-010 -- `int / 64.0` is a double divide truncated to
        // float on assignment.
        let attenuation = if field_mask & SND_ATTENUATION != 0 {
            (MSG_ReadByte() as f64 / 64.0) as c_float
        } else {
            DEFAULT_SOUND_PACKET_ATTENUATION
        };

        // fte's sound extensions
        if (*cl_p()).protocol_pext2 & PEXT2_REPLACEMENTDELTAS != 0 {
            // our mixer can't deal with these, so just parse and ignore
            if field_mask & SND_FTE_PITCHADJ != 0 {
                MSG_ReadByte();
            }
            if field_mask & SND_FTE_TIMEOFS != 0 {
                MSG_ReadShort();
            }
            if field_mask & SND_FTE_VELOCITY != 0 {
                MSG_ReadShort();
                MSG_ReadShort();
                MSG_ReadShort();
            }
        } else if field_mask & (SND_FTE_MOREFLAGS | SND_FTE_PITCHADJ | SND_FTE_TIMEOFS) != 0 {
            c::Con_Warning(c"Unknown meaning for sound flags\n".as_ptr());
        }
        if (*cl_p()).protocol_pext2 & PEXT2_REPLACEMENTDELTAS != 0 {
            if field_mask & SND_DP_PITCH != 0 {
                MSG_ReadShort();
            }
        } else if field_mask & SND_DP_PITCH != 0 {
            c::Con_Warning(c"Unknown meaning for sound flags\n".as_ptr());
        }

        // johnfitz -- PROTOCOL_FITZQUAKE
        if field_mask & SND_LARGEENTITY != 0 {
            ent = (MSG_ReadShort() as u16) as c_int;
            channel = MSG_ReadByte();
        } else {
            let c = (MSG_ReadShort() as u16) as c_int;
            ent = c >> 3;
            channel = c & 7;
        }

        let sound_num = if field_mask & SND_LARGESOUND != 0 {
            (MSG_ReadShort() as u16) as c_int
        } else {
            MSG_ReadByte()
        };

        // johnfitz -- check soundnum
        if sound_num >= MAX_SOUNDS {
            d.a = sound_num;
            return CLPARSE_ERR_SOUNDNUM;
        }

        if ent > (*cl_p()).max_edicts {
            d.a = ent;
            return CLPARSE_ERR_SOUNDENT;
        }

        for p in pos.iter_mut() {
            *p = MSG_ReadCoord(protocolflags());
        }

        S_StartSound(
            ent,
            channel,
            cl_sound_precache(sound_num as isize).cast(),
            pos.as_mut_ptr(),
            // COMPAT: ADR-010 -- `int / 255.0` is a double divide narrowed to
            // the float parameter.
            (volume as f64 / 255.0) as c_float,
            attenuation,
        );
        CLPARSE_OK
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:840

unsafe fn cl_parse_local_sound(d: &mut Detail) -> Raise {
    // SAFETY: `cl` is C-owned; the sfx pointer comes straight out of it.
    unsafe {
        let field_mask = MSG_ReadByte();
        let sound_num = if field_mask & SND_LARGESOUND != 0 {
            MSG_ReadShort()
        } else {
            MSG_ReadByte()
        };
        if sound_num >= MAX_SOUNDS {
            d.a = sound_num;
            return CLPARSE_ERR_LOCALSOUND;
        }

        let sfx = cl_sound_precache(sound_num as isize);
        g::S_LocalSound(ptr::addr_of!((*sfx).name).cast::<c_char>());
        CLPARSE_OK
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:922

unsafe fn cl_parse_server_info(d: &mut Detail) -> Raise {
    // SAFETY: `cl`/`cls` are C-owned; the four precache buffers are
    // module-level statics with process lifetime.
    unsafe {
        let mut gamedirswitchwarning = false;

        c::Con_DPrintf(c"Serverinfo packet received.\n".as_ptr());

        // ericw -- bring up loading plaque for map changes within a demo.
        if (*cls_p()).demoplayback {
            guard!(d, g::ClParse_Glue_BeginLoadingPlaque());
        }

        // wipe the client_state_t struct
        guard!(d, g::ClParse_Glue_ClearState());

        if ptr::addr_of!(crate::sv_main::sv.loadgame).read() {
            V_StopPitchDrift();
        }

        guard!(d, g::ClParse_Glue_KeyClearStates());
        g::IN_ClearStates();

        // parse protocol version number
        let mut i: c_int;
        loop {
            i = MSG_ReadLong();
            if i == PROTOCOL_FTE_PEXT1 {
                (*cl_p()).protocol_pext1 = MSG_ReadLong() as c_uint;
                if (*cl_p()).protocol_pext1 & !PEXT1_ACCEPTED_CLIENT != 0 {
                    d.a = ((*cl_p()).protocol_pext1 & !PEXT1_SUPPORTED_CLIENT) as c_int;
                    return CLPARSE_ERR_PEXT1;
                }
                continue;
            }
            if i == PROTOCOL_FTE_PEXT2 {
                (*cl_p()).protocol_pext2 = MSG_ReadLong() as c_uint;
                if (*cl_p()).protocol_pext2 & !PEXT2_ACCEPTED_CLIENT != 0 {
                    d.a = ((*cl_p()).protocol_pext2 & !PEXT2_SUPPORTED_CLIENT) as c_int;
                    return CLPARSE_ERR_PEXT2;
                }
                continue;
            }
            break;
        }

        // johnfitz -- support multiple protocols
        if i as c_uint != PROTOCOL_NETQUAKE
            && i as c_uint != PROTOCOL_FITZQUAKE
            && i as c_uint != PROTOCOL_RMQ
        {
            c::Con_Printf(c"\n".as_ptr()); // no newline after serverinfo print
            d.a = i;
            return CLPARSE_ERR_VERSION;
        }
        (*cl_p()).protocol = i as c_uint;

        if (*cl_p()).protocol == PROTOCOL_RMQ {
            let supportedflags: c_uint = PRFL_SHORTANGLE
                | PRFL_FLOATANGLE
                | PRFL_24BITCOORD
                | PRFL_FLOATCOORD
                | PRFL_EDICTSCALE
                | PRFL_INT32COORD;

            (*cl_p()).protocolflags = MSG_ReadLong() as c_uint;

            if 0 != ((*cl_p()).protocolflags & !supportedflags) {
                c::Con_Warning(
                    c"PROTOCOL_RMQ protocolflags %i contains unsupported flags\n".as_ptr(),
                    (*cl_p()).protocolflags,
                );
            }
        } else {
            (*cl_p()).protocolflags = 0;
        }

        let gamedir = ptr::addr_of_mut!(GAMEDIR).cast::<c_char>();
        *gamedir = 0;
        if (*cl_p()).protocol_pext2 & PEXT2_PREDINFO != 0 {
            g::q_strlcpy(gamedir, MSG_ReadString(), 1024);
            if !g::COM_GameDirMatches(gamedir) {
                gamedirswitchwarning = true;
            }
        }

        // parse maxclients
        (*cl_p()).maxclients = MSG_ReadByte();
        if (*cl_p()).maxclients < 1 || (*cl_p()).maxclients > MAX_SCOREBOARD {
            d.a = (*cl_p()).maxclients;
            return CLPARSE_ERR_MAXCLIENTS;
        }
        (*cl_p()).scores =
            c::Mem_Alloc((*cl_p()).maxclients as usize * core::mem::size_of::<ScoreBoard>())
                .cast::<ScoreBoard>();

        // parse gametype
        (*cl_p()).gametype = MSG_ReadByte();

        // parse signon message
        let mut str_ = MSG_ReadString();
        g::q_strlcpy(
            ptr::addr_of_mut!((*cl_p()).levelname).cast::<c_char>(),
            str_,
            128,
        );

        // seperate the printfs so the server message can have a color
        c::Con_Printf(c"\n%s\n".as_ptr(), g::Con_Quakebar(40));
        c::Con_Printf(c"%c%s\n".as_ptr(), 2, str_);

        // johnfitz -- tell user which protocol this is
        let protname = ptr::addr_of_mut!(PROTNAME).cast::<c_char>();
        if (*cl_p()).protocol_pext2 & PEXT2_REPLACEMENTDELTAS != 0 {
            g::q_snprintf(protname, 64, c"fte%i".as_ptr(), (*cl_p()).protocol);
        } else {
            g::q_snprintf(protname, 64, c"%i".as_ptr(), (*cl_p()).protocol);
        }
        c::Con_Printf(c"Using protocol %s\n".as_ptr(), protname);
        if gamedirswitchwarning {
            c::Con_Warning(
                c"gamedir mismatch: server \"%s\" ours \"%s\"\n".as_ptr(),
                gamedir,
                COM_GetGameNames(false),
            );
        }

        // precache models
        ptr::write_bytes(
            ptr::addr_of_mut!((*cl_p()).model_precache).cast::<u8>(),
            0,
            MAX_MODELS as usize * core::mem::size_of::<*mut QModel>(),
        );
        let mut nummodels: c_int = 1;
        loop {
            str_ = MSG_ReadString();
            if *str_ == 0 {
                break;
            }
            if nummodels == MAX_MODELS {
                return CLPARSE_ERR_TOOMANYMODELS;
            }
            g::q_strlcpy(model_precache_name(nummodels as usize), str_, MAX_QPATH);
            g::Mod_TouchModel(str_);
            nummodels += 1;
        }

        // johnfitz -- check for excessive models
        if nummodels >= 4096 {
            c::Con_Warning(
                c"%i models exceeds QS limit of 4096 (max = %d).\n".as_ptr(),
                nummodels,
                MAX_MODELS,
            );
        } else if nummodels >= 256 {
            c::Con_DWarning(
                c"%i models exceeds standard limit of 256 (max = %d).\n".as_ptr(),
                nummodels,
                MAX_MODELS,
            );
        }

        // precache sounds
        ptr::write_bytes(
            ptr::addr_of_mut!((*cl_p()).sound_precache).cast::<u8>(),
            0,
            MAX_SOUNDS as usize * core::mem::size_of::<*mut Sfx>(),
        );
        let mut numsounds: c_int = 1;
        loop {
            str_ = MSG_ReadString();
            if *str_ == 0 {
                break;
            }
            if numsounds == MAX_SOUNDS {
                return CLPARSE_ERR_TOOMANYSOUNDS;
            }
            g::q_strlcpy(sound_precache_name(numsounds as usize), str_, MAX_QPATH);
            g::S_TouchSound(str_);
            numsounds += 1;
        }

        // johnfitz -- check for excessive sounds
        if numsounds >= 256 {
            c::Con_DWarning(
                c"%i sounds exceeds standard limit of 256 (max = %d).\n".as_ptr(),
                numsounds,
                MAX_SOUNDS,
            );
        }

        // copy the naked name of the map file to the cl structure -- O.S
        c::COM_StripExtension(
            c::COM_SkipPath(model_precache_name(1)),
            ptr::addr_of_mut!((*cl_p()).mapname).cast::<c_char>(),
            128,
        );

        let mut i = 1;
        while i < nummodels {
            let mut mdl: *mut c_void = ptr::null_mut();
            guard!(
                d,
                g::ClParse_Glue_ModForName(model_precache_name(i as usize), &mut mdl)
            );
            (*cl_p()).model_precache[i as usize] = mdl.cast::<QModel>();
            if (*cl_p()).model_precache[i as usize].is_null() {
                d.s = model_precache_name(i as usize);
                return CLPARSE_ERR_MODELNOTFOUND;
            }
            i += 1;
        }
        g::S_BeginPrecaching();
        let mut i = 1;
        while i < numsounds {
            (*cl_p()).sound_precache[i as usize] =
                S_PrecacheSound(sound_precache_name(i as usize)).cast::<Sfx>();
            i += 1;
        }
        g::S_EndPrecaching();

        // local state
        let world = (*cl_p()).model_precache[1];
        (*cl_p()).worldmodel = world;
        (*cl_entity(0)).model = world;

        guard!(d, g::ClParse_Glue_NewMap());

        // johnfitz -- clear out string
        *ptr::addr_of_mut!(g::con_lastcenterstring).cast::<c_char>() = 0;

        ptr::addr_of_mut!(gv::noclip_anglehack).write(false);

        WARN_ABOUT_NEHAHRA_PROTOCOL = true;

        // johnfitz -- reset developer stats
        ptr::write_bytes(
            ptr::addr_of_mut!(g::dev_stats).cast::<u8>(),
            0,
            core::mem::size_of::<g::devstats_t>(),
        );
        ptr::write_bytes(
            ptr::addr_of_mut!(g::dev_peakstats).cast::<u8>(),
            0,
            core::mem::size_of::<g::devstats_t>(),
        );
        ptr::write_bytes(
            ptr::addr_of_mut!(dev_overflows).cast::<u8>(),
            0,
            core::mem::size_of::<OverflowTimes>(),
        );

        (*cl_p()).requestresend = true;
        (*cl_p()).ackframes_count = 0;
        if (*cl_p()).protocol_pext2 & PEXT2_REPLACEMENTDELTAS != 0 {
            let n = (*cl_p()).ackframes_count as usize;
            (*cl_p()).ackframes[n] = -1;
            (*cl_p()).ackframes_count = (*cl_p()).ackframes_count.wrapping_add(1);
        }
        if (*cl_p()).protocol_pext2 != 0 || ((*cl_p()).protocol_pext1 & PEXT1_CSQC) != 0 {
            (*cl_p()).protocol_particles = true;
        }

        CLPARSE_OK
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:1145

unsafe fn cl_parse_update(mut bits: c_int, d: &mut Detail) -> Raise {
    // SAFETY: `cl`/`cls` are C-owned; `ent` comes from `cl_entity_num`.
    unsafe {
        let mut snap_anim = false;
        let mut forcelink;
        let modnum: c_uint;
        let mut modnum_mut: c_uint;

        if (*cls_p()).signon == SIGNONS - 1 {
            // first update is the final signon stage
            (*cls_p()).signon = SIGNONS;
            guard!(d, g::ClParse_Glue_SignonReply());
        }

        if bits & U_MOREBITS != 0 {
            let i = MSG_ReadByte();
            bits |= i << 8;
        }

        // johnfitz -- PROTOCOL_FITZQUAKE
        if (*cl_p()).protocol == PROTOCOL_FITZQUAKE || (*cl_p()).protocol == PROTOCOL_RMQ {
            if bits & U_EXTEND1 != 0 {
                bits |= MSG_ReadByte() << 16;
            }
            if bits & U_EXTEND2 != 0 {
                bits |= MSG_ReadByte() << 24;
            }
        }

        let num = if bits & U_LONGENTITY != 0 {
            MSG_ReadShort()
        } else {
            MSG_ReadByte()
        };

        let mut ent: *mut Entity = ptr::null_mut();
        raise!(cl_entity_num(num, &mut ent, d));
        let oldframe = (*ent).frame;

        forcelink = (*ent).msgtime != (*cl_p()).mtime[1];

        // johnfitz -- lerping
        if (*ent).msgtime + 0.2 < (*cl_p()).mtime[0] {
            snap_anim = true;
        }

        (*ent).msgtime = (*cl_p()).mtime[0];

        // copy the baseline into the netstate for the rest of the code to use.
        const MI_OFF: usize = core::mem::offset_of!(EntityState, modelindex);
        ptr::copy_nonoverlapping(
            ptr::addr_of!((*ent).baseline).cast::<u8>().add(MI_OFF),
            ptr::addr_of_mut!((*ent).netstate).cast::<u8>().add(MI_OFF),
            core::mem::size_of::<EntityState>() - MI_OFF,
        );

        if bits & U_MODEL != 0 {
            modnum = MSG_ReadByte() as c_uint;
            if modnum >= MAX_MODELS as c_uint {
                return CLPARSE_ERR_BADMODNUM;
            }
        } else {
            modnum = (*ent).baseline.modelindex as c_uint;
        }
        modnum_mut = modnum;

        if bits & U_FRAME != 0 {
            (*ent).frame = MSG_ReadByte();
        } else {
            (*ent).frame = (*ent).baseline.frame as c_int;
        }

        if bits & U_COLORMAP != 0 {
            (*ent).netstate.colormap = MSG_ReadByte() as u8;
        }
        let skin = if bits & U_SKIN != 0 {
            MSG_ReadByte()
        } else {
            (*ent).baseline.skin as c_int
        };
        if skin != (*ent).skinnum {
            (*ent).skinnum = skin;
            if num > 0 && num <= (*cl_p()).maxclients {
                guard!(d, g::ClParse_Glue_TranslateNewPlayerSkin(num - 1));
            }
        }
        if bits & U_EFFECTS != 0 {
            (*ent).effects = MSG_ReadByte();
        } else {
            (*ent).effects = (*ent).baseline.effects as c_int;
        }

        // shift the known values for interpolation
        (*ent).msg_origins[1] = (*ent).msg_origins[0];
        (*ent).msg_angles[1] = (*ent).msg_angles[0];

        if bits & U_ORIGIN1 != 0 {
            (*ent).msg_origins[0][0] = MSG_ReadCoord(protocolflags());
        } else {
            (*ent).msg_origins[0][0] = (*ent).baseline.origin[0];
        }
        if bits & U_ANGLE1 != 0 {
            (*ent).msg_angles[0][0] = MSG_ReadAngle(protocolflags());
        } else {
            (*ent).msg_angles[0][0] = (*ent).baseline.angles[0];
        }

        if bits & U_ORIGIN2 != 0 {
            (*ent).msg_origins[0][1] = MSG_ReadCoord(protocolflags());
        } else {
            (*ent).msg_origins[0][1] = (*ent).baseline.origin[1];
        }
        if bits & U_ANGLE2 != 0 {
            (*ent).msg_angles[0][1] = MSG_ReadAngle(protocolflags());
        } else {
            (*ent).msg_angles[0][1] = (*ent).baseline.angles[1];
        }

        if bits & U_ORIGIN3 != 0 {
            (*ent).msg_origins[0][2] = MSG_ReadCoord(protocolflags());
        } else {
            (*ent).msg_origins[0][2] = (*ent).baseline.origin[2];
        }
        if bits & U_ANGLE3 != 0 {
            (*ent).msg_angles[0][2] = MSG_ReadAngle(protocolflags());
        } else {
            (*ent).msg_angles[0][2] = (*ent).baseline.angles[2];
        }

        // johnfitz -- lerping for movetype_step entities
        (*ent).lerp.movestep = (bits & U_STEP) != 0;
        if (*ent).lerp.movestep {
            (*ent).forcelink = true;
        }

        // johnfitz -- PROTOCOL_FITZQUAKE and PROTOCOL_NEHAHRA
        if (*cl_p()).protocol == PROTOCOL_FITZQUAKE || (*cl_p()).protocol == PROTOCOL_RMQ {
            if bits & U_ALPHA != 0 {
                (*ent).alpha = MSG_ReadByte() as u8;
            } else {
                (*ent).alpha = (*ent).baseline.alpha;
            }
            if bits & U_SCALE != 0 {
                (*ent).netstate.scale = MSG_ReadByte() as u8; // PROTOCOL_RMQ
            }
            if bits & U_FRAME2 != 0 {
                (*ent).frame = ((*ent).frame & 0x00FF) | (MSG_ReadByte() << 8);
            }
            if bits & U_MODEL2 != 0 {
                modnum_mut = (modnum_mut & 0x00FF) | ((MSG_ReadByte() << 8) as c_uint);
                if modnum_mut >= MAX_MODELS as c_uint {
                    return CLPARSE_ERR_BADMODNUM;
                }
            }
            if bits & U_LERPFINISH != 0 {
                // COMPAT: ADR-010 -- `(float)byte / 255` is a float divide,
                // widened for the sum with the double msgtime.
                (*ent).lerp.frame_finish_time =
                    (*ent).msgtime + ((MSG_ReadByte() as c_float) / 255.0f32) as f64;
            } else {
                (*ent).lerp.frame_finish_time = 0.0;
            }
        } else if (*cl_p()).protocol == PROTOCOL_NETQUAKE {
            // HACK: if this bit is set, assume this is PROTOCOL_NEHAHRA
            if bits & U_TRANS != 0 {
                if (*cl_p()).protocol == PROTOCOL_NETQUAKE && WARN_ABOUT_NEHAHRA_PROTOCOL {
                    c::Con_Warning(c"nonstandard update bit, assuming Nehahra protocol\n".as_ptr());
                    WARN_ABOUT_NEHAHRA_PROTOCOL = false;
                }

                let a = MSG_ReadFloat();
                let b = MSG_ReadFloat(); // alpha
                if a == 2.0 {
                    MSG_ReadFloat(); // fullbright (not using this yet)
                }
                (*ent).alpha = entalpha_encode(b) as u8;
            } else {
                (*ent).alpha = (*ent).baseline.alpha;
            }
        } else {
            (*ent).alpha = (*ent).baseline.alpha;
        }

        // johnfitz -- moved here from above
        let model = cl_model_precache(modnum_mut as isize);
        if model != (*ent).model {
            g::R_FreeEntityBLAS(ent.cast());
            (*ent).model = model;
            invalidate_trace_line_cache();
            // automatic animation (torches, etc)
            if !model.is_null() {
                if (*model).synctype == ST_RAND {
                    // COMPAT: ADR-010 -- float divide by 2^24-1.
                    (*ent).syncbase = c::COM_Rand() as c_float / COM_RAND_MAX as c_float;
                } else {
                    (*ent).syncbase = 0.0;
                }
            } else {
                forcelink = true; // hack to make null model players work
            }
            if num > 0 && num <= (*cl_p()).maxclients {
                guard!(d, g::ClParse_Glue_TranslateNewPlayerSkin(num - 1));
            }

            snap_anim = true; // johnfitz -- don't lerp across model changes
        }

        cl_entity_lerp_updated(ent, oldframe, forcelink, snap_anim);

        if forcelink {
            // didn't have an update last message
            (*ent).msg_origins[1] = (*ent).msg_origins[0];
            (*ent).origin = (*ent).msg_origins[0];
            (*ent).msg_angles[1] = (*ent).msg_angles[0];
            (*ent).angles = (*ent).msg_angles[0];
            (*ent).forcelink = true;
        }

        CLPARSE_OK
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:1363

unsafe fn cl_parse_baseline(ent: *mut Entity, version: c_int, d: &mut Detail) -> Raise {
    // SAFETY: `ent` points at a live `entity_t`.
    unsafe {
        if version == 6 {
            return clfte_parse_baseline(ptr::addr_of_mut!((*ent).baseline), d);
        }

        (*ent).baseline = ptr::addr_of!(nullentitystate).read();

        // johnfitz -- PROTOCOL_FITZQUAKE
        let bits: c_int = if version == 7 {
            B_LARGEMODEL | B_LARGEFRAME // dpp7's spawnstatic2
        } else if version == 2 {
            MSG_ReadByte()
        } else {
            0
        };
        (*ent).baseline.modelindex = if bits & B_LARGEMODEL != 0 {
            MSG_ReadShort() as u16
        } else {
            MSG_ReadByte() as u16
        };
        (*ent).baseline.frame = if bits & B_LARGEFRAME != 0 {
            MSG_ReadShort() as u16
        } else {
            MSG_ReadByte() as u16
        };

        (*ent).baseline.colormap = MSG_ReadByte() as u8;
        (*ent).baseline.skin = MSG_ReadByte() as u8;
        for i in 0..3 {
            (*ent).baseline.origin[i] = MSG_ReadCoord(protocolflags());
            (*ent).baseline.angles[i] = MSG_ReadAngle(protocolflags());
        }

        (*ent).baseline.alpha = if bits & B_ALPHA != 0 {
            MSG_ReadByte() as u8
        } else {
            ENTALPHA_DEFAULT as u8
        };
        (*ent).baseline.scale = if bits & B_SCALE != 0 {
            MSG_ReadByte() as u8
        } else {
            ENTSCALE_DEFAULT as u8
        };

        CLPARSE_OK
    }
}

/// `cl_parse.c:1397` `CL_SetStati` / `CL_SetHudStat`. The C macro's value is
/// the `int` assignment's result, so `statsf` gets `(float)(int)val`.
#[inline]
unsafe fn cl_set_stati(stat: usize, val: c_int) {
    // SAFETY: `stat` is a compile-time constant below `MAX_CL_STATS`.
    unsafe {
        (*cl_p()).stats[stat] = val;
        (*cl_p()).statsf[stat] = val as c_float;
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:1407

unsafe fn cl_parse_clientdata() {
    // SAFETY: `cl` is C-owned; all reads come from the C message buffer.
    unsafe {
        let mut bits: c_int = (MSG_ReadShort() as u16) as c_int;

        // johnfitz -- PROTOCOL_FITZQUAKE
        if bits & SU_EXTEND1 != 0 {
            bits |= MSG_ReadByte() << 16;
        }
        if bits & SU_EXTEND2 != 0 {
            bits |= MSG_ReadByte() << 24;
        }

        bits |= SU_ITEMS;

        if bits & SU_VIEWHEIGHT != 0 {
            cl_set_stati(STAT_VIEWHEIGHT, MSG_ReadChar());
        } else {
            cl_set_stati(STAT_VIEWHEIGHT, DEFAULT_VIEWHEIGHT);
        }

        if bits & SU_IDEALPITCH != 0 {
            cl_set_stati(STAT_IDEALPITCH, MSG_ReadChar());
        } else {
            cl_set_stati(STAT_IDEALPITCH, 0);
        }

        (*cl_p()).mvelocity[1] = (*cl_p()).mvelocity[0];
        for i in 0..3 {
            if bits & (SU_PUNCH1 << i) != 0 {
                (*cl_p()).punchangle[i as usize] = MSG_ReadChar() as c_float;
            } else {
                (*cl_p()).punchangle[i as usize] = 0.0;
            }

            if bits & (SU_VELOCITY1 << i) != 0 {
                (*cl_p()).mvelocity[0][i as usize] = (MSG_ReadChar() * 16) as c_float;
            } else {
                (*cl_p()).mvelocity[0][i as usize] = 0.0;
            }
        }

        // johnfitz -- update v_punchangles
        let pa = ptr::addr_of_mut!(gv::v_punchangles).cast::<[c_float; 3]>();
        let pt = ptr::addr_of_mut!(gv::v_punchangles_times).cast::<f64>();
        if (*pa.add(0))[0] != (*cl_p()).punchangle[0]
            || (*pa.add(0))[1] != (*cl_p()).punchangle[1]
            || (*pa.add(0))[2] != (*cl_p()).punchangle[2]
        {
            *pt.add(1) = *pt.add(0);
            *pt.add(0) = (*cl_p()).mtime[0];
            *pa.add(1) = *pa.add(0);
            *pa.add(0) = (*cl_p()).punchangle;
        }

        if bits & SU_ITEMS != 0 {
            cl_set_stati(STAT_ITEMS, MSG_ReadLong());
        }

        (*cl_p()).onground = (bits & SU_ONGROUND) != 0;
        (*cl_p()).inwater = (bits & SU_INWATER) != 0;

        {
            let mut weaponframe: u16 = 0;
            let mut armourval: u16 = 0;
            let mut weaponmodel: u16 = 0;
            let mut activeweapon: c_uint;
            let mut ammo: u16;
            let mut ammovals: [u16; 4] = [0; 4];

            if bits & SU_WEAPONFRAME != 0 {
                weaponframe = MSG_ReadByte() as u16;
            }
            if bits & SU_ARMOR != 0 {
                armourval = MSG_ReadByte() as u16;
            }
            if bits & SU_WEAPON != 0 {
                weaponmodel = MSG_ReadByte() as u16;
            }
            let health = MSG_ReadShort() as i16;
            ammo = MSG_ReadByte() as u16;
            for a in ammovals.iter_mut() {
                *a = MSG_ReadByte() as u16;
            }
            activeweapon = MSG_ReadByte() as c_uint;
            if !ptr::addr_of!(g::standard_quake).read() {
                activeweapon = 1u32 << activeweapon;
            }

            // johnfitz -- PROTOCOL_FITZQUAKE
            if bits & SU_WEAPON2 != 0 {
                weaponmodel |= (MSG_ReadByte() << 8) as u16;
            }
            if bits & SU_ARMOR2 != 0 {
                armourval |= (MSG_ReadByte() << 8) as u16;
            }
            if bits & SU_AMMO2 != 0 {
                ammo |= (MSG_ReadByte() << 8) as u16;
            }
            if bits & SU_SHELLS2 != 0 {
                ammovals[0] |= (MSG_ReadByte() << 8) as u16;
            }
            if bits & SU_NAILS2 != 0 {
                ammovals[1] |= (MSG_ReadByte() << 8) as u16;
            }
            if bits & SU_ROCKETS2 != 0 {
                ammovals[2] |= (MSG_ReadByte() << 8) as u16;
            }
            if bits & SU_CELLS2 != 0 {
                ammovals[3] |= (MSG_ReadByte() << 8) as u16;
            }
            if bits & SU_WEAPONFRAME2 != 0 {
                weaponframe |= (MSG_ReadByte() << 8) as u16;
            }
            if bits & SU_WEAPONALPHA != 0 {
                (*cl_viewent()).alpha = MSG_ReadByte() as u8;
            } else {
                (*cl_viewent()).alpha = ENTALPHA_DEFAULT as u8;
            }

            cl_set_stati(STAT_WEAPONFRAME, weaponframe as c_int);
            cl_set_stati(STAT_ARMOR, armourval as c_int);
            cl_set_stati(STAT_WEAPON, weaponmodel as c_int);
            cl_set_stati(STAT_ACTIVEWEAPON, activeweapon as c_int);
            cl_set_stati(STAT_HEALTH, health as c_int);
            cl_set_stati(STAT_AMMO, ammo as c_int);
            cl_set_stati(STAT_SHELLS, ammovals[0] as c_int);
            cl_set_stati(STAT_NAILS, ammovals[1] as c_int);
            cl_set_stati(STAT_ROCKETS, ammovals[2] as c_int);
            cl_set_stati(STAT_CELLS, ammovals[3] as c_int);
        }
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:1527

unsafe fn cl_new_translation(slot: c_int, d: &mut Detail) -> Raise {
    // SAFETY: `cl` is C-owned. `Sys_Error` aborts rather than jumping, so it
    // is safe to call from a Rust frame (ADR-009).
    unsafe {
        if slot > (*cl_p()).maxclients {
            c::Sys_Error(c"CL_NewTranslation: slot > cl.maxclients".as_ptr());
        }
        guard!(d, g::ClParse_Glue_TranslatePlayerSkin(slot));
        CLPARSE_OK
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:1539

unsafe fn cl_parse_static(version: c_int, d: &mut Detail) -> Raise {
    // SAFETY: `cl` is C-owned; the growth block mirrors C's allocation
    // arithmetic exactly.
    unsafe {
        let i = (*cl_p()).num_statics;
        if i >= (*cl_p()).max_static_entities {
            let mut ec: c_int = 64;
            let newstatics = c::Mem_Realloc(
                (*cl_p()).static_entities.cast::<c_void>(),
                core::mem::size_of::<*mut EntityOpaque>()
                    * ((*cl_p()).max_static_entities + ec) as usize,
            )
            .cast::<*mut EntityOpaque>();
            let mut newents = c::Mem_Alloc(core::mem::size_of::<EntityOpaque>() * ec as usize)
                .cast::<EntityOpaque>();
            if newstatics.is_null() || newents.is_null() {
                return CLPARSE_ERR_TOOMANYSTATICS;
            }
            (*cl_p()).static_entities = newstatics;
            loop {
                let old = ec;
                ec -= 1;
                if old == 0 {
                    break;
                }
                let n = (*cl_p()).max_static_entities;
                *(*cl_p()).static_entities.offset(n as isize) = newents;
                (*cl_p()).max_static_entities = n + 1;
                newents = newents.add(1);
            }
        }

        let ent = (*(*cl_p()).static_entities.offset(i as isize)).cast::<Entity>();
        (*cl_p()).num_statics += 1;
        raise!(cl_parse_baseline(ent, version, d));

        // copy it to the current state
        (*ent).netstate = (*ent).baseline;
        (*ent).eflags = (*ent).netstate.eflags;

        (*ent).model = cl_model_precache((*ent).baseline.modelindex as isize);
        (*ent).frame = (*ent).baseline.frame as c_int;
        (*ent).lerp.prev_frame = (*ent).frame; // johnfitz -- lerping

        (*ent).skinnum = (*ent).baseline.skin as c_int;
        (*ent).effects = (*ent).baseline.effects as c_int;
        (*ent).alpha = (*ent).baseline.alpha; // johnfitz -- alpha

        (*ent).origin = (*ent).baseline.origin;
        (*ent).angles = (*ent).baseline.angles;
        if !(*ent).model.is_null() {
            guard!(d, g::ClParse_Glue_AddEfrags(ent.cast()));
        }

        invalidate_trace_line_cache();
        CLPARSE_OK
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:1587

unsafe fn cl_parse_static_sound(version: c_int) {
    // SAFETY: `cl` is C-owned; `org` is a local whose address does not escape
    // `S_StaticSound`.
    unsafe {
        let mut org: [c_float; 3] = [0.0; 3];
        for o in org.iter_mut() {
            *o = MSG_ReadCoord(protocolflags());
        }

        // johnfitz -- PROTOCOL_FITZQUAKE
        let sound_num = if version == 2 {
            MSG_ReadShort()
        } else {
            MSG_ReadByte()
        };

        let vol = MSG_ReadByte();
        let atten = MSG_ReadByte();

        S_StaticSound(
            cl_sound_precache(sound_num as isize).cast(),
            org.as_mut_ptr(),
            vol,
            atten as c_float,
        );
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:1614

unsafe fn cl_parse_precache(d: &mut Detail) -> Raise {
    // SAFETY: `cl` is C-owned; every index is bounds-checked exactly as C
    // checks it.
    unsafe {
        let code = MSG_ReadShort() as u16;
        let index = (code & 0x3fff) as c_uint;
        let name = MSG_ReadString();
        match ((code >> 14) & 0x3) as c_int {
            0 => {
                // models
                if index < MAX_MODELS as c_uint {
                    let mut mdl: *mut c_void = ptr::null_mut();
                    guard!(d, g::ClParse_Glue_ModForName(name, &mut mdl));
                    (*cl_p()).model_precache[index as usize] = mdl.cast::<QModel>();
                }
            }
            1 => {
                // particles
                if index < MAX_PARTICLETYPES as c_uint {
                    let p = cl_particle_precache(index as isize);
                    if *name != 0 {
                        (*p).name = c::cvar_cmd::q_strdup(name);
                        let mut out: c_int = 0;
                        guard!(d, g::ClParse_Glue_FindParticleType((*p).name, &mut out));
                        (*p).index = out;
                    } else {
                        c::Mem_Free((*p).name.cast::<c_void>());
                        (*p).name = ptr::null();
                        (*p).index = -1;
                    }
                }
            }
            2 => {
                // sounds
                if index < MAX_SOUNDS as c_uint {
                    (*cl_p()).sound_precache[index as usize] = S_PrecacheSound(name).cast::<Sfx>();
                }
            }
            _ => {
                c::Con_Warning(c"CL_ParsePrecache: unsupported precache type\n".as_ptr());
            }
        }
        CLPARSE_OK
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:1659

unsafe fn cl_force_protocol_particles(d: &mut Detail) -> Raise {
    // SAFETY: `cl` is C-owned.
    unsafe {
        (*cl_p()).protocol_particles = true;
        guard!(d, g::ClParse_Glue_EffectinfoEnumerate());
        c::Con_Warning(c"Received svcdp_pointparticles1 but extension not active".as_ptr());
        CLPARSE_OK
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:1671

unsafe fn cl_register_particles(d: &mut Detail) -> Raise {
    // SAFETY: `cl` is C-owned; `mod_known` is strided by the ABI-checked
    // `qmodel_t` size.
    unsafe {
        // make sure the precaches know the right effects
        for i in 0..MAX_PARTICLETYPES as usize {
            let p = cl_particle_precache(i as isize);
            if !(*p).name.is_null() {
                let mut out: c_int = 0;
                guard!(d, g::ClParse_Glue_FindParticleType((*p).name, &mut out));
                (*p).index = out;
            } else {
                (*p).index = -1;
            }
        }

        // and make sure models get the right effects+trails etc too
        let mut i: c_int = 0;
        while i < ptr::addr_of!(g::mod_numknown).read() {
            let mdl = ptr::addr_of_mut!(g::mod_known)
                .cast::<u8>()
                .add(i as usize * core::mem::size_of::<QModel>())
                .cast::<c_void>();
            guard!(d, g::ClParse_Glue_UpdateModelEffects(mdl));
            i += 1;
        }
        CLPARSE_OK
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:1696

unsafe fn cl_parse_particles(type_: c_int, d: &mut Detail) -> Raise {
    // SAFETY: `cl` is C-owned; `org`/`vel` are locals that outlive the guarded
    // calls that read them.
    unsafe {
        let mut org: [c_float; 3] = [0.0; 3];
        let mut vel: [c_float; 3] = [0.0; 3];
        if type_ < 0 {
            // trail
            let entity = MSG_ReadShort();
            let efnum = MSG_ReadShort();
            org[0] = MSG_ReadCoord(protocolflags());
            org[1] = MSG_ReadCoord(protocolflags());
            org[2] = MSG_ReadCoord(protocolflags());
            vel[0] = MSG_ReadCoord(protocolflags());
            vel[1] = MSG_ReadCoord(protocolflags());
            vel[2] = MSG_ReadCoord(protocolflags());

            let mut ent: *mut Entity = ptr::null_mut();
            raise!(cl_entity_num(entity, &mut ent, d));

            if efnum < MAX_PARTICLETYPES && !(*cl_particle_precache(efnum as isize)).name.is_null()
            {
                let p = cl_particle_precache(efnum as isize);
                guard!(
                    d,
                    g::ClParse_Glue_ParticleTrail(
                        org.as_ptr(),
                        vel.as_ptr(),
                        (*p).index,
                        1.0,
                        0,
                        ptr::addr_of_mut!((*ent).trailstate),
                    )
                );
            }
        } else {
            // point
            let efnum = MSG_ReadShort();
            let count: c_int;
            org[0] = MSG_ReadCoord(protocolflags());
            org[1] = MSG_ReadCoord(protocolflags());
            org[2] = MSG_ReadCoord(protocolflags());
            if type_ != 0 {
                vel[0] = 0.0;
                vel[1] = 0.0;
                vel[2] = 0.0;
                count = 1;
            } else {
                vel[0] = MSG_ReadCoord(protocolflags());
                vel[1] = MSG_ReadCoord(protocolflags());
                vel[2] = MSG_ReadCoord(protocolflags());
                count = MSG_ReadShort();
            }
            if efnum < MAX_PARTICLETYPES && !(*cl_particle_precache(efnum as isize)).name.is_null()
            {
                let p = cl_particle_precache(efnum as isize);
                guard!(
                    d,
                    g::ClParse_Glue_RunParticleEffectState(
                        org.as_ptr(),
                        vel.as_ptr(),
                        count as c_float,
                        (*p).index,
                        ptr::null_mut(),
                    )
                );
            }
        }
        CLPARSE_OK
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:1743

/// `cl_parse.c:1743` `SHOWNET`.
macro_rules! shownet {
    ($x:expr) => {{
        if cl_shownet_value() == 2.0 {
            c::Con_Printf(c"%3i:%s\n".as_ptr(), msg_readcount() - 1, $x);
        }
    }};
}

// ---------------------------------------------------------------------------
// cl_parse.c:1747

unsafe fn cl_parse_stat_numeric(stat: c_int, ival: c_int, fval: c_float) {
    // SAFETY: `cl`/`vid` are C-owned; `stat` is bounds-checked.
    unsafe {
        if !(0..MAX_CL_STATS).contains(&stat) {
            c::Con_DWarning(c"svc_updatestat: %i is invalid\n".as_ptr(), stat);
            return;
        }
        (*cl_p()).stats[stat as usize] = ival;
        (*cl_p()).statsf[stat as usize] = fval;
        if stat == STAT_VIEWZOOM {
            ptr::addr_of_mut!(g::vid.recalc_refdef).write(1);
        }
    }
}

unsafe fn cl_parse_stat_float(stat: c_int, fval: c_float) {
    // SAFETY: forwards to the bounds-checked setter.
    // COMPAT: ADR-010 -- C's `(int)` truncates toward zero and is undefined for
    // NaN or out-of-range values; Rust's `as` saturates and maps NaN to 0. Left
    // as `as` deliberately, matching sv_move.rs:618: ADR-010 is per-platform,
    // and on arm64 C lowers to `fcvtzs`, which saturates and maps NaN to 0
    // exactly as Rust does, so emulating x86-64's `cvttss2si` (NaN -> INT_MIN)
    // would break arm64 parity to fix x86-64's. Reachable only from
    // svcfte_updatestatfloat, i.e. only from a server that negotiated
    // PEXT_CSQC and then sent a NaN or |v| >= 2^31 stat.
    unsafe { cl_parse_stat_numeric(stat, fval as c_int, fval) }
}

unsafe fn cl_parse_stat_int(stat: c_int, ival: c_int) {
    // SAFETY: forwards to the bounds-checked setter.
    unsafe { cl_parse_stat_numeric(stat, ival, ival as c_float) }
}

unsafe fn cl_parse_stat_string(stat: c_int, str_: *const c_char) {
    // SAFETY: `cl` is C-owned; `stat` is bounds-checked; `str_` is the
    // NUL-terminated message string.
    unsafe {
        if !(0..MAX_CL_STATS).contains(&stat) {
            c::Con_DWarning(c"svc_updatestat: %i is invalid\n".as_ptr(), stat);
            return;
        }
        c::Mem_Free((*cl_p()).statss[stat as usize].cast::<c_void>());
        (*cl_p()).statss[stat as usize] = c::cvar_cmd::q_strdup(str_);
    }
}

// ---------------------------------------------------------------------------
// cl_parse.c:1784

unsafe fn cl_parse_server_message(d: &mut Detail) -> Raise {
    // SAFETY: `cl`/`cls`/`net_message` are C-owned with process lifetime.
    unsafe {
        let mut lastcmd: c_int;

        if cl_shownet_value() == 1.0 {
            c::Con_Printf(
                c"%i ".as_ptr(),
                ptr::addr_of!(c::net_message.cursize).read(),
            );
        } else if cl_shownet_value() == 2.0 {
            c::Con_Printf(c"------------------\n".as_ptr());
        }

        MSG_BeginReading();

        lastcmd = 0;
        loop {
            if msg_badread() {
                return CLPARSE_ERR_BADMESSAGE;
            }

            let cmd = MSG_ReadByte();

            if cmd == -1 {
                shownet!(c"END OF MESSAGE".as_ptr());

                if (*cl_p()).items != (*cl_p()).stats[STAT_ITEMS] {
                    for i in 0..32u32 {
                        if ((*cl_p()).stats[STAT_ITEMS] as u32 & (1u32 << i)) != 0
                            && ((*cl_p()).items as u32 & (1u32 << i)) == 0
                        {
                            (*cl_p()).item_gettime[i as usize] = (*cl_p()).time as c_float;
                        }
                    }
                    (*cl_p()).items = (*cl_p()).stats[STAT_ITEMS];
                }
                return CLPARSE_OK; // end of message
            }

            // if the high bit of the command byte is set, it is a fast update
            if cmd & U_SIGNAL != 0 {
                // for netquake demos, just parse the last 10 seconds
                if (*cls_p()).demoseeking && (*cls_p()).seektime as f64 > (*cl_p()).mtime[0] + 10.0
                {
                    return CLPARSE_OK;
                }
                shownet!(c"fast update".as_ptr());
                raise!(cl_parse_update(cmd & 127, d));
                continue;
            }

            if cmd < NUM_SVC_STRINGS {
                shownet!(svc_string(cmd as usize));
            }

            // other commands
            match cmd {
                SVC_NOP => {}

                SVC_TIME => {
                    (*cl_p()).mtime[1] = (*cl_p()).mtime[0];
                    (*cl_p()).mtime[0] = MSG_ReadFloat() as f64;
                    if (*cl_p()).protocol_pext2 & PEXT2_PREDINFO != 0 {
                        MSG_ReadShort(); // input sequence ack.
                    }
                }

                SVC_CLIENTDATA => {
                    cl_parse_clientdata();
                }

                SVC_VERSION => {
                    let i = MSG_ReadLong();
                    // johnfitz -- support multiple protocols
                    if i as c_uint != PROTOCOL_NETQUAKE
                        && i as c_uint != PROTOCOL_FITZQUAKE
                        && i as c_uint != PROTOCOL_RMQ
                    {
                        d.a = i;
                        return CLPARSE_ERR_VERSION;
                    }
                    (*cl_p()).protocol = i as c_uint;
                }

                SVC_DISCONNECT => {
                    return CLPARSE_END_DISCONNECTED;
                }

                SVC_PRINT => {
                    let str_ = MSG_ReadString();
                    if !(*cls_p()).demoseeking {
                        c::Con_Printf(c"%s".as_ptr(), str_);
                    }
                }

                SVC_CENTERPRINT => {
                    // johnfitz -- log centerprints to console
                    let str_ = MSG_ReadString();
                    g::SCR_CenterPrint(str_);
                    g::Con_LogCenterPrint(str_);
                }

                SVC_STUFFTEXT => {
                    let str_ = MSG_ReadString();
                    // handle special commands
                    if strlen(str_) > 2
                        && *str_.add(0) == b'/' as c_char
                        && *str_.add(1) == b'/' as c_char
                    {
                        let mut out: c_int = 0;
                        guard!(
                            d,
                            g::ClParse_Glue_CmdExecuteString(str_.add(2), SRC_SERVER, &mut out)
                        );
                        if out == 0 {
                            c::Con_DPrintf(
                                c"Server sent unknown command %s\n".as_ptr(),
                                c::Cmd_Argv(0),
                            );
                        }
                    } else {
                        guard!(d, g::ClParse_Glue_CbufAddText(str_));
                    }
                }

                SVC_DAMAGE => {
                    g::V_ParseDamage();
                }

                SVC_SERVERINFO => {
                    raise!(cl_parse_server_info(d));
                    ptr::addr_of_mut!(g::vid.recalc_refdef).write(1);
                }

                SVC_SETANGLE => {
                    for i in 0..3 {
                        (*cl_p()).viewangles[i] = MSG_ReadAngle(protocolflags());
                    }
                    (*cl_p()).fixangle_time = (*cl_p()).mtime[0];
                }
                SVCFTE_SETANGLEDELTA => {
                    for i in 0..3 {
                        (*cl_p()).viewangles[i] += MSG_ReadAngle16(protocolflags());
                    }
                }

                SVC_SETVIEW => {
                    (*cl_p()).viewentity = MSG_ReadShort();
                }

                SVC_LIGHTSTYLE => {
                    let i = MSG_ReadByte();
                    if i >= MAX_LIGHTSTYLES {
                        c::Sys_Error(c"svc_lightstyle > MAX_LIGHTSTYLES".as_ptr());
                    }
                    let ls = cl_lightstyle(i as isize);
                    g::q_strlcpy(
                        ptr::addr_of_mut!((*ls).map).cast::<c_char>(),
                        MSG_ReadString(),
                        MAX_STYLESTRING,
                    );
                    (*ls).length = strlen(ptr::addr_of!((*ls).map).cast::<c_char>()) as c_int;
                    // johnfitz -- save extra info
                    if (*ls).length != 0 {
                        let mut total: c_int = 0;
                        (*ls).peak = b'a' as c_char;
                        let mut j: c_int = 0;
                        while j < (*ls).length {
                            let mv = (*ls).map[j as usize] as c_int;
                            total += mv - b'a' as c_int;
                            let peak = (*ls).peak as c_int;
                            (*ls).peak = if peak > mv { peak } else { mv } as c_char;
                            j += 1;
                        }
                        (*ls).average = (total / (*ls).length + b'a' as c_int) as c_char;
                    } else {
                        (*ls).peak = b'm' as c_char;
                        (*ls).average = (*ls).peak;
                    }
                }

                SVC_SOUND => {
                    raise!(cl_parse_start_sound_packet(d));
                }

                SVC_STOPSOUND => {
                    let i = MSG_ReadShort();
                    g::S_StopSound(i >> 3, i & 7);
                }

                SVC_UPDATENAME => {
                    let i = MSG_ReadByte();
                    if i >= (*cl_p()).maxclients {
                        return CLPARSE_ERR_UPDATENAME;
                    }
                    let sc = cl_score(i as isize);
                    g::q_strlcpy(
                        ptr::addr_of_mut!((*sc).name).cast::<c_char>(),
                        MSG_ReadString(),
                        MAX_SCOREBOARDNAME,
                    );
                    c::cvar_cmd::Info_SetKey(
                        ptr::addr_of_mut!((*sc).userinfo).cast::<c_char>(),
                        CLIENT_USER_INFO_STRING_SIZE,
                        c"name".as_ptr(),
                        ptr::addr_of!((*sc).name).cast::<c_char>(),
                    );
                }

                SVC_UPDATEFRAGS => {
                    let i = MSG_ReadByte();
                    if i >= (*cl_p()).maxclients {
                        return CLPARSE_ERR_UPDATEFRAGS;
                    }
                    (*cl_score(i as isize)).frags = MSG_ReadShort();
                }

                SVC_UPDATECOLORS => {
                    let i = MSG_ReadByte();
                    if i >= (*cl_p()).maxclients {
                        return CLPARSE_ERR_UPDATECOLORS;
                    }
                    let sc = cl_score(i as isize);
                    (*sc).colors = MSG_ReadByte();
                    raise!(cl_new_translation(i, d));
                    c::cvar_cmd::Info_SetKey(
                        ptr::addr_of_mut!((*sc).userinfo).cast::<c_char>(),
                        CLIENT_USER_INFO_STRING_SIZE,
                        c"topcolor".as_ptr(),
                        g::va(c"%d".as_ptr(), (*sc).colors >> 4),
                    );
                    c::cvar_cmd::Info_SetKey(
                        ptr::addr_of_mut!((*sc).userinfo).cast::<c_char>(),
                        CLIENT_USER_INFO_STRING_SIZE,
                        c"bottomcolor".as_ptr(),
                        g::va(c"%d".as_ptr(), (*sc).colors & 0xf),
                    );
                }

                SVC_PARTICLE => {
                    g::R_ParseParticleEffect();
                }

                SVC_SPAWNBASELINE => {
                    let i = MSG_ReadShort();
                    // must use CL_EntityNum() to force cl.num_entities up
                    let mut ent: *mut Entity = ptr::null_mut();
                    raise!(cl_entity_num(i, &mut ent, d));
                    raise!(cl_parse_baseline(ent, 1, d));
                }

                SVC_SPAWNSTATIC => {
                    raise!(cl_parse_static(1, d));
                }

                SVC_TEMP_ENTITY => {
                    guard!(d, g::ClParse_Glue_ParseTEnt());
                }

                SVC_SETPAUSE => {
                    (*cl_p()).paused = MSG_ReadByte() != 0;
                    if (*cl_p()).paused {
                        g::CDAudio_Pause();
                        g::BGM_Pause();
                    } else {
                        g::CDAudio_Resume();
                        g::BGM_Resume();
                    }
                }

                SVC_SIGNONNUM => {
                    let i = MSG_ReadByte();
                    if i <= (*cls_p()).signon {
                        d.a = i;
                        d.b = (*cls_p()).signon;
                        return CLPARSE_ERR_SIGNON;
                    }
                    (*cls_p()).signon = i;
                    // johnfitz -- check for excessive static ents and efrags
                    if i == 2 {
                        if (*cl_p()).num_statics > 128 {
                            c::Con_DWarning(
                                c"%i static entities exceeds standard limit of 128.\n".as_ptr(),
                                (*cl_p()).num_statics,
                            );
                        }
                        guard!(d, g::ClParse_Glue_CheckEfrags());
                    }
                    guard!(d, g::ClParse_Glue_SignonReply());
                }

                SVC_KILLEDMONSTER => {
                    (*cl_p()).stats[STAT_MONSTERS] += 1;
                    (*cl_p()).statsf[STAT_MONSTERS] = (*cl_p()).stats[STAT_MONSTERS] as c_float;
                }

                SVC_FOUNDSECRET => {
                    (*cl_p()).stats[STAT_SECRETS] += 1;
                    (*cl_p()).statsf[STAT_SECRETS] = (*cl_p()).stats[STAT_SECRETS] as c_float;
                }

                SVC_UPDATESTAT => {
                    let i = MSG_ReadByte();
                    cl_parse_stat_int(i, MSG_ReadLong());
                }

                SVC_SPAWNSTATICSOUND => {
                    cl_parse_static_sound(1);
                }

                SVC_CDTRACK => {
                    (*cl_p()).cdtrack = MSG_ReadByte();
                    (*cl_p()).looptrack = MSG_ReadByte();
                    if ((*cls_p()).demoplayback || (*cls_p()).demorecording)
                        && (*cls_p()).forcetrack != -1
                    {
                        g::BGM_PlayCDtrack((*cls_p()).forcetrack as u8, true);
                    } else {
                        g::BGM_PlayCDtrack((*cl_p()).cdtrack as u8, true);
                    }
                }

                SVC_INTERMISSION => {
                    (*cl_p()).intermission = 1;
                    (*cl_p()).completed_time = (*cl_p()).time as c_int;
                    ptr::addr_of_mut!(g::vid.recalc_refdef).write(1);
                    g::V_RestoreAngles();
                }

                SVC_FINALE => {
                    (*cl_p()).intermission = 2;
                    (*cl_p()).completed_time = (*cl_p()).time as c_int;
                    ptr::addr_of_mut!(g::vid.recalc_refdef).write(1);
                    let str_ = MSG_ReadString();
                    g::SCR_CenterPrint(str_);
                    g::Con_LogCenterPrint(str_);
                    g::V_RestoreAngles();
                }

                SVC_CUTSCENE => {
                    (*cl_p()).intermission = 3;
                    (*cl_p()).completed_time = (*cl_p()).time as c_int;
                    ptr::addr_of_mut!(g::vid.recalc_refdef).write(1);
                    let str_ = MSG_ReadString();
                    g::SCR_CenterPrint(str_);
                    g::Con_LogCenterPrint(str_);
                    g::V_RestoreAngles();
                }

                SVC_SELLSCREEN => {
                    let mut out: c_int = 0;
                    guard!(
                        d,
                        g::ClParse_Glue_CmdExecuteString(c"help".as_ptr(), SRC_COMMAND, &mut out)
                    );
                }

                // johnfitz -- new svc types
                SVC_SKYBOX => {
                    guard!(d, g::ClParse_Glue_LoadSkyBox(MSG_ReadString()));
                }

                SVC_BF => {
                    let mut out: c_int = 0;
                    guard!(
                        d,
                        g::ClParse_Glue_CmdExecuteString(c"bf".as_ptr(), SRC_COMMAND, &mut out)
                    );
                }

                SVC_FOG => {
                    g::Fog_ParseServerMessage();
                }

                SVC_SPAWNBASELINE2 => {
                    // PROTOCOL_FITZQUAKE
                    let i = MSG_ReadShort();
                    let mut ent: *mut Entity = ptr::null_mut();
                    raise!(cl_entity_num(i, &mut ent, d));
                    raise!(cl_parse_baseline(ent, 2, d));
                }

                SVC_SPAWNSTATIC2 => {
                    raise!(cl_parse_static(2, d));
                }

                SVC_SPAWNSTATICSOUND2 => {
                    cl_parse_static_sound(2);
                }

                // used by the 2021 rerelease
                SVC_ACHIEVEMENT => {
                    let str_ = MSG_ReadString();
                    if !g::Steam_SetAchievement(str_) {
                        c::Con_DPrintf(c"Couldn't set achievement \"%s\"\n".as_ptr(), str_);
                    }
                }
                SVC_LOCALSOUND => {
                    raise!(cl_parse_local_sound(d));
                }

                SVCDP_TRAILPARTICLES => {
                    if !(*cl_p()).protocol_particles {
                        raise!(cl_force_protocol_particles(d));
                    }
                    raise!(cl_parse_particles(-1, d));
                }
                SVCDP_POINTPARTICLES => {
                    if !(*cl_p()).protocol_particles {
                        raise!(cl_force_protocol_particles(d));
                    }
                    raise!(cl_parse_particles(0, d));
                }
                SVCDP_POINTPARTICLES1 => {
                    if !(*cl_p()).protocol_particles {
                        raise!(cl_force_protocol_particles(d));
                    }
                    raise!(cl_parse_particles(1, d));
                }

                SVCDP_PRECACHE => {
                    if (*cl_p()).protocol_pext2 == 0 {
                        return CLPARSE_ERR_DPPRECACHE;
                    }
                    raise!(cl_parse_precache(d));
                }
                SVCDP_UPDATESTATBYTE => {
                    if (*cl_p()).protocol_pext2 & PEXT2_REPLACEMENTDELTAS == 0 {
                        return CLPARSE_ERR_UPDATESTATBYTE;
                    }
                    let i = MSG_ReadByte();
                    cl_parse_stat_int(i, MSG_ReadByte());
                }
                SVCFTE_UPDATESTATSTRING => {
                    if (*cl_p()).protocol_pext2 & PEXT2_REPLACEMENTDELTAS == 0 {
                        return CLPARSE_ERR_UPDATESTATSTRING;
                    }
                    let i = MSG_ReadByte();
                    cl_parse_stat_string(i, MSG_ReadString());
                }
                SVCFTE_UPDATESTATFLOAT => {
                    if (*cl_p()).protocol_pext2 & PEXT2_REPLACEMENTDELTAS == 0 {
                        return CLPARSE_ERR_UPDATESTATFLOAT;
                    }
                    let i = MSG_ReadByte();
                    cl_parse_stat_float(i, MSG_ReadFloat());
                }
                SVCFTE_SPAWNSTATIC2 => {
                    if (*cl_p()).protocol_pext2 & PEXT2_REPLACEMENTDELTAS == 0 {
                        return CLPARSE_ERR_SPAWNSTATIC2;
                    }
                    raise!(cl_parse_static(6, d));
                }
                SVCFTE_SPAWNBASELINE2 => {
                    if (*cl_p()).protocol_pext2 & PEXT2_REPLACEMENTDELTAS == 0 {
                        return CLPARSE_ERR_SPAWNBASELINE2;
                    }
                    let i = MSG_ReadEntity((*cl_p()).protocol_pext2) as c_int;
                    // must use CL_EntityNum() to force cl.num_entities up
                    let mut ent: *mut Entity = ptr::null_mut();
                    raise!(cl_entity_num(i, &mut ent, d));
                    raise!(cl_parse_baseline(ent, 6, d));
                }
                SVCFTE_UPDATEENTITIES => {
                    if (*cl_p()).protocol_pext2 & PEXT2_REPLACEMENTDELTAS == 0 {
                        return CLPARSE_ERR_UPDATEENTITIES;
                    }
                    raise!(clfte_parse_entities_update(d));
                }

                SVCFTE_CGAMEPACKET => {
                    if (*cl_p()).protocol_pext1 & PEXT1_CSQC == 0 {
                        return CLPARSE_ERR_CGAMEPACKET;
                    }
                    if (*cl_p()).qcvm.extfuncs.csqc_parse_event != 0 {
                        guard!(d, g::ClParse_Glue_CsqcParseEvent());
                    } else {
                        return CLPARSE_ERR_CSQC_MISSING;
                    }
                }

                SVCFTE_VOICECHAT => {
                    if (*cl_p()).protocol_pext2 & PEXT2_VOICECHAT == 0 {
                        return CLPARSE_ERR_VOICECHAT;
                    }
                    MSG_ReadByte(); // sender
                    MSG_ReadByte(); // gen
                    MSG_ReadByte(); // seq
                    let mut bytes = MSG_ReadShort();
                    loop {
                        let old = bytes;
                        bytes -= 1;
                        if old <= 0 {
                            break;
                        }
                        MSG_ReadByte();
                    }
                }

                _ => {
                    d.a = cmd;
                    d.s = svc_string(lastcmd as usize);
                    return CLPARSE_ERR_ILLEGIBLE;
                }
            }

            lastcmd = cmd; // johnfitz
        }
    }
}

// ---------------------------------------------------------------------------
// Exported status cores. Each is called from exactly one re-raising forwarder
// in Quake/cl_parse_glue.c (ADR-009).

/// `cl_parse.c:105` `CL_EntityNum`.
///
/// # Safety
/// `out` must be a writable `entity_t *` slot and `cl` must be initialised.
/// Called only from `Quake/cl_parse_glue.c`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cl_entity_num(num: c_int, out: *mut *mut c_void) -> Raise {
    // SAFETY: `out` is the glue's stack slot; `cl` is C-owned.
    unsafe {
        let mut d = Detail::new();
        let mut ent: *mut Entity = ptr::null_mut();
        let r = cl_entity_num(num, &mut ent, &mut d);
        if r == CLPARSE_OK {
            *out = ent.cast::<c_void>();
        }
        r
    }
}

/// `cl_parse.c:840` `CL_ParseLocalSound`.
///
/// # Safety
/// `detail` must be a writable `int` slot; reads `net_message` through the
/// plain `MSG_Read*` shims. Called only from `Quake/cl_parse_glue.c`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cl_parse_local_sound(detail: *mut c_int) -> Raise {
    // SAFETY: `detail` is the glue's stack slot.
    unsafe {
        let mut d = Detail::new();
        let r = cl_parse_local_sound(&mut d);
        if r != CLPARSE_OK {
            *detail = d.a;
        }
        r
    }
}

/// `cl_parse.c:1527` `CL_NewTranslation`.
///
/// # Safety
/// `detail` must be a writable `int` slot. Called only from
/// `Quake/cl_parse_glue.c`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cl_new_translation(slot: c_int, detail: *mut c_int) -> Raise {
    // SAFETY: `detail` is the glue's stack slot.
    unsafe {
        let mut d = Detail::new();
        let r = cl_new_translation(slot, &mut d);
        if r != CLPARSE_OK {
            *detail = d.a;
        }
        r
    }
}

/// `cl_parse.c:1671` `CL_RegisterParticles`.
///
/// # Safety
/// `detail` must be a writable `int` slot. Called only from
/// `Quake/cl_parse_glue.c`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cl_register_particles(detail: *mut c_int) -> Raise {
    // SAFETY: `detail` is the glue's stack slot.
    unsafe {
        let mut d = Detail::new();
        let r = cl_register_particles(&mut d);
        if r != CLPARSE_OK {
            *detail = d.a;
        }
        r
    }
}

/// `cl_parse.c:1784` `CL_ParseServerMessage`.
///
/// # Safety
/// `a`, `b` and `s` must be writable slots that outlive the `ClParse_Raise`
/// call the glue makes with them; reads `net_message` through the plain
/// `MSG_Read*` shims. Called only from `Quake/cl_parse_glue.c`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cl_parse_server_message(
    a: *mut c_int,
    b: *mut c_int,
    s: *mut *const c_char,
) -> Raise {
    // SAFETY: `a`/`b`/`s` are the glue's stack slots. `d.s`, when set, points
    // into `MODEL_PRECACHE` or `svc_strings`, both of which have process
    // lifetime, so the glue may dereference it after this returns.
    unsafe {
        let mut d = Detail::new();
        let r = cl_parse_server_message(&mut d);
        if r != CLPARSE_OK {
            *a = d.a;
            *b = d.b;
            *s = d.s;
        }
        r
    }
}
