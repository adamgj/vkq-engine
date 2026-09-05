//! Host/server/client ABI mirrors -- `Quake/server.h`, `Quake/client.h`.
//!
//! Compat-critical (Rust migration Phase 7, ADR-011): `sv`/`svs` and
//! `cl`/`cls` are the process-global server and client state, read and
//! written directly from both languages, so layout drift here is silent
//! memory corruption, not a link error. `sv`/`svs` are now Rust-owned
//! (`quake-capi`'s `sv_main.rs`), which closed their ADR-007 dual view at
//! M6; `cl`/`cls` are still C-owned and close at M7, per
//! `docs/ai/plans/rust-conversion-phase-7.md`.
//!
//! Neither `server.h` nor `client.h` is a bindgen-clean root: both pull
//! `qcvm_t` (`progs.h`) and, via `client_state_t::viewent`, `entity_t`
//! (`render.h`), which itself drags in the SDL-tainted `tasks.h` and the
//! Vulkan-typed `q_render_types.h`. Hence hand-written mirrors, verified
//! per-platform by `quake-ctest/tests/host_abi.rs` against the engine's own
//! headers. `entity_t` (and only `entity_t`) is mirrored as an opaque,
//! size/align-verified blob ([`EntityOpaque`]) rather than field-by-field --
//! it is renderer state that belongs to a later phase, and every other
//! field in this module is a real named field with a probed offset.
//!
//! Unlike `progs.h`'s `edict_t`, neither `server.h` nor `client.h` has any
//! `DEBUG`/`_DEBUG`-conditional field, so nothing in this module forks on
//! the `engine-debug` cargo feature (the embedded [`progs::QcVm`] does not
//! embed `progs::Edict` by value, so it does not fork either -- only
//! `progs::Edict` itself, reached solely through pointers here, does).

use core::ffi::{c_char, c_int, c_uint, c_void};

use crate::model_mem::QModel;
use crate::net::{QSocket, SizeBuf, MAX_DATAGRAM, MAX_MSGLEN};
use crate::progs::{Edict, EntityState, QcVm};
use crate::sound::{Sfx, MAX_SOUNDS};

/// `q_types.h`: `qboolean` is C11 `_Bool`.
pub type QBoolean = bool;

/// `quakedef.h`
pub const MAX_MODELS: usize = 8192;
/// `quakedef.h`
pub const MAX_PARTICLETYPES: usize = 2048;
/// `quakedef.h`
pub const MAX_STYLESTRING: usize = 64;
/// `quakedef.h`: enum member, mirrored as a plain constant.
pub const MAX_CL_STATS: usize = 256;
/// `quakedef.h`
pub const MAX_SCOREBOARDNAME: usize = 32;
/// `quakedef.h`
pub const MAX_LIGHTSTYLES: usize = 64;
/// `vid.h`
pub const VID_CBITS: usize = 6;
/// `vid.h`: `1 << VID_CBITS`
pub const VID_GRADES: usize = 1 << VID_CBITS;
/// `server.h`
pub const SERVER_INFO_STRING_SIZE: usize = 8192;
/// `client.h`
pub const CLIENT_USER_INFO_STRING_SIZE: usize = 8192;
/// `server.h`
pub const NUM_PING_TIMES: usize = 16;
/// `server.h`
pub const NUM_TOTAL_SPAWN_PARMS: usize = 64;
/// `client.h`
pub const NUM_CSHIFTS: usize = 4;
/// `client.h`
pub const MAX_MAPSTRING: usize = 2048;
/// `client.h`
pub const MAX_DEMOS: usize = 8;
/// `client.h`
pub const MAX_DEMONAME: usize = 16;

/// `server.h` `server_state_t`, mirrored as a plain `c_int` field on
/// [`Server::state`] (matching `progs::Edict`'s convention for C enums) so
/// an unrecognized value read across FFI is not UB.
pub const SS_LOADING: c_int = 0;
pub const SS_ACTIVE: c_int = 1;

/// `client.h`'s anonymous `enum {PRESPAWN_DONE, PRESPAWN_FLUSH=1, ...}`,
/// mirrored as a plain `c_int` field on [`Client::sendsignon`].
pub const PRESPAWN_DONE: c_int = 0;
pub const PRESPAWN_FLUSH: c_int = 1;
pub const PRESPAWN_MODELS: c_int = 2;
pub const PRESPAWN_SOUNDS: c_int = 3;
pub const PRESPAWN_PARTICLES: c_int = 4;
pub const PRESPAWN_BASELINES: c_int = 5;
pub const PRESPAWN_STATICS: c_int = 6;
pub const PRESPAWN_AMBIENTS: c_int = 7;
pub const PRESPAWN_SIGNONMSG: c_int = 8;

/// `client.h` `cactive_t`, mirrored as a plain `c_int` field on
/// [`ClientStatic::state`].
pub const CA_DEDICATED: c_int = 0;
pub const CA_DISCONNECTED: c_int = 1;
pub const CA_CONNECTED: c_int = 2;

/// `server.h` `server_static_t` -- the global `svs`. Set once by `SV_Init`
/// and persists across level changes within a process.
#[repr(C)]
pub struct ServerStatic {
    pub maxclients: c_int,
    pub maxclientslimit: c_int,
    /// `struct client_s *`
    pub clients: *mut Client,
    pub serverflags: c_int,
    pub changelevel_issued: QBoolean,
    pub serverinfo: [c_char; SERVER_INFO_STRING_SIZE],
}

/// `server.h`'s anonymous `struct ambientsound_s` (`sv.ambientsounds`
/// element).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AmbientSound {
    pub origin: [f32; 3],
    pub soundindex: c_uint,
    pub volume: f32,
    pub attenuation: f32,
}

/// `server.h`'s anonymous `struct svcustomstat_s` (`sv.customstats`
/// element). `ptr` is `eval_t *` -- `progs.h`'s field-value union, opaque
/// here since Phase 7 does not need its layout to mirror `server_t`.
#[repr(C)]
pub struct SvCustomStat {
    pub idx: c_int,
    /// C field name is `type`, a reserved word in Rust.
    pub r#type: c_int,
    pub fld: c_int,
    pub ptr: *mut c_void,
}

/// `server.h` `server_t` -- the global `sv`.
#[repr(C)]
pub struct Server {
    pub active: QBoolean,
    pub paused: QBoolean,
    pub loadgame: QBoolean,
    pub nomonsters: QBoolean,
    pub lastsave: [c_char; 128],
    pub lastcheck: c_int,
    pub lastchecktime: f64,
    /// Same storage `PR_SwitchQCVM` selects when the server is the active
    /// VM (ADR-008).
    pub qcvm: QcVm,
    pub name: [c_char; 64],
    pub modelname: [c_char; 64],
    pub model_precache: [*const c_char; MAX_MODELS],
    pub models: [*mut QModel; MAX_MODELS],
    pub sound_precache: [*const c_char; MAX_SOUNDS],
    pub lightstyles: [*const c_char; MAX_LIGHTSTYLES],
    /// `server_state_t`; see [`SS_LOADING`]/[`SS_ACTIVE`].
    pub state: c_int,
    pub datagram: SizeBuf,
    pub datagram_buf: [u8; MAX_DATAGRAM],
    pub reliable_datagram: SizeBuf,
    pub reliable_datagram_buf: [u8; MAX_DATAGRAM],
    pub signon: SizeBuf,
    pub signon_buf: [u8; MAX_MSGLEN - 2],
    pub protocol: c_uint,
    pub protocolflags: c_uint,
    pub multicast: SizeBuf,
    pub multicast_buf: [u8; MAX_DATAGRAM],
    pub particle_precache: [*const c_char; MAX_PARTICLETYPES],
    pub static_entities: *mut EntityState,
    pub num_statics: c_int,
    pub max_statics: c_int,
    pub ambientsounds: *mut AmbientSound,
    pub num_ambients: c_int,
    pub max_ambients: c_int,
    pub customstats: [SvCustomStat; MAX_CL_STATS * 2],
    pub numcustomstats: usize,
    pub effectsmask: c_int,
}

/// `protocol.h` `usercmd_t`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct UserCmd {
    pub servertime: f32,
    pub seconds: f32,
    pub viewangles: [f32; 3],
    pub forwardmove: f32,
    pub sidemove: f32,
    pub upmove: f32,
    pub forwardmove_accumulator: f32,
    pub sidemove_accumulator: f32,
    pub upmove_accumulator: f32,
    pub buttons: c_uint,
    pub impulse: c_uint,
    pub sequence: c_uint,
    pub weapon: c_int,
}

/// `server.h`'s anonymous `struct entity_num_state_s`
/// (`client_t.previousentities` element).
#[repr(C)]
pub struct EntityNumState {
    pub num: c_uint,
    pub state: EntityState,
}

/// `server.h`'s anonymous inner struct of `struct deltaframe_s.ents`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DeltaFrameEnt {
    pub num: c_uint,
    pub ebits: c_uint,
    pub csqcbits: c_uint,
}

/// `server.h` `struct deltaframe_s` (`client_t.frames` element).
#[repr(C)]
pub struct DeltaFrame {
    pub sequence: c_int,
    pub timestamp: f32,
    pub resendstatsnum: [c_uint; MAX_CL_STATS / 32],
    pub resendstatsstr: [c_uint; MAX_CL_STATS / 32],
    pub ents: *mut DeltaFrameEnt,
    pub numents: c_int,
    pub maxents: c_int,
}

/// `server.h` `client_t` (`struct client_s`) -- one per `svs.clients` slot.
#[repr(C)]
pub struct Client {
    pub active: QBoolean,
    pub spawned: QBoolean,
    pub dropasap: QBoolean,
    /// See [`PRESPAWN_DONE`] etc.
    pub sendsignon: c_int,
    pub signonidx: c_int,
    pub signon_sounds: c_uint,
    pub signon_models: c_uint,
    pub last_message: f64,
    pub netconnection: *mut QSocket,
    pub cmd: UserCmd,
    pub wishdir: [f32; 3],
    pub message: SizeBuf,
    pub msgbuf: [u8; MAX_MSGLEN],
    pub edict: *mut Edict,
    pub name: [c_char; 32],
    pub colors: c_int,
    pub ping_times: [f32; NUM_PING_TIMES],
    pub num_pings: c_int,
    pub spawn_parms: [f32; NUM_TOTAL_SPAWN_PARMS],
    pub old_frags: c_int,
    pub datagram: SizeBuf,
    pub datagram_buf: [u8; MAX_DATAGRAM],
    pub limit_entities: c_uint,
    pub limit_unreliable: c_uint,
    pub limit_reliable: c_uint,
    pub limit_models: c_uint,
    pub limit_sounds: c_uint,
    pub pextknown: QBoolean,
    pub protocol_pext1: c_uint,
    pub protocol_pext2: c_uint,
    pub resendstatsnum: [c_uint; MAX_CL_STATS / 32],
    pub resendstatsstr: [c_uint; MAX_CL_STATS / 32],
    pub oldstats_i: [c_int; MAX_CL_STATS],
    pub oldstats_f: [f32; MAX_CL_STATS],
    pub oldstats_s: [*mut c_char; MAX_CL_STATS],
    pub previousentities: *mut EntityNumState,
    pub numpreviousentities: usize,
    pub maxpreviousentities: usize,
    pub snapshotresume: c_uint,
    pub pendingentities_bits: *mut c_uint,
    pub numpendingentities: usize,
    pub frames: *mut DeltaFrame,
    pub numframes: usize,
    pub lastacksequence: c_int,
    pub lastmovemessage: c_int,
    pub lastmovetime: f64,
    pub knowntoqc: QBoolean,
    pub userinfo: [c_char; SERVER_INFO_STRING_SIZE],
}

/// `client.h` `cshift_t`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CShift {
    pub destcolor: [c_int; 3],
    pub percent: f32,
}

/// `client.h` `scoreboard_t`.
#[repr(C)]
pub struct ScoreBoard {
    pub name: [c_char; MAX_SCOREBOARDNAME],
    pub entertime: f32,
    pub frags: c_int,
    pub colors: c_int,
    pub ping: c_int,
    pub translations: [u8; VID_GRADES * 256],
    pub userinfo: [c_char; CLIENT_USER_INFO_STRING_SIZE],
}

/// `client.h`'s anonymous particle-precache entry (`PSET_SCRIPT`, which
/// `quakedef.h` defines unconditionally -- same treatment as
/// `model_mem`'s ray-tracing fields).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ParticlePrecacheEntry {
    pub name: *const c_char,
    pub index: c_int,
}

/// `client.h` `client_static_t` -- the global `cls`. Survives disconnects;
/// reset only by `CL_Disconnect`/process restart.
#[repr(C)]
pub struct ClientStatic {
    /// `cactive_t`; see [`CA_DEDICATED`] etc.
    pub state: c_int,
    pub spawnparms: [c_char; MAX_MAPSTRING],
    pub demonum: c_int,
    pub demos: [[c_char; MAX_DEMONAME]; MAX_DEMOS],
    pub demorecording: QBoolean,
    pub demoplayback: QBoolean,
    pub demopaused: QBoolean,
    pub demoseeking: QBoolean,
    pub seektime: f32,
    pub demospeed: f32,
    /// `sys.h` `qfileofs_t` is `long long`.
    pub demo_prespawn_end: i64,
    pub timedemo: QBoolean,
    pub forcetrack: c_int,
    /// `FILE *`, opaque.
    pub demofile: *mut c_void,
    pub td_lastframe: c_int,
    pub td_startframe: c_int,
    pub td_starttime: f32,
    pub signon: c_int,
    pub netcon: *mut QSocket,
    pub message: SizeBuf,
    pub userinfo: [c_char; CLIENT_USER_INFO_STRING_SIZE],
}

/// `render.h` `efrag_t`. `entity` points at the opaque [`EntityOpaque`]
/// blob, not a real `entity_t` -- see the module doc.
#[repr(C)]
pub struct Efrag {
    pub leafnext: *mut Efrag,
    pub entity: *mut EntityOpaque,
}

/// Opaque, size/align-verified stand-in for `render.h` `entity_t`.
///
/// `entity_t` embeds Vulkan- and SDL-tainted state (via `render.h`'s
/// `#include "tasks.h"`, which pulls `q_stdinc.h` and thus `SDL.h`) that is
/// renderer territory, not Phase 7's. Its size and alignment are pinned by
/// `quake-ctest/tests/host_abi.rs` against a local, field-verbatim shadow
/// reproduction of `entity_t` compiled from the real `render.h` field list
/// (using the same Vulkan handle stand-ins the Phase 3 mirrors use) -- not
/// guessed. `#[repr(C, align(8))]` matches `entity_t`'s natural alignment
/// (`double` members), which is required for [`ClientState`]'s fields after
/// `viewent` to land at the right offsets.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
pub struct EntityOpaque(pub [u8; ENTITY_T_OPAQUE_SIZE]);

/// `sizeof(entity_t)` on a 64-bit target, per `quake-ctest/tests/host_abi.rs`.
pub const ENTITY_T_OPAQUE_SIZE: usize = 456;

/// `client.h` `client_state_t` -- the global `cl`.
#[repr(C)]
pub struct ClientState {
    pub movemessages: c_int,
    pub ackedmovemessages: c_int,
    pub movecmds: [UserCmd; 64],
    pub pendingcmd: UserCmd,
    pub stats: [c_int; MAX_CL_STATS],
    pub statsf: [f32; MAX_CL_STATS],
    pub statss: [*mut c_char; MAX_CL_STATS],
    pub items: c_int,
    pub item_gettime: [f32; 32],
    pub faceanimtime: f32,
    pub v_dmg_time: f32,
    pub v_dmg_roll: f32,
    pub v_dmg_pitch: f32,
    pub cshift_empty: CShift,
    pub cshifts: [CShift; NUM_CSHIFTS],
    pub prev_cshifts: [CShift; NUM_CSHIFTS],
    pub mviewangles: [[f32; 3]; 2],
    pub viewangles: [f32; 3],
    pub mvelocity: [[f32; 3]; 2],
    pub velocity: [f32; 3],
    pub punchangle: [f32; 3],
    pub idealpitch: f32,
    pub pitchvel: f32,
    pub nodrift: QBoolean,
    pub driftmove: f32,
    pub laststop: f64,
    pub viewheight: f32,
    pub crouch: f32,
    pub paused: QBoolean,
    pub onground: QBoolean,
    pub inwater: QBoolean,
    pub fixangle_time: f64,
    pub intermission: c_int,
    pub completed_time: c_int,
    pub mtime: [f64; 2],
    pub time: f64,
    pub oldtime: f64,
    pub last_received_message: f32,
    pub model_precache: [*mut QModel; MAX_MODELS],
    pub sound_precache: [*mut Sfx; MAX_SOUNDS],
    pub mapname: [c_char; 128],
    pub levelname: [c_char; 128],
    pub viewentity: c_int,
    pub maxclients: c_int,
    pub gametype: c_int,
    pub worldmodel: *mut QModel,
    pub free_efrags: *mut Efrag,
    pub num_efrags: c_int,
    pub efrag_allocs: *mut *mut Efrag,
    pub num_efragallocs: c_int,
    /// See [`EntityOpaque`].
    pub viewent: EntityOpaque,
    pub entities: *mut EntityOpaque,
    pub max_edicts: c_int,
    pub num_entities: c_int,
    pub static_entities: *mut *mut EntityOpaque,
    pub max_static_entities: c_int,
    pub num_statics: c_int,
    pub cdtrack: c_int,
    pub looptrack: c_int,
    pub scores: *mut ScoreBoard,
    pub protocol: c_uint,
    pub protocolflags: c_uint,
    pub protocol_pext1: c_uint,
    pub protocol_pext2: c_uint,
    /// `PSET_SCRIPT`, which `quakedef.h` defines unconditionally.
    pub protocol_particles: QBoolean,
    pub particle_precache: [ParticlePrecacheEntry; MAX_PARTICLETYPES],
    pub local_particle_precache: [ParticlePrecacheEntry; MAX_PARTICLETYPES],
    pub ackframes: [c_int; 8],
    pub ackframes_count: c_uint,
    pub requestresend: QBoolean,
    pub sendprespawn: QBoolean,
    /// Same storage `PR_SwitchQCVM` selects when the client is the active
    /// VM (ADR-008).
    pub qcvm: QcVm,
    pub zoom: f32,
    pub zoomdir: f32,
    pub serverinfo: [c_char; SERVER_INFO_STRING_SIZE],
}

const _: () = {
    assert!(core::mem::size_of::<UserCmd>() == 60);
    assert!(core::mem::size_of::<CShift>() == 16);
    assert!(core::mem::size_of::<AmbientSound>() == 24);
    assert!(core::mem::size_of::<SvCustomStat>() == 24);
    assert!(core::mem::size_of::<EntityNumState>() == 4 + core::mem::size_of::<EntityState>());
    assert!(core::mem::size_of::<DeltaFrameEnt>() == 12);
    assert!(core::mem::size_of::<ParticlePrecacheEntry>() == 16);
};

/// C: `filelist_item_t` (`Quake/quakedef.h:412-416`) -- the intrusive
/// singly-linked node `host_cmd.c` allocates for every map, mod, demo and
/// savegame name. `FileList_AddEx` over-allocates the node so that a
/// per-list payload (`levelinfo_t`, `modinfo_t`) sits immediately after it;
/// those payload types are file-local to `host_cmd.c` and are mirrored
/// there, not here.
///
/// Compat-critical (ADR-011): `console.c` and `menu.c` walk these lists
/// under the plain C spelling, so the layout is shared, not private.
#[repr(C)]
pub struct FileListItem {
    pub name: [c_char; 32],
    pub next: *mut FileListItem,
}

/// `Quake/quakedef.h:423-452` -- `maptype_t`, in declaration order. The
/// numeric values are load-bearing: `ExtraMaps_Sort` orders by them and
/// `ExtraMaps_IsStart` tests four of them by name.
pub const MAPTYPE_CUSTOM_MOD_START: c_uint = 0;
pub const MAPTYPE_CUSTOM_MOD_LEVEL: c_uint = 1;
pub const MAPTYPE_CUSTOM_MOD_END: c_uint = 2;
pub const MAPTYPE_CUSTOM_MOD_DM: c_uint = 3;
pub const MAPTYPE_MOD_START: c_uint = 4;
pub const MAPTYPE_MOD_LEVEL: c_uint = 5;
pub const MAPTYPE_MOD_END: c_uint = 6;
pub const MAPTYPE_MOD_DM: c_uint = 7;
pub const MAPTYPE_CUSTOM_ID_START: c_uint = 8;
pub const MAPTYPE_CUSTOM_ID_LEVEL: c_uint = 9;
pub const MAPTYPE_CUSTOM_ID_END: c_uint = 10;
pub const MAPTYPE_CUSTOM_ID_DM: c_uint = 11;
pub const MAPTYPE_ID_START: c_uint = 12;
pub const MAPTYPE_ID_EP1_LEVEL: c_uint = 13;
pub const MAPTYPE_ID_EP2_LEVEL: c_uint = 14;
pub const MAPTYPE_ID_EP3_LEVEL: c_uint = 15;
pub const MAPTYPE_ID_EP4_LEVEL: c_uint = 16;
pub const MAPTYPE_ID_END: c_uint = 17;
pub const MAPTYPE_ID_DM: c_uint = 18;
pub const MAPTYPE_ID_LEVEL: c_uint = 19;
pub const MAPTYPE_BMODEL: c_uint = 20;
pub const MAPTYPE_COUNT: c_uint = 21;

const _: () = {
    assert!(core::mem::size_of::<FileListItem>() == 32 + core::mem::size_of::<*mut u8>());
    assert!(core::mem::offset_of!(FileListItem, next) == 32);
};
