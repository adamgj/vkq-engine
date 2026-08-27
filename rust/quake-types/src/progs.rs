//! Progs (QuakeC) VM ABI mirrors — `Quake/pr_comp.h`, `Quake/progdefs.q1`,
//! `Quake/progs.h`.
//!
//! Compat-critical (Rust migration Phase 6, ADR-011): `qcvm_t` is C-owned
//! storage embedded in `server_t sv` (`server.h`) and `client_state_t cl`
//! (`client.h`), and under `-Duse_rust_progs` the Rust VM reads and writes
//! that same instance, walks the C-allocated edict array, and dispatches C
//! builtins out of `qcvm->builtins[]`. Layout drift is therefore silent memory
//! corruption, not a link error.
//!
//! `progs.h` is not a bindgen-clean root (it pulls `progdefs.h`, `common.h`
//! and `protocol.h`), so these are hand-written mirrors, verified per-platform
//! and per-build-profile by `quake-ctest/tests/progs_abi.rs` against the
//! engine's own headers.
//!
//! `edict_t`'s layout differs between build profiles: `DEBUG`/`_DEBUG` builds
//! prepend three bookkeeping fields (`progs.h`). The `engine-debug` cargo
//! feature tracks the C `-D_DEBUG`, exactly as it does for the other mirrors.

use core::ffi::{c_char, c_int, c_uint, c_void};

/// `pr_comp.h`: QC function reference (index into `qcvm->functions`).
pub type FuncT = u32;
/// `pr_comp.h`: QC string reference. Non-negative values index the progs
/// string table; negative values index `knownstrings` as `-1 - num`
/// (`pr_edict.c` `PR_GetString`).
pub type StringT = i32;
/// `progs.h`: `typedef void (*builtin_t) (void)`
pub type BuiltinT = Option<unsafe extern "C" fn()>;
/// `q_types.h`: `qboolean` is C11 `_Bool`
pub type QBoolean = bool;

/// Whether this crate was compiled with the `engine-debug` feature, i.e.
/// whether the [`Edict`] mirror carries the `DEBUG`/`_DEBUG` header prefix.
/// `quake-ctest/tests/progs_abi.rs` asserts this agrees with how the C was
/// compiled — if it does not, every edict offset is three fields out.
pub const ENGINE_DEBUG: bool = cfg!(feature = "engine-debug");

pub const PROG_VERSION: c_int = 6;
/// `progdefs.q1`
pub const PROGHEADER_CRC: c_int = 5927;
pub const MAX_PARMS: usize = 8;
/// `pr_comp.h`: set in `ddef_t::type` when the global belongs in savegames
pub const DEF_SAVEGLOBAL: u16 = 1 << 15;

pub const OFS_NULL: usize = 0;
pub const OFS_RETURN: usize = 1;
pub const OFS_PARM0: usize = 4;
pub const OFS_PARM1: usize = 7;
pub const OFS_PARM2: usize = 10;
pub const OFS_PARM3: usize = 13;
pub const OFS_PARM4: usize = 16;
pub const OFS_PARM5: usize = 19;
pub const OFS_PARM6: usize = 22;
pub const OFS_PARM7: usize = 25;
pub const RESERVED_OFS: usize = 28;

/// `progs.h`
pub const MAX_ENT_LEAFS: usize = 32;
/// `quakedef.h`: highest allowed value for the `max_edicts` cvar
pub const MAX_EDICTS: usize = 32000;
/// `quakedef.h`: lowest allowed value for the `max_edicts` cvar
pub const MIN_EDICTS: usize = 256;
/// `quakedef.h`: a freed edict is never reused while
/// `qcvm->time - e->freetime` is below this.
pub const MIN_EDICT_AGE_FOR_REUSE: f32 = 2.0;
/// `quakedef.h`: savegame loading generates a flood of transient spawns, so
/// a `freetime` at or below this bypasses [`MIN_EDICT_AGE_FOR_REUSE`].
pub const MAX_EDICT_FREETIME_ALWAYS_REUSE: f32 = 2.0;
/// `progs.h`
pub const MAX_AREA_DEPTH: usize = 9;
/// `progs.h`: `(2 << MAX_AREA_DEPTH)`
pub const AREA_NODES: usize = 2 << MAX_AREA_DEPTH;
/// `progs.h` (inside `struct qcvm_s`)
pub const MAX_STACK_DEPTH: usize = 1024;
/// `progs.h` (inside `struct qcvm_s`)
pub const LOCALSTACK_SIZE: usize = 16384;
/// `progs.h`: the `PR_GetTempString` ring, whose wraparound is observable
pub const STRINGTEMP_BUFFERS: usize = 1024;
/// `progs.h`
pub const STRINGTEMP_LENGTH: usize = 1024;

/// `pr_comp.h` `etype_t`
pub mod etype {
    use core::ffi::c_int;

    pub const EV_BAD: c_int = -1;
    pub const EV_VOID: c_int = 0;
    pub const EV_STRING: c_int = 1;
    pub const EV_FLOAT: c_int = 2;
    pub const EV_VECTOR: c_int = 3;
    pub const EV_ENTITY: c_int = 4;
    pub const EV_FIELD: c_int = 5;
    pub const EV_FUNCTION: c_int = 6;
    pub const EV_POINTER: c_int = 7;
    pub const EV_EXT_INTEGER: c_int = 8;
    pub const EV_EXT_UINT32: c_int = 9;
    pub const EV_EXT_SINT64: c_int = 10;
    pub const EV_EXT_UINT64: c_int = 11;
    pub const EV_EXT_DOUBLE: c_int = 12;
}

/// `pr_comp.h` `opcode_t`. The ordinals are progs-file ABI and must stay in
/// this order; `pr_exec.c`'s `pr_opnames[]` is index-aligned with it.
pub mod opcode {
    pub const OP_DONE: u16 = 0;
    pub const OP_MUL_F: u16 = 1;
    pub const OP_MUL_V: u16 = 2;
    pub const OP_MUL_FV: u16 = 3;
    pub const OP_MUL_VF: u16 = 4;
    pub const OP_DIV_F: u16 = 5;
    pub const OP_ADD_F: u16 = 6;
    pub const OP_ADD_V: u16 = 7;
    pub const OP_SUB_F: u16 = 8;
    pub const OP_SUB_V: u16 = 9;
    pub const OP_EQ_F: u16 = 10;
    pub const OP_EQ_V: u16 = 11;
    pub const OP_EQ_S: u16 = 12;
    pub const OP_EQ_E: u16 = 13;
    pub const OP_EQ_FNC: u16 = 14;
    pub const OP_NE_F: u16 = 15;
    pub const OP_NE_V: u16 = 16;
    pub const OP_NE_S: u16 = 17;
    pub const OP_NE_E: u16 = 18;
    pub const OP_NE_FNC: u16 = 19;
    pub const OP_LE: u16 = 20;
    pub const OP_GE: u16 = 21;
    pub const OP_LT: u16 = 22;
    pub const OP_GT: u16 = 23;
    pub const OP_LOAD_F: u16 = 24;
    pub const OP_LOAD_V: u16 = 25;
    pub const OP_LOAD_S: u16 = 26;
    pub const OP_LOAD_ENT: u16 = 27;
    pub const OP_LOAD_FLD: u16 = 28;
    pub const OP_LOAD_FNC: u16 = 29;
    pub const OP_ADDRESS: u16 = 30;
    pub const OP_STORE_F: u16 = 31;
    pub const OP_STORE_V: u16 = 32;
    pub const OP_STORE_S: u16 = 33;
    pub const OP_STORE_ENT: u16 = 34;
    pub const OP_STORE_FLD: u16 = 35;
    pub const OP_STORE_FNC: u16 = 36;
    pub const OP_STOREP_F: u16 = 37;
    pub const OP_STOREP_V: u16 = 38;
    pub const OP_STOREP_S: u16 = 39;
    pub const OP_STOREP_ENT: u16 = 40;
    pub const OP_STOREP_FLD: u16 = 41;
    pub const OP_STOREP_FNC: u16 = 42;
    pub const OP_RETURN: u16 = 43;
    pub const OP_NOT_F: u16 = 44;
    pub const OP_NOT_V: u16 = 45;
    pub const OP_NOT_S: u16 = 46;
    pub const OP_NOT_ENT: u16 = 47;
    pub const OP_NOT_FNC: u16 = 48;
    pub const OP_IF: u16 = 49;
    pub const OP_IFNOT: u16 = 50;
    pub const OP_CALL0: u16 = 51;
    pub const OP_CALL1: u16 = 52;
    pub const OP_CALL2: u16 = 53;
    pub const OP_CALL3: u16 = 54;
    pub const OP_CALL4: u16 = 55;
    pub const OP_CALL5: u16 = 56;
    pub const OP_CALL6: u16 = 57;
    pub const OP_CALL7: u16 = 58;
    pub const OP_CALL8: u16 = 59;
    pub const OP_STATE: u16 = 60;
    pub const OP_GOTO: u16 = 61;
    pub const OP_AND: u16 = 62;
    pub const OP_OR: u16 = 63;
    pub const OP_BITAND: u16 = 64;
    pub const OP_BITOR: u16 = 65;
    /// One past the last valid opcode; `pr_exec.c`'s `default:` arm raises
    /// "Bad opcode %i" for anything at or above this.
    pub const OP_COUNT: u16 = 66;
}

/// `pr_comp.h` `dstatement_t`.
///
/// COMPAT: `a`/`b`/`c` are declared `short` but read two different ways by
/// `pr_exec.c` — reinterpreted as `unsigned short` for global offsets
/// (`OPA`/`OPB`/`OPC`, `OP_RETURN`) and used signed for jump deltas
/// (`OP_IF`/`OP_IFNOT`/`OP_GOTO`). Both readings are preserved.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DStatement {
    pub op: u16,
    pub a: i16,
    pub b: i16,
    pub c: i16,
}

/// `pr_comp.h` `ddef_t`
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DDef {
    /// Includes the [`DEF_SAVEGLOBAL`] bit.
    pub type_: u16,
    pub ofs: u16,
    pub s_name: c_int,
}

/// `pr_comp.h` `dfunction_t`
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DFunction {
    /// Negative values are builtin ordinals (`-first_statement`).
    pub first_statement: c_int,
    pub parm_start: c_int,
    /// Total ints of parms + locals.
    pub locals: c_int,
    pub profile: c_int,
    pub s_name: c_int,
    pub s_file: c_int,
    pub numparms: c_int,
    pub parm_size: [u8; MAX_PARMS],
}

/// `pr_comp.h` `dprograms_t` — the on-disk progs.dat header.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DPrograms {
    pub version: c_int,
    /// Checked against [`PROGHEADER_CRC`]; foreign values drive the
    /// diagnostic switch in `pr_edict.c` `PR_LoadProgs`.
    pub crc: c_int,
    pub ofs_statements: c_int,
    pub numstatements: c_int,
    pub ofs_globaldefs: c_int,
    pub numglobaldefs: c_int,
    pub ofs_fielddefs: c_int,
    pub numfielddefs: c_int,
    pub ofs_functions: c_int,
    pub numfunctions: c_int,
    pub ofs_strings: c_int,
    pub numstrings: c_int,
    pub ofs_globals: c_int,
    pub numglobals: c_int,
    /// Mutated at load by `PR_MergeEngineFieldDefs`, which is what makes
    /// `edict_size` (and therefore savegame output) mod-dependent.
    pub entityfields: c_int,
}

/// `progdefs.q1` `globalvars_t` — the fixed engine-visible head of the progs
/// global block. Field names are QuakeC-visible identifiers, so they are kept
/// verbatim rather than snake-cased.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
#[allow(non_snake_case)]
pub struct GlobalVars {
    pub pad: [c_int; 28],
    pub self_: c_int, // `self` cannot be a Rust raw identifier
    pub other: c_int,
    pub world: c_int,
    pub time: f32,
    pub frametime: f32,
    pub force_retouch: f32,
    pub mapname: StringT,
    pub deathmatch: f32,
    pub coop: f32,
    pub teamplay: f32,
    pub serverflags: f32,
    pub total_secrets: f32,
    pub total_monsters: f32,
    pub found_secrets: f32,
    pub killed_monsters: f32,
    pub parm1: f32,
    pub parm2: f32,
    pub parm3: f32,
    pub parm4: f32,
    pub parm5: f32,
    pub parm6: f32,
    pub parm7: f32,
    pub parm8: f32,
    pub parm9: f32,
    pub parm10: f32,
    pub parm11: f32,
    pub parm12: f32,
    pub parm13: f32,
    pub parm14: f32,
    pub parm15: f32,
    pub parm16: f32,
    pub v_forward: [f32; 3],
    pub v_up: [f32; 3],
    pub v_right: [f32; 3],
    pub trace_allsolid: f32,
    pub trace_startsolid: f32,
    pub trace_fraction: f32,
    pub trace_endpos: [f32; 3],
    pub trace_plane_normal: [f32; 3],
    pub trace_plane_dist: f32,
    pub trace_ent: c_int,
    pub trace_inopen: f32,
    pub trace_inwater: f32,
    pub msg_entity: c_int,
    pub main: FuncT,
    pub StartFrame: FuncT,
    pub PlayerPreThink: FuncT,
    pub PlayerPostThink: FuncT,
    pub ClientKill: FuncT,
    pub ClientConnect: FuncT,
    pub PutClientInServer: FuncT,
    pub ClientDisconnect: FuncT,
    pub SetNewParms: FuncT,
    pub SetChangeParms: FuncT,
}

/// `progdefs.q1` `entvars_t` — the engine-visible field block at the head of
/// every edict. Mod-defined fields extend past it (ADR-006).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EntVars {
    pub modelindex: f32,
    pub absmin: [f32; 3],
    pub absmax: [f32; 3],
    pub ltime: f32,
    pub movetype: f32,
    pub solid: f32,
    pub origin: [f32; 3],
    pub oldorigin: [f32; 3],
    pub velocity: [f32; 3],
    pub angles: [f32; 3],
    pub avelocity: [f32; 3],
    pub punchangle: [f32; 3],
    pub classname: StringT,
    pub model: StringT,
    pub frame: f32,
    pub skin: f32,
    pub effects: f32,
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub size: [f32; 3],
    pub touch: FuncT,
    pub r#use: FuncT,
    pub think: FuncT,
    pub blocked: FuncT,
    pub nextthink: f32,
    pub groundentity: c_int,
    pub health: f32,
    pub frags: f32,
    pub weapon: f32,
    pub weaponmodel: StringT,
    pub weaponframe: f32,
    pub currentammo: f32,
    pub ammo_shells: f32,
    pub ammo_nails: f32,
    pub ammo_rockets: f32,
    pub ammo_cells: f32,
    pub items: f32,
    pub takedamage: f32,
    pub chain: c_int,
    pub deadflag: f32,
    pub view_ofs: [f32; 3],
    pub button0: f32,
    pub button1: f32,
    pub button2: f32,
    pub impulse: f32,
    pub fixangle: f32,
    pub v_angle: [f32; 3],
    pub idealpitch: f32,
    pub netname: StringT,
    pub enemy: c_int,
    pub flags: f32,
    pub colormap: f32,
    pub team: f32,
    pub max_health: f32,
    pub teleport_time: f32,
    pub armortype: f32,
    pub armorvalue: f32,
    pub waterlevel: f32,
    pub watertype: f32,
    pub ideal_yaw: f32,
    pub yaw_speed: f32,
    pub aiment: c_int,
    pub goalentity: c_int,
    pub spawnflags: f32,
    pub target: StringT,
    pub targetname: StringT,
    pub dmg_take: f32,
    pub dmg_save: f32,
    pub dmg_inflictor: c_int,
    pub owner: c_int,
    pub movedir: [f32; 3],
    pub message: StringT,
    pub sounds: f32,
    pub noise: StringT,
    pub noise1: StringT,
    pub noise2: StringT,
    pub noise3: StringT,
}

/// `common.h` `link_t` — the area-tree doubly-linked list node embedded in
/// every edict. Phase 6 never walks it; it is mirrored for layout only.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Link {
    pub prev: *mut Link,
    pub next: *mut Link,
}

/// `protocol.h` `entity_state_t` (with `LERP_BANDAID`, which `protocol.h`
/// defines unconditionally). Phase 6 never reads it; mirrored for `edict_t`
/// layout only.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EntityState {
    pub origin: [f32; 3],
    pub angles: [f32; 3],
    pub modelindex: u16,
    pub frame: u16,
    pub effects: c_uint,
    pub colormap: u8,
    pub skin: u8,
    pub scale: u8,
    pub pmovetype: u8,
    pub traileffectnum: u16,
    pub emiteffectnum: u16,
    pub velocity: [i16; 3],
    pub eflags: u8,
    pub tagindex: u8,
    pub tagentity: u16,
    pub pad: u16,
    pub colormod: [u8; 3],
    pub alpha: u8,
    pub solidsize: c_uint,
    /// `LERP_BANDAID`
    pub lerp: u16,
}

/// `progs.h` `edict_t` — only the *fixed* engine header. The progs-visible
/// field block starts at [`EntVars`] and continues past it for
/// `entityfields`; the true stride is the runtime `qcvm->edict_size`, never
/// `size_of::<Edict>()` (ADR-006).
///
/// COMPAT: `DEBUG`/`_DEBUG` builds prepend three fields, so every offset in
/// this struct is build-profile dependent. Gated on `engine-debug`, which
/// Meson sets alongside the C `-D_DEBUG`.
#[repr(C)]
pub struct Edict {
    #[cfg(feature = "engine-debug")]
    pub edict_ptr: *mut Edict,
    #[cfg(feature = "engine-debug")]
    pub qcvm_owner: *mut QcVm,
    #[cfg(feature = "engine-debug")]
    pub edict_num: u64,
    pub area: Link,
    pub num_leafs: c_uint,
    pub leafnums: [c_int; MAX_ENT_LEAFS],
    pub baseline: EntityState,
    pub alpha: u8,
    pub sendinterval: QBoolean,
    pub sendinterval_default: QBoolean,
    pub oldframe: f32,
    pub oldthinktime: f32,
    pub predthinkpos: [f32; 3],
    pub lastthink: f32,
    /// `sv.time` when the object was freed — drives the `ED_Alloc` FIFO.
    pub freetime: f32,
    pub free: QBoolean,
    pub v: EntVars,
}

/// `progs.h` `prstack_t`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PrStack {
    pub s: c_int,
    pub f: *mut DFunction,
}

/// `progs.h` `freelist_t` — the FIFO of free edict numbers as a circular
/// buffer. Entity numbering is observable (savegames, wire), so its exact
/// behaviour is compat-critical (ADR-006).
#[repr(C)]
pub struct FreeList {
    pub size: usize,
    pub head_index: usize,
    pub circular_buffer: [u16; MAX_EDICTS],
}

/// `progs.h` `areanode_t` — world.c's area tree, which lives inside the qcvm.
/// Phase 6 does not touch it (Phase 7 owns `world.c`); mirrored for layout.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AreaNode {
    pub axis: c_int,
    pub dist: f32,
    pub children: [*mut AreaNode; 2],
    pub trigger_edicts: Link,
    pub solid_edicts: Link,
}

/// `progs.h` `struct pr_extglobals_s` — pointers into `qcvm->globals`, bound
/// by `PR_EnableExtensions`. All members are `float *` or `int *`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PrExtGlobals {
    pub time: *mut f32,
    pub frametime: *mut f32,
    pub input_timelength: *mut f32,
    pub input_movevalues: *mut f32,
    pub input_angles: *mut f32,
    pub input_buttons: *mut f32,
    pub input_impulse: *mut f32,
    pub input_weapon: *mut c_int,
    pub input_cursor_screen: *mut f32,
    pub input_cursor_trace_start: *mut f32,
    pub input_cursor_trace_endpos: *mut f32,
    pub input_cursor_entitynumber: *mut f32,
    pub physics_mode: *mut f32,
    pub cltime: *mut f32,
    pub clframetime: *mut f32,
    pub maxclients: *mut f32,
    pub intermission: *mut f32,
    pub intermission_time: *mut f32,
    pub player_localnum: *mut f32,
    pub player_localentnum: *mut f32,
}

/// `progs.h` `struct pr_extfuncs_s` — QC entry points resolved by name at
/// load. Order follows the `QCEXTFUNCS_COMMON`/`_GAME`/`_SV`/`_CS` macro
/// expansion order, which is the struct's declaration order.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PrExtFuncs {
    pub game_command: FuncT,
    pub end_frame: FuncT,
    pub sv_parse_client_command: FuncT,
    pub sv_run_client_command: FuncT,
    pub csqc_init: FuncT,
    pub csqc_shutdown: FuncT,
    pub csqc_draw_hud: FuncT,
    pub csqc_draw_scores: FuncT,
    pub csqc_input_event: FuncT,
    pub csqc_console_command: FuncT,
    pub csqc_parse_event: FuncT,
    pub csqc_parse_damage: FuncT,
    pub csqc_parse_center_print: FuncT,
    pub csqc_parse_print: FuncT,
}

/// `progs.h` `struct pr_extfields_s` — field offsets the engine wants, or
/// `-1` when the progs does not define them. Declaration order is the
/// `QCEXTFIELDS_ALL`/`_GAME`/`_SS` macro expansion order.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PrExtFields {
    pub alpha: c_int,
    pub scale: c_int,
    pub colormod: c_int,
    pub tag_entity: c_int,
    pub tag_index: c_int,
    pub modelflags: c_int,
    pub origin: c_int,
    pub angles: c_int,
    pub frame: c_int,
    pub skin: c_int,
    pub customphysics: c_int,
    pub gravity: c_int,
    pub items2: c_int,
    pub movement: c_int,
    pub nodrawtoclient: c_int,
    pub drawonlytoclient: c_int,
    pub traileffectnum: c_int,
    pub emiteffectnum: c_int,
    pub button3: c_int,
    pub button4: c_int,
    pub button5: c_int,
    pub button6: c_int,
    pub button7: c_int,
    pub button8: c_int,
    pub viewzoom: c_int,
    pub send_entity: c_int,
    pub send_flags: c_int,
}

/// `progs.h` `struct qcvm_s` — the entire switchable VM state.
///
/// C owns the storage: one instance is embedded in `server_t sv`
/// (`server.h`), another in `client_state_t cl` (`client.h`), and the ambient
/// `qcvm` global points at whichever `PR_SwitchQCVM` last selected (ADR-008).
/// Rust never allocates one during Phase 6 outside tests.
#[repr(C)]
pub struct QcVm {
    pub progs: *mut DPrograms,
    pub functions: *mut DFunction,
    /// `hash_map_t *` — opaque here; `quake_util::hash_map` owns the layout.
    pub function_map: *mut c_void,
    pub statements: *mut DStatement,
    /// Same storage as `pr_global_struct`.
    pub globals: *mut f32,
    /// Points into the progs image until `PR_MergeEngineFieldDefs`
    /// reallocates it; `PR_ClearProgs` tells the two states apart by
    /// comparing against the image.
    pub fielddefs: *mut DDef,
    pub fielddefs_map: *mut c_void,
    /// `progs->entityfields * 4 + sizeof(edict_t) - sizeof(entvars_t)`,
    /// rounded up to pointer alignment. The edict array stride (ADR-006).
    pub edict_size: c_int,
    pub builtins: [BuiltinT; 1024],
    pub numbuiltins: c_int,
    pub argc: c_int,
    /// The `Con_Printf` single-step debugger flag, not the `-tracefile`
    /// oracle (`pr_trace.h`).
    pub trace: QBoolean,
    pub xfunction: *mut DFunction,
    pub xstatement: c_int,
    /// CRC16 of the entire file, taken *before* the in-place byteswap.
    pub progscrc: u16,
    /// Folded MD4 of the file — the `csprogsvers/%x.dat` lookup key.
    pub progshash: c_uint,
    pub progssize: c_uint,
    pub extglobals: PrExtGlobals,
    pub extfuncs: PrExtFuncs,
    pub extfields: PrExtFields,
    pub strings: *mut c_char,
    pub stringssize: c_int,
    pub knownstrings: *mut *const c_char,
    pub knownstringsowned: *mut QBoolean,
    pub maxknownstrings: c_int,
    pub numknownstrings: c_int,
    /// Allocated by `PR_MergeEngineFieldDefs`, not tied to edicts.
    pub progsstrings: c_int,
    pub freeknownstrings: c_int,
    pub globaldefs: *mut DDef,
    pub globaldefs_map: *mut c_void,
    /// Bitmap keyed on `-1 - ref` marking which known strings are zoned.
    pub knownzone: *mut u8,
    pub knownzonesize: usize,
    pub stack: [PrStack; MAX_STACK_DEPTH],
    pub depth: c_int,
    pub localstack: [c_int; LOCALSTACK_SIZE],
    pub localstack_used: c_int,
    pub time: f64,
    pub num_edicts: c_int,
    pub reserved_edicts: c_int,
    pub max_edicts: c_int,
    /// Cannot be array-indexed: `edict_t` is variable sized. Use
    /// `edict_size` strides (ADR-006).
    pub edicts: *mut Edict,
    pub free_list: FreeList,
    /// `struct qmodel_s *` — opaque during Phase 6.
    pub worldmodel: *mut c_void,
    pub get_model: Option<unsafe extern "C" fn(modelindex: c_int) -> *mut c_void>,
    pub areanodes: [AreaNode; AREA_NODES],
    pub numareanodes: c_int,
}

/// COMPAT (ADR-006): `EDICT_TO_PROG` yields a **byte** offset into the edict
/// array, and those values are what QuakeC sees in entity variables and what
/// flows into savegames and the wire protocol.
#[must_use]
pub const fn edict_to_prog(num: c_int, edict_size: c_int) -> c_int {
    num * edict_size
}

/// Inverse of [`edict_to_prog`]. C computes this as pointer arithmetic and
/// truncates toward zero, so a `prog` value that is not a multiple of
/// `edict_size` silently rounds down — preserved.
#[must_use]
pub const fn prog_to_edict_num(prog: c_int, edict_size: c_int) -> c_int {
    prog / edict_size
}

const _: () = {
    assert!(core::mem::size_of::<DStatement>() == 8);
    assert!(core::mem::size_of::<DDef>() == 8);
    assert!(core::mem::size_of::<DFunction>() == 36);
    assert!(core::mem::size_of::<DPrograms>() == 60);
    assert!(core::mem::size_of::<GlobalVars>() == 4 * 92);
    assert!(core::mem::size_of::<EntVars>() == 4 * 105);
    assert!(core::mem::align_of::<EntVars>() == 4);
    assert!(core::mem::size_of::<EntityState>() == 64);
    assert!(core::mem::size_of::<PrExtFuncs>() == 4 * 14);
    assert!(core::mem::size_of::<PrExtFields>() == 4 * 27);
};
